# T5：MCP 主题配置与桌面同步

2026-09-23。本阶段完成配置链路，T6 图片预览尚未实现；新改动未提交。

## 接口

| 工具 | 权限 | 行为 |
|---|---|---|
| get_theme | read | 当前配置、明暗有效值、hash、对比度提示、最近 10 份历史元数据与能力；include_schema 可返回 patch 约束 |
| list_theme_presets | read | Clear/Paper/Slate 元数据 |
| validate_theme | read | expected_revision + patch，只校验与计算，不写库 |
| update_theme | write | expected_revision + patch，core CAS 保存；同值零写，不增加版本 |
| restore_theme | write | expected_revision + historical_revision；恢复历史内容并生成新版本 |

写工具沿用 write_enabled 与写凭据要求，非危险工具。HTTP 只读 token 无权修改，轮换后旧 token 立即失效。stdio 使用现有本地信任模型和写开关。审计仅记录主题字段名与 revision，不记录颜色、字体或任意 patch 值。

例如先调用 `get_theme` 取得当前 revision，再把该值用于 `validate_theme` 和 `update_theme`：

```json
{"expected_revision":0,"patch":{"mode":"light","light_preset":"paper","overrides":{"typography":{"read_size":20}}}}
```

省略字段保持原值，overrides 内 null 删除对应覆盖，`overrides:null` 清除全部覆盖；数组整体替换。冲突返回 `revision_conflict` 与 expected/actual_revision，不覆盖其他调用方的更新。实际 revision 必须读取，示例的 0 不是常量。

写结果 detail 的 `saved_revision` 说明已保存；`live_apply=pending` 仅说明内嵌桌面事件已排入队列，`unavailable` 表示没有成功发送宿主通知（独立 stdio 仍可由桌面轮询接收），`unchanged` 表示无变化。三者都不是截图或渲染确认。成功与主题校验错误均提供 structuredContent 和兼容文本 JSON。

`capabilities.preview.available=false`，原因 `preview_backend_not_integrated`。当前没有 preview/capture/finish 工具。

## 同步与性能边界

内嵌 MCP 在释放 store 锁后发送 theme:changed。桌面重新读当前版本，拒绝旧响应，调用 T3 渲染器；不刷新订阅、列表或正文数据。订阅事件失败仍保留轮询。

独立 stdio 与桌面使用同一个数据库时，前台窗口每 2 秒检查一次，focus/visibility 变化也检查；后台窗口停止定时读取，事件可主动唤起检查。单 flight 合并事件突发，退出后丢弃迟到响应。查询只读 settings 的主题行；同版本不解析历史与解析有效主题，不访问文章正文表。没有进行冷盘性能测量，也不承诺严格 2 秒内完成渲染。

## 验收

- `cargo test --workspace`：393 passed / 0 failed。
- `node --test scripts/tests/*.test.cjs`：25 passed / 0 failed。
- `cargo clippy --workspace --all-targets`：通过，仅原有 core 的 3 条告警。
- `cargo build -p rustrss-desktop -p rustrss-mcp`：已重建，前端资源重新嵌入。
- MCP 专项 5 测试：真实 HTTP 调用、发现与结构化内容、校验只读、权限/写开关/轮换、CAS、同值零写、恢复、坏参数/坏配置、审计脱敏。
- 新增 core 2 测试：跨连接 revision 探测不写库；公布的数值边界与 core 校验一致。
- 新增 JS 3 测试：后台跳过、事件强制检查、单 flight、迟到丢弃、错误后恢复。
- 真产物 `scripts/verify-theme-mcp.py`：私有 Xvfb + SQLite。隐藏窗口后通过内嵌事件应用 Slate，重新显示后像素匹配；恢复历史；独立 stdio 保存 Paper，前台轮询应用。两次像素断言、revision=3、文章渲染次数 1→1。启动时故意继承 Wayland 环境，隔离仍成功。
- 固化结果见 [mcp-theme-results.json](mcp-theme-results.json)；原始日志/截图在该文件 output 目录，临时目录不保证长期保留。截图来自测试脚本，不是 MCP 图片响应。

复跑前构建：

```bash
cargo build -p rustrss-desktop -p rustrss-mcp
cargo build -p rustrss-core --example theme_fixture
GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-invalid EGL_PLATFORM=wayland python3 scripts/verify-theme-mcp.py
```

## 未验证与后续

T5 运行证据仅覆盖 Linux Xvfb，未重新验收原生 Wayland、Windows/macOS；T3 的 Wayland 证据不替代 T5。本轮未测试冷盘、长期轮询资源趋势。Chorus 仅同步文档，正式任务未物化，不声称通过正式评审。

下一步 T6：临时预览会话、请求/配置/渲染版本握手、原生截图与图片预算、保存/取消/CAS、失权/超时/退出回收，以及独立 stdio 到桌面的同 profile 桥接。之后补 T4 设置重组与 T7 聚合验收。
