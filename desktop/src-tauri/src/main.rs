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

    AppState::init(&db_path_str, &cfg.password)
}
