//! Bounded, private local screenshot artifacts. No database or transport dependency.
//! The caller supplies monotonic seconds for expiry and a wall clock for the public deadline.
use serde::Serialize;
use std::{io::Write, path::PathBuf};
use tempfile::{TempDir, TempPath};

pub const TTL_SECONDS: u64 = 600;
pub const MAX_FILES: usize = 32;
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PNG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("screenshot exceeds the PNG byte limit")]
    ImageTooLarge,
    #[error("screenshot storage is full; wait for file expiry")]
    Limit,
    #[error("screenshot file I/O failed")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Serialize)]
pub struct ScreenshotFile {
    pub image_path: PathBuf,
    pub image_bytes: usize,
    pub image_expires_at_ms: u64,
}

struct Entry {
    path: TempPath,
    owner: String,
    expires: u64,
    bytes: usize,
}

/// A service owns one randomly named directory (0700 on Unix) and its files (0600).
/// Paths are generated here, never supplied by a caller. Dropping the store removes them.
/// A crashed process can leave its private directory for OS temporary-file maintenance;
/// this store deliberately does not scan or delete another process's directories.
pub struct PreviewFiles {
    entries: Vec<Entry>,
    directory: TempDir,
}

impl PreviewFiles {
    pub fn new() -> Result<Self, FileError> {
        // Resolve TMPDIR before creation so the public path is absolute even for relative TMPDIR.
        let parent = std::fs::canonicalize(std::env::temp_dir())?;
        if parent.to_str().is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "temporary path is not UTF-8",
            )
            .into());
        }
        let mut builder = tempfile::Builder::new();
        builder.prefix("rustrss-preview-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let directory = builder.tempdir_in(parent)?;
        Ok(Self {
            entries: Vec::new(),
            directory,
        })
    }

    /// Reap expired files and files owned by a revoked token, including finished previews.
    /// Failed deletions remain accounted for and are retried on the next sweep.
    pub fn sweep(&mut self, now: u64, owner: Option<&str>) {
        self.entries.retain(|entry| {
            if now < entry.expires && Some(entry.owner.as_str()) == owner {
                return true;
            }
            match std::fs::remove_file(&entry.path) {
                Ok(()) => false,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => {
                    log::warn!("preview screenshot cleanup failed; will retry");
                    true
                }
            }
        });
    }

    pub fn write(
        &mut self,
        png: &[u8],
        owner: &str,
        now: u64,
        wall_ms: u64,
    ) -> Result<ScreenshotFile, FileError> {
        // All bounds precede file creation/writes. Never evict an unexpired artifact for capacity.
        if png.len() > MAX_PNG_BYTES {
            return Err(FileError::ImageTooLarge);
        }
        self.sweep(now, Some(owner));
        let bytes: usize = self.entries.iter().map(|e| e.bytes).sum();
        if self.entries.len() >= MAX_FILES || png.len() > MAX_BYTES.saturating_sub(bytes) {
            return Err(FileError::Limit);
        }
        let mut file = tempfile::Builder::new()
            .prefix("capture-")
            .suffix(".png")
            .tempfile_in(self.directory.path())?;
        file.write_all(png)?;
        file.flush()?;
        // Publish only after the complete file is closed. Any earlier failure drops the partial file.
        let path = file.into_temp_path();
        let result = ScreenshotFile {
            image_path: path.to_path_buf(),
            image_bytes: png.len(),
            image_expires_at_ms: wall_ms.saturating_add(TTL_SECONDS * 1000),
        };
        self.entries.push(Entry {
            path,
            owner: owner.into(),
            expires: now.saturating_add(TTL_SECONDS),
            bytes: png.len(),
        });
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_complete_files_expire_at_boundary_and_drop_cleans_directory() {
        let mut files = PreviewFiles::new().unwrap();
        let a = files.write(b"frame-a", "owner", 10, 1000).unwrap();
        let b = files.write(b"frame-b", "owner", 10, 1000).unwrap();
        assert_ne!(a.image_path, b.image_path);
        assert!(a.image_path.is_absolute());
        assert_eq!(std::fs::read(&a.image_path).unwrap(), b"frame-a");
        assert_eq!(a.image_expires_at_ms, 601000);
        files.sweep(609, Some("owner"));
        assert!(a.image_path.exists());
        files.sweep(610, Some("owner"));
        assert!(!a.image_path.exists() && !b.image_path.exists());
        let c = files.write(b"frame-c", "owner", 610, 1000).unwrap();
        let directory = files.directory.path().to_path_buf();
        drop(files);
        assert!(!c.image_path.exists() && !directory.exists());
    }

    #[test]
    fn revocation_removes_files_without_deleting_unrelated_paths() {
        let mut files = PreviewFiles::new().unwrap();
        let unrelated = tempfile::NamedTempFile::new().unwrap();
        let a = files.write(b"a", "old", 0, 0).unwrap();
        files.sweep(1, Some("new"));
        assert!(!a.image_path.exists());
        let b = files.write(b"b", "new", 1, 0).unwrap();
        files.sweep(2, None);
        assert!(!b.image_path.exists());
        assert!(unrelated.path().exists());
    }

    #[test]
    fn count_and_byte_limits_precede_writes_and_do_not_evict() {
        let mut files = PreviewFiles::new().unwrap();
        assert!(matches!(
            files.write(&vec![0; MAX_PNG_BYTES + 1], "o", 0, 0),
            Err(FileError::ImageTooLarge)
        ));
        assert_eq!(
            std::fs::read_dir(files.directory.path()).unwrap().count(),
            0
        );
        for _ in 0..MAX_FILES {
            files.write(b"a", "o", 0, 0).unwrap();
        }
        assert!(matches!(
            files.write(b"b", "o", 0, 0),
            Err(FileError::Limit)
        ));
        assert_eq!(
            std::fs::read_dir(files.directory.path()).unwrap().count(),
            MAX_FILES
        );
        files.sweep(TTL_SECONDS, Some("o"));
        let data = vec![0; MAX_PNG_BYTES];
        for _ in 0..MAX_BYTES / MAX_PNG_BYTES {
            files.write(&data, "o", TTL_SECONDS, 0).unwrap();
        }
        assert!(matches!(
            files.write(b"b", "o", TTL_SECONDS, 0),
            Err(FileError::Limit)
        ));
        assert_eq!(
            std::fs::read_dir(files.directory.path()).unwrap().count(),
            MAX_BYTES / MAX_PNG_BYTES
        );
    }

    #[cfg(unix)]
    #[test]
    fn directories_and_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let mut files = PreviewFiles::new().unwrap();
        let a = files.write(b"a", "o", 0, 0).unwrap();
        assert_eq!(
            std::fs::metadata(files.directory.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(a.image_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
