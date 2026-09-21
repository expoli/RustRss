//! 应用状态：一个被互斥保护的数据库连接 + 一个可复用的 HTTP 客户端。
//!
//! 为什么是 `Mutex<Store>`：rusqlite 的连接是 `!Sync`，而 Tauri 的 state 必须
//! `Send + Sync`。锁只包住数据库操作本身，**绝不跨越 await**（抓取阶段不碰库，
//! 见 `rustrss_core::fetch` 的三阶段拆分）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rustrss_core::fetch::{Fetcher, DEFAULT_USER_AGENT};
use rustrss_core::Store;

pub struct AppState {
    store: Mutex<Store>,
    pub fetcher: Fetcher,
    pub db_path: PathBuf,
    /// 应用内托管的 MCP HTTP 服务（用 Arc 以便跨任务共享）
    pub mcp: std::sync::Arc<crate::mcp_server::McpRuntime>,
    /// 托盘是否构建成功（决定「关闭到托盘」策略是否可用：
    /// 托盘没了还把窗口藏起来，用户就永远找不回应用了）
    tray_available: AtomicBool,
}

impl AppState {
    /// 打开界面用的库：路径与 MCP 服务器共用 `rustrss_core::resolve_db_path()`，
    /// 保证「agent 读的库」和「界面看的库」永远是同一个文件。
    pub fn open() -> Result<Self, String> {
        let db_path = rustrss_core::resolve_db_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建数据目录失败（{}）: {e}", parent.display()))?;
        }
        let store = Store::open(&db_path)
            .map_err(|e| format!("打开数据库失败（{}）: {e}", db_path.display()))?;
        let fetcher =
            Fetcher::new(DEFAULT_USER_AGENT).map_err(|e| format!("初始化 HTTP 客户端失败: {e}"))?;
        Ok(Self {
            store: Mutex::new(store),
            fetcher,
            db_path,
            mcp: std::sync::Arc::new(crate::mcp_server::McpRuntime::default()),
            tray_available: AtomicBool::new(false),
        })
    }

    /// 托盘构建成功后置位（setup 阶段调用一次）。
    pub fn set_tray_available(&self, ok: bool) {
        self.tray_available.store(ok, Ordering::Relaxed);
    }

    pub fn tray_available(&self) -> bool {
        self.tray_available.load(Ordering::Relaxed)
    }

    /// 在锁内做一次数据库操作。闭包内**不得有 await**。
    pub fn with_store<T>(&self, f: impl FnOnce(&Store) -> Result<T, String>) -> Result<T, String> {
        let guard = self
            .store
            .lock()
            .map_err(|_| "数据库锁被污染（此前有操作 panic）".to_string())?;
        f(&guard)
    }
}
