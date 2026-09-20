//! RustRss 核心库：订阅源解析、身份判定、存储与抓取。
//!
//! 与界面、Tauri 解耦：GUI 与 MCP 服务器共用这一层的查询逻辑，
//! 避免「两套查询逻辑漂移」（见 PRD §7 架构约束）。

pub mod ai;
pub mod fetch;
pub mod html;
pub mod model;
pub mod parse;
pub mod store;

pub use ai::{AiClient, AiConfig, AiError, Provider, RequestPreview};
pub use fetch::{bounded_map, refresh, refresh_all, Fetcher, FetchResult, RefreshReport};
pub use model::{Entry, Feed, IdOrigin};
pub use parse::{parse, ParseError};
pub use store::{EntryQuery, EntryRow, FeedRow, InsertStats, MarkScope, Store, StoreError};
