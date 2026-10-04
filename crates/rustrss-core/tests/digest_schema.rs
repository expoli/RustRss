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

    // 多变体身份绑定：同日期+范围的旧变体（更早 generated_at）不得混入
    // 头/正文/素材。插入旧变体（profile 'old'，generated_at 更早，items 不同）。
    conn.execute(
        "INSERT INTO digests(report_day, timezone_label, day_start_at, day_end_at,
             utc_offset_start, utc_offset_end, scope_key, scope_json, profile_key,
             profile_json, revision, generated_at, checkpoint_at, manifest_hash,
             article_count, created_at)
         VALUES('2026-10-03','Asia/Shanghai',1000,2000,28800,28800,
             'all','{}','old','{}',1,10,9,'old-m',1,10)",
        [],
    ).unwrap();
    let old_id: i64 = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO digest_bodies(digest_id, revision, schema_ver, content_json, markdown)
         VALUES(?1, 1, 1, '{}', '# 旧变体')",
        params![old_id],
    ).unwrap();
    conn.execute(
        "INSERT INTO digest_items(digest_id, instance_id, entry_id, feed_id,
             source_revision, effective_at, input_hash, truncated, summary_only, title)
         VALUES(?1, 99, 999, 1, 1, 1500, 'old', 0, 0, '旧素材')",
        params![old_id],
    ).unwrap();

    let latest = store.digest_report(day, "all").unwrap().unwrap();
    assert_eq!(latest.markdown, "# 日报正文 v1", "必须取最近完成的变体");
    assert!(latest.items.iter().all(|i| i.title != "旧素材"), "不得混入旧变体素材");

    // 状态检查的 checkpoint 是真检查点（15），不是报告行号（两报告 id 分别 ≥1）
    let status = store.digest_status(1000, 2000, &rustrss_core::store::digest::DigestScope::resolve(&store, &[]).unwrap()).unwrap();
    assert!(status.has_report);
    assert_eq!(status.checkpoint_at, 15, "checkpoint 应来自字段而非行号");
    // checkpoint=15 而 digests.id ∈ {1..}：若实现退回行号（≠15）此断言即红。
    assert_ne!(status.checkpoint_at, 0);
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

/// 日期归属不可漂移（审核 P1 回归）：无 published_at 的条目，首见在第 1 天、
/// 内容在第 2 天变化——投影 effective_at 必须仍锚定首见（第 1 天），不被新
/// fetched_at 搬到第 2 天。
#[test]
fn effective_at_does_not_drift_for_null_published_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    let store = Store::open(&path).unwrap();
    let feed_id = store.add_feed("https://fixture.invalid/rss", Some("F")).unwrap();

    // 第 1 天：首次入库（fetched_at=1000，无发布时间）
    let day1 = rustrss_core::parse(
        b"<rss version='2.0'><channel><title>T</title><item><guid>d</guid><title>N</title><description>D</description></item></channel></rss>",
    ).unwrap().entries;
    store.upsert_entries(feed_id, &day1).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let effective: i64 = conn
        .query_row("SELECT effective_at FROM digest_entry_meta", [], |r| r.get(0))
        .unwrap();

    // 第 2 天：内容变化（fetched_at 推进 86400）
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let day2 = rustrss_core::parse(
        b"<rss version='2.0'><channel><title>T</title><item><guid>d</guid><title>N v2</title><description>D v2</description></item></channel></rss>",
    ).unwrap().entries;
    store.upsert_entries(feed_id, &day2).unwrap();

    let (effective2, revision): (i64, i64) = conn
        .query_row(
            "SELECT effective_at, source_revision FROM digest_entry_meta",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(effective2, effective, "日期归属不得漂移到新的 fetched_at");
    assert_eq!(revision, 2, "内容变化应推进版本");
}

/// 阶段 0 之后插入的条目若缺投影行（模拟修复前的 v2 库），打开时幂等补齐。
#[test]
fn missing_meta_rows_are_backfilled_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    {
        let store = Store::open(&path).unwrap();
        let feed_id = store.add_feed("https://fixture.invalid/rss", Some("F")).unwrap();
        let entries = rustrss_core::parse(
            b"<rss version='2.0'><channel><title>T</title><item><guid>m</guid><title>M</title><description>D</description></item></channel></rss>",
        ).unwrap().entries;
        store.upsert_entries(feed_id, &entries).unwrap();
        // 模拟修复前的 v2 库：投影行缺失
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("DELETE FROM digest_entry_meta", []).unwrap();
    }
    let store = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let (estimated, count): (i64, i64) = conn
        .query_row(
            "SELECT first_seen_estimated, count(*) FROM digest_entry_meta",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, estimated), (1, 1), "缺失投影应在打开时补齐（近似标记）");
    assert_eq!(store.schema_version().unwrap(), 2);
}

/// 订阅源打标（DAO）+ 范围 OR 解析（设计 §5.1/§6，审核 P1-1 关联）：
/// replace-all 语义、OR 匹配、空选择 = 全部。
#[test]
fn feed_tags_roundtrip_and_scope_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    let store = Store::open(&path).unwrap();
    let f1 = store.add_feed("https://a.invalid/rss", Some("A")).unwrap();
    let f2 = store.add_feed("https://b.invalid/rss", Some("B")).unwrap();
    let t1 = store.create_tag("工具", None).unwrap();
    let t2 = store.create_tag("AI", None).unwrap();

    // replace-all：先挂两个，再收敛为一个（整替语义）
    store.set_feed_tags(f1, &[t1.id, t2.id]).unwrap();
    store.set_feed_tags(f1, &[t2.id]).unwrap();
    let f1_tags = store.feed_tags(f1).unwrap();
    assert_eq!(f1_tags.len(), 1);
    assert_eq!(f1_tags[0].id, t2.id);
    store.set_feed_tags(f2, &[t1.id]).unwrap();

    // OR 解析：t2 → {f1}；双标签 → {f1,f2}；空 → 全部（feed_ids=None）
    let scope_t2 = rustrss_core::store::digest::DigestScope::resolve(&store, &[t2.id]).unwrap();
    assert_eq!(scope_t2.feed_ids, Some(vec![f1]));
    let scope_both =
        rustrss_core::store::digest::DigestScope::resolve(&store, &[t1.id, t2.id]).unwrap();
    let mut got = scope_both.feed_ids.clone().unwrap();
    got.sort_unstable();
    assert_eq!(got, vec![f1, f2]);
    let scope_all = rustrss_core::store::digest::DigestScope::resolve(&store, &[]).unwrap();
    assert_eq!(scope_all.feed_ids, None);
    assert_eq!(scope_all.key, "all");
    // scope_key 规范化：顺序无关
    let scope_rev = rustrss_core::store::digest::DigestScope::resolve(&store, &[t2.id, t1.id]).unwrap();
    assert_eq!(scope_rev.key, scope_both.key);
}

/// 冻结清单（阶段 2 核心）：窗口/范围过滤、输入哈希、预算截断（阶段 2）。
#[test]
fn freeze_manifest_filters_and_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    let store = Store::open(&path).unwrap();
    let f1 = store.add_feed("https://a.invalid/rss", Some("A")).unwrap();
    let f2 = store.add_feed("https://b.invalid/rss", Some("B")).unwrap();
    let t = store.create_tag("工具", None).unwrap();
    store.set_feed_tags(f1, &[t.id]).unwrap();
    let mk = |guid: &str, title: &str| {
        rustrss_core::parse(
            format!(
                "<rss version='2.0'><channel><title>T</title><item><guid>{guid}</guid><title>{title}</title><description>正文 {guid}</description></item></channel></rss>"
            ).as_bytes(),
        ).unwrap().entries
    };
    store.upsert_entries(f1, &mk("a1", "A1")).unwrap();
    store.upsert_entries(f1, &mk("a2", "A2")).unwrap();
    store.upsert_entries(f2, &mk("b1", "B1")).unwrap();

    // 时区无关：条目刚抓取（fetched_at = now），用以 now 为中心的开窗
    let now = chrono::Utc::now().timestamp();
    let (start, end) = (now - 3600, now + 3600);
    let manifest = store
        .freeze_manifest(start, end, None)
        .unwrap();
    // 全部范围：三个条目都在窗口内（刚抓取）
    assert_eq!(manifest.entries.len(), 3);
    assert!(!manifest.truncated);
    // 每篇有独立输入哈希；同标题同正文才会同哈希
    let hashes: Vec<_> = manifest.entries.iter().map(|e| e.input_hash.as_str()).collect();
    assert_eq!(hashes.len(), 3);
    // 标签范围（工具 → 只有关联了该标签的 f1）；feed 集经 DigestScope::resolve
    let scope = rustrss_core::store::digest::DigestScope::resolve(&store, &[t.id]).unwrap();
    let scoped = store.freeze_manifest(start, end, scope.feed_ids.as_deref()).unwrap();
    assert_eq!(scoped.entries.len(), 2, "OR 匹配只纳入 f1 的条目");
    assert!(scoped.entries.iter().all(|e| e.feed_id == f1));
    // 输入哈希稳定性：同内容重冻结哈希不变（缓存键稳定的前提）
    let again = store.freeze_manifest(start, end, None).unwrap();
    assert_eq!(
        again.entries.iter().map(|e| e.input_hash.as_str()).collect::<Vec<_>>(),
        hashes
    );
}

/// 预算截断：超过 MANIFEST_MAX_ENTRIES 时取最近 N 篇并标记 truncated。
#[test]
fn freeze_manifest_truncates_to_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    let store = Store::open(&path).unwrap();
    let f1 = store.add_feed("https://a.invalid/rss", Some("A")).unwrap();
    // 造 250 篇（> 上界 200），时间递增：冻结应取最近 200 篇并标记截断
    for i in 0..250 {
        // 小时取 00-13（UTC）：+8 后全部落在本地 10-04 当日窗口内
        let hh = format!("{:02}", (i % 14));
        let entries = rustrss_core::parse(
            format!(
                "<rss version='2.0'><channel><title>T</title><item><guid>g{i}</guid><title>N{i}</title><description>D{i}</description><pubDate>2026-10-04T{hh}:00:00Z</pubDate></item></channel></rss>"
            ).as_bytes(),
        ).unwrap().entries;
        store.upsert_entries(f1, &entries).unwrap();
    }
    // 时区无关：直接用已知 pubDate 的 epoch 窗口（00:00Z–14:00Z），不依赖本机时区
    let start = 1791072000; // 2026-10-04T00:00:00Z
    let end = 1791122400; // 2026-10-04T14:00:00Z
    let manifest = store.freeze_manifest(start, end, None).unwrap();
    assert!(manifest.truncated, "250 篇 > 上界 200 应标记截断");
    assert_eq!(manifest.entries.len(), 200);
    // 确定性断言：从已知 pubDate 模式独立推导期望保留集（不依赖 manifest 自身）。
    // 条目 g{i} 的 pubDate = 2026-10-04T{hh}:00Z，hh = i%14 → effective_at 已知；
    // instance_id 按插入顺序 = i+1。effective_at desc, instance_id desc 排序后
    // 取前 200 = 丢弃 desc 序前 50。
    let base = 1791072000i64; // 2026-10-04T00:00:00Z
    let mut all: Vec<(i64, i64)> = (0..250i64)
        .map(|i| (i + 1, base + (i % 14) * 3600))
        .collect();
    all.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
    let expected: std::collections::BTreeSet<i64> =
        all.into_iter().take(200).map(|(id, _)| id).collect();
    let kept: std::collections::BTreeSet<i64> =
        manifest.entries.iter().map(|e| e.instance_id).collect();
    assert_eq!(kept, expected, "保留集必须是 effective_at 最近的 200 篇");
    // pairs_hash 覆盖全量 250 篇（截断不丢 CAS 指纹的成员）
    assert_eq!(manifest.total_in_window, 250);
    assert!(!manifest.pairs_hash.is_empty());

    // 截断清单的无漂移提交应成功（200 行 items + 全量对指纹）
    store
        .commit_digest_report(
            "2026-10-04", "UTC", start, end, "all", "{}", "p", "{}",
            &manifest.hash, &manifest.pairs_hash, &[],
            manifest.frozen_at, 0, 0,
            "{}", "# 日报", 1, "{}", manifest.entries.len() as i64, &manifest.entries,
        )
        .unwrap();
}

/// 提交 CAS：生成期间素材变化（成员增删/版本变更/源标签重打）必须拒绝提交。
#[test]
fn commit_cas_rejects_when_material_drifts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustrss.sqlite");
    let store = Store::open(&path).unwrap();
    let f1 = store.add_feed("https://a.invalid/rss", Some("A")).unwrap();
    let t = store.create_tag("工具", None).unwrap();
    store.set_feed_tags(f1, &[t.id]).unwrap();
    let mk = |guid: &str, title: &str, hh: u8| {
        rustrss_core::parse(
            format!(
                "<rss version='2.0'><channel><title>T</title><item><guid>{guid}</guid><title>{title}</title><description>正文 {guid}</description><pubDate>2026-10-04T0{hh}:00:00Z</pubDate></item></channel></rss>"
            )
            .as_bytes(),
        )
        .unwrap()
        .entries
    };
    store.upsert_entries(f1, &mk("1", "N1", 1)).unwrap();
    store.upsert_entries(f1, &mk("2", "N2", 2)).unwrap();

    // 时区无关窗口
    let start = 1791072000; // 2026-10-04T00:00:00Z
    let end = 1791122400; // 14:00Z
    let manifest = store.freeze_manifest(start, end, None).unwrap();
    let items = manifest.entries.clone();
    let commit = |store: &Store, pairs: &str, tags: &[i64]| {
        store.commit_digest_report(
            "2026-10-04", "UTC", start, end, "all", "{}", "p", "{}",
            &manifest.hash, pairs, tags,
            manifest.frozen_at, 0, 0,
            "{}", "# 日报", 1, "{}", items.len() as i64, &items,
        )
    };
    // 指纹一致 → 提交成功
    commit(&store, &manifest.pairs_hash, &[]).unwrap();

    // 漂移 1：新条目进入窗口
    store.upsert_entries(f1, &mk("3", "N3", 3)).unwrap();
    let after = store.freeze_manifest(start, end, None).unwrap();
    eprintln!("DEBUG after-add: total={} hash_changed={}", after.total_in_window, after.pairs_hash != manifest.pairs_hash);
    let e = commit(&store, &manifest.pairs_hash, &[]).unwrap_err();
    assert!(e.to_string().contains("素材"), "新成员应被 CAS 拒绝：{e}");

    // 漂移 2：成员版本变化（同 guid 更新正文 → source_revision 递增）
    let manifest2 = store.freeze_manifest(start, end, None).unwrap();
    store
        .upsert_entries(f1, &mk("2", "N2-updated-正文已变化", 2))
        .unwrap();
    let manifest2b = store.freeze_manifest(start, end, None).unwrap();
    eprintln!("DEBUG after-update: rev_drift={}", manifest2b.pairs_hash != manifest2.pairs_hash);
    let e = commit(&store, &manifest2.pairs_hash, &[]).unwrap_err();
    assert!(e.to_string().contains("素材"), "版本漂移应被 CAS 拒绝：{e}");

    // 漂移 3：源标签重打改变范围（f1 摘掉标签后，按标签范围的候选集变化）
    let scope3 = rustrss_core::store::digest::DigestScope::resolve(&store, &[t.id]).unwrap();
    let manifest3 = store.freeze_manifest(start, end, scope3.feed_ids.as_deref()).unwrap();
    store.set_feed_tags(f1, &[]).unwrap();
    let e = commit(&store, &manifest3.pairs_hash, &[t.id]).unwrap_err();
    assert!(e.to_string().contains("素材"), "范围漂移应被 CAS 拒绝：{e}");

    // 无漂移的按标签提交仍成功
    let scope4 = rustrss_core::store::digest::DigestScope::resolve(&store, &[t.id]).unwrap();
    let manifest4 = store.freeze_manifest(start, end, scope4.feed_ids.as_deref()).unwrap();
    store
        .commit_digest_report(
            "2026-10-04", "UTC", start, end, "tags", "{}", "p", "{}",
            &manifest4.hash, &manifest4.pairs_hash, &[t.id],
            manifest4.frozen_at, 0, 0,
            "{}", "# 日报", 1, "{}", manifest4.entries.len() as i64, &manifest4.entries,
        )
        .unwrap();
}
