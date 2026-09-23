//! Isolated real-component theme fixture. No application database or MCP.
#[path = "../src/preview_capture.rs"]
mod preview_capture;
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tauri::{Manager, State, WebviewWindow};
struct Output(PathBuf);
#[tauri::command]
fn narrow(window: WebviewWindow) -> Result<(), String> {
    window
        .set_size(tauri::LogicalSize::new(900., 600.))
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn snapshots() -> Vec<Value> {
    rustrss_core::theme::PRESETS
        .into_iter()
        .map(|id| {
            let config = rustrss_core::theme::ThemeConfig {
                light_preset: id,
                dark_preset: id,
                ..Default::default()
            };
            serde_json::to_value(config.resolve().unwrap()).unwrap()
        })
        .collect()
}
#[tauri::command]
async fn capture_scene(
    window: WebviewWindow,
    output: State<'_, Output>,
    name: String,
) -> Result<Value, String> {
    if !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err("invalid name".into());
    }
    let image = preview_capture::capture(
        &window,
        Duration::from_secs(10),
        preview_capture::MAX_PIXELS,
        preview_capture::MAX_PNG_BYTES,
    )
    .await
    .map_err(|e| format!("{e:?}"))?;
    std::fs::write(output.0.join(format!("{name}.png")), &image.png).map_err(|e| e.to_string())?;
    Ok(
        json!({"file":format!("{name}.png"),"pixels":[image.width,image.height],"bytes":image.png.len(),"scale":image.scale_factor,"logical_size":image.logical_size}),
    )
}
#[tauri::command]
fn finish(window: WebviewWindow, output: State<'_, Output>, report: Value) -> Result<(), String> {
    std::fs::write(
        output.0.join("results.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    window
        .app_handle()
        .exit(if report.get("error").is_some() { 1 } else { 0 });
    Ok(())
}
// Isolated editor IPC: the same core methods used by desktop command glue.
struct EditorStore(std::sync::Mutex<rustrss_core::Store>);
#[tauri::command]
fn editor_state(store: State<'_, EditorStore>) -> Value {
    let s = store.0.lock().unwrap();
    json!({"theme_snapshot":s.theme_snapshot().unwrap(),"history":s.theme_history().unwrap()})
}
#[tauri::command]
fn validate_ui_theme(store: State<'_, EditorStore>, expected_revision: u64, patch: rustrss_core::theme::ThemePatch) -> Result<rustrss_core::theme::ThemeSnapshot, String> {
    store.0.lock().unwrap().validate_theme_patch(expected_revision, &patch).map_err(|e|e.to_string())
}
#[tauri::command]
fn update_ui_theme(store: State<'_, EditorStore>, expected_revision: u64, patch: rustrss_core::theme::ThemePatch) -> Result<Value, String> {
    let snapshot = store.0.lock().unwrap().update_theme(expected_revision, &patch).map_err(|e|e.to_string())?;
    Ok(json!({"theme_snapshot":snapshot}))
}
#[tauri::command]
fn get_ui_theme_history(store: State<'_, EditorStore>) -> Vec<rustrss_core::theme::ThemeConfig> {
    store.0.lock().unwrap().theme_history().unwrap()
}
#[tauri::command]
fn restore_ui_theme(store: State<'_, EditorStore>, expected_revision: u64, historical_revision: u64) -> Result<Value, String> {
    let snapshot = store.0.lock().unwrap().restore_theme(expected_revision, historical_revision).map_err(|e|e.to_string())?;
    Ok(json!({"theme_snapshot":snapshot}))
}
fn main() {
    if !cfg!(target_os = "linux") {
        eprintln!("{:?}", preview_capture::CaptureError::Unavailable);
        std::process::exit(2);
    }
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("theme_ui NEW_OUTPUT_DIRECTORY"),
    );
    std::fs::create_dir(&output).expect("output must not exist");
    let mut context = tauri::generate_context!("examples/snapshot-probe/tauri.conf.json");
    context.config_mut().app.windows.clear();
    let settings_probe = std::env::args().any(|arg| arg == "--settings");
    tauri::Builder::default()
        .manage(EditorStore(std::sync::Mutex::new(rustrss_core::Store::open(":memory:").unwrap())))
        .manage(Output(output))
        .register_uri_scheme_protocol("fixture", move |_ctx, request| {
            let (mime, body) = match request.uri().path() {
                "/index.html" | "/" => (
                    "text/html",
                    include_str!("../../ui/index.html")
                        .replace("src=\"app.js\"", "src=\"fixture.js\"")
                        .into_bytes(),
                ),
                "/fixture.js" => (
                    "text/javascript",
                    if settings_probe { include_bytes!("theme-ui/settings-fixture.js").to_vec() } else { include_bytes!("theme-ui/fixture.js").to_vec() },
                ),
                "/theme-settings.js" => ("text/javascript", include_bytes!("../../ui/theme-settings.js").to_vec()),
                "/theme.js" => (
                    "text/javascript",
                    include_bytes!("../../ui/theme.js").to_vec(),
                ),
                "/theme-sync.js" => (
                    "text/javascript",
                    include_bytes!("../../ui/theme-sync.js").to_vec(),
                ),
                "/components.js" => (
                    "text/javascript",
                    include_bytes!("../../ui/components.js").to_vec(),
                ),
                "/i18n.js" => (
                    "text/javascript",
                    include_bytes!("../../ui/i18n.js").to_vec(),
                ),
                "/style.css" => ("text/css", include_bytes!("../../ui/style.css").to_vec()),
                "/vendor/highlight.min.js" => (
                    "text/javascript",
                    include_bytes!("../../ui/vendor/highlight.min.js").to_vec(),
                ),
                _ => ("text/plain", Vec::new()),
            };
            tauri::http::Response::builder()
                .header("Content-Type", mime)
                .body(body)
                .unwrap()
        })
        .invoke_handler(tauri::generate_handler![
            editor_state, validate_ui_theme, update_ui_theme, get_ui_theme_history, restore_ui_theme,
            snapshots,
            capture_scene,
            finish,
            narrow
        ])
        .setup(|app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "probe",
                tauri::WebviewUrl::CustomProtocol(
                    "fixture://localhost/index.html".parse().unwrap(),
                ),
            )
            .title("RustRss theme UI fixture")
            .inner_size(1240., 820.)
            .build()?;
            #[cfg(target_os = "linux")]
            {
                use gtk::prelude::*;
                eprintln!(
                    "actual_display_backend={}",
                    window.gtk_window()?.display().type_().name()
                );
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(120));
                handle.exit(3);
            });
            Ok(())
        })
        .run(context)
        .expect("theme fixture");
}
