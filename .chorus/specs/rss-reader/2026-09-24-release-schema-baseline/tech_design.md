---
title: Tech Design: schema 基线压平与老库拒绝策略
proposalUuid: 805f1a16-2c20-43f7-b452-f1f33c14e334
documentUuid: 0bc56a01-16a4-4cf6-971a-c9eed2d1eb9e
---

# Tech Design: schema 基线压平与老库拒绝策略

> 设计指引，实现以代码为准；与 PRD 冲突时以 PRD 的验收标准为准。

## 1. 基线形态

- `schema.rs`：`pub const MIGRATIONS: &[&str] = &[BASELINE]`，`BASELINE` 由现 v1..v13 拼成的一条迁移（保留原 SQL 文本与注释里的行内约束说明，去掉“版本演进”叙述）。
- **基线标识**：`pub const BASELINE_APPLICATION_ID: i32 = 0x5253_5331;`（"RSS1"）。基线里 `PRAGMA application_id = BASELINE_APPLICATION_ID`，`user_version` 仍为 `1`。
  - 为何不用版本号区间：基线号是 1，而旧链也存在 v=1 的冻结库 → 「按 `user_version` 区间拒绝」会同时误拒新库与漏放旧库（Round 1 B1）。`application_id` 是 SQLite 自带的“这个库属于哪个应用”标识，且全仓此前未使用（`rg application_id` 无命中）。将来要兼容时它也提供了稳定的引入点。
- `migrate()`：去掉 `version == 13` 的特殊分支；回填逻辑搬到独立函数（如 `store/backfill.rs::apply_pre_release_backfill(conn)`），基线里不调用它 —— 新库不需要回填（数据是新的），仅作将来兼容时的可复用件并配单测。
- 基线版本号保持 `1`（`user_version = 1`），不引入「基线号 = 13」的伪兼容。

## 2. 老库判定与拒绝

判定顺序（`Store::open` 内，先只读探测，不改文件）：

| 探测结果 | 处置 |
|---|---|
| 无任何用户表（`SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'` 为 0） | 视为新库 → 跑基线（写 `application_id` + `user_version`） |
| 有用户表，`PRAGMA application_id != BASELINE_APPLICATION_ID` | **拒绝**：旧开发库（含旧链 v=1 冻结库，其 `application_id = 0`）或外来 sqlite 文件 |
| 有用户表，`application_id == 魔数`，`user_version == 1` | 正常打开 |
| 有用户表，`application_id == 魔数`，`user_version != 1` | **拒绝**：更新的（未来版本的）库 |

- 拒绝返回结构化错误（错误码 + 人类可读原因），**不进入 migrate**，且**拒绝先于任何写**（探测阶段只用只读语义，不建表、不写 PRAGMA）。
- 桌面侧（`AppState::open` 失败）：弹窗（zh-CN/en 双语文案）+ 「导出 OPML」（若库里可读）与「备份后重建」；用户拒绝 → 进程正常退出，库保持原样。
- 只读导出是例外路径：允许以只读方式打开旧库导出 OPML；这条必须显式实现并测试，不能靠“能打开就顺手用”。
- **独立 MCP 二进制**（`crates/rustrss-mcp/src/main.rs:41` 同路径 `Store::open`）：命中同一错误时以非零退出码 + 可读错误输出呈现（HTTP 模式返回结构化错误），**不得因拒绝而自建新库或写盘**（NOTE N2）。

## 3. 等价性断言

- 测试里构造两个库：
  - A：`MIGRATIONS = [BASELINE]` 跑一遍；
  - B：把**旧 13 条链**作为测试夹具（`tests/fixtures/legacy_chain.rs` 或测试内常量）跑到底。
- 规范化 dump：`SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name`，对 `sql` 做空白规范化（折叠连续空白、去尾分号差异）后逐条比较；差异打印对象名。
- `application_id` **不在** `sqlite_master` 里，因此单独断言：新库 == 魔数；旧链夹具跑完的库 == 0（这正是“旧库可被识别”的依据）。
- 覆盖率闸门：断言 dump 里包含预期对象数（防“查询本身返回空导致假绿”），并对基线做一次**变异校验**（删/改一个索引 → 测试必须变红）。

## 3.5 压平的测试波及面（必须同步改，否则静默跳过）

压平后所有“用 `PRAGMA user_version` 造旧库、期待旧链自动升级”的测试都会失效：旧库现在被拒，而 `for version in 1..migrations.len()` 会退化成空区间（测试静默变绿）。已清点 8 个文件：

| 文件 | 现状用例 | 改法 |
|---|---|---|
| `tests/durability.rs` | `every_schema_version_preserves_rows_and_flags_on_upgrade` | 拆两路：旧链夹具 → 断言被拒；基线库 → 断言写入后重开不丢行/read/starred/content；夹具版本数下限防空跑 |
| `tests/store.rs`（21 处引用） | 迁移保全/边界 | 改为基线语义或断言拒绝 |
| `tests/tags.rs` | `migration_v12_upgrade_keeps_existing_data_and_adds_tag_schema` | 同上 |
| `tests/search_unigrams.rs` | `upgrade_indexes_...` / `interrupted_upgrade_rolls_back_...` | 同上；中断回滚语义改为基线创建的中断回滚 |
| `tests/retry_after.rs` | v12 升级夹具 | 同上 |
| `tests/fulltext.rs` | 迁移相关夹具 | 同上 |
| `tests/feed_order.rs` | `v13_upgrade_keeps_alphabetical_order_and_cooldown` | 同上 |
| `tests/backup.rs` | 备份版本边界（含 0 字节文件） | 边界改为 `application_id` + `user_version` 双判 |

原则：能保留语义的改成基线等价用例；只能表达旧链行为的，改成断言“被拒绝”——**不允许删除后不留替代断言**。

## 4. 备份边界

- `store/backup.rs:181` 的 `(0, current_version]` 校验：改为 `application_id == 魔数 && user_version == 1`（双判）；旧备份（`application_id = 0`）、文本文件与 0 字节文件都走“明确拒绝 + 可读原因”（0 字节文件将因 `application_id = 0` 被拒，比原来的“合法空库”边界更紧）。
- 备份文件里写版本号之外，再写一行应用版本（便于将来排查），但不作为校验依据。

## 5. 发布工程收口

- `release.yml`：构建后记录三平台产物大小（`ls -l` 汇总进 job summary 或 artifact 元数据），体积数字回填 spec 的「安装包体积实测值」条目。
- CI 冒烟：`ci.yml` 保 `cargo test --workspace` 为必跑；release 流程不引入新依赖。
- README：「发布步骤」小节（版本 bump → CHANGELOG → tag → 产物与体积记录）。

## 6. 验收顺序（受本机资源约束）

1. core 子集：`cargo test -p rustrss-core`（可与非 GUI 工作并行）。
2. 全量：`cargo test --workspace`。
3. GUI 独占时段：老库拒绝提示 + 只读导出 OPML（隔离实例：`HOME` / `XDG_DATA_HOME` / `XDG_RUNTIME_DIR` 三件套，参照 `scripts/verify-*.py`）。
