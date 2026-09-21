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
use rustrss_core::{EntryQuery, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 50;
/// 列表里的摘要截断长度：够判断「要不要点进去看」，又不至于撑爆上下文
const SUMMARY_CHARS: usize = 140;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArticlesParams {
    /// 只看某个订阅源（feed id，来自 list_feeds）
    pub feed_id: Option<i64>,
    /// 只看未读
    #[serde(default)]
    pub unread_only: bool,
    /// 只看星标
    #[serde(default)]
    pub starred_only: bool,
    /// 最多返回多少条（默认 10，上限 50）
    pub limit: Option<u32>,
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
        let query = EntryQuery {
            feed_id: p.feed_id,
            unread_only: p.unread_only,
            starred_only: p.starred_only,
            limit: Some(clamp_limit(p.limit)),
            read_later_only: false
        };
        self.with_store(|store| match store.list_entries(&query) {
            Ok(rows) => {
                let items: Vec<ArticleMetaOut> = rows.iter().map(meta_out).collect();
                to_json(&serde_json::json!({
                    "count": items.len(),
                    "articles": items,
                    "hint": "正文请用 get_article(id) 单独取",
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
        description = "列出条目元数据（不含正文）。默认 10 条、上限 50；正文用 get_article 单独取。"
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

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| error_json(&format!("序列化失败: {e}")))
}

fn error_json(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}
