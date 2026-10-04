//! Shared read-only projections. MCP uses unbounded legacy JSON; chat adds validation,
//! mandatory scope and byte gates without changing the MCP contract.
use super::chat::ChatTool;
use crate::{EntryQuery, ListSort, Store, TagRow, UnreadGroupBy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::LazyLock;
const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 50;
const SUMMARY_CHARS: usize = 140;
pub const TAG_LIST_MAX: usize = 200;
pub const TAGS_PER_ENTRY_MAX: usize = 20;
const ERROR_INVALID_ARGUMENT: &str = "invalid_argument";
const ERROR_TAG_NOT_FOUND: &str = "tag_not_found";
const ERROR_INTERNAL: &str = "internal_error";
/// `list_articles` 的参数。
///
/// **默认口径固定**：`sort = newest`、`hide_read = false`——不继承用户此刻的界面
/// 设置（`list.sort` / `list.hide_read`）。显式传参才会偏离默认。
/// 分页：`page_size`（别名 `limit`）默认 10、上限 50；翻页把上一页的
/// `next_cursor` 原样回传给 `cursor`。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ListArticlesParams {
    /// 只看某个订阅源（feed id，来自 list_feeds）；与 folder_id 二选一
    pub feed_id: Option<i64>,
    /// 只看某个分组下的订阅源（folder id，来自 list_folders）；与 feed_id 二选一
    pub folder_id: Option<i64>,
    /// 只看未读
    pub unread_only: bool,
    /// 只看星标
    pub starred_only: bool,
    /// 只看「稍后读」
    pub read_later_only: bool,
    /// 只看带某个标签的条目（tag id 来自 list_tags）；与 `tag_name` 二选一
    pub tag_id: Option<i64>,
    /// 只看带某个标签的条目（按名称，大小写不敏感）；与 `tag_id` 二选一
    pub tag_name: Option<String>,
    /// 只看该时刻（含）之后的条目；比较的是列表排序键
    /// `COALESCE(published_at, fetched_at)`（Unix 秒）
    pub since: Option<i64>,
    /// 只看该时刻（含）之前的条目；闭区间，口径同 since
    pub until: Option<i64>,
    /// 每页条数（默认 10，上限 50）；与 limit 同义，page_size 优先
    pub page_size: Option<u32>,
    /// page_size 的兼容别名（旧调用方用）；两者都给时 page_size 生效
    pub limit: Option<u32>,
    /// 上一页返回的 `next_cursor` 原样回传；不传 = 取首页
    pub cursor: Option<String>,
    /// 排序档：`newest`（默认）/ `oldest` / `unread_first`；与 cursor 必须同一档
    pub sort: Option<String>,
    /// 是否隐藏已读（默认 false = 不隐藏；与界面设置无关）
    pub hide_read: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct GetArticleParams {
    /// 条目 id（来自 list_articles / search_articles）
    pub id: i64,
    /// 是否附带 HTML 正文（默认 false，只回纯文本）
    #[serde(default)]
    pub include_html: bool,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// 关键词（在标题与正文中检索；中文两字及以上可精确匹配）
    pub query: String,
    /// 最多返回多少条（默认 10，上限 50）
    pub limit: Option<u32>,
}

/// `digest_list` 的参数。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DigestListParams {
    /// 最多返回多少份（默认 10，上限 50；按日期倒序）
    pub limit: Option<u32>,
}

/// `digest_get` 的参数。
#[derive(Debug, Deserialize)]
pub struct DigestGetParams {
    /// 日期（YYYY-MM-DD，来自 digest_list）
    pub date: String,
    /// 范围键（来自 digest_list；缺省 "all" = 全部订阅源）
    pub scope_key: Option<String>,
    /// 来源清单每页条数（默认 10，上限 50）
    pub items_page_size: Option<u32>,
    /// 来源清单页码（从 1 起；缺省第 1 页）
    pub items_page: Option<u32>,
}

/// `get_unread_summary` 的参数
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UnreadSummaryParams {
    /// 聚合维度：`feed`（默认，按订阅源）或 `folder`（按分组，未分组单列一组）
    pub by: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ListTagsParams {
    pub sort: Option<String>,
}
/// 列表分页游标：`<sort>:<sortkey>:<id>`（`unread_first` 档多一位 `:<read>`）。
///
/// 带上排序档是**故意的**：换了排序档却复用旧游标，keyset 定位到的是另一套顺序里
/// 的坐标，会翻出乱序/重复页。此处直接报错比静默出错好。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cursor {
    sort: ListSort,
    sortkey: i64,
    id: i64,
    read: Option<bool>,
}

impl Cursor {
    fn encode(&self) -> String {
        let base = format!("{}:{}:{}", self.sort.as_str(), self.sortkey, self.id);
        match (self.sort, self.read) {
            (ListSort::UnreadFirst, Some(read)) => {
                format!("{base}:{}", if read { 1 } else { 0 })
            }
            _ => base,
        }
    }

    /// 解析并校验游标。`expected` 是本次请求的排序档：不一致即报错。
    fn parse(raw: Option<&str>, expected: ListSort) -> std::result::Result<Option<Self>, String> {
        let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
            return Ok(None);
        };
        let shape_err =
            || format!("cursor 不合法（{text:?}）：请把上一页的 next_cursor 原样回传，不要自己拼");
        let parts: Vec<&str> = text.split(':').collect();
        let sort = match parts.first().copied() {
            Some("newest") => ListSort::Newest,
            Some("oldest") => ListSort::Oldest,
            Some("unread_first") => ListSort::UnreadFirst,
            _ => return Err(shape_err()),
        };
        if sort != expected {
            return Err(format!(
                "cursor 属于 sort={} 的翻页结果，与本次 sort={} 不匹配",
                sort.as_str(),
                expected.as_str()
            ));
        }
        let need = if sort == ListSort::UnreadFirst { 4 } else { 3 };
        if parts.len() != need {
            return Err(shape_err());
        }
        let sortkey = parts[1].parse::<i64>().map_err(|_| shape_err())?;
        let id = parts[2].parse::<i64>().map_err(|_| shape_err())?;
        let read = match parts.get(3).copied() {
            None => None,
            Some("0") => Some(false),
            Some("1") => Some(true),
            Some(_) => return Err(shape_err()),
        };
        Ok(Some(Self {
            sort,
            sortkey,
            id,
            read,
        }))
    }
}

#[derive(Serialize)]
struct FeedOut {
    id: i64,
    title: String,
    url: String,
    site_url: Option<String>,
    unread: i64,
    /// 上次抓取状态；非 "ok" 时 agent 应知道这个源目前不可信
    status: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct ArticleMetaOut {
    id: i64,
    feed: String,
    title: String,
    url: Option<String>,
    published_at: Option<i64>,
    read: bool,
    starred: bool,
    summary: Option<String>,
    /// 该条目的标签名（只回名称；写工具的 tag_id 用 list_tags 取）。
    /// 超过 [`TAGS_PER_ENTRY_MAX`] 时截断并置 `tags_truncated`
    tags: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    tags_truncated: bool,
}

#[derive(Serialize)]
struct FolderOut {
    id: i64,
    name: String,
    unread: i64,
}

struct ReadTools<'a> {
    store: &'a Store,
    scope_key: Option<&'a str>,
    feed_ids: Option<Vec<i64>>,
    bounded: bool,
}
impl ReadTools<'_> {
    fn feeds(&self) -> crate::store::Result<Vec<crate::FeedRow>> {
        self.store.list_feeds().map(|rows| {
            rows.into_iter()
                .filter(|f| self.feed_ids.as_ref().is_none_or(|ids| ids.contains(&f.id)))
                .collect()
        })
    }
    fn groups(&self, by: UnreadGroupBy) -> crate::store::Result<Vec<crate::UnreadGroup>> {
        if self.feed_ids.is_none() {
            return self.store.unread_summary(by);
        }
        let feeds = self.feeds()?;
        if matches!(by, UnreadGroupBy::Feed) {
            return Ok(feeds
                .into_iter()
                .map(|f| crate::UnreadGroup {
                    id: Some(f.id),
                    name: f.title,
                    unread: f.unread,
                })
                .collect());
        }
        let mut groups = Vec::new();
        for (id, name) in self
            .store
            .list_folders()?
            .into_iter()
            .map(|(id, name)| (Some(id), name))
            .chain(std::iter::once((None, "未分组".into())))
        {
            let members: Vec<_> = feeds.iter().filter(|f| f.folder_id == id).collect();
            if !members.is_empty() {
                groups.push(crate::UnreadGroup {
                    id,
                    name,
                    unread: members.iter().map(|f| f.unread).sum(),
                });
            }
        }
        Ok(groups)
    }
    fn with_store<T>(&self, f: impl FnOnce(&Store) -> T) -> T {
        f(self.store)
    }
    pub fn list_feeds_json(&self) -> String {
        self.with_store(|_store| match self.feeds() {
            Ok(feeds) => {
                let out: Vec<FeedOut> = feeds
                    .into_iter()
                    .map(|f| FeedOut {
                        id: f.id,
                        title: f.title,
                        url: f.url,
                        site_url: f.site_url,
                        unread: f.unread,
                        status: f.last_status,
                        error: f.last_error,
                    })
                    .collect();
                to_json(&serde_json::json!({
                    "count": out.len(),
                    "feeds": out,
                }))
            }
            Err(e) => error_json(&e.to_string()),
        })
    }

    pub fn list_articles_json(&self, p: &ListArticlesParams) -> String {
        // 两个源过滤是两种口径（单源 / 多源），同时给无从裁决 → 直接报错
        if p.feed_id.is_some() && p.folder_id.is_some() {
            return error_json("feed_id 与 folder_id 只能用一个（二者互斥）");
        }
        // 标签过滤同样二选一：id 与名称同时给无从裁决
        if p.tag_id.is_some() && p.tag_name.is_some() {
            return error_body(
                ERROR_INVALID_ARGUMENT,
                "tag_id 与 tag_name 只能用一个（二者互斥）",
            );
        }
        let sort = match parse_sort(p.sort.as_deref()) {
            Ok(s) => s,
            Err(e) => return error_json(&e),
        };
        let cursor = match Cursor::parse(p.cursor.as_deref(), sort) {
            Ok(c) => c,
            Err(e) => return error_json(&e),
        };
        let page_size = clamp_limit(p.page_size.or(p.limit));
        self.with_store(|store| {
            // 分组过滤：先把 folder_id 解析成该分组下全部源 id，再走 core 的多源 IN。
            // 空分组解析出空列表，core 对空列表的语义是「匹配零条」
            // （绝不能退化成「不过滤」把全库倒给 agent）。
            let feed_ids = match p.folder_id {
                Some(folder_id) => match store.feed_ids_in_folder(folder_id) {
                    Ok(ids) => Some(ids),
                    Err(e) => return error_json(&e.to_string()),
                },
                None => None,
            };
            let feed_ids = match (&self.feed_ids, feed_ids) {
                (None, ids) => ids,
                (Some(scope), None) => Some(scope.clone()),
                (Some(scope), Some(ids)) => {
                    Some(ids.into_iter().filter(|id| scope.contains(id)).collect())
                }
            };
            // 标签过滤：tag_name 在这里解析成 id（大小写不敏感，与 UNIQUE COLLATE NOCASE
            // 同口径）；未知标签 → 机器可读的 `tag_not_found`，而不是静默空列表
            let tag_id = match resolve_tag_filter(store, p.tag_id, p.tag_name.as_deref()) {
                Ok(v) => v,
                Err(e) => return error_body(e.code, &e.message),
            };
            let query = EntryQuery {
                folder_id: None, // MCP already resolves folder membership into feed_ids.
                feed_id: p.feed_id,
                feed_ids,
                unread_only: p.unread_only,
                starred_only: p.starred_only,
                read_later_only: p.read_later_only,
                since: p.since,
                until: p.until,
                limit: Some(page_size),
                cursor: cursor.map(|c| (c.sortkey, c.id)),
                cursor_read: cursor.and_then(|c| c.read),
                // MCP 默认口径固定（不继承界面设置）：显式传参才偏离默认
                sort: Some(sort),
                hide_read: Some(p.hide_read.unwrap_or(false)),
                tag_id,
            };
            match store.list_entries(&query) {
                Ok(rows) => {
                    let items: Vec<ArticleMetaOut> = rows.iter().map(meta_out).collect();
                    // 只在「满页」时给下一页游标：不满页说明已到底，
                    // 再给游标会让客户端多翻一页空的（对 agent 是浪费一轮）
                    let next_cursor = if items.len() == page_size as usize {
                        rows.last().map(|r| {
                            Cursor {
                                sort,
                                sortkey: r.sortkey,
                                id: r.id,
                                read: match sort {
                                    ListSort::UnreadFirst => Some(r.read),
                                    _ => None,
                                },
                            }
                            .encode()
                        })
                    } else {
                        None
                    };
                    to_json(&serde_json::json!({
                        "count": items.len(),
                        "page_size": page_size,
                        "articles": items,
                        "next_cursor": next_cursor,
                        "hint": "正文请用 get_article(id) 单独取；翻页把 next_cursor 原样回传",
                    }))
                }
                Err(e) => error_json(&e.to_string()),
            }
        })
    }

    /// 分组清单 + 每组未读合计（未分组单列）
    pub fn list_folders_json(&self) -> String {
        self.with_store(|_store| match self.groups(UnreadGroupBy::Folder) {
            Ok(groups) => {
                let folders: Vec<FolderOut> = groups
                    .iter()
                    .filter_map(|g| {
                        g.id.map(|id| FolderOut {
                            id,
                            name: g.name.clone(),
                            unread: g.unread,
                        })
                    })
                    .collect();
                let ungrouped_unread = groups
                    .iter()
                    .find(|g| g.id.is_none())
                    .map(|g| g.unread)
                    .unwrap_or(0);
                let total_unread: i64 = groups.iter().map(|g| g.unread).sum();
                to_json(&serde_json::json!({
                    "count": folders.len(),
                    "folders": folders,
                    "ungrouped_unread": ungrouped_unread,
                    "total_unread": total_unread,
                }))
            }
            Err(e) => error_json(&e.to_string()),
        })
    }

    /// 日报列表：元数据 + ≤140 字概览（默认 10、上限 50，按日期倒序）
    pub fn digest_list_json(&self, p: &DigestListParams) -> String {
        let limit = clamp_limit(p.limit);
        self.with_store(
            |store| match store.digest_list_scoped(limit, self.scope_key) {
                Ok(items) => {
                    let list: Vec<serde_json::Value> = items
                        .iter()
                        .map(|i| {
                            serde_json::json!({
                                "date": i.report_day,
                                "scope_key": i.scope_key,
                                "generated_at": i.generated_at,
                                "article_count": i.article_count,
                                // ≤140 字概览；报告正文走 digest_get 单取
                                "overview": i.overview,
                            })
                        })
                        .collect();
                    to_json(&serde_json::json!({ "count": list.len(), "digests": list }))
                }
                Err(e) => error_json(&e.to_string()),
            },
        )
    }

    /// 单份日报：头 + 正文（Markdown）+ 来源清单分页（默认 10、上限 50）
    pub fn digest_get_json(&self, p: &DigestGetParams) -> String {
        let scope_key = p
            .scope_key
            .as_deref()
            .unwrap_or(self.scope_key.unwrap_or("all"));
        if self.scope_key.is_some_and(|key| key != scope_key) {
            return error_body("scope_denied", "digest is outside session scope");
        }
        let page_size = clamp_limit(p.items_page_size);
        let page = p.items_page.unwrap_or(1).max(1);
        self.with_store(|store| {
            match if self.bounded {
                store.digest_report_bounded(&p.date, scope_key, TOOL_BYTES)
            } else {
                store
                    .digest_report(&p.date, scope_key)
                    .map(|row| row.map(|report| (report, false)))
            } {
                Ok(Some((report, body_truncated))) => {
                    // 来源清单分页（内存切片：清单上限即 200，无需 SQL 游标）
                    let total = report.items.len();
                    let start = ((page - 1) as usize * page_size as usize).min(total);
                    let end = (start + page_size as usize).min(total);
                    let items: Vec<serde_json::Value> = report.items[start..end]
                        .iter()
                        .map(|i| {
                            serde_json::json!({
                                "title": i.title,
                                "feed_id": i.feed_id,
                                // agent 可用 get_article(feed 过滤) 取正文
                                "effective_at": i.effective_at,
                            })
                        })
                        .collect();
                    // 正文上限：日报本身有 200 篇预算，成品 Markdown 天然有界；
                    // 仍按响应口径设硬顶，超出显式标注（不静默截断语义）
                    const MARKDOWN_MAX: usize = 32 * 1024;
                    let (markdown, truncated) = if report.markdown.len() > MARKDOWN_MAX {
                        // 按 UTF-8 字符边界截断
                        let mut cut = MARKDOWN_MAX;
                        while !report.markdown.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        (report.markdown[..cut].to_string(), true)
                    } else {
                        (report.markdown.clone(), body_truncated)
                    };
                    to_json(&serde_json::json!({
                        "date": report.date,
                        "scope_key": report.scope_key,
                        "generated_at": report.generated_at,
                        "checkpoint_at": report.checkpoint_at,
                        "article_count": report.article_count,
                        "cache_hits": report.cache_hits,
                        "markdown": markdown,
                        "markdown_truncated": truncated,
                        "sources": {
                            "total": total,
                            "page": page,
                            "page_size": page_size,
                            "items": items,
                        },
                    }))
                }
                Ok(None) => error_json(&format!(
                    "未找到 {date}（范围 {scope_key}）的日报；先用 digest_list 取可用日期",
                    date = p.date
                )),
                Err(e) => error_json(&e.to_string()),
            }
        })
    }

    /// 未读聚合（按源或按分组）
    pub fn unread_summary_json(&self, p: &UnreadSummaryParams) -> String {
        let by = match p.by.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            None | Some("feed") => UnreadGroupBy::Feed,
            Some("folder") => UnreadGroupBy::Folder,
            Some(other) => return error_json(&format!("by 只支持 feed / folder，收到 {other:?}")),
        };
        self.with_store(|_store| match self.groups(by) {
            Ok(groups) => {
                let total_unread: i64 = groups.iter().map(|g| g.unread).sum();
                to_json(&serde_json::json!({
                    "by": match by {
                        UnreadGroupBy::Feed => "feed",
                        UnreadGroupBy::Folder => "folder",
                    },
                    "count": groups.len(),
                    "total_unread": total_unread,
                    "groups": groups,
                }))
            }
            Err(e) => error_json(&e.to_string()),
        })
    }

    pub fn get_article_json(&self, p: &GetArticleParams) -> String {
        self.with_store(|store| {
            match if self.bounded {
                store.get_entry_bounded(p.id, self.feed_ids.as_deref(), TOOL_BYTES)
            } else {
                store.get_entry(p.id).map(|row| row.map(|r| (r, false)))
            } {
                Ok(Some((row, truncated))) => {
                    let text = row.content_text.unwrap_or_default();
                    let mut body = serde_json::json!({
                        "id": row.id,
                        "feed": row.feed_title,
                        "title": row.title,
                        "url": row.url,
                        "published_at": row.published_at,
                        "read": row.read,
                        "starred": row.starred,
                        // 纯文本：agent 绝大多数场景只需要这个
                        "text": text,
                    });
                    if truncated {
                        body["text_truncated"] = json!(true);
                    }
                    if p.include_html {
                        body["html"] = serde_json::json!(row.content_html);
                    }
                    to_json(&body)
                }
                Ok(None) => error_json(&format!("未找到条目 {}；先用 list_articles 取 id", p.id)),
                Err(e) => error_json(&e.to_string()),
            }
        })
    }

    pub fn search_articles_json(&self, p: &SearchParams) -> String {
        let limit = clamp_limit(p.limit);
        self.with_store(|store| {
            match store.search_scoped(&p.query, limit, self.feed_ids.as_deref()) {
                Ok(rows) => {
                    let items: Vec<ArticleMetaOut> = rows.iter().map(meta_out).collect();
                    to_json(&serde_json::json!({
                        "query": p.query,
                        "count": items.len(),
                        "articles": items,
                    }))
                }
                Err(e) => error_json(&e.to_string()),
            }
        })
    }

    pub fn db_stats_json(&self) -> String {
        self.with_store(|store| {
            let feeds = self.feeds().map(|f| f.len()).unwrap_or(0);
            let entries = match &self.feed_ids {
                None => store.entry_count().unwrap_or(0),
                Some(ids) => ids
                    .iter()
                    .map(|id| store.entry_count_for_feed(*id).unwrap_or(0))
                    .sum(),
            };
            let unread = if self.feed_ids.is_none() {
                store.unread_total().unwrap_or(0)
            } else {
                self.feeds()
                    .map(|f| f.iter().map(|f| f.unread).sum::<i64>())
                    .unwrap_or(0)
            };
            let starred = store
                .list_entries(&EntryQuery {
                    starred_only: true,
                    feed_ids: self.feed_ids.clone(),
                    limit: Some(1),
                    ..Default::default()
                })
                .map(|v| v.len())
                .unwrap_or(0);
            to_json(&serde_json::json!({
                "feeds": feeds,
                "entries": entries,
                "unread": unread,
                "has_starred": starred > 0,
            }))
        })
    }
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
}
fn meta_out(row: &crate::EntryRow) -> ArticleMetaOut {
    ArticleMetaOut {
        id: row.id,
        feed: row.feed_title.clone(),
        title: row.title.clone(),
        url: row.url.clone(),
        published_at: row.published_at,
        read: row.read,
        starred: row.starred,
        summary: row.summary.as_deref().map(truncate),
        tags: row
            .tags
            .iter()
            .take(TAGS_PER_ENTRY_MAX)
            .map(|t| t.name.clone())
            .collect(),
        tags_truncated: row.tags.len() > TAGS_PER_ENTRY_MAX,
    }
}

fn truncate(text: &str) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= SUMMARY_CHARS {
        return cleaned;
    }
    let cut: String = cleaned.chars().take(SUMMARY_CHARS).collect();
    format!("{cut}…")
}

fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// 排序档参数解析：`None`/空串 = 默认 `newest`；其它不合法值**报错**，
/// 不静默回默认（agent 写错档位时希望被告知，而不是拿到它没要的顺序）。
fn parse_sort(raw: Option<&str>) -> std::result::Result<ListSort, String> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(ListSort::Newest),
        Some("newest") => Ok(ListSort::Newest),
        Some("oldest") => Ok(ListSort::Oldest),
        Some("unread_first") => Ok(ListSort::UnreadFirst),
        Some(other) => Err(format!(
            "sort 只支持 newest / oldest / unread_first，收到 {other:?}"
        )),
    }
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| error_json(&format!("序列化失败: {e}")))
}

fn error_json(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}

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
    store: &crate::Store,
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

fn error_body(code: &str, message: &str) -> String {
    json!({"error_code":code,"error":message}).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub data_json: String,
    pub truncated: bool,
}
#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
}
impl ToolError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_argument",
            message: message.into(),
        }
    }
}

/// Owned protocol types require lazy initialization (String/serde_json::Value).
pub static CHAT_TOOLS: LazyLock<Vec<ChatTool>> = LazyLock::new(|| {
    let integer = json!({"type":"integer"});
    let string = json!({"type":"string"});
    let boolean = json!({"type":"boolean"});
    let specs = [
        (
            "list_feeds",
            "List subscribed feeds and unread counts",
            json!({}),
            vec![],
        ),
        (
            "list_folders",
            "List folders and unread counts",
            json!({}),
            vec![],
        ),
        (
            "list_articles",
            "List article metadata; use get_article for text",
            json!({
                "feed_id":integer,"folder_id":integer,"unread_only":boolean,"starred_only":boolean,
                "read_later_only":boolean,"tag_id":integer,"tag_name":string,"since":integer,"until":integer,
                "page_size":integer,"limit":integer,"cursor":string,"hide_read":boolean,
                "sort":{"type":"string","enum":["newest","oldest","unread_first"]}
            }),
            vec![],
        ),
        (
            "search_articles",
            "Search local titles and text with FTS",
            json!({"query":string,"limit":integer}),
            vec!["query"],
        ),
        (
            "get_article",
            "Read one local article; never fetch remote content",
            json!({"id":integer,"include_html":boolean}),
            vec!["id"],
        ),
        (
            "get_unread_summary",
            "Unread counts grouped by feed or folder",
            json!({"by":{"type":"string","enum":["feed","folder"]}}),
            vec![],
        ),
        (
            "db_stats",
            "Read local library statistics",
            json!({}),
            vec![],
        ),
        (
            "list_tags",
            "List tag metadata (all-library scope only)",
            json!({"sort":{"type":"string","enum":["sidebar","recent"]}}),
            vec![],
        ),
        (
            "digest_list",
            "List already generated digest metadata",
            json!({"limit":integer}),
            vec![],
        ),
        (
            "digest_get",
            "Read an already generated digest, without generating one",
            json!({"date":string,"scope_key":string,"items_page_size":integer,"items_page":integer}),
            vec!["date"],
        ),
    ];
    specs.into_iter().map(|(name, description, properties, required)| ChatTool {
        name:name.into(), description:description.into(),
        parameters_json:json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }).collect()
});
pub fn chat_tools() -> &'static [ChatTool] {
    &CHAT_TOOLS
}

pub fn validate_tool(name: &str, args: &Value) -> Result<(), ToolError> {
    let tool = chat_tools()
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| ToolError {
            code: "unknown_tool",
            message: name.into(),
        })?;
    let object = args
        .as_object()
        .ok_or_else(|| ToolError::invalid("parameters must be an object"))?;
    for key in tool.parameters_json["required"].as_array().unwrap() {
        if !object.contains_key(key.as_str().unwrap()) {
            return Err(ToolError::invalid(format!("missing {key}")));
        }
    }
    for (key, value) in object {
        let schema = &tool.parameters_json["properties"][key];
        let valid = match schema["type"].as_str() {
            Some("integer") => value.as_i64().is_some(),
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            _ => false,
        };
        if !valid
            || schema
                .get("enum")
                .is_some_and(|e| !e.as_array().unwrap().contains(value))
        {
            return Err(ToolError::invalid(format!("invalid {key}")));
        }
    }
    Ok(())
}

/// The unbounded projection preserves the legacy MCP JSON byte-for-byte.
pub fn project_tool(store: &Store, name: &str, args: &Value) -> String {
    project(
        &ReadTools {
            store,
            scope_key: None,
            feed_ids: None,
            bounded: false,
        },
        name,
        args,
    )
}
fn project(tools: &ReadTools<'_>, name: &str, args: &Value) -> String {
    macro_rules! call {
        ($method:ident, $ty:ty) => {
            match serde_json::from_value::<$ty>(args.clone()) {
                Ok(p) => tools.$method(&p),
                Err(e) => error_body("invalid_argument", &e.to_string()),
            }
        };
    }
    match name {
        "list_feeds" => tools.list_feeds_json(),
        "list_folders" => tools.list_folders_json(),
        "list_articles" => call!(list_articles_json, ListArticlesParams),
        "search_articles" => call!(search_articles_json, SearchParams),
        "get_article" => call!(get_article_json, GetArticleParams),
        "get_unread_summary" => call!(unread_summary_json, UnreadSummaryParams),
        "db_stats" => tools.db_stats_json(),
        "list_tags" => call!(list_tags_json, ListTagsParams),
        "digest_list" => call!(digest_list_json, DigestListParams),
        "digest_get" => call!(digest_get_json, DigestGetParams),
        _ => error_body("unknown_tool", name),
    }
}
fn scrub_tool_strings(value: &mut Value) {
    match value {
        Value::String(text) => *text = crate::logging::scrub_log_line(text),
        Value::Array(values) => values.iter_mut().for_each(scrub_tool_strings),
        Value::Object(values) => values.values_mut().for_each(scrub_tool_strings),
        _ => {}
    }
}
pub const TOOL_BYTES: usize = 12 * 1024;
/// Valid JSON envelope on truncation; never feed broken JSON to a provider adapter.
/// excerpt is a UTF-8 prefix of the shared projection. Structured scope count
/// survives repeated truncation; max must fit the empty envelope (<=80 bytes
/// even for a u64 scope count, versus 32 bytes for an unscoped result).
pub fn bound_output(mut output: ToolOutput, max: usize) -> ToolOutput {
    // The empty truncation envelope is 31 bytes; agent callers reserve >=32.
    assert!(max >= 32, "tool envelope requires at least 32 bytes");
    if output.data_json.len() <= max {
        return output;
    }
    let scope_count = serde_json::from_str::<Value>(&output.data_json)
        .ok()
        .and_then(|value| value.get("scope_feed_count").and_then(Value::as_u64));
    let mut envelope = json!({"truncated":true,"excerpt":""});
    if let Some(count) = scope_count {
        envelope["scope_feed_count"] = json!(count);
    }
    assert!(
        envelope.to_string().len() <= max,
        "tool scope envelope exceeds budget"
    );
    let mut cut = output.data_json.len().min(max);
    loop {
        while !output.data_json.is_char_boundary(cut) {
            cut -= 1;
        }
        envelope["excerpt"] = json!(&output.data_json[..cut]);
        let data = envelope.to_string();
        if data.len() <= max {
            output.data_json = data;
            output.truncated = true;
            return output;
        }
        cut = cut.saturating_sub((data.len() - max).max(1));
    }
}
pub fn run_tool(store: &Store, name: &str, args: &Value) -> Result<ToolOutput, ToolError> {
    run_scoped_tool(store, "all", name, args)
}

pub fn scope_feeds(store: &Store, key: &str) -> Result<Option<Vec<i64>>, ToolError> {
    if key == "all" {
        return Ok(None);
    }
    let ids = key
        .strip_prefix("tags:")
        .ok_or_else(|| ToolError::invalid("invalid session scope"))?
        .split(',')
        .map(|s| {
            s.parse::<i64>()
                .map_err(|_| ToolError::invalid("invalid tag id"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    crate::store::digest::DigestScope::resolve(store, &ids)
        .map(|s| s.feed_ids)
        .map_err(|e| ToolError {
            code: "store_error",
            message: e.to_string(),
        })
}
pub fn run_scoped_tool(
    store: &Store,
    scope_key: &str,
    name: &str,
    args: &Value,
) -> Result<ToolOutput, ToolError> {
    validate_tool(name, args)?;
    let feed_ids = scope_feeds(store, scope_key)?;
    if feed_ids.is_some() && name == "list_tags" {
        return Err(ToolError {
            code: "scope_unsupported",
            message: "tag counts are all-library only".into(),
        });
    }
    let tools = ReadTools {
        store,
        scope_key: Some(scope_key),
        feed_ids,
        bounded: true,
    };
    let mut data = project(&tools, name, args);
    let mut value: Value =
        serde_json::from_str(&data).map_err(|e| ToolError::invalid(e.to_string()))?;
    scrub_tool_strings(&mut value);
    data = value.to_string();
    if let Some(error) = value.get("error") {
        let code = match value["error_code"].as_str() {
            Some("invalid_argument") => "invalid_argument",
            Some("scope_denied") => "scope_denied",
            Some("tag_not_found") => "tag_not_found",
            Some("internal_error") => "internal_error",
            _ => "tool_error",
        };
        return Err(ToolError {
            code,
            message: error.to_string(),
        });
    }
    let truncated = value["text_truncated"].as_bool().unwrap_or(false)
        || value["markdown_truncated"].as_bool().unwrap_or(false)
        || value["truncated"].as_bool().unwrap_or(false);
    if let Some(ids) = &tools.feed_ids {
        value["scope_feed_count"] = json!(ids.len());
        data = value.to_string();
    }
    Ok(bound_output(
        ToolOutput {
            data_json: data,
            truncated,
        },
        TOOL_BYTES,
    ))
}
