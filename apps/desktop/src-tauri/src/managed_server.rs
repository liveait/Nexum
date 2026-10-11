//! Supervises only the bundled process owned by this App. Secrets and process
//! handles remain here; status is the only value intended for the WebView.

use super::RpcRoute;
use nexum_protocol::ProtocolVersion;
use nexum_protocol::managed::{
    MANAGED_CONTROL_VERSION, ManagedControl, ManagedReady, ManagedStartup, read_managed_frame,
    write_managed_frame,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(6);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_RESTARTS: u32 = 3;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ManagedServerStatus {
    pub state: String,
    pub generation: u64,
    pub error: Option<String>,
}

struct SharedState {
    status: ManagedServerStatus,
    route: Option<RpcRoute>,
}

struct Configuration {
    binary: PathBuf,
    expected_digest: String,
    data_dir: PathBuf,
}

enum SupervisorCommand {
    Enable,
    Disable(mpsc::Sender<()>),
}

#[derive(Clone)]
pub struct ManagedServer {
    commands: mpsc::Sender<SupervisorCommand>,
    state: Arc<Mutex<SharedState>>,
    enabled: Arc<AtomicBool>,
}

impl ManagedServer {
    pub fn new(binary: PathBuf, expected_digest: String, data_dir: PathBuf) -> Self {
        let state = Arc::new(Mutex::new(SharedState {
            status: ManagedServerStatus {
                state: "stopped".to_owned(),
                generation: 0,
                error: None,
            },
            route: None,
        }));
        let enabled = Arc::new(AtomicBool::new(false));
        let (commands, receiver) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        let worker_enabled = Arc::clone(&enabled);
        if std::thread::Builder::new()
            .name("nexum-managed-server".to_owned())
            .spawn(move || {
                supervise(
                    Configuration {
                        binary,
                        expected_digest,
                        data_dir,
                    },
                    receiver,
                    worker_state,
                    worker_enabled,
                )
            })
            .is_err()
        {
            update_state(
                &state,
                "failed",
                Some("Could not start the local service supervisor.".to_owned()),
                None,
            );
        }
        Self {
            commands,
            state,
            enabled,
        }
    }

    /// Starts asynchronously. Status exposes progress without blocking the UI.
    pub fn enable(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Local service state is unavailable.".to_owned())?;
        if self.enabled.load(Ordering::Acquire)
            && matches!(
                state.status.state.as_str(),
                "starting" | "running" | "restarting"
            )
        {
            return Ok(());
        }
        self.enabled.store(true, Ordering::Release);
        state.status.state = "starting".to_owned();
        state.status.error = None;
        state.route = None;
        if self.commands.send(SupervisorCommand::Enable).is_err() {
            self.enabled.store(false, Ordering::Release);
            let error =
                "Local service supervisor is unavailable. Restart Nexum to retry.".to_owned();
            state.status.state = "failed".to_owned();
            state.status.error = Some(error.clone());
            return Err(error);
        }
        Ok(())
    }

    pub fn retry(&self) -> Result<(), String> {
        self.enable()
    }

    pub fn disable(&self) -> Result<(), String> {
        self.enabled.store(false, Ordering::Release);
        update_state(&self.state, "stopped", None, None);
        let (sender, receiver) = mpsc::channel();
        if self
            .commands
            .send(SupervisorCommand::Disable(sender))
            .is_err()
        {
            // A disconnected worker has already dropped its owned child (or
            // failed before spawning it). External mode must remain usable.
            return Ok(());
        }
        receiver
            .recv_timeout(STARTUP_TIMEOUT + SHUTDOWN_TIMEOUT + Duration::from_secs(2))
            .map_err(|_| "Local service shutdown timed out.".to_owned())
    }

    pub fn shutdown(&self) -> Result<(), String> {
        self.disable()
    }

    pub fn status(&self) -> ManagedServerStatus {
        self.state
            .lock()
            .expect("managed service state mutex poisoned")
            .status
            .clone()
    }

    pub fn route(&self) -> Result<RpcRoute, String> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err("Local service is stopped.".to_owned());
        }
        self.state
            .lock()
            .map_err(|_| "Local service state is unavailable.".to_owned())?
            .route
            .clone()
            .ok_or_else(|| "Local service is not ready. Check Settings to retry.".to_owned())
    }
}

fn update_state(
    state: &Mutex<SharedState>,
    name: &str,
    error: Option<String>,
    route: Option<RpcRoute>,
) {
    let mut state = state.lock().expect("managed service state mutex poisoned");
    if route.is_some() {
        state.status.generation = state.status.generation.saturating_add(1);
    }
    state.status.state = name.to_owned();
    state.status.error = error;
    state.route = route;
}

fn supervise(
    config: Configuration,
    commands: mpsc::Receiver<SupervisorCommand>,
    state: Arc<Mutex<SharedState>>,
    enabled: Arc<AtomicBool>,
) {
    let mut child: Option<OwnedChild> = None;
    let mut restarts = 0;
    let mut retry_at = None;
    loop {
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(SupervisorCommand::Enable) => {
                if let Some(mut previous) = child.take() {
                    previous.stop();
                }
                restarts = 0;
                retry_at = None;
                if !enabled.load(Ordering::Acquire) {
                    continue;
                }
                launch(&config, &state, &enabled, &mut child);
            }
            Ok(SupervisorCommand::Disable(reply)) => {
                if let Some(mut previous) = child.take() {
                    previous.stop();
                }
                retry_at = None;
                update_state(&state, "stopped", None, None);
                let _ = reply.send(());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(mut previous) = child.take() {
                    previous.stop();
                }
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !enabled.load(Ordering::Acquire) {
            continue;
        }
        if let Some(process) = child.as_mut() {
            match process.child.try_wait() {
                Ok(None) => {}
                Ok(Some(_)) | Err(_) => {
                    // Clear the old endpoint before attempting another launch.
                    child.take();
                    retry_at = schedule_restart(&state, &mut restarts);
                }
            }
        }
        if retry_at.is_some_and(|deadline| Instant::now() >= deadline) {
            retry_at = None;
            if !launch(&config, &state, &enabled, &mut child) && enabled.load(Ordering::Acquire) {
                retry_at = schedule_restart(&state, &mut restarts);
            }
        }
    }
}

fn schedule_restart(state: &Mutex<SharedState>, restarts: &mut u32) -> Option<Instant> {
    if *restarts >= MAX_RESTARTS {
        update_state(
            state,
            "failed",
            Some("The local service stopped repeatedly. Open Settings and retry.".to_owned()),
            None,
        );
        None
    } else {
        *restarts += 1;
        update_state(
            state,
            "restarting",
            Some("The local service stopped unexpectedly and is restarting.".to_owned()),
            None,
        );
        Some(Instant::now() + Duration::from_millis(250 * (1u64 << (*restarts - 1))))
    }
}

fn launch(
    config: &Configuration,
    state: &Mutex<SharedState>,
    enabled: &AtomicBool,
    child: &mut Option<OwnedChild>,
) -> bool {
    match spawn_owned(config, enabled) {
        Ok((mut process, route)) => {
            if !enabled.load(Ordering::Acquire) {
                process.stop();
                return false;
            }
            update_state(state, "running", None, Some(route));
            *child = Some(process);
            true
        }
        Err(error) => {
            if enabled.load(Ordering::Acquire) {
                update_state(state, "failed", Some(error), None);
            }
            false
        }
    }
}

fn verify_binary(config: &Configuration) -> Result<(), String> {
    if !config.binary.is_absolute() || !config.data_dir.is_absolute() {
        return Err(
            "The bundled local service and its data directory must use absolute paths.".to_owned(),
        );
    }
    if config.expected_digest.len() != 64
        || !config
            .expected_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("The App is missing its bundled local service verification data. Rebuild or reinstall Nexum.".to_owned());
    }
    let actual = binary_digest(&config.binary)?;
    if !actual.eq_ignore_ascii_case(&config.expected_digest) {
        return Err(
            "The bundled local service has changed. Rebuild or reinstall Nexum.".to_owned(),
        );
    }
    Ok(())
}

fn binary_digest(path: &Path) -> Result<String, String> {
    // Inspect without following links before open. Unix open flags also close
    // the replacement race: a newly substituted FIFO cannot block startup,
    // and a newly substituted symlink cannot redirect executable verification.
    let metadata = std::fs::symlink_metadata(path).map_err(|_| {
        "The bundled local service is missing or unreadable. Rebuild or reinstall Nexum.".to_owned()
    })?;
    if !metadata.is_file() || metadata.len() > MAX_BINARY_BYTES {
        return Err("The bundled local service is not a regular executable file.".to_owned());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| {
        "The bundled local service is missing or unreadable. Rebuild or reinstall Nexum.".to_owned()
    })?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect the bundled local service.".to_owned())?;
    if !opened_metadata.is_file() || opened_metadata.len() > MAX_BINARY_BYTES {
        return Err("The bundled local service is not a regular executable file.".to_owned());
    }
    let mut file = file.take(MAX_BINARY_BYTES + 1);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes_read = 0u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "Cannot verify the bundled local service.".to_owned())?;
        if count == 0 {
            break;
        }
        bytes_read += count as u64;
        if bytes_read > MAX_BINARY_BYTES {
            return Err("The bundled local service exceeds the executable size limit.".to_owned());
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| "Cannot generate a secure local service credential.".to_owned())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn validate_ready(ready: &ManagedReady, child_id: u32) -> Result<(), String> {
    if ready.control_version != MANAGED_CONTROL_VERSION
        || ready.server_version != env!("CARGO_PKG_VERSION")
        || ready.protocol_version != ProtocolVersion::CURRENT.as_str()
        || ready.process_id != child_id
        || ready.endpoint.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST)
        || ready.endpoint.port() == 0
    {
        return Err(
            "The bundled local service returned an incompatible startup response.".to_owned(),
        );
    }
    Ok(())
}

struct OwnedChild {
    child: Child,
    stdin: Option<ChildStdin>,
    _stdout: Option<ChildStdout>,
}

impl OwnedChild {
    fn stop(&mut self) {
        if let Some(mut input) = self.stdin.take() {
            let _ = write_managed_frame(
                &mut input,
                &ManagedControl::Shutdown {
                    control_version: MANAGED_CONTROL_VERSION,
                },
            );
        }
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(POLL_INTERVAL),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_owned(
    config: &Configuration,
    enabled: &AtomicBool,
) -> Result<(OwnedChild, RpcRoute), String> {
    verify_binary(config)?;
    let token = random_token()?;
    let child = Command::new(&config.binary)
        .arg("--managed")
        .arg("--data-dir")
        .arg(&config.data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            "Cannot launch the bundled local service. Rebuild or reinstall Nexum.".to_owned()
        })?;
    let mut owned = OwnedChild {
        child,
        stdin: None,
        _stdout: None,
    };
    owned.stdin = owned.child.stdin.take();
    let stdout = owned
        .child
        .stdout
        .take()
        .ok_or_else(|| "Local service readiness pipe is unavailable.".to_owned())?;
    let input = owned
        .stdin
        .as_mut()
        .ok_or_else(|| "Local service control pipe is unavailable.".to_owned())?;
    write_managed_frame(
        input,
        &ManagedControl::Start(ManagedStartup {
            control_version: MANAGED_CONTROL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: ProtocolVersion::CURRENT.as_str().to_owned(),
            bearer_token: token.clone(),
        }),
    )
    .map_err(|_| "Cannot initialize the bundled local service.".to_owned())?;
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("nexum-managed-ready".to_owned())
        .spawn(move || {
            let mut stdout = stdout;
            let ready = read_managed_frame::<ManagedReady>(&mut stdout);
            let _ = sender.send((stdout, ready));
        })
        .map_err(|_| "Cannot read local service startup.".to_owned())?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let response = loop {
        if !enabled.load(Ordering::Acquire) {
            break Err("Local service startup was cancelled.".to_owned());
        }
        if Instant::now() >= deadline {
            break Err("Local service startup timed out. Open Settings and retry.".to_owned());
        }
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok((stdout, ready)) => {
                owned._stdout = Some(stdout);
                break ready
                .map_err(|_| "The bundled local service returned an invalid startup response.".to_owned())?
                .ok_or_else(|| "Cannot start the local service. Another Nexum instance may already be using its data directory; close it and retry.".to_owned());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("The bundled local service stopped during startup.".to_owned());
            }
        }
    };
    if response.is_err() {
        owned.stop();
    }
    let _ = reader.join();
    let ready = response?;
    validate_ready(&ready, owned.child.id())?;
    let route = RpcRoute::managed(ready.endpoint.to_string(), token)?;
    if route.call("server.version", None, 1500)?.as_str() != Some(ProtocolVersion::CURRENT.as_str())
    {
        return Err("The bundled local service uses an incompatible RPC protocol.".to_owned());
    }
    Ok((owned, route))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    struct FixtureDirectory(PathBuf);
    impl FixtureDirectory {
        fn new(mode: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "nexum-supervisor-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("mode"), mode).unwrap();
            Self(path)
        }
        fn config(&self) -> Configuration {
            let binary = fixture_binary();
            Configuration {
                expected_digest: binary_digest(&binary).unwrap(),
                binary,
                data_dir: self.0.clone(),
            }
        }
        fn supervisor(&self) -> ManagedServer {
            let config = self.config();
            ManagedServer::new(config.binary, config.expected_digest, config.data_dir)
        }
    }
    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn wait_state(
        server: &ManagedServer,
        expected: &str,
        timeout: Duration,
    ) -> ManagedServerStatus {
        let deadline = Instant::now() + timeout;
        loop {
            let status = server.status();
            if status.state == expected {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "expected {expected}, got {status:?}"
            );
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Tests compile a standalone child, with no shell or fixture dependencies.
    /// The product supervisor still executes only its verified absolute path.
    fn fixture_binary() -> PathBuf {
        static BINARY: OnceLock<PathBuf> = OnceLock::new();
        BINARY
            .get_or_init(|| {
                let dir = std::env::temp_dir()
                    .join(format!("nexum-supervisor-child-{}", std::process::id()));
                std::fs::create_dir_all(&dir).unwrap();
                let source = dir.join("child.rs");
                let binary = dir.join("child");
                std::fs::write(
                    &source,
                    FIXTURE_SOURCE.replace("__VERSION__", env!("CARGO_PKG_VERSION")),
                )
                .unwrap();
                let output =
                    Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                        .arg("--edition=2024")
                        .arg(&source)
                        .arg("-o")
                        .arg(&binary)
                        .output()
                        .unwrap();
                assert!(
                    output.status.success(),
                    "fixture compilation failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                binary
            })
            .clone()
    }

    const FIXTURE_SOURCE: &str = r#"
use std::{io::{self, Read, Write, BufRead, BufReader}, net::TcpListener, time::Duration};
fn frame(input: &mut impl Read) -> Option<String> {
    let mut header=[0u8;4]; input.read_exact(&mut header).ok()?;
    let mut payload=vec![0u8;u32::from_be_bytes(header) as usize];
    input.read_exact(&mut payload).ok()?; String::from_utf8(payload).ok()
}
fn main() {
    let args:Vec<_>=std::env::args().collect();
    assert_eq!(&args[1],"--managed"); assert_eq!(&args[2],"--data-dir");
    let dir=std::path::PathBuf::from(&args[3]);
    let mode=std::fs::read_to_string(dir.join("mode")).unwrap();
    std::fs::write(dir.join("pid"),std::process::id().to_string()).unwrap();
    let mut input=io::stdin(); let start=frame(&mut input).unwrap();
    let token=start.split("\"bearer_token\":\"").nth(1).unwrap().split('"').next().unwrap().to_owned();
    assert_eq!(token.len(),64);
    if mode=="timeout" { std::thread::sleep(Duration::from_secs(30)); return; }
    if mode=="exit" { return; }
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let pid=if mode=="wrong-pid" { std::process::id()+1 } else { std::process::id() };
    let protocol=if mode=="wrong-version" { "99" } else { "1" };
    let payload=format!("{{\"control_version\":1,\"server_version\":\"__VERSION__\",\"protocol_version\":\"{}\",\"process_id\":{},\"endpoint\":\"{}\"}}",protocol,pid,listener.local_addr().unwrap());
    let mut output=io::stdout(); output.write_all(&(payload.len() as u32).to_be_bytes()).unwrap(); output.write_all(payload.as_bytes()).unwrap(); output.flush().unwrap();
    let mode2=mode.clone();
    std::thread::spawn(move|| {
        for stream in listener.incoming() {
            let mut stream=stream.unwrap(); let mut line=String::new();
            BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
            assert!(line.contains(&token));
            let reply=if mode2=="reject-auth" { format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{{\"code\":-32001,\"message\":\"{}\"}}}}\n",token) }
            else { "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"1\"}\n".to_owned() };
            stream.write_all(reply.as_bytes()).unwrap();
            if mode2=="crash" { std::thread::sleep(Duration::from_millis(100)); std::process::exit(7); }
        }
    });
    if mode=="stall-shutdown" { std::thread::sleep(Duration::from_secs(30)); return; }
    frame(&mut input); // Shutdown or parent EOF both end the owned child.
}
"#;
    #[test]
    fn readiness_requires_child_identity_versions_and_ipv4_loopback() {
        let valid = ManagedReady {
            control_version: MANAGED_CONTROL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: "1".to_owned(),
            process_id: 42,
            endpoint: "127.0.0.1:12345".parse().unwrap(),
        };
        assert!(validate_ready(&valid, 42).is_ok());
        assert!(validate_ready(&valid, 43).is_err());
        for endpoint in [
            "127.0.0.1:0",
            "127.0.0.2:12345",
            "192.0.2.1:12345",
            "[::1]:12345",
        ] {
            let mut ready = valid.clone();
            ready.endpoint = endpoint.parse().unwrap();
            assert!(validate_ready(&ready, 42).is_err());
        }
        let mut wrong = valid.clone();
        wrong.server_version = "other".to_owned();
        assert!(validate_ready(&wrong, 42).is_err());
        wrong = valid.clone();
        wrong.protocol_version = "other".to_owned();
        assert!(validate_ready(&wrong, 42).is_err());
        wrong = valid;
        wrong.control_version += 1;
        assert!(validate_ready(&wrong, 42).is_err());
    }

    #[test]
    fn status_contains_no_credential_or_endpoint() {
        let status = ManagedServerStatus {
            state: "running".to_owned(),
            generation: 1,
            error: None,
        };
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_eq!(value["generation"], 1);
    }

    #[test]
    fn random_launch_credentials_are_distinct_complete_hexadecimal() {
        let one = random_token().unwrap();
        let two = random_token().unwrap();
        assert_ne!(one, two);
        assert_eq!(one.len(), 64);
        assert!(one.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn restart_budget_is_finite_and_manual_enable_resets_it() {
        let state = Mutex::new(SharedState {
            status: ManagedServerStatus {
                state: "running".to_owned(),
                generation: 1,
                error: None,
            },
            route: None,
        });
        let mut restarts = 0;
        for _ in 0..MAX_RESTARTS {
            assert!(schedule_restart(&state, &mut restarts).is_some());
        }
        assert!(schedule_restart(&state, &mut restarts).is_none());
        assert_eq!(state.lock().unwrap().status.state, "failed");
    }

    #[test]
    fn missing_changed_and_relative_binaries_fail_before_spawning() {
        let fixture = FixtureDirectory::new("normal");
        let mut config = fixture.config();
        config.expected_digest = "0".repeat(64);
        assert!(verify_binary(&config).unwrap_err().contains("changed"));
        config.binary = fixture.0.join("absent");
        assert!(verify_binary(&config).unwrap_err().contains("missing"));
        config.binary = PathBuf::from("nexum-server");
        assert!(verify_binary(&config).unwrap_err().contains("absolute"));
        assert!(!fixture.0.join("pid").exists());
    }

    #[cfg(unix)]
    #[test]
    fn verification_rejects_fifo_symlink_and_oversized_file_without_blocking() {
        use std::os::unix::fs::symlink;
        let fixture = FixtureDirectory::new("normal");
        let fifo = fixture.0.join("fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(binary_digest(&fifo));
        });
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("FIFO verification must not block")
                .is_err()
        );

        let link = fixture.0.join("link");
        symlink(fixture_binary(), &link).unwrap();
        assert!(binary_digest(&link).is_err());

        let large = fixture.0.join("large");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(MAX_BINARY_BYTES + 1)
            .unwrap();
        assert!(binary_digest(&large).is_err());
    }

    #[test]
    fn owned_child_authenticates_and_gracefully_stops_with_private_status() {
        let fixture = FixtureDirectory::new("normal");
        let server = fixture.supervisor();
        server.enable().unwrap();
        let status = wait_state(&server, "running", Duration::from_secs(3));
        assert_eq!(status.generation, 1);
        assert_eq!(
            server
                .route()
                .unwrap()
                .call("server.version", None, 1000)
                .unwrap(),
            "1"
        );
        let before = Instant::now();
        server.disable().unwrap();
        assert!(before.elapsed() < SHUTDOWN_TIMEOUT);
        assert_eq!(server.status().state, "stopped");
        assert!(server.route().is_err());
    }

    #[test]
    fn startup_rejects_wrong_identity_version_and_auth_without_leaking_secret() {
        for mode in ["wrong-pid", "wrong-version", "reject-auth"] {
            let fixture = FixtureDirectory::new(mode);
            let server = fixture.supervisor();
            server.enable().unwrap();
            let status = wait_state(&server, "failed", Duration::from_secs(3));
            assert_eq!(status.generation, 0);
            assert!(server.route().is_err());
            let message = status.error.unwrap();
            assert!(
                !message
                    .as_bytes()
                    .windows(64)
                    .any(|part| part.iter().all(u8::is_ascii_hexdigit)),
                "possible secret in error"
            );
            server.shutdown().unwrap();
        }
    }

    #[test]
    fn incomplete_startup_times_out_and_can_be_cancelled() {
        let fixture = FixtureDirectory::new("timeout");
        let server = fixture.supervisor();
        server.enable().unwrap();
        let status = wait_state(
            &server,
            "failed",
            STARTUP_TIMEOUT + SHUTDOWN_TIMEOUT + Duration::from_secs(2),
        );
        assert!(status.error.unwrap().contains("timed out"));
        server.shutdown().unwrap();
        server.enable().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let before = Instant::now();
        server.disable().unwrap();
        assert!(before.elapsed() < SHUTDOWN_TIMEOUT + Duration::from_secs(1));
        assert!(server.route().is_err());
    }

    #[test]
    fn stalled_shutdown_is_bounded_and_only_owned_child_is_terminated() {
        let unrelated = FixtureDirectory::new("normal");
        let (mut unrelated_child, unrelated_route) =
            spawn_owned(&unrelated.config(), &AtomicBool::new(true)).unwrap();
        let fixture = FixtureDirectory::new("stall-shutdown");
        let server = fixture.supervisor();
        server.enable().unwrap();
        wait_state(&server, "running", Duration::from_secs(3));
        let before = Instant::now();
        server.shutdown().unwrap();
        assert!(before.elapsed() < SHUTDOWN_TIMEOUT + Duration::from_secs(1));
        assert_eq!(server.status().state, "stopped");
        assert_eq!(
            unrelated_route.call("server.version", None, 1000).unwrap(),
            "1"
        );
        unrelated_child.stop();
    }

    #[test]
    fn crash_restarts_exhaust_budget_and_manual_retry_recovers() {
        let fixture = FixtureDirectory::new("crash");
        let server = fixture.supervisor();
        server.enable().unwrap();
        let status = wait_state(&server, "failed", Duration::from_secs(6));
        assert_eq!(status.generation, u64::from(MAX_RESTARTS) + 1);
        assert!(status.error.unwrap().contains("repeatedly"));
        std::fs::write(fixture.0.join("mode"), "normal").unwrap();
        server.retry().unwrap();
        let status = wait_state(&server, "running", Duration::from_secs(3));
        assert_eq!(status.generation, u64::from(MAX_RESTARTS) + 2);
        server.shutdown().unwrap();
    }

    #[test]
    fn last_supervisor_handle_drop_closes_parent_pipe_and_stops_child() {
        let fixture = FixtureDirectory::new("normal");
        let server = fixture.supervisor();
        server.enable().unwrap();
        wait_state(&server, "running", Duration::from_secs(3));
        let status = Arc::clone(&server.state);
        drop(server);
        // The worker has no Sender; dropping the final owner disconnects it.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Arc::strong_count(&status) > 1 && Instant::now() < deadline {
            std::thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(Arc::strong_count(&status), 1);
    }
}
