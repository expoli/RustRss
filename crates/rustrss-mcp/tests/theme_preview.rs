use rustrss_core::Store;
use rustrss_mcp::{
    http::{serve, HttpConfig, HttpHandle},
    preview::{Backend, Frame, RenderFuture, RenderRequest},
    RustRssMcp,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
struct Mock {
    delay: Duration,
    closed: AtomicU64,
    fail: bool,
}
impl Backend for Mock {
    fn close(&self) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
    fn render(&self, _: RenderRequest) -> RenderFuture {
        let delay = self.delay;
        let fail = self.fail;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            if fail {
                Err("capture_failed")
            } else {
                Ok(Frame {
                    png: vec![1; 32],
                    metadata: json!({"mock":true}),
                })
            }
        })
    }
}
struct Fixture {
    path: std::path::PathBuf,
    store: Store,
    handle: HttpHandle,
    backend: Arc<Mock>,
    cleanup_files: Box<dyn Fn() + Send + Sync>,
}
impl Fixture {
    async fn new(delay: Duration, fail: bool) -> Self {
        let path =
            std::env::temp_dir().join(format!("theme-preview-{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        store.set_setting("mcp.token", "reader").unwrap();
        store.set_setting("mcp.write_token", "writer").unwrap();
        store.set_bool_setting("mcp.write_enabled", true).unwrap();
        let backend = Arc::new(Mock {
            delay,
            closed: AtomicU64::new(0),
            fail,
        });
        let server = RustRssMcp::open(&path)
            .unwrap()
            .with_preview_backend(backend.clone());
        let cleanup_files = Box::new(server.preview_file_cleanup());
        let handle = serve(
            server,
            HttpConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                token: None,
            },
        )
        .await
        .unwrap();
        Self {
            path,
            store,
            handle,
            backend,
            cleanup_files,
        }
    }
    async fn call(&self, token: &str, name: &str, args: Value) -> Value {
        call(&self.handle.url(), token, name, args).await
    }
    fn cleanup(self) {
        (self.cleanup_files)();
        self.handle.shutdown();
        drop(self.store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}
async fn call(url: &str, token: &str, name: &str, args: Value) -> Value {
    reqwest::Client::new().post(url).bearer_auth(token).header("accept","application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).send().await.unwrap().json::<Value>().await.unwrap()["result"].clone()
}
fn meta(result: &Value) -> &Value {
    &result["structuredContent"]
}
fn assert_file(result: &Value) {
    assert!(result["content"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["type"] == "text"));
    let p = meta(result);
    assert_eq!(p["image_mime_type"], "image/png");
    assert_eq!(
        std::fs::read(p["image_path"].as_str().unwrap()).unwrap(),
        vec![1; 32]
    );
}
fn initial() -> Value {
    json!({"base_revision":0,"patch":{"light_preset":"paper"}})
}
fn finish(p: &Value, action: &str) -> Value {
    json!({"preview_id":p["preview_id"],"expected_preview_revision":p["preview_revision"],"action":action})
}
#[tokio::test]
async fn temporary_image_patch_cancel_and_permissions() {
    let f = Fixture::new(Duration::ZERO, false).await;
    let before = f.store.all_settings().unwrap();
    let denied = f.call("reader", "preview_theme", initial()).await;
    assert_eq!(denied["isError"], true);
    let result = f.call("writer", "preview_theme", initial()).await;
    let p = meta(&result);
    assert_eq!(p["ok"], true);
    assert_file(&result);
    assert_eq!(f.store.all_settings().unwrap(), before);
    let recapture=f.call("writer","capture_theme_preview",json!({"preview_id":p["preview_id"],"expected_preview_revision":1,"scene":"settings","mode":"dark"})).await;
    assert_eq!(meta(&recapture)["preview_revision"], 1);
    assert_eq!(meta(&recapture)["config_hash"], p["config_hash"]);
    assert_file(&recapture);
    let busy = f.call("writer", "preview_theme", initial()).await;
    assert_eq!(meta(&busy)["error_code"], "preview_busy");
    let stale = f
        .call(
            "writer",
            "capture_theme_preview",
            json!({"preview_id":p["preview_id"],"expected_preview_revision":0}),
        )
        .await;
    assert_eq!(meta(&stale)["error_code"], "revision_conflict");
    let patched=f.call("writer","preview_theme",json!({"base_revision":0,"preview_id":p["preview_id"],"expected_preview_revision":1,"patch":{"overrides":{"typography":{"read_size":24}}}})).await;
    assert_eq!(meta(&patched)["preview_revision"], 2);
    let cancel = f
        .call(
            "writer",
            "finish_theme_preview",
            finish(meta(&patched), "cancel"),
        )
        .await;
    assert_eq!(meta(&cancel)["action"], "cancel");
    assert_eq!(f.store.all_settings().unwrap(), before);
    assert!(f.backend.closed.load(Ordering::SeqCst) > 0);
    f.cleanup();
}
#[tokio::test]
async fn save_cas_conflict_and_idempotent_finish() {
    let f = Fixture::new(Duration::ZERO, false).await;
    let result = f.call("writer", "preview_theme", initial()).await;
    let p = meta(&result);
    let saved = f
        .call("writer", "finish_theme_preview", finish(p, "save"))
        .await;
    assert_eq!(meta(&saved)["saved_revision"], 1);
    assert!(std::path::Path::new(p["image_path"].as_str().unwrap()).exists());
    let retry = f
        .call("writer", "finish_theme_preview", finish(p, "save"))
        .await;
    assert_eq!(retry, saved);
    assert_eq!(f.store.theme_snapshot().unwrap().config.revision, 1);
    let result = f
        .call(
            "writer",
            "preview_theme",
            json!({"base_revision":1,"patch":{"mode":"dark"}}),
        )
        .await;
    let p = meta(&result);
    f.store
        .update_theme(
            1,
            &rustrss_core::theme::ThemePatch::from_json(r#"{"mode":"light"}"#).unwrap(),
        )
        .unwrap();
    let conflict = f
        .call("writer", "finish_theme_preview", finish(p, "save"))
        .await;
    assert_eq!(meta(&conflict)["error_code"], "revision_conflict");
    let cancel = f
        .call("writer", "finish_theme_preview", finish(p, "cancel"))
        .await;
    assert_eq!(meta(&cancel)["ok"], true);
    f.cleanup();
}
#[tokio::test]
async fn concurrent_capture_and_rotation_during_render_fail_closed() {
    let f = Fixture::new(Duration::from_millis(250), false).await;
    let first = f.call("writer", "preview_theme", initial());
    let other = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let busy = f.call("writer", "preview_theme", initial()).await;
        assert_eq!(meta(&busy)["error_code"], "preview_busy");
        f.store
            .set_setting("mcp.write_token", "new-writer")
            .unwrap();
    };
    let (result, ()) = tokio::join!(first, other);
    assert_eq!(meta(&result)["error_code"], "preview_permission_revoked");
    assert!(result["content"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["type"] != "image"));
    assert!(meta(&result)["image_path"].is_null());
    assert_eq!(f.store.theme_snapshot().unwrap().config.revision, 0);
    assert!(f.backend.closed.load(Ordering::SeqCst) > 0);
    f.cleanup();
}
#[tokio::test]
async fn failed_render_retains_candidate_id_for_cancel() {
    let f = Fixture::new(Duration::ZERO, true).await;
    let result = f.call("writer", "preview_theme", initial()).await;
    assert_eq!(meta(&result)["error_code"], "capture_failed");
    assert!(meta(&result)["preview_id"].is_string());
    let cancel = f
        .call(
            "writer",
            "finish_theme_preview",
            finish(meta(&result), "cancel"),
        )
        .await;
    assert_eq!(meta(&cancel)["ok"], true);
    f.cleanup();
}
#[tokio::test]
async fn bridge_checks_profile_and_forwards_file_and_cancel() {
    let f = Fixture::new(Duration::ZERO, false).await;
    f.store.set_bool_setting("mcp.enabled", true).unwrap();
    f.store
        .set_setting("mcp.port", &f.handle.addr.port().to_string())
        .unwrap();
    let bridge = serve(
        RustRssMcp::open(&f.path).unwrap(),
        HttpConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            token: None,
        },
    )
    .await
    .unwrap();
    let caps = call(&bridge.url(), "reader", "get_theme", json!({})).await;
    assert_eq!(meta(&caps)["capabilities"]["preview"]["available"], true);
    assert_eq!(
        meta(&caps)["capabilities"]["preview"]["image_delivery"],
        "local_file"
    );
    assert_eq!(
        meta(&caps)["capabilities"]["preview"]["requires_shared_filesystem"],
        true
    );
    let result = call(&bridge.url(), "writer", "preview_theme", initial()).await;
    assert_file(&result);
    let recapture = call(
        &bridge.url(),
        "writer",
        "capture_theme_preview",
        json!({"preview_id":meta(&result)["preview_id"], "expected_preview_revision":1}),
    )
    .await;
    assert_file(&recapture);
    let cancel = call(
        &bridge.url(),
        "writer",
        "finish_theme_preview",
        finish(meta(&result), "cancel"),
    )
    .await;
    assert_eq!(meta(&cancel)["ok"], true);
    let mismatch = f
        .call(
            "writer",
            "preview_theme",
            json!({"base_revision":0,"patch":{},"profile_id":"other-profile"}),
        )
        .await;
    assert_eq!(meta(&mismatch)["error_code"], "profile_mismatch");
    let other = Fixture::new(Duration::ZERO, false).await;
    f.store
        .set_setting("mcp.port", &other.handle.addr.port().to_string())
        .unwrap();
    let mismatch = call(&bridge.url(), "writer", "preview_theme", initial()).await;
    assert_eq!(meta(&mismatch)["error_code"], "profile_mismatch");
    other.cleanup();
    bridge.shutdown();
    f.cleanup();
}

#[tokio::test]
async fn screenshots_are_local_files_with_explicit_lifetime_and_no_inline_images() {
    let f = Fixture::new(Duration::ZERO, false).await;
    let caps = f.call("reader", "get_theme", json!({})).await;
    assert_eq!(
        meta(&caps)["capabilities"]["preview"]["image_delivery"],
        "local_file"
    );
    assert_eq!(
        meta(&caps)["capabilities"]["preview"]["requires_shared_filesystem"],
        true
    );
    let result = f.call("writer", "preview_theme", initial()).await;
    let p = meta(&result);
    assert_eq!(p["ok"], true);
    assert!(result["content"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["type"] == "text"));
    let path = std::path::Path::new(p["image_path"].as_str().unwrap());
    assert!(path.is_absolute());
    assert_eq!(std::fs::read(path).unwrap(), vec![1; 32]);
    assert_eq!(p["image_mime_type"], "image/png");
    assert_eq!(p["image_bytes"], 32);
    assert!(p["image_expires_at_ms"].as_u64().unwrap() > 0);
    assert!(p["image_read_instruction"]
        .as_str()
        .unwrap()
        .contains("view_image"));
    let recapture = f
        .call(
            "writer",
            "capture_theme_preview",
            json!({"preview_id":p["preview_id"],"expected_preview_revision":1}),
        )
        .await;
    assert_ne!(meta(&recapture)["image_path"], p["image_path"]);
    assert_eq!(std::fs::read(path).unwrap(), vec![1; 32]);
    f.call("writer", "finish_theme_preview", finish(p, "cancel"))
        .await;
    assert!(
        path.exists(),
        "cancel must retain screenshots for their declared lifetime"
    );
    f.store.set_setting("mcp.write_token", "rotated").unwrap();
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(
        !path.exists(),
        "revocation must reap already finished screenshots"
    );
    f.cleanup();
}

#[tokio::test]
async fn file_limit_and_shutdown_preserve_candidate_for_cancel() {
    let f = Fixture::new(Duration::ZERO, false).await;
    let result = f.call("writer", "preview_theme", initial()).await;
    let p = meta(&result);
    let args = json!({"preview_id":p["preview_id"],"expected_preview_revision":1});
    for _ in 1..rustrss_core::theme_preview_files::MAX_FILES {
        assert_file(
            &f.call("writer", "capture_theme_preview", args.clone())
                .await,
        );
    }
    let full = f
        .call("writer", "capture_theme_preview", args.clone())
        .await;
    assert_eq!(meta(&full)["error_code"], "preview_file_limit");
    assert!(meta(&full)["error_message"]
        .as_str()
        .unwrap()
        .contains("wait"));
    assert_eq!(meta(&full)["preview_id"], p["preview_id"]);
    assert!(meta(&full)["image_path"].is_null());
    let path = std::path::Path::new(p["image_path"].as_str().unwrap());
    let directory = path.parent().unwrap().to_path_buf();
    (f.cleanup_files)();
    assert!(
        !directory.exists(),
        "explicit desktop shutdown removes all artifacts"
    );
    let closed = f.call("writer", "capture_theme_preview", args).await;
    assert_eq!(meta(&closed)["error_code"], "preview_file_unavailable");
    assert!(meta(&closed)["image_path"].is_null());
    assert!(
        !directory.exists(),
        "shutdown must prevent late file publication"
    );
    assert_eq!(
        meta(
            &f.call("writer", "finish_theme_preview", finish(p, "cancel"))
                .await
        )["ok"],
        true
    );
    f.cleanup();
}
