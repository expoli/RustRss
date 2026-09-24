# 本机调度与规模性能验收（2026-09-24）

使用隔离数据库、Xvfb、debug产物；未改系统时钟、网络、显示设置。原始数据见本目录 `scheduler-wallclock-results.json`、`scale-performance-results.json`、`cold-start-results.json`、`folder-query-results.json`、`system-dns-results.json`。

## 墙钟调度

运行 `python3 scripts/verify-scheduler-wallclock.py`，10项断言通过。15分钟档在种库后910.19秒收到首次请求（生产调度轮询有间隔）；关闭档跨915.24秒零请求。5秒在途请求期间手动刷新返回“刷新已在进行中（手动或自动），本次已跳过”，没有第二个HTTP请求。新增1篇落库，正文节点和滚动位置保持。通过真实IPC关闭周期、开启启动刷新并重启，10.19秒后请求。

901.6秒采样：启用实例PSS从165.30降至155.49MiB，关闭实例从162.45降至158.18MiB。采样包含子进程；同时进行其它隔离测试，不能把/tmp可用空间下降约108MiB全部归因于调度实例。不是长时间泄漏排除证据。

## 500源 / 10k文章

运行 `python3 scripts/verify-scale-performance.py`：104,300,544字节库，500个侧栏源，5分钟空闲后PSS 141851KiB（138.53MiB），起始147950KiB。500个本地HTTP源各请求一次，每次人为延迟40ms，刷新3.665秒，最大并发6。条目仍10000篇。

33次脚本滚动/点击往返：p95 10.39ms、最大13.67ms；467次rAF间隔p95 16ms、最大57ms。含一次短停顿，不宣称全程零卡顿，也不是人工原生Wayland手感验证。此/tmp库所有页已驻留，0.969秒启动仅为热数据库。

`python3 scripts/verify-cold-start.py /tmp/rustrss-scale-performance-<本次目录>/fixture.sqlite` 复制到本机SSD缓存目录，对自有文件fsync/fadvise，并以mincore确认0/25464页驻留。冷数据库到500源和首屏条目可交互1.034秒，随后热数据库0.708秒。可执行文件/共享库未清缓存；不是整机断电冷启动，也未计v15首次迁移。退出后需清理脚本打印的单个缓存数据库路径。

## 分组首屏修复

同一SSD隔离库分组100源、2000条，生产 `Store::list_entries` 前200行：

|阶段|打开库|冷页首次查询|100次热缓存均值|
|---|---:|---:|---:|
|修复前|1.974ms|527.794ms|87.802ms|
|v15后|1.613ms|25.592ms|2.678ms|

两次查询前mincore均确认0驻留页。原因：排序索引不含feed_id，筛除其它分组时仍需回文章表。v15在两个现有排序索引尾部加feed_id，排序/游标语义不变，先从索引筛除不匹配源。首次迁移重建两个索引，存在一次性IO成本；上述修复后数字在迁移完成后测量。

`cargo run -p rustrss-core --example scope_entries -- <隔离库> <分组ID>` 分开测打开、首查、热查。单元测试使用生产SQL的EXPLAIN字节码，要求feed_id不从entries表游标读取；修复前真实失败，修复后三档通过，恢复旧索引的负对照三档均被识别。原有分页矩阵继续检查不重不漏。两条计数测试允许更小的feed/read覆盖索引，仍禁止裸扫正文表。

## 系统DNS

移除子进程代理环境，`target/debug/examples/refresh_real <隔离库> http://rustrss-fixture-does-not-exist.invalid/feed` 连续两次经系统解析器失败，状态connection_error，文章零写入；getent返回2。未替换解析器、未改系统DNS。不代表断网桌面的完整交互验收。

## 范围限制

- 本批调度/规模/冷启动使用修复v15前的debug桌面；索引修复由重新构建的core示例、真实文件冷页查询及全量测试验证，不冒充桌面端到端改善。
- 原生Wayland体感、release性能、其它平台、全系统冷启动、MrRSS/Papr/Boke对照仍未验证。
- 本地HTTP延迟夹具不代表公网500站的限流行为；真正外呼白名单抓包尚未完成。
