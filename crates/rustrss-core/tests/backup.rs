//! 备份 / 恢复测试（对应 PRD 验收 1–4）。
//!
//! 重点验四件事：
//! 1. 在线快照导出的产物是独立干净的库文件，且与源库逐字段一致（含已读 / 星标 / 稍后读 / 设置）；
//! 2. 并发（第二个连接活着、甚至握着写事务）时导出仍是一致快照：看得见已提交、看不见未提交；
//! 3. 校验把关：非库文件、空文件、版本超前一律拒绝（绝不落地到现库）；
//! 4. 启动替换的崩溃容忍：pending 优先、bak 回滚、只留一份 bak、幂等，以及
//!    **边车删除严格先于 rename(pending→db)** 与「无 pending 绝不触碰 -wal/-shm」。

use std::fs;
use std::path::{Path, PathBuf};

use rustrss_core::backup::{
    apply_pending_restore, bak_path, data_dir_of, export_backup, pending_path, stage_restore,
    validate_backup,
};
use rustrss_core::{Entry, EntryQuery, IdOrigin, Store};

/// 每个测试一个独立目录（pid + 纳秒），结束删掉：不碰真实数据目录。
fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustrss-backup-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("测试目录应能创建");
    dir
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

fn mk_entry(stable_id: &str, title: &str) -> Entry {
    Entry {
        stable_id: stable_id.to_string(),
        id_origin: IdOrigin::SourceData,
        source_id: stable_id.to_string(),
        title: title.to_string(),
        url: Some(format!("https://example.com/{stable_id}")),
        author: None,
        published: None,
        updated: None,
        summary: Some(format!("{title} 的摘要")),
        content_html: Some(format!("<p>{title}</p>")),
        content_text: Some(title.to_string()),
        categories: Vec::new(),
    }
}

/// SQLite 边车（`-wal` / `-shm` 追加在主库名后面）
fn sidecar(db_path: &Path, suffix: &str) -> PathBuf {
    let mut name = db_path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// 条目状态快照（标题 + 三个标记位），用于往返一致比对
fn entry_state(store: &Store) -> Vec<(String, bool, bool, bool)> {
    let mut rows: Vec<_> = store
        .list_entries(&EntryQuery::default())
        .expect("列条目应成功")
        .into_iter()
        .map(|r| (r.title, r.read, r.starred, r.read_later))
        .collect();
    rows.sort();
    rows
}

fn feed_state(store: &Store) -> Vec<(String, String)> {
    let mut rows: Vec<_> = store
        .list_feeds()
        .expect("列订阅应成功")
        .into_iter()
        .map(|f| (f.url, f.title))
        .collect();
    rows.sort();
    rows
}

/// 往库里放一份可辨识的数据，返回两个条目 id。
///
/// `slug` 只用在 URL 上（保持 ASCII：`rsshub::normalize_rsshub_url` 目前按字节切前 9 字节
/// 判断 scheme，非 ASCII URL 会 panic——既有问题，与本任务无关，故测试绕开）；
/// `label` 用在中文字段上，便于断言「恢复的是哪一份库」。
fn seed(store: &Store, slug: &str, label: &str) -> (i64, i64, i64) {
    let feed = store
        .add_feed(&format!("https://{slug}.example/feed.xml"), Some(label))
        .expect("加源应成功");
    store
        .upsert_entries(
            feed,
            &[
                mk_entry("a", &format!("{label} A")),
                mk_entry("b", &format!("{label} B")),
            ],
        )
        .expect("入库应成功");
    let rows = store.list_entries(&EntryQuery::default()).expect("列条目应成功");
    let id_of = |title: &str| {
        rows.iter()
            .find(|r| r.title == title)
            .map(|r| r.id)
            .expect("刚入库的条目应能查到")
    };
    (feed, id_of(&format!("{label} A")), id_of(&format!("{label} B")))
}

// ---------------------------------------------------------------- 1. 导出往返

#[test]
fn export_roundtrip_matches_source_with_a_second_connection_open() {
    let dir = tmp_dir("roundtrip");
    let db = dir.join("rustrss.sqlite");
    let store = Store::open(&db).expect("打开应成功");
    let (_feed, a, b) = seed(&store, "feed", "源");
    store.set_read(&[a], true).unwrap();
    store.set_starred(&[b], true).unwrap();
    store.set_read_later(&[a], true).unwrap();
    store.set_setting("ui.locale", "en").unwrap();

    // 第二个连接（对应 MCP / 另一个消费者的场景）先读一遍，导出时它仍然活着
    let second = Store::open(&db).expect("第二个连接应能打开");
    assert_eq!(second.list_feeds().unwrap().len(), 1);

    // 第二个连接活着时第一连接继续写：导出必须看到最新已提交状态
    let feed2 = store.add_feed("https://extra.example/feed.xml", Some("追加源")).unwrap();
    store.upsert_entries(feed2, &[mk_entry("c", "追加 C")]).unwrap();

    let backup = export_backup(&store, &dir.join("out")).expect("导出应成功");

    // 独立干净的库文件：命名带时间戳、旁边没有边车
    let name = backup.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("RustRss-backup-"), "产物命名: {name}");
    assert!(name.ends_with(".sqlite"), "产物命名: {name}");
    assert!(!sidecar(&backup, "-wal").exists(), "导出产物不应带 -wal");
    assert!(!sidecar(&backup, "-shm").exists(), "导出产物不应带 -shm");

    // 产物是「交出去」的文件：普通 journal 模式，第三方只读连接读它不会在备份目录里
    // 生成 -wal/-shm（只读介质上更是必须），且内容完整。
    {
        let ro = rusqlite::Connection::open_with_flags(
            &backup,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("备份应能只读打开");
        let mode: String = ro.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "delete", "产物应是普通 journal 模式（WAL 标志会被读它的人生成边车）");
        let n: i64 = ro
            .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
            .expect("只读连接应能查内容");
        assert_eq!(n, 3);
    }
    assert!(!sidecar(&backup, "-wal").exists(), "只读读完也不该冒出新 -wal");
    assert!(!sidecar(&backup, "-shm").exists(), "只读读完也不该冒出新 -shm");

    // 新 Store 直接打开备份：逐字段一致
    let restored = Store::open(&backup).expect("备份应能被 Store 打开");
    assert_eq!(feed_state(&restored), feed_state(&store));
    assert_eq!(entry_state(&restored), entry_state(&store), "含已读/星标/稍后读");
    assert_eq!(restored.all_settings().unwrap(), store.all_settings().unwrap());
    assert_eq!(restored.schema_version().unwrap(), store.schema_version().unwrap());
    assert_eq!(restored.starred_total().unwrap(), 1);
    assert_eq!(restored.read_later_total().unwrap(), 1);
    assert_eq!(restored.entry_count().unwrap(), 3);

    drop(second);
    drop(restored);
    cleanup(&dir);
}

#[test]
fn export_ignores_uncommitted_writes_from_the_second_connection() {
    let dir = tmp_dir("uncommitted");
    let db = dir.join("rustrss.sqlite");
    let store = Store::open(&db).unwrap();
    seed(&store, "feed", "源");
    store.set_setting("ui.locale", "zh-CN").unwrap();

    // 第二个连接握着写事务（未提交）：导出既不该被它卡住，也不该看到它的改动
    let writer = rusqlite::Connection::open(&db).expect("第二个连接应能打开");
    writer
        .execute_batch("BEGIN IMMEDIATE")
        .expect("应能拿到写事务");
    writer
        .execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('test.uncommitted', 'x', 0)",
            [],
        )
        .expect("未提交写应成功");

    let backup = export_backup(&store, &dir.join("out")).expect("导出不应被写事务卡住");

    let restored = Store::open(&backup).unwrap();
    assert_eq!(
        restored.setting("test.uncommitted").unwrap(),
        None,
        "未提交的写不能出现在快照里"
    );
    assert_eq!(restored.setting("ui.locale").unwrap().as_deref(), Some("zh-CN"));
    assert_eq!(restored.entry_count().unwrap(), 2, "已提交数据应完整");

    writer.execute_batch("ROLLBACK").unwrap();
    drop(writer);
    drop(restored);
    cleanup(&dir);
}

// ---------------------------------------------------------------- 2. 校验

#[test]
fn validate_accepts_own_backup_and_rejects_garbage() {
    let dir = tmp_dir("validate");
    let db = dir.join("rustrss.sqlite");
    let store = Store::open(&db).unwrap();
    seed(&store, "feed", "源");
    let current = store.schema_version().unwrap();
    let backup = export_backup(&store, &dir.join("out")).unwrap();

    validate_backup(&backup, current).expect("自家备份应通过校验");

    // 情形一：文本文件（open 是惰性的，读 user_version 才暴露「不是数据库」）
    let text = dir.join("not-a-db.sqlite");
    fs::write(&text, "这不是数据库，只是一段文本\n").unwrap();
    let err = validate_backup(&text, current).unwrap_err().to_string();
    assert!(
        err.contains("不是可读的 SQLite 数据库"),
        "错误要指明不是库文件，实际: {err}"
    );

    // 情形二：版本超前（备份来自更新版本的程序）
    let future = dir.join("future.sqlite");
    fs::copy(&backup, &future).unwrap();
    {
        let conn = rusqlite::Connection::open(&future).unwrap();
        conn.pragma_update(None, "user_version", current + 1).unwrap();
    }
    let err = validate_backup(&future, current).unwrap_err().to_string();
    assert!(
        err.contains("高于当前程序支持"),
        "错误要说明版本超前，实际: {err}"
    );

    // 情形三：0 字节文件会被 SQLite 当成「合法空库」（user_version = 0），必须显式拦下
    let empty = dir.join("empty.sqlite");
    fs::write(&empty, b"").unwrap();
    let err = validate_backup(&empty, current).unwrap_err().to_string();
    assert!(
        err.contains("不是 RustRss 的备份"),
        "空库也要拒绝，实际: {err}"
    );

    // 不存在的文件
    assert!(validate_backup(&dir.join("nope.sqlite"), current).is_err());

    // 校验失败绝不该动源库：现库仍在、条数不变
    assert_eq!(store.entry_count().unwrap(), 2);
    cleanup(&dir);
}

// ---------------------------------------------------------------- 3. 暂存 → 启动替换

#[test]
fn stage_then_apply_replaces_db_clears_sidecars_and_keeps_one_bak() {
    let dir_a = tmp_dir("apply-current");
    let dir_b = tmp_dir("apply-backup");

    // A：当前库（恢复后应被换掉的数据）
    let db_a = dir_a.join("rustrss.sqlite");
    let current_version = {
        let store = Store::open(&db_a).unwrap();
        seed(&store, "current", "当前");
        store.set_setting("ui.locale", "zh-CN").unwrap();
        let v = store.schema_version().unwrap();
        drop(store); // 模拟进程退出：SQLite 自己做 checkpoint，边车消失
        v
    };

    // B：用户要恢复的备份文件
    let backup = {
        let store = Store::open(dir_b.join("rustrss.sqlite")).unwrap();
        seed(&store, "backup", "备份");
        store.set_setting("ui.locale", "en").unwrap();
        export_backup(&store, &dir_b.join("out")).unwrap()
    };
    validate_backup(&backup, current_version).expect("备份应通过校验");

    // 前置脏状态：一份更旧的 bak + 非空 stale 边车（模拟上次替换留下的残骸）
    let old_bak = bak_path(&db_a, "20200101-000000");
    fs::write(&old_bak, b"old bak").unwrap();
    fs::write(sidecar(&db_a, "-wal"), b"stale wal bytes").unwrap();
    fs::write(sidecar(&db_a, "-shm"), b"stale shm bytes").unwrap();

    assert_eq!(data_dir_of(&db_a), dir_a, "data_dir_of 应给出主库所在目录");
    stage_restore(&backup, &data_dir_of(&db_a)).expect("暂存应成功");
    assert!(pending_path(&db_a).exists(), "暂存文件应落地");
    assert!(
        !dir_a.join("pending-restore.sqlite.tmp").exists(),
        "中转文件不该残留（写一半崩溃不能留下半份 pending）"
    );

    assert!(apply_pending_restore(&db_a).unwrap(), "有 pending 时应完成替换");
    assert!(!pending_path(&db_a).exists(), "pending 应被消费");
    assert!(
        !sidecar(&db_a, "-wal").exists(),
        "stale -wal 必须删除，否则会被当成恢复库的尾巴回放"
    );
    assert!(!sidecar(&db_a, "-shm").exists(), "stale -shm 必须删除");
    assert!(!old_bak.exists(), "更旧的 bak 应被清理");

    let baks: Vec<PathBuf> = fs::read_dir(&dir_a)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().contains(".bak-"))
        .collect();
    assert_eq!(baks.len(), 1, "只保留最近 1 份 bak，实际: {baks:?}");

    // 库内容 = 备份内容（stale WAL 若残留，这里会报错或读到脏帧）
    let restored = Store::open(&db_a).expect("替换后的库应能正常打开");
    assert_eq!(feed_state(&restored), feed_state(&Store::open(&backup).unwrap()));
    assert_eq!(restored.entry_count().unwrap(), 2);
    assert_eq!(restored.setting("ui.locale").unwrap().as_deref(), Some("en"));
    drop(restored);

    // 保底回滚：留下的那份 bak 是替换前的现库
    let rolled = Store::open(&baks[0]).expect("bak 应能打开");
    assert_eq!(rolled.setting("ui.locale").unwrap().as_deref(), Some("zh-CN"));
    assert!(feed_state(&rolled)[0].1.contains("当前"), "bak 内容应是替换前的库");
    drop(rolled);

    // 幂等：再 apply 一次无事可做，也不该把 bak 恢复回去
    assert!(!apply_pending_restore(&db_a).unwrap(), "无 pending 时应返回 false");
    let again = Store::open(&db_a).unwrap();
    assert_eq!(again.setting("ui.locale").unwrap().as_deref(), Some("en"));

    drop(again);
    cleanup(&dir_a);
    cleanup(&dir_b);
}

#[test]
fn stage_overwrites_previous_pending() {
    let dir = tmp_dir("stage-twice");
    let src = tmp_dir("stage-twice-src");
    let db = dir.join("rustrss.sqlite"); // 只用来定位 pending 路径：本用例的现库还没建
    let backup = {
        let store = Store::open(src.join("rustrss.sqlite")).unwrap();
        seed(&store, "backup", "备份");
        export_backup(&store, &src.join("out")).unwrap()
    };
    stage_restore(&backup, &dir).unwrap();
    fs::write(pending_path(&db), "被上一次暂存写坏的内容").unwrap();
    stage_restore(&backup, &dir).unwrap();
    let bytes = fs::read(pending_path(&db)).unwrap();
    assert_eq!(bytes, fs::read(&backup).unwrap(), "重复暂存应整体覆盖");
    cleanup(&dir);
    cleanup(&src);
}

// ---------------------------------------------------------------- 4. 崩溃态

#[test]
fn crash_state_a_db_missing_with_pending_completes_the_replace() {
    // 崩溃点：现库已挪走（或本来就没开始），pending 还没 rename —— db 缺失 + pending 在
    let dir = tmp_dir("crash-a");
    let src = tmp_dir("crash-a-src");
    let backup = {
        let store = Store::open(src.join("rustrss.sqlite")).unwrap();
        seed(&store, "backup", "备份");
        export_backup(&store, &src.join("out")).unwrap()
    };
    stage_restore(&backup, &dir).unwrap();
    let db = dir.join("rustrss.sqlite");
    assert!(!db.exists(), "前置：主库缺失");

    assert!(apply_pending_restore(&db).unwrap(), "替换意图优先，应完成替换");
    assert!(!pending_path(&db).exists());
    let restored = Store::open(&db).unwrap();
    assert_eq!(restored.entry_count().unwrap(), 2);
    assert!(feed_state(&restored)[0].1.contains("备份"));
    drop(restored);
    cleanup(&dir);
    cleanup(&src);
}

#[test]
fn crash_state_b_db_missing_with_bak_and_no_pending_rolls_back() {
    // 崩溃点：pending 已不在（被消费或手工删掉），主库在 bak 里 —— 回滚 bak
    let dir = tmp_dir("crash-b");
    let db = dir.join("rustrss.sqlite");
    {
        let store = Store::open(&db).unwrap();
        seed(&store, "rollback", "回滚源");
    }
    let bak = bak_path(&db, "20200101-000000");
    fs::rename(&db, &bak).unwrap();
    assert!(!db.exists());

    assert!(apply_pending_restore(&db).unwrap(), "应从 bak 回滚");
    assert!(db.exists(), "主库应被 bak 顶回来");
    assert!(!bak.exists(), "bak 已被消费");
    let rolled = Store::open(&db).unwrap();
    assert_eq!(rolled.entry_count().unwrap(), 2);
    assert!(feed_state(&rolled)[0].1.contains("回滚源"));
    drop(rolled);
    cleanup(&dir);
}

#[test]
fn crash_state_c_stale_sidecars_with_pending_are_cleared_before_replace() {
    // 崩溃点：主库已挪 bak、边车已删、pending 还没 rename —— 构造 db 缺失 + **非空 stale 边车** + pending 在。
    // 这是最危险的一态：若 apply 不删边车（或删除顺序晚于 rename），旧 WAL 帧会被回放进恢复库。
    let dir = tmp_dir("crash-c");
    let src = tmp_dir("crash-c-src");
    let backup = {
        let store = Store::open(src.join("rustrss.sqlite")).unwrap();
        seed(&store, "backup", "备份");
        export_backup(&store, &src.join("out")).unwrap()
    };
    let db = dir.join("rustrss.sqlite");
    stage_restore(&backup, &dir).unwrap();
    fs::write(sidecar(&db, "-wal"), vec![0xABu8; 4096]).unwrap();
    fs::write(sidecar(&db, "-shm"), vec![0xCDu8; 32768]).unwrap();

    assert!(apply_pending_restore(&db).unwrap(), "应完成替换");
    assert!(db.exists());
    assert!(!sidecar(&db, "-wal").exists(), "stale -wal 必须不存在");
    assert!(!sidecar(&db, "-shm").exists(), "stale -shm 必须不存在");

    // 能干净打开且内容 = pending（stale 帧若参与回放，这里读到的是垃圾或直接报错）
    let restored = Store::open(&db).expect("替换后的库应干净可读");
    assert_eq!(restored.entry_count().unwrap(), 2);
    assert!(feed_state(&restored)[0].1.contains("备份"));
    drop(restored);
    cleanup(&dir);
    cleanup(&src);
}

#[test]
fn sidecar_removal_failure_aborts_before_replacing_the_db() {
    // 不变量方向性测试：边车删除**必须先于** rename(pending→db)。
    // 让删除必然失败（把 -wal 造成目录），若实现是「先 rename 再删边车」，此刻主库已被替换、
    // pending 已消失 —— 正是设计要排除的「替换完成 + 边车残留」混合态；正确实现应中止且状态不变。
    let dir = tmp_dir("sidecar-fail");
    let src = tmp_dir("sidecar-fail-src");
    let backup = {
        let store = Store::open(src.join("rustrss.sqlite")).unwrap();
        seed(&store, "backup", "备份");
        export_backup(&store, &src.join("out")).unwrap()
    };
    let db = dir.join("rustrss.sqlite");
    stage_restore(&backup, &dir).unwrap();
    fs::create_dir(sidecar(&db, "-wal")).expect("用目录占位，让 remove_file 必然失败");

    let err = apply_pending_restore(&db).unwrap_err().to_string();
    assert!(err.contains("删除 WAL 边车"), "应报删除边车失败，实际: {err}");
    assert!(!db.exists(), "中止时主库不该被换上（否则会与残留边车混在一起）");
    assert!(pending_path(&db).exists(), "pending 必须留着，下次启动还能重试");

    // 障碍清除后重试：替换照常完成
    fs::remove_dir(sidecar(&db, "-wal")).unwrap();
    assert!(apply_pending_restore(&db).unwrap());
    let restored = Store::open(&db).unwrap();
    assert_eq!(restored.entry_count().unwrap(), 2);
    drop(restored);
    cleanup(&dir);
    cleanup(&src);
}

// ---------------------------------------------------------------- 5. 无 pending 的正常启动

#[test]
fn apply_without_pending_never_touches_db_or_wal() {
    let dir = tmp_dir("no-pending");
    let db = dir.join("rustrss.sqlite");
    let store = Store::open(&db).unwrap();
    seed(&store, "normal", "正常");

    // 连接不关：此刻 -wal 里就是「已提交但未 checkpoint」的事务（正常启动要回放的东西）
    let wal = sidecar(&db, "-wal");
    let shm = sidecar(&db, "-shm");
    assert!(wal.exists(), "写过后应存在 -wal");
    let wal_before = fs::read(&wal).unwrap();
    assert!(!wal_before.is_empty(), "WAL 里应有未 checkpoint 的帧");

    assert!(
        !apply_pending_restore(&db).unwrap(),
        "无 pending 且主库在 → 无事可做"
    );
    assert_eq!(
        fs::read(&wal).unwrap(),
        wal_before,
        "正常启动路径绝不能动 -wal（那是未 checkpoint 的已提交事务）"
    );
    assert!(shm.exists(), "-shm 同样不该被碰");

    // 数据仍在（未 checkpoint 的事务要么在 WAL 里、要么已被回放）
    assert_eq!(store.entry_count().unwrap(), 2);
    drop(store);
    let reopened = Store::open(&db).unwrap();
    assert_eq!(reopened.entry_count().unwrap(), 2);
    drop(reopened);
    cleanup(&dir);
}
