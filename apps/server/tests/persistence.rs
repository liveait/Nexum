use nexum_core::nexum_domain::TaskId;
use nexum_core::nexum_storage::{SqliteRepository, TaskRepository};
use nexum_core::nexum_task::TaskState;
use nexum_protocol::Credential;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
static SERVER_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn server_test_guard() -> std::sync::MutexGuard<'static, ()> {
    SERVER_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
        let path = std::env::temp_dir().join(format!(
            "nexum-server-test-{}-{stamp}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let data = path.join("data");
        fs::create_dir(&path).unwrap();
        fs::create_dir(&data).unwrap();
        fs::create_dir(path.join("downloads")).unwrap();
        Self { root: path, data }
    }

    fn path(&self) -> &Path {
        &self.data
    }

    fn output_path(&self, name: &str) -> PathBuf {
        self.root.join("downloads").join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ServerProcess {
    child: Child,
    port: u16,
}

impl ServerProcess {
    fn start(data_dir: &Path) -> Self {
        Self::start_with_max_connections(data_dir, 100)
    }

    fn start_with_max_connections(data_dir: &Path, max_connections: usize) -> Self {
        Self::start_with_options(data_dir, max_connections, None)
    }

    fn start_with_auth(data_dir: &Path, scheme: &str, token: &str) -> Self {
        Self::start_with_options(data_dir, 100, Some((scheme, token)))
    }

    fn start_with_options(
        data_dir: &Path,
        max_connections: usize,
        auth: Option<(&str, &str)>,
    ) -> Self {
        Self::start_with_options_and_rate_limit(data_dir, max_connections, auth, None)
    }

    fn start_with_rate_limit(data_dir: &Path, auth: Option<(&str, &str)>, burst: usize) -> Self {
        // A refill takes 1,000 seconds, so normal CI scheduling cannot change
        // which request consumes the deliberately small test budget.
        Self::start_with_options_and_rate_limit(data_dir, 100, auth, Some(("0.001", burst)))
    }

    fn start_with_options_and_rate_limit(
        data_dir: &Path,
        max_connections: usize,
        auth: Option<(&str, &str)>,
        rate_limit: Option<(&str, usize)>,
    ) -> Self {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);

        let mut command = Command::new(env!("CARGO_BIN_EXE_nexum-server"));
        command
            .arg("--port")
            .arg(port.to_string())
            .arg("--max-connections")
            .arg(max_connections.to_string())
            .arg("--data-dir")
            .arg(data_dir);
        if let Some((scheme, token)) = auth {
            command
                .arg("--require-auth")
                .arg("true")
                .arg("--auth-scheme")
                .arg(scheme)
                .arg("--auth-token")
                .arg(token);
        }
        if let Some((rps, burst)) = rate_limit {
            command
                .arg("--rate-limit-rps")
                .arg(rps)
                .arg("--rate-limit-burst")
                .arg(burst.to_string());
        }
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut server = Self { child, port };
        for _ in 0..100 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return server;
            }
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!("server exited before listening: {status}");
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("server did not listen on port {port}");
    }

    fn request(&self, method: &str, params: Option<Value>) -> Value {
        self.request_with_credential(method, params, None)
    }

    fn request_with_credential(
        &self,
        method: &str,
        params: Option<Value>,
        credential: Option<Credential>,
    ) -> Value {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        if let Some(credential) = credential {
            request["credential"] = serde_json::to_value(credential).unwrap();
        }
        writeln!(stream, "{request}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        response
    }

    fn call(&self, method: &str, params: Option<Value>) -> Value {
        let response = self.request(method, params);
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    fn call_with_credential(
        &self,
        method: &str,
        params: Option<Value>,
        credential: Credential,
    ) -> Value {
        let response = self.request_with_credential(method, params, Some(credential));
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    fn http_request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (String, String, Value) {
        self.http_request_with_origin(method, path, body, None)
    }

    fn http_request_with_origin(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        origin: Option<&str>,
    ) -> (String, String, Value) {
        self.http_request_with_origin_and_content_type(
            method,
            path,
            body,
            origin,
            "application/json",
        )
    }

    fn http_request_with_origin_and_content_type(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        origin: Option<&str>,
        content_type: &str,
    ) -> (String, String, Value) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let body = body.map(|value| value.to_string()).unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: {content_type}\r\n"
        )
        .unwrap();
        if let Some(origin) = origin {
            write!(stream, "Origin: {origin}\r\n").unwrap();
        }
        write!(stream, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        stream.flush().unwrap();
        let mut encoded = String::new();
        stream.read_to_string(&mut encoded).unwrap();
        let (headers, body) = encoded.split_once("\r\n\r\n").unwrap();
        let status = headers.lines().next().unwrap().to_owned();
        let parsed: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(body).unwrap()
        };
        (status, headers.to_owned(), parsed)
    }

    fn subscribe_events(&self) -> BufReader<TcpStream> {
        let (reader, response) = self.subscribe_events_with_credential(None);
        assert_eq!(response["result"]["subscribed"], true);
        reader
    }

    fn subscribe_events_with_credential(
        &self,
        credential: Option<Credential>,
    ) -> (BufReader<TcpStream>, Value) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = json!({"jsonrpc": "2.0", "id": 1, "method": "events.subscribe"});
        if let Some(credential) = credential {
            request["credential"] = serde_json::to_value(credential).unwrap();
        }
        writeln!(stream, "{request}").unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        (reader, response)
    }

    fn task_state(&self, id: &str) -> String {
        self.call("task.get", Some(json!({"id": id})))["state"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn wait_for_state(&self, id: &str, expected: &str) {
        for _ in 0..100 {
            if self.task_state(id) == expected {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("task {id} did not reach {expected}");
    }

    fn stop(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

struct HttpFixture {
    source: String,
    connected: Receiver<()>,
    release: Sender<()>,
    worker: JoinHandle<()>,
}

impl HttpFixture {
    fn new(body: &'static [u8]) -> Self {
        Self::with_status("200 OK", body)
    }

    fn with_status(status: &'static str, body: &'static [u8]) -> Self {
        Self::with_status_and_attempts(status, body, 1)
    }

    fn with_status_and_attempts(
        status: &'static str,
        body: &'static [u8],
        attempts: usize,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/file", listener.local_addr().unwrap());
        let (connected_tx, connected) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            for attempt in 0..attempts {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut method = [0u8; 3];
                stream.read_exact(&mut method).unwrap();
                assert_eq!(&method, b"GET");
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.flush().unwrap();
                if attempt == 0 {
                    connected_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                let _ = stream.write_all(body);
            }
        });
        Self {
            source,
            connected,
            release,
            worker,
        }
    }

    fn wait_until_connected(&self) {
        self.connected.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    fn finish(self) {
        self.release.send(()).unwrap();
        self.worker.join().unwrap();
    }
}

struct ResumableHttpFixture {
    source: String,
    first_chunk_sent: Receiver<()>,
    release_first: Sender<()>,
    resumed_request: Receiver<String>,
    worker: JoinHandle<()>,
}

impl ResumableHttpFixture {
    fn new(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/file", listener.local_addr().unwrap());
        let (first_chunk_tx, first_chunk_sent) = mpsc::channel();
        let (release_first, release_first_rx) = mpsc::channel();
        let (resumed_tx, resumed_request) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(!request.to_ascii_lowercase().contains("range:"));
            let split = body.len() / 2;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nETag: \"fixture-v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body[..split]).unwrap();
            stream.flush().unwrap();
            first_chunk_tx.send(()).unwrap();
            release_first_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            let _ = stream.write_all(&body[split..]);
            let _ = stream.flush();

            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            resumed_tx.send(request.clone()).unwrap();
            let offset = body.len() / 2;
            write!(
                stream,
                "HTTP/1.1 206 Partial Content\r\nETag: \"fixture-v1\"\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                offset,
                body.len() - 1,
                body.len(),
                body.len() - offset
            )
            .unwrap();
            let _ = stream.write_all(&body[offset..]);
        });
        Self {
            source,
            first_chunk_sent,
            release_first,
            resumed_request,
            worker,
        }
    }

    fn wait_until_first_chunk(&self) {
        self.first_chunk_sent
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
    }

    fn release_first(&self) {
        self.release_first.send(()).unwrap();
    }

    fn wait_for_resume_request(&self) -> String {
        self.resumed_request
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
    }

    fn finish(self) {
        self.worker.join().unwrap();
    }
}

struct ChunkedHttpFixture {
    source: String,
    first_chunk_sent: Receiver<()>,
    release: Sender<()>,
    worker: JoinHandle<()>,
}

impl ChunkedHttpFixture {
    fn new(first: Vec<u8>, second: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/file", listener.local_addr().unwrap());
        let (first_chunk_tx, first_chunk_sent) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
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
            first_chunk_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let _ = stream.write_all(&second);
        });
        Self {
            source,
            first_chunk_sent,
            release,
            worker,
        }
    }

    fn wait_until_first_chunk(&self) {
        self.first_chunk_sent
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
    }

    fn finish(self) {
        self.release.send(()).unwrap();
        self.worker.join().unwrap();
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn tasks_survive_server_restart_and_active_states_requeue() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    for id in ["created", "queued", "downloading", "paused"] {
        server.call(
            "task.create",
            Some(json!({
                "id": id,
                "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
                "destination": dir.output_path(id).to_string_lossy(),
            })),
        );
    }
    for id in ["downloading", "paused", "queued"] {
        server.call("task.queue", Some(json!({"id": id})));
    }
    server.stop();

    let mut repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    for (id, state) in [
        ("downloading", TaskState::Downloading),
        ("paused", TaskState::Paused),
    ] {
        let mut task = repository.get(&TaskId::from(id)).unwrap().unwrap();
        task.state = state;
        repository.update(task).unwrap();
    }
    drop(repository);

    let mut restarted = ServerProcess::start(dir.path());
    assert_eq!(restarted.task_state("created"), "Created");
    for id in ["queued", "downloading", "paused"] {
        assert_eq!(restarted.task_state(id), "Queued");
    }
    restarted.stop();

    let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    for id in ["queued", "downloading", "paused"] {
        let stored = repository.get(&TaskId::from(id)).unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Queued);
    }
    drop(repository);
}

#[test]
fn queued_http_tasks_dispatch_after_server_restart() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let fixture = HttpFixture::new(b"recovered response");
    let mut server = ServerProcess::start(dir.path());
    let destination = dir.output_path("recovered-http");
    server.call(
        "task.create",
        Some(json!({
            "id": "recovered-http",
            "source": fixture.source.clone(),
            "destination": destination.to_string_lossy(),
        })),
    );
    server.stop();

    let mut repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    let mut task = repository
        .get(&TaskId::from("recovered-http"))
        .unwrap()
        .unwrap();
    task.state = TaskState::Queued;
    repository.update(task).unwrap();
    drop(repository);

    let restarted = ServerProcess::start(dir.path());
    fixture.wait_until_connected();
    assert_eq!(restarted.task_state("recovered-http"), "Downloading");
    fixture.finish();
    restarted.wait_for_state("recovered-http", "Completed");
    assert_eq!(fs::read(destination).unwrap(), b"recovered response");
}

#[test]
fn interrupted_http_download_resumes_from_validated_partial_after_restart() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let body = vec![b'a'; 32 * 1024]
        .into_iter()
        .chain(vec![b'b'; 32 * 1024])
        .collect::<Vec<_>>();
    let fixture = ResumableHttpFixture::new(body.clone());
    let destination = dir.output_path("restart-resume");
    let mut server = ServerProcess::start(dir.path());
    server.call(
        "task.create",
        Some(json!({
            "id": "restart-resume",
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": "restart-resume"})));
    fixture.wait_until_first_chunk();

    let mut observed = 0;
    for _ in 0..100 {
        observed =
            server.call("task.get", Some(json!({"id": "restart-resume"})))["downloaded_bytes"]
                .as_u64()
                .unwrap();
        if observed > 0 {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(observed > 0);
    let partial = destination
        .parent()
        .unwrap()
        .join(".restart-resume.nexum.part");
    let sidecar = nexum_core::nexum_engine::resumable_metadata_path(&partial);
    assert!(partial.exists());
    assert!(sidecar.exists());

    server.stop();
    fixture.release_first();
    let restarted = ServerProcess::start(dir.path());
    let resumed_request = fixture.wait_for_resume_request();
    let resumed_request = resumed_request.to_ascii_lowercase();
    assert!(resumed_request.contains("range: bytes=32768-"));
    assert!(resumed_request.contains("if-range: \"fixture-v1\""));
    restarted.wait_for_state("restart-resume", "Completed");
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!partial.exists());
    assert!(!sidecar.exists());
    fixture.finish();
}

#[test]
fn http_download_completes_without_blocking_other_rpc_and_releases_slots() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let body = b"Nexum HTTP download";

    for index in 0..4 {
        let fixture = HttpFixture::new(body);
        let id = format!("http-{index}");
        let destination = dir.output_path(&id);
        server.call(
            "task.create",
            Some(json!({
                "id": id,
                "source": fixture.source,
                "destination": destination.to_string_lossy(),
            })),
        );
        server.call("task.queue", Some(json!({"id": id})));
        fixture.wait_until_connected();
        assert_eq!(server.call("server.version", None), "1");
        assert_eq!(server.task_state(&id), "Downloading");
        fixture.finish();
        server.wait_for_state(&id, "Completed");
        assert_eq!(fs::read(&destination).unwrap(), body);
        let result = server.call("task.get", Some(json!({"id": id})));
        assert_eq!(result["downloaded_bytes"], body.len());
    }

    server.stop();
    let mut restarted = ServerProcess::start(dir.path());
    for index in 0..4 {
        assert_eq!(restarted.task_state(&format!("http-{index}")), "Completed");
    }
    restarted.stop();
}

#[test]
fn active_http_transfer_can_pause_and_resume() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let first = vec![b'a'; 32 * 1024];
    let second = vec![b'b'; 32 * 1024];
    let fixture = ChunkedHttpFixture::new(first, second);
    let destination = dir.output_path("paused-http");
    server.call(
        "task.create",
        Some(json!({
            "id": "paused-http",
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": "paused-http"})));
    fixture.wait_until_first_chunk();

    assert_eq!(
        server.call("task.pause", Some(json!({"id": "paused-http"}))),
        true
    );
    server.wait_for_state("paused-http", "Paused");
    assert!(!destination.exists());

    assert_eq!(
        server.call("task.resume", Some(json!({"id": "paused-http"}))),
        true
    );
    fixture.finish();
    server.wait_for_state("paused-http", "Completed");
    let mut expected = vec![b'a'; 32 * 1024];
    expected.extend(vec![b'b'; 32 * 1024]);
    assert_eq!(fs::read(&destination).unwrap(), expected);
    server.stop();
}

#[test]
fn active_http_remove_cancels_worker_and_preserves_destination() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let first = vec![b'a'; 32 * 1024];
    let second = vec![b'b'; 32 * 1024];
    let fixture = ChunkedHttpFixture::new(first, second);
    let destination = dir.output_path("removed-http");
    fs::write(&destination, b"old bytes").unwrap();
    server.call(
        "task.create",
        Some(json!({
            "id": "removed-http",
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": "removed-http"})));
    fixture.wait_until_first_chunk();
    assert_eq!(
        server.call("task.pause", Some(json!({"id": "removed-http"}))),
        true
    );
    server.wait_for_state("removed-http", "Paused");

    assert_eq!(
        server.call("task.remove", Some(json!({"id": "removed-http"}))),
        true
    );
    assert!(
        server
            .request("task.get", Some(json!({"id": "removed-http"})))
            .get("error")
            .is_some()
    );
    fixture.finish();
    assert_eq!(fs::read(&destination).unwrap(), b"old bytes");
    assert_eq!(
        fs::read_dir(destination.parent().unwrap()).unwrap().count(),
        1
    );
    server.stop();
}

#[test]
fn task_remove_cleans_stable_http_partial_and_sidecar() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let destination = dir.output_path("remove-partial");
    server.call(
        "task.create",
        Some(json!({
            "id": "remove-partial",
            "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            "destination": destination.to_string_lossy(),
        })),
    );
    let partial = destination
        .parent()
        .unwrap()
        .join(".remove-partial.nexum.part");
    let sidecar = partial.with_file_name(".remove-partial.nexum.part.json");
    fs::write(&partial, b"partial bytes").unwrap();
    fs::write(&sidecar, b"resume metadata").unwrap();
    assert_eq!(
        server.call("task.remove", Some(json!({"id": "remove-partial"}))),
        true
    );
    assert!(!partial.exists());
    assert!(!sidecar.exists());
    server.stop();
}

#[test]
fn queued_http_tasks_dispatch_without_explicit_start() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let first = HttpFixture::new(b"first response");
    let second = HttpFixture::new(b"second response");

    for (id, source) in [("first", &first.source), ("second", &second.source)] {
        server.call(
            "task.create",
            Some(json!({
                "id": id,
                "source": source,
                "destination": dir.output_path(id).to_string_lossy(),
            })),
        );
        server.call("task.queue", Some(json!({"id": id})));
    }

    first.wait_until_connected();
    second.wait_until_connected();
    first.finish();
    second.finish();
    server.wait_for_state("first", "Completed");
    server.wait_for_state("second", "Completed");
    assert_eq!(
        fs::read(dir.output_path("first")).unwrap(),
        b"first response"
    );
    assert_eq!(
        fs::read(dir.output_path("second")).unwrap(),
        b"second response"
    );
    server.stop();
}

#[test]
fn http_download_persists_incremental_progress_for_rpc_reads() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let first = vec![b'a'; 32 * 1024];
    let second = vec![b'b'; 32 * 1024];
    let fixture = ChunkedHttpFixture::new(first, second);
    let id = "progress-http";
    let destination = dir.output_path(id);
    server.call(
        "task.create",
        Some(json!({
            "id": id,
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": id})));
    fixture.wait_until_first_chunk();

    let mut observed = 0;
    for _ in 0..100 {
        let task = server.call("task.get", Some(json!({"id": id})));
        observed = task["downloaded_bytes"].as_u64().unwrap();
        if observed > 0 {
            assert_eq!(task["state"], "Downloading");
            assert!(observed < 64 * 1024);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(observed > 0, "no incremental progress was persisted");

    fixture.finish();
    server.wait_for_state(id, "Completed");
    assert_eq!(fs::read(&destination).unwrap().len(), 64 * 1024);
    server.stop();
}

#[test]
fn unsupported_sources_stay_queued_and_http_errors_are_retried() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    server.call(
        "task.create",
        Some(json!({
            "id": "magnet",
            "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            "destination": dir.output_path("magnet").to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": "magnet"})));
    assert!(server.request("task.start", None).get("error").is_some());
    assert_eq!(server.task_state("magnet"), "Queued");

    let fixture =
        HttpFixture::with_status_and_attempts("503 Service Unavailable", b"retry later", 4);
    let destination = dir.output_path("failed-http");
    server.call(
        "task.create",
        Some(json!({
            "id": "failed-http",
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": "failed-http"})));
    assert_eq!(server.task_state("magnet"), "Queued");
    fixture.wait_until_connected();
    fixture.finish();
    server.wait_for_state("failed-http", "Failed");
    assert!(!destination.exists());
    let failed = server.call("task.get", Some(json!({"id": "failed-http"})));
    let error = failed["error"]
        .as_str()
        .filter(|message| !message.is_empty())
        .expect("failed HTTP task should persist an error");
    server.call("task.remove", Some(json!({"id": "magnet"})));
    server.stop();

    let mut restarted = ServerProcess::start(dir.path());
    let failed = restarted.call("task.get", Some(json!({"id": "failed-http"})));
    assert_eq!(failed["state"], "Failed");
    assert_eq!(failed["error"].as_str(), Some(error));
    restarted.stop();
}

#[test]
fn http_download_cannot_replace_server_database_or_lock() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    for file in ["nexum.sqlite", "nexum.lock"] {
        let id = format!("protected-{file}");
        server.call(
            "task.create",
            Some(json!({
                "id": id,
                "source": "http://127.0.0.1:9/file",
                "destination": dir.path().join(file).to_string_lossy(),
            })),
        );
        server.call("task.queue", Some(json!({"id": id})));
        assert!(server.request("task.start", None).get("error").is_some());
        assert_eq!(server.task_state(&id), "Queued");
        server.call("task.remove", Some(json!({"id": id})));
    }
    assert_eq!(server.call("server.version", None), "1");
    server.stop();

    let mut restarted = ServerProcess::start(dir.path());
    assert_eq!(restarted.call("server.version", None), "1");
    restarted.stop();
}

#[test]
fn http_downloads_to_the_same_destination_do_not_overlap() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let first = HttpFixture::new(b"first response");
    let second = HttpFixture::new(b"second response");
    let destination = dir.output_path("shared");
    for (id, source) in [("first", &first.source), ("second", &second.source)] {
        server.call(
            "task.create",
            Some(json!({
                "id": id,
                "source": source,
                "destination": destination.to_string_lossy(),
            })),
        );
        server.call("task.queue", Some(json!({"id": id})));
    }

    first.wait_until_connected();
    assert!(server.request("task.start", None).get("error").is_some());
    assert_eq!(server.task_state("second"), "Queued");
    first.finish();
    server.wait_for_state("first", "Completed");
    assert_eq!(fs::read(&destination).unwrap(), b"first response");

    second.wait_until_connected();
    second.finish();
    server.wait_for_state("second", "Completed");
    assert_eq!(fs::read(&destination).unwrap(), b"second response");
    server.stop();
}

fn server_output_with_timeout(data_dir: &Path, port: Option<u16>) -> Output {
    server_output_with_args(data_dir, port, &[])
}

fn server_output_with_args(data_dir: &Path, port: Option<u16>, extra_args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexum-server"));
    command.arg("--data-dir").arg(data_dir);
    if let Some(port) = port {
        command.arg("--port").arg(port.to_string());
    }
    command.args(extra_args);
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("server did not exit within five seconds");
}

#[test]
fn second_server_cannot_recover_a_live_servers_tasks() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let fixture = HttpFixture::new(b"slow download");
    let mut first = ServerProcess::start(dir.path());
    first.call(
        "task.create",
        Some(json!({
            "id": "running",
            "source": fixture.source,
            "destination": dir.output_path("running").to_string_lossy(),
        })),
    );
    first.call("task.queue", Some(json!({"id": "running"})));
    fixture.wait_until_connected();
    assert_eq!(first.task_state("running"), "Downloading");

    let output = server_output_with_timeout(dir.path(), Some(0));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot acquire data directory lock"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(first.task_state("running"), "Downloading");
    let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    assert_eq!(
        repository
            .get(&TaskId::from("running"))
            .unwrap()
            .unwrap()
            .state,
        TaskState::Downloading
    );
    drop(repository);

    first.stop();
    fixture.finish();
    let mut restarted = ServerProcess::start(dir.path());
    assert_eq!(restarted.task_state("running"), "Failed");
    restarted.stop();
}

#[test]
fn startup_fails_when_data_dir_is_a_file() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let invalid_dir = dir.path().join("not-a-directory");
    fs::write(&invalid_dir, "occupied").unwrap();
    let output = server_output_with_timeout(&invalid_dir, None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot create data directory"));
}

#[test]
fn startup_fails_when_database_cannot_be_opened() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    fs::write(dir.path().join("nexum.sqlite"), "not a SQLite database").unwrap();
    let output = server_output_with_timeout(dir.path(), None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nexum.sqlite"));
}

fn assert_unauthorized(response: &Value) {
    assert_eq!(response["error"]["code"], -32001);
    assert_eq!(response["error"]["message"], "authentication required");
}

fn assert_rate_limited(response: &Value) {
    assert_eq!(response["error"]["code"], -32002, "{response}");
    assert!(response.get("result").is_none(), "{response}");
}

#[test]
fn require_auth_rejects_missing_and_invalid_credentials() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_auth(dir.path(), "ApiKey", "test-secret");

    assert_unauthorized(&server.request("server.version", None));
    assert_unauthorized(&server.request_with_credential(
        "server.version",
        None,
        Some(Credential::ApiKey {
            key: "wrong-secret".to_owned(),
        }),
    ));
    assert_unauthorized(&server.request_with_credential(
        "server.version",
        None,
        Some(Credential::Bearer {
            token: "test-secret".to_owned(),
        }),
    ));

    let credential = Credential::ApiKey {
        key: "test-secret".to_owned(),
    };
    assert_eq!(
        server.call_with_credential("server.version", None, credential.clone()),
        "1"
    );
    assert_eq!(
        server.call_with_credential("server.auth", None, credential),
        json!(["ApiKey"])
    );
    server.stop();
}

#[test]
fn http_jsonrpc_bridge_supports_cors_and_task_queueing() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());

    let (status, headers, options_body) = server.http_request("OPTIONS", "/jsonrpc", None);
    assert_eq!(status, "HTTP/1.1 204 No Content");
    assert!(!headers.contains("Access-Control-Allow-Origin"));
    assert_eq!(options_body, Value::Null);

    let (status, headers, _) = server.http_request_with_origin(
        "OPTIONS",
        "/jsonrpc",
        None,
        Some("chrome-extension://test-id"),
    );
    assert_eq!(status, "HTTP/1.1 204 No Content");
    assert!(headers.contains("Access-Control-Allow-Origin: chrome-extension://test-id"));
    assert!(headers.contains("Vary: Origin"));
    assert!(headers.contains("Access-Control-Allow-Methods: POST, OPTIONS"));
    assert!(headers.contains("Access-Control-Allow-Headers: Content-Type"));

    let (status, headers, _) = server.http_request_with_origin(
        "OPTIONS",
        "/jsonrpc",
        None,
        Some("moz-extension://test-uuid"),
    );
    assert_eq!(status, "HTTP/1.1 204 No Content");
    assert!(headers.contains("Access-Control-Allow-Origin: moz-extension://test-uuid"));

    let task = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "task.create",
        "params": {
            "id": "browser-http",
            "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            "destination": dir.output_path("browser-http").to_string_lossy(),
        }
    });
    let (status, headers, response) = server.http_request("POST", "/jsonrpc", Some(task));
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(!headers.contains("Access-Control-Allow-Origin"));
    assert_eq!(response["result"]["id"], "browser-http");

    let (status, headers, response) = server.http_request_with_origin(
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 3, "method": "server.version"})),
        Some("chrome-extension://test-id"),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(headers.contains("Access-Control-Allow-Origin: chrome-extension://test-id"));
    assert_eq!(response["result"], "1");

    let (status, _, response) = server.http_request_with_origin_and_content_type(
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 4, "method": "server.version"})),
        None,
        "text/plain",
    );
    assert_eq!(status, "HTTP/1.1 415 Unsupported Media Type");
    assert_eq!(
        response["error"],
        "Content-Type application/json is required"
    );

    let queue = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "task.queue",
        "params": {"id": "browser-http"}
    });
    let (_, _, response) = server.http_request("POST", "/jsonrpc", Some(queue));
    assert_eq!(response["result"], true);
    assert_eq!(server.task_state("browser-http"), "Queued");

    let (status, _, response) = server.http_request("POST", "/unknown", Some(json!({})));
    assert_eq!(status, "HTTP/1.1 404 Not Found");
    assert_eq!(response["error"], "unknown HTTP endpoint");
    server.stop();
}

#[test]
fn http_jsonrpc_bridge_rejects_web_page_origins_before_dispatch() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let task = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "task.create",
        "params": {
            "id": "web-origin-task",
            "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            "destination": dir.output_path("web-origin-task").to_string_lossy(),
        }
    });
    let (status, headers, _) = server.http_request_with_origin(
        "POST",
        "/jsonrpc",
        Some(task),
        Some("https://evil.example"),
    );
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    assert!(!headers.contains("Access-Control-Allow-Origin"));
    assert_eq!(server.call("task.list", None), json!([]));

    let (status, headers, _) =
        server.http_request_with_origin("OPTIONS", "/jsonrpc", None, Some("https://evil.example"));
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    assert!(!headers.contains("Access-Control-Allow-Origin"));
    server.stop();
}

#[test]
fn http_jsonrpc_bridge_uses_the_same_authentication_gate() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_auth(dir.path(), "ApiKey", "http-secret");
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server.version"
    });
    let (_, _, response) = server.http_request("POST", "/jsonrpc", Some(request.clone()));
    assert_unauthorized(&response);

    let mut authenticated = request;
    authenticated["credential"] = json!({"ApiKey": {"key": "http-secret"}});
    let (_, _, response) = server.http_request("POST", "/jsonrpc", Some(authenticated));
    assert_eq!(response["result"], "1");
    server.stop();
}

#[test]
fn require_auth_applies_to_event_subscriptions() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_auth(dir.path(), "Bearer", "event-secret");

    let (rejected_events, rejected) = server.subscribe_events_with_credential(None);
    assert_unauthorized(&rejected);
    drop(rejected_events);

    let credential = Credential::Bearer {
        token: "event-secret".to_owned(),
    };
    let (mut events, subscribed) =
        server.subscribe_events_with_credential(Some(credential.clone()));
    assert_eq!(subscribed["result"]["subscribed"], true);

    server.call_with_credential(
        "task.create",
        Some(json!({
            "id": "authenticated-event",
            "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
            "destination": dir.output_path("authenticated-event").to_string_lossy(),
        })),
        credential.clone(),
    );
    server.call_with_credential(
        "task.queue",
        Some(json!({"id": "authenticated-event"})),
        credential,
    );

    let mut observed = Vec::new();
    for _ in 0..12 {
        let mut line = String::new();
        events.read_line(&mut line).unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = serde_json::from_str(&line).unwrap();
        if message["method"] == "events.event" {
            observed.push(message["params"]["event"].as_str().unwrap().to_owned());
        }
        if observed.iter().any(|event| event == "task.created") {
            break;
        }
    }
    assert!(
        observed.iter().any(|event| event == "task.created"),
        "authenticated event subscription did not receive task.created: {observed:?}"
    );
    drop(events);
    server.stop();
}

#[test]
fn rate_limit_is_disabled_by_default_for_tcp_http_and_subscriptions() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());

    for _ in 0..8 {
        assert_eq!(server.call("server.version", None), "1");
        let (status, _, response) = server.http_request(
            "POST",
            "/jsonrpc",
            Some(json!({"jsonrpc": "2.0", "id": 1, "method": "server.version"})),
        );
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(response["result"], "1");
    }
    let first = server.subscribe_events();
    let second = server.subscribe_events();
    drop((first, second));
    server.stop();
}

#[test]
fn rate_limit_budget_is_shared_across_tcp_connections_and_http() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_rate_limit(dir.path(), None, 2);

    let (status, _, body) = server.http_request("OPTIONS", "/jsonrpc", None);
    assert_eq!(status, "HTTP/1.1 204 No Content");
    assert_eq!(body, Value::Null);

    assert_eq!(server.call("server.version", None), "1");
    let (status, _, response) = server.http_request(
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 2, "method": "server.version"})),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(response["result"], "1");

    // request() opens a new TCP connection for each call. Both transports now
    // see the exhausted process-level budget.
    assert_rate_limited(&server.request("server.version", None));
    let (status, _, response) = server.http_request(
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 3, "method": "server.version"})),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_rate_limited(&response);
    server.stop();
}

#[test]
fn tcp_notifications_stay_silent_when_accepted_or_rate_limited() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_rate_limit(dir.path(), None, 1);
    let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let notification = json!({"jsonrpc": "2.0", "method": "server.version"});
    writeln!(stream, "{notification}").unwrap();
    writeln!(stream, "{notification}").unwrap();
    writeln!(
        stream,
        "{}",
        json!({"jsonrpc": "2.0", "id": 77, "method": "server.version"})
    )
    .unwrap();

    // The first notification consumes the only token. Neither it nor the
    // rejected notification can write a response ahead of the numbered call.
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 77);
    assert_rate_limited(&response);

    let (status, _, response) = server.http_request(
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 78, "method": "server.version"})),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_rate_limited(&response);
    server.stop();
}

#[test]
fn http_notifications_return_no_content_when_accepted_or_rate_limited() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_rate_limit(dir.path(), None, 1);
    let notification = json!({"jsonrpc": "2.0", "method": "server.version"});

    let (status, _, response) = server.http_request("POST", "/jsonrpc", Some(notification.clone()));
    assert_eq!(status, "HTTP/1.1 204 No Content");
    assert_eq!(response, Value::Null);

    // HTTP notification spent the shared token, so both transports reject
    // subsequent calls while another over-limit notification remains silent.
    assert_rate_limited(&server.request("server.version", None));
    let (status, _, response) = server.http_request("POST", "/jsonrpc", Some(notification));
    assert_eq!(status, "HTTP/1.1 204 No Content");
    assert_eq!(response, Value::Null);
    let (status, _, response) = server.http_request(
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 79, "method": "server.version"})),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_rate_limited(&response);
    server.stop();
}

#[test]
fn failed_authentication_does_not_spend_rate_limit_budget() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server =
        ServerProcess::start_with_rate_limit(dir.path(), Some(("ApiKey", "rate-secret")), 1);

    let mut malformed_stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    malformed_stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    writeln!(malformed_stream, "{{invalid json").unwrap();
    let mut malformed_response = String::new();
    BufReader::new(malformed_stream)
        .read_line(&mut malformed_response)
        .unwrap();
    let malformed_response: Value = serde_json::from_str(&malformed_response).unwrap();
    assert_eq!(malformed_response["error"]["code"], -32700);

    assert_unauthorized(&server.request("server.version", None));
    assert_unauthorized(&server.request_with_credential(
        "server.version",
        None,
        Some(Credential::ApiKey {
            key: "wrong-secret".to_owned(),
        }),
    ));
    let (status, _, response) = server.http_request(
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "server.version"})),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_unauthorized(&response);

    let credential = Credential::ApiKey {
        key: "rate-secret".to_owned(),
    };
    assert_eq!(
        server.call_with_credential("server.version", None, credential.clone()),
        "1"
    );
    assert_rate_limited(&server.request_with_credential("server.version", None, Some(credential)));
    server.stop();
}

#[test]
fn event_heartbeat_does_not_spend_budget_and_excess_subscription_is_rejected() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_rate_limit(dir.path(), None, 2);
    let mut events = server.subscribe_events();
    events
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(25)))
        .unwrap();

    let mut line = String::new();
    events.read_line(&mut line).unwrap();
    let heartbeat: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(heartbeat["method"], "events.event");
    assert_eq!(heartbeat["params"]["event"], "events.heartbeat");

    assert_eq!(server.call("server.version", None), "1");
    let (rejected_events, rejected) = server.subscribe_events_with_credential(None);
    assert_rate_limited(&rejected);
    drop(rejected_events);
    drop(events);
    server.stop();
}

#[test]
fn rate_limit_arguments_must_be_paired_and_positive() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    for args in [
        &["--rate-limit-rps", "1"][..],
        &["--rate-limit-burst", "1"],
        &["--rate-limit-rps", "0", "--rate-limit-burst", "1"],
        &["--rate-limit-rps", "-1", "--rate-limit-burst", "1"],
        &["--rate-limit-rps", "NaN", "--rate-limit-burst", "1"],
        &["--rate-limit-rps", "inf", "--rate-limit-burst", "1"],
        &["--rate-limit-rps", "1", "--rate-limit-burst", "0"],
    ] {
        let output = server_output_with_args(dir.path(), None, args);
        assert!(!output.status.success(), "server accepted {args:?}");
    }
}

#[test]
fn startup_fails_when_authentication_is_required_without_credentials() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let output = server_output_with_args(dir.path(), None, &["--require-auth", "true"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("require_auth=true requires auth_scheme and a non-empty auth_token"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn event_subscription_broadcasts_task_changes() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let mut events = server.subscribe_events();

    server.call(
        "task.create",
        Some(json!({
            "id": "streamed",
            "source": "https://example.com/file",
            "destination": dir.output_path("streamed").to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": "streamed"})));

    let mut observed = Vec::new();
    for _ in 0..12 {
        let mut line = String::new();
        events.read_line(&mut line).unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = serde_json::from_str(&line).unwrap();
        if message["method"] == "events.event" {
            observed.push(message["params"]["event"].as_str().unwrap().to_owned());
        }
        if observed.iter().any(|event| event == "task.created")
            && observed.iter().any(|event| event == "scheduler.enqueued")
        {
            break;
        }
    }

    assert!(observed.iter().any(|event| event == "task.created"));
    assert!(observed.iter().any(|event| event == "task.state_changed"));
    assert!(observed.iter().any(|event| event == "scheduler.enqueued"));
    server.stop();
}

#[test]
fn max_connections_rejects_excess_connections_and_releases_after_close() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start_with_max_connections(dir.path(), 1);

    let mut first = None;
    for _ in 0..20 {
        let Ok(mut candidate) = TcpStream::connect(("127.0.0.1", server.port)) else {
            thread::sleep(Duration::from_millis(25));
            continue;
        };
        candidate
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let _ = writeln!(
            candidate,
            "{}",
            json!({"jsonrpc": "2.0", "id": 1, "method": "server.version"})
        );
        let mut first_line = String::new();
        if BufReader::new(candidate.try_clone().unwrap())
            .read_line(&mut first_line)
            .is_ok()
            && first_line.contains("\"result\":\"1\"")
        {
            first = Some(candidate);
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let first = first.expect("first connection was not admitted");

    let mut second = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    second
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let _ = writeln!(
        second,
        "{}",
        json!({"jsonrpc": "2.0", "id": 2, "method": "server.version"})
    );
    let mut rejected = [0_u8; 1];
    let second_result = second.read(&mut rejected);
    match second_result {
        Ok(0) => {}
        Ok(bytes) => panic!("rejected connection returned {bytes} bytes"),
        Err(error) => assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::UnexpectedEof
            ),
            "unexpected rejection error: {error}"
        ),
    }

    drop(first);
    let mut released = false;
    for _ in 0..20 {
        let mut third = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        third
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let _ = writeln!(
            third,
            "{}",
            json!({"jsonrpc": "2.0", "id": 3, "method": "server.version"})
        );
        let mut line = String::new();
        if BufReader::new(third).read_line(&mut line).is_ok() && line.contains("\"result\":\"1\"") {
            released = true;
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        released,
        "connection slot was not released after the first client closed"
    );
    server.stop();
}
