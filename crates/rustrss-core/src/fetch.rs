//! 抓取层：HTTP 拉取 + 条件请求 + 有界并发 + 单源失败隔离。
//!
//! 结构上刻意分成两半：
//! - `Fetcher` 只负责「一个 URL → 一次结果」，无状态、可并发；
//! - `refresh` 负责编排：先并发抓，再**串行**写库。
//!
//! 为什么写库串行：SQLite 只有一个写者，并发写只会互相排队；而且这样
//! `Store`（内部是 `!Sync` 的连接）不需要跨任务共享，省掉一把锁。

use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::StatusCode;
use serde::Serialize;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::parse::parse;
use crate::store::{Result, Store};

/// 默认 UA：带产品名与版本，方便源站识别与联系（也是抓取礼貌的一部分）
pub const DEFAULT_USER_AGENT: &str = concat!("RustRss/", env!("CARGO_PKG_VERSION"));

/// feed 单次抓取的响应体上限（8 MiB）。
///
/// 与全文抓取的上限（`fulltext::MAX_BYTES` = 2 MiB）相互独立：feed 要装下整份 XML
/// （含历史条目）所以更宽，但同样必须有闸门——恶意/病态源回一份超大 body（或小体积
/// gzip 炸弹）时，整包缓冲会直接吃光进程内存（AGENTS.md 红线 11：闸门放在获取前或
/// 获取中，不放在整包缓冲之后）。
pub const MAX_FEED_BYTES: usize = 8 * 1024 * 1024;

/// 单次抓取的结果
#[derive(Debug, Clone, PartialEq)]
pub enum FetchResult {
    /// 服务端返回 304：内容没变，本次不应入库
    NotModified {
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// 拉到内容（字节原样交给解析层处理编码）
    Fetched {
        body: Vec<u8>,
        status: u16,
        final_url: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// 失败：网络错误或非 2xx（不 panic、不阻断其他源）
    Failed {
        status: Option<u16>,
        error: String,
    },
}

/// 上次抓取留下的条件请求凭据
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheHeaders {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// 刷新报告：既是日志，也是「本次刷新到底做了什么」的可验证输出
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RefreshReport {
    pub fetched: usize,
    pub not_modified: usize,
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub failures: Vec<FeedFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeedFailure {
    pub feed_id: i64,
    pub url: String,
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchSetupError {
    #[error("构建 HTTP 客户端失败: {0}")]
    Client(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct Fetcher {
    client: reqwest::Client,
}

impl Fetcher {
    pub fn new(user_agent: &str) -> std::result::Result<Self, FetchSetupError> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .gzip(true)
            .brotli(true)
            .build()?;
        Ok(Self { client })
    }

    /// 抓一个 URL。带上 ETag / Last-Modified 即为条件请求。
    /// 响应体走 [`read_body_limited`]，上限为 [`MAX_FEED_BYTES`]。
    pub async fn fetch(&self, url: &str, cache: CacheHeaders) -> FetchResult {
        self.fetch_with_limit(url, cache, MAX_FEED_BYTES).await
    }

    /// [`Fetcher::fetch`] 的可注入上限版本：测试用极小上限跑真实 HTTP 断言闸门，
    /// 不必真造 8 MiB 的 body。生产路径一律由 `fetch()` 传 [`MAX_FEED_BYTES`]。
    pub async fn fetch_with_limit(
        &self,
        url: &str,
        cache: CacheHeaders,
        max_bytes: usize,
    ) -> FetchResult {
        let mut req = self.client.get(url);
        if let Some(etag) = cache.etag.as_deref() {
            req = req.header(IF_NONE_MATCH, etag);
        }
        if let Some(lm) = cache.last_modified.as_deref() {
            req = req.header(IF_MODIFIED_SINCE, lm);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                return FetchResult::Failed {
                    status: None,
                    error: describe_error(&e),
                }
            }
        };

        let status = resp.status();
        let final_url = resp.url().to_string();
        let etag = header_string(resp.headers(), ETAG);
        let last_modified = header_string(resp.headers(), LAST_MODIFIED);

        if status == StatusCode::NOT_MODIFIED {
            return FetchResult::NotModified { etag, last_modified };
        }
        if !status.is_success() {
            return FetchResult::Failed {
                status: Some(status.as_u16()),
                error: format!("HTTP {}", status.as_u16()),
            };
        }

        // 超限与读取失败都是该源本轮失败：交给 apply_results 记失败态
        // （条目不动；etag/last_modified 传 None，由 store 的 COALESCE 保持原值）。
        match read_body_limited(resp, max_bytes).await {
            Ok(body) => FetchResult::Fetched {
                body,
                status: status.as_u16(),
                final_url,
                etag,
                last_modified,
            },
            Err(error) => FetchResult::Failed {
                status: Some(status.as_u16()),
                error,
            },
        }
    }

    /// 带体积上限的抓取（全文获取用）：与 feed 主路径共用 [`read_body_limited`]。
    /// 超限在下载完成前就放弃——不再把 2MB+ 的页面整个缓冲进内存（网关 follow-up：
    /// 原先闸门在整包缓冲后才判，白流量白内存）。
    pub async fn fetch_bytes_limited(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| describe_error(&e))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status().as_u16()));
        }
        read_body_limited(resp, max_bytes).await
    }
}

/// 读响应体并施加体积上限：Content-Length 预检 + 流式累计双重限制。
///
/// feed 主路径（[`Fetcher::fetch_with_limit`]）与全文（[`Fetcher::fetch_bytes_limited`]）
/// 共用这一处实现，闸门口径只有一个。两层都不能省：Content-Length 是**解压前**的长度，
/// gzip/brotli 解压炸弹只能靠流式累计那一层在下载中止住。
async fn read_body_limited(
    mut resp: reqwest::Response,
    max_bytes: usize,
) -> std::result::Result<Vec<u8>, String> {
    if let Some(len) = resp.content_length() {
        if len as usize > max_bytes {
            return Err(over_limit_error(len as usize, max_bytes));
        }
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("读取响应体失败: {e}"))? {
        // 已收到的字节数（超限时它是下限：超出的那一块不再计入）
        let received = body.len() + chunk.len();
        if received > max_bytes {
            return Err(over_limit_error(received, max_bytes));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// 超限文案：带上实际体积与上限，用户/日志能直接看出差多少（与全文路径同款）
fn over_limit_error(size: usize, max_bytes: usize) -> String {
    format!("页面体积 {size} 超过上限 {max_bytes}，已中止下载")
}

/// 有界并发执行：最多同时运行 `concurrency` 个任务。
///
/// 单独抽成公开函数是为了能**脱离 HTTP** 直接验证「并发确实有上限」——
/// 靠 HTTP 时序去测并发会很脆。
pub async fn bounded_map<A, B, F, Fut>(items: Vec<A>, concurrency: usize, f: F) -> Vec<B>
where
    A: Send + 'static,
    B: Send + Sync + 'static,
    F: Fn(A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = B> + Send + 'static,
{
    let limit = concurrency.clamp(1, 64);
    let semaphore = Arc::new(Semaphore::new(limit));
    let task_fn = Arc::new(f);
    let mut set = JoinSet::new();

    for item in items {
        let semaphore = semaphore.clone();
        let task_fn = task_fn.clone();
        set.spawn(async move {
            // 先占坑再执行，超出上限的任务在这里排队
            let _permit = semaphore.acquire_owned().await.expect("信号量不会关闭");
            task_fn(item).await
        });
    }

    let mut out = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(value) = joined {
            out.push(value);
        }
    }
    out
}

/// 一个订阅源的抓取任务（抓取阶段需要的全部信息，不持有 Store）
#[derive(Debug, Clone, PartialEq)]
pub struct RefreshJob {
    pub feed_id: i64,
    pub url: String,
    pub cache: CacheHeaders,
}

/// 抓取阶段的结果
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedFeed {
    pub feed_id: i64,
    pub url: String,
    pub outcome: FetchResult,
}

/// 阶段一：把「要抓什么」从库里读出来（短暂串行，随后不再碰库）
pub fn collect_jobs(store: &Store, feed_ids: &[i64]) -> Result<Vec<RefreshJob>> {
    let mut jobs = Vec::with_capacity(feed_ids.len());
    for id in feed_ids {
        let (url, etag, last_modified) = store.feed_endpoint(*id)?;
        jobs.push(RefreshJob {
            feed_id: *id,
            url,
            cache: CacheHeaders { etag, last_modified },
        });
    }
    Ok(jobs)
}

/// 一次抓取完成后的进度快照（事件载荷）。`ok + failed = done`。
/// 「成功」= 拿到内容或确认未变（Fetched / NotModified）；网络错、HTTP 错、
/// 解析失败都算 failed（与 RefreshReport 的 failures 口径一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RefreshProgress {
    pub done: u32,
    pub total: u32,
    pub ok: u32,
    pub failed: u32,
}

/// 抓取结果是否算「成功」：拿到内容或确认未变。
fn outcome_ok(outcome: &FetchResult) -> bool {
    matches!(
        outcome,
        FetchResult::Fetched { .. } | FetchResult::NotModified { .. }
    )
}

/// 阶段二：并发抓取（此阶段不需要数据库，因此可以安全地跨任务）
pub async fn fetch_jobs(fetcher: &Fetcher, jobs: Vec<RefreshJob>, concurrency: usize) -> Vec<FetchedFeed> {
    fetch_jobs_with_progress(fetcher, jobs, concurrency, |_| {}).await
}

/// [`fetch_jobs`] 的带进度版：每完成一个源回调一次 `on_progress`（无序，到达即报）。
/// 回调在抓取任务里执行，只该做廉价的事（发射事件/写原子变量），不能碰库。
pub async fn fetch_jobs_with_progress(
    fetcher: &Fetcher,
    jobs: Vec<RefreshJob>,
    concurrency: usize,
    on_progress: impl Fn(RefreshProgress) + Send + Sync + 'static,
) -> Vec<FetchedFeed> {
    let total = jobs.len() as u32;
    // (done, ok)：两个原子计数器，失败数 = done - ok，无需第三个
    let counters = Arc::new((AtomicU32::new(0), AtomicU32::new(0)));
    let fetcher = fetcher.clone();
    let on_progress = Arc::new(on_progress);
    bounded_map(jobs, concurrency, move |job| {
        let fetcher = fetcher.clone();
        let counters = counters.clone();
        let on_progress = on_progress.clone();
        async move {
            let outcome = fetcher.fetch(&job.url, job.cache.clone()).await;
            let ok_hit = outcome_ok(&outcome);
            let done = counters.0.fetch_add(1, Ordering::Relaxed) + 1;
            let ok = if ok_hit {
                counters.1.fetch_add(1, Ordering::Relaxed) + 1
            } else {
                counters.1.load(Ordering::Relaxed)
            };
            on_progress(RefreshProgress {
                done,
                total,
                ok,
                failed: done - ok,
            });
            FetchedFeed {
                feed_id: job.feed_id,
                url: job.url,
                outcome,
            }
        }
    })
    .await
}

/// 阶段三：串行落库（写入阶段；顺序执行符合 SQLite 单写者的现实）
pub fn apply_results(store: &Store, results: Vec<FetchedFeed>) -> Result<RefreshReport> {
    let mut report = RefreshReport::default();
    for FetchedFeed {
        feed_id,
        url,
        outcome,
    } in results
    {
        match outcome {
            FetchResult::NotModified { etag, last_modified } => {
                store.record_fetch(
                    feed_id,
                    "not_modified",
                    None,
                    etag.as_deref(),
                    last_modified.as_deref(),
                )?;
                report.not_modified += 1;
            }
            FetchResult::Fetched {
                body,
                etag,
                last_modified,
                ..
            } => match parse(&body) {
                Ok(feed) => {
                    store.update_feed_meta(
                        feed_id,
                        Some(&feed.title),
                        feed.site_url.as_deref(),
                        feed.description.as_deref(),
                        feed.language.as_deref(),
                    )?;
                    let stats = store.upsert_entries(feed_id, &feed.entries)?;
                    store.record_fetch(
                        feed_id,
                        "ok",
                        None,
                        etag.as_deref(),
                        last_modified.as_deref(),
                    )?;
                    report.fetched += 1;
                    report.inserted += stats.inserted;
                    report.updated += stats.updated;
                    report.unchanged += stats.unchanged;
                }
                Err(e) => {
                    let error = e.to_string();
                    // 解析失败也算失败，但缓存头仍然记下来，避免下次白白重拉
                    store.record_fetch(
                        feed_id,
                        "parse_error",
                        Some(&error),
                        etag.as_deref(),
                        last_modified.as_deref(),
                    )?;
                    report.failures.push(FeedFailure {
                        feed_id,
                        url,
                        error,
                    });
                }
            },
            FetchResult::Failed { status, error } => {
                let code = match status {
                    Some(s) => format!("http_{s}"),
                    None => "network_error".to_string(),
                };
                store.record_fetch(feed_id, &code, Some(&error), None, None)?;
                report.failures.push(FeedFailure {
                    feed_id,
                    url,
                    error,
                });
            }
        }
    }
    Ok(report)
}

/// 刷新指定订阅源（三个阶段串起来；界面里因为要跨 await 持锁，会分阶段调用）
pub async fn refresh(
    store: &Store,
    fetcher: &Fetcher,
    feed_ids: &[i64],
    concurrency: usize,
) -> Result<RefreshReport> {
    let jobs = collect_jobs(store, feed_ids)?;
    if jobs.is_empty() {
        return Ok(RefreshReport::default());
    }
    let results = fetch_jobs(fetcher, jobs, concurrency).await;
    apply_results(store, results)
}

/// 刷新全部订阅源
pub async fn refresh_all(store: &Store, fetcher: &Fetcher, concurrency: usize) -> Result<RefreshReport> {
    let ids = store.all_feed_ids()?;
    refresh(store, fetcher, &ids, concurrency).await
}

fn header_string(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 把 reqwest 的错误翻成「能直接显示给人看」的一句话
fn describe_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("超时: {e}")
    } else if e.is_connect() {
        format!("连接失败: {e}")
    } else if e.is_redirect() {
        format!("重定向次数过多: {e}")
    } else {
        format!("请求失败: {e}")
    }
}
