//! 开发期回填逻辑（原 v13 迁移的一部分）。
//!
//! **压平后不再被调用**：基线建的是新库，没有历史数据需要回填。保留它的理由是
//! 将来若要兼容某个已发布库，这里有现成实现可用（含 CJK 单字补全与缩略图回填）；
//! 配套单测保证它仍然可用，不会被无声腐化。
//!
//! 两个回填项：
//! - `search_tokens`：补上 CJK 单字（旧分词只做 bigram，单字查询会退化成全表扫）；
//! - `thumbnail_url`：从正文首图回填（旧数据没有缩略图列时的唯一来源）。
use rusqlite::{params, Connection};

use super::tokens;

/// 回填批次读取的一行：`(id, search_tokens, url, content_html, thumbnail_url)`。
type BackfillRow = (i64, String, Option<String>, Option<String>, Option<String>);

/// 回填 `search_tokens` 的 CJK 单字与缺失的 `thumbnail_url`。
///
/// 分批（每批 5 行）遍历，避免一次性把整库读进内存；返回被改写的行数。
pub fn apply_pre_release_backfill(conn: &Connection) -> rusqlite::Result<usize> {
    let mut cursor = i64::MIN;
    let mut touched = 0usize;
    loop {
        let batch: Vec<BackfillRow> = {
            let mut statement = conn.prepare(
                "SELECT id, search_tokens, url, content_html, thumbnail_url FROM entries WHERE id > ? ORDER BY id LIMIT 5",
            )?;
            let rows = statement
                .query_map([cursor], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        if batch.is_empty() {
            break;
        }
        for (id, old_tokens, base, content_html, old_thumbnail) in batch {
            let new_tokens = tokens::with_cjk_unigrams(&old_tokens);
            let tokens_changed = new_tokens != old_tokens;
            // 同值零写（红线 #7）：已有缩略图且新值相同就不碰；没有新候选图也不动。
            let new_thumbnail = content_html
                .as_deref()
                .and_then(|html| crate::thumbnail::first_image(html, base.as_deref()));
            let thumbnail_changed = match (&new_thumbnail, &old_thumbnail) {
                (Some(new), Some(old)) => new != old,
                (Some(_), None) => true,
                _ => false,
            };
            match (tokens_changed, thumbnail_changed) {
                (true, true) => {
                    conn.execute(
                        "UPDATE entries SET search_tokens=?, thumbnail_url=? WHERE id=?",
                        params![new_tokens, new_thumbnail, id],
                    )?;
                    touched += 1;
                }
                (true, false) => {
                    conn.execute(
                        "UPDATE entries SET search_tokens=? WHERE id=?",
                        params![new_tokens, id],
                    )?;
                    touched += 1;
                }
                (false, true) => {
                    conn.execute(
                        "UPDATE entries SET thumbnail_url=? WHERE id=?",
                        params![new_thumbnail, id],
                    )?;
                    touched += 1;
                }
                (false, false) => {}
            }
            cursor = id;
        }
    }
    Ok(touched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    /// 保留件的可用性：旧数据（无单字、无缩略图）经回填后两者都补上，幂等。
    #[test]
    fn backfill_adds_unigrams_and_thumbnail_and_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn
            .execute_batch(
                r#"
                INSERT INTO feeds(id,url,title,created_at) VALUES(1,'https://e.test/rss','F',1);
                INSERT INTO entries(id,feed_id,stable_id,id_origin,title,url,content_html,content_text,search_tokens,fetched_at)
                VALUES(1,1,'s1','source_data','标题','https://e.test/a','<p>正文</p><img src="/cover.png">','正文','标题',1);
                "#,
            )
            .unwrap();

        let changed = apply_pre_release_backfill(&store.conn).unwrap();
        assert_eq!(changed, 1, "首次回填应改写 1 行");

        let (tokens, thumbnail): (String, Option<String>) = store
            .conn
            .query_row(
                "SELECT search_tokens, thumbnail_url FROM entries WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(
            tokens.split_whitespace().any(|t| t == "标") && tokens.split_whitespace().any(|t| t == "题"),
            "CJK 单字应补入: {tokens}"
        );
        assert_eq!(thumbnail.as_deref(), Some("https://e.test/cover.png"));

        // 幂等：再跑一次不改写任何行
        assert_eq!(apply_pre_release_backfill(&store.conn).unwrap(), 0);
    }
}
