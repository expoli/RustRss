use rustrss_core::ai::chat::{
    execute_chat_turn, ChatBlock, ChatLimits, ChatMessage, ChatRequest, ChatRole,
};
use rustrss_core::ai::{AiClient, AiConfig, AiError, Provider};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn request() -> ChatRequest {
    ChatRequest {
        system: Some("基于日报回答".into()),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            blocks: vec![ChatBlock::Text("问题".into())],
        }],
        tools: vec![],
        limits: ChatLimits::default(),
    }
}
fn config(provider: Provider, uri: &str) -> AiConfig {
    let mut cfg = AiConfig::openai_compatible(uri, "test-model", "secret-key");
    cfg.provider = provider;
    cfg
}
fn endpoint(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAiCompatible => "/chat/completions",
        Provider::Anthropic => "/v1/messages",
        Provider::Gemini => "/v1beta/models/test-model:generateContent",
        Provider::Ollama => "/api/chat",
    }
}
fn response(provider: Provider) -> Value {
    match provider {
        Provider::OpenAiCompatible => {
            json!({"choices":[{"message":{"content":"答案"},"finish_reason":"stop"}],"usage":{"prompt_tokens":11,"completion_tokens":3}})
        }
        Provider::Anthropic => {
            json!({"content":[{"type":"text","text":"答案"}],"stop_reason":"end_turn","usage":{"input_tokens":11,"output_tokens":3}})
        }
        Provider::Gemini => {
            json!({"candidates":[{"content":{"parts":[{"text":"答案"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":3}})
        }
        Provider::Ollama => {
            json!({"message":{"content":"答案"},"done":true,"done_reason":"stop","prompt_eval_count":11,"eval_count":3})
        }
    }
}
async fn successful(provider: Provider) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(endpoint(provider)))
        .respond_with(move |req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            if provider != Provider::Gemini {
                assert_eq!(body["model"], "test-model");
            }
            match provider {
                Provider::OpenAiCompatible => {
                    assert_eq!(
                        req.headers.get("authorization").unwrap(),
                        "Bearer secret-key"
                    );
                    assert_eq!(body["messages"][0]["role"], "system");
                    assert_eq!(body["messages"][1]["content"], "问题");
                    assert_eq!(body["max_tokens"], 4096);
                    assert_eq!(body["stream"], false);
                }
                Provider::Anthropic => {
                    assert_eq!(req.headers.get("x-api-key").unwrap(), "secret-key");
                    assert_eq!(req.headers.get("anthropic-version").unwrap(), "2023-06-01");
                    assert_eq!(body["system"], "基于日报回答");
                    assert_eq!(body["messages"][0]["content"][0]["text"], "问题");
                }
                Provider::Gemini => {
                    assert_eq!(
                        req.url.query_pairs().find(|(k, _)| k == "key").unwrap().1,
                        "secret-key"
                    );
                    assert_eq!(body["contents"][0]["parts"][0]["text"], "问题");
                    assert_eq!(body["generationConfig"]["maxOutputTokens"], 4096);
                }
                Provider::Ollama => {
                    assert!(req.headers.get("authorization").is_none());
                    assert!(req.headers.get("x-api-key").is_none());
                    assert_eq!(body["messages"][1]["content"], "问题");
                    assert_eq!(body["options"]["num_predict"], 4096);
                    assert_eq!(body["stream"], false);
                }
            }
            assert!(body.get("tools").is_none());
            ResponseTemplate::new(200).set_body_json(response(provider))
        })
        .expect(1)
        .mount(&server)
        .await;
    let client = AiClient::new(config(provider, &server.uri())).unwrap();
    let result = execute_chat_turn(&client, &request()).await.unwrap();
    assert_eq!(result.blocks, vec![ChatBlock::Text("答案".into())]);
    assert_eq!(result.usage.input_tokens, Some(11));
    assert_eq!(result.usage.output_tokens, Some(3));
}
async fn provider_error(provider: Provider) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(endpoint(provider)))
        .respond_with(ResponseTemplate::new(429).set_body_string("quota exhausted; secret-key"))
        .expect(1)
        .mount(&server)
        .await;
    let client = AiClient::new(config(provider, &server.uri())).unwrap();
    let error = execute_chat_turn(&client, &request()).await.unwrap_err();
    assert!(matches!(error, AiError::Provider { status: 429, .. }));
    assert!(error.to_string().contains("quota exhausted"));
    assert!(!error.to_string().contains("secret-key"));
}
#[tokio::test]
async fn openai_chat_success() {
    successful(Provider::OpenAiCompatible).await;
}
#[tokio::test]
async fn anthropic_chat_success() {
    successful(Provider::Anthropic).await;
}
#[tokio::test]
async fn gemini_chat_success() {
    successful(Provider::Gemini).await;
}
#[tokio::test]
async fn ollama_chat_success() {
    successful(Provider::Ollama).await;
}
#[tokio::test]
async fn openai_chat_error_scrubs_key() {
    provider_error(Provider::OpenAiCompatible).await;
}
#[tokio::test]
async fn anthropic_chat_error_scrubs_key() {
    provider_error(Provider::Anthropic).await;
}
#[tokio::test]
async fn gemini_chat_error_scrubs_key() {
    provider_error(Provider::Gemini).await;
}
#[tokio::test]
async fn ollama_chat_error_scrubs_key() {
    provider_error(Provider::Ollama).await;
}

#[tokio::test]
async fn chat_deadline_cancels_delayed_http_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response(Provider::Ollama))
                .set_delay(std::time::Duration::from_secs(1)),
        )
        .mount(&server)
        .await;
    let client = AiClient::new(config(Provider::Ollama, &server.uri())).unwrap();
    let mut req = request();
    req.limits.timeout = std::time::Duration::from_millis(20);
    assert!(execute_chat_turn(&client, &req)
        .await
        .unwrap_err()
        .to_string()
        .contains("超时"));
}

#[tokio::test]
async fn gemini_transport_and_malformed_response_do_not_leak_key() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let uri = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let client = AiClient::new(config(Provider::Gemini, &uri)).unwrap();
    let error = execute_chat_turn(&client, &request())
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("secret-key"), "{error}");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("secret-key invalid JSON"))
        .mount(&server)
        .await;
    let client = AiClient::new(config(Provider::Gemini, &server.uri())).unwrap();
    let error = execute_chat_turn(&client, &request())
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("secret-key"), "{error}");
}

#[tokio::test]
async fn chat_response_body_size_gate_and_missing_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(16 * 1024 * 1024 + 1)))
        .mount(&server)
        .await;
    let client = AiClient::new(config(Provider::Ollama, &server.uri())).unwrap();
    assert!(execute_chat_turn(&client, &request())
        .await
        .unwrap_err()
        .to_string()
        .contains("超过上限"));
    let mut cfg = config(Provider::Anthropic, &server.uri());
    cfg.api_key = None;
    assert!(matches!(
        execute_chat_turn(&AiClient::new(cfg).unwrap(), &request()).await,
        Err(AiError::MissingKey)
    ));
}

fn stream_frames(provider: Provider) -> (String, String) {
    let data = |v: Value| format!("data: {v}\r\n\r\n");
    match provider {
        Provider::OpenAiCompatible => (
            data(json!({"choices":[{"delta":{"content":"答案🦀"}}]})),
            format!(
                "{}data: [DONE]\n\n",
                data(
                    json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":11,"completion_tokens":3}})
                )
            ),
        ),
        Provider::Anthropic => (
            format!(
                "{}event: content_block_delta\n{}",
                data(json!({"type":"message_start","message":{"usage":{"input_tokens":11}}})),
                data(
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"答案🦀"}})
                )
            ),
            format!(
                "{}{}",
                data(
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}})
                ),
                data(json!({"type":"message_stop"}))
            ),
        ),
        Provider::Gemini => (
            data(json!({"candidates":[{"content":{"parts":[{"text":"答案🦀"}]}}]})),
            data(
                json!({"candidates":[{"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":3}}),
            ),
        ),
        Provider::Ollama => (
            format!("{}\n", json!({"message":{"content":"答案🦀"},"done":false})),
            format!(
                "{}\n",
                json!({"message":{"content":""},"done":true,"done_reason":"stop","prompt_eval_count":11,"eval_count":3})
            ),
        ),
    }
}
async fn streaming_case(provider: Provider, interrupted: bool) {
    use rustrss_core::ai::chat::execute_chat_turn_streaming;
    let server = MockServer::start().await;
    let (first, end) = stream_frames(provider);
    let body = if interrupted { first } else { first + &end };
    let endpoint = if provider == Provider::Gemini {
        "/v1beta/models/test-model:streamGenerateContent"
    } else {
        endpoint(provider)
    };
    Mock::given(method("POST"))
        .and(path(endpoint))
        .respond_with(move |req: &Request| {
            let value: Value = serde_json::from_slice(&req.body).unwrap();
            if provider == Provider::Gemini {
                assert!(req.url.query_pairs().any(|(k, v)| k == "alt" && v == "sse"));
            } else {
                assert_eq!(value["stream"], true);
            }
            if provider == Provider::OpenAiCompatible {
                assert_eq!(value["stream_options"]["include_usage"], true);
            }
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body.clone())
        })
        .expect(1)
        .mount(&server)
        .await;
    let client = AiClient::new(config(provider, &server.uri())).unwrap();
    let mut text = String::new();
    let result = execute_chat_turn_streaming(&client, &request(), |s| text.push_str(s)).await;
    assert_eq!(text, "答案🦀");
    if interrupted {
        let error = result.unwrap_err().to_string();
        assert!(error.contains("3 字符") && error.contains("EOF"), "{error}");
    } else {
        let response = result.unwrap();
        assert_eq!(response.blocks, vec![ChatBlock::Text(text)]);
        assert_eq!(response.usage.input_tokens, Some(11));
        assert_eq!(response.usage.output_tokens, Some(3));
    }
}
#[tokio::test]
async fn openai_stream_success() {
    streaming_case(Provider::OpenAiCompatible, false).await;
}
#[tokio::test]
async fn openai_stream_interrupted() {
    streaming_case(Provider::OpenAiCompatible, true).await;
}
#[tokio::test]
async fn anthropic_stream_success() {
    streaming_case(Provider::Anthropic, false).await;
}
#[tokio::test]
async fn anthropic_stream_interrupted() {
    streaming_case(Provider::Anthropic, true).await;
}
#[tokio::test]
async fn gemini_stream_success() {
    streaming_case(Provider::Gemini, false).await;
}
#[tokio::test]
async fn gemini_stream_interrupted() {
    streaming_case(Provider::Gemini, true).await;
}
#[tokio::test]
async fn ollama_stream_success() {
    streaming_case(Provider::Ollama, false).await;
}
#[tokio::test]
async fn ollama_stream_interrupted() {
    streaming_case(Provider::Ollama, true).await;
}

#[tokio::test]
async fn openai_stream_options_retry_preserves_unknown_usage() {
    use rustrss_core::ai::chat::execute_chat_turn_streaming;
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|req: &Request| {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        if body.get("stream_options").is_some() { ResponseTemplate::new(400).set_body_string("unsupported parameter: stream_options; secret-key") }
        else { ResponseTemplate::new(200).set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n") }
    }).expect(2).mount(&server).await;
    let client = AiClient::new(config(Provider::OpenAiCompatible, &server.uri())).unwrap();
    let result = execute_chat_turn_streaming(&client, &request(), |_| {})
        .await
        .unwrap();
    assert_eq!(result.usage.input_tokens, None);
    assert_eq!(result.usage.output_tokens, None);
}

#[tokio::test]
async fn streaming_does_not_retry_generic_400_or_leak_gemini_key() {
    use rustrss_core::ai::chat::execute_chat_turn_streaming;
    for provider in [Provider::OpenAiCompatible, Provider::Gemini] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad model secret-key"))
            .expect(1)
            .mount(&server)
            .await;
        let client = AiClient::new(config(provider, &server.uri())).unwrap();
        let error = execute_chat_turn_streaming(&client, &request(), |_| {})
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret-key"));
        assert!(error.contains("400"));
    }
}

#[tokio::test]
async fn streamed_chunks_are_visible_before_http_finishes_and_disconnect_keeps_text() {
    use rustrss_core::ai::chat::execute_chat_turn_streaming;
    use std::io::{Read, Write};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let uri = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(AtomicBool::new(false));
    let server_seen = seen.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = [0; 8192];
        let mut request = Vec::new();
        loop {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buf[..n]);
            if let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
        let bytes = "data: {\"choices\":[{\"delta\":{\"content\":\"字🦀\"}}]}\n\n".as_bytes();
        // HTTP chunks split inside UTF-8, not just protocol line boundaries.
        for byte in bytes {
            write!(socket, "1\r\n").unwrap();
            socket.write_all(&[*byte]).unwrap();
            socket.write_all(b"\r\n").unwrap();
        }
        for _ in 0..100 {
            if server_seen.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            server_seen.load(Ordering::SeqCst),
            "delta was buffered until EOF"
        );
        // Drop without the final HTTP chunk or provider terminator.
    });
    let client = AiClient::new(config(Provider::OpenAiCompatible, &uri)).unwrap();
    let mut text = String::new();
    let error = execute_chat_turn_streaming(&client, &request(), |s| {
        text.push_str(s);
        seen.store(true, Ordering::SeqCst);
    })
    .await
    .unwrap_err();
    server.join().unwrap();
    assert_eq!(text, "字🦀");
    assert!(error.to_string().contains("2 字符"));
}

#[tokio::test]
async fn streaming_response_size_gate_and_deadline_are_enforced() {
    use rustrss_core::ai::chat::execute_chat_turn_streaming;
    let oversized = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(16 * 1024 * 1024 + 1)))
        .mount(&oversized)
        .await;
    let client = AiClient::new(config(Provider::Ollama, &oversized.uri())).unwrap();
    let error =
        execute_chat_turn_streaming(&client, &request(), |_| panic!("oversized response read"))
            .await
            .unwrap_err();
    assert!(error.to_string().contains("超过上限"));
    let delayed = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("{}")
                .set_delay(std::time::Duration::from_secs(1)),
        )
        .mount(&delayed)
        .await;
    let mut req = request();
    req.limits.timeout = std::time::Duration::from_millis(20);
    let client = AiClient::new(config(Provider::Ollama, &delayed.uri())).unwrap();
    let error = execute_chat_turn_streaming(&client, &req, |_| {})
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("超时") && error.contains("0 字符"));
}
