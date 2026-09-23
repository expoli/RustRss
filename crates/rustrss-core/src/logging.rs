//! 日志基础设施：每次启动一个文件 + 启动时保留清理。
//!
//! 只依赖 `log` 门面与已有的 `chrono`，不依赖 Tauri，所以桌面端与 MCP 二进制
//! 都能复用同一套落点规则。设计口径见
//! `.chorus/specs/rss-reader/2026-09-22-logging/tech_design.md`（T1 / Module Contracts）。
//!
//! 三条硬约束：
//! - **写失败不能反过来弄崩应用**：`Log::log` 里忽略写入错误；
//! - **初始化失败返回 `Err` 由调用方降级**（提示一次继续跑），不 panic、不阻断启动；
//! - **清理幂等，且只认自己的 `rustrss-*.log`**：误删别人的文件是比日志没删干净更坏的事故。
//!
//! 可选的终端镜像（从终端调试时看得见日志）见 [`init_with_mirror`]：镜像行与文件行
//! **同源同格式**（同一份 `format_line` 字符串），镜像写失败同样静默忽略。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::{DateTime, Local, SecondsFormat};
use log::{Level, LevelFilter, Log, Metadata, Record};

/// 保留的日志文件个数上限（每次启动清理一次）。
pub const KEEP_FILES: usize = 20;
/// 保留的日志文件总字节上限（50 MB）。
pub const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;

const FILE_PREFIX: &str = "rustrss-";
const FILE_SUFFIX: &str = ".log";
/// 同一秒内最多容忍的启动次数（同秒冲突后缀 `-N` 的上限）。
const MAX_SAME_SECOND_FILES: u32 = 100;

/// 追加写单个日志文件的 [`log::Log`] 实现。
///
/// 锁内只做一次 `write_all` + `flush`（不 fsync），避免刷新线程被磁盘拖住；
/// 写失败不 panic —— 日志不能反过来把应用弄崩。
///
/// 可选的镜像目标（[`open_with_mirror`](Self::open_with_mirror)）：开启时同一行
/// 字符串同时写文件与镜像；镜像写失败静默忽略，绝不影响文件写入。
pub struct FileLogger {
    file: Mutex<File>,
    /// `Some` = 开启镜像。持 `Box<dyn Write + Send>` 而非 `Stderr` 是刻意的测试缝：
    /// 线上由 [`init_with_mirror`] 传 `io::stderr()`，单测传可读回的 writer 来断言
    /// 「镜像与文件行逐字节一致」，不去截真实 stderr。
    mirror: Option<Mutex<Box<dyn Write + Send>>>,
}

impl FileLogger {
    /// 打开（不存在则创建）一个文件作为日志落点，始终追加写（不镜像）。
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self::from_file(file))
    }

    /// 打开文件并开启镜像：每行同时写文件与 `mirror`（测试缝，见字段说明）。
    pub fn open_with_mirror(path: &Path, mirror: Box<dyn Write + Send>) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
            mirror: Some(Mutex::new(mirror)),
        })
    }

    fn from_file(file: File) -> Self {
        Self {
            file: Mutex::new(file),
            mirror: None,
        }
    }

    fn from_file_with_mirror(file: File, mirror: Box<dyn Write + Send>) -> Self {
        Self {
            file: Mutex::new(file),
            mirror: Some(Mutex::new(mirror)),
        }
    }

    /// 按行格式写一条记录（`Log::log` 与单测共用这条路径）。
    fn write_record(&self, record: &Record<'_>, now: DateTime<Local>) -> io::Result<()> {
        // 同一行只格式化一次：文件与镜像写同一份字符串，逐字节一致（各 format 一次
        // 会在毫秒边界分叉——两个时间戳同秒不同毫秒，用户贴出来对不上）。
        self.write_line(&format_line(record, now))
    }

    fn write_line(&self, line: &str) -> io::Result<()> {
        let written = self.write_file(line);
        // 镜像不因文件写失败而停：文件写不进去时，终端那份反而是唯一的线索。
        self.write_mirror(line);
        written
    }

    fn write_file(&self, line: &str) -> io::Result<()> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("日志文件锁中毒"))?;
        file.write_all(line.as_bytes())?;
        // 只把行交给内核（行级 flush），不做 fsync：日志丢几行可以接受，卡住 UI 不行。
        file.flush()
    }

    /// 把同一行写一份到镜像目标（未开启则什么都不做）。
    ///
    /// 行级 flush 保证交互式终端立即看到；写失败（stderr 已关 / EPIPE）一律吞掉——
    /// 终端那头没了不等于应用该出问题。
    fn write_mirror(&self, line: &str) {
        let Some(mirror) = &self.mirror else {
            return;
        };
        let Ok(mut sink) = mirror.lock() else {
            return;
        };
        let _ = sink.write_all(line.as_bytes());
        let _ = sink.flush();
    }
}

/// debug/trace 是否收录这条 target 的记录（info 及以上恒为 `true`）。
///
/// 全局 `set_max_level` 只能按级别切，管不了来源：打开 debug 后第三方依赖
/// （h2/hyper/reqwest…）的 debug 会一并进文件（实测占 2207 行里的 2186 行，而且
/// 绕过调用点自己的打码），所以「谁的 debug 能收」必须在 writer 侧判。
/// 只放行本应用（`rustrss*`：core / 桌面端 / MCP 三个 crate 的模块路径）与 `ui`
/// （前端 `ui_log` 通道的 target）；info/warn/error 不限 target——依赖库的
/// 报错信息仍是排障线索。
fn target_allowed(level: Level, target: &str) -> bool {
    if !matches!(level, Level::Debug | Level::Trace) {
        return true;
    }
    target.starts_with("rustrss") || target == "ui"
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
            && target_allowed(metadata.level(), metadata.target())
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let _ = self.write_record(record, Local::now());
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

/// 日志行格式：`{本地时间 RFC3339 毫秒} {LEVEL:<5} {target}: {message}`。
///
/// 时间戳带本地偏移（`2026-09-22T23:45:01.123+08:00`）——RFC3339 要求偏移字段，
/// 且把日志发给别人时不会与本地时区混淆。
fn format_line(record: &Record<'_>, now: DateTime<Local>) -> String {
    format!(
        "{} {:<5} {}: {}\n",
        now.to_rfc3339_opts(SecondsFormat::Millis, false),
        record.level().as_str(),
        record.target(),
        record.args()
    )
}

/// 创建日志目录与**本次启动**的日志文件，并安装全局 logger（级别 `level`）。
///
/// 返回本次日志文件路径。失败返回 `Err` —— 调用方降级（提示一次并继续运行），
/// 本函数不 panic、不阻断启动。
///
/// 全局 logger 只能装一次：若已有其它 logger（重复调用 init），保留先安装的那个，
/// 本次文件仍会创建并返回路径（可能保持为空），不算失败。
pub fn init(log_dir: &Path, level: LevelFilter) -> Result<PathBuf, String> {
    init_with_mirror(log_dir, level, false)
}

/// 终端镜像判定（**纯函数**，调用方在启动早期判定一次并把结果传给
/// [`init_with_mirror`]，运行期不再重判）。
///
/// - `Some("1")` → 开（脚本/无头场景显式开启，此时 stdout 往往不是终端）；
/// - `Some("0")` → 关（重定向、启动器场景显式关闭）；
/// - 其它（未设置、空串、`"true"`/`" 1 "` 这类非法值）→ 回退 `stdout_is_terminal`。
///   非法值按「没设」处理：宁可回退到探测结果，也不猜用户想开还是想关。
pub fn mirror_enabled(env_value: Option<&str>, stdout_is_terminal: bool) -> bool {
    match env_value {
        Some("1") => true,
        Some("0") => false,
        _ => stdout_is_terminal,
    }
}

/// [`init`] 的带镜像版本：`mirror_stderr = true` 时每条落文件的日志行同时镜像到 stderr。
///
/// 与 [`init`] 同口径：失败返回 `Err` 由调用方降级（不 panic、不阻断启动）；全局 logger
/// 只装一次（已有 logger 时保留先装的，本次文件仍创建并返回路径）。
pub fn init_with_mirror(
    log_dir: &Path,
    level: LevelFilter,
    mirror_stderr: bool,
) -> Result<PathBuf, String> {
    let path = create_log_file(log_dir)?;
    let file = OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开日志文件 {} 失败：{e}", path.display()))?;
    let logger = if mirror_stderr {
        // 镜像统一写 stderr：stdout 留给可能的管道消费者，不被诊断行污染。
        FileLogger::from_file_with_mirror(file, Box::new(io::stderr()))
    } else {
        FileLogger::from_file(file)
    };
    log::set_max_level(level);
    let _ = log::set_boxed_logger(Box::new(logger));
    Ok(path)
}

/// 安装 panic hook：先把 panic 写进日志（payload + 位置），再调用**原 hook**。
///
/// 「原 hook」= 调用本函数时已经装上的那个（取走后包在外层调用）——所以它之前装的
/// hook 仍然会执行，stderr 的既有行为（以及 `RUST_BACKTRACE` 回溯）不受影响。
///
/// 与 [`init`] 一样**不 panic、不阻断**：logger 没装上时 `log::error!` 静默无事，
/// 原 hook 照旧执行；日志写入失败也不会把「一次 panic」变成「两次」。
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "位置未知".to_string());
        // payload + 位置：用户贴日志时这两条是定位的全部依据（stderr 那份用户拿不到）
        log::error!(
            "panic: {}（location={location}）",
            panic_payload_text(info.payload())
        );
        previous(info);
    }));
}

/// panic payload 的可读文本：`panic!("...")` 与 `panic!("{x}")` 两种形态都要认。
fn panic_payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        // 非字符串 payload（`panic_any`）拿不到内容，如实说而不是空着
        "非字符串 payload（类型不可读）".to_string()
    }
}

/// 只建目录与本次日志文件，不碰全局状态（给调用方自组 / 测试用）。
pub fn create_log_file(log_dir: &Path) -> Result<PathBuf, String> {
    create_log_file_at(log_dir, Local::now())
}

/// 可测版本：把「现在」当参数传入，同秒冲突即可确定性复现。
fn create_log_file_at(log_dir: &Path, now: DateTime<Local>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(log_dir)
        .map_err(|e| format!("创建日志目录 {} 失败：{e}", log_dir.display()))?;
    for seq in 0..MAX_SAME_SECOND_FILES {
        let path = log_dir.join(log_file_name(now, seq));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            // create_new：同秒启动不覆盖、不追加到别人的文件里
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("创建日志文件 {} 失败：{e}", path.display())),
        }
    }
    Err(format!(
        "同一秒内日志文件超过 {} 个：{}",
        MAX_SAME_SECOND_FILES,
        log_dir.display()
    ))
}

/// 文件名 `rustrss-YYYYMMDD-HHMMSS.log`（本地时间）；同秒冲突 seq > 0 → `...-N.log`。
fn log_file_name(now: DateTime<Local>, seq: u32) -> String {
    let stem = now.format("%Y%m%d-%H%M%S");
    if seq == 0 {
        format!("{FILE_PREFIX}{stem}{FILE_SUFFIX}")
    } else {
        format!("{FILE_PREFIX}{stem}-{seq}{FILE_SUFFIX}")
    }
}

/// `prune` 结果：删了几个、释放多少、还剩多少（超限时如实报告，不假装成功）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PruneReport {
    pub removed: usize,
    pub removed_bytes: u64,
    pub remaining_files: usize,
    pub remaining_bytes: u64,
}

/// 启动时清理：按 mtime 从旧到新删，直到同时满足「文件数 ≤ `keep_files`」
/// 与「总量 ≤ `max_total_bytes`」。
///
/// - **幂等**：已经满足上限时再调一次不再删任何东西；
/// - **至少保留 1 个（最新）文件**：单个文件自身就超上限时把它留下——否则等于
///   把这次启动的日志也删了，日志功能就白做了（此时 `remaining_bytes` 会如实超限）；
/// - 只认 `rustrss-*.log`，目录里的其它文件一律不碰；
/// - 目录不存在 / 不可读时返回空报告，不 panic。
pub fn prune(log_dir: &Path, keep_files: usize, max_total_bytes: u64) -> PruneReport {
    let mut files = collect_log_files(log_dir);
    let remaining_files = files.len();
    let remaining_bytes: u64 = files.iter().map(|f| f.size).sum();

    // 旧 → 新；mtime 打平时用同秒后缀序号定序（0 是基准名，先于 -1、-2…），保证确定性。
    files.sort_by(|a, b| {
        a.mtime
            .cmp(&b.mtime)
            .then(a.seq.cmp(&b.seq))
            .then(a.path.cmp(&b.path))
    });

    let mut report = PruneReport {
        removed: 0,
        removed_bytes: 0,
        remaining_files,
        remaining_bytes,
    };
    for f in &files {
        if report.remaining_files <= keep_files && report.remaining_bytes <= max_total_bytes {
            break;
        }
        if report.remaining_files <= 1 {
            break; // 绝不删最后一个：保住当前（最新）日志
        }
        if std::fs::remove_file(&f.path).is_ok() {
            report.removed += 1;
            report.removed_bytes += f.size;
            report.remaining_files -= 1;
            report.remaining_bytes -= f.size;
        }
    }
    report
}

struct LogFile {
    path: PathBuf,
    mtime: SystemTime,
    /// 同秒冲突后缀（基准名 = 0），仅用于 mtime 打平时的稳定排序
    seq: u32,
    size: u64,
}

fn collect_log_files(log_dir: &Path) -> Vec<LogFile> {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let name = name.to_str()?;
            if !is_log_file_name(name) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(LogFile {
                path: entry.path(),
                // 拿不到 mtime 就当成最旧（UNIX_EPOCH），保证仍能被清理掉
                mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                seq: seq_of(name),
                size: meta.len(),
            })
        })
        .collect()
}

/// 只认本应用自己的日志文件 `rustrss-*.log`。
fn is_log_file_name(name: &str) -> bool {
    name.starts_with(FILE_PREFIX)
        && name.ends_with(FILE_SUFFIX)
        && name.len() > FILE_PREFIX.len() + FILE_SUFFIX.len()
}

/// 从 `rustrss-YYYYMMDD-HHMMSS[-N].log` 里取同秒后缀（无后缀 = 0，非法 = 0）。
fn seq_of(name: &str) -> u32 {
    let stem = name
        .strip_prefix(FILE_PREFIX)
        .and_then(|s| s.strip_suffix(FILE_SUFFIX))
        .unwrap_or(name);
    // "YYYYMMDD-HHMMSS" 固定 15 字节
    stem.get(15..)
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};
    use std::sync::Arc;
    use std::time::Duration;

    /// 可注入的镜像目标：写入内容攒在共享 buffer 里，测试可读回逐字节比对。
    #[derive(Clone, Default)]
    struct CaptureSink(Arc<Mutex<Vec<u8>>>);

    impl CaptureSink {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().expect("镜像 buffer 锁不应中毒").clone())
                .expect("镜像内容应是 UTF-8")
        }
    }

    impl Write for CaptureSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("镜像 buffer 锁不应中毒")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// 模拟「stderr 已关 / 管道对端没了」：写入必败。
    struct BrokenSink;

    impl Write for BrokenSink {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "stderr 已关"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "stderr 已关"))
        }
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustrss-logging-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("测试目录应能创建");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 基准 mtime：固定值 + 序号秒，让「旧 → 新」完全可控。
    const BASE_MTIME_SECS: u64 = 1_700_000_000;

    fn mk_log(dir: &Path, name: &str, bytes: usize, mtime_secs: u64) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'x'; bytes]).expect("应能写测试日志");
        let file = OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("应能打开测试日志");
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(mtime_secs))
            .expect("应能设置 mtime");
        path
    }

    fn log_name(secs: u64) -> String {
        format!("rustrss-20260101-{:06}.log", secs)
    }

    fn fixed_now() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .unwrap()
            .with_nanosecond(123_000_000)
            .unwrap()
    }

    /// 造一条日志记录（宏在调用点展开，`format_args!` 才能借用调用处的局部变量）。
    macro_rules! record {
        ($level:expr, $target:expr, $($arg:tt)*) => {
            Record::builder()
                .args(format_args!($($arg)*))
                .level($level)
                .target($target)
                .build()
        };
    }

    #[test]
    fn file_name_is_local_time_with_expected_shape() {
        let now = fixed_now();
        assert_eq!(log_file_name(now, 0), "rustrss-20260102-030405.log");
        assert_eq!(log_file_name(now, 1), "rustrss-20260102-030405-1.log");
        assert_eq!(log_file_name(now, 12), "rustrss-20260102-030405-12.log");
    }

    #[test]
    fn same_second_start_gets_a_suffix_instead_of_overwriting() {
        let dir = tmp_dir("same-second");
        let now = fixed_now();
        let first = create_log_file_at(&dir, now).expect("第一次应成功");
        let second = create_log_file_at(&dir, now).expect("同秒第二次也应成功");

        assert_eq!(first.file_name().unwrap(), "rustrss-20260102-030405.log");
        assert_eq!(second.file_name().unwrap(), "rustrss-20260102-030405-1.log");
        assert!(first.is_file() && second.is_file(), "同秒两次不得互相覆盖");
        cleanup(&dir);
    }

    /// AC1 真值表：`1` 强制开、`0` 强制关，其余（未设置 / 空串 / 非法值）跟随 TTY 输入。
    /// 非法值刻意不猜用户意图（`"true"` / `" 1 "` / `"2"` / `"01"` 都按未设置处理）。
    #[test]
    fn mirror_enabled_truth_table() {
        let cases: &[(Option<&str>, bool, bool)] = &[
            (Some("1"), false, true), // 非 TTY 也能靠 env 打开（脚本/无头场景）
            (Some("1"), true, true),
            (Some("0"), true, false), // TTY 也能靠 env 关掉
            (Some("0"), false, false),
            (None, true, true), // 未设置 → 跟随 TTY
            (None, false, false),
            (Some(""), true, true), // 空串 = 未设置
            (Some(""), false, false),
            (Some("true"), true, true), // 非法值 → 跟随 TTY
            (Some("true"), false, false),
            (Some(" 1 "), true, true), // 带空白不 trim：同样按非法值回退
            (Some(" 1 "), false, false),
            (Some("2"), true, true),
            (Some("01"), false, false),
        ];
        for &(env, tty, expected) in cases {
            assert_eq!(
                mirror_enabled(env, tty),
                expected,
                "env={env:?} stdout_is_terminal={tty}"
            );
        }
    }

    /// AC2：镜像开启时镜像内容与文件行**逐字节一致**（同一行字符串只格式化一次）。
    #[test]
    fn mirror_lines_are_byte_identical_to_file_lines() {
        let dir = tmp_dir("mirror-identical");
        let path = dir.join("mirror.log");
        let sink = CaptureSink::default();
        let logger = FileLogger::open_with_mirror(&path, Box::new(sink.clone()))
            .expect("应能打开带镜像的 logger");
        let now = fixed_now();

        logger
            .write_record(&record!(Level::Info, "ui", "[ui] view=all 共 3 条"), now)
            .expect("写入应成功");
        logger
            .write_record(
                &record!(Level::Error, "rustrss_core::store", "写库失败"),
                now,
            )
            .expect("写入应成功");

        let file_text = std::fs::read_to_string(&path).expect("应能读回日志");
        assert_eq!(sink.text(), file_text, "镜像行必须与文件行逐字节一致");
        assert_eq!(file_text.lines().count(), 2, "两条记录两行：{file_text:?}");
        assert!(
            file_text.contains(" INFO  ui: [ui] view=all 共 3 条")
                && file_text.contains(" ERROR rustrss_core::store: 写库失败"),
            "行格式不变：{file_text:?}"
        );
        cleanup(&dir);
    }

    /// AC2：镜像只跟随「通过级别 + target 过滤」的行——与文件行一一对应，
    /// 被过滤掉的依赖库 debug 两边都不出现（不是额外数据源）。
    #[test]
    fn mirror_follows_the_same_level_and_target_gate_as_the_file() {
        let dir = tmp_dir("mirror-gate");
        let path = dir.join("gate.log");
        let sink = CaptureSink::default();
        let logger = FileLogger::open_with_mirror(&path, Box::new(sink.clone()))
            .expect("应能打开带镜像的 logger");

        // 只放大不缩小全局级别，避免与并行用例互相掐架
        log::set_max_level(LevelFilter::Trace);
        Log::log(&logger, &record!(Level::Info, "ui", "ui-info"));
        Log::log(&logger, &record!(Level::Debug, "h2::codec", "dep-debug"));
        Log::log(
            &logger,
            &record!(Level::Debug, "rustrss_core::fetch", "app-debug"),
        );
        Log::log(&logger, &record!(Level::Error, "hyper::proto", "dep-error"));

        let file_text = std::fs::read_to_string(&path).expect("应能读回日志");
        assert_eq!(sink.text(), file_text, "镜像与文件行集合必须一致");
        assert!(
            !file_text.contains("dep-debug"),
            "依赖库 debug 两边都不该出现：{file_text:?}"
        );
        assert_eq!(file_text.lines().count(), 3, "恰好 3 条：{file_text:?}");
        cleanup(&dir);
    }

    /// AC2：镜像关闭时不产生任何镜像输出——`open`/`init` 路径根本没有镜像目标。
    #[test]
    fn mirror_off_has_no_mirror_target_and_writes_only_the_file() {
        let dir = tmp_dir("mirror-off");
        let path = dir.join("off.log");
        let logger = FileLogger::open(&path).expect("应能打开 logger");
        assert!(logger.mirror.is_none(), "未开启镜像时不该有镜像目标");

        logger
            .write_record(&record!(Level::Info, "ui", "only-file"), fixed_now())
            .expect("写入应成功");

        let text = std::fs::read_to_string(&path).expect("应能读回日志");
        assert!(text.contains(" INFO  ui: only-file"), "{text:?}");
        cleanup(&dir);
    }

    /// AC3：镜像写入失败（stderr 已关 / EPIPE）不 panic、不影响文件写入；
    /// 后续行照常落文件（失败按行独立，不污染后面的行）。
    #[test]
    fn broken_mirror_does_not_panic_or_disturb_the_file() {
        let dir = tmp_dir("mirror-broken");
        let path = dir.join("broken.log");
        let logger = FileLogger::open_with_mirror(&path, Box::new(BrokenSink))
            .expect("应能打开带镜像的 logger");
        let now = fixed_now();

        logger
            .write_record(&record!(Level::Info, "ui", "第一条"), now)
            .expect("文件写入应成功（不受镜像失败影响）");
        logger
            .write_record(&record!(Level::Info, "ui", "第二条"), now)
            .expect("后续行也应成功");

        let text = std::fs::read_to_string(&path).expect("应能读回日志");
        assert_eq!(
            text.lines().count(),
            2,
            "镜像失败不得吞掉或破坏文件行：{text:?}"
        );
        assert!(
            text.contains("第一条") && text.contains("第二条"),
            "{text:?}"
        );
        cleanup(&dir);
    }

    /// AC5：`init_with_mirror` 与 `init` 同口径——建目录与本次文件、返回
    /// `Result<PathBuf, String>`、目录不可建时同样返回 Err 而不是 panic。
    #[test]
    fn init_with_mirror_shares_init_contract() {
        let dir = tmp_dir("init-with-mirror");
        let log_dir = dir.join("logs"); // 故意先不存在：init_with_mirror 负责创建
        let path = init_with_mirror(&log_dir, LevelFilter::Debug, false).expect("应成功");
        assert!(log_dir.is_dir(), "应创建日志目录");
        assert!(
            path.starts_with(&log_dir) && path.is_file(),
            "应返回本次文件路径"
        );

        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"not a dir").expect("应能造出阻挡目录创建的普通文件");
        let err = init_with_mirror(&blocked.join("logs"), LevelFilter::Info, false)
            .expect_err("应返回 Err 而不是 panic");
        assert!(err.contains("创建日志目录"), "错误信息应可读：{err}");
        cleanup(&dir);
    }

    #[test]
    fn writer_line_has_local_millis_level_target_and_message() {
        let dir = tmp_dir("writer-line");
        let path = dir.join("writer.log");
        let logger = FileLogger::open(&path).expect("应能打开日志文件");

        logger
            .write_record(
                &record!(log::Level::Info, "ui", "view=all 共 3 条"),
                fixed_now(),
            )
            .expect("写入应成功");

        let text = std::fs::read_to_string(&path).expect("应能读回日志");
        assert_eq!(text.lines().count(), 1, "一条记录一行：{text:?}");

        // 时间戳：本地墙上时间 + 毫秒 + 可被 RFC3339 解析（含偏移）
        let stamp = text.split_whitespace().next().unwrap();
        assert!(stamp.contains(".123"), "毫秒应保留：{stamp}");
        let parsed = DateTime::parse_from_rfc3339(stamp).expect("时间戳应是 RFC3339");
        assert_eq!(parsed.naive_local(), fixed_now().naive_local());
        assert_eq!(parsed.timestamp_millis() % 1000, 123);

        assert!(
            text.contains(" INFO  ui: view=all 共 3 条"),
            "级别 <5 补齐 + target + message：{text:?}"
        );
        cleanup(&dir);
    }

    #[test]
    fn writer_appends_multiple_lines_in_order() {
        let dir = tmp_dir("writer-append");
        let path = dir.join("append.log");
        let logger = FileLogger::open(&path).expect("应能打开日志文件");
        let now = fixed_now();

        logger
            .write_record(
                &record!(log::Level::Info, "rustrss_core::fetch", "first"),
                now,
            )
            .expect("第一条应成功");
        logger
            .write_record(
                &record!(log::Level::Error, "rustrss_core::store", "second"),
                now,
            )
            .expect("第二条应成功");

        let text = std::fs::read_to_string(&path).expect("应能读回日志");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "应追加而不是覆盖：{text:?}");
        assert!(lines[0].ends_with(" INFO  rustrss_core::fetch: first"));
        // ERROR 恰好 5 字符：只隔一个空格
        assert!(
            lines[1].ends_with(" ERROR rustrss_core::store: second"),
            "{:?}",
            lines[1]
        );
        cleanup(&dir);
    }

    #[test]
    fn log_trait_writes_through_the_level_gate() {
        let dir = tmp_dir("writer-trait");
        let path = dir.join("trait.log");
        let logger = FileLogger::open(&path).expect("应能打开日志文件");

        // 只放大不缩小全局级别，避免与并行用例互相掐架（本文件其它用例只用 Info/Debug）
        log::set_max_level(LevelFilter::Trace);
        Log::log(&logger, &record!(log::Level::Info, "ui", "hello"));

        let text = std::fs::read_to_string(&path).expect("应能读回日志");
        assert!(text.contains(" INFO  ui: hello"), "{text:?}");
        cleanup(&dir);
    }

    /// debug 目标过滤（T3 AC4）的纯函数口径：本应用（`rustrss*` / `ui`）放行，
    /// 依赖库丢弃；info 及以上一律放行。
    #[test]
    fn debug_target_filter_admits_app_and_ui_only() {
        for target in [
            "rustrss",
            "rustrss_core::fetch",
            "rustrss_desktop::commands",
            "rustrss_mcp::http",
            "ui",
        ] {
            for level in [Level::Debug, Level::Trace] {
                assert!(
                    target_allowed(level, target),
                    "{target} 的 {level} 应放行（本应用/前端）"
                );
            }
        }

        for target in [
            "",
            "h2::codec",
            "hyper::proto",
            "hyper_util::client",
            "reqwest::async_impl",
            "rustls::client",
            " UI",
            "rustrssx",
        ] {
            let expected = target == "rustrssx"; // 前缀匹配：rustrssx 也算本应用（保守放行）
            for level in [Level::Debug, Level::Trace] {
                assert_eq!(
                    target_allowed(level, target),
                    expected,
                    "{target:?} 的 {level} 放行口径不符"
                );
            }
            for level in [Level::Info, Level::Warn, Level::Error] {
                assert!(
                    target_allowed(level, target),
                    "{target:?} 的 {level} 不限 target（依赖库错误仍是排障线索）"
                );
            }
        }
    }

    /// 写侧端到端：全局门开到 Trace 时，依赖库的 debug/trace 仍不落盘，
    /// 本应用与 `ui` 的 debug 落盘，依赖库的 error/warn 落盘。
    /// 同一条记录按 target 分叉，证明是 target 规则（而非级别门）在起作用。
    #[test]
    fn writer_drops_dependency_debug_but_keeps_their_errors() {
        let dir = tmp_dir("writer-target-filter");
        let path = dir.join("filter.log");
        let logger = FileLogger::open(&path).expect("应能打开日志文件");

        log::set_max_level(LevelFilter::Trace);
        Log::log(&logger, &record!(Level::Debug, "h2::codec", "dep-debug"));
        Log::log(&logger, &record!(Level::Trace, "hyper::proto", "dep-trace"));
        Log::log(&logger, &record!(Level::Debug, "ui", "ui-debug"));
        Log::log(
            &logger,
            &record!(Level::Debug, "rustrss_core::fetch", "app-debug"),
        );
        Log::log(&logger, &record!(Level::Error, "hyper::proto", "dep-error"));
        Log::log(&logger, &record!(Level::Warn, "reqwest::async_impl", "dep-warn"));

        let text = std::fs::read_to_string(&path).expect("应能读回日志");
        assert!(!text.contains("dep-debug"), "依赖库 debug 不该落盘：{text:?}");
        assert!(!text.contains("dep-trace"), "依赖库 trace 不该落盘：{text:?}");
        assert!(text.contains(" DEBUG ui: ui-debug"), "{text:?}");
        assert!(
            text.contains(" DEBUG rustrss_core::fetch: app-debug"),
            "{text:?}"
        );
        assert!(
            text.contains(" ERROR hyper::proto: dep-error"),
            "依赖库 error 必须收录：{text:?}"
        );
        assert!(
            text.contains(" WARN  reqwest::async_impl: dep-warn"),
            "依赖库 warn 必须收录：{text:?}"
        );
        assert_eq!(text.lines().count(), 4, "恰好 4 条：{text:?}");
        cleanup(&dir);
    }

    #[test]
    fn init_creates_dir_and_this_run_file() {
        let dir = tmp_dir("init");
        let log_dir = dir.join("logs"); // 故意先不存在：init 负责创建
        let path = init(&log_dir, LevelFilter::Debug).expect("init 应成功");

        assert!(log_dir.is_dir(), "init 应创建日志目录");
        assert!(path.starts_with(&log_dir), "日志文件应落在传入目录下");
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        assert!(name.starts_with("rustrss-"), "{name}");
        assert!(name.ends_with(".log"), "{name}");
        assert_eq!(name.len(), "rustrss-YYYYMMDD-HHMMSS.log".len(), "{name}");
        assert!(path.is_file());
        cleanup(&dir);
    }

    #[test]
    fn init_returns_err_instead_of_panicking_when_dir_cannot_be_created() {
        let dir = tmp_dir("init-err");
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"not a dir").expect("应能造出阻挡目录创建的普通文件");

        let err = init(&blocked.join("logs"), LevelFilter::Info).expect_err("应返回 Err 而不是 panic");
        assert!(err.contains("创建日志目录"), "错误信息应可读：{err}");
        cleanup(&dir);
    }

    #[test]
    fn prune_keeps_exactly_the_limit() {
        let dir = tmp_dir("prune-keep-limit");
        for i in 0..KEEP_FILES as u64 {
            mk_log(&dir, &log_name(i), 10, BASE_MTIME_SECS + i);
        }

        let report = prune(&dir, KEEP_FILES, MAX_TOTAL_BYTES);
        assert_eq!(report.removed, 0, "恰好 20 个不该删");
        assert_eq!(report.remaining_files, KEEP_FILES);
        assert_eq!(report.remaining_bytes, 10 * KEEP_FILES as u64);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), KEEP_FILES);
        cleanup(&dir);
    }

    #[test]
    fn prune_deletes_oldest_when_over_the_file_limit() {
        let dir = tmp_dir("prune-21");
        let paths: Vec<PathBuf> = (0..=KEEP_FILES as u64)
            .map(|i| mk_log(&dir, &log_name(i), 10, BASE_MTIME_SECS + i))
            .collect();

        let report = prune(&dir, KEEP_FILES, MAX_TOTAL_BYTES);
        assert_eq!(report.removed, 1, "21 个应删 1 个");
        assert!(!paths[0].exists(), "最旧的一个应被删");
        assert!(paths[1..].iter().all(|p| p.exists()), "其余应保留");
        assert_eq!(report.remaining_files, KEEP_FILES);
        assert_eq!(report.removed_bytes, 10);
        cleanup(&dir);
    }

    #[test]
    fn prune_deletes_by_mtime_not_by_name() {
        let dir = tmp_dir("prune-mtime");
        // 名字看着「更新」的文件其实更旧（时钟回拨 / 拷贝）→ 按 mtime 应删它
        let looks_new = mk_log(&dir, "rustrss-20260101-235959.log", 10, BASE_MTIME_SECS);
        let looks_old = mk_log(&dir, "rustrss-20260101-000000.log", 10, BASE_MTIME_SECS + 100);

        let report = prune(&dir, 1, MAX_TOTAL_BYTES);
        assert_eq!(report.removed, 1);
        assert!(!looks_new.exists(), "按 mtime 应删更旧的那个");
        assert!(looks_old.exists());
        cleanup(&dir);
    }

    #[test]
    fn prune_deletes_until_total_bytes_under_the_limit() {
        let dir = tmp_dir("prune-bytes");
        // 5 × 1000 字节，上限 2500 → 删最旧 3 个，剩 2 个 2000 字节
        let paths: Vec<PathBuf> = (0..5)
            .map(|i| mk_log(&dir, &log_name(i), 1000, BASE_MTIME_SECS + i))
            .collect();

        let report = prune(&dir, KEEP_FILES, 2500);
        assert_eq!(report.removed, 3);
        assert_eq!(report.remaining_files, 2);
        assert_eq!(report.remaining_bytes, 2000);
        assert!(report.remaining_bytes <= 2500, "删完应在字节上限内");
        assert!(paths[0..3].iter().all(|p| !p.exists()), "从最旧的删");
        assert!(paths[3..].iter().all(|p| p.exists()), "最新的留下");
        cleanup(&dir);
    }

    #[test]
    fn prune_keeps_newest_file_even_when_it_alone_exceeds_the_limit() {
        let dir = tmp_dir("prune-single-too-big");
        // 单个文件自身超上限：不能把它也删了，否则这次启动就没有日志
        let only = mk_log(&dir, &log_name(0), 1000, BASE_MTIME_SECS);

        let report = prune(&dir, KEEP_FILES, 10);
        assert_eq!(report.removed, 0);
        assert!(only.exists(), "唯一的日志文件不得被删");
        assert_eq!(report.remaining_files, 1);
        assert_eq!(report.remaining_bytes, 1000, "超限要如实报告，不假装成功");
        cleanup(&dir);
    }

    #[test]
    fn prune_keeps_at_least_the_newest_file() {
        let dir = tmp_dir("prune-keep-one");
        let paths: Vec<PathBuf> = (0..3)
            .map(|i| mk_log(&dir, &log_name(i), 10, BASE_MTIME_SECS + i))
            .collect();

        // keep_files = 0 也至少留最新那个（当前正在写的日志）
        let report = prune(&dir, 0, MAX_TOTAL_BYTES);
        assert_eq!(report.removed, 2);
        assert_eq!(report.remaining_files, 1);
        assert!(paths[2].exists(), "最新（当前）日志必须留下");
        cleanup(&dir);
    }

    #[test]
    fn prune_is_idempotent() {
        let dir = tmp_dir("prune-idempotent");
        for i in 0..=KEEP_FILES as u64 {
            mk_log(&dir, &log_name(i), 10, BASE_MTIME_SECS + i);
        }

        let first = prune(&dir, KEEP_FILES, MAX_TOTAL_BYTES);
        assert_eq!(first.removed, 1);
        let after_first = std::fs::read_dir(&dir).unwrap().count();

        let second = prune(&dir, KEEP_FILES, MAX_TOTAL_BYTES);
        let third = prune(&dir, KEEP_FILES, MAX_TOTAL_BYTES);
        assert_eq!(second.removed, 0, "重复调用不应再删");
        assert_eq!(third.removed, 0);
        assert_eq!(second, third, "重复调用结果稳定");
        assert_eq!(second.remaining_files, first.remaining_files);
        assert_eq!(second.remaining_bytes, first.remaining_bytes);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), after_first);
        cleanup(&dir);
    }

    #[test]
    fn prune_never_touches_non_log_files() {
        let dir = tmp_dir("prune-foreign");
        for i in 0..=KEEP_FILES as u64 {
            mk_log(&dir, &log_name(i), 10, BASE_MTIME_SECS + i);
        }
        let txt = dir.join("important.txt");
        std::fs::write(&txt, b"keep me").unwrap();
        let foreign = dir.join("other-app.log");
        std::fs::write(&foreign, b"keep me too").unwrap();

        let report = prune(&dir, KEEP_FILES, MAX_TOTAL_BYTES);
        assert_eq!(report.removed, 1, "只删本应用的日志");
        assert!(txt.exists(), "非日志文件不得误删");
        assert!(foreign.exists(), "别人的 .log 也不得误删");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), KEEP_FILES + 2);
        cleanup(&dir);
    }

    #[test]
    fn prune_on_missing_dir_is_a_noop() {
        let dir = tmp_dir("prune-missing");
        let report = prune(&dir.join("does-not-exist"), KEEP_FILES, MAX_TOTAL_BYTES);
        assert_eq!(report, PruneReport::default());
        cleanup(&dir);
    }

    /// panic payload 的两种常规形态（`panic!("...")` → `&str`；`panic!("{x}")` → `String`）
    /// 都要读出内容，其它类型如实标注（不留空白，否则日志里就只剩「有个 panic」）。
    #[test]
    fn panic_payload_text_reads_str_and_string_payloads() {
        let literal: &(dyn std::any::Any + Send) = &"字面量载荷";
        assert_eq!(panic_payload_text(literal), "字面量载荷");

        let owned = String::from("String 载荷");
        let owned_ref: &(dyn std::any::Any + Send) = &owned;
        assert_eq!(panic_payload_text(owned_ref), "String 载荷");

        let number: &(dyn std::any::Any + Send) = &42u32;
        assert!(panic_payload_text(number).contains("非字符串"));
    }
}
