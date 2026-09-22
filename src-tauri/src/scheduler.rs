//! 自动刷新调度器：定时（按源算到期，默认全局 30 分钟）+ 启动后延迟刷新。
//!
//! 为什么在宿主侧而不是 core：core 保持纯库（没有时钟、没有线程、不认识 Tauri），
//! 「什么时候该干活」是宿主的决定。刷新本体与手动刷新共用
//! `commands::refresh_core`，两条路径不会漂移。
//!
//! tick 的调度单位是**源**：每个源可以用自己的间隔
//! （`feeds.refresh_interval_minutes`，NULL=跟随全局档），每轮算出到期的源，
//! 只抓这一批。到期基准是库里的 `last_fetched_at`——所以手动刷新 / OPML 抓取
//! 也会重置该源的自动计时（刚抓过的源不重复自动刷，见 tech_design 语义迁移说明）。
//!
//! 单 flight：动手前先 CAS `AppState` 的进行中标记，抢不到就静默跳过本轮——连
//! `refresh:start` 都不发，免得前端收到 start 却等不到 done 而卡在「刷新中」。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};

use crate::commands;
use crate::state::AppState;
use rustrss_core::FeedIntervalRow;

/// 每分钟醒一次。最小档位 15 分钟，这个精度足够；同时把「每次 tick 读一次设置 +
/// 扫一遍源表」的读库频率压到最低（锁只在这几下里短暂持有）。
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
        // 首刷仍是**全量**：刚起来时侧栏/未读要一次到位，而且它天然把所有源的
        // 到期基准（last_fetched_at）推到当下，之后 tick 不会立刻重抓。
        tokio::time::sleep(START_DELAY).await;
        if on_start_enabled(&app) {
            background_refresh(&app, None).await;
        }

        loop {
            tokio::time::sleep(TICK).await;
            let (global_minutes, rows) = match scan_inputs(&app) {
                Ok(inputs) => inputs,
                Err(e) => {
                    eprintln!("[rustrss] 按源扫描失败（本轮跳过）: {e}");
                    continue;
                }
            };
            let due = due_feed_ids(global_minutes, &rows, now_secs());
            if due.is_empty() {
                continue; // 没有到期源：连事件都不发
            }
            // 单 flight 抢不到（手动刷新在跑）就直接返回：被跳过的源仍是到期态，
            // 下一分钟接着试，不会累积「欠下的时间」也不需要额外计时。
            background_refresh(&app, Some(due)).await;
        }
    });
}

/// 一次 tick 的扫描输入：全局档（分钟，`off` → None）+ 全部源的
/// `(id, 覆盖分钟, 上次抓取时刻)`。两者在**同一个短锁窗口**里读完，
/// 全局档与设置页共用同一条读取路径（归一化、非法值回退默认都在里面）。
fn scan_inputs(app: &AppHandle) -> Result<(Option<i64>, Vec<FeedIntervalRow>), String> {
    let state = app.state::<AppState>();
    state.with_store(|s| {
        let global = commands::refresh_interval_duration(&commands::refresh_interval_from_store(s))
            .map(|d| (d.as_secs() / 60) as i64);
        let rows = s.feeds_with_interval().map_err(|e| e.to_string())?;
        Ok((global, rows))
    })
}

/// 当前 Unix 时间戳（秒），与库里 `last_fetched_at` 同一坐标系。
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// 本轮到期的源（纯函数，便于穷举边界）。
///
/// `rows` 是 `Store::feeds_with_interval()` 的形状：`(feed_id, 覆盖分钟, 上次抓取时刻)`。
/// - 间隔取「覆盖优先，否则全局」；两边都没有（全局 off 且无覆盖）→ 不排。
///   全局 off **不该**关掉显式覆盖的源：用户给单个源选了档位就是明确意图。
/// - 到期 = `now - last_fetched_at >= 间隔`（差一秒不算到点，整点算到点）；
///   `last_fetched_at` 为 NULL（从未抓过）视为立即到期，不等一个完整间隔。
fn due_feed_ids(global_minutes: Option<i64>, rows: &[FeedIntervalRow], now: i64) -> Vec<i64> {
    rows.iter()
        .filter(|(_, override_minutes, last_fetched_at)| {
            let Some(minutes) = override_minutes.or(global_minutes) else {
                return false;
            };
            match last_fetched_at {
                None => true,
                // saturating_sub：last_fetched_at 在未来（时钟回拨）时按「刚抓过」处理
                Some(ts) => now.saturating_sub(*ts) >= minutes * 60,
            }
        })
        .map(|(id, _, _)| *id)
        .collect()
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
/// `feed_ids = None` 全量（启动首刷），`Some(ids)` 只抓本轮到期的源。
async fn background_refresh(app: &AppHandle, feed_ids: Option<Vec<i64>>) {
    let state = app.state::<AppState>();
    let Ok(_flight) = state.try_begin_refresh() else {
        return; // 已有刷新在跑（手动或上一轮）：本轮跳过，不发事件
    };
    // 采样点必须在抢到单 flight **之后**：从这里到收工之间不会有第二条刷新
    // （手动/自动共用这一个标记）并发改动未读数，前后的差值才只反映本轮抓到的
    // 新文章。
    let before_unread = unread_total(&state);
    let _ = app.emit(EVENT_REFRESH_START, ());
    match commands::refresh_core(&state, feed_ids, CONCURRENCY).await {
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

    /// 一分钟的秒数：让「档位 × 分钟」的算式在测试里保持可读。
    const MIN: i64 = 60;

    #[test]
    fn due_follows_global_when_no_override() {
        let now = 1_700_000_000;
        let rows = [
            (1, None, Some(now - 30 * MIN)),     // 恰好到点
            (2, None, Some(now - 30 * MIN + 1)), // 差 1s 未到
            (3, None, None),                     // 从未抓过 → 立即到期
        ];
        assert_eq!(due_feed_ids(Some(30), &rows, now), vec![1, 3]);
        assert!(
            due_feed_ids(None, &rows, now).is_empty(),
            "全局 off 且无覆盖 → 谁都不排（连从未抓过的也不排）"
        );
    }

    #[test]
    fn override_wins_over_global_in_both_directions() {
        let now = 1_700_000_000;
        // 全局 120：覆盖 15 的源已到点；覆盖 360 的源不该被全局节奏带快
        let rows = [
            (1, Some(15), Some(now - 15 * MIN)),
            (2, Some(360), Some(now - 20 * MIN)),
        ];
        assert_eq!(due_feed_ids(Some(120), &rows, now), vec![1]);
        // 全局 15：覆盖 120 的源不该被全局节奏拖快（20 分钟差得远）
        let rows = [
            (1, Some(120), Some(now - 20 * MIN)),
            (2, None, Some(now - 20 * MIN)),
        ];
        assert_eq!(due_feed_ids(Some(15), &rows, now), vec![2]);
    }

    #[test]
    fn due_boundary_is_exactly_at_the_interval() {
        let now = 1_700_000_000;
        let at = |age_secs: i64, override_minutes| (1i64, override_minutes, Some(now - age_secs));
        // 全局档：差 1s 不算到点，恰好到点算
        assert_eq!(
            due_feed_ids(Some(60), &[at(60 * MIN - 1, None)], now),
            Vec::<i64>::new()
        );
        assert_eq!(due_feed_ids(Some(60), &[at(60 * MIN, None)], now), vec![1]);
        assert_eq!(
            due_feed_ids(Some(60), &[at(60 * MIN + 1, None)], now),
            vec![1]
        );
        // 覆盖档同一口径
        assert_eq!(
            due_feed_ids(Some(60), &[at(15 * MIN - 1, Some(15))], now),
            Vec::<i64>::new()
        );
        assert_eq!(
            due_feed_ids(Some(60), &[at(15 * MIN, Some(15))], now),
            vec![1]
        );
        // last_fetched_at 在未来（时钟回拨）：按刚抓过处理，不排
        assert_eq!(
            due_feed_ids(Some(15), &[at(-5, None)], now),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn global_off_still_fires_explicit_overrides() {
        let now = 1_700_000_000;
        let rows = [
            (1, None, Some(now - 10 * 360 * MIN)), // 跟随全局 + 全局 off → 不排
            (2, Some(15), Some(now - 15 * MIN)),   // 显式覆盖 → 照排
            (3, Some(15), Some(now - 14 * MIN)),   // 覆盖但没到点
        ];
        assert_eq!(due_feed_ids(None, &rows, now), vec![2]);
    }

    #[test]
    fn empty_scan_or_nothing_due_gives_empty_batch() {
        let now = 1_700_000_000;
        assert!(due_feed_ids(Some(30), &[], now).is_empty(), "一个源都没有");
        let fresh = [
            (1, None, Some(now)),
            (2, Some(15), Some(now - MIN)),
            (3, Some(360), Some(now - 30 * MIN)),
        ];
        assert!(due_feed_ids(Some(30), &fresh, now).is_empty(), "都没到点");
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

    /// 端到端（库 → 扫描 → 到期判定）：给一个源设 15 分钟覆盖并刚抓过，
    /// 断言扫描行真的是 `(id, Some(15), 时间戳)`，且刚抓过不排、时间戳拨到
    /// 16 分钟前就排（全局 off 也照排：覆盖优先）。
    #[test]
    fn store_scan_rows_drive_due_calculation() {
        let state = AppState::for_test();
        let now = now_secs();
        let id = state
            .with_store(|s| {
                let id = s
                    .add_feed("https://example.com/covered.xml", Some("覆盖源"))
                    .map_err(|e| e.to_string())?;
                s.set_feed_refresh_interval(id, Some(15))
                    .map_err(|e| e.to_string())?;
                s.record_fetch(id, "ok", None, None, None)
                    .map_err(|e| e.to_string())?;
                Ok(id)
            })
            .unwrap();
        let rows = state
            .with_store(|s| s.feeds_with_interval().map_err(|e| e.to_string()))
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, id);
        assert_eq!(rows[0].1, Some(15), "覆盖档位出库");
        assert!(rows[0].2.is_some(), "抓过之后扫描带上时间戳");

        // 刚抓过：全局 360 与覆盖 15 都没到点
        assert!(due_feed_ids(Some(360), &rows, now).is_empty());
        // 时间戳拨到 16 分钟前：覆盖源到点，且全局 off 不影响它
        let stale = [(id, Some(15), Some(now - 16 * MIN))];
        assert_eq!(due_feed_ids(None, &stale, now), vec![id]);
    }
}
