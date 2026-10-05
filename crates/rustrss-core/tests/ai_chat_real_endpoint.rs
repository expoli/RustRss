//! 真实端点测量（ignored）：并发提取调度器在真实 provider 上的耗时。
//! 运行：RUSTSS_AI_KEY=<key> cargo test --test ai_chat_real_endpoint -- --ignored
//! 端点/模型经环境变量覆盖（默认智谱 OpenAI 兼容 glm-4-flash）。

use rustrss_core::ai::digest_scheduler::DigestExtractionScheduler;
use rustrss_core::ai::{AiClient, AiConfig, Provider};

fn client() -> AiClient {
    let key = std::env::var("RUSTSS_AI_KEY").expect("需要 RUSTSS_AI_KEY");
    AiClient::with_proxy(
        AiConfig {
            provider: Provider::OpenAiCompatible,
            model: std::env::var("MEASURE_MODEL").unwrap_or_else(|_| "glm-4-flash".into()),
            base_url: std::env::var("MEASURE_BASE_URL")
                .unwrap_or_else(|_| "https://open.bigmodel.cn/api/paas/v4".into()),
            api_key: Some(key),
            max_output_tokens: 512,
            reasoning_effort: None,
        },
        &rustrss_core::network::ProxyConfig::default(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "真实端点测量：需要 RUSTSS_AI_KEY 与网络"]
async fn concurrent_extraction_on_real_endpoint() {
    let client = client();
    // 造 24 篇真实形态文章（正文长度贴近真实：500-1500 字符）
    let dir = tempfile::tempdir().unwrap();
    let store = {
        let s = rustrss_core::Store::open(dir.path().join("s.sqlite")).unwrap();
        let feed = s.add_feed("https://perf.invalid/rss", Some("Perf")).unwrap();
        for i in 0..24 {
            let body = format!("这是第 {i} 篇测试文章的正文。{}", "性能测量用的填充句子，包含足够长度以模拟真实正文截断行为。".repeat(12));
            let xml = format!(
                "<rss version='2.0'><channel><title>P</title><item><guid>p{i}</guid><title>性能测量文章 {i}</title><description>{body}</description><pubDate>2026-10-05T{:02}:00:00Z</pubDate></item></channel></rss>",
                i % 24
            );
            s.upsert_entries(feed, &rustrss_core::parse(xml.as_bytes()).unwrap().entries).unwrap();
        }
        s
    };
    let now = chrono::Utc::now().timestamp();
    let start = now - 86_400;
    let end = now + 86_400;
    let manifest = store.freeze_manifest(start, end, None).unwrap();
    assert!(!manifest.entries.is_empty());
    let total = manifest.entries.len();
    println!("候选篇数: {total}");

    // 并发提取（复制 commands.rs 的调度用法，limits=4 上限）
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let scheduler = DigestExtractionScheduler::new(
        std::env::var("MEASURE_BASE_URL").unwrap_or_else(|_| "https://open.bigmodel.cn/api/paas/v4".into()).as_str(),
        4,
        cancel,
    );
    let c = client.clone();
    let t0 = std::time::Instant::now();
    let pending: Vec<(usize, rustrss_core::ai::prompt::AiRequest)> = manifest
        .entries
        .iter()
        .map(|e| {
            (
                e.instance_id as usize,
                rustrss_core::ai::prompt::AiRequest {
                    system: Some("输出一句话要点".into()),
                    user: format!("{}\n\n{}", e.title, e.body),
                },
            )
        })
        .collect();
    let results = scheduler
        .run(
            pending,
            move |(_, request)| {
                let c = c.clone();
                async move { c.complete(request).await }
            },
            |_idx, result| {
                if let Err(e) = result {
                    println!("篇失败: {e}");
                }
            },
        )
        .await;
    let elapsed = t0.elapsed();
    let succeeded = results.iter().filter(|r| matches!(r, Some(Ok(_)))).count();
    println!("并发提取完成: 成功 {succeeded}/{} 耗时 {:?}", results.len(), elapsed);
    assert!(succeeded > 0, "真实端点至少一篇成功");
    // 粗口径：并发 4 下 24 篇应在 60s 内（免费档限流自适应，超时说明调度/限流失效）
    assert!(elapsed < std::time::Duration::from_secs(90), "24 篇并发提取超过 90s：{elapsed:?}");
}
