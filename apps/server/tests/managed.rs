//! Exercise the managed child as an independent parent, including its raw wire format.

use serde_json::{Value, json};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER_TOKEN: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const PIPE_LIMIT: usize = 64 * 1024;
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    root: PathBuf,
    data: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nexum-managed-test-{}-{stamp}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let data = root.join("data");
        Self { root, data }
    }

    fn assert_no_storage(&self) {
        if self.data.exists() {
            assert_eq!(
                fs::read_dir(&self.data).unwrap().count(),
                0,
                "managed startup wrote storage before accepting its credential"
            );
        }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// Drain both pipes continuously so a child cannot deadlock on diagnostics. Capture
// only a bounded prefix, and report overflow instead of allocating without limit.
struct PipeCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
    overflow: Arc<AtomicBool>,
    done: Receiver<io::Result<()>>,
    worker: Option<JoinHandle<()>>,
}

impl PipeCapture {
    fn new(mut pipe: impl Read + Send + 'static) -> Self {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let overflow = Arc::new(AtomicBool::new(false));
        let captured = Arc::clone(&bytes);
        let too_large = Arc::clone(&overflow);
        let (done_tx, done) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = (|| {
                let mut chunk = [0; 1024];
                loop {
                    let count = pipe.read(&mut chunk)?;
                    if count == 0 {
                        return Ok(());
                    }
                    let mut bytes = captured.lock().unwrap();
                    let available = PIPE_LIMIT - bytes.len();
                    bytes.extend_from_slice(&chunk[..count.min(available)]);
                    if count > available {
                        too_large.store(true, Ordering::Relaxed);
                    }
                }
            })();
            let _ = done_tx.send(result);
        });
        Self {
            bytes,
            overflow,
            done,
            worker: Some(worker),
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        assert!(
            !self.overflow.load(Ordering::Relaxed),
            "child exceeded capture limit"
        );
        self.bytes.lock().unwrap().clone()
    }

    fn finish(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.done
                .recv_timeout(Duration::from_secs(1))
                .expect("child pipe did not close")
                .unwrap();
            worker.join().unwrap();
        }
    }
}

struct ManagedChild {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: PipeCapture,
    stderr: PipeCapture,
    endpoint: Option<String>,
}

impl ManagedChild {
    fn spawn(data: &Path) -> Self {
        assert!(data.is_absolute());
        let mut child = Command::new(env!("CARGO_BIN_EXE_nexum-server"))
            .arg("--managed")
            .arg("--data-dir")
            .arg(data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = PipeCapture::new(child.stdout.take().unwrap());
        let stderr = PipeCapture::new(child.stderr.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            stderr,
            endpoint: None,
        }
    }

    fn start(data: &Path) -> Self {
        let mut child = Self::spawn(data);
        child.frame(&startup(TOKEN));
        child.ready();
        child
    }

    fn bytes(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }

    fn frame(&mut self, value: &Value) {
        // Deliberately do not use nexum_protocol's implementation here.
        let json = serde_json::to_vec(value).unwrap();
        let mut bytes = (json.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&json);
        self.bytes(&bytes);
    }

    fn ready(&mut self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            let bytes = self.stdout.snapshot();
            if bytes.len() >= 4 {
                let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
                assert!((1..=4096).contains(&length), "invalid readiness framing");
                if bytes.len() >= length + 4 {
                    assert_eq!(bytes.len(), length + 4, "stdout contains extra data");
                    let ready: Value = serde_json::from_slice(&bytes[4..]).unwrap();
                    assert_eq!(ready["control_version"], 1);
                    assert_eq!(ready["server_version"], env!("CARGO_PKG_VERSION"));
                    assert_eq!(ready["protocol_version"], "1");
                    assert_eq!(ready["process_id"], self.child.id());
                    let endpoint = ready["endpoint"].as_str().unwrap();
                    let address: std::net::SocketAddr = endpoint.parse().unwrap();
                    assert_eq!(address.ip(), std::net::Ipv4Addr::LOCALHOST);
                    assert_ne!(address.port(), 0);
                    assert_eq!(ready.as_object().unwrap().len(), 5);
                    assert!(ready.get("bearer_token").is_none());
                    self.endpoint = Some(endpoint.to_owned());
                    return ready;
                }
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!(
                    "managed server exited before readiness: {status}, {}",
                    String::from_utf8_lossy(&self.stderr.snapshot())
                );
            }
            assert!(
                Instant::now() < deadline,
                "managed startup exceeded its deadline"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn exit(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                self.stdout.finish();
                self.stderr.finish();
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "managed child did not exit within {timeout:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn eof(&mut self) {
        self.stdin.take();
    }

    fn assert_private_output(&self, secret: &str) {
        for bytes in [self.stdout.snapshot(), self.stderr.snapshot()] {
            assert!(
                !String::from_utf8_lossy(&bytes).contains(secret),
                "secret leaked through child output"
            );
        }
    }

    fn assert_one_ready_frame(&self) {
        let bytes = self.stdout.snapshot();
        assert!(bytes.len() >= 4);
        let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(
            bytes.len(),
            length + 4,
            "child wrote more than one readiness frame"
        );
    }

    fn assert_no_listener(&self) {
        #[cfg(target_os = "linux")]
        for descriptor in fs::read_dir(format!("/proc/{}/fd", self.child.id())).unwrap() {
            let descriptor = descriptor.unwrap();
            if let Ok(target) = fs::read_link(descriptor.path()) {
                assert!(
                    !target.to_string_lossy().starts_with("socket:"),
                    "child created a socket before accepting startup"
                );
            }
        }
        #[cfg(target_os = "macos")]
        {
            let listeners = Command::new("/usr/sbin/lsof")
                .args([
                    "-nP",
                    "-a",
                    "-p",
                    &self.child.id().to_string(),
                    "-iTCP",
                    "-sTCP:LISTEN",
                ])
                .output()
                .unwrap();
            assert_eq!(
                listeners.status.code(),
                Some(1),
                "child has a listener before startup"
            );
            assert!(listeners.stdout.is_empty());
        }
    }

    fn rpc(&self, method: &str, params: Option<Value>, token: Option<&str>) -> Value {
        let mut stream = self.connect();
        writeln!(stream, "{}", rpc_request(method, params, token)).unwrap();
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .unwrap_or_else(|error| {
                panic!(
                    "{method} read failed ({error}); credential present: {}; child diagnostics: {}",
                    token.is_some(),
                    String::from_utf8_lossy(&self.stderr.snapshot())
                );
            });
        serde_json::from_str(&line).unwrap()
    }

    fn call(&self, method: &str, params: Option<Value>) -> Value {
        let response = self.rpc(method, params, Some(TOKEN));
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(self.endpoint.as_ref().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
    }

    fn http(&self, method: &str, token: Option<&str>) -> Value {
        let mut stream = self.connect();
        let body = rpc_request(method, None, token).to_string();
        write!(stream,
            "POST /jsonrpc HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("HTTP/1.1 200 OK"), "{headers}");
        serde_json::from_str(body).unwrap()
    }

    fn subscribe(&self, token: Option<&str>) -> (BufReader<TcpStream>, Value) {
        let mut stream = self.connect();
        writeln!(stream, "{}", rpc_request("events.subscribe", None, token)).unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        (reader, serde_json::from_str(&line).unwrap())
    }

    fn wait_task(&self, id: &str, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let task = self.call("task.get", Some(json!({"id": id})));
            if task["state"] == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "task never reached {expected}: {task}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn startup(token: &str) -> Value {
    json!({
        "type": "start", "control_version": 1,
        "server_version": env!("CARGO_PKG_VERSION"), "protocol_version": "1", "bearer_token": token
    })
}

fn rpc_request(method: &str, params: Option<Value>, token: Option<&str>) -> Value {
    let mut request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    if let Some(token) = token {
        request["credential"] = json!({"Bearer": {"token": token}});
    }
    request
}

fn assert_unauthorized(response: &Value) {
    assert_eq!(response["error"]["code"], -32001, "{response}");
    assert!(
        response.get("result").is_none(),
        "unauthorized call produced a result"
    );
}

#[test]
fn managed_readiness_uses_a_private_ephemeral_authenticated_endpoint() {
    let dir = TestDir::new();
    // If a developer's external server is already there, leave it untouched.
    // Otherwise occupy its port, proving managed startup cannot use that default.
    let _external_port = TcpListener::bind("127.0.0.1:39100").ok();
    let mut server = ManagedChild::start(&dir.data);
    assert_ne!(server.endpoint.as_deref(), Some("127.0.0.1:39100"));

    for method in ["server.version", "task.list"] {
        for token in [None, Some(OTHER_TOKEN)] {
            assert_unauthorized(&server.rpc(method, None, token));
            assert_unauthorized(&server.http(method, token));
        }
        let expected = if method == "server.version" {
            json!("1")
        } else {
            json!([])
        };
        assert_eq!(server.call(method, None), expected);
        assert_eq!(server.http(method, Some(TOKEN))["result"], expected);
    }
    for token in [None, Some(OTHER_TOKEN)] {
        let (events, rejected) = server.subscribe(token);
        assert_unauthorized(&rejected);
        drop(events);
    }
    let (mut events, accepted) = server.subscribe(Some(TOKEN));
    assert_eq!(accepted["result"]["subscribed"], true);
    server.call("task.create", Some(json!({
        "id": "private-event", "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        "destination": dir.root.join("private-event").to_string_lossy()
    })));
    let mut line = String::new();
    events.read_line(&mut line).unwrap();
    let event: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["method"], "events.event");
    assert_eq!(event["params"]["event"], "task.created");
    drop(events);

    // The actual process command remains independent of the private pipe secret.
    #[cfg(target_os = "linux")]
    {
        let args = fs::read(format!("/proc/{}/cmdline", server.child.id())).unwrap();
        assert!(!String::from_utf8_lossy(&args).contains(TOKEN));
        let environment = fs::read(format!("/proc/{}/environ", server.child.id())).unwrap();
        assert!(!String::from_utf8_lossy(&environment).contains(TOKEN));
    }
    #[cfg(target_os = "macos")]
    {
        let command = Command::new("/bin/ps")
            .args([
                "eww",
                "-p",
                &server.child.id().to_string(),
                "-o",
                "command=",
            ])
            .output()
            .unwrap();
        assert!(command.status.success());
        assert!(!String::from_utf8_lossy(&command.stdout).contains(TOKEN));
    }
    server.frame(&json!({"type": "shutdown", "control_version": 1}));
    assert!(server.exit(Duration::from_secs(2)).success());
    server.assert_private_output(TOKEN);
    server.assert_one_ready_frame();
    let database = fs::read(dir.data.join("nexum.sqlite")).unwrap();
    assert!(
        !database
            .windows(TOKEN.len())
            .any(|window| window == TOKEN.as_bytes())
    );
}

#[test]
fn managed_eof_releases_the_data_lock_and_keeps_tasks() {
    let dir = TestDir::new();
    let mut server = ManagedChild::start(&dir.data);
    server.call("task.create", Some(json!({
        "id": "retained", "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        "destination": dir.root.join("retained").to_string_lossy()
    })));
    let before = server.call("task.list", None);
    server.eof();
    assert!(server.exit(Duration::from_secs(2)).success());
    server.assert_one_ready_frame();
    server.assert_private_output(TOKEN);
    let mut restarted = ManagedChild::start(&dir.data);
    assert_eq!(restarted.call("task.list", None), before);
    restarted.frame(&json!({"type": "shutdown", "control_version": 1}));
    assert!(restarted.exit(Duration::from_secs(2)).success());
    // Shutdown, as well as EOF, must release the lock.
    let mut third = ManagedChild::start(&dir.data);
    third.eof();
    assert!(third.exit(Duration::from_secs(2)).success());
}

#[test]
fn managed_accepts_a_delayed_request_and_shuts_down_with_idle_connections() {
    let dir = TestDir::new();
    let mut server = ManagedChild::start(&dir.data);
    let mut stream = server.connect();
    // On macOS an accepted socket inherits a listener's nonblocking flag.
    // Delay the request until its worker has entered read_line, so forgetting to
    // reset the accepted socket's flag closes this connection with WouldBlock.
    thread::sleep(Duration::from_millis(100));
    writeln!(
        stream,
        "{}",
        rpc_request("server.version", None, Some(TOKEN))
    )
    .unwrap();
    let mut idle_rpc = BufReader::new(stream);
    let mut line = String::new();
    idle_rpc.read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["result"], "1");

    let (mut idle_events, subscribed) = server.subscribe(Some(TOKEN));
    assert_eq!(subscribed["result"]["subscribed"], true);
    // Both workers remain blocked on their own input/event wait. The private
    // control loop must stop the process without waiting for either worker.
    server.frame(&json!({"type": "shutdown", "control_version": 1}));
    assert!(server.exit(Duration::from_secs(2)).success());
    line.clear();
    assert_eq!(idle_rpc.read_line(&mut line).unwrap(), 0);
    assert_eq!(idle_events.read_line(&mut line).unwrap(), 0);
    server.assert_one_ready_frame();
    server.assert_private_output(TOKEN);
}

#[test]
fn second_managed_child_cannot_adopt_or_stop_the_first_child() {
    let dir = TestDir::new();
    let mut first = ManagedChild::start(&dir.data);
    let mut second = ManagedChild::spawn(&dir.data);
    second.frame(&startup(OTHER_TOKEN));
    assert!(!second.exit(Duration::from_secs(2)).success());
    assert!(second.stdout.snapshot().is_empty());
    second.assert_private_output(OTHER_TOKEN);
    assert_eq!(first.call("server.version", None), "1");
    assert!(first.child.try_wait().unwrap().is_none());
    first.eof();
    assert!(first.exit(Duration::from_secs(2)).success());
}

#[test]
fn managed_startup_rejects_incompatible_versions_and_invalid_tokens() {
    let mut cases = Vec::new();
    for (field, invalid) in [
        ("control_version", json!(2)),
        ("server_version", json!("not-the-built-version")),
        ("protocol_version", json!("9")),
        ("type", json!("shutdown")),
    ] {
        let mut start = startup(TOKEN);
        start[field] = invalid;
        cases.push(start);
    }
    for token in [
        "",
        "short",
        &"z".repeat(64),
        &"a".repeat(63),
        &"a".repeat(65),
    ] {
        cases.push(startup(token));
    }
    for start in cases {
        let dir = TestDir::new();
        let mut server = ManagedChild::spawn(&dir.data);
        server.frame(&start);
        assert!(!server.exit(Duration::from_secs(2)).success());
        assert!(
            server.stdout.snapshot().is_empty(),
            "invalid startup produced readiness"
        );
        server.assert_private_output(TOKEN);
        if let Some(token) = start["bearer_token"]
            .as_str()
            .filter(|token| !token.is_empty())
        {
            server.assert_private_output(token);
        }
        dir.assert_no_storage();
    }
}

#[test]
fn managed_startup_rejects_oversized_truncated_and_malformed_frames() {
    let malformed = format!("{{invalid-json-{TOKEN}");
    let cases = [
        // Only a length is sent: rejecting a frame over the cap must not wait
        // for its claimed payload, or trust it enough to allocate that length.
        ((4097u32).to_be_bytes().to_vec(), false),
        ((u32::MAX).to_be_bytes().to_vec(), false),
        (vec![0, 0], true),
        ([32u32.to_be_bytes().as_slice(), b"{"].concat(), true),
        ([0u32.to_be_bytes().as_slice()].concat(), false),
        (
            [
                (malformed.len() as u32).to_be_bytes().as_slice(),
                malformed.as_bytes(),
            ]
            .concat(),
            false,
        ),
    ];
    for (bytes, eof) in cases {
        let dir = TestDir::new();
        let mut server = ManagedChild::spawn(&dir.data);
        server.bytes(&bytes);
        if eof {
            server.eof();
        }
        assert!(!server.exit(Duration::from_secs(2)).success());
        assert!(server.stdout.snapshot().is_empty());
        server.assert_private_output(TOKEN);
        dir.assert_no_storage();
    }
}

#[test]
fn managed_parent_eof_before_start_creates_no_database() {
    let dir = TestDir::new();
    let mut server = ManagedChild::spawn(&dir.data);
    server.eof();
    server.exit(Duration::from_secs(2));
    assert!(server.stdout.snapshot().is_empty());
    dir.assert_no_storage();
}

#[test]
fn managed_parent_that_never_sends_start_is_bounded() {
    let dir = TestDir::new();
    let mut server = ManagedChild::spawn(&dir.data);
    let started = Instant::now();
    thread::sleep(Duration::from_millis(100));
    assert!(server.child.try_wait().unwrap().is_none());
    assert!(server.stdout.snapshot().is_empty());
    dir.assert_no_storage();
    server.assert_no_listener();
    assert!(!server.exit(Duration::from_secs(7)).success());
    assert!(
        started.elapsed() >= Duration::from_secs(4),
        "child rejected the managed flag instead of waiting for startup"
    );
    assert!(server.stdout.snapshot().is_empty());
    dir.assert_no_storage();
}

// A bounded two-request origin: hold an HTTP transfer open after its first
// chunk, then serve a validator-checked Range request after child recovery.
struct StalledOrigin {
    source: String,
    first_chunk: Receiver<()>,
    release: Sender<()>,
    resumed: Receiver<String>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl StalledOrigin {
    fn new(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let source = format!("http://{}/archive.zip", listener.local_addr().unwrap());
        let (chunk_tx, first_chunk) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let (resumed_tx, resumed) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut first = accept_until(&listener, Duration::from_secs(5))?;
            let _ = read_headers(&mut first)?;
            let split = body.len() / 2;
            write!(
                first,
                "HTTP/1.1 200 OK\r\nETag: \"managed-v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            first.write_all(&body[..split])?;
            first.flush()?;
            let _ = chunk_tx.send(());
            if release_rx.recv_timeout(Duration::from_secs(10)).is_err() {
                return Ok(());
            }
            drop(first);
            let mut second = accept_until(&listener, Duration::from_secs(5))?;
            let request = read_headers(&mut second)?;
            let _ = resumed_tx.send(request);
            write!(
                second,
                "HTTP/1.1 206 Partial Content\r\nETag: \"managed-v1\"\r\nContent-Range: bytes {split}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len() - 1,
                body.len(),
                body.len() - split
            )?;
            second.write_all(&body[split..])?;
            Ok(())
        });
        Self {
            source,
            first_chunk,
            release,
            resumed,
            worker: Some(worker),
        }
    }

    fn finish(mut self) {
        self.worker.take().unwrap().join().unwrap().unwrap();
    }
}

impl Drop for StalledOrigin {
    fn drop(&mut self) {
        let _ = self.release.send(());
        // The worker has socket/channel deadlines, including when a test fails
        // before the second request. Do not make unwinding wait for that request.
    }
}

fn accept_until(listener: &TcpListener, timeout: Duration) -> io::Result<TcpStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // The fixture also uses a nonblocking listener and blocking
                // per-connection reads, so reset macOS's inherited socket flag.
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                return Ok(stream);
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_headers(stream: &mut TcpStream) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 8192 {
            return Err(io::Error::other("origin request headers exceeded cap"));
        }
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).map_err(io::Error::other)
}

fn stalled_download(dir: &TestDir, origin: &StalledOrigin) -> (ManagedChild, PathBuf, PathBuf) {
    let server = ManagedChild::start(&dir.data);
    let destination = dir.root.join("archive.zip");
    let partial = dir.root.join(".archive.zip.nexum.part");
    server.call(
        "task.create",
        Some(json!({
            "id": "stalled", "source": origin.source, "destination": destination.to_string_lossy()
        })),
    );
    server.call("task.queue", Some(json!({"id": "stalled"})));
    origin
        .first_chunk
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let progress = server.call("task.get", Some(json!({"id": "stalled"})));
        if progress["downloaded_bytes"].as_u64().unwrap() > 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "first HTTP chunk was never recorded"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(partial.exists());
    assert!(nexum_core::nexum_engine::resumable_metadata_path(&partial).exists());
    (server, destination, partial)
}

#[test]
fn managed_eof_during_stalled_http_preserves_partial_and_resumes_on_restart() {
    let dir = TestDir::new();
    let body = [vec![b'a'; 32 * 1024], vec![b'b'; 32 * 1024]].concat();
    let origin = StalledOrigin::new(body.clone());
    let (mut server, destination, partial) = stalled_download(&dir, &origin);
    server.eof();
    assert!(server.exit(Duration::from_secs(2)).success());
    server.assert_private_output(TOKEN);
    assert!(
        !destination.exists(),
        "shutdown exposed an incomplete final file"
    );
    assert_eq!(fs::read(&partial).unwrap(), body[..32 * 1024]);
    origin.release.send(()).unwrap();
    let mut restarted = ManagedChild::start(&dir.data);
    let request = origin
        .resumed
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .to_ascii_lowercase();
    assert!(request.contains("range: bytes=32768-"), "{request}");
    assert!(request.contains("if-range: \"managed-v1\""), "{request}");
    restarted.wait_task("stalled", "Completed");
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!partial.exists());
    assert!(!nexum_core::nexum_engine::resumable_metadata_path(&partial).exists());
    restarted.eof();
    assert!(restarted.exit(Duration::from_secs(2)).success());
    origin.finish();
}

#[test]
fn managed_eof_and_recovery_never_replace_an_existing_destination() {
    let dir = TestDir::new();
    let origin = StalledOrigin::new(vec![b'a'; 64 * 1024]);
    let (mut server, destination, partial) = stalled_download(&dir, &origin);
    fs::write(
        &destination,
        b"an external file created during the download",
    )
    .unwrap();
    server.eof();
    assert!(server.exit(Duration::from_secs(2)).success());
    assert!(partial.exists());
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"an external file created during the download"
    );
    origin.release.send(()).unwrap();
    let mut restarted = ManagedChild::start(&dir.data);
    restarted.wait_task("stalled", "Failed");
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"an external file created during the download"
    );
    restarted.eof();
    assert!(restarted.exit(Duration::from_secs(2)).success());
}
