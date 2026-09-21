//! Tauri 命令：界面与 Rust 之间的全部接口。
//!
//! 约定：
//! - 所有命令返回 `Result<_, String>`，错误信息直接可显示给用户；
//! - 参数名在 JS 侧用 camelCase（Tauri 会自动映射到 snake_case）；
//! - 异步命令**不在 await 期间持有数据库锁**——抓取与写库分成两段。

use serde::Serialize;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use rustrss_core::ai::prompt::{AiTask, SummaryLength};
use rustrss_core::ai::{AiClient, AiRequest, AiTaskPlan, CachePolicy};
use rustrss_core::discover::{discover, Discovery};
use rustrss_core::fetch::RefreshReport;
use rustrss_core::{EntryQuery, EntryRow, FeedRow, MarkScope};

use crate::state::AppState;

type R<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbInfo {
    pub db_path: String,
    pub feeds: i64,
    pub entries: i64,
    pub unread: i64,
    pub starred: i64,
    pub later: i64,
}

#[tauri::command]
pub fn db_info(state: State<'_, AppState>) -> R<DbInfo> {
    state.with_store(|s| {
        Ok(DbInfo {
            db_path: state.db_path.display().to_string(),
            feeds: s.list_feeds().map_err(err)?.len() as i64,
            entries: s.entry_count().map_err(err)?,
            unread: s.unread_total().map_err(err)?,
            starred: s.starred_total().map_err(err)?,
            later: s.read_later_total().map_err(err)?,
        })
    })
}

#[tauri::command]
pub fn list_feeds(state: State<'_, AppState>) -> R<Vec<FeedRow>> {
    state.with_store(|s| s.list_feeds().map_err(err))
}

#[tauri::command]
pub fn list_entries(
    state: State<'_, AppState>,
    feed_id: Option<i64>,
    unread_only: Option<bool>,
    starred_only: Option<bool>,
    read_later_only: Option<bool>,
    limit: Option<u32>,
) -> R<Vec<EntryRow>> {
    let query = EntryQuery {
        feed_id,
        unread_only: unread_only.unwrap_or(false),
        starred_only: starred_only.unwrap_or(false),
        read_later_only: read_later_only.unwrap_or(false),
        limit: Some(limit.unwrap_or(200)),
    };
    state.with_store(|s| s.list_entries(&query).map_err(err))
}

#[tauri::command]
pub fn get_entry(state: State<'_, AppState>, id: i64) -> R<Option<EntryRow>> {
    state.with_store(|s| s.get_entry(id).map_err(err))
}

#[tauri::command]
pub fn search(state: State<'_, AppState>, query: String, limit: Option<u32>) -> R<Vec<EntryRow>> {
    state.with_store(|s| s.search(&query, limit.unwrap_or(100)).map_err(err))
}

#[tauri::command]
pub fn set_read(state: State<'_, AppState>, ids: Vec<i64>, read: bool) -> R<usize> {
    state.with_store(|s| s.set_read(&ids, read).map_err(err))
}

/// 稍后读标记：与已读/星标独立。
#[tauri::command]
pub fn set_read_later(state: State<'_, AppState>, ids: Vec<i64>, read_later: bool) -> R<usize> {
    state.with_store(|s| s.set_read_later(&ids, read_later).map_err(err))
}

#[tauri::command]
pub fn set_starred(state: State<'_, AppState>, ids: Vec<i64>, starred: bool) -> R<usize> {
    state.with_store(|s| s.set_starred(&ids, starred).map_err(err))
}

/// 设置键：`j`/`k` 浏览时是否顺便标记已读。
/// 默认值只在 Rust 这一处定义，界面只负责显示与切换，避免两边各写一份而漂移。
const KEY_MARK_READ_ON_NAVIGATE: &str = "ui.mark_read_on_navigate";
const DEFAULT_MARK_READ_ON_NAVIGATE: bool = true;
/// 界面语言：`auto`（跟随系统）/ `zh-CN` / `en`
pub(crate) const KEY_LOCALE: &str = "ui.locale";
const DEFAULT_LOCALE: &str = "auto";
/// 主题：`system`（跟随系统）/ `light` / `dark`
const KEY_THEME: &str = "ui.theme";
const DEFAULT_THEME: &str = "system";
/// 关闭按钮行为：`exit`（退出程序）/ `tray`（最小化到托盘）
pub(crate) const KEY_CLOSE_ACTION: &str = "ui.close_action";
const DEFAULT_CLOSE_ACTION: &str = "exit";

#[derive(Serialize)]
pub struct UiSettings {
    pub mark_read_on_navigate: bool,
    pub locale: String,
    pub theme: String,
    pub close_action: String,
}

fn ui_settings(state: &AppState) -> R<UiSettings> {
    state.with_store(|s| {
        Ok(UiSettings {
            mark_read_on_navigate: s
                .bool_setting(KEY_MARK_READ_ON_NAVIGATE, DEFAULT_MARK_READ_ON_NAVIGATE)
                .map_err(err)?,
            locale: crate::ai::non_empty_setting(s, KEY_LOCALE)
                .unwrap_or_else(|| DEFAULT_LOCALE.to_string()),
            // 主题白名单在读取时也兜底：库里被写坏也不至于把界面弄没颜色
            theme: crate::ai::non_empty_setting(s, KEY_THEME)
                .map(|v| normalize_theme(&v).to_string())
                .unwrap_or_else(|| DEFAULT_THEME.to_string()),
            close_action: crate::ai::non_empty_setting(s, KEY_CLOSE_ACTION)
                .map(|v| normalize_close_action(&v).to_string())
                .unwrap_or_else(|| DEFAULT_CLOSE_ACTION.to_string()),
        })
    })
}

#[tauri::command]
pub fn get_ui_settings(state: State<'_, AppState>) -> R<UiSettings> {
    ui_settings(&state)
}

#[tauri::command]
pub fn set_mark_read_on_navigate(state: State<'_, AppState>, enabled: bool) -> R<UiSettings> {
    state.with_store(|s| {
        s.set_bool_setting(KEY_MARK_READ_ON_NAVIGATE, enabled)
            .map_err(err)?;
        Ok(())
    })?;
    ui_settings(&state)
}

/// 语言白名单：仅 `zh-CN` / `en`，其余（含 `auto` 与拼错值）一律归 `auto`。
fn normalize_locale(value: &str) -> &str {
    match value.trim() {
        "zh-CN" => "zh-CN",
        "en" => "en",
        _ => DEFAULT_LOCALE,
    }
}

/// 主题白名单：仅 `light` / `dark`，其余（含 `system` 与拼错值）一律归 `system`。
fn normalize_theme(value: &str) -> &str {
    match value.trim() {
        "light" => "light",
        "dark" => "dark",
        _ => DEFAULT_THEME,
    }
}

/// 语言：`auto` / `zh-CN` / `en`。非法值一律归为 `auto`，不让拼错的设置把界面卡死。
#[tauri::command]
pub fn set_ui_locale(state: State<'_, AppState>, locale: String) -> R<UiSettings> {
    state
        .with_store(|s| s.set_setting(KEY_LOCALE, normalize_locale(&locale)).map_err(err))?;
    ui_settings(&state)
}

/// 主题：`system`（跟随系统）/ `light` / `dark`。非法值归为 `system`，同 locale 的白名单口径。
#[tauri::command]
pub fn set_ui_theme(state: State<'_, AppState>, theme: String) -> R<UiSettings> {
    state.with_store(|s| s.set_setting(KEY_THEME, normalize_theme(&theme)).map_err(err))?;
    ui_settings(&state)
}

/// 关闭按钮行为白名单：`exit`（退出程序）/ `tray`（最小化到托盘）。
/// 托盘不可用时即使选了 `tray` 也强制走退出，避免窗口被藏起后找不回。
pub(crate) fn normalize_close_action(value: &str) -> &'static str {
    if value.trim() == "tray" { "tray" } else { "exit" }
}

/// 应用退出（绕过关闭行为拦截，用于托盘菜单「退出」与三键的退出分支）。
#[tauri::command]
pub fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// 最小化 / 最大化切换 / 关闭（自绘标题栏三键）。
#[tauri::command]
pub fn window_minimize(app: tauri::AppHandle) -> R<()> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("main") {
        win.minimize().map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub fn window_toggle_maximize(app: tauri::AppHandle) -> R<()> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("main") {
        if win.is_maximized().map_err(err)? {
            win.unmaximize().map_err(err)?;
        } else {
            win.maximize().map_err(err)?;
        }
    }
    Ok(())
}

/// 关闭按钮：按设置走退出或隐藏到托盘。**每次都实时读库**，设置改完立即生效。
#[tauri::command]
pub fn window_close(app: tauri::AppHandle, state: State<'_, AppState>) -> R<()> {
    use tauri::Manager;
    let close_to_tray = state
        .with_store(|s| Ok(normalize_close_action(&crate::ai::non_empty_setting(s, KEY_CLOSE_ACTION).unwrap_or_default()) == "tray"))
        .unwrap_or(false)
        && state.tray_available();
    if close_to_tray {
        if let Some(win) = app.get_webview_window("main") {
            win.hide().map_err(err)?;
        }
    } else {
        app.exit(0);
    }
    Ok(())
}

// ---------------- 文件夹管理（侧栏分组） ----------------

#[tauri::command]
pub fn add_folder(state: State<'_, AppState>, name: String) -> R<i64> {
    state.with_store(|s| s.add_folder(&name).map_err(err))
}

#[tauri::command]
pub fn rename_folder(state: State<'_, AppState>, folder_id: i64, name: String) -> R<()> {
    state.with_store(|s| s.rename_folder(folder_id, &name).map_err(err))
}

#[tauri::command]
pub fn delete_folder(state: State<'_, AppState>, folder_id: i64) -> R<()> {
    state.with_store(|s| s.delete_folder(folder_id).map_err(err))
}

#[tauri::command]
pub fn assign_feed_folder(state: State<'_, AppState>, feed_id: i64, folder_id: Option<i64>) -> R<()> {
    state.with_store(|s| s.assign_folder(feed_id, folder_id).map_err(err))
}

/// 侧栏分组折叠状态（哪些组被折叠），JSON 序列化后存 settings，跨会话保持。
#[tauri::command]
pub fn set_collapsed_folders(state: State<'_, AppState>, ids: Vec<i64>) -> R<()> {
    let json = serde_json::to_string(&ids).map_err(err)?;
    state.with_store(|s| s.set_setting("ui.folders_collapsed", &json).map_err(err))
}

fn collapsed_folders_setting(store: &rustrss_core::Store) -> Vec<i64> {
    crate::ai::non_empty_setting(store, "ui.folders_collapsed")
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// 显示主窗口：窗口以 `visible: false` 创建，前端完成主题/数据初始化后调用，
/// 保证首帧即正确主题（防主题闪变 FOUC）。兜底定时器见 main.rs 的 setup。
#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) -> R<()> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("main") {
        win.show().map_err(err)?;
    }
    Ok(())
}

/// 关闭行为设置（同 window_close 的实时读库语义，写库后下次关闭即生效）。
#[tauri::command]
pub fn set_ui_close_action(state: State<'_, AppState>, action: String) -> R<UiSettings> {
    state
        .with_store(|s| s.set_setting(KEY_CLOSE_ACTION, normalize_close_action(&action)).map_err(err))?;
    ui_settings(&state)
}

fn scope_of(feed_id: Option<i64>) -> MarkScope {
    match feed_id {
        Some(id) => MarkScope::Feed(id),
        None => MarkScope::All,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_whitelist() {
        assert_eq!(normalize_locale("zh-CN"), "zh-CN");
        assert_eq!(normalize_locale(" en "), "en");
        assert_eq!(normalize_locale("auto"), "auto");
        assert_eq!(normalize_locale("fr"), "auto");
        assert_eq!(normalize_locale(""), "auto");
    }

    #[test]
    fn theme_whitelist() {
        assert_eq!(normalize_theme("light"), "light");
        assert_eq!(normalize_theme(" dark "), "dark");
        assert_eq!(normalize_theme("system"), "system");
        assert_eq!(normalize_theme("blue"), "system");
        assert_eq!(normalize_theme(""), "system");
    }
}

#[tauri::command]
pub fn mark_all_read(state: State<'_, AppState>, feed_id: Option<i64>) -> R<usize> {
    state.with_store(|s| s.mark_all(scope_of(feed_id), true).map_err(err))
}

/// 全标已读的撤销（也用于误扫一遍之后的恢复）
#[tauri::command]
pub fn mark_all_unread(state: State<'_, AppState>, feed_id: Option<i64>) -> R<usize> {
    state.with_store(|s| s.mark_all(scope_of(feed_id), false).map_err(err))
}

#[tauri::command]
pub fn add_feed(state: State<'_, AppState>, url: String, title_hint: Option<String>) -> R<i64> {
    state.with_store(|s| s.add_feed(&url, title_hint.as_deref()).map_err(err))
}

/// 添加订阅前的自动发现：输入站点首页时给出真正的 feed 地址。
///
/// 判定逻辑全在 `rustrss_core::discover`（只发一次 GET：内容能按 feed 解析就用输入地址
/// 本身，否则扫页面 head 里的 `<link rel="alternate">`），这里只是胶水，不另起一套
/// 抓取参数。输入本身就是 feed 时原样返回，因此「直接粘贴 feed 地址」没有行为变化。
#[tauri::command]
pub async fn discover_feed(state: State<'_, AppState>, url: String) -> R<Discovery> {
    let fetcher = state.fetcher.clone();
    discover(&fetcher, url.trim()).await.map_err(err)
}

#[tauri::command]
pub fn remove_feed(state: State<'_, AppState>, feed_id: i64) -> R<()> {
    state.with_store(|s| {
        s.remove_feed(feed_id).map_err(err)?;
        Ok(())
    })
}

/// 刷新全部订阅源。
/// 注意分成三段：① 锁内取任务 → ② 无锁并发抓取 → ③ 锁内串行写库。
#[tauri::command]
pub async fn refresh_all(
    state: State<'_, AppState>,
    concurrency: Option<usize>,
) -> R<RefreshReport> {
    let jobs = state.with_store(|s| {
        let ids = s.all_feed_ids().map_err(err)?;
        rustrss_core::collect_jobs(s, &ids).map_err(err)
    })?;
    let fetcher = state.fetcher.clone();
    let results = rustrss_core::fetch_jobs(&fetcher, jobs, concurrency.unwrap_or(6)).await;
    state.with_store(|s| rustrss_core::apply_results(s, results).map_err(err))
}

/// 刷新单个订阅源（失败源上的「重试」用它）
#[tauri::command]
pub async fn refresh_feed(
    state: State<'_, AppState>,
    feed_id: i64,
    concurrency: Option<usize>,
) -> R<RefreshReport> {
    let jobs = state.with_store(|s| rustrss_core::collect_jobs(s, &[feed_id]).map_err(err))?;
    let fetcher = state.fetcher.clone();
    let results = rustrss_core::fetch_jobs(&fetcher, jobs, concurrency.unwrap_or(1)).await;
    state.with_store(|s| rustrss_core::apply_results(s, results).map_err(err))
}

/// 导出 OPML：弹原生保存对话框 → 写文件。返回实际写入路径（用户取消则 None）。
/// 只需过滤与文件名，路径由对话框给出；阻塞式调用在非主线程的 async 命令里是安全的。
#[tauri::command]
pub async fn export_opml(app: tauri::AppHandle, state: State<'_, AppState>) -> R<Option<String>> {
    let content = state.with_store(|s| rustrss_core::opml::export(s).map_err(err))?;
    let picked = app
        .dialog()
        .file()
        .add_filter("OPML", &["opml", "xml"])
        .set_file_name("rustrss.opml")
        .blocking_save_file();
    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path.into_path().map_err(|e| format!("路径无效: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

/// 导入 OPML：弹原生打开对话框 → 读文件 → 导入。返回统计（用户取消则 None）。
#[tauri::command]
pub async fn import_opml(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> R<Option<rustrss_core::opml::ImportReport>> {
    let picked = app
        .dialog()
        .file()
        .add_filter("OPML", &["opml", "xml"])
        .blocking_pick_file();
    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path.into_path().map_err(|e| format!("路径无效: {e}"))?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    state
        .with_store(|s| rustrss_core::opml::import(s, &content).map_err(err))
        .map(Some)
}

/// 用系统默认浏览器打开链接。
///
/// 只允许 http/https —— 文章里的链接是不可信输入，绝不能把 file:// 之类
/// 交给系统打开器；也因此这里只用 `Command::new(程序).arg(地址)`，不经过 shell。
#[tauri::command]
pub fn open_external(url: String) -> R<()> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(format!("只允许打开 http/https 链接，收到: {trimmed}"));
    }
    #[cfg(target_os = "windows")]
    let program = "cmd";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";

    let mut command = std::process::Command::new(program);
    #[cfg(target_os = "windows")]
    command.args(["/C", "start", ""]);
    command.arg(trimmed);
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("调用 {program} 失败: {e}"))
}

/// 前端诊断：打到 stdout，便于无人值守时核对界面状态（stdout 在 GUI 里不进协议通道）
#[tauri::command]
pub fn ui_log(line: String) {
    println!("[ui] {line}");
}

// ---------------------------------------------------------------- MCP

#[derive(Serialize)]
pub struct McpSettingsView {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
    pub running: bool,
    pub url: Option<String>,
    /// 可直接粘贴给客户端的配置片段（含一行命令）
    pub snippet: String,
    /// 服务是否只绑回环——界面上如实展示，而不是只写在文档里
    pub loopback_only: bool,
}

fn mcp_view(state: &AppState) -> R<McpSettingsView> {
    state.with_store(|s| {
        let enabled = s
            .bool_setting(crate::mcp_server::K_ENABLED, false)
            .map_err(err)?;
        let port = crate::mcp_server::port_from_store(s);
        let token = crate::mcp_server::token_from_store(s)?;
        let url = state.mcp.url();
        let snippet = url
            .as_deref()
            .map(|u| crate::mcp_server::client_snippet(u, &token))
            .unwrap_or_default();
        Ok(McpSettingsView {
            enabled,
            port,
            token,
            running: state.mcp.is_running(),
            loopback_only: url
                .as_deref()
                .map(crate::mcp_server::is_loopback_url)
                .unwrap_or(true),
            url,
            snippet,
        })
    })
}

#[tauri::command]
pub fn get_mcp_settings(state: State<'_, AppState>) -> R<McpSettingsView> {
    mcp_view(&state)
}

#[tauri::command]
pub async fn set_mcp_enabled(state: State<'_, AppState>, enabled: bool) -> R<McpSettingsView> {
    let (port, token) = state.with_store(|s| {
        s.set_bool_setting(crate::mcp_server::K_ENABLED, enabled)
            .map_err(err)?;
        let token = crate::mcp_server::token_from_store(s)?;
        Ok((crate::mcp_server::port_from_store(s), token))
    })?;
    if enabled {
        let db = state.db_path.clone();
        state
            .mcp
            .start(&db, token, port)
            .await
            .map_err(|e| format!("启动 MCP HTTP 服务失败: {e}"))?;
    } else {
        state.mcp.stop();
    }
    mcp_view(&state)
}

#[tauri::command]
pub async fn set_mcp_port(state: State<'_, AppState>, port: u16) -> R<McpSettingsView> {
    let (enabled, token) = state.with_store(|s| {
        s.set_setting(crate::mcp_server::K_PORT, &port.to_string())
            .map_err(err)?;
        let enabled = s
            .bool_setting(crate::mcp_server::K_ENABLED, false)
            .map_err(err)?;
        let token = crate::mcp_server::token_from_store(s)?;
        Ok((enabled, token))
    })?;
    if enabled {
        let db = state.db_path.clone();
        state
            .mcp
            .start(&db, token, port)
            .await
            .map_err(|e| format!("换端口失败: {e}"))?;
    }
    mcp_view(&state)
}

/// 轮换 token：先换库里的值，再用新 token 重启服务——旧 token 立即失效。
#[tauri::command]
pub async fn rotate_mcp_token(state: State<'_, AppState>) -> R<McpSettingsView> {
    let (enabled, port, token) = state.with_store(|s| {
        let fresh = rustrss_mcp::http::generate_token();
        s.set_setting(crate::mcp_server::K_TOKEN, &fresh)
            .map_err(err)?;
        let enabled = s
            .bool_setting(crate::mcp_server::K_ENABLED, false)
            .map_err(err)?;
        Ok((enabled, crate::mcp_server::port_from_store(s), fresh))
    })?;
    if enabled {
        let db = state.db_path.clone();
        state
            .mcp
            .start(&db, token, port)
            .await
            .map_err(|e| format!("轮换 token 失败: {e}"))?;
    }
    mcp_view(&state)
}

#[derive(Serialize)]
pub struct AiSettingsView {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub translate_target: String,
    pub has_key: bool,
    /// 凭据来源或不可用原因：如实告知，不让用户猜
    pub key_note: Option<String>,
    pub default_base_url: String,
    /// 「发送前确认要发什么」是否开启
    pub confirm_before_send: bool,
}

fn ai_settings_view(state: &AppState) -> R<AiSettingsView> {
    state.with_store(|s| {
        let provider = crate::ai::provider_from_str(
            &crate::ai::non_empty_setting(s, crate::ai::K_PROVIDER)
                .unwrap_or_else(|| crate::ai::DEFAULT_PROVIDER.to_string()),
        );
        let (key, note) = crate::ai::load_key(crate::ai::provider_to_str(provider))?;
        Ok(AiSettingsView {
            provider: crate::ai::provider_to_str(provider).to_string(),
            model: crate::ai::non_empty_setting(s, crate::ai::K_MODEL).unwrap_or_default(),
            base_url: crate::ai::non_empty_setting(s, crate::ai::K_BASE_URL).unwrap_or_default(),
            translate_target: crate::ai::translate_target(s),
            has_key: key.is_some(),
            key_note: note,
            default_base_url: crate::ai::default_base_url(provider).to_string(),
            confirm_before_send: crate::ai::confirm_before_send(s),
        })
    })
}

#[tauri::command]
pub fn get_ai_settings(state: State<'_, AppState>) -> R<AiSettingsView> {
    ai_settings_view(&state)
}

/// 「发送前确认要发什么」开关。单独一个命令：开关不需要重传整张表单。
#[tauri::command]
pub fn set_ai_confirm_before_send(
    state: State<'_, AppState>,
    enabled: bool,
) -> R<AiSettingsView> {
    state.with_store(|s| {
        s.set_setting(crate::ai::K_CONFIRM_BEFORE_SEND, if enabled { "true" } else { "false" })
            .map_err(err)
    })?;
    ai_settings_view(&state)
}

/// 保存 AI 设置。`api_key` 为 `Some("")` 表示清除凭据，`None` 表示不动它。
#[tauri::command]
pub fn save_ai_settings(
    state: State<'_, AppState>,
    provider: String,
    model: String,
    base_url: String,
    translate_target: String,
    api_key: Option<String>,
) -> R<AiSettingsView> {
    let provider = provider.trim().to_string();
    state.with_store(|s| {
        s.set_setting(crate::ai::K_PROVIDER, &provider).map_err(err)?;
        s.set_setting(crate::ai::K_MODEL, model.trim()).map_err(err)?;
        s.set_setting(crate::ai::K_BASE_URL, base_url.trim())
            .map_err(err)?;
        s.set_setting(crate::ai::K_TRANSLATE_TARGET, translate_target.trim())
            .map_err(err)?;
        if let Some(key) = api_key {
            if key.trim().is_empty() {
                crate::ai::delete_key(&provider)?;
            } else {
                crate::ai::store_key(&provider, key.trim())?;
            }
        }
        Ok(())
    })?;
    ai_settings_view(&state)
}

/// 测试连接：发一个最小请求。
/// 比「只检查 key 是否存在」有意义得多：它同时验证了凭据、模型名与端点三件事。
#[tauri::command]
pub async fn test_ai_connection(state: State<'_, AppState>) -> R<String> {
    let client = state.with_store(crate::ai::client_from_store)?;
    let request = AiRequest {
        system: Some("这是连通性测试。只回答两个字：可用".into()),
        user: "请回复：可用".into(),
    };
    client.complete(request).await.map_err(|e| e.to_string())
}

/// 计划阶段：持锁读库 + 取凭据 + 拼请求（请求本身不在锁内发）。
/// 预览与真实发送共用这一步——否则「预览看到的」和「实际发出的」会漂移。
fn build_ai_plan(
    state: &State<'_, AppState>,
    entry_id: i64,
    task: AiTask,
    policy: CachePolicy,
) -> R<(AiTaskPlan, AiClient)> {
    state.with_store(|s| {
        let client = crate::ai::client_from_store(s)?;
        let plan = rustrss_core::ai::plan_task(s, &client, entry_id, &task, policy)
            .map_err(|e| e.to_string())?;
        Ok((plan, client))
    })
}

/// 界面传来的任务名 → 任务。
/// 目标语言解析与 `ai_translate` **完全一致**（显式 target 优先，否则用设置）：
/// 两处一旦不一致，预览就会与实发漂移。
fn task_from_str(
    state: &State<'_, AppState>,
    task: &str,
    target: Option<String>,
) -> R<AiTask> {
    match task {
        "translate" => {
            let explicit = target
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            let target = match explicit {
                Some(t) => t,
                None => state.with_store(|s| Ok(crate::ai::translate_target(s)))?,
            };
            Ok(AiTask::Translate { target })
        }
        "summarize" => {
            let language = state.with_store(|s| Ok(crate::ai::translate_target(s)))?;
            Ok(AiTask::Summarize {
                length: SummaryLength::Medium,
                language,
            })
        }
        other => Err(format!("未知的 AI 任务: {other}")),
    }
}

/// 预览里最多展示多少字符的请求体（只是显示需要，不影响真正发出的内容）。
const PREVIEW_BODY_CHARS: usize = 4000;

#[derive(Serialize)]
pub struct AiPreviewView {
    pub provider_model: String,
    /// 将要请求的地址
    pub url: String,
    /// 请求头，凭据已打码
    pub headers: Vec<(String, String)>,
    /// 将要发送的请求体（JSON，已格式化）
    pub body: String,
    /// 请求体总字符数
    pub body_chars: usize,
    /// 请求体仅为展示而被截断（不代表真正发送的内容被截断）
    pub body_clipped: bool,
    /// 提示词因正文过长被截断（真正发送的内容确实不完整）
    pub truncated: bool,
    /// 命中缓存：不会外发任何请求
    pub from_cache: bool,
    /// 是否真的会发出网络请求
    pub will_send: bool,
}

/// 「发送前确认要发什么」：返回即将发出的请求（凭据打码）。
///
/// `refresh` 会决定缓存策略，必须与真实发送一致：曾因为这里固定用缓存计划，
/// 「重新生成」时预览说“不会发送”，实际却重新发了请求（且没经过确认）。
#[tauri::command]
pub async fn ai_preview(
    state: State<'_, AppState>,
    entry_id: i64,
    task: String,
    target: Option<String>,
    refresh: Option<bool>,
) -> R<AiPreviewView> {
    let task = task_from_str(&state, &task, target)?;
    let policy = if refresh.unwrap_or(false) {
        CachePolicy::Refresh
    } else {
        CachePolicy::UseCache
    };
    let (plan, client) = build_ai_plan(&state, entry_id, task, policy)?;
    let preview = client
        .preview(&plan.request)
        .map_err(|e| e.to_string())?;
    // 展示串与计数用同一个来源，否则会出现「仅展示前 4000 / 3800 字符」这种自相矛盾
    let pretty = serde_json::to_string_pretty(&preview.body).unwrap_or_else(|_| "{}".into());
    let body_chars = pretty.chars().count();
    let body_clipped = body_chars > PREVIEW_BODY_CHARS;
    let body: String = pretty.chars().take(PREVIEW_BODY_CHARS).collect();

    Ok(AiPreviewView {
        provider_model: plan.provider_model.clone(),
        url: preview.url.clone(),
        headers: preview.headers.clone(),
        body,
        body_chars,
        body_clipped,
        truncated: plan.truncated,
        from_cache: plan.cached.is_some(),
        will_send: plan.cached.is_none(),
    })
}

#[derive(Serialize)]
pub struct AiOutcomeView {
    pub output: String,
    pub from_cache: bool,
    pub truncated: bool,
    pub provider_model: String,
}

/// AI 任务三阶段：计划（持锁）→ 请求（不持锁）→ 落缓存（持锁）
async fn run_ai(
    state: State<'_, AppState>,
    entry_id: i64,
    task: AiTask,
    refresh: bool,
) -> R<AiOutcomeView> {
    let policy = if refresh {
        CachePolicy::Refresh
    } else {
        CachePolicy::UseCache
    };

    let (plan, client) = build_ai_plan(&state, entry_id, task, policy)?;

    if let Some(hit) = plan.cached.clone() {
        return Ok(AiOutcomeView {
            output: hit,
            from_cache: true,
            truncated: plan.truncated,
            provider_model: plan.provider_model,
        });
    }

    let output = client
        .complete(plan.request.clone())
        .await
        .map_err(|e| e.to_string())?;
    state.with_store(|s| {
        rustrss_core::ai::save_task_output(s, &plan, &output).map_err(|e| e.to_string())
    })?;

    Ok(AiOutcomeView {
        output,
        from_cache: false,
        truncated: plan.truncated,
        provider_model: plan.provider_model,
    })
}

#[tauri::command]
pub async fn ai_summarize(
    state: State<'_, AppState>,
    entry_id: i64,
    refresh: Option<bool>,
) -> R<AiOutcomeView> {
    // 与预览走同一份任务构造，避免两处各自拼任务时静默漂移
    let task = task_from_str(&state, "summarize", None)?;
    run_ai(state, entry_id, task, refresh.unwrap_or(false)).await
}

#[tauri::command]
pub async fn ai_translate(
    state: State<'_, AppState>,
    entry_id: i64,
    target: Option<String>,
    refresh: Option<bool>,
) -> R<AiOutcomeView> {
    let task = task_from_str(&state, "translate", target)?;
    run_ai(state, entry_id, task, refresh.unwrap_or(false)).await
}
