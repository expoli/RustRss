# B′ 有效标签实施与验证（2026-10-05）

起点：实际 master HEAD `b2eebd8`（任务书 `6dfef8d` 之后的聊天修复已在主线）。
契约：2026-10-04 用户批准的 **有效标签 = 手动 ∪ 源继承**；关联独立存储，日报依旧只按源标签 OR 选源。

## 消费点逐项确认

- [x] `fill_entry_tags` / `entry_tags_map`：列表、搜索与 `get_entry` 共用有效投影；tag_id 去重，按名称 NOCASE / id 稳定排序，`TagBrief.source = manual | feed | both`。
- [x] `append_entry_filter`：三档排序的首屏/续页均过滤有效 ID；外层继续钉既有排序索引。
- [x] `mark_view(Tag)`：批量目标同一有效 ID 集；目标 read/id 从覆盖索引读，不加载正文；范围外条目不变。
- [x] `TAG_ROW_SELECT` 族现收口到 `tag_row_select()`：list_tags / recent / tag_row 全部按有效集合计未读，仍走 `idx_entries_unread_id`，**未读未延期**。
- [x] `tag_entry_count`：总数、删标签预览与执行同一有效集合计数；删除显式清两种关联，保留文章。
- [x] 写 API：assign/unassign 仍只改变文章手动关联；移除源关联保留同源文章原有手动关联，不能单篇屏蔽继承。
- [x] MCP `meta_out`：保持 tags 名称数组兼容；新增可选 tag_sources（同序前 20 项的 id/name/source），只有有效集合 >20 才截断。重叠20项不误报截断；25项混合来源按名称取前20。
- [x] AI 工具门面共享上述 core 投影自然获得有效标签；经 supervisor 批准仅改共享输出字段，不改聊天循环/降级/费用业务。
- [x] UI 文章选择器：继承/双来源 disabled + 来源文案 + title，鼠标/Enter 共用写保护；手动可编辑。打开时回读最新条目。
- [x] 源打标成功：回读列表/阅读 chips、选择器缓存和侧栏计数，重置分页/总数；列表同 ID 的行保留、chips 局部 patch，阅读 DOM 保留；提交中关闭选择器仍更新数据，迟到阅读回读不覆盖新阅读项。日报/聊天视图不被文章列表覆盖。
- [x] 新增 inherited/both/inheritedHint 的 en/zh-CN key 成对；日报 design 旧条款、spec checkbox、README 与人工验证边界已回填。

## 迁移决定 / SQL 约束

**无需迁移**：v2 已有 `idx_feed_tags_tag(tag_id,feed_id)` 覆盖索引，及 `idx_entries_feed_read`；复用现有索引，无 v4，也不改 v3 DDL。条目投影按 ID 读 feed_id 用已有 v3 `idx_entries_scoped_search_order` 覆盖索引。

有效 ID SQL 统一由 `effective_tag_ids_sql` 生成：手动腿钉 `idx_entry_tags_tag`，继承腿先钉 `idx_feed_tags_tag`，CROSS JOIN 钉 `idx_entries_feed_read`，UNION 只投影 ID。计数、列表过滤与批量目标复用；未读外层钉部分覆盖索引。

本机 SQLite 的 UNION 可采用 `MERGE (UNION)`，继承腿会有 **仅 ID 去重** 的临时排序；测试只允许它出现在 effective_ids 的 materialization 内，禁止外层文章 ORDER BY 排序/裸扫。不是放开正文临时排序。变异测试删除任一 UNION 索引后，三档 × 首/续页、总数、侧栏未读、两种批量方向都必须无法建立计划；既有未读索引变异测试保持。

## 测试与先红后绿

- `explain-before.log`：实现前 3 个 B′ 测试**先红**（缺来源、缺 feed 覆盖腿、删除 feed 索引未转红）。
- `explain-after.log`：同 3 个测试全绿（包含 SQL 同源计划断言/删索引变异）。core tags 全集 20 个测试，不删旧测试。
- `workspace.log`：`cargo test --workspace` **620 passed / 0 failed / 1 pre-existing ignored**（任务书的 588 基线落后于实际 HEAD；本任务新增 3 个 Rust 测试，无新增 ignored）。
- `mcp.log`：`cargo test -p rustrss-mcp` **118 passed / 0 failed**，包括 HTTP 安全、元数据范围、tag_tools；更新现有截断测试验证继承/双来源/20边界/25截断/取消手动不屏蔽。
- `js.log`：`node --test scripts/tests/*.test.cjs` **99 passed / 0 failed**，新增 `tags.test.cjs` 10 个；`node --check ui/app.js` 通过。
- `clippy.log`：`cargo clippy --workspace --all-targets -- -D warnings` **零警告**。
- `build.log`：`cargo build --workspace` 通过；前端变更后重建真实产物再运行 UI 探针。
- `git diff --check` 通过；仓库原有全量 rustfmt 基线不一致，未将无关格式重写纳入本次改动。

## 大库 OS 冷页缓存测量

`cold.json`：12,000 篇、471,326,720 bytes（450 MiB）自有 fixture；正文双大列，每列约 11.5KB。
手动3,000、继承6,000、重叠3,000，有效集合6,000；3轮/查询。每轮 fsync + POSIX_FADV_DONTNEED，mincore 确认 **0 resident pages** 后再开库；不触及用户库或全局 drop_caches。

Debug 产物；冷 = OS 数据库页缓存，不宣称控制器物理冷盘；open 与 SQL 分开报告。200条/页包含有效标签填充；续页游标在驱逐前计算，不把首屏查询混入续页计时。

| 查询 | 冷 SQL 首次 ms（3轮区间） | 热 SQL 均值 ms |
|---|---:|---:|
| 有效总数（删除预览同源） | 16.62–17.50 | 11.60 |
| 单标签未读（侧栏同源） | 16.79–25.23 | 13.00 |
| newest 首屏 / 续页 | 19.86–20.15 / 15.67–17.97 | 13.61 / 12.04 |
| oldest 首屏 / 续页 | 15.98–17.12 / 16.15–17.04 | 12.25 / 12.39 |
| unread_first 首屏 / 续页 | 20.76–23.11 / 21.03–21.76 | 13.56 / 13.82 |

开库首次另计约110–191ms（见 cold.json）；批量只做 SQL 同源计划验证，不将包含正文/F​TS 写入的 mark_view 耗时伪装成目标选择 SQL 耗时。

重现（已存在 fixture 时不要 init 覆盖；只使用自有目录）：

```bash
mkdir -p target/verification-followups
cargo build -p rustrss-core --example scope_counts
target/debug/examples/scope_counts init target/verification-followups/effective.sqlite
# 隔离库添加继承重叠（不碰用户库）
python3 - <<'PY'
import sqlite3
with sqlite3.connect('target/verification-followups/effective.sqlite') as c:
    c.execute('INSERT INTO feed_tags(feed_id,tag_id) SELECT id,1 FROM feeds WHERE id<=60')
    for k,v in [('mcp.enabled','false'),('ui.language','zh-CN')]:
        c.execute('INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(?,?,0)',(k,v))
PY
rustc --edition 2021 .chorus/specs/rss-reader/2026-10-04-mobile-followups/evidence/tags-b-prime/query-probe.rs \
  -L dependency=target/debug/deps --extern rustrss_core=$(ls -t target/debug/deps/librustrss_core-*.rlib | head -1) \
  -o /tmp/rustrss-effective-tag-probe
python3 .chorus/specs/rss-reader/2026-10-04-mobile-followups/evidence/tags-b-prime/measure-cold.py
```

## 重建桌面真实 IPC / DOM 证据

`ui.json` / `ui.log`：Linux Xvfb + 系统 WebKitGTK，private HOME/数据目录 + 自有 fixture 快照；探针走 DOM 点击的真实事件处理与 Tauri IPC，没有往应用添加测试钩子。

- 源继承、双来源中文 title/标记 + native checkbox.disabled 读回；继承英文 From feed/title 读回，i18n key-set 自检通过。
- 文章100的双来源项点击不移除手动关联（SQLite回读）；取消 feed60 的标签后有效总数6000→5900，未读5899、首屏200条、首项5900，过滤结果与列表chips一致。
- 原5900行的 DOM identity 保留；阅读6000的 DOM identity 保留但继承chips清空；取消 feed1 的标签后文章100仍手动关联，选择器恢复可编辑。
- 真实运行二进制 SHA-256：`8779887e72640ab7f63e9510c453653c2cf623bd85d3d62ceb0a752c959f943b`。

```bash
cargo build --workspace
PYTHONDONTWRITEBYTECODE=1 /usr/bin/python3 .chorus/specs/rss-reader/2026-10-04-mobile-followups/evidence/tags-b-prime/verify-ui.py
```

边界：这是 **Inspector DOM 操作 + 真实 IPC/SQLite 回读**，不是 native pointer/TalkBack/截图验收。Android 实体/AVD、320px/大字号、Windows/macOS、native Wayland 未验证，未勾这些跨平台项。其它手机跟进（跳源/退订/日报底栏）不在本任务。未改日报范围或 AI 聊天业务；共享门面新增来源字段是 supervisor 明确批准的最小例外。独立 reviewer gate 由主会话负责。
