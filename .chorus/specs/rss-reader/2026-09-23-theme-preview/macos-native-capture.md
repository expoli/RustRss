# macOS 主题预览截图：实现与待验收证据

状态：适配器已实现；**macOS 构建和运行验收未执行，原生运行项保持 open**。2026-09-28 用户补充允许在特定环境不可用时跳过测试；Chorus 任务因此仅将实现、API 核对及可用环境回归设为必需验收，原生运行项保留为非必需且未验证。本记录不以 Linux 回归、Rust 目标标准库或 API 文档代替 macOS 实机结果。

## 实现边界

- `src-tauri/src/preview_capture.rs` 在 Tauri UI 线程取得当前预览 `WKWebView`，用 `WKSnapshotConfiguration` 的 WebView bounds 与 `afterScreenUpdates` 请求内容区域图像。像素预算在调用 WebKit 前校验；回调检查实际位图尺寸和 PNG 字节数。窗口隐藏/最小化、空图、尺寸不符均失败。
- 超时后接收端关闭；迟到的回调不会把图像交给 MCP。现有 `theme_preview.rs` 的 ready request/revision/hash/scene/mode 校验和 PNG marker 像素校验也应用于 macOS。旧帧无法仅凭 ready 信号通过。
- 保存、取消、CAS、临时文件配额与 TTL 继续使用现有 MCP/core 状态机；此改动没有添加另一套持久化路径。

## API 核对

当前 `Cargo.lock` 锁定 Tauri `2.11.6`、Wry `0.55.1`、`objc2-web-kit` / `objc2-app-kit` / `objc2-foundation` `0.3.2`、`objc2` `0.6.4`、`block2` `0.6.2`。Tauri `PlatformWebview::inner()` 在 macOS 返回 WKWebView 指针；相应的 `objc2-web-kit 0.3.2` 绑定声明 `takeSnapshotWithConfiguration_completionHandler` 和 `WKSnapshotConfiguration` 的 `rect` / `afterScreenUpdates`。核对来源：

- [Apple WKWebView 截图](https://developer.apple.com/documentation/webkit/wkwebview/takesnapshot%28with%3Acompletionhandler%3A%29)、[截图配置](https://developer.apple.com/documentation/webkit/wksnapshotconfiguration)
- [Apple NSWindow backingScaleFactor](https://developer.apple.com/documentation/appkit/nswindow/backingscalefactor)、[NSBitmapImageRep CGImage 初始化](https://developer.apple.com/documentation/appkit/nsbitmapimagerep/init%28cgimage%3A%29-7o5tz)、[PNG 表示](https://developer.apple.com/documentation/appkit/nsbitmapimagerep/representation%28using%3Aproperties%3A%29)
- 本机下载的锁定 crate 源码：`tauri-2.11.6/src/webview/mod.rs`、`objc2-web-kit-0.3.2/src/generated/WKWebView.rs` 和 `WKSnapshotConfiguration.rs`、`objc2-app-kit-0.3.2/src/generated/NSBitmapImageRep.rs`。

## macOS 实机会话步骤

在一台可访问 GUI 的 macOS 11+ 主机上签出**最终提交**，安装 Rust、Python Pillow，随后：

```bash
cargo build --workspace --locked
cargo build -p rustrss-core --example theme_fixture --locked
python3 scripts/verify-theme-preview.py --display macos
```

脚本应输出私有临时目录，保存 `results.json`、三预设 × 明暗 × 三场景 PNG、MCP stdio 桥接截图及桌面日志；`results.json` 记录 OS 版本、WebKit bundle 版本、构建二进制 SHA-256、逻辑/像素尺寸与比例。验收者仍需打开 overview/article/settings 的 PNG，确认可见内容与场景对应，并核对 `results.json` 的 scene、revision、config hash 与三组图像。比较时忽略顶端 8 像素的新鲜度色块。至少在 1x 与 2x 可用显示模式各运行一次；记录实际显示模式和 `devicePixelRatio`。复核隐藏/最小化、超时、失权、版本冲突、保存/取消、32 张/32 MiB 配额和 600 秒 TTL；将具体命令、错误码、SQLite 回读及截图归档到此目录后，才能关闭 AC。

## 当前缺口

实施主机是 Linux；Tailscale 当前无在线 macOS peer，未发现 macOS 远程桌面或 Xcode。`aarch64-apple-darwin` Rust 标准库可安装，但从 Linux 交叉 `cargo check` 停在 `objc2-exception-helper` 的 Objective-C 构建：本机 `cc` 不识别 `-arch` 与 `-mmacosx-version-min`。因此本轮没有 macOS 二进制、PNG、OS/WebKit 版本或真实失败路径证据；只提交实现/可用环境回归供独立评审，原生运行项不作通过声明。
