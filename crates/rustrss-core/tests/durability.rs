//! File-backed upgrade and abrupt process termination, never a user database.
use rustrss_core::{EntryQuery, Store};
use std::time::{Duration, Instant};

#[test]
fn every_schema_version_preserves_rows_and_flags_on_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let migrations = rustrss_core::store::schema::MIGRATIONS;
    for version in 1..migrations.len() {
        let path = dir.path().join(format!("v{version}.sqlite"));
        let conn = rusqlite::Connection::open(&path).unwrap();
        for migration in &migrations[..version] {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", version as i64)
            .unwrap();
        conn.execute_batch("INSERT INTO feeds(id,url,title,created_at,last_status,last_fetched_at,etag) VALUES(1,'https://fixture.invalid/rss','Preserved',1,'ok',1234,'etag');
          INSERT INTO entries(id,feed_id,stable_id,id_origin,title,content_text,read,starred,fetched_at) VALUES(1,1,'sid:fixture','source_data','Article','Cached body',1,1,1234);").unwrap();
        drop(conn);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, migrations.len());
        assert_eq!(store.entry_count().unwrap(), 1, "v{version}");
        let entry = store.get_entry(1).unwrap().unwrap();
        assert!(entry.read && entry.starred, "v{version}");
        assert_eq!(entry.content_text.as_deref(), Some("Cached body"));
        let feed = store.feed_row(1).unwrap().unwrap();
        assert_eq!(feed.title, "Preserved");
        assert_eq!(feed.last_fetched_at, Some(1234));
        assert_eq!(store.cache_headers(1).unwrap().0.as_deref(), Some("etag"));
    }
}

#[test]
fn crash_writer() {
    let Ok(path) = std::env::var("RUSTRSS_CRASH_FIXTURE") else {
        return;
    };
    let store = Store::open(&path).unwrap();
    let id = store
        .add_feed("https://fixture.invalid/rss", Some("Crash fixture"))
        .unwrap();
    let feed = rustrss_core::parse(b"<rss version='2.0'><channel><title>Fixture</title><item><guid>one</guid><title>Article</title><description>Cached body</description></item></channel></rss>").unwrap();
    store.upsert_entries(id, &feed.entries).unwrap();
    let entry = store.list_entries(&EntryQuery::default()).unwrap()[0].id;
    store.set_read(&[entry], true).unwrap();
    store.set_starred(&[entry], true).unwrap();
    store
        .record_fetch(id, "ok", None, Some("etag"), None)
        .unwrap();
    let timestamp = store
        .feed_row(id)
        .unwrap()
        .unwrap()
        .last_fetched_at
        .unwrap();
    std::fs::write(format!("{path}.ready"), timestamp.to_string()).unwrap();
    // Parent kills this process while its Store/SQLite connection is still open.
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn committed_wal_survives_forced_process_termination() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crash.sqlite");
    let ready = dir.path().join("crash.sqlite.ready");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_writer"])
        .env("RUSTRSS_CRASH_FIXTURE", &path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() && Instant::now() < deadline && child.try_wait().unwrap().is_none() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let was_ready = ready.exists();
    let wal_exists = dir
        .path()
        .join("crash.sqlite-wal")
        .metadata()
        .is_ok_and(|m| m.len() > 0);
    let _ = child.kill();
    child.wait().unwrap();
    assert!(
        was_ready && wal_exists,
        "child must commit into a live WAL before termination"
    );
    let timestamp: i64 = std::fs::read_to_string(ready).unwrap().parse().unwrap();
    let store = Store::open(path).unwrap();
    assert_eq!(store.list_feeds().unwrap().len(), 1);
    assert_eq!(store.entry_count().unwrap(), 1);
    let row = store.get_entry(1).unwrap().unwrap();
    assert!(row.read && row.starred);
    assert_eq!(row.content_text.as_deref(), Some("Cached body"));
    let feed = store.feed_row(1).unwrap().unwrap();
    assert_eq!(feed.last_fetched_at, Some(timestamp));
    assert_eq!(feed.last_status.as_deref(), Some("ok"));
    assert_eq!(store.cache_headers(1).unwrap().0.as_deref(), Some("etag"));
}
