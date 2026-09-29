# Windows 原生主题预览截图：实现与未完成验收

日期：2026-09-28。基线：`b7bfadae02b167a70028c88dc1585a6b22cd9090`。本页记录实现和可复验步骤；**没有 Windows 运行通过证据**。用户同日补充允许在特定环境不可用时跳过测试；Chorus 任务仅将实现、锁定 API 与可用环境回归设为必需验收，Windows 原生运行项保留为非必需且未验证。

## 实现

- `src-tauri/src/preview_capture.rs` 在 Windows 使用该窗口自己的 WebView2 `ICoreWebView2::CapturePreview(PNG)`，输出不含桌面或标题栏。截图前从 controller 读取可见状态、raw-pixel Bounds、RasterizationScale，并核对窗口可见且未最小化；截图后再次核对窗口状态。
- Bounds 像素数在发起截图前限制为 6MP；自定义 `IStream` 在 `Write`、`SetSize`、`Seek` 时限制 PNG 至 2MiB，拒绝会绕过限制的 `Clone`。超限立即返回 `ImageTooLarge`，超时后拒绝后续写入及发布；完成回调仍核对字节数、PNG 尺寸与 WebView Bounds。元数据逻辑尺寸按 raw bounds / scale 计算。此修复来自 Idea 整体验码第 1 轮 B1；原先仅在 HGLOBAL 全量写入后检查，已不再使用该无界路径。
- 共用 `theme_preview.rs` 现在对 Windows PNG 做与 Linux 相同的八格 request 标记像素核验。只有新 request/revision/hash/scene 的 ready 与实际 PNG 标记一致，才交给共用预览文件流程；不匹配会在截止时间前重试，超时不发布旧图。
- 无数据迁移；macOS 的独立适配器见 [macOS 截图记录](macos-native-capture.md)。

## 锁定 API 的核对

`Cargo.lock`：Tauri 2.11.6、Wry 0.55.1、`webview2-com` 0.38.2、`windows` 0.61.3。Tauri `with_webview` 在主线程提供 controller；Wry 0.55.1 给 controller 设置物理像素 Bounds。WebView2 的 [CapturePreview 说明](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2?view=webview2-1.0.3856.49#capturepreview) 明确写入 `IStream` 并在完成回调后读取；在首次 ContentLoading 前可能失败。Microsoft 的 [Controller3 说明](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller3?view=webview2-1.0.3856.49#get_boundsmode) 区分 raw-pixel Bounds 和 rasterization-scale Bounds；当前适配器只接受 Wry 使用的 raw-pixel 模式。Rust 函数签名已对照本机 Cargo 缓存中的精确版本源码。

可重跑的签名探针：`cargo check --manifest-path scripts/windows-preview-api-probe/Cargo.toml --target x86_64-pc-windows-gnu --locked`。探针也类型检查实际 `windows_bounded_stream.rs`，但不证明 Tauri 整体构建或运行。

## 2026-09-29 CI 原生构建

[三平台发布预检](https://github.com/expoli/RustRss/actions/runs/36504225374) 使用 `d67ff3e`：Windows 原生 release 编译、NSIS 打包与产物上传通过。该结果关闭原生构建缺口，不代表截图界面、缩放、隐藏/最小化或文件生命周期运行验收通过；这些运行项仍保持 open。

## 2026-09-28 本机验证与阻塞

- `cargo test --workspace --locked --offline`：通过。该结果只验证 Linux 分支及共用协议/文件测试。
- Windows API 签名探针上述命令：通过（精确版本、Windows GNU target；未链接或运行）。
- `cargo build --workspace --bins --example theme_fixture --locked --offline`：通过。当前 KDE Wayland 会话随后执行 `python3 scripts/verify-theme-preview.py --display wayland`，第一个 `preview_theme` HTTP 请求在客户端 10 秒截止时报 `TimeoutError`，没有得到 PNG；隔离日志和数据库在 `/tmp/rustrss-theme-preview-qzg59mud/`。日志仅显示主界面启动和 1240×820 DPR2，尚未定位预览窗口为何未返回 ready。此尝试不算 Linux 运行回归通过，也不归因于 Windows 代码。
- `cargo check -p rustrss-desktop --target x86_64-pc-windows-gnu`：无法完成；本机缺 `x86_64-w64-mingw32-gcc`，原生依赖的 `cc-rs` 构建脚本先失败。没有 Windows 编译成功结论。
- 本机是 Linux KDE Wayland；Tailscale 中两台 Windows peer `DESKTOP-D4CH2E7`、`小庞的a豆` 均离线，未发现可用本地 Windows VM。Windows 二进制指纹、三个场景 PNG、DPR/尺寸、隐藏或最小化、超时、失权、取消及 TTL/配额运行证据均**未取得**。对应 Chorus task 原生运行项保持 open；只提交实现/可用环境回归供独立评审，不作 Windows 运行通过声明。

## Windows 真机复验步骤

1. 在 Windows checkout 精确提交后执行 `cargo test --workspace`、`cargo build --workspace`；记录 `git rev-parse HEAD`、`Get-FileHash .\target\debug\rustrss-desktop.exe -Algorithm SHA256`、WebView2 Runtime 版本、Windows 版本、显示缩放和捕获 UTC 时间。
2. 按 [Linux T6 报告](mcp-theme-preview.md) 的隔离数据库和本地 MCP 授权步骤启动**新构建**的桌面产物。对 overview、article、settings 各发起新的 `preview_theme` / `capture_theme_preview`，每次读 `image_path` 的真实 PNG；记录 request ID、revision、config hash、scene、PNG SHA256、`logical_size`、`pixel_size`、`scale_factor`、PNG 字节数和到期时间。截图必须能辨认各自场景内容及不同 request 色格；不同场景的图片哈希应不同。核对 PNG 解码尺寸和内容区 viewport、DPR，包括可用的 100% 与分数或整数缩放。
3. 在预览窗口隐藏或最小化时发起截图；在截帧期间关闭窗口或让请求超时；撤销写权限并重试；检查均不公布旧 PNG。失败后取消候选 session，按 32 张/32MiB 与 600 秒文件契约复验预算和回收。保存/取消后直接读配置与文件，确认旧图可读到 TTL、取消不写正式配置。
4. 保留每张原始 PNG、MCP 请求/响应（凭据脱敏）、桌面日志、测试命令及结果 JSON，关联步骤 1 的二进制指纹。任一场景仅 ready 成功、文件存在或跨平台测试通过，都不足以关闭 Windows 运行 AC。
