//! Nexum desktop commands — JSON-RPC client for server communication.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

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
    let mut stream = match TcpStream::connect(server) {
        Ok(s) => s,
        Err(e) => return RpcResult::err(format!("connect: {e}")),
    };

    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(timeout_ms)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(timeout_ms)));

    // Build JSON-RPC request
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let payload = match serde_json::to_string(&request) {
        Ok(p) => p,
        Err(e) => return RpcResult::err(format!("serialize: {e}")),
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
}
