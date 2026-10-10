//! Engine adapter boundaries for Nexum.

use nexum_domain::{Progress, TaskId};
use nexum_task::TaskState;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

static NEXT_TEMP_DOWNLOAD: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineCapabilities {
    pub supports_pause: bool,
    pub supports_resume: bool,
    pub supports_remove: bool,
    pub supports_progress: bool,
}

impl EngineCapabilities {
    pub const BASIC: Self = Self {
        supports_pause: false,
        supports_resume: false,
        supports_remove: false,
        supports_progress: true,
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineTask {
    pub task_id: TaskId,
    pub handle: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineError {
    UnsupportedOperation(&'static str),
    TaskNotFound(TaskId),
    Cancelled,
    Failed(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOperation(operation) => {
                write!(f, "unsupported engine operation: {operation}")
            }
            Self::TaskNotFound(id) => write!(f, "engine task not found: {id}"),
            Self::Cancelled => write!(f, "engine transfer cancelled"),
            Self::Failed(message) => write!(f, "engine error: {message}"),
        }
    }
}

impl std::error::Error for EngineError {}

pub trait EngineAdapter {
    fn name(&self) -> &str;
    fn capabilities(&self) -> EngineCapabilities;
    fn start(
        &mut self,
        task_id: &TaskId,
        source: &str,
        destination: &str,
    ) -> Result<EngineTask, EngineError>;
    fn pause(&mut self, task: &EngineTask) -> Result<(), EngineError>;
    fn resume(&mut self, task: &EngineTask) -> Result<(), EngineError>;
    fn remove(&mut self, task: &EngineTask) -> Result<(), EngineError>;
    fn progress(&self, task: &EngineTask) -> Result<Progress, EngineError>;
    fn state(&self, task: &EngineTask) -> Result<EngineTaskState, EngineError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineTaskState {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineSnapshot {
    pub state: EngineTaskState,
    pub progress: Progress,
}

impl EngineSnapshot {
    pub fn new(state: EngineTaskState, progress: Progress) -> Self {
        Self { state, progress }
    }

    pub fn to_task_state(&self) -> TaskState {
        match self.state {
            EngineTaskState::Queued => TaskState::Queued,
            EngineTaskState::Downloading => TaskState::Downloading,
            EngineTaskState::Paused => TaskState::Paused,
            EngineTaskState::Completed => TaskState::Completed,
            EngineTaskState::Failed => TaskState::Failed,
        }
    }
}

pub fn map_engine_snapshot(snapshot: &EngineSnapshot) -> (TaskState, Progress) {
    (snapshot.to_task_state(), snapshot.progress.clone())
}

#[derive(Default)]
pub struct InMemoryEngine {
    tasks: std::collections::HashMap<String, (TaskId, EngineTaskState, Progress)>,
    next_handle: u64,
}

impl InMemoryEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(
        &self,
        task: &EngineTask,
    ) -> Result<&(TaskId, EngineTaskState, Progress), EngineError> {
        self.tasks
            .get(&task.handle)
            .ok_or_else(|| EngineError::TaskNotFound(task.task_id.clone()))
    }

    fn entry_mut(
        &mut self,
        task: &EngineTask,
    ) -> Result<&mut (TaskId, EngineTaskState, Progress), EngineError> {
        self.tasks
            .get_mut(&task.handle)
            .ok_or_else(|| EngineError::TaskNotFound(task.task_id.clone()))
    }
}

impl EngineAdapter for InMemoryEngine {
    fn name(&self) -> &str {
        "in-memory"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            supports_pause: true,
            supports_resume: true,
            supports_remove: true,
            supports_progress: true,
        }
    }

    fn start(
        &mut self,
        task_id: &TaskId,
        _source: &str,
        _destination: &str,
    ) -> Result<EngineTask, EngineError> {
        self.next_handle += 1;
        let handle = format!("memory-{}", self.next_handle);
        self.tasks.insert(
            handle.clone(),
            (
                task_id.clone(),
                EngineTaskState::Downloading,
                Progress::default(),
            ),
        );
        Ok(EngineTask {
            task_id: task_id.clone(),
            handle,
        })
    }

    fn pause(&mut self, task: &EngineTask) -> Result<(), EngineError> {
        let entry = self.entry_mut(task)?;
        if entry.1 == EngineTaskState::Completed {
            return Err(EngineError::Failed("cannot pause a completed task".into()));
        }
        entry.1 = EngineTaskState::Paused;
        Ok(())
    }

    fn resume(&mut self, task: &EngineTask) -> Result<(), EngineError> {
        let entry = self.entry_mut(task)?;
        if entry.1 != EngineTaskState::Paused {
            return Err(EngineError::Failed("task is not paused".into()));
        }
        entry.1 = EngineTaskState::Downloading;
        Ok(())
    }

    fn remove(&mut self, task: &EngineTask) -> Result<(), EngineError> {
        self.tasks
            .remove(&task.handle)
            .map(|_| ())
            .ok_or_else(|| EngineError::TaskNotFound(task.task_id.clone()))
    }

    fn progress(&self, task: &EngineTask) -> Result<Progress, EngineError> {
        Ok(self.entry(task)?.2.clone())
    }

    fn state(&self, task: &EngineTask) -> Result<EngineTaskState, EngineError> {
        Ok(self.entry(task)?.1)
    }
}

pub struct HttpEngine {
    client: reqwest::blocking::Client,
    tasks: std::collections::HashMap<String, (TaskId, EngineTaskState, Progress, String)>,
    next_handle: u64,
}

/// Shared control state for a blocking HTTP transfer.
///
/// A control can be cloned and sent to the thread running
/// [`HttpEngine::download_to_with_control`]. Pause requests are acknowledged at
/// a response chunk boundary, so [`Self::request_pause`] does not return until
/// the worker is no longer reading the response. Cancellation wakes a paused
/// worker and makes the download return [`EngineError::Cancelled`].
#[derive(Clone, Default)]
pub struct HttpTransferControl {
    inner: Arc<(Mutex<HttpTransferControlState>, Condvar)>,
}

#[derive(Default)]
struct HttpTransferControlState {
    pause_requested: bool,
    paused: bool,
    cancelled: bool,
    committed: bool,
    finished: bool,
}

impl HttpTransferControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests a pause and waits until the worker reaches a chunk boundary.
    ///
    /// A transfer which has already finished has no work left to pause and is
    /// treated as successfully paused. If cancellation wins the race, the
    /// request returns [`EngineError::Cancelled`].
    pub fn request_pause(&self) -> Result<(), EngineError> {
        let (lock, wake) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.cancelled {
            return Err(EngineError::Cancelled);
        }
        if state.finished || state.paused {
            return Ok(());
        }

        state.pause_requested = true;
        wake.notify_all();
        while !state.paused && !state.cancelled && !state.finished {
            state = wake
                .wait(state)
                .expect("HTTP transfer control mutex poisoned while waiting");
        }

        if state.cancelled {
            Err(EngineError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Clears a pause request and wakes a worker waiting at a chunk boundary.
    pub fn resume(&self) -> Result<(), EngineError> {
        let (lock, wake) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.cancelled {
            return Err(EngineError::Cancelled);
        }
        state.pause_requested = false;
        state.paused = false;
        wake.notify_all();
        Ok(())
    }

    /// Cancels the transfer. This method is idempotent and wakes a paused
    /// worker so it can return [`EngineError::Cancelled`].
    pub fn cancel(&self) {
        let (lock, wake) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.finished {
            return;
        }
        state.cancelled = true;
        state.pause_requested = false;
        state.paused = false;
        wake.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        self.with_state(|state| state.paused)
    }

    pub fn is_pause_requested(&self) -> bool {
        self.with_state(|state| state.pause_requested)
    }

    pub fn is_cancelled(&self) -> bool {
        self.with_state(|state| state.cancelled)
    }

    /// Returns whether the complete response was committed to the destination.
    pub fn is_committed(&self) -> bool {
        self.with_state(|state| state.committed)
    }

    pub fn is_finished(&self) -> bool {
        self.with_state(|state| state.finished)
    }

    fn with_state<T>(&self, read: impl FnOnce(&HttpTransferControlState) -> T) -> T {
        let (lock, _) = &*self.inner;
        let state = lock.lock().expect("HTTP transfer control mutex poisoned");
        read(&state)
    }

    fn wait_if_paused(&self) -> Result<(), EngineError> {
        let (lock, wake) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.cancelled {
            return Err(EngineError::Cancelled);
        }
        if state.pause_requested {
            state.paused = true;
            wake.notify_all();
            while state.pause_requested && !state.cancelled {
                state = wake
                    .wait(state)
                    .expect("HTTP transfer control mutex poisoned while waiting");
            }
            state.paused = false;
            wake.notify_all();
        }
        if state.cancelled {
            Err(EngineError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn check_cancelled(&self) -> Result<(), EngineError> {
        if self.is_cancelled() {
            Err(EngineError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn mark_finished(&self) {
        let (lock, wake) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        state.finished = true;
        state.pause_requested = false;
        state.paused = false;
        wake.notify_all();
    }

    fn finish_download(
        &self,
        temporary: TemporaryDownload,
        destination: &Path,
    ) -> Result<(), EngineError> {
        let (lock, wake) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.cancelled {
            return Err(EngineError::Cancelled);
        }

        temporary.finish(destination)?;
        state.committed = true;
        state.finished = true;
        state.pause_requested = false;
        state.paused = false;
        wake.notify_all();
        Ok(())
    }

    fn finish_resumable_download(
        &self,
        temporary: ResumableDownload,
        destination: &Path,
    ) -> Result<(), EngineError> {
        let (lock, wake) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.cancelled {
            return Err(EngineError::Cancelled);
        }

        temporary.finish(destination)?;
        state.committed = true;
        state.finished = true;
        state.pause_requested = false;
        state.paused = false;
        wake.notify_all();
        Ok(())
    }
}

/// The validator that makes a partial HTTP response safe to continue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HttpResumeValidator {
    Etag(String),
    LastModified(String),
}

impl HttpResumeValidator {
    fn from_headers(headers: &reqwest::header::HeaderMap) -> Option<Self> {
        headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(|value| Self::Etag(value.to_owned()))
            .or_else(|| {
                headers
                    .get(LAST_MODIFIED)
                    .and_then(|value| value.to_str().ok())
                    .map(|value| Self::LastModified(value.to_owned()))
            })
    }

    fn value(&self) -> &str {
        match self {
            Self::Etag(value) | Self::LastModified(value) => value,
        }
    }
}

/// Metadata persisted next to a resumable partial response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HttpResumeMetadata {
    pub source: String,
    pub destination: String,
    pub validator: HttpResumeValidator,
    pub expected_length: Option<u64>,
}

/// Returns the stable JSON sidecar path for a resumable partial file.
pub fn resumable_metadata_path(partial_path: impl AsRef<Path>) -> PathBuf {
    let partial_path = partial_path.as_ref();
    let file_name = partial_path
        .file_name()
        .map(|name| {
            let mut name = name.to_os_string();
            name.push(".json");
            name
        })
        .unwrap_or_else(|| "nexum-resume.json".into());
    partial_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(file_name)
}

/// Removes a stable partial response and its metadata sidecar.
pub fn remove_resumable_files(partial_path: impl AsRef<Path>) {
    let partial_path = partial_path.as_ref();
    let _ = std::fs::remove_file(partial_path);
    let _ = std::fs::remove_file(resumable_metadata_path(partial_path));
}

fn read_resume_metadata(partial_path: &Path) -> Result<Option<HttpResumeMetadata>, EngineError> {
    let metadata_path = resumable_metadata_path(partial_path);
    match std::fs::read(&metadata_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| EngineError::Failed(format!("invalid HTTP resume metadata: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(EngineError::Failed(error.to_string())),
    }
}

fn write_resume_metadata(
    partial_path: &Path,
    metadata: &HttpResumeMetadata,
) -> Result<(), EngineError> {
    let metadata_path = resumable_metadata_path(partial_path);
    let parent = metadata_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| EngineError::Failed(error.to_string()))?;
    let bytes = serde_json::to_vec(metadata).map_err(|error| {
        EngineError::Failed(format!("could not encode HTTP resume metadata: {error}"))
    })?;

    for attempt in 0..16u32 {
        let mut temporary_name = metadata_path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_else(|| "nexum-resume.json".into());
        temporary_name.push(format!(".tmp-{}-{attempt}", std::process::id()));
        let temporary_path = parent.join(temporary_name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(mut file) => {
                let result = (|| {
                    file.write_all(&bytes)
                        .map_err(|error| EngineError::Failed(error.to_string()))?;
                    file.sync_all()
                        .map_err(|error| EngineError::Failed(error.to_string()))?;
                    drop(file);
                    std::fs::rename(&temporary_path, &metadata_path)
                        .map_err(|error| EngineError::Failed(error.to_string()))
                })();
                let _ = std::fs::remove_file(&temporary_path);
                return result;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(EngineError::Failed(error.to_string())),
        }
    }

    Err(EngineError::Failed(
        "could not allocate a temporary HTTP resume metadata file".into(),
    ))
}

fn ensure_destination_absent(destination: &Path) -> Result<(), EngineError> {
    match std::fs::symlink_metadata(destination) {
        Ok(_) => Err(EngineError::Failed(format!(
            "download destination already exists: {}",
            destination.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(EngineError::Failed(format!(
            "could not inspect download destination {}: {error}",
            destination.display()
        ))),
    }
}

#[cfg(target_os = "macos")]
fn rename_without_replacing(partial: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let partial = CString::new(partial.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    // SAFETY: Both paths are NUL-terminated and remain alive for the call.
    if unsafe { libc::renamex_np(partial.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn rename_without_replacing(partial: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let partial = CString::new(partial.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    // SAFETY: Both paths are NUL-terminated and remain alive for the call.
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            partial.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn rename_without_replacing(partial: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let partial = partial
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both paths are NUL-terminated and remain alive for the call.
    // Omitting MOVEFILE_REPLACE_EXISTING makes this an atomic no-clobber move.
    if unsafe { MoveFileExW(partial.as_ptr(), destination.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn rename_without_replacing(_partial: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn no_replace_rename_unsupported(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::ENOTSUP)
    )
}

#[cfg(windows)]
fn no_replace_rename_unsupported(_error: &std::io::Error) -> bool {
    false
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn no_replace_rename_unsupported(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::Unsupported
}

/// Publishes a fully written partial file without replacing an existing path.
fn commit_download(partial: &Path, destination: &Path) -> Result<(), EngineError> {
    commit_download_with_rename(partial, destination, rename_without_replacing)
}

fn commit_download_with_rename(
    partial: &Path,
    destination: &Path,
    rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), EngineError> {
    let result = match rename(partial, destination) {
        Ok(()) => return Ok(()),
        Err(error) if no_replace_rename_unsupported(&error) => {
            // On older kernels or filesystems without no-clobber rename, hard
            // links retain the atomic publish guarantee. If neither operation
            // is supported, fail rather than expose an empty placeholder file.
            match std::fs::hard_link(partial, destination) {
                Ok(()) => Ok(()),
                Err(link_error) if link_error.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(link_error)
                }
                Err(link_error) => {
                    return Err(EngineError::Failed(format!(
                        "could not atomically commit download to {}: no-clobber rename unavailable ({error}); hard-link fallback failed ({link_error})",
                        destination.display()
                    )));
                }
            }
        }
        Err(error) => Err(error),
    };
    result.map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            EngineError::Failed(format!(
                "download destination already exists: {}",
                destination.display()
            ))
        } else {
            EngineError::Failed(format!(
                "could not commit download to {}: {error}",
                destination.display()
            ))
        }
    })
}

/// Reads a valid partial response's progress without changing it.
pub fn resumable_partial_progress(
    source: &str,
    destination: &str,
    partial_path: impl AsRef<Path>,
) -> Result<Option<Progress>, EngineError> {
    let partial_path = partial_path.as_ref();
    let Some(metadata) = read_resume_metadata(partial_path)? else {
        return Ok(None);
    };
    if metadata.source != source || metadata.destination != destination {
        return Ok(None);
    }
    let downloaded = match std::fs::symlink_metadata(partial_path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata.len(),
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(EngineError::Failed(error.to_string())),
    };
    if downloaded == 0
        || metadata
            .expected_length
            .is_some_and(|total| downloaded > total)
    {
        return Ok(None);
    }
    Ok(Some(Progress::new(downloaded, metadata.expected_length)))
}

struct TemporaryDownload {
    path: PathBuf,
    file: Option<std::fs::File>,
}

impl TemporaryDownload {
    fn create(destination: &Path) -> Result<Self, EngineError> {
        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| EngineError::Failed(error.to_string()))?;

        for _ in 0..16 {
            let sequence = NEXT_TEMP_DOWNLOAD.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".nexum-download-{}-{sequence}.part",
                std::process::id()
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(EngineError::Failed(error.to_string())),
            }
        }

        Err(EngineError::Failed(
            "could not allocate a temporary download file".into(),
        ))
    }

    fn finish(mut self, destination: &Path) -> Result<(), EngineError> {
        self.file
            .as_ref()
            .expect("temporary download file is open")
            .sync_all()
            .map_err(|error| EngineError::Failed(error.to_string()))?;
        drop(self.file.take());
        commit_download(&self.path, destination)
    }
}

impl Drop for TemporaryDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

struct ResumableDownload {
    path: PathBuf,
    metadata_path: PathBuf,
    file: Option<std::fs::File>,
}

impl ResumableDownload {
    fn open(partial_path: &Path, append: bool) -> Result<Self, EngineError> {
        let parent = partial_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| EngineError::Failed(error.to_string()))?;
        let file = if append {
            std::fs::OpenOptions::new().append(true).open(partial_path)
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(partial_path)
        }
        .map_err(|error| EngineError::Failed(error.to_string()))?;
        Ok(Self {
            path: partial_path.to_owned(),
            metadata_path: resumable_metadata_path(partial_path),
            file: Some(file),
        })
    }

    fn finish(mut self, destination: &Path) -> Result<(), EngineError> {
        self.file
            .as_ref()
            .expect("resumable download file is open")
            .sync_all()
            .map_err(|error| EngineError::Failed(error.to_string()))?;
        drop(self.file.take());
        commit_download(&self.path, destination)?;
        // The destination is visible and complete now. Cleanup cannot turn a
        // committed response back into a failed one, even if removal fails.
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.metadata_path);
        Ok(())
    }
}

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end = end.parse().ok()?;
    let total = total.parse().ok()?;
    (end >= start && total > end).then_some((start, end, total))
}

impl Default for HttpEngine {
    fn default() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(5))
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30 * 60))
                .build()
                .expect("failed to build http client"),
            tasks: std::collections::HashMap::new(),
            next_handle: 0,
        }
    }
}

impl HttpEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(
        &self,
        task: &EngineTask,
    ) -> Result<&(TaskId, EngineTaskState, Progress, String), EngineError> {
        self.tasks
            .get(&task.handle)
            .ok_or_else(|| EngineError::TaskNotFound(task.task_id.clone()))
    }

    /// Downloads a complete HTTP response before committing the destination.
    ///
    /// This call blocks and may be run from a worker without holding Core's lock.
    pub fn download_to(&self, source: &str, destination: &str) -> Result<Progress, EngineError> {
        self.download_to_with_progress(source, destination, |_| Ok(()))
    }

    /// Downloads a complete HTTP response and reports each written chunk.
    ///
    /// The callback runs after a chunk has been written to the temporary file.
    /// Returning an error aborts the transfer and leaves the final destination
    /// untouched.
    pub fn download_to_with_progress<F>(
        &self,
        source: &str,
        destination: &str,
        on_progress: F,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        let control = HttpTransferControl::new();
        self.download_to_with_control(source, destination, &control, on_progress)
    }

    /// Downloads a complete HTTP response with cooperative pause and cancel
    /// control, reporting each written chunk through `on_progress`.
    ///
    /// Pause requests take effect at a response chunk boundary. Cancellation
    /// drops the temporary file and leaves the final destination untouched.
    pub fn download_to_with_control<F>(
        &self,
        source: &str,
        destination: &str,
        control: &HttpTransferControl,
        mut on_progress: F,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        let result =
            self.download_to_with_control_inner(source, destination, control, &mut on_progress);
        control.mark_finished();
        result
    }

    /// Downloads an HTTP response into a stable partial file and resumes it
    /// after a process restart when the saved validator and server range match.
    ///
    /// The partial file is published at `destination` only after the complete
    /// response has been received, and an existing destination is never replaced.
    /// Ordinary transfer errors keep a validated
    /// partial response; cancellation removes it together with its sidecar.
    pub fn download_to_resumable<F>(
        &self,
        source: &str,
        destination: &str,
        partial_path: impl AsRef<Path>,
        on_progress: F,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        let control = HttpTransferControl::new();
        self.download_to_resumable_with_control(
            source,
            destination,
            partial_path,
            &control,
            on_progress,
        )
    }

    /// Downloads an HTTP response with cooperative pause and cancel control
    /// using a stable partial file and validator sidecar.
    pub fn download_to_resumable_with_control<F>(
        &self,
        source: &str,
        destination: &str,
        partial_path: impl AsRef<Path>,
        control: &HttpTransferControl,
        mut on_progress: F,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        let partial_path = partial_path.as_ref();
        let result = self.download_to_resumable_inner(
            source,
            destination,
            partial_path,
            control,
            &mut on_progress,
        );
        if matches!(result, Err(EngineError::Cancelled))
            || (control.is_cancelled() && !control.is_committed())
        {
            remove_resumable_files(partial_path);
        }
        control.mark_finished();
        result
    }

    fn download_to_resumable_inner<F>(
        &self,
        source: &str,
        destination: &str,
        partial_path: &Path,
        control: &HttpTransferControl,
        on_progress: &mut F,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        control.check_cancelled()?;
        // The hard-link fallback can leave both paths pointing at the same
        // inode if a crash happens before cleanup. Never open or truncate that
        // partial while a final destination already exists.
        ensure_destination_absent(Path::new(destination))?;

        let mut candidate = match read_resume_metadata(partial_path) {
            Ok(Some(metadata)) => {
                let valid_identity =
                    metadata.source == source && metadata.destination == destination;
                let partial_length = std::fs::symlink_metadata(partial_path)
                    .ok()
                    .filter(|metadata| metadata.file_type().is_file())
                    .map(|metadata| metadata.len());
                if valid_identity
                    && partial_length.is_some_and(|length| {
                        length > 0 && metadata.expected_length.is_none_or(|total| length <= total)
                    })
                {
                    Some((
                        metadata,
                        partial_length.expect("partial length was checked"),
                    ))
                } else {
                    remove_resumable_files(partial_path);
                    None
                }
            }
            Ok(None) => {
                if partial_path.exists() {
                    remove_resumable_files(partial_path);
                }
                None
            }
            Err(_) => {
                remove_resumable_files(partial_path);
                None
            }
        };
        let mut preserve_partial = candidate.is_some();

        let mut request = self.client.get(source);
        if let Some((metadata, offset)) = candidate.as_ref() {
            let range = format!("bytes={offset}-");
            request = request
                .header(RANGE, range)
                .header(IF_RANGE, metadata.validator.value());
        }
        let mut response = request
            .send()
            .map_err(|error| EngineError::Failed(error.to_string()))?;

        let (offset, total, append) = if let Some((saved, offset)) = candidate.take() {
            let response_validator = HttpResumeValidator::from_headers(response.headers());
            let valid_range = response.status().is_success()
                && response.status() == StatusCode::PARTIAL_CONTENT
                && response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_content_range)
                    .is_some_and(|(start, end, total)| {
                        start == offset
                            && saved
                                .expected_length
                                .is_none_or(|expected| expected == total)
                            && response
                                .content_length()
                                .is_none_or(|length| length == end - start + 1)
                    })
                && response_validator.as_ref() == Some(&saved.validator);

            if valid_range {
                let (_, _, total) = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_content_range)
                    .expect("validated content range");
                let metadata = HttpResumeMetadata {
                    source: source.to_owned(),
                    destination: destination.to_owned(),
                    validator: saved.validator,
                    expected_length: Some(total),
                };
                write_resume_metadata(partial_path, &metadata)?;
                (offset, Some(total), true)
            } else if response.status() == StatusCode::OK
                || response.status() == StatusCode::PARTIAL_CONTENT
            {
                remove_resumable_files(partial_path);
                preserve_partial = false;
                response = self
                    .client
                    .get(source)
                    .send()
                    .map_err(|error| EngineError::Failed(error.to_string()))?;
                if !response.status().is_success() {
                    return Err(EngineError::Failed(format!(
                        "HTTP GET returned {}",
                        response.status()
                    )));
                }
                let validator = HttpResumeValidator::from_headers(response.headers());
                let total = response.content_length();
                let metadata = validator.map(|validator| HttpResumeMetadata {
                    source: source.to_owned(),
                    destination: destination.to_owned(),
                    validator,
                    expected_length: total,
                });
                if let Some(metadata) = metadata.as_ref() {
                    write_resume_metadata(partial_path, metadata)?;
                    preserve_partial = true;
                } else {
                    let _ = std::fs::remove_file(resumable_metadata_path(partial_path));
                }
                (0, total, false)
            } else {
                return Err(EngineError::Failed(format!(
                    "HTTP GET returned {}",
                    response.status()
                )));
            }
        } else {
            if !response.status().is_success() {
                return Err(EngineError::Failed(format!(
                    "HTTP GET returned {}",
                    response.status()
                )));
            }
            let validator = HttpResumeValidator::from_headers(response.headers());
            let total = response.content_length();
            let metadata = validator.map(|validator| HttpResumeMetadata {
                source: source.to_owned(),
                destination: destination.to_owned(),
                validator,
                expected_length: total,
            });
            if let Some(metadata) = metadata.as_ref() {
                write_resume_metadata(partial_path, metadata)?;
                preserve_partial = true;
            } else {
                let _ = std::fs::remove_file(resumable_metadata_path(partial_path));
            }
            (0, total, false)
        };

        let mut temporary = ResumableDownload::open(partial_path, append)?;
        let mut downloaded = offset;
        let mut buffer = [0u8; 32 * 1024];
        let result = (|| {
            loop {
                control.wait_if_paused()?;
                let read = response
                    .read(&mut buffer)
                    .map_err(|error| EngineError::Failed(error.to_string()))?;
                if read == 0 {
                    break;
                }
                control.check_cancelled()?;
                temporary
                    .file
                    .as_mut()
                    .expect("resumable download file is open")
                    .write_all(&buffer[..read])
                    .map_err(|error| EngineError::Failed(error.to_string()))?;
                downloaded += read as u64;
                on_progress(Progress::new(downloaded, total))?;
                control.wait_if_paused()?;
            }

            control.check_cancelled()?;
            if total.is_some_and(|expected| downloaded != expected) {
                return Err(EngineError::Failed(format!(
                    "incomplete HTTP response: expected {} bytes, received {downloaded}",
                    total.expect("total was checked")
                )));
            }
            control.check_cancelled()?;
            control.finish_resumable_download(temporary, Path::new(destination))?;
            Ok(Progress::new(downloaded, total))
        })();

        if result.is_err() && (!preserve_partial || matches!(result, Err(EngineError::Cancelled))) {
            remove_resumable_files(partial_path);
        }
        result
    }

    fn download_to_with_control_inner<F>(
        &self,
        source: &str,
        destination: &str,
        control: &HttpTransferControl,
        on_progress: &mut F,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        control.check_cancelled()?;
        let mut response = self
            .client
            .get(source)
            .send()
            .map_err(|error| EngineError::Failed(error.to_string()))?;
        control.check_cancelled()?;
        if !response.status().is_success() {
            return Err(EngineError::Failed(format!(
                "HTTP GET returned {}",
                response.status()
            )));
        }

        let total = response.content_length();
        let destination = Path::new(destination);
        let mut temporary = TemporaryDownload::create(destination)?;
        let mut downloaded = 0u64;
        let mut buffer = [0u8; 32 * 1024];

        loop {
            control.wait_if_paused()?;
            let read = response
                .read(&mut buffer)
                .map_err(|error| EngineError::Failed(error.to_string()))?;
            if read == 0 {
                break;
            }
            control.check_cancelled()?;
            temporary
                .file
                .as_mut()
                .expect("temporary download file is open")
                .write_all(&buffer[..read])
                .map_err(|error| EngineError::Failed(error.to_string()))?;
            downloaded += read as u64;
            on_progress(Progress::new(downloaded, total))?;
            control.wait_if_paused()?;
        }

        control.check_cancelled()?;
        if let Some(expected) = total
            && downloaded != expected
        {
            return Err(EngineError::Failed(format!(
                "incomplete HTTP response: expected {expected} bytes, received {downloaded}"
            )));
        }

        control.check_cancelled()?;
        control.finish_download(temporary, destination)?;
        Ok(Progress::new(downloaded, total))
    }
}

impl EngineAdapter for HttpEngine {
    fn name(&self) -> &str {
        "http"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            supports_pause: false,
            supports_resume: false,
            supports_remove: true,
            supports_progress: true,
        }
    }

    fn start(
        &mut self,
        task_id: &TaskId,
        source: &str,
        destination: &str,
    ) -> Result<EngineTask, EngineError> {
        let progress = self.download_to(source, destination)?;
        self.next_handle += 1;
        let handle = format!("http-{}", self.next_handle);

        self.tasks.insert(
            handle.clone(),
            (
                task_id.clone(),
                EngineTaskState::Completed,
                progress,
                destination.to_owned(),
            ),
        );

        Ok(EngineTask {
            task_id: task_id.clone(),
            handle,
        })
    }

    fn pause(&mut self, _task: &EngineTask) -> Result<(), EngineError> {
        Err(EngineError::UnsupportedOperation("pause"))
    }

    fn resume(&mut self, _task: &EngineTask) -> Result<(), EngineError> {
        Err(EngineError::UnsupportedOperation("resume"))
    }

    fn remove(&mut self, task: &EngineTask) -> Result<(), EngineError> {
        self.tasks
            .remove(&task.handle)
            .map(|_| ())
            .ok_or_else(|| EngineError::TaskNotFound(task.task_id.clone()))
    }

    fn progress(&self, task: &EngineTask) -> Result<Progress, EngineError> {
        Ok(self.entry(task)?.2.clone())
    }

    fn state(&self, task: &EngineTask) -> Result<EngineTaskState, EngineError> {
        Ok(self.entry(task)?.1)
    }
}

pub struct EngineRegistry {
    engines: Vec<Box<dyn EngineAdapter + Send>>,
}

impl Default for EngineRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self {
            engines: Vec::new(),
        }
    }

    pub fn register(&mut self, engine: Box<dyn EngineAdapter + Send>) {
        self.engines.push(engine);
    }

    pub fn get(&self, name: &str) -> Option<&(dyn EngineAdapter + Send)> {
        self.engines
            .iter()
            .find(|engine| engine.name() == name)
            .map(|engine| engine.as_ref())
    }

    pub fn start_engine(
        &mut self,
        name: &str,
        task_id: &TaskId,
        source: &str,
        destination: &str,
    ) -> Result<EngineTask, EngineError> {
        self.engines
            .iter_mut()
            .find(|engine| engine.name() == name)
            .map(|engine| engine.start(task_id, source, destination))
            .ok_or_else(|| EngineError::Failed(format!("engine not found: {name}")))
            .unwrap_or_else(Err)
    }

    pub fn pause_engine(&mut self, name: &str, task: &EngineTask) -> Result<(), EngineError> {
        self.engines
            .iter_mut()
            .find(|engine| engine.name() == name)
            .ok_or_else(|| EngineError::Failed(format!("engine not found: {name}")))?
            .pause(task)
    }

    pub fn resume_engine(&mut self, name: &str, task: &EngineTask) -> Result<(), EngineError> {
        self.engines
            .iter_mut()
            .find(|engine| engine.name() == name)
            .ok_or_else(|| EngineError::Failed(format!("engine not found: {name}")))?
            .resume(task)
    }

    pub fn remove_engine(&mut self, name: &str, task: &EngineTask) -> Result<(), EngineError> {
        self.engines
            .iter_mut()
            .find(|engine| engine.name() == name)
            .ok_or_else(|| EngineError::Failed(format!("engine not found: {name}")))?
            .remove(task)
    }

    pub fn names(&self) -> Vec<&str> {
        self.engines.iter().map(|engine| engine.name()).collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskMapping {
    pub task_id: TaskId,
    pub engine_task: EngineTask,
    pub state: TaskState,
}

impl TaskMapping {
    pub fn new(task_id: TaskId, engine_task: EngineTask) -> Self {
        Self {
            task_id,
            engine_task,
            state: TaskState::Downloading,
        }
    }

    pub fn apply_state(&mut self, state: TaskState) {
        self.state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nexum-engine-test-{}-{}",
                std::process::id(),
                NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn serve_once(response: &'static [u8]) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/file", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut method = [0u8; 3];
            stream.read_exact(&mut method).unwrap();
            assert_eq!(&method, b"GET");
            stream.write_all(response).unwrap();
        });
        (address, server)
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        String::from_utf8(request).unwrap()
    }

    fn serve_dynamic<F>(responses: usize, mut handler: F) -> (String, thread::JoinHandle<()>)
    where
        F: FnMut(usize, &str) -> Vec<u8> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/file", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for index in 0..responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request = read_http_request(&mut stream);
                let response = handler(index, &request);
                stream.write_all(&response).unwrap();
            }
        });
        (address, server)
    }

    fn http_response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn http_response_with_declared_length(
        status: &str,
        headers: &str,
        declared_length: usize,
        body: &[u8],
    ) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {declared_length}\r\nConnection: close\r\n\r\n"
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn serve_in_chunks(
        first: Vec<u8>,
        second: Vec<u8>,
    ) -> (
        String,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/file", listener.local_addr().unwrap());
        let (first_sent_tx, first_sent) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut method = [0u8; 3];
            stream.read_exact(&mut method).unwrap();
            assert_eq!(&method, b"GET");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                first.len() + second.len()
            )
            .unwrap();
            stream.write_all(&first).unwrap();
            stream.flush().unwrap();
            first_sent_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            let _ = stream.write_all(&second);
        });
        (address, first_sent, release_tx, worker)
    }

    fn pause_at_first_progress(
        control: &HttpTransferControl,
        progress: mpsc::Receiver<Progress>,
        release_progress: mpsc::Sender<()>,
    ) -> Progress {
        let first_progress = progress.recv_timeout(Duration::from_secs(3)).unwrap();
        let pause_control = control.clone();
        let (paused_tx, paused_rx) = mpsc::channel();
        let requester = thread::spawn(move || {
            paused_tx.send(pause_control.request_pause()).unwrap();
        });

        // Keep the worker in its progress callback until the pause request is
        // registered, so it cannot enter the next blocking response read.
        let deadline = Instant::now() + Duration::from_secs(3);
        while !control.is_pause_requested() {
            assert!(
                Instant::now() < deadline,
                "pause request was not registered"
            );
            thread::yield_now();
        }
        release_progress.send(()).unwrap();
        paused_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        requester.join().unwrap();
        assert!(control.is_paused());
        first_progress
    }

    #[derive(Default)]
    struct FakeEngine;

    impl EngineAdapter for FakeEngine {
        fn name(&self) -> &str {
            "fake"
        }
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities::BASIC
        }
        fn start(
            &mut self,
            task_id: &TaskId,
            _source: &str,
            _destination: &str,
        ) -> Result<EngineTask, EngineError> {
            Ok(EngineTask {
                task_id: task_id.clone(),
                handle: "fake-1".into(),
            })
        }
        fn pause(&mut self, _task: &EngineTask) -> Result<(), EngineError> {
            Err(EngineError::UnsupportedOperation("pause"))
        }
        fn resume(&mut self, _task: &EngineTask) -> Result<(), EngineError> {
            Err(EngineError::UnsupportedOperation("resume"))
        }
        fn remove(&mut self, _task: &EngineTask) -> Result<(), EngineError> {
            Err(EngineError::UnsupportedOperation("remove"))
        }
        fn progress(&self, _task: &EngineTask) -> Result<Progress, EngineError> {
            Ok(Progress::new(42, Some(100)))
        }
        fn state(&self, _task: &EngineTask) -> Result<EngineTaskState, EngineError> {
            Ok(EngineTaskState::Downloading)
        }
    }

    #[test]
    fn http_engine_rejects_invalid_endpoint_without_creating_task() {
        let mut engine = HttpEngine::new();
        let id = TaskId::from("http-task");
        let result = engine.start(&id, "http://127.0.0.1:1/not-found", "/tmp/file");
        assert!(matches!(result, Err(EngineError::Failed(_))));
    }

    #[test]
    fn http_engine_writes_complete_response_and_reports_completion() {
        let dir = TestDir::new();
        let destination = dir.0.join("nested").join("file.bin");
        let (source, server) =
            serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        let mut engine = HttpEngine::new();

        let task = engine
            .start(
                &TaskId::from("success"),
                &source,
                destination.to_str().unwrap(),
            )
            .unwrap();

        server.join().unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"hello");
        assert_eq!(engine.progress(&task).unwrap(), Progress::new(5, Some(5)));
        assert_eq!(engine.state(&task).unwrap(), EngineTaskState::Completed);
        assert_eq!(
            std::fs::read_dir(destination.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn http_engine_reports_incremental_progress_after_each_chunk() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let first = vec![b'a'; 32 * 1024];
        let second = vec![b'b'; 32 * 1024];
        let (source, first_sent, release, server) = serve_in_chunks(first, second);
        let (progress_tx, progress_rx) = mpsc::channel();
        let source_for_worker = source.clone();
        let destination_for_worker = destination.to_str().unwrap().to_owned();
        let worker = thread::spawn(move || {
            HttpEngine::new().download_to_with_progress(
                &source_for_worker,
                &destination_for_worker,
                |progress| {
                    progress_tx.send(progress.clone()).unwrap();
                    Ok(())
                },
            )
        });

        first_sent.recv().unwrap();
        let first_progress = progress_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(first_progress.downloaded_bytes > 0);
        assert!(first_progress.downloaded_bytes < 64 * 1024);
        assert_eq!(first_progress.total_bytes, Some(64 * 1024));
        release.send(()).unwrap();

        let result = worker.join().unwrap().unwrap();
        server.join().unwrap();
        assert_eq!(result, Progress::new(64 * 1024, Some(64 * 1024)));
        assert_eq!(std::fs::read(&destination).unwrap().len(), 64 * 1024);
    }

    #[test]
    fn controlled_http_download_pauses_and_resumes_at_chunk_boundary() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let first = vec![b'a'; 32 * 1024];
        let second = vec![b'b'; 32 * 1024];
        let (source, first_sent, release, server) = serve_in_chunks(first, second);
        let control = HttpTransferControl::new();
        let (progress_tx, progress_rx) = mpsc::channel();
        let (release_progress_tx, release_progress_rx) = mpsc::channel();
        let source_for_worker = source.clone();
        let destination_for_worker = destination.to_str().unwrap().to_owned();
        let control_for_worker = control.clone();
        let worker = thread::spawn(move || {
            let mut first_progress = Some(progress_tx);
            HttpEngine::new().download_to_with_control(
                &source_for_worker,
                &destination_for_worker,
                &control_for_worker,
                |progress| {
                    if let Some(progress_tx) = first_progress.take() {
                        progress_tx.send(progress).unwrap();
                        release_progress_rx.recv().unwrap();
                    }
                    Ok(())
                },
            )
        });

        first_sent.recv().unwrap();
        let first_progress = pause_at_first_progress(&control, progress_rx, release_progress_tx);
        assert!(first_progress.downloaded_bytes > 0);
        assert!(first_progress.downloaded_bytes < 64 * 1024);
        assert!(!destination.exists());

        control.resume().unwrap();
        release.send(()).unwrap();
        let result = worker.join().unwrap().unwrap();
        server.join().unwrap();

        assert_eq!(result, Progress::new(64 * 1024, Some(64 * 1024)));
        assert_eq!(std::fs::read(&destination).unwrap().len(), 64 * 1024);
        assert!(control.is_finished());
    }

    #[test]
    fn controlled_http_download_cancels_without_replacing_destination() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        std::fs::write(&destination, b"old bytes").unwrap();
        let first = vec![b'a'; 32 * 1024];
        let second = vec![b'b'; 32 * 1024];
        let (source, first_sent, release, server) = serve_in_chunks(first, second);
        let control = HttpTransferControl::new();
        let (progress_tx, progress_rx) = mpsc::channel();
        let (release_progress_tx, release_progress_rx) = mpsc::channel();
        let source_for_worker = source.clone();
        let destination_for_worker = destination.to_str().unwrap().to_owned();
        let control_for_worker = control.clone();
        let worker = thread::spawn(move || {
            let mut first_progress = Some(progress_tx);
            HttpEngine::new().download_to_with_control(
                &source_for_worker,
                &destination_for_worker,
                &control_for_worker,
                |progress| {
                    if let Some(progress_tx) = first_progress.take() {
                        progress_tx.send(progress).unwrap();
                        release_progress_rx.recv().unwrap();
                    }
                    Ok(())
                },
            )
        });

        first_sent.recv().unwrap();
        pause_at_first_progress(&control, progress_rx, release_progress_tx);
        control.cancel();
        assert!(matches!(
            worker.join().unwrap(),
            Err(EngineError::Cancelled)
        ));
        release.send(()).unwrap();
        server.join().unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"old bytes");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
        assert!(control.is_finished());
    }

    #[test]
    fn incomplete_http_response_preserves_existing_destination() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        std::fs::write(&destination, b"old bytes").unwrap();
        let (source, server) =
            serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nshort");

        let result = HttpEngine::new().download_to(&source, destination.to_str().unwrap());

        server.join().unwrap();
        assert!(matches!(result, Err(EngineError::Failed(_))));
        assert_eq!(std::fs::read(&destination).unwrap(), b"old bytes");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn completed_temporary_download_publishes_one_file() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let mut temporary = TemporaryDownload::create(&destination).unwrap();
        temporary
            .file
            .as_mut()
            .unwrap()
            .write_all(b"complete bytes")
            .unwrap();

        temporary.finish(&destination).unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"complete bytes");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn unsupported_no_replace_rename_uses_atomic_hard_link_fallback() {
        let dir = TestDir::new();
        let partial = dir.0.join("file.part");
        let destination = dir.0.join("file.bin");
        std::fs::write(&partial, b"complete bytes").unwrap();

        commit_download_with_rename(&partial, &destination, |_, _| {
            Err(std::io::Error::from_raw_os_error(libc::ENOTSUP))
        })
        .unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"complete bytes");
        assert_eq!(std::fs::read(&partial).unwrap(), b"complete bytes");
    }

    #[cfg(unix)]
    #[test]
    fn completed_download_does_not_replace_a_dangling_symlink() {
        let dir = TestDir::new();
        let partial = dir.0.join("file.part");
        let destination = dir.0.join("file.bin");
        std::fs::write(&partial, b"complete bytes").unwrap();
        std::os::unix::fs::symlink("missing-target", &destination).unwrap();

        let result = commit_download(&partial, &destination);

        assert!(
            matches!(result, Err(EngineError::Failed(message)) if message.contains("destination already exists"))
        );
        assert_eq!(std::fs::read(&partial).unwrap(), b"complete bytes");
        assert_eq!(
            std::fs::read_link(&destination).unwrap(),
            Path::new("missing-target")
        );
    }

    #[test]
    fn completed_http_response_does_not_replace_existing_destination() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        std::fs::write(&destination, b"original bytes").unwrap();
        let (source, server) =
            serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");

        let result = HttpEngine::new().download_to(&source, destination.to_str().unwrap());

        server.join().unwrap();
        assert!(
            matches!(result, Err(EngineError::Failed(message)) if message.contains("destination already exists"))
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"original bytes");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn resumable_http_response_does_not_replace_destination_created_during_transfer() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let partial = dir.0.join(".file.bin.nexum.part");
        let (source, server) =
            serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        let control = HttpTransferControl::new();

        let result = HttpEngine::new().download_to_resumable_with_control(
            &source,
            destination.to_str().unwrap(),
            &partial,
            &control,
            |_| {
                std::fs::write(&destination, b"another file")
                    .map_err(|error| EngineError::Failed(error.to_string()))
            },
        );

        server.join().unwrap();
        assert!(
            matches!(result, Err(EngineError::Failed(message)) if message.contains("destination already exists"))
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"another file");
        assert!(!partial.exists());
        assert!(!resumable_metadata_path(&partial).exists());
        assert!(!control.is_committed());
    }

    #[test]
    fn resumable_http_recovery_never_truncates_a_linked_completed_file() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let partial = dir.0.join(".file.bin.nexum.part");
        std::fs::write(&partial, b"completed bytes").unwrap();
        std::fs::hard_link(&partial, &destination).unwrap();
        let control = HttpTransferControl::new();

        let result = HttpEngine::new().download_to_resumable_with_control(
            "http://127.0.0.1:9/file",
            destination.to_str().unwrap(),
            &partial,
            &control,
            |_| Ok(()),
        );

        assert!(
            matches!(result, Err(EngineError::Failed(message)) if message.contains("destination already exists"))
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"completed bytes");
        assert_eq!(std::fs::read(&partial).unwrap(), b"completed bytes");
        assert!(!control.is_committed());
        assert!(control.is_finished());
    }

    #[test]
    fn resumable_http_download_uses_matching_etag_range() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let partial = dir.0.join(".file.bin.nexum.part");
        let body = b"hello resumable world";
        let offset = 6;
        std::fs::write(&partial, &body[..offset]).unwrap();
        let body_for_server = body.to_vec();
        let (url, server) = serve_dynamic(1, move |_, request| {
            let request = request.to_ascii_lowercase();
            assert!(request.contains("range: bytes=6-"), "{request}");
            assert!(request.contains("if-range: \"v1\""), "{request}");
            http_response(
                "206 Partial Content",
                &format!(
                    "ETag: \"v1\"\r\nContent-Range: bytes 6-{end}/{total}\r\n",
                    end = body_for_server.len() - 1,
                    total = body_for_server.len()
                ),
                &body_for_server[6..],
            )
        });
        write_resume_metadata(
            &partial,
            &HttpResumeMetadata {
                source: url.clone(),
                destination: destination.to_string_lossy().into_owned(),
                validator: HttpResumeValidator::Etag("\"v1\"".into()),
                expected_length: Some(body.len() as u64),
            },
        )
        .unwrap();
        let result = HttpEngine::new().download_to_resumable_with_control(
            &url,
            destination.to_str().unwrap(),
            &partial,
            &HttpTransferControl::new(),
            |_| Ok(()),
        );
        server.join().unwrap();
        assert_eq!(
            result.unwrap(),
            Progress::new(body.len() as u64, Some(body.len() as u64))
        );
        assert_eq!(std::fs::read(&destination).unwrap(), body);
        assert!(!partial.exists());
        assert!(!resumable_metadata_path(&partial).exists());
    }

    #[test]
    fn resumable_http_download_uses_last_modified_validator() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let partial = dir.0.join(".file.bin.nexum.part");
        let body = b"last modified body";
        std::fs::write(&partial, &body[..5]).unwrap();
        let body_for_server = body.to_vec();
        let (url, server) = serve_dynamic(1, move |_, request| {
            let request = request.to_ascii_lowercase();
            assert!(request.contains("range: bytes=5-"), "{request}");
            assert!(
                request.contains("if-range: wed, 21 oct 2015 07:28:00 gmt"),
                "{request}"
            );
            http_response(
                "206 Partial Content",
                &format!(
                    "Last-Modified: Wed, 21 Oct 2015 07:28:00 GMT\r\nContent-Range: bytes 5-{end}/{total}\r\n",
                    end = body_for_server.len() - 1,
                    total = body_for_server.len()
                ),
                &body_for_server[5..],
            )
        });
        write_resume_metadata(
            &partial,
            &HttpResumeMetadata {
                source: url.clone(),
                destination: destination.to_string_lossy().into_owned(),
                validator: HttpResumeValidator::LastModified(
                    "Wed, 21 Oct 2015 07:28:00 GMT".into(),
                ),
                expected_length: Some(body.len() as u64),
            },
        )
        .unwrap();
        let result = HttpEngine::new().download_to_resumable_with_control(
            &url,
            destination.to_str().unwrap(),
            &partial,
            &HttpTransferControl::new(),
            |_| Ok(()),
        );
        server.join().unwrap();
        assert!(result.is_ok());
        assert_eq!(std::fs::read(&destination).unwrap(), body);
    }

    #[test]
    fn resumable_http_download_restarts_after_200_or_validator_change() {
        for changed_validator in [false, true] {
            let dir = TestDir::new();
            let destination = dir.0.join("file.bin");
            let partial = dir.0.join(".file.bin.nexum.part");
            let body: &[u8] = if changed_validator {
                b"new body"
            } else {
                b"full body"
            };
            std::fs::write(&partial, b"old ").unwrap();
            let body_for_server = body.to_vec();
            let (url, server) = serve_dynamic(2, move |index, request| {
                let request = request.to_ascii_lowercase();
                if index == 0 {
                    assert!(request.contains("range: bytes=4-"), "{request}");
                    if changed_validator {
                        http_response(
                            "206 Partial Content",
                            "ETag: \"new\"\r\nContent-Range: bytes 4-7/8\r\n",
                            b"body",
                        )
                    } else {
                        http_response("200 OK", "ETag: \"new\"\r\n", &body_for_server)
                    }
                } else {
                    assert!(!request.contains("range:"), "{request}");
                    http_response("200 OK", "ETag: \"new\"\r\n", &body_for_server)
                }
            });
            write_resume_metadata(
                &partial,
                &HttpResumeMetadata {
                    source: url.clone(),
                    destination: destination.to_string_lossy().into_owned(),
                    validator: HttpResumeValidator::Etag("\"old\"".into()),
                    expected_length: Some(8),
                },
            )
            .unwrap();
            let result = HttpEngine::new().download_to_resumable_with_control(
                &url,
                destination.to_str().unwrap(),
                &partial,
                &HttpTransferControl::new(),
                |_| Ok(()),
            );
            server.join().unwrap();
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(std::fs::read(&destination).unwrap(), body);
        }
    }

    #[test]
    fn resumable_http_download_restarts_after_malformed_content_range() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let partial = dir.0.join(".file.bin.nexum.part");
        let body = b"fresh body";
        std::fs::write(&partial, b"old ").unwrap();
        let (url, server) = serve_dynamic(2, move |index, request| {
            let request = request.to_ascii_lowercase();
            if index == 0 {
                assert!(request.contains("range: bytes=4-"), "{request}");
                http_response(
                    "206 Partial Content",
                    "ETag: \"old\"\r\nContent-Range: bytes 5-7/8\r\n",
                    b"bad",
                )
            } else {
                assert!(!request.contains("range:"), "{request}");
                http_response("200 OK", "ETag: \"new\"\r\n", body)
            }
        });
        write_resume_metadata(
            &partial,
            &HttpResumeMetadata {
                source: url.clone(),
                destination: destination.to_string_lossy().into_owned(),
                validator: HttpResumeValidator::Etag("\"old\"".into()),
                expected_length: Some(9),
            },
        )
        .unwrap();
        let result = HttpEngine::new().download_to_resumable_with_control(
            &url,
            destination.to_str().unwrap(),
            &partial,
            &HttpTransferControl::new(),
            |_| Ok(()),
        );
        server.join().unwrap();
        assert_eq!(result.unwrap(), Progress::new(10, Some(10)));
        assert_eq!(std::fs::read(&destination).unwrap(), body);
    }

    #[test]
    fn resumable_http_download_preserves_partial_on_transfer_error_and_cleans_on_cancel() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let partial = dir.0.join(".file.bin.nexum.part");
        let (url, server) = serve_dynamic(1, |_, _| {
            http_response_with_declared_length("200 OK", "ETag: \"v1\"\r\n", 10, b"short")
        });
        let result = HttpEngine::new().download_to_resumable_with_control(
            &url,
            destination.to_str().unwrap(),
            &partial,
            &HttpTransferControl::new(),
            |_| Ok(()),
        );
        server.join().unwrap();
        assert!(matches!(result, Err(EngineError::Failed(_))));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"short");
        assert!(resumable_metadata_path(&partial).exists());

        let cancel_dir = TestDir::new();
        let cancel_destination = cancel_dir.0.join("file.bin");
        let cancel_partial = cancel_dir.0.join(".file.bin.nexum.part");
        let (cancel_url, first_sent, release, server) =
            serve_in_chunks(vec![b'a'; 32 * 1024], vec![b'b'; 32 * 1024]);
        let control = HttpTransferControl::new();
        let (progress_tx, progress_rx) = mpsc::channel();
        let (release_progress_tx, release_progress_rx) = mpsc::channel();
        let control_for_worker = control.clone();
        let destination_for_worker = cancel_destination.to_str().unwrap().to_owned();
        let partial_for_worker = cancel_partial.clone();
        let worker = thread::spawn(move || {
            let mut first_progress = Some(progress_tx);
            HttpEngine::new().download_to_resumable_with_control(
                &cancel_url,
                &destination_for_worker,
                &partial_for_worker,
                &control_for_worker,
                |progress| {
                    if let Some(progress_tx) = first_progress.take() {
                        progress_tx.send(progress).unwrap();
                        release_progress_rx.recv().unwrap();
                    }
                    Ok(())
                },
            )
        });
        first_sent.recv().unwrap();
        pause_at_first_progress(&control, progress_rx, release_progress_tx);
        control.cancel();
        assert!(matches!(
            worker.join().unwrap(),
            Err(EngineError::Cancelled)
        ));
        release.send(()).unwrap();
        server.join().unwrap();
        assert!(!cancel_partial.exists());
        assert!(!resumable_metadata_path(&cancel_partial).exists());
    }

    #[test]
    fn http_failure_preserves_existing_destination() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        std::fs::write(&destination, b"old bytes").unwrap();
        let (source, server) = serve_once(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );

        let result = HttpEngine::new().download_to(&source, destination.to_str().unwrap());

        server.join().unwrap();
        assert!(matches!(result, Err(EngineError::Failed(_))));
        assert_eq!(std::fs::read(&destination).unwrap(), b"old bytes");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn adapter_exposes_capabilities_and_task_mapping() {
        let mut engine = FakeEngine;
        assert_eq!(engine.name(), "fake");
        assert!(engine.capabilities().supports_progress);

        let id = TaskId::from("task-1");
        let task = engine
            .start(&id, "https://example.com/file", "/tmp/file")
            .unwrap();
        let mapping = TaskMapping::new(id.clone(), task.clone());
        assert_eq!(mapping.task_id, id);
        assert_eq!(mapping.state, TaskState::Downloading);
        assert_eq!(
            engine.progress(&task).unwrap(),
            Progress::new(42, Some(100))
        );
        assert_eq!(engine.state(&task).unwrap(), EngineTaskState::Downloading);
        let snapshot = EngineSnapshot::new(
            engine.state(&task).unwrap(),
            engine.progress(&task).unwrap(),
        );
        assert_eq!(map_engine_snapshot(&snapshot).0, TaskState::Downloading);
    }

    #[test]
    fn in_memory_engine_controls_task_lifecycle() {
        let mut engine = InMemoryEngine::new();
        let id = TaskId::from("task-1");
        let task = engine
            .start(&id, "https://example.com/file", "/tmp/file")
            .unwrap();
        assert_eq!(engine.state(&task).unwrap(), EngineTaskState::Downloading);
        engine.pause(&task).unwrap();
        assert_eq!(engine.state(&task).unwrap(), EngineTaskState::Paused);
        engine.resume(&task).unwrap();
        assert_eq!(engine.state(&task).unwrap(), EngineTaskState::Downloading);
        engine.remove(&task).unwrap();
        assert!(matches!(
            engine.progress(&task),
            Err(EngineError::TaskNotFound(_))
        ));
    }

    #[test]
    fn registry_selects_engines_by_name() {
        let mut registry = EngineRegistry::new();
        registry.register(Box::new(FakeEngine));
        assert_eq!(registry.names(), vec!["fake"]);
        assert!(registry.get("fake").is_some());
        assert!(registry.get("missing").is_none());
        assert!(
            registry
                .start_engine("fake", &TaskId::new("x"), "https://x", "/x")
                .is_ok()
        );
    }

    #[test]
    fn engine_snapshot_maps_state_and_progress() {
        let snapshot =
            EngineSnapshot::new(EngineTaskState::Completed, Progress::new(100, Some(100)));
        let (state, progress) = map_engine_snapshot(&snapshot);
        assert_eq!(state, TaskState::Completed);
        assert_eq!(progress, Progress::new(100, Some(100)));
    }

    #[test]
    fn unsupported_operations_are_explicit() {
        let mut engine = FakeEngine;
        let id = TaskId::from("task-1");
        let task = engine
            .start(&id, "https://example.com/file", "/tmp/file")
            .unwrap();
        assert_eq!(
            engine.pause(&task),
            Err(EngineError::UnsupportedOperation("pause"))
        );
    }
}
