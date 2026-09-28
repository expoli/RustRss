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
fn probe(controller: ICoreWebView2Controller) {
    let mut visible = BOOL::default();
    unsafe { controller.IsVisible(&mut visible).unwrap() };
    let mut bounds = RECT::default();
    unsafe { controller.Bounds(&mut bounds).unwrap() };
    let controller3: ICoreWebView2Controller3 = controller.cast().unwrap();
    let mut mode = COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS;
    unsafe { controller3.BoundsMode(&mut mode).unwrap() };
    let mut scale = 0.0;
    unsafe { controller3.RasterizationScale(&mut scale).unwrap() };
    let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL(std::ptr::null_mut()), true).unwrap() };
    let clone = stream.clone();
    let webview = unsafe { controller.CoreWebView2().unwrap() };
    let callback = CapturePreviewCompletedHandler::create(Box::new(move |status| {
        status?;
        let mut stat = STATSTG::default();
        unsafe { clone.Stat(&mut stat, STATFLAG_NONAME).unwrap() };
        unsafe { clone.Seek(0, STREAM_SEEK_SET, None).unwrap() };
        let mut bytes = vec![0u8; stat.cbSize as usize];
        let mut read = 0;
        unsafe {
            clone.Read(
                bytes.as_mut_ptr().cast(),
                bytes.len() as u32,
                Some(&mut read),
            )
        }
        .ok()
        .unwrap();
        Ok(())
    }));
    unsafe {
        webview
            .CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &callback,
            )
            .unwrap()
    };
}
fn main() {
    let _ = probe as fn(ICoreWebView2Controller);
}
