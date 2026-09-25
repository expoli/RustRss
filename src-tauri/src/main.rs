//! RustRss 桌面应用启动器（Android 走 `lib.rs` 的 `mobile_entry_point`）。
//!
//! 只做一件事：把控制权交给共享入口 `rustrss_desktop_lib::run()`。
//! 所有注册与业务逻辑都在 `src/lib.rs`（桌面与移动端同一份）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rustrss_desktop_lib::run()
}
