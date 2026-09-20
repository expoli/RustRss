//! 手动验证工具：把一个 feed 文件解析并打印摘要。
//!
//! 用法：cargo run -p rustrss-core --example parse_file -- <feed.xml|feed.json>

use rustrss_core::{parse, IdOrigin};

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("用法: parse_file <feed.xml|feed.json>");
            std::process::exit(2);
        }
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("读取 {path} 失败: {e}");
            std::process::exit(1);
        }
    };

    match parse(&bytes) {
        Ok(feed) => {
            println!("标题   : {}", feed.title);
            println!("站点   : {}", feed.site_url.as_deref().unwrap_or("(无)"));
            println!("语言   : {}", feed.language.as_deref().unwrap_or("(无)"));
            println!("条目数 : {}", feed.entries.len());
            let origins = feed
                .entries
                .iter()
                .filter(|e| e.id_origin == IdOrigin::ContentHash)
                .count();
            println!("兜底身份条目: {origins} 条（既无 guid 也无 link，用内容指纹）");
            for e in feed.entries.iter().take(3) {
                println!(
                    "  · [{}] {} | 时间={} | 来源={:?} | 正文={}",
                    e.stable_id,
                    e.title,
                    e.published.map(|d| d.to_rfc3339()).unwrap_or_else(|| "(无)".into()),
                    e.id_origin,
                    match (e.content_html.is_some(), e.content_text.as_deref()) {
                        (true, Some(t)) => format!("html+text({} 字)", t.chars().count()),
                        (false, Some(t)) => format!("text({} 字)", t.chars().count()),
                        (true, None) => "html".to_string(),
                        (false, None) => "(无)".to_string(),
                    }
                );
            }
        }
        Err(e) => {
            eprintln!("解析失败: {e}");
            std::process::exit(1);
        }
    }
}
