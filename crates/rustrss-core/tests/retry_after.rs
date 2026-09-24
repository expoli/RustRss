use rustrss_core::{
    fetch::{self, Fetcher},
    Store,
};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn cooldown_survives_reopen_blocks_requests_and_recovers() {
    let server = MockServer::start().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retry.sqlite");
    let store = Store::open(&path).unwrap();
    let id = store.add_feed(&server.uri(), None).unwrap();
    let client = Fetcher::new("retry-test").unwrap();
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "120"))
        .expect(1)
        .mount(&server)
        .await;
    let first = fetch::refresh(&store, &client, &[id], 1).await.unwrap();
    assert_eq!(first.failures[0].code, "http_429");
    let deadline = store.feed_retry_after(id).unwrap().unwrap();
    assert!(deadline >= chrono::Utc::now().timestamp() + 118);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.feed_retry_after(id).unwrap(), Some(deadline));
    let second = fetch::refresh(&store, &client, &[id], 1).await.unwrap();
    assert_eq!(second.failures[0].code, "retry_deferred");
    assert_eq!(
        store.feed_row(id).unwrap().unwrap().last_status.as_deref(),
        Some("http_429")
    );
    server.verify().await;
    server.reset().await;
    store
        .set_feed_retry_after(id, Some(chrono::Utc::now().timestamp() - 1))
        .unwrap();
    Mock::given(wiremock::matchers::method("GET"))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&server)
        .await;
    let third = fetch::refresh(&store, &client, &[id], 1).await.unwrap();
    assert_eq!(third.not_modified, 1);
    assert_eq!(store.feed_retry_after(id).unwrap(), None);
}

#[test]
fn migration_adds_nullable_cooldown_without_losing_feeds() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v12.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    for migration in &rustrss_core::store::schema::MIGRATIONS[..12] {
        conn.execute_batch(migration).unwrap();
    }
    conn.pragma_update(None, "user_version", 12).unwrap();
    conn.execute("INSERT INTO feeds (url, title, created_at) VALUES ('https://example.invalid/feed', 'Preserved', 1)", []).unwrap();
    drop(conn);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap() as usize, rustrss_core::store::schema::MIGRATIONS.len());
    assert_eq!(store.list_feeds().unwrap()[0].title, "Preserved");
    assert_eq!(store.feed_retry_after(1).unwrap(), None);
}
