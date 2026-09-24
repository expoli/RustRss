//! MCP 工具层测试：直接调工具方法，不经过 stdio。
//!
//! 重点是「列表不回正文」这条口径——它决定了 agent 的上下文会不会被撑爆。

use rustrss_core::{Entry, EntryQuery, IdOrigin, Store};
use rustrss_mcp::{
    GetArticleParams, ListArticlesParams, RustRssMcp, SearchParams, UnreadSummaryParams,
};

fn temp_db(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("rustrss-mcp-test-{tag}-{}.sqlite", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", p.display()));
    }
    p
}

fn entry(stable_id: &str, title: &str, text: &str) -> Entry {
    Entry {
        stable_id: stable_id.to_string(),
        id_origin: IdOrigin::SourceData,
        source_id: stable_id.to_string(),
        title: title.to_string(),
        url: Some(format!("https://example.com/{stable_id}")),
        author: Some("作者".to_string()),
        published: None,
        updated: None,
        summary: Some(text.to_string()),
        content_html: Some(format!("<p>{text}</p>")),
        content_text: Some(text.to_string()),
        thumbnail_url: None,
        categories: Vec::new(),
    }
}

/// 建一个有真实数据的库，返回 (server, db_path)。
/// `tag` 必须每个用例不同：测试并行跑，共用文件会互相删掉对方的库。
fn seeded(tag: &str) -> (RustRssMcp, std::path::PathBuf) {
    let db = temp_db(tag);
    let store = Store::open(&db).expect("建库失败");
    let feed_id = store
        .add_feed("https://example.com/feed.xml", Some("示例源"))
        .expect("加源失败");
    store
        .upsert_entries(
            feed_id,
            &[
                // 标题里刻意放引号与反斜杠：历史上手拼 JSON 会在这里出问题
                entry(
                    "a1",
                    "标题里有 \"引号\" 和 \\ 反斜杠",
                    "这是第一篇的正文，用来验证取正文的工具能拿到全文而不是摘要。",
                ),
                entry("a2", "架构设计笔记", "讨论异步闭包与架构设计的取舍。"),
            ],
        )
        .expect("入库失败");
    let server = RustRssMcp::open(&db).expect("打开库失败");
    (server, db)
}

fn parse(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("输出不是合法 JSON（{e}）：{json}"))
}

#[test]
fn list_feeds_reports_unread_and_status() {
    let (server, db) = seeded("feeds");
    let v = parse(&server.list_feeds_json());
    assert_eq!(v["count"], 1);
    assert_eq!(v["feeds"][0]["title"], "示例源");
    assert_eq!(v["feeds"][0]["unread"], 2);
    assert!(v["feeds"][0].get("status").is_some());
    let _ = std::fs::remove_file(db);
}

#[test]
fn list_articles_returns_metadata_without_body() {
    let (server, db) = seeded("meta");
    let out = server.list_articles_json(&ListArticlesParams {
        unread_only: true,
        limit: Some(5),
        ..Default::default()
    });
    let v = parse(&out);
    assert_eq!(v["count"], 2);

    let first = &v["articles"][0];
    // 元数据在
    assert!(first.get("title").is_some());
    assert!(first.get("id").is_some());
    assert!(first.get("summary").is_some());
    // 正文不在（这是设计口径，不是遗漏）
    assert!(
        first.get("text").is_none() && first.get("html").is_none(),
        "列表工具不得回正文：{first}"
    );
    // 含引号/反斜杠的标题必须仍是合法 JSON
    assert!(v["articles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["title"].as_str().unwrap().contains('"')));
    let _ = std::fs::remove_file(db);
}

#[test]
fn get_article_returns_full_body_and_optional_html() {
    let (server, db) = seeded("body");
    let listed = parse(&server.list_articles_json(&ListArticlesParams {
        limit: Some(10),
        ..Default::default()
    }));
    // 不假设返回顺序：按标题定位到含「正文」字样的那条
    let id = listed["articles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["summary"].as_str().unwrap_or_default().contains("正文"))
        .expect("应能找到目标条目")["id"]
        .as_i64()
        .unwrap();

    let text_only = parse(&server.get_article_json(&GetArticleParams {
        id,
        include_html: false,
    }));
    let body = text_only["text"].as_str().unwrap();
    assert!(body.contains("正文"), "应拿到全文：{body}");
    assert!(body.chars().count() > 10, "不应只是摘要");
    assert!(
        text_only.get("html").is_none(),
        "未要求 HTML 时不该回 HTML（体积控制）"
    );

    let with_html = parse(&server.get_article_json(&GetArticleParams {
        id,
        include_html: true,
    }));
    assert!(with_html["html"].as_str().unwrap().contains("<p>"));
    let _ = std::fs::remove_file(db);
}

#[test]
fn get_article_reports_missing_id_helpfully() {
    let (server, db) = seeded("missing");
    let v = parse(&server.get_article_json(&GetArticleParams {
        id: 99999,
        include_html: false,
    }));
    assert!(v["error"].as_str().unwrap().contains("未找到"));
    assert!(v["error"].as_str().unwrap().contains("list_articles"));
    let _ = std::fs::remove_file(db);
}

#[test]
fn search_works_for_latin_and_chinese() {
    let (server, db) = seeded("search");
    let latin = parse(&server.search_articles_json(&SearchParams {
        query: "asynchronous".to_string(),
        limit: None,
    }));
    assert_eq!(latin["count"], 0, "英文词不在这批数据里");

    let cjk = parse(&server.search_articles_json(&SearchParams {
        query: "架构".to_string(),
        limit: None,
    }));
    assert_eq!(cjk["count"], 1, "中文两字应命中：{cjk}");
    assert_eq!(cjk["articles"][0]["title"], "架构设计笔记");

    let none = parse(&server.search_articles_json(&SearchParams {
        query: "不存在的词".to_string(),
        limit: None,
    }));
    assert_eq!(none["count"], 0);
    let _ = std::fs::remove_file(db);
}

#[test]
fn stats_and_limit_clamping() {
    let (server, db) = seeded("stats");
    let v = parse(&server.db_stats_json());
    assert_eq!(v["feeds"], 1);
    assert_eq!(v["entries"], 2);
    assert_eq!(v["unread"], 2);

    // limit 超上限时应被夹住而不是报错
    let clamped = parse(&server.list_articles_json(&ListArticlesParams {
        limit: Some(9999),
        ..Default::default()
    }));
    assert_eq!(clamped["count"], 2);
    // page_size 是 limit 的同义参数（page_size 优先）
    let by_page_size = parse(&server.list_articles_json(&ListArticlesParams {
        page_size: Some(1),
        limit: Some(9999),
        ..Default::default()
    }));
    assert_eq!(by_page_size["count"], 1, "page_size 优先于 limit: {by_page_size}");
    assert_eq!(by_page_size["page_size"], 1);
    let _ = std::fs::remove_file(db);
}

// ------------------------------------- 读侧补参：过滤 / 排序 / 分页 / 分组 / 未读聚合

/// 多源 + 时间 + 状态夹具（MCP 层用）：
/// - 源 A（不在分组）a0..a6、源 B（在分组「科技」）b0..b6，时间交错：
///   a_i = base + 2i，b_i = base + 2i + 1 → 时间倒序是 b6 a6 b5 a5 … b0 a0；
/// - 状态：a5 已读、b0 已读（各源 6 条未读）、a6 星标、b6 稍后读。
struct Fixture {
    db: std::path::PathBuf,
    feed_a: i64,
    feed_b: i64,
    folder: i64,
    base: i64,
}

fn mk_entry_at(stable_id: &str, at: i64) -> Entry {
    let mut e = entry(stable_id, stable_id, &format!("{stable_id} 的正文"));
    e.published = chrono::DateTime::from_timestamp(at, 0);
    e
}

fn seeded_fixture(tag: &str) -> (RustRssMcp, Fixture) {
    let db = temp_db(tag);
    let base = 1_700_000_000_i64;
    let (feed_a, feed_b, folder) = {
        let store = Store::open(&db).expect("建库失败");
        let feed_a = store
            .add_feed("https://example.com/a.xml", Some("源A"))
            .expect("加源失败");
        let feed_b = store
            .add_feed("https://example.com/b.xml", Some("源B"))
            .expect("加源失败");
        let folder = store.add_folder("科技").expect("建分组失败");
        store.assign_folder(feed_b, Some(folder)).expect("归组失败");
        let a: Vec<Entry> = (0..7)
            .map(|i| mk_entry_at(&format!("a{i}"), base + 2 * i))
            .collect();
        let b: Vec<Entry> = (0..7)
            .map(|i| mk_entry_at(&format!("b{i}"), base + 2 * i + 1))
            .collect();
        store.upsert_entries(feed_a, &a).expect("入库 A 失败");
        store.upsert_entries(feed_b, &b).expect("入库 B 失败");
        let all: Vec<rustrss_core::EntryRow> = store
            .list_entries(&EntryQuery { limit: Some(50), ..Default::default() })
            .unwrap();
        let id_of = |title: &str| all.iter().find(|r| r.title == title).unwrap().id;
        store.set_read(&[id_of("a5"), id_of("b0")], true).unwrap();
        store.set_starred(&[id_of("a6")], true).unwrap();
        store.set_read_later(&[id_of("b6")], true).unwrap();
        (feed_a, feed_b, folder)
    };
    let server = RustRssMcp::open(&db).expect("打开库失败");
    (
        server,
        Fixture { db, feed_a, feed_b, folder, base },
    )
}

/// 一页的 id 序列（把 JSON 里的 articles 拉平）
fn ids_of(v: &serde_json::Value) -> Vec<i64> {
    v["articles"]
        .as_array()
        .expect("articles 应是数组")
        .iter()
        .map(|a| a["id"].as_i64().expect("id 应是整数"))
        .collect()
}

fn titles_in(v: &serde_json::Value) -> Vec<String> {
    v["articles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["title"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_articles_exposes_new_filters_sort_and_page_size() {
    let (server, fx) = seeded_fixture("filters");

    // 分组过滤（folder_id → feed ids）：只剩源 B 的 7 条
    let in_folder = parse(&server.list_articles_json(&ListArticlesParams {
        folder_id: Some(fx.folder),
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(in_folder["count"], 7);
    assert!(titles_in(&in_folder).iter().all(|t| t.starts_with('b')), "{in_folder}");

    // 单源过滤仍旧可用；两个源过滤参数互斥（口径不同，不能同时用）
    let single = parse(&server.list_articles_json(&ListArticlesParams {
        feed_id: Some(fx.feed_a),
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(single["count"], 7);
    assert!(titles_in(&single).iter().all(|t| t.starts_with('a')), "{single}");
    let single_b = parse(&server.list_articles_json(&ListArticlesParams {
        feed_id: Some(fx.feed_b),
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(titles_in(&single_b), titles_in(&in_folder), "单源 feed_id=源B 应与它的分组过滤结果一致");
    let both = parse(&server.list_articles_json(&ListArticlesParams {
        feed_id: Some(fx.feed_a),
        folder_id: Some(fx.folder),
        ..Default::default()
    }));
    assert!(both["error"].as_str().unwrap().contains("只能用一个"), "{both}");

    // 空分组：必须匹配零条，而不是把全库倒出来
    let empty_folder = Store::open(&fx.db).unwrap().add_folder("空分组").unwrap();
    let empty = parse(&server.list_articles_json(&ListArticlesParams {
        folder_id: Some(empty_folder),
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(empty["count"], 0, "空分组应匹配零条: {empty}");

    // 星标 / 稍后读
    let starred = parse(&server.list_articles_json(&ListArticlesParams {
        starred_only: true,
        ..Default::default()
    }));
    assert_eq!(titles_in(&starred), ["a6"]);
    let later = parse(&server.list_articles_json(&ListArticlesParams {
        read_later_only: true,
        ..Default::default()
    }));
    assert_eq!(titles_in(&later), ["b6"]);

    // 时间范围：闭区间（since 含 / until 含），比较 COALESCE(published_at, fetched_at)
    let windowed = parse(&server.list_articles_json(&ListArticlesParams {
        since: Some(fx.base + 4),
        until: Some(fx.base + 6),
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(titles_in(&windowed), ["a3", "b2", "a2"], "闭区间应含两个边界（a2 与 a3）");

    // 排序档：oldest 从 a0 开始；unread_first 首屏全未读
    let oldest = parse(&server.list_articles_json(&ListArticlesParams {
        sort: Some("oldest".to_string()),
        page_size: Some(3),
        ..Default::default()
    }));
    assert_eq!(titles_in(&oldest), ["a0", "b0", "a1"], "最早在前");
    let unread_first = parse(&server.list_articles_json(&ListArticlesParams {
        sort: Some("unread_first".to_string()),
        page_size: Some(3),
        ..Default::default()
    }));
    assert!(
        unread_first["articles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["read"] == false),
        "未读优先档首屏应全是未读: {unread_first}"
    );

    // hide_read：默认 false（不隐藏）；显式 true 时去掉 a5 / b0
    let default_hide = parse(&server.list_articles_json(&ListArticlesParams {
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(default_hide["count"], 14, "默认不隐藏已读");
    let hide = parse(&server.list_articles_json(&ListArticlesParams {
        hide_read: Some(true),
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(hide["count"], 12, "14 条里 a5/b0 已读，隐藏后剩 12 条");
    assert!(!titles_in(&hide).iter().any(|t| t == "a5" || t == "b0"), "{hide}");

    // 每页条数：默认 10 条（14 条数据），上限 50（超限夹住不报错）
    let default_page = parse(&server.list_articles_json(&ListArticlesParams::default()));
    assert_eq!(default_page["count"], 10, "默认每页 10 条");
    assert_eq!(default_page["page_size"], 10);
    assert!(default_page["next_cursor"].is_string(), "满页应给出下一页游标");
    let clamped = parse(&server.list_articles_json(&ListArticlesParams {
        page_size: Some(9999),
        ..Default::default()
    }));
    assert_eq!(clamped["count"], 14);
    assert_eq!(clamped["page_size"], 50);
    assert!(clamped["next_cursor"].is_null(), "不满页不应给游标");

    let _ = std::fs::remove_file(&fx.db);
}

#[test]
fn list_articles_defaults_do_not_follow_ui_settings() {
    let (server, fx) = seeded_fixture("ui-settings");

    // 把界面设置改成「最早在前 + 隐藏已读」——MCP 的默认口径必须不受影响
    {
        let store = Store::open(&fx.db).unwrap();
        store.set_setting("list.sort", "oldest").unwrap();
        store.set_setting("list.hide_read", "true").unwrap();
    }

    let default_call = parse(&server.list_articles_json(&ListArticlesParams {
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(default_call["count"], 14, "默认不隐藏已读（即使界面设置开着 hide_read）");
    assert_eq!(
        titles_in(&default_call).first().map(String::as_str),
        Some("b6"),
        "默认 newest（即使界面设置是 oldest）"
    );
    assert!(titles_in(&default_call).contains(&"a5".to_string()), "已读条目仍应出现");

    // 显式传参才生效
    let explicit = parse(&server.list_articles_json(&ListArticlesParams {
        sort: Some("oldest".to_string()),
        hide_read: Some(true),
        page_size: Some(50),
        ..Default::default()
    }));
    assert_eq!(explicit["count"], 12);
    assert_eq!(titles_in(&explicit).first().map(String::as_str), Some("a0"));

    // 反向证明：设置真的写进去了（不然上面的断言是空的）
    let ui_view = Store::open(&fx.db).unwrap();
    assert_eq!(ui_view.list_sort(), rustrss_core::ListSort::Oldest);
    assert!(ui_view.list_hide_read());

    let _ = std::fs::remove_file(&fx.db);
}

#[test]
fn list_articles_cursor_pages_without_duplicates_or_gaps() {
    let (server, fx) = seeded_fixture("paging");

    // 一次取全（同一请求形态、只把页大小放大）作为基准
    let one_shot = parse(&server.list_articles_json(&ListArticlesParams {
        page_size: Some(50),
        ..Default::default()
    }));
    let expected = ids_of(&one_shot);

    for sort in ["newest", "oldest", "unread_first"] {
        let shape = |cursor: Option<String>| {
            parse(&server.list_articles_json(&ListArticlesParams {
                page_size: Some(3),
                cursor,
                sort: Some(sort.to_string()),
                ..Default::default()
            }))
        };
        let sorted_one_shot = parse(&server.list_articles_json(&ListArticlesParams {
            page_size: Some(50),
            sort: Some(sort.to_string()),
            ..Default::default()
        }));
        let expected = if sort == "newest" { expected.clone() } else { ids_of(&sorted_one_shot) };

        let mut collected: Vec<i64> = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        loop {
            let page = shape(cursor.clone());
            let ids = ids_of(&page);
            assert!(ids.len() <= 3, "{sort} 单页不应超过 page_size");
            collected.extend(ids.iter().copied());
            pages += 1;
            assert!(pages <= 20, "{sort} 翻页不该超过 20 页（游标写错会死循环）");
            match page["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_string()),
                None => break,
            }
        }
        assert_eq!(collected, expected, "{sort} 逐页续扫应与一次取全完全一致（不重不漏）");

        let mut dedup = collected.clone();
        dedup.dedup();
        assert_eq!(dedup.len(), collected.len(), "{sort} 不得重复");
    }

    // 分组 + 未读优先 + 游标（游标多带一位 read 的形态）
    let in_folder_one_shot = parse(&server.list_articles_json(&ListArticlesParams {
        folder_id: Some(fx.folder),
        sort: Some("unread_first".to_string()),
        page_size: Some(50),
        ..Default::default()
    }));
    let mut collected = Vec::new();
    let mut cursor = None;
    loop {
        let page = parse(&server.list_articles_json(&ListArticlesParams {
            folder_id: Some(fx.folder),
            sort: Some("unread_first".to_string()),
            page_size: Some(2),
            cursor: cursor.clone(),
            ..Default::default()
        }));
        collected.extend(ids_of(&page));
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    assert_eq!(collected, ids_of(&in_folder_one_shot), "分组 + 未读优先的续扫应一致");

    let _ = std::fs::remove_file(&fx.db);
}

#[test]
fn list_folders_and_unread_summary_report_complete_groups() {
    let (server, fx) = seeded_fixture("summary");

    let folders = parse(&server.list_folders_json());
    assert_eq!(folders["count"], 1);
    assert_eq!(folders["folders"][0]["id"], serde_json::json!(fx.folder));
    assert_eq!(folders["folders"][0]["name"], "科技");
    assert_eq!(folders["folders"][0]["unread"], 6, "源 B 的 b0 已读，剩 6 条未读");
    assert_eq!(folders["ungrouped_unread"], 6, "源 A 的 a5 已读，剩 6 条未读");
    assert_eq!(folders["total_unread"], 12);

    // 按源：组集合完整（含未读为 0 的源也会出现）
    let by_feed = parse(&server.unread_summary_json(&UnreadSummaryParams { by: None }));
    assert_eq!(by_feed["by"], "feed");
    assert_eq!(by_feed["count"], 2);
    assert_eq!(by_feed["total_unread"], 12);
    let groups = by_feed["groups"].as_array().unwrap();
    let a = groups.iter().find(|g| g["id"] == serde_json::json!(fx.feed_a)).unwrap();
    assert_eq!((a["name"].as_str().unwrap(), a["unread"].as_i64().unwrap()), ("源A", 6));

    let by_folder = parse(&server.unread_summary_json(&UnreadSummaryParams {
        by: Some("folder".to_string()),
    }));
    assert_eq!(by_folder["by"], "folder");
    let groups = by_folder["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2, "一个分组 + 未分组: {by_folder}");
    let grouped = groups.iter().find(|g| g["id"] == serde_json::json!(fx.folder)).unwrap();
    assert_eq!(grouped["unread"], 6);
    let ungrouped = groups.iter().find(|g| g["id"].is_null()).expect("未分组应单列一组");
    assert_eq!((ungrouped["name"].as_str().unwrap(), ungrouped["unread"].as_i64().unwrap()), ("未分组", 6));
    // 未分组垫底（与侧栏顺序一致）
    assert!(groups.last().unwrap()["id"].is_null(), "未分组应在最后一组");

    // 不合法 by：明确报错，不静默回默认
    let bad = parse(&server.unread_summary_json(&UnreadSummaryParams {
        by: Some("source".to_string()),
    }));
    assert!(bad["error"].as_str().unwrap().contains("feed / folder"), "{bad}");

    // 未读为 0 的源仍要出现在结果里（组集合完整，调用方不补空组）
    {
        let store = Store::open(&fx.db).unwrap();
        let a_ids: Vec<i64> = store
            .list_entries(&EntryQuery { feed_id: Some(fx.feed_a), limit: Some(50), ..Default::default() })
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        store.set_read(&a_ids, true).unwrap();
    }
    let by_feed = parse(&server.unread_summary_json(&UnreadSummaryParams { by: None }));
    let groups = by_feed["groups"].as_array().unwrap();
    let a = groups.iter().find(|g| g["id"] == serde_json::json!(fx.feed_a)).unwrap();
    assert_eq!(a["unread"], 0, "未读为 0 的源也要出现: {by_feed}");
    assert_eq!(by_feed["total_unread"], 6, "只剩源 B 的 6 条未读");

    let _ = std::fs::remove_file(&fx.db);
}

#[test]
fn invalid_cursor_and_sort_are_reported_not_silently_ignored() {
    let (server, fx) = seeded_fixture("bad-input");

    let bad_cursor = parse(&server.list_articles_json(&ListArticlesParams {
        cursor: Some("bogus".to_string()),
        ..Default::default()
    }));
    assert!(bad_cursor["error"].as_str().unwrap().contains("cursor"), "{bad_cursor}");

    let truncated = parse(&server.list_articles_json(&ListArticlesParams {
        cursor: Some("newest:123".to_string()),
        ..Default::default()
    }));
    assert!(truncated["error"].as_str().unwrap().contains("cursor"), "{truncated}");

    let bad_sort = parse(&server.list_articles_json(&ListArticlesParams {
        sort: Some("latest".to_string()),
        ..Default::default()
    }));
    assert!(bad_sort["error"].as_str().unwrap().contains("newest / oldest / unread_first"), "{bad_sort}");

    // 游标与排序档不匹配：keyset 坐标属于另一套顺序，必须报错而不是翻出错页
    let first = parse(&server.list_articles_json(&ListArticlesParams {
        page_size: Some(2),
        ..Default::default()
    }));
    let cursor = first["next_cursor"].as_str().unwrap().to_string();
    let mismatch = parse(&server.list_articles_json(&ListArticlesParams {
        page_size: Some(2),
        sort: Some("oldest".to_string()),
        cursor: Some(cursor),
        ..Default::default()
    }));
    assert!(mismatch["error"].as_str().unwrap().contains("不匹配"), "{mismatch}");

    let _ = std::fs::remove_file(&fx.db);
}
