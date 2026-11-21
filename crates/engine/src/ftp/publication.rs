//! Offline publication within the accepted macOS ordinary-desktop scope.
use super::*;

const PLAN_FORMAT: &str = "ftp-publication/v1";

/// Independently supplied origin, producing proof, and current intent owner.
pub struct FtpPublicationRequest<'a> {
    pub completion: FtpCompletionRequest<'a>,
    pub proof: &'a FtpCompletionProof,
    pub owner_generation: &'a str,
    pub authorization: &'a str,
}

/// Bounded intent. An encoded plan alone never authorizes a namespace operation.
#[derive(Clone)]
pub struct FtpPublicationPlan(PlanData);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanData {
    format: String,
    completion: CompletionData,
    source_leaf: String,
    final_leaf: String,
    owner_generation: String,
    authorization: String,
}

/// Point-in-time evidence for a separate, freshly fenced durable completion.
pub struct FtpPublishedCompletion(FtpPublicationPlan);

impl FtpPublishedCompletion {
    pub fn plan(&self) -> &FtpPublicationPlan {
        &self.0
    }

    pub fn progress(&self) -> Progress {
        self.0.progress()
    }
}

impl fmt::Debug for FtpPublicationPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FtpPublicationPlan")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for FtpPublishedCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FtpPublishedCompletion")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FtpPublicationError {
    InvalidPlan,
    AuthorizationMismatch,
    UnsupportedPlatform,
    Transfer(FtpReceiveError),
    Namespace(io::ErrorKind),
}

impl From<FtpReceiveError> for FtpPublicationError {
    fn from(error: FtpReceiveError) -> Self {
        Self::Transfer(error)
    }
}
impl fmt::Display for FtpPublicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan => f.write_str("invalid FTP publication plan"),
            Self::AuthorizationMismatch => f.write_str("FTP publication authorization mismatch"),
            Self::UnsupportedPlatform => {
                f.write_str("FTP publication is unsupported on this platform")
            }
            Self::Transfer(error) => error.fmt(f),
            Self::Namespace(kind) => {
                write!(f, "FTP publication filesystem operation failed ({kind})")
            }
        }
    }
}
impl std::error::Error for FtpPublicationError {}

type Result<T> = std::result::Result<T, FtpPublicationError>;

impl FtpPublicationPlan {
    pub fn prepare(
        file: &File,
        directory: &File,
        source_leaf: &str,
        request: FtpPublicationRequest<'_>,
    ) -> Result<Self> {
        if !valid_leaf(source_leaf) || !valid_authority(&request) {
            return Err(FtpPublicationError::InvalidPlan);
        }
        let final_leaf = destination_leaf(&request.proof.binding().destination)?;
        let plan = Self(PlanData {
            format: PLAN_FORMAT.into(),
            completion: request.proof.inner.clone(),
            source_leaf: source_leaf.into(),
            final_leaf: final_leaf.into(),
            owner_generation: request.owner_generation.into(),
            authorization: request.authorization.into(),
        });
        plan.encode()?;
        plan.check_request(&request)?;
        request.proof.matches(file, directory)?;
        platform::check_parent(directory, &plan)?;
        platform::check_leaf(directory, source_leaf, request.proof)?;
        Ok(plan)
    }

    /// Prepare a new authorization for the same durable intent. Retain the
    /// original proof and both names, including after the staging alias was
    /// lost. Entry verification belongs to the fresh namespace guard.
    pub fn rearm(
        &self,
        file: &File,
        directory: &File,
        request: FtpPublicationRequest<'_>,
    ) -> Result<Self> {
        if !valid_authority(&request) || request.authorization == self.authorization() {
            return Err(FtpPublicationError::AuthorizationMismatch);
        }
        let mut plan = self.clone();
        plan.0.owner_generation = request.owner_generation.into();
        plan.0.authorization = request.authorization.into();
        plan.check_request(&request)?;
        plan.encode()?;
        request.proof.matches(file, directory)?;
        platform::check_parent(directory, &plan)?;
        Ok(plan)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(&self.0).map_err(|_| FtpPublicationError::InvalidPlan)?;
        if bytes.len() > MAX_PROOF_BYTES {
            return Err(FtpPublicationError::InvalidPlan);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PROOF_BYTES {
            return Err(FtpPublicationError::InvalidPlan);
        }
        let invalid = |_| FtpPublicationError::InvalidPlan;
        // Typed decoding rejects duplicate and unexpected fields. Compare the
        // shape too: serde otherwise accepts a missing Option field as null.
        let plan = Self(serde_json::from_slice(bytes).map_err(invalid)?);
        let supplied: serde_json::Value = serde_json::from_slice(bytes).map_err(invalid)?;
        if serde_json::to_value(&plan.0).map_err(invalid)? != supplied {
            return Err(FtpPublicationError::InvalidPlan);
        }
        plan.validate()?;
        Ok(plan)
    }

    fn validate(&self) -> Result<()> {
        let data = &self.0;
        if data.format != PLAN_FORMAT
            || data.completion.format != FORMAT
            || !valid_arg(self.producer_generation(), 256, false)
            || !valid_arg(self.owner_generation(), 256, false)
            || !valid_arg(self.authorization(), 256, false)
            || !valid_leaf(&data.source_leaf)
            || destination_leaf(&self.binding().destination)? != data.final_leaf
            || data.source_leaf == data.final_leaf
        {
            return Err(FtpPublicationError::InvalidPlan);
        }
        validate_binding(self.binding())?;
        Ok(())
    }

    /// Compare the entire original proof with independent origin/producer and
    /// current owner/authorization supplied by the persistence consumer.
    pub fn check_request(&self, request: &FtpPublicationRequest<'_>) -> Result<()> {
        if self.binding() != request.completion.binding
            || self.producer_generation() != request.completion.attempt_generation
            || self.0.completion != request.proof.inner
            || self.owner_generation() != request.owner_generation
            || self.authorization() != request.authorization
        {
            return Err(FtpPublicationError::AuthorizationMismatch);
        }
        Ok(())
    }

    pub fn binding(&self) -> &FtpResumeBinding {
        &self.0.completion.binding
    }
    pub fn producer_generation(&self) -> &str {
        &self.0.completion.attempt_generation
    }
    pub fn owner_generation(&self) -> &str {
        &self.0.owner_generation
    }
    pub fn authorization(&self) -> &str {
        &self.0.authorization
    }
    pub fn progress(&self) -> Progress {
        Progress::new(self.0.completion.length, Some(self.0.completion.length))
    }
}

fn valid_authority(request: &FtpPublicationRequest<'_>) -> bool {
    valid_arg(request.owner_generation, 256, false) && valid_arg(request.authorization, 256, false)
}

fn valid_leaf(value: &str) -> bool {
    valid_arg(value, 255, false) && !matches!(value, "." | "..") && !value.contains(['/', '\\'])
}

fn destination_leaf(destination: &str) -> Result<&str> {
    let absolute = destination
        .strip_prefix('/')
        .ok_or(FtpPublicationError::InvalidPlan)?;
    if !absolute.split('/').all(valid_leaf) {
        return Err(FtpPublicationError::InvalidPlan);
    }
    absolute
        .rsplit('/')
        .next()
        .ok_or(FtpPublicationError::InvalidPlan)
}

fn optional_leaf(directory: &File, leaf: &str, proof: &FtpCompletionProof) -> Result<Option<File>> {
    match platform::check_leaf(directory, leaf, proof) {
        Ok(file) => Ok(Some(file)),
        Err(FtpPublicationError::Namespace(io::ErrorKind::NotFound)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Publish the proof-bound source with one no-clobber linkat. Use a fresh
/// single-use control and hold exclusive writer access through completion.
/// The guard is acquired after hashing, under the cancellation decision lock.
pub fn publish_completion_with_guard<G: FtpCompletionGuard>(
    directory: &File,
    plan: &FtpPublicationPlan,
    request: FtpPublicationRequest<'_>,
    control: &FtpTransferControl,
    guard: impl FnOnce() -> std::result::Result<G, FtpReceiveError>,
) -> Result<FtpPublishedCompletion> {
    Publication {
        directory,
        plan,
        request,
        control,
        reconcile: false,
    }
    .run(guard, digest, File::sync_all)
}

/// Adopt an existing matching final entry, or publish from the matching
/// staging entry if the final name is absent. All work uses the same control.
/// An existing verified final permits a missing redundant source. A present
/// conflicting source is retained and diagnosed after adopting the final.
pub fn reconcile_publication_with_guard<G: FtpCompletionGuard>(
    directory: &File,
    plan: &FtpPublicationPlan,
    request: FtpPublicationRequest<'_>,
    control: &FtpTransferControl,
    guard: impl FnOnce() -> std::result::Result<G, FtpReceiveError>,
) -> Result<FtpPublishedCompletion> {
    Publication {
        directory,
        plan,
        request,
        control,
        reconcile: true,
    }
    .run(guard, digest, File::sync_all)
}

struct Publication<'a> {
    directory: &'a File,
    plan: &'a FtpPublicationPlan,
    request: FtpPublicationRequest<'a>,
    control: &'a FtpTransferControl,
    reconcile: bool,
}

impl Publication<'_> {
    fn run<G: FtpCompletionGuard>(
        &self,
        guard: impl FnOnce() -> std::result::Result<G, FtpReceiveError>,
        hash: impl FnOnce(
            &mut File,
            u64,
            &FtpTransferControl,
        ) -> std::result::Result<[u8; 32], FtpReceiveError>,
        sync: impl FnOnce(&File) -> io::Result<()>,
    ) -> Result<FtpPublishedCompletion> {
        let _finish = begin(self.control)?;
        self.control.checkpoint()?;
        self.plan.check_request(&self.request)?;
        let proof = self.request.proof;
        platform::check_parent(self.directory, self.plan)?;
        let existing = if self.reconcile {
            optional_leaf(self.directory, &self.plan.0.final_leaf, proof)?
        } else {
            None
        };
        let observed_final = existing.is_some();
        let mut held = match existing {
            Some(final_file) => final_file,
            None => platform::check_leaf(self.directory, &self.plan.0.source_leaf, proof)?,
        };
        if hash(&mut held, proof.inner.length, self.control)? != proof.inner.sha256 {
            return Err(FtpReceiveError::LocalPrefixMismatch.into());
        }

        let (lock, _) = &*self.control.0.worker.inner;
        let (validation, release) = loop {
            self.control.checkpoint()?;
            let mut state = lock.lock().expect("FTP publication control mutex poisoned");
            if state.cancelled {
                return Err(FtpReceiveError::Cancelled.into());
            }
            if state.pause_requested {
                continue;
            }
            let reservation = guard()?;
            let validation = self.decide(observed_final, &mut state.committed);
            // Release explicitly on every acquired path, including pre-link errors.
            break (validation, reservation.release());
        };
        // A successful syscall cannot be rolled back. Attempt the directory sync
        // even after a release or post-link validation error. FinishOnDrop keeps
        // waiters running until all I/O has returned.
        let synchronized = if self.control.is_committed() {
            sync(self.directory).map_err(|e| FtpPublicationError::Namespace(e.kind()))
        } else {
            Ok(())
        };
        release?;
        let adopted_existing = validation?;
        synchronized?;
        self.check_entries(adopted_existing)?;
        Ok(FtpPublishedCompletion(self.plan.clone()))
    }

    fn check_entries(&self, adopted_existing: bool) -> Result<()> {
        platform::check_parent(self.directory, self.plan)?;
        platform::check_leaf(self.directory, &self.plan.0.final_leaf, self.request.proof)?;
        if adopted_existing {
            optional_leaf(self.directory, &self.plan.0.source_leaf, self.request.proof)?;
        } else {
            platform::check_leaf(self.directory, &self.plan.0.source_leaf, self.request.proof)?;
        }
        Ok(())
    }

    fn decide(&self, observed_final: bool, committed: &mut bool) -> Result<bool> {
        platform::check_parent(self.directory, self.plan)?;
        if self.reconcile {
            if optional_leaf(self.directory, &self.plan.0.final_leaf, self.request.proof)?.is_some()
            {
                // We hashed this same identity outside the lock. Record the
                // fact before any diagnostic check of the redundant alias.
                *committed = true;
                self.check_entries(true)?;
                return Ok(true);
            }
            if observed_final {
                return Err(FtpPublicationError::Namespace(io::ErrorKind::NotFound));
            }
        }
        platform::check_leaf(self.directory, &self.plan.0.source_leaf, self.request.proof)?;
        platform::link(
            self.directory,
            &self.plan.0.source_leaf,
            &self.plan.0.final_leaf,
        )?;
        *committed = true;
        self.check_entries(false)?;
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    fn fs_error(error: io::Error) -> FtpPublicationError {
        FtpPublicationError::Namespace(error.kind())
    }

    pub(super) fn check_parent(held: &File, plan: &FtpPublicationPlan) -> Result<()> {
        let parent = Path::new(&plan.binding().destination)
            .parent()
            .ok_or(FtpPublicationError::InvalidPlan)?;
        // Refuse symlinks at every component, including ancestor directories.
        let named = File::options()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW_ANY | libc::O_DIRECTORY)
            .open(parent)
            .map_err(fs_error)?;
        let expected = &plan.0.completion.parent_identity;
        if &held_identity(&named, true)? != expected || &held_identity(held, true)? != expected {
            return Err(FtpReceiveError::FileIdentityMismatch.into());
        }
        Ok(())
    }

    pub(super) fn check_leaf(
        directory: &File,
        name: &str,
        proof: &FtpCompletionProof,
    ) -> Result<File> {
        let name = CString::new(name).map_err(|_| FtpPublicationError::InvalidPlan)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: directory is live, name is NUL-terminated, and stat is writable.
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(fs_error(io::Error::last_os_error()));
        }
        // SAFETY: fstatat above initialized stat.
        if unsafe { stat.assume_init() }.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err(FtpReceiveError::FileIdentityMismatch.into());
        }
        // SAFETY: openat borrows a live directory descriptor and C string.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(fs_error(io::Error::last_os_error()));
        }
        // SAFETY: successful openat returned a fresh owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        proof.matches(&file, directory)?;
        // A case/normalization-insensitive lookup may have found a different
        // spelling (including the other leaf). Require the actual entry name.
        let mut path = [0_u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes at most PATH_MAX bytes into this buffer.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, path.as_mut_ptr()) } == -1 {
            return Err(fs_error(io::Error::last_os_error()));
        }
        let end = path
            .iter()
            .position(|b| *b == 0)
            .ok_or(FtpPublicationError::InvalidPlan)?;
        if path[..end].rsplit(|b| *b == b'/').next() != Some(name.as_bytes()) {
            return Err(FtpReceiveError::FileIdentityMismatch.into());
        }
        Ok(file)
    }

    pub(super) fn link(directory: &File, source: &str, target: &str) -> Result<()> {
        let source = CString::new(source).map_err(|_| FtpPublicationError::InvalidPlan)?;
        let target = CString::new(target).map_err(|_| FtpPublicationError::InvalidPlan)?;
        let fd = directory.as_raw_fd();
        // SAFETY: both names are live NUL-terminated strict leaves. Zero flags
        // do not follow a substituted source symlink. linkat never overwrites.
        if unsafe { libc::linkat(fd, source.as_ptr(), fd, target.as_ptr(), 0) } != 0 {
            return Err(fs_error(io::Error::last_os_error()));
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;
    pub(super) fn check_parent(_: &File, _: &FtpPublicationPlan) -> Result<()> {
        Err(FtpPublicationError::UnsupportedPlatform)
    }
    pub(super) fn check_leaf(_: &File, _: &str, _: &FtpCompletionProof) -> Result<File> {
        Err(FtpPublicationError::UnsupportedPlatform)
    }
    pub(super) fn link(_: &File, _: &str, _: &str) -> Result<()> {
        Err(FtpPublicationError::UnsupportedPlatform)
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "publication_tests.rs"]
mod tests;
