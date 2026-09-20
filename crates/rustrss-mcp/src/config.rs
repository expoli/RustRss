//! MCP 服务的配置：端口 / token / 客户端配置片段。
//!
//! 放在 lib 而不是应用里，是因为**配置片段本身就是交付物**：
//! 「粘过去能不能用」取决于这段文本的格式是否正确，所以它必须可测，
//! 也必须能在不开 GUI 的情况下生成（`rustrss-mcp --print-config`）。

use rustrss_core::Store;

pub const K_PORT: &str = "mcp.port";
pub const K_TOKEN: &str = "mcp.token";
/// 默认端口：避开 quick-rss（8745）等常见占用
pub const DEFAULT_PORT: u16 = 8817;
/// 低于此值的端口需要 root，不提供给用户配置
pub const MIN_PORT: u16 = 1024;

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

/// 读取 token；不存在则生成并落库（幂等：多次调用得到同一个值）
pub fn token_from_store(store: &Store) -> Result<String, String> {
    if let Some(existing) = store
        .setting(K_TOKEN)
        .ok()
        .flatten()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        return Ok(existing);
    }
    let fresh = crate::http::generate_token();
    store
        .set_setting(K_TOKEN, &fresh)
        .map_err(|e| format!("写入 token 失败: {e}"))?;
    Ok(fresh)
}

/// 覆盖 token（轮换用）
pub fn set_token(store: &Store, token: &str) -> Result<(), String> {
    store
        .set_setting(K_TOKEN, token)
        .map_err(|e| format!("写入 token 失败: {e}"))
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
