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

/// MCP 启动日志行：**任何 token 片段都不进日志**。
///
/// 迁移前这里打的是 token 前 8 位前缀；日志文件是要用户贴出来排障的，凭据片段同样
/// 算凭据，所以 token 只用来回答「是否已设鉴权」这一个布尔问题。
fn mcp_startup_line(addr: &str, token: &str) -> String {
    format!(
        "[rustrss] MCP HTTP 服务: http://{addr}/mcp（仅回环，{}）",
        if token.is_empty() {
            "无 token"
        } else {
            "需 token 鉴权"
        }
    )
}

/// 启动最早期接入日志：先建本次日志文件（默认 `info`）、装 panic hook，再开库。
///
/// 返回本次日志文件路径；失败只提示一次并返回 `None`（降级为无日志，不阻断启动）。
/// 级别先按默认 `info`——此时设置还读不到（`log.level` 在 SQLite 里），开库后立刻覆盖。
fn init_logging() -> Option<std::path::PathBuf> {
    let logs_dir = rustrss_core::paths::logs_dir();
    let path = match rustrss_core::logging::init(&logs_dir, log::LevelFilter::Info) {
        Ok(path) => path,
        Err(e) => {
            // 本文件唯一保留的 stderr 输出：它的存在前提正是「logger 装不上」
            eprintln!("[rustrss] 日志初始化失败（本次不写日志文件，应用继续启动）: {e}");
            return None;
        }
    };
    // panic 现场进同一份日志；logger 没装上时它静默无事，原 hook 照旧执行
    rustrss_core::logging::install_panic_hook();
    // 保留策略每次启动跑一次（幂等）：清理结果进日志，便于核对「超限后重启是否真的删了」
    let pruned = rustrss_core::logging::prune(
        &logs_dir,
        rustrss_core::logging::KEEP_FILES,
        rustrss_core::logging::MAX_TOTAL_BYTES,
    );
    log::info!("[rustrss] 本次日志文件: {}", path.display());
    if pruned.removed > 0 {
        log::info!(
            "[rustrss] 日志保留清理: 删除 {} 个（剩 {} 个 / {} 字节）",
            pruned.removed,
            pruned.remaining_files,
            pruned.remaining_bytes
        );
    } else {
        log::debug!(
            "[rustrss] 日志保留清理: 无需删除（剩 {} 个 / {} 字节）",
            pruned.remaining_files,
            pruned.remaining_bytes
        );
    }
    Some(path)
}

fn main() {
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
            // 日志最早接入：早于开库（开库失败正是最需要现场的时刻）。降级不阻断启动。
            let _log_file = init_logging();

            // 恢复落地的唯一保证路径：在**任何连接**（Store / MCP）打开之前把暂存的库换上去。
            // 必须在 AppState::open() 之前——连接一开就出现 WAL 边车与文件锁，替换不再安全。
            let db_path = rustrss_core::resolve_db_path();
            match rustrss_core::backup::apply_pending_restore(&db_path) {
                Ok(true) => log::info!("[rustrss] 已应用暂存的数据库恢复: {}", db_path.display()),
                Ok(false) => {}
                // 打不开/换不上都继续启动：现库还在（或在 .bak-* 里），让用户先把界面用起来。
                Err(e) => {
                    log::warn!("[rustrss] 应用暂存的数据库恢复失败（继续用现有库）: {e}")
                }
            }

            let app_state = match AppState::open() {
                Ok(s) => s,
                Err(e) => {
                    log::error!("[rustrss] 启动失败: {e}");
                    return Err(e.into());
                }
            };
            log::info!("[rustrss] 数据库: {}", app_state.db_path.display());
            // 开库后立刻落实 `log.level`（该键由 T3 提供写路径与界面；现在读不到 → 默认 info）。
            // 非法值归一化在 commands::log_level_filter 里（与 T3 的设置写入共用）。
            let level_setting = app_state
                .with_store(|s| {
                    Ok(crate::ai::non_empty_setting(s, commands::KEY_LOG_LEVEL).unwrap_or_default())
                })
                .unwrap_or_default();
            log::set_max_level(commands::log_level_filter(&level_setting));
            log::info!(
                "[rustrss] 日志级别: {}（设置 log.level={:?}）",
                log::max_level(),
                level_setting
            );
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
                tauri::async_runtime::spawn(async move {
                    match runtime.start(&db_path, token.clone(), port).await {
                        Ok(addr) => log::info!("{}", mcp_startup_line(&addr.to_string(), &token)),
                        Err(e) => log::error!("[rustrss] MCP HTTP 服务启动失败: {e}"),
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
                    log::warn!("[rustrss] 托盘不可用，已降级为无托盘模式：{e}");
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
                        log::warn!("[rustrss] 前端 5s 未就绪，强制显示主窗口");
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
            commands::set_feed_config,
            commands::set_refresh_on_start,
            commands::set_refresh_concurrency,
            commands::set_notify_new_articles,
            commands::set_log_level,
            commands::set_ui_locale,
            commands::set_ui_theme,
            commands::set_list_sort,
            commands::set_list_hide_read,
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
            commands::set_ai_reasoning_effort,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// MCP 启动日志行不得带任何 token 片段。
    ///
    /// 迁移前这里是 token 前 8 位前缀；日志文件会被用户贴进 issue 排障，所以凭据
    /// （哪怕片段）一律不落盘——token 只用来回答「是否已设鉴权」。
    #[test]
    fn mcp_startup_line_carries_no_token_material() {
        let token = "9f3c1a7b2e5d4c6f8a0b1c2d3e4f5a6b";
        let line = mcp_startup_line("127.0.0.1:8637", token);
        for probe in [token, &token[..8], &token[..6]] {
            assert!(
                !line.contains(probe),
                "日志行不得含 token 片段 {probe:?}: {line}"
            );
        }
        assert!(line.contains("127.0.0.1:8637"), "地址仍要能看见：{line}");
        assert!(line.contains("需 token 鉴权"), "鉴权状态仍要能看见：{line}");
        // 未设 token（实际不会发生：token_from_store 会现生成）如实标注，不空白
        assert!(mcp_startup_line("127.0.0.1:8637", "").contains("无 token"));
    }

    /// 迁移收口的机械断言：这 5 个文件里不得再有 `println!/eprintln!`。
    ///
    /// 唯一例外是 `init_logging` 的降级提示：那一行执行时全局 logger 还不存在，
    /// 只能走 stderr（PRD「初始化失败降级为无日志，不崩溃、不阻塞启动」）。
    #[test]
    fn migrated_files_have_no_stray_std_prints() {
        const FILES: &[(&str, &str)] = &[
            ("main.rs", include_str!("main.rs")),
            ("commands.rs", include_str!("commands.rs")),
            ("scheduler.rs", include_str!("scheduler.rs")),
            ("notify.rs", include_str!("notify.rs")),
            (
                "discover.rs",
                include_str!("../../crates/rustrss-core/src/discover.rs"),
            ),
        ];
        const ALLOWED: &str = "日志初始化失败";

        let mut stray = Vec::new();
        for (name, src) in FILES {
            for (idx, line) in src.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue; // 注释/文档里的举例不是调用
                }
                if print_call_count(line) == 0 {
                    continue;
                }
                // 降级提示是刻意保留的（logger 装不上时才走到这里）
                if line.contains("eprintln!") && line.contains(ALLOWED) {
                    continue;
                }
                stray.push(format!("{name}:{}: {}", idx + 1, line.trim_start()));
            }
        }
        assert!(
            stray.is_empty(),
            "迁移未收口（这些行仍是 println!/eprintln!）:\n{stray:#?}"
        );
    }

    /// 一行里真正的打印调用次数（`eprintln!` 含子串 `println!`，只数一次）；
    /// 字符串字面量与行注释里的举例不算（本测试源码自己就有 `"eprintln!"` 这类文本）。
    fn print_call_count(line: &str) -> usize {
        let code = code_only(line);
        let eprint = code.matches("eprintln!").count();
        let plain = code.replace("eprintln!", "").matches("println!").count();
        eprint + plain
    }

    /// 去掉字符串字面量与行注释后的代码片段（够用即可：这些文件是普通 Rust 代码）。
    fn code_only(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut in_str = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if in_str {
                if c == '\\' {
                    let _ = chars.next(); // 转义：连同下一个字符一起丢掉
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '/' if chars.peek() == Some(&'/') => break, // 行注释：后面都不是代码
                _ => out.push(c),
            }
        }
        out
    }
}
