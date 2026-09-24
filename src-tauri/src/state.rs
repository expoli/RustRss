//! 应用状态：一个被互斥保护的数据库连接 + 一个可复用的 HTTP 客户端。
//!
//! 为什么是 `Mutex<Store>`：rusqlite 的连接是 `!Sync`，而 Tauri 的 state 必须
//! `Send + Sync`。锁只包住数据库操作本身，**绝不跨越 await**（抓取阶段不碰库，
//! 见 `rustrss_core::fetch` 的三阶段拆分）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rustrss_core::fetch::{Fetcher, DEFAULT_USER_AGENT};
use rustrss_core::{RefreshGate, Store};

/// 单 flight 守卫的别名：实现已上提到 `rustrss_core::refresh_flight`。
///
/// 这里保留 `pub use` 而不是直接删掉：界面侧（commands.rs）与既有测试都写
/// `RefreshFlight`，搬家后它们引用的仍是同一个类型。
pub use rustrss_core::RefreshFlight;

/// 库不兼容错误的前缀（`StoreError::SchemaRefused` 的 Display）。
///
/// 启动路径据此把「库不兼容」与其它启动失败分开：前者走降级界面（双语说明 + 导出 OPML），
/// 后者仍然直接失败退出。
pub const SCHEMA_REFUSED_PREFIX: &str = "数据库不兼容";

/// 这条启动失败是否属于「库不兼容」
pub fn is_schema_refusal(message: &str) -> bool {
    message.starts_with(SCHEMA_REFUSED_PREFIX)
}

pub struct AppState {
    store: Mutex<Store>,
    pub fetcher: Fetcher,
    pub db_path: PathBuf,
    /// 拒绝启动的原因（旧开发库 / 外来 sqlite 文件）：非空时界面只显示拒绝面板与
    /// 「导出 OPML」安全出口，托盘与 MCP 不启动。**不保开发库**（见 `store::schema`）。
    pub refusal: Option<String>,
    /// 应用内托管的 MCP HTTP 服务（用 Arc 以便跨任务共享）
    pub mcp: std::sync::Arc<crate::mcp_server::McpRuntime>,
    /// 托盘是否构建成功（决定「关闭到托盘」策略是否可用：
    /// 托盘没了还把窗口藏起来，用户就永远找不回应用了）
    tray_available: AtomicBool,
    /// 单 flight：是否有刷新（手动/定时/启动/MCP）正在进行中。
    /// 实现与标记都在 `rustrss_core::refresh_flight`——应用内托管的 MCP 服务
    /// 共用同一个 `Arc<RefreshGate>`，所以 agent 的 `refresh` 与界面刷新不叠加。
    refresh_gate: Arc<RefreshGate>,
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
            refusal: None,
            mcp: std::sync::Arc::new(crate::mcp_server::McpRuntime::default()),
            tray_available: AtomicBool::new(false),
            refresh_gate: Arc::new(RefreshGate::new()),
        })
    }

    /// 库不兼容时的降级启动：**不碰用户的库**，用一个内存库把界面撑起来，
    /// 只为让用户看到双语拒绝说明与「导出 OPML」安全出口。
    ///
    /// `db_path` 仍是**真实**库路径（导出 OPML 要用它）；界面侧看到 `refusal` 非空时
    /// 必须只渲染拒绝面板，不得把内存库当成用户数据。
    pub fn open_blocked(db_path: PathBuf, reason: String) -> Result<Self, String> {
        let store = Store::open_in_memory().map_err(|e| format!("初始化临时库失败: {e}"))?;
        let fetcher =
            Fetcher::new(DEFAULT_USER_AGENT).map_err(|e| format!("初始化 HTTP 客户端失败: {e}"))?;
        Ok(Self {
            store: Mutex::new(store),
            fetcher,
            db_path,
            refusal: Some(reason),
            mcp: std::sync::Arc::new(crate::mcp_server::McpRuntime::default()),
            tray_available: AtomicBool::new(false),
            refresh_gate: Arc::new(RefreshGate::new()),
        })
    }

    /// 拒绝启动的原因（`None` = 正常启动）
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    pub fn configured_fetcher(&self) -> Result<Fetcher, String> {
        let config = self.with_store(|s| rustrss_core::network::ProxyConfig::load(s).map_err(|e| e.to_string()))?;
        self.fetcher.configured(&config)
    }

    /// 尝试开始一次刷新（CAS）：已经有一次在跑时返回 Err。
    ///
    /// 手动刷新（按钮/`r`）与定时/启动刷新共用这一个标记——两条路径都在跑的话，
    /// 同一批源会被抓两遍、重复抢库写锁，所以「进行中就跳过」是全局口径。
    /// 返回的守卫在 Drop 时释放标记，`?` 早退或任务被取消都不会把标记卡死。
    pub fn try_begin_refresh(&self) -> Result<RefreshFlight<'_>, String> {
        self.refresh_gate.try_begin()
    }

    /// 单 flight 标记的共享句柄：拉起 MCP 服务时交给它，让 agent 的 `refresh`
    /// 与界面刷新抢同一个标记（两条入口共用一个 gate 实例）。
    pub fn refresh_gate(&self) -> Arc<RefreshGate> {
        Arc::clone(&self.refresh_gate)
    }

    /// 测试用状态：内存库 + 不发声的默认 HTTP 客户端，不碰真实数据目录。
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self {
            store: Mutex::new(Store::open_in_memory().unwrap()),
            fetcher: Fetcher::new(DEFAULT_USER_AGENT).unwrap(),
            db_path: PathBuf::from(":memory:"),
            refusal: None,
            mcp: std::sync::Arc::new(crate::mcp_server::McpRuntime::default()),
            tray_available: AtomicBool::new(false),
            refresh_gate: Arc::new(RefreshGate::new()),
        }
    }

    /// 托盘构建成功后置位（setup 阶段调用一次）。
    pub fn set_tray_available(&self, ok: bool) {
        self.tray_available.store(ok, Ordering::Relaxed);
    }

    pub fn tray_available(&self) -> bool {
        self.tray_available.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn store_is_available(&self) -> bool {
        self.store.try_lock().is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 不碰真实数据目录：内存库 + 默认 HTTP 客户端（不发请求）
    fn test_state() -> AppState {
        AppState::for_test()
    }

    #[test]
    fn refresh_flight_is_single_and_released_on_drop() {
        let state = test_state();

        let first = state.try_begin_refresh().expect("空闲时应能拿到单 flight");
        match state.try_begin_refresh() {
            Ok(_) => panic!("进行中时的第二次必须被拒绝"),
            Err(msg) => assert!(
                msg.contains("刷新已在进行中"),
                "错误信息要能直接展示给用户，实际: {msg}"
            ),
        }

        drop(first);
        assert!(
            state.try_begin_refresh().is_ok(),
            "守卫 Drop 后应恢复空闲（否则刷新就永久卡死了）"
        );
    }

    #[test]
    fn refresh_flight_released_on_early_return() {
        let state = test_state();
        // 模拟 `?` 早退：守卫在作用域结束时释放，不会把标记永久卡在 true
        let outcome: Result<(), String> = (|| -> Result<(), String> {
            let _flight = state.try_begin_refresh()?;
            Err("刷新中途失败".into())
        })();
        assert!(outcome.is_err());
        assert!(
            state.try_begin_refresh().is_ok(),
            "失败路径也必须释放单 flight"
        );
    }
}
