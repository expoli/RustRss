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

### MCP 服务器（已接真实库）

工具集（**只读**，写入类属 P1）：`list_feeds` / `list_articles` / `get_article` / `search_articles` / `db_stats`。

口径：**列表只回元数据 + 短摘要（≤140 字），正文必须用 `get_article` 单独取**；所有列表有上限（默认 10、上限 50）。这是为了不让单次响应撑爆 agent 上下文（见 PRD §6 风险 3）。

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

尚未做（属设置/界面层，已列入待办）：**API key 存入操作系统凭据库**、设置界面、以及「发送前确认」的交互。

### 桌面界面（三栏）

```bash
cargo run -p rustrss-desktop                              # 默认库：~/.local/share/rustrss/rustrss.sqlite
RUSTSS_DB=/tmp/demo.sqlite cargo run -p rustrss-desktop   # 指定库
```

- [x] 三栏：智能视图（全部未读 / 星标 / 全部）+ 订阅源（未读数、抓取失败红点）｜文章列表｜正文
- [x] 键盘导航：`j`/`k` 上下 · `Enter` 打开 · `u` 未读切换 · `s` 星标 · `r` 刷新 · `/` 搜索 · `Esc` 清除 · `g`/`G` 首尾
- [x] 全文搜索（接 FTS5，中文可用）、刷新全部、单源双击重试、添加订阅、浏览器打开、复制链接
- [x] 正文安全渲染：白名单清洗 + 相对地址图片/链接解析（详见下）
- [ ] OPML 导入导出
- [ ] i18n（zh-CN / en）
- [ ] 便携模式（`portable.txt`）、CSP 收紧（当前 `csp: null`）、列表虚拟化（当前硬上限 200 条）
- [ ] 三平台打包（deb / rpm 优先）

两条刻意的设计选择：

1. **只有主动打开才标记已读**（点击 / `j` / `k` / `Enter`）——切换视图或刷新列表不会把没看过的文章标成已读。
2. **正文经白名单清洗后才进 DOM**：脚本、事件处理器、`javascript:` 一律拦下；相对地址的图片与链接会按文章 URL 解析成绝对地址（否则 feed 里的图片全不显示）。
  页面启动时会跑一次自检（构造带 `<script>`/`onerror`/`javascript:` 的脏 HTML，验证清洗结果与相对地址解析），结果上报到 stdout：日志里看到 `sanitizer selftest ok` 即通过——**这个自检不是形式，它已经拓出两个真 bug**（清洗时误删 `body`；把相对图片当非法地址删掉）。

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
