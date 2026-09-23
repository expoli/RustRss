//! Theme configuration tools share core validation/storage with desktop controls.
use crate::{write_contract::WriteOutcome, RustRssMcp};
use rmcp::model::{CallToolResult, ContentBlock};
use rustrss_core::theme::{ThemeError, ThemePatch, ThemeSnapshot, PRESETS, PRESET_VERSION};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GetThemeParams {
    /// Include the bounded JSON schema for patch parameters.
    pub include_schema: bool,
    /// Internal bridge discovery: do not forward discovery again.
    pub local_only: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThemePatchParams {
    /// Revision from get_theme. Stale values fail even for a no-op.
    pub expected_revision: u64,
    /// Sparse theme patch. Obtain its schema via get_theme(include_schema=true).
    /// Omitted keys preserve values; overrides:null clears overrides;
    /// nested null removes an override and inherits the selected preset.
    pub patch: Value,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreThemeParams {
    pub expected_revision: u64,
    /// A revision listed by get_theme.history. Restore creates a new revision.
    pub historical_revision: u64,
}
pub(crate) fn response(value: Value) -> CallToolResult {
    let error = value.get("ok") == Some(&Value::Bool(false));
    let mut result = if error {
        CallToolResult::error(vec![ContentBlock::text(value.to_string())])
    } else {
        CallToolResult::success(vec![ContentBlock::text(value.to_string())])
    };
    result.structured_content = Some(value);
    result
}
pub(crate) fn failure(error: ThemeError) -> CallToolResult {
    let code = match &error {
        ThemeError::RevisionConflict { .. } => "revision_conflict",
        ThemeError::HistoryUnavailable(_) => "history_unavailable",
        ThemeError::UnsupportedSchema(_) => "unsupported_schema",
        ThemeError::Invalid { .. } => "invalid_argument",
        ThemeError::Corrupt(_) => "theme_storage_corrupt",
        ThemeError::Database(_) => "internal_error",
    };
    let mut value =
        serde_json::to_value(WriteOutcome::failed_with(code, error.to_string())).unwrap();
    if let ThemeError::RevisionConflict { expected, actual } = error {
        value["expected_revision"] = json!(expected);
        value["actual_revision"] = json!(actual);
    }
    response(value)
}
impl RustRssMcp {
    pub fn get_theme_result(&self, p: &GetThemeParams) -> CallToolResult {
        match self.with_store(|s| s.theme_state()) {
            Ok((snapshot, history)) => {
                let mut value = json!({"ok":true,"theme":snapshot,"history":history.iter().map(|c|json!({
                    "revision":c.revision,"mode":c.mode,"light_preset":c.light_preset,"dark_preset":c.dark_preset
                })).collect::<Vec<_>>(),"capabilities":{
                    "configuration":true,
                    "change_notification":self.theme_changed.is_some(),
                    "preview":self.preview.capability(),"profile_id":self.profile_id
                }});
                if p.include_schema {
                    value["patch_schema"] = rustrss_core::theme::patch_schema();
                }
                response(value)
            }
            Err(e) => failure(e),
        }
    }
    pub fn theme_presets_result(&self) -> CallToolResult {
        response(
            json!({"ok":true,"preset_version":PRESET_VERSION,"count":3,"presets":PRESETS.iter().map(|id|{
            let label=match id {rustrss_core::theme::PresetId::Clear=>"Clear",rustrss_core::theme::PresetId::Paper=>"Paper",rustrss_core::theme::PresetId::Slate=>"Slate"};
            json!({"id":id,"name":label,"modes":["light","dark"]})
        }).collect::<Vec<_>>()}),
        )
    }
    pub fn validate_theme_result(&self, p: &ThemePatchParams) -> CallToolResult {
        let patch = match ThemePatch::from_json(&p.patch.to_string()) {
            Ok(p) => p,
            Err(e) => return failure(e),
        };
        match self.with_store(|s| s.validate_theme_patch(p.expected_revision, &patch)) {
            Ok(snapshot) => response(
                json!({"ok":true,"persisted":false,"base_revision":p.expected_revision,"theme":snapshot}),
            ),
            Err(e) => failure(e),
        }
    }
    pub fn update_theme_result(&self, p: &ThemePatchParams) -> CallToolResult {
        let patch = match ThemePatch::from_json(&p.patch.to_string()) {
            Ok(p) => p,
            Err(e) => return failure(e),
        };
        self.theme_write_result(
            p.expected_revision,
            self.with_store(|s| s.update_theme(p.expected_revision, &patch)),
        )
    }
    pub fn restore_theme_result(&self, p: &RestoreThemeParams) -> CallToolResult {
        self.theme_write_result(
            p.expected_revision,
            self.with_store(|s| s.restore_theme(p.expected_revision, p.historical_revision)),
        )
    }
    fn theme_write_result(
        &self,
        previous: u64,
        result: Result<ThemeSnapshot, ThemeError>,
    ) -> CallToolResult {
        match result {
            Err(e) => failure(e),
            Ok(snapshot) => {
                let changed = snapshot.config.revision != previous;
                // Store lock is released before notifying a desktop host. An event
                // queued successfully is not an acknowledgement of a rendered frame.
                let live_apply = if !changed {
                    "unchanged"
                } else if self
                    .theme_changed
                    .as_ref()
                    .is_some_and(|f| f(snapshot.config.revision))
                {
                    "pending"
                } else {
                    "unavailable"
                };
                response(serde_json::to_value(WriteOutcome::done(i64::from(changed),vec![],false).with_detail(json!({
                    "saved_revision":snapshot.config.revision,"live_apply":live_apply,"theme":snapshot
                }))).unwrap())
            }
        }
    }
}
