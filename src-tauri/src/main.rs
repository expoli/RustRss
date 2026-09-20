// RustRss M0 探针：只做「能不能做真」的验证，不含任何业务逻辑。
// 前端把诊断数据通过 probe_log 打到 stdout，便于在终端侧与外部指标（RSS/启动耗时）对齐。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, PhysicalPosition};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// 前端调用：把一行诊断信息打到 stdout（终端可见，便于自动采集）
#[tauri::command]
fn probe_log(line: String) {
    println!("[probe] {line}");
}

/// 走 Tauri 剪贴板插件（Rust 侧）写系统剪贴板。
/// 这条路径不依赖 WebKit 的 Web Clipboard API，是产品里可以依赖的可靠通道。
#[tauri::command]
fn clip_write(app: tauri::AppHandle, text: String) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// 读系统剪贴板（用于验证写入是否真的落到了系统剪贴板）
#[tauri::command]
fn clip_read(app: tauri::AppHandle) -> Result<String, String> {
    app.clipboard().read_text().map_err(|e| e.to_string())
}

/// 窗口与显示器状态：多屏 / 缩放场景下定位窗口到底落在哪里（手工拼 JSON，不引入额外依赖）
#[tauri::command]
fn win_info(window: tauri::Window) -> String {
    let mut s = String::from("{");

    match window.is_visible() {
        Ok(v) => s.push_str(&format!("\"visible\":{v},")),
        Err(e) => s.push_str(&format!("\"visible_err\":\"{e}\",")),
    }
    match window.is_minimized() {
        Ok(v) => s.push_str(&format!("\"minimized\":{v},")),
        Err(e) => s.push_str(&format!("\"minimized_err\":\"{e}\",")),
    }
    match window.outer_position() {
        Ok(p) => s.push_str(&format!("\"pos_phys\":[{},{}],", p.x, p.y)),
        Err(e) => s.push_str(&format!("\"pos_err\":\"{e}\",")),
    }
    match window.outer_size() {
        Ok(sz) => s.push_str(&format!("\"outer_phys\":[{},{}],", sz.width, sz.height)),
        Err(e) => s.push_str(&format!("\"outer_err\":\"{e}\",")),
    }
    match window.scale_factor() {
        Ok(f) => s.push_str(&format!("\"scale_factor\":{f},")),
        Err(e) => s.push_str(&format!("\"scale_err\":\"{e}\",")),
    }
    match window.current_monitor() {
        Ok(Some(m)) => s.push_str(&format!(
            "\"monitor\":{{\"name\":\"{}\",\"size_phys\":[{},{}],\"scale\":{},\"pos_phys\":[{},{}]}},",
            m.name().map(|n| n.as_str()).unwrap_or("?"),
            m.size().width,
            m.size().height,
            m.scale_factor(),
            m.position().x,
            m.position().y
        )),
        Ok(None) => s.push_str("\"monitor\":null,"),
        Err(e) => s.push_str(&format!("\"monitor_err\":\"{e}\",")),
    }

    s.push_str("\"monitors\":[");
    if let Ok(list) = window.available_monitors() {
        let items: Vec<String> = list
            .iter()
            .map(|m| {
                format!(
                    "{{\"name\":\"{}\",\"size_phys\":[{},{}],\"scale\":{},\"pos_phys\":[{},{}]}}",
                    m.name().map(|n| n.as_str()).unwrap_or("?"),
                    m.size().width,
                    m.size().height,
                    m.scale_factor(),
                    m.position().x,
                    m.position().y
                )
            })
            .collect();
        s.push_str(&items.join(","));
    }
    s.push_str("]}");
    s
}

/// 把窗口移到指定物理坐标（多屏时用来把探针搬到你在看的那块屏）
#[tauri::command]
fn win_move(window: tauri::Window, x: i32, y: i32) -> Result<(), String> {
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

/// 主动置前并抢焦点（Wayland 下用于确认窗口是否只是被挡住了）
#[tauri::command]
fn win_focus(window: tauri::Window) -> Result<(), String> {
    window.set_focus().map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // 自测：RUSTSS_CLIP_TEST=1 时用插件写一段可识别文本，并读回，
            // 再由外部（xsel/wl-paste）验系统剪贴板是否真的拿到了它。
            if std::env::var("RUSTSS_CLIP_TEST").is_ok() {
                let text = format!("RUSTSS-CLIP-AUTO-{}", std::process::id());
                match app.clipboard().write_text(&text) {
                    Ok(()) => println!("[probe] CLIP-AUTO-WRITE ok text={text}"),
                    Err(e) => println!("[probe] CLIP-AUTO-WRITE err={e}"),
                }
                match app.clipboard().read_text() {
                    Ok(v) => println!("[probe] CLIP-AUTO-READ ok value={v} same={}", v == text),
                    Err(e) => println!("[probe] CLIP-AUTO-READ err={e}"),
                }
            }

            // 多屏环境下用来确定性地把探针窗口放到指定物理坐标：
            //   RUSTSS_WIN_POS="4656,120" 或 RUSTSS_WIN_POS="laptop"（自动放到笔记本内屏）
            if let Ok(spec) = std::env::var("RUSTSS_WIN_POS") {
                let win = app
                    .get_webview_window("main")
                    .expect("找不到 main 窗口");
                let target = if spec.trim() == "laptop" {
                    // 挑面积最小的那块（通常就是笔记本内屏），放到它左上角内缩一点的位置
                    win.available_monitors()
                        .ok()
                        .and_then(|ms| ms.into_iter().min_by_key(|m| m.size().width * m.size().height))
                        .map(|m| (m.position().x + 200, m.position().y + 120))
                } else {
                    let mut it = spec.split(',');
                    match (
                        it.next().and_then(|v| v.trim().parse::<i32>().ok()),
                        it.next().and_then(|v| v.trim().parse::<i32>().ok()),
                    ) {
                        (Some(x), Some(y)) => Some((x, y)),
                        _ => None,
                    }
                };
                match target {
                    Some((x, y)) => {
                        let _ = win.set_position(PhysicalPosition::new(x, y));
                        println!("[probe] WIN-PLACED pos=[{x},{y}] spec={spec}");
                    }
                    None => println!("[probe] WIN-PLACE-FAILED spec={spec}"),
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe_log, clip_write, clip_read, win_info, win_move, win_focus
        ])
        .run(tauri::generate_context!())
        .expect("RustRss M0 探针启动失败");
}
