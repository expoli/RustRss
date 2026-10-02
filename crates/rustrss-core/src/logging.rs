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
//!
//! 写入之外还有一套**只读访问 API**（日志查看/导出的数据面）：[`list_log_files`]
//! 列目录、[`read_log_tail`] 读末尾、[`read_log_file`] 整读；读取前一律过名称白名单
//! 守卫（[`ensure_safe_log_name`]），用户可控的文件名进路径之前先挡掉路径穿越。

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use chrono::{DateTime, Local, SecondsFormat};
use log::{Level, LevelFilter, Log, Metadata, Record};

pub mod scrub;

pub use scrub::scrub_log_line;

/// 保留的日志文件个数上限（每次启动清理一次）。
pub const KEEP_FILES: usize = 20;
/// 保留的日志文件总字节上限（50 MB）。
pub const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;

const FILE_PREFIX: &str = "rustrss-";
const FILE_SUFFIX: &str = ".log";
/// 同一秒内最多容忍的启动次数（同秒冲突后缀 `-N` 的上限）。
const MAX_SAME_SECOND_FILES: u32 = 100;

/// 本次启动正在写的日志文件名，由 [`init_with_mirror`] 首次成功时写入，
/// [`list_log_files`] 用它标记 `is_current`。
///
/// `OnceLock` 只认首值：重复 init（全局 logger 本来也只装一次）不会改写
/// 「当前」的归属——首装 logger 的那个文件才是本次会话真正在写的。
static CURRENT: OnceLock<String> = OnceLock::new();

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
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let _ = CURRENT.set(name.to_string());
    }
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

// ---------------------------------------------------------------------------
// 只读访问：列表 / 末尾读取 / 整读（日志查看与导出的数据面）
//
// 非热路径（用户显式打开设置才触发），无性能红线压力；两条硬口径：
// - **名称白名单守卫在打开任何文件之前**（[`ensure_safe_log_name`]）：用户可控的
//   名字拼进路径之前先拦下分隔符 / `..`，杜绝路径穿越；
// - 内容按字节末尾截取后 UTF-8 lossy 解码，任何字节序列都不 panic。
// ---------------------------------------------------------------------------

/// [`list_log_files`] 的一条结果：目录里一个 `rustrss-*.log` 的元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFileInfo {
    pub name: String,
    pub bytes: u64,
    /// mtime 的 epoch 秒；拿不到时为 `None`（排序时沉底）。
    pub modified_at: Option<i64>,
    /// 同秒冲突后缀（基准名 = 0），mtime 打平时的次级排序键。
    pub seq: u32,
    /// 是否本次启动正在写的日志（与 [`CURRENT`] 比对）。
    pub is_current: bool,
}

/// 列出目录下全部本应用日志（仅 `rustrss-*.log`），最新在前。
///
/// 目录不存在 / 不可读时返回空列表（调用方据此显示空态，不算错误）。只读枚举，
/// 不碰文件内容；名字来自 readdir 本身，不存在穿越问题，故只过 [`is_log_file_name`]。
pub fn list_log_files(log_dir: &Path) -> Vec<LogFileInfo> {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return Vec::new();
    };
    let current = CURRENT.get();
    let mut files: Vec<LogFileInfo> = entries
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
            Some(LogFileInfo {
                name: name.to_string(),
                bytes: meta.len(),
                modified_at: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64),
                seq: seq_of(name),
                is_current: current.is_some_and(|c| c == name),
            })
        })
        .collect();
    // 最新在前：mtime 新的在前；打平时同秒后缀序号大的在前（0 是基准名，先于
    // -1、-2…，与 [`prune`] 的旧→新序刚好相反）；再打平按名字倒序，保证完全确定。
    files.sort_by(|a, b| {
        b.modified_at
            .cmp(&a.modified_at)
            .then(b.seq.cmp(&a.seq))
            .then(b.name.cmp(&a.name))
    });
    files
}

/// [`read_log_tail`] 的结果：末尾内容 + 全文件大小 + 是否被截断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTail {
    pub content: String,
    pub total_bytes: u64,
    pub truncated: bool,
}

/// 读取日志文件末尾至多 `max_bytes` 字节（查看用；排障最关心的最新内容恰在尾部）。
///
/// - 起点 `seek(len - max_bytes)`；`truncated = len > max_bytes`；
/// - 截断起点对齐到下一个 `\n` 之后（整行起步，不带半行/半字符）；窗口内一个换行
///   都没有（单行超长）时无处对齐，就原样输出整个窗口——比空视图有用；
/// - 内容 UTF-8 `from_utf8_lossy` 解码，任何字节序列都不 panic；
/// - 当前日志正被 logger 以 append 模式持有，POSIX 语义下并发读安全（最多读到
///   稍旧的字节量，`total_bytes` 为打开时刻的大小）。
pub fn read_log_tail(log_dir: &Path, name: &str, max_bytes: u64) -> Result<LogTail, String> {
    ensure_safe_log_name(name)?;
    let path = log_dir.join(name);
    let mut file =
        File::open(&path).map_err(|e| format!("打开日志文件 {} 失败：{e}", path.display()))?;
    let total_bytes = file
        .metadata()
        .map_err(|e| format!("读取日志文件 {} 元信息失败：{e}", path.display()))?
        .len();

    let truncated = total_bytes > max_bytes;
    let start = if truncated { total_bytes - max_bytes } else { 0 };
    let to_read = total_bytes - start;
    file.seek(SeekFrom::Start(start))
        .map_err(|e| format!("定位日志文件 {} 失败：{e}", path.display()))?;
    let mut buf = Vec::with_capacity(to_read.min(usize::MAX as u64) as usize);
    file.take(to_read)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取日志文件 {} 失败：{e}", path.display()))?;

    // 对齐到下一行行首：丢掉窗口开头那半行（连同可能被砍半的多字节字符）。
    if truncated {
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            buf.drain(..=pos);
        }
    }
    Ok(LogTail {
        content: String::from_utf8_lossy(&buf).into_owned(),
        total_bytes,
        truncated,
    })
}

/// 整读一个日志文件（导出用）。超过 `max_bytes` 返回可读 `Err`——调用方在打开
/// 任何对话框 / 写任何盘之前就能失败（上限由调用方定，导出链路用 16 MB）。
pub fn read_log_file(log_dir: &Path, name: &str, max_bytes: u64) -> Result<String, String> {
    ensure_safe_log_name(name)?;
    let path = log_dir.join(name);
    let file =
        File::open(&path).map_err(|e| format!("打开日志文件 {} 失败：{e}", path.display()))?;
    // 预检：超限就不读，不把几十 MB 拽进内存再拒绝。
    let size = file
        .metadata()
        .map_err(|e| format!("读取日志文件 {} 元信息失败：{e}", path.display()))?
        .len();
    if size > max_bytes {
        return Err(too_large_message(size, max_bytes));
    }
    // 读的时候多要 1 字节：并发追加把文件顶过上限的竞态（TOCTOU）在这里兑住。
    let mut buf = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取日志文件 {} 失败：{e}", path.display()))?;
    if buf.len() as u64 > max_bytes {
        return Err(too_large_message(buf.len() as u64, max_bytes));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn too_large_message(size: u64, max_bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    format!(
        "文件过大（{:.1} MB），上限 {:.1} MB",
        size as f64 / MIB,
        max_bytes as f64 / MIB
    )
}

/// 读取类 API 的名称白名单守卫：只接受 `rustrss-*.log`，且显式拒绝含路径分隔符
/// （`/`、`\`）或 `..` 的名字——在打开任何文件之前把路径穿越挡掉。
///
/// `rustrss-..log` 这种「形态合法但含 `..`」的边界名同样拒绝：宁可错杀一个不可能由
/// 本程序生成的名字，也不给拼接路径留任何可乘之机。
fn ensure_safe_log_name(name: &str) -> Result<(), String> {
    if !is_log_file_name(name) {
        return Err(format!("非法日志文件名：{name:?}（只接受 rustrss-*.log）"));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!("非法日志文件名：{name:?}（含路径分隔符或 ..）"));
    }
    Ok(())
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

    // -----------------------------------------------------------------------
    // 只读访问 API：list_log_files / read_log_tail / read_log_file
    // -----------------------------------------------------------------------

    #[test]
    fn list_sorts_newest_first_and_filters_to_the_whitelist() {
        let dir = tmp_dir("list");
        mk_log(&dir, &log_name(1), 10, BASE_MTIME_SECS + 1);
        mk_log(&dir, &log_name(2), 20, BASE_MTIME_SECS + 2);
        // 同秒冲突对：mtime 打平时按 seq 定序（基准名先于 -1，故 -1 更新）
        mk_log(&dir, "rustrss-20260101-000000.log", 5, BASE_MTIME_SECS);
        mk_log(&dir, "rustrss-20260101-000000-1.log", 7, BASE_MTIME_SECS);
        // 非白名单：一律不出现
        std::fs::write(dir.join("a.txt"), b"txt").unwrap();
        std::fs::write(dir.join("other-app.log"), b"foreign").unwrap();
        std::fs::write(dir.join("rustrss-.log"), b"empty stem").unwrap();
        std::fs::write(dir.join("rustrss-short"), b"no suffix").unwrap();

        let files = list_log_files(&dir);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "rustrss-20260101-000002.log",
                "rustrss-20260101-000001.log",
                "rustrss-20260101-000000-1.log",
                "rustrss-20260101-000000.log",
            ],
            "最新在前，mtime 打平时 seq 大的在前"
        );

        let newest = &files[0];
        assert_eq!(newest.bytes, 20);
        assert_eq!(newest.modified_at, Some(BASE_MTIME_SECS as i64 + 2));
        assert_eq!(newest.seq, 0);
        let same_second = &files[2];
        assert_eq!(same_second.bytes, 7);
        assert_eq!(same_second.seq, 1, "-1 后缀应解析为 seq=1");
        cleanup(&dir);
    }

    #[test]
    fn list_on_missing_or_empty_dir_is_empty() {
        let dir = tmp_dir("list-missing");
        assert!(list_log_files(&dir.join("does-not-exist")).is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        assert!(list_log_files(&dir).is_empty());
        cleanup(&dir);
    }

    /// `is_current` 的语义钉子：恰好等于 `CURRENT` 的那一条为 `true`，其余为
    /// `false`——不依赖哪个并行用例先跑了 init（CURRENT 是进程级全局）。
    #[test]
    fn list_marks_exactly_the_current_file() {
        let dir = tmp_dir("list-current");
        mk_log(&dir, &log_name(1), 10, BASE_MTIME_SECS + 1);
        mk_log(&dir, &log_name(2), 10, BASE_MTIME_SECS + 2);
        let current = CURRENT.get().cloned();

        let files = list_log_files(&dir);
        assert_eq!(files.len(), 2);
        for f in &files {
            assert_eq!(
                f.is_current,
                current.as_deref() == Some(f.name.as_str()),
                "{} 的 is_current 应与 CURRENT={:?} 一致",
                f.name,
                current
            );
        }

        // CURRENT 已被本进程某次 init 写入时，同名文件必须被标为当前（正例）。
        if let Some(name) = current {
            mk_log(&dir, &name, 3, BASE_MTIME_SECS + 3);
            let files = list_log_files(&dir);
            assert!(
                files.iter().any(|f| f.name == name && f.is_current),
                "CURRENT 指向的文件应被标为当前：{files:?}"
            );
        }
        cleanup(&dir);
    }

    /// init 应把本次文件名写入 CURRENT；OnceLock 只认首值，后续 init 不得覆盖
    /// （与全局 logger 只装一次的口径一致）。
    #[test]
    fn init_records_the_current_log_file_name() {
        let dir = tmp_dir("init-current");
        let before = CURRENT.get().cloned();
        // 用 Trace：全局 max_level 只能放大不能缩小（本文件并行用例的约定），
        // 否则会掐死并行的 Debug 级用例。
        let path = init(&dir.join("logs"), LevelFilter::Trace).expect("init 应成功");
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("文件名应可读")
            .to_string();
        match before {
            None => assert_eq!(
                CURRENT.get(),
                Some(&name),
                "首个 init 应写入本次文件名"
            ),
            Some(prev) => assert_eq!(
                CURRENT.get(),
                Some(&prev),
                "CURRENT 首值不被后续 init 覆盖"
            ),
        }
        cleanup(&dir);
    }

    #[test]
    fn read_log_tail_returns_whole_file_when_within_the_limit() {
        let dir = tmp_dir("tail-whole");
        let text = "第一行\nsecond line\n第三行\n";
        let name = "rustrss-20260101-000001.log";
        std::fs::write(dir.join(name), text).unwrap();

        let tail = read_log_tail(&dir, name, 10_000).expect("应成功");
        assert_eq!(tail.content, text, "未超限时原文整返");
        assert_eq!(tail.total_bytes, text.len() as u64);
        assert!(!tail.truncated);
        cleanup(&dir);
    }

    #[test]
    fn read_log_tail_truncates_at_the_next_line_boundary() {
        let dir = tmp_dir("tail-boundary");
        // L00..L09 每行 4 字节，共 40 字节
        let mut text = String::new();
        for i in 0..10 {
            text.push_str(&format!("L{i:02}\n"));
        }
        let name = "rustrss-20260101-000002.log";
        std::fs::write(dir.join(name), &text).unwrap();

        // 起点 40-21=19，落在 L04 行尾的 \n 上 → 丢掉半行后应从 L05 行首开始
        let tail = read_log_tail(&dir, name, 21).expect("应成功");
        assert!(tail.truncated);
        assert_eq!(tail.total_bytes, 40);
        assert_eq!(
            tail.content, "L05\nL06\nL07\nL08\nL09\n",
            "对齐到完整行首，不带半行：{:?}",
            tail.content
        );
        cleanup(&dir);
    }

    #[test]
    fn read_log_tail_is_utf8_lossy_and_never_panics() {
        let dir = tmp_dir("tail-utf8");
        let mut bytes = Vec::new();
        bytes.extend_from_slice("好的开头\n".as_bytes()); // 13 字节
        bytes.extend_from_slice(b"bad \xff\xfe line\n"); // 12 字节，含非法 UTF-8
        bytes.extend_from_slice("尾巴行\n".as_bytes()); // 10 字节
        let name = "rustrss-20260101-000003.log";
        std::fs::write(dir.join(name), &bytes).unwrap();

        // 非法字节在完整行内 → lossy 替换，不 panic
        let tail = read_log_tail(&dir, name, bytes.len() as u64).expect("应成功");
        assert!(!tail.truncated);
        assert!(
            tail.content.contains('\u{FFFD}'),
            "非法字节应替换为 U+FFFD：{:?}",
            tail.content
        );

        // 截断点砍在非法字节所在的行中间 → 对齐后从下一完整行开始，无半字节半字符
        let tail = read_log_tail(&dir, name, 20).expect("应成功");
        assert!(tail.truncated);
        assert_eq!(
            tail.content, "尾巴行\n",
            "半行连同被砍的非法字节一起被丢掉：{:?}",
            tail.content
        );
        cleanup(&dir);
    }

    /// 窗口内一个换行都没有（单行超长）时无处对齐：原样输出整个窗口（lossy）
    /// ——比空视图有用，且仍然不会 panic。
    #[test]
    fn read_log_tail_without_any_newline_serves_the_raw_window() {
        let dir = tmp_dir("tail-no-newline");
        let name = "rustrss-20260101-000004.log";
        std::fs::write(dir.join(name), vec![b'a'; 100]).unwrap();

        let tail = read_log_tail(&dir, name, 30).expect("应成功");
        assert!(tail.truncated);
        assert_eq!(tail.total_bytes, 100);
        assert_eq!(tail.content.len(), 30);
        assert!(tail.content.chars().all(|c| c == 'a'));
        cleanup(&dir);
    }

    /// 名称白名单守卫：`../x.log`（穿越）、`a.txt`（非日志名）、含分隔符、
    /// 形态合法但含 `..` 的边界名（`rustrss-..log` / `rustrss-a..log`）一律拒绝，
    /// 且在打开任何文件之前就拒绝（目录里什么都没建也能测）。
    #[test]
    fn read_apis_reject_names_outside_the_whitelist() {
        let dir = tmp_dir("guard");
        let bad = [
            "../x.log",         // 穿越上级
            "rustrss-x/y.log",  // 路径分隔符
            "rustrss-x\\y.log", // Windows 分隔符
            "rustrss-a..log",   // 形态合法但含 ..
            "rustrss-..log",    // 边界：前后缀形态合法，仍因 .. 拒绝
            "a.txt",            // 非日志名
            "rustrss-.log",     // 前后缀之间为空
            "",                 // 空名
        ];
        for name in bad {
            let err = read_log_tail(&dir, name, 100).expect_err(name);
            assert!(err.contains("非法日志文件名"), "{name:?} → {err}");
            let err = read_log_file(&dir, name, 100).expect_err(name);
            assert!(err.contains("非法日志文件名"), "{name:?} → {err}");
        }
        cleanup(&dir);
    }

    #[test]
    fn read_apis_report_readable_errors_for_missing_files() {
        let dir = tmp_dir("read-missing");
        let missing = "rustrss-20260101-000009.log";
        let err = read_log_tail(&dir, missing, 100).expect_err("文件不存在应 Err");
        assert!(err.contains("打开日志文件"), "{err}");
        let err = read_log_file(&dir, missing, 100).expect_err("文件不存在应 Err");
        assert!(err.contains("打开日志文件"), "{err}");
        cleanup(&dir);
    }

    #[test]
    fn read_log_file_over_limit_returns_readable_err() {
        let dir = tmp_dir("export-limit");
        let name = "rustrss-20260101-000005.log";
        std::fs::write(dir.join(name), vec![b'x'; 1000]).unwrap();

        let err = read_log_file(&dir, name, 100).expect_err("超限应 Err");
        assert!(err.contains("文件过大"), "{err}");
        assert!(err.contains("上限"), "{err}");

        // 恰好等于上限：不超限，原文整返
        let ok = read_log_file(&dir, name, 1000).expect("等于上限应成功");
        assert_eq!(ok.len(), 1000);
        cleanup(&dir);
    }
}
