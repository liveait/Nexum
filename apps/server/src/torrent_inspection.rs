//! Process-local metainfo jobs; no download, output files or network discovery.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use nexum_core::nexum_resolver::torrent::{self, MetainfoError, MetainfoSummary};
use nexum_protocol::RpcErrorObject;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_ACTIVE: usize = 2;
const MAX_RECORDS: usize = 16;
const PARSE_BUDGET: Duration = Duration::from_secs(5);
const RESULT_TTL: Duration = Duration::from_secs(600);
const REAP_INTERVAL: Duration = Duration::from_millis(10);
const MAX_ENCODED: usize = torrent::MAX_METAINFO_BYTES.div_ceil(3) * 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Principal {
    Anonymous,
    Authenticated,
}

struct Inspected {
    _raw: Vec<u8>,
    summary: MetainfoSummary,
}
type Parse =
    dyn Fn(&[u8], &AtomicBool, Instant) -> Result<MetainfoSummary, MetainfoError> + Send + Sync;
type Worker = JoinHandle<Result<Inspected, MetainfoError>>;

struct Job {
    principal: Principal,
    expires: Instant,
    cancel: Arc<AtomicBool>,
    worker: Option<Worker>,
    joining: bool,
    phase: &'static str,
    error: Option<&'static str>,
    result: Option<Inspected>,
}
#[derive(Default)]
struct Registry {
    stopped: bool,
    jobs: HashMap<String, Job>,
}
struct Inner {
    registry: Mutex<Registry>,
    wake: Condvar,
}

pub(super) struct Inspector {
    inner: Arc<Inner>,
    reaper: Mutex<Option<JoinHandle<()>>>,
    parse: Arc<Parse>,
    ttl: Duration,
    parse_budget: Duration,
}

pub(super) struct ShutdownGuard(Arc<Inspector>);
impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

fn not_found() -> RpcErrorObject {
    RpcErrorObject {
        code: -32031,
        message: "inspection not found or expired".into(),
        data: None,
    }
}
fn exhausted() -> RpcErrorObject {
    RpcErrorObject {
        code: -32030,
        message: "inspection resource limit reached".into(),
        data: None,
    }
}

impl Inspector {
    pub(super) fn new() -> io::Result<Self> {
        Self::with_parser(
            Arc::new(torrent::inspect_metainfo),
            RESULT_TTL,
            PARSE_BUDGET,
        )
    }

    fn with_parser(parse: Arc<Parse>, ttl: Duration, parse_budget: Duration) -> io::Result<Self> {
        let inner = Arc::new(Inner {
            registry: Mutex::new(Registry::default()),
            wake: Condvar::new(),
        });
        let worker_inner = Arc::clone(&inner);
        let reaper = thread::Builder::new()
            .name("nexum-inspection-reaper".into())
            .spawn(move || reap(worker_inner))?;
        Ok(Self {
            inner,
            reaper: Mutex::new(Some(reaper)),
            parse,
            ttl,
            parse_budget,
        })
    }
    pub(super) fn shutdown_guard(self: &Arc<Self>) -> ShutdownGuard {
        ShutdownGuard(Arc::clone(self))
    }
    pub(super) fn shutdown(&self) {
        {
            let mut registry = self
                .inner
                .registry
                .lock()
                .expect("inspection registry poisoned");
            registry.stopped = true;
            for job in registry.jobs.values() {
                job.cancel.store(true, Ordering::Release);
            }
            self.inner.wake.notify_all();
        }
        // Never hold the registry/Core lock while joining work.
        if let Some(reaper) = self
            .reaper
            .lock()
            .expect("inspection reaper poisoned")
            .take()
        {
            let _ = reaper.join();
        }
    }

    pub(super) fn capabilities() -> Value {
        json!({"version": 1, "sources": ["metainfo_base64"], "max_metainfo_bytes": torrent::MAX_METAINFO_BYTES,
            "max_files": torrent::MAX_FILES, "max_pieces": torrent::MAX_PIECES,
            "max_bencode_depth": torrent::MAX_DEPTH, "max_bencode_nodes": torrent::MAX_NODES,
            "max_path_depth": torrent::MAX_PATH_DEPTH, "max_component_bytes": torrent::MAX_COMPONENT_BYTES,
            "max_active": MAX_ACTIVE, "max_records": MAX_RECORDS, "parse_timeout_seconds": PARSE_BUDGET.as_secs(), "ttl_seconds": RESULT_TTL.as_secs()})
    }

    pub(super) fn begin(
        &self,
        principal: Principal,
        params: &Value,
    ) -> Result<Value, RpcErrorObject> {
        let object = params
            .as_object()
            .filter(|p| p.len() == 1)
            .ok_or_else(|| RpcErrorObject::invalid_params("expected only metainfo_base64"))?;
        let encoded = object
            .get("metainfo_base64")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcErrorObject::invalid_params("expected only metainfo_base64"))?;
        if encoded.len() > MAX_ENCODED {
            return Err(RpcErrorObject::invalid_params("metainfo exceeds 512 KiB"));
        }
        // Reserve under the same lock as spawn, so concurrent clients cannot
        // exceed quotas. Invalid input/spawn/entropy failure leaves no record.
        let mut registry = self
            .inner
            .registry
            .lock()
            .expect("inspection registry poisoned");
        if registry.stopped {
            return Err(RpcErrorObject::internal_error(
                "inspection service is stopped",
            ));
        }
        if registry.jobs.len() >= MAX_RECORDS
            || registry
                .jobs
                .values()
                .filter(|job| job.worker.is_some() || job.joining)
                .count()
                >= MAX_ACTIVE
        {
            return Err(exhausted());
        }
        let raw = STANDARD
            .decode(encoded)
            .map_err(|_| RpcErrorObject::invalid_params("invalid metainfo base64"))?;
        if raw.len() > torrent::MAX_METAINFO_BYTES {
            return Err(RpcErrorObject::invalid_params("metainfo exceeds 512 KiB"));
        }
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| RpcErrorObject::internal_error("could not generate inspection ID"))?;
        let id = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if registry.jobs.contains_key(&id) {
            return Err(RpcErrorObject::internal_error("inspection ID collision"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let parse = Arc::clone(&self.parse);
        let deadline = Instant::now() + self.parse_budget;
        let worker = thread::Builder::new()
            .name("nexum-metainfo".into())
            .spawn(move || {
                let summary = parse(&raw, &worker_cancel, deadline)?;
                Ok(Inspected { _raw: raw, summary })
            })
            .map_err(|_| RpcErrorObject::internal_error("could not start inspection worker"))?;
        registry.jobs.insert(
            id.clone(),
            Job {
                principal,
                expires: Instant::now() + self.ttl,
                cancel,
                worker: Some(worker),
                joining: false,
                phase: "parsing",
                error: None,
                result: None,
            },
        );
        self.inner.wake.notify_all();
        Ok(json!({"inspection_id": id}))
    }

    pub(super) fn status(&self, principal: Principal, id: &str) -> Result<Value, RpcErrorObject> {
        let registry = self
            .inner
            .registry
            .lock()
            .expect("inspection registry poisoned");
        let job = registry
            .jobs
            .get(id)
            .filter(|job| {
                job.principal == principal && Instant::now() < job.expires && !registry.stopped
            })
            .ok_or_else(not_found)?;
        let mut status = json!({"inspection_id": id, "phase": job.phase});
        if let Some(result) = &job.result {
            status["result"] = serde_json::to_value(&result.summary).expect("summary serializes");
        }
        if let Some(error) = job.error {
            status["error"] = json!({"code": error});
        }
        Ok(status)
    }

    pub(super) fn cancel(&self, principal: Principal, id: &str) -> Result<Value, RpcErrorObject> {
        let mut registry = self
            .inner
            .registry
            .lock()
            .expect("inspection registry poisoned");
        loop {
            let stopped = registry.stopped;
            let job = registry
                .jobs
                .get_mut(id)
                .filter(|job| {
                    job.principal == principal && Instant::now() < job.expires && !stopped
                })
                .ok_or_else(not_found)?;
            job.cancel.store(true, Ordering::Release);
            if job.joining {
                registry = self
                    .inner
                    .wake
                    .wait(registry)
                    .expect("inspection registry poisoned");
                continue;
            }
            let worker = job.worker.take();
            job.joining = worker.is_some();
            drop(registry);
            if let Some(worker) = worker {
                let _ = worker.join();
            }
            let mut registry = self
                .inner
                .registry
                .lock()
                .expect("inspection registry poisoned");
            if let Some(job) = registry.jobs.get_mut(id) {
                job.joining = false;
                job.phase = "cancelled";
                job.error = None;
                job.result = None;
            }
            self.inner.wake.notify_all();
            return Ok(json!({"cancelled": true}));
        }
    }
}
impl Drop for Inspector {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn reap(inner: Arc<Inner>) {
    loop {
        let mut registry = inner.registry.lock().expect("inspection registry poisoned");
        let now = Instant::now();
        let stopped = registry.stopped;
        let mut joins = Vec::new();
        let previous_records = registry.jobs.len();
        registry.jobs.retain(|id, job| {
            let expired = stopped || now >= job.expires;
            if expired {
                job.cancel.store(true, Ordering::Release);
            }
            if job
                .worker
                .as_ref()
                .is_some_and(|worker| expired || worker.is_finished())
            {
                job.joining = true;
                joins.push((id.clone(), job.worker.take().expect("worker present")));
            }
            !expired || job.joining
        });
        if registry.jobs.len() != previous_records {
            inner.wake.notify_all();
        }
        if !joins.is_empty() {
            drop(registry);
            for (id, worker) in joins {
                let outcome = worker.join();
                let mut registry = inner.registry.lock().expect("inspection registry poisoned");
                if let Some(job) = registry.jobs.get_mut(&id) {
                    job.joining = false;
                    if job.cancel.load(Ordering::Acquire) {
                        job.phase = "cancelled";
                        job.result = None;
                    } else {
                        match outcome {
                            Ok(Ok(result)) => {
                                job.phase = "ready";
                                job.result = Some(result);
                            }
                            Ok(Err(error)) => {
                                job.phase = "failed";
                                job.error = Some(error.code());
                            }
                            Err(_) => {
                                job.phase = "failed";
                                job.error = Some("inspection_worker_failed");
                            }
                        }
                    }
                }
                inner.wake.notify_all();
            }
            continue;
        }
        if stopped && registry.jobs.is_empty() {
            return;
        }
        let wait = registry
            .jobs
            .values()
            .map(|job| job.expires.saturating_duration_since(now))
            .min()
            .unwrap_or(RESULT_TTL);
        let wait = if registry
            .jobs
            .values()
            .any(|job| job.worker.is_some() || job.joining)
        {
            wait.min(REAP_INTERVAL)
        } else {
            wait
        };
        let wait = if wait.is_zero() { REAP_INTERVAL } else { wait };
        let _ = inner
            .wake
            .wait_timeout(registry, wait)
            .expect("inspection registry poisoned");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};

    fn params() -> Value {
        // A three-byte, one-piece v1 torrent; only the info dictionary is hashed.
        let raw = [
            b"d4:infod6:lengthi3e4:name5:a.txt12:piece lengthi4e6:pieces20:".as_slice(),
            &[0; 20],
            b"ee",
        ]
        .concat();
        json!({"metainfo_base64": STANDARD.encode(raw)})
    }
    fn id(value: Value) -> String {
        value["inspection_id"].as_str().unwrap().to_owned()
    }
    fn terminal(inspector: &Inspector, principal: Principal, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = inspector.status(principal, id).unwrap();
            if status["phase"] != "parsing" {
                return status;
            }
            assert!(Instant::now() < deadline, "inspection failed to terminate");
            thread::sleep(Duration::from_millis(1));
        }
    }

    struct Gate {
        counts: Mutex<(usize, usize)>,
        wake: Condvar,
        release: AtomicBool,
    }
    impl Gate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                counts: Mutex::new((0, 0)),
                wake: Condvar::new(),
                release: AtomicBool::new(false),
            })
        }
        fn parser(self: &Arc<Self>) -> Arc<Parse> {
            let gate = Arc::clone(self);
            Arc::new(move |bytes, cancel, deadline| {
                let mut counts = gate.counts.lock().unwrap();
                counts.0 += 1;
                gate.wake.notify_all();
                while !cancel.load(Ordering::Acquire)
                    && !gate.release.load(Ordering::Acquire)
                    && Instant::now() < deadline
                {
                    counts = gate
                        .wake
                        .wait_timeout(counts, Duration::from_millis(2))
                        .unwrap()
                        .0;
                }
                if cancel.load(Ordering::Acquire) {
                    counts.1 += 1;
                    gate.wake.notify_all();
                    // Hold the actual worker after cancellation was observed.
                    // A cancel/expiry/shutdown reply cannot precede this exit.
                    while !gate.release.load(Ordering::Acquire) && Instant::now() < deadline {
                        counts = gate
                            .wake
                            .wait_timeout(counts, Duration::from_millis(2))
                            .unwrap()
                            .0;
                    }
                }
                drop(counts);
                torrent::inspect_metainfo(bytes, cancel, deadline)
            })
        }
        fn wait(&self, entered: usize, cancelled: usize) {
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut counts = self.counts.lock().unwrap();
            while counts.0 < entered || counts.1 < cancelled {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "worker did not reach the controlled boundary"
                );
                counts = self.wake.wait_timeout(counts, remaining).unwrap().0;
            }
        }
        fn open(&self) {
            self.release.store(true, Ordering::Release);
            self.wake.notify_all();
        }
    }

    #[test]
    fn concurrent_begins_reserve_only_two_actual_workers() {
        let gate = Gate::new();
        let inspector = Arc::new(
            Inspector::with_parser(
                gate.parser(),
                Duration::from_secs(30),
                Duration::from_secs(3),
            )
            .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(9));
        let clients = (0..8)
            .map(|_| {
                let inspector = Arc::clone(&inspector);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    inspector.begin(Principal::Anonymous, &params())
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = clients
            .into_iter()
            .map(|client| client.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
        assert!(
            results
                .iter()
                .filter_map(|r| r.as_ref().err())
                .all(|e| e.code == -32030)
        );
        gate.wait(2, 0);
        gate.open();
        for result in results.into_iter().filter_map(Result::ok) {
            assert_eq!(
                terminal(&inspector, Principal::Anonymous, &id(result))["phase"],
                "ready"
            );
        }
        assert!(inspector.begin(Principal::Anonymous, &params()).is_ok());
    }

    #[test]
    fn cancel_joins_before_reply_and_keeps_reservations_until_exit() {
        let gate = Gate::new();
        let inspector = Arc::new(
            Inspector::with_parser(
                gate.parser(),
                Duration::from_secs(30),
                Duration::from_secs(3),
            )
            .unwrap(),
        );
        let first = id(inspector.begin(Principal::Anonymous, &params()).unwrap());
        let second = id(inspector.begin(Principal::Anonymous, &params()).unwrap());
        gate.wait(2, 0);
        let worker_inspector = Arc::clone(&inspector);
        let worker_id = first.clone();
        let (sent, received) = mpsc::channel();
        let caller = thread::spawn(move || {
            let result = worker_inspector.cancel(Principal::Anonymous, &worker_id);
            sent.send(result).unwrap();
        });
        gate.wait(2, 1);
        assert!(matches!(
            received.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(
            inspector
                .begin(Principal::Anonymous, &params())
                .unwrap_err()
                .code,
            -32030
        );
        gate.open();
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap()["cancelled"],
            true
        );
        caller.join().unwrap();
        let status = inspector.status(Principal::Anonymous, &first).unwrap();
        assert_eq!(status["phase"], "cancelled");
        assert!(status.get("result").is_none());
        assert_eq!(
            inspector.cancel(Principal::Anonymous, &first).unwrap()["cancelled"],
            true
        );
        assert_eq!(
            terminal(&inspector, Principal::Anonymous, &second)["phase"],
            "ready"
        );
        assert!(inspector.begin(Principal::Anonymous, &params()).is_ok());
    }

    #[test]
    fn expiry_cancels_and_joins_without_status_driving_cleanup() {
        let gate = Gate::new();
        let inspector = Inspector::with_parser(
            gate.parser(),
            Duration::from_millis(60),
            Duration::from_secs(3),
        )
        .unwrap();
        let id = id(inspector.begin(Principal::Anonymous, &params()).unwrap());
        gate.wait(1, 1);
        assert_eq!(
            inspector
                .status(Principal::Anonymous, &id)
                .unwrap_err()
                .code,
            -32031
        );
        assert!(
            inspector
                .inner
                .registry
                .lock()
                .unwrap()
                .jobs
                .get(&id)
                .unwrap()
                .joining
        );
        gate.open();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut registry = inspector.inner.registry.lock().unwrap();
        while !registry.jobs.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero());
            registry = inspector.inner.wake.wait_timeout(registry, left).unwrap().0;
        }
        drop(registry);
        assert_eq!(
            inspector
                .cancel(Principal::Anonymous, &id)
                .unwrap_err()
                .code,
            -32031
        );
    }

    #[test]
    fn shutdown_guard_waits_for_actual_exit_and_rejects_new_work() {
        let gate = Gate::new();
        let inspector = Arc::new(
            Inspector::with_parser(
                gate.parser(),
                Duration::from_secs(30),
                Duration::from_secs(3),
            )
            .unwrap(),
        );
        inspector.begin(Principal::Anonymous, &params()).unwrap();
        gate.wait(1, 0);
        let guard = inspector.shutdown_guard();
        let (sent, received) = mpsc::channel();
        let shutdown = thread::spawn(move || {
            drop(guard);
            sent.send(()).unwrap();
        });
        gate.wait(1, 1);
        assert!(matches!(
            received.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(inspector.begin(Principal::Anonymous, &params()).is_err());
        gate.open();
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        shutdown.join().unwrap();
        assert!(inspector.inner.registry.lock().unwrap().jobs.is_empty());
        inspector.shutdown();
    }

    #[test]
    fn principal_limits_and_invalid_inputs_are_isolated() {
        let inspector = Inspector::new().unwrap();
        for value in [
            json!({"path":"sentinel-passkey"}),
            json!({"magnet":"sentinel-passkey"}),
            json!({"metainfo_base64":"??"}),
            json!({"metainfo_base64":"", "extra":true}),
            json!({"metainfo_base64":STANDARD.encode(vec![0; torrent::MAX_METAINFO_BYTES+1])}),
        ] {
            let error = inspector.begin(Principal::Anonymous, &value).unwrap_err();
            assert_eq!(error.code, -32602);
            assert!(!format!("{error:?}").contains("sentinel-passkey"));
        }
        assert!(inspector.inner.registry.lock().unwrap().jobs.is_empty());
        let own = id(inspector
            .begin(Principal::Authenticated, &params())
            .unwrap());
        assert_eq!(
            inspector.status(Principal::Anonymous, &own).unwrap_err(),
            not_found()
        );
        assert_eq!(
            inspector.cancel(Principal::Anonymous, &own).unwrap_err(),
            not_found()
        );
        assert_eq!(
            terminal(&inspector, Principal::Authenticated, &own)["phase"],
            "ready"
        );
        let other = Inspector::new().unwrap();
        assert_eq!(
            other.status(Principal::Authenticated, &own).unwrap_err(),
            not_found()
        );
        for _ in 1..MAX_RECORDS {
            let id = id(inspector
                .begin(Principal::Authenticated, &params())
                .unwrap());
            terminal(&inspector, Principal::Authenticated, &id);
        }
        assert_eq!(
            inspector
                .begin(Principal::Authenticated, &params())
                .unwrap_err()
                .code,
            -32030
        );
    }

    #[test]
    fn parser_failures_and_deadlines_are_terminal_and_free_active_slots() {
        let inspector = Inspector::new().unwrap();
        let bad = id(inspector
            .begin(
                Principal::Anonymous,
                &json!({"metainfo_base64":STANDARD.encode(b"sentinel-passkey")}),
            )
            .unwrap());
        let status = terminal(&inspector, Principal::Anonymous, &bad);
        assert_eq!(status["phase"], "failed");
        assert_eq!(status["error"]["code"], "invalid_bencode");
        assert!(!status.to_string().contains("sentinel-passkey"));
        let deadline = Inspector::with_parser(
            Arc::new(torrent::inspect_metainfo),
            RESULT_TTL,
            Duration::ZERO,
        )
        .unwrap();
        let expired = id(deadline.begin(Principal::Anonymous, &params()).unwrap());
        assert_eq!(
            terminal(&deadline, Principal::Anonymous, &expired)["error"]["code"],
            "inspection_deadline_exceeded"
        );
        let valid = id(inspector.begin(Principal::Anonymous, &params()).unwrap());
        assert_eq!(
            terminal(&inspector, Principal::Anonymous, &valid)["phase"],
            "ready"
        );
    }
}
