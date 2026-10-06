pub mod agent;
pub mod api;
pub mod commands;
pub mod db;
pub mod tools;

use commands::{
    get_workspace, remove_session, send_message, set_workspace, stop_message, AppState,
};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let api_key = std::env::var("OPENROUTER_API_KEY").ok();
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(api_key))
        .invoke_handler(tauri::generate_handler![
            greet,
            send_message,
            stop_message,
            remove_session,
            set_workspace,
            get_workspace
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
