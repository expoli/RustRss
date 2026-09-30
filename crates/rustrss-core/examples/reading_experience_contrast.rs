//! Audit the built-in theme values used by the reading experience controls.
use rustrss_core::theme::{ThemeConfig, PRESETS};
use serde_json::json;

fn luminance(hex: &str) -> f64 {
    let values = [1, 3, 5].map(|start| {
        let x = u8::from_str_radix(&hex[start..start + 2], 16).unwrap() as f64 / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * values[0] + 0.7152 * values[1] + 0.0722 * values[2]
}
fn ratio(a: &str, b: &str) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut rows = vec![];
    let mut failed = vec![];
    for preset in PRESETS {
        let snapshot = ThemeConfig {
            light_preset: preset,
            dark_preset: preset,
            ..Default::default()
        }
        .resolve()?;
        for (mode, values) in [("light", snapshot.light), ("dark", snapshot.dark)] {
            let c = values.colors;
            let checks = [
                ("text/background", ratio(&c.text, &c.background), 4.5),
                ("text/panel", ratio(&c.text, &c.panel), 4.5),
                ("muted/background", ratio(&c.muted, &c.background), 4.5),
                ("muted/panel", ratio(&c.muted, &c.panel), 4.5),
                ("primary-label/accent", ratio(&c.panel, &c.accent), 4.5),
                ("accent/panel", ratio(&c.accent, &c.panel), 3.0),
                ("focus/background", ratio(&c.focus, &c.background), 3.0),
                ("focus/panel", ratio(&c.focus, &c.panel), 3.0),
                ("danger/panel", ratio(&c.danger, &c.panel), 3.0),
                ("border/panel", ratio(&c.border, &c.panel), 3.0),
            ];
            for (name, measured, minimum) in checks {
                if measured < minimum {
                    failed.push(format!(
                        "{preset:?}/{mode} {name}: {measured:.2} < {minimum:.1}"
                    ));
                }
                rows.push(json!({"preset":format!("{preset:?}").to_lowercase(),"mode":mode,"pair":name,"ratio":(measured*100.0).round()/100.0,"minimum":minimum}));
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"checks":rows,"failures":failed}))?
    );
    if failed.is_empty() {
        Ok(())
    } else {
        Err("built-in color contrast failed".into())
    }
}
