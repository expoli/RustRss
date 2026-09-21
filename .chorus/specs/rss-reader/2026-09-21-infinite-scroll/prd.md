# PRD: 列表无限滚动（keyset 分页）

- 模块: rss-reader / infinite-scroll
- 日期: 2026-09-21
- 来源: Chorus Idea 8b6bd608（对标差距分析 P0-3）

## 问题

列表硬上限 200 条（`list_entries` limit 默认 200、UI 无加载更多），8k+ 条库老文章永远无法浏览。

## 需求（已 elaboration 确认）

| # | 需求 | 决策 |
|---|------|------|
| R1 | 分页机制 | 复合游标 keyset：游标 = `(COALESCE(published_at, fetched_at), id)`，续扫条件 `(sortkey, id) < (cursor)` 走 v6 表达式索引，深翻页零额外扫描代价 |
| R2 | 前端触发 | IntersectionObserver 哨兵元素接近底部自动追加下一页（批量 200）；逐行 append 不重建已有 DOM |
| R3 | 范围 | 未读/全部/星标/稍后读/单源视图全部支持；搜索仍一次性 200（本版不含） |

## 验收标准

1. 任一列表视图滚到底自动加载下一批（200/批），直至加载完全部匹配条目后停止（无重复、无跳条）。
2. 视图切换/筛选重置分页状态；未读视图单行删除、焦点保持与 2026-09-21 性能修复口径不回归（仍无全量重建）。
3. core `list_entries` 支持 cursor 参数（带筛选组合：feed/unread/starred/later × cursor），带测试覆盖续扫正确性（含同 sortkey 不同 id 的并列排序边界）。
4. 游标续扫走 idx_entries_sortkey（EXPLAIN QUERY PLAN 断言无 TEMP B-TREE）。
5. `cargo test --workspace` 全绿；README「列表虚拟化（当前硬上限 200 条）」待办项更新。
