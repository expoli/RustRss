//! 写调用审计：每次写调用一行 `target = "mcp"` 的 info 日志。
//!
//! 口径（tech_design「写操作契约」）：含**工具名 / 参数摘要 / 影响条数 / 结果**，
//! 参数摘要必须过 core 的 [`scrub_log_line`]（URL 里的 token / userinfo 一律 `***`），
//! **不含正文与凭据**。被授权闸门拒掉的写调用也记一行（`ok=false` + 错误码）——
//! 那正是最需要留痕的"有人在试"。
//!
//! 为什么单独成模块：日志行的形状是可验收的产物（AC 明确要求"含工具名/参数摘要/
//! 影响条数/结果"），把它做成纯函数才能被测试逐字段断言，而不是"读日志碰运气"。

use log::info;
use rmcp::model::JsonObject;
use serde_json::Value;

use crate::registry::GateError;
use rustrss_core::logging::scrub::scrub_log_line;

/// 参数摘要的最大字符数（写参数可能是长 URL 或一段 OPML 文本）
pub const MAX_ARGS_CHARS: usize = 300;

/// 工具调用的结果摘要（从工具返回的 JSON 信封里抽出来）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditSummary {
    pub ok: bool,
    pub affected: Option<i64>,
    pub error_code: Option<String>,
    pub dry_run: bool,
}

impl AuditSummary {
    /// 成功/预览（工具按写契约返回 `{ok, affected, results?, dry_run?}`）
    pub fn ok(affected: i64, dry_run: bool) -> Self {
        Self {
            ok: true,
            affected: Some(affected),
            error_code: None,
            dry_run,
        }
    }

    /// 被授权闸门拒绝：`affected` 无意义（没有执行），带上错误码
    pub fn rejected(err: GateError) -> Self {
        Self {
            ok: false,
            affected: None,
            error_code: Some(err.code().to_string()),
            dry_run: false,
        }
    }

    /// 工具自己返回了失败信封（含 error_code）或抛了协议错误
    pub fn failed(error_code: impl Into<String>) -> Self {
        Self {
            ok: false,
            affected: None,
            error_code: Some(error_code.into()),
            dry_run: false,
        }
    }
}

/// 参数摘要：JSON 紧凑串 → scrub → 截断。
///
/// **先 scrub 再截断**：反过来的话，一个超长参数会把 `token=***` 之前的
/// 凭据片段留在截断点上……更糟的是截断可能正好切掉 `token=` 的键名，
/// 让读者看不出"这里打码过"。
pub fn args_summary(args: Option<&JsonObject>) -> String {
    let Some(args) = args else {
        return "-".to_string();
    };
    let raw = serde_json::to_string(args).unwrap_or_else(|_| "<不可序列化>".to_string());
    let scrubbed = scrub_log_line(&raw);
    if scrubbed.chars().count() <= MAX_ARGS_CHARS {
        return scrubbed;
    }
    let cut: String = scrubbed.chars().take(MAX_ARGS_CHARS).collect();
    format!("{cut}…(截断)")
}

/// 从工具返回的文本体里抽出审计需要的字段。
///
/// 工具返回的不是合法 JSON（或不是信封形状）时不猜：`ok=false` + `error_code=unparsable_result`，
/// 这样审计里仍留下"这次调用结果无法解释"的痕迹。
pub fn summary_from_body(body: &str) -> AuditSummary {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return AuditSummary::failed("unparsable_result");
    };
    let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let affected = value.get("affected").and_then(Value::as_i64);
    let error_code = value
        .get("error_code")
        .and_then(Value::as_str)
        .map(str::to_string);
    let dry_run = value
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    AuditSummary {
        ok,
        affected,
        error_code,
        dry_run,
    }
}

/// Theme patches are user data. Audit field names and revisions, never raw values.
fn theme_args_summary(args: Option<&JsonObject>) -> String {
    let Some(args) = args else {
        return "-".into();
    };
    let mut safe = serde_json::Map::new();
    for key in ["expected_revision", "historical_revision"] {
        if let Some(value) = args.get(key).and_then(Value::as_u64) {
            safe.insert(key.into(), value.into());
        }
    }
    if let Some(patch) = args.get("patch").and_then(Value::as_object) {
        safe.insert(
            "patch_fields".into(),
            serde_json::json!(["mode", "light_preset", "dark_preset", "overrides"]
                .into_iter()
                .filter(|k| patch.contains_key(*k))
                .collect::<Vec<_>>()),
        );
    }
    args_summary(Some(&safe))
}

/// 审计行（纯函数：日志点的内容可被逐字段断言）
pub fn write_line(tool: &str, args: Option<&JsonObject>, summary: &AuditSummary) -> String {
    let affected = summary
        .affected
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".to_string());
    let error_code = summary.error_code.as_deref().unwrap_or("-");
    format!(
        "mcp-write tool={tool} args={} affected={affected} ok={} dry_run={} error_code={error_code}",
        if matches!(tool, "update_theme" | "restore_theme") { theme_args_summary(args) } else { args_summary(args) },
        summary.ok,
        summary.dry_run
    )
}

/// 落一行审计（`target = "mcp"`，与应用日志同一份文件）
pub fn emit(tool: &str, args: Option<&JsonObject>, summary: &AuditSummary) {
    info!(target: "mcp", "{}", write_line(tool, args, summary));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args_of(v: Value) -> JsonObject {
        v.as_object().cloned().expect("测试参数应是对象")
    }

    /// 审计行包含工具名 / 参数摘要 / 影响条数 / 结果四要素
    #[test]
    fn line_carries_all_four_required_fields() {
        let args = args_of(json!({"ids": [1, 2, 3], "read": true}));
        let line = write_line("set_read", Some(&args), &AuditSummary::ok(3, false));
        assert!(line.contains("tool=set_read"), "{line}");
        assert!(
            line.contains(r#"args={"ids":[1,2,3],"read":true}"#),
            "{line}"
        );
        assert!(line.contains("affected=3"), "{line}");
        assert!(line.contains("ok=true"), "{line}");
        assert!(line.contains("error_code=-"), "{line}");
    }

    /// 参数里的凭据形态被打码（私有订阅地址 / 带 token 的 URL / 写 token 本身）
    #[test]
    fn args_summary_scrubs_credentials() {
        let args = args_of(json!({
            "url": "https://alice:hunter2@example.com/private.xml?token=WRITE-TOKEN-48-HEX",
            "ids": [7]
        }));
        let line = write_line("subscribe", Some(&args), &AuditSummary::ok(1, false));
        assert!(!line.contains("hunter2"), "密码不得进审计行: {line}");
        assert!(
            !line.contains("WRITE-TOKEN-48-HEX"),
            "token 不得进审计行: {line}"
        );
        assert!(line.contains("token=***"), "保留参数名便于定位: {line}");
        assert!(line.contains("ids"), "非敏感参数保留: {line}");
    }

    /// 超长参数被截断（OPML 全文之类的入参不能把日志行撑爆）
    #[test]
    fn long_args_are_truncated_after_scrubbing() {
        let long = "x".repeat(2000);
        let args = args_of(json!({"content": long}));
        let line = write_line("import_opml", Some(&args), &AuditSummary::ok(5, true));
        assert!(line.contains("…(截断)"), "{line}");
        assert!(line.chars().count() < MAX_ARGS_CHARS + 120, "{line}");
    }

    /// 拒掉的写调用也留痕：ok=false + 错误码，affected 用 `-` 而不是 0
    #[test]
    fn rejected_calls_are_audited_with_the_error_code() {
        let args = args_of(json!({"ids": [1]}));
        let line = write_line(
            "set_read",
            Some(&args),
            &AuditSummary::rejected(GateError::WriteScopeRequired),
        );
        assert!(line.contains("ok=false"), "{line}");
        assert!(line.contains("error_code=write_scope_required"), "{line}");
        assert!(line.contains("affected=-"), "{line}");
    }

    /// 结果摘要解析：合规信封 / 预览（dry_run）/ 异常返回
    #[test]
    fn summary_is_parsed_from_the_write_envelope() {
        let s = summary_from_body(r#"{"ok":true,"affected":2,"results":[],"dry_run":true}"#);
        assert_eq!(
            s,
            AuditSummary {
                ok: true,
                affected: Some(2),
                error_code: None,
                dry_run: true
            }
        );

        let s = summary_from_body(
            r#"{"ok":false,"affected":0,"results":[],"error_code":"invalid_argument"}"#,
        );
        assert!(!s.ok);
        assert_eq!(s.error_code.as_deref(), Some("invalid_argument"));

        // 非 JSON（旧工具返回纯文本）不装作成功
        let s = summary_from_body("列表里没有这条");
        assert!(!s.ok);
        assert_eq!(s.error_code.as_deref(), Some("unparsable_result"));
    }
}
