use std::{fs, path::PathBuf};

use nexum_domain::{Destination, DownloadSource, Progress};
use nexum_storage::{
    SqliteRepository, StorageError, StoredTask, TaskRepository,
    ftp_resume::{FtpAttempt, FtpCheckpoint, FtpCompletion},
    origin::{OriginRecord, Reservation, random_id},
};
use nexum_task::{DownloadTask, TaskState};
use rusqlite::Connection;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("ftp-publication-{}", random_id().unwrap()));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn repo(&self) -> SqliteRepository {
        SqliteRepository::open(self.0.join("db")).unwrap()
    }
    fn db(&self) -> Connection {
        Connection::open(self.0.join("db")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn create(repo: &mut SqliteRepository) -> (StoredTask, FtpAttempt) {
    let origin = OriginRecord {
        version: 1,
        task_id: "task".into(),
        destination: "/downloads/file".into(),
        canonical_url: "ftp://example.com/file".into(),
        origin: "ftp://example.com:21".into(),
        command_path: "file".into(),
        security: "plain".into(),
        path_mode: "login_relative".into(),
        authenticated: false,
        operation_id: random_id().unwrap(),
        credential: None,
    };
    let mut task: StoredTask = DownloadTask::new(
        "task",
        DownloadSource::new(&origin.canonical_url),
        Destination::new(&origin.destination),
    )
    .into();
    task.state = TaskState::Downloading;
    assert_eq!(repo.reserve_origin(&origin).unwrap(), Reservation::New);
    repo.activate_origin(&origin, task.clone()).unwrap();
    let attempt = repo.claim_ftp_attempt(&origin, None).unwrap();
    let attempt = repo
        .save_ftp_checkpoint(
            &attempt,
            &FtpCheckpoint {
                proof: vec![7],
                trusted_bytes: 2,
                total_bytes: Some(8),
            },
        )
        .unwrap();
    let completion = FtpCompletion {
        proof: vec![11],
        producing_generation: attempt.generation.clone(),
        trusted_bytes: 8,
    };
    let attempt = repo.save_ftp_completion(&attempt, &completion).unwrap();
    (repo.get(&task.id).unwrap().unwrap(), attempt)
}

fn prepare(repo: &mut SqliteRepository, task: &StoredTask, attempt: &FtpAttempt) -> FtpAttempt {
    repo.prepare_ftp_publication(task, attempt, &random_id().unwrap(), b"private plan")
        .unwrap()
}

#[test]
fn claim_preserves_progress_evidence_and_old_authority_until_explicit_rearm() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let (mut task, attempt) = create(&mut repo);
    let prepared = prepare(&mut repo, &task, &attempt);
    assert_eq!(prepared.revision, attempt.revision + 1);
    assert!(
        repo.claim_ftp_attempt(&attempt.origin, Some(&attempt))
            .is_err()
    );
    assert!(repo.guard_ftp_attempt(&attempt).is_err());
    assert!(
        repo.prepare_ftp_publication(&task, &prepared, &random_id().unwrap(), b"second")
            .is_err()
    );
    assert!(
        repo.save_ftp_completion(&prepared, prepared.completion.as_ref().unwrap())
            .is_err()
    );
    assert!(
        repo.save_ftp_checkpoint(&prepared, prepared.checkpoint.as_ref().unwrap())
            .is_err()
    );
    let old_intent = prepared.publication.as_ref().unwrap();
    assert_eq!(old_intent.plan, b"private plan");
    assert!(!format!("{old_intent:?}").contains("112, 114"));
    task.state = TaskState::Queued;
    task.last_error = Some("previous failure".into());
    repo.update(task.clone()).unwrap();
    let (downloading, new) = repo.claim_ftp_publication(&task, &prepared).unwrap();
    assert_eq!(downloading.state, TaskState::Downloading);
    assert_eq!(downloading.progress, task.progress);
    assert_eq!(downloading.last_error, None);
    assert_eq!(new.publication, prepared.publication);
    assert_eq!(new.completion, prepared.completion);
    assert_eq!(new.checkpoint, prepared.checkpoint);
    assert_ne!(new.generation, prepared.generation);
    for old in [&prepared, &new] {
        assert!(repo.guard_ftp_publication(&downloading, old).is_err());
        assert!(repo.complete_ftp_publication(&downloading, old).is_err());
    }
    assert!(
        repo.rearm_ftp_publication(&downloading, &new, &old_intent.authorization, b"copy")
            .is_err()
    );
    let fresh = repo
        .rearm_ftp_publication(&downloading, &new, &random_id().unwrap(), b"rebound")
        .unwrap();
    assert_eq!(fresh.revision, new.revision + 1);
    assert_eq!(
        fresh.publication.as_ref().unwrap().owner_generation,
        fresh.generation
    );
    assert_eq!(fresh.completion, attempt.completion);
    assert!(repo.guard_ftp_attempt(&new).is_err());
    assert!(repo.guard_ftp_publication(&downloading, &new).is_err());
    repo.guard_ftp_publication(&downloading, &fresh)
        .unwrap()
        .release()
        .unwrap();
    assert_eq!(f.repo().ftp_attempt(&attempt.origin).unwrap(), Some(fresh));
}

#[test]
fn both_publication_fences_compare_every_stored_task_field() {
    for mutation in [
        "state='paused'",
        "state='failed'",
        "source='other'",
        "destination='/other'",
        "downloaded_bytes=7",
        "total_bytes=12",
        "speed_bytes_per_second=12",
        "eta_seconds=3",
        "last_error='changed'",
    ] {
        let f = Fixture::new();
        let mut repo = f.repo();
        let (task, attempt) = create(&mut repo);
        let prepared = prepare(&mut repo, &task, &attempt);
        f.db()
            .execute_batch(&format!("UPDATE tasks SET {mutation}"))
            .unwrap();
        assert!(
            repo.guard_ftp_publication(&task, &prepared).is_err(),
            "{mutation}"
        );
        assert!(
            repo.complete_ftp_publication(&task, &prepared).is_err(),
            "{mutation}"
        );
        let current = repo.get(&task.id).unwrap().unwrap();
        if current.state != TaskState::Downloading {
            assert!(repo.guard_ftp_publication(&current, &prepared).is_err());
            assert!(repo.complete_ftp_publication(&current, &prepared).is_err());
        }
        assert_eq!(
            f.db()
                .query_row("SELECT phase FROM ftp_publications", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "prepared"
        );
    }
}

#[test]
fn held_guard_blocks_independent_writer_and_reports_release() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let mut other = f.repo();
    let (task, attempt) = create(&mut repo);
    let prepared = prepare(&mut repo, &task, &attempt);
    let guard = repo.guard_ftp_publication(&task, &prepared).unwrap();
    assert_eq!(
        other.claim_ftp_attempt(&attempt.origin, Some(&prepared)),
        Err(StorageError::Busy)
    );
    guard.release().unwrap();
    let next = other
        .claim_ftp_attempt(&attempt.origin, Some(&prepared))
        .unwrap();
    assert_eq!(next.publication, prepared.publication);
    assert!(repo.guard_ftp_publication(&task, &next).is_err());
}

#[test]
fn final_transaction_rollback_retry_and_completed_orphan_diagnostics() {
    for trigger in [
        "CREATE TRIGGER fail BEFORE UPDATE OF state ON tasks BEGIN SELECT RAISE(ABORT,'stop'); END",
        "CREATE TRIGGER fail BEFORE UPDATE OF phase ON ftp_publications BEGIN SELECT RAISE(ABORT,'stop'); END",
    ] {
        let f = Fixture::new();
        let mut repo = f.repo();
        let (mut task, attempt) = create(&mut repo);
        task.progress.speed_bytes_per_second = 7;
        task.progress.eta_seconds = Some(9);
        task.last_error = Some("earlier".into());
        repo.update(task.clone()).unwrap();
        let prepared = prepare(&mut repo, &task, &attempt);
        let db = f.db();
        db.execute_batch(trigger).unwrap();
        assert!(repo.complete_ftp_publication(&task, &prepared).is_err());
        assert_eq!(repo.get(&task.id).unwrap(), Some(task.clone()));
        assert_eq!(
            repo.ftp_attempt(&attempt.origin).unwrap(),
            Some(prepared.clone())
        );
        assert!(repo.completed_ftp_publications().unwrap().is_empty());
        db.execute_batch("DROP TRIGGER fail").unwrap();
        let done = repo.complete_ftp_publication(&task, &prepared).unwrap();
        assert_eq!(
            done.progress,
            Progress {
                downloaded_bytes: 8,
                total_bytes: Some(8),
                speed_bytes_per_second: 0,
                eta_seconds: None
            }
        );
        assert_eq!(done.state, TaskState::Completed);
        assert_eq!(done.last_error, None);
        assert_eq!(f.repo().get(&task.id).unwrap(), Some(done));
        assert!(repo.ftp_attempt(&attempt.origin).is_err());
        assert_eq!(
            repo.completed_ftp_publications().unwrap()[0].operation_id,
            attempt.origin.operation_id
        );
        assert!(repo.orphan_ftp_publications().unwrap().is_empty());
        repo.remove(&task.id).unwrap();
        let (task2, attempt2) = create(&mut repo);
        assert!(repo.completed_ftp_publications().unwrap().is_empty());
        assert_eq!(
            f.repo().orphan_ftp_publications().unwrap()[0].operation_id,
            attempt.origin.operation_id
        );
        assert!(attempt2.publication.is_none());
        assert!(repo.guard_ftp_publication(&task2, &prepared).is_err());
        assert!(repo.complete_ftp_publication(&task2, &prepared).is_err());
        prepare(&mut repo, &task2, &attempt2);
        repo.remove(&task2.id).unwrap();
        assert_eq!(repo.orphan_ftp_publications().unwrap().len(), 2);
    }
}

#[test]
fn observed_intent_loss_and_mismatches_fail_closed() {
    for mutation in [
        "DELETE FROM ftp_publications",
        "UPDATE ftp_attempts SET publication_seen=0",
        "UPDATE ftp_attempts SET publication_authorization=NULL",
        "UPDATE ftp_publications SET owner_generation='other'",
        "UPDATE ftp_publications SET revision=revision+1",
        "UPDATE ftp_publications SET phase='completed'",
        "DELETE FROM ftp_attempts",
        "UPDATE task_origins SET operation_id='wrong-incarnation'",
    ] {
        let f = Fixture::new();
        let mut repo = f.repo();
        let (task, attempt) = create(&mut repo);
        let prepared = prepare(&mut repo, &task, &attempt);
        f.db().execute_batch(mutation).unwrap();
        assert!(
            repo.claim_ftp_attempt(&attempt.origin, Some(&prepared))
                .is_err(),
            "{mutation}"
        );
        assert!(repo.guard_ftp_publication(&task, &prepared).is_err());
        assert!(repo.complete_ftp_publication(&task, &prepared).is_err());
        assert!(repo.claim_ftp_attempt(&attempt.origin, None).is_err());
    }
}

#[test]
fn recovery_claim_requires_queued_completion_and_rolls_back_generation_and_task() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let (mut task, attempt) = create(&mut repo);
    for state in [
        TaskState::Downloading,
        TaskState::Failed,
        TaskState::Retrying,
        TaskState::Paused,
        TaskState::Created,
    ] {
        task.state = state;
        repo.update(task.clone()).unwrap();
        assert!(repo.claim_ftp_publication(&task, &attempt).is_err());
    }
    task.state = TaskState::Queued;
    task.progress.speed_bytes_per_second = 121;
    task.progress.eta_seconds = Some(17);
    task.last_error = Some("old".into());
    repo.update(task.clone()).unwrap();
    let mut stale = task.clone();
    stale.last_error = None;
    assert!(repo.claim_ftp_publication(&stale, &attempt).is_err());
    let db = f.db();
    for target in ["generation ON ftp_attempts", "state ON tasks"] {
        db.execute_batch(&format!(
            "CREATE TRIGGER fail BEFORE UPDATE OF {target} BEGIN SELECT RAISE(ABORT,'stop'); END"
        ))
        .unwrap();
        assert!(repo.claim_ftp_publication(&task, &attempt).is_err());
        assert_eq!(
            repo.ftp_attempt(&attempt.origin).unwrap(),
            Some(attempt.clone())
        );
        assert_eq!(repo.get(&task.id).unwrap(), Some(task.clone()));
        db.execute_batch("DROP TRIGGER fail").unwrap();
    }
    let (claimed, a) = repo.claim_ftp_publication(&task, &attempt).unwrap();
    assert_eq!(claimed.progress, task.progress);
    assert_eq!(claimed.last_error, None);
    assert_ne!(a.generation, attempt.generation);
    assert_eq!(a.completion, attempt.completion);
    let prepared = prepare(&mut repo, &claimed, &a);
    assert_eq!(prepared.completion, attempt.completion);
}

#[test]
fn prepare_rejects_invalid_authority_and_writes_atomically() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let (task, attempt) = create(&mut repo);
    let id = random_id().unwrap();
    assert!(
        repo.prepare_ftp_publication(&task, &attempt, "invalid", b"x")
            .is_err()
    );
    assert!(
        repo.prepare_ftp_publication(&task, &attempt, &id, b"")
            .is_err()
    );
    assert!(
        repo.prepare_ftp_publication(&task, &attempt, &id, &vec![9; 65537])
            .is_err()
    );
    let db = f.db();
    for sql in [
        "INSERT ON ftp_publications",
        "UPDATE OF publication_seen ON ftp_attempts",
    ] {
        db.execute_batch(&format!(
            "CREATE TRIGGER fail BEFORE {sql} BEGIN SELECT RAISE(ABORT,'stop'); END"
        ))
        .unwrap();
        assert!(
            repo.prepare_ftp_publication(&task, &attempt, &id, b"x")
                .is_err()
        );
        assert_eq!(
            repo.ftp_attempt(&attempt.origin).unwrap(),
            Some(attempt.clone())
        );
        db.execute_batch("DROP TRIGGER fail").unwrap();
    }
    let saved = repo
        .prepare_ftp_publication(&task, &attempt, &id, &vec![9; 65536])
        .unwrap();
    assert_eq!(saved.publication.unwrap().plan.len(), 65536);
}

#[test]
fn version_five_migration_preserves_data_and_rolls_back_ddl_at_each_failure() {
    for (fault, undo) in [
        (
            "ALTER TABLE ftp_attempts ADD COLUMN publication_authorization TEXT",
            "ALTER TABLE ftp_attempts DROP COLUMN publication_authorization",
        ),
        (
            "CREATE TRIGGER fail BEFORE UPDATE OF version ON schema_version WHEN NEW.version=6 BEGIN SELECT RAISE(IGNORE); END",
            "DROP TRIGGER fail",
        ),
        (
            "CREATE TABLE ftp_publications (saved TEXT)",
            "DROP TABLE ftp_publications",
        ),
        (
            "CREATE TRIGGER fail BEFORE UPDATE OF version ON schema_version WHEN NEW.version=6 BEGIN SELECT RAISE(ABORT,'stop'); END",
            "DROP TRIGGER fail",
        ),
    ] {
        let f = Fixture::new();
        let mut repo = f.repo();
        let (task, attempt) = create(&mut repo);
        drop(repo);
        let db = f.db();
        db.execute_batch("DROP TABLE ftp_publications; ALTER TABLE ftp_attempts DROP COLUMN publication_authorization; ALTER TABLE ftp_attempts DROP COLUMN publication_seen; UPDATE schema_version SET version=5").unwrap();
        db.execute_batch(fault).unwrap();
        let schema = || {
            db.prepare("SELECT sql FROM sqlite_schema ORDER BY name")
                .unwrap()
                .query_map([], |r| r.get::<_, Option<String>>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let before = schema();
        assert!(SqliteRepository::open(f.0.join("db")).is_err());
        assert_eq!(schema(), before);
        assert_eq!(
            db.query_row("SELECT version FROM schema_version", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            5
        );
        db.execute_batch(undo).unwrap();
        let repo = f.repo();
        assert_eq!(repo.get(&task.id).unwrap(), Some(task.clone()));
        assert_eq!(
            repo.active_origin(&task.id).unwrap().as_ref(),
            Some(&attempt.origin)
        );
        assert_eq!(repo.instance_id().unwrap(), attempt.instance_id);
        assert_eq!(repo.ftp_attempt(&attempt.origin).unwrap(), Some(attempt));
        assert!(repo.completed_ftp_publications().unwrap().is_empty());
    }
}

#[test]
fn final_write_observes_journal_before_commit_and_preserves_retry() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let (task, attempt) = create(&mut repo);
    let prepared = prepare(&mut repo, &task, &attempt);
    let db = f.db();
    db.execute_batch("CREATE TRIGGER drop_result AFTER UPDATE OF phase ON ftp_publications BEGIN DELETE FROM ftp_publications WHERE operation_id=NEW.operation_id; END").unwrap();
    assert!(repo.complete_ftp_publication(&task, &prepared).is_err());
    assert_eq!(repo.get(&task.id).unwrap(), Some(task.clone()));
    assert_eq!(
        repo.ftp_attempt(&prepared.origin).unwrap(),
        Some(prepared.clone())
    );
    db.execute_batch("DROP TRIGGER drop_result").unwrap();
    repo.complete_ftp_publication(&task, &prepared).unwrap();
}

#[test]
fn concurrent_prepare_has_one_winner_and_old_snapshot_never_authorizes_a_different_plan() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let (task, attempt) = create(&mut repo);
    let barrier = std::sync::Barrier::new(2);
    let candidates = [f.repo(), f.repo()];
    let outcomes = std::thread::scope(|scope| {
        let mut threads = vec![];
        for (index, mut conn) in candidates.into_iter().enumerate() {
            let (barrier, task, attempt) = (&barrier, &task, &attempt);
            threads.push(scope.spawn(move || {
                barrier.wait();
                conn.prepare_ftp_publication(task, attempt, &random_id().unwrap(), &[index as u8])
            }));
        }
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    let saved = outcomes.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(
        repo.ftp_attempt(&attempt.origin).unwrap(),
        Some(saved.clone())
    );
    f.db()
        .execute_batch("UPDATE ftp_publications SET plan=X'0809'")
        .unwrap();
    assert!(repo.guard_ftp_attempt(&saved).is_err());
    assert!(
        repo.claim_ftp_attempt(&attempt.origin, Some(&saved))
            .is_err()
    );
    assert!(repo.guard_ftp_publication(&task, &saved).is_err());
    assert!(repo.complete_ftp_publication(&task, &saved).is_err());
    assert!(
        repo.save_ftp_checkpoint(&saved, saved.checkpoint.as_ref().unwrap())
            .is_err()
    );
}

#[test]
fn old_operation_cannot_be_reserved_again_after_removal() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let (task, attempt) = create(&mut repo);
    prepare(&mut repo, &task, &attempt);
    repo.remove(&task.id).unwrap();
    assert_eq!(
        repo.reserve_origin(&attempt.origin).unwrap(),
        Reservation::Conflict
    );
    let mut other = attempt.origin.clone();
    other.task_id = "different-task".into();
    assert_eq!(repo.reserve_origin(&other).unwrap(), Reservation::Conflict);
    assert_eq!(repo.orphan_ftp_publications().unwrap().len(), 1);
}

#[test]
fn uncompleted_prefix_cannot_be_claimed_or_prepared_as_publication() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let (mut task, attempt) = create(&mut repo);
    f.db().execute_batch("UPDATE ftp_attempts SET completion_seen=0,completion_proof=NULL,completion_version=NULL,completion_bytes=NULL,completion_generation=NULL").unwrap();
    let prefix = repo.ftp_attempt(&attempt.origin).unwrap().unwrap();
    assert!(prefix.completion.is_none());
    assert!(
        repo.prepare_ftp_publication(&task, &prefix, &random_id().unwrap(), b"plan")
            .is_err()
    );
    task.state = TaskState::Queued;
    repo.update(task.clone()).unwrap();
    assert!(repo.claim_ftp_publication(&task, &prefix).is_err());
}

#[test]
fn debug_snapshots_hide_opaque_proofs_and_private_paths() {
    let f = Fixture::new();
    let mut repo = f.repo();
    let (task, attempt) = create(&mut repo);
    let prepared = prepare(&mut repo, &task, &attempt);
    let snapshot = format!("{prepared:?}");
    assert!(!snapshot.contains(task.destination.as_str()));
    for secret in ["[7]", "[11]", "[112, 114"] {
        assert!(!snapshot.contains(secret));
    }
    assert!(!format!("{:?}", prepared.checkpoint.unwrap()).contains("[7]"));
    assert!(!format!("{:?}", prepared.completion.unwrap()).contains("[11]"));
}

#[test]
fn completed_diagnostics_detect_lost_observed_rows_and_mismatched_current_marker() {
    for mutation in [
        "DELETE FROM ftp_publications",
        "DELETE FROM ftp_attempts",
        "UPDATE ftp_attempts SET publication_seen=0",
        "UPDATE ftp_attempts SET publication_authorization='stale'",
        "UPDATE ftp_publications SET revision=revision+1",
    ] {
        let f = Fixture::new();
        let mut repo = f.repo();
        let (task, attempt) = create(&mut repo);
        let prepared = prepare(&mut repo, &task, &attempt);
        repo.complete_ftp_publication(&task, &prepared).unwrap();
        f.db().execute_batch(mutation).unwrap();
        assert!(repo.completed_ftp_publications().is_err(), "{mutation}");
    }
}
