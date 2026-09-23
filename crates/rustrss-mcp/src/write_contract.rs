//! 写操作契约 helper：批量上限、`confirm`、`dry_run` 与统一返回信封。
//!
//! T3/T4 的写工具全部经过这里，避免每个工具各自解释一遍契约（口径漂移的常见来源）。
//! 契约来自 tech_design「写操作契约」：
//! - 批量 ids **≤ 100**，超限 → `invalid_argument`（agent 一次传 1000 条时要有明确答复）；
//! - 危险操作 `confirm: true` 必填，缺失/为 false → `confirm_required`；
//! - `dry_run: true` 只算影响面、**不落库**——由 [`apply_or_preview`] 在结构上保证：
//!   `dry_run` 时"落库"闭包根本不会被调用（不是靠工具作者自觉）；
//! - 返回信封固定 `{ ok, affected, results[], error_code? }`（`results` 给部分失败
//!   的可解释性；只统计影响条数不足以说明"哪几条没成"）。

use serde::{Deserialize, Serialize};

/// 批量操作的硬上限（PRD：超过就报错，不做静默截断——截断会让 agent 以为全做了）
pub const MAX_BATCH_IDS: usize = 100;

pub const ERROR_INVALID_ARGUMENT: &str = "invalid_argument";
pub const ERROR_CONFIRM_REQUIRED: &str = "confirm_required";

/// 写参数的契约错误（工具级错误，随信封一起返回）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractError {
    /// 批量 ids 超过 [`MAX_BATCH_IDS`]
    TooManyIds { count: usize },
    /// 批量 ids 为空：写操作至少要有目标，空列表多半是调用方拼错了
    NoTargets,
    /// 危险操作没给 `confirm: true`
    ConfirmRequired,
}

impl ContractError {
    pub fn code(self) -> &'static str {
        match self {
            ContractError::TooManyIds { .. } | ContractError::NoTargets => ERROR_INVALID_ARGUMENT,
            ContractError::ConfirmRequired => ERROR_CONFIRM_REQUIRED,
        }
    }

    pub fn message(self) -> String {
        match self {
            ContractError::TooManyIds { count } => format!(
                "一次最多处理 {MAX_BATCH_IDS} 条，收到 {count} 条：请分批调用（不要指望静默截断）"
            ),
            ContractError::NoTargets => {
                "ids 不能为空：没有目标就没有可写的东西（若想按条件批量，请用 feed_id + since/until）"
                    .to_string()
            }
            ContractError::ConfirmRequired => {
                "危险操作需要显式确认：请传 confirm: true（可先用 dry_run: true 预览影响面）"
                    .to_string()
            }
        }
    }
}

/// 批量 ids 校验：超限 → `invalid_argument`
pub fn check_batch_ids(ids: &[i64]) -> Result<(), ContractError> {
    if ids.is_empty() {
        return Err(ContractError::NoTargets);
    }
    if ids.len() > MAX_BATCH_IDS {
        return Err(ContractError::TooManyIds { count: ids.len() });
    }
    Ok(())
}

/// 危险操作的确认校验：`Some(true)` 之外一律拒（缺省不是"默认同意"）
pub fn require_confirm(confirm: Option<bool>) -> Result<(), ContractError> {
    match confirm {
        Some(true) => Ok(()),
        _ => Err(ContractError::ConfirmRequired),
    }
}

/// `dry_run: true` 时**只**运行预览分支。
///
/// 预览与实际执行应当共用同一份"算影响面"的代码（tech_design：验收要求 dry_run
/// 与实际影响面一致），所以两个闭包由调用方各自实现；这个函数只负责一件事：
/// dry_run 时绝不触碰第二个闭包。
pub fn apply_or_preview<T>(
    dry_run: Option<bool>,
    preview: impl FnOnce() -> T,
    apply: impl FnOnce() -> T,
) -> T {
    if dry_run.unwrap_or(false) {
        preview()
    } else {
        apply()
    }
}

/// 单个目标的执行结果（部分失败时告诉 agent 是哪几条）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemResult {
    pub id: i64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl ItemResult {
    pub fn ok(id: i64) -> Self {
        Self {
            id,
            ok: true,
            error_code: None,
        }
    }

    pub fn failed(id: i64, error_code: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            error_code: Some(error_code.into()),
        }
    }
}

/// 写操作的统一返回信封：`{ ok, affected, results[], error_code?, dry_run }`
///
/// `error` 是可选的人读说明（机器读 `error_code`）：agent 拿到 `invalid_argument`
/// 时需要知道"怎么改才对"，而不是只能猜。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WriteOutcome {
    pub ok: bool,
    pub affected: i64,
    pub results: Vec<ItemResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `true` = 本次只是预览、库没有变化（agent 不该把它当成"已经改了"）
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
    /// 工具特有的明细（刷新摘要 / 全文抓取信息…）：信封的四要素形状不变，
    /// agent 不必为每个工具记一套解包规则（用 [`WriteOutcome::with_detail`] 挂上）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl WriteOutcome {
    /// 成功：`affected` + 逐项结果
    pub fn done(affected: i64, results: Vec<ItemResult>, dry_run: bool) -> Self {
        Self {
            ok: true,
            affected,
            results,
            error_code: None,
            error: None,
            dry_run,
            detail: None,
        }
    }

    /// 预览：`affected` 是"若真做会影响到多少条"
    pub fn preview(affected: i64, results: Vec<ItemResult>) -> Self {
        Self::done(affected, results, true)
    }

    /// 失败：带机器可读错误码（契约错误或业务错误）
    pub fn failed(error_code: impl Into<String>, affected: i64) -> Self {
        Self {
            ok: false,
            affected,
            results: Vec::new(),
            error_code: Some(error_code.into()),
            error: None,
            dry_run: false,
            detail: None,
        }
    }

    /// 失败 + 人读说明（"怎么改才对"），机器码不变
    pub fn failed_with(error_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: Some(message.into()),
            ..Self::failed(error_code, 0)
        }
    }

    /// 工具特有明细（可选）：挂在信封的 `detail` 下，不改变四要素的形状
    #[must_use]
    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    /// 契约错误的捷径（`invalid_argument` / `confirm_required`）：连带人读说明
    pub fn rejected(err: ContractError) -> Self {
        Self {
            error: Some(err.message()),
            ..Self::failed(err.code(), 0)
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|e| {
            // 本结构全是基础类型，序列化失败意味着模型被改坏了：如实报错而不是空串
            format!(
                "{{\"ok\":false,\"affected\":0,\"results\":[],\"error_code\":\"internal_error\",\"error\":\"序列化写结果失败: {e}\"}}"
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 批量上限：100 通过、101 报 invalid_argument、空列表报错（不静默当成"什么都没做"）
    #[test]
    fn batch_ids_are_capped_at_100() {
        let ids = |n: usize| (1..=n as i64).collect::<Vec<_>>();
        assert!(check_batch_ids(&ids(1)).is_ok());
        assert!(check_batch_ids(&ids(MAX_BATCH_IDS)).is_ok());

        let err = check_batch_ids(&ids(MAX_BATCH_IDS + 1)).unwrap_err();
        assert_eq!(err.code(), "invalid_argument");
        assert!(err.message().contains("100"), "{}", err.message());

        let err = check_batch_ids(&[]).unwrap_err();
        assert_eq!(err.code(), "invalid_argument");
    }

    /// confirm 必填：缺省 / false / 非 true 一律 confirm_required，只有 true 放行
    #[test]
    fn dangerous_writes_require_explicit_confirm() {
        assert!(require_confirm(Some(true)).is_ok());
        for missing in [None, Some(false)] {
            let err = require_confirm(missing).unwrap_err();
            assert_eq!(err.code(), "confirm_required");
            assert!(err.message().contains("confirm"), "{}", err.message());
        }
    }

    /// dry_run 在结构上不落库：预览闭包被调用、落库闭包根本不被调用
    #[test]
    fn dry_run_never_runs_the_apply_branch() {
        let mut applied = 0;
        let previewed = apply_or_preview(
            Some(true),
            || "预览：将影响 3 条",
            || {
                applied += 1;
                "已删除"
            },
        );
        assert_eq!(previewed, "预览：将影响 3 条");
        assert_eq!(applied, 0, "dry_run=true 时落库分支不得执行");

        // 缺省 dry_run 视为 false（真的落库），否则"忘了传参数"会变成静默空操作
        let done = apply_or_preview(None, || "预览", || {
            applied += 1;
            "已删除"
        });
        assert_eq!(done, "已删除");
        assert_eq!(applied, 1);

        // 显式 false 也走落库分支
        let done = apply_or_preview(Some(false), || "预览", || "已删除");
        assert_eq!(done, "已删除");
    }

    /// 返回信封：字段名与可选字段的省略口径（`dry_run=false` 不出现，保持旧调用方兼容）
    #[test]
    fn outcome_envelope_shape() {
        let done = WriteOutcome::done(2, vec![ItemResult::ok(1), ItemResult::failed(2, "article_not_found")], false);
        let v: serde_json::Value = serde_json::from_str(&done.to_json()).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["affected"], 2);
        assert_eq!(v["results"][0]["id"], 1);
        assert_eq!(v["results"][1]["ok"], false);
        assert_eq!(v["results"][1]["error_code"], "article_not_found");
        assert!(v.get("error_code").is_none(), "成功时不带 error_code");
        assert!(v.get("dry_run").is_none(), "非 dry_run 不提 dry_run");
        assert!(v.get("detail").is_none(), "没挂明细就不出现 detail");

        let preview = WriteOutcome::preview(3, Vec::new());
        let v: serde_json::Value = serde_json::from_str(&preview.to_json()).unwrap();
        assert_eq!(v["dry_run"], true);
        assert_eq!(v["affected"], 3, "预览也给影响面，agent 靠它决定要不要真做");
        assert_eq!(v["ok"], true);

        let rejected = WriteOutcome::rejected(ContractError::ConfirmRequired);
        let v: serde_json::Value = serde_json::from_str(&rejected.to_json()).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error_code"], "confirm_required");
        assert_eq!(v["affected"], 0);
        assert!(
            v["error"].as_str().unwrap_or_default().contains("confirm"),
            "人读说明要告诉 agent 怎么改: {v}"
        );

        // 工具特有明细：挂上后出现在 detail，不影响四要素
        let with_detail = WriteOutcome::done(1, vec![ItemResult::ok(9)], false)
            .with_detail(serde_json::json!({"scope": "all", "inserted": 3}));
        let v: serde_json::Value = serde_json::from_str(&with_detail.to_json()).unwrap();
        assert_eq!(v["detail"]["scope"], "all");
        assert_eq!(v["detail"]["inserted"], 3);
        assert_eq!(v["affected"], 1, "detail 不改变 affected 的口径");

        // 业务错误 + 人读说明（refresh 的 rate_limited / fulltext 的失败路径用它）
        let business = WriteOutcome::failed_with("rate_limited", "刷新已在进行中，稍后重试");
        let v: serde_json::Value = serde_json::from_str(&business.to_json()).unwrap();
        assert_eq!(v["error_code"], "rate_limited");
        assert!(v["error"].as_str().unwrap().contains("稍后重试"), "{v}");
    }
}
