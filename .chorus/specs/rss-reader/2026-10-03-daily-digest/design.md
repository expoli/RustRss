# RustRss 每日日报（Daily Digest）· 设计定稿

状态：**设计完成，待实施**。本文由初版草案（主 agent）经 oracle（gpt-6-astra）两轮审查修正后整合定稿；
oracle 的十条修正裁定与开放问题决策全部采纳。本文只含设计，不含代码改动。
日期：2026-10-03 · 关联草案：`design-draft.md`（保留作过程记录）

核心方案一句话：
> 按日期与订阅源标签选取文章，**冻结本次输入快照**，复用内容寻址的文章要点缓存，再分层合成为日报。
> 报告默认读取本地缓存；数据变化只提示、不自动调用 AI；重新生成失败或取消时，上一份成功报告始终保留。

---

## 1. 目标与非目标

**目标**：今日 / 昨日日报入口；对范围内本地已抓取文章做 AI 总结并分层合成为日报；默认读缓存；
检查点（冻结素材清单）对比出「新增 / 变化 / 移出」后提示是否更新；重新生成最大化复用缓存以控制
BYOK 成本；支持按订阅源标签多选限定范围。

**非目标（初版明确不做）**：
- 自动 / 定时生成（进入页面、刷新完成、回到前台只做**本地**状态检查，绝不自动调 AI）；
- 文章级标签过滤（源级标签之外的第二个过滤维度）；
- 「忽略全部缓存重跑所有正文」的持久设置（后续可作为一次性高级动作，需明示成本）；
- 文件导出（初版只做复制 Markdown；core 提供确定性导出纯函数，文件保存后续接系统文档通道）；
- 声称覆盖互联网上该日的全部内容——日报只基于**本地已抓取**内容，UI 需展示最近刷新时间并提供
  独立的「先刷新订阅」动作，不得把「报告已生成」表述为「已获取全部最新内容」。

## 2. 继承决定（口径定论）

1. 新增 `feed_tags`（订阅源级标签），复用 `tags` 字典；与 `entry_tags` 独立存储，但文章有效标签 = 手动 ∪ 源继承（按 tag_id 去重，显示/筛选/计数/标签批量统一）。2026-10-04 用户批准 B′ 修订；取消手动关联不屏蔽继承。
2. 日报范围：选中多个源标签时按 **OR** 匹配订阅源，纳入这些源在目标日期内的**全部文章**；
   不叠加文章级标签过滤。未选标签 = 全部订阅源。
3. 设置中回显命中订阅源数量；侧栏标签未读计数按有效标签集合去重统计（2026-10-04 用户批准修订），日报选源仍只用 feed_tags 的 OR 范围。
4. 复用标签选择器组件：桌面订阅源右键菜单、Android 操作面板均提供打标入口；初版不做独立标签管理页。
5. BYOK 通道、代理、凭据存储全部复用；读取报告与检查更新**零 AI 请求**；生成必须用户触发。
6. 分层纪律：日报业务、范围解析、缓存键、预算、状态机、存储进 `rustrss-core`；Tauri 只做凭据、
   命令、事件桥接；前端原生 JS，双语 zh-CN/en，保持 keyed reconcile / 同值短路 / 阅读位置。
7. MCP 初版只读日报（列表+单取），不新增产生费用的生成工具。

## 3. 十条修正裁定（oracle 审查结论，实现时作为前提）

| # | 裁定 | 依据 |
|---|---|---|
| 1 | 订阅源标签不可替换为文章标签 | 现 schema 只有 `entry_tags`（`schema.rs:202` 起）；用户原义是「带这些标签的订阅源」 |
| 2 | 检查点不能用生成完成时间 | 10:00 冻结素材、10:02 抓到新文章、10:05 完成——用完成时间查会永久漏掉 10:02 这篇。保存**冻结的素材清单**，完成时间仅展示 |
| 3 | `fetched_at` 不是可靠内容版本 | `upsert_entries` 更新内容时改它，但 `set_fulltext` 不改它也不改 `content_hash`（`store/mod.rs:965-1082`）；同秒更新、时钟回拨亦不可靠 |
| 4 | L1 缓存必须按实际输入哈希失效 | 现有 `cache_params()` 只有长度/语言，`cache_tag()` 只有 provider/model（`prompt.rs:70-76`、`ai/mod.rs:112-114`）；正文变化不会自动失效缓存 |
| 5 | 状态检查不能依赖草案的 COUNT 查询 | oracle 实测：无 feed 条件 = `SCAN entries`；带 feed_id 也非覆盖。「单日候选少」不能替代覆盖索引契约 |
| 6 | 日报素材查询不得复用 `list_entries` | 后者继承隐藏已读/排序/搜索/分页等全局列表设置；日报素材独立于已读、星标与列表状态 |
| 7 | 合成任务不能套用单文章 `plan_task` | `plan_task` 必须有 `entry_id` 且 `ai_cache` 外键指向文章；合成不是一篇文章 → 独立编排 + 节点缓存，不伪造文章 ID |
| 8 | 合成缓存键不得包含生成时间 | 时间每次变化必然降低命中率；也不可只按文章 ID（会错误命中过期结果）→ 按**规范化输入 + 子节点输出哈希** |
| 9 | 迁移不是「追加一张表」 | `schema::detect` 只接受 `BASELINE_VERSION == 1`（`schema.rs:47-64`）；必须同步升级 detect/迁移/备份校验与测试，否则升级库再打开被拒（`store/mod.rs:469-506`、`backup.rs:181-241`） |
| 10 | 复用 `AiClient` 有两个前置修正 | 构造器未显式 `tls_certs_only(webpki_root_certs())`（Android TLS 红线）；响应整包 `.text()` 无大小闸门（获取中限额红线） |

## 4. 产品语义

### 4.1 日期归属
```
有效时间 effective_at = published_at（存在时）
                     ≜ first_seen_at（无发布时间时的首次入库时间）
某日日报范围 = [本地 day_start_utc, 次日 day_start_utc)   ← 半开区间
```
- 日界由 core 按当地自然日计算（DST 可产生 23/25 小时一天），**禁止 `start + 86400`**。
- 昨天发布、今天抓到的文章 → 属于昨日日报的**新增素材**；昨天的文章今天修订 → 昨日报告提示变化；
  更早日期的文章变化不算今日素材。
- 已读/星标/稍后读/文章标签变化**不影响**日报素材；不自动抓全文。
- 存量文章无真实首次入库时间：迁移以 `fetched_at` 回填并标记 `first_seen_estimated=1`（近估计值，
  在迁移记录中明示）；新文章从此准确记录。

### 4.2 检查点 = 冻结的素材清单（不是时间点）
每份成功报告保存：
- `checkpoint_at`：输入清单**冻结**时刻（快照时间）；
- `generated_at`：报告成功提交时刻（仅展示）；
- `manifest_hash`：该次素材清单指纹；
- `digest_items`：逐篇素材的实例身份、`source_revision`、实际输入哈希。

UI 文案区分两者：
> 内容快照：10:00 · 生成完成：10:04

例：10:00 冻结 100 篇 → 10:02 抓到第 101 篇 → 10:04 完成。完成后立即提示「有 1 篇新增」——
10:04 不会覆盖素材检查点。

### 4.3 更新提示的类型化
状态接口返回：`added_count / changed_count / removed_count / scope_invalid / configuration_changed`。
「移出」包括：文章删除、取消订阅、源标签关联变化导致离开范围、发布时间修订导致离开该日。
提示文案按类型组合：
> 自上次素材快照后，新增 8 篇、内容变化 2 篇、移出 1 篇。是否更新日报？

### 4.4 三种用户行为
| 行为 | 语义 | 是否可能收费 |
|---|---|---|
| 查看报告 | 读最后完整缓存 + 更新状态 | 否 |
| 更新日报 | 按当前素材重新计划，复用全部有效缓存；输入完全未变时可零请求 | 可能 |
| 强制重写报告 | 复用文章要点与归并节点，**绕过最终合成缓存**，至少一次合成请求 | 是（空素材除外） |

不提供持久化「深度重生成」开关（避免忘记关闭后每天重复付费）；「全部文章重新分析」留作后续一次性
高级动作。换模型/换端点/换要点提示词本身已自然导致相应缓存失效。

### 4.5 今日与昨日
「今日」是截至素材快照的阶段性报告；「昨日」仍可能因晚到文章、全文补抓、源修正而更新。
阅读中的报告绑定具体日期与边界，跨午夜不自动切换正文；「今日/昨日」入口在再次选择时解析成具体日期。

## 5. 数据模型

### 5.1 模型分工
| 存储 | 内容 |
|---|---|
| `feed_tags` | 订阅源↔标签关联 |
| `digest_entry_meta` | 窄文章元数据：稳定实例身份、版本、日期归属（状态检查走它，不碰正文表） |
| `digests` | 报告头：检查点、统计、当前完整版本指针 |
| `digest_bodies` | 报告结构化正文 + 导出 Markdown（单取接口才读） |
| `digest_items` | 当前完整报告的素材清单快照（**故意不外键**关联 entries/feeds——文章删除后历史报告仍可读；instance_id 防 SQLite rowid 复用串扰） |
| 现有 `ai_cache` | 条目级要点，任务名 `digest_item` |
| `digest_node_cache` | 归并树/合成的中间节点缓存（内容寻址） |

### 5.2 DDL（作为新增迁移；不改已发布基线 SQL）
```sql
-- 订阅源级标签
CREATE TABLE feed_tags (
    feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tags(id)  ON DELETE CASCADE,
    PRIMARY KEY (feed_id, tag_id)
);
CREATE INDEX idx_feed_tags_tag ON feed_tags(tag_id, feed_id);

-- 窄文章元数据投影（状态检查的覆盖索引路径）
CREATE TABLE digest_entry_meta (
    instance_id          INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id             INTEGER NOT NULL UNIQUE REFERENCES entries(id) ON DELETE CASCADE,
    feed_id              INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    first_seen_at        INTEGER NOT NULL,
    first_seen_estimated INTEGER NOT NULL DEFAULT 0 CHECK (first_seen_estimated IN (0,1)),
    effective_at         INTEGER NOT NULL,
    source_revision      INTEGER NOT NULL DEFAULT 1 CHECK (source_revision >= 1)
);
CREATE INDEX idx_digest_meta_day ON digest_entry_meta(
    effective_at, feed_id, entry_id, instance_id, source_revision);
CREATE INDEX idx_digest_meta_feed_day ON digest_entry_meta(
    feed_id, effective_at, entry_id, instance_id, source_revision);
-- 存量回填：first_seen 用 fetched_at 近似（标记 estimated），effective_at = COALESCE(published_at, fetched_at)

-- 报告头（revision=0 表示有生成槽位但尚无成功报告；成功才替换）
CREATE TABLE digests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    report_day TEXT NOT NULL,
    timezone_label TEXT NOT NULL,
    day_start_at INTEGER NOT NULL, day_end_at INTEGER NOT NULL,
    utc_offset_start INTEGER NOT NULL, utc_offset_end INTEGER NOT NULL,
    date_basis TEXT NOT NULL DEFAULT 'published_or_first_seen_v1',
    scope_key TEXT NOT NULL, scope_json TEXT NOT NULL,
    profile_key TEXT NOT NULL, profile_json TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    active_job_id INTEGER,
    checkpoint_at INTEGER, generated_at INTEGER, manifest_hash TEXT,
    article_count INTEGER NOT NULL DEFAULT 0,
    cited_count INTEGER NOT NULL DEFAULT 0,
    truncated_count INTEGER NOT NULL DEFAULT 0,
    summary_only_count INTEGER NOT NULL DEFAULT 0,
    stats_json TEXT NOT NULL DEFAULT '{}',
    summary TEXT NOT NULL DEFAULT '' CHECK (length(summary) <= 140),
    stored_bytes INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    CHECK (day_start_at < day_end_at),
    UNIQUE (report_day, day_start_at, day_end_at, date_basis, scope_key, profile_key)
);
CREATE INDEX idx_digests_history ON digests(report_day DESC, generated_at DESC, id DESC);
CREATE INDEX idx_digests_scope_history ON digests(scope_key, report_day DESC, generated_at DESC, id DESC);

-- 报告正文（列表/计数不读它）
CREATE TABLE digest_bodies (
    digest_id INTEGER PRIMARY KEY REFERENCES digests(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    schema_ver INTEGER NOT NULL,
    content_json TEXT NOT NULL,
    markdown TEXT NOT NULL
);

-- 素材清单快照（无外键，历史可读）
CREATE TABLE digest_items (
    digest_id INTEGER NOT NULL REFERENCES digests(id) ON DELETE CASCADE,
    instance_id INTEGER NOT NULL, entry_id INTEGER NOT NULL, feed_id INTEGER NOT NULL,
    source_revision INTEGER NOT NULL, effective_at INTEGER NOT NULL,
    input_hash TEXT NOT NULL,
    truncated INTEGER NOT NULL CHECK (truncated IN (0,1)),
    summary_only INTEGER NOT NULL CHECK (summary_only IN (0,1))
    /* …其余清单列见 digest_items 完整定义 */
);

-- 中间归并/合成节点缓存（内容寻址；合成阶段复用的关键）
CREATE TABLE digest_node_cache (
    node_key TEXT PRIMARY KEY,        -- hash(阶段 || 规范化输入 || 子节点键集合)
    kind TEXT NOT NULL,               -- 'group' | 'final'
    content TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
```

### 5.3 schema 迁移路径（红线 9 的专项）
- **基线常量保持 1（不可改写）**；当前支持版本 = `MIGRATIONS.len()`（现为 2）。`detect()` 接受 1（触发迁移）与 2（就绪）；
- 迁移 1→2 = 新建上述表 + `digest_entry_meta` 回填（一条 `INSERT … SELECT`，不重算正文哈希）；
- 备份/恢复校验同步接受 1 与 2（恢复旧版本库后允许再次迁移）；
- 相关测试：旧库打开→迁移→再打开；备份 1→恢复→迁移→打开；`#[ignore]` 外的真实库冒烟。

## 6. 生成流水线与状态机

### 6.1 三段流水线（预算有界，绝不静默丢文章）
```
Planning  冻结素材清单（manifest：instance_id 集合 + 各篇 input_hash + scope/profile）
          manifest_hash = 清单指纹；写 digests 槽位（revision=0, active_job_id）
Running   ① 条目要点：逐篇 digest_item（ai_cache，键含实际输入哈希）
          ② 分组合成：要点按组合成 → 中间节点（digest_node_cache 内容寻址复用）
          ③ 最终合成：归并树根 → 日报成品（markdown + content_json）
          每次请求前验证实际输入大小；超预算 → 先缩小范围/分组，明确告知，不静默截断丢文章
Committing CAS 提交：比较提交时 manifest_hash 与计划时一致；不一致 → 保留旧报告，
          状态回到「有更新，是否再次更新」
```
- 进度经现有事件通道回报（要点 x/y、归并 n/m、合成中）；支持取消。
- 单 flight：同 (date, scope, profile) 并发重入 → 返回进行中的 job（红线 13 的 CAS + Drop guard 模式）。
- 取消/失败：保留已完成阶段的缓存节点；**旧报告不被覆盖**；状态如实标注（非成功）。
- 「更新日报」在素材与参数完全未变时：零 AI 请求直接复用（`no-change` 结果也如实提示）。

### 6.2 缓存键规则
- 条目要点（ai_cache）：`entry_id + task('digest_item') + params(长度,语言) + provider_model + prompt_version`
  **+ params 内含实际输入哈希与端点身份**（修正 #4/#10）。
- 归并/合成节点：`node_key = hash(阶段 || 规范化输入 || 子节点键集合)`；不含生成时间（修正 #8）。
- 合成缓存命中与费用交代：头部展示「文章要点复用 x/y · 归并节点复用 n/m · 本次请求 k 次」。

### 6.3 配置面
| key | 说明 | 默认 |
|---|---|---|
| `digest.scope_tags` | 源标签 id 数组（JSON）；空 = 全部订阅源 | `[]` |
| `digest.length` / `digest.language` | 要点与日报语言 | 跟随现有 AI 设置 |
设置 UI：标签多选器 + 命中订阅源数回显 + 长度/语言。修改范围/参数 → profile_key 变化 → 生成新的
报告变体（历史保留，见 §7 历史入口）。

## 7. UI

- 侧栏「智能视图」下新增**日报**分组：`今日` `昨日` 两行 + 状态点（无报告 / 有报告 / 有更新 / 生成中）。
- 点击 → 阅读窗格显示日报（复用 `.article` 排版）+ 日报头部：
  - 日期、范围 chips、**内容快照时间、生成完成时间**、条目数、复用统计；
  - 操作：`更新日报`（检查点提示流）、`强制重写`（菜单内，按钮文案明示「复用文章要点，重新组织内容」）、`复制 Markdown`；
  - 提示条：新增/变化/移出分类计数 + 「更新日报 / 忽略」（忽略仅会话内、绑定当前变更指纹）；
  - 生成中：阶段化进度 + 取消；
  - 空态：当日无文章 / 范围 0 条 / AI 未配置（跳设置→AI 的引导按钮）。
- 历史入口：日期选择器或「历史报告」列表（日期 + 范围 + 模型）——初版提供基础历史查看。
- Android：同一 command 面，入口跟随既有响应式导航（列表页头/操作面板）。

## 8. commands 与 MCP

### 8.1 Tauri commands（core 三阶段，锁内不 await）
| 命令 | 语义 |
|---|---|
| `digest_get(date, scope)` | 读报告头 + 正文 + 更新状态（added/changed/removed/…），零 AI |
| `digest_status(date, scope)` | 仅状态检查（走 digest_entry_meta 覆盖索引） |
| `digest_generate(date, scope, mode: update\|rewrite)` | 异步启动流水线，返回 job id；可取消 |
| `digest_cancel(job_id)` / `digest_list()` | 取消 / 有报告日期与变体列表 |

### 8.2 MCP（只读，绝不触发 AI）
- `digest_list`：日期 + 范围 + 摘要 ≤140 字（默认 10、上限 50）；
- `digest_get(date, scope)`：正文 + 来源清单分页单取；
- 与 command 共享 core 路径；响应口径遵守 MCP 既有约定。

## 9. 前置修正（进入日报实施的第一批，独立可测）
1. `AiClient::with_proxy` 显式 `tls_certs_only(webpki_root_certs())`（红线：Android 纯根存储验证）；
2. AI 响应读取加大小闸门（流式累计超限即断，红线 11 获取中限额）；
3. schema `detect`/迁移/备份校验升级至 BASELINE_VERSION 2（§5.3）。

## 10. 测试策略
- core 单测：清单冻结与 manifest_hash 稳定性；输入哈希变化 → 缓存失效；范围 OR 解析；DST 日界；
  预算溢出 → 分组合成；CAS 提交冲突路径；取消后半成品与旧报告保留。
- store 测试：迁移 1→2（旧库打开→迁移→重开）、备份恢复跨版本、digest DAO、meta 维护
  （upsert_entries / set_fulltext / 删除对 digest_entry_meta 的联动）。
- 性能断言：状态检查查询 `EXPLAIN` 走 `idx_digest_meta_day` 覆盖路径（红线 1/2 的变异校验式断言）。
- 命令/MCP：digest_* 命令的缓存命中与零请求路径；MCP 截断与分页口径。
- UI：日报视图增量更新（不重建列表）、双语键齐全。

## 11. 实施切分建议
- **阶段 0**：前置修正（§9）——独立可测，先落地。
- **阶段 1**：迁移 + `feed_tags` + 打标 UI + `digest_entry_meta` + 范围解析 + `digest_status/get` 只读 + 侧栏入口与空态。
- **阶段 2**：生成流水线（要点→归并→合成）+ 状态机 + 进度/取消 + 报告视图完整交互。
- **阶段 3**：MCP 工具 + 历史入口 + Android 适配 + 打磨。

## 12. 边界与开放点（记录在案）
- Android 实机行为（TLS 修正后的真实网络回归）待设备验证；
- 英文界面与长标签极端排版在实现期逐屏检查；
- 一次性的「全部文章重新分析」高级动作、文件导出、订阅计划式自动生成——均留待后续版本；
- 日报成品的富文本渲染样式（引用块/分组标题）在实现期由 UI 按报纸主题语法定稿。
