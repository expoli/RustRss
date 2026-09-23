# WebView 原生截图：可行性与证据

日期：2026-09-23。结论：三平台存在原生 API 路径；Linux 独立 GTK WebView 探针已成功。**Tauri 适配器、MCP 图片传输与跨平台运行仍未验证。**

## 路径核对

| 平台 | 拟议实现 | 当前证据 |
| --- | --- | --- |
| Linux | Tauri with_webview → PlatformWebview.inner() → WebKitWebView.get_snapshot(VISIBLE) → Cairo PNG | 当前 Rust 绑定含 snapshot/snapshot_future；本机 WebKitGTK 2.52.6 实际截图成功 |
| Windows | with_webview → controller.CoreWebView2 → CapturePreview(PNG, IStream, callback) | 官方 API 可用；本机无 Windows 运行环境，未编译/实测 |
| macOS | with_webview → WKWebView → takeSnapshot(configuration, completionHandler) → NSImage/PNG | 官方 API 可用；本机无 macOS 运行环境，未编译/实测 |

Tauri 暴露平台对象，需要按目标平台添加匹配版本的直接依赖与条件编译；当前代码没有可直接调用的跨平台截图适配器。保持系统 WebView，不添加 Chromium，不覆盖显示后端环境变量。开发探针使用 Xvfb 只是验证环境。

官方来源（2026-09-23 查阅）：

- [Tauri 2.11.6 with_webview](https://docs.rs/tauri/2.11.6/tauri/webview/struct.Webview.html#method.with_webview)
- [WebKitGTK get_snapshot](https://webkitgtk.org/reference/webkit2gtk/stable/method.WebView.get_snapshot.html)
- [WebView2 CapturePreview](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2#capturepreview)
- [WKWebView takeSnapshot](https://developer.apple.com/documentation/webkit/wkwebview/takesnapshot(with:completionhandler:))

## Linux 探针

运行命令（仓库根目录）：

```bash
GDK_GL=disable timeout 45s xvfb-run -a -s '-screen 0 1440x1000x24' \
  /usr/bin/python3 .chorus/specs/rss-reader/2026-09-23-theme-preview/snapshot_probe.py \
  .chorus/specs/rss-reader/2026-09-23-theme-preview/mockup.html \
  /tmp/rustrss-theme-snapshot-probe
```

用可见 GTK WebView 加载离线交互稿，清爽浅色 → 纸页深色 → 石墨浅色，重复两轮。每次更改实际主题并放置相应颜色标记；等待 fonts.ready + 两次 rAF，经消息确认 revision 后调用原生内容截图。验证 PNG 可解码、标记像素匹配、内容颜色数量 >100，并记录哈希。

主机缺少 Python GI 的 gi._gi_cairo 转换模块，首次探针停在 surface 转换；最终探针直接通过 ctypes 调用原生 finish/Cairo PNG 编码，不安装/修改系统包。产品计划使用 Rust 绑定，此探针不是产品依赖。

最终证据见 [snapshot-results.json](snapshot-results.json)：

- 6/6 捕获成功，3 个主题分别得到不同哈希；同一主题两次哈希一致。
- 逻辑窗口 1280×900，实际 PNG 为 2560×1800；需显式记录缩放与限制输出尺寸。
- 每张 489,262–506,432 bytes；主题应用至保存截图耗时 242.22–305.39ms。时间包括 PNG 写盘及探针图片检查，不是纯 API 延时，也不是冷启动/生产性能承诺。
- 输出 PNG `/tmp/rustrss-theme-snapshot-probe/1.png` … `6.png`；文件名与 JSON revision 对应，可用脚本重新生成，图片未提交仓库。
- 探针未连接数据库，没有读写用户订阅或主题；由 xvfb-run 回收自建显示进程，Gtk 窗口退出销毁。

## 证据边界与后续执行

| 项目 | 状态 | 下一步 |
| --- | --- | --- |
| Linux 原生截图与相邻主题切换 | 已验证 | T1 移入隔离 Tauri example，重复同一断言 |
| 两次 rAF 后总能得到最新合成帧 | 未证明 | 100 次快速切换 + 延迟字体/图片 + 负载压力，检查标记和主题区域 |
| Windows/macOS 截图 | 当前环境无法运行验证 | 对应主机编译并验证明暗/缩放/超时/窗口隐藏 |
| Linux Wayland、最小化、遮挡 | 未验证 | 原生 Wayland 会话及有 WM 的 X11 复测；不要由 Xvfb 结果推断 |
| 真实生产组件一致性 | 未验证 | T3 抽共享组件，fixture 与主界面使用同一渲染器/CSS |
| MCP image 响应与客户端视觉闭环 | 未验证 | T6 读配置→preview→看图→修改→save/cancel，HTTP/stdio 各测 |
| 对比度/字体完整性 | 未验收 | 测试具体色对、缺字体与 CJK fallback；当前截图仅为设计讨论稿 |

设计决定：先实现可见的独立预览窗口。隐藏窗口/无桌面渲染没有足够证据，首版不承诺。图片返回前必须版本握手、像素尺寸闸门和有效权限检查。
