//! 解析层测试：三种格式 + 身份判定的退化场景。
//!
//! 重点是身份稳定性——它直接决定「同一篇文章会不会被反复入库」。

use rustrss_core::html::html_to_text;
use rustrss_core::{parse, IdOrigin};

const RSS_WITH_GUID: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Rust Blog</title>
    <link>https://blog.rust-lang.org/</link>
    <description>官方博客</description>
    <item>
      <title>Announcing Rust 1.99</title>
      <link>https://blog.rust-lang.org/2026/09/18/Rust-1.99.html</link>
      <guid isPermaLink="false">rust-blog-199</guid>
      <pubDate>Fri, 18 Sep 2026 00:00:00 GMT</pubDate>
      <description><![CDATA[<p>Rust 1.99 发布，包含 <code>async</code> 闭包改进。</p>]]></description>
    </item>
  </channel>
</rss>"#;

const RSS_LINK_ONLY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>No Guid Feed</title>
    <link>https://example.com/</link>
    <item>
      <title>Only a link</title>
      <link>https://example.com/posts/only-a-link</link>
      <description>正文摘要</description>
    </item>
  </channel>
</rss>"#;

const RSS_BARE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Bare Feed</title>
    <link>https://example.org/</link>
    <item>
      <title>既无 guid 也无链接</title>
      <description>这是一条退化场景的条目</description>
      <pubDate>Sat, 19 Sep 2026 12:00:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#;

const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom 示例</title>
  <link href="https://example.net/"/>
  <id>urn:uuid:00000000-0000-0000-0000-000000000000</id>
  <updated>2026-09-20T00:00:00Z</updated>
  <entry>
    <title>Atom 条目</title>
    <link href="https://example.net/atom-entry"/>
    <id>urn:uuid:11111111-1111-1111-1111-111111111111</id>
    <updated>2026-09-20T01:00:00Z</updated>
    <summary>Atom 摘要</summary>
    <content type="html">&lt;p&gt;Atom 正文&lt;/p&gt;</content>
  </entry>
</feed>"#;

const JSON_FEED: &str = r#"{
  "version": "https://jsonfeed.org/version/1.1",
  "title": "JSON Feed 示例",
  "home_page_url": "https://example.com/",
  "items": [
    {
      "id": "jf-1",
      "url": "https://example.com/jf-1",
      "title": "JSON Feed 条目",
      "content_text": "纯文本正文",
      "date_published": "2026-09-19T08:00:00Z"
    }
  ]
}"#;

#[test]
fn rss_with_guid_keeps_source_identity() {
    let feed = parse(RSS_WITH_GUID.as_bytes()).expect("RSS 应能解析");
    assert_eq!(feed.title, "Rust Blog");
    assert_eq!(feed.site_url.as_deref(), Some("https://blog.rust-lang.org/"));
    assert_eq!(feed.entries.len(), 1);

    let e = &feed.entries[0];
    assert_eq!(e.stable_id, "sid:rust-blog-199");
    assert_eq!(e.id_origin, IdOrigin::SourceData);
    assert_eq!(
        e.url.as_deref(),
        Some("https://blog.rust-lang.org/2026/09/18/Rust-1.99.html")
    );
    assert!(e.title.contains("Rust 1.99"));

    // HTML 正文与纯文本正文都要拿到，且纯文本里不应残留标签
    assert!(e.content_html.as_deref().unwrap_or_default().contains("<p>"));
    let text = e.content_text.as_deref().unwrap_or_default();
    assert!(text.contains("Rust 1.99 发布"), "纯文本应有内容，实际={text:?}");
    assert!(!text.contains("<p>"), "纯文本不应含标签，实际={text:?}");
}

#[test]
fn rss_without_guid_but_with_link_is_stable() {
    let a = parse(RSS_LINK_ONLY.as_bytes()).expect("RSS 应能解析");
    let b = parse(RSS_LINK_ONLY.as_bytes()).expect("RSS 应能解析");
    assert_eq!(a.entries[0].id_origin, IdOrigin::SourceData);
    assert_eq!(
        a.entries[0].stable_id, b.entries[0].stable_id,
        "有链接时解析库给出的身份必须跨次稳定"
    );
}

#[test]
fn rss_without_guid_and_link_falls_back_to_content_hash() {
    let a = parse(RSS_BARE.as_bytes()).expect("RSS 应能解析");
    let b = parse(RSS_BARE.as_bytes()).expect("RSS 应能解析");

    assert_eq!(a.entries[0].id_origin, IdOrigin::ContentHash);
    assert!(a.entries[0].stable_id.starts_with("h:"));
    assert_eq!(
        a.entries[0].stable_id, b.entries[0].stable_id,
        "退化场景的兜底身份必须跨次稳定，否则同一篇文章会反复入库"
    );
}

#[test]
fn atom_basics() {
    let feed = parse(ATOM.as_bytes()).expect("Atom 应能解析");
    assert_eq!(feed.title, "Atom 示例");
    assert_eq!(feed.site_url.as_deref(), Some("https://example.net/"));

    let e = &feed.entries[0];
    assert_eq!(e.title, "Atom 条目");
    assert_eq!(e.url.as_deref(), Some("https://example.net/atom-entry"));
    assert_eq!(e.summary.as_deref(), Some("Atom 摘要"));
    assert_eq!(e.content_text.as_deref(), Some("Atom 正文"));
    assert!(e.published.is_some());
}

#[test]
fn json_feed_basics() {
    let feed = parse(JSON_FEED.as_bytes()).expect("JSON Feed 应能解析");
    assert_eq!(feed.title, "JSON Feed 示例");
    assert_eq!(feed.site_url.as_deref(), Some("https://example.com/"));

    let e = &feed.entries[0];
    assert_eq!(e.stable_id, "sid:jf-1");
    assert_eq!(e.url.as_deref(), Some("https://example.com/jf-1"));
    assert_eq!(e.content_text.as_deref(), Some("纯文本正文"));
    // 纯文本内容不该被当成 HTML
    assert_eq!(e.content_html, None);
}

#[test]
fn html_to_text_strips_tags_and_scripts() {
    let html = r#"<div><script>var a = 1 < 2;</script><style>p{color:red}</style>
        <p>第一段 &amp; 实体</p><p>第二段</p></div>"#;
    let text = html_to_text(html);

    assert!(text.contains("第一段 & 实体"), "实体应解码，实际={text:?}");
    assert!(text.contains("第二段"), "实际={text:?}");
    assert!(!text.contains("var a"), "script 内容不该进纯文本，实际={text:?}");
    assert!(!text.contains("color:red"), "style 内容不该进纯文本，实际={text:?}");
    assert!(!text.contains('<'), "不该残留标签，实际={text:?}");
    assert!(text.lines().count() >= 2, "块级标签应保留换行，实际={text:?}");
}

#[test]
fn namespaced_content_and_multiple_enclosures_keep_article_identity() {
    let xml = r#"<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
      <channel><title>Fixture</title><item><title>Article</title>
      <link>https://example.test/article</link>
      <enclosure url="https://example.test/audio.mp3" type="audio/mpeg" length="12"/>
      <enclosure url="https://example.test/image.jpg" type="image/jpeg" length="20"/>
      <content:encoded><![CDATA[<p>中文正文 &amp; 内容</p>]]></content:encoded>
      </item></channel></rss>"#;
    let a = parse(xml.as_bytes()).unwrap();
    let b = parse(xml.as_bytes()).unwrap();
    assert_eq!(a.entries.len(), 1);
    assert_eq!(a.entries[0].url.as_deref(), Some("https://example.test/article"));
    assert_eq!(a.entries[0].stable_id, b.entries[0].stable_id);
    assert!(a.entries[0].content_text.as_deref().unwrap().contains("中文正文 & 内容"));
    assert_eq!(a.entries[0].thumbnail_url.as_deref(), Some("https://example.test/image.jpg"));
}

#[test]
fn thumbnail_prefers_media_rss_then_uses_first_safe_inline_image() {
    let media = r#"<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/">
      <channel><title>Fixture</title><item><title>Media</title>
      <link>https://example.test/path/article</link>
      <media:thumbnail url="../media-thumb.jpg"/>
      <description><![CDATA[<img src="/summary-thumb.png">]]></description>
      </item></channel></rss>"#;
    assert_eq!(parse(media.as_bytes()).unwrap().entries[0].thumbnail_url.as_deref(),
        Some("https://example.test/media-thumb.jpg"));

    let inline = r#"<rss version="2.0"><channel><title>Fixture</title><item>
      <title>Inline</title><link>https://example.test/path/article</link>
      <description><![CDATA[<p><img alt='cover' loading='lazy' src='../cover.jpg?x=1&amp;y=2'></p>]]></description>
      </item></channel></rss>"#;
    assert_eq!(parse(inline.as_bytes()).unwrap().entries[0].thumbnail_url.as_deref(),
        Some("https://example.test/cover.jpg?x=1&y=2"));

    let unsafe_image = r#"<rss version="2.0"><channel><title>Fixture</title><item>
      <title>Unsafe</title><link>https://example.test/article</link>
      <description><![CDATA[<img src="javascript:alert(1)">]]></description>
      </item></channel></rss>"#;
    assert_eq!(parse(unsafe_image.as_bytes()).unwrap().entries[0].thumbnail_url, None);
}

#[test]
fn chinese_legacy_encodings_and_incorrect_declarations() {
    for (bytes, body) in [
        (include_bytes!("fixtures/encoding/gbk.xml").as_slice(), "中文正文"),
        (include_bytes!("fixtures/encoding/gb18030.xml").as_slice(), "中文正文𠀀"),
        (include_bytes!("fixtures/encoding/utf8-declared-gbk.xml").as_slice(), "中文正文"),
    ] {
        let feed = parse(bytes).unwrap();
        assert_eq!(feed.title, "中文订阅");
        assert_eq!(feed.entries[0].title, "中文标题");
        assert_eq!(feed.entries[0].content_text.as_deref(), Some(body));
    }
}

#[test]
fn utf8_conflict_handles_bom_quotes_and_preserves_legitimate_declarations() {
    let wrong = "\u{feff}<?xml version='1.0' encoding='gb18030'?><rss version='2.0'><channel><title>中文</title></channel></rss>";
    assert_eq!(parse(wrong.as_bytes()).unwrap().title, "中文");
    let latin = b"<?xml version='1.0' encoding='ISO-8859-1'?><rss version='2.0'><channel><title>Caf\xe9</title></channel></rss>";
    assert_eq!(parse(latin).unwrap().title, "Café");
}
