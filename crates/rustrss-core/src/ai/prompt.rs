//! AI 任务的 prompt 构造。
//!
//! 两条约定：
//! - `PROMPT_VERSION` 变化会改变缓存键，因此改 prompt 后旧摘要自动失效，不会留下过期结果；
//! - 超长正文**截断并显式标注**（而不是静默丢弃），先保证不因超长而失败。

use serde::{Deserialize, Serialize};

/// 改动 prompt 就把它加一（缓存键含此值）
pub const PROMPT_VERSION: &str = "v1";

/// 单次送入模型的正文上限（字符）。
/// 分块 + 映射归并的做法留待后续；先保证行为可预测、可见。
pub const MAX_INPUT_CHARS: usize = 12_000;

/// 送进模型的一次请求（provider 差异在 ai 层消化）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AiRequest {
    pub system: Option<String>,
    pub user: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryLength {
    Short,
    Medium,
    Long,
}

impl SummaryLength {
    pub fn max_chars(self) -> usize {
        match self {
            SummaryLength::Short => 80,
            SummaryLength::Medium => 200,
            SummaryLength::Long => 400,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SummaryLength::Short => "short",
            SummaryLength::Medium => "medium",
            SummaryLength::Long => "long",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AiTask {
    /// 摘要：`language` 是输出语言（如「中文」）
    Summarize {
        length: SummaryLength,
        language: String,
    },
    /// 翻译：`target` 是目标语言
    Translate { target: String },
}

impl AiTask {
    pub fn name(&self) -> &'static str {
        match self {
            AiTask::Summarize { .. } => "summarize",
            AiTask::Translate { .. } => "translate",
        }
    }

    /// 缓存键的一部分：同一篇文章 + 同一任务 + 同一参数才复用结果
    pub fn cache_params(&self) -> String {
        match self {
            AiTask::Summarize { length, language } => {
                format!("{}:{}", length.as_str(), language)
            }
            AiTask::Translate { target } => target.clone(),
        }
    }
}

/// 待处理的文章
#[derive(Debug, Clone, Copy)]
pub struct ArticleText<'a> {
    pub title: &'a str,
    pub body: &'a str,
}

/// 按上限截断正文；返回 (文本, 是否截断)
pub fn prepare(body: &str) -> (String, bool) {
    let trimmed = body.trim();
    if trimmed.chars().count() <= MAX_INPUT_CHARS {
        return (trimmed.to_string(), false);
    }
    let cut: String = trimmed.chars().take(MAX_INPUT_CHARS).collect();
    (
        format!("{cut}\n\n（正文过长，已截断，以上为前 {MAX_INPUT_CHARS} 字）"),
        true,
    )
}

/// 是否会被截断（不复制文本，只做判断）
pub fn was_truncated(body: &str) -> bool {
    body.trim().chars().count() > MAX_INPUT_CHARS
}

pub fn build(task: &AiTask, article: &ArticleText<'_>) -> AiRequest {
    let (body, truncated) = prepare(article.body);
    let title = if article.title.trim().is_empty() {
        "（无标题）"
    } else {
        article.title.trim()
    };
    let _ = truncated;

    match task {
        AiTask::Summarize { length, language } => AiRequest {
            system: Some(
                "你是一个 RSS 阅读助手。只输出摘要正文，不要前言、解释、标题或 Markdown 标记。"
                    .to_string(),
            ),
            user: format!(
                "标题：{title}\n\n正文：\n{body}\n\n请用{language}写一段不超过 {} 字的摘要。",
                length.max_chars()
            ),
        },
        AiTask::Translate { target } => AiRequest {
            system: Some("你是翻译助手。只输出译文，不要解释、不要原文对照、不要 Markdown 标记。".to_string()),
            user: format!("把下面这篇文章翻译成{target}：\n\n标题：{title}\n\n正文：\n{body}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_body_is_not_truncated() {
        let (text, truncated) = prepare("  很短的一段正文  ");
        assert_eq!(text, "很短的一段正文");
        assert!(!truncated);
    }

    #[test]
    fn long_body_is_truncated_and_marked() {
        let long = "字".repeat(MAX_INPUT_CHARS + 500);
        let (text, truncated) = prepare(&long);
        assert!(truncated, "超长正文必须报告被截断");
        assert!(text.contains("已截断"), "用户应能从文本本身看出被截断");
        assert!(text.chars().count() < long.chars().count());
    }

    #[test]
    fn summarize_request_carries_title_and_length() {
        let req = build(
            &AiTask::Summarize {
                length: SummaryLength::Short,
                language: "中文".into(),
            },
            &ArticleText {
                title: "标题A",
                body: "正文B",
            },
        );
        assert!(req.user.contains("标题A"));
        assert!(req.user.contains("正文B"));
        assert!(req.user.contains("不超过 80 字"));
        assert!(req.system.unwrap().contains("只输出摘要正文"));
    }

    #[test]
    fn translate_request_names_the_target_language() {
        let req = build(
            &AiTask::Translate {
                target: "英文".into(),
            },
            &ArticleText {
                title: "标题",
                body: "正文",
            },
        );
        assert!(req.user.contains("翻译成英文"));
    }

    #[test]
    fn cache_params_distinguish_task_variants() {
        let short = AiTask::Summarize {
            length: SummaryLength::Short,
            language: "中文".into(),
        };
        let long = AiTask::Summarize {
            length: SummaryLength::Long,
            language: "中文".into(),
        };
        let en = AiTask::Summarize {
            length: SummaryLength::Short,
            language: "英文".into(),
        };
        assert_ne!(short.cache_params(), long.cache_params());
        assert_ne!(short.cache_params(), en.cache_params());
        assert_eq!(
            short.cache_params(),
            AiTask::Summarize {
                length: SummaryLength::Short,
                language: "中文".into()
            }
            .cache_params(),
            "相同参数必须得到相同缓存键，否则缓存永远不命中"
        );
    }
}
