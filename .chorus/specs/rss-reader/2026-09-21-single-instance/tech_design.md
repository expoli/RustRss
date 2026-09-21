# Tech Design: 单实例锁

- 模块: rss-reader / single-instance
- 日期: 2026-09-21

## 实现

1. 依赖：`tauri-plugin-single-instance`（Tauri 2 官方插件，Cargo.toml 加依赖，`main.rs` builder 链上 `.plugin(...)`——必须在最前注册）。
2. 条件注册：`resolve_db_path() == default_db_path()`（即未用 `RUSTSS_DB`/参数指向别处）时注册插件；否则跳过（开发者多开诊断副本不受锁）。
   - 判定逻辑做成纯函数 `is_default_db(db_path: &Path) -> bool` 进 `rustrss-core/src/paths.rs`（与 `pick_db_path` 同文件、同测试风格），src-tauri 调用。
3. 二次启动回调：拿 `AppHandle` → `get_webview_window("main")` → `show()` + `set_focus()`（托盘隐藏态一并唤出）；回调由插件在已有实例进程内执行，新进程自动退出。
4. 锁是进程级互斥（同 identifier），与库路径无关；非默认库跳过注册即达成共存。

## 测试

- `paths.rs`：`is_default_db` 纯函数测试（默认路径 true；RUSTSS_DB 指向他处 false；参数路径 false）。
- 手动验证清单：默认库双开（二次进程秒退 + 窗口唤出）、RUSTSS_DB 副本共存、托盘隐藏后二次启动唤出。
