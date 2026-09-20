//! Tauri 命令：界面与 Rust 之间的全部接口。
//!
//! 约定：
//! - 所有命令返回 `Result<_, String>`，错误信息直接可显示给用户；
//! - 参数名在 JS 侧用 camelCase（Tauri 会自动映射到 snake_case）；
//! - 异步命令**不在 await 期间持有数据库锁**——抓取与写库分成两段。

use serde::Serialize;
use tauri::State;

use rustrss_core::fetch::RefreshReport;
use rustrss_core::{EntryQuery, EntryRow, FeedRow, MarkScope};

use crate::state::AppState;

type R<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct DbInfo {
    pub db_path: String,
    pub feeds: i64,
    pub entries: i64,
    pub unread: i64,
    pub starred: i64,
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
    limit: Option<u32>,
) -> R<Vec<EntryRow>> {
    let query = EntryQuery {
        feed_id,
        unread_only: unread_only.unwrap_or(false),
        starred_only: starred_only.unwrap_or(false),
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

#[tauri::command]
pub fn set_starred(state: State<'_, AppState>, ids: Vec<i64>, starred: bool) -> R<usize> {
    state.with_store(|s| s.set_starred(&ids, starred).map_err(err))
}

/// 设置键：`j`/`k` 浏览时是否顺便标记已读。
/// 默认值只在 Rust 这一处定义，界面只负责显示与切换，避免两边各写一份而漂移。
const KEY_MARK_READ_ON_NAVIGATE: &str = "ui.mark_read_on_navigate";
const DEFAULT_MARK_READ_ON_NAVIGATE: bool = true;

#[derive(Serialize)]
pub struct UiSettings {
    pub mark_read_on_navigate: bool,
}

#[tauri::command]
pub fn get_ui_settings(state: State<'_, AppState>) -> R<UiSettings> {
    state.with_store(|s| {
        Ok(UiSettings {
            mark_read_on_navigate: s
                .bool_setting(KEY_MARK_READ_ON_NAVIGATE, DEFAULT_MARK_READ_ON_NAVIGATE)
                .map_err(err)?,
        })
    })
}

#[tauri::command]
pub fn set_mark_read_on_navigate(state: State<'_, AppState>, enabled: bool) -> R<UiSettings> {
    state.with_store(|s| {
        s.set_bool_setting(KEY_MARK_READ_ON_NAVIGATE, enabled)
            .map_err(err)?;
        Ok(UiSettings {
            mark_read_on_navigate: enabled,
        })
    })
}

fn scope_of(feed_id: Option<i64>) -> MarkScope {
    match feed_id {
        Some(id) => MarkScope::Feed(id),
        None => MarkScope::All,
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
