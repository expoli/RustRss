# T1：Tauri 原生截图适配器探针

日期：2026-09-23。状态：Linux/Xvfb 与当前 KDE 原生 Wayland 集成验证；Windows/macOS 未验证。产品主题系统与 MCP 图片工具仍未接入。

## 实现范围

- `src-tauri/src/preview_capture.rs`：Tauri with_webview → WebKitGTK snapshot → PNG。当前只由 example 引用，没有注册到正式应用。
- `snapshot-probe` feature 显式启用可选 Linux webkit2gtk/cairo PNG 依赖；版本匹配现有依赖，不增加浏览器内核。
- `theme_snapshot` example 使用独立 identifier、离线资源与受限 CSP，没有插件、数据库或 MCP 服务。输出目录由 CLI 指定且必须不存在；不能从网页指定任意路径/URL。
- 截图前检查窗口状态和物理像素预算（≤6MP），回调再检查实际像素；PNG 写入器在每次写入前检查累计上限（≤2MiB）。传输尚未实现，base64 限制留给 T6。
- GUI 线程只取得 native surface、检查尺寸并复制有界像素；工作线程重建本地 Cairo surface 编码，GTK/Cairo 对象不跨线程传递。
- deadline 覆盖调度、截图和编码；drop 取消 native request，迟到回调不发布结果。PNG 写入也检查取消，后台编码在下次写回时停止；不承诺可瞬间抢占 Cairo 内部计算。
- 探针控制器演示 revision 前后核对和 CAS single flight；生产 preview_id/session/CAS 状态机仍由 T6 实现，不能把探针当成完整 MCP 安全边界。
- 不支持的平台返回 `unavailable`；对应原生适配器仍待实现/编译/运行验证。

## 复现

```bash
cargo build -p rustrss-desktop --example theme_snapshot --features snapshot-probe
/usr/bin/python3 scripts/verify-theme-snapshot.py --scale 1
/usr/bin/python3 scripts/verify-theme-snapshot.py --scale 2
```

需要系统 WebKitGTK、Xvfb/xvfb-run、Python Pillow。runner 建立新的 HOME/XDG 临时目录，用独立 Xvfb 运行已构建的产物；不改 GDK_BACKEND，不访问用户数据库。程序有 120 秒 watchdog，runner 有外层 timeout；正常退出由 xvfb-run 回收显示进程。产物目录通过 stdout 给出，含 PNG、运行日志和 JSON。

开发 fixture 独立于生产 UI，包含三栏、中英文、长标题、代码和不同主题颜色；它验证截图链路，不用于声称共享生产组件已经完成。示例中的技术文本不进入产品 i18n。

## 验证项目

1. 每个缩放档位在同一窗口连续应用 100 个不同 revision，等待 fonts.ready + 两次 rAF，再请求 capture。
2. 每张 PNG 实际解码：核对唯一 revision 标记像素、正文背景色、物理/逻辑尺寸、scale factor、PNG 字节限制；100 张 SHA256 各不相同。
3. 隐藏窗口、像素超限、编码字节超限、零 deadline、极短异步 deadline 返回预期错误。
4. 原生请求挂起期间更改 revision，完成结果被拒绝；并发第二请求返回 preview_busy；之后可继续截图。
5. 已销毁窗口返回 window_unavailable。关闭发生在截图进行中的所有时序尚未穷举。

证据：[tauri-snapshot-results.json](tauri-snapshot-results.json)，保留两档最终运行摘要、PNG 哈希与原始产物路径。计时从调用截图到探针写盘完成，包含窗口查询、native capture、编码及 PNG 写盘；不含字体/两次 rAF，也不是正式客户端完整闭环耗时。

## 本轮发现及修正

200% 缩放下，hide→show 后立即做预算验证，偶发返回 window_hidden。检查 Tao 0.35.3 源码发现 set_visible 只发送 WindowRequest，最小化状态由 GTK window-state-event 更新。诊断日志进一步确认：is_visible=true、is_minimized=false 之后，截图 precheck 又收到 minimized=true；旧 ICONIFIED 事件晚于“已显示”查询送达。

因此 show 返回和一次状态查询不足以表示绘制恢复。探针先等待可见状态，再以有上限重试取得成功 native frame；适配器完成编码后也重新检查可见/最小化状态。没有通过固定 sleep 将失败隐藏。失败运行产物留在 `/tmp/rustrss-tauri-snapshot-2x-8odab6ok` 与 `/tmp/rustrss-tauri-snapshot-2x-jjjqcpay`；最终证据以结果 JSON 列出的目录为准。

## 局限与接续

- 初始验证环境为 Linux Xvfb 软件渲染 100%/200%；后续补充当前 KDE 原生 Wayland（见下），不能推广为所有 Wayland 合成器、真实 WM 最小化、125%/150%、硬件加速全部通过。
- 两次 rAF + 本批像素断言通过不构成所有合成器/延迟字体/图片/负载下的新帧保证；T3/T6 需继续做真实 fixture 验证。
- Windows/macOS：只有接口调研，本批保留明确 unavailable；未新增未经编译的 native 实现。
- 未运行全量测试；本批完成 example 构建、专项运行验证，以及默认 workspace 构建。
- 下一步 T2：core 主题参数、三预设、旧设置映射、版本化存储；保留上述跨平台待办，T1 总体验收不勾选。

## 当前 KDE Wayland 会话验证

用户明确说明当前桌面为 Wayland 后，直接使用会话 `/run/user/1000/wayland-0` 运行探针。仅子进程移除 DISPLAY 防止回退 XWayland；不设置 GDK_BACKEND，不设置 GDK_GL 或 GDK_SCALE，HOME/配置/缓存仍隔离。

```bash
cargo build -p rustrss-desktop --example theme_snapshot --features snapshot-probe
/usr/bin/python3 scripts/verify-theme-snapshot.py --display wayland
```

结果见 [wayland-snapshot-results.json](wayland-snapshot-results.json)：

- GDK 运行时类型为 **GdkWaylandDisplay**，不是仅依据 XDG_SESSION_TYPE 推断。
- 100/100 帧 marker 与背景像素匹配，PNG 哈希各异；8 类边界检查均通过。
- WebView 逻辑内容区域 1000×653，GTK/WebView scale 与 JS devicePixelRatio 均为 2，PNG 2000×1306；耗时中位数 45.18ms、最大 60.48ms，最大 PNG 153267 bytes。
- `kscreen-doctor -o` 显示此会话 eDP-1 为 150%、HDMI-A-1 为 110%；这是 compositor 输出缩放，不能等同于 GTK 的整数 backing scale=2。本轮没有切换屏幕配置，未验证跨屏移动或每个输出的全部行为。

**发现并修复：窗口尺寸不是内容尺寸。** 原实现从 Tauri window.inner_size 推导 logical_size，在有真实窗口装饰的 Wayland 下得到 1052×752，而 native PNG 仅包含 1000×653 的 WebView。现改为在 UI 线程读取 WebView allocated_width/height/scale_factor，按内容尺寸执行像素预算；回调核验 surface 尺寸，再返回对应元数据。前端同时报告 innerWidth/innerHeight/devicePixelRatio，runner 做独立对照。初次失败证据目录 `/tmp/rustrss-tauri-snapshot-wayland-22vsfzf8`，修复后目录见结果 JSON。

仍未验证 GNOME Wayland、跨屏/分数缩放清晰度、真正的最小化/恢复、输入法和生产 MCP 闭环。本页只验收截图适配器，不宣称完整应用在所有 Wayland 环境均通过。
