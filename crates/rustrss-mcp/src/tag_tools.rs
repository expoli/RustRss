//! T4 的标签工具：`list_tags`（read）+ `create_tag` / `rename_tag` / `assign_tags` /
//! `unassign_tags` / `delete_tag`（write）。
//!
//! 复用 `2026-09-23-mcp-write` 批次落地的基建（`registry` / `write_contract` / `audit`），
//! 不新增机制：
//! - **权限**：注册表登记 scope（`list_tags` = read，其余 = write），全部 `dangerous = false`
//!   ——删标签只清关联、不删文章，不进危险集合；真正执行要 `confirm: true`；
//! - **目标形态**：`assign_tags` / `unassign_tags` 与 `set_read` 同形——`ids[]`（≤100）或
//!   条件级 `{feed_id, since, until}` 二选一；`affected` = 命中条目数（重复调用稳定 =
//!   幂等），`detail.changed` = 本次真正新增/移除的关联行数（重复调用为 0）；
//! - **dry_run 同源**：`delete_tag(dry_run=true)` 走 core 的 `Store::delete_tag(id, true)`，
//!   与真删共用 `Store::tag_entry_count`，所以「预览 N 篇」与「真删影响 N 篇」不可能漂移；
//! - **错误码**：`tag_not_found` / `duplicate_tag_name` / `invalid_argument` 从
//!   `StoreError` 的**结构化变体**映射，不解析错误文案；
//! - **响应体积**：`list_tags` 只回标签元数据（id/名称/颜色/置顶/未读/顺序/最近使用）
//!   且带上限与显式截断标记；条目上的 `tags` 字段只回名称（见 `lib.rs` 的 `ArticleMetaOut`）。

use rustrss_core::{EntryScope, StoreError, TagAssignReport, TagRow, TagTarget};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::write_contract::{self, ItemResult, WriteOutcome, MAX_BATCH_IDS};
use crate::write_tools::{internal_error, scope_json, ERROR_ARTICLE_NOT_FOUND, ERROR_INTERNAL};
use crate::RustRssMcp;

// ---------------------------------------------------------------- 错误码（机器可读，README 同步）

pub const ERROR_TAG_NOT_FOUND: &str = "tag_not_found";
pub const ERROR_DUPLICATE_TAG_NAME: &str = "duplicate_tag_name";
pub const ERROR_INVALID_ARGUMENT: &str = write_contract::ERROR_INVALID_ARGUMENT;

/// `list_tags` 一次最多回多少个标签（超出时 `truncated=true` + `total` 给全量口径，
/// 不静默丢也不无上限地倒给 agent）。
pub const TAG_LIST_MAX: usize = 200;

/// 单篇条目最多回多少个标签名（超出置 `tags_truncated=true`；一个条目的标签数
/// 通常个位数，这个上限只为挡住「一个条目挂几百个标签」的极端情况）。
pub const TAGS_PER_ENTRY_MAX: usize = 20;

// ---------------------------------------------------------------- 入参

/// `list_tags` 的参数
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ListTagsParams {
    /// 排序档：`sidebar`（默认：置顶优先 → 手动顺序 → 名称）或
    /// `recent`（最近使用优先 `last_used_at DESC`，没用过的垫底）
    pub sort: Option<String>,
}

/// `create_tag` 的参数
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTagParams {
    /// 标签名（trim 后不能为空；大小写不敏感唯一，重名报 `duplicate_tag_name`）
    pub name: String,
    /// 颜色 `#RRGGBB`（可选；不给 = 默认色）
    pub color: Option<String>,
}

/// `rename_tag` 的参数
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameTagParams {
    /// 标签 id（来自 list_tags）
    pub tag_id: i64,
    /// 新名称（口径同 `create_tag`；只改大小写允许）
    pub name: String,
}

/// `assign_tags` / `unassign_tags` 的参数：目标（`ids[]` 或条件级二选一）+ 标签 id
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AssignTagsParams {
    /// 目标条目 id（来自 list_articles / search_articles），最多 100 个
    pub ids: Option<Vec<i64>>,
    /// 条件级：只动这个订阅源（id 来自 list_feeds）
    pub feed_id: Option<i64>,
    /// 条件级：只动该时刻（含）之后的条目；比较键与列表排序键同源
    /// `COALESCE(published_at, fetched_at)`（Unix 秒）
    pub since: Option<i64>,
    /// 条件级：只动该时刻（含）之前的条目；闭区间，口径同 since
    pub until: Option<i64>,
    /// 要附加/移除的标签 id（来自 list_tags），1..=100 个
    pub tag_ids: Vec<i64>,
}

/// `delete_tag` 的参数
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DeleteTagParams {
    /// 标签 id（来自 list_tags）
    pub tag_id: i64,
    /// 确认：实际执行必须显式 `true`（缺省/`false` → `confirm_required`）；
    /// `dry_run: true` 的预览不需要它
    pub confirm: Option<bool>,
    /// `true` = 只返回受影响篇数、不落库（预览与实际执行共用 core 同一个计数函数）
    pub dry_run: Option<bool>,
}

// ---------------------------------------------------------------- 工具实现

impl RustRssMcp {
    /// 标签清单：元数据 + 未读计数；带上限与显式截断标记。
    pub fn list_tags_json(&self, p: &ListTagsParams) -> String {
        let sort = match p.sort.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("sidebar") => "sidebar",
            Some("recent") => "recent",
            Some(other) => {
                return error_body(
                    ERROR_INVALID_ARGUMENT,
                    &format!("sort 只支持 sidebar / recent，收到 {other:?}"),
                )
            }
        };
        let listed = self.with_store(|store| {
            if sort == "recent" {
                store.list_tags_recent_first()
            } else {
                store.list_tags()
            }
        });
        match listed {
            Ok(rows) => {
                let total = rows.len();
                let tags: Vec<Value> = rows.iter().take(TAG_LIST_MAX).map(tag_json).collect();
                json!({
                    "count": tags.len(),
                    "total": total,
                    "truncated": total > TAG_LIST_MAX,
                    "sort": sort,
                    "tags": tags,
                    "hint": "标签行只有元数据（无条目正文）；写工具的 tag_id 取自这里",
                })
                .to_string()
            }
            Err(e) => error_body(ERROR_INTERNAL, &format!("读库失败: {e}")),
        }
    }

    pub fn create_tag_json(&self, p: &CreateTagParams) -> String {
        match self.with_store(|store| store.create_tag(&p.name, p.color.as_deref())) {
            Ok(row) => WriteOutcome::done(1, vec![ItemResult::ok(row.id)], false)
                .with_detail(tag_json(&row))
                .to_json(),
            Err(e) => store_error_body(e),
        }
    }

    pub fn rename_tag_json(&self, p: &RenameTagParams) -> String {
        match self.with_store(|store| store.rename_tag(p.tag_id, &p.name)) {
            Ok(row) => WriteOutcome::done(1, vec![ItemResult::ok(row.id)], false)
                .with_detail(tag_json(&row))
                .to_json(),
            Err(e) => store_error_body(e),
        }
    }

    pub fn assign_tags_json(&self, p: &AssignTagsParams) -> String {
        self.tag_link_json(p, true)
    }

    pub fn unassign_tags_json(&self, p: &AssignTagsParams) -> String {
        self.tag_link_json(p, false)
    }

    /// 删标签：只清关联、**不删文章**；`dry_run` 与实际执行在 core 里共用同一个计数函数。
    pub fn delete_tag_json(&self, p: &DeleteTagParams) -> String {
        let dry_run = p.dry_run.unwrap_or(false);
        // 预览是只读的，不需要 confirm；真正删才要（缺省不是"默认同意"）
        if !dry_run {
            if let Err(e) = write_contract::require_confirm(p.confirm) {
                return WriteOutcome::rejected(e).to_json();
            }
        }
        match self.with_store(|s| s.delete_tag(p.tag_id, dry_run)) {
            Ok(report) => {
                let detail = json!({
                    "tag_id": p.tag_id,
                    "note": "删除只清标签关联，文章保留",
                });
                if dry_run {
                    WriteOutcome::preview(report.affected_entries, vec![ItemResult::ok(p.tag_id)])
                        .with_detail(detail)
                        .to_json()
                } else {
                    WriteOutcome::done(
                        report.affected_entries,
                        vec![ItemResult::ok(p.tag_id)],
                        false,
                    )
                    .with_detail(detail)
                    .to_json()
                }
            }
            Err(e) => store_error_body(e),
        }
    }

    /// `assign_tags` / `unassign_tags` 的共同执行体：目标二选一 → 标签存在性 → 落库。
    fn tag_link_json(&self, p: &AssignTagsParams, link: bool) -> String {
        let has_condition = p.feed_id.is_some() || p.since.is_some() || p.until.is_some();
        match (p.ids.is_some(), has_condition) {
            // 混用两种形态时「影响面」无法解释（并集还是交集？），直接判非法
            (true, true) => {
                return WriteOutcome::failed_with(
                    ERROR_INVALID_ARGUMENT,
                    "ids 与条件级参数（feed_id / since / until）只能二选一——混用会让影响面无法解释",
                )
                .to_json()
            }
            (false, false) => {
                return WriteOutcome::failed_with(
                    ERROR_INVALID_ARGUMENT,
                    "缺少目标：请给 ids[]（≤100）或条件级参数（feed_id / since / until 至少一个）",
                )
                .to_json()
            }
            _ => {}
        }
        if let Err(message) = check_tag_ids(&p.tag_ids) {
            return WriteOutcome::failed_with(ERROR_INVALID_ARGUMENT, message).to_json();
        }
        // 标签必须都存在：静默跳过会让 agent 以为打上了。整单拒绝（本次零改动），
        // 避免"一半标签生效一半没生效"这种没法对账的中间态。
        let missing = match self.with_store(|store| -> rustrss_core::store::Result<Option<i64>> {
            for id in &p.tag_ids {
                if store.tag_row(*id)?.is_none() {
                    return Ok(Some(*id));
                }
            }
            Ok(None)
        }) {
            Ok(v) => v,
            Err(e) => return internal_error(&e.to_string()),
        };
        if let Some(id) = missing {
            return WriteOutcome::failed_with(
                ERROR_TAG_NOT_FOUND,
                format!("标签 #{id} 不存在；先用 list_tags 取 id（本次未做任何改动）"),
            )
            .to_json();
        }
        match p.ids.as_deref() {
            Some(ids) => self.tag_link_by_ids(p, ids, link),
            None => self.tag_link_by_scope(p, link),
        }
    }

    /// ids 形态：逐项核对存在性（缺失 → `article_not_found`），只对存在的行落库
    fn tag_link_by_ids(&self, p: &AssignTagsParams, ids: &[i64], link: bool) -> String {
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
            return WriteOutcome {
                results,
                ..WriteOutcome::failed_with(
                    ERROR_ARTICLE_NOT_FOUND,
                    "给定的 id 一个都不存在；先用 list_articles / search_articles 取真实 id",
                )
            }
            .to_json();
        }
        let report =
            self.with_store(|s| self.apply_tag_link(s, p, TagTarget::Entries(existing), link));
        match report {
            Ok(report) => WriteOutcome::done(report.entries, results, false)
                .with_detail(link_detail(&report))
                .to_json(),
            Err(e) => store_error_body(e),
        }
    }

    /// 条件级形态：命中集有界采样当逐项结果（同一个条件 WHERE，采样即会被写入的行）
    fn tag_link_by_scope(&self, p: &AssignTagsParams, link: bool) -> String {
        let scope = EntryScope {
            feed_id: p.feed_id,
            since: p.since,
            until: p.until,
        };
        let sample = match self.with_store(|s| s.entry_ids_scoped(&scope, MAX_BATCH_IDS + 1)) {
            Ok(v) => v,
            // 空条件（三个字段都缺）由 core 判 Invalid → 统一 invalid_argument：
            // 条件级漏参绝不能退化成「全库打标」
            Err(StoreError::Invalid(message)) => {
                return WriteOutcome::failed_with(ERROR_INVALID_ARGUMENT, message).to_json()
            }
            Err(e) => return internal_error(&e.to_string()),
        };
        let truncated = sample.len() > MAX_BATCH_IDS;
        let results: Vec<ItemResult> = sample
            .iter()
            .take(MAX_BATCH_IDS)
            .map(|id| ItemResult::ok(*id))
            .collect();
        let report =
            match self.with_store(|s| self.apply_tag_link(s, p, TagTarget::Scope(scope), link)) {
                Ok(r) => r,
                Err(e) => return store_error_body(e),
            };
        let mut detail = link_detail(&report);
        detail["target"] = scope_json(&scope);
        if truncated {
            detail["results_truncated"] = json!(true);
            detail["results_hint"] =
                json!("命中集超过 100 条：results 只列前 100 个 id（affected 是完整命中数）");
        }
        WriteOutcome::done(report.entries, results, false)
            .with_detail(detail)
            .to_json()
    }

    fn apply_tag_link(
        &self,
        store: &rustrss_core::Store,
        p: &AssignTagsParams,
        target: TagTarget,
        link: bool,
    ) -> rustrss_core::store::Result<TagAssignReport> {
        if link {
            store.assign_tags(&target, &p.tag_ids)
        } else {
            store.unassign_tags(&target, &p.tag_ids)
        }
    }
}

// ---------------------------------------------------------------- 纯函数帮手

/// `list_articles` 标签过滤参数的解析错误（机器可读码 + 人读说明）
pub(crate) struct TagFilterError {
    pub code: &'static str,
    pub message: String,
}

/// 解析 `list_articles` 的标签过滤：`tag_id` / `tag_name` 互斥由调用方先判，
/// 这里把 `tag_name` 解析成 id（大小写不敏感，与 `tags.name` 的 `UNIQUE COLLATE NOCASE`
/// 同口径）。未知标签 / 空名称 → 机器可读错误——**不静默返回空列表**，否则 agent 会把
/// 「没有这个标签」读成「这个标签下没有文章」。
pub(crate) fn resolve_tag_filter(
    store: &rustrss_core::Store,
    tag_id: Option<i64>,
    tag_name: Option<&str>,
) -> std::result::Result<Option<i64>, TagFilterError> {
    if let Some(id) = tag_id {
        return match store.tag_row(id) {
            Ok(Some(_)) => Ok(Some(id)),
            Ok(None) => Err(TagFilterError {
                code: ERROR_TAG_NOT_FOUND,
                message: format!("标签 #{id} 不存在；先用 list_tags 取 id"),
            }),
            Err(e) => Err(TagFilterError {
                code: ERROR_INTERNAL,
                message: format!("读库失败: {e}"),
            }),
        };
    }
    let Some(raw) = tag_name else {
        return Ok(None);
    };
    let needle = raw.trim();
    if needle.is_empty() {
        return Err(TagFilterError {
            code: ERROR_INVALID_ARGUMENT,
            message: "tag_name 不能为空（要给名称就给个有值的）".to_string(),
        });
    }
    match store.list_tags() {
        Ok(rows) => match rows.iter().find(|t| t.name.eq_ignore_ascii_case(needle)) {
            Some(row) => Ok(Some(row.id)),
            None => Err(TagFilterError {
                code: ERROR_TAG_NOT_FOUND,
                message: format!("标签 {needle:?} 不存在；先用 list_tags 取名称"),
            }),
        },
        Err(e) => Err(TagFilterError {
            code: ERROR_INTERNAL,
            message: format!("读库失败: {e}"),
        }),
    }
}

/// 标签 id 批量闸门：空 / 超 100 都是 `invalid_argument`（不静默截断）
fn check_tag_ids(tag_ids: &[i64]) -> std::result::Result<(), String> {
    if tag_ids.is_empty() {
        return Err("tag_ids 不能为空：请先用 list_tags 取标签 id".to_string());
    }
    if tag_ids.len() > MAX_BATCH_IDS {
        return Err(format!(
            "一次最多处理 {MAX_BATCH_IDS} 个标签，收到 {} 个：请分批调用",
            tag_ids.len()
        ));
    }
    Ok(())
}

/// 标签行的响应形状（元数据；不含任何条目正文）
fn tag_json(row: &TagRow) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "color": row.color,
        "pinned": row.pinned,
        "unread": row.unread,
        "sort_order": row.sort_order,
        "last_used_at": row.last_used_at,
    })
}

/// 标签写操作的 `detail`：`changed` 是真正改动的关联行数（重复调用 0），
/// `matched_entries` 与信封 `affected` 同源（命中条目数，重复调用稳定）。
fn link_detail(report: &TagAssignReport) -> Value {
    json!({
        "changed": report.changed,
        "matched_entries": report.entries,
        "tag_ids": report.tag_ids,
    })
}

/// `StoreError` → 机器可读错误码（**按变体**映射，不解析文案）
fn store_error_body(err: StoreError) -> String {
    match err {
        StoreError::DuplicateTagName(name) => WriteOutcome::failed_with(
            ERROR_DUPLICATE_TAG_NAME,
            format!("标签名已存在: {name}（大小写不敏感）；先用 list_tags 取既有 id"),
        )
        .to_json(),
        StoreError::TagNotFound(id) => WriteOutcome::failed_with(
            ERROR_TAG_NOT_FOUND,
            format!("标签 #{id} 不存在；先用 list_tags 取 id"),
        )
        .to_json(),
        StoreError::Invalid(message) => {
            WriteOutcome::failed_with(ERROR_INVALID_ARGUMENT, message).to_json()
        }
        other => internal_error(&other.to_string()),
    }
}

/// 读工具的机器可读错误（与写信封同形：`error_code` + `error`，agent 不必记第二套解包规则）
pub(crate) fn error_body(code: &str, message: &str) -> String {
    json!({ "error_code": code, "error": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RustRssMcp;
    use rustrss_core::{Entry, IdOrigin, Store};

    fn server() -> RustRssMcp {
        RustRssMcp::new(Store::open_in_memory().expect("内存库应能打开"))
    }

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap_or_else(|e| panic!("响应应为 JSON（{e}）: {text}"))
    }

    /// 标签批量闸门：空 / 101 个都报 `invalid_argument`（同一个错误码，不靠文案）
    #[test]
    fn tag_id_batch_is_capped_at_100() {
        assert!(check_tag_ids(&[1]).is_ok());
        assert!(check_tag_ids(&(1..=MAX_BATCH_IDS as i64).collect::<Vec<_>>()).is_ok());

        let empty = check_tag_ids(&[]).unwrap_err();
        assert!(empty.contains("不能为空"), "{empty}");

        let too_many =
            check_tag_ids(&(1..=MAX_BATCH_IDS as i64 + 1).collect::<Vec<_>>()).unwrap_err();
        assert!(too_many.contains("100"), "{too_many}");
    }

    /// `list_tags`：元数据齐全（未读/置顶/颜色/顺序/最近使用），且默认侧栏顺序可用
    #[test]
    fn list_tags_reports_metadata_and_ordering() {
        let server = server();
        let created = parse(&server.create_tag_json(&CreateTagParams {
            name: "Rust".into(),
            color: Some("#3E63DD".into()),
        }));
        let rust_id = created["detail"]["id"].as_i64().unwrap();
        assert_eq!(created["ok"], true);
        assert_eq!(created["affected"], 1);
        // 颜色归一化（大写 → 小写）由 core 完成，MCP 只透传结果
        assert_eq!(created["detail"]["color"], "#3e63dd");
        assert_eq!(created["detail"]["pinned"], false);
        assert_eq!(created["detail"]["unread"], 0);
        assert_eq!(created["detail"]["last_used_at"], Value::Null);

        server.create_tag_json(&CreateTagParams {
            name: "数据库".into(),
            color: None,
        });

        let listed = parse(&server.list_tags_json(&ListTagsParams::default()));
        assert_eq!(listed["count"], 2);
        assert_eq!(listed["total"], 2);
        assert_eq!(listed["truncated"], false);
        assert_eq!(listed["sort"], "sidebar");
        assert_eq!(listed["tags"][0]["id"], rust_id, "侧栏顺序：先建的在前");
        for key in [
            "id",
            "name",
            "color",
            "pinned",
            "unread",
            "sort_order",
            "last_used_at",
        ] {
            assert!(
                listed["tags"][0].get(key).is_some(),
                "缺字段 {key}: {listed}"
            );
        }
    }

    /// 响应体积上限：超过 `TAG_LIST_MAX` 时截断并**显式**告知（total + truncated）
    #[test]
    fn list_tags_is_bounded_and_says_so() {
        let server = server();
        for i in 0..(TAG_LIST_MAX + 3) {
            server.create_tag_json(&CreateTagParams {
                name: format!("tag-{i:03}"),
                color: None,
            });
        }
        let listed = parse(&server.list_tags_json(&ListTagsParams::default()));
        assert_eq!(listed["count"], TAG_LIST_MAX);
        assert_eq!(listed["total"], TAG_LIST_MAX + 3);
        assert_eq!(listed["truncated"], true);
        assert_eq!(
            listed["tags"].as_array().unwrap().len(),
            TAG_LIST_MAX,
            "截断后不得超上限"
        );
    }

    /// 排序档解析：未知档报 `invalid_argument`（不静默回默认）
    #[test]
    fn list_tags_rejects_unknown_sort() {
        let server = server();
        let out = parse(&server.list_tags_json(&ListTagsParams {
            sort: Some("random".into()),
        }));
        assert_eq!(out["error_code"], ERROR_INVALID_ARGUMENT);
    }

    /// `StoreError` → 错误码按变体映射（三种业务错误各一格 + 其余走 internal_error）
    #[test]
    fn store_errors_map_to_machine_readable_codes() {
        let dup = parse(&store_error_body(StoreError::DuplicateTagName(
            "Rust".into(),
        )));
        assert_eq!(dup["error_code"], ERROR_DUPLICATE_TAG_NAME);
        assert_eq!(dup["affected"], 0);
        assert!(dup["error"].as_str().unwrap().contains("Rust"), "{dup}");

        let missing = parse(&store_error_body(StoreError::TagNotFound(42)));
        assert_eq!(missing["error_code"], ERROR_TAG_NOT_FOUND);
        assert!(
            missing["error"].as_str().unwrap().contains("42"),
            "{missing}"
        );

        let invalid = parse(&store_error_body(StoreError::Invalid(
            "颜色格式应为 #RRGGBB".into(),
        )));
        assert_eq!(invalid["error_code"], ERROR_INVALID_ARGUMENT);

        let internal = parse(&store_error_body(StoreError::Io("磁盘炸了".into())));
        assert_eq!(internal["error_code"], ERROR_INTERNAL);
    }

    /// 业务错误码：重名建标签 / 空名 / 非法颜色 / 改名不存在，全走 `error_code` 而不靠文案
    #[test]
    fn tag_write_errors_use_codes_not_messages() {
        let server = server();
        server.create_tag_json(&CreateTagParams {
            name: "Rust".into(),
            color: None,
        });

        let dup = parse(&server.create_tag_json(&CreateTagParams {
            name: "  rust  ".into(), // trim + 大小写不敏感
            color: None,
        }));
        assert_eq!(dup["error_code"], ERROR_DUPLICATE_TAG_NAME);

        let empty = parse(&server.create_tag_json(&CreateTagParams {
            name: "   ".into(),
            color: None,
        }));
        assert_eq!(empty["error_code"], ERROR_INVALID_ARGUMENT);

        let bad_color = parse(&server.create_tag_json(&CreateTagParams {
            name: "新标签".into(),
            color: Some("red".into()),
        }));
        assert_eq!(bad_color["error_code"], ERROR_INVALID_ARGUMENT);

        let missing = parse(&server.rename_tag_json(&RenameTagParams {
            tag_id: 999,
            name: "换名".into(),
        }));
        assert_eq!(missing["error_code"], ERROR_TAG_NOT_FOUND);
    }

    /// 目标形态：混用 / 都缺 / 空 tag_ids 都是 `invalid_argument`（不静默全库打标）
    #[test]
    fn assign_targets_are_mutually_exclusive_and_required() {
        let server = server();
        let entries = |p: &AssignTagsParams| {
            parse(&server.assign_tags_json(p))["error_code"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        };

        let mixed = AssignTagsParams {
            ids: Some(vec![1]),
            feed_id: Some(1),
            tag_ids: vec![1],
            ..Default::default()
        };
        assert_eq!(entries(&mixed), ERROR_INVALID_ARGUMENT);

        let none = AssignTagsParams {
            tag_ids: vec![1],
            ..Default::default()
        };
        assert_eq!(entries(&none), ERROR_INVALID_ARGUMENT);

        let no_tags = AssignTagsParams {
            ids: Some(vec![1]),
            ..Default::default()
        };
        assert_eq!(entries(&no_tags), ERROR_INVALID_ARGUMENT);
    }

    /// 标签不存在：整单拒绝且**零改动**（不是打一半）
    #[test]
    fn assign_with_missing_tag_changes_nothing() {
        let server = server();
        let out = parse(&server.assign_tags_json(&AssignTagsParams {
            feed_id: Some(1),
            tag_ids: vec![7],
            ..Default::default()
        }));
        assert_eq!(out["error_code"], ERROR_TAG_NOT_FOUND);
        assert_eq!(out["affected"], 0);
        assert!(out["detail"].is_null(), "被拒时不该有 detail: {out}");
    }

    /// 条目存在性：ids 形态里不存在的 id 逐项 `article_not_found`；
    /// 一个都不存在时整单失败（不让 agent 只看 `ok=true`）
    #[test]
    fn assign_reports_missing_entries_item_by_item() {
        let path = {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "rustrss-tag-tools-unit-{}.sqlite",
                std::process::id()
            ));
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
            }
            p
        };
        let server = RustRssMcp::open(&path).expect("建库失败");
        let feed = server.with_store(|s| s.add_feed("https://e.test/a.xml", Some("A")).unwrap());
        server.with_store(|s| {
            s.upsert_entries(
                feed,
                &[Entry {
                    stable_id: "a1".into(),
                    id_origin: IdOrigin::SourceData,
                    source_id: "a1".into(),
                    title: "t".into(),
                    url: None,
                    author: None,
                    published: None,
                    updated: None,
                    summary: None,
                    content_html: None,
                    content_text: None,
                    thumbnail_url: None,
                    categories: Vec::new(),
                }],
            )
            .unwrap()
        });
        let tag = parse(&server.create_tag_json(&CreateTagParams {
            name: "Rust".into(),
            color: None,
        }))["detail"]["id"]
            .as_i64()
            .unwrap();
        let entry_id = server.with_store(|s| s.list_entries(&Default::default()).unwrap()[0].id);

        // 混合存在性：存在的标 ok，不存在的标 article_not_found；affected 只算命中
        let out = parse(&server.assign_tags_json(&AssignTagsParams {
            ids: Some(vec![entry_id, 4242]),
            tag_ids: vec![tag],
            ..Default::default()
        }));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["affected"], 1);
        assert_eq!(out["detail"]["changed"], 1);
        assert_eq!(out["results"][0]["id"], entry_id);
        assert_eq!(out["results"][0]["ok"], true);
        assert_eq!(out["results"][1]["error_code"], ERROR_ARTICLE_NOT_FOUND);

        // 重复调用：严格幂等（affected 稳定、changed=0）
        let again = parse(&server.assign_tags_json(&AssignTagsParams {
            ids: Some(vec![entry_id]),
            tag_ids: vec![tag],
            ..Default::default()
        }));
        assert_eq!(again["affected"], 1);
        assert_eq!(again["detail"]["changed"], 0);

        // 全不存在：整单失败 + 逐项原因
        let all_missing = parse(&server.assign_tags_json(&AssignTagsParams {
            ids: Some(vec![998, 999]),
            tag_ids: vec![tag],
            ..Default::default()
        }));
        assert_eq!(all_missing["ok"], false);
        assert_eq!(all_missing["error_code"], ERROR_ARTICLE_NOT_FOUND);
        assert_eq!(
            all_missing["results"][0]["error_code"],
            ERROR_ARTICLE_NOT_FOUND
        );

        let _ = std::fs::remove_file(&path);
    }

    /// 条件级命中集超过 100 条：`results` 只列前 100（有界采样）+ `results_truncated`，
    /// 但 `affected` 仍是完整命中数（与 `set_read` 的条件级口径一致）。
    #[test]
    fn scoped_assign_samples_the_hit_set_and_flags_truncation() {
        let server = server();
        let feed = server.with_store(|s| s.add_feed("https://e.test/big.xml", Some("B")).unwrap());
        server.with_store(|s| {
            let entries: Vec<Entry> = (1..=101)
                .map(|i| Entry {
                    stable_id: format!("b{i}"),
                    id_origin: IdOrigin::SourceData,
                    source_id: format!("b{i}"),
                    title: format!("t{i}"),
                    url: None,
                    author: None,
                    published: None,
                    updated: None,
                    summary: None,
                    content_html: None,
                    content_text: None,
                    thumbnail_url: None,
                    categories: Vec::new(),
                })
                .collect();
            s.upsert_entries(feed, &entries).unwrap()
        });
        let tag = parse(&server.create_tag_json(&CreateTagParams {
            name: "Rust".into(),
            color: None,
        }))["detail"]["id"]
            .as_i64()
            .unwrap();

        let out = parse(&server.assign_tags_json(&AssignTagsParams {
            feed_id: Some(feed),
            tag_ids: vec![tag],
            ..Default::default()
        }));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["affected"], 101, "affected 是完整命中数");
        assert_eq!(out["results"].as_array().unwrap().len(), MAX_BATCH_IDS);
        assert_eq!(out["detail"]["results_truncated"], true);
        assert_eq!(out["detail"]["target"]["feed_id"], feed);
        // 库内确实 101 条关联（采样只影响响应体积，不影响写入）
        assert_eq!(server.with_store(|s| s.tag_entry_count(tag).unwrap()), 101);
    }

    /// `delete_tag`：缺 `confirm` → `confirm_required`；`dry_run` 不打 confirm 也放行，
    /// 且预览与实际共用 core 同一个计数函数（同一数字）
    #[test]
    fn delete_tag_needs_confirm_and_previews_with_the_same_count() {
        let path = {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "rustrss-tag-delete-unit-{}.sqlite",
                std::process::id()
            ));
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
            }
            p
        };
        let server = RustRssMcp::open(&path).expect("建库失败");
        let feed = server.with_store(|s| s.add_feed("https://e.test/a.xml", Some("A")).unwrap());
        server.with_store(|s| {
            let entries: Vec<Entry> = (1..=3)
                .map(|i| Entry {
                    stable_id: format!("a{i}"),
                    id_origin: IdOrigin::SourceData,
                    source_id: format!("a{i}"),
                    title: format!("t{i}"),
                    url: None,
                    author: None,
                    published: None,
                    updated: None,
                    summary: None,
                    content_html: None,
                    content_text: None,
                    thumbnail_url: None,
                    categories: Vec::new(),
                })
                .collect();
            s.upsert_entries(feed, &entries).unwrap()
        });
        let tag = parse(&server.create_tag_json(&CreateTagParams {
            name: "Rust".into(),
            color: None,
        }))["detail"]["id"]
            .as_i64()
            .unwrap();
        let ids: Vec<i64> = server.with_store(|s| {
            s.list_entries(&Default::default())
                .unwrap()
                .iter()
                .map(|r| r.id)
                .collect()
        });
        server.assign_tags_json(&AssignTagsParams {
            ids: Some(ids),
            tag_ids: vec![tag],
            ..Default::default()
        });

        // 缺 confirm：confirm_required，且库不变（标签还在、关联还在）
        let refused = parse(&server.delete_tag_json(&DeleteTagParams {
            tag_id: tag,
            confirm: None,
            dry_run: None,
        }));
        assert_eq!(
            refused["error_code"],
            write_contract::ERROR_CONFIRM_REQUIRED
        );
        assert_eq!(
            server.with_store(|s| s.tag_entry_count(tag).unwrap()),
            3,
            "被拒的删除不得改库"
        );

        // dry_run：不需要 confirm，返回影响篇数且不落库
        let preview = parse(&server.delete_tag_json(&DeleteTagParams {
            tag_id: tag,
            confirm: None,
            dry_run: Some(true),
        }));
        assert_eq!(preview["dry_run"], true);
        assert_eq!(preview["affected"], 3);
        assert!(
            server.with_store(|s| s.tag_row(tag).unwrap()).is_some(),
            "预览后标签仍在"
        );
        assert_eq!(server.with_store(|s| s.tag_entry_count(tag).unwrap()), 3);

        // 真删：affected 与预览一致（同一个计数函数），标签消失、文章保留
        let done = parse(&server.delete_tag_json(&DeleteTagParams {
            tag_id: tag,
            confirm: Some(true),
            dry_run: None,
        }));
        assert_eq!(done["affected"], preview["affected"]);
        assert!(server.with_store(|s| s.tag_row(tag).unwrap()).is_none());
        assert_eq!(
            server.with_store(|s| s.entry_count().unwrap()),
            3,
            "文章保留"
        );
        assert_eq!(
            server.with_store(|s| s.orphan_entry_tag_count().unwrap()),
            0
        );

        // 删不存在的标签：tag_not_found
        let missing = parse(&server.delete_tag_json(&DeleteTagParams {
            tag_id: 999,
            confirm: Some(true),
            dry_run: None,
        }));
        assert_eq!(missing["error_code"], ERROR_TAG_NOT_FOUND);

        let _ = std::fs::remove_file(&path);
    }
}
