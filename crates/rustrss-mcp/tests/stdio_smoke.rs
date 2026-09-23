//! stdio 传输的最小冒烟：把真二进制当子进程拉起，走一遍 initialize → tools/list。
//!
//! 为什么要有它：stdio 是 MCP 客户端最常用的接入方式，而它的握手/协议编码
//! （换行分隔 JSON）与 HTTP 不同；`tools/list` 又刚改成按请求 scope 过滤的
//! 自定义实现——这段代码只能由"真进程 + 真 stdin/stdout"来证明没坏。
//!
//! 口径断言：stdio 的写能力与 HTTP 同口径——默认只列只读工具；写开关打开**且**
//! 写 token 已生成时，T3 登记的写工具才出现（少任何一环都不出现）。

use std::process::Stdio;
use std::time::Duration;

use rustrss_core::Store;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(15);

fn temp_db(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("rustrss-mcp-stdio-{tag}-{}.sqlite", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
    }
    p
}

/// 拉起真二进制；`write_enabled` / `write_token` / `dangerous_enabled` 分别控制
/// 写开关、写 token 是否存在、危险工具开关（前三道都就位才有完整写能力）
async fn spawn_server(
    db: &std::path::Path,
    write_enabled: bool,
    write_token: bool,
    dangerous_enabled: bool,
) -> tokio::process::Child {
    let store = Store::open(db).expect("建库失败");
    store
        .set_bool_setting(rustrss_mcp::config::K_WRITE_ENABLED, write_enabled)
        .expect("写开关失败");
    store
        .set_bool_setting(rustrss_mcp::config::K_DANGEROUS_ENABLED, dangerous_enabled)
        .expect("危险开关失败");
    if write_token {
        rustrss_mcp::config::set_write_token(&store, "smoke-write-token").expect("写 token 失败");
    }
    drop(store);

    Command::new(env!("CARGO_BIN_EXE_rustrss-mcp"))
        .env("RUSTSS_DB", db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("拉起 rustrss-mcp 失败")
}

/// 发一行 JSON-RPC 并读回同一行的响应
async fn roundtrip(
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    message: Value,
) -> Value {
    let mut payload = serde_json::to_string(&message).expect("序列化失败");
    payload.push('\n');
    stdin
        .write_all(payload.as_bytes())
        .await
        .expect("写 stdin 失败");
    stdin.flush().await.expect("flush 失败");
    let line = tokio::time::timeout(TIMEOUT, lines.next_line())
        .await
        .expect("等待 stdio 响应超时")
        .expect("读 stdout 失败")
        .expect("stdio 提前关闭（协议通道被写脏或崩溃）");
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("响应不是 JSON（{e}）：{line}"))
}

async fn handshake_and_list(
    db: &std::path::Path,
    write_enabled: bool,
    write_token: bool,
    dangerous_enabled: bool,
) -> Vec<String> {
    let mut child = spawn_server(db, write_enabled, write_token, dangerous_enabled).await;
    let mut stdin = child.stdin.take().expect("应有 stdin");
    let stdout = child.stdout.take().expect("应有 stdout");
    let mut lines = BufReader::new(stdout).lines();

    let init = roundtrip(
        &mut stdin,
        &mut lines,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2026-07-28",
                "capabilities": {},
                "clientInfo": { "name": "stdio-smoke", "version": "0.0.0" }
            }
        }),
    )
    .await;
    assert_eq!(
        init["result"]["serverInfo"]["name"], "rustrss",
        "stdio 握手应返回服务器信息: {init}"
    );

    // 初始化完成通知（MCP 规范要求）；通知没有响应
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .expect("写初始化通知失败");
    stdin.flush().await.expect("flush 失败");

    let listed = roundtrip(
        &mut stdin,
        &mut lines,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )
    .await;
    let names: Vec<String> = listed["result"]["tools"]
        .as_array()
        .expect("应有 tools 数组")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();

    drop(stdin); // 关掉协议通道，子进程随之退出
    let _ = tokio::time::timeout(TIMEOUT, child.wait()).await;
    names
}

#[tokio::test]
async fn stdio_lists_only_read_tools_by_default() {
    let db = temp_db("default");
    let names = handshake_and_list(&db, false, false, false).await;
    assert_eq!(
        names.len(),
        rustrss_mcp::registry::read_tool_count(),
        "默认只读：{names:?}"
    );
    for name in &names {
        let spec = rustrss_mcp::registry::spec(name).unwrap_or_else(|| panic!("{name} 未登记"));
        assert_eq!(spec.scope, rustrss_mcp::registry::Scope::Read, "{name}");
    }
    let _ = std::fs::remove_file(db);
}

/// stdio 的写能力同样受两道闸约束：只开开关（没有写 token）仍不列写工具；
/// 两环都就位后 T3 的写工具才出现。
#[tokio::test]
async fn stdio_write_tools_need_both_the_switch_and_a_token() {
    // ① 开关开、写 token 不存在 → 仍然只有只读工具（写能力没被显式 provision）
    let db = temp_db("switch-only");
    let names = handshake_and_list(&db, true, false, false).await;
    for name in &names {
        let spec = rustrss_mcp::registry::spec(name).unwrap_or_else(|| panic!("{name} 未登记"));
        assert_eq!(
            spec.scope,
            rustrss_mcp::registry::Scope::Read,
            "写 token 不存在时不该出现写工具：{name}"
        );
    }
    let _ = std::fs::remove_file(db);

    // ② 写开关 + 写 token 就位（危险开关仍关）→ 普通写工具出现，危险工具仍不可见
    let db = temp_db("licensed");
    let names = handshake_and_list(&db, true, true, false).await;
    for name in [
        "set_read",
        "set_starred",
        "set_read_later",
        "refresh",
        "fetch_fulltext",
        // T4 的普通写工具
        "subscribe",
        "update_feed",
        "folder_create",
        "folder_rename",
        "import_opml",
        "export_opml",
    ] {
        assert!(
            names.contains(&name.to_string()),
            "{name} 应可见：{names:?}"
        );
    }
    for name in ["unsubscribe", "folder_delete"] {
        assert!(
            !names.contains(&name.to_string()),
            "危险工具在危险开关关闭时不得出现：{name} / {names:?}"
        );
    }
    let non_dangerous = rustrss_mcp::registry::TOOL_SPECS
        .iter()
        .filter(|s| !s.dangerous)
        .count();
    assert_eq!(names.len(), non_dangerous);
    let _ = std::fs::remove_file(db);

    // ③ 危险开关也开 → 危险工具（unsubscribe / folder_delete）出现
    let db = temp_db("dangerous");
    let names = handshake_and_list(&db, true, true, true).await;
    for name in ["unsubscribe", "folder_delete"] {
        assert!(
            names.contains(&name.to_string()),
            "{name} 应可见：{names:?}"
        );
    }
    assert_eq!(names.len(), rustrss_mcp::registry::TOOL_SPECS.len());
    let _ = std::fs::remove_file(db);
}
