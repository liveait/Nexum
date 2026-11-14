//! Persistence boundaries for Nexum task state.

pub mod origin;

use nexum_domain::{Destination, DownloadSource, Progress, TaskId};
use nexum_task::{DownloadTask, TaskState};
use std::{collections::HashMap, fmt};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageError {
    NotFound(TaskId),
    AlreadyExists(TaskId),
    Other(String),
    Busy,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "task not found: {id}"),
            Self::AlreadyExists(id) => write!(f, "task already exists: {id}"),
            Self::Busy => f.write_str("database busy, retry request"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for StorageError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredTask {
    pub id: TaskId,
    pub source: DownloadSource,
    pub destination: Destination,
    pub state: TaskState,
    pub progress: Progress,
    pub last_error: Option<String>,
}

impl From<DownloadTask> for StoredTask {
    fn from(task: DownloadTask) -> Self {
        Self {
            id: task.id,
            source: task.source,
            destination: task.destination,
            state: task.state,
            progress: task.progress,
            last_error: task.last_error,
        }
    }
}

impl StoredTask {
    pub fn into_task(self) -> DownloadTask {
        DownloadTask {
            id: self.id,
            source: self.source,
            destination: self.destination,
            state: self.state,
            progress: self.progress,
            last_error: self.last_error,
        }
    }
}

pub trait TaskRepository {
    fn insert(&mut self, task: StoredTask) -> Result<(), StorageError>;
    fn get(&self, id: &TaskId) -> Result<Option<StoredTask>, StorageError>;
    fn list(&self) -> Result<Vec<StoredTask>, StorageError>;
    fn update(&mut self, task: StoredTask) -> Result<(), StorageError>;
    fn remove(&mut self, id: &TaskId) -> Result<Option<StoredTask>, StorageError>;
}

#[derive(Default, Debug)]
pub struct InMemoryRepository {
    tasks: HashMap<TaskId, StoredTask>,
}

impl InMemoryRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TaskRepository for InMemoryRepository {
    fn insert(&mut self, task: StoredTask) -> Result<(), StorageError> {
        if self.tasks.contains_key(&task.id) {
            return Err(StorageError::AlreadyExists(task.id));
        }
        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    fn get(&self, id: &TaskId) -> Result<Option<StoredTask>, StorageError> {
        Ok(self.tasks.get(id).cloned())
    }

    fn list(&self) -> Result<Vec<StoredTask>, StorageError> {
        Ok(self.tasks.values().cloned().collect())
    }

    fn update(&mut self, task: StoredTask) -> Result<(), StorageError> {
        if !self.tasks.contains_key(&task.id) {
            return Err(StorageError::NotFound(task.id));
        }
        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    fn remove(&mut self, id: &TaskId) -> Result<Option<StoredTask>, StorageError> {
        Ok(self.tasks.remove(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexum_domain::{Destination, DownloadSource};

    fn stored_task(id: &str) -> StoredTask {
        DownloadTask::new(
            id,
            DownloadSource::new(format!("https://example.com/{id}")),
            Destination::new(format!("/tmp/{id}")),
        )
        .into()
    }

    #[test]
    fn insert_and_get_round_trip() {
        let mut repo = InMemoryRepository::new();
        let task = stored_task("task-1");
        repo.insert(task.clone()).unwrap();
        assert_eq!(repo.get(&TaskId::from("task-1")).unwrap(), Some(task));
    }

    #[test]
    fn duplicate_insert_is_rejected() {
        let mut repo = InMemoryRepository::new();
        repo.insert(stored_task("task-1")).unwrap();
        assert_eq!(
            repo.insert(stored_task("task-1")).unwrap_err(),
            StorageError::AlreadyExists(TaskId::from("task-1"))
        );
    }

    #[test]
    fn update_requires_existing_task() {
        let mut repo = InMemoryRepository::new();
        let task = stored_task("task-1");
        assert_eq!(
            repo.update(task).unwrap_err(),
            StorageError::NotFound(TaskId::from("task-1"))
        );
    }

    #[test]
    fn remove_returns_task() {
        let mut repo = InMemoryRepository::new();
        let task = stored_task("task-1");
        repo.insert(task.clone()).unwrap();
        assert_eq!(repo.remove(&TaskId::from("task-1")).unwrap(), Some(task));
        assert_eq!(repo.get(&TaskId::from("task-1")).unwrap(), None);
    }

    #[test]
    fn task_conversion_round_trips() {
        let task = stored_task("task-1").into_task();
        assert_eq!(task.id, TaskId::from("task-1"));
        assert_eq!(task.state, TaskState::Created);
    }
}

pub struct SqliteRepository {
    connection: rusqlite::Connection,
}

impl SqliteRepository {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StorageError> {
        let connection =
            rusqlite::Connection::open(path).map_err(|e| StorageError::Other(e.to_string()))?;
        let mut repository = Self { connection };
        repository.initialize()?;
        Ok(repository)
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let connection = rusqlite::Connection::open_in_memory()
            .map_err(|e| StorageError::Other(e.to_string()))?;
        let mut repository = Self { connection };
        repository.initialize()?;
        Ok(repository)
    }

    fn initialize(&mut self) -> Result<(), StorageError> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );",
            )
            .map_err(|e| StorageError::Other(e.to_string()))?;

        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .map_err(|e| StorageError::Other(e.to_string()))?;

        if count == 0 {
            self.connection
                .execute("INSERT INTO schema_version (version) VALUES (0)", [])
                .map_err(|e| StorageError::Other(e.to_string()))?;
        }

        self.migrate()
    }

    fn migrate(&mut self) -> Result<(), StorageError> {
        let version: i64 = self
            .connection
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .map_err(|e| StorageError::Other(e.to_string()))?;

        if version > 3 {
            return Err(StorageError::Other(format!(
                "unsupported schema version: {version}"
            )));
        }

        if version < 1 {
            let transaction = self
                .connection
                .transaction()
                .map_err(|e| StorageError::Other(e.to_string()))?;
            transaction
                .execute_batch(
                    "CREATE TABLE tasks (
                    id TEXT PRIMARY KEY,
                    source TEXT NOT NULL,
                    destination TEXT NOT NULL,
                    state TEXT NOT NULL,
                    downloaded_bytes INTEGER NOT NULL,
                    total_bytes INTEGER,
                    speed_bytes_per_second INTEGER NOT NULL,
                    eta_seconds INTEGER
                );
                UPDATE schema_version SET version = 1;",
                )
                .map_err(|e| StorageError::Other(e.to_string()))?;
            transaction
                .commit()
                .map_err(|e| StorageError::Other(e.to_string()))?;
        }

        if version < 2 {
            let transaction = self
                .connection
                .transaction()
                .map_err(|e| StorageError::Other(e.to_string()))?;
            transaction
                .execute("ALTER TABLE tasks ADD COLUMN last_error TEXT", [])
                .map_err(|e| StorageError::Other(e.to_string()))?;
            transaction
                .execute("UPDATE schema_version SET version = 2", [])
                .map_err(|e| StorageError::Other(e.to_string()))?;
            transaction
                .commit()
                .map_err(|e| StorageError::Other(e.to_string()))?;
        }

        self.migrate_origins()
    }

    fn insert_task(
        connection: &rusqlite::Connection,
        task: &StoredTask,
    ) -> Result<(), StorageError> {
        connection.execute(
            "INSERT INTO tasks
             (id, source, destination, state, downloaded_bytes, total_bytes, speed_bytes_per_second, eta_seconds, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                task.id.as_str(),
                task.source.as_str(),
                task.destination.as_str(),
                Self::state_to_str(task.state),
                task.progress.downloaded_bytes,
                task.progress.total_bytes,
                task.progress.speed_bytes_per_second,
                task.progress.eta_seconds,
                task.last_error.as_deref(),
            ],
        ).map_err(|e| match e {
            rusqlite::Error::SqliteFailure(ref error, _) if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY =>
                StorageError::AlreadyExists(task.id.clone()),
            other => StorageError::Other(other.to_string()),
        })?;
        Ok(())
    }

    fn state_to_str(state: TaskState) -> &'static str {
        match state {
            TaskState::Created => "created",
            TaskState::Queued => "queued",
            TaskState::Downloading => "downloading",
            TaskState::Paused => "paused",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
            TaskState::Retrying => "retrying",
        }
    }

    fn state_from_str(value: &str) -> Result<TaskState, StorageError> {
        match value {
            "created" => Ok(TaskState::Created),
            "queued" => Ok(TaskState::Queued),
            "downloading" => Ok(TaskState::Downloading),
            "paused" => Ok(TaskState::Paused),
            "completed" => Ok(TaskState::Completed),
            "failed" => Ok(TaskState::Failed),
            "retrying" => Ok(TaskState::Retrying),
            _ => Err(StorageError::Other(format!("unknown task state: {value}"))),
        }
    }

    fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTask> {
        let state: String = row.get(3)?;
        Ok(StoredTask {
            id: TaskId::from(row.get::<_, String>(0)?),
            source: DownloadSource::new(row.get::<_, String>(1)?),
            destination: Destination::new(row.get::<_, String>(2)?),
            state: Self::state_from_str(&state).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            progress: Progress {
                downloaded_bytes: row.get(4)?,
                total_bytes: row.get(5)?,
                speed_bytes_per_second: row.get(6)?,
                eta_seconds: row.get(7)?,
            },
            last_error: row.get(8)?,
        })
    }
}

impl TaskRepository for SqliteRepository {
    fn insert(&mut self, task: StoredTask) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(origin::database_error)?;
        let reserved: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_origins WHERE task_id=?1)",
                [task.id.as_str()],
                |row| row.get(0),
            )
            .map_err(origin::database_error)?;
        if reserved {
            return Err(StorageError::AlreadyExists(task.id));
        }
        Self::insert_task(&transaction, &task)?;
        transaction.commit().map_err(origin::database_error)
    }

    fn get(&self, id: &TaskId) -> Result<Option<StoredTask>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id, source, destination, state, downloaded_bytes, total_bytes, speed_bytes_per_second, eta_seconds, last_error
             FROM tasks WHERE id = ?1"
        ).map_err(|e| StorageError::Other(e.to_string()))?;
        let mut rows = statement
            .query(rusqlite::params![id.as_str()])
            .map_err(|e| StorageError::Other(e.to_string()))?;
        match rows
            .next()
            .map_err(|e| StorageError::Other(e.to_string()))?
        {
            Some(row) => Self::row_to_task(row)
                .map(Some)
                .map_err(|e| StorageError::Other(e.to_string())),
            None => Ok(None),
        }
    }

    fn list(&self) -> Result<Vec<StoredTask>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id, source, destination, state, downloaded_bytes, total_bytes, speed_bytes_per_second, eta_seconds, last_error
             FROM tasks ORDER BY rowid"
        ).map_err(|e| StorageError::Other(e.to_string()))?;
        let rows = statement
            .query_map([], Self::row_to_task)
            .map_err(|e| StorageError::Other(e.to_string()))?;
        rows.map(|row| row.map_err(|e| StorageError::Other(e.to_string())))
            .collect()
    }

    fn update(&mut self, task: StoredTask) -> Result<(), StorageError> {
        let changed = self
            .connection
            .execute(
                "UPDATE tasks SET source = ?2, destination = ?3, state = ?4, downloaded_bytes = ?5,
             total_bytes = ?6, speed_bytes_per_second = ?7, eta_seconds = ?8, last_error = ?9 WHERE id = ?1",
                rusqlite::params![
                    task.id.as_str(),
                    task.source.as_str(),
                    task.destination.as_str(),
                    Self::state_to_str(task.state),
                    task.progress.downloaded_bytes,
                    task.progress.total_bytes,
                    task.progress.speed_bytes_per_second,
                    task.progress.eta_seconds,
                    task.last_error,
                ],
            )
            .map_err(|e| StorageError::Other(e.to_string()))?;
        if changed == 0 {
            return Err(StorageError::NotFound(task.id));
        }
        Ok(())
    }

    fn remove(&mut self, id: &TaskId) -> Result<Option<StoredTask>, StorageError> {
        self.remove_with_origin(id)
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use nexum_domain::{Destination, DownloadSource};

    fn task(id: &str) -> StoredTask {
        DownloadTask::new(
            id,
            DownloadSource::new("https://example.com/file"),
            Destination::new("/tmp/file"),
        )
        .into()
    }

    #[test]
    fn sqlite_repository_round_trips_tasks() {
        let mut repo = SqliteRepository::open_in_memory().unwrap();
        let original = task("task-1");
        repo.insert(original.clone()).unwrap();
        assert_eq!(repo.get(&TaskId::from("task-1")).unwrap(), Some(original));
    }

    #[test]
    fn sqlite_repository_persists_updates_and_removals() {
        let mut repo = SqliteRepository::open_in_memory().unwrap();
        let mut task = task("task-1");
        repo.insert(task.clone()).unwrap();
        task.state = TaskState::Queued;
        task.progress = Progress::new(42, Some(100));
        task.last_error = Some("previous transfer failed".into());
        repo.update(task.clone()).unwrap();
        assert_eq!(
            repo.get(&TaskId::from("task-1")).unwrap(),
            Some(task.clone())
        );
        assert_eq!(repo.remove(&TaskId::from("task-1")).unwrap(), Some(task));
    }

    #[test]
    fn sqlite_repository_initializes_schema_version() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let version: i64 = repo
            .connection
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn sqlite_repository_migrates_v1_tasks_with_empty_errors() {
        let path = std::env::temp_dir().join(format!(
            "nexum-storage-migration-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE schema_version (version INTEGER NOT NULL);
                     INSERT INTO schema_version (version) VALUES (1);
                     CREATE TABLE tasks (
                         id TEXT PRIMARY KEY,
                         source TEXT NOT NULL,
                         destination TEXT NOT NULL,
                         state TEXT NOT NULL,
                         downloaded_bytes INTEGER NOT NULL,
                         total_bytes INTEGER,
                         speed_bytes_per_second INTEGER NOT NULL,
                         eta_seconds INTEGER
                     );
                     INSERT INTO tasks VALUES
                         ('legacy', 'https://example.com/file', '/tmp/file', 'queued', 12, 100, 0, NULL);",
                )
                .unwrap();
        }

        let repository = SqliteRepository::open(&path).unwrap();
        let stored = repository.get(&TaskId::from("legacy")).unwrap().unwrap();
        assert_eq!(stored.progress.downloaded_bytes, 12);
        assert_eq!(stored.last_error, None);
        let version: i64 = repository
            .connection
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 3);
        drop(repository);
        let _ = std::fs::remove_file(path);
    }
}
