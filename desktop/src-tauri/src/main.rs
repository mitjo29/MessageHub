#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod config;
mod state;

use state::AppState;

fn main() {
    let init_result = try_init();
    match init_result {
        Ok(app_state) => {
            tauri::Builder::default()
                .manage(app_state)
                .invoke_handler(tauri::generate_handler![
                    commands::list_messages,
                    commands::get_message,
                    commands::list_channels,
                    commands::get_config,
                    commands::mark_read,
                    commands::sidebar_counts,
                    commands::save_reply_draft,
                    commands::get_reply_draft,
                    commands::delete_reply_draft,
                    commands::send_email_reply,
                    commands::ai_draft_reply,
                    commands::list_ai_drafts,
                    commands::cloud_config_status,
                ])
                .run(tauri::generate_context!())
                .expect("error while running tauri application");
        }
        Err(err) => {
            // Print to stderr and still open a Tauri window. AppState is not
            // managed, so the commands will return a "state not managed"
            // error when invoked — the frontend's error banner will surface
            // that to the user.
            eprintln!("messagehub-desktop: {}", err);
            tauri::Builder::default()
                .invoke_handler(tauri::generate_handler![
                    commands::list_messages,
                    commands::get_message,
                    commands::list_channels,
                    commands::get_config,
                    commands::mark_read,
                    commands::sidebar_counts,
                    commands::save_reply_draft,
                    commands::get_reply_draft,
                    commands::delete_reply_draft,
                    commands::send_email_reply,
                    commands::ai_draft_reply,
                    commands::list_ai_drafts,
                    commands::cloud_config_status,
                ])
                .run(tauri::generate_context!())
                .expect("error while running tauri application");
        }
    }
}

fn try_init() -> Result<AppState, String> {
    let path = config::resolve_config_path().ok_or_else(|| {
        "messagehub.toml not found (checked ./, ../core/, ../../core/, core/)".to_string()
    })?;
    let cfg = config::load_config(&path)?;

    // `cfg.database` in the TOML is typically "./messagehub.db" — relative to
    // the directory containing messagehub.toml, NOT to the current CWD (which
    // tauri dev sets to desktop/src-tauri/). Resolve it against the config
    // file's parent so the path means what the user meant.
    let db_path = {
        let raw = std::path::Path::new(&cfg.database);
        if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            path.parent().unwrap_or_else(|| std::path::Path::new(".")).join(raw)
        }
    };
    let db_path_str = db_path.to_string_lossy().into_owned();

    eprintln!(
        "messagehub-desktop: config {} → db {}",
        path.display(),
        db_path.display(),
    );

    // Resolve profile_path the same way as `database`: relative paths are
    // anchored at the TOML's parent dir, absolute paths pass through. If
    // the key is absent we pass an empty UserProfile (equivalent to old
    // behavior). UserProfile::load handles missing files gracefully on its
    // own — no extra branch needed here.
    let profile = match cfg.profile_path.as_deref() {
        Some(raw) => {
            let raw_path = std::path::Path::new(raw);
            let resolved = if raw_path.is_absolute() {
                raw_path.to_path_buf()
            } else {
                path.parent().unwrap_or_else(|| std::path::Path::new(".")).join(raw_path)
            };
            eprintln!("messagehub-desktop: profile {}", resolved.display());
            messagehub_core::ai::profile::UserProfile::load(&resolved)
                .map_err(|e| format!("failed to load profile '{}': {}", resolved.display(), e))?
        }
        None => messagehub_core::ai::profile::UserProfile { content: String::new() },
    };

    // Build email-connections map from [[channels]]. Telegram entries are
    // skipped — no send path for them in 7b.3.
    let mut email_connections = std::collections::HashMap::new();
    for entry in &cfg.channels {
        if entry.kind == "email" && entry.enabled {
            let creds: crate::config::EmailCredentials = entry
                .credentials
                .clone()
                .try_into()
                .map_err(|e: toml::de::Error| format!("channel '{}': {}", entry.label, e))?;
            let id = crate::state::stable_channel_id(&entry.kind, &entry.label);
            email_connections.insert(id, crate::state::EmailConnection {
                imap_host: creds.imap_host,
                imap_port: creds.imap_port,
                smtp_host: creds.smtp_host,
                smtp_port: creds.smtp_port,
                username: creds.username,
                password: creds.password,
            });
        }
    }

    AppState::init(&db_path_str, &cfg.password, email_connections, cfg.cloud.as_ref(), profile)
}
