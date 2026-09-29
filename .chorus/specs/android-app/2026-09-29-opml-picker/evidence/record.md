# Android OPML 文件选择修复（2026-09-29）

Chorus task: `120623c8-4e4b-4eb1-99fc-764f8a37960b`。

## 根因与修复

Android `tauri-plugin-dialog 2.7.3` 把扩展名过滤经 MimeTypeMap 转成 EXTRA_MIME_TYPES；`.opml` 未必有对应类型，XML 过滤会把系统识别为普通文件的 OPML 禁用。未修复调试 APK 在原生 Downloads 中，`RustRss-generic.opml` 的标题节点为 `enabled=false`，同目录 `.xml` 为 true。`before-opml-disabled.png`、`before-test.log` 保留先红证据；系统 Recent 中该 OPML 的元数据为 BIN file。

只调整 `import_opml` 的对话框构建：桌面仍加 OPML/XML 扩展名过滤，移动端使用默认可打开文档（*/*）；随后仍走 ContentResolver 和共享 core OPML 解析，不绕过内容校验、不申请额外存储权限、不改导出。Android 根据 MIME 过滤文档的机制见 [官方存储访问文档](https://developer.android.com/training/data-storage/shared/documents-files)。

## 自动验证

`ANDROID_SERIAL=emulator-5580 timeout 180 node scripts/verify-android-opml.mjs <output-dir>`，六场景全部通过。脚本推入小型测试文件、通过产品 Tauri import_opml 命令打开系统选择器，使用原生 Downloads 节点点击并核对返回统计与 list_feeds：

- generic OPML：新增 1 源。
- 同文件重复导入：新增 0、跳过 1。
- XML 格式 OPML：新增 1 源。
- HTML/非 OPML：报无 opml 根错误，订阅集合不变。
- 损坏 XML：报 XML 解析位置错误，订阅集合不变。
- 取消系统选择器：返回 null，订阅集合不变。

`results.json`、`device-test.log` 与 `01..05` PNG 为实际结果。UIAutomation 偶有空树，脚本仅使用新鲜成功 dump；明确导航到 Downloads，取消会走完 Recent/目录返回栈，避免将目录返回误判为取消。

`cargo test --workspace --locked` 全绿；`cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。日志保留。原 core OPML 负例测试仍运行，未引入额外解析路径。

## 验证边界

API36 x86_64 /系统 Downloads 提供者，未直接操作用户实体手机或所有第三方文档/网盘提供者。新版包含此前手机输入/设置修复，正式 v0.2.0 tag/资产保持不变。

## 签名安装包的实际界面检查

新版 SHA-256 `cc1712bb44c19f4e42ab755fe4f5a575ffb10652ff0dc25f477286a7f69aa1f9`，ARM64/x86_64 通用、非 debuggable、versionName 0.2.0 / code2000；证书 SHA256 与原发布密钥一致，见 metadata/signature。

先在上一轮已签名手机界面预览包中，通过原生 Settings → Data → Import OPML 进入选择器：`.opml` 仍禁用，`.xml` 可以实际导入。其订阅行带失败抓取状态和0未读，原生无障碍文本为 `Picker xml ● 0`（不是仅标题）；核对完整标题前缀并截屏。

使用 `adb install -r` 覆盖新预览 APK 后，UID 不变、原 XML 订阅保留；通过相同设置按钮，普通 MIME `.opml` 已启用，实际点击可导入，订阅页同时出现 Picker generic 与 Picker xml。手机纵向设置首页仍保留。`signed-results.json`、`signed-*.png` 为原生实际界面证据；UI 文件选择与调试命令检查分别记录。

这是后续预览包，正式 v0.2.0 tag 与四项资产未修改。
