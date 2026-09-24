---
title: PRD: 跨平台 Rust RSS 阅读器（初始需求梳理）
proposalUuid:
documentUuid:
---

# 跨平台 Rust RSS 阅读器 — 初始需求梳理

> 状态：v0 草稿，待评审。需求基线见同级上层 `.chorus/specs/rss-reader/spec.md`（本文件只承载本次「梳理需求」的分析与取舍，不重复验收条目）。

## 1. 背景与问题

用 Rust 做一个跨平台的 RSS 阅读器，参考 quick-rss 的界面与 AI 接入思路，同时避开 folo 与 MrRSS 各自的短板。三个参考对象的问题拼起来正好是空位：

- quick-rss 思路好（应用只做数据提供者，AI 由用户已有的客户端带进来），界面克制干净，但**只在 Apple 平台**。
- folo 体验完整、支持 RSSHub，而且**已经支持 BYOK**（自带密钥，provider 可选 4 家并支持自定义 base URL，v1.2.2 引入）——「folo 不能自带 API key」这个判断已经过时；它现存的短板是云账号依赖、Electron 形态的资源开销，以及它的 MCP 是**客户端方向**（拿来消费外部 MCP 服务扩展自己的 AI，而不是把自己的订阅暴露出去）。
- MrRSS 功能面极全（多 AI 提供方、插件生态、server mode），但**资源占用被用户判定为过大**，其形态是 Go + Wails v3 + Vue，即 WebView 壳应用——这与其内存表现高度相关（待实测确认）。

## 2. 竞品分析

### 2.1 三个指定参考对象

| 维度 | quick-rss | folo | MrRSS |
|---|---|---|---|
| 平台 | macOS / iOS | 桌面 + 移动 + Web | Win / macOS / Linux（+ Docker 服务端） |
| 形态与栈 | Swift / SwiftUI 原生 App（仓库仅含文档、feeds 与脚本） | Electron（桌面）+ React Native（移动）+ 自建云 | Go + Wails v3 + Vue（WebView 壳） |
| AI 思路 | **MCP server**：`http://127.0.0.1:8745/mcp?token=…`，v3.0 起支持 stdio；供 Claude Desktop / Cursor / VS Code / Grok 调用订阅与文章数据 | 内置云 AI（翻译、摘要、AI 记忆等）**＋ BYOK**：provider 可选 OpenAI / Google / Vercel AI Gateway / OpenRouter，支持自定义 base URL 与自定义请求头，密钥加密存储；**MCP 为客户端方向**——连接外部 MCP 服务来扩展自身 AI，不对外暴露订阅 | 内置 AI 全家桶：摘要、翻译、聊天、AI 搜索；多配置档（OpenAI 兼容 / Claude 原生 / Gemini 原生+兼容 / Ollama）；**无 MCP**（对外能力走 REST API + Swagger + Codex skill 包） |
| 自带 key | 不适用（本身不调模型） | **支持**（4 家 provider + 自定义 base URL，v1.2.2 起） | 支持 |
| 同步 | iCloud（绑 Apple 生态） | 官方云账号 | 本地文件 / 便携模式；服务端模式另算 |
| 其他 | OPML 导入导出、widgets、favicon 修正 | 时间线、列表与社区、RSSHub、动态内容（视频/音频/图片） | 插件（Obsidian / Notion / FreshRSS / Miniflux / RSSHub / SiYuan / Zotero）、URL/XPath/脚本/newsletter 订阅、REST API + Swagger、Codex skill 包 |

证据来源：`quick-rss` 的 README 与 CHANGELOG（v2.8.0 引入 MCP、v3.0.0 加 stdio，端口与 token 形式见 README 的 Grok 配置段）；`MrRSS` 的 `docs/AI_PROVIDERS.md`、`docs/AI_CONFIGURATION.md`、`docs/SKILLS.md` 与 `frontend/public/assets/ai_icons/`；`folo` 的 BYOK / MCP 结论取自一手代码与文案——`apps/desktop/layer/renderer/src/modules/settings/tabs/ai/byok/`（provider 列表见 `byok/constants.ts`，`baseURL` 字段见 `ByokProviderModalContent.tsx`）、`.../ai/mcp/`、`locales/ai/zh-CN.json` 的 `byok.*` 与 `integration.mcp.*` 段、`apps/desktop/changelog/1.2.2.md`。

### 2.2 同类 Rust 项目（用户未提及，但直接影响差异化）

| 项目 | 栈 | 已有能力 | 与本项目的关系 |
|---|---|---|---|
| **Boke** | Rust + Tauri v2 + React | 离线优先三栏、键盘驱动（vim 键位）、文件夹、深浅色、系统托盘、无账号无云 | 最接近「Rust 版 quick-rss」的轻量路线；但它**没有 AI 通道** |
| **Papr** | Rust + Tauri，`papr-core` 共享库 + `papr-cli` | 本地 SQLite、智能视图、标签与规则、全文抓取、**自带 key 的 AI（摘要/问答/日报）**、内置音频、FreshRSS/Miniflux 同步、**面向 agent 的 CLI（TOON 输出，宣称比 JSON 省约 40% token）**、i18n | 功能上几乎覆盖本项目设想，**差别在 AI 通道形态：它是 CLI，不是 MCP** |

其余 Rust 项目多为 TUI（Rivulet、eilmeldung 基于 `news-flash` 库）或早期的 Tauri 尝试（Chaski），不构成同档竞争。

### 2.3 差异化结论

先说被修正的部分：**「自带 key 的内置 AI」不构成差异化**。folo 已有 BYOK（含自定义 base URL），MrRSS 与 Papr 也一直有——它是本项目的**必备项**，不是卖点。定位表述里不能再把它当亮点，否则一查即破。

修正后仍然成立的差异点只有两条：

1. **对外 MCP server（把自己的订阅暴露给外部 agent）**——quick-rss 有但锁 Apple；folo 的 MCP 是反方向的客户端；MrRSS 无 MCP（用 REST API + Codex skill 包替代）；Papr 走 CLI。**这个组合下没有第二个跨三平台的选择。**
2. **本地优先 + 资源可控**——folo 依赖云账号且是 Electron；MrRSS 是 WebView 壳且资源表现被用户否决；Papr/Boke 与 Tauri 同源，资源表现可以正面比，但它们的定位是纯阅读器（Papr 的 AI 走 CLI 通道）。

所以对外的一句话应当收敛成：

> **本地优先、三平台、把订阅通过 MCP 交给你的 agent 的 RSS 阅读器——AI 用你自己的 key 直连，不经过任何中间的云。**

「MCP 而非 CLI」是有意选择：MCP 是当前 AI 客户端的事实标准接入方式（quick-rss 与一批 `feed-mcp` / `rssdeck-mcp` 类第三方服务器都在用），配一次即被客户端自动发现，不需要用户学命令；代价是 MCP 场景下**按 token 计费的上下文开销**比 CLI 的紧凑输出更难控制，需要在返回体设计上补偿（见 §6 风险 3）。

## 3. 目标用户与核心场景

主用户画像：重度信息消费者 / 开发者——订阅几十到几百个源，每天需要一个「快速清空」的入口，也需要把订阅数据交给自己的 AI 工作流。

| # | 场景 | 期望 |
|---|---|---|
| S1 | 每天早上清空未读 | 打开即 Today / 全部未读，键盘连打读完，计数实时下降 |
| S2 | 深度阅读长文 | 三栏切换，正文排版干净（图片、代码高亮、表格），深浅色跟手 |
| S3 | 语言与成本敏感 | 用自己已有的 key（或本地 Ollama）一键摘要/翻译，不必为 AI 再付一份订阅 |
| S4 | 交给 agent 处理 | 在 Claude Code / Cursor 里问「这周我订阅里有什么值得看的」，由 MCP 取数并生成日报 |
| S5 | 迁移与自持 | 从现有阅读器 OPML 导入，数据留在本地，随时可全量导出 |
| S6 | 长期离线可用 | 断网仍能读完已抓取内容；没有账号与云端故障面 |

## 4. 功能需求分级

**P0（v1 必须）**：订阅管理（URL 自动发现 + 文件夹 + 排序 + 删除）、三格式解析（RSS 2.0 / Atom / JSON Feed）、条件请求与增量去重、失败源可见可重试、三栏阅读界面、正文清洗与安全渲染、已读/未读/星标、未读计数、刷新（单源/全源）、OPML 双向、SQLite 本地库 + 便携模式、键盘导航、全文搜索（FTS5）、深浅色、i18n（zh-CN / en）、**内置 AI（摘要 + 翻译，多提供方，自带 key）**、**MCP server（HTTP + stdio，只读工具集）**、三平台打包。

**P1（v1.1 起）**：全文抓取（feed 只给摘要时）、标签与自动规则、AI 文章问答与每日/每周简报、语义搜索（可选 embedding）、MCP 写入工具（标记已读/星标）与资源/提示、主题与排版自定义（字体/字号/行宽）、快捷键自定义、单实例 + 系统托盘 + 后台定时刷新、导出 Markdown 到 Obsidian / SiYuan、阅读进度与稍后读。

**P2（后续）**：FreshRSS / Miniflux 双向同步、自托管 server 与 Web 端、移动端、用户脚本 / XPath 自定义抓取、RSSHub 集成、播客与 TTS。

**明确不做**：账号体系与云同步、社区与推荐信息流、云端代付 AI、内嵌本地大模型运行时。完整列表见需求基线 `spec.md` 的 Non-goals。

## 5. 非功能需求与度量口径

数字**刻意留白只给口径**——在拿到实测基线前不接受拍脑袋的阈值。

| 指标 | 口径 | 处理方式 |
|---|---|---|
| 空闲内存 | 基准库 500 订阅 / 10k 篇；窗口前台、静置 5 分钟、无刷新任务，读进程 RSS | **先测 MrRSS / Papr / Boke 同口径基线，再定阈值**；Tauri 路径的现实预期区间需由 M0 spike 给出 |
| 冷启动 | 到可交互（首屏列表可滚动）耗时，SSD，同上基准库 | 目标 ≤ 2s，spike 后确认或修正 |
| 全量刷新 | 500 源，含网络耗时 | ≤ 60s 且 UI 不阻塞交互；并发有上限 |
| 查询延迟 | 10k 篇文章库上的全文搜索响应 | 交互可感知为即时（体感目标，M0 用真实库测一次） |
| 隐私 | 代理抓包跑完整刷新周期，统计白名单外请求 | 必须为 0（白名单 = 订阅地址 + 用户配置的 AI 端点） |
| 安全 | 构造含 `<script>` / `onerror=` / `javascript:` 的 feed 内容渲染 | 探针断言零执行；MCP 端口 `ss -ltnp` 验证仅回环监听，无 token 请求返回 401 |
| 安装包体积 | 三平台 release 资产 | 记录实测值，不预设目标 |

## 6. 风险与开放问题

1. **Linux 上的 WebView 表现**（高，已有实测数据，不再是推测）——Tauri 在 Linux 走 WebKitGTK，渲染一致性与内存都弱于 Windows（WebView2）/ macOS（WKWebView）。2026-09-20 在本机（Ubuntu 26.04 / WebKitGTK 2.52.6 / KDE Wayland / Intel Arc）实测竞品 Papr v0.15.0 的 Linux 包，三条硬数据：
   - **AppImage（87MB，自带 WebKit 2.50.4）白屏**：渲染进程每次启动都 SIGABRT，日志为 `Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...`，且不重启（60s 窗口内只出现过 1 个 web 进程，之后永久空白）。根因是打包只带了一半 WebKit——UI 进程用自带 2.50.4，而 helper 的 `RUNPATH=$ORIGIN` 指向的目录里没有这个库，于是 helper 回退到系统 2.52.6（core dump 里的 injected bundle 确认来自系统包），跨进程版本不一致使图形初始化协商失败。
   - **deb（7.8MB，不打包 WebKit，用系统库）正常**：同机同指标下渲染进程 6/6 采样存活、零 core、零 EGL 报错。
   - **启动量级参照**：deb 版渲染进程 0.3–1.8s 出现、CPU 静默（≈首帧）1.6–3.4s；换成全新数据目录、禁用 Mesa 着色器缓存都不改变这个量级。（首次安装那一次观测到约 12s 黑屏，未能复现，暂记作一次性冷启动开销。）
   - **由此定下的项目约束**：**不自行打包 WebKit，一律使用系统 WebKitGTK，并按发行版出 native 包**；即便出 AppImage，WebKit 也必须完全交给系统。“自带全部依赖”的 AppImage 在这里是负收益。
   - **M0 实测结果（自己的 Tauri 2 探针，2026-09-20，debug 构建）**：四种启动方式（会话默认 / `GDK_BACKEND=wayland` / `=x11` / 无 XWayland）全部在 0.3s 内出现渲染进程、1.9–2.3s 到 CPU 静默，**零 core 转储、零 EGL 报错**——即 Papr 那条「自带 WebKit 版本错配」的死亡路径被彻底避开（我们只依赖系统 WebKit）。
   - **内存地板**：进程树 PSS **226 MB**（主进程 86 / WebKitWebProcess 123 / WebKitNetworkProcess 17 / bwrap 0），而 Papr 的 release 构建是 **225 MB**（85 / 117 / 22）——两者几乎相同，且我们的 Rust 进程与一个完整应用（Papr）相差只有 1MB。**结论：开销主体是 WebKit 两个进程，不是应用代码。** 因此原先暫定的「空闲 ≤ 200MB」**低于这条技术路线的地板，作废**；阈值应改写为「空闲 PSS ≲ 225MB（release 基准待复核）」。（Release 构建复核进行中；同一时候选方案：接受该地板 / 换原生渲染 / 混合（仅正文视图开 WebView）。）
2. **正文渲染质量 vs 安全**（中）——AI 时代之前的老问题仍是老问题：既要保留 feed 里的图片、表格、代码块，又不能让第三方 HTML 在本地执行。清洗策略需要一份明确的允许清单 + 一条注入用例回归。
3. **MCP 的 token 效率**（中）——Papr 用 CLI + TOON 把输出压到比 JSON 省约 40% token，MCP 无法换格式（协议是 JSON）。补偿手段：默认只回元数据与纯文本摘要、正文按需取、强分页、字段精简。这一点若不做好，agent 侧体验会明显输给 CLI 方案。
4. **AI 长文超上下文**（中）——单篇文章超模型窗口时的分块/截断/映射-归并策略需要先定，否则长文摘要会静默失败。
5. **打包与签名成本**（中）——macOS 未签名会被 Gatekeeper 拦（需向用户说明 `xattr -cr`），Windows 未签名会有 SmartScreen 告警；正式签名涉及年费。是否投入取决于「自用」还是「发布」。
6. **抓取合规与礼貌**（中）——UA 标识、请求频率上限、robots 态度、单站点并发控制，需要在 P0 就写进实现约束，避免给源站造成压力。
7. **差异化前提已被部分推翻**（高）——本轮核对发现 folo 早已支持 BYOK，「自带 key」不再是稀缺能力（见 §2 与 §2.3）。任何对外定位文案都必须先对着竞品当前版本核对一遍再写，避免基于过时认知立卖点。
8. **与 Papr / Boke 的重复感**（中）——两者已经覆盖了大部分基础阅读能力，本项目的价值只能靠「对外 MCP 通道 + 资源表现」讲清楚，否则容易被评价为重复造轮子。
9. **中文源编码**（低）——GBK/GB18030 与声明错误的源需要按字节探测编码，属于已知坑，早测早安心。
10. **Linux 双显示服务器（X11 / Wayland）**（高，与 #1 同源；已定为硬需求）——Wayland 原生支持这条路上有三个有记录在案的坑，必须先验后写：
    - **AppImage 的 GTK hook 会无条件 `export GDK_BACKEND=x11`**（tauri-bundler 的 `linuxdeploy-plugin-gtk.sh`），它会覆盖用户显式设的后端、静默禁用 Wayland 原生能力（tauri#15781、#11790）；在**没有 XWayland** 的会话里，Tauri 打出的 AppImage 还会在 GTK 初始化阶段直接崩（tauri#15902）。顺便修正一条误读：Papr 那行注释引用的 tauri#8541 其实是 **AppImage 的 GSettings schema 错误**（2024-01-04 报、同日关闭），与 Wayland 无关——它的 `GDK_BACKEND=x11` 是从那里拐过来的。→ 结论：Linux 交付以 native 包（deb/rpm）为主，不把 Wayland 支持建在 AppImage 上。
    - **中文输入法是 Tauri v2 的已知问题区**：IME 候选窗位置错误（tauri#11412，报告称 v1 无此问题、v2 有）、fcitx5 在 Wayland 下候选窗不出现/无法切换（note-gen#651、tauri#5986；tauri#8264 记录了 Debian 12 KDE/Wayland 上 fcitx5 的情况）。这些报告跨版本跨发行版，**没有确认的根因或修复**，只能本项目自己实测。
    - **分数缩放受限**：Tauri 在 Linux 走 GTK3 + WebKitGTK 4.1，而 GTK3 不支持 `wp_fractional_scale_v1`，非整数缩放（如 KDE 170%）下会出现缩放与模糊问题；WebKitGTK 2.54 才换上 Skia 合成器、GTK 4.24 才宣传改善，而本机是 WebKitGTK 2.52.6。→ 分数缩放必须实测，不能假定可用。
    - **处理**：IME 与分数缩放作为 **M0 的第一优先级验收项**（先花半小时验这两件事，再写业务代码）；同时把「不强制任何后端」写成不可回退的约束。
    - **M0 实测补充（2026-09-20）**：
      - **中文输入法在 Wayland 下实测可用**：fcitx5 + WebKitGTK 2.52.6，4 次组合会话、`你好世界` 完整上屏，事件序列 `compositionstart → compositionupdate×8 → compositionend → input` 正常。原先列为「可能诏掉方案」的头号风险**降级**（但仍需补 ibus 与 X11 会话下的回归）。
      - **两个后端上报的缩放不一致**：同一台机器、同一个应用，Wayland 下 WebKit 报 `devicePixelRatio=2`、`screen=2327x1309`（对应外接屏逻辑尺寸）；X11 下报 `dpr=1.5`、`screen=3491x1964`。即 GTK3 在两个后端走的是不同缩放路径（Wayland 向上取整到整数，X11 走 Xft.dpi），清晰度需人眼判定。
      - **Wayland 下客户端不能定位窗口**（协议限制，`outer_position()` 恒为 0），因此 e2e 自动化测试里“把窗口放到指定屏”这类动作在 Wayland 后端做不了；X11 后端可以（实测 `set_position(4856,200)` 生效）。需要确定性定位的测试跑 X11 后端。

## 7. 技术决策（已拍板）

| 决策 | 选择 | 影响 |
|---|---|---|
| 平台范围 | 桌面三平台优先（Win/macOS/Linux） | 移动/Web 不进 v1，但核心逻辑与 Tauri 解耦 |
| 界面技术 | Tauri 2 + Web 前端（Rust 后端） | 视觉还原度与 HTML 正文渲染成本最低；**内存风险转移到 Linux WebView**，由 M0 spike 定量 |
| AI 范围 | 双通道：内置 AI（自带 key）+ 对外 MCP server | 工作量约等于两套 AI 集成，是 v1 的主要成本项，也是核心差异化 |
| 数据与同步 | 本地优先 SQLite + OPML；v1 不做云同步 | 需要预留同步接口但 v1 不实现；无服务端运维成本 |
| Linux 显示服务器 | X11 与 Wayland 都必须支持，不强制任何后端 | 不允许继承 AppImage 的 `GDK_BACKEND=x11` 绕法；构建需同时保留 x11 与 wayland 后端 |

由上述决策推导出的架构约束（用于后续 tech_design）：

- **单仓多 crate**：`core`（抓取/解析/存储/AI/MCP，不依赖 Tauri）+ `src-tauri`（命令层与打包）+ 前端（Web）。GUI 与 MCP **共享同一个 core**，避免两套查询逻辑漂移（Papr 的做法值得抄）。
- **AI 抽象**：provider trait + 四类实现（OpenAI 兼容 / Anthropic Messages / Gemini 原生 / Ollama），协议差异在适配层吃掉。
- **存储**：SQLite + WAL + FTS5，schema 迁移版本化。
- **MCP**：与 GUI 同进程内嵌，loopback + token，另支持 stdio 由同一二进制以子命令方式启动。
- **Linux 打包**：不打包 WebKit，使用系统 WebKitGTK，按发行版出 native 包（deb / rpm）；发行版适配属于构建矩阵问题，不允许用「自带全部依赖」的方式掩盖（失败样例见 §6 风险 1）。
- **Linux 桌面集成**：不继承 tauri-bundler 的 AppImage GTK hook（它会强制 X11，见 §6 风险 10）；构建同时保留 x11 与 wayland 后端；托盘、文件对话框、剪贴板、拖放各走会话原生路径，不支持的能力必须降级而不是崩溃。
- **剪贴板**：统一走 `tauri-plugin-clipboard-manager`（Rust 侧），不依赖 WebKit 的 Web Clipboard API。依据：M0 实测插件路径写入后，外部进程（`xsel -b -o`）能读到内容，跨进程闭环成立；而 Web API 路径尚未验证。

## 8. 里程碑建议

| 阶段 | 内容 | 出口条件 |
|---|---|---|
| **M0 技术 spike** | **先验两个最可能毙掉方案的 Linux 项：中文 IME（fcitx5）在 Wayland 下能否在 WebView 输入框正常出候选词且上屏、分数缩放下是否清晰正确**；再做 Tauri 2 骨架 + 三平台内存/启动实测（含 MrRSS/Papr/Boke 同口径基线）+ 一篇含图片与代码块的正文渲染样例 + MCP 连通性原型 | 非功能阈值可写入 `spec.md`；IME、分数缩放、双会话（X11 / Wayland / 无 XWayland）行为均有记录；Linux WebView 风险有定量结论 —— **MCP 与内存/启动/IME 已完成（见 §10）；分数缩放的清晰度判定待补** |
| **M1 可用的阅读器** | 订阅/抓取/三栏/搜索/OPML/状态管理/打包 | 能替代现有阅读器日常使用 |
| **M2 AI 双通道** | 内置 AI（摘要/翻译）+ MCP server + 客户端配置生成 | 在 Claude Code / Cursor 中能对订阅数据做一次完整问答 |
| **M3 打磨** | i18n 补全、主题与排版、托盘与后台刷新、标签规则、P1 项 | 三平台安装包 + 文档齐备 |

## 9. 待用户决策

1. **应用名与包名**（影响 crate 名、bundle id、数据目录，越早定越省事）。
2. ~~**许可证**（folo 为 AGPL-3.0，MrRSS/Papr 为 GPL 系；选 AGPL 会限制闭源与上架，选 MIT/Apache 更宽松）。~~
   **已决策（2026-09-24）：改为 AGPL-3.0-or-later + 商业授权双轨**（原 MIT OR Apache-2.0 仅对切换前版本有效）。
   决策理由、剔除的备选方案与依赖兼容性复核见 `.chorus/specs/rss-reader/2026-09-24-license-policy/adr.md`；
   落地面见同目录 `prd.md`。已知代价：不上 Mac App Store；排除 copyleft 的企业需购买商业授权。
3. **定位是「自用」还是「发布」**——直接决定签名/公证、自动更新、i18n 完整度、文档与 issue 模板的投入。
4. **是否在本仓建 Chorus 项目并把本文件镜像为 Document**（当前 `proposalUuid` / `documentUuid` 为空，尚未镜像）。
5. **首次提交的 JIRA ID**：仓库仍无任何 commit。按提交规范需要 JIRA ID，或你明确同意用 `[cr_id_skip]`。

## 10. M0 技术探针结论（2026-09-20）

探针工程落盘在 `src-tauri/`（Tauri 2 界面探针，含 `probe_log` / `clip_write` / `win_info` 等诊断命令）与 `crates/rustrss-mcp/`（MCP 原型，数据为内嵌样例），均可复跑。

| 验证项 | 结论 | 关键数据 |
|---|---|---|
| 中文输入法（Wayland） | ✅ 可用 | fcitx5 下 4 次组合会话、`你好世界` 完整上屏；事件序列 `compositionstart → compositionupdate×8 → compositionend → input` |
| 四种启动方式 | ✅ 全部正常 | 会话默认 / `GDK_BACKEND=wayland` / `=x11` / 无 XWayland：t_web=0.3s、t_settle 1.9–2.3s（debug），零 core、零 EGL 报错 |
| 内存 PSS（release） | 地板 ~215–247MB | Wayland 明细：主进程 79 / WebKitWebProcess 118 / WebKitNetworkProcess 16；对照 Papr release 225MB |
| 剪贴板 | ✅ 插件路径可用 | `tauri-plugin-clipboard-manager` 写入后由外部 `xsel -b -o` 读到；webview 原生 Ctrl+C 亦可用；`navigator.clipboard.readText` 被拒（NotAllowedError）→ **读操作必须走插件** |
| MCP 连通性 | ✅ 协议层跑通 | rmcp 3.4.0 + stdio；客户端请求 `2026-07-28`，协商回落 `2025-11-25`；4 个工具（list_feeds / list_articles / get_article / search_articles）；列表默认只回元数据 + 60 字摘要，正文按需取 |
| 多屏与缩放上报 | ⚠️ 两后端不一致 | 同机同应用：Wayland 报 dpr=2 / screen 2327×1309；X11 报 dpr=1.5 / screen 3491×1964；清晰度需人眼判定（待补） |
| X11 键盘输入 | ⚠️ 未闭环（环境侧） | 由 agent 后台 `setsid` 启动的窗口在该 KDE/Wayland 会话中收不到键盘事件（鼠标事件正常）；对照：正常启动的 XWayland 应用（飞书会议）打字正常 → **属测试方法问题，非应用缺陷**。另：`xdotool` 的 XTEST 注入在本会话对 xev / zenity 均无效（焦点已确认），不可作为验证手段 |
| 正文渲染 | ⚠️ 部分确认 | 图片、中文、输入法均正常；表格 / 代码块 / 长串溢出待确认 |

**测试方法教训（写下来防重复踩）**：交互类验证（键盘、输入法、焦点）**必须按用户方式启动应用**（启动器或用户自己的终端）；agent 后台启动的窗口只适用于自动指标（内存、启动耗时、崩溃、MCP 协议）。

**由此确定/修正的约束**：不打包 WebKit；剪贴板走 Tauri 插件；MCP 列表工具默认省略正文且分页有上限；空闲内存阈值改写为「PSS ≲ 225MB（release）」（原 200MB 目标作废）。

**M0 未完项**：① 分数缩放的清晰度人眼判定；② X11 键盘在「用户方式启动」下的闭环确认（或到真 X11 会话验证）；③ MCP 的 HTTP（loopback + token）传输与真实客户端联调——建议直接用 pi 的 MCP 网关做客户端，它是 HTTP 形态。
