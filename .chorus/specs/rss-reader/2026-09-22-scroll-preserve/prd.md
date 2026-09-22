# PRD: 后台刷新后保持列表深度滚动位置

- 模块: rss-reader / scroll-preserve
- 日期: 2026-09-22（rev2：评审 B1 修复——exhausted 多页视图同样保持；scrollTop=0 不补偿决策；readSessionIds 全路径）
- 来源: Chorus Idea 9ddf4552（P0-3 完成报告 Follow-up #1）

## 需求（已 elaboration 确认）

| # | 需求 | 决策 |
|---|------|------|
| R1 | 策略 | 多页已加载时 refresh:done 改为 prepend：拉首页 200（同筛选）→ 过滤出比已加载首行 sortkey 新的条目 → insertBefore 逐行插入首行之前；**滚动态（scrollTop>0）做 scrollHeight 补偿，在顶部（=0）不补偿**（新条目立即可见） |
| R2 | 范围 | `length > PAGE_SIZE && !paging.loading && !paging.error` 时走 prepend（**exhausted 不参与**：小库/星标等已耗尽多页视图同样保持）；首屏/单页维持现有 reset 语义 |
| R3 | 未读语义 | prepend 新未读；**所有 set_read 成功点**（openEntry/toggleRead/markAll）维护 readSessionIds，会话已读行不回插；计数照旧；选中行/焦点不动 |

## 验收标准

1. 深滚动（已加载 ≥2 页，含已耗尽视图）时后台刷新：新条目 prepend、滚动位置与选中行完全不动（headless 可测 scrollTop/选中 id 前后一致）。在顶部时新条目直接可见（不补偿）。
2. 首屏/单页/加载中场景行为与现状一致（reset 路径不变）。
3. 未读视图：会话内已读的行（openEntry 与 toggleRead 两路径）刷新后不回插；新未读出现在顶部。
4. 无新条目时 prepend 为空操作（无 DOM 写入、无滚动扰动）；`cargo test --workspace` 全绿；README 与 spec.md 同步。
