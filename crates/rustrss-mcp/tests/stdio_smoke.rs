//! stdio 传输的最小冒烟：把真二进制当子进程拉起，走一遍 initialize → tools/list。
//!
//! 为什么要有它：stdio 是 MCP 客户端最常用的接入方式，而它的握手/协议编码
//! （换行分隔 JSON）与 HTTP 不同；`tools/list` 又刚改成按请求 scope 过滤的
//! 自定义实现——这段代码只能由"真进程 + 真 stdin/stdout"来证明没坏。
//!
//! 口径断言：stdio 侧只列只读工具（写能力默认关、且本批次还没有真实写工具），
//! 开关打开也不会凭空出现写工具（写工具要由 T3/T4 登记）。

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

/// 拉起真二进制；`write_enabled` 控制库里的写开关
async fn spawn_server(db: &std::path::Path, write_enabled: bool) -> tokio::process::Child {
    let store = Store::open(db).expect("建库失败");
    store
        .set_bool_setting(rustrss_mcp::config::K_WRITE_ENABLED, write_enabled)
        .expect("写开关失败");
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

async fn handshake_and_list(db: &std::path::Path, write_enabled: bool) -> Vec<String> {
    let mut child = spawn_server(db, write_enabled).await;
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
    let names = handshake_and_list(&db, false).await;
    assert_eq!(names.len(), 7, "默认只读：{names:?}");
    for name in &names {
        let spec = rustrss_mcp::registry::spec(name).unwrap_or_else(|| panic!("{name} 未登记"));
        assert_eq!(spec.scope, rustrss_mcp::registry::Scope::Read, "{name}");
    }
    let _ = std::fs::remove_file(db);
}

/// stdio 的写能力同样受开关约束：开关打开也不会凭空出现写工具
/// （本批次还没登记任何写工具；这条断言在 T3/T4 之后应改成"写工具开始出现"）。
#[tokio::test]
async fn stdio_write_switch_does_not_invent_tools() {
    let db = temp_db("switched-on");
    let names = handshake_and_list(&db, true).await;
    for name in &names {
        let spec = rustrss_mcp::registry::spec(name).unwrap_or_else(|| panic!("{name} 未登记"));
        assert_eq!(
            spec.scope,
            rustrss_mcp::registry::Scope::Read,
            "未登记的写工具不该出现：{name}"
        );
    }
    let _ = std::fs::remove_file(db);
}
