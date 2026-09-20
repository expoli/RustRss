# AGENTS.md — RustRss

跨平台（Windows / macOS / Linux）RSS 阅读器：Rust + Tauri 2，本地优先 SQLite，AI 双通道（应用内 BYOK + 对外只读 MCP server）。面向 agent 的完整背景与已定决策见 `README.md`；需求基线见 `.chorus/specs/rss-reader/spec.md`，阶段分析见 `.chorus/specs/rss-reader/2026-09-20-initial-requirements/prd.md`。

## 常用命令

```bash
cargo test --workspace          # 全部测试（改完必跑，当前全绿）
cargo build --workspace         # 编译
cargo run -p rustrss-desktop    # 启动桌面应用（无 tauri-cli，不用 cargo tauri dev）
cargo test -p rustrss-core      # 只跑核心库
```

仓库无 `.rustfmt.toml` / `.clippy.toml`，跟随默认格式；提交前建议 `cargo clippy --workspace`。

## 目录结构

```
crates/rustrss-core/    核心库：解析(feed-rs)/SQLite(FTS5)/抓取/OPML/AI adapter。不依赖 Tauri。
crates/rustrss-mcp/     MCP 服务器二进制（stdio + HTTP 双传输），rmcp 3 + axum。
src-tauri/              Tauri 2 壳（包名 rustrss-desktop）：commands/state/keyring，薄封装 core。
ui/                     原生 JS 前端（app.js/i18n.js/index.html/style.css）。无框架、无构建步骤、无 npm。
```

## 硬约束（违反 = 架构性回退）

1. **分层**：业务逻辑进 `rustrss-core`；`src-tauri` 只做 Tauri 胶水（command → core 调用），不写业务。MCP 工具与 Tauri command 共享 core 同一数据路径。
2. **不打包 WebKit**：Linux 用系统 WebKitGTK；依赖里禁止引入会捆绑浏览器内核的东西。
3. **不强制显示后端**：代码里不得设置/覆盖 `GDK_BACKEND`、`QT_QPA_PLATFORM` 等环境变量，X11 与 Wayland 都必须原生可用。
4. **UI 无构建链**：`ui/` 保持原生 JS + 静态文件，不引入打包器/框架；新增文案必须同时补 `ui/i18n.js` 的 zh-CN 与 en 两份 key（有 key-set 一致性测试）。
5. **MCP 安全口径**（均有测试守护）：只绑回环地址；无/错 token 一律 401；`/health` 不鉴权但不含订阅数据。改动 `crates/rustrss-mcp/src/http.rs` 必须保持这四条及其测试。
6. **MCP 响应口径**：列表类工具只回元数据 + ≤140 字摘要，正文必须 `get_article` 单取；列表默认 10 条、上限 50。防止撑爆客户端 agent 上下文。
7. **网络**：reqwest 一律 `default-features = false` + `rustls` + `webpki-roots`（不依赖系统信任库，便于跨发行版出包）。注意 0.13 的 feature 名是 `rustls`，不是 0.12 的 `rustls-tls`。

## 约定

- **数据目录**：按平台惯例（`%APPDATA%` / `~/Library/Application Support` / `~/.local/share`）；程序同级存在便携标记文件时改用 `data/`（逻辑在 `rustrss-core/src/paths.rs`，改动需保持迁移不丢数据并补测试）。
- **条目身份判定**：guid 缺失、link 缺失的退化场景必须稳定（`parse.rs` / `store/`，相关测试在 `tests/parse_formats.rs`）。
- **AI 密钥**：只存 OS keychain，绝不落明文配置/日志。
- **提交信息**：英文 conventional commits，小写祈使句，scope 用模块名，如 `feat(ai): ...`、`fix(paths): ...`（看 `git log` 对齐风格）。
- **文档同步**：改行为必须同步 `README.md`；动到需求/验收口径时回填 `.chorus/specs/rss-reader/spec.md` 的 checkbox（完成一条勾一条，禁止攒批补勾）。
- **测试**：core 的行为改动必须带测试（覆盖 parse/store/fetch/opml/ai/mcp-http）；修 bug 先写复现测试再修。
