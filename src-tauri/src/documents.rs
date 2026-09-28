//! 移动端文档读写：OPML 导入/导出经系统文档选择器（SAF）拿到的 `content://`
//! URI 不能用 `std::fs` 读写，这里经 Kotlin `DocumentsPlugin`
//! （ContentResolver 流）转发。桌面不编译本模块（commands.rs 分流到 std::fs）。

use serde_json::json;

/// setup 阶段注册 Kotlin `DocumentsPlugin` 后存入。
static HANDLE: std::sync::OnceLock<tauri::plugin::PluginHandle<tauri::Wry>> =
    std::sync::OnceLock::new();

/// 注册文档读写插件：必须在 `.setup()` 阶段（活动可用）执行。
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("documents")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("tech.expoli.rustrss", "DocumentsPlugin")?;
            let _ = HANDLE.set(handle);
            Ok(())
        })
        .build()
}

fn handle() -> Result<&'static tauri::plugin::PluginHandle<tauri::Wry>, String> {
    HANDLE.get().ok_or_else(|| "文档读写尚未就绪".into())
}

#[derive(serde::Deserialize)]
struct TextDocOut {
    #[serde(default)]
    text: String,
}

/// 读取系统文档选择器选中的文档全文。
pub fn read_text(uri: &str) -> Result<String, String> {
    let out: TextDocOut = handle()?
        .run_mobile_plugin("readText", json!({ "uri": uri }))
        .map_err(|e| format!("读取文档失败：{e}"))?;
    Ok(out.text)
}

/// 把全文写入系统文档选择器创建的文档。
pub fn write_text(uri: &str, content: &str) -> Result<(), String> {
    handle()?
        .run_mobile_plugin(
            "writeText",
            json!({ "uri": uri, "content": content }),
        )
        .map(|_: serde_json::Value| ())
        .map_err(|e| format!("写入文档失败：{e}"))
}
