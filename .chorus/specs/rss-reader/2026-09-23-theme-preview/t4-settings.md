# T4 设置重组、外观与 Aa 验收

日期：2026-09-23。范围：T4；T7 未开始。机器数据见 [results](t4-settings-results.json)。

## 实现

- 七类：外观与主题、阅读、订阅与更新（含 RSSHub）、AI、外部集成（MCP）、数据与备份、通用（语言/窗口/日志/关于）。原非主题控件 ID 与处理器保留。
- 外观：明暗预设分别选择，保留用户覆盖；显式整套预设操作清空覆盖。全部语义颜色、界面字体/字号、列表密度/摘要/已有缩略图开关、圆角/栏宽可编辑。
- 阅读与正文 Aa：同一字段表、同一 core CAS 保存路径；字体/字号/行高/代码字体/正文宽度/段落间距/阅读布局。字体名支持逗号分隔，Linux 提供现有 fontconfig 枚举建议。
- 输入暂存于内存；core validate 驱动局部文字/代码示例，保存才写库并应用正式 UI。继承单项、取消、恢复历史、冲突留草稿；没有新增任意 CSS/HTML 或网络预览入口。
- 当前视图批量已读/未读在列表头 ✓ 菜单，继续调用原 markAll(scope) 与确认流程；列表头固定两行，长计数不会挤掉视图标题。Aa 紧邻星标/稍后读/AI。
- 设置方向键/Home/End 切分类、Tab 焦点回环、Esc 关闭并返回入口；Aa 使用原生 dialog 的模态焦点管理。

## 验收证据

- `cargo test --workspace`：405 passed / 0 failed，26 个结果段。
- `node --test scripts/tests/*.test.cjs`：33 passed；新增 8 项覆盖共用字段、稀疏 patch、草稿零写、校验迟到/错误、清全部再改单项、七类及入口唯一性、切换语言前的草稿保护。
- `cargo clippy --workspace --all-targets`：exit 0，仅原有 fulltext.rs:130 / store/mod.rs:478,697 三条告警。
- 双语 self-test：468/468 keys。新 JS 与 fixture 脚本语法检查通过。
- 修改 UI 后重建：`cargo build --workspace --features rustrss-desktop/snapshot-probe --example theme_ui --bins`；隔离数据生成器另用 `cargo build -p rustrss-core --example theme_fixture`。
- `python3 scripts/verify-theme-settings.py`：生产编辑器/renderer/markup + 隔离内存 core IPC，12 checks / 3 native captures。覆盖临时零写、明暗独立预设、CAS 冲突、历史恢复新 revision、Aa/阅读互读；保存后正文/列表节点身份不变，段落偏移差 <2px；900×600 窄窗无页面横向溢出。此探针不是完整 app.js 的自动化。
- `python3 scripts/verify-theme-desktop.py`：真实重建桌面、全新 fixture SQLite、Xvfb。原生键盘逐一切 7 分类并验选中色像素，验证焦点回环；原生点击 Aa 改 23px，草稿阶段无 theme_config，保存 revision=1，正文渲染次数 1→1，重启配置未改变。脚本初次因选中色测试常量误写失败；核对 core `#e2e8f4` 后修正脚本，最终 exit 0。
- 静态 ID 对照：旧 UI 移除项仅旧主题/字体控件与两个批量操作按钮（已由共享编辑器/列表菜单替代）；AI/MCP/RSSHub/备份/日志等入口保留。

截图：[真实桌面外观页](t4-appearance.png)、[共享 Aa 暗色面板](t4-aa-reading.png)。图片为固定本地数据，不包含用户订阅。

## 磁盘与清场

`cargo clean` 前：937G 总量、869G 已用、21G 可用、98%；删除 68049 文件，共 64.4GiB。清后：811G 已用、79G 可用、92%。最终编译/测试后约 824G 已用、66G 可用、93%。

仓库仅保留两个明确命名的验收 PNG 与 JSON/文档。自身进程均已退出；临时目录清理被环境执行策略拒绝，未绕过。约 17MiB 本轮证据目录及辅助脚本/日志暂留 `/tmp`；不删除其它轮次数据。Cargo 重建产物保留，不追加 clean。

暂留目录：`rustrss-theme-settings-{ia5xq73p,vdi0o4o6}`、`rustrss-theme-desktop-{v4xbqdhq,z2v6i8fr,56frme76,ltnmq51t}`；辅助文件为本轮 `/tmp/rustrss-t4-*`。仓库 `scripts/__pycache__` 是清前已有的空目录，本轮未生成 pyc，未提交。

## 未验证与后续（归 T7，未执行）

- T4 本轮仅 Xvfb 1×运行。Wayland 的 T6 本地取消真实点击、正文段落位置断言明确列入 T7；未用 Xvfb 结果代替原生 Wayland。
- 30 分钟绝对寿命与 10 分钟空闲墙钟长测，以及 RSS/磁盘增长观察留 T7；本轮没有长测或冷盘性能结论。
- 完整三预设×明暗×三场景原矩阵未重复批跑；T3 fixture 已适配新结构并做语法检查。原生 select/字体建议弹层在不同系统主题、读屏器、分数缩放、Windows/macOS、其他 Wayland compositor 尚未验收。
- 真实第三方 MCP 客户端图片展示仍待 T7；本轮未变更 MCP HTTP 安全/网络/截图生命周期实现。
- Chorus 保持本地任务草案 + 文档镜像，正式 proposal/task 仍未物化；不宣称已完成独立聚合评审。T7 等用户另行指示。
