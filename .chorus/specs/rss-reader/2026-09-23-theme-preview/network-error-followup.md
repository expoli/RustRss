# 网络故障分类与双语提示（2026-09-24）

上一阶段空态修复已提交推送为 `cb9e35b`。本轮继续 DNS、TLS、超时、429 与错误详情显示，所有实例均使用隔离数据库和测试服务。

## 修改

- core 的 FetchResult::Failed 与 FeedFailure 增加稳定 code，HTTP失败使用http_NNN；超时timeout、连接connection_error、请求network_error、重定向redirect_error、非法URL invalid_url、正文读取body_error、体积超限too_large、解析parse_error。数据库last_status和刷新报告共用同一码，原始error仍保留。
- 修复收到200后正文超时/超限等错误被持久化为http_200的问题。传输status仍保留原HTTP值，失败语义按code记录，不靠诊断语言解析。
- DiscoverError携带code；桌面discover_feed错误返回{code,message}，invoke包装保留code并兼容其它命令的旧字符串错误；MCP发现工具原有外层错误契约保持不变，刷新报告新增code字段。
- 侧栏失败提示、添加订阅的发现/初抓失败使用同一前端翻译映射。新增13组双语文案；未知码保留原详情作为兼容回退，旧http_2xx失败记录显示“响应未完整读取”，不重写旧库。
- DNS与TLS连接失败统一为connection_error，不用字符串猜测具体原因。提示检查地址、网络或服务器证书，不建议关闭证书校验。

未添加网络依赖，reqwest仍为rustls+webpki-roots；未改MCP http.rs、安全校验、产品环境变量、30秒生产请求时限或后台调度策略。

## 证据

### Core与前端回归

- DNS：测试专用reqwest resolver返回NotFound，并断言该resolver确实被调用；无代理、无外网DNS、未改系统DNS。验证connection_error传递、持久化及已有条目/ETag/Last-Modified保持。
- 请求头前超时：测试客户端100ms时限配本地延迟服务器；正文中途超时：测试客户端200ms时限配已发送200响应头但未完成正文的服务器。均验证timeout与缓存凭据保持。短时限只用于单元测试。
- 上述三条在新增code前均红（报告code缺失），修复后通过；Node稳定码映射与结构化IPC错误保留测试先红后绿，另覆盖旧字符串错误、旧http_200与未知码回退。
- 429集成测试：一轮仅一请求，保留2条文章与缓存凭据，后续手动重试恢复ok且无重复条目。
- 自动发现错误DTO测试：code与原始诊断同时保留，无链接对应no_feed_link。
- Rust418 passed / 0 failed（26段）；Node47 passed / 0 failed；clippy仅原有3条core警告。运行i18n自检485 keys一致。

### 重建后真实桌面

`verify-empty-error-states.py --extended` 在隔离Xvfb内驱动真实IPC、HTTP和SQLite，保留上轮全部基础空态/失败/恢复检查，新增：

- 429 + Retry-After:120：记录http_429、双语限流提示，一轮刷新只有一次请求，不立即重试。
- 本地服务器发送200响应头及部分正文后停住：使用未缩短的生产30秒时限，界面释放刷新按钮，数据库记timeout，缓存不丢。
- 临时自签名证书（SAN含127.0.0.1）的本地TLS服务器：RustRss在HTTP发送前拒绝连接，数据库记connection_error，缓存不丢；自动发现同样拒绝，并展示本地化连接提示。
- 英文最终档另加正对照：独立Python客户端只信任该临时证书时可以读取同一TLS服务的有效RSS；此信任配置没有传给RustRss，也没有修改系统信任库。
- 英文档检查所覆盖状态栏/侧栏提示中没有中文诊断文字；中文档检查翻译提示。原始中文诊断仍可在SQLite与日志查看。

每档只拍4张窗口截图，不拍用户桌面。未重复原生Wayland矩阵；源码改变后已cargo build重新嵌入UI才跑探针。

## 本轮结果与产物

- 英文浅色：26项检查通过，生产正文超时30.07秒；中文深色：24项检查通过，30.06秒。英文多出TLS服务正对照与无中文诊断检查。两档issues均为空。
- 最终完整Rust测试与clippy退出码均为0；Rust418/0（26段），Node47/0。
- 结构化观察值保存在 [network-error-followup-results.json](network-error-followup-results.json)。本轮3个隔离目录（含截图、数据库和临时TLS私钥）与测试日志已清理；历史保留目录未动。收尾磁盘余量约51G。

## 复现

```bash
df -h .
cargo build -p rustrss-desktop
cargo build -p rustrss-core --example theme_fixture
/usr/bin/python3 scripts/verify-empty-error-states.py --extended
/usr/bin/python3 scripts/verify-empty-error-states.py --extended --locale zh-CN --theme dark
cargo test -p rustrss-core network_failure_tests
```

扩展脚本需要OpenSSL命令生成临时测试证书，私钥0600，仅在隔离目录内存在；产品未增加OpenSSL依赖。Xvfb、ImageMagick、xdotool、Python websockets同前轮。

## 限制与下一步

- DNS是注入解析器验证，不等于系统解析器/真实网络NXDOMAIN验收；TLS覆盖不受信任自签名证书，未穷举过期、主机名不符、企业代理等。
- Retry-After目前只作为测试响应头：本轮证明无立即重试，**没有实现或验收按该头调整后台调度的退避机制**。代理配置与限流退避可以作为下一独立任务。
- 本轮本地化范围是订阅发现、刷新失败侧栏及初抓提示；全文获取、AI等其它错误路径、MCP人类可读诊断及未知历史码仍可能含原始技术文字。
- Xvfb运行证据不代替Wayland、Windows/macOS；没有再次跑主题全矩阵。
- 本轮修复作为独立提交收口；限流退避与代理场景随后单独处理。
