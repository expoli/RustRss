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
    /// 成品 Markdown（导出共用）
    pub markdown: String,
    /// 结构化正文 JSON（overview + sections；UI 直接渲染，不经 Markdown 解析）
    pub content_json: String,
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
            .prepare(
                "SELECT DISTINCT report_day FROM digests
                  WHERE generated_at IS NOT NULL ORDER BY report_day DESC",
            )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 读一份报告（头 + 正文 + 素材清单）。无报告返回 None。
    pub fn digest_report(
        &self,
        date: &str,
        scope_key: &str,
    ) -> super::Result<Option<DigestReport>> {
        let conn = &self.conn;
        // 单一读事务：头、正文、素材清单绑定**同一个**报告身份（审核 P1-3）。
        // 变体选择：同日期+范围可有多个 profile 变体，取最近完成的一份。
        let tx = conn.unchecked_transaction()?;
        let head = tx
            .query_row(
                "SELECT d.id, d.scope_json, d.profile_key, d.checkpoint_at, d.generated_at,
                        d.manifest_hash, b.markdown, b.content_json, d.article_count,
                        COALESCE(json_extract(d.stats_json, '$.cache_hits'), 0)
                   FROM digests d
                   JOIN digest_bodies b
                     ON b.digest_id = d.id AND b.revision = d.revision
                  WHERE d.report_day = ?1 AND d.scope_key = ?2
                    AND d.generated_at IS NOT NULL
                  ORDER BY d.generated_at DESC, d.id DESC LIMIT 1",
                params![date, scope_key],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, i64>(8)?,
                        r.get::<_, i64>(9)?,
                    ))
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(super::StoreError::Sqlite(other)),
            })?;
        let Some((digest_id, scope_json, profile_key, checkpoint_at, generated_at,
                  manifest_hash, markdown, content_json, article_count, cache_hits)) = head
        else {
            return Ok(None)
        };

        let mut stmt = tx.prepare(
            "SELECT instance_id, entry_id, feed_id, source_revision, effective_at,
                    input_hash, title
               FROM digest_items WHERE digest_id = ?1
              ORDER BY effective_at DESC, instance_id",
        )?;
        let items = stmt
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
        drop(stmt);
        tx.commit()?;
        Ok(Some(DigestReport {
            date: date.into(),
            scope_key: scope_key.into(),
            scope_json,
            profile_key,
            checkpoint_at,
            generated_at,
            manifest_hash,
            markdown,
            content_json,
            article_count,
            cache_hits,
            items,
        }))
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
        let Some((report_id, checkpoint_at)) = self.latest_report_head(day_start, day_end, scope)?
        else {
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
            .query_map(params![report_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;

        let mut status = DigestUpdateStatus {
            has_report: true,
            checkpoint_at,
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
    ) -> super::Result<Option<(i64, i64)>> {
        // 无标签选择 = 全部订阅源（scope_key='all'）；有选择时 key 已规范化。
        let scope_key = if scope.feed_ids.is_some() { scope.key.as_str() } else { "all" };
        let conn = &self.conn;
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(checkpoint_at, 0) FROM digests
              WHERE day_start_at = ?1 AND day_end_at = ?2 AND scope_key = ?3
                AND generated_at IS NOT NULL
              ORDER BY generated_at DESC, id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![day_start, day_end, scope_key], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        Ok(rows.next().transpose()?)
    }
}

/// 本地自然日的 UTC 半开区间 `[day_start, next_day_start)`。
///
/// 日界由本地时区计算（DST 可产生 23/25 小时一天，禁止 +86400）；core 不读
/// 系统时区之外的全局状态，调用方传日期字符串即可。
/// 本地自然日的 UTC 边界与时区偏移（秒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigestDayBounds {
    pub start: i64,
    pub end: i64,
    pub offset_start: i32,
    pub offset_end: i32,
}

/// 日界 + 当地时区偏移：偏移由同一 `from_local_datetime` 结果推导，
/// 避免两处换算漂移（审核 P2-9；DST 用 earliest 有效时刻）。
pub fn local_day_bounds(date: &str) -> super::Result<DigestDayBounds> {
    use chrono::{NaiveDate, TimeZone};
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|e| super::StoreError::Invalid(format!("日期格式应为 YYYY-MM-DD：{e}")))?;
    let start_naive = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| super::StoreError::Invalid(format!("无效日期 {date}")))?;
    let next = day
        .succ_opt()
        .ok_or_else(|| super::StoreError::Invalid(format!("日期溢出：{date}")))?;
    let end_naive = next
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| super::StoreError::Invalid("无效的次日".into()))?;
    let to_local = |n: chrono::NaiveDateTime| {
        chrono::Local
            .from_local_datetime(&n)
            .earliest()
            .ok_or_else(|| super::StoreError::Invalid(format!("本地时间不存在：{n}（DST 跳变）")))
    };
    let start_local = to_local(start_naive)?;
    let end_local = to_local(end_naive)?;
    Ok(DigestDayBounds {
        start: start_local.timestamp(),
        end: end_local.timestamp(),
        offset_start: start_local.offset().local_minus_utc(),
        offset_end: end_local.offset().local_minus_utc(),
    })
}

// ---------------------------------------------------------------- 冻结与节点缓存

/// 素材清单的预算上界：超出按 effective_at 取最近的 N 篇，并显式标记截断
/// （设计：超预算先缩小范围/明确告知，不静默丢文章）。
pub const MANIFEST_MAX_ENTRIES: usize = 200;

/// 单组合成的要点字符预算（键点文本累计超过即切下一组）。
pub const GROUP_BUDGET_CHARS: usize = 9_000;

/// 冻结后的单篇素材（含实际送入模型的正文与输入哈希）。
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub entry_id: i64,
    pub instance_id: i64,
    pub feed_id: i64,
    pub source_revision: i64,
    pub effective_at: i64,
    pub title: String,
    pub body: String,
    pub input_hash: String,
    pub truncated: bool,
    pub summary_only: bool,
}

/// 冻结的素材清单 + 指纹。
#[derive(Debug, Clone)]
pub struct Manifest {
    pub entries: Vec<ManifestEntry>,
    pub hash: String,
    /// 成员身份+版本对的指纹（CAS 提交校验用；内容变化必然推进版本，
    /// 因此该指纹足以发现「新增/变化/移出」三类漂移）
    pub pairs_hash: String,
    /// 冻结时刻（checkpoint_at 的取值）
    pub frozen_at: i64,
    pub total_in_window: i64,
    pub truncated: bool,
}

pub(crate) fn sha256_hex(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for p in parts {
        hasher.update(p.as_bytes());
        hasher.update(b"\x1f");
    }
    format!("{:x}", hasher.finalize())
}

impl Store {
    /// 冻结素材清单：日期窗口 + 订阅源范围（feed_tags OR），
    /// 按 effective_at 取最近 [`MANIFEST_MAX_ENTRIES`] 篇，逐篇计算实际输入哈希。
    ///
    /// 候选 id 走 `digest_entry_meta` 覆盖索引；正文按 id 逐条取——只在冻结与
    /// 要点生成时发生，不是状态检查路径。
    pub fn freeze_manifest(
        &self,
        day_start: i64,
        day_end: i64,
        feed_ids: Option<&[i64]>,
    ) -> super::Result<Manifest> {
        let feed_filter_sql = |sql: &mut String| {
            if let Some(ids) = feed_ids {
                let placeholders = vec!["?"; ids.len()].join(",");
                sql.push_str(&format!(" AND m.feed_id IN ({placeholders})"));
            }
        };
        let mut id_sql = String::from(
            "SELECT m.instance_id, m.entry_id, m.feed_id, m.source_revision, m.effective_at
               FROM digest_entry_meta m
              WHERE m.effective_at >= ?1 AND m.effective_at < ?2",
        );
        feed_filter_sql(&mut id_sql);
        id_sql.push_str(" ORDER BY m.effective_at DESC, m.instance_id DESC");
        let mut stmt = self.conn.prepare(&id_sql)?;
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(day_start), Box::new(day_end)];
        if let Some(ids) = feed_ids {
            for id in ids {
                bind.push(Box::new(*id));
            }
        }
        let candidates: Vec<(i64, i64, i64, i64, i64)> = stmt
            .query_map(rusqlite::params_from_iter(bind.iter().map(|b| b.as_ref())), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let total_in_window = candidates.len() as i64;
        drop(stmt);

        // 预算截断：只取最近的 N 篇（明确的缩小范围，total_in_window 交代全量）
        let picked: Vec<_> = candidates.iter().take(MANIFEST_MAX_ENTRIES as usize).collect();

        let mut entries = Vec::new();
        for (instance_id, entry_id, feed_id, source_revision, effective_at) in &picked {
            let (instance_id, entry_id, feed_id, source_revision, effective_at) =
                (*instance_id, *entry_id, *feed_id, *source_revision, *effective_at);
            let row: Option<(String, Option<String>, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT title, content_text, summary FROM entries WHERE id = ?1",
                    params![entry_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            let Some((title, content_text, summary)) = row else { continue };
            let summary_only = content_text
                .as_deref()
                .map(|t| t.trim().is_empty())
                .unwrap_or(true);
            let body = content_text
                .filter(|t| !t.trim().is_empty())
                .or(summary)
                .unwrap_or_default();
            let (body, truncated) = crate::ai::prompt::prepare(&body);
            let input_hash = sha256_hex(&[&title, &body]);
            entries.push(ManifestEntry {
                entry_id,
                feed_id,
                instance_id,
                source_revision,
                effective_at,
                title,
                body,
                input_hash,
                truncated,
                summary_only,
            });
        }

        let total = entries.len() as i64;
        let hash = manifest_hash_of(&entries);
        let pairs_hash = manifest_pairs_hash_of(
            &entries.iter().map(|e| (e.instance_id, e.source_revision)).collect::<Vec<_>>(),
        );
        Ok(Manifest {
            entries,
            hash,
            pairs_hash,
            frozen_at: chrono::Utc::now().timestamp(),
            total_in_window,
            truncated: total_in_window > total,
        })
    }

    /// 归并/合成节点缓存读取（内容寻址）。
    pub fn digest_node_get(&self, node_key: &str) -> super::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT content FROM digest_node_cache WHERE node_key = ?1",
                params![node_key],
                |r| r.get(0),
            )
            .ok())
    }

    /// 归并/合成节点缓存写入。
    pub fn digest_node_put(
        &self,
        node_key: &str,
        kind: &str,
        content: &str,
        input_hash: &str,
    ) -> super::Result<()> {
        self.conn.execute(
            "INSERT INTO digest_node_cache(node_key, kind, content, input_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(node_key) DO UPDATE SET content = excluded.content,
                 input_hash = excluded.input_hash, created_at = excluded.created_at",
            params![node_key, kind, content, input_hash, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }
}

/// 生成配置身份（profile_key）：语言+provider/model+base_url+prompt 版本。
pub fn profile_key_of(language: &str, provider_model: &str, base_url: &str, prompt_version: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"digest-profile-v1");
    for p in [language, provider_model, base_url, prompt_version] {
        hasher.update(p.as_bytes());
        hasher.update(b"\x1e");
    }
    format!("{:x}", hasher.finalize())[..12].to_string()
}

/// 成员身份+版本对指纹（CAS 提交校验用）。
pub fn manifest_pairs_hash_of(pairs: &[(i64, i64)]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"digest-pairs-v1");
    for (instance, revision) in pairs {
        hasher.update(instance.to_le_bytes());
        hasher.update(revision.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// 清单指纹：实例身份 + 内容版本 + 输入哈希（语言不在其中——要点缓存键已含语言）。
pub fn manifest_hash_of(entries: &[ManifestEntry]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"digest-manifest-v1");
    for e in entries {
        hasher.update(e.instance_id.to_le_bytes());
        hasher.update(e.source_revision.to_le_bytes());
        hasher.update(e.input_hash.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

impl Store {
    /// 提交一份完整日报（单事务）：槽位推进（revision+1 + 检查点/统计）、
    /// 写正文（绑定同 revision）、替换素材清单。旧报告在事务外保留语义由
    /// 「失败/取消不调用本函数」保证。
    ///
    /// `manifest_hash` 用于提交 CAS：与调用方冻结时不一致说明素材又变了，
    /// 返回 `StoreError::Invalid`（调用方提示「素材又变了，请再次更新」）。
    #[allow(clippy::too_many_arguments)]
    pub fn commit_digest_report(
        &self,
        report_day: &str,
        timezone_label: &str,
        day_start: i64,
        day_end: i64,
        scope_key: &str,
        scope_json: &str,
        profile_key: &str,
        profile_json: &str,
        manifest_hash: &str,
        expected_pairs_hash: &str,
        checkpoint_at: i64,
        utc_offset_start: i32,
        utc_offset_end: i32,
        content_json: &str,
        markdown: &str,
        schema_ver: i64,
        stats_json: &str,
        article_count: i64,
        items: &[super::digest::ManifestEntry],
    ) -> super::Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let now = chrono::Utc::now().timestamp();
        // 提交 CAS（审核 P1-2）：事务内重算当前「成员身份+版本对」指纹，
        // 与冻结时不一致 = 生成期间素材又变了 → 拒绝覆盖旧报告。
        // 闭包作用域兜住 stmt 借用：错误路径不再把借用带出块外（E0597）
        let current_pairs: Vec<(i64, i64)> = (|| {
            let mut stmt = tx.prepare(
                "SELECT instance_id, source_revision FROM digest_entry_meta
                  WHERE effective_at >= ?1 AND effective_at < ?2",
            )?;
            let rows = stmt.query_map(params![day_start, day_end], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })()
        .map_err(super::StoreError::Sqlite)?;
        if manifest_pairs_hash_of(&current_pairs) != expected_pairs_hash {
            return Err(super::StoreError::Invalid(format!(
                "日报素材在生成期间又发生了变化，请再次「更新日报」"
            )));
        }
        tx.execute(
            "INSERT INTO digests(report_day, timezone_label, day_start_at, day_end_at,
                 utc_offset_start, utc_offset_end, date_basis, scope_key, scope_json,
                 profile_key, profile_json, revision, checkpoint_at, generated_at,
                 manifest_hash, article_count, stats_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'published_or_first_seen_v1', ?7, ?8,
                 ?9, ?10, 1, ?11, ?12, ?13, ?14, ?15, ?12)
             ON CONFLICT(report_day, day_start_at, day_end_at, date_basis,
                 scope_key, profile_key) DO UPDATE SET
                 revision = digests.revision + 1,
                 checkpoint_at = excluded.checkpoint_at,
                 generated_at = excluded.generated_at,
                 manifest_hash = excluded.manifest_hash,
                 article_count = excluded.article_count,
                 stats_json = excluded.stats_json",
            params![
                report_day, timezone_label, day_start, day_end,
                utc_offset_start, utc_offset_end,
                scope_key, scope_json, profile_key, profile_json,
                checkpoint_at, now, manifest_hash, article_count, stats_json
            ],
        )?;
        let digest_id: i64 = tx.query_row(
            "SELECT id FROM digests WHERE report_day = ?1 AND scope_key = ?2 AND profile_key = ?3",
            params![report_day, scope_key, profile_key],
            |r| r.get(0),
        )?;
        let revision: i64 = tx.query_row(
            "SELECT revision FROM digests WHERE id = ?1",
            params![digest_id],
            |r| r.get(0),
        )?;
        tx.execute(
            "DELETE FROM digest_bodies WHERE digest_id = ?1",
            params![digest_id],
        )?;
        tx.execute(
            "INSERT INTO digest_bodies(digest_id, revision, schema_ver, content_json, markdown)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![digest_id, revision, schema_ver, content_json, markdown],
        )?;
        tx.execute("DELETE FROM digest_items WHERE digest_id = ?1", params![digest_id])?;
        for e in items {
            tx.execute(
                "INSERT INTO digest_items(digest_id, instance_id, entry_id, feed_id,
                     source_revision, effective_at, input_hash, truncated, summary_only, title)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    digest_id, e.instance_id, e.entry_id, e.feed_id, e.source_revision,
                    e.effective_at, e.input_hash, e.truncated as i64, e.summary_only as i64, e.title
                ],
            )?;
        }
        tx.commit()?;
        Ok(digest_id)
    }
}

impl Store {
    /// 在飞任务 id（单 flight 检查口）。
    pub fn digest_active_job(&self, date: &str, scope_key: &str) -> super::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT active_job_id FROM digests WHERE report_day = ?1 AND scope_key = ?2",
                params![date, scope_key],
                |r| r.get(0),
            )
            .ok())
    }

    /// 范围 JSON 序列化（与 digests.scope_json 同构）。
    pub fn scope_json_for(&self, tag_ids: &[i64]) -> String {
        format!("{{\"tag_ids\":{}}}", serde_json::to_string(tag_ids).unwrap_or_default())
    }

    /// 生成槽位占位（DB 侧可见性）：无槽位则建 revision=0 空槽，有则标记在飞 job。
    #[allow(clippy::too_many_arguments)]
    pub fn digest_slot_begin(
        &self,
        date: &str,
        scope_key: &str,
        profile_key: &str,
        scope_json: &str,
        day_start: i64,
        day_end: i64,
        job_id: &str,
    ) -> super::Result<()> {
        self.conn.execute(
            "INSERT INTO digests(report_day, timezone_label, day_start_at, day_end_at,
                 utc_offset_start, utc_offset_end, date_basis, scope_key, scope_json,
                 profile_key, profile_json, revision, active_job_id, created_at)
             VALUES(?1, 'Local', ?2, ?3, 0, 0, 'published_or_first_seen_v1', ?4, ?5,
                 ?6, '{}', 0, ?7, ?8)
             ON CONFLICT(report_day, day_start_at, day_end_at, date_basis,
                 scope_key, profile_key) DO UPDATE SET
                 active_job_id = excluded.active_job_id",
            params![date, day_start, day_end, scope_key, scope_json, profile_key, job_id,
                chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// 生成结束（成功/取消/失败）：清掉在飞标记。旧报告不受影响。
    pub fn digest_slot_end(&self, date: &str, scope_key: &str) -> super::Result<()> {
        self.conn.execute(
            "UPDATE digests SET active_job_id = NULL WHERE report_day = ?1 AND scope_key = ?2",
            params![date, scope_key],
        )?;
        Ok(())
    }
}

impl Store {
    /// 日报单篇要点缓存读取：params 含语言+输入哈希+端点身份（内容寻址语义）。
    pub fn digest_item_cached(
        &self,
        entry_id: i64,
        params: &str,
        provider_model: &str,
    ) -> super::Result<Option<String>> {
        let key = super::AiCacheKey {
            entry_id,
            task: "digest_item",
            params,
            provider_model,
            prompt_version: crate::ai::prompt::PROMPT_VERSION,
        };
        self.ai_cached(&key)
    }

    /// 日报单篇要点缓存写入。
    pub fn digest_item_store(
        &self,
        entry_id: i64,
        params: &str,
        provider_model: &str,
        output: &str,
    ) -> super::Result<()> {
        let key = super::AiCacheKey {
            entry_id,
            task: "digest_item",
            params,
            provider_model,
            prompt_version: crate::ai::prompt::PROMPT_VERSION,
        };
        self.ai_store(&key, output)
    }
}
