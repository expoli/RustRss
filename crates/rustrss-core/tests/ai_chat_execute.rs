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
