//! 真实网络端到端验证：真实源 → 抓取 → 解析 → 入库 → 再刷新（观察 304 与去重）。
//!
//! 用法：cargo run -p rustrss-core --example refresh_real -- <db.sqlite> <url> [<url>...]

use rustrss_core::fetch::{refresh, Fetcher, DEFAULT_USER_AGENT};
use rustrss_core::{EntryQuery, Store};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let db = match args.next() {
        Some(d) => d,
        None => {
            eprintln!("用法: refresh_real <db.sqlite> <url> [<url>...]");
            std::process::exit(2);
        }
    };
    let urls: Vec<String> = args.collect();
    if urls.is_empty() {
        eprintln!("至少给一个订阅地址");
        std::process::exit(2);
    }

    let store = Store::open(&db).expect("打开数据库失败");
    let fetcher = Fetcher::new(DEFAULT_USER_AGENT).expect("构建 HTTP 客户端失败");

    let mut ids = Vec::new();
    for url in &urls {
        let id = store.add_feed(url, None).expect("加源失败");
        println!("订阅源 #{id} {url}");
        ids.push(id);
    }

    println!("\n=== 第 1 次刷新 ===");
    let report = refresh(&store, &fetcher, &ids, 4).await.expect("刷新失败");
    print_report(&report);
    print_feeds(&store);

    println!("\n=== 第 2 次刷新（观察条件请求：304 或去重后的「未变」）===");
    let report = refresh(&store, &fetcher, &ids, 4).await.expect("刷新失败");
    print_report(&report);

    println!("\n=== 库内现状 ===");
    println!("条目总数 {}，未读 {}", store.entry_count().unwrap(), store.unread_total().unwrap());
    for e in store
        .list_entries(&EntryQuery {
            limit: Some(3),
            ..Default::default()
        })
        .unwrap()
    {
        println!(
            "  · [{}] {}｜{} 字",
            e.feed_title,
            e.title,
            e.content_text.as_deref().map(|t| t.chars().count()).unwrap_or(0)
        );
    }
}

fn print_report(r: &rustrss_core::RefreshReport) {
    println!(
        "  成功 {}（新增 {} / 更新 {} / 未变 {}）｜未修改 {}｜失败 {}",
        r.fetched,
        r.inserted,
        r.updated,
        r.unchanged,
        r.not_modified,
        r.failures.len()
    );
    for f in &r.failures {
        println!("  ✗ #{} {} → {}", f.feed_id, f.url, f.error);
    }
}

fn print_feeds(store: &Store) {
    for f in store.list_feeds().unwrap() {
        println!(
            "  · #{} {}｜状态={}｜未读={}｜错误={}",
            f.id,
            f.title,
            f.last_status.as_deref().unwrap_or("-"),
            f.unread,
            f.last_error.as_deref().unwrap_or("-")
        );
    }
}
