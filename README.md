# RustRss

**把 RSS 阅读带回桌面。** 一个本地优先的跨平台阅读器：订阅和阅读状态留在自己的设备上；需要时，可选用自带密钥的 AI，或通过 MCP 把订阅接入你的 agent 工作流。

Rust + Tauri 2 · SQLite · RSS / Atom / JSON Feed

![RustRss 三栏阅读界面，使用本地演示文章](docs/images/app-overview.png)

界面截图使用虚构演示内容生成。

## 阅读体验

- **专注阅读**：订阅、文章列表和正文分栏展示；支持搜索、稍后读、星标、标签和键盘操作。文章有效标签为手动标签与源继承标签的并集，统一显示、筛选、计数和批量标记；继承项（含双来源）只读，须在订阅源修改，取消手动关联不屏蔽继承（日报仍只按源标签选源）。
- **报纸版式为默认主题**：四套视觉预设（报纸 Print / Clear / Paper / Slate），默认「报纸」——纸墨配色加一处编辑部红，正文与列表标题用衬线字体、版心居中；明暗色可分别选预设，所有值均可覆盖。
- **数据在本地**：订阅、文章与阅读状态存入 SQLite；无需账号或云同步。
- **AI 由你选择**：配置自己的 OpenAI 兼容、Anthropic、Gemini 或 Ollama 服务，用于文章摘要、翻译和每日日报（按本地日历日聚合订阅内容，AI 生成结构化日报，缓存与范围过滤）。对话助手阶段①已接入桌面/手机聊天 UI：侧栏「AI 助手」、手机日报内分段及「讨论这份日报」，支持持久多轮历史、日报快照绑定（无日报也可新建）、取消与启动中断恢复；阶段② Rust 只读工具循环与阶段③四家流式文本/长历史分页已就绪；真实 BYOK 完整验收、引用与工具过程/降级提示的 UI 消费另批实施。
- **日报返回恢复**：页面重载会记住本会话的日报日期/范围或 AI 助手会话；生成状态从后端恢复，每 3 秒检查，完成事件漏收也会刷新成品；生成完成后的报告读取若瞬时失败，会继续轮询重试，成功读取后再结束恢复。应用进程重启会清除中断生成标记、保留已完成报告；未完成任务需重新生成，不自动续跑。位置记忆使用 `sessionStorage`，不保证厂商 ROM 查杀后保留。
- **面向 agent 的 MCP**：让兼容客户端读取你的订阅、文章与每日日报。服务只监听回环地址，默认只读，写能力需单独授权。

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

## 对话助手（阶段①双端 UI + 阶段②只读资料 + 阶段③流式）

- schema 追加 v2→v3（尚未发布）；会话元数据与消息正文分表，手动删除级联清理。应用启动将遗留 `running` 标为 `interrupted`，普通 core/MCP 开库不恢复、不自动重试计费。
- 日报上下文冻结为第一条 `done` user 消息（`scope_json.has_seed` + `seq=1` 标识）；保存真实范围、checkpoint 与报告正文 hash，重写日报后仍讨论原快照。无日报也可会话，但助手明确说明资料不足。
- 每回合最多 6 次模型请求、10 次只读工具调用、120 秒；每请求输出最多 4096 tokens，已报告 token 下界累计超过 32,000 熔断（即使某次 usage 缺失，后续已知用量仍计入护栏）。输入采用保守 48,000 字符闸门（不是账单 token 估算，含工具声明/反馈），旧历史按完整回合裁剪并返回 `historyTrimmed`。绑定日报在工具模式恒钉住；降级时仅替换请求里的 seed 为相关章节，持久快照/徽章不变。provider usage 缺失即未知；已知会话累计达到 200,000 tokens 后要求新会话，暂不提供继续付费旗标。
- provider/模型/端点变化时旧会话拒绝发送，须开启新会话；同会话单 flight，在飞删除拒绝（先停止、等待终态）。停止会丢弃在途 HTTP future，但已发生费用不保证撤销。
- Tauri 提供 `chat_send/stop/capability/sessions_list/session_get/session_delete`；返回 `sessionId/messageId`（本回合 user id），事件 `chat:started/progress/chunk/done/error` 携带 `sessionId/messageId/seq` 身份（事件 seq 按回合递增，与持久消息 seq 不同）；终态另给 `assistantMessageId/blocks/usage/status`。双端共用聊天容器，以会话身份处理终态事件，不因后台完成抢导航。消息按 id 增量更新，Markdown v1 仅显示安全转义纯文本。
- 发送前按 provider/model/base_url 指纹与会话范围确认端点、正文外发与 token 费用；授权记在本地 `chat.privacyConfirmed.<指纹>`，换端点/模型或范围重新确认。已有会话未成功加载历史时禁用输入、发送与重试，加载后按持久范围显示与确认，加载失败不放行。会话头部提供新建、历史切换与删除；失败保留草稿并可手动重试，成功回执只清空仍与提交快照相同的输入，保留等待期间的新草稿。切换语言即时翻译聊天控件/消息并保留草稿与滚动位置；删除迟到回执不抢文章或其它会话导航。未配 key 显示 AI 设置入口，预算/配置漂移直接展示后端错误并可新建会话。
- 输入框 Enter 发送、Shift+Enter 换行，中文 composition 期间不发送；运行中按钮改为停止。手机助手位于日报 tab 内，不新增底栏 tab，输入区随 WebView/IME 可用高度贴底。
- 会话不存 API key；SQLite 聊天正文不加密，整库备份含聊天及日报上下文。seed 仅显示「日报上下文」徽章，不进入可编辑输入；发送的日报、历史与新消息会交给配置的服务商。来源引用与工具过程/降级徽章的 UI 展示属于后续批次，本批不伪造引用或实机验证。

### 阶段② Rust 只读工具与诚实降级

- 受限范围 FTS 候选阶段使用含 `feed_id` 的覆盖索引，不为判定范围读取正文表；无范围搜索保持原索引。新索引并入**未发布 v3**，不追加 v4、不修改已发布 v1/v2。已跑过旧 v3 的开发库不会自动重跑迁移：请先备份，再重建开发库、从 v2 备份重新迁移，或手动补建 `CREATE INDEX idx_entries_scoped_search_order ON entries(id, read, COALESCE(published_at, fetched_at) DESC, feed_id);`。不能只把现有 v3 的 `user_version` 改回 2（会重复创建聊天表）。

- core `ai::tools` 下沉 MCP 的 10 个只读投影：`list_feeds/list_folders/list_articles/search_articles/get_article/get_unread_summary/db_stats/list_tags/digest_list/digest_get`；MCP 仍返回原有无界单取 JSON，聊天在共享投影上增加 JSON Schema 子集/Rust 校验、凭据脱敏与预算，不开放写入、主题、刷新、抓全文或生成日报。
- 聊天单结果 ≤12KiB、回合证据 ≤48KiB；超限回灌有效 JSON 截断标记与 UTF-8 前缀；受限范围的 `scope_feed_count` 始终保留为结构化字段并计入字节预算，正文/日报在 SQLite 读取时先取有界片段。回合内按工具名与规范化 JSON 参数去重，单批或跨轮的非相邻重复也直接熔断（新解释文本不能绕过）；次数/时间/上下文预算触发停止；只持久最终问答对，中间工具调用与结果不跨回合回放。
- 会话范围从持久 `scope_json.scope_key` 注入；`tags:1,3` 是源标签 OR 的**当前** feed 成员，不是条目标签，也不改冻结日报。列表/FTS/正文/订阅/分组/统计限制到成员范围；日报仅取精确范围（省略 scope_key 时注入会话范围），范围在日期去重和 LIMIT 前过滤。受限会话 `list_tags` 明确拒绝 `scope_unsupported`，防止泄漏全库标签未读计数；结果附 `scope_feed_count`。
- 能力缓存按 provider+端点指纹+模型区分 unknown/confirmed/unsupported，只根据明确 HTTP 400 不支持 tools/function 的错误更新；401、429、超时、无效参数/schema 不当作能力不足。降级后不声明 tools，注入本地 FTS ≤10 条短摘要、≤3 段正文及相关冻结日报章节，合计 ≤8,000 字符；找不到资料明确要求缩小范围。Gemini 签名 opaque 原样回放，思考文本不显示。
- `chat_send` 回执/事件增加 `degraded/toolCallsLog`，终态增加 `degradedReason`；`chat:progress` 工具阶段含 name/summary/ok/truncated。这些字段供下一 UI 批消费，当前 UI 尚未显示能力、过程或「本地检索辅助模式」提示。四家真实 BYOK 检索→取文→回答尚未验收，mock 不替代实测。

### 阶段③ 流式与体验

- core `execute_chat_turn_streaming` 与 `run_agent_turn_streaming` 复用原代理/TLS 根集、16MiB Content-Length/累计体积闸门、120 秒回合护栏与取消路径。只流模型文本，不流工具执行过程/思考链；工具参数收齐后仍按原白名单与预算执行。
- OpenAI 兼容：SSE delta 文本、按 index 拼接工具 arguments，`[DONE]` 必须到达；请求带 `stream_options.include_usage`，仅 HTTP 400 明确提及该参数时去掉重试一次。无 usage 仍标未知，不重试普通 400/401/429。
- Anthropic：SSE `message_start` 输入 usage、`content_block_delta` 的 text/partial_json、`message_delta` 停止原因/输出 usage，`message_stop` 终止。Gemini：`streamGenerateContent?alt=sse` 的 data 候选/完整 functionCall parts 与签名，finishReason 终止，usageMetadata 独立汇总。Ollama：NDJSON message.content 与完整 tool_calls，done:true 终止，prompt_eval_count/eval_count 计量。
- 增量输入是字节，完整行/事件边界才解码 UTF-8；显式 SSE id 重复去重，无 id 的相同文本不能猜作重传（模型可能合法重复）。EOF/连接中断/超时不静默成功，错误含已收字符数；失败与停止保留部分回答，失败信息也落库，不自动付费重发。完整帧已报告的 usage 分量在断流、请求/回合超时与取消时仍随快照落库（即使在途 future 被丢弃），只有缺失分量保持未知，不以 0 覆盖已知用量。
- `chat:chunk {sessionId,messageId,seq,text}` 每约 120ms 或累计 ≥80 字符合并；前端约 120ms 批量 patch 当前 assistant 文本节点。按会话/回合/序号丢重复、迟到和终态后的 chunk；回执前或离开页面收到的文本保留，不抢导航。距底部 ≥48px 时不强制滚底。
- `chat_session_get(since_seq?,limit?)` 默认最新 50 条，limit 限 1–200；since_seq 是当前最早**持久消息** seq，取严格更早页并按正序返回。上滚到顶单 flight 加载 50 条，prepend 用高度差保持位置，已有气泡不重建。双端共用现有显式聊天容器/日报分段，无需修改 mobile.js 路由。消息列表 polite live log、输入 aria-label、停止按钮保持键盘可达。
- Rust 四家成功/断流与工具回灌 mock、真实 HTTP 分包和 JS 竞态测试已通过；Xvfb 重建真产物的流式/上滚/分页/断流证据见 [.chorus 阶段③记录](.chorus/specs/rss-reader/2026-10-04-ai-chat-assistant/evidence/stage3/record.md)。真实 BYOK 与手机实机不属于本批验证。

## 隐私概览

文章库和阅读状态保存在本机。AI 请求会把所选文章内容发送到你配置的服务商；MCP 可让你授权的本地客户端读取订阅。开启文章缩略图时，WebView 会直接请求图片站点。详见[隐私与数据](docs/privacy.md)。

## 许可证

[AGPL-3.0-or-later](LICENSE)：任何人可自由使用、修改、分发，**包括商业用途**；附加的唯一约束是源码开放义务（第 4、5、13 条）。因此若你修改 RustRss 并以网络服务形式对外提供，需向使用者提供完整的对应源码（本仓库即为对应源码）。

**需要闭源分发、把修改版作为商业产品出货，或不愿承担源码开放义务**时，请另行取得[商业授权](LICENSE-COMMERCIAL.md)。

- 历史版本（提交 `cfaf0dc` 及更早，含切换前的 `nightly` 安装包）仍按 MIT OR Apache-2.0 授权，其授权不可撤销，文本见 [`LICENSES/`](LICENSES/README.md)。
- 版权许可不授予商标权，修改版分发请遵守[商标政策](TRADEMARK.md)。
- 贡献代码前请阅读[贡献指南](CONTRIBUTING.md) 中的贡献授权条款。
