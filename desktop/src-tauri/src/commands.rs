use serde::Serialize;
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

// ── commands ──────────────────────────────────────────────────────────────────

/// Return up to `limit` messages starting at `offset`, newest first.
#[tauri::command]
pub fn list_messages(
    limit: u32,
    offset: u32,
    state: State<'_, AppState>,
) -> Result<Vec<MessageRow>, String> {
    let store = state
        .store
        .lock()
        .map_err(|e| format!("store lock poisoned: {}", e))?;

    // TODO(Plan 7b.2 Task 4): accept Filter from frontend instead of defaulting.
    let messages = store
        .list_messages(&messagehub_core::store::MessageFilter::default(), limit, offset)
        .map_err(|e| format!("list_messages failed: {}", e))?;

    let rows = messages
        .iter()
        .map(|msg| {
            // Resolve sender name: look up the contact; fall back to sender_id string.
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
