---
title: Tech Design: MCP 写能力与读侧补齐（T0+T1+T2）
proposalUuid: f29e24ed-edc3-4c88-a72d-4ed77445965d
documentUuid: dac6eda0-13fc-4c47-abea-1f6dcdba7147
---

# Technical Design: MCP 写能力与读侧补齐（T0+T1+T2）

## 概览

四处落点：**core**（查询扩展 + 聚合）、**rustrss-mcp**（工具扩展 + 作用域/开关 + 审计）、**src-tauri**（设置页开关与写 token 管理 + 设置键）、**ui**（MCP 设置区控件 + i18n）。安全模型是本次的核心复杂度：**认证（token 是否正确）与授权（该 token/开关是否允许写）分离**。

## Module Contracts

### 工具契约（名称 / 作用域 / 参数要点 / 备注）

| 工具 | 作用域 | 关键参数 | 备注 |
|---|---|---|---|
| `list_feeds` | read | — | 现有，保持 |
| `list_folders` | read | — | 新增：分组 + 每组未读合计 |
| `list_articles` | read | `feed_id` / `folder_id` / `unread_only` / `starred_only` / `read_later_only` / `since` / `until` / `page_size` / `cursor` / `sort` / `hide_read` | 默认 `sort=newest`、`hide_read=false`（**不继承界面设置**）；`page_size` 默认 10、上限 50 |
| `get_article` | read | `id` / `include_html` | 现有 |
| `search_articles` | read | `query` / `limit` | 现有 |
| `get_unread_summary` | read | `by` = feed \| folder | 新增：聚合未读 |
| `db_stats` | read | — | 现有 |
| `set_read` | write | `ids[]` 或 `{feed_id, since, until}` + `read` | 影响条数 + 逐项结果 |
| `set_starred` | write | 同上 + `starred` | |
| `set_read_later` | write | 同上 + `later` | |
| `refresh` | write | `scope` / `feed_ids[]` / `folder_id` | 与 UI 单 flight 共用 |
| `fetch_fulltext` | write | `id` | 复用全文抓取（2MiB 上限） |
| `subscribe` | write | `url`（含 `rsshub://`） | 幂等：已存在返回既有 id |
| `update_feed` | write | `feed_id` + tri-state 字段 | 复用入口与界面同一归一化 |
| `folder_create` / `folder_rename` | write | `name` / `folder_id` | |
| `folder_delete` | **dangerous** | `folder_id` + `confirm` + `dry_run?` | 删组不删订阅 |
| `unsubscribe` | **dangerous** | `feed_id` + `confirm` + `dry_run?` | 级联删除条目；`dry_run` 返回将删条目数 |
| `import_opml` / `export_opml` | write | 内容/路径 / — | 导入返回 added/skipped/errors |

### 授权模型

- **凭据**：读 token = `mcp.token`（现有）；写 token = `mcp.write_token`（新，设置页生成/轮换/销毁，随机 48 hex）；两个 token 均可用于**连接认证**；
- **HTTP**：**每个请求**按携带的 token 现算 `scope`（`read` | `write`）——**不缓存会话级 scope**，因此写 token 轮换/销毁后，旧 token 的下一个请求立即失去写权限（被拒）；`tools/list` 按**当次请求**的 scope 过滤（读 token 会话看不到写工具）；调用未授权工具返回工具级错误 `write_scope_required`；
- **stdio**：无 token → scope 由设置开关决定（`mcp.write_enabled` 开 → `write`，否则 `read`）；
- **开关**：`mcp.write_enabled`（默认 false）总开关；`mcp.dangerous_enabled`（默认 false）危险工具开关——**两者都开**且 `confirm: true` 时 `unsubscribe` / `folder_delete` 才可执行；
- **认证 vs 授权**：无/错 token → 401（传输层，不变）；已认证但无写权限 → 200 + 工具级错误（保持 MCP 语义，agent 能读懂原因）。

### 写操作契约

- 批量 ≤ 100 ids，超限返回 `invalid_argument`（或 `rate_limited`）；
- 每个写操作返回 `{ "ok": bool, "affected": n, "results": [...], "error_code"?: "..." }`（逐项结果用于部分失败的可解释性）；
- `dry_run: true`：只计算影响面（如 `unsubscribe` 的条目数、`import_opml` 的新增/跳过数），**不落库**；
- 审计：每次写调用一行 `log::info!(target: "mcp", "mcp-write tool=… args_summary=… affected=… ok=…")`（参数摘要需过 `scrub_log_line`；不含正文/凭据）。

### 共享设施上提 core（MCP 与界面共用）

现有两处能力位于 `src-tauri`，`rustrss-mcp`（独立二进制）无法访问，必须上提复用：

- **`scrub_log_line` / `mask_url_userinfo`**（现 `src-tauri/src/commands.rs`）→ 上提到 `rustrss-core`（如 `core::logging::scrub`），src-tauri 改为复用；MCP 审计行与错误路径直接调用 core 版本；
- **刷新单 flight**（现 src-tauri 的 CAS 守卫 + Drop guard）→ 抽到 `rustrss-core`（如 `core::refresh_flight`），src-tauri 与 `rustrss-mcp` 共用同一实现与状态，保证 MCP 的 `refresh` 与界面刷新**不叠加**（进行中调用返回可解释状态）；
- 两者均为**纯搬迁 + 复用**，不改变行为（界面路径的既有测试与单 flight 行为保持绿）。

### core 查询扩展

- `EntryQuery` 增 `since: Option<i64>` / `until: Option<i64>`（对 `COALESCE(published_at, fetched_at)`）与 `feed_ids: Option<Vec<i64>>`；
- SQL 组装：`feed_ids` → `IN (…)` 绑定；时间范围 → `>=` / `<=` 绑定；**为两者补 EXPLAIN 断言**（复用现有 `explain_*` 帮手与变异校验口径，确保不退化 `SCAN entries`；若计划退化则新增表达式索引 `(COALESCE(published_at, fetched_at))` 或复用 v11 索引）；
- `Store::unread_summary(group_by)`：按 feed 或 folder 聚合未读（走既有计数索引口径，禁止触碰正文大列表 B 树——沿用 `counts()` 的教训）；
- 排序/隐藏已读：core 的 `list_entries` 默认仍读设置（界面用）；MCP 侧改为**显式传参**（`sort`/`hide_read`）以覆盖默认——即 MCP 不再吃界面设置。

### 设置键（新增）

`mcp.write_token`（写 token，存在才有写能力）、`mcp.write_enabled`（bool）、`mcp.dangerous_enabled`（bool）。

## 任务拆分与依赖

| 任务 | 内容 | 依赖 |
|---|---|---|
| T1 读侧补齐 | core 查询扩展（since/until/feed_ids + EXPLAIN 断言）+ `unread_summary`；MCP `list_articles` 新参数（含分页/显式 sort/hide_read）、`list_folders`、`get_unread_summary` | — |
| T2 写能力基础设施 | 双 token（生成/轮换/销毁 + 作用域）、两个开关、`tools/list` 过滤、工具级授权错误码、审计日志、批量/confirm/dry_run 契约与共享校验 helper、设置页 MCP 区控件 + i18n | — |
| T3 阅读状态与刷新写工具 | `set_read` / `set_starred` / `set_read_later` / `refresh` / `fetch_fulltext`（含批量与单 flight 语义） | T2 |
| T4 订阅管理写工具 | `subscribe` / `update_feed` / `folder_*` / `unsubscribe`（危险）/ `import_opml` / `export_opml` | T2 |

串行执行（单写者）；T3/T4 可并行逻辑上但受单写者约束串行。

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 授权模型实现错导致写工具被读 token 调用 | 逐条测试：读 token 会话 `tools/list` 不含写工具 + 直接调用返回 `write_scope_required`；开关关闭同理 |
| 危险操作误删（unsubscribe 级联） | `confirm` + `dry_run` + 默认禁用 + 审计行；dry_run 实际影响面与实际执行共享同一计算函数（避免预览与执行不一致） |
| core 新过滤导致查询退化 | EXPLAIN 断言 + 变异校验（去掉索引/条件应转红） |
| 写路径与界面并发（同一源被 UI 与 agent 同时改） | 复用 store 事务与单 flight；refesh 与 UI 共用 CAS；写操作幂等 |
| agent 误用批量接口（一次 1000 条） | 硬上限 100 + `invalid_argument` + 文档写明 |
| 审计泄漏敏感信息 | 参数摘要过 `scrub_log_line`；不记录正文 |

## 验证要求（每任务）

- 测试先行：core 查询/聚合带单测 + EXPLAIN 断言；MCP 工具与授权矩阵（读/写 token × 开关 × 危险工具）带测试；
- 实机：设置页 MCP 区截图（开关/写 token 生成/轮换）；HTTP 实测：读 token 调用写工具 → 工具级错误；写 token 调用写工具 → 成功且库变更可回读；stdio 侧最小冒烟（开关关闭时写工具不可用）；
- 收口：`cargo test --workspace` 全绿 + `cargo clippy --workspace` 无新增；README（MCP 章节）与 durable spec（通道 B 条目）同步。
