# RustRss

**把 RSS 阅读带回桌面。** 一个本地优先的跨平台阅读器：订阅和阅读状态留在自己的设备上；需要时，可选用自带密钥的 AI，或通过 MCP 把订阅接入你的 agent 工作流。

Rust + Tauri 2 · SQLite · RSS / Atom / JSON Feed

![RustRss 三栏阅读界面，使用本地演示文章](docs/images/app-overview.png)

界面截图使用虚构演示内容生成。

## 阅读体验

- **专注阅读**：订阅、文章列表和正文分栏展示；支持搜索、稍后读、星标、标签和键盘操作。
- **报纸版式为默认主题**：四套视觉预设（报纸 Print / Clear / Paper / Slate），默认「报纸」——纸墨配色加一处编辑部红，正文与列表标题用衬线字体、版心居中；明暗色可分别选预设，所有值均可覆盖。
- **数据在本地**：订阅、文章与阅读状态存入 SQLite；无需账号或云同步。
- **AI 由你选择**：配置自己的 OpenAI 兼容、Anthropic、Gemini 或 Ollama 服务，用于文章摘要和翻译。
- **面向 agent 的 MCP**：让兼容客户端读取你的订阅和文章。服务只监听回环地址，默认只读，写能力需单独授权。

![RustRss 外观与主题设置](docs/images/settings.png)

## 开始使用

```bash
cargo run -p rustrss-desktop
```

RustRss 面向 Linux、Windows 和 macOS 桌面；Android 版可在设备上侧载使用——订阅、刷新、离线阅读、AI 摘要/翻译（四家提供商）、外链打开与安全密钥存储均已在模拟器/设备上验证，构建/签名/安装步骤见 [docs/development.md](docs/development.md) 的 Android 段，持续构建由 Android CI（`android-build.yml`）承担。已发布的安装包和版本说明以 [GitHub Releases](https://github.com/expoli/RustRss/releases) 页面为准；也可以查看[快速开始](docs/getting-started.md)从源码运行。

各平台应用图标统一为橙色 Ferris 螃蟹＋蓝色 RSS、深色背景；Android 按系统外形使用普通、圆形或自适应版本，图案与配色一致。所有打包资源从同一份 SVG 生成并同步到 Android 原生工程，维护命令见[图标资源](docs/development.md#图标资源)。

0.3.0 汇总桌面与 Android 阅读体验整合：四主导航、共享阅读视觉基线、手机操作面板与列表工具收敛；更新说明见 [CHANGELOG](CHANGELOG.md#030---2026-10-01)。Android APK 沿用原发布签名，可覆盖安装保留数据。

0.3.1 修复 Windows WebView2 依赖类型不一致造成的发布构建失败，并在普通 CI 中增加 Windows 编译与跨平台应用版本检查。平台专属 WebView 实现仍使用各自的系统后端，但 RustRss 产品版本、Tauri 次版本线和构建工具在发布流程中统一校验。

0.4.0 为 Android 增加设置→通用「诊断日志」：无需 adb 即可在应用内查看并导出沙盒日志（末尾 256KB 截断查看 + 系统文档选择器导出完整文件），排障取证不再依赖开发者工具；详见 [CHANGELOG](CHANGELOG.md#040---2026-10-02)。

0.4.1 修复 Android 端刷新全灭（reqwest 0.13 把 `rustls` 的默认验证器换成 rustls-platform-verifier，Android 上未初始化即每次 HTTPS 抓取 panic），并将「诊断日志」入口移至通用设置与日志级别同区；详见 [CHANGELOG](CHANGELOG.md#041---2026-10-02)。

0.5.0 把界面默认视觉切换为新的「报纸」主题：纸墨配色与一处编辑部红，列表标题与阅读正文用衬线字体，选中行为红色批注语义；四套视觉预设（报纸 / Clear / Paper / Slate）明暗可分别选择，全部数值可覆盖，阅读头操作行合并、设置界面布局与交互一致性修正；详见 [CHANGELOG](CHANGELOG.md#050---2026-10-03)。

0.5.1 无应用代码变更：Android 签名 APK 改由 CI 在推 `v*` tag 时自动构建并附加到 Release（签名材料走 repo secrets，构建后断言发布证书指纹）；详见 [CHANGELOG](CHANGELOG.md#051---2026-10-03)。

Android 手机界面中，文章、订阅、收藏和设置共用完整的底部导航；文章和收藏页的搜索按钮一击展开输入框，明确提示搜索范围为全部文章；取消后恢复原筛选和列表位置。内容区按底栏实际高度布局。设置是一级页面，分类首页和详情均保留底栏；系统返回先回到分类首页，再回到进入设置前的页面。订阅地址入口位于订阅页上方，键盘弹出时页面避让并滚动到当前输入框。阅读先展示字号、行距和段落间距，字体与代码排版、高级 AI 参数按需展开。主题修改仍需点保存，切到其它一级页面会放弃未保存的主题草稿；桌面设置继续使用模态弹窗。Android OPML 导入允许选择普通文档，按实际内容校验，避免文件管理器把 `.opml` 标成普通文件时无法点击；桌面保留 OPML/XML 过滤。Android 的数据设置只提供 OPML 订阅导入导出；OPML 不包含文章、阅读状态或 AI 设置，不支持的整库备份/恢复入口已隐藏。「诊断日志」入口位于通用设置（与日志级别同区）：列出应用沙盒 `logs/` 下的 `rustrss-*.log`（名称/大小/修改时间/当前标记），可查看末尾 256KB（超出时明示截断）并经系统文档选择器导出完整文件，供无需 adb 的排障取证。输入和表单的既有设备证据见 [手机交互验证](.chorus/specs/android-app/2026-09-29-phone-ux/evidence/record.md)，本轮验证见 [底栏与返回记录](.chorus/specs/android-app/2026-09-29-primary-navigation/evidence/record.md)。

设置首页按分类显示当前值摘要；外观和阅读保留草稿预览、保存、放弃与主题历史，详情页返回会保留未保存草稿，离开设置才放弃。代理、AI 和 MCP 表单出错时，输入仍在原位，错误会关联到对应字段。桌面窄窗口仍可设置栏宽、阅读宽度与布局；Android 横屏仍只显示手机支持的操作。桌面 MCP 的危险工具开关需要确认，取消不会更改权限。Android 文档选择器在旋转或显示配置变化后仍可用于 OPML 导入导出；Tauri 核心升级至 2.12.0，设备与桌面验证记录见 [T5 设置证据](.chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t5-settings/record.md)。

桌面版在至少 900 CSS px 宽的窗口保留三栏、聚焦阅读、栏宽设置和键盘/右键菜单；Pixel Tablet 模拟器上，960 CSS px 及以下使用四项底部导航，961 CSS px 起显示三栏，Android 桌面窗口控件始终隐藏。平板窄幅空订阅提示会指向“订阅”页的添加入口。Linux 原生产物的同进程订阅到 OPML 旅程、12 种预设/明暗/语言截图和断点证据见 [T6 汇合记录](.chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t6-convergence/record.md)。标题栏最小化在 XWayland 可读回；KDE Wayland 的 Tauri 状态回读滞后，但 T6 私有嵌套 KWin 证据已观察到可信点击后 compositor 最小化、窗口消失并可恢复。任务栏真实点击和其他桌面环境仍未测。

列表、阅读动作、菜单和设置现共用本地 SVG 图标、控件间距与焦点样式；手机主要操作入口的触控框按至少 48×48 CSS px 布置。配色、字体和预览仍由现有主题设置控制，已保存的覆盖值不会因图标和布局样式更新而重置。本次视觉基线、隔离数据与验证范围见 [阅读体验 T1 证据](.chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/README.md)，手机列表与搜索的前后截图、导航和返回证据见 [T3 设备记录](.chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t3-list/record.md)。

打开文章时列表行保持节点身份；重复进入同一视图时，侧栏的计数、提示和辅助文本不重复写入 DOM。200 行原生桌面打开与星标性能对照见 [T7 性能证据](.chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t7-final/performance-star/record.md)；最终 Linux 同夹具 12 种预设/明暗/语言组合在文章、阅读和设置三页的 36 对截图，以及 Android APK 与 Linux 二进制的哈希和未测范围见 [T7 集成记录](.chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t7-final/record.md)。

订阅、文件夹和标签行现提供独立的“更多操作”按钮；手机以带内部选择页的底部操作面板呈现刷新间隔、移动文件夹及标签颜色，桌面保留右键级联菜单并支持方向键与 Esc。添加订阅支持网站候选源选择和 RSSHub 地址输入；删除文件夹、取消订阅和删除标签会先显示影响并要求确认。Android 模拟器、Linux WebKitGTK 键盘与 TalkBack 语义树的验证步骤及范围见 [T2 操作面板证据](.chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t2-menu/record.md)。

手机阅读页把返回、Aa、星标、稍后读和“更多”保持在首屏；“更多”承载已读、标签、链接、分享、全文和 AI 动作，打开菜单本身不会请求全文或 AI。菜单操作失败会在阅读页显示错误并保留原有阅读状态；TalkBack 浏览阅读页时会跳过被覆盖的列表。AI 发送前仍展示目标、脱敏头和正文预览，可取消或选择以后不再询问；离线图片失败时显示替代文字。Aa 保存会保持当前正文锚点。Android 原生操作、桌面键盘及 TalkBack 的实测范围和安装包哈希见 [T4 阅读器证据](.chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t4-reader/record.md)。

Windows 主题预览截图现通过 WebView2 的原生内容截图接口生成 PNG，并沿用 MCP 预览的修订号、像素标记、文件配额与到期规则。该适配器的 Windows 真机运行验收仍在进行中；当前不能把 Linux 截图结果当作 Windows 场景、缩放和最小化的通过证据。复验范围见 [Windows 预览验收记录](.chorus/specs/rss-reader/2026-09-23-theme-preview/windows-capture.md)。
macOS 主题预览现使用 WKWebView 内容截图适配器；真实 macOS 会话的三场景、缩放与失败路径仍待运行验收，步骤和证据状态见 [macOS 截图记录](.chorus/specs/rss-reader/2026-09-23-theme-preview/macos-native-capture.md)。

Linux 上的最终整合版已在隔离的 KWin Wayland 和 Openbox X11 会话通过场景级预览回归，并完成一次 agent 实际读图、调整、保存和取消后的 SQLite 回读；[运行证据](.chorus/specs/rss-reader/2026-09-23-theme-preview/agent-loop-2026-09-28/README.md)附原始 PNG 与二进制指纹。Windows/macOS 真机环境本次不可用，按用户指示跳过相应运行测试；两平台的原生截图与失败路径仍标为待验。

## 发布步骤

版本变更和已知限制见 [CHANGELOG.md](CHANGELOG.md)。

1. 在根 `Cargo.toml` 的 `[workspace.package]` 和 `src-tauri/tauri.conf.json` 中同步更新版本号，运行 `python3 scripts/check-version-consistency.py` 后提交版本变更。
2. 更新 `CHANGELOG.md`，记录该版本变更和已知限制并提交。正式 Release 说明由工作流自动生成，发布时核对并补充 CHANGELOG 中的内容。
3. 将上述提交推送到 `master`，等待 Linux 测试/Clippy、Windows 编译和 Android 持续构建全部通过。
4. 在这个精确提交上手动触发 `release.yml`（`workflow_dispatch`）。它只构建并上传三平台检查产物，不创建 Release；确认 Linux `.deb`、Windows NSIS `.exe` 和 macOS arm64 `.dmg` 全部通过。
5. 只有 preflight 全绿后，才在同一 SHA 创建并推送 `vX.Y.Z` tag。`release.yml` 会再次校验 tag 与两处版本号，并创建 GitHub Release；macOS 包目前未签名。
6. 从 tag 对应的产品源码，沿用项目发布密钥构建 Android ARM64 / x86_64 通用 release APK，用版本检查脚本和 `apksigner` 核对版本、签名及安装升级后上传到同一 Release。命令与密钥配置见 [Android 发布步骤](docs/development.md#android)；`android-build.yml` 的 debug APK 仅用于持续构建检查，不会自动成为 Release 附件。
7. 从正式附件记录各安装包字节数与 SHA-256，回填 `.chorus/specs/rss-reader/spec.md` 的「安装包体积实测值」条目和 `docs/releases/` 的发布记录。

只需测量体积时，在 GitHub Actions 手动触发 `release.yml`（`workflow_dispatch`）：它构建并上传三平台产物，但不创建 Release；无需为测量打 `v*` tag。

## 文档

| 指南 | 内容 |
| --- | --- |
| [快速开始](docs/getting-started.md) | 环境准备、运行应用、首次订阅 |
| [功能介绍](docs/features.md) | 阅读、搜索、快捷键、外观和订阅管理 |
| [内置 AI](docs/ai.md) | 服务商配置、密钥存储和文章处理 |
| [MCP 集成](docs/mcp.md) | 连接客户端、工具范围与读写权限 |
| [隐私与数据](docs/privacy.md) | 本地数据、外部请求和安全边界 |
| [开发指南](docs/development.md) | 项目结构、构建、日志和发布 |
| [全部文档](docs/README.md) | 按读者角色浏览指南与项目资料 |

## 隐私概览

文章库和阅读状态保存在本机。AI 请求会把所选文章内容发送到你配置的服务商；MCP 可让你授权的本地客户端读取订阅。开启文章缩略图时，WebView 会直接请求图片站点。详见[隐私与数据](docs/privacy.md)。

## 许可证

[AGPL-3.0-or-later](LICENSE)：任何人可自由使用、修改、分发，**包括商业用途**；附加的唯一约束是源码开放义务（第 4、5、13 条）。因此若你修改 RustRss 并以网络服务形式对外提供，需向使用者提供完整的对应源码（本仓库即为对应源码）。

**需要闭源分发、把修改版作为商业产品出货，或不愿承担源码开放义务**时，请另行取得[商业授权](LICENSE-COMMERCIAL.md)。

- 历史版本（提交 `cfaf0dc` 及更早，含切换前的 `nightly` 安装包）仍按 MIT OR Apache-2.0 授权，其授权不可撤销，文本见 [`LICENSES/`](LICENSES/README.md)。
- 版权许可不授予商标权，修改版分发请遵守[商标政策](TRADEMARK.md)。
- 贡献代码前请阅读[贡献指南](CONTRIBUTING.md) 中的贡献授权条款。
