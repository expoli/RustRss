# PRD: RSSHub 订阅抓取时解析

- 模块: rss-reader / rsshub-fetch-resolve
- 日期: 2026-09-22
- 来源: Chorus Idea f70f3241（用户设计纠偏）

## 问题

当前 `add_feed` 落库时把 `rsshub://path` 实例化为 `{镜像}/path`——换镜像后所有存量 URL 指向旧实例，需要手动「迁移」重写。存储应是抽象身份，不应绑定具体实例。

## 已定决策（elaboration 确认）

| # | 决策 |
|---|------|
| D1 | **抓取时解析**：库里永远存 `rsshub://path`；`feed_endpoint`（唯一抓取出口）解析到当前镜像；换镜像零迁移即时生效 |
| D2 | **官方域也转 scheme**：add_feed 把 `https://rsshub.app/path` 转成 `rsshub://path`（同 route 两形态判重）；迁移按钮退役为一次性「存量官方域归一为 scheme」整理 |
| D3 | **显示 scheme 形态**：编辑对话框/tooltip/MCP 显示库里真实存的 `rsshub://path`（如实且 OPML 可移植） |

## 需求

1. `add_feed` 归一化语义变更：scheme 归一（rsshub:// 保留 + 三斜杠归一；官方域 → scheme；其它原样），不再实例化
2. `feed_endpoint` 解析：`rsshub://path` → `{当前 mirror}/path`（mirror 空 = 官方默认）
3. `migrate_rsshub_feeds` 语义改为：存量官方域行 → `rsshub://path`（一次性整理；已是 scheme 的不动）
4. OPML 导出 = scheme 形态（天然可移植）；UI 显示口径同步
5. 测试更新：add 不实例化、两形态判重、endpoint 随镜像变化、迁移新语义、OPML 回环

## 验收标准

1. 新加 `rsshub://test/1` → 库里 url 原样 scheme；feed_endpoint 返回 `{mirror}/test/1`；改镜像设置后下一次刷新走新镜像（零迁移）
2. 官方域 https 添加 → 库里转成 `rsshub://path`；与先加的 scheme 形态判重不重复
3. 存量官方域行跑迁移 → 变 scheme；已是 scheme 的行不受影响
4. cargo test --workspace 全绿；README/spec.md 同步
