//! 标签（文章级 tag）数据层测试。重点验四件事：
//! 1. 迁移 v12：幂等 + 升级路径（旧库 → v12 数据不丢）；
//! 2. store API 行为：CRUD / 批量与条件级打标 / 幂等 / `last_used_at` 推进；
//! 3. 级联不变式：`remove_feed` 与删标签后 `entry_tags` 零孤儿；
//! 4. 性能红线：标签筛选与标签未读计数走索引（EXPLAIN 断言 + 变异校验）。

use rustrss_core::rsshub;
use rustrss_core::store::schema::MIGRATIONS;
use rustrss_core::store::{LIST_HIDE_READ_KEY, LIST_SORT_KEY};
use rustrss_core::{
    Entry, EntryFlagScope, EntryQuery, IdOrigin, ListSort, Store, StoreError, TagBrief, TagTarget,
    TAG_BATCH_MAX_IDS,
};

const BASE_TS: i64 = 1_700_000_000;

fn mk_entry(stable_id: &str, published: i64) -> Entry {
    Entry {
        stable_id: stable_id.to_string(),
        id_origin: IdOrigin::SourceData,
        source_id: stable_id.to_string(),
        title: stable_id.to_string(),
        url: Some(format!("https://example.com/{stable_id}")),
        author: None,
        published: Some(chrono::DateTime::from_timestamp(published, 0).expect("合法时间戳")),
        updated: None,
        summary: Some(format!("正文{stable_id}")),
        content_html: Some(format!("<p>正文{stable_id}</p>")),
        content_text: Some(format!("正文{stable_id}")),
        categories: Vec::new(),
    }
}

/// 临时库文件路径（每个用例一条，避免相互干扰）。
fn temp_db(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rustrss-tags-{label}-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// 内存库 + 两个源（f1：a1/a2/a3；f2：b1），时间递减，全部未读。
///
/// 返回 `(store, f1, f2)`；条目 id 依次是 1..=4（a1=1, a2=2, a3=3, b1=4）。
fn setup() -> (Store, i64, i64) {
    let store = Store::open_in_memory().expect("内存库应能打开");
    let f1 = store
        .add_feed("https://example.com/a.xml", Some("A源"))
        .unwrap();
    let f2 = store
        .add_feed("https://example.com/b.xml", Some("B源"))
        .unwrap();
    store
        .upsert_entries(
            f1,
            &[
                mk_entry("a1", BASE_TS + 300),
                mk_entry("a2", BASE_TS + 200),
                mk_entry("a3", BASE_TS + 100),
            ],
        )
        .unwrap();
    store
        .upsert_entries(f2, &[mk_entry("b1", BASE_TS + 400)])
        .unwrap();
    (store, f1, f2)
}

fn entry_ids(store: &Store, q: EntryQuery) -> Vec<i64> {
    store
        .list_entries(&q)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect()
}

fn tag_names(rows: &[rustrss_core::TagRow]) -> Vec<String> {
    rows.iter().map(|t| t.name.clone()).collect()
}

// ------------------------------------------------------------ 迁移 v12

#[test]
fn migration_v12_upgrade_keeps_existing_data_and_adds_tag_schema() {
    // 真实走一次 v11→v12：用迁移 1..11 **原样**建一个 user_version=11 的真文件库
    // （v11 形状不手抄，避免抄错而漂移），预置源/分组/自定义源名/条目与状态，
    // 再让 Store::open 只跑第 12 条迁移。
    let db_path = temp_db("upgrade");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&MIGRATIONS[..11].join(";")).unwrap();
        conn.execute_batch(
            "INSERT INTO folders (id, name, position) VALUES (1, '分组', 0);
             INSERT INTO feeds (id, url, title, custom_title, folder_id, created_at)
               VALUES (1, 'https://example.com/a.xml', '源站名', '我的名字', 1, 0);
             INSERT INTO entries (id, feed_id, stable_id, id_origin, title, fetched_at)
               VALUES (1, 1, 's1', 'source_data', 'T1', 100),
                      (2, 1, 's2', 'source_data', 'T2', 200);
             UPDATE entries SET read = 1, starred = 1 WHERE id = 1;
             UPDATE entries SET content_text = '正文大列' WHERE id = 1;",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 11).unwrap();
    }

    let store = Store::open(&db_path).expect("打开应自动跑 v12 迁移");
    assert_eq!(
        store.schema_version().unwrap() as usize,
        MIGRATIONS.len(),
        "升级后 schema 应到最新版本"
    );
    // 既有数据与状态不丢
    let (total, unread, starred, _later) = store.counts().unwrap();
    assert_eq!((total, unread, starred), (2, 1, 1), "升级不改变条目与状态");
    assert_eq!(
        store.list_feeds().unwrap()[0].title,
        "我的名字",
        "自定义源名保留"
    );
    assert_eq!(store.list_folders().unwrap().len(), 1, "分组保留");
    assert!(store.get_entry(1).unwrap().unwrap().read, "已读状态保留");

    // 新表 / 新索引就位
    let objects: Vec<String> = {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE name IN
                        ('tags','entry_tags','idx_entry_tags_tag','idx_entries_unread_id')",
            )
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    let mut objects = objects;
    objects.sort();
    assert_eq!(
        objects,
        vec![
            "entry_tags".to_string(),
            "idx_entries_unread_id".to_string(),
            "idx_entry_tags_tag".to_string(),
            "tags".to_string()
        ],
        "v12 应建出两张新表与两个新索引"
    );

    // 升级后标签能力立刻可用，且不产生孤儿关联
    let tag = store.create_tag("Rust", Some("#FF0000")).unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![1, 2]), &[tag.id])
        .unwrap();
    assert_eq!(
        store.entry_tags(1).unwrap(),
        vec![TagBrief {
            id: tag.id,
            name: "Rust".into()
        }]
    );
    assert_eq!(
        store.orphan_entry_tag_count().unwrap(),
        0,
        "升级路径不得留孤儿"
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn migration_v12_is_idempotent_and_keeps_tags() {
    // 二次打开（版本已是 v12）不应重跑迁移：标签与关联原样保留。
    let db_path = temp_db("idempotent");
    let tag_id = {
        let store = Store::open(&db_path).unwrap();
        let tag = store.create_tag("Rust", None).unwrap();
        store
            .assign_tags(&TagTarget::Entries(vec![1]), &[tag.id])
            .unwrap();
        tag.id
    };
    let store = Store::open(&db_path).expect("重开不应失败");
    assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    let rows = store.list_tags().unwrap();
    assert_eq!(rows.len(), 1, "重开不该把标签冲掉");
    assert_eq!(rows[0].id, tag_id);
    // 重开后的新连接上标签能力照常可用（幂等不是「只读」）
    store
        .assign_tags(&TagTarget::Entries(vec![1]), &[tag_id])
        .unwrap();
    assert_eq!(store.orphan_entry_tag_count().unwrap(), 0);
    let _ = std::fs::remove_file(&db_path);
}

// ------------------------------------------------------------ 名称与颜色校验

#[test]
fn tag_name_is_trimmed_and_unique_case_insensitively() {
    let (store, _f1, _f2) = setup();
    let tag = store.create_tag("  Rust  ", None).unwrap();
    assert_eq!(tag.name, "Rust", "名称应 trim");
    assert_eq!(tag.color, None);
    assert_eq!(tag.last_used_at, None, "没用过就没有最近使用时间");
    assert_eq!(tag.sort_order, 0, "第一个标签顺序 0");

    let second = store.create_tag("数据库", None).unwrap();
    assert_eq!(second.sort_order, 1, "新标签排在末尾（取 max+1）");

    for (label, name) in [("完全相同", "Rust"), ("仅大小写差异", "rUsT")] {
        let err = store.create_tag(name, None).unwrap_err();
        assert!(
            matches!(err, StoreError::DuplicateTagName(ref n) if n.eq_ignore_ascii_case("Rust")),
            "{label} 重名应报 DuplicateTagName，实际: {err:?}"
        );
    }
    let err = store.create_tag("   ", None).unwrap_err();
    assert!(
        matches!(err, StoreError::Invalid(_)),
        "空名应报 Invalid，实际: {err:?}"
    );

    // 改名：不撞名可用；撞名（含仅大小写差异）报重名；只改自身大小写允许
    let renamed = store.rename_tag(tag.id, "Rust Lang").unwrap();
    assert_eq!(renamed.name, "Rust Lang");
    assert_eq!(
        tag_names(&store.list_tags().unwrap()),
        ["Rust Lang", "数据库"],
        "改名后按名称重排"
    );
    let err = store.rename_tag(tag.id, "数据库").unwrap_err();
    assert!(
        matches!(err, StoreError::DuplicateTagName(_)),
        "撞名应报重名，实际: {err:?}"
    );
    let err = store.rename_tag(tag.id, "  数据库  ").unwrap_err();
    assert!(
        matches!(err, StoreError::DuplicateTagName(ref n) if n == "数据库"),
        "改名时 trim 后撞名也应报重名，实际: {err:?}"
    );
    let case_only = store.rename_tag(tag.id, "RUST LANG").unwrap();
    assert_eq!(case_only.name, "RUST LANG", "只改自身大小写应允许");
    let err = store.rename_tag(9_999, "别的").unwrap_err();
    assert!(
        matches!(err, StoreError::TagNotFound(9_999)),
        "未知标签应报 TagNotFound，实际: {err:?}"
    );
}

#[test]
fn tag_color_format_is_validated_and_normalized() {
    let (store, _f1, _f2) = setup();
    let tag = store.create_tag("Rust", Some(" #FF0000 ")).unwrap();
    assert_eq!(tag.color.as_deref(), Some("#ff0000"), "颜色统一小写落库");

    for bad in ["red", "#FF00", "#GGGGGG", "FF0000", "#FF00001"] {
        let err = store.create_tag("另一个", Some(bad)).unwrap_err();
        assert!(
            matches!(err, StoreError::Invalid(ref m) if m.contains("#RRGGBB")),
            "非法颜色「{bad}」应报 Invalid，实际: {err:?}"
        );
    }
    // 清除色：None 与空白都归 None
    assert_eq!(store.set_tag_color(tag.id, None).unwrap().color, None);
    assert_eq!(
        store
            .set_tag_color(tag.id, Some("#00FF00"))
            .unwrap()
            .color
            .as_deref(),
        Some("#00ff00")
    );
    assert_eq!(
        store.set_tag_color(tag.id, Some("   ")).unwrap().color,
        None
    );
    let err = store.set_tag_color(tag.id, Some("红色")).unwrap_err();
    assert!(matches!(err, StoreError::Invalid(_)), "实际: {err:?}");
    let err = store.set_tag_color(9_999, Some("#123456")).unwrap_err();
    assert!(
        matches!(err, StoreError::TagNotFound(9_999)),
        "实际: {err:?}"
    );
}

// ------------------------------------------------------------ 列表 / 置顶 / 排序 / 删除

#[test]
fn list_tags_reports_counts_pinning_and_ordering() {
    let (store, _f1, _f2) = setup();
    let rust = store.create_tag("Rust", Some("#111111")).unwrap();
    let db = store.create_tag("数据库", None).unwrap();
    let ops = store.create_tag("运维", None).unwrap();

    // f1 的 a1/a2 + f2 的 b1 → Rust 3 篇；把 a1 标已读 → 未读 2
    store
        .assign_tags(&TagTarget::Entries(vec![1, 2, 4]), &[rust.id])
        .unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![3]), &[db.id])
        .unwrap();
    store.set_read(&[1], true).unwrap();

    let rows = store.list_tags().unwrap();
    assert_eq!(
        tag_names(&rows),
        ["Rust", "数据库", "运维"],
        "默认按 sort_order/名称"
    );
    let rust_row = rows.iter().find(|t| t.id == rust.id).unwrap();
    assert_eq!(rust_row.unread, 2, "未读数应排除已读");
    assert_eq!(rust_row.color.as_deref(), Some("#111111"));
    assert!(!rust_row.pinned);
    assert!(rust_row.last_used_at.is_some(), "打标后应有最近使用时间");
    assert_eq!(rows.iter().find(|t| t.id == db.id).unwrap().unread, 1);
    assert_eq!(
        rows.iter().find(|t| t.id == ops.id).unwrap().unread,
        0,
        "没人用的标签未读 0"
    );

    // 置顶优先（其余仍按 sort_order）
    store.set_tag_pinned(ops.id, true).unwrap();
    assert_eq!(
        tag_names(&store.list_tags().unwrap()),
        ["运维", "Rust", "数据库"]
    );

    // 拖拽排序：下标即 sort_order（单事务）
    store.reorder_tags(&[db.id, rust.id, ops.id]).unwrap();
    assert_eq!(
        tag_names(&store.list_tags().unwrap()),
        ["运维", "数据库", "Rust"],
        "置顶仍优先于手动顺序"
    );

    // 重排碰到不存在的 id → 整体回滚（不允许「重排了一半」）
    let before = store.list_tags().unwrap();
    let err = store.reorder_tags(&[rust.id, 9_999]).unwrap_err();
    assert!(
        matches!(err, StoreError::TagNotFound(9_999)),
        "实际: {err:?}"
    );
    assert_eq!(store.list_tags().unwrap(), before, "失败的重排必须整体回滚");
    assert_eq!(
        store.list_tags_recent_first().unwrap().len(),
        3,
        "最近使用口径也能列出（顺序见下个用例）"
    );

    // tag_entry_count 是关联篇数（不是未读数）
    assert_eq!(store.tag_entry_count(rust.id).unwrap(), 3);
    assert_eq!(store.tag_entry_count(ops.id).unwrap(), 0);
}

#[test]
fn list_tags_recent_first_puts_used_tags_ahead() {
    let db_path = temp_db("recent");
    let (first, second, third) = {
        let store = Store::open(&db_path).unwrap();
        let f = store
            .add_feed("https://example.com/a.xml", Some("A"))
            .unwrap();
        store.upsert_entries(f, &[mk_entry("a1", BASE_TS)]).unwrap();
        let first = store.create_tag("一", None).unwrap().id;
        let second = store.create_tag("二", None).unwrap().id;
        let third = store.create_tag("三", None).unwrap().id;
        (first, second, third)
    };
    // 用哨兵时间戳手工铺「使用记录」（store 没有公开入口改它——只由打标推进）
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE tags SET last_used_at = 1000 WHERE id = ?1",
            rusqlite::params![second],
        )
        .unwrap();
        conn.execute(
            "UPDATE tags SET last_used_at = 2000 WHERE id = ?1",
            rusqlite::params![third],
        )
        .unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    let rows = store.list_tags_recent_first().unwrap();
    assert_eq!(
        rows.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![third, second, first],
        "最近使用优先，没用过的（NULL）垫底；侧栏顺序仍是手动序"
    );
    assert_eq!(
        store
            .list_tags()
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![first, second, third],
        "侧栏口径不受最近使用影响"
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn delete_tag_dry_run_matches_the_real_delete_and_keeps_articles() {
    let (store, _f1, _f2) = setup();
    let tag = store.create_tag("Rust", None).unwrap();
    let other = store.create_tag("数据库", None).unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![1, 2]), &[tag.id, other.id])
        .unwrap();

    // dry_run：只回报影响篇数，不落库
    let preview = store.delete_tag(tag.id, true).unwrap();
    assert_eq!(preview.affected_entries, 2, "影响 2 篇");
    assert!(preview.dry_run);
    assert_eq!(
        store.tag_entry_count(tag.id).unwrap(),
        2,
        "预览不得删任何关联"
    );
    assert!(store.tag_row(tag.id).unwrap().is_some(), "预览不得删标签");

    // 真删：只清关联，文章还在，另一个标签的关联不受影响
    let real = store.delete_tag(tag.id, false).unwrap();
    assert_eq!(
        real.affected_entries, preview.affected_entries,
        "预览与执行必须同源同数"
    );
    assert!(!real.dry_run);
    assert!(store.tag_row(tag.id).unwrap().is_none(), "标签已删");
    assert_eq!(store.entry_count().unwrap(), 4, "文章一篇都不能少");
    assert_eq!(store.tag_entry_count(tag.id).unwrap(), 0);
    assert_eq!(
        store.orphan_entry_tag_count().unwrap(),
        0,
        "删标签不得留孤儿"
    );
    assert_eq!(
        store.entry_tags(1).unwrap(),
        vec![TagBrief {
            id: other.id,
            name: "数据库".into()
        }],
        "同条目上的其它标签必须留着"
    );
    let err = store.delete_tag(tag.id, true).unwrap_err();
    assert!(
        matches!(err, StoreError::TagNotFound(_)),
        "重复删除报 TagNotFound，实际: {err:?}"
    );
}

// ------------------------------------------------------------ 打标 / 取消打标

#[test]
fn assign_and_unassign_batch_are_idempotent_and_touch_last_used_at() {
    let (store, _f1, _f2) = setup();
    let rust = store.create_tag("Rust", None).unwrap();
    let db = store.create_tag("数据库", None).unwrap();
    assert_eq!(rust.last_used_at, None);

    // 批量：未知条目 id 静默跳过（只给真实存在的条目建关联）
    let report = store
        .assign_tags(&TagTarget::Entries(vec![1, 2, 9_999]), &[rust.id, db.id])
        .unwrap();
    assert_eq!(report.changed, 4, "2 条目 × 2 标签");
    assert_eq!(report.entries, 2, "命中条目数不含不存在的 id");
    assert_eq!(report.tag_ids, vec![rust.id, db.id], "标签 id 去重升序");
    assert!(
        store
            .tag_row(rust.id)
            .unwrap()
            .unwrap()
            .last_used_at
            .is_some(),
        "打标推进最近使用"
    );
    assert_eq!(store.orphan_entry_tag_count().unwrap(), 0);

    // 重复附加：零新增、零报错（严格幂等，`last_used_at` 也不该再动——下面的哨兵用例专测）
    let again = store
        .assign_tags(&TagTarget::Entries(vec![1, 2]), &[rust.id, db.id])
        .unwrap();
    assert_eq!(again.changed, 0, "重复附加不得新增关联");
    assert_eq!(store.tag_entry_count(rust.id).unwrap(), 2);

    // 取消：只删给定条目上的关联，幂等
    let removed = store
        .unassign_tags(&TagTarget::Entries(vec![1]), &[rust.id, db.id])
        .unwrap();
    assert_eq!(removed.changed, 2);
    assert!(store.entry_tags(1).unwrap().is_empty());
    assert_eq!(
        store.entry_tags(2).unwrap().len(),
        2,
        "别的条目上的关联不动"
    );
    let removed_again = store
        .unassign_tags(&TagTarget::Entries(vec![1]), &[rust.id, db.id])
        .unwrap();
    assert_eq!(removed_again.changed, 0, "重复取消同样是零副作用");

    // 目标与标签的边界
    let err = store
        .assign_tags(&TagTarget::Entries(vec![]), &[rust.id])
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Invalid(_)),
        "空条目列表应报错，实际: {err:?}"
    );
    let big: Vec<i64> = (1..=(TAG_BATCH_MAX_IDS as i64 + 1)).collect();
    let err = store
        .assign_tags(&TagTarget::Entries(big), &[rust.id])
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Invalid(ref m) if m.contains("100")),
        "批量上限应明确报错，实际: {err:?}"
    );
    let err = store
        .assign_tags(&TagTarget::Entries(vec![1]), &[])
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Invalid(_)),
        "空标签列表应报错，实际: {err:?}"
    );
    let err = store
        .assign_tags(&TagTarget::Entries(vec![1]), &[9_999])
        .unwrap_err();
    assert!(
        matches!(err, StoreError::TagNotFound(9_999)),
        "未知标签应报错，实际: {err:?}"
    );
    let err = store
        .assign_tags(&TagTarget::Scope(EntryFlagScope::default()), &[rust.id])
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Invalid(ref m) if m.contains("条件")),
        "空范围不得当成「全部」，实际: {err:?}"
    );
    assert_eq!(store.orphan_entry_tag_count().unwrap(), 0);
}

#[test]
fn repeat_assign_is_a_strict_no_op_for_last_used_at() {
    // 幂等不是「不报错」而是**零副作用**：`last_used_at` 只在真的改变了关联时推进。
    // 时间戳是秒级，同秒内比较不出变化——所以把哨兵值直接钉进库里再验。
    let db_path = temp_db("last-used");
    let tag_id = {
        let store = Store::open(&db_path).unwrap();
        let f = store
            .add_feed("https://example.com/a.xml", Some("A"))
            .unwrap();
        store
            .upsert_entries(f, &[mk_entry("a1", BASE_TS), mk_entry("a2", BASE_TS + 1)])
            .unwrap();
        let tag = store.create_tag("Rust", None).unwrap();
        store
            .assign_tags(&TagTarget::Entries(vec![1]), &[tag.id])
            .unwrap();
        tag.id
    };
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE tags SET last_used_at = 1 WHERE id = ?1",
            rusqlite::params![tag_id],
        )
        .unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    // 重复附加（关联已存在）→ 哨兵值不许动
    let report = store
        .assign_tags(&TagTarget::Entries(vec![1]), &[tag_id])
        .unwrap();
    assert_eq!(report.changed, 0);
    assert_eq!(
        store.tag_row(tag_id).unwrap().unwrap().last_used_at,
        Some(1),
        "纯重复调用必须零副作用（不得推进 last_used_at）"
    );
    // 真的新增了关联 → 推进（哨兵值一定被换掉）
    let report = store
        .assign_tags(&TagTarget::Entries(vec![2]), &[tag_id])
        .unwrap();
    assert_eq!(report.changed, 1);
    let bumped = store
        .tag_row(tag_id)
        .unwrap()
        .unwrap()
        .last_used_at
        .unwrap();
    assert!(bumped > 1, "新增关联应推进 last_used_at，实际: {bumped}");
    // 取消也是「用了一次」：同样推进（语义与选择器「最近使用」一致）
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE tags SET last_used_at = 2 WHERE id = ?1",
            rusqlite::params![tag_id],
        )
        .unwrap();
    }
    let report = store
        .unassign_tags(&TagTarget::Entries(vec![2]), &[tag_id])
        .unwrap();
    assert_eq!(report.changed, 1);
    assert!(
        store
            .tag_row(tag_id)
            .unwrap()
            .unwrap()
            .last_used_at
            .unwrap()
            > 2,
        "取消打标同样推进最近使用时间"
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn conditional_assign_targets_feed_and_time_range() {
    let (store, f1, f2) = setup();
    let tag = store.create_tag("Rust", None).unwrap();

    // 条件级：f1 + [BASE+150, BASE+250] 闭区间 → 只有 a2（BASE+200）
    let scope = EntryFlagScope {
        feed_id: Some(f1),
        since: Some(BASE_TS + 150),
        until: Some(BASE_TS + 250),
    };
    let report = store
        .assign_tags(&TagTarget::Scope(scope), &[tag.id])
        .unwrap();
    assert_eq!(report.changed, 1);
    assert_eq!(report.entries, 1);
    assert_eq!(
        entry_ids(
            &store,
            EntryQuery {
                tag_id: Some(tag.id),
                ..Default::default()
            }
        ),
        vec![2]
    );

    // 幂等：同一条件再来一次零新增
    let again = store
        .assign_tags(&TagTarget::Scope(scope), &[tag.id])
        .unwrap();
    assert_eq!(again.changed, 0);

    // 全源范围（只给时间条件）→ a1/a3/b1 里落在区间内的都要
    let wide = EntryFlagScope {
        feed_id: None,
        since: Some(BASE_TS),
        until: Some(BASE_TS + 10_000),
    };
    let all = store
        .assign_tags(&TagTarget::Scope(wide), &[tag.id])
        .unwrap();
    assert_eq!(all.entries, 4, "四个条目全部落在区间内");
    assert_eq!(all.changed, 3, "a2 已有该标签，只新增 3 条");
    assert_eq!(
        store.list_tags().unwrap()[0].unread,
        4,
        "四个条目都打上了该标签"
    );

    // 条件级取消：只影响 f2
    let f2_only = EntryFlagScope {
        feed_id: Some(f2),
        since: None,
        until: None,
    };
    let removed = store
        .unassign_tags(&TagTarget::Scope(f2_only), &[tag.id])
        .unwrap();
    assert_eq!(removed.changed, 1);
    assert!(store.entry_tags(4).unwrap().is_empty());
    assert_eq!(store.tag_entry_count(tag.id).unwrap(), 3);
    assert_eq!(store.orphan_entry_tag_count().unwrap(), 0);

    // 条件级同样吃「未知标签」的错
    let err = store
        .assign_tags(&TagTarget::Scope(f2_only), &[9_999])
        .unwrap_err();
    assert!(
        matches!(err, StoreError::TagNotFound(9_999)),
        "实际: {err:?}"
    );
}

// ------------------------------------------------------------ 条目查询扩展

#[test]
fn entry_query_tag_filter_and_rows_carry_tags() {
    let (store, _f1, _f2) = setup();
    let rust = store.create_tag("Rust", None).unwrap();
    let db = store.create_tag("数据库", None).unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![1, 3, 4]), &[rust.id])
        .unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![3]), &[db.id])
        .unwrap();
    store.set_read(&[4], true).unwrap();

    // 过滤：只有打了该标签的条目，且仍按时间倒序（a1=BASE+300 / a3=BASE+100 / b1=BASE+400）
    let tagged = store
        .list_entries(&EntryQuery {
            tag_id: Some(rust.id),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        tagged.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![4, 1, 3]
    );
    assert_eq!(
        tagged.iter().find(|r| r.id == 3).unwrap().tags,
        vec![
            TagBrief {
                id: rust.id,
                name: "Rust".into()
            },
            TagBrief {
                id: db.id,
                name: "数据库".into()
            },
        ],
        "条目输出带 tags（id + 名称；名称序按字节比较，ASCII 在前）"
    );
    assert!(tagged.iter().find(|r| r.id == 1).unwrap().tags.len() == 1);
    assert_eq!(
        store
            .list_entries(&EntryQuery {
                tag_id: Some(rust.id),
                unread_only: true,
                ..Default::default()
            })
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        vec![1, 3],
        "标签过滤与未读视图可叠加"
    );

    // 未知标签 = 匹配零条（不是「不过滤」）
    assert!(store
        .list_entries(&EntryQuery {
            tag_id: Some(9_999),
            ..Default::default()
        })
        .unwrap()
        .is_empty());

    // 阅读页与搜索同样带标签（三条读取路径一个口径）
    let one = store.get_entry(3).unwrap().unwrap();
    assert_eq!(one.tags.len(), 2, "阅读页也带标签");
    assert_eq!(
        store.search("a3", 10).unwrap()[0].tags.len(),
        2,
        "搜索行也带标签"
    );

    // 取消打标后条目输出立刻为空
    store
        .unassign_tags(&TagTarget::Entries(vec![3]), &[rust.id, db.id])
        .unwrap();
    assert!(store.get_entry(3).unwrap().unwrap().tags.is_empty());

    // 分页：标签过滤下的 keyset 续扫不重不漏
    let page1 = store
        .list_entries(&EntryQuery {
            tag_id: Some(rust.id),
            limit: Some(2),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page1.iter().map(|r| r.id).collect::<Vec<_>>(), vec![4, 1]);
    let last = page1.last().unwrap();
    let page2 = store
        .list_entries(&EntryQuery {
            tag_id: Some(rust.id),
            limit: Some(2),
            cursor: Some((last.sortkey, last.id)),
            ..Default::default()
        })
        .unwrap();
    assert!(
        page2.is_empty(),
        "下一页只该剩游标之后的标签条目（a3 已被取消标签）"
    );
}

#[test]
fn tag_filter_keeps_ui_settings_and_mcp_explicit_overrides() {
    // 加了 `tag_id` 之后**既有默认口径不变**：界面路径仍跟随设置，
    // MCP 路径仍靠显式 `sort` / `hide_read` 覆盖（回归）。
    let (store, _f1, _f2) = setup();
    let tag = store.create_tag("Rust", None).unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![1, 2, 3, 4]), &[tag.id])
        .unwrap();
    store.set_read(&[4], true).unwrap();

    // 界面设置：最早在前 + 隐藏已读
    store.set_setting(LIST_SORT_KEY, "oldest").unwrap();
    store.set_setting(LIST_HIDE_READ_KEY, "true").unwrap();
    assert_eq!(
        entry_ids(
            &store,
            EntryQuery {
                tag_id: Some(tag.id),
                ..Default::default()
            }
        ),
        vec![3, 2, 1],
        "带标签过滤时仍跟随界面设置（oldest + 隐藏已读）"
    );

    // MCP 口径：显式 newest + 不隐藏已读（设置被绕过）
    assert_eq!(
        entry_ids(
            &store,
            EntryQuery {
                tag_id: Some(tag.id),
                sort: Some(ListSort::Newest),
                hide_read: Some(false),
                ..Default::default()
            }
        ),
        vec![4, 1, 2, 3],
        "显式 MCP 口径在标签过滤下同样生效"
    );

    // 不带标签过滤的老口径不受影响（同一个断言形状）
    assert_eq!(
        entry_ids(&store, EntryQuery::default()),
        vec![3, 2, 1],
        "无标签过滤 + 跟随设置：口径与加标签字段之前一致"
    );
    assert_eq!(
        entry_ids(
            &store,
            EntryQuery {
                sort: Some(ListSort::Newest),
                hide_read: Some(false),
                ..Default::default()
            }
        ),
        vec![4, 1, 2, 3]
    );
}

// ------------------------------------------------------------ 级联 / 零孤儿

#[test]
fn remove_feed_leaves_no_orphan_entry_tags_and_keeps_other_feeds() {
    let (store, f1, f2) = setup();
    assert!(
        store.foreign_keys_enabled().unwrap(),
        "写入路径以 PRAGMA foreign_keys=ON 为主保险"
    );
    let rust = store.create_tag("Rust", None).unwrap();
    let db = store.create_tag("数据库", None).unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![1, 2, 3, 4]), &[rust.id])
        .unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![3]), &[db.id])
        .unwrap();
    assert_eq!(store.tag_entry_count(rust.id).unwrap(), 4);

    store.remove_feed(f1).unwrap();

    assert_eq!(
        store.orphan_entry_tag_count().unwrap(),
        0,
        "删源后不得有孤儿关联"
    );
    assert_eq!(store.tag_entry_count(rust.id).unwrap(), 1, "只剩 f2 的 b1");
    assert_eq!(store.tag_entry_count(db.id).unwrap(), 0, "f1 的关联全清了");
    assert_eq!(
        store.entry_tags(4).unwrap(),
        vec![TagBrief {
            id: rust.id,
            name: "Rust".into()
        }],
        "别的源的标签关联必须留着"
    );
    assert_eq!(store.list_tags().unwrap().len(), 2, "删源不删标签本身");
    assert_eq!(store.entry_count().unwrap(), 1, "只剩 f2 的条目");

    // 删掉最后一个源：条目与关联一起消失，标签还在
    store.remove_feed(f2).unwrap();
    assert_eq!(store.orphan_entry_tag_count().unwrap(), 0);
    assert_eq!(store.tag_entry_count(rust.id).unwrap(), 0);
    assert_eq!(store.list_tags().unwrap().len(), 2);
    assert_eq!(store.entry_count().unwrap(), 0);

    // 显式清理之外：不用 Store 的删除路径（raw DELETE 条目）时，FK CASCADE 仍然兜住
    // 「零孤儿」——这里是那条不变量的直接钉子。
    let db_path = temp_db("raw-delete");
    {
        let store = Store::open(&db_path).unwrap();
        let f1 = store
            .add_feed("https://example.com/a.xml", Some("A"))
            .unwrap();
        store
            .upsert_entries(f1, &[mk_entry("a1", BASE_TS), mk_entry("a2", BASE_TS + 1)])
            .unwrap();
        let tag = store.create_tag("Rust", None).unwrap();
        store
            .assign_tags(&TagTarget::Entries(vec![1, 2]), &[tag.id])
            .unwrap();
        assert_eq!(store.tag_entry_count(tag.id).unwrap(), 2);
    }
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON; DELETE FROM entries WHERE id = 1;")
            .unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    assert_eq!(
        store.orphan_entry_tag_count().unwrap(),
        0,
        "raw 删除条目后不得留孤儿（FK CASCADE）"
    );
    assert_eq!(store.tag_entry_count(1).unwrap(), 1, "只剩未被删的那条关联");
    let _ = std::fs::remove_file(&db_path);
}

// ------------------------------------------------------------ 查询计划断言

/// 裸表扫描（不带 `USING ...`）——正文大列所在的表 B 树被逐行穿过的信号。
fn bare_scan(plan: &str, table: &str) -> Option<String> {
    plan.split(" | ")
        .find(|l| l.starts_with(&format!("SCAN {table}")) && !l.contains("USING"))
        .map(|l| l.to_string())
}

#[test]
fn tag_filter_plan_rides_sort_and_tag_indexes() {
    // 按标签列条目：外层扫排序索引（LIMIT 一到就停），标签的条目 id 集由
    // `idx_entry_tags_tag` 覆盖索引给出——不得退化成 `SCAN entries`，也不得临时排序
    // （本程序从不 ANALYZE，计划不能看统计脸色）。
    let (store, _f1, _f2) = setup();
    let tag = store.create_tag("Rust", None).unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![1, 2, 4]), &[tag.id])
        .unwrap();

    for sort in [ListSort::Newest, ListSort::Oldest, ListSort::UnreadFirst] {
        let q = EntryQuery {
            tag_id: Some(tag.id),
            sort: Some(sort),
            hide_read: Some(false),
            limit: Some(20),
            ..Default::default()
        };
        let plan = store.explain_list_entries(&q).unwrap().join(" | ");
        assert!(
            bare_scan(&plan, "entries").is_none(),
            "{sort:?} 档标签过滤不得裸扫 entries 表: {plan}"
        );
        assert!(
            !plan.contains("USE TEMP B-TREE"),
            "{sort:?} 档标签过滤不得临时排序（排序索引应按序直取）: {plan}"
        );
        assert!(
            plan.contains("COVERING INDEX idx_entry_tags_tag"),
            "{sort:?} 档标签过滤应走 idx_entry_tags_tag 取标签的条目 id 集: {plan}"
        );
        let sort_index = match sort {
            ListSort::UnreadFirst => "idx_entries_unread_sortkey",
            ListSort::Newest | ListSort::Oldest => "idx_entries_sortkey",
        };
        assert!(
            plan.contains(sort_index),
            "{sort:?} 档标签过滤应扫该档排序索引 {sort_index}: {plan}"
        );
    }

    // 与不带标签过滤的老形态对比：只有多了标签那一层子查询，外层计划不变
    let plain = store
        .explain_list_entries(&EntryQuery {
            limit: Some(20),
            ..Default::default()
        })
        .unwrap()
        .join(" | ");
    assert!(
        !plain.contains("idx_entry_tags_tag"),
        "不加标签过滤时不该引入标签索引: {plain}"
    );

    // 变异校验：把 idx_entry_tags_tag 删掉，断言（经 INDEXED BY）必须建不出计划而不是
    // 静默换一个可能穿正文大列表 B 树的计划。
    let db_path = temp_db("filter-mutation");
    {
        let store = Store::open(&db_path).unwrap();
        let f1 = store
            .add_feed("https://example.com/a.xml", Some("A"))
            .unwrap();
        store
            .upsert_entries(f1, &[mk_entry("a1", BASE_TS)])
            .unwrap();
        let tag = store.create_tag("Rust", None).unwrap();
        store
            .assign_tags(&TagTarget::Entries(vec![1]), &[tag.id])
            .unwrap();
        assert!(store
            .explain_list_entries(&EntryQuery {
                tag_id: Some(tag.id),
                limit: Some(10),
                ..Default::default()
            })
            .is_ok());
    }
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("DROP INDEX idx_entry_tags_tag;")
            .unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    let err = store.explain_list_entries(&EntryQuery {
        tag_id: Some(1),
        limit: Some(10),
        ..Default::default()
    });
    assert!(
        err.is_err(),
        "idx_entry_tags_tag 被删后标签过滤计划必须建不出来（转红），而不是静默换索引"
    );
    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn tag_unread_count_plans_ride_covering_indexes() {
    // 标签未读计数（list_tags 的子查询）与标签关联篇数（delete_tag 的 dry_run）都是
    // COUNT：口径同 counts()/unread_summary——只扫覆盖索引，绝不穿正文大列所在表 B 树。
    let (store, _f1, _f2) = setup();
    let tag = store.create_tag("Rust", None).unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![1, 2, 4]), &[tag.id])
        .unwrap();
    store.set_read(&[1], true).unwrap();
    assert_eq!(store.list_tags().unwrap()[0].unread, 2);
    assert_eq!(store.tag_entry_count(tag.id).unwrap(), 3);

    let plan = store.explain_tag_list().unwrap().join(" | ");
    assert!(
        bare_scan(&plan, "entries").is_none(),
        "标签未读计数不得裸扫 entries 表（read 列排在正文大列之后）: {plan}"
    );
    assert!(
        !plan.contains("SCAN e "),
        "entries 只能按 rowid 走覆盖索引: {plan}"
    );
    assert!(
        plan.contains("COVERING INDEX idx_entries_unread_id"),
        "未读判断应走 (id, read) 覆盖索引: {plan}"
    );
    assert!(
        plan.contains("COVERING INDEX idx_entry_tags_tag"),
        "标签的条目 id 集应走 idx_entry_tags_tag: {plan}"
    );

    let count_plan = store.explain_tag_entry_count(tag.id).unwrap().join(" | ");
    assert!(
        count_plan.contains("COVERING INDEX idx_entry_tags_tag"),
        "关联篇数只扫 idx_entry_tags_tag: {count_plan}"
    );
    assert!(
        !count_plan.contains("entries"),
        "关联篇数的计划里不该出现 entries（根本不碰那张表）: {count_plan}"
    );

    // 变异校验一：删掉 (id, read) 覆盖索引 → 未读计数计划必须建不出来
    let db_path = temp_db("count-mutation-entries");
    let tag_id = {
        let store = Store::open(&db_path).unwrap();
        let f = store
            .add_feed("https://example.com/a.xml", Some("A"))
            .unwrap();
        store.upsert_entries(f, &[mk_entry("a1", BASE_TS)]).unwrap();
        let tag = store.create_tag("Rust", None).unwrap();
        store
            .assign_tags(&TagTarget::Entries(vec![1]), &[tag.id])
            .unwrap();
        assert!(store.explain_tag_list().is_ok());
        tag.id
    };
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("DROP INDEX idx_entries_unread_id;")
            .unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    assert!(
        store.explain_tag_list().is_err(),
        "idx_entries_unread_id 被删后未读计数计划必须建不出来（转红）"
    );
    let _ = std::fs::remove_file(&db_path);

    // 变异校验二：删掉 idx_entry_tags_tag → 未读计数与关联篇数两个计划都必须转红
    let db_path = temp_db("count-mutation-tags");
    {
        let store = Store::open(&db_path).unwrap();
        let f = store
            .add_feed("https://example.com/a.xml", Some("A"))
            .unwrap();
        store.upsert_entries(f, &[mk_entry("a1", BASE_TS)]).unwrap();
        let tag = store.create_tag("Rust", None).unwrap();
        store
            .assign_tags(&TagTarget::Entries(vec![1]), &[tag.id])
            .unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("DROP INDEX idx_entry_tags_tag;")
            .unwrap();
    }
    let store = Store::open(&db_path).unwrap();
    assert!(
        store.explain_tag_list().is_err(),
        "idx_entry_tags_tag 被删后未读计数计划必须建不出来（转红）"
    );
    assert!(
        store.explain_tag_entry_count(tag_id).is_err(),
        "idx_entry_tags_tag 被删后关联篇数计划必须建不出来（转红）"
    );
    let _ = std::fs::remove_file(&db_path);
}

// ------------------------------------------------------------ 与既有能力的关系

#[test]
fn tag_tables_do_not_disturb_existing_aggregates_and_rsshub_paths() {
    // 迁移到 v12 之后，既有聚合 / 未读汇总 / RSSHub 归一化的读路径与计划都不变
    // （新表只被新 API 使用）。
    let (store, _f1, _f2) = setup();
    let tag = store.create_tag("Rust", None).unwrap();
    store
        .assign_tags(&TagTarget::Entries(vec![1, 2]), &[tag.id])
        .unwrap();
    store.set_read(&[1], true).unwrap();

    let (total, unread, starred, later) = store.counts().unwrap();
    assert_eq!((total, unread, starred, later), (4, 3, 0, 0));
    assert_eq!(store.unread_total().unwrap(), 3);
    assert_eq!(
        store
            .unread_by_feed()
            .unwrap()
            .iter()
            .map(|(_, n)| *n)
            .sum::<i64>(),
        3
    );
    assert_eq!(
        store
            .unread_summary(rustrss_core::UnreadGroupBy::Feed)
            .unwrap()
            .len(),
        2
    );
    assert!(store
        .explain_counts()
        .unwrap()
        .join(" | ")
        .contains("idx_entries_sortkey"));
    assert_eq!(
        rsshub::canonical_scheme_url("https://rsshub.app/github/trending"),
        "rsshub://github/trending"
    );
    assert!(!store
        .explain_unread_summary(rustrss_core::UnreadGroupBy::Feed)
        .unwrap()
        .is_empty());
}

#[test]
fn tag_and_flag_plans_share_the_scope_where_clause() {
    // 条件级打标与条件级状态写入共用同一处 WHERE（`EntryFlagScope::where_sql`）：
    // 这里是两者口径一致的钉子——采样到的行集必须能被写入（同一批行的道理）。
    let (store, f1, _f2) = setup();
    let tag = store.create_tag("Rust", None).unwrap();
    let scope = EntryFlagScope {
        feed_id: Some(f1),
        since: Some(BASE_TS + 50),
        until: Some(BASE_TS + 250),
    };
    let sampled = store.entry_ids_scoped(&scope, 10).unwrap();
    assert_eq!(sampled, vec![2, 3], "scope 命中 a2/a3");
    let report = store
        .assign_tags(&TagTarget::Scope(scope), &[tag.id])
        .unwrap();
    assert_eq!(
        report.entries as usize,
        sampled.len(),
        "打标命中数与采样一致"
    );
    assert_eq!(
        entry_ids(
            &store,
            EntryQuery {
                tag_id: Some(tag.id),
                ..Default::default()
            }
        ),
        vec![2, 3],
        "默认最新在前：a2(BASE+200) → a3(BASE+100)"
    );
    assert!(store
        .explain_set_flag_scoped(rustrss_core::EntryFlag::Read, &scope)
        .is_ok());
}
