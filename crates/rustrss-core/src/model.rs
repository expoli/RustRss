//! 领域模型：与具体的 feed 格式、与解析库都解耦。
//!
//! 解析层的产物一律是这里的类型；存储层、GUI、MCP 都只认这些类型。

use chrono::{DateTime, Utc};
use serde::Serialize;

/// 订阅源的条目身份来源。
///
/// 这个字段存在的意义是「可诊断」：条目去重出问题时，第一件要问的就是
/// 身份是从哪儿来的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdOrigin {
    /// 身份来自源提供的数据：真 guid，或解析库基于源内链接算出的哈希（两者都跨次稳定）
    SourceData,
    /// 源既没给 id 也没给链接 → 用内容指纹兜底（退化场景，否则会反复入库）
    ContentHash,
}

/// 解析后的订阅源（源信息 + 条目）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Feed {
    pub title: String,
    pub site_url: Option<String>,
    pub description: Option<String>,
    pub updated: Option<DateTime<Utc>>,
    pub language: Option<String>,
    pub entries: Vec<Entry>,
}

/// 解析后的条目
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    /// 稳定身份：同一篇文章在多次抓取中必须得到同一个值。
    ///
    /// 注意不要直接使用源里给的 id —— 解析库在源缺失 id 时会自动生成随机 UUID，
    /// 那种值跨次抓取不稳定，会把同一篇文章反复入库。
    pub stable_id: String,
    /// 身份来自哪里（用于诊断去重问题）
    pub id_origin: IdOrigin,
    /// 源里原始的 id（可能是真 guid，也可能是解析库生成的占位值）
    pub source_id: String,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub updated: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    /// 需要渲染的 HTML 正文（可能不存在）
    pub content_html: Option<String>,
    /// 纯文本正文（用于搜索与 MCP 输出，避免把 HTML 塞进上下文）
    pub content_text: Option<String>,
    /// 可选文章缩略图（HTTP(S)）；只从结构化媒体或文章图片提取。
    pub thumbnail_url: Option<String>,
    pub categories: Vec<String>,
}
