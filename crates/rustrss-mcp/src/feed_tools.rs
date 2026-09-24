//! T4 的订阅管理写工具：`subscribe` / `update_feed` / `folder_create` /
//! `folder_rename` / `folder_delete`（危险）/ `unsubscribe`（危险）/ `import_opml` /
//! `export_opml`。
//!
//! 口径（与 tech_design / PRD 的写操作契约对齐，agent 读得到）：
//! - **全部落库走与界面同一批 `Store` 方法**（`add_feed` / `set_feed_custom_title` /
//!   `assign_folder` / `set_feed_refresh_interval` / `add_folder` / `rename_folder` /
//!   `delete_folder` / `remove_feed` / `opml::import|export`）——MCP 与界面只是同一
//!   数据层的两个调用方，不存在第二套订阅表写入逻辑；
//! - **归一化同口径**：自定义标题 trim + 空串 = 清除 + 按字符截断 200（与界面
//!   `normalize_custom_title` 同一规则）；刷新间隔只认白名单 15/30/60/120/360，
//!   JSON `null` = 跟随全局（与界面下拉的 `"global"` 同义）。这两条是 `src-tauri`
//!   的实现，MCP 作为独立 crate 不能跨 crate 调用，所以在这里复刻同一语义并由
//!   测试逐条钉住（见 `interval_whitelist_matches_the_ui_choices` 等）；
//! - **幂等**：`subscribe` 重复 URL（含 `rsshub://` 的三斜杠/大写与官方域等价形态）
//!   返回既有 feed id 且 `ok=true`（`detail.already_subscribed=true`），不报错；
//!   `folder_create` 同名也返回既有 id；
//! - **危险工具**（`unsubscribe` / `folder_delete`）：`dangerous_enabled` 的开关闸门
//!   在 `registry::authorize`（未开 → `dangerous_tool_disabled`，且 `tools/list` 里
//!   根本看不到）；过了闸门后实际执行必须 `confirm: true`（缺 → `confirm_required`），
//!   `dry_run: true` 只算影响面、不落库——且**预览与实际执行共用同一个影响面函数**
//!   （`unsubscribe` 用 `Store::entry_count_for_feed`，`folder_delete` 用
//!   `Store::feed_ids_in_folder`），所以"预览 3 条、执行就真的动 3 条"。
//!
//! `subscribe` 的 url 形态：
//! - `rsshub://path`（含 `rsshub:///` 三斜杠、大写 scheme、官方域）：**不联网**，
//!   按存储形态落库（抓取时才按当前镜像解析，见 `core::rsshub`）；
//! - http(s) 地址：**先查已存在**（省钱省网络），未命中再走 core 的首页自动发现
//!   （`core::discover`：输入本身就是 feed 就用输入地址，否则扫
//!   `<link rel="alternate">`），发现出来的地址落库——与界面「添加订阅」完全同一条
//!   路径（界面先 `discover_feed` 再 `add_feed`，MCP 合并成一步）。

use rustrss_core::discover::{discover, DiscoverError, DiscoveryVia};
use rustrss_core::Store;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::write_contract::{self, ItemResult, WriteOutcome, MAX_BATCH_IDS};
use crate::write_tools::{
    internal_error, ERROR_FEED_NOT_FOUND, ERROR_FOLDER_NOT_FOUND, ERROR_FETCH_FAILED,
    ERROR_INTERNAL, ERROR_INVALID_URL,
};
use crate::RustRssMcp;

/// 每源刷新间隔白名单（分钟）——与界面 `src-tauri` 的 `REFRESH_INTERVAL_CHOICES`
/// / 右键菜单同一张表（15/30/60/120/360）。库里的 `refresh_interval_minutes`
/// 是数字列，界面下拉的 `"global"` 在 MCP 里就是 JSON `null`（跟随全局）。
pub const REFRESH_INTERVAL_CHOICES: [i64; 5] = [15, 30, 60, 120, 360];

/// 自定义标题长度上限（字符）——与界面 `MAX_CUSTOM_TITLE_LEN` 同一个数（200）。
/// 侧栏行与 tooltip 都是单行，超长会把布局撑坏；截断按字符，中文不会截成半个字。
pub const MAX_CUSTOM_TITLE_LEN: usize = 200;

/// OPML 输入体积上限（8 MiB）：导入的是订阅清单，不是备份文件。给出上限是为了
/// 「传错文件（比如整个数据库）」时快速失败，而不是把几 GB 读进内存再解析。
pub const MAX_OPML_BYTES: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------- 入参

/// `subscribe` 的参数
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubscribeParams {
    /// 订阅地址：站点首页或 feed 地址（首页会自动发现 feed），或 `rsshub://path`
    pub url: String,
}

/// `update_feed` 的参数：`feed_id` + tri-state patch。
///
/// 三个字段的「不动 / 清除 / 设值」写成两层 `Option`（JSON 键缺省 = 不动；
/// `null` = 清除/跟随全局；有值 = 设值），语义与界面编辑对话框一致。
/// 只给了 `feed_id`、一个字段都不传时**报错**（不是静默空操作）：写工具的空调用
/// 多半是调用方拼错了参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateFeedParams {
    /// 订阅源 id（来自 list_feeds / subscribe）
    pub feed_id: i64,
    /// 自定义标题：键缺省 = 不动；`""`（或纯空白）= 清除（显示回退源站名）；
    /// 其它 = 设自定义标题（trim + 截断到 200 字符）。
    #[serde(default)]
    pub custom_title: Option<String>,
    /// 分组：键缺省 = 不动；`null` = 移出到未分组；数字 = 移入该分组。
    #[serde(default, deserialize_with = "double_option")]
    pub folder_id: Option<Option<i64>>,
    /// 每源刷新间隔（分钟）：键缺省 = 不动；`null` = 跟随全局档；
    /// 数字必须命中白名单 15/30/60/120/360（否则 `invalid_argument`）。
    #[serde(default, deserialize_with = "double_option")]
    pub refresh_interval_minutes: Option<Option<i64>>,
}

/// `folder_create` 的参数
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FolderCreateParams {
    /// 分组名（trim 后不能为空；同名的分组会返回既有 id，不报错）
    pub name: String,
}

/// `folder_rename` 的参数
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FolderRenameParams {
    /// 分组 id（来自 list_folders）
    pub folder_id: i64,
    /// 新名字（trim 后不能为空；与其它分组重名时 `invalid_argument`）
    pub name: String,
}

/// `folder_delete` 的参数（危险工具）
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FolderDeleteParams {
    /// 分组 id（来自 list_folders）
    pub folder_id: i64,
    /// 危险操作确认：实际删除必须显式 `true`（缺省/`false` → `confirm_required`）；
    /// `dry_run: true` 的预览不需要它
    pub confirm: Option<bool>,
    /// `true` = 只返回影响面（将移出到未分组的订阅数），不落库
    pub dry_run: Option<bool>,
}

/// `unsubscribe` 的参数（危险工具）
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UnsubscribeParams {
    /// 订阅源 id（来自 list_feeds）
    pub feed_id: i64,
    /// 危险操作确认：实际删除必须显式 `true`（缺省/`false` → `confirm_required`）；
    /// `dry_run: true` 的预览不需要它
    pub confirm: Option<bool>,
    /// `true` = 只返回影响面（将级联删除的条目数），不落库
    pub dry_run: Option<bool>,
}

/// `import_opml` 的参数：`path` 与 `content` 二选一
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImportOpmlParams {
    /// 本地 OPML 文件路径（UTF-8 文本，≤8 MiB）
    pub path: Option<String>,
    /// OPML 文本内容（与 `path` 二选一；都给或都不给 = `invalid_argument`）
    pub content: Option<String>,
}

/// 两层 `Option` 的 JSON 语义：键缺省走 `#[serde(default)]`（外层 `None` = 不动），
/// 显式 `null` 走到这里（内层 `None` = 清除/跟随全局）。
///
/// 与 `src-tauri` 的 `folder_patch` 是同一个技巧——直接用 `Option<Option<T>>` 会把
/// `null` 吃成「不动」，那样就没有写法能表达「移出分组」。
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

// ---------------------------------------------------------------- 归一化（与界面同口径）

/// 自定义标题归一化：trim 后为空 = 清除（`None`，显示回退源站名）；否则按字符截断。
pub fn normalize_custom_title(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_CUSTOM_TITLE_LEN).collect())
}

/// 刷新间隔白名单校验：不在表里就报错（**不**静默回默认档——这是调用方的显式选择，
/// 猜错档位（比如把 3600 当 360）比报错更糟；与界面 `normalize_feed_refresh_interval`
/// 的口径一致，界面侧的非法值同样报错而不是回退）。
fn check_refresh_interval(minutes: i64) -> Result<(), String> {
    if REFRESH_INTERVAL_CHOICES.contains(&minutes) {
        return Ok(());
    }
    let choices: Vec<String> = REFRESH_INTERVAL_CHOICES.iter().map(i64::to_string).collect();
    Err(format!(
        "不支持的刷新间隔 {minutes} 分钟（可选 null=跟随全局，或 {}）",
        choices.join("/")
    ))
}

/// 订阅地址的结构校验（`rsshub://` 形态在调用前已由 `rsshub::is_scheme_url` 分流）。
///
/// 只挡「明显不是可抓取的 http(s) 地址」的输入（scheme 不对 / 没有 host / 带空白与
/// 控制字符）；能不能真的抓到由发现/抓取路径回答（那是网络问题，报 `fetch_failed`）。
/// 不引 `url` crate 到 `rustrss-mcp`：这里要判断的是「像不像可订阅地址」，而不是
/// 完整 URL 语法，core 的解析/抓取路径才是权威。
fn validate_web_url(url: &str) -> Result<(), String> {
    // scheme 大小写不敏感（`HTTP://` 也是合法 URL）；这里只判「像不像可订阅地址」
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(format!(
            "只支持 http/https 或 rsshub:// 地址，收到 {url:?}（rsshub 用 rsshub://path 形态）"
        ));
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("订阅地址不能包含空白或控制字符".to_string());
    }
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or_default();
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() {
        return Err(format!("订阅地址缺少主机名: {url:?}"));
    }
    Ok(())
}

/// 订阅输入的两种形态
#[derive(Debug, PartialEq, Eq)]
enum SubscribeTarget {
    /// `rsshub://path`：不联网发现，直接按存储形态落库
    Scheme(String),
    /// http(s) 地址：先查已存在，未命中再走首页自动发现
    Web(String),
}

/// 解析 `subscribe` 的 url（纯函数，便于单测）：非法输入返回人读原因。
fn parse_subscribe_target(raw: &str) -> Result<SubscribeTarget, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("订阅地址不能为空".to_string());
    }
    // rsshub 的两种等价写法（`rsshub://path` 与官方域 `https://rsshub.app/path`）
    // 归一后都是**抽象身份**：不联网发现、也不去访问官方实例（换镜像零迁移的前提）。
    let canonical = rustrss_core::rsshub::canonical_scheme_url(url);
    if rustrss_core::rsshub::is_scheme_url(url) || canonical != url {
        return Ok(SubscribeTarget::Scheme(canonical));
    }
    validate_web_url(url)?;
    Ok(SubscribeTarget::Web(url.to_string()))
}

/// 发现依据 → 给 agent 看的短标签（core 的枚举序列化是 PascalCase，这里统一 snake_case）
fn via_label(via: DiscoveryVia) -> &'static str {
    match via {
        DiscoveryVia::Direct => "direct",
        DiscoveryVia::LinkType => "link_type",
        DiscoveryVia::LinkSuffix => "link_suffix",
    }
}

// ---------------------------------------------------------------- subscribe

impl RustRssMcp {
    /// 订阅一个源（幂等；首页自动发现）
    pub async fn subscribe_json(&self, p: &SubscribeParams) -> String {
        let target = match parse_subscribe_target(&p.url) {
            Ok(t) => t,
            Err(message) => return WriteOutcome::failed_with(ERROR_INVALID_URL, message).to_json(),
        };

        match target {
            // ① rsshub:// 形态：不联网（scheme 与官方域归一到同一个存储键）
            SubscribeTarget::Scheme(canonical) => self.register_subscription(&canonical, None, None),
            // ② http(s)：先查已存在（幂等快路径，省掉一次网络往返）
            SubscribeTarget::Web(url) => {
                if let Some(id) = match self.feed_id_by_url(&url) {
                    Ok(v) => v,
                    Err(e) => return internal_error(&e),
                } {
                    return already_subscribed(self, id, None);
                }
                let fetcher = match self.fetcher() {
                    Ok(f) => f,
                    Err(e) => return WriteOutcome::failed_with(ERROR_INTERNAL, e).to_json(),
                };
                match discover(fetcher, &url).await {
                    Ok(found) => {
                        self.register_subscription(
                            &found.feed_url,
                            Some(&url),
                            Some(via_label(found.via)),
                        )
                    }
                    Err(DiscoverError::Fetch { url, error, .. }) => WriteOutcome::failed_with(
                        ERROR_FETCH_FAILED,
                        format!("抓取/发现 {url} 失败：{error}（网络问题可重试，不是参数错）"),
                    )
                    .to_json(),
                    Err(DiscoverError::NoFeedLink { url, detail }) => WriteOutcome::failed_with(
                        ERROR_INVALID_URL,
                        format!("{url} 既不是 feed，页面里也没发现 feed 链接：{detail}"),
                    )
                    .to_json(),
                }
            }
        }
    }

    fn feed_id_by_url(&self, url: &str) -> Result<Option<i64>, String> {
        self.with_store(|s| s.feed_id_by_url(url))
            .map_err(|e| e.to_string())
    }

    /// 落库并按「新建 / 已有」分别回执。
    ///
    /// 查存在性与新增在**同一把锁**里做完（`Store::add_feed` 自身也幂等），所以并发的
    /// 两次 `subscribe` 不会都报「新建」；发现出来的地址已经是订阅（两个首页指向同一个
    /// feed）也走幂等回执。
    fn register_subscription(
        &self,
        url: &str,
        discovered_from: Option<&str>,
        via: Option<&str>,
    ) -> String {
        let created = self.with_store(|s| -> Result<(i64, bool), String> {
            if let Some(id) = s.feed_id_by_url(url).map_err(|e| e.to_string())? {
                return Ok((id, false));
            }
            let id = s.add_feed(url, None).map_err(|e| e.to_string())?;
            Ok((id, true))
        });
        let (id, created) = match created {
            Ok(v) => v,
            Err(e) => return internal_error(&e),
        };
        if !created {
            return already_subscribed(self, id, discovered_from);
        }
        let row = match self.with_store(|s| s.feed_row(id)) {
            Ok(Some(row)) => row,
            Ok(None) => {
                return WriteOutcome::failed_with(
                    ERROR_INTERNAL,
                    format!("订阅 #{id} 落库后读不回来（库状态异常）"),
                )
                .to_json()
            }
            Err(e) => return internal_error(&e.to_string()),
        };
        let mut detail = json!({
            "feed_id": id,
            "title": row.title,
            "url": row.url,
            "already_subscribed": false,
        });
        if let Some(from) = discovered_from {
            detail["discovered_from"] = json!(from);
        }
        if let Some(via) = via {
            detail["via"] = json!(via);
        }
        WriteOutcome::done(1, vec![ItemResult::ok(id)], false)
            .with_detail(detail)
            .to_json()
    }
}

/// 已订阅（幂等路径）的统一回执：`ok=true` + `affected=0` + `results=[ok(id)]`。
/// **不报错**是契约的一部分：agent 重复调用 `subscribe` 不该以为自己做错了。
fn already_subscribed(server: &RustRssMcp, feed_id: i64, discovered_from: Option<&str>) -> String {
    let row = match server.with_store(|s| s.feed_row(feed_id)) {
        Ok(Some(row)) => row,
        Ok(None) => {
            return WriteOutcome::failed_with(
                ERROR_FEED_NOT_FOUND,
                format!("订阅 #{feed_id} 不存在；先用 list_feeds 取 id"),
            )
            .to_json()
        }
        Err(e) => return internal_error(&e.to_string()),
    };
    let mut detail = json!({
        "feed_id": feed_id,
        "title": row.title,
        "url": row.url,
        "already_subscribed": true,
        "affected_hint": "该地址（含 rsshub 等价形态）已经在库里，未新建、未联网抓取",
    });
    if let Some(from) = discovered_from {
        detail["discovered_from"] = json!(from);
    }
    WriteOutcome::done(0, vec![ItemResult::ok(feed_id)], false)
        .with_detail(detail)
        .to_json()
}

// ---------------------------------------------------------------- update_feed

impl RustRssMcp {
    /// 改单个订阅源的三个字段（tri-state：不传 = 不动）
    pub fn update_feed_json(&self, p: &UpdateFeedParams) -> String {
        // ① 先全部归一化/校验，再动库：任何一项非法时一个字段都不写（不留半套修改），
        //    与界面 `set_feed_config_core` 同一纪律。
        let custom_title: Option<Option<String>> = p.custom_title.as_deref().map(normalize_custom_title);
        let folder: Option<Option<i64>> = p.folder_id;
        let interval: Option<Option<i64>> = match p.refresh_interval_minutes {
            None => None,
            Some(minutes) => match minutes {
                None => Some(None), // 显式 null = 跟随全局
                Some(n) => match check_refresh_interval(n) {
                    Ok(()) => Some(Some(n)),
                    Err(message) => {
                        return WriteOutcome::failed_with(
                            write_contract::ERROR_INVALID_ARGUMENT,
                            message,
                        )
                        .to_json()
                    }
                },
            },
        };
        if custom_title.is_none() && folder.is_none() && interval.is_none() {
            return WriteOutcome::failed_with(
                write_contract::ERROR_INVALID_ARGUMENT,
                "至少要给一个要改的字段（custom_title / folder_id / refresh_interval_minutes）——空 patch 不做任何事，多半是参数拼错了",
            )
            .to_json();
        }

        // ② 范围错误码：feed 必须先存在；给了具体分组时分组也要存在
        //    （归一化已完成，这里的 id 是最终落库值）
        match self.with_store(|s| s.feed_row(p.feed_id)) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return WriteOutcome::failed_with(
                    ERROR_FEED_NOT_FOUND,
                    format!("订阅源 #{} 不存在；先用 list_feeds 取 id", p.feed_id),
                )
                .to_json()
            }
            Err(e) => return internal_error(&e.to_string()),
        }
        if let Some(Some(folder_id)) = folder {
            match self.with_store(|s| s.list_folders()) {
                Ok(folders) => {
                    if !folders.iter().any(|(id, _)| *id == folder_id) {
                        return WriteOutcome::failed_with(
                            ERROR_FOLDER_NOT_FOUND,
                            format!("分组 #{folder_id} 不存在；先用 list_folders 取 id"),
                        )
                        .to_json();
                    }
                }
                Err(e) => return internal_error(&e.to_string()),
            }
        }

        // ③ 落库（与界面同一批 store 方法；写完回读同一行给 agent）
        let changed: Vec<&str> = [
            custom_title.is_some().then_some("custom_title"),
            folder.is_some().then_some("folder_id"),
            interval.is_some().then_some("refresh_interval_minutes"),
        ]
        .into_iter()
        .flatten()
        .collect();

        let updated = self.with_store(|s| -> Result<rustrss_core::store::FeedRow, String> {
            if let Some(title) = custom_title.as_ref() {
                s.set_feed_custom_title(p.feed_id, title.as_deref())
                    .map_err(|e| e.to_string())?;
            }
            if let Some(folder_id) = folder {
                s.assign_folder(p.feed_id, folder_id)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(minutes) = interval {
                s.set_feed_refresh_interval(p.feed_id, minutes)
                    .map_err(|e| e.to_string())?;
            }
            s.feed_row(p.feed_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("订阅源 #{} 不存在", p.feed_id))
        });
        let updated = match updated {
            Ok(row) => row,
            Err(e) => return internal_error(&e),
        };

        WriteOutcome::done(1, vec![ItemResult::ok(p.feed_id)], false)
            .with_detail(json!({
                "feed_id": updated.id,
                "title": updated.title,
                "custom_title": updated.custom_title,
                "source_title": updated.source_title,
                "folder_id": updated.folder_id,
                "refresh_interval_minutes": updated.refresh_interval_minutes,
                "updated_fields": changed,
            }))
            .to_json()
    }
}

// ---------------------------------------------------------------- folder CRUD

impl RustRssMcp {
    /// 新建分组（同名返回既有 id：与界面 `add_folder` 的幂等语义一致）
    pub fn folder_create_json(&self, p: &FolderCreateParams) -> String {
        let name = p.name.trim();
        if name.is_empty() {
            return WriteOutcome::failed_with(
                write_contract::ERROR_INVALID_ARGUMENT,
                "分组名不能为空（trim 后至少一个字符）",
            )
            .to_json();
        }
        let created = self.with_store(|s| -> Result<(i64, bool), String> {
            let existing = s
                .list_folders()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|(_, n)| n == name);
            match existing {
                Some((id, _)) => Ok((id, false)),
                None => {
                    let id = s.add_folder(name).map_err(|e| e.to_string())?;
                    Ok((id, true))
                }
            }
        });
        let (id, created) = match created {
            Ok(v) => v,
            Err(e) => return internal_error(&e),
        };
        WriteOutcome::done(i64::from(created), vec![ItemResult::ok(id)], false)
            .with_detail(json!({
                "folder_id": id,
                "name": name,
                "created": created,
                "already_exists": !created,
            }))
            .to_json()
    }

    /// 重命名分组（`folder_not_found` / 空名 / 重名都有显式错误码）
    pub fn folder_rename_json(&self, p: &FolderRenameParams) -> String {
        let name = p.name.trim();
        if name.is_empty() {
            return WriteOutcome::failed_with(
                write_contract::ERROR_INVALID_ARGUMENT,
                "分组名不能为空（trim 后至少一个字符）",
            )
            .to_json();
        }
        // 存在性与重名都在这里判（不靠解析 store 的错误文案）：重名给 `invalid_argument`，
        // 与界面「文件夹「X」已存在」的可读提示同一个码。store 侧仍有自己的唯一性判断，
        // 两者之间即使有竞态窗口，store 也会拒掉多写的一行。
        let folders = match self.with_store(|s| s.list_folders()) {
            Ok(folders) => folders,
            Err(e) => return internal_error(&e.to_string()),
        };
        if !folders.iter().any(|(id, _)| *id == p.folder_id) {
            return WriteOutcome::failed_with(
                ERROR_FOLDER_NOT_FOUND,
                format!("分组 #{} 不存在；先用 list_folders 取 id", p.folder_id),
            )
            .to_json();
        }
        if folders
            .iter()
            .any(|(id, existing)| *id != p.folder_id && existing == name)
        {
            return WriteOutcome::failed_with(
                write_contract::ERROR_INVALID_ARGUMENT,
                format!("分组「{name}」已存在（重命名会撞唯一约束）"),
            )
            .to_json();
        }
        if let Err(e) = self.with_store(|s| s.rename_folder(p.folder_id, name)) {
            return internal_error(&e.to_string());
        }
        WriteOutcome::done(1, vec![ItemResult::ok(p.folder_id)], false)
            .with_detail(json!({ "folder_id": p.folder_id, "name": name }))
            .to_json()
    }

    /// 删分组（危险）：删组不删订阅——组内订阅移出到未分组（与界面同一语义），
    /// 同时清掉侧栏折叠状态里的孤儿 id（界面 `delete_folder` 命令也做这一步）。
    pub fn folder_delete_json(&self, p: &FolderDeleteParams) -> String {
        let dry_run = p.dry_run.unwrap_or(false);
        // 预览是只读的，不需要 confirm；真正删才要（缺省不是"默认同意"）
        if !dry_run {
            if let Err(e) = write_contract::require_confirm(p.confirm) {
                return WriteOutcome::rejected(e).to_json();
            }
        }
        match self.with_store(|s| s.list_folders()) {
            Ok(folders) => {
                if !folders.iter().any(|(id, _)| *id == p.folder_id) {
                    return WriteOutcome::failed_with(
                        ERROR_FOLDER_NOT_FOUND,
                        format!("分组 #{} 不存在；先用 list_folders 取 id", p.folder_id),
                    )
                    .to_json();
                }
            }
            Err(e) => return internal_error(&e.to_string()),
        }

        // 影响面：组内订阅数（**预览与实际共用这一个函数**）
        let impact = self.with_store(|s| folder_impact(s, p.folder_id));
        let feed_ids = match impact {
            Ok(ids) => ids,
            Err(e) => return internal_error(&e),
        };
        let (sample, truncated) = bounded_sample(&feed_ids);
        let detail = json!({
            "folder_id": p.folder_id,
            "feeds_affected": feed_ids.len(),
            "feed_ids": sample,
            "feed_ids_truncated": truncated,
            "note": "删组不删订阅：组内订阅会移出到未分组（folder_id=null）",
        });

        if dry_run {
            return WriteOutcome::preview(feed_ids.len() as i64, vec![ItemResult::ok(p.folder_id)])
                .with_detail(detail)
                .to_json();
        }

        // 落库：同一把锁里「数一遍 + 删组 + 清折叠残留」，所以 affected 与实际一致
        let applied = self.with_store(|s| -> Result<i64, String> {
            let n = folder_impact(s, p.folder_id)?.len() as i64;
            s.delete_folder(p.folder_id).map_err(|e| e.to_string())?;
            let remaining: Vec<i64> = s
                .collapsed_folders()
                .into_iter()
                .filter(|id| *id != p.folder_id)
                .collect();
            s.set_collapsed_folders(&remaining)
                .map_err(|e| e.to_string())?;
            Ok(n)
        });
        match applied {
            Ok(affected) => WriteOutcome::done(affected, vec![ItemResult::ok(p.folder_id)], false)
                .with_detail(detail)
                .to_json(),
            Err(e) => internal_error(&e),
        }
    }
}

/// 删组的影响面：组内订阅 id（与界面「删组不删订阅」的语义一致——订阅行保留、只移出分组）
fn folder_impact(store: &Store, folder_id: i64) -> Result<Vec<i64>, String> {
    store
        .feed_ids_in_folder(folder_id)
        .map_err(|e| e.to_string())
}

/// 逐项目标的有界采样（信封的 `results` / detail 里的 id 列表都不该倒出上万条）
fn bounded_sample(ids: &[i64]) -> (Vec<Value>, bool) {
    let truncated = ids.len() > MAX_BATCH_IDS;
    let sample = ids
        .iter()
        .take(MAX_BATCH_IDS)
        .map(|id| json!(id))
        .collect();
    (sample, truncated)
}

// ---------------------------------------------------------------- unsubscribe

impl RustRssMcp {
    /// 退订（危险）：删源 + 级联删条目；`dry_run` 返回将删除的条目数。
    pub fn unsubscribe_json(&self, p: &UnsubscribeParams) -> String {
        let dry_run = p.dry_run.unwrap_or(false);
        if !dry_run {
            if let Err(e) = write_contract::require_confirm(p.confirm) {
                return WriteOutcome::rejected(e).to_json();
            }
        }
        let row = match self.with_store(|s| s.feed_row(p.feed_id)) {
            Ok(Some(row)) => row,
            Ok(None) => {
                return WriteOutcome::failed_with(
                    ERROR_FEED_NOT_FOUND,
                    format!("订阅源 #{} 不存在；先用 list_feeds 取 id", p.feed_id),
                )
                .to_json()
            }
            Err(e) => return internal_error(&e.to_string()),
        };

        if dry_run {
            let entries = match self.with_store(|s| s.entry_count_for_feed(p.feed_id)) {
                Ok(n) => n,
                Err(e) => return internal_error(&e.to_string()),
            };
            return WriteOutcome::preview(entries, vec![ItemResult::ok(p.feed_id)])
                .with_detail(json!({
                    "feed_id": p.feed_id,
                    "title": row.title,
                    "url": row.url,
                    "entries_affected": entries,
                    "note": "退订会删除该源及其全部条目（外键级联）；确认后传 confirm: true 执行",
                }))
                .to_json();
        }

        // 数一遍影响面 + 删源在同一把锁里（`Store::entry_count_for_feed` 就是预览用的
        // 那一个函数，`remove_feed` 的级联删除数由外键保证与之一致）
        let applied = self.with_store(|s| -> Result<i64, String> {
            let entries = s
                .entry_count_for_feed(p.feed_id)
                .map_err(|e| e.to_string())?;
            s.remove_feed(p.feed_id).map_err(|e| e.to_string())?;
            Ok(entries)
        });
        match applied {
            Ok(entries) => WriteOutcome::done(entries, vec![ItemResult::ok(p.feed_id)], false)
                .with_detail(json!({
                    "feed_id": p.feed_id,
                    "title": row.title,
                    "url": row.url,
                    "entries_affected": entries,
                }))
                .to_json(),
            Err(e) => internal_error(&e),
        }
    }
}

// ---------------------------------------------------------------- OPML

impl RustRssMcp {
    /// 导入 OPML（`path` 或 `content` 二选一）：复用 core 的 `opml::import`
    /// （与界面「导入 OPML」同一条实现，`xmlUrl` 去重、嵌套分组压平成 `父/子`）。
    pub fn import_opml_json(&self, p: &ImportOpmlParams) -> String {
        let content = match (
            p.path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            p.content.as_deref().filter(|s| !s.trim().is_empty()),
        ) {
            (Some(_), Some(_)) => {
                return WriteOutcome::failed_with(
                    write_contract::ERROR_INVALID_ARGUMENT,
                    "path 与 content 只能二选一（都给了不知道该导哪份）",
                )
                .to_json()
            }
            (None, None) => {
                return WriteOutcome::failed_with(
                    write_contract::ERROR_INVALID_ARGUMENT,
                    "需要 path（本地文件路径）或 content（OPML 文本）二选一",
                )
                .to_json()
            }
            (Some(path), None) => match std::fs::read_to_string(path) {
                Ok(text) => {
                    if text.len() > MAX_OPML_BYTES {
                        return WriteOutcome::failed_with(
                            write_contract::ERROR_INVALID_ARGUMENT,
                            format!("{path} 超过 {MAX_OPML_BYTES} 字节上限；确认是不是传错了文件"),
                        )
                        .to_json();
                    }
                    text
                }
                Err(e) => {
                    return WriteOutcome::failed_with(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        format!("读取 {path} 失败：{e}（路径不存在/不可读/非 UTF-8 文本）"),
                    )
                    .to_json()
                }
            },
            (None, Some(content)) => {
                if content.len() > MAX_OPML_BYTES {
                    return WriteOutcome::failed_with(
                        write_contract::ERROR_INVALID_ARGUMENT,
                        format!("OPML 文本超过 {MAX_OPML_BYTES} 字节上限"),
                    )
                    .to_json();
                }
                content.to_string()
            }
        };

        let report = self.with_store(|s| rustrss_core::opml::import(s, &content));
        match report {
            Ok(report) => {
                let results: Vec<ItemResult> = report
                    .added_feed_ids
                    .iter()
                    .copied()
                    .take(MAX_BATCH_IDS)
                    .map(ItemResult::ok)
                    .collect();
                WriteOutcome::done(report.feeds_added as i64, results, false)
                    .with_detail(json!({
                        "added": report.feeds_added,
                        "skipped": report.feeds_skipped,
                        "errors": [],
                        "folders_created": report.folders_created,
                        "outlines_ignored": report.outlines_ignored,
                        "added_feed_ids_truncated": report.added_feed_ids.len() > MAX_BATCH_IDS,
                    }))
                    .to_json()
            }
            Err(rustrss_core::opml::ImportError::Xml(message)) => WriteOutcome::failed_with(
                write_contract::ERROR_INVALID_ARGUMENT,
                format!("OPML 解析失败：{message}"),
            )
            .with_detail(json!({ "added": 0, "skipped": 0, "errors": [message] }))
            .to_json(),
            Err(rustrss_core::opml::ImportError::Store(message)) => WriteOutcome::failed_with(
                ERROR_INTERNAL,
                format!("导入过程中库操作失败：{message}"),
            )
            .with_detail(json!({ "added": 0, "skipped": 0, "errors": [message] }))
            .to_json(),
        }
    }

    /// 导出 OPML（不写文件）：复用 core 的 `opml::export`（与界面「导出 OPML」同一条
    /// 实现），返回的文本可被 `import_opml` 原样回导。
    pub fn export_opml_json(&self) -> String {
        let feeds = match self.with_store(|s| s.list_feeds()) {
            Ok(feeds) => feeds,
            Err(e) => return internal_error(&e.to_string()),
        };
        match self.with_store(rustrss_core::opml::export) {
            Ok(opml) => WriteOutcome::done(feeds.len() as i64, Vec::new(), false)
                .with_detail(json!({ "feeds": feeds.len(), "opml": opml }))
                .to_json(),
            Err(e) => internal_error(&e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 白名单必须与界面 `REFRESH_INTERVAL_CHOICES` 逐值一致（15/30/60/120/360）。
    /// 这张表是跨 crate 复刻的，测试就是"不许漂移"的机械守卫。
    #[test]
    fn interval_whitelist_matches_the_ui_choices() {
        assert_eq!(REFRESH_INTERVAL_CHOICES, [15, 30, 60, 120, 360]);
        for ok in REFRESH_INTERVAL_CHOICES {
            assert!(check_refresh_interval(ok).is_ok(), "{ok} 应通过");
        }
        for bad in [0, 1, 5, 1440, -15] {
            let err = check_refresh_interval(bad).unwrap_err();
            assert!(err.contains("15/30/60/120/360"), "{err}");
        }
    }

    /// 标题归一化与界面同一口径：trim、空串 = 清除、按字符截断 200
    #[test]
    fn custom_title_normalization_matches_the_ui() {
        assert_eq!(normalize_custom_title("  技术周刊  "), Some("技术周刊".into()));
        assert_eq!(normalize_custom_title(""), None);
        assert_eq!(normalize_custom_title("   \t\n "), None);

        let long: String = "汉".repeat(MAX_CUSTOM_TITLE_LEN + 50);
        let cut = normalize_custom_title(&long).unwrap();
        assert_eq!(cut.chars().count(), MAX_CUSTOM_TITLE_LEN, "按字符截断");
        assert!(cut.chars().all(|c| c == '汉'), "中文不截成半个字");
    }

    /// subscribe 的输入形态解析：rsshub scheme 不联网、http(s) 走发现、其余 invalid_url
    #[test]
    fn subscribe_target_parsing_covers_all_shapes() {
        // rsshub 等价形态都归一到同一个存储键
        for raw in [
            "rsshub://telegram/channel/x",
            "rsshub:///telegram/channel/x",
            "RSSHUB://telegram/channel/x",
            "https://rsshub.app/telegram/channel/x",
        ] {
            assert_eq!(
                parse_subscribe_target(raw),
                Ok(SubscribeTarget::Scheme("rsshub://telegram/channel/x".into())),
                "{raw}"
            );
        }
        assert_eq!(
            parse_subscribe_target("  https://example.com/  "),
            Ok(SubscribeTarget::Web("https://example.com/".into()))
        );
        // scheme 大小写不敏感：`HTTP://` 同样是可订阅的 http 地址（rsshub 大写已在上面覆盖）
        assert_eq!(
            parse_subscribe_target("HTTP://example.com/feed.xml"),
            Ok(SubscribeTarget::Web("HTTP://example.com/feed.xml".into()))
        );
        for bad in ["", "   ", "not a url", "ftp://example.com/feed.xml", "http://"] {
            assert!(
                parse_subscribe_target(bad).is_err(),
                "{bad:?} 应判非法（invalid_url）"
            );
        }
    }

    /// `double_option`：键缺省 = 不动、显式 null = 清除/跟随全局、有值 = 设值
    #[test]
    fn update_feed_patch_is_tri_state() {
        let empty: UpdateFeedParams = serde_json::from_str(r#"{"feed_id": 3}"#).unwrap();
        assert_eq!(empty.feed_id, 3);
        assert!(empty.custom_title.is_none() && empty.folder_id.is_none());
        assert!(empty.refresh_interval_minutes.is_none());

        let nulls: UpdateFeedParams =
            serde_json::from_str(r#"{"feed_id": 3, "folder_id": null, "refresh_interval_minutes": null}"#)
                .unwrap();
        assert_eq!(nulls.folder_id, Some(None), "null = 移出分组");
        assert_eq!(nulls.refresh_interval_minutes, Some(None), "null = 跟随全局");

        let full: UpdateFeedParams = serde_json::from_str(
            r#"{"feed_id": 3, "custom_title": "新名", "folder_id": 7, "refresh_interval_minutes": 60}"#,
        )
        .unwrap();
        assert_eq!(full.custom_title.as_deref(), Some("新名"));
        assert_eq!(full.folder_id, Some(Some(7)));
        assert_eq!(full.refresh_interval_minutes, Some(Some(60)));

        let clear: UpdateFeedParams =
            serde_json::from_str(r#"{"feed_id": 3, "custom_title": ""}"#).unwrap();
        assert_eq!(clear.custom_title.as_deref(), Some(""), "空串 = 清除自定义名");
    }
}
