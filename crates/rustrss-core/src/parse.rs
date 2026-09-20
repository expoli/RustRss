//! 把 feed 原始字节解析成领域模型（RSS 0.x/1.0/2.0、Atom、JSON Feed）。

use crate::html::html_to_text;
use crate::model::{Entry, Feed, IdOrigin};
use feed_rs::model::{Entry as FsEntry, Feed as FsFeed, Link as FsLink, Text as FsText};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("feed 解析失败: {0}")]
    Feed(#[from] feed_rs::parser::ParseFeedError),
}

/// 解析一个订阅源的原始字节。
pub fn parse(bytes: &[u8]) -> Result<Feed, ParseError> {
    let parsed = feed_rs::parser::parse(bytes)?;
    Ok(convert_feed(parsed))
}

fn convert_feed(f: FsFeed) -> Feed {
    Feed {
        title: text_of(f.title.as_ref()).unwrap_or_else(|| "（无标题）".to_string()),
        site_url: pick_link(&f.links),
        description: text_of(f.description.as_ref()),
        updated: f.updated,
        language: f.language.clone(),
        entries: f.entries.into_iter().map(convert_entry).collect(),
    }
}

fn convert_entry(e: FsEntry) -> Entry {
    let primary_link = pick_link(&e.links);
    let title = text_of(e.title.as_ref()).unwrap_or_else(|| "（无标题）".to_string());

    // 两种来源都可能带 HTML：Atom 的 content、RSS 2.0 的 content:encoded，
    // 以及经常直接承载正文的 description（映射到 summary）。
    let (summary_html, summary_text) = split_text(text_of(e.summary.as_ref()).as_deref());
    let body = e
        .content
        .as_ref()
        .and_then(|c| c.body.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let (mut content_html, mut content_text) = split_text(body.as_deref());

    // 源没给正文时用摘要兜底：否则列表里有摘要、进阅读页却空白
    if content_text.is_none() {
        content_html = summary_html;
        content_text = summary_text.clone();
    }

    let (stable_id, id_origin) = identity(&e, primary_link.as_deref(), &title, summary_text.as_deref());

    Entry {
        stable_id,
        id_origin,
        source_id: e.id.clone(),
        title,
        url: primary_link,
        author: e.authors.first().and_then(|p| {
            // feed-rs 2.x 里 name 是 String（可能为空串），email 是 Option
            let name = p.name.trim();
            if name.is_empty() {
                p.email.clone().filter(|m| !m.trim().is_empty())
            } else {
                Some(name.to_string())
            }
        }),
        published: e.published.or(e.updated),
        updated: e.updated,
        summary: summary_text,
        content_html,
        content_text,
        categories: e.categories.iter().map(|c| c.term.clone()).collect(),
    }
}

/// 判定条目的稳定身份。
///
/// 解析库在源缺失 id 时会按其规则生成：先试基于首个链接的哈希，再退化为**随机 UUID**。
/// 前者跨次抓取稳定（可以直接用），后者不稳定（会把同一篇文章反复入库），
/// 因此只在「既无链接、id 又呈 UUID 形态」这个真退化场景改用内容指纹。
///
/// 这里故意不给「有链接但无 guid」单独开分支：那种情况解析库给的哈希已经稳定，
/// 而「真 guid vs 生成的哈希」在外部无法区分，硬分类只会引入误导。
fn identity(
    e: &FsEntry,
    primary_link: Option<&str>,
    title: &str,
    summary: Option<&str>,
) -> (String, IdOrigin) {
    let raw_id = e.id.trim();
    let generated_uuid = primary_link.is_none() && looks_like_uuid(raw_id);

    if !raw_id.is_empty() && !generated_uuid {
        return (format!("sid:{raw_id}"), IdOrigin::SourceData);
    }

    let mut hasher = Sha256::new();
    hasher.update(title.trim().as_bytes());
    hasher.update(b"\x1f");
    hasher.update(
        e.published
            .or(e.updated)
            .map(|t| t.to_rfc3339())
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.update(b"\x1f");
    hasher.update(summary.unwrap_or_default().trim().as_bytes());
    (format!("h:{}", hex(&hasher.finalize())), IdOrigin::ContentHash)
}

/// 从一组 link 里挑出「文章自身地址」：优先 rel=alternate 或无 rel，避开 self/enclosure/replies。
fn pick_link(links: &[FsLink]) -> Option<String> {
    let usable = |l: &FsLink| {
        let rel = l.rel.as_deref().unwrap_or("alternate");
        rel == "alternate"
    };
    links
        .iter()
        .find(|l| usable(l))
        .map(|l| l.href.trim().to_string())
        .filter(|h| !h.is_empty())
}

fn text_of(t: Option<&FsText>) -> Option<String> {
    t.map(|t| t.content.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 把一段可能含 HTML 的文本拆成「HTML 形态」与「纯文本形态」。
/// 判定依据是内容而不是 content_type——真实源里的 mime 经常与内容不符。
fn split_text(raw: Option<&str>) -> (Option<String>, Option<String>) {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if looks_like_html(s) => (Some(s.to_string()), Some(html_to_text(s))),
        Some(s) => (None, Some(s.to_string())),
        None => (None, None),
    }
}

/// 粗判是否 HTML。刻意不用 content_type：mime 判定在真实源里经常与内容不符，
/// 而这里只关心「要不要按标签清洗」。
fn looks_like_html(s: &str) -> bool {
    s.contains('<') && s.contains('>')
}

/// UUID 形态判定（不做版本号校验，只要形状像就够了——这里只用来避免误用自动生成的随机值）
fn looks_like_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for (i, c) in b.iter().enumerate() {
        let is_dash_pos = matches!(i, 8 | 13 | 18 | 23);
        if is_dash_pos {
            if *c != b'-' {
                return false;
            }
        } else if !c.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
