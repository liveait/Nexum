use nexum_domain::{Destination, DownloadSource, TaskId};
use nexum_storage::{SqliteRepository, TaskRepository, origin::*};
use nexum_task::DownloadTask;
fn record(repo: &mut SqliteRepository, id: &str) -> OriginRecord {
    OriginRecord {
        version: 1,
        task_id: id.into(),
        destination: "/tmp/file".into(),
        canonical_url: "ftp://example.com/pub/a.bin".into(),
        origin: "ftp://example.com:21".into(),
        command_path: "pub/a.bin".into(),
        security: "plain".into(),
        path_mode: "login_relative".into(),
        authenticated: true,
        operation_id: random_id().unwrap(),
        credential: Some(CredentialRef {
            instance_id: repo.instance_id().unwrap(),
            origin: "ftp://example.com:21".into(),
            credential_id: random_id().unwrap(),
            revision: 1,
        }),
    }
}
fn task(id: &str) -> nexum_storage::StoredTask {
    DownloadTask::new(
        id,
        DownloadSource::new("ftp://example.com/pub/a.bin"),
        Destination::new("/tmp/file"),
    )
    .into()
}
#[test]
fn pending_blocks_legacy_insert_and_activation_is_atomic() {
    let mut repo = SqliteRepository::open_in_memory().unwrap();
    let value = record(&mut repo, "same");
    assert_eq!(repo.reserve_origin(&value).unwrap(), Reservation::New);
    assert!(repo.insert(task("same")).is_err());
    assert_eq!(
        repo.reserve_origin(&value).unwrap(),
        Reservation::Pending(value.clone())
    );
    let mut wrong = value.clone();
    wrong.operation_id = "wrong".into();
    assert!(repo.activate_origin(&wrong, task("same")).is_err());
    assert!(repo.get(&TaskId::from("same")).unwrap().is_none());
    repo.activate_origin(&value, task("same")).unwrap();
    assert_eq!(
        repo.active_origin(&TaskId::from("same")).unwrap(),
        Some(value.clone())
    );
    assert_eq!(
        repo.reserve_origin(&value).unwrap(),
        Reservation::Existing(value)
    );
}
#[test]
fn deletion_keeps_exact_tombstones_and_reuse_cannot_ack_new_reference() {
    let mut repo = SqliteRepository::open_in_memory().unwrap();
    let old = record(&mut repo, "reuse");
    repo.reserve_origin(&old).unwrap();
    repo.activate_origin(&old, task("reuse")).unwrap();
    repo.remove(&TaskId::from("reuse")).unwrap();
    assert!(
        repo.active_origin(&TaskId::from("reuse"))
            .unwrap()
            .is_none()
    );
    assert_eq!(repo.cleanup_records().unwrap(), vec![old.clone()]);
    let fresh = record(&mut repo, "reuse");
    repo.reserve_origin(&fresh).unwrap();
    repo.activate_origin(&fresh, task("reuse")).unwrap();
    repo.ack_cleanup(&fresh).unwrap();
    assert_eq!(repo.cleanup_records().unwrap(), vec![old.clone()]);
    repo.ack_cleanup(&old).unwrap();
    assert!(repo.cleanup_records().unwrap().is_empty());
    assert_eq!(
        repo.active_origin(&TaskId::from("reuse")).unwrap(),
        Some(fresh)
    );
}
#[test]
fn abandon_pending_is_fenced_by_operation_and_survives_reopen() {
    let mut repo = SqliteRepository::open_in_memory().unwrap();
    let value = record(&mut repo, "pending");
    repo.reserve_origin(&value).unwrap();
    let mut wrong = value.clone();
    wrong.operation_id = "wrong".into();
    repo.abandon_origin(&wrong).unwrap();
    assert_eq!(repo.pending_origins().unwrap(), vec![value.clone()]);
    repo.abandon_origin(&value).unwrap();
    assert!(repo.pending_origins().unwrap().is_empty());
    assert_eq!(repo.cleanup_records().unwrap(), vec![value]);
}

#[test]
fn pending_and_tombstone_records_survive_real_database_reopen() {
    let root = std::env::temp_dir().join(format!("nexum-origin-reopen-{}", random_id().unwrap()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("nexum.sqlite");
    let mut repository = SqliteRepository::open(&path).unwrap();
    let pending = record(&mut repository, "pending");
    let identity = repository.instance_id().unwrap();
    repository.reserve_origin(&pending).unwrap();
    drop(repository);
    let mut repository = SqliteRepository::open(&path).unwrap();
    assert_eq!(repository.instance_id().unwrap(), identity);
    assert_eq!(
        repository.pending_origins().unwrap(),
        std::slice::from_ref(&pending)
    );
    repository.abandon_origin(&pending).unwrap();
    drop(repository);
    let repository = SqliteRepository::open(&path).unwrap();
    assert!(repository.pending_origins().unwrap().is_empty());
    assert_eq!(repository.cleanup_records().unwrap(), [pending]);
    let other = SqliteRepository::open_in_memory().unwrap();
    assert_ne!(
        repository.instance_id().unwrap(),
        other.instance_id().unwrap()
    );
    drop(repository);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_task_deletion_rolls_back_tombstone_and_origin_changes() {
    let root = std::env::temp_dir().join(format!("nexum-origin-rollback-{}", random_id().unwrap()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("nexum.sqlite");
    let mut repository = SqliteRepository::open(&path).unwrap();
    let active = record(&mut repository, "keep");
    repository.reserve_origin(&active).unwrap();
    repository.activate_origin(&active, task("keep")).unwrap();
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TRIGGER reject_delete BEFORE DELETE ON tasks BEGIN SELECT RAISE(FAIL,'controlled deletion failure'); END;").unwrap();
    assert!(repository.remove(&TaskId::from("keep")).is_err());
    assert_eq!(
        repository.active_origin(&TaskId::from("keep")).unwrap(),
        Some(active)
    );
    assert!(repository.get(&TaskId::from("keep")).unwrap().is_some());
    assert!(repository.cleanup_records().unwrap().is_empty());
    drop(repository);
    std::fs::remove_dir_all(root).unwrap();
}
