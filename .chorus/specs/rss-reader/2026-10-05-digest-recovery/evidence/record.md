# 日报后台返回防御性恢复 — 2026-10-05

## 改动与边界

- core 状态提供 `generating`；command 以精确 date/scope 查询，历史范围键不再回显为默认 all。
- 重载后由后端标记重建生成占位，3s 无重叠轮询轻量状态；完成事件或标记清除后重新读取成品。日期/范围与 readerToken/轮询序号守护迟到响应。恢复后尚无 job id 时取消按钮禁用，不会发送无效取消。
- `ui.location` sessionStorage 记住日报日期/范围与助手会话。初始化默认页不覆盖旧位置，历史列表加载后恢复；删除的数据或存储拒绝静默回默认页。桌面入口与手机导航共用。
- 发现进程退出会遗留 active_job_id，经主管批准补充显式 `clear_stale_digest_jobs()`：只在 AppState 原生启动调用，与聊天中断恢复同位置；桌面/Android 共用 `lib.rs::run()`。不在 Store::open 或前端重载时清理，避免 MCP 开库/页面刷新误清 live job。
- 进程重启只清中断标记，不续跑 AI，不删除旧成品。sessionStorage 在 ROM 查杀后的存续不保证。真实厂商 ROM/真实 BYOK 生成中查杀未测。

## 机械验证

- `cargo test --workspace`：624 passed / 0 failed / 1 existing ignored。
- `cargo clippy --workspace --all-targets`：通过，无告警。
- `node --test scripts/tests/*.test.cjs`：113 passed / 0 failed。
- `node --check ui/app.js`、`ui/mobile.js`、`scripts/verify-android-digest-recovery.mjs`：通过。
- `cargo build --workspace`：通过，包含嵌入最新 UI 的桌面构建。
- `cargo tauri android build --target x86_64 --debug --apk --ci`：通过。
- Rust 回归：生成标记置位/清除、日期/范围隔离、空报告、多个 profile 查询、文件重开不自动清 marker、显式进程恢复保留成品、digest_get 精确 scope 与双层 generating。
- JS 回归：位置写入/默认页保护、日报与助手恢复、会话发送回执新 id、坏存储/已删数据、生成占位、已知 job 可取消、3s 轮询/漏 done 唤醒、停止/迟到响应/同视图重启轮询失效、初始化次序/导航接线。

## Android 实产物冒烟

隔离 `RustRssT4Review -read-only -port 5582 -no-snapshot`，API 36 / x86_64；不修改已有 emulator-5554 的 release 签名应用/数据。原模拟器覆盖 debug 包被拒绝 `INSTALL_FAILED_UPDATE_INCOMPATIBLE`，故使用隔离只读副本安装；新包安装输出 Success，versionName 0.6.0（安装时间与哈希见下）。

执行：

```bash
ANDROID_SERIAL=emulator-5582 node scripts/verify-android-digest-recovery.mjs \
  .chorus/specs/rss-reader/2026-10-05-digest-recovery/evidence
ANDROID_SERIAL=emulator-5582 DIGEST_ISOLATED_FIXTURE=1 \
  python3 scripts/verify-android-digest-restart.py \
  .chorus/specs/rss-reader/2026-10-05-digest-recovery/evidence
```

1. 真实 Rust IPC：未配置 AI 时生成返回「还没填模型名（设置 → AI）」；前端解除 pending。
2. 后续仅日报 IPC mock（非真实 AI 任务），fixture 模拟持久 active 标记：生成 → 原生 HOME → 返回 → CDP 强制页面重载（确保前端内存丢失）。恢复相同 date/scope 与生成占位，轮询间隔实测约 3001ms。
3. 不发送 digest:done，只清 fixture 标记并提供成品：轮询刷新成品并停止；返回键一次回日报首页（没有重复 reader 历史项）。见 `result.json` 与三张截图。
4. 原生进程恢复：force-stop 隔离应用，离线注入残留 active_job_id + 旧成品（SQLite 主文件和 WAL/SHM 完整捕获，替换前先删除旧边车）；启动真实 AppState，再停进程回读：marker=NULL、Markdown 原样保留、最新日志出现「已清理 1 个中断日报生成标记」。见 `restart-result.json`。这是残留标记夹具，不是假称复现厂商 ROM 查杀。

## 产物

最终重建后安装时间 `2026-10-05 11:03:36`，versionName `0.6.0`，安装输出 Success。未发布 APK，仅 debug 验证。

| 产物 | SHA-256 |
| --- | --- |
| `target/debug/rustrss-desktop` | `079d7c4d51a2fdbe95734fca77a708d4b2bf8462cc21dbf31be3df44d13583dd` |
| `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk` | `9c7b825f65d21dd9ad039a7c1374c4eb829a5da4d95ad0a92d301baf5cf0c51e` |

最后一次 UI 改动后重新构建桌面与 Android 并重跑两项模拟器验证。首次 smoke 脚本调试曾因缺 Page.enable 导致 mock 未安装、随后因 CDP 顶层 const 重声明失败；均已修正，以上 result.json/截图来自最终通过运行。
