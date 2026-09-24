# 开发指南

## 项目结构

| 路径 | 内容 |
| --- | --- |
| `crates/rustrss-core/` | 订阅解析、SQLite、抓取、全文检索、OPML 和 AI adapter |
| `crates/rustrss-mcp/` | 独立 MCP server，stdio 与 HTTP 双传输 |
| `src-tauri/` | Tauri command、状态、系统凭据库和桌面集成 |
| `ui/` | 原生 JavaScript 与静态资源，无前端打包步骤 |

业务逻辑应放在 `rustrss-core`；桌面端与 MCP 共享 core 和同一数据路径。

## 构建与验证

Linux 开发依赖见[快速开始](getting-started.md#从源码运行)。常用命令：

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p rustrss-desktop
```

修改 `ui/` 后要重新构建桌面应用，因为 Tauri 会把静态资源嵌入程序。UI 行为需要无头复现时，项目的流程和限制见 [`rustrss-headless-ui-verification`](../.agents/skills/rustrss-headless-ui-verification/SKILL.md)。

## 数据库与诊断

开发库和外来 SQLite 文件不会自动迁移为当前格式。应用标识或 schema 版本不匹配时会拒绝打开；不要用真实用户库做开发截图或试验。可通过 `RUSTSS_DB` 指定隔离数据库。

日志位于平台数据目录下的 `rustrss/logs/`。在桌面端“设置 → 关于”可打开日志目录和调整级别。问题反馈时，附上最新日志，并移除不希望公开的订阅地址等信息。

## 发布

版本号需要同时更新根 `Cargo.toml` 和 `src-tauri/tauri.conf.json`。推送 `v*` 标签会触发打包：Linux `.deb`、Windows NSIS、macOS arm64 `.dmg`。GitHub Actions 工作流定义和平台依赖见 [`.github/workflows/release.yml`](../.github/workflows/release.yml)；macOS 包目前未签名。

## 项目资料

- [项目约定与性能红线](../AGENTS.md)
- [需求基线](../.chorus/specs/rss-reader/spec.md)
- [手工验收清单](../.chorus/specs/rss-reader/manual-verification-checklist.md)
