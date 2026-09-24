---
title: PRD: Release 前 schema 基线压平与升级策略
proposalUuid: 805f1a16-2c20-43f7-b452-f1f33c14e334
documentUuid: 77226c88-20b2-4eac-b248-d4b9d4a1deea
---

# PRD: Release 前 schema 基线压平与升级策略

## 背景

`crates/rustrss-core/src/store/schema.rs` 现在是 **13 条逐版本迁移**（v1..v12 细粒度 + v13 已把若干未发版的 pre-release 增量合并到一条，见该文件 197 行注释）；`Store::migrate()`（`store/mod.rs:455-502`）在 `version == 13` 分支内还有数据回填循环（把 `search_tokens` 补成含 CJK 单字、从正文首图回填 `thumbnail_url`）。`crates/rustrss-core/tests/durability.rs:6` 以「v1..v12 分别升到 current」守护「升级不丢行、不丢标志」。

带着 13 段历史包袱发 1.0 没有收益：开发期没有需要兼容的用户库，而每一条迁移都是长期维护面与测试面。

## 决策（2026-09-24 用户拍板）

1. **压平成一份基线**：全新库 `user_version = 1`，`MIGRATIONS` 只含一条基线。
2. **不保开发库**：老库（`user_version < 基线`）不提供兼容升级路径，打开时明确提示「先备份 / 导出 OPML，再重建」，**禁止静默丢数据**。
3. 压平**只动迁移列表**；v13 的回填逻辑独立保留为函数（不随压平删除），保留将来若要兼容时复用的可能。
4. 等价性用**机械断言**证明（`sqlite_master` 规范化 dump 比对），不靠人眼扫 schema。
5. 备份/恢复对旧版本备份的态度与第 2 条一致：**明确拒绝**并给出可读原因。
6. 老库判定**不用版本号区间**，用 SQLite 原生标识消歧：基线写入 `PRAGMA application_id` 魔数。理由：基线号是 1，而旧链也存在 v=1 的冻结库，「按版本号区间判断」既会误拒新库、又会漏放旧链 v=1 库。

> 修订记录（Round 1 评审 FAIL 后）：原验收 2 写「老库 `user_version` 落在 `1..=13`」——基线=1 时该谓词字面覆盖全部版本，按字面实现会拒绝所有新库（B1）。已改为下述基于 `application_id` 的判定，并补齐 durability 重写（B2）、README 升级口径（B3）、重建路径验收（B4）的覆盖。

## 目标

- 首发基线：一份 schema，可机械证明与旧链终态等价。
- 升级语义：老库被明确拒绝（附安全出口），不是"悄悄用着结果数据不对"。
- 发布工程收口（并入本变更）：三平台产物体积有实测记录、CI 有必跑冒烟、README 有发布步骤。

## 非目标

- 不做「保留旧库并自动迁移」的兼容层（用户已定不保）。
- 不改任何业务字段语义；不顺手重构 `Store` 之外的模块。
- 不替其它平台做本机无法验证的事（Windows/macOS 产物体积依赖 CI 产物，如实标注）。

## 验收标准

1. 全新库 `application_id = 魔数` 且 `user_version = 1`；`sqlite_master` 的规范化 dump（表 / 列 / 索引 / 部分索引 WHERE / 触发器 / FTS 影子表）与「旧 13 条链跑完后的终态」**机械等价**；比对脚本进仓、失败信息可定位到具体对象，且带对象数下限断言与变异校验（删/改一个对象后必须变红）。
2. 老库判定用 SQLite 原生标识消歧（**不写版本号区间**）：无任何用户表 → 视为新库、建基线；存在用户表且 `application_id != 魔数` → 拒绝（旧开发库 / 外来 sqlite 文件，**含旧链 v=1 冻结库**）；`application_id == 魔数` 且 `user_version != 1` → 拒绝（更新的版本）。拒绝时给出明确提示并要求用户确认（备份 → 重建）；用户拒绝后**不写库**，`application_id` 与 `user_version` 均不变；提示文案 zh-CN / en 双语齐备；独立 MCP 二进制命中同一错误时可读呈现（非零退出或结构化错误），且拒绝先于任何写。
3. 确认 → 备份 → 重建 这条唯一恢复路径可验证：重建后新库 `application_id = 魔数`、`user_version = 1`，可添加订阅并成功刷新一次；重建前的备份文件存在且可读。
4. `durability.rs` 重写后仍能捕获「改 schema 丢行 / 丢标志」回归（先红后绿）：以 13 步旧链夹具断言「旧库被拒」；以基线库断言「新建 → 写入行与 read/starred/content → 重开」保全；并带**夹具版本数下限断言防空跑**（压平后 `for version in 1..migrations.len()` 会退化为空区间，正是 PRD 禁止的空跑形态）。
5. 其余依赖旧链的测试同步改到基线语义，不留被静默跳过的用例（已清点：`store.rs` / `tags.rs` / `search_unigrams.rs` / `retry_after.rs` / `fulltext.rs` / `feed_order.rs` / `backup.rs`）。
6. 备份恢复：旧版本备份被明确拒绝且错误可读（`application_id` / `user_version` 双判）；新基线备份可恢复。
7. 文档：README 增补「老库拒绝与升级语义」小节（不保开发库、启动提示行为、备份 / OPML 安全出口），另含发布步骤小节（版本 bump → CHANGELOG → tag → CI 产物）；与 `spec.md` 口径一致。
8. release workflow 输出三平台产物体积——用 `workflow_dispatch` 触发真实 CI 取数，**不得为拿数字先打 `v*` tag**；`cargo test --workspace` 全绿、clippy 无新增告警。

## 风险与缓解

- **误删回填**：压平后若将来要保开发库，回填函数还在（决策 3）；实现时不得顺手删。
- **等价性漏项**：部分索引的 WHERE 与触发器最容易漏 —— 用 dump 比对覆盖，不人眼。
- **提示形同虚设**：必须真的阻断（拒绝后不继续启动），否则等于静默降级；验收 2 用「拒绝后库未变」证明。
- **判定谓词写错等于全盘失效**：B1 的教训——任何「按 user_version 区间拒绝」的写法在基线=1 时都会误拒新库；实现与测试都必须以 `application_id` 魔数为准，且测试要覆盖「旧链 v=1 冻结库被拒」这一具体反例。
- **本机资源约束**（影响验收方式）：可用磁盘 49G；target 构建串行；GUI 单实例（D-Bus 名取自 `identifier`）——本变更的 GUI 相关验收（启动提示）需排在独占时段。
