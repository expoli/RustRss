---
name: rustrss-dev-loop
description: RustRss 项目的日常开发回路与容易踩的项目特有陷阱：构建/测试/运行命令、ui 嵌入与重建规则、单实例锁与 MCP 端口、日志位置与终端镜像、迁移与性能红线自查、文档同步义务。当要在这个仓库里改代码/排障/自测时使用。
---

# RustRss 开发回路速查

## 命令

```bash
cargo test --workspace            # 全部测试（改完必跑；当前 360+ 全绿）
cargo clippy --workspace --all-targets   # 既有 3 条 core 基线告警，不应新增
cargo run -p rustrss-desktop      # 启动桌面应用（无 tauri-cli；不要用 cargo tauri dev）
cargo test -p rustrss-core        # 只跑核心库
```

## 项目特有的坑

| 坑 | 说明与对策 |
|---|---|
| **改 `ui/**` 不重建 = 跑旧前端** | Tauri 编译期嵌入 `ui/` 与 `tauri.conf.json`。任何前端/配置改动后必须 `cargo build -p rustrss-desktop` 再启动 |
| **单实例锁** | 用默认库时第二次启动会被已有实例接管（新进程自退）。多开诊断副本要用 `RUSTSS_DB=<path>`（不注册锁） |
| **游离实例会占 MCP 端口** | 历史上多个残留实例抢 `127.0.0.1:8817`。排查端口冲突先 `pgrep -af rustrss-desktop` / `rustrss-mcp`，只 kill 归属明确的（看 `/proc/<pid>/environ` 的 `HOME=`） |
| **日志在哪** | 每次启动一份 `logs/rustrss-YYYYMMDD-HHMMSS.log`（`[ui]` 诊断行、panic 都在里面）。终端调试用 `RUSTSS_LOG_STDOUT=1` 强制镜像到 stderr（TTY 下自动开） |
| **`RUSTSS_DB` 子进程缺 `HOME`** | `logs_dir()` 会退化成相对路径（`./.local/...`）→ 无头脚本里显式设置 `HOME` |
| **MCP 写能力默认关** | 需要 `mcp.write_enabled` + 生成 `mcp.write_token`；危险工具还要 `mcp.dangerous_enabled`。自测：`target/debug/rustrss-mcp --print-config` / `--http 127.0.0.1:<port>` |

## 改动前必读的红线（违反 = 重现已知回退）

- **SQLite**：聚合/COUNT 不得触碰正文大列所在表 B 树（用覆盖/部分索引）；新增/修改查询要写 **EXPLAIN 断言 + 变异校验**（去掉索引或条件时测试必须变红）；函数列 ORDER BY/WHERE 配表达式索引，必要时 `INDEXED BY` 钉住。
- **前端**：热路径（打开文章/滚动/计数刷新）禁止全量重建 DOM，用行级 patch；同值短路；UI 出现/消失不得引起布局跳动。
- **锁**：store 锁内不得 `await`；网络重活在锁外；同一动作并发用 CAS 单 flight + Drop guard。
- **分层**：业务逻辑进 `rustrss-core`；`src-tauri` 只做 `invoke → core` 薄封装；UI 无构建链（原生 JS）。
- 完整清单见仓库根 `AGENTS.md`（性能红线 13 条）。

## 迁移与测试约定

- 迁移写在 `crates/rustrss-core/src/store/schema.rs`（顺序追加，当前 v12），**必须**带升级路径测试（旧库 → 新版本数据不丢）。
- 行为改动先写复现测试；修 bug 先红后绿。
- i18n：新增文案同时补 `ui/i18n.js` 的 zh-CN 与 en（有 key-set 一致性测试）。

## 文档同步义务（交付前）

1. 行为变化 → `README.md`；
2. 需求/验收口径 → `.chorus/specs/rss-reader/spec.md` 的 checkbox（**完成一条勾一条**，禁止攒批补勾）；
3. 实机证据与遗留 → `.chorus/specs/rss-reader/manual-verification-checklist.md`（每批次新增一节：机械验证项 / 实机快照 / 环境限制）。

## 无头验证

UI 相关改动要拿实机证据时，走 `rustrss-headless-ui-verification` skill（Xvfb 隔离实例 + 截图 + 像素/回读断言）。
