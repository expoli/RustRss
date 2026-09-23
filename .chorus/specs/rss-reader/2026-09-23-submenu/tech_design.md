---
title: Tech Design: 右键菜单子菜单化（刷新间隔 / 移动到）
proposalUuid: 299bcb64-d94a-4af5-9d53-1005cb335718
documentUuid: 6828b843-cabd-4d0b-aa3d-f5dafe94bd97
---

# Technical Design: 右键菜单子菜单化（刷新间隔 / 移动到）

## 概览

在既有 `openContextMenu`（`ui/app.js`）上增加**子菜单条目类型**与一个子菜单定位逻辑，然后把订阅菜单的两处长列表改为子菜单。不引入构建链、不改 Rust 侧、不动数据结构。

## 现状（落点）

- `openContextMenu(ev, items, anchor)`：纯 `items` 数组渲染（支持 `label`+`action`、`checked`、`header`、`separator`、`danger`），用 `placeMenu()` 做视口定位（贴锚点向下 → 空间不足翻上 → 贴顶兜底，left/top 有下限）；`closeContextMenu()` 移除 `#ctx-menu`。
- 订阅菜单 `openFeedMenu()`（约 1507 行）：条目顺序 立即刷新 / 编辑 / （移动到 …N 个文件夹）/ 刷新间隔（header + 6 个档位）/ 取消订阅；刷新档位来自 `FEED_REFRESH_CHOICES`（`value`/`label()`，`null` = 跟随全局），当前值由 `currentFeedRefreshChoice()` 之类解析。
- 点击外部关闭、Esc 关闭已有全局处理（本次需让其作用于整棵菜单树）。

## 设计

### 条目契约（新增）

```js
// 追加的条目类型（与既有 label/checked/header/separator 共存）
{ label: '刷新间隔 · 跟随全局', submenu: () => [ /* 既有条目类型数组 */ ], chevron: true }
```

- `submenu`：懒求值函数（每次展开时重建，保证勾选/文案取的是最新状态）；
- `chevron`：父项右侧显示展开指示符（`›`，CSS 绘制，不依赖字体图形）；
- 父项**不**绑定 `action`；点击父项改为「切换展开/收起」。

### 渲染与定位

- 渲染：把现有「把 items 渲染进一个 `<div id="ctx-menu">`」的逻辑抽成 `renderMenu(items) -> HTMLElement`（可递归用于子菜单）；子菜单容器复用同一 class（`ctx-menu ctx-submenu`），以 `position: fixed` 挂到 body，`z-index` 高于父菜单。
- 定位：新增 `placeSubmenu(subEl, parentRowEl)`：
  1. 默认 `left = parentRow.right + 2`、`top = parentRow.top`；
  2. 右侧不足（`left + width > innerWidth - pad`）→ 翻到 `parentRow.left - width - 2`；
  3. 垂直：`top` 钳位到 `[pad, innerHeight - height - pad]`；高度超过可用空间 → 设 `max-height` 内部滚动（复用父菜单的滚动样式）；
  4. 贴的是父项行的 `getBoundingClientRect()`（而不是父菜单容器），因此父菜单自身滚动后重新计算即可保持对齐。
- 交互：父项 `mouseenter` → 打开子菜单（关闭其他已开子菜单）；`mouseleave`（父项 + 子菜单）延迟 ~150ms 关闭（避免移动到子菜单途中闪关）；点击父项 → 切换；子菜单内的条目点击 → 执行 action 并关闭整棵树；父菜单滚动 → 收起子菜单（简单且不会错位）。
- 关闭：`closeContextMenu()` 同时移除子菜单；Esc / 点击外部沿用既有入口（确保能关整棵树）。

### 接入（订阅菜单）

- 「刷新间隔」条目：`label = t('menu.refreshInterval') + ' · ' + currentLabel`（`currentLabel` 复用 `FEED_REFRESH_CHOICES` 的 `label()`），`submenu: () => FEED_REFRESH_CHOICES.map(...)`（含 `checked`）；
- 「移动到」条目：`label = t('menu.moveTo') + ' · ' + (currentFolder?.name ?? t('menu.ungrouped'))`，`submenu: () => [...folders(含 checked: isCurrent), 未分组项(含 checked)]`；
- 删除现在平铺的 header/档位/文件夹条目。

### i18n

- 新增 key（zh-CN + en）：`menu.moveTo`（若不存在）、`menu.ungrouped`（若不存在）；分隔符 `·` 直接拼接（不入 i18n）；父项「刷新间隔 · …」复用既有 `menu.refreshInterval`。

### CSS

- `.ctx-submenu`（定位/滚动/最小宽度与父菜单一致）；
- `.ctx-item.has-submenu`（右侧 chevron、`::after { content: '›' }` 或内嵌 span）、hover 高亮沿用既有样式。

## 实施顺序

单任务完成：条目契约 + `renderMenu` 抽取 + `placeSubmenu` + 交互（hover/click/关闭）→ 订阅菜单两处接入 + i18n → CSS → Xvfb 验证 → 回归检查（其余菜单）。

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 悬停/移出时序导致子菜单闪关或点不到 | 关闭延迟（~150ms）+ 父项与子菜单共用一个「悬停区」判定 |
| 父菜单滚动/重渲染后子菜单错位 | 滚动即收起子菜单；懒求值 `submenu()` 每次重算 |
| 视口边缘溢出 | 右侧翻转 + 垂直钳位 + 内部滚动；Xvfb 用右/下边缘用例截图验收 |
| 其余菜单回归 | `openContextMenu` 既有条目类型行为不变，仅新增类型；回归清单：文件夹菜单、列表菜单、设置下拉、Esc/外点关闭 |
| 点击父项误触 | 父项无 action，点击仅切换展开（不会误执行操作） |
