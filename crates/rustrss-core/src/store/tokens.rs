//! 检索用的预分词。
//!
//! 为什么需要：FTS5 的默认 `unicode61` 分词器按非字母数字切分，而中文没有空格，
//! 一整句中文会被当成**一个** token，导致中文检索完全不可用。
//! 因此入库前把文本转成「拉丁词 + 中文 bigram」的空格分隔串；查询侧做同样转换，
//! 中文短语用 FTS5 的短语查询保持顺序。

/// 把文本转成检索用 token 串（空格分隔，可直接写进 `entries.search_tokens`）。
pub fn to_tokens(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if is_cjk(chars[i]) {
            let start = i;
            while i < chars.len() && is_cjk(chars[i]) {
                i += 1;
            }
            let run = &chars[start..i];
            if run.len() >= 2 {
                for w in run.windows(2) {
                    push(&mut out, &w.iter().collect::<String>());
                }
            } else if let Some(c) = run.first() {
                push(&mut out, &c.to_string());
            }
        } else if chars[i].is_ascii_alphanumeric() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            push(&mut out, &word.to_lowercase());
        } else {
            i += 1;
        }
    }
    out
}

/// 查询转换结果：FTS5 表达式 + 需要走 LIKE 兜底的单字中文词。
pub struct QueryPlan {
    /// FTS5 MATCH 表达式；为空表示该查询无法用 FTS 表达
    pub fts: String,
    /// 单字中文词：中文 bigram 索引无法覆盖，改用 LIKE 过滤
    pub like_terms: Vec<String>,
}

/// 把用户查询转成可执行的检索计划。
pub fn plan_query(query: &str) -> QueryPlan {
    let chars: Vec<char> = query.chars().collect();
    let mut terms: Vec<String> = Vec::new();
    let mut like_terms: Vec<String> = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if is_cjk(chars[i]) {
            let start = i;
            while i < chars.len() && is_cjk(chars[i]) {
                i += 1;
            }
            let run: Vec<char> = chars[start..i].to_vec();
            if run.len() >= 2 {
                // 短语查询：bigram 顺序相邻，等于要求原文里连续出现
                let phrase: Vec<String> = run
                    .windows(2)
                    .map(|w| quote(&w.iter().collect::<String>()))
                    .collect();
                terms.push(phrase.join(" "));
            } else if let Some(c) = run.first() {
                like_terms.push(c.to_string());
            }
        } else if chars[i].is_ascii_alphanumeric() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            terms.push(quote(&word.to_lowercase()));
        } else {
            i += 1;
        }
    }

    QueryPlan {
        // FTS5 里以空格分隔即 AND
        fts: terms.join(" "),
        like_terms,
    }
}

/// FTS5 字符串字面量：双引号包裹，内部双引号翻倍
fn quote(token: &str) -> String {
    format!("\"{}\"", token.replace('"', "\"\""))
}

fn push(out: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(token);
}

/// 是否 CJK 字符（覆盖中日韩统一表意文字、扩展区、兼容区与假名）
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF        // 平假名 / 片假名
        | 0x3400..=0x4DBF      // 扩展 A
        | 0x4E00..=0x9FFF      // 基本区
        | 0xF900..=0xFAFF      // 兼容表意文字
        | 0x20000..=0x2FA1F    // 扩展 B 及以后
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_mix_latin_and_cjk_bigrams() {
        let t = to_tokens("Rust 1.99 发布：异步闭包改进");
        assert!(t.contains("rust"), "{t}");
        assert!(t.contains("1"), "{t}");
        assert!(t.contains("99"), "{t}");
        assert!(t.contains("发布"), "{t}");
        assert!(t.contains("异步"), "{t}");
        assert!(t.contains("步闭"), "{t}");
        assert!(!t.contains('：'), "标点不应进入 token：{t}");
    }

    #[test]
    fn query_plan_uses_phrase_for_multi_char_cjk() {
        let p = plan_query("异步闭包");
        assert_eq!(p.fts, "\"异步\" \"步闭\" \"闭包\"");
        assert!(p.like_terms.is_empty());
    }

    #[test]
    fn query_plan_falls_back_to_like_for_single_char() {
        let p = plan_query("架");
        assert!(p.fts.is_empty());
        assert_eq!(p.like_terms, vec!["架".to_string()]);
    }

    #[test]
    fn query_plan_ands_multiple_terms() {
        let p = plan_query("rust 异步");
        assert_eq!(p.fts, "\"rust\" \"异步\"");
    }
}
