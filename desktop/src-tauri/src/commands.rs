use messagehub_core::store::MessageFilter;
use messagehub_core::types::Channel;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

// ── DTOs ──────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
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
pub struct AttachmentInfo {
    pub filename: String,
    pub size_bytes: u64,
}

#[derive(Serialize)]
pub struct MessageDetail {
    #[serde(flatten)]
    pub row: MessageRow,
    pub body: String,
    pub html: Option<String>,
    pub thread_id: String,
    pub attachments: Vec<AttachmentInfo>,
}

#[derive(Serialize)]
pub struct ChannelInfo {
    pub id: String,
    pub channel_type: String,
    pub label: String,
    pub enabled: bool,
    pub status: String,
    pub last_sync_at: Option<String>,
}

#[derive(Serialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
