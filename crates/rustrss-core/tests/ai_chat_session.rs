use rusqlite::{params, Connection};
use rustrss_core::ai::chat::{digest_chat_seed, ChatBlock, ChatUsage};
use rustrss_core::ai::chat_session::{prepare_chat_turn, MAX_CHAT_INPUT_CHARS};
use rustrss_core::ai::{AiClient, AiConfig};
use rustrss_core::Store;

fn client() -> AiClient {
    AiClient::new(AiConfig::ollama("model")).unwrap()
}
fn parts(text: &str) -> String {
    serde_json::to_string(&vec![ChatBlock::Text(text.into())]).unwrap()
}
fn save_report(db: &Connection, markdown: &str) {
    db.execute("INSERT INTO digests(report_day,timezone_label,day_start_at,day_end_at,utc_offset_start,utc_offset_end,scope_key,scope_json,profile_key,profile_json,revision,generated_at,checkpoint_at,manifest_hash,article_count,created_at) VALUES('2026-10-04','UTC',1000,2000,0,0,'tags:3','{\"tag_ids\":[3]}','p','{}',1,10,9,'manifest',1,10)",[]).unwrap();
    db.execute("INSERT INTO digest_bodies(digest_id,revision,schema_ver,content_json,markdown) VALUES(?1,1,1,'{}',?2)",params![db.last_insert_rowid(),markdown]).unwrap();
}

#[test]
fn seed_missing_report_allows_no_report_session() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(digest_chat_seed(&store, "2026-10-04", "all").unwrap(), None);
    let turn =
        prepare_chat_turn(&store, &client(), None, Some("2026-10-04"), None, "问题").unwrap();
    assert_eq!(turn.request.messages.len(), 1);
    assert!(turn.request.system.unwrap().contains("没有日报"));
    assert_eq!(turn.request.tools.len(), 10);
    assert_eq!(turn.request.limits.max_model_requests, 6);
    assert_eq!(turn.request.limits.max_tool_calls, 10);
    assert_eq!(turn.scope_key, "all");
    assert!(store.chat_has_running(turn.session_id).unwrap());
}

#[test]
fn digest_seed_is_truncated_and_report_snapshot_survives_rewrite_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.db");
    let store = Store::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    save_report(&db, &"原".repeat(12_001));
    let (system, context) = digest_chat_seed(&store, "2026-10-04", "tags:3")
        .unwrap()
        .unwrap();
    assert!(system.contains("不编造"));
    assert!(context.contains("已截断"));
    assert_eq!(context.chars().filter(|c| *c == '原').count(), 12_000);
    let turn = prepare_chat_turn(
        &store,
        &client(),
        None,
        Some("2026-10-04"),
        Some("tags:3"),
        "解释日报",
    )
    .unwrap();
    assert_eq!(
        turn.request.messages[0].blocks,
        vec![ChatBlock::Text(context.clone())]
    );
    store
        .chat_finish_message(
            turn.session_id,
            turn.message_id,
            "done",
            &parts("回答"),
            &ChatUsage::default(),
        )
        .unwrap();
    db.execute("UPDATE digest_bodies SET markdown='新版报告'", [])
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let next =
        prepare_chat_turn(&store, &client(), Some(turn.session_id), None, None, "追问").unwrap();
    assert_eq!(
        next.request.messages[0].blocks,
        vec![ChatBlock::Text(context)]
    );
    assert_eq!(next.request.messages.len(), 4);
    let scope: serde_json::Value = serde_json::from_str(
        &store
            .get_session(turn.session_id)
            .unwrap()
            .unwrap()
            .session
            .scope_json,
    )
    .unwrap();
    assert_eq!(scope["scope_key"], "tags:3");
    assert_eq!(scope["has_seed"], true);
    assert!(
        scope.get("user_context").is_none(),
        "日报正文不进入会话列表元数据"
    );
    let history = store.get_session(turn.session_id).unwrap().unwrap();
    assert_eq!(history.messages[0].seq, 1);
    assert_eq!(history.messages[0].status, "done");
    assert_eq!(scope["report_checkpoint_at"], 9);
    assert_eq!(scope["report_scope_json"], "{\"tag_ids\":[3]}");
    assert!(scope["report_hash"].as_str().unwrap().len() == 64);
}

#[test]
fn complete_turn_history_replay_excludes_failed_cancelled_and_interrupted() {
    let store = Store::open_in_memory().unwrap();
    let client = client();
    let first = prepare_chat_turn(&store, &client, None, None, None, "成功问题").unwrap();
    store
        .chat_finish_message(
            first.session_id,
            first.message_id,
            "done",
            &parts("成功答案"),
            &ChatUsage::default(),
        )
        .unwrap();
    for status in ["failed", "cancelled"] {
        let turn = prepare_chat_turn(
            &store,
            &client,
            Some(first.session_id),
            None,
            None,
            "失败问题",
        )
        .unwrap();
        store
            .chat_finish_message(
                turn.session_id,
                turn.message_id,
                status,
                &parts("失败答案"),
                &ChatUsage::default(),
            )
            .unwrap();
    }
    let interrupted = prepare_chat_turn(
        &store,
        &client,
        Some(first.session_id),
        None,
        None,
        "崩溃问题",
    )
    .unwrap();
    store
        .chat_end_running(interrupted.message_id, "interrupted")
        .unwrap();
    let next =
        prepare_chat_turn(&store, &client, Some(first.session_id), None, None, "追问").unwrap();
    assert_eq!(next.request.messages.len(), 3);
    assert_eq!(
        next.request.messages[0].blocks,
        vec![ChatBlock::Text("成功问题".into())]
    );
    assert_eq!(
        next.request.messages[1].blocks,
        vec![ChatBlock::Text("成功答案".into())]
    );
}

#[test]
fn session_single_flight_config_drift_and_unknown_session_rejected() {
    let store = Store::open_in_memory().unwrap();
    let client = client();
    let first = prepare_chat_turn(&store, &client, None, None, None, "你好").unwrap();
    assert!(
        prepare_chat_turn(&store, &client, Some(first.session_id), None, None, "并发")
            .unwrap_err()
            .to_string()
            .contains("正在回答")
    );
    store
        .chat_end_running(first.message_id, "cancelled")
        .unwrap();
    for cfg in [
        AiConfig::ollama("other-model"),
        AiConfig::ollama("model").with_base_url("http://127.0.0.1:9999"),
    ] {
        assert!(prepare_chat_turn(
            &store,
            &AiClient::new(cfg).unwrap(),
            Some(first.session_id),
            None,
            None,
            "漂移"
        )
        .unwrap_err()
        .to_string()
        .contains("请开启新会话"));
    }
    assert!(prepare_chat_turn(&store, &client, Some(999), None, None, "不存在").is_err());
    assert_eq!(
        store
            .get_session(first.session_id)
            .unwrap()
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[test]
fn input_budget_rejects_without_writes_and_history_trims_whole_turns() {
    let store = Store::open_in_memory().unwrap();
    let client = client();
    assert!(prepare_chat_turn(&store, &client, None, None, None, " ").is_err());
    assert!(prepare_chat_turn(
        &store,
        &client,
        None,
        None,
        None,
        &"中".repeat(MAX_CHAT_INPUT_CHARS)
    )
    .is_err());
    assert!(store.list_sessions().unwrap().is_empty());
    let first = prepare_chat_turn(&store, &client, None, None, None, &"甲".repeat(20_000)).unwrap();
    store
        .chat_finish_message(
            first.session_id,
            first.message_id,
            "done",
            &parts(&"乙".repeat(20_000)),
            &ChatUsage::default(),
        )
        .unwrap();
    let second = prepare_chat_turn(
        &store,
        &client,
        Some(first.session_id),
        None,
        None,
        "最近问题",
    )
    .unwrap();
    store
        .chat_finish_message(
            second.session_id,
            second.message_id,
            "done",
            &parts("最近答案"),
            &ChatUsage::default(),
        )
        .unwrap();
    let next = prepare_chat_turn(
        &store,
        &client,
        Some(first.session_id),
        None,
        None,
        &"丙".repeat(10_000),
    )
    .unwrap();
    assert!(next.history_trimmed);
    assert!(next
        .request
        .system
        .unwrap()
        .contains("完整回合已因上下文预算省略"));
    assert_eq!(next.request.messages.len(), 3);
    assert_eq!(
        next.request.messages[0].blocks,
        vec![ChatBlock::Text("最近问题".into())]
    );
    assert_eq!(
        next.request.messages[1].blocks,
        vec![ChatBlock::Text("最近答案".into())]
    );
}

#[test]
fn session_provider_usage_budget_rejects_but_unknown_usage_is_not_invented() {
    let store = Store::open_in_memory().unwrap();
    let client = client();
    let first = prepare_chat_turn(&store, &client, None, None, None, "问题").unwrap();
    store
        .chat_finish_message(
            first.session_id,
            first.message_id,
            "done",
            &parts("答"),
            &ChatUsage {
                input_tokens: Some(196_000),
                output_tokens: Some(4_000),
            },
        )
        .unwrap();
    assert!(
        prepare_chat_turn(&store, &client, Some(first.session_id), None, None, "追问")
            .unwrap_err()
            .to_string()
            .contains("达到会话预算")
    );
    assert!(!store.chat_has_running(first.session_id).unwrap());
    let row = store.get_session(first.session_id).unwrap().unwrap();
    assert_eq!(row.messages[0].usage, ChatUsage::default());
}
