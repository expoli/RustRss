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
fn nullable_cooldown_column_exists_and_feeds_survive_reopen() {
    // 压平前这条叫 migration_adds_nullable_cooldown_...：那时靠 v12 夹具验证「新增列」。
    // 现在列一开始就在基线里：直接断言它存在且可为空（NULL = 无冷却），并验证行不丢。
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("baseline.sqlite");
    drop(Store::open(&path).unwrap());
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("INSERT INTO feeds (url, title, created_at) VALUES ('https://example.invalid/feed', 'Preserved', 1)", []).unwrap();
        let columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('feeds') WHERE name='retry_after_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(columns, 1, "基线必须带 retry_after_at 列");
        let default: Option<String> = conn
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('feeds') WHERE name='retry_after_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(default, None, "该列可空：NULL 表示无冷却");
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.schema_version().unwrap() as usize,
        rustrss_core::store::schema::MIGRATIONS.len()
    );
    assert_eq!(store.list_feeds().unwrap()[0].title, "Preserved");
    assert_eq!(store.feed_retry_after(1).unwrap(), None);
}
