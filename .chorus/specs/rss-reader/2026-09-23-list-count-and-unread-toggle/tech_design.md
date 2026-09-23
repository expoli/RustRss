---
title: Tech Design: 列表计数口径 + 只看未读入口 + 大数据量加载兜底
proposalUuid: 6ce07339-a615-4e1a-9823-9eb03a6ef162
documentUuid: 9902402e-d271-4985-ac01-50bea59619f3
---

# Tech Design: 列表计数口径 + 只看未读入口 + 大数据量加载兜底

## 0. 约束（先摆红线）

- 仓库性能红线 #1：**COUNT 不得触碰 `entries` 正文大列所在的 B 树**——聚合列必须被覆盖索引或部分索引覆盖，新增计数必须带 EXPLAIN 断言（与线上 SQL 同源），并做变异校验（去掉索引/约束时测试要变红）。
- 红线 #6：热路径禁止全量重建 DOM。本批只改「列表头文案 / 开关 / 哨兵行」，不改行渲染路径；续页沿用 append。
- 硬约束 #4：UI 无构建链，新增文案必须同时补 `ui/i18n.js` 的 zh-CN 与 en（key-set 一致性自检会跑）。
- 硬约束 #1：业务逻辑进 `rustrss-core`，`src-tauri` 只做胶水（命令 → core 调用）。

## 1. 计数口径（T1）

### 1.1 总数字段来源（全部走索引计数）

| 视图 | 有效筛选 | N（总数）来源 | 是否已存在 |
|---|---|---|---|
| 全部未读（全局） | unread | `state.db.unread`（`Store::counts()` 子查询 `read_published`） | ✅ 已有 |
| 全部（全局） | 无 | `state.db.entries`（`counts()` 子查询 `sortkey`） | ✅ 已有 |
| 星标 / 稍后读 | starred / read_later | `state.db.starred` / `state.db.later`（v9/v4 部分索引） | ✅ 已有 |
| 单源 | unread | 侧栏 `FeedRow.unread`（`unread_summary`，`INDEXED BY (feed_id, read)`） | ✅ 已有 |
| 单分组 | unread | 侧栏 `FolderRow.unread`（同上，分组维度） | ✅ 已有 |
| 单标签 | unread | 侧栏 `TagRow.unread` | ✅ 已有 |
| 单源 / 单分组 / 单标签 + **「全部」视图** | 无 | 源：复用 `entry_count_for_feed`；标签：复用 `tag_entry_count`；**分组：新增 `entry_count_for_folder`** | 仅分组新增 |
| 开启「隐藏已读」后的任意视图 | unread | 同上「unread」行（有效筛选就是 unread） | ✅ 已有 |
| 搜索 | FTS + hide_read | 不显示 N，只显示「已加载 M 篇」 | — |

关键设计：**「共 N」按有效筛选计算**——开着「隐藏已读」时 N 就是该 scope 的未读数，而不是库总量，避免「列表里只有未读、头部却写全库总量」的第二次歧义。未读类数字**一律取已加载的侧栏状态**（`refreshCounts()` 每次标读后已刷新），不额外发查询；只有「全部视图 + 单源/单分组/单标签」这一格需要新查询。

### 1.2 取数（core：多数是复用，只有分组总数是新增）

- **复用**（均已存在，附 EXPLAIN 断言）：`Store::counts()`（全局四数）、`Store::entry_count_for_feed()`（源总数，`INDEXED BY idx_entries_feed_read`）、`Store::tag_entry_count()`（标签总数，`TAG_ENTRY_COUNT_SQL`，`INDEXED BY idx_entry_tags_tag`）、`unread_summary()` / `TagRow.unread`（各维度未读，走 `idx_entries_unread_id` 部分索引）。
- **新增只有一个**：`Store::entry_count_for_folder(folder_id) -> i64`——命名对齐既有 `entry_count_for_feed`；SQL 沿 `UNREAD_COUNT_BY_FOLDER_SQL` 的同族写法（`entries INDEXED BY idx_entries_feed_read` join `feeds` 按 `folder_id` 定位，不加 read 谓词），COUNT 只扫覆盖索引、不碰正文表 B 树；配 `explain_entry_count_for_folder` + EXPLAIN 断言 + 变异校验（删索引即变红）。
- **未读口径**：一律复用现有部分索引写法（`idx_entries_unread_id(id) WHERE read = 0`）；**不得**改成裸 join `entries.read`（`read` 列排在 11.5KB 正文大列之后，回表取它就是 `counts()` 那条 83-119ms 冷启动教训）。
- **胶水层**：新增命令 `list_scope_total { kind, id } -> i64`，按 kind 分派到上述三个总数；智能视图（未读/全部/星标/稍后读）不经过它，`N` 直接读侧栏已加载的 counts 结果；前端调用点收敛为一个 `viewTotal()` helper，带 (kind, scopeId, effectiveFilter) 缓存与刷新失效。

### 1.3 UI

- `renderList()` 头部文案：`已加载 {m} / 共 {n}`；未读有效筛选下写 `已加载 {m} / 共 {n} 未读`；搜索写 `已加载 {m} 篇`；`n` 未知（查询失败）时降级为 `已加载 {m} 篇` 并在日志留一行 warn（不弹错）。
- 新增 i18n key：`list.loadedOfTotal`、`list.loadedOfTotalUnread`、`list.loadedOnly`、`list.loadMore`、`list.allLoaded`、`list.filterUnread`、`list.filterUnreadHint`（双语各 7 个）。
- 自检：现有 `i18nSelfTest` 自动覆盖 key-set；不改快捷键注册逻辑（见 T2）。

## 2. 「只看未读」常驻开关（T2）

- 位置：列表头右侧、排序按钮左侧；渲染为可切换按钮（`aria-pressed` + 高亮态），文案 `只看未读`；点击走既有 `set_list_hide_read` 命令（**不新增设置键、不改存储口径**）。
- 与排序菜单里的旧入口共用同一设置：两处状态由 `state.settings.list_hide_read` 单一来源渲染，切换后 `applyListSetting()` 立即重查重渲（reset 语义、分页状态重置），并同步刷新两处 UI（同值短路，避免重复写）。
- 快捷键：`U`（`u` 已是「切换已读」，大写 `U` 语义为「只看未读」）；登记进快捷键自检清单（`[ui] shortcut selftest ok (keys=...)` 会打印），并在输入区/覆盖层里照旧不触发。
- 豁免语义不变：星标 / 稍后读视图豁免、搜索不豁免（store 层已实现，本批只做入口）。
- 交互防抖：切开关可能的「列表跳走」用现有 reset 语义（不改灰显/延迟离开策略，见 prd 非目标）。

## 3. 哨兵行：手动「加载更多」 + 进度（T3）

- `installSentinel()` 产出的 `<li class="load-sentinel">` 从纯文案改为两态：
  - 空闲且有下一页：可点击按钮「加载更多（已加载 M / 共 N）」，点击直接调 `loadMore()`（仍受 `paging.loading/error/exhausted` 守卫）。
  - 请求在飞：显示「加载中…」，保持不可点。
- 续页失败后不再只提示「换视图恢复」：保留哨兵行，按钮回到可点态（`paging.error` 时文案加「重试」语义），`loadMore()` 里失败路径改为「允许手动重试、但不自动重试」——用 `paging.error` 只是拦住 `IntersectionObserver` 的自动触发（避免把后端打满），手动点击时显式清 `error` 再请求。
- 终止态：`paging.exhausted` 时不渲染哨兵行（现状保持），但**在列表底部给一行明确的「已到末尾（共 N 篇）」**，避免用户以为还有更多。
- 进度文案复用 T1 的 `viewTotal()`；`M` 取 `state.entries.length`（搜索视图直接显示已加载）。

## 4. 测试计划

- **core 单测**：`entry_count_for_folder`（新增）与复用的 `entry_count_for_feed` / `tag_entry_count` 的正确性（含空分组、跨分组源、同一条多标签不重复计数）+ EXPLAIN 断言（无裸 `SCAN entries`）+ 变异校验（删除索引后断言变红）。
- **commands 测试**：`list_scope_total` 命令路径（各 kind 的返回值与 `list_entries` 同源口径一致；未知 kind/不存在 id 的报错）。
- **UI 自检**：i18n key-set（双语齐备）；快捷键自检包含 `U` 且与既有键位无冲突（已有 `tests::global_shortcuts_have_no_conflicts*` 同族测试）。
- **实机验证**（按仓库惯例落 `manual-verification-checklist.md`）：未读视图头显 `已加载 M / 共 N 未读`（N 实测值写进清单，不写死数字；与左下状态栏/侧栏同口径数字一致）；滚到底 → M 递增、N 不变，到末尾出现「已到末尾」；点「加载更多」按钮能续页；开关切换立即生效且重启保留；开启后标读仍为灰显、重建时离开；搜索视图只显示已加载。
- **性能取数**：新命令按红线 #3 的口径记录——**冷启动后首次调用**（新进程 → 打开该视图即触发）与**热缓存**各测一次，两者都写进验证清单，热缓存数值必须标注（不得只报热缓存）；`[rustrss][slow]` 打点（>50ms 记 warn）与 EXPLAIN 计划一并记录。

## 5. 文档与清单同步（随任务交付）

- 本 change 目录：`prd.md` / `tech_design.md` 编辑后重新镜像（Document 版本自动递增）。
- 清单 `.chorus/specs/rss-reader/spec.md`：改写第 69（渐进加载 → 补「已加载/共 N 进度 + 手动兜底 + 终止态」）与第 70（排序与过滤 → 补「常驻开关 + 快捷键 + 总数口径」）条，按任务完成逐条勾选。
- `README.md`：AI/界面章节里列表交互的说明同步（计数语义、只看未读入口、加载更多）。

## 6. 任务拆分与依赖

| 任务 | 内容 | 依赖 |
|---|---|---|
| T1 | store 侧 scope total 查询 + EXPLAIN/变异断言 + `list_scope_total` 命令 + 头部计数 UI（含搜索回退）+ i18n + 测试 | — |
| T2 | 列表头「只看未读」常驻开关 + 快捷键 `U` + 自检登记 + i18n + 测试 | T1（同文件 UI 区域，避免并发写） |
| T3 | 哨兵行两态（手动「加载更多」/加载中）+ 失败后可手动重试 + 末尾终止态 + 进度文案 + i18n + 测试 | T1（`viewTotal()` 与进度文案）+ T2（同在 `ui/app.js`/`ui/i18n.js`，串行） |

单写者约定：三个任务都在 `ui/app.js` 与 `ui/i18n.js` 上落改动，按 T1→T2→T3 串行执行（依赖已显式声明），不并行写同一文件。
