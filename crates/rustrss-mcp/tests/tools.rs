//! MCP 工具层测试：直接调工具方法，不经过 stdio。
//!
//! 重点是「列表不回正文」这条口径——它决定了 agent 的上下文会不会被撑爆。

use rustrss_core::{Entry, IdOrigin, Store};
use rustrss_mcp::{GetArticleParams, ListArticlesParams, RustRssMcp, SearchParams};

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
        feed_id: None,
        unread_only: true,
        starred_only: false,
        limit: Some(5),
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
        feed_id: None,
        unread_only: false,
        starred_only: false,
        limit: Some(10),
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
        feed_id: None,
        unread_only: false,
        starred_only: false,
        limit: Some(9999),
    }));
    assert_eq!(clamped["count"], 2);
    let _ = std::fs::remove_file(db);
}
