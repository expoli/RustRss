//! 存储层测试。重点验四件事：
//! 1. 同一篇文章重复入库不会变成多条（去重）；
//! 2. 刷新不覆盖阅读状态（已读不会被刷回未读）；
//! 3. 删源能级联清掉条目与全文索引；
//! 4. 中文检索真的能用（FTS5 默认分词器对中文无效，靠预分词解决）。

use rustrss_core::store::schema::MIGRATIONS;
use rustrss_core::{Entry, EntryQuery, IdOrigin, MarkScope, Store};
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
fn add_feed_rewrites_rsshub_urls_via_mirror_setting() {
    let dir = std::env::temp_dir().join(format!(
        "rustrss-mirror-add-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = Store::open(&dir).expect("打开应成功");
    // 未配置镜像：rsshub:// 落库为官方实例
    let id1 = store.add_feed("rsshub://telegram/channel/x", None).unwrap();
    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds[0].url, "https://rsshub.app/telegram/channel/x");
    // 配置自建镜像：rsshub:// 与官方 https 都落库为镜像地址
    store.set_setting(rsshub::MIRROR_KEY, "https://rsshub.example.com").unwrap();
    let id2 = store.add_feed("rsshub://v2ex/topics/hot", None).unwrap();
    let id3 = store.add_feed("https://rsshub.app/36kr/newsflashes", None).unwrap();
    let feeds = store.list_feeds().unwrap();
    let url_of = |id: i64| feeds.iter().find(|f| f.id == id).map(|f| f.url.clone()).unwrap();
    assert_eq!(url_of(id2), "https://rsshub.example.com/v2ex/topics/hot");
    assert_eq!(url_of(id3), "https://rsshub.example.com/36kr/newsflashes");
    // 非 RSSHub 域不受影响；去重幂等（同 URL 再加返回既有 id）
    let id4 = store.add_feed("https://example.com/feed.xml", None).unwrap();
    let feeds_after = store.list_feeds().unwrap(); // 重新拉取后再断言（url_of 闭包捕获的是旧列表）
    assert_eq!(
        feeds_after.iter().find(|f| f.id == id4).map(|f| f.url.clone()).unwrap(),
        "https://example.com/feed.xml"
    );
    assert_eq!(store.add_feed("rsshub://v2ex/topics/hot", None).unwrap(), id2);
    let _ = std::fs::remove_file(&dir);
}

#[test]
fn migration_covers_both_legacy_forms_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!(
        "rustrss-mirror-migrate-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = Store::open(&dir).expect("打开应成功");
    // 旧库存量形态：镜像未配置时，rsshub:// 无法被 add_feed 实例化吗？
    // —— add_feed 收口在无镜像时会实例化为官方 URL；因此 rsshub:// 存量
    // 实际落库为官方地址，官方地址是候选；纯 rsshub:// 残留只在直接改库时出现。
    // 为覆盖"两种存量"，直接用 update_feed_url 构造 rsshub:// 存量。
    let scheme = store.add_feed("rsshub://telegram/channel/x", None).unwrap();
    let official = store.add_feed("https://rsshub.app/36kr/newsflashes", None).unwrap();
    let plain = store.add_feed("https://example.com/feed.xml", None).unwrap();
    // 构造一条真正的 rsshub:// 存量（模拟直接改库/外部写入）
    store.update_feed_url(scheme, "rsshub://telegram/channel/x").unwrap();
    store.set_setting(rsshub::MIRROR_KEY, "https://rsshub.example.com").unwrap();

    // 候选：rsshub:// 存量 + 官方域（普通源不算）
    let candidates = store.list_rsshub_migration_candidates().unwrap();
    assert_eq!(candidates.len(), 2, "rsshub:// 与官方域应为候选");

    // 迁移：store 层只有方法，组合逻辑在命令层——这里直接模拟命令行为
    let mirror = "https://rsshub.example.com";
    let mut migrated = 0;
    for (feed_id, url) in store.list_rsshub_migration_candidates().unwrap() {
        let target = rustrss_core::rsshub::normalize_rsshub_url(&url, mirror);
        if target != url {
            store.update_feed_url(feed_id, &target).unwrap();
            migrated += 1;
        }
    }
    assert_eq!(migrated, 2);

    // 幂等：再跑一遍无变化
    let again: usize = store
        .list_rsshub_migration_candidates()
        .unwrap()
        .into_iter()
        .filter(|(_, url)| rustrss_core::rsshub::normalize_rsshub_url(url, mirror) != *url)
        .count();
    assert_eq!(again, 0);

    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds.iter().find(|f| f.id == scheme).unwrap().url, "https://rsshub.example.com/telegram/channel/x");
    assert_eq!(feeds.iter().find(|f| f.id == official).unwrap().url, "https://rsshub.example.com/36kr/newsflashes");
    assert_eq!(feeds.iter().find(|f| f.id == plain).unwrap().url, "https://example.com/feed.xml");
    let _ = std::fs::remove_file(&dir);
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
fn page_through(store: &Store, base: &EntryQuery, page_size: u32) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut cursor = base.cursor;
    // 页数兜底：游标条件写错（例如没排除游标行本身）会死循环，测试必须失败而不是挂住。
    for _ in 0..200 {
        let page = store
            .list_entries(&EntryQuery {
                limit: Some(page_size),
                cursor,
                ..base.clone()
            })
            .unwrap();
        if page.is_empty() {
            return ids;
        }
        assert!(page.len() <= page_size as usize, "单页不应超过 limit");
        cursor = page.last().map(|r| (r.sortkey, r.id));
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
