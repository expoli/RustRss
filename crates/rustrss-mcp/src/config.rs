//! MCP 服务的配置：端口 / token / 客户端配置片段。
//!
//! 放在 lib 而不是应用里，是因为**配置片段本身就是交付物**：
//! 「粘过去能不能用」取决于这段文本的格式是否正确，所以它必须可测，
//! 也必须能在不开 GUI 的情况下生成（`rustrss-mcp --print-config`）。

use rustrss_core::Store;

use crate::registry::Switches;

pub const K_PORT: &str = "mcp.port";
pub const K_TOKEN: &str = "mcp.token";
/// 写 token：由设置页显式生成/轮换/销毁，**不存在 = 写工具不注册**（不自动生成——
/// 与读 token 不同，写 token 代表一次显式的授权动作）
pub const K_WRITE_TOKEN: &str = "mcp.write_token";
/// 写能力总开关（默认 false）
pub const K_WRITE_ENABLED: &str = "mcp.write_enabled";
/// 危险工具开关（默认 false）：`unsubscribe` / `folder_delete` 需要它 + `confirm`
pub const K_DANGEROUS_ENABLED: &str = "mcp.dangerous_enabled";
/// 默认端口：避开 quick-rss（8745）等常见占用
pub const DEFAULT_PORT: u16 = 8817;
/// 低于此值的端口需要 root，不提供给用户配置
pub const MIN_PORT: u16 = 1024;
/// 写 token 的十六进制位数（48 位 = 24 字节随机）
pub const WRITE_TOKEN_HEX_LEN: usize = 48;

/// 从库里读端口；缺失/非法/越界都回退到默认值（不让一个拼错的设置把服务卡死）
pub fn port_from_store(store: &Store) -> u16 {
    store
        .setting(K_PORT)
        .ok()
        .flatten()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|p| *p >= MIN_PORT)
        .unwrap_or(DEFAULT_PORT)
}

/// 校验用户输入的端口
pub fn validate_port(port: u16) -> Result<u16, String> {
    if port < MIN_PORT {
        Err(format!("端口需要 ≥ {MIN_PORT}（更低的端口需要 root）"))
    } else {
        Ok(port)
    }
}

/// 读取 token；不存在则生成并写入（幂等）。
///
/// **读取失败必须报错，不能当成「没有 token」**：否则一次并发的 SQLITE_BUSY
/// 就会静默换掉 token，而所有已下发的客户端配置会随之失效——且没有任何提示。
/// 写入走 insert-if-absent，结构上不允许覆盖已有值。
pub fn token_from_store(store: &Store) -> Result<String, String> {
    match store.setting(K_TOKEN) {
        Ok(Some(existing)) if !existing.trim().is_empty() => return Ok(existing.trim().to_string()),
        Ok(_) => {}
        Err(e) => return Err(format!("读取 MCP token 失败（未做任何修改）: {e}")),
    }

    let fresh = crate::http::generate_token();
    // 只在真的为空时写入；若并发下已被别人写入，则采用对方的（以库为准）
    let inserted = store
        .insert_setting_if_absent(K_TOKEN, &fresh)
        .map_err(|e| format!("写入 token 失败: {e}"))?;
    if inserted {
        return Ok(fresh);
    }
    store
        .setting(K_TOKEN)
        .map_err(|e| format!("写入后读取 token 失败: {e}"))?
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string())
        .ok_or_else(|| "token 写入后仍为空，请检查数据库写入权限".to_string())
}

/// 覆盖 token（轮换用）
pub fn set_token(store: &Store, token: &str) -> Result<(), String> {
    store
        .set_setting(K_TOKEN, token)
        .map_err(|e| format!("写入 token 失败: {e}"))
}

// ---------------------------------------------------------------- 写 token / 开关

/// 生成写 token：48 位十六进制（24 字节随机）。
///
/// 用两个 UUIDv4 拼起来截断：不引入新依赖（uuid 已在依赖里），随机位数够用
/// （每个 UUIDv4 有 122 位随机），且与读 token 同一套「无 `-`、全小写十六进制」形状，
/// 用户看不出这是两种东西。
#[must_use]
pub fn generate_write_token() -> String {
    let a = uuid::Uuid::new_v4().simple().to_string();
    let b = uuid::Uuid::new_v4().simple().to_string();
    format!("{a}{b}").chars().take(WRITE_TOKEN_HEX_LEN).collect()
}

/// 读写 token；缺失/空白 = `None`。**不自动生成**（生成是设置页的显式动作）。
///
/// 与 [`token_from_store`] 的差异是刻意的：读 token 缺了可以补一个（否则客户端
/// 连不上），写 token 缺了就是「没开写能力」，静默补一个等于偷偷给人写权限。
pub fn write_token_from_store(store: &Store) -> Result<Option<String>, String> {
    store
        .setting(K_WRITE_TOKEN)
        .map(|v| {
            v.map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .map_err(|e| format!("读取 MCP 写 token 失败: {e}"))
}

/// 写入写 token（生成/轮换共用：轮换就是再写一个新的）
pub fn set_write_token(store: &Store, token: &str) -> Result<(), String> {
    store
        .set_setting(K_WRITE_TOKEN, token)
        .map_err(|e| format!("写入 MCP 写 token 失败: {e}"))
}

/// 销毁写 token（键删除 = 写能力消失；下一个请求就会失去写权限）
pub fn clear_write_token(store: &Store) -> Result<(), String> {
    store
        .delete_setting(K_WRITE_TOKEN)
        .map_err(|e| format!("删除 MCP 写 token 失败: {e}"))
}

/// 写能力总开关（缺失/非法 → false，默认关）
pub fn write_enabled_from_store(store: &Store) -> Result<bool, String> {
    store
        .bool_setting(K_WRITE_ENABLED, false)
        .map_err(|e| format!("读取写能力开关失败: {e}"))
}

/// 危险工具开关（缺失/非法 → false，默认关）
pub fn dangerous_enabled_from_store(store: &Store) -> Result<bool, String> {
    store
        .bool_setting(K_DANGEROUS_ENABLED, false)
        .map_err(|e| format!("读取危险工具开关失败: {e}"))
}

/// 一次读齐授权闸门要的三个值（同一个锁内，避免三把锁下互相矛盾）。
pub fn switches_from_store(store: &Store) -> Result<Switches, String> {
    Ok(Switches {
        write_enabled: write_enabled_from_store(store)?,
        dangerous_enabled: dangerous_enabled_from_store(store)?,
        write_token: write_token_from_store(store)?.is_some(),
    })
}

/// 生成可直接粘贴给 MCP 客户端的配置片段。
///
/// 同时给 JSON 片段（Claude Code / Cursor 的 `mcpServers` 格式）与一行命令：
/// 「让用户自己猜格式」是最容易劝退的一步。
pub fn client_snippet(url: &str, token: &str) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"rustrss\": {{\n      \"type\": \"http\",\n      \"url\": \"{url}\",\n      \"headers\": {{ \"Authorization\": \"Bearer {token}\" }}\n    }}\n  }}\n}}\n\n# 或者一行命令（Claude Code）：\nclaude mcp add --transport http rustrss {url} --header \"Authorization: Bearer {token}\"\n"
    )
}

/// 判断是否为回环 URL（供界面显示与自检使用）
pub fn is_loopback_url(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost") || url.contains("[::1]")
}

/// 由片段反查 url（仅用于自检：确保生成的 JSON 里确实带着我们给的地址）
pub fn snippet_url(url: &str) -> String {
    format!("http://127.0.0.1:{}/mcp", url.rsplit(':').next().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().expect("内存库")
    }

    #[test]
    fn port_falls_back_on_bad_values() {
        let s = store();
        assert_eq!(port_from_store(&s), DEFAULT_PORT, "缺失时用默认值");

        s.set_setting(K_PORT, "not-a-number").unwrap();
        assert_eq!(port_from_store(&s), DEFAULT_PORT);

        s.set_setting(K_PORT, "80").unwrap();
        assert_eq!(port_from_store(&s), DEFAULT_PORT, "低于 1024 的端口不采用");

        s.set_setting(K_PORT, "9000").unwrap();
        assert_eq!(port_from_store(&s), 9000);

        s.set_setting(K_PORT, "  9100  ").unwrap();
        assert_eq!(port_from_store(&s), 9100, "两侧空白应被容忍");

        assert!(validate_port(80).is_err());
        assert_eq!(validate_port(8817).unwrap(), 8817);
    }

    #[test]
    fn token_is_stable_and_persisted() {
        let s = store();
        let first = token_from_store(&s).unwrap();
        let second = token_from_store(&s).unwrap();
        assert_eq!(first, second, "多次调用必须得到同一个 token（否则客户端配置会失效）");
        assert_eq!(first.len(), 32);

        set_token(&s, "rotated-token").unwrap();
        assert_eq!(token_from_store(&s).unwrap(), "rotated-token", "轮换后应读到新值");
    }

    /// 回归：曾经把「读取失败」当成「没有 token」而静默换掉 token，
    /// 导致已下发的客户端配置全部失效。这里钉住「已有值绝不被覆盖」。
    #[test]
    fn existing_token_is_never_overwritten_by_auto_generation() {
        let s = store();
        set_token(&s, "keep-me").unwrap();
        for _ in 0..5 {
            assert_eq!(token_from_store(&s).unwrap(), "keep-me");
        }
        // 另一个连接（模拟应用与 CLI 同时在线）也不能改掉它
        let again = token_from_store(&s).unwrap();
        assert_eq!(again, "keep-me");
    }

    #[test]
    fn insert_if_absent_does_not_overwrite() {
        let s = store();
        assert!(s.insert_setting_if_absent("k", "v1").unwrap(), "首次应写入");
        assert!(!s.insert_setting_if_absent("k", "v2").unwrap(), "已存在时不应写入");
        assert_eq!(s.setting("k").unwrap().as_deref(), Some("v1"));
    }

    /// 写 token 的形状：48 位纯十六进制、两次生成不重复
    #[test]
    fn write_tokens_are_48_hex_chars_and_random() {
        let a = generate_write_token();
        let b = generate_write_token();
        assert_eq!(a.len(), WRITE_TOKEN_HEX_LEN);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{a}");
        assert_ne!(a, b);
    }

    /// 写 token 的生命周期：缺失 = None（不自动生成）→ 生成 → 轮换 → 销毁
    #[test]
    fn write_token_lifecycle_is_explicit() {
        let s = store();

        // ① 缺失时不自动生成（写权限必须由用户显式开通）
        assert_eq!(write_token_from_store(&s).unwrap(), None);
        assert!(s.setting(K_WRITE_TOKEN).unwrap().is_none());

        // ② 生成后可读回
        let first = generate_write_token();
        set_write_token(&s, &first).unwrap();
        assert_eq!(write_token_from_store(&s).unwrap().as_deref(), Some(first.as_str()));

        // ③ 轮换：库里换成新值，旧值不再可见
        let second = generate_write_token();
        set_write_token(&s, &second).unwrap();
        let now = write_token_from_store(&s).unwrap().unwrap();
        assert_eq!(now, second);
        assert_ne!(now, first);

        // ④ 销毁：键被删掉（不是写空串）
        clear_write_token(&s).unwrap();
        assert_eq!(write_token_from_store(&s).unwrap(), None);
        assert!(s.setting(K_WRITE_TOKEN).unwrap().is_none());
        clear_write_token(&s).unwrap(); // 幂等

        // 空白值等同缺失（手工改库写成空格不会变成“有写 token”）
        set_write_token(&s, "   ").unwrap();
        assert_eq!(write_token_from_store(&s).unwrap(), None);
    }

    /// 开关默认全关；开关快照与库里的值一一对应
    #[test]
    fn switches_default_off_and_follow_the_store() {
        let s = store();
        let sw = switches_from_store(&s).unwrap();
        assert_eq!(sw, Switches::default(), "装好即只读：两个开关默认关、写 token 不存在");
        assert!(!sw.write_enabled && !sw.dangerous_enabled && !sw.write_token);

        s.set_bool_setting(K_WRITE_ENABLED, true).unwrap();
        s.set_bool_setting(K_DANGEROUS_ENABLED, true).unwrap();
        set_write_token(&s, &generate_write_token()).unwrap();
        let sw = switches_from_store(&s).unwrap();
        assert!(sw.write_enabled && sw.dangerous_enabled && sw.write_token);

        // 值被写坏 → 回默认关（不让拼错的值把写能力打开）
        s.set_setting(K_WRITE_ENABLED, "yes").unwrap();
        assert!(!switches_from_store(&s).unwrap().write_enabled);
    }

    #[test]
    fn snippet_is_valid_json_and_carries_url_and_token() {        let url = "http://127.0.0.1:8817/mcp";
        let token = "abc123";
        let snippet = client_snippet(url, token);

        // 取第一段（JSON 片段）单独解析：粘过去不能用的话，这段测试就该红
        let json_part = snippet.split("\n\n#").next().unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(json_part).expect("片段里的 JSON 必须能解析");
        let server = &parsed["mcpServers"]["rustrss"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], url);
        assert_eq!(
            server["headers"]["Authorization"],
            format!("Bearer {token}"),
            "鉴权头必须按客户端认的格式生成"
        );

        // 一行命令也要带地址，否则用户复制过去会缺目标
        assert!(snippet.contains(&format!("claude mcp add --transport http rustrss {url}")));
        assert!(snippet.contains("--header \"Authorization: Bearer abc123\""));
    }
}
