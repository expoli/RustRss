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
use serde::Serialize;

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

/// 一次调用偶发失败时的重试次数；重试间隔只为不让失败路径打转。
const KEYRING_ATTEMPTS: usize = 3;
const KEYRING_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

/// 凭据库读写统一包一层重试：Secret Service 的 DH 会话加密会**偶发**失配。
///
/// 根因在守护进程侧：ksecretd（kwallet6）≤ 6.24.0 在 DH 共享密钥高位为零时
/// 不按 1024 位补零再 HKDF（KDE #514194，上游 kwallet 6.25.0 才修），于是它和
/// 客户端导出的会话密钥不同，客户端解不开返回的密文，报
/// `Crypto error: Unpad Error`。失配只发生在**那一个会话**里，而 keyring 的每次
/// 操作都会重新建会话，所以重试等于换会话，能绕过去。
/// 本机实测：400 次读里 4 次失配（1.00%），且每次失配的紧跟一次（新会话）都成功
/// —— 所以重试 3 次的残存失败概率约 1e-6 量级。
/// 「没有条目」是确定状态，重试不会改变结果，直接返回。
fn keyring_retry<T>(mut op: impl FnMut() -> Result<T, keyring::Error>) -> Result<T, keyring::Error> {
    let mut last_err = None;
    for attempt in 1..=KEYRING_ATTEMPTS {
        match op() {
            Ok(value) => return Ok(value),
            Err(e @ keyring::Error::NoEntry) => return Err(e),
            Err(e) => {
                if attempt < KEYRING_ATTEMPTS {
                    log::warn!("[rustrss] 凭据库操作失败（第 {attempt} 次），换会话重试: {e}");
                    std::thread::sleep(KEYRING_RETRY_DELAY);
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.expect("循环里至少记录一次错误"))
}

pub fn store_key(provider: &str, key: &str) -> Result<(), String> {
    let entry = Entry::new(KEYRING_SERVICE, &key_account(provider)).map_err(keyring_error)?;
    keyring_retry(|| entry.set_password(key)).map_err(|e| format!("写入凭据库失败：{e}"))
}

/// 环境变量后备通道：临时/CI 用，不进库也不进凭据库。
pub const ENV_KEY: &str = "RUSTSS_AI_KEY";

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

/// 读 key：先看环境变量（便于临时/CI 使用），再查凭据库。
/// 返回 `(key, 来源)`；来源交给界面按当前语言渲染。
pub fn load_key(provider: &str) -> Result<(Option<String>, Option<KeySource>), String> {
    if let Ok(from_env) = std::env::var(ENV_KEY) {
        if !from_env.trim().is_empty() {
            return Ok((
                Some(from_env),
                Some(KeySource::Env {
                    name: ENV_KEY.into(),
                }),
            ));
        }
    }
    // Entry::new 在 secret-service 后端只是建个内存结构（不连 D-Bus），失败基本只可能是空 target；
    // 这里回错误原文：设置页那一路按 kind 本地化，中文说明不会漏进英文界面；
    // 给用户看的前缀由调用方补（见 config_from_store）
    let entry = Entry::new(KEYRING_SERVICE, &key_account(provider)).map_err(|e| e.to_string())?;
    match keyring_retry(|| entry.get_password()) {
        Ok(k) if !k.trim().is_empty() => Ok((Some(k), Some(KeySource::Keyring))),
        Ok(_) => Ok((None, None)),
        Err(keyring::Error::NoEntry) => Ok((None, None)),
        // 只回平台错误原文：给用户看的前缀由调用方补（设置页那一路交给界面本地化）
        Err(e) => Err(e.to_string()),
    }
}

pub fn delete_key(provider: &str) -> Result<(), String> {
    let entry = Entry::new(KEYRING_SERVICE, &key_account(provider)).map_err(keyring_error)?;
    match keyring_retry(|| entry.delete_credential()) {
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
pub fn config_from_store(store: &Store) -> Result<(AiConfig, Option<KeySource>), String> {
    let provider =
        provider_from_str(&non_empty_setting(store, K_PROVIDER).unwrap_or(DEFAULT_PROVIDER.into()));
    let model = non_empty_setting(store, K_MODEL);
    let base_url = non_empty_setting(store, K_BASE_URL);
    let (key, key_source) =
        load_key(provider_to_str(provider)).map_err(|e| format!("读取凭据库失败：{e}"))?;

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

    /// ksecretd 失配时客户端看到的就是这个错误（Crypto error: Unpad Error）
    fn transient_session_error() -> keyring::Error {
        keyring::Error::PlatformFailure(Box::new(std::io::Error::other(
            "Crypto error: Unpad Error",
        )))
    }

    #[test]
    fn keyring_retry_absorbs_transient_failures() {
        let mut calls = 0;
        let got = keyring_retry(|| {
            calls += 1;
            if calls < 2 {
                Err(transient_session_error())
            } else {
                Ok("credential")
            }
        });
        assert_eq!(got.unwrap(), "credential");
        assert_eq!(calls, 2, "第一次失败后应当换会话重试");
    }

    #[test]
    fn keyring_retry_gives_up_after_attempts_and_keeps_last_error() {
        let mut calls = 0;
        let err = keyring_retry::<()>(|| {
            calls += 1;
            Err(transient_session_error())
        })
        .unwrap_err();
        assert_eq!(calls, KEYRING_ATTEMPTS);
        // 错误不能被吞掉：调用方还要把它翻成给用户看的原因
        assert!(err.to_string().contains("Unpad Error"), "实际: {err}");
    }

    #[test]
    fn keyring_retry_does_not_retry_missing_entry() {
        let mut calls = 0;
        let err = keyring_retry::<()>(|| {
            calls += 1;
            Err(keyring::Error::NoEntry)
        })
        .unwrap_err();
        assert_eq!(calls, 1, "条目不存在是确定状态，重试没有意义");
        assert!(matches!(err, keyring::Error::NoEntry));
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
