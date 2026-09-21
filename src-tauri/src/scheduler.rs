//! 自动刷新调度器：定时（间隔可配，默认 30 分钟）+ 启动后延迟刷新。
//!
//! 为什么在宿主侧而不是 core：core 保持纯库（没有时钟、没有线程、不认识 Tauri），
//! 「什么时候该干活」是宿主的决定。刷新本体与手动刷新共用
//! `commands::refresh_core`，两条路径不会漂移。
//!
//! 单 flight：动手前先 CAS `AppState` 的进行中标记，抢不到就静默跳过本轮——连
//! `refresh:start` 都不发，免得前端收到 start 却等不到 done 而卡在「刷新中」。

use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::commands;
use crate::state::AppState;

/// 每分钟醒一次。间隔档位最小 15 分钟，这个精度足够；同时把「每次 tick 读一次
/// 设置」的读库频率压到最低（锁只在这几下里短暂持有）。
const TICK: Duration = Duration::from_secs(60);
/// 启动首刷的延迟：等首屏 loadAll 与主题初始化落定，再让网络流量进场。
const START_DELAY: Duration = Duration::from_secs(10);
/// 后台刷新的抓取并发（与手动 refresh_all 的默认档一致）。
const CONCURRENCY: usize = 6;

/// 后台刷新开始（仅后台刷新发；手动刷新走同步等待，不发事件避免双提示）。
pub const EVENT_REFRESH_START: &str = "refresh:start";
/// 后台刷新结束（成功或失败都会发；前端据此静默 loadAll，不打断阅读焦点）。
pub const EVENT_REFRESH_DONE: &str = "refresh:done";

/// 启动调度：setup 阶段调用一次，之后整个进程生命周期都在后台跑。
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // 启动首刷：先延迟再读开关，用户在头 10 秒里改设置也算数。
        tokio::time::sleep(START_DELAY).await;
        let mut last_attempt = Instant::now();
        if on_start_enabled(&app) {
            background_refresh(&app).await;
            last_attempt = Instant::now();
        }

        loop {
            tokio::time::sleep(TICK).await;
            let Some(interval) = tick_interval(&app) else {
                // 关闭态：不排程，也不累积「欠下的时间」——重新打开后从当前时刻算起
                last_attempt = Instant::now();
                continue;
            };
            if !is_due(interval, last_attempt.elapsed()) {
                continue;
            }
            // 无论这轮是真抓了还是被单 flight 跳过，都按「已处理」重新计时：
            // 被跳过的下一次要等一个完整间隔，而不是下一分钟接着撞。
            last_attempt = Instant::now();
            background_refresh(&app).await;
        }
    });
}

/// 距上次尝试是否已过了完整间隔（纯函数，边界语义便于单测与推理）。
fn is_due(interval: Duration, since_last_attempt: Duration) -> bool {
    since_last_attempt >= interval
}

/// 读一次间隔设置（`off` → None，不调度）。锁只在 with_store 内短暂持有。
fn tick_interval(app: &AppHandle) -> Option<Duration> {
    let state = app.state::<AppState>();
    match commands::refresh_interval_setting(&state) {
        Ok(value) => commands::refresh_interval_duration(&value),
        Err(e) => {
            eprintln!("[rustrss] 读自动刷新间隔失败（本轮跳过）: {e}");
            None
        }
    }
}

/// 启动首刷是否开启。读不到设置时按「不开」处理：宁可不动，也不要带着
/// 未知配置去发一批请求（用户仍可手动刷新）。
fn on_start_enabled(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    match commands::refresh_on_start_setting(&state) {
        Ok(enabled) => enabled,
        Err(e) => {
            eprintln!("[rustrss] 读取「启动时刷新」设置失败，本次不自动刷新: {e}");
            false
        }
    }
}

/// 一次后台刷新：抢单 flight → 采样 → emit start → 共用刷新管线 → 通知/角标 → emit done。
async fn background_refresh(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(_flight) = state.try_begin_refresh() else {
        return; // 已有刷新在跑（手动或上一轮）：本轮跳过，不发事件
    };
    // 采样点必须在抢到单 flight **之后**：从这里到收工之间不会有第二条刷新
    // （手动/自动共用这一个标记）并发改动未读数，前后的差值才只反映本轮抓到的
    // 新文章。
    let before_unread = unread_total(&state);
    let _ = app.emit(EVENT_REFRESH_START, ());
    match commands::refresh_core(&state, None, CONCURRENCY).await {
        Ok(report) => eprintln!(
            "[rustrss] 自动刷新完成: fetched={} not_modified={} inserted={} updated={} failures={}",
            report.fetched,
            report.not_modified,
            report.inserted,
            report.updated,
            report.failures.len()
        ),
        Err(e) => eprintln!("[rustrss] 自动刷新失败: {e}"),
    }
    let _ = app.emit(EVENT_REFRESH_DONE, ());
    // 通知与角标只在后台路径（手动刷新时用户就在界面前，不打扰也不改角标）。
    // 角标与刷新成败无关：库里未读是多少，角标就显示多少。
    let after_unread = unread_total(&state);
    if let Some(after) = after_unread {
        if let Some(before) = before_unread {
            crate::notify::maybe_notify(app, before, after, notify_enabled(&state));
        }
        crate::tray::update_badge(app, after);
    }
}

/// 未读总数；读不到（锁被污染等）返回 `None`：宁可漏一轮通知，也不要让
/// 统计把刷新路径搞崩。
fn unread_total(state: &AppState) -> Option<i64> {
    state
        .with_store(|s| s.unread_total().map_err(|e| e.to_string()))
        .ok()
}

/// 「新文章通知」开关；读不到按「关」处理（默认关，与设置页一致）。
fn notify_enabled(state: &AppState) -> bool {
    match commands::notify_new_articles_setting(state) {
        Ok(enabled) => enabled,
        Err(e) => {
            eprintln!("[rustrss] 读取「新文章通知」设置失败，本轮不通知: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_only_after_a_full_interval() {
        let interval = Duration::from_secs(30 * 60);
        assert!(!is_due(interval, Duration::ZERO), "刚刷新过不应再刷");
        assert!(
            !is_due(interval, interval - Duration::from_millis(1)),
            "差一毫秒也算没到点"
        );
        assert!(is_due(interval, interval), "整点即视为到点");
        assert!(is_due(interval, interval + TICK), "超时后才 tick 也要触发");
    }

    #[test]
    fn notify_switch_defaults_off_and_follows_store() {
        let state = AppState::for_test();
        assert!(!notify_enabled(&state), "默认关（不打扰）");
        assert_eq!(unread_total(&state), Some(0), "空库未读为 0");

        state
            .with_store(|s| {
                s.set_bool_setting(commands::KEY_NOTIFY_NEW_ARTICLES, true)
                    .map_err(|e| e.to_string())
            })
            .expect("写设置");
        assert!(notify_enabled(&state), "开关打开后要读到 true");
    }
}
