# T6 桌面与平板汇合：原生证据

任务 `5b2be699-b3e2-4da1-9675-17983b209b21`。所有订阅、文章、主题和 OPML 均来自临时 SQLite/HOME/XDG 数据目录或任务自有 Android AVD `RustRssT6Tablet` (`emulator-5586`)。未触碰用户数据库、钱包、`emulator-5554`；没有发布包、标签或 Chorus 验收操作。

## 产物与方法

- Linux WebKitGTK `target/debug/rustrss-desktop`：`cargo build -p rustrss-desktop --locked`，SHA-256 `82fea6d84c2326cc7a5f835ac087116e8f44f7b07156df6bdd73b65fdc847b8a`。桌面布局、原生输入、同一进程旅程、视觉十二格和窗口探针均核对此哈希。脚本分别为 `scripts/verify-reading-experience-t6-{desktop-layout,desktop-input,journey,visual,window}.py`。
- Android x86_64 debug APK `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`：`cargo tauri android build --target x86_64 --debug --apk`，SHA-256 `fb7836a6bd2cc313bff06aefcd5bf57d15e49d2c92e1e0895edecb9b31aeeb2b`，安装于任务 AVD。脚本 `scripts/verify-reading-experience-t6-tablet.mjs` 逐尺寸记录包哈希、AVD 型号、Android 36、物理分辨率、密度、WebView CSS 尺寸/DPR/粗指针、截图，并在 `finally` 恢复任务 AVD 的 `wm size` 与 `wm density`。
- AVD 使用 Android Pixel Tablet 硬件 profile，密度 320 dpi、DPR 2，测试时通过 `wm size` 设置相应真实模拟屏分辨率；这是平板 AVD 中运行的原生 Android WebView，不是把手机网页窗口拉宽。Linux 桌面窗口下限为产品配置的 900 CSS px，故 849/850/851 断点由平板 AVD 验证，桌面从 920 px 起验证。模拟器不能代替实体平板或 Windows/macOS。

## 逐 AC 结果

| AC | 原生执行与读回 | 状态及范围 |
|---|---|---|
| 1 布局 | [Linux 几何](desktop-layout-results.json)记录真窗口 920/959/960/961/1049/1050/1051/1280/1440 px：三栏都在屏内、无横向溢出/重复手机导航，标题栏与设置模态框可见。截图含 [1280](desktop-1280.png)、[1440](desktop-1440.png)、[920](desktop-920.png)。[平板八尺寸](tablet-results.json)的 768/849/850/851/959/960/961/1024 CSS px 均有截图 `tablet-<宽度>.png`；960 及以下四项底栏、961/1024 三栏，Android 桌面窗口键隐藏、无溢出。1024 px 打开数据设置后，MCP/整库备份/恢复入口隐藏，OPML 导出及仅订阅说明可见。 | 在两类真实原生运行环境通过；没有将平板判作 Linux 桌面。1024 px Android 仍无桌面窗口按钮。 |
| 2 输入/窗口 | [20 项 Linux XTest 输入](review-desktop-results.json)使用可信原生 Tab/Shift+Tab/Enter/Escape、`?` 和 `/`，覆盖 More、右键源菜单、设置模态焦点进入/退出/归还；Escape 取消后数据库零写。[窗口记录](window-x11-results.json)显示 XWayland 标题栏最大化/还原、最小化到 `isMinimized=true` 和关闭退出码 0。 | **部分**：真实 Wayland 与私有虚拟 KWin 中，按钮及直接 `window_minimize` IPC 均返回成功，但 Tauri `isMinimized=false,isVisible=true`；不能宣称 Wayland 已最小化。[真实 Wayland](window-real-results.json)、[虚拟 KWin](window-results.json)。XWayland 检测到最小化，随后 inspector 调用 `unminimize()/show()` 未读回恢复，因未通过用户任务栏激活，恢复也标为部分。 |
| 3 汇合旅程 | [单进程 17 项](journey-results.json)从本地 HTTP RSS 添加源并落库、新建/移动到文件夹并回读 ID、最旧排序落库、搜索打开文章、星标/稍后读独立落库、创建并附加标签、取消搜索返回列表；在长文滚动 420 px 后保存聚焦模式与 760 px 阅读宽度，正文同一节点和所选条目保留、正文锚点 `115.6875→115.6875`。阅读宽度草稿取消、外观预览零写/保存字号/取消零写均有库读回。最后在**同一进程**通过原生 GTK “Save File” 选择器导出 [OPML](journey-export.opml.xml)，解析 XML 含新源及所在文件夹。 | 通过 Linux 隔离夹具。选择器由 XTest 鼠标/键盘完成，不是调用导出 core 函数伪造；[选择器截图](journey-opml-picker.png)及[聚焦阅读截图](journey-focus-layout.png)保留。 |
| 4 视觉/覆盖值 | [十二格结果](visual-results.json)和 `visual-<预设>-<明暗>-<语言>.png` 为 Clear/Paper/Slate × light/dark × en/zh-CN。每格在原生 WebKitGTK 打开文章，核对原生主题快照各对比度阈值、应用模式/语言、无水平溢出/重复底栏、Settings 焦点。通过 IPC 保存栏宽 220、列表宽 320、UI 字体族/16 px、compact 列表和浅色自定义强调色 `#1455a0`；CSS token、SQLite 配置和进程重启后读回一致。 | 通过 Linux Xvfb 原生产物；十二格是运行截图，不覆盖 Windows/macOS/实体显示器或感知清晰度。 |
| 5 文档/清单 | 本记录、[逐动作矩阵](../../action-matrix.md)、README 与手工清单回填产物、平板方法、范围和限制。`cargo test --workspace --locked` 全通过；`cargo clippy --workspace --all-targets --locked -- -D warnings` 通过；`node --test scripts/tests/*.test.cjs` 61/61；UI/设备脚本语法与 `git diff --check` 通过。 | 本任务保留已知部分项及 T7 最终组合复验；未勾选 OpenSpec T6 任务或执行 Chorus 自检/提交。 |

## 桌面动作与限制

[T6 桌面逐 ID 矩阵](desktop-action-matrix.md)区分本轮实际读回、仅入口/先前任务证据与未重测；主矩阵对应行引用此记录。当前 T6 直接读回的主路径为添加源、文件夹移动、标签附加、搜索/最旧排序、阅读星标/稍后读、设置草稿/宽度/聚焦和 OPML 导出。M18 托盘/日志目录/外链与 M08 桌面拖拽没有在本轮做真实系统侧效果核验，仍是部分或待验；T5 的独立 MCP/备份/旧库证据不冒充本轮执行。Windows/macOS 没有原生机器，此处不宣称验收。

`window_minimize` 直接 IPC 在真实 Wayland 返回 `ok`，但 1.5 秒后 Tauri 状态仍是 `minimized=false,visible=true`；同一二进制在真实 KWin XWayland 则报告 `minimized=true`。这缩小到后端/窗口状态问题，尚未证明 Wayland compositor 是否实际收起窗口；未根据返回 `Ok(())` 推断可见效果。上游 [Tauri issue #15458](https://github.com/tauri-apps/tauri/issues/15458)仅作为相近诊断背景，不能证明本问题的根因。该项应由独立审查决定是否阻断 T6。

## 未提供原生机器的平台复验步骤

**Windows / WebView2**：① 从同一提交构建桌面包，记录二进制/安装包 SHA-256，并用隔离用户配置和合成 SQLite/本地 RSS；② 在 1280/1440 px 真窗口和系统 100%/200% 缩放下记录三栏/聚焦、断点旁最小可用宽、设置模态与 12 格主题截图；③ 用真实键盘/鼠标复跑 Tab/Shift+Tab/Enter/Escape、右键/更多、标题栏最小化→任务栏恢复/最大化/关闭；④ 单进程完成添加源至 OPML 系统保存对话框，解析 XML 和数据库，保留窗口/截图/哈希。

**macOS / WKWebView**：① 同提交构建并记录应用包/可执行文件 SHA-256，使用独立测试帐户和合成数据；② 在 1280/1440 px 真窗口与 Retina 缩放下拍摄三栏/聚焦及 Clear/Paper/Slate × 明暗 × 中英截图，核对覆盖值重启保持；③ 使用实体键盘/触控板验证快捷键、Tab/Shift+Tab/Enter/Escape、上下文菜单和原生窗口最小化→Dock 恢复/最大化/关闭；④ 同一进程完成订阅→阅读/保存→设置草稿→系统 OPML 保存对话框，读回 SQLite/XML 并保存窗口状态与截图。两平台结果须独立标记，不能由 Linux 或 Android 模拟器推断。
