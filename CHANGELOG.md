# Changelog

## [0.5.1] - 2026-10-03

### 改动

- Android 签名 APK 改由 CI 在推 `v*` tag 时自动构建并附加到 Release：签名材料经 repo secrets 还原（密钥本体仍不入库），构建后断言发布证书指纹，桌面三平台与 Android 同一工作流出四资产。本版无应用代码变更，是发布链路的端到端验证版。

## [0.5.0] - 2026-10-03

### 改动

- **「报纸」（Print）成为默认视觉主题**：纸墨配色加一处编辑部红（暗色为夜印暖黑 + 朱红），列表标题与阅读正文改用衬线（16.5px/1.85），报头 2px 墨线为全页唯一粗线，选中行改为「批注」语义（红竖线 + 红标题，无色块）；新增第四套预设 Print，Clear / Paper / Slate 保留可选，明暗可分别选择，所有值仍可覆盖（油-ui 流程：四方向小样对比 → 用户选定 → 三轮独立评审 7.3 → 8.6/8.5）。
- 阅读头操作行合并：标签栏并入操作行（桌面受限单行展示、溢出裁切，触屏独立整行保 48px 触控目标），阅读操作改无框安静语法；文件夹分组头不再占用强调色边，红竖线专属「选中」语义。
- 设置对话框布局与交互一致性修正：主题历史恢复行加可见标签、示例明暗切换并入示例卡头、代理区表单网格与全局统一、数据面板去掉双层重复标题、MCP 轮换 token 按钮降为次级样式、数据库位置新增「复制路径」、导航摘要单行省略且明暗同预设去重。

### 修复

- 分隔线不再继承弱化文字色：运行时 1px 分隔线此前取 muted（暗色下实测满屏重线）；新增主题自适应 `--control-border`（亮 3.4:1 / 暗 4.2:1）供开关轨道与表单控件边界，装饰性分隔线保持 hairline。
- 默认窗口尺寸下工具栏按钮文字换行成两行（实测截图缺陷）。
- 主题数字输入改 `step=any`：Print 默认值本身为 16.5px / 375px，固定整数步长会 stepMismatch 拒掉合法编辑。
- MCP 客户端配置框空态加说明、空内容时禁用复制；标签增删不再移动正文锚点。

## [0.4.1] - 2026-10-02

### 修复

- **Android 刷新全灭修复**：reqwest 0.13 的 `rustls` feature 默认验证器是 rustls-platform-verifier（Android 上需先做 JNI 初始化），未初始化时每次 HTTPS 抓取任务都 panic——设备日志实锤 121 个订阅 117 个逐个 panic、进度停格 4/121、报告缩水为 4 个。现三平台统一改为 `tls_certs_only(webpki_root_certs())` 纯根存储验证（内置 Mozilla 根集 DER），回归红线 7 的 webpki-roots 口径；MCP preview 客户端同口径。

### 改动

- Android「诊断日志」入口从设置→数据移至设置→通用，与日志级别同区。

## [0.4.0] - 2026-10-02

### 新增与改进

- Android 设置→数据新增「诊断日志」：列出应用沙盒 `logs/` 下的 `rustrss-*.log`（名称/大小/修改时间/当前标记，最新在前），查看默认加载末尾 256KB、超出时明示截断（截断上限由后端响应直出，文案不与常量漂移），并经系统文档选择器（CREATE_DOCUMENT）导出完整文件——release 包无需 adb/root 即可取证排障（此前排障只能靠盲猜）。
- 新增只读日志 API（列表/末尾/整读）：文件名白名单守卫（`rustrss-*.log` + 拒绝分隔符与 `..`）在打开任何文件或对话框之前防住路径穿越；整读设 16MB 软上限，超限可读报错不写盘；末尾窗口起点恰在行边界时保留完整首行。日志脱敏仍沿用写入口径，导出原样不二次加工。
- 诊断日志入口仅 Android 显示；桌面不新增入口（命令跨平台编译，行为一致）。

### 验证与限制

- workspace 测试（含 logging 38 个用例与命令守卫/结构测试）与严格 Clippy 全绿；`cargo build -p rustrss-desktop` 嵌入重建通过；i18n 双语言 581 键 selfTest 通过；mobile 分支 `aarch64-linux-android` cargo-check 干净。
- 命令与 UI 任务独立评审、Idea 聚合代码复审均通过（评审备注中的 i18n 常量漂移与行对齐丢行两项已在同批修复）。
- Android 实机运行时核验（块可见性、截断提示、导出三态）待人工完成后回填 durable spec；Windows/macOS 仍无实机验证记录。

## [0.3.1] - 2026-10-01

### 修复

- 统一 Windows 预览截图与 Tauri/Wry 的 WebView2 COM 依赖线，避免 `webview2-com` / `windows-core` 跨版本类型混用导致 NSIS 构建失败；`windows-core` 保留为直接依赖（`#[implement]` 宏展开按本包 extern prelude 解析 `::windows_core::…`，删除即 CI 复红）。

### 发布工程

- Tauri 固定在 2.12.x 次版本线，Windows 平台依赖改为兼容范围；`windows` / `webview2-com` / `windows-core` 三者同处 0.62 / 0.39 依赖线。
- 普通 CI 新增 Windows 专属库编译，在打标签前捕获 WebView2 类型漂移；桌面与 Android 工作流统一使用 Tauri CLI 2.12.1。
- 新增版本一致性检查，校验 Cargo workspace、Tauri 配置、release tag 与 Android APK `versionName` / `versionCode`。

### 验证与限制

- Linux 上 workspace 测试与严格 Clippy 通过；Windows NSIS 与三平台安装包仍需以同一提交的远程 preflight 结果为准。

## [0.3.0] - 2026-10-01

### 新增与改进

- 双端统一四主导航：文章、订阅、收藏、设置在桌面与 Android 收敛为一致的一级目的地；未读/全部筛选收进列表标题旁，星标与稍后读收进收藏页。
- 共享阅读视觉基线：桌面与 Android 阅读页共用同一套排版、操作布局与视觉规范；阅读动作精修（菜单失败可见、缩略图偏好记忆、分页返回状态与无障碍标签）。
- 手机操作面板：订阅与标签管理改为底部动作面板，整理动作（上移/下移、刷新、编辑、移动分组、退订）按危险级别分组并保留确认级联；阅读与设置触控目标加大。
- 列表工具收敛：搜索改为一击展开输入并明确全库范围，排序、隐藏已读、批量标读收进列表「更多」菜单。
- 设置与窗口体验：MCP 写 token 复制/轮换/清除入口统一，旧库启动拒绝屏支持只读导出 OPML，桌面窗口最小化/最大化/关闭与快捷键帮助原位验证。

### 修复

- Android 主导航与视口尺寸统一；菜单删除后焦点回到触发项或存活行；RSSHub 订阅添加恢复。
- 侧栏计数/角标同值短路，消除重复 DOM 写入；设置分类摘要换行堆叠，不再挤压导航行。

### 验证与限制

- 本轮按逐动作迁移台账（M01–M20）推进，T2–T7 在 Android 16（API 36）模拟器与 Linux 桌面（WebKitGTK，Xvfb/KWin）最终产物上完成 Android 三旅程、Linux 同进程 18 项旅程与 49 项设置复测，含星标路径 200 行性能测量。
- 部分桌面组合场景（星标/稍后读子视图、未读计数切换等）未在最终组合单独复测，沿用 T3/T6 证据；Windows/macOS 本轮改动无实机验证记录。
- Android APK 沿用原发布签名，可覆盖安装保留数据。

## [0.2.1] - 2026-09-29

### 修复

- 统一桌面与 Android 应用图标为 Ferris＋RSS 图案；同步 Windows/macOS/iOS 衍生资源，补齐 Android 圆形和自适应图标，并在 CI 校验资源一致性。

- 修复 Android OPML 导入选择器对普通 MIME 类型的 `.opml` 文件禁用问题；移动端按文档内容校验，桌面过滤保留。

- Android 输入时同时避让系统栏和软键盘，订阅地址入口移至页面上方。
- 手机设置改为分类列表与单页详情；优化系统返回、表单触控布局和保存操作，高级排版及 AI 参数默认收起。

### 验证与限制

- Android 签名包沿用原发布证书，可覆盖安装；本轮手机布局、OPML 选择和启动器图标在 API36 x86_64 模拟器中验证。
- 用户实体手机、第三方输入法/文档提供者与其他启动器尚未实测；Windows/macOS/iOS 启动器实际展示尚未验证，iOS 仅统一图标资源。
- 四平台安装资产与来源以本版本 Release 为准；macOS arm64 dmg 仍未签名。

## [0.2.0] - 2026-09-29

### 新增与改进

- Android 侧载应用：手机导航与阅读界面、订阅管理、刷新、离线阅读与搜索、收藏和稍后读、系统分享、OPML 导入导出及设备浏览器打开外链。
- Android AI：OpenAI、Anthropic、Gemini 与 Ollama 的配置、连接测试、摘要和翻译；密钥使用 AndroidKeyStore 管理的加密存储。
- 主题与 MCP 预览：新增 Windows WebView2 和 macOS WKWebView 原生截图适配；补齐 Linux KWin Wayland、Openbox X11 场景截图，以及 agent 读图后调整、保存和取消的验证证据。
- 截图资源限制：Windows/macOS 在输出或编码过程中执行单图 2 MiB 上限，超限、超时或失效结果不会发布。

### 修复

- Android 共享应用入口与桌面专属服务隔离、外链浏览器交接及 CI 构建配置。
- AI 传输错误的敏感信息清理，以及 Android Ollama 服务地址提示。
- macOS 截图依赖的 AppKit 特性配置，以及 PNG 转换代码的严格 Clippy 检查。

### 已知限制

- Windows/macOS 原生编译与桌面打包已通过 CI；截图界面、缩放、隐藏/最小化、超时及文件生命周期仍待真机运行验收。
- GNOME Wayland、物理跨屏、真实最小化和第三方 GUI MCP 客户端等仍保留待验项。
- 桌面 Release 提供 Linux deb（Ubuntu 24.04 基线）、Windows NSIS 和 macOS arm64 dmg。macOS 包未签名，首次打开需右键选择「打开」。
- Android Release 提供使用项目发布密钥签名的 ARM64 / x86_64 通用 APK（Android 7.0+）；构建与侧载步骤见 [开发文档](docs/development.md)。

[0.3.1]: https://github.com/expoli/RustRss/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/expoli/RustRss/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/expoli/RustRss/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/expoli/RustRss/compare/v0.1.0...v0.2.0
