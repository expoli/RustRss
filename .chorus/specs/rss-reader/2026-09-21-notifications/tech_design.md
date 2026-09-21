# Tech Design: 新文章通知 + 托盘未读角标

- 模块: rss-reader / notifications
- 日期: 2026-09-21

## 依赖

- `tauri-plugin-notification`（Tauri 2 官方通知插件；实现时确认版本与平台权限：Linux 走 org.freedesktop.Notification，无需额外系统库）。
- 托盘：现有 libayatana 托盘已在 main.rs（setup_tray）；角标经 `TrayIconBuilder::icon` 换图标实现（数字角标 = 程序化绘制或预置图标集；首版用预置「圆点」图标 + tooltip 数字，数字角标为增强）。

## 分层

- `src-tauri/src/notify.rs` 新模块（Rust 侧驱动，职责单点）：
  - `maybe_notify(app, before_unread, after_unread, enabled)`：差值>0 且 enabled → 聚合通知（标题/文案走 i18n？Rust 侧无 ui/i18n.js——通知文案用 Rust 常量双语按当前 locale 设置选择，与托盘菜单文案同一模式）
  - 通知点击 → 唤出主窗口（notification 插件 action；平台不支持点击回调时降级为仅展示）
- `src-tauri/src/tray_badge.rs` 或并入现有 tray 代码：`update_badge(unread)`——数字→tooltip + 圆点图标切换；托盘不可用 no-op
- `scheduler.rs`：background_refresh 内刷新前取 unread_before，完成后取 unread_after → 调 notify + badge（手动路径不调用）
- commands.rs：`KEY_NOTIFY_NEW_ARTICLES`（默认 false）+ set 命令 + UiSettings 字段；set_read/mark_all 等改变未读数的命令完成后也更新角标（经轻量事件或直接调用——实现时选最小侵入路径）
- ui：设置页开关一行（复用 set-refresh-on-start 模式）+ i18n key ×2

## 测试

- 纯函数：差值判定/开关/文案选择单测
- 手动清单：真机后台刷新触发通知与角标更新；点击聚焦；托盘不可用降级；手动刷新无通知
