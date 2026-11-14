//! Server-owned origin credentials. Provider operations never hold ServerState.
use super::*;
use nexum_core::{
    nexum_domain::{Destination, DownloadSource},
    nexum_storage::{
        StoredTask,
        origin::{CredentialRef, OriginRecord, Reservation, random_id},
    },
    nexum_task::DownloadTask,
};
use nexum_protocol::{
    TaskView,
    transfer::{CreateTransferRequest, OriginSecret},
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderError {
    Missing,
    Locked,
    Denied,
    Unavailable,
    Exists,
}

pub trait CredentialProvider: Send + Sync {
    fn durable(&self) -> bool;
    // Failed atomic adds create no item and never change an existing item.
    fn put(&self, reference: &CredentialRef, secret: &OriginSecret) -> Result<(), ProviderError>;
    fn get(&self, reference: &CredentialRef) -> Result<OriginSecret, ProviderError>;
    fn remove(&self, reference: &CredentialRef) -> Result<(), ProviderError>;
}

pub struct Coordinator {
    mutation: Mutex<()>,
    provider: Arc<dyn CredentialProvider>,
    instance_id: String,
}

fn internal() -> RpcErrorObject {
    RpcErrorObject::internal_error("origin storage operation failed")
}
fn conflict() -> RpcErrorObject {
    RpcErrorObject {
        code: -32009,
        message: "transfer id conflicts with existing input".into(),
        data: None,
    }
}
fn provider_error(error: ProviderError) -> RpcErrorObject {
    let message = match error {
        ProviderError::Missing => "origin credential missing; explicitly reimport the task",
        ProviderError::Locked => "origin credential locked; unlock Keychain and retry",
        ProviderError::Denied => "origin credential access denied; grant access and retry",
        ProviderError::Unavailable => "origin credential provider unavailable",
        ProviderError::Exists => "origin credential reference already exists",
    };
    RpcErrorObject::internal_error(message)
}

impl Coordinator {
    pub fn production(repository: &SqliteRepository) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            mutation: Mutex::new(()),
            provider: Arc::new(SystemProvider),
            instance_id: repository
                .instance_id()
                .map_err(|_| io::Error::other("origin identity unavailable"))?,
        }))
    }

    pub fn create(
        &self,
        state: &Arc<Mutex<ServerState>>,
        params: &Value,
        tls: bool,
    ) -> Result<Value, RpcErrorObject> {
        let input = CreateTransferRequest::parse(params)?;
        if input.secret.is_some() && !tls {
            return Err(RpcErrorObject::invalid_params(
                "password credentials require TLS",
            ));
        }
        if input.secret.is_some() && !self.provider.durable() {
            return Err(provider_error(ProviderError::Unavailable));
        }
        let _mutation = self
            .mutation
            .lock()
            .expect("origin mutation mutex poisoned");
        let mut record = input.origin;
        record.operation_id = random_id().map_err(|_| internal())?;
        if input.secret.is_some() {
            record.credential = Some(CredentialRef {
                instance_id: self.instance_id.clone(),
                origin: record.origin.clone(),
                credential_id: random_id().map_err(|_| internal())?,
                revision: 1,
            });
        }
        let reservation = state
            .lock()
            .expect("server state mutex poisoned")
            .core
            .repository
            .reserve_origin(&record)
            .map_err(|_| internal())?;
        match reservation {
            Reservation::Existing(existing) => {
                if !record.same_input(&existing) {
                    return Err(conflict());
                }
                if let Some(secret) = input.secret.as_ref() {
                    let reference = existing
                        .credential
                        .as_ref()
                        .filter(|reference| self.owns(&existing, reference))
                        .ok_or_else(internal)?;
                    if self.provider.get(reference).map_err(provider_error)? != *secret {
                        return Err(conflict());
                    }
                }
                let locked = state.lock().expect("server state mutex poisoned");
                let task = locked
                    .core
                    .tasks
                    .get(&TaskId::from(record.task_id))
                    .ok_or_else(internal)?;
                return Ok(project(&locked, task));
            }
            Reservation::Conflict | Reservation::Pending(_) => return Err(conflict()),
            Reservation::New => {}
        }
        if let (Some(reference), Some(secret)) = (&record.credential, &input.secret)
            && let Err(error) = self.provider.put(reference, secret)
        {
            // An unsuccessful atomic add owns no provider item. In particular,
            // a duplicate-reference error must never authorize deleting that item.
            let _ = state
                .lock()
                .expect("server state mutex poisoned")
                .core
                .repository
                .discard_pending(&record);
            return Err(provider_error(error));
        }
        let activation = (|| {
            let mut locked = state.lock().expect("server state mutex poisoned");
            let mut candidate = locked.core.tasks.clone();
            let task = candidate
                .create(
                    record.task_id.clone(),
                    DownloadSource::new(record.canonical_url.clone()),
                    Destination::new(record.destination.clone()),
                )
                .map_err(|_| internal())?
                .clone();
            locked
                .core
                .repository
                .activate_origin(&record, StoredTask::from(task.clone()))
                .map_err(|_| internal())
                .map(|()| {
                    locked.core.tasks = candidate;
                    project(&locked, &task)
                })
        })();
        match activation {
            Ok(value) => Ok(value),
            Err(_) => {
                self.abandon(state, &record);
                Err(internal())
            }
        }
    }

    fn owns(&self, record: &OriginRecord, reference: &CredentialRef) -> bool {
        reference.instance_id == self.instance_id
            && reference.origin == record.origin
            && reference.revision == 1
            && reference.credential_id.len() == 32
            && reference
                .credential_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
    }

    fn abandon(&self, state: &Arc<Mutex<ServerState>>, record: &OriginRecord) {
        let abandoned = state
            .lock()
            .expect("server state mutex poisoned")
            .core
            .repository
            .abandon_origin(record)
            .is_ok();
        if abandoned {
            self.cleanup(state);
        }
    }

    fn cleanup(&self, state: &Arc<Mutex<ServerState>>) {
        let records = state
            .lock()
            .expect("server state mutex poisoned")
            .core
            .repository
            .cleanup_records();
        let Ok(records) = records else {
            return;
        };
        for record in records {
            let Some(reference) = record
                .credential
                .as_ref()
                .filter(|reference| self.owns(&record, reference))
            else {
                continue;
            };
            if matches!(
                self.provider.remove(reference),
                Ok(()) | Err(ProviderError::Missing)
            ) {
                let _ = state
                    .lock()
                    .expect("server state mutex poisoned")
                    .core
                    .repository
                    .ack_cleanup(&record);
            }
        }
    }

    pub fn recover(&self, state: &Arc<Mutex<ServerState>>) {
        let _mutation = self
            .mutation
            .lock()
            .expect("origin mutation mutex poisoned");
        // Fetch after acquiring the mutation gate. An earlier startup snapshot may
        // have become active while a request was committing.
        let pending = state
            .lock()
            .expect("server state mutex poisoned")
            .core
            .repository
            .pending_origins();
        if let Ok(pending) = pending {
            for record in pending {
                let _ = state
                    .lock()
                    .expect("server state mutex poisoned")
                    .core
                    .repository
                    .abandon_origin(&record);
            }
        }
        self.cleanup(state);
    }

    pub fn remove(
        &self,
        state: &Arc<Mutex<ServerState>>,
        request: &RpcRequest,
        id: &TaskId,
    ) -> Option<RpcResponse> {
        // A live legacy task already prevents typed reservation of its ID.
        // Keep unrelated HTTP controls independent of a waiting provider.
        let legacy_task = {
            let locked = state.lock().expect("server state mutex poisoned");
            match locked.core.repository.has_origin(id) {
                Ok(false) => locked.core.tasks.get(id).is_some(),
                Ok(true) => false,
                Err(_) => return reply_error(request, internal()),
            }
        };
        if legacy_task {
            return remove_active_http_task(state, request, id);
        }
        let _mutation = self
            .mutation
            .lock()
            .expect("origin mutation mutex poisoned");
        let result = {
            let mut locked = state.lock().expect("server state mutex poisoned");
            match locked.core.repository.active_origin(id) {
                Ok(Some(_)) => locked
                    .core
                    .remove_task(id)
                    .map(|_| true)
                    .map_err(|_| internal()),
                Ok(None) => Ok(false),
                Err(_) => Err(internal()),
            }
        };
        match result {
            Ok(true) => {
                self.cleanup(state);
                reply_bool(request, true)
            }
            Ok(false) => remove_active_http_task(state, request, id),
            Err(error) => reply_error(request, error),
        }
    }
}

pub fn project(state: &ServerState, task: &DownloadTask) -> Value {
    let mut view = TaskView::from(task);
    if let Ok(Some(record)) = state.core.repository.active_origin(&task.id) {
        let authentication = if record.authenticated {
            json!({"kind":"password","username":"validation","password":""})
        } else {
            json!({"kind":"anonymous"})
        };
        let input = json!({"version":record.version,"id":task.id.as_str(),"destination":task.destination.as_str(),"source":{"kind":"ftp","source_url":task.source.as_str(),"security":record.security,"path_mode":record.path_mode,"authentication":authentication}});
        if let Ok(validated) = CreateTransferRequest::parse(&input)
            && record.same_input(&validated.origin)
            && record.authenticated == record.credential.is_some()
            && record
                .credential
                .as_ref()
                .is_none_or(|reference| state.origins.owns(&record, reference))
        {
            view.source = record.canonical_url;
            view.error = task.last_error.clone();
        }
    }
    serde_json::to_value(view).expect("task view serializes")
}

pub fn capabilities() -> Value {
    json!({"create_transfer":{"versions":[1]},"ftp":{"metadata_create":true,"durable_credentials":cfg!(target_os="macos"),"security":["plain"],"transfer":false}})
}

pub fn spawn_recovery(state: &Arc<Mutex<ServerState>>) {
    let state = Arc::clone(state);
    let origins = Arc::clone(&state.lock().expect("server state mutex poisoned").origins);
    if std::thread::Builder::new()
        .name("origin-cleanup".into())
        .spawn(move || origins.recover(&state))
        .is_err()
    {
        eprintln!("origin cleanup worker unavailable, durable cleanup retained");
    }
}

struct SystemProvider;
#[cfg(not(target_os = "macos"))]
impl CredentialProvider for SystemProvider {
    fn durable(&self) -> bool {
        false
    }
    fn put(&self, _: &CredentialRef, _: &OriginSecret) -> Result<(), ProviderError> {
        Err(ProviderError::Unavailable)
    }
    fn get(&self, _: &CredentialRef) -> Result<OriginSecret, ProviderError> {
        Err(ProviderError::Unavailable)
    }
    fn remove(&self, _: &CredentialRef) -> Result<(), ProviderError> {
        Err(ProviderError::Unavailable)
    }
}

#[cfg(target_os = "macos")]
#[path = "origin_keychain.rs"]
mod keychain;
#[cfg(test)]
#[path = "origin_credentials_tests.rs"]
mod tests;
