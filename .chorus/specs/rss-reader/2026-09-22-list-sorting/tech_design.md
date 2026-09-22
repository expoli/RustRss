# Tech Design: 文章列表排序与过滤

- 模块: rss-reader / list-sorting
- 日期: 2026-09-22

## 设置（单一事实源）

```
list.sort ∈ {newest, oldest, unread_first}   默认 newest
list.hide_read ∈ {0,1}                        默认 0
```

两项都是**全局设置**：`Store::list_entries`（以及 `store.search`）在查询时从 `settings` 读出，
界面**不往 `EntryQuery` 传排序/过滤参数**——界面与 MCP 共用同一条读取路径，不存在第二个事实源。
读取侧兜底：非法值一律回默认档 / 默认关（与 locale/theme 同口径）。命令层
`set_list_sort` / `set_list_hide_read` 负责白名单归一化后落库并回显 `UiSettings`。

## store 层（keyset 游标按档位适配）

现有游标：`sortkey = COALESCE(published_at, fetched_at)`，分页 `WHERE sortkey < cursor ORDER BY sortkey DESC LIMIT n`。

| 档位 | ORDER BY | 游标谓词 | 钉住的索引 |
|------|----------|----------|------------|
| newest | `sortkey DESC, id DESC`（现状） | `sortkey <= ? AND (sortkey < ? OR (sortkey = ? AND id < ?))` | `idx_entries_sortkey`（`INDEXED BY`，仅续页时） |
| oldest | `sortkey ASC, id ASC` | 镜像：`sortkey >= ? AND (sortkey > ? OR (sortkey = ? AND id > ?))` | `idx_entries_sortkey`（反扫同一个表达式索引，仅续页时） |
| unread_first | `read ASC, sortkey DESC, id ASC` | `read >= ? AND (read > ? OR (read = ? AND (sortkey < ? OR (sortkey = ? AND id > ?))))` | `idx_entries_unread_sortkey`（**首屏也钉**） |

- **迁移 v11**：`CREATE INDEX idx_entries_unread_sortkey ON entries(read, COALESCE(published_at, fetched_at) DESC, id)`。
  列序/方向与 unread_first 的 ORDER BY 逐列对应——只有这样的复合索引能同时满足「顺序」与
  「read 等值筛选」；不钉的话（本程序从不 ANALYZE）feed 视图会退化成 `idx_entries_feed_read + USE TEMP B-TREE`。
- 游标载荷：newest/oldest 是 `(sortkey, id)`；unread_first 需要第三位 `read`，所以
  `EntryQuery` 增加一个 `cursor_read: Option<bool>`（**游标载荷**，不是设置的副本）；
  unread_first 档给了 cursor 却没给 read 时按首页处理（半截游标不静默翻错页）。
- 混合方向（read 升 + sortkey 降）无法用单条行值比较表达，故 unread_first 的续页是
  「read 范围起点 + 同 read 内的 (sortkey, id) 位置」；扫描段只碰 read/sortkey/id 三个索引列
  （索引内判定），命中行才取整行——与「穿正文溢出页链的全表扫」不是一回事。
- hide_read：非豁免视图的查询加 `AND read = 0`（unread 视图本来就只查未读，冗余无害）；
  **星标 / 稍后读视图豁免**——「星标了但读完了」还要能找到；搜索同样过滤（无搜索豁免）。
- EXPLAIN 断言与线上 SQL 同源（`explain_list_entries` 复用同一个 SQL 构造函数），
  并做过变异校验（删 v11 索引 → INDEXED BY 报错；ORDER BY 少列/换方向 → TEMP B-TREE；去掉 INDEXED BY → feed 续页退化）。

## 命令

- `set_list_sort(sort: String)` + `set_list_hide_read(enabled: bool)`（白名单归一化，回 UiSettings——两键一起回显）
- `list_entries` 增加 `cursor_read` 参数（unread_first 的游标分量，其余档忽略）
- `UiSettings` 增加 `list_sort` / `list_hide_read`

## 前端

- 列表头右侧 icon 按钮（`#btn-list-sort`）→ `openContextMenu` 锚定菜单：三档（checked）+ 分隔 + 隐藏已读（checked）；
  按钮的 onclick 里 `stopPropagation()`——全局「点菜单外面就关」的 click 监听会在同一个事件冒泡里把刚开的菜单关掉
- apply → `invoke set_list_*` → `loadEntries({ reader: false })`（reset 重建列表、正文不动）
- **prependFreshEntries 按档门控**：只有 newest 走 prepend；oldest/unread_first 走静默 reset 路径
  （新条目分别属于列表尾部 / 未读组头部，插头部会让 DOM 顺序与后端顺序不一致）。
  滚动保持随之降级为「只保留 `scrollTop` 像素偏移、不做锚点补偿」（headless 实测记录在验证清单第 15 节）
- 灰显行共存：hide_read 开启时点击行标读 → 行灰显保留（不立即删行，防脱钩），下次重建消失
- 诊断：重建路径日志固定带 `sort=` / `hideRead=` / `head=`（前 5 行 id），headless 与无人值守都能核对顺序

## 测试

- core store：三档顺序断言 + 三档 × 五种视图的续页不重不漏矩阵 + 复合游标半截回退 +
  hide_read 过滤与星标/稍后读豁免 + 搜索过滤 + unread_first/oldest 的 EXPLAIN 断言（变异校验过）+
  v10→v11 真文件迁移
- src-tauri：`set_list_*` 白名单归一化 + 落库往返（`AppState::for_test` 走同一命令体）
- UI：headless 点击序列（Xvfb + xdotool）验三档顺序 / 续页 / 隐藏已读与灰显共存 / 后台刷新门控 / 勾选态与保留；清单见第 15 节
