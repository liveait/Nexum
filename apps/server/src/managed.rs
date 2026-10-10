//! Private desktop process bootstrap. Stdout contains one framed readiness message.

use super::{
    AuthenticationConfig, ConnectionLimiter, EventHub, ServerConfig, ServerState, SharedStream,
    dispatch_available_http_tasks, handle_connection, lock_data_dir, open_core, spawn_event_pump,
};
use nexum_protocol::RpcDispatcher;
use nexum_protocol::managed::{
    MANAGED_CONTROL_VERSION, ManagedControl, ManagedReady, ManagedStartup, read_managed_frame,
    write_managed_frame,
};
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{Shutdown, TcpListener};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(super) fn parse_args(args: &[String]) -> Result<ServerConfig, String> {
    let mut managed = false;
    let mut data_dir = None;
    let mut max_connections = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--managed" if !managed => managed = true,
            "--data-dir" if data_dir.is_none() => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or("managed mode requires --data-dir PATH")?;
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err("managed mode requires an absolute data directory".to_owned());
                }
                data_dir = Some(path);
            }
            "--max-connections" if max_connections.is_none() => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("--max-connections requires a positive integer")?;
                let limit = value
                    .parse::<usize>()
                    .ok()
                    .filter(|limit| *limit > 0)
                    .ok_or("--max-connections requires a positive integer")?;
                max_connections = Some(limit);
            }
            _ => {
                // Never echo arguments: rejected values can contain credentials.
                return Err(
                    "managed mode accepts only --managed, --data-dir, and --max-connections"
                        .to_owned(),
                );
            }
        }
        index += 1;
    }
    if !managed {
        return Err("managed mode requires --managed".to_owned());
    }
    Ok(ServerConfig {
        port: 0,
        data_dir: data_dir.ok_or("managed mode requires an explicit --data-dir PATH")?,
        max_connections: max_connections.unwrap_or(ServerConfig::default().max_connections),
        require_auth: true,
        auth_scheme: Some("Bearer".to_owned()),
        ..ServerConfig::default()
    })
}

enum ControlEvent {
    Frame(ManagedControl),
    Closed,
    Invalid,
}

fn input_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn read_controls(reader: &mut impl io::Read, sender: SyncSender<ControlEvent>) {
    loop {
        let event = match read_managed_frame::<ManagedControl>(reader) {
            Ok(Some(frame)) => ControlEvent::Frame(frame),
            Ok(None) => ControlEvent::Closed,
            Err(_) => ControlEvent::Invalid,
        };
        let finished = !matches!(event, ControlEvent::Frame(_));
        if sender.send(event).is_err() || finished {
            return;
        }
    }
}

fn start_control_reader() -> io::Result<Receiver<ControlEvent>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("nexum-managed-control".to_owned())
        .spawn(move || read_controls(&mut io::stdin().lock(), sender))?;
    Ok(receiver)
}

fn receive_start(receiver: &Receiver<ControlEvent>) -> io::Result<ManagedStartup> {
    let startup = match receiver.recv_timeout(STARTUP_TIMEOUT) {
        Ok(ControlEvent::Frame(ManagedControl::Start(startup))) => startup,
        Ok(ControlEvent::Closed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(input_error("managed control pipe closed before startup"));
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "managed startup deadline exceeded",
            ));
        }
        Ok(ControlEvent::Frame(ManagedControl::Shutdown { .. }) | ControlEvent::Invalid) => {
            return Err(input_error("managed startup frame is invalid"));
        }
    };
    startup
        .validate(env!("CARGO_PKG_VERSION"), RpcDispatcher::version())
        .map_err(|_| input_error("managed startup version or credential is invalid"))?;
    Ok(startup)
}

/// EOF and an explicit Shutdown both end the child. All other post-start input fails closed.
fn should_stop(receiver: &Receiver<ControlEvent>) -> io::Result<bool> {
    match receiver.try_recv() {
        Ok(ControlEvent::Frame(ManagedControl::Shutdown { control_version }))
            if control_version == MANAGED_CONTROL_VERSION =>
        {
            Ok(true)
        }
        Ok(ControlEvent::Closed) | Err(mpsc::TryRecvError::Disconnected) => Ok(true),
        Err(mpsc::TryRecvError::Empty) => Ok(false),
        Ok(ControlEvent::Frame(_) | ControlEvent::Invalid) => {
            Err(input_error("managed control frame is invalid"))
        }
    }
}

pub(super) fn run(args: &[String]) -> io::Result<()> {
    let mut config =
        parse_args(args).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let controls = start_control_reader()?;
    let startup = receive_start(&controls)?;
    config.auth_token = Some(startup.bearer_token);
    let authentication = AuthenticationConfig::from_server_config(&config)
        .map_err(|_| input_error("managed authentication configuration is invalid"))?;
    if should_stop(&controls)? {
        return Ok(());
    }

    let data_dir_lock = lock_data_dir(&config.data_dir)?;
    let state = Arc::new(Mutex::new(ServerState {
        core: open_core(&config.data_dir)?,
        _data_dir_lock: data_dir_lock,
        data_dir: std::fs::canonicalize(&config.data_dir)?,
        active_http: HashMap::new(),
        active_destinations: HashSet::new(),
        events: EventHub::default(),
        authentication,
        rate_limiter: None,
    }));
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let endpoint = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let connection_limiter = ConnectionLimiter::new(config.max_connections);

    spawn_event_pump(Arc::clone(&state));
    dispatch_available_http_tasks(&state);
    if should_stop(&controls)? {
        return Ok(());
    }
    let ready = ManagedReady {
        control_version: MANAGED_CONTROL_VERSION,
        server_version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_version: RpcDispatcher::version().to_string(),
        process_id: std::process::id(),
        endpoint,
    };
    write_managed_frame(&mut io::stdout().lock(), &ready)?;
    eprintln!("Nexum managed server is ready");

    loop {
        if should_stop(&controls)? {
            // Returning from main terminates all workers. Do not join network reads or
            // cancel/remove downloads; their persisted partials are recovered on restart.
            return Ok(());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // macOS inherits the listener's nonblocking mode on accepted sockets.
                stream.set_nonblocking(false)?;
                let Some(permit) = connection_limiter.try_acquire() else {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                };
                let state = Arc::clone(&state);
                let result = std::thread::Builder::new()
                    .name("nexum-managed-rpc".to_owned())
                    .spawn(move || {
                        let _permit = permit;
                        if let Err(error) = handle_connection(SharedStream::plain(stream), state) {
                            eprintln!("managed connection error: {error}");
                        }
                    });
                if let Err(error) = result {
                    eprintln!("could not start managed connection worker: {error}");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(CONTROL_POLL_INTERVAL);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn start() -> ManagedControl {
        ManagedControl::Start(ManagedStartup {
            control_version: MANAGED_CONTROL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: RpcDispatcher::version().to_string(),
            bearer_token: "a".repeat(64),
        })
    }

    #[test]
    fn valid_start_is_followed_by_control_pipe_eof() {
        let mut bytes = Vec::new();
        write_managed_frame(&mut bytes, &start()).unwrap();
        let (sender, receiver) = mpsc::sync_channel(2);
        read_controls(&mut Cursor::new(bytes), sender);
        receive_start(&receiver).unwrap();
        assert!(should_stop(&receiver).unwrap());
    }

    #[test]
    fn duplicate_start_and_version_mismatch_shutdown_fail_closed() {
        let (sender, receiver) = mpsc::sync_channel(2);
        sender.send(ControlEvent::Frame(start())).unwrap();
        assert!(should_stop(&receiver).is_err());
        sender
            .send(ControlEvent::Frame(ManagedControl::Shutdown {
                control_version: MANAGED_CONTROL_VERSION + 1,
            }))
            .unwrap();
        assert!(should_stop(&receiver).is_err());
    }

    #[test]
    fn malformed_control_does_not_include_frame_payload_in_error() {
        let secret = "private-token-do-not-log";
        let mut bytes = Vec::new();
        write_managed_frame(&mut bytes, &serde_json::json!({"type": secret})).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        read_controls(&mut Cursor::new(bytes), sender);
        let error = receive_start(&receiver).unwrap_err();
        assert!(!error.to_string().contains(secret));
    }
}
