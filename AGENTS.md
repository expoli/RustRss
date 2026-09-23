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

## 性能红线（全部来自实测教训，违反 = 重现已知回退；括号内为先例提交）

### SQLite 查询

1. **聚合/COUNT 不得触碰正文大列所在的表 B 树**：聚合列（read/starred/read_later 类标志位）必须被覆盖索引或部分索引覆盖。新加计数/聚合前先写 EXPLAIN 断言（经 `explain_*` 帮手函数与线上 SQL 同源，任何子查询退化为裸 `SCAN entries` 即测试红）。教训：旧 `counts()` 单扫描 4 聚合因 starred/read_later 排在 11.5KB 正文列之后，每次全表穿溢出页链读 ~97MB——热缓存副本上 8-20ms 骗过了评审，冷启动真实库 83-119ms/次且侧栏每次刷新都付；改 4 子查询各走索引后 0.7ms（89f69dc）。
2. **函数列 ORDER BY/WHERE 必须配表达式索引**；planner 无统计时不可靠，允许 `INDEXED BY` 钉索引，且断言必须变异校验（去掉索引/约束时测试要变红）。教训：v6 sortkey 索引前 list_entries 每次全表扫+排序（32ff553）。
3. **只在热缓存副本上测过 ≠ 快**。性能验收必须报告冷启动代价（或明确标注测的是热缓存）；“每次交互都要跑”的查询按冷缓存口径评估。
4. **返回行的 PRAGMA 用 `query_row` 消费，不用 `execute`**（`wal_checkpoint` 等）；修过一个反向修复的回归，修复必须带测试（61c95c6→81398eb）。
5. **数据库文件操作的不变量用崩溃态测试钉住**（备份/恢复的边车先行删除不变量与 A/B/C 崩溃态专测，7cc3a0d）。

### 渲染热路径

6. **热路径（打开文章/滚动/计数刷新）禁止全量重建 DOM**：行级 patch/append/prepend，行构造抽 `buildXxxRow` 复用；侧栏/列表统一 keyed reconcile。教训：曾因每次打开文章全量重建 200 行，CPU 风暴把无率的 get_entry（0.07ms）拖到 60-150ms（f607d85）。
7. **同值短路**：重复性更新（角标/计数/tooltip）先比对旧值，未变则零写入（545400c）。
8. **UI 出现/消失不得引起布局跳动**：常驻固定高度状态区（933cfab）；长文案不得挤压操作按钮。

### 测量与验证

9. **判定“后端慢”先分层计时**：`[rustrss][slow]` 与 `__RENDER_TIMINGS`/`renderList` 打点是主要证据。主键单行查询变慢 = 锁排队或 CPU 饥饿——先查同刻持锁者/渲染风暴，别先怪 SQL。
10. **本地复测要跑用户将运行的产物**：Tauri 编译期嵌入 `ui/` 资源，改前端后必须 `cargo build` 再跑运行时验证（否则跑的是旧 JS，09-21 实踩）。
11. **体积/数量闸门放在获取前或获取中**（Content-Length 预检 + 流式累计超限即断），不放在整包缓冲之后（20e4a02）。

### 锁与并发

12. **store 锁内不得 await**；网络等重活一律锁外；读设置等短锁快取快放。
13. **同一动作的并发重入用 CAS 单 flight + Drop guard**，抢不到静默跳过不排队（refresh 家族，b72e7cd）；跨层字段（如游标 sortkey）由后端直出，前端不自算（P0-3 B1 教训：前端算不了它拿不到的字段）。

## 项目级 Skills（`.agents/skills/`）

跟着仓库走的可复用经验（Pi 与其它遵循 Agent Skills 约定的 agent 都会从仓库根发现）：

| Skill | 用途 |
|---|---|
| `rustrss-dev-loop` | 构建/测试/运行命令 + 本项目特有的坑（ui 嵌入需重建、单实例锁、MCP 端口、日志位置与终端镜像）+ 改动前必读红线 + 文档同步义务 |
| `rustrss-headless-ui-verification` | 无头环境验证 UI 的完整手册：Xvfb 隔离实例配方、`GDK_GL=disable`、陈旧帧对策、像素差/sqlite 回读断言、无 WM 限制清单、证据三态 |
| `chorus-feature-pipeline` | 用 Chorus 从 idea 推到 ship：四道闸门、mcpScript 包封解包、CLI 多 agent 选择、文档镜像、单写者纪律、scope 例外与 FAIL 处理、收口清单 |
| `agent-shell-guard-hygiene` | dcg 守卫命中后的改写对照、`pkill -f` 自杀陷阱、进程归属判定、磁盘满应急 |
