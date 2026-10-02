---
title: "Tech Design: Android 诊断日志查看与导出"
proposalUuid: a2176c7a-3e23-4f88-85ad-df128a710e60
documentUuid: 9a44f2df-ff5d-4f63-9954-f7163b89dc2e
---

# 技术设计 — Android 诊断日志查看与导出

## 现状与复用（现状优先，引用为证）

| 现状 | 位置 | 复用方式 |
|---|---|---|
| 日志文件命名 `rustrss-YYYYMMDD-HHMMSS[-N].log`、私有 `collect_log_files`/`is_log_file_name`、`prune` | `crates/rustrss-core/src/logging.rs` | 提升为公开 API 的基础；命名守卫直接复用 `is_log_file_name` |
| 导出链路 `blocking_save_file` → mobile `documents::write_text` / desktop `std::fs::write` | `src-tauri/src/commands.rs::export_opml` | `export_log` 完全同构（先例：OPML 导出） |
| `content://` 读写 Kotlin `DocumentsPlugin` | `src-tauri/src/documents.rs` | 原样复用 `write_text` |
| 设置页数据 pane（OPML/备份所在分类） | `ui/index.html` `#pane-data` | 新增一个 `setting-block` |
| i18n key-set 一致性测试 | `ui/i18n.js` + 既有测试 | 新文案 zh-CN/en 双份 |

## 模块契约

### 1. core：`crates/rustrss-core/src/logging.rs` 新增公开 API

```rust
pub struct LogFileInfo { pub name: String, pub bytes: u64,
    pub modified_at: Option<i64>, /* epoch 秒 */ pub seq: u32, pub is_current: bool }

pub fn list_log_files(log_dir: &Path) -> Vec<LogFileInfo>   // 仅 rustrss-*.log，最新在前
pub struct LogTail { pub content: String, pub total_bytes: u64, pub truncated: bool }
pub fn read_log_tail(log_dir: &Path, name: &str, max_bytes: u64) -> Result<LogTail, String>
pub fn read_log_file(log_dir: &Path, name: &str, max_bytes: u64) -> Result<String, String> // 导出用，超限报可读 Err
```

语义与守卫：
- **名称白名单**：`read_*` 先过 `is_log_file_name(name)` 且拒绝含 `/`、`\`、`..` 的名字，
  不合法 → `Err`（在打开任何文件/对话框之前）；防路径穿越；
- `is_current`：`logging::init*` 把本次文件名写入 `static CURRENT: OnceLock<String>`，
  列表时比对（不改任何既有初始化签名）；
- 末尾读取：`seek(len - max_bytes)` 起读；`truncated = len > max_bytes`；起点对齐到下一个
  `\n`（避免半行/半字符），UTF-8 `from_utf8_lossy` 不 panic；
- 并发读当前文件安全（logger 以 append 模式持有，POSIX 语义读不互斥）。

### 2. commands：`src-tauri/src/commands.rs` 三个薄封装（无业务逻辑）

```rust
list_logs() -> R<Vec<LogFileInfo>>                       // dir = paths::logs_dir()
read_log(name: String, max_bytes: Option<u64>) -> R<LogTail>   // 默认 256*1024
export_log(app, name: String) -> R<Option<String>>       // 同 export_opml 结构
```

`export_log`：core 读整文件（上限 16MB，超限→可读 Err，不进对话框）→
`blocking_save_file`（`set_file_name(name)`，**不加扩展名 filter**——Android 按 MIME 映射，
`.log` 非标准 MIME，沿用 opml-picker 教训避免选不了）→ mobile `documents::write_text` /
desktop `std::fs::write` → 返回 `Some(落点)`；用户取消 → `None`。均为 async 命令
（阻塞对话框在非主线程 async 命令里安全——`export_opml` 同款注释）。

### 3. UI：`ui/index.html` `#pane-data` 新增 `setting-block`

- 标题「诊断日志」/ "Diagnostic logs"（`data-i18n`）；
- 结构：刷新按钮 + `<ul>` 文件列表（每行：名称、大小、修改时间、「当前」徽标、查看/导出
  两个按钮）+ 折叠的只读 `<pre>` 查看区（`textContent` 单文本节点，max-height 内部滚动，
  **不做逐行 DOM**——渲染红线只禁列表热路径全量重建，此为按需一次性加载的静态文本）；
- 交互：进设置不预载，点「刷新」/首次展开时 `list_logs`；「查看」→ `read_log` 填充并显示
  截断提示条；「导出」→ `export_log`，成功/取消/失败走与 OPML 相同的状态栏三态；
- 桌面隐藏：块加 `data-desktop-hide` 等价机制（跟随既有 `data-desktop-only` 的反向实现，
  若无现成 mobile-only 属性则按其既有 CSS/JS 模式补一个最小等价物）；
- 大小/时间格式化复用/新增小工具函数，不引第三方。

### 4. i18n（`ui/i18n.js`，zh-CN 与 en 同步补）

`settings.logs.title` / `settings.logs.refresh` / `settings.logs.current` / `settings.logs.view`
/ `settings.logs.export` / `settings.logs.empty` / `settings.logs.truncated` /
`settings.logs.exported` / `settings.logs.exportFailed` / `settings.logs.tooLarge` /
`settings.logs.loadFailed` 等以实现为准，**以 key-set 测试绿为准绳**。

## 错误与边界

| 场景 | 行为 |
|---|---|
| 名称非法（穿越/非日志名） | core `Err`，命令透传，UI 状态栏报错；export 在打开对话框前失败 |
| 文件读取失败（权限/消失） | 可读 Err → UI 三态失败 |
| 导出超 16MB | 可读 Err「文件过大（X MB），上限 16MB」，不写盘 |
| 查看 >256KB | `truncated=true` → 界面截断提示条 |
| 用户取消保存 | `Ok(None)` → 「已取消」提示 |
| logs 目录不存在 | `list_log_files` 返回空列表 → 空态文案 |

## 测试计划

- core（`logging.rs` tests，tempdir）：list 排序/过滤/`is_current`/seq；tail 截断+行边界+
  UTF-8 lossy；名称守卫（`../x.log`、`a.txt`、`rustrss-.log` 边界）；整读上限 Err；
- commands：非法名早退（不开对话框）测试，沿 commands.rs 既有测试风格；
- i18n key-set 测试自动覆盖新 key；
- 全量 `cargo test --workspace` + `cargo clippy --workspace --all-targets` 无新增。

## 性能与红线自查

- 非热路径（用户显式打开设置才触发），无 SQLite 改动、无锁引入、无 store 改动；
- 查看区单 `<pre>` 文本节点一次性填充，不触发列表 DOM 重建红线；同值重复「查看」先比对
  已加载的 (name, truncated, bytes) 短路；
- UI 出现/消失不引起布局跳动：查看区常驻占位（折叠高度），截断提示条为块内固定行。

## 明确不做

删除日志、日志级别 UI、桌面入口、检索/过滤、结构化解析。
