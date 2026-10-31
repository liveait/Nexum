use super::*;
use nexum_core::nexum_domain::{Destination, DownloadSource};
use nexum_core::nexum_storage::TaskRepository;
use nexum_core::nexum_task::DownloadTask;
use serde_json::{Value, json};
use std::sync::atomic::AtomicU64;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct RecoveryFixture {
    root: PathBuf,
    data: PathBuf,
    destination: PathBuf,
    source: String,
    context: CompletionContext,
}

impl RecoveryFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nexum-recovery-ready-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let data = root.join("data");
        std::fs::create_dir(&data).unwrap();
        let destination = root.join("download");
        let partial = resumable_partial_path(&destination);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/complete", listener.local_addr().unwrap());
        let origin = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(&mut stream);
            loop {
                request.clear();
                reader.read_line(&mut request).unwrap();
                if request == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 14\r\nConnection: close\r\n\r\ncomplete bytes")
                .unwrap();
        });
        let context = CompletionContext::new("recover", &data);
        let progress = HttpEngine::new()
            .download_to_resumable_with_completion(
                &source,
                destination.to_str().unwrap(),
                &partial,
                &context,
                &HttpTransferControl::new(),
                |_| Ok(()),
            )
            .unwrap();
        origin.join().unwrap();
        let mut task = DownloadTask::new(
            "recover",
            DownloadSource::new(&source),
            Destination::new(destination.to_string_lossy()),
        );
        task.state = TaskState::Downloading;
        task.progress = progress;
        SqliteRepository::open(data.join(DATABASE_FILE))
            .unwrap()
            .insert(task.into())
            .unwrap();
        Self {
            root,
            data,
            destination,
            source,
            context,
        }
    }
}

impl Drop for RecoveryFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn startup_defers_completion_digest_validation() {
    let fixture = RecoveryFixture::new();
    let receipt_path = completion_receipt_path(&fixture.context);
    let mut receipt: Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    // This remains a bounded, correctly bound v1 receipt. Only a full digest
    // check can reject it; startup must leave that check for a controlled worker.
    receipt["file"]["sha256"] = json!("0".repeat(64));
    std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let core = open_core(&fixture.data, BandwidthPolicyKind::Unlimited).unwrap();
    assert_eq!(
        core.core.tasks.get(&TaskId::from("recover")).unwrap().state,
        TaskState::Queued,
        "startup hashed the downloaded file instead of deferring validation"
    );
    assert!(receipt_path.exists());
    assert_eq!(
        std::fs::read(&fixture.destination).unwrap(),
        b"complete bytes"
    );
}

const CHILD_DIRECTORY: &str = "NEXUM_UNIT_RECOVERY_DIRECTORY";
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn gated_reconcile(
    transfer: &HttpTransfer,
) -> Result<nexum_core::nexum_domain::Progress, EngineError> {
    let root = PathBuf::from(std::env::var_os(CHILD_DIRECTORY).unwrap());
    std::fs::write(root.join("worker-started"), b"started").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !root.join("release-worker").exists()
        && !transfer.control.is_pause_requested()
        && !transfer.control.is_cancelled()
    {
        assert!(
            Instant::now() < deadline,
            "test did not release recovery verifier"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // Enter the real verifier. A pause request now stops at its first digest
    // checkpoint; cancellation uses the real terminal/control path as well.
    reconcile_http_transfer(transfer)
}

// A subprocess entry in the unit-test executable, absent from production builds.
// It exercises the same managed bootstrap/RPC/worker loop with a controllable
// verifier dependency. Readiness has a dedicated pipe file because the Rust
// test runner also writes its own progress text to stdout.
#[test]
fn managed_recovery_child() {
    let Some(root) = std::env::var_os(CHILD_DIRECTORY).map(PathBuf::from) else {
        return;
    };
    let args = vec![
        "--managed".into(),
        "--data-dir".into(),
        root.join("data").to_str().unwrap().into(),
    ];
    let mut output = File::create(root.join("ready-frame")).unwrap();
    let result = managed::run_with(&args, gated_reconcile, &mut output);
    if let Err(error) = &result {
        eprintln!("test managed child: {error}");
    }
    // Production main also exits without joining stalled recovery workers on
    // EOF. Explicit exit prevents the test runner from retaining that worker.
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

struct RecoveryChild {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    endpoint: String,
    root: PathBuf,
}

impl RecoveryChild {
    fn start(fixture: &RecoveryFixture) -> Self {
        use std::process::{Command, Stdio};
        for name in ["ready-frame", "worker-started"] {
            let _ = std::fs::remove_file(fixture.root.join(name));
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "recovery_readiness_tests::managed_recovery_child",
                "--nocapture",
            ])
            .env(CHILD_DIRECTORY, &fixture.root)
            .stdin(Stdio::piped())
            .stdout(File::create(fixture.root.join("child-stdout")).unwrap())
            .stderr(File::create(fixture.root.join("child-stderr")).unwrap())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        nexum_protocol::managed::write_managed_frame(
            &mut stdin,
            &json!({
                "type": "start", "control_version": 1,
                "server_version": env!("CARGO_PKG_VERSION"), "protocol_version": "1",
                "bearer_token": TOKEN
            }),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(6);
        let endpoint = loop {
            let bytes = std::fs::read(fixture.root.join("ready-frame")).unwrap_or_default();
            if bytes.len() >= 4 {
                let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
                if bytes.len() == 4 + length {
                    let ready: Value = serde_json::from_slice(&bytes[4..]).unwrap();
                    assert_eq!(ready["process_id"], child.id());
                    break ready["endpoint"].as_str().unwrap().to_owned();
                }
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before Ready"
            );
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("blocked recovery delayed Ready beyond six seconds");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        Self {
            child,
            stdin: Some(stdin),
            endpoint,
            root: fixture.root.clone(),
        }
    }

    fn wait_started(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.root.join("worker-started").exists() {
            assert!(Instant::now() < deadline, "recovery worker did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn call(&self, method: &str, params: Option<Value>) -> Value {
        let mut stream = TcpStream::connect(&self.endpoint).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        writeln!(
            stream,
            "{}",
            json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
                "credential": {"Bearer": {"token": TOKEN}}
            })
        )
        .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert!(response.get("error").is_none(), "{method}: {response}");
        response["result"].clone()
    }

    fn wait_state(&self, id: &str, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let task = self.call("task.get", Some(json!({"id": id})));
            if task["state"] == expected {
                return;
            }
            assert!(Instant::now() < deadline, "expected {expected}, got {task}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn eof(&mut self) {
        self.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(
                Instant::now() < deadline,
                "managed shutdown waited for recovery hashing"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for RecoveryChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn managed_ready_and_rpc_remain_available_during_controlled_recovery() {
    let fixture = RecoveryFixture::new();
    let mut server = RecoveryChild::start(&fixture);
    server.wait_started();
    // Verification is deliberately blocked until after Ready and these RPCs.
    assert_eq!(server.call("server.version", None), json!("1"));
    assert!(completion_receipt_path(&fixture.context).exists());
    server.call(
        "task.create",
        Some(json!({
            "id": "overlap", "source": fixture.source,
            "destination": fixture.destination.to_string_lossy()
        })),
    );
    server.call("task.queue", Some(json!({"id": "overlap"})));
    assert_eq!(
        server.call("task.get", Some(json!({"id": "overlap"})))["state"],
        "Queued"
    );
    assert_eq!(
        server.call("task.pause", Some(json!({"id": "recover"}))),
        true
    );
    let paused = server.call("task.get", Some(json!({"id": "recover"})));
    assert_eq!(paused["state"], "Paused");
    assert_eq!(
        server.call("task.get", Some(json!({"id": "overlap"})))["state"],
        "Queued"
    );
    assert_eq!(
        std::fs::read(&fixture.destination).unwrap(),
        b"complete bytes"
    );
    assert!(completion_receipt_path(&fixture.context).exists());
    // A paused recovery still reserves its target while releasing its scheduler
    // slot. Resuming performs the actual digest and exact completion commit.
    assert_eq!(
        server.call("task.resume", Some(json!({"id": "recover"}))),
        true
    );
    server.wait_state("recover", "Completed");
    let complete = server.call("task.get", Some(json!({"id": "recover"})));
    assert_eq!(complete["downloaded_bytes"], 14);
    assert_eq!(complete["total_bytes"], 14);
    assert_eq!(complete["error"], Value::Null);
    server.eof();
    let stored = SqliteRepository::open(fixture.data.join(DATABASE_FILE))
        .unwrap()
        .get(&TaskId::from("recover"))
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, TaskState::Completed);
    assert_eq!(stored.progress.downloaded_bytes, 14);
}

#[test]
fn managed_cancellation_stops_recovery_before_staging_publication() {
    let fixture = RecoveryFixture::new();
    let partial = resumable_partial_path(&fixture.destination);
    std::fs::rename(&fixture.destination, &partial).unwrap();
    let mut server = RecoveryChild::start(&fixture);
    server.wait_started();
    assert_eq!(
        server.call("task.remove", Some(json!({"id": "recover"}))),
        true
    );
    assert_eq!(server.call("task.list", None), json!([]));
    assert!(!fixture.destination.exists());
    assert!(!partial.exists());
    assert!(!completion_receipt_path(&fixture.context).exists());
    server.eof();
    assert!(
        SqliteRepository::open(fixture.data.join(DATABASE_FILE))
            .unwrap()
            .get(&TaskId::from("recover"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn managed_eof_preserves_blocked_recovery_for_the_next_process() {
    let fixture = RecoveryFixture::new();
    let partial = resumable_partial_path(&fixture.destination);
    std::fs::rename(&fixture.destination, &partial).unwrap();
    let mut server = RecoveryChild::start(&fixture);
    server.wait_started();
    server.eof();
    assert!(partial.exists());
    assert!(!fixture.destination.exists());
    assert!(completion_receipt_path(&fixture.context).exists());
    std::fs::write(fixture.root.join("release-worker"), b"resume").unwrap();
    let mut restarted = RecoveryChild::start(&fixture);
    restarted.wait_state("recover", "Completed");
    assert_eq!(
        std::fs::read(&fixture.destination).unwrap(),
        b"complete bytes"
    );
    restarted.eof();
}

#[test]
fn a_receipt_removed_while_queued_never_turns_recovery_into_a_download() {
    let fixture = RecoveryFixture::new();
    let opened = open_core(&fixture.data, BandwidthPolicyKind::Unlimited).unwrap();
    std::fs::remove_file(completion_receipt_path(&fixture.context)).unwrap();
    std::fs::remove_file(&fixture.destination).unwrap();
    let mut state = state_for(&fixture, opened);
    let transfer = claim_next_http_transfer(&mut state).unwrap().unwrap();
    assert!(transfer.recovery_only);
    let control = transfer.control.clone();
    let state = Arc::new(Mutex::new(state));
    run_http_transfer(Arc::clone(&state), transfer);
    let state = state.lock().unwrap();
    let task = state.core.tasks.get(&TaskId::from("recover")).unwrap();
    assert_eq!(task.state, TaskState::Failed);
    assert!(
        task.last_error
            .as_deref()
            .unwrap()
            .contains("pending completion receipt disappeared")
    );
    assert!(state.core.scheduler.next_queued_task().is_none());
    assert!(state.active_destinations.is_empty());
    assert!(state.active_http.is_empty());
    assert!(control.is_finished());
    assert!(!fixture.destination.exists());
}

fn state_for(fixture: &RecoveryFixture, opened: OpenedCore) -> ServerState {
    ServerState {
        core: opened.core,
        pending_completions: opened.pending_completions,
        _data_dir_lock: lock_data_dir(&fixture.data).unwrap(),
        data_dir: fixture.data.clone(),
        active_http: HashMap::new(),
        reconcile_completion: reconcile_http_transfer,
        active_destinations: HashSet::new(),
        events: EventHub::default(),
        authentication: AuthenticationConfig {
            required: false,
            credential: None,
        },
        rate_limiter: None,
    }
}

#[test]
fn deferred_digest_failure_keeps_evidence_and_does_not_requeue() {
    let fixture = RecoveryFixture::new();
    let receipt_path = completion_receipt_path(&fixture.context);
    let mut receipt: Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    receipt["file"]["sha256"] = json!("0".repeat(64));
    std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let opened = open_core(&fixture.data, BandwidthPolicyKind::Unlimited).unwrap();
    let mut state = state_for(&fixture, opened);
    let transfer = claim_next_http_transfer(&mut state).unwrap().unwrap();
    let control = transfer.control.clone();
    let state = Arc::new(Mutex::new(state));
    run_http_transfer(Arc::clone(&state), transfer);
    let mut state = state.lock().unwrap();
    let task = state.core.tasks.get(&TaskId::from("recover")).unwrap();
    assert_eq!(task.state, TaskState::Failed);
    assert!(
        task.last_error
            .as_deref()
            .unwrap()
            .contains("does not match the completed response")
    );
    assert!(state.core.scheduler.next_queued_task().is_none());
    assert!(control.is_finished());
    assert!(receipt_path.exists());
    assert_eq!(
        std::fs::read(&fixture.destination).unwrap(),
        b"complete bytes"
    );
    let stored = SqliteRepository::open(fixture.data.join(DATABASE_FILE))
        .unwrap()
        .get(&TaskId::from("recover"))
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, TaskState::Failed);
    assert_eq!(stored.last_error, task.last_error);
    let events = state.core.drain_task_events();
    assert!(events.iter().any(|event| matches!(
        event,
        nexum_core::nexum_task::TaskEvent::StateChanged {
            to: TaskState::Failed,
            ..
        }
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, nexum_core::nexum_task::TaskEvent::Removed { .. })),
        "a failed recovery must remain visible to event consumers"
    );
}

#[cfg(unix)]
#[test]
fn queued_recovery_with_a_missing_receipt_and_symlink_fails_visibly() {
    let fixture = RecoveryFixture::new();
    let opened = open_core(&fixture.data, BandwidthPolicyKind::Unlimited).unwrap();
    std::fs::remove_file(completion_receipt_path(&fixture.context)).unwrap();
    let original = fixture.root.join("original");
    std::fs::rename(&fixture.destination, &original).unwrap();
    std::os::unix::fs::symlink(&original, &fixture.destination).unwrap();
    let mut state = state_for(&fixture, opened);
    let transfer = claim_next_http_transfer(&mut state)
        .unwrap()
        .expect("known recovery must reach its verifier despite a replaced destination");
    assert!(transfer.recovery_only);
    let state = Arc::new(Mutex::new(state));
    run_http_transfer(Arc::clone(&state), transfer);
    let state = state.lock().unwrap();
    assert_eq!(
        state
            .core
            .tasks
            .get(&TaskId::from("recover"))
            .unwrap()
            .state,
        TaskState::Failed
    );
    assert!(
        std::fs::symlink_metadata(&fixture.destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(original).unwrap(), b"complete bytes");
}

fn assert_fresh_completed_response_is_not_retried(reject_failure_save: bool) {
    use nexum_core::nexum_domain::Progress;
    use nexum_core::nexum_task::TaskEvent;

    let fixture = RecoveryFixture::new();
    let opened = open_core(&fixture.data, BandwidthPolicyKind::Unlimited).unwrap();
    let mut state = state_for(&fixture, opened);
    let transfer = claim_next_http_transfer(&mut state).unwrap().unwrap();
    let id = TaskId::from("recover");
    // A fresh transfer has not yet registered pending recovery when its
    // Engine returns the first completed response and receipt.
    state.pending_completions.remove(&id);
    let previous_progress = Progress::new(5, Some(14));
    state
        .core
        .update_progress(&id, previous_progress.clone())
        .unwrap();
    state.core.drain_task_events();
    state.core.drain_scheduler_events();
    let database = rusqlite::Connection::open(fixture.data.join(DATABASE_FILE)).unwrap();
    let rejected_states = if reject_failure_save {
        "('completed', 'failed')"
    } else {
        "('completed')"
    };
    database
        .execute_batch(&format!(
            "CREATE TRIGGER reject_fresh_completion BEFORE UPDATE ON tasks
         WHEN NEW.state IN {rejected_states}
         BEGIN SELECT RAISE(FAIL, 'fresh completion save refused'); END;"
        ))
        .unwrap();
    let state = Arc::new(Mutex::new(state));
    // The real Engine already supplied complete bytes and a durable receipt.
    // Exercise the fresh-download branch synchronously, stopping precisely
    // after finalization and before run_http_transfer could dispatch anything.
    let (outcome, dispatch_next) = complete_http_transfer(
        &state,
        &id,
        &transfer.destination_path,
        &transfer.source,
        &transfer.destination,
        &transfer.partial_path,
        &transfer.completion_context,
        false,
        Ok(Progress::new(14, Some(14))),
    );
    assert_eq!(outcome, HttpWorkerOutcome::Failed);
    assert!(dispatch_next);
    let mut state = state.lock().unwrap();
    let task = state.core.tasks.get(&id).unwrap().clone();
    let stored = state.core.repository.get(&id).unwrap().unwrap();
    assert_eq!(task.progress, previous_progress);
    assert_eq!(stored.progress, previous_progress);
    assert!(
        state.core.scheduler.next_queued_task().is_none(),
        "completed response bytes must never enter the network retry queue"
    );
    let events = state.core.drain_task_events();
    if reject_failure_save {
        assert_eq!(task.state, TaskState::Downloading);
        assert_eq!(stored.state, TaskState::Downloading);
        assert_eq!(task.last_error, None);
        assert_eq!(stored.last_error, None);
        assert_eq!(state.core.scheduler.active_len(), 1);
        assert!(
            events.is_empty(),
            "a rejected terminal write must not expose transient states"
        );
    } else {
        assert_eq!(task.state, TaskState::Failed);
        assert_eq!(stored.state, TaskState::Failed);
        assert!(
            task.last_error
                .as_deref()
                .unwrap()
                .contains("could not finalize HTTP transfer")
        );
        assert_eq!(stored.last_error, task.last_error);
        assert_eq!(state.core.scheduler.active_len(), 0);
        assert_eq!(
            events,
            vec![TaskEvent::StateChanged {
                task_id: id,
                from: TaskState::Downloading,
                to: TaskState::Failed,
            }]
        );
    }
    assert!(state.core.drain_scheduler_events().is_empty());
    assert!(state.pending_completions.contains(&TaskId::from("recover")));
    assert!(completion_receipt_path(&fixture.context).exists());
    assert_eq!(
        std::fs::read(&fixture.destination).unwrap(),
        b"complete bytes"
    );
}

#[test]
fn fresh_completed_bytes_do_not_retry_when_completion_persistence_fails() {
    assert_fresh_completed_response_is_not_retried(false);
}

#[test]
fn fresh_completed_bytes_keep_claim_snapshot_when_all_terminal_writes_fail() {
    assert_fresh_completed_response_is_not_retried(true);
}
