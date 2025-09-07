use std::sync::OnceLock;
use std::time::Duration;

use reqwest::{Client, ClientBuilder};
use serde::Deserialize;
use serde_json::json;

static CLIENT: OnceLock<Client> = OnceLock::new();

#[derive(Deserialize)]
pub struct SendWebhookPayload {
    pub wh_type: WebhookType,
    pub content: String,
}

#[derive(Deserialize)]
pub enum WebhookType {
    Feedback,
    BugReport,
}

impl std::fmt::Display for WebhookType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WebhookType::Feedback => "Feedback",
            WebhookType::BugReport => "Bug Report",
        })
    }
}

fn init_client() -> Client {
    ClientBuilder::new()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build HTTP Client")
}

pub async fn send_webhook(wh_type: WebhookType, content: String) -> Result<(), String> {
    let webhook = dotenvy::var("DISCORD_WEBHOOK").expect("DISCORD_WEBOOK env var not found");

    let client = CLIENT.get_or_init(init_client);

    let ts = chrono::Utc::now();

    let payload = json!({
        "embeds": [{
            // TODO To anonymize or not to ?
            /* "author": {
                "name": "username (email)"
            }, */
            "title": wh_type.to_string(),
            "description": content,
            "timestamp": ts.to_rfc3339(),
            "color": 0x7437dd,
            "footer": {
                // TODO Include version ?
                "text": "Sharify"
            }
        }]
    });

    let req = client
        .post(webhook)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("Failed to send webhook request: {err}"))?;

    if !req.status().is_success() {
        return Err(format!(
            "Webhook request failed with status {} and response {:?}",
            req.status(),
            req.text().await
        ));
    }

    Ok(())
}
