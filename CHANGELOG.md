# Changelog

## [Unreleased]

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
