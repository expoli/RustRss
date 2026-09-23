//! RustRss MCP 服务器：把本地库里的订阅数据通过 MCP 暴露给外部 agent。
//!
//! 数据来源是 `rustrss-core` 的 SQLite 库（与界面读同一个库，因此不存在
//! 「两套查询逻辑漂移」的问题）。库路径按 `RUSTSS_DB` 环境变量 → 第一个命令行
//! 参数 → `~/.local/share/rustrss/rustrss.sqlite` 的顺序解析。
//!
//! 工具口径（PRD §6 风险 3）：**列表只回元数据 + 短摘要，正文必须单独取**，
//! 且所有列表都有上限——MCP 的响应直接进 agent 上下文，体积必须可控。
//!
//! 授权（见 [`registry`]）：默认全只读。写工具需要写 token（HTTP 按请求现算 scope）、
//! 写能力总开关与写 token 本身（stdio 同口径），危险工具额外看危险开关；
//! 无权限时返回工具级错误码（`write_scope_required` / `write_disabled` /
//! `dangerous_tool_disabled`），传输层 401 口径不变。
//!
//! 注意：stdout 是协议通道，任何日志只能走 stderr。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub mod audit;
pub mod config;
pub mod http;
pub mod registry;
pub mod write_contract;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Extensions,
    ListToolsResult, PaginatedRequestParams, ResultType, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{tool, tool_handler, tool_router, transport::stdio, ServerHandler, ServiceExt};
use rustrss_core::{EntryQuery, ListSort, RefreshGate, Store, UnreadGroupBy};
use serde_json::Value;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use registry::{GateError, Scope, Switches, ToolSpec};

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
    /// 刷新单 flight：与界面刷新共用同一个 gate（应用内托管时由 src-tauri 注入）。
    /// T3 的 `refresh` 工具会用它，本任务先把共享设施搬到位。
    refresh_gate: Arc<RefreshGate>,
    /// 测试专用桦工具（生产恒为空）。见 [`RustRssMcp::with_test_tool`]。
    test_tools: Arc<Vec<TestTool>>,
}

/// 测试专用桦工具：T3/T4 的真实写工具落地前，授权矩阵需要「已登记、可调用」的写工具。
///
/// 它走的是**与真实工具同一条**路径：同一个注册表元数据、同一个 gating/错误码、
/// 同一份审计行——只是工具体由测试提供（详见 `tests/write_auth.rs`）。
/// 生产环境不会注册任何桦工具（`test_tools` 恒为空），所以这个缝不会把假工具
/// 暴露给用户。
/// 桩工具的工具体：拿真实调用参数（与生产工具同一个来源），返回工具输出文本
pub type TestToolHandler = dyn Fn(Option<&serde_json::Map<String, Value>>) -> String + Send + Sync;

#[derive(Clone)]
pub struct TestTool {
    pub spec: ToolSpec,
    pub description: &'static str,
    pub handler: Arc<TestToolHandler>,
}

impl RustRssMcp {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            refresh_gate: Arc::new(RefreshGate::new()),
            test_tools: Arc::new(Vec::new()),
        }
    }

    /// 打开（必要时创建/迁移）指定路径的库
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, rustrss_core::StoreError> {
        Ok(Self::new(Store::open(path)?))
    }

    /// 注入刷新单 flight 的共享 gate（应用内托管时由 `src-tauri` 传界面的那个）。
    #[must_use]
    pub fn with_refresh_gate(mut self, gate: Arc<RefreshGate>) -> Self {
        self.refresh_gate = gate;
        self
    }

    /// 共享的刷新单 flight gate（`refresh` 工具与界面刷新的唯一交接口）
    pub fn refresh_gate(&self) -> Arc<RefreshGate> {
        Arc::clone(&self.refresh_gate)
    }

    /// **测试专用**：登记一个桦工具（生产不调这个函数）。
    #[doc(hidden)]
    #[must_use]
    pub fn with_test_tool(
        mut self,
        name: &'static str,
        scope: Scope,
        dangerous: bool,
        description: &'static str,
        handler: impl Fn(Option<&serde_json::Map<String, Value>>) -> String + Send + Sync + 'static,
    ) -> Self {
        let mut tools = (*self.test_tools).clone();
        tools.push(TestTool {
            spec: ToolSpec {
                name,
                scope,
                dangerous,
            },
            description,
            handler: Arc::new(handler),
        });
        self.test_tools = Arc::new(tools);
        self
    }

    fn with_store<T>(&self, f: impl FnOnce(&Store) -> T) -> T {
        let guard = self.store.lock().expect("库锁被毒化（前一次调用 panic）");
        f(&guard)
    }

    /// 授权闸门要的开关快照：**每个请求现读**（不缓存）。
    ///
    /// 读失败时的方向是「拒绝写」：拿不到开关状态就当全部关闭（写工具用不了），
    /// 而不是猜成打开——一次 SQLITE_BUSY 不应该变成一条越权路径。
    pub fn switches(&self) -> Switches {
        self.with_store(|store| match config::switches_from_store(store) {
            Ok(sw) => sw,
            Err(e) => {
                log::warn!(target: "mcp", "mcp-write 读取开关失败（按全关处理）: {e}");
                Switches::default()
            }
        })
    }

    /// 按「当次请求携带的凭据」判定 scope；都不匹配则 `None`（→ 401）。
    ///
    /// **每请求现读库里的 token**：写 token 轮换/销毁后，旧值在下一个请求立刻失效
    /// （包括已建立的 HTTP 连接/会话——授权从不缓存到会话上）。
    pub fn scope_of_credential(&self, presented: &str, static_token: Option<&str>) -> Option<Scope> {
        let (write_token, read_token) = self.with_store(|store| {
            let write = config::write_token_from_store(store).ok().flatten();
            let read = store
                .setting(config::K_TOKEN)
                .ok()
                .flatten()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            (write, read)
        });
        if let Some(write) = write_token {
            if http::constant_eq(presented.as_bytes(), write.as_bytes()) {
                return Some(Scope::Write);
            }
        }
        if let Some(read) = read_token {
            if http::constant_eq(presented.as_bytes(), read.as_bytes()) {
                return Some(Scope::Read);
            }
        }
        // 静态读凭据（CLI 的 `RUSTSS_MCP_TOKEN`）只给读权限
        if let Some(static_token) = static_token.filter(|t| !t.is_empty()) {
            if http::constant_eq(presented.as_bytes(), static_token.as_bytes()) {
                return Some(Scope::Read);
            }
        }
        None
    }

    /// 工具清单（静态注册表的工具 + 测试桦工具）的可见子集。
    ///
    /// 过滤与 gating 用同一个 `registry::authorize`——「列出来却调不动」不可能出现。
    pub fn visible_tools(&self, scope: Scope, switches: &Switches) -> Vec<Tool> {
        let mut tools = Self::tool_router().list_all();
        for stub in self.test_tools.iter() {
            tools.push(Tool::new(
                stub.spec.name,
                stub.description,
                Arc::new(serde_json::Map::new()),
            ));
        }
        let specs = self.tool_specs();
        tools.retain(|t| match specs.iter().find(|s| s.name == t.name.as_ref()) {
            Some(spec) => registry::visible(spec, scope, switches),
            None => false,
        });
        tools
    }

    /// 注册表元数据查询：静态表优先，其次是测试桦工具（名字相同以静态表为准，
    /// 避免测试桦工具覆盖真实工具的作用域）。
    fn tool_specs(&self) -> Vec<ToolSpec> {
        let mut specs = registry::TOOL_SPECS.to_vec();
        for stub in self.test_tools.iter() {
            if !specs.iter().any(|s| s.name == stub.spec.name) {
                specs.push(stub.spec);
            }
        }
        specs
    }

    fn test_tool(&self, name: &str) -> Option<TestTool> {
        self.test_tools.iter().find(|t| t.spec.name == name).cloned()
    }

    /// 当次请求的 scope（HTTP 走中间件注入的 [`http::RequestScope`]；stdio 恒为 write）。
    fn scope_of(&self, context: &RequestContext<RoleServer>) -> Scope {
        scope_from_extensions(&context.extensions)
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

/// 当次请求的 scope：HTTP 由鉴权中间件按下发凭据算出（注入扩展），stdio 无该标记。
///
/// - HTTP：读中间件注入的 [`http::RequestScope`]；**注入缺失时 fail-closed 到 `read`**
///   （将来若有人改了中间件却忘了注入，写工具会变成不可用，而不是变成人人可写）；
/// - stdio：本地进程、没有凭据概念，恒为 `write`——写能力仍由开关①写 token 存在性把关，
///   所以“stdio 与 HTTP 同口径”指的是同一套闸门，而不是同一个 scope 来源。
pub fn scope_from_extensions(extensions: &Extensions) -> Scope {
    match extensions.get::<axum::http::request::Parts>() {
        Some(parts) => parts
            .extensions
            .get::<http::RequestScope>()
            .map(|s| s.scope)
            .unwrap_or(Scope::Read),
        None => Scope::Write,
    }
}

/// 工具级错误的响应体（与写信封同形：机器可读 `error_code` + 人类可读 `error`）
fn gate_error_result(err: GateError) -> CallToolResponse {
    CallToolResult::error(vec![ContentBlock::text(err.to_body())]).into()
}

/// 内部一致性错误码：工具暴露了但没登记授权元数据（fail-closed 拒绝，正常情况不会出现）
const UNREGISTERED_TOOL_CODE: &str = "tool_not_registered";

/// 从工具响应里抽出文本体（审计摘要要解析它；非文本/非 Complete 的响应没有可审计内容）
fn response_text(response: &CallToolResponse) -> Option<&str> {
    match response {
        CallToolResponse::Complete(result) => result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str()),
        _ => None,
    }
}

#[tool_handler(
    name = "rustrss",
    instructions = "读取本地 RustRss 订阅库。列表工具只回元数据与短摘要，正文需用 get_article 单独取；写工具需要写 token（HTTP）且写能力已开启，默认只读。"
)]
impl ServerHandler for RustRssMcp {
    /// `tools/list`：按**当次请求**的 scope 与开关过滤（不缓存会话 scope）。
    ///
    /// 读 token 的会话看不到写工具；写开关/写 token/危险开关任一未就位，
    /// 对应的工具也不会出现在列表里（可见性与可调用性是同一个判断）。
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let scope = self.scope_of(&context);
        let switches = self.switches();
        let supports_cache_hints = context.protocol_version().is_some_and(|version| {
            version >= rmcp::model::ProtocolVersion::V_2026_07_28
        });
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            meta: None,
            tools: self.visible_tools(scope, &switches),
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(rmcp::model::CacheScope::Private),
        })
    }

    /// `tools/call`：先过授权闸门（写工具），再过审计，最后才分发到工具体。
    ///
    /// 拒掉时返回的是**工具级错误**（200 + `isError` + `error_code`），不是 401：
    /// 「任务已认证、但没写权限」要让 agent 读得懂，且不能泄露任何订阅数据。
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let name = request.name.to_string();
        let args = request.arguments.clone();
        let scope = self.scope_of(&context);
        let switches = self.switches();
        let spec = self
            .tool_specs()
            .into_iter()
            .find(|s| s.name == name.as_str());
        // 只有登记过的写工具才需要过闸；未登记的名字交给 router 按 MCP 语义拒（工具不存在）
        let is_write = spec.as_ref().is_some_and(|s| s.scope == Scope::Write);
        if spec.is_none() && Self::tool_router().get(&name).is_some() {
            // 路由里有、注册表里没有：元数据缺失就不知道它是读还是写。
            // **fail-closed 直接拒**，而不是“没元数据就不用过闸”——否则某天新增一个
            // 写工具却忘了登记，读 token 就直接能改了。
            // 单测 `every_exposed_tool_is_registered` 会让这种情况在 CI 里先红。
            audit::emit(
                &name,
                args.as_ref(),
                &audit::AuditSummary::failed(UNREGISTERED_TOOL_CODE),
            );
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                serde_json::json!({
                    "ok": false,
                    "affected": 0,
                    "results": [],
                    "error_code": UNREGISTERED_TOOL_CODE,
                    "error": "工具未在授权注册表登记，已拒绝（请登记 scope/dangerous 元数据）",
                })
                .to_string(),
            )])
            .into());
        }
        if let Some(spec) = spec.as_ref().filter(|s| s.scope == Scope::Write) {
            if let Err(err) = registry::authorize(spec, scope, &switches) {
                let summary = audit::AuditSummary::rejected(err);
                audit::emit(&name, args.as_ref(), &summary);
                return Ok(gate_error_result(err));
            }
        }

        let response = match self.test_tool(&name) {
            Some(stub) => CallToolResponse::Complete(CallToolResult::success(vec![
                ContentBlock::text((stub.handler)(args.as_ref())),
            ])),
            None => {
                let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
                Self::tool_router().call(tcc).await?
            }
        };

        if is_write {
            let summary = response_text(&response)
                .map(audit::summary_from_body)
                .unwrap_or_else(|| audit::AuditSummary::failed("empty_result"));
            audit::emit(&name, args.as_ref(), &summary);
            // 写调用失败 ⇒ 统一标成**工具级错误**（body 里仍有 error_code / 逐项结果）。
            // 这样 agent 不必分辨"失败是闸门给的还是工具体的"：规则只有一条
            // （isError=true + 信封里的 error_code），T3/T4 只要遵守写契约就自动一致。
            if !summary.ok {
                if let Some(body) = response_text(&response) {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(body.to_string())]).into());
                }
            }
        }
        Ok(response)
    }
}

/// 解析库路径（与界面共用 core 的同一套规则，保证两边指向同一个文件）
pub fn resolve_db_path() -> PathBuf {
    rustrss_core::resolve_db_path()
}

/// 以 stdio 传输运行（由 MCP 客户端作为子进程拉起）
pub async fn serve_stdio(server: RustRssMcp) -> anyhow::Result<()> {
    let service = server.serve(stdio()).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;
    use rmcp::model::Extensions;

    /// 造一份「HTTP 请求的 parts」（中间件注入 scope 的载体）
    fn plain_parts() -> axum::http::request::Parts {
        HttpRequest::builder()
            .uri("/mcp")
            .body(())
            .expect("构造请求失败")
            .into_parts()
            .0
    }

    fn parts_with_scope(scope: Scope) -> axum::http::request::Parts {
        let mut parts = plain_parts();
        parts
            .extensions
            .insert(http::RequestScope { scope });
        parts
    }

    /// stdio（无 `Parts` 扩展）与 HTTP（有 `Parts`）的 scope 来源不同，但只能是这两条路。
    #[test]
    fn scope_comes_from_the_transport_not_from_the_session() {
        // ① stdio：没有 HTTP 请求扩展 → write（写能力另由开关把关）
        assert_eq!(scope_from_extensions(&Extensions::new()), Scope::Write);

        // ② HTTP：读中间件注入的当次 scope
        for (injected, expected) in [(Scope::Read, Scope::Read), (Scope::Write, Scope::Write)] {
            let mut ext = Extensions::new();
            ext.insert(parts_with_scope(injected));
            assert_eq!(scope_from_extensions(&ext), expected);
        }

        // ③ HTTP 但没注入（中间件被改坏/被绕过）：fail-closed 到 read，不得静默升权
        let mut ext = Extensions::new();
        ext.insert(plain_parts());
        assert_eq!(scope_from_extensions(&ext), Scope::Read);
    }

    /// 注册表与 `#[tool]` 路由不得漂移：每个对外暴露的工具都有 scope/dangerous 元数据。
    ///
    /// 漏登记的工具在过滤时会被丢掉（默认不可见），所以这个断言是「新增工具忘了登记」
    /// 的机械守卫；反向也查（注册表里不得有已下架的工具名）。
    #[test]
    fn every_exposed_tool_is_registered() {
        let exposed: Vec<String> = RustRssMcp::tool_router()
            .list_all()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        for name in &exposed {
            assert!(
                registry::spec(name).is_some(),
                "工具 {name} 已暴露但未在 registry::TOOL_SPECS 登记"
            );
        }
        for spec in registry::TOOL_SPECS {
            assert!(
                exposed.iter().any(|n| n == spec.name),
                "注册表里的 {} 已不存在（下架工具要同步删元数据）",
                spec.name
            );
        }
        assert_eq!(exposed.len(), registry::TOOL_SPECS.len());
    }

    /// 默认（未开写能力）下可见的只有 7 个只读工具，且读 token 会话看不到任何写工具。
    #[test]
    fn default_listing_is_read_only() {
        let server = RustRssMcp::new(Store::open_in_memory().unwrap());
        let off = server.switches();
        assert_eq!(off, Switches::default(), "新库默认全关");

        let read_scope = server.visible_tools(Scope::Read, &off);
        assert_eq!(read_scope.len(), 7);
        for tool in &read_scope {
            let spec = registry::spec(&tool.name).expect("列表里的工具必须已登记");
            assert_eq!(spec.scope, Scope::Read, "{} 不该出现在读会话里", tool.name);
        }
    }

    /// 桦工具（测试专用）走同一套注册表：写桦工具在读 scope 下不可见、
    /// 在写 scope + 全开下可见。
    #[test]
    fn test_tools_use_the_same_registry() {
        let server = RustRssMcp::new(Store::open_in_memory().unwrap())
            .with_test_tool("stub_write", Scope::Write, false, "桦写工具", |_| "{}".into());
        let off = server.switches();
        let names = |scope: Scope, sw: &Switches| {
            server
                .visible_tools(scope, sw)
                .iter()
                .map(|t| t.name.to_string())
                .collect::<Vec<_>>()
        };

        assert!(!names(Scope::Read, &off).contains(&"stub_write".to_string()));
        assert!(!names(Scope::Write, &off).contains(&"stub_write".to_string()), "开关没开也不列");

        let on = Switches { write_enabled: true, dangerous_enabled: false, write_token: true };
        assert!(names(Scope::Write, &on).contains(&"stub_write".to_string()));
        assert!(!names(Scope::Read, &on).contains(&"stub_write".to_string()));
    }

    /// 凭据 → scope：库里的写 token → write；读 token → read；静态凭据 → read；其余 None。
    ///
    /// 关键回归：**每次调用都重新读库**——轮换/销毁后旧值下一次调用就不再命中。
    #[test]
    fn credential_scope_is_recomputed_on_every_call() {
        let store = Store::open_in_memory().unwrap();
        config::set_token(&store, "read-token").unwrap();
        let old_write = config::generate_write_token();
        config::set_write_token(&store, &old_write).unwrap();
        let server = RustRssMcp::new(store);

        assert_eq!(server.scope_of_credential("read-token", None), Some(Scope::Read));
        assert_eq!(server.scope_of_credential(&old_write, None), Some(Scope::Write));
        assert_eq!(server.scope_of_credential("cli-token", Some("cli-token")), Some(Scope::Read));
        assert_eq!(server.scope_of_credential("cli-token", Some("other")), None);
        assert_eq!(server.scope_of_credential("", None), None);
        assert_eq!(server.scope_of_credential("nope", None), None);

        // 轮换：旧写 token 立刻不再是 write（也不再是任何有效凭据的一半）
        let new_write = config::generate_write_token();
        server.with_store(|s| config::set_write_token(s, &new_write).unwrap());
        assert_eq!(server.scope_of_credential(&old_write, None), None, "旧写 token 必须立刻失效");
        assert_eq!(server.scope_of_credential(&new_write, None), Some(Scope::Write));

        // 销毁：新值也失效
        server.with_store(|s| config::clear_write_token(s).unwrap());
        assert_eq!(server.scope_of_credential(&new_write, None), None, "销毁后写 token 必须立刻失效");
        assert_eq!(server.scope_of_credential("read-token", None), Some(Scope::Read), "读 token 不受影响");
    }

    /// 应用内托管：MCP 与界面共用同一个刷新 gate（agent 的 refresh 不得与界面刷叠加）
    #[test]
    fn refresh_gate_is_shared_with_the_host() {
        let gate = Arc::new(RefreshGate::new());
        let server = RustRssMcp::new(Store::open_in_memory().unwrap())
            .with_refresh_gate(Arc::clone(&gate));
        assert!(Arc::ptr_eq(&server.refresh_gate(), &gate));

        // 模拟界面已经拿着单 flight：MCP 侧必须被同一个标记挡住
        let held = gate.try_begin().expect("界面应拿到单 flight");
        assert!(server.refresh_gate().try_begin().is_err(), "MCP 不得叠加刷新");
        drop(held);
        assert!(server.refresh_gate().try_begin().is_ok());
    }
}
