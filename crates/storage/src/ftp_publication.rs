//! Durable FTP publication authority. Core validates the opaque Engine plan before preparing it.

use std::fmt;

use nexum_task::TaskState;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::{
    SqliteRepository, StorageError, StoredTask,
    ftp_resume::{FtpAttempt, FtpAttemptGuard, conflict, read_attempt, verify_origin},
    origin::{OriginRecord, database_error, random_id},
};

const MAX_PLAN_BYTES: usize = 64 * 1024;

#[derive(Clone, Eq, PartialEq)]
pub struct FtpPublicationIntent {
    pub owner_generation: String,
    pub authorization: String,
    pub revision: u64,
    pub plan: Vec<u8>,
}

impl fmt::Debug for FtpPublicationIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FtpPublicationIntent")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FtpPublicationPhase {
    Prepared,
    Completed,
}

/// Diagnostic metadata only. It does not contain a plan or confer attempt authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FtpPublicationRecord {
    pub instance_id: String,
    pub operation_id: String,
    pub task_id: String,
    pub revision: u64,
    pub phase: FtpPublicationPhase,
}

fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(super) fn read_intent(
    db: &Connection,
    origin: &OriginRecord,
    instance: &str,
    expected_phase: &str,
) -> Result<Option<FtpPublicationIntent>, StorageError> {
    let row = db
        .query_row(
            "SELECT version, task_id, phase, owner_generation, authorization, revision,
                CASE WHEN length(plan) BETWEEN 1 AND 65536 THEN plan END
         FROM ftp_publications WHERE instance_id=?1 AND operation_id=?2",
            params![instance, origin.operation_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    FtpPublicationIntent {
                        owner_generation: r.get(3)?,
                        authorization: r.get(4)?,
                        revision: r.get(5)?,
                        plan: r.get(6)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    row.map(|(version, task_id, phase, intent)| {
        if version != 1
            || task_id != origin.task_id
            || phase != expected_phase
            || !valid_token(&intent.owner_generation)
            || !valid_token(&intent.authorization)
            || intent.revision == 0
        {
            return Err(conflict());
        }
        Ok(intent)
    })
    .transpose()
}

fn immediate(db: &Connection) -> Result<Transaction<'_>, StorageError> {
    Transaction::new_unchecked(db, TransactionBehavior::Immediate).map_err(database_error)
}

fn match_task(db: &Connection, expected: &StoredTask) -> Result<(), StorageError> {
    let current = db.query_row(
        "SELECT id,source,destination,state,downloaded_bytes,total_bytes,speed_bytes_per_second,eta_seconds,last_error
         FROM tasks WHERE id=?1",
        [expected.id.as_str()], SqliteRepository::row_to_task,
    ).optional().map_err(database_error)?;
    if current.as_ref() != Some(expected) {
        return Err(conflict());
    }
    Ok(())
}

fn match_snapshot(
    db: &Connection,
    task: &StoredTask,
    expected: &FtpAttempt,
    state: TaskState,
) -> Result<(), StorageError> {
    if task.state != state
        || task.id.as_str() != expected.origin.task_id
        || expected.completion.is_none()
    {
        return Err(conflict());
    }
    match_task(db, task)?;
    let instance = verify_origin(db, &expected.origin)?;
    if read_attempt(db, &expected.origin, &instance)?.as_ref() != Some(expected) {
        return Err(conflict());
    }
    Ok(())
}

fn match_prepared(
    db: &Connection,
    task: &StoredTask,
    expected: &FtpAttempt,
) -> Result<(), StorageError> {
    match_snapshot(db, task, expected, TaskState::Downloading)?;
    let intent = expected.publication.as_ref().ok_or_else(conflict)?;
    if intent.owner_generation != expected.generation {
        return Err(conflict());
    }
    Ok(())
}

fn changed_one(result: rusqlite::Result<usize>) -> Result<(), StorageError> {
    if result.map_err(database_error)? != 1 {
        return Err(conflict());
    }
    Ok(())
}

impl SqliteRepository {
    pub(crate) fn migrate_ftp_publication(&mut self) -> Result<(), StorageError> {
        let tx = immediate(&self.connection)?;
        let version: i64 = tx
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .map_err(database_error)?;
        match version {
            6 => return tx.commit().map_err(database_error),
            5 => {}
            _ => return Err(conflict()),
        }
        tx.execute_batch(
            "ALTER TABLE ftp_attempts ADD COLUMN publication_seen INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE ftp_attempts ADD COLUMN publication_authorization TEXT;
             CREATE TABLE ftp_publications (
                 instance_id TEXT NOT NULL, operation_id TEXT NOT NULL,
                 task_id TEXT NOT NULL, version INTEGER NOT NULL CHECK (version=1),
                 owner_generation TEXT NOT NULL, authorization TEXT NOT NULL,
                 revision INTEGER NOT NULL CHECK (revision>0),
                 phase TEXT NOT NULL CHECK (phase IN ('prepared','completed')),
                 plan BLOB NOT NULL CHECK (length(plan) BETWEEN 1 AND 65536),
                 PRIMARY KEY(instance_id, operation_id)
             );",
        )
        .map_err(database_error)?;
        changed_one(tx.execute("UPDATE schema_version SET version=6 WHERE version=5", []))?;
        tx.commit().map_err(database_error)
    }

    /// Claim a recovered Queued task without resetting its proven whole-file progress.
    /// A retained intent still belongs to its old owner until explicitly rearmed.
    pub fn claim_ftp_publication(
        &mut self,
        task: &StoredTask,
        attempt: &FtpAttempt,
    ) -> Result<(StoredTask, FtpAttempt), StorageError> {
        let tx = immediate(&self.connection)?;
        match_snapshot(&tx, task, attempt, TaskState::Queued)?;
        let mut claimed = attempt.clone();
        claimed.generation = random_id()?;
        let mut downloading = task.clone();
        downloading.state = TaskState::Downloading;
        downloading.last_error = None;
        changed_one(tx.execute(
            "UPDATE ftp_attempts SET generation=?2 WHERE task_id=?1",
            params![task.id.as_str(), claimed.generation],
        ))?;
        changed_one(tx.execute(
            "UPDATE tasks SET state='downloading',last_error=NULL WHERE id=?1",
            [task.id.as_str()],
        ))?;
        match_snapshot(&tx, &downloading, &claimed, TaskState::Downloading)?;
        tx.commit().map_err(database_error)?;
        Ok((downloading, claimed))
    }

    /// Bind the first plan. The caller generates a fresh random_id and binds it into the Engine plan.
    pub fn prepare_ftp_publication(
        &mut self,
        task: &StoredTask,
        attempt: &FtpAttempt,
        authorization: &str,
        plan: &[u8],
    ) -> Result<FtpAttempt, StorageError> {
        if attempt.publication.is_some() {
            return Err(conflict());
        }
        self.write_ftp_publication(task, attempt, authorization, plan)
    }

    /// Rebind an existing immutable operation after an explicit claim of a new generation.
    pub fn rearm_ftp_publication(
        &mut self,
        task: &StoredTask,
        attempt: &FtpAttempt,
        authorization: &str,
        plan: &[u8],
    ) -> Result<FtpAttempt, StorageError> {
        let previous = attempt.publication.as_ref().ok_or_else(conflict)?;
        if previous.authorization == authorization
            || previous.owner_generation == attempt.generation
        {
            return Err(conflict());
        }
        self.write_ftp_publication(task, attempt, authorization, plan)
    }

    fn write_ftp_publication(
        &mut self,
        task: &StoredTask,
        attempt: &FtpAttempt,
        authorization: &str,
        plan: &[u8],
    ) -> Result<FtpAttempt, StorageError> {
        if !valid_token(authorization) || !(1..=MAX_PLAN_BYTES).contains(&plan.len()) {
            return Err(conflict());
        }
        let revision = attempt
            .revision
            .checked_add(1)
            .filter(|r| *r <= i64::MAX as u64)
            .ok_or_else(conflict)?;
        let tx = immediate(&self.connection)?;
        match_snapshot(&tx, task, attempt, TaskState::Downloading)?;
        let sql = if attempt.publication.is_some() {
            "UPDATE ftp_publications SET owner_generation=?4,authorization=?5,revision=?6,plan=?7 WHERE instance_id=?1 AND operation_id=?2 AND task_id=?3 AND phase='prepared'"
        } else {
            "INSERT INTO ftp_publications(instance_id,operation_id,task_id,version,phase,owner_generation,authorization,revision,plan)
             VALUES(?1,?2,?3,1,'prepared',?4,?5,?6,?7)"
        };
        changed_one(tx.execute(
            sql,
            params![
                attempt.instance_id,
                attempt.origin.operation_id,
                task.id.as_str(),
                attempt.generation,
                authorization,
                revision,
                plan
            ],
        ))?;
        changed_one(tx.execute("UPDATE ftp_attempts SET publication_seen=1,publication_authorization=?2,revision=?3 WHERE task_id=?1", params![task.id.as_str(), authorization, revision]))?;
        let mut saved = attempt.clone();
        saved.revision = revision;
        saved.publication = Some(FtpPublicationIntent {
            owner_generation: attempt.generation.clone(),
            authorization: authorization.into(),
            revision,
            plan: plan.into(),
        });
        match_prepared(&tx, task, &saved)?;
        tx.commit().map_err(database_error)?;
        Ok(saved)
    }

    /// Hold SQLite writer exclusion across the bounded namespace decision only.
    pub fn guard_ftp_publication(
        &self,
        task: &StoredTask,
        attempt: &FtpAttempt,
    ) -> Result<FtpAttemptGuard<'_>, StorageError> {
        let transaction = immediate(&self.connection)?;
        match_prepared(&transaction, task, attempt)?;
        Ok(FtpAttemptGuard {
            _transaction: transaction,
        })
    }

    /// Core supplies the independently verified namespace outcome. Recheck all authority after release.
    pub fn complete_ftp_publication(
        &mut self,
        task: &StoredTask,
        attempt: &FtpAttempt,
    ) -> Result<StoredTask, StorageError> {
        let tx = immediate(&self.connection)?;
        match_prepared(&tx, task, attempt)?;
        let bytes = attempt
            .completion
            .as_ref()
            .ok_or_else(conflict)?
            .trusted_bytes;
        let mut completed = task.clone();
        completed.state = TaskState::Completed;
        completed.progress = nexum_domain::Progress {
            downloaded_bytes: bytes,
            total_bytes: Some(bytes),
            speed_bytes_per_second: 0,
            eta_seconds: None,
        };
        completed.last_error = None;
        changed_one(tx.execute("UPDATE tasks SET state='completed',downloaded_bytes=?2,total_bytes=?2,speed_bytes_per_second=0,eta_seconds=NULL,last_error=NULL WHERE id=?1", params![task.id.as_str(),bytes]))?;
        changed_one(tx.execute("UPDATE ftp_publications SET phase='completed' WHERE instance_id=?1 AND operation_id=?2 AND phase='prepared'", params![attempt.instance_id,attempt.origin.operation_id]))?;
        match_task(&tx, &completed)?;
        if read_intent(&tx, &attempt.origin, &attempt.instance_id, "completed")?
            != attempt.publication
        {
            return Err(conflict());
        }
        tx.commit().map_err(database_error)?;
        Ok(completed)
    }

    /// Completed journals still attached to the matching completed task incarnation.
    pub fn completed_ftp_publications(&self) -> Result<Vec<FtpPublicationRecord>, StorageError> {
        // Start with the completed task so a missing journal with a retained
        // observed marker cannot quietly look like an empty diagnostic result.
        self.ftp_publication_records(
            "SELECT p.instance_id,p.operation_id,p.task_id,p.revision,p.phase,p.version,
                    COALESCE(a.publication_seen=1 AND a.instance_id=p.instance_id
                             AND a.publication_authorization=p.authorization AND a.revision=p.revision
                             AND p.phase='completed' AND p.task_id=t.id, 0)
             FROM tasks t JOIN task_origins o ON o.task_id=t.id AND o.status='active'
             LEFT JOIN ftp_attempts a ON a.task_id=t.id
             LEFT JOIN ftp_publications p ON p.operation_id=o.operation_id
                AND p.instance_id=(SELECT identity FROM origin_instance WHERE singleton=1)
             WHERE t.state='completed' AND (a.publication_seen!=0 OR a.publication_authorization IS NOT NULL OR p.operation_id IS NOT NULL)
             ORDER BY p.instance_id,p.operation_id",
        )
    }

    /// Retained journals whose task incarnation is gone, including already reused task IDs.
    pub fn orphan_ftp_publications(&self) -> Result<Vec<FtpPublicationRecord>, StorageError> {
        self.ftp_publication_records(
            "SELECT p.instance_id,p.operation_id,p.task_id,p.revision,p.phase,p.version,1
             FROM ftp_publications p LEFT JOIN task_origins o ON o.task_id=p.task_id AND o.operation_id=p.operation_id
                 AND o.status='active' AND p.instance_id=(SELECT identity FROM origin_instance WHERE singleton=1)
             LEFT JOIN tasks t ON t.id=o.task_id WHERE t.id IS NULL ORDER BY p.instance_id,p.operation_id",
        )
    }

    fn ftp_publication_records(
        &self,
        sql: &str,
    ) -> Result<Vec<FtpPublicationRecord>, StorageError> {
        let mut query = self.connection.prepare(sql).map_err(database_error)?;
        let mut rows = query.query([]).map_err(database_error)?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().map_err(database_error)? {
            if !row.get::<_, bool>(6).map_err(database_error)? {
                return Err(conflict());
            }
            let phase = match (
                row.get::<_, i64>(5).map_err(database_error)?,
                row.get::<_, String>(4).map_err(database_error)?.as_str(),
            ) {
                (1, "prepared") => FtpPublicationPhase::Prepared,
                (1, "completed") => FtpPublicationPhase::Completed,
                _ => return Err(conflict()),
            };
            result.push(FtpPublicationRecord {
                instance_id: row.get(0).map_err(database_error)?,
                operation_id: row.get(1).map_err(database_error)?,
                task_id: row.get(2).map_err(database_error)?,
                revision: row.get(3).map_err(database_error)?,
                phase,
            });
        }
        Ok(result)
    }
}
