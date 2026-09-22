# Tech Design: RSSHub 订阅抓取时解析

## 归一化函数拆分（rsshub.rs）

```rust
/// 存储形态归一（add_feed/导入用）：rsshub:// 保留（三斜杠归一为双斜杠、大写归一为小写）；
/// 官方域 https → rsshub://path；其它原样。不再实例化到镜像。
pub fn canonical_scheme_url(url: &str) -> String

/// 抓取时解析（feed_endpoint 用）：
///   ① rsshub://path → {base}/path；
///   ② 存量官方域 https://rsshub.app/path（含 www）→ {base}/path + 原 query；
///   ③ 其它 URL → 原样。
/// ② 不可省：**未跑归一化的老库存量官方域行必须在抓取时也被改写到当前镜像**，
/// 否则「换镜像即时生效」对这些行失效。因此 resolve_fetch_url 与 canonical_scheme_url
/// 用的是同一套三形态判定，只是作用时机不同（存时归一 vs 抓时解析）。
/// 空/非法 base = 官方默认（含 www → rsshub.app 的归一）。
pub fn resolve_fetch_url(url: &str, base: &str) -> String
```

`normalize_rsshub_url` 退役（已删除，调用点全部迁移到上两者；两套语义并存会漂移）。

## store 改动

1. `add_feed`：`canonical_scheme_url(url)`（去重键 = scheme 形态；**不再读镜像设置**）
2. `feed_endpoint`：读出 url 后 `resolve_fetch_url(&url, &mirror_setting)`——store 内直接
   读 MIRROR_KEY（feed_endpoint 是 Store 方法，走 self.setting），是唯一解析出口
3. `feed_id_by_url`（OPML 导入去重键）：查询键同样走 `canonical_scheme_url`，否则
   「scheme 行 + 官方域 xmlUrl」会被算成新增而实际被 add_feed 判重
4. `update_feed_url`（归一化落库）不变

## 存量整理语义（core `Store::normalize_rsshub_feeds` + src-tauri 命令）

判据与目标 URL 都在 core：`canonical_scheme_url(url) != url` 才改写（把官方域行转
scheme，`rsshub://` 行不动——规范化后等于自身，天然幂等零改动）。预览
（`count_rsshub_normalization_candidates`）与执行共用同一判据，条数不可能漂移；
两个函数都不读镜像设置（归一化只动存储形态）。Tauri 命令只是薄胶水。
UI 按钮文案改「归一化 RSSHub 地址」。

## 添加流程（`rsshub://` 输入可用）

`discover()` 对 scheme 输入**短路**（不发请求）：直接返回 `canonical_scheme_url(input)`
+ `via=Direct`。此后与普通地址同路：`add_feed` 存 scheme → 首次抓取经 `feed_endpoint`
解析到当前镜像。放在 core（而非 Tauri 命令）是为了让界面与未来调用方共用一条路径。

## 兼容性

- 存量已实例化到自建镜像的行：无法识别（不在官方域），保持原样直抓旧镜像——与今天行为一致，不劣化
- OPML 导出：导 scheme 形态（可移植）；导入：canonical 归一（循环稳定，两种写法判重）
- MCP list 的 url 字段：scheme 形态（显示口径 D3）；编辑对话框/tooltip 显示的也是该字段，无需改前端

## 测试

- rsshub.rs 单测：canonical（scheme 保留/三斜杠/大写/官方域转/其它原样/幂等）+
  resolve（scheme→mirror、空 mirror→官方、官方域行→mirror、非 scheme 原样）
- store：add 不实例化（含镜像已配置时）+ 两形态判重 + endpoint 随 mirror 设置变化
  （含存量官方域行）+ 归一化新语义（官方域/三斜杠改写、scheme 行幂等、冲突 skipped、
  预览条数一致）+ feed_id_by_url 归一判重
- discover：scheme 输入零网络 + 添加流程全链（发现 → 落库 → 首次抓取打镜像）
- fetch 端到端：rsshub:// feed 经 feed_endpoint 解析后抓 wiremock 镜像；换镜像重抓新地址
- OPML：导出 scheme 形态、旧官方域 xmlUrl 回导判重
