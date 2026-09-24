//! Time list queries separately from opening the store; use an isolated fixture.
use rustrss_core::{EntryQuery, Store};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = args.get(1).ok_or("usage: scope_entries PATH FOLDER_ID")?;
    let folder = args.get(2).ok_or("missing folder id")?.parse()?;
    let begin = Instant::now();
    let store = Store::open(path)?;
    let open_ms = begin.elapsed().as_secs_f64() * 1000.;
    let query = EntryQuery { folder_id: Some(folder), limit: Some(200), ..Default::default() };
    let begin = Instant::now();
    let first = store.list_entries(&query)?;
    let first_ms = begin.elapsed().as_secs_f64() * 1000.;
    let expected: Vec<_> = first.iter().map(|row| row.id).collect();
    let begin = Instant::now();
    for _ in 0..100 {
        let rows = store.list_entries(&query)?;
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), expected);
    }
    println!("{}", serde_json::json!({"folder": folder, "rows": first.len(),
        "open_ms": open_ms, "first_ms": first_ms,
        "warm_mean_ms": begin.elapsed().as_secs_f64() * 10.}));
    Ok(())
}
