//! 全文提取与写回测试。三块：
//! 1. 提取行为——真实页面 fixture（典型文章页 / 非 HTML / 空 body）与体积闸门；
//! 2. 写回——`set_fulltext` 写正文并重算检索 token，之后刷新**不覆盖**已抓正文；
//! 3. 判定——`needs_fulltext`（前端「获取全文」按钮的显示条件）。

use rustrss_core::fulltext::{self, FulltextError, MAX_BYTES, SUMMARY_TEXT_MIN_CHARS};
use rustrss_core::store::schema::MIGRATIONS;
use rustrss_core::{Entry, EntryQuery, IdOrigin, Store};

const ARTICLE_URL: &str = "https://blog.example.com/posts/http-message-signatures";
const TYPICAL: &str = include_str!("fixtures/fulltext/typical-article.html");
const NON_HTML: &str = include_str!("fixtures/fulltext/non-html.txt");
const EMPTY: &str = include_str!("fixtures/fulltext/empty.html");

/// 摘要型条目：源只给了摘要（正文缺失），正是「获取全文」的目标
fn mk_summary_entry(stable_id: &str, title: &str, summary: &str) -> Entry {
    Entry {
        stable_id: stable_id.to_string(),
        id_origin: IdOrigin::SourceData,
        source_id: stable_id.to_string(),
        title: title.to_string(),
        url: Some(ARTICLE_URL.to_string()),
        author: None,
        published: None,
        updated: None,
        summary: Some(summary.to_string()),
        content_html: None,
        content_text: None,
        categories: Vec::new(),
    }
}

fn setup() -> (Store, i64) {
    let store = Store::open_in_memory().expect("内存库应能打开");
    let feed_id = store
        .add_feed("https://blog.example.com/feed.xml", Some("示例博客"))
        .expect("加源应成功");
    (store, feed_id)
}

/// 入库一条并返回它的行 id
fn upsert_one(store: &Store, feed_id: i64, entry: &Entry) -> i64 {
    store
        .upsert_entries(feed_id, std::slice::from_ref(entry))
        .expect("入库应成功");
    store.list_entries(&EntryQuery::default()).unwrap()[0].id
}

// ---------------------------------------------------------------- 提取行为

#[test]
fn typical_article_extracts_body_and_drops_page_chrome() {
    let out = fulltext::extract(TYPICAL, ARTICLE_URL).expect("典型文章页应能提取");

    // 正文段落、列表与引用都在
    assert!(
        out.content_text.contains("签名基的构造是这个协议里最容易写错的地方"),
        "正文段落应被提取：{}",
        out.content_text
    );
    assert!(out.content_text.contains("协议实现里的绝大多数事故"), "正文结尾应在");
    assert!(out.content_text.contains("把参与签名的字段固定成白名单"), "列表内容应在");

    // 页面装饰（导航 / 推广位 / 相关阅读 / 页脚广告）不进正文
    for chrome in [
        "订阅我们的邮件列表",
        "相关阅读",
        "加入会员解锁全部历史文章",
        "本页由赞助商提供带宽支持",
        "© 2026 示例博客",
    ] {
        assert!(
            !out.content_text.contains(chrome),
            "页面装饰不应进正文（{chrome}）：{}",
            out.content_text
        );
    }

    // 两个输出的一致性：content_html 是带标签的正文片段，content_text 是它的纯文本
    assert!(
        out.content_html.contains("<p>") && out.content_html.contains("<li>"),
        "content_html 应保留段落与列表标签：{}",
        out.content_html
    );
    assert!(!out.content_text.contains('<'), "纯文本里不该有标签");
    assert_eq!(
        out.content_text,
        rustrss_core::html::html_to_text(&out.content_html),
        "content_text 必须与全库同一套 HTML→文本换算（否则检索与展示会漂移）"
    );
}

#[test]
fn non_html_body_is_rejected_before_parsing() {
    let err = fulltext::extract(NON_HTML, ARTICLE_URL).expect_err("非 HTML 应被拒绝");
    assert!(matches!(err, FulltextError::NotHtml), "实际: {err}");
    assert!(
        err.to_string().contains("不是 HTML"),
        "错误要能直接显示给用户: {err}"
    );
}

#[test]
fn empty_body_has_no_content_to_extract() {
    let err = fulltext::extract(EMPTY, ARTICLE_URL).expect_err("空 body 应报「没有正文」");
    assert!(matches!(err, FulltextError::NoContent), "实际: {err}");
}

#[test]
fn url_must_be_absolute_http() {
    for bad in ["/posts/1", "example.com/post", "file:///etc/passwd", "  "] {
        let err = fulltext::extract(TYPICAL, bad).expect_err("非绝对 http 地址应被拒绝");
        assert!(matches!(err, FulltextError::BadUrl), "地址 {bad:?} 实际: {err}");
    }
}

#[test]
fn size_gate_rejects_over_limit_and_passes_the_boundary() {
    assert_eq!(MAX_BYTES, 2 * 1024 * 1024, "上限口径就是 2MB");

    let over = vec![b'<'; MAX_BYTES + 1];
    let err = fulltext::extract_bytes(&over, ARTICLE_URL).expect_err("超限应被拒绝");
    assert!(
        matches!(err, FulltextError::TooLarge { size, max } if size == MAX_BYTES + 1 && max == MAX_BYTES),
        "实际: {err}"
    );

    // 恰好等于上限：闸门必须放行（否则「正好 2MB」的合法页面会被误杀）。
    // 这堆字节不是 HTML，所以放行后的失败点是 NotHtml——正好证明它没被体积挡下。
    let at_limit = vec![b'x'; MAX_BYTES];
    let err = fulltext::extract_bytes(&at_limit, ARTICLE_URL).expect_err("不是 HTML");
    assert!(matches!(err, FulltextError::NotHtml), "实际: {err}");
}

// ---------------------------------------------------------------- 写回与刷新

#[test]
fn set_fulltext_writes_back_content_tokens_and_flag() {
    let (store, feed_id) = setup();
    let id = upsert_one(&store, feed_id, &mk_summary_entry("e1", "HTTP 消息签名", "一小段摘要"));

    let before = store.get_entry(id).unwrap().expect("条目在");
    assert!(before.needs_fulltext, "摘要型条目应先显示「获取全文」");

    let out = fulltext::extract(TYPICAL, ARTICLE_URL).unwrap();
    store
        .set_fulltext(id, &out.content_html, &out.content_text)
        .expect("写回应成功");

    let after = store.get_entry(id).unwrap().expect("条目在");
    assert_eq!(after.content_html.as_deref(), Some(out.content_html.as_str()));
    assert_eq!(after.content_text.as_deref(), Some(out.content_text.as_str()));
    assert!(!after.needs_fulltext, "抓到全文后不该再显示按钮");

    // 新正文必须马上可搜：检索字段是写回时重算的（否则「抓到了但搜不到」）
    let hits = store.search("签名基", 10).unwrap();
    assert_eq!(hits.len(), 1, "只有这篇含该词组");
    assert_eq!(hits[0].id, id);

    // 写一个不存在的条目：给可读错误，而不是静默成功
    let err = store.set_fulltext(9999, "<p>x</p>", "x").unwrap_err();
    assert!(err.to_string().contains("不存在"), "实际: {err}");
}

#[test]
fn refresh_keeps_fetched_fulltext_and_updates_metadata_only() {
    let (store, feed_id) = setup();
    let mut entry = mk_summary_entry("e1", "原标题", "原摘要");
    let id = upsert_one(&store, feed_id, &entry);

    let out = fulltext::extract(TYPICAL, ARTICLE_URL).unwrap();
    store.set_fulltext(id, &out.content_html, &out.content_text).unwrap();

    // 源侧内容全变了（标题、摘要、源给的正文都换了一套）
    entry.title = "改过的标题".to_string();
    entry.summary = Some("改过的摘要".to_string());
    entry.content_html = Some("<p>源里的新摘要</p>".to_string());
    entry.content_text = Some("源里的新摘要".to_string());
    let stats = store.upsert_entries(feed_id, &[entry.clone()]).unwrap();
    assert_eq!(stats.updated, 1);

    let row = store.get_entry(id).unwrap().unwrap();
    assert_eq!(row.title, "改过的标题", "元数据照常更新");
    assert_eq!(row.summary.as_deref(), Some("改过的摘要"));
    assert_eq!(
        row.content_html.as_deref(),
        Some(out.content_html.as_str()),
        "已抓正文不能被刷新覆盖回源摘要"
    );
    assert_eq!(row.content_text.as_deref(), Some(out.content_text.as_str()));
    assert!(!row.needs_fulltext);

    // 指纹已跟着源更新：再刷一次应是 unchanged（不会因为「正文常驻」而反复写库）
    let stats = store.upsert_entries(feed_id, &[entry]).unwrap();
    assert_eq!(stats.updated, 0);
    assert_eq!(stats.unchanged, 1);
}

#[test]
fn refresh_still_updates_content_when_never_fetched() {
    let (store, feed_id) = setup();
    let entry = mk_summary_entry("e1", "标题", "旧摘要");
    let id = upsert_one(&store, feed_id, &entry);

    // 没抓过全文的条目：刷新照旧更新正文（保护上面那条 CASE 分支不误伤普通更新）
    let mut changed = entry;
    changed.content_html = Some("<p>源更新了正文</p>".to_string());
    changed.content_text = Some("源更新了正文".to_string());
    store.upsert_entries(feed_id, &[changed]).unwrap();

    let row = store.get_entry(id).unwrap().unwrap();
    assert_eq!(row.content_text.as_deref(), Some("源更新了正文"));
}

// ---------------------------------------------------------------- needs_fulltext 判定

#[test]
fn needs_fulltext_judgement() {
    let url = Some(ARTICLE_URL);
    let short = "摘".repeat(SUMMARY_TEXT_MIN_CHARS - 1);
    let long = "文".repeat(SUMMARY_TEXT_MIN_CHARS);

    // 摘要型：正文缺失 / 明显偏短（阈值的一线之隔都验）
    assert!(fulltext::is_summary_entry(url, None, None, false), "没有正文");
    assert!(
        fulltext::is_summary_entry(url, Some("<p>摘要</p>"), Some("摘要"), false),
        "只有一小段摘要"
    );
    assert!(
        fulltext::is_summary_entry(url, None, Some(&short), false),
        "比阈值短一个字也算摘要"
    );

    // 全文型：够长就不再显示按钮
    assert!(
        !fulltext::is_summary_entry(url, None, Some(&long), false),
        "正好到阈值即视为全文"
    );

    // 抓不了 / 已抓过，都不该显示按钮
    assert!(!fulltext::is_summary_entry(None, None, None, false), "没有原文地址");
    assert!(
        !fulltext::is_summary_entry(Some("   "), None, None, false),
        "空白地址等于没有地址"
    );
    assert!(
        !fulltext::is_summary_entry(url, None, Some(&short), true),
        "已抓过的条目不再显示按钮（重开零网络）"
    );

    // content_text 缺失时按 content_html 现算长度
    let long_html = format!("<p>{long}</p>");
    assert!(!fulltext::is_summary_entry(url, Some(&long_html), None, false));
    let short_html = format!("<p>{}</p>", "摘".repeat(SUMMARY_TEXT_MIN_CHARS - 1));
    assert!(fulltext::is_summary_entry(url, Some(&short_html), None, false));

    // 长摘要标记（lkml.org/rss.php 约定）：超过长度阈值但带 `(Summary)` 前缀
    let lkml_summary = format!(
        "Guangshuo Li writes: (Summary) {}",
        "正文".repeat(SUMMARY_TEXT_MIN_CHARS)
    );
    assert!(
        fulltext::is_summary_entry(url, None, Some(&lkml_summary), false),
        "lkml 长摘要（超阈值）也该显示按钮"
    );
    // 标记必须在头部：正文中途出现 `(Summary)` 不算
    let marker_late = format!(
        "{} Guangshuo Li writes: (Summary)",
        "正文".repeat(SUMMARY_TEXT_MIN_CHARS)
    );
    assert!(!fulltext::is_summary_entry(url, None, Some(&marker_late), false),
        "标记在 96 字窗口之外不算摘要");
    // 普通长文不带标记：不显示按钮
    let plain_long = "这是完整的长文。".repeat(100);
    assert!(!fulltext::is_summary_entry(url, None, Some(&plain_long), false));
}

/// Anubis 反爬质询页：拒绝提取而不是把质询文本写回覆盖真实摘要
#[test]
fn bot_challenge_page_is_rejected_not_extracted() {
    const CHALLENGE: &str = include_str!("fixtures/fulltext/anubis-challenge.html");
    let err = fulltext::extract(CHALLENGE, ARTICLE_URL).unwrap_err();
    assert!(matches!(err, FulltextError::BotChallenge), "实际: {err:?}");
    // 提示必须引导用户去浏览器，而不是让他们以为重试能好
    let msg = err.to_string();
    assert!(msg.contains("浏览器"), "提示语: {msg}");
}

#[test]
fn store_reports_needs_fulltext_for_summary_rows_only() {
    let (store, feed_id) = setup();

    // ① 摘要型（正文缺失）→ true
    let summary_id = upsert_one(&store, feed_id, &mk_summary_entry("s1", "摘要型", "摘要"));

    // ② 全文型（源给了长正文）→ false
    let mut full = mk_summary_entry("s2", "全文型", "摘要");
    full.content_html = Some(format!("<p>{}</p>", "文".repeat(SUMMARY_TEXT_MIN_CHARS)));
    full.content_text = Some("文".repeat(SUMMARY_TEXT_MIN_CHARS));
    let full_id = upsert_one(&store, feed_id, &full);

    // ③ 没有原文地址 → false（抓不了就别给按钮）
    let mut no_url = mk_summary_entry("s3", "无地址", "摘要");
    no_url.url = None;
    let no_url_id = upsert_one(&store, feed_id, &no_url);

    assert!(store.get_entry(summary_id).unwrap().unwrap().needs_fulltext);
    assert!(!store.get_entry(full_id).unwrap().unwrap().needs_fulltext);
    assert!(!store.get_entry(no_url_id).unwrap().unwrap().needs_fulltext);

    // 列表行不带正文，一律不判定（否则每条都会被当成摘要型）
    let listed = store.list_entries(&EntryQuery::default()).unwrap();
    assert_eq!(listed.len(), 3);
    assert!(
        listed.iter().all(|r| !r.needs_fulltext),
        "列表行不应携带全文判定结果"
    );
}

#[test]
fn schema_reaches_fulltext_migration() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(
        store.schema_version().unwrap() as usize,
        MIGRATIONS.len(),
        "迁移应把 schema 推到最新版本"
    );
    assert!(MIGRATIONS.len() >= 7, "全文标记位是第 7 条迁移");
}
