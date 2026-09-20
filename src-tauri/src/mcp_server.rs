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
//!
//! 端口 / token / 客户端配置片段的下沉在 `rustrss_mcp::config`（那一层可测、
//! 也能被 CLI 复用），本模块只负责“应用内怎么跑这个服务”。

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Mutex;

use rustrss_mcp::http::{serve, HttpConfig, HttpHandle};
use rustrss_mcp::RustRssMcp;

/// 是否由应用托管 MCP 服务（与“服务本身的配置”分开：这是应用侧开关）
pub const K_ENABLED: &str = "mcp.enabled";

// 供 commands 层使用，实现只有一份
pub use rustrss_mcp::config::{
    client_snippet, is_loopback_url, port_from_store, token_from_store, DEFAULT_PORT,
    K_PORT, K_TOKEN,
};

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

