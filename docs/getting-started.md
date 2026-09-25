# 快速开始

RustRss 是 Rust + Tauri 2 桌面应用。你可以下载 [GitHub Releases](https://github.com/expoli/RustRss/releases) 中已经发布的安装包，或从源码运行。

## 从源码运行

先安装 Rust stable。Linux 最低支持 Ubuntu 24.04；Linux 还需要 GTK、WebKitGTK 和托盘依赖。Ubuntu 24.04 可运行：

```bash
sudo apt install -y libdbus-1-dev libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

在仓库根目录启动桌面应用：

```bash
cargo run -p rustrss-desktop
```

如需使用指定数据库：

```bash
RUSTSS_DB=/path/to/rustrss.sqlite cargo run -p rustrss-desktop
```

Linux 使用系统 WebKitGTK；应用不会打包浏览器内核，也不会强制选择 X11 或 Wayland。

## 添加第一条订阅

1. 点击工具栏中的“添加订阅”，输入 feed 地址或网站首页地址。
2. RustRss 会尝试从首页发现 RSS / Atom feed；确认后即可开始阅读。
3. 需要迁移订阅时，使用侧栏的 OPML 导入/导出入口。

应用默认把数据库放在各平台的用户数据目录中。使用 `RUSTSS_DB` 可以指定其他路径；请保管好数据库文件，并定期使用应用中的备份功能。

## 安装包状态

项目配置了 Linux `.deb`、Windows NSIS 和 macOS arm64 `.dmg` 的发布工作流。macOS 镜像未签名。安装包是否已发布、对应版本和注意事项请以 [Releases](https://github.com/expoli/RustRss/releases) 页面为准。

下一步可阅读[功能介绍](features.md)，或先了解[隐私与数据](privacy.md)。
