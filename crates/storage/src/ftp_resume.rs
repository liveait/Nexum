//! Atomic FTP attempt claims and opaque, bounded Engine checkpoints.

use std::fmt;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{
    SqliteRepository, StorageError,
    ftp_publication::{self, FtpPublicationIntent, FtpPublicationPhase},
    origin::{self, OriginRecord},
};

const MAX_PROOF: usize = 64 * 1024;

#[derive(Clone, Eq, PartialEq)]
pub struct FtpCheckpoint {
    pub proof: Vec<u8>,
    pub trusted_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct FtpCompletion {
    pub proof: Vec<u8>,
    pub producing_generation: String,
    pub trusted_bytes: u64,
}

/// A snapshot for compare-and-swap. Reclaim only after the prior writer exits.
#[derive(Clone, Eq, PartialEq)]
pub struct FtpAttempt {
    pub instance_id: String,
    pub origin: OriginRecord,
    pub generation: String,
    pub revision: u64,
    pub checkpoint: Option<FtpCheckpoint>,
    pub completion: Option<FtpCompletion>,
    pub publication: Option<FtpPublicationIntent>,
}

impl fmt::Debug for FtpCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FtpCheckpoint")
            .field("trusted_bytes", &self.trusted_bytes)
            .field("total_bytes", &self.total_bytes)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for FtpCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FtpCompletion")
            .field("trusted_bytes", &self.trusted_bytes)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for FtpAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FtpAttempt")
            .field("revision", &self.revision)
            .field("checkpoint", &self.checkpoint)
            .field("completion", &self.completion)
            .field("publication", &self.publication)
            .finish_non_exhaustive()
    }
}

/// Holds the SQLite writer reservation until a caller publishes its decision.
/// Drop promptly; callers must not perform network or long-running work.
pub struct FtpAttemptGuard<'a> {
    pub(super) _transaction: rusqlite::Transaction<'a>,
}

impl FtpAttemptGuard<'_> {
    /// Ends this read-only reservation and reports SQLite release errors.
    pub fn release(self) -> Result<(), StorageError> {
        self._transaction.rollback().map_err(origin::database_error)
    }
}

pub(super) fn conflict() -> StorageError {
    StorageError::Other("FTP attempt conflict".into())
}
fn invalid() -> StorageError {
    StorageError::Other("invalid FTP checkpoint".into())
}

fn validate_checkpoint(checkpoint: &FtpCheckpoint) -> Result<(), StorageError> {
    if checkpoint.proof.is_empty()
        || checkpoint.proof.len() > MAX_PROOF
        || checkpoint.trusted_bytes > i64::MAX as u64
        || checkpoint
            .total_bytes
            .is_some_and(|total| total < checkpoint.trusted_bytes || total > i64::MAX as u64)
    {
        return Err(invalid());
    }
    Ok(())
}

fn verify_origin(connection: &Connection, record: &OriginRecord) -> Result<String, StorageError> {
    verify_origin_for_phase(connection, record, FtpPublicationPhase::Prepared)
}

pub(super) fn verify_origin_for_phase(
    connection: &Connection,
    record: &OriginRecord,
    phase: FtpPublicationPhase,
) -> Result<String, StorageError> {
    if record.version != 1
        || record.security != "plain"
        || record.authenticated != record.credential.is_some()
        || origin::read_record(connection, &record.task_id, "active")?.as_ref() != Some(record)
    {
        return Err(conflict());
    }
    let instance: String = connection
        .query_row(
            "SELECT identity FROM origin_instance WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(origin::database_error)?;
    if record.credential.as_ref().is_some_and(|credential| {
        credential.instance_id != instance
            || credential.origin != record.origin
            || credential.revision == 0
    }) {
        return Err(conflict());
    }
    let matches: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks t JOIN task_origins o ON o.task_id=t.id
         WHERE t.id=?1 AND t.source=?2 AND t.destination=?3 AND o.operation_id=?4
         AND (t.state='completed')=?5)",
            params![
                record.task_id,
                record.canonical_url,
                record.destination,
                record.operation_id,
                phase == FtpPublicationPhase::Completed,
            ],
            |row| row.get(0),
        )
        .map_err(origin::database_error)?;
    if !matches {
        return Err(conflict());
    }
    Ok(instance)
}

fn read_attempt(
    connection: &Connection,
    record: &OriginRecord,
    instance: &str,
) -> Result<Option<FtpAttempt>, StorageError> {
    read_attempt_for_phase(connection, record, instance, FtpPublicationPhase::Prepared)
}

pub(super) fn read_attempt_for_phase(
    connection: &Connection,
    record: &OriginRecord,
    instance: &str,
    phase: FtpPublicationPhase,
) -> Result<Option<FtpAttempt>, StorageError> {
    let encoded = serde_json::to_string(record).map_err(|_| invalid())?;
    let publication = ftp_publication::read_intent(connection, record, instance, phase)?;
    let row = connection.query_row(
        "SELECT version,instance_id,origin_record,generation,revision,
                proof,trusted_bytes,total_bytes,
                completion_version,completion_proof,completion_generation,completion_bytes,completion_seen,
                publication_seen,publication_authorization
         FROM ftp_attempts WHERE task_id=?1",
        [&record.task_id],
        |r| Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, u64>(4)?,
            r.get::<_, Option<Vec<u8>>>(5)?,
            r.get::<_, Option<u64>>(6)?,
            r.get::<_, Option<u64>>(7)?,
            r.get::<_, Option<i64>>(8)?,
            r.get::<_, Option<Vec<u8>>>(9)?,
            r.get::<_, Option<String>>(10)?,
            r.get::<_, Option<u64>>(11)?,
            r.get::<_, i64>(12)?,
            r.get::<_, i64>(13)?,
            r.get::<_, Option<String>>(14)?,
        ))
    ).optional().map_err(origin::database_error)?;
    if row.is_none() && publication.is_some() {
        return Err(conflict());
    }
    row.map(
        |(
            version,
            row_instance,
            row_origin,
            generation,
            revision,
            proof,
            trusted,
            total,
            completion_version,
            completion_proof,
            completion_generation,
            completion_bytes,
            completion_seen,
            publication_seen,
            publication_authorization,
        )| {
            if version != 1 || row_instance != instance || row_origin != encoded {
                return Err(conflict());
            }
            let checkpoint = match (proof, trusted) {
                (Some(proof), Some(trusted_bytes)) => {
                    let value = FtpCheckpoint {
                        proof,
                        trusted_bytes,
                        total_bytes: total,
                    };
                    validate_checkpoint(&value)?;
                    Some(value)
                }
                (None, None) if total.is_none() => None,
                _ => return Err(invalid()),
            };
            if generation.is_empty() || generation.len() > 256 {
                return Err(invalid());
            }
            let completion = match (
                completion_seen,
                completion_version,
                completion_proof,
                completion_generation,
                completion_bytes,
            ) {
                (0, None, None, None, None) => None,
                (1, Some(1), Some(proof), Some(producing_generation), Some(trusted_bytes))
                    if !proof.is_empty()
                        && proof.len() <= MAX_PROOF
                        && !producing_generation.is_empty()
                        && producing_generation.len() <= 256
                        && trusted_bytes <= i64::MAX as u64 =>
                {
                    Some(FtpCompletion {
                        proof,
                        producing_generation,
                        trusted_bytes,
                    })
                }
                _ => return Err(invalid()),
            };
            if let Some(ref completion) = completion {
                let progress: (u64, Option<u64>) = connection
                    .query_row(
                        "SELECT downloaded_bytes,total_bytes FROM tasks WHERE id=?1",
                        [&record.task_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(origin::database_error)?;
                if progress != (completion.trusted_bytes, Some(completion.trusted_bytes))
                    || checkpoint.as_ref().is_some_and(|p| {
                        p.trusted_bytes > completion.trusted_bytes
                            || p.total_bytes
                                .is_some_and(|total| total != completion.trusted_bytes)
                    })
                {
                    return Err(invalid());
                }
            }
            match (&publication, publication_seen, publication_authorization) {
                (None, 0, None) => {}
                (Some(intent), 1, Some(marker))
                    if marker == intent.authorization
                        && intent.revision == revision
                        && completion.is_some() => {}
                _ => return Err(conflict()),
            }
            Ok(FtpAttempt {
                instance_id: row_instance,
                origin: record.clone(),
                generation,
                revision,
                checkpoint,
                completion,
                publication,
            })
        },
    )
    .transpose()
}

impl SqliteRepository {
    /// Fence claims/removal while a previously read attempt is used to publish
    /// a synchronous in-memory decision. This does not authorize future I/O.
    pub fn guard_ftp_attempt(
        &self,
        expected: &FtpAttempt,
    ) -> Result<FtpAttemptGuard<'_>, StorageError> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
                .map_err(origin::database_error)?;
        let instance = verify_origin(&transaction, &expected.origin)?;
        if read_attempt(&transaction, &expected.origin, &instance)?.as_ref() != Some(expected) {
            return Err(conflict());
        }
        Ok(FtpAttemptGuard {
            _transaction: transaction,
        })
    }
    pub(crate) fn migrate_ftp_completion(&mut self) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(origin::database_error)?;
        let version: i64 = transaction
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .map_err(origin::database_error)?;
        if (5..=6).contains(&version) {
            return transaction.commit().map_err(origin::database_error);
        }
        if version != 4 {
            return Err(invalid());
        }
        transaction
            .execute_batch(
                "ALTER TABLE ftp_attempts ADD COLUMN completion_seen INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE ftp_attempts ADD COLUMN completion_version INTEGER;
            ALTER TABLE ftp_attempts ADD COLUMN completion_proof BLOB;
            ALTER TABLE ftp_attempts ADD COLUMN completion_generation TEXT;
            ALTER TABLE ftp_attempts ADD COLUMN completion_bytes INTEGER;
            UPDATE schema_version SET version=5;",
            )
            .map_err(origin::database_error)?;
        transaction.commit().map_err(origin::database_error)
    }
    pub(crate) fn migrate_ftp_resume(&mut self) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(origin::database_error)?;
        let version: i64 = transaction
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .map_err(origin::database_error)?;
        if version >= 4 {
            return transaction.commit().map_err(origin::database_error);
        }
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS ftp_attempts(
                task_id TEXT PRIMARY KEY,
                version INTEGER NOT NULL CHECK(version=1),
                instance_id TEXT NOT NULL,
                origin_record TEXT NOT NULL,
                generation TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision>=0),
                proof BLOB,
                trusted_bytes INTEGER,
                total_bytes INTEGER,
                CHECK(proof IS NULL OR (length(proof)>0 AND length(proof)<=65536)),
                CHECK((proof IS NULL AND trusted_bytes IS NULL AND total_bytes IS NULL) OR
                      (proof IS NOT NULL AND trusted_bytes>=0 AND (total_bytes IS NULL OR total_bytes>=trusted_bytes)))
            );
            UPDATE schema_version SET version=4;"
        ).map_err(origin::database_error)?;
        transaction.commit().map_err(origin::database_error)
    }

    pub fn ftp_attempt(&self, record: &OriginRecord) -> Result<Option<FtpAttempt>, StorageError> {
        // A read transaction binds the origin and attempt to one snapshot.
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(origin::database_error)?;
        let instance = verify_origin(&transaction, record)?;
        let attempt = read_attempt(&transaction, record, &instance)?;
        transaction.commit().map_err(origin::database_error)?;
        Ok(attempt)
    }

    /// Reserves database commit ownership, preserving the old proof unchanged.
    /// This does not acquire a file writer or a scheduler slot.
    pub fn claim_ftp_attempt(
        &mut self,
        record: &OriginRecord,
        expected: Option<&FtpAttempt>,
    ) -> Result<FtpAttempt, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(origin::database_error)?;
        let instance = verify_origin(&transaction, record)?;
        let current = read_attempt(&transaction, record, &instance)?;
        if current.as_ref() != expected {
            return Err(conflict());
        }
        let generation = origin::random_id()?;
        let encoded = serde_json::to_string(record).map_err(|_| invalid())?;
        let changed = transaction.execute(
            "INSERT INTO ftp_attempts(task_id,version,instance_id,origin_record,generation,revision)
             VALUES(?1,1,?2,?3,?4,0)
             ON CONFLICT(task_id) DO UPDATE SET generation=excluded.generation",
            params![record.task_id,instance,encoded,generation]
        ).map_err(origin::database_error)?;
        if changed != 1 {
            return Err(conflict());
        }
        let claimed = match current {
            Some(previous) => FtpAttempt {
                generation,
                ..previous
            },
            None => FtpAttempt {
                instance_id: instance.clone(),
                origin: record.clone(),
                generation,
                revision: 0,
                checkpoint: None,
                completion: None,
                publication: None,
            },
        };
        let after = verify_origin(&transaction, record)?;
        if read_attempt(&transaction, record, &after)?.as_ref() != Some(&claimed) {
            return Err(conflict());
        }
        transaction.commit().map_err(origin::database_error)?;
        Ok(claimed)
    }

    /// Commits proof and normalized task progress in the same transaction.
    /// The Engine-aware consumer validates the opaque proof's metadata first.
    pub fn save_ftp_checkpoint(
        &mut self,
        attempt: &FtpAttempt,
        checkpoint: &FtpCheckpoint,
    ) -> Result<FtpAttempt, StorageError> {
        validate_checkpoint(checkpoint)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(origin::database_error)?;
        let instance = verify_origin(&transaction, &attempt.origin)?;
        let current = read_attempt(&transaction, &attempt.origin, &instance)?;
        if current.as_ref() != Some(attempt) {
            return Err(conflict());
        }
        if attempt.completion.is_some() {
            return Err(conflict());
        }
        if attempt
            .checkpoint
            .as_ref()
            .is_some_and(|previous| checkpoint.trusted_bytes < previous.trusted_bytes)
        {
            return Err(conflict());
        }
        let next = attempt
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= i64::MAX as u64)
            .ok_or_else(invalid)?;
        let changed = transaction
            .execute(
                "UPDATE ftp_attempts SET revision=?2,proof=?3,trusted_bytes=?4,total_bytes=?5
             WHERE task_id=?1 AND generation=?6 AND revision=?7",
                params![
                    attempt.origin.task_id,
                    next,
                    checkpoint.proof,
                    checkpoint.trusted_bytes,
                    checkpoint.total_bytes,
                    attempt.generation,
                    attempt.revision
                ],
            )
            .map_err(origin::database_error)?;
        if changed != 1 {
            return Err(conflict());
        }
        let changed=transaction.execute(
            "UPDATE tasks SET downloaded_bytes=?2,total_bytes=?3,speed_bytes_per_second=0,eta_seconds=NULL WHERE id=?1",
            params![attempt.origin.task_id,checkpoint.trusted_bytes,checkpoint.total_bytes]
        ).map_err(origin::database_error)?;
        if changed != 1 {
            return Err(conflict());
        }
        let saved = read_attempt(&transaction, &attempt.origin, &instance)?.ok_or_else(conflict)?;
        transaction.commit().map_err(origin::database_error)?;
        Ok(saved)
    }

    /// Atomically records opaque completion evidence and complete-file progress.
    pub fn save_ftp_completion(
        &mut self,
        attempt: &FtpAttempt,
        completion: &FtpCompletion,
    ) -> Result<FtpAttempt, StorageError> {
        if completion.proof.is_empty()
            || completion.proof.len() > MAX_PROOF
            || completion.producing_generation != attempt.generation
            || completion.trusted_bytes > i64::MAX as u64
            || attempt.checkpoint.as_ref().is_some_and(|p| {
                p.trusted_bytes > completion.trusted_bytes
                    || p.total_bytes
                        .is_some_and(|total| total != completion.trusted_bytes)
            })
        {
            return Err(invalid());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(origin::database_error)?;
        let instance = verify_origin(&transaction, &attempt.origin)?;
        let current = read_attempt(&transaction, &attempt.origin, &instance)?;
        if current.as_ref() != Some(attempt) || attempt.completion.is_some() {
            return Err(conflict());
        }
        let next = attempt
            .revision
            .checked_add(1)
            .filter(|r| *r <= i64::MAX as u64)
            .ok_or_else(invalid)?;
        let changed=transaction.execute("UPDATE ftp_attempts SET revision=?2,completion_seen=1,completion_version=1,completion_proof=?3,completion_generation=?4,completion_bytes=?5 WHERE task_id=?1 AND generation=?6 AND revision=?7",params![attempt.origin.task_id,next,completion.proof,completion.producing_generation,completion.trusted_bytes,attempt.generation,attempt.revision]).map_err(origin::database_error)?;
        if changed != 1 {
            return Err(conflict());
        }
        let changed=transaction.execute("UPDATE tasks SET downloaded_bytes=?2,total_bytes=?2,speed_bytes_per_second=0,eta_seconds=NULL WHERE id=?1",params![attempt.origin.task_id,completion.trusted_bytes]).map_err(origin::database_error)?;
        if changed != 1 {
            return Err(conflict());
        }
        let saved = read_attempt(&transaction, &attempt.origin, &instance)?.ok_or_else(conflict)?;
        transaction.commit().map_err(origin::database_error)?;
        Ok(saved)
    }
}

#[cfg(test)]
mod completion_guard_tests {
    use super::*;
    #[test]
    fn actual_sqlite_rollback_failure_is_reported() {
        let connection = Connection::open_in_memory().unwrap();
        let transaction =
            rusqlite::Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
                .unwrap();
        transaction.execute_batch("ROLLBACK").unwrap();
        let guard = FtpAttemptGuard {
            _transaction: transaction,
        };
        assert!(guard.release().is_err());
    }
}
