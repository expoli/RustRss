//! Preview orchestration and bounded same-profile desktop bridge.
use crate::{
    http::RequestIdentity,
    theme_tools::{failure, response, GetThemeParams},
    RustRssMcp,
};
use base64::Engine;
use rmcp::{
    model::{CallToolResult, ContentBlock},
    service::{RequestContext, RoleServer},
};
use rustrss_core::{
    theme::{ThemePatch, ThemeSnapshot},
    theme_preview::Preview,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

pub fn fingerprint(value: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(value.as_ref()))
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scene {
    Overview,
    #[default]
    Article,
    Settings,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Light,
    Dark,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewParams {
    pub base_revision: u64,
    pub patch: Value,
    pub preview_id: Option<String>,
    pub expected_preview_revision: Option<u64>,
    #[serde(default)]
    pub scene: Scene,
    #[serde(default)]
    pub mode: Mode,
    /// Optional profile guard; bridge fills this automatically.
    pub profile_id: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureParams {
    pub preview_id: String,
    pub expected_preview_revision: u64,
    #[serde(default)]
    pub scene: Scene,
    #[serde(default)]
    pub mode: Mode,
    pub profile_id: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Save,
    Cancel,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FinishParams {
    pub preview_id: String,
    pub expected_preview_revision: u64,
    pub action: Action,
    pub profile_id: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct RenderRequest {
    pub request_id: String,
    pub preview_revision: u64,
    pub snapshot: ThemeSnapshot,
    pub scene: Scene,
    pub mode: Mode,
}
pub struct Frame {
    pub png: Vec<u8>,
    pub metadata: Value,
}
pub type RenderFuture = Pin<Box<dyn Future<Output = Result<Frame, &'static str>> + Send>>;
pub trait Backend: Send + Sync {
    fn render(&self, request: RenderRequest) -> RenderFuture;
    fn close(&self);
    fn dismissed(&self) -> bool {
        false
    }
}
struct Finished {
    id: String,
    owner: String,
    revision: u64,
    action: Action,
    result: Value,
    at: u64,
}
#[derive(Default)]
struct State {
    active: Option<Preview>,
    finished: std::collections::VecDeque<Finished>,
}
pub struct Service {
    backend: Option<Arc<dyn Backend>>,
    state: tokio::sync::Mutex<State>,
    started: Instant,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            backend: None,
            state: Default::default(),
            started: Instant::now(),
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if let Some(b) = &self.backend {
            b.close();
        }
    }
}
impl Service {
    fn now(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
    pub fn capability(&self) -> Value {
        if self.backend.is_some() {
            json!({"available":true,"backend":"native_webview","fixture_version":1,"idle_seconds":600,"max_seconds":1800,"viewports":[[1280,900],[960,640]],"deadline_seconds":10,"max_pixels":6000000,"max_png_bytes":2097152})
        } else {
            json!({"available":false,"reason":"preview_backend_unavailable"})
        }
    }
    fn sweep(&self, state: &mut State, owner: Option<&str>) {
        if state.active.as_ref().is_some_and(|p| {
            p.expired(self.now())
                || Some(p.owner.as_str()) != owner
                || self.backend.as_ref().is_some_and(|b| b.dismissed())
        }) {
            state.active = None;
            if let Some(b) = &self.backend {
                b.close();
            }
        }
        state
            .finished
            .retain(|f| self.now().saturating_sub(f.at) < 600 && Some(f.owner.as_str()) == owner);
    }
}
fn error(code: &str) -> CallToolResult {
    response(
        json!({"ok":false,"affected":0,"error_code":code,"retryable":matches!(code,"preview_busy"|"render_timeout"|"capture_failed"|"preview_backend_unavailable")}),
    )
}
fn owner_from_store(store: &rustrss_core::Store) -> Option<String> {
    if !store.bool_setting("mcp.write_enabled", false).ok()? {
        return None;
    }
    crate::config::write_token_from_store(store)
        .ok()?
        .map(|t| fingerprint(&t))
}
impl RustRssMcp {
    #[must_use]
    pub fn with_preview_backend(mut self, backend: Arc<dyn Backend>) -> Self {
        self.preview = Arc::new(Service {
            backend: Some(backend),
            state: Default::default(),
            started: Instant::now(),
        });
        let weak = Arc::downgrade(&self.preview);
        let store = Arc::clone(&self.store);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let Some(service) = weak.upgrade() else {
                    break;
                };
                let owner = store.lock().ok().and_then(|s| owner_from_store(&s));
                if let Ok(mut state) = service.state.try_lock() {
                    service.sweep(&mut state, owner.as_deref());
                };
            }
        });
        self
    }
    fn preview_owner(&self) -> Option<String> {
        self.with_store(owner_from_store)
    }
    fn request_owner(&self, context: &RequestContext<RoleServer>) -> Option<String> {
        // HTTP extensions are nested in request Parts by rmcp; local stdio is trusted.
        let identity = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|p| p.extensions.get::<RequestIdentity>())
            .or_else(|| context.extensions.get::<RequestIdentity>());
        if context
            .extensions
            .get::<axum::http::request::Parts>()
            .is_some()
            && identity.is_none()
        {
            return None;
        }
        let current = self.preview_owner()?;
        if identity.is_some_and(|i| i.0 != current) {
            None
        } else {
            Some(current)
        }
    }
    pub async fn theme_capabilities_result(&self, p: &GetThemeParams) -> CallToolResult {
        let result = self.get_theme_result(p);
        if p.local_only || self.preview.backend.is_some() {
            return result;
        }
        let mut body = result.structured_content.clone().unwrap();
        if body["ok"] == true {
            match self
                .bridge("get_theme", json!({"local_only":true}), false)
                .await
            {
                Ok(remote) => {
                    if let Some(value) = remote.structured_content {
                        body["capabilities"]["preview"] = value["capabilities"]["preview"].clone();
                    }
                }
                Err(code) => {
                    body["capabilities"]["preview"] = json!({"available":false,"reason":code})
                }
            }
        }
        response(body)
    }
    async fn bridge(
        &self,
        name: &str,
        mut args: Value,
        write: bool,
    ) -> Result<CallToolResult, &'static str> {
        let (port, token) = self
            .with_store(|s| {
                if !s.bool_setting("mcp.enabled", false).unwrap_or(false) {
                    return None;
                }
                let port = s.setting("mcp.port").ok().flatten()?.parse::<u16>().ok()?;
                let token = s
                    .setting(if write {
                        "mcp.write_token"
                    } else {
                        "mcp.token"
                    })
                    .ok()
                    .flatten()?;
                Some((port, token))
            })
            .ok_or("preview_backend_unavailable")?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|_| "preview_backend_unavailable")?;
        // Discovery cannot recursively bridge. Verify profile before forwarding writes.
        async fn rpc(
            client: &reqwest::Client,
            port: u16,
            token: &str,
            name: &str,
            args: Value,
        ) -> Result<CallToolResult, &'static str> {
            let mut response=client.post(format!("http://127.0.0.1:{port}/mcp")).bearer_auth(token).header("accept","application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).send().await.map_err(|_|"preview_backend_unavailable")?;
            if !response.status().is_success() {
                return Err("preview_backend_unavailable");
            }
            const LIMIT: usize = 3 * 1024 * 1024;
            if response.content_length().is_some_and(|n| n > LIMIT as u64) {
                return Err("image_too_large");
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| "capture_failed")? {
                if chunk.len() > LIMIT.saturating_sub(bytes.len()) {
                    return Err("image_too_large");
                }
                bytes.extend_from_slice(&chunk);
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| "capture_failed")?;
            serde_json::from_value(value["result"].clone()).map_err(|_| "capture_failed")
        }
        let discover = rpc(
            &client,
            port,
            &token,
            "get_theme",
            json!({"local_only":true}),
        )
        .await?;
        let body = discover
            .structured_content
            .as_ref()
            .ok_or("preview_backend_unavailable")?;
        if body["capabilities"]["profile_id"] != self.profile_id {
            return Err("profile_mismatch");
        }
        if name == "get_theme" {
            return Ok(discover);
        }
        if body["capabilities"]["preview"]["available"] != true {
            return Err("preview_backend_unavailable");
        }
        args["profile_id"] = json!(self.profile_id);
        rpc(&client, port, &token, name, args).await
    }
    pub async fn preview_call(
        &self,
        name: &str,
        args: Value,
        context: &RequestContext<RoleServer>,
    ) -> CallToolResult {
        let Some(owner) = self.request_owner(context) else {
            return error("preview_permission_revoked");
        };
        if args
            .get("profile_id")
            .and_then(Value::as_str)
            .is_some_and(|id| id != self.profile_id)
        {
            return error("profile_mismatch");
        }
        if self.preview.backend.is_none() {
            let result = match tokio::time::timeout(Duration::from_secs(10), self.bridge(name, args, true)).await {
                Ok(result)=>result.unwrap_or_else(error),
                Err(_)=>error("render_timeout"),
            };
            return if self.preview_owner().as_deref() == Some(&owner) {
                result
            } else {
                error("preview_permission_revoked")
            };
        }
        self.preview_local(name, args, &owner).await
    }
    async fn preview_local(&self, name: &str, args: Value, owner: &str) -> CallToolResult {
        let service = &self.preview;
        let Ok(mut state) = service.state.try_lock() else {
            return error("preview_busy");
        };
        service.sweep(&mut state, self.preview_owner().as_deref());
        let backend = service.backend.as_ref().unwrap();
        if name == "finish_theme_preview" {
            let p: FinishParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(_) => return error("invalid_argument"),
            };
            for f in &state.finished {
                if f.id == p.preview_id
                    && f.owner == owner
                    && f.revision == p.expected_preview_revision
                    && f.action == p.action
                {
                    return response(f.result.clone());
                }
            }
            let Some(active) = &state.active else {
                return error("preview_expired");
            };
            if let Err(e) = active.check(
                &p.preview_id,
                owner,
                p.expected_preview_revision,
                service.now(),
            ) {
                return error(e);
            }
            if self.preview_owner().as_deref() != Some(owner) {
                return error("preview_permission_revoked");
            }
            let result = if p.action == Action::Save {
                let saved = match self
                    .with_store(|s| s.commit_theme_preview(active.base_revision, &active.candidate))
                {
                    Ok(v) => v,
                    Err(e) => return failure(e),
                };
                let live = if self
                    .theme_changed
                    .as_ref()
                    .is_some_and(|f| f(saved.config.revision))
                {
                    "pending"
                } else {
                    "unavailable"
                };
                json!({"ok":true,"affected":1,"action":"save","saved_revision":saved.config.revision,"config_hash":saved.config_hash,"live_apply":live})
            } else {
                json!({"ok":true,"affected":0,"action":"cancel"})
            };
            if state.finished.len() == 16 {
                state.finished.pop_front();
            }
            state.finished.push_back(Finished {
                id: p.preview_id,
                owner: owner.into(),
                revision: p.expected_preview_revision,
                action: p.action,
                result: result.clone(),
                at: service.now(),
            });
            state.active = None;
            backend.close();
            return response(result);
        }
        let (scene, mode) = if name == "preview_theme" {
            let p: PreviewParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(_) => return error("invalid_argument"),
            };
            let patch = match ThemePatch::from_json(&p.patch.to_string()) {
                Ok(p) => p,
                Err(e) => return failure(e),
            };
            if let Some(id) = p.preview_id {
                let Some(active) = state.active.as_mut() else {
                    return error("preview_expired");
                };
                let Some(revision) = p.expected_preview_revision else {
                    return error("invalid_argument");
                };
                if let Err(e) = active.check(&id, owner, revision, service.now()) {
                    return error(e);
                }
                if active.base_revision != p.base_revision {
                    return error("revision_conflict");
                }
                if let Err(e) = active.patch(&patch, service.now()) {
                    return failure(e);
                }
            } else {
                if p.expected_preview_revision.is_some() {
                    return error("invalid_argument");
                }
                if state.active.is_some() {
                    return error("preview_busy");
                }
                let config =
                    match self.with_store(|s| s.validate_theme_patch(p.base_revision, &patch)) {
                        Ok(s) => s.config,
                        Err(e) => return failure(e),
                    };
                state.active = Some(Preview::new(
                    uuid::Uuid::new_v4().to_string(),
                    owner.into(),
                    config,
                    service.now(),
                ));
            }
            (p.scene, p.mode)
        } else {
            let p: CaptureParams = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(_) => return error("invalid_argument"),
            };
            let Some(active) = state.active.as_mut() else {
                return error("preview_expired");
            };
            if let Err(e) = active.check(
                &p.preview_id,
                owner,
                p.expected_preview_revision,
                service.now(),
            ) {
                return error(e);
            }
            active.touch(service.now());
            (p.scene, p.mode)
        };
        let active = state.active.as_ref().unwrap();
        let snapshot = match active.candidate.resolve() {
            Ok(s) => s,
            Err(e) => return failure(e),
        };
        let request = RenderRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            preview_revision: active.revision,
            snapshot,
            scene,
            mode,
        };
        let mut metadata = json!({"ok":true,"affected":0,"persisted":false,"preview_id":active.id,"base_revision":active.base_revision,"preview_revision":active.revision,"request_id":request.request_id,"config_hash":request.snapshot.config_hash,"scene":scene,"mode":mode,"fixture_version":1});
        let frame = tokio::time::timeout(Duration::from_secs(10), backend.render(request)).await;
        if self.preview_owner().as_deref() != Some(owner) {
            state.active = None;
            backend.close();
            return error("preview_permission_revoked");
        }
        if state
            .active
            .as_ref()
            .is_some_and(|p| p.expired(service.now()))
        {
            state.active = None;
            backend.close();
            return error("preview_expired");
        }
        let frame = match frame {
            Ok(Ok(f)) => f,
            other => {
                let code = match other {
                    Ok(Err(e)) => e,
                    _ => "render_timeout",
                };
                // Keep a recoverable candidate and expose its id even if first capture failed.
                metadata["ok"] = json!(false);
                metadata["error_code"] = json!(code);
                metadata["retryable"] = json!(true);
                return response(metadata);
            }
        };
        if frame.png.len() > 2 * 1024 * 1024 {
            return error("image_too_large");
        }
        metadata["capture"] = frame.metadata;
        let mut result = response(metadata);
        result.content.push(ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(frame.png),
            "image/png",
        ));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Waiting {
        closed: AtomicU64,
        cancelled: Arc<AtomicU64>,
    }
    struct Dropped(Arc<AtomicU64>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Backend for Waiting {
        fn close(&self) {
            self.closed.fetch_add(1, Ordering::SeqCst);
        }
        fn render(&self, _: RenderRequest) -> RenderFuture {
            let cancelled = self.cancelled.clone();
            Box::pin(async move {
                let _guard = Dropped(cancelled);
                std::future::pending().await
            })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn deadline_drops_render_and_releases_single_flight() {
        let store = rustrss_core::Store::open_in_memory().unwrap();
        store.set_bool_setting("mcp.write_enabled", true).unwrap();
        store.set_setting("mcp.write_token", "owner").unwrap();
        let cancelled = Arc::new(AtomicU64::new(0));
        let backend = Arc::new(Waiting {
            closed: AtomicU64::new(0),
            cancelled: cancelled.clone(),
        });
        let server = RustRssMcp::new(store).with_preview_backend(backend);
        let owner = fingerprint("owner");
        let result = server
            .preview_local(
                "preview_theme",
                json!({"base_revision":0,"patch":{}}),
                &owner,
            )
            .await;
        let value = result.structured_content.unwrap();
        assert_eq!(value["error_code"], "render_timeout");
        assert_eq!(cancelled.load(Ordering::SeqCst), 1);
        let cancelled=server.preview_local("finish_theme_preview",json!({"preview_id":value["preview_id"],"expected_preview_revision":value["preview_revision"],"action":"cancel"}),&owner).await;
        assert_eq!(cancelled.structured_content.unwrap()["ok"], true);
    }
    #[test]
    fn sweep_expires_memory_and_closes_resources_without_requests() {
        let backend = Arc::new(Waiting {
            closed: AtomicU64::new(0),
            cancelled: Arc::new(AtomicU64::new(0)),
        });
        let service = Service {
            backend: Some(backend.clone()),
            state: Default::default(),
            started: Instant::now() - Duration::from_secs(600),
        };
        let mut state = State {
            active: Some(Preview::new(
                "id".into(),
                "owner".into(),
                Default::default(),
                0,
            )),
            finished: Default::default(),
        };
        service.sweep(&mut state, Some("owner"));
        assert!(state.active.is_none());
        assert_eq!(backend.closed.load(Ordering::SeqCst), 1);
    }
}
