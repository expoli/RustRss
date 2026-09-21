# PRD: 一键备份/恢复数据库

- 模块: rss-reader / backup-restore
- 日期: 2026-09-21（rev3：R3 单元同步边车删除先行顺序——backup API 导出、启动时替换、崩溃容忍算法）
- 来源: Chorus Idea 9107ade6（对标差距分析 P0-6）

## 问题

库在平台数据目录（如 `~/.local/share/rustrss/`），用户要自己找目录拷文件；换机/重装无应用内备份入口。与 OPML 导入导出互补（OPML 只管订阅列表，备份含阅读状态/AI 缓存/设置）。

## 需求（已 elaboration 确认 + 评审修订）

| # | 需求 | 决策 |
|---|------|------|
| R1 | 备份 | 设置页按钮 → 目录选择器（取消则不动库）→ **rusqlite backup API 在线快照**（对活跃读写一致，无 checkpoint/WAL 尾巴依赖）→ 产物 `RustRss-backup-YYYYMMDD-HHMMSS.sqlite`（独立干净库文件） |
| R2 | 恢复 | 选文件 → 校验（只读打开、`user_version` ≤ 当前；超版本拒绝）→ 确认框（覆盖警告）→ 暂存 `pending-restore.sqlite` → **提示重启；重启时在任何连接（Store / MCP）打开之前执行替换**（唯一保证路径；不做退出时替换——MCP 第二连接与 Windows rename 限制使其不可行） |
| R3 | 替换算法（崩溃容忍） | pending 存在时：db 缺失则不报错继续（替换意图优先）；rename 现库为 `bak-<ts>`（保底回滚）→ **删除 stale `-wal`/`-shm` 边车（严格先于下一步）** → rename pending → db（不变量：边车删除先行，使「替换完成+边车残留」崩溃态不可达）；db 缺失且 bak 存在且 pending 不存在 → rename bak → db 回滚；旧 bak 只保留最近 1 份 |
| R4 | 分层 | core 提供纯函数（在线备份导出 / 校验 / 暂存 / 启动替换），src-tauri 只做 dialog + 薄命令；启动替换在 main.rs 最前（AppState/MCP 打开之前） |

## 验收标准

1. 备份产物是独立干净的库文件，可在另一台机器直接使用；**在有并发读（第二连接）的情况下导出仍一致**。
2. 恢复：校验失败（非库文件/版本超前）给出明确错误且不损坏现有库；确认后暂存成功；重启后加载备份内容；**残留的非空 `-wal`/`-shm` 不污染恢复结果**。
3. 崩溃容忍（core 测试）：替换两步 rename 之间崩溃（db 缺失 + pending 在）→ 重启后正确完成替换；崩溃后 pending 已不在但 db 缺失 + bak 在 → 回滚到 bak。
4. core 纯函数带测试：备份→恢复往返数据一致（feeds/entries/**read/starred**/settings）；替换幂等；无 pending 不动现库。
5. `cargo test --workspace` 全绿；设置页新增两个按钮带 zh-CN/en 双语 key；README 同步。
