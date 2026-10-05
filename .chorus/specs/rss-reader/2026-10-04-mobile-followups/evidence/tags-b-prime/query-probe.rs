//! Verification-only probe; run against an owned fixture, never a live user database.
use rustrss_core::{EntryQuery, ListSort, Store};
use std::time::Instant;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let path = &args[1];
    assert!(path.contains("target/verification-followups/"));
    let mode = &args[2];
    let start = Instant::now();
    let store = Store::open(path).unwrap();
    let open_ms = start.elapsed().as_secs_f64() * 1000.;
    let query = || {
        if mode == "count" {
            assert_eq!(store.tag_entry_count(1).unwrap(), 6000);
        } else if mode == "sidebar" {
            assert_eq!(store.tag_row(1).unwrap().unwrap().unread, 6000);
        } else {
            let sort = if mode.starts_with("oldest") { ListSort::Oldest }
                else if mode.starts_with("unread_first") { ListSort::UnreadFirst }
                else { ListSort::Newest };
            let q = EntryQuery { tag_id: Some(1), sort: Some(sort), hide_read: Some(false), limit: Some(200),
                cursor: mode.ends_with("next").then(|| (args[3].parse().unwrap(), args[4].parse().unwrap())),
                cursor_read: Some(false), ..Default::default() };
            assert_eq!(store.list_entries(&q).unwrap().len(), 200);
        }
    };
    let start = Instant::now();
    query();
    let first_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    for _ in 0..10 { query(); }
    println!("{{\"mode\":\"{mode}\",\"open_ms\":{open_ms},\"first_ms\":{first_ms},\"warm_mean_ms\":{}}}", start.elapsed().as_secs_f64() * 100.);
}
