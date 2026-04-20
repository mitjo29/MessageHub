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
    AppState::init(&cfg.database, &cfg.password)
}
