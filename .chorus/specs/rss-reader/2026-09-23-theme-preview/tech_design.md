# 技术设计：主题配置与渲染预览

状态：core 主题模型/存储已实现（[T2](core-theme-model.md)）；[T3 UI/共享渲染](shared-theme-renderer.md) 已实现；MCP 接入为拟议实现。截图资源预算仍待产品集成校准。

## 当前基础

`ui/style.css` 已有颜色/字体变量；`ui/app.js` 的 applyTheme/applyFontConfig 负责应用；`src-tauri/src/commands.rs` 存 ui.theme 和字体设置。MCP 已有 registry/scope/switches/audit，内嵌与独立进程共用 core/SQLite。现有设置返回值驱动 UI 更新，不能把外部进程写入数据库等同于界面已重绘。

仓库锁定 tauri 2.11.6 / wry 0.55.1 / webkit2gtk 2.0.2。Tauri `with_webview` 可在 UI 线程访问平台对象；平台句柄不能跨 await 传递。截图结果通过有界通道返回。

## 分层

1. **core/theme**：ThemeConfig、Patch、Preset、归一化/校验、语义 token、配置指纹、原子 CAS 保存、有限历史；不依赖 Tauri。
2. **MCP**：工具 schema、权限、审计、结构化响应与 ImageContent；调用 core 和抽象 ThemePreviewBackend，不依赖 WebView 类型。
3. **桌面适配器**：实现预览后端、窗口/事件/平台截图；所有主题语义规则复用 core。
4. **UI**：原生 JS 模块共享组件和 token 应用器；预览页面注入固定 fixture，禁止加载生产数据库。CSS 变量局部变更并同值短路。

### 两种 MCP 部署

- 内嵌 HTTP：注入桌面预览后端，通过内存请求队列调度 UI 线程。
- 独立 stdio/HTTP：注入桥接客户端。首版选择桥接到用户已启用的桌面 loopback MCP 服务，复用现有授权，不新增开放端口。无需桌面服务的 get/validate/update 配置操作仍直接使用 core。
- 桥接必须核对同一 profile/database 的稳定标识；禁止把测试库的请求送到默认窗口。目标未启用/不同库返回 `preview_backend_unavailable` / `profile_mismatch`。内嵌服务明确使用本地后端，禁止递归转发。
- get_theme 返回实际 preview capability 及原因；UI 的客户端配置区解释独立 stdio 预览需启用桌面 MCP。后续可用受限本地 IPC 消除这一依赖。
- 所有图片获取接口走现有鉴权；health 不新增订阅/主题内容。预览工具归入普通写 scope，不打开 dangerous scope。

## 配置模型

ThemeConfig 外层包含 schema_version、revision、mode（system/light/dark）、light_preset、dark_preset、overrides。effective 配置由预设加覆盖计算；系统模式变更不写 revision。

| 分组 | 字段/初始约束 |
| --- | --- |
| colors.light/dark | 背景/侧栏/面板/正文/次要文字/强调/选中/边框/焦点/错误/星标、代码与 diff 语义色；首版仅 #RRGGBB |
| typography | ui/read/mono 字体族列表；最多 4 项、单项 ≤128 字符；使用明确系统回退；UI 12–20px，正文 13–28px（兼容旧 13px 偏好），代码 12–24px，行距 1.3–2.2 |
| list | density compact/comfortable；summary_lines 0–3；thumbnail boolean |
| reader | width 480–960 CSS px；paragraph_gap 0.5–2 em；layout three_column/focus |
| chrome | radius 0–16px；sidebar 180–300px；list 260–460px；窄窗口按可用尺寸收缩 |

字段未知/越界/无效色值拒绝；字体缺失允许回退并反馈 requested/resolved family 或无法确定。混合 CJK glyph fallback 不能只凭 computedStyle 声称全部文字使用同一字体。输入是数据，不能包含执行代码或任意样式属性。

patch 仅更新显式字段；null 代表删除覆盖并继承预设（schema 明确）；数组全量替换。预设版本应纳入 config_hash，内置预设改变时保持可追踪。

legacy 设置首次读取映射为当前等效主题；首次保存才写新模型并保留旧值供诊断。旧 set_ui_theme/set_font_config 改为同一 core 写路径，避免两套来源。单事务比较 expected_revision 并保存；失败不部分写入。restore 是基于当前 revision 的一次新保存，不能回退 revision 数字。历史只保留最近 10 份。

validate 返回字段错误与已计算的色对/对比度；普通文本 4.5:1、大字/控件边界 3:1 作为目标。低对比度给警告供调整，内置预设验收需达标；不以静态色对检查宣称全面可访问性合规。

## MCP 草案

| 工具 | 权限 | 输入与结果 |
| --- | --- | --- |
| get_theme | read | 当前配置/effective/revision、可用能力与参数范围（可选 include_schema） |
| list_theme_presets | read | 有界预设元数据与默认参数 |
| validate_theme | read | patch/base_revision → errors/warnings/effective，不创建窗口 |
| update_theme | write | patch/expected_revision → saved_revision，live_apply=pending/applied/unavailable；配置保存不假称已渲染 |
| restore_theme | write | preset 或历史版本、expected_revision；原子保存 |
| preview_theme | write | patch、base_revision、可选 preview_id/expected_preview_revision、scene/mode → 预览图及元数据 |
| capture_theme_preview | write | preview_id/expected_preview_revision、scene/mode → 图及元数据；scene 是枚举，不接受 URL/文件路径 |
| finish_theme_preview | write | preview_id、expected_preview_revision、action=save/cancel；save 必须 CAS base_revision |

图像工具返回一个 image/png MCP ImageContent（base64）与简短 JSON 文本；若声明 outputSchema，同时提供符合 schema 的 structuredContent 元数据。首版不返回临时路径要求客户端自行读文件。不支持图片的客户端仍能取得元数据，但工具无法保证客户端模型具备视觉能力。

错误码：invalid_argument、revision_conflict、preview_busy、preview_expired、preview_backend_unavailable、profile_mismatch、render_timeout、capture_failed、image_too_large，以及现有授权错误。错误附可重试标志，不附 token。预览图和全量 base64 不写日志。

## 预览生命周期

`created → applying → ready → capturing → ready → committed / cancelled / expired`

- 每个 profile 首版最多一个活动预览，绑定创建它的鉴权会话/短期能力标识；不以“知道 preview_id”替代授权。同一写 token 多客户端的持有者属于同一授权主体，不能声称已实现逐 agent 身份隔离。
- base_revision 为创建时的正式配置版本；preview_revision 每次有效修改单调增加。修改必须携带 expected_preview_revision；并发抓图/修改采用 single flight，争用返回 preview_busy。
- 临时配置仅驻留预览后端内存；默认空闲 10 分钟过期、上限生命周期 30 分钟。取消/过期/退出只释放临时资源，不回写旧正式配置。
- 保存比较 base_revision；用户手动修改过正式配置则冲突，保留候选，读取新配置后显式重建预览。重复 finish(save) 用短期完成记录返回已有结果，避免重试重复提交。
- 断开连接不立即保存；写权限撤销阻止后续操作与提交，窗口本地取消/过期清理独立可用。

## 渲染与截图握手

1. 后端分配 request_id，把归一化配置、config_hash、preview_revision、scene 和明确 light/dark 模式送到固定预览窗口。
2. 前端应用变量/受控布局，等待 document.fonts.ready、本地图片 decode 与布局更新，再经两次 requestAnimationFrame 发 ready。动画/光标闪烁在预览场景禁用。
3. 后端校验 ready 的窗口身份、请求、revision、hash 与当前状态；调用平台原生截图；回调再次核验，版本已变则丢弃图，不标记成功。
4. **两次 rAF 不保证所有平台合成器已刷新**。产品 spike 要采用连续变色/标记像素与内容对照确认新帧；必要时增加平台绘制同步，不能用固定 sleep 宣称保证。
5. 返回 preview/config 版本、主题模式、fixture 版本、scene、逻辑/像素尺寸、缩放因子、字体回退信息、capture_ms 与 captured_at。

默认固定 1280×900 CSS px；仅允许有限 viewport 档位。截图前限制 physical pixel 总量 ≤6MP；PNG 原始数据 ≤2MiB（base64 后约 2.67MiB）；一请求一图。超限优先降采样到约定输出尺寸并注明 output_scale，仍超限返回错误。需验证各客户端消息上限，预算尚未验收。

首版 10 秒 deadline，超时取消原生请求（若平台支持）并以 generation 丢弃迟到回调。图像编码/缩放在允许的平台线程之外完成，store 锁不跨 await。隐藏/最小化窗口若不可可靠绘制则返回状态或使用明确可见预览窗口，不捕获桌面其它内容。

## 保存后的 UI 同步

内嵌写入后发带 revision 的事件；独立进程写入采用前台低频 revision 检查与窗口 focus 时重读补偿（建议 2 秒，后续测量）。事件先订阅再读快照，乱序 revision 丢弃。轮询只读小设置项，不刷新订阅/列表/正文。字体/宽度变更通过阅读段落锚点保持位置；颜色变更只写 CSS 变量。窗口未响应时 update 返回 saved + pending，不阻塞数据库事务。

## 技术证据与未决项

见 [截图验证](capture-feasibility.md) 与 [T1 Tauri 探针](tauri-capture-spike.md)。已在隔离 Tauri example 中实现 Linux 原生适配器，完成 Xvfb 100%/200% 与当前 KDE 原生 Wayland 专项验证；尚未接入产品预览/MCP。Windows/macOS、其他 Wayland compositor、真实 WM 最小化、跨屏/分数缩放清晰度、客户端图片显示、标准组件复用为后续阻断式验收项。hide/show 后必须等待可绘制状态，不能把 show 返回当成绘制完成。截图 logical_size/scale 与预算必须取 WebView 内容区域，不能从带装饰的窗口尺寸推导；需要与 JS viewport/DPR 交叉核验。
