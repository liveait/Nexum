//! Offline FTP publication. The caller holds the data-directory ownership,
//! joins any old writer and retains exclusive file access through completion.
//! A future Server worker must run hashing outside its outer Core/RPC mutex.

use std::{cell::RefCell, fmt, fs::File};

use nexum_domain::TaskId;
use nexum_engine::{
    EngineError,
    ftp::{
        self, FtpCompletionProof, FtpCompletionRequest, FtpPublicationError, FtpPublicationPlan,
        FtpPublicationRequest, FtpPublishedCompletion, FtpResumeBinding, FtpTransferControl,
    },
};
use nexum_storage::{
    SqliteRepository, StorageError, StoredTask, ftp_publication::FtpPublicationRecord,
    ftp_resume::FtpAttempt, origin::random_id,
};
use nexum_task::TaskState;

use crate::{
    Core, CoreError,
    ftp_resume::{CompletionGuard, FtpResumeAttempt, typed},
};

/// Independent proof, origin and original producing generation from the attempt.
/// Stable across claims; only the owner and publication authorization change.
struct Completion {
    binding: FtpResumeBinding,
    producer: String,
    proof: FtpCompletionProof,
}

impl Completion {
    fn from_attempt(attempt: &FtpResumeAttempt) -> Result<Self, CoreError> {
        Ok(Self {
            binding: attempt.binding().clone(),
            producer: attempt.completion_generation().ok_or_else(invalid)?.into(),
            proof: attempt.completion().ok_or_else(invalid)?.clone(),
        })
    }

    fn request<'a>(&'a self, owner: &'a str, authorization: &'a str) -> FtpPublicationRequest<'a> {
        FtpPublicationRequest {
            completion: FtpCompletionRequest {
                binding: &self.binding,
                attempt_generation: &self.producer,
            },
            proof: &self.proof,
            owner_generation: owner,
            authorization,
        }
    }

    fn retained_plan(&self, attempt: &FtpAttempt) -> Result<FtpPublicationPlan, CoreError> {
        let intent = attempt.publication.as_ref().ok_or_else(invalid)?;
        let plan = FtpPublicationPlan::decode(&intent.plan).map_err(engine_error)?;
        plan.check_request(&self.request(&intent.owner_generation, &intent.authorization))
            .map_err(engine_error)?;
        Ok(plan)
    }
}

/// Snapshot saved by prepare/rearm. Each use requires the same complete
/// Downloading task, active incarnation, attempt, owner and durable intent.
pub struct PreparedFtpPublication {
    task: StoredTask,
    attempt: FtpAttempt,
    completion: Completion,
    authorization: String,
    plan: FtpPublicationPlan,
}

impl fmt::Debug for PreparedFtpPublication {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedFtpPublication")
            .finish_non_exhaustive()
    }
}

/// Produced only by a successful Core-fenced namespace operation. A separate
/// transaction must still confirm that this exact snapshot owns completion.
pub struct PublishedFtpCompletion<'p> {
    prepared: &'p PreparedFtpPublication,
    evidence: FtpPublishedCompletion,
}

impl fmt::Debug for PublishedFtpCompletion<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublishedFtpCompletion")
            .finish_non_exhaustive()
    }
}

fn invalid() -> CoreError {
    engine_error(FtpPublicationError::AuthorizationMismatch)
}

fn engine_error(error: FtpPublicationError) -> CoreError {
    match error {
        FtpPublicationError::Transfer(ftp::FtpReceiveError::Cancelled) => {
            EngineError::Cancelled.into()
        }
        _ => EngineError::Failed(error.to_string()).into(),
    }
}

impl Core<SqliteRepository> {
    fn publication_task(&self, id: &TaskId) -> Result<StoredTask, StorageError> {
        self.tasks
            .get(id)
            .cloned()
            .map(StoredTask::from)
            .ok_or_else(|| StorageError::NotFound(id.clone()))
    }

    fn match_publication_task(&self, expected: &StoredTask) -> Result<(), StorageError> {
        if self.publication_task(&expected.id)? != *expected {
            return Err(StorageError::Other("FTP publication task changed".into()));
        }
        Ok(())
    }

    /// Startup recovery with the ordinary recovery rules plus Failed tasks
    /// with a validated pending FTP intent. Call under exclusive startup ownership.
    pub fn recover_with_ftp_publications(&mut self) -> Result<usize, CoreError> {
        self.recover_selecting(|repository, task| {
            let Some(origin) = repository.active_origin(&task.id)? else {
                return Ok(false);
            };
            if origin.security != "plain" {
                return Ok(false);
            }
            let Some(stored) = repository.ftp_attempt(&origin)? else {
                return Ok(false);
            };
            if stored.publication.is_none() {
                return Ok(false);
            }
            let attempt = typed(stored)?;
            Completion::from_attempt(&attempt)?.retained_plan(&attempt.stored)?;
            Ok(true)
        })
    }

    /// Claim a selected, recovered Queued task without resetting proven progress.
    /// None means no slot or queued entry. A retained intent needs explicit rearm.
    pub fn claim_queued_ftp_completion(
        &mut self,
        attempt: &FtpResumeAttempt,
    ) -> Result<Option<FtpResumeAttempt>, CoreError> {
        let completion = Completion::from_attempt(attempt)?;
        if attempt.stored.publication.is_some() {
            completion.retained_plan(&attempt.stored)?;
        }
        let id = TaskId::from(attempt.binding().task_id.clone());
        let task = self.publication_task(&id)?;
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        if !scheduler.claim_queued(&mut tasks, &id)? {
            return Ok(None);
        }
        tasks.set_error(&id, None)?;
        let (_, stored) = self
            .repository
            .claim_ftp_publication(&task, &attempt.stored)?;
        let mut claimed = attempt.clone();
        claimed.stored = stored;
        self.tasks = tasks;
        self.scheduler = scheduler;
        Ok(Some(claimed))
    }

    pub fn prepare_ftp_publication(
        &mut self,
        attempt: &FtpResumeAttempt,
        file: &File,
        directory: &File,
        source_leaf: &str,
    ) -> Result<PreparedFtpPublication, CoreError> {
        self.prepare_publication(attempt, file, directory, Some(source_leaf))
    }

    /// Preserve the stored leaves, original proof and producer. The held file
    /// may be the original final when its redundant staging alias is absent.
    pub fn rearm_ftp_publication(
        &mut self,
        attempt: &FtpResumeAttempt,
        file: &File,
        directory: &File,
    ) -> Result<PreparedFtpPublication, CoreError> {
        self.prepare_publication(attempt, file, directory, None)
    }

    fn prepare_publication(
        &mut self,
        attempt: &FtpResumeAttempt,
        file: &File,
        directory: &File,
        source_leaf: Option<&str>,
    ) -> Result<PreparedFtpPublication, CoreError> {
        let task = self.publication_task(&TaskId::from(attempt.binding().task_id.clone()))?;
        let completion = Completion::from_attempt(attempt)?;
        let authorization = random_id()?;
        let request = completion.request(attempt.generation(), &authorization);
        let plan = match source_leaf {
            Some(leaf) => FtpPublicationPlan::prepare(file, directory, leaf, request),
            None => completion
                .retained_plan(&attempt.stored)?
                .rearm(file, directory, request),
        }
        .map_err(engine_error)?;
        let bytes = plan.encode().map_err(engine_error)?;
        let stored = if source_leaf.is_some() {
            self.repository.prepare_ftp_publication(
                &task,
                &attempt.stored,
                &authorization,
                &bytes,
            )?
        } else {
            self.repository
                .rearm_ftp_publication(&task, &attempt.stored, &authorization, &bytes)?
        };
        Ok(PreparedFtpPublication {
            task,
            attempt: stored,
            completion,
            authorization,
            plan,
        })
    }

    pub fn publish_ftp_completion<'p>(
        &self,
        prepared: &'p PreparedFtpPublication,
        directory: &File,
        control: &FtpTransferControl,
    ) -> Result<PublishedFtpCompletion<'p>, CoreError> {
        self.run_publication(prepared, directory, control, false)
    }

    pub fn reconcile_ftp_publication<'p>(
        &self,
        prepared: &'p PreparedFtpPublication,
        directory: &File,
        control: &FtpTransferControl,
    ) -> Result<PublishedFtpCompletion<'p>, CoreError> {
        self.run_publication(prepared, directory, control, true)
    }

    fn run_publication<'p>(
        &self,
        prepared: &'p PreparedFtpPublication,
        directory: &File,
        control: &FtpTransferControl,
        reconcile: bool,
    ) -> Result<PublishedFtpCompletion<'p>, CoreError> {
        let request = prepared
            .completion
            .request(&prepared.attempt.generation, &prepared.authorization);
        let failure = RefCell::new(None);
        let guard = || {
            CompletionGuard::capture(
                self.match_publication_task(&prepared.task).and_then(|()| {
                    self.repository
                        .guard_ftp_publication(&prepared.task, &prepared.attempt)
                }),
                &failure,
            )
        };
        let result = if reconcile {
            ftp::reconcile_publication_with_guard(
                directory,
                &prepared.plan,
                request,
                control,
                guard,
            )
        } else {
            ftp::publish_completion_with_guard(directory, &prepared.plan, request, control, guard)
        };
        if let Some(error) = failure.into_inner() {
            return Err(error.into());
        }
        Ok(PublishedFtpCompletion {
            prepared,
            evidence: result.map_err(engine_error)?,
        })
    }

    /// Save final progress, Completed and the completed journal together. All
    /// task/scheduler events are candidates until Storage commits successfully.
    pub fn complete_ftp_publication(
        &mut self,
        published: &PublishedFtpCompletion<'_>,
    ) -> Result<(), CoreError> {
        let prepared = published.prepared;
        let expected = prepared.completion.retained_plan(&prepared.attempt)?;
        if published.evidence.plan().encode().map_err(engine_error)?
            != expected.encode().map_err(engine_error)?
        {
            return Err(invalid());
        }
        self.match_publication_task(&prepared.task)?;
        let id = &prepared.task.id;
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        tasks.update_progress(id, published.evidence.progress())?;
        tasks.set_error(id, None)?;
        scheduler.mark_finished(&mut tasks, id, TaskState::Completed)?;
        self.repository
            .complete_ftp_publication(&prepared.task, &prepared.attempt)?;
        self.tasks = tasks;
        self.scheduler = scheduler;
        Ok(())
    }

    pub fn completed_ftp_publications(&self) -> Result<Vec<FtpPublicationRecord>, CoreError> {
        Ok(self.repository.completed_ftp_publications()?)
    }

    pub fn orphan_ftp_publications(&self) -> Result<Vec<FtpPublicationRecord>, CoreError> {
        Ok(self.repository.orphan_ftp_publications()?)
    }
}
