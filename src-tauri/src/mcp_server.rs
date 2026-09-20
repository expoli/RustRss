//! 应用内托管 MCP 的 HTTP 服务。
//!
//! 三个关键约束（来自 spec 的验收点）：
//! 1. 只监听回环地址；
//! 2. 无 token / token 不对一律拒绝；
//! 3. token 可轮换，且轮换后旧 token 立即失效（旧服务先停再换新 token 起）。
//!
//! 这里为 MCP 单独开一个数据库连接：界面与 MCP 共用同一个库文件，
//! SQLite 的 WAL 支持多连接读、写由 busy_timeout 排队，代价与复杂度都低于
//! 让 `!Sync` 的连接跨线程共享。

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Mutex;

use rustrss_core::Store;
use rustrss_mcp::http::{generate_token, serve, HttpConfig, HttpHandle};
use rustrss_mcp::RustRssMcp;

pub const K_ENABLED: &str = "mcp.enabled";
pub const K_PORT: &str = "mcp.port";
pub const K_TOKEN: &str = "mcp.token";
/// 默认端口：避开 quick-rss（8745）等常见占用
pub const DEFAULT_PORT: u16 = 8817;

pub struct McpRuntime {
    handle: Mutex<Option<HttpHandle>>,
}

impl Default for McpRuntime {
    fn default() -> Self {
        Self {
            handle: Mutex::new(None),
        }
    }
}

impl McpRuntime {
    pub fn is_running(&self) -> bool {
        self.handle.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub fn url(&self) -> Option<String> {
        self.handle
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|h| h.url()))
    }

    pub fn addr(&self) -> Option<SocketAddr> {
        self.handle.lock().ok().and_then(|g| g.as_ref().map(|h| h.addr))
    }

    /// 停止服务（端口随之释放）
    pub fn stop(&self) {
        if let Ok(mut guard) = self.handle.lock() {
            if let Some(handle) = guard.take() {
                handle.shutdown();
            }
        }
    }

    /// 启动服务；已在运行时先停掉（用于换端口/换 token）
    pub async fn start(
        &self,
        db_path: &Path,
        token: String,
        port: u16,
    ) -> Result<SocketAddr, String> {
        self.stop();
        let server = RustRssMcp::open(db_path).map_err(|e| format!("打开数据库失败: {e}"))?;
        let bind: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .map_err(|e| format!("端口 {port} 非法: {e}"))?;
        let handle = serve(server, HttpConfig { bind, token })
            .await
            .map_err(|e| e.to_string())?;
        let addr = handle.addr;
        if let Ok(mut guard) = self.handle.lock() {
            *guard = Some(handle);
        }
        Ok(addr)
    }
}

/// 读取或初始化 token（首次启用时生成并落库）
pub fn token_from_store(store: &Store) -> Result<String, String> {
    if let Some(existing) = crate::ai::non_empty_setting(store, K_TOKEN) {
        return Ok(existing);
    }
    let fresh = generate_token();
    store
        .set_setting(K_TOKEN, &fresh)
        .map_err(|e| e.to_string())?;
    Ok(fresh)
}

pub fn port_from_store(store: &Store) -> u16 {
    crate::ai::non_empty_setting(store, K_PORT)
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(DEFAULT_PORT)
}

/// 生成可直接粘贴给 MCP 客户端的配置片段。
///
/// 同时给出 JSON 片段（Claude Code / Cursor 的 mcpServers 格式）与一行命令，
/// 因为「让用户自己去猜格式」是最容易劝退的一步。
pub fn client_snippet(url: &str, token: &str) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"rustrss\": {{\n      \"type\": \"http\",\n      \"url\": \"{url}\",\n      \"headers\": {{ \"Authorization\": \"Bearer {token}\" }}\n    }}\n  }}\n}}\n\n# 或者一行命令（Claude Code）：\nclaude mcp add --transport http rustrss {url} --header \"Authorization: Bearer {token}\"\n"
    )
}

/// 判断是否为回环 URL（供界面显示与自检使用）
pub fn is_loopback_url(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost") || url.contains("[::1]")
}
