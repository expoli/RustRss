//! T3 写工具的端到端测试：真 HTTP（MCP streamable HTTP 传输）+ 真库 + 写 token。
//!
//! 覆盖：
//! - `set_read` / `set_starred` / `set_read_later`：ids 形态与条件级形态、逐项结果、
//!   幂等、写后 `db_stats` 与「另一个连接（模拟界面）回读」一致；
//! - `refresh`：三类 scope、与界面**共用**同一个单 flight（占住 gate 时返回 `rate_limited`
//!   且一个请求都不发）、本轮摘要；
//! - `fetch_fulltext`：正常路径写回 + 幂等（零网络）+ 体积超限 / 反爬质询 / 非 HTML /
//!   条目不存在 / 无原文地址的错误码；
//! - 授权：读 token 与关开关时三者都不可用且错误码正确；
//! - 审计：每次调用一行 `target=mcp`，参数摘要过 scrub。
//!
//! 网络部分全部打在本机起的极简 HTTP 服务器上（`127.0.0.1:0`），不依赖外网。

use std::sync::{Arc, Mutex, OnceLock};

use rustrss_core::{Entry, EntryQuery, IdOrigin, Store};
use rustrss_mcp::config;
use rustrss_mcp::http::{serve, HttpConfig, HttpHandle};
use rustrss_mcp::write_tools::{
    ERROR_ARTICLE_NOT_FOUND, ERROR_FEED_NOT_FOUND, ERROR_FOLDER_NOT_FOUND,
    ERROR_FULLTEXT_BOT_CHALLENGE, ERROR_FULLTEXT_NOT_HTML, ERROR_FULLTEXT_TOO_LARGE,
    ERROR_INVALID_URL, ERROR_RATE_LIMITED,
};
use rustrss_mcp::RustRssMcp;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ACCEPT_BOTH: &str = "application/json, text/event-stream";
const READ_TOKEN: &str = "read-token";
const WRITE_TOKEN: &str = "write-token";

/// 真页面 fixture（与 core 的全文测试共用同一份，避免"测试专用 HTML 恰好能提取"的自欺）
const ARTICLE_HTML: &str =
    include_str!("../../rustrss-core/tests/fixtures/fulltext/typical-article.html");

// ---------------------------------------------------------------- 极简 HTTP 测试服务器

/// 按路径返回预置响应的最小 HTTP/1.1 服务器（tokio 裸 TCP，不引新依赖）。
struct TestServer {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    /// `routes`：(路径, 响应体, content-type)
    async fn start(routes: Vec<(String, Vec<u8>, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("测试端口应可监听");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let routes = routes.clone();
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move {
                    // 请求头读完就够（工具只发 GET）
                    let mut buf: Vec<u8> = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while buf.len() < 64 * 1024 {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&chunk[..n]);
                                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                        }
                    }
                    let head = String::from_utf8_lossy(&buf).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                    recorded.lock().expect("命中锁被毒化").push(path.clone());

                    let response: Vec<u8> = match routes.iter().find(|(p, _, _)| *p == path) {
                        Some((_, body, ctype)) => {
                            let head = format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                body.len()
                            );
                            let mut out = head.into_bytes();

                            out.extend_from_slice(body);
                            out
                        }
                        None => b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                            .to_vec(),
                    };
                    let _ = sock.write_all(&response).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { base, hits }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn hits(&self) -> Vec<String> {
        self.hits.lock().expect("命中锁被毒化").clone()
    }
}

/// 最小 RSS 2.0 源（`items` 条）
fn feed_xml(guid_prefix: &str, items: usize) -> Vec<u8> {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>测试源</title><link>http://127.0.0.1/</link><description>测试</description>"#,
    );
    for i in 1..=items {
        out.push_str(&format!(
            "<item><title>新条目 {guid_prefix}-{i}</title><link>http://127.0.0.1/p/{guid_prefix}-{i}</link><guid>{guid_prefix}-{i}</guid><description>正文 {guid_prefix}-{i}</description><pubDate>Wed, 23 Sep 2026 0{i}:00:00 GMT</pubDate></item>"
        ));
    }
    out.push_str("</channel></rss>");
    out.into_bytes()
}

/// 反爬质询页（core 的检测特征）
const CHALLENGE_HTML: &str =
    "<html><head><title>Making sure you're not a bot</title></head><body>Verifying…</body></html>";

// ---------------------------------------------------------------- 库与 MCP 服务夹具

fn temp_db(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "rustrss-mcp-write-tools-{tag}-{}.sqlite",
        std::process::id()
    ));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
    }
    p
}

fn entry(stable_id: &str, url: Option<String>, text: &str) -> Entry {
    Entry {
        stable_id: stable_id.to_string(),
        id_origin: IdOrigin::SourceData,
        source_id: stable_id.to_string(),
        title: format!("条目 {stable_id}"),
        url,
        author: None,
        published: None,
        updated: None,
        summary: Some(text.to_string()),
        content_html: None,
        content_text: Some(text.to_string()),
        categories: Vec::new(),
    }
}

/// 建库：2 个源（每源 2 条），第 2 个源放进分组；返回 (库路径, 源 id, 分组 id)
fn seeded_db(tag: &str) -> (std::path::PathBuf, i64, i64, i64) {
    let path = temp_db(tag);
    let store = Store::open(&path).expect("建库失败");
    let feed_a = store
        .add_feed("http://127.0.0.1-未使用/a.xml", Some("A源"))
        .expect("加源失败");
    let feed_b = store
        .add_feed("http://127.0.0.1-未使用/b.xml", Some("B源"))
        .expect("加源失败");
    store
        .upsert_entries(
            feed_a,
            &[
                entry("a1", Some("https://example.com/a1".into()), "正文一"),
                entry("a2", Some("https://example.com/a2".into()), "正文二"),
            ],
        )
        .expect("入库失败");
    store
        .upsert_entries(
            feed_b,
            &[
                entry("b1", Some("https://example.com/b1".into()), "正文三"),
                entry("b2", Some("https://example.com/b2".into()), "正文四"),
            ],
        )
        .expect("入库失败");
    let folder = store.add_folder("技术").expect("建分组失败");
    store.assign_folder(feed_b, Some(folder)).expect("归组失败");
    (path, feed_a, feed_b, folder)
}

/// 打开写能力：读 token + 写 token + 两个开关
fn license(path: &std::path::Path) {
    let store = Store::open(path).expect("开库失败");
    config::set_token(&store, READ_TOKEN).expect("写读 token 失败");
    config::set_write_token(&store, WRITE_TOKEN).expect("写写 token 失败");
    store
        .set_bool_setting(config::K_WRITE_ENABLED, true)
        .expect("开写开关失败");
    store
        .set_bool_setting(config::K_DANGEROUS_ENABLED, true)
        .expect("开危险开关失败");
}

fn entry_ids(path: &std::path::Path) -> Vec<i64> {
    let store = Store::open(path).expect("开库失败");
    store
        .list_entries(&EntryQuery::default())
        .expect("读条目失败")
        .iter()
        .map(|r| r.id)
        .collect()
}

fn id_of(path: &std::path::Path, stable_id: &str) -> i64 {
    let store = Store::open(path).expect("开库失败");
    store
        .list_entries(&EntryQuery::default())
        .expect("读条目失败")
        .into_iter()
        .find(|r| r.stable_id == stable_id)
        .unwrap_or_else(|| panic!("{stable_id} 不在库里"))
        .id
}

/// 拉起 MCP 的 HTTP 服务（应用内托管的形状：不持静态 token，只认库里的值）
async fn start_mcp(
    path: &std::path::Path,
    gate: Option<Arc<rustrss_core::RefreshGate>>,
) -> HttpHandle {
    let mut server = RustRssMcp::open(path).expect("打开库失败");
    if let Some(gate) = gate {
        server = server.with_refresh_gate(gate);
    }
    serve(
        server,
        HttpConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            token: None,
        },
    )
    .await
    .expect("HTTP 服务应能启动")
}

// ---------------------------------------------------------------- MCP 客户端小工具

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn initialize(client: &reqwest::Client, base: &str, token: &str) {
    let response = client
        .post(format!("{base}/mcp"))
        .header("accept", ACCEPT_BOTH)
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2026-07-28",
                "capabilities": {},
                "clientInfo": { "name": "write-tools-test", "version": "0.0.0" }
            }
        }))
        .send()
        .await
        .expect("initialize 请求失败");
    assert_eq!(response.status().as_u16(), 200, "握手应成功");
}

/// 调工具；返回 (HTTP 状态, isError, 文本体解析后的 JSON)
async fn call_tool(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    args: Value,
) -> (u16, bool, Value) {
    let response = client
        .post(format!("{base}/mcp"))
        .header("accept", ACCEPT_BOTH)
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        }))
        .send()
        .await
        .expect("tools/call 请求失败");
    let status = response.status().as_u16();
    if status != 200 {
        return (status, false, Value::Null);
    }
    let body: Value = response.json().await.expect("tools/call 应为 JSON");
    let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    let parsed = serde_json::from_str(text).unwrap_or(Value::Null);
    (status, is_error, parsed)
}

async fn list_tool_names(client: &reqwest::Client, base: &str, token: &str) -> Vec<String> {
    let response = client
        .post(format!("{base}/mcp"))
        .header("accept", ACCEPT_BOTH)
        .bearer_auth(token)
        .json(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))
        .send()
        .await
        .expect("tools/list 请求失败");
    let body: Value = response.json().await.expect("tools/list 应为 JSON");
    body["result"]["tools"]
        .as_array()
        .expect("应有 tools 数组")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------- 阅读状态（AC1）

#[tokio::test]
async fn read_state_writes_land_in_the_database_and_agree_with_read_tools() {
    let (db, feed_a, _feed_b, _folder) = seeded_db("state");
    license(&db);
    let ids = entry_ids(&db);
    let handle = start_mcp(&db, None).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① ids 形态：逐项结果 + 命中条数
    let (status, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_read",
        json!({"ids": [ids[0], ids[1], 999_999]}),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!is_error, "{body}");
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["affected"], 2, "只有存在的两条被写: {body}");
    assert_eq!(
        body["results"][2]["error_code"], ERROR_ARTICLE_NOT_FOUND,
        "{body}"
    );

    // ② 读工具（读 token）看到的计数立刻一致
    let (_, _, stats) = call_tool(&http, &base, READ_TOKEN, "db_stats", json!({})).await;
    assert_eq!(stats["unread"], 2, "4 条里 2 条已读: {stats}");

    // ③ 另一个连接（界面的读路径）回读一致
    {
        let store = Store::open(&db).expect("开库失败");
        assert!(store.get_entry(ids[0]).unwrap().unwrap().read);
        assert!(store.get_entry(ids[1]).unwrap().unwrap().read);
        assert_eq!(store.unread_total().unwrap(), 2);
        assert_eq!(
            store.counts().unwrap().1,
            2,
            "counts() 的未读口径（界面侧栏用）"
        );
    }

    // ④ 幂等：重复调用 affected 稳定
    let (_, _, again) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_read",
        json!({"ids": [ids[0], ids[1]]}),
    )
    .await;
    assert_eq!(again["affected"], 2, "重复调用返回值稳定: {again}");

    // ⑤ 条件级形态：影响面与直查一致（先用 list_articles(since) 数一遍未读）
    let all = entry_ids(&db);
    let before = {
        let (_, _, listed) = call_tool(
            &http,
            &base,
            READ_TOKEN,
            "list_articles",
            json!({"feed_id": feed_a, "unread_only": true, "limit": 50}),
        )
        .await;
        listed["count"].as_i64().expect("count 应是数字")
    };
    assert_eq!(before, 2, "A 源还剩 2 条未读");
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_read",
        json!({"feed_id": feed_a}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(
        body["affected"], before,
        "条件级影响面必须与直查一致: {body}"
    );
    assert_eq!(body["detail"]["target"]["feed_id"], feed_a, "{body}");
    assert_eq!(
        body["results"].as_array().unwrap().len(),
        before as usize,
        "命中集在 100 条以内时逐项结果给全: {body}"
    );
    let (_, _, stats) = call_tool(&http, &base, READ_TOKEN, "db_stats", json!({})).await;
    assert_eq!(stats["unread"], 0, "条件级写入也要真的落库: {stats}");

    // ⑥ 条件级幂等（第二次命中数不变）
    let (_, _, again) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_read",
        json!({"feed_id": feed_a}),
    )
    .await;
    assert_eq!(again["affected"], before, "{again}");

    // ⑦ 条件级时间范围：since 在未来 → 一条都不命中，但不是错误
    let (_, _, empty) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_read",
        json!({"feed_id": feed_a, "since": 4_102_444_800_i64}),
    )
    .await;
    assert_eq!(empty["ok"], true, "{empty}");
    assert_eq!(empty["affected"], 0, "{empty}");

    // ⑧ 三个工具各管各的列
    let (_, _, starred) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_starred",
        json!({"ids": [all[0]], "starred": true}),
    )
    .await;
    assert_eq!(starred["affected"], 1, "{starred}");
    let (_, _, later) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_read_later",
        json!({"ids": [all[0]]}),
    )
    .await;
    assert_eq!(later["affected"], 1, "later 缺省 true: {later}");
    let (_, _, unstar) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_starred",
        json!({"ids": [all[0]], "starred": false}),
    )
    .await;
    assert_eq!(unstar["affected"], 1, "{unstar}");
    {
        let store = Store::open(&db).expect("开库失败");
        let row = store.get_entry(all[0]).unwrap().unwrap();
        assert!(!row.starred, "取消星标");
        assert!(row.read_later, "取消星标不该动稍后读");
        assert!(row.read, "取消星标不该动已读");
    }

    // ⑨ 入参校验错误码（不靠文案）
    for (args, label) in [
        (json!({"ids": []}), "空 ids"),
        (json!({"ids": (1..=101).collect::<Vec<i64>>()}), "超限"),
        (json!({"ids": [all[0]], "feed_id": feed_a}), "混用"),
        (json!({}), "什么都没给"),
    ] {
        let (_, is_error, body) = call_tool(&http, &base, WRITE_TOKEN, "set_read", args).await;
        assert_eq!(body["error_code"], "invalid_argument", "{label}: {body}");
        assert!(is_error, "{label} 应标 isError: {body}");
    }

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 刷新（AC2）

#[tokio::test]
async fn refresh_covers_three_scopes_and_shares_the_single_flight() {
    // 测试源服务器：两个源各一份 feed
    let server = TestServer::start(vec![
        (
            "/a.xml".to_string(),
            feed_xml("a", 2),
            "application/rss+xml".to_string(),
        ),
        (
            "/b.xml".to_string(),
            feed_xml("b", 1),
            "application/rss+xml".to_string(),
        ),
    ])
    .await;

    let (db, feed_a, feed_b, folder) = seeded_db("refresh");
    license(&db);
    {
        // 把两个源指向刚起的测试服务器（走 store 公开 API，不碰 raw SQL）
        let store = Store::open(&db).expect("开库失败");
        store
            .update_feed_url(feed_a, &server.url("/a.xml"))
            .expect("改 A 源地址失败");
        store
            .update_feed_url(feed_b, &server.url("/b.xml"))
            .expect("改 B 源地址失败");
    }

    // 与界面共用同一个 gate：这里由测试扮演"界面"
    let gate = Arc::new(rustrss_core::RefreshGate::new());
    let handle = start_mcp(&db, Some(Arc::clone(&gate))).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① 单 flight：界面正拿着 gate → 本轮不执行、不发任何请求、返回 rate_limited
    let held = gate.try_begin().expect("先占住单 flight");
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "refresh",
        json!({"scope": "all"}),
    )
    .await;
    assert!(is_error, "进行中要标 isError: {body}");
    assert_eq!(body["error_code"], ERROR_RATE_LIMITED, "{body}");
    assert_eq!(body["affected"], 0, "{body}");
    assert!(
        server.hits().is_empty(),
        "被单 flight 挡下的刷新一个请求都不该发: {:?}",
        server.hits()
    );
    drop(held);

    // ② scope=feed_ids：只刷这一批，返回本轮摘要
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "refresh",
        json!({"scope": "feed_ids", "feed_ids": [feed_a]}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["detail"]["scope"], "feed_ids", "{body}");
    assert_eq!(body["detail"]["feeds"], 1, "{body}");
    assert_eq!(body["detail"]["inserted"], 2, "A 源 2 条新条目: {body}");
    assert_eq!(body["affected"], 2, "affected = 本轮入库条目数: {body}");
    assert_eq!(body["detail"]["failure_count"], 0, "{body}");
    assert_eq!(server.hits(), vec!["/a.xml".to_string()], "只该抓 A 源");

    // ③ scope=folder：只刷组内的源（B）
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "refresh",
        json!({"scope": "folder", "folder_id": folder}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(
        body["detail"]["scope"],
        format!("folder:{folder}"),
        "{body}"
    );
    assert_eq!(body["detail"]["inserted"], 1, "B 源 1 条: {body}");
    assert!(
        server.hits().contains(&"/b.xml".to_string()),
        "{:?}",
        server.hits()
    );

    // ④ scope=all：两个源都刷；再刷一遍内容没变 → unchanged，affected 归 0
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "refresh",
        json!({"scope": "all"}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["detail"]["feeds"], 2, "{body}");
    assert_eq!(
        body["detail"]["inserted"], 0,
        "重复刷新不该重复入库: {body}"
    );
    assert_eq!(
        body["detail"]["unchanged"], 3,
        "A 源 2 条 + B 源 1 条都没变: {body}"
    );
    assert_eq!(body["affected"], 0, "{body}");

    // ⑤ 新条目真的进了库（与读工具一致）
    let (_, _, listed) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"feed_id": feed_a, "limit": 50}),
    )
    .await;
    assert_eq!(listed["count"], 4, "A 源原有 2 条 + 新抓 2 条: {listed}");

    // ⑥ 参数与范围错误码
    for (args, expected, label) in [
        (
            json!({"scope": "feed_ids", "feed_ids": [404]}),
            ERROR_FEED_NOT_FOUND,
            "不存在的源",
        ),
        (
            json!({"scope": "folder", "folder_id": 404}),
            ERROR_FOLDER_NOT_FOUND,
            "不存在的分组",
        ),
        (
            json!({"scope": "everything"}),
            "invalid_argument",
            "未知 scope",
        ),
        (
            json!({"scope": "all", "feed_ids": [feed_a]}),
            "invalid_argument",
            "矛盾参数",
        ),
    ] {
        let (_, is_error, body) = call_tool(&http, &base, WRITE_TOKEN, "refresh", args).await;
        assert_eq!(body["error_code"], expected, "{label}: {body}");
        assert!(is_error, "{label}: {body}");
    }

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 全文抓取（AC3）

#[tokio::test]
async fn fetch_fulltext_writes_back_and_reports_each_error_class() {
    let server = TestServer::start(vec![
        (
            "/article".to_string(),
            ARTICLE_HTML.as_bytes().to_vec(),
            "text/html; charset=utf-8".to_string(),
        ),
        (
            "/challenge".to_string(),
            CHALLENGE_HTML.as_bytes().to_vec(),
            "text/html".to_string(),
        ),
        (
            "/plain.txt".to_string(),
            b"just plain text, not html".to_vec(),
            "text/plain".to_string(),
        ),
        (
            "/huge".to_string(),
            vec![b'x'; rustrss_core::fulltext::MAX_BYTES + 1024 * 1024],
            "text/html".to_string(),
        ),
    ])
    .await;

    let db = temp_db("fulltext");
    {
        let store = Store::open(&db).expect("建库失败");
        let feed_id = store
            .add_feed("http://127.0.0.1-未使用/feed.xml", Some("示例博客"))
            .expect("加源失败");
        // 摘要型条目（正文很短 → needs_fulltext=true）
        let summary = |id: &str, url: Option<String>| {
            let mut e = entry(id, url, "只有一段很短的摘要");
            e.summary = Some("只有一段很短的摘要".to_string());
            e
        };
        store
            .upsert_entries(
                feed_id,
                &[
                    summary("s-article", Some(server.url("/article"))),
                    summary("s-challenge", Some(server.url("/challenge"))),
                    summary("s-plain", Some(server.url("/plain.txt"))),
                    summary("s-huge", Some(server.url("/huge"))),
                    summary("s-nourl", None),
                ],
            )
            .expect("入库失败");
    }
    license(&db);
    let handle = start_mcp(&db, None).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① 正常路径：抓取 → 提取 → 写回；正文用 get_article 单取（写工具不倒正文）
    let article_id = id_of(&db, "s-article");
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "fetch_fulltext",
        json!({"id": article_id}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["affected"], 1, "{body}");
    assert!(
        body["detail"]["chars"].as_i64().unwrap_or(0) > 500,
        "{body}"
    );
    assert!(body.get("text").is_none(), "写工具不随响应倒正文: {body}");

    let (_, _, article) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "get_article",
        json!({"id": article_id}),
    )
    .await;
    assert!(
        article["text"]
            .as_str()
            .unwrap_or_default()
            .contains("签名基的构造"),
        "写回后 get_article 应拿到提取出的正文: {}",
        article["text"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect::<String>()
    );
    {
        let store = Store::open(&db).expect("开库失败");
        let row = store.get_entry(article_id).unwrap().unwrap();
        assert!(!row.needs_fulltext, "写回后不再提示获取全文");
        assert!(row.content_text.unwrap_or_default().len() > 500);
    }

    // ② 幂等：第二次调用零网络（页面服务器的命中数不增加）
    let hits_before = server.hits();
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "fetch_fulltext",
        json!({"id": article_id}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["detail"]["already_fulltext"], true, "{body}");
    assert_eq!(body["affected"], 0, "幂等：没有新的写入: {body}");
    assert_eq!(
        server.hits(),
        hits_before,
        "已抓过的条目不得再发请求（零网络）"
    );

    // ③ 错误分类：反爬质询 / 非 HTML / 体积超限 / 条目不存在 / 没有原文地址
    for (stable_id, expected, label) in [
        ("s-challenge", ERROR_FULLTEXT_BOT_CHALLENGE, "反爬质询页"),
        ("s-plain", ERROR_FULLTEXT_NOT_HTML, "非 HTML"),
        ("s-huge", ERROR_FULLTEXT_TOO_LARGE, "超过 2MiB"),
        ("s-nourl", ERROR_INVALID_URL, "没有原文地址"),
    ] {
        let id = id_of(&db, stable_id);
        let (_, is_error, body) = call_tool(
            &http,
            &base,
            WRITE_TOKEN,
            "fetch_fulltext",
            json!({"id": id}),
        )
        .await;
        assert!(is_error, "{label} 应标 isError: {body}");
        assert_eq!(body["error_code"], expected, "{label}: {body}");
        assert!(
            body["error"].as_str().unwrap_or_default().chars().count() > 4,
            "{label} 要给人读得懂的说明: {body}"
        );
    }
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "fetch_fulltext",
        json!({"id": 999_999}),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error_code"], ERROR_ARTICLE_NOT_FOUND, "{body}");

    // ④ 失败路径不落库：库里这几条仍是摘要型
    {
        let store = Store::open(&db).expect("开库失败");
        // `s-nourl` 不在这个循环里：没有原文地址的条目本来就不是「待抓」（needs_fulltext=false），
        // 它的错误码断言在上面的循环里已经覆盖
        for stable_id in ["s-challenge", "s-plain", "s-huge"] {
            let id = id_of(&db, stable_id);
            assert!(
                store.get_entry(id).unwrap().unwrap().needs_fulltext,
                "{stable_id} 不该被写坏"
            );
        }
    }

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 授权（AC4 / AC6）

#[tokio::test]
async fn write_tools_are_unavailable_with_a_read_token_or_with_switches_off() {
    let (db, _feed_a, _feed_b, _folder) = seeded_db("auth");
    license(&db);
    let ids = entry_ids(&db);
    let handle = start_mcp(&db, None).await;
    let base = format!("http://{}", handle.addr);
    let http = client();

    // ① 读 token：列表里看不到写工具，直接调用一律 write_scope_required（不是 401）
    initialize(&http, &base, READ_TOKEN).await;
    let names = list_tool_names(&http, &base, READ_TOKEN).await;
    for name in [
        "set_read",
        "set_starred",
        "set_read_later",
        "refresh",
        "fetch_fulltext",
    ] {
        assert!(
            !names.contains(&name.to_string()),
            "读会话不该看到 {name}: {names:?}"
        );
    }
    for (tool, args) in [
        ("set_read", json!({"ids": [ids[0]]})),
        ("set_starred", json!({"ids": [ids[0]]})),
        ("set_read_later", json!({"ids": [ids[0]]})),
        ("refresh", json!({"scope": "all"})),
        ("fetch_fulltext", json!({"id": ids[0]})),
    ] {
        let (status, is_error, body) = call_tool(&http, &base, READ_TOKEN, tool, args).await;
        assert_eq!(status, 200, "{tool}：已认证但无授权 = 工具级错误");
        assert!(is_error, "{tool}: {body}");
        assert_eq!(body["error_code"], "write_scope_required", "{tool}: {body}");
    }
    {
        let store = Store::open(&db).expect("开库失败");
        assert_eq!(store.unread_total().unwrap(), 4, "被拒的调用不得改库");
    }

    // ② 开关关掉（写 token 还在）：write_disabled
    {
        let store = Store::open(&db).expect("开库失败");
        store
            .set_bool_setting(config::K_WRITE_ENABLED, false)
            .expect("关写开关失败");
    }
    let names = list_tool_names(&http, &base, WRITE_TOKEN).await;
    for name in ["set_read", "refresh", "fetch_fulltext"] {
        assert!(
            !names.contains(&name.to_string()),
            "开关关着不该列 {name}: {names:?}"
        );
    }
    for (tool, args) in [
        ("set_read", json!({"ids": [ids[0]]})),
        ("refresh", json!({"scope": "all"})),
        ("fetch_fulltext", json!({"id": ids[0]})),
    ] {
        let (_, is_error, body) = call_tool(&http, &base, WRITE_TOKEN, tool, args).await;
        assert!(is_error, "{tool}: {body}");
        assert_eq!(body["error_code"], "write_disabled", "{tool}: {body}");
    }

    // ③ 写 token 销毁后：下一个请求连认证都过不去（401），写能力随之消失
    {
        let store = Store::open(&db).expect("开库失败");
        config::clear_write_token(&store).expect("销毁写 token 失败");
        store
            .set_bool_setting(config::K_WRITE_ENABLED, true)
            .expect("开写开关失败");
    }
    let (status, _, _) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_read",
        json!({"ids": [ids[0]]}),
    )
    .await;
    assert_eq!(status, 401, "写 token 销毁后旧值立刻失效");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 审计（AC4）

/// 捕获日志的全局 logger（一个测试进程里只能装一次）
#[derive(Clone, Default)]
struct Capture {
    lines: Arc<Mutex<Vec<(String, String)>>>,
}

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        self.lines
            .lock()
            .expect("捕获锁被毒化")
            .push((record.target().to_string(), record.args().to_string()));
    }

    fn flush(&self) {}
}

fn install_capture() -> Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE
        .get_or_init(|| {
            let capture = Capture::default();
            let _ = log::set_boxed_logger(Box::new(capture.clone()));
            log::set_max_level(log::LevelFilter::Info);
            capture
        })
        .clone()
}

#[tokio::test]
async fn every_write_tool_call_leaves_a_scrubbed_audit_line() {
    let capture = install_capture();
    let (db, _feed_a, _feed_b, _folder) = seeded_db("audit");
    license(&db);
    let ids = entry_ids(&db);
    let handle = start_mcp(&db, None).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // 日志捕获是**进程级**的，同进程并行跑的其它用例也会写审计行。所以每次调用都在
    // 参数里塞一个本次独有的 `probe` 标记，只认领带自己标记的行（与 write_auth.rs
    // 用独立樁工具名同一个道理）。
    let probe = format!(
        "audit-probe-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    // set_read：参数里塞一个凭据形态的值（审计行必须打码）
    let (_, _, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "set_read",
        json!({
            "ids": [ids[0]],
            "probe": probe,
            "note": "https://alice:hunter2@example.com/private.xml?token=SECRET-TOKEN",
        }),
    )
    .await;
    assert_eq!(body["ok"], true, "{body}");

    // 被拒的调用也要留痕（读 token 调写工具）
    let (_, _, _) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "set_read",
        json!({"ids": [ids[1]], "probe": probe}),
    )
    .await;
    // refresh 与 fetch_fulltext 也要有行
    let (_, _, _) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "refresh",
        json!({"scope": "feed_ids", "feed_ids": [404], "probe": probe}),
    )
    .await;
    let (_, _, _) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "fetch_fulltext",
        json!({"id": ids[0], "probe": probe}),
    )
    .await;

    let lines: Vec<String> = capture
        .lines
        .lock()
        .expect("捕获锁被毒化")
        .iter()
        .filter(|(target, _)| target == "mcp")
        .map(|(_, line)| line.clone())
        .collect();

    // 只认领带本次 probe 标记的行（同进程其它用例的审计行不参与断言）
    let audited: Vec<&String> = lines
        .iter()
        .filter(|l| l.contains("mcp-write tool=") && l.contains(&probe))
        .collect();
    for tool in ["set_read", "refresh", "fetch_fulltext"] {
        assert!(
            audited.iter().any(|l| l.contains(&format!("tool={tool}"))),
            "缺少 {tool} 的审计行: {audited:#?}"
        );
    }
    assert_eq!(
        audited
            .iter()
            .filter(|l| l.contains("tool=set_read"))
            .count(),
        2,
        "set_read 成功与被拒各一行: {audited:#?}"
    );

    let set_read_ok = audited
        .iter()
        .find(|l| l.contains("tool=set_read") && l.contains("ok=true"))
        .expect("应有 set_read 成功行");
    assert!(set_read_ok.contains("affected=1"), "{set_read_ok}");
    assert!(
        !set_read_ok.contains("hunter2"),
        "凭据不得进审计行: {set_read_ok}"
    );
    assert!(!set_read_ok.contains("SECRET-TOKEN"), "{set_read_ok}");
    assert!(
        set_read_ok.contains("token=***"),
        "打码痕迹要留下: {set_read_ok}"
    );

    let rejected = audited
        .iter()
        .find(|l| l.contains("tool=set_read") && l.contains("ok=false"))
        .expect("应有被拒行");
    assert!(
        rejected.contains("error_code=write_scope_required"),
        "{rejected}"
    );

    let refresh_line = audited
        .iter()
        .find(|l| l.contains("tool=refresh"))
        .expect("refresh 也要有审计行");
    assert!(
        refresh_line.contains("error_code=feed_not_found"),
        "{refresh_line}"
    );

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}
