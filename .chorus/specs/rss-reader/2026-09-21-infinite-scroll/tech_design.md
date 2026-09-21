# Tech Design: 列表无限滚动（keyset 分页）

- 模块: rss-reader / infinite-scroll
- 日期: 2026-09-21（rev2：评审 B1 修复——sortkey 直出数据契约；行值比较 fallback；NULL 语义钉死。rev2b：本地镜像重同步 + search 继承说明修正）

## core 层（crates/rustrss-core/src/store/mod.rs）

- **数据契约（评审 B1 修复）**：`ENTRY_SELECT_LIST` 与 `ENTRY_SELECT` 增加 `COALESCE(e.published_at, e.fetched_at) AS sortkey` 列，`EntryRow` 增加 `sortkey: i64` 字段直出——前端不自己算游标，直接取末行 `(sortkey, id)`；`get_entry` 同步带出（列序两处共享，map_entry_row 一处改）。
- `EntryQuery` 增加可选 `cursor: Option<(i64, i64)>`（sortkey 恒有值：`fetched_at NOT NULL`，生产数据不存在双 NULL）。
- 续扫条件首选行值比较：
  ```sql
  AND (COALESCE(e.published_at, e.fetched_at), e.id) < (?, ?)
  ```
  **fallback（设计内置）**：实现时若 EXPLAIN 显示行值比较未命中 idx_entries_sortkey，改用展开式等价比较：
  ```sql
  AND (COALESCE(e.published_at, e.fetched_at) < ?k
       OR (COALESCE(e.published_at, e.fetched_at) = ?k AND e.id < ?id))
  ```
  两者语义等价；以 EXPLAIN QUERY PLAN 实测命中为准，任务 AC 的断言保证最终形态无 TEMP B-TREE。
- `list_entries` 生成 SQL 时若 cursor 存在则追加该条件；返回行仍按 `ORDER BY COALESCE(...) DESC, id DESC LIMIT n`。
- `search()` 复用 `ENTRY_SELECT_LIST`，会**被动继承** sortkey 列（map_entry_row 共享所需）——行为不变（多带一个字段），非「路径解耦」；搜索查询本身不加 cursor。

## src-tauri（commands.rs）

- `list_entries` 命令增加可选 `cursor_sortkey: Option<i64>, cursor_id: Option<i64>` 透传 core（两者必须同时出现）。

## ui/app.js

- `state.entries` 追加模型：`loadEntries(reset)`；reset=true 重建（现有路径），false 时取末行 `(sortkey, id)`（sortkey 由后端直出，前端不自算）调 `list_entries` 追加，行构建复用 renderList 内的单行构造（提取 `buildEntryRow(e)` 共用，不重建已有 DOM）。
- 列表容器尾部放哨兵 `<li class="load-sentinel">`，IntersectionObserver(root=list 容器, rootMargin '600px') 触发 `loadEntries(false)`；加载中防重入；返回条数 < 批量或空即断开 observer。
- 视图切换（setView/loadEntries reset 路径）重装 observer；unread 视图单行删除后游标取剩余末行，逻辑不变。

## 测试

- core：tests/store.rs 增 keyset 用例——插入含相同 published_at 的并列行与 **published_at 为 NULL（fetched_at 补位）** 的行（fetched_at NOT NULL，双 NULL 不可经 upsert 构造，不在测试范围），验证第一页末游标续扫无缝（总数一致、无重复、无跳条）；**EXPLAIN QUERY PLAN 断言覆盖筛选形态（`WHERE read=0` + cursor 与 feed_id + cursor 两种）**，均走 idx_entries_sortkey 无 TEMP B-TREE（若行值比较不命中则按 fallback 展开式改写后重验）。
- UI：手动验证清单（8k 库滚到底、各视图切换、j/k 导航、未读删除单行）。
