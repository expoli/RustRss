//! Opt-in integration probe. Does not start the reader, MCP, or a database.
#[path = "../src/preview_capture.rs"]
mod preview_capture;

use preview_capture::{capture, CaptureError, MAX_PIXELS, MAX_PNG_BYTES};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{Manager, State, WebviewWindow};

struct Probe {
    output: PathBuf,
    revision: AtomicU64,
    busy: AtomicBool,
    records: Mutex<Vec<Value>>,
    display_backend: Mutex<String>,
}
fn authorize(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "probe" {
        return Err("invalid_window".into());
    }
    Ok(())
}
#[tauri::command]
fn begin_frame(window: WebviewWindow, state: State<'_, Probe>) -> Result<u64, String> {
    authorize(&window)?;
    Ok(state.revision.fetch_add(1, Ordering::SeqCst) + 1)
}
async fn capture_frame(
    window: &WebviewWindow,
    state: &Probe,
    revision: u64,
) -> Result<Value, String> {
    if state.revision.load(Ordering::SeqCst) != revision {
        return Err("revision_conflict".into());
    }
    state
        .busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "preview_busy")?;
    struct Release<'a>(&'a AtomicBool);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _guard = Release(&state.busy);
    let started = Instant::now();
    let image = capture(window, Duration::from_secs(10), MAX_PIXELS, MAX_PNG_BYTES)
        .await
        .map_err(|error| {
            serde_json::to_value(error)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })?;
    if state.revision.load(Ordering::SeqCst) != revision {
        return Err("revision_conflict".into());
    }
    let name = format!("frame-{revision:03}.png");
    std::fs::write(state.output.join(&name), &image.png).map_err(|e| e.to_string())?;
    let record = json!({"revision": revision, "file": name,
        "pixel_size": [image.width, image.height], "scale_factor": image.scale_factor,
        "logical_size": image.logical_size,
        "png_bytes": image.png.len(), "capture_ms": started.elapsed().as_secs_f64()*1000.0});
    state.records.lock().unwrap().push(record.clone());
    Ok(record)
}
#[tauri::command]
async fn capture_ready(
    window: WebviewWindow,
    state: State<'_, Probe>,
    revision: u64,
) -> Result<Value, String> {
    authorize(&window)?;
    capture_frame(&window, &state, revision).await
}
#[tauri::command]
async fn exercise_limits(window: WebviewWindow, state: State<'_, Probe>) -> Result<Value, String> {
    authorize(&window)?;
    let mut checks = Vec::new();
    window.hide().map_err(|e| e.to_string())?;
    wait_visible(&window, false).await?;
    let hidden = capture(&window, Duration::from_secs(5), MAX_PIXELS, MAX_PNG_BYTES)
        .await
        .err();
    window.show().map_err(|e| e.to_string())?;
    wait_visible(&window, true).await?;
    wait_renderable(&window).await?;
    checks.push(json!({"case":"hidden_window", "actual":hidden, "expected":"window_hidden"}));
    let pixels = capture(&window, Duration::from_secs(5), 1, MAX_PNG_BYTES)
        .await
        .err();
    checks.push(json!({"case":"pixel_budget", "actual":pixels, "expected":"image_too_large"}));
    let bytes = capture(&window, Duration::from_secs(5), MAX_PIXELS, 32)
        .await
        .err();
    checks.push(json!({"case":"png_byte_budget", "actual":bytes, "expected":"image_too_large"}));
    let zero = capture(&window, Duration::ZERO, MAX_PIXELS, MAX_PNG_BYTES)
        .await
        .err();
    checks.push(json!({"case":"expired_deadline", "actual":zero, "expected":"render_timeout"}));
    let timeout = capture(&window, Duration::from_nanos(1), MAX_PIXELS, MAX_PNG_BYTES)
        .await
        .err();
    checks.push(json!({"case":"native_deadline", "actual":timeout, "expected":"render_timeout"}));
    let old = state.revision.load(Ordering::SeqCst);
    let (stale, ()) = tokio::join!(capture_frame(&window, &state, old), async {
        state.revision.fetch_add(1, Ordering::SeqCst);
    });
    checks.push(
        json!({"case":"stale_completion", "actual":stale.err(), "expected":"revision_conflict"}),
    );
    let rev = state.revision.load(Ordering::SeqCst);
    let (first, second) = tokio::join!(
        capture_frame(&window, &state, rev),
        capture_frame(&window, &state, rev)
    );
    checks.push(
        json!({"case":"concurrent_capture", "actual":second.err(), "expected":"preview_busy"}),
    );
    if first.is_err() {
        return Err(format!("capture did not recover: {first:?}"));
    }
    let auxiliary = tauri::WebviewWindowBuilder::new(
        window.app_handle(),
        "closing-probe",
        tauri::WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    auxiliary.destroy().map_err(|e| e.to_string())?;
    let closed = capture(
        &auxiliary,
        Duration::from_secs(2),
        MAX_PIXELS,
        MAX_PNG_BYTES,
    )
    .await
    .err();
    checks.push(json!({"case":"closed_window", "actual":closed, "expected":"window_unavailable"}));
    if checks.iter().any(|c| c["actual"] != c["expected"]) {
        return Err(format!("probe checks failed: {checks:?}"));
    }
    Ok(json!(checks))
}

// Tao queues visibility changes; show()/hide() returning is not an acknowledgement.
// Poll the actual state with a bounded deadline instead of a fixed settle sleep.
async fn wait_visible(window: &WebviewWindow, expected: bool) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let visible = window.is_visible().map_err(|e| e.to_string())?;
        let minimized = window.is_minimized().map_err(|e| e.to_string())?;
        if visible == expected && (!expected || !minimized) {
            eprintln!(
                "visibility acknowledged: visible={visible} minimized={minimized} wait_ms={}",
                start.elapsed().as_millis()
            );
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(2) {
            return Err("visibility_timeout".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// After hide/show, GTK can deliver the old ICONIFIED event after is_visible=true.
// Require a successful native frame (including post-capture state checks).
async fn wait_renderable(window: &WebviewWindow) -> Result<(), String> {
    let start = Instant::now();
    let mut retries = 0;
    loop {
        match capture(window, Duration::from_secs(1), MAX_PIXELS, MAX_PNG_BYTES).await {
            Ok(_) => {
                eprintln!(
                    "renderability acknowledged: retries={retries} wait_ms={}",
                    start.elapsed().as_millis()
                );
                return Ok(());
            }
            Err(CaptureError::WindowHidden) if start.elapsed() < Duration::from_secs(2) => {
                retries += 1;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(format!("renderability failed: {error:?}")),
        }
    }
}
#[tauri::command]
fn finish_probe(
    window: WebviewWindow,
    state: State<'_, Probe>,
    details: Value,
) -> Result<(), String> {
    authorize(&window)?;
    let report = json!({"platform": std::env::consts::OS, "tauri":"2.11.6",
        "display_backend": *state.display_backend.lock().unwrap(),
        "scope":"isolated Tauri example; synthetic fixture; no database or MCP",
        "captures": *state.records.lock().unwrap(), "details": details});
    std::fs::write(
        state.output.join("results.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "{}",
        json!({"result":state.output.join("results.json"),"error":details.get("error")})
    );
    window
        .app_handle()
        .exit(if details.get("error").is_some() { 1 } else { 0 });
    Ok(())
}
fn main() {
    if !cfg!(target_os = "linux") {
        eprintln!(
            "{}",
            serde_json::to_string(&CaptureError::Unavailable).unwrap()
        );
        std::process::exit(2);
    }
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("usage: theme_snapshot NEW_OUTPUT_DIRECTORY"),
    );
    std::fs::create_dir(&output).expect("output directory must not already exist");
    let state = Probe {
        output,
        revision: AtomicU64::new(0),
        busy: AtomicBool::new(false),
        records: Mutex::new(Vec::new()),
        display_backend: Mutex::new("unknown".into()),
    };
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            begin_frame,
            capture_ready,
            exercise_limits,
            finish_probe
        ])
        .setup(|app| {
            #[cfg(target_os = "linux")]
            {
                use gtk::prelude::*;
                let window = app.get_webview_window("probe").unwrap();
                let backend = window.gtk_window()?.display().type_().name().to_owned();
                eprintln!("actual_display_backend={backend}");
                *app.state::<Probe>().display_backend.lock().unwrap() = backend;
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(120));
                eprintln!("probe watchdog expired");
                handle.exit(3);
            });
            Ok(())
        })
        .run(tauri::generate_context!(
            "examples/snapshot-probe/tauri.conf.json"
        ))
        .expect("run snapshot probe");
}
