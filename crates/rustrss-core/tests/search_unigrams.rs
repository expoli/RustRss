use rustrss_core::store::backfill::apply_pre_release_backfill;
use rustrss_core::store::schema::{BASELINE_VERSION, MIGRATIONS};
use rustrss_core::Store;

/// 基线库 + 预置行（token 只含 bigram、缩略图列为空）——
/// 相当于压平前需要回填的老数据，用来验证保留件 `store::backfill`。
fn baseline_database(path: &std::path::Path) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(path).unwrap();
    for migration in MIGRATIONS { db.execute_batch(migration).unwrap(); }
    db.pragma_update(None, "user_version", BASELINE_VERSION).unwrap();
    db.execute_batch(r#"
        INSERT INTO feeds(id,url,title,created_at) VALUES(1,'https://example.invalid','Fixture',1);
        INSERT INTO entries(id,feed_id,stable_id,id_origin,title,url,content_html,content_text,search_tokens,read,starred,fetched_at)
        VALUES(1,1,'one','source_data','标题','https://example.invalid/article/1','<p><img src="../cover.jpg"></p>','稀有词','标题 稀有 有词',1,1,10),
              (2,1,'two','source_data','标题','https://example.invalid/article/2',NULL,'其它正文','标题 其它 它正 正文 稀有 有词',0,0,20),
              (3,1,'three','source_data','稀有标题','https://example.invalid/article/3',NULL,'正文','稀有 有标 标题 正文',0,0,30);"#).unwrap();
    db
}

#[test]
fn backfill_indexes_existing_characters_preserves_fields_and_time_order() {
    // 压平前：跑 v13 迁移时**隐式**回填单字与缩略图；现在回填是保留件，显式调用。
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("fixture.sqlite");
    let db=baseline_database(&path);
    assert!(apply_pre_release_backfill(&db).unwrap() >= 1, "回填应改写既有行");
    drop(db);
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
fn interrupted_baseline_creation_rolls_back_then_retries() {
    // 压平前是「迁移中断回滚 + 重试」；现在对应「基线建库中断回滚 + 重试」。
    // 用一个**非表**对象占位：detect 仍判 Fresh（用户表数为 0），但基线建表必失败。
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("fixture.sqlite");
    {
        let db=rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE VIEW feeds AS SELECT 1 AS id;").unwrap();
    }
    assert!(Store::open(&path).is_err(),"基线建库失败必须冒泡，不得当成功");
    let db=rusqlite::Connection::open(&path).unwrap();
    assert_eq!(db.query_row("PRAGMA user_version",[],|r|r.get::<_,i64>(0)).unwrap(),0,"失败必须回滚版本号");
    let partial:i64=db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='folders'",[],|r|r.get(0)).unwrap();
    assert_eq!(partial,0,"失败必须回滚已建的基线对象（不留半成品库）");
    db.execute_batch("DROP VIEW feeds").unwrap();
    let store=Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap() as usize,MIGRATIONS.len(),"清理后可重试成功");
}
