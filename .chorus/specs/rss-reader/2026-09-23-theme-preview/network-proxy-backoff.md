# 限流退避与应用内代理验收（2026-09-24）

基线：64d65fb已提交推送。本轮改动尚未提交。

## 行为与范围

- schema v13新增feeds.retry_after_at。429/503解析Retry-After秒数或RFC2822兼容的HTTP日期；过去日期视为现在，秒数加法饱和。429无有效头默认60秒；503无有效头保持原刷新节奏。
- collect_jobs读取期限，fetch_jobs在期限内不发网络请求，返回retry_deferred；apply_results不把这次本地拒绝当作新抓取，不改last_fetched_at、上次真实错误或缓存。成功/304清除期限。桌面手动、后台启动/定时与MCP刷新共用管线；不提供手动绕过。
- 设置→订阅新增代理模式（环境/直连/自定义HTTP或HTTPS）、地址、逗号分隔绕过列表。配置保存到network.proxy；拒绝地址内用户名/密码及非代理路径，非法值不覆盖已保存配置。未实现代理凭据管理或SOCKS。
- desktop与MCP读取同一core配置。订阅、发现、全文、RSSHub探测与AI请求使用该配置；新请求使用新设置，已有请求保留原客户端。Fetcher缓存当前配置的连接池；AI按既有生命周期构造客户端。本地MCP预览/控制通道仍no_proxy。
- 前端双语错误和保存状态；地址控件继承现有样式，状态区留固定空间。未加入新依赖或修改产品显示后端变量，未改MCP http.rs。

## 验收证据

结构化记录：[network-proxy-backoff-results.json](network-proxy-backoff-results.json)。

- cargo test --workspace：430 passed / 0 failed，28段，EXIT=0。
- node --test scripts/tests/*.test.cjs：49 passed / 0 failed。
- cargo clippy --workspace --all-targets：EXIT=0，只有原有fulltext/store三条警告。
- 期限测试覆盖v12升级保留订阅、落盘关闭重开、期限内零额外请求、过期恢复、缓存保持、HTTP日期/秒数/无效值/边界。新增期限API前测试编译红，实施后绿；状态与请求数量均独立断言。
- 代理测试采用进程隔离环境变量：HTTP_PROXY、NO_PROXY、407；自定义代理切换、绕过与直连；HTTPS确实发送CONNECT并报告隧道407拒绝；代理不可达时不回退直连。
- AI测试验证请求到达自定义代理并解析响应；MCP测试在服务创建后经另一数据库连接保存代理，再刷新，证明未固定使用启动配置。
- 英文浅色真实桌面34项通过：含生产30秒正文超时、TLS拒绝、429期限内手动零请求、代理保存、实际转发、绕过、直连、重启保留和恢复环境模式。
- 中文深色24项通过：基础空态/错误回归及代理全流程；未重复30秒/TLS扩展段。CSS修正后已重新cargo build再跑此档，并人工检查截图中地址框与状态区。
- 两档各5张截图，均来自隔离Xvfb窗口；I18N.selfTest通过。不是原生Wayland或跨平台验收。

## 复现

```bash
df -h .
cargo build -p rustrss-core --example theme_fixture
cargo build -p rustrss-desktop
/usr/bin/python3 scripts/verify-empty-error-states.py --extended --proxy
/usr/bin/python3 scripts/verify-empty-error-states.py --proxy --locale zh-CN --theme dark
cargo test --workspace
node --test scripts/tests/*.test.cjs
cargo clippy --workspace --all-targets
```

## 未覆盖与限制

- HTTPS CONNECT验证的是隧道建立请求及407拒绝，不声称完成真实公网HTTPS代理成功请求；HTTPS代理自身TLS握手、企业CA、认证代理与SOCKS未验收。
- 环境模式沿用reqwest环境变量行为，不声称读取KDE系统代理配置。
- 未重复旧主题全矩阵、原生Wayland、Windows/macOS或30分钟长测；真实系统DNS、读屏器等历史待办仍独立存在。
- 期限单位为订阅源，未按服务器域名协调不同订阅；全量刷新报告仍把本地延期计入失败并保留明确code，不伪报成功。

## 产物收尾

两档本轮隔离目录、临时数据库/截图/TLS私钥及构建测试日志已清理，JSON观察值保留。历史目录和用户.codex配置未动。
