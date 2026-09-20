//! Feed 自动发现的 wiremock 测试。全部打本地 mock server，不依赖外网与真实站点。
//!
//! 覆盖五类场景：
//! 1. 输入地址本身就是 feed（RSS / Atom / JSON Feed）→ 原样返回；
//! 2. HTML 页面里有多个 `<link rel="alternate">` 候选 → 返回第一个，其余进 alternatives；
//! 3. 候选 href 是相对路径 → 按页面（重定向后的）地址解析成绝对地址；
//! 4. `type` 缺失/非标准 → 按 href 后缀兜底（且不抢标准 type 的位置）；
//! 5. 没有候选 → 报错并保留原始原因。

use rustrss_core::discover::{discover, DiscoveryVia};
use rustrss_core::Fetcher;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RSS_DIRECT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>直连 RSS</title><link>https://example.com/</link>
<item><title>第一条</title><link>https://example.com/1</link><guid>g1</guid>
<description>正文</description></item>
</channel></rss>"#;

const ATOM_DIRECT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>直连 Atom</title>
<id>urn:test:atom</id><updated>2026-09-20T00:00:00Z</updated>
<entry><title>第一条</title><id>urn:test:atom:1</id>
<updated>2026-09-20T00:00:00Z</updated><link href="https://example.com/1"/></entry>
</feed>"#;

const JSON_DIRECT: &str = r#"{"version":"https://jsonfeed.org/version/1.1","title":"直连 JSON Feed",
"items":[{"id":"1","title":"第一条","url":"https://example.com/1","content_text":"正文"}]}"#;

fn fetcher() -> Fetcher {
    Fetcher::new("RustRss-test/0.0").expect("HTTP 客户端应能构建")
}

async fn serve(server: &MockServer, url_path: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(url_path))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

/// 场景 1：输入地址直接返回 feed 内容 → 原样返回该地址，不做 HTML 解析
#[tokio::test]
async fn url_returning_feed_content_is_used_as_is() {
    let server = MockServer::start().await;
    serve(&server, "/rss.xml", RSS_DIRECT).await;
    serve(&server, "/atom.xml", ATOM_DIRECT).await;
    serve(&server, "/feed.json", JSON_DIRECT).await;

    for url_path in ["/rss.xml", "/atom.xml", "/feed.json"] {
        let url = format!("{}{url_path}", server.uri());
        let found = discover(&fetcher(), &url)
            .await
            .unwrap_or_else(|e| panic!("{url} 应被识别为 feed: {e}"));
        assert_eq!(found.feed_url, url, "{url} 应原样返回");
        assert_eq!(found.via, DiscoveryVia::Direct);
        assert!(found.alternatives.is_empty());
    }
}

/// 场景 2：HTML 页面多个候选 → 返回文档顺序里的第一个，其余进 alternatives（同时被记日志）
#[tokio::test]
async fn html_page_with_multiple_candidates_returns_the_first() {
    let server = MockServer::start().await;
    let body = format!(
        r#"<!DOCTYPE html><html><head><title>站点首页</title>
<link rel="alternate" type="application/rss+xml" href="{base}/main.xml">
<link rel="alternate" type="application/atom+xml" href="{base}/comments.xml">
</head><body>正文</body></html>"#,
        base = server.uri()
    );
    serve(&server, "/", &body).await;

    let found = discover(&fetcher(), &format!("{}/", server.uri()))
        .await
        .expect("页面里有两个候选，应该能发现 feed");
    assert_eq!(found.feed_url, format!("{}/main.xml", server.uri()));
    assert_eq!(found.via, DiscoveryVia::LinkType);
    assert_eq!(
        found.alternatives,
        vec![format!("{}/comments.xml", server.uri())]
    );
}

/// 场景 3：相对 href（根相对 + 文档相对）按页面地址解析为绝对地址
#[tokio::test]
async fn relative_href_is_resolved_against_the_page_url() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/blog/index.html",
        r#"<!DOCTYPE html><html><head>
<link rel="alternate" type="application/rss+xml" href="/root-feed.xml">
<link rel="alternate" type="application/atom+xml" href="atom.xml">
</head></html>"#,
    )
    .await;

    let found = discover(&fetcher(), &format!("{}/blog/index.html", server.uri()))
        .await
        .expect("相对 href 也应发现 feed");
    assert_eq!(found.feed_url, format!("{}/root-feed.xml", server.uri()));
    assert_eq!(
        found.alternatives,
        vec![format!("{}/blog/atom.xml", server.uri())]
    );
}

/// 场景 3 补充：解析基准是**重定向后**的页面地址，不是用户输入的地址
#[tokio::test]
async fn relative_href_uses_the_final_page_url_after_redirect() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/home"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/blog/", server.uri())),
        )
        .mount(&server)
        .await;
    serve(
        &server,
        "/blog/",
        r#"<html><head><link rel="alternate" type="application/rss+xml" href="atom.xml"></head></html>"#,
    )
    .await;

    let found = discover(&fetcher(), &format!("{}/home", server.uri()))
        .await
        .expect("重定向页面也应发现 feed");
    assert_eq!(found.feed_url, format!("{}/blog/atom.xml", server.uri()));
}

/// 场景 4：type 缺失 / 非标准值 → 按 href 后缀宽松兜底，并标记为兜底路径
#[tokio::test]
async fn candidates_without_standard_type_fall_back_to_href_suffix() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/no-type",
        r#"<!DOCTYPE html><html><head>
<link rel="alternate" href="/feed.rss">
</head></html>"#,
    )
    .await;
    serve(
        &server,
        "/wrong-type",
        r#"<!DOCTYPE html><html><head>
<link rel="alternate" type="text/html" href="feed.json?utm=1">
</head></html>"#,
    )
    .await;

    let found = discover(&fetcher(), &format!("{}/no-type", server.uri()))
        .await
        .expect("type 缺失时按后缀兜底");
    assert_eq!(found.feed_url, format!("{}/feed.rss", server.uri()));
    assert_eq!(found.via, DiscoveryVia::LinkSuffix);

    let found = discover(&fetcher(), &format!("{}/wrong-type", server.uri()))
        .await
        .expect("type 写错时按后缀兜底");
    assert_eq!(found.feed_url, format!("{}/feed.json?utm=1", server.uri()));
    assert_eq!(found.via, DiscoveryVia::LinkSuffix);
}

/// 场景 4 补充：兜底候选排在前面也不能抢占标准 type 的位置（“兜底”的语义）
#[tokio::test]
async fn standard_type_wins_over_an_earlier_suffix_candidate() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/mixed",
        r#"<html><head>
<link rel="alternate" href="/sitemap.xml">
<link rel="alternate" type="application/rss+xml" href="/feed.xml">
</head></html>"#,
    )
    .await;

    let found = discover(&fetcher(), &format!("{}/mixed", server.uri()))
        .await
        .expect("应优先采用标准 type 的候选");
    assert_eq!(found.feed_url, format!("{}/feed.xml", server.uri()));
    assert_eq!(found.via, DiscoveryVia::LinkType);
    assert_eq!(
        found.alternatives,
        vec![format!("{}/sitemap.xml", server.uri())]
    );
}

/// 场景 5：页面没有候选 → 明确报错，且保留原始原因
#[tokio::test]
async fn page_without_any_candidate_reports_the_original_reason() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/plain",
        "<html><head><title>没有 feed 的页面</title></head><body>正文</body></html>",
    )
    .await;

    let err = discover(&fetcher(), &format!("{}/plain", server.uri()))
        .await
        .expect_err("无候选应报错");
    let message = err.to_string();
    assert!(
        message.contains("no feed link found"),
        "错误应包含原始原因: {message}"
    );
    assert!(
        message.contains("rel=\"alternate\""),
        "错误应说明缺什么: {message}"
    );
}

/// 场景 5 补充：既不是 feed 也不是 HTML 的内容同样报错
#[tokio::test]
async fn content_that_is_neither_feed_nor_html_is_rejected() {
    let server = MockServer::start().await;
    serve(&server, "/text", "这不是 feed，也不是 HTML").await;

    let err = discover(&fetcher(), &format!("{}/text", server.uri()))
        .await
        .expect_err("不是 feed 也不是 HTML 应报错");
    let message = err.to_string();
    assert!(
        message.contains("no feed link found"),
        "错误应包含原始原因: {message}"
    );
    assert!(
        message.contains("HTML"),
        "错误应说明内容不是 HTML: {message}"
    );
}

/// 场景 5 补充：抓取失败时错误里保留 HTTP 原始原因
#[tokio::test]
async fn fetch_failure_keeps_the_original_reason() {
    let server = MockServer::start().await; // 不挂 mock → 404

    let err = discover(&fetcher(), &format!("{}/missing", server.uri()))
        .await
        .expect_err("404 应报错");
    let message = err.to_string();
    assert!(message.contains("404"), "错误应保留原始原因: {message}");
}
