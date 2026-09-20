---
title: PRD: 正文代码块语法高亮
proposalUuid: 09608a57-9084-49f6-bc6e-0f68f45fc40b
documentUuid: e37f8795-76aa-4108-88b8-5e638b48c6d5
---

# 代码块语法高亮 — 需求与取舍

> 状态：待评审。需求基线见 `.chorus/specs/rss-reader/spec.md` 正文渲染验收点「代码块有语法高亮」（本文件只承载分析与取舍，不重复验收条目）。

## 1. 背景与问题

正文白名单渲染已保留 `pre/code`（`ui/app.js` sanitize），但无语法高亮——正文渲染验收点中唯一缺口。

## 2. 已定决策（elaboration round 1，2026-09-20）

| 决策点 | 选择 | 理由 |
| --- | --- | --- |
| 选型 | vendor highlight.js 单文件到 `ui/vendor/`（BSD-3-Clause，保留原 license 头） | AGENTS.md 约束 4 无构建链；CDN 违背本地优先且离线不可用；自写正则覆盖差 |
| 语言检测 | `code` 的 `language-*` class 优先，无 class 则 auto-detect，失败原样显示 | feed 正文常无 class；auto-detect 失败无害 |
| 触发时机 | 仅打开的文章正文渲染时 | 开销只在阅读时付出；列表摘要不渲染 code 块 |
| 配色 | 自定义 token 色值挂入现有 CSS 变量体系，适配应用深浅双主题（与功能②的 data-theme 兼容） | 官方单主题与界面割裂 |
| 安全边界 | sanitize 产物为输入；为 class 检测放行 `pre`/`code` 的 `class` 属性（值仅限 `language-*` 前缀）；高亮输出不回灌 sanitize | 绝不二次接触原始 HTML；限前缀防任意类名（Round 1 BLOCKER 修复） |
| 文档义务 | README 特性清单 + spec.md 正文渲染验收项处理（勾选或注明） | AGENTS.md 硬性义务 |

## 3. 改动面

| 文件 | 改动 |
| --- | --- |
| `ui/vendor/highlight.min.js`（新增） | hljs 核心（常用语言子集，控制体积） |
| `ui/app.js` | sanitize 白名单放行 `pre`/`code` 的 `class`（仅 `language-*` 前缀）；渲染后对 `pre code` 调 hljs（class 优先 / auto 兜底） |
| `ui/style.css` | 高亮 token 配色（深浅两套，接 CSS 变量/data-theme） |
| `ui/index.html` | 引入 vendor 脚本（本地相对路径） |
| `README.md` / `spec.md` | 特性说明与验收项回填 |

## 4. 非目标

- 不做编辑器级高亮（嵌套结构/超长行的性能极限优化）。
- 不支持含 HTML 的 code 块内二次渲染（sanitize 后是纯文本）。
- 不加语言选择器/手动指定 UI。

## 5. 风险

- hljs auto-detect 对短代码块误判：表现为错误配色，无害；class 优先策略已覆盖常见 feed。
- vendor 文件体积（核心+常用语言约 100-150KB 本地文件，无网络成本）。
- 与功能②的 data-theme 切换叠加：token 色需覆盖跟随系统态（`html:not([data-theme])`）与固定态（Round 1 NOTE）
- sanitize 放行 class 的额外风险面：仅 `language-*` 前缀，未知语言 hljs 按纯文本处理；AC 要求测试覆盖

## 6. 评审记录

- Round 1（2026-09-20）：FAIL，1 BLOCKER——ALLOWED_TAGS 中 pre/code 空属性白名单剥掉 class，class 优先检测死路；采用评审建议 A：白名单放行 class（限 language-* 前缀）+ 测试。2 NOTE 已吸收（token 色覆盖跟随系统态；CSP null 不阻塞已确认）。Round 2 待审。
