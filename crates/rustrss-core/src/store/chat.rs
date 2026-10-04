//! 持久聊天：列表只读元数据，历史按序 JOIN 正文；不保存凭据。
use super::{Result, Store, StoreError};
use crate::ai::chat::ChatUsage;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatSessionRow {
    pub id: i64,
    pub title: String,
    pub provider: String,
    pub model: String,
    pub endpoint_id: String,
    pub scope_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatMessageRow {
    pub id: i64,
    pub session_id: i64,
    pub seq: i64,
    pub run_id: Option<i64>,
    pub role: String,
    pub status: String,
    pub usage: ChatUsage,
    pub created_at: i64,
    pub parts_json: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatSession {
    pub session: ChatSessionRow,
    pub messages: Vec<ChatMessageRow>,
}

const CHAT_RUNNING_SQL: &str =
    "SELECT EXISTS(SELECT 1 FROM chat_messages WHERE session_id=?1 AND status='running')";
const SESSION_COLUMNS: &str =
    "id,title,provider,model,endpoint_id,scope_json,created_at,updated_at";
fn session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatSessionRow> {
    Ok(ChatSessionRow {
        id: r.get(0)?,
        title: r.get(1)?,
        provider: r.get(2)?,
        model: r.get(3)?,
        endpoint_id: r.get(4)?,
        scope_json: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

fn append_on(
    conn: &rusqlite::Connection,
    session_id: i64,
    role: &str,
    status: &str,
    parts_json: &str,
    usage: &ChatUsage,
) -> Result<(i64, i64)> {
    let seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq),0)+1 FROM chat_messages WHERE session_id=?1",
        [session_id],
        |r| r.get(0),
    )?;
    conn.execute("INSERT INTO chat_messages(session_id,seq,role,status,input_tokens,output_tokens,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![session_id,seq,role,status,usage.input_tokens.map(i64::try_from).transpose().map_err(|_| StoreError::Invalid("input_tokens 超出 SQLite 范围".into()))?,usage.output_tokens.map(i64::try_from).transpose().map_err(|_| StoreError::Invalid("output_tokens 超出 SQLite 范围".into()))?,Utc::now().timestamp_millis()])?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chat_message_bodies(message_id,parts_json) VALUES (?1,?2)",
        params![id, parts_json],
    )?;
    conn.execute(
        "UPDATE chat_sessions SET updated_at=?2 WHERE id=?1",
        params![session_id, Utc::now().timestamp_millis()],
    )?;
    Ok((id, seq))
}

impl Store {
    pub fn create_session(
        &self,
        title: &str,
        provider: &str,
        model: &str,
        endpoint_id: &str,
        scope_json: &str,
    ) -> Result<i64> {
        self.create_session_with_seed(title, provider, model, endpoint_id, scope_json, None)
    }

    pub(crate) fn create_session_with_seed(
        &self,
        title: &str,
        provider: &str,
        model: &str,
        endpoint_id: &str,
        scope_json: &str,
        seed: Option<&str>,
    ) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let now = Utc::now().timestamp_millis();
        tx.execute("INSERT INTO chat_sessions(title,provider,model,endpoint_id,scope_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?6)", params![title,provider,model,endpoint_id,scope_json,now])?;
        let id = tx.last_insert_rowid();
        if let Some(seed) = seed {
            let parts = serde_json::to_string(&vec![crate::ai::chat::ChatBlock::Text(seed.into())])
                .map_err(|e| StoreError::Invalid(e.to_string()))?;
            append_on(&tx, id, "user", "done", &parts, &ChatUsage::default())?;
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn append_message(
        &self,
        session_id: i64,
        role: &str,
        status: &str,
        parts_json: &str,
        usage: &ChatUsage,
    ) -> Result<(i64, i64)> {
        let tx = self.conn.unchecked_transaction()?;
        let result = append_on(&tx, session_id, role, status, parts_json, usage)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn update_session_touch(&self, session_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE chat_sessions SET updated_at=?2 WHERE id=?1",
            params![session_id, Utc::now().timestamp_millis()],
        )?;
        Ok(())
    }

    pub fn update_session_title(&self, session_id: i64, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE chat_sessions SET title=?2,updated_at=?3 WHERE id=?1",
            params![session_id, title, Utc::now().timestamp_millis()],
        )?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<ChatSessionRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM chat_sessions ORDER BY updated_at DESC,id"
        ))?;
        let rows = stmt
            .query_map([], session_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_session(&self, id: i64) -> Result<Option<ChatSession>> {
        self.get_session_page(id, None, u32::MAX)
    }

    /// Latest bounded page, or messages strictly before the oldest displayed seq.
    /// Returned in ascending order; model history continues using get_session.
    pub fn get_session_page(
        &self,
        id: i64,
        since_seq: Option<i64>,
        limit: u32,
    ) -> Result<Option<ChatSession>> {
        let tx = self.conn.unchecked_transaction()?;
        let session = tx
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM chat_sessions WHERE id=?1"),
                [id],
                session_row,
            )
            .optional()?;
        let Some(session) = session else {
            return Ok(None);
        };
        let messages = {
            let mut stmt = tx.prepare("SELECT m.id,m.session_id,m.seq,m.run_id,m.role,m.status,m.input_tokens,m.output_tokens,m.created_at,b.parts_json FROM chat_messages m JOIN chat_message_bodies b ON b.message_id=m.id WHERE m.session_id=?1 AND m.seq < ?2 ORDER BY m.seq DESC LIMIT ?3")?;
            let rows = stmt
                .query_map(
                    params![id, since_seq.unwrap_or(i64::MAX), limit.max(1)],
                    |r| {
                        Ok(ChatMessageRow {
                            id: r.get(0)?,
                            session_id: r.get(1)?,
                            seq: r.get(2)?,
                            run_id: r.get(3)?,
                            role: r.get(4)?,
                            status: r.get(5)?,
                            usage: ChatUsage {
                                input_tokens: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                                output_tokens: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                            },
                            created_at: r.get(8)?,
                            parts_json: r.get(9)?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<_>>()?;
            let mut rows: Vec<ChatMessageRow> = rows;
            rows.reverse();
            rows
        };
        tx.commit()?;
        Ok(Some(ChatSession { session, messages }))
    }

    /// 删除会话（级联消息/正文/引用）。业务规则在 core：**在飞（running）会话
    /// 拒绝删除**——同一事务内检查，删除/迟到响应无竞态（评审 P1：此前只由
    /// Tauri 壳层检查进程内任务，core CRUD 本身不设防）。
    pub fn delete_session(&self, id: i64) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let running: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_messages
                                WHERE session_id = ?1 AND status = 'running')",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v != 0)?;
        if running {
            return Err(super::StoreError::Invalid(
                "会话正在生成中，请先停止再删除".into(),
            ));
        }
        let deleted = tx.execute("DELETE FROM chat_sessions WHERE id=?1", [id])? > 0;
        tx.commit()?;
        Ok(deleted)
    }

    /// 宿主启动时调用；普通 Store::open 不恢复，避免 MCP 次要连接误伤在飞任务。
    pub fn mark_running_interrupted(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE chat_messages SET status='interrupted' WHERE status='running'",
            [],
        )?)
    }

    pub fn chat_has_running(&self, session_id: i64) -> Result<bool> {
        Ok(self
            .conn
            .query_row(CHAT_RUNNING_SQL, [session_id], |r| r.get(0))?)
    }

    #[doc(hidden)]
    pub fn explain_chat_running(&self, session_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {CHAT_RUNNING_SQL}"))?;
        let rows = stmt.query_map([session_id], |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 只转移本回合的 running 行；迟到任务不能清理后来回合。
    pub fn chat_end_running(&self, message_id: i64, status: &str) -> Result<()> {
        if status == "running" {
            return Err(StoreError::Invalid("结束状态不能是 running".into()));
        }
        self.conn.execute(
            "UPDATE chat_messages SET status=?2 WHERE id=?1 AND status='running'",
            params![message_id, status],
        )?;
        Ok(())
    }

    /// 原子受理，同库多个连接也不能重入；事务先查 running 再写入。
    pub fn chat_begin_message(&self, session_id: i64, parts_json: &str) -> Result<(i64, i64)> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let running: bool = tx.query_row(CHAT_RUNNING_SQL, [session_id], |r| r.get(0))?;
        if running {
            return Err(StoreError::Invalid("该会话正在回答中".into()));
        }
        let result = append_on(
            &tx,
            session_id,
            "user",
            "running",
            parts_json,
            &ChatUsage::default(),
        )?;
        tx.execute("UPDATE chat_messages SET run_id=id WHERE id=?1", [result.0])?;
        tx.commit()?;
        Ok(result)
    }

    /// 回合落库与 running 清理同事务；已被恢复/清理的迟到回合拒绝落库。
    pub fn chat_finish_message(
        &self,
        session_id: i64,
        user_id: i64,
        status: &str,
        parts_json: &str,
        usage: &ChatUsage,
    ) -> Result<i64> {
        if status == "running" {
            return Err(StoreError::Invalid("结束状态不能是 running".into()));
        }
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE chat_messages SET status=?3 WHERE id=?1 AND session_id=?2 AND status='running'",
            params![user_id, session_id, status],
        )?;
        if changed == 0 {
            return Err(StoreError::Invalid("该回合已结束".into()));
        }
        let (id, _) = append_on(&tx, session_id, "assistant", status, parts_json, usage)?;
        tx.execute(
            "UPDATE chat_messages SET run_id=?2 WHERE id=?1",
            params![id, user_id],
        )?;
        tx.commit()?;
        Ok(id)
    }
}
