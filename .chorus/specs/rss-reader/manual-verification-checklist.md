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
