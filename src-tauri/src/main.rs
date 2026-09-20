//! RustRss 桌面应用入口。
//!
//! 数据层全部来自 `rustrss-core`：界面与 MCP 服务器读同一个库、走同一套查询逻辑。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod mcp_server;
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

/// panic 也要留下痕迹。
///
/// 从终端启动时 stderr 能看见，但用户从启动器点开时什么都没有；
/// 而这个应用会从网络拉不可信内容，崩一次而无现场是很难受的事。
fn install_panic_logger() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let line = format!(
            "[{stamp}] panic: {info}\n  RUST_BACKTRACE={}\n",
            std::env::var("RUST_BACKTRACE").unwrap_or_else(|_| "未启用".into())
        );

        let path = rustrss_core::default_data_dir().join("crash.log");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = file.write_all(line.as_bytes());
            eprintln!("[rustrss] 崩溃已记录到 {}", path.display());
        }
        eprintln!("{line}");
        default_hook(info);
    }));
}

fn main() {
    install_panic_logger();
    let app_state = match AppState::open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[rustrss] 启动失败: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("[rustrss] 数据库: {}", app_state.db_path.display());

    // 启用了 MCP HTTP 服务就在启动时拉起（只绑回环）
    if app_state
        .with_store(|s| Ok(s.bool_setting(mcp_server::K_ENABLED, false).unwrap_or(false)))
        .unwrap_or(false)
    {
        let db_path = app_state.db_path.clone();
        let runtime = app_state.mcp.clone();
        let (port, token) = app_state
            .with_store(|s| {
                Ok((
                    mcp_server::port_from_store(s),
                    mcp_server::token_from_store(s)?,
                ))
            })
            .unwrap_or_else(|_| (mcp_server::DEFAULT_PORT, String::new()));
        let token_prefix: String = token.chars().take(8).collect();
        tauri::async_runtime::spawn(async move {
            match runtime.start(&db_path, token, port).await {
                Ok(addr) => eprintln!(
                    "[rustrss] MCP HTTP 服务: http://{addr}/mcp（仅回环，需 token，前缀 {token_prefix}…）"
                ),
                Err(e) => eprintln!("[rustrss] MCP HTTP 服务启动失败: {e}"),
            }
        });
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
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
            commands::mark_all_unread,
            commands::get_ui_settings,
            commands::set_mark_read_on_navigate,
            commands::set_ui_locale,
            commands::set_ui_theme,
            commands::get_ai_settings,
            commands::save_ai_settings,
            commands::test_ai_connection,
            commands::ai_summarize,
            commands::ai_translate,
            commands::ai_preview,
            commands::set_ai_confirm_before_send,
            commands::get_mcp_settings,
            commands::set_mcp_enabled,
            commands::set_mcp_port,
            commands::rotate_mcp_token,
            commands::add_feed,
            commands::discover_feed,
            commands::remove_feed,
            commands::export_opml,
            commands::import_opml,
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
