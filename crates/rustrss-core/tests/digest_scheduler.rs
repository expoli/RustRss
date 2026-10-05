use rustrss_core::ai::digest_scheduler::DigestExtractionScheduler;
use rustrss_core::ai::{parse_retry_after, AiClient, AiConfig, AiError, AiRequest};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::time::Instant;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
fn throttle(retry_after: Option<Duration>) -> AiError {
    AiError::Provider {
        status: 429,
        message: "slow down".into(),
        retry_after,
    }
}

#[test]
fn retry_after_seconds_http_dates_and_invalid_values() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784111777);
    assert_eq!(
        parse_retry_after(" 12 ", now),
        Some(Duration::from_secs(12))
    );
    for date in [
        "Sun, 06 Nov 1994 08:49:47 GMT",
        "Sunday, 06-Nov-94 08:49:47 GMT",
        "Sun Nov  6 08:49:47 1994",
    ] {
        assert_eq!(parse_retry_after(date, now), Some(Duration::from_secs(10)));
    }
    assert_eq!(
        parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", now),
        Some(Duration::ZERO)
    );
    for bad in ["garbage", "-1", "1.5", ""] {
        assert_eq!(parse_retry_after(bad, now), None);
    }
}

#[tokio::test(start_paused = true)]
async fn progressive_concurrency_and_out_of_order_results_retain_manifest_order() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Mutex::new(Vec::new()));
    let mut finished = Vec::new();
    let results = DigestExtractionScheduler::new("test-ramp", 4, flag())
        .run(
            (0..18).collect(),
            {
                let active = active.clone();
                let peak = peak.clone();
                let started = started.clone();
                move |i| {
                    let active = active.clone();
                    let peak = peak.clone();
                    let started = started.clone();
                    async move {
                        let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(n, Ordering::SeqCst);
                        started.lock().unwrap().push((i, n));
                        tokio::time::sleep(Duration::from_millis(if i == 3 { 200 } else { 10 }))
                            .await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(i.to_string())
                    }
                }
            },
            |i, _| finished.push(i),
        )
        .await;
    assert_eq!(&started.lock().unwrap()[..3], &[(0, 1), (1, 1), (2, 1)]);
    assert_eq!(peak.load(Ordering::SeqCst), 4);
    assert!(finished.iter().position(|i| *i == 4) < finished.iter().position(|i| *i == 3));
    for (i, output) in results.into_iter().enumerate() {
        assert_eq!(output.unwrap().unwrap(), i.to_string());
    }
}

#[tokio::test(start_paused = true)]
async fn cancel_drains_paid_results_and_does_not_start_pending_items() {
    let cancel = flag();
    let starts = Arc::new(Mutex::new(Vec::new()));
    let mut cached = Vec::new();
    let results = DigestExtractionScheduler::new("test-cancel", 4, cancel.clone())
        .run(
            (0..12).collect(),
            {
                let starts = starts.clone();
                move |i| {
                    let cancel = cancel.clone();
                    let starts = starts.clone();
                    async move {
                        starts.lock().unwrap().push(i);
                        if i == 4 {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        tokio::time::sleep(Duration::from_millis(if i == 3 { 100 } else { 10 }))
                            .await;
                        Ok(i.to_string())
                    }
                }
            },
            |i, output| cached.push((i, output.as_ref().unwrap().clone())),
        )
        .await;
    assert_eq!(*starts.lock().unwrap(), vec![0, 1, 2, 3, 4]);
    assert_eq!(cached.len(), 5);
    assert!(results[..5].iter().all(Option::is_some));
    assert!(results[5..].iter().all(Option::is_none));
    assert!(cached.contains(&(3, "3".into())) && cached.contains(&(4, "4".into())));
}

#[tokio::test(start_paused = true)]
async fn one_retry_only_and_permanent_errors_do_not_skip_other_items() {
    let attempts = Arc::new(Mutex::new([0; 5]));
    let outputs = DigestExtractionScheduler::new("test-retry", 4, flag())
        .run(
            (0..5).collect(),
            {
                let attempts = attempts.clone();
                move |i: usize| {
                    let attempts = attempts.clone();
                    async move {
                        attempts.lock().unwrap()[i] += 1;
                        match i {
                            0 => Err(throttle(Some(Duration::ZERO))),
                            1 => Err(AiError::Transport("timeout".into())),
                            2 => Err(AiError::Provider {
                                status: 401,
                                message: "unauthorized".into(),
                                retry_after: None,
                            }),
                            3 => Err(AiError::BadResponse("invalid JSON".into())),
                            _ => Ok("success after failures".into()),
                        }
                    }
                }
            },
            |_, _| {},
        )
        .await;
    assert_eq!(*attempts.lock().unwrap(), [2, 2, 1, 1, 1]);
    assert!(outputs[..4].iter().all(|o| o.as_ref().unwrap().is_err()));
    assert!(outputs[4].as_ref().unwrap().is_ok());
}

#[tokio::test(start_paused = true)]
async fn cancellation_during_retry_cooldown_does_not_send_retry() {
    let cancel = flag();
    let attempts = Arc::new(AtomicUsize::new(0));
    let outputs = DigestExtractionScheduler::new("test-cancel-retry", 4, cancel.clone())
        .run(
            vec![0, 1],
            {
                let attempts = attempts.clone();
                move |_| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    cancel.store(true, Ordering::Relaxed);
                    async { Err(throttle(Some(Duration::from_secs(10)))) }
                }
            },
            |_, _| {},
        )
        .await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(outputs[0].as_ref().unwrap().is_err());
    assert!(outputs[1].is_none());
}

#[tokio::test]
async fn complete_retains_retry_after_header_for_all_providers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "2")
                .set_body_string("quota"),
        )
        .mount(&server)
        .await;
    let configs = [
        AiConfig::openai_compatible(&server.uri(), "m", "k"),
        AiConfig::anthropic("m", "k").with_base_url(&server.uri()),
        AiConfig::gemini("m", "k").with_base_url(&server.uri()),
        AiConfig::ollama("m").with_base_url(&server.uri()),
    ];
    for config in configs {
        let result = AiClient::new(config)
            .unwrap()
            .complete(AiRequest {
                system: None,
                user: "hello".into(),
            })
            .await;
        assert!(
            matches!(result, Err(AiError::Provider { status: 429, retry_after: Some(delay), .. }) if delay == Duration::from_secs(2))
        );
    }
}

#[tokio::test]
async fn mock_provider_extraction_interleaves_requests() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(50))
                .set_body_json(serde_json::json!({"choices":[{"message":{"content":"points"}}]})),
        )
        .expect(9)
        .mount(&server)
        .await;
    let client = AiClient::new(AiConfig::openai_compatible(&server.uri(), "m", "k")).unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let outputs = DigestExtractionScheduler::new(&server.uri(), 4, flag())
        .run(
            (0..9).collect(),
            {
                let active = active.clone();
                let peak = peak.clone();
                move |i: usize| {
                    let client = client.clone();
                    let active = active.clone();
                    let peak = peak.clone();
                    async move {
                        let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(n, Ordering::SeqCst);
                        let result = client
                            .complete(AiRequest {
                                system: None,
                                user: i.to_string(),
                            })
                            .await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        result
                    }
                }
            },
            |_, _| {},
        )
        .await;
    assert!(peak.load(Ordering::SeqCst) >= 2);
    assert!(outputs
        .iter()
        .all(|o| o.as_ref().unwrap().as_ref().unwrap() == "points"));
}

#[tokio::test(start_paused = true)]
async fn two_tasks_same_endpoint_share_429_cooldown() {
    let fired = Arc::new(tokio::sync::Notify::new());
    let tries = Arc::new(AtomicUsize::new(0));
    let start = Instant::now();
    let first = tokio::spawn({
        let fired = fired.clone();
        async move {
            DigestExtractionScheduler::new("test-shared/", 4, flag())
                .run(
                    vec![0],
                    move |_| {
                        let n = tries.fetch_add(1, Ordering::SeqCst);
                        let fired = fired.clone();
                        async move {
                            if n == 0 {
                                fired.notify_one();
                                Err(throttle(Some(Duration::from_millis(200))))
                            } else {
                                Ok("first".into())
                            }
                        }
                    },
                    |_, _| {},
                )
                .await
        }
    });
    fired.notified().await;
    // The worker sets endpoint cooldown synchronously before returning to us.
    let second = DigestExtractionScheduler::new("test-shared", 4, flag())
        .run(
            vec![0],
            move |_| async move {
                assert!(Instant::now().duration_since(start) >= Duration::from_millis(200));
                Ok("second".into())
            },
            |_, _| {},
        )
        .await;
    assert!(second[0].as_ref().unwrap().is_ok());
    assert!(first.await.unwrap()[0].as_ref().unwrap().is_ok());
}

#[tokio::test(start_paused = true)]
async fn billing_exhaustion_reported_as_429_is_not_retried() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let outputs = DigestExtractionScheduler::new("test-billing", 4, flag())
        .run(
            vec![0],
            {
                let attempts = attempts.clone();
                move |_| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    async {
                        Err(AiError::Provider {
                            status: 429,
                            message: r#"{"error":{"code":"insufficient_quota"}}"#.into(),
                            retry_after: Some(Duration::from_secs(60)),
                        })
                    }
                }
            },
            |_, _| {},
        )
        .await;
    assert!(outputs[0].as_ref().unwrap().is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}
