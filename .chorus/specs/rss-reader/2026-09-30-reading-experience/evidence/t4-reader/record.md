# T4 阅读器、操作与 AI 发送确认

任务 `27b49f74-9114-4a67-ac2e-dc326c8c6d87`。全部设备写入只发生在独立 `emulator-5582` 和临时桌面库；没有操作用户的 `emulator-5554` 或真实订阅。主结果是 [Android 26 项回读](results.json)、[桌面 8 项](desktop-results.json) 与 [TalkBack 语义树](talkback-dom.json)。Android 点击和返回由 `adb input` 发到真实系统，CDP 只读取状态与填入夹具输入；状态由 Tauri IPC、系统窗口或服务端计数回读。

## 安装包与夹具

| 产物 | SHA-256 | 说明 |
| --- | --- | --- |
| `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk` | `7a719a531d33d216b8b82268f76ab648e08a4b19890f086ee996b60c3ae9dddb` | 703,502,654 字节；Android 36 x86_64 模拟器实测 |
| `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk` | `c9fd508e53240461af3e268dc3532a6f75edad1592c6bedc7dc1548364695d72` | 73,071,838 字节；0.2.1，ARM64 + x86_64；`apksigner verify` 成功，证书 SHA-256 `89269c116afaa9ca546f7287b268f00c14d5598b6c633715a74fbbcd28de8cc5` |
| `target/debug/rustrss-desktop` | `10a19cb23e56c14f8385446416422ba8c479011c04f0df5e56b21c290f5a05a9` | 虚拟 KWin Wayland 的真实 Linux WebKitGTK |

先运行 `cargo run -p rustrss-core --example reading_experience_fixture -- /tmp/rustrss-t4-fixture.sqlite 30`，然后在这份临时库中将 `entries.id=1` 的 `url` 改为 `http://10.0.2.2:18080/missing`、`entries.id=2` 改为 `http://10.0.2.2:18080/page`，并把 `settings.key='ui.mark_read_on_navigate'` 设为 `false`。本次夹具 SHA-256 为 `03911253c9ff26ac7f1eb0d8790e0e9dc6c301b7f6455248d04dfeeb40eddd91`。脚本通过真实 `get_ui_settings` 回读该标记，不能只相信预设说明。列表的直接点按显式传 `markRead:true`，故入口记录从 `read=false` 变为 `true`；更多菜单随之提供“标为未读”。测试同时断言原列表行 DOM 身份未变且其 `read` 类已更新。

```sql
UPDATE entries SET url='http://10.0.2.2:18080/missing' WHERE id=1;
UPDATE entries SET url='http://10.0.2.2:18080/page' WHERE id=2;
INSERT INTO settings(key,value,updated_at) VALUES('ui.mark_read_on_navigate','false',1790738844)
  ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at;
```

用 `python3 scripts/reader-fixture-server.py` 启动只返回合成全文和 Ollama 格式响应的本机 18080 服务；它不记录 prompt 或请求头，只把请求数写到 `/tmp/rustrss-t4-{ai,page}-count.txt`。将夹具库推入任务 AVD 的应用数据目录，安装 debug APK；转发该进程的 WebView 调试端口到 `tcp:9229` 后执行 `ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/verify-reading-experience-reader.mjs .chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t4-reader`。测试对 AI 只配置 `ollama`、`synthetic-t4`、`http://10.0.2.2:18080`，没有 API key。桌面执行 `dbus-run-session -- /usr/bin/python3 scripts/verify-reading-experience-reader-desktop.py <evidence-dir>`；TalkBack 执行 `ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/record-reading-experience-reader-talkback.mjs <evidence-dir>`。

将同一夹具再次重置进已安装的 debug APK 后，执行 `ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/capture-reading-experience-reader-matrix.mjs <evidence-dir>`。它通过 Tauri IPC 设置 locale/theme、重载应用、Android 原生点按同一篇长文并截取四种组合；[矩阵几何与配色回读](matrix-results.json)包含每张图的设置原值、首段比例、目标尺寸和溢出判定。

## 五项验收

| AC | 实测 |
| --- | --- |
| 1 阅读层级 | 360 CSS px × 776 可用高度（设备 `wm size` 为 360×800 CSS 口径）中，双行短标题后首段位于 27.85%，四行中英长标题后仍为 35.22%；均小于 40%。元信息单行；Back、Aa、星标、稍后读、更多都为 48×48 CSS px。截图：[长标题正文](01-reader.png)、[更多面板](02-more.png)。标签添加位于元信息之后，不改变首段阈值。 |
| 2 M09–M12 动作 | 更多打开前后 AI/全文请求数皆为 0，原生 Back 关闭后焦点回到“更多”。标未读、星标、稍后读、标签搜索/新建/附加/移除均回读存储；标签 chip 具有按钮语义和 48px 高度，点击后进入对应标签视图。复制链接与剪贴板一致；[系统浏览器](03-external-browser.png) 和 [Android 原生分享表](04-native-share.png) 都由系统打开，Back 取消后回阅读器。全文 `/missing` 的 503 留 `needs_fulltext=true`，同一篇再次点按在合成服务恢复后成功并写回；另一篇 `/page` 也提取并写回。 |
| 3 AI 确认和结果 | [合成请求预览](06-ai-confirm-synthetic.png)显示真实目标、仅 Content-Type 的脱敏头、2725 字符正文；打开和取消都没有发送。确认一次对应服务端计数一次；摘要、翻译、重新生成、关闭保留正文、503 失败与重试恢复、勾选“不再询问”的持久值均回读。[摘要结果](07-ai-result.png)、[恢复结果](08-ai-recovered.png)。公开证据中的正文与 prompt 都来自生成夹具，没有用户文章、密钥或原始请求日志。 |
| 4 长文、Aa、返回 | 81 段中英正文、代码块、表格在阅读器内无横向溢出。`example.invalid` 图片加载失败后显示带图像语义的 alt 文本占位，见首屏；不会留下破图图标。现有全局列表缩略图开关的双向切换及其与阅读器离线图片的边界见下方补证。Aa 预览/放弃不增加主题 revision；保存字号后 revision 0→1，同一正文锚点保持可见，[Aa 截图](09-aa-saved-anchor.png)。非空 `English` 搜索时原生输入法出现、系统 Back 返回仍保留查询与结果；400 行夹具的第二页、最早排序和非零列表滚动位置返回前后相等，见下方补证。 |
| 5 跨端/可访问性 | Android 原生 Back 覆盖更多、系统浏览器、分享表、输入法及搜索结果阅读返回。TalkBack 服务启用后，[阅读器树](talkback-reader.xml)和[更多树](talkback-more.xml)出现命名按钮与 7 个当前动作（全文已抓取后不再显示），菜单行高≥48px；服务测试后复原到关闭状态。[桌面结果](desktop-results.json)在真实 WebKitGTK 窗口检查菜单 role、初始焦点、ArrowDown、Esc/焦点恢复、聚焦动作写入一次、Aa 命名字段。桌面键通过 Inspector 发 DOM `KeyboardEvent`，不能证明实体键盘默认 Enter 激活；TalkBack 树不证明实际语音输出。 |

### 同一长文的语言和主题对照

四张截图均来自最终 debug APK、同一合成长标题与正文（四行标题）；可用视口 360×776，首段均为 35.22%，操作目标均≥48×48 CSS px，元信息单行，阅读器和页面均无横向溢出。浅色背景/正文为 `rgb(247,248,250)`/`rgb(28,31,38)`，深色为 `rgb(20,22,26)`/`rgb(230,232,236)`；语言切换后“更多”显示“More”。

| 组合 | 安装包设备截图 |
| --- | --- |
| 中文 · 浅色 | [zh-light-reader.png](zh-light-reader.png) |
| 中文 · 深色 | [zh-dark-reader.png](zh-dark-reader.png) |
| English · Light | [en-light-reader.png](en-light-reader.png) |
| English · Dark | [en-dark-reader.png](en-dark-reader.png) |

没有在同一夹具上保留 T3 旧包的“改前”四组合截图；本表证明最终包四种状态的一致性，不声称像素级前后对照。

### AC4 补证：现有图片开关、离线占位与第二页返回

[补证结果 JSON](followup-results.json)在**同一已安装 debug APK**上通过 3 项检查。另取独立 400 行合成库：从 `reading_experience_fixture 200` 生成 200 行，经 Python 自带 FTS5 的 `sqlite3` 将其复制为 201–400 行（`stable_id` 加 `-t4page2`，发布时间减 100000 秒），并把 `entries.id=1.thumbnail_url` 设为本机合成 `http://10.0.2.2:18080/thumbnail.svg`。该库 SHA-256 为 `bcee169937f14b9db3dcb11bc95d0bb6f77dbe3f8712bbff8235c90324ebd38a`；合成服务器 `/thumbnail.svg` 只返回一个 64×64 蓝色 SVG 和请求计数。先将库复制进任务 AVD，将 `/tmp/rustrss-t4-image-count.txt` 置零，再运行 `ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/verify-reading-experience-reader-followup.mjs <evidence-dir>`。脚本先把排序和列表缩略图恢复到基线，以保证重复运行。

- **图片开关范围**：通过原有 `update_ui_theme` core IPC 将 `list.thumbnail` 从 true→false→true，并重载已安装应用。列表缩略图真实加载为 64px，CSS 显示状态依次为 `block`、`none`、`block`，core 快照也回读 false/true；[开](10-thumbnail-on.png)、[关](11-thumbnail-off.png)、[恢复](12-thumbnail-restored.png)。关时列表仍保留缩略图 DOM；这是现有**列表显示偏好**，不宣称禁止外部请求。两次打开文章，`example.invalid` 外部正文图都显示同一 `Fixture illustration` 离线替代文字，正文图片加载策略未因列表开关改变。合成缩略图端点累计收到 1→2 个请求，精确说明了这里的可见性和网络范围。
- **第二页和返回锚点**：All 视图最早排序落库为 `oldest`，列表由 200/400 续页到 400/400；选取已加载后半页中的复制夹具行 `296`，从非零 `scrollTop=26672.38` 打开文章。Android 系统 Back 后，All、`oldest`、400/400、400 行、顶部可见行 `363`、偏移约 -32px 和滚动值全部与打开前相同；所选行变为实际打开的 `296`。截图：[第二页列表](13-page-two-list.png)、[阅读](14-page-two-reader.png)、[返回](15-page-two-back.png)。正文搜索的非空查询和取消后的 All 锚点由主结果的 `search-reader-android-back-ime`、`search-cancel-all-scroll-anchor` 单独证明。

`node --test scripts/tests/*.test.cjs` 61/61、`cargo test --workspace --locked` 全通过；`git diff --check` 无空白错误。T2 共享菜单回归在本任务 AVD 复测原生 Back 零写、焦点恢复及选择后一次写入。截图均取自安装后的 Android WebView/系统界面，不是 CSS 注入预览。通用 APK 的 ARM64 ABI 与签名已检查，但没有物理 ARM64 手机；Windows/macOS 和实体键盘留待跨端总验收。
