use messagehub_core::store::MessageFilter;
use messagehub_core::types::{Channel, ChannelConfig, Message};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

/// Pick the configured channel a reply to `message` should go out on.
///
/// Precedence:
/// 1. If `message.received_on_channel_id` is set (post-migration-008 rows),
///    use the channel with that exact id. If no config matches that id —
///    likely the channel was deleted from `messagehub.toml` since the
///    message was ingested — return an error rather than silently
///    falling back, because a fallback to the wrong account is the bug
///    we're trying to prevent (B-004).
/// 2. If `received_on_channel_id` is `None` (legacy rows from before
///    migration 008): we can't prove which account received it. If
///    exactly one config matches `message.channel`, use it (the
///    overwhelmingly common single-account case is unaffected). If
///    multiple match, error — the user must reply from the new UI on a
///    fresh message.
///
/// Returns the chosen `ChannelConfig` or an error string suitable for
/// surfacing to the UI.
fn resolve_reply_channel(
    message: &Message,
    configs: Vec<ChannelConfig>,
) -> Result<ChannelConfig, String> {
    if let Some(rcv_id) = message.received_on_channel_id {
        return configs
            .into_iter()
            .find(|c| c.id == rcv_id)
            .ok_or_else(|| {
                format!(
                    "Cannot reply: the {} channel this message arrived on (id {}) \
                     is no longer configured",
                    message.channel, rcv_id
                )
            });
    }

    // Legacy row (pre-migration-008): no recorded receiving channel. Safe
    // only when there's exactly one candidate — anything else risks
    // re-introducing B-004.
    let mut matches: Vec<ChannelConfig> = configs
        .into_iter()
        .filter(|c| c.channel == message.channel)
        .collect();
    match matches.len() {
        0 => Err(format!("No {} channel configured to reply on", message.channel)),
        1 => Ok(matches.remove(0)),
        n => Err(format!(
            "Cannot disambiguate which of {} configured {} accounts to reply from \
             (legacy message has no receiving-channel record — open the message \
             again after re-syncing to populate it)",
            n, message.channel
        )),
    }
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    pub id: String,
    pub timestamp: String,
    pub channel: String,
    pub channel_label: Option<String>,
    pub sender_name: String,
    pub subject: Option<String>,
    pub preview: String,
    pub category: Option<String>,
    pub priority: Option<u8>,
    pub is_read: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    pub filename: String,
    pub size_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDetail {
    #[serde(flatten)]
    pub row: MessageRow,
    pub body: String,
    pub html: Option<String>,
    pub thread_id: String,
    pub attachments: Vec<AttachmentInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    pub id: String,
    pub channel_type: String,
    pub label: String,
    pub enabled: bool,
    pub status: String,
    pub last_sync_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiConfig {
    pub db_path: String,
    pub channel_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelCount {
    pub channel_type: String,
    pub total: u64,
    pub unread: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarCounts {
    pub all: u64,
    pub unread: u64,
    pub priority_high: u64,
    pub by_channel: Vec<ChannelCount>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyDraftDto {
    pub thread_id: String,
    pub in_reply_to_message_id: String,
    pub body: String,
    pub subject: Option<String>,
    pub updated_at: String,
}

impl From<&messagehub_core::store::ReplyDraft> for ReplyDraftDto {
    fn from(d: &messagehub_core::store::ReplyDraft) -> Self {
        Self {
            thread_id: d.thread_id.to_string(),
            in_reply_to_message_id: d.in_reply_to_message_id.to_string(),
            body: d.body.clone(),
            subject: d.subject.clone(),
            updated_at: d.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDraftDto {
    pub draft_id: String,
    pub body: String,
    pub confidence: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDraftSummaryDto {
    pub id: String,
    pub created_at: String,
    pub confidence: f32,
    pub preview: String,
    pub body: String,
    pub has_user_edit: bool,
}

impl From<&messagehub_core::store::DraftRecord> for AiDraftSummaryDto {
    fn from(d: &messagehub_core::store::DraftRecord) -> Self {
        let body = d
            .user_edited_output
            .as_deref()
            .unwrap_or(&d.output)
            .to_string();
        let preview: String = body.chars().take(80).collect();
        Self {
            id: d.id.to_string(),
            created_at: d.created_at.clone(),
            confidence: d.confidence,
            preview,
            body,
            has_user_edit: d.user_edited_output.is_some(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatusDto {
    pub configured: bool,
    pub model: Option<String>,
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Build a `MessageRow` DTO from a core `Message`.
///
/// Channel label lookup uses `channel_labels_by_variant` so it works for all
/// channel types regardless of config UUID.
///
/// TODO: disambiguate multi-account by looking at the message's receiving
/// address (e.g. the `To:` header stored in metadata) to pick the matching
/// label when several Email configs exist; deferred to 7b.2.
fn build_message_row(
    msg: &messagehub_core::types::Message,
    state: &AppState,
    sender_name: String,
) -> MessageRow {
    // Pick the first configured label for this channel variant.
    let channel_label = state
        .channel_labels_by_variant
        .get(&msg.channel)
        .and_then(|labels| labels.first())
        .cloned();

    // Build a short preview from the plain-text body (first 200 chars).
    let preview = msg
        .content
        .text
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();

    MessageRow {
        id: msg.id.to_string(),
        timestamp: msg.timestamp.to_rfc3339(),
        channel: msg.channel.to_db_str().to_string(),
        channel_label,
        sender_name,
        subject: msg.content.subject.clone(),
        preview,
        category: msg.category.clone(),
        priority: msg.priority.map(|p| p.value()),
        is_read: msg.is_read,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Filter {
    All,
    Unread,
    PriorityHigh,
    #[serde(rename_all = "camelCase")]
    Channel { channel_type: String },
}

impl Filter {
    fn to_core(&self) -> Result<MessageFilter, String> {
        Ok(match self {
            Filter::All => MessageFilter::default(),
            Filter::Unread => MessageFilter {
                unread_only: true,
                ..Default::default()
            },
            Filter::PriorityHigh => MessageFilter {
                min_priority: Some(4),
                ..Default::default()
            },
            Filter::Channel { channel_type } => {
                let ch = Channel::from_db_str(channel_type)
                    .ok_or_else(|| format!("unknown channel_type: {}", channel_type))?;
                MessageFilter {
                    channel: Some(ch),
                    ..Default::default()
                }
            }
        })
    }
}

// ── commands ──────────────────────────────────────────────────────────────────

/// Return up to `limit` messages starting at `offset`, newest first, scoped
/// by the supplied filter.
#[tauri::command]
pub fn list_messages(
    filter: Filter,
    limit: u32,
    offset: u32,
    state: State<'_, AppState>,
) -> Result<Vec<MessageRow>, String> {
    let core_filter = filter.to_core()?;

    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    let messages = store
        .list_messages(&core_filter, limit, offset)
        .map_err(|e| format!("list_messages failed: {}", e))?;

    let rows = messages
        .iter()
        .map(|msg| {
            let sender_name = store
                .get_contact(&msg.sender_id)
                .map(|c| c.display_name)
                .unwrap_or_else(|_| msg.sender_id.to_string());
            build_message_row(msg, &state, sender_name)
        })
        .collect();

    Ok(rows)
}

/// Return the full message detail (body + attachments) for a given id.
#[tauri::command]
pub fn get_message(id: String, state: State<'_, AppState>) -> Result<MessageDetail, String> {
    let uuid = Uuid::parse_str(&id).map_err(|e| format!("invalid id: {}", e))?;

    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    let msg = store
        .get_message(&uuid)
        .map_err(|e| format!("get_message failed: {}", e))?;

    let sender_name = store
        .get_contact(&msg.sender_id)
        .map(|c| c.display_name)
        .unwrap_or_else(|_| msg.sender_id.to_string());

    let row = build_message_row(&msg, &state, sender_name);

    let attachments = msg
        .content
        .attachments
        .iter()
        .map(|a| AttachmentInfo {
            filename: a.filename.clone(),
            size_bytes: a.size_bytes,
        })
        .collect();

    Ok(MessageDetail {
        row,
        body: msg.content.text.unwrap_or_default(),
        html: msg.content.html,
        thread_id: msg.thread_id.to_string(),
        attachments,
    })
}

/// Return all configured channels.
#[tauri::command]
pub fn list_channels(state: State<'_, AppState>) -> Result<Vec<ChannelInfo>, String> {
    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    let configs = store
        .list_channel_configs()
        .map_err(|e| format!("list_channel_configs failed: {}", e))?;

    let infos = configs
        .into_iter()
        .map(|c| ChannelInfo {
            id: c.id.to_string(),
            channel_type: c.channel.to_db_str().to_string(),
            label: c.label,
            enabled: c.enabled,
            status: c.status.db_str().to_string(),
            last_sync_at: c.last_sync_at.map(|t| t.to_rfc3339()),
        })
        .collect();

    Ok(infos)
}

/// Return UI-level config info (db path + number of configured channels).
#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<UiConfig, String> {
    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    let channel_count = store
        .list_channel_configs()
        .map_err(|e| format!("list_channel_configs failed: {}", e))?
        .len();

    Ok(UiConfig {
        db_path: state.db_path.clone(),
        channel_count,
    })
}

/// Flip the `is_read` flag for a message.
#[tauri::command]
pub fn mark_read(
    id: String,
    read: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = Uuid::parse_str(&id).map_err(|e| format!("invalid id: {}", e))?;

    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    store
        .mark_read(&uuid, read)
        .map_err(|e| format!("mark_read failed: {}", e))
}

/// Return a batched snapshot of sidebar counts: one entry per view + per
/// channel. One SQL COUNT(*) per field; for ~5 channels that's ~13 cheap
/// indexed queries, far less flicker than three-plus separate invokes.
#[tauri::command]
pub fn sidebar_counts(state: State<'_, AppState>) -> Result<SidebarCounts, String> {
    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    let all = store
        .count_messages(&MessageFilter::default())
        .map_err(|e| format!("count all failed: {}", e))?;

    let unread = store
        .count_messages(&MessageFilter {
            unread_only: true,
            ..Default::default()
        })
        .map_err(|e| format!("count unread failed: {}", e))?;

    // Route through Filter::PriorityHigh.to_core() so the ≥4 threshold
    // stays single-sourced in to_core(); .expect is sound because only
    // the Channel branch of to_core can fail.
    let pri_filter = Filter::PriorityHigh
        .to_core()
        .expect("PriorityHigh branch is infallible");
    let priority_high = store
        .count_messages(&pri_filter)
        .map_err(|e| format!("count priorityHigh failed: {}", e))?;

    let configs = store
        .list_channel_configs()
        .map_err(|e| format!("list_channel_configs failed: {}", e))?;

    let mut by_channel = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cfg in &configs {
        if !seen.insert(cfg.channel) {
            // Multiple configs per channel variant (e.g. two Email accounts)
            // roll up to the variant level so 7b.2's sidebar has one row
            // per channel. Multi-account UI is deferred.
            continue;
        }
        let total = store
            .count_messages(&MessageFilter {
                channel: Some(cfg.channel),
                ..Default::default()
            })
            .map_err(|e| format!("count channel {} failed: {}", cfg.channel, e))?;
        let chan_unread = store
            .count_messages(&MessageFilter {
                channel: Some(cfg.channel),
                unread_only: true,
                ..Default::default()
            })
            .map_err(|e| format!("count unread for channel {} failed: {}", cfg.channel, e))?;
        by_channel.push(ChannelCount {
            channel_type: cfg.channel.to_db_str().to_string(),
            total,
            unread: chan_unread,
        });
    }

    Ok(SidebarCounts {
        all,
        unread,
        priority_high,
        by_channel,
    })
}

#[tauri::command]
pub fn save_reply_draft(
    thread_id: String,
    in_reply_to_message_id: String,
    body: String,
    subject: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let thread = Uuid::parse_str(&thread_id).map_err(|e| format!("bad thread_id: {}", e))?;
    let msg = Uuid::parse_str(&in_reply_to_message_id)
        .map_err(|e| format!("bad in_reply_to_message_id: {}", e))?;
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    store
        .upsert_reply_draft(&messagehub_core::store::NewReplyDraft {
            thread_id: thread,
            in_reply_to_message_id: msg,
            body: &body,
            subject: subject.as_deref(),
        })
        .map_err(|e| format!("upsert_reply_draft: {}", e))
}

#[tauri::command]
pub fn get_reply_draft(
    thread_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ReplyDraftDto>, String> {
    let thread = Uuid::parse_str(&thread_id).map_err(|e| format!("bad thread_id: {}", e))?;
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    let draft = store
        .get_reply_draft(&thread)
        .map_err(|e| format!("get_reply_draft: {}", e))?;
    Ok(draft.as_ref().map(ReplyDraftDto::from))
}

#[tauri::command]
pub fn delete_reply_draft(
    thread_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let thread = Uuid::parse_str(&thread_id).map_err(|e| format!("bad thread_id: {}", e))?;
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    store
        .delete_reply_draft(&thread)
        .map_err(|e| format!("delete_reply_draft: {}", e))
}

#[tauri::command]
pub async fn send_email_reply(
    thread_id: String,
    in_reply_to_message_id: String,
    body: String,
    subject: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use messagehub_core::adapters::email::{EmailAdapter, ImapSettings};
    use messagehub_core::adapters::ChannelAdapter;
    use messagehub_core::types::{Channel, MessageContent, ReplyHeaders};

    let thread = Uuid::parse_str(&thread_id).map_err(|e| format!("bad thread_id: {}", e))?;
    let irt = Uuid::parse_str(&in_reply_to_message_id)
        .map_err(|e| format!("bad in_reply_to_message_id: {}", e))?;

    // Gather everything we need under the store lock, then drop it before
    // any .await — MutexGuard is !Send so holding it across await is a
    // compile error anyway, and the codebase's runtime layer follows this
    // pattern consistently.
    let (channel_config, to_addr, in_reply_to_hdr, references_hdr) = {
        let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
        let message = store
            .get_message(&irt)
            .map_err(|e| format!("get_message: {}", e))?;

        if message.channel != Channel::Email {
            return Err("send_email_reply only supports Email channels".into());
        }

        let configs = store
            .list_channel_configs()
            .map_err(|e| format!("list_channel_configs: {}", e))?;
        let channel_cfg = resolve_reply_channel(&message, configs)?;

        let contact = store
            .get_contact(&message.sender_id)
            .map_err(|e| format!("get_contact: {}", e))?;
        let to = contact
            .identities
            .iter()
            .find(|id| id.channel == Channel::Email)
            .map(|id| id.address.clone())
            .ok_or_else(|| {
                "No recipient address known for this contact on Email".to_string()
            })?;

        let original_msg_id = message
            .metadata
            .get("message_id")
            .cloned()
            .ok_or_else(|| {
                "Cannot reply: original message has no Message-ID header".to_string()
            })?;

        let mut references: Vec<String> = message
            .metadata
            .get("references")
            .map(|s| {
                s.split_whitespace()
                    .map(|r| r.trim_matches(|c| c == '<' || c == '>').to_string())
                    .filter(|r| !r.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        references.push(original_msg_id.clone());

        (channel_cfg, to, original_msg_id, references)
    };

    // AppState.email_connections is a sync HashMap; cloning the value is cheap.
    let conn = state
        .email_connections
        .get(&channel_config.id)
        .cloned()
        .ok_or_else(|| {
            "No credentials configured for this channel in messagehub.toml".to_string()
        })?;

    // Subject dedup guard — UI supposedly already applied "Re: " prefix, but
    // belt-and-braces if a future UI path forgets.
    let subject_final = if subject.trim_start().to_ascii_lowercase().starts_with("re:") {
        subject
    } else if subject.is_empty() {
        "Re:".to_string()
    } else {
        format!("Re: {}", subject)
    };

    let content = MessageContent {
        text: Some(body),
        html: None,
        subject: Some(subject_final),
        attachments: Vec::new(),
        reply_headers: Some(ReplyHeaders {
            to: to_addr,
            in_reply_to: in_reply_to_hdr,
            references: references_hdr,
        }),
    };

    // EmailAdapter::connect reads channel_config.keychain_ref as "user:password".
    // The AppState.email_connections stored the credentials separately, so we
    // synthesize a config-for-connect here with the right keychain_ref shape.
    let mut config_for_connect = channel_config.clone();
    config_for_connect.keychain_ref = format!("{}:{}", conn.username, conn.password);

    let mut adapter = EmailAdapter::with_settings(ImapSettings {
        host: conn.imap_host.clone(),
        port: conn.imap_port,
        smtp_host: conn.smtp_host.clone(),
        smtp_port: conn.smtp_port,
    });
    adapter
        .connect(&config_for_connect)
        .await
        .map_err(|e| format!("connect: {}", e))?;
    let send_result = adapter.send_reply("", &content).await;
    let _ = adapter.disconnect().await; // best-effort

    send_result.map_err(|e| format!("smtp send: {}", e))?;

    // Best-effort draft cleanup. The email already left — no recoverable
    // failure from here on propagates to the caller. Both the lock
    // acquisition (poisoning) and the delete itself log-and-swallow.
    match state.store.lock() {
        Ok(store) => {
            if let Err(e) = store.delete_reply_draft(&thread) {
                eprintln!("send_email_reply: delete_reply_draft failed: {}", e);
            }
        }
        Err(e) => {
            eprintln!("send_email_reply: store lock poisoned, skipping draft cleanup: {}", e);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn ai_draft_reply(
    message_id: String,
    redact: bool,
    state: State<'_, AppState>,
) -> Result<AiDraftDto, String> {
    use messagehub_core::ai::cloud::CloudConfig;

    let msg = Uuid::parse_str(&message_id).map_err(|e| format!("bad message_id: {}", e))?;
    let cloud = state
        .cloud
        .clone()
        .ok_or_else(|| "Cloud not configured — add [cloud] to messagehub.toml".to_string())?;

    let store = state.store.clone();
    let outcome = cloud
        .draft_reply_via(store, msg, CloudConfig { redact })
        .await
        .map_err(|e| format!("draft_reply: {}", e))?;

    Ok(AiDraftDto {
        draft_id: outcome.id.to_string(),
        body: outcome.output,
        confidence: outcome.confidence,
    })
}

#[tauri::command]
pub fn list_ai_drafts(
    message_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<AiDraftSummaryDto>, String> {
    let msg = Uuid::parse_str(&message_id).map_err(|e| format!("bad message_id: {}", e))?;
    let store = state.store.lock().map_err(|e| format!("store lock: {}", e))?;
    let rows = store
        .list_drafts_for_message(&msg)
        .map_err(|e| format!("list_drafts_for_message: {}", e))?;
    Ok(rows
        .iter()
        .filter(|r| r.action_type == "draft_reply")
        .map(AiDraftSummaryDto::from)
        .collect())
}

#[tauri::command]
pub fn cloud_config_status(state: State<'_, AppState>) -> Result<CloudStatusDto, String> {
    Ok(CloudStatusDto {
        configured: state.cloud.is_some(),
        model: state.cloud_model.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use messagehub_core::runtime::status::ChannelStatus;
    use messagehub_core::types::{Channel, ChannelConfig, Message, MessageContent};
    use std::collections::HashMap;

    fn cfg(id: Uuid, channel: Channel, label: &str) -> ChannelConfig {
        ChannelConfig {
            id,
            channel,
            label: label.to_string(),
            keychain_ref: "user:pass".to_string(),
            enabled: true,
            poll_interval_secs: 60,
            last_sync_cursor: None,
            last_sync_at: None,
            status: ChannelStatus::Healthy,
            last_error: None,
            consecutive_failures: 0,
        }
    }

    fn msg(channel: Channel, received_on: Option<Uuid>) -> Message {
        Message {
            id: Uuid::new_v4(),
            channel,
            thread_id: Uuid::new_v4(),
            sender_id: Uuid::new_v4(),
            content: MessageContent {
                text: Some("hi".into()),
                html: None,
                subject: None,
                attachments: vec![],
                reply_headers: None,
            },
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
            priority: None,
            category: None,
            is_read: false,
            is_archived: false,
            external_id: None,
            received_on_channel_id: received_on,
        }
    }

    // ── resolve_reply_channel ──────────────────────────────────────────

    #[test]
    fn resolver_picks_received_on_channel_when_set() {
        let work = Uuid::new_v4();
        let personal = Uuid::new_v4();
        let configs = vec![
            cfg(work, Channel::Email, "work"),
            cfg(personal, Channel::Email, "personal"),
        ];
        // Message arrived on the personal account.
        let m = msg(Channel::Email, Some(personal));
        let chosen = resolve_reply_channel(&m, configs).unwrap();
        assert_eq!(chosen.id, personal, "must reply from the receiving account");
        assert_eq!(chosen.label, "personal");
    }

    #[test]
    fn resolver_errors_when_received_on_channel_no_longer_exists() {
        let stale = Uuid::new_v4();
        let other = Uuid::new_v4();
        let configs = vec![cfg(other, Channel::Email, "other")];
        let m = msg(Channel::Email, Some(stale));
        // Don't silently fall back — that would re-introduce B-004.
        assert!(resolve_reply_channel(&m, configs).is_err());
    }

    #[test]
    fn resolver_falls_back_to_single_matching_variant_when_legacy_null() {
        let only = Uuid::new_v4();
        let configs = vec![cfg(only, Channel::Email, "solo")];
        let m = msg(Channel::Email, None); // legacy row, pre-migration-008
        let chosen = resolve_reply_channel(&m, configs).unwrap();
        assert_eq!(chosen.id, only);
    }

    #[test]
    fn resolver_errors_when_legacy_null_and_multiple_variant_matches() {
        let work = Uuid::new_v4();
        let personal = Uuid::new_v4();
        let configs = vec![
            cfg(work, Channel::Email, "work"),
            cfg(personal, Channel::Email, "personal"),
        ];
        let m = msg(Channel::Email, None);
        assert!(
            resolve_reply_channel(&m, configs).is_err(),
            "ambiguous legacy row must error rather than guess"
        );
    }

    #[test]
    fn resolver_errors_when_no_matching_variant_at_all() {
        let tg = Uuid::new_v4();
        let configs = vec![cfg(tg, Channel::Telegram, "tg")];
        let m = msg(Channel::Email, None);
        assert!(resolve_reply_channel(&m, configs).is_err());
    }

    // ── existing tests ────────────────────────────────────────────────

    #[test]
    fn filter_all_maps_to_default() {
        let core = Filter::All.to_core().unwrap();
        assert!(core.channel.is_none());
        assert!(!core.unread_only);
        assert!(core.min_priority.is_none());
        assert!(!core.archived);
    }

    #[test]
    fn filter_unread_sets_flag() {
        let core = Filter::Unread.to_core().unwrap();
        assert!(core.unread_only);
        assert!(core.min_priority.is_none());
    }

    #[test]
    fn filter_priority_high_sets_threshold_to_4() {
        let core = Filter::PriorityHigh.to_core().unwrap();
        assert_eq!(core.min_priority, Some(4));
    }

    #[test]
    fn filter_channel_resolves_known() {
        let core = Filter::Channel {
            channel_type: "Email".into(),
        }
        .to_core()
        .unwrap();
        assert_eq!(core.channel, Some(Channel::Email));
    }

    #[test]
    fn filter_channel_rejects_unknown() {
        let err = Filter::Channel {
            channel_type: "NotAChannel".into(),
        }
        .to_core()
        .unwrap_err();
        assert!(err.contains("unknown channel_type"));
    }

    #[test]
    fn ai_draft_summary_dto_from_draft_record() {
        use messagehub_core::store::DraftRecord;
        let d = DraftRecord {
            id: Uuid::new_v4(),
            message_id: Some(Uuid::new_v4()),
            action_type: "draft_reply".into(),
            input_redacted: "body".into(),
            output: "Thanks!".into(),
            user_edited_output: None,
            confidence: 0.73,
            provider: "anthropic".into(),
            model: "claude-sonnet-4-6".into(),
            created_at: "2026-04-21T10:00:00Z".into(),
        };
        let dto = AiDraftSummaryDto::from(&d);
        assert_eq!(dto.preview, "Thanks!");
        assert_eq!(dto.body, "Thanks!");
        assert!(!dto.has_user_edit);
        assert!((dto.confidence - 0.73).abs() < 1e-6);
    }

    #[test]
    fn ai_draft_summary_uses_user_edit_when_present() {
        use messagehub_core::store::DraftRecord;
        let d = DraftRecord {
            id: Uuid::new_v4(),
            message_id: Some(Uuid::new_v4()),
            action_type: "draft_reply".into(),
            input_redacted: "body".into(),
            output: "original".into(),
            user_edited_output: Some("edited".into()),
            confidence: 0.5,
            provider: "anthropic".into(),
            model: "claude-sonnet-4-6".into(),
            created_at: "2026-04-21T10:00:00Z".into(),
        };
        let dto = AiDraftSummaryDto::from(&d);
        assert_eq!(dto.preview, "edited");
        assert_eq!(dto.body, "edited");
        assert!(dto.has_user_edit);
    }

    #[test]
    fn reply_draft_dto_round_trips() {
        use messagehub_core::store::ReplyDraft;
        use chrono::TimeZone;

        let d = ReplyDraft {
            thread_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            in_reply_to_message_id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            body: "hi".to_string(),
            subject: Some("Re: ping".to_string()),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 4, 21, 10, 0, 0).unwrap(),
        };
        let dto = ReplyDraftDto::from(&d);
        assert_eq!(dto.thread_id, "00000000-0000-0000-0000-000000000001");
        assert_eq!(dto.body, "hi");
        assert_eq!(dto.subject.as_deref(), Some("Re: ping"));
        assert!(dto.updated_at.starts_with("2026-04-21T10:00:00"));
    }
}
