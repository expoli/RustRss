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
}

/// 导出全部订阅为 OPML 2.0
pub fn export(store: &Store) -> Result<String, crate::store::StoreError> {
    let folders = store.list_folders()?;
    let feeds = store.list_feeds()?;

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<opml version=\"2.0\">\n");
    out.push_str("  <head>\n    <title>RustRss 订阅</title>\n  </head>\n");
    out.push_str("  <body>\n");

    // 先按文件夹输出，再输出未归类的
    for (folder_id, folder_name) in &folders {
        let members: Vec<_> = feeds.iter().filter(|f| f.folder_id == Some(*folder_id)).collect();
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
    Ok(out)
}

fn outline_for_feed(feed: &crate::store::FeedRow, indent: usize) -> String {
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
                b"opml" => saw_opml = true,
                b"outline" => stack.push(outline_from(&e)),
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"outline" {
                    let node = outline_from(&e);
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(node),
                        None => roots.push(node),
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"outline" {
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
        let value = attr
            .unescape_value()
            .map(|v| v.to_string())
            .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
        match attr.key.as_ref() {
            b"text" => node.text = Some(value),
            b"title" => node.title = Some(value),
            b"xmlUrl" | b"xmlurl" => node.xml_url = Some(value),
            b"htmlUrl" | b"htmlurl" => node.html_url = Some(value),
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
