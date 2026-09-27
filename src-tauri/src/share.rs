//! 移动端分享：把阅读器的「分享」按钮接到 Android 系统分享面板
//! （`ACTION_SEND` chooser）。
//!
//! 路由链：JS `share_url` 命令 → 这里的 `PluginHandle.run_mobile_plugin`
//! → Kotlin `SharePlugin`（app 模块内，`gen/android`，包名同应用）。
//! 桌面不编译本模块的插件部分：JS 侧按 `pointer: coarse` 分流，桌面
//! 「分享」回退为写剪贴板（shareEntry，app.js）。

/// Android 插件句柄（setup 阶段注册完成后存入 app state）。
#[cfg(mobile)]
pub struct ShareMobile(pub tauri::plugin::PluginHandle<tauri::Wry>);

/// 注册 Kotlin `SharePlugin`：必须在 `.setup()` 阶段（活动可用）执行。
#[cfg(mobile)]
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri::Manager;
    tauri::plugin::Builder::new("share")
        .setup(|app, api| {
            let handle = api.register_android_plugin("tech.expoli.rustrss", "SharePlugin")?;
            app.manage(ShareMobile(handle));
            Ok(())
        })
        .build()
}

/// 阅读器「分享」：移动端拉起系统分享面板；桌面命令不达（JS 分流到剪贴板），
/// 保留显式错误分支只为注册表在两个平台都成立。
#[tauri::command]
pub async fn share_url(app: tauri::AppHandle, title: String, url: String) -> Result<(), String> {
    #[cfg(mobile)]
    {
        use tauri::Manager;
        let handle = app.state::<ShareMobile>().0.clone();
        handle
            .run_mobile_plugin("shareUrl", serde_json::json!({ "title": title, "url": url }))
            .map_err(|e| e.to_string())
    }
    #[cfg(desktop)]
    {
        let _ = (app, title, url);
        Err("分享需要移动端系统能力，桌面端请使用「复制链接」".into())
    }
}
