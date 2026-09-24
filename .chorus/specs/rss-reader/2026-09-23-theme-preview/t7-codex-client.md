# Codex 客户端闭环补验

2026-09-24，基线 `76e6c05`，用户自行启动 Codex CLI 0.156.1，原生 Wayland 隔离 fixture。仅配置进程级回环 HTTP MCP 与六个主题工具，没有修改全局客户端配置或用户订阅库。

## 已验证

- 用户确认：Codex 终端只显示文本/图片标记，但模型能描述画面。读取用户指定的 Herdr 客户端记录，模型描述了侧栏计数、代码块、表格，以及两版正文和引用的换行变化。此项证明模型收图，不代表终端向用户显示了实际图片。
- Paper 浅色文章从 18px/720 调至 22px/540，同一候选升至 revision2。临时阶段直接读取 SQLite，正式主题仍未落库。
- 保存后正式 revision1，Paper/light、正文22px、宽度540；桌面日志确认应用。随后 dark 候选取消，affected0，正式配置不变。
- 提取指定客户端会话中三张原始 PNG，均1920×1280，约316–322KB。模型称重拍只有顶部色条，但实际 PNG 完整；重拍与第二版只在左上64×8新帧标记有像素差。没有证据表明服务端裁图，客户端/模型为何这样描述仍未确认。

## 实际客户端发现与修复

首次 preview 请求把 patch 发成 JSON 字符串，服务端拒绝；客户端改为对象后完成闭环。原始 tools/list 的 patch schema 是不受约束的 `true`，并非明确的 string 类型。现让 validate_theme、update_theme、preview_theme 直接复用 core 的 patch schema，发布对象、可选字段、null、枚举及边界；反序列化与存储路径不变。

新增 `theme_patch_tool_schemas_publish_the_core_object_contract` 通过真实 HTTP tools/list 检查三个工具与 core 同源。修复前明确失败（type 为 Null，预期 object）；修复后通过。移除 schema 标注将恢复同一失败条件。

- `cargo test --workspace`：406 passed / 0 failed，26段。
- `node --test scripts/tests/*.test.cjs`：40 passed / 0 failed。
- `cargo clippy --workspace --all-targets`：EXIT0，仅原有3条 core 告警。
- `cargo build --workspace`：EXIT0；新二进制 stdio tools/list 确认三个 patch 都声明为 object 且 additionalProperties=false。

[机器证据](t7-codex-client-results.json)包含审计、SQLite 回读、图片哈希/尺寸/像素差和测试结果。没有重复截图矩阵或墙钟长测。

## 未验证与下一步

修复后的 schema 已经 HTTP 测试及新 Codex 会话验证，patch 显示对象结构，validate/preview 对象参数首次成功。旧会话的 string 声明为历史观察；首轮成功路径含一次对象参数重试。Pi、第三方 GUI 图片显示及其它平台未测。重拍图片在模型侧的解释异常尚未归因。

可复测：运行 `/usr/bin/python3 scripts/start-theme-wayland-probe.py`，从生成的 instance.json 读取回环端口，按本轮 launcher 的进程级 MCP 配置连接；要求客户端直接发送对象 patch，完成 preview → 修改同一候选 → capture → save，再新建候选 cancel，并回读隔离 SQLite。客户端需刷新工具目录；观察客户端图片与模型描述应分别记录。

用户客户端仍引用临时工作目录，因此保留该目录，助手创建的隔离桌面已停止，用户终端会话未关闭。本轮临时 PNG 与测试日志已清理；磁盘收尾约70G可用。

## 同会话补验（10:47–10:49）

用户授权向下方 Herdr pane 发送提示词，在同一隔离库恢复新版服务后执行。服务 HTTP tools/list 已直接核对三个 patch 均为 object。

| 调用 | 模型本次视觉观测 |
| --- | --- |
| preview 18px/720 | 完整可读，引用及表格完整 |
| capture revision1 article | 仅看见色条，保留异常 |
| preview 22px/540 | 完整可读，正文换行、表格下移 |
| capture revision2 article | 完整可读，换行及表格位置一致 |
| capture revision2 settings | 完整可读，预设边框和字号可辨 |

主助手重新解码本次五张会话原始 PNG，均1920×1280。两组 article preview/capture 的像素差都仅限左上64×8标记，正文逐像素相同。因此模型所说第二组“行间距看起来更大”没有原始像素证据支持，第一组仅见色条也不能归为服务端裁图。视觉解释/传递异常仍未定位；不能否定已成功的 preview 和后两次 capture。

服务日志六次操作均成功，无 save；SQLite 直接回读正式配置仍 revision1、light/Paper、22px/540，与首轮保存值完整一致。本轮没有落地新的 PNG 文件，没有重跑构建或测试。当时新会话 schema 目录转换尚待验证，已由下节关闭。

## 新会话最小复测（10:53–10:55，基线 3c203de）

用户同意后，在已退出旧客户端的下方 pane 启动全新 Codex 会话，重新加载同一隔离服务工具目录。模型实际工具声明中 validate_theme.patch 和 preview_theme.patch 均为对象结构；客户端对嵌套 overrides 显示 unknown | null，没有展开全部嵌套约束。完整校验仍由 core 执行。

- validate 与 preview 直接传对象（19px/680），均首次成功、无重试。
- preview 完整可读；capture 再次被模型描述为仅见色条。
- 主助手解码本会话两张原始 PNG，均1920×1280、约317KB；像素差仍仅64×8新帧标记，正文完全相同。异常尚未定位，不归因为服务端裁图。
- cancel 成功，无 save；客户端比对 config/theme/history 全部一致，主助手直接回读 SQLite 也与此前正式配置完全一致。

新会话 schema 缺口关闭；重拍视觉解释问题、GUI 图片展示及其它平台仍保留。隔离桌面已停止，用户客户端工作目录暂留；没有生成临时 PNG 或重复构建/全量测试。此节记录新会话补验；后续文件输出契约见 [文件输出报告](mcp-preview-files.md)。
