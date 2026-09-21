//! RSSHub 订阅地址归一化。
//!
//! 三种输入形态：
//! 1. `rsshub://path`（含 `rsshub:///path` 三斜杠）——OPML 常见的自定义 scheme，
//!    本身不可抓取，必须实例化为 `https://{base}/{path}`；
//! 2. `https://rsshub.app/path`——官方 https 形态，自建/镜像用户应指向自己的实例；
//! 3. 其它 URL——原样保留。
//!
//! 归一化发生在 feed URL 落库（`Store::add_feed`）前，因此库内 URL 永远等于
//! 实际抓取地址，重试、tooltip、失败标记自然一致。

pub const DEFAULT_BASE: &str = "https://rsshub.app";

/// 设置键：RSSHub 实例地址（空 = 官方默认）。
pub const MIRROR_KEY: &str = "rsshub.mirror";

/// 官方实例域名：仅当订阅 URL 指向它们时才做 base 替换。
pub const OFFICIAL_HOSTS: [&str; 2] = ["rsshub.app", "www.rsshub.app"];

/// 把订阅地址归一化为实际抓取地址。
///
/// `base`：RSSHub 实例地址（设置键 `rsshub.mirror`）；空或非法时回退官方实例。
pub fn normalize_rsshub_url(url: &str, base: &str) -> String {
    let url = url.trim();
    let base = clean_base(base);

    // 形态 1：自定义 scheme（rsshub://path / rsshub:///path）
    if let Some(path) = url
        .strip_prefix("rsshub://")
        .map(|p| p.trim_start_matches('/'))
        .filter(|p| !p.is_empty())
    {
        return format!("{base}/{path}");
    }

    // 形态 2：官方 https 形态 → 替换为实例 base（base 即官方时原样）
    if let Ok(parsed) = url::Url::parse(url) {
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        if OFFICIAL_HOSTS.contains(&host.as_str()) && base != DEFAULT_BASE {
            // path+query：Url::path() 从 domain 后开始
            let mut out = format!("{}{}", base, parsed.path());
            if let Some(q) = parsed.query() {
                out.push('?');
                out.push_str(q);
            }
            return out;
        }
    }

    // 形态 3：其它 URL 原样
    url.to_string()
}

/// 校验并清理 base：去尾斜杠；空值回退官方实例；非 http(s) 视为非法返回 None。
pub fn clean_base(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return DEFAULT_BASE.to_string();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        DEFAULT_BASE.to_string()
    }
}

/// base 是否为用户自定义（非官方默认）。
pub fn is_custom_base(base: &str) -> bool {
    let cleaned = clean_base(base);
    cleaned != DEFAULT_BASE
}


/// 探测一个 URL 的可达性：拿到响应（无论内容能否解析）即视为可达；
/// 网络层失败（超时/DNS）返回 Err。用于「测试连接」。
pub async fn probe_url(
    fetcher: &crate::fetch::Fetcher,
    url: &str,
) -> Result<bool, String> {
    use crate::fetch::{CacheHeaders, FetchResult};
    match fetcher.fetch(url, CacheHeaders::default()).await {
        FetchResult::Fetched { .. } | FetchResult::NotModified { .. } => Ok(true),
        FetchResult::Failed { status: Some(_), error } => Ok(false),
        FetchResult::Failed { status: None, error } => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_scheme_is_instantiated() {
        assert_eq!(
            normalize_rsshub_url("rsshub://telegram/channel/xxx", ""),
            "https://rsshub.app/telegram/channel/xxx"
        );
        // 三斜杠（空 host）
        assert_eq!(
            normalize_rsshub_url("rsshub:///gofans", ""),
            "https://rsshub.app/gofans"
        );
        // 自建实例
        assert_eq!(
            normalize_rsshub_url("rsshub://v2ex/topics/hot", "https://rsshub.example.com"),
            "https://rsshub.example.com/v2ex/topics/hot"
        );
    }

    #[test]
    fn official_https_is_rebased_only_for_custom_base() {
        // base 为官方（含空）时原样
        assert_eq!(
            normalize_rsshub_url("https://rsshub.app/github/trending/weekly/any", ""),
            "https://rsshub.app/github/trending/weekly/any"
        );
        // 自建实例：替换 base，path+query 原样
        assert_eq!(
            normalize_rsshub_url(
                "https://rsshub.app/36kr/newsflashes?limit=10",
                "https://rsshub.example.com/"
            ),
            "https://rsshub.example.com/36kr/newsflashes?limit=10"
        );
    }

    #[test]
    fn other_urls_pass_through() {
        assert_eq!(
            normalize_rsshub_url("https://example.com/feed.xml", "https://mirror.example"),
            "https://example.com/feed.xml"
        );
        assert_eq!(
            normalize_rsshub_url("https://www.rsshub.app/x", DEFAULT_BASE),
            "https://www.rsshub.app/x"
        );
    }

    #[test]
    fn clean_base_normalizes() {
        assert_eq!(clean_base(""), DEFAULT_BASE);
        assert_eq!(clean_base("  "), DEFAULT_BASE);
        assert_eq!(clean_base("https://rsshub.example.com/"), "https://rsshub.example.com");
        assert_eq!(clean_base("http://127.0.0.1:1200"), "http://127.0.0.1:1200");
        assert_eq!(clean_base("rsshub.example.com"), DEFAULT_BASE, "非 http(s) 回退官方");
    }
}
