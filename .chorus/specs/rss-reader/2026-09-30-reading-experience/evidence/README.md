# T1 共享视觉基线与证据边界

Chorus task `9427496f-cda4-4c27-b2ae-46858f791e9b`，规划源为 Idea `0d7ce904-be25-47ae-8fa6-ba652d0c9146` / Proposal `b5585be7-a1d3-4311-8c1b-ce458ad649d6`。改动前产品 commit：`bc680d5155b7c6afcee355fe6aa9bc74ef456031`。实施仓库是 RustRss；Chorus 工作区的 `concept.html` 是沟通示意，不是运行截图。

## 固定隔离数据

`crates/rustrss-core/examples/reading_experience_fixture.rs` 仅接受不存在的数据库路径和 30/200 两个行数。执行示例：

```bash
cargo run -p rustrss-core --example reading_experience_fixture -- /tmp/rustrss-reading-30.sqlite 30
cargo run -p rustrss-core --example reading_experience_fixture -- /tmp/rustrss-reading-200.sqlite 200
```

数据使用 `example.invalid`，不包含真实订阅、正文或凭据。30 篇/200 篇均有中英混排、每七篇一篇长标题、半数有图、独立的已读/星标/稍后读状态、代码、表格与 80 段长文；三个订阅源（含长名称）和 24 个文件夹。200 行用于列表节点身份、同值零写与性能比较。主题 fixture 原有 `theme_fixture` 仍可用于 UI/MCP 预览专项；本批没有修改 core 主题配置结构。

## 改前图片来源

- Android 2026-09-29 已签名预览包的 `signed/preview-articles.png`、`preview-reading.png`、`preview-settings.png` 位于 `.chorus/specs/android-app/2026-09-29-primary-navigation/evidence/`；详见同目录 `record.md`。它们是历史设备基线，非 T1 改后 APK 截图。测试数据和视口可能与新 fixture 不同，T7 必须再用同场景重建产物获取前后对照。
- 桌面公开示意 `docs/images/app-overview.png`、`docs/images/settings.png` 为 1440×900，使用虚构数据，只能作旧布局参考。
- `before-articles.png` / `before-settings.png` 是设置页和 System UI ANR 环境失败图，不能作为文章页面的视觉基线；`record.md` 已明确这一限制。

## 本轮可用运行环境

- 本机有 Linux WebKitGTK 和私有虚拟 KWin Wayland；T6/T7 使用解包到 `/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb` 的私有 Xvfb 运行原生 Linux 二进制，完成六套内置明暗组合及最终 12 格中英矩阵。
- `adb devices -l` 在 2026-09-30 显示 `emulator-5554` 在线（Android 16、1080×2400、420 dpi、三键导航）；它是既有会话，所有权未确认，本任务不改装或停止它。独立 Android APK、手势/三键、IME 和系统字体检查属于 T7；T1 的 CSS 尺寸证据不冒称原生命中测试。
- Windows/macOS 与物理 Android 设备未在本机提供，运行状态保持未验证。

## 证据索引

- `t3-list/record.md`：同 30 行夹具中英/浅深、360/412 手机真机 WebView 前后截图与首标题坐标；200 行排序/计数/重试/刷新、搜索返回锚点、系统导航/横屏/大字体检查和本轮签名通用 APK 哈希。
- [T7 最终集成记录](t7-final/record.md)：最终 APK/Linux 哈希、原生旅程、同夹具前后矩阵、手机/平板/无障碍/性能及剩余平台限制。
- `action-matrix.md`：M01–M20 共 159 行，逐动作记录平台、副作用、证据与未测范围；T2–T7 按行回填。
- `fixture-results.json`：30/200 行隔离库的实际条目、文件夹、状态、长标题、图与代码/表格计数；数据库留在 `/tmp`，可按上面命令重建。
- `contrast.json`：从 core 的六个内置解析快照计算文字、焦点、按钮、边框对比度；不会改写任意用户自定义颜色。测试方法：sRGB 线性化、(Lmax+0.05)/(Lmin+0.05)。
- `theme-ui.json` 与同目录 `theme-*.png`：重建 Linux WebKitGTK 的真实组件矩阵。本批私有虚拟 KWin Wayland 的 12 个预设/模式/语言单元各有 3 个不同场景帧；`theme-preview.json` 与 `preview-*.png` 记录真实 MCP 预览、临时零写、MCP 保存/取消及失权回收。Wayland 下未验预览窗本地关闭取消（报告 `local_cancel=false`）。`probe.log`、`preview.log` 留原命令输出。
- `touch-360x800.json` 与同名 PNG：Chrome 独立静态样式夹具，CDP 打开 360×800、`pointer: coarse` 后量 15 个可见目标的 CSS 矩形，包含阅读器添加标签和设置开关的完整点击区域；这是桌面浏览器的触控媒体查询检查，不等于 Android 原生坐标或 TalkBack 验收。
- 独立评审首轮发现 `.tag-add` 和设置 `.switch` 低于 48×48 CSS px；修复后两者在 `touch-360x800.json` 中均为 48×48，原生组件/设置/预览矩阵已使用重建的 `theme_ui` 重跑。Android 原生验证仍留后续任务。
- `theme-settings.json` 与 `settings-*.png`：共享设置编辑器在真实 WebKitGTK 上完成 23 项检查，覆盖系统模式、预览零写、保存/丢弃、历史恢复、CAS 冲突、Aa 共享字段和正文锚点。
- `cargo test --workspace`、`cargo build --workspace`、`cargo clippy --workspace --all-targets` 及 60 个 JS 测试在本批通过；`theme_ui` 要显式用 `--features snapshot-probe` 构建。证据文件里的 `binary_sha256` 应与重建产物核对。运行矩阵没有替代 Android APK 设备验收。
- 后续 `theme-mcp`、设置、构建和完整测试结果须附命令、退出码、二进制哈希；旧轮次报告仅作背景，不当成本轮通过。
