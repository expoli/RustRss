//! In-memory preview policy. No windows, timers, credentials, or persistent writes.
use crate::theme::{ThemeConfig, ThemePatch, ThemeSnapshot};
pub const IDLE_SECONDS: u64 = 600;
pub const MAX_SECONDS: u64 = 1800;
#[derive(Clone)]
pub struct Preview {
    pub id: String,
    pub owner: String,
    pub base_revision: u64,
    pub revision: u64,
    pub candidate: ThemeConfig,
    created: u64,
    touched: u64,
}
impl Preview {
    pub fn new(id: String, owner: String, config: ThemeConfig, now: u64) -> Self {
        Self {
            id,
            owner,
            base_revision: config.revision,
            revision: 1,
            candidate: config,
            created: now,
            touched: now,
        }
    }
    pub fn expired(&self, now: u64) -> bool {
        now.saturating_sub(self.touched) >= IDLE_SECONDS
            || now.saturating_sub(self.created) >= MAX_SECONDS
    }
    pub fn check(
        &self,
        id: &str,
        owner: &str,
        revision: u64,
        now: u64,
    ) -> Result<(), &'static str> {
        if self.expired(now) {
            return Err("preview_expired");
        }
        if self.id != id || self.owner != owner {
            return Err("preview_not_owned");
        }
        if self.revision != revision {
            return Err("revision_conflict");
        }
        Ok(())
    }
    pub fn touch(&mut self, now: u64) {
        self.touched = now;
    }
    pub fn patch(&mut self, patch: &ThemePatch, now: u64) -> crate::theme::Result<ThemeSnapshot> {
        let next = self.candidate.apply(patch)?;
        if next != self.candidate {
            self.revision += 1;
            self.candidate = next;
        }
        self.touch(now);
        self.candidate.resolve()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_revision_idle_and_max_lifetime() {
        let mut p = Preview::new("id".into(), "owner".into(), ThemeConfig::default(), 0);
        assert_eq!(p.check("id", "other", 1, 0), Err("preview_not_owned"));
        assert_eq!(p.check("id", "owner", 0, 0), Err("revision_conflict"));
        assert!(!p.expired(599));
        assert!(p.expired(600));
        p.touch(1700);
        assert!(!p.expired(1799));
        assert!(p.expired(1800));
    }
    #[test]
    fn patch_is_transactional_and_noop_preserves_revision() {
        let mut p = Preview::new("id".into(), "owner".into(), ThemeConfig::default(), 0);
        p.patch(&ThemePatch::default(), 1).unwrap();
        assert_eq!(p.revision, 1);
        let patch =
            ThemePatch::from_json(r#"{"overrides":{"typography":{"read_size":20}}}"#).unwrap();
        p.patch(&patch, 2).unwrap();
        assert_eq!(p.revision, 2);
        assert_eq!(p.base_revision, 0);
        let bad =
            ThemePatch::from_json(r#"{"overrides":{"typography":{"read_size":100}}}"#).unwrap();
        assert!(p.patch(&bad, 3).is_err());
        assert_eq!(p.revision, 2);
    }
}
