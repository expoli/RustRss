//! 备份 / 恢复：在线快照导出、备份校验、暂存与启动时替换。
//!
//! 两条路径的机制都与 WAL 边车强相关，改动前先读这段：
//!
//! - **导出**走 rusqlite 的 backup API（`sqlite3_backup_*`）而不是复制文件：复制只拿到主库
//!   那一刻的字节，`-wal` 里未 checkpoint 的已提交事务会丢。backup API 在源连接上开一致性读快照，
//!   导出期间库可继续读写（不必先 checkpoint），产物是独立干净的库文件（不依赖 `-wal`/`-shm`）。
//! - **恢复**不放在退出时替换（MCP 的第二条连接可能还活着；Windows 上打开的文件也不能 rename），
//!   而是把备份暂存成 `pending-restore.sqlite`，由 [`apply_pending_restore`] 在**下一次启动、
//!   任何连接打开之前**完成替换——那时没有边车、没有锁，rename / delete 都是原子的。
//!
//! # 不变量（别改顺序）
//!
//! 替换路径上「删除 `-wal`/`-shm`」**严格先于** `rename(pending → db)`，且只在 pending 分支里做：
//!
//! - 两种顺序在正常路径下等价，差别只在两步之间崩溃时的残留：先 rename 后删边车会留下 stale WAL，
//!   而那时 pending 已被消费，下次启动本函数直接 no-op，随后 `Store::open` 会把旧库的 WAL 帧回
//!   放进新库——静默数据损坏（「主库是备份内容、WAL 是旧内容」这种混合态无法自愈，因为 WAL 里
//!   的帧自带盐值与校验，SQLite 只会照单回放或报错）。
//! - 先删边车则「pending 已消费 + 主库已替换 + 边车残留」这一崩溃态在构造上不可达：崩溃只会落在
//!   「边车已删、pending 还在」这一侧，下次启动继续把替换做完。
//! - 反过来，「入口处无条件删边车」也不行：正常启动（无 pending）时边车就是**未 checkpoint 的
//!   已提交事务**，删掉等于丢数据。边车归 `Store::open` 正常回放，所以删除只能在 pending 分支内、
//!   且旧库已挪走（或本就不存在）之后做。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use rusqlite::backup::Backup;
use rusqlite::{Connection, OpenFlags};

use super::{Result, Store, StoreError};

/// 待恢复库的暂存名（与主库同目录）。
pub const PENDING_FILE: &str = "pending-restore.sqlite";
/// 暂存时的中转名：先整体写它、再原子 rename 成 [`PENDING_FILE`]，
/// 这样「复制到一半进程被杀」不会留下半个库被当成完整备份拿去替换。
const PENDING_TMP: &str = "pending-restore.sqlite.tmp";
/// 现库保底回滚副本的中缀：`rustrss.sqlite.bak-20260921-180302`
const BAK_MARKER: &str = ".bak-";
/// 备份产物名前缀（时间戳到秒）
const EXPORT_PREFIX: &str = "RustRss-backup-";

fn io_err(op: &str, path: &Path, e: std::io::Error) -> StoreError {
    StoreError::Io(format!("{op} {} 失败: {e}", path.display()))
}

/// 时间戳后缀（`YYYYMMDD-HHMMSS`，**UTC**）。
///
/// 用 UTC 不用本地时间：文件名要按字典序当时间序用（[`new_bak_path`]/`bak_candidates` 靠它
/// 认「最新一份 bak」），而本地时间在夏令时回拨时会倒退。同秒重复则按覆盖处理。
fn stamp() -> String {
    Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

/// 新的 bak 路径（当前时间戳）。
fn new_bak_path(db_path: &Path) -> PathBuf {
    bak_path(db_path, &stamp())
}

/// 主库所在目录（无父目录时按当前目录算，与 `Store::open` 的建目录逻辑同一口径）。
///
/// 这是 [`stage_restore`] 的 `data_dir` 参数该传的东西——`db_path.parent()` 在相对路径
/// （`RUSTSS_DB=rustrss.sqlite`）下会得到空路径，和这里不是一回事，所以统一走本函数。
pub fn data_dir_of(db_path: &Path) -> PathBuf {
    match db_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// 暂存文件路径（**入参是主库路径，不是数据目录**；数据目录可用 [`data_dir_of`] 推）。
/// 恢复命令与 [`apply_pending_restore`] 都从这里取，避免两处各写一遍名字。
pub fn pending_path(db_path: &Path) -> PathBuf {
    data_dir_of(db_path).join(PENDING_FILE)
}

/// 一份 bak 的路径（`<主库名>.bak-<时间戳>`）。带时间戳参数是为了让测试能构造「更旧的 bak」。
pub fn bak_path(db_path: &Path, stamp: &str) -> PathBuf {
    let mut name = db_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    name.push_str(BAK_MARKER);
    name.push_str(stamp);
    data_dir_of(db_path).join(name)
}

/// SQLite 边车路径：`-wal` / `-shm` 是**追加**在主库文件名后面的，不能用 `with_extension`
/// （那会把 `rustrss.sqlite` 变成 `rustrss-wal`，删错文件）。
fn sidecar(db_path: &Path, suffix: &str) -> PathBuf {
    let mut name = db_path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// 现存 bak 列表（按文件名升序 = 时间戳升序，`YYYYMMDD-HHMMSS` 定宽所以字典序即时间序）。
fn bak_candidates(db_path: &Path) -> Result<Vec<PathBuf>> {
    let dir = data_dir_of(db_path);
    let prefix = format!(
        "{}{BAK_MARKER}",
        db_path.file_name().unwrap_or_default().to_string_lossy()
    );
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        // 目录不存在 = 还没跑过应用：不是错误，按「没有 bak」处理
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err("读取目录", &dir, e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io_err("读取目录", &dir, e))?;
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        if is_file && entry.file_name().to_string_lossy().starts_with(&prefix) {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// 只保留 `keep`，其余 bak 尽力删除。
///
/// 删不掉不让替换失败：多留一份 bak 只是占点空间，而中止替换会把用户卡在
/// 「库在 bak 里、pending 还没换上」的中间态。core 不写日志，失败静默，下次替换会再试。
fn prune_baks(db_path: &Path, keep: &Path) {
    for path in bak_candidates(db_path).unwrap_or_default() {
        if path != keep {
            let _ = fs::remove_file(&path);
        }
    }
}

/// 删除 stale `-wal` / `-shm`（不存在视为已删）。删不掉要报错——宁可中止替换，
/// 也不能让旧边车和新库拼在一起（见模块头部的不变量）。
fn remove_sidecars(db_path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let path = sidecar(db_path, suffix);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("删除 WAL 边车", &path, e)),
        }
    }
    Ok(())
}

/// 在线快照导出：把当前库（含 WAL 里未 checkpoint 的已提交事务）完整写成一个独立库文件。
///
/// 返回产物路径；`dest_dir` 不存在会创建。
pub fn export_backup(store: &Store, dest_dir: &Path) -> Result<PathBuf> {
    if !dest_dir.exists() {
        fs::create_dir_all(dest_dir).map_err(|e| io_err("创建目录", dest_dir, e))?;
    }
    let dest = dest_dir.join(format!("{EXPORT_PREFIX}{}.sqlite", stamp()));
    if dest.exists() {
        // 时间戳到秒：同一秒内第二次导出会落在同一路径。先删旧产物，
        // 免得中途失败时留下一个「半新半旧」的文件被用户当备份带走。
        fs::remove_file(&dest).map_err(|e| io_err("覆盖旧备份", &dest, e))?;
    }

    let mut out = Connection::open(&dest)?;
    {
        // 快照的一致性由 backup API 自己在源连接上开读事务保证：导出期间并发的写入
        // 照常提交，但不会被切成两半塞进产物。
        let backup = Backup::new(&store.conn, &mut out)?;
        backup.run_to_completion(256, Duration::from_millis(0), None)?;
    }
    // 归一化回普通 journal 模式：源库是 WAL，「WAL」标志会跟着第 1 页一起复制进产物。
    // 产物是要交给用户（可能换机器、拷到只读介质）的，保持 WAL 标志会让任何读它的人
    // 在被打开目录里生成 `-wal`/`-shm` 两个伴生文件，只读目录下甚至会直接打不开。
    let _: String = out.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
    // 句柄关掉（drop 即 sqlite3_close）：产物不留下 -wal/-shm，也不占住用户的库。
    drop(out);
    Ok(dest)
}

/// 校验用户选定的备份文件：能只读打开、`user_version` 落在 `(0, current_version]` 内。
///
/// 只读打开是刻意的：校验阶段绝不能动用户的库，也不该给候选备份文件写一个字节。
pub fn validate_backup(path: &Path, current_version: i64) -> Result<()> {
    if !path.exists() {
        return Err(StoreError::Invalid(format!(
            "备份文件不存在: {}",
            path.display()
        )));
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| StoreError::Invalid(format!("无法只读打开 {}: {e}", path.display())))?;
    // open 是惰性的：文件不是 SQLite 库要到真读页时才报错，所以这里显式读一次
    // user_version —— 它在库头（第 1 页）里，读它同时验了文件头与可读性。
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| {
            StoreError::Invalid(format!("{} 不是可读的 SQLite 数据库: {e}", path.display()))
        })?;
    if version <= 0 {
        // 0 字节文件会被 SQLite 当成「合法的空库」（user_version = 0），不拦的话
        // 用户选错文件就会用一个空库覆盖现库。本应用的库跑过迁移，版本必 ≥ 1。
        return Err(StoreError::Invalid(format!(
            "{} 不是 RustRss 的备份：schema 版本为 {version}，像空库或别的应用的库",
            path.display()
        )));
    }
    if version > current_version {
        return Err(StoreError::Invalid(format!(
            "备份的 schema 版本 {version} 高于当前程序支持的 {current_version}：请先升级 RustRss 再恢复",
        )));
    }
    Ok(())
}

/// 暂存恢复：把用户选定的备份复制成主库同目录的 `pending-restore.sqlite`，重启后生效。
///
/// `data_dir` = 主库所在目录（[`data_dir_of`]）；调用方应先 [`validate_backup`]
/// （本函数不校验内容，只保证「要么完整落地、要么什么都不留下」）。
pub fn stage_restore(backup: &Path, data_dir: &Path) -> Result<()> {
    if !data_dir.exists() {
        fs::create_dir_all(data_dir).map_err(|e| io_err("创建目录", data_dir, e))?;
    }
    let tmp = data_dir.join(PENDING_TMP);
    fs::copy(backup, &tmp).map_err(|e| io_err("复制备份", backup, e))?;
    let pending = data_dir.join(PENDING_FILE);
    if pending.exists() {
        // 重复暂存（Windows 的 rename 不覆盖已存在文件）
        fs::remove_file(&pending).map_err(|e| io_err("覆盖旧暂存", &pending, e))?;
    }
    // 先写中转名再 rename：半份文件绝不会以 pending 的名字出现（否则重启时拿它覆盖现库）。
    fs::rename(&tmp, &pending).map_err(|e| io_err("暂存恢复文件", &tmp, e))?;
    Ok(())
}

/// 启动时应用暂存的恢复。**必须在任何连接（`Store` / MCP）打开之前调用**。
///
/// 返回是否真的动了库文件（`false` = 无事可做，正常启动路径）。全程纯文件操作，不开连接：
/// 只有此时 rename / delete 才既不被打开的文件挡住（Windows），也不会有边车写入者。
pub fn apply_pending_restore(db_path: &Path) -> Result<bool> {
    let pending = pending_path(db_path);

    if !pending.exists() {
        // 无 pending：唯一要收的残局是「上次已把现库挪成 bak、还没换上 pending」——
        // 只在主库缺失时才有意义（主库还在就说明替换没开始，或已完成）。
        // 注意这里**绝不碰** -wal/-shm：正常启动时它们是未 checkpoint 的已提交事务，
        // 删掉就是丢数据（`Store::open` 会正常回放）。
        if !db_path.exists() {
            if let Some(bak) = bak_candidates(db_path)?.last().cloned() {
                fs::rename(&bak, db_path).map_err(|e| io_err("从备份回滚", &bak, e))?;
                return Ok(true);
            }
        }
        return Ok(false);
    }

    if db_path.exists() {
        let bak = new_bak_path(db_path);
        if bak.exists() {
            // 同一秒内重复替换会撞名（时间戳到秒）；Windows 的 rename 不覆盖已存在文件
            fs::remove_file(&bak).map_err(|e| io_err("删除旧备份", &bak, e))?;
        }
        // 先产生新 bak 再清理旧 bak：顺序反过来（先删旧）的话，两步之间崩溃就一份可回滚的库都没了。
        fs::rename(db_path, &bak).map_err(|e| io_err("备份现库", db_path, e))?;
        prune_baks(db_path, &bak);
    }
    // 主库缺失不报错：上次崩溃在两次 rename 之间，替换意图优先（pending 才是用户要的结果）。
    // 此刻旧库要么已挪成 bak、要么本就不存在，边车只属于旧库，删除安全。

    // 不变量：边车删除严格先于下一步 rename（理由见模块头部注释）。
    remove_sidecars(db_path)?;
    fs::rename(&pending, db_path).map_err(|e| io_err("应用恢复", &pending, e))?;
    Ok(true)
}
