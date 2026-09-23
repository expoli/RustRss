//! MCP 的 HTTP（streamable HTTP）传输：回环地址 + token 鉴权。
//!
//! 设计选择与理由：
//! - **只绑回环**：`bind` 由调用方给，但这里额外断言是回环地址，避免被配置成 0.0.0.0；
//! - **token 鉴权**：接受 `Authorization: Bearer <token>` 或 `?token=<token>`。
//!   后者是给那些不方便自定义请求头的客户端用的（quick-rss 也是这个做法）。
//!   无 token / token 不对一律 401，不做“只读免鉴权”这类模糊地带。
//!   鉴权只罩 `/mcp`，`/health` 不鉴权（它不返回任何订阅数据）；
//! - **每个请求现算 scope**：鉴权中间件把凭据交给 [`RustRssMcp::scope_of_credential`]，
//!   后者**每个请求都重新读库**里的 `mcp.token` / `mcp.write_token`——所以写 token
//!   轮换或销毁后，旧值在下一个请求（含已建立的连接）立即失效，不靠重启服务；
//!   算出的 scope 注入请求扩展（[`RequestScope`]），handler 侧直接读，不在服务实例上
//!   存任何会话级权限；
//! - **无会话（stateless）+ JSON 响应**：新版 MCP 规范已移除会话（SEP-2567），
//!   而且无会话意味着客户端不需要回传 `Mcp-Session-Id`，配置更省心；
//! - **客户端必须带 `Accept: application/json, text/event-stream`**：这是 MCP 传输规范
//!   的要求（rmcp 会因缺失而返回 406），不是我们的额外限制。

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use tokio::sync::oneshot;

use crate::registry::Scope;
use crate::RustRssMcp;

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub bind: SocketAddr,
    /// 启动时下发的**静态读凭据**：CLI 用（`RUSTSS_MCP_TOKEN`，或未设时随机生成的那一串）。
    /// `None` = 只认库里的值——应用内托管走这条路：token 由设置页生成/轮换/销毁，
    /// 服务本身不持副本，所以轮换不需要重启服务也立即生效。
    pub token: Option<String>,
}

/// 当次请求的 scope：由鉴权中间件按「请求携带的凭据 + 库里此刻的 token」算出后注入。
///
/// 单独一个 newtype（而不是直接塞 [`Scope`]）是为了让这份扩展只有一个来源，
/// 不会与将来别的 `Scope` 语义撞车。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestScope {
    pub scope: Scope,
}

#[derive(Clone)]
pub struct RequestIdentity(pub String);

#[derive(Debug, thiserror::Error)]
pub enum HttpServeError {
    #[error("HTTP 服务只允许绑定回环地址，收到 {0}")]
    NotLoopback(SocketAddr),
    #[error("绑定 {0} 失败: {1}")]
    Bind(SocketAddr, std::io::Error),
    #[error("启动失败: {0}")]
    Serve(String),
}

/// 运行中的 HTTP 服务句柄：`shutdown()` 后端口会被释放
#[derive(Debug)]
pub struct HttpHandle {
    pub addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
}

impl HttpHandle {
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// 请求停止（尽力而为：服务端会退出 accept 循环）
    pub fn shutdown(self) {
        let _ = self.shutdown.send(());
    }
}

/// 生成一个新的令牌（UUIDv4 的 32 位十六进制，够用且好复制）
pub fn generate_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub async fn serve(server: RustRssMcp, cfg: HttpConfig) -> Result<HttpHandle, HttpServeError> {
    if !cfg.bind.ip().is_loopback() {
        return Err(HttpServeError::NotLoopback(cfg.bind));
    }
    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .map_err(|e| HttpServeError::Bind(cfg.bind, e))?;
    let addr = listener.local_addr().map_err(|e| HttpServeError::Serve(e.to_string()))?;

    // 无会话 + JSON 响应：客户端不必回传 session id。
    // 注意 StreamableHttpServerConfig 是 non_exhaustive，不能用结构体字面量构造。
    let mut mcp_config = StreamableHttpServerConfig::default();
    mcp_config.legacy_session_mode = false;
    mcp_config.json_response = true;
    // 鉴权中间件要一个服务句柄（读库现算 scope），但 `server` 会被搬进下面的工厂闭包
    let server_for_auth = server.clone();
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );

    // 鉴权用的服务句柄：与 handler 共用同一个库连接——每个请求都拿它现读 token。
    let auth_server = server_for_auth;
    let static_token: Option<Arc<str>> = cfg.token.clone().map(Arc::from);
    let guard = move |mut req: Request, next: Next| {
        let server = auth_server.clone();
        let static_token = static_token.clone();
        async move {
            match authenticate(&server, static_token.as_deref(), req.headers(), req.uri()) {
                Some(scope) => {
                    let identity = crate::preview::fingerprint(presented_token(req.headers(), req.uri()).unwrap_or_default());
                    req.extensions_mut().insert(RequestIdentity(identity));
                    req.extensions_mut().insert(RequestScope { scope });
                    next.run(req).await
                }
                None => (
                    StatusCode::UNAUTHORIZED,
                    "missing or invalid token; use `Authorization: Bearer <token>` or `?token=<token>`",
                )
                    .into_response(),
            }
        }
    };

    // 鉴权只罩 /mcp：/health 只需报告存活，不含任何订阅数据，不应要求 token
    let protected = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(guard));
    let app = Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .merge(protected);

    let (tx, rx) = oneshot::channel::<()>();
    let server_task = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = rx.await;
    });
    tokio::spawn(async move {
        if let Err(e) = server_task.await {
            eprintln!("[rustrss-mcp] HTTP 服务退出: {e}");
        }
    });

    Ok(HttpHandle {
        addr,
        shutdown: tx,
    })
}

/// 鉴权 + 现算 scope：命中返回该请求的 scope，否则 `None`（→ 401）。
///
/// 顺序：库里此刻的写 token（→ `write`）→ 库里此刻的读 token / 静态读 token（→ `read`）。
/// 库里读不出来（连接被毒化）时只认静态 token——fail-closed，不猜。
fn authenticate(
    server: &RustRssMcp,
    static_token: Option<&str>,
    headers: &header::HeaderMap,
    uri: &Uri,
) -> Option<Scope> {
    let presented = presented_token(headers, uri)?;
    server.scope_of_credential(&presented, static_token)
}

/// 两种携带方式都接受：请求头（标准）与查询串（客户端不方便设头时用）。
/// 返回**原样的凭据串**（比较交给调用方，避免两处各自实现常量时间比较）。
fn presented_token(headers: &header::HeaderMap, uri: &Uri) -> Option<String> {
    if let Some(value) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let value = value.trim();
        let provided = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
            .unwrap_or(value);
        if !provided.is_empty() {
            return Some(provided.to_string());
        }
    }
    if let Some(query) = uri.query() {
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("token=") {
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// 单个凭据与期望值的比较（仅测试用：生产走
/// [`authenticate`] + [`RustRssMcp::scope_of_credential`]——库里可能有两个 token，
/// 单一期望值表达不了）。保留它是因为它恰好把「两种携带方式都能认」框在一个断言里。
#[cfg(test)]
fn authorized(token: &str, headers: &header::HeaderMap, uri: &Uri) -> bool {
    presented_token(headers, uri)
        .map(|provided| constant_eq(provided.as_bytes(), token.as_bytes()))
        .unwrap_or(false)
}

/// 定长比较，避免用 `==` 做逐字节短路比较（本地服务，风险低，但习惯要正）。
///
/// `pub(crate)`：凭据判定在 lib 侧的 `scope_of_credential`，比较实现只能有一份。
pub(crate) fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 便于调用方判断「这个地址能不能拿来绑」
pub fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_must_match_exactly() {
        let uri: Uri = "/mcp?token=abc".parse().unwrap();
        let mut headers = header::HeaderMap::new();
        assert!(authorized("abc", &headers, &uri));
        assert!(!authorized("abcd", &headers, &uri));
        assert!(!authorized("ab", &headers, &uri));

        headers.insert(header::AUTHORIZATION, "Bearer xyz".parse().unwrap());
        let plain: Uri = "/mcp".parse().unwrap();
        assert!(authorized("xyz", &headers, &plain));
        assert!(!authorized("abc", &headers, &plain));
    }

    #[test]
    fn tokens_are_random_and_long_enough() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
