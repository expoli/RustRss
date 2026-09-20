# RustRss

跨平台（Windows / macOS / Linux）RSS 阅读器，Rust 实现。核心主张：**本地优先 + AI 双通道**（应用内自带 key 的摘要/翻译 + 对外 MCP server 让 agent 直接读你的订阅）。

- 需求基线：`.chorus/specs/rss-reader/spec.md`
- 当前阶段分析（竞品、风险、决策）：`.chorus/specs/rss-reader/2026-09-20-initial-requirements/prd.md`

## 已定决策

| 项 | 选择 |
| --- | --- |
| 平台 | 桌面三平台优先（Win / macOS / Linux），**Linux 需同时支持 X11 与 Wayland** |
| 界面技术 | Tauri 2 + Web 前端（Rust 后端） |
| AI | 双通道：内置 AI（用户自带 key）+ 对外 MCP server |
| 数据 | 本地优先 SQLite + OPML；v1 不做云同步 |
| 许可证 | MIT OR Apache-2.0 |
| 定位 | 先自用；发布能力留在架构里但不投入 |
| bundle id | `tech.expoli.rustrss` |

## 硬约束（来自竞品实测，见 PRD 风险 1 / 10）

- **不打包 WebKit**，一律使用系统 WebKitGTK，按发行版出 native 包。
- **不强制显示后端**（不继承 AppImage 那套 `GDK_BACKEND=x11`），X11 与 Wayland 都要原生可用。

## 当前状态：M0 技术探针

`src-tauri/` + `ui/` 是一个最小 Tauri 2 应用，唯一目的是验证四件事，不含业务逻辑：

1. **中文输入法**（fcitx5 / ibus）在 Wayland 与 X11 下能否正常组合输入并上屏
2. **分数缩放**（125% / 150% / 175%）下是否清晰、尺寸是否正确
3. **资源占用**与启动耗时（与 Papr / MrRSS / Boke 同口径对比）
4. **正文渲染**质量（图片、表格、代码块、长串溢出、中英混排）

### 依赖

```bash
sudo apt install -y libdbus-1-dev libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

说明：`libdbus-1-dev` 容易被漏掉——缺它时 `cargo build` 会先在 `libdbus-sys` 的构建脚本上失败（`Package dbus-1 was not found`），根本走不到 GTK/WebKit 那一步（实测 2026-09-20，exit=101）。

### 运行（三种会话各跑一遍）

```bash
cargo run -p rustrss-desktop                              # 按会话自动选择后端
GDK_BACKEND=wayland cargo run -p rustrss-desktop          # 强制 Wayland
GDK_BACKEND=x11 cargo run -p rustrss-desktop              # 强制 X11
env -u DISPLAY GDK_BACKEND=wayland cargo run -p rustrss-desktop   # 模拟无 XWayland
```

页面把诊断数据同时显示在界面上、并通过 `probe_log` 打到 stdout（前缀 `[probe]`），便于外部脚本采集。
