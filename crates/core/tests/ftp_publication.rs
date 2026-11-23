//! Public Core consumer on the platform supporting directory-relative FTP publication.
#![cfg(target_os = "macos")]

#[path = "support/ftp_server.rs"]
mod ftp_server;

use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::SocketAddr,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use nexum_core::{Core, CoreError, ftp_resume::FtpResumeAttempt};
use nexum_domain::{Destination, DownloadSource, Progress, TaskId};
use nexum_engine::ftp::{self, FtpCompletionRequest, FtpCredentials, FtpTransferControl};
use nexum_scheduler::{Priority, SchedulerConfig, SchedulerEvent};
use nexum_storage::{
    SqliteRepository, StorageError, StoredTask, TaskRepository,
    ftp_publication::FtpPublicationPhase,
    origin::{OriginRecord, Reservation, random_id},
};
use nexum_task::{DownloadTask, TaskEvent, TaskState};
use rusqlite::Connection;

type Owner = Core<SqliteRepository>;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("nexum-ftp-publication-{}", random_id().unwrap()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        fs::create_dir(root.join("downloads")).unwrap();
        fs::write(root.join("downloads/sentinel"), b"untouched").unwrap();
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn id() -> TaskId {
    "ftp".into()
}
fn full() -> Progress {
    Progress::new(8, Some(8))
}
fn open(root: &Path) -> Owner {
    Core::with_repository(
        SchedulerConfig::default(),
        SqliteRepository::open(root.join("state.db")).unwrap(),
    )
    .unwrap()
}
fn sql(root: &Path, statement: &str) {
    Connection::open(root.join("state.db"))
        .unwrap()
        .execute_batch(statement)
        .unwrap();
}
fn entries(root: &Path) -> (File, File) {
    (
        File::open(root.join("downloads/partial")).unwrap(),
        File::open(root.join("downloads")).unwrap(),
    )
}
fn create(owner: &mut Owner, root: &Path, endpoint: SocketAddr) -> OriginRecord {
    let origin = format!("ftp://{endpoint}");
    let record = OriginRecord {
        version: 1,
        task_id: id().as_str().into(),
        destination: root.join("downloads/final").to_str().unwrap().into(),
        canonical_url: format!("{origin}/pub/a.bin"),
        origin,
        command_path: "pub/a.bin".into(),
        path_mode: "login_relative".into(),
        security: "plain".into(),
        authenticated: false,
        operation_id: random_id().unwrap(),
        credential: None,
    };
    assert_eq!(
        owner.repository.reserve_origin(&record).unwrap(),
        Reservation::New
    );
    owner
        .repository
        .activate_origin(
            &record,
            DownloadTask::new(
                id(),
                DownloadSource::new(&record.canonical_url),
                Destination::new(&record.destination),
            )
            .into(),
        )
        .unwrap();
    record
}
fn receive(owner: &mut Owner, root: &Path, endpoint: SocketAddr) -> FtpResumeAttempt {
    create(owner, root, endpoint);
    owner.recover().unwrap();
    owner.queue_task(&id(), Priority::NORMAL).unwrap();
    assert!(owner.claim_queued(&id()).unwrap());
    let attempt = owner.claim_ftp_attempt(&id(), None).unwrap();
    let mut file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(root.join("downloads/partial"))
        .unwrap();
    let dir = File::open(root.join("downloads")).unwrap();
    let source: SocketAddr = attempt
        .binding()
        .source_origin
        .strip_prefix("ftp://")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(source, endpoint);
    let proof = ftp::receive_complete(
        source,
        FtpCredentials::Anonymous,
        &mut file,
        &dir,
        FtpCompletionRequest {
            binding: attempt.binding(),
            attempt_generation: attempt.generation(),
        },
        &FtpTransferControl::default(),
    )
    .unwrap();
    let saved = owner.save_ftp_completion(&attempt, &proof).unwrap();
    assert_eq!(saved.completion_generation(), Some(attempt.generation()));
    owner.drain_task_events();
    owner.drain_scheduler_events();
    saved
}
fn seeded() -> (Fixture, Owner, FtpResumeAttempt) {
    let fixture = Fixture::new();
    let (endpoint, origin) = ftp_server::origin(false);
    let mut owner = open(&fixture.0);
    let saved = receive(&mut owner, &fixture.0, endpoint);
    origin.join().unwrap();
    (fixture, owner, saved)
}

fn assert_task(owner: &Owner, state: TaskState, active: usize) {
    let task = owner.tasks.get(&id()).unwrap().clone();
    assert_eq!(task.state, state);
    assert_eq!(task.progress, full());
    assert_eq!(
        owner.repository.get(&id()).unwrap().unwrap(),
        StoredTask::from(task)
    );
    assert_eq!(owner.scheduler.active_len(), active);
}
fn no_events(owner: &mut Owner) {
    assert!(owner.drain_task_events().is_empty());
    assert!(owner.drain_scheduler_events().is_empty());
}
fn assert_completed_events(owner: &mut Owner) {
    assert_eq!(
        owner.drain_scheduler_events(),
        vec![SchedulerEvent::Completed { task_id: id() }]
    );
    assert_eq!(
        owner.drain_task_events(),
        vec![
            TaskEvent::ProgressChanged {
                task_id: id(),
                progress: full()
            },
            TaskEvent::StateChanged {
                task_id: id(),
                from: TaskState::Downloading,
                to: TaskState::Completed
            },
        ]
    );
}
const REJECT_COMPLETED: &str = "CREATE TRIGGER refuse BEFORE UPDATE OF phase ON ftp_publications WHEN NEW.phase='completed' BEGIN SELECT RAISE(ABORT,'reject journal'); END";

#[test]
fn recovery_claim_preserves_whole_progress_scheduler_and_events() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    owner
        .fail_task_without_retry(&id(), "pending publish")
        .unwrap();
    let other = owner
        .create_task(
            "ordinary",
            DownloadSource::new("https://example.org/file"),
            Destination::new("other"),
        )
        .unwrap();
    owner.queue_task(&other.id, Priority::NORMAL).unwrap();
    assert!(owner.claim_queued(&other.id).unwrap());
    owner
        .fail_task_without_retry(&other.id, "ordinary failure")
        .unwrap();
    drop(owner);
    let mut owner = open(&f.0);
    assert_eq!(owner.recover_with_ftp_publications().unwrap(), 2);
    assert_eq!(owner.tasks.get(&other.id).unwrap().state, TaskState::Failed);
    assert_task(&owner, TaskState::Queued, 0);
    let previous = owner.load_ftp_attempt(&id()).unwrap().unwrap();
    owner.drain_task_events();
    owner.drain_scheduler_events();
    sql(
        &f.0,
        "CREATE TRIGGER refuse BEFORE UPDATE OF state ON tasks BEGIN SELECT RAISE(ABORT,'reject claim'); END",
    );
    assert!(matches!(
        owner.claim_queued_ftp_completion(&previous),
        Err(CoreError::Storage(_))
    ));
    assert_task(&owner, TaskState::Queued, 0);
    assert_eq!(owner.scheduler.queued_len(), 1);
    assert_eq!(
        owner.load_ftp_attempt(&id()).unwrap().unwrap().generation(),
        previous.generation()
    );
    no_events(&mut owner);
    sql(&f.0, "DROP TRIGGER refuse");
    let next = owner
        .claim_queued_ftp_completion(&previous)
        .unwrap()
        .unwrap();
    assert_ne!(next.generation(), saved.generation());
    assert_eq!(next.completion_generation(), Some(saved.generation()));
    assert_task(&owner, TaskState::Downloading, 1);
    assert_eq!(owner.tasks.get(&id()).unwrap().last_error, None);
    assert_eq!(
        owner.drain_scheduler_events(),
        vec![SchedulerEvent::Started { task_id: id() }]
    );
    assert_eq!(
        owner.drain_task_events(),
        vec![TaskEvent::StateChanged {
            task_id: id(),
            from: TaskState::Queued,
            to: TaskState::Downloading
        }]
    );
}

#[test]
fn intent_and_completion_write_faults_preserve_memory_and_events() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    sql(
        &f.0,
        "CREATE TRIGGER refuse BEFORE UPDATE OF publication_seen ON ftp_attempts BEGIN SELECT RAISE(ABORT,'reject intent'); END",
    );
    assert!(matches!(
        owner.prepare_ftp_publication(&saved, &file, &dir, "partial"),
        Err(CoreError::Storage(_))
    ));
    assert_eq!(
        owner.load_ftp_attempt(&id()).unwrap().unwrap().revision(),
        saved.revision()
    );
    assert!(!f.0.join("downloads/final").exists());
    assert_task(&owner, TaskState::Downloading, 1);
    no_events(&mut owner);
    sql(&f.0, "DROP TRIGGER refuse");
    let intent = owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    let db = Connection::open(f.0.join("state.db")).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let control = FtpTransferControl::default();
    assert!(matches!(
        owner.publish_ftp_completion(&intent, &dir, &control),
        Err(CoreError::Storage(StorageError::Busy))
    ));
    assert!(control.is_finished());
    assert!(!control.is_committed());
    assert!(!f.0.join("downloads/final").exists());
    db.execute_batch("ROLLBACK").unwrap();
    let published = owner
        .publish_ftp_completion(&intent, &dir, &FtpTransferControl::default())
        .unwrap();
    sql(&f.0, REJECT_COMPLETED);
    assert!(matches!(
        owner.complete_ftp_publication(&published),
        Err(CoreError::Storage(_))
    ));
    assert_task(&owner, TaskState::Downloading, 1);
    no_events(&mut owner);
    assert!(owner.completed_ftp_publications().unwrap().is_empty());
    assert_eq!(fs::read(f.0.join("downloads/final")).unwrap(), b"firstend");
    sql(&f.0, "DROP TRIGGER refuse");
    owner.complete_ftp_publication(&published).unwrap();
    assert_task(&owner, TaskState::Completed, 0);
}

#[test]
fn atomic_completed_releases_slot_only_after_sqlite_commit() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    let intent = owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    let published = owner
        .publish_ftp_completion(&intent, &dir, &FtpTransferControl::default())
        .unwrap();
    assert_task(&owner, TaskState::Downloading, 1);
    no_events(&mut owner);
    for (column, value, original) in [
        ("speed_bytes_per_second", "17", "0"),
        ("eta_seconds", "2", "NULL"),
        ("last_error", "'unexpected'", "NULL"),
    ] {
        sql(&f.0, &format!("UPDATE tasks SET {column}={value}"));
        assert!(matches!(
            owner.complete_ftp_publication(&published),
            Err(CoreError::Storage(_))
        ));
        assert_eq!(owner.scheduler.active_len(), 1);
        no_events(&mut owner);
        sql(&f.0, &format!("UPDATE tasks SET {column}={original}"));
    }
    owner
        .tasks
        .set_error(&id(), Some("changed in memory".into()))
        .unwrap();
    assert!(matches!(
        owner.complete_ftp_publication(&published),
        Err(CoreError::Storage(_))
    ));
    assert_eq!(
        owner.repository.get(&id()).unwrap().unwrap().last_error,
        None
    );
    assert_eq!(owner.scheduler.active_len(), 1);
    no_events(&mut owner);
    owner.tasks.set_error(&id(), None).unwrap();
    owner.complete_ftp_publication(&published).unwrap();
    assert_task(&owner, TaskState::Completed, 0);
    assert_completed_events(&mut owner);
    let diagnostics = owner.completed_ftp_publications().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].operation_id, saved.binding().operation_id);
    assert_eq!(diagnostics[0].phase, FtpPublicationPhase::Completed);
    assert!(owner.load_ftp_attempt(&id()).is_err());
    assert!(owner.complete_ftp_publication(&published).is_err());
    no_events(&mut owner);
}

#[test]
fn stale_owner_cannot_publish_or_save_completed() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    let intent = owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    owner
        .tasks
        .set_error(&id(), Some("changed in memory".into()))
        .unwrap();
    let control = FtpTransferControl::default();
    assert!(matches!(
        owner.publish_ftp_completion(&intent, &dir, &control),
        Err(CoreError::Storage(_))
    ));
    assert!(control.is_finished());
    assert!(!f.0.join("downloads/final").exists());
    owner.tasks.set_error(&id(), None).unwrap();
    sql(&f.0, "UPDATE tasks SET speed_bytes_per_second=1");
    let control = FtpTransferControl::default();
    assert!(matches!(
        owner.publish_ftp_completion(&intent, &dir, &control),
        Err(CoreError::Storage(_))
    ));
    assert!(control.is_finished());
    assert!(!f.0.join("downloads/final").exists());
    sql(&f.0, "UPDATE tasks SET speed_bytes_per_second=0");
    let published = owner
        .publish_ftp_completion(&intent, &dir, &FtpTransferControl::default())
        .unwrap();
    let mut successor = open(&f.0);
    successor.recover_with_ftp_publications().unwrap();
    let prev = successor.load_ftp_attempt(&id()).unwrap().unwrap();
    let claimed = successor
        .claim_queued_ftp_completion(&prev)
        .unwrap()
        .unwrap();
    let new_intent = successor
        .rearm_ftp_publication(&claimed, &file, &dir)
        .unwrap();
    assert!(matches!(
        owner.reconcile_ftp_publication(&intent, &dir, &FtpTransferControl::default()),
        Err(CoreError::Storage(_))
    ));
    assert!(matches!(
        owner.complete_ftp_publication(&published),
        Err(CoreError::Storage(_))
    ));
    no_events(&mut owner);
    let adopted = successor
        .reconcile_ftp_publication(&new_intent, &dir, &FtpTransferControl::default())
        .unwrap();
    successor.complete_ftp_publication(&adopted).unwrap();
    assert_task(&successor, TaskState::Completed, 0);
}

#[test]
fn recreated_task_id_cannot_adopt_prior_intent() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    let intent = owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    owner.remove_task(&id()).unwrap();
    let orphan = owner.orphan_ftp_publications().unwrap();
    assert_eq!(orphan.len(), 1);
    assert_eq!(orphan[0].operation_id, saved.binding().operation_id);
    let endpoint = saved
        .binding()
        .source_origin
        .trim_start_matches("ftp://")
        .parse()
        .unwrap();
    let replacement = create(&mut owner, &f.0, endpoint);
    assert_ne!(replacement.operation_id, orphan[0].operation_id);
    owner.recover_with_ftp_publications().unwrap();
    owner.queue_task(&id(), Priority::NORMAL).unwrap();
    assert!(owner.claim_queued(&id()).unwrap());
    assert!(owner.load_ftp_attempt(&id()).unwrap().is_none());
    assert!(
        owner
            .reconcile_ftp_publication(&intent, &dir, &FtpTransferControl::default())
            .is_err()
    );
    assert_eq!(owner.orphan_ftp_publications().unwrap(), orphan);
    assert!(!f.0.join("downloads/final").exists());
    assert_eq!(
        fs::read(f.0.join("downloads/partial")).unwrap(),
        b"firstend"
    );
}

#[test]
fn foreign_existing_final_is_preserved_and_conflicting_staging_retains_committed_fact() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    let intent = owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    let final_path = f.0.join("downloads/final");
    fs::write(&final_path, b"firstend").unwrap();
    let ino = fs::metadata(&final_path).unwrap().ino();
    for recovery in [false, true] {
        let control = FtpTransferControl::default();
        let result = if recovery {
            owner.reconcile_ftp_publication(&intent, &dir, &control)
        } else {
            owner.publish_ftp_completion(&intent, &dir, &control)
        };
        assert!(result.is_err());
        assert!(control.is_finished());
        assert!(!control.is_committed());
        assert_eq!(fs::metadata(&final_path).unwrap().ino(), ino);
    }
    fs::remove_file(&final_path).unwrap();
    owner
        .publish_ftp_completion(&intent, &dir, &FtpTransferControl::default())
        .unwrap();
    let final_ino = fs::metadata(&final_path).unwrap().ino();
    let staging = f.0.join("downloads/partial");
    fs::remove_file(&staging).unwrap();
    fs::write(&staging, b"foreign").unwrap();
    let control = FtpTransferControl::default();
    assert!(
        owner
            .reconcile_ftp_publication(&intent, &dir, &control)
            .is_err()
    );
    assert!(control.is_finished());
    assert!(control.is_committed());
    assert_eq!(fs::metadata(&final_path).unwrap().ino(), final_ino);
    assert_eq!(fs::read(&staging).unwrap(), b"foreign");
    assert_eq!(fs::read(&final_path).unwrap(), b"firstend");
    assert_task(&owner, TaskState::Downloading, 1);
    no_events(&mut owner);
}

const CHILD_ROOT: &str = "NEXUM_PUBLICATION_CHILD_ROOT";
const CRASH_POINT: &str = "NEXUM_PUBLICATION_POINT";
const PROCESS_TEST: &str = "real_ftp_publication_converges_offline_after_process_crash";

// The parent owns both private pipes. The marker is sent only once the exact
// durable/public API boundary is observable; the producer cannot advance.
fn park(producer: &str) -> ! {
    println!("NEXUM-PARKED {producer}");
    std::io::stdout().flush().unwrap();
    let mut command = [0];
    let _ = std::io::stdin().read_exact(&mut command);
    panic!("producer's parent closed or wrote to the gate without killing it")
}

fn recover_child(root: &Path, producer: &str) {
    let mut owner = open(root);
    let downloads = root.join("downloads");
    let dir = File::open(&downloads).unwrap();
    assert!(
        std::env::var_os(CRASH_POINT).is_none(),
        "restart inherited the producer crash point"
    );
    assert_eq!(owner.recover_with_ftp_publications().unwrap(), 1);
    assert_task(&owner, TaskState::Queued, 0);
    let old = owner.load_ftp_attempt(&id()).unwrap().unwrap();
    let endpoint: SocketAddr = old
        .binding()
        .source_origin
        .strip_prefix("ftp://")
        .unwrap()
        .parse()
        .unwrap();
    assert!(std::net::TcpStream::connect(endpoint).is_err());
    assert_eq!(old.completion_generation(), Some(producer));
    let claimed = owner.claim_queued_ftp_completion(&old).unwrap().unwrap();
    assert_ne!(claimed.generation(), producer);
    assert_eq!(claimed.completion_generation(), Some(producer));
    assert_task(&owner, TaskState::Downloading, 1);
    assert_eq!(
        owner.drain_scheduler_events(),
        vec![SchedulerEvent::Started { task_id: id() }]
    );
    owner.drain_task_events();
    let origin = owner.repository.active_origin(&id()).unwrap().unwrap();
    let stored = owner.repository.ftp_attempt(&origin).unwrap().unwrap();
    assert_eq!(stored.generation, claimed.generation());
    let intent = if stored.publication.is_some() {
        let held = match File::open(&claimed.binding().destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                File::open(downloads.join("partial")).unwrap()
            }
            opened => opened.unwrap(),
        };
        owner.rearm_ftp_publication(&claimed, &held, &dir).unwrap()
    } else {
        let staged = File::open(downloads.join("partial")).unwrap();
        owner
            .prepare_ftp_publication(&claimed, &staged, &dir, "partial")
            .unwrap()
    };
    let control = FtpTransferControl::default();
    let result = owner
        .reconcile_ftp_publication(&intent, &dir, &control)
        .unwrap();
    assert!(control.is_finished() && control.is_committed());
    owner.complete_ftp_publication(&result).unwrap();
    assert_completed_events(&mut owner);
    assert_task(&owner, TaskState::Completed, 0);
    assert_eq!(owner.completed_ftp_publications().unwrap().len(), 1);
}

fn producer_child(root: &Path, point: &str, endpoint: SocketAddr) {
    let mut owner = open(root);
    let downloads = root.join("downloads");
    let dir = File::open(&downloads).unwrap();
    let saved = receive(&mut owner, root, endpoint);
    if point == "proof" {
        park(saved.generation());
    }
    let file = File::open(downloads.join("partial")).unwrap();
    let intent = owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    if point == "intent-before-link" {
        park(saved.generation());
    }
    let control = FtpTransferControl::default();
    let published = owner
        .publish_ftp_completion(&intent, &dir, &control)
        .unwrap();
    assert!(control.is_committed());
    if point == "refused" {
        sql(root, REJECT_COMPLETED);
        assert!(matches!(
            owner.complete_ftp_publication(&published),
            Err(CoreError::Storage(_))
        ));
        assert_task(&owner, TaskState::Downloading, 1);
        no_events(&mut owner);
    }
    park(saved.generation())
}

#[test]
fn real_ftp_publication_converges_offline_after_process_crash() {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        if let Ok(producer) = std::env::var("NEXUM_PUBLICATION_RECOVER") {
            recover_child(Path::new(&root), &producer);
        } else {
            producer_child(
                Path::new(&root),
                &std::env::var(CRASH_POINT).unwrap(),
                std::env::var("NEXUM_PUBLICATION_ENDPOINT")
                    .unwrap()
                    .parse()
                    .unwrap(),
            );
        }
        return;
    }
    for point in [
        "proof",
        "intent-before-link",
        "after-link",
        "refused",
        "final-only",
    ] {
        let f = Fixture::new();
        let (endpoint, source) = ftp_server::origin(false);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", PROCESS_TEST, "--nocapture"])
            .env(CHILD_ROOT, &f.0)
            .env(CRASH_POINT, point)
            .env_remove("NEXUM_PUBLICATION_RECOVER")
            .env("NEXUM_PUBLICATION_ENDPOINT", endpoint.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        let mut child = command.spawn().unwrap();
        // Child::wait closes an attached stdin handle. Keep the parent end
        // alive until wait observes SIGKILL, including on a delayed delivery.
        let gate = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some((_, producer)) = line.split_once("NEXUM-PARKED ") {
                    tx.send(producer.to_owned()).unwrap();
                    return;
                }
            }
        });
        let parked = rx.recv_timeout(Duration::from_secs(20));
        let killed = child.kill();
        let status = child.wait().unwrap();
        drop(gate);
        reader.join().unwrap();
        let producer = parked
            .unwrap_or_else(|err| panic!("{point} producer exited {status} before gate: {err}"));
        killed.unwrap();
        assert_eq!(status.signal(), Some(9));
        source.join().unwrap();
        assert!(std::net::TcpStream::connect(endpoint).is_err());

        let staging = f.0.join("downloads/partial");
        let final_path = f.0.join("downloads/final");
        let inode = fs::metadata(&staging).unwrap().ino();
        let linked = !matches!(point, "proof" | "intent-before-link");
        assert_eq!(final_path.exists(), linked);
        if linked {
            assert_eq!(fs::metadata(&final_path).unwrap().ino(), inode);
        }
        if point == "final-only" {
            fs::remove_file(&staging).unwrap();
        }
        if point == "refused" {
            sql(&f.0, "DROP TRIGGER refuse");
        }
        let before = open(&f.0);
        assert_eq!(
            before.repository.get(&id()).unwrap().unwrap().state,
            TaskState::Downloading
        );
        assert!(before.completed_ftp_publications().unwrap().is_empty());
        drop(before);

        let recovery = command
            .env_remove(CRASH_POINT)
            .env_remove("NEXUM_PUBLICATION_ENDPOINT")
            .env("NEXUM_PUBLICATION_RECOVER", producer)
            .output()
            .unwrap();
        assert!(
            recovery.status.success(),
            "{point}: {} {}",
            String::from_utf8_lossy(&recovery.stdout),
            String::from_utf8_lossy(&recovery.stderr)
        );
        assert_eq!(fs::metadata(&final_path).unwrap().ino(), inode);
        if point == "final-only" {
            assert!(!staging.exists());
        } else {
            assert_eq!(fs::metadata(&staging).unwrap().ino(), inode);
        }
        let digest = Command::new("/usr/bin/shasum")
            .args(["-a", "256"])
            .arg(&final_path)
            .output()
            .unwrap();
        assert!(digest.status.success());
        assert_eq!(
            String::from_utf8(digest.stdout)
                .unwrap()
                .split_whitespace()
                .next(),
            Some("51981fe39b8d2abefd9767a4f59bbb171afa26a031a60eb71ae2bc3917598a55")
        );
        assert_eq!(
            fs::read(f.0.join("downloads/sentinel")).unwrap(),
            b"untouched"
        );
        let mut after = open(&f.0);
        after.recover_with_ftp_publications().unwrap();
        assert_task(&after, TaskState::Completed, 0);
        assert_eq!(after.completed_ftp_publications().unwrap().len(), 1);
    }
}

#[test]
fn publication_debug_and_errors_redact_paths_proofs_and_authority() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    let private_leaf = "ftp-passkey-SENTINEL-artifact-staging";
    fs::rename(
        f.0.join("downloads/partial"),
        f.0.join("downloads").join(private_leaf),
    )
    .unwrap();
    let prepared = owner
        .prepare_ftp_publication(&saved, &file, &dir, private_leaf)
        .unwrap();
    let origin = owner.repository.active_origin(&id()).unwrap().unwrap();
    let stored = owner.repository.ftp_attempt(&origin).unwrap().unwrap();
    let auth = &stored.publication.as_ref().unwrap().authorization;
    let cancelled = FtpTransferControl::default();
    cancelled.cancel();
    let error = owner
        .reconcile_ftp_publication(&prepared, &dir, &cancelled)
        .unwrap_err();
    assert_eq!(
        error,
        CoreError::Engine(nexum_engine::EngineError::Cancelled)
    );
    assert!(cancelled.is_finished());
    assert!(!f.0.join("downloads/final").exists());
    let published = owner
        .publish_ftp_completion(&prepared, &dir, &FtpTransferControl::default())
        .unwrap();
    let rendered = format!("{saved:?} {prepared:?} {published:?} {error:?}");
    for secret in [
        f.0.to_str().unwrap(),
        saved.binding().source_origin.as_str(),
        private_leaf,
        auth,
        saved.generation(),
        "ftp-held-completion/v1",
    ] {
        assert!(
            !rendered.contains(secret),
            "FTP diagnostic included private input"
        );
    }
}

#[test]
fn retained_plan_is_validated_before_claim_and_rearm_failures_leave_old_authority() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    drop(owner);
    let db = Connection::open(f.0.join("state.db")).unwrap();
    let original: Vec<u8> = db
        .query_row("SELECT plan FROM ftp_publications", [], |row| row.get(0))
        .unwrap();
    let mut owner = open(&f.0);
    owner.recover_with_ftp_publications().unwrap();
    owner.drain_task_events();
    owner.drain_scheduler_events();
    sql(&f.0, "UPDATE ftp_publications SET plan=X'7B7D'");
    let invalid = owner.load_ftp_attempt(&id()).unwrap().unwrap();
    assert!(matches!(
        owner.claim_queued_ftp_completion(&invalid),
        Err(CoreError::Engine(_))
    ));
    assert_task(&owner, TaskState::Queued, 0);
    assert_eq!(owner.scheduler.queued_len(), 1);
    no_events(&mut owner);
    let mut spaced = original;
    spaced.push(b' ');
    db.execute("UPDATE ftp_publications SET plan=?1", [&spaced])
        .unwrap();
    let previous = owner.load_ftp_attempt(&id()).unwrap().unwrap();
    let next = owner
        .claim_queued_ftp_completion(&previous)
        .unwrap()
        .unwrap();
    owner.drain_task_events();
    owner.drain_scheduler_events();
    sql(
        &f.0,
        "CREATE TRIGGER refuse BEFORE UPDATE OF publication_authorization ON ftp_attempts BEGIN SELECT RAISE(ABORT,'reject rearm'); END",
    );
    assert!(matches!(
        owner.rearm_ftp_publication(&next, &file, &dir),
        Err(CoreError::Storage(_))
    ));
    assert_task(&owner, TaskState::Downloading, 1);
    no_events(&mut owner);
    let retained: (Vec<u8>, String, u64) = db
        .query_row(
            "SELECT plan,owner_generation,revision FROM ftp_publications",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        retained,
        (
            spaced,
            previous.generation().to_owned(),
            previous.revision()
        )
    );
    sql(&f.0, "DROP TRIGGER refuse");
    let prepared = owner.rearm_ftp_publication(&next, &file, &dir).unwrap();
    let published = owner
        .reconcile_ftp_publication(&prepared, &dir, &FtpTransferControl::default())
        .unwrap();
    owner.complete_ftp_publication(&published).unwrap();
    assert_task(&owner, TaskState::Completed, 0);
}

#[test]
fn damaged_completion_preserves_evidence_at_publication_fences_and_failed_startup() {
    let (f, mut owner, saved) = seeded();
    let (file, dir) = entries(&f.0);
    let prepared = owner
        .prepare_ftp_publication(&saved, &file, &dir, "partial")
        .unwrap();
    let db = Connection::open(f.0.join("state.db")).unwrap();
    let damage = || {
        db.execute(
            "UPDATE ftp_attempts SET completion_proof=zeroblob(65537)",
            [],
        )
        .unwrap()
    };
    let journal = || {
        db.query_row(
            "SELECT phase,authorization,owner_generation,revision,plan FROM ftp_publications",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .unwrap()
    };
    let retained = journal();
    damage();
    let control = FtpTransferControl::default();
    assert!(matches!(
        owner.publish_ftp_completion(&prepared, &dir, &control),
        Err(CoreError::Storage(_))
    ));
    assert!(control.is_finished() && !control.is_committed());
    assert_task(&owner, TaskState::Downloading, 1);
    no_events(&mut owner);
    assert!(!f.0.join("downloads/final").exists());
    assert_eq!(
        fs::read(f.0.join("downloads/partial")).unwrap(),
        b"firstend"
    );
    assert_eq!(journal(), retained);

    db.execute(
        "UPDATE ftp_attempts SET completion_proof=?1",
        [saved.completion().unwrap().encode().unwrap()],
    )
    .unwrap();
    let published = owner
        .publish_ftp_completion(&prepared, &dir, &FtpTransferControl::default())
        .unwrap();
    damage();
    assert!(matches!(
        owner.complete_ftp_publication(&published),
        Err(CoreError::Storage(_))
    ));
    assert_task(&owner, TaskState::Downloading, 1);
    no_events(&mut owner);
    assert_eq!(journal(), retained);
    owner
        .fail_task_without_retry(&id(), "pending publication")
        .unwrap();
    drop(owner);
    let mut restarted = open(&f.0);
    assert!(matches!(
        restarted.recover_with_ftp_publications(),
        Err(CoreError::Storage(_))
    ));
    let task = restarted.repository.get(&id()).unwrap().unwrap();
    assert_eq!(task.state, TaskState::Failed);
    assert_eq!(task.progress, full());
    assert!(restarted.tasks.get(&id()).is_none());
    assert_eq!(restarted.scheduler.queued_len(), 0);
    assert_eq!(restarted.scheduler.active_len(), 0);
    no_events(&mut restarted);
    assert_eq!(journal(), retained);
    assert_eq!(
        db.query_row(
            "SELECT length(completion_proof) FROM ftp_attempts",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        65537
    );
    assert_eq!(fs::read(f.0.join("downloads/final")).unwrap(), b"firstend");
    assert_eq!(
        file.metadata().unwrap().ino(),
        fs::metadata(f.0.join("downloads/final")).unwrap().ino()
    );
}
