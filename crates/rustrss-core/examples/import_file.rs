//! 手动验证工具：把真实 feed 文件导入 SQLite，并演示去重与检索。
//!
//! 用法：cargo run -p rustrss-core --example import_file -- <feed.xml> <db.sqlite> [search]

use rustrss_core::{parse, EntryQuery, Store};

fn main() {
    let mut args = std::env::args().skip(1);
    let (feed_path, db_path, query) = match (args.next(), args.next()) {
        (Some(f), Some(d)) => (f, d, args.next()),
        _ => {
            eprintln!("用法: import_file <feed.xml> <db.sqlite> [search]");
            std::process::exit(2);
        }
    };

    let bytes = std::fs::read(&feed_path).expect("读取 feed 文件失败");
    let feed = parse(&bytes).expect("解析 feed 失败");

    let store = Store::open(&db_path).expect("打开数据库失败");
    let feed_id = store
        .add_feed(&format!("file://{feed_path}"), Some(&feed.title))
        .expect("加源失败");

    let stats = store.upsert_entries(feed_id, &feed.entries).expect("入库失败");

    println!("源      : {}（{} 条）", feed.title, feed.entries.len());
    println!(
        "本次入库: 新增 {} / 更新 {} / 未变 {}",
        stats.inserted, stats.updated, stats.unchanged
    );
    println!("库内条目: {}（未读 {}）", store.entry_count().unwrap(), store.unread_total().unwrap());

    let latest = store
        .list_entries(&EntryQuery {
            feed_id: Some(feed_id),
            limit: Some(2),
            ..Default::default()
        })
        .unwrap();
    for e in latest {
        println!(
            "  · [{}] {}｜来源={}｜正文={} 字",
            e.stable_id,
            e.title,
            e.id_origin,
            e.content_text.as_deref().map(|t| t.chars().count()).unwrap_or(0)
        );
    }

    if let Some(q) = query {
        let hits = store.search(&q, 5).unwrap();
        println!("检索「{q}」命中 {} 条：", hits.len());
        for h in hits.iter().take(3) {
            println!("  · {}", h.title);
        }
    }
}
