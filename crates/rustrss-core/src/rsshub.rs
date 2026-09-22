//! RSSHub 订阅地址的两套语义：**存储形态**与**抓取形态**。
//!
//! 存储形态（`canonical_scheme_url`，`Store::add_feed`/OPML 导入收口）：
//! 库内 URL 是抽象身份，**不绑定任何实例**——
//! 1. `rsshub://path`（含 `rsshub:///path` 三斜杠、大写 scheme）→ `rsshub://path`；
//! 2. `https://rsshub.app/path`（含 `www.`）→ `rsshub://path`（官方域与 scheme 同一身份）；
//! 3. 其它 URL → 原样保留。
//!
//! 抓取形态（`resolve_fetch_url`，唯一调用点 `Store::feed_endpoint`）：
//! 1. `rsshub://path` → `{base}/path`；
//! 2. 存量官方域行 `https://rsshub.app/path` → `{base}/path`（老库存量兼容；
//!    scheme 化迁移之外的库也能被镜像设置接管）；
//! 3. 其它 URL → 原样直抓。
//!
//! 这样换镜像时**零迁移**：库里存 scheme，抓取时解析到当前实例，下一次刷新即生效。
//! 存量已实例化到自建镜像的行不在官方域、也不带 scheme，保持原样直抓旧实例
//! （与拆分前的行为一致，不劣化）。

pub const DEFAULT_BASE: &str = "https://rsshub.app";

/// 设置键：RSSHub 实例地址（空 = 官方默认）。
pub const MIRROR_KEY: &str = "rsshub.mirror";

/// 官方实例域名：仅当订阅 URL 指向它们时才做 base 替换。
pub const OFFICIAL_HOSTS: [&str; 2] = ["rsshub.app", "www.rsshub.app"];

/// 把订阅地址归一化为**存储形态**（add_feed / OPML 导入用）。
///
/// 不再实例化到镜像：scheme 保留（三斜杠归一为双斜杠）、官方域转 scheme、其它原样。
/// 因此去重键就是「同一条路由的两种写法」的共同形态（`rsshub://path`）。
pub fn canonical_scheme_url(url: &str) -> String {
    let url = url.trim();
    if let Some(path) = scheme_path(url) {
        return format!("rsshub://{path}");
    }
    if let Some((path, query)) = official_path_query(url) {
        return format!("rsshub://{path}{query}");
    }
    url.to_string()
}

/// 把库内地址解析为**实际抓取地址**（`Store::feed_endpoint` 用）。
///
/// `base`：RSSHub 实例地址（设置键 `rsshub.mirror`）；空或非法时回退官方实例。
/// 同时处理 scheme 行与存量官方域行——两者都是「指向 RSSHub 官方路由」的形态，
/// 都必须跟随镜像设置，否则换镜像后老存量行仍抓旧实例。
pub fn resolve_fetch_url(url: &str, base: &str) -> String {
    let url = url.trim();
    let base = clean_base(base);
    if let Some(path) = scheme_path(url) {
        return format!("{base}/{path}");
    }
    if let Some((path, query)) = official_path_query(url) {
        return format!("{base}/{path}{query}");
    }
    url.to_string()
}

/// 是否为自定义 scheme 订阅（`rsshub://path`，大小写不敏感，含三斜杠）。
///
/// path 为空的 `rsshub://` 不是有效订阅（无路由可抓），返回 false——调用方按普通
/// URL 处理，错误信息更直白。
pub fn is_scheme_url(url: &str) -> bool {
    scheme_path(url.trim()).is_some()
}

/// 取 `rsshub://{path}` 的 path；不是 scheme 或 path 为空时返回 None。
///
/// 用 `get(..9)` 而非字节切片：多字节字符开头（如中文地址）落在前 9 字节内时，
/// 字节切片会 panic；`get` 在非字符边界返回 None（回归：非 ASCII 开头的地址带进来就崩）。
fn scheme_path(url: &str) -> Option<&str> {
    if url.len() < 9
        || !url
            .get(..9)
            .is_some_and(|p| p.eq_ignore_ascii_case("rsshub://"))
    {
        return None;
    }
    // 前缀是 ASCII（eq_ignore_ascii_case 只匹配 ASCII 大小写），第 9 字节必是边界
    let path = url[9..].trim_start_matches('/');
    (!path.is_empty()).then_some(path)
}

/// 取官方域 https URL 的 `(path, "?query")`；非官方域或空 path 返回 None。
fn official_path_query(url: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if !OFFICIAL_HOSTS.contains(&host.as_str()) {
        return None;
    }
    // Url::path() 从 domain 后开始，恒以 '/' 打头
    let path = parsed.path().trim_start_matches('/');
    if path.is_empty() {
        return None;
    }
    let query = parsed.query().map(|q| format!("?{q}")).unwrap_or_default();
    Some((path.to_string(), query))
}

/// 校验并清理 base：去尾斜杠；空值回退官方实例；非 http(s) 视为非法返回官方。
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
        FetchResult::Failed { status: Some(_), error: _ } => Ok(false),
        FetchResult::Failed { status: None, error } => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIRROR: &str = "https://rsshub.example.com";

    // ------------------------------------------------ 存储形态

    #[test]
    fn canonical_keeps_scheme_and_normalizes_slashes() {
        assert_eq!(canonical_scheme_url("rsshub://telegram/channel/xxx"), "rsshub://telegram/channel/xxx");
        // 三斜杠（空 host）归一为双斜杠
        assert_eq!(canonical_scheme_url("rsshub:///gofans"), "rsshub://gofans");
        // 大写 scheme 归一为小写
        assert_eq!(canonical_scheme_url("RSSHUB://v2ex/topics/hot"), "rsshub://v2ex/topics/hot");
        // 首尾空白被 trim
        assert_eq!(canonical_scheme_url("  rsshub://36kr/newsflashes  "), "rsshub://36kr/newsflashes");
        // 无有效 path 的裸 scheme 不是订阅地址，原样保留（调用方按普通 URL 处理）
        assert_eq!(canonical_scheme_url("rsshub://"), "rsshub://");
    }

    #[test]
    fn canonical_rebases_official_host_to_scheme() {
        assert_eq!(
            canonical_scheme_url("https://rsshub.app/github/trending/weekly/any"),
            "rsshub://github/trending/weekly/any"
        );
        // www 与 query 都要保住
        assert_eq!(
            canonical_scheme_url("https://www.rsshub.app/36kr/newsflashes?limit=10"),
            "rsshub://36kr/newsflashes?limit=10"
        );
        // 裸官方域（无路由）原样保留，不生成空 scheme
        assert_eq!(canonical_scheme_url("https://rsshub.app/"), "https://rsshub.app/");
    }

    #[test]
    fn canonical_passes_other_urls_through() {
        assert_eq!(canonical_scheme_url("https://example.com/feed.xml"), "https://example.com/feed.xml");
        // 自建镜像存量行不在官方域：识别不了，原样保留（不劣化）
        assert_eq!(
            canonical_scheme_url("https://mirror.example.com/telegram/channel/x"),
            "https://mirror.example.com/telegram/channel/x"
        );
        assert_eq!(canonical_scheme_url(""), "");
    }

    #[test]
    fn canonical_is_idempotent() {
        for input in [
            "rsshub://v2ex/topics/hot",
            "https://rsshub.app/36kr/newsflashes?limit=10",
            "https://example.com/feed.xml",
        ] {
            let once = canonical_scheme_url(input);
            assert_eq!(canonical_scheme_url(&once), once, "canonical 必须幂等：{input}");
        }
    }

    #[test]
    fn multibyte_url_does_not_panic_in_scheme_check() {
        // 回归：url[..9] 字节切片在多字节字符落在前 9 字节时 panic（如中文开头地址）。
        // get(..9) 在非字符边界返回 None，正常落入官方域判断分支。
        let dirty = "中文地址不是合法 URL 也会被 parse 拒绝";
        assert_eq!(canonical_scheme_url(dirty), dirty);
        assert_eq!(resolve_fetch_url(dirty, MIRROR), dirty);
    }

    // ------------------------------------------------ 抓取形态

    #[test]
    fn resolve_scheme_to_current_base() {
        assert_eq!(
            resolve_fetch_url("rsshub://telegram/channel/xxx", MIRROR),
            "https://rsshub.example.com/telegram/channel/xxx"
        );
        // 空 / 非法 base = 官方默认
        assert_eq!(
            resolve_fetch_url("rsshub://telegram/channel/xxx", ""),
            "https://rsshub.app/telegram/channel/xxx"
        );
        assert_eq!(
            resolve_fetch_url("rsshub://v2ex/topics/hot", "rsshub.example.com"),
            "https://rsshub.app/v2ex/topics/hot"
        );
        // 三斜杠与 query、base 尾斜杠都要正确
        assert_eq!(
            resolve_fetch_url("rsshub:///36kr/newsflashes?limit=10", "http://127.0.0.1:1200/"),
            "http://127.0.0.1:1200/36kr/newsflashes?limit=10"
        );
    }

    #[test]
    fn resolve_rewrites_legacy_official_host_rows() {
        // 存量官方域行（未跑归一化迁移）同样跟随镜像——否则换镜像后老行仍抓旧实例
        assert_eq!(
            resolve_fetch_url("https://rsshub.app/36kr/newsflashes", MIRROR),
            "https://rsshub.example.com/36kr/newsflashes"
        );
        assert_eq!(
            resolve_fetch_url("https://www.rsshub.app/36kr/newsflashes?limit=10", MIRROR),
            "https://rsshub.example.com/36kr/newsflashes?limit=10"
        );
        // base 即官方时等价于原地址（www 归一到官方域）
        assert_eq!(
            resolve_fetch_url("https://rsshub.app/v2ex/topics/hot", ""),
            "https://rsshub.app/v2ex/topics/hot"
        );
        assert_eq!(
            resolve_fetch_url("https://www.rsshub.app/v2ex/topics/hot", ""),
            "https://rsshub.app/v2ex/topics/hot"
        );
    }

    #[test]
    fn resolve_passes_other_urls_through() {
        assert_eq!(
            resolve_fetch_url("https://example.com/feed.xml", MIRROR),
            "https://example.com/feed.xml"
        );
        // 已实例化到自建镜像的存量行：不认识，原样直抓
        assert_eq!(
            resolve_fetch_url("https://mirror.example.com/v2ex/topics/hot", MIRROR),
            "https://mirror.example.com/v2ex/topics/hot"
        );
    }

    #[test]
    fn is_scheme_url_only_for_non_empty_path() {
        assert!(is_scheme_url("rsshub://v2ex/topics/hot"));
        assert!(is_scheme_url("  RSSHUB:///gofans "));
        assert!(!is_scheme_url("rsshub://"));
        assert!(!is_scheme_url("https://rsshub.app/x"));
        assert!(!is_scheme_url("https://example.com/feed.xml"));
    }

    // ------------------------------------------------ base 工具

    #[test]
    fn clean_base_normalizes() {
        assert_eq!(clean_base(""), DEFAULT_BASE);
        assert_eq!(clean_base("  "), DEFAULT_BASE);
        assert_eq!(clean_base("https://rsshub.example.com/"), "https://rsshub.example.com");
        assert_eq!(clean_base("http://127.0.0.1:1200"), "http://127.0.0.1:1200");
        assert_eq!(clean_base("rsshub.example.com"), DEFAULT_BASE, "非 http(s) 回退官方");
    }

    #[test]
    fn is_custom_base_detects_default() {
        assert!(!is_custom_base(""));
        assert!(!is_custom_base("https://rsshub.app/"));
        assert!(is_custom_base("https://rsshub.example.com"));
    }
}
