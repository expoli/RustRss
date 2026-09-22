# Tech Design: 每源独立刷新间隔

- 模块: rss-reader / per-feed-refresh
- 日期: 2026-09-22

## 数据

- 迁移 v8：`ALTER TABLE feeds ADD COLUMN refresh_interval_minutes INTEGER`（NULL=跟随全局）。
- `store/mod.rs`：`set_feed_refresh_interval(feed_id, Option<i64>)`（白名单校验在命令层，与全局档共用 REFRESH_INTERVAL_CHOICES）；`feeds_with_interval() -> Vec<(i64, Option<i64>, Option<i64> /*last_fetched_at*/)>`（调度扫描用，短锁）；FeedRow 透出 refresh_interval_minutes（侧栏 tooltip 可显）。

## 调度（src-tauri/scheduler.rs 改造）

- 现状：60s tick 读全局 interval → 全量 refresh_core(None)。**改为按源扫描**：
  - tick 内读全局档 + `feeds_with_interval()`（一次短锁）
  - 纯函数 `due_feed_ids(global_minutes, rows, now) -> Vec<i64>`：
    - 全局 off 且源无覆盖 → 不排；全局 off 且源有覆盖 → 按覆盖排（全局关不该关掉显式覆盖的源）
    - 到期判定 `now - COALESCE(feed.interval, global)*60 >= last_fetched_at`（last_fetched_at NULL 视为立即到期）
  - due 非空 → 单 flight + refresh_core(Some(ids))（refresh:start/done 事件与通知角标逻辑全复用）
- 纯函数单测：NULL 跟随、覆盖优先、边界（差 1s 未到期/恰好到期）、全局 off × 覆盖、空 due。

## 命令与 UI

- `set_feed_refresh_interval(feed_id, value: Option<String>)`：值白名单归一（"15"/…/"360"，null/"global" → None），写 feeds 列；返回更新后 FeedRow。
- `openFeedMenu`（app.js）：加「刷新间隔」组（跟随全局 + 5 档，当前项前打勾，与移入文件夹同层分隔）；选择即 invoke；成功后 refreshCounts 刷新侧栏（tooltip/勾选态来自 FeedRow.refresh_interval_minutes）。
  - **菜单原语扩展（评审 NOTE）**：现有 openContextMenu 只支持 {label, danger, action}，需小扩展支持 {separator: true} 与 {checked: true}（通用能力，一次扩展右键菜单与文件夹菜单共用）。
  - **语义迁移说明（评审 NOTE）**：到期基准用库内 last_fetched_at——手动刷新/OPML 抓取也会重置该源的自动刷新计时（视为特性：刚刷过的源不重复自动刷）。
- i18n：menu.refreshInterval / menu.refreshFollowGlobal / menu.refreshMin15…（复用既有 settings.refreshMin* 文案 key 的值或新 key，实现时取一致口径）；**settings.refreshIntervalHint 双语更新**（覆盖例外口径，评审 B1）。

## 测试

- core：迁移 v7→v8 真文件测试；set_feed_refresh_interval 往返 + 白名单拒绝。
- src-tauri：due_feed_ids 纯函数全分支；命令归一化。
- 手动清单：真机右键设置后 15 分钟源到点刷新（headless 可用短档 + 本地 fixture 服务复现 P0-1 的验证法）。
