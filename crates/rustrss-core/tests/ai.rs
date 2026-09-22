//! AI 适配层测试：四类 provider 的请求形状与响应解析、错误透传、缓存行为。
//! 全部用 wiremock 本地服务，不需要真实 key，也不花额度。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rustrss_core::ai::prompt::{AiTask, SummaryLength, MAX_INPUT_CHARS};
use rustrss_core::ai::{run_task, AiClient, AiConfig, AiError, CachePolicy};
use rustrss_core::{Entry, EntryQuery, IdOrigin, Store};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn seeded_store(body: &str) -> (Store, i64) {
    let store = Store::open_in_memory().unwrap();
    let feed_id = store
        .add_feed("https://example.com/feed.xml", Some("示例源"))
        .unwrap();
    store
        .upsert_entries(
            feed_id,
            &[Entry {
                stable_id: "a1".into(),
                id_origin: IdOrigin::SourceData,
                source_id: "a1".into(),
                title: "测试文章标题".into(),
                url: Some("https://example.com/a1".into()),
                author: None,
                published: None,
                updated: None,
                summary: Some("摘要文本".into()),
                content_html: None,
                content_text: Some(body.into()),
                categories: Vec::new(),
            }],
        )
        .unwrap();
    let entry_id = store.list_entries(&EntryQuery::default()).unwrap()[0].id;
    (store, entry_id)
}

fn summarize_short() -> AiTask {
    AiTask::Summarize {
        length: SummaryLength::Short,
        language: "中文".into(),
    }
}

#[tokio::test]
async fn openai_compatible_shape_and_parsing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::header("authorization", "Bearer test-key"))
        .respond_with(|req: &Request| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(body["model"], "test-model");
            assert_eq!(body["stream"], false);
            assert_eq!(body["messages"][0]["role"], "system");
            assert_eq!(body["messages"][1]["role"], "user");
            assert!(
                body["messages"][1]["content"]
                    .as_str()
                    .unwrap()
                    .contains("正文内容"),
                "用户消息里应包含文章正文"
            );
            ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "role": "assistant", "content": "这是摘要" } }]
            }))
        })
        .expect(1)
        .mount(&server)
        .await;

    let config = AiConfig::openai_compatible(&format!("{}/v1", server.uri()), "test-model", "test-key");
    let client = AiClient::new(config).unwrap();
    let out = client
        .complete(rustrss_core::ai::prompt::build(
            &summarize_short(),
            &rustrss_core::ai::prompt::ArticleText {
                title: "标题",
                body: "正文内容",
            },
        ))
        .await
        .unwrap();
    assert_eq!(out, "这是摘要");
}

#[tokio::test]
async fn anthropic_shape_and_parsing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(wiremock::matchers::header("x-api-key", "ak-test"))
        .and(wiremock::matchers::header("anthropic-version", "2023-06-01"))
        .respond_with(|req: &Request| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            // Anthropic 的系统提示是顶层字段，不是 message 角色
            assert!(body["system"].as_str().unwrap().contains("只输出摘要"));
            assert_eq!(body["messages"][0]["role"], "user");
            ResponseTemplate::new(200)
                .set_body_json(json!({ "content": [{ "type": "text", "text": "Anthropic 摘要" }] }))
        })
        .expect(1)
        .mount(&server)
        .await;

    let config = AiConfig {
        provider: rustrss_core::ai::Provider::Anthropic,
        model: "claude-x".into(),
        base_url: server.uri(),
        api_key: Some("ak-test".into()),
        max_output_tokens: 512,
    };
    let client = AiClient::new(config).unwrap();
    let out = client
        .complete(rustrss_core::ai::prompt::build(
            &summarize_short(),
            &rustrss_core::ai::prompt::ArticleText {
                title: "t",
                body: "b",
            },
        ))
        .await
        .unwrap();
    assert_eq!(out, "Anthropic 摘要");
}

#[tokio::test]
async fn gemini_path_carries_model_and_parsing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-x:generateContent"))
        .respond_with(|req: &Request| {
            // 原生协议把 key 放在查询串里
            assert!(req.url.query().unwrap_or_default().contains("key=gk-test"));
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            assert!(body["systemInstruction"]["parts"][0]["text"]
                .as_str()
                .unwrap()
                .contains("只输出摘要"));
            ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{ "content": { "parts": [{ "text": "Gemini 摘要" }] } }]
            }))
        })
        .expect(1)
        .mount(&server)
        .await;

    let config = AiConfig {
        provider: rustrss_core::ai::Provider::Gemini,
        model: "gemini-x".into(),
        base_url: server.uri(),
        api_key: Some("gk-test".into()),
        max_output_tokens: 512,
    };
    let client = AiClient::new(config).unwrap();
    let out = client
        .complete(rustrss_core::ai::prompt::build(
            &summarize_short(),
            &rustrss_core::ai::prompt::ArticleText {
                title: "t",
                body: "b",
            },
        ))
        .await
        .unwrap();
    assert_eq!(out, "Gemini 摘要");
}

#[tokio::test]
async fn ollama_needs_no_key_and_merges_system_into_prompt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(|req: &Request| {
            assert!(
                req.headers.get("authorization").is_none(),
                "本地 Ollama 不应带鉴权头"
            );
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let prompt = body["prompt"].as_str().unwrap();
            assert!(prompt.contains("只输出摘要"), "系统提示应并入 prompt");
            assert!(prompt.contains("正文"));
            ResponseTemplate::new(200).set_body_json(json!({ "response": "Ollama 摘要" }))
        })
        .expect(1)
        .mount(&server)
        .await;

    let config = AiConfig::ollama("llama-x").with_base_url(&server.uri());
    let client = AiClient::new(config).unwrap();
    let out = client
        .complete(rustrss_core::ai::prompt::build(
            &summarize_short(),
            &rustrss_core::ai::prompt::ArticleText {
                title: "t",
                body: "正文",
            },
        ))
        .await
        .unwrap();
    assert_eq!(out, "Ollama 摘要");
}

#[tokio::test]
async fn provider_error_is_surfaced_and_key_is_scrubbed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(|_req: &Request| {
            // 模拟把 key 回显在错误里的网关
            ResponseTemplate::new(429)
                .set_body_string(r#"{"error":"rate limited for key sk-leaky-123"}"#)
        })
        .mount(&server)
        .await;

    let config = AiConfig::openai_compatible(&format!("{}/v1", server.uri()), "m", "sk-leaky-123");
    let client = AiClient::new(config).unwrap();
    let err = client
        .complete(rustrss_core::ai::prompt::build(
            &summarize_short(),
            &rustrss_core::ai::prompt::ArticleText {
                title: "t",
                body: "b",
            },
        ))
        .await
        .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("429"), "应保留原始状态码: {message}");
    assert!(message.contains("rate limited"), "应保留 provider 原始错误: {message}");
    assert!(!message.contains("sk-leaky-123"), "错误信息里不得出现 key: {message}");
}

#[tokio::test]
async fn task_result_is_cached_and_not_requested_twice() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |_req: &Request| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": format!("第 {} 次摘要", n + 1) } }]
            }))
        })
        .mount(&server)
        .await;

    let (store, entry_id) = seeded_store("这是一篇用于测试缓存的正文。");
    let client = AiClient::new(AiConfig::openai_compatible(
        &format!("{}/v1", server.uri()),
        "m",
        "k",
    ))
    .unwrap();
    let task = summarize_short();

    let first = run_task(&store, &client, entry_id, &task, CachePolicy::UseCache)
        .await
        .unwrap();
    assert_eq!(first.output, "第 1 次摘要");
    assert!(!first.from_cache);

    // 第二次：命中缓存，不应再打模型
    let second = run_task(&store, &client, entry_id, &task, CachePolicy::UseCache)
        .await
        .unwrap();
    assert_eq!(second.output, "第 1 次摘要");
    assert!(second.from_cache, "第二次应来自缓存");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "命中缓存时不应再请求模型");

    // 显式要求重新生成 → 会再请求一次，并覆盖缓存
    let third = run_task(&store, &client, entry_id, &task, CachePolicy::Refresh)
        .await
        .unwrap();
    assert_eq!(third.output, "第 2 次摘要");
    assert!(!third.from_cache);
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // 参数不同的任务不能复用同一份缓存
    let other = AiTask::Summarize {
        length: SummaryLength::Long,
        language: "中文".into(),
    };
    let _ = run_task(&store, &client, entry_id, &other, CachePolicy::UseCache)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3, "不同参数应各自请求");

    // 清缓存后应重新请求
    store.ai_cache_clear(Some(entry_id)).unwrap();
    let after_clear = run_task(&store, &client, entry_id, &task, CachePolicy::UseCache)
        .await
        .unwrap();
    assert!(!after_clear.from_cache);
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn oversized_body_is_truncated_and_reported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(|req: &Request| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let user = body["messages"][1]["content"].as_str().unwrap();
            assert!(user.contains("已截断"), "超长正文应在 prompt 里显式标注截断");
            ResponseTemplate::new(200)
                .set_body_json(json!({ "choices": [{ "message": { "content": "ok" } }] }))
        })
        .mount(&server)
        .await;

    let long_body = "字".repeat(MAX_INPUT_CHARS + 100);
    let (store, entry_id) = seeded_store(&long_body);
    let client = AiClient::new(AiConfig::openai_compatible(
        &format!("{}/v1", server.uri()),
        "m",
        "k",
    ))
    .unwrap();

    let outcome = run_task(&store, &client, entry_id, &summarize_short(), CachePolicy::UseCache)
        .await
        .unwrap();
    assert!(outcome.truncated, "结果里应报告正文被截断");
}

#[tokio::test]
async fn missing_entry_is_reported_not_silently_empty() {
    let (store, _id) = seeded_store("正文");
    let client = AiClient::new(AiConfig::ollama("llama")).unwrap();
    let err = run_task(&store, &client, 99999, &summarize_short(), CachePolicy::UseCache)
        .await
        .unwrap_err();
    assert!(matches!(err, AiError::EntryNotFound(99999)), "{err:?}");
}

/// 空正文诊断：推理模型烧完预算（finish_reason=length + reasoning_content 非空 +
/// content 空）时，错误信息必须指向「调大上限/换模型」，而不是笼统的「没有文本」。
#[tokio::test]
async fn empty_content_with_reasoning_budget_reports_actionable_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "content": "",
                    "reasoning_content": "我们需要回答用户……（思考链占满预算）"
                }
            }]
        })))
        .mount(&server)
        .await;

    let config = AiConfig::openai_compatible(&format!("{}/v1", server.uri()), "r1", "k");
    let client = AiClient::new(config).unwrap();
    let err = client
        .complete(rustrss_core::ai::prompt::build(
            &summarize_short(),
            &rustrss_core::ai::prompt::ArticleText { title: "t", body: "b" },
        ))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("token 上限截断"), "实际: {msg}");
    assert!(msg.contains("思考链"), "应指向推理模型: {msg}");
    assert!(msg.contains("输出上限"), "应给出可操作建议: {msg}");
}

/// 纯截断（无思考链）也要说明原因，且保留原始响应片段便于排查。
#[tokio::test]
async fn empty_content_plain_length_reports_truncation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "finish_reason": "length", "message": { "content": "" } }]
        })))
        .mount(&server)
        .await;

    let config = AiConfig::openai_compatible(&format!("{}/v1", server.uri()), "m", "k");
    let client = AiClient::new(config).unwrap();
    let err = client
        .complete(rustrss_core::ai::prompt::build(
            &summarize_short(),
            &rustrss_core::ai::prompt::ArticleText { title: "t", body: "b" },
        ))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("token 上限截断"), "实际: {msg}");
    assert!(!msg.contains("思考链"), "无思考链不该误报: {msg}");
}
