//! Native WebView capture shared by production theme previews and opt-in probes.
//! No database, network, or desktop capture.
use std::time::Duration;
use tauri::WebviewWindow;

pub const MAX_PIXELS: u64 = 6_000_000;
pub const MAX_PNG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureError {
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Unavailable,
    // 以下变体只在桌面路径构造；移动端保留同一份错误分类（与桌面共用 render 流程）。
    #[cfg_attr(mobile, allow(dead_code))]
    WindowUnavailable,
    #[cfg_attr(mobile, allow(dead_code))]
    WindowHidden,
    #[cfg_attr(mobile, allow(dead_code))]
    ImageTooLarge,
    #[cfg_attr(mobile, allow(dead_code))]
    CaptureFailed,
    RenderTimeout,
}

pub struct Capture {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub logical_size: [u32; 2],
    pub scale_factor: f64,
    #[allow(dead_code)] // Standalone examples do not all consume backend metadata.
    pub display_backend: String,
}

/// Deadlines include dispatch, native capture, and PNG encoding. Callers must
/// independently gate the request revision before dispatch and after completion.
pub async fn capture(
    window: &WebviewWindow,
    deadline: Duration,
    max_pixels: u64,
    max_bytes: usize,
) -> Result<Capture, CaptureError> {
    if deadline.is_zero() {
        return Err(CaptureError::RenderTimeout);
    }
    let pixels = max_pixels.min(MAX_PIXELS);
    let bytes = max_bytes.min(MAX_PNG_BYTES);
    tokio::time::timeout(deadline, platform_capture(window, pixels, bytes))
        .await
        .map_err(|_| CaptureError::RenderTimeout)?
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
async fn platform_capture(_: &WebviewWindow, _: u64, _: usize) -> Result<Capture, CaptureError> {
    Err(CaptureError::Unavailable)
}

#[cfg(target_os = "windows")]
async fn platform_capture(
    window: &WebviewWindow,
    max_pixels: u64,
    max_bytes: usize,
) -> Result<Capture, CaptureError> {
    use std::sync::{Arc, Mutex};
    use webview2_com::{CapturePreviewCompletedHandler, Microsoft::Web::WebView2::Win32::*};
    use windows::{
        core::{Interface, BOOL},
        Win32::{
            Foundation::{HGLOBAL, RECT},
            System::Com::{
                StructuredStorage::CreateStreamOnHGlobal, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET,
            },
        },
    };

    if !window
        .is_visible()
        .map_err(|_| CaptureError::WindowUnavailable)?
        || window
            .is_minimized()
            .map_err(|_| CaptureError::WindowUnavailable)?
    {
        return Err(CaptureError::WindowHidden);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let deliver = sender.clone();
    let target = window.clone();
    window
        .with_webview(move |platform| {
            let result = (|| {
                let controller = platform.controller();
                let mut visible = BOOL::default();
                unsafe { controller.IsVisible(&mut visible) }
                    .map_err(|_| CaptureError::WindowUnavailable)?;
                if !visible.as_bool()
                    || !target
                        .is_visible()
                        .map_err(|_| CaptureError::WindowUnavailable)?
                    || target
                        .is_minimized()
                        .map_err(|_| CaptureError::WindowUnavailable)?
                {
                    return Err(CaptureError::WindowHidden);
                }
                let mut bounds = RECT::default();
                unsafe { controller.Bounds(&mut bounds) }
                    .map_err(|_| CaptureError::WindowUnavailable)?;
                let width = u32::try_from(bounds.right - bounds.left)
                    .map_err(|_| CaptureError::WindowUnavailable)?;
                let height = u32::try_from(bounds.bottom - bounds.top)
                    .map_err(|_| CaptureError::WindowUnavailable)?;
                check_size(width, height, max_pixels)?;
                let controller3: ICoreWebView2Controller3 =
                    controller.cast().map_err(|_| CaptureError::CaptureFailed)?;
                let mut bounds_mode = COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS;
                unsafe { controller3.BoundsMode(&mut bounds_mode) }
                    .map_err(|_| CaptureError::CaptureFailed)?;
                if bounds_mode != COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS {
                    return Err(CaptureError::CaptureFailed);
                }
                let mut scale = 0.0;
                unsafe { controller3.RasterizationScale(&mut scale) }
                    .map_err(|_| CaptureError::CaptureFailed)?;
                if !scale.is_finite() || scale <= 0.0 {
                    return Err(CaptureError::CaptureFailed);
                }
                // Wry 0.55.1 sets controller Bounds in raw physical pixels.
                let logical_width = (f64::from(width) / scale).round() as u32;
                let logical_height = (f64::from(height) / scale).round() as u32;
                if logical_width == 0 || logical_height == 0 {
                    return Err(CaptureError::WindowUnavailable);
                }
                let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL(std::ptr::null_mut()), true) }
                    .map_err(|_| CaptureError::CaptureFailed)?;
                let callback_stream = stream.clone();
                let callback_sender = deliver.clone();
                let webview = unsafe { controller.CoreWebView2() }
                    .map_err(|_| CaptureError::CaptureFailed)?;
                let callback = CapturePreviewCompletedHandler::create(Box::new(move |status| {
                    if callback_sender
                        .lock()
                        .unwrap()
                        .as_ref()
                        .is_none_or(|tx| tx.is_closed())
                    {
                        return Ok(());
                    }
                    let captured = (|| {
                        status.map_err(|_| CaptureError::CaptureFailed)?;
                        let mut stat = STATSTG::default();
                        unsafe { callback_stream.Stat(&mut stat, STATFLAG_NONAME) }
                            .map_err(|_| CaptureError::CaptureFailed)?;
                        if stat.cbSize > max_bytes as u64 {
                            return Err(CaptureError::ImageTooLarge);
                        }
                        unsafe { callback_stream.Seek(0, STREAM_SEEK_SET, None) }
                            .map_err(|_| CaptureError::CaptureFailed)?;
                        let mut png = vec![0; stat.cbSize as usize];
                        let mut read = 0;
                        unsafe {
                            callback_stream.Read(
                                png.as_mut_ptr().cast(),
                                png.len() as u32,
                                Some(&mut read),
                            )
                        }
                        .ok()
                        .map_err(|_| CaptureError::CaptureFailed)?;
                        if read as usize != png.len() {
                            return Err(CaptureError::CaptureFailed);
                        }
                        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
                        let reader = decoder
                            .read_info()
                            .map_err(|_| CaptureError::CaptureFailed)?;
                        let info = reader.info();
                        check_size(info.width, info.height, max_pixels)?;
                        if info.width != width || info.height != height {
                            return Err(CaptureError::CaptureFailed);
                        }
                        Ok(Capture {
                            png,
                            width,
                            height,
                            logical_size: [logical_width, logical_height],
                            scale_factor: scale,
                            display_backend: "WebView2".into(),
                        })
                    })();
                    if let Some(tx) = callback_sender.lock().unwrap().take() {
                        let _ = tx.send(captured);
                    }
                    Ok(())
                }));
                unsafe {
                    webview.CapturePreview(
                        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                        &stream,
                        &callback,
                    )
                }
                .map_err(|_| CaptureError::CaptureFailed)?;
                Ok(())
            })();
            if let Err(error) = result {
                if let Some(tx) = deliver.lock().unwrap().take() {
                    let _ = tx.send(Err(error));
                }
            }
        })
        .map_err(|_| CaptureError::WindowUnavailable)?;
    let image = rx.await.map_err(|_| CaptureError::WindowUnavailable)??;
    if !window
        .is_visible()
        .map_err(|_| CaptureError::WindowUnavailable)?
        || window
            .is_minimized()
            .map_err(|_| CaptureError::WindowUnavailable)?
    {
        return Err(CaptureError::WindowHidden);
    }
    Ok(image)
}

#[cfg(target_os = "linux")]
async fn platform_capture(
    window: &WebviewWindow,
    max_pixels: u64,
    max_bytes: usize,
) -> Result<Capture, CaptureError> {
    use gtk::prelude::{ObjectExt, WidgetExt};
    use webkit2gtk::{
        gio, gio::prelude::CancellableExt, SnapshotOptions, SnapshotRegion, WebViewExt,
    };

    struct CancelOnDrop(gio::Cancellable);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            self.0.cancel();
        }
    }
    let cancel = CancelOnDrop(gio::Cancellable::new());
    let native_cancel = cancel.0.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let target = window.clone();
    window
        .with_webview(move |platform| {
            let view = platform.inner();
            let display_backend = WidgetExt::display(&view).type_().name().to_string();
            let logical_width = view.allocated_width().max(0) as u32;
            let logical_height = view.allocated_height().max(0) as u32;
            let scale = WidgetExt::scale_factor(&view).max(1) as u32;
            // Run these checks on the UI thread immediately before allocating
            // the native snapshot, including a physical-pixel budget precheck.
            let check = || {
                if !target
                    .is_visible()
                    .map_err(|_| CaptureError::WindowUnavailable)?
                {
                    return Err(CaptureError::WindowHidden);
                }
                if target
                    .is_minimized()
                    .map_err(|_| CaptureError::WindowUnavailable)?
                {
                    return Err(CaptureError::WindowHidden);
                }
                let width = logical_width
                    .checked_mul(scale)
                    .ok_or(CaptureError::ImageTooLarge)?;
                let height = logical_height
                    .checked_mul(scale)
                    .ok_or(CaptureError::ImageTooLarge)?;
                check_size(width, height, max_pixels)
            };
            if tx.is_closed() || native_cancel.is_cancelled() {
                return;
            }
            if let Err(error) = check() {
                let _ = tx.send(Err(error));
                return;
            }
            view.snapshot(
                SnapshotRegion::Visible,
                SnapshotOptions::NONE,
                Some(&native_cancel),
                move |result| {
                    if tx.is_closed() {
                        return; // Timeout/drop: never encode or publish a late frame.
                    }
                    let raw = (|| {
                        let surface = result.map_err(|_| CaptureError::CaptureFailed)?;
                        let image = cairo::ImageSurface::try_from(surface)
                            .map_err(|_| CaptureError::CaptureFailed)?;
                        let width = image.width() as u32;
                        let height = image.height() as u32;
                        check_size(width, height, max_pixels)?;
                        if width != logical_width * scale || height != logical_height * scale {
                            return Err(CaptureError::CaptureFailed);
                        }
                        let mut data = Vec::new();
                        image
                            .with_data(|slice| data.extend_from_slice(slice))
                            .map_err(|_| CaptureError::CaptureFailed)?;
                        Ok((
                            data,
                            image.format(),
                            width,
                            height,
                            image.stride(),
                            [logical_width, logical_height],
                            scale,
                            display_backend,
                        ))
                    })();
                    let _ = tx.send(raw);
                },
            );
        })
        .map_err(|_| CaptureError::WindowUnavailable)?;
    let (data, format, width, height, stride, logical_size, scale_factor, display_backend) =
        rx.await.map_err(|_| CaptureError::WindowUnavailable)??;
    // Transfer bytes, not a GTK/Cairo handle, to the encoding worker.
    let encoding_cancel = cancel.0.clone();
    let image = tokio::task::spawn_blocking(move || {
        if encoding_cancel.is_cancelled() {
            return Err(CaptureError::RenderTimeout);
        }
        let surface =
            cairo::ImageSurface::create_for_data(data, format, width as i32, height as i32, stride)
                .map_err(|_| CaptureError::CaptureFailed)?;
        let mut writer = LimitedWriter {
            bytes: Vec::new(),
            limit: max_bytes,
            exceeded: false,
            cancel: encoding_cancel,
        };
        let result = surface.write_to_png(&mut writer);
        if writer.cancel.is_cancelled() {
            return Err(CaptureError::RenderTimeout);
        }
        if writer.exceeded {
            return Err(CaptureError::ImageTooLarge);
        }
        result.map_err(|_| CaptureError::CaptureFailed)?;
        Ok(Capture {
            png: writer.bytes,
            width,
            height,
            logical_size,
            scale_factor: f64::from(scale_factor),
            display_backend,
        })
    })
    .await
    .map_err(|_| CaptureError::CaptureFailed)??;
    // A window may close/hide while the native callback or encoder is pending.
    if !window
        .is_visible()
        .map_err(|_| CaptureError::WindowUnavailable)?
    {
        return Err(CaptureError::WindowHidden);
    }
    if window
        .is_minimized()
        .map_err(|_| CaptureError::WindowUnavailable)?
    {
        return Err(CaptureError::WindowHidden);
    }
    Ok(image)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn check_size(width: u32, height: u32, limit: u64) -> Result<(), CaptureError> {
    if width == 0 || height == 0 {
        return Err(CaptureError::WindowUnavailable);
    }
    if u64::from(width) * u64::from(height) > limit {
        return Err(CaptureError::ImageTooLarge);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
    cancel: webkit2gtk::gio::Cancellable,
}

#[cfg(target_os = "linux")]
impl std::io::Write for LimitedWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        use webkit2gtk::gio::prelude::CancellableExt;
        if self.cancel.is_cancelled() {
            return Err(std::io::Error::other("capture cancelled"));
        }
        if data.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::Error::other("PNG byte limit exceeded"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
