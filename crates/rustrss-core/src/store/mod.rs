//! SQLite 存储层。
//!
//! 设计要点：
//! - **去重靠 `UNIQUE(feed_id, stable_id)`**：同一篇文章反复刷新只会更新，不会新增；
//! - **刷新不动状态**：`read` / `starred` 不在 upsert 的更新列里，已读不会被刷回未读；
//! - **无谓写入可跳过**：内容指纹没变就只计数 `unchanged`，不写库；
//! - 正文真身只存一份（FTS5 用外部内容表，不做冗余副本）。
//!
//! 子模块 [`backup`]：库的在线快照导出与「重启时替换」式恢复（含 WAL 边车顺序不变量）。

pub mod backup;
pub mod schema;
pub mod tokens;

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{Entry, IdOrigin};

use schema::MIGRATIONS;
use tokens::{plan_query, to_tokens};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("文件系统错误: {0}")]
    Io(String),
    #[error("数据不合法: {0}")]
    Invalid(String),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

pub(crate) const FOLDERS_COLLAPSED_KEY: &str = "ui.folders_collapsed";

/// 订阅源行查询的共同部分：列清单与聚合口径只写一份，`list_feeds` / `feed_row`
/// 共用（新增列时只改这里，避免两处 SQL 漂移）。
const FEED_ROW_SELECT: &str = "\
    SELECT f.id, f.url, f.title, f.site_url, f.folder_id,
           COALESCE(SUM(CASE WHEN e.read = 0 THEN 1 ELSE 0 END), 0) AS unread,
           f.last_status, f.last_error, f.last_fetched_at, f.refresh_interval_minutes
      FROM feeds f LEFT JOIN entries e ON e.feed_id = f.id";

const FEED_ROW_GROUP_BY: &str = "\
    GROUP BY f.id, f.url, f.title, f.site_url, f.folder_id, f.last_status, f.last_error,
             f.last_fetched_at, f.refresh_interval_minutes";

fn feed_row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<FeedRow> {
    Ok(FeedRow {
        id: r.get(0)?,
        url: r.get(1)?,
        title: r.get(2)?,
        site_url: r.get(3)?,
        folder_id: r.get(4)?,
        unread: r.get(5)?,
        last_status: r.get(6)?,
        last_error: r.get(7)?,
        last_fetched_at: r.get(8)?,
        refresh_interval_minutes: r.get(9)?,
    })
}

/// 订阅源行（带未读数）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeedRow {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub site_url: Option<String>,
    pub folder_id: Option<i64>,
    pub unread: i64,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_fetched_at: Option<i64>,
    /// 每源刷新间隔覆盖（分钟）；`None` = 跟随全局档。
    pub refresh_interval_minutes: Option<i64>,
}

/// 调度扫描行：`(feed_id, 覆盖分钟, 上次抓取时刻)`。
/// 宿主侧按源算到期只用得上这三列，用别名把元组留给类型而不留给调用点。
pub type FeedIntervalRow = (i64, Option<i64>, Option<i64>);

/// 文件夹行（侧栏分组）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FolderRow {
    pub id: i64,
    pub name: String,
    pub position: i64,
}

/// 条目行（列表与阅读页共用）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntryRow {
    pub id: i64,
    pub feed_id: i64,
    pub feed_title: String,
    pub stable_id: String,
    pub id_origin: String,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<i64>,
    pub summary: Option<String>,
    pub content_html: Option<String>,
    pub content_text: Option<String>,
    /// 是否值得显示「获取全文」：摘要型条目（正文缺失或明显偏短）且未抓过、有原文地址。
    ///
    /// 只由阅读页路径（`get_entry`）真判定；列表/搜索的行不带正文，恒为 false
    /// ——所以**不要**用列表行去驱动这个按钮。内部标记位 `fulltext_fetched` 不出库，
    /// 判定结论代替它出库（见 `crate::fulltext::is_summary_entry`）。
    pub needs_fulltext: bool,
    pub read: bool,
    pub starred: bool,
    pub read_later: bool,
    /// 列表排序键 `COALESCE(published_at, fetched_at)`，与 ORDER BY 同一个值直出。
    /// 分页游标由 (sortkey, id) 组成——前端直接取末行，不自己重算表达式（重算
    /// 会与 SQL 侧漂移，且前端根本看不到 fetched_at）。
    pub sortkey: i64,
}

/// 一次入库的统计：用于验证「重复刷新不产生重复条目」
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct InsertStats {
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
}

/// 条目列表查询条件
#[derive(Debug, Clone, Default)]
pub struct EntryQuery {
    pub feed_id: Option<i64>,
    pub unread_only: bool,
    pub starred_only: bool,
    pub read_later_only: bool,
    pub limit: Option<u32>,
    /// keyset 续扫游标：上一页末行的 `(sortkey, id)`（`EntryRow::sortkey` 直出）。
    /// `None` = 取首页；只返回排在游标之后的行，因此与 `limit` 一起构成稳定分页
    /// ——期间新增条目不会让下一页重复或跳条。
    pub cursor: Option<(i64, i64)>,
}

/// 「全部标记已读」的作用域
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarkScope {
    All,
    Feed(i64),
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// 打开（或创建）数据库文件并执行迁移。
    ///
    /// 父目录不存在会自动创建——首次运行时 `~/.local/share/rustrss/` 往往还不存在，
    /// 而 sqlite 自己不会建目录（否则报 `unable to open database file`，很难定位）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| StoreError::Io(format!("创建目录 {} 失败: {e}", parent.display())))?;
            }
        }
        Self::init(Connection::open(path)?)
    }

    /// 内存数据库（测试用）。
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        // journal_mode 会返回一行，必须用 query_row 消费掉
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let mut version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        while (version as usize) < MIGRATIONS.len() {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(MIGRATIONS[version as usize])?;
            version += 1;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
        Ok(())
    }

    /// 当前 schema 版本（迁移后应等于 MIGRATIONS.len()）
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    // ---------------------------------------------------------------- 文件夹

    pub fn add_folder(&self, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("文件夹名不能为空".into()));
        }
        if let Some(id) = self
            .conn
            .query_row("SELECT id FROM folders WHERE name = ?1", params![name], |r| {
                r.get::<_, i64>(0)
            })
            .ok()
        {
            return Ok(id);
        }
        self.conn
            .execute("INSERT INTO folders (name) VALUES (?1)", params![name])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_folders(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name FROM folders ORDER BY position, name")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 结构化版（含 position），供命令层直接序列化给 UI。
    pub fn list_folders_ordered(&self) -> Result<Vec<FolderRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, position FROM folders ORDER BY position, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(FolderRow {
                id: r.get(0)?,
                name: r.get(1)?,
                position: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 把订阅源放进某个文件夹（`None` 表示移出）
    pub fn assign_folder(&self, feed_id: i64, folder_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET folder_id = ?1 WHERE id = ?2",
            params![folder_id, feed_id],
        )?;
        Ok(())
    }

    /// 重命名文件夹。名字撞唯一约束时给可读错误（而不是裸 sqlite 错误）。
    pub fn rename_folder(&self, folder_id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("文件夹名不能为空".into()));
        }
        let dup: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM folders WHERE name = ?1 AND id != ?2",
                params![name, folder_id],
                |r| r.get::<_, i64>(0),
            )
            .ok();
        if dup.is_some() {
            return Err(StoreError::Invalid(format!("文件夹「{name}」已存在")));
        }
        self.conn.execute(
            "UPDATE folders SET name = ?1 WHERE id = ?2",
            params![name, folder_id],
        )?;
        Ok(())
    }

    /// 删除文件夹：不删订阅，具内订阅移出到未分组（folder_id 置 NULL）。
    pub fn delete_folder(&self, folder_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET folder_id = NULL WHERE folder_id = ?1",
            params![folder_id],
        )?;
        self.conn
            .execute("DELETE FROM folders WHERE id = ?1", params![folder_id])?;
        Ok(())
    }

    /// 分组排序（侧栏组顺序）。
    pub fn set_folder_position(&self, folder_id: i64, position: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE folders SET position = ?1 WHERE id = ?2",
            params![position, folder_id],
        )?;
        Ok(())
    }

    /// 侧栏折叠状态（JSON 数组存 settings），跨会话保持。
    pub fn collapsed_folders(&self) -> Vec<i64> {
        self.setting(FOLDERS_COLLAPSED_KEY)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str::<Vec<i64>>(&v).ok())
            .unwrap_or_default()
    }

    pub fn set_collapsed_folders(&self, ids: &[i64]) -> Result<()> {
        let json =
            serde_json::to_string(ids).map_err(|e| StoreError::Invalid(e.to_string()))?;
        self.set_setting(FOLDERS_COLLAPSED_KEY, &json)
    }

    /// WAL 收尾：把 WAL 合并回主库并截断（大批量写入后调用，防 WAL 无限增长拖慢读取）。
    pub fn checkpoint_wal(&self) -> Result<()> {
        // wal_checkpoint 返回一行 (busy, log, checkpointed)：execute() 会因返回结果报错
        // （61c95c6 误改成 execute，方向反了，导致每次刷新都打「WAL checkpoint 失败」）。
        // 必须用 query_row 消费掉这一行。
        let _unused: (i64, i64, i64) = self.conn.query_row(
            "PRAGMA wal_checkpoint(TRUNCATE)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(())
    }

    /// RSSHub 迁移候选：rsshub:// scheme 与官方域两种存量。
    pub fn list_rsshub_migration_candidates(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url FROM feeds
              WHERE url LIKE 'rsshub://%'
                 OR url LIKE 'https://rsshub.app/%'
                 OR url LIKE 'https://www.rsshub.app/%'
              ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 更新订阅抓取地址（迁移用）。目标 URL 已被其它订阅占用时报可读错误。
    pub fn update_feed_url(&self, feed_id: i64, new_url: &str) -> Result<()> {
        let dup: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM feeds WHERE url = ?1 AND id != ?2",
                params![new_url, feed_id],
                |r| r.get::<_, i64>(0),
            )
            .ok();
        if dup.is_some() {
            return Err(StoreError::Invalid(format!(
                "目标地址已存在于其它订阅（feed #{dup:?}），已跳过"
            )));
        }
        self.conn.execute(
            "UPDATE feeds SET url = ?1 WHERE id = ?2",
            params![new_url, feed_id],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- 订阅源

    /// 添加订阅源；同一 URL 重复添加返回既有 id（幂等）。
    pub fn add_feed(&self, url: &str, title_hint: Option<&str>) -> Result<i64> {
        let url = url.trim();
        if url.is_empty() {
            return Err(StoreError::Invalid("订阅地址不能为空".into()));
        }
        // RSSHub 归一化收口：rsshub:// 与官方域统一实例化为实际抓取地址，
        // 库内 URL 永远等于真实抓取地址（OPML 导入与手动添加都走这里）。
        let mirror = self
            .setting(crate::rsshub::MIRROR_KEY)
            .unwrap_or_default()
            .unwrap_or_default();
        let url = crate::rsshub::normalize_rsshub_url(url, &mirror);
        let url = url.trim();
        if let Some(id) = self
            .conn
            .query_row("SELECT id FROM feeds WHERE url = ?1", params![url], |r| {
                r.get::<_, i64>(0)
            })
            .ok()
        {
            return Ok(id);
        }
        let title = title_hint
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or(url);
        self.conn.execute(
            "INSERT INTO feeds (url, title, created_at) VALUES (?1, ?2, ?3)",
            params![url, title, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_feeds(&self) -> Result<Vec<FeedRow>> {
        let sql = format!("{FEED_ROW_SELECT} {FEED_ROW_GROUP_BY} ORDER BY f.title COLLATE NOCASE");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], feed_row_from)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 单个订阅源行。设置命令成功后就回这一行给界面，不必重拉整个侧栏。
    pub fn feed_row(&self, feed_id: i64) -> Result<Option<FeedRow>> {
        let sql = format!("{FEED_ROW_SELECT} WHERE f.id = ?1 {FEED_ROW_GROUP_BY}");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![feed_id], feed_row_from)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// 设置每源刷新间隔覆盖（`None` = 跟随全局），返回更新后的行。
    /// 白名单校验在命令层（与全局档共用 `REFRESH_INTERVAL_CHOICES`），这里只落列。
    pub fn set_feed_refresh_interval(&self, feed_id: i64, minutes: Option<i64>) -> Result<FeedRow> {
        self.conn.execute(
            "UPDATE feeds SET refresh_interval_minutes = ?1 WHERE id = ?2",
            params![minutes, feed_id],
        )?;
        self.feed_row(feed_id)?
            .ok_or_else(|| StoreError::Invalid(format!("订阅 #{feed_id} 不存在")))
    }

    /// 调度扫描用：全部源的 `(id, 覆盖分钟, 上次抓取时刻)`。只读三列，扫描很轻，
    /// 供宿主侧的 tick 在**一次短锁**里拿到「谁到点了」的全部输入。
    pub fn feeds_with_interval(&self) -> Result<Vec<FeedIntervalRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, refresh_interval_minutes, last_fetched_at FROM feeds ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 抓取成功后写回源元信息（标题 / 站点 / 语言）。
    pub fn update_feed_meta(
        &self,
        feed_id: i64,
        title: Option<&str>,
        site_url: Option<&str>,
        description: Option<&str>,
        language: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET
                title = COALESCE(NULLIF(?2, ''), title),
                site_url = COALESCE(?3, site_url),
                description = COALESCE(?4, description),
                language = COALESCE(?5, language)
             WHERE id = ?1",
            params![feed_id, title, site_url, description, language],
        )?;
        Ok(())
    }

    /// 记录抓取结果：状态、错误、以及供下次条件请求使用的缓存头。
    pub fn record_fetch(
        &self,
        feed_id: i64,
        status: &str,
        error: Option<&str>,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET
                last_status = ?2,
                last_error = ?3,
                etag = COALESCE(?4, etag),
                last_modified = COALESCE(?5, last_modified),
                last_fetched_at = ?6
             WHERE id = ?1",
            params![feed_id, status, error, etag, last_modified, now()],
        )?;
        Ok(())
    }

    /// 读取条件请求所需的缓存头。
    pub fn cache_headers(&self, feed_id: i64) -> Result<(Option<String>, Option<String>)> {
        let row = self.conn.query_row(
            "SELECT etag, last_modified FROM feeds WHERE id = ?1",
            params![feed_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(row)
    }

    /// 删除订阅源（条目由外键级联删除，全文索引由触发器同步清理）。
    pub fn remove_feed(&self, feed_id: i64) -> Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM feeds WHERE id = ?1", params![feed_id])?)
    }

    /// 抓取所需的元信息：地址 + 上次留下的条件请求凭据。
    pub fn feed_endpoint(&self, feed_id: i64) -> Result<(String, Option<String>, Option<String>)> {
        Ok(self.conn.query_row(
            "SELECT url, etag, last_modified FROM feeds WHERE id = ?1",
            params![feed_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }

    /// 全部订阅源 id（用于「刷新全部」）
    pub fn all_feed_ids(&self) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM feeds ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 按地址查源 id（OPML 导入去重用；地址按 trim 后比较，与 add_feed 一致）
    pub fn feed_id_by_url(&self, url: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM feeds WHERE url = ?1",
                params![url.trim()],
                |r| r.get::<_, i64>(0),
            )
            .ok())
    }

    // ---------------------------------------------------------------- 条目入库

    /// 批量入库。已存在的条目按内容指纹决定「更新」还是「跳过」。
    pub fn upsert_entries(&self, feed_id: i64, entries: &[Entry]) -> Result<InsertStats> {
        let tx = self.conn.unchecked_transaction()?;
        let mut stats = InsertStats::default();
        let fetched_at = now();

        for e in entries {
            let fingerprint = fingerprint(e);
            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT id, content_hash FROM entries WHERE feed_id = ?1 AND stable_id = ?2",
                    params![feed_id, e.stable_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();

            match existing {
                Some((id, old_hash)) if old_hash == fingerprint => {
                    stats.unchanged += 1;
                    let _ = id;
                }
                Some((id, _)) => {
                    // 已抓过全文的条目：正文三列以「库里现有的值」为准（CASE 里引用旧值，
                    // 等于不动），元数据照常更新。理由是显式决策：RSS 源给的永远是摘要，
                    // 刷新把它覆盖回去就等于用户抓来的正文被静默丢弃（PRD R3）。
                    tx.execute(
                        "UPDATE entries SET
                            id_origin = ?2, title = ?3, url = ?4, author = ?5,
                            published_at = ?6, updated_at = ?7, summary = ?8,
                            content_html = CASE WHEN fulltext_fetched = 1 THEN content_html ELSE ?9 END,
                            content_text = CASE WHEN fulltext_fetched = 1 THEN content_text ELSE ?10 END,
                            search_tokens = CASE WHEN fulltext_fetched = 1 THEN search_tokens ELSE ?11 END,
                            content_hash = ?12, fetched_at = ?13
                         WHERE id = ?1",
                        params![
                            id,
                            origin_str(e.id_origin),
                            e.title,
                            e.url,
                            e.author,
                            ts(e.published),
                            ts(e.updated),
                            e.summary,
                            e.content_html,
                            e.content_text,
                            search_tokens_for(e),
                            fingerprint,
                            fetched_at
                        ],
                    )?;
                    stats.updated += 1;
                }
                None => {
                    tx.execute(
                        "INSERT INTO entries (
                            feed_id, stable_id, id_origin, title, url, author,
                            published_at, updated_at, summary, content_html, content_text,
                            search_tokens, content_hash, read, starred, fetched_at
                         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,0,0,?14)",
                        params![
                            feed_id,
                            e.stable_id,
                            origin_str(e.id_origin),
                            e.title,
                            e.url,
                            e.author,
                            ts(e.published),
                            ts(e.updated),
                            e.summary,
                            e.content_html,
                            e.content_text,
                            search_tokens_for(e),
                            fingerprint,
                            fetched_at
                        ],
                    )?;
                    stats.inserted += 1;
                }
            }
        }
        tx.commit()?;
        Ok(stats)
    }

    // ---------------------------------------------------------------- 全文写回

    /// 写回抓取到的全文正文（摘要型条目的「获取全文」）。
    ///
    /// 一处调用要做完三件事，否则后续拼接会不一致：正文两列 + **重算检索 token**
    /// （新正文要马上可搜）+ 置 `fulltext_fetched`（刷新不覆盖、重开零网络都靠它）。
    pub fn set_fulltext(&self, entry_id: i64, content_html: &str, content_text: &str) -> Result<()> {
        let existing: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT title, summary FROM entries WHERE id = ?1",
                params![entry_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((title, summary)) = existing else {
            return Err(StoreError::Invalid(format!("条目 #{entry_id} 不存在")));
        };
        let tokens = search_tokens_of(&[
            Some(title.as_str()),
            summary.as_deref(),
            Some(content_text),
        ]);
        self.conn.execute(
            "UPDATE entries SET content_html = ?2, content_text = ?3,
                search_tokens = ?4, fulltext_fetched = 1
             WHERE id = ?1",
            params![entry_id, content_html, content_text, tokens],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- 条目查询

    pub fn list_entries(&self, q: &EntryQuery) -> Result<Vec<EntryRow>> {
        let (sql, values) = list_entries_sql(q);
        self.query_entries(&sql, values, map_entry_row_list)
    }

    /// 诊断/测试用：`list_entries` 实际 SQL 的 EXPLAIN QUERY PLAN。
    ///
    /// 存在的意义是让「续扫必须走 idx_entries_sortkey 且不做临时排序」这条性能
    /// 契约能被断言，而断言跑在与线上逐字相同的 SQL 上——测试另抄一份 SQL 会在
    /// 实现改动后静默漂移。
    #[doc(hidden)]
    pub fn explain_list_entries(&self, q: &EntryQuery) -> Result<Vec<String>> {
        let (sql, values) = list_entries_sql(q);
        let mut stmt = self.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        let rows = stmt.query_map(params_from_iter(values), |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_entry(&self, id: i64) -> Result<Option<EntryRow>> {
        let sql = format!("{ENTRY_SELECT} WHERE e.id = ?");
        let mut rows = self.query_entries(&sql, vec![Value::Integer(id)], map_entry_row)?;
        Ok(rows.pop())
    }

    /// 全文搜索：拉丁词与中文 bigram 走 FTS5，单字中文走 LIKE 兜底。
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<EntryRow>> {
        let plan = plan_query(query);
        let mut values: Vec<Value> = Vec::new();
        let mut sql = entry_list_sql(false);
        let has_fts = !plan.fts.is_empty();

        if has_fts {
            sql.push_str(" JOIN entries_fts ON entries_fts.rowid = e.id");
        }
        sql.push_str(" WHERE 1=1");
        if has_fts {
            sql.push_str(" AND entries_fts MATCH ?");
            values.push(Value::Text(plan.fts.clone()));
        }
        for term in &plan.like_terms {
            sql.push_str(" AND (e.title LIKE ? OR e.content_text LIKE ?)");
            let pattern = format!("%{term}%");
            values.push(Value::Text(pattern.clone()));
            values.push(Value::Text(pattern));
        }
        if has_fts {
            sql.push_str(" ORDER BY bm25(entries_fts), COALESCE(e.published_at, e.fetched_at) DESC");
        } else {
            sql.push_str(" ORDER BY COALESCE(e.published_at, e.fetched_at) DESC, e.id DESC");
        }
        sql.push_str(" LIMIT ?");
        values.push(Value::Integer(limit.min(500) as i64));

        self.query_entries(&sql, values, map_entry_row_list)
    }

    /// 列表/搜索/阅读页共用一个查询主体，但**映射函数不同**：只有阅读页的完整行
    /// 才有正文与 `fulltext_fetched`，列表行用 `map_entry_row_list`（不做全文判定，
    /// 免得拿 NULL 正文把每条都判成摘要型）。
    fn query_entries(
        &self,
        sql: &str,
        values: Vec<Value>,
        map: fn(&rusqlite::Row<'_>) -> rusqlite::Result<EntryRow>,
    ) -> Result<Vec<EntryRow>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params_from_iter(values), map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------------------------------------------------------------- 状态

    pub fn set_read(&self, ids: &[i64], read: bool) -> Result<usize> {
        self.set_flag("read", ids, read)
    }

    pub fn set_starred(&self, ids: &[i64], starred: bool) -> Result<usize> {
        self.set_flag("starred", ids, starred)
    }

    /// 稍后读标记：与已读/星标相互独立。
    pub fn set_read_later(&self, ids: &[i64], read_later: bool) -> Result<usize> {
        self.set_flag("read_later", ids, read_later)
    }

    /// 稍后读总数（智能视图计数用）。
    pub fn read_later_total(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM entries WHERE read_later = 1", [], |r| r.get(0))
            .map_err(Into::into)
    }

    /// 全部计数（entry/unread/starred/read_later）。
    ///
    /// 历史教训：旧版是单扫描 4 聚合（SUM CASE × 3），但 starred/read_later 不在任何
    /// 覆盖索引里，且这两列排在正文大列之后—— planner 只能全表扫，逐行穿过溢出页链。
    /// 热缓存下 8.5k 条只要 ~10ms（曾因此误判可接受）；冷启动真实库上实测 83-119ms，
    /// 且侧栏每次计数刷新都付一次（打开文章去抖后也会触发），锁排队还会拖着 get_entry
    /// 一起变慢。改为 4 个子查询后各自走覆盖索引（total→sortkey / unread→read_published /
    /// starred→v9 部分索引 / later→v4 部分索引），实测 <1ms，冷热无关。
    pub fn counts(&self) -> Result<(i64, i64, i64, i64)> {
        self.conn
            .query_row(COUNTS_SQL, [], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(Into::into)
    }

    /// counts 的 EXPLAIN 断言入口（与线上 SQL 同源，见 `explain_list_entries` 先例）。
    #[doc(hidden)]
    pub fn explain_counts(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(&format!("EXPLAIN QUERY PLAN {COUNTS_SQL}"))?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn set_flag(&self, column: &str, ids: &[i64], value: bool) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        // column 只来自本模块内的字面量，不拼接外部输入
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!("UPDATE entries SET {column} = ? WHERE id IN ({placeholders})");
        let mut values: Vec<Value> = vec![Value::Integer(i64::from(value))];
        values.extend(ids.iter().map(|id| Value::Integer(*id)));
        Ok(self.conn.execute(&sql, params_from_iter(values))?)
    }

    /// 双向的全部标记：`read = true` 即「全部已读」，`false` 即「全部未读」（撤销用）
    pub fn mark_all(&self, scope: MarkScope, read: bool) -> Result<usize> {
        let target = i64::from(read);
        let n = match scope {
            MarkScope::All => self.conn.execute(
                "UPDATE entries SET read = ?1 WHERE read <> ?1",
                params![target],
            )?,
            MarkScope::Feed(feed_id) => self.conn.execute(
                "UPDATE entries SET read = ?1 WHERE read <> ?1 AND feed_id = ?2",
                params![target, feed_id],
            )?,
        };
        Ok(n)
    }

    // ---------------------------------------------------------------- 设置

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, now()],
        )?;
        Ok(())
    }

    /// 只在键不存在时写入；返回是否真的写入了。
    ///
    /// 用于「生成一次、以后永不覆盖」的值（如 MCP token）：结构上堵住
    /// 「已有值被静默改写」这类事故。
    pub fn insert_setting_if_absent(&self, key: &str, value: &str) -> Result<bool> {
        let n = self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO NOTHING",
            params![key, value, now()],
        )?;
        Ok(n > 0)
    }

    pub fn all_settings(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings ORDER BY key")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 布尔设置的读取：**缺失或值不合法都回退到默认值**（不让拼错的值把功能卡死）
    pub fn bool_setting(&self, key: &str, default: bool) -> Result<bool> {
        Ok(match self.setting(key)?.as_deref() {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => default,
        })
    }

    pub fn set_bool_setting(&self, key: &str, value: bool) -> Result<()> {
        self.set_setting(key, if value { "true" } else { "false" })
    }

    /// 每个源的未读数
    pub fn unread_by_feed(&self) -> Result<Vec<(i64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT feed_id, COUNT(*) FROM entries WHERE read = 0 GROUP BY feed_id ORDER BY feed_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 未读总数
    pub fn unread_total(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM entries WHERE read = 0", [], |r| {
                r.get(0)
            })?)
    }

    /// 条目总数
    pub fn entry_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?)
    }

    /// 星标总数
    pub fn starred_total(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE starred = 1",
            [],
            |r| r.get(0),
        )?)
    }

    // ---------------------------------------------------------------- AI 缓存

    /// 读缓存：命中则不必再请求模型
    pub fn ai_cached(&self, key: &AiCacheKey<'_>) -> Result<Option<String>> {
        let row = self
            .conn
            .query_row(
                "SELECT output FROM ai_cache
                 WHERE entry_id = ?1 AND task = ?2 AND params = ?3
                   AND provider_model = ?4 AND prompt_version = ?5",
                params![
                    key.entry_id,
                    key.task,
                    key.params,
                    key.provider_model,
                    key.prompt_version
                ],
                |r| r.get::<_, String>(0),
            )
            .ok();
        Ok(row)
    }

    /// 写缓存（同键则覆盖并刷新时间）
    pub fn ai_store(&self, key: &AiCacheKey<'_>, output: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ai_cache
                (entry_id, task, params, provider_model, prompt_version, output, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(entry_id, task, params, provider_model, prompt_version)
             DO UPDATE SET output = excluded.output, created_at = excluded.created_at",
            params![
                key.entry_id,
                key.task,
                key.params,
                key.provider_model,
                key.prompt_version,
                output,
                now()
            ],
        )?;
        Ok(())
    }

    /// 清缓存：`Some(entry_id)` 只清该条，`None` 清全部（用于「重新生成」）
    pub fn ai_cache_clear(&self, entry_id: Option<i64>) -> Result<usize> {
        let n = match entry_id {
            Some(id) => self
                .conn
                .execute("DELETE FROM ai_cache WHERE entry_id = ?1", params![id])?,
            None => self.conn.execute("DELETE FROM ai_cache", [])?,
        };
        Ok(n)
    }
}

/// AI 缓存键：身份由「文章 + 任务 + 参数 + 模型 + prompt 版本」共同决定
#[derive(Debug, Clone, Copy)]
pub struct AiCacheKey<'a> {
    pub entry_id: i64,
    pub task: &'a str,
    pub params: &'a str,
    pub provider_model: &'a str,
    pub prompt_version: &'a str,
}

/// counts() 的 SQL：4 个子查询各自走覆盖索引（total→sortkey / unread→read_published /
/// starred→v9 部分索引 / later→v4 部分索引），绝不触碰正文大列所在的表 B 树。
/// 抽成独立常量是为了让 EXPLAIN 断言与线上 SQL 逐字同源（见 `explain_counts`）。
const COUNTS_SQL: &str = "SELECT (SELECT COUNT(*) FROM entries),
       (SELECT COUNT(*) FROM entries WHERE read = 0),
       (SELECT COUNT(*) FROM entries WHERE starred = 1),
       (SELECT COUNT(*) FROM entries WHERE read_later = 1)";

const ENTRY_SELECT: &str = "SELECT e.id, e.feed_id, f.title, e.stable_id, e.id_origin, e.title,
        e.url, e.author, e.published_at, e.summary, e.content_html, e.content_text,
        e.read, e.starred, e.read_later, COALESCE(e.published_at, e.fetched_at) AS sortkey,
        e.fulltext_fetched
    FROM entries e JOIN feeds f ON f.id = e.feed_id";

/// 列表/搜索专用列（不含 FROM）：不带正文全文（content 列以 NULL 占位，列序与
/// 完整版前 16 列一致）。8k 条库上单次查询从 ~12MB 传输降到几百 KB——正文一律
/// get_entry 单取。正文列为 NULL，因此这些行不做全文判定（`map_entry_row_list`），
/// 也不带 `fulltext_fetched` 列（只有 `ENTRY_SELECT` 有第 17 列）。
///
/// （原本是一整条 `ENTRY_SELECT_LIST`，拆成「列 + `entry_list_sql` 拼 FROM」是为了
/// 续扫时能在 FROM 上带索引提示，而列只保留一份。）
const ENTRY_LIST_COLUMNS: &str = "SELECT e.id, e.feed_id, f.title, e.stable_id, e.id_origin, e.title,
        e.url, e.author, e.published_at, e.summary, NULL, NULL,
        e.read, e.starred, e.read_later, COALESCE(e.published_at, e.fetched_at) AS sortkey";

/// 列表/搜索查询的公共主体（列 + 条目 JOIN 订阅源，列表要源标题）。
///
/// `indexed_by_sortkey`（仅 keyset 续扫用）在 FROM 上钉死表达式索引，理由见
/// `list_entries_sql` 里那段注释。
fn entry_list_sql(indexed_by_sortkey: bool) -> String {
    let hint = if indexed_by_sortkey { " INDEXED BY idx_entries_sortkey" } else { "" };
    format!("{ENTRY_LIST_COLUMNS} FROM entries e{hint} JOIN feeds f ON f.id = e.feed_id")
}

/// `list_entries` 的 SQL 与参数（游标续扫条件在这里拼接）。
///
/// 抽成独立函数是为了让 EXPLAIN 断言与线上 SQL 逐字同源（见 `explain_list_entries`）。
fn list_entries_sql(q: &EntryQuery) -> (String, Vec<Value>) {
    let mut sql = entry_list_sql(q.cursor.is_some());
    let mut values: Vec<Value> = Vec::new();
    sql.push_str(" WHERE 1=1");
    if let Some(feed_id) = q.feed_id {
        sql.push_str(" AND e.feed_id = ?");
        values.push(Value::Integer(feed_id));
    }
    if q.unread_only {
        sql.push_str(" AND e.read = 0");
    }
    if q.starred_only {
        sql.push_str(" AND e.starred = 1");
    }
    if q.read_later_only {
        sql.push_str(" AND e.read_later = 1");
    }
    if let Some((sortkey, id)) = q.cursor {
        // keyset 续扫：语义就是行值比较 `(sortkey, id) < (?, ?)`。
        //
        // 实测（sqlite 3.53.2，8000 条库，带/不带 ANALYZE 统计都验过）：
        // 1. 行值比较不会被当成 idx_entries_sortkey 上的范围约束——计划退化成
        //    `SCAN ... USING INDEX`，续页得从索引头部扫到游标位置，逐页变慢；
        // 2. 只写展开式（下面的 OR）又会走 MULTI-INDEX OR + `USE TEMP B-TREE FOR ORDER BY`。
        // 因此这里多带一条冗余的前导范围约束 `sortkey <= ?`：它给表达式索引一个
        // 真正可用的 SEARCH 起点；括号内的 OR 把语义收回到严格小于（同值按 id 续扫）。
        //
        // 另外 FROM 上带 INDEXED BY（经 entry_list_sql）：不过滤时靠统计就能选中
        // idx_entries_sortkey，但 read/feed_id 这类有等值索引的筛选形态，在没有
        // sqlite_stat1 的新库里 planner 会改选等值索引 + 临时排序（本程序从不 ANALYZE）。
        // 续扫是逐页路径，计划不能看统计的脸色——钉住索引后各筛选形态实测均为
        // `SEARCH e USING INDEX idx_entries_sortkey (<expr><? )`，无 TEMP B-TREE。
        sql.push_str(
            " AND COALESCE(e.published_at, e.fetched_at) <= ?
                 AND (COALESCE(e.published_at, e.fetched_at) < ?
                      OR (COALESCE(e.published_at, e.fetched_at) = ? AND e.id < ?))",
        );
        values.push(Value::Integer(sortkey));
        values.push(Value::Integer(sortkey));
        values.push(Value::Integer(sortkey));
        values.push(Value::Integer(id));
    }
    sql.push_str(" ORDER BY COALESCE(e.published_at, e.fetched_at) DESC, e.id DESC LIMIT ?");
    values.push(Value::Integer(q.limit.unwrap_or(50).min(500) as i64));
    (sql, values)
}

fn map_entry_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    let mut row = map_entry_columns(r)?;
    // 内部标记位（列序在最后）只用于判定结论，不进 EntryRow
    let fetched = r.get::<_, i64>(16)? != 0;
    row.needs_fulltext = crate::fulltext::is_summary_entry(
        row.url.as_deref(),
        row.content_html.as_deref(),
        row.content_text.as_deref(),
        fetched,
    );
    Ok(row)
}

/// 列表/搜索行：不带正文，也不做全文判定（`needs_fulltext` 恒 false）。
fn map_entry_row_list(r: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    map_entry_columns(r)
}

/// 两个映射函数共用的列序（与 `ENTRY_SELECT` / `ENTRY_LIST_COLUMNS` 前 16 列逐列对应）。
fn map_entry_columns(r: &rusqlite::Row<'_>) -> rusqlite::Result<EntryRow> {
    Ok(EntryRow {
        id: r.get(0)?,
        feed_id: r.get(1)?,
        feed_title: r.get(2)?,
        stable_id: r.get(3)?,
        id_origin: r.get(4)?,
        title: r.get(5)?,
        url: r.get(6)?,
        author: r.get(7)?,
        published_at: r.get(8)?,
        summary: r.get(9)?,
        content_html: r.get(10)?,
        content_text: r.get(11)?,
        needs_fulltext: false,
        read: r.get::<_, i64>(12)? != 0,
        starred: r.get::<_, i64>(13)? != 0,
        read_later: r.get::<_, i64>(14)? != 0,
        sortkey: r.get(15)?,
    })
}

fn origin_str(o: IdOrigin) -> &'static str {
    match o {
        IdOrigin::SourceData => "source_data",
        IdOrigin::ContentHash => "content_hash",
    }
}

/// 内容指纹：判断这次刷新内容到底变没变。字段间用分隔符防拼接歧义。
fn fingerprint(e: &Entry) -> String {
    let mut h = Sha256::new();
    let mut field = |v: Option<&str>| {
        h.update(v.unwrap_or("\u{1e}").as_bytes());
        h.update(b"\x1f"); // 字段分隔符
    };
    field(Some(&e.title));
    field(e.url.as_deref());
    field(e.author.as_deref());
    field(e.summary.as_deref());
    field(e.content_html.as_deref());
    field(e.content_text.as_deref());
    h.update(e.published.map(|t| t.timestamp()).unwrap_or_default().to_string().as_bytes());
    h.update(e.updated.map(|t| t.timestamp()).unwrap_or_default().to_string().as_bytes());
    hex(&h.finalize())
}

fn search_tokens_for(e: &Entry) -> String {
    search_tokens_of(&[
        Some(e.title.as_str()),
        e.summary.as_deref(),
        e.content_text.as_deref(),
    ])
}

/// 检索字段的拼接口径：标题 + 摘要 + 正文纯文本（入库与全文写回共用一处，
/// 免得两条写入路径各拼一套而漂移）。
fn search_tokens_of(parts: &[Option<&str>]) -> String {
    let mut text = String::new();
    for part in parts.iter().flatten() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(part);
    }
    to_tokens(&text)
}

fn now() -> i64 {
    Utc::now().timestamp()
}

fn ts(t: Option<DateTime<Utc>>) -> Option<i64> {
    t.map(|t| t.timestamp())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
