//! T3 的写工具：阅读状态（`set_read` / `set_starred` / `set_read_later`）、
//! `refresh`、`fetch_fulltext`。
//!
//! 三个状态工具是同一套实现的三张皮（只差「写哪一列 + 取值字段名」），所以目标解析、
//! 逐项结果与信封组装只写一份——三份复制正是口径漂移的起点。
//!
//! 口径（与 tech_design「写操作契约」对齐，agent 读得到）：
//! - 目标两种形态**二选一**：`ids[]`（≤100）或条件级 `{feed_id, since, until}`（至少一个）；
//!   混用/都不给都是 `invalid_argument`——条件级漏参绝不能退化成「全库」；
//! - `affected` 是**命中条数**（含此前已是目标状态的行），与 `set_read(ids)` 的 SQLite
//!   计数口径一致 → 重复调用返回值稳定（幂等）；
//! - `results` 是逐项结果：`ids` 形态逐个给（缺失的标 `article_not_found`），
//!   条件级形态给命中集里前 100 个 id（`detail.results_truncated` 标出截断）；
//! - `refresh` 与界面刷新抢**同一个** [`rustrss_core::RefreshGate`]：抢不到返回
//!   `rate_limited`（不排队、不叠加）；
//! - `fetch_fulltext` 复用既有全文抓取与 2MiB 流式闸门；成功只回元数据 + 提示，
//!   **不倒正文**（正文口径始终是 `get_article` 单取，见 PRD 的响应体积要求）。

use std::sync::Arc;

use rustrss_core::{
    apply_results, collect_jobs, fetch_jobs, fulltext, EntryFlag, EntryFlagScope, FulltextError,
    RefreshReport, Store,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::write_contract::{self, ItemResult, WriteOutcome, MAX_BATCH_IDS};
use crate::RustRssMcp;

/// 一次刷新最多同时抓几个源（与界面默认并发档一致）。
///
/// MCP 有意**不读**界面的并发设置：那个设置键与白名单定义在 `src-tauri`，
/// 跨 crate 复制一份只会等着漂移；刷新本身受单 flight 约束，不是并发风暴的来源。
const REFRESH_CONCURRENCY: usize = 6;

// ---------------------------------------------------------------- 错误码（机器可读，README 同步）

pub const ERROR_ARTICLE_NOT_FOUND: &str = "article_not_found";
pub const ERROR_FEED_NOT_FOUND: &str = "feed_not_found";
pub const ERROR_FOLDER_NOT_FOUND: &str = "folder_not_found";
pub const ERROR_INVALID_URL: &str = "invalid_url";
pub const ERROR_RATE_LIMITED: &str = "rate_limited";
pub const ERROR_FETCH_FAILED: &str = "fetch_failed";
pub const ERROR_FULLTEXT_TOO_LARGE: &str = "fulltext_too_large";
pub const ERROR_FULLTEXT_BOT_CHALLENGE: &str = "fulltext_bot_challenge";
pub const ERROR_FULLTEXT_NOT_HTML: &str = "fulltext_not_html";
pub const ERROR_FULLTEXT_NO_CONTENT: &str = "fulltext_no_content";
pub const ERROR_FULLTEXT_EXTRACT_FAILED: &str = "fulltext_extract_failed";
/// 进程内错误（库操作失败 / HTTP 客户端没建起来）：agent 不必分辨，但要知道不是它传错了
pub const ERROR_INTERNAL: &str = "internal_error";

// ---------------------------------------------------------------- 入参

/// `set_read` 的参数（`ids[]` 或条件级，二选一）
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SetReadParams {
    /// 目标条目 id（来自 list_articles / search_articles），最多 100 个
    pub ids: Option<Vec<i64>>,
    /// 条件级：只动这个订阅源（id 来自 list_feeds）
    pub feed_id: Option<i64>,
    /// 条件级：只动该时刻（含）之后的条目；比较键与列表排序键同源
    /// `COALESCE(published_at, fetched_at)`（Unix 秒）
    pub since: Option<i64>,
    /// 条件级：只动该时刻（含）之前的条目；闭区间，口径同 since
    pub until: Option<i64>,
    /// true（默认）= 标记为已读；false = 取消已读
    pub read: Option<bool>,
}

/// `set_starred` 的参数（形状同 `set_read`，取值字段是 `starred`）
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SetStarredParams {
    /// 目标条目 id（来自 list_articles / search_articles），最多 100 个
    pub ids: Option<Vec<i64>>,
    /// 条件级：只动这个订阅源（id 来自 list_feeds）
    pub feed_id: Option<i64>,
    /// 条件级：只动该时刻（含）之后的条目；口径同 set_read
    pub since: Option<i64>,
    /// 条件级：只动该时刻（含）之前的条目；口径同 set_read
    pub until: Option<i64>,
    /// true（默认）= 加星标；false = 取消星标
    pub starred: Option<bool>,
}

/// `set_read_later` 的参数（形状同 `set_read`，取值字段是 `later`）
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SetReadLaterParams {
    /// 目标条目 id（来自 list_articles / search_articles），最多 100 个
    pub ids: Option<Vec<i64>>,
    /// 条件级：只动这个订阅源（id 来自 list_feeds）
    pub feed_id: Option<i64>,
    /// 条件级：只动该时刻（含）之后的条目；口径同 set_read
    pub since: Option<i64>,
    /// 条件级：只动该时刻（含）之前的条目；口径同 set_read
    pub until: Option<i64>,
    /// true（默认）= 加入「稍后读」；false = 移出
    pub later: Option<bool>,
}

/// `refresh` 的参数
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RefreshParams {
    /// 刷新范围：`all`（全部源）/ `feed_ids`（配合 feed_ids）/ `folder`（配合 folder_id）；
    /// 不传时按给定参数推断（给了 feed_ids 就刷这批，给了 folder_id 就刷该组，否则全量）
    pub scope: Option<String>,
    /// `scope=feed_ids` 要刷的源（≤100）
    pub feed_ids: Option<Vec<i64>>,
    /// `scope=folder` 要刷的分组（id 来自 list_folders）
    pub folder_id: Option<i64>,
}

/// `fetch_fulltext` 的参数
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FetchFulltextParams {
    /// 条目 id（摘要型条目的正文补全；已抓过则零网络直接返回当前状态）
    pub id: i64,
}

/// 三个状态工具的公共执行形状（值字段名各工具不同 → 由各工具自己取好再进来）
#[derive(Debug, Default, Clone, Copy)]
struct FlagRequest<'a> {
    ids: Option<&'a [i64]>,
    feed_id: Option<i64>,
    since: Option<i64>,
    until: Option<i64>,
}

/// 参数校验失败：机器码 + 人读说明
#[derive(Debug, Clone, PartialEq, Eq)]
struct ArgError {
    code: &'static str,
    message: String,
}

impl ArgError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

// ---------------------------------------------------------------- 阅读状态

impl RustRssMcp {
    pub fn set_read_json(&self, p: &SetReadParams) -> String {
        self.set_flag_json(
            EntryFlag::Read,
            p.read.unwrap_or(true),
            FlagRequest {
                ids: p.ids.as_deref(),
                feed_id: p.feed_id,
                since: p.since,
                until: p.until,
            },
        )
    }

    pub fn set_starred_json(&self, p: &SetStarredParams) -> String {
        self.set_flag_json(
            EntryFlag::Starred,
            p.starred.unwrap_or(true),
            FlagRequest {
                ids: p.ids.as_deref(),
                feed_id: p.feed_id,
                since: p.since,
                until: p.until,
            },
        )
    }

    pub fn set_read_later_json(&self, p: &SetReadLaterParams) -> String {
        self.set_flag_json(
            EntryFlag::ReadLater,
            p.later.unwrap_or(true),
            FlagRequest {
                ids: p.ids.as_deref(),
                feed_id: p.feed_id,
                since: p.since,
                until: p.until,
            },
        )
    }

    /// 三个状态工具的共同执行体：解析目标形态 → 执行 → 统一信封
    fn set_flag_json(&self, flag: EntryFlag, value: bool, req: FlagRequest<'_>) -> String {
        let has_condition = req.feed_id.is_some() || req.since.is_some() || req.until.is_some();
        match (req.ids, has_condition) {
            // 混用两种形态时「影响面」无法解释（是并集还是交集？），直接判非法
            (Some(_), true) => WriteOutcome::failed_with(
                write_contract::ERROR_INVALID_ARGUMENT,
                "ids 与条件级参数（feed_id / since / until）只能二选一——混用会让影响面无法解释",
            )
            .to_json(),
            (None, false) => WriteOutcome::failed_with(
                write_contract::ERROR_INVALID_ARGUMENT,
                "缺少目标：请给 ids[]（≤100）或条件级参数（feed_id / since / until 至少一个）",
            )
            .to_json(),
            (Some(ids), false) => self.set_flag_by_ids(flag, value, ids),
            (None, true) => self.set_flag_by_scope(
                flag,
                value,
                &EntryFlagScope {
                    feed_id: req.feed_id,
                    since: req.since,
                    until: req.until,
                },
            ),
        }
    }

    /// ids 形态：逐项核对存在性（缺失 → `article_not_found`），只对存在的行落库
    fn set_flag_by_ids(&self, flag: EntryFlag, value: bool, ids: &[i64]) -> String {
        if let Err(e) = write_contract::check_batch_ids(ids) {
            return WriteOutcome::rejected(e).to_json();
        }
        let existing = match self.with_store(|s| s.existing_entry_ids(ids)) {
            Ok(v) => v,
            Err(e) => return internal_error(&e.to_string()),
        };
        // 逐项结果按调用方给的顺序（去重），不按库里的顺序——agent 好对账
        let mut seen = std::collections::HashSet::new();
        let results: Vec<ItemResult> = ids
            .iter()
            .filter(|id| seen.insert(**id))
            .map(|id| match existing.contains(id) {
                true => ItemResult::ok(*id),
                false => ItemResult::failed(*id, ERROR_ARTICLE_NOT_FOUND),
            })
            .collect();

        if existing.is_empty() {
            // 一条都不存在：整单失败（`ok=false` + 逐项原因），别让 agent 只看 `ok=true` 以为改到了
            return WriteOutcome {
                results,
                ..WriteOutcome::failed_with(
                    ERROR_ARTICLE_NOT_FOUND,
                    "给定的 id 一个都不存在；先用 list_articles / search_articles 取真实 id",
                )
            }
            .to_json();
        }

        match self.with_store(|s| set_flag_ids(s, flag, &existing, value)) {
            Ok(affected) => WriteOutcome::done(affected as i64, results, false).to_json(),
            Err(e) => internal_error(&e.to_string()),
        }
    }

    /// 条件级形态：命中集有界采样当逐项结果（同一个 WHERE，采样即会被写入的行）
    fn set_flag_by_scope(&self, flag: EntryFlag, value: bool, scope: &EntryFlagScope) -> String {
        let sample = match self.with_store(|s| s.entry_ids_scoped(scope, MAX_BATCH_IDS + 1)) {
            Ok(v) => v,
            Err(e) => return internal_error(&e.to_string()),
        };
        let truncated = sample.len() > MAX_BATCH_IDS;
        let results: Vec<ItemResult> = sample
            .into_iter()
            .take(MAX_BATCH_IDS)
            .map(ItemResult::ok)
            .collect();

        let affected = match self.with_store(|s| s.set_flag_scoped(flag, value, scope)) {
            Ok(n) => n as i64,
            Err(e) => return internal_error(&e.to_string()),
        };

        let mut detail = json!({ "target": scope_json(scope) });
        if truncated {
            detail["results_truncated"] = json!(true);
            detail["results_hint"] =
                json!("命中集超过 100 条：results 只列前 100 个 id（affected 是完整命中数）");
        }
        WriteOutcome::done(affected, results, false)
            .with_detail(detail)
            .to_json()
    }
}

fn scope_json(scope: &EntryFlagScope) -> Value {
    let mut map = serde_json::Map::new();
    if let Some(feed_id) = scope.feed_id {
        map.insert("feed_id".into(), json!(feed_id));
    }
    if let Some(since) = scope.since {
        map.insert("since".into(), json!(since));
    }
    if let Some(until) = scope.until {
        map.insert("until".into(), json!(until));
    }
    Value::Object(map)
}

/// 状态列写入的分派（三个既有 store 方法口径一致，只是列不同）
fn set_flag_ids(
    store: &Store,
    flag: EntryFlag,
    ids: &[i64],
    value: bool,
) -> rustrss_core::store::Result<usize> {
    match flag {
        EntryFlag::Read => store.set_read(ids, value),
        EntryFlag::Starred => store.set_starred(ids, value),
        EntryFlag::ReadLater => store.set_read_later(ids, value),
    }
}

// ---------------------------------------------------------------- 刷新

/// 已校验的刷新目标（`feed_ids` 一定都是真实存在的源）
#[derive(Debug, Clone, PartialEq, Eq)]
struct RefreshTarget {
    label: String,
    feed_ids: Vec<i64>,
}

impl RustRssMcp {
    /// 解析 + 校验 `refresh` 的范围参数（不做任何抓取，便于单测）
    fn refresh_target(&self, p: &RefreshParams) -> Result<RefreshTarget, ArgError> {
        let scope = match p.scope.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => s.to_ascii_lowercase(),
            None => match (p.feed_ids.is_some(), p.folder_id.is_some()) {
                (true, true) => {
                    return Err(ArgError::new(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        "feed_ids 与 folder_id 只能二选一",
                    ))
                }
                (true, false) => "feed_ids".to_string(),
                (false, true) => "folder".to_string(),
                (false, false) => "all".to_string(),
            },
        };

        match scope.as_str() {
            "all" => {
                if p.feed_ids.is_some() || p.folder_id.is_some() {
                    return Err(ArgError::new(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        "scope=all 不能同时给 feed_ids / folder_id（矛盾参数会让「刷了什么」无法解释）",
                    ));
                }
                let ids = self
                    .with_store(|s| s.all_feed_ids())
                    .map_err(|e| internal_arg_error(&e.to_string()))?;
                Ok(RefreshTarget {
                    label: "all".to_string(),
                    feed_ids: ids,
                })
            }
            "feed_ids" => {
                if p.folder_id.is_some() {
                    return Err(ArgError::new(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        "feed_ids 与 folder_id 只能二选一",
                    ));
                }
                let ids = p.feed_ids.as_deref().unwrap_or_default();
                if ids.is_empty() {
                    return Err(ArgError::new(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        "scope=feed_ids 需要非空的 feed_ids[]（空列表不是「全部」）",
                    ));
                }
                if ids.len() > MAX_BATCH_IDS {
                    return Err(ArgError::new(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        format!(
                            "feed_ids 一次最多 {MAX_BATCH_IDS} 个，收到 {} 个：请分批调用",
                            ids.len()
                        ),
                    ));
                }
                for id in ids {
                    let row = self
                        .with_store(|s| s.feed_row(*id))
                        .map_err(|e| internal_arg_error(&e.to_string()))?;
                    if row.is_none() {
                        return Err(ArgError::new(
                            ERROR_FEED_NOT_FOUND,
                            format!("订阅源 #{id} 不存在；先用 list_feeds 取 id"),
                        ));
                    }
                }
                Ok(RefreshTarget {
                    label: "feed_ids".to_string(),
                    feed_ids: ids.to_vec(),
                })
            }
            "folder" => {
                if p.feed_ids.is_some() {
                    return Err(ArgError::new(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        "feed_ids 与 folder_id 只能二选一",
                    ));
                }
                let folder_id = p.folder_id.ok_or_else(|| {
                    ArgError::new(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        "scope=folder 需要 folder_id（id 来自 list_folders）",
                    )
                })?;
                let folders = self
                    .with_store(|s| s.list_folders())
                    .map_err(|e| internal_arg_error(&e.to_string()))?;
                if !folders.iter().any(|(id, _)| *id == folder_id) {
                    return Err(ArgError::new(
                        ERROR_FOLDER_NOT_FOUND,
                        format!("分组 #{folder_id} 不存在；先用 list_folders 取 id"),
                    ));
                }
                let ids = self
                    .with_store(|s| s.feed_ids_in_folder(folder_id))
                    .map_err(|e| internal_arg_error(&e.to_string()))?;
                Ok(RefreshTarget {
                    label: format!("folder:{folder_id}"),
                    feed_ids: ids,
                })
            }
            other => Err(ArgError::new(
                write_contract::ERROR_INVALID_ARGUMENT,
                format!("scope 只支持 all / feed_ids / folder，收到 {other:?}"),
            )),
        }
    }

    /// 刷新（与界面共用单 flight）
    pub async fn refresh_json(&self, p: &RefreshParams) -> String {
        let target = match self.refresh_target(p) {
            Ok(t) => t,
            Err(e) => return WriteOutcome::failed_with(e.code, e.message).to_json(),
        };

        // 单 flight：应用内托管时界面与 MCP 拿的是**同一个** `Arc<RefreshGate>`，
        // 所以这里抢不到就说明界面（或上一轮 agent 调用）正在刷——不排队、不叠加。
        let gate = Arc::clone(&self.refresh_gate);
        let _flight = match gate.try_begin() {
            Ok(flight) => flight,
            Err(msg) => {
                return WriteOutcome::failed_with(
                    ERROR_RATE_LIMITED,
                    format!("{msg}；本次未执行（稍后重试即可，不要并发重试）"),
                )
                .to_json()
            }
        };

        let fetcher = match self.fetcher() {
            Ok(f) => f,
            Err(e) => return WriteOutcome::failed_with(ERROR_INTERNAL, e).to_json(),
        };

        // ① 锁内取任务 → ② 锁外抓取 → ③ 锁内落库（锁绝不跨 await，与界面 refresh_core 同一纪律）
        let jobs = match self.with_store(|s| collect_jobs(s, &target.feed_ids)) {
            Ok(j) => j,
            Err(e) => return internal_error(&e.to_string()),
        };
        let fetched = if jobs.is_empty() {
            Vec::new()
        } else {
            fetch_jobs(fetcher, jobs, REFRESH_CONCURRENCY).await
        };
        let report = match self.with_store(|s| {
            let report = apply_results(s, fetched);
            // 大批量写入后收尾 WAL（同一连接；失败只告警，不影响已提交的数据）
            if let Err(e) = s.checkpoint_wal() {
                log::warn!(target: "mcp", "mcp-write refresh WAL checkpoint 失败（不影响数据）: {e}");
            }
            report
        }) {
            Ok(r) => r,
            Err(e) => return internal_error(&e.to_string()),
        };

        refresh_outcome(&target, &report).to_json()
    }
}

/// 刷新结果信封：`affected` = 本轮入库条目数（新增 + 更新），`results` 只列失败源
/// （成功源见 `detail`），摘要字段沿用 core 的 [`RefreshReport`] 口径。
fn refresh_outcome(target: &RefreshTarget, report: &RefreshReport) -> WriteOutcome {
    let results: Vec<ItemResult> = report
        .failures
        .iter()
        .map(|f| ItemResult::failed(f.feed_id, ERROR_FETCH_FAILED))
        .collect();
    let detail = json!({
        "scope": target.label,
        "feeds": target.feed_ids.len(),
        "fetched": report.fetched,
        "not_modified": report.not_modified,
        "inserted": report.inserted,
        "updated": report.updated,
        "unchanged": report.unchanged,
        "failure_count": report.failures.len(),
        "failures": report.failures,
    });
    WriteOutcome::done((report.inserted + report.updated) as i64, results, false)
        .with_detail(detail)
}

// ---------------------------------------------------------------- 全文抓取

impl RustRssMcp {
    /// 单条补全文：摘要型条目抓原文页 → 提取正文 → 写回库。
    ///
    /// 三段结构与界面 `fetch_fulltext_core` 一致：① 锁内读条目 + 幂等判定 →
    /// ② **锁外**抓取（网络永不持库锁）→ ③ 锁内提取写回。已抓过的条目零网络返回
    /// （`affected: 0` + `detail.already_fulltext`）。
    pub async fn fetch_fulltext_json(&self, p: &FetchFulltextParams) -> String {
        let entry = match self.with_store(|s| s.get_entry(p.id)) {
            Ok(e) => e,
            Err(e) => return internal_error(&e.to_string()),
        };
        let Some(entry) = entry else {
            return WriteOutcome::failed_with(
                ERROR_ARTICLE_NOT_FOUND,
                format!("条目 #{} 不存在；先用 list_articles 取 id", p.id),
            )
            .to_json();
        };
        let Some(url) = entry
            .url
            .clone()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
        else {
            return WriteOutcome::failed_with(
                ERROR_INVALID_URL,
                format!("条目 #{} 没有原文地址，无法获取全文", p.id),
            )
            .to_json();
        };

        if !entry.needs_fulltext {
            let chars = entry
                .content_text
                .as_deref()
                .unwrap_or_default()
                .chars()
                .count();
            return WriteOutcome::done(0, vec![ItemResult::ok(p.id)], false)
                .with_detail(json!({
                    "already_fulltext": true,
                    "chars": chars,
                    "hint": "库里已有正文（本次未发请求）；正文用 get_article(id) 取",
                }))
                .to_json();
        }

        let fetcher = match self.fetcher() {
            Ok(f) => f,
            Err(e) => return WriteOutcome::failed_with(ERROR_INTERNAL, e).to_json(),
        };
        // 体积闸门在 core 的流式抓取里（Content-Length 预检 + 下载中累计超限即断）
        let body = match fetcher.fetch_bytes_limited(&url, fulltext::MAX_BYTES).await {
            Ok(body) => body,
            Err(e) => {
                return WriteOutcome::failed_with(
                    fetch_error_code(&e),
                    format!("获取原文失败: {e}"),
                )
                .to_json()
            }
        };
        let extracted = match fulltext::extract_bytes(&body, &url) {
            Ok(x) => x,
            Err(e) => {
                return WriteOutcome::failed_with(fulltext_error_code(&e), e.to_string()).to_json()
            }
        };

        let chars = extracted.content_text.chars().count();
        if let Err(e) = self
            .with_store(|s| s.set_fulltext(p.id, &extracted.content_html, &extracted.content_text))
        {
            return internal_error(&e.to_string());
        }
        WriteOutcome::done(1, vec![ItemResult::ok(p.id)], false)
            .with_detail(json!({
                "chars": chars,
                "hint": "正文已写回库；内容用 get_article(id) 单取（写工具不倒正文，避免一次响应撑爆上下文）",
            }))
            .to_json()
    }
}

/// 抓取阶段错误 → 机器码。体积超限与网络/HTTP 错分开：agent 的处置不同
/// （超限 = 别重试；网络错 = 可重试）。
///
/// core 的抓取错误是 `String`（`fetch.rs` 不在本任务改动范围），所以这里按闸门文案
/// 认体积超限。这层耦合由端到端测试钉住：
/// `tests/write_tools.rs` 用 3MiB 页面断言码为 `fulltext_too_large`——core 改了措辞就转红。
fn fetch_error_code(message: &str) -> &'static str {
    if message.contains("超过上限") || message.contains("字节上限") {
        ERROR_FULLTEXT_TOO_LARGE
    } else {
        ERROR_FETCH_FAILED
    }
}

/// 提取阶段错误 → 机器码（`FulltextError` 的每个变体都有可处置的答案）
fn fulltext_error_code(err: &FulltextError) -> &'static str {
    match err {
        FulltextError::TooLarge { .. } => ERROR_FULLTEXT_TOO_LARGE,
        FulltextError::BotChallenge => ERROR_FULLTEXT_BOT_CHALLENGE,
        FulltextError::NotHtml => ERROR_FULLTEXT_NOT_HTML,
        FulltextError::NoContent => ERROR_FULLTEXT_NO_CONTENT,
        FulltextError::BadUrl => ERROR_INVALID_URL,
        FulltextError::Extract(_) => ERROR_FULLTEXT_EXTRACT_FAILED,
    }
}

fn internal_error(message: &str) -> String {
    WriteOutcome::failed_with(ERROR_INTERNAL, format!("库操作失败: {message}")).to_json()
}

fn internal_arg_error(message: &str) -> ArgError {
    ArgError::new(ERROR_INTERNAL, format!("读库失败: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustrss_core::{Entry, IdOrigin};
    use serde_json::Value;

    fn entry(stable_id: &str, url: Option<&str>) -> Entry {
        Entry {
            stable_id: stable_id.to_string(),
            id_origin: IdOrigin::SourceData,
            source_id: stable_id.to_string(),
            title: format!("条目 {stable_id}"),
            url: url.map(str::to_string),
            author: None,
            published: None,
            updated: None,
            summary: Some("摘要".to_string()),
            content_html: None,
            content_text: Some("正文".to_string()),
            categories: Vec::new(),
        }
    }

    /// 建一个带 1 源 `count` 条条目的 server（内存库，无网络）
    fn seeded(count: usize) -> (RustRssMcp, Vec<i64>) {
        let server = RustRssMcp::new(Store::open_in_memory().expect("内存库应能打开"));
        let entries: Vec<Entry> = (1..=count)
            .map(|i| entry(&format!("a{i}"), Some(&format!("https://example.com/a{i}"))))
            .collect();
        server.with_store(|s| {
            let feed_id = s
                .add_feed("https://example.com/feed.xml", Some("示例源"))
                .unwrap();
            s.upsert_entries(feed_id, &entries).unwrap();
        });
        let ids = server
            .with_store(|s| s.list_entries(&rustrss_core::EntryQuery::default()))
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        (server, ids)
    }

    fn parse(json: &str) -> Value {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("不是合法 JSON（{e}）：{json}"))
    }

    fn unread(server: &RustRssMcp) -> i64 {
        server.with_store(|s| s.unread_total()).unwrap()
    }

    /// ids 形态：命中/缺失逐项给结果，`affected` 与未读数的变化一致
    #[test]
    fn set_read_by_ids_reports_per_item_and_moves_the_counters() {
        let (server, ids) = seeded(3);
        assert_eq!(unread(&server), 3);

        let out = parse(&server.set_read_json(&SetReadParams {
            ids: Some(vec![ids[0], ids[1], 999_999]),
            ..Default::default()
        }));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["affected"], 2, "只有存在的两条被写: {out}");
        assert_eq!(out["results"][0], json!({"id": ids[0], "ok": true}));
        assert_eq!(
            out["results"][2],
            json!({"id": 999_999, "ok": false, "error_code": ERROR_ARTICLE_NOT_FOUND}),
            "{out}"
        );
        assert_eq!(
            unread(&server),
            1,
            "未读数必须跟着降（与界面同一 store 路径）"
        );

        // 幂等：重复调用返回值稳定（口径是「命中」而不是「值真的变了」）
        let again = parse(&server.set_read_json(&SetReadParams {
            ids: Some(vec![ids[0], ids[1]]),
            ..Default::default()
        }));
        assert_eq!(again["affected"], 2, "{again}");
        assert_eq!(again["ok"], true);

        // 反向：read=false 撤销
        let back = parse(&server.set_read_json(&SetReadParams {
            ids: Some(vec![ids[0], ids[1]]),
            read: Some(false),
            ..Default::default()
        }));
        assert_eq!(back["affected"], 2);
        assert_eq!(unread(&server), 3, "撤销后回到未读");
    }

    /// 一条都不存在：整单失败（`ok=false` + `article_not_found`），逐项原因仍在
    #[test]
    fn set_read_with_no_existing_id_fails_the_whole_call() {
        let (server, _ids) = seeded(1);
        let out = parse(&server.set_read_json(&SetReadParams {
            ids: Some(vec![404, 405]),
            ..Default::default()
        }));
        assert_eq!(out["ok"], false, "{out}");
        assert_eq!(out["error_code"], ERROR_ARTICLE_NOT_FOUND, "{out}");
        assert_eq!(out["affected"], 0);
        assert_eq!(
            out["results"].as_array().unwrap().len(),
            2,
            "逐项原因保留: {out}"
        );
        assert_eq!(unread(&server), 1, "没命中就不该改库");
    }

    /// 入参形态校验：空 ids / 超限 / 混用 / 都不给，四类全判 `invalid_argument`
    #[test]
    fn set_flag_target_shape_is_validated() {
        let (server, ids) = seeded(2);
        let too_many: Vec<i64> = (1..=MAX_BATCH_IDS as i64 + 1).collect();
        let cases: Vec<(SetReadParams, &str)> = vec![
            (
                SetReadParams {
                    ids: Some(Vec::new()),
                    ..Default::default()
                },
                "空 ids",
            ),
            (
                SetReadParams {
                    ids: Some(too_many),
                    ..Default::default()
                },
                "超限",
            ),
            (
                SetReadParams {
                    ids: Some(ids.clone()),
                    feed_id: Some(1),
                    ..Default::default()
                },
                "混用 ids 与条件",
            ),
            (SetReadParams::default(), "什么都没给"),
        ];
        for (params, label) in cases {
            let out = parse(&server.set_read_json(&params));
            assert_eq!(out["ok"], false, "{label}: {out}");
            assert_eq!(
                out["error_code"],
                write_contract::ERROR_INVALID_ARGUMENT,
                "{label}: {out}"
            );
            assert_eq!(out["affected"], 0, "{label}: {out}");
        }
        assert_eq!(unread(&server), 2, "非法调用一个字节都不该改");
    }

    /// 条件级形态：影响面 = 命中条数，逐项结果是命中集的采样（超过 100 条时标截断）
    #[test]
    fn set_read_by_condition_samples_the_hit_set_and_flags_truncation() {
        let (server, _ids) = seeded(150);

        let out = parse(&server.set_read_json(&SetReadParams {
            feed_id: Some(1),
            read: Some(true),
            ..Default::default()
        }));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["affected"], 150, "命中 150 条: {out}");
        assert_eq!(out["detail"]["target"]["feed_id"], 1, "{out}");
        assert_eq!(out["detail"]["results_truncated"], true, "{out}");
        assert_eq!(
            out["results"].as_array().unwrap().len(),
            MAX_BATCH_IDS,
            "{out}"
        );
        assert_eq!(unread(&server), 0, "条件级写入必须真的落库");

        // 小命中集：不标截断
        let (small, _) = seeded(3);
        let out = parse(&small.set_read_json(&SetReadParams {
            feed_id: Some(1),
            ..Default::default()
        }));
        assert_eq!(out["affected"], 3, "{out}");
        assert_eq!(out["results"].as_array().unwrap().len(), 3, "{out}");
        assert_eq!(out["detail"]["results_truncated"], Value::Null, "{out}");
    }

    /// 条件级没命中任何行：`ok=true` + `affected=0`（不是错误——条件是合法的，只是没内容）
    #[test]
    fn set_read_by_condition_with_no_hit_is_a_noop() {
        let (server, _ids) = seeded(2);
        let out = parse(&server.set_read_json(&SetReadParams {
            feed_id: Some(1),
            since: Some(i64::MAX - 1),
            ..Default::default()
        }));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["affected"], 0, "{out}");
        assert_eq!(out["results"].as_array().unwrap().len(), 0, "{out}");
        assert_eq!(unread(&server), 2);
    }

    /// 三个状态工具互不串味：各自的取值字段与目标列一一对应
    #[test]
    fn starred_and_read_later_have_their_own_columns() {
        let (server, ids) = seeded(2);
        assert_eq!(
            parse(&server.set_starred_json(&SetStarredParams {
                ids: Some(ids.clone()),
                ..Default::default()
            }))["affected"],
            2
        );
        assert_eq!(
            parse(&server.set_read_later_json(&SetReadLaterParams {
                ids: Some(ids.clone()),
                ..Default::default()
            }))["affected"],
            2
        );
        let row = server.with_store(|s| s.get_entry(ids[0])).unwrap().unwrap();
        assert!(row.starred && row.read_later && !row.read, "{row:?}");

        // 取消（显式 false）各管各的列
        parse(&server.set_starred_json(&SetStarredParams {
            ids: Some(ids.clone()),
            starred: Some(false),
            ..Default::default()
        }));
        let row = server.with_store(|s| s.get_entry(ids[0])).unwrap().unwrap();
        assert!(
            !row.starred && row.read_later && !row.read,
            "取消星标不该动稍后读: {row:?}"
        );
    }

    /// 刷新范围参数校验：三类 scope + 四种非法形态（都不发网络请求）
    #[test]
    fn refresh_target_validation_covers_all_three_scopes() {
        let (server, _ids) = seeded(1);

        let ok = server
            .refresh_target(&RefreshParams {
                scope: Some("all".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(ok.feed_ids.len(), 1);
        assert_eq!(ok.label, "all");

        let ok = server
            .refresh_target(&RefreshParams {
                scope: Some("feed_ids".into()),
                feed_ids: Some(vec![1]),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(ok.feed_ids, vec![1]);

        let missing_folder = server
            .refresh_target(&RefreshParams {
                scope: Some("folder".into()),
                folder_id: Some(1),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(
            missing_folder.code, ERROR_FOLDER_NOT_FOUND,
            "不存在的分组要能被认出来"
        );

        server.with_store(|s| s.add_folder("技术").unwrap());
        let folder_id = server.with_store(|s| s.list_folders()).unwrap()[0].0;
        server.with_store(|s| s.assign_folder(1, Some(folder_id)).unwrap());
        let ok = server
            .refresh_target(&RefreshParams {
                scope: Some("folder".into()),
                folder_id: Some(folder_id),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(ok.feed_ids, vec![1], "组内源都要刷");
        assert_eq!(ok.label, format!("folder:{folder_id}"));

        // 缺省 scope 按参数推断
        let inferred = server
            .refresh_target(&RefreshParams {
                feed_ids: Some(vec![1]),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(inferred.label, "feed_ids");

        // 非法形态：未知 scope / 矛盾参数 / 空 feed_ids / 不存在的源与分组
        let bad = |p: RefreshParams| server.refresh_target(&p).unwrap_err();
        assert_eq!(
            bad(RefreshParams {
                scope: Some("everything".into()),
                ..Default::default()
            })
            .code,
            write_contract::ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            bad(RefreshParams {
                scope: Some("all".into()),
                feed_ids: Some(vec![1]),
                ..Default::default()
            })
            .code,
            write_contract::ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            bad(RefreshParams {
                scope: Some("feed_ids".into()),
                feed_ids: Some(Vec::new()),
                ..Default::default()
            })
            .code,
            write_contract::ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            bad(RefreshParams {
                folder_id: Some(1),
                feed_ids: Some(vec![1]),
                ..Default::default()
            })
            .code,
            write_contract::ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            bad(RefreshParams {
                scope: Some("feed_ids".into()),
                feed_ids: Some(vec![404]),
                ..Default::default()
            })
            .code,
            ERROR_FEED_NOT_FOUND
        );
        assert_eq!(
            bad(RefreshParams {
                scope: Some("folder".into()),
                folder_id: Some(404),
                ..Default::default()
            })
            .code,
            ERROR_FOLDER_NOT_FOUND
        );
    }

    /// 单 flight：界面（或上一轮）持着 gate 时，MCP 的刷新必须被拒且不碰库
    #[tokio::test]
    async fn refresh_reports_rate_limited_when_the_shared_gate_is_held() {
        let (server, _ids) = seeded(1);
        // 绑定成一个变量：守卫借用的是这个 Arc，直接用临时值会在语句结束就被释放
        let gate = server.refresh_gate();
        let held = gate.try_begin().expect("先占住单 flight");

        let out = parse(
            &server
                .refresh_json(&RefreshParams {
                    scope: Some("all".into()),
                    ..Default::default()
                })
                .await,
        );
        assert_eq!(out["ok"], false, "{out}");
        assert_eq!(out["error_code"], ERROR_RATE_LIMITED, "{out}");
        assert_eq!(out["affected"], 0, "{out}");
        assert!(
            out["error"].as_str().unwrap_or_default().contains("进行中"),
            "要说清为什么: {out}"
        );

        // 参数校验在抢 gate 之前：非法 scope 仍然是 invalid_argument（不是 rate_limited）
        let out = parse(
            &server
                .refresh_json(&RefreshParams {
                    scope: Some("nope".into()),
                    ..Default::default()
                })
                .await,
        );
        assert_eq!(
            out["error_code"],
            write_contract::ERROR_INVALID_ARGUMENT,
            "{out}"
        );

        drop(held);
        // 放行后能再次开始（这里只验参数层与 gate 状态，真抓取在 e2e 里）
        let out = parse(
            &server
                .refresh_json(&RefreshParams {
                    scope: Some("feed_ids".into()),
                    feed_ids: Some(vec![404]),
                    ..Default::default()
                })
                .await,
        );
        assert_eq!(out["error_code"], ERROR_FEED_NOT_FOUND, "{out}");
        assert!(
            !server.refresh_gate().is_active(),
            "守卫必须已释放（不能把标记卡死）"
        );
    }

    /// 全文抓取的错误路径：条目不存在 / 没有原文地址（都不发网络请求）
    #[tokio::test]
    async fn fetch_fulltext_rejects_missing_entries_and_missing_urls() {
        let (server, ids) = seeded(1);

        let out = parse(
            &server
                .fetch_fulltext_json(&FetchFulltextParams { id: 404 })
                .await,
        );
        assert_eq!(out["ok"], false, "{out}");
        assert_eq!(out["error_code"], ERROR_ARTICLE_NOT_FOUND, "{out}");

        // 另建一条没有原文地址的条目：无法抓取，但要说清楚
        let no_url_id = {
            server.with_store(|s| {
                let feed_id = s.all_feed_ids().unwrap()[0];
                s.upsert_entries(feed_id, &[entry("no-url", None)]).unwrap();
                s.list_entries(&rustrss_core::EntryQuery::default())
                    .unwrap()
                    .into_iter()
                    .find(|r| r.stable_id == "no-url")
                    .unwrap()
                    .id
            })
        };
        assert_ne!(no_url_id, ids[0]);
        let out = parse(
            &server
                .fetch_fulltext_json(&FetchFulltextParams { id: no_url_id })
                .await,
        );
        assert_eq!(out["ok"], false, "{out}");
        assert_eq!(out["error_code"], ERROR_INVALID_URL, "{out}");
        assert!(
            out["error"]
                .as_str()
                .unwrap_or_default()
                .contains("原文地址"),
            "要说清缺什么: {out}"
        );
    }

    /// 错误码分类：体积闸门 vs 网络错、提取变体逐一有码（纯函数，不靠文案猜业务语义）
    #[test]
    fn error_codes_are_classified_explicitly() {
        assert_eq!(
            fetch_error_code("页面体积 3145728 超过上限 2097152，已中止下载"),
            ERROR_FULLTEXT_TOO_LARGE
        );
        assert_eq!(
            fetch_error_code("页面超过 2097152 字节上限（实际 3145728 字节），已拒绝抓取"),
            ERROR_FULLTEXT_TOO_LARGE
        );
        assert_eq!(fetch_error_code("连接被拒绝"), ERROR_FETCH_FAILED);
        assert_eq!(fetch_error_code("HTTP 404"), ERROR_FETCH_FAILED);

        assert_eq!(
            fulltext_error_code(&FulltextError::BotChallenge),
            ERROR_FULLTEXT_BOT_CHALLENGE
        );
        assert_eq!(
            fulltext_error_code(&FulltextError::NotHtml),
            ERROR_FULLTEXT_NOT_HTML
        );
        assert_eq!(
            fulltext_error_code(&FulltextError::NoContent),
            ERROR_FULLTEXT_NO_CONTENT
        );
        assert_eq!(
            fulltext_error_code(&FulltextError::BadUrl),
            ERROR_INVALID_URL
        );
        assert_eq!(
            fulltext_error_code(&FulltextError::TooLarge { size: 10, max: 5 }),
            ERROR_FULLTEXT_TOO_LARGE
        );
        assert_eq!(
            fulltext_error_code(&FulltextError::Extract("boom".into())),
            ERROR_FULLTEXT_EXTRACT_FAILED
        );
    }
}
