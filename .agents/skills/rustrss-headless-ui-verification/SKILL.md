---
name: rustrss-headless-ui-verification
description: 在无显示器/CI 环境里验证 RustRss（Tauri 2 + WebKitGTK）的 UI 行为：Xvfb 隔离实例、截图取证、像素级断言、sqlite 回读、以及 WebKitGTK 无 WM 环境的坑（陈旧帧、tooltip、点击被吞）。当需要「改 UI 后拿实机证据」「复现用户报的界面问题」「验证热键/拖拽/弹窗交互」时使用。
---

# RustRss 无头 UI 验证手册

适用：改动 `ui/**` 后需要**机器可核验**的实机证据（而不是"看起来没问题"）；或在无桌面会话的机器上复现/定位界面问题。

## 0. 前提（最容易踩的一条）

**Tauri 编译期把 `ui/` 嵌进二进制**：改完前端必须 `cargo build -p rustrss-desktop` 再跑，否则跑的是旧 JS（`tauri.conf.json` 同理）。验证前先确认二进制 mtime 晚于 ui/ 改动。

## 1. 隔离实例（绝不碰用户真实库）

```bash
mkdir -p /tmp/<tag>/home
cp ~/.local/share/rustrss/rustrss.sqlite /tmp/<tag>/db.sqlite      # 用户库副本
Xvfb :99 -screen 0 1920x1200x24 &                                   # 若未跑
cd /path/to/RustRss
DISPLAY=:99 GDK_BACKEND=x11 GDK_GL=disable \
  HOME=/tmp/<tag>/home RUSTSS_DB=/tmp/<tag>/db.sqlite \
  setsid target/debug/rustrss-desktop >/tmp/<tag>/run.log 2>&1 </dev/null &
```
- **`GDK_GL=disable` 必须**：WebKitGTK 2.5x 在 Xvfb 下用软渲染，开 GL 会白屏/黑屏。
- `setsid` + 重定向：避免前台阻塞；结束时 `kill <自己起的 PID>`，**不动用户实例**。
- 数据目录跟随 `HOME`；日志在 `/tmp/<tag>/home/.local/share/rustrss/logs/`（**`[ui]` 行只进日志文件**，stdout 默认静默；需要 stderr 镜像时加 `RUSTSS_LOG_STDOUT=1`）。

## 2. 截图取证的三个坑

1. **陈旧帧**：无 WM + 软渲染下 WebKitGTK 可能停在旧帧 —— 截图前对窗口制造一次 damage：
   ```bash
   xdotool windowsize $WID 1239 820 && xdotool windowsize $WID 1240 820   # 尺寸回弹
   ```
2. **自检**：连拍两张，`md5sum` 必须不同；相同 = 没重画（重做 damage）。
3. **窗口几何**：`xdotool search --onlyvisible --name RustRss getwindowgeometry`。无 WM 时窗口固定在 `0,0`，尺寸可能就是 `1240x820`；**不要硬编码**，先查。

```bash
DISPLAY=:99 import -window root /tmp/<tag>/NN.png
python3 -c "from PIL import Image; Image.open('/tmp/<tag>/NN.png').crop((0,0,1240,820)).save('/tmp/<tag>/NN.png')"
```

## 3. 断言技术（把「看起来对」变成机器可判）

| 目标 | 做法 |
|---|---|
| 内容/位置未变（如"按 u 后滚动位置保持"） | `PIL.ImageChops.difference` 对关注区域算差异像素数与 bbox；经验阈值：AE < 关注区 1% 且 bbox 只落在预期小控件上 |
| 数据真的写入 | `sqlite3 /tmp/<tag>/db.sqlite "select ..."` 直查（系统 sqlite3 CLI **无 FTS5**，但普通表可读） |
| 前端状态 | 应用自证日志（`[ui] ...`）：优先找现成的诊断行，其次在改动处加一行可检索的 `log()`；关键交互（滚动保持/计数/视图）都可打点 |
| 预置状态 | 直接写副本库：`INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(...)`；造标签/日志文件同样用 SQL/文件系统 |

## 4. 无 WM 环境的已知限制（别误判成缺陷）

- **GTK tooltip 会留在截图里**（鼠标悬停触发，移开鼠标再截）。
- **最小化/最大化**无 WM 时无意义（只有关闭可测）。
- **部分元素点击被吞**：工具栏 `data-tauri-drag-region` 上的按钮在无 WM 时 XTEST 可能不生效 → 换等价路径验证，或如实标注「点击一跳未机器验证」。
- **原生拖拽（HTML5 DnD）后可能留下陈旧 hover 底色**：DOM/类名无残留即属环境重绘怪癖（留有 WM 时未复现的记录）。

## 5. 证据清单（交付时给这些）

1. 截图路径 + 对应验收点（每张说明"看图能确认什么"）；
2. 关键日志行（`[ui]` 诊断）；
3. sqlite 回读值（写入类功能）；
4. 断言脚本输出（像素差/md5/计数）；
5. **如实区分**：已验证 / 未验证 / 无法验证（例：Wayland、HiDPI、IME、真实文件管理器交互）。

## 6. 清理

```bash
kill <自己起的 PID>
rm -rf /tmp/<tag>                 # 字面路径，dcg 放行；勿用变量路径
```
