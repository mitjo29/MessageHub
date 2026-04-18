//! Stub — filled in by Task 4.

use async_trait::async_trait;

use crate::error::Result;

#[async_trait]
pub trait CloudProvider: Send + Sync {
    async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String>;
}

pub struct AnthropicCloud;
