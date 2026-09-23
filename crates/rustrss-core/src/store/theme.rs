//! Theme settings are one versioned envelope: current + bounded history commit together.
use super::{now, Store};
use crate::theme::{
    Result, ThemeConfig, ThemeError, ThemeMode, ThemePatch, ThemeSnapshot, MAX_REVISION,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;

const KEY: &str = "ui.theme_config";
const HISTORY_LIMIT: usize = 10;
const ENVELOPE_LIMIT: usize = 256 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    storage_version: u32,
    current: ThemeConfig,
    history: Vec<ThemeConfig>,
}

impl Store {
    /// Current and bounded history from the same consistent database snapshot.
    pub fn theme_state(&self) -> Result<(ThemeSnapshot, Vec<ThemeConfig>)> {
        let tx = self.conn.unchecked_transaction()?;
        let envelope = read_envelope(&tx)?;
        let result = (envelope.current.resolve()?, envelope.history);
        tx.commit()?;
        Ok(result)
    }

    /// Foreground polling reads only the small settings row, never article data.
    /// Unchanged revisions skip history validation and effective-theme resolution.
    pub fn theme_snapshot_if_changed(
        &self,
        known_revision: Option<u64>,
    ) -> Result<Option<ThemeSnapshot>> {
        let tx = self.conn.unchecked_transaction()?;
        let revision: Option<i64> = tx
            .query_row(
                "SELECT json_extract(value, '$.current.revision') FROM settings WHERE key = ?1",
                [KEY],
                |row| row.get(0),
            )
            .optional()?;
        let revision = u64::try_from(revision.unwrap_or(0))
            .map_err(|_| ThemeError::Corrupt("negative revision".into()))?;
        if known_revision == Some(revision) {
            tx.commit()?;
            return Ok(None);
        }
        let snapshot = read_envelope(&tx)?.current.resolve()?;
        tx.commit()?;
        Ok(Some(snapshot))
    }

    /// Pure read: absent versioned config maps existing settings without writing.
    pub fn theme_snapshot(&self) -> Result<ThemeSnapshot> {
        let tx = self.conn.unchecked_transaction()?;
        let result = read_envelope(&tx)?.current.resolve()?;
        tx.commit()?;
        Ok(result)
    }

    /// Oldest to newest, at most ten prior revisions; does not expose arbitrary settings.
    pub fn theme_history(&self) -> Result<Vec<ThemeConfig>> {
        let tx = self.conn.unchecked_transaction()?;
        let history = read_envelope(&tx)?.history;
        tx.commit()?;
        Ok(history)
    }

    /// Validates a candidate without persisting or advancing its revision.
    pub fn validate_theme_patch(
        &self,
        expected_revision: u64,
        patch: &ThemePatch,
    ) -> Result<ThemeSnapshot> {
        let current = self.theme_snapshot()?.config;
        check_revision(expected_revision, current.revision)?;
        current.apply(patch)?.resolve()
    }

    /// BEGIN IMMEDIATE serializes competing SQLite connections, not just one AppState mutex.
    /// A stale revision is rejected even if its patch would otherwise be a no-op.
    pub fn update_theme(
        &self,
        expected_revision: u64,
        patch: &ThemePatch,
    ) -> Result<ThemeSnapshot> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let envelope = read_envelope(&tx)?;
        check_revision(expected_revision, envelope.current.revision)?;
        let next = envelope.current.apply(patch)?;
        let snapshot = save(&tx, envelope, next)?;
        tx.commit()?;
        Ok(snapshot)
    }

    /// Existing UI controls change only their explicit fields against the latest
    /// configuration within one SQLite write transaction.
    pub fn update_theme_controls(&self, patch: &ThemePatch) -> Result<ThemeSnapshot> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let envelope = read_envelope(&tx)?;
        let next = envelope.current.apply(patch)?;
        let snapshot = save(&tx, envelope, next)?;
        tx.commit()?;
        Ok(snapshot)
    }

    /// Restoring creates a new revision; it never rolls the monotonic counter back.
    pub fn restore_theme(
        &self,
        expected_revision: u64,
        historical_revision: u64,
    ) -> Result<ThemeSnapshot> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let envelope = read_envelope(&tx)?;
        check_revision(expected_revision, envelope.current.revision)?;
        let next = if historical_revision == envelope.current.revision {
            envelope.current.clone()
        } else {
            envelope
                .history
                .iter()
                .find(|c| c.revision == historical_revision)
                .cloned()
                .ok_or(ThemeError::HistoryUnavailable(historical_revision))?
        };
        let snapshot = save(&tx, envelope, next)?;
        tx.commit()?;
        Ok(snapshot)
    }
}

fn check_revision(expected: u64, actual: u64) -> Result<()> {
    if expected != actual {
        return Err(ThemeError::RevisionConflict { expected, actual });
    }
    Ok(())
}
fn setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?", [key], |r| {
            r.get(0)
        })
        .optional()?)
}
fn read_envelope(conn: &Connection) -> Result<Envelope> {
    let Some(raw) = setting(conn, KEY)? else {
        return Ok(Envelope {
            storage_version: 1,
            current: legacy_config(conn)?,
            history: vec![],
        });
    };
    if raw.len() > ENVELOPE_LIMIT {
        return Err(ThemeError::Corrupt("envelope exceeds byte limit".into()));
    }
    let envelope: Envelope =
        serde_json::from_str(&raw).map_err(|e| ThemeError::Corrupt(e.to_string()))?;
    if envelope.storage_version != 1 {
        return Err(ThemeError::Corrupt("unsupported storage version".into()));
    }
    if envelope.history.len() > HISTORY_LIMIT {
        return Err(ThemeError::Corrupt("too much history".into()));
    }
    envelope.current.resolve()?;
    let mut last = None;
    for old in &envelope.history {
        old.resolve()?;
        if old.revision >= envelope.current.revision || last.is_some_and(|n| old.revision <= n) {
            return Err(ThemeError::Corrupt(
                "history revisions are not ordered".into(),
            ));
        }
        last = Some(old.revision);
    }
    Ok(envelope)
}
fn save(conn: &Connection, mut envelope: Envelope, mut next: ThemeConfig) -> Result<ThemeSnapshot> {
    next.revision = envelope.current.revision;
    next.resolve()?;
    if next == envelope.current {
        return next.resolve();
    }
    next.revision = next
        .revision
        .checked_add(1)
        .filter(|n| *n <= MAX_REVISION)
        .ok_or_else(|| ThemeError::Corrupt("revision exhausted".into()))?;
    let snapshot = next.resolve()?;
    envelope.history.push(envelope.current);
    if envelope.history.len() > HISTORY_LIMIT {
        envelope.history.remove(0);
    }
    envelope.current = next;
    let raw = serde_json::to_string(&envelope).map_err(|e| ThemeError::Corrupt(e.to_string()))?;
    if raw.len() > ENVELOPE_LIMIT {
        return Err(ThemeError::Corrupt("envelope exceeds byte limit".into()));
    }
    conn.execute(
        "INSERT INTO settings(key,value,updated_at) VALUES(?1,?2,?3)
        ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
        params![KEY, raw, now()],
    )?;
    Ok(snapshot)
}

fn legacy_config(conn: &Connection) -> Result<ThemeConfig> {
    let mode = match setting(conn, "ui.theme")?.as_deref().map(str::trim) {
        Some("light") => ThemeMode::Light,
        Some("dark") => ThemeMode::Dark,
        _ => ThemeMode::System,
    };
    let mut fonts = serde_json::Map::new();
    for (key, field) in [
        ("ui.font_ui", "ui_family"),
        ("ui.font_read", "read_family"),
        ("ui.font_mono", "mono_family"),
    ] {
        if let Some(raw) = setting(conn, key)? {
            // Match the legacy desktop cleanup and preserve the chosen family.
            let cleaned: String = raw
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
                .collect();
            let family = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
            if !family.is_empty() {
                fonts.insert(
                    field.into(),
                    json!([
                        family,
                        if field == "mono_family" {
                            "monospace"
                        } else {
                            "sans-serif"
                        }
                    ]),
                );
            }
        }
    }
    // The old reader inherited the UI family when no explicit reader font was set.
    if !fonts.contains_key("read_family") {
        if let Some(ui) = fonts.get("ui_family").cloned() {
            fonts.insert("read_family".into(), ui);
        }
    }
    for (key, field, default, low, high) in [
        ("ui.font_read_size", "read_size", 14.0, 13.0, 18.0),
        ("ui.font_read_line", "line_height", 1.55, 1.5, 1.8),
    ] {
        if let Some(raw) = setting(conn, key)? {
            let n = raw
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .unwrap_or(default);
            fonts.insert(
                field.into(),
                json!((n.clamp(low, high) * 100.).round() / 100.),
            );
        }
    }
    ThemeConfig::default().apply(&ThemePatch {
        mode: Some(mode),
        overrides: json!({"typography":fonts}),
        ..ThemePatch::default()
    })
}
