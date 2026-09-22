//! 数据目录与数据库路径解析。
//!
//! 界面与 MCP 服务器**必须用同一套规则**：否则 agent 读的库和用户界面看的库
//! 可能不是同一个文件，表现为「agent 说没订阅但我明明订阅了」这类怪事。

use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "rustrss";
pub const DB_FILE: &str = "rustrss.sqlite";
pub const LOG_DIR: &str = "logs";

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

/// 日志目录（数据目录下 `logs/`）。
///
/// 与 [`default_data_dir()`] 同源，因此界面、MCP 与日志永远落在同一个数据目录下。
/// **不负责创建**（由 `logging::init` 按需建）。便携模式（程序同级 `data/`）当前未实现，
/// 未来落地后本函数会随 [`default_data_dir()`] 自动跟随。
pub fn logs_dir() -> PathBuf {
    default_data_dir().join(LOG_DIR)
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

/// 这个路径是否就是默认库位置。
///
/// 给单实例锁用：默认库下要挡住第二个实例（否则两个进程抢同一个 MCP 端口、
/// 双写同一个 SQLite），而 `$RUSTSS_DB` / 参数把库指到别处属于开发者跑诊断
/// 副本的合法用法，不该被锁住。
///
/// 只与 [`default_db_path()`] 比较，因此判定与当前工作目录无关。
///
/// 是**字面路径**比较：不做 canonicalize，也不碰文件系统（保持纯函数、可在
/// 测试里直接跑）。所以 `RUSTSS_DB` 写相对路径、而它恰好又指向默认库时会被
/// 当成「非默认库」——那时退化成今天的行为（不锁），不会是「误锁」。
pub fn is_default_db(db_path: &Path) -> bool {
    db_path == default_db_path()
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
    fn logs_dir_is_the_data_dirs_logs_subdir() {
        // 只断言路径组成（便携模式未实现，不做多模式断言）
        assert_eq!(logs_dir(), default_data_dir().join("logs"));
        assert_eq!(logs_dir().file_name().unwrap(), LOG_DIR);
        assert_eq!(logs_dir().parent().unwrap(), default_data_dir());
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

    #[test]
    fn only_the_default_path_counts_as_the_default_db() {
        // 默认库（含「被忽略的选项参数」这类输入）→ 认为是默认库，单实例锁生效
        assert!(is_default_db(&default_db_path()));
        assert!(is_default_db(&pick_db_path(None, Some("--print-config".into()))));

        // $RUSTSS_DB / 参数指向他处 → 不是默认库，锁要让开（可多开诊断副本）
        assert!(!is_default_db(&pick_db_path(
            Some("/tmp/other.sqlite".into()),
            None
        )));
        assert!(!is_default_db(&pick_db_path(
            None,
            Some("/tmp/other.sqlite".into())
        )));

        // 与 cwd 无关：只跟 default_db_path() 比，
        // 裸相对文件名（站在数据目录里跑时最容易被误判成默认库）不认
        assert!(!is_default_db(Path::new(DB_FILE)));
    }
}
