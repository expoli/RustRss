use rustrss_core::{theme::ThemeConfig, Store};
use rustrss_mcp::{
    http::{serve, HttpConfig, HttpHandle},
    RustRssMcp,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

struct Fixture {
    path: std::path::PathBuf,
    store: Store,
    handle: HttpHandle,
}
impl Fixture {
    async fn new(notifications: Option<Arc<AtomicU64>>) -> Self {
        let path =
            std::env::temp_dir().join(format!("rustrss-theme-mcp-{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        store.set_setting("mcp.token", "reader").unwrap();
        store.set_setting("mcp.write_token", "writer").unwrap();
        store.set_bool_setting("mcp.write_enabled", true).unwrap();
        let mut server = RustRssMcp::open(&path).unwrap();
        if let Some(counter) = notifications {
            server = server.with_theme_notifications(move |revision| {
                counter.store(revision, Ordering::SeqCst);
                true
            });
        }
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
        }
    }
    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        let response = reqwest::Client::new()
            .post(self.handle.url())
            .bearer_auth(token)
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }
    async fn call(&self, token: &str, name: &str, args: Value) -> Value {
        self.rpc(token, "tools/call", json!({"name":name,"arguments":args}))
            .await
    }
    fn cleanup(self) {
        self.handle.shutdown();
        drop(self.store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}
fn body(v: &Value) -> Value {
    serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn read_discovery_schema_and_validation_are_non_mutating() {
    let f = Fixture::new(None).await;
    let before = f.store.all_settings().unwrap();
    let tools = f.rpc("reader", "tools/list", json!({})).await;
    let names = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    for name in ["get_theme", "list_theme_presets", "validate_theme"] {
        assert!(names.contains(&name));
    }
    assert!(!names.contains(&"update_theme"));
    let get = f
        .call("reader", "get_theme", json!({"include_schema":true}))
        .await;
    let g = body(&get);
    assert_eq!(get["result"]["structuredContent"], g);
    assert_eq!(g["theme"]["config"]["revision"], 0);
    assert_eq!(g["capabilities"]["preview"]["available"], false);
    assert_eq!(g["patch_schema"]["additionalProperties"], false);
    let presets = body(&f.call("reader", "list_theme_presets", json!({})).await);
    assert_eq!(presets["count"], 3);
    let validated=body(&f.call("reader","validate_theme",json!({"expected_revision":0,"patch":{"light_preset":"paper","overrides":{"typography":{"read_size":24}}}})).await);
    assert_eq!(validated["persisted"], false);
    assert_eq!(validated["theme"]["light"]["typography"]["read_size"], 24.);
    assert_eq!(f.store.all_settings().unwrap(), before);
    f.cleanup();
}

#[tokio::test]
async fn write_cas_noop_history_restore_and_notification() {
    let count = Arc::new(AtomicU64::new(0));
    let f = Fixture::new(Some(count.clone())).await;
    let params = json!({"expected_revision":0,"patch":{"light_preset":"paper"}});
    let denied = f.call("reader", "update_theme", params.clone()).await;
    assert_eq!(body(&denied)["error_code"], "write_scope_required");
    let saved = body(&f.call("writer", "update_theme", params.clone()).await);
    assert_eq!(saved["detail"]["saved_revision"], 1);
    assert_eq!(saved["detail"]["live_apply"], "pending");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let stale = f.call("writer", "update_theme", params).await;
    assert_eq!(stale["result"]["isError"], true);
    assert_eq!(body(&stale)["actual_revision"], 1);
    assert_eq!(
        stale["result"]["structuredContent"]["error_code"],
        "revision_conflict"
    );
    let raw = f.store.setting("ui.theme_config").unwrap();
    count.store(99, Ordering::SeqCst);
    let noop = body(
        &f.call(
            "writer",
            "update_theme",
            json!({"expected_revision":1,"patch":{"light_preset":"paper"}}),
        )
        .await,
    );
    assert_eq!(noop["affected"], 0);
    assert_eq!(noop["detail"]["live_apply"], "unchanged");
    assert_eq!(count.load(Ordering::SeqCst), 99);
    assert_eq!(raw, f.store.setting("ui.theme_config").unwrap());
    let restore = body(
        &f.call(
            "writer",
            "restore_theme",
            json!({"expected_revision":1,"historical_revision":0}),
        )
        .await,
    );
    assert_eq!(restore["detail"]["saved_revision"], 2);
    assert_eq!(
        f.store.theme_snapshot().unwrap().config.light_preset,
        ThemeConfig::default().light_preset
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    f.cleanup();
}

#[tokio::test]
async fn invalid_patch_and_corrupt_store_fail_without_writes() {
    let f = Fixture::new(None).await;
    let before = f.store.all_settings().unwrap();
    for patch in [
        json!({"bogus":null}),
        json!({"overrides":{"colors":{"light":{"accent":"url(secret)"}}}}),
        json!({"overrides":{"typography":{"read_size":100}}}),
        json!({"overrides":{"reader":{"typo":null}}}),
    ] {
        let r = f
            .call(
                "writer",
                "update_theme",
                json!({"expected_revision":0,"patch":patch}),
            )
            .await;
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(body(&r)["error_code"], "invalid_argument");
    }
    assert_eq!(before, f.store.all_settings().unwrap());
    f.store.set_setting("ui.theme_config", "broken").unwrap();
    let r = f.call("reader", "get_theme", json!({})).await;
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(body(&r)["error_code"], "theme_storage_corrupt");
    assert_eq!(
        f.store.setting("ui.theme_config").unwrap().as_deref(),
        Some("broken")
    );
    f.cleanup();
}

#[tokio::test]
async fn disabled_writes_and_rotated_tokens_take_effect_on_next_request() {
    let f = Fixture::new(None).await;
    let p = json!({"expected_revision":0,"patch":{"mode":"dark"}});
    f.store
        .set_bool_setting("mcp.write_enabled", false)
        .unwrap();
    assert_eq!(
        body(&f.call("writer", "update_theme", p.clone()).await)["error_code"],
        "write_disabled"
    );
    f.store.set_bool_setting("mcp.write_enabled", true).unwrap();
    f.store
        .set_setting("mcp.write_token", "new-writer")
        .unwrap();
    let response=reqwest::Client::new().post(f.handle.url()).bearer_auth("writer").header("accept","application/json, text/event-stream").json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_theme","arguments":p}})).send().await.unwrap();
    assert_eq!(response.status(), 401);
    let saved = body(
        &f.call(
            "new-writer",
            "update_theme",
            json!({"expected_revision":0,"patch":{"mode":"dark"}}),
        )
        .await,
    );
    assert_eq!(saved["detail"]["live_apply"], "unavailable");
    assert_eq!(saved["detail"]["saved_revision"], 1);
    f.cleanup();
}

#[test]
fn audit_does_not_log_theme_values_or_unknown_input() {
    let args = json!({"expected_revision":5,"patch":{"overrides":{"typography":{"read_family":["PRIVATE-FONT"]}},"unknown":"PRIVATE"},"credential":"SECRET"});
    let line = rustrss_mcp::audit::write_line(
        "update_theme",
        args.as_object(),
        &rustrss_mcp::audit::AuditSummary::ok(1, false),
    );
    assert!(line.contains("expected_revision"));
    assert!(line.contains("overrides"));
    assert!(!line.contains("PRIVATE"));
    assert!(!line.contains("SECRET"));
}

#[tokio::test]
async fn theme_patch_tool_schemas_publish_the_core_object_contract() {
    let f = Fixture::new(None).await;
    let tools = f.rpc("writer", "tools/list", json!({})).await;
    let expected = rustrss_core::theme::patch_schema();
    for name in ["validate_theme", "update_theme", "preview_theme"] {
        let tool = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        let patch = &tool["inputSchema"]["properties"]["patch"];
        assert_eq!(
            patch["type"], "object",
            "{name}: patch must not be an unconstrained schema or JSON string"
        );
        assert_eq!(patch["additionalProperties"], false);
        assert_eq!(
            patch["properties"], expected["properties"],
            "{name}: publish the same sparse/null/bounds contract as core"
        );
        assert_eq!(
            patch["max_serialized_bytes"],
            expected["max_serialized_bytes"]
        );
    }
    f.cleanup();
}

#[tokio::test]
async fn preview_tool_descriptions_require_local_image_reading() {
    let f = Fixture::new(None).await;
    let result = f.rpc("writer", "tools/list", json!({})).await;
    for name in ["preview_theme", "capture_theme_preview"] {
        let tool = result["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        let description = tool["description"].as_str().unwrap();
        for required in ["image_path", "view_image", "filesystem", "inline", "600s"] {
            assert!(description.contains(required), "{name}: missing {required}");
        }
    }
    f.cleanup();
}
