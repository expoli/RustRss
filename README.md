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
| 字体 | 内置字体栈 + 用户可覆盖三类字体族（界面 / 正文 / 等宽）与正文字号、行高；字体枚举只在 Linux 走 fontconfig（`fc-list`），Windows / macOS 本版只提供「跟随系统」（不引 font-kit 这类重依赖） |
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

### MCP 服务器（stdio + HTTP 两种传输，已接真实库）

工具集（**只读**，写入类属 P1）：`list_feeds` / `list_articles` / `get_article` / `search_articles` / `db_stats`。

口径：**列表只回元数据 + 短摘要（≤140 字），正文必须用 `get_article` 单独取**；所有列表有上限（默认 10、上限 50）。这是为了不让单次响应撑爆 agent 上下文（见 PRD §6 风险 3）。

列表口径与界面**同源**：`list_articles` / `search_articles` 走 core 的同一条查询路径，因此跟随用户当前的列表设置——排序档（`list.sort`）取的就是用户在列表头选的那一档，「隐藏已读」（`list.hide_read`）开着时也已关的已读条目（星标 / 稍后读视图的豁免照旧）。理由是硬约束 1（共享数据路径，不存在两套查询逻辑）；若以后需要「agent 不受界面过滤影响」，应在 EntryQuery 上开显式参数而不是让两条路径各读一份设置。

两种传输：

- **stdio**：客户端把 `rustrss-mcp` 当子进程拉起；
- **HTTP**（streamable HTTP）：`rustrss-mcp --http 127.0.0.1:8817`（token 取 `RUSTSS_MCP_TOKEN`，未设则随机生成并打印），或在应用「**设置 → MCP**」里启用——设置页会直接给出可粘贴的客户端配置片段与一行 `claude mcp add` 命令，并可一键复制。

约束（均有测试）：**只绑回环**（非回环地址直接拒绝启动）、无 token / 错 token 一律 401、`/health` 不鉴权且不含任何订阅数据、token 轮换后旧值立即失效。客户端需带 `Accept: application/json, text/event-stream`——这是 MCP 传输规范的要求（rmcp 不对则 406），不是本项目的额外限制。

实测（2026-09-20）：

- `ss -ltn` 显示监听 `127.0.0.1:8817`（不是 `0.0.0.0`）；`/health` 无 token → 200；`/mcp` 无 token 或错 token → 401；对 token（查询串或 `Authorization: Bearer`）→ 200 且能取到真实订阅数据。
- **真实客户端**：Claude Code 以 HTTP + Authorization 头接入，正确报出 3 个订阅源；当被要求列未读条目时，它发现库里未读为 0 并**拒绝编造**，指出前提不成立。

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
- [x] 列表渐进加载：滚到底自动续下一批（每批 200 条，keyset 游标分页；游标取上一批末行的 `(sortkey, id)`，由后端直出），未读 / 全部 / 星标 / 稍后读 / 单源视图都支持，加载完全部匹配条目即停（无重复、无跳条）；续页只 append 新行、不重建已有 DOM。搜索仍是一次性 200 条（本版不含分页）。**后台刷新保持滚动位置**：已加载多页（`length > 200`，与 `exhausted` 无关——小库/星标这类已耗尽的多页视图同样保持）时只把比列表首行新的条目 prepend 到头部，并按 `scrollHeight` 增量补偿 `scrollTop`（在顶部 `scrollTop===0` 时不补偿，新条目立即可见）——**只在「最新在前」档**，其余排序档走整列重建（见下一条列表排序）；会话内读过的条目（`Enter` / `u` / 全部已读）不回插。单页、首屏、续页在飞或上批失败仍走整列重建
- [x] 列表排序与过滤（列表头右侧图标按钮 → 菜单）：三档 —— 最新在前（默认）/ 最早在前 / 未读优先（未读组内仍最新在前，即 `read ASC, sortkey DESC`），加一个「隐藏已读」开关。两项都是**全局设置**（`settings` 表的 `list.sort` / `list.hide_read`），**store 在查询时直接读**——界面与 MCP 共用同一条读取路径，前端不往查询参数里再塞一份副本（避免两个事实源），非法值回默认档；切换立即重查重渲（reset 语义、分页状态重置），菜单里当前档与开关都打勾，跨视图切换与重启保留。「隐藏已读」豁免**星标 / 稍后读**视图（读完的星标还得能找回），搜索同样过滤（无搜索豁免）；开启后点行标读只灰显留行（不立即删行，防高亮/操作目标脱钩），行在下次列表重建时离开。keyset 游标按档适配：最早在前方向反转（`sortkey ASC, id ASC`，反扫同一个表达式索引）、未读优先用复合键 `(read, sortkey, id)`（游标多带一位 `read`，缺了就按首页处理而不是静默翻错页），三档续页都不重不漏（`append … dup=0` 自证）。迁移 **v11** 新增表达式索引 `(read, COALESCE(published_at, fetched_at) DESC, id)`，未读优先档全程 `INDEXED BY` 钉住它（本程序从不 ANALYZE，不钉的话 feed 视图会退化成等值索引 + 临时排序）；EXPLAIN 断言与线上 SQL 同源且经变异校验（`unread_first_cursor_plans_use_v11_composite_index` / `oldest_pages_reverse_scan_the_sortkey_index`）。**后台刷新只在「最新在前」档 prepend**：最早在前的新条目属于列表尾部、未读优先的新未读属于未读组头部，插到 DOM 头部会让 DOM 顺序与后端顺序不一致；这两档退化为静默 reset（列表与分页状态重建、正文与阅读位置不动，**只保留 `scrollTop` 像素偏移、不做锚点补偿**——已文档化的取舍：顺序正确优先）。i18n 双语；headless 验证与遗留人工项见验证清单第 15 节
- [x] 键盘导航：`j`/`k` 上下 · `Enter` 打开 · `u` 未读切换 · `s` 星标 · `r` 刷新 · `/` 搜索 · `Esc` 清除 · `g`/`G` 首尾
- [x] 设置面板：「`j`/`k` 浏览时标记已读」开关（默认开）+ 当前视图全部已读 / 全部未读（撤销）+ 界面语言 + 主题三态（跟随系统/浅色/深色，选择持久化；启动时窗口以隐藏方式创建、主题就绪后显示，无主题闪变）+ 关闭按钮行为（退出 / 最小化到托盘；托盘不可用时始终退出）+ 自动刷新（间隔 关/15/30/60/120/360 分钟、启动时刷新开关、新文章通知开关，默认关）+ 字体（见下）+ AI + MCP
- [x] 字体可配置（设置 → 通用 → 字体分区）：界面 / 正文 / 等宽**三类字体族互相独立**（后两个默认「跟随系统」：正文字体跟随界面字体、等宽字体用内置等宽栈），正文字号 13–18px、正文字高 1.5–1.8 可调；全部走 CSS 变量（`--font-ui` / `--font-read` / `--font-mono` / `--font-read-size` / `--font-read-line`），**保存即生效、无需重启，跨会话保持**。字体列表由 `list_font_families` 枚举系统字体（Linux `fc-list`，3s 超时、spawn 失败/超时/非零退出均降级为空表；Windows / macOS 本版返回空表），设置页打开时预取一次并缓存（`fc-list` 实测 18ms）；下拉复用自绘菜单（首项「跟随系统」= 清除变量回内置字体栈），含空格 / CJK 的族名写 CSS 变量时统一加引号并转义。滑块 **`input` 只改 CSS 变量做实时预览、`change`（松手）才写库**（拖动期间零 IPC、零 DB 写入），字号 clamp 13–18 / 行高 clamp 1.5–1.8 在 Rust 侧再夹一次（库里不存越界值）；设置弹层遮住了正文区，因此字体分区里带一块**与正文同源 CSS 变量的实时预览块**（拖动/切换即时可见）。字体枚举失败时三个下拉仍可用（只剩「跟随系统」）并把原因写在 tooltip 上
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

1. **已读判定可配置，且有撤销**：默认 `j`/`k` 浏览时就标记已读（NetNewsWire / Reeder 这类键盘阅读器的主流做法：扫一遍即已读）；设置里可关掉，改成「只有 `Enter`、鼠标点击或 `u` 才改变状态」。误扫之后用「当前视图全部未读」一键回退。无论开关如何，**切换视图与刷新列表都不会改变已读状态**。
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
- **行格式**：`2026-09-22T23:45:01.123 INFO  rustrss_core::fetch: …`（本地时间带毫秒、级别、来源模块）。界面诊断行进同一份文件，target 为 `ui`、正文保留 `[ui]` 前缀——**它们不在 stdout 里**，看界面在干什么要看日志文件。
- **内容**：抓取/入库/刷新/调度/迁移/托盘降级等 Rust 侧关键事件 + 界面诊断行；panic 另写一条 `ERROR`（payload + 位置），stderr 现场照旧。落盘前对凭据形态打码（URL 里的 userinfo、`key=`/`token=` 查询串），AI 错误文本在 core 侧已打码。
- **级别**：设置 → 关于 →「日志级别」（存库键 `log.level`，默认 `info`）。`debug` 记录更细的过程（刷新批次、逐源失败、HTTP 请求耗时），**改完即时生效、无需重启**，跨重启保持；debug 级只收录本应用（`rustrss*` 与 `ui`）的记录，依赖库（h2/hyper/reqwest…）的 debug 不写文件。
- **保留策略**：每次启动清理一次，**保留最近 20 个文件且总量 ≤ 50MB**（两条上限独立生效，超限从最旧的开始删）；清理结果写进当次日志（`日志保留清理: 删除 N 个（剩 N 个 / N 字节）`）。
- **反馈问题**：设置 → 关于 →「打开日志目录」用系统文件管理器打开该目录，把**最新的那个文件**附在 issue 里。启动器起不来（如裸容器没装 `xdg-open`）会在状态栏给出可读错误（含原因）且应用不崩溃。**已知取舍**：启动器在、但它自己打不开（环境里没有文件管理器）时无法检测——`spawn` 成功不等于目录真的打开（实测 Xvfb 下 `xdg-open` 静默 `exit 0`），此时只显示「已**请**系统文件管理器打开」，文案刻意不说「已打开」；与既有「浏览器打开」同一口径。
- **初始化失败不阻断启动**：日志目录不可写（权限 / 磁盘满）时降级为「本次不写日志文件」，只在 stderr 打印一行原因，其余功能照常。

## 进度

### M1 · core 数据层（进行中）

- [x] 解析：RSS 0.x/1.0/2.0、Atom、JSON Feed → 统一领域模型（基于 feed-rs 2.4）
- [x] 条目身份判定：源 id/guid 优先；退化场景（既无 id 也无链接）改用内容指纹，保证跨次抓取稳定
- [x] HTML → 纯文本（去标签、剔除 script/style、实体解码、保留块级换行）
- [x] SQLite 存储：schema 迁移、按 `stable_id` 去重 upsert、已读/星标、未读计数、文件夹、抓取状态与缓存头
- [x] 全文检索：FTS5 + 中文预分词（拉丁出词、中文出 bigram；单字中文走 LIKE 兜底）
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
