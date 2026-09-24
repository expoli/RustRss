# 中文单字候选索引（2026-09-24）

## 原因与修复

原plan_query对单字只生成LIKE，需要扫描title/content_text。万篇SSD冷页稀疏/无结果查询661–706ms，见search-offline.md。

v16在已有预分词串后补缺少的CJK单字；已有bigram包含原字符，所以迁移不重新解析/抓取文章。Store迁移事务内按主键每批200条读取词串，修改后由原FTS触发器维护索引，版本与数据一起提交。新入库与全文写回通过同一to_tokens生成单字。

搜索保留LIKE原字段校验，避免仅在作者/摘要出现的单字扩大原匹配范围；纯单字仍时间排序，混合/多字查询仍bm25。纯单字外层钉时间索引，以FTS rowid集合筛选，取满LIMIT即停，避免高频字先读所有候选正文记录再排序。新增单字影响FTS词长统计，相关度数值可能变化。

## 复现与变异

- tokenizer单字候选测试先真实失败（fts为空）。
- search_unigrams两条真实文件测试：v15升级匹配/时间顺序/标志正文保持/重启；移除真实FTS单字即少一个结果的负对照；摘要类词串单独命中被原字段校验排除；隐藏已读仍生效。
- 人为触发器在第2行中断迁移：第1行修改与user_version均回滚，移除故障后可重试。
- 生产search_sql同源EXPLAIN：单字走排序索引与FTS、无临时排序；删除INDEXED BY约束的负对照出现TEMP B-TREE，测试成立。

## 性能及反例

```sh
cargo build -p rustrss-core --example search_scale
/usr/bin/python3 scripts/verify-search-scale.py
```

脚本创建/tmp生产upsert夹具及打印的SSD缓存库；每词前fsync/fadvise并mincore确认零驻留页。完成后清理这两个自有路径。raw结果见search-unigrams-results.json。

首次候选实现让常见“文”退化到249ms，保留在rejected_single_character_attempt中；调整为外层时间索引后，仅复跑受影响的3个单字档，避免重新生成整库：

|单字|旧冷页首次|修复后冷页首次|修复后热查|
|---|---:|---:|---:|
|文（取200条）|17.94ms|33.88ms|2.72–2.94ms|
|稀（1条）|706.04ms|17.80ms|2.05–2.19ms|
|龘（0条）|660.57ms|16.05ms|1.89–2.11ms|

高频字比旧LIMIT早停路径有额外候选集合成本，但消除了稀疏/无结果全正文扫描。新样本库50,061,312字节（原44,736,512），打开并迁移约2789ms；基准中迁移前文件页已有驻留，这不是冷页升级耗时。第一次升级不适用既有稳定态≤2秒启动结论。

## 仍未完成

## v17：宽泛相关度查询延迟读取列表行

旧的 BM25 SQL 会在排序前为每个 FTS 命中读取 `entries` 列表字段；这些字段位于正文大列之后，10k篇时宽泛查询约214–238ms。v17添加 `(id, read, COALESCE(published_at, fetched_at))` 覆盖索引，候选阶段只从它取ID、隐藏已读位和时间排序键，算BM25并截取前200；物化结果后才查文章标题、摘要等列表字段。

`ranked_search_sql` 是运行与EXPLAIN共用的SQL。查询计划要求 `idx_entries_search_order` 覆盖扫描，并要求先物化候选结果；删除索引的负对照会令生产SQL无法准备。集成回归覆盖相同BM25分数时的时间/ID次序、LIMIT和隐藏已读先过滤。

10k生产upsert样本每次查询前 `fsync`、`fadvise(DONTNEED)`，并由`mincore`确认12222页零驻留。结果见 `search-ranked-results.json`：宽泛英文“common”冷查77ms、后续24–32ms；中文“中文”冷查77ms、后续约25ms；“新闻”冷查65ms、后续约25ms。稀疏/无结果单字维持约9–19ms冷查，热查约2ms。v15旧版宽泛查询约214–238ms。

本轮由v15样本升级到v17约2.68秒，包含v16单字补词和v17索引构建；升级前样本已有热页驻留，因此不是冷页迁移耗时。`dbstat`测得v17排序索引占167,936字节；升级后该样本文件为50,061,312字节。

全文搜索“即时”AC仍开放：本机Debug core查询通过，桌面真实输入到列表绘制耗时、Release、Wayland原生输入及Windows/macOS尚未复测。
