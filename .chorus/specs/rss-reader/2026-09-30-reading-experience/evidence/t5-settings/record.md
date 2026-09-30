# T5 设置、表单、确认与反馈：实现和验证记录

任务 `b36221e4-0d51-46e8-a7a5-111e33894ebc`；本记录只覆盖隔离 fixture、任务自有 Android AVD `RustRssT5` (`emulator-5584`) 和独立 Xvfb 桌面进程。未使用 `emulator-5554` 或真实用户数据库。T4 的证据仍在 `evidence/t4-reader/`，没有复用为本任务的设备通过项。

## 产物与复现

- Android x86_64 **debug** APK：`src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`，SHA-256 `1295b53869f9db87d66ba81eb78b38f55e8646e8c5e6da49daa02b13a236492c`。用 `cargo tauri android build --target x86_64 --debug --apk` 重建，`adb -s emulator-5584 install -r ...` 安装；本任务未签名发布包、打 tag 或发布 Release。
- Linux WebKitGTK 二进制：`target/debug/rustrss-desktop`，SHA-256 `db89edc2b81655164064f2e5d895db36aa4ad1fee54c1255bea14311154c813d`。用 `cargo build -p rustrss-desktop --locked` 重建。
- `cargo test --workspace --locked` 全部通过；`node --test scripts/tests/*.test.cjs` 61/61；三个 UI JS 文件的 `node --check`、桌面验证脚本的 `py_compile`、`git diff --check` 通过。
- [Android 分类、返回、错误 JSON](android-results.json)（32 项）、[Android 主题/AI/RSSHub JSON](android-followup-results.json)（20 项）、[真实横屏 JSON](android-landscape-results.json)、[原生副作用 JSON](android-required-results.json)（14 项）、[手工原生读回](android-manual-results.json)、[RSSHub 迁移](android-rsshub-migration.json)、[离线重试](android-offline-retry.json)与 [SAF 最终轮次](android-saf-results.json#L27)对应上面的最终 APK 哈希；[桌面 JSON](desktop-results.json)（44 项）和 [旧库原生验收](legacy-refusal-runtime-results.json)对应上面的 Linux 二进制哈希。复现入口是 `scripts/verify-reading-experience-settings-{android,followup,rotation,required}.mjs`、`scripts/verify-reading-experience-settings-desktop.py` 和 `scripts/verify-legacy-refusal-ui.py`；桌面每次创建独立 HOME、XDG 与数据库。

本机 Maven Central 对新依赖返回 HTTP 403，构建时短暂把阿里云 Maven 公共镜像加入生成的 Gradle repository 列表；APK 构建后已恢复该文件，git 无此变更。此差异只影响本机获取依赖，未改变 APK 源码。完整网络可达的 CI 可直接使用现有 `mavenCentral()`。

## 逐 AC 结果

| AC | 执行与结果 | 范围界限 |
|---|---|---|
| 1：分类、原字段/动作与反馈 | 最终 APK 上六类分类/详情、摘要、Back 和 Android 支持的原控件通过 [32 项设备检查](android-results.json)；重建 Linux 上七类分类/详情及窄窗口桌面动作通过 [44 项检查](desktop-results.json)。M13–M20 逐行动作状态见[矩阵](../../action-matrix.md)。 | 部分旧动作只证明入口，矩阵标为部分或待验。 |
| 2：主题草稿/CAS、非主题错误 | Android 草稿零写、详情 Back 保留草稿、离开设置丢弃、保存修订、六预设/三模式、单项继承/全清除/颜色覆盖、历史刷新/恢复、陈旧 CAS 拒写均有读回。代理非法 URL 输入保留且字段错误关联，数据库未变；修正后保存。MCP 无效端口的 UI 和 Rust IPC 均拒绝且数据库未变。 | UI 字体、列表密度等字段已保存读回，重启视觉对照仍按矩阵标部分。 |
| 3：代理/RSSHub/AI、凭据、数据/通用/MCP | Android 代理/绕过/刷新字段保存，RSSHub 合成 `/version` 测试及单个旧域候选预览→取消→归一化→重复预览，四家 AI provider/model 保存，Ollama 合成 HTTP 测试，Android Keystore 密钥状态与清除。最终 APK 通过 SAF 导入 feed ID 5、导出四个合成源。Linux MCP 监听、鉴权、token 生命周期、权限确认与取消均通过；GTK 数据库备份/恢复及旧库拒绝→只读 OPML 导出→退出均通过。 | Linux 私有 Secret Service 会话中 ksecretd 注册服务，但合成写入报 `no result found`，未触及用户钱包；[限制记录](desktop-keyring-isolated.txt)。AI 测试失败分支、RSSHub 测试失败分支、日志目录外部打开未做。 |
| 4：平台能力与返回 | Android 真横屏 `orientation=1`，系统逻辑 2400×1080、WebView 915×364 CSS px；MCP、整库备份/恢复、托盘隐藏，OPML 订阅限制说明和导入/导出可见。桌面真窗口缩至 920 CSS px 后，栏宽、阅读宽度/布局、MCP 和整库操作可见。新进程上原生 KEYCODE_BACK 走设置详情→分类→来源页、阅读器→列表、源菜单→原列表且焦点归还。 | 915 CSS px 真横屏不代表大于 960 CSS px 平板；平板组合留 T6。 |
| 5：确认、焦点、语言/字号 | MCP 危险权限启用取消零写、确认回读。Android 原生 IME 使可视高度 867→554 CSS px，AI 端点焦点及下缘仍在可用区域；首次 Back 只收键盘。zh/en × light/dark 四组合截图，系统 `font_scale=1.3` 与应用字号 20 截图，系统值已恢复。非法代理错误时设置正文高度和视口尺寸不变、无横向溢出，输入保留且 `aria-invalid=true`。 | 截图来自任务自有模拟器，不代表实体手机或所有系统字号档。 |
| 6：构建、测试、迁移台账 | 上述重建 APK/桌面哈希、自动检查、native SAF/MCP/DB/旧库/离线读回及矩阵的 M13–M20 每一行状态已记录。 | 部分/待验行需后续复核；这不是发布就绪结论。 |

## Android SAF 生命周期修复

锁定的 Tauri 2.11.6 在显示密度改变导致 Activity 重建后，`PluginManager.kt` 保留未注册的 `ActivityResultLauncher`，点击 OPML 导出立即取消；冷重启进程可打开 DocumentsUI。该问题对应 [Tauri issue #15506](https://github.com/tauri-apps/tauri/issues/15506)，修复已合入 [PR #15798](https://github.com/tauri-apps/tauri/pull/15798) 并随 Tauri 2.12.0 发布。项目的 `tauri` 与 `tauri-build` 最小版本及 `Cargo.lock` 已升级。最终 APK 上同一个 PID `7703` 经 `wm density 400` 重建 Activity 后，DocumentsUI 打开并写出 299 字节 [OPML](exported-after-recreation.opml.xml)；密度随后重置。最终 APK 又通过原生 ACTION_OPEN_DOCUMENT 导入新合成源并 ACTION_CREATE_DOCUMENT 导出两源 470 字节文件，SHA-256 `18041cfd94a0e09bdbc8e0a4679a67b22752cc2066a24dfce5aafc9f0fbcc906`。

修复 Android Back 后再次重建的 APK（本记录顶部哈希）在真横屏恢复后打开 ACTION_CREATE_DOCUMENT，导出四个合成源 768 字节 [OPML](exported-backfix-final.opml.xml)，再经 ACTION_OPEN_DOCUMENT 导入 [合成 OPML](import-backfix-final.opml.xml) 为 feed ID 5。[SAF JSON](android-saf-results.json)同时保留前一 APK 的诊断轮次和最终轮次哈希。升级前的 [空 OPML](exported-fixture.opml.xml) 和 [首次导入后 OPML](exported-after-import.opml.xml)仅保留为诊断。未在真实手机、其它文档提供器或签名发布 APK 上重复验证。

## 新进程 Android Back 修复

在新进程打开设置详情时，WebView `history.length=3`、`history.state={rr:2}`，但 Tauri `onBackButtonPress` 的 `canGoBack=false`；原生默认因此退出 Activity。直接执行 `history.back()` 能正确返回分类首页。`ui/mobile.js` 现在注册单个 Android Tauri Back 回调，有既有 `historyStack` 条目时消费其 History API 条目，根目的地调用应用退出；列表/阅读器/菜单的原 `popstate` 路径保持单一。最终 APK 新进程的详情→首页→来源原生 Back 路线见 [14 项检查](android-required-results.json)，阅读器→列表及源更多→原列表/触发项焦点回归见 [手工记录](android-manual-results.json)。

## 保留的限制

设备/服务读回与控件可达在矩阵中分开写。M13 部分重启视觉、M14 桌面阅读布局保存、M16 AI 测试失败分支及 Linux keyring、M18 外部链接/托盘/日志目录动作、M20 空态原生视觉仍是部分或待验。Xvfb 无窗口管理器，标题栏与系统托盘手势未执行。Linux keyring 的私有会话尝试和错误见[隔离探针](desktop-keyring-isolated.txt)。这些项目均未标为完整“通过”。
