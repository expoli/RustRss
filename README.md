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

本机实测：凭据库探针（`cargo run -p rustrss-desktop --example keyring_probe`）在本机 KDE 下完成写入/读回/清理完整往返——**API key 的存储方案不建立在「应该能行」的假设上**。

### 桌面界面（三栏）

```bash
cargo run -p rustrss-desktop                              # 默认库：~/.local/share/rustrss/rustrss.sqlite
RUSTSS_DB=/tmp/demo.sqlite cargo run -p rustrss-desktop   # 指定库
```

- [x] 三栏：智能视图（全部未读 / 星标 / 全部）+ 订阅源（未读数、抓取失败红点）｜文章列表｜正文
- [x] 键盘导航：`j`/`k` 上下 · `Enter` 打开 · `u` 未读切换 · `s` 星标 · `r` 刷新 · `/` 搜索 · `Esc` 清除 · `g`/`G` 首尾
- [x] 设置面板：「`j`/`k` 浏览时标记已读」开关（默认开）+ 当前视图全部已读 / 全部未读（撤销）+ 界面语言 + 主题三态（跟随系统/浅色/深色，选择持久化；启动时窗口以隐藏方式创建、主题就绪后显示，无主题闪变）+ 关闭按钮行为（退出 / 最小化到托盘；托盘不可用时始终退出）+ AI + MCP
- [x] 自绘标题栏：无系统装饰，顶栏可拖拽 / 双击最大化，右上角最小化 / 最大化 / 关闭三键（代码完成 + 编译与真实 Chromium 加载验证；**点击行为与拖拽需桌面会话人工验证**，见验证清单）
- [x] MCP：stdio 与 HTTP 双传输，HTTP 仅回环 + token；应用内一键生成并复制客户端配置
- [x] 全文搜索（接 FTS5，中文可用）、刷新全部、单源双击重试、添加订阅（首页 URL 自动发现 feed）、浏览器打开、复制链接
- [x] 正文安全渲染：白名单清洗 + 相对地址图片/链接解析（详见下）；代码块语法高亮（vendor highlight.js，`language-*` class 优先 + 自动检测，深浅双主题 token 配色；桌面像素效果需人工核验）
- [x] OPML 导入 / 导出（嵌套文件夹压平成 `父/子`；按 `xmlUrl` 去重）
- [x] 系统托盘：显示/隐藏窗口 + 退出（菜单文案跟随界面语言设置，`auto` 时固定中文；托盘文案在 Rust 侧维护、不经 `ui/i18n.js` 的 key-set 自测——这是已知例外；托盘不可用时自动降级为无托盘并日志说明，不崩溃。注意：托盘在真实桌面会话下的行为需人工验证，headless 环境仅验证了代码路径与降级逻辑）
- [x] Linux 打包：产出 `.deb`（**8.1MB，不打包 WebKit**，依赖声明 `libwebkit2gtk-4.1-0, libgtk-3-0, libayatana-appindicator3-1`）
- [x] i18n：zh-CN / en（169 个 key；启动时比对两份字典的 key 集合并把结果打到 stdout，缺 key 数为 0 可机械核对）
- [ ] 便携模式（`portable.txt`）、CSP 收紧（当前 `csp: null`）、列表虚拟化（当前硬上限 200 条）
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

界面行为的三个细节（都是被实际使用或诊断抓出来的）：

- **选中项会自动滚入可视区**（`scrollIntoView({block:'nearest'})`）。先前选中项变化走的是「全量重建列表」且从不滚动，于是按 `j` 往下走时高亮会跑到列表可视范围之外。现在选中项变化只改行高亮，不重建 DOM；只有数据集变化（比如未读视图里读完一篇）才重建，并保持阅读位置。
- 列表列的高度用 `grid-template-rows: minmax(0, 1fr)` 显式约束，否则行的 auto 高度会被内容撑开、整列能滚过窗口底部。
- **普通 `<script>` 的顶层函数声明会变成 window 属性**：`i18n.js` 导出 `applyStaticI18n` 这类名字后，`app.js` 再写同名的顶层 `const` 会在 WebKit 下报 `Can't create duplicate variable that shadows a global property`，而且是**解析期**错误——整份脚本一句都不执行，界面表现为「什么都不发生」。两个文件现在都包在 IIFE 里，只暴露 `window.I18N`。
  页内保留了一个错误上报探针（捕获脚加载失败与未捕获异常，上报到日志）——装它之前，这类失败是“无信息”的；靠它才拿到上面那行报错。

关于第 2 条：页面启动时会跑一次自检（构造带 `<script>`/`onerror`/`javascript:` 的脏 HTML，验证清洗结果与相对地址解析），结果上报到 stdout：日志里看到 `sanitizer selftest ok` 即通过。**这个自检不是形式，它已经抓出两个真 bug**：清洗时误删了 `body` 自身（启动直接失败），以及把相对地址当非法协议删除（导致 feed 里的图片全不显示）。

**添加订阅**：输入框既收站点首页也收 feed 地址，两者是同一条路径——先发现、再订阅。发现只发一次 GET：返回内容本身能按 feed 解析时，输入地址（重定向后）就是订阅地址；是 HTML 则扫 `<head>` 里的 `<link rel="alternate">`，`application/rss+xml` / `atom+xml` / `feed+json` 三个标准 `type` 优先，`type` 缺失或写错时按 href 后缀兜底，相对 href 按页面地址解析成绝对地址。拿到 feed 地址后走原有订阅 + 首次抓取路径；找不到候选则报错并保留原始原因（不做 `/feed`、`/rss.xml` 之类的路径猜测），输入内容与按钮都留在原地，直接重试即可。逻辑在 `rustrss-core/src/discover.rs`，桌面侧只包一层 `discover_feed` command。

## 进度

### M1 · core 数据层（进行中）

- [x] 解析：RSS 0.x/1.0/2.0、Atom、JSON Feed → 统一领域模型（基于 feed-rs 2.4）
- [x] 条目身份判定：源 id/guid 优先；退化场景（既无 id 也无链接）改用内容指纹，保证跨次抓取稳定
- [x] HTML → 纯文本（去标签、剔除 script/style、实体解码、保留块级换行）
- [x] SQLite 存储：schema 迁移、按 `stable_id` 去重 upsert、已读/星标、未读计数、文件夹、抓取状态与缓存头
- [x] 全文检索：FTS5 + 中文预分词（拉丁出词、中文出 bigram；单字中文走 LIKE 兜底）
- [x] 抓取：条件请求（ETag / Last-Modified）、有界并发、单源失败隔离、增量入库

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

### 运行（三种会话各跑一遍）

```bash
cargo run -p rustrss-desktop                              # 按会话自动选择后端
GDK_BACKEND=wayland cargo run -p rustrss-desktop          # 强制 Wayland
GDK_BACKEND=x11 cargo run -p rustrss-desktop              # 强制 X11
env -u DISPLAY GDK_BACKEND=wayland cargo run -p rustrss-desktop   # 模拟无 XWayland
```

页面把诊断数据同时显示在界面上、并通过 `probe_log` 打到 stdout（前缀 `[probe]`），便于外部脚本采集。
