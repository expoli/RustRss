//! MCP 的 HTTP（streamable HTTP）传输：回环地址 + token 鉴权。
//!
//! 设计选择与理由：
//! - **只绑回环**：`bind` 由调用方给，但这里额外断言是回环地址，避免被配置成 0.0.0.0；
//! - **token 鉴权**：接受 `Authorization: Bearer <token>` 或 `?token=<token>`。
//!   后者是给那些不方便自定义请求头的客户端用的（quick-rss 也是这个做法）。
//!   无 token / token 不对一律 401，不做“只读免鉴权”这类模糊地带。
//!   鉴权只罩 `/mcp`，`/health` 不鉴权（它不返回任何订阅数据）；
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

use crate::RustRssMcp;

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub bind: SocketAddr,
    /// 客户端必须携带的令牌；轮换即换值
    pub token: String,
}

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
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );

    let token = Arc::new(cfg.token);
    let guard = move |req: Request, next: Next| {
        let token = token.clone();
        async move {
            if authorized(&token, req.headers(), req.uri()) {
                next.run(req).await
            } else {
                (
                    StatusCode::UNAUTHORIZED,
                    "missing or invalid token; use `Authorization: Bearer <token>` or `?token=<token>`",
                )
                    .into_response()
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

/// 两种携带方式都接受：请求头（标准）与查询串（客户端不方便设头时用）
fn authorized(token: &str, headers: &header::HeaderMap, uri: &Uri) -> bool {
    let expected = token.as_bytes();
    if let Some(value) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let value = value.trim();
        let provided = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
            .unwrap_or(value);
        if constant_eq(provided.as_bytes(), expected) {
            return true;
        }
    }
    if let Some(query) = uri.query() {
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("token=") {
                if constant_eq(value.as_bytes(), expected) {
                    return true;
                }
            }
        }
    }
    false
}

/// 定长比较，避免用 `==` 做逐字节短路比较（本地服务，风险低，但习惯要正）
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
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
