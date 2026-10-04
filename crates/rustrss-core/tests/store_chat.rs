use rusqlite::{params, Connection};
use rustrss_core::ai::chat::{ChatBlock, ChatUsage};
use rustrss_core::store::schema::{BASELINE_VERSION, MIGRATIONS};
use rustrss_core::Store;

fn session(store: &Store) -> i64 {
    store
        .create_session("日报追问", "ollama", "model", "endpoint-hash", "{}")
        .unwrap()
}
fn parts(text: &str) -> String {
    serde_json::to_string(&vec![ChatBlock::Text(text.into())]).unwrap()
}

#[test]
fn v2_migrates_to_v3_and_reopens_without_losing_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.db");
    let db = Connection::open(&path).unwrap();
    for migration in &MIGRATIONS[..2] {
        db.execute_batch(migration).unwrap();
    }
    db.pragma_update(None, "user_version", 2).unwrap();
    db.execute(
        "INSERT INTO feeds(url,title,created_at) VALUES ('https://test/feed','old',1)",
        [],
    )
    .unwrap();
    drop(db);
    let store = Store::open(&path).unwrap();
    assert_eq!(BASELINE_VERSION, 1);
    assert_eq!(store.schema_version().unwrap(), 3);
    assert_eq!(store.list_feeds().unwrap()[0].title, "old");
    let id = session(&store);
    store
        .append_message(
            id,
            "user",
            "done",
            &parts("持久正文"),
            &ChatUsage::default(),
        )
        .unwrap();
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), 3);
    assert_eq!(
        reopened.get_session(id).unwrap().unwrap().messages[0].parts_json,
        parts("持久正文")
    );
}

#[test]
fn crud_roundtrip_seq_unique_and_all_cascades() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.db");
    let store = Store::open(&path).unwrap();
    let id = session(&store);
    let other = session(&store);
    let usage = ChatUsage {
        input_tokens: Some(12),
        output_tokens: Some(7),
    };
    let (first, seq1) = store
        .append_message(id, "user", "done", &parts("问题"), &ChatUsage::default())
        .unwrap();
    let (second, seq2) = store
        .append_message(id, "assistant", "done", &parts("答案"), &usage)
        .unwrap();
    assert_eq!((seq1, seq2), (1, 2));
    store.update_session_title(id, "改名").unwrap();
    store.update_session_touch(id).unwrap();
    let history = store.get_session(id).unwrap().unwrap();
    assert_eq!(history.session.title, "改名");
    assert_eq!(history.session.provider, "ollama");
    assert_eq!(history.session.model, "model");
    assert_eq!(history.session.endpoint_id, "endpoint-hash");
    assert_eq!(history.session.scope_json, "{}");
    assert_eq!(
        history.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(history.messages[1].usage, usage);
    let db = Connection::open(&path).unwrap();
    db.execute("UPDATE chat_sessions SET updated_at=1 WHERE id=?1", [other])
        .unwrap();
    assert_eq!(store.list_sessions().unwrap()[0].id, id);
    assert!(db.execute("INSERT INTO chat_messages(session_id,seq,role,status,created_at) VALUES (?1,1,'user','done',1)",[id]).is_err());
    db.execute("INSERT INTO chat_sources(message_id,source_key,kind,source_identity,revision,hash,title,excerpt) VALUES (?1,'digest:1','digest','1',1,'hash','title','excerpt')",[second]).unwrap();
    assert!(store.delete_session(id).unwrap());
    assert!(!store.delete_session(id).unwrap());
    assert!(store.get_session(id).unwrap().is_none());
    for table in ["chat_messages", "chat_message_bodies", "chat_sources"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(store.list_sessions().unwrap()[0].id, other);
}

#[test]
fn startup_recovery_is_explicit_idempotent_and_preserves_terminal_messages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.db");
    let store = Store::open(&path).unwrap();
    let id = session(&store);
    store
        .append_message(
            id,
            "user",
            "running",
            &parts("未完成"),
            &ChatUsage::default(),
        )
        .unwrap();
    for status in ["done", "failed", "cancelled", "interrupted"] {
        store
            .append_message(
                id,
                "assistant",
                status,
                &parts(status),
                &ChatUsage::default(),
            )
            .unwrap();
    }
    drop(store);
    let store = Store::open(&path).unwrap();
    // 次要 Store 连接不能误伤活跃回合，宿主启动显式恢复。
    assert_eq!(
        store.get_session(id).unwrap().unwrap().messages[0].status,
        "running"
    );
    assert_eq!(store.mark_running_interrupted().unwrap(), 1);
    assert_eq!(store.mark_running_interrupted().unwrap(), 0);
    let history = store.get_session(id).unwrap().unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .map(|m| m.status.as_str())
            .collect::<Vec<_>>(),
        vec!["interrupted", "done", "failed", "cancelled", "interrupted"]
    );
    assert_eq!(history.messages[0].parts_json, parts("未完成"));
}

#[test]
fn database_single_flight_and_atomic_completion_reject_late_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.db");
    let first = Store::open(&path).unwrap();
    let second = Store::open(&path).unwrap();
    let id = session(&first);
    let (user, _) = first.chat_begin_message(id, &parts("问")).unwrap();
    assert!(second.chat_begin_message(id, &parts("重复")).is_err());
    let assistant = first
        .chat_finish_message(id, user, "done", &parts("答"), &ChatUsage::default())
        .unwrap();
    assert!(!first.chat_has_running(id).unwrap());
    assert!(second
        .chat_finish_message(id, user, "done", &parts("迟到"), &ChatUsage::default())
        .is_err());
    let history = first.get_session(id).unwrap().unwrap();
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].id, assistant);
    assert!(history.messages.iter().all(|m| m.run_id == Some(user)));
    let (next, _) = second.chat_begin_message(id, &parts("追问")).unwrap();
    first.chat_end_running(user, "interrupted").unwrap();
    assert!(second.chat_has_running(id).unwrap());
    second.chat_end_running(next, "cancelled").unwrap();
    assert!(!first.chat_has_running(id).unwrap());
    let (last, _) = first.chat_begin_message(id, &parts("落库失败")).unwrap();
    assert!(first
        .chat_finish_message(id, last, "running", "[]", &ChatUsage::default())
        .is_err());
    assert!(first
        .chat_finish_message(
            id,
            last,
            "done",
            "[]",
            &ChatUsage {
                input_tokens: Some(u64::MAX),
                output_tokens: None
            }
        )
        .is_err());
    assert!(
        first.chat_has_running(id).unwrap(),
        "终态和正文写入应同事务回滚"
    );
    first.chat_end_running(last, "interrupted").unwrap();
}

#[test]
fn append_failure_rolls_back_metadata_and_sequence() {
    let store = Store::open_in_memory().unwrap();
    let id = session(&store);
    assert!(store
        .append_message(id, "invalid", "done", "[]", &ChatUsage::default())
        .is_err());
    assert!(store
        .append_message(999, "user", "done", "[]", &ChatUsage::default())
        .is_err());
    assert!(store
        .append_message(
            id,
            "user",
            "done",
            "[]",
            &ChatUsage {
                input_tokens: Some(u64::MAX),
                output_tokens: None
            }
        )
        .is_err());
    assert_eq!(
        store
            .append_message(id, "user", "done", "[]", &ChatUsage::default())
            .unwrap()
            .1,
        1
    );
}

#[test]
fn sources_excerpt_byte_gate_and_running_query_uses_partial_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.db");
    let store = Store::open(&path).unwrap();
    let id = session(&store);
    let (message, _) = store
        .append_message(id, "user", "done", "[]", &ChatUsage::default())
        .unwrap();
    let db = Connection::open(&path).unwrap();
    assert!(db.execute("INSERT INTO chat_sources(message_id,source_key,kind,source_identity,hash,title,excerpt) VALUES (?1,'k','digest','1','h','t',?2)",params![message,"中".repeat(4097)]).is_err());
    let plan = store.explain_chat_running(id).unwrap().join(" ");
    assert!(plan.contains("idx_chat_running"), "{plan}");
    db.execute("DROP INDEX idx_chat_running", []).unwrap();
    // 新连接避免 SQLite EXPLAIN 缓存保留旧 schema 的计划文本。
    let without_index = Store::open(&path).unwrap();
    assert!(!without_index
        .explain_chat_running(id)
        .unwrap()
        .join(" ")
        .contains("idx_chat_running"));
}

/// 在飞会话删除保护在 core 层生效（评审 P1）：running 拒绝且数据不变；
/// 标记 interrupted（终态）后删除成功；跨连接读取一致。
#[test]
fn delete_session_rejects_inflight_and_allows_terminal() {
    let store = Store::open_in_memory().unwrap();
    let sid = store.create_session("标题", "openai", "gpt-x", "ep", "{}").unwrap();
    let no_usage = ChatUsage::default();
    let (msg, _seq) = store
        .append_message(sid, "user", "running", r#"{"parts":[]}"#, &no_usage)
        .unwrap();

    // running：拒绝，且会话与消息原样保留
    let err = store.delete_session(sid).unwrap_err();
    assert!(err.to_string().contains("正在生成中"), "{err}");
    let session = store.get_session(sid).unwrap().unwrap();
    assert!(!session.messages.is_empty(), "会话未被删除");
    let still = session.messages.iter().find(|m| m.id == msg).unwrap();
    assert_eq!(still.status, "running");

    // 终态（interrupted）：删除成功且级联清空
    store.mark_running_interrupted().unwrap();
    assert!(store.delete_session(sid).unwrap());
    assert!(store.get_session(sid).unwrap().is_none());
}

/// 跨连接一致性：文件库上第二个 Store 实例的删除同样受 core 防护约束。
#[test]
fn delete_session_guard_is_cross_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.sqlite");
    let store = Store::open(&path).unwrap();
    let sid = store.create_session("标题", "openai", "gpt-x", "ep", "{}").unwrap();
    let no_usage = ChatUsage::default();
    store
        .append_message(sid, "user", "running", r#"{"parts":[]}"#, &no_usage)
        .unwrap();

    let other = Store::open(&path).unwrap(); // 模拟另一连接/进程
    let err = other.delete_session(sid).unwrap_err();
    assert!(err.to_string().contains("正在生成中"));
    // 评审 P2 note：由原连接标记终态、另一连接执行删除——真实跨连接形态
    store.mark_running_interrupted().unwrap();
    drop(store);
    assert!(other.delete_session(sid).unwrap());
}
