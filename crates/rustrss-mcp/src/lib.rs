//! RustRss MCP 服务器：把本地库里的订阅数据通过 MCP 暴露给外部 agent。
//!
//! 数据来源是 `rustrss-core` 的 SQLite 库（与界面读同一个库，因此不存在
//! 「两套查询逻辑漂移」的问题）。库路径按 `RUSTSS_DB` 环境变量 → 第一个命令行
//! 参数 → `~/.local/share/rustrss/rustrss.sqlite` 的顺序解析。
//!
//! 工具口径（PRD §6 风险 3）：**列表只回元数据 + 短摘要，正文必须单独取**，
//! 且所有列表都有上限——MCP 的响应直接进 agent 上下文，体积必须可控。
//!
//! 授权（见 [`registry`]）：默认全只读。写工具需要写 token（HTTP 按请求现算 scope）、
//! 写能力总开关与写 token 本身（stdio 同口径），危险工具额外看危险开关；
//! 无权限时返回工具级错误码（`write_scope_required` / `write_disabled` /
//! `dangerous_tool_disabled`），传输层 401 口径不变。
//!
//! 注意：stdout 是协议通道，任何日志只能走 stderr。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub mod audit;
pub mod config;
pub mod feed_tools;
pub mod http;
pub mod registry;
pub mod tag_tools;
pub mod theme_tools;
pub mod preview;
pub mod write_contract;
pub mod write_tools;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Extensions,
    ListToolsResult, PaginatedRequestParams, ResultType, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{tool, tool_handler, tool_router, transport::stdio, ServerHandler, ServiceExt};
use rustrss_core::{Fetcher, RefreshGate, Store};
use serde_json::Value;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use registry::{GateError, Scope, Switches, ToolSpec};

/// `list_articles` 的参数。
///
/// **默认口径固定**：`sort = newest`、`hide_read = false`——不继承用户此刻的界面
/// 设置（`list.sort` / `list.hide_read`）。显式传参才会偏离默认。
/// 分页：`page_size`（别名 `limit`）默认 10、上限 50；翻页把上一页的
/// `next_cursor` 原样回传给 `cursor`。
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct ListArticlesParams {
    /// 只看某个订阅源（feed id，来自 list_feeds）；与 folder_id 二选一
    pub feed_id: Option<i64>,
    /// 只看某个分组下的订阅源（folder id，来自 list_folders）；与 feed_id 二选一
    pub folder_id: Option<i64>,
    /// 只看未读
    pub unread_only: bool,
    /// 只看星标
    pub starred_only: bool,
    /// 只看「稍后读」
    pub read_later_only: bool,
    /// 只看带某个标签的条目（tag id 来自 list_tags）；与 `tag_name` 二选一
    pub tag_id: Option<i64>,
    /// 只看带某个标签的条目（按名称，大小写不敏感）；与 `tag_id` 二选一
    pub tag_name: Option<String>,
    /// 只看该时刻（含）之后的条目；比较的是列表排序键
    /// `COALESCE(published_at, fetched_at)`（Unix 秒）
    pub since: Option<i64>,
    /// 只看该时刻（含）之前的条目；闭区间，口径同 since
    pub until: Option<i64>,
    /// 每页条数（默认 10，上限 50）；与 limit 同义，page_size 优先
    pub page_size: Option<u32>,
    /// page_size 的兼容别名（旧调用方用）；两者都给时 page_size 生效
    pub limit: Option<u32>,
    /// 上一页返回的 `next_cursor` 原样回传；不传 = 取首页
    pub cursor: Option<String>,
    /// 排序档：`newest`（默认）/ `oldest` / `unread_first`；与 cursor 必须同一档
    pub sort: Option<String>,
    /// 是否隐藏已读（默认 false = 不隐藏；与界面设置无关）
    pub hide_read: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetArticleParams {
    /// 条目 id（来自 list_articles / search_articles）
    pub id: i64,
    /// 是否附带 HTML 正文（默认 false，只回纯文本）
    #[serde(default)]
    pub include_html: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchParams {
    /// 关键词（在标题与正文中检索；中文两字及以上可精确匹配）
    pub query: String,
    /// 最多返回多少条（默认 10，上限 50）
    pub limit: Option<u32>,
}

/// `digest_list` 的参数。
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct DigestListParams {
    /// 最多返回多少份（默认 10，上限 50；按日期倒序）
    pub limit: Option<u32>,
}

/// `digest_get` 的参数。
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DigestGetParams {
    /// 日期（YYYY-MM-DD，来自 digest_list）
    pub date: String,
    /// 范围键（来自 digest_list；缺省 "all" = 全部订阅源）
    pub scope_key: Option<String>,
    /// 来源清单每页条数（默认 10，上限 50）
    pub items_page_size: Option<u32>,
    /// 来源清单页码（从 1 起；缺省第 1 页）
    pub items_page: Option<u32>,
}

/// `get_unread_summary` 的参数
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct UnreadSummaryParams {
    /// 聚合维度：`feed`（默认，按订阅源）或 `folder`（按分组，未分组单列一组）
    pub by: Option<String>,
}

#[derive(Clone)]
pub struct RustRssMcp {
    store: Arc<Mutex<Store>>,
    /// 刷新单 flight：与界面刷新共用同一个 gate（应用内托管时由 src-tauri 注入）。
    /// `refresh` 工具（T3）用它，所以 agent 的刷新永远不会与界面刷新叠加。
    refresh_gate: Arc<RefreshGate>,
    /// HTTP 客户端（`refresh` / `fetch_fulltext` 共用一份，与界面各自持有自己的那份）。
    ///
    /// 保存构造**结果**而不是直接 `expect`：reqwest 客户端建不起来是环境问题
    /// （TLS 后端缺失之类），服务本身还应付得了只读工具——那种情况下让写工具
    /// 如实报 `internal_error`，好过整个进程 panic。
    fetcher: std::result::Result<Fetcher, String>,
    /// 测试专用桦工具（生产恒为空）。见 [`RustRssMcp::with_test_tool`]。
    test_tools: Arc<Vec<TestTool>>,
    theme_changed: Option<Arc<dyn Fn(u64) -> bool + Send + Sync>>,
    preview: Arc<preview::Service>,
    profile_id: String,
}

/// 测试专用桦工具：T3/T4 的真实写工具落地前，授权矩阵需要「已登记、可调用」的写工具。
///
/// 它走的是**与真实工具同一条**路径：同一个注册表元数据、同一个 gating/错误码、
/// 同一份审计行——只是工具体由测试提供（详见 `tests/write_auth.rs`）。
/// 生产环境不会注册任何桦工具（`test_tools` 恒为空），所以这个缝不会把假工具
/// 暴露给用户。
/// 桩工具的工具体：拿真实调用参数（与生产工具同一个来源），返回工具输出文本
pub type TestToolHandler = dyn Fn(Option<&serde_json::Map<String, Value>>) -> String + Send + Sync;

#[derive(Clone)]
pub struct TestTool {
    pub spec: ToolSpec,
    pub description: &'static str,
    pub handler: Arc<TestToolHandler>,
}

impl RustRssMcp {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            refresh_gate: Arc::new(RefreshGate::new()),
            fetcher: Fetcher::new(rustrss_core::fetch::DEFAULT_USER_AGENT)
                .map_err(|e| format!("初始化 HTTP 客户端失败: {e}")),
            test_tools: Arc::new(Vec::new()),
            theme_changed: None,
            preview: Arc::new(preview::Service::default()),
            profile_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// Host notification only: true means queued, not rendered. Called outside store lock.
    #[must_use]
    pub fn with_theme_notifications(
        mut self,
        callback: impl Fn(u64) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.theme_changed = Some(Arc::new(callback));
        self
    }

    /// 打开（必要时创建/迁移）指定路径的库
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, rustrss_core::StoreError> {
        let store = Store::open(&path)?;
        let mut server = Self::new(store);
        let canonical = std::fs::canonicalize(path).map_err(|e| rustrss_core::StoreError::Io(e.to_string()))?;
        server.profile_id = preview::fingerprint(canonical.as_os_str().as_encoded_bytes());
        Ok(server)
    }

    /// 注入刷新单 flight 的共享 gate（应用内托管时由 `src-tauri` 传界面的那个）。
    #[must_use]
    pub fn with_refresh_gate(mut self, gate: Arc<RefreshGate>) -> Self {
        self.refresh_gate = gate;
        self
    }

    /// 共享的刷新单 flight gate（`refresh` 工具与界面刷新的唯一交接口）
    pub fn refresh_gate(&self) -> Arc<RefreshGate> {
        Arc::clone(&self.refresh_gate)
    }

    /// 注入 HTTP 客户端（宿主想复用自己那份客户端时用；测试也能借此不建新客户端）
    #[must_use]
    pub fn with_fetcher(mut self, fetcher: Fetcher) -> Self {
        self.fetcher = Ok(fetcher);
        self
    }

    /// 写工具要用的 HTTP 客户端；构造失败时把原因交回调用方（工具层包成 `internal_error`）
    pub(crate) fn fetcher(&self) -> std::result::Result<Fetcher, String> {
        let config = self.with_store(rustrss_core::network::ProxyConfig::load).map_err(|e| e.to_string())?;
        self.fetcher.as_ref().map_err(Clone::clone)?.configured(&config)
    }


    /// **测试专用**：登记一个桦工具（生产不调这个函数）。
    #[doc(hidden)]
    #[must_use]
    pub fn with_test_tool(
        mut self,
        name: &'static str,
        scope: Scope,
        dangerous: bool,
        description: &'static str,
        handler: impl Fn(Option<&serde_json::Map<String, Value>>) -> String + Send + Sync + 'static,
    ) -> Self {
        let mut tools = (*self.test_tools).clone();
        tools.push(TestTool {
            spec: ToolSpec {
                name,
                scope,
                dangerous,
            },
            description,
            handler: Arc::new(handler),
        });
        self.test_tools = Arc::new(tools);
        self
    }

    pub(crate) fn with_store<T>(&self, f: impl FnOnce(&Store) -> T) -> T {
        let guard = self.store.lock().expect("库锁被毒化（前一次调用 panic）");
        f(&guard)
    }

    /// 授权闸门要的开关快照：**每个请求现读**（不缓存）。
    ///
    /// 读失败时的方向是「拒绝写」：拿不到开关状态就当全部关闭（写工具用不了），
    /// 而不是猜成打开——一次 SQLITE_BUSY 不应该变成一条越权路径。
    pub fn switches(&self) -> Switches {
        self.with_store(|store| match config::switches_from_store(store) {
            Ok(sw) => sw,
            Err(e) => {
                log::warn!(target: "mcp", "mcp-write 读取开关失败（按全关处理）: {e}");
                Switches::default()
            }
        })
    }

    /// 按「当次请求携带的凭据」判定 scope；都不匹配则 `None`（→ 401）。
    ///
    /// **每请求现读库里的 token**：写 token 轮换/销毁后，旧值在下一个请求立刻失效
    /// （包括已建立的 HTTP 连接/会话——授权从不缓存到会话上）。
    pub fn scope_of_credential(&self, presented: &str, static_token: Option<&str>) -> Option<Scope> {
        let (write_token, read_token) = self.with_store(|store| {
            let write = config::write_token_from_store(store).ok().flatten();
            let read = store
                .setting(config::K_TOKEN)
                .ok()
                .flatten()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            (write, read)
        });
        if let Some(write) = write_token {
            if http::constant_eq(presented.as_bytes(), write.as_bytes()) {
                return Some(Scope::Write);
            }
        }
        if let Some(read) = read_token {
            if http::constant_eq(presented.as_bytes(), read.as_bytes()) {
                return Some(Scope::Read);
            }
        }
        // 静态读凭据（CLI 的 `RUSTSS_MCP_TOKEN`）只给读权限
        if let Some(static_token) = static_token.filter(|t| !t.is_empty()) {
            if http::constant_eq(presented.as_bytes(), static_token.as_bytes()) {
                return Some(Scope::Read);
            }
        }
        None
    }

    /// 工具清单（静态注册表的工具 + 测试桦工具）的可见子集。
    ///
    /// 过滤与 gating 用同一个 `registry::authorize`——「列出来却调不动」不可能出现。
    pub fn visible_tools(&self, scope: Scope, switches: &Switches) -> Vec<Tool> {
        let mut tools = Self::tool_router().list_all();
        for stub in self.test_tools.iter() {
            tools.push(Tool::new(
                stub.spec.name,
                stub.description,
                Arc::new(serde_json::Map::new()),
            ));
        }
        let specs = self.tool_specs();
        tools.retain(|t| match specs.iter().find(|s| s.name == t.name.as_ref()) {
            Some(spec) => registry::visible(spec, scope, switches),
            None => false,
        });
        tools
    }

    /// 注册表元数据查询：静态表优先，其次是测试桦工具（名字相同以静态表为准，
    /// 避免测试桦工具覆盖真实工具的作用域）。
    fn tool_specs(&self) -> Vec<ToolSpec> {
        let mut specs = registry::TOOL_SPECS.to_vec();
        for stub in self.test_tools.iter() {
            if !specs.iter().any(|s| s.name == stub.spec.name) {
                specs.push(stub.spec);
            }
        }
        specs
    }

    fn test_tool(&self, name: &str) -> Option<TestTool> {
        self.test_tools.iter().find(|t| t.spec.name == name).cloned()
    }

    /// 当次请求的 scope（HTTP 走中间件注入的 [`http::RequestScope`]；stdio 恒为 write）。
    fn scope_of(&self, context: &RequestContext<RoleServer>) -> Scope {
        scope_from_extensions(&context.extensions)
    }

    pub fn list_feeds_json(&self) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "list_feeds", &serde_json::json!({})))
    }

    pub fn list_articles_json(&self, p: &ListArticlesParams) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "list_articles", &serde_json::to_value(p).expect("typed tool parameters")))
    }

    /// 分组清单 + 每组未读合计（未分组单列）
    pub fn list_folders_json(&self) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "list_folders", &serde_json::json!({})))
    }

    /// 日报列表：元数据 + ≤140 字概览（默认 10、上限 50，按日期倒序）
    pub fn digest_list_json(&self, p: &DigestListParams) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "digest_list", &serde_json::to_value(p).expect("typed tool parameters")))
    }

    /// 单份日报：头 + 正文（Markdown）+ 来源清单分页（默认 10、上限 50）
    pub fn digest_get_json(&self, p: &DigestGetParams) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "digest_get", &serde_json::to_value(p).expect("typed tool parameters")))
    }

    /// 未读聚合（按源或按分组）
    pub fn unread_summary_json(&self, p: &UnreadSummaryParams) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "get_unread_summary", &serde_json::to_value(p).expect("typed tool parameters")))
    }

    pub fn get_article_json(&self, p: &GetArticleParams) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "get_article", &serde_json::to_value(p).expect("typed tool parameters")))
    }

    pub fn search_articles_json(&self, p: &SearchParams) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "search_articles", &serde_json::to_value(p).expect("typed tool parameters")))
    }

    pub fn db_stats_json(&self) -> String {
        self.with_store(|store| rustrss_core::ai::tools::project_tool(store, "db_stats", &serde_json::json!({})))
    }
}

#[tool_router]
impl RustRssMcp {
    #[tool(description = "Create or patch an in-memory theme preview and return an absolute local PNG image_path of fixed UI fixtures (no inline image). Open image_path using view_image or an equivalent local image-reading tool before judging or saving. Requires access to the desktop host filesystem; if the client cannot read it, report visual verification unavailable. Files expire after 600s and can be removed earlier on token revocation or desktop exit. Requires write scope. Supply base_revision from get_theme. For updates also supply preview_id and expected_preview_revision. No settings are saved until finish_theme_preview(save).")]
    async fn preview_theme(&self, Parameters(p): Parameters<preview::PreviewParams>, context: RequestContext<RoleServer>) -> CallToolResult {
        self.preview_call("preview_theme", serde_json::to_value(p).unwrap(), &context).await
    }
    #[tool(description = "Capture a fixed scene from an owned theme preview at expected_preview_revision. Returns an absolute local PNG image_path plus image_expires_at_ms and render/version metadata, never an inline image. Open image_path with view_image or your local image-reading tool to inspect the complete screenshot. Requires access to the desktop host filesystem; a path or metadata alone does not prove visual verification. Files survive save/cancel until their 600s expiry, unless token revocation or desktop exit removes them early. Requires write scope.")]
    async fn capture_theme_preview(&self, Parameters(p): Parameters<preview::CaptureParams>, context: RequestContext<RoleServer>) -> CallToolResult {
        self.preview_call("capture_theme_preview", serde_json::to_value(p).unwrap(), &context).await
    }
    #[tool(description = "Save or cancel an owned preview. Save uses CAS against its base_revision; conflicts preserve the candidate. Screenshot files are retained until their original expiry (unless permission is revoked or the desktop exits). Repeating the same finish is idempotent for the last 16 finishes within ten minutes. Requires write scope.")]
    async fn finish_theme_preview(&self, Parameters(p): Parameters<preview::FinishParams>, context: RequestContext<RoleServer>) -> CallToolResult {
        self.preview_call("finish_theme_preview", serde_json::to_value(p).unwrap(), &context).await
    }
    #[tool(
        description = "Read the current theme, effective light/dark parameters, revision, last ten history revisions and preview capability. include_schema=true returns patch constraints. Does not write or render."
    )]
    async fn get_theme(&self, Parameters(p): Parameters<theme_tools::GetThemeParams>) -> CallToolResult {
        self.theme_capabilities_result(&p).await
    }
    #[tool(
        description = "List the four built-in theme presets (metadata only). Use validate_theme to resolve a preset without saving."
    )]
    fn list_theme_presets(&self) -> CallToolResult {
        self.theme_presets_result()
    }
    #[tool(
        description = "Validate a sparse theme patch against expected_revision without saving. Returns effective light/dark values, hash and contrast warnings. Get patch_schema from get_theme. Null in overrides restores inheritance."
    )]
    fn validate_theme(
        &self,
        Parameters(p): Parameters<theme_tools::ThemePatchParams>,
    ) -> CallToolResult {
        self.validate_theme_result(&p)
    }
    #[tool(
        description = "Persist a validated theme patch using expected_revision CAS. Requires write scope. No-op leaves revision unchanged. Returns saved_revision and live_apply pending/unavailable/unchanged; pending is not a rendered-frame acknowledgement. No screenshot is returned."
    )]
    fn update_theme(
        &self,
        Parameters(p): Parameters<theme_tools::ThemePatchParams>,
    ) -> CallToolResult {
        self.update_theme_result(&p)
    }
    #[tool(
        description = "Restore one of get_theme.history revisions using expected_revision CAS. Requires write scope. Creates a new monotonic revision; never rolls back the revision counter."
    )]
    fn restore_theme(
        &self,
        Parameters(p): Parameters<theme_tools::RestoreThemeParams>,
    ) -> CallToolResult {
        self.restore_theme_result(&p)
    }

    #[tool(description = "列出订阅源及其未读数（含上次抓取状态；状态非 ok 的源数据可能不是最新）")]
    fn list_feeds(&self) -> String {
        self.list_feeds_json()
    }

    #[tool(
        description = "列出分组（文件夹）及其未读合计；未分组订阅的未读单列（ungrouped_unread）、合计 total_unread"
    )]
    fn list_folders(&self) -> String {
        self.list_folders_json()
    }

    #[tool(
        description = "列出条目元数据（不含正文）。默认口径固定：sort=newest 且不隐藏已读（不跟随界面设置，显式传参才覆盖）；每页默认 10 条、上限 50，翻页把 next_cursor 回传给 cursor；可用 feed_id / folder_id（二选一）、tag_id / tag_name（二选一，按标签筛选；未知标签报 tag_not_found）、unread_only、starred_only、read_later_only、since / until（对 COALESCE(published_at,fetched_at) 的闭区间，Unix 秒）、sort（newest/oldest/unread_first）、hide_read。每条带 tags（文章手动 ∪ 源继承的有效标签名，去重按名称稳定排序，最多 20 个，超出置 tags_truncated）及同序 tag_sources（id/name/source，manual/feed/both）。正文用 get_article 单独取。"
    )]
    fn list_articles(&self, Parameters(p): Parameters<ListArticlesParams>) -> String {
        self.list_articles_json(&p)
    }

    #[tool(description = "取单篇文章正文（默认纯文本，可要求附带 HTML 原始正文）")]
    fn get_article(&self, Parameters(p): Parameters<GetArticleParams>) -> String {
        self.get_article_json(&p)
    }

    #[tool(description = "全文搜索标题与正文（中文需两字及以上）")]
    fn search_articles(&self, Parameters(p): Parameters<SearchParams>) -> String {
        self.search_articles_json(&p)
    }

    #[tool(
        description = "列出已生成的每日日报（元数据 + ≤140 字概览，按日期倒序；默认 10 份、上限 50）。正文用 digest_get 单取。"
    )]
    fn digest_list(&self, Parameters(p): Parameters<DigestListParams>) -> String {
        self.digest_list_json(&p)
    }

    #[tool(
        description = "取单份每日日报：头部（日期/范围/统计）+ 成品 Markdown + 来源清单分页（items_page_size 默认 10、上限 50）。日期与 scope_key 来自 digest_list。只读，不触发任何 AI 生成。"
    )]
    fn digest_get(&self, Parameters(p): Parameters<DigestGetParams>) -> String {
        self.digest_get_json(&p)
    }

    #[tool(
        description = "未读聚合：by = feed（默认，按订阅源）或 folder（按分组，未分组单列一组），返回每组的未读数与 total_unread"
    )]
    fn get_unread_summary(&self, Parameters(p): Parameters<UnreadSummaryParams>) -> String {
        self.unread_summary_json(&p)
    }

    #[tool(description = "库的总体统计：订阅源数、条目数、未读数")]
    fn db_stats(&self) -> String {
        self.db_stats_json()
    }

    // ------------------------------------------------------------ 写工具（T3：阅读状态 / 刷新 / 全文）

    #[tool(
        description = "标记条目为已读/未读。目标二选一：ids[]（≤100 个，缺失的 id 逐项回 article_not_found）或条件级 {feed_id, since, until}（至少一个；since/until 是闭区间，键同列表排序键 COALESCE(published_at,fetched_at)，Unix 秒）。read 缺省 true（标记已读）。返回 {ok, affected, results, error_code?, detail}：affected = 命中条数（重复调用返回值稳定 = 幂等），条件级命中的 id 会以有界采样（前 100 个）回在 results 里。写后 db_stats/界面计数立即一致（同一 store）。错误码：invalid_argument（ids 为空/超限/两种目标混用/条件缺失）、article_not_found、write_disabled、write_scope_required。"
    )]
    fn set_read(&self, Parameters(p): Parameters<write_tools::SetReadParams>) -> String {
        self.set_read_json(&p)
    }

    #[tool(
        description = "给条目加/取消星标。目标与返回口径同 set_read（ids[] ≤100 或条件级 {feed_id, since, until}）；starred 缺省 true。幂等。错误码同 set_read。"
    )]
    fn set_starred(&self, Parameters(p): Parameters<write_tools::SetStarredParams>) -> String {
        self.set_starred_json(&p)
    }

    #[tool(
        description = "把条目加入/移出「稍后读」。目标与返回口径同 set_read（ids[] ≤100 或条件级 {feed_id, since, until}）；later 缺省 true。幂等。错误码同 set_read。"
    )]
    fn set_read_later(&self, Parameters(p): Parameters<write_tools::SetReadLaterParams>) -> String {
        self.set_read_later_json(&p)
    }

    #[tool(
        description = "刷新订阅源（抓取新条目）。scope=all（全部）/ feed_ids（配合 feed_ids[]，≤100）/ folder（配合 folder_id）；不传时按参数推断，都没有 = all。与界面刷新共用同一个单 flight：已有刷新在跑时本轮不执行，返回 error_code=rate_limited（不排队、不叠加，稍后重试）。返回 {ok, affected, results, error_code?, detail}：affected = 本轮入库条目数（新增 + 更新），results 只列失败源（error_code=fetch_failed），detail 给本轮摘要（feeds/fetched/not_modified/inserted/updated/unchanged/failure_count/failures）。错误码：invalid_argument、feed_not_found、folder_not_found、rate_limited、write_disabled、write_scope_required。"
    )]
    async fn refresh(&self, Parameters(p): Parameters<write_tools::RefreshParams>) -> String {
        self.refresh_json(&p).await
    }

    #[tool(
        description = "给单篇摘要型条目补全文：抓原文页 → 提取正文 → 写回库（受 2MiB 体积闸门保护）。已抓过/本来就是全文型时零网络返回（detail.already_fulltext=true，affected=0）。正文不随本工具的响应返回：写回后用 get_article(id) 取。错误码：article_not_found、invalid_url（条目没有原文地址）、fulltext_too_large（超过 2MiB）、fulltext_bot_challenge（站点要浏览器验证）、fulltext_not_html、fulltext_no_content、fetch_failed（网络/HTTP 错，可重试）、write_disabled、write_scope_required。"
    )]
    async fn fetch_fulltext(
        &self,
        Parameters(p): Parameters<write_tools::FetchFulltextParams>,
    ) -> String {
        self.fetch_fulltext_json(&p).await
    }

    // ------------------------------------------------------------ 写工具（T4：订阅管理）

    #[tool(
        description = "订阅一个源。url 两种形态：站点首页或 feed 地址（自动发现 feed：内容本身能按 feed 解析就用输入地址，否则扫页面 head 里的 <link rel=\"alternate\">，与界面「添加订阅」同一条 core 路径）、或 rsshub://path（三斜杠 rsshub:///path、大写 scheme、https://rsshub.app/path 都归一为 rsshub://path，不联网）。幂等：地址（含 rsshub 等价形态）已在库里时不报错，返回 ok=true、affected=0、detail.already_subscribed=true 与既有 feed_id；新订阅 affected=1。返回 {ok, affected, results, error_code?, detail}，detail 含 feed_id / title / url / already_subscribed（首页发现路径还有 discovered_from 与 via=direct|link_type|link_suffix）。错误码：invalid_url（空/非 http(s)/无主机名/页面里没发现 feed）、fetch_failed（网络或 HTTP 错，可重试）、write_disabled、write_scope_required。"
    )]
    async fn subscribe(&self, Parameters(p): Parameters<feed_tools::SubscribeParams>) -> String {
        self.subscribe_json(&p).await
    }

    #[tool(
        description = "改单个订阅源的显示名 / 分组 / 每源刷新间隔（tri-state patch：键**缺省 = 不动**）。custom_title：空串（或纯空白）= 清除自定义名、显示回退源站名；有值 = 设自定义名（trim + 截断 200 字符）。folder_id：null = 移出到未分组；数字 = 移入该分组。refresh_interval_minutes：null = 跟随全局档；数字必须命中白名单 15/30/60/120/360（归一化与落库与界面编辑对话框同源：同一批 Store 方法）。返回 {ok, affected, results, error_code?, detail}，detail 回读更新后的行（title / custom_title / source_title / folder_id / refresh_interval_minutes / updated_fields）。错误码：feed_not_found、folder_not_found、invalid_argument（三个字段一个都没给 / 间隔不在白名单）、write_disabled、write_scope_required。"
    )]
    fn update_feed(&self, Parameters(p): Parameters<feed_tools::UpdateFeedParams>) -> String {
        self.update_feed_json(&p)
    }

    #[tool(
        description = "新建分组（文件夹）。同名分组已存在时返回既有 id 且 ok=true、affected=0（detail.already_exists=true），不报错。错误码：invalid_argument（名字 trim 后为空）、write_disabled、write_scope_required。"
    )]
    fn folder_create(&self, Parameters(p): Parameters<feed_tools::FolderCreateParams>) -> String {
        self.folder_create_json(&p)
    }

    #[tool(
        description = "重命名分组。错误码：folder_not_found、invalid_argument（名字为空或与其它分组重名）、write_disabled、write_scope_required。"
    )]
    fn folder_rename(&self, Parameters(p): Parameters<feed_tools::FolderRenameParams>) -> String {
        self.folder_rename_json(&p)
    }

    #[tool(
        description = "删分组（危险工具：需写能力 + 危险开关都开启）。删组**不删订阅**：组内订阅移出到未分组（folder_id=null），与界面同一语义。必须 confirm: true 才执行（缺 → confirm_required）；dry_run: true 只返回影响面（detail.feeds_affected = 将移出的订阅数、feed_ids 采样）且库不变——预览与实际执行共用同一个影响面函数。错误码：folder_not_found、confirm_required、dangerous_tool_disabled、write_disabled、write_scope_required。"
    )]
    fn folder_delete(&self, Parameters(p): Parameters<feed_tools::FolderDeleteParams>) -> String {
        self.folder_delete_json(&p)
    }

    #[tool(
        description = "退订（危险工具：需写能力 + 危险开关都开启）。删订阅源并**级联删除其全部条目**（全文索引由触发器同步清理）。必须 confirm: true 才执行（缺 → confirm_required）；dry_run: true 只返回影响面（affected/detail.entries_affected = 将删除的条目数）且库不变——预览与实际执行共用同一个计数函数（Store::entry_count_for_feed），所以「预览多少条就真删多少条」。错误码：feed_not_found、confirm_required、dangerous_tool_disabled、write_disabled、write_scope_required。"
    )]
    fn unsubscribe(&self, Parameters(p): Parameters<feed_tools::UnsubscribeParams>) -> String {
        self.unsubscribe_json(&p)
    }

    #[tool(
        description = "导入 OPML（path 本地文件 或 content 文本，二选一）。复用 core 的 opml::import——与界面「导入 OPML」同一条实现：按 xmlUrl 去重（已存在记 skipped 且不移动分组），嵌套分组压平成「父/子」。返回 {ok, affected, results, error_code?, detail}：affected = 本次新增订阅数，results 逐项给新增 feed id（>100 截断），detail 含 added / skipped / errors（成功时空数组）/ folders_created / outlines_ignored。错误码：invalid_argument（path/content 都给或都不给 / 读文件失败 / 非 UTF-8 / 超 8MiB / XML 解析失败）、internal_error、write_disabled、write_scope_required。"
    )]
    fn import_opml(&self, Parameters(p): Parameters<feed_tools::ImportOpmlParams>) -> String {
        self.import_opml_json(&p)
    }

    #[tool(
        description = "导出全部订阅为 OPML 文本（**不写文件**，与界面「导出 OPML」同一条 core 实现）。返回 {ok, affected, results, error_code?, detail}：affected = 导出的订阅数，detail.opml 是 OPML 2.0 文本（含分组结构），可原样交给 import_opml 回导（往返幂等：第二次导入全部记 skipped）。错误码：internal_error、write_disabled、write_scope_required。"
    )]
    fn export_opml(&self) -> String {
        self.export_opml_json()
    }

    // ------------------------------------------------------------ 标签（T4：read 一个 + write 五个）

    #[tool(
        description = "列出标签及其未读计数（**只读工具**：读 token 也能用）。sort=sidebar（默认：置顶优先 → 手动顺序 → 名称）或 recent（最近使用优先，没用过的垫底）。只回元数据（id / 名称 / 颜色 / 置顶 / 未读计数 / 顺序 / 最近使用），一次最多 200 个：超出时 truncated=true 且 total 给全量口径（不静默丢）。写工具的 tag_id 取自这里。"
    )]
    fn list_tags(&self, Parameters(p): Parameters<tag_tools::ListTagsParams>) -> String {
        self.list_tags_json(&p)
    }

    #[tool(
        description = "新建标签（写工具）。name trim 后不能为空；标签名大小写不敏感唯一——重名报 duplicate_tag_name（先用 list_tags 确认），空名 / 非法颜色（非 #RRGGBB）报 invalid_argument。返回 {ok, affected=1, results=[id], detail=标签行}。"
    )]
    fn create_tag(&self, Parameters(p): Parameters<tag_tools::CreateTagParams>) -> String {
        self.create_tag_json(&p)
    }

    #[tool(
        description = "重命名标签（写工具）。tag_id 不存在 → tag_not_found；重名（大小写不敏感）→ duplicate_tag_name；空名 → invalid_argument。允许仅大小写变化（rust → Rust）。返回更新后的标签行在 detail。"
    )]
    fn rename_tag(&self, Parameters(p): Parameters<tag_tools::RenameTagParams>) -> String {
        self.rename_tag_json(&p)
    }

    #[tool(
        description = "给条目附加标签（写工具，幂等）。目标二选一：ids[]（≤100）或条件级 {feed_id, since, until}（至少一个，闭区间，口径同 set_read）；混用 / 都缺 → invalid_argument。tag_ids 1..=100 个（来自 list_tags；有一个不存在 → tag_not_found 且本次零改动）。返回 {ok, affected, results, detail}：affected = 命中条目数（重复调用稳定），detail.changed = 本次真正新增的关联行数（重复调用 0），不存在的条目 id 逐项回 article_not_found。"
    )]
    fn assign_tags(&self, Parameters(p): Parameters<tag_tools::AssignTagsParams>) -> String {
        self.assign_tags_json(&p)
    }

    #[tool(
        description = "移除条目上的标签（写工具，幂等）。目标与返回口径同 assign_tags：affected = 命中条目数、detail.changed = 本次真正移除的关联行数（重复调用 0）；只清手动关联，文章保留；取消手动关联不屏蔽源继承（须在订阅源上修改）。"
    )]
    fn unassign_tags(&self, Parameters(p): Parameters<tag_tools::AssignTagsParams>) -> String {
        self.unassign_tags_json(&p)
    }

    #[tool(
        description = "删除标签（写工具；只清关联、不删文章，**不是危险工具**）。实际执行必须 confirm: true（缺 → confirm_required）；dry_run: true 只返回受影响篇数（affected）且库不变——预览与实际执行共用 core 同一个计数函数（Store::delete_tag）。tag_id 不存在 → tag_not_found。"
    )]
    fn delete_tag(&self, Parameters(p): Parameters<tag_tools::DeleteTagParams>) -> String {
        self.delete_tag_json(&p)
    }
}

/// 当次请求的 scope：HTTP 由鉴权中间件按下发凭据算出（注入扩展），stdio 无该标记。
///
/// - HTTP：读中间件注入的 [`http::RequestScope`]；**注入缺失时 fail-closed 到 `read`**
///   （将来若有人改了中间件却忘了注入，写工具会变成不可用，而不是变成人人可写）；
/// - stdio：本地进程、没有凭据概念，恒为 `write`——写能力仍由开关①写 token 存在性把关，
///   所以“stdio 与 HTTP 同口径”指的是同一套闸门，而不是同一个 scope 来源。
pub fn scope_from_extensions(extensions: &Extensions) -> Scope {
    match extensions.get::<axum::http::request::Parts>() {
        Some(parts) => parts
            .extensions
            .get::<http::RequestScope>()
            .map(|s| s.scope)
            .unwrap_or(Scope::Read),
        None => Scope::Write,
    }
}

/// 工具级错误的响应体（与写信封同形：机器可读 `error_code` + 人类可读 `error`）
fn gate_error_result(err: GateError) -> CallToolResponse {
    CallToolResult::error(vec![ContentBlock::text(err.to_body())]).into()
}

/// 内部一致性错误码：工具暴露了但没登记授权元数据（fail-closed 拒绝，正常情况不会出现）
const UNREGISTERED_TOOL_CODE: &str = "tool_not_registered";

/// 从工具响应里抽出文本体（审计摘要要解析它；非文本/非 Complete 的响应没有可审计内容）
fn response_text(response: &CallToolResponse) -> Option<&str> {
    match response {
        CallToolResponse::Complete(result) => result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str()),
        _ => None,
    }
}

#[tool_handler(
    name = "rustrss",
    instructions = "读取本地 RustRss 订阅库。列表工具只回元数据与短摘要，正文需用 get_article 单独取；写工具需要写 token（HTTP）且写能力已开启，默认只读。"
)]
impl ServerHandler for RustRssMcp {
    /// `tools/list`：按**当次请求**的 scope 与开关过滤（不缓存会话 scope）。
    ///
    /// 读 token 的会话看不到写工具；写开关/写 token/危险开关任一未就位，
    /// 对应的工具也不会出现在列表里（可见性与可调用性是同一个判断）。
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let scope = self.scope_of(&context);
        let switches = self.switches();
        let supports_cache_hints = context.protocol_version().is_some_and(|version| {
            version >= rmcp::model::ProtocolVersion::V_2026_07_28
        });
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            meta: None,
            tools: self.visible_tools(scope, &switches),
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(rmcp::model::CacheScope::Private),
        })
    }

    /// `tools/call`：先过授权闸门（写工具），再过审计，最后才分发到工具体。
    ///
    /// 拒掉时返回的是**工具级错误**（200 + `isError` + `error_code`），不是 401：
    /// 「任务已认证、但没写权限」要让 agent 读得懂，且不能泄露任何订阅数据。
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let name = request.name.to_string();
        let args = request.arguments.clone();
        let scope = self.scope_of(&context);
        let switches = self.switches();
        let spec = self
            .tool_specs()
            .into_iter()
            .find(|s| s.name == name.as_str());
        // 只有登记过的写工具才需要过闸；未登记的名字交给 router 按 MCP 语义拒（工具不存在）
        let is_write = spec.as_ref().is_some_and(|s| s.scope == Scope::Write);
        if spec.is_none() && Self::tool_router().get(&name).is_some() {
            // 路由里有、注册表里没有：元数据缺失就不知道它是读还是写。
            // **fail-closed 直接拒**，而不是“没元数据就不用过闸”——否则某天新增一个
            // 写工具却忘了登记，读 token 就直接能改了。
            // 单测 `every_exposed_tool_is_registered` 会让这种情况在 CI 里先红。
            audit::emit(
                &name,
                args.as_ref(),
                &audit::AuditSummary::failed(UNREGISTERED_TOOL_CODE),
            );
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                serde_json::json!({
                    "ok": false,
                    "affected": 0,
                    "results": [],
                    "error_code": UNREGISTERED_TOOL_CODE,
                    "error": "工具未在授权注册表登记，已拒绝（请登记 scope/dangerous 元数据）",
                })
                .to_string(),
            )])
            .into());
        }
        if let Some(spec) = spec.as_ref().filter(|s| s.scope == Scope::Write) {
            if let Err(err) = registry::authorize(spec, scope, &switches) {
                let summary = audit::AuditSummary::rejected(err);
                audit::emit(&name, args.as_ref(), &summary);
                return Ok(gate_error_result(err));
            }
        }

        let response = match self.test_tool(&name) {
            Some(stub) => CallToolResponse::Complete(CallToolResult::success(vec![
                ContentBlock::text((stub.handler)(args.as_ref())),
            ])),
            None => {
                let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
                match Self::tool_router().call(tcc).await {
                    Ok(response) => response,
                    Err(error) => {
                        if is_write {
                            audit::emit(
                                &name,
                                args.as_ref(),
                                &audit::AuditSummary::failed("tool_dispatch_error"),
                            );
                        }
                        return Err(error);
                    }
                }
            }
        };

        if is_write {
            let summary = response_text(&response)
                .map(audit::summary_from_body)
                .unwrap_or_else(|| audit::AuditSummary::failed("empty_result"));
            audit::emit(&name, args.as_ref(), &summary);
            // 写调用失败 ⇒ 统一标成**工具级错误**（body 里仍有 error_code / 逐项结果）。
            // 这样 agent 不必分辨"失败是闸门给的还是工具体的"：规则只有一条
            // （isError=true + 信封里的 error_code），T3/T4 只要遵守写契约就自动一致。
            if !summary.ok && !matches!(&response, CallToolResponse::Complete(r) if r.is_error == Some(true)) {
                if let Some(body) = response_text(&response) {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(body.to_string())]).into());
                }
            }
        }
        Ok(response)
    }
}

/// 解析库路径（与界面共用 core 的同一套规则，保证两边指向同一个文件）
pub fn resolve_db_path() -> PathBuf {
    rustrss_core::resolve_db_path()
}

/// 以 stdio 传输运行（由 MCP 客户端作为子进程拉起）
pub async fn serve_stdio(server: RustRssMcp) -> anyhow::Result<()> {
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;
    use rmcp::model::Extensions;

    /// 造一份「HTTP 请求的 parts」（中间件注入 scope 的载体）
    fn plain_parts() -> axum::http::request::Parts {
        HttpRequest::builder()
            .uri("/mcp")
            .body(())
            .expect("构造请求失败")
            .into_parts()
            .0
    }

    fn parts_with_scope(scope: Scope) -> axum::http::request::Parts {
        let mut parts = plain_parts();
        parts
            .extensions
            .insert(http::RequestScope { scope });
        parts
    }

    /// stdio（无 `Parts` 扩展）与 HTTP（有 `Parts`）的 scope 来源不同，但只能是这两条路。
    #[test]
    fn scope_comes_from_the_transport_not_from_the_session() {
        // ① stdio：没有 HTTP 请求扩展 → write（写能力另由开关把关）
        assert_eq!(scope_from_extensions(&Extensions::new()), Scope::Write);

        // ② HTTP：读中间件注入的当次 scope
        for (injected, expected) in [(Scope::Read, Scope::Read), (Scope::Write, Scope::Write)] {
            let mut ext = Extensions::new();
            ext.insert(parts_with_scope(injected));
            assert_eq!(scope_from_extensions(&ext), expected);
        }

        // ③ HTTP 但没注入（中间件被改坏/被绕过）：fail-closed 到 read，不得静默升权
        let mut ext = Extensions::new();
        ext.insert(plain_parts());
        assert_eq!(scope_from_extensions(&ext), Scope::Read);
    }

    /// 注册表与 `#[tool]` 路由不得漂移：每个对外暴露的工具都有 scope/dangerous 元数据。
    ///
    /// 漏登记的工具在过滤时会被丢掉（默认不可见），所以这个断言是「新增工具忘了登记」
    /// 的机械守卫；反向也查（注册表里不得有已下架的工具名）。
    #[test]
    fn every_exposed_tool_is_registered() {
        let exposed: Vec<String> = RustRssMcp::tool_router()
            .list_all()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        for name in &exposed {
            assert!(
                registry::spec(name).is_some(),
                "工具 {name} 已暴露但未在 registry::TOOL_SPECS 登记"
            );
        }
        for spec in registry::TOOL_SPECS {
            assert!(
                exposed.iter().any(|n| n == spec.name),
                "注册表里的 {} 已不存在（下架工具要同步删元数据）",
                spec.name
            );
        }
        assert_eq!(exposed.len(), registry::TOOL_SPECS.len());
    }

    /// 默认（未开写能力）下可见的只有只读工具，且读 token 会话看不到任何写工具。
    #[test]
    fn default_listing_is_read_only() {
        let server = RustRssMcp::new(Store::open_in_memory().unwrap());
        let off = server.switches();
        assert_eq!(off, Switches::default(), "新库默认全关");

        let read_scope = server.visible_tools(Scope::Read, &off);
        assert_eq!(read_scope.len(), registry::read_tool_count());
        for tool in &read_scope {
            let spec = registry::spec(&tool.name).expect("列表里的工具必须已登记");
            assert_eq!(spec.scope, Scope::Read, "{} 不该出现在读会话里", tool.name);
        }

        // 写开关/写 token/危险开关都就位后，写 scope 的会话才看得到全部工具
        let on = Switches {
            write_enabled: true,
            dangerous_enabled: true,
            write_token: true,
        };
        let write_names: Vec<String> = server
            .visible_tools(Scope::Write, &on)
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(
            write_names.len(),
            registry::TOOL_SPECS.len(),
            "全开时写会话看到全部工具"
        );
        for name in [
            "set_read",
            "set_starred",
            "set_read_later",
            "refresh",
            "fetch_fulltext",
            // T4：订阅管理（`folder_delete` / `unsubscribe` 是危险工具，危险开关关着时不列）
            "subscribe",
            "update_feed",
            "folder_create",
            "folder_rename",
            "import_opml",
            "export_opml",
            // T4：标签（写工具全非危险；`list_tags` 是读工具，不在此列）
            "create_tag",
            "rename_tag",
            "assign_tags",
            "unassign_tags",
            "delete_tag",
        ] {
            assert!(write_names.contains(&name.to_string()), "{write_names:?}");
        }
    }


    /// 桦工具（测试专用）走同一套注册表：写桦工具在读 scope 下不可见、
    /// 在写 scope + 全开下可见。
    #[test]
    fn test_tools_use_the_same_registry() {
        let server = RustRssMcp::new(Store::open_in_memory().unwrap())
            .with_test_tool("stub_write", Scope::Write, false, "桦写工具", |_| "{}".into());
        let off = server.switches();
        let names = |scope: Scope, sw: &Switches| {
            server
                .visible_tools(scope, sw)
                .iter()
                .map(|t| t.name.to_string())
                .collect::<Vec<_>>()
        };

        assert!(!names(Scope::Read, &off).contains(&"stub_write".to_string()));
        assert!(!names(Scope::Write, &off).contains(&"stub_write".to_string()), "开关没开也不列");

        let on = Switches { write_enabled: true, dangerous_enabled: false, write_token: true };
        assert!(names(Scope::Write, &on).contains(&"stub_write".to_string()));
        assert!(!names(Scope::Read, &on).contains(&"stub_write".to_string()));
    }

    /// 凭据 → scope：库里的写 token → write；读 token → read；静态凭据 → read；其余 None。
    ///
    /// 关键回归：**每次调用都重新读库**——轮换/销毁后旧值下一次调用就不再命中。
    #[test]
    fn credential_scope_is_recomputed_on_every_call() {
        let store = Store::open_in_memory().unwrap();
        config::set_token(&store, "read-token").unwrap();
        let old_write = config::generate_write_token();
        config::set_write_token(&store, &old_write).unwrap();
        let server = RustRssMcp::new(store);

        assert_eq!(server.scope_of_credential("read-token", None), Some(Scope::Read));
        assert_eq!(server.scope_of_credential(&old_write, None), Some(Scope::Write));
        assert_eq!(server.scope_of_credential("cli-token", Some("cli-token")), Some(Scope::Read));
        assert_eq!(server.scope_of_credential("cli-token", Some("other")), None);
        assert_eq!(server.scope_of_credential("", None), None);
        assert_eq!(server.scope_of_credential("nope", None), None);

        // 轮换：旧写 token 立刻不再是 write（也不再是任何有效凭据的一半）
        let new_write = config::generate_write_token();
        server.with_store(|s| config::set_write_token(s, &new_write).unwrap());
        assert_eq!(server.scope_of_credential(&old_write, None), None, "旧写 token 必须立刻失效");
        assert_eq!(server.scope_of_credential(&new_write, None), Some(Scope::Write));

        // 销毁：新值也失效
        server.with_store(|s| config::clear_write_token(s).unwrap());
        assert_eq!(server.scope_of_credential(&new_write, None), None, "销毁后写 token 必须立刻失效");
        assert_eq!(server.scope_of_credential("read-token", None), Some(Scope::Read), "读 token 不受影响");
    }

    /// 日报只读工具口径：列表 ≤140 概览 + 分页上限；单取分页与缺报告报错。
    #[test]
    fn digest_tools_metadata_and_pagination() {
        let store = Store::open_in_memory().unwrap();
        // 造一个源 + 一条入窗条目（digest_entry_meta 由 upsert 维护）
        let feed = store.add_feed("https://a.invalid/rss", Some("A")).unwrap();
        let entries = rustrss_core::parse(
            "<rss version='2.0'><channel><title>T</title><item><guid>g1</guid><title>N1</title><description>D1</description><pubDate>2026-10-04T01:00:00Z</pubDate></item></channel></rss>".as_bytes(),
        ).unwrap().entries;
        store.upsert_entries(feed, &entries).unwrap();
        let start = 1791072000i64; // 2026-10-04T00:00Z
        let end = 1791122400i64;
        let manifest = store.freeze_manifest(start, end, None).unwrap();
        let content = serde_json::json!({
            "overview": "概".repeat(200), // >140 字：验证截断
            "sections": []
        });
        store
            .commit_digest_report(
                "2026-10-04", "UTC", start, end, "all", "{}", "p", "{}",
                &manifest.hash, &manifest.pairs_hash, &[],
                manifest.frozen_at, 0, 0,
                &content.to_string(), "# 日报", 1, "{}",
                manifest.entries.len() as i64, &manifest.entries,
            )
            .unwrap();

        let server = RustRssMcp::new(store);
        // digest_list：概览截到 ≤140 字
        let list = server.digest_list_json(&DigestListParams { limit: None });
        let v: serde_json::Value = serde_json::from_str(&list).unwrap();
        assert_eq!(v["count"], 1);
        let overview = v["digests"][0]["overview"].as_str().unwrap();
        assert_eq!(overview.chars().count(), 140, "概览必须截到 140 字");
        assert!(!list.contains(&"概".repeat(200)), "列表不得带出全文概览");
        // digest_list limit 上限 50：语义上 clamp 不报错（count 与库内报告数一致）
        let capped = server.digest_list_json(&DigestListParams { limit: Some(999) });
        let cv: serde_json::Value = serde_json::from_str(&capped).unwrap();
        assert_eq!(cv["count"], 1, "超大 limit 被 clamp，不报错也不放大");

        // digest_get：分页
        let get = server.digest_get_json(&DigestGetParams {
            date: "2026-10-04".into(),
            scope_key: Some("all".into()),
            items_page_size: Some(999), // clamp → 50
            items_page: Some(1),
        });
        let gv: serde_json::Value = serde_json::from_str(&get).unwrap();
        assert_eq!(gv["sources"]["page_size"], 50);
        assert_eq!(gv["article_count"], 1);

        // 缺报告 → 机器可读错误
        let missing = server.digest_get_json(&DigestGetParams {
            date: "2026-10-01".into(),
            scope_key: None,
            items_page_size: None,
            items_page: None,
        });
        assert!(missing.contains("未找到"), "缺报告必须报错：{missing}");
    }

    /// 应用内托管：MCP 与界面共用同一个刷新 gate（agent 的 refresh 不得与界面刷叠加）
    #[test]
    fn refresh_gate_is_shared_with_the_host() {
        let gate = Arc::new(RefreshGate::new());
        let server = RustRssMcp::new(Store::open_in_memory().unwrap())
            .with_refresh_gate(Arc::clone(&gate));
        assert!(Arc::ptr_eq(&server.refresh_gate(), &gate));

        // 模拟界面已经拿着单 flight：MCP 侧必须被同一个标记挡住
        let held = gate.try_begin().expect("界面应拿到单 flight");
        assert!(server.refresh_gate().try_begin().is_err(), "MCP 不得叠加刷新");
        drop(held);
        assert!(server.refresh_gate().try_begin().is_ok());
    }
}
