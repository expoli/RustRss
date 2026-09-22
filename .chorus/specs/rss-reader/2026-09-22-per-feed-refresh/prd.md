# PRD: 每源独立刷新间隔

- 模块: rss-reader / per-feed-refresh
- 日期: 2026-09-22
- 来源: Chorus Idea 49aeb608（P0-1 完成报告 Follow-up #3）

## 需求（已 elaboration 确认）

| # | 需求 | 决策 |
|---|------|------|
| R1 | 语义 | feeds.refresh_interval_minutes NULL=跟随全局档；覆盖值同一套白名单 15/30/60/120/360；「跟随全局」恢复 NULL |
| R2 | UI | 源右键菜单「刷新间隔」子项（跟随全局/15/30/60/120/360，当前项勾选），与「移入文件夹」同层，立即生效 |
| R3 | 调度 | 60s tick 内按源扫描：到期 = COALESCE(feed.interval, 全局) + last_fetched_at，到期源批量走现有 refresh_core(Some(ids))，单 flight 保护；到期为空跳过 |
| R4 | 持久化 | feeds 表加列（迁移 v8，INTEGER NULL）；全局设置不动作为默认档 |
| R5 | 设置提示文案（评审 B1 补回） | 设置页全局刷新间隔的 Hint 文案更新为双语明确例外：「全局关闭时，已单独设置间隔的源仍会按各自间隔刷新」；右键菜单内「跟随全局」项同样口径 |

## 验收标准

1. 右键某源设 15 分钟后：该源按 15 分钟节奏自动刷新（其余源仍按全局档）；设「跟随全局」后恢复全局节奏；重启后设置保留。
2. 全局档改动实时影响所有「跟随全局」的源，不影响已覆盖的源。
3. 到期计算纯函数带单元测试（NULL 覆盖/到期边界/全局 off 时覆盖源仍刷新）；迁移 v8 真文件升级测试；右键菜单 i18n 双语 key。
4. 手动刷新/OPML 抓取/单 flight 行为不回归；`cargo test --workspace` 全绿；README 同步。
5. 设置页 refreshIntervalHint 文案（zh-CN/en 双语）覆盖「全局关×覆盖仍刷新」例外；右键菜单「跟随全局」文案同口径。
