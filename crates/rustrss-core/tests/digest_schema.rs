//! 每日日报的 schema 基础（阶段 0）：v1→v2 迁移、窄投影覆盖索引、备份跨版本。
//!
//! 设计见 `.chorus/specs/rss-reader/2026-10-03-daily-digest/design.md` §5/§9/§10。

use rustrss_core::store::schema::MIGRATIONS;
use rustrss_core::Store;
use rusqlite::params;

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

    fn plan_is_covering(conn: &rusqlite::Connection, sql: &str) -> bool {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    let details: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    // 两条覆盖索引（day 先导 / feed 先导）都是设计内路径；
    // 必须 SEARCH（范围定位），整条索引 SCAN 不可接受。
    details
        .iter()
        .any(|d| d.contains("SEARCH") && d.contains("USING COVERING INDEX idx_digest_meta_"))
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

/// 阴性对照：无过滤的 count 必然产生 `SCAN … USING COVERING INDEX`——
/// plan_is_covering 必须拒绝它（只认 SEARCH），否则弱断言假绿无法被证伪
///（独立审核要求的反例证据）。
#[test]
fn full_index_scan_is_not_accepted_as_covering_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    v1_database(&path);
    let _store = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();

    // 只取 effective_at：无过滤时 planner 全扫 idx_digest_meta_day（该列是其首列）。
    let scan_sql = "SELECT effective_at FROM digest_entry_meta";
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {scan_sql}"))
        .unwrap();
    let details: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    // 反例前提成立：这个 plan 确实是「SCAN + 覆盖索引」的组合。
    assert!(
        details.iter().any(|d| {
            d.contains("SCAN") && d.contains("USING COVERING INDEX idx_digest_meta_")
        }),
        "反例前提：无过滤查询应为 SCAN+覆盖索引，实际: {details:?}"
    );
    // 被测断言必须拒绝它。
    assert!(
        !plan_is_covering(&conn, scan_sql),
        "SCAN+覆盖索引不得通过 SEARCH 断言"
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

/// 投影维护（审核 P1-1）：新增/内容更新/全文抓取三条写路径都要推进窄投影。
#[test]
fn digest_meta_is_maintained_across_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    let store = Store::open(&path).unwrap();
    let feed_id = store.add_feed("https://fixture.invalid/rss", Some("F")).unwrap();

    let entry = || rustrss_core::parse(
        b"<rss version='2.0'><channel><title>T</title><item><guid>g1</guid><title>N1</title><description>D1</description></item></channel></rss>",
    ).unwrap().entries;
    store.upsert_entries(feed_id, &entry()).unwrap();

    let conn = rusqlite::Connection::open(&path).unwrap();
    let row = |sql: &str| -> (i64, i64, i64) {
        conn.query_row(sql, [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        }).unwrap()
    };
    // 新增：first_seen 准确（estimated=0）、revision=1
    let (est, rev, _) = row(
        "SELECT first_seen_estimated, source_revision, first_seen_at FROM digest_entry_meta");
    assert_eq!((est, rev), (0, 1), "新条目投影应为首见准确 + 版本 1");

    // 内容更新（fingerprint 变化 → 走 update 分支）：版本 +1
    let changed = rustrss_core::parse(
        b"<rss version='2.0'><channel><title>T</title><item><guid>g1</guid><title>N1 v2</title><description>D1 v2</description></item></channel></rss>",
    ).unwrap().entries;
    store.upsert_entries(feed_id, &changed).unwrap();
    let (_, rev2, _) = row(
        "SELECT first_seen_estimated, source_revision, first_seen_at FROM digest_entry_meta");
    assert_eq!(rev2, 2, "内容更新应推进版本");

    // 全文抓取：真实内容变化，版本再 +1
    store.set_fulltext(1, "<p>full</p>", "full text").unwrap();
    let (_, rev3, _) = row(
        "SELECT first_seen_estimated, source_revision, first_seen_at FROM digest_entry_meta");
    assert_eq!(rev3, 3, "全文抓取应推进版本");
}

/// 报告身份绑定（审核 P1-3）：头/正文/素材必须来自同一 digest；未完成槽位
/// 不进 digest_days（审核 P2-4）。
#[test]
fn digest_report_binds_one_identity_and_days_exclude_slots() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    let store = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let day = "2026-10-03";
    conn.execute_batch(
        r#"INSERT INTO digests(report_day, timezone_label, day_start_at, day_end_at,
             utc_offset_start, utc_offset_end, scope_key, scope_json, profile_key,
             profile_json, revision, created_at)
           VALUES('2026-10-03','Asia/Shanghai',1000,2000,28800,28800,
             'all','{}','p','{}', 0, 10);"#,
    )
    .unwrap();
    let slot: i64 = conn.last_insert_rowid();
    // 槽位（revision=0，无 generated_at）不应出现在 digest_days。
    assert!(
        store.digest_days().unwrap().is_empty(),
        "未完成槽位不应进入历史日期"
    );
    assert!(store.digest_report(day, "all").unwrap().is_none());

    // 完成报告：revision 推进到 1，写 body + 两行 items。
    conn.execute(
        "UPDATE digests SET revision=1, generated_at=20, checkpoint_at=15,
            manifest_hash='m1', article_count=2, created_at=20 WHERE id=?1",
        params![slot],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO digest_bodies(digest_id, revision, schema_ver, content_json, markdown)
         VALUES(?1, 1, 1, '{}', '# 日报正文 v1')",
        params![slot],
    )
    .unwrap();
    for (instance, title) in [(1i64, "第一篇"), (2i64, "第二篇")] {
        conn.execute(
            "INSERT INTO digest_items(digest_id, instance_id, entry_id, feed_id,
                 source_revision, effective_at, input_hash, truncated, summary_only, title)
             VALUES(?1, ?2, ?2, 1, 1, 1500, 'h', 0, 0, ?3)",
            rusqlite::params![slot, instance, title],
        )
        .unwrap();
    }

    let report = store.digest_report(day, "all").unwrap().expect("应读到完成报告");
    assert_eq!(report.markdown, "# 日报正文 v1");
    assert_eq!(report.items.len(), 2);
    assert_eq!(report.checkpoint_at, 15);
    assert_eq!(report.generated_at, 20);
    // 再开一次库重读：同一身份（防并发替换混配的回归）
    let store2 = Store::open(&path).unwrap();
    let again = store2.digest_report(day, "all").unwrap().unwrap();
    assert_eq!(again.items.len(), 2);
    assert_eq!(again.markdown, "# 日报正文 v1");
}

/// 回填边界：published_at 为 NULL 的存量文章，effective_at 用 fetched_at。
#[test]
fn backfill_handles_null_published_at() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(MIGRATIONS[0]).unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        db.execute_batch(
            r#"INSERT INTO feeds(id,url,title,created_at) VALUES(1,'https://x.invalid','F',1);
               INSERT INTO entries(id,feed_id,stable_id,id_origin,title,url,fetched_at)
               VALUES(1,1,'n','s','无发布时间','https://x.invalid/a',7777);"#,
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let (effective, estimated): (i64, i64) = conn
        .query_row(
            "SELECT effective_at, first_seen_estimated FROM digest_entry_meta WHERE entry_id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((effective, estimated), (7777, 1));
    drop(store);
    // 迁移后重开（幂等）
    let store2 = Store::open(&path).unwrap();
    assert_eq!(store2.schema_version().unwrap(), 2);
}
