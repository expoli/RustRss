//! RustRss 核心库：订阅源解析、身份判定、存储与抓取。
//!
//! 与界面、Tauri 解耦：GUI 与 MCP 服务器共用这一层的查询逻辑，
//! 避免「两套查询逻辑漂移」（见 PRD §7 架构约束）。

pub mod ai;
pub mod discover;
pub mod fetch;
pub mod fulltext;
pub mod html;
pub mod logging;
pub mod model;
pub mod opml;
pub mod parse;
pub mod paths;
pub mod refresh_flight;
pub mod rsshub;
pub mod store;

pub use ai::{AiClient, AiConfig, AiError, Provider, RequestPreview};
pub use discover::{discover, DiscoverError, Discovery, DiscoveryVia};
pub use fetch::{
    apply_results, bounded_map, collect_jobs, fetch_jobs, fetch_jobs_with_progress, refresh,
    refresh_all, Fetcher, FetchResult, RefreshProgress, RefreshReport,
};
pub use fulltext::{extract, extract_bytes, Extracted, FulltextError};
pub use paths::{default_data_dir, default_db_path, resolve_db_path};
pub use logging::scrub_log_line;
pub use model::{Entry, Feed, IdOrigin};
pub use refresh_flight::{RefreshFlight, RefreshGate};
pub use parse::{parse, ParseError};
pub use store::backup;
pub use store::{
    EntryFlag, EntryFlagScope, EntryQuery, EntryRow, FeedIntervalRow, FeedRow, InsertStats,
    ListSort, MarkScope, Store, StoreError, UnreadGroup, UnreadGroupBy,
};
