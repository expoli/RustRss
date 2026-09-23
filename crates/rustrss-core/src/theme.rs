//! Versioned theme data shared by future desktop and MCP callers. No CSS execution.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: u32 = 1;
pub const PRESET_VERSION: u32 = 1;
pub const MAX_PATCH_BYTES: usize = 16 * 1024;
pub const MAX_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("invalid theme field {path}: {reason}")]
    Invalid { path: String, reason: String },
    #[error("unsupported theme schema {0}")]
    UnsupportedSchema(u32),
    #[error("theme revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("theme history revision {0} is unavailable")]
    HistoryUnavailable(u64),
    #[error("theme storage is corrupt: {0}")]
    Corrupt(String),
    #[error("theme database error: {0}")]
    Database(#[from] rusqlite::Error),
}
pub type Result<T> = std::result::Result<T, ThemeError>;
fn invalid(path: &str, reason: &str) -> ThemeError {
    ThemeError::Invalid {
        path: path.into(),
        reason: reason.into(),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetId {
    #[default]
    Clear,
    Paper,
    Slate,
}
pub const PRESETS: [PresetId; 3] = [PresetId::Clear, PresetId::Paper, PresetId::Slate];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    Comfortable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderLayout {
    ThreeColumn,
    Focus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    pub background: String,
    pub sidebar: String,
    pub panel: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub selected: String,
    pub hover: String,
    pub border: String,
    pub focus: String,
    pub danger: String,
    pub star: String,
    pub code_background: String,
    pub code_text: String,
    pub code_keyword: String,
    pub code_string: String,
    pub code_number: String,
    pub code_comment: String,
    pub code_function: String,
    pub code_type: String,
    pub code_variable: String,
    pub diff_add_background: String,
    pub diff_add_text: String,
    pub diff_delete_background: String,
    pub diff_delete_text: String,
    pub diff_hunk_background: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Typography {
    pub ui_family: Vec<String>,
    pub read_family: Vec<String>,
    pub mono_family: Vec<String>,
    pub ui_size: f64,
    pub read_size: f64,
    pub mono_size: f64,
    pub line_height: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListStyle {
    pub density: Density,
    pub summary_lines: u8,
    pub thumbnail: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderStyle {
    pub width: f64,
    pub paragraph_gap: f64,
    pub layout: ReaderLayout,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChromeStyle {
    pub radius: f64,
    pub sidebar_width: f64,
    pub list_width: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeValues {
    pub colors: Colors,
    pub typography: Typography,
    pub list: ListStyle,
    pub reader: ReaderStyle,
    pub chrome: ChromeStyle,
}

fn empty_object() -> Value {
    json!({})
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeConfig {
    pub schema_version: u32,
    pub revision: u64,
    pub mode: ThemeMode,
    pub light_preset: PresetId,
    pub dark_preset: PresetId,
    /// Validated sparse tree: colors.light/dark and common typography/list/reader/chrome.
    pub overrides: Value,
}
impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            revision: 0,
            mode: ThemeMode::System,
            light_preset: PresetId::Clear,
            dark_preset: PresetId::Clear,
            overrides: empty_object(),
        }
    }
}

/// Missing fields preserve existing preferences. Inside overrides, null removes a
/// field/subtree; overrides:null clears all overrides. Arrays replace atomically.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemePatch {
    pub mode: Option<ThemeMode>,
    pub light_preset: Option<PresetId>,
    pub dark_preset: Option<PresetId>,
    #[serde(default = "empty_object")]
    pub overrides: Value,
}
impl Default for ThemePatch {
    fn default() -> Self {
        Self {
            mode: None,
            light_preset: None,
            dark_preset: None,
            overrides: empty_object(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContrastCheck {
    pub mode: ThemeMode,
    pub pair: String,
    pub ratio: f64,
    pub minimum: f64,
    pub passes: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ThemeSnapshot {
    pub config: ThemeConfig,
    pub light: ThemeValues,
    pub dark: ThemeValues,
    pub preset_version: u32,
    pub config_hash: String,
    pub contrast: Vec<ContrastCheck>,
}

pub fn preset(id: PresetId, dark: bool) -> ThemeValues {
    let (bg, side, panel, text, muted, accent, selected) = match (id, dark) {
        (PresetId::Clear, false) => (
            "#f7f8fa", "#eef1f5", "#ffffff", "#1c1f26", "#626975", "#2056cc", "#e2e8f4",
        ),
        (PresetId::Clear, true) => (
            "#14161a", "#1a1d23", "#1a1d23", "#e6e8ec", "#abb3c0", "#7ab4ff", "#2b3140",
        ),
        (PresetId::Paper, false) => (
            "#fffdf7", "#eee9df", "#faf7f0", "#39382e", "#655f53", "#875020", "#eee4d4",
        ),
        (PresetId::Paper, true) => (
            "#302b22", "#242119", "#2b271f", "#e9e0cf", "#b8ac96", "#e4b379", "#483b29",
        ),
        (PresetId::Slate, false) => (
            "#fcfcfd", "#e9edef", "#f5f7f8", "#242c34", "#52616d", "#345f64", "#dfe9e9",
        ),
        (PresetId::Slate, true) => (
            "#242d31", "#191e21", "#20272b", "#e0e8e8", "#a2b3b6", "#a5cfca", "#314845",
        ),
    };
    let danger = if dark { "#ffb3ab" } else { "#9a2c2c" };
    let green = if dark { "#7ee787" } else { "#116329" };
    let ui = vec![
        "-apple-system".into(),
        "Noto Sans CJK SC".into(),
        "Segoe UI".into(),
        "sans-serif".into(),
    ];
    ThemeValues {
        colors: Colors {
            background: bg.into(),
            sidebar: side.into(),
            panel: panel.into(),
            text: text.into(),
            muted: muted.into(),
            accent: accent.into(),
            selected: selected.into(),
            hover: selected.into(),
            border: muted.into(),
            focus: accent.into(),
            danger: danger.into(),
            star: accent.into(),
            code_background: panel.into(),
            code_text: text.into(),
            code_keyword: accent.into(),
            code_string: green.into(),
            code_number: accent.into(),
            code_comment: muted.into(),
            code_function: accent.into(),
            code_type: green.into(),
            code_variable: text.into(),
            diff_add_background: if dark { "#173322" } else { "#e1f3e4" }.into(),
            diff_add_text: green.into(),
            diff_delete_background: if dark { "#412627" } else { "#fbe5e3" }.into(),
            diff_delete_text: danger.into(),
            diff_hunk_background: selected.into(),
        },
        typography: Typography {
            ui_family: ui.clone(),
            read_family: if id == PresetId::Paper {
                vec!["Noto Serif CJK SC".into(), "Georgia".into(), "serif".into()]
            } else {
                ui
            },
            mono_family: vec![
                "JetBrains Mono".into(),
                "Cascadia Mono".into(),
                "Menlo".into(),
                "monospace".into(),
            ],
            ui_size: 14.,
            read_size: match id {
                PresetId::Clear => 14.,
                PresetId::Paper => 18.,
                PresetId::Slate => 16.,
            },
            mono_size: 13.,
            line_height: match id {
                PresetId::Clear => 1.55,
                PresetId::Paper => 1.9,
                PresetId::Slate => 1.7,
            },
        },
        list: ListStyle {
            density: if id == PresetId::Slate {
                Density::Compact
            } else {
                Density::Comfortable
            },
            summary_lines: 2,
            thumbnail: true,
        },
        reader: ReaderStyle {
            width: 680.,
            paragraph_gap: 1.,
            layout: ReaderLayout::ThreeColumn,
        },
        chrome: ChromeStyle {
            radius: if id == PresetId::Paper { 5. } else { 8. },
            sidebar_width: 220.,
            list_width: 340.,
        },
    }
}

impl ThemePatch {
    pub fn from_json(raw: &str) -> Result<Self> {
        if raw.len() > MAX_PATCH_BYTES {
            return Err(invalid("patch", "exceeds byte limit"));
        }
        serde_json::from_str(raw).map_err(|e| invalid("patch", &e.to_string()))
    }
}
impl ThemeConfig {
    pub fn apply(&self, patch: &ThemePatch) -> Result<Self> {
        self.resolve()?;
        let size = serde_json::to_vec(patch)
            .map_err(|e| invalid("patch", &e.to_string()))?
            .len();
        if size > MAX_PATCH_BYTES {
            return Err(invalid("patch", "exceeds byte limit"));
        }
        validate_override_tree(&patch.overrides, true)?;
        let mut next = self.clone();
        if let Some(mode) = patch.mode {
            next.mode = mode;
        }
        if let Some(id) = patch.light_preset {
            next.light_preset = id;
        }
        if let Some(id) = patch.dark_preset {
            next.dark_preset = id;
        }
        merge_patch(&mut next.overrides, &patch.overrides);
        if next.overrides.is_null() {
            next.overrides = empty_object();
        }
        canonicalize(&mut next.overrides, "");
        next.resolve()?;
        Ok(next)
    }

    pub fn resolve(&self) -> Result<ThemeSnapshot> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ThemeError::UnsupportedSchema(self.schema_version));
        }
        if self.revision > MAX_REVISION {
            return Err(invalid("revision", "outside safe integer range"));
        }
        if serde_json::to_vec(&self.overrides)
            .map_err(|e| invalid("overrides", &e.to_string()))?
            .len()
            > MAX_PATCH_BYTES
        {
            return Err(invalid("overrides", "exceeds byte limit"));
        }
        validate_override_tree(&self.overrides, false)?;
        let effective = |id, dark| -> Result<ThemeValues> {
            let mut base = serde_json::to_value(preset(id, dark)).unwrap();
            let mut patch = self.overrides.clone();
            if let Some(colors) = patch.get_mut("colors") {
                *colors = colors
                    .get(if dark { "dark" } else { "light" })
                    .cloned()
                    .unwrap_or_else(empty_object);
            }
            merge_patch(&mut base, &patch);
            let values: ThemeValues =
                serde_json::from_value(base).map_err(|e| invalid("overrides", &e.to_string()))?;
            validate_values(&values)?;
            Ok(values)
        };
        let light = effective(self.light_preset, false)?;
        let dark = effective(self.dark_preset, true)?;
        // Revision is excluded: identical configurations have stable fingerprints.
        let mut fingerprint = self.clone();
        fingerprint.revision = 0;
        let encoded = serde_json::to_vec(&(PRESET_VERSION, fingerprint, &light, &dark)).unwrap();
        let config_hash = format!("{:x}", Sha256::digest(encoded));
        let mut contrast = contrast_checks(&light.colors, ThemeMode::Light);
        contrast.extend(contrast_checks(&dark.colors, ThemeMode::Dark));
        Ok(ThemeSnapshot {
            config: self.clone(),
            light,
            dark,
            preset_version: PRESET_VERSION,
            config_hash,
            contrast,
        })
    }
}

// Validate patch paths even for null removals, so typo:null cannot silently pass.
fn validate_override_tree(value: &Value, patch: bool) -> Result<()> {
    let defaults = serde_json::to_value(preset(PresetId::Clear, false)).unwrap();
    let mut shape = defaults.clone();
    shape["colors"] = json!({"light":defaults["colors"], "dark":defaults["colors"]});
    fn walk(v: &Value, shape: &Value, path: &str, patch: bool) -> Result<()> {
        if v.is_null() && patch {
            return Ok(());
        }
        if let Some(fields) = shape.as_object() {
            let object = v
                .as_object()
                .ok_or_else(|| invalid(path, "expected object"))?;
            for (key, val) in object {
                let child = format!("{path}.{key}");
                let schema = fields
                    .get(key)
                    .ok_or_else(|| invalid(&child, "unknown field"))?;
                walk(val, schema, &child, patch)?;
            }
        } else if v.is_null() {
            return Err(invalid(path, "null must be removed before storage"));
        }
        Ok(())
    }
    walk(value, &shape, "overrides", patch)
}
fn merge_patch(target: &mut Value, patch: &Value) {
    if let Some(fields) = patch.as_object() {
        if !target.is_object() {
            *target = empty_object();
        }
        let target = target.as_object_mut().unwrap();
        for (key, value) in fields {
            if value.is_null() {
                target.remove(key);
            } else {
                merge_patch(target.entry(key).or_insert(Value::Null), value);
            }
        }
    } else {
        *target = patch.clone();
    }
}
fn canonicalize(value: &mut Value, path: &str) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                canonicalize(val, &format!("{path}.{key}"));
            }
            map.retain(|_, val| !val.as_object().is_some_and(|v| v.is_empty()));
        }
        Value::String(s) if path.starts_with(".colors.") => *s = s.to_ascii_lowercase(),
        // JSON 20 and 20.0 represent the same size. Keep integer-only fields
        // typed as integers and normalize the remaining numeric overrides.
        Value::Number(n) if path != ".list.summary_lines" => {
            if let Some(normalized) = n.as_f64().and_then(serde_json::Number::from_f64) {
                *n = normalized;
            }
        }
        _ => {}
    }
}
fn range(path: &str, value: f64, min: f64, max: f64) -> Result<()> {
    if !value.is_finite() || value < min || value > max {
        return Err(invalid(path, "outside allowed range"));
    }
    Ok(())
}
fn validate_values(v: &ThemeValues) -> Result<()> {
    for (key, color) in serde_json::to_value(&v.colors)
        .unwrap()
        .as_object()
        .unwrap()
    {
        let s = color.as_str().unwrap();
        if s.len() != 7
            || !s.starts_with('#')
            || !s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        {
            return Err(invalid(&format!("colors.{key}"), "expected #RRGGBB"));
        }
    }
    for (key, families) in [
        ("ui_family", &v.typography.ui_family),
        ("read_family", &v.typography.read_family),
        ("mono_family", &v.typography.mono_family),
    ] {
        if families.is_empty() || families.len() > 4 {
            return Err(invalid(key, "expected 1 to 4 font families"));
        }
        for family in families {
            if family.trim().is_empty()
                || family.chars().count() > 128
                || family.chars().any(char::is_control)
            {
                return Err(invalid(key, "invalid font family"));
            }
        }
    }
    for (path, n, low, high) in [
        ("typography.ui_size", v.typography.ui_size, 12., 20.),
        ("typography.read_size", v.typography.read_size, 13., 28.),
        ("typography.mono_size", v.typography.mono_size, 12., 24.),
        ("typography.line_height", v.typography.line_height, 1.3, 2.2),
        ("reader.width", v.reader.width, 480., 960.),
        ("reader.paragraph_gap", v.reader.paragraph_gap, 0.5, 2.),
        ("chrome.radius", v.chrome.radius, 0., 16.),
        ("chrome.sidebar_width", v.chrome.sidebar_width, 180., 300.),
        ("chrome.list_width", v.chrome.list_width, 260., 460.),
    ] {
        range(path, n, low, high)?;
    }
    if v.list.summary_lines > 3 {
        return Err(invalid("list.summary_lines", "expected 0 to 3"));
    }
    Ok(())
}
fn luminance(hex: &str) -> f64 {
    let channel = |start| {
        let x = u8::from_str_radix(&hex[start..start + 2], 16).unwrap() as f64 / 255.;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
}
fn contrast_checks(c: &Colors, mode: ThemeMode) -> Vec<ContrastCheck> {
    [
        ("body", &c.text, &c.background, 4.5),
        ("secondary", &c.muted, &c.panel, 4.5),
        ("selection", &c.text, &c.selected, 4.5),
        ("link", &c.accent, &c.background, 4.5),
        ("focus", &c.focus, &c.panel, 3.),
        ("code", &c.code_text, &c.code_background, 4.5),
        ("diff_add", &c.diff_add_text, &c.diff_add_background, 4.5),
        (
            "diff_delete",
            &c.diff_delete_text,
            &c.diff_delete_background,
            4.5,
        ),
    ]
    .into_iter()
    .map(|(pair, fg, bg, minimum)| {
        let a = luminance(fg);
        let b = luminance(bg);
        let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        ContrastCheck {
            mode,
            pair: pair.into(),
            ratio,
            minimum,
            passes: ratio >= minimum,
        }
    })
    .collect()
}

/// Compatibility inputs from the existing desktop controls. None preserves a field.
#[derive(Default)]
pub struct FontPatch {
    pub ui: Option<String>,
    pub read: Option<String>,
    pub mono: Option<String>,
    pub size: Option<f64>,
    pub line: Option<f64>,
}
pub fn normalize_font_family(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter_map(|c| {
            if !c.is_control() {
                Some(c)
            } else if c.is_whitespace() {
                Some(' ')
            } else {
                None
            }
        })
        .take(100)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn clamp_font_number(value: f64, default: f64, (min, max): (f64, f64)) -> f64 {
    if !value.is_finite() {
        return default;
    }
    (value.clamp(min, max) * 100.).round() / 100.
}
impl FontPatch {
    pub fn theme_patch(self) -> ThemePatch {
        let mut fields = serde_json::Map::new();
        for (key, input, fallback) in [
            ("ui_family", self.ui, "sans-serif"),
            ("read_family", self.read, "sans-serif"),
            ("mono_family", self.mono, "monospace"),
        ] {
            if let Some(raw) = input {
                let name = normalize_font_family(&raw);
                fields.insert(
                    key.into(),
                    if name.is_empty() {
                        Value::Null
                    } else {
                        json!([name, fallback])
                    },
                );
            }
        }
        if let Some(n) = self.size {
            fields.insert(
                "read_size".into(),
                json!(clamp_font_number(n, 14., (13., 28.))),
            );
        }
        if let Some(n) = self.line {
            fields.insert(
                "line_height".into(),
                json!(clamp_font_number(n, 1.55, (1.3, 2.2))),
            );
        }
        ThemePatch {
            overrides: json!({"typography":fields}),
            ..ThemePatch::default()
        }
    }
}
impl ThemeSnapshot {
    /// Empty string means inherit the chosen preset, not a second legacy store.
    pub fn font_override(&self, field: &str) -> String {
        self.config
            .overrides
            .get("typography")
            .and_then(|v| v.get(field))
            .and_then(|v| v.get(0))
            .and_then(Value::as_str)
            .unwrap_or("")
            .into()
    }
}

/// Discoverable sparse-patch schema. Validation remains authoritative in apply().
pub fn patch_schema() -> Value {
    fn node(value: &Value, path: &str) -> Value {
        let schema = if let Some(fields) = value.as_object() {
            let properties = fields
                .iter()
                .map(|(k, v)| (k.clone(), node(v, &format!("{path}.{k}"))))
                .collect::<serde_json::Map<_, _>>();
            json!({"type":"object","additionalProperties":false,"properties":properties})
        } else if value.is_array() {
            json!({"type":"array","minItems":1,"maxItems":4,"items":{"type":"string","minLength":1,"maxLength":128,"description":"Nonblank family name, no control characters; missing fonts may fall back."}})
        } else if value.is_boolean() {
            json!({"type":"boolean"})
        } else if value.is_string() {
            match path {
                "list.density" => json!({"enum":["compact","comfortable"]}),
                "reader.layout" => json!({"enum":["three_column","focus"]}),
                _ => json!({"type":"string","pattern":"^#[0-9a-fA-F]{6}$"}),
            }
        } else {
            let (low, high) = match path {
                "typography.ui_size" => (12., 20.),
                "typography.read_size" => (13., 28.),
                "typography.mono_size" => (12., 24.),
                "typography.line_height" => (1.3, 2.2),
                "reader.width" => (480., 960.),
                "reader.paragraph_gap" => (0.5, 2.),
                "chrome.radius" => (0., 16.),
                "chrome.sidebar_width" => (180., 300.),
                "chrome.list_width" => (260., 460.),
                "list.summary_lines" => (0., 3.),
                _ => unreachable!("unknown theme numeric field"),
            };
            json!({"type":if path=="list.summary_lines" {"integer"} else {"number"},"minimum":low,"maximum":high})
        };
        json!({"anyOf":[schema,{"type":"null"}],"description":"Omit to preserve; null removes the override."})
    }
    let sample = serde_json::to_value(preset(PresetId::Clear, false)).unwrap();
    let mut properties = serde_json::Map::new();
    for (key, value) in sample.as_object().unwrap() {
        properties.insert(
            key.clone(),
            if key == "colors" {
                node(&json!({"light":value,"dark":value}), "colors")
            } else {
                node(value, key)
            },
        );
    }
    json!({"type":"object","additionalProperties":false,"max_serialized_bytes":MAX_PATCH_BYTES,
    "properties":{
        "mode":{"enum":["system","light","dark"]},
        "light_preset":{"enum":["clear","paper","slate"]},"dark_preset":{"enum":["clear","paper","slate"]},
        "overrides":{"anyOf":[{"type":"object","additionalProperties":false,"properties":properties},{"type":"null"}],"description":"null clears all overrides; arrays replace atomically."}
    }})
}
