//! Small isolated production UI fixture. Refuses to replace any existing database.
use rustrss_core::{Entry, IdOrigin, Store};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("theme_fixture NEW_DATABASE")?;
    if std::path::Path::new(&path).exists() {
        return Err("database already exists".into());
    }
    let store = Store::open(path)?;
    let feed = store.add_feed(
        "https://example.invalid/theme",
        Some("Theme fixture · 主题示例"),
    )?;
    let body="<h2>Readable typography · 可读排版</h2><p>Local fixture. 这是本地固定示例。</p><pre><code class=\"language-rust\">fn main() { println!(\"Hello\"); }</code></pre>".to_owned()+&"<p>阅读中的位置应该保持稳定。 Shared typography should preserve the current paragraph and selected row. </p>".repeat(80);
    let entries = (0..30)
        .map(|i| Entry {
            stable_id: format!("theme-{i}"),
            source_id: format!("theme-{i}"),
            id_origin: IdOrigin::SourceData,
            title: format!("Reading thoughtfully · 阅读与排版 {i}"),
            url: None,
            author: Some("Local fixture".into()),
            published: chrono::DateTime::from_timestamp(1790121600, 0),
            updated: None,
            summary: Some(
                "A local sample for checking theme rendering. 中文摘要检查换行与密度。".into(),
            ),
            content_html: Some(body.clone()),
            content_text: Some("Local fixture".into()),
            categories: vec![],
        })
        .collect::<Vec<_>>();
    store.upsert_entries(feed, &entries)?;
    for (k, v) in [
        ("refresh.on_start", "false"),
        ("refresh.interval_minutes", "off"),
        ("mcp.enabled", "false"),
        ("ui.locale", "en"),
        ("ui.theme", "light"),
    ] {
        store.set_setting(k, v)?;
    }
    Ok(())
}
