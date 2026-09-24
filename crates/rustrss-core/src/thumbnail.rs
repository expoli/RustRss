use url::Url;

pub(crate) fn resolve(raw: &str, base: Option<&str>) -> Option<String> {
    let raw = quick_xml::escape::unescape(raw).ok()?;
    let raw = raw.trim();
    let url = match Url::parse(raw) {
        Ok(url) => url,
        Err(_) => Url::parse(base?).ok()?.join(raw).ok()?,
    };
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(url.into())
}

pub(crate) fn first_image(html: &str, base: Option<&str>) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find('<') {
        let start = cursor + relative;
        if lower[start..].starts_with("<!--") {
            cursor = lower[start + 4..].find("-->").map(|end| start + 4 + end + 3)?;
            continue;
        }
        let end = tag_end(html, start)?;
        let tag = &html[start + 1..end];
        let name_end = tag.find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>').unwrap_or(tag.len());
        let name = &tag[..name_end];
        if name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style") {
            let close = format!("</{name}");
            cursor = lower[end + 1..].find(&close).map(|at| end + 1 + at)?;
            continue;
        }
        if name.eq_ignore_ascii_case("img") {
            if let Some(src) = attribute(&tag[name_end..], "src") {
                if let Some(url) = resolve(src, base) {
                    return Some(url);
                }
            }
        }
        cursor = end + 1;
    }
    None
}

fn tag_end(html: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, ch) in html[start..].char_indices().skip(1) {
        match (quote, ch) {
            (Some(q), c) if q == c => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => return Some(start + offset),
            _ => {}
        }
    }
    None
}

fn attribute<'a>(attrs: &'a str, wanted: &str) -> Option<&'a str> {
    let bytes = attrs.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        while pos < bytes.len() && (bytes[pos].is_ascii_whitespace() || bytes[pos] == b'/') { pos += 1; }
        let start = pos;
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && !matches!(bytes[pos], b'=' | b'/') { pos += 1; }
        if start == pos { pos += 1; continue; }
        let name = &attrs[start..pos];
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        if pos == bytes.len() || bytes[pos] != b'=' { continue; }
        pos += 1;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        if pos == bytes.len() { break; }
        let (value_start, value_end) = if matches!(bytes[pos], b'\'' | b'"') {
            let quote = bytes[pos];
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos] != quote { pos += 1; }
            let end = pos;
            pos = (pos + 1).min(bytes.len());
            (start, end)
        } else {
            let start = pos;
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() { pos += 1; }
            (start, pos)
        };
        if name.eq_ignore_ascii_case(wanted) { return attrs.get(value_start..value_end); }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_image_skips_comments_and_script_and_resolves_relative_url() {
        let html = "<!-- <img src='bad.jpg'> --><script>\"<img src=bad.jpg>\"</script><IMG alt=x SRC='../a.jpg?x=1&amp;y=2'>";
        assert_eq!(first_image(html, Some("https://example.test/feed/item")),
            Some("https://example.test/a.jpg?x=1&y=2".into()));
    }

    #[test]
    fn unsafe_or_credentialed_image_urls_are_rejected() {
        for src in ["javascript:alert(1)", "data:image/png;base64,abc", "file:///tmp/a.png", "https://user:pass@example.test/a.png"] {
            assert_eq!(resolve(src, None), None, "accepted {src}");
        }
        assert_eq!(resolve("/a.png", None), None);
    }
}
