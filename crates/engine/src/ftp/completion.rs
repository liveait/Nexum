//! Complete FTP responses, proven through caller-owned handles before publication.

use serde::{Deserialize, Serialize};

use super::*;
use crate::file_identity::FileIdentity;
use resume::{digest, held_identity, prepare_resume, validate_binding};

#[path = "publication.rs"]
mod publication;
pub use publication::{
    FtpPublicationError, FtpPublicationPlan, FtpPublicationRequest, FtpPublishedCompletion,
    publish_completion_with_guard, reconcile_publication_with_guard,
};

const FORMAT: &str = "ftp-held-completion/v1";
const MAX_PROOF_BYTES: usize = 64 * 1024;

/// Origin and exact producing generation obtained independently of the proof.
pub struct FtpCompletionRequest<'a> {
    pub binding: &'a FtpResumeBinding,
    pub attempt_generation: &'a str,
}

/// Evidence of a positive terminal FTP response and a synced complete held file.
/// Persistence, namespace ownership and publication remain caller responsibilities.
#[derive(Clone)]
pub struct FtpCompletionProof {
    inner: CompletionData,
}

#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CompletionData {
    format: String,
    binding: FtpResumeBinding,
    attempt_generation: String,
    parent_identity: FileIdentity,
    file_identity: FileIdentity,
    length: u64,
    sha256: [u8; 32],
}

impl fmt::Debug for FtpCompletionProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FtpCompletionProof")
            .finish_non_exhaustive()
    }
}

impl FtpCompletionProof {
    pub fn binding(&self) -> &FtpResumeBinding {
        &self.inner.binding
    }

    pub fn attempt_generation(&self) -> &str {
        &self.inner.attempt_generation
    }

    pub fn progress(&self) -> Progress {
        Progress::new(self.inner.length, Some(self.inner.length))
    }

    pub fn encode(&self) -> Result<Vec<u8>, FtpReceiveError> {
        self.validate()?;
        let bytes = serde_json::to_vec(&self.inner).map_err(|_| FtpReceiveError::InvalidProof)?;
        if bytes.len() > MAX_PROOF_BYTES {
            return Err(FtpReceiveError::InvalidProof);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, FtpReceiveError> {
        if bytes.len() > MAX_PROOF_BYTES {
            return Err(FtpReceiveError::InvalidProof);
        }
        let proof = Self {
            inner: serde_json::from_slice(bytes).map_err(|_| FtpReceiveError::InvalidProof)?,
        };
        proof.validate()?;
        Ok(proof)
    }

    fn validate(&self) -> Result<(), FtpReceiveError> {
        validate_binding(&self.inner.binding)?;
        if self.inner.format != FORMAT || !valid_arg(&self.inner.attempt_generation, 256, false) {
            return Err(FtpReceiveError::InvalidProof);
        }
        Ok(())
    }

    fn matches(&self, file: &File, directory: &File) -> Result<(), FtpReceiveError> {
        if held_identity(file, false)? != self.inner.file_identity
            || held_identity(directory, true)? != self.inner.parent_identity
        {
            return Err(FtpReceiveError::FileIdentityMismatch);
        }
        if file.metadata().map_err(local_io)?.len() != self.inner.length {
            return Err(FtpReceiveError::LocalPrefixMismatch);
        }
        Ok(())
    }
}

fn prepare(
    file: &File,
    directory: &File,
    request: FtpCompletionRequest<'_>,
) -> Result<FtpCompletionProof, FtpReceiveError> {
    let proof = FtpCompletionProof {
        inner: CompletionData {
            format: FORMAT.into(),
            binding: request.binding.clone(),
            attempt_generation: request.attempt_generation.into(),
            parent_identity: held_identity(directory, true)?,
            file_identity: held_identity(file, false)?,
            length: 0,
            sha256: [0; 32],
        },
    };
    proof.encode()?;
    Ok(proof)
}

fn validate_credentials(
    binding: &FtpResumeBinding,
    credentials: &FtpCredentials<'_>,
) -> Result<(), FtpReceiveError> {
    if binding.credential.is_some() != matches!(credentials, FtpCredentials::Password { .. }) {
        return Err(FtpReceiveError::ResumeBindingMismatch);
    }
    Ok(())
}

/// Receive, validate the terminal reply, sync and hash without opening a path.
/// The caller owns all writes and joins this worker before reusing its handles.
/// Pause/cancel stay active throughout hashing. A proof does not publish a file.
pub fn receive_complete(
    endpoint: SocketAddr,
    credentials: FtpCredentials<'_>,
    file: &mut File,
    directory: &File,
    request: FtpCompletionRequest<'_>,
    control: &FtpTransferControl,
) -> Result<FtpCompletionProof, FtpReceiveError> {
    let _finish = begin(control)?;
    control.checkpoint()?;
    let proof = prepare(file, directory, request)?;
    validate_credentials(&proof.inner.binding, &credentials)?;
    if file.metadata().map_err(local_io)?.len() != 0
        || file.stream_position().map_err(local_io)? != 0
    {
        return Err(FtpReceiveError::InvalidTarget);
    }
    let progress = transfer_data(
        endpoint,
        &proof.inner.binding.command_path,
        credentials,
        file,
        control,
        0,
        None,
    )?;
    seal(
        file,
        directory,
        proof,
        progress,
        control,
        digest,
        File::sync_all,
    )
}

/// Resume from a prefix proof and produce distinct terminal-response evidence.
pub fn resume_complete(
    endpoint: SocketAddr,
    credentials: FtpCredentials<'_>,
    file: &mut File,
    directory: &File,
    request: FtpResumeRequest<'_>,
    control: &FtpTransferControl,
) -> Result<FtpCompletionProof, FtpReceiveError> {
    let _finish = begin(control)?;
    control.checkpoint()?;
    let proof = prepare(
        file,
        directory,
        FtpCompletionRequest {
            binding: request.binding,
            attempt_generation: request.attempt_generation,
        },
    )?;
    prepare_resume(file, directory, &request, &credentials, control)?;
    let progress = transfer_data(
        endpoint,
        &request.binding.command_path,
        credentials,
        file,
        control,
        request.proof.progress().downloaded_bytes,
        request.proof.progress().total_bytes,
    )?;
    seal(
        file,
        directory,
        proof,
        progress,
        control,
        digest,
        File::sync_all,
    )
}

fn seal(
    file: &mut File,
    directory: &File,
    mut proof: FtpCompletionProof,
    progress: Progress,
    control: &FtpTransferControl,
    hash: impl FnOnce(&mut File, u64, &FtpTransferControl) -> Result<[u8; 32], FtpReceiveError>,
    sync: impl FnOnce(&File) -> std::io::Result<()>,
) -> Result<FtpCompletionProof, FtpReceiveError> {
    if progress.total_bytes != Some(progress.downloaded_bytes) {
        return Err(FtpReceiveError::InvalidProof);
    }
    proof.inner.length = progress.downloaded_bytes;
    proof.matches(file, directory)?;
    proof.inner.sha256 = hash(file, proof.inner.length, control)?;
    sync(file).map_err(local_io)?;
    control.checkpoint()?;
    proof.matches(file, directory)?;
    proof.encode()?;
    control.finish_success()?;
    Ok(proof)
}

/// Read-only, offline verification against an independently loaded origin and
/// exact producing generation. Reclaiming a generation needs separate retained-
/// proof authorization. This verifies held objects, not their former pathnames.
pub fn verify_completion(
    file: &mut File,
    directory: &File,
    request: FtpCompletionRequest<'_>,
    proof: &FtpCompletionProof,
    control: &FtpTransferControl,
) -> Result<Progress, FtpReceiveError> {
    verify_with(file, directory, request, proof, control, digest)
}

/// A held read-only reservation that reports errors when released.
pub trait FtpCompletionGuard {
    fn release(self) -> Result<(), FtpReceiveError>;
}
impl FtpCompletionGuard for () {
    fn release(self) -> Result<(), FtpReceiveError> {
        Ok(())
    }
}

/// Verify with a bounded local snapshot fence held through the finished
/// decision. It publishes no control progress and returns point-in-time
/// evidence only after successful release, not future I/O authorization.
/// Never perform network, reentrant Core calls, or long work in the guard.
pub fn verify_completion_with_guard<G: FtpCompletionGuard>(
    file: &mut File,
    directory: &File,
    request: FtpCompletionRequest<'_>,
    proof: &FtpCompletionProof,
    control: &FtpTransferControl,
    guard: impl FnOnce() -> Result<G, FtpReceiveError>,
) -> Result<Progress, FtpReceiveError> {
    verify_with_guard(
        file,
        directory,
        request,
        proof,
        control,
        digest,
        (guard, false),
    )
}

fn verify_with(
    file: &mut File,
    directory: &File,
    request: FtpCompletionRequest<'_>,
    proof: &FtpCompletionProof,
    control: &FtpTransferControl,
    hash: impl FnOnce(&mut File, u64, &FtpTransferControl) -> Result<[u8; 32], FtpReceiveError>,
) -> Result<Progress, FtpReceiveError> {
    verify_with_guard(
        file,
        directory,
        request,
        proof,
        control,
        hash,
        (|| Ok(()), true),
    )
}

fn verify_with_guard<G: FtpCompletionGuard>(
    file: &mut File,
    directory: &File,
    request: FtpCompletionRequest<'_>,
    proof: &FtpCompletionProof,
    control: &FtpTransferControl,
    hash: impl FnOnce(&mut File, u64, &FtpTransferControl) -> Result<[u8; 32], FtpReceiveError>,
    guard: (impl FnOnce() -> Result<G, FtpReceiveError>, bool),
) -> Result<Progress, FtpReceiveError> {
    let _finish = begin(control)?;
    control.checkpoint()?;
    proof.validate()?;
    validate_binding(request.binding)?;
    if &proof.inner.binding != request.binding
        || proof.inner.attempt_generation != request.attempt_generation
    {
        return Err(FtpReceiveError::ResumeBindingMismatch);
    }
    proof.matches(file, directory)?;
    if hash(file, proof.inner.length, control)? != proof.inner.sha256 {
        return Err(FtpReceiveError::LocalPrefixMismatch);
    }
    proof.matches(file, directory)?;
    control.finish_success_checked(
        if guard.1 {
            Some(proof.progress())
        } else {
            None
        },
        guard.0,
    )?;
    Ok(proof.progress())
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
