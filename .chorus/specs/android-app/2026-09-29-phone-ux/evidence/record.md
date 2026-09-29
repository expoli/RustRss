# Android 手机输入与设置交互验证（2026-09-29）

Chorus task: `f225e9c5-35e1-4065-a3e4-ebb68049d345`，Android 原提案的交付后可用性修复。

## 复现与根因

用户手机截图中，底部订阅地址被键盘覆盖；设置为桌面横向标签，阅读面板同时展示字体栈、代码字体、桌面三栏宽度等参数。

独立 Android API 36 / x86_64 AVD 中安装正式 v0.2.0 APK（SHA-256 `b10b05f27f2d8d0c539cc99fb81120ba9859e735bb640617f08a5c1cdb0e0868`），打开订阅、添加地址并实际点击输入框，Gboard 弹出后输入完全被覆盖，见 `before-feed-keyboard-obscured.png`。用户原阅读设置见 `user-reading-before.jpg`。原 Activity 只使用 systemBars/displayCutout 的边距并消费所有 insets，未为 IME 留出空间；Manifest 也未声明 adjustResize。

修复将 Android 内容根视图底部边距设为系统栏/键盘底边的最大值，Manifest 声明 adjustResize，WebView 高度变化后滚动当前输入框。与 [Android 官方键盘处理文档](https://developer.android.com/develop/ui/views/layout/sw-keyboard) 的 insets/adjustResize 用法对照。

## 实现与设备检查

- 订阅入口在内容上方，显式 URL 标签、URL 输入键盘和 Enter 提交；标题栏添加直接聚焦同一输入框；空页提供可执行指引，手机日常页面不展示数据库路径和桌面标签快捷键提示。
- 设置首页为纵向分类列表，详情页有返回与关闭；系统 Back 先回分类再关闭，显式关闭会消费详情和弹层两条历史。复用原设置提交路径与主题草稿，不另建一份设置状态。
- 表单改为标签在上、输入在下，交互目标至少 44 CSS px；阅读常用项为字号/行距/段落间距，字体与代码参数放入默认收起的高级排版；手机隐藏桌面列宽/阅读布局/j-k 行为选项，以及 Android 不支持的数据库备份/恢复入口，说明 OPML 只含订阅；AI 高级参数默认收起。
- 测试脚本先通过原生触摸进入订阅并点击标题栏添加，再点击输入；后续设置导航用脚本驱动。五类输入通过实际 `adb shell input tap` 触发 Gboard，断言系统 IME 已显示，当前输入有焦点且矩形完整在 WebView 内；视口由 867 CSS px 缩至 554/599 px，见 `results.json`（22 项检查全部通过）。
- 主题保存提升持久化 revision；放弃恢复字段且 revision 不变；关闭后重开不保留未保存草稿；系统 Back 与页面返回均检查。
- 使用 CDP Emulation 在真实 Android WebView 中检查 360/412 CSS px 视口，六个设置分类无横向溢出；双语界面和 desktop-only 隐藏检查通过。视口模拟与真实设备键盘检查分别记录。
- Linux 原生桌面隔离实例运行重建后的应用，检查设置侧栏、全部阅读字段、展开的 AI 高级参数及原订阅输入位置。`desktop-results.json` 与 `desktop-settings.png` 为机械检查与 WebView 快照；该快照仅作布局辅助证据。

## 复测命令

```bash
cargo tauri android build --target x86_64 --debug --apk --ci
adb -s <isolated-emulator> install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb -s <isolated-emulator> shell am start -n tech.expoli.rustrss/.MainActivity
# 调试 WebView 的 socket 名后缀为应用 PID；只转发本机回环端口。
adb -s <isolated-emulator> forward tcp:9227 localabstract:webview_devtools_remote_<pid>
# 先关闭残留键盘；脚本会等待 JS 绑定完成并重置程序化导航留下的焦点。
ANDROID_SERIAL=<isolated-emulator> timeout 180 node scripts/verify-android-mobile.mjs <temporary-evidence-dir>
```

签名 release 构建：`cargo tauri android build --target aarch64 --target x86_64 --apk --ci`。测试 APK 与已发布 v0.2.0 分开保存；发布 tag 和已有资产不变。

## 截图

| 文件 | 可确认的内容 |
| --- | --- |
| `01-subscriptions.png` | 顶部 URL 入口和空订阅指引 |
| `02-feed-keyboard.png` | 订阅输入框与提交按钮在真实键盘上方 |
| `03-settings-home.png` | 纵向分类首页 |
| `04-reading.png` | 常用阅读项、收起的高级排版和预览、明确保存操作 |
| `05-reading-keyboard.png` | 字号输入在数字键盘上方 |
| `05b-font-keyboard.png` | 高级字体输入在实际键盘上方 |
| `06-ai-settings.png` | 单列 AI 配置与收起的高级参数 |
| `07-ai-endpoint-keyboard.png` / `08-ai-key-keyboard.png` | 地址/密钥输入可见 |
| `09-reading-en.png` | 英文阅读设置布局 |

## 验证边界

本轮验证 API 36 x86_64 模拟器/Gboard，未直接操作用户实体手机及万象拼音；Android 7–15、厂商输入法、旋转/大字体及屏幕阅读器仍需额外设备检查。没有在用户已有模拟器 `emulator-5554` 上安装、卸载或清数据。

## 独立评审

Round 1，Chorus comment `a2b3cfb2-1219-4696-9ff3-80dd8280d49c`：`VERDICT: PASS WITH NOTES`，6/6 AC 通过、0 BLOCKER。独立评审重跑 Rust/JS/严格 Clippy、原生 Linux 桌面、22 项 Android 设备检查，并展开所有 appearance/reading/AI 高级区域检查 360/412 px 无横向溢出。

评审对正式 v0.2.0 与同签名预览包进行了原生“订阅 → 标题栏添加 → 点击地址”对照，旧版被键盘遮挡、新版地址和添加按钮可见。对照截图为 `review-baseline-entry.png` 和 `review-preview-entry.png`。

保留注记 N1：本轮 API36 模拟器冷启动后，直接点常驻地址框的第一次触摸可能仅取得焦点，第二次才唤起 IME；禁用 scrollIntoView 后现象仍在。标题栏添加再点地址的路径与旧版原生输入连接行为一致。没有宣称已解决直接一次触摸唤起或用户实体手机/万象拼音的行为。

## 签名预览包交付检查

最终产品代码为 `7880c6f91499707c99bf01129776f7c0a6cbc8b9`。预览文件 `RustRss_0.2.0_android-phone-ux-preview.apk`，SHA-256 `15ab09e6ab6c76a935d0461f87d47cb69c40335c5feac343388ae7e7e3ada097`，ARM64/x86_64 通用、versionName 0.2.0 / code 2000、非 debuggable，与正式包证书一致；详见 `apk-metadata.json` 和 `apk-signature.txt`。

在独立模拟器安装正式 v0.2.0，加入本机 HTTP RSS 测试订阅并确认出现，再使用 `adb install -r` 覆盖安装签名预览包：UID 保持相同，订阅仍在。新版签名包通过实际标题栏添加/输入触摸唤起 Gboard，WebView 原生可用边界缩至 `[0,63][1080,1517]`，地址框 `[42,341][887,459]` 完整位于其内；设置首页正常。`signed-results.json` 与 `signed-*.png` 为交付检查证据。原生 UIAutomator 不一定返回输入法节点，因此使用系统 IME 显示状态、缩小的原生 WebView 边界及屏幕截图交叉确认。

修正后的完整调试验证脚本在新安装状态通过 22 项检查；`device-test.log` 保留输出。脚本等待原生无障碍节点就绪，以实际订阅/添加按钮触摸建立输入连接，再严格检查 IME、缩小高度及输入矩形，没有把失败重试记成通过。

GitHub `ci`（36526158250）与 `android-build`（36526158220）均通过产品提交 `7880c6f`。现有 v0.2.0 tag 和四项 Release 资产保持不变，此文件是后续手机交互预览包。
