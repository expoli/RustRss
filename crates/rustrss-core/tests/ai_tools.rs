use rustrss_core::ai::tools::{
    bound_output, chat_tools, run_scoped_tool, run_tool, validate_tool, ToolOutput, TOOL_BYTES,
};
use rustrss_core::{EntryQuery, Store};
use serde_json::{json, Value};

#[test]
fn whitelist_required_types_enums_and_unknown_tools_are_enforced() {
    assert_eq!(chat_tools().len(), 10);
    for forbidden in [
        "refresh",
        "get_theme",
        "fetch_fulltext",
        "set_read",
        "digest_generate",
        "sql",
        "url",
    ] {
        assert_eq!(
            validate_tool(forbidden, &json!({})).unwrap_err().code,
            "unknown_tool"
        );
    }
    for (name, args) in [
        ("get_article", json!({})),
        ("get_article", json!({"id":"1"})),
        ("list_articles", json!({"sort":"bad"})),
        ("search_articles", json!([])),
        ("list_feeds", json!({"scope":"all"})),
        ("list_tags", json!({"sort":"newest"})),
    ] {
        assert_eq!(
            validate_tool(name, &args).unwrap_err().code,
            "invalid_argument"
        );
    }
    let store = Store::open_in_memory().unwrap();
    assert!(run_tool(&store, "list_articles", &json!({"limit":-1})).is_err());
    assert!(run_tool(&store, "list_articles", &json!({"feed_id":1,"folder_id":1})).is_err());
}

fn fixture() -> (Store, i64, i64, i64) {
    let store = Store::open_in_memory().unwrap();
    let a = store
        .add_feed("https://a.invalid", Some("allowed"))
        .unwrap();
    let b = store.add_feed("https://b.invalid", Some("secret")).unwrap();
    let tag = store.create_tag("scope", None).unwrap().id;
    store.set_feed_tags(a, &[tag]).unwrap();
    for (feed, title) in [(a, "allowed"), (b, "secret")] {
        let xml = format!("<rss version='2.0'><channel><title>T</title><item><guid>{title}</guid><title>Rust {title}</title><description>Rust {title} {}</description></item></channel></rss>", "界".repeat(20_000));
        store
            .upsert_entries(feed, &rustrss_core::parse(xml.as_bytes()).unwrap().entries)
            .unwrap();
    }
    let aid = store
        .list_entries(&EntryQuery {
            feed_id: Some(a),
            ..Default::default()
        })
        .unwrap()[0]
        .id;
    let bid = store
        .list_entries(&EntryQuery {
            feed_id: Some(b),
            ..Default::default()
        })
        .unwrap()[0]
        .id;
    (store, tag, aid, bid)
}

#[test]
fn current_feed_tag_scope_cannot_be_overridden_and_never_marks_read() {
    let (store, tag, aid, bid) = fixture();
    let scope = format!("tags:{tag}");
    for name in [
        "list_feeds",
        "list_folders",
        "list_articles",
        "get_unread_summary",
        "db_stats",
        "digest_list",
    ] {
        let out = run_scoped_tool(&store, &scope, name, &json!({})).unwrap();
        assert!(!out.data_json.contains("secret"), "{name}");
        let v: Value = serde_json::from_str(&out.data_json).unwrap();
        assert_eq!(v["scope_feed_count"], 1);
    }
    for query in ["Rust", "界界", "界"] {
        let out =
            run_scoped_tool(&store, &scope, "search_articles", &json!({"query":query})).unwrap();
        assert!(!out.data_json.contains("secret"));
        let v: Value = serde_json::from_str(&out.data_json).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["articles"][0]["id"], aid);
    }
    assert!(run_scoped_tool(&store, &scope, "get_article", &json!({"id":bid})).is_err());
    assert_eq!(
        run_scoped_tool(&store, &scope, "list_tags", &json!({}))
            .unwrap_err()
            .code,
        "scope_unsupported"
    );
    assert!(run_scoped_tool(
        &store,
        &scope,
        "digest_get",
        &json!({"date":"2026-10-04","scope_key":"all"})
    )
    .is_err());
    let all: Value = serde_json::from_str(
        &run_tool(&store, "search_articles", &json!({"query":"Rust"}))
            .unwrap()
            .data_json,
    )
    .unwrap();
    assert_eq!(all["count"], 2);
    let foreign = store.get_entry(bid).unwrap().unwrap().feed_id;
    let out: Value = serde_json::from_str(
        &run_scoped_tool(&store, &scope, "list_articles", &json!({"feed_id":foreign}))
            .unwrap()
            .data_json,
    )
    .unwrap();
    assert_eq!(out["count"], 0);
    for id in [aid, bid] {
        assert!(!store.get_entry(id).unwrap().unwrap().read);
    }
    // Current membership changes do not grant an all-library fallback.
    store
        .set_feed_tags(store.get_entry(aid).unwrap().unwrap().feed_id, &[])
        .unwrap();
    let empty: Value = serde_json::from_str(
        &run_scoped_tool(&store, &scope, "search_articles", &json!({"query":"Rust"}))
            .unwrap()
            .data_json,
    )
    .unwrap();
    assert_eq!(empty["count"], 0);
}

#[test]
fn body_is_clipped_at_sql_read_then_json_byte_gate_and_utf8_is_valid() {
    let (store, _, id, _) = fixture();
    let (row, truncated) = store.get_entry_bounded(id, None, 1200).unwrap().unwrap();
    assert!(truncated);
    assert_eq!(row.content_text.unwrap().chars().count(), 1200);
    let out = run_tool(&store, "get_article", &json!({"id":id,"include_html":true})).unwrap();
    assert!(out.truncated);
    assert!(out.data_json.len() <= TOOL_BYTES);
    assert_eq!(
        serde_json::from_str::<Value>(&out.data_json).unwrap()["truncated"],
        true
    );
    for max in [32, 64, 1024, TOOL_BYTES] {
        let out = bound_output(
            ToolOutput {
                data_json: json!({"text":"界\\\"".repeat(20_000)}).to_string(),
                truncated: false,
            },
            max,
        );
        assert!(out.data_json.len() <= max);
        assert!(out.truncated);
        serde_json::from_str::<Value>(&out.data_json).unwrap();
    }
}

#[test]
fn chat_scrubs_url_credentials_without_changing_legacy_mcp_projection() {
    let store = Store::open_in_memory().unwrap();
    store
        .add_feed(
            "https://alice:secret@example.invalid/feed?token=private",
            Some("A"),
        )
        .unwrap();
    let raw = rustrss_core::ai::tools::project_tool(&store, "list_feeds", &json!({}));
    assert!(raw.contains("secret"));
    assert!(raw.contains("private"));
    let chat = run_tool(&store, "list_feeds", &json!({})).unwrap();
    assert!(!chat.data_json.contains("secret"));
    assert!(!chat.data_json.contains("private"));
    serde_json::from_str::<Value>(&chat.data_json).unwrap();
}

#[test]
fn digest_scope_precedes_date_dedup_and_limit_and_get_injects_default_scope() {
    let store = Store::open_in_memory().unwrap();
    let tag = store.create_tag("scope", None).unwrap().id;
    let scope = format!("tags:{tag}");
    let manifest = store.freeze_manifest(1791072000, 1791122400, None).unwrap();
    for (key, markdown) in [
        (scope.as_str(), "# scoped report"),
        ("all", "# other report"),
    ] {
        store
            .commit_digest_report(
                "2026-10-04",
                "UTC",
                1791072000,
                1791122400,
                key,
                "{}",
                "p",
                "{}",
                &manifest.hash,
                &manifest.pairs_hash,
                &[],
                manifest.frozen_at,
                0,
                0,
                "{\"overview\":\"report\",\"sections\":[]}",
                markdown,
                1,
                "{}",
                0,
                &manifest.entries,
            )
            .unwrap();
    }
    // Latest global per-date variant is all; filtering that page afterwards
    // would silently hide the existing tags report.
    assert_eq!(store.digest_list(1).unwrap()[0].scope_key, "all");
    let list: Value = serde_json::from_str(
        &run_scoped_tool(&store, &scope, "digest_list", &json!({"limit":1}))
            .unwrap()
            .data_json,
    )
    .unwrap();
    assert_eq!(list["count"], 1);
    assert_eq!(list["digests"][0]["scope_key"], scope);
    let get = run_scoped_tool(&store, &scope, "digest_get", &json!({"date":"2026-10-04"})).unwrap();
    assert!(get.data_json.contains("scoped report"));
    assert!(!get.data_json.contains("other report"));
    assert_eq!(
        run_scoped_tool(
            &store,
            &scope,
            "digest_get",
            &json!({"date":"2026-10-04","scope_key":"all"})
        )
        .unwrap_err()
        .code,
        "scope_denied"
    );
}

#[test]
fn digest_chat_body_is_clipped_in_sql_without_loading_structured_content() {
    let store = Store::open_in_memory().unwrap();
    let manifest = store.freeze_manifest(1791072000, 1791122400, None).unwrap();
    let markdown = "界".repeat(20_000);
    let content =
        json!({"overview":"report","sections":[],"extra":"structured".repeat(20_000)}).to_string();
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
            &content,
            &markdown,
            1,
            "{}",
            0,
            &manifest.entries,
        )
        .unwrap();
    let (excerpt, truncated) = store
        .digest_report_bounded("2026-10-04", "all", 1200)
        .unwrap()
        .unwrap();
    assert!(truncated);
    assert_eq!(excerpt.markdown.chars().count(), 1200);
    assert_eq!(excerpt.content_json, "{}");
    assert_eq!(
        store
            .digest_report("2026-10-04", "all")
            .unwrap()
            .unwrap()
            .markdown,
        markdown
    );
    let bounded = run_tool(&store, "digest_get", &json!({"date":"2026-10-04"})).unwrap();
    assert!(bounded.truncated);
    assert!(bounded.data_json.len() <= TOOL_BYTES);
    serde_json::from_str::<Value>(&bounded.data_json).unwrap();
}

#[test]
fn scoped_large_result_retains_scope_count_through_repeated_byte_truncation() {
    let (store, tag, id, _) = fixture();
    let out = run_scoped_tool(
        &store,
        &format!("tags:{tag}"),
        "get_article",
        &json!({"id":id,"include_html":true}),
    )
    .unwrap();
    assert!(out.truncated);
    assert!(out.data_json.len() <= TOOL_BYTES);
    let value: Value = serde_json::from_str(&out.data_json).unwrap();
    assert_eq!(value["scope_feed_count"], 1);
    assert_eq!(value["truncated"], true);
    for max in [64, 128, 1024] {
        let bounded = bound_output(out.clone(), max);
        assert!(bounded.data_json.len() <= max);
        let value: Value = serde_json::from_str(&bounded.data_json).unwrap();
        assert_eq!(value["scope_feed_count"], 1);
        assert_eq!(value["truncated"], true);
    }
}
