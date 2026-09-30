//! Deterministic, isolated UI fixture. Refuses to replace an existing database.
use rustrss_core::{Entry, IdOrigin, Store};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: reading_experience_fixture NEW_DATABASE 30|200")?;
    let count: usize = args.next().ok_or("expected 30 or 200")?.parse()?;
    if args.next().is_some() || !matches!(count, 30 | 200) {
        return Err("expected 30 or 200".into());
    }
    if std::path::Path::new(&path).exists() {
        return Err("database already exists".into());
    }

    let store = Store::open(&path)?;
    let feeds = [
        store.add_feed(
            "https://example.invalid/zh.xml",
            Some("中文阅读 · 日常观察"),
        )?,
        store.add_feed(
            "https://example.invalid/en.xml",
            Some("A deliberately long subscription title for wrapping and menu placement"),
        )?,
        store.add_feed("https://example.invalid/mixed.xml", Some("混合 / Mixed"))?,
    ];
    for index in 0..24 {
        store.add_folder(&format!("Folder {index:02} · 分类与长名称"))?;
    }

    for (feed_index, feed_id) in feeds.iter().enumerate() {
        let entries = (feed_index..count)
            .step_by(feeds.len())
            .map(|i| {
                let title = if i % 7 == 0 {
                    format!("{i:03} · 一篇用于测试长标题换行的文章：阅读布局在中文和 English words 混排时必须完整保留标题与操作入口")
                } else if i % 2 == 0 {
                    format!("{i:03} · 安静地阅读：字阶与留白")
                } else {
                    format!("{i:03} · Reading with clear hierarchy")
                };
                let media = if i % 2 == 0 {
                    "<figure><img src=\"https://example.invalid/fixture-image.png\" alt=\"Fixture illustration\"><figcaption>图文说明 · Caption</figcaption></figure>"
                } else {
                    ""
                };
                Entry {
                    stable_id: format!("reading-{i:03}"),
                    source_id: format!("reading-{i:03}"),
                    id_origin: IdOrigin::SourceData,
                    title,
                    url: None,
                    author: Some(if i % 2 == 0 { "示例作者" } else { "Fixture author" }.into()),
                    published: chrono::DateTime::from_timestamp(1790726400 - i as i64 * 3600, 0),
                    updated: None,
                    summary: Some("隔离测试摘要。A local summary for reading, truncation and text scaling.".into()),
                    content_html: Some(format!(
                        "<p>阅读从正文开始。Reading starts here.</p>{media}<pre><code class=\"language-rust\">fn main() {{ println!(\"fixture\"); }}</code></pre><table><tr><th>项目</th><th>Value</th></tr><tr><td>阅读</td><td>{i}</td></tr></table>{}",
                        "<p>长文段落。Long article paragraph with mixed language and stable scroll position.</p>".repeat(if i == 0 { 80 } else { 3 })
                    )),
                    content_text: Some("Local fixture. 本地测试。".into()),
                    thumbnail_url: if i % 2 == 0 { Some("https://example.invalid/fixture-image.png".into()) } else { None },
                    categories: vec![],
                }
            })
            .collect::<Vec<_>>();
        store.upsert_entries(*feed_id, &entries)?;
    }

    // Independent flags: all four combinations occur, including starred + later.
    let conn = rusqlite::Connection::open(&path)?;
    conn.execute(
        "UPDATE entries SET read = CAST(substr(stable_id, 9) AS INTEGER) % 2, starred = (CAST(substr(stable_id, 9) AS INTEGER) / 2) % 2, read_later = (CAST(substr(stable_id, 9) AS INTEGER) / 4) % 2",
        [],
    )?;
    for (key, value) in [
        ("refresh.on_start", "false"),
        ("refresh.interval_minutes", "off"),
        ("mcp.enabled", "false"),
        ("ui.locale", "zh-CN"),
        ("ui.theme", "light"),
    ] {
        store.set_setting(key, value)?;
    }
    println!("created {count} entries and 24 folders in {path}");
    Ok(())
}
