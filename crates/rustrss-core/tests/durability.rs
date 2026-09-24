//! 基线 schema、老库拒绝与强杀持久性的守护测试（一律用文件库，不用用户数据库）。
//!
//! 压平后「逐版本升级」不再是产品行为，逐版本循环会退化成 `1..1` 空区间
//! ——那是**静默空跑**。本文件用下面四条断言替代它，覆盖没有丢：
//! 1. 夹具步数下限（防空跑）；
//! 2. 基线 schema 与旧链终态**结构化等价**（含变异校验）；
//! 3. 旧库被拒绝且**一个字节都没写**；
//! 4. 基线库写入后重开，行与状态位保全。
#[path = "fixtures/legacy_chain.rs"]
mod legacy_chain;

use legacy_chain::LEGACY_CHAIN;
use rusqlite::Connection;
use rustrss_core::store::schema::{self, SchemaState, BASELINE_APPLICATION_ID, BASELINE_VERSION, MIGRATIONS};
use rustrss_core::store::StoreError;
use rustrss_core::{EntryQuery, Store};
use std::time::{Duration, Instant};

/// 旧链步数下限：压平前是 13。夹具被削短会让等价与拒绝断言一起空跑。
const LEGACY_STEPS: usize = 13;

fn run_baseline(conn: &Connection) {
    for (i, migration) in MIGRATIONS.iter().enumerate() {
        conn.execute_batch(migration).unwrap();
        conn.pragma_update(None, "user_version", (i + 1) as i64)
            .unwrap();
    }
}

fn legacy_terminal_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    for migration in LEGACY_CHAIN {
        conn.execute_batch(migration).unwrap();
    }
    conn.pragma_update(None, "user_version", LEGACY_STEPS as i64)
        .unwrap();
    conn
}

fn app_id(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap()
}

/// 结构化 dump：表（列名/类型/非空/默认值/主键序）→ 索引（含部分索引 WHERE 与表达式）
/// → 触发器 → 虚拟表 SQL（FTS 的 `content=` / `content_rowid=` 只在 SQL 里）。
///
/// 只比结构，不比 `sqlite_master` 的原文：`ALTER TABLE ADD COLUMN` 会把新列追加进
/// 原文（旧链终态的 `CREATE TABLE` 文本与新写的基线不同），但结构必须一致。
fn structure_dump(conn: &Connection) -> String {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    let tables: Vec<String> = {
        let mut st = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        let rows = st
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    };
    for t in tables {
        out.push_str(&format!("TABLE {t}\n"));
        {
            let mut st = conn.prepare(&format!("PRAGMA table_info({t})")).unwrap();
            let rows = st
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            for (cid, name, ty, notnull, dflt, pk) in rows {
                out.push_str(&format!(
                    "  COL {cid} {name} type={ty} notnull={notnull} default={dflt:?} pk={pk}\n"
                ));
            }
        }
        {
            // 外键维度：`ON DELETE` 行为（CASCADE / SET NULL）是本仓「零孤儿」不变量的地方，
            // 光比列/索引/触发器看不到它——“少了 ON DELETE CASCADE”也算等价是假绿。
            let mut st = conn
                .prepare(&format!("PRAGMA foreign_key_list({t})"))
                .unwrap();
            let mut rows = st
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            for (id, seq, target, from, to, on_update, on_delete, match_) in rows {
                out.push_str(&format!(
                    "  FK {id}.{seq} -> {target}.{to:?} from={from} on_update={on_update} on_delete={on_delete} match={match_}\n"
                ));
            }
        }
        {
            let mut st = conn.prepare(&format!("PRAGMA index_list({t})")).unwrap();
            let rows = st
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            // `PRAGMA index_list` 的顺序未定义（受建索引顺序影响）→ 按名排序后再比
            let mut rows = rows;
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            for (iname, unique, origin, partial) in rows {
                let sql: Option<String> = conn
                    .query_row(
                        "SELECT sql FROM sqlite_master WHERE type='index' AND name=?",
                        [&iname],
                        |r| r.get(0),
                    )
                    .ok();
                let mut cols: Vec<(i64, String)> = {
                    let mut s2 = conn
                        .prepare(&format!("PRAGMA index_info({iname})"))
                        .unwrap();
                    s2.query_map([], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .unwrap()
                    .map(|x| {
                        let (i, n) = x.unwrap();
                        (i, n.unwrap_or_else(|| "<expr>".into()))
                    })
                    .collect()
                };
                cols.sort();
                let names: Vec<String> = cols.into_iter().map(|(_, c)| c).collect();
                out.push_str(&format!(
                    "  IDX {iname} unique={unique} origin={origin} partial={partial} cols={} sql={}\n",
                    names.join(","),
                    norm(sql.as_deref().unwrap_or("<autoindex>"))
                ));
            }
        }
        let triggers: Vec<(String, Option<String>)> = {
            let mut st = conn
                .prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger' AND tbl_name=? ORDER BY name")
                .unwrap();
            st.query_map([&t], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        for (name, sql) in triggers {
            out.push_str(&format!(
                "  TRG {name} {}\n",
                norm(sql.as_deref().unwrap_or(""))
            ));
        }
        let vtab: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name=?",
                [&t],
                |r| r.get(0),
            )
            .unwrap_or(None);
        if let Some(sql) = vtab {
            if sql.to_ascii_uppercase().contains("VIRTUAL TABLE") {
                out.push_str(&format!("  VIRTUAL {}\n", norm(&sql)));
            }
        }
    }
    out
}

/// 基线对象数（表 / 显式索引 / 触发器 / 外键）。
///
/// 为什么要有这组数字：结构化 dump 是「逐对象比对」，若某一**整类**对象在两边都缺失
/// （例如外键或触发器整体没建），逐对象比对照样相等——那是假绿。数量钉死后，丢整类会
/// 立刻变红。改基线必须同步这里，改动因此必须是有意的。
const BASELINE_COUNTS: (i64, i64, i64, i64) = (12, 11, 3, 5);

fn object_counts(conn: &Connection) -> (i64, i64, i64, i64) {
    let scalar = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    let tables = scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    );
    let indexes = scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND sql IS NOT NULL",
    );
    let triggers = scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger'");
    let tables_list: Vec<String> = {
        let mut st = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
            .unwrap();
        st.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let mut foreign_keys = 0i64;
    for t in tables_list {
        foreign_keys += scalar(&format!("SELECT COUNT(*) FROM pragma_foreign_key_list('{t}')"));
    }
    (tables, indexes, triggers, foreign_keys)
}

#[test]
fn legacy_chain_fixture_is_intact() {
    assert_eq!(
        LEGACY_CHAIN.len(),
        LEGACY_STEPS,
        "旧链夹具步数变了：等价与拒绝断言会随之空跑"
    );
    assert_eq!(MIGRATIONS.len(), 1, "首发基线只应有一条迁移");
    assert_eq!(BASELINE_VERSION, 1);
}

#[test]
fn baseline_schema_is_equivalent_to_legacy_terminal_state() {
    let baseline = Connection::open_in_memory().unwrap();
    run_baseline(&baseline);
    let legacy = legacy_terminal_conn();

    let (a, b) = (structure_dump(&baseline), structure_dump(&legacy));
    if a != b {
        let (al, bl): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
        for i in 0..al.len().max(bl.len()) {
            if al.get(i) != bl.get(i) {
                panic!(
                    "结构化 dump 第 {i} 行不同:\n基线: {:?}\n旧链: {:?}",
                    al.get(i),
                    bl.get(i)
                );
            }
        }
        panic!("结构化 dump 长度不同: {} vs {}", al.len(), bl.len());
    }

    // application_id 不在 sqlite_master 里，单独断言：基线写魔数、旧链从不写
    assert_eq!(app_id(&baseline), BASELINE_APPLICATION_ID as i64);
    assert_eq!(app_id(&legacy), 0, "旧链任何版本都不写 application_id");

    // 对象数下限（按类别钉死）：防「某一整类对象在两边都缺失」的假绿
    assert_eq!(
        object_counts(&baseline),
        BASELINE_COUNTS,
        "基线对象数与钉死的数字不符（改了基线就要同步 BASELINE_COUNTS）"
    );
    assert_eq!(
        object_counts(&legacy),
        BASELINE_COUNTS,
        "旧链终态对象数应与基线一致"
    );

    // 变异校验：**删**与**改**两半都要能变红，且都在独立 scratch 库上做
    // （不与已打开的 Store 并发改 schema）
    let removal = Connection::open_in_memory().unwrap();
    removal
        .execute_batch(&MIGRATIONS.join(";").replace(
            "CREATE INDEX idx_entries_starred ON entries(starred) WHERE starred = 1",
            "-- 变异：删掉星标部分索引",
        ))
        .unwrap();
    assert_ne!(
        structure_dump(&removal),
        structure_dump(&legacy),
        "删掉一个索引后必须不再等价"
    );

    let modification = Connection::open_in_memory().unwrap();
    modification
        .execute_batch(
            &MIGRATIONS
                .join(";")
                .replace("ON DELETE CASCADE", "ON DELETE SET NULL"),
        )
        .unwrap();
    assert_ne!(
        structure_dump(&modification),
        structure_dump(&legacy),
        "把外键的 ON DELETE CASCADE 改成 SET NULL 后必须不再等价（外键维度不能被漏掉）"
    );
    assert_ne!(
        object_counts(&removal),
        object_counts(&legacy),
        "对象数校验也应对「删」敏感（删了一个索引：11 → 10）"
    );
}

#[test]
fn legacy_frozen_db_is_refused_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.sqlite");
    {
        let conn = Connection::open(&path).unwrap();
        for migration in LEGACY_CHAIN {
            conn.execute_batch(migration).unwrap();
        }
        conn.pragma_update(None, "user_version", LEGACY_STEPS as i64)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO feeds(id,url,title,created_at) VALUES(1,'https://fixture.invalid/rss','Frozen',1);",
        )
        .unwrap();
    }
    let before = std::fs::read(&path).unwrap();

    let err = match Store::open(&path) {
        Ok(_) => panic!("旧链冻结库必须被拒绝，实际却成功打开"),
        Err(e) => e,
    };
    assert!(
        matches!(err, StoreError::SchemaRefused { .. }),
        "旧链冻结库必须被拒绝，实际: {err:?}"
    );
    assert_eq!(err.code(), "schema_refused");

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "拒绝路径不得写库（含 journal_mode 等 pragma 写入）"
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        LEGACY_STEPS as i64
    );
    assert_eq!(app_id(&conn), 0);

    // 反向对照：同目录下的**基线**库可以正常打开（避免「拒绝一切」的假绿）
    let fresh = dir.path().join("fresh.sqlite");
    let store = Store::open(&fresh).unwrap();
    assert_eq!(store.schema_version().unwrap(), BASELINE_VERSION);
}

#[test]
fn baseline_rows_and_flags_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("baseline.sqlite");
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), BASELINE_VERSION);
        let id = store
            .add_feed("https://fixture.invalid/rss", Some("Preserved"))
            .unwrap();
        let feed = rustrss_core::parse(b"<rss version='2.0'><channel><title>Fixture</title><item><guid>one</guid><title>Article</title><description>Cached body</description></item></channel></rss>").unwrap();
        store.upsert_entries(id, &feed.entries).unwrap();
        store
            .record_fetch(id, "ok", None, Some("etag"), None)
            .unwrap();
        let entry = store.list_entries(&EntryQuery::default()).unwrap()[0].id;
        store.set_read(&[entry], true).unwrap();
        store.set_starred(&[entry], true).unwrap();
    }

    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), BASELINE_VERSION);
    assert_eq!(store.entry_count().unwrap(), 1);
    let entry = store.get_entry(1).unwrap().unwrap();
    assert!(entry.read && entry.starred);
    assert_eq!(entry.content_text.as_deref(), Some("Cached body"));
    let row = store.feed_row(1).unwrap().unwrap();
    assert_eq!(row.title, "Preserved");
    assert!(row.last_fetched_at.is_some());
    assert_eq!(store.cache_headers(1).unwrap().0.as_deref(), Some("etag"));
    drop(store);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(app_id(&conn), BASELINE_APPLICATION_ID as i64);
}

#[test]
fn legacy_v1_frozen_db_is_refused_at_detect_level() {
    // 关键反例：旧链只跑到 v1 的冻结库——有用户表、application_id=0，而 user_version
    // **恰好等于基线（1）**。若 detect 改回「按版本区间判断」（Round-1 B1 的复发形态），
    // 这个库会被当成新库接受、随后运行时查询报错；断言落在 `detect()` 本身而不是备份校验里。
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-v1.sqlite");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(LEGACY_CHAIN[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        schema::detect(&conn).unwrap(),
        SchemaState::Foreign { user_version: 1 },
        "v1 冻结库必须按缺少应用标识被识别为 Foreign，而不是按 user_version=1 当成 Current"
    );
    assert_eq!(app_id(&conn), 0);
    drop(conn);
    let err = match Store::open(&path) {
        Ok(_) => panic!("v1 冻结库必须被拒绝"),
        Err(e) => e,
    };
    assert!(matches!(err, StoreError::SchemaRefused { .. }), "{err:?}");
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
