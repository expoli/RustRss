---
title: PRD: Feed 自动发现（粘贴网站首页 URL 即可订阅）
proposalUuid: aa12f34b-4c97-48cd-b934-2aa42e4d8e62
documentUuid: eb8634cf-f765-46be-b501-767cfdc78993
---

# Feed 自动发现 — 需求与取舍

> 状态：待评审。需求基线见 `.chorus/specs/rss-reader/spec.md`「订阅与抓取」节（本文件只承载本次 change 的分析与取舍，不重复验收条目）。

## 1. 背景与问题

当前 RustRss 添加订阅只接受直接的 feed URL（`/feed`、`atom.xml` 等）。用户手头通常只有网站首页地址：粘贴首页会当作 feed 抓取并解析失败，必须自己翻网页源码找 feed 地址，首次订阅体验断裂。这是 spec 验收项「输入网站首页 URL 时能自动发现 feed（解析 `<link rel=alternate>`）；输入 feed URL 直接订阅成功」的唯一未实现部分。

## 2. 方案概述

- `rustrss-core` 新增 discovery 模块：抓取输入 URL → 判断返回内容是 feed（RSS/Atom/JSON Feed 根元素）还是 HTML 页面；HTML 则解析 `<head>` 中 `<link rel="alternate" type="application/rss+xml|atom+xml|feed+json">` 的 `href`，相对路径按页面 URL 解析为绝对地址。
- `src-tauri` 增加对应 command；UI 添加订阅流程先走发现、再落现有订阅路径。
- 复用现有 reqwest（rustls + webpki-roots）fetch 路径与超时策略（fetch.rs 现无响应大小上限，不虚构「大小策略」）。

## 3. 已定决策（elaboration round 1，2026-09-20）

| 决策点 | 选择 | 理由 |
| --- | --- | --- |
| 多候选 feed | 自动取第一个主 feed，其余记日志 | Simplicity First：多数站点 head 只有一个主 feed；v1 不加候选选择 UI |
| 发现失败行为 | 明确报错，保留原始原因 | spec 只要求 link 解析；路径猜测（/feed、/rss.xml）会对不存在路径发额外请求，v1 不做 |
| 逻辑分层 | core 纯逻辑模块，wiremock 可测 | AGENTS.md 硬约束 1：业务逻辑进 core，src-tauri 只做胶水 |
| 抓取限制 | 复用现有 fetch 超时策略 | 避免两套行为 |
| feed 类型 | rss+xml、atom+xml、feed+json 三种都认 | 解析器已支持 JSON Feed，发现侧对齐成本极低 |

## 4. 非目标

- 不做 feed 地址路径猜测（`/feed`、`/rss.xml` 探测）。
- 不做多候选选择 UI。
- 不处理需要 JS 渲染才输出 link 标签的 SPA 站点（head 静态解析不到就报错）。

## 5. 风险与文档同步义务

- 部分站点 `<link>` 的 `type` 缺失或写错：按 `rel=alternate` + href 后缀宽松匹配作为兜底（记录日志）。
- 相对 `href` 解析必须与 `ui/app.js` sanitize 里的 `new URL(src, baseUrl)` 语义一致。
- 按 AGENTS.md 约定：落地后必须同步 README.md「添加订阅」描述，并即时勾选 spec.md 对应验收项——已入任务 2 验收项。

## 5a. 评审记录

- Round 1（2026-09-20）：FAIL，2 BLOCKER——T2 AC2 与发现优先流程自相矛盾（改为结果导向）、缺文档同步 AC（已补）；2 NOTE——兜底 AC 缺失（已补）、「大小策略」措辞不实（已修正）。Round 2 待审。

## 6. 改动面

| 文件 | 改动 |
| --- | --- |
| `crates/rustrss-core/src/discover.rs`（新增） | 发现纯逻辑 + 单元/wiremock 测试 |
| `crates/rustrss-core/src/lib.rs` | 导出模块 |
| `src-tauri/src/commands.rs` | 新增 discover command |
| `ui/app.js` / `ui/i18n.js` | 添加订阅先发现再订阅；失败文案（zh-CN/en 双份 key） |
