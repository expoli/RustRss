//! T4 标签工具的端到端测试：真 HTTP（MCP streamable HTTP 传输）+ 真库副本 + 读/写 token。
//!
//! 覆盖：
//! - `list_tags`：读 token 与写 token 都可用；未读计数 / 置顶 / 颜色齐全；`tools/list`
//!   的可见性与 `tools/call` 的授权一致（读 token 看不到写工具）；
//! - `create_tag` / `rename_tag` / `assign_tags` / `unassign_tags`：写 token 可用且
//!   库回读一致（另开连接模拟界面）；重复调用幂等；读 token 调用 → `write_scope_required`；
//! - `delete_tag`：缺 `confirm` → `confirm_required`；`dry_run` 返回影响篇数且库不变，
//!   与实际执行的 `affected` 相等（core 同一个计数函数）；
//! - `list_articles`：`tag_id` / `tag_name` 过滤、互斥报 `invalid_argument`、未知标签报
//!   `tag_not_found`、条目带 `tags` 字段；默认口径不被界面设置带偏；
//! - 审计：每个 tag 写调用（含 dry_run 与被拒）一行 `target=mcp`，参数摘要过 scrub。
//!
//! 无外网依赖（HTTP 服务绑 `127.0.0.1:0`，库里直接种数据）。

use std::sync::{Arc, Mutex, OnceLock};

use rustrss_core::{Entry, EntryQuery, IdOrigin, Store, TagTarget};
use rustrss_mcp::config;
use rustrss_mcp::http::{serve, HttpConfig, HttpHandle};
use rustrss_mcp::tag_tools::{
    ERROR_DUPLICATE_TAG_NAME, ERROR_INVALID_ARGUMENT, ERROR_TAG_NOT_FOUND, TAGS_PER_ENTRY_MAX,
};
use rustrss_mcp::write_tools::ERROR_ARTICLE_NOT_FOUND;
use rustrss_mcp::RustRssMcp;
use serde_json::{json, Value};

const ACCEPT_BOTH: &str = "application/json, text/event-stream";
const READ_TOKEN: &str = "read-token";
const WRITE_TOKEN: &str = "write-token";

// ---------------------------------------------------------------- 夹具

fn temp_db(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "rustrss-mcp-tag-tools-{tag}-{}.sqlite",
        std::process::id()
    ));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
    }
    p
}

fn entry(stable_id: &str, text: &str) -> Entry {
    Entry {
        stable_id: stable_id.to_string(),
        id_origin: IdOrigin::SourceData,
        source_id: stable_id.to_string(),
        title: format!("条目 {stable_id}"),
        url: Some(format!("https://example.com/{stable_id}")),
        author: None,
        published: None,
        updated: None,
        summary: Some(text.to_string()),
        content_html: None,
        content_text: Some(text.to_string()),
        thumbnail_url: None,
        categories: Vec::new(),
    }
}

/// 建库：1 个源 + 4 条（偶数 stable_id 标已读）；返回库路径
fn seeded_db(tag: &str) -> std::path::PathBuf {
    let path = temp_db(tag);
    let store = Store::open(&path).expect("建库失败");
    let feed = store
        .add_feed("https://example.com/feed.xml", Some("示例源"))
        .expect("加源失败");
    store
        .upsert_entries(
            feed,
            &[
                entry("a1", "正文一"),
                entry("a2", "正文二"),
                entry("a3", "正文三"),
                entry("a4", "正文四"),
            ],
        )
        .expect("入库失败");
    let read_ids: Vec<i64> = store
        .list_entries(&EntryQuery::default())
        .expect("读条目失败")
        .into_iter()
        .filter(|r| r.stable_id.ends_with('2') || r.stable_id.ends_with('4'))
        .map(|r| r.id)
        .collect();
    store.set_read(&read_ids, true).expect("标已读失败");
    path
}

/// 打开写能力（读 token + 写 token + 写开关；危险开关**不开**——tag 写工具不受它约束）
fn license(path: &std::path::Path) {
    let store = Store::open(path).expect("开库失败");
    config::set_token(&store, READ_TOKEN).expect("写读 token 失败");
    config::set_write_token(&store, WRITE_TOKEN).expect("写写 token 失败");
    store
        .set_bool_setting(config::K_WRITE_ENABLED, true)
        .expect("开写开关失败");
}

async fn start_mcp(path: &std::path::Path) -> HttpHandle {
    let server = RustRssMcp::open(path).expect("打开库失败");
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

// ---------------------------------------------------------------- 库回读（另一条连接，模拟界面）

fn entry_id(path: &std::path::Path, stable_id: &str) -> i64 {
    let store = Store::open(path).expect("开库失败");
    // 显式传参：夹具查找不受界面设置（list.hide_read / list.sort）影响
    store
        .list_entries(&EntryQuery {
            hide_read: Some(false),
            sort: Some(rustrss_core::ListSort::Newest),
            ..Default::default()
        })
        .expect("读条目失败")
        .into_iter()
        .find(|r| r.stable_id == stable_id)
        .unwrap_or_else(|| panic!("{stable_id} 不在库里"))
        .id
}

fn entry_ids(path: &std::path::Path) -> Vec<i64> {
    let store = Store::open(path).expect("开库失败");
    // 显式传参：这组 id 是**夹具**，不能受界面设置（list.hide_read / list.sort）影响
    store
        .list_entries(&EntryQuery {
            hide_read: Some(false),
            sort: Some(rustrss_core::ListSort::Newest),
            ..Default::default()
        })
        .expect("读条目失败")
        .iter()
        .map(|r| r.id)
        .collect()
}

fn tag_names_for_entry(path: &std::path::Path, entry_id: i64) -> Vec<String> {
    let store = Store::open(path).expect("开库失败");
    store
        .entry_tags(entry_id)
        .expect("读标签失败")
        .into_iter()
        .map(|t| t.name)
        .collect()
}

fn tag_count(path: &std::path::Path) -> usize {
    Store::open(path)
        .expect("开库失败")
        .list_tags()
        .expect("读标签失败")
        .len()
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
                "clientInfo": { "name": "tag-tools-test", "version": "0.0.0" }
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

/// 造两个标签（一个带颜色 + 未读若干），返回 (alpha_id, beta_id)
fn seed_tags(path: &std::path::Path) -> (i64, i64) {
    let store = Store::open(path).expect("开库失败");
    let alpha = store
        .create_tag("alpha", Some("#3E63DD"))
        .expect("建 alpha 失败");
    let beta = store.create_tag("beta", None).expect("建 beta 失败");
    store.set_tag_pinned(beta.id, true).expect("置顶失败");
    let a1 = entry_id(path, "a1");
    let a2 = entry_id(path, "a2");
    store
        .assign_tags(&TagTarget::Entries(vec![a1, a2]), &[alpha.id])
        .expect("打标失败");
    (alpha.id, beta.id)
}

// ---------------------------------------------------------------- AC1：list_tags 与可见性

#[tokio::test]
async fn list_tags_works_for_both_tokens_and_visibility_matches_authorization() {
    let db = seeded_db("list-tags");
    license(&db);
    let (alpha, beta) = seed_tags(&db);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();

    // ① 读 token：`list_tags` 可用（read 工具），且写工具不在 tools/list 里
    initialize(&http, &base, READ_TOKEN).await;
    let read_names = list_tool_names(&http, &base, READ_TOKEN).await;
    assert!(
        read_names.contains(&"list_tags".to_string()),
        "{read_names:?}"
    );
    for name in [
        "create_tag",
        "rename_tag",
        "assign_tags",
        "unassign_tags",
        "delete_tag",
    ] {
        assert!(
            !read_names.contains(&name.to_string()),
            "读 token 不该看到写工具 {name}: {read_names:?}"
        );
    }
    let (status, is_error, listed) =
        call_tool(&http, &base, READ_TOKEN, "list_tags", json!({})).await;
    assert_eq!(status, 200);
    assert!(!is_error, "读工具不该报 isError: {listed}");
    assert_eq!(listed["count"], 2, "{listed}");
    assert_eq!(listed["truncated"], false);
    let alpha_row = listed["tags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(alpha))
        .expect("alpha 应在清单里");
    // 元数据齐全：颜色 + 未读计数（alpha 挂 2 条，其中 a2 已读 → unread=1）
    assert_eq!(alpha_row["name"], "alpha");
    assert_eq!(alpha_row["color"], "#3e63dd");
    assert_eq!(alpha_row["pinned"], false);
    assert_eq!(
        alpha_row["unread"], 1,
        "未读计数应只算未读条目: {alpha_row}"
    );
    let beta_row = listed["tags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(beta))
        .expect("beta 应在清单里");
    assert_eq!(beta_row["color"], Value::Null);
    assert_eq!(beta_row["pinned"], true);
    assert_eq!(listed["tags"][0]["id"], json!(beta), "侧栏顺序：置顶优先");
    // 排序档可选：recent 与 sidebar 都是合法档
    let (_, _, recent) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_tags",
        json!({"sort": "recent"}),
    )
    .await;
    assert_eq!(recent["sort"], "recent");
    assert_eq!(recent["count"], 2);

    // ② 写 token：list_tags 同样可用；写工具出现在 tools/list 里
    initialize(&http, &base, WRITE_TOKEN).await;
    let write_names = list_tool_names(&http, &base, WRITE_TOKEN).await;
    for name in [
        "list_tags",
        "create_tag",
        "rename_tag",
        "assign_tags",
        "unassign_tags",
        "delete_tag",
    ] {
        assert!(
            write_names.contains(&name.to_string()),
            "{name} 应可见: {write_names:?}"
        );
    }
    let (_, _, listed_by_write_token) =
        call_tool(&http, &base, WRITE_TOKEN, "list_tags", json!({})).await;
    assert_eq!(listed_by_write_token["count"], 2);

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- AC2：写工具 + 错误码

#[tokio::test]
async fn tag_writes_land_in_the_database_and_read_paths_agree() {
    let db = seeded_db("writes");
    license(&db);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // create_tag → 库回读（另一条连接）一致
    let (_, _, created) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "create_tag",
        json!({"name": "  技术  ", "note": "https://alice:hunter2@example.com/x?token=SECRET-TOKEN"}),
    )
    .await;
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(created["affected"], 1);
    let tag_id = created["detail"]["id"].as_i64().expect("应回标签 id");
    assert_eq!(created["detail"]["name"], "技术", "name 应被 trim");
    let store = Store::open(&db).expect("开库失败");
    assert_eq!(
        store
            .tag_row(tag_id)
            .expect("读标签失败")
            .expect("标签应存在")
            .name,
        "技术",
        "库回读一致"
    );

    // 重名（大小写不敏感）→ duplicate_tag_name（不靠文案）
    let (_, is_error, dup) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "create_tag",
        json!({"name": "技术"}),
    )
    .await;
    assert!(is_error, "写工具失败应标 isError: {dup}");
    assert_eq!(dup["error_code"], ERROR_DUPLICATE_TAG_NAME);

    // 非法颜色 → invalid_argument
    let (_, _, bad_color) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "create_tag",
        json!({"name": "颜色坏", "color": "blue"}),
    )
    .await;
    assert_eq!(bad_color["error_code"], ERROR_INVALID_ARGUMENT);

    // rename_tag → 库回读一致；不存在的标签 → tag_not_found
    let (_, _, renamed) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "rename_tag",
        json!({"tag_id": tag_id, "name": "Tech"}),
    )
    .await;
    assert_eq!(renamed["ok"], true, "{renamed}");
    assert_eq!(renamed["detail"]["name"], "Tech");
    assert_eq!(
        Store::open(&db)
            .expect("开库失败")
            .tag_row(tag_id)
            .expect("读标签失败")
            .expect("标签应存在")
            .name,
        "Tech"
    );
    let (_, _, missing) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "rename_tag",
        json!({"tag_id": 4242, "name": "无处可改"}),
    )
    .await;
    assert_eq!(missing["error_code"], ERROR_TAG_NOT_FOUND);
    // 标签不存在的打标：整单拒绝、affected=0
    let (_, _, assign_missing_tag) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "assign_tags",
        json!({"ids": [entry_id(&db, "a1")], "tag_ids": [4242]}),
    )
    .await;
    assert_eq!(assign_missing_tag["error_code"], ERROR_TAG_NOT_FOUND);
    assert_eq!(assign_missing_tag["affected"], 0);

    // assign_tags（ids 形态）→ affected = 命中条目数、changed = 真正新增；库回读一致
    let a1 = entry_id(&db, "a1");
    let a3 = entry_id(&db, "a3");
    let (_, _, assigned) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "assign_tags",
        json!({"ids": [a1, a3], "tag_ids": [tag_id]}),
    )
    .await;
    assert_eq!(assigned["ok"], true, "{assigned}");
    assert_eq!(assigned["affected"], 2);
    assert_eq!(assigned["detail"]["changed"], 2);
    assert_eq!(
        tag_names_for_entry(&db, a1),
        vec!["Tech".to_string()],
        "库回读：a1 应有 Tech"
    );

    // 幂等：重复附加 affected 稳定、changed=0
    let (_, _, again) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "assign_tags",
        json!({"ids": [a1, a3], "tag_ids": [tag_id]}),
    )
    .await;
    assert_eq!(again["affected"], 2, "命中口径稳定才叫幂等: {again}");
    assert_eq!(again["detail"]["changed"], 0);

    // 不存在的条目 id 逐项回 article_not_found（存在的照样打上）
    let (_, _, partial) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "assign_tags",
        json!({"ids": [entry_id(&db, "a2"), 9999], "tag_ids": [tag_id]}),
    )
    .await;
    assert_eq!(partial["ok"], true, "{partial}");
    assert_eq!(partial["results"][1]["error_code"], ERROR_ARTICLE_NOT_FOUND);
    assert_eq!(partial["affected"], 1);

    // unassign_tags → changed=1，库回读只剩一个条目带该标签
    let (_, _, unassigned) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unassign_tags",
        json!({"ids": [a1], "tag_ids": [tag_id]}),
    )
    .await;
    assert_eq!(unassigned["affected"], 1, "{unassigned}");
    assert_eq!(unassigned["detail"]["changed"], 1);
    assert!(
        tag_names_for_entry(&db, a1).is_empty(),
        "库回读：a1 已无标签"
    );
    assert_eq!(
        Store::open(&db)
            .expect("开库失败")
            .entry_tags(a3)
            .expect("读标签失败")
            .len(),
        1,
        "别的条目上的关联不动"
    );

    // 条件级（feed_id + since/until 闭区间）：命中 a3 与 a4 中的未读一条 → 用无 since 的
    // feed_id 条件命中全部 4 条（含已读——与列表「不隐藏已读」默认口径同档）
    let (_, _, scoped) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "assign_tags",
        json!({"feed_id": Store::open(&db).unwrap().list_feeds().unwrap()[0].id, "tag_ids": [tag_id]}),
    )
    .await;
    assert_eq!(scoped["affected"], 4, "{scoped}");
    assert!(
        scoped["detail"]["target"]["feed_id"].is_i64(),
        "条件级 target 要回显条件: {scoped}"
    );
    for id in entry_ids(&db) {
        assert_eq!(tag_names_for_entry(&db, id), vec!["Tech".to_string()]);
    }

    // 目标形态与标签批量的边界 → invalid_argument（不静默全库打标）
    for args in [
        json!({"ids": [a1], "feed_id": 1, "tag_ids": [tag_id]}),
        json!({"tag_ids": [tag_id]}),
        json!({"ids": [a1], "tag_ids": []}),
        json!({"ids": [a1], "tag_ids": (1..=101).collect::<Vec<i64>>()}),
    ] {
        let (_, _, bad) = call_tool(&http, &base, WRITE_TOKEN, "assign_tags", args.clone()).await;
        assert_eq!(
            bad["error_code"], ERROR_INVALID_ARGUMENT,
            "args={args} body={bad}"
        );
    }

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn read_token_calls_to_tag_write_tools_are_refused_without_touching_the_db() {
    let db = seeded_db("read-scope");
    license(&db);
    let (alpha, _beta) = seed_tags(&db);
    let a1 = entry_id(&db, "a1");
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, READ_TOKEN).await;

    let calls = vec![
        ("create_tag", json!({"name": "偷建"})),
        ("rename_tag", json!({"tag_id": alpha, "name": "偷改"})),
        ("assign_tags", json!({"ids": [a1], "tag_ids": [alpha]})),
        ("unassign_tags", json!({"ids": [a1], "tag_ids": [alpha]})),
        ("delete_tag", json!({"tag_id": alpha, "confirm": true})),
    ];
    for (tool, args) in &calls {
        let (status, is_error, body) =
            call_tool(&http, &base, READ_TOKEN, tool, args.clone()).await;
        assert_eq!(status, 200, "{tool} 应是工具级错误而不是 401");
        assert!(is_error, "{tool} 被拒应标 isError: {body}");
        assert_eq!(body["error_code"], "write_scope_required", "{tool}: {body}");
    }

    // 库未被触碰：标签仍在、名称未变、a1 仍带 alpha
    let store = Store::open(&db).expect("开库失败");
    assert_eq!(
        store
            .tag_row(alpha)
            .expect("读标签失败")
            .expect("标签应存在")
            .name,
        "alpha"
    );
    assert_eq!(tag_count(&db), 2);
    assert_eq!(tag_names_for_entry(&db, a1), vec!["alpha".to_string()]);

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn tag_write_tools_follow_the_write_switch_as_well() {
    let db = seeded_db("switch-off");
    license(&db);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // 开关开着：写工具可用（危险开关没开也不影响 tag 写工具——它们不在危险集合里）
    let (_, _, ok) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "create_tag",
        json!({"name": "开着"}),
    )
    .await;
    assert_eq!(ok["ok"], true, "{ok}");

    // 另一条连接（模拟设置页）关掉写能力总开关
    Store::open(&db)
        .expect("开库失败")
        .set_bool_setting(config::K_WRITE_ENABLED, false)
        .expect("关写开关失败");

    // tools/list 里写工具不再出现；调用 → write_disabled，库不变
    let names = list_tool_names(&http, &base, WRITE_TOKEN).await;
    assert!(!names.contains(&"create_tag".to_string()), "{names:?}");
    assert!(
        names.contains(&"list_tags".to_string()),
        "读工具不受写开关影响: {names:?}"
    );
    for (tool, args) in [
        ("create_tag", json!({"name": "偷建"})),
        ("rename_tag", json!({"tag_id": 1, "name": "偷改"})),
        ("assign_tags", json!({"ids": [1], "tag_ids": [1]})),
        ("unassign_tags", json!({"ids": [1], "tag_ids": [1]})),
        ("delete_tag", json!({"tag_id": 1, "confirm": true})),
    ] {
        let (status, is_error, body) = call_tool(&http, &base, WRITE_TOKEN, tool, args).await;
        assert_eq!(status, 200, "{tool} 应是工具级错误而不是 401");
        assert!(is_error, "{tool} 被拒应标 isError: {body}");
        assert_eq!(body["error_code"], "write_disabled", "{tool}: {body}");
    }
    assert_eq!(tag_count(&db), 1, "被拒的调用不得建出标签");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- AC3：delete_tag 与审计

/// 覆盖 `delete_tag` 的 confirm/dry_run 语义（本用例在**危险开关关闭**下跑：
/// tag 删除不属于危险工具，只受写能力闸门 + confirm 约束）。
#[tokio::test]
async fn delete_tag_requires_confirm_and_dry_run_matches_the_real_delete() {
    let db = seeded_db("delete");
    license(&db);
    let (alpha, _beta) = seed_tags(&db);
    let store = Store::open(&db).expect("开库失败");
    let entries_before = store.entry_count().expect("数条目失败");
    assert_eq!(store.tag_entry_count(alpha).expect("数关联失败"), 2);
    drop(store);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // 缺 confirm → confirm_required，库不变
    let (_, is_error, refused) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "delete_tag",
        json!({"tag_id": alpha}),
    )
    .await;
    assert!(is_error, "{refused}");
    assert_eq!(refused["error_code"], "confirm_required");
    assert_eq!(tag_count(&db), 2, "被拒后标签仍在");
    assert_eq!(
        Store::open(&db).unwrap().tag_entry_count(alpha).unwrap(),
        2,
        "被拒后关联仍在"
    );

    // dry_run（不需要 confirm）→ 影响篇数 + 库不变
    let (_, _, preview) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "delete_tag",
        json!({"tag_id": alpha, "dry_run": true}),
    )
    .await;
    assert_eq!(preview["dry_run"], true, "{preview}");
    assert_eq!(preview["affected"], 2);
    assert_eq!(tag_count(&db), 2, "dry_run 后标签仍在");
    assert_eq!(
        Store::open(&db).unwrap().tag_entry_count(alpha).unwrap(),
        2,
        "dry_run 后关联仍在"
    );

    // 真删 → affected 与预览相等（core 同一个计数函数），标签消失、文章保留
    let (_, _, done) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "delete_tag",
        json!({"tag_id": alpha, "confirm": true}),
    )
    .await;
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(done["affected"], preview["affected"], "预览与执行必须同源");
    assert_eq!(tag_count(&db), 1, "标签被删");
    let store = Store::open(&db).expect("开库失败");
    assert_eq!(
        store.entry_count().expect("数条目失败"),
        entries_before,
        "文章保留"
    );
    assert_eq!(store.orphan_entry_tag_count().expect("数孤儿失败"), 0);
    assert!(
        tag_names_for_entry(&db, entry_ids(&db)[0]).is_empty(),
        "关联被清"
    );

    // 删不存在的标签 → tag_not_found
    let (_, _, missing) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "delete_tag",
        json!({"tag_id": alpha, "confirm": true}),
    )
    .await;
    assert_eq!(missing["error_code"], ERROR_TAG_NOT_FOUND);

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

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
async fn every_tag_write_call_leaves_a_scrubbed_audit_line() {
    let capture = install_capture();
    let db = seeded_db("audit");
    license(&db);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, WRITE_TOKEN).await;

    // 日志捕获是进程级的：用本次独有的 probe 标记只认领自己的行
    let probe = format!(
        "tag-probe-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let a1 = entry_id(&db, "a1");

    let (_, _, created) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "create_tag",
        json!({
            "name": format!("标签-{probe}"),
            "note": "https://alice:hunter2@example.com/private.xml?token=SECRET-TOKEN",
            "probe": probe,
        }),
    )
    .await;
    assert_eq!(created["ok"], true, "{created}");
    let tag_id = created["detail"]["id"].as_i64().unwrap();

    let (_, _, assigned) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "assign_tags",
        json!({"ids": [a1], "tag_ids": [tag_id], "probe": probe}),
    )
    .await;
    assert_eq!(assigned["affected"], 1, "{assigned}");

    let (_, _, renamed) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "rename_tag",
        json!({"tag_id": tag_id, "name": format!("改名-{probe}"), "probe": probe}),
    )
    .await;
    assert_eq!(renamed["ok"], true, "{renamed}");

    let (_, _, unassigned) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "unassign_tags",
        json!({"ids": [a1], "tag_ids": [tag_id], "probe": probe}),
    )
    .await;
    assert_eq!(unassigned["affected"], 1, "{unassigned}");

    // dry_run 也要有行（且标出 dry_run）
    let (_, _, preview) = call_tool(
        &http,
        &base,
        WRITE_TOKEN,
        "delete_tag",
        json!({"tag_id": tag_id, "dry_run": true, "probe": probe}),
    )
    .await;
    assert_eq!(preview["dry_run"], true, "{preview}");

    // 被拒的调用（读 token 调写工具）也要留痕
    let (_, _, refused) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "delete_tag",
        json!({"tag_id": tag_id, "confirm": true, "probe": probe}),
    )
    .await;
    assert_eq!(refused["error_code"], "write_scope_required", "{refused}");

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

    for tool in [
        "create_tag",
        "assign_tags",
        "rename_tag",
        "unassign_tags",
        "delete_tag",
    ] {
        assert!(
            audited.iter().any(|l| l.contains(&format!("tool={tool}"))),
            "缺少 {tool} 的审计行: {audited:#?}"
        );
    }
    let create_line = audited
        .iter()
        .find(|l| l.contains("tool=create_tag"))
        .expect("应有 create_tag 行");
    assert!(create_line.contains("ok=true"), "{create_line}");
    assert!(create_line.contains("affected=1"), "{create_line}");
    assert!(
        !create_line.contains("hunter2"),
        "凭据不得进审计行: {create_line}"
    );
    assert!(!create_line.contains("SECRET-TOKEN"), "{create_line}");
    assert!(
        create_line.contains("token=***"),
        "打码痕迹要留下: {create_line}"
    );

    let preview_line = audited
        .iter()
        .find(|l| l.contains("tool=delete_tag") && l.contains("ok=true"))
        .expect("应有 delete_tag dry_run 行");
    assert!(preview_line.contains("dry_run=true"), "{preview_line}");

    let rejected_line = audited
        .iter()
        .find(|l| l.contains("tool=delete_tag") && l.contains("ok=false"))
        .expect("应有 delete_tag 被拒行");
    assert!(
        rejected_line.contains("error_code=write_scope_required"),
        "{rejected_line}"
    );

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

// ---------------------------------------------------------------- AC4：list_articles 标签过滤

#[tokio::test]
async fn list_articles_filters_by_tag_and_carries_tag_names() {
    let db = seeded_db("list-filter");
    license(&db);
    let (alpha, beta) = seed_tags(&db);
    let a1 = entry_id(&db, "a1");
    let a2 = entry_id(&db, "a2");
    Store::open(&db)
        .expect("开库失败")
        .assign_tags(&TagTarget::Entries(vec![a2]), &[beta])
        .expect("打标失败");
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, READ_TOKEN).await;

    // ① 默认列表：条目带 tags 字段（名称数组），且不隐藏已读
    let (_, _, all) = call_tool(&http, &base, READ_TOKEN, "list_articles", json!({})).await;
    assert_eq!(all["count"], 4, "默认不隐藏已读: {all}");
    let tagged = all["articles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == json!(a1))
        .expect("a1 应在列表里");
    assert_eq!(tagged["tags"], json!(["alpha"]), "{tagged}");
    assert!(
        tagged.get("tags_truncated").is_none(),
        "未截断时不出现该字段"
    );

    // ② tag_id 过滤：只剩挂该标签的条目
    let (_, _, by_id) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"tag_id": alpha}),
    )
    .await;
    assert_eq!(by_id["count"], 2, "{by_id}");
    assert!(
        by_id["articles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["tags"].as_array().unwrap().contains(&json!("alpha"))),
        "过滤回来的每条都应带该标签: {by_id}"
    );

    // ③ tag_name 过滤（大小写不敏感）：口径与 tag_id 一致
    let (_, _, by_name) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"tag_name": "ALPHA"}),
    )
    .await;
    assert_eq!(by_name["count"], by_id["count"], "{by_name}");
    assert_eq!(by_name["articles"][0]["id"], by_id["articles"][0]["id"]);

    // ④ 互斥 / 未知标签 / 空名称 → 机器可读错误码（不静默空列表）
    let (_, _, both) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"tag_id": alpha, "tag_name": "beta"}),
    )
    .await;
    assert_eq!(both["error_code"], ERROR_INVALID_ARGUMENT, "{both}");
    let (_, _, unknown_id) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"tag_id": 4242}),
    )
    .await;
    assert_eq!(unknown_id["error_code"], ERROR_TAG_NOT_FOUND);
    let (_, _, unknown_name) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"tag_name": "不存在的标签"}),
    )
    .await;
    assert_eq!(unknown_name["error_code"], ERROR_TAG_NOT_FOUND);
    let (_, _, empty_name) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"tag_name": "   "}),
    )
    .await;
    assert_eq!(empty_name["error_code"], ERROR_INVALID_ARGUMENT);

    // ⑤ 过滤与分页/排序/其它过滤共存：tag + unread_only
    let (_, _, unread_tagged) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"tag_id": alpha, "unread_only": true}),
    )
    .await;
    assert_eq!(
        unread_tagged["count"], 1,
        "alpha 的 2 条里只有 a1 未读: {unread_tagged}"
    );

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}

/// AC4 的回归面：注入界面设置（`list.sort` / `list.hide_read`）后，MCP 的默认口径不变；
/// 同时验证条目 `tags` 字段的体积上限（`TAGS_PER_ENTRY_MAX` + 显式截断标记）。
#[tokio::test]
async fn list_articles_default_view_ignores_ui_settings_and_bounds_the_tags_field() {
    let db = seeded_db("defaults");
    license(&db);
    let store = Store::open(&db).expect("开库失败");
    // 界面设置：最早在前 + 隐藏已读（都不该影响 MCP 的默认口径）
    store
        .set_setting("list.sort", "oldest")
        .expect("写设置失败");
    store
        .set_setting("list.hide_read", "true")
        .expect("写设置失败");
    // 一个条目挂 > TAGS_PER_ENTRY_MAX 个标签：验证 tags 字段的截断口径
    let many: Vec<i64> = (0..(TAGS_PER_ENTRY_MAX as i64 + 5))
        .map(|i| {
            store
                .create_tag(&format!("tag-{i:02}"), None)
                .expect("建标签失败")
                .id
        })
        .collect();
    let a1 = entry_id(&db, "a1");
    let a4 = entry_id(&db, "a4");
    store
        .assign_tags(&TagTarget::Entries(vec![a4]), &many)
        .expect("打标失败");
    drop(store);
    let handle = start_mcp(&db).await;
    let base = format!("http://{}", handle.addr);
    let http = client();
    initialize(&http, &base, READ_TOKEN).await;

    // 默认口径：newest + 不隐藏已读（4 条都在，且第一条是最新的 a4）
    let (_, _, default_call) =
        call_tool(&http, &base, READ_TOKEN, "list_articles", json!({})).await;
    assert_eq!(
        default_call["count"], 4,
        "默认不隐藏已读（即使界面设置开着 hide_read）"
    );
    assert_eq!(default_call["page_size"], 10);
    let first = &default_call["articles"][0];
    assert_eq!(first["id"], json!(a4), "默认 newest：a4（最后入库）在前");
    let (_, _, ui_view) = call_tool(
        &http,
        &base,
        READ_TOKEN,
        "list_articles",
        json!({"sort": "oldest", "hide_read": true}),
    )
    .await;
    assert_eq!(ui_view["count"], 2, "显式 override 才跟随 UI 口径");
    assert_eq!(ui_view["articles"][0]["id"], json!(a1));

    // tags 字段带上限 + 显式截断标记；名称本身仍是名称（不是 id）
    let many_row = default_call["articles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == json!(a4))
        .expect("挂了很多标签的条目应在列表里");
    assert_eq!(
        many_row["tags"].as_array().unwrap().len(),
        TAGS_PER_ENTRY_MAX
    );
    assert_eq!(many_row["tags_truncated"], true);
    assert!(many_row["tags"][0].is_string(), "{many_row}");

    handle.shutdown();
    let _ = std::fs::remove_file(db);
}
