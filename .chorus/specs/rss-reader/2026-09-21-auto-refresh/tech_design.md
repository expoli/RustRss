# Tech Design: 自动刷新三件套

- 模块: rss-reader / auto-refresh
- 日期: 2026-09-21（rev2：refresh:start/done 双事件协议；settings key 常量位置定于 src-tauri/commands.rs）

## 架构（分层与现有原则对齐）

```
ui/app.js                 设置项交互（间隔选择/开关）+ OPML 导入后触发抓取
src-tauri/commands.rs     set_refresh_interval / set_refresh_on_start 命令（薄封装）
src-tauri/scheduler.rs    新增：tokio 定时器 + 单 flight（AtomicBool）+ 启动延迟刷新
crates/rustrss-core       settings 表读写 + 刷新核心（复用现有 refresh 管线，零改动）
```

- **core 保持纯库**：只复用现有 settings 表读写原语，不新增刷新相关常量；key 常量与归一化在 src-tauri/commands.rs（与 `KEY_MARK_READ_ON_NAVIGATE` 同级）；调度属于宿主职责，进 src-tauri。
- **单 flight**：`AppState` 增加原子进行中标记（与手动 `refresh_all` 共用），tick 时 CAS 失败即跳过；手动刷新路径同样尊重该标记。
- **性能**：刷新本体复用现有有界并发（concurrency 6）与条件请求（ETag/Last-Modified），WAL 写入；调度器只在 tick 时短暂拿锁读设置，不长期持锁。

## 数据与设置

| key | 默认 | 合法值 |
|-----|------|--------|
| `refresh.interval_minutes` | 30 | `off` 或 `15/30/60/120/360` |
| `refresh.on_start` | true | `true/false` |

写入走现有 `set_setting`；**key 常量与归一化逻辑放 src-tauri/commands.rs**（与 `KEY_MARK_READ_ON_NAVIGATE` 同级；core 只提供 settings 表读写原语，不新增刷新相关常量）；读取带默认值兜底（非法值归默认，白名单归一化模式与 locale/theme 一致）。

## 关键流程

1. **定时**：`scheduler::spawn(state)` 在 setup 阶段启动；每分钟醒一次读 interval → 到点且无进行中刷新 → **先 `app.emit("refresh:start")`** 再触发刷新核心（与手动共用函数）→ 完成后 `app.emit("refresh:done")`。前端 `refresh:start` → setStatus 刷新中提示；`refresh:done` → 静默 `loadAll()`（不重置阅读焦点）。
2. **启动**：同一 scheduler 内，若 on_start 为真，首 tick 延迟 10s 触发一次（同样发 start/done 两事件）。
3. **OPML**：`import_opml` 返回新增 feed id 列表（ImportReport 扩展字段）→ 前端对这批源调 `refresh_feeds(ids)`（新增命令，复用单源刷新管线，期间 setStatus 提示）→ 完成后 `loadAll()`。手动刷新（按钮/`r`）不发事件、沿用现有同步等待模式，避免双提示。
4. **前端**：设置页两个新控件（select + checkbox），变更即保存；`refresh:start` → 状态栏提示、`refresh:done` → 侧栏/列表静默刷新（不重置阅读焦点）。

## 测试

- src-tauri：interval 归一化纯函数测试（off/15/30/60/120/360 → Duration/None，非法归默认）
- 单 flight：标记 CAS 行为单测（不依赖真实网络）
- i18n：新 key 双份齐（既有 key-set 测试自动覆盖）
- 文档同步：README.md 行为描述 + .chorus/specs/rss-reader/spec.md checkbox 回填（进任务 AC）
