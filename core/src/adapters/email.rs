use std::collections::HashMap;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mail_parser::MimeHeaders;
use tracing::{debug, info, warn};

use crate::error::{CoreError, Result};
use crate::types::{Channel, ChannelConfig, MessageContent};
use super::{ChannelAdapter, RawAttachment, RawMessage};

/// Email adapter using IMAP for fetching and SMTP for sending.
pub struct EmailAdapter {
    imap_host: Option<String>,
    imap_port: u16,
    smtp_host: Option<String>,
    smtp_port: u16,
    username: Option<String>,
    password: Option<String>,
    connected: bool,
}

/// IMAP connection settings parsed from channel config metadata.
#[derive(Debug, Clone)]
pub struct ImapSettings {
    pub host: String,
    pub port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
}

impl Default for ImapSettings {
    fn default() -> Self {
        Self {
            host: "imap.gmail.com".to_string(),
            port: 993,
            smtp_host: "smtp.gmail.com".to_string(),
            smtp_port: 587,
        }
    }
}

impl EmailAdapter {
    pub fn new() -> Self {
        Self {
            imap_host: None,
            imap_port: 993,
            smtp_host: None,
            smtp_port: 587,
            username: None,
            password: None,
            connected: false,
        }
    }

    pub fn with_settings(settings: ImapSettings) -> Self {
        Self {
            imap_host: Some(settings.host),
            imap_port: settings.port,
            smtp_host: Some(settings.smtp_host),
            smtp_port: settings.smtp_port,
            username: None,
            password: None,
            connected: false,
        }
    }

    /// Parse IMAP settings from the channel config label.
    /// Expected format: "user@example.com" — host is derived from domain.
    fn derive_settings(email: &str) -> ImapSettings {
        let domain = email.split('@').nth(1).unwrap_or("gmail.com");
        match domain {
            "gmail.com" | "googlemail.com" => ImapSettings {
                host: "imap.gmail.com".to_string(),
                port: 993,
                smtp_host: "smtp.gmail.com".to_string(),
                smtp_port: 587,
            },
            "outlook.com" | "hotmail.com" | "live.com" => ImapSettings {
                host: "outlook.office365.com".to_string(),
                port: 993,
                smtp_host: "smtp.office365.com".to_string(),
                smtp_port: 587,
            },
            other => ImapSettings {
                host: format!("imap.{}", other),
                port: 993,
                smtp_host: format!("smtp.{}", other),
                smtp_port: 587,
            },
        }
    }

    async fn imap_fetch_since(
        &self,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<RawMessage>> {
        let host = self.imap_host.as_ref().ok_or_else(|| {
            CoreError::Connection("IMAP host not set".to_string())
        })?;
        let username = self.username.as_ref().ok_or_else(|| {
            CoreError::Connection("username not set".to_string())
        })?;
        let password = self.password.as_ref().ok_or_else(|| {
            CoreError::Connection("password not set".to_string())
        })?;

        // Connect via TLS with tokio compat layer
        use tokio_util::compat::TokioAsyncReadCompatExt;

        let addr = format!("{}:{}", host, self.imap_port);
        let tcp = tokio::net::TcpStream::connect(&addr)
            .await
            .map_err(|e| CoreError::Connection(format!("TCP connect to {} failed: {}", addr, e)))?;

        let tls = async_native_tls::TlsConnector::new();
        let tls_stream = tls
            .connect(host.as_str(), tcp.compat())
            .await
            .map_err(|e| CoreError::Connection(format!("TLS handshake failed: {}", e)))?;

        let client = async_imap::Client::new(tls_stream);

        let mut session = client
            .login(username, password)
            .await
            .map_err(|(e, _)| CoreError::Auth(format!("IMAP login failed: {}", e)))?;

        session
            .select("INBOX")
            .await
            .map_err(|e| CoreError::Channel(format!("INBOX select failed: {}", e)))?;

        // Build IMAP search query
        let search_query = if let Some(since_dt) = since {
            let date_str = since_dt.format("%d-%b-%Y").to_string();
            format!("SINCE {}", date_str)
        } else {
            "ALL".to_string()
        };

        let uids = session
            .uid_search(&search_query)
            .await
            .map_err(|e| CoreError::Channel(format!("IMAP search failed: {}", e)))?;

        if uids.is_empty() {
            let _ = session.logout().await;
            return Ok(vec![]);
        }

        // Limit to most recent 100 UIDs
        let mut uid_list: Vec<u32> = uids.into_iter().collect();
        uid_list.sort();
        let uid_list: Vec<u32> = uid_list.into_iter().rev().take(100).collect();
        let uid_range = uid_list
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let messages_stream = session
            .uid_fetch(&uid_range, "RFC822")
            .await
            .map_err(|e| CoreError::Channel(format!("IMAP fetch failed: {}", e)))?;

        use futures::TryStreamExt;
        let fetched: Vec<_> = messages_stream
            .try_collect()
            .await
            .map_err(|e| CoreError::Channel(format!("IMAP stream failed: {}", e)))?;

        let mut raw_messages = Vec::new();

        for fetch in &fetched {
            let body = match fetch.body() {
                Some(b) => b,
                None => continue,
            };

            let parsed = match mail_parser::MessageParser::default().parse(body) {
                Some(p) => p,
                None => {
                    warn!("failed to parse email body");
                    continue;
                }
            };

            let from = parsed.from();
            let (sender_name, sender_address) = if let Some(from_list) = from {
                let addr = from_list.first();
                match addr {
                    Some(a) => (
                        a.name().unwrap_or("Unknown").to_string(),
                        a.address().unwrap_or("").to_string(),
                    ),
                    None => ("Unknown".to_string(), String::new()),
                }
            } else {
                ("Unknown".to_string(), String::new())
            };

            let message_id = parsed
                .message_id()
                .unwrap_or("")
                .to_string();

            let subject = parsed.subject().map(|s| s.to_string());
            let text_body = parsed.body_text(0).map(|t| t.to_string());
            let html_body = parsed.body_html(0).map(|h| h.to_string());

            let timestamp = parsed
                .date()
                .map(|d| {
                    DateTime::from_timestamp(d.to_timestamp(), 0)
                        .unwrap_or_else(Utc::now)
                })
                .unwrap_or_else(Utc::now);

            // Extract threading headers
            let in_reply_to = parsed
                .in_reply_to()
                .as_text_list()
                .and_then(|list| list.first().map(|s| s.to_string()));

            let references = parsed
                .references()
                .as_text_list()
                .and_then(|list| {
                    if list.is_empty() {
                        None
                    } else {
                        Some(list.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" "))
                    }
                });

            let mut metadata = HashMap::new();
            metadata.insert("message_id".to_string(), message_id.clone());
            if let Some(ref irt) = in_reply_to {
                metadata.insert("in_reply_to".to_string(), irt.clone());
            }
            if let Some(ref refs) = references {
                metadata.insert("references".to_string(), refs.clone());
            }
            if let Some(uid) = fetch.uid {
                metadata.insert("imap_uid".to_string(), uid.to_string());
            }

            let attachments: Vec<RawAttachment> = parsed
                .attachments()
                .map(|a: &mail_parser::MessagePart<'_>| {
                    let ct = a.content_type();
                    let mime = ct
                        .map(|c| {
                            let main = c.ctype();
                            let sub = c.subtype().unwrap_or("octet-stream");
                            format!("{}/{}", main, sub)
                        })
                        .unwrap_or_else(|| "application/octet-stream".to_string());
                    RawAttachment {
                        filename: a.attachment_name().unwrap_or("unnamed").to_string(),
                        mime_type: mime,
                        size_bytes: a.len() as u64,
                    }
                })
                .collect();

            // Thread ID: use References chain root, In-Reply-To, or Message-ID
            let thread_id = references
                .as_ref()
                .and_then(|r| r.split_whitespace().next().map(|s| s.to_string()))
                .or(in_reply_to.clone())
                .unwrap_or_else(|| message_id.clone());

            raw_messages.push(RawMessage {
                external_id: message_id,
                channel: Channel::Email,
                external_thread_id: Some(thread_id),
                sender_name,
                sender_address,
                text: text_body,
                html: html_body,
                subject,
                attachments,
                timestamp,
                metadata,
            });
        }

        let _ = session.logout().await;
        debug!(count = raw_messages.len(), "email messages fetched via IMAP");
        Ok(raw_messages)
    }
}

#[async_trait]
impl ChannelAdapter for EmailAdapter {
    async fn connect(&mut self, config: &ChannelConfig) -> Result<()> {
        let settings = if self.imap_host.is_some() {
            ImapSettings {
                host: self.imap_host.clone().unwrap_or_default(),
                port: self.imap_port,
                smtp_host: self.smtp_host.clone().unwrap_or_default(),
                smtp_port: self.smtp_port,
            }
        } else {
            Self::derive_settings(&config.label)
        };

        self.imap_host = Some(settings.host);
        self.imap_port = settings.port;
        self.smtp_host = Some(settings.smtp_host);
        self.smtp_port = settings.smtp_port;

        // In production, keychain_ref is used to look up credentials.
        // For now we use it directly as "user:password" format.
        let creds = &config.keychain_ref;
        let parts: Vec<&str> = creds.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(CoreError::Auth(
                "credentials must be in 'user:password' format".to_string(),
            ));
        }
        self.username = Some(parts[0].to_string());
        self.password = Some(parts[1].to_string());
        self.connected = true;

        info!(
            imap_host = %self.imap_host.as_deref().unwrap_or(""),
            username = %self.username.as_deref().unwrap_or(""),
            "Email adapter configured"
        );
        Ok(())
    }

    async fn fetch_messages(&self, since: Option<DateTime<Utc>>) -> Result<Vec<RawMessage>> {
        if !self.connected {
            return Err(CoreError::Connection("not connected".to_string()));
        }
        self.imap_fetch_since(since).await
    }

    async fn send_reply(&self, thread_id: &str, content: &MessageContent) -> Result<()> {
        if !self.connected {
            return Err(CoreError::Connection("not connected".to_string()));
        }

        let smtp_host = self.smtp_host.as_ref().ok_or_else(|| {
            CoreError::Connection("SMTP host not set".to_string())
        })?;
        let username = self.username.as_ref().ok_or_else(|| {
            CoreError::Connection("username not set".to_string())
        })?;
        let password = self.password.as_ref().ok_or_else(|| {
            CoreError::Connection("password not set".to_string())
        })?;

        let text = content.text.as_deref().ok_or_else(|| {
            CoreError::InvalidInput("email body text is required".to_string())
        })?;

        let subject = content.subject.as_deref().unwrap_or("Re:");

        let email = lettre::Message::builder()
            .from(username.parse().map_err(|e: lettre::address::AddressError| {
                CoreError::InvalidInput(format!("invalid from address: {}", e))
            })?)
            .to(thread_id.parse().map_err(|e: lettre::address::AddressError| {
                CoreError::InvalidInput(format!("invalid to address: {}", e))
            })?)
            .subject(subject)
            .body(text.to_string())
            .map_err(|e| CoreError::Channel(format!("failed to build email: {}", e)))?;

        use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
        use lettre::transport::smtp::authentication::Credentials;

        let creds = Credentials::new(username.clone(), password.clone());

        let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp_host)
            .map_err(|e| CoreError::Connection(format!("SMTP connect failed: {}", e)))?
            .port(self.smtp_port)
            .credentials(creds)
            .build();

        mailer
            .send(email)
            .await
            .map_err(|e| CoreError::Channel(format!("SMTP send failed: {}", e)))?;

        info!(to = %thread_id, "email sent via SMTP");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        self.username = None;
        self.password = None;
        info!("Email adapter disconnected");
        Ok(())
    }

    fn channel_type(&self) -> Channel {
        Channel::Email
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_adapter_channel_type() {
        let adapter = EmailAdapter::new();
        assert_eq!(adapter.channel_type(), Channel::Email);
    }

    #[test]
    fn test_derive_settings_gmail() {
        let settings = EmailAdapter::derive_settings("user@gmail.com");
        assert_eq!(settings.host, "imap.gmail.com");
        assert_eq!(settings.port, 993);
        assert_eq!(settings.smtp_host, "smtp.gmail.com");
        assert_eq!(settings.smtp_port, 587);
    }

    #[test]
    fn test_derive_settings_outlook() {
        let settings = EmailAdapter::derive_settings("user@outlook.com");
        assert_eq!(settings.host, "outlook.office365.com");
        assert_eq!(settings.smtp_host, "smtp.office365.com");
    }

    #[test]
    fn test_derive_settings_custom_domain() {
        let settings = EmailAdapter::derive_settings("user@company.io");
        assert_eq!(settings.host, "imap.company.io");
        assert_eq!(settings.smtp_host, "smtp.company.io");
    }

    #[tokio::test]
    async fn test_connect_parses_credentials() {
        let mut adapter = EmailAdapter::new();
        let config = ChannelConfig {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Email,
            label: "test@gmail.com".to_string(),
            keychain_ref: "test@gmail.com:app-password-123".to_string(),
            enabled: true,
            poll_interval_secs: 30,
            last_sync_cursor: None,
            last_sync_at: None,
        };

        adapter.connect(&config).await.unwrap();

        assert_eq!(adapter.username.as_deref(), Some("test@gmail.com"));
        assert_eq!(adapter.password.as_deref(), Some("app-password-123"));
        assert_eq!(adapter.imap_host.as_deref(), Some("imap.gmail.com"));
        assert!(adapter.connected);
    }

    #[tokio::test]
    async fn test_connect_rejects_bad_credentials_format() {
        let mut adapter = EmailAdapter::new();
        let config = ChannelConfig {
            id: uuid::Uuid::new_v4(),
            channel: Channel::Email,
            label: "test@gmail.com".to_string(),
            keychain_ref: "no-colon-here".to_string(),
            enabled: true,
            poll_interval_secs: 30,
            last_sync_cursor: None,
            last_sync_at: None,
        };

        let result = adapter.connect(&config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("user:password"));
    }

    #[tokio::test]
    async fn test_fetch_without_connect_fails() {
        let adapter = EmailAdapter::new();
        let result = adapter.fetch_messages(None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }

    #[tokio::test]
    async fn test_disconnect_clears_state() {
        let mut adapter = EmailAdapter::new();
        adapter.username = Some("user".to_string());
        adapter.password = Some("pass".to_string());
        adapter.connected = true;

        adapter.disconnect().await.unwrap();

        assert!(adapter.username.is_none());
        assert!(adapter.password.is_none());
        assert!(!adapter.connected);
    }

    #[test]
    fn test_with_settings() {
        let settings = ImapSettings {
            host: "mail.custom.com".to_string(),
            port: 143,
            smtp_host: "send.custom.com".to_string(),
            smtp_port: 25,
        };
        let adapter = EmailAdapter::with_settings(settings);
        assert_eq!(adapter.imap_host.as_deref(), Some("mail.custom.com"));
        assert_eq!(adapter.imap_port, 143);
        assert_eq!(adapter.smtp_host.as_deref(), Some("send.custom.com"));
        assert_eq!(adapter.smtp_port, 25);
    }
}
