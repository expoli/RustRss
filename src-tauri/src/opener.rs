//! 移动端外链：把阅读器的「在浏览器打开」与正文锚点接到 Android 设备浏览器
//! （`ACTION_VIEW` intent）。
//!
//! 路由链：JS `open_external` 命令 → `validate_external_url`（只放行
//! http/https、拒绝空白/控制/元字符）→ 这里的 `PluginHandle.run_mobile_plugin`
//! → Kotlin `LinkPlugin`（app 模块内，`gen/android`，包名同应用；Kotlin 侧再
//! 校验一次 scheme 兜底）。桌面不编译本模块的插件部分：桌面 `open_external`
//! 维持原生子进程路径（commands.rs），行为不变。
//!
//! Kotlin 插件在 startActivity 外包 try/catch：模拟器/设备上没有可处理该
//! URL 的 activity 时（极罕见），返回明确错误而不是崩溃。

/// Android 插件句柄（setup 阶段注册完成后存入 app state）。
#[cfg(mobile)]
pub struct OpenMobile(pub tauri::plugin::PluginHandle<tauri::Wry>);

/// 注册 Kotlin `LinkPlugin`：必须在 `.setup()` 阶段（活动可用）执行。
#[cfg(mobile)]
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri::Manager;
    tauri::plugin::Builder::new("opener")
        .setup(|app, api| {
            let handle = api.register_android_plugin("tech.expoli.rustrss", "LinkPlugin")?;
            app.manage(OpenMobile(handle));
            Ok(())
        })
        .build()
}

/// 移动端打开已校验的 URL；桌面不编译（commands.rs 的 desktop 分支负责）。
#[cfg(mobile)]
pub fn open_url(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    use tauri::Manager;
    let handle = app.state::<OpenMobile>().0.clone();
    handle
        .run_mobile_plugin("openUrl", serde_json::json!({ "url": url }))
        .map_err(|e| e.to_string())
}
