# 每日日报（Daily Digest）· 初版设计

状态：**已被定稿取代**——oracle 两轮审查后整合为 `design.md`（本文件的 8 个开放问题与遗留偏差已全部裁定，其中「订阅源标签被偷换成文章标签」等十条修正见 design.md §3）。
作者：主 agent（glm-5.3-flash 会话）· 2026-10-03
需求原文：添加今日日报、昨日日报功能；可对内容使用 AI 总结展示；默认缓存报告；可强制重新生成；生成时检查上次生成时间（检查点），其后有更新内容则提示是否重新生成；重新生成考虑 cache 命中率；提供配置（如按标签多选过滤订阅源范围，或其它方案）。

---

## 1. 概念模型

- **日报（Digest）** = 某一自然日（本地时区）内、选定范围内文章的 AI 总结成品 + 元信息（生成时间、条目数、缓存命中数、检查点、范围参数）。
- UI 入口为「今日 / 昨日」，内部一律是**查看日期（date）**——午夜翻天后「今日」自然变成对昨天的补看，语义不漂移。
- **检查点（checkpoint）** = 该日报上次生成完成的时刻。自检查点之后**新抓取到**且发布时间落在该日的条目 = 「有更新」。

## 2. 与现有架构的贴合

- AI：完全复用 BYOK 通道（`AiClient` / `AiConfig`，四 provider），代理、思考强度、输出上限全部沿用现有设置。
- 缓存：**分层**，不新造缓存机制——
  - **L1 条目摘要**：复用现有 `ai_cache` 表，新任务名 `digest_item`。缓存键 `(entry_id, task, params, provider_model, prompt_version)` 中 params=长度/语言。**与日报日期、范围无关** → 同一条目跨日期、跨范围复用（今天生成过的条目摘要，明天的「上周日报」照样命中）。
  - **L2 日报成品**：新表 `digests`（见 §3），存最终合成文本 + checkpoint + 范围/参数 + 命中统计。
- 生成流程遵守仓库红线 12（锁内不 await）：沿用 `plan_task → complete → save_task_output` 的三阶段同构。

## 3. 数据模型

```sql
CREATE TABLE digests (
    id            INTEGER PRIMARY KEY,
    date          TEXT    NOT NULL,            -- 'YYYY-MM-DD'（本地时区）
    scope_key     TEXT    NOT NULL,            -- 'all' 或 'tags:1,3'（排序后的标签 id）
    params        TEXT    NOT NULL,            -- {length, language, model_tag} 摘要参数指纹
    content       TEXT    NOT NULL,            -- 最终日报（Markdown）
    checkpoint_at INTEGER NOT NULL,            -- 本次生成完成时刻（epoch 秒）
    entries_total INTEGER NOT NULL,            -- 范围内条目总数
    cache_hits    INTEGER NOT NULL,            -- L1 命中数（未重复花钱的条目摘要数）
    created_at    INTEGER NOT NULL,
    UNIQUE (date, scope_key, params)
);
```

- 一天 × 一个范围 × 一套参数 = 一份日报；改设置（换标签/换模型/换长度）自然产生新行，旧行保留（可回看历史）。
- **不存**逐条摘要到 digests——L1 里已经有；digests 只存成品。

## 4. 生成流程（两段式合成，cache 命中率的核心）

```
[用户点击 生成/重新生成]
 1. 取范围内条目：published_at ∈ [date 00:00, date+1d) （走 idx_entries_read_published
    或 (feed_id,published)；标签过滤 → JOIN feed_tags/entry_tags，范围查询先取候选集再过滤，
    单日候选集小，不为检查点新加索引）
 2. 逐条 L1 摘要：AiTask::DigestItem（新任务：要点提炼，params=长度+语言）
    ├─ UseCache 命中 → 直接用（cache_hits += 1）
    └─ 未命中 → 调模型 → 写入 L1
    ▶ 进度反馈：x/y（状态栏 + 日报头部进度条），支持取消
 3. 合成：AiTask::DigestCompose（输入 = 条目摘要列表 + 日期 + 条目元信息，
    缓存键 = 条目集合指纹（按 entry_id 排序拼接）+ checkpoint + 参数）
    └─ 输出写 L2（digests 表）
 4. 展示成品 + 元信息条（生成时间 / 共 N 篇 / L1 命中 M 篇 / 检查点）
```

- **强制重新生成**的两级语义：
  - 「有更新，重新生成」（检查点提示流，默认）：跳过 L2，**L1 照常命中**——只有新增/变化条目真正花钱，这就是「考虑 cache 命中率」；
  - 「深度重生成」（高级开关，默认关）：连 L1 也 `CachePolicy::Refresh`——条目摘要全部重跑（换模型后想要新摘要时用）。
- 条目**内容变化**（fulltext 重取）何时让 L1 失效：沿用 ai_cache 的 params 指纹机制不含内容哈希——初版**不做**内容哈希失效（ai_cache 现状对 summarize 也如此），列入开放问题。

## 5. 检查点与新内容提示

- 打开日报或返回视图时：`SELECT count(*) FROM entries WHERE published_at ∈ [date, date+1d) AND fetched_at > checkpoint_at AND (范围过滤)`。
  - 单日候选集小（先按 published 索引取当日集合，再过滤 fetched_at 与范围），不触碰正文大列，符合红线 1。
- `count > 0` → 日报头部出现提示条：「自上次生成后有 N 篇新内容 → [重新生成] [忽略]」。忽略是**会话内**的（不落库）；下次进入仍提示。
- 今日日报的特殊性：一天未结束，生成的是「阶段性日报」；再次进入提示有更新是常态而非异常。

## 6. 配置

存 settings 表（沿用 key-value）：

| key | 值 | 默认 |
| --- | --- | --- |
| `digest.scope_tags` | JSON 数组（标签 id）或 `[]`=全部 | `[]` |
| `digest.length` | short / medium / long | 跟随现有 summarize 默认 |
| `digest.language` | 字符串 | 跟随现有设置 |
| `digest.deep_refresh` | bool | false |

- 范围语义：`scope_tags` 非空 = **只统计带任一所选标签的条目**（entry_tags 已有该 join）；
  修改范围后，date 相同也生成新的 scope_key 行（不覆盖旧范围报告）。
- UI：设置 → 阅读（或新「日报」分区）：标签多选器（复用现有标签数据）+ 长度 + 深度重生成开关。AI 模型/语言不单独设，跟随 AI 面板既有设置。

## 7. UI

- 侧栏「智能视图」组下新增**「日报」**分组：`今日` `昨日` 两行（带状态点：无报告/有报告/有更新）。
- 点击 → 右栏阅读窗格展示日报（复用 `.article` 排版）+ 日报专属头部：
  - 日期、范围（标签 chips）、生成时间、条目数、L1 命中数；
  - 按钮：`重新生成`（检查点提示流）、`深度重生成`（菜单内）、`复制`；
  - 检查点提示条（§5）；
  - 生成中：进度条（x/y 条目摘要阶段 + 合成阶段）+ 取消。
- 空态：当日无文章 / 范围过滤后 0 条 / AI 未配置（引导按钮跳设置→AI）。

## 8. Tauri commands 与 MCP

- core：`store::digests` DAO + `ai::digest`（两段任务编排，进度经现有 status 通道回报）。
- commands：`digest_get(date, scope)`（读缓存+检查点状态）、`digest_generate(date, scope, deep)`（异步，可取消）、`digest_list()`（有报告的日期列表，供侧栏状态点）。
- MCP（分层纪律：与 command 共享 core 路径）：`digest_get` 工具，返回成品 + 元信息；受响应口径约束——日报本身就是摘要，但超长时仍需按既有截断口径截断并说明。
- Android：跟随现有响应式布局（日报入口在列表页头或设置？初版桌面优先，Android 复用同一 command 面）。

## 9. 迁移与兼容

- schema 迁移：新表 digests（现有迁移框架追加）；ai_cache 无改动。
- 老库升级无损；卸载语义不变。

## 10. 开放问题（请 oracle 重点完善）

1. 合成阶段的上下文管理：条目摘要很多（100+ 条/日）时如何分批（map-reduce 式二级合成？）与 token 预算控制。
2. 条目内容变化（fulltext 重取/正文更新）导致 L1 过期——是否值得引入 content_hash 失效？成本与收益。
3. 自动生成（如启动时后台生成昨日日报）是否进初版，还是纯手动 + 检查点提示？
4. 日报成品的导出（Markdown/剪贴板已有；是否要文件导出）。
5. 时区边界：跨时区旅行/系统时区变更后，历史日报 date 归属是否需要处理。
6. 多范围报告的 UI 收纳：scope_key 维度的历史报告如何浏览（初版只展示当前配置范围的报告？）。
7. MCP 工具口径细化：digest_get 的截断与分页。
8. 检查点提示的「忽略」是否需要持久化（当前设计为会话内）。
