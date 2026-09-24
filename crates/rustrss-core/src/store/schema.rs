//! 存储层 schema 与基线。
//!
//! 首发前把 13 条逐版本迁移**压平成一份基线**：开发期没有需要兼容的用户库，
//! 版本演进只带来长期维护面与测试面。基线里的 DDL 是「旧链终态」的等价形态
//! （表/列序/索引/部分索引 WHERE/触发器/FT​S 影子表逐项对齐，见 `tests/durability.rs`
//! 的结构化等价断言）。
//!
//! **老库不保**：旧开发库（旧链 v1..v13 的冻结库）与外来 sqlite 文件不提供兼容
//! 升级路径，由 [`BASELINE_APPLICATION_ID`]（SQLite `PRAGMA application_id`）识别并
//! 拒绝，见 [`detect`]。判定**不用 `user_version` 区间**——基线号是 1，而旧链也
//! 存在 v=1 的冻结库，按区间判断会同时误拒新库与漏放旧库。
//!
//! 基线一旦发布，后续改动**只能追加**新迁移（`MIGRATIONS` 追加，不修改既有条目）。

use rusqlite::Connection;

/// 基线库的应用标识（写入 `PRAGMA application_id`）。
///
/// 十六进制 `0x5253_5331` = ASCII `"RSS1"`。基线创建时写入；旧链任何版本都不写它
/// （值为 0），因此「有用户表但没有魔数」就是老库/外来文件的判据。
pub const BASELINE_APPLICATION_ID: i32 = 0x5253_5331;

/// 基线 schema 版本（`PRAGMA user_version`）。
pub const BASELINE_VERSION: i64 = 1;

/// 迁移列表：索引 i 对应 `user_version` i → i+1。
///
/// 首发前只含基线一条。基线发布后，只允许**追加**（新的索引 i+1），
/// 不得回头修改已发布的条目。
pub const MIGRATIONS: &[&str] = &[BASELINE];

/// 打开一个**已有**库时看到的 schema 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaState {
    /// 没有任何用户表：新库，可建基线（0 字节文件与新建空文件都落这里）。
    Fresh,
    /// 我们的基线库，且版本一致。
    Current,
    /// 有用户表但没有魔数：旧开发库（含旧链 v1 冻结库）或外来 sqlite 文件。
    Foreign { user_version: i64 },
    /// 我们的库，但版本与基线不一致（更新的版本，或被外部/中断操作弄成的状态）。
    VersionMismatch { user_version: i64 },
}

/// 探测库的 schema 状态。**只读**：不建表、不写 PRAGMA。
///
/// 调用方据此决定「建基线 / 直接打开 / 拒绝」（老库拒绝对外呈现见桌面与 MCP 侧）。
pub fn detect(conn: &Connection) -> rusqlite::Result<SchemaState> {
    let user_tables: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        [],
        |r| r.get(0),
    )?;
    let application_id: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(if user_tables == 0 {
        SchemaState::Fresh
    } else if application_id != BASELINE_APPLICATION_ID as i64 {
        SchemaState::Foreign { user_version }
    } else if user_version == BASELINE_VERSION {
        SchemaState::Current
    } else {
        SchemaState::VersionMismatch { user_version }
    })
}

/// 基线 DDL。列序即旧链终态的列序（追加顺序），不要按「逻辑分组」重排——
/// 结构化等价断言按 `PRAGMA table_info` 的顺序逐列比较。
///
/// 行内注释保留了各列的性能/语义理由（AGENTS.md 性能红线的依据），
/// 但不再保留「哪个版本加的」这类演进叙述。
const BASELINE: &str = r#"
    PRAGMA application_id = 0x52535331;

    CREATE TABLE folders (
        id       INTEGER PRIMARY KEY,
        name     TEXT NOT NULL UNIQUE,
        position INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE feeds (
        id              INTEGER PRIMARY KEY,
        url             TEXT NOT NULL UNIQUE,   -- 归一化后的订阅身份（RSSHub 存 rsshub://path，抓取时解析）
        title           TEXT NOT NULL,          -- 源站名（每次抓取 success 都会被刷新覆盖）
        site_url        TEXT,
        description     TEXT,
        language        TEXT,
        folder_id       INTEGER REFERENCES folders(id) ON DELETE SET NULL,
        -- 条件请求（ETag / Last-Modified）与抓取状态
        etag            TEXT,
        last_modified   TEXT,
        last_fetched_at INTEGER,
        last_status     TEXT,
        last_error      TEXT,
        created_at      INTEGER NOT NULL,
        -- 每源刷新间隔覆盖（分钟，NULL = 跟随全局档）；白名单校验在命令层
        refresh_interval_minutes INTEGER,
        -- 用户自定义标题：显示层读 COALESCE(custom_title, title)，NULL = 跟随源站名
        custom_title    TEXT,
        -- 429/503 的 Retry-After 期限；期限内不发请求
        retry_after_at  INTEGER,
        -- 组内手动排序；NULL = 名称序
        position        INTEGER
    );

    CREATE TABLE entries (
        id            INTEGER PRIMARY KEY,
        feed_id       INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
        stable_id     TEXT NOT NULL,
        id_origin     TEXT NOT NULL,
        title         TEXT NOT NULL,
        url           TEXT,
        author        TEXT,
        published_at  INTEGER,
        updated_at    INTEGER,
        summary       TEXT,
        content_html  TEXT,
        content_text  TEXT,
        -- 预分词后的检索字段：拉丁词 + 中文 bigram（FTS5 默认分词器处理不了中文）
        search_tokens TEXT NOT NULL DEFAULT '',
        -- 用于判断「这次刷新到底有没有变化」，避免无谓写入
        content_hash  TEXT NOT NULL DEFAULT '',
        read          INTEGER NOT NULL DEFAULT 0,
        starred       INTEGER NOT NULL DEFAULT 0,
        fetched_at    INTEGER NOT NULL,
        -- 稍后读标记
        read_later       INTEGER NOT NULL DEFAULT 0,
        -- 全文抓取标记位：摘要型条目一旦抓到原文正文就以此为准（刷新不再覆盖、重开零网络）
        fulltext_fetched INTEGER NOT NULL DEFAULT 0,
        -- 列表缩略图（Media RSS > 图片 enclosure > 摘要/正文首图；只存 HTTP(S)）
        thumbnail_url    TEXT,
        UNIQUE (feed_id, stable_id)
    );

    -- 列表排序键表达式索引：ORDER BY COALESCE(published_at, fetched_at) DESC 是函数列，
    -- 没有索引会全表扫 + 排序。末列 feed_id 让同 sortkey 的并列行也能被 keyset 游标稳定续扫。
    CREATE INDEX idx_entries_sortkey
        ON entries(COALESCE(published_at, fetched_at) DESC, id DESC, feed_id);
    -- 未读优先档（read ASC, sortkey DESC, id ASC）的复合排序索引：
    -- 列序/方向必须与 ORDER BY 逐列对应，否则 planner 会选等值索引再加 TEMP B-TREE 排序。
    CREATE INDEX idx_entries_unread_sortkey
        ON entries(read, COALESCE(published_at, fetched_at) DESC, id, feed_id);
    -- 搜索相关度候选取行前的排序索引
    CREATE INDEX idx_entries_search_order
        ON entries(id, read, COALESCE(published_at, fetched_at) DESC);
    CREATE INDEX idx_entries_feed_published ON entries(feed_id, published_at DESC);
    CREATE INDEX idx_entries_read_published ON entries(read, published_at DESC);
    -- 侧栏未读聚合索引（feeds JOIN entries 按 feed 聚合未读走索引扫描）
    CREATE INDEX idx_entries_feed_read ON entries(feed_id, read);
    -- 部分索引族：只索引被标记的行（常态体积忽略），计数/筛选全程不碰正文大列所在的表 B 树
    CREATE INDEX idx_entries_read_later ON entries(read_later) WHERE read_later = 1;
    CREATE INDEX idx_entries_starred ON entries(starred) WHERE starred = 1;
    CREATE INDEX idx_entries_unread_id ON entries(id) WHERE read = 0;

    -- 外部内容表：正文真身只在 entries 里存一份
    CREATE VIRTUAL TABLE entries_fts USING fts5(
        title,
        search_tokens,
        content='entries',
        content_rowid='id'
    );

    CREATE TRIGGER entries_fts_ai AFTER INSERT ON entries BEGIN
        INSERT INTO entries_fts(rowid, title, search_tokens)
        VALUES (new.id, new.title, new.search_tokens);
    END;

    CREATE TRIGGER entries_fts_ad AFTER DELETE ON entries BEGIN
        INSERT INTO entries_fts(entries_fts, rowid, title, search_tokens)
        VALUES ('delete', old.id, old.title, old.search_tokens);
    END;

    CREATE TRIGGER entries_fts_au AFTER UPDATE OF title, search_tokens ON entries BEGIN
        INSERT INTO entries_fts(entries_fts, rowid, title, search_tokens)
        VALUES ('delete', old.id, old.title, old.search_tokens);
        INSERT INTO entries_fts(rowid, title, search_tokens)
        VALUES (new.id, new.title, new.search_tokens);
    END;

    -- AI 结果缓存（同一文章 + 同一任务 + 同一参数 + 同一模型 + 同一 prompt 版本才复用）
    CREATE TABLE ai_cache (
        id             INTEGER PRIMARY KEY,
        entry_id       INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
        task           TEXT NOT NULL,
        params         TEXT NOT NULL,
        provider_model TEXT NOT NULL,
        prompt_version TEXT NOT NULL,
        output         TEXT NOT NULL,
        created_at     INTEGER NOT NULL,
        UNIQUE (entry_id, task, params, provider_model, prompt_version)
    );

    CREATE INDEX idx_ai_cache_entry ON ai_cache(entry_id);

    -- 设置（键值对）：AI provider、凭据引用、并发数、主题、网络代理等都挂在这里
    CREATE TABLE settings (
        key        TEXT PRIMARY KEY,
        value      TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );

    -- 文章级标签。`name` 是用户可见身份：UNIQUE COLLATE NOCASE 让「Rust」与「rust」
    -- 天然是同一个标签；`last_used_at` 供选择器「最近使用优先」；`sort_order`/`pinned`
    -- 是侧栏手动顺序。`entry_tags` 两端 CASCADE：删源/删条目/删标签都必须零孤儿。
    CREATE TABLE tags (
        id           INTEGER PRIMARY KEY,
        name         TEXT NOT NULL UNIQUE COLLATE NOCASE,
        color        TEXT,
        pinned       INTEGER NOT NULL DEFAULT 0,
        sort_order   INTEGER NOT NULL DEFAULT 0,
        last_used_at INTEGER,
        created_at   INTEGER NOT NULL
    );

    CREATE TABLE entry_tags (
        entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
        tag_id   INTEGER NOT NULL REFERENCES tags(id)    ON DELETE CASCADE,
        PRIMARY KEY (entry_id, tag_id)
    );

    CREATE INDEX idx_entry_tags_tag ON entry_tags(tag_id, entry_id);
"#;
