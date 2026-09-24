//! Isolated 10k search fixture and timings using production indexing and queries.
use rustrss_core::{Entry, IdOrigin, Store};
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).ok_or("usage: search_scale init|QUERY PATH")?;
    let path = Path::new(args.get(2).ok_or("missing isolated database path")?);
    if mode == "init" && path.exists() {
        return Err("refuse to overwrite existing fixture".into());
    }
    let start = Instant::now();
    let store = Store::open(path)?;
    let open_ms = start.elapsed().as_secs_f64() * 1000.;
    if mode == "init" {
        let feed = store.add_feed("https://fixture.invalid/search", Some("Search fixture"))?;
        for batch in 0..100 {
            let rows: Vec<_> = (0..100).map(|offset| {
                let n = batch * 100 + offset;
                let body = format!("中文阅读 common body {} {}", "ordinary text ".repeat(80),
                    if n == 5000 { "uniqueneedle 稀有词" } else { "" });
                Entry {
                    stable_id: format!("id:{n}"), source_id: format!("id:{n}"),
                    id_origin: IdOrigin::SourceData, title: format!("新闻 Article {n}"),
                    url: None, author: None, published: None, updated: None,
                    summary: Some("Cached summary".into()), content_html: Some(format!("<p>{body}</p>")),
                    content_text: Some(body), thumbnail_url: None, categories: vec![],
                }
            }).collect();
            store.upsert_entries(feed, &rows)?;
        }
        store.set_setting("refresh.interval_minutes", "off")?;
        store.set_setting("refresh.on_start", "false")?;
        store.checkpoint_wal()?;
        println!("{}", serde_json::json!({"entries": store.entry_count()?}));
        return Ok(());
    }
    let mut timings = Vec::new();
    let mut ids = Vec::new();
    for run in 0..6 {
        let start = Instant::now();
        let rows = store.search(mode, 200)?;
        timings.push(start.elapsed().as_secs_f64() * 1000.);
        let current: Vec<_> = rows.iter().map(|row| row.id).collect();
        if run == 0 { ids = current; } else { assert_eq!(ids, current); }
        assert!(rows.iter().all(|row| row.content_html.is_none() && row.content_text.is_none()));
    }
    let expected = match mode.as_str() {
        "uniqueneedle" | "稀有词" | "稀" => 1,
        "absenttoken" | "龘" => 0,
        "common" | "中文" | "文" | "新闻" => 200,
        _ => return Err("use a documented fixture query".into()),
    };
    assert_eq!(ids.len(), expected);
    println!("{}", serde_json::json!({"query":mode,"rows":ids.len(),"open_ms":open_ms,
        "first_ms":timings[0],"warm_ms":&timings[1..]}));
    Ok(())
}
