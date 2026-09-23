//! T4 订阅管理写工具的端到端测试：真 HTTP（MCP streamable HTTP 传输）+ 真库 + 写 token。
//!
//! 覆盖（对应任务 AC）：
//! - `subscribe`：首页自动发现（真站点 HTML → `<link rel="alternate">`）、`rsshub://` 三种等价
//!   形态归一为同一身份、**幂等**（重复 URL 返回既有 id 且 `detail.already_subscribed=true`、
//!   库不新增）、直接 feed 地址、`invalid_url` / `fetch_failed` 的错误码分界；
//! - `update_feed`：tri-state patch（未传不动 / null 清除 / 有值落库）逐项 sqlite 回读；
//! - `folder_create` / `folder_rename` / `folder_delete`：幂等建组、重命名、**删组不删订阅**；
//! - 危险工具：`dangerous_enabled=false` → `dangerous_tool_disabled`（且 `tools/list` 不列）、
//!   缺 `confirm` → `confirm_required`、`dry_run` 返回影响面且库不变、**dry_run 与实际执行
//!   影响面一致**（同一函数：`Store::entry_count_for_feed` / `Store::feed_ids_in_folder`）；
//! - `import_opml` / `export_opml`：与界面同一实现（core `opml`），导出文本可回导、二次导入
//!   全部 skipped（幂等去重）、路径形态可用、解析失败 → `invalid_argument`；
//! - 授权与审计：读 token 调用写工具 → `write_scope_required`；每个工具一行 `target=mcp`
//!   审计行（含 dry_run / 被拒）。
//!
//! 网络部分全部打在本机起的极简 HTTP 服务器上（`127.0.0.1:0`），不依赖外网；`rsshub://`
//! 形态**不联网**（这正是它可测的前提）。

use std::sync::{Arc, Mutex, OnceLock};

use rustrss_core::{Entry, IdOrigin, Store};
use rustrss_mcp::config;
use rustrss_mcp::http::{serve, HttpConfig, HttpHandle};
use rustrss_mcp::{registry, RustRssMcp};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ACCEPT_BOTH: &str = "application/json, text/event-stream";
const READ_TOKEN: &str = "read-token";
const WRITE_TOKEN: &str = "write-token";

// ---------------------------------------------------------------- 极简 HTTP 测试服务器

/// 按路径返回预置响应的最小 HTTP/1.1 服务器（tokio 裸 TCP，不引新依赖）。
/// `hits` 记录收到的路径——「已存在的地址不再联网」这条断言靠它。
struct TestServer {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
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

/// 站点首页：head 里带标准 feed 链接（发现路径用）
fn homepage_html() -> Vec<u8> {
    r#"<!doctype html><html><head><title>示例站点</title>
<link rel="alternate" type="application/rss+xml" title="示例源" href="/feed.xml">
</head><body>hello</body></html>"#
        .as_bytes()
        .to_vec()
}

/// 无 feed 链接的页面（NoFeedLink → invalid_url）
fn page_without_feed() -> Vec<u8> {
    r#"<!doctype html><html><head><title>没有 feed</title></head><body>nothing here</body></html>"#
        .as_bytes()
        .to_vec()
}

/// 最小 RSS 2.0 源
fn feed_xml(items: usize) -> Vec<u8> {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>测试源</title><link>http://127.0.0.1/</link><description>测试</description>"#,
    );
    for i in 1..=items {
        out.push_str(&format!(
            "<item><title>条目 {i}</title><link>http://127.0.0.1/p/{i}</link><guid>g{i}</guid><description>正文 {i}</description><pubDate>Wed, 23 Sep 2026 0{i}:00:00 GMT</pubDate></item>"
        ));
    }
    out.push_str("</channel></rss>");
    out.into_bytes()
}

/// 测试站点的标准路由集合
fn site_routes() -> Vec<(String, Vec<u8>, String)> {
    vec![
        ("/".to_string(), homepage_html(), "text/html".to_string()),
        (
            "/feed.xml".to_string(),
            feed_xml(2),
            "application/rss+xml".to_string(),
        ),
        (
            "/direct.xml".to_string(),
            feed_xml(1),
            "application/rss+xml".to_string(),
        ),
        (
            "/nofeed".to_string(),
            page_without_feed(),
            "text/html".to_string(),
        ),
    ]
}

// ---------------------------------------------------------------- 库与 MCP 服务夹具

fn temp_db(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "rustrss-mcp-feed-tools-{tag}-{}.sqlite",
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

/// 建库：2 个源（A 源 2 条、B 源 2 条），B 源放进分组「技术」；返回 (库路径, A, B, 分组)
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

/// 空库（只有 schema）
fn empty_db(tag: &str) -> std::path::PathBuf {
    temp_db(tag)
}

/// 打开写能力：读 token + 写 token + 两个开关（危险开关也开）
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

/// 只开写能力，危险开关保持关闭（危险工具闸门测试用）
fn license_without_dangerous(path: &std::path::Path) {
    license(path);
    let store = Store::open(path).expect("开库失败");
    store
        .set_bool_setting(config::K_DANGEROUS_ENABLED, false)
        .expect("关危险开关失败");
}

fn set_dangerous(path: &std::path::Path, on: bool) {
    let store = Store::open(path).expect("开库失败");
    store
        .set_bool_setting(config::K_DANGEROUS_ENABLED, on)
        .expect("写危险开关失败");
}

async fn start_mcp(path: &std::path::Path) -> HttpHandle {
    serve(
        RustRssMcp::open(path).expect("打开库失败"),
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
                "clientInfo": { "name": "feed-tools-test", "version": "0.0.0" }
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

fn feed_row(path: &std::path::Path, feed_id: i64) -> Option<rustrss_core::store::FeedRow> {
    Store::open(path)
        .expect("开库失败")
        .feed_row(feed_id)
        .expect("读源失败")
}

fn folder_names(path: &std::path::Path) -> Vec<(i64, String)> {
    Store::open(path)
        .expect("开库失败")
        .list_folders()
        .expect("读分组失败")
}

#[tokio::test]
async fn subscribe_failure_codes_are_distinguishable() {
    let db = empty_db("subscribe-errors");
    license(&db);
    let server = TestServer::start(site_routes()).await;
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① 参数级非法 → invalid_url（空串 / 非 http(s) / 无主机名 / 页面上找不到 feed 链接）
    let cases: Vec<(String, &str)> = vec![
        (String::new(), "invalid_url"),
        ("   ".to_string(), "invalid_url"),
        ("not a url".to_string(), "invalid_url"),
        ("ftp://example.com/feed.xml".to_string(), "invalid_url"),
        ("http://".to_string(), "invalid_url"),
        (server.url("/nofeed"), "invalid_url"),
    ];
    for (url, expected) in cases {
        let (status, is_error, body) =
            call_tool(&http, &base, WRITE_TOKEN, "subscribe", json!({ "url": url })).await;
        assert_eq!(status, 200, "{url}");
        assert!(is_error, "{url} 应报错: {body}");
        assert_eq!(body["ok"], false, "{url}: {body}");
        assert_eq!(body["error_code"], expected, "{url}: {body}");
    }

    // ② 网络/HTTP 失败 → fetch_failed（可重试，不是参数错）
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "subscribe",
        json!({ "url": server.url("/missing") }),
    )
    .await;
    assert!(is_error, "{body}");
    assert_eq!(body["error_code"], "fetch_failed", "{body}");

    // ③ 输入本身就是 feed → direct 路径（不扫页面）
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "subscribe",
        json!({ "url": server.url("/direct.xml") }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["detail"]["via"], "direct", "{body}");
    assert_eq!(body["detail"]["url"], server.url("/direct.xml"), "{body}");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- AC1：subscribe（发现 / 幂等 / rsshub）

#[tokio::test]
async fn subscribe_discovers_homepage_and_is_idempotent() {
    let db = empty_db("subscribe");
    license(&db);
    let server = TestServer::start(site_routes()).await;
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① 站点首页 → 自动发现出真正的 feed 地址
    let (status, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "subscribe",
        json!({ "url": server.url("/") }),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!is_error, "{body}");
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["affected"], 1, "新订阅 affected=1: {body}");
    let feed_id = body["detail"]["feed_id"].as_i64().expect("应有 feed_id");
    assert_eq!(body["detail"]["url"], server.url("/feed.xml"), "{body}");
    assert_eq!(body["detail"]["via"], "link_type", "{body}");
    assert_eq!(body["detail"]["discovered_from"], server.url("/"), "{body}");
    assert_eq!(body["detail"]["already_subscribed"], false, "{body}");
    assert!(
        !body["detail"]["title"].as_str().unwrap_or_default().is_empty(),
        "新订阅要回标题: {body}"
    );

    // ② 写后读：读 token 的另一条连接用 list_feeds 看得到（同一库、同一数据路径）
    let (_, _, feeds) = call_tool(&http, &base, READ_TOKEN, "list_feeds", json!({})).await;
    let listed = feeds["feeds"].as_array().expect("应有 feeds 数组");
    assert!(
        listed.iter().any(|f| f["id"] == feed_id),
        "list_feeds 应包含新订阅 #{feed_id}: {feeds}"
    );

    // ③ 幂等：同一首页再来一次 → 既有 id、affected=0、库不新增
    let (_, is_error, again) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "subscribe",
        json!({ "url": server.url("/") }),
    )
    .await;
    assert!(!is_error, "重复订阅不得报错: {again}");
    assert_eq!(again["ok"], true, "{again}");
    assert_eq!(again["affected"], 0, "没有新行就不该报 affected=1: {again}");
    assert_eq!(again["detail"]["feed_id"], feed_id, "{again}");
    assert_eq!(again["detail"]["already_subscribed"], true, "{again}");

    // ④ 直接给 feed 地址（已在库里）→ 幂等快路径，一个发现请求都不发
    let hits_before = server.hits().len();
    let (_, _, direct) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "subscribe",
        json!({ "url": server.url("/feed.xml") }),
    )
    .await;
    assert_eq!(direct["detail"]["feed_id"], feed_id, "{direct}");
    assert_eq!(direct["detail"]["already_subscribed"], true, "{direct}");
    assert_eq!(direct["affected"], 0, "{direct}");
    assert_eq!(
        server.hits().len(),
        hits_before,
        "已存在的地址不得再发网络请求（幂等快路径）: {:?}",
        server.hits()
    );

    // ⑤ 库不新增：只有一条订阅
    let store = Store::open(&db).expect("开库失败");
    assert_eq!(store.list_feeds().expect("读源失败").len(), 1, "重复订阅不得新增行");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn subscribe_rsshub_forms_share_one_identity() {
    let db = empty_db("subscribe-rsshub");
    license(&db);
    // 不建 HTTP 服务器：`rsshub://` 形态必须**不联网**（联网的话这个用例只能靠失败来暴露）
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① 三斜杠形态：落库归一为双斜杠
    let (_, is_error, first) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "subscribe",
        json!({ "url": "rsshub:///telegram/channel/x" }),
    )
    .await;
    assert!(!is_error, "{first}");
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["affected"], 1, "{first}");
    assert_eq!(first["detail"]["url"], "rsshub://telegram/channel/x", "{first}");
    let feed_id = first["detail"]["feed_id"].as_i64().expect("应有 feed_id");

    // ② 大写 scheme 与 ③ 官方域等价形态 → 同一个身份，不新增
    for form in [
        "RSSHUB://telegram/channel/x",
        "https://rsshub.app/telegram/channel/x",
    ] {
        let (_, is_error, again) = call_tool(
            &http,
            &base,
            WRITE_TOKEN,
            "subscribe",
            json!({ "url": form }),
        )
        .await;
        assert!(!is_error, "{form}: {again}");
        assert_eq!(again["detail"]["feed_id"], feed_id, "{form}: {again}");
        assert_eq!(again["detail"]["already_subscribed"], true, "{form}: {again}");
        assert_eq!(again["affected"], 0, "{form}: {again}");
    }

    // ④ 库里只有一条、存储形态是 scheme
    let store = Store::open(&db).expect("开库失败");
    let feeds = store.list_feeds().expect("读源失败");
    assert_eq!(feeds.len(), 1, "等价形态不得各算一条");
    assert_eq!(feeds[0].url, "rsshub://telegram/channel/x");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- AC2：update_feed / 分组 CRUD

#[tokio::test]
async fn update_feed_patch_is_tri_state_and_lands_in_sqlite() {
    let (db, feed_a, _feed_b, folder) = seeded_db("update-feed");
    license(&db);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① 只改标题
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "update_feed",
        json!({ "feed_id": feed_a, "custom_title": "  我的A源  " }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["detail"]["updated_fields"], json!(["custom_title"]), "{body}");
    assert_eq!(body["detail"]["title"], "我的A源", "trim 后落库: {body}");
    {
        let row = feed_row(&db, feed_a).expect("源应存在");
        assert_eq!(row.custom_title.as_deref(), Some("我的A源"));
        assert_eq!(row.title, "我的A源", "显示名 = COALESCE(custom_title, title)");
        assert_eq!(row.source_title, "A源");
        assert_eq!(row.folder_id, None, "未传 folder_id 不得被动");
        assert_eq!(row.refresh_interval_minutes, None, "未传间隔不得被动");
    }

    // ② 只改分组：未传的 custom_title 必须原样保留（tri-state 的核心）
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "update_feed",
        json!({ "feed_id": feed_a, "folder_id": folder }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["detail"]["updated_fields"], json!(["folder_id"]), "{body}");
    {
        let row = feed_row(&db, feed_a).expect("源应存在");
        assert_eq!(row.folder_id, Some(folder));
        assert_eq!(row.custom_title.as_deref(), Some("我的A源"), "未传字段不得被清");
    }

    // ③ 只改刷新间隔（白名单内）
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "update_feed",
        json!({ "feed_id": feed_a, "refresh_interval_minutes": 60 }),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(
        body["detail"]["refresh_interval_minutes"], 60,
        "{body}"
    );
    assert_eq!(feed_row(&db, feed_a).unwrap().refresh_interval_minutes, Some(60));

    // ④ 显式 null：移出分组 + 跟随全局档
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "update_feed",
        json!({ "feed_id": feed_a, "folder_id": null, "refresh_interval_minutes": null }),
    )
    .await;
    assert!(!is_error, "{body}");
    {
        let row = feed_row(&db, feed_a).expect("源应存在");
        assert_eq!(row.folder_id, None, "null = 移出分组");
        assert_eq!(row.refresh_interval_minutes, None, "null = 跟随全局");
    }

    // ⑤ 空串标题 = 清除自定义名，显示回退源站名
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "update_feed",
        json!({ "feed_id": feed_a, "custom_title": "   " }),
    )
    .await;
    assert!(!is_error, "{body}");
    {
        let row = feed_row(&db, feed_a).expect("源应存在");
        assert_eq!(row.custom_title, None);
        assert_eq!(row.title, "A源", "清空后显示回退源站名");
    }

    // ⑥ 范围/参数错误码（不靠文案）
    for (args, expected) in [
        (json!({ "feed_id": 999_999, "custom_title": "x" }), "feed_not_found"),
        (
            json!({ "feed_id": feed_a, "folder_id": 999_999 }),
            "folder_not_found",
        ),
        (
            json!({ "feed_id": feed_a, "refresh_interval_minutes": 99 }),
            "invalid_argument",
        ),
        (json!({ "feed_id": feed_a }), "invalid_argument"),
    ] {
        let (_, is_error, body) =
            call_tool(&http, &base, WRITE_TOKEN, "update_feed", args.clone()).await;
        assert!(is_error, "{args} 应报错: {body}");
        assert_eq!(body["error_code"], expected, "{args}: {body}");
    }

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn folder_crud_and_delete_keep_subscriptions() {
    let (db, feed_a, feed_b, existing) = seeded_db("folders");
    license(&db);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① 建组：新建 affected=1；同名幂等（既有 id、affected=0、不报错）
    let (_, is_error, created) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "folder_create",
        json!({ "name": "新组" }),
    )
    .await;
    assert!(!is_error, "{created}");
    assert_eq!(created["affected"], 1, "{created}");
    let new_id = created["detail"]["folder_id"].as_i64().expect("应有 folder_id");
    let (_, is_error, again) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "folder_create",
        json!({ "name": "新组" }),
    )
    .await;
    assert!(!is_error, "同名建组不得报错: {again}");
    assert_eq!(again["detail"]["folder_id"], new_id, "{again}");
    assert_eq!(again["affected"], 0, "{again}");
    assert_eq!(again["detail"]["already_exists"], true, "{again}");
    let (_, is_error, empty_name) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "folder_create",
        json!({ "name": "   " }),
    )
    .await;
    assert!(is_error, "{empty_name}");
    assert_eq!(empty_name["error_code"], "invalid_argument", "{empty_name}");

    // ② 重命名
    let (_, is_error, renamed) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "folder_rename",
        json!({ "folder_id": new_id, "name": "改名组" }),
    )
    .await;
    assert!(!is_error, "{renamed}");
    assert!(
        folder_names(&db).contains(&(new_id, "改名组".to_string())),
        "重命名应落库: {:?}",
        folder_names(&db)
    );
    // 不存在 / 空名 / 与其它组重名
    for (args, expected) in [
        (json!({ "folder_id": 999_999, "name": "x" }), "folder_not_found"),
        (json!({ "folder_id": new_id, "name": "  " }), "invalid_argument"),
        (json!({ "folder_id": new_id, "name": "技术" }), "invalid_argument"),
    ] {
        let (_, is_error, body) =
            call_tool(&http, &base, WRITE_TOKEN, "folder_rename", args.clone()).await;
        assert!(is_error, "{args} 应报错: {body}");
        assert_eq!(body["error_code"], expected, "{args}: {body}");
    }

    // ③ 把两个源都放进新组（含已归组的 B 源），再验证「删组不删订阅」
    for id in [feed_a, feed_b] {
        let (_, is_error, body) = call_tool(
            &http,
            &base,
            WRITE_TOKEN,
            "update_feed",
            json!({ "feed_id": id, "folder_id": new_id }),
        )
        .await;
        assert!(!is_error, "{body}");
        assert_eq!(feed_row(&db, id).unwrap().folder_id, Some(new_id));
    }
    // 折叠状态里预置这个组（界面删除命令会顺手清孤儿 id，MCP 同口径）
    Store::open(&db)
        .unwrap()
        .set_collapsed_folders(&[new_id, existing])
        .unwrap();

    // ④ 缺 confirm → confirm_required（危险开关开着）
    let (_, is_error, rejected) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "folder_delete",
        json!({ "folder_id": new_id }),
    )
    .await;
    assert!(is_error, "{rejected}");
    assert_eq!(rejected["error_code"], "confirm_required", "{rejected}");

    // ⑤ dry_run（无需 confirm）：影响面 = 组内订阅数；库不变
    let (_, is_error, preview) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "folder_delete",
        json!({ "folder_id": new_id, "dry_run": true }),
    )
    .await;
    assert!(!is_error, "{preview}");
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["dry_run"], true, "{preview}");
    assert_eq!(preview["affected"], 2, "{preview}");
    assert_eq!(preview["detail"]["feeds_affected"], 2, "{preview}");
    assert!(
        folder_names(&db).contains(&(new_id, "改名组".to_string())),
        "dry_run 不得删组"
    );
    for id in [feed_a, feed_b] {
        assert_eq!(
            feed_row(&db, id).unwrap().folder_id,
            Some(new_id),
            "dry_run 不得移出订阅"
        );
    }

    // ⑥ 真删：affected 与 dry_run 一致；组没了、订阅还在且移出到未分组
    let (_, is_error, done) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "folder_delete",
        json!({ "folder_id": new_id, "confirm": true }),
    )
    .await;
    assert!(!is_error, "{done}");
    assert_eq!(
        done["affected"], preview["affected"],
        "dry_run 与实际必须共用同一影响面: {done} vs {preview}"
    );
    assert!(
        !folder_names(&db).iter().any(|(id, _)| *id == new_id),
        "组应已删除: {:?}",
        folder_names(&db)
    );
    let store = Store::open(&db).expect("开库失败");
    for id in [feed_a, feed_b] {
        let row = store.feed_row(id).unwrap().unwrap_or_else(|| panic!("订阅 #{id} 不得被删"));
        assert_eq!(row.folder_id, None, "删组后订阅应移出到未分组");
    }
    assert_eq!(
        store.collapsed_folders(),
        vec![existing],
        "删除的组要从折叠状态里清掉（与界面同一行为）"
    );

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- AC3：危险工具闸门

#[tokio::test]
async fn unsubscribe_is_dangerous_and_dry_run_matches_execution() {
    let (db, feed_a, _feed_b, _folder) = seeded_db("unsubscribe");
    license_without_dangerous(&db);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // ① 危险开关关：列表看不到（可见性 = 可调用性），调用 → dangerous_tool_disabled
    let names = list_tool_names(&http, &base, WRITE_TOKEN).await;
    assert!(!names.contains(&"unsubscribe".to_string()), "{names:?}");
    assert!(!names.contains(&"folder_delete".to_string()), "{names:?}");
    assert_eq!(
        names.len(),
        registry::TOOL_SPECS.len() - 2,
        "危险工具不该出现在列表里: {names:?}"
    );
    let (_, is_error, gated) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unsubscribe",
        json!({ "feed_id": feed_a, "confirm": true }),
    )
    .await;
    assert!(is_error, "{gated}");
    assert_eq!(gated["error_code"], "dangerous_tool_disabled", "{gated}");
    assert!(feed_row(&db, feed_a).is_some(), "被拒的调用不得改库");
    assert_eq!(
        Store::open(&db).unwrap().entry_count_for_feed(feed_a).unwrap(),
        2,
        "条目也不得被动"
    );
    let (_, is_error, folder_gated) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "folder_delete",
        json!({ "folder_id": 1, "confirm": true }),
    )
    .await;
    assert!(is_error, "{folder_gated}");
    assert_eq!(
        folder_gated["error_code"], "dangerous_tool_disabled",
        "{folder_gated}"
    );

    // ② 打开危险开关（开关每请求现读，运行中的服务立刻生效）
    set_dangerous(&db, true);
    let names = list_tool_names(&http, &base, WRITE_TOKEN).await;
    assert!(names.contains(&"unsubscribe".to_string()), "{names:?}");
    assert!(names.contains(&"folder_delete".to_string()), "{names:?}");
    assert_eq!(names.len(), registry::TOOL_SPECS.len(), "{names:?}");

    // ③ 缺 confirm → confirm_required，库不变
    let (_, is_error, rejected) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unsubscribe",
        json!({ "feed_id": feed_a }),
    )
    .await;
    assert!(is_error, "{rejected}");
    assert_eq!(rejected["error_code"], "confirm_required", "{rejected}");
    assert!(feed_row(&db, feed_a).is_some(), "缺 confirm 不得删");

    // ④ dry_run（无需 confirm）：返回将删条目数，库不变
    let (_, is_error, preview) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unsubscribe",
        json!({ "feed_id": feed_a, "dry_run": true }),
    )
    .await;
    assert!(!is_error, "{preview}");
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["dry_run"], true, "{preview}");
    assert_eq!(preview["affected"], 2, "将删 2 条: {preview}");
    assert_eq!(preview["detail"]["entries_affected"], 2, "{preview}");
    assert!(feed_row(&db, feed_a).is_some(), "dry_run 不得删源");
    assert_eq!(
        Store::open(&db).unwrap().entry_count_for_feed(feed_a).unwrap(),
        2,
        "dry_run 不得删条目"
    );

    // ⑤ 真删：affected 与 dry_run 一致（同一影响面函数）；
    //    回读：list_feeds 不含该源、条目级联删除
    let (_, is_error, done) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unsubscribe",
        json!({ "feed_id": feed_a, "confirm": true }),
    )
    .await;
    assert!(!is_error, "{done}");
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(
        done["affected"], preview["affected"],
        "dry_run 与实际必须共用同一影响面计算: {done} vs {preview}"
    );
    assert_eq!(done["detail"]["entries_affected"], 2, "{done}");
    let (_, _, feeds) = call_tool(&http, &base, READ_TOKEN, "list_feeds", json!({})).await;
    assert!(
        !feeds["feeds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["id"] == feed_a),
        "退订后 list_feeds 不得再含该源: {feeds}"
    );
    let store = Store::open(&db).expect("开库失败");
    assert!(store.feed_row(feed_a).unwrap().is_none(), "源应已删除");
    assert_eq!(
        store.entry_count_for_feed(feed_a).unwrap(),
        0,
        "条目应级联删除（同一仓库回读）"
    );

    // ⑥ 再退订 → feed_not_found
    let (_, is_error, gone) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unsubscribe",
        json!({ "feed_id": feed_a, "confirm": true }),
    )
    .await;
    assert!(is_error, "{gone}");
    assert_eq!(gone["error_code"], "feed_not_found", "{gone}");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- AC4：OPML 往返

fn write_temp_opml(tag: &str, content: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rustrss-feed-tools-{tag}-{}.opml",
        std::process::id()
    ));
    std::fs::write(&path, content).expect("写临时 OPML 失败");
    path
}

#[tokio::test]
async fn opml_round_trip_and_import_errors() {
    // 源库：2 个源（B 源在「技术」组里）
    let (src_db, _feed_a, _feed_b, _folder) = seeded_db("opml-src");
    license(&src_db);
    let src = start_mcp(&src_db).await;
    let src_base = format!("http://{}", src.addr);
    let http = client();
    initialize(&http, &src_base, WRITE_TOKEN).await;

    let (_, is_error, exported) = call_tool(&http, &src_base, WRITE_TOKEN, "export_opml", json!({})).await;
    assert!(!is_error, "{exported}");
    assert_eq!(exported["affected"], 2, "导出 2 个源: {exported}");
    let opml = exported["detail"]["opml"]
        .as_str()
        .expect("应有 OPML 文本")
        .to_string();
    assert!(opml.contains("<opml version=\"2.0\">"), "{opml}");
    assert!(opml.contains("xmlUrl="), "{opml}");
    assert!(opml.contains("技术"), "分组结构也要导出: {opml}");
    assert!(opml.contains("A源"), "{opml}");

    // 目标库：导入 → added/skipped/errors + 分组结构
    let dst_db = empty_db("opml-dst");
    license(&dst_db);
    let dst = start_mcp(&dst_db).await;
    let dst_base = format!("http://{}", dst.addr);
    initialize(&http, &dst_base, WRITE_TOKEN).await;

    let (_, is_error, imported) = call_tool(
        &http,
        &dst_base,
        WRITE_TOKEN,
        "import_opml",
        json!({ "content": opml }),
    )
    .await;
    assert!(!is_error, "{imported}");
    assert_eq!(imported["ok"], true, "{imported}");
    assert_eq!(imported["affected"], 2, "{imported}");
    assert_eq!(imported["detail"]["added"], 2, "{imported}");
    assert_eq!(imported["detail"]["skipped"], 0, "{imported}");
    assert_eq!(imported["detail"]["errors"], json!([]), "{imported}");
    {
        let store = Store::open(&dst_db).expect("开库失败");
        let feeds = store.list_feeds().expect("读源失败");
        assert_eq!(feeds.len(), 2, "两个源都要进来");
        let folders = store.list_folders().expect("读分组失败");
        let tech = folders
            .iter()
            .find(|(_, name)| name == "技术")
            .unwrap_or_else(|| panic!("分组结构应随 OPML 导入: {folders:?}"));
        assert!(
            feeds.iter().any(|f| f.folder_id == Some(tech.0)),
            "组内源应归组: {feeds:?}"
        );
    }

    // 二次导入：全部 skipped（xmlUrl 去重，幂等）
    let (_, is_error, again) = call_tool(
        &http,
        &dst_base,
        WRITE_TOKEN,
        "import_opml",
        json!({ "content": opml }),
    )
    .await;
    assert!(!is_error, "{again}");
    assert_eq!(again["affected"], 0, "{again}");
    assert_eq!(again["detail"]["added"], 0, "{again}");
    assert_eq!(again["detail"]["skipped"], 2, "{again}");
    assert_eq!(again["detail"]["errors"], json!([]), "{again}");
    assert_eq!(
        Store::open(&dst_db).unwrap().list_feeds().unwrap().len(),
        2,
        "二次导入不得新增行"
    );

    // 路径形态：从文件读同一份 OPML（结果与 content 形态一致）
    let opml_path = write_temp_opml("same", &opml);
    let (_, is_error, by_path) = call_tool(
        &http,
        &dst_base,
        WRITE_TOKEN,
        "import_opml",
        json!({ "path": opml_path.display().to_string() }),
    )
    .await;
    assert!(!is_error, "{by_path}");
    assert_eq!(by_path["detail"]["added"], 0, "{by_path}");
    assert_eq!(by_path["detail"]["skipped"], 2, "{by_path}");
    let _ = std::fs::remove_file(&opml_path);

    // 路径形态导入一份新源（added=1）
    let extra = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\"><head><title>t</title></head><body><outline text=\"额外源\" title=\"额外源\" type=\"rss\" xmlUrl=\"https://example.com/extra-{}.xml\"/></body></opml>\n",
        std::process::id()
    );
    let extra_path = write_temp_opml("extra", &extra);
    let (_, is_error, added) = call_tool(
        &http,
        &dst_base,
        WRITE_TOKEN,
        "import_opml",
        json!({ "path": extra_path.display().to_string() }),
    )
    .await;
    assert!(!is_error, "{added}");
    assert_eq!(added["affected"], 1, "{added}");
    assert_eq!(added["detail"]["added"], 1, "{added}");
    let _ = std::fs::remove_file(&extra_path);

    // 导出 → 回导闭环：目标库再导出，源集合与源库一致（往返不丢）
    let (_, _, re_export) = call_tool(&http, &dst_base, WRITE_TOKEN, "export_opml", json!({})).await;
    assert_eq!(re_export["affected"], 3, "{re_export}");
    let text = re_export["detail"]["opml"].as_str().unwrap_or_default();
    for needle in ["a.xml", "b.xml", "extra-"] {
        assert!(text.contains(needle), "回导后的导出应含 {needle}: {text}");
    }

    // 错误路径：都不给 / 都给 / 非 XML / 路径不存在 → invalid_argument
    for (args, expected) in [
        (json!({}), "invalid_argument"),
        (json!({ "path": "/tmp/x.opml", "content": "<opml/>" }), "invalid_argument"),
        (json!({ "content": "not xml at all" }), "invalid_argument"),
        (json!({ "content": "<opml><body></opml>" }), "invalid_argument"),
        (
            json!({ "path": "/nonexistent/nope-1234.opml" }),
            "invalid_argument",
        ),
    ] {
        let (_, is_error, body) =
            call_tool(&http, &dst_base, WRITE_TOKEN, "import_opml", args.clone()).await;
        assert!(is_error, "{args} 应报错: {body}");
        assert_eq!(body["error_code"], expected, "{args}: {body}");
    }

    src.shutdown();
    dst.shutdown();
    let _ = std::fs::remove_file(src_db);
    let _ = std::fs::remove_file(dst_db);
}

// ---------------------------------------------------------------- 审计（AC6 的日志面）

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
async fn every_subscription_tool_call_leaves_a_scrubbed_audit_line() {
    let capture = install_capture();
    let (db, feed_a, _feed_b, _folder) = seeded_db("audit");
    license_without_dangerous(&db);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // 日志捕获是进程级的：每次调用塞一个本次独有的 probe 标记，只认领自己的行。
    let probe = format!(
        "feed-audit-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let p = json!(probe);

    // 每个写工具至少调一次（含被拒路径）。参数里带上凭据形态的值，顺带验证打码。
    let leaky = "https://alice:hunter2@example.com/private.xml?token=SECRET-TOKEN";
    let calls: Vec<(&str, Value)> = vec![
        ("subscribe", json!({"url": format!("rsshub://audit/{probe}"), "probe": p, "note": leaky})),
        ("update_feed", json!({"feed_id": feed_a, "custom_title": "审计名", "probe": p})),
        ("folder_create", json!({"name": format!("审计组-{probe}"), "probe": p})),
        ("folder_rename", json!({"folder_id": 404, "name": probe, "probe": p})),
        ("folder_delete", json!({"folder_id": 404, "confirm": true, "probe": p})),
        ("unsubscribe", json!({"feed_id": 404, "dry_run": true, "probe": p})),
        ("import_opml", json!({"content": "not-opml", "probe": p})),
        ("export_opml", json!({"probe": p})),
    ];
    for (name, args) in &calls {
        let (_, _, _) = call_tool(&http, &base, WRITE_TOKEN, name, args.clone()).await;
    }
    // 危险开关没开时的 unsubscribe：闸门拒绝也要留痕（那正是最需要留痕的"有人在试"）
    let (_, _, _) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unsubscribe",
        json!({"feed_id": feed_a, "confirm": true, "probe": p}),
    )
    .await;
    // 开了危险开关的 dry_run：审计行要能区分预览与真做
    set_dangerous(&db, true);
    let (_, _, _) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unsubscribe",
        json!({"feed_id": feed_a, "dry_run": true, "probe": p}),
    )
    .await;
    set_dangerous(&db, false);
    // 读 token 调写工具 → write_scope_required 也要留痕
    let (_, _, _) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "subscribe",
        json!({"url": format!("rsshub://audit/read/{probe}"), "probe": p}),
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
    let audited: Vec<&String> = lines
        .iter()
        .filter(|l| l.contains("mcp-write tool=") && l.contains(&probe))
        .collect();

    for (name, _) in &calls {
        assert!(
            audited.iter().any(|l| l.contains(&format!("tool={name}"))),
            "缺少 {name} 的审计行: {audited:#?}"
        );
    }
    // dry_run 必须体现在审计行里（预览与真做要能分开）；被闸门拒掉的也有行
    let dry = audited
        .iter()
        .find(|l| l.contains("tool=unsubscribe") && l.contains("dry_run=true"))
        .expect("应有 unsubscribe 的 dry_run 行");
    assert!(dry.contains("ok=true"), "{dry}");
    assert!(dry.contains("affected=2"), "{dry}");
    // 被拒路径（危险开关关）与无写权限路径都留痕
    assert!(
        audited
            .iter()
            .any(|l| l.contains("tool=unsubscribe") && l.contains("error_code=dangerous_tool_disabled")),
        "应留下危险开关拒绝的行: {audited:#?}"
    );
    assert!(
        audited
            .iter()
            .any(|l| l.contains("tool=subscribe") && l.contains("error_code=write_scope_required")),
        "应留下无写权限的行: {audited:#?}"
    );
    // 参数打码：凭据形态的值不得出现在审计行里，但打码痕迹要在
    let subscribe_line = audited
        .iter()
        .find(|l| l.contains("tool=subscribe") && l.contains("ok=true"))
        .expect("应有 subscribe 成功行");
    assert!(!subscribe_line.contains("hunter2"), "{subscribe_line}");
    assert!(!subscribe_line.contains("SECRET-TOKEN"), "{subscribe_line}");
    assert!(subscribe_line.contains("token=***"), "{subscribe_line}");
    // 成功行带 affected（subscribe 新建 = 1）
    assert!(subscribe_line.contains("affected=1"), "{subscribe_line}");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}
