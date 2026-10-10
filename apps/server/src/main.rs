//! Nexum TCP server with configuration support.

mod managed;

use nexum_core::{
    Core,
    nexum_domain::TaskId,
    nexum_engine::{
        EngineError, HttpEngine, HttpTransferControl, remove_resumable_files,
        resumable_partial_progress,
    },
    nexum_resolver::ResolveKind,
    nexum_scheduler::SchedulerConfig,
    nexum_storage::SqliteRepository,
    nexum_task::TaskState,
};
use nexum_protocol::{
    Credential, EventBuffer, EventEnvelope, EventNotification, RateLimit, RpcDispatcher,
    RpcErrorObject, RpcRequest, RpcResponse, TlsConfig, parse_request, serialize_response,
};
use rustls::{ServerConnection, StreamOwned};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

enum Transport {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ServerConnection, TcpStream>>),
}

#[derive(Clone)]
struct SharedStream(Arc<Mutex<Transport>>);

impl SharedStream {
    fn plain(stream: TcpStream) -> Self {
        Self(Arc::new(Mutex::new(Transport::Plain(stream))))
    }

    fn tls(stream: StreamOwned<ServerConnection, TcpStream>) -> Self {
        Self(Arc::new(Mutex::new(Transport::Tls(Box::new(stream)))))
    }

    fn complete_handshake(&self) -> io::Result<()> {
        let mut locked = self
            .0
            .lock()
            .map_err(|_| io::Error::other("stream mutex poisoned"))?;
        if let Transport::Tls(stream) = &mut *locked {
            stream.conn.complete_io(&mut stream.sock)?;
        }
        Ok(())
    }
}

impl Read for SharedStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut locked = self
            .0
            .lock()
            .map_err(|_| io::Error::other("stream mutex poisoned"))?;
        match &mut *locked {
            Transport::Plain(stream) => stream.read(buffer),
            Transport::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for SharedStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut locked = self
            .0
            .lock()
            .map_err(|_| io::Error::other("stream mutex poisoned"))?;
        match &mut *locked {
            Transport::Plain(stream) => stream.write(buffer),
            Transport::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut locked = self
            .0
            .lock()
            .map_err(|_| io::Error::other("stream mutex poisoned"))?;
        match &mut *locked {
            Transport::Plain(stream) => stream.flush(),
            Transport::Tls(stream) => stream.flush(),
        }
    }
}

type ServerCore = Core<SqliteRepository>;
const DATABASE_FILE: &str = "nexum.sqlite";
const LOCK_FILE: &str = "nexum.lock";

struct ServerState {
    core: ServerCore,
    // Workers retain the lock along with the database until process termination.
    _data_dir_lock: File,
    data_dir: PathBuf,
    active_http: HashMap<TaskId, HttpTransfer>,
    active_destinations: HashSet<PathBuf>,
    events: EventHub,
    authentication: AuthenticationConfig,
    rate_limiter: Option<RateLimiter>,
}

#[derive(Default)]
struct EventHub {
    next_sequence: u64,
    next_subscriber: u64,
    subscribers: HashMap<u64, SyncSender<EventNotification>>,
}

#[derive(Clone)]
struct AuthenticationConfig {
    required: bool,
    credential: Option<Credential>,
}

struct RateLimiter {
    config: RateLimit,
    tokens: f64,
    updated_at: Instant,
}

impl RateLimiter {
    fn new(config: RateLimit, now: Instant) -> Self {
        Self {
            tokens: f64::from(config.burst_size),
            config,
            updated_at: now,
        }
    }

    fn try_acquire(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.updated_at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.config.requests_per_second)
            .min(f64::from(self.config.burst_size));
        self.updated_at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

impl AuthenticationConfig {
    fn from_server_config(config: &ServerConfig) -> Result<Self, String> {
        let credential = config.configured_credential()?;
        if config.require_auth && credential.is_none() {
            return Err(
                "require_auth=true requires auth_scheme and a non-empty auth_token".to_owned(),
            );
        }
        Ok(Self {
            required: config.require_auth,
            credential,
        })
    }

    fn authorize(&self, request: &RpcRequest) -> Result<(), RpcErrorObject> {
        if !self.required {
            return Ok(());
        }
        if self.credential.as_ref().is_some_and(|expected| {
            request
                .credential
                .as_ref()
                .is_some_and(|actual| actual.matches(expected))
        }) {
            Ok(())
        } else {
            Err(RpcErrorObject::unauthorized("authentication required"))
        }
    }

    fn schemes(&self) -> Vec<String> {
        match self.credential.as_ref() {
            Some(Credential::Bearer { .. }) => vec!["Bearer".to_owned()],
            Some(Credential::ApiKey { .. }) => vec!["ApiKey".to_owned()],
            Some(Credential::None) | None => vec!["none".to_owned()],
        }
    }
}

#[derive(Clone)]
struct ConnectionLimiter {
    limit: usize,
    active: Arc<AtomicUsize>,
}

struct ConnectionPermit {
    active: Arc<AtomicUsize>,
}

impl ConnectionLimiter {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            active: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn try_acquire(&self) -> Option<ConnectionPermit> {
        loop {
            let active = self.active.load(Ordering::Acquire);
            if active >= self.limit {
                return None;
            }
            if self
                .active
                .compare_exchange_weak(active, active + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(ConnectionPermit {
                    active: Arc::clone(&self.active),
                });
            }
        }
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl EventHub {
    fn subscribe(&mut self) -> (u64, Receiver<EventNotification>) {
        let (sender, receiver) = mpsc::sync_channel(256);
        let id = self.next_subscriber;
        self.next_subscriber = self.next_subscriber.saturating_add(1);
        self.subscribers.insert(id, sender);
        (id, receiver)
    }

    fn unsubscribe(&mut self, id: u64) {
        self.subscribers.remove(&id);
    }

    fn publish(&mut self, events: Vec<EventEnvelope>) {
        let mut closed = Vec::new();
        for event in events {
            self.next_sequence = self.next_sequence.saturating_add(1);
            let notification = EventNotification::new(self.next_sequence, event);
            for (&id, sender) in &self.subscribers {
                match sender.try_send(notification.clone()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                        closed.push(id)
                    }
                }
            }
        }
        closed.sort_unstable();
        closed.dedup();
        for id in closed {
            self.subscribers.remove(&id);
        }
    }

    fn current_sequence(&self) -> u64 {
        self.next_sequence
    }

    #[cfg(test)]
    fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }
}

fn collect_core_events(state: &mut ServerState) {
    let mut buffer = EventBuffer::new();
    buffer.collect_core(&mut state.core);
    state.events.publish(buffer.drain());
}

#[derive(Clone)]
struct HttpTransfer {
    id: TaskId,
    source: String,
    destination: String,
    destination_path: PathBuf,
    partial_path: PathBuf,
    control: HttpTransferControl,
    completion: Arc<HttpWorkerCompletion>,
    latest_progress: Arc<Mutex<nexum_core::nexum_domain::Progress>>,
    remove_requested: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HttpWorkerOutcome {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Default)]
struct HttpWorkerCompletion {
    outcome: Mutex<Option<HttpWorkerOutcome>>,
    wake: Condvar,
}

impl HttpWorkerCompletion {
    fn finish(&self, outcome: HttpWorkerOutcome) {
        let mut stored = self
            .outcome
            .lock()
            .expect("HTTP worker completion mutex poisoned");
        *stored = Some(outcome);
        self.wake.notify_all();
    }

    fn wait(&self, timeout: Duration) -> Option<HttpWorkerOutcome> {
        let mut stored = self
            .outcome
            .lock()
            .expect("HTTP worker completion mutex poisoned");
        if stored.is_none() {
            let (guard, result) = self
                .wake
                .wait_timeout(stored, timeout)
                .expect("HTTP worker completion mutex poisoned while waiting");
            stored = guard;
            if result.timed_out() && stored.is_none() {
                return None;
            }
        }
        *stored
    }
}

const PROGRESS_MIN_BYTES: u64 = 1024 * 1024;
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(250);
const HTTP_CONTROL_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

fn resumable_partial_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .expect("checked destination has a file name")
        .to_string_lossy();
    destination.with_file_name(format!(".{file_name}.nexum.part"))
}

#[derive(Default)]
struct ProgressReporter {
    last_persisted_bytes: u64,
    last_persisted_at: Option<Instant>,
}

impl ProgressReporter {
    fn should_persist(&self, progress: &nexum_core::nexum_domain::Progress) -> bool {
        self.last_persisted_at.is_none()
            || progress
                .downloaded_bytes
                .saturating_sub(self.last_persisted_bytes)
                >= PROGRESS_MIN_BYTES
            || self
                .last_persisted_at
                .is_some_and(|instant| instant.elapsed() >= PROGRESS_MIN_INTERVAL)
    }

    fn record(&mut self, progress: &nexum_core::nexum_domain::Progress) {
        self.last_persisted_bytes = progress.downloaded_bytes;
        self.last_persisted_at = Some(Instant::now());
    }
}

/// Server configuration loaded from a simple config file or defaults.
pub struct ServerConfig {
    pub port: u16,
    pub max_connections: usize,
    pub data_dir: PathBuf,
    pub require_auth: bool,
    pub auth_scheme: Option<String>,
    pub auth_token: Option<String>,
    pub rate_limit: Option<RateLimit>,
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerConfig")
            .field("port", &self.port)
            .field("max_connections", &self.max_connections)
            .field("data_dir", &self.data_dir)
            .field("require_auth", &self.require_auth)
            .field("auth_scheme", &self.auth_scheme)
            .field("rate_limit", &self.rate_limit)
            .field("tls_cert_path", &self.tls_cert_path)
            .field("tls_key_path", &self.tls_key_path)
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 39100,
            max_connections: 100,
            data_dir: PathBuf::from("./data"),
            require_auth: false,
            auth_scheme: None,
            auth_token: None,
            rate_limit: None,
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}

fn configured_rate_limit(
    requests_per_second: Option<f64>,
    burst_size: Option<u32>,
) -> Result<Option<RateLimit>, String> {
    match (requests_per_second, burst_size) {
        (None, None) => Ok(None),
        (Some(requests_per_second), Some(burst_size))
            if requests_per_second.is_finite() && requests_per_second > 0.0 && burst_size > 0 =>
        {
            Ok(Some(RateLimit {
                requests_per_second,
                burst_size,
            }))
        }
        (Some(_), Some(_)) => {
            Err("rate limit requires a positive finite rps and a positive burst".to_owned())
        }
        _ => Err("rate_limit_rps and rate_limit_burst must be configured together".to_owned()),
    }
}

impl ServerConfig {
    /// Parse a simple key=value config file, falling back to defaults for missing fields.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read config file {path}: {e}"))?;
        let mut port = None;
        let mut max_connections = None;
        let mut data_dir = None;
        let mut require_auth = None;
        let mut auth_scheme = None;
        let mut auth_token = None;
        let mut rate_limit_rps = None;
        let mut rate_limit_burst = None;
        let mut tls_cert_path = None;
        let mut tls_key_path = None;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                match key.trim() {
                    "port" => {
                        if let Ok(p) = value.trim().parse::<u16>() {
                            port = Some(p);
                        }
                    }
                    "max_connections" => {
                        if let Ok(m) = value.trim().parse::<usize>() {
                            max_connections = Some(m);
                        }
                    }
                    "data_dir" => {
                        data_dir = Some(PathBuf::from(value.trim()));
                    }
                    "require_auth" => {
                        require_auth = Some(value.trim().parse::<bool>().unwrap_or(false));
                    }
                    "auth_scheme" => auth_scheme = Some(value.trim().to_owned()),
                    "auth_token" => auth_token = Some(value.trim().to_owned()),
                    "rate_limit_rps" => {
                        rate_limit_rps = Some(value.trim().parse::<f64>().map_err(|_| {
                            "rate_limit_rps must be a positive finite number".to_owned()
                        })?);
                    }
                    "rate_limit_burst" => {
                        rate_limit_burst = Some(value.trim().parse::<u32>().map_err(|_| {
                            "rate_limit_burst must be a positive integer".to_owned()
                        })?);
                    }
                    "tls_cert_path" => tls_cert_path = Some(PathBuf::from(value.trim())),
                    "tls_key_path" => tls_key_path = Some(PathBuf::from(value.trim())),
                    key if key.starts_with("rate_limit_") => {
                        return Err(format!("unknown rate limit config key: {key}"));
                    }
                    _ => {}
                }
            } else if line.starts_with("rate_limit_") {
                return Err(format!("invalid rate limit config line: {line}"));
            }
        }
        Ok(Self {
            port: port.unwrap_or(Self::default().port),
            max_connections: max_connections.unwrap_or(Self::default().max_connections),
            data_dir: data_dir.unwrap_or(Self::default().data_dir),
            require_auth: require_auth.unwrap_or(Self::default().require_auth),
            auth_scheme,
            auth_token,
            rate_limit: configured_rate_limit(rate_limit_rps, rate_limit_burst)?,
            tls_cert_path,
            tls_key_path,
        })
    }

    fn configured_credential(&self) -> Result<Option<Credential>, String> {
        match (&self.auth_scheme, &self.auth_token) {
            (None, None) => Ok(None),
            (Some(scheme), Some(token)) if !token.trim().is_empty() => {
                match scheme.trim().to_ascii_lowercase().as_str() {
                    "bearer" => Ok(Some(Credential::Bearer {
                        token: token.clone(),
                    })),
                    "apikey" => Ok(Some(Credential::ApiKey { key: token.clone() })),
                    _ => Err("auth_scheme must be Bearer or ApiKey".to_owned()),
                }
            }
            (Some(_), Some(_)) => Err("auth_token must not be empty".to_owned()),
            (Some(_), None) | (None, Some(_)) => {
                Err("auth_scheme and auth_token must be configured together".to_owned())
            }
        }
    }
}

fn run_http_transfer(state: Arc<Mutex<ServerState>>, transfer: HttpTransfer) {
    let progress_state = Arc::clone(&state);
    let progress_task_id = transfer.id.clone();
    let latest_progress = Arc::clone(&transfer.latest_progress);
    let control = transfer.control.clone();
    let mut progress_reporter = ProgressReporter::default();
    let result = HttpEngine::new().download_to_resumable_with_control(
        &transfer.source,
        &transfer.destination,
        &transfer.partial_path,
        &control,
        move |progress| {
            *latest_progress
                .lock()
                .expect("HTTP progress mutex poisoned") = progress.clone();
            if !progress_reporter.should_persist(&progress) {
                return Ok(());
            }
            let mut state = progress_state.lock().expect("server state mutex poisoned");
            let result = state
                .core
                .update_progress(&progress_task_id, progress.clone())
                .map_err(|error| {
                    EngineError::Failed(format!("could not persist download progress: {error:?}"))
                });
            if result.is_ok() {
                progress_reporter.record(&progress);
            }
            result
        },
    );
    let completion = transfer.completion.clone();
    let cancelled = ((transfer.remove_requested.load(Ordering::Acquire) || control.is_cancelled())
        && !control.is_committed())
        || matches!(result, Err(EngineError::Cancelled));
    let mut outcome = if cancelled {
        HttpWorkerOutcome::Cancelled
    } else if result.is_ok() {
        HttpWorkerOutcome::Completed
    } else {
        HttpWorkerOutcome::Failed
    };
    let mut dispatch_next = !cancelled;
    {
        let mut state = state.lock().expect("server state mutex poisoned");
        let cancelled = ((transfer.remove_requested.load(Ordering::Acquire)
            || control.is_cancelled())
            && !control.is_committed())
            || matches!(result, Err(EngineError::Cancelled));
        if cancelled {
            outcome = HttpWorkerOutcome::Cancelled;
            dispatch_next = false;
        } else {
            match result {
                Ok(progress) => {
                    let finish_result = state
                        .core
                        .update_progress(&transfer.id, progress)
                        .and_then(|()| state.core.finish_task(&transfer.id, TaskState::Completed));
                    if let Err(error) = finish_result {
                        outcome = HttpWorkerOutcome::Failed;
                        eprintln!("task {} completion failed: {error:?}", transfer.id);
                        if let Err(finish_error) = state.core.finish_task_with_error(
                            &transfer.id,
                            TaskState::Failed,
                            format!("could not finalize HTTP transfer: {error:?}"),
                        ) {
                            eprintln!(
                                "task {} failure state could not be saved: {finish_error:?}",
                                transfer.id
                            );
                        }
                    }
                }
                Err(error) => {
                    outcome = HttpWorkerOutcome::Failed;
                    eprintln!("task {} HTTP transfer failed: {error}", transfer.id);
                    if let Err(finish_error) = state.core.finish_task_with_error(
                        &transfer.id,
                        TaskState::Failed,
                        error.to_string(),
                    ) {
                        eprintln!(
                            "task {} failure state could not be saved: {finish_error:?}",
                            transfer.id
                        );
                    }
                }
            }
        }
        state.active_http.remove(&transfer.id);
        state.active_destinations.remove(&transfer.destination_path);
    }

    completion.finish(outcome);
    if dispatch_next {
        dispatch_available_http_tasks(&state);
    }
}

fn checked_destination(destination: &str, data_dir: &Path) -> io::Result<PathBuf> {
    let destination = Path::new(destination);
    if !matches!(
        destination.components().next_back(),
        Some(Component::Normal(_))
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "download destination must name a file",
        ));
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let path = std::fs::canonicalize(parent)?.join(
        destination
            .file_name()
            .expect("validated destination has a file name"),
    );
    if path.starts_with(data_dir) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "download destination cannot be inside the server data directory",
        ));
    }
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "download destination cannot be a symbolic link",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if path.to_str().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "download destination path must be valid UTF-8",
        ));
    }
    Ok(path)
}

fn reply_error(request: &RpcRequest, error: RpcErrorObject) -> Option<RpcResponse> {
    request
        .id
        .clone()
        .map(|id| RpcResponse::error(Some(id), error))
}

fn claim_next_http_transfer(
    state: &mut ServerState,
) -> Result<Option<HttpTransfer>, RpcErrorObject> {
    let Some(next_id) = state
        .core
        .scheduler
        .next_queued_task_matching(|id| {
            state.core.tasks.get(id).is_some_and(|task| {
                matches!(
                    state.core.resolve_source(task.source.as_str()),
                    Ok(result) if matches!(result.kind, ResolveKind::Http | ResolveKind::Https)
                ) && checked_destination(task.destination.as_str(), &state.data_dir)
                    .is_ok_and(|path| !state.active_destinations.contains(&path))
            })
        })
        .cloned()
    else {
        return Ok(None);
    };
    let task = state
        .core
        .tasks
        .get(&next_id)
        .expect("queued task exists")
        .clone();
    let source = task.source.as_str().to_owned();
    let destination_path = checked_destination(task.destination.as_str(), &state.data_dir)
        .expect("matching queued task has a permitted destination");
    let destination = destination_path
        .to_str()
        .expect("RPC destination path is valid UTF-8")
        .to_owned();
    let partial_path = resumable_partial_path(&destination_path);
    let resume_progress = resumable_partial_progress(&source, &destination, &partial_path)
        .ok()
        .flatten();
    let id = match state.core.claim_queued(&next_id) {
        Ok(true) => next_id,
        Ok(false) => return Ok(None),
        Err(error) => return Err(RpcErrorObject::internal_error(format!("{error:?}"))),
    };
    if let Some(progress) = resume_progress.clone()
        && let Err(error) = state.core.update_progress(&id, progress)
    {
        return Err(RpcErrorObject::internal_error(format!(
            "could not restore HTTP resume progress: {error:?}"
        )));
    }
    let active_destination = destination_path.clone();
    let transfer = HttpTransfer {
        id: id.clone(),
        source,
        destination,
        destination_path,
        partial_path,
        control: HttpTransferControl::new(),
        completion: Arc::new(HttpWorkerCompletion::default()),
        latest_progress: Arc::new(Mutex::new(resume_progress.unwrap_or_default())),
        remove_requested: Arc::new(AtomicBool::new(false)),
    };
    state.active_http.insert(id, transfer.clone());
    state.active_destinations.insert(active_destination);

    Ok(Some(transfer))
}

fn spawn_http_transfer(
    state: &Arc<Mutex<ServerState>>,
    transfer: HttpTransfer,
) -> Result<(), Box<(HttpTransfer, io::Error)>> {
    let id = transfer.id.clone();
    let failed_transfer = transfer.clone();
    let worker_state = Arc::clone(state);
    std::thread::Builder::new()
        .name(format!("nexum-http-{id}"))
        .spawn(move || run_http_transfer(worker_state, transfer))
        .map(|_| ())
        .map_err(|error| Box::new((failed_transfer, error)))
}

fn handle_http_spawn_failure(
    state: &Arc<Mutex<ServerState>>,
    transfer: &HttpTransfer,
    error: &io::Error,
) {
    let mut locked = state.lock().expect("server state mutex poisoned");
    locked.active_http.remove(&transfer.id);
    locked
        .active_destinations
        .remove(&transfer.destination_path);
    if let Err(finish_error) =
        locked
            .core
            .finish_task_with_error(&transfer.id, TaskState::Failed, error.to_string())
    {
        eprintln!(
            "task {} failure state could not be saved: {finish_error:?}",
            transfer.id
        );
    }
}

fn dispatch_available_http_tasks(state: &Arc<Mutex<ServerState>>) {
    loop {
        let transfer = {
            let mut locked = state.lock().expect("server state mutex poisoned");
            match claim_next_http_transfer(&mut locked) {
                Ok(Some(transfer)) => transfer,
                Ok(None) => break,
                Err(error) => {
                    eprintln!("could not claim queued HTTP task: {}", error.message);
                    break;
                }
            }
        };

        if let Err(error) = spawn_http_transfer(state, transfer) {
            let (transfer, error) = *error;
            eprintln!("could not start HTTP worker: {error}");
            // Leave a failed worker claim queued for a later explicit kick, rather
            // than spinning if the process cannot create any more threads.
            handle_http_spawn_failure(state, &transfer, &error);
            break;
        }
    }
}

fn start_http_task(state: &Arc<Mutex<ServerState>>, request: &RpcRequest) -> Option<RpcResponse> {
    let transfer = {
        let mut locked = state.lock().expect("server state mutex poisoned");
        match claim_next_http_transfer(&mut locked) {
            Ok(Some(transfer)) => transfer,
            Ok(None) => {
                let error = if locked.core.scheduler.next_queued_task().is_some() {
                    RpcErrorObject::invalid_params(
                        "no queued HTTP/HTTPS task has an available, permitted destination",
                    )
                } else {
                    RpcErrorObject::task_not_found("no queued task")
                };
                return reply_error(request, error);
            }
            Err(error) => return reply_error(request, error),
        }
    };

    let id = transfer.id.clone();
    if let Err(error) = spawn_http_transfer(state, transfer) {
        let (transfer, error) = *error;
        eprintln!("could not start HTTP worker: {error}");
        handle_http_spawn_failure(state, &transfer, &error);
        return reply_error(request, RpcErrorObject::internal_error(error.to_string()));
    }

    dispatch_available_http_tasks(state);
    request.id.clone().map(|request_id| {
        RpcResponse::success(Some(request_id), serde_json::Value::String(id.to_string()))
    })
}

fn reply_bool(request: &RpcRequest, value: bool) -> Option<RpcResponse> {
    request
        .id
        .clone()
        .map(|id| RpcResponse::success(Some(id), serde_json::Value::Bool(value)))
}

fn request_task_id(request: &RpcRequest) -> Option<TaskId> {
    request
        .params
        .as_ref()
        .and_then(|params| params.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(TaskId::from)
}

fn pause_active_http_task(
    state: &Arc<Mutex<ServerState>>,
    request: &RpcRequest,
    id: &TaskId,
) -> Option<RpcResponse> {
    let transfer = {
        let locked = state.lock().expect("server state mutex poisoned");
        locked.active_http.get(id).cloned()
    }?;

    if let Err(error) = transfer.control.request_pause() {
        return reply_error(
            request,
            RpcErrorObject::invalid_params(format!("could not pause HTTP transfer: {error}")),
        );
    }
    if !transfer.control.is_paused() {
        return reply_error(
            request,
            RpcErrorObject::invalid_params("HTTP transfer finished before it could be paused"),
        );
    }

    let progress = transfer
        .latest_progress
        .lock()
        .expect("HTTP progress mutex poisoned")
        .clone();
    let mut resume_worker = false;
    let result = {
        let mut locked = state.lock().expect("server state mutex poisoned");
        if locked
            .active_http
            .get(id)
            .is_none_or(|active| active.remove_requested.load(Ordering::Acquire))
        {
            Err(RpcErrorObject::invalid_params(
                "HTTP transfer is being removed",
            ))
        } else {
            match locked.core.tasks.get(id).map(|task| task.state) {
                Some(TaskState::Paused) => Ok(()),
                Some(TaskState::Downloading) => locked
                    .core
                    .update_progress(id, progress)
                    .and_then(|()| locked.core.pause_task(id))
                    .map_err(|error| RpcErrorObject::internal_error(format!("{error:?}"))),
                Some(state) => Err(RpcErrorObject::invalid_params(format!(
                    "HTTP transfer is in {state:?} state"
                ))),
                None => Err(RpcErrorObject::task_not_found("task not found")),
            }
        }
    };
    if result.is_err() {
        resume_worker = true;
    }
    if resume_worker {
        let _ = transfer.control.resume();
    }
    match result {
        Ok(()) => {
            dispatch_available_http_tasks(state);
            reply_bool(request, true)
        }
        Err(error) => reply_error(request, error),
    }
}

fn resume_active_http_task(
    state: &Arc<Mutex<ServerState>>,
    request: &RpcRequest,
    id: &TaskId,
) -> Option<RpcResponse> {
    let transfer = {
        let locked = state.lock().expect("server state mutex poisoned");
        locked.active_http.get(id).cloned()
    }?;

    let resumed = {
        let mut locked = state.lock().expect("server state mutex poisoned");
        if transfer.remove_requested.load(Ordering::Acquire) {
            return reply_error(
                request,
                RpcErrorObject::invalid_params("HTTP transfer is being removed"),
            );
        }
        match locked.core.tasks.get(id).map(|task| task.state) {
            Some(TaskState::Paused) => locked
                .core
                .resume_task(id)
                .map_err(|error| RpcErrorObject::internal_error(format!("{error:?}"))),
            Some(TaskState::Downloading) => Ok(false),
            Some(state) => Err(RpcErrorObject::invalid_params(format!(
                "HTTP transfer is in {state:?} state"
            ))),
            None => Err(RpcErrorObject::task_not_found("task not found")),
        }
    };
    match resumed {
        Ok(true) => match transfer.control.resume() {
            Ok(()) => reply_bool(request, true),
            Err(error) => reply_error(
                request,
                RpcErrorObject::invalid_params(format!("could not resume HTTP transfer: {error}")),
            ),
        },
        Ok(false) => reply_bool(request, false),
        Err(error) => reply_error(request, error),
    }
}

fn remove_active_http_task(
    state: &Arc<Mutex<ServerState>>,
    request: &RpcRequest,
    id: &TaskId,
) -> Option<RpcResponse> {
    let mut destination_path = None;
    let mut partial_path = None;
    loop {
        let active = {
            let locked = state.lock().expect("server state mutex poisoned");
            locked.active_http.get(id).cloned()
        };
        let Some(active) = active else {
            break;
        };

        destination_path = Some(active.destination_path.clone());
        partial_path = Some(active.partial_path.clone());
        active.remove_requested.store(true, Ordering::Release);
        active.control.cancel();
        if active.completion.wait(HTTP_CONTROL_WAIT_TIMEOUT).is_none() {
            return reply_error(
                request,
                RpcErrorObject::internal_error("timed out waiting for the HTTP worker to stop"),
            );
        }
    }

    let result = {
        let mut locked = state.lock().expect("server state mutex poisoned");
        if let Some(active) = locked.active_http.get(id).cloned() {
            destination_path = Some(active.destination_path.clone());
            Some(active)
        } else {
            match locked.core.remove_task(id) {
                Ok(task) => {
                    if destination_path.is_none() {
                        destination_path =
                            checked_destination(task.destination.as_str(), &locked.data_dir).ok();
                    }
                    if partial_path.is_none()
                        && let Some(path) = destination_path.as_ref()
                    {
                        partial_path = Some(resumable_partial_path(path));
                    }
                    None
                }
                Err(error) => {
                    return reply_error(
                        request,
                        RpcErrorObject::internal_error(format!("{error:?}")),
                    );
                }
            }
        }
    };
    if result.is_some() {
        return remove_active_http_task(state, request, id);
    }

    if let Some(path) = destination_path {
        let mut locked = state.lock().expect("server state mutex poisoned");
        locked.active_destinations.remove(&path);
    }
    if let Some(path) = partial_path {
        remove_resumable_files(path);
    }
    dispatch_available_http_tasks(state);
    reply_bool(request, true)
}

fn dispatch_server_request(
    state: &Arc<Mutex<ServerState>>,
    request: &RpcRequest,
) -> Option<RpcResponse> {
    if request.method == "task.start" {
        return start_http_task(state, request);
    }

    if let Some(id) = request_task_id(request) {
        match request.method.as_str() {
            "task.pause" => {
                if let Some(response) = pause_active_http_task(state, request, &id) {
                    return Some(response);
                }
            }
            "task.resume" => {
                if let Some(response) = resume_active_http_task(state, request, &id) {
                    return Some(response);
                }
            }
            "task.remove" => {
                if let Some(response) = remove_active_http_task(state, request, &id) {
                    return Some(response);
                }
            }
            _ => {}
        }
    }

    if request.method == "server.auth" {
        let schemes = {
            let locked = state.lock().expect("server state mutex poisoned");
            locked.authentication.schemes()
        };
        return request.id.clone().map(|id| {
            RpcResponse::success(
                Some(id),
                serde_json::to_value(schemes).expect("authentication schemes serialize"),
            )
        });
    }

    let response = {
        let mut locked = state.lock().expect("server state mutex poisoned");
        RpcDispatcher::dispatch(&mut locked.core, request)
    };

    if request.method == "task.queue" {
        dispatch_available_http_tasks(state);
    }
    response
}

const EVENT_PUMP_INTERVAL: Duration = Duration::from_millis(50);
const EVENT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

fn spawn_event_pump(state: Arc<Mutex<ServerState>>) {
    let result = std::thread::Builder::new()
        .name("nexum-events".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(EVENT_PUMP_INTERVAL);
                let mut locked = state.lock().expect("server state mutex poisoned");
                collect_core_events(&mut locked);
            }
        });
    if let Err(error) = result {
        eprintln!("could not start event pump: {error}");
    }
}

fn write_json_line(stream: &mut SharedStream, encoded: &str) -> io::Result<()> {
    stream.write_all(encoded.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

const HTTP_MAX_HEADER_BYTES: usize = 16 * 1024;
const HTTP_MAX_BODY_BYTES: usize = 1024 * 1024;

struct HttpRequest {
    method: String,
    path: String,
    content_length: Option<usize>,
    body: Vec<u8>,
    origin: Option<String>,
    content_type: Option<String>,
}

fn is_http_request_line(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    matches!(
        parts.next(),
        Some("GET" | "HEAD" | "OPTIONS" | "POST" | "PUT" | "DELETE")
    ) && parts.next().is_some()
        && parts
            .next()
            .is_some_and(|version| version.starts_with("HTTP/"))
}

fn write_http_response(
    stream: &mut SharedStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    cors_origin: Option<&str>,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        body.len()
    )?;
    if let Some(origin) = cors_origin {
        write!(stream, "Access-Control-Allow-Origin: {origin}\r\n")?;
        stream.write_all(b"Vary: Origin\r\n")?;
        stream.write_all(b"Access-Control-Allow-Methods: POST, OPTIONS\r\n")?;
        stream.write_all(b"Access-Control-Allow-Headers: Content-Type\r\n")?;
    }
    stream.write_all(b"Connection: close\r\n\r\n")?;
    stream.write_all(body)?;
    stream.flush()
}

fn write_http_json(
    stream: &mut SharedStream,
    status: &str,
    value: &serde_json::Value,
    cors_origin: Option<&str>,
) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(|error| io::Error::other(error.to_string()))?;
    write_http_response(stream, status, "application/json", &body, cors_origin)
}

fn write_http_jsonrpc_response(
    stream: &mut SharedStream,
    response: Option<RpcResponse>,
    cors_origin: Option<&str>,
) -> io::Result<()> {
    let Some(response) = response else {
        return write_http_response(
            stream,
            "204 No Content",
            "application/json",
            &[],
            cors_origin,
        );
    };
    let encoded =
        serialize_response(&response).map_err(|error| io::Error::other(error.to_string()))?;
    write_http_response(
        stream,
        "200 OK",
        "application/json",
        encoded.as_bytes(),
        cors_origin,
    )
}

fn http_error(
    stream: &mut SharedStream,
    status: &str,
    message: &str,
    cors_origin: Option<&str>,
) -> io::Result<()> {
    write_http_json(
        stream,
        status,
        &serde_json::json!({"error": message}),
        cors_origin,
    )
}

fn http_cors_origin(origin: Option<&str>) -> Result<Option<&str>, ()> {
    let Some(origin) = origin else {
        return Ok(None);
    };
    let is_extension_origin = [
        "chrome-extension://",
        "moz-extension://",
        "safari-web-extension://",
    ]
    .into_iter()
    .any(|prefix| {
        origin.strip_prefix(prefix).is_some_and(|extension_id| {
            !extension_id.is_empty()
                && extension_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    });
    if is_extension_origin {
        Ok(Some(origin))
    } else {
        Err(())
    }
}

fn authorize_and_limit_request(
    state: &Arc<Mutex<ServerState>>,
    request: &RpcRequest,
) -> Result<(), RpcErrorObject> {
    let mut locked = state.lock().expect("server state mutex poisoned");
    locked.authentication.authorize(request)?;
    if locked
        .rate_limiter
        .as_mut()
        .is_some_and(|limiter| !limiter.try_acquire(Instant::now()))
    {
        return Err(RpcErrorObject::rate_limited("rate limit exceeded"));
    }
    Ok(())
}

fn read_http_request(
    reader: &mut BufReader<SharedStream>,
    first_line: &str,
) -> io::Result<HttpRequest> {
    if first_line.len() > HTTP_MAX_HEADER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "HTTP request line is too large",
        ));
    }
    let mut request_line = first_line.split_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    let version = request_line.next().unwrap_or_default().to_owned();
    if method.is_empty() || path.is_empty() || !version.starts_with("HTTP/") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "malformed HTTP request line",
        ));
    }

    let mut header_bytes = first_line.len();
    let mut content_length = None;
    let mut content_type = None;
    let mut origin = None;
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP headers ended unexpectedly",
            ));
        }
        header_bytes = header_bytes.saturating_add(read);
        if header_bytes > HTTP_MAX_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HTTP headers are too large",
            ));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "malformed HTTP header",
            ));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let length = value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid HTTP content length")
            })?;
            if content_length.replace(length).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "duplicate HTTP content length",
                ));
            }
        } else if name.trim().eq_ignore_ascii_case("content-type") {
            if content_type.replace(value.trim().to_owned()).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "duplicate HTTP content type",
                ));
            }
        } else if name.trim().eq_ignore_ascii_case("origin") {
            if origin.replace(value.trim().to_owned()).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "duplicate HTTP origin",
                ));
            }
        } else if name.trim().eq_ignore_ascii_case("transfer-encoding")
            && !value.trim().eq_ignore_ascii_case("identity")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "chunked HTTP requests are not supported",
            ));
        }
    }

    let mut body = Vec::new();
    if let Some(length) = content_length {
        if length > HTTP_MAX_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HTTP request body is too large",
            ));
        }
        body.resize(length, 0);
        reader.read_exact(&mut body)?;
    }
    Ok(HttpRequest {
        method,
        path,
        content_length,
        body,
        origin,
        content_type,
    })
}

fn handle_http_connection(
    mut stream: SharedStream,
    mut reader: BufReader<SharedStream>,
    first_line: String,
    state: Arc<Mutex<ServerState>>,
) -> io::Result<()> {
    let parsed = read_http_request(&mut reader, &first_line);
    let HttpRequest {
        method,
        path,
        content_length,
        body,
        origin,
        content_type,
    } = match parsed {
        Ok(request) => request,
        Err(error) => {
            return http_error(&mut stream, "400 Bad Request", &error.to_string(), None);
        }
    };
    let cors_origin = match http_cors_origin(origin.as_deref()) {
        Ok(origin) => origin,
        Err(()) => {
            return http_error(
                &mut stream,
                "403 Forbidden",
                "cross-origin request is not allowed",
                None,
            );
        }
    };

    if path != "/jsonrpc" {
        return http_error(
            &mut stream,
            "404 Not Found",
            "unknown HTTP endpoint",
            cors_origin,
        );
    }
    if method == "OPTIONS" {
        return write_http_response(
            &mut stream,
            "204 No Content",
            "application/json",
            &[],
            cors_origin,
        );
    }
    if method != "POST" {
        return http_error(
            &mut stream,
            "405 Method Not Allowed",
            "only POST /jsonrpc is supported",
            cors_origin,
        );
    }
    if !content_type.as_deref().is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
    }) {
        return http_error(
            &mut stream,
            "415 Unsupported Media Type",
            "Content-Type application/json is required",
            cors_origin,
        );
    }
    if content_length.is_none() {
        return http_error(
            &mut stream,
            "411 Length Required",
            "HTTP content length is required",
            cors_origin,
        );
    }

    let request = match std::str::from_utf8(&body)
        .map_err(|error| error.to_string())
        .and_then(|body| parse_request(body).map_err(|error| error.to_string()))
    {
        Ok(request) => request,
        Err(error) => {
            let response = RpcResponse::error(None, RpcErrorObject::parse_error(error));
            return write_http_jsonrpc_response(&mut stream, Some(response), cors_origin);
        }
    };
    if let Err(error) = authorize_and_limit_request(&state, &request) {
        return write_http_jsonrpc_response(&mut stream, reply_error(&request, error), cors_origin);
    }
    if request.method == "events.subscribe" {
        return write_http_jsonrpc_response(
            &mut stream,
            reply_error(
                &request,
                RpcErrorObject::invalid_params("events.subscribe requires a TCP connection"),
            ),
            cors_origin,
        );
    }
    write_http_jsonrpc_response(
        &mut stream,
        dispatch_server_request(&state, &request),
        cors_origin,
    )
}

fn handle_event_subscription(
    mut stream: SharedStream,
    state: Arc<Mutex<ServerState>>,
    request: RpcRequest,
) -> io::Result<()> {
    let (subscriber_id, receiver, mut last_delivered_sequence) = {
        let mut locked = state.lock().expect("server state mutex poisoned");
        let (subscriber_id, receiver) = locked.events.subscribe();
        (subscriber_id, receiver, locked.events.current_sequence())
    };

    let acknowledged = request.id.clone().map(|id| {
        RpcResponse::success(
            Some(id),
            serde_json::json!({"subscribed": true, "method": EventNotification::METHOD}),
        )
    });
    if let Some(response) = acknowledged {
        let encoded =
            serialize_response(&response).map_err(|error| io::Error::other(error.to_string()))?;
        if let Err(error) = write_json_line(&mut stream, &encoded) {
            let mut locked = state.lock().expect("server state mutex poisoned");
            locked.events.unsubscribe(subscriber_id);
            return Err(error);
        }
    }

    loop {
        match receiver.recv_timeout(EVENT_HEARTBEAT_INTERVAL) {
            Ok(notification) => {
                let encoded = serde_json::to_string(&notification)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                if let Err(error) = write_json_line(&mut stream, &encoded) {
                    let mut locked = state.lock().expect("server state mutex poisoned");
                    locked.events.unsubscribe(subscriber_id);
                    return Err(error);
                }
                last_delivered_sequence = notification.params.sequence;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let heartbeat = EventNotification::new(
                    last_delivered_sequence,
                    EventEnvelope::new("events.heartbeat", serde_json::json!({})),
                );
                let encoded = serde_json::to_string(&heartbeat)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                if let Err(error) = write_json_line(&mut stream, &encoded) {
                    let mut locked = state.lock().expect("server state mutex poisoned");
                    locked.events.unsubscribe(subscriber_id);
                    return Err(error);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let mut locked = state.lock().expect("server state mutex poisoned");
    locked.events.unsubscribe(subscriber_id);
    Ok(())
}

fn handle_connection(mut stream: SharedStream, state: Arc<Mutex<ServerState>>) -> io::Result<()> {
    let reader_stream = stream.clone();
    let mut reader = BufReader::new(reader_stream);
    let mut first_line = String::new();

    loop {
        first_line.clear();
        if reader.read_line(&mut first_line)? == 0 {
            break;
        }
        if first_line.trim().is_empty() {
            continue;
        }
        if is_http_request_line(&first_line) {
            return handle_http_connection(stream, reader, first_line, state);
        }

        let response = match parse_request(&first_line) {
            Ok(request) => match authorize_and_limit_request(&state, &request) {
                Err(error) => reply_error(&request, error),
                Ok(()) if request.method == "events.subscribe" => {
                    return handle_event_subscription(stream, state, request);
                }
                Ok(()) => dispatch_server_request(&state, &request),
            },
            Err(error) => {
                let response =
                    RpcResponse::error(None, RpcErrorObject::parse_error(error.to_string()));
                Some(response)
            }
        };

        if let Some(response) = response {
            let encoded = serialize_response(&response)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            stream.write_all(encoded.as_bytes())?;
            stream.write_all(b"\n")?;
            stream.flush()?;
        }
    }

    Ok(())
}

fn parse_cli_flags() -> Result<(ServerConfig, PathBuf, bool, bool), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    parse_cli_args(&args)
}

fn parse_cli_args(args: &[String]) -> Result<(ServerConfig, PathBuf, bool, bool), String> {
    let mut config = ServerConfig::default();
    let mut config_path = PathBuf::new();
    let mut show_version = false;
    let mut show_help = false;
    let mut has_cli_port = false;
    let mut has_cli_data_dir = false;
    let mut has_cli_max_connections = false;
    let mut has_cli_require_auth = false;
    let mut has_cli_auth_scheme = false;
    let mut has_cli_auth_token = false;
    let mut has_cli_tls_cert = false;
    let mut has_cli_tls_key = false;
    let mut cli_rate_limit_rps = None;
    let mut cli_rate_limit_burst = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                i += 1;
                let path = args.get(i).ok_or("--config requires a path")?;
                if path.is_empty() {
                    return Err("--config requires a non-empty path".to_owned());
                }
                config_path = PathBuf::from(path);
            }
            "--port" => {
                has_cli_port = true;
                i += 1;
                if i < args.len()
                    && let Ok(port) = args[i].parse::<u16>()
                {
                    config.port = port;
                }
            }
            "--data-dir" => {
                has_cli_data_dir = true;
                i += 1;
                if i < args.len() {
                    config.data_dir = PathBuf::from(&args[i]);
                }
            }
            "--max-connections" => {
                has_cli_max_connections = true;
                i += 1;
                if i < args.len()
                    && let Ok(max) = args[i].parse::<usize>()
                {
                    config.max_connections = max;
                }
            }
            "--require-auth" => {
                has_cli_require_auth = true;
                i += 1;
                if i < args.len()
                    && let Ok(req) = args[i].parse::<bool>()
                {
                    config.require_auth = req;
                }
            }
            "--auth-scheme" => {
                has_cli_auth_scheme = true;
                i += 1;
                if i < args.len() {
                    config.auth_scheme = Some(args[i].clone());
                }
            }
            "--auth-token" => {
                has_cli_auth_token = true;
                i += 1;
                if i < args.len() {
                    config.auth_token = Some(args[i].clone());
                }
            }
            "--tls-cert-path" => {
                has_cli_tls_cert = true;
                i += 1;
                config.tls_cert_path = Some(PathBuf::from(
                    args.get(i).ok_or("--tls-cert-path requires a path")?,
                ));
            }
            "--tls-key-path" => {
                has_cli_tls_key = true;
                i += 1;
                config.tls_key_path = Some(PathBuf::from(
                    args.get(i).ok_or("--tls-key-path requires a path")?,
                ));
            }
            "--rate-limit-rps" => {
                i += 1;
                let value = args.get(i).ok_or("--rate-limit-rps requires a value")?;
                cli_rate_limit_rps = Some(
                    value
                        .parse::<f64>()
                        .map_err(|_| "--rate-limit-rps must be a positive finite number")?,
                );
            }
            "--rate-limit-burst" => {
                i += 1;
                let value = args.get(i).ok_or("--rate-limit-burst requires a value")?;
                cli_rate_limit_burst = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| "--rate-limit-burst must be a positive integer")?,
                );
            }
            "--version" => show_version = true,
            "--help" | "-h" => show_help = true,
            flag if flag.starts_with("--rate-limit-") => {
                return Err(format!("unknown rate limit option: {flag}"));
            }
            _ => {}
        }
        i += 1;
    }

    // Load from config file if provided, applying file defaults only where CLI didn't override
    if !config_path.is_empty() {
        let file_config = ServerConfig::from_file(config_path.to_str().unwrap_or(""))?;
        if !has_cli_port {
            config.port = file_config.port;
        }
        if !has_cli_data_dir {
            config.data_dir = file_config.data_dir;
        }
        if !has_cli_max_connections {
            config.max_connections = file_config.max_connections;
        }
        if !has_cli_require_auth {
            config.require_auth = file_config.require_auth;
        }
        if !has_cli_auth_scheme {
            config.auth_scheme = file_config.auth_scheme;
        }
        if !has_cli_auth_token {
            config.auth_token = file_config.auth_token;
        }
        if !has_cli_tls_cert {
            config.tls_cert_path = file_config.tls_cert_path;
        }
        if !has_cli_tls_key {
            config.tls_key_path = file_config.tls_key_path;
        }
        if cli_rate_limit_rps.is_none() && cli_rate_limit_burst.is_none() {
            config.rate_limit = file_config.rate_limit;
        }
    }
    if cli_rate_limit_rps.is_some() || cli_rate_limit_burst.is_some() {
        config.rate_limit = configured_rate_limit(cli_rate_limit_rps, cli_rate_limit_burst)?;
    }

    Ok((config, config_path, show_version, show_help))
}

fn print_usage() {
    eprintln!("usage: nexum-server [OPTIONS]");
    eprintln!("Options:");
    eprintln!("  --config PATH       Load config from file");
    eprintln!("  --port PORT         Server port (default: 39100)");
    eprintln!("  --data-dir PATH     Data directory (default: ./data)");
    eprintln!("  --managed           Desktop-managed mode; requires an absolute --data-dir");
    eprintln!("  --max-connections N Max concurrent connections (default: 100)");
    eprintln!("  --require-auth BOOL Require authentication for all requests");
    eprintln!("  --auth-scheme SCHEME Authentication scheme (Bearer or ApiKey)");
    eprintln!("  --auth-token TOKEN  Authentication secret");
    eprintln!("  --rate-limit-rps N  Maximum average RPC requests per second (requires burst)");
    eprintln!("  --rate-limit-burst N Maximum RPC burst size (requires rps)");
    eprintln!("  --tls-cert-path PATH PEM server certificate (enables TLS with key)");
    eprintln!("  --tls-key-path PATH PEM server private key (enables TLS with certificate)");
    eprintln!("  --version           Show server and protocol version");
    eprintln!("  --help, -h          Show this help");
}

fn lock_data_dir(data_dir: &Path) -> io::Result<File> {
    std::fs::create_dir_all(data_dir).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot create data directory {}: {error}",
                data_dir.display()
            ),
        )
    })?;
    let lock_path = data_dir.join(LOCK_FILE);
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "cannot open data directory lock {}: {error}",
                    lock_path.display()
                ),
            )
        })?;
    lock_file.try_lock().map_err(|error| {
        io::Error::other(format!(
            "cannot acquire data directory lock {}: {error}",
            lock_path.display()
        ))
    })?;
    Ok(lock_file)
}

fn open_core(data_dir: &Path) -> io::Result<ServerCore> {
    let database_path = data_dir.join(DATABASE_FILE);
    let repository = SqliteRepository::open(&database_path)
        .map_err(|error| io::Error::other(format!("{}: {error}", database_path.display())))?;
    let mut core = Core::with_repository(SchedulerConfig::default(), repository)
        .map_err(|error| io::Error::other(error.to_string()))?;
    core.recover()
        .map_err(|error| io::Error::other(format!("task recovery failed: {error:?}")))?;
    Ok(core)
}

fn load_tls_server_config(config: &ServerConfig) -> io::Result<Option<Arc<rustls::ServerConfig>>> {
    let tls_config = TlsConfig {
        cert_path: config.tls_cert_path.clone(),
        key_path: config.tls_key_path.clone(),
        ca_path: None,
    };
    if tls_config.is_complete() {
        return tls_config
            .load_server_config()
            .map(|server| Some(Arc::new(server)))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()));
    }
    if config.tls_cert_path.is_some() || config.tls_key_path.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tls_cert_path and tls_key_path must be configured together",
        ));
    }
    Ok(None)
}

fn managed_mode_requested(args: &[String]) -> bool {
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--managed" => return true,
            "--config" | "--port" | "--data-dir" | "--max-connections" | "--require-auth"
            | "--auth-scheme" | "--auth-token" | "--tls-cert-path" | "--tls-key-path"
            | "--rate-limit-rps" | "--rate-limit-burst" => index += 2,
            _ => index += 1,
        }
    }
    false
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if managed_mode_requested(&args) {
        return managed::run(&args);
    }
    let (config, _config_path, show_version, show_help) =
        parse_cli_flags().map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    if show_version {
        eprintln!("nexum-server {}", env!("CARGO_PKG_VERSION"));
        eprintln!("protocol {}", RpcDispatcher::version());
        return Ok(());
    }

    if show_help {
        print_usage();
        return Ok(());
    }

    let authentication = AuthenticationConfig::from_server_config(&config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let tls_server = load_tls_server_config(&config)?;

    let data_dir_lock = lock_data_dir(&config.data_dir)?;
    let state = Arc::new(Mutex::new(ServerState {
        core: open_core(&config.data_dir)?,
        _data_dir_lock: data_dir_lock,
        data_dir: std::fs::canonicalize(&config.data_dir)?,
        active_http: HashMap::new(),
        active_destinations: HashSet::new(),
        events: EventHub::default(),
        authentication,
        rate_limiter: config
            .rate_limit
            .clone()
            .map(|limit| RateLimiter::new(limit, Instant::now())),
    }));
    let address = format!("127.0.0.1:{}", config.port);
    let listener = TcpListener::bind(&address)?;
    let connection_limiter = ConnectionLimiter::new(config.max_connections);

    spawn_event_pump(Arc::clone(&state));
    dispatch_available_http_tasks(&state);

    eprintln!("Nexum server listening on {address}");
    eprintln!(
        "max_connections: {}, data_dir: {:?}, rate_limit: {:?}",
        config.max_connections, config.data_dir, config.rate_limit
    );

    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let Some(permit) = connection_limiter.try_acquire() else {
                    let _ = stream.shutdown(Shutdown::Both);
                    eprintln!("connection limit reached; closing incoming connection");
                    continue;
                };
                let stream = match tls_server.as_ref() {
                    Some(config) => match ServerConnection::new(Arc::clone(config)) {
                        Ok(connection) => SharedStream::tls(StreamOwned::new(connection, stream)),
                        Err(error) => {
                            eprintln!("could not initialize TLS connection: {error}");
                            continue;
                        }
                    },
                    None => SharedStream::plain(stream),
                };
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    let _permit = permit;
                    if let Err(error) = stream.complete_handshake() {
                        eprintln!("TLS handshake failed: {error}");
                        return;
                    }
                    if let Err(error) = handle_connection(stream, state) {
                        eprintln!("connection error: {error}");
                    }
                });
            }
            Err(error) => eprintln!("accept error: {error}"),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn managed_args(options: &[&str]) -> Vec<String> {
        options.iter().map(|option| (*option).to_owned()).collect()
    }

    #[test]
    fn managed_mode_detection_preserves_external_option_values() {
        for flag in [
            "--config",
            "--port",
            "--data-dir",
            "--max-connections",
            "--require-auth",
            "--auth-scheme",
            "--auth-token",
            "--tls-cert-path",
            "--tls-key-path",
            "--rate-limit-rps",
            "--rate-limit-burst",
        ] {
            assert!(!managed_mode_requested(&managed_args(&[flag, "--managed"])));
        }
        let args = managed_args(&[
            "--auth-scheme",
            "Bearer",
            "--auth-token",
            "--managed",
            "--require-auth",
            "true",
        ]);
        assert!(!managed_mode_requested(&args));
        let (config, _, _, _) = parse_cli_args(&args).unwrap();
        assert!(config.require_auth);
        assert_eq!(config.auth_token.as_deref(), Some("--managed"));
    }

    #[test]
    fn managed_mode_detection_accepts_real_option_in_any_position() {
        for options in [
            vec!["--managed", "--data-dir", "/tmp/nexum"],
            vec!["--data-dir", "/tmp/nexum", "--managed"],
            vec!["--auth-token", "--managed", "--managed"],
            vec!["--ignored", "--managed"],
        ] {
            assert!(managed_mode_requested(&managed_args(&options)));
        }
        assert!(!managed_mode_requested(&managed_args(&[
            "--ignored",
            "value"
        ])));
    }

    #[test]
    fn managed_mode_requires_explicit_absolute_data_dir() {
        for options in [
            vec!["--managed"],
            vec!["--managed", "--data-dir"],
            vec!["--managed", "--data-dir", "./data"],
            vec!["--managed", "--data-dir", ""],
        ] {
            assert!(managed::parse_args(&managed_args(&options)).is_err());
        }
        let config = managed::parse_args(&managed_args(&[
            "--managed",
            "--data-dir",
            "/tmp/nexum-managed",
            "--max-connections",
            "12",
        ]))
        .unwrap();
        assert_eq!(config.port, 0);
        assert_eq!(config.max_connections, 12);
        assert!(config.require_auth);
        assert_eq!(config.auth_scheme.as_deref(), Some("Bearer"));
        assert!(config.auth_token.is_none());
        assert!(config.tls_cert_path.is_none());
        assert!(config.tls_key_path.is_none());
        assert!(config.rate_limit.is_none());
    }

    #[test]
    fn managed_mode_rejects_external_flags_and_duplicate_options() {
        for flag in [
            "--config",
            "--port",
            "--require-auth",
            "--auth-scheme",
            "--auth-token",
            "--tls-cert-path",
            "--tls-key-path",
            "--rate-limit-rps",
            "--rate-limit-burst",
            "--version",
            "--help",
            "--unknown",
        ] {
            let args = managed_args(&[
                "--managed",
                "--data-dir",
                "/tmp/nexum-managed",
                flag,
                "private-token-do-not-log",
            ]);
            let error = managed::parse_args(&args).unwrap_err();
            assert!(!error.contains("private-token-do-not-log"));
        }
        for options in [
            vec!["--managed", "--managed", "--data-dir", "/tmp/nexum"],
            vec![
                "--managed",
                "--data-dir",
                "/tmp/nexum",
                "--data-dir",
                "/tmp/nexum",
            ],
            vec![
                "--managed",
                "--data-dir",
                "/tmp/nexum",
                "--max-connections",
                "10",
                "--max-connections",
                "10",
            ],
        ] {
            assert!(managed::parse_args(&managed_args(&options)).is_err());
        }
    }

    #[test]
    fn managed_mode_rejects_zero_or_invalid_connection_limit() {
        for limit in ["0", "-1", "invalid", "184467440737095516160"] {
            assert!(
                managed::parse_args(&managed_args(&[
                    "--managed",
                    "--data-dir",
                    "/tmp/nexum",
                    "--max-connections",
                    limit,
                ]))
                .is_err()
            );
        }
    }

    #[test]
    fn config_from_default_returns_defaults() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 39100);
        assert_eq!(config.max_connections, 100);
        assert_eq!(config.data_dir, PathBuf::from("./data"));
        assert!(!config.require_auth);
        assert!(config.auth_scheme.is_none());
        assert!(config.auth_token.is_none());
        assert!(config.rate_limit.is_none());
    }

    #[test]
    fn server_version_returns_correct_value() {
        assert_eq!(RpcDispatcher::version(), "1");
    }

    #[test]
    fn connection_limiter_releases_permit_when_handler_finishes() {
        let limiter = ConnectionLimiter::new(1);
        let permit = limiter.try_acquire().expect("first connection should fit");
        assert!(limiter.try_acquire().is_none());
        drop(permit);
        assert!(limiter.try_acquire().is_some());
    }

    #[test]
    fn zero_connection_limit_rejects_all_connections() {
        let limiter = ConnectionLimiter::new(0);
        assert!(limiter.try_acquire().is_none());
    }

    #[test]
    fn rate_limiter_refills_at_configured_rate_up_to_burst() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(
            RateLimit {
                requests_per_second: 2.0,
                burst_size: 2,
            },
            start,
        );
        assert!(limiter.try_acquire(start));
        assert!(limiter.try_acquire(start));
        assert!(!limiter.try_acquire(start));
        assert!(limiter.try_acquire(start + Duration::from_millis(500)));
        assert!(!limiter.try_acquire(start + Duration::from_millis(500)));
        assert!(limiter.try_acquire(start + Duration::from_secs(10)));
        assert!(limiter.try_acquire(start + Duration::from_secs(10)));
        assert!(!limiter.try_acquire(start + Duration::from_secs(10)));
    }

    #[test]
    fn rate_limit_requires_complete_positive_finite_configuration() {
        assert!(configured_rate_limit(None, None).unwrap().is_none());
        assert!(
            configured_rate_limit(Some(10.0), Some(20))
                .unwrap()
                .is_some()
        );
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(configured_rate_limit(Some(invalid), Some(20)).is_err());
        }
        assert!(configured_rate_limit(Some(10.0), Some(0)).is_err());
        assert!(configured_rate_limit(Some(10.0), None).is_err());
        assert!(configured_rate_limit(None, Some(20)).is_err());
    }

    #[test]
    fn parse_config_file_with_all_fields() {
        let dir = std::env::temp_dir().join("nexum-server-test-config");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.txt");
        {
            let mut f = std::fs::File::create(&config_file).unwrap();
            writeln!(f, "port=8080").unwrap();
            writeln!(f, "max_connections=50").unwrap();
            writeln!(f, "data_dir=/tmp/nexum").unwrap();
            writeln!(f, "require_auth=true").unwrap();
            writeln!(f, "auth_scheme=ApiKey").unwrap();
            writeln!(f, "auth_token=test-secret").unwrap();
            writeln!(f, "rate_limit_rps=10").unwrap();
            writeln!(f, "rate_limit_burst=20").unwrap();
            writeln!(f, "tls_cert_path=/tmp/server-cert.pem").unwrap();
            writeln!(f, "tls_key_path=/tmp/server-key.pem").unwrap();
        }
        let config = ServerConfig::from_file(config_file.to_str().unwrap()).unwrap();
        assert_eq!(config.port, 8080);
        assert_eq!(config.max_connections, 50);
        assert_eq!(config.data_dir, PathBuf::from("/tmp/nexum"));
        assert!(config.require_auth);
        assert_eq!(config.auth_scheme.as_deref(), Some("ApiKey"));
        assert_eq!(config.auth_token.as_deref(), Some("test-secret"));
        let rate_limit = config.rate_limit.unwrap();
        assert_eq!(rate_limit.requests_per_second, 10.0);
        assert_eq!(rate_limit.burst_size, 20);
        assert_eq!(
            config.tls_cert_path,
            Some(PathBuf::from("/tmp/server-cert.pem"))
        );
        assert_eq!(
            config.tls_key_path,
            Some(PathBuf::from("/tmp/server-key.pem"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_config_file_uses_defaults_for_missing_fields() {
        let dir = std::env::temp_dir().join("nexum-server-test-partial");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.txt");
        {
            let mut f = std::fs::File::create(&config_file).unwrap();
            writeln!(f, "port=9090").unwrap();
            // Only port is set, max_connections and data_dir should use defaults
        }
        let config = ServerConfig::from_file(config_file.to_str().unwrap()).unwrap();
        assert_eq!(config.port, 9090);
        assert_eq!(config.max_connections, 100);
        assert_eq!(config.data_dir, PathBuf::from("./data"));
        assert!(config.tls_cert_path.is_none());
        assert!(config.tls_key_path.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_rejects_incomplete_tls_paths() {
        let dir = std::env::temp_dir().join("nexum-server-test-tls-partial");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.txt");
        std::fs::write(&config_file, "tls_cert_path=/tmp/server-cert.pem\n").unwrap();
        let config = ServerConfig::from_file(config_file.to_str().unwrap()).unwrap();
        assert!(load_tls_server_config(&config).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_config_file_ignores_comments() {
        let dir = std::env::temp_dir().join("nexum-server-test-comments");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.txt");
        {
            let mut f = std::fs::File::create(&config_file).unwrap();
            writeln!(f, "# This is a comment").unwrap();
            writeln!(f, "port=4444").unwrap();
        }
        let config = ServerConfig::from_file(config_file.to_str().unwrap()).unwrap();
        assert_eq!(config.port, 4444);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_config_file_returns_error_for_missing_file() {
        let result = ServerConfig::from_file("/nonexistent/path/config.txt");
        assert!(result.is_err());
    }

    #[test]
    fn parse_config_file_rejects_incomplete_rate_limit() {
        let dir =
            std::env::temp_dir().join(format!("nexum-server-rate-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.txt");
        std::fs::write(&config_file, "rate_limit_rps=10\n").unwrap();
        assert!(ServerConfig::from_file(config_file.to_str().unwrap()).is_err());
        std::fs::write(&config_file, "rate_limit_rps=NaN\nrate_limit_burst=20\n").unwrap();
        assert!(ServerConfig::from_file(config_file.to_str().unwrap()).is_err());
        std::fs::write(&config_file, "rate_limit_rpss=10\nrate_limit_bursts=20\n").unwrap();
        assert!(ServerConfig::from_file(config_file.to_str().unwrap()).is_err());
        std::fs::write(&config_file, "rate_limit_rps 10\nrate_limit_burst=20\n").unwrap();
        assert!(ServerConfig::from_file(config_file.to_str().unwrap()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn configured_credential_requires_a_supported_complete_pair() {
        let mut config = ServerConfig::default();
        assert_eq!(config.configured_credential().unwrap(), None);

        config.auth_scheme = Some("bearer".to_owned());
        config.auth_token = Some("test-secret".to_owned());
        assert_eq!(
            config.configured_credential().unwrap(),
            Some(Credential::Bearer {
                token: "test-secret".to_owned()
            })
        );

        config.auth_scheme = Some("Basic".to_owned());
        assert_eq!(
            config.configured_credential().unwrap_err(),
            "auth_scheme must be Bearer or ApiKey"
        );

        config.auth_scheme = Some("ApiKey".to_owned());
        config.auth_token = Some("  ".to_owned());
        assert_eq!(
            config.configured_credential().unwrap_err(),
            "auth_token must not be empty"
        );
    }

    #[test]
    fn required_auth_rejects_missing_credentials_at_startup_configuration() {
        let config = ServerConfig {
            require_auth: true,
            ..ServerConfig::default()
        };
        assert_eq!(
            AuthenticationConfig::from_server_config(&config)
                .err()
                .unwrap(),
            "require_auth=true requires auth_scheme and a non-empty auth_token"
        );
    }

    #[test]
    fn parse_args_returns_default_config_without_flags() {
        let (config, config_path, show_version, show_help) = parse_cli_args(&[]).unwrap();
        assert_eq!(config.port, 39100);
        assert_eq!(config.max_connections, 100);
        assert!(config_path.is_empty());
        assert!(!show_version);
        assert!(!show_help);
    }

    #[test]
    fn parse_args_rejects_incomplete_rate_limit_and_missing_config_file() {
        let rps_only = ["--rate-limit-rps".to_owned(), "10".to_owned()];
        assert!(parse_cli_args(&rps_only).is_err());
        let invalid_burst = [
            "--rate-limit-rps".to_owned(),
            "10".to_owned(),
            "--rate-limit-burst".to_owned(),
            "0".to_owned(),
        ];
        assert!(parse_cli_args(&invalid_burst).is_err());
        let missing_config = [
            "--config".to_owned(),
            "/nonexistent/path/config.txt".to_owned(),
        ];
        assert!(parse_cli_args(&missing_config).is_err());
        let empty_config = ["--config".to_owned(), String::new()];
        assert!(parse_cli_args(&empty_config).is_err());
        let misspelled_flag = ["--rate-limit-rsp".to_owned(), "10".to_owned()];
        assert!(parse_cli_args(&misspelled_flag).is_err());
    }

    #[test]
    fn cli_rate_limit_pair_overrides_config_file_pair() {
        let dir =
            std::env::temp_dir().join(format!("nexum-server-rate-override-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.txt");
        std::fs::write(&config_file, "rate_limit_rps=10\nrate_limit_burst=20\n").unwrap();
        let args = [
            "--config".to_owned(),
            config_file.to_string_lossy().into_owned(),
            "--rate-limit-rps".to_owned(),
            "5".to_owned(),
            "--rate-limit-burst".to_owned(),
            "7".to_owned(),
        ];
        let (config, _, _, _) = parse_cli_args(&args).unwrap();
        let rate_limit = config.rate_limit.unwrap();
        assert_eq!(rate_limit.requests_per_second, 5.0);
        assert_eq!(rate_limit.burst_size, 7);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn event_hub_broadcasts_sequenced_notifications_and_cleans_closed_subscribers() {
        let mut hub = EventHub::default();
        let (_, first) = hub.subscribe();
        let (_, second) = hub.subscribe();

        hub.publish(vec![EventEnvelope::new(
            "task.created",
            serde_json::json!({"task_id": "one"}),
        )]);

        assert_eq!(first.recv().unwrap().params.sequence, 1);
        assert_eq!(second.recv().unwrap().params.event, "task.created");
        assert_eq!(hub.subscriber_count(), 2);

        drop(first);
        hub.publish(vec![EventEnvelope::new(
            "task.removed",
            serde_json::json!({"task_id": "one"}),
        )]);
        assert_eq!(hub.subscriber_count(), 1);
        assert_eq!(second.recv().unwrap().params.sequence, 2);
    }
}
