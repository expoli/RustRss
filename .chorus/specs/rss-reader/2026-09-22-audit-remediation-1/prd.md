---
title: PRD: 审计修复批次一（RustRss）
proposalUuid: 06d055c5-a192-4a91-b7ce-5aab34bbd513
documentUuid: a2ec1564-de26-43da-8285-b833893e692f
---

# PRD: 审计修复批次一（RustRss）

## 背景

2026-09-22 对 RustRss（基线 HEAD 87d14ce）做了四角度全量审计：安全 / 核心正确性 / 前端状态机 / 视觉交互（四份报告：/tmp/rustrss-audit-2026-09-22/）。总计 P0×1、P1×3、P2×21、Nit×13。本批次按严重度顺序修复 P0 与全部 P1、依赖安全项与最小 CSP 纵深；其余 P2/Nit 留批次二。

所有条目均带文件:行号证据，其中「feed 抓取无体积上限」由安全组与核心组两个独立模型分别命中（跨组印证）。

## 需求

### FR-1（P0，安全）Windows 外链命令注入修复

`open_external`（src-tauri/src/commands.rs:2188-2197）在 Windows 用 `cmd /C start "" <url>` 启动浏览器：`&` 是合法 URL 字符（query 分隔符），cmd 会把它解析为命令分隔符 → 恶意 feed 的链接 + 用户单击「浏览器打开」即可执行任意命令。
要求：Windows 打开外链必须**不经 cmd/shell 解析**；URL 合法性在打开前硬化（拒绝空白、控制字符与 `"` `<` `>` `|` `^` 反引号）；Linux/macOS 行为不变；带单元测试。

### FR-2（P1）feed 抓取体积闸门

`fetch()`（crates/rustrss-core/src/fetch.rs:105-131）用 `resp.bytes().await` 整包缓冲 + 自动解压，无体积上限；现有 `fetch_bytes_limited()`（fetch.rs:152，Content-Length 预检 + 流式累计双重限制）只用于全文抓取。恶意/病态源可令刷新进程内存耗尽，违反项目红线 #11。
要求：feed 抓取主路径同样受双重限制保护，上限 8 MiB（新常量，独立于全文的 2 MiB）；超限时中止下载，**条目不被写入、etag/last_modified 不被覆盖**（失败态沿用既有失败路径记录 last_status/last_error/last_fetched_at，与 5xx/网络错误等失败模式一致），刷新结果给出可读错误原因；ETag/304、编码、重定向等既有行为不回归。

### FR-3（依赖安全，审计定级 P2，零成本顺带修复）quick-xml 依赖升级

`crates/rustrss-core/Cargo.toml:9` 的 quick-xml 0.38.4 命中 RUSTSEC-2026-0194（二次方耗时）与 RUSTSEC-2026-0195（NsReader 内存耗尽），cargo audit 实测报 2 vulnerabilities。
要求：直接依赖升级到 ≥0.42（Cargo.lock 中已有 0.42.0 供其他 crate 使用）；OPML 导入路径行为不变；`cargo audit` 结果中不再有 quick-xml 条目。

### FR-4（P1）前端 boot 失败死窗修复

`ui/app.js:2626-2634` 初始化 catch 内直接 `return`，跳过其后全部事件绑定（2635-3021）：自绘标题栏三键、全局键盘、后台刷新监听全失效；无系统装饰的窗口无法通过 UI 关闭。
要求：初始化失败时，最小绑定集（标题栏最小化/最大化/关闭）无条件生效，窗口始终可关闭；正常路径行为不变。

### FR-5（P1）前端 toggle 系列局部 patch

`ui/app.js:1860/1878/1891` 的 toggleRead/toggleStar/toggleReadLater 都走 renderReader 全量重建：阅读中按 u/s/l 后正文 scrollTop 归零、AI 面板被重置为 hidden 空体（已生成的摘要丢失）、sanitize+hljs 全量重跑；toggleStar 还整表重建。
要求：这三个动作改为行级/局部 patch——正文滚动位置与 AI 面板内容保持；列表对应行状态即时更新（星标/稍后读视图内因成员资格变化需要移除行时，允许该视图的定向移除）；既有交互语义（标记已读、星标、稍后读、计数）不变。

### FR-6（P2 纵深，安全）最小 CSP

`src-tauri/tauri.conf.json:24` `csp: null`，渲染层无纵深；当前 XSS 防护全靠前端 sanitize。withGlobalTauri 因「UI 无构建链」硬约束必须保留。
要求：配置最小可用 CSP（script-src 'self'；style-src 'self' 'unsafe-inline'；img-src 允许 https/http/data；connect-src ipc: http://ipc.localhost）；前端增加 `securitypolicyviolation` 日志监听（走既有 ui_log）作为机制化回归探针；实机回归：主流程渲染、文章图片、设置页、AI 面板全部正常且零违规日志。

## 非功能要求

- `cargo test --workspace` 全绿；`cargo clippy --workspace` 无新增警告；`cargo audit` 对 quick-xml 清零——**该 workspace 级验收由收口任务 T5 承接**（T1/T2/T3 只各自跑对应 crate，避免批次结束时该要求无人执行）
- core 行为改动必须先有复现测试（项目约定）；前端 UI 改动需 Xvfb 实机证据（截图/日志）
- 不引入 UI 构建链；不新增重型依赖（FR-1 采用零新依赖方案：rundll32）
- 提交为英文 conventional commits；改行为同步 README（如有用户可见行为变化——FR-2 的抓取上限、FR-6 的 CSP 需在 README 或相关文档注明）

## 验收口径（行为级）

1. 恶意/病态 feed 响应超过 8 MiB 时：刷新中止该项、给出可读错误、数据库无写入痕迹
2. Windows 构建下外链打开不经过 cmd：代码路径无 `cmd /C start`；URL 校验函数对含空白/控制字符/`"<>|^`` 的输入拒绝并有单测
3. quick-xml ≥0.42 且 OPML 导入测试全绿、cargo audit 无该包条目
4. 初始化抛错时窗口三键仍可用（最小绑定无条件执行）
5. 阅读中按 u/s/l：正文滚动位置不变、AI 面板内容不被清空、列表行状态即时更新
6. CSP 生效：主流程零 securitypolicyviolation 日志，文章图片/AI 面板/设置页渲染正常

## 范围外

- 其余 P2×20 与 Nit×13（含视觉组 author 字面、footer 路径/i18n）→ 批次二
- 前端竞态类 P2（需要运行时复现设计的）→ 批次二
