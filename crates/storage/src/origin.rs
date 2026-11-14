//! Secret-free, versioned origin bindings and durable cross-store cleanup.
use crate::{SqliteRepository, StorageError, StoredTask};
use nexum_domain::TaskId;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRef {
    pub instance_id: String,
    pub origin: String,
    pub credential_id: String,
    pub revision: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginRecord {
    pub version: u32,
    pub task_id: String,
    pub destination: String,
    pub canonical_url: String,
    pub origin: String,
    pub command_path: String,
    pub security: String,
    pub path_mode: String,
    pub authenticated: bool,
    pub operation_id: String,
    pub credential: Option<CredentialRef>,
}
impl OriginRecord {
    pub fn same_input(&self, other: &Self) -> bool {
        self.version == other.version
            && self.task_id == other.task_id
            && self.destination == other.destination
            && self.canonical_url == other.canonical_url
            && self.origin == other.origin
            && self.command_path == other.command_path
            && self.security == other.security
            && self.path_mode == other.path_mode
            && self.authenticated == other.authenticated
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reservation {
    New,
    Pending(OriginRecord),
    Existing(OriginRecord),
    Conflict,
}

pub fn random_id() -> Result<String, StorageError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| StorageError::Other("random identity unavailable".into()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
pub(crate) fn database_error(error: rusqlite::Error) -> StorageError {
    if matches!(&error, rusqlite::Error::SqliteFailure(code, _) if matches!(code.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
    {
        StorageError::Busy
    } else {
        StorageError::Other("origin database operation failed".into())
    }
}
fn read_record(
    connection: &Connection,
    id: &str,
    state: &str,
) -> Result<Option<OriginRecord>, StorageError> {
    let encoded: Option<String> = connection
        .query_row(
            "SELECT record FROM task_origins WHERE task_id=?1 AND status=?2",
            params![id, state],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;
    encoded
        .map(|value| {
            serde_json::from_str(&value)
                .map_err(|_| StorageError::Other("invalid origin record".into()))
        })
        .transpose()
}
fn records(
    connection: &Connection,
    table: &str,
    state: Option<&str>,
) -> Result<Vec<OriginRecord>, StorageError> {
    let sql = if state.is_some() {
        format!("SELECT record FROM {table} WHERE status=?1 ORDER BY rowid")
    } else {
        format!("SELECT record FROM {table} ORDER BY rowid")
    };
    let mut statement = connection.prepare(&sql).map_err(database_error)?;
    let mut rows = if let Some(state) = state {
        statement.query([state])
    } else {
        statement.query([])
    }
    .map_err(database_error)?;
    let mut result = Vec::new();
    while let Some(row) = rows.next().map_err(database_error)? {
        let encoded: String = row.get(0).map_err(database_error)?;
        result.push(
            serde_json::from_str(&encoded)
                .map_err(|_| StorageError::Other("invalid origin record".into()))?,
        );
    }
    Ok(result)
}
fn encode(record: &OriginRecord) -> Result<String, StorageError> {
    serde_json::to_string(record).map_err(|_| StorageError::Other("invalid origin record".into()))
}
fn tombstone(connection: &Connection, record: &OriginRecord) -> Result<(), StorageError> {
    if record.credential.is_some() {
        connection
            .execute(
                "INSERT OR IGNORE INTO origin_cleanup(operation_id,record) VALUES(?1,?2)",
                params![record.operation_id, encode(record)?],
            )
            .map_err(database_error)?;
    }
    Ok(())
}
impl SqliteRepository {
    pub(crate) fn migrate_origins(&mut self) -> Result<(), StorageError> {
        self.connection
            .busy_timeout(std::time::Duration::from_millis(250))
            .map_err(database_error)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS origin_instance (singleton INTEGER PRIMARY KEY CHECK(singleton=1), identity TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS task_origins(task_id TEXT PRIMARY KEY, operation_id TEXT UNIQUE NOT NULL, status TEXT NOT NULL CHECK(status IN ('pending','active')), record TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS origin_cleanup(operation_id TEXT PRIMARY KEY, record TEXT NOT NULL);
            UPDATE schema_version SET version=3;").map_err(database_error)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO origin_instance(singleton,identity) VALUES(1,?1)",
                [random_id()?],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)
    }
    pub fn instance_id(&self) -> Result<String, StorageError> {
        self.connection
            .query_row(
                "SELECT identity FROM origin_instance WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(database_error)
    }
    pub fn reserve_origin(&mut self, record: &OriginRecord) -> Result<Reservation, StorageError> {
        let encoded = encode(record)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        if let Some(existing) = read_record(&transaction, &record.task_id, "active")? {
            return Ok(Reservation::Existing(existing));
        }
        if let Some(existing) = read_record(&transaction, &record.task_id, "pending")? {
            return Ok(Reservation::Pending(existing));
        }
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1)",
                [&record.task_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if exists {
            return Ok(Reservation::Conflict);
        }
        transaction.execute(
            "INSERT INTO task_origins(task_id,operation_id,status,record) VALUES(?1,?2,'pending',?3)",
            params![record.task_id, record.operation_id, encoded],
        ).map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        Ok(Reservation::New)
    }
    pub fn activate_origin(
        &mut self,
        record: &OriginRecord,
        task: StoredTask,
    ) -> Result<(), StorageError> {
        if task.id.as_str() != record.task_id
            || task.source.as_str() != record.canonical_url
            || task.destination.as_str() != record.destination
        {
            return Err(StorageError::Other("origin task mismatch".into()));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let existing = read_record(&transaction, &record.task_id, "pending")?;
        if existing.as_ref() != Some(record) {
            return Err(StorageError::Other("origin reservation mismatch".into()));
        }
        Self::insert_task(&transaction, &task)?;
        let count = transaction.execute(
            "UPDATE task_origins SET status='active' WHERE task_id=?1 AND operation_id=?2 AND status='pending'",
            params![record.task_id, record.operation_id],
        ).map_err(database_error)?;
        if count != 1 {
            return Err(StorageError::Other("origin reservation mismatch".into()));
        }
        transaction.commit().map_err(database_error)
    }
    pub fn active_origin(&self, id: &TaskId) -> Result<Option<OriginRecord>, StorageError> {
        read_record(&self.connection, id.as_str(), "active")
    }
    pub fn pending_origins(&self) -> Result<Vec<OriginRecord>, StorageError> {
        records(&self.connection, "task_origins", Some("pending"))
    }
    pub fn cleanup_records(&self) -> Result<Vec<OriginRecord>, StorageError> {
        records(&self.connection, "origin_cleanup", None)
    }
    pub fn abandon_origin(&mut self, record: &OriginRecord) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        if read_record(&transaction, &record.task_id, "pending")?.as_ref() == Some(record) {
            tombstone(&transaction, record)?;
            transaction.execute(
                "DELETE FROM task_origins WHERE task_id=?1 AND operation_id=?2 AND status='pending'",
                params![record.task_id, record.operation_id],
            ).map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)
    }
    pub fn discard_pending(&mut self, record: &OriginRecord) -> Result<(), StorageError> {
        self.connection.execute(
            "DELETE FROM task_origins WHERE task_id=?1 AND operation_id=?2 AND status='pending' AND record=?3",
            params![record.task_id, record.operation_id, encode(record)?],
        ).map_err(database_error)?;
        Ok(())
    }
    pub fn has_origin(&self, id: &TaskId) -> Result<bool, StorageError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_origins WHERE task_id=?1)",
                [id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error)
    }
    pub fn ack_cleanup(&mut self, record: &OriginRecord) -> Result<(), StorageError> {
        self.connection
            .execute(
                "DELETE FROM origin_cleanup WHERE operation_id=?1 AND record=?2",
                params![record.operation_id, encode(record)?],
            )
            .map_err(database_error)?;
        Ok(())
    }
    pub(crate) fn remove_with_origin(
        &mut self,
        id: &TaskId,
    ) -> Result<Option<StoredTask>, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let existing = transaction.query_row(
            "SELECT id,source,destination,state,downloaded_bytes,total_bytes,speed_bytes_per_second,eta_seconds,last_error FROM tasks WHERE id=?1",
            [id.as_str()],
            Self::row_to_task,
        ).optional().map_err(database_error)?;
        if let Some(record) = read_record(&transaction, id.as_str(), "active")? {
            tombstone(&transaction, &record)?;
        }
        transaction
            .execute(
                "DELETE FROM task_origins WHERE task_id=?1 AND status='active'",
                [id.as_str()],
            )
            .map_err(database_error)?;
        transaction
            .execute("DELETE FROM tasks WHERE id=?1", [id.as_str()])
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        Ok(existing)
    }
}
