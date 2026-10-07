//! 每日日报的生成编排（同步核心件）。
//!
//! 异步驱动在 Tauri 命令层（锁外发请求），本模块提供可持锁调用的同步件：
//! 冻结素材清单、输入哈希、分组、合成请求构造、节点缓存、提交。
//! 设计：`.chorus/specs/rss-reader/2026-10-03-daily-digest/design.md` §6。

use crate::ai::prompt::{self, AiRequest, AiTask};
use crate::store::digest::{sha256_hex, ManifestEntry};
use crate::store::Store;

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
///
/// 直接构造请求（不经 `prompt::prepare`）：合成输入由分组预算控制总量，
/// 12k 静默截断在这里会静默丢小节——预算语义由分组层负责（审核 P1-7）。
pub fn final_request(language: &str, day: &str, sections: &[String]) -> AiRequest {
    let mut body = String::new();
    for (i, section) in sections.iter().enumerate() {
        body.push_str(&format!("—— 第 {} 部分 ——\n{section}\n\n", i + 1));
    }
    AiRequest {
        system: Some(
            "你是每日日报的编辑。把给定的要点素材汇编成一份连贯的日报。输出 Markdown。"
                .to_string(),
        ),
        user: format!(
            "日期：{day}\n\n{body}\n\n请用{language}把以上素材汇编成一份每日日报：\n- 以一段 2-3 句的总览开头；\n- 按「今天发生了什么」的意义分成小节，每节以 `## ` 标题开始；\n- 保留具体事实（数字、名称、结果），不要空话；\n- 末尾不要总结陈词。"
        ),
    }
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

/// 日报单篇要点的计划：请求已构造、缓存键已算出（同步，可持锁调用）。
///
/// 与通用 `plan_task` 的三点差异（设计 §6.2 / 审核 P1-1）：
/// 1. 正文用**冻结清单**里的（不重读当前文章——生成期间正文可能已变）；
/// 2. 缓存 params 含**实际输入哈希**（正文变化自动失效）；
/// 3. 缓存 params 含**端点身份**（provider/model + base_url，换端点不命中旧要点）。
pub struct ItemPlan {
    pub request: AiRequest,
    pub truncated: bool,
    pub cached: Option<String>,
}

/// 缓存 params 的字段分隔符（U+001F，正文不可打印字符，避免与内容冲突）。
const SEP: char = '\u{1f}';

/// 端点身份标识（进缓存键）：provider/model + base_url 的组合摘要。
pub fn endpoint_identity(client: &crate::ai::AiClient) -> String {
    let cfg = client.config();
    sha256_hex(&[&cfg.cache_tag(), &cfg.base_url])
}

/// 单篇要点：构造请求 + 查缓存。
/// `provider_model` 来自 `client.config().cache_tag()`。
pub fn plan_item(
    store: &Store,
    entry: &ManifestEntry,
    language: &str,
    provider_model: &str,
    endpoint: &str,
) -> Result<ItemPlan, crate::store::StoreError> {
    let input_hash = sha256_hex(&[&entry.title, &entry.body]);
    let params = format!("{language}{SEP}{input_hash}{SEP}{endpoint}");
    let cached = store.digest_item_cached(entry.entry_id, &params, provider_model)?;
    Ok(ItemPlan {
        request: item_request(language, entry),
        truncated: entry.truncated,
        cached,
    })
}

/// 落缓存：键与 [`plan_item`] 同构。
pub fn save_item(
    store: &Store,
    entry: &ManifestEntry,
    language: &str,
    provider_model: &str,
    endpoint: &str,
    output: &str,
) -> Result<(), crate::store::StoreError> {
    let input_hash = sha256_hex(&[&entry.title, &entry.body]);
    let params = format!("{language}{SEP}{input_hash}{SEP}{endpoint}");
    store.digest_item_store(entry.entry_id, &params, provider_model, output)
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

/// 节点缓存键：阶段 + 日期 + 语言 + 生成配置身份 + 子输入/输出哈希
/// （内容寻址，不含生成时间；换模型/端点自动不命中——审核 P1-3）。
pub fn node_key(kind: &str, day: &str, language: &str, config_tag: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"digest-node-v1");
    hasher.update(crate::ai::prompt::PROMPT_VERSION.as_bytes());
    hasher.update(kind.as_bytes());
    hasher.update(day.as_bytes());
    hasher.update(language.as_bytes());
    hasher.update(config_tag.as_bytes());
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
    let mut seen_content = false;
    for line in md.lines() {
        // ## 断节；顶部单独的 H1 总标题行直接丢弃（UI 头部已有日期标题，
        // 保留会以 escape 文本裸露 # 语法）；正文中的 H1 当 H2 断节
        if let Some(title) = line.strip_prefix("## ") {
            if let Some((t, ls)) = current.take() {
                sections.push((t, ls.join("\n").trim().to_string()));
            }
            current = Some((title.trim().to_string(), Vec::new()));
            continue;
        }
        if line.strip_prefix("# ").is_some() {
            if seen_content {
                if let Some((t, ls)) = current.take() {
                    sections.push((t, ls.join("\n").trim().to_string()));
                }
                current = Some((line[2..].trim().to_string(), Vec::new()));
            }
            continue;
        }
        if !line.trim().is_empty() {
            seen_content = true;
        }
        match &mut current {
            Some((_, ls)) => ls.push(line),
            None => overview_lines.push(line),
        }
    }
    if let Some((t, ls)) = current.take() {
        sections.push((t, ls.join("\n").trim().to_string()));
    }
    // overview/sections 里的行内 markdown 语法剥除（**粗体**、`代码`）
    let strip = |text: &str| text.replace("**", "").replace('`', "");
    let overview = strip(overview_lines.join("\n").trim());
    let overview = overview.trim().to_string();
    let sections = sections
        .into_iter()
        .map(|(t, text)| (strip(&t), strip(&text)))
        .collect();
    (overview, sections)
}

#[cfg(test)]
mod parse_tests {
    use super::parse_digest_markdown;

    #[test]
    fn h1_and_h2_both_break_sections() {
        let (overview, sections) = parse_digest_markdown(
            "# 每日日报 2026-10-07\n**总览**： 今天很热闹。\n\n## OpenAI 发布\n- 要点一\n\n## 安全动态\n- 要点二",
        );
        // H1 与引导段落后首个 H2 断节：overview 不含 H1 行与 ** 语法
        assert!(overview.contains("今天很热闹"), "{overview}");
        assert!(!overview.contains('#'), "{overview}");
        assert!(!overview.contains("**"), "{overview}");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "OpenAI 发布");
        assert_eq!(sections[1].0, "安全动态");
    }

    #[test]
    fn inline_markup_stripped_from_overview_and_sections() {
        let (overview, sections) = parse_digest_markdown(
            "**总览**： `重点`内容\n\n## 小节\n- **粗体**列表 与 `代码` 混排",
        );
        assert!(!overview.contains("**") && !overview.contains('`'));
        assert!(!sections[0].1.contains("**") && !sections[0].1.contains('`'));
        assert!(sections[0].1.contains("粗体列表"));
    }

    #[test]
    fn no_heading_means_single_overview() {
        let (overview, sections) = parse_digest_markdown("只有一段话。");
        assert_eq!(sections.len(), 0);
        assert_eq!(overview, "只有一段话。");
    }
}
