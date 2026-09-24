//! 压平前的旧链（13 条迁移）测试夹具 —— 只读存档，仅供测试。
//!
//! 用途（两条不变量都靠它证明）：
//! 1. 结构化等价：旧链跑到底的终态 schema == 基线的 schema；
//! 2. 老库拒绝：旧链冻结库（没有 application_id 魔数）打开时必须被拒。
//!
//! 由 git show <压平前的 HEAD>:crates/rustrss-core/src/store/schema.rs 机械提取，
//! 不要手工编辑（改它等于改历史）。
pub const LEGACY_CHAIN: &[&str] = &[
    // v1：订阅源 / 文件夹 / 条目 / 全文索引
    r#"
    CREATE TABLE folders (
        id       INTEGER PRIMARY KEY,
        name     TEXT NOT NULL UNIQUE,
        position INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE feeds (
        id              INTEGER PRIMARY KEY,
        url             TEXT NOT NULL UNIQUE,   -- 归一化后的订阅身份（RSSHub 存 rsshub://path，抓取时解析）
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
    // v8：每源刷新间隔覆盖（分钟，NULL=跟随全局档）。白名单校验在命令层
    // （与全局档共用一张表），迁移只加列：存量源 NULL 即为「跟随全局」，
    // 行为与升级前一致。
    r#"
    ALTER TABLE feeds ADD COLUMN refresh_interval_minutes INTEGER;
    "#,
    // v9：星标部分索引。旧版 counts() 是单扫描 4 聚合，starred 不在任何覆盖索引且列在
    // 正文大列之后 → 全表扫穿溢出页链（冷启动真实库实测 ~100ms/次，侧栏每次刷新都付）。
    // 改成 4 个子查询后 starred 需要自己的索引；部分索引只含已星标行（常态 0 行，体积忽略）。
    r#"
    CREATE INDEX idx_entries_starred ON entries(starred) WHERE starred = 1;
    "#,
    // v10：用户自定义订阅源标题。feeds.title 是**源站名**，每次抓取 success 都会被
    // update_feed_meta 覆盖（源站改名跟着变）；用户改名必须另存一列，否则下次刷新即丢。
    // 显示层统一读 COALESCE(custom_title, title)，NULL = 跟随源站名。
    r#"
    ALTER TABLE feeds ADD COLUMN custom_title TEXT;
    "#,
    // v11：未读优先（list.sort=unread_first）的复合排序索引。
    // 该档 ORDER BY 是 `read ASC, COALESCE(published_at, fetched_at) DESC, id ASC`，
    // 列序/方向与下面索引逐列对应——只有这样的复合索引才能同时满足「顺序」与
    // 「read 等值筛选」，否则 planner 会选 idx_entries_feed_read 之类等值索引
    // 再加 TEMP B-TREE 排序（本程序从不 ANALYZE，planner 没有统计可依）。
    // 后缀 id 用升序（与 newest 档的 id DESC 不同）：索引最后一列正是 ORDER BY 的
    // 末列，SQLite 才能只靠索引走完排序，同 sortkey 的并列行也能被 keyset 游标稳定续扫。
    r#"
    CREATE INDEX idx_entries_unread_sortkey
        ON entries(read, COALESCE(published_at, fetched_at) DESC, id);
    "#,
    // v12：文章级标签（`tags` + `entry_tags`）
    // - `tags.name` 是用户可见身份：`UNIQUE COLLATE NOCASE` 让「Rust」与「rust」天然
    //   是同一个标签（重名报 duplicate_tag_name），应用层不需要再做归一化列；
    // - `last_used_at`：打标时更新（见 `Store::assign_tags`），选择器「最近使用优先」
    //   的唯一数据来源；`sort_order` 是侧栏手动顺序（小的在前），`pinned` 置顶优先；
    // - `entry_tags` 两端 `ON DELETE CASCADE`：删源 / 删条目 / 删标签都必须零孤儿。
    //   `PRAGMA foreign_keys=ON` 在 `Store::init` 里对每个连接开启（连接级设置），
    //   写入路径（`remove_feed` / `delete_tag`）另有显式清理兜底——不变量不依赖
    //   调用方是否记得开这个 pragma；
    // - `idx_entry_tags_tag(tag_id, entry_id)`：按标签取条目 / 标签计数走它；反向
    //   （按条目取标签）由主键 `(entry_id, tag_id)` 的隐式索引覆盖；
    // - `idx_entries_unread_id(id) WHERE read = 0`：标签**未读计数**的覆盖索引
    //   （部分索引只索引未读行，与 v4/v9 同族）。`read` 列在 entries 里排在 11.5KB
    //   正文大列之后，按 rowid 回表取它就要穿溢出页链（counts() 教训：冷启动
    //   83-119ms/次）；这个部分索引把「这行读过没有」变成索引里就有的判断，于是按 tag
    //   计数全程只扫索引、不碰正文大列所在的表 B 树。之所以不用全量 `(id, read)`：
    //   实测全量版会夺走 `counts()` 里 `COUNT(*) FROM entries` 子查询的索引选择
    //   （两版都不碰表 B 树，但「不改既有形态」的爆炸半径更小），而部分索引只含未读行、
    //   体积也更小。
    r#"
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
    CREATE INDEX idx_entries_unread_id ON entries(id) WHERE read = 0;
    "#,
    // v13 is the first unreleased development schema. Keep all pre-release
    // additions together; once shipped, future migrations become append-only.
    r#"
    ALTER TABLE feeds ADD COLUMN retry_after_at INTEGER;
    ALTER TABLE feeds ADD COLUMN position INTEGER;
    ALTER TABLE entries ADD COLUMN thumbnail_url TEXT;
    DROP INDEX idx_entries_sortkey;
    DROP INDEX idx_entries_unread_sortkey;
    CREATE INDEX idx_entries_sortkey
        ON entries(COALESCE(published_at, fetched_at) DESC, id DESC, feed_id);
    CREATE INDEX idx_entries_unread_sortkey
        ON entries(read, COALESCE(published_at, fetched_at) DESC, id, feed_id);
    CREATE INDEX idx_entries_search_order
        ON entries(id, read, COALESCE(published_at, fetched_at) DESC);
    DROP TRIGGER IF EXISTS entries_fts_au;
    CREATE TRIGGER entries_fts_au AFTER UPDATE OF title, search_tokens ON entries BEGIN
        INSERT INTO entries_fts(entries_fts, rowid, title, search_tokens)
        VALUES ('delete', old.id, old.title, old.search_tokens);
        INSERT INTO entries_fts(rowid, title, search_tokens)
        VALUES (new.id, new.title, new.search_tokens);
    END;
    "#,
];
