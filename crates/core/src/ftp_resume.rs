//! Engine-aware FTP checkpoint persistence. File writers remain caller-owned.

use nexum_domain::TaskId;
use nexum_engine::{
    EngineError,
    ftp::{FtpCredentialBinding, FtpPathMode, FtpPrefixProof, FtpResumeBinding},
};
use nexum_storage::{
    SqliteRepository, StorageError,
    ftp_resume::{FtpAttempt, FtpCheckpoint},
    origin::OriginRecord,
};

use crate::{Core, CoreError};

#[derive(Clone, Debug)]
pub struct FtpResumeAttempt {
    stored: FtpAttempt,
    binding: FtpResumeBinding,
    proof: Option<FtpPrefixProof>,
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

fn typed(stored: FtpAttempt) -> Result<FtpResumeAttempt, CoreError> {
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
    Ok(FtpResumeAttempt {
        stored,
        binding,
        proof,
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
        })
    }
}
