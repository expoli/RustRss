---
title: Tech Design: 日志终端镜像
proposalUuid: 5d80bacd-e74f-497b-86bb-3f630e1f574a
documentUuid: bdc61e38-6b74-4c0f-b8ea-f4be9ad1d1b3
---

# Technical Design: 日志终端镜像

## 概览

在 core 的日志实现里增加一个**可选 stderr 镜像**：`FileLogger` 写文件的同时（判定通过后）把同一行写 stderr；是否镜像由启动时算出的布尔值决定。判定逻辑放在 core（纯函数，可测），启用现场在 src-tauri 启动早期。

## Module Contracts

| 契约 | 定义 |
|---|---|
| `logging::mirror_enabled(env_value: Option<&str>, stdout_is_terminal: bool) -> bool` | 纯函数：`Some("1")` → true；`Some("0")` → false；其它（含 `None`、非法值、空串）→ 回退 `stdout_is_terminal` |
| `logging::init(log_dir, level)` | 保持现签名；内部默认不镜像（向后兼容既有调用与测试） |
| `logging::init_with_mirror(log_dir, level, mirror_stderr: bool)`（或等价构造） | 新增：镜像开关显式传入；`init` 以 `false` 委托给它 |
| 镜像行格式 | 与文件行**逐字节一致**：`{本地 RFC3339 毫秒} {LEVEL:<5} {target}: {message}` + 换行 |
| 镜像写入 | `stderr`（`std::io::stderr()`，行级 flush，保证交互式可见性；写失败静默忽略，不影响文件写入与进程） |
| 判定时机 | 进程启动早期一次（`main.rs` 的 `init_logging()` 里计算并传入） |

## 落点

- `crates/rustrss-core/src/logging.rs`：`mirror_enabled` 纯函数；`FileLogger` 增加 `mirror_stderr: bool` 字段与镜像写分支（复用同一份 `format_line` 输出，保证逐字节一致）；`init_with_mirror`。
- `src-tauri/src/main.rs`：`init_logging()` 里 `let mirror = logging::mirror_enabled(std::env::var("RUSTSS_LOG_STDOUT").ok().as_deref(), std::io::stdout().is_terminal());` → `logging::init_with_mirror(&logs_dir, LevelFilter::Info, mirror)`；启动后第一行日志（`本次日志文件: …`）即可见，便于确认镜像已开。
- `README.md`：日志章节补充「从终端运行时日志同时镜像到 stderr；`RUSTSS_LOG_STDOUT=1/0` 强制开关」。

## 行为不变式（必须保持）

1. 文件写入路径/顺序/级别与 target 过滤/scrub/保留策略**完全不变**；
2. 镜像行**只**在判定通过且该行通过级别+target 过滤时输出（与文件行一一对应，不额外输出）；
3. 镜像写入失败（stderr 关闭/EPIPE）不得 panic、不得影响文件写入与业务；
4. 既有测试与调用点不回归（`init` 签名与语义保持）。

## 实施顺序

单任务：core（判定 + 镜像 + 单测）→ src-tauri（启用现场）→ README → 验证（终端/重定向/环境变量三种场景 + 回归）。

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 镜像与文件行不一致（时间戳分别生成 → 同秒不同毫秒） | **同一行字符串**只格式化一次，文件与 stderr 写同一 buffer |
| stderr 写阻塞影响启动/刷新 | 行级写 + 忽略错误；不做重试/缓冲 |
| TTY 检测在容器/CI 误判 | `RUSTSS_LOG_STDOUT=0/1` 显式覆盖；判定函数有真值表单测 |
| 桌面启动噪声 | 非 TTY 默认不镜像（启动器/桌面场景 stdout 不是终端） |
