//! 应用内托管 MCP 的 HTTP 服务。
//!
//! 三个关键约束（来自 spec 的验收点）：
//! 1. 只监听回环地址；
//! 2. 无 token / token 不对一律拒绝；
//! 3. token 可轮换，且轮换后旧 token **立即**失效——服务不持任何 token 副本，
//!    鉴权中间件每个请求现读库里的值（见 `rustrss_mcp::http`），
//!    所以轮换/销毁写 token 连重启都不需要。
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
    client_snippet, dangerous_enabled_from_store, is_loopback_url, port_from_store,
    token_from_store, write_enabled_from_store, write_token_from_store, DEFAULT_PORT,
    K_DANGEROUS_ENABLED, K_PORT, K_WRITE_ENABLED,
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

    /// 停止服务（端口随之释放）
    pub fn stop(&self) {
        if let Ok(mut guard) = self.handle.lock() {
            if let Some(handle) = guard.take() {
                handle.shutdown();
            }
        }
    }

    /// 启动服务；已在运行时先停掉（用于换端口）
    ///
    /// `gate`：刷新单 flight 的共享标记——与界面手刷/定时刷共用一个实例，
    /// 所以 agent 的 `refresh` 与界面刷新不会叠加。
    ///
    /// 不再传 token：鉴权中间件**每个请求现读**库里的 `mcp.token` / `mcp.write_token`，
    /// 所以轮换或销毁后旧值下一个请求即失效，不需要重启服务。
    pub async fn start(
        &self,
        db_path: &Path,
        port: u16,
        gate: std::sync::Arc<rustrss_core::RefreshGate>,
        app: tauri::AppHandle,
    ) -> Result<SocketAddr, String> {
        self.stop();
        let preview_app = app.clone();
        let mut server = RustRssMcp::open(db_path)
            .map_err(|e| format!("打开数据库失败: {e}"))?
            .with_refresh_gate(gate)
            .with_theme_notifications(move |revision| {
                use tauri::Emitter;
                app.emit("theme:changed", revision).is_ok()
            });
        if cfg!(target_os = "linux") {
            server = server.with_preview_backend(crate::theme_preview::backend(preview_app));
        }
        let bind: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .map_err(|e| format!("端口 {port} 非法: {e}"))?;
        let handle = serve(
            server,
            HttpConfig {
                bind,
                // 应用内托管：不持静态副本，只认库里此刻的值（轮换即时生效）
                token: None,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        let addr = handle.addr;
        if let Ok(mut guard) = self.handle.lock() {
            *guard = Some(handle);
        }
        Ok(addr)
    }
}
