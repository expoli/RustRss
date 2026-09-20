---
title: PRD: 深浅色主题手动固定（三态切换 + 跨会话保持）
proposalUuid: 15bcf50e-2c1f-4a7d-b241-da7b053b9e82
documentUuid: 33b808ae-4979-49c0-bef6-6351f4f636d9
---

# 主题手动固定 — 需求与取舍

> 状态：待评审。需求基线见 `.chorus/specs/rss-reader/spec.md`「外观」节验收点「深浅色主题跟随系统，也可手动固定；选择跨会话保持」（本文件只承载本次 change 的分析与取舍，不重复验收条目）。

## 1. 背景与问题

当前主题只跟随系统（`ui/style.css:16` `@media (prefers-color-scheme: light)`），用户无法手动固定深/浅色。spec 验收点要求手动固定 + 跨会话保持，是「主题」验收项的唯一缺口。

## 2. 方案概述（elaboration round 1 已定决策，2026-09-20）

| 决策点 | 选择 | 理由 |
| --- | --- | --- |
| 选项形态 | 三态：跟随系统 / 浅色 / 深色，默认跟随系统 | spec 原文「跟随系统，也可手动固定」即三态语义 |
| 持久化 | 复用 SQLite `settings` 表，`get_ui_settings` 扩展 `theme` 字段（string，对齐 `locale` 先例） | 跨会话保持天然满足；与「权威值在 Rust 侧」现状架构一致；不用 localStorage |
| CSS 实现 | 重构为 CSS 变量 + `html[data-theme]` 属性覆盖 `prefers-color-scheme` | 单文件内三态统一；避免两套样式表漂移；具体变量命名由实现者定 |
| 入口 | 设置对话框 General 分区（与 locale 同区） | 界面克制（spec 主张），不加工具栏按钮 |
| 实时性 | 「跟随系统」态监听 `matchMedia('...').addEventListener('change')` 实时切换 | 跟随系统不实时切会显得「假跟随」 |
| 文档义务 | README 设置说明同步 + spec.md 主题验收项勾选 | AGENTS.md 硬性约定 |

## 3. 改动面

| 文件 | 改动 |
| --- | --- |
| `crates/rustrss-core/src/store/` | settings 读写已有基础设施，确认 string 键存取可用（可能零改动） |
| `src-tauri/src/{commands.rs,ai.rs 或 state}` | `UiSettings` 加 `theme` 字段 + `set_ui_setting` 类命令（对齐现有 `mark_read_on_navigate`/`locale` 模式） |
| `ui/style.css` | 颜色值收编为 CSS 变量；`html[data-theme="light"|"dark"]` 覆盖 + 默认跟随系统 |
| `ui/app.js` | 三态选择渲染/持久化调用 + matchMedia 监听 + data-theme 应用 |
| `ui/i18n.js` | 新文案 zh-CN/en 双份 key + key-set 自检通过 |
| `ui/index.html` | General 分区加选择控件 |

## 4. 非目标

- 不做自定义主题色/字号缩放联动（字号是另一验收项，另行处理）。
- 不做工具栏快捷切换按钮。
- 不为高亮代码块定制 token 配色（功能③范围）。

## 5. 风险与边界

- CSS 变量重构涉及 style.css 全量颜色（458 行），需逐处核对不漏；验证手段：深/浅/跟随三态下逐屏人工过一遍 + 现有 i18n/测试全绿。
- `data-theme` 与 `prefers-color-scheme` 的层叠顺序要保证「跟随系统」态下系统变化仍生效（用 JS 监听写属性，而非 CSS 层叠技巧，行为最直白）。
- 评审前车之鉴：README/spec 同步义务必须进 AC；i18n 双 key 必须进 AC。

## 6. 评审记录

- Round 1 待审。
