//! HTTP 传输测试：鉴权必须真的生效，且对 token 的客户端能完成一次真实调用。
//!
//! 用本地测试库 + `127.0.0.1:0`（端口交给系统分配），不依赖外网。

use rustrss_core::{Entry, IdOrigin, Store};
use rustrss_mcp::http::{generate_token, serve, HttpConfig};
use rustrss_mcp::RustRssMcp;
use serde_json::{json, Value};

fn seeded(tag: &str) -> (RustRssMcp, std::path::PathBuf) {
    let mut path = std::env::temp_dir();
    path.push(format!("rustrss-mcp-http-{tag}-{}.sqlite", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let store = Store::open(&path).expect("建库失败");
    let feed_id = store
        .add_feed("https://example.com/feed.xml", Some("示例源"))
        .expect("加源失败");
    store
        .upsert_entries(
            feed_id,
            &[Entry {
                stable_id: "a1".into(),
                id_origin: IdOrigin::SourceData,
                source_id: "a1".into(),
                title: "HTTP 传输测试文章".into(),
                url: Some("https://example.com/a1".into()),
                author: None,
                published: None,
                updated: None,
                summary: Some("摘要".into()),
                content_html: None,
                content_text: Some("正文".into()),
                categories: Vec::new(),
            }],
        )
        .expect("入库失败");
    (RustRssMcp::open(&path).expect("打开库失败"), path)
}

fn mcp_body(method: &str, id: u32) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} })
}

/// MCP 传输规范要求客户端声明同时接受这两种类型；rmcp 不对则返回 406。
const ACCEPT_BOTH: &str = "application/json, text/event-stream";

#[tokio::test]
async fn rejects_requests_without_a_valid_token() {
    let (server, db) = seeded("auth");
    let token = generate_token();
    let handle = serve(
        server,
        HttpConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            token: Some(token.clone()),
        },
    )
    .await
    .expect("HTTP 服务应能启动");

    let base = format!("http://{}", handle.addr);
    let client = reqwest::Client::new();

    // /health 不鉴权（不含任何订阅数据）
    let health = client
        .get(format!("{base}/health"))
        .send()
        .await
        .expect("health 请求失败");
    assert_eq!(health.status(), 200);

    // 无 token → 401（鉴权发生在 MCP 层之前，所以请求体与否不影响结论）
    let no_token = client
        .post(format!("{base}/mcp"))
        .header("accept", ACCEPT_BOTH)
        .json(&mcp_body("tools/list", 1))
        .send()
        .await
        .expect("请求失败");
    assert_eq!(no_token.status(), 401, "无 token 必须被拒");

    // 错误 token → 401
    let wrong = client
        .post(format!("{base}/mcp?token=wrong-token"))
        .header("accept", ACCEPT_BOTH)
        .json(&mcp_body("tools/list", 1))
        .send()
        .await
        .expect("请求失败");
    assert_eq!(wrong.status(), 401, "错误 token 必须被拒");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn accepts_initialize_and_serves_real_data_with_both_token_styles() {
    let (server, db) = seeded("serve");
    let token = generate_token();
    let handle = serve(
        server,
        HttpConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            token: Some(token.clone()),
        },
    )
    .await
    .expect("HTTP 服务应能启动");
    let base = format!("http://{}", handle.addr);
    let client = reqwest::Client::new();

    // ① 查询串携带 token（客户端不方便设头时用这种）
    let init = client
        .post(format!("{base}/mcp?token={token}"))
        .header("accept", ACCEPT_BOTH)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2026-07-28",
                "capabilities": {},
                "clientInfo": { "name": "rustrss-http-test", "version": "0.0.0" }
            }
        }))
        .send()
        .await
        .expect("initialize 请求失败");
    assert_eq!(init.status(), 200, "正确 token 应被接受");
    let init_body: Value = init.json().await.expect("initialize 应为 JSON");
    assert_eq!(
        init_body["result"]["serverInfo"]["name"], "rustrss",
        "应返回服务器信息: {init_body}"
    );

    // ② Authorization 头（标准做法）
    let tools = client
        .post(format!("{base}/mcp"))
        .header("accept", ACCEPT_BOTH)
        .bearer_auth(&token)
        .json(&mcp_body("tools/list", 2))
        .send()
        .await
        .expect("tools/list 请求失败");
    assert_eq!(tools.status(), 200);
    let tools_body: Value = tools.json().await.expect("tools/list 应为 JSON");
    let names: Vec<String> = tools_body["result"]["tools"]
        .as_array()
        .expect("应有 tools 数组")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    assert!(names.contains(&"list_feeds".to_string()), "{names:?}");

    // ③ 真的取一次数据，确认这条路走的是同一个库
    let call = client
        .post(format!("{base}/mcp?token={token}"))
        .header("accept", ACCEPT_BOTH)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "list_feeds", "arguments": {} }
        }))
        .send()
        .await
        .expect("tools/call 请求失败");
    assert_eq!(call.status(), 200);
    let call_body: Value = call.json().await.expect("tools/call 应为 JSON");
    let text = call_body["result"]["content"][0]["text"]
        .as_str()
        .expect("应返回文本内容");
    assert!(text.contains("示例源"), "应取到真实数据: {text}");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn refuses_to_bind_a_non_loopback_address() {
    let (server, db) = seeded("bind");
    let err = serve(
        server,
        HttpConfig {
            bind: "0.0.0.0:0".parse().unwrap(),
            token: Some(generate_token()),
        },
    )
    .await
    .expect_err("非回环地址必须被拒绝");
    assert!(err.to_string().contains("回环"), "{err}");
    let _ = std::fs::remove_file(db);
}
