# T5 设置、表单、确认与反馈：实现和验证记录

任务 `b36221e4-0d51-46e8-a7a5-111e33894ebc`；本记录只覆盖隔离 fixture、任务自有 Android AVD `RustRssT5` (`emulator-5584`) 和独立 Xvfb 桌面进程。未使用 `emulator-5554` 或真实用户数据库。T4 的证据仍在 `evidence/t4-reader/`，没有复用为本任务的设备通过项。

## 产物与复现

- Android x86_64 **debug** APK：`src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`，第二轮 SHA-256 `02681dbe1a21de18befddf3d5f9f3f4933f4af045e97c8c114c2d5bbe85fc915`。用 `cargo tauri android build --target x86_64 --debug --apk` 重建，`adb -s emulator-5584 install -r ...` 安装；本任务未签名发布包、打 tag 或发布 Release。
- Linux WebKitGTK 二进制：`target/debug/rustrss-desktop`，第二轮 SHA-256 `b26aa17b67eb4c97da89f0fe3835495d25e429cec886365298faa496e3db7142`。用 `cargo build -p rustrss-desktop --locked` 重建。
- 第一轮 `cargo test --workspace --locked` 全部通过；`node --test scripts/tests/*.test.cjs` 61/61；三个 UI JS 文件的 `node --check`、桌面验证脚本的 `py_compile`、`git diff --check` 通过。第二轮聚焦复核见下节。
- 第二轮 APK 上重跑 [Android 分类、返回、错误 JSON](android-results.json)（32 项）、[Android 主题/AI/RSSHub JSON](android-followup-results.json)（20 项），并在清空任务 AVD 的应用数据后完成 [空态 11 项](android-empty-results.json)；这三份结果对应第二轮 APK 哈希。第二轮 [桌面 JSON](desktop-results.json)（49 项）对应第二轮 Linux 哈希。第一轮 [真实横屏 JSON](android-landscape-results.json)、[原生副作用 JSON](android-required-results.json)（14 项）、[手工原生读回](android-manual-results.json)、[RSSHub 迁移](android-rsshub-migration.json)、[离线重试](android-offline-retry.json)与 [SAF 最终轮次](android-saf-results.json#L27)对应第一轮 APK `1295b53869f9db87d66ba81eb78b38f55e8646e8c5e6da49daa02b13a236492c`；[旧库原生验收](legacy-refusal-runtime-results.json)对应第一轮 Linux 二进制 `db89edc2b81655164064f2e5d895db36aa4ad1fee54c1255bea14311154c813d`。复现入口是 `scripts/verify-reading-experience-settings-{android,followup,rotation,required,empty}.mjs`、`scripts/verify-reading-experience-settings-desktop.py` 和 `scripts/verify-legacy-refusal-ui.py`；桌面每次创建独立 HOME、XDG 与数据库。

本机 Maven Central 对新依赖返回 HTTP 403，构建时短暂把阿里云 Maven 公共镜像加入生成的 Gradle repository 列表；APK 构建后已恢复该文件，git 无此变更。此差异只影响本机获取依赖，未改变 APK 源码。完整网络可达的 CI 可直接使用现有 `mavenCentral()`。

## 第二轮审查补证

审查指出窄桌面阅读布局只有控件可达、Android 空态只有静态单测。第二轮在真正 920px X11 窗口中修改阅读宽度为 760px、布局为聚焦阅读；[49 项桌面结果](desktop-results.json)记录保存修订从 0 到 1、CSS `--reader-width` 从 680px 到 760px、主网格从 `150px 240px 530px` 到 `150px 220px 550px`，以及关闭/重开设置、应用重启后的读回。[窗口截图](desktop-reader-layout-920.png)显示这两个已保存值。

Android 在任务 AVD 上先备份任务应用数据到 `/tmp/t5-owned-app-before-empty.tar`，随后仅对 `tech.expoli.rustrss` 执行 `pm clear`，使用第二轮 debug APK 与空数据库。[11 项原生结果](android-empty-results.json)记录空订阅文案及“Add feed”入口、无结果搜索的“Clear search”/“Retry search”、Android 无障碍树、原生 Tab 从清除到重试、按钮执行效果、重试前后的列表/视口几何完全相同以及无横向溢出；[空订阅截图](android-empty-subscriptions.png)与[无结果截图](android-empty-search.png)保留可视读回。直接启用 TalkBack 朗读未执行，焦点和无障碍验证以原生按键及 UI 层级树为界。

第二轮改动仅涉及空态按钮/文案/样式及桌面证据脚本；Android 原有 32+20 项在第二轮 APK 上重跑通过。第二轮 `node --test scripts/tests/*.test.cjs` 61/61，UI 与空态验证脚本 `node --check`、桌面脚本 AST 语法检查及 `git diff --check` 通过；Rust 层未在第二轮变更，工作区 Cargo 测试结果沿用第一轮。

## 逐 AC 结果

| AC | 执行与结果 | 范围界限 |
|---|---|---|
| 1：分类、原字段/动作与反馈 | 第二轮 APK 上六类分类/详情、摘要、Back 和 Android 支持的原控件通过 [32 项设备检查](android-results.json)；重建 Linux 上七类分类/详情及窄窗口桌面动作通过 [49 项检查](desktop-results.json)，包括阅读宽度/布局保存、设置重开、应用重启后回读和网格效果。M13–M20 逐行动作状态见[矩阵](../../action-matrix.md)。 | 部分旧动作只证明入口，矩阵标为部分或待验。 |
| 2：主题草稿/CAS、非主题错误 | Android 草稿零写、详情 Back 保留草稿、离开设置丢弃、保存修订、六预设/三模式、单项继承/全清除/颜色覆盖、历史刷新/恢复、陈旧 CAS 拒写均有读回。代理非法 URL 输入保留且字段错误关联，数据库未变；修正后保存。MCP 无效端口的 UI 和 Rust IPC 均拒绝且数据库未变。 | UI 字体、列表密度等字段已保存读回，重启视觉对照仍按矩阵标部分。 |
| 3：代理/RSSHub/AI、凭据、数据/通用/MCP | Android 代理/绕过/刷新字段保存，RSSHub 合成 `/version` 测试及单个旧域候选预览→取消→归一化→重复预览，四家 AI provider/model 保存，Ollama 合成 HTTP 测试，Android Keystore 密钥状态与清除。最终 APK 通过 SAF 导入 feed ID 5、导出四个合成源。Linux MCP 监听、鉴权、token 生命周期、权限确认与取消均通过；GTK 数据库备份/恢复及旧库拒绝→只读 OPML 导出→退出均通过。 | Linux 私有 Secret Service 会话中 ksecretd 注册服务，但合成写入报 `no result found`，未触及用户钱包；[限制记录](desktop-keyring-isolated.txt)。AI 测试失败分支、RSSHub 测试失败分支、日志目录外部打开未做。 |
| 4：平台能力与返回 | Android 真横屏 `orientation=1`，系统逻辑 2400×1080、WebView 915×364 CSS px；MCP、整库备份/恢复、托盘隐藏，OPML 订阅限制说明和导入/导出可见。桌面真窗口缩至 920 CSS px 后，栏宽、阅读宽度/布局、MCP 和整库操作可见。新进程上原生 KEYCODE_BACK 走设置详情→分类→来源页、阅读器→列表、源菜单→原列表且焦点归还。 | 915 CSS px 真横屏不代表大于 960 CSS px 平板；平板组合留 T6。 |
| 5：确认、焦点、语言/字号 | MCP 危险权限启用取消零写、确认回读。Android 原生 IME 使可视高度 867→554 CSS px，AI 端点焦点及下缘仍在可用区域；首次 Back 只收键盘。zh/en × light/dark 四组合截图，系统 `font_scale=1.3` 与应用字号 20 截图，系统值已恢复。非法代理错误时设置正文高度和视口尺寸不变、无横向溢出，输入保留且 `aria-invalid=true`。第二轮空库与无结果搜索的添加/清除/重试用词、Android 无障碍树、原生 Tab 顺序、动作效果及重试前后列表/视口几何见 [11 项检查](android-empty-results.json)。 | 截图来自任务自有模拟器，不代表实体手机或所有系统字号档；未运行实体 TalkBack 朗读。 |
| 6：构建、测试、迁移台账 | 上述重建 APK/桌面哈希、自动检查、native SAF/MCP/DB/旧库/离线读回及矩阵的 M13–M20 每一行状态已记录。 | 部分/待验行需后续复核；这不是发布就绪结论。 |

## Android SAF 生命周期修复

锁定的 Tauri 2.11.6 在显示密度改变导致 Activity 重建后，`PluginManager.kt` 保留未注册的 `ActivityResultLauncher`，点击 OPML 导出立即取消；冷重启进程可打开 DocumentsUI。该问题对应 [Tauri issue #15506](https://github.com/tauri-apps/tauri/issues/15506)，修复已合入 [PR #15798](https://github.com/tauri-apps/tauri/pull/15798) 并随 Tauri 2.12.0 发布。项目的 `tauri` 与 `tauri-build` 最小版本及 `Cargo.lock` 已升级。最终 APK 上同一个 PID `7703` 经 `wm density 400` 重建 Activity 后，DocumentsUI 打开并写出 299 字节 [OPML](exported-after-recreation.opml.xml)；密度随后重置。最终 APK 又通过原生 ACTION_OPEN_DOCUMENT 导入新合成源并 ACTION_CREATE_DOCUMENT 导出两源 470 字节文件，SHA-256 `18041cfd94a0e09bdbc8e0a4679a67b22752cc2066a24dfce5aafc9f0fbcc906`。

修复 Android Back 后再次重建的第一轮 APK（哈希 `1295b53869f9db87d66ba81eb78b38f55e8646e8c5e6da49daa02b13a236492c`）在真横屏恢复后打开 ACTION_CREATE_DOCUMENT，导出四个合成源 768 字节 [OPML](exported-backfix-final.opml.xml)，再经 ACTION_OPEN_DOCUMENT 导入 [合成 OPML](import-backfix-final.opml.xml) 为 feed ID 5。[SAF JSON](android-saf-results.json)同时保留前一 APK 的诊断轮次和第一轮最终轮次哈希。升级前的 [空 OPML](exported-fixture.opml.xml) 和 [首次导入后 OPML](exported-after-import.opml.xml)仅保留为诊断。未在真实手机、其它文档提供器或签名发布 APK 上重复验证。

## 新进程 Android Back 修复

在新进程打开设置详情时，WebView `history.length=3`、`history.state={rr:2}`，但 Tauri `onBackButtonPress` 的 `canGoBack=false`；原生默认因此退出 Activity。直接执行 `history.back()` 能正确返回分类首页。`ui/mobile.js` 现在注册单个 Android Tauri Back 回调，有既有 `historyStack` 条目时消费其 History API 条目，根目的地调用应用退出；列表/阅读器/菜单的原 `popstate` 路径保持单一。第一轮 APK 新进程的详情→首页→来源原生 Back 路线见 [14 项检查](android-required-results.json)，阅读器→列表及源更多→原列表/触发项焦点回归见 [手工记录](android-manual-results.json)。第二轮 APK 重跑分类/Back 32 项通过。

## 保留的限制

设备/服务读回与控件可达在矩阵中分开写。第二轮 920px 阅读宽度/布局和 Android 原生空态已补证：[Linux 截图](desktop-reader-layout-920.png)、[空订阅](android-empty-subscriptions.png)、[无结果搜索](android-empty-search.png)。M13 部分重启视觉、M16 AI 测试失败分支及 Linux keyring、M18 外部链接/托盘/日志目录动作仍是部分或待验。Xvfb 无窗口管理器，标题栏与系统托盘手势未执行。Linux keyring 的私有会话尝试和错误见[隔离探针](desktop-keyring-isolated.txt)。这些项目均未标为完整“通过”。
