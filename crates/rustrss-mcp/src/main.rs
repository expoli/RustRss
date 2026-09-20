//! RustRss MCP 服务器入口（stdio 传输）。
//!
//! 日志一律走 stderr：stdout 是 MCP 协议通道，混入任何其他输出都会让客户端解析失败。

use rustrss_mcp::{resolve_db_path, serve_stdio, RustRssMcp};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_path = resolve_db_path();
    eprintln!("[rustrss-mcp] 数据库: {}", db_path.display());

    let server = RustRssMcp::open(&db_path)?;
    serve_stdio(server).await
}
