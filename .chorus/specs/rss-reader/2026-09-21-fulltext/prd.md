# PRD: 全文获取（摘要型 feed 抓原文提取正文）

- 模块: rss-reader / fulltext
- 日期: 2026-09-21
- 来源: Chorus Idea 1d4cda6d（对标差距分析 P0-4）

## 问题

部分 RSS 源只输出摘要，阅读器直接展示 summary，正文需跳出系统浏览器——阅读体验断裂。

## 需求（已 elaboration 确认）

| # | 需求 | 决策 |
|---|------|------|
| R1 | 提取实现 | 成熟 Rust readability 类 crate（实现时评估 crates.io 健康度/最近发布/体积，劣选 dom-based 备选）；不自研启发式 |
| R2 | 触发 | 阅读界面手动「获取全文」按钮，仅摘要型条目（content_html 缺失或明显偏短）显示；点击转 loading，成功重渲染 |
| R3 | 缓存 | 持久化写回：entries 新增 `fulltext_fetched` 标记位（迁移 v7）；写回 content_html + content_text + search_tokens；后续刷新 upsert 不覆盖已抓正文（只更新元数据）；重开零网络 |
| R4 | 网络与安全 | 复用现有 Fetcher（rustls/webpki-roots/UA/超时一致）；响应体上限 2MB 超限拒绝；失败/超时/非 HTML 降级显示原摘要 + 状态栏错误；提取结果进现有 sanitize 管线 |

## 验收标准

1. 摘要型条目显示「获取全文」按钮；点击后正文替换为提取结果并重渲染；全文型条目不显示按钮。
2. 抓取结果持久化：同一篇文章第二次打开不再发网络请求（直接读库）；之后的 feed 刷新不把已抓正文覆盖回摘要（content_hash 变化只更新元数据）。
3. 失败路径：非 HTML / 超时 / 超 2MB 均给出状态栏错误且原文摘要不受影响。
4. core 层提取与写回逻辑带测试（真实 HTML 样本 fixture：典型文章页/非 HTML/空 body）；迁移 v7 幂等（v6→v7 走真文件测试模式）。
5. `cargo test --workspace` 全绿；i18n 双语 key；README 同步。
