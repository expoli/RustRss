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

use std::collections::HashMap;
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
    /// 标签名重复（大小写不敏感）。单独成档，MCP 侧据此回 `duplicate_tag_name`
    /// 而不是去猜错误文案（同 folder 改名冲突的处理口径）。
    #[error("标签名已存在: {0}")]
    DuplicateTagName(String),
    /// 标签 id 不存在（改名 / 改色 / 删除 / 打标的目标标签都必须真实存在）。
    #[error("标签不存在: #{0}")]
    TagNotFound(i64),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

pub(crate) const FOLDERS_COLLAPSED_KEY: &str = "ui.folders_collapsed";

/// 列表排序档位的设置键（值：`newest` / `oldest` / `unread_first`）。
///
/// 排序与「隐藏已读」都是**全局设置**，`list_entries` 在查询时直接读库——前端不再
/// 往 `EntryQuery` 里塞一份副本，避免两个事实源（MCP 与界面共用同一条读取路径）。
pub const LIST_SORT_KEY: &str = "list.sort";
/// 「隐藏已读」开关的设置键（布尔，默认关）。过滤时星标/稍后读视图豁免，理由见
/// `list_entries_sql` 的过滤分支。
pub const LIST_HIDE_READ_KEY: &str = "list.hide_read";

/// 列表排序档位（`list.sort`；库里的值缺失或写坏都回退 [`ListSort::Newest`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSort {
    /// 最新在前（默认）：`sortkey DESC, id DESC`
    Newest,
    /// 最早在前：`sortkey ASC, id ASC`——反扫 [`ListSort::Newest`] 用的同一个索引
    Oldest,
    /// 未读优先：`read ASC, sortkey DESC, id ASC`（未读组内仍最新在前）
    UnreadFirst,
}

impl ListSort {
    /// 存储值 → 档位；非法值归默认档（与 locale/theme 同一口径：库被写坏也不卡死）。
    pub fn from_setting(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("oldest") => Self::Oldest,
            Some("unread_first") => Self::UnreadFirst,
            _ => Self::Newest,
        }
    }

    /// 档位的存储值（命令层白名单归一化后落库用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::UnreadFirst => "unread_first",
        }
    }
}

/// 订阅源行查询的共同部分：列清单与聚合口径只写一份，`list_feeds` / `feed_row`
/// 共用（新增列时只改这里，避免两处 SQL 漂移）。
///
/// `title` 一列出的是**显示名**（`COALESCE(custom_title, title)`），源站原名
/// 另出一列 `source_title`——编辑对话框要拿它当 placeholder（见 `FeedRow`）。
/// 条目表的 `feed_title` 列用同一个表达式（见 `ENTRY_SELECT` / `ENTRY_LIST_COLUMNS`）。
const FEED_DISPLAY_TITLE: &str = "COALESCE(f.custom_title, f.title)";

const FEED_ROW_SELECT: &str = "\
    SELECT f.id, f.url, COALESCE(f.custom_title, f.title) AS title, f.site_url, f.folder_id,
           COALESCE(SUM(CASE WHEN e.read = 0 THEN 1 ELSE 0 END), 0) AS unread,
           f.last_status, f.last_error, f.last_fetched_at, f.refresh_interval_minutes,
           f.custom_title, f.title
      FROM feeds f LEFT JOIN entries e ON e.feed_id = f.id";

const FEED_ROW_GROUP_BY: &str = "\
    GROUP BY f.id, f.url, f.title, f.custom_title, f.site_url, f.folder_id, f.last_status,
             f.last_error, f.last_fetched_at, f.refresh_interval_minutes";

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
        custom_title: r.get(10)?,
        source_title: r.get(11)?,
    })
}

/// 订阅源行（带未读数）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeedRow {
    pub id: i64,
    pub url: String,
    /// **显示名**：`COALESCE(custom_title, title)`。侧栏 / 列表 / 阅读区都用它。
    pub title: String,
    pub site_url: Option<String>,
    pub folder_id: Option<i64>,
    pub unread: i64,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_fetched_at: Option<i64>,
    /// 每源刷新间隔覆盖（分钟）；`None` = 跟随全局档。
    pub refresh_interval_minutes: Option<i64>,
    /// 用户自定义标题；`None` = 没设过（显示跟随源站名）。
    /// 编辑对话框拿它回填输入框，空输入框 = 清除自定义。
    pub custom_title: Option<String>,
    /// 源站原始标题（会被抓取刷新覆盖）。编辑对话框把它当 placeholder，
    /// 让用户随时看得见「清除自定义后会显示成什么」。
    pub source_title: String,
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
    /// 该条目身上的标签（名称序）。列表 / 搜索 / 阅读页三条路径都在
    /// [`Store::query_entries`] 里统一填充（一次查询覆盖整页），口径只有一份。
    pub tags: Vec<TagBrief>,
}

/// 一次入库的统计：用于验证「重复刷新不产生重复条目」
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct InsertStats {
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
}

/// 存量 RSSHub 地址归一化的结果（[`Store::normalize_rsshub_feeds`]）。
///
/// 字段名沿用历史上的「迁移」口径，界面的结果文案键不动；语义已改为一次性归一化。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MigrationOutcome {
    /// 已改写为 `rsshub://path` 的行数
    pub migrated: i64,
    /// 目标地址已被其它订阅占用而跳过的行数
    pub skipped: i64,
    /// 其它失败明细（单项失败不影响其余行）
    pub errors: Vec<String>,
}

/// 条目列表查询条件
#[derive(Debug, Clone, Default)]
pub struct EntryQuery {
    pub feed_id: Option<i64>,
    /// 多源过滤（`feed_id IN (…)`）。语义：
    /// - `None` = 不过滤（全部源）；
    /// - `Some(非空)` = 只取这些源的条目（未知 id 自然匹配零条）；
    /// - `Some(空)` = **匹配零条**（不是「不过滤」）——「某分组下没有订阅」这类
    ///   上游解析结果直接传空列表时，绝不能反过来把全库倒出去。
    pub feed_ids: Option<Vec<i64>>,
    pub unread_only: bool,
    pub starred_only: bool,
    pub read_later_only: bool,
    /// 时间下界（含）：对 `COALESCE(published_at, fetched_at)`（即列表排序键
    /// `EntryRow::sortkey`）比较，`>=`。`None` = 不过滤。
    /// 用 COALESCE 而不是裸 `published_at`：源站不给时间时条目按抓取时刻参与
    /// 时间过滤，与列表排序键是同一个值，口径不会两套。
    pub since: Option<i64>,
    /// 时间上界（**含**）：同 [`EntryQuery::since`] 的表达式，`<=`。
    /// `since` / `until` 都是闭区间；`since > until` 自然匹配零条。
    pub until: Option<i64>,
    pub limit: Option<u32>,
    /// keyset 续扫游标：上一页末行的 `(sortkey, id)`（`EntryRow::sortkey` 直出）。
    /// `None` = 取首页；只返回排在游标之后的行，因此与 `limit` 一起构成稳定分页
    /// ——期间新增条目不会让下一页重复或跳条。
    pub cursor: Option<(i64, i64)>,
    /// 游标的 `read` 分量：**只**在 [`ListSort::UnreadFirst`] 档需要（该档排序键是
    /// `(read, sortkey, id)`，缺这一位就定位不到续扫起点）。
    ///
    /// 它是游标载荷的一部分，不是排序/过滤设置的副本——排序档与「隐藏已读」两个
    /// 设置一律由 [`Store::list_entries`] 在查询时从库里读，故此处没有对应字段。
    /// `UnreadFirst` 档下给了 `cursor` 却没给这一位时按首页处理（半截游标不静默翻错页）。
    pub cursor_read: Option<bool>,
    /// 显式排序档覆盖：`None` = 跟随界面设置（`list.sort`），`Some` = 用这一档。
    ///
    /// 界面走 `None`（设置是单一事实源）；MCP 列表工具走 `Some`——agent 的默认
    /// 口径固定为 `newest`，不被用户此刻的界面选择左右。
    pub sort: Option<ListSort>,
    /// 显式「隐藏已读」覆盖：`None` = 跟随界面设置（`list.hide_read`）。
    /// 语义同 [`EntryQuery::sort`]：MCP 侧固定传 `Some(false)` 作为默认。
    pub hide_read: Option<bool>,
    /// 按标签过滤（关联存在性）。`None` = 不过滤——**既有默认口径不变**（界面跟随
    /// 设置、MCP 固定 newest + 不隐藏已读）。
    ///
    /// 走 `idx_entry_tags_tag` 取该标签的条目 id 集，再按既有排序索引取条目（排序索引
    /// 在带标签过滤时被 `INDEXED BY` 钉住，见 `list_entries_sql`），因此**不会退化成
    /// `SCAN entries`**——断言与变异校验见 `explain_list_entries` 的标签用例。
    pub tag_id: Option<i64>,
}

/// 「全部标记已读」的作用域
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarkScope {
    All,
    Feed(i64),
}

/// 条目状态位（`set_read` / `set_starred` / `set_read_later` 同族的第三套入口）。
///
/// 用枚举而不是 `&str` 列名：MCP 的写工具把入参透传到这一层，字符串列名在这里
/// 就是一条拼错即改错列的通道（SQLite 不会报错，只会默默写别的列）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryFlag {
    Read,
    Starred,
    ReadLater,
}

impl EntryFlag {
    fn column(self) -> &'static str {
        match self {
            EntryFlag::Read => "read",
            EntryFlag::Starred => "starred",
            EntryFlag::ReadLater => "read_later",
        }
    }
}

/// 条件级状态写入的目标范围（MCP：`set_read({feed_id, since, until})`）。
///
/// 三个字段全为 `None` 的空范围**不是**「全部」：它代表调用方没给条件，
/// 由上层判 `invalid_argument`。这里若把空范围当「不过滤」，一次漏参就会静默改全库。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntryFlagScope {
    /// 只看某个订阅源
    pub feed_id: Option<i64>,
    /// 下界（含），比较键与列表排序键同源：`COALESCE(published_at, fetched_at)`
    pub since: Option<i64>,
    /// 上界（含），口径同 `since`
    pub until: Option<i64>,
}

impl EntryFlagScope {
    pub fn is_empty(&self) -> bool {
        self.feed_id.is_none() && self.since.is_none() && self.until.is_none()
    }

    /// 条件级查询/写入共用的 WHERE 片段（采样与写入必须看到同一批行，
    /// 两处各拼一遍迟早漂移）。空范围返回 `None`（调用方已经判过非法）。
    fn where_sql(&self) -> Option<(String, Vec<Value>)> {
        let mut clauses: Vec<&str> = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        if let Some(feed_id) = self.feed_id {
            clauses.push("feed_id = ?");
            values.push(Value::Integer(feed_id));
        }
        if let Some(since) = self.since {
            clauses.push("COALESCE(published_at, fetched_at) >= ?");
            values.push(Value::Integer(since));
        }
        if let Some(until) = self.until {
            clauses.push("COALESCE(published_at, fetched_at) <= ?");
            values.push(Value::Integer(until));
        }
        if clauses.is_empty() {
            return None;
        }
        Some((clauses.join(" AND "), values))
    }
}

/// 条件级条目标识（`feed_id` / `since` / `until`，闭区间）——就是 [`EntryFlagScope`]，
/// 加别名只为在标签 API 的签名上读得通：**口径只有一套**，不允许改写一份。
pub type EntryScope = EntryFlagScope;

/// 标签行（带未读计数）。
///
/// `color` 为 `#rrggbb` 或 `None`（默认色）；`pinned` 置顶优先；`sort_order` 是侧栏
/// 手动顺序（小的在前）；`last_used_at` 驱动选择器「最近使用优先」（`None` = 还没用过）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub pinned: bool,
    pub sort_order: i64,
    pub last_used_at: Option<i64>,
    /// 该标签下**未读**条目数（计数只扫覆盖索引，不碰正文大列所在表 B 树）。
    pub unread: i64,
}

/// 条目身上的标签（[`EntryRow::tags`]）：只带 id + 名称，够渲染 chips 与点击筛选。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagBrief {
    pub id: i64,
    pub name: String,
}

/// [`Store::delete_tag`] 的结果。
///
/// `affected_entries` 由 [`Store::tag_entry_count`] 算出：`dry_run` 预览与实际执行
/// **共用同一个计数函数**，于是「预览说 N 篇」与「真删影响 N 篇」不可能漂移。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct DeleteTagReport {
    /// 将失去该标签的条目数（删除只清关联，**不删文章**）
    pub affected_entries: i64,
    /// 是否为预览（true = 没有落库）
    pub dry_run: bool,
}

/// [`Store::assign_tags`] / [`Store::unassign_tags`] 的结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagAssignReport {
    /// 真正新增（assign）或移除（unassign）的关联行数；重复调用为 0（幂等）。
    pub changed: usize,
    /// 目标命中的条目数（与标签个数无关；batch 档只算库里真实存在的 id）。
    pub entries: i64,
    /// 本次操作的标签 id（去重升序）。
    pub tag_ids: Vec<i64>,
}

/// 标签写操作的目标：显式条目 id 批量（≤ [`TAG_BATCH_MAX_IDS`]）或条件级范围。
#[derive(Debug, Clone, PartialEq)]
pub enum TagTarget {
    /// 显式条目 id（未知 id 静默跳过——只给真实存在的条目建关联）。
    Entries(Vec<i64>),
    /// 条件级 `{feed_id, since, until}`（至少给一项，空范围报错而**不是**「全部」）。
    Scope(EntryScope),
}

/// 标签批量条目 id 上限。
///
/// 与 MCP 写契约（`write_contract::MAX_BATCH_IDS`）同一口径：core 这里是最后一道
/// 防线，超限明确报错而不是静默截断。
pub const TAG_BATCH_MAX_IDS: usize = 100;

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

    /// 某个文件夹里的订阅源 id（MCP 的 `folder_id` 过滤解析成 `feed_ids IN (…)` 用）。
    ///
    /// 空组返回空列表——调用方必须把空列表当作「匹配零条」传递（见
    /// [`EntryQuery::feed_ids`] 的语义），不能当成「不过滤」。
    pub fn feed_ids_in_folder(&self, folder_id: i64) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM feeds WHERE folder_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![folder_id], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

    /// RSSHub 归一化候选：`rsshub://` scheme 与官方域两种存量。
    ///
    /// scheme 行也在候选里（`rsshub:///path` 三斜杠、大写 scheme 这类非规范写法需要
    /// 归一），但规范化后等于自身，因此幂等跳过——判定统一用 `canonical_scheme_url`。
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

    /// 归一化预览：真正会被改写的存量条数（规范化后与自身不同的行）。
    ///
    /// 与 [`Store::normalize_rsshub_feeds`] 共用同一判据，因此「预览说几条」
    /// 与「执行改几条」不可能漂移。
    pub fn count_rsshub_normalization_candidates(&self) -> Result<i64> {
        let n = self
            .list_rsshub_migration_candidates()?
            .into_iter()
            .filter(|(_, url)| crate::rsshub::canonical_scheme_url(url) != *url)
            .count();
        Ok(n as i64)
    }

    /// 存量归一化：官方域行（以及三斜杠/大写等非规范 scheme 行）→ `rsshub://path`。
    ///
    /// 一次性整理，不绑定镜像实例——抓取地址由 [`Store::feed_endpoint`] 解析。
    /// 已是规范 scheme 的行天然幂等跳过；目标地址被其它订阅占用计 `skipped`，
    /// 其它错误进 `errors`，单项失败不影响其余行。
    pub fn normalize_rsshub_feeds(&self) -> Result<MigrationOutcome> {
        let mut outcome = MigrationOutcome::default();
        for (feed_id, url) in self.list_rsshub_migration_candidates()? {
            let target = crate::rsshub::canonical_scheme_url(&url);
            if target == url {
                continue; // 已规范化（幂等）
            }
            // 目标地址冲突预检：被其它订阅占用则计 skipped
            if self.feed_id_by_url(&target)?.is_some() {
                outcome.skipped += 1;
                continue;
            }
            match self.update_feed_url(feed_id, &target) {
                Ok(()) => outcome.migrated += 1,
                Err(e) => outcome.errors.push(format!("feed #{feed_id}: {e}")),
            }
        }
        Ok(outcome)
    }

    /// 更新订阅地址（存量归一化落库用）。目标 URL 已被其它订阅占用时报可读错误。
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
        // RSSHub 归一化收口（**存储形态**）：rsshub:// 保留为抽象身份，官方域转 scheme，
        // 其它原样。这里**不读镜像设置**——解析推迟到抓取时（feed_endpoint），
        // 于是换镜像零迁移；去重键即「同一条路由的两种写法」的共同形态。
        let url = crate::rsshub::canonical_scheme_url(url);
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
        // 排序用显示名：自定义名改了要立刻重排，否则侧栏名字变了位置不动。
        let sql = format!(
            "{FEED_ROW_SELECT} {FEED_ROW_GROUP_BY} ORDER BY {FEED_DISPLAY_TITLE} COLLATE NOCASE"
        );
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

    /// 设置用户自定义标题（`None` = 清除自定义、显示回退源站名），返回更新后的行。
    /// 归一化（trim / 空串=清除）在命令层；这里只落列。
    /// 注意：自定义名不会被抓取刷新覆盖——`update_feed_meta` 只写 `title`。
    pub fn set_feed_custom_title(&self, feed_id: i64, custom_title: Option<&str>) -> Result<FeedRow> {
        self.conn.execute(
            "UPDATE feeds SET custom_title = ?1 WHERE id = ?2",
            params![custom_title, feed_id],
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
    ///
    /// 只写 `title`（**源站名**），绝不碰 `custom_title`——用户改的名要在刷新后保留，
    /// 显示层的 `COALESCE(custom_title, title)` 自然会继续用自定义名。
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
    ///
    /// 标签关联（`entry_tags`）走两道保险：`PRAGMA foreign_keys=ON`（`Store::init`，
    /// 连接级）已经能让 `feeds → entries → entry_tags` 逐级级联，这里再**显式**清
    /// 一遍——不变量不该以「调用方记得开 pragma」为前提（测试断言零孤儿）。
    pub fn remove_feed(&self, feed_id: i64) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM entry_tags WHERE entry_id IN
                (SELECT id FROM entries WHERE feed_id = ?1)",
            params![feed_id],
        )?;
        let n = tx.execute("DELETE FROM feeds WHERE id = ?1", params![feed_id])?;
        tx.commit()?;
        Ok(n)
    }

    /// 抓取所需的元信息：**解析后的抓取地址** + 上次留下的条件请求凭据。
    ///
    /// 库内 URL 是存储形态（scheme / 存量官方域 / 普通 URL），这里是唯一的解析出口，
    /// 按当前镜像设置（`rsshub.mirror`）解析成实际抓取地址——改镜像后下一次刷新
    /// 即生效，无需任何迁移。
    pub fn feed_endpoint(&self, feed_id: i64) -> Result<(String, Option<String>, Option<String>)> {
        let (url, etag, last_modified): (String, Option<String>, Option<String>) = self
            .conn
            .query_row(
                "SELECT url, etag, last_modified FROM feeds WHERE id = ?1",
                params![feed_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
        let mirror = self.setting(crate::rsshub::MIRROR_KEY)?.unwrap_or_default();
        Ok((
            crate::rsshub::resolve_fetch_url(&url, &mirror),
            etag,
            last_modified,
        ))
    }

    /// 全部订阅源 id（用于「刷新全部」）
    pub fn all_feed_ids(&self) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM feeds ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 按地址查源 id（OPML 导入去重用）。查询键先做**存储形态归一**，与 `add_feed`
    /// 的比较口径一致——否则同一路由的 scheme 写法与官方域写法会各算一条，导入计数
    /// 就会说「新增」而实际上被 add_feed 判重返回了既有 id。
    pub fn feed_id_by_url(&self, url: &str) -> Result<Option<i64>> {
        let key = crate::rsshub::canonical_scheme_url(url);
        let key = key.trim();
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM feeds WHERE url = ?1",
                params![key],
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
        // 排序档与「隐藏已读」的默认值在查询时从设置读（单一事实源）：界面不往
        // EntryQuery 里塞副本，也就不会与库里真正的设置漂移。查询自带覆盖
        // （`q.sort` / `q.hide_read`，MCP 用）时以显式值为准。
        let (sql, values) = list_entries_sql(q, self.effective_sort(q), self.effective_hide_read(q));
        self.query_entries(&sql, values, map_entry_row_list)
    }

    /// 本次查询实际生效的排序档：显式覆盖 → 设置。
    fn effective_sort(&self, q: &EntryQuery) -> ListSort {
        q.sort.unwrap_or_else(|| self.list_sort())
    }

    /// 本次查询实际生效的「隐藏已读」：显式覆盖 → 设置。
    fn effective_hide_read(&self, q: &EntryQuery) -> bool {
        q.hide_read.unwrap_or_else(|| self.list_hide_read())
    }

    /// 诊断/测试用：`list_entries` 实际 SQL 的 EXPLAIN QUERY PLAN。
    ///
    /// 存在的意义是让「续扫必须走本档的排序索引且不做临时排序」这条性能
    /// 契约能被断言，而断言跑在与线上逐字相同的 SQL 上——测试另抄一份 SQL 会在
    /// 实现改动后静默漂移。设置（排序档 / 隐藏已读）与线上同源，因此把设置写进库
    /// 再断言计划，验的就是用户真会跑到的那个形态。
    #[doc(hidden)]
    pub fn explain_list_entries(&self, q: &EntryQuery) -> Result<Vec<String>> {
        let (sql, values) = list_entries_sql(q, self.effective_sort(q), self.effective_hide_read(q));
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
    ///
    /// 搜索的排序仍是「相关度（bm25）优先 / 无词时按时间」——排序档是列表的展示
    /// 口径，不参与搜索排序；但「隐藏已读」是全局列表过滤，搜索同样生效（无视图豁免）。
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<EntryRow>> {
        let plan = plan_query(query);
        let mut values: Vec<Value> = Vec::new();
        let mut sql = entry_list_sql(None);
        let has_fts = !plan.fts.is_empty();

        if has_fts {
            sql.push_str(" JOIN entries_fts ON entries_fts.rowid = e.id");
        }
        sql.push_str(" WHERE 1=1");
        if self.list_hide_read() {
            sql.push_str(" AND e.read = 0");
        }
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
        let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        // 标签随行出库（三条读取路径同一个收口 → 不存在「列表有、阅读页没有」的漂移）
        self.fill_entry_tags(&mut out)?;
        Ok(out)
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

    /// 单个订阅源的条目数（MCP `unsubscribe` 的影响面：删源会级联删掉多少条）。
    ///
    /// **预览与实际执行共用这一个函数**（MCP 的 `dry_run` 与实际删除都调它），
    /// 所以「预览说 N 条」与「真删了 N 条」不可能漂移。
    /// 走 `(feed_id, read)` 覆盖索引（`INDEXED BY` 钉住，口径同
    /// [`Store::unread_summary`]）：COUNT 绝不触碰正文大列所在的表 B 树。
    pub fn entry_count_for_feed(&self, feed_id: i64) -> Result<i64> {
        Ok(self
            .conn
            .query_row(ENTRY_COUNT_FOR_FEED_SQL, params![feed_id], |r| r.get(0))?)
    }

    /// [`Store::entry_count_for_feed`] 的 EXPLAIN 断言入口（与线上 SQL 逐字同源）。
    #[doc(hidden)]
    pub fn explain_entry_count_for_feed(&self, feed_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {ENTRY_COUNT_FOR_FEED_SQL}"))?;
        let rows = stmt.query_map(params![feed_id], |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 单个分组下的条目数（列表头「已加载 M / 共 N」的分组维度；界面 `list_scope_total`）。
    ///
    /// 口径与 [`Store::entry_count_for_feed`] / [`Store::unread_summary`] 一致：
    /// 只扫 `(feed_id, read)` 覆盖索引，不碰正文大列所在的表 B 树；未分组的源不计入
    /// 任何分组（同侧栏口径）。
    pub fn entry_count_for_folder(&self, folder_id: i64) -> Result<i64> {
        Ok(self
            .conn
            .query_row(ENTRY_COUNT_FOR_FOLDER_SQL, params![folder_id], |r| {
                r.get(0)
            })?)
    }

    /// [`Store::entry_count_for_folder`] 的 EXPLAIN 断言入口（与线上 SQL 逐字同源）。
    #[doc(hidden)]
    pub fn explain_entry_count_for_folder(&self, folder_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {ENTRY_COUNT_FOR_FOLDER_SQL}"))?;
        let rows = stmt.query_map(params![folder_id], |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

    /// 条件级写入的 EXPLAIN 断言入口（与线上 SQL 同源，口径同 [`Store::explain_counts`]）。
    #[doc(hidden)]
    pub fn explain_set_flag_scoped(
        &self,
        flag: EntryFlag,
        scope: &EntryFlagScope,
    ) -> Result<Vec<String>> {
        let Some((where_sql, values)) = scope.where_sql() else {
            return Err(StoreError::Invalid(
                "条件级写入至少需要一个条件（feed_id / since / until）".to_string(),
            ));
        };
        let sql = format!(
            "EXPLAIN QUERY PLAN UPDATE entries SET {} = 1 WHERE {where_sql}",
            flag.column()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        // EXPLAIN 也要绑定同数量的参数（SQLite 在 prepare 阶段就数 `?`）
        let rows = stmt.query_map(params_from_iter(values), |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 条件级批量标记：目标 = `feed_id`（可选）+ 时间范围（闭区间，键同列表排序键）。
    ///
    /// 返回**命中条数**，含此前已是目标状态的行——与 `set_read(ids)` 同口径
    /// （SQLite 的 UPDATE 计数是「匹配到的行」而不是「值真的变了的行」），所以
    /// 重复调用返回值稳定：幂等 = 第二次仍是同一个数、且不报错。
    ///
    /// 空范围返回 `Invalid`（**不**退化成「全部」）：条件级写入漏参的后果是改全库。
    pub fn set_flag_scoped(
        &self,
        flag: EntryFlag,
        value: bool,
        scope: &EntryFlagScope,
    ) -> Result<usize> {
        let Some((where_sql, mut values)) = scope.where_sql() else {
            return Err(StoreError::Invalid(
                "条件级写入至少需要一个条件（feed_id / since / until）".to_string(),
            ));
        };
        let sql = format!("UPDATE entries SET {} = ? WHERE {where_sql}", flag.column());
        // 值参数排在 WHERE 之前：`?` 按出现顺序绑定
        values.insert(0, Value::Integer(i64::from(value)));
        Ok(self.conn.execute(&sql, params_from_iter(values))?)
    }

    /// 条件级命中的 id 采样（**有界**，按 id 升序）：写工具的「逐项结果」用它，
    /// 不去物化整个命中集（条件可能命中上万条，倒出去只会撑爆 agent 上下文）。
    ///
    /// 与 [`Store::set_flag_scoped`] 共用同一处 WHERE：采样看到的就是会被写入的那批行。
    pub fn entry_ids_scoped(&self, scope: &EntryFlagScope, limit: usize) -> Result<Vec<i64>> {
        let Some((where_sql, mut values)) = scope.where_sql() else {
            return Err(StoreError::Invalid(
                "条件级查询至少需要一个条件（feed_id / since / until）".to_string(),
            ));
        };
        let sql = format!("SELECT id FROM entries WHERE {where_sql} ORDER BY id LIMIT ?");
        values.push(Value::Integer(limit as i64));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 批量存在性检查：返回 `ids` 里真实存在的（去重、升序）。
    ///
    /// 写工具据此逐项回 `article_not_found`——只看 UPDATE 的条数变少无法说明「哪几条没成」。
    pub fn existing_entry_ids(&self, ids: &[i64]) -> Result<Vec<i64>> {
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<i64> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
        let mut out = Vec::with_capacity(unique.len());
        // SQLite 的变量上限是 999，取 500 留余量（MCP 侧另有 ≤100 的批量闸门）
        for chunk in unique.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT id FROM entries WHERE id IN ({placeholders})");
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter().copied()), |r| {
                r.get::<_, i64>(0)
            })?;
            out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        out.sort_unstable();
        Ok(out)
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

    // ---------------------------------------------------------------- 标签

    /// 新建标签（`color` 可空 = 默认色）。
    ///
    /// - 名称 trim 后非空；重名（大小写不敏感）报 [`StoreError::DuplicateTagName`]；
    /// - `sort_order` 取「当前最大值 + 1」：新标签落在侧栏末尾，不会因为默认 0 而
    ///   插到拖拽排序过的列表中间；
    /// - `last_used_at` 建时为空——「用过」只由 [`Store::assign_tags`] 推进，
    ///   于是选择器的「最近使用优先」不会被“建了一堆没用过”的标签带偏。
    pub fn create_tag(&self, name: &str, color: Option<&str>) -> Result<TagRow> {
        let name = normalize_tag_name(name)?;
        let color = normalize_tag_color(color)?;
        if self.tag_id_by_name(&name)?.is_some() {
            return Err(StoreError::DuplicateTagName(name));
        }
        let inserted = self.conn.execute(
            "INSERT INTO tags (name, color, pinned, sort_order, last_used_at, created_at)
             VALUES (?1, ?2, 0, (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM tags), NULL, ?3)",
            params![name, color, now()],
        );
        match inserted {
            Ok(_) => {}
            // 另一进程（桌面端与 MCP 共用同一个库文件）抢先建了同名标签：仍回可读错误
            Err(e) if unique_violation(&e) => return Err(StoreError::DuplicateTagName(name)),
            Err(e) => return Err(e.into()),
        }
        self.tag_row(self.conn.last_insert_rowid())?
            .ok_or_else(|| StoreError::Invalid("新建的标签读不出来".into()))
    }

    /// 重命名标签。名称口径同 [`Store::create_tag`]：大小写不敏感唯一，
    /// 但**只改大小写**（`rust` → `Rust`）允许（唯一约束排除自身）。
    pub fn rename_tag(&self, tag_id: i64, name: &str) -> Result<TagRow> {
        let name = normalize_tag_name(name)?;
        let dup: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM tags WHERE name = ?1 AND id != ?2",
                params![name, tag_id],
                |r| r.get::<_, i64>(0),
            )
            .ok();
        if dup.is_some() {
            return Err(StoreError::DuplicateTagName(name));
        }
        match self.conn.execute(
            "UPDATE tags SET name = ?1 WHERE id = ?2",
            params![name, tag_id],
        ) {
            Ok(0) => return Err(StoreError::TagNotFound(tag_id)),
            Ok(_) => {}
            Err(e) if unique_violation(&e) => return Err(StoreError::DuplicateTagName(name)),
            Err(e) => return Err(e.into()),
        }
        self.tag_row(tag_id)?.ok_or(StoreError::TagNotFound(tag_id))
    }

    /// 设置 / 清除颜色（`None` 或空白 = 清除，回到默认色）。
    pub fn set_tag_color(&self, tag_id: i64, color: Option<&str>) -> Result<TagRow> {
        let color = normalize_tag_color(color)?;
        let value = color.map(Value::Text).unwrap_or(Value::Null);
        self.update_tag(tag_id, "color", value)?;
        self.tag_row(tag_id)?.ok_or(StoreError::TagNotFound(tag_id))
    }

    /// 置顶开关（侧栏置顶优先）。
    pub fn set_tag_pinned(&self, tag_id: i64, pinned: bool) -> Result<TagRow> {
        self.update_tag(tag_id, "pinned", Value::Integer(i64::from(pinned)))?;
        self.tag_row(tag_id)?.ok_or(StoreError::TagNotFound(tag_id))
    }

    /// 标签清单（侧栏顺序：置顶优先 → 手动顺序 → 名称）。含未读计数、颜色、
    /// 置顶、`sort_order`、`last_used_at`。
    pub fn list_tags(&self) -> Result<Vec<TagRow>> {
        self.query_tags(TAG_SIDEBAR_ORDER)
    }

    /// 标签清单（选择器顺序：`last_used_at DESC`，没记录过的垫底 → 手动顺序 → 名称）。
    ///
    /// 与 [`Store::list_tags`] 同一份 SQL 与字段（只有 ORDER BY 不同），因此选择器与
    /// 侧栏不会看到两套字段口径。
    pub fn list_tags_recent_first(&self) -> Result<Vec<TagRow>> {
        self.query_tags(TAG_RECENT_ORDER)
    }

    /// 单个标签行（写操作后回给界面，用法同 [`Store::feed_row`]）。
    pub fn tag_row(&self, tag_id: i64) -> Result<Option<TagRow>> {
        let sql = format!("{TAG_ROW_SELECT} WHERE t.id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![tag_id], tag_row_from)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// 删除标签：只清关联，**不删文章**。`dry_run = true` 只回报影响篇数、不落库。
    ///
    /// 两个分支的 `affected_entries` 都来自 [`Store::tag_entry_count`]（同源），
    /// 所以确认弹窗上的数字与真删的影响面不可能两套。
    pub fn delete_tag(&self, tag_id: i64, dry_run: bool) -> Result<DeleteTagReport> {
        if self.tag_row(tag_id)?.is_none() {
            return Err(StoreError::TagNotFound(tag_id));
        }
        let affected_entries = self.tag_entry_count(tag_id)?;
        if dry_run {
            return Ok(DeleteTagReport {
                affected_entries,
                dry_run: true,
            });
        }
        let tx = self.conn.unchecked_transaction()?;
        // 显式清关联（FK CASCADE 也在，但连接级 pragma 不该成为不变量的前提——
        // 两道保险让「零孤儿」在本程序自己的删除路径上无条件成立）
        tx.execute("DELETE FROM entry_tags WHERE tag_id = ?1", params![tag_id])?;
        tx.execute("DELETE FROM tags WHERE id = ?1", params![tag_id])?;
        tx.commit()?;
        Ok(DeleteTagReport {
            affected_entries,
            dry_run: false,
        })
    }

    /// 标签关联篇数：`delete_tag` 的 `dry_run` 与实际执行共用这一个函数。
    ///
    /// 只扫 `idx_entry_tags_tag` 覆盖索引，不碰 `entries` 表 B 树（EXPLAIN 断言与
    /// 变异校验见 [`Store::explain_tag_entry_count`]）。
    pub fn tag_entry_count(&self, tag_id: i64) -> Result<i64> {
        Ok(self
            .conn
            .query_row(TAG_ENTRY_COUNT_SQL, params![tag_id], |r| r.get(0))?)
    }

    /// [`Store::tag_entry_count`] 的 EXPLAIN 断言入口（与线上 SQL 逐字同源）。
    #[doc(hidden)]
    pub fn explain_tag_entry_count(&self, tag_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {TAG_ENTRY_COUNT_SQL}"))?;
        let rows = stmt.query_map(params![tag_id], |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// [`Store::list_tags`] 的 EXPLAIN 断言入口（含每个标签的未读计数子查询，
    /// 与线上 SQL 逐字同源）。
    #[doc(hidden)]
    pub fn explain_tag_list(&self) -> Result<Vec<String>> {
        let sql = format!("{TAG_ROW_SELECT}{TAG_SIDEBAR_ORDER}");
        let mut stmt = self.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 给条目附加标签（批量 ≤100 或条件级；幂等；同事务推进 `last_used_at`）。
    pub fn assign_tags(&self, target: &TagTarget, tag_ids: &[i64]) -> Result<TagAssignReport> {
        self.tag_link(target, tag_ids, true)
    }

    /// 给条目移除标签（目标与幂等口径同 [`Store::assign_tags`]；**不**推进
    /// `last_used_at`——「最近使用」只由打标推进，见 [`Store::assign_tags`]）。
    pub fn unassign_tags(&self, target: &TagTarget, tag_ids: &[i64]) -> Result<TagAssignReport> {
        self.tag_link(target, tag_ids, false)
    }

    /// 批量重排：列表下标即新的 `sort_order`（小的在前），**单事务**写入——
    /// 中途碰到不存在的标签 id 则整体回滚（不允许「重排了一半」）。
    ///
    /// 同一 id 只认第一次出现的位置；空列表是空操作。未在列表里的标签保留原顺序值
    /// （界面拖拽给的是整份可见列表）。
    pub fn reorder_tags(&self, ids_in_order: &[i64]) -> Result<()> {
        let mut ids: Vec<i64> = Vec::with_capacity(ids_in_order.len());
        for id in ids_in_order {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        if ids.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        for (position, tag_id) in ids.iter().enumerate() {
            let n = tx.execute(
                "UPDATE tags SET sort_order = ?1 WHERE id = ?2",
                params![position as i64, tag_id],
            )?;
            if n == 0 {
                return Err(StoreError::TagNotFound(*tag_id));
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 单个条目的标签（名称序）——chips 渲染与测试的观察口。
    pub fn entry_tags(&self, entry_id: i64) -> Result<Vec<TagBrief>> {
        let mut map = self.entry_tags_map(&[entry_id])?;
        Ok(map.remove(&entry_id).unwrap_or_default())
    }

    /// 测试断言口：孤儿 `entry_tags` 行数（两侧都要能在主子表里找到才算合法）。
    ///
    /// 直接查而不是靠 FK 报错：即使某个连接没开 `PRAGMA foreign_keys`，不变量也照样能验。
    #[doc(hidden)]
    pub fn orphan_entry_tag_count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM entry_tags et
              WHERE NOT EXISTS (SELECT 1 FROM entries e WHERE e.id = et.entry_id)
                 OR NOT EXISTS (SELECT 1 FROM tags t WHERE t.id = et.tag_id)",
            [],
            |r| r.get(0),
        )?)
    }

    /// 测试断言口：本连接是否真的开着外键（连接级 pragma，`Store::init` 里打开）。
    #[doc(hidden)]
    pub fn foreign_keys_enabled(&self) -> Result<bool> {
        Ok(self
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))?)
    }

    /// 单列标签字段更新（`color` / `pinned` 共用）：不存在即报 [`StoreError::TagNotFound`]。
    /// `column` 只来自本模块内的字面量，不拼接外部输入。
    fn update_tag(&self, tag_id: i64, column: &str, value: Value) -> Result<()> {
        let n = self.conn.execute(
            &format!("UPDATE tags SET {column} = ?1 WHERE id = ?2"),
            params![value, tag_id],
        )?;
        if n == 0 {
            return Err(StoreError::TagNotFound(tag_id));
        }
        Ok(())
    }

    fn query_tags(&self, order_by: &str) -> Result<Vec<TagRow>> {
        let sql = format!("{TAG_ROW_SELECT}{order_by}");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], tag_row_from)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn tag_id_by_name(&self, name: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                // 列上的 COLLATE NOCASE 让比较就是「大小写不敏感」的那个口径
                "SELECT id FROM tags WHERE name = ?1",
                params![name],
                |r| r.get::<_, i64>(0),
            )
            .ok())
    }

    fn ensure_tag_exists(&self, tag_id: i64) -> Result<()> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE id = ?1)",
            params![tag_id],
            |r| r.get(0),
        )?;
        if exists {
            Ok(())
        } else {
            Err(StoreError::TagNotFound(tag_id))
        }
    }

    /// `assign_tags` / `unassign_tags` 的共用实现（单事务）。
    ///
    /// - 关联写入用 `INSERT OR IGNORE`：重复附加无副作用（幂等），返回值是**真正
    ///   新增**的行数（不是「匹配到的行数」）；
    /// - `last_used_at` 与关联写入在**同一个事务**里推进，且只在真的改变了关联时
    ///   才推进——于是纯重复调用是严格零副作用。这是选择器「最近使用优先」的唯一
    ///   数据来源；
    /// - 只有**打标**（`link = true`）推进 `last_used_at`：PRD FR-1 / tech_design
    ///   schema 注释与 `Store::create_tag` 的文档口径都是「打标时更新」，取消打标
    ///   不算一次使用（否则刚移除的标签会跳到选择器最前）。
    /// - 不存在的标签 id 直接报 [`StoreError::TagNotFound`]（静默忽略会让调用方
    ///   以为打上了）；不存在的条目 id 静默跳过（`SELECT ... FROM entries` 只出真实
    ///   存在的行，也顺便免掉 FK 报错）。
    fn tag_link(&self, target: &TagTarget, tag_ids: &[i64], link: bool) -> Result<TagAssignReport> {
        let tag_ids = dedupe_ascending(tag_ids);
        if tag_ids.is_empty() {
            return Err(StoreError::Invalid("至少要给一个标签 id".into()));
        }
        for tag_id in &tag_ids {
            self.ensure_tag_exists(*tag_id)?;
        }
        let (entry_where, entry_values) = target_entry_where(target)?;
        let tag_placeholders = vec!["?"; tag_ids.len()].join(",");
        let tx = self.conn.unchecked_transaction()?;
        let entries: i64 = tx.query_row(
            &format!("SELECT COUNT(*) FROM entries e WHERE {entry_where}"),
            params_from_iter(entry_values.iter().cloned()),
            |r| r.get(0),
        )?;
        let tag_values = || -> Vec<Value> {
            let mut values: Vec<Value> = tag_ids.iter().map(|id| Value::Integer(*id)).collect();
            values.extend(entry_values.iter().cloned());
            values
        };
        let changed = if link {
            tx.execute(
                &format!(
                    "INSERT OR IGNORE INTO entry_tags (entry_id, tag_id)
                     SELECT e.id, t.id FROM tags t CROSS JOIN entries e
                      WHERE t.id IN ({tag_placeholders}) AND {entry_where}"
                ),
                params_from_iter(tag_values()),
            )?
        } else {
            tx.execute(
                &format!(
                    "DELETE FROM entry_tags WHERE tag_id IN ({tag_placeholders})
                       AND entry_id IN (SELECT e.id FROM entries e WHERE {entry_where})"
                ),
                params_from_iter(tag_values()),
            )?
        };
        if changed > 0 && link {
            let mut values: Vec<Value> = vec![Value::Integer(now())];
            values.extend(tag_ids.iter().map(|id| Value::Integer(*id)));
            tx.execute(
                &format!("UPDATE tags SET last_used_at = ?1 WHERE id IN ({tag_placeholders})"),
                params_from_iter(values),
            )?;
        }
        tx.commit()?;
        Ok(TagAssignReport {
            changed,
            entries,
            tag_ids,
        })
    }

    /// 给整页条目行填标签（`EntryRow::tags`）：一次查询覆盖整页，不做每行一次。
    ///
    /// 列表 / 搜索 / 阅读页都收口在 [`Store::query_entries`]，所以三处口径一致。
    fn fill_entry_tags(&self, rows: &mut [EntryRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        let mut map = self.entry_tags_map(&ids)?;
        for row in rows.iter_mut() {
            row.tags = map.remove(&row.id).unwrap_or_default();
        }
        Ok(())
    }

    /// `entry_id -> 标签（名称序）`。按条目取关联走主键 `(entry_id, tag_id)` 的隐式
    /// 索引；条目多时分批绑定（SQLite 变量上限 999，取 500 留余量，同
    /// [`Store::existing_entry_ids`]）。
    fn entry_tags_map(&self, entry_ids: &[i64]) -> Result<HashMap<i64, Vec<TagBrief>>> {
        let mut out: HashMap<i64, Vec<TagBrief>> = HashMap::new();
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<i64> = entry_ids
            .iter()
            .copied()
            .filter(|id| seen.insert(*id))
            .collect();
        for chunk in unique.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT et.entry_id, t.id, t.name
                   FROM entry_tags et JOIN tags t ON t.id = et.tag_id
                  WHERE et.entry_id IN ({placeholders})
                  ORDER BY t.name COLLATE NOCASE, t.id"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter().copied()), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    TagBrief {
                        id: r.get(1)?,
                        name: r.get(2)?,
                    },
                ))
            })?;
            for row in rows {
                let (entry_id, brief) = row?;
                out.entry(entry_id).or_default().push(brief);
            }
        }
        Ok(out)
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

    /// 删除一个设置键（幂等：键本来就不存在也算成功）。
    ///
    /// 用于「销毁」类能力（如 MCP 写 token）：键消失 = 能力不存在，
    /// 比写空串少一层「空值算不算没有」的歧义。
    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(())
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

    /// 列表排序档（`list.sort`）：缺失或非法值一律回默认 [`ListSort::Newest`]。
    ///
    /// 读取侧的兜底与主题/语言同一口径：库里被写坏也不至于让列表打不开。
    pub fn list_sort(&self) -> ListSort {
        ListSort::from_setting(self.setting(LIST_SORT_KEY).ok().flatten().as_deref())
    }

    /// 「隐藏已读」开关（`list.hide_read`）：缺失或非法值回默认关。
    pub fn list_hide_read(&self) -> bool {
        self.bool_setting(LIST_HIDE_READ_KEY, false).unwrap_or(false)
    }

    /// 每个源的未读数
    pub fn unread_by_feed(&self) -> Result<Vec<(i64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT feed_id, COUNT(*) FROM entries WHERE read = 0 GROUP BY feed_id ORDER BY feed_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 未读聚合：按 [`UnreadGroupBy::Feed`] 或 [`UnreadGroupBy::Folder`] 出每组未读数。
    ///
    /// 口径：
    /// - 组集合是**完整**的：全部订阅源（或全部分组，含没订阅的空组）都在结果里，
    ///   未读为 0 也照出——调用方不必自己补齐空组，也不会因为「没出现在结果里」
    ///   而误判成「不存在」；
    /// - `Folder` 档把 `folder_id IS NULL` 的订阅合成「未分组」一组，垫在最后
    ///   （与侧栏顺序一致），空组也出（unread = 0）；
    /// - 计数走覆盖索引（见 `UNREAD_COUNT_BY_FEED_SQL` / `UNREAD_COUNT_BY_FOLDER_SQL`），
    ///   绝不碰正文大列所在的表 B 树——这是 `counts()` 那条教训的同一口径。
    pub fn unread_summary(&self, by: UnreadGroupBy) -> Result<Vec<UnreadGroup>> {
        let counts: HashMap<Option<i64>, i64> = self.unread_counts(by)?.into_iter().collect();
        let count_of = |key: Option<i64>| counts.get(&key).copied().unwrap_or(0);
        match by {
            UnreadGroupBy::Feed => {
                let mut stmt = self.conn.prepare(FEED_NAMES_SQL)?;
                let rows = stmt.query_map([], |r| {
                    Ok(UnreadGroup {
                        id: Some(r.get(0)?),
                        name: r.get(1)?,
                        unread: 0,
                    })
                })?;
                let mut groups = rows.collect::<rusqlite::Result<Vec<_>>>()?;
                for g in &mut groups {
                    g.unread = count_of(g.id);
                }
                Ok(groups)
            }
            UnreadGroupBy::Folder => {
                let mut groups: Vec<UnreadGroup> = self
                    .list_folders_ordered()?
                    .into_iter()
                    .map(|f| UnreadGroup {
                        unread: count_of(Some(f.id)),
                        id: Some(f.id),
                        name: f.name,
                    })
                    .collect();
                // 未分组是「始终存在」的一类：只要库里有未分组订阅就出这一组
                let has_ungrouped: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM feeds WHERE folder_id IS NULL)",
                    [],
                    |r| r.get(0),
                )?;
                if has_ungrouped {
                    groups.push(UnreadGroup {
                        id: None,
                        name: UNGROUPED_LABEL.to_string(),
                        unread: count_of(None),
                    });
                }
                Ok(groups)
            }
        }
    }

    /// `unread_summary` 的 EXPLAIN 断言入口（与线上 SQL 同源：走的还是那两个常量）。
    ///
    /// 只解释**计数** SQL——名字来自 `feeds` / `folders` 两张小表（自身不带正文大列），
    /// 需要防退化的就是这里。
    #[doc(hidden)]
    pub fn explain_unread_summary(&self, by: UnreadGroupBy) -> Result<Vec<String>> {
        let sql = unread_count_sql(by);
        let mut stmt = self.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(3))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 未读计数：`(分组键, 未读数)`。分组键的 `None` 只在 folder 档出现（未分组）。
    fn unread_counts(&self, by: UnreadGroupBy) -> Result<Vec<(Option<i64>, i64)>> {
        let mut stmt = self.conn.prepare(unread_count_sql(by))?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?))
        })?;
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

/// 单源条目数的 SQL（MCP 退订影响面）：`INDEXED BY` 钉住 `(feed_id, read)` 覆盖索引，
/// COUNT 只扫索引 B 树、不回表；抽成常量让 EXPLAIN 断言与线上 SQL 逐字同源。
const ENTRY_COUNT_FOR_FEED_SQL: &str =
    "SELECT COUNT(*) FROM entries INDEXED BY idx_entries_feed_read WHERE feed_id = ?1";

/// 分组条目总数的 SQL（列表头「已加载 M / 共 N」的分组维度）。
///
/// 形状同 [`UNREAD_COUNT_BY_FOLDER_SQL`]，但**不带 read 谓词**：钉
/// `idx_entries_feed_read` 扫 (feed_id, read) 覆盖索引，再拿 feed_id 回 `feeds`
/// （小表、PK 查）要 folder_id——COUNT 不穿正文大列所在的表 B 树（仓库红线 #1）。
const ENTRY_COUNT_FOR_FOLDER_SQL: &str = "SELECT COUNT(*) FROM entries e INDEXED BY idx_entries_feed_read
      JOIN feeds f ON f.id = e.feed_id WHERE f.folder_id = ?1";

/// 标签关联篇数的 SQL（`delete_tag` 的 dry_run 与实际执行**共用**）：
/// `INDEXED BY` 钉住 `(tag_id, entry_id)` 覆盖索引——COUNT 只扫 entry_tags 的索引，
/// 根本不碰 entries 表 B 树。抽成常量让 EXPLAIN 断言与线上 SQL 逐字同源。
const TAG_ENTRY_COUNT_SQL: &str =
    "SELECT COUNT(*) FROM entry_tags INDEXED BY idx_entry_tags_tag WHERE tag_id = ?1";

/// 标签行 + 未读计数的列清单（[`Store::list_tags`] / [`Store::tag_row`] 共用，
/// 新增列只改这里——同 `FEED_ROW_SELECT` 的道理）。
///
/// 未读计数子查询里的两个 `INDEXED BY` 都是性能红线的钉子：
/// - 内层 `idx_entry_tags_tag(tag_id, entry_id)` 给出「这个标签的条目 id 集」；
/// - 外层 `idx_entries_unread_id(id) WHERE read = 0`（部分索引，只含未读行）按 rowid
///   走覆盖索引——「这行未读」就是索引的存在性事实，不需要回表。`read` 列在 entries 里
///   排在 11.5KB 正文大列之后，回表取它就要穿溢出页链（counts() 教训：冷启动
///   83-119ms/次）。两个索引任一被删，`explain_tag_list` 直接报错（变异校验）。
const TAG_ROW_SELECT: &str = "\
    SELECT t.id, t.name, t.color, t.pinned, t.sort_order, t.last_used_at,
           (SELECT COUNT(*) FROM entries e INDEXED BY idx_entries_unread_id
             WHERE e.read = 0
               AND e.id IN (SELECT et.entry_id FROM entry_tags et
                             INDEXED BY idx_entry_tags_tag WHERE et.tag_id = t.id)) AS unread
      FROM tags t";

/// 侧栏标签区顺序：置顶优先 → 手动顺序 → 名称（不区分大小写，与源名同口径）。
const TAG_SIDEBAR_ORDER: &str = " ORDER BY t.pinned DESC, t.sort_order, t.name COLLATE NOCASE";

/// 选择器顺序：最近使用优先（没记录过的垫底）→ 手动顺序 → 名称。
/// `last_used_at IS NULL` 先升序把「没用过」排到后面，再按时间倒序。
const TAG_RECENT_ORDER: &str =
    " ORDER BY t.last_used_at IS NULL, t.last_used_at DESC, t.sort_order, t.name COLLATE NOCASE";

/// 未读聚合（[`Store::unread_summary`]）的分组维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreadGroupBy {
    /// 按订阅源
    Feed,
    /// 按文件夹（未分组的订阅合并为 `id = None` 的一组）
    Folder,
}

/// 未读聚合的一行（[`Store::unread_summary`]）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnreadGroup {
    /// 分组身份：`Feed` 档是订阅源 id；`Folder` 档是文件夹 id，`None` = 未分组
    pub id: Option<i64>,
    /// 显示名（源口径 `COALESCE(custom_title, title)`，与界面同一个表达式）
    pub name: String,
    /// 该组未读数（0 也照出：组集合完整，调用方不补空组）
    pub unread: i64,
}

/// 未分组订阅那一组的显示名（与界面 `menu.ungrouped` / `feedEdit.ungrouped` 同词）。
const UNGROUPED_LABEL: &str = "未分组";

/// 源显示名清单（未读聚合只借它取名字；`feeds` 是小表，不涉正文大列）。
/// 排序口径与 `list_feeds` 一致（显示名不区分大小写）。
const FEED_NAMES_SQL: &str = "SELECT f.id, COALESCE(f.custom_title, f.title) AS name
      FROM feeds f ORDER BY name COLLATE NOCASE";

/// 按源聚合未读的计数 SQL。用到的列（feed_id / read）都在 `idx_entries_feed_read (feed_id, read)`
/// 里 → 覆盖索引扫描，绝不碰正文大列所在的表 B 树（`counts()` 那条教训的同一口径）。
///
/// 为什么钉 `INDEXED BY` 而不是交给 planner：
/// - 本程序从不 ANALYZE，计划不该看统计脸色；
/// - 实测（无统计的新库）planner 会选同样覆盖的 `idx_entries_unread_sortkey`，
///   虽然也不碰表 B 树，但排序键不是分组键，要多一次 `USE TEMP B-TREE FOR GROUP BY`；
///   钉住本索引后索引序就是 GROUP BY 键，免掉那次重排。
const UNREAD_COUNT_BY_FEED_SQL: &str = "SELECT e.feed_id, COUNT(*) FROM entries e INDEXED BY idx_entries_feed_read
      WHERE e.read = 0 GROUP BY e.feed_id";

/// 按文件夹聚合未读的计数 SQL。同样钉 `idx_entries_feed_read`：先按 (feed_id, read)
/// 覆盖索引扫出未读行，再拿 feed_id 回 `feeds` 查（小表，PK 查）要 folder_id。
const UNREAD_COUNT_BY_FOLDER_SQL: &str = "SELECT f.folder_id, COUNT(*) FROM entries e INDEXED BY idx_entries_feed_read
      JOIN feeds f ON f.id = e.feed_id WHERE e.read = 0 GROUP BY f.folder_id";

fn unread_count_sql(by: UnreadGroupBy) -> &'static str {
    match by {
        UnreadGroupBy::Feed => UNREAD_COUNT_BY_FEED_SQL,
        UnreadGroupBy::Folder => UNREAD_COUNT_BY_FOLDER_SQL,
    }
}

const ENTRY_SELECT: &str = "SELECT e.id, e.feed_id, COALESCE(f.custom_title, f.title) AS feed_title,
        e.stable_id, e.id_origin, e.title,
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
const ENTRY_LIST_COLUMNS: &str =
    "SELECT e.id, e.feed_id, COALESCE(f.custom_title, f.title) AS feed_title,
        e.stable_id, e.id_origin, e.title,
        e.url, e.author, e.published_at, e.summary, NULL, NULL,
        e.read, e.starred, e.read_later, COALESCE(e.published_at, e.fetched_at) AS sortkey";

/// 列表/搜索查询的公共主体（列 + 条目 JOIN 订阅源，列表要源标题）。
///
/// `pin = Some(档位)` 时在 FROM 上钉死该档的排序索引；`None` 交给 planner
/// （搜索按相关度排序，没有可钉的排序索引）。
fn entry_list_sql(pin: Option<ListSort>) -> String {
    let hint = match pin {
        Some(ListSort::UnreadFirst) => " INDEXED BY idx_entries_unread_sortkey",
        Some(ListSort::Newest | ListSort::Oldest) => " INDEXED BY idx_entries_sortkey",
        None => "",
    };
    format!("{ENTRY_LIST_COLUMNS} FROM entries e{hint} JOIN feeds f ON f.id = e.feed_id")
}

/// 某一档的 `ORDER BY`：**列序与方向都与该档钉住的索引逐列对应**（这是
/// `USE TEMP B-TREE FOR ORDER BY` 不出现的前提，见 `list_entries_sql` 的注释）。
fn list_order_by(sort: ListSort) -> &'static str {
    match sort {
        ListSort::Newest => {
            " ORDER BY COALESCE(e.published_at, e.fetched_at) DESC, e.id DESC LIMIT ?"
        }
        ListSort::Oldest => {
            // 升序 = 反扫同一个表达式索引（SQLite 支持逆序扫描，不需要第二个索引）
            " ORDER BY COALESCE(e.published_at, e.fetched_at) ASC, e.id ASC LIMIT ?"
        }
        ListSort::UnreadFirst => {
            // 未读组内仍是最新在前；末列 id 用升序与 v11 索引的末列一致。
            // 并列关系（同 read 同 sortkey）下方向本身无意义，但必须与索引一致，
            // 否则 SQLite 只能临时排序。
            " ORDER BY e.read ASC, COALESCE(e.published_at, e.fetched_at) DESC, e.id ASC LIMIT ?"
        }
    }
}

/// `list_entries` 的 SQL 与参数（排序档 / 隐藏已读 / 游标续扫条件都在这里拼接）。
///
/// 抽成独立函数是为了让 EXPLAIN 断言与线上 SQL 逐字同源（见 `explain_list_entries`）；
/// `sort` 与 `hide_read` 由调用方从设置读出传入（测试里先写设置再断言计划，
/// 验的就是线上真会跑到的 SQL）。
fn list_entries_sql(q: &EntryQuery, sort: ListSort, hide_read: bool) -> (String, Vec<Value>) {
    // INDEXED BY 的必要性（实测，sqlite 3.53.2，无 sqlite_stat1 的新库）：
    // - `newest`/`oldest` 的续页：不过滤时靠统计也能选中 idx_entries_sortkey，但
    //   read/feed_id 这类带等值索引的筛选形态，planner 会改选等值索引 + 临时排序
    //   （本程序从不 ANALYZE）。续页是逐页路径，计划不能看统计的脸色。
    // - `unread_first`：首屏也钉。它的 ORDER BY 只有 v11 复合索引能同时满足顺序与
    //   `read` 等值筛选；不钉的话 feed 视图退化成 idx_entries_feed_read +
    //   `USE TEMP B-TREE FOR LAST 2 TERMS OF ORDER BY`（每页重排一遍筛选集合）。
    // - `newest`/`oldest` + 多源 `IN`：同样钉住。实测（sqlite 3.x，无 ANALYZE 的新库）
    //   feed_ids 会把 planner 引到 `idx_entries_feed_read (feed_id=?)` +
    //   `USE TEMP B-TREE FOR ORDER BY`——每页把筛选集合重排一遍；钉排序索引后变成
    //   按序 SEARCH + 逐行过滤 feed_id（同游标续扫的道理）。
    // - `newest`/`oldest` + 标签过滤：同理钉住（否则 planner 会驱动 `idx_entry_tags_tag`
    //   再临时排序）。标签的条目 id 集由 `idx_entry_tags_tag` 的子查询给出，外层扫的是
    //   排序索引——不得退化成 `SCAN entries`（见 `explain_list_entries` 的标签用例）。
    let pin = match sort {
        ListSort::UnreadFirst => Some(sort),
        ListSort::Newest | ListSort::Oldest => {
            if q.cursor.is_some() || q.feed_ids.is_some() || q.tag_id.is_some() {
                Some(sort)
            } else {
                None
            }
        }
    };
    let mut sql = entry_list_sql(pin);
    let mut values: Vec<Value> = Vec::new();
    sql.push_str(" WHERE 1=1");
    if let Some(feed_id) = q.feed_id {
        sql.push_str(" AND e.feed_id = ?");
        values.push(Value::Integer(feed_id));
    }
    if let Some(feed_ids) = &q.feed_ids {
        if feed_ids.is_empty() {
            // 空列表 = 匹配零条（"某分组下没有订阅"不能让查询退化成"不过滤"）。
            // 用常量假条件而不是 `IN ()`：后者是 SQL 语法错，且这样 EXPLAIN 仍可跑。
            sql.push_str(" AND 0");
        } else {
            let placeholders = vec!["?"; feed_ids.len()].join(",");
            sql.push_str(&format!(" AND e.feed_id IN ({placeholders})"));
            values.extend(feed_ids.iter().map(|id| Value::Integer(*id)));
        }
    }
    // 时间范围：对排序键同一个表达式（COALESCE(published_at, fetched_at)）比较，
    // 闭区间（since 含 / until 含）。绑定顺序与上面出现顺序逐一对齐。
    if let Some(since) = q.since {
        sql.push_str(" AND COALESCE(e.published_at, e.fetched_at) >= ?");
        values.push(Value::Integer(since));
    }
    if let Some(until) = q.until {
        sql.push_str(" AND COALESCE(e.published_at, e.fetched_at) <= ?");
        values.push(Value::Integer(until));
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
    // 标签过滤：存在性检查。两层索引各司其职——子查询用 `idx_entry_tags_tag`
    // 取「这个标签的条目 id 集」，外层继续按排序索引序取条目（LIMIT 一到就停）。
    // `INDEXED BY` 钉在子查询上：索引被删/被改名时 EXPLAIN 直接报错，而不是静默
    // 换成某个会穿正文大列表 B 树的计划（变异校验就是这么转红的）。
    if let Some(tag_id) = q.tag_id {
        sql.push_str(
            " AND e.id IN (SELECT et.entry_id FROM entry_tags et\n                 INDEXED BY idx_entry_tags_tag WHERE et.tag_id = ?)",
        );
        values.push(Value::Integer(tag_id));
    }
    // 隐藏已读：星标/稍后读视图豁免——「星标了但读完了」还要能找到（PRD 需求 3）。
    // 未读视图本来就有 `read = 0`，重复一次无害。
    if hide_read && !q.starred_only && !q.read_later_only {
        sql.push_str(" AND e.read = 0");
    }
    match sort {
        ListSort::Newest => {
            if let Some((sortkey, id)) = q.cursor {
                // keyset 续扫：语义就是行值比较 `(sortkey, id) < (?, ?)`。
                //
                // 实测（sqlite 3.53.2，8000 条库，带/不带 ANALYZE 统计都验过）：
                // 1. 行值比较不会被当成 idx_entries_sortkey 上的范围约束——计划退化成
                //    `SCAN ... USING INDEX`，续页得从索引头部扫到游标位置，逐页变慢；
                // 2. 只写展开式（下面的 OR）又会走 MULTI-INDEX OR + `USE TEMP B-TREE FOR ORDER BY`。
                // 因此这里多带一条冗余的前导范围约束 `sortkey <= ?`：它给表达式索引一个
                // 真正可用的 SEARCH 起点；括号内的 OR 把语义收回到严格小于（同值按 id 续扫）。
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
        }
        ListSort::Oldest => {
            if let Some((sortkey, id)) = q.cursor {
                // 与 newest 镜像：游标是「上一页末行」，续页取排在它后面的行，而升序档的
                // 「后面」是更大（键值更大）。冗余前导约束 `sortkey >= ?` 给反扫一个
                // SEARCH 起点，括号内 OR 把语义收回严格大于（同值按 id 继续）。
                sql.push_str(
                    " AND COALESCE(e.published_at, e.fetched_at) >= ?
                 AND (COALESCE(e.published_at, e.fetched_at) > ?
                      OR (COALESCE(e.published_at, e.fetched_at) = ? AND e.id > ?))",
                );
                values.push(Value::Integer(sortkey));
                values.push(Value::Integer(sortkey));
                values.push(Value::Integer(sortkey));
                values.push(Value::Integer(id));
            }
        }
        ListSort::UnreadFirst => {
            // 复合键游标 `(read, sortkey, id)`：排序键的第 1 列是先升后降的混合方向，
            // 不能用单条行值比较表达，因此拆成 read 的范围起点 + 「同 read 内的
            // (sortkey, id) 位置」两层；`read >= ?` 是显式的范围兜底（OR 分支里的
            // `read > ?` 实测也能给出同样的起点）。
            //
            // 计划成本：read 是索引首列，SEARCH 起点落在 read 边界上，页内按索引序
            // 往前走读到的都是 read/sortkey/id 三个索引列（不回表），命中行才取整行
            // ——与 v6 教训里「穿正文溢出页链的全表扫」不是一回事。
            if let (Some((sortkey, id)), Some(read)) = (q.cursor, q.cursor_read) {
                let read = i64::from(read);
                sql.push_str(
                    " AND e.read >= ?
                 AND (e.read > ? OR (e.read = ?
                      AND (COALESCE(e.published_at, e.fetched_at) < ?
                           OR (COALESCE(e.published_at, e.fetched_at) = ? AND e.id > ?))))",
                );
                values.push(Value::Integer(read));
                values.push(Value::Integer(read));
                values.push(Value::Integer(read));
                values.push(Value::Integer(sortkey));
                values.push(Value::Integer(sortkey));
                values.push(Value::Integer(id));
            }
        }
    }
    sql.push_str(list_order_by(sort));
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
        // 先置空，由 `Store::query_entries` 统一整页填充（映射函数拿不到库连接）
        tags: Vec::new(),
    })
}

fn origin_str(o: IdOrigin) -> &'static str {
    match o {
        IdOrigin::SourceData => "source_data",
        IdOrigin::ContentHash => "content_hash",
    }
}

fn tag_row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<TagRow> {
    Ok(TagRow {
        id: r.get(0)?,
        name: r.get(1)?,
        color: r.get(2)?,
        pinned: r.get::<_, i64>(3)? != 0,
        sort_order: r.get(4)?,
        last_used_at: r.get(5)?,
        unread: r.get(6)?,
    })
}

/// 标签名归一化：trim 后非空（空串报错而不是静默建成空名标签）。
fn normalize_tag_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StoreError::Invalid("标签名不能为空".into()));
    }
    Ok(name.to_string())
}

/// 颜色归一化：`None` / 空白 = 无颜色；否则必须是 `#RRGGBB`（大小写都收，
/// 统一小写落库，免得 `#FFF000` 与 `#fff000` 在比较时是两种值）。
fn normalize_tag_color(color: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = color else { return Ok(None) };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let valid =
        raw.len() == 7 && raw.starts_with('#') && raw[1..].chars().all(|c| c.is_ascii_hexdigit());
    if !valid {
        return Err(StoreError::Invalid(format!(
            "颜色格式应为 #RRGGBB（实收「{raw}」）"
        )));
    }
    Ok(Some(format!("#{}", raw[1..].to_ascii_lowercase())))
}

/// 唯一约束冲突判定：按错误码而不是文案（错误文案不是契约）。
fn unique_violation(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(e, _) if e.code == rusqlite::ErrorCode::ConstraintViolation)
}

/// 去重 + 升序：同一 id 传多次只算一次（写路径不该把重复当多次处理）。
fn dedupe_ascending(ids: &[i64]) -> Vec<i64> {
    let mut unique: Vec<i64> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    unique
}

/// 标签写操作的目标 → `entries e` 的 WHERE 片段 + 参数（assign / unassign / 计数
/// 三处共用，免得写入与计数各拼一份而漂移）。
fn target_entry_where(target: &TagTarget) -> Result<(String, Vec<Value>)> {
    match target {
        TagTarget::Entries(ids) => {
            let ids = dedupe_ascending(ids);
            if ids.is_empty() {
                return Err(StoreError::Invalid("批量条目 id 不能为空".into()));
            }
            if ids.len() > TAG_BATCH_MAX_IDS {
                return Err(StoreError::Invalid(format!(
                    "一次最多处理 {TAG_BATCH_MAX_IDS} 条，收到 {} 条：请分批调用（不要指望静默截断）",
                    ids.len()
                )));
            }
            let placeholders = vec!["?"; ids.len()].join(",");
            Ok((
                format!("e.id IN ({placeholders})"),
                ids.into_iter().map(Value::Integer).collect(),
            ))
        }
        TagTarget::Scope(scope) => scope.where_sql().ok_or_else(|| {
            StoreError::Invalid("条件级标签操作至少需要一个条件（feed_id / since / until）".into())
        }),
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
