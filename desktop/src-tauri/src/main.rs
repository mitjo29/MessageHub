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
            // For 7b.1 we print to stderr and also open a Tauri window with a
            // plain error message. Commands will be unreachable because the
            // state never registered, but the user sees *something*.
            eprintln!("messagehub-desktop: {}", err);
            tauri::Builder::default()
                .manage(InitError(err))
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

struct InitError(String);

fn try_init() -> Result<AppState, String> {
    let path = config::resolve_config_path()
        .ok_or_else(|| "messagehub.toml not found (checked ./ and ../core/)".to_string())?;
    let cfg = config::load_config(&path)?;
    AppState::init(&cfg.database, &cfg.password)
}
