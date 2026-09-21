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
        .setup(|app| {
            // 托盘不可用是预期内情况（Wayland 无 StatusNotifierItem / 缺
            // libappindicator 等）：显式降级，日志说明，主流程照常。
            // 托盘是否可用决定「关闭到托盘」策略是否允许（见 commands::window_close）。
            match setup_tray(app) {
                Ok(()) => {
                    use tauri::Manager;
                    app.state::<crate::state::AppState>().set_tray_available(true);
                }
                Err(e) => {
                    eprintln!("[rustrss] 托盘不可用，已降级为无托盘模式：{e}");
                }
            }
            // 兜底：窗口以隐藏方式创建，正常由前端在主题/数据就绪后调
            // show_main_window 显示；若前端 5s 仍未就绪（脚本异常等），
            // 强制显示，避免用户面对一个永不出现的窗口。
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(5));
                use tauri::Manager;
                if let Some(win) = handle.get_webview_window("main") {
                    if !win.is_visible().unwrap_or(true) {
                        eprintln!("[rustrss] 前端 5s 未就绪，强制显示主窗口");
                        let _ = win.show();
                    }
                }
            });
            Ok(())
        })
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
            commands::set_read_later,
            commands::mark_all_read,
            commands::mark_all_unread,
            commands::get_ui_settings,
            commands::set_mark_read_on_navigate,
            commands::set_ui_locale,
            commands::set_ui_theme,
            commands::set_ui_close_action,
            commands::add_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::assign_feed_folder,
            commands::set_collapsed_folders,
            commands::show_main_window,
            commands::exit_app,
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_close,
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
        .on_window_event(|window, event| {
            // 拦截系统层关闭（如 Alt+F4）：按设置退出或隐藏到托盘。
            // 与 commands::window_close（三键）同一套策略，托盘不可用时强制退出。
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                use tauri::Manager;
                let state = window.app_handle().state::<crate::state::AppState>();
                let close_to_tray = state
                    .with_store(|s| {
                        Ok(commands::normalize_close_action(&crate::ai::non_empty_setting(s, commands::KEY_CLOSE_ACTION).unwrap_or_default()) == "tray")
                    })
                    .unwrap_or(false)
                    && state.tray_available();
                if close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("RustRss 启动失败");
}

/// 构建系统托盘：图标 + 「显示/隐藏窗口」「退出」菜单。任一步失败都原样
/// 返回错误，由调用方统一降级，不在托盘内部自行吞错。
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::TrayIconBuilder;
    use tauri::Manager;

    // 菜单文案跟随设置里的界面语言；`auto`/读不到时按 zh-CN 处理
    //（精确跟随系统语言需引入 sys-locale 依赖，v1 不做）。
    let locale = app
        .try_state::<AppState>()
        .map(|s| {
            s.with_store(|st| {
                Ok(crate::ai::non_empty_setting(st, commands::KEY_LOCALE)
                    .unwrap_or_default())
            })
            .unwrap_or_default()
        })
        .unwrap_or_default();
    let (toggle_label, quit_label) = if locale == "en" {
        ("Show/Hide Window", "Quit")
    } else {
        ("显示/隐藏窗口", "退出")
    };

    let toggle = MenuItemBuilder::with_id("tray-toggle", toggle_label).build(app)?;
    let quit = MenuItemBuilder::with_id("tray-quit", quit_label).build(app)?;
    let menu = MenuBuilder::new(app).items(&[&toggle, &quit]).build()?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip("RustRss");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    app.on_menu_event(|app, event| match event.id().as_ref() {
        "tray-toggle" => {
            if let Some(win) = app.get_webview_window("main") {
                let visible = win.is_visible().unwrap_or(false);
                let _ = if visible { win.hide() } else { win.show() };
            }
        }
        "tray-quit" => app.exit(0),
        _ => {}
    });
    Ok(())
}
