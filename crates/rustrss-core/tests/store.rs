//! 存储层测试。重点验四件事：
//! 1. 同一篇文章重复入库不会变成多条（去重）；
//! 2. 刷新不覆盖阅读状态（已读不会被刷回未读）；
//! 3. 删源能级联清掉条目与全文索引；
//! 4. 中文检索真的能用（FTS5 默认分词器对中文无效，靠预分词解决）。

use rustrss_core::store::schema::MIGRATIONS;
use rustrss_core::store::{LIST_HIDE_READ_KEY, LIST_SORT_KEY};
use rustrss_core::{Entry, EntryQuery, IdOrigin, ListSort, MarkScope, Store, UnreadGroupBy};
use rustrss_core::rsshub;

fn mk_entry(stable_id: &str, title: &str, text: &str) -> Entry {
    Entry {
        stable_id: stable_id.to_string(),
        id_origin: IdOrigin::SourceData,
        source_id: stable_id.to_string(),
        title: title.to_string(),
        url: Some(format!("https://example.com/{stable_id}")),
        author: None,
        published: None,
        updated: None,
        summary: Some(text.to_string()),
        content_html: Some(format!("<p>{text}</p>")),
        content_text: Some(text.to_string()),
        categories: Vec::new(),
    }
}

fn setup() -> (Store, i64) {
    let store = Store::open_in_memory().expect("内存库应能打开");
    let feed_id = store
        .add_feed("https://example.com/feed.xml", Some("示例源"))
        .expect("加源应成功");
    (store, feed_id)
}

#[test]
fn schema_reaches_latest_version() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(
        store.schema_version().unwrap() as usize,
        MIGRATIONS.len(),
        "迁移应把 schema 推到最新版本"
    );
}

#[test]
fn add_feed_is_idempotent() {
    let store = Store::open_in_memory().unwrap();
    let a = store.add_feed("https://example.com/f.xml", None).unwrap();
    let b = store.add_feed("https://example.com/f.xml", Some("改名也无效")).unwrap();
    assert_eq!(a, b, "同一 URL 重复添加应返回既有 id");
    assert_eq!(store.list_feeds().unwrap().len(), 1);
}

#[test]
fn upsert_dedupes_by_stable_id() {
    let (store, feed_id) = setup();
    let entries = vec![
        mk_entry("a1", "第一篇", "正文一"),
        mk_entry("a2", "第二篇", "正文二"),
    ];

    let first = store.upsert_entries(feed_id, &entries).unwrap();
    assert_eq!(first.inserted, 2);
    assert_eq!(first.updated, 0);
    assert_eq!(store.entry_count().unwrap(), 2);

    // 原样再入库：内容没变 → 既不该新增，也不该写库
    let second = store.upsert_entries(feed_id, &entries).unwrap();
    assert_eq!(second.inserted, 0);
    assert_eq!(second.updated, 0);
    assert_eq!(second.unchanged, 2);
    assert_eq!(store.entry_count().unwrap(), 2, "重复刷新不该产生新条目");

    // 内容变了 → 计为更新
    let mut changed = entries.clone();
    changed[0].title = "第一篇（改过）".to_string();
    let third = store.upsert_entries(feed_id, &changed).unwrap();
    assert_eq!(third.updated, 1);
    assert_eq!(third.unchanged, 1);
    assert_eq!(store.entry_count().unwrap(), 2);
}

#[test]
fn refresh_keeps_read_and_starred_state() {
    let (store, feed_id) = setup();
    store
        .upsert_entries(feed_id, &[mk_entry("a1", "标题", "原文")])
        .unwrap();
    let id = store.list_entries(&EntryQuery::default()).unwrap()[0].id;
    store.set_read(&[id], true).unwrap();
    store.set_starred(&[id], true).unwrap();

    // 模拟「正文被源更新了」的一次刷新
    store
        .upsert_entries(feed_id, &[mk_entry("a1", "标题", "更新后的正文")])
        .unwrap();

    let row = store.get_entry(id).unwrap().expect("条目还在");
    assert!(row.read, "刷新不应把已读刷回未读");
    assert!(row.starred, "刷新不应丢掉星标");
    assert_eq!(row.content_text.as_deref(), Some("更新后的正文"));
}

#[test]
fn unread_counts_track_state() {
    let (store, feed_id) = setup();
    store
        .upsert_entries(
            feed_id,
            &[
                mk_entry("a1", "一", "x"),
                mk_entry("a2", "二", "y"),
                mk_entry("a3", "三", "z"),
            ],
        )
        .unwrap();
    assert_eq!(store.unread_total().unwrap(), 3);
    assert_eq!(store.unread_by_feed().unwrap(), vec![(feed_id, 3)]);

    let all = store.list_entries(&EntryQuery::default()).unwrap();
    store.set_read(&[all[0].id], true).unwrap();
    assert_eq!(store.unread_total().unwrap(), 2);

    let unread = store
        .list_entries(&EntryQuery {
            unread_only: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(unread.len(), 2);

    let marked = store.mark_all(MarkScope::Feed(feed_id), true).unwrap();
    assert_eq!(marked, 2);
    assert_eq!(store.unread_total().unwrap(), 0);

    // 反向（撤销）：全部未读
    let unmarked = store.mark_all(MarkScope::Feed(feed_id), false).unwrap();
    assert_eq!(unmarked, 3, "含之前手动改过的那条也应被改回");
    assert_eq!(store.unread_total().unwrap(), 3);
}

#[test]
fn settings_roundtrip_and_defaults() {
    let store = Store::open_in_memory().unwrap();

    // 缺失 → 用默认值
    assert!(store.bool_setting("ui.mark_read_on_navigate", true).unwrap());
    assert!(store.setting("ui.mark_read_on_navigate").unwrap().is_none());

    store.set_bool_setting("ui.mark_read_on_navigate", false).unwrap();
    assert!(!store.bool_setting("ui.mark_read_on_navigate", true).unwrap());

    // 重复写入是覆盖而不是报错
    store.set_bool_setting("ui.mark_read_on_navigate", true).unwrap();
    assert!(store.bool_setting("ui.mark_read_on_navigate", false).unwrap());

    // 值不合法时回退到默认值，而不是把功能卡死
    store.set_setting("ui.mark_read_on_navigate", "yes-please").unwrap();
    assert!(store.bool_setting("ui.mark_read_on_navigate", true).unwrap());
    assert!(!store.bool_setting("ui.mark_read_on_navigate", false).unwrap());

    let all = store.all_settings().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, "ui.mark_read_on_navigate");
}

/// 销毁设置键（MCP 写 token 的「销毁」路径就在这上面）：删掉后读回是「不存在」，
/// 且重复删除幂等——能力开关类设置不能因为多删一次就报错。
#[test]
fn delete_setting_is_idempotent_and_removes_the_key() {
    let store = Store::open_in_memory().unwrap();

    store.set_setting("mcp.write_token", "deadbeef").unwrap();
    assert_eq!(store.setting("mcp.write_token").unwrap().as_deref(), Some("deadbeef"));

    store.delete_setting("mcp.write_token").unwrap();
    assert!(store.setting("mcp.write_token").unwrap().is_none());
    assert!(!store.all_settings().unwrap().iter().any(|(k, _)| k == "mcp.write_token"));

    // 幂等：删不存在的键不报错
    store.delete_setting("mcp.write_token").unwrap();
    store.delete_setting("never.existed").unwrap();
}

#[test]
fn deleting_feed_cascades_entries_and_search_index() {
    let (store, feed_id) = setup();
    store
        .upsert_entries(feed_id, &[mk_entry("a1", "WebKit 渲染", "WebKit 相关内容")])
        .unwrap();
    assert_eq!(store.search("WebKit", 10).unwrap().len(), 1);

    store.remove_feed(feed_id).unwrap();
    assert_eq!(store.entry_count().unwrap(), 0, "条目应随源级联删除");
    assert!(
        store.search("WebKit", 10).unwrap().is_empty(),
        "全文索引也应随之清理（外部内容表 + 触发器）"
    );
}

#[test]
fn search_supports_latin_and_chinese() {
    let (store, feed_id) = setup();
    store
        .upsert_entries(
            feed_id,
            &[
                mk_entry("a1", "WebKit 换用 Skia", "WebKitGTK 的合成器改用 Skia 实现"),
                mk_entry("a2", "架构设计笔记", "本文讨论架构设计的取舍，异步闭包也在其中"),
                mk_entry("a3", "无关条目", "这条与检索词完全无关"),
            ],
        )
        .unwrap();

    // 拉丁词：大小写不敏感
    let hits = store.search("webkit", 10).unwrap();
    assert_eq!(hits.len(), 1, "拉丁词检索应命中 1 条: {hits:?}");
    assert_eq!(hits[0].stable_id, "a1");

    // 中文两字词（bigram 命中）
    let hits = store.search("架构", 10).unwrap();
    assert_eq!(hits.len(), 1, "中文两字词应命中: {hits:?}");

    // 中文长词：短语查询，要求连续出现
    assert_eq!(store.search("架构设计", 10).unwrap().len(), 1);
    assert_eq!(store.search("计取", 10).unwrap().len(), 0, "不连续的片段不该命中");

    // 中文单字：bigram 覆盖不到 → 走 LIKE 兜底
    assert_eq!(store.search("架", 10).unwrap().len(), 1);

    // 无匹配
    assert!(store.search("不存在的词", 10).unwrap().is_empty());
}

#[test]
fn list_entries_filters_and_limits() {
    let store = Store::open_in_memory().unwrap();
    let f1 = store.add_feed("https://a.example/f", Some("A")).unwrap();
    let f2 = store.add_feed("https://b.example/f", Some("B")).unwrap();
    store
        .upsert_entries(
            f1,
            &[
                mk_entry("a1", "A1", "x"),
                mk_entry("a2", "A2", "y"),
            ],
        )
        .unwrap();
    store.upsert_entries(f2, &[mk_entry("b1", "B1", "z")]).unwrap();

    assert_eq!(store.list_entries(&EntryQuery::default()).unwrap().len(), 3);

    let only_f1 = store
        .list_entries(&EntryQuery {
            feed_id: Some(f1),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(only_f1.len(), 2);
    assert!(only_f1.iter().all(|e| e.feed_id == f1));
    assert_eq!(only_f1[0].feed_title, "A");
    // 改名后列表里的 feed_title 也跟着变（同一个 COALESCE 显示口径）
    store.set_feed_custom_title(f1, Some("A 自定义")).unwrap();
    let renamed = store
        .list_entries(&EntryQuery {
            feed_id: Some(f1),
            ..Default::default()
        })
        .unwrap();
    assert!(
        renamed.iter().all(|e| e.feed_title == "A 自定义"),
        "列表需要显示名而不是源站名: {:?}",
        renamed.iter().map(|e| &e.feed_title).collect::<Vec<_>>()
    );
    assert_eq!(
        store.get_entry(renamed[0].id).unwrap().unwrap().feed_title,
        "A 自定义",
        "阅读区（get_entry）同一口径"
    );
    let hits = store.search("A1", 10).unwrap();
    assert!(!hits.is_empty(), "搜索应命中");
    assert_eq!(hits[0].feed_title, "A 自定义", "搜索结果同一口径");

    let limited = store
        .list_entries(&EntryQuery {
            limit: Some(1),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(limited.len(), 1);
}

#[test]
fn fetch_status_and_cache_headers_roundtrip() {
    let (store, feed_id) = setup();
    assert_eq!(store.cache_headers(feed_id).unwrap(), (None, None));

    store
        .record_fetch(
            feed_id,
            "ok",
            None,
            Some("W/\"abc\""),
            Some("Wed, 21 Oct 2026 07:28:00 GMT"),
        )
        .unwrap();

    let (etag, lm) = store.cache_headers(feed_id).unwrap();
    assert_eq!(etag.as_deref(), Some("W/\"abc\""));
    assert!(lm.is_some());

    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds[0].last_status.as_deref(), Some("ok"));

    // 失败也要记下来，便于界面上标记「抓取失败」的源
    store
        .record_fetch(feed_id, "http_404", Some("Not Found"), None, None)
        .unwrap();
    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds[0].last_status.as_deref(), Some("http_404"));
    assert_eq!(feeds[0].last_error.as_deref(), Some("Not Found"));
}

#[test]
fn read_later_is_independent_and_queryable() {
    let (store, feed_id) = setup();
    store
        .upsert_entries(feed_id, &[mk_entry("a1", "第一篇", "内容")])
        .unwrap();
    store
        .upsert_entries(feed_id, &[mk_entry("a2", "第二篇", "内容")])
        .unwrap();

    // 标记稍后读不影响已读/星标
    let all = store.list_entries(&EntryQuery::default()).unwrap();
    let ids: Vec<i64> = all.iter().map(|e| e.id).collect();
    store.set_read_later(&[ids[0]], true).unwrap();
    store.set_read(&[ids[0]], true).unwrap();

    let row = store.get_entry(ids[0]).unwrap().unwrap();
    assert!(row.read_later, "应已标记稍后读");
    assert!(row.read, "稍后读不应改变已读状态");
    assert!(!row.starred, "稍后读不应改变星标状态");

    // 视图：含已读条目，取消后退出视图
    let later = store
        .list_entries(&EntryQuery {
            read_later_only: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(store.read_later_total().unwrap(), 1);

    store.set_read_later(&[ids[0]], false).unwrap();
    assert_eq!(store.read_later_total().unwrap(), 0);
    let later = store
        .list_entries(&EntryQuery {
            read_later_only: true,
            ..Default::default()
        })
        .unwrap();
    assert!(later.is_empty());
    let _ = feed_id;
}

#[test]
fn migration_preserves_existing_rows_on_upgrade() {
    // 真实走一次 v3→v4 升级：手工建一个 user_version=3 的库（entries 为 v3
    // 形状，无 read_later 列）并预置数据，Store::open 应只跑第 4 条迁移
    // （ALTER 加列 + 部分索引），既有行的 read/starred 保留、read_later 可用。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-migration-test-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let conn = rusqlite::Connection::open(&db_path).expect("应能建旧库");
        conn.execute_batch(
            r#"
            PRAGMA user_version = 3;
            CREATE TABLE feeds (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL
            );
            CREATE TABLE entries (
                id INTEGER PRIMARY KEY,
                feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
                stable_id TEXT NOT NULL,
                id_origin TEXT NOT NULL,
                title TEXT NOT NULL,
                url TEXT,
                author TEXT,
                published_at INTEGER,
                updated_at INTEGER,
                summary TEXT,
                content_html TEXT,
                content_text TEXT,
                search_tokens TEXT NOT NULL DEFAULT '',
                content_hash TEXT NOT NULL DEFAULT '',
                read INTEGER NOT NULL DEFAULT 0,
                starred INTEGER NOT NULL DEFAULT 0,
                fetched_at INTEGER NOT NULL,
                UNIQUE (feed_id, stable_id)
            );
            INSERT INTO feeds (id, url, title) VALUES (1, 'https://example.com/f.xml', '源');
            INSERT INTO entries (id, feed_id, stable_id, id_origin, title, search_tokens, content_hash, read, starred, fetched_at)
            VALUES (1, 1, 'm1', 'SourceData', '迁移保留', 'm1', 'h', 1, 0, 0);
            "#,
        )
        .expect("建 v3 形状库应成功");
    }
    let store = Store::open(&db_path).expect("打开应自动跑 v4 迁移");
    let row = store.get_entry(1).unwrap().unwrap();
    assert!(row.read, "既有 read=1 应保留");
    assert!(!row.starred);
    store.set_read_later(&[1], true).unwrap();
    let row = store.get_entry(1).unwrap().unwrap();
    assert!(row.read_later, "迁移后新列应可写读");
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn migration_v6_to_v7_adds_fulltext_flag_on_real_file() {
    // 真实走一次 v6→v7：用迁移 1..6 **原样**建一个 user_version=6 的真文件库
    // （v6 形状不手抄，避免抄错而漂移），预置一条已读的摘要型条目，再让 Store::open
    // 只跑第 7 条迁移。断言：既有行与状态保留、新列默认 0（摘要型条目因此能显示
    // 「获取全文」）、写回可用且可搜、二次打开不重复执行。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-v7-migration-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let conn = rusqlite::Connection::open(&db_path).expect("应能建 v6 库");
        for sql in &MIGRATIONS[..6] {
            conn.execute_batch(sql).expect("应能跑 v1..v6 迁移");
        }
        conn.pragma_update(None, "user_version", 6).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO feeds (id, url, title, created_at)
            VALUES (1, 'https://example.com/feed.xml', '源', 0);
            INSERT INTO entries (id, feed_id, stable_id, id_origin, title, url, summary,
                                 search_tokens, content_hash, read, starred, fetched_at, read_later)
            VALUES (1, 1, 'm1', 'source_data', '迁移保留', 'https://example.com/p1', '一句摘要',
                    '迁移保留', 'h', 1, 0, 0, 0);
            "#,
        )
        .expect("预置数据应成功");
    }

    let store = Store::open(&db_path).expect("打开应自动跑 v7 迁移");
    assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
    let row = store.get_entry(1).unwrap().expect("既有条目应保留");
    assert_eq!(row.title, "迁移保留");
    assert!(row.read, "既有 read=1 应保留");
    assert!(row.needs_fulltext, "新列默认 0 + 正文缺失 → 摘要型待抓");

    // 新列随即可写：写回正文后标记生效，检索索引也跟着更新
    store
        .set_fulltext(1, "<p>抓到的正文</p>", "抓到的正文")
        .unwrap();
    let row = store.get_entry(1).unwrap().unwrap();
    assert!(!row.needs_fulltext);
    assert_eq!(row.content_text.as_deref(), Some("抓到的正文"));
    assert_eq!(store.search("抓到的", 10).unwrap().len(), 1, "写回后应可搜");

    // 幂等：二次打开不重复执行迁移，数据原样
    drop(store);
    let store = Store::open(&db_path).expect("二次打开应成功");
    assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(
        store.get_entry(1).unwrap().unwrap().content_text.as_deref(),
        Some("抓到的正文")
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn migration_v7_to_v8_adds_per_feed_interval_on_real_file() {
    // 真实走一次 v7→v8：用迁移 1..7 **原样**建一个 user_version=7 的真文件库
    // （v7 形状不手抄，避免抄错而漂移），预置一个源与一条已读条目，再让
    // Store::open 只跑第 8 条迁移。断言：既有行与状态保留、新列默认 NULL
    // （存量源继续跟随全局）、覆盖值可写且重启后仍在、二次打开幂等。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-v8-migration-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let conn = rusqlite::Connection::open(&db_path).expect("应能建 v7 库");
        for sql in &MIGRATIONS[..7] {
            conn.execute_batch(sql).expect("应能跑 v1..v7 迁移");
        }
        conn.pragma_update(None, "user_version", 7).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO feeds (id, url, title, created_at)
            VALUES (1, 'https://example.com/feed.xml', '源', 0);
            INSERT INTO entries (id, feed_id, stable_id, id_origin, title, url, summary,
                                 search_tokens, content_hash, read, starred, fetched_at, read_later)
            VALUES (1, 1, 'm1', 'source_data', '迁移保留', 'https://example.com/p1', '一句摘要',
                    '迁移保留', 'h', 1, 0, 0, 0);
            "#,
        )
        .expect("预置数据应成功");
    }

    let store = Store::open(&db_path).expect("打开应自动跑 v8 迁移");
    assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds.len(), 1, "既有的源应保留");
    assert_eq!(
        feeds[0].refresh_interval_minutes, None,
        "升级后新列为 NULL＝跟随全局（行为与升级前一致）"
    );
    let row = store.get_entry(1).unwrap().expect("既有条目应保留");
    assert_eq!(row.title, "迁移保留");
    assert!(row.read, "既有 read=1 应保留");
    assert_eq!(store.feeds_with_interval().unwrap(), vec![(1, None, None)]);

    // 新列随即可写：覆盖值落库，二次打开（重启）后仍在
    store.set_feed_refresh_interval(1, Some(15)).unwrap();
    drop(store);
    let store = Store::open(&db_path).expect("二次打开应成功");
    assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(
        store.list_feeds().unwrap()[0].refresh_interval_minutes,
        Some(15),
        "覆盖值应持久化（重启保留）"
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn feed_refresh_interval_round_trip_and_scan() {
    let (store, feed_id) = setup();
    let other = store
        .add_feed("https://example.com/other.xml", Some("另一源"))
        .unwrap();

    // 默认：NULL（跟随全局），扫描查询原样透出
    assert_eq!(
        store.feeds_with_interval().unwrap(),
        vec![(feed_id, None, None), (other, None, None)],
        "两个源都未覆盖、都未抓过"
    );

    // 写覆盖 → 返回行、列表行、扫描行三处都透出新列
    let row = store.set_feed_refresh_interval(feed_id, Some(15)).unwrap();
    assert_eq!(row.id, feed_id);
    assert_eq!(row.refresh_interval_minutes, Some(15));
    let listed = store.list_feeds().unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|f| f.id == feed_id)
            .unwrap()
            .refresh_interval_minutes,
        Some(15)
    );
    assert_eq!(
        store.feeds_with_interval().unwrap(),
        vec![(feed_id, Some(15), None), (other, None, None)],
        "只有被覆盖的那个源带出档位"
    );

    // 抓取一次后扫描能读到 last_fetched_at（调度到期基准）
    store.record_fetch(feed_id, "ok", None, None, None).unwrap();
    let scanned = store.feeds_with_interval().unwrap();
    assert!(
        scanned[0].2.is_some(),
        "抓取后扫描应带出 last_fetched_at: {scanned:?}"
    );

    // 恢复跟随全局
    let row = store.set_feed_refresh_interval(feed_id, None).unwrap();
    assert_eq!(row.refresh_interval_minutes, None);
    assert_eq!(store.feeds_with_interval().unwrap()[0].1, None);

    // 不存在的源：可读错误，不静默成功
    let err = store.set_feed_refresh_interval(9999, Some(30)).unwrap_err();
    assert!(err.to_string().contains("不存在"), "实际: {err}");
}

#[test]
fn custom_title_is_display_name_and_survives_refresh() {
    // 自定义标题的全部语义都在这条：设/清/刷新不覆盖/排序跟着显示名。
    let (store, feed_id) = setup();
    let other = store.add_feed("https://example.com/z.xml", Some("源 ZZ")).unwrap();

    // 未设自定义：显示名 = 源站名，source_title 同样，custom_title 为 NULL
    let row = store.feed_row(feed_id).unwrap().unwrap();
    assert_eq!(row.title, "示例源");
    assert_eq!(row.source_title, "示例源");
    assert_eq!(row.custom_title, None);

    // 设自定义：显示名换成它，源站名保留（对话框 placeholder 要的就是它）
    let row = store.set_feed_custom_title(feed_id, Some("我的贴名")).unwrap();
    assert_eq!(row.title, "我的贴名");
    assert_eq!(row.source_title, "示例源");
    assert_eq!(row.custom_title.as_deref(), Some("我的贴名"));

    // 刷新源元信息（源站改名）不动自定义名，但源站名要按新值存下
    store
        .update_feed_meta(feed_id, Some("示例源（改版）"), None, None, None)
        .unwrap();
    let row = store.feed_row(feed_id).unwrap().unwrap();
    assert_eq!(row.title, "我的贴名", "刷新不得覆盖用户自定义名");
    assert_eq!(
        row.source_title, "示例源（改版）",
        "源站名该跟着刷新更新（只影响清除自定义后的回退值）"
    );

    // 清除自定义：显示名回退到源站名
    let row = store.set_feed_custom_title(feed_id, None).unwrap();
    assert_eq!(row.title, "示例源（改版）");
    assert_eq!(row.custom_title, None);

    // 侧栏顺序跟着**显示名**走：把排在后面的源改成排最前的自定义名 → 它排第一
    store.set_feed_custom_title(other, Some("000 排前面")).unwrap();
    let listed = store.list_feeds().unwrap();
    assert_eq!(listed[0].id, other, "自定义名改了侧栏要立刻重排: {listed:?}");
    // 清除自定义 → 恢复按源站名排序（不写死 CJK 顺序，用同一套 ASCII NOCASE 口径算期望值）
    store.set_feed_custom_title(other, None).unwrap();
    let listed = store.list_feeds().unwrap();
    let names: Vec<String> = listed.iter().map(|f| f.title.clone()).collect();
    let mut expected = names.clone();
    expected.sort_by_key(|n| n.to_lowercase());
    assert_eq!(names, expected, "侧栏顺序应等于按显示名排序: {listed:?}");

    // 不存在的源：可读错误，不静默成功
    let err = store.set_feed_custom_title(9999, Some("x")).unwrap_err();
    assert!(err.to_string().contains("不存在"), "实际: {err}");
}

#[test]
fn migration_v9_to_v10_adds_custom_title_on_real_file() {
    // 真文件走一次 v9→v10：v9 库升级后既有订阅/条目与状态全部保留，新列 NULL
    // （存量源继续显示源站名）、可写且重启后仍在、二次打开幂等。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-v10-test-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&MIGRATIONS[..9].join(";")).unwrap();
        conn.execute_batch(
            "INSERT INTO feeds (id, url, title, created_at) VALUES (1, 'https://a', '源 A', 0);
             INSERT INTO entries (id, feed_id, stable_id, id_origin, title, url, summary,
                                  search_tokens, content_hash, read, starred, fetched_at, read_later)
               VALUES (1, 1, 'm1', 'source_data', '迁移保留', 'https://a/1', '摘要', '迁移保留', 'h', 1, 1, 0, 0);
             PRAGMA user_version = 9;",
        )
        .unwrap();
    }

    let store = Store::open(&db_path).expect("打开应自动跑 v10 迁移");
    assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds.len(), 1, "既有订阅应保留");
    assert_eq!(feeds[0].custom_title, None, "升级后新列为 NULL＝继续显示源站名");
    assert_eq!(feeds[0].title, "源 A");
    assert_eq!(feeds[0].source_title, "源 A");
    let row = store.get_entry(1).unwrap().expect("既有条目应保留");
    assert_eq!(row.feed_title, "源 A");
    assert!(row.read && row.starred, "既有阅读状态应保留");

    // 新列随即可写：自定义名落库，重启后仍在，列表/条目两处都按显示名出
    store.set_feed_custom_title(1, Some("改过的名")).unwrap();
    drop(store);
    let store = Store::open(&db_path).expect("二次打开应成功");
    assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds[0].title, "改过的名", "自定义名应持久化（重启保留）");
    assert_eq!(feeds[0].custom_title.as_deref(), Some("改过的名"));
    assert_eq!(
        store.get_entry(1).unwrap().unwrap().feed_title,
        "改过的名",
        "条目行同步显示自定义名"
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn folder_rename_delete_and_reassign() {
    let (store, feed_id) = setup();
    let f1 = store.add_folder("开发").unwrap();
    let f2 = store.add_folder("论坛").unwrap();
    store.assign_folder(feed_id, Some(f1)).unwrap();

    // 重命名 + 冲突检测
    store.rename_folder(f1, "AI 资讯").unwrap();
    assert_eq!(store.list_folders().unwrap()[0].1, "AI 资讯");
    let err = store.rename_folder(f1, "论坛").unwrap_err();
    assert!(err.to_string().contains("已存在"), "重名应给可读错误: {err}");

    // position 排序
    store.set_folder_position(f2, -1).unwrap();
    assert_eq!(store.list_folders().unwrap()[0].0, f2, "position 小的排前面");

    // 删除文件夹：订阅回到未分组，不删订阅
    store.assign_folder(feed_id, Some(f1)).unwrap();
    store.delete_folder(f1).unwrap();
    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds.len(), 1, "订阅不应被删除");
    assert_eq!(feeds[0].folder_id, None, "订阅应回到未分组");
    assert!(store.list_folders().unwrap().iter().all(|(id, _)| *id != f1));
}

#[test]
fn add_feed_stores_scheme_identity_and_dedupes_both_forms() {
    let dir = std::env::temp_dir().join(format!(
        "rustrss-mirror-add-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = Store::open(&dir).expect("打开应成功");
    // 未配置镜像：rsshub:// 原样落库（存储形态 = 抽象身份，不实例化）
    let id1 = store.add_feed("rsshub://telegram/channel/x", None).unwrap();
    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds[0].url, "rsshub://telegram/channel/x");
    assert_eq!(feeds[0].id, id1);
    // 配置自建镜像后添加：库内 url 仍是 scheme——add_feed 不读镜像设置
    store.set_setting(rsshub::MIRROR_KEY, "https://rsshub.example.com").unwrap();
    let id2 = store.add_feed("rsshub://v2ex/topics/hot", None).unwrap();
    let id3 = store.add_feed("https://rsshub.app/36kr/newsflashes", None).unwrap();
    let feeds = store.list_feeds().unwrap();
    let url_of = |id: i64| feeds.iter().find(|f| f.id == id).map(|f| f.url.clone()).unwrap();
    assert_eq!(url_of(id2), "rsshub://v2ex/topics/hot", "scheme 不实例化");
    assert_eq!(url_of(id3), "rsshub://36kr/newsflashes", "官方域转 scheme");
    // 两形态判重：同一路由的 scheme 与官方域写法是同一个订阅
    assert_eq!(
        store.add_feed("https://rsshub.app/v2ex/topics/hot", None).unwrap(),
        id2,
        "scheme 与官方域形态应判重为同一订阅"
    );
    assert_eq!(
        store.add_feed("https://www.rsshub.app/v2ex/topics/hot", None).unwrap(),
        id2,
        "www 官方域也是同一订阅"
    );
    // 非 RSSHub 域不受影响；去重幂等（同 URL 再加返回既有 id）
    let id4 = store.add_feed("https://example.com/feed.xml", None).unwrap();
    let feeds_after = store.list_feeds().unwrap(); // 重新拉取后再断言（url_of 闭包捕获的是旧列表）
    assert_eq!(
        feeds_after.iter().find(|f| f.id == id4).map(|f| f.url.clone()).unwrap(),
        "https://example.com/feed.xml"
    );
    assert_eq!(store.add_feed("rsshub://v2ex/topics/hot", None).unwrap(), id2);
    // 非规范 scheme 归一（三斜杠 / 大写）后同样判重
    assert_eq!(store.add_feed("rsshub:///v2ex/topics/hot", None).unwrap(), id2);
    assert_eq!(store.add_feed("RSSHUB://v2ex/topics/hot", None).unwrap(), id2);
    let _ = std::fs::remove_file(&dir);
}

#[test]
fn feed_endpoint_resolves_scheme_and_legacy_rows_against_mirror() {
    let store = Store::open_in_memory().expect("内存库应能打开");
    let scheme = store.add_feed("rsshub://test/1", None).unwrap();
    // 存量官方域行（没跑过归一化迁移的老库）+ 已实例化到自建镜像的历史行 + 普通源
    let legacy = store.add_feed("https://rsshub.app/legacy/1", None).unwrap();
    let frozen = store.add_feed("https://old.example.com/frozen/1", None).unwrap();
    let plain = store.add_feed("https://example.com/feed.xml", None).unwrap();
    // add_feed 已把官方域转成 scheme；直接改库模拟「迁移前的老库」
    store
        .update_feed_url(legacy, "https://rsshub.app/legacy/1?limit=10")
        .unwrap();

    let endpoint = |id: i64| store.feed_endpoint(id).unwrap().0;
    // 无镜像：scheme 与存量官方域都落到官方实例
    assert_eq!(endpoint(scheme), "https://rsshub.app/test/1");
    assert_eq!(endpoint(legacy), "https://rsshub.app/legacy/1?limit=10");
    assert_eq!(endpoint(frozen), "https://old.example.com/frozen/1");
    assert_eq!(endpoint(plain), "https://example.com/feed.xml");

    // 改镜像：下一次抓取立刻走新实例，零迁移；已实例化的历史行保持直抓不劣化
    store.set_setting(rsshub::MIRROR_KEY, "https://rsshub.example.com/").unwrap();
    assert_eq!(endpoint(scheme), "https://rsshub.example.com/test/1");
    assert_eq!(endpoint(legacy), "https://rsshub.example.com/legacy/1?limit=10");
    assert_eq!(endpoint(frozen), "https://old.example.com/frozen/1");
    assert_eq!(endpoint(plain), "https://example.com/feed.xml");

    // 再改一次镜像：库内 url 不变（解析只在抓取出口发生）
    store.set_setting(rsshub::MIRROR_KEY, "http://127.0.0.1:1200").unwrap();
    assert_eq!(endpoint(scheme), "http://127.0.0.1:1200/test/1");
    let stored = store.list_feeds().unwrap();
    assert_eq!(
        stored.iter().find(|f| f.id == scheme).unwrap().url,
        "rsshub://test/1",
        "镜像变化不得改写库内 url（否则又回到绑定实例的老问题）"
    );
    // 条件请求凭据照旧随行返回
    store.record_fetch(scheme, "ok", None, Some("\"v1\""), None).unwrap();
    let (url, etag, _) = store.feed_endpoint(scheme).unwrap();
    assert_eq!(url, "http://127.0.0.1:1200/test/1");
    assert_eq!(etag.as_deref(), Some("\"v1\""));
}

#[test]
fn normalization_rewrites_official_rows_and_keeps_scheme_rows() {
    let store = Store::open_in_memory().expect("内存库应能打开");
    // 模拟老库（归一化之前）：官方域行与纯 scheme 行并存；再加一条普通源与三斜杠 scheme
    let official = store.add_feed("https://rsshub.app/36kr/newsflashes?limit=10", None).unwrap();
    let scheme = store.add_feed("rsshub://telegram/channel/x", None).unwrap();
    let triple = store.add_feed("rsshub://gofans", None).unwrap();
    let plain = store.add_feed("https://example.com/feed.xml", None).unwrap();
    store.update_feed_url(official, "https://rsshub.app/36kr/newsflashes?limit=10").unwrap();
    store.update_feed_url(triple, "rsshub:///gofans").unwrap();
    store.set_setting(rsshub::MIRROR_KEY, "https://rsshub.example.com").unwrap();

    // 候选：scheme 与官方域都进候选，普通源不进
    let candidates = store.list_rsshub_migration_candidates().unwrap();
    assert_eq!(candidates.len(), 3, "官方域 + scheme + 三斜杠 scheme");
    // 预览只数真正会被改写的行（三斜杠 + 官方域），已规范的 scheme 行不算
    assert_eq!(store.count_rsshub_normalization_candidates().unwrap(), 2);

    let outcome = store.normalize_rsshub_feeds().unwrap();
    assert_eq!(outcome.migrated, 2, "官方域与三斜杠行被改写");
    assert_eq!(outcome.skipped, 0);
    assert!(outcome.errors.is_empty());

    let feeds = store.list_feeds().unwrap();
    let url_of = |id: i64| feeds.iter().find(|f| f.id == id).map(|f| f.url.clone()).unwrap();
    assert_eq!(url_of(official), "rsshub://36kr/newsflashes?limit=10", "官方域 → scheme");
    assert_eq!(url_of(scheme), "rsshub://telegram/channel/x", "已是 scheme 的行不动");
    assert_eq!(url_of(triple), "rsshub://gofans", "三斜杠归一");
    assert_eq!(url_of(plain), "https://example.com/feed.xml");

    // 幂等：再跑一遍零改动、零计数（这是「按钮可反复点」的保证）
    assert_eq!(store.count_rsshub_normalization_candidates().unwrap(), 0);
    assert_eq!(store.normalize_rsshub_feeds().unwrap().migrated, 0);

    // 冲突：目标地址已被其它订阅占用时计 skipped，不报错、不改写
    // 构造：一条已是 scheme 的行 + 一条被改写成该 scheme「官方域形态」的行（同路由两种写法）
    let holder = store.add_feed("rsshub://telegram/channel/x", None).unwrap();
    let clash = store.add_feed("https://example.com/to-be-legacy", None).unwrap();
    store
        .update_feed_url(clash, "https://rsshub.app/telegram/channel/x")
        .unwrap();
    assert_eq!(store.count_rsshub_normalization_candidates().unwrap(), 1);
    let outcome = store.normalize_rsshub_feeds().unwrap();
    assert_eq!(outcome.migrated, 0);
    assert_eq!(outcome.skipped, 1, "目标已被占用应跳过而不是报错");
    assert!(outcome.errors.is_empty());
    assert_eq!(url_of_via(&store, clash), "https://rsshub.app/telegram/channel/x", "冲突行保持原样");
    assert_eq!(url_of_via(&store, holder), "rsshub://telegram/channel/x");
}

fn url_of_via(store: &Store, id: i64) -> String {
    store
        .list_feeds()
        .unwrap()
        .into_iter()
        .find(|f| f.id == id)
        .map(|f| f.url)
        .unwrap()
}

#[test]
fn list_entries_order_by_uses_sortkey_index() {
    // v6 表达式索引的验收：视图切换高频走的 ORDER BY COALESCE(...) 必须命中
    // idx_entries_sortkey，而不是全表扫 + 临时 B-tree 排序（8k 条库上后者实测
    // 15-19ms，是界面卡顿的组成部分）。用真实文件库验，方便第二连接读执行计划。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-sortkey-test-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let store = Store::open(&db_path).unwrap();
        let feed_id = store
            .add_feed("https://example.com/feed.xml", Some("示例源"))
            .unwrap();
        let entries: Vec<Entry> = (0..30)
            .map(|i| mk_entry(&format!("s{i}"), &format!("标题{i}"), &format!("正文{i}")))
            .collect();
        store.upsert_entries(feed_id, &entries).unwrap();
    }
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let plan: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM entries
                 ORDER BY COALESCE(published_at, fetched_at) DESC, id DESC LIMIT 200",
            )
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(3)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    let joined = plan.join(" | ");
    assert!(
        joined.contains("idx_entries_sortkey"),
        "列表排序应走 v6 表达式索引，实际计划: {joined}"
    );
    assert!(
        !joined.to_lowercase().contains("temp b-tree"),
        "不应再出现临时排序树，实际计划: {joined}"
    );
    let _ = std::fs::remove_file(&db_path);
}

// -------------------------------------------------------- keyset 续扫（无限滚动）

/// 造一条 published_at 可控的条目：`None` = 让 `fetched_at` 补位（列表 sortkey 的第二来源）。
fn mk_entry_published(stable_id: &str, title: &str, published: Option<i64>) -> Entry {
    let mut e = mk_entry(stable_id, title, "正文");
    e.published = published.and_then(|t| chrono::DateTime::from_timestamp(t, 0));
    e
}

/// 从 `base.cursor`（没有就从首页）开始逐页续扫，返回拼接后的 id 序列。
///
/// 游标的 `read` 分量（unread_first 档需要）从末行直出——与前端 `paging.cursor`
/// 取的是同一样东西，测试不得自己"再造"一份。
fn page_through(store: &Store, base: &EntryQuery, page_size: u32) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut cursor = base.cursor;
    let mut cursor_read = base.cursor_read;
    // 页数兜底：游标条件写错（例如没排除游标行本身）会死循环，测试必须失败而不是挂住。
    for _ in 0..200 {
        let page = store
            .list_entries(&EntryQuery {
                limit: Some(page_size),
                cursor,
                cursor_read,
                ..base.clone()
            })
            .unwrap();
        if page.is_empty() {
            return ids;
        }
        assert!(page.len() <= page_size as usize, "单页不应超过 limit");
        cursor = page.last().map(|r| (r.sortkey, r.id));
        cursor_read = page.last().map(|r| r.read);
        ids.extend(page.iter().map(|r| r.id));
    }
    panic!("续扫 200 页仍未取空：游标条件可能写错");
}

#[test]
fn keyset_cursor_pages_through_tied_and_null_published_rows() {
    let (store, feed_id) = setup();
    let base = 1_700_000_000_i64;
    let mut entries = Vec::new();
    // 并列 sortkey：8 条同一 published_at，只能靠 id 兜底排序
    for i in 0..8 {
        entries.push(mk_entry_published(&format!("tie{i}"), &format!("并列{i}"), Some(base)));
    }
    // published_at 缺失：sortkey 由 fetched_at 补位（同一次 upsert 内 fetched_at 相同）
    for i in 0..8 {
        entries.push(mk_entry_published(&format!("null{i}"), &format!("无时间{i}"), None));
    }
    // 各自独立时间戳
    for i in 0..6 {
        entries.push(mk_entry_published(&format!("uniq{i}"), &format!("独立{i}"), Some(base - 100 + i)));
    }
    store.upsert_entries(feed_id, &entries).unwrap();

    let full = store
        .list_entries(&EntryQuery {
            limit: Some(500),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(full.len(), 22, "三组条目都应入库");
    assert!(
        full.windows(2).all(|w| (w[0].sortkey, w[0].id) > (w[1].sortkey, w[1].id)),
        "列表必须按 (sortkey, id) 严格降序"
    );
    // sortkey 是直出列：有 published_at 就必须等于它，缺失时由 fetched_at 补位
    for r in &full {
        match r.published_at {
            Some(t) => assert_eq!(r.sortkey, t, "有 published_at 时 sortkey 应等于它"),
            None => assert!(r.sortkey > base, "published_at 缺失时 sortkey 应由 fetched_at 补位"),
        }
    }
    assert_eq!(
        full.iter().filter(|r| r.sortkey == base).count(),
        8,
        "并列 sortkey 组应原样保留（不是被合并或丢掉）"
    );

    let paged = page_through(&store, &EntryQuery::default(), 5);
    assert_eq!(
        paged,
        full.iter().map(|r| r.id).collect::<Vec<_>>(),
        "续扫序列应与一次取全完全一致：无重复、无跳条、顺序相同"
    );

    // keyset 语义：翻页期间新入库的条目排在游标之前（更新），续页不重放它
    let first_page = store
        .list_entries(&EntryQuery {
            limit: Some(5),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(first_page.len(), 5);
    let cursor = first_page.last().map(|r| (r.sortkey, r.id));
    store
        .upsert_entries(
            feed_id,
            // 未来时间戳：确保新条目排在游标之前（列表最新处），而不是插到中间
            &[mk_entry_published("late", "翻页期间的新条目", Some(2_000_000_000))],
        )
        .unwrap();
    let rest = page_through(&store, &EntryQuery { cursor, ..Default::default() }, 5);
    assert_eq!(rest.len(), 22 - 5, "续页应只含游标之后的旧条目，不含新插入的那条");
    assert_eq!(
        rest,
        full.iter().skip(5).map(|r| r.id).collect::<Vec<_>>(),
        "续页内容应与首页之后的原序列一致"
    );
}

#[test]
fn keyset_cursor_supports_every_filter_shape() {
    let (store, f1) = setup();
    let f2 = store.add_feed("https://example.com/feed2.xml", Some("源二")).unwrap();
    let entries: Vec<Entry> = (0..20)
        .map(|i| mk_entry_published(&format!("e{i}"), &format!("标题{i}"), Some(1_700_000_000 + i)))
        .collect();
    store.upsert_entries(f1, &entries[..12]).unwrap();
    store.upsert_entries(f2, &entries[12..]).unwrap();

    let ids: Vec<i64> = store
        .list_entries(&EntryQuery {
            limit: Some(500),
            ..Default::default()
        })
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids.len(), 20);
    // 让四种筛选各有差异：前 10 条已读、每 3 条一个星标、星标里 2 条稍后读
    store.set_read(&ids[..10], true).unwrap();
    let starred: Vec<i64> = ids.iter().copied().step_by(3).collect();
    store.set_starred(&starred, true).unwrap();
    store.set_read_later(&starred[..2], true).unwrap();

    let shapes = [
        ("全部", EntryQuery::default()),
        ("未读", EntryQuery { unread_only: true, ..Default::default() }),
        ("单源", EntryQuery { feed_id: Some(f1), ..Default::default() }),
        ("星标", EntryQuery { starred_only: true, ..Default::default() }),
        ("稍后读", EntryQuery { read_later_only: true, ..Default::default() }),
        ("单源+未读", EntryQuery { feed_id: Some(f1), unread_only: true, ..Default::default() }),
    ];
    for (label, base) in shapes {
        let full = store
            .list_entries(&EntryQuery {
                limit: Some(500),
                ..base.clone()
            })
            .unwrap();
        assert!(!full.is_empty(), "{label} 视图应有数据，否则本用例对该形态没有约束力");
        let paged = page_through(&store, &base, 4);
        assert_eq!(
            paged,
            full.iter().map(|r| r.id).collect::<Vec<_>>(),
            "{label} 视图续扫应与一次取全完全一致"
        );
    }
}

#[test]
fn keyset_cursor_plans_use_sortkey_index() {
    // 续扫是逐页高频路径，计划不能打成「等值索引 + 临时排序」：那会每页重排一遍
    // 整个筛选集合。这里断言的是 list_entries 真实生成的 SQL（explain_list_entries
    // 复用同一个 SQL 构造函数，不存在测试里另抄一份而漂移的问题）。
    //
    // 用内存库（等于从未 ANALYZE 的新库）：本程序不跑 ANALYZE，所以 read/feed_id
    // 这类带等值索引的形态若只靠 planner 自选，实测会退化成 idx_entries_read_published
    // / idx_entries_feed_read + TEMP B-TREE——续扫 SQL 因此显式 INDEXED BY。
    let (store, feed_id) = setup();
    let entries: Vec<Entry> = (0..20)
        .map(|i| mk_entry_published(&format!("p{i}"), &format!("标题{i}"), Some(1_700_000_000 + i)))
        .collect();
    store.upsert_entries(feed_id, &entries).unwrap();
    let first = store
        .list_entries(&EntryQuery {
            limit: Some(5),
            ..Default::default()
        })
        .unwrap();
    store.set_read(&first[..2].iter().map(|r| r.id).collect::<Vec<_>>(), true).unwrap();
    let cursor = Some((1_700_000_010_i64, first[2].id));

    let cases = [
        (
            "read=0 + cursor",
            EntryQuery { unread_only: true, limit: Some(50), cursor, ..Default::default() },
        ),
        (
            "feed_id + cursor",
            EntryQuery { feed_id: Some(feed_id), limit: Some(50), cursor, ..Default::default() },
        ),
    ];
    for (label, q) in cases {
        let plan = store.explain_list_entries(&q).unwrap().join(" | ");
        assert!(
            plan.contains("SEARCH e USING INDEX idx_entries_sortkey"),
            "{label} 续扫应按 idx_entries_sortkey 定位游标（seek），实际计划: {plan}"
        );
        assert!(
            !plan.to_lowercase().contains("temp b-tree"),
            "{label} 续扫不应临时排序，实际计划: {plan}"
        );
    }
}

// ------------------------------------- 排序档（list.sort）与隐藏已读（list.hide_read）

/// 三档排序的固定样本：主序列 e0..e5（published 递增，e0 最旧）+ 两条并列 sortkey。
/// 状态：e0 / e2 / e5 已读（未读 = e1 e3 e4 + tie0 tie1）；e2 已读且星标（豁免验证用）；
/// e5 已读且稍后读。
///
/// 期望顺序：
/// - newest       ：时间倒序，并列按 id 倒序 → tie1 tie0 e5 e4 e3 e2 e1 e0
/// - oldest       ：时间升序，并列按 id 升序 → e0 e1 e2 e3 e4 e5 tie0 tie1
/// - unread_first ：未读组（组内时间倒序）→ tie0 tie1 e4 e3 e1，已读组 → e5 e2 e0
fn setup_sort_fixture() -> (Store, i64) {
    let (store, feed_id) = setup();
    let base = 1_700_000_000_i64;
    let mut entries: Vec<Entry> = (0..6)
        .map(|i| mk_entry_published(&format!("e{i}"), &format!("e{i}"), Some(base + i)))
        .collect();
    for t in ["tie0", "tie1"] {
        entries.push(mk_entry_published(t, t, Some(base + 100)));
    }
    store.upsert_entries(feed_id, &entries).unwrap();

    let rows = store
        .list_entries(&EntryQuery { limit: Some(500), ..Default::default() })
        .unwrap();
    let id_of = |title: &str| rows.iter().find(|r| r.title == title).unwrap().id;
    let read_ids: Vec<i64> = ["e0", "e2", "e5"].iter().map(|t| id_of(t)).collect();
    store.set_read(&read_ids, true).unwrap();
    store.set_starred(&[id_of("e2")], true).unwrap();
    store.set_read_later(&[id_of("e5")], true).unwrap();
    (store, feed_id)
}

/// 当前设置下的列表顺序（按标题，断言失败时一眼看出顺序差在哪）。
fn list_titles(store: &Store, q: EntryQuery) -> Vec<String> {
    store
        .list_entries(&EntryQuery {
            limit: Some(500),
            ..q
        })
        .unwrap()
        .iter()
        .map(|r| r.title.clone())
        .collect()
}

fn all_entries() -> EntryQuery {
    EntryQuery::default()
}

#[test]
fn list_sort_three_modes_order_and_setting_fallback() {
    let (store, _feed) = setup_sort_fixture();

    // 默认档（设置缺失）= newest，行为与加排序功能前完全一致
    assert_eq!(store.list_sort(), ListSort::Newest);
    let newest = ["tie1", "tie0", "e5", "e4", "e3", "e2", "e1", "e0"];
    assert_eq!(list_titles(&store, all_entries()), newest);

    store.set_setting(LIST_SORT_KEY, "oldest").unwrap();
    assert_eq!(store.list_sort(), ListSort::Oldest);
    assert_eq!(
        list_titles(&store, all_entries()),
        ["e0", "e1", "e2", "e3", "e4", "e5", "tie0", "tie1"],
        "最早在前：时间升序，并列按 id 升序"
    );

    store.set_setting(LIST_SORT_KEY, "unread_first").unwrap();
    assert_eq!(store.list_sort(), ListSort::UnreadFirst);
    assert_eq!(
        list_titles(&store, all_entries()),
        ["tie0", "tie1", "e4", "e3", "e1", "e5", "e2", "e0"],
        "未读优先：未读全在前（组内时间倒序），已读组在后"
    );

    // 白名单：trim 容忍；非法值（含大小写变体与空串）一律回默认档，不让拼错的设置卡死列表
    assert_eq!(
        ListSort::from_setting(Some(" unread_first ")),
        ListSort::UnreadFirst
    );
    for garbage in ["", "  ", "OLDEST", "Newest", "unread", "latest"] {
        assert_eq!(
            ListSort::from_setting(Some(garbage)),
            ListSort::Newest,
            "{garbage:?} 应归默认档"
        );
    }
    assert_eq!(ListSort::from_setting(None), ListSort::Newest);
    assert_eq!(ListSort::UnreadFirst.as_str(), "unread_first");

    // 库里被写坏 → 查询仍能跑，按默认档出（读取侧兜底，与 locale/theme 同口径）
    store.set_setting(LIST_SORT_KEY, "bogus").unwrap();
    assert_eq!(store.list_sort(), ListSort::Newest);
    assert_eq!(list_titles(&store, all_entries()), newest);
}

#[test]
fn list_sort_pagination_matrix_three_modes_by_five_views() {
    // 三档 × 五种视图：续扫序列与一次取全完全一致（不重不漏、顺序相同）。页大小特意小于
    // 未读组（5 条），让 unread_first 的游标经历「组内续扫 → 跨组边界 → 已读组内」三种位置。
    let (store, feed_id) = setup_sort_fixture();
    for sort in ["newest", "oldest", "unread_first"] {
        store.set_setting(LIST_SORT_KEY, sort).unwrap();
        let shapes = [
            ("全部", EntryQuery::default()),
            (
                "未读",
                EntryQuery {
                    unread_only: true,
                    ..Default::default()
                },
            ),
            (
                "星标",
                EntryQuery {
                    starred_only: true,
                    ..Default::default()
                },
            ),
            (
                "稍后读",
                EntryQuery {
                    read_later_only: true,
                    ..Default::default()
                },
            ),
            (
                "单源",
                EntryQuery {
                    feed_id: Some(feed_id),
                    ..Default::default()
                },
            ),
        ];
        for (label, base) in shapes {
            let full = store
                .list_entries(&EntryQuery { limit: Some(500), ..base.clone() })
                .unwrap();
            assert!(!full.is_empty(), "{sort}/{label} 视图应有数据，否则本用例无约束力");
            let paged = page_through(&store, &base, 3);
            assert_eq!(
                paged,
                full.iter().map(|r| r.id).collect::<Vec<_>>(),
                "{sort}/{label} 续扫应与一次取全完全一致（不重不漏、顺序相同）"
            );
        }
    }
}

#[test]
fn unread_first_cursor_is_composite_and_missing_read_half_falls_back_to_first_page() {
    let (store, _feed) = setup_sort_fixture();
    store.set_setting(LIST_SORT_KEY, "unread_first").unwrap();

    // 首页末行是未读的 e4（组内）——续页必须接着 e3 e1，然后跨到已读组
    let first = store
        .list_entries(&EntryQuery { limit: Some(3), ..Default::default() })
        .unwrap();
    assert_eq!(
        first.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["tie0", "tie1", "e4"]
    );
    let cursor = (first[2].sortkey, first[2].id);
    let next = store
        .list_entries(&EntryQuery {
            limit: Some(3),
            cursor: Some(cursor),
            cursor_read: Some(first[2].read),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        next.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["e3", "e1", "e5"],
        "组内续扫后应跨到已读组，且已读组内仍时间倒序"
    );
    // 已读组内的续页（游标 read=1）
    let next2 = store
        .list_entries(&EntryQuery {
            limit: Some(3),
            cursor: Some((next[2].sortkey, next[2].id)),
            cursor_read: Some(next[2].read),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        next2.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["e2", "e0"]
    );

    // 半截游标（只有 sortkey/id、没有 read 分量）按首页处理：宁可重取首页，也不静默翻到错页
    let no_read_half = store
        .list_entries(&EntryQuery {
            limit: Some(3),
            cursor: Some(cursor),
            cursor_read: None,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        no_read_half.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["tie0", "tie1", "e4"],
        "缺 read 分量时应按首页处理"
    );
}

#[test]
fn hide_read_filters_lists_but_starred_and_later_views_are_exempt() {
    let (store, feed_id) = setup_sort_fixture();
    assert!(!store.list_hide_read(), "默认关");
    assert_eq!(list_titles(&store, all_entries()).len(), 8);

    store.set_setting(LIST_HIDE_READ_KEY, "true").unwrap();
    assert!(store.list_hide_read());
    // 全部 / 单源 / 未读视图：已读行（e0 e2 e5）消失；未读视图本来就只有未读，冗余无害
    let unread_titles = ["tie1", "tie0", "e4", "e3", "e1"];
    assert_eq!(list_titles(&store, all_entries()), unread_titles);
    assert_eq!(list_titles(&store, EntryQuery { feed_id: Some(feed_id), ..Default::default() }), unread_titles);
    assert_eq!(list_titles(&store, EntryQuery { unread_only: true, ..Default::default() }), unread_titles);
    // 星标 / 稍后读视图豁免：读完了的星标（e2）与稍后读（e5）必须还找得到
    assert_eq!(
        list_titles(&store, EntryQuery { starred_only: true, ..Default::default() }),
        ["e2"],
        "hide_read 不得把已读星标藏掉"
    );
    assert_eq!(
        list_titles(&store, EntryQuery { read_later_only: true, ..Default::default() }),
        ["e5"],
        "hide_read 不得把已读稍后读藏掉"
    );
    // 搜索同样过滤（无搜索豁免：PRD 需求 3「所有视图生效」）
    assert_eq!(
        store.search("正文", 50).unwrap().len(),
        5,
        "隐藏已读对搜索结果同样生效"
    );
    // 三个档位下过滤都在（过滤器与排序正交）
    store.set_setting(LIST_SORT_KEY, "oldest").unwrap();
    assert_eq!(list_titles(&store, all_entries()), ["e1", "e3", "e4", "tie0", "tie1"]);
    store.set_setting(LIST_SORT_KEY, "unread_first").unwrap();
    assert_eq!(list_titles(&store, all_entries()), ["tie0", "tie1", "e4", "e3", "e1"]);

    // 关回去：已读行回来；库里写坏 → 回默认关
    store.set_setting(LIST_HIDE_READ_KEY, "false").unwrap();
    assert!(!store.list_hide_read());
    assert_eq!(list_titles(&store, all_entries()).len(), 8);
    store.set_setting(LIST_HIDE_READ_KEY, "bogus").unwrap();
    assert!(!store.list_hide_read());
}

#[test]
fn unread_first_cursor_plans_use_v11_composite_index() {
    // v11 复合索引的验收：unread_first 档首屏与续页都必须走
    // idx_entries_unread_sortkey，且不得出现 TEMP B-TREE。断言跑在 list_entries 真实
    // 生成的 SQL 上（explain_list_entries 复用同一个 SQL 构造函数，不存在测试另抄一份
    // 而漂移的问题）。
    //
    // 变异敏感度（手工核对过，改动会变红）：
    // - 删/改 v11 迁移 → FROM 上的 INDEXED BY 找不到索引，explain 直接报错；
    // - ORDER BY 少一列或方向错（例如漏掉 read 前缀）→ 索引不再满足顺序，
    //   plan 里出现 `USE TEMP B-TREE FOR ORDER BY`，下面的断言变红。
    let (store, feed_id) = setup_sort_fixture();
    store.set_setting(LIST_SORT_KEY, "unread_first").unwrap();

    // 首屏（无游标）：索引顺序即最终顺序，只需按序取前 N
    let first_plan = store
        .explain_list_entries(&EntryQuery { limit: Some(3), ..Default::default() })
        .unwrap()
        .join(" | ");
    assert!(
        first_plan.contains("idx_entries_unread_sortkey"),
        "首屏应走 v11 复合索引，实际计划: {first_plan}"
    );

    let first = store
        .list_entries(&EntryQuery { limit: Some(3), ..Default::default() })
        .unwrap();
    let cursor = (first[2].sortkey, first[2].id);
    let cases = [
        (
            "无筛选续页",
            EntryQuery {
                limit: Some(3),
                cursor: Some(cursor),
                cursor_read: Some(false),
                ..Default::default()
            },
        ),
        (
            "feed 等值筛选续页",
            EntryQuery {
                feed_id: Some(feed_id),
                limit: Some(3),
                cursor: Some(cursor),
                cursor_read: Some(false),
                ..Default::default()
            },
        ),
        (
            "read=0 筛选续页",
            EntryQuery {
                unread_only: true,
                limit: Some(3),
                cursor: Some(cursor),
                cursor_read: Some(false),
                ..Default::default()
            },
        ),
    ];
    for (label, q) in cases {
        let plan = store.explain_list_entries(&q).unwrap().join(" | ");
        assert!(
            plan.contains("SEARCH e USING INDEX idx_entries_unread_sortkey"),
            "{label} 应按 v11 索引定位游标（seek），实际计划: {plan}"
        );
        assert!(
            !plan.to_lowercase().contains("temp b-tree"),
            "{label} 不应临时排序，实际计划: {plan}"
        );
    }

    // 对照：同一份数据用 newest 档时走 v6 索引（两档各自钉自己的索引，互不串门）
    store.set_setting(LIST_SORT_KEY, "newest").unwrap();
    let newest_plan = store
        .explain_list_entries(&EntryQuery {
            limit: Some(3),
            cursor: Some(cursor),
            ..Default::default()
        })
        .unwrap()
        .join(" | ");
    assert!(
        newest_plan.contains("idx_entries_sortkey") && !newest_plan.contains("unread_sortkey"),
        "newest 档不应改用 v11 索引，实际计划: {newest_plan}"
    );
}

#[test]
fn oldest_pages_reverse_scan_the_sortkey_index() {
    // 最早在前用「反扫 v6 表达式索引」实现（不新增索引），且续页同样是 SEARCH 定位而非
    // 从头扫描 + 临时排序。第二组用例带 feed 等值筛选：这种形态不钉索引时 planner 会改选
    // idx_entries_feed_read + 临时排序（本程序从不 ANALYZE），所以断言对「去掉 INDEXED BY」
    // 这个变异是敏感的（手工核对过）。
    let (store, feed_id) = setup_sort_fixture();
    store.set_setting(LIST_SORT_KEY, "oldest").unwrap();

    let first = store
        .list_entries(&EntryQuery { limit: Some(3), ..Default::default() })
        .unwrap();
    assert_eq!(
        first.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["e0", "e1", "e2"]
    );
    let q = EntryQuery {
        limit: Some(3),
        cursor: Some((first[2].sortkey, first[2].id)),
        ..Default::default()
    };
    let plan = store.explain_list_entries(&q).unwrap().join(" | ");
    assert!(
        plan.contains("SEARCH e USING INDEX idx_entries_sortkey"),
        "oldest 续页应反扫表达式索引定位游标，实际计划: {plan}"
    );
    assert!(
        !plan.to_lowercase().contains("temp b-tree"),
        "oldest 续页不应临时排序，实际计划: {plan}"
    );
    let next = store.list_entries(&q).unwrap();
    assert_eq!(
        next.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["e3", "e4", "e5"],
        "更旧的接着来（升序档的续页方向不能反）"
    );

    store.set_setting(LIST_SORT_KEY, "oldest").unwrap();
    let feed_q = EntryQuery { feed_id: Some(feed_id), ..q.clone() };
    let feed_plan = store.explain_list_entries(&feed_q).unwrap().join(" | ");
    assert!(
        feed_plan.contains("SEARCH e USING INDEX idx_entries_sortkey")
            && !feed_plan.to_lowercase().contains("temp b-tree"),
        "oldest + 单源续页也必须钉住表达式索引（否则等值索引 + 临时排序），实际计划: {feed_plan}"
    );
    assert_eq!(
        store
            .list_entries(&feed_q)
            .unwrap()
            .iter()
            .map(|r| r.title.as_str())
            .collect::<Vec<_>>(),
        ["e3", "e4", "e5"]
    );
}

#[test]
fn migration_v10_to_v11_adds_unread_sortkey_index_on_real_file() {
    // 真文件走一次 v10→v11：v10 库（无复合索引）升级后既有行/状态全保留，
    // unread_first 查询立刻吃到新索引；升级前同一形态用不上该索引（差异即索引的功劳）。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-v11-test-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&MIGRATIONS[..10].join(";")).unwrap();
        conn.execute_batch(
            "INSERT INTO feeds (id, url, title, created_at) VALUES (1, 'https://a', '源 A', 0);
             INSERT INTO entries (id, feed_id, stable_id, id_origin, title, url, summary,
                                  search_tokens, content_hash, read, starred, fetched_at, read_later)
               VALUES (1, 1, 'm1', 'source_data', '未读的', 'https://a/1', '摘要', 'x', 'h', 0, 0, 100, 0),
                      (2, 1, 'm2', 'source_data', '已读的', 'https://a/2', '摘要', 'x', 'h', 1, 1, 200, 0);
             PRAGMA user_version = 10;",
        )
        .unwrap();
        // 升级前：v10 库里根本没有这个索引（下面的断言靠它区分「迁移的功劳」）
        let before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_entries_unread_sortkey'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, 0, "v10 库不应预先存在 v11 索引");
    }

    let store = Store::open(&db_path).expect("打开应自动跑 v11 迁移");
    assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    assert_eq!(store.entry_count().unwrap(), 2, "既有条目应保留");
    let row = store.get_entry(2).unwrap().expect("既有条目应保留");
    assert!(row.read && row.starred, "既有阅读状态应保留");

    store.set_setting(LIST_SORT_KEY, "unread_first").unwrap();
    assert_eq!(
        list_titles(&store, all_entries()),
        ["未读的", "已读的"],
        "未读优先：未读在前"
    );
    let plan = store
        .explain_list_entries(&EntryQuery { limit: Some(10), ..Default::default() })
        .unwrap()
        .join(" | ");
    assert!(
        plan.contains("idx_entries_unread_sortkey"),
        "升级后应吃到 v11 索引，实际计划: {plan}"
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn checkpoint_wal_succeeds_on_file_backed_db() {
    // 回归：61c95c6 曾把 checkpoint_wal 改成 execute()，而 wal_checkpoint 返回一行，
    // 每次调用都报「Execute returned results」（刷新后 stderr 刷错误日志）。
    // 用真文件库（WAL 生效）验证返回 Ok 且 WAL 文件被截断。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-checkpoint-test-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let store = Store::open(&db_path).unwrap();
        let feed_id = store.add_feed("https://example.com/feed.xml", Some("示例源")).unwrap();
        store.upsert_entries(feed_id, &[mk_entry("a", "标题", "正文")]).unwrap();
        store.checkpoint_wal().unwrap();
    }
    // checkpoint(TRUNCATE) 后 WAL 应为 0 字节
    let wal = std::path::PathBuf::from(format!("{}-wal", db_path.display()));
    if wal.exists() {
        assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0, "TRUNCATE 后 WAL 应为空");
    }
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&wal);
    let _ = std::fs::remove_file(format!("{}-shm", db_path.display()));
}

#[test]
fn counts_rides_indexes_never_the_table_btree() {
    // 回归：旧版单扫描 4 聚合因 starred/read_later 不在覆盖索引且排在正文大列之后，
    // planner 只能全表扫——冷启动真实库（8.5k 条 × 11.5KB 正文）实测 83-119ms/次且
    // 侧栏每次计数刷新都付。改 4 子查询后必须全部走覆盖索引（EXPLAIN 与线上 SQL
    // 同源，经 explain_counts），任何一个子查询退化为裸 SCAN entries 即失败。
    let store = Store::open_in_memory().unwrap();
    let feed_id = store.add_feed("https://example.com/feed.xml", Some("示例源")).unwrap();
    store
        .upsert_entries(feed_id, &[mk_entry("a", "标题", "正文"), mk_entry("b", "标题2", "正文2")])
        .unwrap();
    store.set_starred(&[1], true).unwrap();
    store.set_read_later(&[2], true).unwrap();
    store.set_read(&[1], true).unwrap();

    let (total, unread, starred, later) = store.counts().unwrap();
    assert_eq!((total, unread, starred, later), (2, 1, 1, 1), "4 计数值语义不变");

    let plan = store.explain_counts().unwrap().join(" | ");
    let bare_scan = plan
        .split(" | ")
        .find(|l| l.contains("SCAN entries") && !l.contains("USING"))
        .map(|l| l.to_string());
    assert!(
        bare_scan.is_none(),
        "任何子查询都不得退化为裸表扫描（正文大列的溢出页链代价），实际计划: {plan}"
    );
    for needle in [
        "idx_entries_sortkey",
        "idx_entries_starred",
        "idx_entries_read_later",
    ] {
        assert!(plan.contains(needle), "子查询应走 {needle}，实际计划: {plan}");
    }
    // 未读子查询：v11 之前走 idx_entries_read_published，v11 加了 (read, sortkey) 复合索引后
    // planner 改选 idx_entries_unread_sortkey——两个都是覆盖索引、都不穿正文大列的表 B 树，
    // 哪个都行；关键是不得退化成裸扫（上面已断言）。
    assert!(
        plan.contains("idx_entries_read_published") || plan.contains("idx_entries_unread_sortkey"),
        "未读子查询应走覆盖索引（read_published 或 v11 的 unread_sortkey），实际计划: {plan}"
    );
}

#[test]
fn migration_v8_to_v9_adds_starred_partial_index_on_real_file() {
    // 真文件走一次 v8→v9：v8 库（无星标部分索引）升级后 idx_entries_starred 存在、
    // counts 的 EXPLAIN 即刻全走索引、既有行与计数不变。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-v9-test-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&MIGRATIONS[..8].join(";")).unwrap();
        conn.execute_batch(
            "INSERT INTO feeds (id, url, title, created_at) VALUES (1, 'https://a', 'A', 0);
             INSERT INTO entries (id, feed_id, stable_id, id_origin, title, fetched_at)
               VALUES (1, 1, 's1', 'source_data', 'T1', 100), (2, 1, 's2', 'source_data', 'T2', 200);
             UPDATE entries SET starred = 1 WHERE id = 1;
             PRAGMA user_version = 8;",
        )
        .unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    let (total, unread, starred, _later) = store.counts().unwrap();
    assert_eq!((total, unread, starred), (2, 2, 1), "升级不改变计数");
    let plan = store.explain_counts().unwrap().join(" | ");
    assert!(
        !plan.split(" | ").any(|l| l.contains("SCAN entries") && !l.contains("USING")),
        "升级后即走索引: {plan}"
    );
    let _ = std::fs::remove_file(&db_path);
}

// ------------------------------------- 读侧扩展：since / until / feed_ids / unread_summary

/// 多源 + 时间范围夹具：
/// - 两个源 f1 / f2（f2 在分组里），各 6 条；
/// - `t0..t3` 有 published_at；`np0/np1` 无 published_at（sortkey 由 fetched_at 补位）；
/// - 状态：f1 的首条（最新那条）已读，其余未读。
fn setup_range_fixture() -> (Store, i64, i64, i64) {
    let (store, f1) = setup();
    let f2 = store.add_feed("https://example.com/feed2.xml", Some("源二")).unwrap();
    let folder = store.add_folder("分组").unwrap();
    store.assign_folder(f2, Some(folder)).unwrap();
    let base = 1_700_000_000_i64;
    let mk = |id: &str, published: Option<i64>| mk_entry_published(id, id, published);
    store
        .upsert_entries(
            f1,
            &[
                mk("a0", Some(base)),
                mk("a1", Some(base + 10)),
                mk("a2", Some(base + 20)),
                mk("a_np", None),
            ],
        )
        .unwrap();
    store
        .upsert_entries(
            f2,
            &[
                mk("b0", Some(base + 5)),
                mk("b1", Some(base + 15)),
                mk("b2", Some(base + 25)),
                mk("b_np", None),
            ],
        )
        .unwrap();
    let mut rows = store.list_entries(&EntryQuery { limit: Some(500), ..Default::default() }).unwrap();
    // 只把 f1 里 published = base + 20 的那条标已读（区分「源内最新」与普通行）
    let read_id = rows
        .iter()
        .find(|r| r.title == "a2")
        .map(|r| r.id)
        .unwrap();
    store.set_read(&[read_id], true).unwrap();
    rows.clear();
    (store, f1, f2, folder)
}

fn titles_of(store: &Store, q: EntryQuery) -> Vec<String> {
    store
        .list_entries(&EntryQuery { limit: Some(500), ..q })
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect()
}

#[test]
fn time_range_is_closed_and_measures_the_sortkey_expression() {
    let (store, _f1, _f2, _folder) = setup_range_fixture();
    let base = 1_700_000_000_i64;

    // 闭区间：>= since 且 <= until（边界值本身都在内）
    assert_eq!(
        titles_of(&store, EntryQuery { since: Some(base + 10), until: Some(base + 20), ..Default::default() }),
        ["a2", "b1", "a1"],
        "since/until 都是闭区间：base+10（a1/b1）与 base+20（a2）都应命中"
    );
    // 单边。注意：缺 published_at 的两条（a_np / b_np）的 sortkey 是入库时刻 fetched_at
    // （远大于 base），所以只要下界超过 base，它们就会和边界内的带时间条目一起出现。
    assert_eq!(
        titles_of(&store, EntryQuery { since: Some(base + 25), ..Default::default() }),
        ["b_np", "a_np", "b2"],
        "只要 since 时只卡下界；缺 published_at 的条目按 fetched_at 参与比较"
    );
    assert_eq!(
        titles_of(&store, EntryQuery { until: Some(base), ..Default::default() }),
        ["a0"],
        "只要 until 时只卡上界"
    );
    // since > until 自然匹配零条（不是报错）
    assert!(titles_of(&store, EntryQuery { since: Some(base + 30), until: Some(base), ..Default::default() }).is_empty());

    // published_at 缺失的条目参与过滤（sortkey 由 fetched_at 补位）：
    // 它的 fetched_at 是「入库当下」，必然 > base，因此 since=base 时它必须出现，
    // 而 since = 未来时刻时不会出现。这就是「口径是 COALESCE 而不是裸 published_at」的证据。
    let with_missing = titles_of(&store, EntryQuery { since: Some(base), ..Default::default() });
    assert!(with_missing.iter().any(|t| t == "a_np"), "缺 published_at 的条目应按 fetched_at 参与时间过滤：{with_missing:?}");
    assert!(titles_of(&store, EntryQuery { since: Some(i64::MAX - 1), ..Default::default() }).is_empty(), "未来时刻之后没有任何条目");
}

#[test]
fn feed_ids_filters_multi_source_and_empty_list_matches_nothing() {
    let (store, f1, f2, _folder) = setup_range_fixture();

    let two = titles_of(&store, EntryQuery { feed_ids: Some(vec![f1, f2]), ..Default::default() });
    assert_eq!(two.len(), 8, "两个源合起来 8 条：{two:?}");

    let only_second = titles_of(&store, EntryQuery { feed_ids: Some(vec![f2]), ..Default::default() });
    assert_eq!(only_second.len(), 4);
    assert!(only_second.iter().all(|t| t.starts_with('b')), "{only_second:?}");

    // 与其它条件叠加（AND 语义）
    let filtered = titles_of(
        &store,
        EntryQuery {
            feed_ids: Some(vec![f1, f2]),
            unread_only: true,
            limit: Some(500),
            ..Default::default()
        },
    );
    assert_eq!(filtered.len(), 7, "a2 已读，其余 7 条未读：{filtered:?}");

    // 未知 id：匹配零条
    assert!(titles_of(&store, EntryQuery { feed_ids: Some(vec![9999]), ..Default::default() }).is_empty());

    // 空列表 = 匹配零条（**不是**不过滤）：这是「某分组下没有订阅」的安全语义，
    // 写错方向就会把全库倒给调用方。
    assert!(
        titles_of(&store, EntryQuery { feed_ids: Some(vec![]), ..Default::default() }).is_empty(),
        "空 feed_ids 必须匹配零条，而不是退化成不过滤"
    );
    // None 才是不过滤
    assert_eq!(titles_of(&store, EntryQuery { feed_ids: None, ..Default::default() }).len(), 8);

    // 分组 → 源 id 解析（MCP 的 folder_id 走这条）
    let folder_ids = store.feed_ids_in_folder(_folder_of(&store, f2)).unwrap();
    assert_eq!(folder_ids, vec![f2]);
    assert!(store.feed_ids_in_folder(9999).unwrap().is_empty(), "不存在的分组解析出空列表");
}

fn _folder_of(store: &Store, feed_id: i64) -> i64 {
    store
        .list_feeds()
        .unwrap()
        .into_iter()
        .find(|f| f.id == feed_id)
        .unwrap()
        .folder_id
        .expect("夹具里 f2 应已归组")
}

#[test]
fn time_range_and_feed_ids_page_without_duplicates_or_gaps() {
    let (store, f1, f2, _folder) = setup_range_fixture();
    let base = 1_700_000_000_i64;
    let shapes = [
        (
            "时间范围",
            EntryQuery { since: Some(base - 1), until: Some(base + 20), ..Default::default() },
        ),
        (
            "多源 IN",
            EntryQuery { feed_ids: Some(vec![f1, f2]), ..Default::default() },
        ),
        (
            "多源 + 时间范围 + 未读",
            EntryQuery {
                feed_ids: Some(vec![f1, f2]),
                since: Some(base - 1),
                until: Some(base + 20),
                unread_only: true,
                ..Default::default()
            },
        ),
    ];
    for (label, base_q) in shapes {
        let full = store
            .list_entries(&EntryQuery { limit: Some(500), ..base_q.clone() })
            .unwrap();
        assert!(!full.is_empty(), "{label} 应有数据，否则本用例无约束力");
        let paged = page_through(&store, &base_q, 2);
        assert_eq!(
            paged,
            full.iter().map(|r| r.id).collect::<Vec<_>>(),
            "{label} 逐页续扫应与一次取全完全一致"
        );
        let mut dedup = paged.clone();
        dedup.dedup();
        assert_eq!(dedup.len(), paged.len(), "{label} 续扫不得重复");
    }
}

#[test]
fn time_range_and_multi_source_plans_ride_indexes_not_table_btree() {
    let (store, f1, f2, _folder) = setup_range_fixture();
    let base = 1_700_000_000_i64;

    // 时间范围（首屏）：范围条件直接吃 v6 表达式索引（同一 COALESCE 表达式），
    // 且 ORDER BY 由索引序满足（不得出现临时排序树）。
    let time_q = EntryQuery { since: Some(base), until: Some(base + 20), limit: Some(20), ..Default::default() };
    let plan = store.explain_list_entries(&time_q).unwrap().join(" | ");
    assert!(
        plan.contains("SEARCH e USING INDEX idx_entries_sortkey"),
        "时间范围应按表达式索引定位（range seek），实际计划: {plan}"
    );
    assert!(!plan.to_lowercase().contains("temp b-tree"), "时间范围不得临时排序: {plan}");

    // 多源 IN：钉排序索引按序扫 + 逐行过滤 feed_id，不得退化成等值索引 + 临时排序
    let multi_q = EntryQuery { feed_ids: Some(vec![f1, f2]), limit: Some(20), ..Default::default() };
    let plan = store.explain_list_entries(&multi_q).unwrap().join(" | ");
    assert!(
        plan.contains("USING INDEX idx_entries_sortkey"),
        "多源 IN 应走排序索引按序取，实际计划: {plan}"
    );
    assert!(!plan.to_lowercase().contains("temp b-tree"), "多源 IN 不得临时排序: {plan}");

    // 组合形态也不得出现裸表扫描（正文大列的溢出页链代价）
    let combo = EntryQuery {
        feed_ids: Some(vec![f1, f2]),
        since: Some(base),
        until: Some(base + 25),
        limit: Some(20),
        ..Default::default()
    };
    let plan = store.explain_list_entries(&combo).unwrap().join(" | ");
    assert!(
        !plan.split(" | ").any(|l| l.contains("SCAN entries") && !l.contains("USING")),
        "组合过滤同样不得裸扫 entries：{plan}"
    );
}

#[test]
fn time_range_plan_assertion_turns_red_without_sortkey_index() {
    // 变异校验：把 v6 表达式索引拿掉，上面那条「时间范围走 idx_entries_sortkey」的
    // 断言必须真的转红（否则断言只是恰好成立，而不是在守索引）。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-range-plan-mutation-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let base = 1_700_000_000_i64;
    {
        let store = Store::open(&db_path).unwrap();
        let feed_id = store.add_feed("https://example.com/feed.xml", Some("示例源")).unwrap();
        store
            .upsert_entries(feed_id, &[mk_entry_published("a", "A", Some(base))])
            .unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("DROP INDEX idx_entries_sortkey;").unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    let plan = store
        .explain_list_entries(&EntryQuery { since: Some(base - 1), until: Some(base + 1), limit: Some(20), ..Default::default() })
        .unwrap()
        .join(" | ");
    assert!(
        !plan.contains("idx_entries_sortkey"),
        "索引已删，计划里不该还有它（有则说明断言指错了对象）: {plan}"
    );
    assert!(
        plan.to_lowercase().contains("temp b-tree") || plan.split(" | ").any(|l| l.contains("SCAN entries") && !l.contains("USING")),
        "掉索引后应退化成临时排序或裸扫——这正是断言要拦住的形态: {plan}"
    );
    // 钉索引的多源形态掉索引后直接报错（计划根本建不出来）——同为「转红」
    let err = store.explain_list_entries(&EntryQuery { feed_ids: Some(vec![1]), limit: Some(20), ..Default::default() });
    assert!(err.is_err(), "INDEXED BY 的索引被删后 EXPLAIN 必须报错，而不是静默换个索引");
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn unread_summary_groups_by_feed_and_folder() {
    let (store, f1, f2, folder) = setup_range_fixture();

    // 按源：组集合完整（每条都有自己的一组），未读为 0 的组也出
    let by_feed = store.unread_summary(UnreadGroupBy::Feed).unwrap();
    assert_eq!(by_feed.len(), 2, "两个源两组: {by_feed:?}");
    let f1_group = by_feed.iter().find(|g| g.id == Some(f1)).unwrap();
    assert_eq!(f1_group.name, "示例源");
    assert_eq!(f1_group.unread, 3, "f1 四条里 a2 已读: {f1_group:?}");
    let f2_group = by_feed.iter().find(|g| g.id == Some(f2)).unwrap();
    assert_eq!((f2_group.name.as_str(), f2_group.unread), ("源二", 4));

    // 多组 + 未读为 0：把 f1 全标已读，f1 组应仍在结果里且 unread = 0
    let f1_ids: Vec<i64> = store
        .list_entries(&EntryQuery { feed_id: Some(f1), limit: Some(500), ..Default::default() })
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    store.set_read(&f1_ids, true).unwrap();
    let by_feed = store.unread_summary(UnreadGroupBy::Feed).unwrap();
    assert_eq!(by_feed.len(), 2);
    assert_eq!(by_feed.iter().find(|g| g.id == Some(f1)).unwrap().unread, 0, "未读为 0 的源也要出现");

    // 按分组：命中的分组 + 未分组（id = None，垫最后）；空组 unread = 0
    let empty_folder = store.add_folder("空组").unwrap();
    let by_folder = store.unread_summary(UnreadGroupBy::Folder).unwrap();
    assert_eq!(by_folder.iter().filter(|g| g.id == Some(folder)).count(), 1, "已知分组应恰好一组: {by_folder:?}");
    assert_eq!(by_folder.iter().find(|g| g.id == Some(folder)).unwrap().unread, 4);
    let empty = by_folder.iter().find(|g| g.id == Some(empty_folder)).expect("空组也要出现在结果里");
    assert_eq!((empty.name.as_str(), empty.unread), ("空组", 0));
    let ungrouped = by_folder.iter().find(|g| g.id.is_none()).expect("有未分组订阅就该出这一组");
    assert_eq!((ungrouped.name.as_str(), ungrouped.unread), ("未分组", 0), "f1 已全部已读");

    // 合计：各组合计应等于全库未读总数
    let total: i64 = by_folder.iter().map(|g| g.unread).sum();
    assert_eq!(total, store.unread_total().unwrap());

    // 空库：没有任何源 → 没有任何组（不报错）
    let empty_store = Store::open_in_memory().unwrap();
    assert!(empty_store.unread_summary(UnreadGroupBy::Feed).unwrap().is_empty());
    assert!(empty_store.unread_summary(UnreadGroupBy::Folder).unwrap().is_empty());
}

#[test]
fn unread_summary_rides_covering_index_never_table_btree() {
    // 回归口径同 counts()：聚合不得触碰正文大列所在的表 B 树（读未读就够，
    // 一旦回表就是每行穿 11KB 正文的溢出页链）。断言与线上 SQL 同源
    // （explain_unread_summary 走的就是那两个常量）。
    let (store, _f1, _f2, _folder) = setup_range_fixture();
    for (label, by) in [
        ("by feed", UnreadGroupBy::Feed),
        ("by folder", UnreadGroupBy::Folder),
    ] {
        let plan = store.explain_unread_summary(by).unwrap().join(" | ");
        assert!(
            !plan.split(" | ").any(|l| l.contains("SCAN entries") && !l.contains("USING")),
            "{label}: 不得裸扫 entries 表 B 树: {plan}"
        );
        assert!(
            plan.contains("COVERING INDEX idx_entries_feed_read"),
            "{label}: 应走 (feed_id, read) 覆盖索引: {plan}"
        );
    }

    // 变异校验：把覆盖索引删掉，断言（经 INDEXED BY）必须转红——证明断言确实钉在
    // 那个索引上，而不是恰好成立。
    let db_path = std::env::temp_dir().join(format!(
        "rustrss-unread-summary-mutation-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    {
        let store = Store::open(&db_path).unwrap();
        let feed_id = store.add_feed("https://example.com/feed.xml", Some("示例源")).unwrap();
        store.upsert_entries(feed_id, &[mk_entry("a", "A", "正文")]).unwrap();
        assert!(store.explain_unread_summary(UnreadGroupBy::Feed).is_ok());
    }
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("DROP INDEX idx_entries_feed_read;").unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    assert!(
        store.explain_unread_summary(UnreadGroupBy::Feed).is_err(),
        "覆盖索引被删后计划必须建不出来（转红），而不是静默换一个可能穿表 B 树的索引"
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn entry_query_explicit_sort_and_hide_read_override_settings() {
    // MCP 侧「默认口径固定为 newest + 不隐藏已读」靠的就是这两个显式覆盖：
    // None = 跟随界面设置（界面路径不变），Some = 以显式值为准（MCP 路径不吃界面设置）。
    let (store, _feed) = setup_sort_fixture();

    // 界面设置：最早在前 + 隐藏已读
    store.set_setting(LIST_SORT_KEY, "oldest").unwrap();
    store.set_setting(LIST_HIDE_READ_KEY, "true").unwrap();
    assert_eq!(
        list_titles(&store, all_entries()),
        ["e1", "e3", "e4", "tie0", "tie1"],
        "不传覆盖时仍跟随设置（界面行为不变）"
    );

    // 显式覆盖 sort=newest + hide_read=false（= MCP 默认口径），设置被完全绕过
    assert_eq!(
        list_titles(
            &store,
            EntryQuery { sort: Some(ListSort::Newest), hide_read: Some(false), ..Default::default() }
        ),
        ["tie1", "tie0", "e5", "e4", "e3", "e2", "e1", "e0"],
        "显式 newest + 不隐藏已读：设置里的 oldest/hide_read 都不生效"
    );

    // 反向：设置是 newest + 不隐藏已读时，显式 oldest + hide_read=true 也要生效
    store.set_setting(LIST_SORT_KEY, "newest").unwrap();
    store.set_setting(LIST_HIDE_READ_KEY, "false").unwrap();
    assert_eq!(
        list_titles(
            &store,
            EntryQuery { sort: Some(ListSort::Oldest), hide_read: Some(true), ..Default::default() }
        ),
        ["e1", "e3", "e4", "tie0", "tie1"],
        "显式 oldest + 隐藏已读应生效"
    );

    // EXPLAIN 与线上 SQL 同源 → 覆盖同样生效：设置是 newest，显式 unread_first 必须钉 v11 索引
    let plan = store
        .explain_list_entries(&EntryQuery {
            sort: Some(ListSort::UnreadFirst),
            limit: Some(3),
            ..Default::default()
        })
        .unwrap()
        .join(" | ");
    assert!(
        plan.contains("idx_entries_unread_sortkey"),
        "显式排序档在 EXPLAIN 路径也要生效（与线上同一份 SQL），实际计划: {plan}"
    );
}
