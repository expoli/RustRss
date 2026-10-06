use super::*;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Listener, Manager};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[test]
fn digest_concurrency_defaults_and_clamps_without_changing_profile() {
    let store = rustrss_core::Store::open_in_memory().unwrap();
    use rustrss_core::ai::Provider;
    for (provider, default) in [
        (Provider::Ollama, 1),
        (Provider::OpenAiCompatible, 4),
        (Provider::Anthropic, 4),
        (Provider::Gemini, 4),
    ] {
        assert_eq!(
            crate::ai::digest_concurrency_from_store(&store, provider),
            default
        );
    }
    for (value, expected) in [("0", 1), ("1", 1), ("8", 8), ("99", 8), ("bad", 4)] {
        store
            .set_setting(crate::ai::K_DIGEST_CONCURRENCY, value)
            .unwrap();
        assert_eq!(
            crate::ai::digest_concurrency_from_store(&store, Provider::OpenAiCompatible),
            expected
        );
    }
}

async fn generate_fixture(failure: bool, cancel: bool) {
    let server = MockServer::start().await;
    let starts = Arc::new(Mutex::new(Vec::new()));
    let app = tauri::test::mock_app();
    app.manage(AppState::for_test());
    let state = app.state::<AppState>();
    let date = "2026-10-12";
    let bounds = rustrss_core::store::digest::local_day_bounds(date).unwrap();
    let manifest = state.with_store(|s| {
        s.set_setting(crate::ai::K_PROVIDER, "ollama").unwrap();
        s.set_setting(crate::ai::K_MODEL, "mock").unwrap();
        s.set_setting(crate::ai::K_BASE_URL, &server.uri()).unwrap();
        s.set_setting(crate::ai::K_DIGEST_CONCURRENCY, "4").unwrap();
        let feed = s.add_feed("https://digest-test.invalid/rss", Some("fixture")).unwrap();
        for i in 0..9 {
            let mut parsed = rustrss_core::parse(format!("<rss version='2.0'><channel><title>T</title><item><guid>{i}</guid><title>I{i}</title><description>body</description><pubDate>1970-01-01T00:00:00Z</pubDate></item></channel></rss>").as_bytes()).unwrap();
            let entry = &mut parsed.entries[0];
            entry.published = Some(entry.published.unwrap() + Duration::from_secs((bounds.start + 100 + i) as u64));
            s.upsert_entries(feed, &parsed.entries).unwrap();
        }
        s.freeze_manifest(bounds.start, bounds.end, None).map_err(err)
    }).unwrap();
    let ordered_titles: Vec<_> = manifest.entries.iter().map(|e| e.title.clone()).collect();
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with({
            let starts = starts.clone();
            let ordered_titles = ordered_titles.clone();
            move |request: &Request| {
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                let prompt = body["prompt"].as_str().unwrap();
                if prompt.contains("只输出要点本身") {
                    let index = ordered_titles
                        .iter()
                        .position(|title| prompt.contains(&format!("标题：{title}\n")))
                        .unwrap();
                    starts.lock().unwrap().push((index, Instant::now()));
                    if failure && index == 4 {
                        return ResponseTemplate::new(401).set_body_string("unauthorized");
                    }
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_millis(if index == 3 { 200 } else { 40 }))
                        .set_body_json(serde_json::json!({"response":format!("points-{index}")}))
                } else {
                    if prompt.contains("合并成一小节") {
                        let positions: Vec<_> = ordered_titles
                            .iter()
                            .map(|title| prompt.find(&format!("【{title}】")).unwrap())
                            .collect();
                        assert!(
                            positions.windows(2).all(|pair| pair[0] < pair[1]),
                            "synthesis must restore manifest order"
                        );
                    }
                    ResponseTemplate::new(200).set_body_json(
                        serde_json::json!({"response":"overview\n\n## section\ntext"}),
                    )
                }
            }
        })
        .mount(&server)
        .await;
    let events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let terminal = Arc::new(Mutex::new(None));
    let events_copy = events.clone();
    app.listen("digest:progress", move |event| {
        let value: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
        if cancel && value["stage"] == "items" && value["done"] == 4 {
            assert!(digest_cancel(value["jobId"].as_str().unwrap().into()).unwrap());
        }
        events_copy.lock().unwrap().push(value);
    });
    let terminal_copy = terminal.clone();
    app.listen("digest:done", move |event| {
        *terminal_copy.lock().unwrap() =
            Some(serde_json::from_str::<serde_json::Value>(event.payload()).unwrap());
    });
    let job = digest_generate(app.handle().clone(), state, date.into(), None)
        .await
        .unwrap();
    assert_eq!(job.entries, 9);
    tokio::time::timeout(Duration::from_secs(5), async {
        while terminal.lock().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let done = terminal.lock().unwrap().clone().unwrap();
    assert_eq!(done["ok"], !failure && !cancel);
    assert_eq!(done["failed"], if failure { 1 } else { 0 });
    if cancel {
        assert_eq!(done["cancelled"], true);
    }
    let events = events.lock().unwrap();
    let item_done: Vec<_> = events
        .iter()
        .filter(|e| e["stage"] == "items")
        .map(|e| e["done"].as_i64().unwrap())
        .collect();
    // 并发调度下 done 按完成序推进：首 0、非降。cancel 时排空在途——终值落在
    // 取消点与全量之间（并发时序相关，不钉死具体值）
    let expected = if cancel { 5 } else { 9 };
    assert_eq!(item_done.first(), Some(&0));
    assert!(item_done.windows(2).all(|w| w[0] <= w[1]), "进度非降：{item_done:?}");
    if cancel {
        assert!(
            *item_done.last().unwrap() >= 5 && *item_done.last().unwrap() <= 9,
            "取消排空终值范围：{item_done:?}"
        );
    } else {
        assert_eq!(item_done.last(), Some(&9));
    }
    assert!(events
        .iter()
        .all(|e| e["stage"] != "items" || e["total"] == 9));
    let starts = starts.lock().unwrap();
    // 并发下发出的请求数 = 取消点前启动数（≥串行取消点的 5，≤9）
    assert!(
        (expected as usize..=9).contains(&starts.len()),
        "发出的提取请求数：{}",
        starts.len()
    );
    let t3 = starts.iter().find(|(i, _)| *i == 3).unwrap().1;
    let t4 = starts.iter().find(|(i, _)| *i == 4).unwrap().1;
    assert!(
        t4.duration_since(t3) < Duration::from_millis(100),
        "two extraction requests overlap"
    );
    let client = crate::ai::client_from_state(&app.state::<AppState>()).unwrap();
    let endpoint = rustrss_core::ai::digest::endpoint_identity(&client);
    let cache_count = app
        .state::<AppState>()
        .with_store(|s| {
            let mut count = 0;
            for (i, entry) in manifest.entries.iter().enumerate() {
                let plan = rustrss_core::ai::digest::plan_item(
                    s,
                    entry,
                    "中文",
                    &client.config().cache_tag(),
                    &endpoint,
                )
                .unwrap();
                // cancel 时并发排空在途：取消点后启动的篇也可能完成并入缓存——
                // 缓存命中集合 ≥ 串行取消点口径且 ≤ 全量
                if i < expected as usize && !(failure && i == 4) {
                    assert_eq!(plan.cached, Some(format!("points-{i}")));
                    count += 1;
                }
            }
            assert_eq!(
                s.digest_report(date, "all").unwrap().is_some(),
                !failure && !cancel
            );
            Ok(count)
        })
        .unwrap();
    assert!(cache_count >= expected - i64::from(failure), "缓存命中 ≥ 串行取消点口径");
    assert!(cache_count <= 9, "缓存命中 ≤ 全量");
}

#[tokio::test]
async fn digest_generate_concurrent_mock_provider_restores_order_and_caches() {
    generate_fixture(false, false).await;
}

#[tokio::test]
async fn digest_generate_item_failure_drains_other_items_before_overall_failure() {
    generate_fixture(true, false).await;
}

#[tokio::test]
async fn digest_generate_cancel_drains_two_in_flight_and_caches_both() {
    generate_fixture(false, true).await;
}
