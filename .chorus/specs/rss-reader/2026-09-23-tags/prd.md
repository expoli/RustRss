---
title: PRD: 标签（文章级 tag）
proposalUuid: 0b5f9a68-c377-463d-be03-e74b4415e8c7
documentUuid: bf8d16f3-dda1-4c1f-9407-722b973da0c0
---

# PRD: 标签（文章级 tag）

## 背景

用户提出：阅读器需要 **tag（文章级标签）**，且 **MCP 必须支持 tag 的增删改查并按权限分权**。现状：库内只有 `folders`（**feed 级**分组）与条目上的 `read`/`starred`/`read_later`，没有标签模型。

竞品调研（决定设计取舍）：
- **FreshRSS**：用户 labels（可增删改、支持自动打标、搜 `label:`）与 feed 自带只读 article tags 是**两套**；label 与 category 共用 API 命名空间 → **撞名会失败**；
- **Miniflux**：feed 只能归一个 category（≈文件夹）；条目 tags 来自 feed 且存 `text[]` **无索引 → 按标签查询慢**；用户可编辑标签是最近极简 PR；
- **Inoreader**：folders（feed 分组，一个 feed 可属多个）与 **tags（文章级标签）**分开；每个 tag 自带 feed、有 dashboard（排序/置顶/增删改）、规则、阅读时 `T` 打标、多 tag 交叉筛选；
- **Feedly**：Boards（保存容器）+ `T` 快捷键；**NetNewsWire**：文章级打标仍是未实现 request；**Folo**：社区诉求是「首页 top tag 条一键筛选」。

细化已确认：仅**文章级**标签；v1 管理增强做**置顶 + 颜色 + 拖拽排序**；`delete_tag` = write 级 + confirm + dry_run；**不**解析 RSS `<category>`。

## 需求

### FR-1 数据模型与 core 能力（迁移 v12）

- 新表：`tags(id, name TEXT NOT NULL UNIQUE COLLATE NOCASE, color TEXT, pinned INTEGER NOT NULL DEFAULT 0, sort_order INTEGER NOT NULL DEFAULT 0, last_used_at INTEGER, created_at INTEGER NOT NULL)`（`last_used_at` 在打标时更新，驱动选择器「最近使用优先」）；`entry_tags(entry_id, tag_id, PRIMARY KEY(entry_id, tag_id))`，两端 `ON DELETE CASCADE`；
- 索引：`idx_entry_tags_tag(tag_id, entry_id)`（按标签列条目）；标签计数所需索引按查询计划确定并断言；
- core API：创建 / 重命名 / 删除标签；列出标签（含**未读计数**、置顶、颜色、顺序）；给条目**批量附加/移除**标签（ids ≤100 或条件级 `{feed_id, since, until}`）；设置置顶/颜色/顺序；按标签筛选条目（`tag_id` / `tag_name`）且条目输出可携带其标签；
- **级联覆盖**：`remove_feed` / 退订导致的条目删除必须同时清理 `entry_tags`（补测试）；
- 迁移 v12 幂等、含升级路径测试（旧库 → v12 后数据不丢）。

### FR-2 UI：阅读与筛选交互

- 阅读器：meta 行展示 tag chips（点击 = 按该标签筛选列表）＋「＋标签」按钮；选择器支持 type-ahead 过滤、Enter 新建并附加、**最近使用优先**（排序键 `last_used_at DESC`，无记录回退 `sort_order`/名称）；
- **语义分工明示**：在标签选择器/空状态处展示「星标=收藏 · 稍后读=待读 · 标签=主题分类」（i18n 双语，有断言）；
- 快捷键 **`t`**：给当前文章打开标签选择器（与 u/s/l 同族）；选择器内 ↑↓ 选择、Enter 确认、Esc 关闭；
- 列表行：显示最多 2 个 chips（超出显示 +N）；
- 侧栏/列表支持**标签视图**（按标签筛选，等价于现有智能视图机制）；
- UI 明确三套语义分工：**星标=收藏 · 稍后读=待读 · 标签=主题分类**。

### FR-3 UI：标签管理

- 侧栏新增可折叠**「标签」区**：列出标签（颜色圆点 + 名称 + 未读计数），**置顶优先**，支持**拖拽排序**；
- 标签项右键菜单：重命名 / 颜色 / 置顶切换 / 删除（复用既有子菜单机制与样式）；
- 删除标签需确认弹窗，并显示**将影响 N 篇文章**（影响面与实际执行同源）。

### FR-4 MCP：标签增删改查（分权限）

| 工具 | scope | 说明 |
|---|---|---|
| `list_tags` | **read** | 含未读计数、置顶、颜色；只读 token 可见 |
| `create_tag` / `rename_tag` | **write** | 受写 token + 写开关约束 |
| `assign_tags` / `unassign_tags` | **write** | ids ≤100 或条件级；复用写契约与审计 |
| `delete_tag` | **write + `confirm` 必填** | 支持 `dry_run` 预览影响文章数；删除只清关联不删文章 |
| `list_articles` 增 `tag_id`/`tag_name` 过滤 + 返回体带 `tags` | read | 不改变既有默认口径（newest + 不隐藏已读） |

- 全部复用**现有 MCP 基建**（来源：`2026-09-23-mcp-write` 批次已落地的 `crates/rustrss-mcp/src/{registry,write_contract,audit}.rs`，commit `30a476c`）：工具注册表（scope/dangerous）、**每请求现算 scope**、工具级错误码、`target=mcp` 审计（参数过 scrub）、写契约（批量 ≤100 / confirm / dry_run）；
- 错误码（新增）：`tag_not_found`、`duplicate_tag_name`、`invalid_argument`；沿用 `write_scope_required` / `write_disabled` / `confirm_required`。

## 非功能要求

- **性能红线**：标签筛选与计数必须有索引 + **EXPLAIN 断言 + 变异校验**（不得退化 `SCAN entries`、不得触碰正文大列表 B 树——沿用 `counts()` 教训）；拖拽排序只改顺序字段，不做全量重写；
- i18n 双语（新增文案 zh-CN + en，key-set 测试守护）；core 不依赖 Tauri；UI 无构建链；
- 行为变更同步 README 与 durable spec；MCP 文档补 tag 工具与权限口径。

## 验收口径（行为级）

1. 给一篇文章打 2 个标签 → 重启后仍在；两处（阅读器 chips / 列表行）一致；
2. 按标签筛选：列表只显示该标签条目，计数正确；标签视图与「全部未读」等智能视图并存不冲突；
3. `t` 快捷键可打开选择器并完成「输入 → 新建 → 附加」；Esc 无副作用；
4. 侧栏标签区：颜色/置顶/拖拽排序重启后保持；重命名即时生效；删除弹窗显示影响篇数，确认后标签消失且文章仍在（仅关联清除）；
5. MCP：读 token 只能 `list_tags` 与按标签读；写 token 可增删改查；`delete_tag` 缺 `confirm` 报 `confirm_required`，`dry_run` 返回影响数且不落库；重名创建报 `duplicate_tag_name`；
6. 性能：按标签查询与标签计数走索引（EXPLAIN 断言 + 变异校验通过）；`remove_feed` 后无孤儿 `entry_tags`。

## 范围外（v2 候选）

- 解析 RSS `<category>` 作为只读来源标签；feed 级标签（新条目默认标签）；
- 自动打标规则（FreshRSS auto-label 模式）；搜索框 `tag:` 语法糖；标签层级/嵌套；跨标签交叉筛选（多标签 AND/OR）。
