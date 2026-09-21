---
title: PRD: 稍后读——文章级标记与智能视图
proposalUuid: 9be65a9f-df7f-41a4-96a9-22c922a017b9
documentUuid: d3458076-0dda-4302-99ab-75aaf35ea873
---

# 稍后读 — 需求与取舍

> 状态：待评审。参考 Papr「未读/星标/稍后读」三视图。现有未读/星标/全部三个智能视图，稍后读完全不存在（entries 无字段）。

## 1. 已定决策（elaboration round 1，2026-09-21）

| 决策点 | 选择 | 理由 |
| --- | --- | --- |
| 数据模型 | `entries` 加 `read_later INTEGER NOT NULL DEFAULT 0`（新迁移）+ 部分索引 | 独立标记位语义清晰；部分索引只覆盖 read_later=1 行 |
| 标记入口 | 阅读器动作按钮 + 条目列表按钮（与星标同款交互） | 覆盖「浏览时标记」与「阅读时标记」两个场景，学习成本为零 |
| 排序与已读关系 | 视图按发布时间倒序；稍后读独立于已读（已读仍留在视图中） | 稍后读本义是"留到以后看"，不扭曲已读语义 |
| 快捷键 | `l` 键切换（阅读器内），更新快捷键一览 | spec 已有键盘主流程验收点；成本低 |
| 未读数联动 | 不联动：不影响未读总数，计数只显示在视图上 | 未读数语义保持单一 |
| MCP | 不扩展 | 稍后读属个人视图，P1 再议 |
| 文档义务 | README 特性/快捷键 + spec.md 如实回填 | AGENTS.md 硬性义务 |

## 2. 改动面

| 文件 | 改动 |
| --- | --- |
| `crates/rustrss-core/src/store/schema.rs` | 新迁移：ALTER TABLE + 部分索引 |
| `crates/rustrss-core/src/store/mod.rs` | `set_read_later` + EntryQuery 支持 later 视图 + 单测 |
| `src-tauri/src/commands.rs` | `set_read_later` 命令 |
| `ui/app.js` | 阅读器按钮 / 列表条目按钮 / 稍后读视图 / `l` 快捷键 + 计数更新 |
| `ui/i18n.js` | 新文案 zh-CN/en 双份 key |
| `README.md` / `spec.md` | 特性/快捷键与验收项如实回填 |

## 3. 非目标

- 稍后读的 MCP 暴露。
- 稍后读分组/标签（仅单一标记）。
- 跨设备同步（v1 无同步）。

## 4. 风险

- 迁移必须走既有 MIGRATIONS 机制（user_version 递增、独立事务），旧库升级数据不丢——AC 有测试。
- 列表按钮与已有星标按钮的布局挤占：小屏宽度下注意换行/省略。

## 5. 评审记录

- Round 1 待审。
