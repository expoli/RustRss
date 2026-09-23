//! 日志行打码：落盘 / 入审计前抹掉凭据形态。
//!
//! 从这里（core）而不是 `src-tauri` 提供：日志设施本身（[`super`]）在 core，
//! MCP 二进制（`rustrss-mcp`）也要在审计行里打码，而它不能依赖 Tauri。
//! 唯一实现、三处调用（界面 `[ui]` 行、逐源失败行、MCP 写审计行）。

/// 日志行打码：抹掉 URL 里的凭据形态——userinfo（`https://user:pass@host/x`）与
/// 查询串里的敏感参数值（`?key=…` / `&token=…`）。
///
/// 为什么在这个口子上做：前端/调用方无从知道「哪个值是凭据」，而日志一旦落盘就会
/// 被用户贴进 issue。只动已知的凭据**形态**，正常日志行（`view=all count=3`）原样通过。
/// 边界：不处理 `Authorization: Bearer …` 这类**请求头**——现有日志点都不打印请求头
/// （逐点审查见 MCP 写能力的 T2 报告）；将来要打请求头，先在调用点按 AI 预览的
/// `***已隐藏***` 口径打码。
pub fn scrub_log_line(line: &str) -> String {
    const SENSITIVE: &[&str] = &[
        "key",
        "apikey",
        "api_key",
        "api-key",
        "token",
        "access_token",
        "refresh_token",
        "secret",
        "client_secret",
        "password",
        "passwd",
    ];
    /// 值的结束位置：下一个参数（`&`）、终止展示符，或空白。
    fn value_end(rest: &str) -> usize {
        rest.find(|c: char| {
            c == '&' || c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>' | ',' | ']' | '}')
        })
        .unwrap_or(rest.len())
    }

    let line = mask_url_userinfo(line);
    let mut out = String::with_capacity(line.len());
    let mut rest = line.as_str();
    loop {
        let Some(sep) = rest.find(['?', '&']) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..=sep]);
        let tail = &rest[sep + 1..];
        let Some(eq) = tail.find('=') else {
            rest = tail;
            continue;
        };
        let name = &tail[..eq];
        if SENSITIVE.iter().any(|s| name.eq_ignore_ascii_case(s)) {
            let end = value_end(&tail[eq + 1..]);
            out.push_str(name);
            out.push_str("=***");
            rest = &tail[eq + 1 + end..];
        } else {
            // 非敏感参数：原样输出「名字=」，值里的下一个 `&` 继续由循环处理
            out.push_str(&tail[..=eq]);
            rest = &tail[eq + 1..];
        }
    }
    out
}

/// 抹掉 URL 的 userinfo：`https://user:pass@host/x` → `https://***@host/x`。
///
/// 私有订阅地址可能把 HTTP Basic 凭据直接写在 URL 里（前端也把这些 URL 打进 `[ui]` 行）。
fn mask_url_userinfo(text: &str) -> String {
    /// authority 段的结束位置（路径/查询/空白/展示符）。
    fn authority_end(rest: &str) -> usize {
        rest.find(|c: char| {
            matches!(c, '/' | '?' | '#' | ' ' | '"' | '\'' | ')' | '>' | ',' | ']' | '}')
                || c.is_control()
        })
        .unwrap_or(rest.len())
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(pos) = rest.find("://") else {
            out.push_str(rest);
            return out;
        };
        let after = &rest[pos + 3..];
        let end = authority_end(after);
        out.push_str(&rest[..pos + 3]);
        match after[..end].rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&after[at..end]);
            }
            None => out.push_str(&after[..end]),
        }
        rest = &after[end..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 落盘前打码：含凭据形态的输入（URL 查询串 / userinfo）抹成 `***`，正常日志行不受影响。
    #[test]
    fn scrub_log_line_masks_credentials_in_urls() {
        // 真实泄漏路径 1：AI 错误正文回显端点 URL（Gemini 把 key 放在查询串里）
        let ai_err = "ai summarize failed: error sending request for url \
                      (https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key=AIzaSy-SECRET-9f3c1a7b)";
        let scrubbed = scrub_log_line(ai_err);
        assert!(
            !scrubbed.contains("AIzaSy-SECRET-9f3c1a7b"),
            "key 不得留在日志里: {scrubbed}"
        );
        assert!(scrubbed.contains("key=***"), "应保留参数名便于定位: {scrubbed}");
        assert!(
            scrubbed.contains("generateContent"),
            "非敏感部分要保留: {scrubbed}"
        );

        // 真实泄漏路径 2：私有订阅地址把凭据放在查询串里（前端 [ui] 行会带整个 URL）
        let feed = "https://example.com/private.xml?token=9f3c1a7b2e5d4c6f&x=1";
        let scrubbed = scrub_log_line(feed);
        assert!(!scrubbed.contains("9f3c1a7b2e5d4c6f"));
        assert!(scrubbed.contains("token=***"));
        assert!(
            scrubbed.contains("x=1"),
            "同一条 URL 上的其它参数要保留: {scrubbed}"
        );

        // 真实泄漏路径 3：URL userinfo 里的 HTTP Basic 凭据
        let basic = "https://alice:hunter2@example.com/private.xml";
        let scrubbed = scrub_log_line(basic);
        assert!(!scrubbed.contains("hunter2"), "密码不得留在日志里: {scrubbed}");
        assert!(!scrubbed.contains("alice"), "用户名也不留: {scrubbed}");
        assert!(scrubbed.contains("***@example.com"), "主机名要保留: {scrubbed}");

        // 多种常见参数名
        for (raw, secret) in [
            ("https://x.test/a?api_key=SECRET-A", "SECRET-A"),
            ("https://x.test/a?api-key=SECRET-B", "SECRET-B"),
            ("https://x.test/a?access_token=SECRET-C&y=2", "SECRET-C"),
            ("https://x.test/a?password=SECRET-D", "SECRET-D"),
            ("https://x.test/a?foo=1&client_secret=SECRET-E", "SECRET-E"),
            ("https://x.test/a?KEY=SECRET-F", "SECRET-F"),
        ] {
            let out = scrub_log_line(raw);
            assert!(!out.contains(secret), "{raw} → {out}");
            assert!(out.contains("=***"), "{raw} → {out}");
        }

        // 正常日志行原样通过（打码不能把界面诊断行搞花）
        for line in [
            "view=all count=3 exhausted=true",
            "[ui] renderList rows=200 12ms",
            "ai summarize entry=7 from_cache=true chars=42 model=openai:gpt-4o-mini",
            "settings pane=ai base=https://api.openai.com/v1",
            "[rustrss] 已暂存恢复文件 /tmp/a.sqlite（下次启动替换 /tmp/b.sqlite）",
        ] {
            assert_eq!(scrub_log_line(line), line, "不该动: {line}");
        }

        // MCP 写审计的参数摘要：token 形态（含写 token 本身）一律不得整值出现
        let audit_args = r#"{"ids":[1,2],"url":"https://user:pw@example.com/feed.xml?token=WRITE-TOKEN-48"}"#;
        let scrubbed = scrub_log_line(audit_args);
        assert!(!scrubbed.contains("WRITE-TOKEN-48"), "{scrubbed}");
        assert!(!scrubbed.contains("pw"), "{scrubbed}");
        assert!(scrubbed.contains("ids"), "非敏感参数要保留: {scrubbed}");
    }
}
