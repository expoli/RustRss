//! RustRss 桌面应用入口。
//!
//! 数据层全部来自 `rustrss-core`：界面与 MCP 服务器读同一个库、走同一套查询逻辑。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod mcp_server;
mod notify;
mod scheduler;
mod state;
mod tray;

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

    let mut builder = tauri::Builder::default();
    // 单实例锁：只在默认库下注册。
    // 第二个实例启动时插件在本进程（已有实例）里跑回调——把主窗口唤出来
    // （可能正藏在托盘里），然后让新进程自己退出；否则两个进程会抢同一个
    // MCP 端口、双写同一个 SQLite。
    // `RUSTSS_DB`/参数指向其他库时不注册：多开诊断副本是合法用法。
    // 必须放在 builder 链最前：插件按注册顺序执行，放后面新进程会先跑完
    // 其他插件与应用 setup 才退出（上游 README 明确要求 first）。
    // 判断用 resolve_db_path()（与 AppState::open 同源），此时还没开库。
    if rustrss_core::paths::is_default_db(&rustrss_core::resolve_db_path()) {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }));
    }

    // 后续所有重活（恢复落地/开库/MCP）都在 .setup() 里做——单实例插件
    // 已在链首生效：第二个进程在跑到 setup 之前就被退出了，
    // 不会再开一次库、也不会再抢一次 MCP 端口（原先 main() 直开的两条残留日志
    // 与「staging 后开二实例会在实例 1 运行中执行替换」的窗口一并消除）。
    // Tauri 2 里 setup 先于配置窗口创建，状态 manage 先于前端首个 invoke。
    builder
        .setup(|app| {
            // 恢复落地的唯一保证路径：在**任何连接**（Store / MCP）打开之前把暂存的库换上去。
            // 必须在 AppState::open() 之前——连接一开就出现 WAL 边车与文件锁，替换不再安全。
            let db_path = rustrss_core::resolve_db_path();
            match rustrss_core::backup::apply_pending_restore(&db_path) {
                Ok(true) => {
                    eprintln!("[rustrss] 已应用暂存的数据库恢复: {}", db_path.display())
                }
                Ok(false) => {}
                // 打不开/换不上都继续启动：现库还在（或在 .bak-* 里），让用户先把界面用起来。
                Err(e) => eprintln!("[rustrss] 应用暂存的数据库恢复失败（继续用现有库）: {e}"),
            }

            let app_state = match AppState::open() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[rustrss] 启动失败: {e}");
                    return Err(e.into());
                }
            };
            eprintln!("[rustrss] 数据库: {}", app_state.db_path.display());
            use tauri::Manager;
            app.manage(app_state);

            // 启用了 MCP HTTP 服务就在启动时拉起（只绑回环）
            let app_state = app.state::<AppState>();
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

            // 托盘不可用是预期内情况（Wayland 无 StatusNotifierItem / 缺
            // libappindicator 等）：显式降级，日志说明，主流程照常。
            // 托盘是否可用决定「关闭到托盘」策略是否允许（见 commands::window_close）。
            match crate::tray::setup_tray(app) {
                Ok(()) => {
                    use tauri::Manager;
                    app.state::<crate::state::AppState>().set_tray_available(true);
                }
                Err(e) => {
                    eprintln!("[rustrss] 托盘不可用，已降级为无托盘模式：{e}");
                }
            }
            // 自动刷新调度器：定时（间隔可配）+ 启动后延迟 10s 一次。
            // 与手动刷新共用同一条管线（commands::refresh_core），各自受单 flight 保护。
            crate::scheduler::spawn(app.handle().clone());
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
        // 系统通知（后台刷新抓到新文章时用；文案与开关见 notify.rs / commands.rs）
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::db_info,
            commands::sidebar_data,
            commands::list_feeds,
            commands::list_entries,
            commands::get_entry,
            commands::fetch_fulltext,
            commands::search,
            commands::set_read,
            commands::set_starred,
            commands::set_read_later,
            commands::mark_all_read,
            commands::mark_all_unread,
            commands::get_ui_settings,
            commands::set_mark_read_on_navigate,
            commands::set_refresh_interval,
            commands::set_feed_refresh_interval,
            commands::set_refresh_on_start,
            commands::set_refresh_concurrency,
            commands::set_notify_new_articles,
            commands::set_ui_locale,
            commands::set_ui_theme,
            commands::set_ui_close_action,
            commands::set_font_config,
            commands::list_font_families,
            commands::get_rsshub_mirror,
            commands::set_rsshub_mirror,
            commands::test_rsshub_mirror,
            commands::preview_rsshub_migration,
            commands::migrate_rsshub_feeds,
            commands::list_folders,
            commands::get_collapsed_folders,
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
            commands::backup_db,
            commands::restore_db,
            commands::refresh_all,
            commands::refresh_feeds,
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
