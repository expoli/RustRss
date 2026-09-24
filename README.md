# RustRss

跨平台（Windows / macOS / Linux）RSS 阅读器，Rust 实现。核心主张：**本地优先 + AI 双通道**（应用内自带 key 的摘要/翻译 + 对外 MCP server 让 agent 直接读你的订阅）。

- 需求基线：`.chorus/specs/rss-reader/spec.md`
- 当前阶段分析（竞品、风险、决策）：`.chorus/specs/rss-reader/2026-09-20-initial-requirements/prd.md`
- T7 Linux 跨阶段验收与已知缺口：[执行报告](.chorus/specs/rss-reader/2026-09-23-theme-preview/t7-aggregate.md)。T4 后的 MCP 固定预览已修复，共用只读外观编辑器；原生 Wayland 真实点击取消与正文位置已在用户在场时补验通过。用户侧独立评审已通过；两项 P2 的死引用清理与全 UI 契约守护见 [收尾报告](.chorus/specs/rss-reader/2026-09-23-theme-preview/p2-contract-cleanup.md)。
- 主题与 MCP 视觉预览设计（UI 主题与 Linux MCP 预览已接入）：[需求与交互稿](.chorus/specs/rss-reader/2026-09-23-theme-preview/prd.md)、[技术设计](.chorus/specs/rss-reader/2026-09-23-theme-preview/tech_design.md)、[截图可行性证据](.chorus/specs/rss-reader/2026-09-23-theme-preview/capture-feasibility.md)。
- 开发用 Tauri 截图探针（独立 example，Linux 已验证，尚未接入主题/MCP）：[构建方式与验证证据](.chorus/specs/rss-reader/2026-09-23-theme-preview/tauri-capture-spike.md)。
- 主题 core API 已实现：三预设/明暗参数、局部更新、旧设置映射、CAS 版本与历史恢复；UI 与 MCP 配置工具已接入，见 [T2 说明](.chorus/specs/rss-reader/2026-09-23-theme-preview/core-theme-model.md)。

- 设置现分为外观与主题、阅读、订阅与更新、AI、外部集成、数据与备份、通用七类。外观页可独立选择浅/深色 Clear / Paper / Slate，并编辑颜色、字体、列表和栏宽；切换预设保留覆盖，只有“使用整套预设”清除全部覆盖。修改先在局部示例预览，保存后应用；关闭放弃草稿。支持单项跟随预设、最近 10 份历史恢复与版本冲突提示。
- 正文工具栏 **Aa** 与阅读设置共用排版字段和 core 保存路径；保存后保留当前正文节点与段落锚点。当前视图批量标读/未读移至列表头 **✓** 菜单，scope 与原确认流程不变。设置支持方向键/Home/End 切页、Tab 焦点回环、Esc 关闭。见 [T4 实现与验收](.chorus/specs/rss-reader/2026-09-23-theme-preview/t4-settings.md)。
- 文章列表可显示缩略图：优先采用 Media RSS 缩略图，其次图片附件，再回退到摘要/正文首图；只接受 HTTP(S)，已有外观设置可关闭。列表直接由 WebView 延迟加载远程图片，不发送 Referer；**此请求不走应用内订阅代理**（不受代理 / NO_PROXY 设置约束），图片站会看到客户端 IP 与请求时间。这是「除订阅源与显式配置的 AI 端点外零外呼」的**已知例外**，例外仅覆盖缩略图（关掉开关后请求消失）；分类口径与验收见 [缩略图外呼口径 PRD](.chorus/specs/rss-reader/2026-09-24-thumbnail-egress-policy/prd.md)。

主题 MCP 的 `validate_theme`、`update_theme`、`preview_theme` 在工具目录直接发布 core 的对象 patch schema；传 JSON 对象，不传编码后的字符串。Codex CLI 已完成收图→调整→保存/取消补验，用户终端仅显示图片标记；GUI 图片展示仍未验收，见 [客户端报告](.chorus/specs/rss-reader/2026-09-23-theme-preview/t7-codex-client.md)。

原生 KDE Wayland 的125%/150%缩放、设置弹窗与真实点击已补验，用户确认清晰；系统明暗切换跟随及手动模式保持也通过。仅覆盖本机外屏，详情见 [原生验收](.chorus/specs/rss-reader/2026-09-23-theme-preview/native-settings.md)。

### 空列表与抓取失败

无订阅时会提示添加订阅或导入 OPML，搜索无结果时提示更换关键词。订阅已添加但首次抓取失败时，状态栏明确提示失败与刷新重试，已添加的源会保留；抓取失败不会清除缓存文章。验证范围见 [空态与网络失败报告](.chorus/specs/rss-reader/2026-09-23-theme-preview/empty-error-states.md)。

订阅发现与抓取失败现在按稳定错误码显示双语说明，包括超时、连接、HTTP限流、解析及内容读取失败；原始诊断保留在日志/数据库。DNS与TLS连接失败使用统一连接提示，生产请求时限仍为30秒。故障注入与覆盖限制见 [网络错误补验](.chorus/specs/rss-reader/2026-09-23-theme-preview/network-error-followup.md)。

### 字体建议加载

外观、阅读与 Aa 共用系统字体建议。首次异步加载完成后，已打开的编辑器会更新建议，保留当前焦点、输入文字和未保存草稿；加载失败时，下次打开可重试。也可以手动输入逗号分隔的字体族与通用回退字体。

### MCP 预览截图文件

可用 `/usr/bin/python3 scripts/verify-theme-preview-files.py` 代替模型执行文件协议验收：隔离 Xvfb 实例中创建、重拍、保存、取消、读取 PNG 与 SQLite，并实际等待600秒回收；需先构建桌面与 `theme_fixture` 示例。此脚本验证文件交付，不判断模型是否理解图片。

`preview_theme` / `capture_theme_preview` **只返回本地文件信息，不再返回内联图片或 base64**。成功响应的文本和 `structuredContent` 包含 `image_path`（绝对路径）、`image_mime_type`、`image_bytes`、`image_expires_at_ms`、`image_read_instruction`；原有 `capture.pixel_size`、预览版本和配置 hash 仍用于核对。

Agent 应先用 `view_image` 或客户端读图工具打开 `image_path`，实际看图后再调整/保存；不能读文件时应报告视觉验证不可用，不能根据路径或元数据声称看过图。`get_theme.capabilities.preview` 声明 `image_delivery=local_file`、`requires_shared_filesystem=true`、`inline_images=false`：需要与桌面进程共享文件系统和访问权限，远程客户端或隔离容器不自动可用。独立 stdio 沿同库桥接返回桌面写出的同一路径。

截图位于系统临时目录（Linux 通常 `/tmp/rustrss-preview-*`），每次生成独立 PNG；Unix 目录0700、文件0600。单张≤2MiB，每个桌面预览服务最多32张/32MiB。生成后600秒到期（后台每5秒清理），保存/取消不提前删图；token失效、关闭 MCP、桌面正常退出会提前清理。容量满返回 `preview_file_limit`，应等旧文件到期；文件I/O失败返回 `preview_file_unavailable`，候选ID仍可取消。崩溃/强杀可能留下该次私有目录，由系统临时目录维护回收；程序不扫描删除其它实例的文件。旧客户端需重新加载工具目录并适配文件返回契约。

## 已定决策

| 项 | 选择 |
| --- | --- |
| 平台 | 桌面三平台优先（Win / macOS / Linux），**Linux 需同时支持 X11 与 Wayland** |
| 界面技术 | Tauri 2 + Web 前端（Rust 后端） |
| AI | 双通道：内置 AI（用户自带 key）+ 对外 MCP server |
| 数据 | 本地优先 SQLite + OPML；v1 不做云同步 |
| 许可证 | MIT OR Apache-2.0 |
| 定位 | 先自用；发布能力留在架构里但不投入 |
| 字体 | 内置字体栈 + 用户可覆盖三类字体族（界面 / 正文 / 等宽）与正文字号、行高；字体枚举只在 Linux 走 fontconfig（`fc-list`），Windows / macOS 本版不枚举本机字体，可输入字体族名或跟随预设（不引 font-kit 这类重依赖） |
| bundle id | `tech.expoli.rustrss` |

## 硬约束（来自竞品实测，见 PRD 风险 1 / 10）

- **不打包 WebKit**，一律使用系统 WebKitGTK，按发行版出 native 包。
- **不强制显示后端**（不继承 AppImage 那套 `GDK_BACKEND=x11`），X11 与 Wayland 都要原生可用。

## 仓库结构

```
crates/rustrss-core/   核心库：解析 / 身份判定 /（待补）存储与抓取，与界面解耦
crates/rustrss-mcp/    MCP 服务器（stdio；M0 原型接内嵌样例数据）
src-tauri/             Tauri 2 桌面应用（当前为 M0 诊断探针）
ui/                    桌面应用前端（当前为探针页面）
```

- MCP 主题配置已接入：读取/校验/保存/历史恢复共用 core，内嵌事件与独立进程轮询同步 UI；[T5 说明与验收](.chorus/specs/rss-reader/2026-09-23-theme-preview/mcp-theme-config.md)。Linux 已接入临时预览与 PNG 返回，见 [T6 验收](.chorus/specs/rss-reader/2026-09-23-theme-preview/mcp-theme-preview.md)。Windows/macOS 预览明确返回不可用。

### MCP 服务器（stdio + HTTP 两种传输，已接真实库）

工具集：**只读** 11 个 —— `list_feeds` / `list_folders` / `list_articles` / `get_article` / `search_articles` / `get_unread_summary` / `db_stats` / `list_tags` / `get_theme` / `list_theme_presets` / `validate_theme`；**写** 23 个 —— `set_read` / `set_starred` / `set_read_later` / `refresh` / `fetch_fulltext`（阅读状态与刷新）+ `subscribe` / `update_feed` / `folder_create` / `folder_rename` / `folder_delete` / `unsubscribe` / `import_opml` / `export_opml`（订阅管理，其中 `folder_delete` / `unsubscribe` 是**危险工具**，另受危险开关约束）+ `create_tag` / `rename_tag` / `assign_tags` / `unassign_tags` / `delete_tag`（标签，全部非危险：删标签只清关联、不删文章）。另有 `update_theme` / `restore_theme`（主题配置，非危险）以及 `preview_theme` / `capture_theme_preview` / `finish_theme_preview`（主题预览，非危险）。写工具默认不可用，需要写 token + 写开关，见下面的权限模型。

口径：**列表只回元数据 + 短摘要（≤140 字），正文必须用 `get_article` 单独取**；所有列表有上限（默认 10、上限 50）。这是为了不让单次响应撑爆 agent 上下文（见 PRD §6 风险 3）。

`list_articles` 的默认口径**固定**为 `sort=newest` + 不隐藏已读，**不继承界面设置**（`list.sort` / `list.hide_read`）——agent 拿到的默认视图不该被用户此刻的界面选择左右。可用参数：`feed_id` / `folder_id`（二选一）、`tag_id` / `tag_name`（二选一，按标签筛选，未知标签报 `tag_not_found`；同时给报 `invalid_argument`）、`unread_only`、`starred_only`、`read_later_only`、`since` / `until`（对 `COALESCE(published_at, fetched_at)` 的**闭区间**，Unix 秒）、`sort`（`newest` / `oldest` / `unread_first`）、`hide_read`、`page_size`（别名 `limit`）。每条条目带 `tags`（该条目的标签名数组，最多 20 个，超过置 `tags_truncated=true`；标签 id 用 `list_tags` 取）。翻页是 keyset 游标：把上一页返回的 `next_cursor` 原样回传给 `cursor`，不重不漏（游标带排序档、`unread_first` 档还带 `read` 分量——换档复用旧游标会明确报错，而不是翻出错页）；不满页时 `next_cursor` 为 `null`。

`list_folders` 给分组 + 每组未读合计（未分组单列 `ungrouped_unread`），`get_unread_summary`（`by=feed|folder`）给完整的未读分组清单（未读为 0 的组也出现）。三者与界面走 core 的同一条数据路径（硬约束 1）：`EntryQuery` 上的 `since` / `until` / `feed_ids` / `sort` / `hide_read` 只是**显式传参**——界面路径不传（继续跟随设置），MCP 路径全传（默认口径因此与界面设置解耦），不存在第二套查询逻辑。

#### 写工具口径（阅读状态 / 刷新 / 全文）

三个状态工具（`set_read` / `set_starred` / `set_read_later`）共用一套形状，只有「写哪一位 + 取值字段名」不同：

- **目标二选一**：`ids[]`（≤100 条；`read` / `starred` / `later` 缺省 `true`，传 `false` 即撤销）**或**条件级 `{feed_id, since, until}`（至少给一个；`since` / `until` 是闭区间，比较键与列表排序键同源 `COALESCE(published_at, fetched_at)`）。**混用或都不给 = `invalid_argument`**——条件级漏参绝不能退化成「改全库」（core 侧对空条件也会拒：`Store::set_flag_scoped` 返回 `Invalid`）；
- 返回统一信封 `{ok, affected, results, error_code?, detail?}`：`affected` 是**命中条数**（含此前已是目标状态的行，与 `set_read(ids)` 的 SQLite 计数口径一致）→ 重复调用返回值稳定，即**幂等**；`results` 逐项给（ids 形态按调用方给的顺序，缺失的 id 标 `article_not_found`；条件级给命中集里前 100 个 id，截断时 `detail.results_truncated=true`）。写的是与界面**同一个库、同一条 store 路径**，所以 `db_stats` 与界面计数立即一致；
- 条件级写入走索引（feed 等值 / sortkey 表达式索引，EXPLAIN 断言 + 变异校验在 core 测试里），不会退化成扫正文大列的全表扫。

`refresh`：`scope` = `all` / `feed_ids`（配合 `feed_ids[]`，≤100）/ `folder`（配合 `folder_id`），不传时按参数推断；与界面刷新**共用同一个单 flight**（应用内托管时是同一个 `Arc<RefreshGate>` 实例）——已有刷新在跑时本轮不执行，返回 `error_code=rate_limited`（不排队、不叠加），界面侧不受影响。返回 `affected` = 本轮入库条目数（新增+更新），`results` 只列失败源（`fetch_failed`），`detail` 给本轮摘要：`feeds` / `fetched` / `not_modified` / `inserted` / `updated` / `unchanged` / `failure_count` / `failures`（沿用 core 的刷新统计）。

`fetch_fulltext`：给单篇摘要型条目补全文（抓原文页 → 提取 → 写回），复用既有全文抓取与 **2MiB 流式闸门**（Content-Length 预检 + 下载中累计超限即断）。已抓过/全文型时**零网络**返回（`affected=0` + `detail.already_fulltext=true`）。**正文不随响应返回**：写回后用 `get_article(id)` 取（与列表口径一致，避免一次响应撑爆上下文）。错误码：`invalid_url`（条目没有原文地址）/ `fulltext_too_large` / `fulltext_bot_challenge`（站点要浏览器验证）/ `fulltext_not_html` / `fulltext_no_content` / `fetch_failed`（网络或 HTTP 错，可重试）/ `article_not_found`。

#### 写工具口径（订阅管理）

八个订阅管理工具都走**与界面同一批 `Store` 方法**（硬约束 1：MCP 与界面是同一数据层的两个调用方）。归一化有两个跨 crate 复刻的语义（`src-tauri` 的实现 MCP 不能直接调），测试逐条钉住：

- **自定义标题**：`trim` → 空串 = 清除（显示回退源站名）→ 按字符截断 200（与界面 `normalize_custom_title` 同口径）；
- **每源刷新间隔**：只认白名单 `15/30/60/120/360`，JSON `null` = 跟随全局（界面下拉里的 `"global"`），非法值报 `invalid_argument` 而**不静默回默认档**。

`subscribe`：`url` 支持站点首页（自动发现——输入本身能按 feed 解析就用输入地址，否则扫 head 里的 `<link rel="alternate">`，与界面「添加订阅」同一条 core 路径）与 `rsshub://path`（三斜杠 `rsshub:///path`、大写 scheme、官方域 `https://rsshub.app/path` 都归一为 `rsshub://path`，且**不联网**——换镜像零迁移的前提）。**幂等**：地址已在库里时不报错，返回 `ok=true`、`affected=0`、`detail.already_subscribed=true` 与既有 `feed_id`；直接给已订阅的 feed 地址时连发现请求都不发。错误码：`invalid_url`（空 / 非 http(s) / 无主机名 / 页面里没发现 feed）、`fetch_failed`（网络或 HTTP 错，可重试）。新订阅的 `title` 在首次抓取前等于地址，抓取后由源站名更新（与界面一致）。

`update_feed`：`feed_id` + **tri-state patch**——键缺省 = 不动；`custom_title: ""` = 清除自定义名；`folder_id: null` = 移出到未分组；`refresh_interval_minutes: null` = 跟随全局档。三个字段一个都不给 → `invalid_argument`（空 patch 多半是参数拼错，不静默空操作）。错误码：`feed_not_found` / `folder_not_found` / `invalid_argument`。

`folder_create` / `folder_rename`：同名建组返回既有 id（`detail.already_exists=true`，不报错）；重命名重名或空名 → `invalid_argument`，组不存在 → `folder_not_found`。

`folder_delete`（**危险**）/ `unsubscribe`（**危险**）：先过危险开关（关着 → `dangerous_tool_disabled`，且 `tools/list` 里不出现），实际执行必须 `confirm: true`（缺 → `confirm_required`）。`dry_run: true` **不需要 confirm**（预览是只读的）且不落库；**预览与实际执行共用同一个影响面函数**，所以「预览几条就真删几条」：`unsubscribe` 用 `Store::entry_count_for_feed`（单源 COUNT 走 `(feed_id, read)` 覆盖索引，不穿正文大列的表 B 树，EXPLAIN 断言 + 变异校验在 core 测试里），`folder_delete` 用 `Store::feed_ids_in_folder`。语义与界面一致：**删组不删订阅**（组内订阅移出到未分组，并清掉侧栏折叠状态里的孤儿 id），退订则级联删除该源的全部条目。

`import_opml`（`path` 本地文件或 `content` 文本，二选一；≤8 MiB）与 `export_opml`：复用 core 的 `opml::import|export`——与界面「导入 / 导出 OPML」同一条实现。导入按 `xmlUrl` 去重（已存在记 `skipped`、不移动分组），嵌套分组压平成 `父/子`；返回 `affected` = 新增数，`detail` 含 `added` / `skipped` / `errors`（成功时空数组）/ `folders_created` / `outlines_ignored`。导出把 OPML 文本放在 `detail.opml`（**不写文件**），同一个文本可原样回导（二次导入全部 `skipped`，有往返测试）。

#### 写工具口径（标签管理）

标签工具走 core 的 `Store` 标签 API（与界面同一数据层），“打标签”的效果与界面里按 `t` 打标完全一致：

- `list_tags`（**只读**）：`sort=sidebar`（默认：置顶优先 → 手动顺序 → 名称）或 `recent`（最近使用优先，没用过的垫底）；每行含 `id` / `name` / `color`（`#rrggbb` 或 `null`）/ `pinned` / `unread`（该标签下**未读**条目数）/ `sort_order` / `last_used_at`。只回元数据（无条目正文），一次最多 200 个：超出时 `truncated=true` 且 `total` 给全量口径。
- `create_tag`（`name` + 可选 `color`）：`name` trim 后非空，大小写不敏感唯一——重名报 `duplicate_tag_name`（**不是**幂等返回既有 id，与 `folder_create` 有意不同：建标签重名多半是撞名而不是重试）；`color` 必须是 `#RRGGBB`，否则 `invalid_argument`。
- `rename_tag`（`tag_id` + `name`）：口径同 `create_tag`；`tag_id` 不存在 → `tag_not_found`；允许仅改大小写。
- `assign_tags` / `unassign_tags`：目标与 `set_read` 同形——`ids[]`（≤100）或条件级 `{feed_id, since, until}`（至少一个；`since` / `until` 为闭区间，口径同列表排序键）；混用/都缺 → `invalid_argument`（条件级漏参绝不能退化成全库打标）。`tag_ids` 1..=100 个（来自 `list_tags`；有一个不存在 → `tag_not_found` 且本次零改动）。返回写信封：`affected` = 命中条目数（重复调用稳定 = 幂等），`detail.changed` = 本次真正新增/移除的关联行数（重复调用为 0），不存在的条目 id 逐项标 `article_not_found`。取消打标**不**推进 `last_used_at`（见下一条）。
- `delete_tag`（`tag_id`；`confirm` / `dry_run`）：只清 `entry_tags` 关联、**不删文章**（`affected` = 该标签下将失去关联的篇数）；预览走 core 的 `Store::delete_tag(id, true)`，与实际执行共用 `Store::tag_entry_count`，所以「预览 N 篇」与「真删影响 N 篇」不可能漂移。tag 删除**不在危险工具集合**（不进 `mcp.dangerous_enabled`），只要求写能力 + `confirm: true`；`dry_run: true` 的预览不需要 `confirm`。
- 错误码（机器可读，不靠文案）：`tag_not_found` / `duplicate_tag_name` / `invalid_argument`（空名/非法颜色/空 `tag_ids`/超 100/目标形态混用或缺失）/ `confirm_required`，沿用 `write_scope_required` / `write_disabled`。
- `last_used_at` 只由**打标**推进（`assign_tags`），取消打标不推进——与 PRD FR-1 / tech_design 「打标时更新」同口径（批次收口时的最小 core 修正）。

两种传输：

- **stdio**：客户端把 `rustrss-mcp` 当子进程拉起；
- **HTTP**（streamable HTTP）：`rustrss-mcp --http 127.0.0.1:8817`（token 取 `RUSTSS_MCP_TOKEN`，未设则随机生成并打印），或在应用「**设置 → MCP**」里启用——设置页会直接给出可粘贴的客户端配置片段与一行 `claude mcp add` 命令，并可一键复制。

约束（均有测试）：**只绑回环**（非回环地址直接拒绝启动）、无 token / 错 token 一律 401、`/health` 不鉴权且不含任何订阅数据、token 轮换后旧值立即失效。客户端需带 `Accept: application/json, text/event-stream`——这是 MCP 传输规范的要求（rmcp 不对则 406），不是本项目的额外限制。

#### 权限模型（读写分权）

认证与授权分离：**token 对不对**决定能不能连上（传输层 401），**这个请求能不能写**决定工具级结果（200 + `isError` + `error_code`）。默认装好就是只读。

| 凭据 / 开关 | 默认值 | 作用 |
|---|---|---|
| `mcp.token`（读 token，常驻） | 首次启用时自动生成 | 连接认证 + 只读工具；`tools/list` 里看不到任何写工具 |
| `mcp.write_token`（写 token，48 位十六进制） | **不存在** | 设置页「生成 / 轮换 / 销毁」；不存在时写工具**不注册**（列不出来也调不动） |
| `mcp.write_enabled`（写能力总开关） | **关** | 关着时写工具不注册；调用返回 `write_disabled` |
| `mcp.dangerous_enabled`（危险工具开关） | **关** | 只管不可逆操作（`unsubscribe` / `folder_delete`）；关着时调用返回 `dangerous_tool_disabled` |

- **HTTP**：每个请求按携带的 token **现算** scope——写 token → 写能力（还需两个开关），读 token → 只读；`tools/list` 也按当次请求的 scope 过滤（读 token 的会话看不到写工具）。**不缓存会话级 scope**：写 token 轮换或销毁后，旧值的**下一个请求立刻失效**（包括已建立的连接），不需要重启服务。
- **stdio**：没有凭据概念，写能力由同一套开关把关（写开关 + 写 token 必须都已就位），与 HTTP 同口径。
- **无权限时返回工具级错误码**：`write_scope_required`（没有写凭据）/ `write_disabled`（开关或写 token 没就位）/ `dangerous_tool_disabled`；危险操作还要 `confirm: true`（缺失 → `confirm_required`），并支持 `dry_run: true` 只预览影响面、不落库。业务错误码同样机器可读：`article_not_found` / `invalid_argument`（ids 为空/超限/两种目标混用/条件缺失）/ `rate_limited`（刷新进行中）/ `feed_not_found` / `folder_not_found` / `tag_not_found` / `duplicate_tag_name` / `fulltext_*`（见上）。被拒的调用一律不改库（有测试钉住）。
- **审计**：每次写调用（含被拒的）落一行 `target=mcp` 日志（工具名 / 参数摘要 / 影响条数 / 结果），参数摘要过 `scrub_log_line`（URL 里的 token、userinfo 一律 `***`），不含正文与凭据。日志落在与桌面端同一目录的日志文件里（`rustrss-mcp --http`/stdio 也会装同一套文件日志）。
- 传输层口径不变：无/错 token 仍 401，`/health` 仍不鉴权且不含订阅数据。

实测（2026-09-20）：

- `ss -ltn` 显示监听 `127.0.0.1:8817`（不是 `0.0.0.0`）；`/health` 无 token → 200；`/mcp` 无 token 或错 token → 401；对 token（查询串或 `Authorization: Bearer`）→ 200 且能取到真实订阅数据。
- **真实客户端**：Claude Code 以 HTTP + Authorization 头接入，正确报出 3 个订阅源；当被要求列未读条目时，它发现库里未读为 0 并**拒绝编造**，指出前提不成立。

实测（2026-09-23，T3 写工具；真二进制 `rustrss-mcp --http` + 真库 + 本地 HTTP 源，53 项断言全绿）：

- 读 token 调用 `set_read` / `refresh` / `fetch_fulltext` → 一律 `write_scope_required` 且库不变；写 token 下 `refresh(scope=all)` 抓到 2 条 → `sqlite3` 回读 `SELECT COUNT(*) FROM entries` = 2，重复刷新 `inserted=0`（源端 304，无重复条目）。
- `set_read(ids)` → `affected=2`，`sqlite3` 回读 `read` 列 = `1,1`、未读数 0，`db_stats.unread` = 0（与 `sqlite3` 的口径一致）；重复调用 `affected` 仍为 2（幂等）；`set_read(feed_id, read=false)` 撤销后未读数回到 2。
- `fetch_fulltext` → `fulltext_fetched=1`、`content_text` 607 字（越 500 字阈值），`get_article` 取到的正是抓回来的正文；二次调用 `already_fulltext=true` 且 `affected=0`（零网络）。错误码按类命中：`article_not_found` / `feed_not_found` / `folder_not_found` / `invalid_argument` / `write_scope_required`；审计行 grep 到 `mcp-write tool=set_read|set_starred|set_read_later|refresh|fetch_fulltext`，被拒的调用也有行（`error_code=write_scope_required`）。

实测（2026-09-23，T4 订阅管理；真二进制 `rustrss-mcp --http` + 真库 + 本地 HTTP 源，`/tmp/t4-live` 隔离 HOME）：

- `subscribe(首页)` → `affected=1`、`via=link_type`、落库地址是发现出来的 `/feed.xml`；同一首页再来一次 → `affected=0` + `already_subscribed=true` + 同一 id；`rsshub:///live/demo` → `RSSHUB://live/demo` → `https://rsshub.app/live/demo` 三种写法都归到同一个 `rsshub://live/demo`（后两次 `affected=0`）。
- 读 token 的 `list_feeds` 立刻看得到新订阅；`refresh(feed_ids)` 入库 2 条（`sqlite3` 回读 `COUNT(*) entries = 2`）；`update_feed{自定义名 + 归组}` → `sqlite3` 回读 `我的实机源|1|`。
- `folder_delete` dry_run → `feeds_affected=1`（组与订阅都不动）；confirm → 组消失、订阅仍在且 `folder_id` 变空。
- `unsubscribe` dry_run → `affected=2` 且 `sqlite3` 回读 feeds=1 / entries=2（未落库）；confirm → `affected=2`，回读 feeds=0 / entries=0（条目级联删除），`list_feeds` 里不再出现。
- 错误码实机命中：读 token 调 `subscribe` → `write_scope_required`；缺 `confirm` → `confirm_required`；源不存在 → `feed_not_found`。审计行 19 行（含 dry_run 与被拒），日志里凭据串 0 命中。

实测（2026-09-23，T4 标签；真二进制 `rustrss-mcp --http` + 真库 + 本地 HTTP 源，`mktemp -d` 隔离 HOME）：

- 读 token：`tools/list` 只有 8 个只读工具（有 `list_tags`、没有 `create_tag`）；`list_tags` 可用；调 `create_tag` / `assign_tags` → 工具级 `write_scope_required`（`isError=true`，库不变）。写 token：`create_tag` → `affected=1` + 归一化颜色 `#3e63dd`；重名报 `duplicate_tag_name`；`color: "blue"` 报 `invalid_argument`。
- `assign_tags(ids)` → `affected=2` / `detail.changed=2`，`sqlite3` 回读 `entry_tags=2`；`list_articles(tag_name=live)`（小写，大小写不敏感）→ 2 条且每条回出 `tags: ["Live"]`；`list_articles(tag_id + tag_name)` → `invalid_argument`；未知标签 → `tag_not_found`。
- **`last_used_at` 口径收口**：`sqlite3` 把哨兵值写成 1000 → `unassign_tags` 后回读仍为 `1000`（取消不推进）；再 `assign_tags` 后变为真实时间戳（打标才推进）。
- `delete_tag` 缺 `confirm` → `confirm_required`；`dry_run` → `affected=2` 且回读 `tags=1 / entry_tags=2 / entries=3`（未落库）；`confirm` → `affected=2`（与预览同源）、回读 `tags=0 / entry_tags=0 / entries=3`（标签消失、文章保留）。
- 审计行 16 行 `mcp-write tool=…`（含 `dry_run=true` 与 `write_scope_required` / `duplicate_tag_name` / `confirm_required` / `tag_not_found` 失败行）；日志里 `hunter2` / `SECRET-TOKEN` / 读写 token 串 0 命中，`token=***` 打码行保留。

<details>
<summary>客户端配置片段示例（应用内可直接复制）</summary>

```json
{
  "mcpServers": {
    "rustrss": {
      "type": "http",
      "url": "http://127.0.0.1:8817/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

```bash
# 或者一行命令（Claude Code）
claude mcp add --transport http rustrss http://127.0.0.1:8817/mcp --header "Authorization: Bearer <token>"
```
</details>

```bash
# 直接跑（stdio；库路径：$RUSTSS_DB → 第一个参数 → ~/.local/share/rustrss/rustrss.sqlite）
cargo build -p rustrss-mcp
./target/debug/rustrss-mcp /tmp/rustrss.sqlite

# 接到 Claude Code（一次性，不动全局配置）
cat > /tmp/rustrss-mcp.json <<'JSON'
{
  "mcpServers": {
    "rustrss": {
      "command": "/绝对路径/target/debug/rustrss-mcp",
      "args": [],
      "env": { "RUSTSS_DB": "/tmp/rustrss.sqlite" }
    }
  }
}
JSON
claude -p "用 rustrss 工具告诉我订阅源与未读数" \
  --mcp-config /tmp/rustrss-mcp.json --allowedTools "mcp__rustrss"
```

已用 Claude Code 做过真实客户端验证：它能正确报出源数/未读数、列出条目，并会主动提醒「抓取状态非 ok 的源数据可能不是最新」（`list_feeds` 里的 `status` 字段）——该字段就是为此保留的。

### 内置 AI（自带 key）

已适配四类端点：**OpenAI 兼容 / Anthropic 原生 / Gemini 原生 / Ollama**；任务：**摘要**（短/中/长）、**翻译**。

```bash
# 只看将要发送的请求（不打模型、零花费；凭据已打码）—— UI 上「发送前确认」拿的就是这个
RUSTSS_AI_PROVIDER=openai RUSTSS_AI_MODEL=gpt-x RUSTSS_AI_KEY=sk-xxx \
  cargo run -p rustrss-core --example ai_demo -- /tmp/rustrss.sqlite preview summarize

# 真正调用（结果落库缓存，重复打开不再请求）
RUSTSS_AI_PROVIDER=anthropic RUSTSS_AI_MODEL=claude-x RUSTSS_AI_KEY=... \
  cargo run -p rustrss-core --example ai_demo -- /tmp/rustrss.sqlite run translate
```

环境变量：`RUSTSS_AI_PROVIDER` / `RUSTSS_AI_MODEL` / `RUSTSS_AI_BASE_URL` / `RUSTSS_AI_KEY`（缺省回退 `ANTHROPIC_API_KEY`、`OPENAI_API_KEY`、`GEMINI_API_KEY`）/ `RUSTSS_AI_TARGET` / `RUSTSS_AI_ENTRY`。

三条约定（均有测试钉住）：

1. **key 只进请求头**；错误信息、请求预览、日志三处统一打码。注意 Gemini 原生协议把 key 放在 URL 查询串里——曾经是个真实泄露点，被 `preview_hides_credentials_for_every_provider` 拓出来。
2. **结果按「文章 + 任务 + 参数 + 模型 + prompt 版本」缓存**（`ai_cache` 表）：重复打开不重复请求；改 prompt 后旧缓存自动失效。
3. **超长正文截断并显式标注**（当前上限 12000 字符），不静默失败；分块/映射-归并留待后续。

尚未做：**「发送前确认要发什么」的交互**（`AiClient::preview()` 已具备，尚未接到界面上）。

### AI 在界面里怎么用

「设置 → AI」：选服务商（Ollama / OpenAI 兼容 / Anthropic / Gemini）、填模型与端点（留空用默认）、填 API key。

- **API key 存操作系统凭据库**（`keyring`：Linux 走 Secret Service、macOS 走 Keychain、Windows 走凭据管理器），**不进数据库**——数据库会被导出、同步、复制给别人，key 一旦进去就会跟着跑。
- 每个服务商一个凭据条目，切换服务商不会互相覆盖；界面只显示「已设置/未设置」，并如实告知来源（凭据库 / 环境变量）。
- 凭据库不可用时返回明确错误（提示需要 Secret Service / KWallet），**不静默降级成明文文件**；临时可用环境变量 `RUSTSS_AI_KEY` 代替。
- 凭据库读写带重试（每次重试都会新开一个 DH 会话）：ksecretd（kwallet6 ≤ 6.24.0）在 DH 共享密钥高位为零时不按 1024 位补零再 HKDF（[KDE #514194](https://bugs.kde.org/show_bug.cgi?id=514194)，上游 kwallet 6.25.0 修复），会让一次读/写偶发失败成 `Crypto error: Unpad Error`（本机实测 400 次读里 4 次，每次失败的下一次都成功）；重试即换会话，可缓解但不能保证成功。凭据库读取/写入与重试均在数据库锁外；Ollama 创建客户端不依赖凭据库。系统根治仍需带 KDE #514194 修复的发行版包（6.25.0+ 或回移补丁）。2026-09-23 本机仍为 6.24.0-0ubuntu1，本地 APT 候选相同，刷新索引受 sudo 交互认证阻挡，尚未升级。
- 启动时读凭据库失败不再中断界面初始化：设置页显示「未设置 + 失败原因」，其余功能照常可用；真正要用 key 的 AI 请求仍会明确报错。
- 「测试连接」会真的发一个最小请求——它比「检查 key 是否存在」有意义得多：同时验证了凭据、模型名与端点三件事。
- 正文里点「AI 摘要 / AI 翻译」→ 结果面板会标出**来自缓存还是本次新请求**、以及正文是否因超长被截断；旁边有「重新生成」。
- AI 输出 token 上限可调（设置 → AI → 输出上限，默认 4096、范围 256–32768）：推理模型的思考链也在同一预算内，上限太小时正文会一个字没产出（finish_reason=length + 空 content），错误提示会针对性指出该调大上限或换非推理模型。思考强度也可调（设置 → AI → 思考强度：跟随默认/最低/低/中/高，仅 OpenAI 兼容接口随请求发送 reasoning_effort；摘要/翻译任务调低可显著提速省 token，deepseek-r1 固定思考不受此参数影响）。

本机实测：凭据库探针（`cargo run -p rustrss-desktop --example keyring_probe`）在本机 KDE 下完成写入/读回/清理完整往返——**API key 的存储方案不建立在「应该能行」的假设上**。

### 桌面界面（三栏）

```bash
cargo run -p rustrss-desktop                              # 默认库：~/.local/share/rustrss/rustrss.sqlite
RUSTSS_DB=/tmp/demo.sqlite cargo run -p rustrss-desktop   # 指定库
```

- [x] 三栏：智能视图（全部未读 / 星标 / 全部）+ 订阅源（未读数、抓取失败红点）｜文章列表｜正文
- [x] 列表渐进加载：滚到底自动续下一批（每批 200 条，keyset 游标分页；游标取上一批末行的 `(sortkey, id)`，由后端直出），未读 / 全部 / 星标 / 稍后读 / 单源视图都支持，加载完全部匹配条目即停（无重复、无跳条）；续页只 append 新行、不重建已有 DOM。搜索仍是一次性 200 条（本版不含分页）。**后台刷新保持滚动位置**：已加载多页（`length > 200`，与 `exhausted` 无关——小库/星标这类已耗尽的多页视图同样保持）时只把比列表首行新的条目 prepend 到头部，并按 `scrollHeight` 增量补偿 `scrollTop`（在顶部 `scrollTop===0` 时不补偿，新条目立即可见）——**只在「最新在前」档**，其余排序档走整列重建（见下一条列表排序）；会话内读过的条目（`Enter` / `u` / 全部已读）不回插。单页、首屏、续页在飞或上批失败仍走整列重建。**列表头给进度而不是只报已加载**：「已加载 M / 共 N」，`N` 按当前视图**有效筛选**取索引计数——未读筛选（未读视图，或开着「隐藏已读」）下 `N` 就是该 scope 的未读数，直接取侧栏已加载的计数（零额外查询）；「全部视图 + 单源 / 单标签」才走 `list_scope_total`（`entry_count_for_feed` / `entry_count_for_folder` / `tag_entry_count`，均 `INDEXED BY` 钉覆盖索引 + EXPLAIN 断言 + 变异校验）；搜索视图不查 FTS 总数、只显示已加载，取数失败降级成「已加载 M 篇」并记一行日志（不弹错）。`M` 与 `N` 同口径（未读筛选下灰显的已读行不计入 `M`），因此 `M ≤ N` 恒成立。**列表尾部行三态**：有下一页且无请求在飞时是可点的「加载更多（已加载 M / 共 N）」并同时保留滚动自动续批；请求在飞时显示「加载中…」且禁用；续页失败后变「加载失败，点此重试」——手动点击才清失败态重试，自动触发在失败态仍被拦（不制造重试风暴），到末尾（`exhausted`）则改显「已到末尾（共 N 篇）」终止态而不是留白（见验证清单第 22.3 节）
- [x] 列表排序与过滤（列表头右侧图标按钮 → 菜单）：三档 —— 最新在前（默认）/ 最早在前 / 未读优先（未读组内仍最新在前，即 `read ASC, sortkey DESC`），加一个「隐藏已读」开关。两项都是**全局设置**（`settings` 表的 `list.sort` / `list.hide_read`），**store 在查询时直接读**——界面与 MCP 共用同一条读取路径，前端不往查询参数里再塞一份副本（避免两个事实源），非法值回默认档；切换立即重查重渲（reset 语义、分页状态重置），菜单里当前档与开关都打勾，跨视图切换与重启保留。「隐藏已读」豁免**星标 / 稍后读**视图（读完的星标还得能找回），搜索同样过滤（无搜索豁免）；开启后点行标读只灰显留行（不立即删行，防高亮/操作目标脱钩），行在下次列表重建时离开。keyset 游标按档适配：最早在前方向反转（`sortkey ASC, id ASC`，反扫同一个表达式索引）、未读优先用复合键 `(read, sortkey, id)`（游标多带一位 `read`，缺了就按首页处理而不是静默翻错页），三档续页都不重不漏（`append … dup=0` 自证）。迁移 **v11** 新增表达式索引 `(read, COALESCE(published_at, fetched_at) DESC, id)`，未读优先档全程 `INDEXED BY` 钉住它（本程序从不 ANALYZE，不钉的话 feed 视图会退化成等值索引 + 临时排序）；EXPLAIN 断言与线上 SQL 同源且经变异校验（`unread_first_cursor_plans_use_v11_composite_index` / `oldest_pages_reverse_scan_the_sortkey_index`）。**后台刷新只在「最新在前」档 prepend**：最早在前的新条目属于列表尾部、未读优先的新未读属于未读组头部，插到 DOM 头部会让 DOM 顺序与后端顺序不一致；这两档退化为静默 reset（列表与分页状态重建、正文与阅读位置不动，**只保留 `scrollTop` 像素偏移、不做锚点补偿**——已文档化的取舍：顺序正确优先）。i18n 双语；headless 验证与遗留人工项见验证清单第 15 节；另有**列表头常驻「只看未读」开关**（与菜单里的「隐藏已读」同一设置、同源于 `list.hide_read`，`aria-pressed` + 高亮态、默认关、跨重启保留）+ 快捷键 `U`（大写；输入区/覆盖层内不触发）——入口提到明处（菜单里那一项用户实测找不到），证据见验证清单第 22.2 节
- [x] 键盘导航：`j`/`k` 上下 · `Enter` 打开 · `u` 未读切换 · `s` 星标 · `l` 稍后读 · `t` 标签 · `U` 只看未读开关 · `r` 刷新 · `/` 搜索 · `Esc` 清除 · `g`/`G` 首尾
- [x] 分组文章视图：点击分组名称查看组内文章，箭头独立折叠/展开；支持三档排序、只看未读、200 条分页、总数与当前分组批量标记。空分组返回空列表；移动源刷新当前分组，删除当前分组返回全部视图。
- [x] 列表异步响应按请求版本落地：切换视图/搜索/重置后，旧首页、续页与总数响应不能覆盖当前状态；快速切换文章时只展示最后请求的文章。标签变更使总数缓存失效；头尾计数同值零写入，分页按钮禁用态与请求生命周期一致。
- [x] 设置面板：「`j`/`k` 浏览时标记已读」开关（默认开）+ 当前视图全部已读 / 全部未读（按当前有效筛选作用于全部匹配条目，含未加载页；搜索也不限于已加载的 200 条，空搜索不操作；星标/稍后读仍豁免隐藏已读）+ 界面语言 + 主题三态（跟随系统/浅色/深色，选择持久化；启动时窗口以隐藏方式创建、主题就绪后显示，无主题闪变）+ 关闭按钮行为（退出 / 最小化到托盘；托盘不可用时始终退出）+ 自动刷新（间隔 关/15/30/60/120/360 分钟、启动时刷新开关、新文章通知开关，默认关）+ 字体（见下）+ AI + MCP
- [x] 字体可配置（设置 → 通用 → 字体分区）：界面 / 正文 / 等宽**三类字体族互相独立**（未自定义时「跟随主题」：各自使用预设字体栈），正文字号 13–28px、正文字高 1.3–2.2 可调；全部走 CSS 变量（`--font-ui` / `--font-read` / `--font-mono` / `--font-read-size` / `--font-read-line`），**保存即生效、无需重启，跨会话保持**。字体列表由 `list_font_families` 枚举系统字体（Linux `fc-list`，3s 超时、spawn 失败/超时/非零退出均降级为空表；Windows / macOS 本版返回空表），设置页打开时预取一次并缓存（`fc-list` 实测 18ms）；下拉复用自绘菜单（首项「跟随主题」= 清除该字体覆盖，恢复预设栈），含空格 / CJK 的族名写 CSS 变量时统一加引号并转义。滑块 **`input` 只改 CSS 变量做实时预览、`change`（松手）才写库**（拖动期间零 IPC、零 DB 写入），字号 clamp 13–28 / 行高 clamp 1.3–2.2 在 Rust 侧再夹一次（库里不存越界值）；设置弹层遮住了正文区，因此字体分区里带一块**与正文同源 CSS 变量的实时预览块**（拖动/切换即时可见）。字体枚举失败时三个下拉仍可用（只剩「跟随主题」）并把原因写在 tooltip 上
- [x] 自绘标题栏：无系统装饰，顶栏可拖拽 / 双击最大化，右上角最小化 / 最大化 / 关闭三键（代码完成 + 编译与真实 Chromium 加载验证；**点击行为与拖拽需桌面会话人工验证**，见验证清单）
- [x] MCP：stdio 与 HTTP 双传输，HTTP 仅回环 + token；应用内一键生成并复制客户端配置
- [x] 全文搜索（接 FTS5，中文可用）、刷新全部、单源双击重试、添加订阅（首页 URL 自动发现 feed）、浏览器打开、复制链接
- [x] 稍后读：阅读器按钮 / 列表条目 ⚑ 标记 / `l` 快捷键，与已读、星标独立；侧栏「稍后读」智能视图
- [x] RSSHub 实例：设置里可配自建/镜像地址。`rsshub://path`（含三斜杠、大写）与 `https://rsshub.app/path` 是**同一订阅身份**，库里统一只存 `rsshub://path`（抽象地址，不绑定任何实例）——抓取时才由唯一出口 `Store::feed_endpoint` 按当前实例解析（`resolve_fetch_url`），所以**换实例零迁移**、下一次刷新即生效（库内 url 不变、OPML 导出天然可移植）；已实例化到自建实例的历史行识别不了、原样直抓（不劣化，与拆分前一致）。添加订阅输入框直接支持 `rsshub://path`（发现阶段短路不联网，首次抓取经实例解析）。设置页「归一化 RSSHub 地址」是**一次性地址整理**（存量 `rsshub.app` 行 → `rsshub://path`；预览条数与实际改写条数共用同一判据，已是 scheme 的行幂等不动），换实例不需要点它
- [x] 正文安全渲染：白名单清洗 + 相对地址图片/链接解析（详见下）；代码块语法高亮（vendor highlight.js，`language-*` class 优先 + 自动检测，深浅双主题 token 配色；超过 16KB 的超大代码块跳过 auto-detect 以保证大文章的打开速度，显式 `language-diff` 放宽到 64KB；桌面像素效果需人工核验）
- [x] 渲染层最小 CSP 纵深（`app.security.csp`，编译期嵌入）：`default-src 'self'` + 逐类最小放行——脚本只允许自身（Tauri 构建时把 `ui/` 内联脚本与 JS 资产哈希注入 `script-src`，故 `index.html` 的内联诊断脚本照常执行）、样式放行 `'unsafe-inline'`（自绘菜单/字体实时预览靠内联样式）、图片额外放行 `data:` 与 `http(s):`（feed 正文图 / 图标）、连接只放行 Tauri IPC（`ipc:` + Windows 的 `http://ipc.localhost`）、字体只放行自身与 `data:`。`withGlobalTauri` 因「UI 无构建链」保持 `true`（无打包器就 import 不了 `@tauri-apps/api`，原生 JS 只能经全局 `window.__TAURI__` 调命令）——所以 CSP 是 sanitize 之外的**纵深**，不是它的替代；`ui/app.js` 常驻 `securitypolicyviolation` 探针，每次违规经 `ui_log` 打一行（含被拒指令与来源），主流程回归后核对日志为「零违规」即通过（见审计修复批次一 T5）
- [x] 补丁/diff 渲染：邮件列表源（lkml 等）把补丁拆成一连串段落、没有代码块——sanitize 后做一次 diff 区域归一（连续 +/-/@@/头行段落合并成单个 `pre>code.language-diff`，保守门槛防误吞普通段落），再走 hljs：绿增红删整行底色 + 左缘强调条 + hunk 头蓝底（深浅双主题，修复过「加行配红」的错映射）
- [x] 摘要型条目一键获取全文：正文缺失或明显偏短（阈值 500 字）、或带源端摘要标记（lkml.org 的 `某人 writes: (Summary)` 长摘要超过阈值也命中）、且未抓过又有原文地址的条目，在阅读器显示「获取全文」按钮；点击后转 loading（防重入）→ 抓原文页 → readability 提取正文 → 写回库并用返回的行重渲染；失败（非 HTML / 超时 / 超 2MB / 无正文 / **反爬质询页**）在状态栏报错，**原摘要原样保留**，可直接再点重试。反爬质询页（Anubis 等，需浏览器过 JS 验证）会被识别并拒绝写回，提示用户用「浏览器打开」；写回会打上 `fulltext_fetched` 标记：同一篇第二次打开零网络，之后 feed 刷新也不会把已抓正文覆盖回摘要（core 侧见 `crates/rustrss-core/src/fulltext.rs` 与 `fetch_fulltext` command，前端见 `ui/app.js` 的 `fetchFulltext`；手动清单第 7 节）
- [x] OPML 导入 / 导出（嵌套文件夹压平成 `父/子`；按 `xmlUrl` 去重；导入后自动只抓新增的那批源，`feeds_added=0` 的重复导入不抓）
- [x] 一键备份 / 恢复：设置 → 数据里「备份数据库…」选目录 → rusqlite backup API **在线快照**（导出期间库可继续读写，不需要先 checkpoint），产物 `RustRss-backup-<时间戳>.sqlite`（UTC）是独立干净的库文件（普通 journal 模式，拷到另一台机器直接可用）；「从备份恢复…」选文件 → **只读**校验（非 SQLite 文件 / 空库 / `user_version` 超前一律拒绝，且此步不动现库）→ 覆盖确认 → 暂存 `pending-restore.sqlite`，**重启后在任何连接（Store / MCP）打开之前替换**（退出时替换不可行：MCP 第二条连接还活着、Windows 不能 rename 打开中的文件）。替换前先把现库另存为 `.bak-<时间戳>` 保底回滚（只留最近 1 份）；stale `-wal`/`-shm` **严格先于** `rename(pending→db)` 删除，且**无 pending 时绝不触碰边车**——正常启动时那是未 checkpoint 的已提交事务，删掉就是丢数据（core 侧 `crates/rustrss-core/src/store/backup.rs`，测试 `crates/rustrss-core/tests/backup.rs`；手动清单第 8 节）
- [x] 自动刷新：定时刷新（默认 30 分钟，可关；调度器每分钟读一次设置，改完无需重启）+ 刷新并发档位 3/6/12/24 路（默认 6：弱网/限流敏感选 3，数百订阅选 12，本地 RSSHub 镜像选 24；手动与后台同口径）+ 启动后 10 秒首刷（默认开，可关）+ **每源独立间隔**：侧栏右键某源 →「刷新间隔」子菜单（父项直出当前档位，如「刷新间隔 · 跟随全局（每 30 分钟）」；悬停/点击展开的 6 档：跟随全局 / 15/30/60/120/360 分钟，当前档位打勾，跟随项的文案里回显当前全局档，如「跟随全局（关闭）」），点选即生效并持久化——列在 `feeds.refresh_interval_minutes`（迁移 v8，NULL = 跟随全局），有覆盖的源在侧栏 tooltip 里写明「独立刷新间隔：…」。**覆盖优先于全局开关**：全局档设成关闭时，已单独设置了间隔的源仍按各自间隔刷新（设置页的间隔 Hint 已写明这条例外；想彻底停自动刷新就关全局并让该源「跟随全局」）。到期基准是库内的 `last_fetched_at`——手动刷新 / OPML 导入也会把这个源的自动刷新计时归零（刚抓过的源不重复自动刷）。后台刷新只发 `refresh:start` / `refresh:done` 事件：状态栏提示「后台刷新中…」，完成后静默更新侧栏与列表——多页已加载时只 prepend 新条目并保持列表滚动位置（见本节渐进加载一条），单页/首屏才整列重建；两种情况都**不重渲染正文**，正在读的长文与正文滚动位置保持原位；手动刷新仍是同步等待、不发事件，两条路径共享单 flight 不会叠加（见验证清单第 5 / 11 节）
- [x] 新文章通知：后台定时/启动刷新在**抢到单 flight 之后**采样未读数，前后差值 > 0 且设置里「新文章通知」（默认关）打开时弹**一条聚合**系统通知「N 篇新文章」（`tauri-plugin-notification`，2.x）。手动刷新路径不通知（用户就在界面前）；通知文案是 Rust 侧双语常量、按 `ui.locale` 选，与托盘菜单同一模式（不经 `ui/i18n.js`）。点击行为交给系统/桌面环境：Windows/macOS 点击激活应用，Linux 依 DE 而定（最差仅展示）——桌面端插件不提供点击回调，已作为平台差异记录
- [x] 侧栏文件夹分组：可折叠组头 + 组内未读合计；右键新建 / 重命名 / 删除 / 移动订阅到文件夹（「移动到」为子菜单：父项直出当前分组，子菜单列出全部分组 + 未分组并打勾；删除组不删订阅；折叠状态跨会话保持；拖拽归组待后续）
- [x] 订阅源编辑对话框（侧栏右键某源 →「编辑」）：标题（placeholder 是源站名，**留空 = 回到源站名**）、文件夹、刷新间隔三个可改字段收拢在一处，订阅地址只读展示可选中复制（本版不支持改 URL）。字段先在本地暂存，点「保存」才一次落库（取消 / Esc / 点遮罩**零副作用**），且只写用户真改过的字段（未改的字段一个都不动）。自定义标题存 `feeds.custom_title`（迁移 v10，NULL = 跟随源站名），显示层统一走 `COALESCE(custom_title, title)`——侧栏、列表行、阅读区元信息、列表标题、OPML 导出与 MCP 输出同一口径；刷新（含启动首刷）只更新源站名 `feeds.title`，**不覆盖自定义名**（源站改名后自定义名照旧，清空输入框才回退到最新的源站名）。保存后侧栏（含按新显示名重排）与列表行 / 阅读区元信息 / 状态栏同步更新，且**不重建列表与正文**（滚动位置、选中态保持原位）。侧栏右键菜单原有的「移动到」「刷新间隔」快捷项保留（改为子菜单：父项显示当前分组/当前档位，子菜单打勾，见本节自动刷新的「每源独立间隔」一条），两条路径共用同一套归一化与落库（`assign_folder` + 刷新间隔白名单）
- [x] 系统托盘：显示/隐藏窗口 + 退出（菜单文案跟随界面语言设置，`auto` 时固定中文；托盘文案在 Rust 侧维护、不经 `ui/i18n.js` 的 key-set 自测——这是已知例外；托盘不可用时自动降级为无托盘并日志说明，不崩溃。注意：托盘在真实桌面会话下的行为需人工验证，headless 环境仅验证了代码路径与降级逻辑）
- [x] 应用图标：Ferris（Rust 蟹吉祥物）+ RSS 电波标记（源文件 src-tauri/icons/src/ 含几何生成脚本）；Wayland 下 cargo run 的任务栏图标需装 desktop 文件（packaging/rustrss-desktop.desktop 模板）
- [x] 托盘未读角标：未读 > 0 时在托盘图标右上角画红点（由现有窗口图标派生，不预置图片资源）+ tooltip「RustRss · N 篇未读」（**保留品牌名**），未读清零后恢复原图标。角标由后台刷新与改变未读数的命令（`set_read` / 全部已读 / 全部未读 / 删除订阅）同步；手动刷新路径不动角标（下一轮后台刷新自愈）。托盘不可用时静默 no-op，不刷错误日志（见验证清单第 9 节）
- [x] 单实例锁：用**默认库**时第二次启动会在毫秒级被已有实例接管（唤出主窗口后新进程自退）——避免两进程抢同一个 MCP 端口、双写同一个 SQLite。注意：新进程在被接管前有短暂启动期（历史行为是先开库再被退出，现已把开库/拉起 MCP 全部移到单实例判定之后，新进程不再触碰库与端口）；`RUSTSS_DB`/参数把库指到别处时**不注册锁**，多开诊断副本不受影响
- [x] 日志（诊断材料）：每次启动在数据目录 `logs/` 下新建一个日志文件（`rustrss-YYYYMMDD-HHMMSS.log`），行内含本地时间戳（毫秒）/级别/来源模块，Rust 侧关键事件与界面诊断行（`[ui]`，target=ui）进同一份文件；panic 写一条 ERROR（payload + 位置）且保留 stderr 现场；每次启动清理保留最近 20 个文件且总量 ≤50MB（两条上限独立生效、超限从最旧删）；设置 → 关于有「日志级别」（`log.level`，info/debug，改完即时生效、跨重启保持，debug 只收录本应用 `rustrss*`/`ui` 的记录）与「打开日志目录」（系统文件管理器打开，路径作独立参数不经 shell，失败给可读错误不崩溃）；初始化失败降级为不写日志、不阻断启动。详见下节
- [x] Linux 打包：产出 `.deb`（**8.1MB，不打包 WebKit**，依赖声明 `libwebkit2gtk-4.1-0, libgtk-3-0, libayatana-appindicator3-1`）
- [x] i18n：zh-CN / en（317 个 key；启动时比对两份字典的 key 集合并把结果打到 stdout，缺 key 数为 0 可机械核对）
- [ ] 便携模式（`portable.txt`）
- [ ] 超长列表的 DOM 上限：渐进加载会把已加载的行全部留在 DOM 里（8k 库全扫后约 8k 行，实测滚动与 j/k 均无卡顿），如需更激进的取舍可再做虚拟滚动
- [ ] 发布构建开 `strip`（当前未开，`Installed-Size` 25MB 偏大）、rpm/Windows/macOS 打包

安装（deb）：

```bash
sudo apt install ./target/release/bundle/deb/RustRss_0.0.0_amd64.deb
```

打包命令（需要 Node，CLI 经由 npx 调用，不装全局）：

```bash
npx -y @tauri-apps/cli@latest build --bundles deb
```

三条刻意的设计选择：

1. **已读判定可配置，可批量设为未读**：默认 `j`/`k` 浏览时就标记已读（NetNewsWire / Reeder 这类键盘阅读器的主流做法：扫一遍即已读）；设置里可关掉，改成「只有 `Enter`、鼠标点击或 `u` 才改变状态」。「当前视图全部未读」是批量设状态，不恢复之前的状态；需要处理已读文章时请进入全部/单源视图并关闭「只看未读」。无论开关如何，**切换视图与刷新列表都不会改变已读状态**。
2. **正文经白名单清洗后才进 DOM**（详见下）。
3. **设置存在数据库里**（`settings` 键值表，与订阅同一份数据）——默认值只在 Rust 侧定义一处，界面只负责显示与切换，避免两边各写一份而漂移。

界面行为的七个细节（都是被实际使用或诊断抓出来的）：

- **状态栏在右下角**：刷新进度/错误提示显示在阅读区底部的右对齐状态行，常驻固定高度（空时只剩一条分隔线）——原先放在工具栏右侧会被长文案（如「刷新完成: 成功 89 | 未修改 0 | 新增 7113 | 失败 0」）截断并紧贴设置按钮；移下来后工具栏按钮不再被挤，状态文案也有 72ch 的展示空间。

- **后台刷新不抢阅读焦点，也不打断列表滚动位置**：定时/启动刷新（Rust 侧 `scheduler.rs`）在动手前先 `emit("refresh:start")`、收工后 `emit("refresh:done")`；界面收到 start 只在状态栏写一行「后台刷新中…」。收到 done 分两支：**已加载多页 + 当前是「最新在前」档**（`state.entries.length > 200`，判据不含 `exhausted`——小库/星标这类已耗尽的多页视图同样要保位置；续页在飞或上批失败时退回重建，避免与 `loadMore` 的游标/append 竞态）只把「比列表首行新」的条目 `insertBefore` 到头部，并按 `scrollHeight` 增量补偿 `scrollTop`（在顶部 `scrollTop===0` 不补偿，新条目立即可见）；**其余排序档不 prepend**（最早在前的新条目属于列表尾部、未读优先的属于未读组头部，插到头部会与后端顺序不一致），退化为静默 reset（整列重建 + 分页状态重置，**只保留 `scrollTop` 像素偏移、不做锚点补偿**），**单页/首屏**沿用整列重建。所有这些路径都**不重渲染正文**、也不重建已加载的列表行（重建会把列表和正文的滚动位置一起打回顶部，也是打开文章时的 CPU 尖峰来源），续页游标/`exhausted`/尾部哨兵一律不动。会话内读过的条目（`Enter` 打开、`u` 切换、全部已读）记在 `readSessionIds` 里、刷新时不回插——未读视图里读过的那行已从列表删掉，而刷新的查询可能先于 `set_read` 提交。手动刷新是同步等待且不发事件，所以不会出现「后台提示 + 手动提示」两条文案叠在一起。

- **侧栏渲染走单一 keyed reconcile 路径**：所有触发方（阅读后的计数刷新、视图切换、文件夹管理、语言切换、搜索态）都只调 `renderSidebar()` 一个入口，内部按 key 复用已有行、只更新变化字段、按期望顺序归位、清掉消失的行。刻意不做「全量重建 + 窄版补丁」两套路径——两条路径迟早漂移出陈旧计数的安静 bug。#feeds 内的行也不挂逐行监听器，点击/双击/右键由容器统一代理，事件时刻从 state 现查数据对象，行复用拿不到过期闭包。
- **选中项会自动滚入可视区**（`scrollIntoView({block:'nearest'})`）。先前选中项变化走的是「全量重建列表」且从不滚动，于是按 `j` 往下走时高亮会跑到列表可视范围之外。现在选中项变化只改行高亮，不重建 DOM；未读视图里读完一篇也只移除那一行 DOM（200 行全量重建是打开文章时的 CPU 尖峰，会把并发的后端命令拖慢一个量级），仅列表被读空**且没有下一批**时才重建出「暂无未读」占位（游标之后还有未读就续一批接着读）。
- **列表滚到底自动续页，且只 append 不重建**：列表尾部放一个空哨兵 `<li class="load-sentinel">`，`IntersectionObserver`（`root` = 列表容器，`rootMargin: 600px`）在它接近可视区时拉下一批（每批 200）。每批渲染完都换一个新哨兵节点再 `observe`：观察者只在「相交状态变化」时回调，哨兵一直留在可视区内（这批行填不满一屏、或列表被读空变短）时不会再有通知、续页就卡住了，而重新 `observe` 必然先给一次初始通知，正好把「还没填满就接着拉」接上。游标用上一批**已取到**的末行的 `(sortkey, id)`——不是「当前列表最后一行」：未读视图里读完一篇会把它从列表移除，若拿剩下的末行当游标，读空一整批之后就再也取不到后面的未读条目（游标是结果流里的位置，不随某行被移出列表而后退）。返回不足一批即到末尾，此时撤掉哨兵、断开观察者；请求在飞时防重入，避免哨兵连续触发把同一批行 append 两遍（每次续页会把重复条数打进日志：`append rows=… dup=0`）。
- 列表列的高度用 `grid-template-rows: minmax(0, 1fr)` 显式约束，否则行的 auto 高度会被内容撑开、整列能滚过窗口底部。
- **普通 `<script>` 的顶层函数声明会变成 window 属性**：`i18n.js` 导出 `applyStaticI18n` 这类名字后，`app.js` 再写同名的顶层 `const` 会在 WebKit 下报 `Can't create duplicate variable that shadows a global property`，而且是**解析期**错误——整份脚本一句都不执行，界面表现为「什么都不发生」。两个文件现在都包在 IIFE 里，只暴露 `window.I18N`。
  页内保留了一个错误上报探针（捕获脚加载失败与未捕获异常，上报到日志）——装它之前，这类失败是“无信息”的；靠它才拿到上面那行报错。

关于第 2 条：页面启动时会跑一次自检（构造带 `<script>`/`onerror`/`javascript:` 的脏 HTML，验证清洗结果与相对地址解析），结果上报到 stdout：日志里看到 `sanitizer selftest ok` 即通过。**这个自检不是形式，它已经抓出两个真 bug**：清洗时误删了 `body` 自身（启动直接失败），以及把相对地址当非法协议删除（导致 feed 里的图片全不显示）。

**添加订阅**：输入框既收站点首页也收 feed 地址，两者是同一条路径——先发现、再订阅。发现只发一次 GET：返回内容本身能按 feed 解析时，输入地址（重定向后）就是订阅地址；是 HTML 则扫 `<head>` 里的 `<link rel="alternate">`，`application/rss+xml` / `atom+xml` / `feed+json` 三个标准 `type` 优先，`type` 缺失或写错时按 href 后缀兜底，相对 href 按页面地址解析成绝对地址。拿到 feed 地址后走原有订阅 + 首次抓取路径；找不到候选则报错并保留原始原因（不做 `/feed`、`/rss.xml` 之类的路径猜测），输入内容与按钮都留在原地，直接重试即可。逻辑在 `rustrss-core/src/discover.rs`，桌面侧只包一层 `discover_feed` command。

### 日志（诊断材料）

每次启动在数据目录的 `logs/` 下新建一个日志文件——Linux `~/.local/share/rustrss/logs/`、macOS `~/Library/Application Support/rustrss/logs/`、Windows `%APPDATA%\rustrss\logs\`（`XDG_DATA_HOME` 生效时跟随它）。注意日志目录只跟随数据目录，**不跟着 `RUSTSS_DB` 走**（诊断副本的库可以指到别处，日志仍在同一个数据目录）。

- **文件名**：`rustrss-YYYYMMDD-HHMMSS.log`（本地时间；同一秒内第二次启动追加 `-N` 后缀）。
- **行格式**：`2026-09-22T23:45:01.123 INFO  rustrss_core::fetch: …`（本地时间带毫秒、级别、来源模块）。界面诊断行进同一份文件，target 为 `ui`、正文保留 `[ui]` 前缀——**它们不在 stdout 里**（从终端运行时随同一行镜像到 stderr，见下条），看界面在干什么要看日志文件。
- **终端镜像**：从终端运行（stdout 是终端）时，日志行在写文件的同时**镜像一份到 stderr**——`cargo run -p rustrss-desktop` 就能直接看到启动行与 `[ui]` 诊断行，不必先去找日志文件；重定向到文件或由桌面启动器拉起时 stdout 不是终端，默认**不**镜像，桌面场景没有额外输出。`RUSTSS_LOG_STDOUT=1` 强制开启（无头/脚本场景）、`=0` 强制关闭；未设置、空串或其它非法值按「stdout 是否终端」判，**启动时只判一次**。镜像行与文件行**同源同格式**（同一份字符串，逐字节一致，含时间戳毫秒），级别与 target 过滤口径也相同——镜像不是第二个数据源，文件日志的内容/顺序/保留策略都不受影响；镜像写失败（stderr 已关）静默忽略，不影响文件写入与应用。
- **内容**：抓取/入库/刷新/调度/迁移/托盘降级等 Rust 侧关键事件 + 界面诊断行；panic 另写一条 `ERROR`（payload + 位置），stderr 现场照旧。落盘前对凭据形态打码（URL 里的 userinfo、`key=`/`token=` 查询串），AI 错误文本在 core 侧已打码。
- **级别**：设置 → 关于 →「日志级别」（存库键 `log.level`，默认 `info`）。`debug` 记录更细的过程（刷新批次、逐源失败、HTTP 请求耗时），**改完即时生效、无需重启**，跨重启保持；debug 级只收录本应用（`rustrss*` 与 `ui`）的记录，依赖库（h2/hyper/reqwest…）的 debug 不写文件。
- **保留策略**：每次启动清理一次，**保留最近 20 个文件且总量 ≤ 50MB**（两条上限独立生效，超限从最旧的开始删）；清理结果写进当次日志（`日志保留清理: 删除 N 个（剩 N 个 / N 字节）`）。
- **反馈问题**：设置 → 关于 →「打开日志目录」用系统文件管理器打开该目录，把**最新的那个文件**附在 issue 里。启动器起不来（如裸容器没装 `xdg-open`）会在状态栏给出可读错误（含原因）且应用不崩溃。**已知取舍**：启动器在、但它自己打不开（环境里没有文件管理器）时无法检测——`spawn` 成功不等于目录真的打开（实测 Xvfb 下 `xdg-open` 静默 `exit 0`），此时只显示「已**请**系统文件管理器打开」，文案刻意不说「已打开」；与既有「浏览器打开」同一口径。
- **初始化失败不阻断启动**：日志目录不可写（权限 / 磁盘满）时降级为「本次不写日志文件」，只在 stderr 打印一行原因，其余功能照常。

## 发布步骤

版本号在**两处**同步：根 `Cargo.toml` 的 `[workspace.package].version` 与 `src-tauri/tauri.conf.json` 的 `version`（当前均为 `0.0.0`，首发前一起 bump）。

1. bump 上述两处版本号；
2. 更新 `CHANGELOG.md`——**尚未创建**：首次发布前按 Added / Changed / Fixed 分组建起来；
3. 提交后打 tag 并推送：`git tag vX.Y.Z && git push origin vX.Y.Z` → 触发 `.github/workflows/release.yml`，产出 Linux deb（ubuntu-22.04 基线）/ Windows NSIS / macOS dmg（arm64，未签名，首次打开需右键→打开）并自动创建 GitHub Release（自动生成变更说明）；
4. **体积记录**：release workflow 的 `Record bundle size` 步骤把每个平台的产物体积写进该 job 的 summary；把三平台数字回填 `.chorus/specs/rss-reader/spec.md` 的「安装包体积报出实测值」条目；
5. **只量体积、不发版**：`gh workflow run release.yml`（`workflow_dispatch`；`release` job 限定 tag，因此不会创建 Release）——**不要为了拿数字先打 tag**。

CI（`.github/workflows/ci.yml`）在 push / PR 时必跑 `cargo test --workspace --locked`；clippy 目前是 report-only，另有 3 条既有告警待清零。

## 进度

### M1 · core 数据层（进行中）

- [x] 解析：RSS 0.x/1.0/2.0、Atom、JSON Feed → 统一领域模型（基于 feed-rs 2.4）
- [x] 条目身份判定：源 id/guid 优先；退化场景（既无 id 也无链接）改用内容指纹，保证跨次抓取稳定
- [x] HTML → 纯文本（去标签、剔除 script/style、实体解码、保留块级换行）
- [x] SQLite 存储：schema 迁移、按 `stable_id` 去重 upsert、已读/星标、未读计数、文件夹、抓取状态与缓存头
- [x] 全文检索：FTS5 + 中文预分词（拉丁词、中文 bigram 与单字候选；单字保留标题/正文 LIKE 语义校验）
- [x] 抓取：条件请求（ETag / Last-Modified）、有界并发、单源失败隔离、增量入库、单源响应体积上限 8 MiB（`MAX_FEED_BYTES`：超限在下载中即中止，该源本轮按失败上报——不写条目、不覆盖 ETag，错误文案含实际体积与上限；全文抓取另有 2 MiB 上限）

至此 `抓取 → 解析 → 入库 → 检索` 数据通路已闭合。

**两层保护**（真实源实测：3 个源第二次刷新时，2 个返回 304 零写入，1 个不支持 304 但被内容指纹判为「未变」，条目数不变）：
1. 条件请求能省则省；
2. 源站不配合时，靠内容指纹保证不重复入库。

两条不容退让的保证（均有测试）：
1. **重复刷新不产生重复条目**——`UNIQUE(feed_id, stable_id)` + 内容指纹；内容没变则不解写库。
2. **刷新不覆盖阅读状态**——`read` / `starred` 不在 upsert 的更新列里。

单元测试：`cargo test -p rustrss-core`（含 wiremock 本地服务，不依赖外网）

用真实源手工验证：

```bash
# 只看解析结果
curl -sL -A "RustRss/0.0" -o /tmp/feed.xml https://lwn.net/headlines/rss
cargo run -p rustrss-core --example parse_file -- /tmp/feed.xml

# 解析 + 入库 + 检索（同一文件跑两次，第二次应全部计为「未变」）
cargo run -p rustrss-core --example import_file -- /tmp/feed.xml /tmp/rustrss.sqlite "kernel"

# 真实网络：抓取 → 解析 → 入库 → 再刷新（观察 304 与去重）
cargo run -p rustrss-core --example refresh_real -- /tmp/rustrss.sqlite \
  https://sspai.com/feed https://www.ruanyifeng.com/blog/atom.xml https://lwn.net/headlines/rss
```

### M0 技术探针（已完成，结论见 PRD §10）

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

### CI/CD（GitHub Actions）

三个工作流（`.github/workflows/`）：

- `ci.yml`：push(master) / PR → `cargo test` + debug 构建 + clippy（report-only，既有警告待清零后再收紧门槛）
- `nightly.yml`：push(master) / 手动 → Linux deb（ubuntu-22.04 基线）+ Windows NSIS，挂到滚动 pre-release 标签 `nightly`（整删整传，始终对应当前 master；macOS 因私有仓库 10 倍计费不在 nightly）
- `release.yml`：推 `v*` 标签 → Linux deb + Windows NSIS + macOS dmg（arm64，未签名）+ GitHub Release（自动变更说明）；发版前先 bump `workspace.package` 与 `tauri.conf.json` 的版本号再打 tag

### 运行（三种会话各跑一遍）

```bash
cargo run -p rustrss-desktop                              # 按会话自动选择后端
GDK_BACKEND=wayland cargo run -p rustrss-desktop          # 强制 Wayland
GDK_BACKEND=x11 cargo run -p rustrss-desktop              # 强制 X11
env -u DISPLAY GDK_BACKEND=wayland cargo run -p rustrss-desktop   # 模拟无 XWayland
```

页面把诊断数据同时显示在界面上、并通过 `probe_log` 打到 stdout（前缀 `[probe]`），便于外部脚本采集。

### 订阅源限流等待

429/503 的有效 `Retry-After`（秒数或 HTTP 日期）会持久化为下一次允许请求的时间，桌面、启动/定时刷新及 MCP 刷新共用该期限。期限内手动刷新也不会发送请求，报告返回 `retry_deferred`，保留上次真实失败状态和缓存；期限结束后恢复正常刷新。429 缺少有效期限时默认等待60秒；503未给有效期限时维持原刷新间隔。等待不占用网络线程，不自动密集重试。

### 应用内代理

设置 → 订阅 → 网络代理提供环境代理、直连、自定义HTTP/HTTPS代理三档，作用于订阅、发现、全文、RSSHub探测与AI请求。MCP订阅工具读取相同配置；本地MCP控制/截图通道仍保持回环直连。保存后新请求使用新配置，正在执行的请求不被中断。自定义模式可填写逗号分隔的绕过列表；目前不接受含用户名/密码的代理地址，代理凭据不会写入配置数据库。

解析兼容：GBK/GB18030按XML声明解码；当整个文档实际为合法UTF-8且仍错误声明GBK/GB18030时，按UTF-8解析，避免重复解码乱码。此规则不猜测其它未知编码；GB18030四字节字符、BOM、单引号声明和正常Latin-1均有回归样本。

订阅排序：可在侧栏同一文件夹内（或未分组区内）拖动订阅行，放到目标行上半部/下半部表示前插/后插；右键菜单也提供组内上移/下移。顺序存入SQLite并在重启后保持，未手动排列的分组继续按名称排序，新订阅排在已排列项之后。跨文件夹移动仍使用现有编辑/分组菜单，拖拽不改变分组。

分组列表查询：schema v15在现有排序索引末尾加入订阅ID，筛除其它分组文章时不再逐条读取正文所在记录，排序和分页规则不变。首次升级会重建两个索引，有一次性磁盘IO成本。隔离10k文章库的分组200行查询，冷数据库页约528ms降至26ms、热缓存约88ms降至2.7ms；这些是core查询测量，不代表整窗延时。复现与范围见 `.chorus/specs/rss-reader/2026-09-23-theme-preview/scheduler-performance.md`。

键盘帮助：在非输入区按 `?` 打开快捷键一览，Esc或关闭按钮退出。智能视图与订阅行支持Tab聚焦、Enter/空格激活；j/k/g/G导航文章时焦点转入列表，随后Enter不会再次激活原侧栏。帮助弹窗阻止背景导航，输入框与设置页不误触发。实际X11按键、万篇库搜索与缓存阅读证据见 `.chorus/specs/rss-reader/2026-09-23-theme-preview/keyboard-search.md`。

搜索性能：schema v16补入中文单字候选，先走FTS筛选、再校验标题/正文，纯单字仍按时间排序；v17为相关度检索增加覆盖排序索引，先对命中ID评分并截取前200，再读取这些文章的列表元数据。10k样本从v15升级到v17约2.68秒（含单字词迁移与索引构建）；SSD零驻留页宽泛英文/中文查询从旧版约214–238ms降到首次约65–77ms，热查约25ms。稀疏/无结果单字从661–706ms降到约9–19ms。Xvfb隔离桌面输入到列表渲染宽泛搜索约95ms；首次升级成本为一次性，新增单字会影响BM25词长统计，相关度分数可能变化。原生Wayland、release与其它平台仍待测，见 `.chorus/specs/rss-reader/2026-09-23-theme-preview/search-unigrams.md`。
