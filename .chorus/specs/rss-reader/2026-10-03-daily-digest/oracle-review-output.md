Task: You are reviving a previous subagent conversation.
Original run: 69014aba-cd2c-4400-bb12-192c36f0b566
Original agent: oracle
Original session file: /home/tcy/.pi/agent/sessions/--home-tcy-Github-RustRss--/2026-10-03T08-59-05-860Z_01a100fd-0b03-7077-9dad-05a6b3db04be/forks/2026-10-03T14-52-17-734Z_01a10240-67c6-7077-9dad-05a8d9ee61ae.jsonl
Use the stored session context as background. Answer the orchestrator's follow-up below. Do not assume the original child session is still running.
Follow-up:
你在写最终设计文档时被 30 分钟超时切断。继续完成：把最终推荐方案完整写出并输出为最终回复。要求：
1. 已完成的裁定（Inherited decisions + Diagnosis 十条修正）作为定论直接采用，不要再重新验证；
2. 补完剩余部分：数据模型最终 DDL（含 feed_tags、digests、schema 迁移路径的具体写法——注意 BASELINE_VERSION 约束，说明 detect/migrate 需要怎么改）、生成状态机（含检查点冻结输入清单、竞态、取消后的半成品语义）、配置面、core/commands/UI/MCP 职责划分、AiClient 的 tls_certs_only 与响应大小上限两个前置修正、测试策略、边界清单；
3. 输出为结构化 Markdown 设计文档（我会直接整合进 design.md），不要再跑验证工具，直接写完。
# 每日日报（Daily Digest）设计方案
状态：**方案审查完成，推荐作为实现基线；尚未实施。**
核心裁定：
> 日报是“某个自然日、某个订阅源范围、某套生成参数下的一份本地报告”。  
> 打开报告默认只读缓存；检查更新不调用 AI；重新生成按冻结的输入快照执行，并复用未变化的文章要点与中间合成结果。
初版同时覆盖桌面和 Android，不加入定时生成、后台系统服务或新的云端依赖。
---
## 1. Inherited decisions：继承与已确认决定
### 1.1 用户需求
- 提供今日日报、昨日日报。
- 用现有 BYOK AI 对范围内内容进行总结。
- 报告默认持久化缓存，允许强制重新生成。
- 根据上次报告的检查点发现新内容或内容变化，提示用户决定是否更新。
- 再生成时关注缓存命中率和调用成本。
- 支持按多选标签限定关注的**订阅源**。
### 1.2 已与主 agent 确认的标签口径
这是定论，不再作为开放问题：
1. 新增 `feed_tags`，复用现有 `tags` 字典。
2. `feed_tags` 与 `entry_tags` 是独立关联，互不隐含。
3. 选中多个标签时，对订阅源采用 **OR** 匹配。
4. 纳入匹配订阅源的当日全部文章，不再叠加文章标签过滤。
5. 空标签选择表示全部订阅源；范围选择器回显匹配的订阅源数量。
6. 文章标签侧栏及其未读计数维持现状，不计入 `feed_tags`。
7. 复用标签选择器，为订阅源增加标签编辑入口；初版不增加独立标签管理页。
### 1.3 架构约束
- 日报业务、范围解析、缓存键、预算、状态机和存储逻辑在 core。
- Tauri 只处理 IPC、凭据获取、任务生命周期及事件桥接。
- UI 保持原生 JS/CSS；新增文案同时提供 zh-CN/en。
- 不在 store 锁内等待网络或其他异步操作。
- 保持既有文章列表节点与阅读位置；进度更新不得重建列表或正文。
- MCP 初版只读取日报，不新增可调用 BYOK、产生费用的生成工具。
- 不自动刷新订阅、抓全文、打开外链或向其他目的地发送内容。
---
## 2. Diagnosis：十条修正裁定
以下结论直接采用。
| # | 草案问题 | 最终裁定 |
|---|---|---|
| 1 | 把用户要求的订阅源标签替换成文章标签 | 新增 `feed_tags`，保持两种关联独立。现有 schema 只有文章级标签。 |
| 2 | 用生成**完成时间**作为检查点 | 改为生成前冻结输入清单，保存快照时间与源版本。完成时间只用于展示，否则会漏掉生成期间入库的文章。 |
| 3 | 只检查 `fetched_at > checkpoint` | 不足以覆盖全文抓取、删除、范围成员变化、同秒更新及系统时钟回拨；改为比较当前候选清单与已保存清单。 |
| 4 | 假定现有 `ai_cache` 已处理内容失效 | 现有键不包含实际输入哈希；日报任务必须加入实际输入哈希、端点身份及阶段参数。不能直接沿用普通摘要的缓存有效性假设。 |
| 5 | 假定单日 COUNT 自动走覆盖索引 | 此前 EXPLAIN 已确认草案全源查询为 `SCAN entries`。新增窄元数据投影和覆盖索引，状态检查不碰正文表 B 树。 |
| 6 | 认为直接追加 schema 迁移即可 | `detect()` 当前只接受 `BASELINE_VERSION=1`。必须同步修改检测、迁移、备份校验及相关测试，否则升级后的库再次打开会被拒绝。 |
| 7 | 把合成也硬塞进当前单文章 `plan_task` | 保留 `AiClient` 和三阶段模式；另建 `ai::digest` 编排。日报合成没有合法单篇 `entry_id`，不能伪造文章满足缓存外键。 |
| 8 | 合成缓存键包含检查点时间，缺少中间缓存 | 时间会无意义地破坏命中。缓存键改为稳定输入及子节点输出指纹，并缓存归并节点。 |
| 9 | 批量生成缺少冻结计划、费用授权及并发契约 | 加入预检、批量确认、单 flight、任务标识、提交 CAS、取消与失败恢复语义；禁止静默增加发送范围。 |
| 10 | 默认“复用客户端即满足所有网络约束” | `AiClient` 缺少显式纯 webpki 根验证，且响应整包读取无大小闸门。两项都是实现前置修正。 |
另外，日报查询不能继承列表的“隐藏已读”、当前搜索、当前排序或已加载分页范围。日报素材选择必须使用独立 DAO。
---
## 3. 八个开放问题的最终决策
| 问题 | 决策与理由 | 初版 |
|---|---|---|
| 1. 大量文章的上下文管理 | 有界的“文章要点 → 缓存归并树 → 日报”流程。每次请求都验证实际输入大小；超预算先缩小范围，不静默丢文章。 | 必须 |
| 2. 内容变化后的缓存失效 | 对**真正发送的规范化输入**求哈希；不能拿 RSS 入库去重用的 `content_hash` 代替。全文抓取路径也必须推进源版本。 | 必须 |
| 3. 自动生成 | 不做。进入页面、刷新完成、回到前台只检查本地更新；生成由用户明确触发。 | 不做 |
| 4. 导出 | 支持复制 Markdown；core 提供确定性的 Markdown 导出函数。保存 `.md` 文件后续复用桌面/Android 文档导出通道。 | 复制必做；文件导出后续 |
| 5. 时区变化 | 保存日期、时区说明和实际 UTC 起止边界。历史报告不因系统时区改变而重新归日。 | 必须 |
| 6. 多范围报告收纳 | 初版只有一套当前配置，但保留已生成的历史范围/参数变体；历史选择器显示范围及模型，不做多套自动化订阅计划。 | 简单历史必做 |
| 7. MCP 截断与分页 | 列表只给元数据和 ≤140 字简介；正文单取、有界分页；来源列表另分页。读取绝不触发 AI。 | 必须 |
| 8. 忽略提示持久化 | 仅会话内忽略，绑定当前变化指纹；变化指纹改变后重新提示。忽略不推进检查点。 | 会话内即可 |
---
## 4. 产品语义
### 4.1 日期归属
初版推荐固定口径：
```text
有效日期时间 = published_at 存在时使用 published_at
             否则使用不可变的 first_seen_at
```
某日日报选取：
```text
day_start_at <= effective_at < day_end_at
```
具体约束：
- 起止边界由 core 按当地自然日计算，不能直接加减 `86400` 秒。
- 昨天发布、今天才抓到的文章，属于昨日报告的新增素材。
- 昨天的文章今天修订，昨日报告应提示内容变化。
- 更早日期的文章发生变化，不算今日日报的新增素材。
- 已读、星标、稍后读、文章标签变化不影响日报素材。
- 不自动获取文章全文；使用已经存于本地的正文或摘要。
- 这是“已抓取内容的日报”，不承诺覆盖互联网上该日的全部内容。
已有数据库无法还原历史文章真正的首次入库时间。迁移时以现有 `fetched_at` 回填并标记为估计值；新文章从此准确记录。
### 4.2 检查点不是一个完成时间
每份成功报告保存：
- `checkpoint_at`：输入清单冻结时刻。
- `generated_at`：报告成功提交时刻。
- `manifest_hash`：该次输入清单的指纹。
- 每篇素材的实例身份、源版本、实际输入哈希。
UI 文案应区分：
> 内容快照：10:00  
> 生成完成：10:04
例：
```text
10:00 冻结 100 篇素材
10:02 抓到第 101 篇
10:04 报告生成完成
```
新报告仍是那 100 篇的报告，完成后立即显示“有 1 篇新增内容”。不能用 10:04 覆盖输入检查点。
### 4.3 更新类型
状态接口至少返回：
```text
added_count
changed_count
removed_count
scope_invalid
configuration_changed
```
这里的“移除”包括删除文章、取消订阅、源标签变化导致文章离开范围，以及发布时间修订导致文章离开该日。
提示文案不能始终写“新增 N 篇”。应能够表示：
> 新增 3 篇，内容变化 2 篇，移出范围 1 篇。
### 4.4 两个重新生成动作
初版提供：
1. **更新报告**
   - 对当前素材重新制定计划。
   - 复用文章要点、归并节点和可复用的最终结果。
   - 没有实际输入变化时可零 AI 请求完成。
2. **强制重写报告**
   - 复用未变化的文章要点和归并节点。
   - 强制绕过最终合成缓存，至少重新执行最终合成。
   - 明示：“复用文章要点，重新组织日报内容”。
不把“深度重生成”做成持久设置，避免用户忘记关闭后每天重复付费。
“全部文章重新分析”可后续作为一次性的高级动作；换模型、换端点、换要点提示词本身已自然导致相应缓存失效。
---
## 5. 最终数据模型
### 5.1 数据组织原则
- 复用 `ai_cache` 存文章级日报要点。
- 新增归并/最终节点缓存，不伪造文章记录。
- 报告头与正文分表，列表和计数不读取报告正文。
- 一个日期窗口、范围、生成参数变体保存一份最新成功报告。
- 重生成成功才替换该变体；失败或取消不覆盖旧报告。
- 来源快照不外键关联文章：文章删除后，历史报告仍可阅读。
- 使用不复用的文章实例身份，避免 SQLite 行号复用后旧引用指向新文章。
### 5.2 新迁移 DDL
以下作为新增迁移内容；不修改已发布的基线 SQL。
```sql
-- 1. 订阅源级标签，与 entry_tags 独立。
CREATE TABLE feed_tags (
    feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tags(id)  ON DELETE CASCADE,
    PRIMARY KEY (feed_id, tag_id)
);
CREATE INDEX idx_feed_tags_tag
    ON feed_tags(tag_id, feed_id);
-- 2. 日报查询的窄元数据投影，不放正文。
CREATE TABLE digest_entry_meta (
    instance_id          INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id             INTEGER NOT NULL UNIQUE
                         REFERENCES entries(id) ON DELETE CASCADE,
    feed_id              INTEGER NOT NULL
                         REFERENCES feeds(id) ON DELETE CASCADE,
    first_seen_at        INTEGER NOT NULL,
    first_seen_estimated INTEGER NOT NULL DEFAULT 0
                         CHECK (first_seen_estimated IN (0, 1)),
    effective_at         INTEGER NOT NULL,
    source_revision      INTEGER NOT NULL DEFAULT 1
                         CHECK (source_revision >= 1)
);
CREATE INDEX idx_digest_meta_day
    ON digest_entry_meta(
        effective_at, feed_id, entry_id, instance_id, source_revision
    );
CREATE INDEX idx_digest_meta_feed_day
    ON digest_entry_meta(
        feed_id, effective_at, entry_id, instance_id, source_revision
    );
-- 历史首次入库时间只能近似回填；不重算所有正文的哈希。
INSERT INTO digest_entry_meta (
    entry_id, feed_id, first_seen_at, first_seen_estimated, effective_at
)
SELECT
    id, feed_id, fetched_at, 1, COALESCE(published_at, fetched_at)
FROM entries;
-- 3. 报告头：一个参数变体的一份最新成功结果。
-- revision = 0 表示已有生成槽位，但尚无成功报告。
CREATE TABLE digests (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    report_day         TEXT NOT NULL,
    timezone_label     TEXT NOT NULL,
    day_start_at       INTEGER NOT NULL,
    day_end_at         INTEGER NOT NULL,
    utc_offset_start   INTEGER NOT NULL,
    utc_offset_end     INTEGER NOT NULL,
    date_basis         TEXT NOT NULL
                       DEFAULT 'published_or_first_seen_v1',
    scope_key          TEXT NOT NULL,
    scope_json         TEXT NOT NULL,
    profile_key        TEXT NOT NULL,
    profile_json       TEXT NOT NULL,
    revision           INTEGER NOT NULL DEFAULT 0
                       CHECK (revision >= 0),
    active_job_id      INTEGER,
    completed_job_id   INTEGER,
    checkpoint_at      INTEGER,
    generated_at       INTEGER,
    manifest_hash      TEXT,
    final_node_key     TEXT,
    article_count      INTEGER NOT NULL DEFAULT 0,
    cited_count        INTEGER NOT NULL DEFAULT 0,
    truncated_count    INTEGER NOT NULL DEFAULT 0,
    summary_only_count INTEGER NOT NULL DEFAULT 0,
    stats_json         TEXT NOT NULL DEFAULT '{}',
    summary            TEXT NOT NULL DEFAULT ''
                       CHECK (length(summary) <= 140),
    stored_bytes       INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL,
    CHECK (day_start_at < day_end_at),
    UNIQUE (
        report_day, day_start_at, day_end_at,
        date_basis, scope_key, profile_key
    )
);
CREATE INDEX idx_digests_history
    ON digests(report_day DESC, completed_job_id DESC, id DESC);
CREATE INDEX idx_digests_scope_history
    ON digests(scope_key, report_day DESC, id DESC);
-- 4. 报告正文：只由单份读取接口加载。
CREATE TABLE digest_bodies (
    digest_id   INTEGER PRIMARY KEY
                REFERENCES digests(id) ON DELETE CASCADE,
    revision    INTEGER NOT NULL CHECK (revision >= 1),
    schema_ver  INTEGER NOT NULL,
    content_json TEXT NOT NULL,
    markdown     TEXT NOT NULL
);
-- 5. 成功报告使用过的素材清单及引用快照。
-- entry_id / feed_id 故意不外键关联当前文章和订阅源。
CREATE TABLE digest_items (
    digest_id       INTEGER NOT NULL
                    REFERENCES digests(id) ON DELETE CASCADE,
    instance_id     INTEGER NOT NULL,
    entry_id        INTEGER NOT NULL,
    feed_id         INTEGER NOT NULL,
    source_revision INTEGER NOT NULL,
    effective_at    INTEGER NOT NULL,
    input_hash      TEXT NOT NULL,
    truncated       INTEGER NOT NULL CHECK (truncated IN (0, 1)),
    summary_only    INTEGER NOT NULL CHECK (summary_only
# RustRss 每日日报设计方案
**状态：推荐定稿，待产品确认后实施。**  
**范围：设计，不包含代码修改或发布操作。**
核心方案：
> 按日期与订阅源标签选取文章，冻结本次输入快照，复用内容寻址的文章要点缓存，再分层合成为日报。报告默认读取本地缓存；数据变化只提示，不自动调用 AI。重新生成失败或取消时，上一份成功报告始终保留。
---
## 1. Inherited decisions：继承与已确认决定
### 1.1 用户需求
- 提供今日日报、昨日日报。
- 使用现有 BYOK AI 服务总结内容。
- 默认缓存报告，允许强制重新生成。
- 根据上次生成的检查点识别新增、更新内容，提示用户是否重新生成。
- 再生成应尽可能复用已有处理结果，减少费用。
- 支持多选标签限定关注的订阅源。
### 1.2 已与主 agent 确认的订阅源标签口径
1. 新增 `feed_tags`，复用现有 `tags` 字典。
2. `feed_tags` 与 `entry_tags` 是独立关联，互不隐含。
3. 选择多个标签时，订阅源按 **OR** 匹配；取这些源在日报日期范围内的全部文章。
4. 未选择标签，且
# RustRss 每日日报：最终推荐设计
**状态：设计定稿建议，尚未实现。**
目标：提供「今日日报 / 昨日日报」，按订阅范围汇总本地文章；默认读取缓存，准确提示内容变化，支持主动重新生成，同时控制 AI 成本。
---
## 1. Inherited decisions｜继承决定
以下约束不变：
- 使用现有 BYOK AI 通道，不增加外部服务。
- 业务、查询、缓存和生成策略进入 `rustrss-core`；Tauri 负责凭据、任务生命周期和界面桥接。
- 前端保持原生 JS/CSS，不引入构建链。
- 读取报告、检查更新不产生 AI 请求；生成必须由用户触发。
- 保留现有文章列表节点身份、阅读位置与增量更新方式。
- 所有新文案同时提供 zh-CN / en。
- SQLite 查询不能因为日报状态检查而扫描正文表。
- 取消、失败和并发重入不能破坏上一份完整报告。
- 本轮交付设计，不修改应用代码、版本或发布资产。
### 已与主 agent 确认的范围裁定
**采用订阅源级标签，纳入初版。**
1. 新增 `feed_tags`，复用现有 `tags` 字典。
2. `feed_tags` 与 `entry_tags` 是独立关联，互不隐含。
3. 日报按任一选中标签匹配订阅源，即 OR 语义；包含这些订阅源在目标日期内的全部文章。
4. 没有选择标签时，范围为全部订阅源。
5. 设置中显示匹配的订阅源数量。
6. 文章级标签筛选不进入初版。
7. 侧栏标签未读数仍只统计文章标签，不能混入源标签。
8. 复用标签选择器，桌面订阅源菜单与 Android 操作面板均提供入口。
---
## 2. Diagnosis｜十条修正裁定
### 2.1 不能把订阅源标签替换为文章标签
现有 `tags/entry_tags` 是文章级标签，不能直接满足用户提出的订阅范围配置。
**裁定：新增 `feed_tags`，不改变文章标签语义。**
### 2.2 不能把生成完成时间当作可靠检查点
例如：
- 10:00 冻结文章列表；
- 10:02 抓到新文章；
- 10:05 报告生成完成。
若检查点写成 10:05，随后查询 `fetched_at > checkpoint`，10:02 的文章将被漏掉。
**裁定：检查点对应生成前冻结的输入清单。生成完成时间只用于显示。**
### 2.3 `fetched_at` 不足以表达内容变化
`Store::upsert_entries` 会在 RSS 内容变化时更新 `fetched_at`，但 `Store::set_fulltext` 不更新它，也不更新 RSS 的 `content_hash`。
因此：
- `fetched_at` 不是不可变的首次发现时间；
- 全文变化可能绕过时间戳检查；
- 秒级时间戳、系统时钟回拨也不适合承担版本标识。
**裁定：增加窄元数据投影和文章版本；检查更新比较快照成员及其版本。**
### 2.4 现有 AI 缓存没有自动包含正文哈希
现有键为：
```text
(entry_id, task, params, provider_model, prompt_version)
```
`params` 当前主要是摘要长度、语言或翻译目标，不能据此认为正文变化会自动失效。
同时，`AiConfig::cache_tag()` 只有 provider/model，没有自定义端点身份。
**裁定：日报缓存键必须包含实际模型输入哈希、端点身份和有效生成参数。**
### 2.5 草案中的 COUNT 查询违反性能前提
已用当前基线 DDL 在内存 SQLite 检查：
```text
原草案全库日期 + fetched_at COUNT：SCAN entries
带 feed_id：使用 published 索引，但不是覆盖查询
```
“单日候选应该很少”不能替代覆盖索引保证。
**裁定：日报状态检查使用独立窄表及覆盖索引，不读取 `entries` 正文表。**
### 2.6 不能直接复用默认 `list_entries` 作为日报输入
列表查询可能继承全局排序和隐藏已读设置。日报不应因用户读过文章、切换列表排序而改变素材。
**裁定：独立日报查询。初版包含已读和未读，不继承文章列表筛选。**
### 2.7 合成任务不能硬塞进单篇文章缓存接口
现有 `plan_task` 必须有 `entry_id`，且 `ai_cache` 外键指向文章。合成任务不是一篇文章。
**裁定：**
- 条目要点复用 `ai_cache` 的存储结构；
- 合成使用独立任务构造和中间节点缓存；
- 不创建“虚拟文章”承载日报缓存。
### 2.8 检查点不能进入合成缓存键
每次生成时间不同，若把它加入缓存键，会让内容完全相同的请求也无法复用。
**裁定：缓存由内容和配置决定；时间仅作为报告元信息。**
### 2.9 追加迁移还必须修改 schema 检测和备份校验
当前 `schema::detect` 只把 `user_version == BASELINE_VERSION == 1` 视为可接受。只追加迁移会导致升级后的数据库再次打开时被拒绝。
**裁定：保留发布基线不变，同时支持“已发布旧版本 → 当前版本”的升级路径。**
### 2.10 现有 `AiClient` 需要两个前置修正
源码中的 AI 客户端构造尚未显式调用 `tls_certs_only`，响应读取使用整包 `.text()`。
**裁定：日报接入前，修正共享 AI 客户端的根证书验证与响应大小限制，不能另造一个只对日报正确的客户端。**
上述 TLS 结论是源码审查发现，不代表已经进行 Android 网络复现。
---
## 3. 草案开放问题 1–8 的结论
| 问题 | 决策 | 初版 |
|---|---|---|
| 1. 大量文章与上下文预算 | 有界条目提炼＋稳定分组合成＋必要时递归归并；所有阶段发送前检查预算 | 必须 |
| 2. 正文变化使缓存失效 | 实际输入哈希必须进入条目缓存键；RSS `content_hash` 不能直接替代 | 必须 |
| 3. 自动生成 | 不做。允许自动进行本地状态检查，但不自动调用 AI | 不实现自动生成 |
| 4. 导出 | 初版复制 Markdown；内部先提供纯函数 Markdown 导出格式，文件保存可后续接系统文档接口 | 复制必须，文件导出后置 |
| 5. 时区变化 | 保存报告日期、实际 UTC 起止边界、时区显示信息；历史报告不随系统时区重解释 | 必须 |
| 6. 多范围报告收纳 | 一个当前配置＋历史报告列表；历史显示日期、范围和模型，不做多套定时订阅计划 | 必须有基础历史入口 |
| 7. MCP | 只读读取报告及来源；列表遵循默认 10、最多 50、摘要 ≤140 字；正文单取并有分页上限 | 基础只读工具 |
| 8. 忽略提示 | 仅会话内忽略，绑定当前变更指纹；后续又有变化时重新提示 | 必须；不持久化忽略 |
---
## 4. 产品语义
### 4.1 日期归属
初版采用：
```text
文章有发布时间：按发布时间归日
文章无发布时间：按首次入库时间归日
```
理由：
- 与“昨天发布了什么”的日报直觉一致；
- 晚抓到的昨日文章可以补入昨日报告；
- 无发布时间的文章不会因再次刷新而从昨日移动到今日。
首次入库时间只对新数据准确。迁移前没有该字段的文章，以现有 `fetched_at` 初始化；不能宣称恢复了真实首次发现时间。
**这项回退只用于日报，不顺带修改既有文章列表的排序语义。**
日期范围使用：
```text
[本地日期开始对应的 UTC 时刻, 下一本地日期开始对应的 UTC 时刻)
```
不得用 `start + 86400`，因为夏令时可能形成 23/25 小时的一天。
### 4.2 今日与昨日
- 「今日」是阶段性报告，不代表当天已经完整结束。
- 「昨日」仍可能因晚到文章、全文补抓或源内容修正而更新。
- 阅读中的报告绑定具体日期与边界，跨午夜不自动切换正文。
- 「今日 / 昨日」快捷入口在再次选择时解析成具体日期。
### 4.3 本地资料边界
报告只使用已经入库的内容：
- 不自动抓全文；
- 不自动访问文章链接；
- 不在生成开始前偷偷刷新全部订阅；
- 不因 AI 输出建议而请求新网址。
界面显示最近订阅刷新时间，并提供独立的「先刷新订阅」操作。不能把“报告已生成”描述为“已获取互联网上全部最新内容”。
### 4.4 更新提示
提示应区分：
- 新增文章；
- 已有文章内容变化；
- 文章移出范围或被删除；
- 订阅源标签关联导致范围变化。
示例：
> 自上次素材快照后，新增 8 篇、内容变化 2 篇、移出 1 篇。是否更新日报？
不能一律写成“N 篇新文章”。
### 4.5 三种用户行为
| 行为 | 语义 | 是否可能收费 |
|---|---|---|
| 查看报告 | 读取最后完整缓存，附带更新状态 | 否 |
| 更新日报 | 基于当前快照生成，复用全部有效缓存；完全相同则不请求 | 可能 |
| 强制重写报告 | 即使素材没变也重新进行最终合成，但复用文章要点和中间结果 | 至少一次请求，除空资料情况 |
初版不提供持久化的“每次都全量重跑”设置。
“忽略所有文章缓存、重新分析全部正文”可留待后续，以单次高级操作提供，必须额外说明成本。
---
## 5. 数据模型
### 5.1 模型分工
| 存储 | 内容 |
|---|---|
| `feed_tags` | 源与标签的关联 |
| `digest_entry_meta` | 窄文章元数据、稳定实例身份、版本、日期归属 |
| `digests` | 报告身份、检查点、统计、当前完整版本 |
| `digest_bodies` | 报告结构化正文与导出 Markdown |
| `digest_items` | 当前完整报告的素材清单及来源快照 |
| 现有 `ai_cache` | 条目级结构化要点，任务名为 `digest_item` |
| `digest_node_cache` | 中间归并
# RustRss 每日日报：最终推荐设计
**状态：设计完成，尚未实施。**  
目标：提供今日／昨日日报，默认读取本地缓存；发现素材变化后提示更新；允许强制重写，同时复用未变化的 AI 处理中间结果。
本文保留初稿的产品方向，但修正检查点、缓存身份、订阅源标签、数据库迁移和生成生命周期等关键问题。
---
## 一、Inherited decisions：继承与已确认的决定
1. **交付的是设计，不是立即实施或发布新版本。**
2. 日报覆盖桌面和 Android；复用现有 BYOK 提供方、代理设置、安全凭据存储。
3. 默认查看缓存，不因打开页面、切换日期或发现更新而自动调用 AI。
4. 业务规则、查询、缓存、生成编排进入 `rustrss-core`；Tauri 只承担凭据、命令、事件和平台胶水。
5. 前端继续使用原生 JS/CSS，补齐 zh-CN/en；保留 keyed reconcile、同值短路和阅读位置。
6. **订阅源级标签纳入初版，已与主 agent 确认：**
   - 新增 `feed_tags`，复用 `tags` 字典。
   - `feed_tags` 与 `entry_tags` 互不隐含。
   - 日报按所选源标签 **OR** 匹配订阅源，再纳入这些源的当日全部文章。
   - 不叠加文章标签过滤。
   - 侧栏标签未读计数仍只统计文章标签。
   - 设置中显示匹配的订阅源数量。
7. 首版不增加外部服务，不做自动定时生成、不增加 MCP 付费生成能力。
---
## 二、Diagnosis：直接采用的十项修正
| # | 修正后的定论 | 依据与影响 |
|---|---|---|
| 1 | **不能把源标签替换成文章标签。** | 当前 schema 只有 `entry_tags`；用户要求的是带标签的订阅源。新增 `feed_tags`。 |
| 2 | **检查点不能使用生成完成时间。** | 10:00 取素材、10:05 完成期间，10:02 入库的文章会被完成时间检查点永久漏掉。必须保存生成前冻结的素材清单。 |
| 3 | **`fetched_at` 不是可靠的内容版本。** | `upsert_entries` 在内容更新时改它；`set_fulltext` 不改它，也不改 `content_hash`。见 `store/mod.rs:965–1082`。 |
| 4 | **L1 必须按实际输入内容失效。** | `AiTask::cache_params()` 目前只有长度／语言；`AiConfig::cache_tag()` 只有提供方／模型，没有正文哈希或端点身份。不能宣称现有缓存已解决这些问题。 |
| 5 | **不能依赖初稿的 COUNT 查询。** | 已完成的 SQLite 分析显示：初稿无 feed 条件的查询为 `SCAN entries`；带 feed 条件也不是覆盖扫描。日报应有独立窄元数据查询路径。 |
| 6 | **合成缓存不能包含生成时间。** | 时间每次变化会主动破坏命中率；仅按文章 ID 计算又会错误命中过期结果。应按规范化输入及子节点输出哈希计算。 |
| 7 | **日报不能直接套用单文章任务接口。** | `plan_task` 必须读取一个 `entry_id`；`ai_cache` 也有文章外键。合成请求应有独立构造器和节点缓存，不能伪造文章 ID。 |
| 8 | **追加迁移还不够。** | `schema::detect()` 目前只接受 `BASELINE_VERSION == 1`。直接追加迁移后，新版本库再次打开会被拒绝；检测、迁移和备份恢复必须一并升级。 |
| 9 | **复用 AiClient 前有两个前置修正。** | 当前构造器缺少显式 `tls_certs_only`；响应使用无界 `resp.text()`。分别违反 Android 根证书约束与获取中限额要求。 |
| 10 | **“异步命令”不等于完整任务生命周期。** | 必须定义冻结计划、批量发送确认、单 flight、取消、失败重试、原子提交及旧报告保留，不能仅把逐篇请求塞进一个 command。 |
另外，日报查询**不继承列表的隐藏已读、排序、搜索、当前分页或选择状态**。当前 `list_entries()` 会读取全局列表设置，不能不加区分地复用。
---
## 三、产品语义
### 3.1 日期范围
初版采用：
> 按文章发布时间归日；没有发布时间的文章，按首次进入本地库的时间归日。
- 使用本地自然日的半开区间：`[day_start_utc, day_end_utc)`。
- 日界线由 core 计算，前端不自行减去 86,400 秒。
- 今日报告是“截至某次素材快照的阶段性日报”。
- 昨日报告仍可能因晚到文章、正文更新或全文抓取而出现更新。
- 发布时间不属于该日的文章，不因今天刚抓到而自动进入今日日报。
- 日报只基于**本地已经抓取的内容**，不代表源站内容完整，也不代表实时互联网检索。
没有发布时间时使用独立的 `first_seen_at`，不能继续使用会变化的 `entries.fetched_at`。
对存量文章，首次入库时间无法准确追溯：迁移时以已有 `fetched_at` 初始化，并在设计／迁移记录中明确这是近
