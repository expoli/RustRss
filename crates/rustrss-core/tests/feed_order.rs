use rustrss_core::Store;
fn ids(store: &Store) -> Vec<i64> {
    store.list_feeds().unwrap().iter().map(|f| f.id).collect()
}
#[test]
fn move_is_persistent_and_invalid_targets_leave_order_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("order.sqlite");
    let s = Store::open(&path).unwrap();
    let a = s.add_feed("https://example.test/a", Some("A")).unwrap();
    let b = s.add_feed("https://example.test/b", Some("B")).unwrap();
    let c = s.add_feed("https://example.test/c", Some("C")).unwrap();
    assert_eq!(ids(&s), vec![a, b, c]);
    s.move_feed(c, a, true).unwrap();
    assert_eq!(ids(&s), vec![c, a, b]);
    s.move_feed(c, b, false).unwrap();
    assert_eq!(ids(&s), vec![a, b, c]);
    s.move_feed(b, a, true).unwrap();
    assert_eq!(ids(&s), vec![b, a, c]);
    assert!(s.move_feed(c, 999, true).is_err());
    assert_eq!(ids(&s), vec![b, a, c]);
    s.move_feed(a, a, true).unwrap();
    assert_eq!(ids(&s), vec![b, a, c]);
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(ids(&s), vec![b, a, c]);
}

#[test]
fn group_boundary_is_enforced_and_new_feeds_append() {
    let s = Store::open_in_memory().unwrap();
    let a = s.add_feed("https://example.test/a", Some("A")).unwrap();
    let b = s.add_feed("https://example.test/b", Some("B")).unwrap();
    let other = s.add_feed("https://example.test/c", Some("Other")).unwrap();
    let group = s.add_folder("Group").unwrap();
    s.assign_folder(other, Some(group)).unwrap();
    assert!(s.move_feed(a, other, true).is_err());
    assert_eq!(ids(&s), vec![a, b, other]);
    s.move_feed(b, a, true).unwrap();
    let added = s.add_feed("https://example.test/new", Some("AAA")).unwrap();
    let ungrouped: Vec<_> = s
        .list_feeds()
        .unwrap()
        .iter()
        .filter(|f| f.folder_id.is_none())
        .map(|f| f.id)
        .collect();
    assert_eq!(ungrouped, vec![b, a, added]);
    assert_eq!(s.feed_row(other).unwrap().unwrap().folder_id, Some(group));
}

#[test]
fn v13_upgrade_keeps_alphabetical_order_and_cooldown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v13.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    for migration in &rustrss_core::store::schema::MIGRATIONS[..13] {
        conn.execute_batch(migration).unwrap();
    }
    conn.execute_batch("PRAGMA user_version=13; INSERT INTO feeds(id,url,title,created_at,retry_after_at) VALUES(1,'https://example.test/z','Z',1,1234),(2,'https://example.test/a','A',1,NULL);").unwrap();
    drop(conn);
    let s = Store::open(&path).unwrap();
    assert_eq!(ids(&s), vec![2, 1]);
    assert_eq!(s.feed_retry_after(1).unwrap(), Some(1234));
}
