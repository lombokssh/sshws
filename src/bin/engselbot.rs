use teloxide::{prelude::*, types::{KeyboardMarkup, KeyboardButton}};
use uuid::Uuid;
use serde_json::json;
use sqlx::PgPool;

fn censor_number(n: &str) -> String {
    if n.len() > 6 {
        let masked = "*".repeat(n.len() - 6);
        format!("{}{}{}", &n[..4], masked, &n[n.len()-2..])
    } else {
        n.to_string()
    }
}

async fn check_xl_quota(number: &str) -> Result<String, reqwest::Error> {
    let url = format!("https://xl-ku.my.id/end.php?check=package&number={}&version=2", number);
    let json: serde_json::Value = reqwest::Client::new().get(&url).send().await?.json().await?;

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
            if let Some(name) = pkg["name"].as_str() {
                result.push_str(&format!("📦 <b>{}</b>\n", name));
            }
            if let Some(expiry) = pkg["expiry"].as_str() {
                result.push_str(&format!("   Exp: {}\n", expiry));
            }
            if let Some(quotas) = pkg["quotas"].as_array() {
                for q in quotas {
                    let q_name = q["name"].as_str().unwrap_or("");
                    let q_rem = q["remaining"].as_str().unwrap_or("");
                    if !q_name.is_empty() && !q_rem.is_empty() {
                        result.push_str(&format!("   - {}: <b>{}</b>\n", q_name, q_rem));
                    }
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
    let id = user.id.0 as i64;
    let now = chrono::Utc::now();

    let result = sqlx::query(
        r#"
        INSERT INTO telegram_users (
            id, username, first_name, last_name, language_code,
            is_premium, last_active_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
        ON CONFLICT (id) DO UPDATE SET
            username       = EXCLUDED.username,
            first_name     = EXCLUDED.first_name,
            last_name      = EXCLUDED.last_name,
            language_code  = EXCLUDED.language_code,
            is_premium     = EXCLUDED.is_premium,
            start_count    = telegram_users.start_count + 1,
            last_active_at = EXCLUDED.last_active_at,
            updated_at     = EXCLUDED.updated_at
        "#,
    )
    .bind(id)
    .bind(user.username.as_deref())
    .bind(&user.first_name)
    .bind(user.last_name.as_deref())
    .bind(user.language_code.as_deref())
    .bind(user.is_premium)
    .bind(now)
    .execute(pool)
    .await;

    if let Err(e) = result {
        log::error!("Failed to upsert user {}: {}", id, e);
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok(); // ponytail: lazy .env loader, ignore failure if file is missing
    pretty_env_logger::init();

    log::info!("Starting engselbot...");

    let enable_user_sync = std::env::var("ENABLE_USER_SYNC").map(|v| v == "true").unwrap_or(false);

    // Setup DB pool only when user sync is enabled
    let db_pool: Option<PgPool> = if enable_user_sync {
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set if ENABLE_USER_SYNC is true");
        let pool = PgPool::connect(&database_url).await
            .expect("Failed to connect to PostgreSQL");
        // ponytail: run migrations automatically on startup, no external CLI needed
        sqlx::migrate!("./migrations").run(&pool).await
            .expect("Failed to run migrations");
        log::info!("PostgreSQL connected & migrations applied.");
        Some(pool)
    } else {
        None
    };

    let bot = Bot::from_env();
    let me = bot.get_me().await.expect("Failed to get bot info");
    let bot_username = me.username().to_lowercase();
    log::info!("Bot username: @{}", bot_username);

    // ponytail: kept simple repl instead of full dispatcher. ReplyKeyboardMarkup sends normal messages,
    // skipping callback query boilerplate entirely.
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let bot_username = bot_username.clone();
        let pool = db_pool.clone();

        async move {
            if let Some(ref p) = pool {
                if let Some(user) = msg.from.as_ref() {
                    let user_clone = user.clone();
                    let p = p.clone();
                    tokio::spawn(async move { upsert_user(&p, &user_clone).await; });
                }
            }

            if let Some(text) = msg.text() {
                log::info!("Received text from chat {}: {}", msg.chat.id, text);
                // ponytail: thread_id for forum/topic groups, None in regular groups/DM
                let thread_id = msg.thread_id;
                // Normalisasi teks untuk grup: hapus @bot_username dari perintah
                let first_word = text.split_whitespace().next().unwrap_or("");
                let clean_text = if first_word.starts_with('/') && first_word.contains('@') {
                    let idx = first_word.find('@').unwrap();
                    let mentioned = first_word[idx+1..].to_lowercase();
                    // Jika mention bukan bot kita, skip
                    if mentioned != bot_username {
                        return Ok(());
                    }
                    text.replace(&first_word[idx..], "")
                } else {
                    text.to_string()
                };

                match clean_text.as_str() {
                    "/start" | "/gen" => {
                        let keyboard = KeyboardMarkup::new(vec![
                            vec![KeyboardButton::new("VLESS"), KeyboardButton::new("TROJAN"), KeyboardButton::new("VMESS")],
                            vec![KeyboardButton::new("Cek Kuota XL/Axis")],
                        ]).resize_keyboard().one_time_keyboard();

                        let req = bot.send_message(msg.chat.id, "🚀 <b>Small, Fast &amp; High Performance</b> ⚡\n\nPlease choose a menu:")
                            .reply_markup(keyboard)
                            .parse_mode(teloxide::types::ParseMode::Html);
                        let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                        req.await?;
                    }
                    "Cek Kuota XL/Axis" | "/start cek" => {
                        if msg.chat.is_private() {
                            let req = bot.send_message(msg.chat.id, "Silakan kirimkan nomor XL atau Axis Anda (tanpa spasi):\n\nContoh: <code>0859xxxxxx</code>")
                                .parse_mode(teloxide::types::ParseMode::Html);
                            let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                            req.await?;
                        } else {
                            let req = bot.send_message(msg.chat.id, "🔒 Fitur cek kuota hanya tersedia di <b>private chat</b>.\n\nKlik tombol di bawah untuk chat langsung dengan bot:")
                                .parse_mode(teloxide::types::ParseMode::Html)
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

                        let (url, yaml) = if clean_text == "VLESS" {
                            let url = format!("vless://{}@{}:443?encryption=none&security=tls&sni={}&fp=chrome&type=ws&host={}&path=%2Fvless#kita_temenan_aja", uuid, host, host, host);
                            let yaml = format!(r#"- name: "kita temenan aja"
  type: vless
  server: {0}
  port: 443
  uuid: {1}
  network: ws
  tls: true
  udp: true
  sni: "{0}"
  ws-opts:
    path: "/vless"
    headers:
      host: "{0}""#, host, uuid);
                            (url, yaml)
                        } else if clean_text == "TROJAN" {
                            let url = format!("trojan://{}@{}:443?security=tls&sni={}&type=ws&host={}&path=%2Ftrojan#kita_temenan_aja", uuid, host, host, host);
                            let yaml = format!(r#"- name: "kita temenan aja"
  type: trojan
  server: {0}
  port: 443
  password: {1}
  network: ws
  tls: true
  udp: true
  sni: "{0}"
  ws-opts:
    path: "/trojan"
    headers:
      host: "{0}""#, host, uuid);
                            (url, yaml)
                        } else {
                            use base64::{Engine as _, engine::general_purpose::STANDARD};
                            let vmess_json = json!({
                                "v": "2", "ps": "kita temenan aja", "add": host,
                                "port": "443", "id": uuid, "aid": "0", "scy": "auto",
                                "net": "ws", "type": "none", "host": host,
                                "path": "/vmess", "tls": "tls", "sni": host, "alpn": ""
                            }).to_string();
                            let url = format!("vmess://{}", STANDARD.encode(vmess_json));
                            let yaml = format!(r#"- name: "kita temenan aja"
  type: vmess
  server: {0}
  port: 443
  uuid: {1}
  alterId: 0
  cipher: auto
  network: ws
  tls: true
  udp: true
  sni: "{0}"
  ws-opts:
    path: "/vmess"
    headers:
      host: "{0}""#, host, uuid);
                            (url, yaml)
                        };

                        let response = format!("⚡ <b>Small, Fast &amp; High Performance!</b>\n\n<b>{}:</b>\n<code>{}</code>\n\n<b>CLASH META / V2RAY:</b>\n<code>\n{}\n</code>", clean_text, url, yaml);

                        let qr_url = reqwest::Url::parse_with_params("https://api.qrserver.com/v1/create-qr-code/", &[("size", "400x400"), ("margin", "10"), ("data", &url)]).unwrap();
                        let req = bot.send_photo(msg.chat.id, teloxide::types::InputFile::url(qr_url))
                            .caption(response)
                            .parse_mode(teloxide::types::ParseMode::Html);
                        let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                        req.await?;
                    }
                    _ => {
                        let text = text.trim();
                        // Cek nomor hanya di private chat
                        let number = if msg.chat.is_private() && (text.starts_with("08") || text.starts_with("628") || text.starts_with("+628")) {
                            Some(text)
                        } else {
                            None
                        };

                        if let Some(num) = number {
                            let req = bot.send_message(msg.chat.id, format!("🔄 Mengecek kuota <code>{}</code>...", censor_number(num)))
                                .parse_mode(teloxide::types::ParseMode::Html);
                            let req = if let Some(tid) = thread_id { req.message_thread_id(tid) } else { req };
                            let msg_reply = req.await?;

                            match check_xl_quota(num).await {
                                Ok(response) => {
                                    bot.edit_message_text(msg.chat.id, msg_reply.id, response)
                                        .parse_mode(teloxide::types::ParseMode::Html)
                                        .await?;
                                }
                                Err(_) => {
                                    bot.edit_message_text(msg.chat.id, msg_reply.id, "❌ Terjadi kesalahan saat menghubungi server.").await?;
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        }
    })
    .await;
}
