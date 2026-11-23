//! Engine-aware FTP checkpoint persistence. File writers remain caller-owned.

use nexum_domain::TaskId;
use nexum_engine::{
    EngineError,
    ftp::{
        self, FtpCompletionProof, FtpCompletionRequest, FtpCredentialBinding, FtpPathMode,
        FtpPrefixProof, FtpResumeBinding, FtpTransferControl,
    },
};
use nexum_storage::{
    SqliteRepository, StorageError,
    ftp_resume::{FtpAttempt, FtpCheckpoint, FtpCompletion},
    origin::OriginRecord,
};

use crate::{Core, CoreError};

pub(super) struct CompletionGuard<'a> {
    inner: nexum_storage::ftp_resume::FtpAttemptGuard<'a>,
    failure: &'a std::cell::RefCell<Option<StorageError>>,
}
impl<'a> CompletionGuard<'a> {
    pub(super) fn capture(
        guard: Result<nexum_storage::ftp_resume::FtpAttemptGuard<'a>, StorageError>,
        failure: &'a std::cell::RefCell<Option<StorageError>>,
    ) -> Result<Self, ftp::FtpReceiveError> {
        guard.map(|inner| Self { inner, failure }).map_err(|error| {
            failure.replace(Some(error));
            ftp::FtpReceiveError::ResumeBindingMismatch
        })
    }
}
impl ftp::FtpCompletionGuard for CompletionGuard<'_> {
    fn release(self) -> Result<(), ftp::FtpReceiveError> {
        self.inner.release().map_err(|error| {
            self.failure.replace(Some(error));
            ftp::FtpReceiveError::ResumeBindingMismatch
        })
    }
}

#[derive(Clone)]
pub struct FtpResumeAttempt {
    pub(super) stored: FtpAttempt,
    binding: FtpResumeBinding,
    proof: Option<FtpPrefixProof>,
    completion: Option<FtpCompletionProof>,
}

impl std::fmt::Debug for FtpResumeAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpResumeAttempt")
            .field("revision", &self.revision())
            .finish_non_exhaustive()
    }
}

impl FtpResumeAttempt {
    pub fn generation(&self) -> &str {
        &self.stored.generation
    }
    pub fn revision(&self) -> u64 {
        self.stored.revision
    }
    pub fn binding(&self) -> &FtpResumeBinding {
        &self.binding
    }
    pub fn proof(&self) -> Option<&FtpPrefixProof> {
        self.proof.as_ref()
    }
    pub fn completion(&self) -> Option<&FtpCompletionProof> {
        self.completion.as_ref()
    }
    pub fn completion_generation(&self) -> Option<&str> {
        self.stored
            .completion
            .as_ref()
            .map(|c| c.producing_generation.as_str())
    }
}

fn invalid() -> CoreError {
    CoreError::Engine(EngineError::Failed("invalid FTP checkpoint binding".into()))
}

fn binding(origin: &OriginRecord, instance: &str) -> Result<FtpResumeBinding, CoreError> {
    if origin.version != 1
        || origin.security != "plain"
        || origin.authenticated != origin.credential.is_some()
    {
        return Err(invalid());
    }
    let path_mode = match origin.path_mode.as_str() {
        "login_relative" => FtpPathMode::LoginRelative,
        "absolute" => FtpPathMode::Absolute,
        _ => return Err(invalid()),
    };
    if origin.credential.as_ref().is_some_and(|credential| {
        credential.instance_id != instance
            || credential.origin != origin.origin
            || credential.revision == 0
    }) {
        return Err(invalid());
    }
    Ok(FtpResumeBinding {
        server_instance_id: instance.into(),
        task_id: origin.task_id.clone(),
        operation_id: origin.operation_id.clone(),
        source_origin: origin.origin.clone(),
        command_path: origin.command_path.clone(),
        path_mode,
        destination: origin.destination.clone(),
        credential: origin
            .credential
            .as_ref()
            .map(|credential| FtpCredentialBinding {
                credential_id: credential.credential_id.clone(),
                revision: credential.revision,
            }),
    })
}

pub(super) fn typed(stored: FtpAttempt) -> Result<FtpResumeAttempt, CoreError> {
    let binding = binding(&stored.origin, &stored.instance_id)?;
    let proof = stored
        .checkpoint
        .as_ref()
        .map(|checkpoint| {
            let proof = FtpPrefixProof::decode(&checkpoint.proof).map_err(|_| invalid())?;
            if proof.binding() != &binding
                || proof.progress().downloaded_bytes != checkpoint.trusted_bytes
                || proof.progress().total_bytes != checkpoint.total_bytes
            {
                return Err(invalid());
            }
            Ok(proof)
        })
        .transpose()?;
    let completion = stored
        .completion
        .as_ref()
        .map(|record| {
            let proof = FtpCompletionProof::decode(&record.proof).map_err(|_| invalid())?;
            if proof.binding() != &binding
                || proof.attempt_generation() != record.producing_generation
                || proof.progress().downloaded_bytes != record.trusted_bytes
                || proof.progress().total_bytes != Some(record.trusted_bytes)
            {
                return Err(invalid());
            }
            Ok(proof)
        })
        .transpose()?;
    Ok(FtpResumeAttempt {
        stored,
        binding,
        proof,
        completion,
    })
}

impl Core<SqliteRepository> {
    pub fn load_ftp_attempt(&self, id: &TaskId) -> Result<Option<FtpResumeAttempt>, CoreError> {
        let origin = self.repository.active_origin(id)?.ok_or_else(invalid)?;
        self.repository.ftp_attempt(&origin)?.map(typed).transpose()
    }

    /// Claims database commit ownership only. The caller must hold the Server
    /// data-directory lock, stop/join any old writer and serialize file access.
    pub fn claim_ftp_attempt(
        &mut self,
        id: &TaskId,
        expected: Option<&FtpResumeAttempt>,
    ) -> Result<FtpResumeAttempt, CoreError> {
        let origin = self.repository.active_origin(id)?.ok_or_else(invalid)?;
        let instance = self.repository.instance_id()?;
        let binding = binding(&origin, &instance)?;
        if self.tasks.get(id).is_none() {
            return Err(StorageError::NotFound(id.clone()).into());
        }
        let stored = self
            .repository
            .claim_ftp_attempt(&origin, expected.map(|attempt| &attempt.stored))?;
        Ok(FtpResumeAttempt {
            stored,
            binding,
            proof: expected.and_then(|attempt| attempt.proof.clone()),
            completion: expected.and_then(|attempt| attempt.completion.clone()),
        })
    }

    /// The Engine must have synced the proof's held file after its writer was
    /// joined. The transaction fences this commit and atomically saves proof
    /// and task progress before any in-memory task/event change is published.
    pub fn save_ftp_checkpoint(
        &mut self,
        attempt: &FtpResumeAttempt,
        proof: &FtpPrefixProof,
    ) -> Result<FtpResumeAttempt, CoreError> {
        if proof.binding() != attempt.binding()
            || proof.attempt_generation() != attempt.generation()
        {
            return Err(invalid());
        }
        let id = TaskId::from(attempt.stored.origin.task_id.clone());
        let mut tasks = self.tasks.clone();
        tasks.update_progress(&id, proof.progress())?;
        let progress = proof.progress();
        let checkpoint = FtpCheckpoint {
            proof: proof.encode().map_err(|_| invalid())?,
            trusted_bytes: progress.downloaded_bytes,
            total_bytes: progress.total_bytes,
        };
        let saved = self
            .repository
            .save_ftp_checkpoint(&attempt.stored, &checkpoint)?;
        self.tasks = tasks;
        Ok(FtpResumeAttempt {
            stored: saved,
            binding: attempt.binding.clone(),
            proof: Some(proof.clone()),
            completion: None,
        })
    }

    /// The producer has completed and synced its held file; this commits proof
    /// and progress only, and grants no pathname or publication ownership.
    pub fn save_ftp_completion(
        &mut self,
        attempt: &FtpResumeAttempt,
        proof: &FtpCompletionProof,
    ) -> Result<FtpResumeAttempt, CoreError> {
        if proof.binding() != attempt.binding()
            || proof.attempt_generation() != attempt.generation()
        {
            return Err(invalid());
        }
        let id = TaskId::from(attempt.stored.origin.task_id.clone());
        let mut tasks = self.tasks.clone();
        tasks.update_progress(&id, proof.progress())?;
        let record = FtpCompletion {
            proof: proof.encode().map_err(|_| invalid())?,
            producing_generation: attempt.generation().to_owned(),
            trusted_bytes: proof.progress().downloaded_bytes,
        };
        let saved = self
            .repository
            .save_ftp_completion(&attempt.stored, &record)?;
        let saved = typed(saved)?;
        self.tasks = tasks;
        Ok(saved)
    }

    /// Caller owns and joins all writers and retains namespace reservations.
    /// This returns verification evidence, not an authorization to publish.
    pub fn verify_ftp_completion(
        &self,
        attempt: &FtpResumeAttempt,
        file: &mut std::fs::File,
        directory: &std::fs::File,
        control: &FtpTransferControl,
    ) -> Result<nexum_domain::Progress, CoreError> {
        let id = TaskId::from(attempt.stored.origin.task_id.clone());
        let current = self.repository.ftp_attempt(&attempt.stored.origin)?;
        if current.as_ref() != Some(&attempt.stored) || self.tasks.get(&id).is_none() {
            return Err(invalid());
        }
        let proof = attempt.completion.as_ref().ok_or_else(invalid)?;
        let producing_generation = attempt.completion_generation().ok_or_else(invalid)?;
        let failure = std::cell::RefCell::new(None);
        let result = ftp::verify_completion_with_guard(
            file,
            directory,
            FtpCompletionRequest {
                binding: attempt.binding(),
                attempt_generation: producing_generation,
            },
            proof,
            control,
            || {
                CompletionGuard::capture(
                    self.repository.guard_ftp_attempt(&attempt.stored),
                    &failure,
                )
            },
        );
        if let Some(error) = failure.into_inner() {
            return Err(error.into());
        }
        result.map_err(|error| match error {
            ftp::FtpReceiveError::Cancelled => CoreError::Engine(EngineError::Cancelled),
            _ => CoreError::Engine(EngineError::Failed(error.to_string())),
        })
    }
}
