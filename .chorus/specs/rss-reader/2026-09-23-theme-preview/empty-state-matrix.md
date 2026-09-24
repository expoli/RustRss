# 空状态/错误状态场景矩阵 + 原生截帧的场景级限制（2026-09-24）

## 为什么补这一批

PRD 第 47 行要求三个固定场景（总览 / 文章 / 设置）包含中英文、长标题、**空/错误状态**等；原矩阵夹具渲染 30 条正文与两个订阅，**只覆盖了错误状态那条 feed 行，没有任何空状态**，而 idea 评论与清单 §27 却把「空状态夹具矩阵」写成环境依赖、并整体结论为「本机无法闭环」。该结论对本项不成立——夹具、示例与矩阵脚本都在本机，可用同一条生产组件路径跑。

## 本轮做了什么

- 新增夹具变体 `src-tauri/examples/theme-ui/empty-fixture.js`（`theme_ui --empty`）：侧栏零订阅 + 一条抓取失败的源（复刻 `ui/app.js` 的 `.dot` 与 `sidebar.feedTooltipFailed` 语义）、列表空行（复刻 `renderList` 的 `<li class="dim">` + `listEmptyKey()` 文案）、阅读区空状态（复刻 `renderReaderEmpty` 的三段）。
- 新增探针 `scripts/verify-theme-empty-states.py`：3 预设 × 2 模式 × 2 语言 × 3 场景 = **36 格**，逐格走原生 WebView 截图；断言每格空文案就是当前语言的渲染文本、失败源 `.dot` 可见且 tooltip 非空、中英文文案互不相同；记录 `binary` + `binary_sha256` + `binary_mtime` + `captured_at`。
- 证据：`empty-state-matrix-results.json`（探针 exit 0）。
- **变异校验**：把空列表文案改成字面量后探针 **exit=1**，还原后 exit=0——断言不是空跑。

## 本轮发现（并改变了既有证据的口径）

**原生截帧在本环境只随主题刷新，不随场景刷新。** 主矩阵 39 张图里，每个（预设/模式/语言）单元格的 overview / article / settings **三张图逐字节相同**（12 个单元格 × 3 = 36 张，实际只有 12 个不同帧；`layout-focus` 与同批 3 张也相同）。已排除「等一次合成就好」：把 `settle` 从 2 帧延长到 400ms 后重复依旧。

因此：
- 既有「39 captures / 23 checks / 24 pixel checks（每后端）」的说法里，**逐场景的像素断言不成立**（那是同一帧被比了三次），只有**逐主题/模式/语言**的像素结论成立；逐场景结论必须由 DOM 断言承担（本轮给设置场景补了语义化 DOM 断言：列表右缘处的最上层元素不得属于侧栏/列表）。
- 这是与清单 §30 不同的**第二种陈旧帧模式**：§30 是 `import -window root` 取到别人的帧；这里是 `view.snapshot()` 在无合成器环境下不刷新场景。先前「产品图都走原生截图，故 §30 不适用」的推论**不足以**推出「场景级像素可信」。
- 处置：矩阵探针改为**记录** `scene_frames_per_cell`（现为全 1）而不是按场景断像素；空状态矩阵同理——它的逐格主张由 DOM 断言承担。

## 仍未验（保留 open）

跨屏/其它合成器的分数缩放、读屏器（`/usr/bin/orca` 在本机存在，本机可试而未试）、原生 select/字体建议弹窗的深色与键盘路径；Windows / macOS / GNOME-Wayland（未安装 GNOME 会话）与第三方 GUI MCP 客户端。
