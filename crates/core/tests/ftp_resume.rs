use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;

use nexum_core::{Core, ftp_resume::FtpResumeAttempt};
use nexum_domain::{Destination, DownloadSource, Progress, TaskId};
use nexum_engine::ftp::{
    self, FtpCheckpointRequest, FtpCredentials, FtpPrefixProof, FtpReceiveError, FtpResumeRequest,
    FtpTransferControl,
};
use nexum_scheduler::SchedulerConfig;
use nexum_storage::{
    SqliteRepository, TaskRepository,
    origin::{OriginRecord, Reservation, random_id},
};
use nexum_task::DownloadTask;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("nexum-ftp-persistence-{}", random_id().unwrap()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn db(&self) -> PathBuf {
        self.0.join("nexum.sqlite")
    }
    fn file(&self) -> File {
        File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(self.0.join("partial"))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn core(path: PathBuf) -> Core<SqliteRepository> {
    Core::with_repository(
        SchedulerConfig::default(),
        SqliteRepository::open(path).unwrap(),
    )
    .unwrap()
}
fn create(core: &mut Core<SqliteRepository>) -> OriginRecord {
    let record = OriginRecord {
        version: 1,
        task_id: "ftp".into(),
        destination: "/test/file.bin".into(),
        canonical_url: "ftp://example.com/pub/a.bin".into(),
        origin: "ftp://example.com:21".into(),
        command_path: "pub/a.bin".into(),
        path_mode: "login_relative".into(),
        security: "plain".into(),
        authenticated: false,
        operation_id: random_id().unwrap(),
        credential: None,
    };
    assert_eq!(
        core.repository.reserve_origin(&record).unwrap(),
        Reservation::New
    );
    core.repository
        .activate_origin(
            &record,
            DownloadTask::new(
                "ftp",
                DownloadSource::new(&record.canonical_url),
                Destination::new(&record.destination),
            )
            .into(),
        )
        .unwrap();
    core.recover().unwrap();
    core.drain_task_events();
    record
}
fn proof(
    file: &mut File,
    directory: &File,
    attempt: &FtpResumeAttempt,
    bytes: u64,
) -> FtpPrefixProof {
    ftp::checkpoint(
        FtpCheckpointRequest {
            binding: attempt.binding(),
            attempt_generation: attempt.generation(),
            trusted_bytes: bytes,
            total_bytes: Some(8),
        },
        file,
        directory,
        &FtpTransferControl::default(),
    )
    .unwrap()
}

#[test]
fn core_saves_exact_engine_proof_and_progress_and_reclaims_with_the_old_proof() {
    let fixture = Fixture::new();
    let mut core = core(fixture.db());
    create(&mut core);
    let id = TaskId::from("ftp");
    let first = core.claim_ftp_attempt(&id, None).unwrap();
    let mut file = fixture.file();
    file.write_all(b"first").unwrap();
    let directory = File::open(&fixture.0).unwrap();
    let proof = proof(&mut file, &directory, &first, 5);
    let saved = core.save_ftp_checkpoint(&first, &proof).unwrap();
    assert_eq!(saved.revision(), 1);
    assert_eq!(
        saved.proof().unwrap().encode().unwrap(),
        proof.encode().unwrap()
    );
    assert_eq!(
        core.tasks.get(&id).unwrap().progress,
        Progress::new(5, Some(8))
    );
    assert_eq!(
        core.repository.get(&id).unwrap().unwrap().progress,
        Progress::new(5, Some(8))
    );
    core.drain_task_events();
    assert!(core.save_ftp_checkpoint(&first, &proof).is_err());
    assert!(core.drain_task_events().is_empty());
    let next = core.claim_ftp_attempt(&id, Some(&saved)).unwrap();
    assert_ne!(next.generation(), saved.generation());
    assert_eq!(
        next.proof().unwrap().attempt_generation(),
        saved.generation()
    );
    assert!(core.save_ftp_checkpoint(&saved, &proof).is_err());
    assert_eq!(
        core.load_ftp_attempt(&id).unwrap().unwrap().generation(),
        next.generation()
    );
}

#[test]
fn database_failures_roll_back_checkpoint_progress_and_core_events() {
    for statement in [
        "CREATE TRIGGER reject_save BEFORE UPDATE OF proof ON ftp_attempts BEGIN SELECT RAISE(ABORT,'reject checkpoint'); END",
        "CREATE TRIGGER reject_save BEFORE UPDATE OF downloaded_bytes ON tasks BEGIN SELECT RAISE(ABORT,'reject task'); END",
    ] {
        let fixture = Fixture::new();
        let mut core = core(fixture.db());
        create(&mut core);
        let id = TaskId::from("ftp");
        let attempt = core.claim_ftp_attempt(&id, None).unwrap();
        let mut file = fixture.file();
        file.write_all(b"first").unwrap();
        let proof = proof(&mut file, &File::open(&fixture.0).unwrap(), &attempt, 5);
        rusqlite::Connection::open(fixture.db())
            .unwrap()
            .execute_batch(statement)
            .unwrap();
        assert!(core.save_ftp_checkpoint(&attempt, &proof).is_err());
        let stored = core.load_ftp_attempt(&id).unwrap().unwrap();
        assert_eq!(stored.revision(), 0);
        assert!(stored.proof().is_none());
        assert_eq!(core.tasks.get(&id).unwrap().progress, Progress::default());
        assert_eq!(
            core.repository.get(&id).unwrap().unwrap().progress,
            Progress::default()
        );
        assert!(core.drain_task_events().is_empty());
        assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"first");
    }
}

#[test]
fn only_one_database_connection_can_reclaim_the_same_snapshot() {
    let fixture = Fixture::new();
    let mut owner = core(fixture.db());
    let record = create(&mut owner);
    let original = owner.repository.claim_ftp_attempt(&record, None).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let workers = (0..2)
        .map(|_| {
            let mut repository = SqliteRepository::open(fixture.db()).unwrap();
            let record = record.clone();
            let original = original.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                repository.claim_ftp_attempt(&record, Some(&original))
            })
        })
        .collect::<Vec<_>>();
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_err()).count(),
        1
    );
    let current = owner.repository.ftp_attempt(&record).unwrap().unwrap();
    assert_ne!(current.generation, original.generation);
}

#[test]
fn failed_checkpoint_preserves_old_proof_and_rejects_the_unsaved_tail() {
    let fixture = Fixture::new();
    let mut core = core(fixture.db());
    create(&mut core);
    let id = TaskId::from("ftp");
    let first = core.claim_ftp_attempt(&id, None).unwrap();
    let mut file = fixture.file();
    file.write_all(b"first").unwrap();
    let directory = File::open(&fixture.0).unwrap();
    let first_proof = proof(&mut file, &directory, &first, 5);
    let saved = core.save_ftp_checkpoint(&first, &first_proof).unwrap();
    use std::io::Seek;
    file.seek(std::io::SeekFrom::End(0)).unwrap();
    file.write_all(b"end").unwrap();
    let unsaved = proof(&mut file, &directory, &saved, 8);
    rusqlite::Connection::open(fixture.db()).unwrap().execute_batch("CREATE TRIGGER reject_save BEFORE UPDATE OF proof ON ftp_attempts BEGIN SELECT RAISE(ABORT,'save rejected'); END").unwrap();
    assert!(core.save_ftp_checkpoint(&saved, &unsaved).is_err());
    let current = core.load_ftp_attempt(&id).unwrap().unwrap();
    assert_eq!(current.revision(), saved.revision());
    let next = core.claim_ftp_attempt(&id, Some(&current)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let result = ftp::resume(
        listener.local_addr().unwrap(),
        FtpCredentials::Anonymous,
        &mut file,
        &directory,
        FtpResumeRequest {
            binding: next.binding(),
            attempt_generation: next.generation(),
            proof: next.proof().unwrap(),
        },
        &FtpTransferControl::default(),
    );
    assert_eq!(result, Err(FtpReceiveError::UncommittedTail));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"firstend");
    assert_eq!(
        core.repository.get(&id).unwrap().unwrap().progress,
        Progress::new(5, Some(8))
    );
}

#[test]
fn repairs_a_rejected_checkpoint_tail_then_resumes_using_the_sqlite_proof() {
    let fixture = Fixture::new();
    let mut owner = core(fixture.db());
    create(&mut owner);
    let id = TaskId::from("ftp");
    let first = owner.claim_ftp_attempt(&id, None).unwrap();
    let mut file = fixture.file();
    file.write_all(b"first").unwrap();
    let directory = File::open(&fixture.0).unwrap();
    let first_proof = proof(&mut file, &directory, &first, 5);
    let saved = owner.save_ftp_checkpoint(&first, &first_proof).unwrap();
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(b"end").unwrap();
    let unsaved = proof(&mut file, &directory, &saved, 8);
    let connection = rusqlite::Connection::open(fixture.db()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_save BEFORE UPDATE OF proof ON ftp_attempts BEGIN SELECT RAISE(ABORT,'reject save'); END").unwrap();
    assert!(owner.save_ftp_checkpoint(&saved, &unsaved).is_err());
    drop(owner);
    let mut owner = core(fixture.db());
    owner.recover().unwrap();
    let old = owner.load_ftp_attempt(&id).unwrap().unwrap();
    let next = owner.claim_ftp_attempt(&id, Some(&old)).unwrap();
    let request = || FtpResumeRequest {
        binding: next.binding(),
        attempt_generation: next.generation(),
        proof: next.proof().unwrap(),
    };
    let repaired = ftp::repair_uncommitted_tail(
        &mut file,
        &directory,
        request(),
        &FtpTransferControl::default(),
    )
    .unwrap();
    assert_eq!(repaired, Progress::new(5, Some(8)));
    assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"first");
    let unchanged = owner.load_ftp_attempt(&id).unwrap().unwrap();
    assert_eq!(unchanged.generation(), next.generation());
    assert_eq!(unchanged.revision(), saved.revision());
    assert_eq!(
        unchanged.proof().unwrap().encode().unwrap(),
        first_proof.encode().unwrap()
    );
    assert_eq!(
        owner.repository.get(&id).unwrap().unwrap().progress,
        repaired
    );
    connection
        .execute_batch("DROP TRIGGER reject_save")
        .unwrap();
    let (endpoint, origin) = origin(false);
    let complete = ftp::resume(
        endpoint,
        FtpCredentials::Anonymous,
        &mut file,
        &directory,
        request(),
        &FtpTransferControl::default(),
    )
    .unwrap();
    origin.join().unwrap();
    assert_eq!(complete, Progress::new(8, Some(8)));
    let final_proof = proof(&mut file, &directory, &next, 8);
    let committed = owner.save_ftp_checkpoint(&next, &final_proof).unwrap();
    assert_eq!(committed.revision(), 2);
    assert_eq!(
        owner.repository.get(&id).unwrap().unwrap().progress,
        complete
    );
    assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"firstend");
    assert!(owner.save_ftp_checkpoint(&saved, &first_proof).is_err());
}

fn origin(partial: bool) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let start = std::time::Instant::now();
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(start.elapsed() < std::time::Duration::from_secs(10));
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut control = BufReader::new(stream);
        fn reply(control: &mut BufReader<std::net::TcpStream>, line: &str) {
            control.get_mut().write_all(line.as_bytes()).unwrap();
        }
        fn command(control: &mut BufReader<std::net::TcpStream>, expected: &str) {
            let mut text = String::new();
            control.read_line(&mut text).unwrap();
            assert_eq!(text, format!("{expected}\r\n"));
        }
        reply(&mut control, "220 Ready\r\n");
        command(&mut control, "USER anonymous");
        reply(&mut control, "230 Accepted\r\n");
        command(&mut control, "TYPE I");
        reply(&mut control, "200 Binary\r\n");
        command(&mut control, "SIZE pub/a.bin");
        reply(&mut control, "213 8\r\n");
        command(&mut control, "EPSV");
        let data_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        reply(
            &mut control,
            &format!(
                "229 Passive (|||{}|)\r\n",
                data_listener.local_addr().unwrap().port()
            ),
        );
        command(&mut control, "RETR pub/a.bin");
        reply(&mut control, "150 Data\r\n");
        let (mut data, _) = data_listener.accept().unwrap();
        data.write_all(if partial { b"first" } else { b"firstend" })
            .unwrap();
        drop(data);
        if !partial {
            reply(&mut control, "226 Complete\r\n");
            command(&mut control, "SIZE pub/a.bin");
            reply(&mut control, "213 8\r\n");
            assert_eq!(control.read(&mut [0]).unwrap(), 0);
        }
    });
    (endpoint, worker)
}

#[test]
fn restarts_a_real_ftp_download_through_the_core_persistence_consumer() {
    const CHILD: &str = "NEXUM_FTP_SQLITE_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let root = PathBuf::from(path);
        let mut owner = core(root.join("nexum.sqlite"));
        owner.recover().unwrap();
        let attempt = owner.claim_ftp_attempt(&TaskId::from("ftp"), None).unwrap();
        let mut file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.join("partial"))
            .unwrap();
        let endpoint = std::env::var("NEXUM_FTP_SQLITE_ENDPOINT")
            .unwrap()
            .parse()
            .unwrap();
        let control = FtpTransferControl::default();
        assert_eq!(
            ftp::receive(
                endpoint,
                "pub/a.bin",
                FtpCredentials::Anonymous,
                &mut file,
                &control
            ),
            Err(FtpReceiveError::InvalidReply(ftp::FtpStage::Completion))
        );
        assert_eq!(control.progress().unwrap().downloaded_bytes, 5);
        let proof = proof(&mut file, &File::open(&root).unwrap(), &attempt, 5);
        owner.save_ftp_checkpoint(&attempt, &proof).unwrap();
        return;
    }
    let fixture = Fixture::new();
    let mut original = core(fixture.db());
    create(&mut original);
    drop(original);
    let (endpoint, first_origin) = origin(true);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "restarts_a_real_ftp_download_through_the_core_persistence_consumer",
            "--nocapture",
        ])
        .env(CHILD, &fixture.0)
        .env("NEXUM_FTP_SQLITE_ENDPOINT", endpoint.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    first_origin.join().unwrap();
    let mut owner = core(fixture.db());
    owner.recover().unwrap();
    let id = TaskId::from("ftp");
    let old = owner.load_ftp_attempt(&id).unwrap().unwrap();
    assert_eq!(old.proof().unwrap().progress(), Progress::new(5, Some(8)));
    let next = owner.claim_ftp_attempt(&id, Some(&old)).unwrap();
    let mut file = File::options()
        .read(true)
        .write(true)
        .open(fixture.0.join("partial"))
        .unwrap();
    let directory = File::open(&fixture.0).unwrap();
    let (endpoint, second_origin) = origin(false);
    let result = ftp::resume(
        endpoint,
        FtpCredentials::Anonymous,
        &mut file,
        &directory,
        FtpResumeRequest {
            binding: next.binding(),
            attempt_generation: next.generation(),
            proof: next.proof().unwrap(),
        },
        &FtpTransferControl::default(),
    )
    .unwrap();
    second_origin.join().unwrap();
    assert_eq!(result, Progress::new(8, Some(8)));
    let final_proof = proof(&mut file, &directory, &next, 8);
    owner.drain_task_events();
    assert!(owner.save_ftp_checkpoint(&old, &final_proof).is_err());
    assert!(owner.drain_task_events().is_empty());
    let saved = owner.save_ftp_checkpoint(&next, &final_proof).unwrap();
    assert_eq!(saved.revision(), 2);
    assert_eq!(owner.repository.get(&id).unwrap().unwrap().progress, result);
    assert_eq!(owner.tasks.get(&id).unwrap().progress, result);
    assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"firstend");
    assert_eq!(
        owner.tasks.get(&id).unwrap().state,
        nexum_task::TaskState::Created
    );
    drop(owner);
    let mut restarted = core(fixture.db());
    restarted.recover().unwrap();
    assert_eq!(
        restarted
            .load_ftp_attempt(&id)
            .unwrap()
            .unwrap()
            .proof()
            .unwrap()
            .encode()
            .unwrap(),
        final_proof.encode().unwrap()
    );
}

#[test]
fn completion_proof_survives_a_core_consumer_process_and_verifies_with_the_origin_offline() {
    use ftp::{FtpCompletionProof, FtpCompletionRequest};
    const CHILD: &str = "NEXUM_FTP_COMPLETION_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let root = PathBuf::from(path);
        let mut owner = core(root.join("nexum.sqlite"));
        owner.recover().unwrap();
        let id = TaskId::from("ftp");
        let old = owner.load_ftp_attempt(&id).unwrap();
        let attempt = owner.claim_ftp_attempt(&id, old.as_ref()).unwrap();
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(root.join("partial"))
            .unwrap();
        let directory = File::open(&root).unwrap();
        // Fixture-owned journal, already open before production APIs run.
        // This is not a product journal-publication implementation.
        let mut journal = File::options()
            .write(true)
            .create_new(true)
            .open(root.join("complete-proof.json"))
            .unwrap();
        let endpoint = std::env::var("NEXUM_FTP_COMPLETION_ENDPOINT")
            .unwrap()
            .parse()
            .unwrap();
        let control = FtpTransferControl::default();
        let complete = if let Some(prefix) = attempt.proof() {
            ftp::resume_complete(
                endpoint,
                FtpCredentials::Anonymous,
                &mut file,
                &directory,
                FtpResumeRequest {
                    binding: attempt.binding(),
                    attempt_generation: attempt.generation(),
                    proof: prefix,
                },
                &control,
            )
        } else {
            ftp::receive_complete(
                endpoint,
                FtpCredentials::Anonymous,
                &mut file,
                &directory,
                FtpCompletionRequest {
                    binding: attempt.binding(),
                    attempt_generation: attempt.generation(),
                },
                &control,
            )
        }
        .unwrap();
        assert!(control.is_finished());
        assert_eq!(complete.progress(), Progress::new(8, Some(8)));
        assert_eq!(complete.attempt_generation(), attempt.generation());
        journal.write_all(&complete.encode().unwrap()).unwrap();
        journal.sync_all().unwrap();
        return;
    }
    for resumed in [false, true] {
        let fixture = Fixture::new();
        let mut original = core(fixture.db());
        create(&mut original);
        let mut file = fixture.file();
        if resumed {
            let attempt = original
                .claim_ftp_attempt(&TaskId::from("ftp"), None)
                .unwrap();
            file.write_all(b"first").unwrap();
            let prefix = proof(&mut file, &File::open(&fixture.0).unwrap(), &attempt, 5);
            original.save_ftp_checkpoint(&attempt, &prefix).unwrap();
        }
        drop(file);
        drop(original);
        let (endpoint, origin) = origin(false);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","completion_proof_survives_a_core_consumer_process_and_verifies_with_the_origin_offline","--nocapture"])
            .env(CHILD,&fixture.0).env("NEXUM_FTP_COMPLETION_ENDPOINT",endpoint.to_string()).output().unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        origin.join().unwrap();
        assert!(std::net::TcpStream::connect(endpoint).is_err());
        let mut restarted = core(fixture.db());
        restarted.recover().unwrap();
        let id = TaskId::from("ftp");
        let attempt = restarted.load_ftp_attempt(&id).unwrap().unwrap();
        let encoded = fs::read(fixture.0.join("complete-proof.json")).unwrap();
        assert!(encoded.len() <= 65536);
        let complete = FtpCompletionProof::decode(&encoded).unwrap();
        let mut file = File::open(fixture.0.join("partial")).unwrap();
        let directory = File::open(&fixture.0).unwrap();
        let verified = ftp::verify_completion(
            &mut file,
            &directory,
            FtpCompletionRequest {
                binding: attempt.binding(),
                attempt_generation: attempt.generation(),
            },
            &complete,
            &FtpTransferControl::default(),
        )
        .unwrap();
        assert_eq!(verified, Progress::new(8, Some(8)));
        assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"firstend");
        assert_eq!(complete.encode().unwrap(), encoded);
        let expected = Progress::new(
            if resumed { 5 } else { 0 },
            if resumed { Some(8) } else { None },
        );
        assert_eq!(
            restarted.repository.get(&id).unwrap().unwrap().progress,
            expected
        );
        assert_eq!(
            restarted.tasks.get(&id).unwrap().state,
            nexum_task::TaskState::Created
        );
        let next = restarted.claim_ftp_attempt(&id, Some(&attempt)).unwrap();
        assert_eq!(
            ftp::verify_completion(
                &mut file,
                &directory,
                FtpCompletionRequest {
                    binding: next.binding(),
                    attempt_generation: next.generation()
                },
                &complete,
                &FtpTransferControl::default()
            ),
            Err(FtpReceiveError::ResumeBindingMismatch)
        );
    }
}

#[test]
fn sqlite_completion_survives_real_ftp_process_restart_and_offline_reclaim() {
    use ftp::FtpCompletionRequest;
    const CHILD: &str = "NEXUM_FTP_DURABLE_COMPLETION_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let root = PathBuf::from(path);
        let mut owner = core(root.join("nexum.sqlite"));
        owner.recover().unwrap();
        let id = TaskId::from("ftp");
        let attempt = owner.claim_ftp_attempt(&id, None).unwrap();
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(root.join("partial"))
            .unwrap();
        let directory = File::open(&root).unwrap();
        let endpoint = std::env::var("NEXUM_FTP_DURABLE_ENDPOINT")
            .unwrap()
            .parse()
            .unwrap();
        let proof = ftp::receive_complete(
            endpoint,
            FtpCredentials::Anonymous,
            &mut file,
            &directory,
            FtpCompletionRequest {
                binding: attempt.binding(),
                attempt_generation: attempt.generation(),
            },
            &FtpTransferControl::default(),
        )
        .unwrap();
        let saved = owner.save_ftp_completion(&attempt, &proof).unwrap();
        assert_eq!(saved.completion_generation(), Some(attempt.generation()));
        assert_eq!(
            owner.repository.get(&id).unwrap().unwrap().progress,
            Progress::new(8, Some(8))
        );
        return;
    }
    let fixture = Fixture::new();
    let mut initial = core(fixture.db());
    create(&mut initial);
    drop(fixture.file());
    drop(initial);
    let (endpoint, source) = origin(false);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sqlite_completion_survives_real_ftp_process_restart_and_offline_reclaim",
            "--nocapture",
        ])
        .env(CHILD, &fixture.0)
        .env("NEXUM_FTP_DURABLE_ENDPOINT", endpoint.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    source.join().unwrap();
    assert!(std::net::TcpStream::connect(endpoint).is_err());
    let mut restarted = core(fixture.db());
    restarted.recover().unwrap();
    let id = TaskId::from("ftp");
    let previous = restarted.load_ftp_attempt(&id).unwrap().unwrap();
    assert_eq!(
        previous.completion_generation(),
        Some(previous.generation())
    );
    let claimed = restarted.claim_ftp_attempt(&id, Some(&previous)).unwrap();
    assert_ne!(
        claimed.generation(),
        claimed.completion_generation().unwrap()
    );
    assert_eq!(
        restarted.repository.get(&id).unwrap().unwrap().progress,
        Progress::new(8, Some(8))
    );
    assert_eq!(
        restarted.tasks.get(&id).unwrap().state,
        nexum_task::TaskState::Created
    );
    let directory = File::open(&fixture.0).unwrap();
    let mut file = File::open(fixture.0.join("partial")).unwrap();
    let cancelled = FtpTransferControl::default();
    cancelled.cancel();
    assert!(matches!(
        restarted.verify_ftp_completion(&claimed, &mut file, &directory, &cancelled),
        Err(nexum_core::CoreError::Engine(
            nexum_engine::EngineError::Cancelled
        ))
    ));
    assert_eq!(cancelled.progress(), None);
    assert!(
        restarted
            .verify_ftp_completion(
                &previous,
                &mut file,
                &directory,
                &FtpTransferControl::default()
            )
            .is_err()
    );
    let successful_control = FtpTransferControl::default();
    assert_eq!(
        restarted
            .verify_ftp_completion(&claimed, &mut file, &directory, &successful_control)
            .unwrap(),
        Progress::new(8, Some(8))
    );
    assert_eq!(successful_control.progress(), None);
    fs::rename(fixture.0.join("partial"), fixture.0.join("original")).unwrap();
    fs::write(fixture.0.join("partial"), b"firstend").unwrap();
    let mut replacement = File::open(fixture.0.join("partial")).unwrap();
    assert!(
        restarted
            .verify_ftp_completion(
                &claimed,
                &mut replacement,
                &directory,
                &FtpTransferControl::default()
            )
            .is_err()
    );
    assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"firstend");
}

#[test]
fn sqlite_immediate_lock_allows_read_but_final_fence_times_out_and_bounds_cancel() {
    use ftp::{FtpCompletionRequest, FtpReceiveError};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    struct Guard<'a>(nexum_storage::ftp_resume::FtpAttemptGuard<'a>);
    impl ftp::FtpCompletionGuard for Guard<'_> {
        fn release(self) -> Result<(), FtpReceiveError> {
            self.0
                .release()
                .map_err(|_| FtpReceiveError::ResumeBindingMismatch)
        }
    }
    let fixture = Fixture::new();
    let mut owner = core(fixture.db());
    let record = create(&mut owner);
    let id = TaskId::from("ftp");
    let attempt = owner.claim_ftp_attempt(&id, None).unwrap();
    let mut file = fixture.file();
    let directory = File::open(&fixture.0).unwrap();
    let (endpoint, source) = origin(false);
    let proof = ftp::receive_complete(
        endpoint,
        FtpCredentials::Anonymous,
        &mut file,
        &directory,
        FtpCompletionRequest {
            binding: attempt.binding(),
            attempt_generation: attempt.generation(),
        },
        &FtpTransferControl::default(),
    )
    .unwrap();
    source.join().unwrap();
    let saved = owner.save_ftp_completion(&attempt, &proof).unwrap();
    let locker = rusqlite::Connection::open(fixture.db()).unwrap();
    locker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let snapshot = owner.repository.ftp_attempt(&record).unwrap().unwrap();
    let check = FtpTransferControl::default();
    let began = Instant::now();
    assert!(matches!(
        owner.verify_ftp_completion(&saved, &mut file, &directory, &check),
        Err(nexum_core::CoreError::Storage(
            nexum_storage::StorageError::Busy
        ))
    ));
    assert!(began.elapsed() < Duration::from_secs(2));
    assert_eq!(check.progress(), None);
    let (tx, rx) = mpsc::channel();
    let control = FtpTransferControl::default();
    let worker_control = control.clone();
    let worker = thread::spawn(move || {
        ftp::verify_completion_with_guard(
            &mut file,
            &directory,
            FtpCompletionRequest {
                binding: saved.binding(),
                attempt_generation: saved.completion_generation().unwrap(),
            },
            saved.completion().unwrap(),
            &worker_control,
            || {
                tx.send(()).unwrap();
                owner
                    .repository
                    .guard_ftp_attempt(&snapshot)
                    .map(Guard)
                    .map_err(|_| FtpReceiveError::ResumeBindingMismatch)
            },
        )
    });
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let began = Instant::now();
    control.cancel();
    let elapsed = began.elapsed();
    let result = worker.join().unwrap();
    assert_eq!(result, Err(FtpReceiveError::ResumeBindingMismatch));
    assert_eq!(control.progress(), None);
    assert!(
        elapsed < Duration::from_secs(2),
        "cancel remained blocked for {elapsed:?}"
    );
    locker.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn damaged_or_missing_sqlite_completion_never_authorizes_reclaim_or_changes_files() {
    use ftp::FtpCompletionRequest;
    for mutation in [
        "UPDATE ftp_attempts SET completion_proof=x'00' WHERE task_id='ftp'",
        "UPDATE ftp_attempts SET completion_generation='forged' WHERE task_id='ftp'",
        "UPDATE ftp_attempts SET completion_version=9 WHERE task_id='ftp'",
        "UPDATE ftp_attempts SET completion_version=NULL,completion_proof=NULL,completion_generation=NULL,completion_bytes=NULL WHERE task_id='ftp'",
        "UPDATE tasks SET downloaded_bytes=7 WHERE id='ftp'",
    ] {
        let fixture = Fixture::new();
        let mut owner = core(fixture.db());
        create(&mut owner);
        let id = TaskId::from("ftp");
        let attempt = owner.claim_ftp_attempt(&id, None).unwrap();
        let mut file = fixture.file();
        let directory = File::open(&fixture.0).unwrap();
        let (endpoint, source) = origin(false);
        let proof = ftp::receive_complete(
            endpoint,
            FtpCredentials::Anonymous,
            &mut file,
            &directory,
            FtpCompletionRequest {
                binding: attempt.binding(),
                attempt_generation: attempt.generation(),
            },
            &FtpTransferControl::default(),
        )
        .unwrap();
        source.join().unwrap();
        let saved = owner.save_ftp_completion(&attempt, &proof).unwrap();
        owner.drain_task_events();
        let db = rusqlite::Connection::open(fixture.db()).unwrap();
        db.execute_batch(mutation).unwrap();
        assert!(owner.load_ftp_attempt(&id).is_err(), "{mutation}");
        assert!(
            owner.claim_ftp_attempt(&id, Some(&saved)).is_err(),
            "{mutation}"
        );
        assert!(owner.drain_task_events().is_empty());
        assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"firstend");
        assert_eq!(
            db.query_row(
                "SELECT completion_seen FROM ftp_attempts WHERE task_id='ftp'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}

#[test]
fn sqlite_claim_is_fenced_until_guarded_verification_decision() {
    use ftp::{FtpCompletionRequest, FtpReceiveError};
    use nexum_storage::ftp_resume::FtpAttemptGuard;
    use std::sync::mpsc;
    use std::time::Duration;
    struct ObservedGuard<'a> {
        _held: FtpAttemptGuard<'a>,
        control: FtpTransferControl,
        observed: mpsc::Sender<Option<Progress>>,
    }
    impl ftp::FtpCompletionGuard for ObservedGuard<'_> {
        fn release(self) -> Result<(), ftp::FtpReceiveError> {
            self.observed.send(self.control.progress()).unwrap();
            self._held
                .release()
                .map_err(|_| ftp::FtpReceiveError::ResumeBindingMismatch)
        }
    }
    let fixture = Fixture::new();
    let mut owner = core(fixture.db());
    let origin_record = create(&mut owner);
    let id = TaskId::from("ftp");
    let attempt = owner.claim_ftp_attempt(&id, None).unwrap();
    let mut file = fixture.file();
    let directory = File::open(&fixture.0).unwrap();
    let (endpoint, source) = origin(false);
    let proof = ftp::receive_complete(
        endpoint,
        FtpCredentials::Anonymous,
        &mut file,
        &directory,
        FtpCompletionRequest {
            binding: attempt.binding(),
            attempt_generation: attempt.generation(),
        },
        &FtpTransferControl::default(),
    )
    .unwrap();
    source.join().unwrap();
    let saved = owner.save_ftp_completion(&attempt, &proof).unwrap();
    let snapshot = owner
        .repository
        .ftp_attempt(&origin_record)
        .unwrap()
        .unwrap();
    let mut contender = SqliteRepository::open(fixture.db()).unwrap();
    let (start_tx, start_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let record = origin_record.clone();
    let previous = snapshot.clone();
    let worker = thread::spawn(move || {
        start_rx.recv().unwrap();
        entered_tx.send(()).unwrap();
        result_tx
            .send(contender.claim_ftp_attempt(&record, Some(&previous)))
            .unwrap();
    });
    let control = FtpTransferControl::default();
    let (observed_tx, observed_rx) = mpsc::channel();
    let result = ftp::verify_completion_with_guard(
        &mut file,
        &directory,
        FtpCompletionRequest {
            binding: saved.binding(),
            attempt_generation: saved.completion_generation().unwrap(),
        },
        saved.completion().unwrap(),
        &control,
        || {
            let held = owner
                .repository
                .guard_ftp_attempt(&snapshot)
                .map_err(|_| FtpReceiveError::ResumeBindingMismatch)?;
            start_tx.send(()).unwrap();
            entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(
                matches!(
                    result_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
                    Err(nexum_storage::StorageError::Busy)
                ),
                "claim must reach SQLite and fail busy while guard is held"
            );
            Ok(ObservedGuard {
                _held: held,
                control: control.clone(),
                observed: observed_tx,
            })
        },
    );
    assert_eq!(result.unwrap(), Progress::new(8, Some(8)));
    assert_eq!(
        observed_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        None
    );
    worker.join().unwrap();
    let reclaimed = owner
        .repository
        .claim_ftp_attempt(&origin_record, Some(&snapshot))
        .unwrap();
    assert_ne!(reclaimed.generation, snapshot.generation);
    assert_eq!(reclaimed.completion, snapshot.completion);
}

#[test]
fn corrupted_or_unknown_engine_proof_is_retained_and_rejected_by_the_consumer() {
    for unknown in [false, true] {
        let fixture = Fixture::new();
        let mut owner = core(fixture.db());
        create(&mut owner);
        let id = TaskId::from("ftp");
        let attempt = owner.claim_ftp_attempt(&id, None).unwrap();
        let mut file = fixture.file();
        file.write_all(b"first").unwrap();
        let proof = proof(&mut file, &File::open(&fixture.0).unwrap(), &attempt, 5);
        let saved = owner.save_ftp_checkpoint(&attempt, &proof).unwrap();
        let bad = if unknown {
            String::from_utf8(proof.encode().unwrap())
                .unwrap()
                .replace("ftp-held-prefix/v1", "ftp-held-prefix/v9")
                .into_bytes()
        } else {
            b"corrupted-proof".to_vec()
        };
        let connection = rusqlite::Connection::open(fixture.db()).unwrap();
        connection
            .execute(
                "UPDATE ftp_attempts SET proof=?1 WHERE task_id='ftp'",
                [&bad],
            )
            .unwrap();
        owner.drain_task_events();
        assert!(owner.load_ftp_attempt(&id).is_err());
        assert!(owner.claim_ftp_attempt(&id, Some(&saved)).is_err());
        assert!(owner.drain_task_events().is_empty());
        assert_eq!(
            connection
                .query_row(
                    "SELECT proof FROM ftp_attempts WHERE task_id='ftp'",
                    [],
                    |row| row.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
            bad
        );
        assert_eq!(
            owner.repository.get(&id).unwrap().unwrap().progress,
            Progress::new(5, Some(8))
        );
        assert_eq!(fs::read(fixture.0.join("partial")).unwrap(), b"first");
    }
}

#[test]
fn a_rejected_reclaim_retains_the_previous_generation_and_proof() {
    let fixture = Fixture::new();
    let mut owner = core(fixture.db());
    create(&mut owner);
    let id = TaskId::from("ftp");
    let first = owner.claim_ftp_attempt(&id, None).unwrap();
    let mut file = fixture.file();
    file.write_all(b"first").unwrap();
    let proof = proof(&mut file, &File::open(&fixture.0).unwrap(), &first, 5);
    let saved = owner.save_ftp_checkpoint(&first, &proof).unwrap();
    owner.drain_task_events();
    rusqlite::Connection::open(fixture.db()).unwrap().execute_batch(
        "CREATE TRIGGER reject_claim BEFORE UPDATE OF generation ON ftp_attempts BEGIN SELECT RAISE(ABORT,'reject claim'); END"
    ).unwrap();
    assert!(owner.claim_ftp_attempt(&id, Some(&saved)).is_err());
    let retained = owner.load_ftp_attempt(&id).unwrap().unwrap();
    assert_eq!(retained.generation(), saved.generation());
    assert_eq!(retained.revision(), saved.revision());
    assert_eq!(
        retained.proof().unwrap().encode().unwrap(),
        proof.encode().unwrap()
    );
    assert!(owner.drain_task_events().is_empty());
    assert_eq!(
        owner.repository.get(&id).unwrap().unwrap().progress,
        Progress::new(5, Some(8))
    );
}
