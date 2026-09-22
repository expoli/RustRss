//! 抓取层测试。全部用 `wiremock` 的本地 mock server，不依赖外网与真实源站。
//!
//! 验的是 spec 里那几条硬要求：
//! - 条件请求（ETag / Last-Modified）与 **304 不入库**；
//! - 单个源失败不阻断其他源；
//! - 并发有上限（`bounded_map` 单独验，避免依赖 HTTP 时序）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rustrss_core::fetch::{FetchResult, Fetcher, CacheHeaders};
use rustrss_core::{bounded_map, refresh, EntryQuery, Store};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const RSS_TWO_ITEMS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel>
<title>测试源</title><link>https://example.com/</link>
<item><title>第一条</title><link>https://example.com/1</link><guid>g1</guid>
  <description>正文一</description></item>
<item><title>第二条</title><link>https://example.com/2</link><guid>g2</guid>
  <description>正文二</description></item>
</channel></rss>"#;

const NOT_A_FEED: &str = "<html><body><h1>这是一个网页，不是 feed</h1></body></html>";

fn fetcher() -> Fetcher {
    Fetcher::new("RustRss-test/0.0").expect("HTTP 客户端应能构建")
}

#[tokio::test]
async fn fetch_returns_body_status_and_validators() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Last-Modified", "Wed, 21 Oct 2026 07:28:00 GMT")
                .set_body_string(RSS_TWO_ITEMS),
        )
        .mount(&server)
        .await;

    let url = format!("{}/feed.xml", server.uri());
    match fetcher().fetch(&url, CacheHeaders::default()).await {
        FetchResult::Fetched {
            body,
            status,
            final_url,
            etag,
            last_modified,
        } => {
            assert_eq!(status, 200);
            assert_eq!(final_url, url);
            assert_eq!(etag.as_deref(), Some("\"v1\""));
            assert!(last_modified.is_some());
            assert_eq!(body, RSS_TWO_ITEMS.as_bytes());
        }
        other => panic!("期望 Fetched，实际 {other:?}"),
    }
}

#[tokio::test]
async fn conditional_request_sends_validators_and_handles_304() {
    let server = MockServer::start().await;
    // 只在收到 If-None-Match 时回 304：既验证「凭据确实发出去了」，也验证 304 的处理
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(|req: &Request| {
            let has_validator = req.headers.contains_key("if-none-match")
                && req.headers.contains_key("if-modified-since");
            if has_validator {
                ResponseTemplate::new(304).insert_header("ETag", "\"v1\"")
            } else {
                ResponseTemplate::new(200).set_body_string("不该走到这里")
            }
        })
        .mount(&server)
        .await;

    let url = format!("{}/feed.xml", server.uri());
    let cached = CacheHeaders {
        etag: Some("\"v1\"".to_string()),
        last_modified: Some("Wed, 21 Oct 2026 07:28:00 GMT".to_string()),
    };
    match fetcher().fetch(&url, cached).await {
        FetchResult::NotModified { etag, .. } => assert_eq!(etag.as_deref(), Some("\"v1\"")),
        other => panic!("期望 NotModified，实际 {other:?}"),
    }
}

#[tokio::test]
async fn http_error_is_reported_not_panicking() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing.xml"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let url = format!("{}/missing.xml", server.uri());
    match fetcher().fetch(&url, CacheHeaders::default()).await {
        FetchResult::Failed { status, error } => {
            assert_eq!(status, Some(404));
            assert!(error.contains("404"), "{error}");
        }
        other => panic!("期望 Failed，实际 {other:?}"),
    }
}

#[tokio::test]
async fn refresh_then_conditional_refresh_skips_unchanged() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    // 第一次给内容 + ETag；之后一律 304（模拟源站「内容没变」）
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(move |_req: &Request| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"v1\"")
                    .set_body_string(RSS_TWO_ITEMS)
            } else {
                ResponseTemplate::new(304).insert_header("ETag", "\"v1\"")
            }
        })
        .mount(&server)
        .await;

    let store = Store::open_in_memory().unwrap();
    let url = format!("{}/feed.xml", server.uri());
    let feed_id = store.add_feed(&url, None).unwrap();

    // 第一次：应入 2 条
    let first = refresh(&store, &fetcher(), &[feed_id], 4).await.unwrap();
    assert_eq!(first.fetched, 1);
    assert_eq!(first.inserted, 2);
    assert_eq!(store.entry_count().unwrap(), 2);
    assert_eq!(store.list_feeds().unwrap()[0].last_status.as_deref(), Some("ok"));
    // 源标题应从 feed 里学到
    assert_eq!(store.list_feeds().unwrap()[0].title, "测试源");

    // 第二次：304 → 不入库、不重复计数
    let second = refresh(&store, &fetcher(), &[feed_id], 4).await.unwrap();
    assert_eq!(second.not_modified, 1);
    assert_eq!(second.inserted, 0);
    assert_eq!(second.updated, 0);
    assert!(second.failures.is_empty());
    assert_eq!(store.entry_count().unwrap(), 2, "304 不该产生任何新条目");

    // 缓存凭据已存下，供下次条件请求使用
    let (etag, _lm) = store.cache_headers(feed_id).unwrap();
    assert_eq!(etag.as_deref(), Some("\"v1\""));

    // 全量刷新入口也要能用
    let third = rustrss_core::refresh_all(&store, &fetcher(), 4).await.unwrap();
    assert_eq!(third.not_modified, 1);
}

#[tokio::test]
async fn one_bad_feed_does_not_block_the_others() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ok.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RSS_TWO_ITEMS))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/bad.xml"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/not-a-feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(NOT_A_FEED))
        .mount(&server)
        .await;

    let store = Store::open_in_memory().unwrap();
    let ok = store.add_feed(&format!("{}/ok.xml", server.uri()), None).unwrap();
    let bad = store.add_feed(&format!("{}/bad.xml", server.uri()), None).unwrap();
    let junk = store
        .add_feed(&format!("{}/not-a-feed.xml", server.uri()), None)
        .unwrap();

    let report = refresh(&store, &fetcher(), &[ok, bad, junk], 2).await.unwrap();

    assert_eq!(report.fetched, 1, "只有 ok 源成功");
    assert_eq!(report.inserted, 2);
    assert_eq!(report.failures.len(), 2, "坏源与非 feed 源都应被记为失败");

    let by_id: std::collections::HashMap<i64, String> = store
        .list_feeds()
        .unwrap()
        .into_iter()
        .map(|f| (f.id, f.last_status.unwrap_or_default()))
        .collect();
    assert_eq!(by_id.get(&ok).map(String::as_str), Some("ok"));
    assert_eq!(by_id.get(&bad).map(String::as_str), Some("http_500"));
    assert_eq!(by_id.get(&junk).map(String::as_str), Some("parse_error"));

    // 好源的数据不该受坏源影响
    assert_eq!(store.entry_count().unwrap(), 2);
    assert_eq!(
        store.list_entries(&EntryQuery::default()).unwrap().len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bounded_map_caps_concurrency() {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let (counter, peak) = (in_flight.clone(), max_seen.clone());

    let items: Vec<u32> = (0..12).collect();
    let results = bounded_map(items, 3, move |i| {
        let counter = counter.clone();
        let peak = peak.clone();
        async move {
            let now = counter.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            counter.fetch_sub(1, Ordering::SeqCst);
            i * 2
        }
    })
    .await;

    assert_eq!(results.len(), 12, "结果数量不应丢失");
    assert!(results.contains(&22), "结果内容应完整: {results:?}");

    let peak = max_seen.load(Ordering::SeqCst);
    assert!(peak <= 3, "并发上限被突破：同时 {peak} 个（上限 3）");
    assert!(peak > 1, "并发完全没生效（退化成串行）：峰值 {peak}");
}

// ---------------------------------------------------------------- 体积上限抓取

#[tokio::test]
async fn fetch_bytes_limited_rejects_oversized_content_length() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 64]))
        .expect(1)
        .mount(&server)
        .await;
    // Content-Length=64 > 4：在下载前就拒绝
    let fetcher = Fetcher::new("test").unwrap();
    let err = fetcher
        .fetch_bytes_limited(&format!("{}/big", server.uri()), 4)
        .await
        .unwrap_err();
    assert!(err.contains("超过上限"), "实际错误: {err}");
}

#[tokio::test]
async fn fetch_bytes_limited_streams_and_returns_small_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/small"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
        .mount(&server)
        .await;
    let fetcher = Fetcher::new("test").unwrap();
    let body = fetcher
        .fetch_bytes_limited(&format!("{}/small", server.uri()), 1024)
        .await
        .unwrap();
    assert_eq!(body, b"hello");
}

// ---------------------------------------------------------------- 进度回调（fetch_jobs_with_progress）

/// 进度回调逐源触发：done 单调递增至 total、每步 ok+failed=done、
/// 成功/失败口径与 RefreshReport 一致（HTTP 错算 failed，200/304 算 ok）。
#[tokio::test]
async fn fetch_jobs_progress_counts_match_outcomes() {
    use rustrss_core::fetch::{fetch_jobs_with_progress, CacheHeaders, RefreshJob};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ok1.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RSS_TWO_ITEMS))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ok2.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RSS_TWO_ITEMS))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dead.xml"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let jobs: Vec<RefreshJob> = ["ok1", "ok2", "dead"]
        .iter()
        .enumerate()
        .map(|(i, name)| RefreshJob {
            feed_id: i as i64 + 1,
            url: format!("{}/{}.xml", server.uri(), name),
            cache: CacheHeaders::default(),
        })
        .collect();

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let out = fetch_jobs_with_progress(&fetcher(), jobs, 2, move |p| {
        sink.lock().unwrap().push(p);
    })
    .await;

    assert_eq!(out.len(), 3, "三个源都返回了抓取结果");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3, "每完成一个源回调一次");
    // 每个快照的不变量
    for p in &seen {
        assert_eq!(p.total, 3);
        assert_eq!(p.ok + p.failed, p.done, "ok+failed=done");
    }
    // done 覆盖 1..=3 各一次（并发完成顺序不定，但恰好每源一报）
    let mut dones: Vec<u32> = seen.iter().map(|p| p.done).collect();
    dones.sort_unstable();
    assert_eq!(dones, vec![1, 2, 3]);
    // 终态：2 成功 1 失败
    let last = seen.last().unwrap();
    assert_eq!((last.done, last.ok, last.failed), (3, 2, 1));
}
