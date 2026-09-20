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
pub fn resolve_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("RUSTSS_DB") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(p) = std::env::args().nth(1) {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
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
}
