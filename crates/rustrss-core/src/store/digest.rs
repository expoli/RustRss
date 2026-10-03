//! 每日日报（Daily Digest）的数据访问：范围解析、报告读取、检查点状态对比。
//!
//! 设计：`.chorus/specs/rss-reader/2026-10-03-daily-digest/design.md` §5/§6。
//! 红线：状态检查走 `digest_entry_meta` 覆盖索引，不扫 `entries` 正文表；
//! 报告正文（digest_bodies）只由单取接口读取，列表/计数不碰。
//! 生成流水线在 `ai::digest`（后续阶段），本模块只管数据面。

use super::{Store, TagBrief};
use rusqlite::params;

/// 日报范围：源标签多选（OR）；空 = 全部订阅源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestScope {
    /// 用户选中的源标签 id（原样，未排序）
    pub tag_ids: Vec<i64>,
    /// 规范化 scope_key（排序后），与 digests.scope_key 同构："all" / "tags:1,3"
    pub key: String,
    /// OR 匹配到的订阅源 id（空标签选择 = None 表示全部）
    pub feed_ids: Option<Vec<i64>>,
}

impl DigestScope {
    /// 从设置里的标签 id 数组解析范围。`tag_ids` 顺序无关。
    pub fn resolve(store: &Store, tag_ids: &[i64]) -> Result<Self, super::StoreError> {
        let mut ids: Vec<i64> = tag_ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return Ok(Self { tag_ids: vec![], key: "all".into(), feed_ids: None });
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT DISTINCT feed_id FROM feed_tags WHERE tag_id IN ({placeholders})"
        );
        let conn = &store.conn;
        let mut stmt = conn.prepare(&sql)?;
        let feed_ids: Vec<i64> = stmt
            .query_map(rusqlite::params_from_iter(ids.iter().copied()), |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let key = format!("tags:{}", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
        Ok(Self { tag_ids: ids, key, feed_ids: Some(feed_ids) })
    }
}

/// 日报报告（头 + 正文 + 素材清单）。
#[derive(Debug, Clone, PartialEq)]
pub struct DigestReport {
    pub date: String,
    pub scope_key: String,
    pub scope_json: String,
    pub profile_key: String,
    /// 内容快照（输入清单冻结时刻）
    pub checkpoint_at: i64,
    /// 生成完成时刻（仅展示）
    pub generated_at: i64,
    pub manifest_hash: String,
    /// 成品 Markdown（渲染与导出共用）
    pub markdown: String,
    pub article_count: i64,
    pub cache_hits: i64,
    pub items: Vec<DigestItemRef>,
}

/// 素材清单快照行（无外键：文章删除后历史报告仍可读）。
#[derive(Debug, Clone, PartialEq)]
pub struct DigestItemRef {
    pub instance_id: i64,
    pub entry_id: i64,
    pub feed_id: i64,
    pub source_revision: i64,
    pub effective_at: i64,
    pub input_hash: String,
    pub title: String,
}

/// 与已存报告的更新对比（状态条的类型化计数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DigestUpdateStatus {
    pub has_report: bool,
    pub added: i64,
    pub changed: i64,
    pub removed: i64,
    pub checkpoint_at: i64,
    pub article_count: i64,
    /// 素材候选总数（当前范围 + 日期窗）
    pub candidate_count: i64,
}

impl Store {
    /// 订阅源的标签（名称序）——打标对话框与 chips 渲染口。
    pub fn feed_tags(&self, feed_id: i64) -> super::Result<Vec<TagBrief>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.name
               FROM feed_tags ft JOIN tags t ON t.id = ft.tag_id
              WHERE ft.feed_id = ?1
              ORDER BY t.name COLLATE NOCASE, t.id",
        )?;
        let rows = stmt.query_map(params![feed_id], |r| {
            Ok(TagBrief { id: r.get(0)?, name: r.get(1)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 整体替换订阅源的标签关联（与文章打标同一交互语义：勾选集 = 最终集）。
    /// 单事务：要么全换成功，要么不动（审阅红线 5）。
    pub fn set_feed_tags(&self, feed_id: i64, tag_ids: &[i64]) -> super::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM feed_tags WHERE feed_id = ?1", params![feed_id])?;
        for tag_id in tag_ids {
            tx.execute(
                "INSERT INTO feed_tags(feed_id, tag_id) VALUES (?1, ?2)",
                params![feed_id, tag_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 有日报的日期（历史入口用；不读正文）。
    pub fn digest_days(&self) -> super::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT report_day FROM digests ORDER BY report_day DESC")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 读一份报告（头 + 正文 + 素材清单）。无报告返回 None。
    pub fn digest_report(
        &self,
        date: &str,
        scope_key: &str,
    ) -> super::Result<Option<DigestReport>> {
        let head = self
            .conn
            .query_row(
                "SELECT d.scope_json, d.profile_key, d.checkpoint_at, d.generated_at,
                        d.manifest_hash, b.markdown, d.article_count,
                        COALESCE(json_extract(d.stats_json, '$.cache_hits'), 0)
                   FROM digests d JOIN digest_bodies b ON b.digest_id = d.id
                  WHERE d.report_day = ?1 AND d.scope_key = ?2
                    AND d.generated_at IS NOT NULL",
                params![date, scope_key],
                |r| {
                    Ok(DigestReport {
                        date: date.into(),
                        scope_key: scope_key.into(),
                        scope_json: r.get(0)?,
                        profile_key: r.get(1)?,
                        checkpoint_at: r.get(2)?,
                        generated_at: r.get(3)?,
                        manifest_hash: r.get(4)?,
                        markdown: r.get(5)?,
                        article_count: r.get(6)?,
                        cache_hits: r.get(7)?,
                        items: vec![],
                    })
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(super::StoreError::Sqlite(other)),
            })?;
        let Some(mut report) = head else { return Ok(None) };

        let digest_id: i64 = self.conn.query_row(
            "SELECT id FROM digests WHERE report_day = ?1 AND scope_key = ?2",
            params![date, scope_key],
            |r| r.get(0),
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT instance_id, entry_id, feed_id, source_revision, effective_at,
                    input_hash, title
               FROM digest_items WHERE digest_id = ?1
              ORDER BY effective_at DESC, instance_id",
        )?;
        report.items = stmt
            .query_map(params![digest_id], |r| {
                Ok(DigestItemRef {
                    instance_id: r.get(0)?,
                    entry_id: r.get(1)?,
                    feed_id: r.get(2)?,
                    source_revision: r.get(3)?,
                    effective_at: r.get(4)?,
                    input_hash: r.get(5)?,
                    title: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(report))
    }

    /// 与已存报告的更新对比：走 `digest_entry_meta` 覆盖索引与报告素材清单，
    /// 不读 entries 正文（红线 1/2 的覆盖路径，EXPLAIN 断言见 tests/digest_schema.rs）。
    ///
    /// `day_start/day_end`：本地自然日对应的 UTC 半开区间，由调用方（commands）按
    /// 当地时区计算——core 不读系统时区。
    pub fn digest_status(
        &self,
        day_start: i64,
        day_end: i64,
        scope: &DigestScope,
    ) -> super::Result<DigestUpdateStatus> {
        let conn = &self.conn;
        // 当前候选集（范围内条目）的 (instance_id, source_revision)。
        let feed_filter = |sql: &mut String| {
            if let Some(ids) = &scope.feed_ids {
                let placeholders = vec!["?"; ids.len()].join(",");
                sql.push_str(&format!(" AND feed_id IN ({placeholders})"));
            }
        };
        let mut candidate_sql =
            String::from("SELECT instance_id, source_revision FROM digest_entry_meta \
                          WHERE effective_at >= ?1 AND effective_at < ?2");
        feed_filter(&mut candidate_sql);
        let mut stmt = conn.prepare(&candidate_sql)?;
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(day_start), Box::new(day_end)];
        if let Some(ids) = &scope.feed_ids {
            for id in ids {
                bind.push(Box::new(*id));
            }
        }
        let current: std::collections::HashMap<i64, i64> = stmt
            .query_map(rusqlite::params_from_iter(bind.iter().map(|b| b.as_ref())), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
        let candidate_count = current.len() as i64;

        // 已存报告的素材清单。
        let Some(report) = self.latest_report_head(day_start, day_end, scope)? else {
            return Ok(DigestUpdateStatus {
                has_report: false,
                candidate_count,
                ..Default::default()
            });
        };
        let mut stmt = self.conn.prepare(
            "SELECT instance_id, source_revision FROM digest_items WHERE digest_id = ?1",
        )?;
        let stored: std::collections::HashMap<i64, i64> = stmt
            .query_map(params![report], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;

        let mut status = DigestUpdateStatus {
            has_report: true,
            checkpoint_at: report,
            article_count: stored.len() as i64,
            candidate_count,
            ..Default::default()
        };
        for (instance, revision) in &current {
            match stored.get(instance) {
                None => status.added += 1,
                Some(old) if old != revision => status.changed += 1,
                Some(_) => {}
            }
        }
        status.removed = stored.keys().filter(|k| !current.contains_key(*k)).count() as i64;
        Ok(status)
    }

    /// 最近一份成功报告头（返回 digests.id），供状态对比取素材清单。
    fn latest_report_head(
        &self,
        day_start: i64,
        day_end: i64,
        scope: &DigestScope,
    ) -> super::Result<Option<i64>> {
        // 无标签选择 = 全部订阅源（scope_key='all'）；有选择时 key 已规范化。
        let scope_key = if scope.feed_ids.is_some() { scope.key.as_str() } else { "all" };
        let conn = &self.conn;
        let mut stmt = conn.prepare(
            "SELECT id FROM digests
              WHERE day_start_at = ?1 AND day_end_at = ?2 AND scope_key = ?3
                AND generated_at IS NOT NULL
              ORDER BY generated_at DESC, id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![day_start, day_end, scope_key], |r| r.get(0))?;
        Ok(rows.next().transpose()?)
    }
}

/// 本地自然日的 UTC 半开区间 `[day_start, next_day_start)`。
///
/// 日界由本地时区计算（DST 可产生 23/25 小时一天，禁止 +86400）；core 不读
/// 系统时区之外的全局状态，调用方传日期字符串即可。
pub fn local_day_bounds(date: &str) -> super::Result<(i64, i64)> {
    use chrono::{NaiveDate, TimeZone};
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|e| super::StoreError::Invalid(format!("日期格式应为 YYYY-MM-DD：{e}")))?;
    let to_ts = |n: chrono::NaiveDateTime| {
        chrono::Local
            .from_local_datetime(&n)
            .earliest()
            .map(|t| t.timestamp())
            .ok_or_else(|| super::StoreError::Invalid(format!("本地时间不存在：{n}（DST 跳变）")))
    };
    let start = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| super::StoreError::Invalid(format!("无效日期 {date}")))?;
    let next = day
        .succ_opt()
        .ok_or_else(|| super::StoreError::Invalid(format!("日期溢出：{date}")))?;
    let end = next
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| super::StoreError::Invalid("无效的次日".into()))?;
    Ok((to_ts(start)?, to_ts(end)?))
}
