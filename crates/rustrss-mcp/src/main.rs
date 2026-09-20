// RustRss MCP 服务器（M0 连通性原型）
//
// 目的：验证「把订阅数据通过 MCP 暴露给外部 agent」这条链路能跑通，并确定工具集口径。
// 本原型的数据是内嵌样例（不接数据库），只验证协议与工具设计。
//
// 工具设计口径（见 PRD §6 风险 3）：
//   - 默认只回元数据 + 纯文本摘要，**不回正文 HTML 大字段**，避免单次响应撑爆 agent 上下文
//   - 正文必须通过 get_article 单独取
//   - 列表工具一律带 limit，避免无界返回
use rmcp::{
    handler::server::wrapper::Parameters, tool, tool_handler, tool_router, transport::stdio,
    ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct ListArticlesParams {
    /// 只看某个订阅源（feed id，不填=全部）
    feed_id: Option<String>,
    /// 只看未读（不填=全部）
    unread_only: Option<bool>,
    /// 最多返回多少条（默认 10，上限 50）
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetArticleParams {
    /// 条目 id（来自 list_articles）
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchParams {
    /// 关键词（在标题与正文里匹配）
    query: String,
    /// 最多返回多少条（默认 10，上限 50）
    limit: Option<u32>,
}

struct Fixture {
    feeds: Vec<(&'static str, &'static str, u32)>, // (id, 标题, 未读数)
    articles: Vec<(&'static str, &'static str, &'static str, &'static str, bool, &'static str)>,
    // (id, feed_id, 标题, 日期, 已读, 正文)
}

#[derive(Clone)]
struct RustRssMcp {
    data: std::sync::Arc<Fixture>,
}

impl RustRssMcp {
    fn new() -> Self {
        Self {
            data: std::sync::Arc::new(Fixture {
                feeds: vec![
                    ("feed-rust-blog", "Rust Blog", 2),
                    ("feed-lwn", "LWN.net", 1),
                ],
                articles: vec![
                    (
                        "art-1",
                        "feed-rust-blog",
                        "Announcing Rust 1.99",
                        "2026-09-18",
                        false,
                        "Rust 1.99 发布，包含异步闭包改进与更快的增量编译。本次发布还调整了 \
                         Cargo 的默认并行度。完整发布说明见官方博客。",
                    ),
                    (
                        "art-2",
                        "feed-rust-blog",
                        "Cargo 的依赖解析策略变更",
                        "2026-09-17",
                        false,
                        "Cargo 调整了依赖解析中特性统一（feature unification）的默认行为，\
                         以减少意外编译更多依赖的情况。本文说明变更动机与迁移建议。",
                    ),
                    (
                        "art-3",
                        "feed-lwn",
                        "Linux 7.0 的实时调度改进",
                        "2026-09-16",
                        true,
                        "7.0 合并窗口中的实时调度补丁减少了高优先级任务的最坏调度延迟，\
                         文中给出了 500 微秒负载下的实测数据。",
                    ),
                    (
                        "art-4",
                        "feed-lwn",
                        "WebKitGTK 2.54 换用 Skia 合成器",
                        "2026-09-16",
                        false,
                        "WebKitGTK 2.54 将合成器从 TextureMapper 换为基于 Skia 的实现，\
                         官方说明提到分数缩放下渲染质量的改善。",
                    ),
                ],
            }),
        }
    }

    fn article_meta(&self, row: &(&'static str, &'static str, &'static str, &'static str, bool, &'static str)) -> String {
        let (id, feed, title, date, read, body) = row;
        // 摘要：正文前 60 字（纯文本），字段名刻意短，减少 token 体积
        let summary: String = body.chars().take(60).collect();
        format!(
            r#"{{"id":"{id}","feed":"{feed}","title":"{title}","date":"{date}","read":{read},"summary":"{summary}…"}}"#
        )
    }
}

#[tool_router]
impl RustRssMcp {
    #[tool(description = "列出所有订阅源及其未读数")]
    fn list_feeds(&self) -> String {
        let items: Vec<String> = self
            .data
            .feeds
            .iter()
            .map(|(id, title, unread)| {
                format!(r#"{{"id":"{id}","title":"{title}","unread":{unread}}}"#)
            })
            .collect();
        format!(r#"{{"feeds":[{}]}}"#, items.join(","))
    }

    #[tool(
        description = "列出条目元数据（不含正文）。默认只回 10 条；正文请用 get_article 单独取。"
    )]
    fn list_articles(&self, Parameters(p): Parameters<ListArticlesParams>) -> String {
        let limit = p.limit.unwrap_or(10).min(50) as usize;
        let items: Vec<String> = self
            .data
            .articles
            .iter()
            .filter(|row| p.feed_id.as_deref().map_or(true, |f| row.1 == f))
            .filter(|row| {
                p.unread_only
                    .map_or(true, |only_unread| !only_unread || !row.4)
            })
            .take(limit)
            .map(|row| self.article_meta(row))
            .collect();
        format!(r#"{{"count":{},"articles":[{}]}}"#, items.len(), items.join(","))
    }

    #[tool(description = "取单篇文章的完整正文（含全文纯文本）")]
    fn get_article(&self, Parameters(p): Parameters<GetArticleParams>) -> String {
        match self.data.articles.iter().find(|row| row.0 == p.id) {
            Some((id, feed, title, date, read, body)) => format!(
                r#"{{"id":"{id}","feed":"{feed}","title":"{title}","date":"{date}","read":{read},"text":"{body}"}}"#
            ),
            None => format!(r#"{{"error":"未找到条目 {id}","hint":"先用 list_articles 取 id"}}"#, id = p.id),
        }
    }

    #[tool(description = "在标题与正文中搜索关键词")]
    fn search_articles(&self, Parameters(p): Parameters<SearchParams>) -> String {
        let limit = p.limit.unwrap_or(10).min(50) as usize;
        let needle = p.query.to_lowercase();
        let items: Vec<String> = self
            .data
            .articles
            .iter()
            .filter(|row| {
                row.2.to_lowercase().contains(&needle) || row.5.to_lowercase().contains(&needle)
            })
            .take(limit)
            .map(|row| self.article_meta(row))
            .collect();
        format!(
            r#"{{"query":"{}","count":{},"articles":[{}]}}"#,
            p.query,
            items.len(),
            items.join(",")
        )
    }
}

#[tool_handler(
    name = "rustrss",
    version = "0.0.0",
    instructions = "本地 RSS 订阅数据访问（M0 原型：数据为内嵌样例，非真实库）"
)]
impl ServerHandler for RustRssMcp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdio 传输：由 MCP 客户端（Claude Code / Cursor / Codex 等）作为子进程拉起
    let service = RustRssMcp::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
