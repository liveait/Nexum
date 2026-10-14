//! A durable proof of a complete HTTP response, independent of HTTP validators.

use super::{EngineError, HttpTransferControl, Progress, TaskId, commit_download};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const RECEIPT_VERSION: u32 = 1;
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
static NEXT_RECEIPT: AtomicU64 = AtomicU64::new(0);

/// Private journal location for one persisted task. Callers serialize attempts
/// for a task, and clear its receipt when deleting or reusing its identifier.
#[derive(Clone, Debug)]
pub struct CompletionContext {
    task_id: TaskId,
    directory: PathBuf,
}

impl CompletionContext {
    pub fn new(task_id: impl Into<TaskId>, data_dir: impl AsRef<Path>) -> Self {
        Self {
            task_id: task_id.into(),
            directory: data_dir.as_ref().join("http-completions"),
        }
    }

    /// Returns true when the context has no real directory (not yet initialized).
    pub fn is_empty(&self) -> bool {
        self.directory.as_path().is_empty()
    }
}

/// The filename is a hash, so an RPC task identifier never becomes a path.
pub fn completion_receipt_path(context: &CompletionContext) -> PathBuf {
    context.directory.join(format!(
        "{:x}.json",
        Sha256::digest(context.task_id.to_string().as_bytes())
    ))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    file: u64,
    created: Option<(u64, u32)>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileSnapshot {
    identity: FileIdentity,
    length: u64,
    modified: (u64, u32),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileProof {
    snapshot: FileSnapshot,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionReceipt {
    version: u32,
    task_id: String,
    source: String,
    destination: String,
    partial: PathBuf,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    file: FileProof,
    resume_metadata: Option<FileProof>,
}

fn failure(message: impl Into<String>) -> EngineError {
    EngineError::Failed(format!(
        "HTTP completion reconciliation: {}",
        message.into()
    ))
}

fn timestamp(value: SystemTime) -> Result<(u64, u32), EngineError> {
    let value = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| failure("file timestamp is before the Unix epoch"))?;
    Ok((value.as_secs(), value.subsec_nanos()))
}

#[cfg(unix)]
fn identity(_file: &File, metadata: &Metadata) -> Result<FileIdentity, EngineError> {
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity {
        device: metadata.dev(),
        file: metadata.ino(),
        created: metadata
            .created()
            .ok()
            .and_then(|value| timestamp(value).ok()),
    })
}

#[cfg(windows)]
fn identity(file: &File, _metadata: &Metadata) -> Result<FileIdentity, EngineError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
    };
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: The file owns a live handle and info points to sufficient storage.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) } == 0 {
        return Err(failure(std::io::Error::last_os_error().to_string()));
    }
    // SAFETY: A successful call initialized the structure.
    let info = unsafe { info.assume_init() };
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(failure("file is a reparse point"));
    }
    let created = ((info.ftCreationTime.dwHighDateTime as u64) << 32)
        | info.ftCreationTime.dwLowDateTime as u64;
    Ok(FileIdentity {
        device: info.dwVolumeSerialNumber as u64,
        file: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        // FILETIME's native ticks are sufficient for identity; this field need
        // not use the Unix timestamp representation used for modification time.
        created: Some((created, 0)),
    })
}

#[cfg(not(any(unix, windows)))]
fn identity(_file: &File, _metadata: &Metadata) -> Result<FileIdentity, EngineError> {
    Err(failure(
        "stable file identities are unsupported on this platform",
    ))
}

fn snapshot(file: &File) -> Result<FileSnapshot, EngineError> {
    let metadata = file
        .metadata()
        .map_err(|error| failure(error.to_string()))?;
    if !metadata.is_file() {
        return Err(failure("completion proof requires a regular file"));
    }
    Ok(FileSnapshot {
        identity: identity(file, &metadata)?,
        length: metadata.len(),
        modified: timestamp(
            metadata
                .modified()
                .map_err(|error| failure(error.to_string()))?,
        )?,
    })
}

/// Inspect before opening and inspect the held handle again. O_NONBLOCK keeps
/// a raced FIFO from hanging the worker; O_NOFOLLOW rejects a raced symlink.
fn open_regular(path: &Path) -> Result<Option<File>, EngineError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(failure(format!("{} is not a regular file", path.display())));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failure(error.to_string())),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failure(error.to_string())),
    };
    snapshot(&file)?;
    Ok(Some(file))
}

fn ensure_directory(context: &CompletionContext, create: bool) -> Result<bool, EngineError> {
    if create {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&context.directory) {
            Ok(()) => {
                if let Some(parent) = context.directory.parent() {
                    sync_directory(parent)?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(failure(error.to_string())),
        }
    }
    match std::fs::symlink_metadata(&context.directory) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(true),
        Ok(_) => Err(failure(
            "completion journal directory is not a real directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(failure(error.to_string())),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), EngineError> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_DIRECTORY)
        .open(path)
        .map_err(|error| failure(error.to_string()))?;
    file.sync_all().map_err(|error| failure(error.to_string()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), EngineError> {
    // Rust has no portable directory flush on Windows. File contents are
    // flushed before publication; reconciliation guarantees process crashes.
    Ok(())
}

fn read_receipt(context: &CompletionContext) -> Result<Option<CompletionReceipt>, EngineError> {
    if !ensure_directory(context, false)? {
        return Ok(None);
    }
    let Some(mut file) = open_regular(&completion_receipt_path(context))? else {
        return Ok(None);
    };
    let before = snapshot(&file)?;
    if before.length > MAX_RECEIPT_BYTES {
        return Err(failure("completion receipt exceeds the size limit"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| failure(error.to_string()))?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES || snapshot(&file)? != before {
        return Err(failure("completion receipt changed while reading"));
    }
    let receipt: CompletionReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| failure(format!("invalid completion receipt: {error}")))?;
    if receipt.version != RECEIPT_VERSION
        || receipt.task_id != context.task_id.to_string()
        || receipt.file.snapshot.length != receipt.downloaded_bytes
        || receipt
            .total_bytes
            .is_some_and(|total| total != receipt.downloaded_bytes)
        || receipt.file.sha256.len() != 64
        || !receipt
            .file
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || receipt.resume_metadata.as_ref().is_some_and(|proof| {
            proof.snapshot.length > MAX_RECEIPT_BYTES
                || proof.sha256.len() != 64
                || !proof.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err(failure("completion receipt has invalid proof fields"));
    }
    Ok(Some(receipt))
}

fn bound_receipt(
    context: &CompletionContext,
    source: &str,
    destination: &str,
    partial: &Path,
) -> Result<Option<CompletionReceipt>, EngineError> {
    let Some(receipt) = read_receipt(context)? else {
        return Ok(None);
    };
    if receipt.source != source || receipt.destination != destination || receipt.partial != partial
    {
        return Err(failure("completion receipt does not match the task"));
    }
    Ok(Some(receipt))
}

/// Performs bounded binding validation only. It never hashes a download, so a
/// server may use this while recovering its task queue before becoming ready.
pub fn has_pending_completion(
    context: &CompletionContext,
    source: &str,
    destination: &str,
    partial_path: impl AsRef<Path>,
) -> Result<bool, EngineError> {
    Ok(bound_receipt(context, source, destination, partial_path.as_ref())?.is_some())
}

fn hash_file(
    file: &mut File,
    control: &HttpTransferControl,
    maximum: Option<u64>,
) -> Result<FileProof, EngineError> {
    let before = snapshot(file)?;
    if maximum.is_some_and(|maximum| before.length > maximum) {
        return Err(failure("metadata file exceeds the size limit"));
    }
    let mut digest = Sha256::new();
    let mut length = 0u64;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        control.wait_if_paused()?;
        let read = file
            .read(&mut buffer)
            .map_err(|error| failure(error.to_string()))?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(read as u64)
            .ok_or_else(|| failure("file length overflow"))?;
        if length > before.length || maximum.is_some_and(|maximum| length > maximum) {
            return Err(failure("file grew while computing its completion proof"));
        }
        digest.update(&buffer[..read]);
    }
    control.wait_if_paused()?;
    if length != before.length || snapshot(file)? != before {
        return Err(failure("file changed while computing its completion proof"));
    }
    Ok(FileProof {
        snapshot: before,
        sha256: format!("{:x}", digest.finalize()),
    })
}

fn verified_file(
    path: &Path,
    proof: &FileProof,
    control: &HttpTransferControl,
) -> Result<Option<File>, EngineError> {
    let Some(mut file) = open_regular(path)? else {
        return Ok(None);
    };
    if snapshot(&file)? != proof.snapshot
        || hash_file(&mut file, control, Some(proof.snapshot.length))? != *proof
    {
        return Err(failure(format!(
            "{} does not match the completed response",
            path.display()
        )));
    }
    // Verify the pathname still selects this held regular file after hashing.
    let Some(current) = open_regular(path)? else {
        return Err(failure("completed response disappeared while verifying"));
    };
    if snapshot(&current)? != proof.snapshot {
        return Err(failure("completed response was replaced while verifying"));
    }
    Ok(Some(file))
}

fn write_receipt(
    context: &CompletionContext,
    receipt: &CompletionReceipt,
) -> Result<(), EngineError> {
    ensure_directory(context, true)?;
    let bytes = serde_json::to_vec(receipt).map_err(|error| failure(error.to_string()))?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(failure("completion receipt exceeds the size limit"));
    }
    let path = context.directory.join(format!(
        ".receipt-{}-{}.tmp",
        std::process::id(),
        NEXT_RECEIPT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| failure(error.to_string()))?;
    let result = (|| {
        file.write_all(&bytes)
            .map_err(|error| failure(error.to_string()))?;
        file.sync_all()
            .map_err(|error| failure(error.to_string()))?;
        drop(file);
        commit_download(&path, &completion_receipt_path(context))?;
        sync_directory(&context.directory)
    })();
    let _ = std::fs::remove_file(&path);
    result
}

pub fn prepare_completion(
    context: &CompletionContext,
    source: &str,
    destination: &str,
    partial: &Path,
    writer: &File,
    progress: &Progress,
    control: &HttpTransferControl,
) -> Result<CompletionReceipt, EngineError> {
    // Skip completion for contexts without a real directory (session downloads).
    if context.directory.as_path().is_empty() {
        return Err(EngineError::UnsupportedOperation(
            "no completion context provided".into(),
        ));
    }
    control.wait_if_paused()?;
    if read_receipt(context)?.is_some() {
        return Err(failure(
            "a previous completed response still needs reconciliation",
        ));
    }
    writer
        .sync_all()
        .map_err(|error| failure(error.to_string()))?;
    let writer_snapshot = snapshot(writer)?;
    let Some(mut reader) = open_regular(partial)? else {
        return Err(failure("complete partial response disappeared"));
    };
    let file = hash_file(&mut reader, control, None)?;
    if file.snapshot != writer_snapshot || file.snapshot.length != progress.downloaded_bytes {
        return Err(failure("partial response changed before publication"));
    }
    let metadata_path = super::resumable_metadata_path(partial);
    let resume_metadata = if let Some(mut metadata) = open_regular(&metadata_path)? {
        Some(hash_file(&mut metadata, control, Some(MAX_RECEIPT_BYTES))?)
    } else {
        None
    };
    let receipt = CompletionReceipt {
        version: RECEIPT_VERSION,
        task_id: context.task_id.to_string(),
        source: source.to_owned(),
        destination: destination.to_owned(),
        partial: partial.to_owned(),
        downloaded_bytes: progress.downloaded_bytes,
        total_bytes: progress.total_bytes,
        file,
        resume_metadata,
    };
    control.wait_if_paused()?;
    write_receipt(context, &receipt)?;
    Ok(receipt)
}

fn validate_published(receipt: &CompletionReceipt) -> Result<(), EngineError> {
    let Some(file) = open_regular(Path::new(&receipt.destination))? else {
        return Err(failure("published response disappeared"));
    };
    if snapshot(&file)? != receipt.file.snapshot {
        return Err(failure(
            "published response does not match the prepared file",
        ));
    }
    Ok(())
}

/// Publish under the transfer-control lock so cancellation cannot win after the
/// no-clobber operation while still being reported as a cancelled transfer.
pub fn publish_completion(
    receipt: &CompletionReceipt,
    control: &HttpTransferControl,
) -> Result<(), EngineError> {
    control.wait_if_paused()?;
    let Some(file) = open_regular(&receipt.partial)? else {
        return Err(failure(
            "complete partial response disappeared before publication",
        ));
    };
    if snapshot(&file)? != receipt.file.snapshot {
        return Err(failure(
            "complete partial response was replaced before publication",
        ));
    }
    let (lock, wake) = &*control.inner;
    let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
    if state.cancelled {
        return Err(EngineError::Cancelled);
    }
    commit_download(&receipt.partial, Path::new(&receipt.destination))?;
    state.committed = true;
    state.finished = true;
    state.pause_requested = false;
    state.paused = false;
    wake.notify_all();
    validate_published(receipt)?;
    if let Some(parent) = Path::new(&receipt.destination).parent() {
        sync_directory(parent)?;
    }
    if receipt.partial.parent() != Path::new(&receipt.destination).parent()
        && let Some(parent) = receipt.partial.parent()
    {
        sync_directory(parent)?;
    }
    Ok(())
}

/// Reconcile only a response proved complete before publication. File size or
/// content equality alone never establishes ownership: stable file identity is
/// required as well. The large-file read runs in the worker and obeys pause and
/// cancellation. A matching complete partial can publish without any HTTP GET.
pub fn reconcile_completion(
    context: &CompletionContext,
    source: &str,
    destination: &str,
    partial_path: impl AsRef<Path>,
    control: &HttpTransferControl,
) -> Result<Option<Progress>, EngineError> {
    control.wait_if_paused()?;
    let Some(receipt) = bound_receipt(context, source, destination, partial_path.as_ref())? else {
        return Ok(None);
    };
    if let Some(_file) = verified_file(Path::new(destination), &receipt.file, control)? {
        let (lock, wake) = &*control.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.cancelled {
            return Err(EngineError::Cancelled);
        }
        validate_published(&receipt)?;
        state.committed = true;
        state.finished = true;
        state.pause_requested = false;
        state.paused = false;
        wake.notify_all();
    } else {
        let Some(_file) = verified_file(&receipt.partial, &receipt.file, control)? else {
            return Err(failure(
                "both the complete partial and its published response are missing",
            ));
        };
        publish_completion(&receipt, control)?;
    }
    Ok(Some(Progress::new(
        receipt.downloaded_bytes,
        receipt.total_bytes,
    )))
}

/// Remove a partial only if its identity and stable metadata still match, and
/// a small validator sidecar only if its contents match as well. No download
/// bytes are read. This is separate from journal deletion so callers can
/// retain the receipt until the final SQLite update succeeds.
pub fn cleanup_completion_partial(
    context: &CompletionContext,
    source: &str,
    destination: &str,
    partial_path: impl AsRef<Path>,
) -> Result<(), EngineError> {
    let partial = partial_path.as_ref();
    let Some(receipt) = bound_receipt(context, source, destination, partial)? else {
        return Ok(());
    };
    let control = HttpTransferControl::new();
    // Avoid rehashing a multi-gigabyte hard-link alias after the task is
    // durably complete, especially during server startup.
    if let Some(file) = open_regular(partial)? {
        if snapshot(&file)? != receipt.file.snapshot {
            return Err(failure(
                "complete partial was replaced or modified before cleanup",
            ));
        }
        let Some(current) = open_regular(partial)? else {
            return Err(failure("complete partial disappeared before cleanup"));
        };
        if snapshot(&current)? != receipt.file.snapshot {
            return Err(failure("complete partial was replaced before cleanup"));
        }
        std::fs::remove_file(partial).map_err(|error| failure(error.to_string()))?;
    }
    if let Some(proof) = receipt.resume_metadata {
        let metadata = super::resumable_metadata_path(partial);
        if verified_file(&metadata, &proof, &control)?.is_some() {
            std::fs::remove_file(metadata).map_err(|error| failure(error.to_string()))?;
        }
    }
    Ok(())
}

/// Delete only the private receipt. Published download files are never removed.
pub fn clear_completion(context: &CompletionContext) -> Result<(), EngineError> {
    if !ensure_directory(context, false)? {
        return Ok(());
    }
    match std::fs::remove_file(completion_receipt_path(context)) {
        Ok(()) => sync_directory(&context.directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failure(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HttpEngine;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    const SOURCE: &str = "http://127.0.0.1:1/no-network-request";
    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        context: CompletionContext,
        destination: String,
        partial: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "nexum-completion-test-{}-{}",
                std::process::id(),
                NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            let data = root.join("data");
            let downloads = root.join("downloads");
            std::fs::create_dir(&data).unwrap();
            std::fs::create_dir(&downloads).unwrap();
            let downloads = std::fs::canonicalize(downloads).unwrap();
            let context = CompletionContext::new("../task/with/path", data);
            Self {
                root,
                context,
                destination: downloads.join("file.bin").to_str().unwrap().to_owned(),
                partial: downloads.join(".file.bin.nexum.part"),
            }
        }

        fn prepare(&self, bytes: &[u8], total: Option<u64>) -> CompletionReceipt {
            let mut writer = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.partial)
                .unwrap();
            writer.write_all(bytes).unwrap();
            prepare_completion(
                &self.context,
                SOURCE,
                &self.destination,
                &self.partial,
                &writer,
                &Progress::new(bytes.len() as u64, total),
                &HttpTransferControl::new(),
            )
            .unwrap()
        }

        fn reconcile(
            &self,
            control: &HttpTransferControl,
        ) -> Result<Option<Progress>, EngineError> {
            reconcile_completion(
                &self.context,
                SOURCE,
                &self.destination,
                &self.partial,
                control,
            )
        }

        fn cleanup(&self) -> Result<(), EngineError> {
            cleanup_completion_partial(&self.context, SOURCE, &self.destination, &self.partial)
        }

        fn rewrite(&self, receipt: &CompletionReceipt) {
            std::fs::write(
                completion_receipt_path(&self.context),
                serde_json::to_vec(receipt).unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn prepared_unknown_length_response_publishes_without_http_or_validator() {
        let fixture = Fixture::new();
        fixture.prepare(b"completed response", None);
        let control = HttpTransferControl::new();
        let result = HttpEngine::new()
            .download_to_resumable_with_completion(
                SOURCE,
                &fixture.destination,
                &fixture.partial,
                &fixture.context,
                &control,
                |_| panic!("no network transfer should be performed"),
            )
            .unwrap();
        assert_eq!(result, Progress::new(18, None));
        assert!(control.is_committed());
        assert_eq!(
            std::fs::read(&fixture.destination).unwrap(),
            b"completed response"
        );
        assert!(completion_receipt_path(&fixture.context).exists());
        fixture.cleanup().unwrap();
        clear_completion(&fixture.context).unwrap();
        assert!(!fixture.partial.exists());
        assert!(!completion_receipt_path(&fixture.context).exists());
        assert_eq!(
            std::fs::read(&fixture.destination).unwrap(),
            b"completed response"
        );
    }

    #[test]
    fn published_response_recovers_original_total_and_keeps_receipt_until_acknowledged() {
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", Some(14));
        commit_download(&fixture.partial, Path::new(&fixture.destination)).unwrap();
        let control = HttpTransferControl::new();
        assert_eq!(
            fixture.reconcile(&control).unwrap(),
            Some(Progress::new(14, Some(14)))
        );
        assert!(control.is_committed());
        assert!(
            has_pending_completion(
                &fixture.context,
                SOURCE,
                &fixture.destination,
                &fixture.partial
            )
            .unwrap()
        );
        // A second process recovery remains idempotent until SQLite succeeds.
        assert_eq!(
            fixture.reconcile(&HttpTransferControl::new()).unwrap(),
            Some(Progress::new(14, Some(14)))
        );
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_fallback_recovery_cleans_only_the_staging_alias() {
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        crate::commit_download_with_rename(
            &fixture.partial,
            Path::new(&fixture.destination),
            |_, _| Err(std::io::Error::from_raw_os_error(libc::ENOSYS)),
        )
        .unwrap();
        assert!(fixture.partial.exists());
        fixture.reconcile(&HttpTransferControl::new()).unwrap();
        fixture.cleanup().unwrap();
        assert!(!fixture.partial.exists());
        assert_eq!(
            std::fs::read(&fixture.destination).unwrap(),
            b"complete bytes"
        );
        assert!(completion_receipt_path(&fixture.context).exists());
    }

    #[test]
    fn equal_content_in_another_file_is_not_owned_by_the_task() {
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        std::fs::write(&fixture.destination, b"complete bytes").unwrap();
        assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        assert_eq!(
            std::fs::read(&fixture.destination).unwrap(),
            b"complete bytes"
        );
        assert_eq!(std::fs::read(&fixture.partial).unwrap(), b"complete bytes");
        assert!(completion_receipt_path(&fixture.context).exists());
    }

    #[test]
    fn same_size_foreign_destination_is_not_overwritten_or_completed() {
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        std::fs::write(&fixture.destination, b"foreign  bytes").unwrap();
        assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        assert_eq!(
            std::fs::read(&fixture.destination).unwrap(),
            b"foreign  bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn changed_bytes_with_restored_mtime_are_rejected_by_the_digest() {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        let metadata = std::fs::metadata(&fixture.partial).unwrap();
        std::fs::write(&fixture.partial, b"corrupt  bytes").unwrap();
        let file = File::open(&fixture.partial).unwrap();
        let times = [
            libc::timespec {
                tv_sec: metadata.atime(),
                tv_nsec: metadata.atime_nsec(),
            },
            libc::timespec {
                tv_sec: metadata.mtime(),
                tv_nsec: metadata.mtime_nsec(),
            },
        ];
        // SAFETY: The descriptor is live and times points to two timespecs.
        assert_eq!(
            unsafe { libc::futimens(file.as_raw_fd(), times.as_ptr()) },
            0
        );
        assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        assert_eq!(std::fs::read(&fixture.partial).unwrap(), b"corrupt  bytes");
        assert!(!Path::new(&fixture.destination).exists());
    }

    #[test]
    fn cleanup_preserves_a_replaced_partial_and_receipt() {
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        commit_download(&fixture.partial, Path::new(&fixture.destination)).unwrap();
        std::fs::write(&fixture.partial, b"new user file").unwrap();
        assert!(fixture.cleanup().is_err());
        assert_eq!(std::fs::read(&fixture.partial).unwrap(), b"new user file");
        assert!(completion_receipt_path(&fixture.context).exists());
    }

    #[test]
    fn cleanup_preserves_replaced_validator_sidecar() {
        let fixture = Fixture::new();
        let sidecar = crate::resumable_metadata_path(&fixture.partial);
        std::fs::write(&sidecar, b"original validator").unwrap();
        fixture.prepare(b"complete bytes", None);
        commit_download(&fixture.partial, Path::new(&fixture.destination)).unwrap();
        std::fs::remove_file(&sidecar).unwrap();
        std::fs::write(&sidecar, b"new user metadata").unwrap();
        assert!(fixture.cleanup().is_err());
        assert_eq!(std::fs::read(sidecar).unwrap(), b"new user metadata");
    }

    #[test]
    fn bounded_binding_read_rejects_task_source_destination_and_partial_mismatch() {
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        assert!(
            has_pending_completion(
                &fixture.context,
                "http://other",
                &fixture.destination,
                &fixture.partial
            )
            .is_err()
        );
        assert!(
            has_pending_completion(&fixture.context, SOURCE, "/another", &fixture.partial).is_err()
        );
        assert!(
            has_pending_completion(
                &fixture.context,
                SOURCE,
                &fixture.destination,
                fixture.root.join("another.part")
            )
            .is_err()
        );
        let mut receipt = read_receipt(&fixture.context).unwrap().unwrap();
        receipt.task_id = "another task".into();
        fixture.rewrite(&receipt);
        assert!(
            has_pending_completion(
                &fixture.context,
                SOURCE,
                &fixture.destination,
                &fixture.partial
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_oversized_and_inconsistent_receipts_never_publish() {
        let fixture = Fixture::new();
        let receipt = fixture.prepare(b"complete bytes", None);
        for bytes in [
            b"{broken".to_vec(),
            vec![b' '; MAX_RECEIPT_BYTES as usize + 1],
        ] {
            std::fs::write(completion_receipt_path(&fixture.context), bytes).unwrap();
            assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        }
        let mut invalid = receipt.clone();
        invalid.total_bytes = Some(999);
        fixture.rewrite(&invalid);
        assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        invalid = receipt;
        invalid.file.sha256 = "0".repeat(64);
        fixture.rewrite(&invalid);
        assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        assert!(!Path::new(&fixture.destination).exists());
        assert_eq!(std::fs::read(&fixture.partial).unwrap(), b"complete bytes");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_destination_and_partial_are_rejected_without_following() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        let victim = fixture.root.join("victim");
        std::fs::write(&victim, b"user contents").unwrap();
        symlink(&victim, &fixture.destination).unwrap();
        assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        std::fs::remove_file(&fixture.destination).unwrap();
        std::fs::remove_file(&fixture.partial).unwrap();
        symlink(&victim, &fixture.partial).unwrap();
        assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        assert!(fixture.cleanup().is_err());
        assert_eq!(std::fs::read(victim).unwrap(), b"user contents");
    }

    #[cfg(unix)]
    #[test]
    fn receipt_fifo_and_symlink_are_rejected_without_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        let receipt = completion_receipt_path(&fixture.context);
        std::fs::remove_file(&receipt).unwrap();
        let name = CString::new(receipt.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is NUL-terminated and remains live for this call.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let start = Instant::now();
        assert!(
            has_pending_completion(
                &fixture.context,
                SOURCE,
                &fixture.destination,
                &fixture.partial
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        std::fs::remove_file(&receipt).unwrap();
        symlink(&fixture.partial, &receipt).unwrap();
        assert!(
            has_pending_completion(
                &fixture.context,
                SOURCE,
                &fixture.destination,
                &fixture.partial
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&fixture.partial).unwrap(), b"complete bytes");
    }

    #[cfg(unix)]
    #[test]
    fn journal_directory_symlink_is_rejected_and_permissions_are_private() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        assert_eq!(
            std::fs::metadata(&fixture.context.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(completion_receipt_path(&fixture.context))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        std::fs::rename(
            &fixture.context.directory,
            fixture.root.join("actual-journal"),
        )
        .unwrap();
        symlink(
            fixture.root.join("actual-journal"),
            &fixture.context.directory,
        )
        .unwrap();
        assert!(fixture.reconcile(&HttpTransferControl::new()).is_err());
        assert!(clear_completion(&fixture.context).is_err());
    }

    #[test]
    fn pause_then_cancel_during_reconciliation_preserves_complete_evidence() {
        let fixture = Fixture::new();
        fixture.prepare(b"complete bytes", None);
        let control = HttpTransferControl::new();
        {
            let (lock, _) = &*control.inner;
            lock.lock().unwrap().pause_requested = true;
        }
        let worker_control = control.clone();
        let context = fixture.context.clone();
        let destination = fixture.destination.clone();
        let partial = fixture.partial.clone();
        let (send, receive) = mpsc::channel();
        let worker = thread::spawn(move || {
            send.send(reconcile_completion(
                &context,
                SOURCE,
                &destination,
                &partial,
                &worker_control,
            ))
            .unwrap();
        });
        control.request_pause().unwrap();
        assert!(control.is_paused());
        control.cancel();
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(EngineError::Cancelled)
        );
        worker.join().unwrap();
        assert!(fixture.partial.exists());
        assert!(completion_receipt_path(&fixture.context).exists());
        assert!(!Path::new(&fixture.destination).exists());
    }

    #[test]
    fn an_empty_complete_response_is_recoverable() {
        let fixture = Fixture::new();
        fixture.prepare(b"", Some(0));
        assert_eq!(
            fixture.reconcile(&HttpTransferControl::new()).unwrap(),
            Some(Progress::new(0, Some(0)))
        );
        assert_eq!(std::fs::metadata(&fixture.destination).unwrap().len(), 0);
    }

    #[test]
    fn real_unknown_length_http_completion_journals_without_a_validator() {
        let fixture = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/file", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                connection.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ncomplete bytes")
                .unwrap();
        });
        let result = HttpEngine::new()
            .download_to_resumable_with_completion(
                &source,
                &fixture.destination,
                &fixture.partial,
                &fixture.context,
                &HttpTransferControl::new(),
                |_| Ok(()),
            )
            .unwrap();
        server.join().unwrap();
        assert_eq!(result, Progress::new(14, None));
        assert!(!crate::resumable_metadata_path(&fixture.partial).exists());
        assert!(
            has_pending_completion(
                &fixture.context,
                &source,
                &fixture.destination,
                &fixture.partial
            )
            .unwrap()
        );
        // The fixture listener is gone: a second GET would fail immediately.
        let result = HttpEngine::new()
            .download_to_resumable_with_completion(
                &source,
                &fixture.destination,
                &fixture.partial,
                &fixture.context,
                &HttpTransferControl::new(),
                |_| panic!("unexpected second request"),
            )
            .unwrap();
        assert_eq!(result, Progress::new(14, None));
    }
}
