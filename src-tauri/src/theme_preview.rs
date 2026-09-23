//! Thin desktop adapter: fixed UI fixture, validated ready handshake, native capture.
use rustrss_mcp::preview::{fingerprint, Backend, Frame, RenderFuture, RenderRequest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{Manager, WebviewWindow};
const LABEL: &str = "theme-preview";
#[derive(Clone, Serialize)]
struct Payload {
    #[serde(flatten)]
    request: RenderRequest,
    marker: Vec<String>,
}
#[derive(Clone, Deserialize)]
pub struct Ready {
    request_id: String,
    preview_revision: u64,
    config_hash: String,
    scene: String,
    mode: String,
    viewport: [u32; 2],
    dpr: f64,
    fonts: Value,
}
struct Pending {
    payload: Payload,
    sender: Option<tokio::sync::oneshot::Sender<Ready>>,
}
#[derive(Default)]
struct Shared {
    pending: Mutex<Option<Pending>>,
    dismissed: AtomicBool,
}
pub fn register(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.manage(Arc::new(Shared::default())).register_uri_scheme_protocol("theme-fixture", |_ctx,request| {
        let (mime,body)=match request.uri().path() {
            "/"|"/index.html" => ("text/html",include_str!("../../ui/index.html").replace("src=\"app.js\"","src=\"preview.js\"").into_bytes()),
            "/preview.js" => ("text/javascript",include_bytes!("../../ui/preview.js").to_vec()),
            "/theme-settings.js" => ("text/javascript",include_bytes!("../../ui/theme-settings.js").to_vec()),
            "/theme.js" => ("text/javascript",include_bytes!("../../ui/theme.js").to_vec()),
            "/theme-sync.js" => ("text/javascript",include_bytes!("../../ui/theme-sync.js").to_vec()),
            "/components.js" => ("text/javascript",include_bytes!("../../ui/components.js").to_vec()),
            "/i18n.js" => ("text/javascript",include_bytes!("../../ui/i18n.js").to_vec()),
            "/style.css" => ("text/css",include_bytes!("../../ui/style.css").to_vec()),
            "/vendor/highlight.min.js" => ("text/javascript",include_bytes!("../../ui/vendor/highlight.min.js").to_vec()),
            _=>("text/plain",Vec::new()),
        };
        tauri::http::Response::builder().header("Content-Type",mime)
            .header("Content-Security-Policy","default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src data:; font-src 'self' data:; connect-src ipc: http://ipc.localhost")
            .body(body).unwrap()
    })
}
fn check(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == LABEL {
        Ok(())
    } else {
        Err("invalid_window".into())
    }
}
#[tauri::command]
pub fn preview_request(window: WebviewWindow, app: tauri::AppHandle) -> Result<Value, String> {
    check(&window)?;
    let shared = app.state::<Arc<Shared>>();
    let guard = shared.pending.lock().unwrap();
    Ok(guard
        .as_ref()
        .map(|p| serde_json::to_value(&p.payload).unwrap())
        .unwrap_or(Value::Null))
}
fn matches_ready(payload: &Payload, ready: &Ready) -> bool {
    ready.request_id == payload.request.request_id
        && ready.preview_revision == payload.request.preview_revision
        && ready.config_hash == payload.request.snapshot.config_hash
        && json!(ready.scene) == json!(payload.request.scene)
        && json!(ready.mode) == json!(payload.request.mode)
        && ready.viewport[0] > 0
        && ready.viewport[1] > 0
        && ready.dpr.is_finite()
        && ready.dpr > 0.
        && ready.dpr <= 4.
}
#[tauri::command]
pub fn preview_ready(
    window: WebviewWindow,
    app: tauri::AppHandle,
    ready: Ready,
) -> Result<(), String> {
    check(&window)?;
    if serde_json::to_vec(&ready.fonts)
        .map_err(|_| "invalid_fonts")?
        .len()
        > 16384
    {
        return Err("invalid_fonts".into());
    }
    let shared = app.state::<Arc<Shared>>();
    let mut pending = shared.pending.lock().unwrap();
    let p = pending.as_mut().ok_or("stale_render")?;
    if !matches_ready(&p.payload, &ready) {
        log::warn!(
            "theme preview rejected ready: viewport={:?} dpr={} request_match={} revision_match={}",
            ready.viewport,
            ready.dpr,
            ready.request_id == p.payload.request.request_id,
            ready.preview_revision == p.payload.request.preview_revision
        );
        return Err("stale_render".into());
    }
    if let Some(sender) = p.sender.take() {
        let _ = sender.send(ready);
    }
    Ok(())
}
#[tauri::command]
pub fn preview_cancel(window: WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    check(&window)?;
    let shared = app.state::<Arc<Shared>>();
    shared.dismissed.store(true, Ordering::SeqCst);
    shared.pending.lock().unwrap().take();
    window.destroy().map_err(|e| e.to_string())
}
struct Desktop {
    app: tauri::AppHandle,
    shared: Arc<Shared>,
}
pub fn backend(app: tauri::AppHandle) -> Arc<dyn Backend> {
    let shared = app.state::<Arc<Shared>>().inner().clone();
    Arc::new(Desktop { app, shared })
}
struct PendingGuard {
    shared: Arc<Shared>,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        let mut p = self.shared.pending.lock().unwrap();
        if p.as_ref()
            .is_some_and(|p| p.payload.request.request_id == self.id)
        {
            p.take();
        }
    }
}
impl Backend for Desktop {
    fn dismissed(&self) -> bool {
        self.shared.dismissed.load(Ordering::SeqCst)
    }
    fn close(&self) {
        self.shared.pending.lock().unwrap().take();
        if let Some(w) = self.app.get_webview_window(LABEL) {
            let _ = w.destroy();
        }
    }
    fn render(&self, request: RenderRequest) -> RenderFuture {
        let app = self.app.clone();
        let shared = self.shared.clone();
        Box::pin(async move {
            let started = Instant::now();
            let hash = fingerprint(&request.request_id);
            let marker = (0..8)
                .map(|i| format!("#{}", &hash[i * 6..i * 6 + 6]))
                .collect::<Vec<_>>();
            let payload = Payload {
                request: request.clone(),
                marker: marker.clone(),
            };
            let (tx, rx) = tokio::sync::oneshot::channel();
            *shared.pending.lock().unwrap() = Some(Pending {
                payload,
                sender: Some(tx),
            });
            let _guard = PendingGuard {
                shared: shared.clone(),
                id: request.request_id.clone(),
            };
            shared.dismissed.store(false, Ordering::SeqCst);
            let (window_tx, window_rx) = tokio::sync::oneshot::channel();
            let target = app.clone();
            let close_state = shared.clone();
            app.run_on_main_thread(move || {
                let result = (|| {
                    if window_tx.is_closed() { return Err("render_timeout"); }
                    if let Some(w) = target.get_webview_window(LABEL) {
                        w.show().map_err(|_| "window_unavailable")?;
                        return Ok(w);
                    }
                    let url = "theme-fixture://localhost/index.html".parse().unwrap();
                    let w = tauri::WebviewWindowBuilder::new(
                        &target,
                        LABEL,
                        tauri::WebviewUrl::External(url),
                    )
                    .title("RustRss — Theme preview")
                    .inner_size(1280., 900.)
                    .resizable(false)
                    .decorations(false)
                    .on_navigation(|url| url.scheme() == "theme-fixture")
                    .build()
                    .map_err(|_| "window_unavailable")?;
                    w.on_window_event(move |event| {
                        if matches!(event, tauri::WindowEvent::Destroyed) {
                            close_state.dismissed.store(true, Ordering::SeqCst);
                            close_state.pending.lock().unwrap().take();
                        }
                    });
                    Ok::<_, &'static str>(w)
                })();
                if let Err(Ok(window)) = window_tx.send(result) { let _ = window.destroy(); }
            })
            .map_err(|_| "window_unavailable")?;
            let window = window_rx.await.map_err(|_| "window_unavailable")??;
            window
                .eval("window.dispatchEvent(new Event('theme-preview-request'))")
                .map_err(|_| "window_unavailable")?;
            let mut ready = rx.await.map_err(|_| "window_unavailable")?;
            if ready.viewport != [1280, 900] && ready.viewport != [960, 640] {
                // A Wayland compositor may constrain the initial size to its work area.
                // Negotiate one smaller fixed content viewport, then await a new ready.
                let (tx, rx) = tokio::sync::oneshot::channel();
                if let Some(p) = shared.pending.lock().unwrap().as_mut() {
                    p.sender = Some(tx);
                }
                window
                    .set_size(tauri::LogicalSize::new(960., 640.))
                    .map_err(|_| "viewport_mismatch")?;
                ready = rx.await.map_err(|_| "window_unavailable")?;
                if ready.viewport != [960, 640] {
                    return Err("viewport_mismatch");
                }
            }
            loop {
                if shared.dismissed.load(Ordering::SeqCst) {
                    return Err("window_unavailable");
                }
                let image = crate::preview_capture::capture(
                    &window,
                    Duration::from_secs(10).saturating_sub(started.elapsed()),
                    6_000_000,
                    2 * 1024 * 1024,
                )
                .await
                .map_err(|e| match e {
                    crate::preview_capture::CaptureError::ImageTooLarge => "image_too_large",
                    crate::preview_capture::CaptureError::RenderTimeout => "render_timeout",
                    _ => "capture_failed",
                })?;
                if image.logical_size != ready.viewport
                    || (f64::from(image.scale_factor) - ready.dpr).abs() > 0.01
                {
                    return Err("viewport_mismatch");
                }
                let expected = marker.clone();
                let scale = image.scale_factor;
                let verified = tokio::task::spawn_blocking(move || {
                    let fresh = verify_marker(&image.png, &expected, scale);
                    (image, fresh)
                })
                .await
                .map_err(|_| "capture_failed")?;
                let (image, fresh) = verified;
                if fresh {
                    let current = shared
                        .pending
                        .lock()
                        .unwrap()
                        .as_ref()
                        .is_some_and(|p| p.payload.request.request_id == request.request_id);
                    if !current {
                        return Err("stale_render");
                    }
                    return Ok(Frame {
                        png: image.png,
                        metadata: json!({"display_backend":image.display_backend,"logical_size":image.logical_size,"pixel_size":[image.width,image.height],"scale_factor":image.scale_factor,"output_scale":1,"capture_ms":started.elapsed().as_secs_f64()*1000.,"captured_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),"fonts":ready.fonts,"freshness_marker_verified":true}),
                    });
                }
                // Retry only after checking the actual native pixels, never assume rAF implies a fresh compositor frame.
                if started.elapsed() >= Duration::from_secs(9) {
                    return Err("render_timeout");
                }
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
        })
    }
}
#[cfg(target_os = "linux")]
fn verify_marker(bytes: &[u8], colors: &[String], scale: u32) -> bool {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let Ok(mut reader) = decoder.read_info() else {
        return false;
    };
    let mut data = vec![0; reader.output_buffer_size()];
    let Ok(info) = reader.next_frame(&mut data) else {
        return false;
    };
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        _ => return false,
    };
    colors.iter().enumerate().all(|(i, c)| {
        let x = (i as u32 * 4 + 2) * scale;
        let y = 2 * scale;
        let offset = ((y * info.width + x) * channels) as usize;
        let expected = (0..3)
            .map(|k| u8::from_str_radix(&c[1 + k * 2..3 + k * 2], 16).unwrap())
            .collect::<Vec<_>>();
        data.get(offset..offset + 3) == Some(expected.as_slice())
    })
}
#[cfg(not(target_os = "linux"))]
fn verify_marker(_: &[u8], _: &[String], _: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ready_requires_request_revision_hash_scene_mode_and_content_geometry() {
        let request = RenderRequest {
            request_id: "request".into(),
            preview_revision: 2,
            snapshot: rustrss_core::theme::ThemeConfig::default()
                .resolve()
                .unwrap(),
            scene: rustrss_mcp::preview::Scene::Article,
            mode: rustrss_mcp::preview::Mode::Light,
        };
        let payload = Payload {
            request,
            marker: vec![],
        };
        let ready = Ready {
            request_id: "request".into(),
            preview_revision: 2,
            config_hash: payload.request.snapshot.config_hash.clone(),
            scene: "article".into(),
            mode: "light".into(),
            viewport: [1280, 900],
            dpr: 1.,
            fonts: json!({}),
        };
        assert!(matches_ready(&payload, &ready));
        let mut wrong = ready.clone();
        wrong.request_id = "old".into();
        assert!(!matches_ready(&payload, &wrong));
        let mut wrong = ready.clone();
        wrong.preview_revision = 1;
        assert!(!matches_ready(&payload, &wrong));
        let mut wrong = ready.clone();
        wrong.config_hash = "old".into();
        assert!(!matches_ready(&payload, &wrong));
        let mut wrong = ready.clone();
        wrong.scene = "settings".into();
        assert!(!matches_ready(&payload, &wrong));
        let mut wrong = ready.clone();
        wrong.mode = "dark".into();
        assert!(!matches_ready(&payload, &wrong));
        let mut wrong = ready.clone();
        wrong.viewport = [0, 0];
        assert!(!matches_ready(&payload, &wrong));
        let mut wrong = ready;
        wrong.dpr = f64::NAN;
        assert!(!matches_ready(&payload, &wrong));
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn native_marker_rejects_stale_pixels() {
        let colors = vec!["#112233".into(); 8];
        let mut raw = vec![0u8; 40 * 8 * 3];
        for i in 0..8 {
            let offset = (2 * 40 + i * 4 + 2) * 3;
            raw[offset..offset + 3].copy_from_slice(&[17, 34, 51]);
        }
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 40, 8);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&raw)
                .unwrap();
        }
        assert!(verify_marker(&bytes, &colors, 1));
        let mut stale = colors;
        stale[7] = "#112234".into();
        assert!(!verify_marker(&bytes, &stale, 1));
    }
}
