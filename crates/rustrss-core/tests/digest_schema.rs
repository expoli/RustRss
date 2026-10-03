//! 每日日报的 schema 基础（阶段 0）：v1→v2 迁移、窄投影覆盖索引、备份跨版本。
//!
//! 设计见 `.chorus/specs/rss-reader/2026-10-03-daily-digest/design.md` §5/§9/§10。

use rustrss_core::store::schema::MIGRATIONS;
use rustrss_core::Store;

/// 造一个 v1 库（只含基线）：一条 feed + 一条已发布文章。
fn v1_database(path: &std::path::Path) {
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(MIGRATIONS[0]).unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    db.execute_batch(
        r#"INSERT INTO feeds(id,url,title,created_at) VALUES(1,'https://example.invalid/rss','Fixture',10);
           INSERT INTO entries(id,feed_id,stable_id,id_origin,title,url,content_text,published_at,fetched_at,read)
           VALUES(1,1,'one','source_data','昨天的文章','https://example.invalid/a/1','正文',1000,2000,0);"#,
    )
    .unwrap();
}

#[test]
fn v1_database_migrates_to_v2_on_open_and_backfills_meta() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    v1_database(&path);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);

    // 迁移在 Store::open 内完成（不依赖单独调用），新表就位：
    let conn = rusqlite::Connection::open(&path).unwrap();
    for table in [
        "feed_tags",
        "digest_entry_meta",
        "digests",
        "digest_bodies",
        "digest_items",
        "digest_node_cache",
    ] {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "表 {table} 应由迁移创建");
    }

    // 窄投影回填：first_seen 用 fetched_at 近似（estimated=1），effective_at = published_at。
    let (first_seen, estimated, effective): (i64, i64, i64) = conn
        .query_row(
            "SELECT first_seen_at, first_seen_estimated, effective_at FROM digest_entry_meta WHERE entry_id=1",
            [],
            |r| r.get(0).map(|a: i64| (a, r.get::<_, i64>(1).unwrap(), r.get::<_, i64>(2).unwrap())),
        )
        .unwrap();
    assert_eq!((first_seen, estimated, effective), (2000, 1, 1000));

    // 既有数据无损（迁移只追加，不动旧表）。
    let (title, feed_id): (String, i64) = conn
        .query_row("SELECT title, feed_id FROM entries WHERE id=1", [], |r| {
            r.get(0).map(|t: String| (t, r.get::<_, i64>(1).unwrap()))
        })
        .unwrap();
    assert_eq!((title.as_str(), feed_id), ("昨天的文章", 1));
}

#[test]
fn status_check_uses_covering_index_not_a_scan() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    v1_database(&path);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    let conn = rusqlite::Connection::open(&path).unwrap();

    // 日报状态检查的代表性查询（新增计数）：必须走 idx_digest_meta_day 覆盖路径。
    const STATUS_SQL: &str = "SELECT count(*) FROM digest_entry_meta \
         WHERE effective_at >= 0 AND effective_at < 86400 AND feed_id = 1";

    fn plan_is_covering(conn: &rusqlite::Connection, sql: &str) -> bool {
        let mut stmt = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let details: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        // 两条覆盖索引（day 先导 / feed 先导）都是设计内路径；SCAN 才是失败。
        details
            .iter()
            .any(|d| d.contains("USING COVERING INDEX idx_digest_meta_"))
    }

    let dbg: Vec<String> = {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {STATUS_SQL}")).unwrap();
        stmt.query_map([], |r| r.get::<_, String>(3)).unwrap().map(|r| r.unwrap()).collect()
    };
    eprintln!("实际 plan: {dbg:?}");
    assert!(
        plan_is_covering(&conn, STATUS_SQL),
        "状态检查必须走覆盖索引（红线 1/2）"
    );

    // 变异校验：删掉两条覆盖索引后同一查询退化为 SCAN —— 证明断言真的在盯索引。
    conn.execute("DROP INDEX idx_digest_meta_day", []).unwrap();
    conn.execute("DROP INDEX idx_digest_meta_feed_day", []).unwrap();
    assert!(
        !plan_is_covering(&conn, STATUS_SQL),
        "索引删除后断言必须变红（防假绿）"
    );
}

#[test]
fn older_backup_is_accepted_and_migration_completes_after_restore() {
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("v1-backup.sqlite");

    // 用 v1 结构造一份备份（带应用魔数，user_version=1）。
    v1_database(&backup);

    // 当前程序是 v2：旧备份必须**接受**（恢复后打开时追平迁移），而不是被拒。
    rustrss_core::backup::validate_backup(&backup, 2).expect("旧版本备份应被接受并允许迁移");

    // 恢复（暂存）后打开：迁移追平，数据完整。
    rustrss_core::backup::stage_restore(&backup, dir.path()).unwrap();
    let restored = dir.path().join("rustrss.sqlite");
    std::fs::copy(dir.path().join("pending-restore.sqlite"), &restored).unwrap();
    let store = Store::open(&restored).unwrap();
    assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
    let conn = rusqlite::Connection::open(&restored).unwrap();
    let feed_count: i64 = conn
        .query_row("SELECT count(*) FROM feeds WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(feed_count, 1, "恢复的数据在迁移后完整");
}
