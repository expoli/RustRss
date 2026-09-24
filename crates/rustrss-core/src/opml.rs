//! OPML 导入 / 导出。
//!
//! 迁移必备：从现有阅读器搬过来（或搬出去）。刻意保持「只做数据搬运」：
//! - 导出包含文件夹结构（OPML 的 outline 嵌套）；
//! - 导入按 `xmlUrl` 去重（已存在的源不会被重复添加，也不会被搬到别的文件夹）；
//! - 嵌套文件夹压平成 `父/子` 这种名字（我们的 folders 表是平的，够用且好实现）。

use crate::store::Store;

/// 导入结果：数字要能对上，否则用户无从判断「为什么只进来一半」
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ImportReport {
    pub feeds_added: usize,
    pub feeds_skipped: usize,
    pub folders_created: usize,
    /// 忽略的 outline（既没有 xmlUrl 也没有子节点，例如纯文字大纲）
    pub outlines_ignored: usize,
    /// 本次真正新增的 feed id（按 OPML 里的出现顺序）。
    /// 调用方据此只抓新导入的源，而不是导入后立刻全量再刷一遍。
    pub added_feed_ids: Vec<i64>,
}

/// 导出全部订阅为 OPML 2.0
pub fn export(store: &Store) -> Result<String, crate::store::StoreError> {
    let folders = store.list_folders()?;
    let feeds: Vec<ExportFeed> = store
        .list_feeds()?
        .into_iter()
        .map(|f| ExportFeed {
            url: f.url,
            title: f.title,
            site_url: f.site_url,
            folder_id: f.folder_id,
        })
        .collect();
    // 导出顺序与只读通路对齐（按标题、再按 URL）：否则两条路径对同一批订阅产出不同行序，
    // 「只读导出 == 正常导出」就只能对恰好同序的夹具成立（评审 NOTE）。
    let mut feeds = feeds;
    feeds.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.url.cmp(&b.url)));
    Ok(render(&folders, &feeds))
}

/// **只读**导出：给「库不兼容、被拒绝打开」的场景用（老开发库不能被打开成 `Store`，
/// 但订阅列表仍要能救出来）。
///
/// 只读打开、只查 v1 就有的最小列（`feeds` 的 url/title/site_url/folder_id 与 `folders` 的
/// id/name），因此对旧链任意版本的库都能工作；导出过程**不写一个字节**（不给候选库
/// 建 WAL 边车、不写 PRAGMA）。
/// 与正常导出的差异（如实记下、不假装等价）：这里用**源站标题**（旧库不一定有
/// `custom_title`）且按「标题, URL」定序；库里存在自定义标题或手动排序时，标题与行序
/// 会与应用的导出不同——这是「抢救订阅」的路径。
pub fn export_read_only(path: &std::path::Path) -> Result<String, crate::store::StoreError> {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| {
        crate::store::StoreError::Invalid(format!("无法只读打开 {}: {e}", path.display()))
    })?;
    let folders: Vec<(i64, String)> = {
        let mut st = conn.prepare("SELECT id, name FROM folders ORDER BY position, name")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let feeds: Vec<ExportFeed> = {
        let mut st =
            conn.prepare("SELECT url, title, site_url, folder_id FROM feeds ORDER BY title, url")?;
        let rows = st.query_map([], |r| {
            Ok(ExportFeed {
                url: r.get(0)?,
                title: r.get(1)?,
                site_url: r.get(2)?,
                folder_id: r.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    Ok(render(&folders, &feeds))
}

/// 导出所需的最小字段集（`Store` 与只读路径共用）
struct ExportFeed {
    url: String,
    title: String,
    site_url: Option<String>,
    folder_id: Option<i64>,
}

fn render(folders: &[(i64, String)], feeds: &[ExportFeed]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<opml version=\"2.0\">\n");
    out.push_str("  <head>\n    <title>RustRss 订阅</title>\n  </head>\n");
    out.push_str("  <body>\n");

    // 先按文件夹输出，再输出未归类的
    for (folder_id, folder_name) in folders {
        let members: Vec<_> = feeds
            .iter()
            .filter(|f| f.folder_id == Some(*folder_id))
            .collect();
        if members.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "    <outline text=\"{}\" title=\"{}\">\n",
            escape_attr(folder_name),
            escape_attr(folder_name)
        ));
        for feed in members {
            out.push_str(&outline_for_feed(feed, 6));
        }
        out.push_str("    </outline>\n");
    }
    for feed in feeds.iter().filter(|f| f.folder_id.is_none()) {
        out.push_str(&outline_for_feed(feed, 4));
    }

    out.push_str("  </body>\n</opml>\n");
    out
}

fn outline_for_feed(feed: &ExportFeed, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let title = escape_attr(&feed.title);
    let xml_url = escape_attr(&feed.url);
    match feed.site_url.as_deref() {
        Some(site) if !site.trim().is_empty() => format!(
            "{pad}<outline type=\"rss\" text=\"{title}\" title=\"{title}\" xmlUrl=\"{xml_url}\" htmlUrl=\"{}\"/>\n",
            escape_attr(site)
        ),
        _ => format!(
            "{pad}<outline type=\"rss\" text=\"{title}\" title=\"{title}\" xmlUrl=\"{xml_url}\"/>\n"
        ),
    }
}

/// 从 OPML 文本导入订阅。已存在的源会被跳过（按 xmlUrl 判断）。
pub fn import(store: &Store, xml: &str) -> Result<ImportReport, ImportError> {
    let outlines = parse(xml)?;
    let mut report = ImportReport::default();

    // 递归遍历：folders 沿路径累积，最终名字用 "/" 连接
    fn walk(
        store: &Store,
        nodes: &[Outline],
        parents: &[String],
        report: &mut ImportReport,
    ) -> Result<(), ImportError> {
        for node in nodes {
            match node.xml_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
                Some(url) => {
                    if store
                        .feed_id_by_url(url)
                        .map_err(|e| ImportError::Store(e.to_string()))?
                        .is_some()
                    {
                        report.feeds_skipped += 1;
                        continue;
                    }
                    let title = node.display_title();
                    let feed_id = store
                        .add_feed(url, Some(&title))
                        .map_err(|e| ImportError::Store(e.to_string()))?;
                    if let Some(html) = node.html_url.as_deref().filter(|h| !h.trim().is_empty()) {
                        let _ = store.update_feed_meta(feed_id, None, Some(html), None, None);
                    }
                    if !parents.is_empty() {
                        let folder_name = parents.join("/");
                        let before = store
                            .list_folders()
                            .map_err(|e| ImportError::Store(e.to_string()))?
                            .len();
                        let folder_id = store
                            .add_folder(&folder_name)
                            .map_err(|e| ImportError::Store(e.to_string()))?;
                        let after = store
                            .list_folders()
                            .map_err(|e| ImportError::Store(e.to_string()))?
                            .len();
                        if after > before {
                            report.folders_created += 1;
                        }
                        store
                            .assign_folder(feed_id, Some(folder_id))
                            .map_err(|e| ImportError::Store(e.to_string()))?;
                    }
                    report.feeds_added += 1;
                    report.added_feed_ids.push(feed_id);
                }
                None => {
                    if node.children.is_empty() {
                        // 纯文字大纲（没有订阅也没有子节点）：忽略但计数，避免"静默丢弃"
                        report.outlines_ignored += 1;
                        continue;
                    }
                    let mut path = parents.to_vec();
                    path.push(node.display_title());
                    walk(store, &node.children, &path, report)?;
                }
            }
        }
        Ok(())
    }

    walk(store, &outlines, &[], &mut report)?;
    Ok(report)
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("OPML 解析失败: {0}")]
    Xml(String),
    #[error("数据库错误: {0}")]
    Store(String),
}

/// OPML 里的一条 outline
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outline {
    pub text: Option<String>,
    pub title: Option<String>,
    pub xml_url: Option<String>,
    pub html_url: Option<String>,
    pub children: Vec<Outline>,
}

impl Outline {
    fn display_title(&self) -> String {
        self.title
            .as_deref()
            .or(self.text.as_deref())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("（无标题）")
            .to_string()
    }
}

/// 用 quick-xml 解析出 outline 树；遇到没有 `<opml>` 根的输入会报错而不是静默返回空。
fn parse(xml: &str) -> Result<Vec<Outline>, ImportError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<Outline> = Vec::new();
    let mut roots: Vec<Outline> = Vec::new();
    let mut saw_opml = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "opml" => saw_opml = true,
                "outline" => stack.push(outline_from(&e)),
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == "outline" {
                    let node = outline_from(&e);
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(node),
                        None => roots.push(node),
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == "outline" {
                    if let Some(node) = stack.pop() {
                        match stack.last_mut() {
                            Some(parent) => parent.children.push(node),
                            None => roots.push(node),
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(ImportError::Xml(format!("{e}（位置 {})", reader.buffer_position()))),
            _ => {}
        }
    }

    if !saw_opml {
        return Err(ImportError::Xml("输入里没有 <opml> 根节点，可能不是 OPML 文件".into()));
    }
    Ok(roots)
}

fn outline_from(e: &quick_xml::events::BytesStart<'_>) -> Outline {
    let mut node = Outline::default();
    for attr in e.attributes().flatten() {
        // 必须解码实体：OPML 里的标题常常长成 "LWN &amp; friends"，
        // 直接拿原始字节会把 &amp; 原样显示给用户（测试拓出了这条）。
        // quick-xml ≥0.42 起属性值本身就是 UTF-8 字符串，`unescape_value()` 已废弃，
        // 改用等价的 `normalized_value()`（同样是 Implicit1_0 + 预定义实体解析）。
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map(|v| v.to_string())
            .unwrap_or_else(|_| attr.value.to_string());
        match attr.key.as_ref() {
            "text" => node.text = Some(value),
            "title" => node.title = Some(value),
            "xmlUrl" | "xmlurl" => node.xml_url = Some(value),
            "htmlUrl" | "htmlurl" => node.html_url = Some(value),
            _ => {}
        }
    }
    node
}

/// XML 属性值转义（标题里常有 & " 这类字符，不转义就会生成非法 OPML）
fn escape_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
