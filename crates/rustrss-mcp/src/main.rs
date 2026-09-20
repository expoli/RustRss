//! RustRss MCP 服务器入口。
//!
//! 两种传输：
//! - 默认 **stdio**：由 MCP 客户端作为子进程拉起；
//! - `--http [addr]`：以 HTTP（streamable HTTP）方式提供，便于不开 GUI 时使用，
//!   或让客户端以 URL 方式接入。token 取 `RUSTSS_MCP_TOKEN`，未设置则随机生成并打印。
//!
//! 日志一律走 stderr：stdio 模式下 stdout 是协议通道，混入任何其他输出都会让客户端解析失败。

use rustrss_core::Store;
use rustrss_mcp::http::{generate_token, serve, HttpConfig};
use rustrss_mcp::{resolve_db_path, serve_stdio, RustRssMcp};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db_path = resolve_db_path();

    // 不启动服务也能生成客户端配置（token 与服务将来用的是同一个）
    if args.iter().any(|a| a == "--print-config") {
        let store = Store::open(&db_path)?;
        let port = rustrss_mcp::config::port_from_store(&store);
        let token = rustrss_mcp::config::token_from_store(&store).map_err(anyhow::Error::msg)?;
        let url = format!("http://127.0.0.1:{port}/mcp");
        println!("# 数据库: {}", db_path.display());
        println!("{}", rustrss_mcp::config::client_snippet(&url, &token));
        return Ok(());
    }

    if let Some(index) = args.iter().position(|a| a == "--http") {
        let bind = args
            .get(index + 1)
            .filter(|a| a.contains(':'))
            .cloned()
            .unwrap_or_else(|| "127.0.0.1:8817".to_string());
        let addr: std::net::SocketAddr = bind.parse()?;
        let token = std::env::var("RUSTSS_MCP_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(generate_token);

        eprintln!("[rustrss-mcp] 数据库: {}", db_path.display());
        let server = RustRssMcp::open(&db_path)?;
        let handle = serve(
            server,
            HttpConfig {
                bind: addr,
                token: token.clone(),
            },
        )
        .await?;
        eprintln!("[rustrss-mcp] HTTP: http://{}/mcp（仅回环）", handle.addr);
        eprintln!("[rustrss-mcp] token: {token}");
        eprintln!("[rustrss-mcp] 客户端需带 Accept: application/json, text/event-stream");
        // 常驻；由 shell 的 Ctrl+C 结束
        std::future::pending::<()>().await;
        handle.shutdown();
        return Ok(());
    }

    eprintln!("[rustrss-mcp] 数据库: {}", db_path.display());
    let server = RustRssMcp::open(&db_path)?;
    serve_stdio(server).await
}
