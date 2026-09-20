//! 存储层测试。重点验四件事：
//! 1. 同一篇文章重复入库不会变成多条（去重）；
//! 2. 刷新不覆盖阅读状态（已读不会被刷回未读）；
//! 3. 删源能级联清掉条目与全文索引；
//! 4. 中文检索真的能用（FTS5 默认分词器对中文无效，靠预分词解决）。

use rustrss_core::store::schema::MIGRATIONS;
use rustrss_core::{Entry, EntryQuery, IdOrigin, MarkScope, Store};

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

    let marked = store.mark_all_read(MarkScope::Feed(feed_id)).unwrap();
    assert_eq!(marked, 2);
    assert_eq!(store.unread_total().unwrap(), 0);
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
