---
title: Tech Design: 标签（文章级 tag）
proposalUuid: 0b5f9a68-c377-463d-be03-e74b4415e8c7
documentUuid:
---

# Technical Design: 标签（文章级 tag）

## 概览

四个模块化任务：**core 数据层（迁移 v12 + store API + 索引断言）→ UI 交互（阅读/列表/筛选/快捷键）→ UI 管理（侧栏区 + 置顶/颜色/拖拽 + 重命名/删除）→ MCP 工具（分权限）**。全部沿用既有架构：core 不依赖 Tauri；UI 沿用原生 JS + 子菜单机制；MCP 复用 T2 已落地的注册表/写契约/审计。

## Module Contracts

### Schema（迁移 v12）

```sql
CREATE TABLE tags (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,   -- 大小写不敏感唯一（重名 → duplicate_tag_name）
  color TEXT,                                  -- 形如 '#RRGGBB'，NULL = 默认色
  pinned INTEGER NOT NULL DEFAULT 0,
  sort_order INTEGER NOT NULL DEFAULT 0,       -- 拖拽排序（小的在前）
  last_used_at INTEGER,                        -- 打标时更新：选择器「最近使用优先」的排序键
  created_at INTEGER NOT NULL
);
CREATE TABLE entry_tags (
  entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
  tag_id   INTEGER NOT NULL REFERENCES tags(id)    ON DELETE CASCADE,
  PRIMARY KEY (entry_id, tag_id)
);
CREATE INDEX idx_entry_tags_tag ON entry_tags(tag_id, entry_id);
```

- 迁移幂等 + **升级路径测试**（旧库 → v12 数据不丢）；
- 级联：`PRAGMA foreign_keys=ON` 或显式删除（实现须在 `remove_feed`/条目删除路径验证 **无孤儿 `entry_tags`**，有测试）。

### core API（`rustrss-core`）

| 方法 | 语义 / 约束 |
|---|---|
| `create_tag(name, color?) -> Tag` | 名称 trim 后非空；重名（大小写不敏感）→ 明确错误 |
| `rename_tag(id, name)` / `set_tag_color(id, color?)` | 名称唯一约束同上；color 需校验格式 |
| `delete_tag(id, dry_run) -> DeleteTagReport{ affected_entries }` | 只清关联；`dry_run` 返回影响篇数（**与实际执行共享同一计数函数**） |
| `list_tags() -> Vec<TagWithCounts>` | 含未读计数、置顶、颜色、sort_order；**选择器排序口径：`last_used_at DESC`（无记录回退 sort_order/名称）**；计数走索引（EXPLAIN 断言） |
| `assign_tags(...)` 附带副作用 | 打标时更新被选标签的 `last_used_at`（同一事务）——这是「最近使用优先」的唯一数据来源 |
| `assign_tags(entry_ids|filter, tag_ids)` / `unassign_tags(...)` | 批量 ≤100 ids 或条件级 `{feed_id, since, until}`；幂等（重复附加无副作用） |
| `set_tag_pinned(id, bool)` / `reorder_tags(ids_in_order)`（或 `set_tag_order(id, order)`） | 排序写顺序字段；重排为单事务批量更新 |
| 条目查询扩展：`EntryQuery` 增 `tag_id: Option<i64>`；条目输出增 `tags: Vec<String>`（或 id+name） | 复用 T1 已加的显式参数模式；**默认口径不变**（界面跟随设置、MCP 固定 newest + 不隐藏已读） |

### 索引与查询策略（性能红线）

- 按标签列条目：`idx_entry_tags_tag(tag_id, entry_id)` → 先取 tag 的 entry_id 集，再按既有排序索引取条目（或反向 join），**必须带 EXPLAIN 断言**（不得退化 `SCAN entries`）；
- 标签未读计数：`COUNT(*) ... WHERE tag_id=? AND read=0` 路径须走覆盖索引（`idx_entry_tags_tag` + entries 的 read 部分索引/覆盖索引），**不得触碰正文大列表 B 树**；
- 断言与线上 SQL 同源，并做**变异校验**（去掉索引/条件应转红）——沿用 v11 与审计批次的既定做法。

### UI 落点与交互

| 位置 | 文件 | 交互 |
|---|---|---|
| 阅读器 chips + 选择器 | `ui/app.js`（渲染 + 键盘）、`ui/style.css`（chip/圆点/选择器样式）、`ui/i18n.js` | meta 行 chips（点击筛选）＋「＋标签」；选择器：输入过滤、↑↓/Enter/Esc、Enter 新建并附加、**最近使用优先（`last_used_at DESC`）**；空状态明示语义分工（星标=收藏 · 稍后读=待读 · 标签=主题分类） |
| `t` 快捷键 | `ui/app.js`（keydown 分发） | 与 u/s/l 同族；选择器打开时键盘焦点在输入框（Esc 无副作用） |
| 列表行 chips | `ui/app.js`/`style.css` | ≤2 个 chips + `+N`；行 hover 显示标签按钮 |
| 标签视图 | `ui/app.js`（views 机制） | 与「全部未读/星标/稍后读」并列的筛选入口（tag 维度） |
| 侧栏「标签」区 | `ui/app.js`/`index.html`/`style.css` | 折叠区；颜色圆点 + 名称 + 未读计数；置顶优先；**拖拽排序**（原生 drag 事件，落库顺序字段） |
| 标签项菜单 | 复用 `openContextMenu` + 子菜单机制 | 重命名 / 颜色（预设色板）/ 置顶切换 / 删除（确认弹窗 + 影响篇数） |

### MCP 契约（复用 `2026-09-23-mcp-write` 批次已落地的基建：`registry.rs` / `write_contract.rs` / `audit.rs`，commit `30a476c`）

- 注册表登记：`list_tags` = (read, dangerous=false)；`create_tag`/`rename_tag`/`assign_tags`/`unassign_tags`/`delete_tag` = (write, dangerous=false)；`delete_tag` 内部强制 `confirm: true`；
- `list_articles` 增参 `tag_id`/`tag_name`（二者互斥，同传报 `invalid_argument`），响应条目增 `tags` 字段（名称数组，控制在体积上限内）；
- 错误码新增：`tag_not_found`、`duplicate_tag_name`、`invalid_argument`；沿用 `write_scope_required`/`write_disabled`/`confirm_required`；
- 审计：全部 tag 写操作落 `target=mcp`（含 `dry_run` 与失败行）；
- **危险工具集合保持 MCP 批次的既定定义**（`unsubscribe` / `folder_delete` 由该批次 T4 落地并受 `mcp.dangerous_enabled` 开关约束）；**tag 删除不进该集合**，仅要求 `confirm: true`。

## 任务拆分与依赖

| 任务 | 内容 | 依赖 |
|---|---|---|
| T1 core 数据层 | 迁移 v12 + store API + 级联覆盖 + EXPLAIN 断言与变异校验 + 升级路径测试 | — |
| T2 UI 交互 | 阅读器 chips/选择器/`t` 快捷键/列表行 chips/标签视图 + i18n | T1 |
| T3 UI 管理 | 侧栏标签区（计数/置顶/颜色/拖拽排序）+ 重命名/删除（确认+影响篇数）+ i18n | T2（同文件区域，串行避免冲突） |
| T4 MCP 工具 | list/create/rename/assign/unassign/delete（confirm+dry_run）+ list_articles tag 过滤与 tags 字段 + README | T1 |

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 标签计数/筛选触碰正文 B 树 → 慢（Miniflux 的教训） | 覆盖索引 + EXPLAIN 断言 + 变异校验；计数只走索引 |
| 级联遗漏 → 孤儿 `entry_tags` | FK CASCADE + `remove_feed` 路径显式测试断言零孤儿 |
| 拖拽排序在 200 行列表上的 DOM 成本 | 拖拽只在侧栏标签区（标签数量少）；排序写单事务，不做全量重写 |
| 颜色可读性（深浅主题） | 圆点 + 文字标签组合展示；预设色板挑两主题下对比度达标者；不改变文字颜色 |
| `t` 快捷键与既有键冲突 | 现状键位（j/k/u/s/l/r/g/G///?/Enter/Esc）不含 `t`；实现前再核对一次并加测试 |
| MCP 与 UI 数据一致性 | 同一 store 路径（同 core API）；MCP 写后界面刷新沿用既有事件/静默刷新机制 |
| 名称唯一性（大小写/空白） | `UNIQUE COLLATE NOCASE` + trim 校验 + 重名测试（含仅大小写差异） |
