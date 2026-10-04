use rustrss_core::ai::tools::{project_tool, run_tool, TOOL_BYTES};
use rustrss_core::Store;
use rustrss_mcp::tag_tools::ListTagsParams;
use rustrss_mcp::{
    DigestGetParams, DigestListParams, GetArticleParams, ListArticlesParams, RustRssMcp,
    SearchParams, UnreadSummaryParams,
};
use serde_json::{json, Value};

#[test]
fn all_ten_mcp_wrappers_share_exact_core_projections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.sqlite");
    let store = Store::open(&path).unwrap();
    let feed = store
        .add_feed("https://example.invalid/feed", Some("Feed"))
        .unwrap();
    let entries = rustrss_core::parse(b"<rss version='2.0'><channel><title>Feed</title><item><guid>a</guid><title>Rust</title><description>Rust body</description><pubDate>2026-10-04T01:00:00Z</pubDate></item></channel></rss>").unwrap().entries;
    store.upsert_entries(feed, &entries).unwrap();
    store.create_tag("Rust", None).unwrap();
    let id = store.list_entries(&Default::default()).unwrap()[0].id;
    let manifest = store.freeze_manifest(1791072000, 1791122400, None).unwrap();
    store
        .commit_digest_report(
            "2026-10-04",
            "UTC",
            1791072000,
            1791122400,
            "all",
            "{}",
            "p",
            "{}",
            &manifest.hash,
            &manifest.pairs_hash,
            &[],
            manifest.frozen_at,
            0,
            0,
            "{\"overview\":\"Rust\",\"sections\":[]}",
            "# Rust report",
            1,
            "{}",
            manifest.entries.len() as i64,
            &manifest.entries,
        )
        .unwrap();
    let mcp = RustRssMcp::open(&path).unwrap();
    let cases = [
        ("list_feeds", json!({}), mcp.list_feeds_json()),
        ("list_folders", json!({}), mcp.list_folders_json()),
        (
            "list_articles",
            json!({}),
            mcp.list_articles_json(&ListArticlesParams::default()),
        ),
        (
            "get_article",
            json!({"id":id}),
            mcp.get_article_json(&GetArticleParams {
                id,
                include_html: false,
            }),
        ),
        (
            "search_articles",
            json!({"query":"Rust"}),
            mcp.search_articles_json(&SearchParams {
                query: "Rust".into(),
                limit: None,
            }),
        ),
        (
            "get_unread_summary",
            json!({}),
            mcp.unread_summary_json(&UnreadSummaryParams::default()),
        ),
        ("db_stats", json!({}), mcp.db_stats_json()),
        (
            "list_tags",
            json!({}),
            mcp.list_tags_json(&ListTagsParams::default()),
        ),
        (
            "digest_list",
            json!({}),
            mcp.digest_list_json(&DigestListParams::default()),
        ),
        (
            "digest_get",
            json!({"date":"2026-10-04"}),
            mcp.digest_get_json(&DigestGetParams {
                date: "2026-10-04".into(),
                scope_key: None,
                items_page_size: None,
                items_page: None,
            }),
        ),
    ];
    for (name, args, mcp_json) in cases {
        assert_eq!(
            project_tool(&store, name, &args),
            mcp_json,
            "{name}: exact MCP bytes"
        );
        let bounded = run_tool(&store, name, &args).unwrap();
        assert!(!bounded.truncated, "{name}");
        assert_eq!(
            bounded.data_json, mcp_json,
            "{name}: bounded small projection"
        );
        assert_eq!(
            serde_json::from_str::<Value>(&bounded.data_json).unwrap(),
            serde_json::from_str::<Value>(&mcp_json).unwrap()
        );
    }
    assert!(!store.get_entry(id).unwrap().unwrap().read);
}

#[test]
fn large_chat_result_is_utf8_prefix_of_unchanged_mcp_projection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.sqlite");
    let store = Store::open(&path).unwrap();
    // Large metadata exercises the common projection byte gate independently
    // of the article SQL excerpt gate.
    for i in 0..100 {
        store
            .add_feed(
                &format!("https://example.invalid/{i}"),
                Some(&"界\\\"".repeat(100)),
            )
            .unwrap();
    }
    let mcp = RustRssMcp::open(&path).unwrap();
    let unbounded = mcp.list_feeds_json();
    assert_eq!(unbounded, project_tool(&store, "list_feeds", &json!({})));
    let bounded = run_tool(&store, "list_feeds", &json!({})).unwrap();
    assert!(bounded.truncated);
    assert!(bounded.data_json.len() <= TOOL_BYTES);
    let value: Value = serde_json::from_str(&bounded.data_json).unwrap();
    assert_eq!(value["truncated"], true);
    assert!(unbounded.starts_with(value["excerpt"].as_str().unwrap()));
}
