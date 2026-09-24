use rustrss_core::{store::schema::MIGRATIONS, Store};

fn old_database(path: &std::path::Path) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(path).unwrap();
    for migration in &MIGRATIONS[..12] { db.execute_batch(migration).unwrap(); }
    db.execute_batch(r#"PRAGMA user_version=12;
        INSERT INTO feeds(id,url,title,created_at) VALUES(1,'https://example.invalid','Fixture',1);
        INSERT INTO entries(id,feed_id,stable_id,id_origin,title,url,content_html,content_text,search_tokens,read,starred,fetched_at)
        VALUES(1,1,'one','source_data','标题','https://example.invalid/article/1','<p><img src="../cover.jpg"></p>','稀有词','标题 稀有 有词',1,1,10),
              (2,1,'two','source_data','标题','https://example.invalid/article/2',NULL,'其它正文','标题 其它 它正 正文 稀有 有词',0,0,20),
              (3,1,'three','source_data','稀有标题','https://example.invalid/article/3',NULL,'正文','稀有 有标 标题 正文',0,0,30);"#).unwrap();
    db
}

#[test]
fn upgrade_indexes_existing_characters_preserves_fields_and_time_order() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("fixture.sqlite");
    drop(old_database(&path));
    let store=Store::open(&path).unwrap();
    assert_eq!(store.search("稀",200).unwrap().iter().map(|r|r.id).collect::<Vec<_>>(),vec![3,1]);
    let entry=store.get_entry(1).unwrap().unwrap();assert!(entry.read && entry.starred);
    assert_eq!(entry.content_text.as_deref(),Some("稀有词"));
    assert_eq!(entry.thumbnail_url.as_deref(),Some("https://example.invalid/cover.jpg"));
    let db=rusqlite::Connection::open(&path).unwrap();
    let tokens:String=db.query_row("SELECT search_tokens FROM entries WHERE id=1",[],|r|r.get(0)).unwrap();
    assert!(tokens.split_whitespace().any(|s|s=="稀"));
    // Negative control: removing the unigram from real indexed data must make
    // the candidate-index requirement observable, not silently use a full scan.
    db.execute("UPDATE entries SET search_tokens='标题 稀有 有词' WHERE id=1",[]).unwrap();
    assert_eq!(store.search("稀",200).unwrap().len(),1);
    db.execute("UPDATE entries SET search_tokens=? WHERE id=1",[&tokens]).unwrap();
    store.set_bool_setting("list.hide_read",true).unwrap();
    assert_eq!(store.search("稀",200).unwrap().len(),1);
    drop(store);let reopened=Store::open(&path).unwrap();
    assert_eq!(reopened.schema_version().unwrap(),MIGRATIONS.len() as i64);
    assert_eq!(db.query_row("SELECT search_tokens FROM entries WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),tokens);
}

#[test]
fn interrupted_upgrade_rolls_back_tokens_and_version_then_retries() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("fixture.sqlite");
    let db=old_database(&path);
    db.execute_batch("CREATE TRIGGER fail_upgrade BEFORE UPDATE OF search_tokens ON entries WHEN old.id=2 BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(db.query_row("PRAGMA user_version",[],|r|r.get::<_,i64>(0)).unwrap(),12);
    assert_eq!(db.query_row("SELECT search_tokens FROM entries WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"标题 稀有 有词");
    assert_eq!(db.query_row("SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name='thumbnail_url'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    db.execute_batch("DROP TRIGGER fail_upgrade").unwrap();
    assert_eq!(Store::open(&path).unwrap().search("稀",200).unwrap().len(),2);
}
