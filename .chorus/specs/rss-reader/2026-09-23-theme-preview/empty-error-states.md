# 无订阅、空搜索与网络失败状态（2026-09-24）

前置原生Wayland验收已提交推送为4555e42；本轮在其后检查业务空态与失败路径。

## 发现与修复

1. 零订阅时列表只显示“没有未读文章”，没有下一步指引。现在明确无订阅，并提示上方“添加订阅”或设置内导入OPML。
2. 搜索无匹配时复用了“这里还没有文章”。现在明确没有匹配文章，提示更换关键词。清空搜索仍恢复未读视图；不改变库内容。
3. 自动发现成功、初次抓取失败返回RefreshReport.failures时，doAddFeed未检查failures，错误地显示“Added: 0 new articles”。现在显示“订阅已添加，但抓取失败”，带首条错误及刷新重试提示，状态栏使用错误样式；保留已添加订阅，侧栏仍显示失败标记，恢复输入按钮并清空已完成订阅的URL，避免引导重复添加。

只改UI分支与三组双语文案，数据/抓取逻辑和MCP安全路径未改。

## 验证方式

新增 `scripts/verify-empty-error-states.py`：隔离SQLite、Xvfb与真实桌面产物；Python回环HTTP服务器制造自动发现503、发现成功但初抓503、无效RSS、连接中断，再切回有效RSS。无需外网，不切换系统主题/缩放，用户锁屏不影响该隔离显示。

旧产物复现三个问题，脚本EXIT=1，记录三条问题；其余11项通过。Node回归在修复前2失败/1成功（首次抓取错误实得status.added；空态函数尚不存在），修复后全通过。端到端脚本同时守护实际渲染调用路径，避免只测孤立辅助函数。

修复后英文浅色、中文深色各17项断言/4张窗口截图通过，覆盖：

- 无订阅提示、窗口无水平溢出、双语key与目标配色生效。
- 自动发现失败不写订阅、URL保留可重试、按钮恢复。
- 首抓失败保留一个订阅、侧栏失败标记及错误状态文案。
- 有效响应恢复文章并清除错误；无匹配搜索不删缓存，改关键词恢复结果。
- HTTP503、解析失败、连接中断均保留缓存、允许刷新；断网时缓存正文可打开。
- 最后恢复成功，数据库仍仅一篇文章，持久化错误标记清除，无重复插入。

`cargo test --workspace` 413 passed / 0 failed（26段）；Node45 passed / 0 failed；clippy EXIT=0，仅原有3条core警告；i18n运行自检472 keys。已先cargo build重新嵌入UI后才跑运行探针。

## 复现

```bash
df -h .
cargo build -p rustrss-desktop
cargo build -p rustrss-core --example theme_fixture
/usr/bin/python3 scripts/verify-empty-error-states.py
/usr/bin/python3 scripts/verify-empty-error-states.py --locale zh-CN --theme dark
```

依赖Xvfb、xdotool、ImageMagick、distro Python websockets。脚本复用native-settings探针的回环inspector读取器，不调用它的全局桌面设置修改路径。截图由Xvfb窗口获取，不截用户桌面。

机器证据：[旧版复现与修复后结果](empty-error-states-results.json)。

## 未验证与剩余事项

- 本轮是Xvfb真实WebKit/IPC/SQLite验证，不把它当成Wayland/Windows/macOS验收；未穷举语言×明暗×预设矩阵。
- 网络以回环HTTP故障注入覆盖503、解析失败、断连接；DNS、TLS证书错误、长超时、代理、服务器限流未覆盖。
- 原始后端错误详情仍可能包含中文技术文案（英文界面亦如此）；本轮只新增双语状态提示，未重构core错误分类/翻译。
- 未更改普通刷新汇总的既有“失败数量+侧栏标记”提示方式；没有重新设计所有错误弹窗。
- 无产品全量重构，验收完成时新增UI修复及报告尚未commit/push，后续按用户指示单独提交。

本轮4个隔离目录与临时日志/PNG均已清理，仅保留JSON证据；没有pycache误产物，历史暂留目录与Cargo缓存未动。
