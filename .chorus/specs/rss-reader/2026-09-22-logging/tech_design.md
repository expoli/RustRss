---
title: Tech Design: 日志功能（每次启动独立日志文件 + 关于页入口）
proposalUuid: b0a7595b-a530-49c1-82f7-a7c89fc90a9a
documentUuid:
---

# Technical Design: 日志功能

## 概览

日志实现放 **core**（`log` 门面 + 自研文件 writer，复用已有 `chrono`），src-tauri 只负责启动早期初始化与设置读取；UI 侧维持现有 `ui_log` 通道（改为写入 logger）。四个模块化任务串行执行（单写者）：core 基础设施 → 接入与迁移 → log.level 设置 → 关于页入口 + 收口验证。

## 架构与落点

| 层 | 文件 | 改动 |
|---|---|---|
| core | `crates/rustrss-core/src/paths.rs` | 新增 `logs_dir()`：`default_data_dir()/logs`（便携模式自然跟随 `data/`） |
| core | `crates/rustrss-core/src/logging.rs`（新） | `init` / 文件 writer（`log::Log` 实现）/ `prune` / panic hook 安装函数 |
| core | `crates/rustrss-core/src/discover.rs` | 2 处 `println!/eprintln!` → `log` 宏 |
| core | `crates/rustrss-core/Cargo.toml` | 新增 `log = "0.4"`（门面，无传递负担） |
| src-tauri | `src-tauri/src/main.rs` | 启动最早期：读 `log.level` → `logging::init()` → 安装 panic hook；10 处 println/eprintln 迁移 |
| src-tauri | `src-tauri/src/commands.rs` | `ui_log` 改为 `log::info!(target: "ui", ...)`；新增 `open_logs_dir` 命令；8 处 println/eprintln 迁移 |
| src-tauri | `src-tauri/src/scheduler.rs` / `notify.rs` | 5 + 2 处迁移 |
| src-tauri | `src-tauri/Cargo.toml` | 新增 `log = "0.4"`（src-tauri 直接用宏） |
| ui | `ui/index.html` / `ui/app.js` | 关于页按钮（调用 `open_logs_dir`）；设置页 log.level 控件 |
| ui | `ui/i18n.js` | 新增 key（zh-CN + en 双份） |

## Module Contracts

| 契约 | 定义 |
|---|---|
| `paths::logs_dir() -> PathBuf` | 数据目录下 `logs/`（= `default_data_dir()/logs`）；不负责创建。**便携模式未实现（durable spec 未完成项），本批不做该路径断言**；未来便携落地后本函数自动跟随 |
| `logging::init(log_dir: &Path, level: log::LevelFilter) -> Result<PathBuf, String>` | 创建目录（不存在时）、创建本次日志文件、安装全局 logger；返回本次文件路径。**失败返回 Err，调用方降级**（不 panic、不阻断启动） |
| 日志文件命名 | `rustrss-YYYYMMDD-HHMMSS.log`（本地时间；同秒启动冲突时追加 `-N`） |
| 日志行格式 | `{本地时间 RFC3339 毫秒} {LEVEL:<5} {target}: {message}`（如 `2026-09-22T23:45:01.123 INFO  ui: view=all ...`） |
| `logging::prune(log_dir: &Path, keep_files: usize, max_total_bytes: u64) -> PruneReport` | 幂等清理：按 mtime 从旧到新删除，直到同时满足文件数 ≤ keep_files 且总量 ≤ max_total_bytes；返回删除数与剩余量；**每次启动调用一次**（init 成功后） |
| 保留参数 | `keep_files = 20`，`max_total_bytes = 50 * 1024 * 1024` |
| panic hook | `logging::install_panic_hook()`：先 `log::error!`（含 payload + location），再调用原 hook（保留 stderr 默认行为）；安装失败忽略 |
| 级别语义 | `log.level` 白名单 `["info", "debug"]`（默认 `info`）；持久化值非法时回落 `info`；变更时 `log::set_max_level()` 即时生效 |
| debug 内容 | 迁移时给 HTTP 请求/刷新等关键路径补 `log::debug!`（如请求 URL、状态码、耗时）；info 级保持现有输出语义 |
| `open_logs_dir` 命令 | 确保 `logs_dir()` 存在 → 平台启动器：Linux `xdg-open` / macOS `open` / Windows `explorer`，**目录路径作为独立进程参数**（不经 shell）；失败返回可读错误 |
| 敏感信息 | 迁移点逐处审查：key/token 不得进入任何日志行；复用现有 scrub（`scrub`/错误打码）口径；补测试断言 scrub 后不含 key 形态 |

## 分任务设计（4 个任务，串行）

### T1 日志基础设施（core）

`paths::logs_dir()` + `logging.rs`（文件 writer、`init` 不含全局安装的可测部分、`prune`）+ 单测：
- 命名与本地时间戳格式；同秒冲突后缀；
- `prune` 边界：恰好 20 个不删、21 个删 1、总量超 50 MB 按最旧删、混合场景、单文件超上限（应删到只剩它或空）；
- writer：写入后文件内容含级别/target/消息，多行追加正确；
- `logs_dir()` 路径断言（= `default_data_dir()/logs`，跟随现有 `paths` 测试风格；**不做便携模式多路径断言**——“便携模式”当前未实现，已列入本批范围外）。

### T2 接入与迁移（src-tauri + core）

- `logging::init` 的全局安装包装（`log::set_logger` + `set_max_level`）+ `install_panic_hook`；
- src-tauri 启动最早期调用：**以默认 `info` 初始化**（此时设置还读不到，因为 `log.level` 存在 SQLite）→ 打开 DB 后立即读取 `log.level` 并 `set_max_level` 应用；init 失败走降级（一次 `eprintln!` 提示 + 继续）；
- 27 处 `println!/eprintln!` 迁移为 `log` 宏（info/warn/error 分级；保持信息量不降级），**并产出「原语句 → 新宏与级别」迁移对照表**作为“信息量不降级”的可核验证据（随 report 提交）；
- `ui_log` → `log::info!(target: "ui", ...)`（保留 `[ui]` 语义便于检索，如 message 前缀 `[ui]`）；
- 关键路径补 `log::debug!`（HTTP 请求/刷新/调度 tick 等）；
- 敏感信息逐点审查 + 测试（scrub 断言）。

### T3 log.level 设置项

- store 键 `log.level`（`info`/`debug`，默认 `info`，非法回落）；
- 设置页控件（下拉）+ i18n 双语；
- 变更即时生效：设置保存路径里调用 `log::set_max_level()`；重启后从设置读取；
- 测试：钳位/非法值回落/默认值。

### T4 关于页入口 + 收口

- `open_logs_dir` 命令（安全启动器，目录路径独立参数）+ 关于页按钮 + i18n；
- README 补日志说明（日志位置、保留策略、如何反馈）；
- **实机验证（Xvfb）**：启动生成日志文件（含 [ui] 行）→ 造 25 个文件/超量 → 重启后清理正确 → 关于页点按钮（Xvfb 无文件管理器 → 可读错误提示不崩）→ debug 级别切换后出现 debug 行；
- **批次收口**：`cargo test --workspace` 全绿 + `cargo clippy --workspace` 无新增警告；更新 durable spec 勾选。

## 实施顺序与依赖

T1 → T2 → T3 → T4（串行；单写者约束）。显式依赖：T2 依赖 T1；T3 依赖 T2（级别由 logger 生效）；T4 依赖 T3（收口）。

**排期**：本功能实现排在「审计修复批次一」（T3/T4/T5 未完）之后——同仓库同时只能有一个写者。

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 日志目录不可写 / 磁盘满 | `init` 失败即降级（无日志继续运行），不 panic、不阻断启动 |
| 迁移 println! 丢失原有信息语义 | 逐点分级映射并保留原文案；产出「原语句 → 新宏与级别」迁移对照表（T2 报告交付物），供 reviewer 逐行比对 |
| debug 级别日志量激增 | 保留策略（20 个/50 MB）兜底；debug 只在关键路径，不做全量 trace |
| Xvfb 无文件管理器 | 降级为可读错误提示（不算失败，验证时如实记录） |
| 敏感信息泄漏进日志 | 迁移点逐处审查 + scrub 测试 + code-review 复核 |
| 日志写入阻塞 UI/刷新线程 | 单文件追加写、行级 flush；批量刷新场景避免每行 fsync（按文件句柄写入即可） |
