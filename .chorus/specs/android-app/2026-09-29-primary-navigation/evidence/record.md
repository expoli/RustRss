# Android 一级导航与底栏验证（2026-09-29）

Chorus task: `9b4debc8-67db-4cd2-a15d-fa3399cfb47f`。
产品提交：`2b671473ea7c07192d7e09a63806154eba3a237b`。

## 复现与修复

用户提供的四张手机截图中，文章/收藏底栏文字被裁切、订阅完整，设置则覆盖底栏。
在保留的修复前 debug 构建上，将 UI 字号设为 18px，文章/收藏的布局视口变为
432×908，而实际可见视口仍为约 412.19×867.05；订阅两种视口一致。
`baseline-results.json` 与 `before-*.png` 留存对照。此构建是已有手机设置修复后的
调试包，不冒称重新下载/安装的正式 v0.2.1。

- 搜索输入设置 `min-width:0`，在工具栏剩余宽度内收缩，消除横向溢出。
- 内容、状态镜像与底栏共用 flex 布局，按实际底栏高度分配空间，移除写死的底栏
  高度和 fixed 定位。文章、订阅、收藏和设置使用同一四入口底栏。
- 手机设置复用同一 DOM/初始化/草稿收尾逻辑，移入主内容区；分类首页及详情
  都保留设置高亮的底栏，隐藏桌面模态关闭按钮。切换一级页面会关闭设置并
  丢弃未保存主题草稿；系统 Back 从详情到分类首页，再回到来源页。
- 手机设置使用 region 语义，桌面仍为 dialog/aria-modal 和 Tab 焦点循环；布局
  模式变化时同步语义。Linux 重建的原生 Wayland WebKitGTK 检查见
  `desktop-results.json`（DOM 键盘事件检查，未冒称物理键盘操作）。
- 三键导航模式补验发现 TauriActivity 默认关闭 Wry 的历史返回：设置详情直接
  返回启动器。`before-threebutton-back-results.json` 是失败证据。MainActivity
  明确启用 `handleBackNavigation`，让阅读器/设置 History 在两种系统导航模式
  下都能处理 Back。

## 设备检查

测试使用独立 Android API36/x86_64 AVD，系统输入法为 Gboard。
`three-button/results.json` 和 `gesture/results.json` 各留存 23 项检查：四个一级页面、设置详情/首页/来源页、
主题未保存草稿丢弃、真实 IME 输入可见、三个 UI 字号（14/18/24 CSS px）、
列表中间及末尾进入阅读器后的原生 Back 返回、原列表行/滚动保持，以及桌面
宽度媒体查询检查。每个几何检查均同时比较实际 visualViewport；只满足
innerHeight 而超出实际屏幕的布局视口不能通过。

列表测试使用共享 core 的 `theme_fixture` 创建 30 篇离线示例，并将其中两篇
星标；从可见行进行 adb 原生触摸，不使用点击屏幕外行的脚本伪造滚动保持。
截图由 `adb exec-out screencap` 获取，并保留 UIAutomator XML。

复测命令（仅自己的隔离模拟器；先转发当前应用 PID 的 attached WebView）：

```bash
ANDROID_SERIAL=emulator-5580 timeout 180 node scripts/verify-android-navigation.mjs /tmp/evidence --populated
```

切换系统导航模式会重建 Activity，CDP 可能短暂保留 detached 旧 WebView。
脚本选择 attached 表面，并对命令设超时，避免把旧页面的 IPC 等待当作应用失败。
第一次复用旧 AVD 时发生 System UI ANR 和 system_server 包卸载崩溃，随后改用
干净独立 AVD；失败环境不计作通过，也未修改用户 emulator-5554。

## 构建与交付

workspace Rust 测试、严格 all-target Clippy、59 个现有 JS 测试通过；日志在本目录。
重建 Android 双架构原证书签名预览包，版本仍为 0.2.1/code2001，不替换已经发布
的 v0.2.1 资产。`apk-metadata.json`、`apk-signature.log`、`apk-badging.log`、
`signed-build.log` 包含源提交、SHA256、字节数、证书、ARM64/x86_64 和非调试核对。

从下载的正式 v0.2.1 APK（安装的 base.apk SHA256 为 b0e9474f…）开始，注入
独立测试库并截图，再 `adb install -r` 覆盖为预览包：UID10218 与测试订阅保留。
`signed/results.json` 和原生截图/XML 检查四个一级页面、阅读设置详情、收藏空
列表的底栏。所有按钮都完整位于原生 WebView `[0,63][1080,2337]` 内。
还进行了实际左侧边缘滑动手势：详情→分类首页→来源收藏页，通过；这两步
与调试脚本的 KEYCODE_BACK 检查区分记录。此脚本固定测试序列和路径，运行前
必须准备隔离 AVD 与测试库，不能用于初始化真实用户手机。

边界：本轮不证明实体 ARM64 手机、厂商输入法、Android 系统字体缩放、其它
Android API 或 Windows/macOS 原生运行行为。CSS 字号测试与系统字体缩放是
不同检查；在真实 Android WebView 中模拟桌面宽度也不替代原生 Linux 检查。
