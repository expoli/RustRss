# 万篇搜索与隔离断网验收（2026-09-24）

## 真实生产分词与搜索

新增`crates/rustrss-core/examples/search_scale.rs`，用core upsert而非直接伪造search_tokens建立10000篇中英文正文，再调用生产Store::search返回200条以内元数据。每个词重复6次，断言结果ID稳定、匹配数量、正文未混入列表。

```sh
cargo build -p rustrss-core --example search_scale
target/debug/examples/search_scale init /tmp/<自建目录>/fixture.sqlite
target/debug/examples/search_scale common /tmp/<自建目录>/fixture.sqlite
```

支持查询：uniqueneedle、common、中文、文、新闻、稀有词、稀、龘、absenttoken。初始化拒绝覆盖已有文件。首轮/tmp为热页：宽泛英文/中文FTS首次约30–31ms、热查22–25ms，单字“文”约1ms；不能用于声明SSD冷启动性能。

复制SQLite到SSD自有缓存文件后，对每个词查询前fsync/fadvise，mincore确认0/10922页驻留（44,736,512字节）。完整原始数据见search-scale-results.json，debug构建：

|查询|结果数|冷页首次|随后5次范围|
|---|---:|---:|---:|
|uniqueneedle|1|4.03ms|0.16–0.23ms|
|common|200|213.64ms|165.08–227.71ms|
|中文|200|234.04ms|178.24–237.17ms|
|文|200|17.94ms|0.96–1.11ms|
|新闻|200|238.18ms|202.14–258.97ms|
|稀有词|1|1.83ms|0.21–0.28ms|
|稀|1|706.04ms|183.70–239.54ms|
|龘|0|660.57ms|181.75–217.27ms|
|absenttoken|0|2.71ms|0.13–0.17ms|

SSD上的重复查询仍明显慢于tmpfs，不能将其统称为“所有数据已热”。已有证据不足以承诺即时搜索，AC继续开放。

历史基线已确认单字计划走title/content_text LIKE，稀疏/无结果需扫完整库；宽泛FTS也有候选元数据成本。v16/v17优化与迁移/语义/计划测试及新冷页数据见`search-unigrams.md`和`search-ranked-results.json`，本节保留旧版反例作为对照。

## 独立网络命名空间

```sh
unshare --user --map-root-user --net sh -c 'ip link set lo up && /usr/bin/python3 scripts/verify-keyboard-search.py --offline'
```

前提：Linux允许非特权用户命名空间、已有重建桌面/theme_fixture、Xvfb、xdotool、Python websockets。只在子进程命名空间启用lo，主机网络不变。脚本检查该命名空间只有lo，应用代理为direct，源地址为192.0.2.1，无法访问外部网络；Inspector经隔离lo取证。

最新16项通过，结果见offline-namespace-results.json：包含真实按键、宽泛搜索200行结果与输入至渲染测量；真实r刷新返回connection_error、随后j仍切换并显示缓存正文且无dialog、10000条缓存未丢。没有更改用户源/设置或KDE授权。

范围：已验证无外网时缓存阅读与失败后继续导航；未逐篇打开10000篇，也未验证外部图片缓存、其它平台/原生Wayland。故保留完整离线AC的这些边界。
