pub mod agent;
pub mod api;
pub mod commands;
pub mod db;
pub mod tools;

use commands::{
    fetch_remote_models, get_api_config, get_global_rules, get_models, get_provider_statuses,
    get_providers, get_selected_model, get_workspace, load_sessions, pull_ollama_model,
    refresh_provider_models, remove_session, remove_workspace, save_session, send_message,
    set_api_config, set_global_rules, set_pinned_models, set_providers, set_selected_model,
    set_workspace, stop_message, AppState,
};
use tauri::Manager;

#[cfg(windows)]
mod win_icon {
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageW(hWnd: isize, msg: u32, wParam: usize, lParam: isize) -> isize;
        fn CreateIconFromResourceEx(
            presbits: *mut u8,
            cb_size: u32,
            is_icon: i32,
            version: u32,
            cx_desired: i32,
            cy_desired: i32,
            ui_flags: u32,
        ) -> isize;
    }

    /// Tauri 仅设置窗口 Small 图标；任务栏/跳转列表使用的 Big 图标缺失时
    /// Windows 会回退到系统默认图标，这里手动补设。
    pub unsafe fn set_taskbar_icon(hwnd: isize, png: &[u8]) {
        let hicon = CreateIconFromResourceEx(
            png.as_ptr() as *mut u8,
            png.len() as u32,
            1,
            0x0003_0000,
            0,
            0,
            0x0000_0040, // LR_DEFAULTSIZE
        );
        if hicon != 0 {
            SendMessageW(hwnd, 0x0080 /* WM_SETICON */, 1 /* ICON_BIG */, hicon);
        }
    }
}

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
        .setup(move |app| {
            #[cfg(any(windows, target_os = "linux"))]
            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::include_image!("icons/128x128.png");
                let _ = window.set_icon(icon);
                #[cfg(windows)]
                if let Ok(hwnd) = window.hwnd() {
                    unsafe {
                        win_icon::set_taskbar_icon(
                            hwnd.0 as isize,
                            include_bytes!("../icons/128x128.png"),
                        );
                    }
                }
            }
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = db::open(&dir.join("aura.db"))?;
            app.manage(AppState::new(api_key, db));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            send_message,
            stop_message,
            remove_session,
            load_sessions,
            save_session,
            set_workspace,
            get_workspace,
            get_global_rules,
            set_global_rules,
            get_models,
            get_selected_model,
            set_selected_model,
            get_api_config,
            set_api_config,
            get_providers,
            set_providers,
            get_provider_statuses,
            refresh_provider_models,
            fetch_remote_models,
            set_pinned_models,
            pull_ollama_model,
            remove_workspace
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
