# Tech Design: 全文获取

- 模块: rss-reader / fulltext
- 日期: 2026-09-21

## 依赖选型（实现时第一步）

候选：`readability` crate（dom-based，Mozilla 算法移植）或等价 `readability-rs`；评估标准：最近 12 个月有发布、无安全公告、传递依赖可控（html5ever 系可接受，项目已有 DOM 处理经验）。选定后在任务报告中记录选型理由。

## core 层

- `crates/rustrss-core/src/fulltext.rs` 新模块：
  - `extract(html: &str, url: &str) -> Result<Extracted>` → { content_html, content_text }（content_text 复用 html.rs 的 to_text）
  - `MAX_BYTES = 2MB` 上限在调用侧截断拒绝
- 迁移 v7：`ALTER TABLE entries ADD COLUMN fulltext_fetched INTEGER NOT NULL DEFAULT 0`
- `store/mod.rs` 新方法：
  - `set_fulltext(entry_id, html, text)` → 写 content_html/content_text/search_tokens（重算 bigram）+ fulltext_fetched=1
  - `upsert_entries` 的 UPDATE 分支改为：`fulltext_fetched = 1` 时保留 content_html/content_text/search_tokens（用 COALESCE/条件 SQL），其余字段照常更新
- `EntryRow` 无需暴露 fulltext_fetched 给 UI（内部标记）——`is_summary_entry` 判定在 Rust 侧做：`get_entry` 返回时附带（或在命令层算），给前端一个 `needs_fulltext: bool`

## src-tauri 层

- `fetch_fulltext(entry_id)` 命令：
  1. 查 entry url + needs_fulltext 判定；已抓则直接返回（幂等）
  2. Fetcher 抓 url（锁外，网络永不持锁）；超限/非 HTML 报错
  3. `extract()` → `set_fulltext`（短暂持锁）
  4. 返回更新后的 EntryRow

## ui/app.js

- renderReader：`entry.needs_fulltext` 时在 actions 行加「获取全文」按钮（loading 态防重入）；成功后用返回的 EntryRow 重渲染阅读区（正文已更新）；失败 setStatus 错误。
- i18n key：reader.fetchFulltext / reader.fetching / status.fulltextFailed 等。

## 测试

- core tests/fulltext.rs：fixture 样本（typical-article.html / non-html.txt / empty.html）→ extract 行为；set_fulltext 写回与 upsert 不覆盖（v7 标记生效）断言；迁移 v6→v7 真文件升级测试。
- src-tauri：无（薄封装）；手动清单：真源摘要文章抓取、断网失败路径、二次打开零网络。
