//! 每日日报的生成编排（同步核心件）。
//!
//! 异步驱动在 Tauri 命令层（锁外发请求），本模块提供可持锁调用的同步件：
//! 冻结素材清单、输入哈希、分组、合成请求构造、节点缓存、提交。
//! 设计：`.chorus/specs/rss-reader/2026-10-03-daily-digest/design.md` §6。

use crate::ai::prompt::{self, AiRequest, AiTask};
use crate::store::digest::ManifestEntry;

/// 组合成的请求：把一组要点合成小节摘要。
pub fn group_request(language: &str, day: &str, group: &[(String, String)]) -> AiRequest {
    let mut body = String::new();
    for (title, points) in group {
        body.push_str(&format!("【{title}】\n{points}\n\n"));
    }
    AiRequest {
        system: Some(
            "你是每日日报的编辑。把同一天的一组文章要点合并成一小节摘要，去除重复、保留事实。输出 Markdown。"
                .to_string(),
        ),
        user: format!("日期：{day}\n\n{body}\n\n请用{language}合并成一小节（不超过 300 字），保留具体事实。"),
    }
}

/// 最终合成请求：把各组小节合成完整日报。
pub fn final_request(language: &str, day: &str, sections: &[String]) -> AiRequest {
    let mut body = String::new();
    for (i, section) in sections.iter().enumerate() {
        body.push_str(&format!("—— 第 {} 部分 ——\n{section}\n\n", i + 1));
    }
    prompt::build(
        &AiTask::DigestCompose { language: language.into() },
        &prompt::ArticleText { title: day, body: &body },
    )
}


/// 日报单篇要点任务（走 ai_cache，跨日期复用）。
pub fn item_task(language: &str) -> AiTask {
    AiTask::DigestItem { language: language.into() }
}

/// 单篇要点请求（编排层在缓存未命中时发这个）。
pub fn item_request(language: &str, entry: &ManifestEntry) -> AiRequest {
    prompt::build(
        &item_task(language),
        &prompt::ArticleText { title: &entry.title, body: &entry.body },
    )
}

/// 一组要点素材（标题 + 要点文本）。
pub type GroupEntry = (String, String);

/// 按字符预算把要点贪心分组（保持 effective_at 顺序）。
pub fn group_key_points(
    entries: &[GroupEntry],
    budget_chars: usize,
) -> Vec<Vec<GroupEntry>> {
    let mut groups: Vec<Vec<GroupEntry>> = vec![Vec::new()];
    let mut used = 0usize;
    for (title, points) in entries {
        let cost = title.chars().count() + points.chars().count();
        let group = groups.last_mut().unwrap();
        if used + cost > budget_chars && !group.is_empty() {
            groups.push(Vec::new());
            used = 0;
        }
        used += cost;
        groups.last_mut().unwrap().push((title.clone(), points.clone()));
    }
    groups
}

/// 节点缓存键：阶段 + 日期 + 语言 + 子输入哈希（内容寻址，不含生成时间）。
pub fn node_key(kind: &str, day: &str, language: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"digest-node-v1");
    hasher.update(kind.as_bytes());
    hasher.update(day.as_bytes());
    hasher.update(language.as_bytes());
    for p in parts {
        hasher.update(b"\x1e");
        hasher.update(p.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// 把合成输出的 Markdown 解析为结构化内容（overview + `## ` 小节）。
/// 解析是确定性的：没有 `## ` 时整体作为 overview、无小节。
pub fn parse_digest_markdown(md: &str) -> (String, Vec<(String, String)>) {
    let mut overview_lines: Vec<&str> = Vec::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in md.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            if let Some((t, ls)) = current.take() {
                sections.push((t, ls.join("\n").trim().to_string()));
            }
            current = Some((title.trim().to_string(), Vec::new()));
            continue;
        }
        match &mut current {
            Some((_, ls)) => ls.push(line),
            None => overview_lines.push(line),
        }
    }
    if let Some((t, ls)) = current.take() {
        sections.push((t, ls.join("\n").trim().to_string()));
    }
    let overview = overview_lines.join("\n").trim().to_string();
    (overview, sections)
}
