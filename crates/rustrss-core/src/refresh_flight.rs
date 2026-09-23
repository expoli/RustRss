//! 刷新单 flight：同一进程内「是否有一次刷新在跑」的唯一标记。
//!
//! 从 `src-tauri` 上提 core 的原因：MCP 的 `refresh` 工具与界面的刷新按钮必须
//! **不叠加**（同一批源被抓两遍、重复抢库写锁）。`rustrss-mcp` 是独立 crate，
//! 拿不到 Tauri 侧的 `AppState`，所以标记本身放 core，两侧共用同一实现（应用内
//! 托管 MCP 时还共用同一个 [`RefreshGate`] 实例）。
//!
//! 口径（与迁移前逐字一致）：CAS 抢标记，抢不到立即返回 `Err`（不排队）；
//! 守卫 Drop 时释放——`?` 早退、任务被取消、panic 展开都不会把标记永久卡死。

use std::sync::atomic::{AtomicBool, Ordering};

/// 刷新单 flight 的共享标记（`Arc<RefreshGate>` 在界面与 MCP 之间传）
#[derive(Debug, Default)]
pub struct RefreshGate {
    active: AtomicBool,
}

impl RefreshGate {
    pub const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
        }
    }

    /// 尝试开始一次刷新（CAS）：已经有一次在跑时返回 `Err`（错误信息可直接展示）。
    ///
    /// 手动刷新（按钮/`r`）、定时/启动刷新、MCP 的 `refresh` 工具共用这一个标记——
    /// 三条路径都在跑的话同一批源会被抓两遍，所以「进行中就跳过」是全局口径。
    pub fn try_begin(&self) -> Result<RefreshFlight<'_>, String> {
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "刷新已在进行中（手动或自动），本次已跳过".to_string())?;
        Ok(RefreshFlight { gate: self })
    }

    /// 是否有刷新在跑（只读，用于给调用方一个可解释的状态）
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

/// 单 flight 守卫：活着就代表「有刷新在跑」，Drop 释放。
pub struct RefreshFlight<'a> {
    gate: &'a RefreshGate,
}

impl Drop for RefreshFlight<'_> {
    fn drop(&mut self) {
        self.gate.active.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn flight_is_exclusive_and_released_on_drop() {
        let gate = RefreshGate::new();
        let first = gate.try_begin().expect("空闲时应能拿到单 flight");
        assert!(gate.is_active());
        match gate.try_begin() {
            Ok(_) => panic!("进行中时的第二次必须被拒绝"),
            Err(msg) => assert!(
                msg.contains("刷新已在进行中"),
                "错误信息要能直接展示给用户，实际: {msg}"
            ),
        }

        drop(first);
        assert!(!gate.is_active(), "Drop 后应恢复空闲");
        assert!(gate.try_begin().is_ok(), "Drop 后必须能再次开始");
    }

    #[test]
    fn flight_released_on_early_return() {
        let gate = RefreshGate::new();
        // 模拟 `?` 早退：守卫在作用域结束时释放，不会把标记永久卡在 true
        let outcome: Result<(), String> = (|| -> Result<(), String> {
            let _flight = gate.try_begin()?;
            Err("刷新中途失败".into())
        })();
        assert!(outcome.is_err());
        assert!(gate.try_begin().is_ok(), "失败路径也必须释放单 flight");
    }

    /// 跨线程共享：界面与 MCP（两个入口共用一个 `Arc<RefreshGate>`）抢的是同一个标记。
    #[test]
    fn shared_gate_blocks_second_caller_across_threads() {
        let gate = Arc::new(RefreshGate::new());
        let held = gate.try_begin().expect("首个调用应拿到守卫");

        let other = Arc::clone(&gate);
        let blocked = std::thread::spawn(move || other.try_begin().is_err())
            .join()
            .expect("线程不应 panic");
        assert!(blocked, "另一个入口（如 MCP）必须被同一个标记挡住");
        assert!(gate.is_active(), "守卫还在，标记不得被别的线程清掉");

        drop(held);
        assert!(gate.try_begin().is_ok());
    }
}
