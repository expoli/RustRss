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
    WindowUnavailable,
    WindowHidden,
    ImageTooLarge,
    CaptureFailed,
    RenderTimeout,
}

pub struct Capture {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub logical_size: [u32; 2],
    pub scale_factor: u32,
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

#[cfg(not(target_os = "linux"))]
async fn platform_capture(_: &WebviewWindow, _: u64, _: usize) -> Result<Capture, CaptureError> {
    Err(CaptureError::Unavailable)
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
            scale_factor,
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

#[cfg(target_os = "linux")]
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
