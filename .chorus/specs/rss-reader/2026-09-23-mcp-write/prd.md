---
title: PRD: MCP 写能力与读侧补齐（T0+T1+T2）
proposalUuid: f29e24ed-edc3-4c88-a72d-4ed77445965d
documentUuid:
---

# PRD: MCP 写能力与读侧补齐（T0+T1+T2）

## 背景

现有 MCP 为**只读 5 工具**（`list_feeds` / `list_articles` / `get_article` / `search_articles` / `db_stats`）：agent 能看不能动；读侧本身也有缺口——需求基线（spec 通道 B）要求「列条目（按源/未读/**时间**过滤、**分页**）」，而 `EntryQuery` 无时间过滤字段、MCP 也未暴露已实现的 keyset 游标。本批次让 agent 能真正操作订阅工具。

**用户已拍板**：① 读写 token 分权；② 危险工具默认禁用 + 设置页开启；③ **T3 不做**（不暴露 AI 摘要/翻译、不做 settings 读写）。细化补定：危险工具 = `unsubscribe` + `folder_delete`；stdio 与 HTTP 同口径（写能力受同一开关，默认关）；MCP 列表默认固定「最新在前 + 不隐藏已读」。

## 需求

### FR-1 读侧补齐

- `list_articles` 增参：`read_later_only`、`page_size` + `cursor`（keyset 分页，与界面同源游标）、`folder_id`、时间范围 `since`/`until`、`sort` / `hide_read`（显式覆盖）；
- **MCP 默认口径固定**：未显式传参时 = 最新在前 + 不隐藏已读（不随界面设置变化；界面排序/隐藏已读仍只影响界面）；
- 新增 `list_folders`（分组树 + 每组未读合计）；
- 新增 `get_unread_summary`（按源/分组的未读聚合，供 agent 决定处理顺序）。

### FR-2 写能力基础设施（安全先行）

- **读写 token 分权**：读 token 常驻（沿用现 `mcp.token`）；**写 token** 在设置页显式生成/轮换/销毁，未生成则写工具不可用；
- **写能力总开关**（默认关）；**危险工具开关**（默认关，单独开启后 `unsubscribe` / `folder_delete` 才注册）；
- **传输口径**：stdio 与 HTTP 一致——写能力同样受开关约束；HTTP 侧按连接持有 token 判定（读 token 会话看不到写工具，写 token 会话可见可用）；
- 无权限时返回**工具级错误**（`write_scope_required` / `write_disabled` / `dangerous_tool_disabled`），不是 401（已认证、无授权）；
- **危险操作契约**：`confirm: true` 必填；`dry_run: true` 返回影响面预览（如将删除的条目数）而不落库；
- **批量与限流**：批量 ids ≤ 100；`refresh` 与界面共用单 flight 不叠加；
- **审计**：所有写操作落日志（复用现有日志设施，`target = mcp`），含工具名、参数摘要、影响条数、结果；
- **错误码机器可读**：`feed_not_found` / `article_not_found` / `invalid_url` / `invalid_argument` / `write_disabled` / `write_scope_required` / `dangerous_tool_disabled` / `rate_limited` / `confirm_required`；
- **幂等**：`subscribe` 重复 URL 返回既有 feed id（不报错）；`set_*` 重复设置幂等。

### FR-3 阅读状态与刷新（写闭环）

- `set_read` / `set_starred` / `set_read_later`：入参 `ids[]`（≤100）或条件级（`feed_id` + `since/until` 等），返回「影响条数 + 逐项结果」；
- `refresh`：`scope` = `all` / `feed_ids[]` / `folder_id`；与界面单 flight 共用，进行中返回 `rate_limited` 或 in-flight 状态；返回本轮结果摘要（fetched/inserted/failures）；
- `fetch_fulltext`：单条补全文（复用既有全文抓取，受体积上限保护）。

### FR-4 订阅管理

- `subscribe`（`url` 或 `rsshub://path`；首页自动发现；**幂等**）；
- `update_feed`（`custom_title` / `folder_id` / `refresh_interval_minutes`，tri-state patch 与界面同源）；
- `folder_create` / `folder_rename` / `folder_delete`（删组不删订阅，与界面语义一致）；
- `unsubscribe`（**危险**：`confirm` + `dry_run`）；
- `import_opml`（内容或路径；返回 added/skipped/errors）+ `export_opml`（返回 OPML 文本）。

## 非功能要求

- **既有 MCP 安全口径不放宽**：仅回环、token 鉴权、无/错 token 401、`/health` 不含订阅数据（均有测试）；
- 响应口径沿用：列表只回元数据 + ≤140 字摘要、默认 10 / 上限 50、正文用 `get_article` 单取；
- core 不依赖 Tauri；UI 无构建链；i18n 双语（设置页新文案）；
- **性能红线**：core 新增的时间范围/多源过滤必须带 EXPLAIN 断言（不得退化为裸 `SCAN entries`；必要时新增表达式索引并做变异校验）；
- 测试先行：每个新工具与安全开关均有测试；行为变更同步 README 与 durable spec。

## 验收口径（行为级）

1. agent 用**读 token** 连接：只能看到/调用只读工具；写工具调用返回 `write_scope_required`（不泄露数据、不改库）；
2. 未生成写 token 或写开关关闭时：写工具不可用（工具列表不含或调用返回 `write_disabled`）；
3. 危险工具开关关闭时：`unsubscribe` / `folder_delete` 不可用；开启后 `confirm: true` 才生效，`dry_run: true` 只返回预览且库不变；
4. 典型闭环可完成：`list_articles(unread_only, since)` → `get_article` → `set_read(ids)` → `db_stats` 未读数一致；
5. `subscribe` 重复 URL 幂等；`unsubscribe` 后 `list_feeds` 不再含该源且条目级联删除；
6. 分页：`page_size`+`cursor` 连续翻页不重不漏（与界面同一游标语义）；
7. 所有写操作在日志文件中留有审计行（工具名/参数摘要/影响条数）。

## 范围外

- **T3**：AI 摘要/翻译、settings 读写白名单；
- 任意 SQL / 文件系统路径、备份/恢复、API key 读写；
- 条目硬删除（RSS 语义不做）；
- MCP 传输层大改（不引入 OAuth；token 即凭据）。
