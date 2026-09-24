# 人工验证清单（需真实桌面 GUI 会话）

> headless 环境无法覆盖的平台级验证项，全部来自评审 NOTE 与报告 Follow-ups。
> 每项验证通过后在方括号打勾并注明日期与环境（发行版 / 桌面会话类型）。

## 1. 主题三态像素核验（功能②，commit 530e6de）

- [ ] 深色（默认）下：三栏、设置对话框、AI 面板、正文（含引用/表格/代码块）逐屏无不可读文本
- [ ] 深浅两态下**原生控件**（设置页下拉框/输入框、滚动条、弹出的选项列表）背景与文字跟主题（曾漏检：select 在深色下白底，已用 color-scheme 修复，commit 876c3d4）
- [ ] 浅色（手动固定）下同上
- [ ] 深色（手动固定）下同上
- [ ] 跟随系统态：系统切换深浅时界面实时跟随、无需重启
- [ ] 重启后主题选择保持；把库里 `ui.theme` 改成垃圾值后启动回退到跟随系统
- [ ] 分数缩放 125% / 150% / 170% 下三态显示正常（与 spec 缩放验收点合并验）

## 2. 托盘行为与降级（功能④，commit 48e50dd）

- [ ] 自绘标题栏（commit 4d043a2）：顶栏拖拽 / 双击最大化可用；右上角最小化 / 最大化 / 关闭三键均生效且随主题
- [ ] 关闭行为设置：选「最小化到托盘」后点关闭 → 窗口隐藏、托盘可唤回；选「退出程序」或托盘不可用 → 点关闭直接退出
- [ ] X11 会话：托盘图标出现，显示/隐藏窗口、退出菜单可用
- [ ] Wayland 会话（KDE + GNOME 各一）：同上
- [ ] 无 XWayland 的 Wayland 会话：主流程可用；托盘不可用时无崩溃且日志含 `[rustrss] 托盘不可用`
- [ ] 缺 libayatana 的最小环境（可用容器模拟）：应用正常启动、降级日志出现
- [ ] 窗口关闭按钮 = 直接退出；托盘「显示」对已隐藏窗口生效

## 3. 添加订阅端到端（功能①，commits ad573dc + aa45fc1）

- [ ] 粘贴网站首页 URL（如 https://www.ruanyifeng.com/blog/）：自动发现并订阅成功
- [ ] 粘贴直接 feed URL：行为与旧版一致（订阅成功、地址与输入一致）
- [ ] 粘贴无 feed 的页面：错误提示含原因，输入保留，可重试
- [ ] 订阅含 `&amp;` / 数字实体参数的 feed 地址，抓取 URL 正确（实体解码回归项）

## 4. 代码块高亮像素核验（功能③，commits d128f7a + fa0059c）

- [ ] 打开含 `language-*` class 代码块的文章：按语言高亮
- [ ] 无 class 代码块：auto-detect 高亮；乱造内容原样纯文本显示、无报错
- [ ] 深浅两态下 token 配色均可读；手动固定深色时系统为浅色的高亮不串色

## 5. 自动刷新三件套（功能⑤，2026-09-21-auto-refresh）

> 后端启动首刷的 10s 延迟与刷新管线已用 headless 冒烟验证（Xvfb + `RUSTSS_DB` 临时库 + 本地 feed：
> 启动 10.2s 时打印「自动刷新完成: fetched=1 inserted=2」，库内条目标题 Item A / Item B，feed last_status=ok）；
> 下列是必须真实 GUI 会话才能看的交互项。

### 5.1 前端（设置控件 + 事件 + OPML 增量抓取）已机械验证的部分

headless 跑法：`Xvfb :99` + `GDK_BACKEND=x11`（**测试进程的环境，不是应用代码设置**；注意用户会话若已导出
`GDK_BACKEND=wayland`，不覆盖它会连到真实桌面会话去）+ `RUSTSS_DB=/tmp/...` + 本地 feed（`python3 -m http.server`，
为了截到状态栏提示特意 sleep 3s）+ `xdotool` 驱动真实 GUI 会话，读回 `sqlite3` 与 `ui_log` 的 stdout：

- 设置页「通用 → 自动刷新」渲染正确：间隔 select 六档（关闭/每 15 分钟/每 30 分钟/每 60 分钟/每 2 小时/每 6 小时，
  与 Rust 侧白名单一一对应）+ 启动时刷新开关；值来自 `get_ui_settings`（重启后仍回显 15 与关）。
- 改间隔 → 库内 `refresh.interval_minutes=15`、日志 `refreshInterval=15`；关开关 → 库内 `refresh.on_start=false`、
  状态栏「已关闭「启动时刷新」」、日志 `refreshOnStart=false`。
- 后台刷新事件链路：日志出现 `[ui] refresh:start` → 状态栏「后台刷新中…」（已截屏）→ `[ui] refresh:done`。
- **静默刷新不打扰阅读**（关键不变量，日志自证）：打开 60 段长文滚到第 21 段（scrollTop=870）后触发启动首刷，
  日志 `refresh:done 静默完成 selected=5→5 正文=3869→3869字 scrollTop=870→870`，同时侧栏计数 5→6、
  列表插入新条目——即“列表/侧栏更新但正文与滚动位置零变化”。
- OPML 导入后自动抓取：导入含 2 个新源的 OPML → 日志 `import_opml added=2` → `refresh_feeds imported=2` →
  侧栏出现 2 个源与计数（Local Feed A 2 / Local Feed B 2，共 4 条），库内两源 `last_status=ok`，
  状态栏「刷新完成：成功 2｜未修改 0｜新增 4｜失败 0」。重复导入（0 新增）不触发的分支由
  `crates/rustrss-core/tests/opml.rs`（`added_feed_ids` 为空）+ 前端 `feeds_added > 0` 条件共同保证。
- `ui/i18n.js` 双份 key 一致（226 个，启动自检 `i18n selftest ok (keys=226)`，另有 node 侧等价脚本核对
  `data-i18n*` 引用全部存在）。

### 5.2 仍需真实桌面会话人工核验

- [ ] 设置 → 通用：改「自动刷新间隔」（关/15/30/60/120/360 分钟）与「启动时刷新」开关，重启后值保持；把库里 `refresh.interval_minutes` 改成垃圾值重启 → 回显默认 30 分钟（`refresh.on_start` 改垃圾值 → 默认开）
- [ ] 英文界面（设置 → 界面语言 = English）下新增文案显示正确：`Auto refresh` / `Auto refresh interval` /
      `Every 15 minutes` / `Refresh on startup` 与状态栏 `Refreshing in the background…`
- [ ] 间隔设为 15 分钟并保持应用运行：到点自动刷新一次，状态栏出现「后台刷新中…」提示、完成后侧栏计数与列表静默更新且**不打断当前阅读焦点**
- [ ] 后台刷新进行中再点刷新按钮（或在 15 分钟档下到点再触发一次）：不出现两条并发刷新；手动这次得到「刷新已在进行中」提示而非静默叠加（单 flight）
- [ ] 间隔设为「关」：等待 ≥15 分钟不再出现自动刷新（启动首刷仍按开关独立生效）
- [ ] 启动后 10s 内侧栏计数与列表自动更新（启动首刷）；关掉「启动时刷新」后重启不再自动刷
- [ ] OPML 导入 N 个新源后：只抓新增的那些源（状态栏有提示），完成后侧栏未读计数出现；重复导入同一文件（0 新增）不再触发抓取
- [ ] 手动刷新（按钮 / `r` 键）仍然只有一条同步提示，不出现「后台刷新中…」与手动提示叠加的双提示
- [ ] 慢网络下状态栏「后台刷新中…」一直显示到抓取结束（本地 feed 的窗口很短，本任务是靠故意 sleep 3s 才截到的）

## 6. 列表无限滚动（功能⑥，2026-09-21-infinite-scroll）

> headless 跑法：`Xvfb :104`（1920x1200）+ `GDK_BACKEND=x11`（**测试进程环境，不是应用代码设置**）+
> `HOME` 隔离 + `RUSTSS_DB` 指向临时库副本（8204 条：8000 合成条 + Feed C 202 条 + Feed D 2 条；
> 未读 6203 / 星标 320 / 稍后读 200 / Feed A 4000）；`xdotool` 驱动真实 GUI，读回 `ui_log` 的 stdout
> 与 `sqlite3` 计数比对。不碰真实数据目录。

### 6.1 已机械验证的部分（日志/库计数自证）

- **8k 库滚到底不重不漏**：各视图「G 跳到末行 → 哨兵续页」直到末尾，末页日志
  `append rows=0 total=8204 dup=0 exhausted=true`（全部视图，41 次续页）、`total=6000`（未读）、
  `total=320`（星标）、`total=200`（稍后读，恰好一整批 → 空页判定正确）、`total=4000`（Feed A）；
  每个视图的 total 与 `sqlite3` 对应 `COUNT(*)` 完全一致，全部 append 行 `dup=0`。
- **真实滚动路径**（非键盘驱动）：鼠标滚轮 400 格 → `append rows=200 total=400 dup=0 exhausted=false`；
  即在列表容器内滚动触发哨兵，而不是只有跳转键才触发。
- **j/k 在多批加载后仍正确**：全部视图加载 2600 条后连按 j/j/j/j/k/k/j → `open id=` 依次
  2401→2402→2403→2404→2403→2402→2403，无跳号、无重建。
- **未读视图单行删除 + 焦点保持**：Enter 打开一条 → 日志 `open id=1 markRead=true read=false`，
  其后 0 行 `renderList`（仍无全量重建）；列表 200→199 篇、侧栏未读 6000→5999、高亮移到下一行。
- **列表被读短时自动补页**：未读视图连按 220 次 Enter（全程不滚动）→ 剩余 16 行时
  `append rows=200 total=216 dup=0 exhausted=false`（哨兵自动补页，不会读空后停在「暂无未读」）；
  220 次打开 220 个不同 id（distinct=220），全程 0 行 `renderList`。
- **搜索仍是一次性 200**：库内 FTS 命中 8204 条时界面日志 `view=search count=200 exhausted=true`，
  滚到底 0 次 append（分页状态与列表视图解耦，不装哨兵）。
- **视图切换重装 observer**：搜索视图 → Esc 回未读视图 → 连续 12 次续页正常（observer 重装生效）。
- **续页失败即停（不重试风暴）**：注入故障 `DROP INDEX idx_entries_sortkey` 后连续 5 次触发续页，
  日志只有 1 行 `loadMore failed view=starred: 数据库错误: no such index: idx_entries_sortkey`、
  0 次 append，状态栏显示「加载更多失败：数据库错误: …」（已截屏）；应用其余部分仍可用。
- i18n 双份 key 一致（现 228 个，启动自检 `i18n selftest ok (keys=228)`）。

### 6.2 仍需真实桌面会话人工核验

- [ ] 真机 8k 库上快速连续滚动（触控板惯性 / 滚轮长按）的体感：是否出现「滚到底等一小会儿才补上」的空窗、有没有掉帧
- [ ] Wayland 会话（KDE 与 GNOME 各一）下滚动触发续页行为与 X11 一致（本次是 X11/Xvfb）
- [ ] 分数缩放 125% / 150% / 170% 下哨兵的触发距离（`rootMargin: 600px` 按 CSS 像素算，缩放后可视行数变化）手感是否合适
- [ ] 断网或后端持续失败后的恢复：失败后换视图/刷新能重新开始续页（本次用 DROP INDEX 只验了「失败即停 + 状态栏提示」）

## 7. 全文获取（功能⑦，2026-09-21-fulltext）

> 后端（提取 / 2MB 闸门 / 写回 / 幂等 / 刷新不覆盖）由 core 任务的 core 测试与 command 测试覆盖；
> 本节是**前端按钮路径**的 headless 实测记录——按 PRD 验收 1/2/3/5 逐条对证据，重点是「点了发生什么」
> 与「重开/失败时库与界面各自是什么状态」，这些 core 测试看不到。
>
> headless 跑法：`Xvfb :99`（1920x1200）+ `GDK_BACKEND=x11`（**测试进程的环境，不是应用代码设置**）+ `HOME` 隔离 +
> `RUSTSS_DB` 指向临时库 + 本地回环夹具服务器（`/tmp/rustrss-fulltext/fixture_server.py`：`/feed.xml` 四个条目、
> `/post-summary.html`（真实形状文章页，**故意延迟 3s**）、`/post-full.html`、`/api.json`；每次请求写一行 `access.log`）；
> `xdotool` 驱动真实 GUI（点击按钮 + `j`/`k`/`G`/`r` 按键），读回 `ui_log` 的 stdout、`sqlite3` 与夹具服务器的 `access.log`。
> 库内人工播种 4 条：摘要型（30 字摘要）/ 全文型（664 字正文）/ 指向 JSON 的摘要型 / 指向 `127.0.0.1:1` 的摘要型；
> 自动刷新与启动首刷在库内关掉（`refresh.interval_minutes=off`、`refresh.on_start=false`），网络请求只剩被验证的那一条。

### 7.1 已机械验证的部分（截图 + 日志 + 库 + 服务端访问日志自证）

- **摘要型条目显示按钮**：打开条目 1（摘要 30 字）时 actions 行出现「获取全文」（在「复制链接」与「AI 摘要」之间），
  同一行的全文型条目（664 字）打开后**没有**这个按钮（两张截图对比；判定在后端 `is_summary_entry`，前端只读 `needs_fulltext`）。
- **loading 态 + 防重入**：服务端刻意延迟 3s，按钮被点两次（间隔 0.35s）→ 截图显示按钮变灰、文案「正在获取全文…」、
  正文仍是原摘要；夹具 `access.log` 里 `GET /post-summary.html` **只有 1 行**——第二次点击没有发出第二个请求，
  防重入不是靠按钮 disabled（那会被下一次重渲染清掉），而是模块级 `fulltextInFlight` 标记。
- **成功用返回的 EntryRow 重渲染**：日志 `fetch_fulltext ok entry=1 html=651chars stillNeeds=false` 后紧跟
  `renderReader id=1 sanitize=5 innerHTML-set=6 highlight=6 total=6ms`；截图显示正文已替换为提取结果
  （以「夹具正文标记」开头、导航区与「页脚广告位」都没进正文）、按钮消失、状态栏「已获取全文，正文已更新」；
  库内该行 `fulltext_fetched=1`、`content_html` 651 字、`content_text` 为提取正文（原文摘要被替换，符合预期）。
- **二次打开零网络**：`j` 到全文型条目再 `k` 回条目 1 → 日志 `open id=1 markRead=false read=false`，
  夹具 `access.log` 与操作前**逐字节相同**（`diff` 为空）。**重启应用**后再打开同一篇（`G`）同样零新增请求，
  截图确认正文仍是提取结果、无按钮——即「重开零网络」在 GUI 层也成立（不只后端幂等测试）。
- **失败降级 ①非 HTML**：条目 3 指向 `/api.json`（`application/json`）→ 状态栏红字
  「获取全文失败：目标不是 HTML 页面，已跳过全文提取」，按钮恢复为可点的「获取全文」（可重试），
  正文仍显示原摘要；库内该行 `fulltext_fetched=0` / `content_html` 仍 NULL / 摘要逐字节不变（前后快照 `diff` 为空）。
- **失败降级 ②网络不可达**：条目 4 指向 `127.0.0.1:1` → 状态栏「获取全文失败：获取原文失败: 连接失败: error sending request…」，
  同样按钮恢复、正文与库内状态不变（前后快照 `diff` 为空）。
- **刷新不覆盖已抓正文（PRD 验收 2 后半，端到端）**：抓到条目 1 的正文后按 `r` 手动刷新，夹具第 2 次 `/feed.xml`
  把该条的 description 改了一版（`content_hash` 变了 → upsert 走 UPDATE 分支）→ 库内 `summary` 更新为
  「…（源站已更新：这句话是第二次抓取才有的）」，而 `content_html`/`content_text` 仍是抓到的正文、`fulltext_fetched` 仍为 1；
  条目未重复（`COUNT(*)=4`）。全文型条目（未抓过）在刷新后按常规被源正文覆盖，符合设计。
- **i18n 双语**：启动自检 `i18n selftest ok (keys=233)`（两份字典各 233 个 key，缺 key 数为 0；另有 node 侧等价脚本
  核对 `index.html` 的 125 处 `data-i18n*` 引用与 `app.js` 的 106 个 `t()` key 全部存在）；把 `ui.locale` 改成 `en` 重启后，
  按钮显示 `Get full text`，失败提示包装文案为 `Could not fetch the full text: …`（截图）。
- 全部改动集中在 `ui/app.js`（`renderReader` 一行按钮 + `fetchFulltext`）/ `ui/i18n.js`（5 个 key ×2）/ `README.md`；
  `cargo test --workspace` 全绿。

### 7.2 仍需真实桌面会话人工核验

- [ ] 真源抓取（如只输出摘要的博客 feed）：提取质量、正文里的相对图片/链接、状态栏提示是否符合预期（本次夹具是合成页）
- [ ] 断网（拔网 / DNS 失败 / 代理不可用）下点「获取全文」：状态栏错误与摘要保留（本次用 connect-refused 端口模拟，未模拟 DNS/代理）
- [ ] 大文章（几十 KB 正文）抓取后的打开耗时与滚动手感（提取结果是单块 DOM，是否可感知卡顿）
- [ ] Wayland 会话（KDE 与 GNOME 各一）下按钮点击、loading 态与重渲染与 X11 一致（本次是 X11/Xvfb）
- [ ] 英文界面下后端返回的错误原文仍是中文（`Could not fetch the full text: 目标不是 HTML 页面…`）——全应用一致的既有现象，
      不是本次引入；是否统一后端文案另行决定

## 8. 备份 / 恢复（功能⑧，2026-09-21-backup-restore）

> 恢复的核心风险（边车删除顺序、三种崩溃态、无 pending 不碰边车）由 core 测试覆盖：
> `crates/rustrss-core/tests/backup.rs`（10 个用例：往返一致含 read/starred/read_later/settings、第二连接并发导出、
> 未提交写不可见、校验三情形、暂存→替换+边车清理+只留 1 份 bak、幂等、崩溃态 A/B/C、无 pending 不触碰 `-wal`/`-shm`，
> 以及「边车删除失败必须中止替换」这条顺序方向性测试）。
> 本节记录**真实启动路径**的 headless 实测（PRD 验收 2/3 的端到端部分）与原生对话框链路的人工核验项。

### 8.1 已机械验证的部分（Xvfb + 隔离 HOME + 临时库自证）

headless 跑法：`Xvfb :99`（1920x1200）+ `GDK_BACKEND=x11`（**测试进程环境，不是应用代码设置**）+
`HOME=/tmp/rustrss-e2e/home`（隔离，不碰真实数据目录）+ `RUSTSS_DB` 指向临时库；用 `sqlite3` 造数据与读回，
证据是应用 stdout 的 `[rustrss]` / `[ui]` 日志与数据目录快照。

- **恢复在启动时生效，且先于任何连接打开（PRD 验收 2）**：`old.sqlite`（2 条订阅 OLD-A/OLD-B）里预置
  `pending-restore.sqlite`（另一份库，1 条订阅 NEWDB-MARKER）与**非空垃圾 `-wal`/`-shm`**，重启应用后日志顺序为
  `[rustrss] 已应用暂存的数据库恢复: …/old.sqlite` → `[rustrss] 数据库: …/old.sqlite`；UI 日志
  `renderSidebar feeds=1`、`loaded … refreshInterval=off refreshOnStart=false`（这两项设置**只存在于 pending 库里**）——
  即界面读到的是替换后的库，而不是进程内旧连接；`sqlite3` 读回 old.sqlite 只剩 `NEWDB-MARKER`，
  `pending-restore.sqlite` 消失，`old.sqlite.bak-20260921-181009` 里是 OLD-A/OLD-B（保底回滚内容正确）。
- **无 pending 的正常启动绝不碰边车（不丢未 checkpoint 的已提交事务）**：应用运行中用外部连接写入 2 条订阅 →
  `-wal` 涨到 160712 字节 → `kill -9` 模拟崩溃 → 重启日志**没有**「已应用暂存的数据库恢复」，UI `renderSidebar feeds=2`，
  `sqlite3` 读回 `WAL-A,WAL-B`。（若 apply 在入口无条件删边车，此时主库只有 4096 字节头部，这 2 条订阅就丢了。）
- i18n：启动自检 `i18n selftest ok (keys=244)`（zh/en 各 244，含本次新增 11 个 key；该自检同时核对 `index.html`
  里每个 `data-i18n*` 都能取到文案）；另有 node 侧等价脚本核对两份字典 key 集合一致、`index.html` 130 处
  `data-i18n*` 与 `app.js` 112 个 `t()` key 全部存在；`app.js` 里 `el('act-backup-db')` / `el('act-restore-db')`
  两个 id 在 `index.html` 均有定义。
- `cargo test --workspace` 全绿（含 core `tests/backup.rs` 10 例 + src-tauri 的 `restore_confirm_text` 双语/重启提示单测）。

### 8.2 仍需真实桌面会话人工核验

> 本机 Xvfb 下 XTEST 鼠标/键盘事件没能送进应用窗口：点击「设置」无任何日志或界面变化，
> `xdotool getmouselocation` 在该窗口区域内返回 `WINDOW=0`（`visible:false` 与 `WEBKIT_DISABLE_COMPOSITING_MODE=1`
> 两种跑法都试过）。因此**原生对话框那一段链路（目录/文件选择器 + 覆盖确认）没有做点击验证**，只做了代码审查与单测。

- [ ] 设置 → 数据：「备份数据库…」→ 目录选择器选目录（试一个含中文的路径）→ 状态栏「已备份到 …」，目录里出现
      `RustRss-backup-<时间戳>.sqlite`；`sqlite3` 能直接打开查询（旁边没有 `-wal`/`-shm`），拷到另一台机器可直接用
- [ ] 「从备份恢复…」：zh-CN 与 en 两种界面语言下各走一次——确认框文案与状态栏「重启后生效」提示；
      点取消不产生 `pending-restore.sqlite`，点确认才产生
- [ ] 选错文件（文本文件 / 0 字节文件 / 别的应用的 sqlite / `user_version` 超前的库）：状态栏报错、**现库不变**、不产生 pending
- [ ] 恢复后重启：库内容 = 备份内容；数据目录出现 `.bak-<时间戳>`（只留 1 份）；把 `.bak-*` 手工改回 `rustrss.sqlite` 能回到恢复前状态
- [ ] Wayland 会话（KDE / GNOME 各一）下两个按钮与原生对话框行为与 X11 一致（本次是 X11/Xvfb）

## 9. 新文章通知与托盘未读角标（功能⑨，2026-09-21-notifications）

> 判定纯函数、角标图标绘制与「无托盘 no-op」由 `src-tauri` 单测覆盖
> （`notify.rs` / `tray.rs` / `scheduler.rs`）。本节记录**后台刷新触发链路**的 headless 实测，
> 托盘角标的**像素级外观**必须真实桌面会话人工核验（headless 下 appindicator 不向会话总线注册
> StatusNotifierItem，D-Bus 里读不到 `IconPixmap`）。
>
> headless 跑法：`Xvfb :99`（1920x1200）+ `GDK_BACKEND=x11`（**测试进程的环境，不是应用代码设置**）+
> `HOME` 隔离 + `RUSTSS_DB` 指向临时库 + 本地夹具 feed（`/tmp/rustrss-notify/fixture_server.py`：
> `/feed.xml` 两个条目，每次请求写一行 `access.log`）；库内播种 1 个指向夹具的订阅与
> `refresh.on_start=true` / `refresh.interval_minutes=off` / `notify.new_articles=1|0` / `ui.locale=en`；
> 复现脚本 `/tmp/rustrss-notify/smoke.sh`（两轮：开关开 / 开关关）。

### 9.1 已机械验证的部分（应用 stdout 日志 + 库回读自证）

- **后台刷新触发通知（PRD 验收 1 的核心）**：开关开的一轮，日志顺序为
  `[ui] loaded … unread=0` → `[rustrss] 自动刷新完成: fetched=1 not_modified=0 inserted=2` →
  `[rustrss] 新文章通知: 2 篇（locale=en）` → `[ui] loaded … unread=2`；库内 `unread=2 / entries=2`。
  即：后台（启动首刷）路径比对前后未读数 0→2 → 差值 2 → 弹一条**聚合**通知（不是每条一条）。
- **开关关（默认值）→ 不通知**：另一轮库内 `notify.new_articles=0`，同样 `inserted=2`，日志里**没有**
  任何「新文章通知」行——开关是唯一的门（缺 key 时默认关，等价于这轮）。
- **采样点在单 flight 之内**：`before_unread` 在 `try_begin_refresh()` 拿到守卫**之后**采样
  （`scheduler.rs::background_refresh`），手动/后台共用同一个标记，采样窗口内不会有第二条刷新并发改未读数。
- **手动刷新路径不通知**：`maybe_notify` 只出现在 `scheduler.rs::background_refresh`
  （`grep -rn "maybe_notify" src-tauri/src` 只在 `notify.rs` 定义 + `scheduler.rs` 调用一处），
  `refresh_all` / `refresh_feeds` / `refresh_feed` 三个手动命令不经过它；手动路径同样不动角标
  （角标在下一轮后台刷新自愈）。
- **角标同步的触发点**：后台刷新后（`scheduler.rs`）与改变未读数的命令之后
  （`set_read` / `mark_all_read` / `mark_all_unread` / `remove_feed` → `commands::sync_badge`）；
  托盘构建成功后还会用库内当前未读初始化一次（`tray::setup_tray` 末尾）。
- **角标绘制与降级（单测）**：`paint_badge` 在右上角画出角标色 `(229,72,77,255)`、其余像素不动，
  尺寸不合法（宽高 0 / 字节数不符）返回 `None` 保持原图标；`badge_tooltip` 0 篇 = `RustRss`，
  > 0 篇 = `RustRss · N 篇未读` / `RustRss · N unread`（**品牌名始终保留**）；
  `update_badge_without_a_tray_is_a_silent_noop` 用 tauri mock app（无托盘）验证 no-op 不 panic。
- **降级不刷屏（headless 实测）**：两轮运行的应用日志里 `托盘` / `角标` / `badge` 相关错误行数为 0、
  `panic` 行数为 0；通知发不出去（会话无通知守护进程）时插件把底层失败吞在异步任务里，日志只有
  `[rustrss] 新文章通知: N 篇（locale=…）` 一行。
- **设置读写与 i18n**：`[ui] loaded … notifyNewArticles=true|false`（说明 `get_ui_settings` 回读正确、
  库内 key `notify.new_articles` 生效）；启动自检 `i18n selftest ok (keys=246)`，
  另有 node 侧等价脚本核对两份字典 key 集合一致（各 246）、`index.html` 124 处与 `app.js` 112 处
  引用全部存在；`set_notify_new_articles` 的序列化字段（`notify_new_articles`）与前端读取一致。
- `cargo test --workspace` 全绿（src-tauri 23 个单测，含本次新增 7 个）。
- **已知无关噪音（非本次引入，未修）**：每轮后台刷新会有一行
  `[rustrss] WAL checkpoint 失败（不影响数据）: 数据库错误: Execute returned results`——
  HEAD 的 `Store::checkpoint_wal` 用 `conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")`，而该 PRAGMA
  返回一行，rusqlite 的 `execute` 必然报 `ExecuteReturnedResults`（commit 61c95c6 引入）。
  数据无风险（只是不截断 WAL），与本功能无关，已作为发现上报。

### 9.2 仍需真实桌面会话人工核验

- [ ] 设置 → 通用 →「自动刷新」块出现「新文章通知」开关，默认关；打开后重启仍为开（库内 `notify.new_articles=1`）
- [ ] 打开开关、间隔设 15 分钟（或等启动首刷）到点有新增未读：弹出**一条**「2 篇新文章」聚合通知（不是每条一条）
- [ ] 点击通知：主窗口被唤出并 focus（Windows/macOS 由系统激活应用；Linux 依 DE——桌面端插件不提供点击回调，仅展示也可接受，已作为平台差异记录）
- [ ] 关掉开关后后台刷新不再弹通知；手动刷新（按钮 / `r`）在开关打开时也**不**弹通知
- [ ] 托盘角标：未读 > 0 时托盘图标右上角出现红点、悬停 tooltip 为「RustRss · N 篇未读」；读完全部未读后红点消失、tooltip 回到 `RustRss`（数字角标是后续增强项，本次只做红点 + tooltip 数字）
- [ ] 英文界面（设置 → 界面语言 = English）下：通知正文 `N new articles`、tooltip `RustRss · N unread`
- [ ] Wayland（KDE / GNOME 各一）与无 StatusNotifierItem 的环境：托盘不可用时应用正常跑、无错误刷屏；有托盘时角标行为与 X11 一致
- [ ] 无通知守护进程的会话（如仅有窗口管理器的 X11）：后台刷新弹出通知失败时应用无卡顿、无错误弹窗、后续刷新照常

## 10. 单实例锁（真机补充项）

headless 已机械验证（P0-5 任务报告）：默认库二次启动 264ms 自退、D-Bus ExecuteCallback 唤窗、焦点转移实测、RUSTSS_DB 多开共存。以下需要真实桌面会话核验：

- [ ] 托盘隐藏态（关闭到托盘）后二次启动：主窗口从托盘唤出并获得焦点（headless 无法复现 GTK 侧隐藏语义）
- [ ] Wayland 会话下二次启动行为一致

## 11. 每源独立刷新间隔（2026-09-22-per-feed-refresh，前端任务 2e1fab5f）

> headless 跑法：`Xvfb :99`（1920x1200）+ `GDK_BACKEND=x11`（**测试进程环境，不是应用代码设置**）+
> `HOME`/`XDG_DATA_HOME` 隔离到 `/tmp/rr-perfeed-ui/home` + `RUSTSS_DB=/tmp/rr-perfeed-ui/e2e.sqlite` +
> 本地 fixture 服务（`python3 -m http.server 8792`，3 个源 feed_a/b/c.xml，访问日志即请求证据）+
> `xdotool` 驱动真实 GUI；读回 `sqlite3`、应用 stdout（`[ui]` 行）、fixture 访问日志。
> 两处环境细节：① 加 `WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1`，否则菜单区域截图出现黑块；
> ② GTK tooltip 是独立 X 窗口，用 `import -window <应用窗口>` 会截成一整块黑，**要截 tooltip 得用 `import -window root`**。
> 用户真实库（`~/.local/share/rustrss/rustrss.sqlite`）mtime 全程停在 09-21 18:59，未被触碰。

### 11.1 已机械验证的部分（截图 + 库回读 + 访问日志自证）

- **菜单结构与勾选（zh-CN）**：右键某源 →「移动到：Work」→ 分隔线 → 灰字组标题「刷新间隔」→
  ✓跟随全局（关闭）/ 每 15 分钟 / 每 30 分钟 / 每 60 分钟 / 每 2 小时 / 每 6 小时（截图 `shot-menu-with-folder.png`）。
  点「每 15 分钟」后重开菜单，✓ 移到 15 那一档且该行加粗（`shot-menu-checked15-crop.png`）——勾选态来自
  `FeedRow.refresh_interval_minutes`，经 `refreshCounts()` 的同一条 `sidebar_data` 路径刷新。
  「跟随全局」项把当前全局档写进文案（全局 off → 「跟随全局（关闭）」；若全局是 30 则是「跟随全局（每 30 分钟）」），
  与设置页 Hint 的例外口径一致。
- **点选即落库**：点「每 15 分钟」→ 应用日志 `[ui] feed 2 refreshInterval=15` 紧接 `[ui] renderSidebar feeds=3`
  （= `invoke('set_feed_refresh_interval')` 成功后 `refreshCounts()`），库内 `feeds.id=2` 由 NULL 变 15；
  点「跟随全局（关闭）」→ 回 NULL（日志 `feed 2 refreshInterval=global`）；再点「每 30 分钟」→ 30。
  侧栏 tooltip 同步显示独立档（zh `独立刷新间隔：每 15 分钟` `shot-tooltip-b-crop.png`；
  en `Own refresh interval: Every 30 minutes` `shot-tooltip-en2-crop.png`），跟随全局的源不显示这一行。
- **覆盖优先于全局关闭（本任务的关键例外）**：库内全局档 `refresh.interval_minutes=off`、`refresh.on_start=false`，
  三个源的 `last_fetched_at` 都预置成约 19 分钟前 —— A 覆盖 15min、B 跟随全局、C 覆盖 360min。
  **先**验证 B 跟随全局且全局 off：tick 只刷 A（fixture 日志仅 `GET /feed_a.xml`）。
  再用右键把 B 设成 15 分钟：下一 tick（08:47:34）fixture 日志只多一条 `GET /feed_b.xml`，B 的 `last_fetched_at`
  前移到 08:47:34、标题被刷新成 feed 里的 `Fixture B`；C（未到点）与 A（2 分钟前刚刷过、未到点）零请求。
  → 全局关闭时，已单独设置间隔的源仍按各自间隔刷新，未到点的源不刷。
- **「跟随全局」把源交回全局开关**：08:48:37 用菜单把 B 设回「跟随全局（关闭）」后，08:48:34 与 08:49:34
  两个 tick 都没有新的 `feed_b` 请求，B 的 `last_fetched_at` 停在 08:47:34。
- **重启保留**：B 设成 30 分钟后杀进程重启（顺便把 `ui.locale` 改成 en）：库内 B 仍为 30，重开右键菜单 ✓ 落在
  「Every 30 minutes」（`shot-menu-en-crop.png`），tooltip 显示 `Own refresh interval: Every 30 minutes`
  （`shot-tooltip-en2-crop.png`）——重启后勾选态与 tooltip 都从同一列读回。
- **设置页 Hint 双语（评审 B1 的例外）**：zh-CN `shot-hint-zh-crop.png`「开启后到点自动抓取订阅；正在进行的刷新
  不会被叠加。全局关闭时，已单独设置间隔的源仍会按各自间隔刷新」；en `shot-hint-en-crop.png`
  「… When the global setting is off, feeds with their own interval still refresh on their own schedule」。
- **i18n 与测试**：应用启动自检 `i18n selftest ok (keys=249)`（新增 `menu.refreshInterval` /
  `menu.refreshFollowGlobalWith` / `sidebar.feedTooltipInterval` 三个 key，档位文案复用 `settings.refreshMin*`，
  避免两份翻译漂移）；node 侧等价脚本核对两份字典各 249 key、`index.html` 132 处与 `app.js` 121 处引用全部存在；
  `cargo test --workspace` 全绿。
- **事件代理模式不变**：右键仍由 `#feeds` 容器统一代理（`contextmenu` → `openFeedMenu`），行复用不重挂监听；
  菜单原语 `{separator}` / `{header}` / `{checked}` 加在共用的 `openContextMenu` 上，文件夹菜单不受影响。

### 11.2 仍需真实桌面会话人工核验

- [ ] 真实 WebKitGTK 主题/字体下：分隔线、灰字组标题、✓ 勾选标记与标签对齐显示正常（headless 截图已确认结构，像素风格需真机复核），长文案不被 `max-width: 260px` 截断
- [ ] 悬停 source 行：tooltip 三行文案正常显示（Xvfb 下 GTK tooltip 窗口整块黑，只能靠 root 截图读文字）
- [ ] 15 分钟档真等到点：右键设 15 分钟后不动应用，到点自动刷新一次（headless 是预置 19 分钟前的 `last_fetched_at` + 60s tick 复现的，没有真等 15 分钟）
- [ ] 英文界面（Settings → Language = English）下右键菜单与 tooltip 文案；切换语言后重开菜单文案跟着变
- [ ] 全局档从「关闭」改成 30 分钟：下一 tick 里「跟随全局」的源按新档补刷，已覆盖的源不受影响（后端 `due_feed_ids` 纯函数单测已覆盖该分支）
- [ ] Wayland 会话（KDE / GNOME 各一）下右键菜单定位与点击命中（本次是 X11/Xvfb）

## 12. 后台刷新保持列表滚动位置（2026-09-22-scroll-preserve，前端任务 c891f49f）

> headless 跑法：`Xvfb :99`（1920x1200，**无窗口管理器**）+ `GDK_BACKEND=x11`（**测试进程环境，不是应用代码设置**）+
> `HOME`/`XDG_DATA_HOME` 隔离到 `/tmp/rustrss-scroll/home` + `RUSTSS_DB=/tmp/rustrss-scroll/db.sqlite`（真库
> `~/.local/share/rustrss/rustrss.sqlite` 的 mtime 全程停在 09-21 18:59，未被触碰）+ 本地夹具 feed
> （`/tmp/rustrss-scroll/fixture_server.py`，只绑 `127.0.0.1:8799`：`new3` = 300 条已入库种子 + 3 条更新的条目、
> `base300` = 与库内逐条一致的 300 条、`small6` = 5 条 + 1 条新的；**响应前刻意 sleep 12s**，把「启动首刷（10s）→
> fetch → refresh:done」的时点推后到约 t+22s，留出滚到目标位置的窗口）+ `xdotool` 驱动真实 GUI
> （滚轮 400 格触发尾部哨兵续页；`G`/`g` 跳末行/首行）。驱动器 `/tmp/rustrss-scroll/run_scenario.py`（自带断言，
> 逐场景证据 `/tmp/rustrss-scroll/run-<场景>.out` + 截图 `/tmp/rustrss-scroll/shot-<场景>.png`）。
> 库内种子：1 个源 + 未读条目（`published_at` 递减，id 1..300）、`refresh.interval_minutes=off`、`refresh.on_start=true`、
> `ui.mark_read_on_navigate=false`；后台刷新由**启动首刷**触发（定时档最小 15 分钟，headless 里等不起）。

### 12.1 已机械验证的部分（日志 + 截图 + 库回读自证）

刷新分支的自证日志（`refresh:done prepend …`）：`rows` 新增条数 · `ids` 插入顺序 · `sessionReadSkipped` 被会话已读集合
挡下的候选数 · `atTop` 是否在顶部 · `listScrollTop` 前后 · `height` 前后 · `top` 视口顶部第一条可见行 id ·
`head` 列表首行 id · `children` 列表 DOM 行数 · `total` 状态行数。

- **场景 1 · 深滚动保住视口（已加载 2 页，且 `exhausted=true` 也走 prepend——评审 B1 的那条）**：
  未读视图滚轮 400 格 → `append rows=100 total=300 dup=0 exhausted=true`；启动首刷插入 3 条新条目后
  `prepend rows=3 ids=301,302,303 atTop=false listScrollTop=23873→24119 height=24600→24846 top=292→292 head=301
  children=303 total=303`。即：`scrollTop` 的位移恰好等于 `scrollHeight` 的增量（像素守恒 246 = 3×82），
  视口顶部仍是同一行（id 292），新条目按 sortkey DESC 排在最上（head=301，ids 301,302,303 而非反序）；
  同一次刷新的 `refresh:done 静默完成 selected=1→1 正文=1078→1078字 scrollTop=0→0` 说明正文与选中行没动；
  整份日志只有首屏那一次 `renderList`（8k 行的全量重建是打开文章时的 CPU 尖峰来源，prepend 一个都不重建）——
  截图 `shot-deep.png` 与刷新前一样停在 Item 291..299。库侧 `inserted=3 updated=300`。
- **场景 2 · 在顶部（scrollTop=0）不补偿、新条目立即可见**：`G` 跳末行（哨兵续页，`append rows=100 … exhausted=true`）
  再 `g` 跳回首行 → `atTop=true listScrollTop=0→0 top=1→301 head=301 children=303 total=303`：
  视口顶部从 id 1 变成 id 301（最新那条新条目），列表没有为了压住视口而往下顶——
  截图 `shot-top.png` 顶栏下方依次是 New 1 / New 2 / New 3 / Item 000（原首行仍在原位、仍是选中行）。
- **场景 3 · 零新增 = 空操作**：夹具返回与库内逐条一致的 300 条（`inserted=0 updated=300`）→
  `prepend rows=0 listScrollTop=23873→23873 top=292→292 head=1 children=300 total=300`：
  `scrollTop` 逐像素不变、视口行不变、DOM 行数与首行不变；整份日志在 refresh 之后**没有任何 `renderList`**
  （全量重建的唯一入口），代码在 `fresh.length === 0` 时于任何 DOM 操作之前 return——即列表 DOM 一个写都没有，
  侧栏计数仍走原来的 `refreshCounts()`（日志里能看到 `renderSidebar`）。
- **场景 4/5 · 未读视图的会话已读行不回插（openEntry 与 toggleRead 两条路径各一次）**：
  两条路径都是「深滚到第 2 页后把首行（id 1 `Item 000`）标已读」——openEntry 走 `Enter`（日志
  `open id=1 markRead=true read=false`），toggleRead 走 `u`（日志 `renderList rows=299` + `open id=2 markRead=false`）；
  读后 `sqlite3` 读回 `entries.id=1 read=1`。随后把该行在库里翻回 `read=0`（`flip … changed=1 read=0`）——
  **这就是那个竞态窗口**：未读视图里该行已被删掉，而刷新查询仍会把它当成首页候选。
  刷新日志两条路径都是 `prepend rows=3 ids=301,302,303 sessionReadSkipped=1 children=302 total=302 head=301`：
  被挡下的候选恰好 1 条，DOM/状态行数 302（299+3）而不是 303，首行是最新的新条目——即该行没有回插。
  刷新后库内它仍是 `read=0`（`[(1,'Item 000',0), (2,'Item 001',0)]`），说明这是**前端会话集合**挡下的，
  而不是靠服务端 unread 过滤。相同条件下的 3 条新条目照常出现在顶部。
- **场景 6 · 单页库走原来的 reset 路径（评审范围条件 `length > PAGE_SIZE` 的另一侧）**：库内 5 条 + 夹具 1 条新的 →
  日志里**没有** `prepend` 行，`refresh:done` 之后是 `renderList rows=6`（整列重建）、
  且没有 `renderReader`（正文仍不重渲染）；`inserted=1`。
- **场景 7 · 续页失败（`paging.error`）走 reset，不与 append 竞态**：先 `DROP INDEX idx_entries_sortkey` 让
  续页必失败 → 滚轮触发 → `loadMore failed view=unread: 数据库错误: no such index: idx_entries_sortkey`（`paging.error=true`）；
  随后的启动首刷不走 prepend，而是 `renderList rows=200`（回首页、清掉错误态），避免了「prepend 头部 + 同时续页**
  append 尾部」两处同时改列表。
- **i18n / 测试**：本次未新增用户可见文案（i18n key 仍 249，启动自检 `i18n selftest ok (keys=249)`）；
  `cargo test --workspace` 全绿（152 例，纯前端改动不涉及 Rust 断言）。

### 12.2 已知取舍

- **prepend 不重建已加载行**：源站改了已加载条目的标题/摘要时，行内内容要等下一次整列重建（换视图 / 换筛选 /
  手动刷新 / 单页视图）才可见；新条目、未读计数、侧栏不受影响。这是「深滚动不能被重建打断」的必然代价。
- **`paging.error` / 续页在飞时退回整列重建**：这两种情况下列表会回到顶部（错误态优先于位置保持），
  与「刷新不该打断滚动」的取舍相反，但避免了与 `loadMore` 的游标/append 竞态。

### 12.3 仍需真实桌面会话人工核验

- [ ] 高 DPI（125% / 150% / 170%）下补偿的像素守恒：`scrollHeight` 增量与 `scrollTop` 位移在非整数 DPR 下是否仍严丝合缝（本次 Xvfb DSF=1）
- [ ] Wayland 会话（KDE / GNOME 各一）下深滚动 + 定时刷新到点：列表不跳、新条目在顶部（本次是 X11/Xvfb）
- [ ] 真等到定时档到点（15/30 分钟）时的体感：新条目出现的位置、侧栏计数、状态栏提示（本次用启动首刷 + 夹具延迟复现）
- [ ] 触控板惯性滚动尚未停下时刷新到点：补偿会不会造成可见跳动（本次只有离散滚轮事件）
- [ ] 大库（8k 行）深滚动下的刷新开销：`topVisibleRowId` 与 prepend 遍历的是列表 DOM 子节点（本次 303 行，实测 refresh 期间无可感卡顿）
- [ ] 长列表里行高差异很大（含 2 行摘要 / 无摘要混排）时，`top`（视口顶部行）是否仍逐次一致

## 13. 字体配置（2026-09-22-font-config，任务 d27f1965）

> 纯函数（字号/行高 clamp、字体族归一化、fc-list 输出解析）与命令体（5 个 key 的部分写入、
> 越界夹回、空串=跟随系统）由 `src-tauri` 单测覆盖（8 个新用例）；`fc-list` 超时/spawn 失败
> 两条降级路径有真实子进程的测试（`sh -c 'sleep 30'` + 200ms 超时）。
> 本节记录**真实应用**（Xvfb + 隔离 HOME + 临时库 + xdotool 真实点击/拖动）与
> **真实浏览器引擎**（Chromium 加载 `ui/index.html`、桩掉 IPC 边界）的自证。

### 13.1 已机械验证的部分

headless 跑法：`Xvfb :99` + `GDK_BACKEND=x11`（测试进程环境，非应用代码设置）+ `HOME=/tmp/rustrss-font/home`
（隔离，不碰真实数据目录）+ `RUSTSS_DB` 指向临时库；库内播种 1 个订阅 + 1 个富正文条目
（中文段落 / 英文 / `pre>code`）；xdotool 送真实鼠标事件（本轮实测**能**送进窗口，与第 8 节记录的
「XTEST 送不进」不同——那轮用的是 `visible:false` + 合成事件，本轮是窗口显示后按窗口坐标点击）。

- **字体枚举与降级（Linux / 无 fontconfig 两条分支）**：日志 `font families=336`
  （隔离 HOME 下 `fc-list --format=$'%{family[0]}\n' | sort -u | wc -l` 同为 336；真实 HOME 下 383，
  差的 47 个是用户自装字体——即枚举结果跟着进程环境走）。单测另一侧：不存在的二进制 → 空表、
  `sh -c 'sleep 30'` + 200ms 超时 → 空表且立刻返回（子进程被 kill_on_drop 杀掉）。
- **启动即恢复（跨会话保持）**：库里预置 `ui.font_read_size=17` 后重启，启动日志
  `fonts ui=default read=follow-ui mono=default size=17px line=1.55`；改成三条字体 + 13px/1.8 后再重启，
  日志 `fonts ui=AR PL UKai CN read=AR PL UMing CN mono=Andale Mono size=13px line=1.80`，
  截图 `shot-17-restart.png` 与重启前逐像素同观感（侧栏 Kai、正文 Ming、代码块 Andale Mono）。
- **滑块：input 只改 CSS 变量、change 才写库（真实应用 + 真实点击拖动）**：按住字号滑块从 17 拖到 13
  的过程中日志逐档打印 `font font_read_size=14 preview（仅 CSS 变量）` → `=13 preview`，
  同时 `sqlite3` 读回 `ui.font_read_size=17`（**没写库**）；松手后日志 `=13 saved`、库里变成 `13`。
  行高同理（拖动中 1.7/1.8 preview + 库内无 `ui.font_read_line` 行 → 松手后 `1.8`）。
  拖动中截图 `shot-6-drag-13.png`（仍按住）显示数值标签 13px 且预览块字号明显小于 17px 那张。
- **三类字体独立（Chromium 真实引擎 + 真实应用）**：Chromium 计算样式 —
  `body` = `"Noto Sans CJK SC"`（界面字体）、`.article` = `"DejaVu Sans"`（正文字体）、
  `.article code` = `sans-serif`（等宽字体）+ 三个下拉标签各自回显；真实应用里改成
  UI=AR PL UKai CN / 正文=AR PL UMing CN / 等宽=Andale Mono 后截图 `shot-16-article-three-fonts.png`：
  侧栏是 Kai、正文是 Ming、代码块是 Andale Mono，且改正文/等宽时侧栏字体不动。
- **「跟随系统」= 清除变量**：Chromium 里选「跟随系统」后内联样式里 `--font-ui` 被 `removeProperty`
  移除、计算值回落到 `:root` 的内置栈；真实应用里把正文字体选回「跟随系统」→ 库里 `ui.font_read` 为空、
  正文重新跟随界面字体、代码块仍是 Andale Mono（截图 `shot-18-follow-system.png`）。
- **CJK / 空格族名的 CSS 引号**：Chromium 计算样式与内联样式双证 —— 含空格名写入为
  `--font-ui: "Noto Sans CJK SC"`、`--font-ui: "DejaVu Sans"`，`body` 的实际 font-family 就是该值
  （不引号会被 CSS 拆成多个家族名）；`sans-serif` 这类通用族保持裸写（加引号会变成具体家族名）；
  真实应用选 CJK 名 `AR PL UKai CN` 后整个界面确实换成了该字体（截图 `shot-10-ui-font-picked.png`）。
- **预览块与正文同源**：Chromium 里 `.font-preview` 的计算 font-family / font-size / line-height
  分别等于 `--font-read` / `--font-read-size`（17×1.7=28.9px）/ `--font-read-line`，
  `code` 等于 `--font-mono`、字号 = 正文 × 0.89（17→15.13px）。
- **无越界值入库**：单测断言 `set_font_config` 落库前 clamp（99→18、0.4→1.5、NaN→默认）且库内是
  `18` / `1.5` 这种干净字符串；读取路径对 `9` / `not-a-number` 同样夹回或回默认。
- **i18n**：启动自检 `i18n selftest ok (keys=265)`；node/python 侧等价核对 zh/en 各 265 个 key
  一一对应、`index.html` 130 处 `data-i18n*` 与 `app.js` 的 `t()` key 全部存在。
- **`cargo test --workspace` 全绿**（162 例，含本次新增 8 例）；`cargo clippy --workspace --all-targets`
  的告警全在 `rustrss-core`（与本次无关，CI 既知现状）。
- **并入本功能的 UI 修复（独立 commit）**：设置页自绘下拉此前**开不出来**——打开菜单的那一次点击
  继续冒泡到 `document` 上的「点外面就关」监听器，菜单在同一事件里被创建又被删除。语言 / 主题 /
  关闭行为 / 刷新间隔 / 字体五个下拉全中招（Chromium 侧复现：`btn.click()` 后 `#ctx-menu` 立即为 null）。
  修法是让监听器把 `.setting-dropdown` 上的点击不算「外面」。修后 Chromium 与真实应用都能开菜单
  （截图 `shot-9-menu-open.png`、`shot-23-dark-menu.png`），「再点同一个下拉 = 关闭」「点外面关闭」
  均保持原语义。
- **长菜单可滚动**：字体列表 336 项，`#ctx-menu` 增加 `max-height: min(70vh, 460px) + overflow-y: auto`
  且菜单项 `flex: 0 0 auto`（不设高度上限时定位会把 `top` 钳成负数，超出视口的部分点不到）。
  深色/浅色两套主题下菜单、滑块、预览块配色都走变量（截图 `shot-20/22`）。

### 13.2 仍需真实桌面会话人工核验

- [ ] 真机（非 Xvfb）下拖动滑块的手感：预览帧率、是否有可见延迟（本轮只验了语义与最终像素）
- [ ] Windows / macOS 上字体分区只显示「跟随系统」+ tooltip 提示（本轮只有 Linux 环境；代码是
      `cfg!(target_os = "linux")` 分支，非 Linux 返回空表，未在真机跑过）
- [ ] 高分屏（125% / 150% / 170%）下滑块 thumb 与数值标签的像素对齐（本轮 DSF=1）
- [ ] 装了上千字体的机器上 `fc-list` 的实测耗时与 3s 超时的余量（本轮 336 个字体 / 18ms）
- [ ] 中文输入法激活时在下拉菜单里的键盘操作（本轮只用鼠标）
- [ ] 弹层遮住触发按钮时的菜单定位观感：菜单高于视口可用空间时会被向上钳位、盖住触发按钮本身
      （点外面或选中条目都能关，但「再点同一个下拉关闭」在该位置够不到按钮）——是否需要改成向上展开待定

## 14. 订阅源编辑对话框（2026-09-22-feed-edit，任务 312ff15d）

> headless 跑法：`Xvfb :99`（1920x1200，无窗口管理器）+ `GDK_BACKEND=x11`（**测试进程环境，不是应用代码设置**）+
> `HOME=/tmp/rustrss-feededit/home`（隔离）+ `RUSTSS_DB=/tmp/rustrss-feededit/db.sqlite`（临时库）+
> 本地 fixture（`python3 -m http.server 8912`，`feed.xml` / `feed2.xml`）+ `xdotool` 真实鼠标/键盘事件；
> 读回 `sqlite3`、应用 stdout 的 `[ui]` 行、`import -window <应用窗口>` 截图。
> 库内先塞一个源（`feeds` 行，`title='Local Feed A'`），启动后 10 秒首刷从 fixture 学到源站名并入库两条目——
> 即「源站名由抓取写、用户改名不该被它洗掉」这条链路的真机起点。

### 14.1 已机械验证的部分（截图 + 库回读 + stdout 自证）

- **菜单入口与位置**：右键某源 → 立即刷新 → 分隔线 → **编辑** → 移动到：X → 分隔线 → 刷新间隔组 → 取消订阅
  （截图 `shot-02-menu.png`；与 PRD 要求的「立即刷新之后、移入文件夹之前」一致）。
- **对话框内容**：标题输入框 placeholder = **源站名**（`Fixture Channel`）、文件夹下拉 = 未分组 + 已有分组、
  刷新间隔下拉 = 跟随全局（每 30 分钟）+ 五档、订阅地址只读展示（截图 `shot-03-dialog.png`）；
  已有自定义名时输入框预填该值（截图 `shot-16-dialog-prefill.png`）。
- **自绘下拉不与弹窗互踩**：下拉触发按钮带 `.setting-dropdown` 类 → 开菜单那一次点击不被 `document`
  上「点菜单外面就关」的监听当成外部点击（截图 `shot-04-interval-menu.png` / `shot-11-folder-menu.png`：
  菜单真实打开并锚在字段行下沿）；Esc 关弹窗时菜单一并关掉（两者都是「取消」语义）。
- **保存：部分写入 + 一次落库**。只改标题 → 库内 `custom_title='我的改写名'`，`folder_id` / `refresh_interval_minutes`
  原样；改标题 + 文件夹 → `custom_title` 与 `folder_id=1`（开发）都落库、间隔仍是 NULL；
  清空标题 + 选未分组 + 选每 30 分钟 → `custom_title=NULL`、`folder_id=NULL`、`refresh_interval_minutes=30`。
  三条命令的 stdout 行分别印证同一件事（`source_title` 与显示名都在回读行里）：
  `feed 1 config saved name="我的改写名" source="Fixture Channel" folder=1 interval=global`、
  `feed 1 config saved name="Fixture Channel v2" source="Fixture Channel v2" folder=none interval=30`、
  `feed 2 config saved name="AAA 第二源" source="Second Source" folder=none interval=global`。
- **立即生效，且不重建列表/正文**：保存后侧栏（含未读合计与位置）、列表行的源名、列表标题（视图标题）、
  阅读区元信息、状态栏「已保存：X」同步更新（截图 `shot-13-saved.png`、`shot-17-reader-sync.png`、
  `shot-20-cleared.png`）；文章仍开着、正文 DOM 未重建（`shot-17` 与保存前同一篇）。
- **改名后侧栏立刻按显示名重排**：把第二个源改名为 `AAA 第二源` 后它立刻排到 `Fixture Channel v2` 之前
  （截图 `shot-23-resort.png`）——`list_feeds` 的 `ORDER BY COALESCE(custom_title, title) COLLATE NOCASE`。
- **刷新不覆盖自定义名**：fixture 的 `<title>` 改成 `Fixture Channel v2` 后点「刷新全部」→ 库内
  `title='Fixture Channel v2'`（源站名跟着刷新走）、`custom_title='我的改写名'` 保持不变，界面继续显示自定义名
  （截图 `shot-15-after-refresh.png`）；启动首刷路径同样如此（重启后截图 `shot-24-restart.png` 仍显示自定义名）。
- **取消零副作用**：改下拉档位后按 Esc / 点取消 → 库里三列一个都没动（`title/custom_title/refresh_interval_minutes`
  仍为原值），stdout 也没有 `config saved` 行（截图 `shot-06-esc-cancel.png`）。
- **重启保持**：杀掉进程重启 → 自定义名与新顺序照旧（截图 `shot-24-restart.png`；对应 core 侧的
  v9→v10 真文件迁移测试 `migration_v9_to_v10_adds_custom_title_on_real_file`）。
- **i18n**：启动自检 `i18n selftest ok (keys=301)`；node 侧等价核对 zh/en 各 301 个 key 一一对应，
  `app.js` 的 `t()` key 与 `index.html` 的 148 处 `data-i18n*` 全部存在（缺 0 个）。
- **单测**：core 新增 3 例（迁移 v9→v10、显示层 COALESCE + 刷新不覆盖 + 排序、刷新管线端到端不覆盖）、
  `src-tauri` 新增 3 例（补丁对象三态解析、自定义名归一化、只写补丁内字段且非法值不写一半）；
  `cargo test --workspace` 全绿。

### 14.2 仍需真实桌面会话人工核验

- [ ] 真实桌面会话（X11 / Wayland 各一）下对话框与自绘下拉的定位观感：下拉菜单贴字段行下沿展开，
      可用空间不足时会盖住「保存 / 取消」按钮（点外面即可关，功能不受影响）
- [ ] 输入法（fcitx / ibus）激活时在标题输入框里输入中文：候选框定位与回车提交行为（本轮用的是
      `xdotool type` 直接送按键，没经过输入法）
- [ ] 高分屏（125% / 150%）下对话框与下拉的像素对齐（本轮 DSF=1）
- [ ] tooltip 文案：给某源设了独立间隔后，侧栏 tooltip 的「独立刷新间隔：…」在真实桌面下的展示
      （GTK tooltip 是独立 X 窗口，headless 截图整块黑）

## 15. 列表排序与过滤（2026-09-22-list-sorting，任务 8fc0cf98）

> headless 跑法：`Xvfb :98`（1920x1200，无窗口管理器；`:99` 上占着**上一次跑残的验证实例**
> （`/tmp/mark-verify`，非本任务）与别人的 fixture 服务，故另起 `:98`，保证点击落在自己的窗口上）+
> `GDK_BACKEND=x11`（**测试进程环境，不是应用代码设置**）+ `HOME=/tmp/rustrss-sort-verify/w2/home`（隔离）+
> `RUSTSS_DB=/tmp/rustrss-sort-verify/w2/data/rustrss.sqlite`（临时库）+ 本地夹具 feed（`feed_server.py`，
> 只绑 `127.0.0.1:8911`，**每次请求比上次多一条**，用来观察后台刷新）+ `xdotool` 真实鼠标/键盘 +
> `import -window root` 截图 + 应用 stdout 的 `[ui]` 行（重建路径固定打 `view=… sort=… hideRead=… count=… head=<前 5 行 id>`）。
> 夹具库 `baseline.sqlite`（`prepare_baseline.sh` 可重建）：2 个源 + 260 条（`e0` 最旧 → `e259` 最新，`published_at` 递增），
> 奇数下标已读（130 已读 / 130 未读），另有 `e1`（已读+星标）、`e3`（已读+稍后读）；
> `refresh.interval_minutes=off`；排序档与首刷开关按场景改写。脚本：`launchA.sh` / `launchBCD.sh` / `launchE.sh`。

### 15.1 已机械验证的部分（stdout 自证 + 库回读 + 截图）

- **三档顺序**（每档都与「库里同口径 ORDER BY 直查的期望顺序」逐位对齐）：

| 档 | 库内期望头部 | 应用日志 | 截图 |
|---|---|---|---|
| newest | 260,259,258,257,256 | `view=all sort=newest hideRead=0 count=200 exhausted=false head=260,259,258,257,256` | `shotA-01-all-newest.png` |
| oldest | 1,2,3,4,5 | `view=all sort=oldest … head=1,2,3,4,5` | `shotA-02-all-oldest.png`（e0/e1/e2/e3，已读灰显、★/⚑ 标记都在） |
| unread_first | 259,257,255,253,251 | `view=all sort=unread_first … head=259,257,255,253,251` | `shotA-04-all-unread-first.png`（e199/e197/e195/e193，未读组在最前） |

- **三档续页不重不漏**：三档各自按 `G`（跳末行 → 列表滚到底 → 尾部哨兵续页）后日志都是
  `append rows=60 total=260 dup=0 exhausted=true` —— `dup` 是运行时自证（重复行会被计数）。unread_first 的复合游标
  （read 分量）因此真跑了一轮：页 1 末行落在未读组内、页 2 跨到已读组（`open id=120` 即页 1 末行，已读行）。
- **隐藏已读**：开 → `view=all sort=newest hideRead=1 count=128 exhausted=true`（260 条里已读行消失）；
  关 → `count=200`（回到整页）。星标 / 稍后读视图的**豁免**由 store 测试
  `hide_read_filters_lists_but_starred_and_later_views_are_exempt` 钉住（UI 侧不写第二套判断，豁免只在查询层）。
- **灰显防删行共存**：隐藏已读开着时点一行标读 → 日志只有 `open id=17 markRead=true read=false`，**没有 list 重建**
  （`renderList` 行不出现），列表计数仍 128 篇、该行灰显留在原位（截图 `shotA-08-hide-read-row-stays.png`）；
  紧接着切档触发 reset 重建后 `count=127`，那行才离开（`shotA-09`）。
- **后台刷新不破坏非 newest 排序**（场景 B/C/D 各一次启动，只跑启动首刷；feed 每次请求多一条）：
  - **oldest**（B）：刷新前已加载 2 页、`head=1,2,3,4,5` → `refresh:start` → `自动刷新完成: fetched=1 inserted=4` →
    `refresh:done` → `refresh:done prepend skipped（sort=oldest）：改走静默 reset，listScrollTop=15676→15676（reset 重建，无滚动锚点补偿）`
    → 刷新后 `view=all sort=oldest … count=200 exhausted=false head=1,2,3,4,5`（4 条新条目落在尾部，头部没被顶走）。
  - **unread_first**（C）：刷新后 `head=265,264,263,262,261`，与库内 `ORDER BY read, sortkey DESC, id LIMIT 8` 查出的
    `new-4,new-3,new-2,new-1,new-0,e258,…` 完全一致（新未读本就该在未读组头部）；同样记 `prepend skipped`。
  - **newest（回归对照）**（D）：仍走 prepend —— `refresh:done prepend rows=6 ids=266,265,264,263,262,261 … atTop=false
    listScrollTop=15676→16168 height=21320→21812 top=69→69 head=266`（视口顶部行 id 没动）。
- **滚动保持的实际行为**（把「降级」写准）：非 newest 档走 reset 重建，**只保留 `scrollTop` 像素偏移、不做锚点补偿**
  （实测 `15676→15676`：oldest 档新条目在尾部、视觉上不动；unread_first 档内容整体位移）。
- **勾选态与保留**（E）：菜单里当前档与开关都打勾（`shotE-02-menu-both-checked.png`：✓未读优先 + ✓隐藏已读）；
  切视图（全部 ↔ 全部未读）后每条重建日志都写着 `sort=unread_first hideRead=1`；重启（B/C/D 三档各一次）后按库内档位出列表。
- **i18n**：启动自检 `i18n selftest ok (keys=306)`（新增 5 个 key）；node 侧等价核对 zh/en 各 306 个 key 一一对应、
  `index.html` 的 149 处 `data-i18n*` 引用全部存在（缺 0）。
- **单测**：core 新增 6 例（三档顺序与设置兜底、三档×五视图续页矩阵、复合游标与半截回退、hide_read 过滤与豁免、
  unread_first 计划断言、oldest 计划断言、v10→v11 真文件迁移），`src-tauri` 新增 1 例（命令体归一 + 落库往返）；
  `cargo test --workspace` 全绿。
- **顺带修掉的真 bug**：列表头按钮打开菜单后立刻被全局「点菜单外面就关」的 click 监听关掉（同一个 click 事件的冒泡）——
  按钮 onclick 里 `stopPropagation()` 修掉（commit 见改动清单）。这条只有真点一遍才会暴露（headless 点击序列抓出来的）。

### 15.2 仍需真实桌面会话人工核验

- [ ] 真实桌面会话（X11 / Wayland 各一）下排序按钮的鼠标观感与 tooltip（GTK tooltip 是独立 X 窗口，headless 截图整块黑）
- [ ] 分数缩放（125% / 150% / 170%）下列表头「标题 + 计数 + 排序按钮」这一行的对齐与按钮尺寸（本轮 DSF=1）
- [ ] 英文界面下菜单四行文案（Newest first / Oldest first / Unread first / Hide read）与按钮 tooltip 的显示宽度
- [ ] 真实长列表（8k 条）下三档切换与 oldest / unread_first 续页的手感与冷启动代价
      （本轮库内 260 条；计划正确性由 EXPLAIN 断言 + v11 索引钉住）

## 16. RSSHub 抓取时解析（2026-09-22-rsshub-resolve，任务 85b991b4）

> headless 跑法：自建 `Xvfb :97`（1920x1200，无窗口管理器；不动 :99 上别人的实例）+
> `GDK_BACKEND=x11`（**测试进程环境，不是应用代码设置**）+ `HOME=/tmp/rsshub-resolve-verify/home` +
> `RUSTSS_DB=/tmp/rsshub-resolve-verify/data/rustrss.sqlite` + 两个本地「镜像」（`python3 -m http.server`
> 分别绑 `127.0.0.1:8931` / `8932`，`/test/1`、`/test/2`、`/legacy/1` 三份 RSS 夹具，A/B 的标题与内容不同）+
> `xdotool` 键盘/鼠标 + `import -window root` 截图 + 应用 stdout 的 `[ui]` 行 + `sqlite3` 直查临时库。
> 镜像设置用 `sqlite3` 外部改（等价于点「保存」：`feed_endpoint` 每次抓取现读设置），
> 这样绕开「headless 里点头部按钮会触发窗口拖拽」的干扰（见下）。

### 16.1 已机械验证的部分（stdout 自证 + 请求日志 + 库回读 + 截图）

- **存量官方域行在抓取时也被改写（本轮的核心纠偏）**：库里预置 `rsshub://test/1` 与
  `https://rsshub.app/legacy/1`（前者 scheme、后者存量官方域形态），镜像设 `8931` → 刷新后镜像 A 的
  access log 同时出现 `GET /test/1 200` 与 `GET /legacy/1 200`（镜像 B 零请求）——两条都跟随镜像；
  库内 url 保持原样（`SELECT id,url FROM feeds` 仍是这两种形态，刷新不写 url）。
- **换镜像零迁移**：外部把 `rsshub.mirror` 从 `8931` 改成 `8932`（应用不重启）→ 下一次刷新镜像 B 收到
  `GET /legacy/1` + `GET /test/1`（304，条件请求生效），镜像 A 的日志不再增长；再改回 A 并把 A 的夹具
  `touch` 成新 mtime → 刷新后 A 返回 200、源标题与条目标题从「镜像B-*」变回「镜像A-*」
  （`镜像A-new` / `A-new-第一条`），证明真的抓的是新镜像而不是缓存。
- **输入框直接收 `rsshub://`（N2，走真实 UI）**：设置页 `Tab` 到「添加订阅」打开输入行后
  `xdotool type "rsshub://test/2"` + `Enter` →
  `discover_feed rsshub://test/2 -> rsshub://test/2 via=Direct alternatives=0`（**不发发现请求**）→
  `add_feed id=3 url=rsshub://test/2`（库内 scheme 形态）→ 首次抓取打当前镜像 B 的 `GET /test/2 200`，
  条目 `B-new-第一条` 入库、源标题学到 `镜像B-new`。
- **「归一化 RSSHub 地址」按钮语义与文案**（截图 `20-rsshub-nav.png` / `23-normalize-click.png` / `25-crop2.png`）：
  设置 → RSSHub 面板显示新文案（实例地址 hint「RSSHub 订阅（rsshub:// 与存量 rsshub.app）抓取时改用此实例；
  留空用官方」、分组「地址整理」+ 按钮「归一化 RSSHub 地址」）；点按钮 → 确认框
  「将把 **1** 条订阅地址改写成 rsshub:// 形态，确定执行？」（预览恰好数出那一条官方域存量）→ 回车确认 →
  `rsshub normalize: migrated=1 skipped=0 errors=0`，库内该行变 `rsshub://legacy/1`，两条 scheme 行不动；
  再点一次 → 状态栏「没有需要归一化的订阅」（**幂等**，无确认框）。
- **归一化后抓取口径不变**：`rsshub://legacy/1` 仍解析到镜像 A 的 `/legacy/1`（`GET /legacy/1 304`），
  即「归一化只改存储形态、不改抓取地址」。
- **i18n**：启动自检 `i18n selftest ok (keys=306)`（`data-i18n*` 引用全部存在）；
  node 侧核对 zh/en 各 306 个 key 一一对应，改动的 5 个 key 双语都已更新。
- **单测**：core 新增/改写 8 例（canonical/resolve 两套语义、add 不实例化 + 两形态判重、
  endpoint 随镜像变化（含存量官方域行）、归一化幂等与冲突 skipped、scheme 输入零网络、
  添加流程全链、wiremock 抓取端到端（换镜像重抓）、OPML scheme 回环），
  `cargo test --workspace` 全绿（194 例）。

### 16.2 环境限制与仍需真机核验

- **headless 下点头部按钮会拖窗**：无窗口管理器时，点击 `header`（`data-tauri-drag-region`）附近的按钮会被
  当成窗口拖拽——本轮实测点击坐标被「吞掉」甚至把窗口挪走（`xdotool getwindowgeometry` 从 `0,0` 变 `-664,45`）。
  绕过办法：头部按钮改用 `Tab` + `Space`（键盘可达），或先 `xdotool windowmove <win> 0 0` 归位再点；
  对话框内部的点击（导航、按钮）正常。**这条只是 headless 自动化限制，不是应用缺陷**。
- [ ] 真实桌面会话（X11 / Wayland 各一）下：设置页 RSSHub 面板的输入 + 「保存 / 测试连接 / 归一化」按钮观感与点击
- [ ] 真实桌面会话下粘 `rsshub://path` 到添加框的完整手感（含发现阶段状态栏文案「正在发现…」→「已订阅 N 篇」）
- [ ] 断网 / 镜像 5xx 时 scheme 源的失败标记与错误文案（本轮夹具都是 200/304）

## 17. 审计修复批次一（2026-09-22-audit-remediation-1，任务 3dcfe44c / ba8c1c96 / 45d45a47 / 78fee86f / b8698b1e）

### 17.1 已机械验证的部分（单测 + 实机截图/日志 + 独立复跑）

- **T1 外链注入（8fead17）**：`validate_external_url` 单测（合法 http/https 原样通过；javascript:/file:/data: 拒绝；空白/控制字符/`"<>|^`` 拒绝；空串拒绝）；`external_open_command` 断言程序名不含 cmd/sh、URL 为独立参数；reviewer 复跑 `cargo test -p rustrss-desktop` 47 passed；literal grep 确认实现区无 `cmd`/`"/C"`。
- **T2 feed 体积闸门（d77e65d）**：Content-Length 预检与 chunked 累计两条中止路径测试（raw-socket 装置，断言正文未读完即中止）；超限后条目不写、etag/last_modified 不覆盖、失败态走既有路径；reviewer 复跑 `cargo test -p rustrss-core` 140 passed。
- **T3 quick-xml ≥0.42（8396f73）**：`cargo audit` 0 漏洞；红基线对显式 commit d77e65d 的 lock 复现 2 条（RUSTSEC-2026-0194/0195）；OPML 6/6 用例；`unescape_value→normalized_value` 行为等价经 vendored 源码逐字比对。
- **T4 前端（a060444）**：Xvfb 实机——长文滚到中部按 u/s/l 三次 `readerScrollTop` 相同、阅读区像素 AE=0、其后无 `renderReader` 重渲染；星标/稍后读视图定向移除行；坏 DB 启动后 ✕ 可关窗（对修复前代码 A/B 复现死窗）；reviewer 独立复跑两项核心场景。
- **T5 CSP（f74235d）**：Xvfb 主流程零 `securitypolicyviolation`（43 行日志 0 违规）；负向对照证明探针会报（img-src / connect-src 各一行，含 blockedURI 与来源）；含图文章真实渲染；`cargo test --workspace` 203 passed（16 目标，覆盖三个 crate），`cargo clippy --workspace` 仅 3 条既有基线警告。

### 17.2 环境限制与仍需真机核验

- **Windows 运行时未实测（T1 遗留）**：本机无 Windows/mingw，`open_external` 的 rundll32 路径只有单元测试 + `rustc --target x86_64-pc-windows-gnu --emit=metadata` 类型检查。真机核验步骤：Windows 构建启动后，打开一篇 link 含 `&` 的文章（如 `https://example.com/?a=1&b=2`）点「浏览器打开」→ 应打开完整 URL，且不得出现由 shell 解析产生的额外命令（rundll32 自身进程属预期）；同时确认标题栏三键（最小化/最大化/关闭）可用。
- **UI 修复（T4）与 CSP（T5）仅在 X11/Xvfb 验证**：Wayland 原生会话、macOS、Windows 的界面回归（滚动保持 / 按钮态 / 零 CSP 违规）未覆盖；CSP 在 Windows WebView2 下 `connect-src ipc:` 的兼容性需真机回归。
- **日志功能批次（2026-09-22-logging）**：已实现并收口，验证记录见第 18 节。

## 18. 日志功能（2026-09-22-logging，任务 7ae90b5b / 7cee65c2 / 69a5a579 / fabcc214）

### 18.1 已机械验证的部分（单测 + Xvfb 实机日志/截图 + 库回读自证）

headless 跑法：`Xvfb :99` + `GDK_BACKEND=x11`（**测试进程的环境，不是应用代码设置**）+ 隔离 `HOME=/tmp/t4-live/home`（数据目录随之隔离）+ `RUSTSS_DB` 指向真实库副本 + `setsid` 后台启动 + `xdotool` 真实点击 + `import` 截屏；改 `ui/` 后先 `cargo build -p rustrss-desktop`（Tauri 编译期嵌入 `ui/`）。

- **T1 core 基础设施（7ae90b5b，c3c8170）**：`paths::logs_dir()` = `default_data_dir()/logs`；`logging.rs` 单测覆盖文件命名（`rustrss-YYYYMMDD-HHMMSS.log` + 同秒 `-N` 后缀）、行格式（本地时间毫秒 + 级别 + target + message）、`prune` 边界（恰好 20 不删 / 21 删 1 / 超 50MB 按 mtime 从旧删 / 单文件超上限）、`init` 目录不可创建时返回 `Err` 而不是 panic。
- **T2 接入与迁移（7cee65c2，c8b2b38）**：启动最早期以默认 `info` 初始化并装 panic hook、开库后按 `log.level` 覆盖；`ui_log` 改为 `log::info!(target: "ui", "[ui] …")`；`crates/rustrss-core/tests/logging_runtime.rs` 独占进程断言：级别门（info 下 debug 不落盘 / 切 debug 后落盘）、debug 目标过滤（`h2`/`hyper`/`reqwest` 的 debug/trace 丢弃，本应用与 `ui` 保留，依赖的 info/warn/error 仍收录）、panic 写 payload + 位置且链式调用原 hook（stderr 现场不丢）。
- **T3 log.level（69a5a579，792e77b）**：白名单 `info`/`debug` + 非法回落 + 写库钳位 + 即时生效（`set_max_level`）有测试；设置页下拉 + i18n 双语。
- **T4（fabcc214）**：`open_logs_dir` 单测（启动器程序名只可能是 `xdg-open`/`open`/`explorer`、绝不含 `cmd`/`sh`、目录路径是唯一独立参数；启动器缺失时返回可读错误而不是 panic）。
- **本批收口实机复跑（2026-09-23，Xvfb :99 + 隔离 HOME + 库副本）**：
  - 每次启动各写一份新日志（连续 5 次启动 5 份），行如 `2026-09-23T01:47:05.433+08:00 INFO  rustrss_desktop: [rustrss] 本次日志文件: …`、`INFO  ui: [ui] app.js start`——`[ui]` 行只在日志文件里，stdout 无。
  - 保留策略：预置 25 个文件 / 62,914,994 字节（3×20MB 最旧 + 22 个 1KB）→ 重启后 **20 个 / 1939 字节**（两条上限同时满足），日志行 `日志保留清理: 删除 6 个（剩 20 个 / 377 字节）`，被删的正是最旧的 3 个 20MB 与最旧的 3 个小文件（`ls -lS` 复读）。
  - 关于页按钮：`xdotool` 真实点击（设置 → 关于 →「打开日志目录」）→ 日志 `[ui] open_logs_dir ok` / `[ui] open_logs_dir failed: …`；正常环境状态栏「已请系统文件管理器打开日志目录」，无启动器环境红字可读错误且进程存活（截图）。
  - 目录兜底：把 `logs/` 整个移走后点按钮 → 目录被重新创建（空目录），即 `create_dir_all` 分支生效。
  - 级别：库内 `log.level=debug` 后启动 → 出现 `DEBUG` 行（`调度: 启动首刷延迟 10s 结束…`、`刷新批次开始: 源=120 并发=12`）。
  - 降级（初始化失败不阻断启动）：`chmod 500 logs/` → 启动仅一行 stderr `[rustrss] 日志初始化失败（本次不写日志文件，应用继续启动）: …（os error 13）`，界面照常渲染 120 个源 / 13197 条（截图），不新增日志文件。
  - 敏感信息：库内 `mcp.token`（32 位）明文在所有本批日志文件中 0 命中；`key=` / `token=` / `Bearer ` / `password=` 形态 0 命中；MCP 启动行只有 URL +「需 token 鉴权」。
  - `cargo test --workspace` **231 passed / 0 failed（17 个测试目标）**；`cargo clippy --workspace --all-targets` 仅 3 条既有基线警告（`rustrss-core`：`fulltext.rs:130` redundant closure、`store/mod.rs:292` 与 `:499` redundant `ok()`），无新增。

### 18.2 环境限制与仍需真机核验

- **Windows / macOS 未实测**：`explorer` / `open` 路径只有单测 + 类型检查。真机核验步骤：点「打开日志目录」→ Windows 应开资源管理器、macOS 应开 Finder 并定位到 `logs/`。
- **「启动器存在但自身失败」不报错（已知取舍）**：状态栏文案是「已**请**系统文件管理器打开」而不是「已打开」，因为 `spawn` 成功 ≠ 目录真的打开了。实测本机（隔离 HOME + Xvfb）`xdg-open` 静默 `exit 0` 且不打开任何窗口 → 应用无从区分（退出码没有信号；Windows `explorer` 成功也常返回 1，用退出码判断会误报）。只有**启动器不存在**（裸容器 / 未装 xdg-utils）才走可读错误路径——与既有 `open_external` 同一口径。
- **真实桌面会话点击未覆盖**：本次是 Xvfb（无桌面会话、无 D-Bus 文件管理器），「按钮 → 命令 → 系统启动器」这一跳由真实点击 + 日志自证；「文件管理器真的打开该目录」需真机（KDE / GNOME，含 Wayland 各一次）。
- **每次点击留一个短暂 `defunct` 子进程**：`Command::spawn` 不 reap，直到应用退出（与既有 `open_external` 完全相同）；一次点击一个、无增长风险，未改。
- **聚合代码复审（2026-09-23）的 3 条非阻断 NOTE**（已知残余，后续可评估）：
  1. **依赖库的 info/warn/error 行不经 `scrub_log_line`**（debug/trace 已被 target 过滤拦下，但 info 以上不限 target）：`url` crate 的 Display 会掩密码但保留 username，极端情况下私有源 URL 的 username 可能出现在依赖库的罕见错误行里；
  2. **init 降级模式下 `[ui]` 诊断行全丢**（logger 装不上时无处可写，仅一行 stderr 提示）：headless 冒烟若靠 grep stdout 找 `[ui]` 会失掉信号，属已知取舍；
  3. **panic payload 落盘不打码**（panic 信息里若含敏感串不会被 scrub；上游 AI/UI 路径已有 core 侧打码，残余风险低）。

## 19. 右键菜单子菜单化（2026-09-23-submenu，任务 4dfbce24）

### 19.1 已机械验证的部分

- **子菜单机制**：`submenuRect` 纯函数 5 个自检用例 + 2 个对抗探针（bottom+flip、tiny-viewport）不变量成立；`placeSubmenu` 贴父项行 `getBoundingClientRect()`（非菜单容器），右→左翻转 + 垂直钳位/内部滚动，Xvfb 实测各用例 `outofview=0`；`renderMenuItems` 父子同源，非 submenu 分支与旧代码逐行等价。
- **订阅菜单两处**：刷新间隔父项显示当前档位（子菜单 6 档打勾）；移动到父项显示当前分组（子菜单含「未分组」且当前项打勾）；点击「每 15 分钟」→ `sqlite3` 回读 `feeds.id=92 = 15`，重开菜单勾选/文案同步。
- **关闭与切换**：Esc / 点击外部 / 点击子项关闭整棵树；悬停其他父项切换子菜单；父菜单滚轮滚动收起子菜单。**顺带修复既有 bug**：Esc 原本永远关不掉右键菜单（判定被前置 early-return 挡死，diff 可证）。
- **体积对照**：菜单体高度 460px → 198px（≤ 1/2）；条目 31+3 seps → 5+2 seps。
- **回归面**：`#ctx-menu` CSS 选择器零残留（根菜单保留 id，5 处调用点存活）；文件夹菜单、列表区菜单、feed-edit 下拉按代码等价论证不变；i18n zh=en=316、被删 key（`menu.moveToWithName`/`menu.moveToUngrouped`）全仓零引用。
- **门禁**：`cargo test --workspace` 231 passed / 0 failed；`cargo clippy --workspace` 无新增；菜单渲染/子菜单定位 boot 自检通过。

### 19.2 限制与遗留

- **翻转左分支在出厂几何下不可达**（244px 侧栏 + 窗口 `minWidth 900`）：该分支由 boot 自检守护，实机取证时用了一行仅测试用的 CSS 加宽（已还原、未提交、diff 验证一致）。
- **设置页下拉在 Xvfb 下无法点击实测**（既有 `data-tauri-drag-region` 无 WM 吞点击问题，与本改动无关），其回归以「anchor 路径等价」论证（列表菜单 + feed-edit 弹窗已覆盖同一代码路径）。
- **未新增键盘导航**：子菜单可达性与既有菜单口径一致（Esc 关闭；无方向键导航），如需键盘完整导航建议单列需求。

## 20. MCP 写能力批次（2026-09-23-mcp-write；T1 `c0e1a54d` / T2 `e9c08c82` / T3 `d6e20fb2` / T4 `880642d8`）

### 20.1 已机械验证的部分（单测 + 真 HTTP e2e + 真二进制实机）

**T1 读侧补齐（`c0e1a54d`）**：`list_articles` 新参（`since`/`until`/`feed_ids`/`cursor`/`page_size`/`sort`/`hide_read`）+ `list_folders` + `get_unread_summary`；默认口径固定「最新在前 + 不隐藏已读」（不继承界面设置）；core 侧 EXPLAIN 断言（含删索引变异校验）与 `unread_summary` 覆盖索引断言；e2e `tests/tools.rs`（11 条）覆盖分页不重不漏、显式参数覆盖设置、未读聚合完整分组。

**T2 写能力基础设施（`e9c08c82`，`30a476c`）**：

- `registry.rs`：一张表同时驱动 `tools/list` 过滤与调用闸门（`visible() == authorize().is_ok()`，同一函数）；顺序 scope → `write_enabled` → 写 token 存在性 → `dangerous_enabled`；错误码 `write_scope_required` / `write_disabled` / `dangerous_tool_disabled`；路由里存在但未登记的工具 fail-closed 拒绝（`tool_not_registered`）。
- 每请求现读库里的 token 现算 scope（HTTP），stdio 无凭据概念 → scope 恒 write + 同一套开关；**写 token 轮换/销毁后旧值下一个请求立刻失效**（不缓存会话 scope，`write_auth.rs` 有专测）；401 与 `/health` 口径不变。
- `write_contract.rs`：批量 ids ≤100、`confirm` 必填、`dry_run` 在结构上不落库（`apply_or_preview` 不调用落库闭包）、统一返回信封 `{ok, affected, results, error_code?, error?, dry_run?, detail?}`。
- 审计：每次写调用（含被闸门拒掉的）一行 `target=mcp`，参数摘要先 `scrub_log_line` 再截断（300 字），不含正文与凭据。
- 共享设施上提 core（纯搬迁，行为不变）：`scrub_log_line`、刷新单 flight（`RefreshGate`——MCP 与界面注入**同一个** `Arc`，lib 单测证明 agent refresh 不与界面刷新叠加）。
- 设置页 MCP 区（写开关 / 危险开关 / 写 token 生成·轮换·销毁）机械接线测试通过（`cargo test -p rustrss-desktop` 56 项含 `mcp_write_settings_*`），i18n 双语 key 集合一致。

**T3 阅读状态与刷新（`d6e20fb2`，`7b72463`）**：`set_read`/`set_starred`/`set_read_later`（`ids[]` ≤100 或条件级 `{feed_id, since, until}`，`affected` = 命中条数 → 幂等，逐项结果含 `article_not_found`）、`refresh`（三类 scope，与界面共用单 flight，抢不到 `rate_limited` 且一个请求都不发）、`fetch_fulltext`（复用 2MiB 流式闸门，已抓过零网络）；core 增量 `EntryFlag`/`EntryFlagScope` + `set_flag_scoped`/`entry_ids_scoped`/`existing_entry_ids`（EXPLAIN 断言 + 变异校验）。e2e `tests/write_tools.rs` 5 条 + 单测 11 条 + 授权矩阵 6 条。

**T4 订阅管理（`880642d8`，`c2f7023` + 小修 `a293b69`）**：

- `subscribe`：首页自动发现（core `discover`，输入本身是 feed 时原样使用）→ 落库发现出的地址；`rsshub://path`（三斜杠 / 大写 / 官方域）归一为同一身份且**不联网**；幂等（重复 URL 返回既有 id、`affected=0`、`detail.already_subscribed=true`、库不新增；已订阅的直接 feed 地址不再发发现请求——e2e 用测试服务器的命中计数断言）；`invalid_url` / `fetch_failed` 分界有断言。
- `update_feed`：tri-state（未传字段不动、`null` 移出分组 / 跟随全局、空串标题回退源站名）逐项 `sqlite` 回读断言；白名单外间隔 → `invalid_argument`；空 patch → `invalid_argument`；`feed_not_found` / `folder_not_found`。
- `folder_create` / `folder_rename` / `folder_delete`：同名建组幂等、重名/空名 `invalid_argument`、**删组不删订阅**（订阅保留且 `folder_id` 置空，侧栏折叠状态的孤儿 id 一并清理）。
- `unsubscribe`（危险）：危险开关关 → `dangerous_tool_disabled`（且 `tools/list` 不列）、缺 `confirm` → `confirm_required`、`dry_run` 返回条目数且库不变、**dry_run 与实际 `affected` 相等**（共用 `Store::entry_count_for_feed`）、执行后 `list_feeds` 不含该源且条目级联删除（`sqlite` 回读 0）。
- `import_opml` / `export_opml`：与界面同一 core 实现；导入返回 `added`/`skipped`/`errors`（+ `folders_created`/`outlines_ignored`）、二次导入全 skipped（`xmlUrl` 去重）、`path` 与 `content` 两种入参都能用、导出文本可回导；非 XML / 缺参 / 都给 → `invalid_argument`。
- 审计：`tests/feed_tools.rs` 的审计用例断言 8 个工具各有一行、dry_run 行带 `dry_run=true`、危险开关拒绝与读 token 拒绝都留痕、参数串里的 `hunter2` / `SECRET-TOKEN` 不落盘而 `token=***` 保留。
- core 增量：`Store::entry_count_for_feed` + `explain_entry_count_for_feed`，`INDEXED BY idx_entries_feed_read` 钉住覆盖索引（EXPLAIN 断言 + 删索引变异校验），COUNT 不穿正文大列所在的表 B 树。

**批次收口**：`cargo test --workspace` **321 passed / 0 failed**（19 个测试目标：core 186 / desktop 56 / mcp 79）；`cargo clippy --workspace --all-targets` 仅 3 条既有 core 警告（`fulltext.rs:130` redundant closure、`store/mod.rs:379` 与 `:598` matching on `Some` with `ok()`），无新增；durable spec「通道 B：对外 MCP server」的**写能力条目**已逐条对照证据勾选。

### 20.2 实机快照（T4；真二进制 + 真库 + 本地 HTTP 源，`HOME` 隔离在 `/tmp/t4-live`）

跑法：`rustrss-mcp --print-config` 建库并迁移（`user_version=11`）→ `sqlite3` 写入读/写 token 与两个开关（真二进制只认库里的凭据）→ `rustrss-mcp --http 127.0.0.1:18098` → curl 走 MCP JSON-RPC（`initialize` + `tools/call`）。脚本与完整 transcript 为临时产物，跑完已清理。

- 首页 `subscribe` → `affected=1`、`via=link_type`、落库 `/feed.xml`；同首页与直接 feed 地址复订 → `affected=0` + `already_subscribed=true` + 同一 id；`rsshub:///live/demo` → `RSSHUB://live/demo` → `https://rsshub.app/live/demo` 三次调用只有第一次 `affected=1`，库里只有一条 `rsshub://live/demo`。
- 读 token `list_feeds` 立刻看到新订阅；`refresh(feed_ids)` → `inserted=2`，`sqlite3` 回读 `entries=2`；`update_feed{自定义名 + 归组}` → `sqlite3` 回读 `我的实机源|1|`。
- `folder_delete` dry_run → `feeds_affected=1`（组与订阅都不动）；confirm → 组没了、订阅还在且 `folder_id` 为空。
- `unsubscribe` dry_run → `affected=2`，回读 feeds=1 / entries=2（未落库）；confirm → `affected=2`，回读 feeds=0 / entries=0（条目级联删除），`list_feeds` 清空。
- 错误码实机命中：读 token 调 `subscribe` → `write_scope_required`；缺 `confirm` → `confirm_required`；源不存在 → `feed_not_found`。
- 审计：日志文件 19 行 `mcp-write tool=…`（含 dry_run 与三条被拒），`write-tok-e2e` / `read-tok-e2e` 在日志中 0 命中。

### 20.3 环境限制与仍需核验

- **Windows / macOS 未实测**：MCP 是独立进程（不依赖 Tauri），但本批次只在 Linux 上跑过 HTTP/stdio。真机核验步骤：Windows/macOS 构建后 `rustrss-mcp --http`，按 20.2 跑一遍 subscribe → refresh → unsubscribe 闭环。
- **设置页 MCP 区未做 GUI 实机**（T2 遗留）：开关 / 写 token 生成·轮换·销毁只有机械接线测试（扫 `index.html` / `app.js` / `main.rs` + 命令单测），未在 Xvfb 下点击与截图；「关掉危险开关后列表里看不到 `unsubscribe` / `folder_delete`」在 MCP 层有测试，但设置页开关的点击路径未端到端跑过。
- **归一化是跨 crate 同语义复刻**：自定义标题与刷新间隔白名单在 `src-tauri`（界面）与 `rustrss-mcp` 各有一份实现（MCP 是独立二进制，不能跨 crate 调用）。数值与语义有测试逐条钉住（`interval_whitelist_matches_the_ui_choices`、`custom_title_normalization_matches_the_ui`），但两边仍是两份代码，改一边不会自动让另一边转红；彻底消除需要把归一化上提 `rustrss-core`（本任务范围明确禁止触碰 `src-tauri/**`，留作后续）。
- **`import_opml` 未提供 `dry_run`**：tech_design 的 `dry_run` 举例提过「import 的新增/跳过数」，但 core 的 `opml::import` 没有预览模式；为不让预览变成第二套遍历逻辑（预览与执行漂移），本批次只给两个危险工具做 dry_run。若确需，正确做法是在 core 侧重构出可预览的导入。
- **未对真实 RSSHub 实例做端到端抓取**：只验证了 scheme 归一、等价形态去重与「不联网落库」；`rsshub://` 的抓取解析（按镜像解析实际地址）在既有批次与 `refresh` 的实机里覆盖，本批次未重测。
- **界面入口未新增**：订阅 / 分组的界面路径本来就存在（侧栏菜单 / 编辑对话框 / OPML 按钮），MCP 只是同一数据层的第二个调用方；本批次未改 UI 文案，因此没有 i18n key 变化。

## 21. 标签批次（2026-09-23-tags；T1 `73a4433` / T2 `4bee3cf` / T3 `f397762` / T4 `14ac1a0` + `fc89939`）

### 21.1 已机械验证的部分（core 单测 + Xvfb 实机 + 真 HTTP e2e + 真二进制实机）

**T1 core 数据层（`73a4433`）**：迁移 v12（`tags` + `entry_tags` + `idx_entry_tags_tag` + 覆盖索引 `idx_entries_unread_id`）幂等且有旧库升级路径测试（数据不丢）；store API create / rename / set_color / set_pinned / delete(`dry_run` 与实际共用 `tag_entry_count`) / list(`list_tags` 侧栏序 + `list_tags_recent_first` 选择器序) / assign / unassign(`TagTarget::Entries` ≤100 或条件级)/ reorder；级联零孤儿（`remove_feed` 路径显式删除 + FK 双保险，`orphan_entry_tag_count` 断言）；EXPLAIN 断言 + 删索引变异校验（标签筛选走 `idx_entry_tags_tag`、未读计数走覆盖索引，不穿正文大列所在表 B 树）；`EntryQuery.tag_id` 默认口径不变。`crates/rustrss-core/tests/tags.rs` 17 条通过（T3 评审独立复跑 `cargo test --workspace` 341 passed / 0 failed）。

**T2 UI 交互（`4bee3cf`）**：阅读器 meta 行 chips（点击 = 标签视图）＋选择器（type-ahead、↑↓/Enter/Esc、Enter 新建并附加、候选在新建行之前、已附加行 ✓ 可取消、最近使用优先直接来自 core 的 `list_tags_recent_first`）；`t` 快捷键（列表态退到首行 / 阅读器态，`tagPicker:open/close reason=esc` 证明 Esc 无副作用）；列表行 ≤2 chips + `+N`（行级 patch，不重建列表/正文）；标签视图 `{kind:'tag'}`（列表头「标签：<名>」，与未读/星标/稍后读并存）；语义分工文案双语 + i18n key-set 348=348。Xvfb :107 实机 15 张截图（`/tmp/rustrss-tags-verify/EVIDENCE.md`，临时产物，跑完清理）；新增键位测试做变异校验（注入重复 `case 'u'` → FAILED，还原 → ok）；`cargo test --workspace` 339 passed / 0 failed。

**T3 UI 管理（`f397762`）**：侧栏可折叠「标签」区（`sidebar_data.tags` 直出 core `list_tags`，一次锁一次 IPC，前端不排序不数数；折叠状态 `ui.tags_collapsed` 跨会话保持）；原生拖拽排序（整份可见顺序交 `reorder_tags` 单事务，DOM 只挪被拖节点，`tagReorder: created=0 removed=0` 作不重建的证据；置顶组与普通组不混排，跨组落点直接拒绝）；右键菜单（重命名 / 颜色 8 色预设 + 默认，当前项打勾 / 置顶切换 / 删除），重名报 core 的可读错误；删除先 `dry_run` 拿「将影响 N 篇」再确认，确认后标签消失、文章仍在、行 chips 就地更新。Xvfb :109 实机 26 张截图 + 5 份日志（`/tmp/rustrss-tags-t3/EVIDENCE.md`，临时产物，跑完清理）；`tagPalette selftest` 对比度 light 3.07 / dark 3.24（≥3:1）；i18n 375=375；`cargo test --workspace` 341 passed / 0 failed。

**T4 MCP 工具（`14ac1a0`）**：

- 新增 `crates/rustrss-mcp/src/tag_tools.rs`：`list_tags`（read）+ `create_tag` / `rename_tag` / `assign_tags` / `unassign_tags` / `delete_tag`（write，全部 `dangerous=false`）；注册表登记后自动获得 `tools/list` 过滤 + 调用闸门 + `target=mcp` 审计（与既有基建同一条路径，无新机制）。
- `list_articles` 增 `tag_id` / `tag_name`（互斥 → `invalid_argument`；未知标签 → `tag_not_found`，不静默空列表）且每条带 `tags`（名称数组，≤20 个，超出置 `tags_truncated`）。
- 错误码按 `StoreError` 变体映射（不解析文案）：`tag_not_found` / `duplicate_tag_name` / `invalid_argument`；沿用 `write_scope_required` / `write_disabled` / `confirm_required`。
- 单元测试 11 条（`tag_tools` 模块内：批量闸门、清单元数据与上限、排序档、错误码映射、目标形态、条目存在性、delete 的 confirm/dry_run 同源）+ 真 HTTP e2e 8 条（`crates/rustrss-mcp/tests/tag_tools.rs`）：读 token 可见性/写 token 可用、写调用落库回读、读 token 与关写开关两种拒绝（`write_scope_required` / `write_disabled`，库不变）、`delete_tag` 缺 confirm、`dry_run` 影响数与真删相等且 dry_run 后库不变、`list_articles` 过滤与 `tags` 字段、注入界面设置后默认口径不变、审计行含 dry_run/被拒且参数过 scrub。
- 批次收口修正（core 一行条件 + 一个测试）：`last_used_at` 只由**打标**推进（`tag_link` 的 `changed > 0 && link`），取消打标不推进——PRD FR-1 / tech_design schema 注释 / `Store::create_tag` 文档 / `schema.rs` 注释四处口径一致；T2 评审 Note-2 与 T3 评审 Note-4 均点名留 T4 裁定。
- 门禁：`cargo test --workspace` **360 passed / 0 failed**（含收尾补测的条件级截断分支）；`cargo clippy --workspace --all-targets` 仅 3 条既有 rustrss-core 基线警告（`fulltext.rs:130` redundant closure、`store/mod.rs:461`/`:680` matching on `Some` with `ok()`），无新增。

### 21.2 实机快照（T4；真二进制 + 真库 + 本地 HTTP 源，`mktemp -d` 隔离 HOME）

跑法：`rustrss-mcp --print-config` 建库并迁移 → `sqlite3` 写入读/写 token 与写开关（危险开关**不开**，用于证明 tag 写工具不受它约束）→ `python3 -m http.server`（随机端口）提供 3 条 RSS → `rustrss-mcp --http 127.0.0.1:<随机端口>` → curl 走 MCP JSON-RPC。脚本与完整 transcript 为临时产物，跑完已清理。关键行：

- 读 token `tools/list` = 8 个只读工具（含 `list_tags`，不含任何写工具）；`list_tags` 返回 `count=0`（空库）；读 token 调 `create_tag` / `assign_tags` → `isError=true` + `write_scope_required`，库不变。
- 写 token `subscribe` + `refresh` → 夹具源入库 3 条（源标题 `实机标签源`）；`create_tag{"Live","#3E63DD"}` → `affected=1`、颜色归一为 `#3e63dd`、`last_used_at=null`；重名（`live`）→ `duplicate_tag_name`；`color:"blue"` → `invalid_argument`。
- `assign_tags(ids=[1,2])` → `affected=2` / `detail.changed=2`，`sqlite3` 回读 `entry_tags=2`、`tags=1|Live|#3e63dd`。
- **`last_used_at` 哨兵**：`sqlite3` 置 `last_used_at=1000` → `unassign_tags(ids=[1])` 后回读仍为 `1000`（取消不推进）；再 `assign_tags` 后变为真实时间戳（打标才推进）。
- `list_articles(tag_name=live)`（小写 → 命中 `Live`，大小写不敏感）→ 2 条且每条 `tags: ["Live"]`；`tag_id`+`tag_name` 同传 → `invalid_argument`；`tag_name="不存在"` → `tag_not_found`；`unread_only` 与 tag 过滤可叠加。
- `rename_tag` → 库回读 `1|Live 改名`；rename 不存在的 id → `tag_not_found`；`unassign_tags` 库回读 `entry_tags 2→1`。
- `delete_tag` 缺 `confirm` → `confirm_required`（库不变）；`dry_run` → `affected=2` 且回读 `tags=1 / entry_tags=2 / entries=3`（未落库）；`confirm` → `affected=2`（与预览相等，同源）、回读 `tags=0 / entry_tags=0 / entries=3`（标签消失、文章保留）；删不存在的 id → `tag_not_found`。
- 审计：日志 16 行 `mcp-write tool=…`，覆盖 5 个 tag 写工具 + dry_run 行（`dry_run=true`）+ 被拒/失败行（`write_scope_required` / `duplicate_tag_name` / `invalid_argument` / `confirm_required` / `tag_not_found`）；凭据串 `hunter2` / `SECRET-TOKEN` / 读写 token 0 命中，`token=***` 打码行保留。

### 21.3 环境限制与仍需核验

- **Windows / macOS 未实测**：MCP 是独立进程（不依赖 Tauri），本批次只在 Linux 上跑过 HTTP。真机核验步骤：构建后 `rustrss-mcp --http`，按 21.2 跑一遍 `create_tag → assign_tags → list_articles(tag) → delete_tag(dry_run/confirm)` 闭环。
- **T3 遗留（跨组拖放拒绝）**：置顶组与普通组的落点被直接拒绝（T3 实机 `0 次 reorder`、库不变）——core 的 `ORDER BY` 恒把 `pinned` 排前，跨组落库也不会生效；若将来要支持「拖成置顶/取消置顶」，需要在 core 侧定义语义（当前不在范围内）。
- **T3 遗留（Xvfb 陈旧绘制）**：原生 DnD 在 WebKitGTK + 无 WM 环境下会把拖拽终点行的 hover 底色留在画面上（DOM/类名核对无残留，判定为环境重绘怪癖）；T2 也记录过「WebKit 停在旧帧，需 resize 回弹触发重画」。真实桌面有 WM 时未复现，未在真实桌面会话复核。
- **T3 评审 Note-1（颜色子菜单点击）**：reviewer 未亲自点穿颜色子菜单（父项 + chevron、`set_tag_color` 命令测试、库存色圆点渲染已验）；本次亦未补点击（无 Xvfb 会话），留待真实桌面会话。
- **MCP 与界面并发写**：两者共用同一库文件与同一 core API（WAL + 锁），`create_tag` 的跨进程重名冲突在 core 侧有 `unique_violation` 映射测试；但**未做**「界面与 MCP 同时打标同一批条目」的并发实机压测（单写者场景，core 事务保证原子性）。
- **未对真实 RSSHub 实例做标签端到端**：标签与 RSSHub 解析无交集，本批次未重测 RSSHub 抓取（既有批次覆盖）。
- **`search_articles` 不带 tag 过滤**（聚合复审 NOTE）：按标签筛选仅提供在 `list_articles`（`tag_id`/`tag_name`）；搜索结果暂不支持按标签收窄，与提案「搜索 `tag:` 语法留 v2」口径一致。

## 22. 列表计数口径 + 只看未读入口 + 加载更多兜底（2026-09-23-list-count-and-unread-toggle）

### 22.1 T1 列表计数口径（「已加载 M / 共 N」）· 提交 `9bca8bc`

**机制证据（可复跑）**

- `cargo test -p rustrss-core --test store folder_and_tag_scope_totals_stay_on_covering_indexes`：分组/标签/源三个维度的总数与 `list_entries` 行数同口径（空分组 0、不存在 id 0、未分组源不计入任何分组、同一条多标签不重复计数）；EXPLAIN 断言要求计划里出现 `COVERING INDEX idx_entries_feed_read` 且不得裸 `SCAN entries`；随后 `DROP INDEX idx_entries_feed_read` 再断言 `explain_entry_count_for_folder` **报错（变异校验转红）**——证明断言确实钉在那条索引上。
- `cargo test -p rustrss-desktop list_scope_total`：命令三个分派分支 + 未知 kind 报「未知的统计范围」+ 返回值与列表行数一致。
- `cargo test --workspace`：全绿（core `store` 51 / desktop 64 / mcp 55 / 其余见输出）。

**实机证据**（Xvfb `:99` + `GDK_BACKEND=x11` + 真库副本 `/tmp/rustrss-verify.sqlite`，跑完已清理）

> 环境备注：只设 `DISPLAY` 不够——GDK 在 `WAYLAND_DISPLAY` 缺失时会回落到 `wayland-0`，应用会连到用户真实会话而 Xvfb 里看不到窗口；harness 需显式 `GDK_BACKEND=x11`（**只加在测试环境，未写进应用代码**，不触碰硬约束 3）。另复现了清单 21.3 记录的「无 WM 下 WebKit 陈旧绘制」：侧栏出现过一块未重绘的黑色区域（DOM/日志核对无异常）。

- 未读视图：`view=unread sort=newest hideRead=0 count=200 loaded=200 total=13372 totalKind=unread header=已加载 200 / 共 13372 未读`；同刻侧栏「全部未读」= 13372、左下状态栏全库 = 13476 → 头部 N 与侧栏同口径一致。
- 全部视图 + 单源（Phoronix，feed#90）：`view total feed#90 n=57 header=已加载 57 / 共 57 篇`；`sqlite3` 独立核对 `90|Phoronix|total=57|unread=57`。
- 全部视图 + 单标签（临时标签「验证标签」tag#1：30 条打标 / 25 未读）：`view total tag#1 n=30`，头部 `已加载 30 / 共 30 篇`。
- **有效筛选口径（T1 AC7）**：`list.hide_read=true` 后点开 AI情报局（库内 `total=176 / unread=134`）→ `view=feed#92 sort=newest hideRead=1 count=134 loaded=134 total=134 totalKind=unread header=已加载 134 / 共 134 未读`，且该次会话里 `list_scope_total feed#92` 调用 **0 次**——N 取的是 134（未读）而不是 176（全源），且未读口径零额外查询。
- 搜索视图：`view=search count=37`，头部 `已加载 37 篇`（不查 FTS 总数）。
- **耗时（红线 #3，如实标注）**：`[rustrss] list_scope_total feed#90: 0ms n=57`（进程内首次调用）、`feed#92: 0ms n=176`（同进程第二次）；另用独立 Python/sqlite3 进程跑同形 SQL：`folder#1 0.63ms（3347 行）` / `feed#92 0.10ms（175 行）`。**三处均为 OS 页缓存未清的热缓存口径，已标注**；SQL 本身是 `INDEXED BY` 覆盖索引 COUNT（计划由 EXPLAIN 断言钉住），真冷盘口径需清 OS 缓存（需 root），本批未做、不冒充热缓存为冷启动。
- 清理：临时库副本 / 截图 / harness 日志已删；`pkill`（应用 / Xvfb / `xeyes`）无残留。

**尚未覆盖（留给后续任务或人工）**

- T2（常驻「只看未读」开关 + 快捷键 `U`）与 T3（哨兵手动「加载更多」+ 末尾终止态）尚未实现，其 AC 在各自任务里单独取证。
- 界面当前**没有单分组视图**（侧栏分组只做折叠/展开）：「共 N」的分组维度已在 core/命令层备好并测试，但界面上暂时没有入口可点——真实入口出现时复用 `list_scope_total { kind: "folder" }` 即可。

**任务评审备注的处理（评论 `de35fc24`，判定 PASS WITH NOTES）**

- Note-1（单分组视图无入口）/ Note-2（真冷盘未测，红线 #3 允许「明确标注热缓存」分支）：评审裁定非阻塞，维持现状并在上面「尚未覆盖」里记明。
- Note-3（PRD 说改第 70 条但实际未动）：已补——spec.md 第 70 条尾部加上指向第 71 / 72 条的指针，与「结构重组、语义无损」的裁定一致。
- Note-4（评审称「仓库无 invoke 漂移守卫」）：**该 note 本身不准确**。守卫确实存在：文档在 `src-tauri/src/main.rs:403`、提取器 `invoked_commands()` 在 `main.rs:552`、断言 `missing.is_empty()` 在 `main.rs:446-452`。做了一次变异校验：向 `ui/app.js` 里临时插入 `invoke('definitely_not_registered')` → `cargo test -p rustrss-desktop mcp_write_settings_controls_and_commands_are_wired` 在 `main.rs:450` panic（守卫真的会拦）；还原后该测试通过。

### 22.2 T2 常驻「只看未读」开关 + 快捷键 `U` · 提交 `b323dd8`

**机制证据**：`cargo test --workspace` 全绿（store 侧豁免语义的既有测试保持绿——本批没动 core 查询）；启动日志 `shortcut selftest ok (keys=j ArrowDown k ArrowUp Enter u s l t U r g G ; 输入区/覆盖层里一律不触发)`（`U` 已登记、无重复键位、`t` 未丢）、`i18n selftest ok (keys=383)`。

**实机证据**（Xvfb `:99` + `GDK_BACKEND=x11` + 真库副本，跑完已清理）

- 常驻开关渲染：列表头右侧出现「只看未读」；未开启时灰框，开启后 `aria-pressed=true` → 强调色文字 + 蓝边框（截图逐项核对）。
- 点击切换：`list settings: sort=newest hideRead=1` → 立即重查重渲（`view=unread ... hideRead=1 count=200 loaded=200 header=已加载 200 / 共 13386 未读`），库内 `settings.list.hide_read` 同步为 `true`；再点一次回到 `hideRead=0` / 库内 `false`。
- 快捷键 `U`：`xdotool key U` → `list settings: sort=newest hideRead=1`（与点击走同一动作、同一日志口径）。
- 与排序菜单同源：开启状态下打开排序菜单 →「隐藏已读」项带勾选 ✓（截图核对）；两处读的是同一个 `list.hide_read`，无第二份状态。
- 重启保留：库内 `hide_read=true` 时重启 → 启动日志 `view=unread ... hideRead=1`，且按钮为按下态（截图：按钮带强调色边框 + 头部写「共 13386 未读」）。
- 未回归：星标 / 稍后读视图豁免「隐藏已读」、搜索不豁免（store 层既有测试覆盖，本批未改查询路径）。

**环境备注（下次省时间）**：Xvfb 下窗口须先 `xdotool windowraise` + `windowfocus` 才接收指针事件（本次首轮点击全无效即此因，键盘事件不受影响）；列表头按钮的实际屏幕坐标与目测估计差约 90px——先截图量位置再点。

**任务评审备注的处理（T2，评论判定 PASS WITH NOTES，0 阻塞）**

- Note-1（tech_design 写「排序按钮左侧」，实现放在右侧）：以实现为准修文档——tech_design §2 改为「排序按钮之后（最右）」，并重新镜像 Document（`9902402e`，版本 +1）。
- Note-2（`b323dd8` 夹带一处无关格式改动：`renderListCount()` 两行并一行）：已还原换行，diff 里不再有无关格式改动。
- Note-3（README 键盘导航总表未列 `U`）：已把 `U`（只看未读开关）补进总表那一行。

### 22.3 T3 哨兵行手动兜底 + 续页失败可重试 + 末尾终止态 · 提交 `7835f1a`

**机制证据**：`cargo test --workspace` 全绿（200/批、keyset 游标、不重不漏、后台刷新 prepend 的既有测试保持绿——本批未动这些路径）；启动日志 `i18n selftest ok (keys=387)`。

**实机证据**（Xvfb `:99` + 真库副本）

- **空闲进度按钮（AC1）**：续页 append 后日志 `append rows=200 fresh=200 total=400 dup=0 exhausted=false sentinel="加载更多（已加载 400 / 共 13389）"`——按钮文案与 M / N 都进了日志。说明：该态是瞬态（哨兵进入视口 600px 内即自动续批），稳态截图难抓，故以日志钉住（`append` 行新增 `sentinel="…"` 字段）。
- **末尾终止态（AC4）**：Phoronix 源视图 57 条一次加载完 → 截图底部显示「已到末尾（共 57 篇）」。
- **失败 → 手动重试（AC3，真实失败注入）**：
  1. 注入：对副本库执行 `ALTER TABLE entries RENAME TO entries_hidden`（等价于后端读真失败）；
  2. 滚到哨兵触发续页 → `loadMore failed view=unread: 数据库错误: no such table: entries`；
  3. 静置 6s 后再统计：`grep -c "loadMore failed"` 仍为 **1** → 自动触发在失败态确实被拦住（没有重试风暴）；
  4. 恢复表名 → 点击尾部按钮 → `loadMore manual retry view=unread` + `append rows=200 fresh=200 total=400 dup=0 exhausted=false` → 续页成功、按钮回到进度态。
- **未回归（AC5）**：分页/游标/append 相关 core 测试全绿；后台刷新 prepend 与静默语义未改本批代码。

**未覆盖 / 说明**

- 「请求在飞 → 加载中…且禁用」这一态在本机是毫秒级（本地 SQLite），未逐帧截图；由代码路径（`setSentinelLoading(true)` → `disabled` + 文案）与四种形态的文案 harness（node 打印：空闲 / 无总数 / 失败 / 末尾，中英各一遍）核对。
- 失败注入用 `ALTER TABLE … RENAME`（真实命令失败），不是断网；两者都走 `loadMore` 的同一 catch 路径。
- 本轮遇到的两个环境坑（已可用于下次）：WAL 库下外部 `BEGIN EXCLUSIVE` 拦不住读者（要注入读失败得改 DDL/表名）；Xvfb 里窗口需 `windowraise`+`windowfocus` 才收指针事件。

**任务评审备注的处理（T3，评论 `07b38be6`，判定 FAIL）**

- **Blocker-1（AC-8 声称已勾选第 73 条、实际仍为 `- [ ]`）**：属实。我在 T3 的两个提交里只改了第 73 条的**文本**（加「T3 交付 …」说明）而没有真正把复选框改成 `[x]`，devEvidence 却写了「勾选第 73 条」——验收证据失实。已修正：`spec.md` 第 73 条改为 `- [x]`（提交 `d93d308`）。教训已记：交付前先 `grep -n "^- \[ \]" .chorus/specs/rss-reader/spec.md` 自查，再写 devEvidence。
- **Note-1（在飞态无逐帧截图）/ Note-2（失败注入用 DDL 改表名）**：评审裁定不阻塞；对应说明保留在本节。
- **Note-3（`loadMore` 上方旧 2 行注释未删、新旧重复）**：已删除旧注释，只留与当前实现一致的一份（提交 `d93d308`）。

### 22.4 聚合复核 B1 修复（任务 `bcb9f341`，idea 评论 `705e9600` 判 FAIL）· 提交 `b3e2ae2`

**修复内容**

- **B1（阻塞，跨任务缝隙）**：`refreshCounts()` 末尾补 `renderListCount()` + 尾部文案刷新（`refreshSentinelFooter()`）——精度修正（聚合复核 round 2 的 Note-3）：头部是**同值短路**（未变零写入），尾部三形态是**幂等写**（未做旧值比对；每次计数刷新一次短字符串赋值、不在渲染热路径上，已作为已知小项记进 Follow-ups）——此前 `state.db` 更新后没有人重渲 `#list-count`，会话内标读时侧栏未读已变、头部「共 N」与尾部进度仍是旧值，直到下次列表重建才追上。
- **NOTE-1（缓存 key 未校验）**：`viewTotalSync()` 只在 `viewTotalCache.key === viewTotalKey()` 时返回缓存值，否则返回 null（避免拿到其他视图的陈旧总数）。
- **NOTE-3（搜索缺终止行）**：搜索视图也画终止行，文案用既有 `list.loadedOnly`（「已加载 M 篇」），不声称 FTS 总数。

**实机证据**（Xvfb `:99` + 真库副本；Xvfb 下窗口先 `windowraise` + `windowfocus`）

- **B1**：标读前头部「已加载 200 / 共 13371 未读」与侧栏「全部未读 13371」一致；点开一篇（日志 `open id=13866 markRead=true read=false`）→ 出现 `renderSidebar feeds=120`、**0 条 `view=`**（列表未重建）→ 截图同刻头部「已加载 199 / 共 13370 未读」+ 侧栏「13370」——口径一致，滞后消失。
- **NOTE-3**：搜索 `Ubuntu` → `view=search count=42 exhausted=true`；滚到底部终止行显示「已加载 42 篇」（截图），与头部计数一致且不声称总数。
- **回归**：`cargo test --workspace` 全绿；分页三态、只看未读开关、末尾终止态行为未变。

**踩坑记录（给下次）**：Tauri 把 `ui/` 编译期嵌入，改完前端必须先 `cargo build -p rustrss-desktop` 再启动验证（仓库红线 #10）——本次首轮验证误跑了旧二进制，看到的仍是修复前行为，重建后复测才对。另：截图核对数字时头部/侧栏要同屏裁一张图（分开裁会看不到"同刻一致性"）。

**任务复核残留的修复（任务复核评论 `6531779c` Note-3，PASS WITH NOTES）**

- 残留（与 B1 同类）：终止行（`exhausted` / 搜索）是静态文本，而 `refreshCounts` 原来调用的 `setSentinelLoading` 在没有按钮时提前返回——标读后终止行的 N 会滞后到下次列表重建。
- 修：抽出 `sentinelTerminalText()`（终止行文案）与 `refreshSentinelFooter()`（按钮态含在飞禁用 + 终止行文本，统一重算），`refreshCounts()` 与 `loadMore()` 都改调它，删掉只服务按钮的 `setSentinelLoading`。
- 实机证据：feed#93 + 只看未读（47 条未读、`exhausted=true`、终止行「已到末尾（共 47 篇）」）→ 点开一篇（日志 `open id=9489 markRead=true read=false`）→ 出现 `renderSidebar`、**0 条 `view=`**（未重建）→ 同位置截图终止行变为「已到末尾（共 46 篇）」。提交 `a332afe`。


## 23. 遗留问题逐项修复（2026-09-23）

### 23.1 当前视图批量标读作用域

- 复现：`node scripts/tests/mark-view.test.cjs` 原代码 7 项失败，缺失显式 scope；修改后补充分组共 8 项通过。
- core：`mark_view_is_scoped_and_not_limited_to_loaded_page` 验证 230 条跨页的 feed/tag/starred/later/folder/FTS/单字中文搜索范围，范围外不变、幂等、空分组/空搜索/未知 ID、隐藏已读与星标豁免；`mark_view_rejects_missing_or_unknown_scope` 验证 IPC 缺参/未知 kind 不退化为全库。
- 批量 SQL 与列表/搜索复用过滤函数，单语句先物化目标 ID，再更新；避免 FTS 触发器与 read 变化影响本次目标集合。
- 「全部未读」遵循有效筛选，不是撤销；未读视图中执行没有变更，需切到全部并关闭只看未读后处理已读文章。
- 实机：重建 desktop 后在独立 Xvfb + 12000 篇样本库点击星标视图 → Settings/Reading → Mark this view read；日志 `mark_all_read scope=starred changed=100`，SQLite 断言 `(sum(read AND starred), sum(read AND NOT starred)) = (100, 0)`。


### 23.2 计数/异步状态、分组、凭据锁

- `node scripts/tests/list-state.test.cjs`：6 项通过；先红后绿覆盖同值零写入、总数乱序覆盖、跨视图旧分页游标、分组参数/未读口径、正文请求乱序和移源后分组刷新。
- core 分组测试：230 条、三档游标无重复无遗漏、空分组为空；EXPLAIN 不临时排序；删排序/标签索引后同源 EXPLAIN 断言失败。
- desktop：模拟凭据读取时 `try_lock` 必须成功，设置页与 AI 客户端各一项；凭据不可用只更新设置页状态。保存凭据已移到 store 闭包之外。
- 分组实机：点击名称日志 `view=folder` 与 `view total folder#1 n=6000`，箭头单独折叠。截图/运行日志在 `target/verification-followups/`；可用 `python3 scripts/verify-followups-ui.py` 对隔离夹具复跑，脚本自动清理所属进程。
- 构建/回归：`cargo test --workspace` 372 passed，Node 14 passed；`cargo clippy --workspace --all-targets` 仅原有 3 条 core 告警；`cargo build -p rustrss-desktop` 成功。新增脚本仅用于验证，无前端构建链。
- 环境：首次 Xvfb 启动继承 Wayland 会话，修正为独立 XDG_RUNTIME_DIR 后验证；无产品代码强制显示后端。真实 Wayland/Windows/macOS 的本批交互未复测。

### 23.3 冷页缓存计数

- NVMe 文件系统上的合成夹具：120 源、12000 篇、470286336 bytes；正文每篇约 12KB，另有 HTML 和分词内容。夹具与真实库隔离。
- 每项三次新进程；fsync + POSIX_FADV_DONTNEED 仅作用于夹具文件，mincore 每轮确认 `resident_before=0 / 114816 pages`。未清全局缓存，不声称存储控制器也冷。
- 首次 SQL：feed(100) 0.244–0.382ms；folder(6000) 3.350–5.606ms；tag(3000) 0.920–2.817ms。打开库另计 1.07–3.66ms；热态 100 次均值分别约 0.019/0.519/0.242ms。
- 复现命令见 `scripts/measure-scope-counts.py`；原始数据见 `2026-09-23-followups-counts.json`。这是 core 计数，不能等同 IPC/渲染或真实用户库耗时。
- 实机另观察一次分组 `list_entries: 138ms`，这是列表查询路径，需要进一步分层定位；未把计数达标写成列表性能达标。

### 23.4 系统侧阻挡

- installed/candidate 均 `kwallet6 6.24.0-0ubuntu1`；`apt-get --simulate --only-upgrade install kwallet6` 报 0 upgraded。
- 尝试刷新 APT 索引被 `sudo: interactive authentication is required` 阻挡，未升级任何系统包。本地索引可能陈旧；需管理员刷新索引后确认发行版的修复包或回移补丁，再安装并重新登录验证。应用侧保留 3 次有限重试，不把概率缓解当作系统根治。

## 24. 完整主题与 MCP 预览设计验证（2026-09-23）

- 已验证：独立 GTK WebView + WebKitGTK 2.52.6 原生内容截图；三种配色循环两轮，6/6 标记像素匹配；同主题两次 PNG 哈希相同。最终 PNG 2560×1800、489262–506432 bytes，应用配置至探针保存/检查耗时 242.22–305.39ms（包含写盘/图片检查，不是生产延时承诺）。
- 证据：[可行性报告与复现命令](2026-09-23-theme-preview/capture-feasibility.md)、[结果 JSON](2026-09-23-theme-preview/snapshot-results.json)、[独立交互稿](2026-09-23-theme-preview/mockup.html)。
- 本次仅增加设计文档与开发探针，未修改产品主题行为，不重复运行全量产品测试；探针在隔离 Xvfb 下运行，不访问用户数据库，不安装系统包。
- 未验证：Tauri 集成、MCP 图片响应、真实组件复用、最小化/隐藏、Wayland、客户端图片显示；Windows/macOS 当前主机无法运行验证。详见可行性报告后续步骤，保持对应验收未勾选。

### 24.1 T1 Tauri 隔离 example 后续进展

- Linux Tauri 2.11.6 原生截图适配器已接入 opt-in `theme_snapshot` example，未接入正式应用/MCP。构建和运行不访问用户数据库；只新增与现有版本匹配的可选 GTK/Cairo 依赖。
- 100%/200% 各 100 帧，唯一 revision 标记像素与正文背景逐张匹配，尺寸/比例/字节上限核验通过；8 类边界检查通过。详见 [专项报告](2026-09-23-theme-preview/tauri-capture-spike.md) 与 [结果](2026-09-23-theme-preview/tauri-snapshot-results.json)。
- 发现并处理 hide/show 后 GTK ICONIFIED 延迟事件：恢复条件使用有上限的成功原生帧握手；捕获前后均检查窗口状态。代码不强制显示后端。
- example 构建及默认 `cargo build --workspace` 成功；未运行全量测试。本次专项证据不覆盖真实 WM 最小化、Wayland、分数缩放、Windows/macOS、延迟资源或生产主题/MCP 闭环。

### 24.2 当前 KDE 原生 Wayland 补验

- 当前会话 socket 可访问；runner 保留真实 runtime，不覆盖 GDK_BACKEND/GL/scale，只在子进程移除 DISPLAY；运行时确认 GdkWaylandDisplay。
- 100/100 帧版本标记与背景像素一致，8 类边界检查通过；WebView 1000×653 CSS px、DPR=2、PNG 2000×1306，尺寸与前端 viewport 独立核对；截图中位数 45.18ms，最大 60.48ms。
- 修复实际发现的尺寸误报：旧窗口口径 1052×752 包含非 WebView 区域；现按 WebView allocation/scale 校验并返回内容尺寸。
- 修复后 Xvfb/GdkX11Display 100% 回归同样通过 100 帧和 8 类边界检查，产物 `/tmp/rustrss-tauri-snapshot-xvfb-h59cxxep`。
- [Wayland 结果](2026-09-23-theme-preview/wayland-snapshot-results.json) 与 [报告/复现方法](2026-09-23-theme-preview/tauri-capture-spike.md)。不据此宣称 GNOME、跨屏/全部分数缩放、真实最小化、Windows/macOS 或正式应用 MCP 均已验收。

### 24.3 T2 core 主题模型与存储

- Clear/Paper/Slate 各有明暗配色；参数覆盖颜色、字体、列表、阅读区与栏宽。旧设置仅在新记录不存在时无写映射，保留旧正文 13px 合法值；实际界面接入留到 T3。
- 新增 13 项主题专项测试通过；`cargo test --workspace` 共 385 passed、0 failed。覆盖参数拒绝、低对比提示、坏记录保护、跨连接版本竞争、最多 10 份历史与恢复、SQL 失败整体回滚。
- 同值零写入：修复 JSON 的 20/20.0 表示差异导致额外版本；阻断 INSERT 的触发器下同值请求仍成功，有效修改失败且历史/当前值不变。
- `cargo clippy -p rustrss-core --all-targets` 完成，仅原有 3 条 core 告警；新增文件 rustfmt 检查通过。运行日志位于 `/tmp/rustrss-theme-t2-workspace-tests.log` 与 `/tmp/rustrss-theme-t2-clippy.log`（临时文件，不作为持久产物）。
- [core 模型说明与复现命令](2026-09-23-theme-preview/core-theme-model.md)。本轮不操作真实用户库；未接入 UI/MCP、未验证主题渲染效果或端到端图片闭环。


### 24.4 T3 真实 UI 与共享渲染

- 现有主题/字体入口改用同一 core 配置；外观增加 Clear/Paper/Slate 预设，显式重置覆盖；正文范围扩至 13–28px / 1.3–2.2。保存后主题无需重建文章/列表，颜色修改只更新变量，字体/宽度变更保留可见段落锚点。
- Rust 工作区 386 passed、0 failed；Node 22 passed。新增覆盖旧控件与版本配置一致性、同值零 DOM 写、颜色变量/系统模式分支、字体转义、滑块预览回退、段落锚点、迟到字体回调、乱序配置响应。clippy 仅原有 3 条 core 告警。
- 重新 `cargo build -p rustrss-desktop` 后跑正式应用隔离夹具：UI 选择 Paper、调大正文字号，SQLite 验证 revision 1→2；重启配置未被重写，Paper 背景像素一致。原始日志与截图路径记录在 [结果摘要](2026-09-23-theme-preview/theme-ui-results.json)。
- opt-in `theme_ui` example 复用生产 HTML/CSS/i18n、列表/侧栏/正文模板与主题应用器；Xvfb 100%/200% 与当前 KDE 原生 Wayland 各 39 张截图、23 项行为检查、12 项主题背景像素 + 12 项设置弹层遮挡像素检查通过。含中英文、代码/diff、长文、空阅读区、错误源、缺失字体回退、900px 窄窗；缺字形的实际字体不作推断。
- 顺手修复实测滚动条穿透弹层：原生 overlay scrollbar 的灰色像素使遮挡断言失败，改主题滚动条后通过。DOM 节点身份、同值零 MutationObserver 记录、字号/宽度改动锚点误差 <2px 在 fixture 中检查。
- 当前仅缩略图样式开关：真实条目列表尚无缩略图元数据，不额外逐篇取正文。设置重组/历史恢复界面、MCP 配置与图片闭环、外部进程同步留到后续任务。
- 未验证：Windows/macOS/GNOME、真实系统明暗切换、跨屏分数缩放、逐字形回退、主题变化期间的远程图片重排。详见 [T3 报告与复现](2026-09-23-theme-preview/shared-theme-renderer.md)。

- T3 脚本补验：两个 Xvfb 子进程剔除 GDK_BACKEND/WAYLAND_DISPLAY/EGL_PLATFORM；显式继承冲突值复跑仍通过（39/23/24、Paper revision=2、重启保持）。滑块字号断言为 >18 且持久化，不固定为 24；本轮读回 25。命令及日志路径见 T3 报告/结果摘要。

### 24.5 T5 MCP 主题配置与同步（2026-09-23）

- [x] 读取/预设/校验/保存/恢复五工具，core 共用校验和 CAS；read/write 权限、写开关、凭据轮换、同值零写、审计脱敏专项测试。
- [x] 重建真桌面：隐藏窗口接收内嵌事件，独立 stdio 写同库由前台轮询接收；恢复历史，Slate/Paper 两次像素断言，文章渲染次数不增加。
- [x] 新 probe 继承 GDK_BACKEND=wayland / 无效 WAYLAND_DISPLAY / EGL_PLATFORM=wayland 后仍通过私有 Xvfb 隔离。
- [ ] T5 原生 Wayland、Windows/macOS 运行验收、冷盘与长期资源测量。
- [ ] T6 MCP 图片响应、临时预览生命周期及真实客户端截图闭环。

证据、接口与复跑方法：[T5 报告](2026-09-23-theme-preview/mcp-theme-config.md)。测试截图不代表已实现 MCP 截图接口。

### 24.6 T6 Linux MCP 预览闭环（2026-09-23）

- [x] preview/capture/finish 三工具，11 read + 23 write；写权限/所有者/同 profile 校验、临时不落库、CAS 保存、取消与幂等完成。
- [x] ready 元组与原生像素标记校验；失败/超时返回错误，native 取消和迟到结果丢弃；6MP/2MiB PNG/3MiB 桥接限额。
- [x] 重建产物：Xvfb 100%/200% 各 25 张 PNG；前者 1280×900/1×，后者 1280×900/2×；临时零写、最终 revision=1、正文渲染次数 1→1、连续关闭重建、重拍、本地取消及撤权回收。
- [x] 当前 KDE 原生 Wayland 24 张 PNG，实际 GdkWaylandDisplay，960×640 内容区/2×；HTTP/stdio 收图、保存/取消、重拍与撤权回收通过。
- [x] 405 Rust / 25 Node 通过；397/397 i18n；Clippy 仅原有 3 告警。http.rs 四条安全测试保持通过。
- [ ] Windows/macOS 适配器、GNOME/其它 compositor、跨屏与真实最小化。
- [ ] Wayland 本地按钮点击、正文阅读位置；30 分钟墙钟长测、冷盘和长期资源占用。
- [ ] 第三方 GUI MCP 客户端的图片显示与消息体上限；T7 聚合验收。

报告：[T6 交付说明](2026-09-23-theme-preview/mcp-theme-preview.md)；机器证据包含二进制 SHA256、每图尺寸/字节/后端：[结果](2026-09-23-theme-preview/mcp-theme-preview-results.json)。截图为固定本地 fixture，不包含用户订阅。

### 24.7 T4 设置重组 / 外观 / Aa（2026-09-23）

- [x] Rust 405、Node 33；Clippy 仅原有 3 告警；双语 468/468。
- [x] 重建真实 UI 后 Xvfb 单档：七类原生键盘导航与焦点回环，Aa 草稿零写、23px 保存 revision=1、重启保持、正文渲染 1→1。
- [x] 生产编辑器 + core 隔离探针 12 checks / 3 captures：局部预览、明暗独立预设、CAS 冲突、历史恢复、Aa/阅读同步、段落偏移 <2px、窄窗。
- [ ] T7：原生 Wayland 本地取消真实点击 + 正文段落位置；30 分钟长测与内存/磁盘趋势；完整主题矩阵/跨平台/真实客户端图片反馈。等待用户指示，不自动执行。

详细步骤、截图、磁盘 clean 前后记录与限制见 [T4 报告](2026-09-23-theme-preview/t4-settings.md)。


### 24.8 T7 跨阶段执行（2026-09-24；不是独立评审）

- [x] T4→T6 旧ID/缺资源/错误CSS token回归：先复现，修复、重建后测试；Node36，i18n469；Clippy仅原3警告。
- [x] Xvfb100%/200%完整主题×模式×场景×双语72组合；当前KDE Wayland200%另36组合，加布局9张，共117图；每档23行为/24像素断言。
- [x] 真桌面MCP Xvfb100% 25图：HTTP/stdio、临时零写、幂等保存、取消、失权回收、正文1→1。
- [ ] Open：原生Wayland真实点击取消+正文位置；用户不在场，停止授权重试。辅助DOM测量单独记录，不当成通过。
- [ ] 第三方GUI客户端、其它平台/空状态专项/原生弹层/读屏器/分数缩放；独立评审由用户安排。

范围、数据来源、现场单跑脚本与前置条件：[T7执行报告](2026-09-23-theme-preview/t7-aggregate.md)。

- [x] T7 墙钟：600s空闲与1800s绝对上限实际回收599.75s/1800.52s；RSS/profile增长与磁盘记录见报告，未作长期无泄漏承诺。


### 24.9 T7 独立评审 P2 收尾（2026-09-24）

- [x] 新 ID 扫描在清理前准确报出 app.js 八个旧 ID；清理后通过。涵盖全 ui JS（含 vendor），允许模板/运行时创建，不按动态 ID 前缀放行。
- [x] 变异校验：逐文件注入旧 ID、十种引用样本、去掉动态创建路径、data-id 冒充 ID；新增未服务 JS/CSS、移除既有路由、失效入口替换均被拒。
- [x] Rust405/0（26段）、Node40/0、Clippy仅原3警告；重建后单档Xvfb桌面probe验证七类/焦点/Aa草稿零写/23px保存/重启/正文1→1。
- [ ] 原生Wayland输入等原有open item未在本轮复跑；不扩大P2范围。

删改依据、静态检查局限与机器结果见 [P2报告](2026-09-23-theme-preview/p2-contract-cleanup.md)。用户已告知独立评审VERDICT通过；本次不自任独立评审。


### 24.10 原生 Wayland 用户在场补验（2026-09-24）

- [x] 基线04960a3，GdkWaylandDisplay/DPR2；保存24→22px及用户本地取消后，段落偏移0.09375px不变，文章/行/段落节点保留。
- [x] 记录器就绪后用户真实点击，isTrusted=true；预览销毁、正式配置未变、取消后保存被拒。关闭24.8/24.9中的该历史open item。
- [ ] 自动portal指针分支、其它平台等本轮未验；未请求授权弹窗。

[机器证据](2026-09-23-theme-preview/t7-wayland-input-results.json)。本轮隔离实例退出、临时产物已清理。

### 24.11 Codex CLI 客户端补验（2026-09-24）

- [x] 用户启动实际客户端；模型描述两版图片差异；临时不落库、保存 revision1、取消不改正式配置，SQLite 直接回读。
- [x] 修复三工具 patch 的无约束 schema，HTTP 工具目录回归先红后绿；Rust406/Node40通过，Clippy仅原有3告警。
- [x] 修复后新 Codex 会话刷新目录：patch 为对象结构，validate/preview 对象参数首次成功；取消后 SQLite 配置不变。
- [ ] 第三方 GUI 图片显示、重拍模型解释异常的归因。终端图片标记不算 GUI 验收。

证据及限制见 [客户端报告](2026-09-23-theme-preview/t7-codex-client.md)。

同会话补验：对象 patch 两次首次成功；两次 preview、revision2 article/settings capture 均被模型读到完整画面；revision1 capture 仍只报告色条。原始两组文章图仅64×8标记不同，正文像素完全一致。取消后 SQLite 配置不变；随后新会话 schema 目录转换已测通过，详见报告最新小节。

### 24.12 MCP 本地截图文件输出（2026-09-24）

- [x] 无内联图，返回路径/过期时间；工具提示本地读图，共享文件系统能力声明。
- [x] 私有权限、独立文件、容量/到期边界、失权回收、保存取消可读、桥接文件、shutdown清理与禁止晚写的自动测试。
- [x] Rust413/Node40；clippy仅原有3告警；重建后原生Wayland一档24图从文件读回。
- [x] 文件10分钟墙钟后来由24.13的脚本补验通过。
- [ ] 其它平台/容器路径可达性仍未验收。

见 [文件输出报告](2026-09-23-theme-preview/mcp-preview-files.md)。

- [x] 原生Wayland：实际文件权限/PNG尺寸/取消保留/SQLite无写入；正常exit_app后截图目录消失。
- [ ] 实际客户端连续近似图视觉稳定性：文件路径读图仍有第二图色条观测；非交互Codex另被其MCP审批策略阻断，不计完整自动闭环通过。

### 24.13 无模型文件协议与 UI 体验补验（2026-09-24）

- [x] 文件协议8项检查：真实600秒等待，605.01秒三文件后台回收，等待期间不发请求；PNG/权限/保存取消/SQLite断言通过。
- [x] 字体异步建议与失败重试修复，两条复现测试先红后绿；Rust413/Node42，clippy仅原有3告警。
- [x] 重新嵌入资源后 WebKit 设置夹具23项/3图；Xvfb真实键盘七分类、焦点循环、Aa草稿/保存/重启与节点保持通过。
- [x] 双语空阅读区、模拟系统偏好事件与 CSS zoom125%/150% 的限定范围检查通过。
- [ ] 原生系统明暗设置切换、Wayland原生分数缩放、其余空/错误业务状态、跨平台与模型视觉稳定性未在本轮验证。

[报告与可复现命令](2026-09-23-theme-preview/file-probe-ui-followup.md)。UI临时目录已清理，最后一批目录/日志因守卫拒绝而暂留；不动历史目录与Cargo缓存。

### 24.14 原生 Wayland 缩放与系统偏好（2026-09-24）

- [x] KScreen确认125%/150%，KWin确认隔离窗口属于该外屏；主界面/设置弹窗边界与原生PNG检查通过。
- [x] 两档用户确认清晰、布局正常；设置→阅读四次isTrusted真实点击坐标在目标按钮内。
- [x] 实际BreezeDark/BreezeLight切换：system跟随，显式light/dark保持；SQLite不变、正文节点与段落锚点保持。
- [x] 完成后回读确认外屏110%、内屏150%、位置与BreezeLight恢复。自动20项/人工两档通过，无产品改动。
- [ ] 175%、其它平台/显卡、全部弹窗、量化锐度对比未覆盖。

本项关闭24.13中的这两个限定缺口；其它空/错误状态与跨平台项目仍待验收。[详细证据](2026-09-23-theme-preview/native-settings.md)。

### 24.15 空列表与网络失败（2026-09-24）

- [x] 修复三处提示：无订阅指引、无搜索匹配说明、已订阅但首次抓取失败不误报成功。
- [x] 旧产物三个端到端断言红；修复后英文浅色/中文深色各17项、4图通过。发现503/首次抓取503/无效RSS/断连接/恢复已覆盖。
- [x] SQLite证明失败不丢缓存、成功重试不重复插入；断网时缓存正文可读。Rust413/Node45，clippy仅原有3告警，i18n472 keys。
- [ ] DNS/TLS/超时/限流、跨平台/完整主题矩阵、原始后端错误详情的全面本地化未覆盖。

[报告](2026-09-23-theme-preview/empty-error-states.md)。运行前已重新构建嵌入UI，使用隔离Xvfb与本地HTTP，不影响用户桌面或正式数据库。

### 24.16 网络分类与本地化补验（2026-09-24）

- [x] 稳定code贯通core/数据库/刷新报告/发现IPC，侧栏与添加订阅使用双语说明；旧http_2xx失败有兼容提示。
- [x] DNS注入、短时限头前/正文超时、429缓存保护与手动恢复、发现错误DTO回归。Rust418/Node47通过，clippy仅原有3警告，i18n485 keys。
- [x] 重建后Xvfb真实生产30秒正文超时、自签名TLS拒绝、429单次请求、缓存保护及恢复通过；英文档有TLS正对照及无中文诊断断言。
- [ ] 系统真实DNS、其它TLS错误、代理、Retry-After后台退避及其它产品错误路径全面本地化未验收。

[详细报告](2026-09-23-theme-preview/network-error-followup.md)。原始诊断不丢弃，TLS校验不放宽；未使用用户正式库或修改桌面设置。

### 24.17 限流退避与应用内代理

- [x] Rust430/0、Node49/0、clippy仅原有3警告；期限升级/重启/缓存与代理配置验证。
- [x] 重建后Xvfb英文浅色34项、中文深色24项通过：代理设置保存、路由/绕过/直连、重启保持；英文另含真实30秒超时、TLS拒绝和期限内零请求。
- [x] 代理夹具：HTTP环境代理/NO_PROXY/407、自定义切换/不可达无回退、HTTPS CONNECT拒绝、AI请求与MCP配置动态回读。
- [ ] HTTPS代理自身TLS及真实公网成功隧道、认证/SOCKS、原生Wayland与其它平台本轮未验；不将本地夹具结论扩展到这些环境。
- 报告：`2026-09-23-theme-preview/network-proxy-backoff.md`；结果JSON同目录。

### 24.18 订阅排序、正文安全与数据库持久性

- [x] 组内原生指针拖拽、顺序落库/重启保持，删除取消/确认：Xvfb英文22项与中文23项通过；中文另验恶意正文无执行及事件正对照。
- [x] v14排序迁移、组边界/非法目标拒绝；v1至v13逐版本升级后基础行/标志保持；真实子进程提交到WAL后强杀再打开恢复。
- [x] Rust439/0（30段）、Node51/0；clippy仅原有3警告。报告与JSON：`2026-09-23-theme-preview/subscription-order-security.md`。
- [ ] 原生Wayland/其它平台拖拽、磁盘断电恢复未验；不以Xvfb或进程强杀结论替代。
- 本轮7个隔离探针目录、截图、数据库及构建测试日志已清理；历史暂留目录未动。

### 24.19 调度墙钟、规模性能与分组查询

- [x] 15分钟真实墙钟：启用/关闭两实例、在途手动拒绝、零重复请求、正文节点/滚动保持；启动后10.19秒首刷，共10项断言。
- [x] debug 500源/10k文章：空闲5分钟PSS138.53MiB，刷新3.665秒、最大并发6；脚本交互最大13.67ms，rAF最大57ms，保留短停顿限制。
- [x] SSD文件mincore零驻留页后启动1.034秒，热启动0.708秒；仅数据库冷页，非全系统冷启动。
- [x] v15覆盖feed_id修复分组首屏查询；真实SQL字节码复现先红、旧索引负对照三档均检出；逐版本迁移自动扩展至v14→v15，原分页矩阵通过。
- [x] Rust440/0（30段）、Node51/0，clippy原有3警告；脚本AST与git diff --check通过。
- [ ] release/原生Wayland体感、其它平台、竞品对照与完整外呼抓包尚未验。性能修复尚未重新构建桌面端做整窗测量，不以core测量替代。
- 报告与原始JSON：`2026-09-23-theme-preview/scheduler-performance.md`。

### 24.20 键盘帮助与万篇搜索

- [x] 补齐`?`双语帮助、智能视图/订阅行键盘聚焦激活、导航焦点交接；负对照与4条新测试。
- [x] 重建桌面后Xvfb真实Tab/空格/问号/j/Enter/搜索/Esc串联12项通过；万篇稀疏英文FTS热查询2–4ms。
- [x] Rust440/0，Node55/0；clippy原有3警告，git diff --check通过。
- [ ] 完整键盘AC、原生Wayland按键、读屏器未闭合；宽泛英文UI搜索和英文/中文core查询见24.23。报告：`2026-09-23-theme-preview/keyboard-search.md`。

### 24.21 搜索冷页反例与隔离断网

- [x] 生产upsert万篇中英文夹具，9类查询结果数/重复ID一致/不带正文断言；SSD每词前mincore零驻留页证据。
- [x] v15性能反例已复现：宽泛FTS冷查214–238ms，稀疏/无结果单字LIKE冷查661–706ms；零驻留页，不以tmpfs热查替代。v16/v17优化见24.22–24.23。
- [x] 独立网络命名空间仅lo，真实键盘r刷新失败后仍可j打开缓存，10000条未丢；共 **16** 项通过（以 `offline-namespace-results.json` 的 `checks` 长度为准；此前清单写 15、报告写 16，现统一为 16），不改主机网络。
- [ ] 外部图片离线、原生Wayland/其它平台、逐篇阅读未验。报告：`2026-09-23-theme-preview/search-offline.md`。

### 24.22 单字搜索索引

- [x] v16单字候选索引与200条分批事务迁移；原字段语义/时间排序/已读过滤保持，迁移中断回滚并重试。
- [x] 生产SQL计划与变异：纯单字无临时排序，去掉索引约束即TEMP B-TREE；删除索引中的单字也能触发匹配数量负对照。
- [x] SSD零驻留页稀疏/无结果单字16–18ms，常见字首屏34ms；初次249ms回退记录保留。首次升级约2.79秒及5.3MB体积成本已披露。
- [x] Rust443/0（31段）、Node55/0、clippy原有3告警，桌面重建、脚本AST与diff检查通过。
- [ ] 桌面端单字输入端到端、release/其它平台待验。v17宽泛FTS SSD冷页首次65–77ms、热查约25ms，详见24.23与 `search-unigrams.md`。

### 24.23 宽泛相关度搜索的延迟取行

- [x] v17新增 `(id, read, sortkey)` 覆盖索引；生产相关度查询先收集匹配ID/BM25/排序键，LIMIT后才读取列表元数据，避免每个FTS命中都从正文大列所在的entries记录取字段。
- [x] 同源EXPLAIN断言候选集使用覆盖索引并物化；删除该索引的负对照无法准备生产查询，防止静默退化。
- [x] 10k条/50MB生产upsert样本，每档查询前mincore确认12222页零驻留：宽泛英文“common”冷查77ms、热查24–32ms；宽泛中文“中文”冷查77ms、热查约25ms；“新闻”冷查65ms、热查约25ms。结果200条且跨重复查询ID稳定。
- [x] 同分结果保留最新时间/id优先；隐藏已读在LIMIT之前生效；集成回归通过。数据见 `search-ranked-results.json`。
- [x] Xvfb隔离桌面真实键入到列表绘制约95ms（200条、10k命中）；英文/中文SSD冷页core查询首次65–77ms，避免用热缓存代替冷页结论。
- [ ] 原生Wayland、release构建与其它平台尚未测，因此全文搜索“即时”AC继续开放。

### 24.24 文章列表缩略图

- Media RSS 缩略图优先，其次图片 enclosure，最后摘要/正文首图；相对 URL 按条目源地址解析，只接受 HTTP(S)。
- 缩略图随条目落库，旧开发库迁移时从正文首图回填；全文后取的图片仅在没有既有缩略图时补入。
- WebView 列表延迟加载图片，不发送 Referer；图片请求不经过应用订阅代理。外观设置沿用“显示已有缩略图”开关。该行为是 spec「外呼白名单」的**已知例外**（口径与验收见 [缩略图外呼口径 PRD](2026-09-24-thumbnail-egress-policy/prd.md)）；图片请求由 WebKitNetworkProcess 发起，**应用日志看不到**，验收须以代理/抓包或**系统调用层（本次用的就是 `strace -f -e trace=connect`）**记录为准。
- core 解析、存储/重开、全文回填测试与主题开关单测已补；运行时验收命令：`python3 scripts/verify-thumbnail-ui.py`，断言真实 WebView 图片加载、lazy/referrer 属性及主题开关。
- 本机 Xvfb 隔离运行通过 2 项：WebView 从临时 loopback HTTP 服务取到 1×1 PNG，服务端收到的请求没有 `Referer`；更新主题配置后同一图片节点隐藏、无列表行重建。
- **已验证（2026-09-24，证据均在 [缩略图外呼口径](2026-09-24-thumbnail-egress-policy/prd.md) 变更目录下）**：① 公网真 feed 全链路——`github.blog/feed/` @10:16:43Z 解析 10 条、4 条回填 `thumbnail_url`（SQLite 回读）并在真实 WebView 渲染（`thumbnail-public-feed-results.json` + `thumbnail-public-feed.png`）；② 404 / 不可达主机 / **403 热链拒绝（本地夹具）** 三腿静默失败，无阻塞式 dialog、无行重建、无布局跳动（行几何前后逐项一致、缩略图槽位恒为 48×48）、仍可滚动（`thumbnail-remote-results.json` + `-failures.png` + `-desktop.log`）；③ 离线（私有 netns 仅 lo，`unshare -rn` 单命令复跑）列表可读可滚、缩略图静默失败、缓存正文可读（`thumbnail-offline-results.json`）；④ **关闭缩略图开关后零图片请求**（`egress-strace-toggle-off.log`：该相位 **AF_INET/AF_INET6 连接数为 0**，`toggle_off_destinations` 为空；日志里另有 16 条 `AF_UNIX` connect 是 X11/dbus 的本地 IPC，不算网络外呼）。
- **仍未验证（保留 open）**：① 公网图片站的**热链策略**未实测——用本地 403 夹具替代（站点策略不可控、测了不可复现）；② 未用**公网 RSSHub 实例**跑一次（本轮用本地实例验证 `rsshub://` 解析到实例主机这一步）；③ 本环境 **IPv6 不可达**，AAAA 目标只记录了连接尝试，IPv6 实际可用时的行为未验；④ `spec.md` 的**重定向逐跳归类**规则未被执行（本次 capture 未出现重定向）。

## 25. 首发 schema 基线压平与老库拒绝（2026-09-24-release-schema-baseline）

- [x] 迁移链压平成一份基线 + `application_id = 0x52535331`；老库（旧链任意版本，含 v=1 冻结库）按魔数识别并在任何写入之前拒绝（结构等价断言 + 变异校验 + `detect()` 级 v1 反例）。
- [x] 拒绝不写库：`durability` 断言文件字节不变；`validate_backup` 只读校验且不写候选文件（五类输入：自家 / 文本 / 空库 / 带魔数的更高版本 / 旧库无魔数）。
- [x] 安全出口：`opml::export_read_only` 对旧库只读导出（零写入、无 WAL 边车、与 `Store` 通路按标题定序等价）；桌面遮罩 + 独立 MCP 二进制 `exit=1` + 可读错误（实测）。
- [x] 运行时验收 `scripts/verify-legacy-refusal-ui.py`（exit=0）：遮罩渲染 + 旧库 SHA256 不变 + 备份可读 + 重建后 `application_id=0x52535331`/`user_version=1` 且抓取成功；截图与结果 JSON 见 `2026-09-24-release-schema-baseline/`。
- [ ] **未验（保留 open）**：⓪ **release.yml 的体积记录步骤从未真正跑过**（只做过 YAML 语法校验；CI 因账单停跑，见 ①）；① 面板上的「导出 OPML」走原生 GTK 保存对话框，无 WM 的 Xvfb 驱动不了 → 只断言按钮存在，导出语义由 core 测试覆盖；② 三平台安装包体积与 CI 结论因 GitHub Actions 账单/消费上限停跑而无法取得（`gh run view 35981648535`）；③ Windows/macOS 上的拒绝界面与只读导出未验。

## 26. 本机可测验收收口：键盘 / 搜索 / 离线与外部图片 / 本地化（2026-09-24-local-acceptance-closeout）

- [x] 键盘主流程闭环：新增 `A` = 当前视图全部标记已读（与菜单同入口 `markAll`，双语帮助 + 自检守护）；探针 `scripts/verify-keyboard-mainflow.py` exit=0，含库回读 `unread_left=0`、`r` 刷新 30→31 条、`?` 帮助含 `A`。
- [x] 离线：既有 16 项命名空间证据（`2026-09-23-theme-preview/offline-namespace-results.json`）+ `scripts/verify-thumbnail-offline.py`（列表可读可滚、缩略图静默失败、缓存正文 7344 字可读，`unshare -rn` 单命令可复现）。
- [x] HTTPS 代理成功隧道：`scripts/verify-https-connect-tunnel.py` exit=0 —— 本地 CONNECT 代理日志出现 `github.blog:443`（证明确实走隧道而非直连），经隧道抓回 10 条；对照组「代理不可达」给出可读 `connection_error` 且缓存 10 条仍在。
- [ ] **未验（保留 open）**：① **企业证书 / 需要认证的代理**（需真实网关与凭据，本机无此环境）；② 原生 Wayland 会话下的搜索输入；③ release 构建下 10k 库的搜索「即时」口径（core 口径 16–18ms / 65–77ms 已测，release 整窗未测）；④ 读屏器与跨平台缩放。

## 27. 完整视觉主题 / 设置重构 / MCP 预览批次的台账回写（2026-09-24）

本批（2026-09-23-theme-preview，T1–T7 及后续网络/搜索/缩略图）**没有 Chorus proposal/task**，交付以提交 + 该目录产物留痕；本节把「已交付」与「未验」按证据分开登记，避免台账与证据再次不一致。

- [x] 三套整套视觉预设与用户覆盖、UI/MCP 共用校验/CAS/版本/恢复（spec.md:160 已勾选并引四条证据）。
- [x] 空/错误状态：无订阅 / 无搜索匹配 / 首次抓取失败不误报，失败保留缓存（`empty-error-states.md`）。
- [x] 分数缩放：KDE Wayland 125%/150% 与 Xvfb 100%/200%（`native-settings.md`）；原生 Wayland 真实点击取消 + 正文位置（`t7-wayland-input-results.json`，用户在场补验）。
- [x] MCP 预览文件契约：绝对路径 + 到期时间、尺寸/体积/配额/TTL 回收（`mcp-preview-files.md`）。
- [ ] **未验（保留 open，全部环境/他人依赖）**：① Windows/macOS 原生截图与真实客户端闭环；② 第三方 GUI 客户端（本轮仅 Codex CLI 收图）；③ GNOME / 无 XWayland / 读屏器；④ **用户侧独立聚合评审**（T7 报告明确 fresh reviewer 由用户另派，未派）；⑤ 企业证书/认证代理（与 §26 同源）。

## 28. 搜索端到端（release 口径）（2026-09-24-local-acceptance-closeout / 任务 94422722）

- [x] WebView 键入 → 列表渲染端到端可用：`scripts/verify-search-e2e.py`（隔离实例 + xdotool 真实键事件 + MutationObserver 计时），release 构建、10k 生产形状库。
- [x] 时延实测量与口径标注：宽泛查询冷页 **268ms**、热 **267ms**（200 行封顶页）；单字 CJK 热 **254ms**。库文件每轮新拷贝 → 首查为零驻留页（冷）。
- [ ] **未达「可感知为即时」** → 全文搜索条目保持 open（core 口径 16–18ms/65–77ms 不能代替 UI 成本）。
- [ ] **未验（保留 open）**：① 原生 Wayland 会话下的搜索输入（需用户在场）；② fcitx5 / ibus 候选窗与合成输入（外部依赖）；③ release 口径下的 500 源/其它平台整窗表现。

## 29. 剩余错误本地化：代理族码 + 双语运行时证据（2026-09-24-local-acceptance-closeout / 任务 40c124dc）

- [x] **代理族 5 个失败码**接入界面：`proxy_client_lock` / `proxy_client_setup` / `proxy_invalid_url` / `proxy_invalid_config` / `proxy_credentials_not_supported` → `fetchError.*` 双语文案（此前一律落到「抓取失败，请稍后重试」，用户看不出是代理配置坏了）。
- [x] **契约测试**（机制而非自觉）：`scripts/tests/fetch-error-mapping.test.cjs` —— ① core 能发的码集合必须全被 `fetchFailureMessage` 映射；② 每个映射 key 在两份字典里各有一份。变异校验：删掉 `proxy_client_lock` 那条映射 → 该测试 exit=1；还原 → exit=0。
- [x] **运行时双语证据**：`scripts/verify-error-localization.py`（本地故障夹具 + 隔离实例 + zh-CN / en 各启动一次）exit=0 —— 429（限流文案）、503、404、非 feed（`parse_error`）在侧栏 tooltip 里**两种语言文案不同**，且库里的 `last_status` 码先被断言为非 ok。
- [ ] **未验（保留 open）**：① 非抓取类 Rust 文案（全文抓取 / AI / 命令层）仍是中文，需返回错误码由界面翻译；② fcitx5/ibus 候选窗与合成输入；③ DNS/TLS 原始后端详情在界面上的呈现深度；④ 其它平台。
