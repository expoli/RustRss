# 开发指南

## 项目结构

| 路径 | 内容 |
| --- | --- |
| `crates/rustrss-core/` | 订阅解析、SQLite、抓取、全文检索、OPML 和 AI adapter |
| `crates/rustrss-mcp/` | 独立 MCP server，stdio 与 HTTP 双传输 |
| `src-tauri/` | Tauri command、状态、系统凭据库和桌面集成 |
| `ui/` | 原生 JavaScript 与静态资源，无前端打包步骤 |

业务逻辑应放在 `rustrss-core`；桌面端与 MCP 共享 core 和同一数据路径。

## 构建与验证

Linux 开发依赖见[快速开始](getting-started.md#从源码运行)。常用命令：

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p rustrss-desktop
```

修改 `ui/` 后要重新构建桌面应用，因为 Tauri 会把静态资源嵌入程序。UI 行为需要无头复现时，项目的流程和限制见 [`rustrss-headless-ui-verification`](../.agents/skills/rustrss-headless-ui-verification/SKILL.md)。

### 图标资源

唯一图案源是 `src-tauri/icons/src/rustrss-icon.svg`。用 Tauri CLI 2.12.0 与 Python 3 生成 Linux/Windows/macOS 图标及 Android/iOS 衍生资源：

```bash
python3 scripts/sync-icons.py
python3 scripts/sync-icons.py --check
```

生成命令同时更新 `src-tauri/icons/` 与 Android 实际打包输入 `src-tauri/gen/android/app/src/main/res/`，并记录源文件和衍生资源哈希。CI 的检查命令不需要安装 Tauri CLI。修改图案源或生成脚本后应重新生成并提交相关资源与 `provenance.json`，不能只替换桌面 PNG。

Android 普通、圆形及 API26+ 自适应图标共用图案；自适应前景去除背景并缩至 60%，背景使用 `#1c1f26`，让螃蟹与 RSS 保持在 [Android 官方规定的安全圆](https://developer.android.com/codelabs/basic-android-kotlin-compose-training-change-app-icon) 内。外形随系统启动器变化。iOS 图标资源同步生成不代表已支持或实机验证 iOS。

### Android

桌面与 Android 共用 `src-tauri/src/lib.rs` 的入口（`run()`）：单实例锁、系统托盘、应用内 MCP HTTP 服务是桌面专属（`cfg(desktop)`），Android 启动不注册它们；桌面专属命令（文件夹选择、窗口三键等）在移动端显式报「暂不支持」。移动端数据根由入口注入应用沙盒（`rustrss_core::paths::set_data_root`），库与日志仍走同一套目录规则。AI key 的凭据存取在 `src-tauri/src/credentials.rs` 分层：桌面走系统 keyring，Android 经 Kotlin `SecureStorePlugin` 用 Android Keystore（AES/GCM）加密后存应用私有存储——明文不落 SQLite/偏好文件/日志。

构建需要 Android SDK/NDK、JDK 17 与 `cargo tauri`：

```bash
cargo tauri android build --target x86_64 --debug
# 产物：src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

**侧载发布（release APK）**：release 构建需要签名密钥。后续版本沿用项目已有密钥；只在首次建立签名身份时生成一次（自签名，有效期 10000 天）：

```bash
keytool -genkeypair -v -keystore src-tauri/gen/android/app/android-release.keystore \
  -alias rustrss -keyalg RSA -keysize 2048 -validity 10000
```

并在 `src-tauri/gen/android/keystore.properties`（git-ignored）里写入：

```properties
storeFile=app/android-release.keystore
storePassword=<生成时输入的 store 口令>
keyAlias=rustrss
keyPassword=<生成时输入的 key 口令>
```

从发布 tag 的产品源码构建双架构 APK（无 `--debug` 即 release）：

```bash
cargo tauri android build --target aarch64 --target x86_64 --apk --ci
# 产物：src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

用 Android SDK 的 `aapt dump badging <apk>` 核对版本、`arm64-v8a` / `x86_64` 架构和非 debuggable 状态，用 `apksigner verify --verbose --print-certs <apk>` 核对有效签名及证书与上一版本一致。安装升级验证通过后，重命名为 `RustRss_X.Y.Z_android-universal.apk`，用 `gh release upload vX.Y.Z <apk>` 上传到对应 Release，并下载回读核对 SHA-256。私钥和 `keystore.properties` 均不提交、不作为附件上传。Android CI 生成 debug APK 用于构建检查，目前不自动发布签名 release APK。

**安装/升级**：`adb install -r <apk>`（升级用 `-r` 保留
应用数据；侧载到无 adb 的手机时，把 APK 传到设备点击安装，升级直接覆盖安装同签名 APK，
数据保留）。换签名密钥 = 换应用身份，必须先卸载旧版再装新版（数据不迁移）。

## 数据库与诊断

开发库和外来 SQLite 文件不会自动迁移为当前格式。应用标识或 schema 版本不匹配时会拒绝打开；不要用真实用户库做开发截图或试验。可通过 `RUSTSS_DB` 指定隔离数据库。

日志位于平台数据目录下的 `rustrss/logs/`。在桌面端“设置 → 关于”可打开日志目录和调整级别。问题反馈时，附上最新日志，并移除不希望公开的订阅地址等信息。

## 发布

push 到 master 或提交 PR 会运行测试与 Clippy，不生成安装包。nightly 安装包需从 `master` 手动触发；正式桌面版则由匹配版本的 `v*` tag 触发。版本号需要同时更新根 `Cargo.toml` 和 `src-tauri/tauri.conf.json`，并与 tag 一致；发布工作流会在构建前检查三者。工作流生成 Linux `.deb`、Windows NSIS 和 macOS `.dmg`；Android 签名 APK 按上面的步骤构建并补到同一 Release。手动触发 release 工作流只构建检查产物，不创建 Release。工作流定义和平台依赖见 [`.github/workflows/release.yml`](../.github/workflows/release.yml)；macOS 包目前未签名。

## 项目资料

- [项目约定与性能红线](../AGENTS.md)
- [需求基线](../.chorus/specs/rss-reader/spec.md)
- [手工验收清单](../.chorus/specs/rss-reader/manual-verification-checklist.md)
