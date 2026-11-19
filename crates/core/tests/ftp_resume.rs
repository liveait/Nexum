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
