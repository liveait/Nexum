//! Nexum desktop commands — JSON-RPC client for server communication.

#[path = "credential_store.rs"]
mod credential_store;

pub use credential_store::{clear_credential, credential_status, save_credential};

use nexum_protocol::{Credential, RpcRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub enum JsonRpcError {
    #[serde(untagged)]
    ConnectError(String),
    #[serde(untagged)]
    RpcError { code: i32, message: String },
    #[serde(untagged)]
    Timeout(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcResult {
    pub success: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ServerEvent {
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub generation: u64,
    pub sequence: u64,
    pub event: String,
    pub data: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EventStreamStatus {
    pub server: String,
    pub generation: u64,
    pub connected: bool,
    pub error: Option<String>,
    pub resync_required: bool,
}

#[derive(Default)]
pub struct EventSubscriptionManager {
    next_generation: AtomicU64,
    active: Mutex<Option<ActiveSubscription>>,
}

struct ActiveSubscription {
    generation: u64,
    control: Arc<SubscriptionControl>,
}

/// Serializes stop with event emission so a stopped generation cannot publish
/// another status or notification after stop returns.
#[derive(Default)]
struct SubscriptionControl {
    stopped: AtomicBool,
    emission: Mutex<()>,
}

impl SubscriptionControl {
    fn stop(&self) {
        let _guard = self.emission.lock().expect("event emission mutex poisoned");
        self.stopped.store(true, Ordering::Release);
    }

    fn run_if_active(&self, action: impl FnOnce()) -> bool {
        let _guard = self.emission.lock().expect("event emission mutex poisoned");
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        action();
        true
    }

    fn emit_if_active<S>(&self, app: &AppHandle, event: &str, payload: S) -> bool
    where
        S: Serialize + Clone,
    {
        self.run_if_active(|| {
            let _ = app.emit(event, payload);
        })
    }
}

impl EventSubscriptionManager {
    pub fn start(&self, app: AppHandle, server: String) -> u64 {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let control = Arc::new(SubscriptionControl::default());
        let previous = self
            .active
            .lock()
            .expect("event subscription mutex poisoned")
            .replace(ActiveSubscription {
                generation,
                control: Arc::clone(&control),
            });
        if let Some(previous) = previous {
            previous.control.stop();
        }

        std::thread::Builder::new()
            .name("nexum-events".to_owned())
            .spawn(move || run_event_subscription(app, server, generation, control))
            .expect("could not start desktop event subscription");
        generation
    }

    pub fn stop(&self, generation: u64) {
        let mut active = self
            .active
            .lock()
            .expect("event subscription mutex poisoned");
        if active
            .as_ref()
            .is_some_and(|subscription| subscription.generation == generation)
            && let Some(subscription) = active.take()
        {
            subscription.control.stop();
        }
    }
}

const EVENT_CONNECT_RETRY_MIN: Duration = Duration::from_millis(250);
const EVENT_CONNECT_RETRY_MAX: Duration = Duration::from_secs(5);
const EVENT_READ_TIMEOUT: Duration = Duration::from_secs(1);

fn run_event_subscription(
    app: AppHandle,
    server: String,
    generation: u64,
    control: Arc<SubscriptionControl>,
) {
    let mut retry_delay = EVENT_CONNECT_RETRY_MIN;
    while !control.stopped.load(Ordering::Acquire) {
        match connect_event_stream(&server, &control.stopped) {
            Ok(mut reader) => {
                retry_delay = EVENT_CONNECT_RETRY_MIN;
                if !control.emit_if_active(
                    &app,
                    "server-event-status",
                    EventStreamStatus {
                        server: server.clone(),
                        generation,
                        connected: true,
                        error: None,
                        resync_required: false,
                    },
                ) {
                    return;
                }
                let disconnected_error;
                let mut resync_required = false;
                let mut last_sequence = None;
                loop {
                    if control.stopped.load(Ordering::Acquire) {
                        return;
                    }
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => {
                            disconnected_error = Some("event stream closed".to_owned());
                            break;
                        }
                        Ok(_) => {
                            if let Some(mut event) = parse_server_event(&line) {
                                if event.event == "events.heartbeat" {
                                    continue;
                                }
                                match record_event_sequence(&mut last_sequence, event.sequence) {
                                    Ok(true) => {
                                        event.server = server.clone();
                                        event.generation = generation;
                                        if !control.emit_if_active(&app, "server-event", event) {
                                            return;
                                        }
                                    }
                                    Ok(_) => {}
                                    Err((expected, received)) => {
                                        disconnected_error = Some(format!(
                                            "event sequence gap: expected {expected}, received {received}"
                                        ));
                                        resync_required = true;
                                        break;
                                    }
                                }
                            }
                        }
                        Err(error) if is_timeout(&error) => continue,
                        Err(error) => {
                            disconnected_error = Some(format!("event stream read failed: {error}"));
                            break;
                        }
                    }
                }
                if !control.emit_if_active(
                    &app,
                    "server-event-status",
                    EventStreamStatus {
                        server: server.clone(),
                        generation,
                        connected: false,
                        error: disconnected_error,
                        resync_required,
                    },
                ) {
                    return;
                }
            }
            Err(error) => {
                if !control.emit_if_active(
                    &app,
                    "server-event-status",
                    EventStreamStatus {
                        server: server.clone(),
                        generation,
                        connected: false,
                        error: Some(error),
                        resync_required: false,
                    },
                ) {
                    return;
                }
            }
        }

        if wait_for_stop(&control.stopped, retry_delay) {
            return;
        }
        retry_delay = std::cmp::min(retry_delay.saturating_mul(2), EVENT_CONNECT_RETRY_MAX);
    }
}

fn connect_event_stream(server: &str, stop: &AtomicBool) -> Result<BufReader<TcpStream>, String> {
    if stop.load(Ordering::Acquire) {
        return Err("event subscription stopped".to_owned());
    }
    let credential = credential_store::credential_for_request(server)?;
    let mut stream = TcpStream::connect(server).map_err(|error| format!("connect: {error}"))?;
    ensure_safe_credential_peer(&stream, credential.is_some())?;
    stream
        .set_read_timeout(Some(EVENT_READ_TIMEOUT))
        .map_err(|error| format!("read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(EVENT_READ_TIMEOUT))
        .map_err(|error| format!("write timeout: {error}"))?;

    let payload = encode_request("events.subscribe", None, credential)?;
    stream
        .write_all(payload.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|error| format!("write: {error}"))?;

    let mut reader = BufReader::new(stream);
    let ack = read_line_until_available(&mut reader, stop)?
        .ok_or_else(|| "event stream closed before acknowledgement".to_owned())?;
    let response: Value =
        serde_json::from_str(&ack).map_err(|error| format!("parse ack: {error}"))?;
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("event subscription rejected");
        return Err(message.to_owned());
    }
    if response
        .get("result")
        .and_then(|result| result.get("subscribed"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("server returned an invalid event subscription acknowledgement".to_owned());
    }
    Ok(reader)
}

fn read_line_until_available(
    reader: &mut BufReader<TcpStream>,
    stop: &AtomicBool,
) -> Result<Option<String>, String> {
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(None),
            Ok(_) => return Ok(Some(line)),
            Err(error) if is_timeout(&error) => continue,
            Err(error) => return Err(format!("read: {error}")),
        }
    }
}

fn parse_server_event(line: &str) -> Option<ServerEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("jsonrpc")?.as_str()? != "2.0" || value.get("method")?.as_str()? != "events.event"
    {
        return None;
    }
    let mut event: ServerEvent = serde_json::from_value(value.get("params")?.clone()).ok()?;
    event.server.clear();
    event.generation = 0;
    Some(event)
}

/// Accept only the first or next contiguous event sequence on one stream.
/// Duplicate and stale notifications are harmless; a forward gap means the
/// client must reconnect and refresh its full task snapshot because the server
/// does not provide replay.
fn record_event_sequence(
    last_sequence: &mut Option<u64>,
    sequence: u64,
) -> Result<bool, (u64, u64)> {
    let Some(previous) = *last_sequence else {
        *last_sequence = Some(sequence);
        return Ok(true);
    };
    if sequence == previous {
        return Ok(false);
    }
    if sequence == previous.saturating_add(1) {
        *last_sequence = Some(sequence);
        return Ok(true);
    }
    if sequence > previous {
        return Err((previous.saturating_add(1), sequence));
    }
    Ok(false)
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

fn wait_for_stop(stop: &AtomicBool, duration: Duration) -> bool {
    let mut remaining = duration;
    while !remaining.is_zero() {
        if stop.load(Ordering::Acquire) {
            return true;
        }
        let slice = std::cmp::min(remaining, Duration::from_millis(50));
        std::thread::sleep(slice);
        remaining = remaining.saturating_sub(slice);
    }
    stop.load(Ordering::Acquire)
}

fn encode_request(
    method: &str,
    params: Option<Value>,
    credential: Option<Credential>,
) -> Result<String, String> {
    let mut request = RpcRequest::new(1, method, params);
    if let Some(credential) = credential {
        request = request.with_credential(credential);
    }
    serde_json::to_string(&request).map_err(|_| "could not serialize RPC request".to_owned())
}

fn peer_is_safe_for_credential(has_credential: bool, peer: SocketAddr) -> bool {
    !has_credential || peer.ip().is_loopback()
}

fn ensure_safe_credential_peer(stream: &TcpStream, has_credential: bool) -> Result<(), String> {
    if !has_credential {
        return Ok(());
    }
    let peer = stream
        .peer_addr()
        .map_err(|_| "could not verify credential destination".to_owned())?;
    if peer_is_safe_for_credential(has_credential, peer) {
        Ok(())
    } else {
        Err("credential can only be sent to a loopback server".to_owned())
    }
}

impl RpcResult {
    pub fn ok(result: Value) -> Self {
        Self {
            success: true,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(message: String) -> Self {
        Self {
            success: false,
            result: None,
            error: Some(message),
        }
    }

    fn into_result(self) -> Result<Value, String> {
        if self.success {
            self.result
                .ok_or_else(|| "server returned no result".to_owned())
        } else {
            Err(self
                .error
                .unwrap_or_else(|| "unknown server error".to_owned()))
        }
    }
}

/// Call a JSON-RPC method on the server.
pub fn call_rpc(server: &str, method: &str, params: Option<Value>, timeout_ms: u64) -> RpcResult {
    let credential = match credential_store::credential_for_request(server) {
        Ok(credential) => credential,
        Err(error) => return RpcResult::err(error),
    };
    let mut stream = match TcpStream::connect(server) {
        Ok(s) => s,
        Err(e) => return RpcResult::err(format!("connect: {e}")),
    };
    if let Err(error) = ensure_safe_credential_peer(&stream, credential.is_some()) {
        return RpcResult::err(error);
    }

    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(timeout_ms)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(timeout_ms)));

    let payload = match encode_request(method, params, credential) {
        Ok(p) => p,
        Err(e) => return RpcResult::err(e),
    };

    // Send request
    if let Err(e) = stream.write_all(payload.as_bytes()) {
        return RpcResult::err(format!("write: {e}"));
    }
    if let Err(e) = stream.write_all(b"\n") {
        return RpcResult::err(format!("newline: {e}"));
    }
    if let Err(e) = stream.flush() {
        return RpcResult::err(format!("flush: {e}"));
    }

    // Read response
    let reader = BufReader::new(stream);
    let line = match reader.lines().next() {
        Some(Ok(l)) => l,
        Some(Err(e)) => return RpcResult::err(format!("read: {e}")),
        None => return RpcResult::err("no response".to_owned()),
    };

    // Parse response
    match serde_json::from_str::<serde_json::Value>(&line) {
        Ok(response) => {
            if let Some(err) = response.get("error") {
                if let Some(code) = err.get("code").and_then(|c| c.as_i64()) {
                    let message = err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error");
                    RpcResult::err(format!("[{}] {}", code, message))
                } else {
                    RpcResult::err("missing error code".to_owned())
                }
            } else {
                RpcResult::ok(response.get("result").cloned().unwrap_or(Value::Null))
            }
        }
        Err(e) => RpcResult::err(format!("parse: {e}")),
    }
}

/// Call server.version and return the protocol version string.
pub fn server_version(server: &str, timeout_ms: u64) -> Result<String, String> {
    call_rpc(server, "server.version", None, timeout_ms)
        .into_result()?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "server returned an invalid protocol version".to_owned())
}

/// Call task.list and return task items.
pub fn task_list(server: &str, timeout_ms: u64) -> Result<Vec<Value>, String> {
    match call_rpc(server, "task.list", None, timeout_ms).into_result()? {
        Value::Array(tasks) => Ok(tasks),
        _ => Err("server returned an invalid task list".to_owned()),
    }
}

/// Call task.get and return a single task.
pub fn task_get(server: &str, task_id: &str, timeout_ms: u64) -> Result<Value, String> {
    let params = serde_json::json!({"id": task_id});
    call_rpc(server, "task.get", Some(params), timeout_ms).into_result()
}

/// Call task.create and return the created task.
pub fn task_create(
    server: &str,
    id: &str,
    source: &str,
    destination: &str,
    timeout_ms: u64,
) -> Result<Value, String> {
    let params = serde_json::json!({
        "id": id,
        "source": source,
        "destination": destination,
    });
    call_rpc(server, "task.create", Some(params), timeout_ms).into_result()
}

/// Call task.queue to queue a task.
pub fn task_queue(server: &str, task_id: &str, timeout_ms: u64) -> Result<bool, String> {
    let params = serde_json::json!({"id": task_id});
    expect_bool(call_rpc(server, "task.queue", Some(params), timeout_ms))
}

/// Call task.start to start the next queued task.
pub fn task_start(server: &str, timeout_ms: u64) -> Result<String, String> {
    call_rpc(server, "task.start", None, timeout_ms)
        .into_result()?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "server returned an invalid task ID".to_owned())
}

/// Call task.pause to pause a task.
pub fn task_pause(server: &str, task_id: &str, timeout_ms: u64) -> Result<bool, String> {
    let params = serde_json::json!({"id": task_id});
    expect_bool(call_rpc(server, "task.pause", Some(params), timeout_ms))
}

/// Call task.resume to resume a task.
pub fn task_resume(server: &str, task_id: &str, timeout_ms: u64) -> Result<bool, String> {
    let params = serde_json::json!({"id": task_id});
    expect_bool(call_rpc(server, "task.resume", Some(params), timeout_ms))
}

/// Call task.remove to remove a task.
pub fn task_remove(server: &str, task_id: &str, timeout_ms: u64) -> Result<bool, String> {
    let params = serde_json::json!({"id": task_id});
    expect_bool(call_rpc(server, "task.remove", Some(params), timeout_ms))
}

fn expect_bool(result: RpcResult) -> Result<bool, String> {
    result
        .into_result()?
        .as_bool()
        .ok_or_else(|| "server returned an invalid boolean result".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_result_ok_serializes() {
        let result = RpcResult::ok(serde_json::json!({"tasks": []}));
        assert!(result.success);
        assert!(!result.error.is_some());
    }

    #[test]
    fn rpc_result_err_serializes() {
        let result = RpcResult::err("test error".to_owned());
        assert!(!result.success);
        assert_eq!(result.error, Some("test error".to_owned()));
    }

    #[test]
    fn call_timeout_configures_stream() {
        // We can't actually test with a real server without one running,
        // but this test verifies the function signature is correct.
        let _ = call_rpc("127.0.0.1:1", "server.version", None, 1000);
    }

    #[test]
    fn parses_event_notification_and_ignores_other_lines() {
        let line = r#"{"jsonrpc":"2.0","method":"events.event","params":{"server":"forged","generation":999,"sequence":4,"event":"task.created","data":{"task_id":"t1"}}}"#;
        let event = parse_server_event(line).expect("event notification should parse");
        assert_eq!(event.server, "");
        assert_eq!(event.generation, 0);
        assert_eq!(event.sequence, 4);
        assert_eq!(event.event, "task.created");
        assert_eq!(event.data["task_id"], "t1");
        assert!(parse_server_event(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).is_none());
    }

    #[test]
    fn event_sequence_requires_contiguous_forward_progress() {
        let mut last = None;
        assert_eq!(record_event_sequence(&mut last, 4), Ok(true));
        assert_eq!(record_event_sequence(&mut last, 4), Ok(false));
        assert_eq!(record_event_sequence(&mut last, 5), Ok(true));
        assert_eq!(record_event_sequence(&mut last, 3), Ok(false));
        assert_eq!(record_event_sequence(&mut last, 7), Err((6, 7)));
    }

    #[test]
    fn ordinary_and_event_requests_encode_protocol_credentials() {
        let task = encode_request(
            "task.list",
            None,
            Some(Credential::Bearer {
                token: "test-token".to_owned(),
            }),
        )
        .unwrap();
        let task: Value = serde_json::from_str(&task).unwrap();
        assert_eq!(task["method"], "task.list");
        assert_eq!(task["credential"]["Bearer"]["token"], "test-token");

        let event = encode_request(
            "events.subscribe",
            None,
            Some(Credential::ApiKey {
                key: "test-key".to_owned(),
            }),
        )
        .unwrap();
        let event: Value = serde_json::from_str(&event).unwrap();
        assert_eq!(event["method"], "events.subscribe");
        assert_eq!(event["credential"]["ApiKey"]["key"], "test-key");

        let anonymous: Value =
            serde_json::from_str(&encode_request("server.version", None, None).unwrap()).unwrap();
        assert!(anonymous["credential"].is_null());
    }

    #[test]
    fn credential_destination_must_be_an_actual_loopback_peer() {
        assert!(peer_is_safe_for_credential(
            true,
            "127.4.5.6:39100".parse().unwrap()
        ));
        assert!(peer_is_safe_for_credential(
            true,
            "[::1]:39100".parse().unwrap()
        ));
        assert!(!peer_is_safe_for_credential(
            true,
            "192.168.1.5:39100".parse().unwrap()
        ));
        assert!(peer_is_safe_for_credential(
            false,
            "192.168.1.5:39100".parse().unwrap()
        ));
    }

    #[test]
    fn stopped_subscription_rejects_later_emission() {
        let control = SubscriptionControl::default();
        let emitted = std::cell::Cell::new(0);
        assert!(control.run_if_active(|| emitted.set(emitted.get() + 1)));
        control.stop();
        assert!(!control.run_if_active(|| emitted.set(emitted.get() + 1)));
        assert_eq!(emitted.get(), 1);
    }

    #[test]
    fn event_status_serializes_local_generation() {
        let status = EventStreamStatus {
            server: "localhost:39100".to_owned(),
            generation: 7,
            connected: true,
            error: None,
            resync_required: false,
        };
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["generation"], 7);
    }
}
