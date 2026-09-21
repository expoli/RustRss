//! 新文章系统通知：Rust 侧驱动，不依赖前端存活。
//!
//! 只在**后台刷新**路径被调用（`scheduler::background_refresh`）——手动刷新时用户
//! 就在界面前，新条目已经在列表里，再弹一条系统通知只是打扰。
//!
//! 判定与文案是纯函数（文件末单测覆盖）；这里的胶水只做三件事：读界面语言设置 →
//! 组文案 → 交给 `tauri-plugin-notification` 展示。
//!
//! 点击行为：插件的 action 事件只有移动端有，桌面端没有点击回调。点击后的行为
//! 交给系统/桌面环境（Windows/macOS 点击会激活应用；Linux 依 DE 而定，最差是仅
//! 展示）。不为它加平台分支，也不让它影响刷新路径。

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::commands;
use crate::state::AppState;

/// 判定：要不要发通知、发几条（`None` = 不发）。
///
/// 负差（刷新期间用户读了文章，未读反而少了）不是「新文章」，归零处理。
pub fn decide(enabled: bool, before_unread: i64, after_unread: i64) -> Option<i64> {
    let new_unread = (after_unread - before_unread).max(0);
    (enabled && new_unread > 0).then_some(new_unread)
}

/// 通知文案（标题 + 正文）。Rust 侧双语常量，按 `ui.locale` 选——与托盘菜单同一
/// 模式（界面文案的唯一出处仍是 `ui/i18n.js`，这里只管 Rust 自己发的通知）。
pub fn copy(locale: &str, count: i64) -> (&'static str, String) {
    if locale.trim() == "en" {
        let body = if count == 1 {
            "1 new article".to_string()
        } else {
            format!("{count} new articles")
        };
        ("RustRss", body)
    } else {
        // `auto`（跟随系统）与读不到设置时按中文，与托盘菜单同一口径
        ("RustRss", format!("{count} 篇新文章"))
    }
}

/// 后台刷新前后比对未读数，有新增且开关打开时弹一条**聚合**通知。
///
/// `enabled` 由调用方传入而不是在这里读库：判定因此是纯函数，调度器也能在刷新
/// 开始前把开关读定，不受刷新期间用户改设置影响。
pub fn maybe_notify(app: &AppHandle, before_unread: i64, after_unread: i64, enabled: bool) {
    let Some(count) = decide(enabled, before_unread, after_unread) else {
        return;
    };
    let locale = app
        .try_state::<AppState>()
        .map(|s| commands::ui_locale_setting(&s).unwrap_or_default())
        .unwrap_or_default();
    let (title, body) = copy(&locale, count);
    // 一行日志：通知是「异步发出去」的（插件在后台任务里调 notify-rust），
    // 有它才能在 headless 冒烟里核对「后台刷新确实触发了通知」。
    eprintln!("[rustrss] 新文章通知: {count} 篇（locale={locale}）");
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        // 发不出去（无通知守护进程、会话没有 D-Bus 服务等）不该影响刷新与角标
        eprintln!("[rustrss] 系统通知发送失败（忽略）: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_needs_enabled_switch_and_positive_delta() {
        assert_eq!(decide(true, 3, 8), Some(5), "开关开 + 新增 5 → 通知 5 条");
        assert_eq!(decide(true, 0, 1), Some(1), "只新增 1 条也要通知");
        assert_eq!(decide(true, 8, 8), None, "没有新增不发");
        assert_eq!(decide(false, 3, 8), None, "开关关（默认）不发");
        assert_eq!(
            decide(true, 8, 3),
            None,
            "刷新期间读掉了一些：不是新文章，不发"
        );
        assert_eq!(decide(false, 8, 3), None, "开关关时负差也不发");
    }

    #[test]
    fn copy_follows_locale_and_english_plural() {
        assert_eq!(copy("en", 1), ("RustRss", "1 new article".to_string()));
        assert_eq!(copy("en", 3), ("RustRss", "3 new articles".to_string()));
        assert_eq!(copy("zh-CN", 3), ("RustRss", "3 篇新文章".to_string()));
        // `auto`（跟随系统）与读不到设置时按中文，与托盘菜单同一口径
        assert_eq!(copy("auto", 1), ("RustRss", "1 篇新文章".to_string()));
        assert_eq!(copy("", 2), ("RustRss", "2 篇新文章".to_string()));
        assert_eq!(copy(" en ", 2).1, "2 new articles", "带空白的值也要认");
    }
}
