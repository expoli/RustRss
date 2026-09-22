//! AI 设置与凭据。
//!
//! 分工刻意的：**provider / model / base_url 等非敏感设置存数据库**（跟着订阅一起备份），
//! **API key 存操作系统凭据库**（Linux 走 Secret Service，macOS 走 Keychain，Windows 走凭据管理器）。
//!
//! 为什么 key 不进数据库：数据库会被导出、同步、复制给别人，key 一旦进去就会跟着跑。
//! 凭据库不可用时返回明确错误，不静默降级成明文文件。

use keyring::Entry;
use rustrss_core::ai::{AiClient, AiConfig, Provider};
use rustrss_core::Store;

pub const KEYRING_SERVICE: &str = "rustrss";
pub const K_PROVIDER: &str = "ai.provider";
pub const K_MODEL: &str = "ai.model";
pub const K_BASE_URL: &str = "ai.base_url";
pub const K_TRANSLATE_TARGET: &str = "ai.translate_target";
pub const K_CONFIRM_BEFORE_SEND: &str = "ai.confirm_before_send";
pub const K_MAX_OUTPUT_TOKENS: &str = "ai.max_output_tokens";
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

/// 每个 provider 一个凭据条目：切换 provider 不会互相覆盖 key
pub fn key_account(provider: &str) -> String {
    format!("ai.api_key.{provider}")
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

fn keyring_error(err: keyring::Error) -> String {
    format!(
        "无法访问系统凭据库：{err}。\
         （Linux 需要 Secret Service，例如 KWallet 或 GNOME Keyring；\
         也可临时用环境变量 RUSTSS_AI_KEY 代替）"
    )
}

pub fn store_key(provider: &str, key: &str) -> Result<(), String> {
    Entry::new(KEYRING_SERVICE, &key_account(provider))
        .map_err(keyring_error)?
        .set_password(key)
        .map_err(|e| format!("写入凭据库失败：{e}"))
}

/// 读 key：先看环境变量（便于临时/CI 使用），再查凭据库。
/// 返回 `(key, 说明)`，说明用于在设置界面如实告知来源。
pub fn load_key(provider: &str) -> Result<(Option<String>, Option<String>), String> {
    if let Ok(from_env) = std::env::var("RUSTSS_AI_KEY") {
        if !from_env.trim().is_empty() {
            return Ok((Some(from_env), Some("来自环境变量 RUSTSS_AI_KEY".into())));
        }
    }
    let entry = Entry::new(KEYRING_SERVICE, &key_account(provider)).map_err(keyring_error)?;
    match entry.get_password() {
        Ok(k) if !k.trim().is_empty() => Ok((Some(k), Some("来自系统凭据库".into()))),
        Ok(_) => Ok((None, None)),
        Err(keyring::Error::NoEntry) => Ok((None, None)),
        Err(e) => Err(format!("读取凭据库失败：{e}")),
    }
}

pub fn delete_key(provider: &str) -> Result<(), String> {
    let entry = Entry::new(KEYRING_SERVICE, &key_account(provider)).map_err(keyring_error)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("清除凭据失败：{e}")),
    }
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
pub fn config_from_store(store: &Store) -> Result<(AiConfig, Option<String>), String> {
    let provider =
        provider_from_str(&non_empty_setting(store, K_PROVIDER).unwrap_or(DEFAULT_PROVIDER.into()));
    let model = non_empty_setting(store, K_MODEL);
    let base_url = non_empty_setting(store, K_BASE_URL);
    let (key, key_source) = load_key(provider_to_str(provider))?;

    let model = model.ok_or_else(|| "还没填模型名（设置 → AI）".to_string())?;
    let base = base_url.unwrap_or_else(|| default_base_url(provider).to_string());

    let config = match provider {
        Provider::OpenAiCompatible => {
            AiConfig::openai_compatible(&base, &model, key.clone().unwrap_or_default())
        }
        Provider::Anthropic => {
            AiConfig::anthropic(&model, key.clone().unwrap_or_default()).with_base_url(&base)
        }
        Provider::Gemini => AiConfig::gemini(&model, key.clone().unwrap_or_default()).with_base_url(&base),
        Provider::Ollama => AiConfig::ollama(&model).with_base_url(&base),
    }
    .with_max_output_tokens(max_output_tokens_from_store(store))
    .with_reasoning_effort(reasoning_effort_from_store(store));
    Ok((config, key_source))
}

pub fn client_from_store(store: &Store) -> Result<AiClient, String> {
    let (config, _) = config_from_store(store)?;
    AiClient::new(config).map_err(|e| e.to_string())
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
}
