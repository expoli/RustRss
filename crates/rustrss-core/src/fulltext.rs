//! 全文提取：把「摘要型」条目指向的原文页转成可渲染正文。
//!
//! 依赖选型（2026-09-21 实测，理由见任务报告）：
//! - 选定 `dom_smoothie`（readability.js 的 Rust 移植，Mozilla 算法同源）；
//! - 近 12 个月持续发布（0.13→0.18，评估当日刚发 0.18.2），活跃维护；
//! - 传递依赖可控：`dom_query`（html5ever 系，本仓库 tauri 侧已有同名依赖）与若干叶子
//!   crate（flagset / gjson / html-escape / phf / tendril）；无网络、无 TLS、无系统库，
//!   `cargo audit` 对新依赖零新增公告；
//! - 反例：`readability` 0.3.0（2023-12 后停更）与 `readability-rs` 0.5.0
//!   （2024-12 后停更）都停在 html5ever 0.26 + markup5ever_rcdom 0.2，
//!   会在依赖树里多留一套老解析器。
//!
//! 分工：这里只负责「页面 → 正文」，**不做安全清洗**——提取结果进的是与 feed 正文
//! 同一条渲染管线（见 `html.rs` 顶部的同一句说明）。

use dom_smoothie::{Readability, ReadabilityError};

use crate::html::html_to_text;

/// 单次抓取的响应体上限：超限直接拒绝，绝不进解析器。
///
/// 网页正文提取是 CPU 密集的树操作，页面体积是唯一可控的输入端，所以闸门放在
/// 「字节 → 文本」这一步（`extract_bytes`），调用方拿到的任何路径都绕不过它。
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// 「摘要型」的正文长度阈值（纯文本字数）。
///
/// 阈值取 500：摘要型 feed 的 teaser 通常 < 300 字，真实正文通常 > 1000 字。
/// 有意偏向「宁可多显示一次按钮」——误报的代价是用户点一下、拿到与原文相近的内容，
/// 漏判的代价是功能对该条目不可达（PRD 的验收点是「摘要型条目显示按钮」）。
pub const SUMMARY_TEXT_MIN_CHARS: usize = 500;

/// 提取结果：正文 HTML + 由它派生的纯文本（检索 token 与 MCP 输出都用后者）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub content_html: String,
    pub content_text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FulltextError {
    #[error("页面超过 {max} 字节上限（实际 {size} 字节），已拒绝抓取")]
    TooLarge { size: usize, max: usize },
    #[error("目标不是 HTML 页面，已跳过全文提取")]
    NotHtml,
    #[error("原文地址无效（需要绝对的 http/https 地址）")]
    BadUrl,
    #[error("页面里没有可提取的正文")]
    NoContent,
    #[error("正文提取失败: {0}")]
    Extract(String),
}

/// 响应体 → 可写回正文：大小闸门 → 宽松解码 → 提取。
///
/// 编码按 UTF-8 宽松解码（个别字节用替换符顶上）：正文提取是「尽力而为」的降级路径，
/// 非 UTF-8 页面里的少量替换符不影响可读性，而按字符集嗅探再转码会把这一层拖成
/// 一个独立的编码子系统。
pub fn extract_bytes(body: &[u8], url: &str) -> Result<Extracted, FulltextError> {
    if body.len() > MAX_BYTES {
        return Err(FulltextError::TooLarge {
            size: body.len(),
            max: MAX_BYTES,
        });
    }
    extract(&String::from_utf8_lossy(body), url)
}

/// HTML 文本 → 可写回正文。`url` 是原文地址，既是相对链接（href/src）的解析基准，
/// 也参与 `Readability` 自身的候选评分。
pub fn extract(html: &str, url: &str) -> Result<Extracted, FulltextError> {
    let url = absolute_http_url(url)?;
    if !looks_like_html(html) {
        return Err(FulltextError::NotHtml);
    }

    let mut readability = Readability::new(html, Some(&url), None)
        .map_err(|e| FulltextError::Extract(e.to_string()))?;
    let article = readability.parse().map_err(|e| match e {
        // 页面能解析但没有候选正文（空 body、纯脚本壳、登录墙）：不是「出错」，
        // 而是「没得抓」，给用户一句能懂的话即可。
        ReadabilityError::GrabFailed => FulltextError::NoContent,
        other => FulltextError::Extract(other.to_string()),
    })?;

    let content_html = article.content.trim().to_string();
    // 纯文本走 html.rs 同一个转换：检索 token、MCP 输出、阅读页降级展示三处保持一致。
    let content_text = html_to_text(&content_html);
    if content_text.trim().is_empty() {
        return Err(FulltextError::NoContent);
    }
    Ok(Extracted {
        content_html,
        content_text,
    })
}

/// 该条目是否还值得抓全文（前端「获取全文」按钮的显示条件）。
///
/// 三个条件缺一不可：没抓过（抓过就以库里的正文为准）、有可抓的原文地址、
/// 现有正文「明显偏短」。
pub fn is_summary_entry(
    url: Option<&str>,
    content_html: Option<&str>,
    content_text: Option<&str>,
    fulltext_fetched: bool,
) -> bool {
    if fulltext_fetched {
        return false;
    }
    if url.map(str::trim).unwrap_or_default().is_empty() {
        return false;
    }
    readable_text_len(content_html, content_text) < SUMMARY_TEXT_MIN_CHARS
}

/// 现有正文的纯文本字数：优先信已存的 `content_text`（解析层与它是同一来源派生的），
/// 缺失时才从 `content_html` 现算。
fn readable_text_len(content_html: Option<&str>, content_text: Option<&str>) -> usize {
    match content_text.map(str::trim).filter(|t| !t.is_empty()) {
        Some(text) => text.chars().count(),
        None => content_html
            .map(|html| html_to_text(html).chars().count())
            .unwrap_or(0),
    }
}

/// 原文地址必须是绝对的 http/https：`Readability` 要求绝对 URL，而 feed 里的链接
/// 偶尔是相对的（本仓库 `parse.rs` 原样保存 href）——那种情况给一句明确的错误，
/// 而不是让提取器在相对链接上做无意义的解析。
fn absolute_http_url(url: &str) -> Result<String, FulltextError> {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url.to_string())
    } else {
        Err(FulltextError::BadUrl)
    }
}

/// 粗判「这是不是一个 HTML 文档」。
///
/// 只看文档开头一小段里的结构标记：JSON / 纯文本 / PDF / RSS(XML) 都在这里被挡下，
/// 免得把非 HTML 的东西喂进提取器、最后给用户一句莫名其妙的解析错误。
fn looks_like_html(html: &str) -> bool {
    const MARKERS: [&str; 8] = [
        "<!doctype", "<html", "<head", "<body", "<article", "<main", "<div", "<p",
    ];
    let head: String = html
        .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .chars()
        .take(4096)
        .collect();
    if !head.starts_with('<') {
        return false;
    }
    let head = head.to_ascii_lowercase();
    MARKERS.iter().any(|m| head.contains(m))
}
