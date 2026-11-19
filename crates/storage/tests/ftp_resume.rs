use nexum_domain::{Destination, DownloadSource, TaskId};
use nexum_storage::{SqliteRepository, TaskRepository, ftp_resume::FtpCheckpoint, origin::*};
use nexum_task::DownloadTask;

fn create(repo: &mut SqliteRepository) -> OriginRecord {
    let record = OriginRecord {
        version: 1,
        task_id: "ftp".into(),
        destination: "/tmp/ftp".into(),
        canonical_url: "ftp://example.com/pub/a.bin".into(),
        origin: "ftp://example.com:21".into(),
        command_path: "pub/a.bin".into(),
        security: "plain".into(),
        path_mode: "login_relative".into(),
        authenticated: false,
        operation_id: random_id().unwrap(),
        credential: None,
    };
    repo.reserve_origin(&record).unwrap();
    repo.activate_origin(
        &record,
        DownloadTask::new(
            "ftp",
            DownloadSource::new(&record.canonical_url),
            Destination::new(&record.destination),
        )
        .into(),
    )
    .unwrap();
    record
}
fn checkpoint() -> FtpCheckpoint {
    FtpCheckpoint {
        proof: b"opaque-engine-proof".to_vec(),
        trusted_bytes: 5,
        total_bytes: Some(8),
    }
}

#[test]
fn checkpoint_and_progress_are_saved_together_and_stale_attempts_are_fenced() {
    let mut repo = SqliteRepository::open_in_memory().unwrap();
    let origin = create(&mut repo);
    let first = repo.claim_ftp_attempt(&origin, None).unwrap();
    assert!(repo.claim_ftp_attempt(&origin, None).is_err());
    let saved = repo.save_ftp_checkpoint(&first, &checkpoint()).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.checkpoint, Some(checkpoint()));
    assert_eq!(
        repo.get(&TaskId::from("ftp"))
            .unwrap()
            .unwrap()
            .progress
            .downloaded_bytes,
        5
    );
    assert!(repo.save_ftp_checkpoint(&first, &checkpoint()).is_err());
    assert!(repo.claim_ftp_attempt(&origin, Some(&first)).is_err());
    let next = repo.claim_ftp_attempt(&origin, Some(&saved)).unwrap();
    assert_ne!(next.generation, saved.generation);
    assert_eq!(next.checkpoint, saved.checkpoint);
    assert!(repo.save_ftp_checkpoint(&saved, &checkpoint()).is_err());
    assert_eq!(repo.ftp_attempt(&origin).unwrap(), Some(next));
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("nexum-ftp-store-{}", random_id().unwrap()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn db(&self) -> std::path::PathBuf {
        self.0.join("nexum.sqlite")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn migrates_schema_three_preserving_origins_tasks_and_instance_identity() {
    let fixture = Fixture::new();
    let mut repo = SqliteRepository::open(fixture.db()).unwrap();
    let origin = create(&mut repo);
    let instance = repo.instance_id().unwrap();
    let task = repo.get(&TaskId::from("ftp")).unwrap();
    drop(repo);
    let connection = rusqlite::Connection::open(fixture.db()).unwrap();
    connection
        .execute_batch("DROP TABLE ftp_attempts; UPDATE schema_version SET version=3;")
        .unwrap();
    drop(connection);
    let mut repo = SqliteRepository::open(fixture.db()).unwrap();
    assert_eq!(repo.instance_id().unwrap(), instance);
    assert_eq!(repo.get(&TaskId::from("ftp")).unwrap(), task);
    assert_eq!(
        repo.active_origin(&TaskId::from("ftp")).unwrap(),
        Some(origin.clone())
    );
    assert!(repo.ftp_attempt(&origin).unwrap().is_none());
    let attempt = repo.claim_ftp_attempt(&origin, None).unwrap();
    repo.save_ftp_checkpoint(&attempt, &checkpoint()).unwrap();
    drop(repo);
    let repo = SqliteRepository::open(fixture.db()).unwrap();
    assert_eq!(
        repo.ftp_attempt(&origin).unwrap().unwrap().checkpoint,
        Some(checkpoint())
    );
    let connection = rusqlite::Connection::open(fixture.db()).unwrap();
    let version: i64 = connection
        .query_row("SELECT version FROM schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
    drop(repo);
    connection
        .execute("UPDATE schema_version SET version=5", [])
        .unwrap();
    assert!(SqliteRepository::open(fixture.db()).is_err());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM ftp_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn removal_rollback_and_recreated_task_cannot_accept_the_old_attempt() {
    let fixture = Fixture::new();
    let mut repo = SqliteRepository::open(fixture.db()).unwrap();
    let old = create(&mut repo);
    let attempt = repo.claim_ftp_attempt(&old, None).unwrap();
    let attempt = repo.save_ftp_checkpoint(&attempt, &checkpoint()).unwrap();
    let connection = rusqlite::Connection::open(fixture.db()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_remove BEFORE DELETE ON tasks BEGIN SELECT RAISE(ABORT,'reject removal'); END").unwrap();
    assert!(repo.remove(&TaskId::from("ftp")).is_err());
    assert_eq!(repo.ftp_attempt(&old).unwrap(), Some(attempt.clone()));
    assert!(repo.get(&TaskId::from("ftp")).unwrap().is_some());
    connection
        .execute_batch("DROP TRIGGER reject_remove")
        .unwrap();
    repo.remove(&TaskId::from("ftp")).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM ftp_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let new = create(&mut repo);
    assert_ne!(old.operation_id, new.operation_id);
    assert!(repo.ftp_attempt(&new).unwrap().is_none());
    let fresh = repo.claim_ftp_attempt(&new, None).unwrap();
    assert!(repo.save_ftp_checkpoint(&attempt, &checkpoint()).is_err());
    assert!(repo.claim_ftp_attempt(&old, Some(&attempt)).is_err());
    assert_eq!(repo.ftp_attempt(&new).unwrap(), Some(fresh));
    assert_eq!(
        repo.get(&TaskId::from("ftp"))
            .unwrap()
            .unwrap()
            .progress
            .downloaded_bytes,
        0
    );
}

#[test]
fn bounded_checkpoint_metadata_and_progress_never_regress() {
    let mut repo = SqliteRepository::open_in_memory().unwrap();
    let origin = create(&mut repo);
    let first = repo.claim_ftp_attempt(&origin, None).unwrap();
    for bad in [
        FtpCheckpoint {
            proof: vec![0; 65537],
            ..checkpoint()
        },
        FtpCheckpoint {
            proof: Vec::new(),
            ..checkpoint()
        },
        FtpCheckpoint {
            trusted_bytes: 9,
            ..checkpoint()
        },
        FtpCheckpoint {
            trusted_bytes: u64::MAX,
            total_bytes: None,
            ..checkpoint()
        },
    ] {
        assert!(repo.save_ftp_checkpoint(&first, &bad).is_err());
        assert_eq!(repo.ftp_attempt(&origin).unwrap(), Some(first.clone()));
    }
    let saved = repo.save_ftp_checkpoint(&first, &checkpoint()).unwrap();
    for field in 0..5 {
        let mut forged = saved.clone();
        match field {
            0 => forged.instance_id.push('x'),
            1 => forged.origin.operation_id.push('x'),
            2 => forged.origin.origin.push('x'),
            3 => forged.origin.destination.push('x'),
            4 => {
                forged.origin.credential = Some(CredentialRef {
                    instance_id: forged.instance_id.clone(),
                    origin: forged.origin.origin.clone(),
                    credential_id: "unknown".into(),
                    revision: 2,
                })
            }
            _ => unreachable!(),
        }
        assert!(repo.save_ftp_checkpoint(&forged, &checkpoint()).is_err());
    }
    let smaller = FtpCheckpoint {
        trusted_bytes: 3,
        ..checkpoint()
    };
    assert!(repo.save_ftp_checkpoint(&saved, &smaller).is_err());
    assert_eq!(repo.ftp_attempt(&origin).unwrap(), Some(saved));
}
