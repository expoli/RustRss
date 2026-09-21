---
title: PRD: 订阅文件夹——侧栏分组展示与管理
proposalUuid: 33b811ee-96a8-4cf0-9139-4ab94ea41716
documentUuid: f135c4a6-9531-4ce5-9ea9-a247f2a4d29c
---

# 订阅文件夹 — 需求与取舍

> 状态：待评审。参考 MrRSS：侧栏文件夹分组 + 组头聚合未读数 + 折叠。数据层早已存在（folders 表 / feeds.folder_id / add_folder·list_folders·assign_folder），本 change 把它暴露到 UI 并补齐管理闭环。

## 1. 已定决策（elaboration round 1，2026-09-21）

| 决策点 | 选择 | 理由 |
| --- | --- | --- |
| 分组渲染 | 可折叠分组 + 组头聚合未读数 | 对标参考产品；聚合未读数是分组的核心价值 |
| 层级策略 | v1 维持扁平单层，OPML 嵌套继续压平为 `父/子` 组名 | folders 表/OPML 压平是现状；真嵌套需迁移 + 递归 UI，参考产品亦为单层组 |
| 删除文件夹 | 订阅保留移出到未分组（folder_id=NULL） | 订阅是用户资产；与外键 ON DELETE SET NULL 一致 |
| 归组交互 | 右键菜单（订阅→移动到；分组→重命名/删除） | 实现小、语义清晰；拖拽列非目标（后续增强） |
| 未分组位置 | 分组之后平铺（组按 position 排序） | 组有 position 字段天然在前；未分组作默认区垫底 |
| 折叠状态 | 存 SQLite settings（跨会话保持） | 与主题/语言同机制 |
| 文档义务 | README 特性 + spec.md 订阅相关验收项如实回填 | AGENTS.md 硬性义务 |

## 2. 改动面

| 文件 | 改动 |
| --- | --- |
| `crates/rustrss-core/src/store/mod.rs` | 补 `rename_folder` / `delete_folder`（订阅置 NULL）/ `set_folder_position` + 单测 |
| `src-tauri/src/commands.rs` | folder CRUD / 归组命令（重命名冲突、删除确认语义与既有命令一致） |
| `ui/app.js` | renderSidebar 分组渲染（组头 + 聚合未读 + 折叠）、右键菜单（移动到/重命名/删除）、新建分组入口、折叠状态持久化 |
| `ui/i18n.js` | 新文案 zh-CN/en 双份 key |
| `README.md` / `spec.md` | 特性清单与验收项如实回填 |

## 3. 非目标

- 文件夹真嵌套（parent_id）。
- 拖拽归组。
- 文件夹级抓取/标记已读等批量操作（后续增强）。

## 4. 风险

- 右键菜单为自绘（无原生菜单）：需处理点击外部关闭、键盘 Escape。
- 组聚合未读数依赖 feed 未读数求和：与现有单 feed 未读数同源，注意刷新后即时更新。
- 重命名撞 UNIQUE 约束：错误信息透出为可读提示。

## 5. 评审记录

- Round 1 待审。
