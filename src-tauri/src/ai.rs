//! AI 设置与凭据。
//!
//! 分工刻意的：**provider / model / base_url 等非敏感设置存数据库**（跟着订阅一起备份），
//! **API key 存操作系统凭据库**（Linux 走 Secret Service，macOS 走 Keychain，Windows 走凭据管理器；
//! Android 走 Keystore 加密的私有存储，实现在 credentials.rs）。
//!
//! 为什么 key 不进数据库：数据库会被导出、同步、复制给别人，key 一旦进去就会跟着跑。
//! 凭据库不可用时返回明确错误，不静默降级成明文文件。

use rustrss_core::ai::{AiClient, AiConfig, Provider};
use rustrss_core::Store;
use serde::Serialize;
// 凭据存取的平台实现已下沉到 credentials.rs（桌面 keyring / Android Keystore）；
// 对外入口保持原路径（commands.rs 与测试都走 crate::ai::*）。
pub use crate::credentials::{delete_key, load_key, store_key};
#[cfg(test)]
use crate::credentials::ENV_KEY;

pub const K_PROVIDER: &str = "ai.provider";
pub const K_MODEL: &str = "ai.model";
pub const K_BASE_URL: &str = "ai.base_url";
pub const K_TRANSLATE_TARGET: &str = "ai.translate_target";
pub const K_CONFIRM_BEFORE_SEND: &str = "ai.confirm_before_send";
pub const K_MAX_OUTPUT_TOKENS: &str = "ai.max_output_tokens";
pub const K_DIGEST_CONCURRENCY: &str = "ai.digest_concurrency";
pub const K_REASONING_EFFORT: &str = "ai.reasoning_effort";

pub const DEFAULT_PROVIDER: &str = "ollama";
pub const DEFAULT_TRANSLATE_TARGET: &str = "中文";
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 4096;
/// 上限范围：太小会在长文翻译上截断（推理模型思考链更耗），太大在小上下文模型上直接 400。
pub const MAX_OUTPUT_TOKENS_LIMITS: (u32, u32) = (256, 32_768);
/// 思考强度白名单：""=跟随模型默认（不发参数）+ OpenAI reasoning_effort 的四档。
pub const REASONING_EFFORT_CHOICES: [&str; 5] = ["", "minimal", "low", "medium", "high"];

/// 读思考强度：白名单外的值一律归 None（跟随默认）。None 表示请求里不带这个参数。
pub fn reasoning_effort_from_store(store: &Store) -> Option<String> {
    let v = non_empty_setting(store, K_REASONING_EFFORT).unwrap_or_default();
    if REASONING_EFFORT_CHOICES.contains(&v.as_str()) {
        Some(v).filter(|s| !s.is_empty())
    } else {
        None
    }
}

/// 读输出上限（缺失/非法回退默认；clamp 到上下限内）。
pub fn max_output_tokens_from_store(store: &Store) -> u32 {
    non_empty_setting(store, K_MAX_OUTPUT_TOKENS)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
        .clamp(MAX_OUTPUT_TOKENS_LIMITS.0, MAX_OUTPUT_TOKENS_LIMITS.1)
}

/// Extraction ceiling only; unset/invalid uses cloud=4, local Ollama=1.
pub fn digest_concurrency_from_store(store: &Store, provider: Provider) -> usize {
    non_empty_setting(store, K_DIGEST_CONCURRENCY)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(if provider == Provider::Ollama { 1 } else { 4 })
        .clamp(1, 8)
}

pub fn provider_from_str(value: &str) -> Provider {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai" | "openai_compatible" | "openai-compatible" => Provider::OpenAiCompatible,
        "anthropic" => Provider::Anthropic,
        "gemini" => Provider::Gemini,
        _ => Provider::Ollama,
    }
}

pub fn provider_to_str(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAiCompatible => "openai",
        Provider::Anthropic => "anthropic",
        Provider::Gemini => "gemini",
        Provider::Ollama => "ollama",
    }
}

/// 各 provider 的默认端点（界面上留空即用这些）
pub fn default_base_url(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAiCompatible => "https://api.openai.com/v1",
        Provider::Anthropic => "https://api.anthropic.com",
        Provider::Gemini => "https://generativelanguage.googleapis.com",
        Provider::Ollama => "http://127.0.0.1:11434",
    }
}

/// key 的来源，或读不到的原因。
///
/// 刻意是结构化数据而不是拼好的中文句子：中文说明由界面按当前语言渲染，
/// en 界面里不会冒出中文括号/中文备注，机器侧也能直接判定来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeySource {
    /// 来自系统凭据库
    Keyring,
    /// 来自环境变量（名字由后端给出，如 RUSTSS_AI_KEY）
    Env { name: String },
    /// 读不到：凭据库不可用或读取失败；`error` 是平台错误原文
    Unavailable { error: String },
}

pub fn non_empty_setting(store: &Store, key: &str) -> Option<String> {
    store
        .setting(key)
        .ok()
        .flatten()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 组装 AiConfig：设置来自数据库，key 来自凭据库（或环境变量）
fn config_snapshot(store: &Store) -> Result<AiConfig, String> {
    let provider =
        provider_from_str(&non_empty_setting(store, K_PROVIDER).unwrap_or(DEFAULT_PROVIDER.into()));
    let model = non_empty_setting(store, K_MODEL);
    let base_url = non_empty_setting(store, K_BASE_URL);

    let model = model.ok_or_else(|| "还没填模型名（设置 → AI）".to_string())?;
    let base = base_url.unwrap_or_else(|| default_base_url(provider).to_string());

    let config = match provider {
        Provider::OpenAiCompatible => {
            AiConfig::openai_compatible(&base, &model, String::new())
        }
        Provider::Anthropic => {
            AiConfig::anthropic(&model, String::new()).with_base_url(&base)
        }
        Provider::Gemini => AiConfig::gemini(&model, String::new()).with_base_url(&base),
        Provider::Ollama => AiConfig::ollama(&model).with_base_url(&base),
    }
    .with_max_output_tokens(max_output_tokens_from_store(store))
    .with_reasoning_effort(reasoning_effort_from_store(store));
    Ok(config)
}

pub fn client_from_state(state: &crate::state::AppState) -> Result<AiClient, String> {
    client_with_key_loader(state, load_key)
}

fn client_with_key_loader(
    state: &crate::state::AppState,
    load: impl FnOnce(&str) -> Result<(Option<String>, Option<KeySource>), String>,
) -> Result<AiClient, String> {
    let mut config = state.with_store(config_snapshot)?;
    if config.provider != Provider::Ollama {
        config.api_key = load(provider_to_str(config.provider))
            .map_err(|e| format!("读取凭据库失败：{e}"))?.0;
    }
    let proxy = state.with_store(|s| rustrss_core::network::ProxyConfig::load(s).map_err(|e| e.to_string()))?;
    AiClient::with_proxy(config, &proxy).map_err(|e| e.to_string())
}

pub fn translate_target(store: &Store) -> String {
    non_empty_setting(store, K_TRANSLATE_TARGET).unwrap_or_else(|| DEFAULT_TRANSLATE_TARGET.into())
}

/// 「发送前确认要发什么」：默认开启。
/// 投喂给 AI 的是正文，默认让用户先看一眼比默认静默外发更合适。
pub fn confirm_before_send(store: &Store) -> bool {
    match non_empty_setting(store, K_CONFIRM_BEFORE_SEND) {
        Some(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0" | "off" | "no"),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_client_creation_does_not_hold_store_lock() {
        let state = crate::state::AppState::for_test();
        state.with_store(|s| {
            s.set_setting(K_PROVIDER, "openai").unwrap();
            s.set_setting(K_MODEL, "test-model").unwrap();
            Ok(())
        }).unwrap();
        let mut called = false;
        client_with_key_loader(&state, |_| {
            called = true;
            assert!(state.store_is_available());
            Ok((Some("test-key".into()), Some(KeySource::Keyring)))
        }).unwrap();
        assert!(called);
    }


    #[test]
    fn confirm_before_send_defaults_to_on_and_only_accepts_explicit_off() {
        let store = Store::open_in_memory().unwrap();
        // 没设置过 → 问（把正文发出去前让用户看一眼是安全侧默认）
        assert!(confirm_before_send(&store));

        for value in ["false", "FALSE", " false ", "0", "off", "no"] {
            store.set_setting(K_CONFIRM_BEFORE_SEND, value).unwrap();
            assert!(!confirm_before_send(&store), "{value:?} 应当解析为关闭");
        }

        for value in ["true", "on", "", "   ", "随便写的"] {
            store.set_setting(K_CONFIRM_BEFORE_SEND, value).unwrap();
            // 空值等同未设置；无法识别的值不该静默变成「不问了」
            assert!(confirm_before_send(&store), "{value:?} 应当回到默认开启");
        }
    }

    /// 前后端契约：ui/app.js 的 keySourceText() 按 kind 分支取值，
    /// 字段名/取值变了要同步改前端（这里钉住形状）
    #[test]
    fn key_source_serializes_to_the_shape_the_ui_switches_on() {
        let cases = [
            (KeySource::Keyring, r#"{"kind":"keyring"}"#),
            (
                KeySource::Env {
                    name: ENV_KEY.into(),
                },
                r#"{"kind":"env","name":"RUSTSS_AI_KEY"}"#,
            ),
            (
                KeySource::Unavailable {
                    error: "boom".into(),
                },
                r#"{"kind":"unavailable","error":"boom"}"#,
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(serde_json::to_string(&source).unwrap(), expected);
        }
    }
}
