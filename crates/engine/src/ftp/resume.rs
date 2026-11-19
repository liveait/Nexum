//! Versioned prefix proofs for a caller-owned, exclusively accessed FTP file.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::SocketAddr;

use nexum_domain::Progress;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    FtpCredentials, FtpReceiveError, FtpTransferControl, begin, local_io, transfer, valid_arg,
};
use crate::file_identity::{FileIdentity, identity};

const MAX_PROOF_BYTES: usize = 64 * 1024;
const FORMAT: &str = "ftp-held-prefix/v1";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FtpPathMode {
    LoginRelative,
    Absolute,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FtpCredentialBinding {
    pub credential_id: String,
    pub revision: u64,
}

/// Secret-free origin and task incarnation selected by the persistence owner.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FtpResumeBinding {
    pub server_instance_id: String,
    pub task_id: String,
    pub operation_id: String,
    pub source_origin: String,
    pub command_path: String,
    pub path_mode: FtpPathMode,
    pub destination: String,
    pub credential: Option<FtpCredentialBinding>,
}

pub struct FtpCheckpointRequest<'a> {
    pub binding: &'a FtpResumeBinding,
    pub attempt_generation: &'a str,
    pub trusted_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// The caller atomically claims a fresh generation and supplies the current
/// binding independently of the stored proof. Serialize attempts for the file.
pub struct FtpResumeRequest<'a> {
    pub binding: &'a FtpResumeBinding,
    pub attempt_generation: &'a str,
    pub proof: &'a FtpPrefixProof,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FtpPrefixProof {
    format: String,
    binding: FtpResumeBinding,
    attempt_generation: String,
    parent_identity: FileIdentity,
    partial_identity: FileIdentity,
    trusted_bytes: u64,
    prefix_sha256: [u8; 32],
    total_bytes: Option<u64>,
}

impl FtpPrefixProof {
    pub fn encode(&self) -> Result<Vec<u8>, FtpReceiveError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| FtpReceiveError::InvalidProof)?;
        if bytes.len() > MAX_PROOF_BYTES {
            return Err(FtpReceiveError::InvalidProof);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, FtpReceiveError> {
        if bytes.len() > MAX_PROOF_BYTES {
            return Err(FtpReceiveError::InvalidProof);
        }
        let proof: Self =
            serde_json::from_slice(bytes).map_err(|_| FtpReceiveError::InvalidProof)?;
        proof.validate()?;
        Ok(proof)
    }

    fn validate(&self) -> Result<(), FtpReceiveError> {
        validate_binding(&self.binding)?;
        if self.format != FORMAT
            || !valid_arg(&self.attempt_generation, 256, false)
            || self
                .total_bytes
                .is_some_and(|total| total < self.trusted_bytes)
        {
            return Err(FtpReceiveError::InvalidProof);
        }
        Ok(())
    }
}

fn validate_binding(binding: &FtpResumeBinding) -> Result<(), FtpReceiveError> {
    let valid = valid_arg(&binding.server_instance_id, 256, false)
        && valid_arg(&binding.task_id, 256, false)
        && valid_arg(&binding.operation_id, 256, false)
        && valid_arg(&binding.command_path, 32768, false)
        && valid_arg(&binding.destination, 32768, false)
        && valid_arg(&binding.source_origin, 4096, false)
        && binding.credential.as_ref().is_none_or(|credential| {
            valid_arg(&credential.credential_id, 256, false) && credential.revision > 0
        })
        && match binding.path_mode {
            FtpPathMode::LoginRelative => !binding.command_path.starts_with('/'),
            FtpPathMode::Absolute => binding.command_path.starts_with('/'),
        };
    let origin =
        reqwest::Url::parse(&binding.source_origin).map_err(|_| FtpReceiveError::InvalidProof)?;
    if !valid
        || origin.scheme() != "ftp"
        || origin.host_str().is_none()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
        || !matches!(origin.path(), "" | "/")
        || origin.port() == Some(0)
        || binding.source_origin.contains(['@', '%', '\\'])
    {
        return Err(FtpReceiveError::InvalidProof);
    }
    Ok(())
}

fn held_identity(file: &File, directory: bool) -> Result<FileIdentity, FtpReceiveError> {
    let metadata = file.metadata().map_err(local_io)?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(FtpReceiveError::InvalidTarget);
    }
    identity(file, &metadata).map_err(|_| FtpReceiveError::FileIdentityMismatch)
}

fn exact_length(file: &File, trusted: u64) -> Result<(), FtpReceiveError> {
    let length = file.metadata().map_err(local_io)?.len();
    if length > trusted {
        return Err(FtpReceiveError::UncommittedTail);
    }
    if length < trusted {
        return Err(FtpReceiveError::LocalPrefixMismatch);
    }
    Ok(())
}

fn digest(
    file: &mut (impl Read + Seek),
    bytes: u64,
    control: &FtpTransferControl,
) -> Result<[u8; 32], FtpReceiveError> {
    file.seek(SeekFrom::Start(0)).map_err(local_io)?;
    let mut remaining = bytes;
    let mut sha = Sha256::new();
    let mut buffer = [0u8; 32 * 1024];
    while remaining != 0 {
        control.checkpoint()?;
        let count = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..count]).map_err(local_io)?;
        control.checkpoint()?;
        sha.update(&buffer[..count]);
        remaining -= count as u64;
    }
    control.checkpoint()?;
    file.seek(SeekFrom::Start(0)).map_err(local_io)?;
    Ok(sha.finalize().into())
}

/// Creates a proof after the caller has joined the transfer worker. The caller
/// supplies the verified logical length, atomically persists encode() output
/// after this succeeds and owns the journal's directory-entry safety.
pub fn checkpoint(
    request: FtpCheckpointRequest<'_>,
    file: &mut File,
    directory: &File,
    control: &FtpTransferControl,
) -> Result<FtpPrefixProof, FtpReceiveError> {
    let _finish = begin(control)?;
    control.checkpoint()?;
    validate_binding(request.binding)?;
    if !valid_arg(request.attempt_generation, 256, false) {
        return Err(FtpReceiveError::InvalidProof);
    }
    let partial_identity = held_identity(file, false)?;
    let parent_identity = held_identity(directory, true)?;
    exact_length(file, request.trusted_bytes)?;
    let prefix_sha256 = digest(file, request.trusted_bytes, control)?;
    file.sync_all().map_err(local_io)?;
    exact_length(file, request.trusted_bytes)?;
    if held_identity(file, false)? != partial_identity {
        return Err(FtpReceiveError::FileIdentityMismatch);
    }
    let proof = FtpPrefixProof {
        format: FORMAT.into(),
        binding: request.binding.clone(),
        attempt_generation: request.attempt_generation.into(),
        parent_identity,
        partial_identity,
        trusted_bytes: request.trusted_bytes,
        prefix_sha256,
        total_bytes: request.total_bytes,
    };
    proof.encode()?;
    control.finish_success()?;
    Ok(proof)
}

/// Verifies local proof, then compares the prefix of one zero-offset RETR
/// before appending. No path is opened, truncated, removed or published.
pub fn resume(
    endpoint: SocketAddr,
    credentials: FtpCredentials<'_>,
    file: &mut File,
    directory: &File,
    request: FtpResumeRequest<'_>,
    control: &FtpTransferControl,
) -> Result<Progress, FtpReceiveError> {
    let _finish = begin(control)?;
    control.checkpoint()?;
    let proof = request.proof;
    proof.validate()?;
    validate_binding(request.binding)?;
    if &proof.binding != request.binding
        || !valid_arg(request.attempt_generation, 256, false)
        || request.attempt_generation == proof.attempt_generation
        || request.binding.credential.is_some()
            != matches!(credentials, FtpCredentials::Password { .. })
    {
        return Err(FtpReceiveError::ResumeBindingMismatch);
    }
    if held_identity(file, false)? != proof.partial_identity
        || held_identity(directory, true)? != proof.parent_identity
    {
        return Err(FtpReceiveError::FileIdentityMismatch);
    }
    exact_length(file, proof.trusted_bytes)?;
    if digest(file, proof.trusted_bytes, control)? != proof.prefix_sha256 {
        return Err(FtpReceiveError::LocalPrefixMismatch);
    }
    exact_length(file, proof.trusted_bytes)?;
    control
        .0
        .worker
        .record_session_progress(Progress::new(proof.trusted_bytes, proof.total_bytes));
    transfer(
        endpoint,
        &request.binding.command_path,
        credentials,
        file,
        control,
        proof.trusted_bytes,
        proof.total_bytes,
    )
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    struct GatedReader {
        bytes: Cursor<Vec<u8>>,
        read_started: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
    }

    impl Read for GatedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = self.bytes.read(buffer)?;
            if let Some(started) = self.read_started.take() {
                started.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            Ok(count)
        }
    }

    impl Seek for GatedReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.bytes.seek(position)
        }
    }

    #[test]
    fn local_hash_checks_controls_after_each_read_before_trusting_progress() {
        for cancelled in [false, true] {
            let control = FtpTransferControl::default();
            let worker_control = control.clone();
            let (started, read_started) = mpsc::channel();
            let (release, continue_read) = mpsc::channel();
            let (done, result) = mpsc::channel();
            let worker = thread::spawn(move || {
                let _finish = begin(&worker_control).unwrap();
                let mut reader = GatedReader {
                    bytes: Cursor::new(vec![7; 65536]),
                    read_started: Some(started),
                    release: continue_read,
                };
                done.send(digest(&mut reader, 65536, &worker_control))
                    .unwrap();
            });
            read_started.recv_timeout(Duration::from_secs(3)).unwrap();
            let pause_control = control.clone();
            let (acknowledged, ack) = mpsc::channel();
            let waiter = thread::spawn(move || {
                acknowledged.send(pause_control.request_pause()).unwrap();
            });
            let (lock, wake) = &*control.0.worker.inner;
            let state = lock.lock().unwrap();
            let (state, _) = wake
                .wait_timeout_while(state, Duration::from_secs(3), |state| {
                    !state.pause_requested
                })
                .unwrap();
            let requested = state.pause_requested;
            drop(state);
            release.send(()).unwrap();
            let paused = ack.recv_timeout(Duration::from_secs(3));
            if paused.is_err() || !requested {
                control.cancel();
            }
            waiter.join().unwrap();
            assert!(requested);
            assert_eq!(paused.unwrap(), Ok(true));
            assert!(control.progress().is_none());
            assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
            if cancelled {
                control.cancel();
            } else {
                control.resume().unwrap();
            }
            let outcome = result.recv_timeout(Duration::from_secs(3)).unwrap();
            worker.join().unwrap();
            if cancelled {
                assert_eq!(outcome, Err(FtpReceiveError::Cancelled));
            } else {
                assert_eq!(
                    outcome.unwrap(),
                    <[u8; 32]>::from(Sha256::digest(vec![7; 65536]))
                );
            }
        }
    }
}
