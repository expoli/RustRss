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
