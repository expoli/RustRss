//! Tauri 命令：界面与 Rust 之间的全部接口。
//!
//! 约定：
//! - 所有命令返回 `Result<_, String>`，错误信息直接可显示给用户；
//! - 参数名在 JS 侧用 camelCase（Tauri 会自动映射到 snake_case）；
//! - 异步命令**不在 await 期间持有数据库锁**——抓取与写库分成两段。

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

use rustrss_core::ai::prompt::{AiTask, SummaryLength};
use rustrss_core::ai::{AiClient, AiRequest, AiTaskPlan, CachePolicy};
use rustrss_core::discover::{discover, Discovery};
use rustrss_core::fetch::RefreshReport;
use rustrss_core::fulltext;
use rustrss_core::{EntryQuery, EntryRow, FeedRow, MarkScope};

use crate::state::AppState;

type R<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// 侧栏聚合数据：一次锁获取返回全部，消除多命令并发抢锁。
#[derive(Serialize)]
pub struct SidebarData {
    pub db: DbInfo,
    pub feeds: Vec<FeedRow>,
    pub folders: Vec<rustrss_core::store::FolderRow>,
    /// 侧栏「标签」区的行（core 的侧栏口径：置顶优先 → 手动顺序 → 名称，带未读计数）。
    /// 与选择器的 `list_tags(recent_first=true)` 是**同一张表、两个 ORDER BY**，
    /// 未读计数只有 core 这一个事实源，界面不自己数。
    pub tags: Vec<rustrss_core::TagRow>,
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

/// 重查询耗时打点：超过 50ms 记一行 warn（帮助定位界面卡顿）。
///
/// 保留 `[rustrss][slow]` 标记：AGENTS.md 把「分层计时」列为性能定位的第一手段，
/// 检索口径不能跟着迁移漂移。
fn log_slow(name: &str, started: std::time::Instant) {
    let elapsed = started.elapsed();
    if elapsed.as_millis() >= 50 {
        log::warn!("[rustrss][slow] {name}: {}ms", elapsed.as_millis());
    }
}

#[tauri::command]
pub async fn db_info(state: State<'_, AppState>) -> R<DbInfo> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| {
        let (entries, unread, starred, later) = s.counts().map_err(err)?;
        let feeds = s.list_feeds().map_err(err)?;
        Ok(DbInfo {
            db_path: state.db_path.display().to_string(),
            feeds: feeds.len() as i64,
            entries,
            unread,
            starred,
            later,
        })
    });
    log_slow("db_info", t);
    r
}

#[tauri::command]
pub async fn list_feeds(state: State<'_, AppState>) -> R<Vec<FeedRow>> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.list_feeds().map_err(err));
    log_slow("list_feeds", t);
    r
}

#[tauri::command]
// Tauri command 的参数就是 IPC 的键名，只能平铺；拆结构体会改变前端调用契约。
#[allow(clippy::too_many_arguments)]
pub async fn list_entries(
    state: State<'_, AppState>,
    feed_id: Option<i64>,
    unread_only: Option<bool>,
    starred_only: Option<bool>,
    read_later_only: Option<bool>,
    tag_id: Option<i64>,
    limit: Option<u32>,
    cursor_sortkey: Option<i64>,
    cursor_id: Option<i64>,
    cursor_read: Option<bool>,
) -> R<Vec<EntryRow>> {
    let query = EntryQuery {
        feed_id,
        unread_only: unread_only.unwrap_or(false),
        starred_only: starred_only.unwrap_or(false),
        read_later_only: read_later_only.unwrap_or(false),
        // 标签视图 = 按标签筛选的普通列表：排序/隐藏已读仍跟随界面设置（与 feed 视图同档）
        tag_id,
        limit: Some(limit.unwrap_or(200)),
        cursor: cursor_pair(cursor_sortkey, cursor_id),
        // 只有 unread_first 档用得上（store 侧只在该档读它）；其他档传了也被忽略
        cursor_read,
        // 界面路径不传新过滤/覆盖（follow 设置），机械补全使新字段走默认值
        ..Default::default()
    };
    let t = std::time::Instant::now();
    let rows = state.with_store(|s| s.list_entries(&query).map_err(err));
    log_slow("list_entries", t);
    rows
}

#[tauri::command]
pub async fn get_entry(state: State<'_, AppState>, id: i64) -> R<Option<EntryRow>> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.get_entry(id).map_err(err));
    let r = r.map(|opt| opt.map(slim_entry));
    log_slow("get_entry", t);
    r
}

/// 阅读页只渲染 content_html；有 HTML 时不再传纯文本副本（大文章可省近一半 IPC 体积）
fn slim_entry(mut entry: EntryRow) -> EntryRow {
    if entry.content_html.is_some() {
        entry.content_text = None;
    }
    entry
}

/// 获取全文：抓原文页 → 提取正文 → 写回库（摘要型条目的「获取全文」）。
///
/// 三段结构，与刷新同一条纪律：
/// ① 锁内读条目 + 幂等判定 → ② **锁外**抓取（网络永不持锁）→ ③ 锁内写回并回读。
/// 失败路径只返回一句可直接显示的错误：库里正文不动，界面继续显示原有摘要。
#[tauri::command]
pub async fn fetch_fulltext(state: State<'_, AppState>, entry_id: i64) -> R<EntryRow> {
    fetch_fulltext_core(&state, entry_id).await
}

/// `fetch_fulltext` 的本体：不依赖 Tauri 才能单测（幂等与降级两条路径都有断言）。
///
/// 不打 `log_slow`：这段耗时以网络为主，而那个打点是给「重查询拖慢界面」用的
/// —— 把 1s 的网络等待也记成慢查询只会淹没真信号。
pub(crate) async fn fetch_fulltext_core(state: &AppState, entry_id: i64) -> R<EntryRow> {
    // ① 幂等：已抓过（或本来就是全文型）直接回当前内容，不发任何请求
    let entry = state
        .with_store(|s| s.get_entry(entry_id).map_err(err))?
        .ok_or_else(|| format!("条目 #{entry_id} 不存在"))?;
    let url = entry
        .url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| "该条目没有原文地址，无法获取全文".to_string())?;
    if !entry.needs_fulltext {
        return Ok(slim_entry(entry));
    }

    // ② 抓取：网络阶段不持库锁（与 refresh_core 同一口径）。
    // 用带体积上限的流式抓取：Content-Length 预检 + 下载中累计超限即中止，
    // 不再把超限页面整个缓冲（follow-up：原先闸门在整包后才判，白流量白内存）。
    let started = std::time::Instant::now();
    let body = state
        .fetcher
        .fetch_bytes_limited(&url, fulltext::MAX_BYTES)
        .await
        .map_err(|e| format!("获取原文失败: {e}"))?;
    // HTTP 明细只在 debug 级；URL 与错误正文不落盘（部分原文链接自带一次性凭据）
    log::debug!(
        "[rustrss] 全文抓取完成: entry={entry_id} 字节={} 耗时={}ms",
        body.len(),
        started.elapsed().as_millis()
    );

    // ③ 提取（体积闸门/非 HTML/空正文都在 core 里把关）后写回，写回只在锁内做
    let extracted = fulltext::extract_bytes(&body, &url).map_err(err)?;
    let row = state.with_store(|s| {
        s.set_fulltext(entry_id, &extracted.content_html, &extracted.content_text)
            .map_err(err)?;
        s.get_entry(entry_id).map_err(err)
    })?;
    row.map(slim_entry)
        .ok_or_else(|| "写回后条目不见了（数据异常）".to_string())
}

#[tauri::command]
pub fn search(state: State<'_, AppState>, query: String, limit: Option<u32>) -> R<Vec<EntryRow>> {
    state.with_store(|s| s.search(&query, limit.unwrap_or(100)).map_err(err))
}

#[tauri::command]
pub fn set_read(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
    read: bool,
) -> R<usize> {
    let n = state.with_store(|s| s.set_read(&ids, read).map_err(err))?;
    sync_badge(&app, &state);
    Ok(n)
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

/// 自动刷新间隔：`off`（关）或分钟档位，默认 30 分钟。
/// 白名单表是唯一来源（归一化与时长解析都从它读，避免两处各写一份而漂移）。
const KEY_REFRESH_INTERVAL: &str = "refresh.interval_minutes";
const REFRESH_INTERVAL_CHOICES: [(&str, u64); 5] = [
    ("15", 15),
    ("30", 30),
    ("60", 60),
    ("120", 120),
    ("360", 360),
];
const DEFAULT_REFRESH_INTERVAL: &str = "30";
/// 启动时自动刷新（默认开）
const KEY_REFRESH_ON_START: &str = "refresh.on_start";
/// 刷新并发档位（网络抓取阶段的同时在飞源数）。白名单 3/6/12/24：
/// 3=弱网/代理/源站限流敏感；6=默认（单源同时刻只有 1 个请求，6 个不同源并发，
/// 大多数源站限流不触发，也是 HTTP/1.1 同域并发的惯例值）；12=数百订阅重度用户；
/// 24=本地 RSSHub/内网源追求最快刷完。更大值无意义（bounded_map 硬钳 64，
/// 过大并发只会能触发更多 429）。
const KEY_REFRESH_CONCURRENCY: &str = "refresh.concurrency";
pub(crate) const DEFAULT_REFRESH_CONCURRENCY: u32 = 6;
pub(crate) const REFRESH_CONCURRENCY_CHOICES: [u32; 4] = [3, 6, 12, 24];
const DEFAULT_REFRESH_ON_START: bool = true;
/// 新文章系统通知（默认关）。只有后台刷新路径会触发（手动刷新时用户就在看）。
pub(crate) const KEY_NOTIFY_NEW_ARTICLES: &str = "notify.new_articles";
const DEFAULT_NOTIFY_NEW_ARTICLES: bool = false;
/// 日志级别：`info`（默认）/ `debug`。取不到或非法一律回落 `info`。
/// 键的读路径在启动（`main.rs`）与设置回显，写路径是 `set_log_level`。
pub const KEY_LOG_LEVEL: &str = "log.level";
/// `log.level` 的默认值（也是非法值的回落目标）。
pub(crate) const DEFAULT_LOG_LEVEL: &str = "info";

/// `log.level` 白名单归一化：只认 `debug`，其余（缺失 / 非法 / 未来新档）一律 `info`。
/// 写库前过这道，库里不存拼错的值；读侧（启动 / 回显）同源，坏值不会被带进 logger。
pub(crate) fn normalize_log_level(raw: &str) -> &'static str {
    if raw.trim().eq_ignore_ascii_case("debug") {
        "debug"
    } else {
        DEFAULT_LOG_LEVEL
    }
}

/// `log.level` → `log::LevelFilter`（复用同一份归一化，避免读写两套口径分叉）。
pub fn log_level_filter(raw: &str) -> log::LevelFilter {
    match normalize_log_level(raw) {
        "debug" => log::LevelFilter::Debug,
        _ => log::LevelFilter::Info,
    }
}

/// 从设置读日志级别并归一化（缺失 / 非法 → 默认 `info`）：回显与启动应用共用。
pub(crate) fn log_level_from_store(s: &rustrss_core::Store) -> String {
    crate::ai::non_empty_setting(s, KEY_LOG_LEVEL)
        .map(|v| normalize_log_level(&v).to_string())
        .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_string())
}
// 列表排序档与「隐藏已读」的键名与默认档白名单在 core 侧定义（`rustrss_core::store::LIST_SORT_KEY` /
// `ListSort`）：命令层只负责归一化与落库，store 在每次查询时从库读这两个键——这里是唯一写入点。
/// 字体族设置（设置 → 外观 → 字体）。空串 = 跟随内置字体栈（前端清除对应 CSS 变量）。
const KEY_FONT_UI: &str = "ui.font_ui";
const KEY_FONT_READ: &str = "ui.font_read";
const KEY_FONT_MONO: &str = "ui.font_mono";
/// 正文字号（px）与行高。白名单之外的值一律 clamp 回区间（滑块区间 13-18 / 1.5-1.8）。
const KEY_FONT_READ_SIZE: &str = "ui.font_read_size";
const KEY_FONT_READ_LINE: &str = "ui.font_read_line";
const DEFAULT_FONT_READ_SIZE: f64 = 14.0;
const DEFAULT_FONT_READ_LINE: f64 = 1.55;
const FONT_READ_SIZE_RANGE: (f64, f64) = (13.0, 18.0);
const FONT_READ_LINE_RANGE: (f64, f64) = (1.5, 1.8);
/// 字体族名长度上限：坏数据不该把 CSS 值撑成天文数字（下拉标签也放不下）。
const MAX_FONT_FAMILY_LEN: usize = 100;
/// 字体枚举超时：fc-list 正常在几十毫秒返回，装了上千字体的机器也就几百毫秒。
const FONT_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Serialize)]
pub struct UiSettings {
    pub mark_read_on_navigate: bool,
    pub locale: String,
    pub theme: String,
    pub close_action: String,
    pub rsshub_mirror: String,
    /// 回显给界面的是归一化后的值（`off` 或 `15/30/60/120/360`），前端直接当 select 的值用
    pub refresh_interval_minutes: String,
    /// 刷新并发档位（归一化后的 3/6/12/24，前端直接当下拉值用）
    pub refresh_concurrency: u32,
    pub refresh_on_start: bool,
    pub notify_new_articles: bool,
    /// 字体族（空串 = 跟随系统/内置字体栈）
    pub font_ui: String,
    pub font_read: String,
    pub font_mono: String,
    /// 正文字号 px 与行高（已 clamp 回 13-18 / 1.5-1.8，前端直接当滑块值用）
    pub font_read_size: f64,
    pub font_read_line: f64,
    /// 列表排序档（归一化后的 `newest` / `oldest` / `unread_first`，前端直接当菜单值用）
    pub list_sort: String,
    /// 列表「隐藏已读」开关（星标 / 稍后读视图豁免，由 store 侧决定）
    pub list_hide_read: bool,
    /// 日志级别（归一化后的 `info` / `debug`，前端直接当下拉值用）
    pub log_level: String,
}

/// 间隔白名单归一化：`off` 或 `15/30/60/120/360`；其余（含拼错值、负数、空串）一律归默认 30。
pub(crate) fn normalize_refresh_interval(value: &str) -> &'static str {
    let trimmed = value.trim();
    if trimmed == "off" {
        return "off";
    }
    REFRESH_INTERVAL_CHOICES
        .iter()
        .find(|(label, _)| *label == trimmed)
        .map(|(label, _)| *label)
        .unwrap_or(DEFAULT_REFRESH_INTERVAL)
}

/// 间隔档位 → 调度时长；`off` 返回 `None`（不调度）。
pub(crate) fn refresh_interval_duration(value: &str) -> Option<std::time::Duration> {
    REFRESH_INTERVAL_CHOICES
        .iter()
        .find(|(label, _)| *label == normalize_refresh_interval(value))
        .map(|(_, minutes)| std::time::Duration::from_secs(minutes * 60))
}

/// 每源刷新间隔归一化：`None` / `"global"` / 空串 → `None`（跟随全局档）；
/// 档位白名单与全局设置**共用同一张表**（`REFRESH_INTERVAL_CHOICES`）。
///
/// 与全局档的差别：非法值只给可读错误，不像全局那样静默回默认档——这是用户对
/// 单个源的显式选择，猜错档位（比如把 `3600` 当 360）比报错更糟。`off` 也不接受：
/// 单源没有「关闭」档，要彻底停自动刷新就关全局并让该源跟随全局。
fn normalize_feed_refresh_interval(value: Option<&str>) -> Result<Option<i64>, String> {
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if raw == "global" {
        return Ok(None);
    }
    REFRESH_INTERVAL_CHOICES
        .iter()
        .find(|(label, _)| *label == raw)
        .map(|(_, minutes)| Some(*minutes as i64))
        .ok_or_else(|| format!("不支持的刷新间隔「{raw}」（可选 global 或 15/30/60/120/360）"))
}

/// 库里的间隔设置 → 归一化后的档位串（界面与调度器共用这一条读取路径）。
pub(crate) fn refresh_interval_from_store(store: &rustrss_core::Store) -> String {
    crate::ai::non_empty_setting(store, KEY_REFRESH_INTERVAL)
        .map(|v| normalize_refresh_interval(&v).to_string())
        .unwrap_or_else(|| DEFAULT_REFRESH_INTERVAL.to_string())
}

/// 字体族名归一化：trim + 折叠空白 + 去控制字符 + 截断到 [`MAX_FONT_FAMILY_LEN`]；
/// 空串 = 跟随内置字体栈（前端清除对应 CSS 变量）。
///
/// 不在这里剥引号/逗号这类「CSS 里有含义」的字符：写入 CSS 变量时由前端统一
/// 加引号并转义（`cssFamily`），在这里改动反而会把真实存在的族名改错。
pub(crate) fn normalize_font_family(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .filter_map(|c| {
            if !c.is_control() {
                Some(c)
            } else if c.is_whitespace() {
                // 制表/换行这类空白控制字符当成词间空格（否则 "Foo\tBar" 会粘成 "FooBar"）
                Some(' ')
            } else {
                None
            }
        })
        .take(MAX_FONT_FAMILY_LEN)
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 字号/行高的数值归一化：非有限值回默认，越界 clamp 回区间，固定两位小数。
///
/// 两位小数是为了库里的值干净（滑块步长 0.05，浮点乘加会带出 1.5500000000000003）。
fn clamp_font_number(value: f64, default: f64, (min, max): (f64, f64)) -> f64 {
    if !value.is_finite() {
        return default;
    }
    (value.clamp(min, max) * 100.0).round() / 100.0
}

/// 字体设置的读取：缺失/空串 → 空字体族（前端清除变量 → 回内置字体栈）。
fn font_family_setting(store: &rustrss_core::Store, key: &str) -> String {
    normalize_font_family(&crate::ai::non_empty_setting(store, key).unwrap_or_default())
}

/// 字体设置的读取：缺失、非数字、非有限值都回默认；越界值 clamp 回区间
/// （与 locale/theme 的白名单同一口径：库里被写坏也不让字号把正文排版搞崩）。
fn font_number_setting(
    store: &rustrss_core::Store,
    key: &str,
    default: f64,
    range: (f64, f64),
) -> f64 {
    crate::ai::non_empty_setting(store, key)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|v| clamp_font_number(v, default, range))
        .unwrap_or(default)
}

/// 数字 → 库内字符串：`14.0` 存 "14"（而不是 "14.0"），`1.55` 存 "1.55"。
fn font_number_text(value: f64) -> String {
    format!("{value}")
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
            // 镜像 base 走统一 clean：空/非法回退官方默认
            rsshub_mirror: crate::ai::non_empty_setting(s, rustrss_core::rsshub::MIRROR_KEY)
                .map(|v| rustrss_core::rsshub::clean_base(&v))
                .unwrap_or_else(|| rustrss_core::rsshub::DEFAULT_BASE.to_string()),
            refresh_interval_minutes: refresh_interval_from_store(s),
            refresh_concurrency: refresh_concurrency_from_store(s),
            // 布尔设置的非法值在 bool_setting 里已回退默认（与 mark_read_on_navigate 同口径）
            refresh_on_start: s
                .bool_setting(KEY_REFRESH_ON_START, DEFAULT_REFRESH_ON_START)
                .map_err(err)?,
            notify_new_articles: s
                .bool_setting(KEY_NOTIFY_NEW_ARTICLES, DEFAULT_NOTIFY_NEW_ARTICLES)
                .map_err(err)?,
            font_ui: font_family_setting(s, KEY_FONT_UI),
            font_read: font_family_setting(s, KEY_FONT_READ),
            font_mono: font_family_setting(s, KEY_FONT_MONO),
            font_read_size: font_number_setting(
                s,
                KEY_FONT_READ_SIZE,
                DEFAULT_FONT_READ_SIZE,
                FONT_READ_SIZE_RANGE,
            ),
            font_read_line: font_number_setting(
                s,
                KEY_FONT_READ_LINE,
                DEFAULT_FONT_READ_LINE,
                FONT_READ_LINE_RANGE,
            ),
            list_sort: s.list_sort().as_str().to_string(),
            list_hide_read: s.list_hide_read(),
            log_level: log_level_from_store(s),
        })
    })
}

/// 侧栏一次拉全：db 计数 + 订阅（含未读聚合）+ 文件夹，单次锁获取。
/// 供 refreshCounts 使用，避免三个并发命令互相抢 Mutex 排队。
#[tauri::command]
pub async fn sidebar_data(state: State<'_, AppState>) -> R<SidebarData> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| {
        // COUNT×4 合并为一次扫描
        let (entries, unread, starred, later) = s.counts().map_err(err)?;
        let feeds = s.list_feeds().map_err(err)?;
        Ok(SidebarData {
            db: DbInfo {
                db_path: state.db_path.display().to_string(),
                feeds: feeds.len() as i64,
                entries,
                unread,
                starred,
                later,
            },
            feeds,
            folders: s.list_folders_ordered().map_err(err)?,
            tags: s.list_tags().map_err(err)?,
        })
    });
    log_slow("sidebar_data", t);
    r
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

/// 自动刷新间隔：`off` 或分钟档位。非法值归一化后再落库（库里不存拼错的值）。
#[tauri::command]
pub fn set_refresh_interval(state: State<'_, AppState>, minutes: String) -> R<UiSettings> {
    state.with_store(|s| {
        s.set_setting(KEY_REFRESH_INTERVAL, normalize_refresh_interval(&minutes))
            .map_err(err)
    })?;
    ui_settings(&state)
}

/// 设置某个源的刷新间隔覆盖（`null` / `"global"` = 恢复跟随全局档），
/// 返回更新后的行（前端拿它更新勾选态与 tooltip，不必重拉整个侧栏）。
#[tauri::command]
pub fn set_feed_refresh_interval(
    state: State<'_, AppState>,
    feed_id: i64,
    value: Option<String>,
) -> R<FeedRow> {
    let minutes = normalize_feed_refresh_interval(value.as_deref())?;
    state.with_store(|s| s.set_feed_refresh_interval(feed_id, minutes).map_err(err))
}

/// 订阅源配置补丁（`set_feed_config` 的参数对象）。
///
/// 每个字段都区分「不动」与「清除」：键缺省 / `null` = 不动（不落库）；
/// 显式给值才写。平铺三个参数做不出这个语义（`Option<i64>` 的 `null`
/// 到底是「不动」还是「移出到未分组」无法区分），所以用对象补丁。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedConfigPatch {
    /// 自定义标题：键缺省/`null` = 不动；`""`（或纯空白）= 清除（回退源站名）；
    /// 其它 = 设自定义标题。
    pub custom_title: Option<String>,
    /// 文件夹：键缺省 = 不动；`null` = 移出到未分组；数字 = 移入该文件夹。
    /// 用 `deserialize_with` 是因为 `Option<FolderPatch>` 会把 `null` 吃成
    /// 「不动」（见 `folder_patch`），那样就没有写法能表达「移出到未分组」。
    #[serde(default, deserialize_with = "folder_patch")]
    pub folder_id: Option<FolderPatch>,
    /// 刷新间隔：键缺省/`null` = 不动；`"global"` = 跟随全局；白名单档位 = 覆盖。
    /// 归一化复用 `normalize_feed_refresh_interval`（与右键菜单同一条白名单）。
    pub refresh_interval: Option<String>,
}

/// 文件夹补丁：`null` 与数字都合法，缺键不合法（缺键由 `Option` 层表达「不动」）。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum FolderPatch {
    /// JSON `null`：移出到未分组（与 `assign_feed_folder(feedId, null)` 同义）
    Ungrouped,
    /// JSON 数字：移入该文件夹
    Id(i64),
}

/// `folderId` 字段的自定义解析：`null` → `Some(Ungrouped)`，数字 → `Some(Id)`。
/// 键缺省时 serde 不走这里（`#[serde(default)]` 直接给 `None` = 不动）。
fn folder_patch<'de, D>(de: D) -> Result<Option<FolderPatch>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    FolderPatch::deserialize(de).map(Some)
}

/// 自定义标题归一化：trim 后为空 = 清除（`None`）；过长截断到 [`MAX_CUSTOM_TITLE_LEN`]
/// 个字符（侧栏行与 tooltip 都是单行，不截断会让布局被一段长标题撑坏）。
/// 截断按字符（不是字节）——中文标题不会被截成半个字。
pub(crate) fn normalize_custom_title(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_CUSTOM_TITLE_LEN).collect())
}

/// 侧栏/对话框输入的标题上限；界面侧 `maxlength` 用同一个数，正常操作摸不到上限。
pub(crate) const MAX_CUSTOM_TITLE_LEN: usize = 200;

/// 保存订阅源编辑对话框的三个可改字段：None/缺键 = 不动，只写用户真改过的字段。
/// 返回更新后的 FeedRow（显示名 + 源站名 + 自定义值 + 文件夹 + 间隔，前端不用重拉侧栏）。
///
/// 文件夹落库复用 `assign_folder`（与 `assign_feed_folder` 同一条存储路径），
/// 间隔归一化复用 `normalize_feed_refresh_interval`（与右键菜单同一张白名单）。
#[tauri::command]
pub fn set_feed_config(
    state: State<'_, AppState>,
    feed_id: i64,
    patch: FeedConfigPatch,
) -> R<FeedRow> {
    set_feed_config_core(&state, feed_id, &patch)
}

/// `set_feed_config` 的本体：不依赖 Tauri（`State` 不好在单测里构造），
/// 归一化 + 落库 + 回读三段与命令完全同源。
pub(crate) fn set_feed_config_core(
    state: &AppState,
    feed_id: i64,
    patch: &FeedConfigPatch,
) -> R<FeedRow> {
    // 先归一化全部字段再落库：任何一个字段非法时一个字段都不写（不留半套修改），
    // 且用户拿到的是可读错误而不是写了一半的状态。
    // 两处双层 Option：外层 None = 不动（不落库），内层 None = 清除/跟随全局。
    let custom_title = patch.custom_title.as_deref().map(normalize_custom_title);
    let folder = match patch.folder_id {
        None => None,
        Some(FolderPatch::Ungrouped) => Some(None),
        Some(FolderPatch::Id(id)) => Some(Some(id)),
    };
    let minutes = match &patch.refresh_interval {
        None => None,
        Some(v) => Some(normalize_feed_refresh_interval(Some(v))?),
    };

    state.with_store(|s| {
        if let Some(title) = custom_title.as_ref() {
            s.set_feed_custom_title(feed_id, title.as_deref()).map_err(err)?;
        }
        if let Some(folder_id) = folder {
            s.assign_folder(feed_id, folder_id).map_err(err)?;
        }
        if let Some(minutes) = minutes {
            s.set_feed_refresh_interval(feed_id, minutes).map_err(err)?;
        }
        s.feed_row(feed_id)
            .map_err(err)?
            .ok_or_else(|| format!("订阅 #{feed_id} 不存在"))
    })
}

/// 启动时是否自动刷新一次。调度器在启动首 tick 时读这个值。
#[tauri::command]
pub fn set_refresh_on_start(state: State<'_, AppState>, enabled: bool) -> R<UiSettings> {
    state.with_store(|s| {
        s.set_bool_setting(KEY_REFRESH_ON_START, enabled)
            .map_err(err)
    })?;
    ui_settings(&state)
}

/// 并发档位白名单归一化：3/6/12/24；其余（含拼错值、越界、空串）一律归默认 6。
pub(crate) fn normalize_refresh_concurrency(value: u32) -> u32 {
    if REFRESH_CONCURRENCY_CHOICES.contains(&value) {
        value
    } else {
        DEFAULT_REFRESH_CONCURRENCY
    }
}

/// 读并发档位（缺失/非法回退默认 6）。调度器每轮 tick 与前端手动刷新都用它。
pub(crate) fn refresh_concurrency_from_store(s: &rustrss_core::Store) -> u32 {
    crate::ai::non_empty_setting(s, KEY_REFRESH_CONCURRENCY)
        .and_then(|v| v.parse::<u32>().ok())
        .map(normalize_refresh_concurrency)
        .unwrap_or(DEFAULT_REFRESH_CONCURRENCY)
}

/// 设置刷新并发档位（下拉 3/6/12/24）。
#[tauri::command]
pub fn set_refresh_concurrency(state: State<'_, AppState>, value: u32) -> R<UiSettings> {
    let normalized = normalize_refresh_concurrency(value);
    state.with_store(|s| {
        s.set_setting(KEY_REFRESH_CONCURRENCY, &normalized.to_string())
            .map_err(err)
    })?;
    ui_settings(&state)
}

/// 新文章系统通知开关（默认关）。调度器在每轮后台刷新后读这个值决定要不要弹。
#[tauri::command]
pub fn set_notify_new_articles(state: State<'_, AppState>, enabled: bool) -> R<UiSettings> {
    state.with_store(|s| {
        s.set_bool_setting(KEY_NOTIFY_NEW_ARTICLES, enabled)
            .map_err(err)
    })?;
    ui_settings(&state)
}

/// 日志级别（`info` / `debug`）的命令体：归一 → 落库 → **立刻**应用全局级别。
///
/// 即时生效靠这里的 `set_max_level`（不重启就生效）；重启后由启动路径从同一个键
/// 重新应用。debug 下第三方依赖的 debug 会在 core writer 侧按 target 丢弃
/// （见 `rustrss_core::logging`），所以切到 debug 不会把日志文件刷成依赖库的流水账。
fn apply_log_level(state: &AppState, level: &str) -> R<UiSettings> {
    let normalized = normalize_log_level(level);
    state.with_store(|s| s.set_setting(KEY_LOG_LEVEL, normalized).map_err(err))?;
    log::set_max_level(log_level_filter(normalized));
    log::info!(
        "[rustrss] 日志级别已切换: {}（设置 log.level={normalized:?}）",
        log::max_level()
    );
    ui_settings(state)
}

/// 设置日志级别（设置 → 关于 → 日志级别）。
#[tauri::command]
pub fn set_log_level(state: State<'_, AppState>, level: String) -> R<UiSettings> {
    apply_log_level(&state, &level)
}

/// 列表排序档白名单：`newest`（默认）/ `oldest` / `unread_first`。
/// 其余（含大小写变体、拼错值、空串）一律归默认档——与 locale/theme 同一口径，
/// 不让拼错的设置把列表卡死。
pub(crate) fn normalize_list_sort(value: &str) -> &'static str {
    match value.trim() {
        "oldest" => "oldest",
        "unread_first" => "unread_first",
        _ => "newest",
    }
}

/// 列表排序档（三档）的命令体：白名单归一 → 落库 → 回显一份 UiSettings。
/// 命令层只管写；store 每次查询从设置读（单一事实源），前端不往 list_entries 带档位。
fn apply_list_sort(state: &AppState, sort: &str) -> R<UiSettings> {
    state.with_store(|s| {
        s.set_setting(
            rustrss_core::store::LIST_SORT_KEY,
            normalize_list_sort(sort),
        )
        .map_err(err)
    })?;
    ui_settings(state)
}

/// 列表排序档（三档）。写库后 store 的每次查询都从设置读它（单一事实源），
/// 前端不再把档位带进 `list_entries` 参数。
#[tauri::command]
pub fn set_list_sort(state: State<'_, AppState>, sort: String) -> R<UiSettings> {
    apply_list_sort(&state, &sort)
}

/// 列表「隐藏已读」开关的命令体。豁免规则（星标 / 稍后读视图不受影响）在 store 查询层，
/// 命令层只管写开关。
fn apply_list_hide_read(state: &AppState, enabled: bool) -> R<UiSettings> {
    state.with_store(|s| {
        s.set_bool_setting(rustrss_core::store::LIST_HIDE_READ_KEY, enabled)
            .map_err(err)
    })?;
    ui_settings(state)
}

/// 列表「隐藏已读」开关。
#[tauri::command]
pub fn set_list_hide_read(state: State<'_, AppState>, enabled: bool) -> R<UiSettings> {
    apply_list_hide_read(&state, enabled)
}

/// 读「启动时自动刷新」开关（缺失/非法回退默认 true）。
pub(crate) fn refresh_on_start_setting(state: &AppState) -> R<bool> {
    state.with_store(|s| {
        s.bool_setting(KEY_REFRESH_ON_START, DEFAULT_REFRESH_ON_START)
            .map_err(err)
    })
}

/// 读「新文章通知」开关（缺失/非法回退默认 false）。
pub(crate) fn notify_new_articles_setting(state: &AppState) -> R<bool> {
    state.with_store(|s| {
        s.bool_setting(KEY_NOTIFY_NEW_ARTICLES, DEFAULT_NOTIFY_NEW_ARTICLES)
            .map_err(err)
    })
}

/// 未读总数。后台刷新的前后差值、已读操作后的角标同步都走这一条读取路径。
pub(crate) fn unread_total(state: &AppState) -> R<i64> {
    state.with_store(|s| s.unread_total().map_err(err))
}

/// 未读数变化后同步托盘角标。读库失败就跳过：角标是装饰，不该让命令本身失败。
pub(crate) fn sync_badge(app: &tauri::AppHandle, state: &AppState) {
    if let Ok(unread) = unread_total(state) {
        crate::tray::update_badge(app, unread);
    }
}

/// 界面语言设置的原值（`auto` / `zh-CN` / `en`）。Rust 侧自己发的文案（托盘菜单、
/// 系统通知、角标 tooltip）按它选双语常量；界面文案的唯一出处仍是 `ui/i18n.js`。
pub(crate) fn ui_locale_setting(state: &AppState) -> R<String> {
    state.with_store(|s| Ok(crate::ai::non_empty_setting(s, KEY_LOCALE).unwrap_or_default()))
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

// ---------------- RSSHub 实例（镜像） ----------------

/// 当前 RSSHub 实例地址（默认官方 https://rsshub.app）。
#[tauri::command]
pub fn get_rsshub_mirror(state: State<'_, AppState>) -> R<String> {
    state.with_store(|s| {
        Ok(crate::ai::non_empty_setting(s, rustrss_core::rsshub::MIRROR_KEY)
            .map(|v| rustrss_core::rsshub::clean_base(&v))
            .unwrap_or_else(|| rustrss_core::rsshub::DEFAULT_BASE.to_string()))
    })
}

/// 设置 RSSHub 实例地址。空值 = 恢复官方默认；非法形态报可读错误。
#[tauri::command]
pub fn set_rsshub_mirror(state: State<'_, AppState>, mirror: String) -> R<String> {
    let trimmed = mirror.trim();
    if !trimmed.is_empty()
        && !trimmed.starts_with("http://")
        && !trimmed.starts_with("https://")
    {
        return Err("镜像地址需以 http:// 或 https:// 开头".into());
    }
    state.with_store(|s| {
        s.set_setting(rustrss_core::rsshub::MIRROR_KEY, trimmed).map_err(err)?;
        Ok(rustrss_core::rsshub::clean_base(trimmed))
    })
}

/// 测试实例可达性：依次尝试 /version、/、/rsshub/rss、/feed/rsshub/rss，
/// 任一返回 2xx 即可达。前两个是每个 RSSHub 实例必有的轻量路由（欢迎页/版本
/// 探针，实测 2026-09-22：本机实例 / 与 /version 200，但 /health 与两个内容路由
/// 均 404——实例版本差异下内容路由探测必埪埪误报）；后两个保留作为内容路由
/// 覆盖验证（部分部署会屏蔽根路径）。
#[tauri::command]
pub async fn test_rsshub_mirror(
    state: State<'_, AppState>,
    mirror: Option<String>,
) -> R<String> {
    use rustrss_core::rsshub::clean_base;
    let base = clean_base(&mirror.unwrap_or_default());
    let fetcher = state.fetcher.clone();
    for path in ["/version", "/", "/rsshub/rss", "/feed/rsshub/rss"] {
        let url = format!("{base}{path}");
        let started = std::time::Instant::now();
        let probe = rustrss_core::rsshub::probe_url(&fetcher, &url).await;
        // 探测路径与耗时进 debug（只打 path，不打可能带凭据的 base；错误正文同样不打）
        let status = match &probe {
            Ok(true) => "2xx",
            Ok(false) => "非 2xx",
            Err(_) => "网络错误",
        };
        log::debug!(
            "[rustrss] RSSHub 探测 {path}: {status} 耗时={}ms",
            started.elapsed().as_millis()
        );
        match probe {
            Ok(true) => return Ok(format!("可达：{url}")),
            Ok(false) => continue,
            Err(e) => {
                // 网络层错误（超时/DNS）直接报告，不再尝试下一个路径
                return Err(format!("{url} :: {e}"));
            }
        }
    }
    Ok("全部探测路由均未返回 2xx（实例可达但路由未覆盖，或被拦截）".into())
}

/// 归一化预览：真正会被改写的存量订阅数（官方域 / 非规范 scheme 行）。
///
/// 判据在 core（与执行共用同一函数），因此预览条数必然等于执行时改写的条数；
/// 也和镜像设置无关——归一化只动存储形态，解析是抓取时的事。
#[tauri::command]
pub fn preview_rsshub_migration(state: State<'_, AppState>) -> R<i64> {
    state.with_store(|s| s.count_rsshub_normalization_candidates().map_err(err))
}

/// 执行归一化：存量官方域（与三斜杠/大写等非规范 scheme）订阅改写为 `rsshub://path`。
///
/// 抓取地址不落库——由 `Store::feed_endpoint` 在抓取时按当前镜像解析，所以这个
/// 按钮是「一次性地址整理」，不是换镜像的必经步骤。冲突行计 skipped；其它失败
/// 收进 errors，单项失败不影响其余行。
#[tauri::command]
pub fn migrate_rsshub_feeds(state: State<'_, AppState>) -> R<rustrss_core::store::MigrationOutcome> {
    state.with_store(|s| s.normalize_rsshub_feeds().map_err(err))
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
    state.with_store(|s| {
        s.delete_folder(folder_id).map_err(err)?;
        // 清理折叠状态里的孤儿 id（避免残留）
        let remaining: Vec<i64> = s
            .collapsed_folders()
            .into_iter()
            .filter(|id| *id != folder_id)
            .collect();
        s.set_collapsed_folders(&remaining).map_err(err)
    })
}

#[tauri::command]
pub fn assign_feed_folder(state: State<'_, AppState>, feed_id: i64, folder_id: Option<i64>) -> R<()> {
    state.with_store(|s| s.assign_folder(feed_id, folder_id).map_err(err))
}

/// 侧栏分组折叠状态（哪些组被折叠），JSON 序列化后存 settings，跨会话保持。
/// 列出文件夹（按 position 排序），UI 分组渲染的枚举入口。
#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>) -> R<Vec<rustrss_core::store::FolderRow>> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.list_folders_ordered().map_err(err));
    log_slow("list_folders", t);
    r
}

// ---------------------------------------------------------------- 标签（薄壳）
//
// 业务逻辑全在 rustrss-core/src/store：名称校验、大小写不敏感唯一、幂等、计数、
// `last_used_at` 推进都在那里；这里只做 Tauri command → core 的透传（AGENTS.md 分层：
// src-tauri 不写业务）。选择器与侧栏拿的是同一份 `TagRow`，`recent_first` 只切换 core
// 里已定好的 ORDER BY——前端不自己排序（最近使用优先的唯一数据源是 core）。

/// 标签清单。`recent_first` = 选择器口径（`last_used_at DESC`，没用过的垫底）；
/// 缺省 = 侧栏口径（置顶优先 → 手动顺序 → 名称）。
#[tauri::command]
pub async fn list_tags(
    state: State<'_, AppState>,
    recent_first: Option<bool>,
) -> R<Vec<rustrss_core::TagRow>> {
    let t = std::time::Instant::now();
    let recent = recent_first.unwrap_or(false);
    let r = state.with_store(|s| {
        let rows = if recent {
            s.list_tags_recent_first()
        } else {
            s.list_tags()
        };
        rows.map_err(err)
    });
    log_slow("list_tags", t);
    r
}

/// 新建标签（名称 trim / 唯一性 / 排序位置都由 core 决定；重名回可读错误）。
#[tauri::command]
pub async fn create_tag(state: State<'_, AppState>, name: String) -> R<rustrss_core::TagRow> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.create_tag(&name, None).map_err(err));
    log_slow("create_tag", t);
    r
}

/// 给单个条目打标签（阅读器/列表行的交互路径；批量 ≤100 与条件级是 MCP 的口径）。
#[tauri::command]
pub async fn assign_tags(
    state: State<'_, AppState>,
    entry_id: i64,
    tag_ids: Vec<i64>,
) -> R<rustrss_core::TagAssignReport> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| {
        s.assign_tags(
            &rustrss_core::TagTarget::Entries(vec![entry_id]),
            &tag_ids,
        )
        .map_err(err)
    });
    log_slow("assign_tags", t);
    r
}

/// 取消单个条目的标签（幂等口径与 [`assign_tags`] 相同）。
#[tauri::command]
pub async fn unassign_tags(
    state: State<'_, AppState>,
    entry_id: i64,
    tag_ids: Vec<i64>,
) -> R<rustrss_core::TagAssignReport> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| {
        s.unassign_tags(
            &rustrss_core::TagTarget::Entries(vec![entry_id]),
            &tag_ids,
        )
        .map_err(err)
    });
    log_slow("unassign_tags", t);
    r
}

/// 重命名标签（名称 trim/非空、大小写不敏感唯一、重名报可读错误——全在 core）。
#[tauri::command]
pub async fn rename_tag(
    state: State<'_, AppState>,
    tag_id: i64,
    name: String,
) -> R<rustrss_core::TagRow> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.rename_tag(tag_id, &name).map_err(err));
    log_slow("rename_tag", t);
    r
}

/// 设置/清除标签颜色（`None`/空串 = 默认色；格式校验在 core，非法值回可读错误）。
#[tauri::command]
pub async fn set_tag_color(
    state: State<'_, AppState>,
    tag_id: i64,
    color: Option<String>,
) -> R<rustrss_core::TagRow> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.set_tag_color(tag_id, color.as_deref()).map_err(err));
    log_slow("set_tag_color", t);
    r
}

/// 置顶/取消置顶（置顶的标签在侧栏排最前，排序口径在 core 的 `list_tags`）。
#[tauri::command]
pub async fn set_tag_pinned(
    state: State<'_, AppState>,
    tag_id: i64,
    pinned: bool,
) -> R<rustrss_core::TagRow> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.set_tag_pinned(tag_id, pinned).map_err(err));
    log_slow("set_tag_pinned", t);
    r
}

/// 拖拽排序落库：参数是**整份可见顺序**，core 单事务按下标写 `sort_order`
/// （中途遇到不存在的 id 整体回滚），不是逐行多次 IPC。
#[tauri::command]
pub async fn reorder_tags(state: State<'_, AppState>, tag_ids: Vec<i64>) -> R<()> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.reorder_tags(&tag_ids).map_err(err));
    log_slow("reorder_tags", t);
    r
}

/// 删除标签：`dry_run=true` 只回影响篇数（不落库），`false` 才真删。
/// 两个分支的影响数来自 core 的同一个计数函数（预览与执行不可能漂移）；
/// 删除只清 `entry_tags` 关联，**文章保留**。
#[tauri::command]
pub async fn delete_tag(
    state: State<'_, AppState>,
    tag_id: i64,
    dry_run: Option<bool>,
) -> R<rustrss_core::DeleteTagReport> {
    let t = std::time::Instant::now();
    let r = state.with_store(|s| s.delete_tag(tag_id, dry_run.unwrap_or(false)).map_err(err));
    log_slow("delete_tag", t);
    r
}

/// 读取侧栏「标签」区的折叠状态（未设置 = 展开）。
#[tauri::command]
pub fn get_tags_collapsed(state: State<'_, AppState>) -> R<bool> {
    state.with_store(|s| Ok(tags_collapsed_setting(s)))
}

#[tauri::command]
pub fn set_tags_collapsed(state: State<'_, AppState>, collapsed: bool) -> R<()> {
    state.with_store(|s| {
        s.set_setting(TAGS_COLLAPSED_KEY, if collapsed { "true" } else { "false" })
            .map_err(err)
    })
}

/// 读取侧栏折叠状态（未设置返回空表）。
#[tauri::command]
pub fn get_collapsed_folders(state: State<'_, AppState>) -> R<Vec<i64>> {
    state.with_store(|s| Ok(collapsed_folders_setting(s)))
}

#[tauri::command]
pub fn set_collapsed_folders(state: State<'_, AppState>, ids: Vec<i64>) -> R<()> {
    let json = serde_json::to_string(&ids).map_err(err)?;
    state.with_store(|s| s.set_setting("ui.folders_collapsed", &json).map_err(err))
}

/// 侧栏「标签」区折叠状态的设置键（与文件夹折叠同属界面 ephemera，走 key-value 设置）。
const TAGS_COLLAPSED_KEY: &str = "ui.tags_collapsed";

/// 读折叠状态：只有明确写成 `true` 才当折叠（未设置/写坏 = 展开）。
fn tags_collapsed_setting(store: &rustrss_core::Store) -> bool {
    matches!(store.setting(TAGS_COLLAPSED_KEY), Ok(Some(v)) if v.trim() == "true")
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

/// 字体配置：一次可写 5 个 key，`None` = 该项不动（下拉只改自己那一项；滑块只写自己的值）。
///
/// - 字体族：空串 = 恢复跟随系统（清除 CSS 变量）；
/// - 字号 clamp 13-18、行高 clamp 1.5-1.8：越界值不报错，直接夹回区间后落库——
///   滑块是唯一写入方，夹回比报错更贴近用户意图（而库里因此不会出现 9px 这种值）。
///
/// 返回值是归一化后的完整设置：前端拿同一份回读值刷 CSS 变量，两边不各算一份。
#[tauri::command]
pub fn set_font_config(
    state: State<'_, AppState>,
    font_ui: Option<String>,
    font_read: Option<String>,
    font_mono: Option<String>,
    read_size: Option<f64>,
    read_line: Option<f64>,
) -> R<UiSettings> {
    set_font_config_core(&state, font_ui, font_read, font_mono, read_size, read_line)
}

/// `set_font_config` 的本体：不依赖 Tauri（`State` 不好在单测里构造），
/// 归一化 + 落库 + 回读三段与命令完全同源。
pub(crate) fn set_font_config_core(
    state: &AppState,
    font_ui: Option<String>,
    font_read: Option<String>,
    font_mono: Option<String>,
    read_size: Option<f64>,
    read_line: Option<f64>,
) -> R<UiSettings> {
    state.with_store(|s| {
        for (key, value) in [
            (KEY_FONT_UI, font_ui.as_deref()),
            (KEY_FONT_READ, font_read.as_deref()),
            (KEY_FONT_MONO, font_mono.as_deref()),
        ] {
            if let Some(raw) = value {
                s.set_setting(key, &normalize_font_family(raw)).map_err(err)?;
            }
        }
        // 数值项：只有显式给值才写（None 表示「这次不改字号」）
        if let Some(raw) = read_size {
            let size = clamp_font_number(raw, DEFAULT_FONT_READ_SIZE, FONT_READ_SIZE_RANGE);
            s.set_setting(KEY_FONT_READ_SIZE, &font_number_text(size))
                .map_err(err)?;
        }
        if let Some(raw) = read_line {
            let line = clamp_font_number(raw, DEFAULT_FONT_READ_LINE, FONT_READ_LINE_RANGE);
            s.set_setting(KEY_FONT_READ_LINE, &font_number_text(line))
                .map_err(err)?;
        }
        Ok(())
    })?;
    ui_settings(state)
}

/// 系统字体族列表（按名字排序、去重）。
///
/// 平台口径：**只有 Linux 枚举**（`fc-list`，fontconfig 在 deb 依赖里已声明）；
/// Windows / macOS 返回空表，界面在那两个平台的字体下拉只显示「跟随系统」——
/// 为字体枚举引入 font-kit / DWrite / CoreText 绑定的依赖成本高于收益（后续可增强）。
///
/// 性能红线 #12：子进程是重活，整段交给 tokio 的进程 API + 超时，命令的 async 主线上
/// 没有任何阻塞等待，也不碰数据库锁；拿不到就是空表，不让「打开设置页」失败。
#[tauri::command]
pub async fn list_font_families() -> R<Vec<String>> {
    Ok(probe_font_families().await)
}

/// `list_font_families` 的本体（不依赖 Tauri，单测直接跑真机路径）。
///
/// Linux 走 `fc-list --format=%{family[0]}`；其余平台空表（见上）。
/// 平台分支用运行时 `cfg!` 而不是 `#[cfg]` 属性：两个分支在**所有**平台都参与编译，
/// Windows / macOS 的 nightly 构建因此也能类型检查到 fc-list 这条路——`#[cfg]` 掉的
/// 分支在本机（只跑 Linux）永远不编译，写坏了要到打包时才发现。
pub(crate) async fn probe_font_families() -> Vec<String> {
    if cfg!(target_os = "linux") {
        // `--format` 的长写法比 `-f` 在旧版 fontconfig 上更稳；`%{family[0]}` = 首个族名
        run_font_command("fc-list", &["--format=%{family[0]}\n"], FONT_LIST_TIMEOUT).await
    } else {
        Vec::new()
    }
}

/// 解析 fc-list 输出（每行一个族名）→ 排序去重的族名表。
///
/// 纯函数：真机装了哪些字体不可控，解析/去重/排序用固定输入单测。
fn parse_font_families(raw: &str) -> Vec<String> {
    let mut families: Vec<String> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    families.sort();
    families.dedup();
    families
}

/// 跑一条「输出字体族」的外部命令：成功 → 解析后的族名表（非 UTF-8 字节用
/// lossy 替换，不会丢字体）；spawn 失败 / 超时 / 非零退出降级为空表（并留
/// 一行 stderr 便于诊断）。
///
/// `kill_on_drop`：超时后子进程真被杀掉，不留一个还在跑 fc-list 的孤儿。
/// （`std::process` 没有带超时的 `wait`，自己轮询 `try_wait` 在子进程写满管道时会死锁。）
async fn run_font_command(bin: &str, args: &[&str], timeout: std::time::Duration) -> Vec<String> {
    let run = tokio::process::Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(timeout, run).await {
        Ok(Ok(out)) if out.status.success() => parse_font_families(&String::from_utf8_lossy(&out.stdout)),
        Ok(Ok(out)) => {
            log::warn!(
                "[rustrss][fonts] {bin} 退出码 {:?}，字体列表降级为空",
                out.status.code()
            );
            Vec::new()
        }
        // spawn 失败（未装 fontconfig）：正常降级，不当作错误
        Ok(Err(e)) => {
            log::warn!("[rustrss][fonts] 无法执行 {bin}: {e}（字体列表降级为空）");
            Vec::new()
        }
        Err(_) => {
            log::warn!(
                "[rustrss][fonts] {bin} 超过 {}ms 未返回，已终止（字体列表降级为空）",
                timeout.as_millis()
            );
            Vec::new()
        }
    }
}

fn scope_of(feed_id: Option<i64>) -> MarkScope {
    match feed_id {
        Some(id) => MarkScope::Feed(id),
        None => MarkScope::All,
    }
}

/// keyset 续扫游标：取自上一页末行直出的 `(sortkey, id)`。
///
/// 两值必须同时出现：只给一半（例如前端手滑传错）就当作无游标从首页取，
/// 不让半截游标静默翻到错误的页。
fn cursor_pair(cursor_sortkey: Option<i64>, cursor_id: Option<i64>) -> Option<(i64, i64)> {
    cursor_sortkey.zip(cursor_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 侧栏标签区管理命令实际走的那条路径（`State` 在单测里用 `AppState::for_test()`，
    /// 与 T1 的 core 单测同一套夹具口径）：
    /// ① 重命名生效、重名报可读错误；
    /// ② 置顶/颜色/顺序落库后按侧栏口径回读（置顶优先 → 手动顺序）；
    /// ③ `delete_tag` 的 dry_run 预览数 = 真删影响数（同源计数函数），且只清关联：
    ///    标签消失、`entry_tags` 关联清空、**文章仍在**；
    /// ④ 标签区折叠状态：默认展开、可写可读、写坏值仍展开（不让坏设置卡死区块）。
    #[test]
    fn tag_management_command_path_persists_and_delete_is_previewed() {
        let state = AppState::for_test();
        let feed = state
            .with_store(|s| s.add_feed("https://example.com/a.xml", Some("A源")).map_err(err))
            .unwrap();
        let entry = rustrss_core::Entry {
            stable_id: "a1".into(),
            id_origin: rustrss_core::IdOrigin::SourceData,
            source_id: "a1".into(),
            title: "a1".into(),
            url: Some("https://example.com/a1".into()),
            author: None,
            published: None,
            updated: None,
            summary: Some("正文".into()),
            content_html: Some("<p>正文</p>".into()),
            content_text: Some("正文".into()),
            categories: Vec::new(),
        };
        state
            .with_store(|s| s.upsert_entries(feed, &[entry]).map_err(err))
            .unwrap();

        let (alpha, beta, gamma) = state
            .with_store(|s| {
                let a = s.create_tag("alpha", Some("#e5484d")).map_err(err)?;
                let b = s.create_tag("beta", None).map_err(err)?;
                let g = s.create_tag("gamma", None).map_err(err)?;
                Ok((a, b, g))
            })
            .unwrap();
        state
            .with_store(|s| {
                s.assign_tags(
                    &rustrss_core::TagTarget::Entries(vec![1]),
                    &[alpha.id, beta.id],
                )
                .map_err(err)
            })
            .unwrap();

        // ① 重命名：生效 + 重名（大小写不敏感）回可读错误
        let renamed = state
            .with_store(|s| s.rename_tag(gamma.id, "gamma2").map_err(err))
            .unwrap();
        assert_eq!(renamed.name, "gamma2");
        let dup = state.with_store(|s| s.rename_tag(gamma.id, "ALPHA").map_err(err));
        let msg = dup.expect_err("与已有标签仅大小写不同也应拒绝");
        assert!(msg.contains("标签名已存在"), "重名错误要可读: {msg}");

        // ② 置顶 / 颜色 / 顺序：回读口径 = 置顶优先 → sort_order
        state
            .with_store(|s| s.set_tag_pinned(beta.id, true).map_err(err))
            .unwrap();
        state
            .with_store(|s| s.set_tag_color(alpha.id, Some("#3E63DD")).map_err(err))
            .unwrap();
        state
            .with_store(|s| s.reorder_tags(&[gamma.id, beta.id, alpha.id]).map_err(err))
            .unwrap();
        let rows = state.with_store(|s| s.list_tags().map_err(err)).unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["beta", "gamma2", "alpha"], "置顶的排最前");
        assert_eq!(rows[2].color.as_deref(), Some("#3e63dd"), "颜色归一为小写落库");

        // ③ dry_run 预览与真删同源；只清关联，文章保留
        let preview = state
            .with_store(|s| s.delete_tag(alpha.id, true).map_err(err))
            .unwrap();
        assert!(preview.dry_run);
        assert_eq!(preview.affected_entries, 1, "alpha 关联 1 篇");
        assert!(
            state
                .with_store(|s| Ok(s.tag_row(alpha.id).map_err(err)?.is_some()))
                .unwrap(),
            "dry_run 不得落库"
        );
        let report = state
            .with_store(|s| s.delete_tag(alpha.id, false).map_err(err))
            .unwrap();
        assert!(!report.dry_run);
        assert_eq!(
            report.affected_entries, preview.affected_entries,
            "预览数与真删影响数必须同源"
        );
        assert!(
            state
                .with_store(|s| Ok(s.tag_row(alpha.id).map_err(err)?.is_none()))
                .unwrap(),
            "标签已删除"
        );
        let kept: Vec<String> = state
            .with_store(|s| s.entry_tags(1).map_err(err))
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(kept, vec!["beta"], "entry_tags 里只该剩下没被删的标签");
        assert!(
            state
                .with_store(|s| Ok(s.get_entry(1).map_err(err)?.is_some()))
                .unwrap(),
            "删除标签不得动文章"
        );

        // ④ 折叠状态：默认展开；写了才折叠；写坏值不把区块卡在折叠态
        assert!(!state.with_store(|s| Ok(tags_collapsed_setting(s))).unwrap());
        state
            .with_store(|s| s.set_setting(TAGS_COLLAPSED_KEY, "true").map_err(err))
            .unwrap();
        assert!(state.with_store(|s| Ok(tags_collapsed_setting(s))).unwrap());
        state
            .with_store(|s| s.set_setting(TAGS_COLLAPSED_KEY, "yes").map_err(err))
            .unwrap();
        assert!(!state.with_store(|s| Ok(tags_collapsed_setting(s))).unwrap());
    }

    /// 日志级别归一化：只认 `debug`，其余（缺失/非法/未来新档）一律 `info`。
    /// 读侧（启动/回显）与写侧（`set_log_level`）共用这一个函数，坏值不会被带进 logger。
    #[test]
    fn log_level_filter_only_accepts_debug() {
        assert_eq!(log_level_filter("debug"), log::LevelFilter::Debug);
        assert_eq!(
            log_level_filter(" DEBUG "),
            log::LevelFilter::Debug,
            "大小写与空白都要归一化"
        );
        for raw in ["info", "", "  ", "trace", "warn", "garbage", "debugx"] {
            assert_eq!(
                log_level_filter(raw),
                log::LevelFilter::Info,
                "{raw:?} 应回落 info"
            );
            assert_eq!(
                normalize_log_level(raw),
                DEFAULT_LOG_LEVEL,
                "{raw:?} 归一化应回落默认档"
            );
        }
        assert_eq!(normalize_log_level("Debug"), "debug", "落库值是白名单内的小写档");
    }

    /// 测试内直读库里**原样**的 `log.level`（`ui_settings` 回显的是归一值，
    /// 看它区分不出「入库前钳位」与「读取时兜底」）。
    fn stored_log_level(state: &AppState) -> String {
        state
            .with_store(|s| {
                crate::ai::non_empty_setting(s, KEY_LOG_LEVEL)
                    .ok_or_else(|| "log.level 应已写入".to_string())
            })
            .unwrap()
    }

    /// 日志级别设置（T3 AC1/AC3）：缺失 → 默认 `info`；库里被写坏 → 回显回落 `info`；
    /// 写路径先把非法值钳位成 `info` 再落库；每次写入都**立刻**改全局级别（无需重启）。
    #[test]
    fn set_log_level_clamps_unknown_values_and_applies_immediately() {
        let state = AppState::for_test();

        // 缺失键：默认 info（不能因为设置没写过就没日志）
        assert_eq!(ui_settings(&state).unwrap().log_level, DEFAULT_LOG_LEVEL);

        // 库里被写坏（如 sqlite3 手工改库）：回显回落 info，且坏值不会被拿去设级别
        state
            .with_store(|s| s.set_setting(KEY_LOG_LEVEL, "verbose").map_err(err))
            .unwrap();
        assert_eq!(ui_settings(&state).unwrap().log_level, "info");
        assert_eq!(
            log_level_filter(&stored_log_level(&state)),
            log::LevelFilter::Info,
            "坏值不得进 logger"
        );

        // 写路径：大小写/空白归一后再落库，且改完立刻生效
        let applied = apply_log_level(&state, " DEBUG ").unwrap();
        assert_eq!(applied.log_level, "debug");
        assert_eq!(stored_log_level(&state), "debug", "库里存归一值");
        assert_eq!(
            log::max_level(),
            log::LevelFilter::Debug,
            "写入后立刻生效（不必重启）"
        );

        // 白名单外的值：库里落 info（不是原样写入），级别也立刻回落
        let applied = apply_log_level(&state, "trace").unwrap();
        assert_eq!(applied.log_level, "info", "白名单外回落 info");
        assert_eq!(stored_log_level(&state), "info", "非法值不得入库");
        assert_eq!(log::max_level(), log::LevelFilter::Info, "回落也要立刻生效");
    }

    /// 打码实现上提到 core 后，界面侧仍走同一实现（re-export 的冒烟断言）：
    /// `ui_log` 与逐源失败行用的就是这条路径，实现搬家不能把这里变成“没过 scrub”。
    #[test]
    fn ui_path_still_uses_the_shared_scrub() {
        let line = scrub_log_line("ai failed: https://api.test/v1?key=SECRET-9f3") ;
        assert!(!line.contains("SECRET-9f3"), "界面路径必须继续打码: {line}");
        assert!(line.contains("key=***"), "参数名要保留: {line}");
    }

    /// MCP 写能力设置（T2 AC1/AC6）：生成/轮换/销毁写 token + 两个开关都默认关。
    ///
    /// 这条用例同时钉住「设置页看到的」与「MCP 侧闸门读到的」是同一份存储：
    /// 生成写 token 后，直接把库里的开关快照喂给 `rustrss_mcp` 的注册表判断。
    #[test]
    fn mcp_write_settings_lifecycle_and_defaults() {
        use rustrss_mcp::registry::{self, Scope, Switches};

        let state = AppState::for_test();

        // ① 默认：写开关/危险开关关，写 token 不存在（装好即只读）
        let view = mcp_view(&state).unwrap();
        assert!(!view.write_enabled && !view.dangerous_enabled);
        assert!(view.write_token.is_none(), "写 token 不得默认生成");

        // ② 生成：48 位十六进制；重复生成不覆盖（幂等）
        let first = mcp_generate_write_token(&state).unwrap();
        let token = first.write_token.clone().expect("生成后应有写 token");
        assert_eq!(token.len(), rustrss_mcp::config::WRITE_TOKEN_HEX_LEN);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        let again = mcp_generate_write_token(&state).unwrap();
        assert_eq!(again.write_token.as_deref(), Some(token.as_str()), "生成不得覆盖已有值");

        // ③ 开关落到库：写工具在“写 token + 总开关”都就位后才会被登记
        state
            .with_store(|s| {
                s.set_bool_setting(crate::mcp_server::K_WRITE_ENABLED, true)
                    .map_err(err)
            })
            .unwrap();
        let switches = state
            .with_store(|s| Ok(rustrss_mcp::config::switches_from_store(s).unwrap()))
            .unwrap();
        assert_eq!(
            switches,
            Switches {
                write_enabled: true,
                dangerous_enabled: false,
                write_token: true
            }
        );
        let write_spec = registry::ToolSpec {
            name: "stub_write",
            scope: Scope::Write,
            dangerous: false,
        };
        assert!(registry::authorize(&write_spec, Scope::Write, &switches).is_ok());
        let dangerous_spec = registry::ToolSpec {
            name: "stub_unsubscribe",
            scope: Scope::Write,
            dangerous: true,
        };
        assert_eq!(
            registry::authorize(&dangerous_spec, Scope::Write, &switches)
                .unwrap_err()
                .code(),
            "dangerous_tool_disabled",
            "危险开关未开时危险工具不可用"
        );

        // ④ 危险开关也能单独打开
        let view = mcp_set_dangerous_enabled(&state, true).unwrap();
        assert!(view.dangerous_enabled);
        let switches = state
            .with_store(|s| Ok(rustrss_mcp::config::switches_from_store(s).unwrap()))
            .unwrap();
        assert!(registry::authorize(&dangerous_spec, Scope::Write, &switches).is_ok());

        // ⑤ 轮换：库里的值换成新的（旧值下一个请求即失效，靠的是“每请求现读”）
        let rotated = mcp_rotate_write_token(&state).unwrap();
        let fresh = rotated.write_token.expect("轮换后应有值");
        assert_ne!(fresh, token);
        assert_eq!(fresh.len(), rustrss_mcp::config::WRITE_TOKEN_HEX_LEN);

        // ⑥ 销毁：键被删除，写工具随之不再注册
        let cleared = mcp_clear_write_token(&state).unwrap();
        assert!(cleared.write_token.is_none());
        let switches = state
            .with_store(|s| Ok(rustrss_mcp::config::switches_from_store(s).unwrap()))
            .unwrap();
        assert!(!switches.write_token);
        assert_eq!(
            registry::authorize(&write_spec, Scope::Write, &switches)
                .unwrap_err()
                .code(),
            "write_disabled",
            "写 token 不存在时写工具不可用"
        );

        // ⑦ 总开关也能关回去（回滚路径）
        let view = mcp_set_write_enabled(&state, false).unwrap();
        assert!(!view.write_enabled);
    }

    #[test]
    fn cursor_pair_needs_both_halves() {
        assert_eq!(cursor_pair(Some(1_700_000_000), Some(42)), Some((1_700_000_000, 42)));
        assert_eq!(cursor_pair(Some(1_700_000_000), None), None, "只给 sortkey 不成游标");
        assert_eq!(cursor_pair(None, Some(42)), None, "只给 id 不成游标");
        assert_eq!(cursor_pair(None, None), None, "无游标 = 首页");
    }

    #[test]
    fn restore_confirm_text_is_bilingual_and_warns_about_restart() {
        let path = "/tmp/RustRss-backup-20260921-120000.sqlite";
        let zh = restore_confirm_text(path, false);
        assert!(zh.contains(path), "确认框要显示用户选的具体备份路径");
        assert!(zh.contains("下次启动"), "必须说明重启后生效，实际: {zh}");
        assert!(zh.contains(".bak-"), "要预告现库会另存为 bak，实际: {zh}");
        assert!(!zh.is_ascii(), "zh 分支不该是英文文案");

        let en = restore_confirm_text(path, true);
        assert!(en.contains(path));
        assert!(en.contains("next time RustRss starts"), "实际: {en}");
        assert!(en.contains(".bak-"));
        assert!(en.is_ascii(), "en 分支不该混中文（路径本身是 ASCII）");
    }

    #[test]
    fn locale_whitelist() {
        assert_eq!(normalize_locale("zh-CN"), "zh-CN");
        assert_eq!(normalize_locale(" en "), "en");
        assert_eq!(normalize_locale("auto"), "auto");
        assert_eq!(normalize_locale("fr"), "auto");
        assert_eq!(normalize_locale(""), "auto");
    }

    /// 列表排序 / 隐藏已读的命令体：白名单归一 + 落库 + 回 UiSettings（菜单与设置页的
    /// 回显数据源）；非法排序值归默认档而不是把垃圾写进库；两键互不干扰。
    #[test]
    fn list_sort_and_hide_read_command_path_round_trips() {
        let state = AppState::for_test();

        // 默认：newest + 不隐藏（与加排序功能前的行为一致）
        let s = ui_settings(&state).unwrap();
        assert_eq!(s.list_sort, "newest");
        assert!(!s.list_hide_read);

        // 三档往返；非法值（含前后空白、大小写变体、空串）一律归 newest
        for (input, expected) in [
            ("newest", "newest"),
            (" oldest ", "oldest"),
            ("unread_first", "unread_first"),
            ("OLDEST", "newest"),
            ("unread", "newest"),
            ("", "newest"),
        ] {
            let s = apply_list_sort(&state, input).unwrap();
            assert_eq!(s.list_sort, expected, "{input:?} 应归一为 {expected}");
            assert_eq!(
                state.with_store(|st| Ok(st.list_sort().as_str())).unwrap(),
                expected,
                "库里存的应是归一后的值（{input:?}）"
            );
        }

        // 隐藏已读开关往返
        let s = apply_list_hide_read(&state, true).unwrap();
        assert!(s.list_hide_read);
        assert!(state.with_store(|st| Ok(st.list_hide_read())).unwrap());
        assert_eq!(s.list_sort, "newest", "改开关不该动排序档");
        let s = apply_list_hide_read(&state, false).unwrap();
        assert!(!s.list_hide_read);

        // 两键独立持久化：重新读设置仍在（菜单勾选靠这份回显）
        apply_list_sort(&state, "oldest").unwrap();
        apply_list_hide_read(&state, true).unwrap();
        let s = ui_settings(&state).unwrap();
        assert_eq!(s.list_sort, "oldest");
        assert!(s.list_hide_read);
    }

    #[test]
    fn refresh_interval_whitelist() {
        for value in ["15", "30", "60", "120", "360"] {
            assert_eq!(normalize_refresh_interval(value), value);
        }
        assert_eq!(normalize_refresh_interval("off"), "off");
        assert_eq!(normalize_refresh_interval(" 15 "), "15", "前后空白应被容忍");
        // 非法值（含拼错、零、负数、空串）一律归默认 30
        for garbage in ["", "  ", "7", "0", "-15", "15m", "daily", "OFF", "3600"] {
            assert_eq!(
                normalize_refresh_interval(garbage),
                DEFAULT_REFRESH_INTERVAL,
                "{garbage:?} 应归默认"
            );
        }
    }

    #[test]
    fn refresh_interval_duration_tiers_and_off() {
        use std::time::Duration;
        assert_eq!(refresh_interval_duration("off"), None, "关 → 不调度");
        assert_eq!(
            refresh_interval_duration("15"),
            Some(Duration::from_secs(15 * 60))
        );
        assert_eq!(
            refresh_interval_duration("30"),
            Some(Duration::from_secs(30 * 60))
        );
        assert_eq!(
            refresh_interval_duration("60"),
            Some(Duration::from_secs(60 * 60))
        );
        assert_eq!(
            refresh_interval_duration("120"),
            Some(Duration::from_secs(120 * 60))
        );
        assert_eq!(
            refresh_interval_duration("360"),
            Some(Duration::from_secs(360 * 60))
        );
        // 非法值走默认档，而不是无声地关掉自动刷新
        assert_eq!(
            refresh_interval_duration("bogus"),
            Some(Duration::from_secs(30 * 60))
        );
        assert_eq!(
            refresh_interval_duration(""),
            Some(Duration::from_secs(30 * 60))
        );
    }

    #[test]
    fn font_family_normalization_trims_collapses_and_bounds() {
        assert_eq!(normalize_font_family("  Noto Sans CJK SC "), "Noto Sans CJK SC");
        assert_eq!(normalize_font_family("Foo\tBar"), "Foo Bar", "制表/换行不该进 CSS 值");
        assert_eq!(normalize_font_family("\n  "), "", "纯空白 = 跟随系统");
        assert_eq!(normalize_font_family("霞鹜文楷"), "霞鹜文楷", "CJK 族名原样保留");
        // 控制字符被剔除（换行会把设置页下拉标签撑成两行）
        assert_eq!(normalize_font_family("Inter\r\n  UI"), "Inter UI");
        // 超长值截断：坏数据不该把 CSS 值/标签撑爆
        let long = "A".repeat(500);
        assert_eq!(normalize_font_family(&long).chars().count(), MAX_FONT_FAMILY_LEN);
        // 引号/逗号不在这里剥（前端加引号并转义），否则会把真实族名改错
        assert_eq!(normalize_font_family("Foo, Bar"), "Foo, Bar");
    }

    #[test]
    fn font_numbers_clamp_to_slider_ranges() {
        let size = FONT_READ_SIZE_RANGE;
        let line = FONT_READ_LINE_RANGE;
        assert_eq!(clamp_font_number(14.0, DEFAULT_FONT_READ_SIZE, size), 14.0);
        assert_eq!(clamp_font_number(9.0, DEFAULT_FONT_READ_SIZE, size), 13.0, "低于区间夹回下限");
        assert_eq!(clamp_font_number(99.0, DEFAULT_FONT_READ_SIZE, size), 18.0, "高于区间夹回上限");
        assert_eq!(clamp_font_number(1.55, DEFAULT_FONT_READ_LINE, line), 1.55);
        assert_eq!(clamp_font_number(0.5, DEFAULT_FONT_READ_LINE, line), 1.5);
        assert_eq!(clamp_font_number(3.0, DEFAULT_FONT_READ_LINE, line), 1.8);
        // 非有限值回默认（而不是把 NaN 落库）
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(clamp_font_number(bad, DEFAULT_FONT_READ_SIZE, size), DEFAULT_FONT_READ_SIZE);
        }
        // 两位小数：滑块步长 0.05，浮点尾噪不许进库
        assert_eq!(clamp_font_number(1.5500000000000003, DEFAULT_FONT_READ_LINE, line), 1.55);
        assert_eq!(clamp_font_number(1.5999999, DEFAULT_FONT_READ_LINE, line), 1.6);
    }

    #[test]
    fn ui_settings_reads_font_defaults_and_survives_broken_values() {
        let state = AppState::for_test();

        // 全新库：三类字体都跟随系统，字号/行高回默认
        let fresh = ui_settings(&state).unwrap();
        assert_eq!(fresh.font_ui, "");
        assert_eq!(fresh.font_read, "");
        assert_eq!(fresh.font_mono, "");
        assert_eq!(fresh.font_read_size, DEFAULT_FONT_READ_SIZE);
        assert_eq!(fresh.font_read_line, DEFAULT_FONT_READ_LINE);

        // 库里被写坏：非数字/超区间值都得夹回或回默认，而不是让正文排版崩掉
        state
            .with_store(|s| {
                s.set_setting(KEY_FONT_READ_SIZE, "9").map_err(err)?;
                s.set_setting(KEY_FONT_READ_LINE, "not-a-number").map_err(err)?;
                s.set_setting(KEY_FONT_UI, "  Inter  ").map_err(err)
            })
            .unwrap();
        let broken = ui_settings(&state).unwrap();
        assert_eq!(broken.font_read_size, 13.0, "9 夹回下限");
        assert_eq!(broken.font_read_line, DEFAULT_FONT_READ_LINE, "非数字回默认");
        assert_eq!(broken.font_ui, "Inter", "读取时同样 trim");
    }

    /// 命令体 = 「归一 + 落库 + 回读」：只写显式给值的项，None 不动库里原有的值。
    #[test]
    fn set_font_config_writes_only_given_keys_and_clamps() {
        let state = AppState::for_test();

        // 第一次：只设 UI 字体（下拉只改自己那一项）
        let after_ui = set_font_config_core(
            &state,
            Some("Noto Sans CJK SC".into()),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(after_ui.font_ui, "Noto Sans CJK SC");
        assert_eq!(after_ui.font_read, "", "没传的项不该被写");
        assert_eq!(after_ui.font_read_size, DEFAULT_FONT_READ_SIZE);

        // 第二次：滑块只写字号，越界值夹回上限；已设的字体族不被冲掉
        let after_size = set_font_config_core(&state, None, None, None, Some(99.0), Some(0.4)).unwrap();
        assert_eq!(after_size.font_read_size, 18.0);
        assert_eq!(after_size.font_read_line, 1.5);
        assert_eq!(after_size.font_ui, "Noto Sans CJK SC", "滑块不该动字体族");

        // 第三次：三类字体互不影响，等等宽字体只改 code 那一项
        let after_mono = set_font_config_core(&state, None, None, Some("JetBrains Mono".into()), None, None)
            .unwrap();
        assert_eq!(after_mono.font_mono, "JetBrains Mono");
        assert_eq!(after_mono.font_ui, "Noto Sans CJK SC");
        assert_eq!(after_mono.font_read, "", "正文字体仍是跟随 UI 字体");

        // 库内数值是干净字符串（14.0 存 "14"，不带 \".0\"）
        let stored: Vec<(String, String)> = state
            .with_store(|s| Ok(s.all_settings().unwrap()))
            .unwrap();
        let get = |key: &str| {
            stored
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(get(KEY_FONT_READ_SIZE), "18");
        assert_eq!(get(KEY_FONT_READ_LINE), "1.5");

        // 空串 = 恢复跟随系统（清除变量走前端，库里存空值）
        let cleared = set_font_config_core(&state, Some(String::new()), None, None, None, None).unwrap();
        assert_eq!(cleared.font_ui, "", "空串应恢复「跟随系统」");
        assert_eq!(cleared.font_mono, "JetBrains Mono", "清一个不该动另一个");
    }

    #[test]
    fn parse_font_families_sorts_dedups_and_drops_blanks() {
        let raw = "Noto Sans CJK SC\nDejaVu Sans\n\n  Noto Sans CJK SC  \nInter\n";
        assert_eq!(
            parse_font_families(raw),
            vec![
                "DejaVu Sans".to_string(),
                "Inter".to_string(),
                "Noto Sans CJK SC".to_string()
            ]
        );
        assert!(parse_font_families("").is_empty(), "空输出 → 空表");
        assert!(parse_font_families("\n \n").is_empty(), "只有空白行 → 空表");
    }

    /// 环境相关的两条分支都要成立：装了 fc-list 就有非空有序列表，
    /// 没装（或非 Linux）就是空表 —— 后者是「降级而不是报错」的那条验收点。
    #[tokio::test]
    async fn probe_font_families_matches_environment() {
        let families = probe_font_families().await;
        if fc_list_available() {
            assert!(!families.is_empty(), "有 fc-list 时应枚举出字体族");
            assert!(
                families.windows(2).all(|w| w[0] < w[1]),
                "应已排序且无重复"
            );
        } else {
            assert!(families.is_empty(), "拿不到字体列表时必须降级为空表");
        }
    }

    /// spawn 失败（未装 fontconfig 的机器）：空表，不报错、不 panic。
    #[tokio::test]
    async fn run_font_command_degrades_when_binary_missing() {
        let families = run_font_command("rustrss-definitely-no-such-binary", &[], std::time::Duration::from_secs(1)).await;
        assert!(families.is_empty(), "二进制不存在 → 空表");
    }

    /// 超时：卡住的子进程被放弃并杀掉，调用方在超时后立刻拿到空表（而不是无限等）。
    #[cfg(unix)]
    #[tokio::test]
    async fn run_font_command_times_out_and_kills_child() {
        let started = std::time::Instant::now();
        let families = run_font_command(
            "sh",
            &["-c", "sleep 30"],
            std::time::Duration::from_millis(200),
        )
        .await;
        let elapsed = started.elapsed();
        assert!(families.is_empty(), "超时 → 空表");
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "必须在超时后马上返回，实际等了 {elapsed:?}"
        );
    }

    /// fc-list 是否可用（测试环境相关，两条分支都断言）。`cfg!` 而不是 `#[cfg]`：
    /// 与 `probe_font_families` 同一口径，非 Linux 平台一样能编译到这里。
    fn fc_list_available() -> bool {
        if !cfg!(target_os = "linux") {
            return false;
        }
        std::process::Command::new("fc-list")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[test]
    fn ui_settings_normalizes_refresh_values() {
        let state = AppState::for_test();

        // 两个 key 都没写过：间隔走默认 30，启动刷新默认开
        let fresh = ui_settings(&state).unwrap();
        assert_eq!(fresh.refresh_interval_minutes, "30");
        assert!(fresh.refresh_on_start);

        // 库里被写坏（拼错的值）：读出来必须是默认，而不是把调度器搞成静默关闭
        state
            .with_store(|s| {
                s.set_setting(KEY_REFRESH_INTERVAL, "every-30-min")
                    .map_err(err)?;
                s.set_setting(KEY_REFRESH_ON_START, "maybe").map_err(err)
            })
            .unwrap();
        let broken = ui_settings(&state).unwrap();
        assert_eq!(broken.refresh_interval_minutes, DEFAULT_REFRESH_INTERVAL);
        assert!(broken.refresh_on_start, "非法布尔值应回退默认 true");

        // 合法值原样回显，前端 select 直接拿它当 value
        state
            .with_store(|s| {
                s.set_setting(KEY_REFRESH_INTERVAL, "off").map_err(err)?;
                s.set_bool_setting(KEY_REFRESH_ON_START, false).map_err(err)
            })
            .unwrap();
        let saved = ui_settings(&state).unwrap();
        assert_eq!(saved.refresh_interval_minutes, "off");
        assert!(!saved.refresh_on_start);
    }

    #[test]
    fn refresh_concurrency_whitelist_roundtrip() {
        let state = AppState::for_test();

        // 没写过：默认 6
        assert_eq!(ui_settings(&state).unwrap().refresh_concurrency, 6);

        // 白名单值原样保存与回显（set_refresh_concurrency 命令入口先把非法值
        // 归一化再写库，这里验写入-读取往返与读取侧兑底）
        for ok in [3u32, 6, 12, 24] {
            state
                .with_store(|s| {
                    s.set_setting(KEY_REFRESH_CONCURRENCY, &ok.to_string())
                        .map_err(err)
                })
                .unwrap();
            assert_eq!(ui_settings(&state).unwrap().refresh_concurrency, ok);
        }

        // 命令入口的归一化：档位外的值一律归默认 6
        for bad in [0u32, 1, 5, 64, 999] {
            assert_eq!(normalize_refresh_concurrency(bad), 6, "非法档位 {bad}");
        }

        // 库里被写坏（非数字/越界数字）：读出来必须是默认，调度器不因此埪埪
        for bad in ["lots", "0", "5", "64"] {
            state
                .with_store(|s| s.set_setting(KEY_REFRESH_CONCURRENCY, bad).map_err(err))
                .unwrap();
            assert_eq!(
                ui_settings(&state).unwrap().refresh_concurrency,
                6,
                "坏值 {bad}"
            );
        }
    }

    #[test]
    fn feed_refresh_interval_normalization_uses_shared_whitelist() {
        use normalize_feed_refresh_interval as n;
        assert_eq!(n(None), Ok(None), "缺省 = 跟随全局");
        assert_eq!(n(Some("global")), Ok(None));
        assert_eq!(n(Some(" global ")), Ok(None), "容忍前后空白");
        assert_eq!(n(Some("")), Ok(None));
        // 白名单与全局档共用同一张表：每个档位都算出一个分钟数
        for (label, minutes) in REFRESH_INTERVAL_CHOICES {
            assert_eq!(
                n(Some(label)),
                Ok(Some(minutes as i64)),
                "{label} 档应归一为 {minutes} 分钟"
            );
        }
        // 非法值报错而不是静默套默认档（单源是显式选择）
        for garbage in ["off", "0", "7", "15m", "3600", "daily", " GLOBAL "] {
            assert!(
                n(Some(garbage)).is_err(),
                "{garbage:?} 应被拒绝（不允许静默套档）"
            );
        }
    }

    /// 命令体 = 「归一 + 落库」两步（`State` 不好在单测里构造，所以用 for_test 的
    /// AppState 走同一条核心路径）：归一后的档位落库可读回，「跟随全局」能恢复 NULL。
    #[test]
    fn feed_refresh_interval_command_path_persists_and_restores() {
        let state = AppState::for_test();
        let feed_id = state
            .with_store(|s| s.add_feed("https://example.com/f.xml", None).map_err(err))
            .unwrap();

        let minutes = normalize_feed_refresh_interval(Some("15")).unwrap();
        let row = state
            .with_store(|s| s.set_feed_refresh_interval(feed_id, minutes).map_err(err))
            .unwrap();
        assert_eq!(row.refresh_interval_minutes, Some(15));
        assert_eq!(
            state
                .with_store(|s| s.feeds_with_interval().map_err(err))
                .unwrap(),
            vec![(feed_id, Some(15), None)],
            "调度扫描应看到覆盖档位"
        );

        let minutes = normalize_feed_refresh_interval(Some("global")).unwrap();
        let row = state
            .with_store(|s| s.set_feed_refresh_interval(feed_id, minutes).map_err(err))
            .unwrap();
        assert_eq!(row.refresh_interval_minutes, None, "「跟随全局」恢复 NULL");
    }

    // ------------------------------------------------- 订阅源编辑（set_feed_config）

    /// 补丁对象的解析口径：键缺省与 `null` 都不是「清除」——只有显式给值才写。
    /// 这是「部分写入不动其它字段」的前置条件，所以逐字段验一遍。
    #[test]
    fn feed_config_patch_distinguishes_absent_from_clear() {
        // 空对象：三个字段都「不动」
        let empty: FeedConfigPatch = serde_json::from_str("{}").unwrap();
        assert!(empty.custom_title.is_none());
        assert!(empty.folder_id.is_none());
        assert!(empty.refresh_interval.is_none());

        // 显式 null：customTitle / refreshInterval 仍是不动；folderId 的 null 是「移出到未分组」
        let nulls: FeedConfigPatch =
            serde_json::from_str(r#"{"customTitle":null,"folderId":null,"refreshInterval":null}"#)
                .unwrap();
        assert_eq!(nulls.custom_title, None, "null = 不动");
        assert_eq!(nulls.refresh_interval, None, "null = 不动");
        assert_eq!(
            nulls.folder_id,
            Some(FolderPatch::Ungrouped),
            "folderId=null 是移出到未分组（与 assign_feed_folder 同义）"
        );

        // 显式值（camelCase 键名，与 JS 侧一致）
        let full: FeedConfigPatch = serde_json::from_str(
            r#"{"customTitle":"我的名","folderId":7,"refreshInterval":"30"}"#,
        )
        .unwrap();
        assert_eq!(full.custom_title.as_deref(), Some("我的名"));
        assert_eq!(full.folder_id, Some(FolderPatch::Id(7)));
        assert_eq!(full.refresh_interval.as_deref(), Some("30"));

        // 空串 = 显式清除自定义名（与 null 区分）
        let clear: FeedConfigPatch = serde_json::from_str(r#"{"customTitle":""}"#).unwrap();
        assert_eq!(clear.custom_title.as_deref(), Some(""));
    }

    #[test]
    fn custom_title_normalization_trims_clears_and_caps() {
        assert_eq!(normalize_custom_title(""), None, "空串 = 清除");
        assert_eq!(normalize_custom_title("   \t "), None, "纯空白 = 清除");
        assert_eq!(normalize_custom_title("  我的贴名  ").as_deref(), Some("我的贴名"));
        let long = "字".repeat(MAX_CUSTOM_TITLE_LEN + 50);
        let capped = normalize_custom_title(&long).unwrap();
        assert_eq!(capped.chars().count(), MAX_CUSTOM_TITLE_LEN, "按字符截断");
    }

    /// 命令体走 `AppState::for_test`（真实内存库）：只写补丁里给出的字段，
    /// 其余字段一个都不动；不存在的源给可读错误。
    #[test]
    fn set_feed_config_writes_only_patched_fields() {
        let state = AppState::for_test();
        let (feed_id, other) = state
            .with_store(|s| {
                let a = s.add_feed("https://example.com/f.xml", Some("源站名")).map_err(err)?;
                let b = s.add_feed("https://example.com/g.xml", Some("另一源")).map_err(err)?;
                let folder = s.add_folder("开发").map_err(err)?;
                s.assign_folder(a, Some(folder)).map_err(err)?;
                s.set_feed_refresh_interval(a, Some(60)).map_err(err)?;
                Ok((a, b))
            })
            .unwrap();

        // 只改标题：文件夹/间隔原样不动
        let row = set_feed_config_core(
            &state,
            feed_id,
            &FeedConfigPatch {
                custom_title: Some("  我的贴名  ".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(row.title, "我的贴名", "空白应被 trim");
        assert_eq!(row.source_title, "源站名");
        assert_eq!(row.custom_title.as_deref(), Some("我的贴名"));
        assert!(row.folder_id.is_some(), "未给的 folderId 不该被清掉");
        assert_eq!(row.refresh_interval_minutes, Some(60), "未给的间隔不该被清掉");
        // 别的源不受影响
        let other_row = state.with_store(|s| s.feed_row(other).map_err(err)).unwrap().unwrap();
        assert_eq!(other_row.title, "另一源");
        assert_eq!(other_row.custom_title, None);

        // 只改间隔：自定义名与文件夹不动
        let row = set_feed_config_core(
            &state,
            feed_id,
            &FeedConfigPatch {
                refresh_interval: Some("global".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(row.refresh_interval_minutes, None, "global = 恢复跟随全局");
        assert_eq!(row.custom_title.as_deref(), Some("我的贴名"), "自定义名不该被清掉");
        assert!(row.folder_id.is_some(), "文件夹不该被清掉");

        // 只改文件夹到未分组：自定义名保留
        let row = set_feed_config_core(
            &state,
            feed_id,
            &FeedConfigPatch {
                folder_id: Some(FolderPatch::Ungrouped),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(row.folder_id, None, "null = 移出到未分组");
        assert_eq!(row.custom_title.as_deref(), Some("我的贴名"));

        // 空串 = 清除自定义：显示名回退源站名
        let row = set_feed_config_core(
            &state,
            feed_id,
            &FeedConfigPatch {
                custom_title: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(row.custom_title, None);
        assert_eq!(row.title, "源站名", "清除后显示名回退源站名");

        // 非法间隔：可读错误，且一个字段都不写（先把自定义名设回去再试）
        set_feed_config_core(
            &state,
            feed_id,
            &FeedConfigPatch {
                custom_title: Some("还在".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let msg = set_feed_config_core(
            &state,
            feed_id,
            &FeedConfigPatch {
                custom_title: Some("新的名".to_string()),
                refresh_interval: Some("7".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(msg.contains("7"), "错误信息该点名非法值: {msg}");
        let row = state.with_store(|s| s.feed_row(feed_id).map_err(err)).unwrap().unwrap();
        assert_eq!(row.custom_title.as_deref(), Some("还在"), "非法间隔时不该写一半");

        // 不存在的源：可读错误
        let err = set_feed_config_core(
            &state,
            9999,
            &FeedConfigPatch {
                custom_title: Some("x".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("不存在"), "实际: {err}");
    }

    #[test]
    fn theme_whitelist() {
        assert_eq!(normalize_theme("light"), "light");
        assert_eq!(normalize_theme(" dark "), "dark");
        assert_eq!(normalize_theme("system"), "system");
        assert_eq!(normalize_theme("blue"), "system");
        assert_eq!(normalize_theme(""), "system");
    }

    // ------------------------------------------------------------ 获取全文

    fn fulltext_entry(stable_id: &str, url: &str, content: Option<&str>) -> rustrss_core::Entry {
        rustrss_core::Entry {
            stable_id: stable_id.to_string(),
            id_origin: rustrss_core::IdOrigin::SourceData,
            source_id: stable_id.to_string(),
            title: format!("标题 {stable_id}"),
            url: Some(url.to_string()),
            author: None,
            published: None,
            updated: None,
            summary: Some("源给的一句摘要".to_string()),
            content_html: content.map(|c| format!("<p>{c}</p>")),
            // 解析层在源没给正文时会用摘要兜底，这里照那个形状造数据
            content_text: Some(content.unwrap_or("源给的一句摘要").to_string()),
            categories: Vec::new(),
        }
    }

    /// 内存库 + 一条条目，返回它的行 id
    fn state_with_entry(entry: rustrss_core::Entry) -> (AppState, i64) {
        let state = AppState::for_test();
        let id = state
            .with_store(|s| {
                let feed_id = s
                    .add_feed("https://example.com/feed.xml", Some("源"))
                    .map_err(err)?;
                s.upsert_entries(feed_id, std::slice::from_ref(&entry))
                    .map_err(err)?;
                s.list_entries(&rustrss_core::EntryQuery::default())
                    .map_err(err)
                    .map(|rows| rows[0].id)
            })
            .expect("预置数据应成功");
        (state, id)
    }

    #[tokio::test]
    async fn fetch_fulltext_is_idempotent_and_touches_no_network() {
        // 全文型条目（源自带长正文）本来就不该抓。地址故意用保证解析不出的 .invalid：
        // 一旦真发了请求，这条断言就会拿到 Err 而不是 Ok，「零网络」不是靠自觉而是靠断言。
        let long = "文".repeat(600);
        let (state, id) = state_with_entry(fulltext_entry(
            "e-full",
            "https://nonexistent.invalid/post",
            Some(&long),
        ));

        let row = fetch_fulltext_core(&state, id)
            .await
            .expect("正文已够长：应直接回当前内容");
        assert!(!row.needs_fulltext, "已抓/全文型不该再提示获取全文");
        assert!(row.content_html.is_some(), "回读的应是库里的正文");
    }

    #[tokio::test]
    async fn fetch_fulltext_degrades_and_keeps_the_summary() {
        // 摘要型 + 连不上的地址（127.0.0.1:1 立即拒绝）：必须给可显示的错误，
        // 且库里状态原样（摘要还在、仍待抓 → 用户可重试）。
        let (state, id) =
            state_with_entry(fulltext_entry("e-sum", "http://127.0.0.1:1/post", None));

        let message = fetch_fulltext_core(&state, id)
            .await
            .expect_err("连不上应报错");
        assert!(
            message.contains("获取原文失败"),
            "错误要能直接显示给用户: {message}"
        );

        let row = state
            .with_store(|s| s.get_entry(id).map_err(err))
            .unwrap()
            .unwrap();
        assert_eq!(
            row.content_text.as_deref(),
            Some("源给的一句摘要"),
            "失败不该动库里的摘要"
        );
        assert!(row.needs_fulltext, "失败后仍待抓，可重试");
    }

    #[tokio::test]
    async fn fetch_fulltext_second_call_is_offline_after_write_back() {
        // 「重开零网络」：正文已在库里（第一次抓取成功的形状）+ 地址指向不可达主机。
        // 第二次调用若还发请求就必然报错，「直接读库」因此是被断言的行为。
        let (state, id) = state_with_entry(fulltext_entry(
            "e-done",
            "https://nonexistent.invalid/post",
            None,
        ));
        state
            .with_store(|s| s.set_fulltext(id, "<p>抓到的正文</p>", "抓到的正文").map_err(err))
            .unwrap();

        let row = fetch_fulltext_core(&state, id)
            .await
            .expect("已抓过：应零网络直接回库");
        assert_eq!(row.content_html.as_deref(), Some("<p>抓到的正文</p>"));
        assert!(!row.needs_fulltext);
    }

    /// 起一个只服务一次请求的最小 HTTP 服务器（回环地址，不碰外部网络），返回可抓取的 URL。
    async fn serve_once(body: String) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("应能绑定回环端口");
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}/post")
    }

    /// 一篇真实形状的页面：正文够长（写回后 needs_fulltext 才会转假）+ 页脚装饰
    fn article_page() -> String {
        let paragraph = "正文段落：HTTP 消息签名把完整性保证带进了应用层，签名基的构造是最容易写错的地方。"
            .repeat(24);
        format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>示例文章</title></head>\
             <body><nav><a href=\"/a\">导航一</a></nav>\
             <article><p>{paragraph}</p><p>收尾段落。</p></article>\
             <footer>页脚广告位</footer></body></html>"
        )
    }

    #[tokio::test]
    async fn fetch_fulltext_end_to_end_fetches_extracts_and_writes_back() {
        // 真跑一遍命令本体：抓取（回环 HTTP 服务器）→ 提取 → 写回 → 回读。
        let url = serve_once(article_page()).await;
        let (state, id) = state_with_entry(fulltext_entry("e-e2e", &url, None));

        let row = fetch_fulltext_core(&state, id)
            .await
            .expect("应抓到页面并写回正文");
        assert!(!row.needs_fulltext, "抓到后不该再提示获取全文");
        let html = row.content_html.expect("回读应带正文 HTML");
        assert!(html.contains("签名基的构造"), "正文应来自页面: {html}");
        assert!(!html.contains("页脚广告位"), "页脚装饰不该进正文");

        // 库里也真的写上了，并且新正文马上可搜（重开这篇文章只读库）
        let stored = state
            .with_store(|s| s.get_entry(id).map_err(err))
            .unwrap()
            .unwrap();
        assert!(
            stored.content_text.as_deref().unwrap().contains("消息签名"),
            "纯文本正文应已入库"
        );
        let hits = state
            .with_store(|s| s.search("签名基", 10).map_err(err))
            .unwrap();
        assert_eq!(hits.len(), 1, "写回后应能搜到新正文");
    }

    #[tokio::test]
    async fn fetch_fulltext_reports_missing_url() {
        let (state, id) = state_with_entry(fulltext_entry("e-nourl", "", None));
        let message = fetch_fulltext_core(&state, id)
            .await
            .expect_err("没有原文地址应报错");
        assert!(message.contains("没有原文地址"), "实际: {message}");
    }

    // ------------------------------------------------------------ 外链打开（安全）

    /// 合法 http/https 原样通过；首尾空白先 trim 再返回「清洗后」的 URL。
    #[test]
    fn external_url_accepts_http_and_https_verbatim() {
        for url in [
            "http://example.com/post",
            "https://example.com/post?a=1&b=2",
        ] {
            assert_eq!(validate_external_url(url).unwrap(), url, "合法链接应原样通过: {url}");
        }
        assert_eq!(
            validate_external_url("  https://example.com/post  ").unwrap(),
            "https://example.com/post",
            "首尾空白应被 trim"
        );

        // `&` 与 `%` 是合法 URL 字符（query 分隔符 / 百分号编码），不能拦住：
        // 命令注入真正的根治点是「不经 shell」，白名单只是纵深。
        let with_metachars = "https://example.com/?q=1&calc.exe&x=%PATH%";
        assert_eq!(validate_external_url(with_metachars).unwrap(), with_metachars);
    }

    /// 非 http/https 协议一律拒绝（`javascript:` / `file:` / `data:` 会被系统打开器当执行入口）。
    #[test]
    fn external_url_rejects_other_schemes() {
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "ftp://example.com/x",
            "https:/example.com",
            "",
            "   ",
        ] {
            let err = validate_external_url(url)
                .expect_err("非 http/https 应被拒绝");
            assert!(err.contains("只允许打开 http/https"), "{url:?} 的错误应说明协议要求，实际: {err}");
        }
    }

    /// 空白 / 控制字符（U+0000–U+001F、U+007F）与 shell 元字符一律拒绝 ——
    /// 这是纵深防御：即便将来某条路径真的经过 shell，这些字符也进不去。
    #[test]
    fn external_url_rejects_whitespace_control_and_metacharacters() {
        for (label, url) in [
            ("空格", "https://example.com/a b"),
            ("制表符", "https://example.com/a\tb"),
            ("换行", "https://example.com/a\nb"),
            ("回车", "https://example.com/a\rb"),
            ("NUL", "https://example.com/a\u{0}b"),
            ("U+001F", "https://example.com/a\u{1f}b"),
            ("DEL U+007F", "https://example.com/a\u{7f}b"),
            ("反引号", "https://example.com/a`whoami`"),
            ("双引号", "https://example.com/\"a\""),
            ("<", "https://example.com/<a>"),
            (">", "https://example.com/>a"),
            ("|", "https://example.com/a|b"),
            ("^", "https://example.com/a^b"),
        ] {
            let err = validate_external_url(url).expect_err("含空白/控制/元字符的链接应被拒绝");
            assert!(err.contains("不允许的字符"), "{label} 的错误应指出字符问题，实际: {err}");
        }
    }

    /// 打开命令：URL 必须原样作为**独立参数**传入，且程序名只可能是三平台打开器之一
    /// （Windows 曾用 `cmd /C start`，shell 会把 URL 里的 `&` 当命令分隔符 → 命令注入）。
    #[test]
    fn external_open_command_never_uses_a_shell() {
        let url = "https://example.com/?a=1&calc.exe";
        let command = external_open_command(url);
        let program = command.get_program().to_string_lossy().into_owned();
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        assert!(
            !program.contains("cmd") && !program.contains("sh"),
            "不允许经 shell 打开外链，实际程序: {program}"
        );
        assert!(
            matches!(program.as_str(), "rundll32" | "open" | "xdg-open"),
            "实际程序: {program}"
        );
        assert!(
            args.iter().any(|a| a == url),
            "URL 必须原样作为独立参数传入，实际参数: {args:?}"
        );
        assert!(
            !args.iter().any(|a| a == "/C" || a == "start"),
            "参数里不该出现 cmd 的 /C start，实际参数: {args:?}"
        );

        #[cfg(target_os = "windows")]
        assert_eq!(
            args,
            vec!["url.dll,FileProtocolHandler".to_string(), url.to_string()],
            "Windows 走 rundll32 url.dll,FileProtocolHandler <url>"
        );
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(args, vec![url.to_string()], "Linux 行为不变：xdg-open <url>");
        #[cfg(target_os = "macos")]
        assert_eq!(args, vec![url.to_string()], "macOS 行为不变：open <url>");
    }

    /// 打开日志目录的命令同口径：程序名只可能是三平台文件管理器启动器、绝不经 shell，
    /// 目录路径作为**唯一独立参数**原样传入（含空格与中文的路径也不能被拆开）。
    #[test]
    fn open_dir_command_never_uses_a_shell() {
        let dir = std::path::Path::new("/tmp/rustrss logs 测试 目录");
        let command = open_dir_command(dir);
        let program = command.get_program().to_string_lossy().into_owned();
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        assert!(
            !program.contains("cmd") && !program.contains("sh"),
            "不允许经 shell 打开目录，实际程序: {program}"
        );
        assert!(
            matches!(program.as_str(), "explorer" | "open" | "xdg-open"),
            "实际程序: {program}"
        );
        assert_eq!(
            args,
            vec![dir.to_string_lossy().into_owned()],
            "目录路径必须原样作为唯一独立参数"
        );
    }

    /// 启动器不存在 → 返回可读错误（含程序名与目录），而不是 panic。
    /// 「无文件管理器 / 未装 xdg-utils」的机器就走这条路径，错误文案要能直接展示给用户。
    #[test]
    fn spawn_dir_opener_reports_missing_launcher() {
        let dir = std::path::Path::new("/tmp/rustrss-logs-missing-launcher");
        let missing = std::process::Command::new("rustrss-no-such-file-manager-9f3c1a7b");
        let err = spawn_dir_opener(missing, dir).expect_err("启动器不存在时应返回 Err");
        assert!(err.contains("rustrss-no-such-file-manager-9f3c1a7b"), "错误里要有程序名: {err}");
        assert!(err.contains("/tmp/rustrss-logs-missing-launcher"), "错误里要有目录路径: {err}");
        assert!(err.contains("失败"), "错误要是可读提示: {err}");
    }
}

#[tauri::command]
pub fn mark_all_read(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    feed_id: Option<i64>,
) -> R<usize> {
    let n = state.with_store(|s| s.mark_all(scope_of(feed_id), true).map_err(err))?;
    sync_badge(&app, &state);
    Ok(n)
}

/// 全标已读的撤销（也用于误扫一遍之后的恢复）
#[tauri::command]
pub fn mark_all_unread(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    feed_id: Option<i64>,
) -> R<usize> {
    let n = state.with_store(|s| s.mark_all(scope_of(feed_id), false).map_err(err))?;
    sync_badge(&app, &state);
    Ok(n)
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
pub fn remove_feed(app: tauri::AppHandle, state: State<'_, AppState>, feed_id: i64) -> R<()> {
    state.with_store(|s| {
        s.remove_feed(feed_id).map_err(err)?;
        Ok(())
    })?;
    // 删源会级联删条目，未读数可能骤降：立刻摆正角标，不等下一轮后台刷新
    sync_badge(&app, &state);
    Ok(())
}

/// 刷新核心（手动 refresh_all / refresh_feeds 与定时/启动刷新共用）：
/// ① 锁内取任务 → ② 无锁并发抓取 → ③ 锁内串行写库 + WAL 收尾。
///
/// 单 flight 不在这里判定：调用方先拿 `try_begin_refresh` 的守卫，这样手动与自动
/// 走的一定是同一条管线，不会出现「两套路径各改一半」的漂移。
/// `feed_ids = None` 表示全量。`on_progress` 每抓完一个源回调一次（None = 不报进度，
/// 单源刷新这类瞬时操作不需要）。
pub(crate) async fn refresh_core<P: Fn(rustrss_core::RefreshProgress) + Send + Sync + 'static>(
    state: &AppState,
    feed_ids: Option<Vec<i64>>,
    concurrency: usize,
    on_progress: Option<P>,
) -> R<RefreshReport> {
    // 阶段一：锁内取任务（全量时读 id 列表与取任务在同一个锁窗口里做完）
    let jobs = match feed_ids {
        Some(ids) => state.with_store(|s| rustrss_core::collect_jobs(s, &ids).map_err(err))?,
        None => state.with_store(|s| {
            let ids = s.all_feed_ids().map_err(err)?;
            rustrss_core::collect_jobs(s, &ids).map_err(err)
        })?,
    };
    if jobs.is_empty() {
        log::debug!("[rustrss] 刷新跳过: 没有可抓的源");
        return Ok(RefreshReport::default());
    }
    // 阶段二：无锁并发抓取（带进度回调时逐源发事件，前端状态栏实时显示 N/M）
    let fetcher = state.fetcher.clone();
    let started = std::time::Instant::now();
    log::debug!(
        "[rustrss] 刷新批次开始: 源={} 并发={concurrency}",
        jobs.len()
    );
    let results = match on_progress {
        Some(cb) => rustrss_core::fetch_jobs_with_progress(&fetcher, jobs, concurrency, cb).await,
        None => rustrss_core::fetch_jobs(&fetcher, jobs, concurrency).await,
    };
    // 阶段三：锁内串行写库
    let report = state.with_store(|s| {
        let r = rustrss_core::apply_results(s, results).map_err(err);
        // 大批量写入后收尾 WAL（同一连接、此刻无读者竞争，TRUNCATE 立即归零）
        if let Err(e) = s.checkpoint_wal() {
            log::warn!("[rustrss] WAL checkpoint 失败（不影响数据）: {e}");
        }
        r
    })?;
    // 批次级明细（源数/耗时/失败源）：debug 级，受 log.level 控制。
    // 逐源失败行的 url/error 先过 scrub：私有订阅地址可能把凭据放在查询串里。
    log::debug!(
        "[rustrss] 刷新批次完成: fetched={} not_modified={} inserted={} updated={} unchanged={} failures={} 耗时={}ms",
        report.fetched,
        report.not_modified,
        report.inserted,
        report.updated,
        report.unchanged,
        report.failures.len(),
        started.elapsed().as_millis()
    );
    for f in &report.failures {
        log::debug!(
            "[rustrss] 源抓取失败: feed={} url={} err={}",
            f.feed_id,
            scrub_log_line(&f.url),
            scrub_log_line(&f.error)
        );
    }
    Ok(report)
}

/// 刷新全部订阅源（界面按钮 / `r` 键）。逐源发 `refresh:progress` 事件，
/// 前端状态栏实时显示「N/M · 成功 X · 失败 Y」。
/// 尊重单 flight：已有刷新（定时或上一轮手动）在跑时直接返回可读错误，不叠加第二条管线。
#[tauri::command]
pub async fn refresh_all(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    concurrency: Option<usize>,
) -> R<RefreshReport> {
    let _flight = state.try_begin_refresh()?;
    refresh_core(&state, None, concurrency.unwrap_or(6), Some(progress_emitter(app))).await
}

/// 刷新指定一批订阅源（OPML 导入后只抓新增的那些）。
/// 与 refresh_all 共享单 flight 与管线，只差「抓哪些源」。
#[tauri::command]
pub async fn refresh_feeds(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    feed_ids: Vec<i64>,
    concurrency: Option<usize>,
) -> R<RefreshReport> {
    let _flight = state.try_begin_refresh()?;
    refresh_core(&state, Some(feed_ids), concurrency.unwrap_or(6), Some(progress_emitter(app))).await
}

/// 构造逐源进度的事件发射闭包：每抓完一个源向前端发 `refresh:progress`。
fn progress_emitter(
    app: tauri::AppHandle,
) -> impl Fn(rustrss_core::RefreshProgress) + Send + Sync + 'static {
    use tauri::Emitter;
    move |p| {
        let _ = app.emit(crate::scheduler::EVENT_REFRESH_PROGRESS, p);
    }
}

/// 刷新单个订阅源（失败源上的「重试」用它，新增订阅后的首抓也用它）。
/// 有意不占单 flight：这是「用户针对某个源」的小管线（concurrency 1），
/// 被后台全量刷新挡掉反而会把「新增订阅 → 首抓」这条主流程变成错误提示。
#[tauri::command]
pub async fn refresh_feed(
    state: State<'_, AppState>,
    feed_id: i64,
    concurrency: Option<usize>,
) -> R<RefreshReport> {
    refresh_core(&state, Some(vec![feed_id]), concurrency.unwrap_or(1), None::<fn(_)>).await
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

/// 备份数据库：选目录 → 在线快照导出（rusqlite backup API，导出期间库可继续读写）。
///
/// 返回产物路径；用户取消返回 None。产物是独立干净的库文件，可直接拷到另一台机器使用。
#[tauri::command]
pub async fn backup_db(app: tauri::AppHandle, state: State<'_, AppState>) -> R<Option<String>> {
    let picked = app.dialog().file().blocking_pick_folder();
    let Some(folder) = picked else {
        return Ok(None);
    };
    let dest_dir = folder.into_path().map_err(|e| format!("路径无效: {e}"))?;
    state
        .with_store(|s| rustrss_core::backup::export_backup(s, &dest_dir).map_err(err))
        .map(|path| {
            log::info!("[rustrss] 已导出备份: {}", path.display());
            Some(path.display().to_string())
        })
}

/// 恢复数据库：选文件 → 校验（只读打开 + `user_version`）→ 确认 → 暂存，重启后生效。
///
/// 返回暂存的备份路径（用户取消或放弃确认返回 None）。真正的替换在下次启动、任何连接打开
/// 之前由 `rustrss_core::backup::apply_pending_restore` 完成（退出时替换不可行：MCP 的第二条
/// 连接还活着，Windows 也不能 rename 打开中的文件）。
#[tauri::command]
pub async fn restore_db(app: tauri::AppHandle, state: State<'_, AppState>) -> R<Option<String>> {
    let picked = app
        .dialog()
        .file()
        .add_filter("SQLite", &["sqlite", "sqlite3", "db"])
        .blocking_pick_file();
    let Some(file_path) = picked else {
        return Ok(None);
    };
    let src = file_path.into_path().map_err(|e| format!("路径无效: {e}"))?;

    // 校验要拿「当前 schema 版本」，确认文案要跟随界面语言：同一把锁里读完就放（别跨 await 持锁）。
    let (current_version, locale) = state.with_store(|s| {
        Ok((
            s.schema_version().map_err(err)?,
            crate::ai::non_empty_setting(s, KEY_LOCALE).unwrap_or_default(),
        ))
    })?;
    // 校验失败直接报错返回：此步之前不落地任何文件，现库一个字节都不会动。
    rustrss_core::backup::validate_backup(&src, current_version).map_err(err)?;

    // `auto` 与读不到设置时按中文处理——与托盘菜单同一口径（Rust 侧没有系统语言，
    // 精确跟随系统语言需引入 sys-locale，v1 不做）。
    let en = normalize_locale(&locale) == "en";
    let confirmed = app
        .dialog()
        .message(restore_confirm_text(&src.display().to_string(), en))
        .title(if en { "Restore from backup" } else { "从备份恢复" })
        .buttons(MessageDialogButtons::OkCancel)
        .blocking_show();
    if !confirmed {
        return Ok(None);
    }

    let data_dir = rustrss_core::backup::data_dir_of(&state.db_path);
    rustrss_core::backup::stage_restore(&src, &data_dir).map_err(err)?;
    log::info!(
        "[rustrss] 已暂存恢复文件 {}（下次启动替换 {}）",
        src.display(),
        state.db_path.display()
    );
    Ok(Some(src.display().to_string()))
}

/// 恢复确认框文案（`en` = 英文界面，与托盘菜单同一套 locale 口径）。
///
/// 抽成函数是为了能在单测里机械核对两件事：双语都在、都带「重启后生效」提示——
/// 用户点了确认却没重启就以为已经恢复了，是这条流程最容易踩的坑。
fn restore_confirm_text(src: &str, en: bool) -> String {
    if en {
        format!(
            "Replace the current database with this backup?\n\n{src}\n\nThe current database is kept as a .bak-<timestamp> file first; the replacement happens the next time RustRss starts."
        )
    } else {
        format!(
            "用这个备份替换当前数据库？\n\n{src}\n\n当前数据库会先另存为 .bak-<时间戳> 保底回滚；替换在本应用下次启动时完成。"
        )
    }
}

/// 外链在交给系统打开器之前的硬化校验（纵深防御，不是唯一防线）。
///
/// 只放行 http/https，且拒绝空白、控制字符与 shell 元字符：URL 来自 feed，
/// 属不可信输入。注意 `&`、`%` 是合法 URL 字符（query 分隔符 / 百分号编码），
/// 白名单**不能**靠它们挡住命令注入 —— 根治点是 [`external_open_command`]
/// 始终把 URL 当独立进程参数传入、不经任何 shell 解析。
fn validate_external_url(url: &str) -> Result<String, String> {
    /// 各平台 shell 上有特殊含义、且不该出现在合法 URL 里的字符（不含 `&` `%`）。
    const FORBIDDEN: [char; 6] = ['"', '<', '>', '|', '^', '`'];

    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(format!("只允许打开 http/https 链接，收到: {trimmed}"));
    }
    if let Some(c) = trimmed
        .chars()
        .find(|c| c.is_whitespace() || c.is_control() || FORBIDDEN.contains(c))
    {
        return Err(format!("链接含不允许的字符 {c:?}（不允许空白、控制字符与 \" < > | ^ `）: {trimmed}"));
    }
    Ok(trimmed.to_string())
}

/// 组装「用系统默认程序打开 URL」的命令。
///
/// URL 一律作为**独立进程参数**传入，任何平台都不经 shell 解析。Windows 走
/// `rundll32 url.dll,FileProtocolHandler`：URL 若经 cmd.exe 解释，其中的 `&` 会被
/// 当命令分隔符、`%VAR%` 会被展开 —— 恶意 feed 的链接 + 用户单击即命令执行
/// （审计 P0-1），因此这条路径上不允许再出现任何 shell 启动器。
fn external_open_command(url: &str) -> std::process::Command {
    #[cfg(target_os = "windows")]
    let (program, prefix): (&str, &[&str]) = ("rundll32", &["url.dll,FileProtocolHandler"]);
    #[cfg(target_os = "macos")]
    let (program, prefix): (&str, &[&str]) = ("open", &[]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (program, prefix): (&str, &[&str]) = ("xdg-open", &[]);

    let mut command = std::process::Command::new(program);
    command.args(prefix).arg(url);
    command
}

/// 用系统默认浏览器打开链接。
///
/// 只允许 http/https —— 文章里的链接是不可信输入，绝不能把 file:// 之类
/// 交给系统打开器；也因此这里只用 `Command::new(程序).arg(地址)`，不经过 shell。
#[tauri::command]
pub fn open_external(url: String) -> R<()> {
    let url = validate_external_url(&url)?;
    let mut command = external_open_command(&url);
    let program = command.get_program().to_string_lossy().into_owned();
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("调用 {program} 失败: {e}"))
}

/// 平台文件管理器启动器：Linux `xdg-open` / macOS `open` / Windows `explorer`。
fn dir_opener_program() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "explorer"
    }
    #[cfg(target_os = "macos")]
    {
        "open"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "xdg-open"
    }
}

/// 组装「用系统文件管理器打开目录」的命令。
///
/// 与 [`external_open_command`] 同一份硬化口径：程序名固定，目录路径作为**独立进程
/// 参数**传入，任何平台都不经 shell 解析（Windows 上若走 `cmd /C start`，路径里的
/// `&` 之类会被当命令分隔符）。
fn open_dir_command(dir: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new(dir_opener_program());
    command.arg(dir);
    command
}

/// 启动已组装好的启动器；失败返回**可读错误**（含程序名、原因与目录），不 panic。
///
/// 机器上连启动器都没有时（裸容器 / 未装 xdg-utils）走的就是这条路径。启动器“存在但
/// 自己失败”（如 Xvfb 下没有文件管理器）在 `spawn` 之后才发生，进程已经起来了，这里
/// 看不见——那属于系统启动器的错误输出，不由本函数负责（见 README 日志一节）。
///
/// 文案把**原因排在目录路径前面**：状态栏按 72ch 截断，原因是被截掉后最影响判断的那半。
fn spawn_dir_opener(mut command: std::process::Command, dir: &std::path::Path) -> R<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("调用 {program} 打开日志目录失败: {e}（目录: {}）", dir.display()))
}

/// 关于页「打开日志目录」：确保目录存在后交给系统文件管理器（不内嵌日志查看器）。
///
/// 目录不存在时先建：一次日志都没落地时（例如首次启动后立刻点按钮）给文件管理器一个
/// 不存在的路径，用户只会看到系统那句含糊的报错。失败返回可读错误字符串，由界面放进
/// 状态栏——不弹窗、不 panic、不影响其它功能。
#[tauri::command]
pub fn open_logs_dir() -> R<()> {
    let dir = rustrss_core::paths::logs_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("创建日志目录 {} 失败: {e}", dir.display()))?;
    spawn_dir_opener(open_dir_command(&dir), &dir)
}

/// 前端诊断：进同一个日志文件（target=ui），保留 `[ui]` 前缀便于检索（headless 冒烟
/// 与人工排障都靠它核对界面状态）。
///
/// 落盘前过 [`scrub_log_line`]：前端会把 AI/MCP 的原始错误文本原样送进来（例如
/// Gemini 的 `?key=` 查询串、带凭据的订阅地址），而日志文件是要用户贴出去排障的。
#[tauri::command]
pub fn ui_log(line: String) {
    log::info!(target: "ui", "[ui] {}", scrub_log_line(&line));
}

/// 日志行打码：实现已上提到 `rustrss-core`（MCP 侧也复用同一份），
/// 这里保留 re-export 让界面侧的调用点（`ui_log`、逐源失败行）照旧直接用。
pub use rustrss_core::logging::scrub::scrub_log_line;

// ---------------------------------------------------------------- MCP

#[derive(Serialize)]
pub struct McpSettingsView {
    pub enabled: bool,
    pub port: u16,
    /// 读 token（常驻；随设置页一起下发给用户填客户端）
    pub token: String,
    pub running: bool,
    pub url: Option<String>,
    /// 可直接粘贴给客户端的配置片段（含一行命令）
    pub snippet: String,
    /// 服务是否只绑回环——界面上如实展示，而不是只写在文档里
    pub loopback_only: bool,
    /// 写能力总开关（`mcp.write_enabled`，默认关）
    pub write_enabled: bool,
    /// 危险工具开关（`mcp.dangerous_enabled`，默认关）
    pub dangerous_enabled: bool,
    /// 写 token（`mcp.write_token`）；`None` = 未生成 ⇒ 写工具不注册。
    /// 与读 token 同口径地回给界面：设置页要能展示/复制它。
    pub write_token: Option<String>,
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
        let write_token = crate::mcp_server::write_token_from_store(s)?;
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
            write_enabled: crate::mcp_server::write_enabled_from_store(s)?,
            dangerous_enabled: crate::mcp_server::dangerous_enabled_from_store(s)?,
            write_token,
        })
    })
}

#[tauri::command]
pub fn get_mcp_settings(state: State<'_, AppState>) -> R<McpSettingsView> {
    mcp_view(&state)
}

#[tauri::command]
pub async fn set_mcp_enabled(state: State<'_, AppState>, enabled: bool) -> R<McpSettingsView> {
    let port = state.with_store(|s| {
        s.set_bool_setting(crate::mcp_server::K_ENABLED, enabled)
            .map_err(err)?;
        Ok(crate::mcp_server::port_from_store(s))
    })?;
    if enabled {
        let db = state.db_path.clone();
        state
            .mcp
            .start(&db, port, state.refresh_gate())
            .await
            .map_err(|e| format!("启动 MCP HTTP 服务失败: {e}"))?;
    } else {
        state.mcp.stop();
    }
    mcp_view(&state)
}

#[tauri::command]
pub async fn set_mcp_port(state: State<'_, AppState>, port: u16) -> R<McpSettingsView> {
    let enabled = state.with_store(|s| {
        s.set_setting(crate::mcp_server::K_PORT, &port.to_string())
            .map_err(err)?;
        s.bool_setting(crate::mcp_server::K_ENABLED, false)
            .map_err(err)
    })?;
    if enabled {
        let db = state.db_path.clone();
        state
            .mcp
            .start(&db, port, state.refresh_gate())
            .await
            .map_err(|e| format!("换端口失败: {e}"))?;
    }
    mcp_view(&state)
}

/// 轮换读 token：换库里的值即可——鉴权按请求现读，旧值下一个请求就失效（无需重启）。
#[tauri::command]
pub async fn rotate_mcp_token(state: State<'_, AppState>) -> R<McpSettingsView> {
    state.with_store(|s| {
        let fresh = rustrss_mcp::http::generate_token();
        rustrss_mcp::config::set_token(s, &fresh).map_err(err)
    })?;
    mcp_view(&state)
}

/// 生成写 token（已存在则沿用，不覆盖——「生成」是显式动作，「轮换」才是换值）。
///
/// 逻辑放 `&AppState` 版本里，命令只是薄壳：这两条路径（界面点击 / 单测）
/// 因此走的是同一段代码。
fn mcp_generate_write_token(state: &AppState) -> R<McpSettingsView> {
    state.with_store(|s| {
        if crate::mcp_server::write_token_from_store(s)?.is_none() {
            let fresh = rustrss_mcp::config::generate_write_token();
            rustrss_mcp::config::set_write_token(s, &fresh).map_err(err)?;
        }
        Ok(())
    })?;
    mcp_view(state)
}

/// 轮换写 token：库里换成新值，旧值下一请求即失去写权限（含已建立的 HTTP 连接）。
fn mcp_rotate_write_token(state: &AppState) -> R<McpSettingsView> {
    state.with_store(|s| {
        let fresh = rustrss_mcp::config::generate_write_token();
        rustrss_mcp::config::set_write_token(s, &fresh).map_err(err)
    })?;
    mcp_view(state)
}

/// 销毁写 token：键被删除，写工具随之不再注册（与开关关闭是两件事，都要显式做）。
fn mcp_clear_write_token(state: &AppState) -> R<McpSettingsView> {
    state.with_store(|s| rustrss_mcp::config::clear_write_token(s).map_err(err))?;
    mcp_view(state)
}

/// 写能力总开关（默认关）。单独一个命令：开关不需要重传整张表单。
fn mcp_set_write_enabled(state: &AppState, enabled: bool) -> R<McpSettingsView> {
    state.with_store(|s| {
        s.set_bool_setting(crate::mcp_server::K_WRITE_ENABLED, enabled)
            .map_err(err)
    })?;
    mcp_view(state)
}

/// 危险工具开关（默认关）；关掉后 `unsubscribe` / `folder_delete` 不可用。
fn mcp_set_dangerous_enabled(state: &AppState, enabled: bool) -> R<McpSettingsView> {
    state.with_store(|s| {
        s.set_bool_setting(crate::mcp_server::K_DANGEROUS_ENABLED, enabled)
            .map_err(err)
    })?;
    mcp_view(state)
}

#[tauri::command]
pub async fn generate_mcp_write_token(state: State<'_, AppState>) -> R<McpSettingsView> {
    mcp_generate_write_token(&state)
}

#[tauri::command]
pub async fn rotate_mcp_write_token(state: State<'_, AppState>) -> R<McpSettingsView> {
    mcp_rotate_write_token(&state)
}

#[tauri::command]
pub async fn clear_mcp_write_token(state: State<'_, AppState>) -> R<McpSettingsView> {
    mcp_clear_write_token(&state)
}

#[tauri::command]
pub fn set_mcp_write_enabled(state: State<'_, AppState>, enabled: bool) -> R<McpSettingsView> {
    mcp_set_write_enabled(&state, enabled)
}

#[tauri::command]
pub fn set_mcp_dangerous_enabled(state: State<'_, AppState>, enabled: bool) -> R<McpSettingsView> {
    mcp_set_dangerous_enabled(&state, enabled)
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
    /// 单次回答的输出 token 上限（归一化后 256-32768，默认 4096）：
    /// 推理模型思考链也在这个预算里，太小会把正文轴掉
    pub max_output_tokens: u32,
    /// 思考强度（""=跟随模型默认；minimal/low/medium/high；仅 OpenAI 兼容接口发送）
    pub reasoning_effort: String,
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
            max_output_tokens: crate::ai::max_output_tokens_from_store(s),
            reasoning_effort: crate::ai::reasoning_effort_from_store(s).unwrap_or_default(),
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

/// 思考强度开关（立即生效，无需点保存——独立于 AI 表单的调优旋钮）。
#[tauri::command]
pub fn set_ai_reasoning_effort(
    state: State<'_, AppState>,
    value: String,
) -> R<AiSettingsView> {
    let normalized = if crate::ai::REASONING_EFFORT_CHOICES.contains(&value.as_str()) {
        value
    } else {
        String::new()
    };
    state.with_store(|s| {
        s.set_setting(crate::ai::K_REASONING_EFFORT, &normalized)
            .map_err(err)
    })?;
    ai_settings_view(&state)
}

/// 保存 AI 设置。`api_key` 为 `Some("")` 表示清除凭据，`None` 表示不动它。
/// `max_output_tokens` 为 `None` 表示不动它（缺失/非法时读倒回退默认）。
#[tauri::command]
pub fn save_ai_settings(
    state: State<'_, AppState>,
    provider: String,
    model: String,
    base_url: String,
    translate_target: String,
    api_key: Option<String>,
    max_output_tokens: Option<u32>,
) -> R<AiSettingsView> {
    let provider = provider.trim().to_string();
    state.with_store(|s| {
        s.set_setting(crate::ai::K_PROVIDER, &provider).map_err(err)?;
        s.set_setting(crate::ai::K_MODEL, model.trim()).map_err(err)?;
        s.set_setting(crate::ai::K_BASE_URL, base_url.trim())
            .map_err(err)?;
        s.set_setting(crate::ai::K_TRANSLATE_TARGET, translate_target.trim())
            .map_err(err)?;
        if let Some(max) = max_output_tokens {
            let clamped = max.clamp(
                crate::ai::MAX_OUTPUT_TOKENS_LIMITS.0,
                crate::ai::MAX_OUTPUT_TOKENS_LIMITS.1,
            );
            s.set_setting(crate::ai::K_MAX_OUTPUT_TOKENS, &clamped.to_string())
                .map_err(err)?;
        }
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
    let started = std::time::Instant::now();
    let result = client.complete(request).await;
    // 只记结果与耗时：错误正文可能回显带凭据的端点 URL（如 Gemini 把 key 放在查询串里）
    log::debug!(
        "[rustrss] AI 连通性测试: {} 耗时={}ms",
        if result.is_ok() { "通过" } else { "失败" },
        started.elapsed().as_millis()
    );
    result.map_err(|e| e.to_string())
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
        log::debug!(
            "[rustrss] AI 命中缓存: model={} 字符={}",
            plan.provider_model,
            hit.chars().count()
        );
        return Ok(AiOutcomeView {
            output: hit,
            from_cache: true,
            truncated: plan.truncated,
            provider_model: plan.provider_model,
        });
    }

    let started = std::time::Instant::now();
    let output = client.complete(plan.request.clone()).await;
    // 只记 model / 成败 / 耗时：provider 的错误正文可能带端点凭据（界面会如实提示，但不落盘）
    log::debug!(
        "[rustrss] AI 请求完成: model={} {} 耗时={}ms",
        plan.provider_model,
        if output.is_ok() { "成功" } else { "失败" },
        started.elapsed().as_millis()
    );
    let output = output.map_err(|e| e.to_string())?;
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
