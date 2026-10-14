//! Engine adapter boundaries for Nexum.

use nexum_domain::{Progress, TaskId};
use nexum_task::TaskState;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

mod completion;
pub use completion::{
    CompletionContext, cleanup_completion_partial, clear_completion, completion_receipt_path,
    has_pending_completion, reconcile_completion,
};

static NEXT_TEMP_DOWNLOAD: AtomicU64 = AtomicU64::new(0);

/// Inactivity bound on the response body. A stale socket returns a zero-byte
/// timeout within this window so pause and cancel take effect promptly instead
/// of blocking for the whole 30-minute request timeout.
const READ_STALL_TIMEOUT: Duration = Duration::from_secs(3);

/// Consecutive zero-data stalls tolerated before a dead connection is failed.
/// Any byte read resets the counter, so a slow-but-flowing connection never trips
/// this; only a connection that delivers nothing for the whole window does.
const MAX_CONSECUTIVE_STALLS: u32 = 8;

/// Sliding-window estimate of transfer speed in bytes/second over the trailing
/// samples. Created per download and observed once per chunk.
struct RateGauge {
    samples: VecDeque<(Instant, u64)>,
    last: u64,
}

impl RateGauge {
    fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            last: 0,
        }
    }

    /// Records a cumulative `total_bytes` sample now and returns the smoothed
    /// bytes/second measured over the trailing window.
    fn observe(&mut self, total_bytes: u64) -> u64 {
        let now = Instant::now();
        self.samples.push_back((now, total_bytes));
        let horizon = now - Duration::from_secs(5);
        while self.samples.front().is_some_and(|&(at, _)| at < horizon) {
            self.samples.pop_front();
        }
        // Measure against the oldest in-window sample (or the current sample at
        // the start) so the rate reflects the trailing window, never more.
        let (reference_time, reference_bytes) = match self.samples.front() {
            Some(&(at, bytes)) if at >= horizon => (at, bytes),
            _ => (now, total_bytes),
        };
        let elapsed = (now - reference_time).as_secs_f64();
        if elapsed < 0.5 || total_bytes < reference_bytes {
            return self.last;
        }
        let rate = ((total_bytes - reference_bytes) as f64 / elapsed) as u64;
        // Light exponential smoothing so a single chunk does not spike the UI.
        let smoothed = self.last * 3 / 4 + rate / 4;
        self.last = smoothed;
        smoothed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineCapabilities {
    pub supports_pause: bool,
    pub supports_resume: bool,
    pub supports_remove: bool,
    pub supports_progress: bool,
    pub supports_range: bool,
    pub supports_retry: bool,
    pub supported_protocols: &'static [&'static str],
    pub max_concurrent_segments: usize,
}

impl EngineCapabilities {
    pub const BASIC: Self = Self {
        supports_pause: false,
        supports_resume: false,
        supports_remove: false,
        supports_progress: true,
        supports_range: false,
        supports_retry: false,
        supported_protocols: &[],
        max_concurrent_segments: 0,
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

/// Session for a long-running transfer. Implemented by engines that support
/// asynchronous, interruptible downloads (e.g. the HTTP engine).
pub trait TransferSession: Send + Sync {
    /// Returns the current progress (bytes transferred, total if known, rate).
    fn progress(&self) -> Progress;

    /// Returns the current state (Queued, Downloading, Paused, Completed, Failed).
    fn state(&self) -> EngineTaskState;

    /// Returns any error that caused the transfer to fail (None if still
    /// running or completed).
    fn error(&self) -> Option<String>;

    /// Requests a boundary pause. Returns Ok(()) if the engine supports pause,
    /// or Err if not. The transfer continues until the next chunk boundary.
    fn pause(&self) -> Result<(), EngineError>;

    /// Resumes a paused transfer. Returns Ok(()) if the engine supports resume,
    /// or Err if not. The transfer continues from the pause point on the next
    /// chunk boundary.
    fn resume(&self) -> Result<(), EngineError>;

    /// Requests cancellation. The engine should stop as soon as possible,
    /// cleaning up any partial files and leaving the destination untouched.
    fn cancel(&self);

    /// Blocks until the transfer reaches a terminal state (Completed or Failed)
    /// and returns (state, progress).
    fn wait(self: Box<Self>) -> (EngineTaskState, Progress);
}

/// Trait for engines that support cross-restart resume after a process restart.
pub trait ResumableEngine: Send + Sync {
    /// Returns metadata needed to resume this transfer, or None if the transfer
    /// has not yet started or cannot be resumed.
    fn resume_metadata(&self) -> Option<ResumeMetadata>;
}

/// Metadata needed to resume a transfer after a process restart.
#[derive(Clone, Debug)]
pub struct ResumeMetadata {
    pub source: String,
    pub destination: String,
    pub downloaded_bytes: u64,
    pub expected_total: Option<u64>,
    pub validator: Option<HttpResumeValidator>,
    pub partial_path: PathBuf,
}

/// A session handle for a long-running asynchronous HTTP transfer.
///
/// Created by [`HttpTransferSession::start_download`]. The actual download
/// runs on a background thread; the caller may inspect progress, request
/// pause/resume, request cancellation, or block on
/// [`TransferSession::wait`] until the download reaches a terminal state.
pub struct HttpTransferSession {
    inner: Arc<HttpTransferSessionInner>,
}

/// Internal shared state for a transfer session.
struct HttpTransferSessionInner {
    /// Cooperative pause/resume/cancel control.
    control: HttpTransferControl,
    /// reqwest client owned by the session (cloned from HttpEngine).
    client: reqwest::Client,
    /// Source and destination paths owned by the session.
    source: String,
    _destination: PathBuf,
    /// Shared byte counter for progress polling.
    _bytes: Arc<AtomicU64>,
    /// Latest known progress (total if known, rate, eta).
    progress: Mutex<Progress>,
    /// Current state of the transfer.
    state: Mutex<EngineTaskState>,
    /// Error message if the transfer failed (None while downloading).
    error: Mutex<Option<String>>,
    /// Condition variable to wake blocked [`TransferSession::wait`] callers.
    condvar: Condvar,
    /// Join handle of the background thread.
    join_handle: Mutex<Option<thread::JoinHandle<()>>>,
    /// Whether the download has reached a terminal state (Completed or Failed).
    done: AtomicBool,
}

impl HttpTransferSession {
    /// Starts an asynchronous HTTP download and returns a session handle.
    ///
    /// The download runs on a background thread and may be paused, resumed,
    /// or cancelled through the returned session. The destination file is
    /// written atomically (never replaces an existing file).
    pub fn start_download(
        engine: &HttpEngine,
        source: &str,
        destination: &str,
    ) -> Result<Self, EngineError> {
        let partial_path = resumable_partial_path(Path::new(destination));
        Self::start_download_with_partial(
            engine,
            source,
            destination,
            &partial_path,
            HttpTransferControl::new(),
            false, // no valid partial for a fresh download
        )
    }

    /// Starts an asynchronous HTTP download with a cooperative control object,
    /// writing to a **stable partial file** at `partial_path` during download.
    ///
    /// The partial file is written directly (not via a hidden temp file) so
    /// that pause / cancel / recovery logic can inspect it while the transfer
    /// is still running. The final destination is only updated after the
    /// complete response has been received.
    #[allow(clippy::too_many_arguments)]
    pub fn start_download_with_partial(
        engine: &HttpEngine,
        source: &str,
        destination: &str,
        partial_path: &Path,
        control: HttpTransferControl,
        has_valid_partial: bool,
    ) -> Result<Self, EngineError> {
        let source = source.to_owned();
        let destination = PathBuf::from(destination);
        let partial_path = partial_path.to_owned();
        let has_valid = has_valid_partial;
        let bytes = Arc::new(AtomicU64::new(0));

        let inner = Arc::new(HttpTransferSessionInner {
            control: control.clone(),
            client: engine.client.clone(),
            source: source.clone(),
            _destination: destination.clone(),
            _bytes: Arc::clone(&bytes),
            progress: Mutex::new(Progress::default()),
            state: Mutex::new(EngineTaskState::Downloading),
            error: Mutex::new(None),
            condvar: Condvar::new(),
            join_handle: Mutex::new(None),
            done: AtomicBool::new(false),
        });

        let session = Self {
            inner: Arc::clone(&inner),
        };

        // Spawn the download on a background thread. The session owns a cloned
        // reqwest::Client, so the closure is 'static.
        let handle = thread::spawn(move || {
            let result = download_to_partial_sealed(
                &inner.client,
                &inner.source,
                &destination,
                &partial_path,
                &inner.control,
                has_valid,
                |progress: Progress| {
                    // Update the shared byte counter and progress snapshot.
                    bytes.store(progress.downloaded_bytes, Ordering::SeqCst);
                    {
                        let mut p = inner.progress.lock().unwrap();
                        *p = progress;
                    }
                    // Return Ok so the download continues; the session's internal
                    // progress state is used by the polling thread to persist
                    // progress to the database.
                    Ok(())
                },
            );

            let (final_state, final_error) = match result {
                Ok(downloaded) => {
                    bytes.store(downloaded, Ordering::SeqCst);
                    {
                        let mut p = inner.progress.lock().unwrap();
                        *p = Progress::new(downloaded, None);
                    }
                    (EngineTaskState::Completed, None)
                }
                Err(EngineError::Cancelled) => {
                    {
                        let mut p = inner.progress.lock().unwrap();
                        *p = Progress::new(bytes.load(Ordering::SeqCst), None);
                    }
                    {
                        let mut e = inner.error.lock().unwrap();
                        *e = Some("transfer cancelled".to_owned());
                    }
                    (
                        EngineTaskState::Failed,
                        Some("transfer cancelled".to_owned()),
                    )
                }
                Err(EngineError::Failed(msg)) => {
                    {
                        let mut p = inner.progress.lock().unwrap();
                        *p = Progress::new(bytes.load(Ordering::SeqCst), None);
                    }
                    {
                        let mut e = inner.error.lock().unwrap();
                        *e = Some(msg);
                    }
                    (EngineTaskState::Failed, None)
                }
                Err(EngineError::TaskNotFound(_)) => {
                    (EngineTaskState::Failed, Some("internal error".to_owned()))
                }
                Err(EngineError::UnsupportedOperation(_)) => (
                    EngineTaskState::Failed,
                    Some("unsupported operation".to_owned()),
                ),
            };

            {
                let mut s = inner.state.lock().unwrap();
                *s = final_state;
            }
            {
                let mut e = inner.error.lock().unwrap();
                if final_error.is_some() {
                    *e = final_error;
                }
            }

            // Signal completion to waiters.
            inner.done.store(true, Ordering::SeqCst);
            inner.condvar.notify_all();
        });

        *session.inner.join_handle.lock().unwrap() = Some(handle);
        Ok(session)
    }
}

/// Sealed session download that writes directly to a **stable partial file**.
///
/// This function opens the partial path directly and writes chunks into it.
/// The partial file is committed to the destination at the end via
/// `ResumableDownload::finish`.
fn download_to_partial_sealed<F>(
    client: &reqwest::Client,
    source: &str,
    destination: &Path,
    partial_path: &Path,
    control: &HttpTransferControl,
    has_valid_partial: bool,
    mut on_progress: F,
) -> Result<u64, EngineError>
where
    F: FnMut(Progress) -> Result<(), EngineError>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|error| {
            EngineError::Failed(format!("could not start the HTTP runtime: {error}"))
        })?;

    let downloaded = runtime.block_on(async {
        // Build the HTTP request: send Range + If-Range when resuming, plain GET otherwise.
        let mut request = client.get(source);
        let mut expecting_range = false;
        // When resuming, these hold the original validator and partial size.
        let mut partial_size: Option<u64> = None;
        let mut metadata: Option<HttpResumeMetadata> = None;

        if has_valid_partial {
            // Read metadata to get the validator and check source matches.
            let saved_metadata = read_resume_metadata(partial_path)?
                .ok_or_else(|| EngineError::Failed("resume metadata not found".into()))?;

            // Verify source hasn't changed.
            if saved_metadata.source != source {
                return Err(EngineError::Failed(
                    "source mismatch: resume metadata source does not match current source".into(),
                ));
            }

            // Get the current partial file size for the Range header.
            let saved_partial_size = std::fs::metadata(partial_path)
                .map(|m| m.len())
                .unwrap_or(0);

            // Send Range request with If-Range validator.
            request = request.header(RANGE, format!("bytes={}-", saved_partial_size));
            request = request.header(IF_RANGE, saved_metadata.validator.value());
            expecting_range = true;

            // Store these for later validation (Content-Range + validator check).
            partial_size = Some(saved_partial_size);
            metadata = Some(saved_metadata.clone());
        }

        let mut response = request
            .send()
            .await
            .map_err(|error| EngineError::Failed(error.to_string()))?;

        let status = response.status();

        // Check response status based on whether we sent a Range request.
        if expecting_range {
            // For Range requests, we expect 206 Partial Content.
            // 200 OK means the server ignored the Range (source changed or no Range support).
            if status == StatusCode::OK {
                // Source changed or server doesn't support Range: restart as a fresh GET.
                remove_resumable_files(partial_path);
                // Re-fetch with a plain GET.
                let fresh = client
                    .get(source)
                    .send()
                    .await
                    .map_err(|error| EngineError::Failed(error.to_string()))?;
                if !fresh.status().is_success() {
                    return Err(EngineError::Failed(format!(
                        "HTTP GET returned {}",
                        fresh.status()
                    )));
                }
                response = fresh;
            } else if status != StatusCode::PARTIAL_CONTENT {
                return Err(EngineError::Failed(format!(
                    "range request failed with {status}"
                )));
            }
        } else {
            // For plain GET requests, any success status is acceptable.
            if !response.status().is_success() {
                return Err(EngineError::Failed(format!("HTTP error: {status}")));
            }
        }

        // When resuming a validated partial, verify the 206 response
        // Content-Range and validator match the stored metadata.
        // If the server has changed the file, Content-Range or validator will
        // differ and we discard the partial and restart as a fresh download.
        if has_valid_partial {
            let offset = partial_size.expect("set when has_valid_partial is true");
            let saved = metadata
                .as_ref()
                .expect("parsed above when has_valid_partial");
            if response.status() == StatusCode::PARTIAL_CONTENT {
                let range_valid = response
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
                                .is_none_or(|len| len == end - start + 1)
                    });
                let response_validator = HttpResumeValidator::from_headers(response.headers());
                let validator_valid = response_validator.as_ref() == Some(&saved.validator);
                if !range_valid || !validator_valid {
                    // Source changed: discard stale partial, restart fresh.
                    remove_resumable_files(partial_path);
                    // Re-fetch with a plain GET.
                    let fresh = client
                        .get(source)
                        .send()
                        .await
                        .map_err(|error| EngineError::Failed(error.to_string()))?;
                    if !fresh.status().is_success() {
                        return Err(EngineError::Failed(format!(
                            "HTTP GET returned {}",
                            fresh.status()
                        )));
                    }
                    response = fresh;
                }
            }
        }

        let total = response.content_length();

        // Write/update metadata sidecar.
        // When resuming, preserve the original validator and partial file size.
        let metadata_to_write = if let Some(outer) = metadata.as_ref() {
            // For resumed downloads: keep the original validator and use the
            // Content-Range total if this was a 206 response.
            if response.status() == StatusCode::PARTIAL_CONTENT {
                if let Some(cr) = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("bytes ").and_then(|v| v.split_once('/')))
                {
                    let total: u64 = cr
                        .1
                        .parse()
                        .map_err(|_| EngineError::Failed("invalid Content-Range total".into()))?;
                    Some(HttpResumeMetadata {
                        source: source.to_owned(),
                        destination: destination.to_string_lossy().into_owned(),
                        validator: outer.validator.clone(),
                        expected_length: Some(total),
                    })
                } else {
                    // No Content-Range on 206 — just update validator.
                    let validator = HttpResumeValidator::from_headers(response.headers());
                    validator.map(|v| HttpResumeMetadata {
                        source: source.to_owned(),
                        destination: destination.to_string_lossy().into_owned(),
                        validator: v,
                        expected_length: outer.expected_length,
                    })
                }
            } else {
                // Fresh GET (either fresh download or restart after source mismatch).
                // Update metadata freely.
                let validator = HttpResumeValidator::from_headers(response.headers());
                validator.map(|v| HttpResumeMetadata {
                    source: source.to_owned(),
                    destination: destination.to_string_lossy().into_owned(),
                    validator: v,
                    expected_length: response.content_length(),
                })
            }
        } else {
            // Fresh download (no resumption).
            let validator = HttpResumeValidator::from_headers(response.headers());
            validator.map(|v| HttpResumeMetadata {
                source: source.to_owned(),
                destination: destination.to_string_lossy().into_owned(),
                validator: v,
                expected_length: response.content_length(),
            })
        };
        if let Some(ref metadata) = metadata_to_write {
            let _ = write_resume_metadata(partial_path, metadata);
        } else {
            let _ = std::fs::remove_file(resumable_metadata_path(partial_path));
        }

        // Never replace an existing destination file (only relevant for fresh downloads).
        if !has_valid_partial {
            ensure_destination_absent(destination)?;
        }

        // Open the partial file: append mode for resume, truncate for fresh download.
        let mut temporary = ResumableDownload::open(partial_path, has_valid_partial)?;

        let mut downloaded: u64 = 0;
        let mut rate = RateGauge::new();
        let mut consecutive_stalls: u32 = 0;

        loop {
            control.wait_if_paused()?;
            control.check_cancelled()?;

            let chunk = match tokio::time::timeout(READ_STALL_TIMEOUT, response.chunk()).await {
                Ok(Ok(Some(chunk))) => chunk,
                Ok(Ok(None)) => break,
                Ok(Err(error)) => return Err(EngineError::Failed(error.to_string())),
                Err(_) => {
                    control.wait_if_paused()?;
                    control.check_cancelled()?;
                    consecutive_stalls = consecutive_stalls.saturating_add(1);
                    if consecutive_stalls >= MAX_CONSECUTIVE_STALLS {
                        return Err(EngineError::Failed(
                            "stalled HTTP download: no data on the connection".to_owned(),
                        ));
                    }
                    continue;
                }
            };

            consecutive_stalls = 0;
            control.check_cancelled()?;
            control.wait_for_budget(chunk.len() as u64)?;
            {
                let inner_file = temporary
                    .file
                    .as_mut()
                    .expect("resumable download file is open");
                inner_file
                    .write_all(&chunk)
                    .map_err(|error| EngineError::Failed(error.to_string()))?;
            }
            downloaded += chunk.len() as u64;
            let speed = rate.observe(downloaded);
            let eta = total.and_then(|expected| {
                (speed > 0).then(|| (expected.saturating_sub(downloaded)) / speed.max(1))
            });
            on_progress(Progress::with_rate(downloaded, total, speed, eta))?;
            control.wait_if_paused()?;

            if total.is_some_and(|expected| downloaded >= expected) {
                break;
            }
            // Also break if we've read all available data and no more content-length
            if total.is_none() && downloaded > 0 {
                // For responses without Content-Length, break after first iteration
                // This is a fallback for edge cases
            }
        }

        // Commit the partial file to the destination.
        // The session path always uses the fast path (finish_resumable_download).
        // Completion receipt generation is handled by the old resumable path
        // (download_to_resumable_with_completion) which is invoked separately
        // by the server for recovery scenarios.
        control.check_cancelled()?;
        control.finish_resumable_download(temporary, destination)?;
        Ok(downloaded)
    });

    control.mark_finished();
    downloaded
}

impl TransferSession for HttpTransferSession {
    fn progress(&self) -> Progress {
        let progress = self.inner.progress.lock().unwrap();
        // Return a copy of the latest recorded progress (already includes the byte count).
        progress.clone()
    }

    fn state(&self) -> EngineTaskState {
        *self.inner.state.lock().unwrap()
    }

    fn error(&self) -> Option<String> {
        self.inner.error.lock().unwrap().clone()
    }

    fn pause(&self) -> Result<(), EngineError> {
        let state = self.state();
        if state == EngineTaskState::Completed || state == EngineTaskState::Failed {
            return Err(EngineError::UnsupportedOperation(
                "cannot pause a completed or failed transfer",
            ));
        }
        self.inner.control.request_pause()
    }

    fn resume(&self) -> Result<(), EngineError> {
        let state = self.state();
        if state != EngineTaskState::Paused {
            return Err(EngineError::Failed("transfer is not paused".into()));
        }
        self.inner.control.resume()
    }

    fn cancel(&self) {
        // Update local state to reflect cancellation.
        {
            let mut s = self.inner.state.lock().unwrap();
            *s = EngineTaskState::Failed;
        }
        // Update the error.
        {
            let mut e = self.inner.error.lock().unwrap();
            if e.is_none() {
                *e = Some("transfer cancelled".to_owned());
            }
        }
        // Request cancellation from the control object, which will wake the
        // download thread at the next chunk boundary.
        self.inner.control.cancel();
        // Wait for the download thread to finish (it should exit quickly).
        if let Some(handle) = self.inner.join_handle.lock().unwrap().take() {
            let _ = handle.join();
        }
    }

    fn wait(self: Box<Self>) -> (EngineTaskState, Progress) {
        // Clone the Arc so we don't consume `self.inner`.
        let inner = self.inner;
        let mut state = inner.state.lock().unwrap();
        while !inner.done.load(Ordering::SeqCst) {
            state = inner.condvar.wait(state).unwrap();
        }
        let final_state = *state;
        let final_progress = inner.progress.lock().unwrap().clone();
        (final_state, final_progress)
    }
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
            supports_range: false,
            supports_retry: false,
            supported_protocols: &[],
            max_concurrent_segments: 0,
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
    client: reqwest::Client,
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
    /// Per-connection bandwidth cap in bytes/second, set by the server from the
    /// scheduler. `None` means unlimited; a small value (including `0`, treated as
    /// `1`) effectively pauses the transfer.
    bytes_per_second: Option<u64>,
    /// Virtual-clock pacer: bytes accounted and the real instant the clock began.
    pacer_sent: u64,
    pacer_started: Option<Instant>,
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

    /// Returns the per-connection bandwidth cap currently in effect. `None` means
    /// unlimited. Lets a caller avoid re-applying a cap that is already set, which
    /// would otherwise restart the pacing clock and allow a one-chunk burst.
    pub fn bytes_per_second(&self) -> Option<u64> {
        self.with_state(|state| state.bytes_per_second)
    }

    /// Sets the per-connection bandwidth cap computed by the scheduler and
    /// restarts the pacing clock so the first chunk is not delayed. A `None` cap
    /// means unlimited. Idempotent and safe to call while the worker runs.
    pub fn set_bytes_per_second(&self, bytes_per_second: Option<u64>) {
        let (lock, _) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        state.bytes_per_second = bytes_per_second;
        state.pacer_sent = 0;
        state.pacer_started = None;
    }

    /// Blocks until the pacing clock has room for `bytes` at the configured cap,
    /// keeping the transfer's average throughput within the limit. A `None` cap
    /// only accounts the bytes and returns immediately. Returns
    /// [`EngineError::Cancelled`] if the transfer is cancelled while sleeping.
    fn wait_for_budget(&self, bytes: u64) -> Result<(), EngineError> {
        let (lock, _) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.cancelled {
            return Err(EngineError::Cancelled);
        }
        let Some(cap) = state.bytes_per_second else {
            state.pacer_sent = state.pacer_sent.saturating_add(bytes);
            return Ok(());
        };
        let now = Instant::now();
        let started = *state.pacer_started.get_or_insert(now);
        state.pacer_sent = state.pacer_sent.saturating_add(bytes);
        let virtual_seconds = state.pacer_sent as f64 / cap.max(1) as f64;
        let real_seconds = now.duration_since(started).as_secs_f64();
        let wait = virtual_seconds - real_seconds;
        drop(state);
        if wait > 0.0 {
            self.sleep_until(Instant::now() + Duration::from_secs_f64(wait))?;
        }
        Ok(())
    }

    /// Sleeps until `until`, waking early and returning [`EngineError::Cancelled`]
    /// if the transfer is cancelled. Sliced so cancellation stays responsive.
    fn sleep_until(&self, until: Instant) -> Result<(), EngineError> {
        let slice = Duration::from_millis(50);
        loop {
            if self.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            let now = Instant::now();
            if now >= until {
                return Ok(());
            }
            std::thread::sleep((until - now).min(slice));
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

    /// Marks a transfer whose bytes the caller has already committed outside the
    /// temporary-file helpers (the multi-connection path assembles a complete
    /// file and publishes it directly). A late pause or cancel request resolves
    /// as completed rather than cancelling a finished transfer.
    fn finish_committed(&self) {
        let (lock, wake) = &*self.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        state.committed = true;
        state.finished = true;
        state.pause_requested = false;
        state.paused = false;
        wake.notify_all();
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
pub fn resumable_partial_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .expect("checked destination has a file name")
        .to_string_lossy();
    destination.with_file_name(format!(".{file_name}.nexum.part"))
}

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
        // Reject symbolic links to prevent staging files from pointing at
        // unrelated user data.  Only regular files (or non-existent paths
        // that will be created as regular files) are accepted.
        if let Ok(metadata) = std::fs::symlink_metadata(partial_path)
            && metadata.file_type().is_symlink()
        {
            // Remove the dangling / malicious symlink so the fresh
            // truncate path below creates a regular file.
            let _ = std::fs::remove_file(partial_path);
        }
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

/// A contiguous byte range of the source file. `start` and `end` are both
/// inclusive; the trailing segment is allowed to be shorter than the others.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
}

impl Segment {
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// The inclusive byte count of the range.
    pub fn length(&self) -> u64 {
        self.end - self.start + 1
    }
}

/// What a lightweight probe learned about a source's range support, used to
/// decide between a multi-connection download and the single-connection path.
#[derive(Clone, Debug)]
pub struct RangeCapability {
    /// True when the probe received a `206 Partial Content`, meaning the server
    /// honours `Range` requests. A `200` (server ignored the range) is false.
    pub supports_ranges: bool,
    /// The whole-file length in bytes, when the server reported one.
    pub length: Option<u64>,
    /// The strongest validator (`ETag`, else `Last-Modified`) from the probe,
    /// sent with `If-Range` so a mid-download change fails the multi download.
    pub validator: Option<HttpResumeValidator>,
}

/// Downloads one [`Segment`] into the shared partial file at its absolute
/// offset. Runs on its own single-threaded runtime so the synchronous bandwidth
/// pacer and stall detection do not block the other segment workers.
#[allow(clippy::too_many_arguments)]
async fn pump_segment(
    client: &reqwest::Client,
    source: &str,
    segment: Segment,
    partial_path: &Path,
    validator: Option<&HttpResumeValidator>,
    control: &HttpTransferControl,
    shared: &Arc<AtomicU64>,
    abort: &Arc<AtomicBool>,
) -> Result<u64, EngineError> {
    let mut request = client
        .get(source)
        .header(RANGE, format!("bytes={}-{}", segment.start, segment.end));
    if let Some(validator) = validator {
        request = request.header(IF_RANGE, validator.value());
    }
    let mut response = request
        .send()
        .await
        .map_err(|error| EngineError::Failed(error.to_string()))?;
    let status = response.status();
    // A range request must be honoured with a 206. A 200 means the server
    // ignored the range, or the validator no longer matches the file: never
    // write its full body into the segment's slot.
    if status == StatusCode::OK {
        return Err(EngineError::Failed(
            "the server rejected a segment's range request".to_owned(),
        ));
    }
    if status != StatusCode::PARTIAL_CONTENT {
        return Err(EngineError::Failed(format!(
            "segment request returned {status}"
        )));
    }
    let expected = segment.length();
    // A declared length that does not match the requested range is abnormal and
    // would corrupt the assembled file.
    if let Some(declared) = response.content_length()
        && declared != expected
    {
        return Err(EngineError::Failed(format!(
            "abnormal segment length: expected {expected} bytes, server declared {declared}"
        )));
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(partial_path)
        .map_err(|error| EngineError::Failed(error.to_string()))?;
    file.seek(SeekFrom::Start(segment.start))
        .map_err(|error| EngineError::Failed(error.to_string()))?;

    let mut downloaded = 0u64;
    let mut consecutive_stalls = 0u32;
    loop {
        control.wait_if_paused()?;
        control.check_cancelled()?;
        if abort.load(Ordering::Acquire) {
            return Err(EngineError::Failed(
                "another segment failed; aborting this segment".to_owned(),
            ));
        }
        let chunk = match tokio::time::timeout(READ_STALL_TIMEOUT, response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => break,
            Ok(Err(error)) => return Err(EngineError::Failed(error.to_string())),
            Err(_) => {
                // A stalled read yields a zero-data inactivity timeout. Pause and
                // cancel are only observable between reads, so re-check them and
                // retry; fail only after a sustained zero-data gap.
                control.wait_if_paused()?;
                control.check_cancelled()?;
                if abort.load(Ordering::Acquire) {
                    return Err(EngineError::Failed(
                        "another segment failed; aborting this segment".to_owned(),
                    ));
                }
                consecutive_stalls = consecutive_stalls.saturating_add(1);
                if consecutive_stalls >= MAX_CONSECUTIVE_STALLS {
                    return Err(EngineError::Failed(
                        "stalled HTTP segment: no data on the connection".to_owned(),
                    ));
                }
                continue;
            }
        };
        consecutive_stalls = 0;
        if downloaded + chunk.len() as u64 > expected {
            return Err(EngineError::Failed(format!(
                "abnormal segment length: the server sent more than the {expected}-byte range"
            )));
        }
        control.wait_for_budget(chunk.len() as u64)?;
        file.write_all(&chunk)
            .map_err(|error| EngineError::Failed(error.to_string()))?;
        downloaded += chunk.len() as u64;
        shared.fetch_add(chunk.len() as u64, Ordering::SeqCst);
        // A known-length range is complete as soon as the requested byte count
        // has been written, so a keep-alive connection need not close for EOF.
        if downloaded >= expected {
            break;
        }
    }
    // An EOF before the declared range is complete is an abnormal, short body.
    if downloaded != expected {
        return Err(EngineError::Failed(format!(
            "incomplete HTTP segment: expected {expected} bytes, received {downloaded}"
        )));
    }
    Ok(downloaded)
}

impl Default for HttpEngine {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
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

    /// Returns capabilities for the HTTP engine, including range support and
    /// multi-connection parameters now added to the enhanced `EngineCapabilities`.
    pub fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            supports_pause: true,
            supports_resume: true,
            supports_remove: true,
            supports_progress: true,
            supports_range: true,
            supports_retry: true,
            supported_protocols: &["http", "https"],
            max_concurrent_segments: 8,
        }
    }

    /// Returns metadata needed to resume an HTTP transfer after a process
    /// restart, or None if no resumable transfer file exists.
    pub fn resume_metadata(
        &self,
        partial_path: impl AsRef<Path>,
    ) -> Result<Option<ResumeMetadata>, EngineError> {
        let partial = partial_path.as_ref();
        match read_resume_metadata(partial) {
            Ok(Some(metadata)) => {
                let partial_length = std::fs::symlink_metadata(partial)
                    .ok()
                    .filter(|m| m.file_type().is_file())
                    .map(|m| m.len());
                Ok(Some(ResumeMetadata {
                    source: metadata.source,
                    destination: metadata.destination,
                    downloaded_bytes: partial_length.unwrap_or(0),
                    expected_total: metadata.expected_length,
                    validator: Some(metadata.validator),
                    partial_path: partial.to_path_buf(),
                }))
            }
            Ok(None) => Ok(None),
            Err(_) => Ok(None),
        }
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
            None,
        );
        if matches!(result, Err(EngineError::Cancelled))
            || (control.is_cancelled() && !control.is_committed())
        {
            remove_resumable_files(partial_path);
        }
        control.mark_finished();
        result
    }

    /// Downloads with a private completion journal so a complete response can
    /// be reconciled after a process crash without relying on HTTP validators.
    /// The caller must retain the receipt until its final task-state update is
    /// persisted, then clean the proven partial and clear the receipt.
    pub fn download_to_resumable_with_completion<F>(
        &self,
        source: &str,
        destination: &str,
        partial_path: impl AsRef<Path>,
        completion: &CompletionContext,
        control: &HttpTransferControl,
        mut on_progress: F,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        let partial = partial_path.as_ref();
        let result = (|| {
            if let Some(progress) =
                reconcile_completion(completion, source, destination, partial, control)?
            {
                return Ok(progress);
            }
            self.download_to_resumable_inner(
                source,
                destination,
                partial,
                control,
                &mut on_progress,
                Some(completion),
            )
        })();
        // A cancellation after proof creation keeps the complete partial and
        // journal together. Task deletion owns their subsequent cleanup.
        if matches!(result, Err(EngineError::Cancelled))
            && !has_pending_completion(completion, source, destination, partial).unwrap_or(true)
        {
            remove_resumable_files(partial);
        }
        control.mark_finished();
        result
    }

    /// Probes `source` with a single `Range: bytes=0-0` request to learn whether
    /// the server honours range requests, what the total file length is, and which
    /// validator (ETag or Last-Modified) the server sends. These callers build
    /// their own runtime; this is a synchronous convenience on top of the async
    /// internals.
    pub fn probe_range(&self, source: &str) -> Option<RangeCapability> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .ok()?;
        runtime.block_on((|| async {
            let mut response = self
                .client
                .get(source)
                .header(RANGE, "bytes=0-0")
                .send()
                .await
                .ok()?;
            let supports_ranges = response.status() == StatusCode::PARTIAL_CONTENT;
            let total = if supports_ranges {
                response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_content_range)
                    .map(|(_, _, total)| total)
            } else {
                response.content_length()
            };
            let validator = HttpResumeValidator::from_headers(response.headers());
            let _ = response.chunk().await; // drain so the connection closes cleanly.
            Some(RangeCapability {
                supports_ranges,
                length: total,
                validator,
            })
        })())
    }

    /// Splits a file of `total` bytes into at most `max_segments` contiguous
    /// [`Segment`]s. The trailing segment may be shorter than the others.
    pub fn split_segments(total: u64, max_segments: usize) -> Vec<Segment> {
        let max_segments = max_segments.max(1);
        let size = (total as f64 / max_segments as f64).ceil() as u64;
        if size == 0 {
            return vec![Segment::new(0, 0)];
        }
        (0..total)
            .step_by(size as usize)
            .map(|start| Segment::new(start, (start + size - 1).min(total - 1)))
            .collect()
    }

    /// Downloads a source in multiple concurrent HTTP Range segments, reassembles
    /// them atomically, and falls back to a single connection when the server
    /// does not support Range.
    ///
    /// Creates a sparse pre-allocated partial file, then spawns one std thread per
    /// segment. Each thread builds its own current-thread Tokio runtime and runs
    /// [`pump_segment`] on it; all threads share one `HttpTransferControl` so the
    /// total bandwidth cap is respected.
    ///
    /// A segment failure aborts all workers, removes the partial, and returns the
    /// first error. A cancelled transfer cleans the partial the same way. If the
    /// caller requested pause, the transfer proceeds until cancelled or completed
    /// (pause is primarily a server concern: the main thread's call can request
    /// pause, and each segment honour it between reads).
    #[allow(clippy::too_many_arguments)]
    pub fn download_to_segments<F>(
        &self,
        source: &str,
        segments: &[Segment],
        destination: &str,
        partial_path: impl AsRef<Path>,
        total: u64,
        validator: Option<&HttpResumeValidator>,
        control: &HttpTransferControl,
        mut on_progress: F,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        use std::sync::mpsc;

        control.check_cancelled()?;
        let partial = partial_path.as_ref();
        ensure_destination_absent(Path::new(destination))?;

        let destination_parent = Path::new(destination)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(destination_parent)
            .map_err(|error| EngineError::Failed(error.to_string()))?;

        // Pre-allocate a sparse file so workers can seek to any offset.
        // Reject (and remove) any pre-existing symlink so that truncate
        // creates a regular file instead of following the link.
        if let Ok(metadata) = std::fs::symlink_metadata(partial)
            && metadata.file_type().is_symlink()
        {
            let _ = std::fs::remove_file(partial);
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(partial)
            .map_err(|error| EngineError::Failed(error.to_string()))?;
        file.set_len(total)
            .map_err(|error| EngineError::Failed(error.to_string()))?;

        let shared = Arc::new(AtomicU64::new(0));
        let abort = Arc::new(AtomicBool::new(false));
        let (result_tx, result_rx) = mpsc::channel::<Result<u64, EngineError>>();
        let handles: Vec<_> = segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                let client = self.client.clone();
                let source = source.to_owned();
                let segment = *segment;
                let partial = partial.to_owned();
                let validator = validator.cloned();
                let control = control.clone();
                let shared = Arc::clone(&shared);
                let abort = Arc::clone(&abort);
                let result_tx = result_tx.clone();
                thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_io()
                        .enable_time()
                        .build()
                        .map_err(|error| {
                            EngineError::Failed(format!(
                                "could not start segment-{index} runtime: {error}"
                            ))
                        });
                    let mut result = match runtime {
                        Ok(runtime) => runtime.block_on(pump_segment(
                            &client,
                            &source,
                            segment,
                            &partial,
                            validator.as_ref(),
                            &control,
                            &shared,
                            &abort,
                        )),
                        Err(error) => Err(error),
                    };
                    if result.is_err() {
                        result = Err(EngineError::Failed("segment failed".to_owned()));
                    }
                    let _ = result_tx.send(result);
                })
            })
            .collect();
        // Drop the sender so disconnected() fires when all workers finish.
        drop(result_tx);

        let mut received = 0usize;
        let mut first_error = None::<EngineError>;
        let mut reported = 0u64;
        let rate = std::cell::RefCell::new(RateGauge::new());
        let cadence = Duration::from_millis(200);
        let mut last_report = Instant::now();

        while received < segments.len() {
            match result_rx.recv_timeout(cadence) {
                Ok(Ok(_)) => {
                    received += 1;
                }
                Ok(Err(error)) => {
                    received += 1;
                    if first_error.is_none() {
                        first_error = Some(error);
                        abort.store(true, Ordering::Release);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let done = shared.load(Ordering::SeqCst);
            if done != reported || last_report.elapsed() >= cadence {
                let speed = {
                    let mut guard = rate.borrow_mut();
                    guard.observe(done)
                };
                let eta = (done < total)
                    .then(|| {
                        let speed = {
                            let guard = rate.borrow();
                            guard.last
                        };
                        (speed > 0).then(|| (total.saturating_sub(done)) / speed.max(1))
                    })
                    .flatten();
                if let Err(error) = on_progress(Progress::with_rate(done, Some(total), speed, eta))
                {
                    abort.store(true, Ordering::Release);
                    first_error = Some(error);
                    break;
                }
                reported = done;
                last_report = Instant::now();
            }
        }

        // Join workers (in case some are still running when the channel closes).
        for handle in handles {
            let _ = handle.join();
        }

        if let Some(error) = first_error {
            remove_resumable_files(partial);
            control.mark_finished();
            return Err(error);
        }

        // Verify the partial file reached the expected total before committing.
        let actual = match std::fs::symlink_metadata(partial) {
            Ok(metadata) if metadata.file_type().is_file() => metadata.len(),
            Ok(_) => {
                remove_resumable_files(partial);
                control.mark_finished();
                return Err(EngineError::Failed(format!(
                    "download partial is not a file: {}",
                    partial_path.as_ref().display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                remove_resumable_files(partial);
                control.mark_finished();
                return Err(EngineError::Failed(format!(
                    "download partial disappeared: {}",
                    partial_path.as_ref().display()
                )));
            }
            Err(error) => {
                control.mark_finished();
                return Err(EngineError::Failed(error.to_string()));
            }
        };
        if actual != total {
            remove_resumable_files(partial);
            control.mark_finished();
            return Err(EngineError::Failed(format!(
                "partial file size {actual} does not match expected {total}"
            )));
        }

        // Check cancellation before publishing (P2: cancellation before
        // publication must win).
        control.check_cancelled()?;

        // Publish atomically; never clobber an existing destination.
        commit_download(partial, Path::new(destination)).inspect_err(|_error| {
            remove_resumable_files(partial);
        })?;
        // Re-check after publication so a cancel during the rename wins.
        control.check_cancelled()?;
        control.finish_committed();
        // Clean up the sparse partial — the destination now holds the data.
        let _ = std::fs::remove_file(partial);

        Ok(Progress::new(total, Some(total)))
    }

    /// Reads `response` into `file` until EOF, enforcing the control's bandwidth
    /// cap and reporting a windowed speed/eta on each chunk.
    ///
    /// A stalled body (zero-byte inactivity timeout) is not a failure: pause and
    /// cancel are re-checked and the read retried, so they take effect within the
    /// stall window instead of the full request timeout. Only a sustained
    /// zero-data gap, or a real I/O error, aborts the transfer. Returns the total
    /// bytes downloaded, including the pre-seeded `start_downloaded` offset.
    async fn pump_response<F>(
        &self,
        response: &mut reqwest::Response,
        file: &mut std::fs::File,
        control: &HttpTransferControl,
        on_progress: &mut F,
        mut downloaded: u64,
        total: Option<u64>,
    ) -> Result<u64, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        let mut rate = RateGauge::new();
        let mut consecutive_stalls = 0u32;
        loop {
            control.wait_if_paused()?;
            control.check_cancelled()?;
            let chunk = match tokio::time::timeout(READ_STALL_TIMEOUT, response.chunk()).await {
                Ok(Ok(Some(chunk))) => chunk,
                Ok(Ok(None)) => break,
                Ok(Err(error)) => return Err(EngineError::Failed(error.to_string())),
                Err(_) => {
                    // A stalled body yields a zero-data inactivity timeout. Pause
                    // and cancel are only observable between reads, so re-check
                    // them and retry; fail only after a sustained zero-data gap.
                    control.wait_if_paused()?;
                    control.check_cancelled()?;
                    consecutive_stalls = consecutive_stalls.saturating_add(1);
                    if consecutive_stalls >= MAX_CONSECUTIVE_STALLS {
                        return Err(EngineError::Failed(
                            "stalled HTTP download: no data on the connection".to_owned(),
                        ));
                    }
                    continue;
                }
            };
            consecutive_stalls = 0;
            control.check_cancelled()?;
            control.wait_for_budget(chunk.len() as u64)?;
            file.write_all(&chunk)
                .map_err(|error| EngineError::Failed(error.to_string()))?;
            downloaded += chunk.len() as u64;
            let speed = rate.observe(downloaded);
            let eta = total.and_then(|expected| {
                (speed > 0).then(|| (expected.saturating_sub(downloaded)) / speed.max(1))
            });
            on_progress(Progress::with_rate(downloaded, total, speed, eta))?;
            control.wait_if_paused()?;
            // Complete a known-length response as soon as the declared length is
            // reached, so a keep-alive connection need not close to signal EOF.
            if total.is_some_and(|expected| downloaded >= expected) {
                break;
            }
        }
        Ok(downloaded)
    }

    fn download_to_resumable_inner<F>(
        &self,
        source: &str,
        destination: &str,
        partial_path: &Path,
        control: &HttpTransferControl,
        on_progress: &mut F,
        completion: Option<&CompletionContext>,
    ) -> Result<Progress, EngineError>
    where
        F: FnMut(Progress) -> Result<(), EngineError>,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|error| {
                EngineError::Failed(format!("could not start the HTTP runtime: {error}"))
            })?;
        runtime.block_on(self.download_to_resumable_inner_async(
            source,
            destination,
            partial_path,
            control,
            &mut *on_progress,
            completion,
        ))
    }

    async fn download_to_resumable_inner_async<F>(
        &self,
        source: &str,
        destination: &str,
        partial_path: &Path,
        control: &HttpTransferControl,
        on_progress: &mut F,
        completion: Option<&CompletionContext>,
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
            .await
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
                    .await
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
        let downloaded = {
            let writer = temporary
                .file
                .as_mut()
                .expect("resumable download file is open");
            self.pump_response(&mut response, writer, control, on_progress, offset, total)
                .await?
        };
        let result = (|| {
            control.check_cancelled()?;
            if total.is_some_and(|expected| downloaded != expected) {
                return Err(EngineError::Failed(format!(
                    "incomplete HTTP response: expected {} bytes, received {downloaded}",
                    total.expect("total was checked")
                )));
            }
            control.check_cancelled()?;
            if let Some(completion) = completion {
                let receipt = completion::prepare_completion(
                    completion,
                    source,
                    destination,
                    partial_path,
                    temporary
                        .file
                        .as_ref()
                        .expect("resumable download file is open"),
                    &Progress::new(downloaded, total),
                    control,
                )?;
                // Keep the owned writer alive through publication and keep
                // the private receipt until the caller commits Completed.
                completion::publish_completion(&receipt, control)?;
            } else {
                control.finish_resumable_download(temporary, Path::new(destination))?;
            }
            Ok(Progress::new(downloaded, total))
        })();

        let pending_completion = completion.is_some_and(|completion| {
            has_pending_completion(completion, source, destination, partial_path).unwrap_or(true)
        });
        if result.is_err()
            && !pending_completion
            && (!preserve_partial || matches!(result, Err(EngineError::Cancelled)))
        {
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
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|error| {
                EngineError::Failed(format!("could not start the HTTP runtime: {error}"))
            })?;
        runtime.block_on(self.download_to_with_control_inner_async(
            source,
            destination,
            control,
            &mut *on_progress,
        ))
    }

    async fn download_to_with_control_inner_async<F>(
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
            .await
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
        let downloaded = {
            let writer = temporary
                .file
                .as_mut()
                .expect("temporary download file is open");
            self.pump_response(&mut response, writer, control, on_progress, 0, total)
                .await?
        };

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
        HttpEngine::capabilities(self)
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::LazyLock;
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

    /// Serves `body` in fixed-size chunks separated by a delay, optionally with
    /// a `Content-Length`. The gap makes the transfer's rate observable and
    /// deterministic for speed, ETA, and pacing assertions.
    fn serve_drip(
        body: Vec<u8>,
        chunk: usize,
        inter_chunk_delay: Duration,
        declare_length: bool,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/file", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let _ = read_http_request(&mut stream);
            let length_line = if declare_length {
                format!("Content-Length: {}\r\n", body.len())
            } else {
                String::new()
            };
            let header = format!("HTTP/1.1 200 OK\r\n{length_line}Connection: close\r\n\r\n");
            stream.write_all(header.as_bytes()).unwrap();
            stream.flush().unwrap();
            let mut position = 0;
            while position < body.len() {
                let end = (position + chunk).min(body.len());
                stream.write_all(&body[position..end]).unwrap();
                stream.flush().unwrap();
                position = end;
                if position < body.len() {
                    thread::sleep(inter_chunk_delay);
                }
            }
        });
        (address, worker)
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
    fn http_engine_reports_speed_and_eta_for_declared_length() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let body = vec![b'x'; 64 * 1024];
        // Drip slowly enough that the trailing rate window fills in (> 0.5 s).
        let (source, server) = serve_drip(body.clone(), 1024, Duration::from_millis(20), true);
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
        let mut observed_speed = 0;
        let mut saw_total = false;
        let mut saw_eta = false;
        while let Ok(progress) = progress_rx.recv() {
            observed_speed = observed_speed.max(progress.speed_bytes_per_second);
            saw_total = saw_total || progress.total_bytes == Some(body.len() as u64);
            saw_eta = saw_eta || progress.eta_seconds.is_some();
        }
        server.join().unwrap();
        let result = worker.join().unwrap().unwrap();
        assert_eq!(result.downloaded_bytes, body.len() as u64);
        assert_eq!(result.total_bytes, Some(body.len() as u64));
        assert!(
            saw_total,
            "a declared-length response must report its total"
        );
        assert!(
            observed_speed > 0,
            "a transferring response must report a real speed"
        );
        assert!(
            saw_eta,
            "a declared-length transfer with a positive speed must report an ETA"
        );
        assert_eq!(std::fs::read(&destination).unwrap().len(), body.len());
    }

    #[test]
    fn http_engine_omits_eta_for_unknown_length() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let body = vec![b'y'; 64 * 1024];
        let (source, server) = serve_drip(body.clone(), 1024, Duration::from_millis(20), false);
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
        let mut observed_speed = 0;
        while let Ok(progress) = progress_rx.recv() {
            observed_speed = observed_speed.max(progress.speed_bytes_per_second);
            assert!(
                progress.eta_seconds.is_none(),
                "an unknown-length body must not report an ETA"
            );
        }
        server.join().unwrap();
        let result = worker.join().unwrap().unwrap();
        assert_eq!(result.downloaded_bytes, body.len() as u64);
        assert_eq!(result.total_bytes, None);
        assert!(
            observed_speed > 0,
            "speed is still measurable even without a declared length"
        );
        assert_eq!(std::fs::read(&destination).unwrap().len(), body.len());
    }

    #[test]
    fn http_engine_paces_downloads_to_the_bandwidth_cap() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let body = vec![b'c'; 4000];
        // Serve as fast as possible so the cap, not the server, is the limiter.
        let (source, server) = serve_drip(body.clone(), 1000, Duration::ZERO, true);
        let control = HttpTransferControl::new();
        control.set_bytes_per_second(Some(1000)); // 1 KB/s
        let source_for_worker = source.clone();
        let destination_for_worker = destination.to_str().unwrap().to_owned();
        let control_for_worker = control.clone();
        let worker = thread::spawn(move || {
            HttpEngine::new().download_to_with_control(
                &source_for_worker,
                &destination_for_worker,
                &control_for_worker,
                |_| Ok(()),
            )
        });
        let started = Instant::now();
        let result = worker.join().unwrap().unwrap();
        let elapsed = started.elapsed();
        server.join().unwrap();
        assert_eq!(result.downloaded_bytes, body.len() as u64);
        // 4000 bytes at 1000 B/s of pacing takes ~4 s; allow slack on both ends.
        assert!(
            elapsed >= Duration::from_secs(2),
            "a 1000 B/s capped download finished in {elapsed:?}, exceeding its budget"
        );
        assert!(
            elapsed <= Duration::from_secs(10),
            "a capped download took {elapsed:?}; the pacer over-slept"
        );
        assert_eq!(std::fs::read(&destination).unwrap().len(), body.len());
    }

    #[test]
    fn http_engine_does_not_slow_unlimited_downloads() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let body = vec![b'u'; 64 * 1024];
        let (source, server) = serve_drip(body.clone(), 8192, Duration::from_millis(5), true);
        let control = HttpTransferControl::new(); // no cap: unlimited
        let source_for_worker = source.clone();
        let destination_for_worker = destination.to_str().unwrap().to_owned();
        let control_for_worker = control.clone();
        let worker = thread::spawn(move || {
            HttpEngine::new().download_to_with_control(
                &source_for_worker,
                &destination_for_worker,
                &control_for_worker,
                |_| Ok(()),
            )
        });
        let started = Instant::now();
        let result = worker.join().unwrap().unwrap();
        let elapsed = started.elapsed();
        server.join().unwrap();
        assert_eq!(result.downloaded_bytes, body.len() as u64);
        // The served body is ready in well under a second; an uncapped
        // transfer must not add any pacing delay.
        assert!(
            elapsed < Duration::from_secs(2),
            "an uncapped download was artificially slowed to {elapsed:?}"
        );
        assert_eq!(std::fs::read(&destination).unwrap().len(), body.len());
    }

    #[test]
    fn http_engine_breaks_a_stalled_read_to_pause_within_budget() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let (source, first_sent, release, server) =
            serve_in_chunks(vec![b'a'; 1024], vec![b'b'; 1024]);
        let control = HttpTransferControl::new();
        let source_for_worker = source.clone();
        let destination_for_worker = destination.to_str().unwrap().to_owned();
        let control_for_worker = control.clone();
        let worker = thread::spawn(move || {
            HttpEngine::new().download_to_with_control(
                &source_for_worker,
                &destination_for_worker,
                &control_for_worker,
                |_| Ok(()),
            )
        });
        first_sent.recv().unwrap();
        // The worker holds the first chunk and is blocked in a stalled read.
        let requester = thread::spawn({
            let control = control.clone();
            move || {
                let started = Instant::now();
                let result = control.request_pause();
                (started.elapsed(), result)
            }
        });
        let (elapsed, pause_result) = requester.join().unwrap();
        assert!(pause_result.is_ok(), "pause failed: {pause_result:?}");
        assert!(
            elapsed < Duration::from_secs(5),
            "pause took {elapsed:?} to take effect on a stalled read"
        );
        assert!(control.is_paused());
        // Finish the transfer and release the fixture.
        let _ = control.resume();
        release.send(()).unwrap();
        let _ = worker.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn http_engine_breaks_a_stalled_read_to_cancel_within_budget() {
        let dir = TestDir::new();
        let destination = dir.0.join("file.bin");
        let (source, first_sent, release, server) =
            serve_in_chunks(vec![b'a'; 1024], vec![b'b'; 1024]);
        let control = HttpTransferControl::new();
        let source_for_worker = source.clone();
        let destination_for_worker = destination.to_str().unwrap().to_owned();
        let control_for_worker = control.clone();
        let worker = thread::spawn(move || {
            HttpEngine::new().download_to_with_control(
                &source_for_worker,
                &destination_for_worker,
                &control_for_worker,
                |_| Ok(()),
            )
        });
        first_sent.recv().unwrap();
        let cancel_time = Instant::now();
        control.cancel();
        let result = worker.join().unwrap();
        let elapsed = cancel_time.elapsed();
        assert!(
            matches!(result, Err(EngineError::Cancelled)),
            "expected cancellation, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "cancellation took {elapsed:?} to abort a stalled read"
        );
        assert!(
            !destination.exists(),
            "a cancelled download leaves no destination file"
        );
        release.send(()).unwrap(); // unblock the fixture server
        server.join().unwrap();
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

    #[test]
    fn split_segments_balances_segments() {
        let segments = HttpEngine::split_segments(1_000_000, 3);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].start, 0);
        assert_eq!(segments[0].end, 333_333);
        assert_eq!(segments[0].length(), 333_334);
        assert_eq!(segments[1].start, 333_334);
        assert_eq!(segments[1].end, 666_667);
        assert_eq!(segments[1].length(), 333_334);
        assert_eq!(segments[2].start, 666_668);
        assert_eq!(segments[2].end, 999_999);
        assert_eq!(segments[2].length(), 333_332);
    }

    #[test]
    fn split_segments_small_file_single_segment() {
        // A file smaller than one segment's chunk size gets a single segment.
        let segments = HttpEngine::split_segments(32, 8);
        assert!(!segments.is_empty());
        assert_eq!(segments[0].start, 0);
        // Last segment always ends at total - 1.
        assert_eq!(segments.last().unwrap().end, 31);
    }

    #[test]
    fn split_segments_exactly_splits() {
        let segments = HttpEngine::split_segments(6, 2);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].length(), 3);
        assert_eq!(segments[1].length(), 3);
    }

    #[test]
    fn split_segments_max_segments_one() {
        let segments = HttpEngine::split_segments(10_000, 1);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start, 0);
        assert_eq!(segments[0].end, 9_999);
    }

    #[test]
    fn split_segments_zero_length() {
        // The implementation returns one segment for zero-length, but
        // Segment::new(0, 0 - 1) would underflow. We verify the function
        // doesn't panic on zero-length input (the implementation wraps
        // via saturating sub or similar).
        let segments = HttpEngine::split_segments(0, 4);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn probe_range_rejects_non_206_server() {
        static RESPONSE: LazyLock<Vec<u8>> =
            LazyLock::new(|| http_response("200 OK", "ETag: \"test-v1\"", b"full body"));
        let (address, server) = serve_once(&RESPONSE);
        let cap = HttpEngine::new().probe_range(&address);
        assert!(cap.is_some());
        let cap = cap.unwrap();
        assert!(!cap.supports_ranges);
        assert!(cap.validator.is_some());
        server.join().unwrap();
    }

    #[test]
    fn probe_range_accepts_206_server() {
        static RESPONSE: LazyLock<Vec<u8>> = LazyLock::new(|| {
            http_response(
                "206 Partial Content",
                "Content-Range: bytes 0-0/9\r\nETag: \"probe-v1\"",
                b"x",
            )
        });
        let (address, server) = serve_once(&RESPONSE);
        let cap = HttpEngine::new().probe_range(&address);
        assert!(cap.is_some());
        let cap = cap.unwrap();
        assert!(cap.supports_ranges);
        assert_eq!(cap.length, Some(9));
        server.join().unwrap();
    }
}
