//! RustRss 桌面应用入口。
//!
//! 数据层全部来自 `rustrss-core`：界面与 MCP 服务器读同一个库、走同一套查询逻辑。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use tauri_plugin_clipboard_manager::ClipboardExt;

use state::AppState;

#[tauri::command]
fn clip_write(app: tauri::AppHandle, text: String) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[tauri::command]
fn clip_read(app: tauri::AppHandle) -> Result<String, String> {
    app.clipboard().read_text().map_err(|e| e.to_string())
}

fn main() {
    let app_state = match AppState::open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[rustrss] 启动失败: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("[rustrss] 数据库: {}", app_state.db_path.display());

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::db_info,
            commands::list_feeds,
            commands::list_entries,
            commands::get_entry,
            commands::search,
            commands::set_read,
            commands::set_starred,
            commands::mark_all_read,
            commands::add_feed,
            commands::remove_feed,
            commands::refresh_all,
            commands::refresh_feed,
            commands::open_external,
            commands::ui_log,
            clip_write,
            clip_read,
        ])
        .run(tauri::generate_context!())
        .expect("RustRss 启动失败");
}
