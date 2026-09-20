//! 极简 HTML → 纯文本。
//!
//! 用途是给搜索索引和 MCP 输出提供不含标签的正文，**不承担安全清洗职责**——
//! 正文渲染侧的安全清洗是另一件事（见 spec 的「正文渲染」验收点）。

/// 去标签、剔除 script/style、解少量实体、压缩空白，块级标签保留换行。
pub fn html_to_text(html: &str) -> String {
    const BLOCK_TAGS: [&str; 16] = [
        "p", "div", "br", "li", "ul", "ol", "tr", "table", "h1", "h2", "h3", "h4", "h5", "h6",
        "blockquote", "pre",
    ];

    // 只做 ASCII 小写化：字节长度与 char 边界都保持不变，因此下面的索引可以共用
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;

    while i < lower.len() {
        let rest = &lower[i..];

        // script / style 的内容不进纯文本
        if rest.starts_with("<script") || rest.starts_with("<style") {
            let close = if rest.starts_with("<script") { "</script>" } else { "</style>" };
            match lower[i..].find(close) {
                Some(pos) => {
                    i += pos + close.len();
                    continue;
                }
                None => break, // 未闭合，直接结束
            }
        }

        if rest.starts_with('<') {
            match rest.find('>') {
                Some(pos) => {
                    let tag = &rest[1..pos];
                    let name: String = tag
                        .trim_start_matches('/')
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric())
                        .collect();
                    if BLOCK_TAGS.contains(&name.to_ascii_lowercase().as_str()) {
                        out.push('\n');
                    }
                    i += pos + 1;
                    continue;
                }
                None => break, // 未闭合的 '<'，收尾
            }
        }

        match html[i..].chars().next() {
            Some(ch) => {
                out.push(ch);
                i += ch.len_utf8();
            }
            None => break,
        }
    }

    collapse_whitespace(&decode_entities(&out))
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
}

/// 连续空白压成一个空格；换行保留但去掉行首行尾空白，并合并空行。
fn collapse_whitespace(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut last_space = false;

    for c in s.chars() {
        match c {
            '\n' | '\r' => {
                lines.push(current.trim().to_string());
                current.clear();
                last_space = false;
            }
            _ if c.is_whitespace() => {
                if !last_space && !current.is_empty() {
                    current.push(' ');
                    last_space = true;
                }
            }
            _ => {
                current.push(c);
                last_space = false;
            }
        }
    }
    lines.push(current.trim().to_string());

    lines
        .into_iter()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
