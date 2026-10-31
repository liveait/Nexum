use nexum_core::nexum_domain::TaskId;
use nexum_core::nexum_domain::{Destination, DownloadSource, Progress};
use nexum_core::nexum_engine::{
    CompletionContext, HttpEngine, HttpTransferControl, completion_receipt_path,
};
use nexum_core::nexum_storage::{SqliteRepository, TaskRepository};
use nexum_core::nexum_task::{DownloadTask, TaskState};
use nexum_protocol::Credential;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

    fn start_with_tls(data_dir: &Path, certificate_dir: &Path) -> (Self, Vec<u8>) {
        Self::start_with_tls_and_auth(data_dir, certificate_dir, None)
    }

    fn start_with_tls_and_auth(
        data_dir: &Path,
        certificate_dir: &Path,
        auth: Option<(&str, &str)>,
    ) -> (Self, Vec<u8>) {
        let generated = generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let certificate_path = certificate_dir.join("server-cert.pem");
        let key_path = certificate_dir.join("server-key.pem");
        fs::write(&certificate_path, generated.cert.pem()).unwrap();
        fs::write(&key_path, generated.key_pair.serialize_pem()).unwrap();
        let certificate_der = generated.cert.der().to_vec();
        let server = Self::start_with_options_and_rate_limit_and_tls(
            data_dir,
            100,
            auth,
            None,
            Some((&certificate_path, &key_path)),
        );
        (server, certificate_der)
    }

    fn start_with_options_and_rate_limit(
        data_dir: &Path,
        max_connections: usize,
        auth: Option<(&str, &str)>,
        rate_limit: Option<(&str, usize)>,
    ) -> Self {
        Self::start_with_options_and_rate_limit_and_tls(
            data_dir,
            max_connections,
            auth,
            rate_limit,
            None,
        )
    }

    fn start_with_options_and_rate_limit_and_tls(
        data_dir: &Path,
        max_connections: usize,
        auth: Option<(&str, &str)>,
        rate_limit: Option<(&str, usize)>,
        tls: Option<(&Path, &Path)>,
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
        if let Some((certificate_path, key_path)) = tls {
            command
                .arg("--tls-cert-path")
                .arg(certificate_path)
                .arg("--tls-key-path")
                .arg(key_path);
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

    /// Starts the server with a fixed base set of arguments plus `extra_args`,
    /// for exercising flags that are not part of the common option chain (e.g.
    /// `--bandwidth`).
    fn start_with_extra_args(data_dir: &Path, extra_args: &[&str]) -> Self {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let mut command = Command::new(env!("CARGO_BIN_EXE_nexum-server"));
        command
            .arg("--port")
            .arg(port.to_string())
            .arg("--data-dir")
            .arg(data_dir);
        for arg in extra_args {
            command.arg(arg);
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

    /// A plain `task.*` call with an explicit read timeout, for handlers that
    /// legitimately block (e.g. a pause that waits out a stalled read).
    fn request_with_timeout(
        &self,
        method: &str,
        params: Option<Value>,
        read_timeout: Duration,
    ) -> Value {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.set_read_timeout(Some(read_timeout)).unwrap();
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        writeln!(stream, "{request}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn call_with_timeout(
        &self,
        method: &str,
        params: Option<Value>,
        read_timeout: Duration,
    ) -> Value {
        let response = self.request_with_timeout(method, params, read_timeout);
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

type TlsClientStream = StreamOwned<ClientConnection, TcpStream>;

fn tls_client_stream(port: u16, certificate_der: &[u8]) -> TlsClientStream {
    try_tls_client_stream(port, certificate_der, "localhost").unwrap()
}

fn try_tls_client_stream(
    port: u16,
    certificate_der: &[u8],
    server_name: &str,
) -> std::io::Result<TlsClientStream> {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(certificate_der.to_vec()))
        .unwrap();
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server_name = ServerName::try_from(server_name.to_owned()).unwrap();
    let connection = ClientConnection::new(Arc::new(config), server_name).unwrap();
    let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut stream = StreamOwned::new(connection, socket);
    stream.conn.complete_io(&mut stream.sock)?;
    Ok(stream)
}

fn tls_rpc_request(port: u16, certificate_der: &[u8], request: Value) -> Value {
    let mut stream = tls_client_stream(port, certificate_der);
    writeln!(stream, "{request}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn tls_http_request(
    port: u16,
    certificate_der: &[u8],
    method: &str,
    path: &str,
    body: Option<Value>,
    origin: Option<&str>,
) -> (String, String, Value) {
    let mut stream = tls_client_stream(port, certificate_der);
    let body = body.map(|value| value.to_string()).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\n"
    )
    .unwrap();
    if let Some(origin) = origin {
        write!(stream, "Origin: {origin}\r\n").unwrap();
    }
    write!(stream, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    stream.flush().unwrap();
    let mut encoded_bytes = Vec::new();
    match stream.read_to_end(&mut encoded_bytes) {
        Ok(_) => {}
        // The server closes the TLS socket without sending close_notify after
        // one-shot HTTP responses. The response bytes are still complete.
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {}
        Err(error) => panic!("could not read TLS HTTP response: {error}"),
    }
    let encoded = String::from_utf8(encoded_bytes).unwrap();
    let (headers, body) = encoded.split_once("\r\n\r\n").unwrap();
    let status = headers.lines().next().unwrap().to_owned();
    let parsed = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).unwrap()
    };
    (status, headers.to_owned(), parsed)
}

fn plaintext_probe_tls_port(port: u16) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .write_all(
            b"POST /jsonrpc HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    response
}

struct HttpFixture {
    source: String,
    connected: Receiver<()>,
    release: Sender<()>,
    worker: JoinHandle<()>,
}

struct CompletionFixture {
    source: String,
    destination: PathBuf,
    partial: PathBuf,
    context: CompletionContext,
    progress: Progress,
    body: &'static [u8],
}

impl CompletionFixture {
    // Create the actual engine receipt from a response with no validator and
    // no Content-Length. The HTTP listener exits before Server is started, so
    // successful recovery proves it did not issue another network request.
    fn published(dir: &TestDir, id: &str, body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/complete", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream.write_all(body).unwrap();
        });
        let destination = fs::canonicalize(dir.output_path(id).parent().unwrap())
            .unwrap()
            .join(id);
        let partial = destination.with_file_name(format!(".{id}.nexum.part"));
        let context = CompletionContext::new(id, dir.path());
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
        worker.join().unwrap();
        assert_eq!(progress.total_bytes, None);
        assert!(completion_receipt_path(&context).exists());
        Self {
            source,
            destination,
            partial,
            context,
            progress,
            body,
        }
    }

    /// Produce a real v1 receipt using the new segment consumer, with every
    /// HTTP request served before this fixture returns. No hand-written proof.
    fn segmented(dir: &TestDir, id: &str, body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/segmented", listener.local_addr().unwrap());
        let segments = HttpEngine::split_segments(body.len() as u64, 3);
        let count = segments.len();
        let worker = thread::spawn(move || {
            for _ in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                let range = request
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("range").then_some(value.trim())
                    })
                    .unwrap();
                let (start, end) = range
                    .strip_prefix("bytes=")
                    .unwrap()
                    .split_once('-')
                    .unwrap();
                let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
                write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nETag: \"segmented-recovery\"\r\nConnection: close\r\n\r\n", body.len(), end-start+1).unwrap();
                stream.write_all(&body[start..=end]).unwrap();
            }
        });
        let destination = fs::canonicalize(dir.output_path(id).parent().unwrap())
            .unwrap()
            .join(id);
        let partial = destination.with_file_name(format!(".{id}.nexum.part"));
        let context = CompletionContext::new(id, dir.path());
        fs::write(
            nexum_core::nexum_engine::resumable_metadata_path(&partial),
            b"unowned sidecar",
        )
        .unwrap();
        let progress = HttpEngine::new()
            .download_to_segments_with_completion(
                &source,
                &segments,
                destination.to_str().unwrap(),
                &partial,
                body.len() as u64,
                Some(&nexum_core::nexum_engine::HttpResumeValidator::Etag(
                    "\"segmented-recovery\"".into(),
                )),
                &context,
                &HttpTransferControl::new(),
                |_| Ok(()),
            )
            .unwrap();
        worker.join().unwrap();
        Self {
            source,
            destination,
            partial,
            context,
            progress,
            body,
        }
    }

    fn seed_task(&self, dir: &TestDir, id: &str, state: TaskState) {
        let mut repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
        let mut task = DownloadTask::new(
            id,
            DownloadSource::new(&self.source),
            Destination::new(self.destination.to_string_lossy()),
        );
        task.state = state;
        task.progress = self.progress.clone();
        task.last_error =
            (state == TaskState::Failed).then(|| "completion save interrupted".to_owned());
        repository.insert(task.into()).unwrap();
    }
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
                // Consume the full request before closing the fixture socket;
                // unread request bytes can otherwise cause a TCP reset that
                // races delivery of this deliberately short response.
                let mut request = method.to_vec();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
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

// Advertise real Range support so a large localhost response exercises the
// same Server consumer path as a public origin, including any probe requests.
struct RangeAdvertisingHttpFixture {
    source: String,
    stop: Sender<()>,
    worker: Option<JoinHandle<Vec<String>>>,
}

impl RangeAdvertisingHttpFixture {
    fn new(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let source = format!(
            "http://localhost:{}/large",
            listener.local_addr().unwrap().port()
        );
        let (stop, stopped) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            loop {
                if stopped.try_recv().is_ok() {
                    return requests;
                }
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("could not accept HTTP fixture request: {error}"),
                };
                // macOS can inherit O_NONBLOCK from the listening socket.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                let head = request.starts_with("HEAD ");
                let range = request.lines().find_map(|line| {
                    let line = line.to_ascii_lowercase();
                    let value = line.strip_prefix("range: bytes=")?;
                    let (start, end) = value.split_once('-').unwrap();
                    Some((
                        start.parse::<usize>().unwrap(),
                        if end.is_empty() {
                            body.len() - 1
                        } else {
                            end.parse::<usize>().unwrap()
                        },
                    ))
                });
                requests.push(request);
                let (status, start, end) = match range {
                    Some((start, end)) => ("206 Partial Content", start, end),
                    None => ("200 OK", 0, body.len() - 1),
                };
                let mut response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: \"large-v1\"\r\nConnection: close\r\n",
                    end - start + 1,
                );
                if range.is_some() {
                    response.push_str(&format!(
                        "Content-Range: bytes {start}-{end}/{}\r\n",
                        body.len()
                    ));
                }
                response.push_str("\r\n");
                // A probe may drop its connection as soon as headers arrive.
                let _ = stream.write_all(response.as_bytes());
                if !head {
                    let _ = stream.write_all(&body[start..=end]);
                }
            }
        });
        Self {
            source,
            stop,
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.stop.send(()).unwrap();
        self.worker.take().unwrap().join().unwrap()
    }
}

impl Drop for RangeAdvertisingHttpFixture {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = self.stop.send(());
            let _ = worker.join();
        }
    }
}

fn large_http_body() -> Vec<u8> {
    // Distinct bytes across the former segments expose wrong offsets or
    // reordered data; the exact content is also checked against its receipt.
    (0..1536 * 1024 + 137)
        .map(|index| (((index * 31) ^ (index >> 9)) % 251) as u8)
        .collect()
}

fn assert_one_full_http_get(requests: &[String]) {
    assert_eq!(requests.len(), 1, "unexpected HTTP requests: {requests:#?}");
    assert!(requests[0].starts_with("GET /large HTTP/1.1\r\n"));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("range:"))
    );
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
fn bandwidth_config_throttles_download_and_reports_capped_speed() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    // 5000 bytes at a 1 KiB/s cap takes ~5 s of pacing.
    let body: &'static [u8] = Box::leak(vec![b't'; 5000].into_boxed_slice());
    let fixture = HttpFixture::new(body);
    let mut server =
        ServerProcess::start_with_extra_args(dir.path(), &["--bandwidth", "fixed", "1000"]);
    let destination = dir.output_path("bandwidth");
    server.call(
        "task.create",
        Some(json!({
            "id": "bandwidth",
            "source": fixture.source.clone(),
            "destination": destination.to_string_lossy(),
        })),
    );
    let started = Instant::now();
    server.call("task.queue", Some(json!({"id": "bandwidth"})));
    fixture.wait_until_connected();
    fixture.finish(); // release the body; the worker paces it to 1000 B/s

    // Poll while the paced transfer runs, tracking the reported speed.
    let mut peak_speed = 0u64;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let task = server.call("task.get", Some(json!({"id": "bandwidth"})));
        peak_speed = peak_speed.max(task["speed_bytes_per_second"].as_u64().unwrap_or(0));
        if task["state"].as_str() == Some("Completed") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "capped download did not complete in time"
        );
        thread::sleep(Duration::from_millis(50));
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_secs(3),
        "a 5000 B / 1000 B/s capped download finished in {elapsed:?}, above its budget"
    );
    assert!(
        peak_speed <= 1000 * 8,
        "reported speed {peak_speed} B/s far exceeds the 1000 B/s cap"
    );
    assert_eq!(fs::read(&destination).unwrap().len(), 5000);
    server.stop();
}

#[test]
fn shared_bandwidth_policy_splits_the_total_between_competing_tasks() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    // Two independent fixtures, served as fast as possible, so the scheduler's
    // pacing — not the server — is what limits them.
    let body_a: &'static [u8] = Box::leak(vec![b'a'; 2000].into_boxed_slice());
    let body_b: &'static [u8] = Box::leak(vec![b'b'; 2000].into_boxed_slice());
    let fixture_a = HttpFixture::new(body_a);
    let fixture_b = HttpFixture::new(body_b);
    // A shared total of 2000 B/s split across the two active tasks is 1000 B/s
    // each; 2000 bytes each takes ~2 s. Unlimited would finish well under a
    // second, so the lower bound proves the shared cap is applied to both tasks.
    let mut server =
        ServerProcess::start_with_extra_args(dir.path(), &["--bandwidth", "shared", "2000"]);
    let dest_a = dir.output_path("shared-a");
    let dest_b = dir.output_path("shared-b");
    server.call(
        "task.create",
        Some(json!({
            "id": "shared-a",
            "source": fixture_a.source.clone(),
            "destination": dest_a.to_string_lossy(),
        })),
    );
    server.call(
        "task.create",
        Some(json!({
            "id": "shared-b",
            "source": fixture_b.source.clone(),
            "destination": dest_b.to_string_lossy(),
        })),
    );
    let started = Instant::now();
    server.call("task.queue", Some(json!({"id": "shared-a"})));
    fixture_a.wait_until_connected();
    server.call("task.queue", Some(json!({"id": "shared-b"})));
    fixture_b.wait_until_connected();
    fixture_a.finish();
    fixture_b.finish();

    server.wait_for_state("shared-a", "Completed");
    server.wait_for_state("shared-b", "Completed");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(1500),
        "a 2000 B/s shared cap across two 2000 B tasks finished in {elapsed:?}; the cap was not applied to both"
    );
    assert!(
        elapsed <= Duration::from_secs(30),
        "the shared cap over-slept: {elapsed:?}"
    );
    assert_eq!(fs::read(&dest_a).unwrap().len(), 2000);
    assert_eq!(fs::read(&dest_b).unwrap().len(), 2000);
    server.stop();
}

#[test]
fn pausing_a_stalled_download_returns_within_budget() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    // Headers are sent, then the body is held, so the worker's read stalls.
    let fixture = HttpFixture::new(b"stalled body");
    let mut server = ServerProcess::start(dir.path());
    let destination = dir.output_path("stalled-pause");
    server.call(
        "task.create",
        Some(json!({
            "id": "stalled-pause",
            "source": fixture.source.clone(),
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": "stalled-pause"})));
    fixture.wait_until_connected();
    let started = Instant::now();
    let result = server.call_with_timeout(
        "task.pause",
        Some(json!({"id": "stalled-pause"})),
        Duration::from_secs(10),
    );
    let elapsed = started.elapsed();
    assert_eq!(result, json!(true), "pause should succeed");
    assert!(
        elapsed < Duration::from_secs(5),
        "task.pause took {elapsed:?} to return on a stalled read"
    );
    assert_eq!(server.task_state("stalled-pause"), "Paused");
    fixture.finish();
    server.stop();
}

#[test]
fn removing_a_stalled_download_returns_within_budget() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let fixture = HttpFixture::new(b"stalled body");
    let mut server = ServerProcess::start(dir.path());
    let destination = dir.output_path("stalled-remove");
    server.call(
        "task.create",
        Some(json!({
            "id": "stalled-remove",
            "source": fixture.source.clone(),
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": "stalled-remove"})));
    fixture.wait_until_connected();
    let started = Instant::now();
    let result = server.call_with_timeout(
        "task.remove",
        Some(json!({"id": "stalled-remove"})),
        Duration::from_secs(10),
    );
    let elapsed = started.elapsed();
    assert_eq!(result, json!(true), "remove should succeed");
    assert!(
        elapsed < Duration::from_secs(5),
        "task.remove took {elapsed:?} to return on a stalled read"
    );
    let list = server.call("task.list", None);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|task| task["id"] != "stalled-remove"),
        "a removed task must leave the task list"
    );
    assert!(
        !destination.exists(),
        "a removed download leaves no destination file"
    );
    fixture.finish();
    server.stop();
}

#[test]
fn large_http_download_uses_one_get_and_persists_exact_completion() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let body = large_http_body();
    let fixture = RangeAdvertisingHttpFixture::new(body.clone());
    let id = "large-http-completion";
    let destination = dir.output_path(id);
    let mut server = ServerProcess::start(dir.path());
    server.call(
        "task.create",
        Some(json!({
            "id": id,
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": id})));
    server.wait_for_state(id, "Completed");
    let task = server.call("task.get", Some(json!({"id": id})));
    server.stop();
    assert_one_full_http_get(&fixture.finish());
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert_eq!(task["downloaded_bytes"], body.len());
    assert_eq!(task["total_bytes"], body.len());
    assert_eq!(task["error"], Value::Null);
    let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    let stored = repository.get(&TaskId::from(id)).unwrap().unwrap();
    assert_eq!(stored.state, TaskState::Completed);
    assert_eq!(stored.progress.downloaded_bytes, body.len() as u64);
    assert_eq!(stored.progress.total_bytes, Some(body.len() as u64));
    assert!(!completion_receipt_path(&CompletionContext::new(id, dir.path())).exists());
}

#[test]
fn large_http_completion_sqlite_failure_recovers_without_origin() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let body = large_http_body();
    let fixture = RangeAdvertisingHttpFixture::new(body.clone());
    let id = "large-http-completion-save-failure";
    let destination = dir.output_path(id);
    let context = CompletionContext::new(id, dir.path());
    let mut server = ServerProcess::start(dir.path());
    server.call(
        "task.create",
        Some(json!({
            "id": id,
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    let connection = rusqlite::Connection::open(dir.path().join("nexum.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_large_completion BEFORE UPDATE ON tasks
         WHEN NEW.state = 'completed' BEGIN SELECT RAISE(FAIL, 'completion save refused'); END;",
        )
        .unwrap();
    server.call("task.queue", Some(json!({"id": id})));
    server.wait_for_state(id, "Failed");
    assert_eq!(fs::read(&destination).unwrap(), body);
    let receipt_path = completion_receipt_path(&context);
    assert!(
        receipt_path.exists(),
        "completed large response lost its recovery receipt"
    );
    let receipt: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    assert_eq!(
        receipt["file"]["sha256"],
        "f1fee4f08908ad9f81ebd057747783a9783aac7bba5625e02379566f014d21cc"
    );
    assert_eq!(receipt["downloaded_bytes"], body.len());
    assert_eq!(receipt["total_bytes"], body.len());
    server.stop();
    let requests = fixture.finish();
    assert_one_full_http_get(&requests);
    connection
        .execute_batch("DROP TRIGGER reject_large_completion")
        .unwrap();
    drop(connection);

    // The listener is closed before startup: successful receipt recovery must
    // persist Completed without fetching the origin again.
    let mut restarted = ServerProcess::start(dir.path());
    restarted.wait_for_state(id, "Completed");
    let task = restarted.call("task.get", Some(json!({"id": id})));
    assert_eq!(task["downloaded_bytes"], body.len());
    assert_eq!(task["total_bytes"], body.len());
    assert_eq!(task["error"], Value::Null);
    assert_eq!(fs::read(&destination).unwrap(), body);
    assert!(!receipt_path.exists());
    restarted.stop();
    let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    let stored = repository.get(&TaskId::from(id)).unwrap().unwrap();
    assert_eq!(stored.state, TaskState::Completed);
    assert_eq!(stored.progress.downloaded_bytes, body.len() as u64);
    assert_eq!(stored.progress.total_bytes, Some(body.len() as u64));
}

#[test]
fn large_http_download_preserves_existing_destination() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let fixture = RangeAdvertisingHttpFixture::new(large_http_body());
    let id = "large-http-existing-destination";
    let destination = dir.output_path(id);
    let original = b"existing file must survive a large download";
    fs::write(&destination, original).unwrap();
    let mut server = ServerProcess::start(dir.path());
    server.call(
        "task.create",
        Some(json!({
            "id": id,
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    server.call("task.queue", Some(json!({"id": id})));
    server.wait_for_state(id, "Failed");
    let task = server.call("task.get", Some(json!({"id": id})));
    assert!(
        task["error"]
            .as_str()
            .unwrap()
            .contains("destination already exists")
    );
    assert_eq!(fs::read(&destination).unwrap(), original);
    assert!(!completion_receipt_path(&CompletionContext::new(id, dir.path())).exists());
    server.stop();
    assert!(
        fixture.finish().is_empty(),
        "an existing target must be rejected before HTTP requests"
    );
}

#[test]
fn completion_reconciles_published_unknown_length_response_without_network() {
    let _test_guard = server_test_guard();
    for state in [TaskState::Downloading, TaskState::Failed] {
        let dir = TestDir::new();
        let id = "published-completion";
        let fixture =
            CompletionFixture::published(&dir, id, b"complete response without a validator");
        fixture.seed_task(&dir, id, state);
        let mut server = ServerProcess::start(dir.path());
        server.wait_for_state(id, "Completed");
        let task = server.call("task.get", Some(json!({"id": id})));
        assert_eq!(task["downloaded_bytes"], fixture.body.len());
        assert_eq!(task["total_bytes"], Value::Null);
        assert_eq!(task["error"], Value::Null);
        assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
        assert!(!completion_receipt_path(&fixture.context).exists());
        server.stop();
        let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
        let stored = repository.get(&TaskId::from(id)).unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Completed);
        assert_eq!(stored.progress.downloaded_bytes, fixture.body.len() as u64);
        assert_eq!(stored.progress.total_bytes, None);
    }
}

#[test]
fn completion_republishes_proven_partial_without_network() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let id = "unpublished-completion";
    let fixture = CompletionFixture::published(&dir, id, b"complete unpublished response");
    // Moving the published inode back to staging constructs the exact durable
    // journal + complete staging boundary before the no-clobber publication.
    fs::rename(&fixture.destination, &fixture.partial).unwrap();
    fixture.seed_task(&dir, id, TaskState::Downloading);
    let server = ServerProcess::start(dir.path());
    server.wait_for_state(id, "Completed");
    assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
    assert!(!fixture.partial.exists());
    assert!(!completion_receipt_path(&fixture.context).exists());
}

#[test]
fn completion_sqlite_failure_survives_process_termination_and_restart() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let fixture = HttpFixture::new(b"published before the completion database save");
    let id = "completion-save-failure";
    let destination = dir.output_path(id);
    let context = CompletionContext::new(id, dir.path());
    let mut server = ServerProcess::start(dir.path());
    server.call(
        "task.create",
        Some(json!({
            "id": id,
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    let connection = rusqlite::Connection::open(dir.path().join("nexum.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_completion BEFORE UPDATE ON tasks
         WHEN NEW.state = 'completed' BEGIN SELECT RAISE(FAIL, 'completion save refused'); END;",
        )
        .unwrap();
    server.call("task.queue", Some(json!({"id": id})));
    fixture.wait_until_connected();
    fixture.finish();
    server.wait_for_state(id, "Failed");
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"published before the completion database save"
    );
    assert!(completion_receipt_path(&context).exists());
    server.stop();
    connection
        .execute_batch("DROP TRIGGER reject_completion")
        .unwrap();
    drop(connection);
    let restarted = ServerProcess::start(dir.path());
    restarted.wait_for_state(id, "Completed");
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"published before the completion database save"
    );
    assert!(!completion_receipt_path(&context).exists());
}

#[test]
fn completion_receipt_survives_when_neither_terminal_state_can_be_saved() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let fixture = HttpFixture::new(b"complete response despite both terminal saves failing");
    let id = "terminal-save-failure";
    let destination = dir.output_path(id);
    let context = CompletionContext::new(id, dir.path());
    let mut server = ServerProcess::start(dir.path());
    server.call(
        "task.create",
        Some(json!({
            "id": id,
            "source": fixture.source,
            "destination": destination.to_string_lossy(),
        })),
    );
    let connection = rusqlite::Connection::open(dir.path().join("nexum.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_terminal BEFORE UPDATE ON tasks
         WHEN NEW.state IN ('completed', 'failed')
         BEGIN SELECT RAISE(FAIL, 'terminal save refused'); END;",
        )
        .unwrap();
    server.call("task.queue", Some(json!({"id": id})));
    fixture.wait_until_connected();
    fixture.finish();
    for _ in 0..100 {
        if destination.exists() && completion_receipt_path(&context).exists() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"complete response despite both terminal saves failing"
    );
    assert!(completion_receipt_path(&context).exists());
    assert_eq!(server.task_state(id), "Downloading");
    server.stop();
    connection
        .execute_batch("DROP TRIGGER reject_terminal")
        .unwrap();
    drop(connection);
    let restarted = ServerProcess::start(dir.path());
    restarted.wait_for_state(id, "Completed");
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"complete response despite both terminal saves failing"
    );
    assert!(!completion_receipt_path(&context).exists());
}

#[test]
fn segmented_completion_receipts_recover_through_the_existing_server_without_network() {
    let _test_guard = server_test_guard();
    for staged in [false, true] {
        let dir = TestDir::new();
        let id = if staged {
            "segmented-staged"
        } else {
            "segmented-published"
        };
        let fixture =
            CompletionFixture::segmented(&dir, id, b"complete segmented response through Server");
        if staged {
            fs::rename(&fixture.destination, &fixture.partial).unwrap();
        }
        fixture.seed_task(
            &dir,
            id,
            if staged {
                TaskState::Failed
            } else {
                TaskState::Downloading
            },
        );
        // The segment listener has exited. A second request would fail; startup
        // must consume the real Engine receipt through existing reconciliation.
        let mut server = ServerProcess::start(dir.path());
        server.wait_for_state(id, "Completed");
        let task = server.call("task.get", Some(json!({"id": id})));
        assert_eq!(task["downloaded_bytes"], fixture.body.len() as u64);
        assert_eq!(task["total_bytes"], fixture.body.len() as u64);
        assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
        assert!(!completion_receipt_path(&fixture.context).exists());
        assert!(!fixture.partial.exists());
        assert_eq!(
            fs::read(nexum_core::nexum_engine::resumable_metadata_path(
                &fixture.partial
            ))
            .unwrap(),
            b"unowned sidecar"
        );
        server.stop();
        let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
        let stored = repository.get(&TaskId::from(id)).unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Completed);
        assert_eq!(stored.progress, fixture.progress);
    }
}

#[test]
fn completion_malformed_receipt_reports_failure_and_preserves_download() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let id = "malformed-completion";
    let fixture =
        CompletionFixture::published(&dir, id, b"published data must survive malformed metadata");
    let receipt = completion_receipt_path(&fixture.context);
    fs::write(&receipt, b"not a completion journal").unwrap();
    fixture.seed_task(&dir, id, TaskState::Downloading);
    let server = ServerProcess::start(dir.path());
    server.wait_for_state(id, "Failed");
    let task = server.call("task.get", Some(json!({"id": id})));
    assert!(
        task["error"]
            .as_str()
            .unwrap()
            .contains("completion reconciliation")
    );
    assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
    assert_eq!(fs::read(&receipt).unwrap(), b"not a completion journal");
}

#[test]
fn completion_recovery_save_failure_keeps_proof_and_does_not_retry_network() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let id = "recovery-save-failure";
    let fixture = CompletionFixture::published(&dir, id, b"verified completed recovery bytes");
    fixture.seed_task(&dir, id, TaskState::Downloading);
    let mut repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    let mut stored = repository.get(&TaskId::from(id)).unwrap().unwrap();
    stored.progress = Progress::new(3, None);
    repository.update(stored).unwrap();
    drop(repository);
    let database = rusqlite::Connection::open(dir.path().join("nexum.sqlite")).unwrap();
    database
        .execute_batch(
            "CREATE TRIGGER reject_recovery_completion BEFORE UPDATE ON tasks
         WHEN NEW.state = 'completed' BEGIN SELECT RAISE(FAIL, 'recovery save refused'); END;",
        )
        .unwrap();
    let mut server = ServerProcess::start(dir.path());
    server.wait_for_state(id, "Failed");
    let task = server.call("task.get", Some(json!({"id": id})));
    // Core claims intentionally reset the working snapshot. A rejected final
    // commit must retain that claim snapshot, rather than publish final bytes
    // independently of Completed.
    assert_eq!(task["downloaded_bytes"], 0);
    assert_eq!(task["total_bytes"], Value::Null);
    assert!(
        task["error"]
            .as_str()
            .unwrap()
            .contains("could not finalize HTTP transfer")
    );
    assert!(
        server.request("task.start", None)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no queued task")
    );
    assert!(completion_receipt_path(&fixture.context).exists());
    assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
    server.stop();
    let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    let failed = repository.get(&TaskId::from(id)).unwrap().unwrap();
    assert_eq!(failed.state, TaskState::Failed);
    assert_eq!(failed.progress, Progress::default());
    drop(repository);
    database
        .execute_batch("DROP TRIGGER reject_recovery_completion")
        .unwrap();
    drop(database);
    let mut restarted = ServerProcess::start(dir.path());
    restarted.wait_for_state(id, "Completed");
    restarted.stop();
    let stored = SqliteRepository::open(dir.path().join("nexum.sqlite"))
        .unwrap()
        .get(&TaskId::from(id))
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, TaskState::Completed);
    assert_eq!(stored.progress, fixture.progress);
    assert!(!completion_receipt_path(&fixture.context).exists());
}

#[test]
fn completion_startup_cleans_durable_completed_hardlink_residue() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let id = "completed-residue";
    let fixture =
        CompletionFixture::published(&dir, id, b"completed response with hardlink residue");
    fs::hard_link(&fixture.destination, &fixture.partial).unwrap();
    fixture.seed_task(&dir, id, TaskState::Completed);
    let server = ServerProcess::start(dir.path());
    assert_eq!(server.task_state(id), "Completed");
    assert!(!fixture.partial.exists());
    assert!(!completion_receipt_path(&fixture.context).exists());
    assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
}

#[test]
fn completion_rejects_same_bytes_in_a_replacement_destination() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let id = "replacement-completion";
    let fixture = CompletionFixture::published(&dir, id, b"identical bytes are insufficient proof");
    let original = dir.output_path("original-proof-inode");
    fs::rename(&fixture.destination, &original).unwrap();
    fs::write(&fixture.destination, fixture.body).unwrap();
    fixture.seed_task(&dir, id, TaskState::Failed);
    let server = ServerProcess::start(dir.path());
    server.wait_for_state(id, "Failed");
    assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
    assert_eq!(fs::read(original).unwrap(), fixture.body);
    assert!(completion_receipt_path(&fixture.context).exists());
    let task = server.call("task.get", Some(json!({"id": id})));
    assert!(
        task["error"]
            .as_str()
            .unwrap()
            .contains("completion reconciliation")
    );
}

#[cfg(unix)]
#[test]
fn completion_recovery_reports_symlink_destination_instead_of_leaving_it_queued() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let id = "symlink-completion";
    let fixture = CompletionFixture::published(&dir, id, b"never follow the replacement link");
    let original = dir.output_path("original-proof-inode");
    fs::rename(&fixture.destination, &original).unwrap();
    std::os::unix::fs::symlink(&original, &fixture.destination).unwrap();
    fixture.seed_task(&dir, id, TaskState::Failed);
    let server = ServerProcess::start(dir.path());
    server.wait_for_state(id, "Failed");
    let task = server.call("task.get", Some(json!({"id": id})));
    assert!(
        task["error"]
            .as_str()
            .unwrap()
            .contains("completion reconciliation")
    );
    assert!(
        fs::symlink_metadata(&fixture.destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(original).unwrap(), fixture.body);
}

#[test]
fn completion_removal_preserves_replaced_partial_and_reuse_discards_old_receipt() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let id = "reused-completion";
    let fixture = CompletionFixture::published(&dir, id, b"old completed response");
    let foreign = b"a user replacement staging file";
    fs::write(&fixture.partial, foreign).unwrap();
    fixture.seed_task(&dir, id, TaskState::Completed);
    let server = ServerProcess::start(dir.path());
    assert!(completion_receipt_path(&fixture.context).exists());
    let response = server.request("task.remove", Some(json!({"id": id})));
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("task removed")
    );
    assert_eq!(fs::read(&fixture.partial).unwrap(), foreign);
    assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
    assert!(completion_receipt_path(&fixture.context).exists());
    assert!(
        server
            .request("task.get", Some(json!({"id": id})))
            .get("error")
            .is_some()
    );
    let replacement = dir.output_path("new-task-destination");
    server.call(
        "task.create",
        Some(json!({
            "id": id,
            "source": fixture.source,
            "destination": replacement.to_string_lossy(),
        })),
    );
    assert_eq!(server.task_state(id), "Created");
    assert!(!completion_receipt_path(&fixture.context).exists());
    assert_eq!(fs::read(&fixture.partial).unwrap(), foreign);
    assert_eq!(fs::read(&fixture.destination).unwrap(), fixture.body);
}

#[test]
fn completion_orphan_cleanup_failure_blocks_task_id_reuse() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let id = "blocked-reuse";
    let context = CompletionContext::new(id, dir.path());
    let receipt = completion_receipt_path(&context);
    fs::create_dir_all(receipt.parent().unwrap()).unwrap();
    // A directory at the receipt filename makes remove_file fail on every
    // platform, without root-user permission exceptions or production hooks.
    fs::create_dir(&receipt).unwrap();
    let server = ServerProcess::start(dir.path());
    let response = server.request(
        "task.create",
        Some(json!({
            "id": id,
            "source": "http://127.0.0.1:1/unreachable",
            "destination": dir.output_path(id).to_string_lossy(),
        })),
    );
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("clear completion receipt")
    );
    assert!(
        server
            .request("task.get", Some(json!({"id": id})))
            .get("error")
            .is_some()
    );
    assert!(receipt.is_dir());
}

#[test]
fn completion_removal_cleanup_failure_still_dispatches_next_queued_download() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let server = ServerProcess::start(dir.path());
    let mut fixtures = Vec::new();
    // Default scheduling has three slots; hold all three at the HTTP body so
    // the fourth task can advance only after an active task is removed.
    for index in 0..4 {
        let fixture = HttpFixture::new(b"body held at a transfer boundary");
        let id = format!("cleanup-dispatch-{index}");
        server.call(
            "task.create",
            Some(json!({
                "id": id,
                "source": fixture.source,
                "destination": dir.output_path(&id).to_string_lossy(),
            })),
        );
        server.call("task.queue", Some(json!({"id": id})));
        if index < 3 {
            fixture.wait_until_connected();
        }
        fixtures.push(fixture);
    }
    assert_eq!(server.task_state("cleanup-dispatch-3"), "Queued");
    let context = CompletionContext::new("cleanup-dispatch-0", dir.path());
    let receipt = completion_receipt_path(&context);
    fs::create_dir_all(&receipt).unwrap();
    let mut remove_stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    remove_stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    writeln!(remove_stream, "{}", json!({
        "jsonrpc": "2.0", "id": 1, "method": "task.remove", "params": {"id": "cleanup-dispatch-0"},
    })).unwrap();
    // request_pause observes the cancellation flag and returns immediately.
    // This makes cancellation deterministic before releasing the HTTP body.
    let pause_response = server.request("task.pause", Some(json!({"id": "cleanup-dispatch-0"})));
    assert!(
        pause_response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cancelled")
    );
    fixtures.remove(0).finish();
    let mut response = String::new();
    BufReader::new(remove_stream)
        .read_line(&mut response)
        .unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("task removed")
    );
    assert!(receipt.is_dir());
    fixtures[2].wait_until_connected();
    assert_eq!(server.task_state("cleanup-dispatch-3"), "Downloading");
    for fixture in fixtures {
        fixture.finish();
    }
    server.wait_for_state("cleanup-dispatch-3", "Completed");
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
    fs::write(&destination, b"old bytes").unwrap();
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
fn http_downloads_to_the_same_destination_do_not_overlap_or_replace() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let mut server = ServerProcess::start(dir.path());
    let first = HttpFixture::new(b"first response");
    let destination = dir.output_path("shared");
    for (id, source) in [
        ("first", first.source.as_str()),
        ("second", "http://127.0.0.1:9/file"),
    ] {
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

    server.wait_for_state("second", "Failed");
    assert!(
        server.call("task.get", Some(json!({"id": "second"})))["error"]
            .as_str()
            .unwrap()
            .contains("destination already exists")
    );
    assert_eq!(fs::read(&destination).unwrap(), b"first response");
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

fn assert_tls_startup_fails(
    data_dir: &Path,
    certificate_path: Option<&Path>,
    key_path: Option<&Path>,
    expected_error: &str,
) {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let mut args = Vec::new();
    if let Some(path) = certificate_path {
        args.extend(["--tls-cert-path", path.to_str().unwrap()]);
    }
    if let Some(path) = key_path {
        args.extend(["--tls-key-path", path.to_str().unwrap()]);
    }
    let output = server_output_with_args(data_dir, Some(port), &args);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "server started with invalid TLS configuration: {stderr}"
    );
    assert!(
        stderr.contains(expected_error),
        "expected {expected_error:?}, got: {stderr}"
    );
}

#[test]
fn tls_startup_rejects_incomplete_or_missing_certificate_material() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let generated = generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate_path = dir.root.join("server-cert.pem");
    let key_path = dir.root.join("server-key.pem");
    fs::write(&certificate_path, generated.cert.pem()).unwrap();
    fs::write(&key_path, generated.key_pair.serialize_pem()).unwrap();
    let missing_path = dir.root.join("missing.pem");

    assert_tls_startup_fails(
        dir.path(),
        Some(&certificate_path),
        None,
        "tls_cert_path and tls_key_path must be configured together",
    );
    assert_tls_startup_fails(
        dir.path(),
        None,
        Some(&key_path),
        "tls_cert_path and tls_key_path must be configured together",
    );
    assert_tls_startup_fails(
        dir.path(),
        Some(&missing_path),
        Some(&key_path),
        "cannot read TLS file",
    );
    assert_tls_startup_fails(
        dir.path(),
        Some(&certificate_path),
        Some(&missing_path),
        "cannot read TLS file",
    );
}

#[test]
fn tls_startup_rejects_invalid_pem_and_mismatched_private_key() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let generated = generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let unrelated = generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate_path = dir.root.join("server-cert.pem");
    let key_path = dir.root.join("server-key.pem");
    let empty_path = dir.root.join("empty.pem");
    let invalid_certificate_path = dir.root.join("invalid-cert.pem");
    let invalid_key_path = dir.root.join("invalid-key.pem");
    let unrelated_key_path = dir.root.join("unrelated-key.pem");
    fs::write(&certificate_path, generated.cert.pem()).unwrap();
    fs::write(&key_path, generated.key_pair.serialize_pem()).unwrap();
    fs::write(&empty_path, b"").unwrap();
    fs::write(
        &invalid_certificate_path,
        b"-----BEGIN CERTIFICATE-----\n%%%\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    fs::write(
        &invalid_key_path,
        b"-----BEGIN PRIVATE KEY-----\n%%%\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    fs::write(&unrelated_key_path, unrelated.key_pair.serialize_pem()).unwrap();

    assert_tls_startup_fails(
        dir.path(),
        Some(&empty_path),
        Some(&key_path),
        "contains no certificates",
    );
    assert_tls_startup_fails(
        dir.path(),
        Some(&invalid_certificate_path),
        Some(&key_path),
        "invalid TLS certificate PEM",
    );
    assert_tls_startup_fails(
        dir.path(),
        Some(&certificate_path),
        Some(&empty_path),
        "contains no private key",
    );
    assert_tls_startup_fails(
        dir.path(),
        Some(&certificate_path),
        Some(&invalid_key_path),
        "invalid TLS private key PEM",
    );
    assert_tls_startup_fails(
        dir.path(),
        Some(&certificate_path),
        Some(&unrelated_key_path),
        "cannot build TLS server configuration",
    );
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
    // Listening readiness precedes asynchronous recovery dispatch/retries. The
    // fixture is closed, so wait for the terminal failure instead of racing a
    // legitimate Queued/Downloading state on the first RPC after startup.
    restarted.wait_for_state("running", "Failed");
    assert_eq!(restarted.task_state("running"), "Failed");
    let repository = SqliteRepository::open(dir.path().join("nexum.sqlite")).unwrap();
    let recovered = repository.get(&TaskId::from("running")).unwrap().unwrap();
    assert_eq!(recovered.state, TaskState::Failed);
    assert!(
        recovered.last_error.is_some(),
        "failed recovery must persist its transfer error"
    );
    drop(repository);
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

#[test]
fn tls_handshake_rejects_untrusted_and_wrong_hostname_certificates() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let (mut server, certificate_der) = ServerProcess::start_with_tls(dir.path(), &dir.root);
    let unrelated = generate_simple_self_signed(vec!["unrelated.example".to_owned()]).unwrap();

    let untrusted = try_tls_client_stream(server.port, unrelated.cert.der().as_ref(), "localhost")
        .expect_err("an unrelated root must not trust the Server certificate");
    assert!(
        matches!(
            untrusted
                .get_ref()
                .and_then(|error| error.downcast_ref::<rustls::Error>()),
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer | rustls::CertificateError::BadSignature
            ))
        ),
        "unexpected trust error: {untrusted}"
    );

    let wrong_hostname = try_tls_client_stream(server.port, &certificate_der, "wrong.example")
        .expect_err("a trusted certificate must not match an unrelated hostname");
    assert!(
        matches!(
            wrong_hostname
                .get_ref()
                .and_then(|error| error.downcast_ref::<rustls::Error>()),
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::NotValidForName
                    | rustls::CertificateError::NotValidForNameContext { .. }
            ))
        ),
        "unexpected hostname error: {wrong_hostname}"
    );

    // A failed handshake must not switch the listener to plaintext or stop it.
    let response = tls_rpc_request(
        server.port,
        &certificate_der,
        json!({"jsonrpc": "2.0", "id": 1, "method": "server.version"}),
    );
    assert_eq!(response["result"], "1");
    server.stop();
}

#[test]
fn tls_transport_rejects_plaintext_and_serves_https_jsonrpc_with_cors() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let (mut server, certificate_der) = ServerProcess::start_with_tls(dir.path(), &dir.root);

    // A TLS listener must not accept an HTTP request as a plaintext fallback.
    let plaintext_response = plaintext_probe_tls_port(server.port);
    assert!(!plaintext_response.starts_with(b"HTTP/1.1"));
    assert!(!plaintext_response.starts_with(b"{\"jsonrpc\""));

    let (status, headers, body) = tls_http_request(
        server.port,
        &certificate_der,
        "OPTIONS",
        "/jsonrpc",
        None,
        Some("chrome-extension://test-id"),
    );
    assert_eq!(status, "HTTP/1.1 204 No Content");
    assert!(headers.contains("Access-Control-Allow-Origin: chrome-extension://test-id"));
    assert_eq!(body, Value::Null);

    let (status, headers, body) = tls_http_request(
        server.port,
        &certificate_der,
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "server.version"})),
        Some("chrome-extension://test-id"),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(headers.contains("Access-Control-Allow-Origin: chrome-extension://test-id"));
    assert_eq!(body["result"], "1");

    let (status, headers, body) = tls_http_request(
        server.port,
        &certificate_der,
        "POST",
        "/jsonrpc",
        Some(json!({"jsonrpc": "2.0", "id": 2, "method": "server.version"})),
        Some("https://evil.example"),
    );
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    assert!(!headers.contains("Access-Control-Allow-Origin"));
    assert_eq!(body["error"], "cross-origin request is not allowed");

    let response = tls_rpc_request(
        server.port,
        &certificate_der,
        json!({"jsonrpc": "2.0", "id": 3, "method": "server.version"}),
    );
    assert_eq!(response["result"], "1");
    server.stop();
}

#[test]
fn tls_transport_applies_authentication_to_tcp_and_https_requests() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let (mut server, certificate_der) = ServerProcess::start_with_tls_and_auth(
        dir.path(),
        &dir.root,
        Some(("ApiKey", "tls-secret")),
    );

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server.version"
    });
    let response = tls_rpc_request(server.port, &certificate_der, request.clone());
    assert_unauthorized(&response);

    let mut authenticated = request.clone();
    authenticated["credential"] = json!({"ApiKey": {"key": "tls-secret"}});
    let response = tls_rpc_request(server.port, &certificate_der, authenticated.clone());
    assert_eq!(response["result"], "1");

    let (status, headers, response) = tls_http_request(
        server.port,
        &certificate_der,
        "POST",
        "/jsonrpc",
        Some(request),
        Some("chrome-extension://test-id"),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(headers.contains("Access-Control-Allow-Origin: chrome-extension://test-id"));
    assert_unauthorized(&response);

    let (status, headers, response) = tls_http_request(
        server.port,
        &certificate_der,
        "POST",
        "/jsonrpc",
        Some(authenticated),
        Some("chrome-extension://test-id"),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(headers.contains("Access-Control-Allow-Origin: chrome-extension://test-id"));
    assert_eq!(response["result"], "1");
    server.stop();
}

#[test]
fn tls_authenticated_event_subscription_receives_task_events() {
    let _test_guard = server_test_guard();
    let dir = TestDir::new();
    let (mut server, certificate_der) = ServerProcess::start_with_tls_and_auth(
        dir.path(),
        &dir.root,
        Some(("ApiKey", "tls-event-secret")),
    );

    let rejected = tls_rpc_request(
        server.port,
        &certificate_der,
        json!({"jsonrpc": "2.0", "id": 1, "method": "events.subscribe"}),
    );
    assert_unauthorized(&rejected);

    let mut stream = tls_client_stream(server.port, &certificate_der);
    writeln!(
        stream,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "events.subscribe",
            "credential": {"ApiKey": {"key": "tls-event-secret"}}
        })
    )
    .unwrap();
    let mut events = BufReader::new(stream);
    let mut line = String::new();
    events.read_line(&mut line).unwrap();
    let subscribed: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(subscribed["result"]["subscribed"], true);

    let created = tls_rpc_request(
        server.port,
        &certificate_der,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "task.create",
            "params": {
                "id": "tls-event-task",
                "source": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
                "destination": dir.output_path("tls-event-task").to_string_lossy(),
            },
            "credential": {"ApiKey": {"key": "tls-event-secret"}}
        }),
    );
    assert_eq!(created["result"]["id"], "tls-event-task");

    let mut observed_created = false;
    for _ in 0..12 {
        line.clear();
        match events.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.trim().is_empty() {
                    continue;
                }
                let message: Value = serde_json::from_str(&line).unwrap();
                if message["method"] == "events.event"
                    && message["params"]["event"] == "task.created"
                {
                    observed_created = true;
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(error) => panic!("could not read TLS event: {error}"),
        }
    }
    assert!(observed_created, "TLS event stream missed task.created");
    drop(events);
    server.stop();
}
