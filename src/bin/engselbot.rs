use teloxide::{prelude::*, types::{KeyboardMarkup, KeyboardButton}};
use uuid::Uuid;
use serde_json::json;

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
            match text {
                "/start" | "/gen" => {
                    let keyboard = KeyboardMarkup::new(vec![vec![
                        KeyboardButton::new("VLESS"),
                        KeyboardButton::new("TROJAN"),
                    ]]).resize_keyboard().one_time_keyboard();
                    
                    bot.send_message(msg.chat.id, "🚀 <b>Small, Fast & High Performance</b> ⚡\n\nPlease choose a protocol to generate your account:")
                        .reply_markup(keyboard)
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
                _ => {}
            }
        }
        Ok(())
        }
    })
    .await;
}
