//! 系统托盘：图标 / 菜单 / 未读角标。
//!
//! **桌面端专属**：`tauri::tray` 只在 `all(desktop, feature = "tray-icon")` 下存在，
//! 所以构建入口 [`setup_tray`] 与配套辅助函数只在桌面编译；移动端保留
//! [`update_badge`] 的空实现，让调用方（commands / scheduler）不必各自加 cfg。
//!
//! **构建失败**的降级在调用方（`lib.rs` 的 setup）统一处理——这里不吞错；
//! **运行时**的角标更新则是静默 no-op：托盘不可用是预期内场景（Wayland 无
//! StatusNotifierItem、缺 libappindicator、headless 冒烟），不该往 stderr 刷错误。

use tauri::{AppHandle, Runtime};
#[cfg(desktop)]
use tauri::Manager;

#[cfg(desktop)]
use crate::commands;
#[cfg(desktop)]
use crate::state::AppState;

/// 托盘 id：与 `setup_tray` 里的 `TrayIconBuilder::with_id` 必须一致，
/// `update_badge` 靠它取回托盘（取不到即托盘不可用）。
#[cfg(desktop)]
const TRAY_ID: &str = "main-tray";

/// 角标圆点颜色（RGBA）。
#[cfg(desktop)]
const BADGE_COLOR: [u8; 4] = [229, 72, 77, 255];

/// 构建系统托盘：图标 + 「显示/隐藏窗口」「退出」菜单。任一步失败都原样返回
/// 错误，由调用方统一降级，不在托盘内部自行吞错。
///
/// 只在桌面端编译（`tauri::tray` 在移动端不存在）：Android 调用方在
/// `lib.rs` 的 setup 里被 cfg 挡住，不会走到这里。
#[cfg(desktop)]
pub fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::TrayIconBuilder;

    // 菜单文案跟随设置里的界面语言；`auto`/读不到时按 zh-CN 处理
    //（精确跟随系统语言需引入 sys-locale 依赖，v1 不做）。
    let (toggle_label, quit_label) = if locale(app.handle()) == "en" {
        ("Show/Hide Window", "Quit")
    } else {
        ("显示/隐藏窗口", "退出")
    };

    let toggle = MenuItemBuilder::with_id("tray-toggle", toggle_label).build(app)?;
    let quit = MenuItemBuilder::with_id("tray-quit", quit_label).build(app)?;
    let menu = MenuBuilder::new(app).items(&[&toggle, &quit]).build()?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("RustRss");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    app.on_menu_event(|app, event| match event.id().as_ref() {
        "tray-toggle" => {
            if let Some(win) = app.get_webview_window("main") {
                let visible = win.is_visible().unwrap_or(false);
                let _ = if visible { win.hide() } else { win.show() };
            }
        }
        "tray-quit" => app.exit(0),
        _ => {}
    });

    // 启动时就把角标摆正：库里已有的未读不该等到第一次后台刷新才可见
    // （自动刷新可以关掉，那时就永远等不到了）。
    update_badge(app.handle(), initial_unread(app));
    Ok(())
}

/// 启动时读一次未读总数；读不到按 0（角标只是装饰，不阻塞托盘构建）。
#[cfg(desktop)]
fn initial_unread(app: &tauri::App) -> i64 {
    app.try_state::<AppState>()
        .map(|s| commands::unread_total(&s).unwrap_or(0))
        .unwrap_or(0)
}

/// 同步托盘角标：未读 > 0 → 图标右上角加红点 + tooltip 带数字；= 0 → 恢复原图标。
///
/// 托盘不可用（取不到托盘 / 取不到图标）时静默 no-op——降级路径不产生错误日志。
/// 对 runtime 泛型：测试用 mock app（无托盘）也能走这条路径验「不 panic」。
/// Android 没有系统托盘：分派到空实现（调用方无需感知平台）。
pub fn update_badge<R: Runtime>(app: &AppHandle<R>, unread: i64) {
    #[cfg(desktop)]
    update_badge_desktop(app, unread);
    #[cfg(mobile)]
    let _ = (app, unread);
}

/// 桌面端的角标实现（`app.tray_by_id` 需要 `tauri::tray`，移动端不存在）。
#[cfg(desktop)]
fn update_badge_desktop<R: Runtime>(app: &AppHandle<R>, unread: i64) {
    // 同值短路：每次文章导航都会 sync_badge，COUNT 后的重绘+tooltip 才是可感开销；
    // 未读数没变就完全不碰托盘（swap 保证并发下不重不漏：并发同值时必有一方执行）。
    // -1 初值强制首绘；托盘从无到有的运行期重建不存在（启动时 setup_tray 内首绘）。
    static LAST_UNREAD: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
    use std::sync::atomic::Ordering::Relaxed;
    if LAST_UNREAD.swap(unread, Relaxed) == unread {
        return;
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return; // 托盘不可用是预期场景（headless / Wayland 无 SNI / 缺库）
    };
    let _ = tray.set_tooltip(Some(badge_tooltip(&locale(app), unread)));
    let Some(base) = app.default_window_icon() else {
        return;
    };
    let icon = if unread > 0 {
        match paint_badge(base.rgba(), base.width(), base.height()) {
            Some(rgba) => tauri::image::Image::new_owned(rgba, base.width(), base.height()),
            // 图标尺寸异常：宁可没有角标，也不要画出一张坏图
            None => base.clone().to_owned(),
        }
    } else {
        base.clone().to_owned()
    };
    let _ = tray.set_icon(Some(icon));
}

/// 托盘 tooltip：**保留 RustRss 品牌名**，后面缀未读数（0 时只有品牌名）。
#[cfg(desktop)]
fn badge_tooltip(locale: &str, unread: i64) -> String {
    if unread <= 0 {
        return "RustRss".to_string();
    }
    if locale.trim() == "en" {
        format!("RustRss · {unread} unread")
    } else {
        format!("RustRss · {unread} 篇未读")
    }
}

/// 界面语言设置（原样 `auto` / `zh-CN` / `en`）；读不到按空串（= 中文文案）。
#[cfg(desktop)]
fn locale<R: Runtime>(app: &AppHandle<R>) -> String {
    app.try_state::<AppState>()
        .map(|s| commands::ui_locale_setting(&s).unwrap_or_default())
        .unwrap_or_default()
}

/// 由窗口图标派生带角标的图标：在右上角画一个实心圆点（尺寸小，数字画不下——
/// 数字走 tooltip）。
///
/// 尺寸不合法（宽高为 0 / 字节数与宽高不符）时返回 `None`，调用方保持原图标。
#[cfg(desktop)]
fn paint_badge(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let expected = width as usize * height as usize * 4;
    if width == 0 || height == 0 || rgba.len() != expected {
        return None;
    }
    let radius = (width.min(height) / 6).max(2) as i32;
    let (cx, cy) = (width as i32 - radius - 1, radius + 1);
    let mut out = rgba.to_vec();
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy <= radius * radius {
                let i = (y as u32 * width + x as u32) as usize * 4;
                out[i..i + 4].copy_from_slice(&BADGE_COLOR);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_keeps_brand_and_appends_count() {
        assert_eq!(badge_tooltip("zh-CN", 0), "RustRss");
        assert_eq!(badge_tooltip("en", 0), "RustRss");
        assert_eq!(badge_tooltip("zh-CN", 7), "RustRss · 7 篇未读");
        assert_eq!(badge_tooltip("en", 7), "RustRss · 7 unread");
        for tooltip in [
            badge_tooltip("zh-CN", 7),
            badge_tooltip("en", 7),
            badge_tooltip("auto", 0),
        ] {
            assert!(
                tooltip.starts_with("RustRss"),
                "tooltip 必须保留品牌名，实际: {tooltip}"
            );
        }
    }

    #[test]
    fn paint_badge_draws_dot_in_top_right_only() {
        const W: u32 = 32;
        const H: u32 = 32;
        let base = vec![7u8; (W * H * 4) as usize];
        let out = paint_badge(&base, W, H).expect("尺寸合法应有结果");
        assert_eq!(out.len(), base.len(), "只改像素，不改尺寸");

        let radius = (W.min(H) / 6).max(2);
        let cx = W - radius - 1;
        let cy = radius + 1;
        let idx = (cy * W + cx) as usize * 4;
        assert_eq!(&out[idx..idx + 4], &BADGE_COLOR, "圆心应是角标色");
        assert_eq!(&out[0..4], &base[0..4], "左下角像素不受影响");
        assert_ne!(out, base, "确实画了角标");
    }

    #[test]
    fn paint_badge_refuses_malformed_icons() {
        let base = vec![7u8; 32 * 32 * 4];
        assert!(paint_badge(&base, 0, 32).is_none(), "宽为 0");
        assert!(paint_badge(&base, 32, 0).is_none(), "高为 0");
        assert!(paint_badge(&base, 32, 33).is_none(), "字节数与宽高不符");
        assert!(paint_badge(&[], 32, 32).is_none());
    }

    #[test]
    fn update_badge_without_a_tray_is_a_silent_noop() {
        // 托盘不可用（Wayland 无 SNI / 缺 libappindicator / 构建失败降级）时：
        // 角标更新不能 panic、也不写日志——降级不刷屏是这条路径的全部要求。
        let app = tauri::test::mock_app();
        assert!(app.tray_by_id(TRAY_ID).is_none(), "mock app 里没有托盘");
        update_badge(app.handle(), 5);
        update_badge(app.handle(), 0);
    }
}
