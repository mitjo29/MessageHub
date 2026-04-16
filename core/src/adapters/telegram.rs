use std::collections::HashMap;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, info};

use crate::error::{CoreError, Result};
use crate::types::{Channel, ChannelConfig, MessageContent};
use super::{ChannelAdapter, RawMessage};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

/// Telegram Bot API adapter using long-polling via `getUpdates`.
pub struct TelegramAdapter {
    client: Client,
    bot_token: Option<String>,
    last_update_id: Option<i64>,
}

impl TelegramAdapter {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            bot_token: None,
            last_update_id: None,
        }
    }

    fn api_url(&self, method: &str) -> Result<String> {
        let token = self.bot_token.as_ref().ok_or_else(|| {
            CoreError::Connection("not connected: no bot token".to_string())
        })?;
        Ok(format!("{}/bot{}/{}", TELEGRAM_API_BASE, token, method))
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    async fn connect(&mut self, config: &ChannelConfig) -> Result<()> {
        // In production, `keychain_ref` is used to look up the token from the OS keychain.
        // For now, we store it directly (will be replaced by keychain integration in a later plan).
        self.bot_token = Some(config.keychain_ref.clone());

        // Validate the token by calling getMe
        let url = self.api_url("getMe")?;
        let resp: TelegramResponse<TelegramUser> = self
            .client
            .get(&url)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| CoreError::Parse(e.to_string()))?;

        if !resp.ok {
            return Err(CoreError::Auth(format!(
                "Telegram getMe failed: {}",
                resp.description.unwrap_or_default()
            )));
        }

        let bot = resp.result.ok_or_else(|| {
            CoreError::Auth("Telegram getMe returned no result".to_string())
        })?;

        info!(bot_username = %bot.username.unwrap_or_default(), "Telegram connected");
        Ok(())
    }

    async fn fetch_messages(&self, _since: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        let url = self.api_url("getUpdates")?;

        // Use offset to only get new updates
        let mut params = vec![("timeout", "5".to_string()), ("allowed_updates", "[\"message\"]".to_string())];
        if let Some(last_id) = self.last_update_id {
            params.push(("offset", (last_id + 1).to_string()));
        }

        let resp: TelegramResponse<Vec<TelegramUpdate>> = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| CoreError::Parse(e.to_string()))?;

        if !resp.ok {
            return Err(CoreError::Channel(format!(
                "getUpdates failed: {}",
                resp.description.unwrap_or_default()
            )));
        }

        let updates = resp.result.unwrap_or_default();
        let mut raw_messages = Vec::new();

        for update in &updates {
            if let Some(ref msg) = update.message {
                let sender = msg.from.as_ref();
                let sender_name = sender
                    .map(|u| {
                        let mut name = u.first_name.clone();
                        if let Some(ref last) = u.last_name {
                            name.push(' ');
                            name.push_str(last);
                        }
                        name
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                let sender_address = sender
                    .and_then(|u| u.username.clone())
                    .unwrap_or_else(|| {
                        sender.map(|u| u.id.to_string()).unwrap_or_default()
                    });

                let timestamp = DateTime::from_timestamp(msg.date, 0)
                    .unwrap_or_else(Utc::now);

                let mut metadata = HashMap::new();
                metadata.insert("chat_id".to_string(), msg.chat.id.to_string());
                metadata.insert("chat_type".to_string(), msg.chat.chat_type.clone());
                metadata.insert("update_id".to_string(), update.update_id.to_string());
                if let Some(ref title) = msg.chat.title {
                    metadata.insert("chat_title".to_string(), title.clone());
                }

                raw_messages.push(RawMessage {
                    external_id: msg.message_id.to_string(),
                    channel: Channel::Telegram,
                    external_thread_id: Some(msg.chat.id.to_string()),
                    sender_name,
                    sender_address,
                    text: msg.text.clone(),
                    html: None,
                    subject: None,
                    attachments: vec![],
                    timestamp,
                    metadata,
                });
            }
        }

        // Note: last_update_id should be updated by the caller (AdapterManager)
        // after successful processing. For now we track it via metadata.
        debug!(count = raw_messages.len(), "telegram messages fetched");
        Ok(raw_messages)
    }

    async fn send_reply(&self, thread_id: &str, content: &MessageContent) -> Result<()> {
        let url = self.api_url("sendMessage")?;

        let text = content.text.as_deref().ok_or_else(|| {
            CoreError::InvalidInput("message text is required for Telegram".to_string())
        })?;

        let body = serde_json::json!({
            "chat_id": thread_id,
            "text": text,
        });

        let resp: TelegramResponse<serde_json::Value> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?
            .json()
            .await
            .map_err(|e| CoreError::Parse(e.to_string()))?;

        if !resp.ok {
            return Err(CoreError::Channel(format!(
                "sendMessage failed: {}",
                resp.description.unwrap_or_default()
            )));
        }

        info!(chat_id = %thread_id, "telegram message sent");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.bot_token = None;
        self.last_update_id = None;
        info!("Telegram disconnected");
        Ok(())
    }

    fn channel_type(&self) -> Channel {
        Channel::Telegram
    }
}

// --- Telegram API response types ---

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    #[allow(dead_code)]
    is_bot: bool,
    first_name: String,
    last_name: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    from: Option<TelegramUser>,
    chat: TelegramChat,
    date: i64,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
    title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegram_adapter_channel_type() {
        let adapter = TelegramAdapter::new();
        assert_eq!(adapter.channel_type(), Channel::Telegram);
    }

    #[test]
    fn test_api_url_without_token() {
        let adapter = TelegramAdapter::new();
        let result = adapter.api_url("getMe");
        assert!(result.is_err());
    }

    #[test]
    fn test_api_url_with_token() {
        let mut adapter = TelegramAdapter::new();
        adapter.bot_token = Some("123:ABC".to_string());
        let url = adapter.api_url("getMe").unwrap();
        assert_eq!(url, "https://api.telegram.org/bot123:ABC/getMe");
    }

    #[tokio::test]
    async fn test_disconnect_clears_state() {
        let mut adapter = TelegramAdapter::new();
        adapter.bot_token = Some("token".to_string());
        adapter.last_update_id = Some(42);

        adapter.disconnect().await.unwrap();

        assert!(adapter.bot_token.is_none());
        assert!(adapter.last_update_id.is_none());
    }

    #[tokio::test]
    async fn test_send_reply_requires_text() {
        let mut adapter = TelegramAdapter::new();
        adapter.bot_token = Some("fake-token".to_string());

        let content = MessageContent {
            text: None,
            html: None,
            subject: None,
            attachments: vec![],
        };

        let result = adapter.send_reply("123", &content).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("text is required"));
    }
}
