//! Reproducible isolated fixture and count timings. Never use a live database for init.
use rustrss_core::{Entry, IdOrigin, Store};
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = Path::new(args.get(2).ok_or("usage: scope_counts init|feed|folder|tag PATH")?);
    let mode = args.get(1).ok_or("missing mode")?;
    if mode == "init" {
        if path.exists() { return Err("init refuses to overwrite an existing file".into()); }
        let store = Store::open(path)?;
        let folder = store.add_folder("Verification folder")?;
        let tag = store.create_tag("Verification tag", None)?;
        let body = "local benchmark content ".repeat(512);
        for f in 0..120 {
            let feed = store.add_feed(&format!("https://example.invalid/{f}"), Some(&format!("Fixture {f}")))?;
            if f < 60 { store.assign_folder(feed, Some(folder))?; }
            let rows: Vec<_> = (0..100).map(|n| Entry {
                stable_id: format!("{f}-{n}"), source_id: format!("{f}-{n}"), id_origin: IdOrigin::SourceData,
                title: format!("Fixture {f} article {n}"), url: None, author: None,
                published: None, updated: None, summary: Some("fixture".into()),
                content_html: Some(body.clone()), content_text: Some(body.clone()), thumbnail_url: None, categories: vec![],
            }).collect();
            store.upsert_entries(feed, &rows)?;
        }
        store.set_setting("refresh.on_start", "false")?;
        store.set_setting("refresh.interval_minutes", "off")?;
        // Assign a representative tag without fetching article bodies.
        let conn = rusqlite::Connection::open(path)?;
        conn.execute("INSERT INTO entry_tags(entry_id, tag_id) SELECT id, ? FROM entries WHERE feed_id <= 30", [tag.id])?;
        conn.execute("UPDATE entries SET starred = 1 WHERE feed_id = 1", [])?;
        println!("{}", serde_json::json!({"entries": 12000, "folder": folder, "tag": tag.id}));
        return Ok(());
    }
    let begin = Instant::now();
    let store = Store::open(path)?;
    let open_ms = begin.elapsed().as_secs_f64() * 1000.;
    let count = || match mode.as_str() {
        "feed" => store.entry_count_for_feed(1),
        "folder" => store.entry_count_for_folder(1),
        "tag" => store.tag_entry_count(1),
        _ => Err(rustrss_core::StoreError::Invalid("unknown mode".into())),
    };
    let begin = Instant::now();
    let n = count()?;
    let first_ms = begin.elapsed().as_secs_f64() * 1000.;
    let begin = Instant::now();
    for _ in 0..100 { assert_eq!(count()?, n); }
    println!("{}", serde_json::json!({"scope": mode, "count": n, "open_ms": open_ms,
        "first_ms": first_ms, "warm_mean_ms": begin.elapsed().as_secs_f64() * 10.}));
    Ok(())
}
