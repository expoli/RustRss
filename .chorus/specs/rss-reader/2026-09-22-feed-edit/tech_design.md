# Tech Design: 订阅源编辑对话框

- 模块: rss-reader / feed-edit
- 日期: 2026-09-22

## 核心问题：自定义标题的存储与刷新保护

feeds.title 会被 feed 元数据刷新覆盖（源站改名跟着变）。自定义标题需要独立列：

```sql
ALTER TABLE feeds ADD COLUMN custom_title TEXT;  -- NULL = 用源站名
-- 迁移 v10：显示层 COALESCE(custom_title, title)
```

- `sidebar_data` / `list_feeds` / EntryRow.feed_title 全部改 `COALESCE(custom_title, title)`
- feed 元数据 upsert（apply_results 里 title 更新）**不动 custom_title**
- 条目表 feed_title 冗余列：现有 entries 不存 feed_title（EntryRow 查询时 JOIN）？——实查 store 的 FEED_ROW_SELECT / list_entries JOIN，displayed feed_title 换 COALESCE 表达式即可，无需数据迁移

## 命令（src-tauri）

```rust
#[tauri::command]
pub fn update_feed(
    state: State<'_, AppState>,
    feed_id: i64,
    title: Option<String>,      // None=不动；Some("")=清除自定义；Some(s)=设自定义
    folder_id: Option<i64>,     // 不动=省略语义需要 Option<Option<i64>> 或哨兵；用 JSON 参数对象更清晰
    refresh_interval: Option<Option<u32>>,  // 同 set_feed_refresh_interval 口径
) -> R<FeedRow>
```

简化：`set_feed_config(feedId, customTitle: Option<String>, folderId: Option<i64>, refreshIntervalMinutes: Option<String>)`——None 字段不动；customTitle 空串=清除。folder 复用 reassign_feed 落库、interval 复用现有归一化。

## UI

- openFeedMenu 加「编辑」项（立即刷新后），action → openFeedEditDialog(feed)
- 对话框复用 confirm-sheet 样式（small-sheet）：三字段 + 保存/取消
  - 文件夹/间隔用自绘下拉（SETTING_DROPDOWNS 是设置页专用，这里用局部状态 + openContextMenu anchored 到字段行——同设置页模式）
  - 标题 input + placeholder=源站名
  - URL 只读 code 块
- 保存 → invoke → 更新 state.feeds + renderSidebar + 若当前 feed 视图则 viewTitle 更新 + 状态栏提示

## 测试

- core: custom_title 迁移 + COALESCE 显示 + 刷新 upsert 不覆盖 custom_title
- src-tauri: set_feed_config 部分写入（不动字段不落库）
- UI: 手动清单（改标题立即生效 + 刷新保留 + 对话框取消无副作用）
