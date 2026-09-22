---
title: Tech Design: 审计修复批次一（RustRss）
proposalUuid: 06d055c5-a192-4a91-b7ce-5aab34bbd513
documentUuid: 5b08d056-d0d8-43c6-8139-ab1af50ba018
---

# Technical Design: 审计修复批次一（RustRss）

## 概览

五项互不冲突的小改（1 个安全 P0、2 个 P1 修复、1 个依赖升级、1 个 CSP 纵深），全部为外科手术式修改，不动架构。执行上因「单写者」约束串行跑；依赖关系见任务 DAG（T5 依赖 T4：两者都改 ui/app.js 且都要重建 + Xvfb 回归，必须串行）。

## Module Contracts（供任务 AC 引用）

| 契约 | 定义 |
|---|---|
| `validate_external_url(url: &str) -> Result<String, String>`（src-tauri/src/commands.rs） | trim 后必须 `http://`/`https://` 开头；拒绝任何空白字符、控制字符（U+0000–U+001F、U+007F）、以及 `"` `<` `>` `|` `^` `` ` ``；返回清洗后的 URL 或可读错误。**打开前统一调用**（三平台共用） |
| Windows 启动器 | `rundll32` + 参数 `["url.dll,FileProtocolHandler", url]`，URL 作为独立进程参数（不经 shell）。Linux 保持 `xdg-open`，macOS 保持 `open` |
| `MAX_FEED_BYTES`（crates/rustrss-core/src/fetch.rs） | `pub const MAX_FEED_BYTES: usize = 8 * 1024 * 1024;` feed 抓取上限；全文仍用 fulltext::MAX_BYTES=2MiB |
| 超限错误文案 | 复用现有风格：`页面体积 {n} 超过上限 {max}，已中止下载`（与 fetch_bytes_limited 一致）；该错误作为该源本轮刷新失败上报：**不得写入条目、不得覆盖 etag/last_modified**，失败态沿用既有失败路径（`record_fetch(code, Some(err), None, None)`——etag/last_modified 传 None 走 COALESCE 不丢，last_status/last_error/last_fetched_at 记录失败，与 5xx/网络错误一致） |
| CSP（src-tauri/tauri.conf.json） | `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https: http:; connect-src ipc: http://ipc.localhost; font-src 'self' data:`；withGlobalTauri 保持 true |
| CSP 违规探针（ui/app.js） | `document.addEventListener('securitypolicyviolation', e => log(...))`，经既有 `ui_log` 输出；常驻（日志噪声可忽略，每次违规一行） |
| 最小绑定集（ui/app.js） | 标题栏三键（最小化/最大化/关闭）绑定提取为独立函数，初始化正常路径与 catch 路径都无条件执行 |

## 分项设计

### T1 FR-1 外链注入

- 位置：`src-tauri/src/commands.rs` open_external。
- 改动：新增 `validate_external_url`；Windows 分支 program 从 `cmd` 改 `rundll32`，args 改 `["url.dll,FileProtocolHandler", url]`（删除 `["/C","start",""]`）；macOS/Linux 不变。
- 测试：`#[cfg(test)]` 单测覆盖：合法 http/https 通过并原样返回；`javascript:`/`file:`/`data:` 拒绝；含空格、`\t`、`\n`、`&|^<>\`"` 拒绝；空串拒绝。`cargo test -p rustrss-desktop`（若该 crate 当前无测试目标则新建 `#[cfg(test)] mod tests`；注意 Tauri 命令函数本身不含状态，纯函数可直接测）。
- 说明：`&` 是合法 URL 字符，无法靠白名单拒绝 → 真正修复靠**不经 shell**；白名单是纵深。

### T2 FR-2 体积闸门

- 位置：`crates/rustrss-core/src/fetch.rs`。
- 改动：抽出共享的限流读体辅助（如 `read_body_limited(resp, max_bytes) -> Result<Vec<u8>, String>`），`fetch_bytes_limited` 与 `fetch()` 都走它；`fetch()` 用 `MAX_FEED_BYTES`。
- 保持：ETag/Last-Modified 请求头、304 路径、gzip/br 自动解压（reqwest feature 不动）、错误类型/文案风格。
- 可测性：为测试注入上限，可加 `pub(crate)` 或 `pub` 的 `fetch_with_limit(..., max_bytes)` 供 tests/fetch.rs 使用（小限值 + wiremock 大响应即可断言），`fetch()` 调它并传常量。
- 测试（tests/fetch.rs 追加）：① Content-Length 超限 → 立即报错；② 无 Content-Length 的 chunked 大响应 → 累计超限中止；③ 正常小响应不受影响（现有测试回归）。
- 注意：超限错误必须走「该源失败」路径（scheduler/commands 已有的失败处理），不得部分写入：条目不动、etag/last_modified 不覆盖，last_status/last_error 记录失败态（它是 UI 红点与失败 tooltip 的依据，与其它失败模式一致）；不得把本轮记为成功（不得写 status=ok / last_status='not_modified'）。

### T3 FR-3 quick-xml

- 位置：`crates/rustrss-core/Cargo.toml`（`quick-xml = "0.38"` → `"0.42"`）；`cargo update -p quick-xml` 更新 lock。
- 验证：`cargo build -p rustrss-core`；`cargo test -p rustrss-core`（OPML 相关测试：tests/ 里 opml 用例）；`cargo audit` 输出无 quick-xml 条目。若 0.38→0.42 有 API 差异，最小化适配 `opml.rs`（Reader API 预期兼容，worker 以编译结果为准）。
- 注意：不顺手升其他依赖。

### T4 FR-4/FR-5 前端修复

- FR-4：`ui/app.js` 初始化结构改为「最小绑定集无条件执行」——把标题栏三键的绑定抽成函数，正常路径与 catch 路径都调用；catch 内仍展示错误。
- FR-5：toggleRead/toggleStar/toggleReadLater 改为局部 patch：
  - 列表行：更新该行已读/星标/稍后读的 class 与标记（复用 markViewedRead 的行级 patch 模式，1860 附近）；
  - 阅读区：只更新动作按钮的激活态（如「标为未读/加星标」文案或状态），**不重建正文 DOM**，保持 scrollTop 与 AI 面板 DOM/内容；
  - 星标/稍后读视图内取消标记 → 行离开视图：允许定向移除行（不要整表重建）；
  - 计数（侧栏/列表头）仍要更新（走既有 refreshCounts 的轻量路径或局部更新）。
- 验证（Xvfb 实机，题目给出既有配方）：打开一篇长文 → 滚动到中部 → 按 `u`：正文 scrollTop 不变（日志/脚本断言）、AI 面板（若有内容）不清空；按 `s`/`l` 同理；列表行状态即时变化；侧栏计数正确。
- 风险：AI 面板状态保持需要读现有 renderReader 与 AI 面板渲染逻辑，改动必须外科手术式，不重构。

### T5 FR-6 CSP

- 位置：`src-tauri/tauri.conf.json`（`app.security.csp`）；`ui/app.js` 加违规探针（一行监听）。
- 顺序注意：CSP 生效需要**重建 + 重启**（Tauri 编译期嵌入配置）；因此 T5 在 T4 之后执行，共用同一次 Xvfb 回归。
- 验证：Xvfb 实机跑：默认视图 → 打开含图片的文章（真实库里 cloudflare/security-audit-skill 有图片）→ 设置页 → AI 面板；检查 run.log 无 `securitypolicyviolation`（探针输出）；截图对照（渲染无破坏）。
- **批次收口**：本任务作为批次最后一个任务，额外承接 workspace 级全量验收——`cargo test --workspace` 全绿 + `cargo clippy --workspace` 无新增警告（T1/T2/T3 只跑各自 crate，避免该要求无人执行）。
- 风险：CSP 写错会白屏/sanitize 失效表现异常 → 必须先本地重建并在 Xvfb 验证通过才算完成；若发现某资源类型必须放行，按最小必要原则补进指令并在验证清单注明理由。

## 实施顺序

T1 → T2 → T3 → T4 → T5（严格串行：唯一写者；T5 依赖 T4）。T1/T2/T3 可先跑核心测试，T4/T5 以 Xvfb 实机证据收口；批次最终以 T5 的 workspace 级全量验收（test/clippy）收口。

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| rundll32 在本机（Linux）无法运行时验证 | 白名单纯函数单测 + 代码审查断言「无 cmd 解析」；Windows 运行时验证列入验证清单遗留项 |
| 8 MiB 上限误伤超大 feed | 上限取 8 MiB（正常 feed 的数十倍）；错误信息可读、单源失败不影响其他源 |
| toggle 局部 patch 破坏 AI 面板/滚动 | 实机断言（scrollTop 前后相等 + AI 面板内容保留）+ 现有交互回归面检查 |
| CSP 破坏图片/字体渲染 | 实机回归 + 违规探针日志 + 只放行最小必要来源 |
