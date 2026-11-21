use nexum_domain::{Destination, DownloadSource, TaskId};
use nexum_storage::{
    SqliteRepository, TaskRepository,
    ftp_resume::{FtpCheckpoint, FtpCompletion},
    origin::*,
};
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
fn completion_and_whole_file_progress_survive_reclaims_without_prefix_replacement() {
    let mut repo = SqliteRepository::open_in_memory().unwrap();
    let origin = create(&mut repo);
    let attempt = repo.claim_ftp_attempt(&origin, None).unwrap();
    let completion = FtpCompletion {
        proof: b"opaque-completion".to_vec(),
        producing_generation: attempt.generation.clone(),
        trusted_bytes: 8,
    };
    let saved = repo.save_ftp_completion(&attempt, &completion).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.completion, Some(completion.clone()));
    assert_eq!(
        repo.get(&TaskId::from("ftp"))
            .unwrap()
            .unwrap()
            .progress
            .downloaded_bytes,
        8
    );
    assert!(repo.save_ftp_checkpoint(&saved, &checkpoint()).is_err());
    assert!(repo.save_ftp_completion(&saved, &completion).is_err());
    let reclaimed = repo.claim_ftp_attempt(&origin, Some(&saved)).unwrap();
    assert_ne!(reclaimed.generation, completion.producing_generation);
    assert_eq!(reclaimed.completion, Some(completion));
    assert!(repo.claim_ftp_attempt(&origin, Some(&saved)).is_err());
}

#[test]
fn schema_four_migration_preserves_existing_checkpoint_and_progress() {
    let fixture = Fixture::new();
    let mut repo = SqliteRepository::open(fixture.db()).unwrap();
    let origin = create(&mut repo);
    let attempt = repo.claim_ftp_attempt(&origin, None).unwrap();
    let expected = repo.save_ftp_checkpoint(&attempt, &checkpoint()).unwrap();
    drop(repo);
    let db = rusqlite::Connection::open(fixture.db()).unwrap();
    db.execute_batch("ALTER TABLE ftp_attempts DROP COLUMN completion_seen; ALTER TABLE ftp_attempts DROP COLUMN completion_version; ALTER TABLE ftp_attempts DROP COLUMN completion_proof; ALTER TABLE ftp_attempts DROP COLUMN completion_generation; ALTER TABLE ftp_attempts DROP COLUMN completion_bytes; UPDATE schema_version SET version=4;").unwrap();
    drop(db);
    let repo = SqliteRepository::open(fixture.db()).unwrap();
    assert_eq!(repo.ftp_attempt(&origin).unwrap(), Some(expected));
    assert_eq!(
        repo.get(&TaskId::from("ftp"))
            .unwrap()
            .unwrap()
            .progress
            .downloaded_bytes,
        5
    );
}

#[test]
fn completion_transaction_failure_and_task_recreation_preserve_isolation() {
    for trigger in [
        "CREATE TRIGGER reject_save BEFORE UPDATE OF completion_proof ON ftp_attempts BEGIN SELECT RAISE(ABORT,'reject completion'); END",
        "CREATE TRIGGER reject_save BEFORE UPDATE OF downloaded_bytes ON tasks BEGIN SELECT RAISE(ABORT,'reject progress'); END",
    ] {
        let fixture = Fixture::new();
        let mut repo = SqliteRepository::open(fixture.db()).unwrap();
        let old = create(&mut repo);
        let attempt = repo.claim_ftp_attempt(&old, None).unwrap();
        let completion = FtpCompletion {
            proof: b"complete".to_vec(),
            producing_generation: attempt.generation.clone(),
            trusted_bytes: 8,
        };
        let db = rusqlite::Connection::open(fixture.db()).unwrap();
        db.execute_batch(trigger).unwrap();
        assert!(repo.save_ftp_completion(&attempt, &completion).is_err());
        assert_eq!(repo.ftp_attempt(&old).unwrap(), Some(attempt.clone()));
        assert_eq!(
            repo.get(&TaskId::from("ftp"))
                .unwrap()
                .unwrap()
                .progress
                .downloaded_bytes,
            0
        );
        db.execute_batch("DROP TRIGGER reject_save").unwrap();
        let saved = repo.save_ftp_completion(&attempt, &completion).unwrap();
        repo.remove(&TaskId::from("ftp")).unwrap();
        let replacement = create(&mut repo);
        assert_ne!(replacement.operation_id, old.operation_id);
        assert!(repo.claim_ftp_attempt(&old, Some(&saved)).is_err());
        assert!(repo.ftp_attempt(&replacement).unwrap().is_none());
    }
}

fn migration_snapshot(db: &rusqlite::Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "schema_version",
        "origin_instance",
        "task_origins",
        "tasks",
        "ftp_attempts",
    ]
    .iter()
    .map(|table| {
        let mut query = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let count = query.column_count();
        query
            .query_map([], |row| {
                (0..count)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}

#[test]
fn completion_migration_failure_rolls_back_all_columns_and_preserves_retry() {
    for mid_alter in [false, true] {
        let fixture = Fixture::new();
        let mut repo = SqliteRepository::open(fixture.db()).unwrap();
        let origin = create(&mut repo);
        let attempt = repo.claim_ftp_attempt(&origin, None).unwrap();
        let expected = repo.save_ftp_checkpoint(&attempt, &checkpoint()).unwrap();
        drop(repo);
        let db = rusqlite::Connection::open(fixture.db()).unwrap();
        db.execute_batch("ALTER TABLE ftp_attempts DROP COLUMN completion_seen; ALTER TABLE ftp_attempts DROP COLUMN completion_version; ALTER TABLE ftp_attempts DROP COLUMN completion_proof; ALTER TABLE ftp_attempts DROP COLUMN completion_generation; ALTER TABLE ftp_attempts DROP COLUMN completion_bytes; UPDATE schema_version SET version=4;").unwrap();
        if mid_alter {
            // Deliberately malformed schema: the first ALTER can succeed but
            // the second must fail. It must leave no additional partial DDL.
            db.execute_batch("ALTER TABLE ftp_attempts ADD COLUMN completion_version INTEGER")
                .unwrap();
        } else {
            db.execute_batch("CREATE TRIGGER reject_migration BEFORE UPDATE ON schema_version WHEN NEW.version=5 BEGIN SELECT RAISE(ABORT,'reject migration'); END").unwrap();
        }
        let before = migration_snapshot(&db);
        let columns: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('ftp_attempts') ORDER BY cid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(SqliteRepository::open(fixture.db()).is_err());
        assert_eq!(migration_snapshot(&db), before);
        let after: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('ftp_attempts') ORDER BY cid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(after, columns);
        if mid_alter {
            db.execute_batch("ALTER TABLE ftp_attempts DROP COLUMN completion_version")
                .unwrap();
        } else {
            db.execute_batch("DROP TRIGGER reject_migration").unwrap();
        }
        drop(db);
        let repo = SqliteRepository::open(fixture.db()).unwrap();
        assert_eq!(repo.ftp_attempt(&origin).unwrap(), Some(expected));
        assert_eq!(
            repo.get(&TaskId::from("ftp"))
                .unwrap()
                .unwrap()
                .progress
                .downloaded_bytes,
            5
        );
    }
}

#[test]
fn concurrent_completion_reclaims_allow_only_one_cas_winner() {
    use std::sync::{Arc, Barrier};
    use std::thread;
    let fixture = Fixture::new();
    let mut repo = SqliteRepository::open(fixture.db()).unwrap();
    let origin = create(&mut repo);
    let attempt = repo.claim_ftp_attempt(&origin, None).unwrap();
    let completion = FtpCompletion {
        proof: b"complete".to_vec(),
        producing_generation: attempt.generation.clone(),
        trusted_bytes: 8,
    };
    let saved = repo.save_ftp_completion(&attempt, &completion).unwrap();
    let gate = Arc::new(Barrier::new(2));
    let threads = (0..2)
        .map(|_| {
            let path = fixture.db();
            let origin = origin.clone();
            let saved = saved.clone();
            let gate = gate.clone();
            thread::spawn(move || {
                let mut repo = SqliteRepository::open(path).unwrap();
                gate.wait();
                repo.claim_ftp_attempt(&origin, Some(&saved))
            })
        })
        .collect::<Vec<_>>();
    let outcomes = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|r| r.is_err()).count(), 1);
    assert_eq!(
        repo.ftp_attempt(&origin).unwrap().unwrap().completion,
        Some(completion)
    );
}

#[test]
fn observed_completion_cannot_silently_disappear_or_disagree_with_progress() {
    for statement in [
        "UPDATE ftp_attempts SET completion_version=NULL,completion_proof=NULL,completion_generation=NULL,completion_bytes=NULL WHERE task_id='ftp'",
        "UPDATE ftp_attempts SET completion_version=9 WHERE task_id='ftp'",
        "UPDATE tasks SET downloaded_bytes=7 WHERE id='ftp'",
    ] {
        let fixture = Fixture::new();
        let mut repo = SqliteRepository::open(fixture.db()).unwrap();
        let origin = create(&mut repo);
        let attempt = repo.claim_ftp_attempt(&origin, None).unwrap();
        let complete = FtpCompletion {
            proof: b"complete".to_vec(),
            producing_generation: attempt.generation.clone(),
            trusted_bytes: 8,
        };
        let saved = repo.save_ftp_completion(&attempt, &complete).unwrap();
        let db = rusqlite::Connection::open(fixture.db()).unwrap();
        db.execute_batch(statement).unwrap();
        assert!(repo.ftp_attempt(&origin).is_err());
        assert!(repo.claim_ftp_attempt(&origin, Some(&saved)).is_err());
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
    assert_eq!(version, 5);
    drop(repo);
    connection
        .execute("UPDATE schema_version SET version=6", [])
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
