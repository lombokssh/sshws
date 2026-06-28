use teloxide::{prelude::*, types::{KeyboardMarkup, KeyboardButton}};
use uuid::Uuid;
use serde_json::json;

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
    let client = reqwest::Client::new();
    let res = client.get(&url)
        .header("accept", "*/*")
        .header("accept-language", "id-ID,id;q=0.9,en-ID;q=0.8,en;q=0.7,en-US;q=0.6")
        .header("dnt", "1")
        .header("priority", "u=1, i")
        .header("referer", "https://xl-ku.my.id/api")
        .header("sec-ch-ua", "\"Google Chrome\";v=\"149\", \"Chromium\";v=\"149\", \"Not)A;Brand\";v=\"24\"")
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", "\"macOS\"")
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36")
        .send()
        .await?;

    let json: serde_json::Value = res.json().await?;
    
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

async fn save_or_update_user(user: teloxide::types::User, api_url: String, api_key: String) {
    let now = chrono::Utc::now().to_rfc3339();
        
        let variables = json!({
            "where": { "id": user.id.0.to_string() },
            "create": {
                "id": user.id.0.to_string(),
                "username": user.username,
                "firstName": user.first_name,
                "lastName": user.last_name,
                "languageCode": user.language_code,
                "isPremium": user.is_premium,
                "role": "USER",
                "isActive": true,
                "blockedBot": false,
                "startCount": 1,
                "lastActiveAt": now,
                "isInGroup": false,
                "isInChannel": false
            },
            "update": {
                "username": user.username,
                "firstName": user.first_name,
                "lastName": user.last_name,
                "languageCode": user.language_code,
                "isPremium": user.is_premium,
                "lastActiveAt": now,
                "updatedAt": now
            }
        });

        let _ = reqwest::Client::new().post(&api_url)
            .header("X-API-Key", api_key)
            .json(&json!({
                "query": r#"mutation($where: JSON!, $create: JSON!, $update: JSON!) { upsertData(model: "telegramUser", where: $where, create: $create, update: $update) }"#,
                "variables": variables
            }))
            .send()
            .await;
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok(); // ponytail: lazy .env loader, ignore failure if file is missing
    pretty_env_logger::init();
    
    log::info!("Starting engselbot...");
    
    let enable_user_sync = std::env::var("ENABLE_USER_SYNC").map(|v| v == "true").unwrap_or(false);
    let api_url = std::env::var("GRAPHQL_API_URL").unwrap_or_default();
    let api_key = std::env::var("GRAPHQL_API_KEY").unwrap_or_default();
    
    if enable_user_sync && (api_url.is_empty() || api_key.is_empty()) {
        panic!("GRAPHQL_API_URL and GRAPHQL_API_KEY must be set if ENABLE_USER_SYNC is true");
    }
    
    let bot = Bot::from_env();
    
    // ponytail: kept simple repl instead of full dispatcher. ReplyKeyboardMarkup sends normal messages, 
    // skipping callback query boilerplate entirely.
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let api_url = api_url.clone();
        let api_key = api_key.clone();
        
        async move {
            if enable_user_sync {
                if let Some(user) = msg.from.as_ref() {
                    let user_clone = user.clone();
                    let url = api_url.clone();
                    let key = api_key.clone();
                    tokio::spawn(async move {
                        save_or_update_user(user_clone, url, key).await;
                    });
                }
            }

        if let Some(text) = msg.text() {
            log::info!("Received text from chat {}: {}", msg.chat.id, text);
            // Normalisasi teks untuk grup: hapus @bot_username dari perintah
            let first_word = text.split_whitespace().next().unwrap_or("");
            let clean_text = if first_word.starts_with('/') && first_word.contains('@') {
                let idx = first_word.find('@').unwrap();
                text.replace(&first_word[idx..], "")
            } else {
                text.to_string()
            };

            match clean_text.as_str() {
                "/start" | "/gen" => {
                    let keyboard = KeyboardMarkup::new(vec![
                        vec![KeyboardButton::new("VLESS"), KeyboardButton::new("TROJAN")],
                        vec![KeyboardButton::new("Cek Kuota XL/Axis")],
                    ]).resize_keyboard().one_time_keyboard();
                    
                    bot.send_message(msg.chat.id, "🚀 <b>Small, Fast & High Performance</b> ⚡\n\nPlease choose a menu:")
                        .reply_markup(keyboard)
                        .parse_mode(teloxide::types::ParseMode::Html)
                        .await?;
                }
                "Cek Kuota XL/Axis" => {
                    bot.send_message(msg.chat.id, "Silakan kirimkan nomor XL atau Axis Anda (tanpa spasi):\n\nContoh: <code>0859xxxxxx</code>")
                        .parse_mode(teloxide::types::ParseMode::Html)
                        .await?;
                }
                "VLESS" | "TROJAN" => {
                    let uuid = Uuid::new_v4().to_string();
                    let host = "free.engsel.qzz.io";
                    
                    let (url, yaml) = if text == "VLESS" {
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
                    } else {
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
                    };
                    
                    let response = format!("⚡ <b>Small, Fast & High Performance!</b>\n\n<b>{2}:</b>\n<code>{0}</code>\n\n<b>CLASH META / V2RAY:</b>\n<code>\n{1}\n</code>", url, yaml, text);
                    
                    bot.send_message(msg.chat.id, response)
                        .parse_mode(teloxide::types::ParseMode::Html)
                        .await?;
                }
                _ => {
                    let text = text.trim();
                    let is_private = msg.chat.is_private();
                    // Di grup, hanya proses nomor jika pesan tersebut adalah reply ke bot
                    let is_reply_to_bot = msg.reply_to_message().is_some();
                    let should_check = is_private || is_reply_to_bot;

                    let number = if text.starts_with("/xl ") || text.starts_with("/axis ") {
                        text.split_whitespace().nth(1)
                    } else if should_check && (text.starts_with("08") || text.starts_with("628") || text.starts_with("+628")) {
                        Some(text)
                    } else {
                        None
                    };

                    if let Some(num) = number {
                        let msg_reply = bot.send_message(msg.chat.id, format!("🔄 Mengecek kuota <code>{}</code>...", censor_number(num)))
                            .parse_mode(teloxide::types::ParseMode::Html)
                            .await?;
                        
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
