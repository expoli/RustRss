---
title: PRD: RSSHub 自定义镜像与 rsshub:// 实例化
proposalUuid: 295ad89b-7fb1-446f-b8ee-48961b0121a2
documentUuid: c637f820-bff7-4404-b3ab-8d473b9a3146
---

# RSSHub 自定义镜像与 rsshub:// 实例化 — 需求与取舍

> 状态：待评审（Round 2）。对标 folo/fluent-reader 等的 RSSHub 实例配置能力。**v0→v1 变更**：真实 OPML 导入发现大量 `rsshub://path` 自定义 scheme 订阅（30 条抓取全部 builder error），`rsshub://` 必须先实例化为 `https://{base}/{path}` 才能抓取——这使「默认 base」从"空=不映射"改为"默认 `https://rsshub.app`"。

## 1. 已定决策（elaboration round 1 + 实测变更 round 2，2026-09-21）

| 决策点 | 选择 | 理由 |
| --- | --- | --- |
| RSSHub 实例 base | 设置键 `rsshub.mirror`，**默认 `https://rsshub.app`** | `rsshub://` 无实例即不可用，必须有默认；自建/镜像用户改此值 |
| 改写范围（三种形态） | ① `rsshub://path` → `{base}/{path}`（含 `rsshub:///path` 三斜杠）② `https://rsshub.app/path` → `{base}/path`（base 非官方时）③ 其它 URL 原样 | 覆盖 OPML 实测的两种来源 + 官方 https 形态 |
| 生效时机（收口点） | **feed URL 落库的单一收口**：add_feed / discover 后落库 / OPML 导入三入口统一改写 | 库内 URL = 实际抓取 URL，抓取/重试/tooltip 自然一致 |
| 批量迁移 | 设置页「迁移现有订阅」：预览命中数（`rsshub://` + `rsshub.app` 两种存量）→ 确认 → 执行并反馈条数；已迁移的跳过（幂等） | 存量 30 条失败 feed 靠此修复 |
| 校验 | 保存时 URL 形态校验（http/s、无路径尾斜杠）+ 「测试连接」拉 `{base}/feed/rsshub/rss` | 配置即得确定性反馈 |
| 文档义务 | README（RSSHub 实例配置 + 用法）+ spec.md 如实回填 | AGENTS.md 义务 |

## 2. 改动面

| 文件 | 改动 |
| --- | --- |
| `crates/rustrss-core/src/` | `normalize_rsshub_url(url, base) -> String` 纯逻辑（三种形态）+ 单测 |
| `crates/rustrss-core/src/opml.rs`（或 store 落库收口） | 导入路径应用改写 |
| `src-tauri/src/commands.rs` | `get/set_rsshub_mirror`（形态校验）、`test_rsshub_mirror`、`migrate_rsshub_feeds`；add_feed/discover_feed 调用改写 |
| `ui/app.js` / `ui/i18n.js` / `ui/index.html` | 「RSSHub」设置分区（实例 base + 测试连接 + 迁移按钮与反馈）|
| `README.md` / `spec.md` | 设置说明与验收项回填 |

## 3. 语义细则

- 归一化：`rsshub://`/`rsshub:///` 的 path 去多余前导斜杠后拼 `{base}/{path}`；`https://rsshub.app/{path}` 仅在 base ≠ 官方域时替换；query 原样保留。
- 收口点选择：`add_feed`（store）与 OPML 导入的 upsert 前各调一次 `normalize_rsshub_url`（base 从 settings 读取）；discover 产出的 feed URL 最终也走 add_feed，天然覆盖。
- 幂等：已带非官方 base 的 URL 再跑迁移不变（host 判定）。

## 4. 非目标

- 多实例列表与切换。
- 非 RSSHub 的通用 scheme/域名改写。
- 已有订阅的自动无感迁移——用户显式点击迁移按钮。

## 5. 风险

- 镜像/自建实例的路由覆盖差异：抓取失败走既有 feed 失败标记（非本功能引入）。
- 用户自建实例未配置时，`rsshub://` 订阅默认走官方实例——可能仍被限流，但至少 URL 形态合法、失败原因可读。

## 6. 评审记录

- Round 1：评审进行中时需求变更（用户实测 rsshub:// scheme 30 条失败），主动撤回修订后以 Round 2 重审。
- Round 2 待审。
