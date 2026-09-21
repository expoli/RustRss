# PRD: 单实例锁

- 模块: rss-reader / single-instance
- 日期: 2026-09-21
- 来源: Chorus Idea 57a30595（对标差距分析 P0-5）

## 问题

可同时开两个实例：第二个实例 MCP 端口冲突仅打日志继续跑（`Address already in use`），且两进程双写同一 SQLite（WAL 允许，但数据一致性与 token 状态有风险）。诊断时需 `RUSTSS_DB` 指向副本才安全。

## 需求（已 elaboration 确认）

| # | 需求 | 决策 |
|---|------|------|
| R1 | 锁机制 | tauri-plugin-single-instance（官方、跨平台：Windows 命名 mutex / macOS/Linux 锁文件） |
| R2 | 多开共存 | 使用**默认库**时强制单实例；`RUSTSS_DB`/参数指向非默认库时**跳过插件注册**（诊断/多副本是开发者合法用法） |
| R3 | 二次启动行为 | 唤出已有主窗口（show + set_focus，含托盘隐藏态）后退出自己 |

## 验收标准

1. 默认库下二次启动：新进程立即退出，已有主窗口被唤出并聚焦（含此前隐藏到托盘的状态）。
2. `RUSTSS_DB=/tmp/x.sqlite` 启动的实例不受默认实例锁影响，可共存（诊断路径回归）。
3. 锁判定「是否默认库路径」的纯函数带单元测试（paths 层已有 pick_db_path 测试模式）。
4. `cargo test --workspace` 全绿；README 行为说明同步。
