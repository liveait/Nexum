//! Nexum JSON-RPC 2.0 protocol primitives.

pub use nexum_security::{
    AuthenticationError, AuthenticationScheme, Credential, RateLimit, TlsConfig,
};

use nexum_core::Core;
use nexum_domain::{Destination, DownloadSource, TaskId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const JSONRPC_VERSION: &str = "2.0";

/// Protocol version negotiated between client and server.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum ProtocolVersion {
    #[default]
    V1,
}

impl ProtocolVersion {
    pub const CURRENT: Self = Self::V1;
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "1",
        }
    }
}

impl std::fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub const ERR_TASK_NOT_FOUND: i32 = -32004;
pub const ERR_UNAUTHORIZED: i32 = -32001;
pub const ERR_RATE_LIMITED: i32 = -32002;
pub const ERR_INTERNAL: i32 = -32603;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RpcRequest {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    /// Client protocol version. Defaults to V1 for backward compatibility.
    #[serde(default)]
    pub version: Option<ProtocolVersion>,
    /// Optional credential attached to the request.
    #[serde(default)]
    pub credential: Option<Credential>,
}
impl RpcRequest {
    pub fn new(id: impl Into<Value>, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(id.into()),
            method: method.into(),
            params,
            version: None,
            credential: None,
        }
    }
    pub fn with_credential(mut self, credential: Credential) -> Self {
        self.credential = Some(credential);
        self
    }
    pub fn notification(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id: None,
            method: method.into(),
            params,
            version: None,
            credential: None,
        }
    }
    pub fn validate(&self) -> Result<(), RpcError> {
        if self.jsonrpc != JSONRPC_VERSION {
            return Err(RpcError::InvalidRequest("jsonrpc must be 2.0".into()));
        }
        if self.method.trim().is_empty() {
            return Err(RpcError::InvalidRequest("method must not be empty".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorObject>,
}
impl RpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn error(id: Option<Value>, error: RpcErrorObject) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: None,
            error: Some(error),
        }
    }
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RpcErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}
impl RpcErrorObject {
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self {
            code: -32700,
            message: message.into(),
            data: None,
        }
    }
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            data: None,
        }
    }
    pub fn method_not_found(method: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: format!("method not found: {}", method.into()),
            data: None,
        }
    }
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }
    pub fn task_not_found(message: impl Into<String>) -> Self {
        Self {
            code: ERR_TASK_NOT_FOUND,
            message: message.into(),
            data: None,
        }
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            code: ERR_UNAUTHORIZED,
            message: message.into(),
            data: None,
        }
    }
    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self {
            code: ERR_RATE_LIMITED,
            message: message.into(),
            data: None,
        }
    }
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self {
            code: ERR_INTERNAL,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpcError {
    Parse(String),
    InvalidRequest(String),
}
impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "JSON-RPC parse error: {message}"),
            Self::InvalidRequest(message) => write!(f, "invalid JSON-RPC request: {message}"),
        }
    }
}
impl std::error::Error for RpcError {}
pub fn parse_request(input: &str) -> Result<RpcRequest, RpcError> {
    let request: RpcRequest =
        serde_json::from_str(input).map_err(|error| RpcError::Parse(error.to_string()))?;
    request.validate()?;
    Ok(request)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct TaskView {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub state: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
}
impl From<&nexum_task::DownloadTask> for TaskView {
    fn from(task: &nexum_task::DownloadTask) -> Self {
        Self {
            id: task.id.to_string(),
            source: task.source.as_str().to_owned(),
            destination: task.destination.as_str().to_owned(),
            state: format!("{:?}", task.state),
            downloaded_bytes: task.progress.downloaded_bytes,
            total_bytes: task.progress.total_bytes,
            error: task.last_error.clone(),
        }
    }
}

pub struct RpcDispatcher;
impl RpcDispatcher {
    /// Returns the protocol version string supported by this server.
    pub fn version() -> &'static str {
        ProtocolVersion::CURRENT.as_str()
    }

    /// Returns the authentication schemes supported by this server.
    pub fn auth_schemes() -> &'static [&'static str] {
        &["none"]
    }

    pub fn dispatch<R: nexum_core::nexum_storage::TaskRepository>(
        core: &mut Core<R>,
        request: &RpcRequest,
    ) -> Option<RpcResponse> {
        let id = request.id.clone();
        let params = request.params.clone().unwrap_or(Value::Null);
        let result = match request.method.as_str() {
            "task.get" => Self::task_get(core, &params),
            "task.list" => Self::task_list(core),
            "task.create" => Self::task_create(core, &params),
            "task.queue" => Self::task_queue(core, &params),
            "task.start" => Self::task_start(core),
            "task.pause" => Self::task_pause(core, &params),
            "task.resume" => Self::task_resume(core, &params),
            "task.remove" => Self::task_remove(core, &params),
            "server.version" => Self::server_version(),
            "server.auth" => Self::server_auth(),
            _ => Err(DispatchError::MethodNotFound),
        };
        let id = id.as_ref().cloned()?;
        Some(match result {
            Ok(value) => RpcResponse::success(Some(id), value),
            Err(DispatchError::InvalidParams(message)) => {
                RpcResponse::error(Some(id), RpcErrorObject::invalid_params(message))
            }
            Err(DispatchError::TaskNotFound(message)) => {
                RpcResponse::error(Some(id), RpcErrorObject::task_not_found(message))
            }
            Err(DispatchError::Internal(message)) => {
                RpcResponse::error(Some(id), RpcErrorObject::internal_error(message))
            }
            Err(DispatchError::MethodNotFound) => {
                RpcResponse::error(Some(id), RpcErrorObject::method_not_found(&request.method))
            }
        })
    }

    fn server_version() -> Result<Value, DispatchError> {
        Ok(Value::String(RpcDispatcher::version().to_owned()))
    }

    fn server_auth() -> Result<Value, DispatchError> {
        let schemes: Vec<String> = RpcDispatcher::auth_schemes()
            .iter()
            .map(|&s| s.to_owned())
            .collect();
        serde_json::to_value(schemes).map_err(|e| DispatchError::Internal(e.to_string()))
    }

    fn task_get<R: nexum_core::nexum_storage::TaskRepository>(
        core: &Core<R>,
        params: &Value,
    ) -> Result<Value, DispatchError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DispatchError::InvalidParams("missing id".into()))?;
        let task = core
            .tasks
            .get(&TaskId::from(id))
            .ok_or_else(|| DispatchError::TaskNotFound("task not found".into()))?;
        serde_json::to_value(TaskView::from(task))
            .map_err(|e| DispatchError::Internal(e.to_string()))
    }
    fn task_list<R: nexum_core::nexum_storage::TaskRepository>(
        core: &Core<R>,
    ) -> Result<Value, DispatchError> {
        let tasks: Vec<TaskView> = core.tasks.list().map(TaskView::from).collect();
        serde_json::to_value(tasks).map_err(|e| DispatchError::Internal(e.to_string()))
    }
    fn task_create<R: nexum_core::nexum_storage::TaskRepository>(
        core: &mut Core<R>,
        params: &Value,
    ) -> Result<Value, DispatchError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DispatchError::InvalidParams("missing id".into()))?;
        let source = params
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| DispatchError::InvalidParams("missing source".into()))?;
        let destination = params
            .get("destination")
            .and_then(Value::as_str)
            .ok_or_else(|| DispatchError::InvalidParams("missing destination".into()))?;
        let task = core
            .create_task(
                TaskId::from(id),
                DownloadSource::new(source),
                Destination::new(destination),
            )
            .map_err(|e| DispatchError::Internal(format!("{e:?}")))?;
        serde_json::to_value(TaskView::from(&task))
            .map_err(|e| DispatchError::Internal(e.to_string()))
    }
    fn task_queue<R: nexum_core::nexum_storage::TaskRepository>(
        core: &mut Core<R>,
        params: &Value,
    ) -> Result<Value, DispatchError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DispatchError::InvalidParams("missing id".into()))?;
        core.queue_task(
            &TaskId::from(id),
            nexum_core::nexum_scheduler::Priority::NORMAL,
        )
        .map_err(|e| match &e {
            nexum_core::CoreError::Scheduler(
                nexum_core::nexum_scheduler::SchedulerError::Task(
                    nexum_task::TaskServiceError::NotFound(_),
                ),
            ) => DispatchError::TaskNotFound("task not found".into()),
            _ => DispatchError::Internal(format!("{e:?}")),
        })?;
        Ok(Value::Bool(true))
    }
    fn task_start<R: nexum_core::nexum_storage::TaskRepository>(
        core: &mut Core<R>,
    ) -> Result<Value, DispatchError> {
        let id = core
            .start_next()
            .map_err(|e| DispatchError::Internal(format!("{e:?}")))?
            .ok_or_else(|| DispatchError::TaskNotFound("no queued task".into()))?;
        Ok(Value::String(id.to_string()))
    }
    fn task_pause<R: nexum_core::nexum_storage::TaskRepository>(
        core: &mut Core<R>,
        params: &Value,
    ) -> Result<Value, DispatchError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DispatchError::InvalidParams("missing id".into()))?;
        core.pause_task(&TaskId::from(id))
            .map_err(|e| DispatchError::Internal(format!("{e:?}")))?;
        Ok(Value::Bool(true))
    }
    fn task_resume<R: nexum_core::nexum_storage::TaskRepository>(
        core: &mut Core<R>,
        params: &Value,
    ) -> Result<Value, DispatchError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DispatchError::InvalidParams("missing id".into()))?;
        Ok(Value::Bool(
            core.resume_task(&TaskId::from(id))
                .map_err(|e| DispatchError::Internal(format!("{e:?}")))?,
        ))
    }
    fn task_remove<R: nexum_core::nexum_storage::TaskRepository>(
        core: &mut Core<R>,
        params: &Value,
    ) -> Result<Value, DispatchError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DispatchError::InvalidParams("missing id".into()))?;
        core.remove_task(&TaskId::from(id))
            .map_err(|e| DispatchError::Internal(format!("{e:?}")))?;
        Ok(Value::Bool(true))
    }
}

enum DispatchError {
    InvalidParams(String),
    TaskNotFound(String),
    Internal(String),
    MethodNotFound,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EventEnvelope {
    pub event: String,
    pub data: Value,
}
impl EventEnvelope {
    pub fn new(event: impl Into<String>, data: Value) -> Self {
        Self {
            event: event.into(),
            data,
        }
    }
}

/// A JSON-RPC notification sent on a subscribed event connection.
///
/// Event notifications intentionally carry incremental data. Clients should
/// refresh their task snapshot after receiving one instead of trying to build a
/// complete TaskView from the event payload.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EventNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: EventNotificationParams,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EventNotificationParams {
    pub sequence: u64,
    pub event: String,
    pub data: Value,
}

impl EventNotification {
    pub const METHOD: &'static str = "events.event";

    pub fn new(sequence: u64, event: EventEnvelope) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            method: Self::METHOD.into(),
            params: EventNotificationParams {
                sequence,
                event: event.event,
                data: event.data,
            },
        }
    }
}

pub fn serialize_response(response: &RpcResponse) -> Result<String, RpcError> {
    serde_json::to_string(response).map_err(|error| RpcError::Parse(error.to_string()))
}

pub fn task_event_to_envelope(event: &nexum_task::TaskEvent) -> EventEnvelope {
    use nexum_task::TaskEvent;
    match event {
        TaskEvent::Created { task_id } => EventEnvelope::new(
            "task.created",
            serde_json::json!({"task_id": task_id.to_string()}),
        ),
        TaskEvent::StateChanged { task_id, from, to } => EventEnvelope::new(
            "task.state_changed",
            serde_json::json!({"task_id": task_id.to_string(), "from": format!("{from:?}"), "to": format!("{to:?}")}),
        ),
        TaskEvent::ProgressChanged { task_id, progress } => EventEnvelope::new(
            "task.progress",
            serde_json::json!({"task_id": task_id.to_string(), "downloaded_bytes": progress.downloaded_bytes, "total_bytes": progress.total_bytes, "speed_bytes_per_second": progress.speed_bytes_per_second, "eta_seconds": progress.eta_seconds}),
        ),
        TaskEvent::Removed { task_id } => EventEnvelope::new(
            "task.removed",
            serde_json::json!({"task_id": task_id.to_string()}),
        ),
    }
}

pub fn scheduler_event_to_envelope(
    event: &nexum_core::nexum_scheduler::SchedulerEvent,
) -> EventEnvelope {
    use nexum_core::nexum_scheduler::SchedulerEvent;
    match event {
        SchedulerEvent::Enqueued { task_id, priority } => EventEnvelope::new(
            "scheduler.enqueued",
            serde_json::json!({"task_id": task_id.to_string(), "priority": priority.value()}),
        ),
        SchedulerEvent::Started { task_id } => EventEnvelope::new(
            "scheduler.started",
            serde_json::json!({"task_id": task_id.to_string()}),
        ),
        SchedulerEvent::Paused { task_id } => EventEnvelope::new(
            "scheduler.paused",
            serde_json::json!({"task_id": task_id.to_string()}),
        ),
        SchedulerEvent::Resumed { task_id } => EventEnvelope::new(
            "scheduler.resumed",
            serde_json::json!({"task_id": task_id.to_string()}),
        ),
        SchedulerEvent::Completed { task_id } => EventEnvelope::new(
            "scheduler.completed",
            serde_json::json!({"task_id": task_id.to_string()}),
        ),
        SchedulerEvent::Failed { task_id } => EventEnvelope::new(
            "scheduler.failed",
            serde_json::json!({"task_id": task_id.to_string()}),
        ),
        SchedulerEvent::Retrying { task_id, attempt } => EventEnvelope::new(
            "scheduler.retrying",
            serde_json::json!({"task_id": task_id.to_string(), "attempt": attempt}),
        ),
    }
}

#[derive(Default)]
pub struct EventBuffer {
    events: Vec<EventEnvelope>,
}
impl EventBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn collect_core<R: nexum_core::nexum_storage::TaskRepository>(
        &mut self,
        core: &mut Core<R>,
    ) {
        self.events.extend(
            core.drain_task_events()
                .into_iter()
                .map(|e| task_event_to_envelope(&e)),
        );
        self.events.extend(
            core.drain_scheduler_events()
                .into_iter()
                .map(|e| scheduler_event_to_envelope(&e)),
        );
    }

    pub fn push(&mut self, event: EventEnvelope) {
        self.events.push(event);
    }

    pub fn drain(&mut self) -> Vec<EventEnvelope> {
        std::mem::take(&mut self.events)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexum_core::Core;
    use nexum_domain::{Destination, DownloadSource};
    use serde_json::json;

    #[test]
    fn parses_json_rpc_request() {
        let request =
            parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"task.list","params":{"limit":10}}"#)
                .unwrap();
        assert_eq!(request.method, "task.list");
        assert_eq!(request.id, Some(json!(1)));
    }
    #[test]
    fn parses_request_without_version_defaults_to_v1() {
        let request = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"task.list"}"#).unwrap();
        assert_eq!(request.version, None);
    }
    #[test]
    fn parses_request_with_explicit_version() {
        let request =
            parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"task.list","version":"V1"}"#)
                .unwrap();
        assert_eq!(request.version, Some(ProtocolVersion::V1));
    }
    #[test]
    fn parses_request_with_bearer_credential() {
        let request = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"task.list","credential":{"Bearer":{"token":"abc123"}}}"#).unwrap();
        match request.credential {
            Some(Credential::Bearer { token }) => assert_eq!(token, "abc123"),
            _ => panic!("expected Bearer credential"),
        }
    }
    #[test]
    fn parses_request_with_apikey_credential() {
        let request = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"task.list","credential":{"ApiKey":{"key":"my-key"}}}"#).unwrap();
        match request.credential {
            Some(Credential::ApiKey { key }) => assert_eq!(key, "my-key"),
            _ => panic!("expected ApiKey credential"),
        }
    }
    #[test]
    fn parses_request_with_none_credential() {
        let request = parse_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"task.list","credential":{"None":null}}"#,
        )
        .unwrap();
        assert_eq!(request.credential, Some(Credential::None));
    }
    #[test]
    fn rejects_wrong_protocol_version() {
        let error = parse_request(r#"{"jsonrpc":"1.0","id":1,"method":"task.list"}"#).unwrap_err();
        assert!(matches!(error, RpcError::InvalidRequest(_)));
    }
    #[test]
    fn serializes_success_response() {
        let response = RpcResponse::success(Some(json!(1)), json!({"tasks":[]}));
        let encoded = serialize_response(&response).unwrap();
        assert!(encoded.contains(r#""jsonrpc":"2.0""#));
        assert!(encoded.contains(r#""result":{"tasks":[]}"#));
    }
    #[test]
    fn notifications_have_no_response() {
        let request = RpcRequest::notification("task.list", None);
        let mut core = Core::new(nexum_scheduler::SchedulerConfig::default()).unwrap();
        assert!(RpcDispatcher::dispatch(&mut core, &request).is_none());
    }
    #[test]
    fn server_version_returns_current() {
        assert_eq!(RpcDispatcher::version(), "1");
    }
    #[test]
    fn server_version_method_works() {
        let mut core = Core::new(nexum_scheduler::SchedulerConfig::default()).unwrap();
        let request = RpcRequest::new(1, "server.version", None);
        let response = RpcDispatcher::dispatch(&mut core, &request).unwrap();
        assert!(response.is_success());
        assert_eq!(
            response.result.as_ref().and_then(Value::as_str).unwrap(),
            json!("1")
        );
    }
    #[test]
    fn server_auth_returns_supported_schemes() {
        let mut core = Core::new(nexum_scheduler::SchedulerConfig::default()).unwrap();
        let request = RpcRequest::new(1, "server.auth", None);
        let response = RpcDispatcher::dispatch(&mut core, &request).unwrap();
        assert!(response.is_success());
        let result = response.result.unwrap();
        let schemes: Vec<&str> = result
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(schemes, vec!["none"]);
    }

    #[test]
    fn task_view_exposes_last_transfer_error() {
        let mut core = Core::new(nexum_scheduler::SchedulerConfig::default()).unwrap();
        let id = TaskId::from("failed-task");
        core.create_task(
            id.clone(),
            DownloadSource::new("https://example.com/file"),
            Destination::new("/tmp/file"),
        )
        .unwrap();
        core.queue_task(&id, nexum_scheduler::Priority::NORMAL)
            .unwrap();
        core.claim_next().unwrap();
        core.finish_task_with_error(&id, nexum_task::TaskState::Failed, "connection refused")
            .unwrap();

        let request = RpcRequest::new(1, "task.get", Some(json!({"id": "failed-task"})));
        let response = RpcDispatcher::dispatch(&mut core, &request).unwrap();
        assert_eq!(response.result.unwrap()["error"], "connection refused");

        let request = RpcRequest::new(2, "task.list", None);
        let response = RpcDispatcher::dispatch(&mut core, &request).unwrap();
        assert_eq!(response.result.unwrap()[0]["error"], "connection refused");
    }
    #[test]
    fn unknown_method_is_reported() {
        let request = RpcRequest::new(1, "task.unknown", None);
        let mut core = Core::new(nexum_scheduler::SchedulerConfig::default()).unwrap();
        let response = RpcDispatcher::dispatch(&mut core, &request).unwrap();
        assert_eq!(response.error.unwrap().code, -32601);
    }
    #[test]
    fn scheduler_event_maps_to_stable_envelope() {
        let event = nexum_core::nexum_scheduler::SchedulerEvent::Retrying {
            task_id: TaskId::from("t1"),
            attempt: 2,
        };
        let envelope = scheduler_event_to_envelope(&event);
        assert_eq!(envelope.event, "scheduler.retrying");
        assert_eq!(envelope.data["attempt"], 2);
    }

    #[test]
    fn event_buffer_collects_core_events() {
        let mut core = Core::new(nexum_scheduler::SchedulerConfig::default()).unwrap();
        core.create_task(
            TaskId::from("t1"),
            DownloadSource::new("https://example.com/file"),
            Destination::new("/tmp/file"),
        )
        .unwrap();
        let mut buffer = EventBuffer::new();
        buffer.collect_core(&mut core);
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.drain()[0].event, "task.created");
    }

    #[test]
    fn event_notification_serializes_as_json_rpc_notification() {
        let notification = EventNotification::new(
            7,
            EventEnvelope::new("task.created", json!({"task_id": "t1"})),
        );
        let value = serde_json::to_value(notification).unwrap();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["method"], "events.event");
        assert_eq!(value["params"]["sequence"], 7);
        assert_eq!(value["params"]["event"], "task.created");
        assert_eq!(value["params"]["data"]["task_id"], "t1");
    }

    #[test]
    fn task_event_maps_to_stable_envelope() {
        let event = nexum_task::TaskEvent::Created {
            task_id: TaskId::from("t1"),
        };
        let envelope = task_event_to_envelope(&event);
        assert_eq!(envelope.event, "task.created");
        assert_eq!(envelope.data["task_id"], "t1");
    }

    #[test]
    fn protocol_version_serializes_and_deserializes() {
        let v = ProtocolVersion::V1;
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"V1\"");
        let parsed: ProtocolVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ProtocolVersion::V1);
    }

    #[test]
    fn protocol_version_display() {
        assert_eq!(format!("{}", ProtocolVersion::V1), "1");
    }

    #[test]
    fn rpc_request_with_credential_builds_correctly() {
        let request = RpcRequest::new(1, "task.list", None).with_credential(Credential::Bearer {
            token: "secret".into(),
        });
        match request.credential {
            Some(Credential::Bearer { token }) => assert_eq!(token, "secret"),
            _ => panic!("expected Bearer credential"),
        }
    }

    #[test]
    fn authentication_scheme_from_header() {
        assert_eq!(
            AuthenticationScheme::from_header("Bearer token123"),
            AuthenticationScheme::Bearer("token123".into())
        );
        assert_eq!(
            AuthenticationScheme::from_header("ApiKey key456"),
            AuthenticationScheme::ApiKey("key456".into())
        );
        assert_eq!(
            AuthenticationScheme::from_header(""),
            AuthenticationScheme::None
        );
    }
}
