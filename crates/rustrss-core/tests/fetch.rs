//! 抓取层测试。全部用 `wiremock` 的本地 mock server，不依赖外网与真实源站。
//!
//! 验的是 spec 里那几条硬要求：
//! - 条件请求（ETag / Last-Modified）与 **304 不入库**；
//! - 单个源失败不阻断其他源；
//! - 并发有上限（`bounded_map` 单独验，避免依赖 HTTP 时序）。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rustrss_core::fetch::{
    apply_results, CacheHeaders, FetchResult, FetchedFeed, Fetcher, MAX_FEED_BYTES,
};
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

// ---------------------------------------------------------------- T2 体积闸门（feed 主路径）

/// 手写最小 HTTP/1.1 服务器：造 wiremock 造不出的响应时序。
///
/// 闸门的关键分界是「响应头已到、正文还没到 / 还没结束」——wiremock 只会把响应
/// 整包吐出来，表达不了这个中间态，也就区分不了「闸门在获取前/获取中生效」与
/// 「整包缓冲完才判」。服务端把整个正文发完才置 `body_finished`：测试据此断言
/// 客户端是在正文结束**之前**就中止的，不靠计时猜。
struct RawServer {
    base: String,
    body_finished: Arc<AtomicBool>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl RawServer {
    /// `head` 是完整响应头；`frames` 紧随其后发出；`tail` 里的帧被按住最多 2 秒
    /// （测试放行 = 断言已做完，连接就此断开、不再补发正文）。
    fn start(head: String, frames: Vec<Vec<u8>>, tail: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("测试用端口应可监听");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let body_finished = Arc::new(AtomicBool::new(false));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (finished, rel) = (body_finished.clone(), release.clone());
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(mut sock) = incoming else { break };
                // 先把请求头读掉再回响应，免得响应还没写完就被 RST
                let mut buf = [0u8; 1024];
                let mut seen = Vec::new();
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") && seen.len() < 16 * 1024 {
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                let mut complete = sock.write_all(head.as_bytes()).is_ok();
                for frame in &frames {
                    complete = complete && sock.write_all(frame).is_ok();
                }
                if complete && !tail.is_empty() {
                    // 正文还没结束：按住 tail，等测试放行
                    let (lock, cvar) = &*rel;
                    let guard = cvar
                        .wait_timeout(lock.lock().unwrap(), Duration::from_secs(2))
                        .unwrap()
                        .0;
                    complete = if *guard {
                        // 放行 = 不再补发（客户端早已中止）
                        false
                    } else {
                        tail.iter().all(|f| sock.write_all(f).is_ok())
                    };
                }
                if complete {
                    finished.store(true, Ordering::SeqCst);
                }
            }
        });
        Self {
            base,
            body_finished,
            release,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// 服务端是否把整个正文发完了（false = 客户端在正文结束前就中止）
    fn body_finished(&self) -> bool {
        self.body_finished.load(Ordering::SeqCst)
    }

    /// 放行被按住的正文帧（测试收尾）
    fn finish(&self) {
        let (lock, cvar) = &*self.release;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    }
}

/// chunked 分帧：`<hex 长度>\r\n<数据>\r\n`
fn chunk(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\r\n");
    out
}

fn chunked_head() -> String {
    "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_string()
}

/// ① Content-Length 超限：预检在正文到达前就报错（不进入下载）。
#[tokio::test]
async fn fetch_rejects_oversized_content_length_before_body_arrives() {
    let body = vec![b'x'; 64];
    let server = RawServer::start(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ),
        Vec::new(),
        vec![body],
    );

    let started = Instant::now();
    let outcome = fetcher()
        .fetch_with_limit(&server.url("/feed.xml"), CacheHeaders::default(), 4)
        .await;
    let elapsed = started.elapsed();

    match outcome {
        FetchResult::Failed { status, error } => {
            assert_eq!(
                status,
                Some(200),
                "读取阶段失败沿用原状态码（与既有读取失败一致）"
            );
            assert!(
                error.contains("64") && error.contains("超过上限"),
                "文案应含实际体积与上限: {error}"
            );
        }
        other => panic!("期望 Failed，实际 {other:?}"),
    }
    assert!(
        !server.body_finished(),
        "Content-Length 预检必须在正文到达前拒绝（实际读完了正文）"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "预检应立即返回，实际 {elapsed:?}"
    );
    server.finish();
}

/// ② 没有 Content-Length 的 chunked 大响应：按累计字节在下载中砍断。
#[tokio::test]
async fn fetch_aborts_chunked_body_mid_stream() {
    // 首帧就 64 字节（上限 4），后续帧与终止块被按住：闸门必须在正文结束前中止
    let server = RawServer::start(
        chunked_head(),
        vec![chunk(&[b'x'; 64])],
        vec![chunk(&[b'y'; 32]), b"0\r\n\r\n".to_vec()],
    );

    let started = Instant::now();
    let outcome = fetcher()
        .fetch_with_limit(&server.url("/feed.xml"), CacheHeaders::default(), 4)
        .await;
    let elapsed = started.elapsed();

    match outcome {
        FetchResult::Failed { status, error } => {
            assert_eq!(status, Some(200));
            assert!(error.contains("超过上限"), "实际错误: {error}");
        }
        other => panic!("期望 Failed，实际 {other:?}"),
    }
    assert!(
        !server.body_finished(),
        "chunked 累计超限应在正文结束前中止（实际读完了整包）"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "累计超限应立即中止，实际 {elapsed:?}"
    );
    server.finish();
}

/// ③ `fetch()`（生产路径，不注上限）走的就是 `MAX_FEED_BYTES`：声明超限即拒绝，
/// 且不必真造 8 MiB 的 body；全文上限保持 2 MiB 不受影响。
#[tokio::test]
async fn fetch_applies_named_max_feed_bytes_limit() {
    assert_eq!(MAX_FEED_BYTES, 8 * 1024 * 1024, "feed 上限口径就是 8MiB");
    assert_eq!(
        rustrss_core::fulltext::MAX_BYTES,
        2 * 1024 * 1024,
        "全文上限不变（两套上限各自独立）"
    );

    let declared = MAX_FEED_BYTES + 1;
    let server = RawServer::start(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
        ),
        Vec::new(),
        vec![b"x".to_vec()],
    );

    match fetcher()
        .fetch(&server.url("/feed.xml"), CacheHeaders::default())
        .await
    {
        FetchResult::Failed { error, .. } => {
            assert!(
                error.contains(&declared.to_string())
                    && error.contains(&MAX_FEED_BYTES.to_string()),
                "文案应含实际体积 {declared} 与上限 {MAX_FEED_BYTES}: {error}"
            );
        }
        other => panic!("期望 Failed，实际 {other:?}"),
    }
    assert!(!server.body_finished(), "预检在正文到达前拒绝");
    server.finish();
}

/// 手写服务器自身的正例：上限内的 chunked 响应要被完整读出——
/// 否则上面两条失败用例可能只是「harness 写坏了」。
/// 顺带钉住全文入口（`fetch_bytes_limited`）也走同一实现。
#[tokio::test]
async fn chunked_body_under_limit_is_returned_intact() {
    let server = RawServer::start(
        chunked_head(),
        vec![chunk(b"hello "), chunk(b"world"), b"0\r\n\r\n".to_vec()],
        Vec::new(),
    );
    let body = fetcher()
        .fetch_bytes_limited(&server.url("/small"), 1024)
        .await
        .expect("上限内的小响应应成功");
    assert_eq!(body, b"hello world");
    assert!(server.body_finished(), "整个正文都送达了才谈得上成功");
    server.finish();
}

/// ④ 超限那一轮不得留下部分写入：条目不动、ETag/Last-Modified 不被覆盖、
/// 状态按该源失败记录（走既有失败路径，与 http_500 / parse_error 同口径），原因可读。
#[tokio::test]
async fn oversize_failure_writes_no_data_for_the_source() {
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

    let store = Store::open_in_memory().unwrap();
    let url = format!("{}/feed.xml", server.uri());
    let feed_id = store.add_feed(&url, None).unwrap();

    // 先一轮正常抓取：库里有 2 条 + 缓存凭据
    let first = refresh(&store, &fetcher(), &[feed_id], 1).await.unwrap();
    assert_eq!((first.fetched, first.inserted), (1, 2));
    let (etag_before, lm_before) = store.cache_headers(feed_id).unwrap();
    assert_eq!(etag_before.as_deref(), Some("\"v1\""));

    // 这一轮闸门拒绝（注入 4 字节上限），结果按「该源本轮失败」交给落库阶段
    let cache = CacheHeaders {
        etag: etag_before.clone(),
        last_modified: lm_before.clone(),
    };
    let outcome = fetcher().fetch_with_limit(&url, cache, 4).await;
    let error = match &outcome {
        FetchResult::Failed { error, .. } => error.clone(),
        other => panic!("期望 Failed，实际 {other:?}"),
    };
    assert!(error.contains("超过上限"), "实际错误: {error}");

    let report = apply_results(
        &store,
        vec![FetchedFeed {
            feed_id,
            url: url.clone(),
            outcome,
        }],
    )
    .unwrap();

    assert_eq!(report.failures.len(), 1, "超限必须作为该源本轮失败上报");
    assert_eq!(report.failures[0].error, error, "上报的就是闸门的可读原因");
    assert_eq!(store.entry_count().unwrap(), 2, "不得写入/改动任何条目");
    let (etag_after, lm_after) = store.cache_headers(feed_id).unwrap();
    assert_eq!(
        (etag_after.as_deref(), lm_after.as_deref()),
        (Some("\"v1\""), Some("Wed, 21 Oct 2026 07:28:00 GMT")),
        "不得覆盖 ETag / Last-Modified（否则下次条件请求会拿错凭据）"
    );
    let row = store.feed_row(feed_id).unwrap().unwrap();
    assert!(
        !matches!(
            row.last_status.as_deref(),
            Some("ok") | Some("not_modified")
        ),
        "不得把本轮当成功: {:?}",
        row.last_status
    );
    assert!(
        row.last_error
            .as_deref()
            .is_some_and(|e| e.contains("超过上限")),
        "失败原因应留给界面: {:?}",
        row.last_error
    );
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

/// 用户改过自定义标题后走**完整刷新管线**（fetch → parse → apply_results）：
/// 源站改名只更新 source_title，自定义名与显示名都不被覆盖。
/// 与 store 层的 update_feed_meta 单测互补——这条钉的是「刷新真的不会把用户改的名洗掉」。
#[tokio::test]
async fn refresh_keeps_custom_title_and_tracks_source_title() {
    let v1 = RSS_TWO_ITEMS.to_string();
    let v2 = RSS_TWO_ITEMS.replace("<title>测试源</title>", "<title>测试源（改版）</title>");
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(move |_req: &Request| {
            // 第一次给 v1，之后给 v2（无 ETag/Last-Modified → 不会走 304 分支，
            // 每次都真解析一遍 body）
            let body = if counter.fetch_add(1, Ordering::SeqCst) == 0 { v1.clone() } else { v2.clone() };
            ResponseTemplate::new(200).set_body_string(body)
        })
        .mount(&server)
        .await;

    let store = Store::open_in_memory().unwrap();
    let feed_id = store
        .add_feed(&format!("{}/feed.xml", server.uri()), Some("旧名"))
        .unwrap();
    store.set_feed_custom_title(feed_id, Some("我的贴名")).unwrap();

    refresh(&store, &fetcher(), &[feed_id], 4).await.unwrap();
    let row = store
        .list_feeds()
        .unwrap()
        .into_iter()
        .find(|f| f.id == feed_id)
        .unwrap();
    assert_eq!(row.title, "我的贴名", "刷新不得覆盖用户自定义名");
    assert_eq!(row.source_title, "测试源", "源站名照常学到");
    assert!(
        store
            .list_entries(&EntryQuery::default())
            .unwrap()
            .iter()
            .all(|e| e.feed_title == "我的贴名"),
        "列表/阅读区显示的也是自定义名"
    );

    // 源站改版：源站名跟着刷新走，自定义名与显示名都不动
    refresh(&store, &fetcher(), &[feed_id], 4).await.unwrap();
    let row = store.feed_row(feed_id).unwrap().unwrap();
    assert_eq!(row.title, "我的贴名");
    assert_eq!(row.source_title, "测试源（改版）");

    // 清除自定义后才跟着源站走（用户主动选择回退）
    let row = store.set_feed_custom_title(feed_id, None).unwrap();
    assert_eq!(row.title, "测试源（改版）");
}

/// RSSHub 抓取时解析的端到端：库里存 `rsshub://path`，抓取经 feed_endpoint 解析到
/// **当前镜像**；改镜像后下一次刷新打新实例，库内 url 不变（零迁移）。
#[tokio::test]
async fn rsshub_scheme_feed_resolves_through_mirror_at_fetch_time() {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    for server in [&first, &second] {
        Mock::given(method("GET"))
            .and(path("/test/1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RSS_TWO_ITEMS))
            .mount(server)
            .await;
    }

    let store = Store::open_in_memory().unwrap();
    let feed_id = store.add_feed("rsshub://test/1", None).unwrap();
    assert_eq!(store.list_feeds().unwrap()[0].url, "rsshub://test/1");

    // 镜像 A：抓取地址 = {A}/test/1
    store
        .set_setting(rustrss_core::rsshub::MIRROR_KEY, &first.uri())
        .unwrap();
    let report = refresh(&store, &fetcher(), &[feed_id], 1).await.unwrap();
    assert_eq!(report.fetched, 1, "scheme 源应能经镜像抓到");
    assert_eq!(report.inserted, 2);
    assert_eq!(
        hit_count(&first, "/test/1").await,
        1,
        "第一次抓取应打镜像 A"
    );
    assert_eq!(hit_count(&second, "/test/1").await, 0, "未配置的镜像不该被打");
    // 库内身份不变：仍然只有 scheme 形态（刷新不写 url）
    assert_eq!(store.list_feeds().unwrap()[0].url, "rsshub://test/1");

    // 换镜像 B：**不跑任何迁移**，下一次刷新直接打 B
    store
        .set_setting(rustrss_core::rsshub::MIRROR_KEY, &second.uri())
        .unwrap();
    refresh(&store, &fetcher(), &[feed_id], 1).await.unwrap();
    assert_eq!(
        hit_count(&second, "/test/1").await,
        1,
        "换镜像后下一次刷新应打镜像 B（零迁移）"
    );
    assert_eq!(hit_count(&first, "/test/1").await, 1, "镜像 A 不再被打");
    assert_eq!(store.list_feeds().unwrap()[0].url, "rsshub://test/1");
}

/// 存量官方域行（未跑归一化）也要跟随镜像——否则换镜像后老行仍抓旧实例。
#[tokio::test]
async fn legacy_official_host_row_follows_mirror_at_fetch_time() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/legacy/1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RSS_TWO_ITEMS))
        .mount(&server)
        .await;

    let store = Store::open_in_memory().unwrap();
    let feed_id = store.add_feed("https://example.com/pad", None).unwrap();
    // 直接改库构造老库存量形态（add_feed 会把官方域转成 scheme）
    store
        .update_feed_url(feed_id, "https://rsshub.app/legacy/1")
        .unwrap();
    store
        .set_setting(rustrss_core::rsshub::MIRROR_KEY, &server.uri())
        .unwrap();

    let report = refresh(&store, &fetcher(), &[feed_id], 1).await.unwrap();
    assert_eq!(report.fetched, 1);
    assert_eq!(report.inserted, 2);
    assert_eq!(hit_count(&server, "/legacy/1").await, 1);
    assert_eq!(store.list_feeds().unwrap()[0].url, "https://rsshub.app/legacy/1");
}

/// 某个 mock server 上某个 path 被请求了几次（用收到的请求记录，而不是靠断言时序）
async fn hit_count(server: &MockServer, want_path: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path() == want_path)
        .count()
}
