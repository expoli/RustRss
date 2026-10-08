# Android v0.3.1 覆盖升级验证（2026-10-08）

目标：验证正式发布签名 APK 从 0.2.1 原地升级（`install -r`）到 0.3.1 后数据保留与基本可用性。

设备：专用 headless 模拟器 `emulator-5580`（AVD `RustRss_API_36`，x86_64 / API 36 / Pixel 7 /
Gboard / Pixel Launcher / 系统 Downloads）。启动前先移除了 AVD 上残留的 0.6.0(code6000) 开发
签名构建（签名不同无法覆盖，也无法保留其数据）。

流程（`upgrade-smoke.py`，全程 adb + uiautomator 驱动，结果见 `results.json`）：

1. 安装正式发布渠道下载的 v0.2.1 APK（73,050,838 bytes），实际安装 base.apk SHA256 与
   发布资产 `b0e9474f…` 一致；versionName 0.2.1 / code 2001。
2. 在订阅页用 URL 添加两个真实订阅（宿主机 18080 端口 HTTP server，模拟器经 10.0.2.2
   访问，真实走网络抓取 + feed-rs 解析 + SQLite 落库）：
   `Upgrade Fixture A`（2 items）与 `Upgrade Fixture B`（1 item）。UID 10229。
3. force-stop 后 `install -r` 升级到正式 v0.3.1 APK（73,548,720 bytes），实际安装 SHA256
   与发布资产 `99e66688…` 一致；versionName 0.3.1 / code 3001。
4. 升级后断言：UID 不变（10229 → 10229）；Pixel Launcher 图标实际点击启动成功；
   订阅页两条 fixture 均保留（`v031-retained-data.png`）；root sqlite3 回读
   `feeds` 表两条记录 URL 无损、`entries` 表 3 条全在；热启动订阅页 URL 输入框在
   Gboard 弹出时仍完整可见（原生 bounds y2=521 < 900，`v031-ime-visible-entry.png`）；
   设置页分类列表正常渲染（`v031-settings-home.png`）。

截图：`v031-seeded-021-data.png`（0.2.1 种子态）、`v031-launcher.png`（升级后图标）、
`v031-retained-data.png`、`v031-ime-visible-entry.png`、`v031-settings-home.png`。

边界与说明：

- 本轮 OPML 导入路径的 uiautomator 自动化未走通（选择器内选文件后 app 无响应），未作为
  依据也不据此声称 0.2.1 OPML 功能异常——该功能已在 v0.2.1 发布轮以实机证据验证
  （`../v0.2.1-android/`）；本轮改用 URL 添加做种子，顺带覆盖了真实抓取解析路径。
- 覆盖升级方向为 0.2.1 → 0.3.1；0.2.0 → 0.3.1 跨两个版本未测。实体 ARM64 手机、
  第三方输入法/启动器未直接操作。
- 测试种子为宿主机本地 fixture，不涉及用户真实数据。
