use rustrss_core::theme::{PresetId, ThemeConfig, ThemeError, ThemeMode, ThemePatch, PRESETS};
use rustrss_core::Store;
use serde_json::{json, Value};

fn patch(v: Value) -> ThemePatch {
    ThemePatch::from_json(&v.to_string()).unwrap()
}

#[test]
fn six_presets_resolve_and_pass_declared_contrast_pairs() {
    for id in PRESETS {
        let c = ThemeConfig {
            light_preset: id,
            dark_preset: id,
            ..ThemeConfig::default()
        };
        let snapshot = c.resolve().unwrap();
        assert_eq!(snapshot.config_hash.len(), 64);
        for pair in snapshot.contrast {
            assert!(pair.passes, "{id:?}: {pair:?}");
        }
    }
}

#[test]
fn legacy_read_preserves_preferences_and_does_not_write() {
    let s = Store::open_in_memory().unwrap();
    for (k, v) in [
        ("ui.theme", "dark"),
        ("ui.font_ui", "  Noto\tSans  "),
        ("ui.font_read_size", "13"),
        ("ui.font_read_line", "1.7"),
        ("unrelated", "keep"),
    ] {
        s.set_setting(k, v).unwrap();
    }
    let before = s.all_settings().unwrap();
    let snap = s.theme_snapshot().unwrap();
    assert_eq!(snap.config.mode, ThemeMode::Dark);
    assert_eq!(snap.light.typography.read_size, 13.);
    assert_eq!(snap.light.typography.line_height, 1.7);
    assert_eq!(snap.light.typography.ui_family[0], "Noto Sans");
    assert_eq!(
        snap.light.typography.read_family,
        snap.light.typography.ui_family
    );
    assert_eq!(before, s.all_settings().unwrap());
    assert!(s.theme_history().unwrap().is_empty());
    let saved = s
        .update_theme(0, &patch(json!({"light_preset":"paper"})))
        .unwrap();
    assert_eq!(saved.config.revision, 1);
    assert_eq!(saved.light.typography.read_size, 13.);
    for (k, v) in before {
        assert_eq!(s.setting(&k).unwrap(), Some(v));
    }
}

#[test]
fn broken_legacy_values_follow_old_fallbacks() {
    let s = Store::open_in_memory().unwrap();
    s.set_setting("ui.theme", "invalid").unwrap();
    s.set_setting("ui.font_read_size", "999").unwrap();
    s.set_setting("ui.font_read_line", "NaN").unwrap();
    let snap = s.theme_snapshot().unwrap();
    assert_eq!(snap.config.mode, ThemeMode::System);
    assert_eq!(snap.light.typography.read_size, 18.);
    assert_eq!(snap.light.typography.line_height, 1.55);
}

#[test]
fn patch_is_partial_with_mode_specific_colors_and_null_inheritance() {
    let c = ThemeConfig::default()
        .apply(&patch(json!({"overrides":{
            "colors":{"light":{"accent":"#AABBCC"}},
            "typography":{"read_size":20,"read_family":["Example","serif"]}
        }})))
        .unwrap();
    let snap = c.resolve().unwrap();
    assert_eq!(snap.light.colors.accent, "#aabbcc");
    assert_ne!(snap.dark.colors.accent, "#aabbcc");
    assert_eq!(snap.dark.typography.read_size, 20.);
    let next = c
        .apply(&patch(
            json!({"light_preset":"paper","overrides":{"typography":{"read_family":null}}}),
        ))
        .unwrap();
    assert_eq!(next.resolve().unwrap().light.typography.read_size, 20.);
    assert_eq!(
        next.resolve().unwrap().light.typography.read_family[0],
        "Noto Serif CJK SC"
    );
    assert_eq!(next.apply(&ThemePatch::default()).unwrap(), next);
    let reset = next.apply(&patch(json!({"overrides":null}))).unwrap();
    assert_eq!(reset.overrides, json!({}));
    assert_eq!(reset.resolve().unwrap().light.typography.read_size, 18.);
}

#[test]
fn validation_rejects_unknown_null_paths_types_and_ranges_without_writes() {
    let s = Store::open_in_memory().unwrap();
    let before = s.all_settings().unwrap();
    for value in [
        json!({"bogus":null}),
        json!({"colors":{"light":{"bogus":null}}}),
        json!({"typography":{"read_size":12}}),
        json!({"typography":{"read_size":"18"}}),
        json!({"typography":{"read_family":[]}}),
        json!({"typography":{"read_family":["bad\nfont"]}}),
        json!({"typography":{"read_family":["1","2","3","4","5"]}}),
        json!({"colors":{"dark":{"text":"red"}}}),
        json!({"colors":{"dark":{"text":"#ffffff00"}}}),
        json!({"reader":{"width":10000}}),
        json!({"list":{"summary_lines":4}}),
        json!({"chrome":{"radius":-1}}),
        json!({"list":{"density":"unknown"}}),
        json!({"reader":{"layout":"arbitrary_html"}}),
    ] {
        assert!(s
            .update_theme(0, &patch(json!({"overrides":value})))
            .is_err());
        assert_eq!(before, s.all_settings().unwrap());
    }
    assert!(ThemePatch::from_json(r#"{"revision":100}"#).is_err());
    assert!(ThemePatch::from_json(&format!(
        "{{\"overrides\":{{\"x\":\"{}\"}}}}",
        "a".repeat(17_000)
    ))
    .is_err());
}

#[test]
fn contrast_warnings_do_not_reject_a_valid_user_palette() {
    let c = ThemeConfig::default()
        .apply(&patch(
            json!({"overrides":{"colors":{"light":{"text":"#ffffff","background":"#ffffff"}}}}),
        ))
        .unwrap();
    assert!(c.resolve().unwrap().contrast.iter().any(|x| !x.passes));
}

#[test]
fn preview_validation_is_read_only_and_stale_revision_is_rejected() {
    let s = Store::open_in_memory().unwrap();
    let before = s.all_settings().unwrap();
    assert_eq!(
        s.validate_theme_patch(0, &patch(json!({"mode":"dark"})))
            .unwrap()
            .config
            .revision,
        0
    );
    assert_eq!(before, s.all_settings().unwrap());
    s.update_theme(0, &patch(json!({"mode":"dark"}))).unwrap();
    assert!(matches!(
        s.validate_theme_patch(0, &ThemePatch::default()),
        Err(ThemeError::RevisionConflict { .. })
    ));
    assert!(matches!(
        s.update_theme(0, &ThemePatch::default()),
        Err(ThemeError::RevisionConflict { .. })
    ));
}

#[test]
fn same_value_patch_does_not_write_or_advance_revision() {
    let s = Store::open_in_memory().unwrap();
    s.update_theme(0, &ThemePatch::default()).unwrap();
    assert!(s.setting("ui.theme_config").unwrap().is_none());
    let a = s
        .update_theme(
            0,
            &patch(json!({"overrides":{"typography":{"read_size":20}}})),
        )
        .unwrap();
    let raw = s.setting("ui.theme_config").unwrap();
    let b = s
        .update_theme(
            1,
            &patch(json!({"overrides":{"typography":{"read_size":20.0}}})),
        )
        .unwrap();
    assert_eq!(a.config.revision, b.config.revision);
    assert_eq!(a.config_hash, b.config_hash);
    assert_eq!(raw, s.setting("ui.theme_config").unwrap());
}

#[test]
fn history_is_bounded_and_restore_advances_revision() {
    let s = Store::open_in_memory().unwrap();
    for i in 0..14 {
        s.update_theme(i, &patch(json!({"overrides":{"chrome":{"radius":i+1}}})))
            .unwrap();
    }
    let history = s.theme_history().unwrap();
    assert_eq!(
        history.iter().map(|c| c.revision).collect::<Vec<_>>(),
        (4..14).collect::<Vec<_>>()
    );
    let restored = s.restore_theme(14, 4).unwrap();
    assert_eq!(restored.config.revision, 15);
    assert_eq!(restored.light.chrome.radius, 4.);
    assert!(matches!(
        s.restore_theme(15, 0),
        Err(ThemeError::HistoryUnavailable(0))
    ));
    assert_eq!(s.restore_theme(15, 15).unwrap().config.revision, 15);
}

#[test]
fn corrupt_or_future_settings_are_not_overwritten() {
    let s = Store::open_in_memory().unwrap();
    for raw in ["broken".to_string(), json!({"storage_version":1,"current":{
        "schema_version":99,"revision":1,"mode":"system","light_preset":"clear","dark_preset":"clear","overrides":{}
    },"history":[]}).to_string()] {
        s.set_setting("ui.theme_config",&raw).unwrap();
        assert!(s.theme_snapshot().is_err());
        assert!(s.update_theme(0,&patch(json!({"mode":"dark"}))).is_err());
        assert_eq!(s.setting("ui.theme_config").unwrap().unwrap(),raw);
    }
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustrss-theme-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn db(&self) -> std::path::PathBuf {
        self.0.join("theme.sqlite")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn separate_connections_compare_and_swap_and_reopen() {
    let fixture = Fixture::new();
    let a = Store::open(fixture.db()).unwrap();
    let b = Store::open(fixture.db()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = [a, b]
        .into_iter()
        .enumerate()
        .map(|(n, store)| {
            let gate = barrier.clone();
            std::thread::spawn(move || {
                gate.wait();
                store.update_theme(0, &patch(json!({"mode":if n==0 {"light"} else {"dark"}})))
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(ThemeError::RevisionConflict { .. })))
            .count(),
        1
    );
    let reopened = Store::open(fixture.db()).unwrap();
    assert_eq!(reopened.theme_snapshot().unwrap().config.revision, 1);
    assert_eq!(reopened.theme_history().unwrap().len(), 1);
}

#[test]
fn failed_sql_write_rolls_back_current_and_history() {
    let fixture = Fixture::new();
    let s = Store::open(fixture.db()).unwrap();
    s.update_theme(0, &patch(json!({"mode":"dark"}))).unwrap();
    let before = s.setting("ui.theme_config").unwrap();
    let conn = rusqlite::Connection::open(fixture.db()).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER reject_theme BEFORE INSERT ON settings WHEN NEW.key='ui.theme_config'
        BEGIN SELECT RAISE(ABORT,'injected theme write failure'); END;",
    )
    .unwrap();
    // The trigger rejects even identical UPSERTs; a no-op must perform no write.
    assert_eq!(
        s.update_theme(1, &patch(json!({"mode":"dark"})))
            .unwrap()
            .config
            .revision,
        1
    );
    assert!(s.update_theme(1, &patch(json!({"mode":"light"}))).is_err());
    assert_eq!(s.setting("ui.theme_config").unwrap(), before);
    conn.execute_batch("DROP TRIGGER reject_theme").unwrap();
    assert_eq!(
        s.update_theme(1, &patch(json!({"mode":"light"})))
            .unwrap()
            .config
            .revision,
        2
    );
}

#[test]
fn hash_ignores_revision_and_retains_preset_identity() {
    let a = ThemeConfig::default();
    let mut b = a.clone();
    b.revision = 5;
    assert_eq!(
        a.resolve().unwrap().config_hash,
        b.resolve().unwrap().config_hash
    );
    b.light_preset = PresetId::Paper;
    assert_ne!(
        a.resolve().unwrap().config_hash,
        b.resolve().unwrap().config_hash
    );
}
