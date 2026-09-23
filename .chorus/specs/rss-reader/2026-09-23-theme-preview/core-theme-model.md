# T2：core 主题模型与版本化存储

日期：2026-09-23。本页记录 T2 交付时的 core API；UI 后续已在 [T3](shared-theme-renderer.md) 接入，MCP 仍未接入。

## 模型

`crates/rustrss-core/src/theme.rs` 提供 ThemeConfig、ThemePatch、ThemeSnapshot 与 Clear/Paper/Slate 三个预设。每个预设有 light/dark 两套值，包括语义颜色、三类字体、字号/行距、列表密度/摘要/缩略图、阅读宽度/布局、圆角和栏宽。

- schema_version=1；preset_version=1 纳入 SHA256 指纹。revision 不纳入指纹，恢复相同配置可以比较相同 hash。
- ThemeConfig 记录 mode、light_preset、dark_preset、用户 overrides；切换预设默认保留覆盖项。
- patch 只改显式字段，省略保留原值。overrides 内 null 删除对应覆盖；overrides:null 清空所有覆盖；字体数组全量替换。
- 色值仅接受 #RRGGBB，规范为小写；尺寸类数字将整数与浮点形式归一化（20 与 20.0 不产生多余版本）。summary_lines 仍是整数。
- unknown field 即使值是 null 也拒绝；类型、范围、字体数量/长度/控制字符校验；patch/overrides 最大 16KiB。
- 字体是数据，未来前端必须使用逐字体引用/转义，不能把字体族列表拼成任意 CSS。core 不枚举系统字体，也不声称知道实际字体回退结果。
- 13px 正文是旧设置允许值，因此将草案中的 14px 下限改为 **13px**；新上限仍为 28px。旧值清洗/越界回退与旧桌面行为保持一致。
- 返回明暗各 8 个对比度检查项（正文、次要文字、选中正文、链接、焦点、代码正文、diff 增/删）；三预设声明检查全部通过。用户低对比配色返回 passes=false，不阻止存储。不是全面可访问性认证。

## 存储与并发

`crates/rustrss-core/src/store/theme.rs` 在现有 settings 表使用一个保留键 `ui.theme_config`，值为 `{storage_version,current,history}`。不修改 schema user_version，不操作 entries 或正文索引。

| core 方法 | 行为 |
| --- | --- |
| theme_snapshot | 一致性事务读取；没有新记录时映射旧 ui.theme/font 设置，不写库 |
| theme_history | 返回最多 10 份历史，按 revision 递增 |
| validate_theme_patch | 核对 expected_revision，返回候选/effective/hash/contrast，不保存 |
| update_theme | BEGIN IMMEDIATE → 比较版本 → 校验/应用 patch → 单次写 envelope → COMMIT |
| restore_theme | 查找历史快照，在当前版本基础上作为新 revision 保存；不倒退计数 |

默认/同值更新不物化记录、不写 timestamp、不增加版本；过期请求即使内容相同仍返回 RevisionConflict。首次有效修改保存 revision=1，并保存旧映射快照 revision=0。超过 10 份历史删最旧项，整个 envelope 上限 256KiB；最大 revision 为 JavaScript safe integer。

读取未知 schema、坏 JSON、无效历史顺序或过大记录时返回错误，不以默认值覆盖。SQL 失败回滚，当前版本与历史一起保留。两个独立数据库连接同时从相同 revision 写入，一成功一冲突。

## 调用示例

```rust
use rustrss_core::theme::ThemePatch;

let current = store.theme_snapshot()?;
let patch = ThemePatch::from_json(r##"{
  "light_preset": "paper",
  "overrides": {
    "typography": { "read_size": 18 },
    "colors": { "light": { "accent": "#875020" } }
  }
}"##)?;
let candidate = store.validate_theme_patch(current.config.revision, &patch)?;
let saved = store.update_theme(current.config.revision, &patch)?;
```

若要完整恢复某套预设，显式指定 light_preset/dark_preset 并传 overrides:null；mode 是否改为 system 由调用者明确指定。此处没有通用 set_setting 对外接口。

## 验证

- 新增 `crates/rustrss-core/tests/theme.rs`，13 项测试：六套预设与声明对比度、旧设置无写映射/非法值兼容、局部更新/继承、参数拒绝、低对比提示、只读预览/冲突、同值零写、历史上限与恢复、坏/未来记录保护、跨连接竞争/重开、SQL 故障回滚、指纹稳定性。
- `same_value_patch_does_not_write_or_advance_revision` 首次复现 20 与 20.0 导致 revision 从 1 升到 2，归一化后通过。
- SQL 故障测试装 BEFORE INSERT 触发器：同值更新成功（证明未执行 UPSERT），有效修改被触发器拒绝且记录不变；移除触发器后可继续保存。
- 运行命令：`cargo test -p rustrss-core --test theme`、`cargo test --workspace`、`cargo clippy -p rustrss-core --all-targets`。最终摘要见 manual-verification-checklist §24.3。

## 接入边界

T3 必须把旧 set_ui_theme/set_font_config 与设置读取切到这个 core 数据源，再开放新配置给 UI/MCP；不能一边读新记录、一边继续仅写旧键。T2 交付时新 API 尚无产品调用方；后续 T3 已统一 UI 入口，详见 T3 报告。

T3 继续做真实组件/CSS token 映射与阅读位置保护；T4 做设置重组；T5/T6 再接 MCP 权限、同步和图片闭环。T1 跨平台余项独立保留。
