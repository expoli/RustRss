use rustrss_core::ai::chat::{ChatBlock, ChatLimits, ChatMessage, ChatRequest, ChatRole};
use rustrss_core::ai::chat_agent::{
    chat_capability, retrieval_evidence, run_agent_turn, tools_unsupported, ChatCapability,
};
use rustrss_core::ai::tools::{ToolOutput, TOOL_BYTES};
use rustrss_core::ai::{AiClient, AiConfig, AiError, Provider};
use rustrss_core::Store;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn request() -> ChatRequest {
    ChatRequest {
        system: Some("read only".into()),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            blocks: vec![ChatBlock::Text("Rust".into())],
        }],
        tools: vec![],
        limits: ChatLimits::default(),
    }
}
fn client(provider: Provider, uri: &str) -> AiClient {
    static MODELS: AtomicUsize = AtomicUsize::new(0);
    let model = format!("test-model-{}", MODELS.fetch_add(1, Ordering::SeqCst));
    let mut cfg = AiConfig::openai_compatible(uri, &model, "secret");
    cfg.provider = provider;
    AiClient::new(cfg).unwrap()
}
fn output() -> Result<ToolOutput, rustrss_core::ai::tools::ToolError> {
    Ok(ToolOutput {
        data_json: json!({"articles":[],"count":0}).to_string(),
        truncated: false,
    })
}
fn final_response(provider: Provider) -> Value {
    match provider {
        Provider::OpenAiCompatible => {
            json!({"choices":[{"message":{"content":"answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}})
        }
        Provider::Ollama => {
            json!({"message":{"content":"answer"},"done":true,"prompt_eval_count":3,"eval_count":2})
        }
        Provider::Anthropic => {
            json!({"content":[{"type":"text","text":"answer"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}})
        }
        Provider::Gemini => {
            json!({"candidates":[{"content":{"parts":[{"text":"answer"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2}})
        }
    }
}
fn call_response(provider: Provider, calls: &[(&str, Value)]) -> Value {
    let functions: Vec<_> = calls.iter().enumerate().map(|(i,(name,args))| match provider {
        Provider::OpenAiCompatible => json!({"id":format!("c{i}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}),
        Provider::Ollama => json!({"function":{"name":name,"arguments":args}}),
        Provider::Anthropic => json!({"type":"tool_use","id":format!("c{i}"),"name":name,"input":args}),
        Provider::Gemini => json!({"functionCall":{"name":name,"args":args},"thoughtSignature":format!("signature{i}")}),
    }).collect();
    match provider {
        Provider::OpenAiCompatible => {
            json!({"choices":[{"message":{"content":null,"tool_calls":functions},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":2}})
        }
        Provider::Ollama => {
            json!({"message":{"tool_calls":functions},"done":true,"prompt_eval_count":3,"eval_count":2})
        }
        Provider::Anthropic => {
            json!({"content":functions,"stop_reason":"tool_use","usage":{"input_tokens":3,"output_tokens":2}})
        }
        Provider::Gemini => {
            json!({"candidates":[{"content":{"parts":functions},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2}})
        }
    }
}
async fn two_rounds(provider: Provider) {
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            assert!(body.get("tools").is_some());
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(200).set_body_json(call_response(
                    provider,
                    &[
                        ("search_articles", json!({"query":"Rust"})),
                        ("list_tags", json!({})),
                    ],
                ))
            } else {
                let text = body.to_string();
                assert!(text.contains("articles"));
                match provider {
                    Provider::OpenAiCompatible => {
                        assert_eq!(body["messages"][3]["role"], "tool");
                        assert_eq!(body["messages"][4]["role"], "tool");
                    }
                    Provider::Ollama => {
                        assert_eq!(body["messages"][3]["tool_name"], "search_articles");
                        assert_eq!(body["messages"][4]["tool_name"], "list_tags");
                    }
                    Provider::Anthropic => {
                        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
                        assert_eq!(body["messages"][2]["content"][1]["type"], "tool_result");
                    }
                    Provider::Gemini => {
                        assert_eq!(
                            body["contents"][1]["parts"][0]["thoughtSignature"],
                            "signature0"
                        );
                        assert_eq!(
                            body["contents"][2]["parts"][0]["functionResponse"]["name"],
                            "search_articles"
                        );
                        assert_eq!(
                            body["contents"][2]["parts"][1]["functionResponse"]["name"],
                            "list_tags"
                        );
                    }
                }
                ResponseTemplate::new(200).set_body_json(final_response(provider))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let mut executions = 0;
    let mut progress = 0;
    let out = run_agent_turn(
        &client(provider, &server.uri()),
        request(),
        |_, _| {
            executions += 1;
            output()
        },
        |_| panic!("no retrieval"),
        || false,
        |_| progress += 1,
    )
    .await
    .unwrap();
    assert_eq!(out.final_blocks, vec![ChatBlock::Text("answer".into())]);
    assert_eq!(executions, 2);
    assert_eq!(progress, 2);
    assert_eq!(out.tool_calls_log.len(), 2);
    assert_eq!(out.usage.input_tokens, Some(6));
    assert_eq!(out.usage.output_tokens, Some(4));
    assert!(!out.degraded);
}
#[tokio::test]
async fn openai_multi_tool_roundtrip() {
    two_rounds(Provider::OpenAiCompatible).await;
}
#[tokio::test]
async fn ollama_multi_tool_roundtrip() {
    two_rounds(Provider::Ollama).await;
}
#[tokio::test]
async fn anthropic_multi_tool_roundtrip() {
    two_rounds(Provider::Anthropic).await;
}
#[tokio::test]
async fn gemini_multi_tool_roundtrip() {
    two_rounds(Provider::Gemini).await;
}

async fn budget(
    model_max: u32,
    tool_max: u32,
    batch: usize,
    expected_requests: usize,
    expected_tools: usize,
) {
    let server = MockServer::start().await;
    let n = Arc::new(AtomicUsize::new(0));
    let counter = n.clone();
    Mock::given(method("POST"))
        .respond_with(move |_: &Request| {
            let round = counter.fetch_add(1, Ordering::SeqCst);
            let calls: Vec<_> = (0..batch)
                .map(|i| ("list_articles", json!({"limit":round*batch+i+1})))
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(call_response(Provider::OpenAiCompatible, &calls))
        })
        .mount(&server)
        .await;
    let mut req = request();
    req.limits.max_model_requests = model_max;
    req.limits.max_tool_calls = tool_max;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        req,
        |_, _| output(),
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(n.load(Ordering::SeqCst), expected_requests);
    assert_eq!(out.tool_calls_log.len(), expected_tools);
    assert!(matches!(&out.final_blocks[0],ChatBlock::Text(s) if s.contains("预算")));
}
#[tokio::test]
async fn hard_six_requests_even_when_caller_loosens_limits() {
    budget(99, 99, 1, 6, 5).await;
}
#[tokio::test]
async fn hard_ten_tools_and_multi_call_batch_is_atomic() {
    budget(99, 99, 4, 3, 8).await;
}
#[tokio::test]
async fn tightened_model_budget() {
    budget(2, 10, 1, 2, 1).await;
}
#[tokio::test]
async fn tightened_tool_budget() {
    budget(6, 1, 1, 2, 1).await;
}

#[tokio::test]
async fn repeated_query_without_text_stops_before_second_execution() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(call_response(
            Provider::OpenAiCompatible,
            &[("list_articles", json!({}))],
        )))
        .expect(2)
        .mount(&server)
        .await;
    let mut tools = 0;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| {
            tools += 1;
            output()
        },
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(tools, 1);
    assert!(matches!(&out.final_blocks[0],ChatBlock::Text(s) if s.contains("重复")));
}

#[tokio::test]
async fn unknown_and_missing_arguments_never_reach_callback() {
    let server = MockServer::start().await;
    let n = Arc::new(AtomicUsize::new(0));
    let counter = n.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(200).set_body_json(call_response(
                    Provider::OpenAiCompatible,
                    &[("set_read", json!({})), ("get_article", json!({}))],
                ))
            } else {
                let body = String::from_utf8(req.body.clone()).unwrap();
                assert!(body.contains("unknown_tool"));
                assert!(body.contains("invalid_argument"));
                ResponseTemplate::new(200).set_body_json(final_response(Provider::OpenAiCompatible))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| panic!("must reject first"),
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert!(out.tool_calls_log.iter().all(|log| !log.ok));
}

#[tokio::test]
async fn explicit_unsupported_tools_injects_scoped_fts_and_caches_plain_qa() {
    let server = MockServer::start().await;
    let store = Store::open_in_memory().unwrap();
    let feed = store.add_feed("https://a.invalid", Some("A")).unwrap();
    store.upsert_entries(feed,&rustrss_core::parse(b"<rss version='2.0'><channel><title>A</title><item><guid>r</guid><title>Rust evidence</title><description>Rust local body</description></item></channel></rss>").unwrap().entries).unwrap();
    let n = Arc::new(AtomicUsize::new(0));
    let counter = n.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                assert!(body.get("tools").is_some());
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error":{"message":"This model does not support tools"}}))
            } else {
                assert!(body.get("tools").is_none());
                assert!(body.to_string().contains("Rust local body"));
                assert!(body.to_string().contains("绑定日报章节"));
                ResponseTemplate::new(200).set_body_json(final_response(Provider::OpenAiCompatible))
            }
        })
        .expect(3)
        .mount(&server)
        .await;
    let c = client(Provider::OpenAiCompatible, &server.uri());
    for _ in 0..2 {
        let out = run_agent_turn(
            &c,
            request(),
            |_, _| panic!(),
            |_| retrieval_evidence(&store, "all", "Rust", Some("Rust frozen report")),
            || false,
            |_| {},
        )
        .await
        .unwrap();
        assert!(out.degraded);
        assert_eq!(
            out.degraded_reason.as_deref(),
            Some("provider_tools_unsupported")
        );
    }
    assert_eq!(
        chat_capability(
            c.config().provider,
            &c.config().model,
            &rustrss_core::ai::digest::endpoint_identity(&c)
        ),
        ChatCapability::Unsupported
    );
}

#[tokio::test]
async fn authentication_rate_limit_and_generic_schema_errors_never_degrade() {
    for status in [401, 429, 400] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status).set_body_string("unsupported tool schema type"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let c = client(Provider::OpenAiCompatible, &server.uri());
        assert!(run_agent_turn(
            &c,
            request(),
            |_, _| panic!(),
            |_| panic!("not capability failure"),
            || false,
            |_| {}
        )
        .await
        .is_err());
        assert_eq!(
            chat_capability(
                c.config().provider,
                &c.config().model,
                &rustrss_core::ai::digest::endpoint_identity(&c)
            ),
            ChatCapability::Unknown
        );
    }
}
#[test]
fn provider_specific_capability_error_phrases_are_narrow() {
    for (provider, message) in [
        (Provider::OpenAiCompatible, "Unsupported parameter: 'tools'"),
        (Provider::Ollama, "model does not support tools"),
        (Provider::Anthropic, "tool use is not supported"),
        (Provider::Gemini, "function calling is not supported"),
    ] {
        assert!(tools_unsupported(
            provider,
            &AiError::Provider {
                status: 400,
                message: message.into()
            }
        ));
        for status in [401, 429, 500] {
            assert!(!tools_unsupported(
                provider,
                &AiError::Provider {
                    status,
                    message: message.into()
                }
            ));
        }
        assert!(!tools_unsupported(
            provider,
            &AiError::Transport("timeout tools unsupported".into())
        ));
    }
}

#[tokio::test]
async fn cancellation_drops_delayed_http_future_promptly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(final_response(Provider::OpenAiCompatible))
                .set_delay(std::time::Duration::from_secs(2)),
        )
        .mount(&server)
        .await;
    let flag = Arc::new(AtomicBool::new(false));
    let cancel = flag.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        cancel.store(true, Ordering::SeqCst);
    });
    let start = std::time::Instant::now();
    let error = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| panic!(),
        |_| panic!(),
        || flag.load(Ordering::SeqCst),
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
}

#[tokio::test]
async fn tool_byte_and_total_evidence_gates_are_visible_to_model() {
    let server = MockServer::start().await;
    let n = Arc::new(AtomicUsize::new(0));
    let counter = n.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let round = counter.fetch_add(1, Ordering::SeqCst);
            if round == 0 {
                let calls: Vec<_> = (0..1)
                    .map(|i| ("list_articles", json!({"limit":i+1})))
                    .collect();
                ResponseTemplate::new(200)
                    .set_body_json(call_response(Provider::OpenAiCompatible, &calls))
            } else {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                let results: Vec<_> = body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|m| m["role"] == "tool")
                    .collect();
                let mut total = 0;
                for result in results {
                    let data = result["content"].as_str().unwrap();
                    assert!(data.len() <= TOOL_BYTES);
                    total += data.len();
                    assert_eq!(
                        serde_json::from_str::<Value>(data).unwrap()["truncated"],
                        true
                    );
                }
                assert!(total <= 48 * 1024);
                ResponseTemplate::new(200).set_body_json(final_response(Provider::OpenAiCompatible))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| {
            Ok(ToolOutput {
                data_json: json!({"text":"界".repeat(20_000)}).to_string(),
                truncated: false,
            })
        },
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert!(out.tool_calls_log.iter().all(|l| l.truncated));
    assert_eq!(out.tool_calls_log.len(), 1);
}

#[test]
fn retrieval_caps_evidence_and_reports_empty_hits_honestly() {
    let store = Store::open_in_memory().unwrap();
    assert!(retrieval_evidence(&store, "all", "missing", None)
        .unwrap()
        .contains("未找到"));
    let evidence = retrieval_evidence(&store, "all", "Rust", Some(&"Rust ".repeat(4000))).unwrap();
    assert!(evidence.chars().count() <= 8000);
}

#[tokio::test]
async fn fallback_replaces_only_structurally_pinned_report_seed() {
    use rustrss_core::ai::chat_agent::prepare_local_retrieval;
    use rustrss_core::ai::chat_session::prepare_chat_turn;
    let server = MockServer::start().await;
    let c = client(Provider::OpenAiCompatible, &server.uri());
    let store = Store::open_in_memory().unwrap();
    let manifest = store.freeze_manifest(1791072000, 1791122400, None).unwrap();
    let report = format!("{}\n\nRust frozen chapter", "unrelated ".repeat(1000));
    store
        .commit_digest_report(
            "2026-10-04",
            "UTC",
            1791072000,
            1791122400,
            "all",
            "{}",
            "p",
            "{}",
            &manifest.hash,
            &manifest.pairs_hash,
            &[],
            manifest.frozen_at,
            0,
            0,
            "{\"overview\":\"Rust\",\"sections\":[]}",
            &report,
            1,
            "{}",
            0,
            &manifest.entries,
        )
        .unwrap();
    let turn =
        prepare_chat_turn(&store, &c, None, Some("2026-10-04"), Some("all"), "Rust").unwrap();
    assert!(turn.frozen_report.is_some());
    let sid = turn.session_id;
    let frozen = turn.frozen_report.clone();
    let n = Arc::new(AtomicUsize::new(0));
    let counter = n.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                assert!(body["messages"][1]["content"]
                    .as_str()
                    .unwrap()
                    .contains(&"unrelated ".repeat(100)));
                ResponseTemplate::new(400).set_body_string("This model does not support tools")
            } else {
                assert!(body.get("tools").is_none());
                assert_eq!(body["messages"][1]["content"], "Rust");
                let evidence = body["messages"][2]["content"].as_str().unwrap();
                assert!(evidence.contains("Rust frozen chapter"));
                assert!(!evidence.contains("unrelated"));
                assert!(evidence.chars().count() < 8000);
                ResponseTemplate::new(200).set_body_json(final_response(Provider::OpenAiCompatible))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let out = run_agent_turn(
        &c,
        turn.request,
        |_, _| panic!(),
        |req| {
            prepare_local_retrieval(
                &store,
                &turn.scope_key,
                "Rust",
                turn.frozen_report.as_deref(),
                req,
            )
        },
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert!(out.degraded);
    let session = store.get_session(sid).unwrap().unwrap();
    assert!(session.session.scope_json.contains("\"has_seed\":true"));
    assert_eq!(
        serde_json::from_str::<Vec<ChatBlock>>(&session.messages[0].parts_json).unwrap(),
        vec![ChatBlock::Text(frozen.unwrap())]
    );
    // Even identical user text survives; seed is not guessed by text matching.
    let mut request = request();
    request.messages = vec![
        ChatMessage {
            role: ChatRole::User,
            blocks: vec![ChatBlock::Text("Rust identical".into())]
        };
        2
    ];
    prepare_local_retrieval(&store, "all", "Rust", Some("Rust identical"), &mut request).unwrap();
    assert_eq!(request.messages.len(), 1);
    assert_eq!(
        request.messages[0].blocks,
        vec![ChatBlock::Text("Rust identical".into())]
    );
}

#[tokio::test]
async fn prepared_session_scope_is_injected_into_real_agent_callback() {
    use rustrss_core::ai::chat_session::prepare_chat_turn;
    use rustrss_core::ai::tools::run_scoped_tool;
    let store = Store::open_in_memory().unwrap();
    let a = store
        .add_feed("https://a.invalid", Some("allowed"))
        .unwrap();
    let b = store.add_feed("https://b.invalid", Some("secret")).unwrap();
    let tag = store.create_tag("scope", None).unwrap().id;
    store.set_feed_tags(a, &[tag]).unwrap();
    for (feed, title) in [(a, "allowed"), (b, "secret")] {
        let xml=format!("<rss version='2.0'><channel><title>T</title><item><guid>{title}</guid><title>Rust {title}</title><description>Rust {title} body</description></item></channel></rss>");
        store
            .upsert_entries(feed, &rustrss_core::parse(xml.as_bytes()).unwrap().entries)
            .unwrap();
    }
    let bid = store
        .list_entries(&rustrss_core::EntryQuery {
            feed_id: Some(b),
            ..Default::default()
        })
        .unwrap()[0]
        .id;
    for scope in ["all".to_string(), format!("tags:{tag}")] {
        let restricted = scope != "all";
        let server = MockServer::start().await;
        let c = client(Provider::OpenAiCompatible, &server.uri());
        let turn = prepare_chat_turn(&store, &c, None, None, Some(&scope), "Rust").unwrap();
        let n = Arc::new(AtomicUsize::new(0));
        let counter = n.clone();
        Mock::given(method("POST"))
            .respond_with(move |req: &Request| {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(200).set_body_json(call_response(
                        Provider::OpenAiCompatible,
                        &[
                            ("search_articles", json!({"query":"Rust"})),
                            ("get_article", json!({"id":bid})),
                        ],
                    ))
                } else {
                    let body: Value = serde_json::from_slice(&req.body).unwrap();
                    let data: Value =
                        serde_json::from_str(body["messages"][3]["content"].as_str().unwrap())
                            .unwrap();
                    assert_eq!(data["count"], if restricted { 1 } else { 2 });
                    let bodytext = body.to_string();
                    assert_eq!(bodytext.contains("Rust secret body"), !restricted);
                    ResponseTemplate::new(200)
                        .set_body_json(final_response(Provider::OpenAiCompatible))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let out = run_agent_turn(
            &c,
            turn.request,
            |name, args| run_scoped_tool(&store, &turn.scope_key, name, args),
            |_| panic!(),
            || false,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(out.tool_calls_log[1].ok, !restricted);
        assert!(!store.get_entry(bid).unwrap().unwrap().read);
    }
}

#[tokio::test]
async fn agent_timeout_does_not_poison_capability_cache() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(final_response(Provider::OpenAiCompatible))
                .set_delay(std::time::Duration::from_secs(1)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client(Provider::OpenAiCompatible, &server.uri());
    let mut request = request();
    request.limits.timeout = std::time::Duration::from_millis(30);
    let err = run_agent_turn(&c, request, |_, _| panic!(), |_| panic!(), || false, |_| {})
        .await
        .unwrap_err();
    assert!(err.to_string().contains("超时"));
    assert_eq!(
        chat_capability(
            c.config().provider,
            &c.config().model,
            &rustrss_core::ai::digest::endpoint_identity(&c)
        ),
        ChatCapability::Unknown
    );
}

#[tokio::test]
async fn repeated_query_in_one_batch_stops_before_duplicate_callback() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(call_response(
            Provider::OpenAiCompatible,
            &[("db_stats", json!({})), ("db_stats", json!({}))],
        )))
        .expect(1)
        .mount(&server)
        .await;
    let mut calls = 0;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| {
            calls += 1;
            output()
        },
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(calls, 1);
    assert!(matches!(&out.final_blocks[0],ChatBlock::Text(s) if s.contains("重复")));
}

#[tokio::test]
async fn cumulative_evidence_budget_truncates_last_result_before_feedback() {
    let server = MockServer::start().await;
    let n = Arc::new(AtomicUsize::new(0));
    let counter = n.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let round = counter.fetch_add(1, Ordering::SeqCst);
            if round == 0 {
                let calls: Vec<_> = (0..5)
                    .map(|i| ("list_articles", json!({"limit":i+1})))
                    .collect();
                ResponseTemplate::new(200)
                    .set_body_json(call_response(Provider::OpenAiCompatible, &calls))
            } else {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                let results: Vec<_> = body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|m| m["role"] == "tool")
                    .collect();
                assert_eq!(results.len(), 5);
                let mut bytes = 0;
                for result in &results {
                    let data = result["content"].as_str().unwrap();
                    assert!(data.len() <= TOOL_BYTES);
                    bytes += data.len();
                }
                assert!(bytes <= 48 * 1024);
                assert_eq!(
                    serde_json::from_str::<Value>(results[4]["content"].as_str().unwrap()).unwrap()
                        ["truncated"],
                    true
                );
                ResponseTemplate::new(200).set_body_json(final_response(Provider::OpenAiCompatible))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    // Multibyte evidence keeps character context under its independent 48k gate.
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| {
            Ok(ToolOutput {
                data_json: json!({"text":"界".repeat(3900)}).to_string(),
                truncated: false,
            })
        },
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(out.tool_calls_log.len(), 5);
    assert!(out.tool_calls_log.last().unwrap().truncated);
}

#[tokio::test]
async fn usage_unknown_is_not_invented_and_output_limit_is_hard_clamped() {
    let server = MockServer::start().await;
    let n = Arc::new(AtomicUsize::new(0));
    let counter = n.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(body["max_tokens"], 4096);
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(200).set_body_json(call_response(
                    Provider::OpenAiCompatible,
                    &[("db_stats", json!({}))],
                ))
            } else {
                let mut value = final_response(Provider::OpenAiCompatible);
                value.as_object_mut().unwrap().remove("usage");
                ResponseTemplate::new(200).set_body_json(value)
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let mut req = request();
    req.limits.max_output_tokens = 99999;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        req,
        |_, _| output(),
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(out.usage.input_tokens, None);
    assert_eq!(out.usage.output_tokens, None);
    assert!(out.usage_unknown);
}

#[tokio::test]
async fn all_providers_explicit_capability_error_falls_back_without_tools() {
    for (provider, phrase) in [
        (
            Provider::OpenAiCompatible,
            "This model does not support tools",
        ),
        (Provider::Ollama, "model does not support tools"),
        (Provider::Anthropic, "tool use is not supported"),
        (Provider::Gemini, "function calling is not supported"),
    ] {
        let server = MockServer::start().await;
        let n = Arc::new(AtomicUsize::new(0));
        let counter = n.clone();
        Mock::given(method("POST"))
            .respond_with(move |req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    assert!(body.get("tools").is_some());
                    ResponseTemplate::new(400).set_body_json(json!({"error":{"message":phrase}}))
                } else {
                    assert!(body.get("tools").is_none());
                    assert!(body.to_string().contains("local FTS evidence"));
                    ResponseTemplate::new(200).set_body_json(final_response(provider))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let out = run_agent_turn(
            &client(provider, &server.uri()),
            request(),
            |_, _| panic!(),
            |_| Ok("local FTS evidence".into()),
            || false,
            |_| {},
        )
        .await
        .unwrap();
        assert!(out.degraded);
        assert_eq!(out.final_blocks, vec![ChatBlock::Text("answer".into())]);
    }
}

#[tokio::test]
async fn known_turn_usage_fuse_keeps_provider_billable_totals() {
    let server = MockServer::start().await;
    let mut value = call_response(Provider::OpenAiCompatible, &[("db_stats", json!({}))]);
    value["usage"] = json!({"prompt_tokens":32000,"completion_tokens":1});
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .expect(1)
        .mount(&server)
        .await;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| panic!(),
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(out.usage.input_tokens, Some(32000));
    assert_eq!(out.usage.output_tokens, Some(1));
    assert!(out.tool_calls_log.is_empty());
    assert!(matches!(&out.final_blocks[0],ChatBlock::Text(t) if t.contains("token 预算")));
}

#[test]
fn fallback_retrieval_scope_snippet_counts_and_total_chars_are_bounded() {
    let store = Store::open_in_memory().unwrap();
    let allowed = store
        .add_feed("https://allowed.invalid", Some("A"))
        .unwrap();
    let denied = store.add_feed("https://denied.invalid", Some("B")).unwrap();
    let tag = store.create_tag("scope", None).unwrap().id;
    store.set_feed_tags(allowed, &[tag]).unwrap();
    for (feed, prefix) in [(allowed, "allowed"), (denied, "secret")] {
        let mut xml = "<rss version='2.0'><channel><title>T</title>".to_string();
        for i in 0..12 {
            xml.push_str(&format!("<item><guid>{prefix}{i}</guid><title>Rust {prefix}{i}</title><description>Rust {prefix}{}</description></item>", "界".repeat(2000)));
        }
        xml.push_str("</channel></rss>");
        store
            .upsert_entries(feed, &rustrss_core::parse(xml.as_bytes()).unwrap().entries)
            .unwrap();
    }
    let report = (0..3)
        .map(|i| format!("Rust chapter{i} {}", "报".repeat(1500)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let evidence =
        retrieval_evidence(&store, &format!("tags:{tag}"), "Rust", Some(&report)).unwrap();
    assert!(evidence.chars().count() <= 8000);
    assert!(!evidence.contains("secret"));
    assert!(evidence.matches("[id=").count() <= 10);
    assert_eq!(evidence.matches("[正文片段").count(), 3);
    assert!(evidence.contains("已截断"));
}

async fn nonadjacent_duplicate_query(batch: bool) {
    let server = MockServer::start().await;
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    Mock::given(method("POST"))
        .respond_with(move |_: &Request| {
            let round = counter.fetch_add(1, Ordering::SeqCst);
            let calls = if batch {
                vec![
                    ("search_articles", json!({"query":"A","limit":10})),
                    ("search_articles", json!({"query":"B"})),
                    ("search_articles", json!({"limit":10,"query":"A"})),
                ]
            } else {
                vec![(
                    "search_articles",
                    json!({"query":if round == 1 { "B" } else { "A" }}),
                )]
            };
            let mut response = call_response(Provider::OpenAiCompatible, &calls);
            if batch {
                response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] =
                    json!(r#"{ "query": "A", "limit": 10 }"#);
                response["choices"][0]["message"]["tool_calls"][2]["function"]["arguments"] =
                    json!(r#"{"limit":10,"query":"A"}"#);
            }
            // New explanatory text must not bypass duplicate suppression.
            response["choices"][0]["message"]["content"] = json!(format!("round {round}"));
            ResponseTemplate::new(200).set_body_json(response)
        })
        .mount(&server)
        .await;
    let mut a_executions = 0;
    let mut b_executions = 0;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, args| {
            match args["query"].as_str().unwrap() {
                "A" => a_executions += 1,
                "B" => b_executions += 1,
                _ => unreachable!(),
            }
            output()
        },
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(a_executions, 1, "A must never execute on the retry path");
    assert_eq!(b_executions, 1);
    assert_eq!(requests.load(Ordering::SeqCst), if batch { 1 } else { 3 });
    assert!(matches!(&out.final_blocks[0], ChatBlock::Text(t) if t.contains("重复查询")));
}

#[tokio::test]
async fn nonadjacent_duplicate_in_one_batch_is_not_executed() {
    nonadjacent_duplicate_query(true).await;
}

#[tokio::test]
async fn nonadjacent_duplicate_across_rounds_is_not_executed() {
    nonadjacent_duplicate_query(false).await;
}

#[tokio::test]
async fn unknown_usage_then_known_lower_bound_exceeding_budget_stops() {
    let server = MockServer::start().await;
    let counter = Arc::new(AtomicUsize::new(0));
    let n = counter.clone();
    Mock::given(method("POST"))
        .respond_with(move |_: &Request| {
            let round = n.fetch_add(1, Ordering::SeqCst);
            let mut value = call_response(
                Provider::OpenAiCompatible,
                &[("search_articles", json!({"query":format!("round{round}")}))],
            );
            if round == 0 {
                value.as_object_mut().unwrap().remove("usage");
            } else {
                value["usage"] = json!({"prompt_tokens":16000,"completion_tokens":1});
            }
            ResponseTemplate::new(200).set_body_json(value)
        })
        .mount(&server)
        .await;
    let mut tools = 0;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| {
            tools += 1;
            output()
        },
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 3);
    assert_eq!(tools, 2);
    assert_eq!(out.usage.input_tokens, None);
    assert_eq!(out.usage.output_tokens, None);
    assert!(out.usage_unknown);
    assert!(matches!(&out.final_blocks[0], ChatBlock::Text(t) if t.contains("token 预算")));
}

/// 已截断结果被再包装时，scope_feed_count 必须提升到顶层（评审 P2：嵌套截断
/// 丢失范围元数据的路径）。工具直接返回一个已带 truncated 信封的大结果。
#[tokio::test]
async fn nested_truncation_promotes_scope_count_to_top_level() {
    let server = MockServer::start().await;
    let big_inner = json!({
        "truncated": true,
        "scope_feed_count": 7,
        "excerpt": "长".repeat(6000),
    });
    let n = Arc::new(AtomicUsize::new(0));
    let counter = n.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let round = counter.fetch_add(1, Ordering::SeqCst);
            if round == 0 {
                let calls = vec![("search_articles", json!({"query":"q"}))];
                ResponseTemplate::new(200)
                    .set_body_json(call_response(Provider::OpenAiCompatible, &calls))
            } else {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                let tool_msg = body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["role"] == "tool")
                    .unwrap();
                let data = tool_msg["content"].as_str().unwrap();
                let parsed: Value = serde_json::from_str(data).unwrap();
                // 顶层必须直接可见范围计数（不要求模型深入嵌套 data）
                assert_eq!(parsed["scope_feed_count"], 7, "顶层保留范围计数：{parsed}");
                assert_eq!(parsed["truncated"], true);
                ResponseTemplate::new(200).set_body_json(final_response(Provider::OpenAiCompatible))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let out = run_agent_turn(
        &client(Provider::OpenAiCompatible, &server.uri()),
        request(),
        |_, _| {
            Ok(ToolOutput {
                data_json: big_inner.to_string(),
                truncated: true,
            })
        },
        |_| panic!(),
        || false,
        |_| {},
    )
    .await
    .unwrap();
    assert!(!out.degraded);
}

fn as_stream(provider: Provider, response: Value) -> String {
    let data = |v: Value| format!("data: {v}\n\n");
    match provider {
        Provider::OpenAiCompatible => {
            let mut delta = response["choices"][0]["message"].clone();
            if let Some(calls) = delta["tool_calls"].as_array_mut() {
                for (index, call) in calls.iter_mut().enumerate() {
                    call["index"] = json!(index);
                }
            }
            format!(
                "{}data: [DONE]\n\n",
                data(
                    json!({"choices":[{"delta":delta,"finish_reason":response["choices"][0]["finish_reason"]}],"usage":response["usage"]})
                )
            )
        }
        Provider::Anthropic => {
            let mut frames =
                data(json!({"type":"message_start","message":{"usage":response["usage"]}}));
            for (index, block) in response["content"].as_array().unwrap().iter().enumerate() {
                frames.push_str(&data(
                    json!({"type":"content_block_start","index":index,"content_block":block}),
                ));
            }
            frames.push_str(&data(json!({"type":"message_delta","delta":{"stop_reason":response["stop_reason"]},"usage":response["usage"]})));
            frames.push_str(&data(json!({"type":"message_stop"})));
            frames
        }
        Provider::Gemini => data(response),
        Provider::Ollama => format!("{response}\n"),
    }
}
async fn streaming_agent_tools(provider: Provider) {
    use rustrss_core::ai::chat_agent::run_agent_turn_streaming;
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    Mock::given(method("POST"))
        .respond_with(move |req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            let response = if c.fetch_add(1, Ordering::SeqCst) == 0 {
                call_response(
                    provider,
                    &[
                        ("search_articles", json!({"query":"Rust"})),
                        ("list_tags", json!({})),
                    ],
                )
            } else {
                assert!(body.to_string().contains("articles"));
                if provider == Provider::Gemini {
                    assert!(body.to_string().contains("signature0"));
                }
                final_response(provider)
            };
            ResponseTemplate::new(200).set_body_string(as_stream(provider, response))
        })
        .expect(2)
        .mount(&server)
        .await;
    let mut text = String::new();
    let mut logs = vec![];
    let outcome = run_agent_turn_streaming(
        &client(provider, &server.uri()),
        request(),
        |_, _| output(),
        |_| panic!("fallback"),
        || false,
        |log| logs.push(log.name.clone()),
        |s| text.push_str(s),
    )
    .await
    .unwrap();
    assert_eq!(text, "answer");
    assert_eq!(outcome.final_blocks, vec![ChatBlock::Text(text)]);
    assert_eq!(logs, ["search_articles", "list_tags"]);
    assert_eq!(outcome.tool_calls_log.len(), 2);
    assert_eq!(outcome.usage.input_tokens, Some(6));
    assert_eq!(outcome.usage.output_tokens, Some(4));
}
#[tokio::test]
async fn openai_streaming_agent_assembles_tools_then_streams_answer() {
    streaming_agent_tools(Provider::OpenAiCompatible).await;
}
#[tokio::test]
async fn anthropic_streaming_agent_assembles_tools_then_streams_answer() {
    streaming_agent_tools(Provider::Anthropic).await;
}
#[tokio::test]
async fn gemini_streaming_agent_assembles_tools_then_streams_answer() {
    streaming_agent_tools(Provider::Gemini).await;
}
#[tokio::test]
async fn ollama_streaming_agent_assembles_tools_then_streams_answer() {
    streaming_agent_tools(Provider::Ollama).await;
}
