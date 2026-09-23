# T7 跨阶段验收执行报告

2026-09-24。基线 `5abd0c6`（T4），本次包含发现后的契约修复。
这是执行证据汇总，**不是独立聚合评审或全平台通过声明**。未创建 Chorus proposal/task；fresh reviewer 由用户另派。

## 结论与证据来源

T4 设置重组破坏了 T6 固定预览契约，已修复并重建后复验。Linux 共享组件矩阵、MCP Xvfb 生命周期回归、配置跨入口一致性通过。真实墙钟生命周期结果见下节和 [机器数据](t7-results.json)。

**Open item：原生 Wayland 真实点击 + 正文位置断言。** 用户不在电脑前，授权未完成；按用户最新指示终止授权进程，不再触发弹窗。该组合项既不判失败，也不标通过。已有 inspector DOM 测量仅作辅助证据，不替代可信指针输入。复跑方式见后文。

| 来源 | 采用的证据 | 限制 |
|---|---|---|
| 本轮执行 | 重建、Node、专项 Rust、Clippy、矩阵、MCP、原生 DOM/SQLite、墙钟 | 仅下述环境与样本 |
| 用户独立复跑 T4 | Rust 405/0，26 段；Node 33；Clippy 原 3 警告；i18n 468；settings 12 checks/3 captures；desktop 7 分类、focus_wrap、Aa 草稿不写、23px/revision1、正文1→1、重启保持，并直读 SQLite | 是 T4 基线结果，不能冒充本次修复后的全量 Rust 结果 |
| 用户此前独立复跑 T3/T5/T6 | T3 386/22 与 probe；T5 393/25、事件/stdio 同步；T6 405/25、Xvfb 1×/2×各25图、Wayland24图与4条安全不变量 | 历史版本证据，不能覆盖后来的共享结构变更；本轮正是发现了这种回归 |
| 既有阶段报告 | [T3](shared-theme-renderer.md)、[T4](t4-settings.md)、[T5](mcp-theme-config.md)、[T6](mcp-theme-preview.md) 中的设计、旧测量、例外清单 | 原作者自述；未复跑部分仅作追溯 |

## 发现、修复与复现测试

- `ui/preview.js` 仍引用 T4 已移除的 `set-theme`、`set-theme-preset`、旧字体/滑块 ID；新共享 markup 下渲染会在 ready 前异常。
- 固定资源协议漏服务 `ui/theme-settings.js`。现在嵌入并服务该文件，预览复用生产外观编辑器的只读模式；禁用表单、移除保存操作，不读历史、不调用配置 IPC。
- 本地取消按钮引用未定义的 `--text` / `--border`，改用 `--fg` / `--line`；`--bg` 本来就是有效背景 token，保留。没有新增另一套调色板。
- `scripts/tests/preview-contract.test.cjs` 检查 ID 存在、共享脚本资源可达、inline CSS token 定义。前两项在修复前红；旧代码的 CSS 反例明确为 `--text`、`--border`，修复后 3 项绿。
- 验收脚本旧坐标 y=120 在 T4 两行列表头上，改为 y=150 打开文章；首次脚本失败不算产品通过。长测 Xvfb 使用独立 `XDG_RUNTIME_DIR`，避免 GTK 在没有 WAYLAND_DISPLAY 时仍连接调用者 runtime 下的默认 Wayland socket。显示变量隔离仅在测试环境层，产品不强制后端。

重建：`cargo build --workspace --features rustrss-desktop/snapshot-probe --bins --example theme_ui`，exit 0；运行验证使用这次新嵌入资源的二进制。本轮未重复全量 Rust 405 项；新增/受影响路径及安全契约的专项结果存入机器数据。

## 矩阵与产品路径

| 路径 | 覆盖 | 新鲜结果 |
|---|---|---|
| 共享生产组件，Xvfb 100% | Clear/Paper/Slate × light/dark × overview/article/settings × zh-CN/en =36；另3个布局场景 | 39 captures /23 checks /24 pixel checks；常规1240×820像素 |
| 同上，Xvfb 200% | 同36+3 | 39/23/24；常规2480×1640像素 |
| 同上，当前 KDE 原生 Wayland 200% | 同36+3 | 39/23/24；GdkWaylandDisplay，常规逻辑1240×772，像素2480×1544（WM 内容区尺寸） |
| 真桌面 + MCP HTTP/stdio，Xvfb 100% | 预设/模式/场景与生命周期 | 25图；临时不写、stdio桥接图片、幂等保存、Xvfb本地取消、撤权回收；正文渲染1→1 |
| 真桌面，当前 KDE Wayland 200% | 临时预览、保存与段落辅助测量 | 原生PNG逻辑960×640、物理1920×1280；字体24→22px，宽度540→620；同文章/行/段落节点，段落偏移0.09375→0.09375 CSS px；SQLite revision2 |

矩阵没有省略要求的 100%/200% 组合；但不是每个后端都各跑两档：Wayland 100% 未跑；完整 MCP 生命周期 200% 未在本轮重跑。117 张组件图和25张 MCP 图属于不同验证路径，不能合并称为142个完整桌面交互。

留存样本：[MCP 只读外观场景](t7-preview-settings.png)、[200% 英文共享组件](t7-matrix-settings.png)。截图只包含固定本地夹具。

真实 UI 的 `validate_ui_theme` 与 MCP `validate_theme` 对同一 revision/patch 返回完全一致的 snapshot/hash，SQLite revision 不变。原生 MCP 设置预览检查：只读 fieldset、0个历史控件、6张预设卡、52个颜色字段、外观 pane 可见。

本轮 Rust 专项38/38（core theme17、MCP theme5/preview5/http3/write_auth6、desktop preview2），四条HTTP安全不变量在本轮均通过；`http.rs` 未修改。本轮 Node 36/36，双语 self-test 469 keys、0问题；Clippy exit0，仅原有 `fulltext.rs:130`、`store/mod.rs:478/697` 三条告警。JS 增量含新增3条预览契约测试。后续 Wayland 输入助手仅语法检查，不能算运行通过。

## 真实墙钟与资源

运行 `python3 scripts/verify-theme-lifetime.py`，并行两个隔离 profile：空闲实例不续期，活动实例每120秒捕获；没有改系统时间、缩短TTL或测试时钟注入。每30秒采样进程树 RSS、窗口、profile文件大小、全盘余量；每120秒记录 `df -h .`，低于10GiB自动中止。PNG仅在内存校验签名/预算/原生后端/新帧，不保存批量图片。

两条实际墙钟断言均通过（脚本 exit0）：空闲 **599.75秒**回收；持续捕获的活动预览 **1800.52秒**回收。计时从创建请求前开始，core按整数秒计时，因此允许约1秒量化误差，脚本阈值为目标-2至目标+12秒。两者回收后保存均被拒，采样期间 `ui.theme_config` 始终不存在。

| profile | 捕获次数 | 进程树RSS：初值→最后采样（KiB） | RSS采样范围（KiB） | profile文件增长 |
|---|---:|---:|---:|---:|
| idle | 1 | 645544→350032 | 349988–645544 | 1702 bytes |
| absolute | 15 | 646752→519484 | 518656–648956 | 22988 bytes |

60次资源采样，最后一次约1771秒；因此absolute最后资源采样仍在回收前，不冒充回收后的RSS。idle窗口由主窗+预览降为仅主窗，进程树4→3。absolute在1800.52秒观察到预览消失；随后脚本finally停止两套隔离应用及Xvfb。原始逐次捕获/错误/采样保存在机器数据中。


RSS 是主进程及 WebKit 子进程的 RSS 求和，不是 PSS，会重复计共享页。目录字节是文件逻辑长度，不是物理块占用。30分钟趋势不足以证明长期无泄漏；这也是热态固定数据验证，不含冷盘或生产订阅规模性能结论。全盘余量变化还包含同期增量编译及其它进程，不能归因于预览。

## 逐条对照 PRD AC

| AC | 跨 phase 结论 | 证据及尚缺部分 |
|---|---|---|
| 1 预设/语义颜色与状态 | Linux 样本覆盖；不作全平台声明 | T3 token清单 + 本轮108组合图、原生背景/遮挡像素；没有穷举每个原生系统弹层/读屏器状态 |
| 2 旧配置/未知schema/无关数据 | core 契约覆盖 | T2/用户T4 Rust基线 + 本轮 theme 专项；运行夹具的正式主题由SQLite回读。未直接升级用户真实库 |
| 3 UI/MCP等效、同值零写 | 对同一patch的跨入口快照一致；同值分支由专项测试覆盖 | 本轮UI/MCP hash、Node共享renderer、core/MCP测试；不是每个patch字段组合的穷举 |
| 4 临时/原子保存/取消/超时 | Linux已测路径通过 | 本轮MCP5测试、25图与真实TTL；本地Wayland取消组合仍open |
| 5 图片元数据与版本 | 已测图片与失效握手契约通过 | MCP metadata、freshness marker、桌面2项握手测试；原生PNG尺寸单独核验 |
| 6 三场景共用生产组件 | 共享组件/代码diff/引用/表格/本地图片已覆盖；空状态覆盖不足 | 本轮矩阵 + 只读外观页修复；固定场景主要是有数据状态，未形成独立空订阅/空列表矩阵，保持该AC未全勾 |
| 7 权限/轮换/本地取消 | HTTP/权限与Xvfb本地取消通过；Wayland组合open | 读token/撤权专项与probe；不能用Xvfb替代Wayland真实点击 |
| 8 关闭/超时/冲突/unavailable | 错误与配置独立路径由专项/历史证据覆盖；平台运行不足 | 本轮render失败/桥接/CAS测试；早期原生inspector场景出现render_timeout，后续重试成功，未证明该偶发超时根因已消除 |
| 9 不重建/焦点/段落锚点 | Xvfb产品回归+Node、T4用户复验；原生组合open | 本轮正文1→1；Wayland DOM偏移辅助证据不算真实点击组合通过；没有覆盖任意远程图片重排 |
| 10 跨平台运行 | 未完成 | X11和当前KDE样本；Windows/macOS截图适配未实现/不可用；GNOME、其它compositor未测 |

PRD checkbox 保留未满足的全范围项；durable spec 增加本轮已完成的子项。不能由“矩阵通过”推导整个特性或独立聚合评审通过。

## Open item 单跑：用户在场的原生 Wayland 输入

前置条件：本机 KDE Wayland 桌面会话、用户在场；保留真实 `WAYLAND_DISPLAY` / `XDG_RUNTIME_DIR`，不得切换到 Xvfb；允许本轮隔离窗口的临时指针控制。需 distro `/usr/bin/python3` 的 `websockets` / `gi`、`qdbus6`、`journalctl`，以及已经重建的桌面与 `theme_fixture`。WebKit inspector 只绑回环，用于夹具测量；[环境变量说明](https://trac.webkit.org/wiki/EnvironmentVariables)。不向用户真实订阅库启动 inspector。

终端A，从仓库根执行：

```bash
df -h .
# 仅当二进制缺失或 ui/ 已改动时构建；运行前重新确认空间
cargo build -p rustrss-desktop
cargo build -p rustrss-core --example theme_fixture
/usr/bin/python3 scripts/start-theme-wayland-probe.py
```

保留终端A，等待隔离桌面启动；记下打印的 `/tmp/rustrss-t7-native-XXXX/instance.json`。终端B替换下面的路径：

```bash
/usr/bin/python3 scripts/verify-theme-wayland.py /tmp/rustrss-t7-native-XXXX/instance.json
/usr/bin/python3 scripts/verify-theme-wayland-cancel.py /tmp/rustrss-t7-native-XXXX/instance.json --portal
```

第二条仅请求**一次**KDE授权，120秒超时不重试。允许后定位仅属于该隔离PID的预览，检查窗口/内容坐标一致、指针到位后才发真实点击；不匹配则失败退出而不乱点。也可去掉 `--portal`，由用户直接点隔离预览的取消按钮。脚本用每轮唯一标记记录 `event.isTrusted`，检查窗口销毁、保存被拒、正式主题完全未变、文章/选择/段落节点与偏移。完成后终端A Ctrl-C退出，权限会话在finally关闭；原生输入助手**本轮仅语法检查，仍需这次现场运行**。

如果候选已超过10分钟空闲期限，重新启动隔离probe并按上述顺序单跑；不要拿已过期会话验证取消。正文辅助测量的准备动作由inspector DOM automation完成，真实取消必须来自指针。失败原始证据保留，不能仅凭metadata宣称实际点击。

## 其它未验证项与剩余风险

- 第三方 GUI MCP 客户端的图片呈现、消息体限制和模型根据图片继续调整：只测了真实 HTTP/stdio 协议客户端，未测真实客户端UI。
- Windows/macOS、GNOME、跨屏125/150%分数缩放、原生选择/字体建议弹层、读屏器、真实最小化/系统明暗切换；逐字形fallback仍unknown。
- 真实订阅列表缩略图数据来源仍未实现；固定本地图片仅验证样式。
- 空状态场景专项不足；权限/旧响应/资源上限依赖专项测试和既有phase证据，不能把所有历史结论都标为本轮重跑。
- 原生inspector初次捕获出现过render_timeout，重试成功；初版辅助测量还曾等待旧正文节点导致脚本失败，已改为等待新节点。报告只采纳最终成功辅助测量，组合项仍open。
- 后续独立评审由用户安排；本轮不物化任务、不自行评审、不自动启动其它工作。

## 磁盘与清场

开跑约66G可用；长测期间降至约62G可用（94%）。增量专项构建与其它活动计入全盘，profile实际增长单独记录。没有执行 cargo clean，也没删除 target 或此前T4用户已接受的目录。

本轮隔离应用、Xvfb、原生辅助桌面及授权进程已退出；KWin临时脚本已卸载。尝试用字面 `/tmp` 路径删除本轮目录，被自动执行策略拒绝：`rm -f style commands are not permitted`。未改用其它方式绕过，原T4暂留目录未动。以下本轮目录约58MiB暂留：

- `/tmp/rustrss-t7-lifetime-70anypj2`（成功长测）、`/tmp/rustrss-t7-lifetime-vam9gv76`（首次环境隔离失败）。
- `/tmp/rustrss-t7-native-o2y1buk4`（辅助DOM/SQLite证据，无可信输入通过结论）。
- `/tmp/rustrss-theme-ui-jtnegbpq`、`/tmp/rustrss-theme-ui-sqhj99db`、`/tmp/rustrss-theme-ui-yithk493`（三档矩阵）。
- `/tmp/rustrss-theme-preview-3qgujmtk`（MCP成功回归）、`/tmp/rustrss-theme-preview-u3bm6e7a`（旧坐标失败）。
- 本轮辅助脚本/日志 `/tmp/rustrss-t7-*`；没有继续尝试清理。

最终 `df -h .`：937G总量、829G已用、62G可用、94%。仓库只留明确命名的两张验收PNG及机器JSON；未生成/提交 `__pycache__`、临时PNG/日志。机器JSON保留完整矩阵元数据、每次TTL捕获、60次资源采样、测试结果及二进制SHA256；原图批次暂存于上述目录。

提交前自检（非独立评审）：P0要求/事实、P1文风/结构、P2产物范围已核对；open item未勾通过，独立评审状态保持pending。
