---
slug: rss-reader
title: 跨平台 RSS 阅读器（Rust + Tauri 2，AI 双通道）
status: active
created: 2026-09-20
---

## Intent

做一个本地优先、跨平台（Windows / macOS / Linux）的 RSS 阅读器，用 Rust 实现。三条主张：

1. **AI 双通道**——应用内可用自己的 API key 做摘要/翻译/问答，同时把订阅与文章通过 MCP **暴露**给外部 agent（Claude Code / Cursor / Codex 等）。自带 key 已是同类产品的必备项（folo 的 BYOK、MrRSS 的多配置档、Papr 的 BYOK 都有），不当作卖点；差异点在对外 MCP 这一侧——quick-rss 有但只支持 Apple 平台，folo 的 MCP 是消费外部服务的客户端方向，MrRSS 无 MCP。
2. **原生量级的资源占用**——内存与启动开销向原生应用看齐，避开 WebView 壳应用（MrRSS 属于该形态）的资源痛点。
3. **干净克制的阅读界面**——参考 quick-rss 的三栏布局与视觉密度，不做社区/推荐信息流。

同步、账号、移动端、云 AI 代付都不在 v1 范围内（见 Non-goals）。

## Requirements

### 平台与交付

桌面三平台为 v1 目标；核心逻辑与 Tauri 解耦，便于后续接其他前端。

- [ ] Windows / macOS / Linux 三平台均可从 release 资产安装并启动，启动后能完成「添加订阅 → 刷新 → 阅读 → 标记已读 → OPML 导出」全流程
- [ ] 数据目录遵循各平台惯例（`%APPDATA%` / `~/Library/Application Support` / `~/.local/share`）；存在便携标记文件时改为程序同级 `data/` 目录，且迁移后订阅与已读状态不丢
- [x] 外部链接仅允许 http/https，且打开动作不经 shell 解析（Windows 下 URL 中的 `&` 等元字符无法注入命令；见 2026-09-22-audit-remediation-1）——⚠ **局限**：Windows 端运行时未实测（证据截至单测 + Windows 分支 `rustc --emit=metadata` 类型检查），遗留项见验证清单 §17.2

### Linux 显示服务器（X11 / Wayland）

两者都必须支持，且不允许用环境变量把用户按到某一个后端上。

- [ ] 同一份构建产物能在 X11 会话、Wayland 会话（KDE 与 GNOME 至少各测一个）、以及「Wayland 会话但无 XWayland」三种环境下启动并完成主流程
- [ ] 应用不强制设置 `GDK_BACKEND` / `QT_QPA_PLATFORM`；用户显式设置的后端选择不被覆盖，设置页或日志能看到实际生效的后端
- [ ] 中文输入法（fcitx5 与 ibus 至少各测一个）在搜索框与 AI 输入框中可用：候选窗出现在光标处、可中英切换、能提交中文；Wayland 与 X11 下都要验
- [ ] 分数缩放（125% / 150% / 170%）下窗口尺寸、字号与渲染清晰度正确，不出现文字模糊或界面缩成半屏；整数缩放（100% / 200%）无回归
- [ ] 无 XWayland 的环境下不崩溃（托盘等能力允许降级，但主流程必须可用）
- [ ] 托盘图标、文件对话框（OPML 导入导出）、剪贴板与拖放、窗口装饰在两种会话下行为一致；任一能力不可用时显式降级而非崩溃

### 订阅与抓取

- [x] 输入网站首页 URL 时能自动发现 feed（解析 `<link rel="alternate">`）；输入 feed URL 直接订阅成功
- [x] RSSHub 订阅支持自定义实例：`rsshub://path`（含 `rsshub:///` 三斜杠、大写 scheme）与 `https://rsshub.app/path` 落库统一归为 `rsshub://path`（不实例化；去重键就是该形态，两种写法判为同一订阅）；唯一抓取出口 `Store::feed_endpoint` 按当前实例解析（存量官方域行同样跟随实例），换实例零迁移；设置页「归一化 RSSHub 地址」把存量 `rsshub.app` 行一次性整理为 scheme（预览条数→确认→反馈）；添加订阅输入 `rsshub://path` 可用（发现阶段短路、不联网）；OPML 导出为 scheme 形态且回导判重
- [x] 支持 RSS 2.0 / Atom / JSON Feed 三种格式解析，含 CDATA、命名空间、多 `enclosure`、缺 `guid` 的条目（parse_formats.rs九条测试；多附件不误作文章链接）
- [ ] 单源与全源刷新可用；刷新使用 ETag / Last-Modified 条件请求，服务端返回 304 时不重复入库
- [x] 同一源重复刷新不产生重复条目；条目身份判定在 guid 缺失、link 缺失的退化场景下仍稳定（parse_formats稳定ID与store/fetch重复入库回归）
- [ ] 单个源抓取失败（超时 / 404 / 非 feed 内容）不阻断其他源刷新，失败源在 UI 上有可见标记与错误原因，可单独重试
- [x] 单源抓取受体积上限保护（8 MiB）：超限即中止下载，条目不被写入、ETag 不被覆盖（失败态按既有失败路径记录 last_status/last_error，与其它抓取失败一致），并给出可读失败原因（见 2026-09-22-audit-remediation-1）
- [x] 订阅可归入文件夹、可重命名与拖拽排序；删除订阅需二次确认（文件夹/重命名见第14节；v14组内拖拽排序持久化，取消/确认删除与重启保持已验，见subscription-order-security.md；原生跨平台拖拽未验）
- [ ] OPML 导入可容纳嵌套分组与重复 URL，导出结果可被其他阅读器（至少 FreshRSS 与 Inoreader）导入
- [x] 定时刷新与启动时刷新可配置（间隔 `off`/15/30/60/120/360 分钟，默认 30；启动后 10 秒首刷默认开）；后台刷新只发 `refresh:start` / `refresh:done` 事件，界面提示但静默更新侧栏与列表、不重渲染正文（阅读焦点与正文滚动位置保持原位）；多页已加载时列表只 prepend 新条目并保持滚动位置（在顶部 `scrollTop===0` 不补偿），单页/首屏维持整列重建；后台与手动刷新共享单 flight，不叠加
- [x] 每个源可单独设置刷新间隔：侧栏右键「刷新间隔」组（跟随全局 / 15/30/60/120/360 分钟，当前档位打勾，立即生效并持久化到 `feeds.refresh_interval_minutes`）；**覆盖优先于全局开关** —— 全局档关闭时，已单独设置间隔的源仍按各自间隔刷新，全局档改动实时影响「跟随全局」的源；设置页间隔 Hint 与菜单「跟随全局」项双语写明这条例外
- [x] 新文章通知与托盘未读角标：后台定时/启动刷新抢到单 flight 后采样未读数，前后差值 > 0 且「新文章通知」开关（默认关）打开时弹一条**聚合**系统通知（手动刷新不通知）；托盘未读数 > 0 时图标加红点、tooltip 显示「RustRss · N 篇未读」，清零后恢复原图标，托盘不可用时静默降级；通知与角标文案在 Rust 侧按 `ui.locale` 维护（与托盘菜单同一例外口径）
- [x] OPML 导入成功后自动只抓本次新增的源（`feeds_added=0` 不触发），期间状态栏提示，完成后侧栏计数自动出现

### 诊断与日志

- [x] 每次启动创建独立日志文件（`logs/` 下按启动时间命名），行内含时间戳/级别/模块，覆盖 Rust 侧关键事件与 UI 诊断行（`[ui]`）
- [x] 崩溃（panic）写入日志文件（含 payload 与位置），不改变既有 stderr 行为
- [x] 日志保留最近 20 个文件且总量 ≤50MB，启动时自动清理最旧的
- [x] 设置 → 关于 提供「打开日志目录」（系统文件管理器打开，路径不经 shell 解析）
- [x] 日志级别可在设置中切换（info/debug，默认 info，切换即时生效）
- [x] 日志中不出现 AI API key / MCP token 等敏感信息
- [x] 日志初始化失败不阻断应用启动（降级为无日志继续运行）
- [x] 从终端运行时日志镜像一份到 stderr（stdout 是终端即开；`RUSTSS_LOG_STDOUT=0/1` 强制覆盖）；镜像行为与文件行逐字节一致，非终端启动无额外输出（见 2026-09-23-log-tty）——已交付 `c36c0bb`，实机四场景（TTY / 非 TTY / env=1 / env=0）均验证，评审另做变异验证确认断言非空转

### 阅读体验

- [ ] 三栏布局：左栏（智能视图 + 订阅列表 + 未读数）、中栏（条目列表：标题/时间/摘要/已读态）、右栏（正文）
- [x] 条目列表渐进加载：滚到底自动续下一批（每批 200 条，keyset 游标分页：游标 = 上一批末行的 `(sortkey, id)`，未读优先档另带 `read` 分量），未读 / 全部 / 星标 / 稍后读 / 单源视图均支持，加载完全部匹配条目后停止（无重复、无跳条）；续页只 append 新行、不重建已有 DOM；后台刷新（多页已加载时）只 prepend 新条目并按 `scrollHeight` 增量补偿 `scrollTop`——**仅「最新在前」档**，其余排序档走整列重建（见下一条），会话内已读条目不回插，续页游标/`exhausted`/哨兵不受影响；搜索仍为一次性 200 条；列表头同时给出「已加载 M / 共 N」进度：N 按**当前视图有效筛选**取索引计数（未读筛选下就是该 scope 的未读数，因此 M ≤ N 恒成立），搜索视图不查 FTS 总数、只显示已加载（见 2026-09-23-list-count-and-unread-toggle）；尾部行三态（可点「加载更多（已加载 M / 共 N）」/「加载中…」禁用/失败可重试）与末尾终止态「已到末尾（共 N 篇）」见第 73 条（见 2026-09-23-list-count-and-unread-toggle）
- [x] 列表排序与过滤（列表头右侧图标按钮 → 菜单）：三档排序（最新在前 / 最早在前 / 未读优先）+「隐藏已读」开关，两项都是**全局设置**（`list.sort` / `list.hide_read`）且由 store 在查询时直接读取（界面与 MCP 同一条路径，`EntryQuery` 不带重复的排序字段）；切换立即重查重渲（reset 语义、分页状态重置），当前档与开关在菜单里打勾，跨视图切换与重启保留；keyset 游标按档适配（oldest 反向、unread_first 复合键 `(read, sortkey, id)`），三档续页不重不漏；迁移 v11 加表达式索引 `(read, COALESCE(published_at, fetched_at) DESC, id)`，unread_first 全程 `INDEXED BY` 钉住它且带变异校验的 EXPLAIN 断言；隐藏已读豁免星标 / 稍后读视图、搜索不豁免；开启后标读只灰显留行，行在下次重建时离开；后台刷新只在 newest 档 prepend，其余档静默 reset（无滚动锚点补偿）；i18n 双语；见验证清单第 15 节；**列表头常驻「只看未读」开关 + 快捷键 `U`**（同一 `list.hide_read` 设置、与菜单项同源同步、`aria-pressed` + 高亮态、默认关、跨重启保留；大写 `U`，输入区/覆盖层不触发）与计数口径「已加载 M / 共 N」见第 71 / 72 条（T1 `9bca8bc`；T2 见验证清单第 22.2 节）
- [x] 列表计数口径：列表头显示「已加载 M / 共 N」（有效筛选为未读时写「共 N 未读」），N 按有效筛选取索引计数（新增分组/标签总数查询带 EXPLAIN 断言与变异校验）；搜索视图只显示已加载（见 2026-09-23-list-count-and-unread-toggle）——T1 交付 `9bca8bc`（`list_scope_total` 命令 + 头部文案 + 测试）/ 实机证据见验证清单第 22 节
- [x] 「只看未读」常驻入口：列表头可见开关（复用 `list.hide_read`、默认关、与排序菜单同源同步）+ 快捷键 `U`；豁免星标 / 稍后读、不豁免搜索的既有规则不变（见 2026-09-23-list-count-and-unread-toggle；T2 交付 `b323dd8`，证据见验证清单第 22.2 节）
- [x] 加载兜底与终止态：哨兵行支持手动「加载更多（已加载 M / 共 N）」、失败可原地重试（不恢复自动重试），`exhausted` 时显示「已到末尾（共 N 篇）」而非留白哨兵（见 2026-09-23-list-count-and-unread-toggle；T3 交付 `7835f1a` + `d93d308`（评审修正勾选），证据见验证清单第 22.3 节）
- [x] 正文渲染保留段落、标题、列表、引用、表格、图片、代码块；代码块有语法高亮；相对路径图片按源站解析为绝对 URL
- [x] 条目列表支持缩略图：优先 Media RSS、再图片附件、最后摘要/正文首图；只加载 HTTP(S)，延迟加载且不发送 Referer；复用外观设置开关；远程请求由 WebView 发起，不走应用抓取代理
- [x] feed 内容中的脚本与事件处理器不在渲染上下文中执行（构造含 `<script>`、`onerror=`、`javascript:` 的 feed 内容，渲染后探针断言无执行）
- [x] 渲染层有最小 CSP 纵深（`script-src 'self'` 等）：主流程零 `securitypolicyviolation`，文章图片/AI 面板/设置页渲染正常（见 2026-09-22-audit-remediation-1）
- [x] 深浅色主题跟随系统，也可手动固定；选择跨会话保持
- [x] 字体可配置（设置 → 通用 → 字体）：界面 / 正文 / 等宽三类字体族互相独立（默认内置字体栈，正文字体跟随界面字体）；正文字号 13–18px、行高 1.5–1.8 可调；CSS 变量驱动，改完即时生效、无需重启，重启后保持；Linux 枚举 fontconfig 系统字体（`fc-list`，超时/失败降级为空表），Windows / macOS 本版只支持「跟随系统」；字号/行高滑块拖动即时预览、松手才写库；i18n 双语
- [ ] 键盘可完成主流程：上下移动、打开、返回、切换视图、搜索、标记已读、刷新、全部标记已读；`?` 显示快捷键一览
  → 2026-09-24补齐`?`弹窗与侧栏Tab/Enter/空格，Xvfb真实按键验证视图切换、j/Enter打开、搜索与Esc；完整批量菜单/刷新串联及原生Wayland仍待验，见keyboard-search.md。
- [ ] 全文搜索（标题 + 正文）可返回结果并按相关度/时间排序，10k 篇文章库内查询响应可感知为即时
  → 2026-09-24万篇SSD冷页反例：宽泛FTS214–238ms，稀疏/无结果中文单字LIKE661–706ms；即时性能未闭合，见search-offline.md。
  → v16单字候选索引后稀疏/无结果冷查16–18ms；v17相关度查询延迟读取文章元数据后，宽泛FTS首次65–77ms、热查约25ms；Xvfb桌面键入到200行渲染95ms。计划负对照及结果顺序/隐藏已读回归通过；原生Wayland/release/其它平台仍待验，AC保持开放，见search-unigrams.md。
- [ ] 复制文章标题 / 链接 / 选中正文到系统剪贴板可用，且 Wayland 与 X11 下都验；实现走 Tauri 剪贴板插件而非 Web Clipboard API
- [ ] 已读/未读、星标状态即时反映到列表与计数（批量作用域独立验收见下一条）
- [x] 「当前视图全部已读/未读」限定为当前有效筛选：全部 / 未读 / 单源 / 分组 / 星标 / 稍后读 / 标签 / 搜索；包含未加载页；未知或缺失 scope 拒绝，空搜索不操作；全部未读不宣称撤销（2026-09-23 follow-ups，实机星标 100 条回读范围外不变）
- [x] 稍后读：条目可标记/取消（阅读器按钮、列表 ⚑ 标记、`l` 键），独立于已读/星标；侧栏「稍后读」智能视图可用
- [x] 订阅右键菜单的「刷新间隔」与「移动到」以**子菜单**呈现（悬浮向右展开、点击父项切换；父项显示当前档位/当前分组，子菜单对当前项打勾）；空间不足时自动翻转/钳位不溢出视口（见 2026-09-23-submenu）
- [x] 标签（文章级）：给文章打/取消标签、按标签筛选；侧栏标签区（未读计数、置顶、颜色、拖拽排序）；打开选择器支持最近使用优先与新建；MCP 可按权限增删改查标签（`delete_tag` 需 `confirm` + `dry_run`，不进危险工具集合）（见 2026-09-23-tags）——已交付 T1 `73a4433` / T2 `4bee3cf` / T3 `f397762` / T4 `14ac1a0`，逐条证据见验证清单第 21 节
- [ ] 无网络时可阅读已抓取的全部文章，不出现阻塞式错误弹窗
  → 2026-09-24独立网络命名空间仅lo，缓存阅读、真实刷新失败后继续导航、10000条未丢通过；未逐篇打开/覆盖外部图片，见search-offline.md。
- [x] 初始化失败时窗口仍可关闭（最小事件绑定集无条件生效，见 2026-09-22-audit-remediation-1）
- [x] 阅读中标记已读/星标/稍后读不重置正文滚动位置、不清空 AI 面板内容（行级 patch 而非整区重建，见 2026-09-22-audit-remediation-1）

### AI 双通道

**通道 A：应用内 AI（用户自带 key）**

- [ ] 可配置至少四类提供方：OpenAI 兼容端点（含自建/中转）、Anthropic Messages、Gemini 原生、Ollama；每类有「测试连接」并保留原始错误信息
- [ ] 对当前文章可一键摘要（长度可选）与翻译（目标语言可选），结果落库缓存，重复打开不重复请求
- [ ] 请求内容仅限当前文章与必要指令；发送前用户可确认将要发送的内容；未配置提供方时 AI 入口不出现或明确置灰
- [ ] API key 持久化在操作系统凭据库；日志、错误提示、导出数据中不出现明文 key

**通道 B：对外 MCP server**

- [x] 可同时以 loopback HTTP 与 stdio 两种传输提供 MCP 服务，应用内直接生成可复制的客户端配置片段
- [x] 只读工具集至少覆盖：列订阅、列条目（按源/未读/**时间**过滤、**分页**）、搜索条目、取单篇正文（2026-09-23 补齐：`list_articles` 支持 `feed_id`/`folder_id`/`unread_only`/`starred_only`/`read_later_only`/`since`/`until`/`page_size`+`cursor`/`sort`/`hide_read`，新增 `list_folders`（分组 + 每组未读）与 `get_unread_summary`（按源/分组未读聚合）；时间过滤为对 `COALESCE(published_at, fetched_at)` 的闭区间，分页为 keyset 游标；⚠ 遗留：`search_articles` 仍继承界面 `hide_read`（本批 AC 外，待后续处理））
- [x] MCP 提供**写能力**（阅读状态/刷新/订阅管理），且满足：读 token 会话看不到写工具（scope 分权，且**写 token 轮换/销毁后旧值立即失效**——授权按请求现算）；写能力总开关与危险工具开关**默认关闭**；危险操作（退订/删分组）需 `confirm` 并支持 `dry_run` 预览；写操作有审计日志；列表默认口径固定为「最新在前 + 不隐藏已读」（显式参数可覆盖）——见 2026-09-23-mcp-write（2026-09-23 完成：T2 读写分权/开关/审计/写契约；T3 `set_read`/`set_starred`/`set_read_later`（ids ≤100 或条件级 {feed_id, since, until}、幂等、逐项结果）、`refresh`（三类 scope，与界面共用单 flight，进行中返回 `rate_limited`）、`fetch_fulltext`（复用 2MiB 闸门）；T4 `subscribe`（首页发现 + `rsshub://` 等价形态归一 + 幂等）、`update_feed`（tri-state patch）、`folder_create`/`folder_rename`/`folder_delete`（删组不删订阅）、`unsubscribe`（级联删条目）、`import_opml`/`export_opml`（与界面同一 core 实现，往返可回导）——全部写工具登记 scope/dangerous，危险工具默认禁用（`dangerous_tool_disabled`）+ `confirm`（`confirm_required`）+ `dry_run`（预览与实际共用同一影响面函数），每次调用（含被拒）一行 `target=mcp` 审计；证据：`cargo test --workspace` 321 passed / `cargo clippy` 无新增、`crates/rustrss-mcp/tests/{write_auth,write_tools,feed_tools}.rs` 授权矩阵与 e2e、实机 `rustrss-mcp --http` 真库跑通「subscribe → list_feeds → unsubscribe(dry_run) → unsubscribe(confirm) → 回读消失且条目级联删除」（快照见验证清单第 20 节））
- [x] MCP 仅监听回环地址；未携带正确 token 的请求一律拒绝；token 可轮换且轮换后旧 token 立即失效
- [x] 同一条查询在 MCP 与 GUI 中返回一致结果（同一 core 数据层）——`EntryQuery` 的 `since`/`until`/`feed_ids`/`sort`/`hide_read` 是**显式参数**：界面路径不传（跟随 `list.sort`/`list.hide_read` 设置），MCP 路径全传且默认固定「最新在前 + 不隐藏已读」；两条路径共用同一份 SQL 与同一批索引，不存在第二套查询逻辑
- [x] 返回体默认省略正文 HTML 大字段（元数据 + 纯文本摘要），正文按需单独取，避免 agent 上下文被单次响应撑爆

### 数据与隐私

- [x] 全部数据存于本地单个 SQLite 库（WAL 模式），进程被强杀后重启，订阅、文章、已读/星标状态与最后一次成功刷新时间均不丢（durability真实子进程强杀测试；不代表断电恢复）
- [x] 数据库 schema 带版本号与可前向迁移的迁移脚本；旧版本库升级后行数与状态一致（durability逐版本v1至v13升级v14，基础行/状态保持）
- [x] 除「订阅源地址」「用户显式配置的 AI 端点」「文章缩略图指向的图片站」三类目的地外，应用不产生任何外呼；用代理/抓包记录一次完整刷新周期（含列表缩略图加载）的全部目的地并按下方口径归类，白名单外请求数为 0（2026-09-24 实测：`strace connect` 记录一次完整周期，目的地 = 订阅源 github.blog / 图片站 raw.githubusercontent / 当前 RSSHub 实例 / 系统 DNS / 环回，`unclassified=[]`；**关闭缩略图开关后零图片请求**。证据 `2026-09-24-thumbnail-egress-policy/egress-destinations-results.json` 与 `egress-strace.log`；公网结论按站点与时间标注，不外推）
  - **口径（白名单分类，2026-09-24 裁决）**：① 订阅源地址 = feeds 表里的抓取入口（含 `rsshub://path` 按当前实例解析后的实际主机，**含抓取时的 HTTP 重定向落点**，逐跳计入订阅源侧——抓取策略是 `Policy::limited(5)`）；② AI 端点 = 用户显式配置的 provider base_url；③ 图片站 = 文章缩略图 URL 的主机（**含该请求的 HTTP 重定向目标，逐跳计入图片站侧**）。其余任何目的地都算「白名单外」，必须为 0。
  - **记录方式**：图片请求由 WebView（WebKitNetworkProcess）发起、不经应用 reqwest 通道，**应用日志看不到**——因此以代理/抓包或**系统调用层（如 `strace -f -e trace=connect`）**记录为准（应用日志可作为订阅与 AI 侧的补充），否则本项会空洞通过。
  - **⚠️ 已知例外**：缩略图由列表 WebView 直连图片站，不发送 Referer、不走应用内订阅代理（不受代理 / NO_PROXY 设置约束），图片站因此可见客户端 IP 与请求时间；关闭「显示已有缩略图」后该类请求消失。该例外**仅**覆盖缩略图；其它任何新增外呼都要单独裁决，不得援引本条。裁决与验收见 [缩略图外呼口径 PRD](2026-09-24-thumbnail-egress-policy/prd.md)。
- [ ] 无账号、无遥测、无云端依赖；卸载或删除数据目录即为彻底清理

### 非功能

- [ ] 冷启动到可交互 ≤ 2s（基准库：500 订阅 / 10k 篇文章，SSD）
- [ ] 空闲常驻内存（窗口在前台、无刷新任务、静置 5 分钟）落在一个经实测校准的预算内；先测 MrRSS / Papr / Boke 同口径基线，再定本项目阈值并写入本文件
  → **已定：空闲 PSS ≤ 260MB**。实测基线（release 构建、3 个源 / 28 篇、无操作）：**PSS 209MB**（主进程 89 / WebKitWebProcess 103 / WebKitNetworkProcess 17）。注意：这是小库基线，1 万篇规模需复测。
- [ ] 全量刷新 500 源时 UI 保持可交互（滚动与点击无卡顿），抓取并发有上限，单站点不因并发过高被拒绝
- [ ] 中文源（含 GBK/GB18030 编码与错误声明的源）解析后无乱码
- [ ] zh-CN 与 en 两份文案的 key 集合完全一致，缺 key 数为 0；界面无硬编码文案
  → **前两项已机械验证**：启动时自检比对两份字典并上报（现 109 个 key，`i18n selftest ok`）；`index.html` 里已无游离在 `data-i18n` 之外的面向用户中文。
  → **仍缺**：Rust 侧的错误文案仍是中文（如「打开数据库失败」「只允许打开 http/https 链接」）。要做得多返回错误码、由界面翻译，本轮未做（已知缺口）。
- [ ] 安装包体积报出实测值（三平台分别记录）

## Non-goals

- 账号体系、云同步、社区/发现/推荐信息流（folo 方向）
- 移动端（iOS / Android）与浏览器端
- 云端代付 AI（所有 AI 请求走用户自己的 key 直连）
- 内嵌本地大模型运行时（本地能力仅通过 Ollama 等外部端点接入）
- 第三方 RSS 服务的双向同步（FreshRSS / Miniflux）——列为后续阶段，v1 只需架构上不堵死
- 用户脚本 / XPath 自定义抓取规则、RSSHub 路由集成、播客与 TTS

### 2026-09-23 遗留修复验收

- [x] 分组名称进入分组列表，箭头独立折叠；分组分页/排序/计数/批量标记闭合，空分组不退化全库。
- [x] 旧列表/续页/搜索/总数/正文请求不能覆盖更新后的视图；标签变更更新总数，尾部文案与禁用属性同值零写入。
- [x] 凭据库访问及其重试不持有数据库锁，设置页可显示凭据不可用状态。
- [x] 单源/分组/标签计数有独立 OS 冷页缓存与热缓存证据，明确区分 SQL 首次耗时、打开库耗时与整窗延时。
- [ ] 系统 KWallet 安装带 #514194 修复的包并验证运行中的守护进程（sudo 交互认证阻挡）。
- [x] 分组首屏首次慢查询进一步分层测量与优化：v15排序索引覆盖feed_id，独立SSD冷页样本中core查询528ms→26ms、热查88ms→2.7ms；生产SQL字节码与旧索引变异测试守护。整窗与其它平台延时未据此宣称达标，见scheduler-performance.md。

### 2026-09-23 完整主题与 MCP 视觉预览（Linux 已实现，聚合验收有 open item）

设计与验收详见 [PRD](2026-09-23-theme-preview/prd.md)、[技术设计](2026-09-23-theme-preview/tech_design.md)、[分阶段任务](2026-09-23-theme-preview/tasks.md)。

- [x] 主题/设置/MCP 预览方案与任务草案落库；Linux 独立 WebKitGTK 原生截图探针成功（6 次，固定示例；仅底层可行性，不代表 Tauri/MCP 产品交付）。
- [x] Linux Tauri 隔离截图 example 在 Xvfb 100%/200% 各完成 100 帧像素验证，覆盖隐藏/尺寸与字节预算/超时/失效版本/并发/关闭错误；可选 feature，不接入正式产品。证据见 [T1 报告](2026-09-23-theme-preview/tauri-capture-spike.md)；Windows/macOS/Wayland 与真实 WM 最小化未验收。
- [ ] 三套整套视觉预设与用户覆盖，统一配色/字体/列表/阅读区参数；UI/MCP 共用校验/迁移/版本/恢复。
- [x] T2 core 数据模型与存储：三预设明暗值、严格 patch 校验、旧设置只读映射、CAS/同值零写、最近 10 份历史与单调恢复；13 项专项测试覆盖。当时仅 core，后续 UI/MCP 接入见 T3/T5；见 [core 交付](2026-09-23-theme-preview/core-theme-model.md)。
- [x] 当前 KDE 原生 Wayland 的隔离截图探针通过 100 帧与 8 类边界检查；实际 GdkWaylandDisplay，WebView 内容尺寸与 JS viewport/DPR 相符，修正装饰区域导致的尺寸误报。仅当前会话验收，跨屏/GNOME/真实最小化/生产 MCP 仍未验收，见 [Wayland 证据](2026-09-23-theme-preview/wayland-snapshot-results.json)。
- [x] T3 UI 接入 core：现有主题/字体设置统一存储，三套预设选择、共享语义 CSS 参数与真实组件 fixture、同值零写/阅读锚点保护；Linux X11/当前 KDE Wayland 专项验证见 [T3 报告](2026-09-23-theme-preview/shared-theme-renderer.md)。
- [ ] 真实订阅列表的缩略图元数据来源与展示（T3 仅已有图片元素样式开关和本地 fixture 验证）。
- [x] T4 设置七类重组、独立外观页与 Aa 共用排版、草稿预览/保存/历史恢复；旧非主题控件 ID 保留，批量操作移至列表头。Linux Xvfb 真产物七类键盘导航/焦点回环与双语检查通过，见 [T4 证据](2026-09-23-theme-preview/t4-settings.md)。跨平台和读屏器未验收。
- [x] T6 Linux：MCP 临时修改→真实组件渲染→PNG 与版本→再调整→保存/取消；内嵌与独立 stdio 同库桥接验证通过，见 [T6 报告](2026-09-23-theme-preview/mcp-theme-preview.md)。其它平台及第三方 GUI 客户端未验收。
- [x] T6 Linux：临时配置不落库、CAS 防覆盖、ready/原生像素双确认、超时取消与失权回收；Xvfb 正文无重建。TTL 用时钟边界测试，未做 30 分钟墙钟长测；阅读锚点由 T3 复用，Wayland 正文位置未在 T6 重验。
- [ ] Windows/macOS/Linux X11/Wayland 原生截图及真实客户端闭环完成运行验证；图片尺寸/体积有界。

- [x] T5 MCP 主题配置五工具：共用 core 校验/CAS/历史；默认权限、轮换、审计；内嵌事件与独立 stdio 前台轮询同步，Linux Xvfb 真产物验证见 [T5 报告](2026-09-23-theme-preview/mcp-theme-config.md)。本项不代表截图接口或跨平台验收。

- [x] T7 跨阶段契约修复：固定预览适配 T4 共享外观编辑器与资源协议；ID/脚本/CSS token 回归测试，重建后 Linux MCP 25 图通过。
- [x] T7 共享组件完整矩阵：三预设×明暗×三场景×双语，在 Xvfb 100%/200% 覆盖72组合；当前 KDE Wayland 200%另36组合，见 [T7报告](2026-09-23-theme-preview/t7-aggregate.md)。
- [x] T7 原生 Wayland 真实点击取消 + 正文位置：2026-09-24 用户在场补验通过，isTrusted=true、取消不落库、窗口销毁、节点/偏移保留；见 [现场证据](2026-09-23-theme-preview/t7-wayland-input-results.json)。原先因用户不在场而保留的 open item 已关闭。
- [ ] T7 第三方GUI客户端、空状态专项、其它平台/读屏器/分数缩放及用户侧独立聚合评审。

- [x] T7 真实墙钟寿命：空闲599.75秒/绝对1800.52秒回收，过期保存被拒、临时零写；60次RSS/目录/磁盘采样，详见 T7 报告。

- [x] T7 评审 P2 收尾：移除 app.js 已失效的旧主题/字体控件路径；全部 UI JS 的静态 ID 创建契约、脚本/样式资源服务契约及变异校验已通过，见 [收尾报告](2026-09-23-theme-preview/p2-contract-cleanup.md)。本项不关闭原生 Wayland 输入 open item。

- [x] Codex CLI 实际客户端模型收图、调整、保存/取消与 SQLite 回读；主题工具发布 core 对象 patch schema，回归先红后绿。终端仅图片标记，未代替第三方 GUI 验收；修复后新 Codex 会话目录已复测：对象声明、validate/preview 首次成功，见 [报告](2026-09-23-theme-preview/t7-codex-client.md)。

- [x] MCP 截图文件契约：preview/capture 返回绝对 PNG 路径及到期时间，不返回内联图片；工具描述要求本地读图，能力声明共享文件系统前提；core 管理私有文件、配额、到期/失权回收，保存取消保留文件，桌面退出主动清理。单元/协议验收及运行范围见 [文件输出报告](2026-09-23-theme-preview/mcp-preview-files.md)。

- [x] 字体建议异步更新：外观/阅读/Aa 加载完成即刷新建议，不重置焦点与草稿；失败允许下次打开重试，已关闭编辑器忽略晚到刷新。两条复现测试先红后绿；运行证据见 [后续体验检查](2026-09-23-theme-preview/file-probe-ui-followup.md)。
- [x] MCP 文件600秒真实时钟回收：无模型客户端脚本检查创建/重拍/保存/取消/PNG权限尺寸/SQLite回读，等待期间无新MCP请求，605.01秒三文件均回收；见 [补验报告](2026-09-23-theme-preview/file-probe-ui-followup.md)。

- [x] KDE原生Wayland补验：外屏125%/150%主界面与设置弹窗、用户清晰度确认和真实点击通过；真实系统浅/深切换跟随system，显式light/dark不受影响，SQLite无写与正文锚点保持，结束恢复原桌面。范围见 [原生验收](2026-09-23-theme-preview/native-settings.md)。

- [x] 区分无订阅与空搜索提示；初次抓取失败不误报成功，保留订阅并提示刷新重试。回环HTTP的503/无效RSS/断连接与恢复均保留缓存，双语UI补验见 [空态与失败报告](2026-09-23-theme-preview/empty-error-states.md)。

- [x] 订阅抓取稳定错误码及双语提示：响应体超时不再记为http_200，数据库/刷新报告共用code，自动发现保留结构化错误；DNS注入、TLS自签名拒绝、生产30秒超时、429与缓存/恢复补验通过。范围见 [网络补验](2026-09-23-theme-preview/network-error-followup.md)。
- [x] 订阅429/503的Retry-After持久化等待；期限内桌面/MCP刷新不发请求，重启保留、到期恢复及缓存不变（network-proxy-backoff.md）。
- [x] 应用内环境/直连/自定义HTTP(S)代理与绕过列表；桌面/MCP共享配置，订阅/发现/全文/AI路径接入；不保存代理凭据。已验HTTP路由/切换/失败，HTTPS仅CONNECT拒绝，跨平台未验（network-proxy-backoff.md）。
