//! One held file for the currently disabled segmented HTTP prototype.
//!
//! Writes use the held handle and absolute offsets. Path checks reject observed
//! replacements before writes, publication and cleanup. Path-based publication
//! and unlink are still subject to a check/use race with an actor concurrently
//! mutating the directory; this is not an atomic filesystem ownership protocol.

use super::file_identity::{FileIdentity, identity};
use super::{EngineError, commit_download};
use std::fs::{File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};

pub(super) struct SegmentStaging {
    file: File,
    path: PathBuf,
    identity: FileIdentity,
}

fn failure(message: impl std::fmt::Display) -> EngineError {
    EngineError::Failed(format!("segment staging: {message}"))
}

impl SegmentStaging {
    pub(super) fn create(path: &Path, length: u64) -> Result<Self, EngineError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(path).map_err(failure)?;
        let metadata = file.metadata().map_err(failure)?;
        if !metadata.is_file() {
            return Err(failure("newly created staging is not a regular file"));
        }
        let owned = Self {
            identity: identity(&file, &metadata)?,
            file,
            path: path.to_owned(),
        };
        if let Err(error) = owned.file.set_len(length) {
            return Err(owned.cleanup_after_error(failure(error)));
        }
        Ok(owned)
    }

    fn held_metadata(&self) -> Result<Metadata, EngineError> {
        let metadata = self.file.metadata().map_err(failure)?;
        if !metadata.is_file() || identity(&self.file, &metadata)? != self.identity {
            return Err(failure("held file identity changed"));
        }
        Ok(metadata)
    }

    fn path_is_owned(&self, path: &Path) -> Result<bool, EngineError> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(failure(error)),
        };
        #[cfg(unix)]
        {
            Ok(identity(&self.file, &metadata)? == self.identity)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
            let _ = metadata;
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
                .map_err(failure)?;
            let metadata = file.metadata().map_err(failure)?;
            Ok(metadata.is_file() && identity(&file, &metadata)? == self.identity)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = metadata;
            Err(failure(
                "stable file identities are unsupported on this platform",
            ))
        }
    }

    pub(super) fn ensure_owned(&self) -> Result<(), EngineError> {
        self.held_metadata()?;
        if !self.path_is_owned(&self.path)? {
            return Err(failure("staging path no longer refers to the owned file"));
        }
        Ok(())
    }

    pub(super) fn write_all_at(
        &self,
        mut bytes: &[u8],
        mut offset: u64,
    ) -> Result<(), EngineError> {
        while !bytes.is_empty() {
            self.ensure_owned()?;
            #[cfg(unix)]
            let result = {
                use std::os::unix::fs::FileExt;
                self.file.write_at(bytes, offset)
            };
            #[cfg(windows)]
            let result = {
                use std::os::windows::fs::FileExt;
                // seek_write uses this absolute offset even though Windows also
                // updates its cursor. No worker uses cursor-based seek/write.
                self.file.seek_write(bytes, offset)
            };
            #[cfg(not(any(unix, windows)))]
            let result: std::io::Result<usize> = Err(std::io::ErrorKind::Unsupported.into());
            match result {
                Ok(0) => return Err(failure(std::io::Error::from(std::io::ErrorKind::WriteZero))),
                Ok(written) => {
                    bytes = &bytes[written..];
                    offset = offset
                        .checked_add(written as u64)
                        .ok_or_else(|| failure("write offset overflow"))?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(failure(error)),
            }
        }
        Ok(())
    }

    pub(super) fn publish(&self, destination: &Path, expected: u64) -> Result<(), EngineError> {
        self.ensure_owned()?;
        let actual = self.held_metadata()?.len();
        if actual != expected {
            return Err(failure(format!(
                "held file size {actual} does not match expected {expected}"
            )));
        }
        self.file.sync_all().map_err(failure)?;
        // Recheck after the flush. This remains a path-based rename/link with
        // an explicitly unclosed check/use window, not handle publication.
        self.ensure_owned()?;
        commit_download(&self.path, destination)?;
        if !self.path_is_owned(destination)? {
            return Err(failure(
                "published destination does not refer to the owned file",
            ));
        }
        self.cleanup()?;
        Ok(())
    }

    /// A missing/replaced name is not ours to remove. In particular this
    /// prototype never created or owned the resumable HTTP JSON sidecar.
    fn cleanup(&self) -> Result<(), EngineError> {
        if self.path_is_owned(&self.path)? {
            std::fs::remove_file(&self.path).map_err(failure)?;
        }
        Ok(())
    }

    pub(super) fn cleanup_after_error(&self, error: EngineError) -> EngineError {
        match self.cleanup() {
            Ok(()) => error,
            // Retain the cancellation type used by transfer consumers.
            Err(_) if matches!(error, EngineError::Cancelled) => error,
            Err(cleanup) => failure(format!("{error}; cleanup also failed: {cleanup}")),
        }
    }
}
