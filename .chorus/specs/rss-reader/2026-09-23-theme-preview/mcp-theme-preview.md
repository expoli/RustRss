# T6：Linux MCP 主题预览闭环

2026-09-23。T5 已提交并推送 `1324b45`；T6 工作区改动尚未提交。

## 使用与结果

桌面应用启用 MCP 服务、写权限和写 token。使用同一数据库的独立 stdio/HTTP 服务可桥接到桌面 loopback MCP，不新增端口；目标 profile 标识不一致时拒绝。未启动/未启用桌面服务时，配置五工具仍可工作，预览返回不可用；Windows/macOS 原生适配器尚未实现。

1. `get_theme` 获取当前 revision、preview capability 和 profile_id；可选 include_schema 查看 patch 约束。
2. `preview_theme` 传 base_revision、patch、scene（overview/article/settings）、mode（light/dark）。返回预览 id、候选版本、配置 hash、一张 MCP image/png 和捕获元数据；临时配置不落库。
3. 继续 `preview_theme` 需携带 preview_id 和 expected_preview_revision；或用 `capture_theme_preview` 重新拍摄指定场景。模式/场景只决定截图，不隐式修改正式主题模式。
4. `finish_theme_preview` 携带 id、候选版本及 action=save/cancel。save 用创建时 base_revision 原子 CAS 保存完整候选（包含删除覆盖）；冲突保留候选供取消或重建。cancel 不写正式配置。最近 16 次完成、10 分钟内的同动作重试返回原结果。

例如（revision 请替换为实际读取值）：

```json
{"base_revision":0,"patch":{"mode":"light","light_preset":"paper","overrides":{"typography":{"read_size":20}}},"scene":"article","mode":"light"}
```

三工具均为普通 write scope，受写开关与写凭据保护。整个工具集为 11 read + 23 write。审计记录字段名、版本和保存/取消动作，不记录 patch 值、凭据、图片或 base64。知道 preview_id 不能越过权限和所有者检查；同一写 token 的持有者属于同一主体，没有逐 agent 身份隔离。

## 生命周期与截图约束

- 每个桌面宿主/profile 同时一个活动预览；默认库已有单实例约束。使用 RUSTSS_DB 显式启动多个宿主时，各宿主独立管理临时会话。
- 候选仅在内存中，空闲 10 分钟/总寿命 30 分钟，后台每 5 秒清理；撤销写权限/轮换凭据也清理。应用退出丢弃内存，绝不自动保存。关闭预览窗口或点击「关闭并取消预览」可本地取消。
- single flight 覆盖修改/截图/finish，争用返回 preview_busy；超时丢弃原生请求与迟到回调。截图失败保留候选并返回 id 供重试/取消；独立桥接发生传输超时、未收到 id 时可关闭本地窗口或等待清理。
- 独立可见窗口，固定本地示例，复用正式 index/CSS/组件/主题渲染器；不读取订阅，不抓桌面或其它窗口，禁止外部导航/远程图片。主窗口的列表、正文与滚动区域不参与临时预览。
- 前端等待字体、本地图片和两次 rAF；后端校验请求 id、候选版本、hash、scene/mode。PNG 中还要匹配请求对应的 8 格像素标记，验证原生画面新鲜度；标记占左上角 32×4 CSS px，并保留在返回图内。
- 默认 1280×900 CSS px；Wayland compositor 约束尺寸时协商 960×640 并重新确认。返回实际 WebView 内容尺寸、像素尺寸、scale_factor、字体请求/计算栈、时间与 display_backend；不把计算字体栈当作逐字形回退证明。
- 一请求一张 PNG；原生分配前检查 ≤6MP，编码期间限制 ≤2MiB，桥接流式限制响应 ≤3MiB；超限拒绝，不降采样，output_scale=1。渲染/截图和桥接工具调用均有 10 秒截止时间。
- save 返回 saved_revision 与 live_apply=pending/unavailable；表示保存/通知状态，不把主窗口通知当作画面确认。截图元数据只确认预览窗口。

## 验收证据

- `cargo test --workspace`：405 passed / 0 failed。
- `node --test scripts/tests/*.test.cjs`：25 passed。
- `cargo clippy --workspace --all-targets`：仅原有 3 条 core 告警，无新增告警。
- i18n：397/397 key 一致。
- 修改 UI 后重新执行 `cargo build -p rustrss-desktop -p rustrss-mcp`，运行的是真实重建产物。
- 新增 12 项 Rust 测试覆盖：候选事务性/同值版本、所有者/TTL 边界、完整替换保存/CAS、HTTP 权限与图片响应、重拍、取消、幂等保存、冲突保留、并发、渲染中凭据轮换、失败保留、同库桥接/错误库拒绝、截止时间取消并释放 single flight、失效清理、ready 元组与像素标记变异。
- http.rs 仅增加请求凭据指纹扩展；回环限制、无/错 token 401、公开且不含订阅数据的 health、只读 token 禁写四条安全测试仍通过。
- 真 HTTP 与真 stdio 子进程：三预设×明暗×三个场景，解码实际 MCP ImageContent，核对 PNG 尺寸、预算、颜色、标记；临时配置零持久化、保存 revision=1、重复保存不增加版本、取消与失权回收、连续关闭/重建。
- Xvfb 100%/200% 均打开真实文章，保存前后 renderReader 次数 1→1。Wayland 无自动点击正文，本轮次数 0→0，不据此声明正文位置验证通过。

具体次数、PNG 大小/耗时、原始输出目录见 [机器结果](mcp-theme-preview-results.json)。这些耗时是本机运行时样本，不是冷盘性能承诺。固定示例图片与用户订阅无关；临时目录可能被系统清理。

复跑：

```bash
cargo build -p rustrss-core --example theme_fixture
cargo build -p rustrss-desktop -p rustrss-mcp
python3 scripts/verify-theme-preview.py
GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-invalid EGL_PLATFORM=wayland python3 scripts/verify-theme-preview.py --scale 2
python3 scripts/verify-theme-preview.py --display wayland
```

Xvfb 分支清除冲突显示环境变量，Wayland 分支保留当前原生会话；产品代码不修改显示后端。200% 点击坐标按缩放计算。Wayland 初次复跑发现 compositor 将内容区约束为 1190×810，旧固定尺寸 ready 检查超时；增加 960×640 协商后同脚本通过，实际 GdkWaylandDisplay/2× 内容尺寸已核对。

## 未验证与下一步

- Windows/macOS 原生截图未实现；GNOME Wayland、跨屏移动、真实最小化与更多 compositor 未验收。
- Wayland 本地取消按钮点击和正文位置未自动验收；Xvfb 已验证。
- 没有真实等待 10/30 分钟做长测，TTL 为时钟边界与清理逻辑测试，撤权回收有真窗口证据；长期 CPU/内存/冷盘待测。
- 已通过真实 HTTP/stdio JSON-RPC 客户端收图并解码，不代表 Claude Desktop 等第三方 GUI 的图片显示和消息上限已验收；留到 T7。
- 字形实际回退未知；暂不支持自定义 viewport、任意 URL、用户文章场景或图片降采样。
- 下一步 T4 设置重组/独立外观页/Aa 面板与历史恢复入口，然后 T7 聚合验收。Chorus 仍为文档镜像，本轮未声称正式评审通过。

### 留存样图

- [Paper 阅读场景（Xvfb）](mcp-preview-paper.png)
- [Slate 设置场景（原生 KDE Wayland）](mcp-preview-wayland.png)

最终复跑：Xvfb 1×/2× 各 25 张，Wayland 24 张；最大 PNG 分别 274192 / 634125 / 338881 字节，均在 2MiB 以内。Wayland 协商 960×640、原生像素 1920×1280。
