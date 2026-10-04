//! 四家非流式对话协议夹具；不创建客户端，不发网络请求。
use rustrss_core::ai::{
    chat::{
        decode_chat_response, encode_chat_request, ChatBlock, ChatLimits, ChatMessage, ChatRequest,
        ChatResponse, ChatRole, ChatTool, ChatUsage, StopReason,
    },
    AiError, Provider,
};
use serde_json::{json, Value};
use std::time::Duration;

fn call(id: &str, name: &str, args: &str) -> ChatBlock {
    ChatBlock::ToolCall {
        id: id.into(),
        name: name.into(),
        args_json: args.into(),
    }
}

fn request() -> ChatRequest {
    ChatRequest {
        system: Some("只读助手".into()),
        messages: vec![
            ChatMessage {
                role: ChatRole::User,
                blocks: vec![ChatBlock::Text("找新闻".into())],
            },
            ChatMessage {
                role: ChatRole::Assistant,
                blocks: vec![
                    ChatBlock::Text("先搜".into()),
                    call("call_0", "search", r#"{ "query": "RSS" }"#),
                    ChatBlock::Text("再取文".into()),
                    call("call_1", "get_article", r#"{"id":7}"#),
                ],
            },
            ChatMessage {
                role: ChatRole::User,
                blocks: vec![
                    ChatBlock::ToolResult {
                        call_id: "call_0".into(),
                        data_json: r#"{"ids":[7]}"#.into(),
                        error: None,
                    },
                    ChatBlock::Text("搜索完成".into()),
                    ChatBlock::ToolResult {
                        call_id: "call_1".into(),
                        data_json: "null".into(),
                        error: Some("未找到".into()),
                    },
                    ChatBlock::Text("继续".into()),
                ],
            },
        ],
        tools: vec![ChatTool {
            name: "search".into(),
            description: "搜索文章".into(),
            parameters_json: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        }],
        limits: ChatLimits::default(),
    }
}

fn schema() -> Value {
    json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]})
}

#[test]
fn openai_request_tools_and_results_fixture() {
    let body = encode_chat_request(Provider::OpenAiCompatible, &request()).unwrap();
    assert_eq!(
        body,
        json!({
            "messages": [
                {"role":"system","content":"只读助手"},
                {"role":"user","content":"找新闻"},
                {"role":"assistant","content":"先搜再取文","tool_calls":[
                    {"id":"call_0","type":"function","function":{"name":"search","arguments":"{ \"query\": \"RSS\" }"}},
                    {"id":"call_1","type":"function","function":{"name":"get_article","arguments":"{\"id\":7}"}}
                ]},
                {"role":"tool","tool_call_id":"call_0","content":"{\"ids\":[7]}"},
                {"role":"user","content":"搜索完成"},
                {"role":"tool","tool_call_id":"call_1","content":"{\"data\":null,\"error\":\"未找到\"}"},
                {"role":"user","content":"继续"}
            ],
            "tools":[{"type":"function","function":{"name":"search","description":"搜索文章","parameters":schema()}}],
            "max_tokens":4096,"stream":false
        })
    );
}

#[test]
fn anthropic_request_ordered_blocks_fixture() {
    let body = encode_chat_request(Provider::Anthropic, &request()).unwrap();
    assert_eq!(
        body,
        json!({
            "system":"只读助手", "max_tokens":4096,
            "messages":[
                {"role":"user","content":[{"type":"text","text":"找新闻"}]},
                {"role":"assistant","content":[
                    {"type":"text","text":"先搜"},
                    {"type":"tool_use","id":"call_0","name":"search","input":{"query":"RSS"}},
                    {"type":"text","text":"再取文"},
                    {"type":"tool_use","id":"call_1","name":"get_article","input":{"id":7}}
                ]},
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"call_0","content":"{\"ids\":[7]}","is_error":false},
                    {"type":"text","text":"搜索完成"},
                    {"type":"tool_result","tool_use_id":"call_1","content":"{\"data\":null,\"error\":\"未找到\"}","is_error":true},
                    {"type":"text","text":"继续"}
                ]}
            ],
            "tools":[{"name":"search","description":"搜索文章","input_schema":schema()}]
        })
    );
}

#[test]
fn gemini_request_function_response_looks_up_name_fixture() {
    let body = encode_chat_request(Provider::Gemini, &request()).unwrap();
    assert_eq!(
        body,
        json!({
            "systemInstruction":{"parts":[{"text":"只读助手"}]},
            "generationConfig":{"maxOutputTokens":4096},
            "contents":[
                {"role":"user","parts":[{"text":"找新闻"}]},
                {"role":"model","parts":[
                    {"text":"先搜"},{"functionCall":{"name":"search","args":{"query":"RSS"}}},
                    {"text":"再取文"},{"functionCall":{"name":"get_article","args":{"id":7}}}
                ]},
                {"role":"user","parts":[
                    {"functionResponse":{"name":"search","response":{"ids":[7]}}},
                    {"text":"搜索完成"},
                    {"functionResponse":{"name":"get_article","response":{"data":null,"error":"未找到"}}},
                    {"text":"继续"}
                ]}
            ],
            "tools":[{"functionDeclarations":[{"name":"search","description":"搜索文章","parameters":schema()}]}]
        })
    );
}

#[test]
fn ollama_chat_request_tools_and_tool_name_fixture() {
    let body = encode_chat_request(Provider::Ollama, &request()).unwrap();
    assert_eq!(
        body,
        json!({
            "messages":[
                {"role":"system","content":"只读助手"},
                {"role":"user","content":"找新闻"},
                {"role":"assistant","content":"先搜再取文","tool_calls":[
                    {"function":{"name":"search","arguments":{"query":"RSS"}}},
                    {"function":{"name":"get_article","arguments":{"id":7}}}
                ]},
                {"role":"tool","tool_name":"search","content":"{\"ids\":[7]}"},
                {"role":"user","content":"搜索完成"},
                {"role":"tool","tool_name":"get_article","content":"{\"data\":null,\"error\":\"未找到\"}"},
                {"role":"user","content":"继续"}
            ],
            "tools":[{"type":"function","function":{"name":"search","description":"搜索文章","parameters":schema()}}],
            "options":{"num_predict":4096},"stream":false
        })
    );
}

fn expected_calls() -> Vec<ChatBlock> {
    vec![
        ChatBlock::Text("查资料".into()),
        call("call_0", "search", r#"{"query":"RSS"}"#),
        call("call_1", "get_article", r#"{"id":7}"#),
    ]
}

#[test]
fn openai_response_multiple_calls_and_usage_fixture() {
    let body = json!({"id":"chatcmpl-x","choices":[{"index":0,"message":{"role":"assistant","content":"查资料","tool_calls":[
        {"id":"call_0","type":"function","function":{"name":"search","arguments":"{ \"query\": \"RSS\" }"}},
        {"id":"call_1","type":"function","function":{"name":"get_article","arguments":"{\"id\":7}"}}
    ]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":123,"completion_tokens":45,"total_tokens":168}});
    assert_eq!(
        decode_chat_response(Provider::OpenAiCompatible, &body).unwrap(),
        ChatResponse {
            blocks: expected_calls(),
            stop_reason: StopReason::ToolUse,
            usage: ChatUsage {
                input_tokens: Some(123),
                output_tokens: Some(45)
            }
        }
    );
}

#[test]
fn anthropic_response_multiple_calls_and_usage_fixture() {
    let body = json!({"id":"msg-x","type":"message","role":"assistant","content":[
        {"type":"text","text":"查资料"},
        {"type":"tool_use","id":"call_0","name":"search","input":{"query":"RSS"}},
        {"type":"tool_use","id":"call_1","name":"get_article","input":{"id":7}}
    ],"stop_reason":"tool_use","usage":{"input_tokens":123,"output_tokens":45}});
    assert_eq!(
        decode_chat_response(Provider::Anthropic, &body).unwrap(),
        ChatResponse {
            blocks: expected_calls(),
            stop_reason: StopReason::ToolUse,
            usage: ChatUsage {
                input_tokens: Some(123),
                output_tokens: Some(45)
            }
        }
    );
}

#[test]
fn gemini_response_object_args_normalized_and_usage_fixture() {
    let body = json!({"candidates":[{"content":{"role":"model","parts":[
        {"text":"查资料"},
        {"functionCall":{"name":"search","args":{"query":"RSS"}}},
        {"functionCall":{"name":"get_article","args":{"id":7}}}
    ]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":123,"candidatesTokenCount":45,"totalTokenCount":168}});
    assert_eq!(
        decode_chat_response(Provider::Gemini, &body).unwrap(),
        ChatResponse {
            blocks: expected_calls(),
            stop_reason: StopReason::ToolUse,
            usage: ChatUsage {
                input_tokens: Some(123),
                output_tokens: Some(45)
            }
        }
    );
}

#[test]
fn ollama_response_arguments_object_and_string_without_usage_fixture() {
    let body = json!({"model":"llama-x","message":{"role":"assistant","content":"查资料","tool_calls":[
        {"function":{"name":"search","arguments":{"query":"RSS"}}},
        {"function":{"name":"get_article","arguments":"{ \"id\": 7 }"}}
    ]},"done":true,"done_reason":"stop"});
    assert_eq!(
        decode_chat_response(Provider::Ollama, &body).unwrap(),
        ChatResponse {
            blocks: expected_calls(),
            stop_reason: StopReason::ToolUse,
            usage: ChatUsage::default()
        }
    );
}

#[test]
fn all_providers_plain_text_and_missing_usage_fixtures() {
    let cases = [
        (
            Provider::OpenAiCompatible,
            json!({"choices":[{"message":{"content":"你好"},"finish_reason":"stop"}]}),
        ),
        (
            Provider::Anthropic,
            json!({"content":[{"type":"text","text":"你好"}],"stop_reason":"end_turn"}),
        ),
        (
            Provider::Gemini,
            json!({"candidates":[{"content":{"parts":[{"text":"你好"}]},"finishReason":"STOP"}]}),
        ),
        (
            Provider::Ollama,
            json!({"message":{"content":"你好"},"done":true}),
        ),
    ];
    for (provider, fixture) in cases {
        assert_eq!(
            decode_chat_response(provider, &fixture).unwrap(),
            ChatResponse {
                blocks: vec![ChatBlock::Text("你好".into())],
                stop_reason: StopReason::EndTurn,
                usage: ChatUsage::default()
            },
            "{provider:?}"
        );
    }
}

#[test]
fn all_providers_max_tokens_and_unknown_stop_reason_fixtures() {
    let cases = [
        (
            Provider::OpenAiCompatible,
            json!({"choices":[{"message":{"content":null},"finish_reason":"length"}]}),
            "/choices/0/finish_reason",
        ),
        (
            Provider::Anthropic,
            json!({"content":[],"stop_reason":"max_tokens"}),
            "/stop_reason",
        ),
        (
            Provider::Gemini,
            json!({"candidates":[{"content":{"parts":[]},"finishReason":"MAX_TOKENS"}]}),
            "/candidates/0/finishReason",
        ),
        (
            Provider::Ollama,
            json!({"message":{"content":""},"done":true,"done_reason":"length","prompt_eval_count":5,"eval_count":0}),
            "/done_reason",
        ),
    ];
    for (provider, mut fixture, pointer) in cases {
        let response = decode_chat_response(provider, &fixture).unwrap();
        assert_eq!(response.stop_reason, StopReason::MaxTokens, "{provider:?}");
        assert!(response.blocks.is_empty());
        if provider == Provider::Ollama {
            assert_eq!(
                response.usage,
                ChatUsage {
                    input_tokens: Some(5),
                    output_tokens: Some(0)
                }
            );
        }
        *fixture.pointer_mut(pointer).unwrap() = json!("unknown");
        assert_eq!(
            decode_chat_response(provider, &fixture)
                .unwrap()
                .stop_reason,
            StopReason::Other
        );
    }
}

#[test]
fn gemini_and_ollama_unknown_result_id_is_request_error() {
    for provider in [Provider::Gemini, Provider::Ollama] {
        let mut req = request();
        req.messages[2].blocks[0] = ChatBlock::ToolResult {
            call_id: "unknown".into(),
            data_json: "{}".into(),
            error: None,
        };
        let error = encode_chat_request(provider, &req).unwrap_err();
        assert!(matches!(error, AiError::Request(_)));
        let message = error.to_string();
        assert!(
            message.contains("unknown") && message.contains("call_0") && message.contains("call_1")
        );
    }
}

#[test]
fn all_providers_minimal_request_omits_optional_fields_and_uses_output_limit() {
    let req = ChatRequest {
        system: None,
        messages: vec![ChatMessage {
            role: ChatRole::User,
            blocks: vec![ChatBlock::Text("你好".into())],
        }],
        tools: vec![],
        limits: ChatLimits {
            max_output_tokens: 99,
            ..ChatLimits::default()
        },
    };
    for provider in [
        Provider::OpenAiCompatible,
        Provider::Anthropic,
        Provider::Gemini,
        Provider::Ollama,
    ] {
        let body = encode_chat_request(provider, &req).unwrap();
        assert!(body.get("tools").is_none());
        assert!(body.get("system").is_none() && body.get("systemInstruction").is_none());
        let limit = match provider {
            Provider::Gemini => &body["generationConfig"]["maxOutputTokens"],
            Provider::Ollama => &body["options"]["num_predict"],
            _ => &body["max_tokens"],
        };
        assert_eq!(limit, &json!(99));
        assert!(body.get("max_model_requests").is_none());
    }
}

#[test]
fn malformed_requests_and_responses_return_typed_errors() {
    for provider in [
        Provider::OpenAiCompatible,
        Provider::Anthropic,
        Provider::Gemini,
        Provider::Ollama,
    ] {
        let mut req = request();
        req.messages[1].blocks[1] = call("call_0", "search", "not json");
        assert!(matches!(
            encode_chat_request(provider, &req),
            Err(AiError::Request(_))
        ));
        req.messages[1].blocks[1] = call("call_0", "search", "[]");
        assert!(matches!(
            encode_chat_request(provider, &req),
            Err(AiError::Request(_))
        ));
        req = request();
        req.messages[1].role = ChatRole::User;
        assert!(matches!(
            encode_chat_request(provider, &req),
            Err(AiError::Request(_))
        ));
        assert!(matches!(
            decode_chat_response(provider, &json!({})),
            Err(AiError::BadResponse(_))
        ));
    }
    let fixtures = [
        (
            Provider::OpenAiCompatible,
            json!({"choices":[{"message":{"tool_calls":[{"id":"x","function":{"name":"search","arguments":"invalid"}}]}}]}),
        ),
        (
            Provider::Anthropic,
            json!({"content":[{"type":"tool_use","id":"x","name":"search","input":[]}]}),
        ),
        (
            Provider::Gemini,
            json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"search","args":"{}"}}]}}]}),
        ),
        (
            Provider::Ollama,
            json!({"message":{"tool_calls":[{"function":{"name":"search","arguments":null}}]}}),
        ),
    ];
    for (provider, fixture) in fixtures {
        assert!(matches!(
            decode_chat_response(provider, &fixture),
            Err(AiError::BadResponse(_))
        ));
    }
}

#[test]
fn gemini_array_tool_result_is_wrapped_as_response_object() {
    let mut req = request();
    req.messages[2].blocks[0] = ChatBlock::ToolResult {
        call_id: "call_0".into(),
        data_json: "[7]".into(),
        error: None,
    };
    let body = encode_chat_request(Provider::Gemini, &req).unwrap();
    assert_eq!(
        body["contents"][2]["parts"][0],
        json!({"functionResponse":{"name":"search","response":{"data":[7]}}})
    );
}

#[test]
fn chat_types_serde_roundtrip_and_default_limits() {
    let req = request();
    assert_eq!(req.limits.max_model_requests, 6);
    assert_eq!(req.limits.max_tool_calls, 10);
    assert_eq!(req.limits.timeout, Duration::from_secs(120));
    assert_eq!(req.limits.max_output_tokens, 4096);
    assert_eq!(
        serde_json::from_value::<ChatRequest>(serde_json::to_value(&req).unwrap()).unwrap(),
        req
    );
    let response = ChatResponse {
        blocks: expected_calls(),
        stop_reason: StopReason::ToolUse,
        usage: ChatUsage {
            input_tokens: None,
            output_tokens: Some(42),
        },
    };
    assert_eq!(
        serde_json::from_value::<ChatResponse>(serde_json::to_value(&response).unwrap()).unwrap(),
        response
    );
}

#[test]
fn openai_nullable_content_and_tool_calls_fixtures() {
    let response = decode_chat_response(
        Provider::OpenAiCompatible,
        &json!({
            "choices":[{"message":{"content":null,"tool_calls":[
                {"id":"call_0","type":"function","function":{"name":"search","arguments":"{}"}}
            ]},"finish_reason":"tool_calls"}]
        }),
    )
    .unwrap();
    assert_eq!(response.blocks, vec![call("call_0", "search", "{}")]);
    assert_eq!(response.stop_reason, StopReason::ToolUse);
    let response = decode_chat_response(Provider::OpenAiCompatible, &json!({
        "choices":[{"message":{"content":"{\"functionCall\":{\"name\":\"search\"}}","tool_calls":null},"finish_reason":"stop"}]
    })).unwrap();
    assert_eq!(
        response.blocks,
        vec![ChatBlock::Text(
            r#"{"functionCall":{"name":"search"}}"#.into()
        )]
    );
    assert_eq!(response.stop_reason, StopReason::EndTurn);
}

#[test]
fn anthropic_and_gemini_response_preserves_interleaved_text_and_calls() {
    let fixtures = [
        (
            Provider::Anthropic,
            json!({"content":[
            {"type":"text","text":"先搜"},
            {"type":"tool_use","id":"call_0","name":"search","input":{}},
            {"type":"text","text":"再取"},
            {"type":"tool_use","id":"call_1","name":"get_article","input":{"id":7}}
        ],"stop_reason":"tool_use"}),
        ),
        (
            Provider::Gemini,
            json!({"candidates":[{"content":{"parts":[
            {"text":"先搜"},{"functionCall":{"name":"search","args":{}}},
            {"text":"再取"},{"functionCall":{"name":"get_article","args":{"id":7}}}
        ]},"finishReason":"STOP"}]}),
        ),
    ];
    for (provider, fixture) in fixtures {
        let response = decode_chat_response(provider, &fixture).unwrap();
        assert_eq!(
            response.blocks,
            vec![
                ChatBlock::Text("先搜".into()),
                call("call_0", "search", "{}"),
                ChatBlock::Text("再取".into()),
                call("call_1", "get_article", r#"{"id":7}"#)
            ]
        );
    }
}
