use teloxide::{prelude::*, types::{KeyboardMarkup, KeyboardButton, InputFile, ParseMode}};
use uuid::Uuid;
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;

fn censor_number(n: &str) -> String {
    if n.len() > 6 {
        format!("{}{}{}", &n[..4], "*".repeat(n.len() - 6), &n[n.len()-2..])
    } else {
        n.to_string()
    }
}

async fn check_xl_quota(number: &str) -> Result<String, reqwest::Error> {
    let json: serde_json::Value = reqwest::Client::new()
        .get(format!("https://xl-ku.my.id/end.php?check=package&number={}&version=2", number))
        .send().await?.json().await?;

    if !json["success"].as_bool().unwrap_or(false) {
        return Ok(format!("❌ Gagal mengecek kuota atau nomor tidak valid:\n{}", json["message"].as_str().unwrap_or("Unknown Error")));
    }

    let api_num = json["data"]["subs_info"]["msisdn"].as_str().unwrap_or(number);
    let mut result = format!("📱 <b>Nomor:</b> <code>{}</code>\n", censor_number(api_num));
    if let Some(exp_date) = json["data"]["subs_info"]["exp_date"].as_str() {
        result.push_str(&format!("Masa Aktif: {}\n\n", exp_date));
    }
    if let Some(packages) = json["data"]["package_info"]["packages"].as_array() {
        for pkg in packages {
            if let Some(name) = pkg["name"].as_str() { result.push_str(&format!("📦 <b>{}</b>\n", name)); }
            if let Some(expiry) = pkg["expiry"].as_str() { result.push_str(&format!("   Exp: {}\n", expiry)); }
            if let Some(quotas) = pkg["quotas"].as_array() {
                for q in quotas {
                    let (n, r) = (q["name"].as_str().unwrap_or(""), q["remaining"].as_str().unwrap_or(""));
                    if !n.is_empty() && !r.is_empty() { result.push_str(&format!("   - {}: <b>{}</b>\n", n, r)); }
                }
            }
            result.push_str("\n");
        }
    } else {
        result.push_str("Tidak ada paket aktif.\n");
    }
    Ok(result)
}

async fn upsert_user(pool: &PgPool, user: &teloxide::types::User) {
    let res = sqlx::query(r#"
        INSERT INTO telegram_users (id, username, first_name, last_name, language_code, is_premium, last_active_at, updated_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$7)
        ON CONFLICT (id) DO UPDATE SET
            username=EXCLUDED.username, first_name=EXCLUDED.first_name, last_name=EXCLUDED.last_name,
            language_code=EXCLUDED.language_code, is_premium=EXCLUDED.is_premium,
            start_count=telegram_users.start_count+1,
            last_active_at=EXCLUDED.last_active_at, updated_at=EXCLUDED.updated_at
    "#)
    .bind(user.id.0 as i64).bind(user.username.as_deref()).bind(&user.first_name)
    .bind(user.last_name.as_deref()).bind(user.language_code.as_deref())
    .bind(user.is_premium).bind(chrono::Utc::now())
    .execute(pool).await;
    if let Err(e) = res { log::error!("upsert_user {}: {}", user.id.0, e); }
}

async fn save_message(pool: &PgPool, msg: &Message) {
    // ponytail: detect type by presence, not a full enum
    let kind = if msg.text().is_some() { "text" }
        else if msg.photo().is_some() { "photo" }
        else if msg.video().is_some() { "video" }
        else if msg.document().is_some() { "document" }
        else if msg.animation().is_some() { "animation" }
        else if msg.audio().is_some() { "audio" }
        else if msg.voice().is_some() { "voice" }
        else if msg.sticker().is_some() { "sticker" }
        else { "other" };
        
    let chat_type = if msg.chat.is_private() { "private" }
        else if msg.chat.is_group() { "group" }
        else if msg.chat.is_supergroup() { "supergroup" }
        else if msg.chat.is_channel() { "channel" }
        else { "unknown" };

    let reply_to_id = msg.reply_to_message().map(|m| m.id.0);

    let res = sqlx::query(
        "INSERT INTO messages (message_id, chat_id, chat_title, chat_type, user_id, text, message_type, reply_to_message_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)"
    )
    .bind(msg.id.0)
    .bind(msg.chat.id.0)
    .bind(msg.chat.title())
    .bind(chat_type)
    .bind(msg.from.as_ref().map(|u| u.id.0 as i64))
    .bind(msg.text().or(msg.caption()))
    .bind(kind)
    .bind(reply_to_id)
    .execute(pool).await;
    if let Err(e) = res { log::error!("save_message: {}", e); }
}

// ponytail: pass the template message directly instead of a hand-rolled enum;
// forward_message reuses Telegram's file cache — no file_id wrangling needed.
async fn do_broadcast(bot: &Bot, pool: &PgPool, owner_chat_id: ChatId, template_msg: &Message, caption: &str) -> Result<(), teloxide::RequestError> {
    use sqlx::Row;
    let rows = match sqlx::query("SELECT id FROM telegram_users WHERE is_active=TRUE AND blocked_bot=FALSE")
        .fetch_all(pool).await
    {
        Err(e) => { bot.send_message(owner_chat_id, format!("❌ DB error: {}", e)).await?; return Ok(()); }
        Ok(r) => r,
    };

    let total = rows.len();
    let status = bot.send_message(owner_chat_id, format!("📡 Broadcasting to {} users…", total)).await?;
    let (mut sent, mut failed) = (0u32, 0u32);

    for row in &rows {
        let dst = ChatId(row.get::<i64, _>("id"));
        // ponytail: copy_message reuses server-side file — zero re-upload, works for all media types
        let res = if template_msg.text().is_some() {
            bot.send_message(dst, caption).parse_mode(ParseMode::Html).await.map(|_| ())
        } else {
            bot.copy_message(dst, template_msg.chat.id, template_msg.id)
                .caption(caption).await.map(|_| ())
        };
        match res { Ok(_) => sent += 1, Err(e) => { log::warn!("broadcast→{}: {}", dst, e); failed += 1; } }
        tokio::time::sleep(Duration::from_millis(50)).await; // ponytail: ~20/s, Telegram limit is 30/s
    }

    bot.edit_message_text(owner_chat_id, status.id,
        format!("✅ Done! 📨 Sent: {} | ❌ Failed: {}", sent, failed)).await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    pretty_env_logger::init();
    log::info!("Starting engselbot…");

    let enable_user_sync = std::env::var("ENABLE_USER_SYNC").map(|v| v == "true").unwrap_or(false);
    let owner_id: i64 = std::env::var("OWNER_ID").expect("OWNER_ID must be set")
        .parse().expect("OWNER_ID must be a valid integer");

    let db_pool: Option<PgPool> = if enable_user_sync {
        let pool = PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"))
            .await.expect("Failed to connect to PostgreSQL");
        sqlx::migrate!("./migrations").run(&pool).await.expect("Failed to run migrations");
        log::info!("PostgreSQL connected & migrations applied.");
        Some(pool)
    } else {
        None
    };

    let bot = Bot::from_env();
    let bot_username = bot.get_me().await.expect("Failed to get bot info").username().to_lowercase();
    log::info!("Bot username: @{}", bot_username);

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let bot_username = bot_username.clone();
        let pool = db_pool.clone();

        async move {
            let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
            
            if let Ok(json_msg) = serde_json::to_string(&msg) {
                log::info!("Incoming message JSON: {}", json_msg);
            }

            if let Some(ref p) = pool {
                if let Some(user) = msg.from.as_ref() {
                    let (u, p2) = (user.clone(), p.clone());
                    tokio::spawn(async move { upsert_user(&p2, &u).await; });
                }
                // save every incoming message regardless of type
                let (msg_snap, p2) = (msg.clone(), p.clone());
                tokio::spawn(async move { save_message(&p2, &msg_snap).await; });
            }

            // --- BROADCAST (owner only: text /broadcast <msg>, or any media with caption /broadcast <caption>) ---
            let broadcast_caption = msg.text()
                .filter(|t| t.trim().starts_with("/broadcast "))
                .map(|t| t.trim()[11..].trim().to_string())
                .or_else(|| msg.caption()
                    .filter(|c| c.trim().starts_with("/broadcast"))
                    .map(|c| c.trim().strip_prefix("/broadcast").unwrap_or("").trim().to_string())
                );

            if let Some(cap) = broadcast_caption {
                if sender_id != owner_id {
                    bot.send_message(msg.chat.id, "⛔ Owner only.").await?;
                    return Ok(());
                }
                match pool.as_ref() {
                    None => { bot.send_message(msg.chat.id, "❌ Set ENABLE_USER_SYNC=true and DATABASE_URL.").await?; }
                    Some(p) => { do_broadcast(&bot, p, msg.chat.id, &msg, &cap).await?; }
                }
                return Ok(());
            }

            // --- TEXT COMMANDS ---
            let Some(text) = msg.text() else { return Ok(()); };
            log::info!("chat {} text: {}", msg.chat.id, text);
            let thread_id = msg.thread_id;

            let first_word = text.split_whitespace().next().unwrap_or("");
            let clean_text = if first_word.starts_with('/') && first_word.contains('@') {
                let idx = first_word.find('@').unwrap();
                if first_word[idx+1..].to_lowercase() != bot_username { return Ok(()); }
                text.replace(&first_word[idx..], "")
            } else {
                text.to_string()
            };

            match clean_text.as_str() {
                "/start" | "/gen" => {
                    let kb = KeyboardMarkup::new(vec![
                        vec![KeyboardButton::new("VLESS"), KeyboardButton::new("TROJAN"), KeyboardButton::new("VMESS")],
                        vec![KeyboardButton::new("Cek Kuota XL/Axis")],
                    ]).resize_keyboard().one_time_keyboard();
                    let req = bot.send_message(msg.chat.id, "🚀 <b>Small, Fast &amp; High Performance</b> ⚡\n\nPlease choose a menu:")
                        .reply_markup(kb).parse_mode(ParseMode::Html);
                    let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                    req.await?;
                }

                "Cek Kuota XL/Axis" | "/start cek" => {
                    if msg.chat.is_private() {
                        let req = bot.send_message(msg.chat.id, "Silakan kirimkan nomor XL/Axis Anda:\n\nContoh: <code>0859xxxxxx</code>")
                            .parse_mode(ParseMode::Html);
                        let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                        req.await?;
                    } else {
                        let req = bot.send_message(msg.chat.id, "🔒 Fitur ini hanya tersedia di <b>private chat</b>.")
                            .parse_mode(ParseMode::Html)
                            .reply_markup(teloxide::types::InlineKeyboardMarkup::new(vec![vec![
                                teloxide::types::InlineKeyboardButton::url(
                                    "💬 Chat Privat",
                                    format!("https://t.me/{}?start=cek", bot_username).parse().unwrap(),
                                )
                            ]]));
                        let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                        req.await?;
                    }
                }

                "VLESS" | "TROJAN" | "VMESS" => {
                    let uuid = Uuid::new_v4().to_string();
                    let host = "free.engsel.qzz.io";
                    let (url, yaml) = match clean_text.as_str() {
                        "VLESS" => {
                            let url = format!("vless://{}@{}:443?encryption=none&security=tls&sni={}&fp=chrome&type=ws&host={}&path=%2Fvless#kita_temenan_aja", uuid, host, host, host);
                            let yaml = format!("- name: \"kita temenan aja\"\n  type: vless\n  server: {0}\n  port: 443\n  uuid: {1}\n  network: ws\n  tls: true\n  udp: true\n  sni: \"{0}\"\n  ws-opts:\n    path: \"/vless\"\n    headers:\n      host: \"{0}\"", host, uuid);
                            (url, yaml)
                        }
                        "TROJAN" => {
                            let url = format!("trojan://{}@{}:443?security=tls&sni={}&type=ws&host={}&path=%2Ftrojan#kita_temenan_aja", uuid, host, host, host);
                            let yaml = format!("- name: \"kita temenan aja\"\n  type: trojan\n  server: {0}\n  port: 443\n  password: {1}\n  network: ws\n  tls: true\n  udp: true\n  sni: \"{0}\"\n  ws-opts:\n    path: \"/trojan\"\n    headers:\n      host: \"{0}\"", host, uuid);
                            (url, yaml)
                        }
                        _ => { // VMESS
                            use base64::{Engine as _, engine::general_purpose::STANDARD};
                            let j = json!({"v":"2","ps":"kita temenan aja","add":host,"port":"443","id":uuid,
                                "aid":"0","scy":"auto","net":"ws","type":"none","host":host,
                                "path":"/vmess","tls":"tls","sni":host,"alpn":""}).to_string();
                            let url = format!("vmess://{}", STANDARD.encode(j));
                            let yaml = format!("- name: \"kita temenan aja\"\n  type: vmess\n  server: {0}\n  port: 443\n  uuid: {1}\n  alterId: 0\n  cipher: auto\n  network: ws\n  tls: true\n  udp: true\n  sni: \"{0}\"\n  ws-opts:\n    path: \"/vmess\"\n    headers:\n      host: \"{0}\"", host, uuid);
                            (url, yaml)
                        }
                    };
                    let response = format!("⚡ <b>Small, Fast &amp; High Performance!</b>\n\n<b>{}:</b>\n<code>{}</code>\n\n<b>CLASH META / V2RAY:</b>\n<code>\n{}\n</code>", clean_text, url, yaml);
                    let qr = reqwest::Url::parse_with_params("https://api.qrserver.com/v1/create-qr-code/", &[("size","400x400"),("margin","10"),("data",&url)]).unwrap();
                    let req = bot.send_photo(msg.chat.id, InputFile::url(qr)).caption(response).parse_mode(ParseMode::Html);
                    let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                    req.await?;
                }

                _ => {
                    let t = text.trim();
                    if msg.chat.is_private() && (t.starts_with("08") || t.starts_with("628") || t.starts_with("+628")) {
                        let req = bot.send_message(msg.chat.id, format!("🔄 Mengecek kuota <code>{}</code>…", censor_number(t)))
                            .parse_mode(ParseMode::Html);
                        let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                        let reply = req.await?;
                        match check_xl_quota(t).await {
                            Ok(r) => { bot.edit_message_text(msg.chat.id, reply.id, r).parse_mode(ParseMode::Html).await?; }
                            Err(_) => { bot.edit_message_text(msg.chat.id, reply.id, "❌ Terjadi kesalahan saat menghubungi server.").await?; }
                        }
                    }
                }
            }
            Ok(())
        }
    }).await;
}
