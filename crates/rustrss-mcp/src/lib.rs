//! RustRss MCP 服务器：把本地库里的订阅数据通过 MCP 暴露给外部 agent。
//!
//! 数据来源是 `rustrss-core` 的 SQLite 库（与界面读同一个库，因此不存在
//! 「两套查询逻辑漂移」的问题）。库路径按 `RUSTSS_DB` 环境变量 → 第一个命令行
//! 参数 → `~/.local/share/rustrss/rustrss.sqlite` 的顺序解析。
//!
//! 工具口径（PRD §6 风险 3）：**列表只回元数据 + 短摘要，正文必须单独取**，
//! 且所有列表都有上限——MCP 的响应直接进 agent 上下文，体积必须可控。
//! 只读：写入类工具按 spec 属 P1，暂未开放。
//!
//! 注意：stdout 是协议通道，任何日志只能走 stderr。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub mod config;
pub mod http;

use rmcp::{
    handler::server::wrapper::Parameters, tool, tool_handler, tool_router, transport::stdio,
    ServerHandler, ServiceExt,
};
use rustrss_core::{EntryQuery, ListSort, Store, UnreadGroupBy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 50;
/// 列表里的摘要截断长度：够判断「要不要点进去看」，又不至于撑爆上下文
const SUMMARY_CHARS: usize = 140;

/// `list_articles` 的参数。
///
/// **默认口径固定**：`sort = newest`、`hide_read = false`——不继承用户此刻的界面
/// 设置（`list.sort` / `list.hide_read`）。显式传参才会偏离默认。
/// 分页：`page_size`（别名 `limit`）默认 10、上限 50；翻页把上一页的
/// `next_cursor` 原样回传给 `cursor`。
#[derive(Debug, Default, Deserialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetArticleParams {
    /// 条目 id（来自 list_articles / search_articles）
    pub id: i64,
    /// 是否附带 HTML 正文（默认 false，只回纯文本）
    #[serde(default)]
    pub include_html: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// 关键词（在标题与正文中检索；中文两字及以上可精确匹配）
    pub query: String,
    /// 最多返回多少条（默认 10，上限 50）
    pub limit: Option<u32>,
}

/// `get_unread_summary` 的参数
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UnreadSummaryParams {
    /// 聚合维度：`feed`（默认，按订阅源）或 `folder`（按分组，未分组单列一组）
    pub by: Option<String>,
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
        let shape_err = || {
            format!(
                "cursor 不合法（{text:?}）：请把上一页的 next_cursor 原样回传，不要自己拼"
            )
        };
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
        Ok(Some(Self { sort, sortkey, id, read }))
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
}

#[derive(Serialize)]
struct FolderOut {
    id: i64,
    name: String,
    unread: i64,
}

#[derive(Clone)]
pub struct RustRssMcp {
    store: Arc<Mutex<Store>>,
}

impl RustRssMcp {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    /// 打开（必要时创建/迁移）指定路径的库
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, rustrss_core::StoreError> {
        Ok(Self::new(Store::open(path)?))
    }

    fn with_store<T>(&self, f: impl FnOnce(&Store) -> T) -> T {
        let guard = self.store.lock().expect("库锁被毒化（前一次调用 panic）");
        f(&guard)
    }

    pub fn list_feeds_json(&self) -> String {
        self.with_store(|store| match store.list_feeds() {
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
            let query = EntryQuery {
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
        self.with_store(|store| match store.unread_summary(UnreadGroupBy::Folder) {
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

    /// 未读聚合（按源或按分组）
    pub fn unread_summary_json(&self, p: &UnreadSummaryParams) -> String {
        let by = match p.by.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            None | Some("feed") => UnreadGroupBy::Feed,
            Some("folder") => UnreadGroupBy::Folder,
            Some(other) => {
                return error_json(&format!("by 只支持 feed / folder，收到 {other:?}"))
            }
        };
        self.with_store(|store| match store.unread_summary(by) {
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
        self.with_store(|store| match store.get_entry(p.id) {
            Ok(Some(row)) => {
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
                if p.include_html {
                    body["html"] = serde_json::json!(row.content_html);
                }
                to_json(&body)
            }
            Ok(None) => error_json(&format!("未找到条目 {}；先用 list_articles 取 id", p.id)),
            Err(e) => error_json(&e.to_string()),
        })
    }

    pub fn search_articles_json(&self, p: &SearchParams) -> String {
        let limit = clamp_limit(p.limit);
        self.with_store(|store| match store.search(&p.query, limit) {
            Ok(rows) => {
                let items: Vec<ArticleMetaOut> = rows.iter().map(meta_out).collect();
                to_json(&serde_json::json!({
                    "query": p.query,
                    "count": items.len(),
                    "articles": items,
                }))
            }
            Err(e) => error_json(&e.to_string()),
        })
    }

    pub fn db_stats_json(&self) -> String {
        self.with_store(|store| {
            let feeds = store.list_feeds().map(|f| f.len()).unwrap_or(0);
            let entries = store.entry_count().unwrap_or(0);
            let unread = store.unread_total().unwrap_or(0);
            let starred = store
                .list_entries(&EntryQuery {
                    starred_only: true,
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
}

#[tool_router]
impl RustRssMcp {
    #[tool(description = "列出订阅源及其未读数（含上次抓取状态；状态非 ok 的源数据可能不是最新）")]
    fn list_feeds(&self) -> String {
        self.list_feeds_json()
    }

    #[tool(
        description = "列出分组（文件夹）及其未读合计；未分组订阅的未读单列（ungrouped_unread）、合计 total_unread"
    )]
    fn list_folders(&self) -> String {
        self.list_folders_json()
    }

    #[tool(
        description = "列出条目元数据（不含正文）。默认口径固定：sort=newest 且不隐藏已读（不跟随界面设置，显式传参才覆盖）；每页默认 10 条、上限 50，翻页把 next_cursor 回传给 cursor；可用 feed_id / folder_id（二选一）、unread_only、starred_only、read_later_only、since / until（对 COALESCE(published_at,fetched_at) 的闭区间，Unix 秒）、sort（newest/oldest/unread_first）、hide_read。正文用 get_article 单独取。"
    )]
    fn list_articles(&self, Parameters(p): Parameters<ListArticlesParams>) -> String {
        self.list_articles_json(&p)
    }

    #[tool(description = "取单篇文章正文（默认纯文本，可要求附带 HTML 原始正文）")]
    fn get_article(&self, Parameters(p): Parameters<GetArticleParams>) -> String {
        self.get_article_json(&p)
    }

    #[tool(description = "全文搜索标题与正文（中文需两字及以上）")]
    fn search_articles(&self, Parameters(p): Parameters<SearchParams>) -> String {
        self.search_articles_json(&p)
    }

    #[tool(
        description = "未读聚合：by = feed（默认，按订阅源）或 folder（按分组，未分组单列一组），返回每组的未读数与 total_unread"
    )]
    fn get_unread_summary(&self, Parameters(p): Parameters<UnreadSummaryParams>) -> String {
        self.unread_summary_json(&p)
    }

    #[tool(description = "库的总体统计：订阅源数、条目数、未读数")]
    fn db_stats(&self) -> String {
        self.db_stats_json()
    }
}

#[tool_handler(
    name = "rustrss",
    instructions = "读取本地 RustRss 订阅库。列表工具只回元数据与短摘要，正文需用 get_article 单独取；只读，不会修改你的阅读状态。"
)]
impl ServerHandler for RustRssMcp {}

/// 解析库路径（与界面共用 core 的同一套规则，保证两边指向同一个文件）
pub fn resolve_db_path() -> PathBuf {
    rustrss_core::resolve_db_path()
}

/// 以 stdio 传输运行（由 MCP 客户端作为子进程拉起）
pub async fn serve_stdio(server: RustRssMcp) -> anyhow::Result<()> {    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn meta_out(row: &rustrss_core::EntryRow) -> ArticleMetaOut {
    ArticleMetaOut {
        id: row.id,
        feed: row.feed_title.clone(),
        title: row.title.clone(),
        url: row.url.clone(),
        published_at: row.published_at,
        read: row.read,
        starred: row.starred,
        summary: row.summary.as_deref().map(truncate),
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
