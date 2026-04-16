use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::types::{Channel, ChannelConfig, MessageContent};
use super::{ChannelAdapter, RawMessage};

/// A mock adapter for testing. Stores messages in memory and records all
/// method calls for assertion.
#[derive(Clone)]
pub struct MockAdapter {
    messages: Arc<Mutex<Vec<RawMessage>>>,
    sent_replies: Arc<Mutex<Vec<SentReply>>>,
    connected: Arc<Mutex<bool>>,
    channel: Channel,
    fail_connect: Arc<Mutex<bool>>,
    fail_fetch: Arc<Mutex<bool>>,
    fail_send: Arc<Mutex<bool>>,
}

/// A recorded reply sent through the mock adapter.
#[derive(Debug, Clone)]
pub struct SentReply {
    pub thread_id: String,
    pub content: MessageContent,
    pub sent_at: DateTime<Utc>,
}

impl MockAdapter {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
            sent_replies: Arc::new(Mutex::new(Vec::new())),
            connected: Arc::new(Mutex::new(false)),
            channel: Channel::Telegram,
            fail_connect: Arc::new(Mutex::new(false)),
            fail_fetch: Arc::new(Mutex::new(false)),
            fail_send: Arc::new(Mutex::new(false)),
        }
    }

    pub fn with_channel(mut self, channel: Channel) -> Self {
        self.channel = channel;
        self
    }

    /// Add a message that will be returned by `fetch_messages`.
    pub fn add_message(&self, msg: RawMessage) {
        self.messages.lock().unwrap().push(msg);
    }

    /// Add multiple messages at once.
    pub fn add_messages(&self, msgs: Vec<RawMessage>) {
        self.messages.lock().unwrap().extend(msgs);
    }

    /// Get all replies sent through this adapter.
    pub fn sent_replies(&self) -> Vec<SentReply> {
        self.sent_replies.lock().unwrap().clone()
    }

    /// Check if the adapter is currently connected.
    pub fn is_connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }

    /// Configure the adapter to fail on connect.
    pub fn set_fail_connect(&self, fail: bool) {
        *self.fail_connect.lock().unwrap() = fail;
    }

    /// Configure the adapter to fail on fetch.
    pub fn set_fail_fetch(&self, fail: bool) {
        *self.fail_fetch.lock().unwrap() = fail;
    }

    /// Configure the adapter to fail on send.
    pub fn set_fail_send(&self, fail: bool) {
        *self.fail_send.lock().unwrap() = fail;
    }

    /// Clear all stored messages.
    pub fn clear_messages(&self) {
        self.messages.lock().unwrap().clear();
    }
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelAdapter for MockAdapter {
    async fn connect(&mut self, _config: &ChannelConfig) -> Result<()> {
        if *self.fail_connect.lock().unwrap() {
            return Err(crate::error::CoreError::Connection(
                "mock connect failure".to_string(),
            ));
        }
        *self.connected.lock().unwrap() = true;
        Ok(())
    }

    async fn fetch_messages(&self, since: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        if *self.fail_fetch.lock().unwrap() {
            return Err(crate::error::CoreError::Channel(
                "mock fetch failure".to_string(),
            ));
        }

        let messages = self.messages.lock().unwrap();
        let filtered: Vec<RawMessage> = if let Some(since_dt) = since {
            messages
                .iter()
                .filter(|m| m.timestamp > since_dt)
                .cloned()
                .collect()
        } else {
            messages.clone()
        };

        Ok(filtered)
    }

    async fn send_reply(&self, thread_id: &str, content: &MessageContent) -> Result<()> {
        if *self.fail_send.lock().unwrap() {
            return Err(crate::error::CoreError::Channel(
                "mock send failure".to_string(),
            ));
        }

        self.sent_replies.lock().unwrap().push(SentReply {
            thread_id: thread_id.to_string(),
            content: content.clone(),
            sent_at: Utc::now(),
        });

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        *self.connected.lock().unwrap() = false;
        Ok(())
    }

    fn channel_type(&self) -> Channel {
        self.channel
    }
}
