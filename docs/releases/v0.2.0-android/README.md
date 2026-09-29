# v0.2.0 Android 签名 APK 验证

2026-09-29 从 `v0.2.0` 的产品源码构建，工作区产品文件经 `git diff --exit-code v0.2.0 -- Cargo.toml Cargo.lock crates src-tauri ui` 核对一致。使用本机已有项目发布密钥；私钥和口令不属于验证附件。

```bash
cargo tauri android build --target aarch64 --target x86_64 --apk --ci
aapt dump badging app-universal-release.apk
apksigner verify --verbose --print-certs app-universal-release.apk
```

- 包名 `tech.expoli.rustrss`，versionName `0.2.0`，versionCode `2000`，minSdk `24`（Android 7.0+），无 debuggable 标志。
- 两个 ABI：`arm64-v8a` / `x86_64`。APK v2 签名有效，证书 SHA-256 与本机保存的原有签名 0.1.0 release APK 相同，见 `apk-signature.txt`。
- 使用独立 Android API 36 / x86_64 模拟器，不改动已有模拟器：先安装原有签名 0.1.0 release APK 并启动，在新建应用沙盒写入测试文件，再 `adb install -r` 安装本次 APK。
- 覆盖安装返回 `Success`；应用 UID 与测试文件内容保留；升级后包版本为 0.2.0 / 2000，进程正常运行，MainActivity 打开文章页与手机底部导航，见 `upgrade-launch.png`；启动观察窗口日志无 `FATAL EXCEPTION` / `Fatal signal`。
- 这是模拟器安装、启动与覆盖升级检查；ARM64 实体手机运行未在本次补包中重新验证。
- Release 上传后重新下载，字节数与 SHA-256 同时核对本地产物和 GitHub 资产元数据；校验值见 `apk-metadata.json` 与上级发布记录。
