//! 存储层 schema 与迁移。
//!
//! 迁移按 `PRAGMA user_version` 递增；每个迁移在独立事务里执行。
//! 迁移一旦发布就不再修改（改动只能追加新迁移）。

/// 迁移列表：索引 i 对应 user_version i → i+1。
pub const MIGRATIONS: &[&str] = &[
    // v1：订阅源 / 文件夹 / 条目 / 全文索引
    r#"
    CREATE TABLE folders (
        id       INTEGER PRIMARY KEY,
        name     TEXT NOT NULL UNIQUE,
        position INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE feeds (
        id              INTEGER PRIMARY KEY,
        url             TEXT NOT NULL UNIQUE,   -- 规范化后的抓取地址
        title           TEXT NOT NULL,
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
        created_at      INTEGER NOT NULL
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
        UNIQUE (feed_id, stable_id)
    );

    CREATE INDEX idx_entries_feed_published ON entries(feed_id, published_at DESC);
    CREATE INDEX idx_entries_read_published ON entries(read, published_at DESC);

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

    CREATE TRIGGER entries_fts_au AFTER UPDATE ON entries BEGIN
        INSERT INTO entries_fts(entries_fts, rowid, title, search_tokens)
        VALUES ('delete', old.id, old.title, old.search_tokens);
        INSERT INTO entries_fts(rowid, title, search_tokens)
        VALUES (new.id, new.title, new.search_tokens);
    END;
    "#,
    // v2：AI 结果缓存（同一文章 + 同一任务 + 同一参数 + 同一模型 + 同一 prompt 版本才复用）
    r#"
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
    "#,
    // v3：设置（键值对）。后续 AI provider、凭据引用、并发数等都挂在这里。
    r#"
    CREATE TABLE settings (
        key        TEXT PRIMARY KEY,
        value      TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );
    "#,
    // v4：稍后读标记 + 部分索引（只索引已标记行，体积与查询都最优）
    r#"
    ALTER TABLE entries ADD COLUMN read_later INTEGER NOT NULL DEFAULT 0;
    CREATE INDEX idx_entries_read_later ON entries(read_later) WHERE read_later = 1;
    "#,
    // v5：侧栏未读聚合索引（feeds JOIN entries 按 feed 聚合未读时走索引扫描）
    r#"
    CREATE INDEX idx_entries_feed_read ON entries(feed_id, read);
    "#,
    // v6：列表排序键表达式索引。list_entries 的 ORDER BY COALESCE(published_at,
    // fetched_at) DESC, id DESC 是函数列，此前用不了任何索引 → 每次视图切换
    // 全表扫 + 排序（8k 条库实测 15-19ms）；走本索引后按序取前 N（~1-2ms）。
    r#"
    CREATE INDEX idx_entries_sortkey ON entries(COALESCE(published_at, fetched_at) DESC, id DESC);
    "#,
    // v7：全文抓取标记位（摘要型条目一旦抓到原文正文，就以此为准）。
    // 这一位同时支撑两条已定行为：刷新 upsert 不再覆盖已抓正文（只更新元数据）、
    // 重开同一篇文章零网络（`needs_fulltext` 直接为假）。
    r#"
    ALTER TABLE entries ADD COLUMN fulltext_fetched INTEGER NOT NULL DEFAULT 0;
    "#,
];
