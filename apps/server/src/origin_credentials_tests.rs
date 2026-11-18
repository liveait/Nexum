use super::*;
use nexum_core::nexum_storage::TaskRepository;
#[derive(Default)]
struct MemoryProvider {
    items: Mutex<HashMap<String, (String, OriginSecret)>>,
    put_error: Mutex<Option<ProviderError>>,
    existing_on_put: Mutex<Option<OriginSecret>>,
    get_error: Mutex<Option<ProviderError>>,
    remove_error: Mutex<Option<ProviderError>>,
    gate: Mutex<Option<Arc<Gate>>>,
}
#[derive(Default)]
struct Gate {
    state: Mutex<(bool, bool)>,
    signal: Condvar,
}
fn key(reference: &CredentialRef) -> String {
    format!("{reference:?}")
}
impl CredentialProvider for MemoryProvider {
    fn durable(&self) -> bool {
        true
    }
    fn put(
        &self,
        reference: &CredentialRef,
        owner: &str,
        secret: &OriginSecret,
    ) -> Result<(), ProviderError> {
        if let Some(gate) = self.gate.lock().unwrap().clone() {
            let mut state = gate.state.lock().unwrap();
            state.0 = true;
            gate.signal.notify_all();
            while !state.1 {
                state = gate.signal.wait(state).unwrap();
            }
        }
        if let Some(error) = *self.put_error.lock().unwrap() {
            return Err(error);
        }
        let mut items = self.items.lock().unwrap();
        if let Some(existing) = self.existing_on_put.lock().unwrap().take() {
            items.insert(key(reference), ("preexisting-owner".into(), existing));
        }
        if items.contains_key(&key(reference)) {
            return Err(ProviderError::Exists);
        }
        items.insert(key(reference), (owner.to_owned(), secret.clone()));
        Ok(())
    }
    fn get(&self, reference: &CredentialRef, owner: &str) -> Result<OriginSecret, ProviderError> {
        if let Some(error) = *self.get_error.lock().unwrap() {
            return Err(error);
        }
        self.items
            .lock()
            .unwrap()
            .get(&key(reference))
            .filter(|(stored_owner, _)| stored_owner == owner)
            .map(|(_, secret)| secret.clone())
            .ok_or(ProviderError::Missing)
    }
    fn remove(&self, reference: &CredentialRef, owner: &str) -> Result<(), ProviderError> {
        if let Some(error) = *self.remove_error.lock().unwrap() {
            return Err(error);
        }
        let mut items = self.items.lock().unwrap();
        if items
            .get(&key(reference))
            .is_some_and(|(stored_owner, _)| stored_owner == owner)
        {
            items.remove(&key(reference));
        }
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    state: Arc<Mutex<ServerState>>,
    provider: Arc<MemoryProvider>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("nexum-origin-{}", random_id().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let opened = open_core(&root, BandwidthPolicyKind::Unlimited).unwrap();
        let provider = Arc::new(MemoryProvider::default());
        let origins = Arc::new(Coordinator {
            mutation: Mutex::new(()),
            provider: provider.clone(),
            instance_id: opened.core.repository.instance_id().unwrap(),
        });
        let state = Arc::new(Mutex::new(ServerState {
            origins,
            core: opened.core,
            pending_completions: opened.pending_completions,
            _data_dir_lock: lock_data_dir(&root).unwrap(),
            data_dir: root.clone(),
            active_http: HashMap::new(),
            reconcile_completion: reconcile_http_transfer,
            active_destinations: HashSet::new(),
            events: EventHub::default(),
            authentication: AuthenticationConfig {
                required: false,
                credential: None,
            },
            rate_limiter: None,
            inspections: Arc::new(torrent_inspection::Inspector::new().unwrap()),
        }));
        Self {
            root,
            state,
            provider,
        }
    }
    fn origins(&self) -> Arc<Coordinator> {
        self.state.lock().unwrap().origins.clone()
    }
    fn input(&self, id: &str) -> Value {
        json!({"version":1,"id":id,"destination":self.root.join("output"),"source":{"kind":"ftp","source_url":"ftp://example.com/pub/a.bin","security":"plain","authentication":{"kind":"password","username":"USER_SENTINEL","password":"PASSWORD_SENTINEL"}}})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn password_retries_compare_secrets_and_references_are_scoped() {
    let fixture = Fixture::new();
    let origins = fixture.origins();
    let input = fixture.input("one");
    let first = origins.create(&fixture.state, &input, true).unwrap();
    assert_eq!(first["source"], "ftp://example.com/pub/a.bin");
    assert_eq!(origins.create(&fixture.state, &input, true).unwrap(), first);
    for change in [
        json!({"kind":"anonymous"}),
        json!({"kind":"password","username":"other","password":"PASSWORD_SENTINEL"}),
        json!({"kind":"password","username":"USER_SENTINEL","password":"changed"}),
    ] {
        let mut changed = input.clone();
        changed["source"]["authentication"] = change;
        assert_eq!(
            origins
                .create(&fixture.state, &changed, true)
                .unwrap_err()
                .code,
            -32009
        );
    }
    assert_eq!(fixture.provider.items.lock().unwrap().len(), 1);
    let locked = fixture.state.lock().unwrap();
    let record = locked
        .core
        .repository
        .active_origin(&TaskId::from("one"))
        .unwrap()
        .unwrap();
    assert_eq!(
        record.credential.as_ref().unwrap().instance_id,
        locked.core.repository.instance_id().unwrap()
    );
    assert_eq!(
        record.credential.as_ref().unwrap().origin,
        "ftp://example.com:21"
    );
    assert_eq!(record.credential.as_ref().unwrap().revision, 1);
    assert!(!serde_json::to_string(&record).unwrap().contains("SENTINEL"));
    drop(locked);
    assert!(
        !std::fs::read(fixture.root.join(DATABASE_FILE))
            .unwrap()
            .windows(b"SENTINEL".len())
            .any(|bytes| bytes == b"SENTINEL")
    );
}
#[test]
fn locked_missing_and_denied_retry_never_mutate_or_fallback() {
    let fixture = Fixture::new();
    let origins = fixture.origins();
    let input = fixture.input("one");
    origins.create(&fixture.state, &input, true).unwrap();
    for error in [
        ProviderError::Locked,
        ProviderError::Denied,
        ProviderError::Missing,
        ProviderError::Unavailable,
    ] {
        *fixture.provider.get_error.lock().unwrap() = Some(error);
        assert_eq!(
            origins.create(&fixture.state, &input, true).unwrap_err(),
            provider_error(error)
        );
        assert_eq!(fixture.provider.items.lock().unwrap().len(), 1);
        assert_eq!(
            fixture
                .state
                .lock()
                .unwrap()
                .core
                .repository
                .list()
                .unwrap()
                .len(),
            1
        );
    }
}
#[test]
fn activation_failure_compensates_and_failed_compensation_survives_restart() {
    for failed_cleanup in [false, true] {
        let fixture = Fixture::new();
        let origins = fixture.origins();
        rusqlite::Connection::open(fixture.root.join(DATABASE_FILE)).unwrap().execute_batch("CREATE TRIGGER reject_insert BEFORE INSERT ON tasks BEGIN SELECT RAISE(FAIL,'controlled insert failure'); END;").unwrap();
        if failed_cleanup {
            *fixture.provider.remove_error.lock().unwrap() = Some(ProviderError::Locked);
        }
        assert!(
            origins
                .create(&fixture.state, &fixture.input("fail"), true)
                .is_err()
        );
        assert!(
            fixture
                .state
                .lock()
                .unwrap()
                .core
                .repository
                .list()
                .unwrap()
                .is_empty()
        );
        assert!(
            fixture
                .state
                .lock()
                .unwrap()
                .core
                .tasks
                .list()
                .next()
                .is_none()
        );
        assert_eq!(
            fixture.provider.items.lock().unwrap().len(),
            usize::from(failed_cleanup)
        );
        assert_eq!(
            fixture
                .state
                .lock()
                .unwrap()
                .core
                .repository
                .cleanup_records()
                .unwrap()
                .len(),
            usize::from(failed_cleanup)
        );
        *fixture.provider.remove_error.lock().unwrap() = None;
        origins.recover(&fixture.state);
        assert!(fixture.provider.items.lock().unwrap().is_empty());
        assert!(
            fixture
                .state
                .lock()
                .unwrap()
                .core
                .repository
                .cleanup_records()
                .unwrap()
                .is_empty()
        );
    }
}
#[test]
fn delete_is_durable_before_cleanup_and_reuse_keeps_new_reference() {
    let fixture = Fixture::new();
    let origins = fixture.origins();
    let input = fixture.input("reuse");
    origins.create(&fixture.state, &input, true).unwrap();
    *fixture.provider.remove_error.lock().unwrap() = Some(ProviderError::Denied);
    let request = RpcRequest::new(1, "task.remove", Some(json!({"id":"reuse"})));
    assert!(
        origins
            .remove(&fixture.state, &request, &TaskId::from("reuse"))
            .unwrap()
            .is_success()
    );
    assert!(
        fixture
            .state
            .lock()
            .unwrap()
            .core
            .repository
            .list()
            .unwrap()
            .is_empty()
    );
    let old = fixture
        .state
        .lock()
        .unwrap()
        .core
        .repository
        .cleanup_records()
        .unwrap()
        .remove(0);
    origins.create(&fixture.state, &input, true).unwrap();
    let fresh = fixture
        .state
        .lock()
        .unwrap()
        .core
        .repository
        .active_origin(&TaskId::from("reuse"))
        .unwrap()
        .unwrap();
    assert_ne!(old.credential, fresh.credential);
    *fixture.provider.remove_error.lock().unwrap() = None;
    origins.recover(&fixture.state);
    assert_eq!(fixture.provider.items.lock().unwrap().len(), 1);
    assert!(
        fixture
            .provider
            .get(fresh.credential.as_ref().unwrap(), &fresh.operation_id)
            .is_ok()
    );
}
#[test]
fn pending_recovery_releases_only_exact_owned_items() {
    let fixture = Fixture::new();
    let origins = fixture.origins();
    let parsed = CreateTransferRequest::parse(&fixture.input("pending")).unwrap();
    let mut record = parsed.origin;
    record.operation_id = random_id().unwrap();
    record.credential = Some(CredentialRef {
        instance_id: origins.instance_id.clone(),
        origin: record.origin.clone(),
        credential_id: random_id().unwrap(),
        revision: 1,
    });
    fixture
        .state
        .lock()
        .unwrap()
        .core
        .repository
        .reserve_origin(&record)
        .unwrap();
    fixture
        .provider
        .put(
            record.credential.as_ref().unwrap(),
            &record.operation_id,
            parsed.secret.as_ref().unwrap(),
        )
        .unwrap();
    let mut foreign = record.credential.as_ref().unwrap().clone();
    foreign.instance_id = random_id().unwrap();
    fixture
        .provider
        .put(
            &foreign,
            &record.operation_id,
            parsed.secret.as_ref().unwrap(),
        )
        .unwrap();
    fixture.state.lock().unwrap().core.repository =
        SqliteRepository::open(fixture.root.join(DATABASE_FILE)).unwrap();
    origins.recover(&fixture.state);
    origins.recover(&fixture.state);
    assert!(
        fixture
            .state
            .lock()
            .unwrap()
            .core
            .repository
            .pending_origins()
            .unwrap()
            .is_empty()
    );
    assert!(fixture.provider.get(&foreign, &record.operation_id).is_ok());
    assert_eq!(
        fixture
            .provider
            .get(record.credential.as_ref().unwrap(), &record.operation_id),
        Err(ProviderError::Missing)
    );
}
#[test]
fn blocked_provider_preserves_core_responsiveness_and_pending_fences_legacy() {
    let fixture = Fixture::new();
    let origins = fixture.origins();
    let gate = Arc::new(Gate::default());
    *fixture.provider.gate.lock().unwrap() = Some(gate.clone());
    let input = fixture.input("blocked");
    let state = fixture.state.clone();
    let worker = std::thread::spawn(move || origins.create(&state, &input, true));
    let wait = gate.state.lock().unwrap();
    let (wait, timeout) = gate
        .signal
        .wait_timeout_while(wait, Duration::from_secs(3), |state| !state.0)
        .unwrap();
    assert!(!timeout.timed_out());
    drop(wait);
    let request = RpcRequest::new(
        1,
        "task.create",
        Some(
            json!({"id":"blocked","source":"https://example.com/file","destination":fixture.root.join("other")}),
        ),
    );
    let response = dispatch_server_request(&fixture.state, &request).unwrap();
    assert!(!response.is_success());
    assert!(
        dispatch_server_request(&fixture.state, &RpcRequest::new(2, "task.list", None))
            .unwrap()
            .is_success()
    );
    let origins = fixture.origins();
    let state = fixture.state.clone();
    let recovery = std::thread::spawn(move || origins.recover(&state));
    {
        let mut wait = gate.state.lock().unwrap();
        wait.1 = true;
        gate.signal.notify_all();
    }
    assert!(worker.join().unwrap().is_ok());
    recovery.join().unwrap();
    assert_eq!(fixture.provider.items.lock().unwrap().len(), 1);
    assert!(
        fixture
            .state
            .lock()
            .unwrap()
            .core
            .repository
            .active_origin(&TaskId::from("blocked"))
            .unwrap()
            .is_some()
    );
}
#[test]
fn provider_add_failure_does_not_remove_existing_exact_item() {
    let fixture = Fixture::new();
    let origins = fixture.origins();
    let existing = OriginSecret {
        username: "PREEXISTING_USER".into(),
        password: "PREEXISTING_SECRET".into(),
    };
    *fixture.provider.existing_on_put.lock().unwrap() = Some(existing.clone());
    let database = rusqlite::Connection::open(fixture.root.join(DATABASE_FILE)).unwrap();
    database.execute_batch("CREATE TRIGGER reject_pending_delete BEFORE DELETE ON task_origins BEGIN SELECT RAISE(FAIL,'controlled pending deletion failure'); END;").unwrap();
    assert_eq!(
        origins
            .create(&fixture.state, &fixture.input("collision"), true)
            .unwrap_err(),
        provider_error(ProviderError::Exists)
    );
    let pending = fixture
        .state
        .lock()
        .unwrap()
        .core
        .repository
        .pending_origins()
        .unwrap()
        .remove(0);
    let reference = pending.credential.as_ref().unwrap();
    assert_eq!(
        fixture.provider.items.lock().unwrap().get(&key(reference)),
        Some(&("preexisting-owner".into(), existing.clone()))
    );
    database
        .execute_batch("DROP TRIGGER reject_pending_delete;")
        .unwrap();
    drop(database);
    fixture.state.lock().unwrap().core.repository =
        SqliteRepository::open(fixture.root.join(DATABASE_FILE)).unwrap();
    origins.recover(&fixture.state);
    assert_eq!(
        fixture.provider.items.lock().unwrap().get(&key(reference)),
        Some(&("preexisting-owner".into(), existing.clone()))
    );
    assert!(
        fixture
            .state
            .lock()
            .unwrap()
            .core
            .repository
            .pending_origins()
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .state
            .lock()
            .unwrap()
            .core
            .repository
            .cleanup_records()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn concurrent_identical_creation_commits_one_task_and_provider_item() {
    let fixture = Fixture::new();
    let mut workers = Vec::new();
    for _ in 0..8 {
        let origins = fixture.origins();
        let state = fixture.state.clone();
        let input = fixture.input("same");
        workers.push(std::thread::spawn(move || {
            origins.create(&state, &input, true)
        }));
    }
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect();
    assert!(results.iter().all(|result| result == &results[0]));
    assert_eq!(fixture.provider.items.lock().unwrap().len(), 1);
    assert_eq!(
        fixture
            .state
            .lock()
            .unwrap()
            .core
            .repository
            .list()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn typed_projection_requires_matching_versioned_binding() {
    let fixture = Fixture::new();
    fixture
        .origins()
        .create(&fixture.state, &fixture.input("one"), true)
        .unwrap();
    let database = rusqlite::Connection::open(fixture.root.join(DATABASE_FILE)).unwrap();
    database.execute("UPDATE task_origins SET record=json_set(record,'$.canonical_url','ftp://different.example/a.bin') WHERE task_id='one'", []).unwrap();
    let locked = fixture.state.lock().unwrap();
    let task = locked.core.tasks.get(&TaskId::from("one")).unwrap();
    assert_eq!(project(&locked, task)["source"], "[legacy non-HTTP source]");
}

#[test]
fn blocked_provider_does_not_delay_unrelated_http_removal() {
    let fixture = Fixture::new();
    let request = RpcRequest::new(
        1,
        "task.create",
        Some(
            json!({"id":"http","source":"https://example.com/file","destination":fixture.root.join("http")}),
        ),
    );
    assert!(
        dispatch_server_request(&fixture.state, &request)
            .unwrap()
            .is_success()
    );
    let gate = Arc::new(Gate::default());
    *fixture.provider.gate.lock().unwrap() = Some(gate.clone());
    let origins = fixture.origins();
    let state = fixture.state.clone();
    let input = fixture.input("blocked");
    let creation = std::thread::spawn(move || origins.create(&state, &input, true));
    let wait = gate.state.lock().unwrap();
    let (wait, timeout) = gate
        .signal
        .wait_timeout_while(wait, Duration::from_secs(3), |state| !state.0)
        .unwrap();
    assert!(!timeout.timed_out());
    drop(wait);
    let state = fixture.state.clone();
    let (sender, receiver) = mpsc::channel();
    let deletion = std::thread::spawn(move || {
        sender
            .send(dispatch_server_request(
                &state,
                &RpcRequest::new(2, "task.remove", Some(json!({"id":"http"}))),
            ))
            .unwrap();
    });
    let response = receiver.recv_timeout(Duration::from_millis(300));
    {
        let mut wait = gate.state.lock().unwrap();
        wait.1 = true;
        gate.signal.notify_all();
    }
    creation.join().unwrap().unwrap();
    deletion.join().unwrap();
    assert!(
        response
            .expect("unrelated HTTP removal waited for Keychain")
            .unwrap()
            .is_success()
    );
}

#[test]
fn provider_operation_owner_fences_reads_and_removal_without_changing_uniqueness() {
    let fixture = Fixture::new();
    fixture
        .origins()
        .create(&fixture.state, &fixture.input("owner"), true)
        .unwrap();
    let record = fixture
        .state
        .lock()
        .unwrap()
        .core
        .repository
        .active_origin(&TaskId::from("owner"))
        .unwrap()
        .unwrap();
    let reference = record.credential.as_ref().unwrap();
    let secret = fixture
        .provider
        .get(reference, &record.operation_id)
        .unwrap();
    assert_eq!(
        fixture.provider.get(reference, "different-owner"),
        Err(ProviderError::Missing)
    );
    assert_eq!(
        fixture.provider.put(
            reference,
            "different-owner",
            &OriginSecret {
                username: "other".into(),
                password: "other".into()
            }
        ),
        Err(ProviderError::Exists)
    );
    fixture
        .provider
        .remove(reference, "different-owner")
        .unwrap();
    assert_eq!(
        fixture.provider.get(reference, &record.operation_id),
        Ok(secret)
    );
}
