//! Feed 自动发现：先看输入 URL 本身是不是 feed，不是再从 HTML 里找
//! `<link rel="alternate">` 指向的 feed。
//!
//! `rsshub://path` 这类自定义 scheme **不联网发现**：它不可直连抓取，存储时保持
//! scheme 形态（`canonical_scheme_url` 归一），首次抓取由 `Store::feed_endpoint`
//! 解析到当前镜像（见 `rsshub.rs` 的两套语义）。
//!
//! 判定依据是**内容**而不是 Content-Type：真实站点的 mime 经常与内容不符
//! （同 `parse.rs` 的取舍），而且抓取层本来就只回传字节。
//! HTTP 细节（UA、超时、重定向、压缩）全部复用 `fetch::Fetcher`，不另起一套。

use serde::Serialize;
use url::Url;

use crate::fetch::{CacheHeaders, FetchResult, Fetcher};
use crate::parse::parse;

/// 标准 feed MIME：`type` 属性里写这三个之一即为明确候选
const FEED_MIMES: [&str; 3] = [
    "application/rss+xml",
    "application/atom+xml",
    "application/feed+json",
];

/// `type` 缺失或写错时的兜底后缀（按 href 宽松匹配）
const FEED_SUFFIXES: [&str; 4] = [".xml", ".rss", ".atom", ".json"];

/// 主 feed 是怎么定下来的（既是日志内容，也是调用方可展示的依据）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DiscoveryVia {
    /// 输入地址直接返回 feed 内容，无需发现
    Direct,
    /// 页面 `<link rel="alternate">` 的 type 是标准 feed MIME
    LinkType,
    /// type 缺失/不标准，按 href 后缀兜底
    LinkSuffix,
}

/// 发现结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Discovery {
    /// 采用的 feed 绝对地址（输入地址发生重定向时是重定向后的地址）
    pub feed_url: String,
    /// 主 feed 的判定依据
    pub via: DiscoveryVia,
    /// 页面里的其余候选（绝对地址）——v1 只用它们记日志
    pub alternatives: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("抓取 {url} 失败：{error}")]
    Fetch { url: String, error: String },
    #[error("在 {url} 未发现 feed（no feed link found）：{detail}")]
    NoFeedLink { url: String, detail: String },
}

/// 发现 feed。输入既可以是站点首页，也可以是 feed 地址本身。
///
/// 只发一次 GET：既判断「这个地址本身是不是 feed」，也拿到页面 head 里的候选。
/// 不做路径猜测（`/feed`、`/rss.xml` 探测会对不存在的路径发额外请求）。
///
/// `rsshub://path`（含三斜杠/大写）直接返回归一后的 scheme 形态，不发请求——
/// 这个 scheme 不可抓取，走网络只会得到「URL scheme 不支持」。归一化与
/// `Store::add_feed` 同源，所以「发现出来的地址」与「落库地址」永远一致。
pub async fn discover(fetcher: &Fetcher, url: &str) -> Result<Discovery, DiscoverError> {
    if crate::rsshub::is_scheme_url(url) {
        return Ok(Discovery {
            feed_url: crate::rsshub::canonical_scheme_url(url),
            via: DiscoveryVia::Direct,
            alternatives: Vec::new(),
        });
    }
    let (body, page_url) = match fetcher.fetch(url, CacheHeaders::default()).await {
        FetchResult::Fetched {
            body, final_url, ..
        } => (body, final_url),
        FetchResult::NotModified { .. } => {
            return Err(DiscoverError::Fetch {
                url: url.to_string(),
                error: "服务端返回 304（发现请求不带条件头）".to_string(),
            })
        }
        FetchResult::Failed { error, .. } => {
            return Err(DiscoverError::Fetch {
                url: url.to_string(),
                error,
            })
        }
    };

    // 能按 feed 解析出来，说明这个地址本身就能订阅：直接用它，不做二次请求
    if parse(&body).is_ok() {
        return Ok(Discovery {
            feed_url: page_url,
            via: DiscoveryVia::Direct,
            alternatives: Vec::new(),
        });
    }

    let base = Url::parse(&page_url).map_err(|e| DiscoverError::Fetch {
        url: page_url.clone(),
        error: format!("页面地址无法解析: {e}"),
    })?;
    let html = String::from_utf8_lossy(&body);
    let (typed, suffixed) = feed_candidates(&html, &base);

    // 标准 type 优先，后缀兜底的只在没有标准候选时上场（这就是「兜底」的语义）
    let (feed_url, via, alternatives) = if let Some((first, rest)) = typed.split_first() {
        let mut alternatives = rest.to_vec();
        alternatives.extend(suffixed);
        (first.clone(), DiscoveryVia::LinkType, alternatives)
    } else if let Some((first, rest)) = suffixed.split_first() {
        (first.clone(), DiscoveryVia::LinkSuffix, rest.to_vec())
    } else {
        return Err(DiscoverError::NoFeedLink {
            url: page_url,
            detail: if looks_like_html(&html) {
                "页面里没有 rel=\"alternate\" 的 feed 链接".to_string()
            } else {
                "内容既不能按 feed 解析，也不是 HTML 页面".to_string()
            },
        });
    };

    if !alternatives.is_empty() {
        log::warn!(
            "[rustrss] {page_url} 有多个候选 feed，采用第一个 {feed_url}，其余：{}",
            alternatives.join(", ")
        );
    }
    if via == DiscoveryVia::LinkSuffix {
        log::warn!("[rustrss] {page_url} 的 feed 链接缺标准 type，按 href 后缀兜底：{feed_url}");
    }

    Ok(Discovery {
        feed_url,
        via,
        alternatives,
    })
}

/// 扫页面里的 `<link rel="alternate">`，分成「标准 type」与「后缀兜底」两组，
/// 组内保持文档顺序。
fn feed_candidates(html: &str, base: &Url) -> (Vec<String>, Vec<String>) {
    // 只做 ASCII 小写化：字节长度不变，因此下标可以在两份字符串上共用
    let lower = html.to_ascii_lowercase();
    // link 标签正常只出现在 head：有 </head> 就只扫到它为止，免得把正文里的
    // 模板片段也算成候选；没有 </head> 的半截 HTML 就整篇扫。
    let scope = lower.find("</head>").unwrap_or(lower.len());
    let (lower, html) = (&lower[..scope], &html[..scope]);

    let mut typed = Vec::new();
    let mut suffixed = Vec::new();
    let mut i = 0;

    while let Some(pos) = lower[i..].find("<link") {
        let start = i + pos + "<link".len();
        // 标签名要在这里结束，避免把 <linkmap 这类当成 link 标签
        if !matches!(
            lower.as_bytes().get(start).copied(),
            None | Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>')
        ) {
            i = start;
            continue;
        }
        let Some(close) = lower[start..].find('>') else {
            break; // 未闭合，收尾
        };
        let tag = &html[start..start + close];
        i = start + close + 1;

        if !has_alternate_rel(tag) {
            continue;
        }
        let Some(href) = attr(tag, "href")
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
        else {
            continue;
        };
        let Ok(abs) = base.join(&href) else {
            continue; // 解析不了的 href 直接跳过，不影响其它候选
        };
        let abs = abs.to_string();

        if attr(tag, "type")
            .map(|t| mime_of(&t))
            .is_some_and(|t| FEED_MIMES.contains(&t.as_str()))
        {
            typed.push(abs);
        } else if has_feed_suffix(&abs) {
            suffixed.push(abs);
        }
    }

    (typed, suffixed)
}

/// `rel` 是空格分隔的 token 列表，含 `alternate` 即可
fn has_alternate_rel(tag: &str) -> bool {
    attr(tag, "rel")
        .map(|v| {
            v.split_whitespace()
                .any(|t| t.eq_ignore_ascii_case("alternate"))
        })
        .unwrap_or(false)
}

/// `application/rss+xml; charset=utf-8` 这类带参数的也认，只比分号前的部分
fn mime_of(raw: &str) -> String {
    raw.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// 后缀兜底判定：只看路径部分（`/feed.xml?utm=1`、`/feed.xml#top` 也算）
fn has_feed_suffix(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    FEED_SUFFIXES.iter().any(|s| path.ends_with(s))
}

/// 取标签里的属性值，支持双引号 / 单引号 / 无引号三种写法（HTML 都合法）。
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut i = 0;

    while let Some(pos) = lower[i..].find(name) {
        let start = i + pos;
        let end = start + name.len();
        i = end;

        // 属性名前必须是分隔符，否则 `data-href` 里的 href 也会被当成 href
        if start > 0 && !lower.as_bytes()[start - 1].is_ascii_whitespace() {
            continue;
        }
        let rest = tag[end..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let body = &rest[1..];
                &body[..body.find(q).unwrap_or(body.len())]
            }
            _ => &rest[..rest.find(char::is_whitespace).unwrap_or(rest.len())],
        };

        // 链接里的 `&` 常写成实体（如 WordPress 的 `?feed=rss2&amp;cat=3`），
        // 另有部分站点用数字实体（如 `&#x3D;` 代 `=`、`&#38;` 代 `&`）。
        return Some(decode_entities(value));
    }

    None
}

/// 解码属性值里的常见 HTML 实体：命名实体（&amp;/&lt;/&gt;/&quot;/&apos;/&nbsp;）
/// + 十进制/十六进制数字实体。未知实体原样保留（宽容处理，不做严格校验）。
fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        // 找实体终点：`;`（规范）或最多 10 字符内的非实体字符边界
        let semicolon = after.find(';').filter(|&i| i <= 10);
        let (decoded, consumed) = match semicolon {
            Some(i) => {
                let name = &after[1..i];
                let decoded: String = match name {
                    "amp" => "&".to_string(),
                    "lt" => "<".to_string(),
                    "gt" => ">".to_string(),
                    "quot" => "\"".to_string(),
                    "apos" => "'".to_string(),
                    "nbsp" => "\u{a0}".to_string(),
                    _ if let Some(hex) = name
                        .strip_prefix("#x")
                        .or_else(|| name.strip_prefix("#X"))
                        .and_then(|h| u32::from_str_radix(h, 16).ok()) =>
                    {
                        char::from_u32(hex).map(String::from).unwrap_or_default()
                    }
                    _ if let Some(dec) =
                        name.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()) =>
                    {
                        char::from_u32(dec).map(String::from).unwrap_or_default()
                    }
                    _ => after[..i + 1].to_string(), // 未知实体：原样保留（含 & 和 ;）
                };
                (decoded, i + 1)
            }
            None => ("&".to_string(), 1), // 孤立 &：原样保留
        };
        out.push_str(&decoded);
        rest = &rest[pos + consumed..];
    }
    out.push_str(rest);
    out
}

/// 粗判形状，只为把错误写得更贴切（不做安全清洗，同 `html.rs` 的取舍）
fn looks_like_html(s: &str) -> bool {
    s.contains('<') && s.contains('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_entities_covers_named_numeric_and_unknown() {
        assert_eq!(decode_entities("a&amp;b"), "a&b");
        assert_eq!(decode_entities("&#x3D;"), "=");
        assert_eq!(decode_entities("&#61;"), "=");
        assert_eq!(decode_entities("a&lt;b&quot;c"), "a<b\"c");
        assert_eq!(decode_entities("&unknown;"), "&unknown;"); // 未知实体原样
        assert_eq!(decode_entities("a & b"), "a & b"); // 孤立 & 原样
        assert_eq!(decode_entities("plain"), "plain");
        assert_eq!(decode_entities("&#xzz;"), "&#xzz;"); // 非法数字原样
    }

    fn base() -> Url {
        Url::parse("https://example.com/blog/index.html").expect("测试基址应能解析")
    }

    #[test]
    fn attr_supports_quoted_and_unquoted_values() {
        let tag = " rel=\"alternate\" type='application/rss+xml' href=/feed.xml  data-href=\"/x\"";
        assert_eq!(attr(tag, "rel").as_deref(), Some("alternate"));
        assert_eq!(attr(tag, "type").as_deref(), Some("application/rss+xml"));
        assert_eq!(attr(tag, "href").as_deref(), Some("/feed.xml"));
        // 前缀相同的属性名不能互相串台
        assert_eq!(attr(tag, "data-href").as_deref(), Some("/x"));
    }

    #[test]
    fn candidates_prefer_standard_type_and_resolve_relative_hrefs() {
        let html = r#"<html><head>
<link rel="alternate" type="text/html" href="sitemap.xml">
<link rel="alternate" type="application/rss+xml; charset=utf-8" href="/feed.xml">
<link rel="alternate" type="application/atom+xml" href="atom.xml">
<link rel="stylesheet" href="/style.css">
</head><body><link rel="alternate" type="application/rss+xml" href="/in-body.xml"></body></html>"#;
        let (typed, suffixed) = feed_candidates(html, &base());
        assert_eq!(
            typed,
            vec![
                "https://example.com/feed.xml".to_string(),
                "https://example.com/blog/atom.xml".to_string(),
            ]
        );
        // 正文里的同名标签不算（只扫 head），str.stylesheet 也不算
        assert_eq!(
            suffixed,
            vec!["https://example.com/blog/sitemap.xml".to_string()]
        );
    }

    #[test]
    fn suffix_fallback_matches_path_ignoring_query_and_fragment() {
        assert!(has_feed_suffix("https://example.com/feed.xml?utm=1"));
        assert!(has_feed_suffix("https://example.com/Feed.RSS#top"));
        assert!(!has_feed_suffix("https://example.com/feed.xmls"));
        assert!(!has_feed_suffix("https://example.com/xml"));
    }

    #[test]
    fn amp_entities_in_href_are_decoded() {
        let html = r#"<html><head><link rel="alternate" type="application/rss+xml" href="/feed?cat=3&amp;fmt=rss"></head></html>"#;
        let (typed, _) = feed_candidates(html, &base());
        assert_eq!(
            typed,
            vec!["https://example.com/feed?cat=3&fmt=rss".to_string()]
        );
    }
}
