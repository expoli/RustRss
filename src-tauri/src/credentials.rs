//! 凭据存储的平台分层（AI API key 的唯一落点）。
//!
//! 桌面：系统 keyring（Linux Secret Service / macOS Keychain / Windows 凭据管理器），
//! 实现从 ai.rs 原样搬入，行为不变。
//! 移动端（Android）：`SecureStorePlugin`（gen/android，app 模块）——secret 用
//! Android Keystore 的 AES/GCM 密钥加密后存进应用私有 SharedPreferences，
//! 明文只存在于调用与解密瞬间，不落 SQLite / 偏好文件 / 日志。
//!
//! 两边对 ai.rs 暴露同一组入口：`store_key` / `load_key` / `delete_key`；
//! 来源统一报 `KeySource::Keyring`（界面文案「来自系统凭据库」对两类后端都成立）。

use crate::ai::KeySource;

/// 环境变量后备通道：临时/CI 用，不进库也不进凭据库。
pub const ENV_KEY: &str = "RUSTSS_AI_KEY";

pub const KEYRING_SERVICE: &str = "rustrss";

/// 每个 provider 一个凭据条目：切换 provider 不会互相覆盖 key
pub fn key_account(provider: &str) -> String {
    format!("ai.api_key.{provider}")
}

/// 注册 Kotlin `SecureStorePlugin`（Android Keystore 加密存储），并把句柄
/// 存进本模块：ai.rs 的读写调用在任意线程发生，OnceLock 全局可见。
#[cfg(mobile)]
pub fn mobile_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("secure-store")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("tech.expoli.rustrss", "SecureStorePlugin")?;
            mobile::set_handle(handle);
            Ok(())
        })
        .build()
}

pub fn store_key(provider: &str, key: &str) -> Result<(), String> {
    #[cfg(desktop)]
    {
        desktop::store(provider, key)
    }
    #[cfg(mobile)]
    {
        mobile::store(provider, key)
    }
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
    #[cfg(desktop)]
    {
        desktop::load(provider)
    }
    #[cfg(mobile)]
    {
        mobile::load(provider)
    }
}

pub fn delete_key(provider: &str) -> Result<(), String> {
    #[cfg(desktop)]
    {
        desktop::delete(provider)
    }
    #[cfg(mobile)]
    {
        mobile::delete(provider)
    }
}

// ---------------- 桌面：系统 keyring ----------------

#[cfg(desktop)]
mod desktop {
    use super::{key_account, KEYRING_SERVICE};
    use keyring::Entry;

    pub(crate) fn keyring_error(err: keyring::Error) -> String {
        format!(
            "无法访问系统凭据库：{err}。\
             （Linux 需要 Secret Service，例如 KWallet 或 GNOME Keyring；\
             也可临时用环境变量 RUSTSS_AI_KEY 代替）"
        )
    }

    /// 一次调用偶发失败时的重试次数；重试间隔只为不让失败路径打转。
    pub(crate) const KEYRING_ATTEMPTS: usize = 3;
    pub(crate) const KEYRING_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

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
    pub(crate) fn keyring_retry<T>(
        mut op: impl FnMut() -> Result<T, keyring::Error>,
    ) -> Result<T, keyring::Error> {
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

    pub(crate) fn store(provider: &str, key: &str) -> Result<(), String> {
        let entry = Entry::new(KEYRING_SERVICE, &key_account(provider)).map_err(keyring_error)?;
        keyring_retry(|| entry.set_password(key)).map_err(|e| format!("写入凭据库失败：{e}"))
    }

    pub(crate) fn load(
        provider: &str,
    ) -> Result<(Option<String>, Option<crate::ai::KeySource>), String> {
        // Entry::new 在 secret-service 后端只是建个内存结构（不连 D-Bus），失败基本只可能是空 target；
        // 这里回错误原文：设置页那一路按 kind 本地化，中文说明不会漏进英文界面；
        // 给用户看的前缀由调用方补（见 config_snapshot）
        let entry = Entry::new(KEYRING_SERVICE, &key_account(provider)).map_err(|e| e.to_string())?;
        match keyring_retry(|| entry.get_password()) {
            Ok(k) if !k.trim().is_empty() => {
                Ok((Some(k), Some(crate::ai::KeySource::Keyring)))
            }
            Ok(_) => Ok((None, None)),
            Err(keyring::Error::NoEntry) => Ok((None, None)),
            // 只回平台错误原文：给用户看的前缀由调用方补（设置页那一路交给界面本地化）
            Err(e) => Err(e.to_string()),
        }
    }

    pub(crate) fn delete(provider: &str) -> Result<(), String> {
        let entry = Entry::new(KEYRING_SERVICE, &key_account(provider)).map_err(keyring_error)?;
        match keyring_retry(|| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("清除凭据失败：{e}")),
        }
    }
}

#[cfg(desktop)]
#[cfg(test)]
mod desktop_tests {
    use super::desktop::{keyring_retry, KEYRING_ATTEMPTS};

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
}

// ---------------- 移动端：Android Keystore（SecureStorePlugin） ----------------

#[cfg(mobile)]
mod mobile {
    use super::{key_account, KEYRING_SERVICE};
    use serde_json::json;
    use std::sync::OnceLock;
    use tauri::plugin::PluginHandle;

    /// setup 阶段注册 Kotlin `SecureStorePlugin` 后存入；ai.rs 的调用都发生在
    /// 设置页/AI 动作之后，理论上不会早于注册。未就绪时报显式错误而不是假装没存。
    static HANDLE: OnceLock<PluginHandle<tauri::Wry>> = OnceLock::new();

    pub(crate) fn set_handle(handle: PluginHandle<tauri::Wry>) {
        let _ = HANDLE.set(handle);
    }

    fn handle() -> Result<&'static PluginHandle<tauri::Wry>, String> {
        HANDLE.get().ok_or_else(|| "凭据存储尚未就绪".into())
    }

    pub(crate) fn store(provider: &str, key: &str) -> Result<(), String> {
        handle()?
            .run_mobile_plugin::<serde_json::Value>(
                "set",
                json!({
                    "service": KEYRING_SERVICE,
                    "account": key_account(provider),
                    "password": key,
                }),
            )
            .map(|_| ())
            .map_err(|e| format!("写入凭据库失败：{e}"))
    }

    #[derive(serde::Deserialize)]
    struct GetOut {
        #[serde(default)]
        password: String,
    }

    pub(crate) fn load(
        provider: &str,
    ) -> Result<(Option<String>, Option<crate::ai::KeySource>), String> {
        let raw: serde_json::Value = handle()?
            .run_mobile_plugin(
                "get",
                json!({
                    "service": KEYRING_SERVICE,
                    "account": key_account(provider),
                }),
            )
            .map_err(|e| e.to_string())?;
        let out: GetOut = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        // 只记长度，不记内容（凭据不进日志）
        log::info!("[rustrss] credentials get: len={}", out.password.len());
        // 空串 = 条目不存在（与桌面 NoEntry → (None, None) 同语义）
        if out.password.trim().is_empty() {
            Ok((None, None))
        } else {
            Ok((Some(out.password), Some(crate::ai::KeySource::Keyring)))
        }
    }

    pub(crate) fn delete(provider: &str) -> Result<(), String> {
        handle()?
            .run_mobile_plugin::<serde_json::Value>(
                "delete",
                json!({
                    "service": KEYRING_SERVICE,
                    "account": key_account(provider),
                }),
            )
            .map(|_| ())
            .map_err(|e| format!("清除凭据失败：{e}"))
    }
}
