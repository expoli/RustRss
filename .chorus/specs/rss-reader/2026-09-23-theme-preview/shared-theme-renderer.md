# T3：真实 UI 接入与共享主题渲染

日期：2026-09-23。产品 UI 已接入 T2 core；设置重组、MCP 配置与预览会话仍分别留到 T4/T5/T6。T3 提交范围包含本页记录的实现与验证脚本。

## 用户可见行为

- 设置 → 通用 → 外观增加 Clear / Paper / Slate。选择预设同时切换明暗预设，并清除外观覆盖；提示文字明确说明这一行为。明暗/跟随系统仍是独立选择。
- 字体的空选项改为「跟随主题」，正文 13–28px、行高 1.3–2.2。旧设置读取保持 T2 的兼容映射；首次修改才写 `ui.theme_config`，保留旧键用于诊断。
- 原 `set_ui_theme` / `set_font_config` 与新 `update_ui_theme` 共用 core 模型与事务。旧控件只合并自身字段；预设选择携带 expected_revision，冲突时重读并报错，不覆盖新配置。
- 字体、栏宽、密度等变化按当前可见段落/列表行补偿滚动；颜色变化只更新对应变量。异步字体完成时校验当前 generation、节点连接及滚动位置，避免拉回用户新滚动的位置。

## 实现边界

`ui/theme.js` 是共享应用器；core 返回明暗有效值，前端不另算预设。`system` 用媒体查询监听切换完整调色板，系统变化不写数据库。旧响应 revision 小于当前值时丢弃主题部分。

`ui/components.js` 提取真实列表内容、正文头部/正文容器、侧栏视图/订阅行模板；输入由生产调用方 escape/sanitize，组件不接受未清洗的订阅 HTML。示例复用这些模板、生产 `index.html` 设置结构、`style.css`、i18n 与代码高亮库。

| core 语义组 | 渲染映射 |
| --- | --- |
| background/sidebar/panel/selected/hover | `--bg` / `--bg-sidebar` / `--bg-panel` / `--bg-active` / `--bg-hover` |
| text/muted/accent/border/focus/danger/star | `--fg` / `--fg-dim` / `--accent` / `--line` / `--focus` / `--danger` / `--star` |
| code 与 diff | `--code-*` / `--diff-*`，包含背景、正文、语法 token 与增删行 |
| typography | 三类安全引用字体栈、UI/正文/代码字号、正文行高 |
| list | 行 padding、摘要 0–3 行、缩略图显示属性 |
| reader/chrome | 正文最大宽度、段间距、focus 紧凑导航、圆角、栏宽；900px 窗口收缩栏宽 |

主题应用不重建列表、侧栏或正文 DOM；不触碰 entries SQL，也不为主题取正文大列。焦点布局保留侧栏与列表可达，不隐藏导航。

**缩略图限制**：生产列表当前没有缩略图元数据。本次只实现已有缩略图元素的显示样式，并用本地 fixture 图片验证开关；不宣称真实订阅已支持缩略图，也不为此逐篇获取正文。

设置弹层的滚动条穿透在像素断言中先失败（Clear 浅色面板应为白色，实际得到灰色滚动条像素）；改用参与文档层叠的主题滚动条后，X11/当前 Wayland 的 12 张设置场景均通过遮挡像素检查。

## 复现

```bash
cargo test --workspace
node --test scripts/tests/*.test.cjs
cargo clippy --workspace --all-targets
cargo build -p rustrss-desktop
cargo build -p rustrss-core --example theme_fixture
python3 scripts/verify-theme-desktop.py
cargo build -p rustrss-desktop --example theme_ui --features snapshot-probe
python3 scripts/verify-theme-ui.py
python3 scripts/verify-theme-ui.py --scale 2
python3 scripts/verify-theme-ui.py --display wayland
```

`theme_ui` 仅 opt-in example：固定资源协议，直接嵌入真实 UI 文件，不连接数据库/MCP。39 张截图包括三预设 × 明暗 × 双语 × 总览/正文/设置 36 张，以及 focus、窄窗正文、窄窗设置。请求字体栈可以记录，不能凭 computedStyle 宣称逐字形的实际字体。

`verify-theme-desktop.py` 则运行重新构建的正式桌面二进制：新建小样本库，检查启动无写入 → UI 选择 Paper → 调大字号 → SQLite 回读 revision 1/2 → 重启保持与背景像素。两类证据分别覆盖共享组件和实际 IPC，不相互替代。

## 验收及未验证部分

- Rust 386 / Node 22 项通过；clippy 仅原有 3 条告警。
- Xvfb 100%/200% 与当前 KDE Wayland 各 39 张截图、23 项行为检查、24 项像素检查通过。
- 正式桌面夹具最终回读：Paper、25px、revision=2；重启保持。
- 留存示例：[Paper 浅色](t3-paper-light.png)、[Slate 深色](t3-slate-dark.png)（来自原生 Wayland 组件 fixture）。


最终机器结果见 [T3 结果摘要](theme-ui-results.json)，机械验证与现场记录见验证清单 §24.4。截图与完整日志保留在结果记录的 `/tmp` 目录，可通过以上命令复现；临时路径不是可分发的产品预览接口。

未验证：Windows/macOS、GNOME Wayland、真实系统明暗切换（媒体查询分支有 Node 测试）、跨屏分数缩放、逐字形字体回退、主题变化期间延迟远程图片重排。UI 稳定性证据来自固定本地素材，不覆盖任意第三方网页。

T4 将重组设置与补齐可视化编辑/历史恢复入口。MCP 配置工具、外部进程更新通知/轮询、预览会话/版本握手/保存取消仍未接入；T3 不包含这些产品能力。


### Xvfb 环境隔离补验

两个测试脚本的 Xvfb 子进程均剔除 `GDK_BACKEND`、`WAYLAND_DISPLAY`、`EGL_PLATFORM`，避免继承 KDE 会话的 Wayland 强制选择。产品代码没有设置或覆盖这些变量。原生 Wayland 分支保留真实会话配置。

```bash
GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-invalid EGL_PLATFORM=wayland python3 scripts/verify-theme-ui.py
GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-invalid EGL_PLATFORM=wayland python3 scripts/verify-theme-desktop.py
```

修复后两条命令均通过：组件 probe 为 39 captures / 23 checks / 24 pixel checks；正式桌面 revision=2、Paper、restart_preserved=true，字号实际读回 25px。字号由滑块点击位置决定，脚本只断言比预设 18px 大且重启保持，不把 24/25 当固定值。历史首次记录的 24px 是当轮观测，最终 T3 记录为 25px。GTK 启动失败且无结果 JSON 时，组件脚本会给出日志路径和退出码。
