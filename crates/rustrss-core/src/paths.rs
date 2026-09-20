//! 数据目录与数据库路径解析。
//!
//! 界面与 MCP 服务器**必须用同一套规则**：否则 agent 读的库和用户界面看的库
//! 可能不是同一个文件，表现为「agent 说没订阅但我明明订阅了」这类怪事。

use std::path::PathBuf;

pub const APP_DIR: &str = "rustrss";
pub const DB_FILE: &str = "rustrss.sqlite";

#[cfg(target_os = "windows")]
fn platform_data_root() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(target_os = "macos")]
fn platform_data_root() -> PathBuf {
    home().join("Library").join("Application Support")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_data_root() -> PathBuf {
    // 遵循 XDG：$XDG_DATA_HOME 优先，否则 ~/.local/share
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.trim().is_empty() {
            return PathBuf::from(xdg);
        }
    }
    home().join(".local").join("share")
}

fn home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// 应用数据目录（不存在则不创建，由调用方按需创建）
pub fn default_data_dir() -> PathBuf {
    platform_data_root().join(APP_DIR)
}

/// 默认库位置
pub fn default_db_path() -> PathBuf {
    default_data_dir().join(DB_FILE)
}

/// 解析库位置：`$RUSTSS_DB` → 第一个命令行参数 → 平台默认位置。
///
/// 界面与 MCP 共用这个函数，因此两边永远指向同一个文件。
///
/// 注意：**只把“不像选项”的参数当作库路径**。曾经这里无条件取 `args[1]`，
/// 结果 `rustrss-mcp --print-config` 把 `--print-config` 当成了库路径，
/// 在当前目录凭空建了个同名数据库，token 也从那个空库里生成 ——
/// 表现为「应用和 CLI 看同一个库却读到不同 token」。
pub fn resolve_db_path() -> PathBuf {
    pick_db_path(
        std::env::var("RUSTSS_DB").ok(),
        std::env::args().nth(1),
    )
}

/// 可测的纯函数版：决定库位置
pub fn pick_db_path(env_value: Option<String>, first_arg: Option<String>) -> PathBuf {
    if let Some(p) = env_value {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(p) = first_arg {
        let trimmed = p.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('-') {
            return PathBuf::from(trimmed);
        }
    }
    default_db_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_is_inside_the_app_dir() {
        let p = default_db_path();
        assert_eq!(p.file_name().unwrap(), DB_FILE);
        assert_eq!(p.parent().unwrap().file_name().unwrap(), APP_DIR);
    }

    #[test]
    fn env_var_wins_over_default() {
        // 只验证解析顺序，不改真实环境：设置后立即读回
        let previous = std::env::var("RUSTSS_DB").ok();
        std::env::set_var("RUSTSS_DB", "/tmp/rustrss-explicit.sqlite");
        assert_eq!(resolve_db_path(), PathBuf::from("/tmp/rustrss-explicit.sqlite"));
        match previous {
            Some(v) => std::env::set_var("RUSTSS_DB", v),
            None => std::env::remove_var("RUSTSS_DB"),
        }
    }

    #[test]
    fn flags_are_not_mistaken_for_a_database_path() {
        // 回归：`--print-config` 这类选项曾被当成库路径，
        // 于是在 CWD 里建了个名为 `--print-config` 的数据库（应用与 CLI 因此读到不同 token）。
        let from_flag = pick_db_path(None, Some("--print-config".into()));
        assert_eq!(from_flag, default_db_path(), "选项不得被当作库路径");

        let from_short = pick_db_path(None, Some("-h".into()));
        assert_eq!(from_short, default_db_path());

        // 真正的路径仍然生效
        let explicit = pick_db_path(None, Some("/tmp/my.sqlite".into()));
        assert_eq!(explicit, PathBuf::from("/tmp/my.sqlite"));

        // 环境变量优先于参数
        let both = pick_db_path(Some("/tmp/env.sqlite".into()), Some("/tmp/arg.sqlite".into()));
        assert_eq!(both, PathBuf::from("/tmp/env.sqlite"));

        // 空值/空白一律忽略
        assert_eq!(pick_db_path(Some("   ".into()), None), default_db_path());
        assert_eq!(pick_db_path(None, Some("  ".into())), default_db_path());
    }
}
