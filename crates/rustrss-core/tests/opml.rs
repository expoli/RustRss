//! OPML 导入导出的测试。
//!
//! 重点是三件事：导出能带标题里的特殊字符、导入能去重、往返不丢源。

use rustrss_core::opml::{self, ImportReport};
use rustrss_core::Store;

/// 一份贴近真实阅读器导出的 OPML：嵌套文件夹 + htmlUrl + 纯文字大纲 + 特殊字符
const REAL_WORLD_OPML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head><title>subscriptions</title></head>
  <body>
    <outline text="技术" title="技术">
      <outline type="rss" text="Rust Blog" title="Rust Blog"
               xmlUrl="https://blog.rust-lang.org/feed.xml" htmlUrl="https://blog.rust-lang.org/"/>
      <outline text="资讯" title="资讯">
        <outline type="rss" text="LWN &amp; friends" title="LWN &amp; friends"
                 xmlUrl="https://lwn.net/headlines/rss"/>
      </outline>
    </outline>
    <outline text="随手写的一段话（没有订阅也没有子节点）"/>
    <outline type="rss" text="少数派" title="少数派" xmlUrl="https://sspai.com/feed"/>
  </body>
</opml>"#;

#[test]
fn import_adds_feeds_folders_and_counts() {
    let store = Store::open_in_memory().unwrap();
    let report = opml::import(&store, REAL_WORLD_OPML).unwrap();

    assert_eq!(report.feeds_added, 3);
    assert_eq!(report.feeds_skipped, 0);
    // 「技术」与「技术/资讯」两个文件夹
    assert_eq!(report.folders_created, 2);
    assert_eq!(report.outlines_ignored, 1);

    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds.len(), 3);

    // 新增 id 列表是导入后增量抓取的依据：数量对得上，且每个 id 都能查到源
    assert_eq!(report.added_feed_ids.len(), 3, "{report:?}");
    let added_urls: Vec<&str> = report
        .added_feed_ids
        .iter()
        .map(|id| {
            feeds
                .iter()
                .find(|f| f.id == *id)
                .expect("新增 id 应能在库里查到")
                .url
                .as_str()
        })
        .collect();
    assert!(
        added_urls.iter().any(|u| u.contains("rust-lang")),
        "{added_urls:?}"
    );
    assert!(
        added_urls.iter().any(|u| u.contains("lwn.net")),
        "{added_urls:?}"
    );
    assert!(
        added_urls.iter().any(|u| u.contains("sspai")),
        "{added_urls:?}"
    );

    // 嵌套文件夹压平成 父/子
    let folders = store.list_folders().unwrap();
    let names: Vec<&str> = folders.iter().map(|(_, n)| n.as_str()).collect();
    assert!(names.contains(&"技术"), "{names:?}");
    assert!(names.contains(&"技术/资讯"), "{names:?}");

    // 实体的标题被正确解码（&amp; → &）
    let lwn = feeds.iter().find(|f| f.url.contains("lwn.net")).expect("LWN 应存在");
    assert_eq!(lwn.title, "LWN & friends");
    // htmlUrl 被记成 site_url
    let rust = feeds
        .iter()
        .find(|f| f.url.contains("rust-lang"))
        .expect("Rust Blog 应存在");
    assert_eq!(rust.site_url.as_deref(), Some("https://blog.rust-lang.org/"));
    // 未归类的源
    let sspai = feeds.iter().find(|f| f.url.contains("sspai")).expect("少数派应存在");
    assert!(sspai.folder_id.is_none());
}

#[test]
fn import_is_idempotent_by_url() {
    let store = Store::open_in_memory().unwrap();
    let first = opml::import(&store, REAL_WORLD_OPML).unwrap();
    assert_eq!(first.feeds_added, 3);

    let second = opml::import(&store, REAL_WORLD_OPML).unwrap();
    assert_eq!(second.feeds_added, 0);
    assert_eq!(second.feeds_skipped, 3, "同一份 OPML 再导入一次应全部跳过");
    assert!(
        second.added_feed_ids.is_empty(),
        "全部跳过时不应有新增 id（否则会被重复抓一遍）: {second:?}"
    );
    assert_eq!(store.list_feeds().unwrap().len(), 3, "不得出现重复源");
    assert_eq!(second.folders_created, 0, "文件夹也不应重复创建");
}

#[test]
fn export_escapes_special_characters_and_survives_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    let folder = store.add_folder("技术/资讯").unwrap();
    let tricky = store
        .add_feed(
            "https://example.com/feed?a=1&b=2",
            Some("A & B \"quoted\" <tag>"),
        )
        .unwrap();
    store.assign_folder(tricky, Some(folder)).unwrap();
    store.add_feed("https://example.com/plain.xml", Some("普通源")).unwrap();

    let xml = opml::export(&store).unwrap();
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(xml.contains("version=\"2.0\""));
    // 属性里的特殊字符必须转义，否则生成的 OPML 是非法的
    assert!(xml.contains("A &amp; B &quot;quoted&quot; &lt;tag&gt;"), "{xml}");
    assert!(
        xml.contains("xmlUrl=\"https://example.com/feed?a=1&amp;b=2\""),
        "{xml}"
    );

    // 往干净的库里往返一次，源数量应一致
    let fresh = Store::open_in_memory().unwrap();
    let report = opml::import(&fresh, &xml).unwrap();
    assert_eq!(report.feeds_added, 2);
    assert_eq!(report.feeds_skipped, 0);
    assert_eq!(fresh.list_feeds().unwrap().len(), 2);
    assert_eq!(fresh.list_folders().unwrap().len(), 1);
}

#[test]
fn malformed_input_is_reported_not_silently_empty() {
    let store = Store::open_in_memory().unwrap();

    let err = opml::import(&store, "<html><body>这不是 OPML</body></html>").unwrap_err();
    assert!(err.to_string().contains("没有 <opml>"), "{err}");

    let err = opml::import(&store, "<opml><body><outline").unwrap_err();
    assert!(err.to_string().contains("解析失败"), "{err}");

    // 出错时不应留下半截数据
    assert_eq!(store.list_feeds().unwrap().len(), 0);
}

#[test]
fn empty_opml_is_not_an_error() {
    let store = Store::open_in_memory().unwrap();
    let xml = r#"<?xml version="1.0"?><opml version="2.0"><head/><body/></opml>"#;
    let report = opml::import(&store, xml).unwrap();
    assert_eq!(report, ImportReport::default());
}

/// RSSHub scheme 订阅的 OPML 回环：导出 scheme 形态（可移植），再导入不重复。
///
/// 三种写法（scheme / 官方域 / www 官方域）在库内是同一个身份，导出只出 scheme；
/// 用官方域形态的 OPML 再导入也必须判为「已存在」，否则换台机器/重导一次就会
/// 报告新增而实际什么都没加（计数与实际不符）。
#[test]
fn rsshub_scheme_urls_export_as_scheme_and_reimport_as_duplicates() {
    let store = Store::open_in_memory().unwrap();
    store.add_feed("rsshub://test/1", Some("测试路由")).unwrap();
    assert_eq!(store.list_feeds().unwrap()[0].url, "rsshub://test/1");

    // 导出：xmlUrl 是 scheme 形态，不含任何实例域名
    let xml = opml::export(&store).unwrap();
    assert!(xml.contains("xmlUrl=\"rsshub://test/1\""), "{xml}");
    assert!(!xml.contains("rsshub.app"), "导出不得绑定实例域名: {xml}");

    // 自己导出的东西再导入：全部跳过，不新增
    let again = opml::import(&store, &xml).unwrap();
    assert_eq!(again.feeds_added, 0);
    assert_eq!(again.feeds_skipped, 1);
    assert_eq!(store.list_feeds().unwrap().len(), 1);

    // 官方域形态的 OPML（别人导出的旧文件）导入同一路由：也是重复，不是新增
    let legacy = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0"><head><title>s</title></head><body>
<outline type="rss" text="X" title="X" xmlUrl="https://rsshub.app/test/1"/>
</body></opml>"#;
    let legacy_report = opml::import(&store, legacy).unwrap();
    assert_eq!(legacy_report.feeds_added, 0, "官方域形态应判为同一订阅");
    assert_eq!(legacy_report.feeds_skipped, 1);
    assert_eq!(store.list_feeds().unwrap().len(), 1);

    // 往干净库导入旧形态 OPML：落库即 scheme（此后随镜像变化，无需迁移）
    let fresh = Store::open_in_memory().unwrap();
    let report = opml::import(&fresh, legacy).unwrap();
    assert_eq!(report.feeds_added, 1);
    assert_eq!(report.added_feed_ids.len(), 1);
    assert_eq!(fresh.list_feeds().unwrap()[0].url, "rsshub://test/1");
}
