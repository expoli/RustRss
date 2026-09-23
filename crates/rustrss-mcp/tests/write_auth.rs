//! 写能力授权矩阵的端到端测试（HTTP 传输 + 真实库）。
//!
//! 覆盖 T2 的验收点：
//! - 读 token 会话：`tools/list` 不含写工具，直接调用返回 `write_scope_required`；
//! - `write_enabled=false` → `write_disabled`；`dangerous_enabled=false` → `dangerous_tool_disabled`；
//! - **写 token 轮换/销毁后旧值下一个请求即失效**（同一个 reqwest Client，连接池复用）；
//! - 无/错 token 仍 401、`/health` 不鉴权且不含订阅数据；
//! - 写调用落 `target=mcp` 审计行（参数摘要过 scrub）；
//! - 写契约 helper 的端到端行为：批量 >100 → `invalid_argument`、`confirm` 缺失 →
//!   `confirm_required`、`dry_run` 不落库。
//!
//! **关于桩工具**：T3/T4 的真实写工具尚未落地，授权矩阵需要"已登记、可调用"的写工具。
//! 桩工具通过 `with_test_tool` 注册，走的是与真实工具**完全相同**的路径
//! （同一注册表元数据、同一 gating/错误码、同一审计行、同一写契约 helper），
//! 只有工具体是测试提供的。生产环境不注册任何桩工具。

use std::sync::{Arc, Mutex, OnceLock};

use rustrss_core::{Entry, IdOrigin, Store};
use rustrss_mcp::config;
use rustrss_mcp::http::{serve, HttpConfig, HttpHandle};
use rustrss_mcp::write_contract::{self, WriteOutcome};
use rustrss_mcp::{registry, RustRssMcp};
use serde_json::{json, Value};

/// MCP 传输规范要求客户端声明同时接受这两种类型；rmcp 不对则返回 406。
const ACCEPT_BOTH: &str = "application/json, text/event-stream";

// ---------------------------------------------------------------- 测试夹具

fn temp_db(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "rustrss-mcp-write-auth-{tag}-{}.sqlite",
        std::process::id()
    ));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
    }
    p
}

fn entry(stable_id: &str, title: &str) -> Entry {
    Entry {
        stable_id: stable_id.to_string(),
        id_origin: IdOrigin::SourceData,
        source_id: stable_id.to_string(),
        title: title.to_string(),
        url: Some(format!("https://example.com/{stable_id}")),
        author: None,
        published: None,
        updated: None,
        summary: Some("摘要".to_string()),
        content_html: None,
        content_text: Some("正文".to_string()),
        categories: Vec::new(),
    }
}

/// 建库 + 造数据，返回 (库路径, 条目 id 列表, 源的 stub 用连接)
fn seeded_db(tag: &str) -> (std::path::PathBuf, Vec<i64>) {
    let path = temp_db(tag);
    let store = Store::open(&path).expect("建库失败");
    let feed_id = store
        .add_feed("https://example.com/feed.xml", Some("示例源"))
        .expect("加源失败");
    store
        .upsert_entries(feed_id, &[entry("a1", "第一篇"), entry("a2", "第二篇")])
        .expect("入库失败");
    let ids = store
        .list_entries(&rustrss_core::EntryQuery::default())
        .expect("读条目失败")
        .iter()
        .map(|r| r.id)
        .collect::<Vec<_>>();
    (path, ids)
}

/// 只写读 token（写能力全关、写 token 不存在）：默认只读的那一档
fn open_read_only(path: &std::path::Path) {
    let store = Store::open(path).expect("开库失败");
    config::set_token(&store, "read-token").expect("写读 token 失败");
}

/// 读 token（常驻）+ 全部开关打开后的一个"已开通写能力"的库
fn open_fully_licensed(path: &std::path::Path) -> String {
    let store = Store::open(path).expect("开库失败");
    config::set_token(&store, "read-token").expect("写读 token 失败");
    config::set_write_token(&store, "write-token").expect("写写 token 失败");
    store
        .set_bool_setting(config::K_WRITE_ENABLED, true)
        .expect("开写开关失败");
    store
        .set_bool_setting(config::K_DANGEROUS_ENABLED, true)
        .expect("开危险开关失败");
    "write-token".to_string()
}

/// 桩工具用的独立连接（生产里写工具走服务自己的连接，测试里桩工具走这个）
fn stub_store(path: &std::path::Path) -> Arc<Mutex<Store>> {
    Arc::new(Mutex::new(Store::open(path).expect("桩工具开库失败")))
}

/// 桩写工具：完全按写契约实现（T3 的 `set_read` 就是这个形状）
fn set_read_stub(store: Arc<Mutex<Store>>) -> impl Fn(Option<&serde_json::Map<String, Value>>) -> String + Send + Sync {
    move |args| {
        let args = args.cloned().unwrap_or_default();
        let ids: Vec<i64> = args
            .get("ids")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        let dry_run = args.get("dry_run").and_then(Value::as_bool);
        if let Err(e) = write_contract::check_batch_ids(&ids) {
            return WriteOutcome::rejected(e).to_json();
        }
        let store = Arc::clone(&store);
        let target = ids.clone();
        write_contract::apply_or_preview(
            dry_run,
            || {
                let guard = store.lock().expect("库锁被毒化");
                // 预览与实际执行共用同一份"算影响面"的口径（PRD：预览要和执行一致）
                let affected = guard
                    .list_entries(&rustrss_core::EntryQuery {
                        unread_only: true,
                        ..Default::default()
                    })
                    .map(|rows| rows.iter().filter(|r| target.contains(&r.id)).count() as i64)
                    .unwrap_or(0);
                WriteOutcome::preview(affected, vec![])
            },
            || {
                let guard = store.lock().expect("库锁被毒化");
                let affected = guard.set_read(&target, true).unwrap_or(0) as i64;
                WriteOutcome::done(affected, vec![], false)
            },
        )
        .to_json()
    }
}

/// 桩危险工具：只校验契约（confirm），不做破坏性动作
fn unsubscribe_stub() -> impl Fn(Option<&serde_json::Map<String, Value>>) -> String + Send + Sync {
    |args| {
        let confirm = args
            .and_then(|a| a.get("confirm"))
            .and_then(Value::as_bool);
        if let Err(e) = write_contract::require_confirm(confirm) {
            return WriteOutcome::rejected(e).to_json();
        }
        WriteOutcome::done(1, vec![], false).to_json()
    }
}

fn server_with_stubs(path: &std::path::Path) -> RustRssMcp {
    let store = stub_store(path);
    RustRssMcp::open(path)
        .expect("打开库失败")
        .with_test_tool(
            "stub_set_read",
            registry::Scope::Write,
            false,
            "桩工具：按 ids 标记已读（写契约）",
            set_read_stub(store),
        )
        .with_test_tool(
            "stub_unsubscribe",
            registry::Scope::Write,
            true,
            "桩工具：危险操作（需要 confirm）",
            unsubscribe_stub(),
        )
        // 审计用例专用：日志捕获是进程级的，各测试并行跑，用独立工具名才能只认自己那几行
        .with_test_tool(
            "stub_audit_probe",
            registry::Scope::Write,
            false,
            "桩工具：只回一个写信封（审计用）",
            |_| WriteOutcome::done(2, vec![], false).to_json(),
        )
}

async fn start(server: RustRssMcp) -> HttpHandle {
    serve(
        server,
        HttpConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            // 应用内托管的形状：不持静态 token，只认库里此刻的值
            token: None,
        },
    )
    .await
    .expect("HTTP 服务应能启动")
}

/// 建一个对同一地址复用的客户端（复用连接 = "已建立的连接/会话"）
fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn initialize(client: &reqwest::Client, base: &str, token: &str) -> reqwest::Response {
    client
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
                "clientInfo": { "name": "write-auth-test", "version": "0.0.0" }
            }
        }))
        .send()
        .await
        .expect("initialize 请求失败")
}

async fn list_tools(client: &reqwest::Client, base: &str, token: &str) -> (u16, Vec<String>) {
    let response = client
        .post(format!("{base}/mcp"))
        .header("accept", ACCEPT_BOTH)
        .bearer_auth(token)
        .json(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))
        .send()
        .await
        .expect("tools/list 请求失败");
    let status = response.status().as_u16();
    if status != 200 {
        return (status, Vec::new());
    }
    let body: Value = response.json().await.expect("tools/list 应为 JSON");
    let names = body["result"]["tools"]
        .as_array()
        .expect("应有 tools 数组")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    (status, names)
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

// ---------------------------------------------------------------- 读 token：看不到也调不动写工具

#[tokio::test]
async fn read_token_cannot_see_or_call_write_tools() {
    let (db, ids) = seeded_db("read-scope");
    let server = server_with_stubs(&db);
    open_read_only(&db);
    let handle = start(server).await;
    let base = format!("http://{}", handle.addr);
    let http = client();

    assert_eq!(initialize(&http, &base, "read-token").await.status(), 200);

    // ① tools/list 只列只读工具：写工具（含桩）一个都不出现
    let (status, names) = list_tools(&http, &base, "read-token").await;
    assert_eq!(status, 200);
    assert_eq!(names.len(), registry::TOOL_SPECS.len(), "只应列出只读工具: {names:?}");
    assert!(!names.contains(&"stub_set_read".to_string()), "{names:?}");
    assert!(!names.contains(&"stub_unsubscribe".to_string()), "{names:?}");
    for name in &names {
        assert_eq!(
            registry::spec(name).expect("列表里的工具必须已登记").scope,
            registry::Scope::Read,
            "{name} 不该对读 token 可见"
        );
    }

    // ② 直接调用写工具：工具级错误 write_scope_required（不是 401，库不变）
    let (status, is_error, body) = call_tool(
        &http,
        &base,
        "read-token",
        "stub_set_read",
        json!({"ids": ids.clone()}),
    )
    .await;
    assert_eq!(status, 200, "已认证但无授权 = 工具级错误，不是 401");
    assert!(is_error, "工具级错误必须标 isError: {body}");
    assert_eq!(body["error_code"], "write_scope_required", "{body}");

    let store = Store::open(&db).unwrap();
    assert_eq!(
        store.unread_total().unwrap(),
        2,
        "被拒的写调用不得改库（也不该泄露数据）"
    );

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 开关矩阵

#[tokio::test]
async fn write_switch_and_dangerous_switch_gate_independently() {
    let (db, ids) = seeded_db("switches");
    let server = server_with_stubs(&db);
    let handle = start(server).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    let write_token = open_fully_licensed(&db);

    // ① 写能力总开关关着：列表不含写工具；调用 → write_disabled
    {
        let store = Store::open(&db).unwrap();
        store
            .set_bool_setting(config::K_WRITE_ENABLED, false)
            .unwrap();
    }
    let (_, names) = list_tools(&http, &base, &write_token).await;
    assert_eq!(names.len(), registry::TOOL_SPECS.len(), "{names:?}");
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        &write_token,
        "stub_set_read",
        json!({"ids": ids.clone()}),
    )
    .await;
    assert!(is_error);
    assert_eq!(body["error_code"], "write_disabled", "{body}");

    // ② 打开总开关：写工具出现且可调用（真写成功）
    {
        let store = Store::open(&db).unwrap();
        store
            .set_bool_setting(config::K_WRITE_ENABLED, true)
            .unwrap();
        store
            .set_bool_setting(config::K_DANGEROUS_ENABLED, false)
            .unwrap();
    }
    let (_, names) = list_tools(&http, &base, &write_token).await;
    assert!(names.contains(&"stub_set_read".to_string()), "{names:?}");
    assert!(
        !names.contains(&"stub_unsubscribe".to_string()),
        "危险开关关着时危险工具不该出现: {names:?}"
    );

    // ③ 危险工具开关关着：调用 → dangerous_tool_disabled
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        &write_token,
        "stub_unsubscribe",
        json!({"id": 1, "confirm": true}),
    )
    .await;
    assert!(is_error);
    assert_eq!(body["error_code"], "dangerous_tool_disabled", "{body}");

    // ④ 打开危险开关：仍需 confirm（契约）→ 缺 confirm 是 confirm_required
    {
        let store = Store::open(&db).unwrap();
        store
            .set_bool_setting(config::K_DANGEROUS_ENABLED, true)
            .unwrap();
    }
    let (_, names) = list_tools(&http, &base, &write_token).await;
    assert!(names.contains(&"stub_unsubscribe".to_string()), "{names:?}");
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        &write_token,
        "stub_unsubscribe",
        json!({"id": 1}),
    )
    .await;
    assert!(is_error);
    assert_eq!(body["error_code"], "confirm_required", "{body}");

    let (_, is_error, body) = call_tool(
        &http,
        &base,
        &write_token,
        "stub_unsubscribe",
        json!({"id": 1, "confirm": true}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["ok"], true, "{body}");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 轮换 / 销毁即时失效

#[tokio::test]
async fn write_token_rotation_and_destruction_kill_the_old_credential_at_once() {
    let (db, _ids) = seeded_db("rotate");
    let server = server_with_stubs(&db);
    let handle = start(server).await;
    let base = format!("http://{}", handle.addr);
    // 同一个客户端 → 复用连接池里的同一条连接（"已建立的连接"）
    let http = client();
    let old_token = open_fully_licensed(&db);

    // ① 旧写 token 可用：列表里能看到写工具
    assert_eq!(initialize(&http, &base, &old_token).await.status(), 200);
    let (status, names) = list_tools(&http, &base, &old_token).await;
    assert_eq!(status, 200);
    assert!(names.contains(&"stub_set_read".to_string()), "{names:?}");

    // ② 轮换（另一个连接写库，服务不重启）：旧值下一个请求立刻 401
    let new_token = config::generate_write_token();
    {
        let store = Store::open(&db).unwrap();
        config::set_write_token(&store, &new_token).unwrap();
    }
    let (status, names) = list_tools(&http, &base, &old_token).await;
    assert_eq!(status, 401, "轮换后旧写 token 必须立刻失效");
    assert!(names.is_empty());

    // ③ 新值可用
    let (status, names) = list_tools(&http, &base, &new_token).await;
    assert_eq!(status, 200);
    assert!(names.contains(&"stub_set_read".to_string()), "{names:?}");

    // ④ 销毁：新值也立刻失效，读 token 不受影响
    {
        let store = Store::open(&db).unwrap();
        config::clear_write_token(&store).unwrap();
    }
    let (status, _) = list_tools(&http, &base, &new_token).await;
    assert_eq!(status, 401, "销毁后写 token 必须立刻失效");
    let (status, names) = list_tools(&http, &base, "read-token").await;
    assert_eq!(status, 200, "读 token 不该被写 token 的销毁波及");

    // ⑤ 写 token 不存在时，写工具不再注册（读会话看不到；库也不认这个值）
    assert!(!names.contains(&"stub_set_read".to_string()), "{names:?}");
    let (status, _, _) = call_tool(&http, &base, "read-token", "stub_set_read", json!({"ids": [1]})).await;
    assert_eq!(status, 200);
    let (status, _, _) = call_tool(&http, &base, &old_token, "stub_set_read", json!({"ids": [1]})).await;
    assert_eq!(status, 401, "已销毁的 token 不得再换来任何权限");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 401 与 /health 口径不变

#[tokio::test]
async fn health_stays_public_and_carries_no_subscription_data() {
    let (db, _ids) = seeded_db("health");
    let server = server_with_stubs(&db);
    let handle = start(server).await;
    let base = format!("http://{}", handle.addr);
    let http = client();

    let health = http
        .get(format!("{base}/health"))
        .send()
        .await
        .expect("health 请求失败");
    assert_eq!(health.status(), 200);
    let body = health.text().await.expect("health 应有文本体");
    assert_eq!(body, "ok");
    assert!(!body.contains("示例源"), "health 不得含订阅数据: {body}");

    // 无 token / 错 token 一律 401（含"看起来像写 token"的值）
    for probe in [None, Some("wrong"), Some("write-token")] {
        let mut request = http
            .post(format!("{base}/mcp"))
            .header("accept", ACCEPT_BOTH)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}));
        if let Some(token) = probe {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.expect("请求失败");
        assert_eq!(
            response.status().as_u16(),
            401,
            "probe={probe:?} 必须 401（认证与授权分离）"
        );
    }

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 写契约 helper（端到端）

#[tokio::test]
async fn dry_run_previews_without_touching_the_database() {
    let (db, ids) = seeded_db("dry-run");
    let server = server_with_stubs(&db);
    let handle = start(server).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    let write_token = open_fully_licensed(&db);

    // ① dry_run：返回影响面，库一个字节都不变
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        &write_token,
        "stub_set_read",
        json!({"ids": ids.clone(), "dry_run": true}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["dry_run"], true, "{body}");
    assert_eq!(body["affected"], 2, "预览要给影响面: {body}");
    let store = Store::open(&db).unwrap();
    assert_eq!(store.unread_total().unwrap(), 2, "dry_run 不得落库");

    // ② 真做：同样的 ids，库真的变了
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        &write_token,
        "stub_set_read",
        json!({"ids": ids.clone()}),
    )
    .await;
    assert!(!is_error, "{body}");
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["affected"], 2, "{body}");
    assert_eq!(store.unread_total().unwrap(), 0, "真做必须落库");

    // ③ 批量超限：invalid_argument（不静默截断）
    let too_many: Vec<i64> = (1..=write_contract::MAX_BATCH_IDS as i64 + 1).collect();
    let (_, is_error, body) = call_tool(
        &http,
        &base,
        &write_token,
        "stub_set_read",
        json!({"ids": too_many}),
    )
    .await;
    assert!(is_error);
    assert_eq!(body["error_code"], "invalid_argument", "{body}");
    assert!(body["error"].as_str().unwrap().contains("100"), "{body}");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- 审计

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
        self.lines.lock().expect("捕获锁被毒化").push((
            record.target().to_string(),
            record.args().to_string(),
        ));
    }

    fn flush(&self) {}
}

fn install_capture() -> Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE
        .get_or_init(|| {
            let capture = Capture::default();
            // 装 logger 失败（同进程已被别处装过）也不影响断言：那样捕获为空，测试会明确报错
            let _ = log::set_boxed_logger(Box::new(capture.clone()));
            log::set_max_level(log::LevelFilter::Info);
            capture
        })
        .clone()
}

#[tokio::test]
async fn write_calls_leave_an_audit_line_with_scrubbed_args() {
    let capture = install_capture();
    let (db, ids) = seeded_db("audit");
    let server = server_with_stubs(&db);
    let handle = start(server).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    let write_token = open_fully_licensed(&db);

    // 参数里塞一个凭据形态的值：审计行必须打码
    let args = json!({
        "ids": ids,
        "note": "https://alice:hunter2@example.com/private.xml?token=SECRET-WRITE-TOKEN"
    });
    let (_, is_error, body) = call_tool(&http, &base, &write_token, "stub_audit_probe", args).await;
    assert!(!is_error, "{body}");

    // 被拒的调用也要留痕
    let (_, _, _) = call_tool(
        &http,
        &base,
        "read-token",
        "stub_audit_probe",
        json!({"ids": [1]}),
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
        .filter(|l| l.contains("tool=stub_audit_probe"))
        .collect();
    assert_eq!(audited.len(), 2, "成功与被拒各一行：{lines:#?}");

    let ok_line = audited
        .iter()
        .find(|l| l.contains("ok=true"))
        .expect("应有成功行");
    assert!(ok_line.contains("affected=2"), "{ok_line}");
    assert!(ok_line.contains("args="), "{ok_line}");
    assert!(!ok_line.contains("hunter2"), "凭据不得进审计行: {ok_line}");
    assert!(
        !ok_line.contains("SECRET-WRITE-TOKEN"),
        "token 不得进审计行: {ok_line}"
    );
    assert!(ok_line.contains("token=***"), "打码痕迹要留下: {ok_line}");

    let rejected_line = audited
        .iter()
        .find(|l| l.contains("ok=false"))
        .expect("应有被拒行");
    assert!(
        rejected_line.contains("error_code=write_scope_required"),
        "{rejected_line}"
    );

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}
