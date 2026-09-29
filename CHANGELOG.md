# Changelog

## [Unreleased]

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

[0.2.1]: https://github.com/expoli/RustRss/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/expoli/RustRss/compare/v0.1.0...v0.2.0
