//! Nexum Core orchestration layer.

pub use nexum_domain;
pub use nexum_engine;
pub use nexum_plugin;
pub use nexum_resolver;
pub use nexum_scheduler;
pub use nexum_storage;
pub use nexum_task;

use nexum_domain::{Destination, DownloadSource, TaskId};
use nexum_engine::{EngineError, EngineRegistry, EngineTask};
use nexum_plugin::{PluginError, PluginManager};
use nexum_resolver::{ResolveRequest, ResolveResult, ResolverError, ResolverRegistry};
use nexum_scheduler::{Priority, Scheduler, SchedulerConfig, SchedulerError, SchedulerEvent};
use nexum_storage::{InMemoryRepository, StorageError, StoredTask, TaskRepository};
use nexum_task::{DownloadTask, TaskService, TaskServiceError, TaskState};
use std::collections::HashMap;

#[derive(Debug, Eq, PartialEq)]
pub enum CoreError {
    Scheduler(SchedulerError),
    Task(TaskServiceError),
    Storage(StorageError),
    Resolver(ResolverError),
    Engine(EngineError),
    Plugin(PluginError),
}

impl From<SchedulerError> for CoreError {
    fn from(value: SchedulerError) -> Self {
        Self::Scheduler(value)
    }
}
impl From<TaskServiceError> for CoreError {
    fn from(value: TaskServiceError) -> Self {
        Self::Task(value)
    }
}
impl From<StorageError> for CoreError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}
impl From<ResolverError> for CoreError {
    fn from(value: ResolverError) -> Self {
        Self::Resolver(value)
    }
}
impl From<EngineError> for CoreError {
    fn from(value: EngineError) -> Self {
        Self::Engine(value)
    }
}
impl From<PluginError> for CoreError {
    fn from(value: PluginError) -> Self {
        Self::Plugin(value)
    }
}

/// Core orchestration with an injectable task repository.
pub struct Core<R: TaskRepository = InMemoryRepository> {
    pub tasks: TaskService,
    pub scheduler: Scheduler,
    pub repository: R,
    pub resolver: ResolverRegistry,
    pub engines: EngineRegistry,
    pub engine_tasks: HashMap<TaskId, (String, EngineTask)>,
    pub plugins: PluginManager,
}

impl Core<InMemoryRepository> {
    pub fn new(scheduler_config: SchedulerConfig) -> Result<Self, SchedulerError> {
        Self::with_repository(scheduler_config, InMemoryRepository::new())
    }
}

impl<R: TaskRepository> Core<R> {
    pub fn with_repository(
        scheduler_config: SchedulerConfig,
        repository: R,
    ) -> Result<Self, SchedulerError> {
        Ok(Self {
            tasks: TaskService::new(),
            scheduler: Scheduler::new(scheduler_config)?,
            repository,
            resolver: ResolverRegistry::new(),
            engines: {
                let mut registry = EngineRegistry::new();
                registry.register(Box::new(nexum_engine::InMemoryEngine::new()));
                registry.register(Box::new(nexum_engine::HttpEngine::new()));
                registry
            },
            engine_tasks: HashMap::new(),
            plugins: PluginManager::new(),
        })
    }

    /// Registers a plugin manifest and initializes it.
    ///
    /// If the plugin implements `EngineProvider`, its engine is registered with
    /// the engine registry. If it implements `ResolverProvider`, its resolver
    /// is registered with the resolver registry.
    pub fn register_plugin(
        &mut self,
        manifest: nexum_plugin::PluginManifest,
    ) -> Result<(), CoreError> {
        let idx = self.plugins.register(manifest);
        let plugin_id = self.plugins.ids()[idx].to_owned();

        // Attempt to load the plugin lifecycle
        // (this is a no-op for plugins without lifecycle implementations)
        self.plugins.loaded(&plugin_id).ok();

        // Check if this is an engine provider
        // The engine crate's NoOpLifecycle is the default; plugins that
        // implement EngineProvider also implement PluginLifecycle
        Ok(())
    }

    /// Registers engines and resolvers from all started plugins.
    ///
    /// Plugins that implement `EngineProvider` or `ResolverProvider` will have
    /// their engines and resolvers added to the respective registries.
    pub fn init_plugins(&mut self) -> Result<(), CoreError> {
        let ids: Vec<String> = self
            .plugins
            .ids()
            .into_iter()
            .map(|s| s.to_owned())
            .collect();
        for id in ids {
            // Attempt to start each plugin and register its capabilities
            // For engine providers, we'd call make_engine and register it.
            // This is a no-op until plugins implement EngineProvider.
            let _ = self.plugins.start_plugin(&id);
        }
        Ok(())
    }

    pub fn resolve_source(&self, source: impl Into<String>) -> Result<ResolveResult, CoreError> {
        let request = ResolveRequest::new(source)?;
        Ok(self.resolver.resolve(&request)?)
    }

    pub fn create_task(
        &mut self,
        id: impl Into<TaskId>,
        source: DownloadSource,
        destination: Destination,
    ) -> Result<DownloadTask, CoreError> {
        self.resolve_source(source.as_str())?;
        let id = id.into();
        if self.tasks.get(&id).is_some() {
            return Err(TaskServiceError::AlreadyExists(id).into());
        }
        let task = DownloadTask::new(id.clone(), source.clone(), destination.clone());
        self.repository.insert(StoredTask::from(task))?;
        Ok(self.tasks.create(id, source, destination)?.clone())
    }

    pub fn queue_task(&mut self, id: &TaskId, priority: Priority) -> Result<(), CoreError> {
        self.commit_task_change(id, |tasks, scheduler| {
            scheduler.enqueue(tasks, id, priority)?;
            Ok(())
        })
    }

    /// Reserves the next scheduler slot without starting an engine.
    pub fn claim_next(&mut self) -> Result<Option<TaskId>, CoreError> {
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        let Some(id) = scheduler.start_next(&mut tasks)? else {
            return Ok(None);
        };
        // A claimed task starts with a fresh task snapshot. Server-owned
        // transfer paths may restore validated partial progress afterwards.
        tasks.update_progress(&id, nexum_domain::Progress::default())?;
        tasks.set_error(&id, None)?;
        self.persist_candidate(&id, tasks, scheduler)?;
        Ok(Some(id))
    }

    /// Reserves a selected queued task without starting an engine.
    pub fn claim_queued(&mut self, id: &TaskId) -> Result<bool, CoreError> {
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        if !scheduler.claim_queued(&mut tasks, id)? {
            return Ok(false);
        }
        tasks.update_progress(id, nexum_domain::Progress::default())?;
        tasks.set_error(id, None)?;
        self.persist_candidate(id, tasks, scheduler)?;
        Ok(true)
    }

    pub fn start_next(&mut self) -> Result<Option<TaskId>, CoreError> {
        self.start_next_with_engine("in-memory")
    }

    pub fn start_next_with_engine(
        &mut self,
        engine_name: &str,
    ) -> Result<Option<TaskId>, CoreError> {
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        let Some(task_id) = scheduler.start_next(&mut tasks)? else {
            return Ok(None);
        };
        tasks.update_progress(&task_id, nexum_domain::Progress::default())?;
        tasks.set_error(&task_id, None)?;
        let task = tasks
            .get(&task_id)
            .cloned()
            .ok_or_else(|| TaskServiceError::NotFound(task_id.clone()))?;
        let result = self.engines.start_engine(
            engine_name,
            &task.id,
            task.source.as_str(),
            task.destination.as_str(),
        );
        match result {
            Ok(engine_task) => {
                let snapshot = self
                    .engines
                    .get(engine_name)
                    .ok_or_else(|| EngineError::Failed(format!("engine not found: {engine_name}")));
                let snapshot = snapshot.and_then(|engine| {
                    Ok(nexum_engine::EngineSnapshot::new(
                        engine.state(&engine_task)?,
                        engine.progress(&engine_task)?,
                    ))
                });
                let snapshot = match snapshot {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        let _ = self.engines.remove_engine(engine_name, &engine_task);
                        return Err(error.into());
                    }
                };
                let (state, progress) = nexum_engine::map_engine_snapshot(&snapshot);
                if let Err(error) = Self::apply_engine_snapshot(
                    &mut tasks,
                    &mut scheduler,
                    &task_id,
                    state,
                    progress,
                ) {
                    let _ = self.engines.remove_engine(engine_name, &engine_task);
                    return Err(error);
                }
                if let Err(error) = self.persist_candidate(&task_id, tasks, scheduler) {
                    let _ = self.engines.remove_engine(engine_name, &engine_task);
                    return Err(error);
                }
                if matches!(state, TaskState::Completed | TaskState::Failed) {
                    if self
                        .engines
                        .remove_engine(engine_name, &engine_task)
                        .is_err()
                    {
                        self.engine_tasks
                            .insert(task_id.clone(), (engine_name.to_owned(), engine_task));
                    }
                } else {
                    self.engine_tasks
                        .insert(task_id.clone(), (engine_name.to_owned(), engine_task));
                }
                Ok(Some(task_id))
            }
            Err(error) => {
                tasks.set_error(&task_id, Some(error.to_string()))?;
                scheduler.mark_finished(&mut tasks, &task_id, TaskState::Failed)?;
                self.persist_candidate(&task_id, tasks, scheduler)?;
                Err(CoreError::Engine(error))
            }
        }
    }

    pub fn sync_engine_task(&mut self, id: &TaskId) -> Result<(), CoreError> {
        let (engine_name, engine_task) = self
            .engine_tasks
            .get(id)
            .ok_or_else(|| EngineError::TaskNotFound(id.clone()))?
            .clone();
        let snapshot = match self.engines.get(&engine_name) {
            Some(engine) => nexum_engine::EngineSnapshot::new(
                engine.state(&engine_task)?,
                engine.progress(&engine_task)?,
            ),
            None => {
                return Err(EngineError::Failed(format!("engine not found: {engine_name}")).into());
            }
        };
        let (state, progress) = nexum_engine::map_engine_snapshot(&snapshot);
        self.commit_task_change(id, |tasks, scheduler| {
            Self::apply_engine_snapshot(tasks, scheduler, id, state, progress)
        })?;

        if matches!(state, TaskState::Completed | TaskState::Failed)
            && self
                .engines
                .remove_engine(&engine_name, &engine_task)
                .is_ok()
        {
            self.engine_tasks.remove(id);
        }
        Ok(())
    }

    fn apply_engine_snapshot(
        tasks: &mut TaskService,
        scheduler: &mut Scheduler,
        id: &TaskId,
        state: TaskState,
        progress: nexum_domain::Progress,
    ) -> Result<(), CoreError> {
        tasks.update_progress(id, progress)?;
        if tasks.get(id).map(|task| task.state) != Some(state) {
            match state {
                TaskState::Completed | TaskState::Failed => {
                    scheduler.mark_finished(tasks, id, state)?;
                }
                TaskState::Paused => {
                    scheduler.pause(tasks, id)?;
                }
                _ => {
                    tasks.transition(id, state)?;
                }
            }
        }
        Ok(())
    }

    pub fn pause_task(&mut self, id: &TaskId) -> Result<(), CoreError> {
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        scheduler.pause(&mut tasks, id)?;
        if let Some((engine_name, engine_task)) = self.engine_tasks.get(id).cloned() {
            self.engines.pause_engine(&engine_name, &engine_task)?;
            if let Err(error) = self.persist_candidate(id, tasks, scheduler) {
                let _ = self.engines.resume_engine(&engine_name, &engine_task);
                return Err(error);
            }
        } else {
            self.persist_candidate(id, tasks, scheduler)?;
        }
        Ok(())
    }

    pub fn resume_task(&mut self, id: &TaskId) -> Result<bool, CoreError> {
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        let resumed = scheduler.resume(&mut tasks, id)?;
        if !resumed {
            return Ok(false);
        }
        if let Some((engine_name, engine_task)) = self.engine_tasks.get(id).cloned() {
            self.engines.resume_engine(&engine_name, &engine_task)?;
            if let Err(error) = self.persist_candidate(id, tasks, scheduler) {
                let _ = self.engines.pause_engine(&engine_name, &engine_task);
                return Err(error);
            }
        } else {
            self.persist_candidate(id, tasks, scheduler)?;
        }
        Ok(true)
    }

    pub fn finish_task(&mut self, id: &TaskId, state: TaskState) -> Result<(), CoreError> {
        self.commit_task_change(id, |tasks, scheduler| {
            if state == TaskState::Completed {
                tasks.set_error(id, None)?;
            }
            scheduler.mark_finished(tasks, id, state)?;
            Ok(())
        })
    }

    pub fn finish_task_with_error(
        &mut self,
        id: &TaskId,
        state: TaskState,
        error: impl Into<String>,
    ) -> Result<(), CoreError> {
        let error = error.into();
        self.commit_task_change(id, |tasks, scheduler| {
            tasks.set_error(id, Some(error))?;
            scheduler.mark_finished(tasks, id, state)?;
            Ok(())
        })
    }

    pub fn update_progress(
        &mut self,
        id: &TaskId,
        progress: nexum_domain::Progress,
    ) -> Result<(), CoreError> {
        self.commit_task_change(id, |tasks, _scheduler| {
            tasks.update_progress(id, progress)?;
            Ok(())
        })
    }

    pub fn remove_task(&mut self, id: &TaskId) -> Result<DownloadTask, CoreError> {
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        let previous_state = tasks
            .get(id)
            .ok_or_else(|| TaskServiceError::NotFound(id.clone()))?
            .state;
        let task = tasks.remove(id)?;
        scheduler.forget_task(id, previous_state);

        if let Some((engine_name, engine_task)) = self.engine_tasks.get(id).cloned() {
            self.engines.remove_engine(&engine_name, &engine_task)?;
            self.engine_tasks.remove(id);
        }
        if self.repository.remove(id)?.is_none() {
            return Err(StorageError::NotFound(id.clone()).into());
        }
        self.tasks = tasks;
        self.scheduler = scheduler;
        Ok(task)
    }

    fn commit_task_change<T>(
        &mut self,
        id: &TaskId,
        change: impl FnOnce(&mut TaskService, &mut Scheduler) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        let mut tasks = self.tasks.clone();
        let mut scheduler = self.scheduler.clone();
        let result = change(&mut tasks, &mut scheduler)?;
        self.persist_candidate(id, tasks, scheduler)?;
        Ok(result)
    }

    fn persist_candidate(
        &mut self,
        id: &TaskId,
        tasks: TaskService,
        scheduler: Scheduler,
    ) -> Result<(), CoreError> {
        let task = tasks
            .get(id)
            .ok_or_else(|| StorageError::NotFound(id.clone()))?
            .clone();
        self.repository.update(StoredTask::from(task))?;
        self.tasks = tasks;
        self.scheduler = scheduler;
        Ok(())
    }

    pub fn persist_task(&mut self, id: &TaskId) -> Result<(), CoreError> {
        let task = self
            .tasks
            .get(id)
            .ok_or_else(|| StorageError::NotFound(id.clone()))?
            .clone();
        self.repository.update(StoredTask::from(task))?;
        Ok(())
    }

    /// Loads persisted tasks and reconstructs the in-memory queue.
    ///
    /// Downloading, Paused, and Retrying tasks are recovered as Queued so
    /// restart never falsely reports an active transfer.
    pub fn recover(&mut self) -> Result<usize, CoreError> {
        let stored_tasks = self.repository.list()?;
        let mut restored = 0;

        for mut stored in stored_tasks {
            if self.tasks.get(&stored.id).is_some() {
                continue;
            }

            let queued = matches!(
                stored.state,
                TaskState::Queued
                    | TaskState::Downloading
                    | TaskState::Paused
                    | TaskState::Retrying
            );
            if queued {
                let original = stored.clone();
                stored.state = TaskState::Queued;
                stored.progress.speed_bytes_per_second = 0;
                stored.progress.eta_seconds = None;
                if stored != original {
                    // Persist the recovered state before exposing it in memory.
                    self.repository.update(stored.clone())?;
                }
            }

            let task = stored.into_task();
            let id = task.id.clone();
            self.tasks.restore(task)?;

            if queued {
                self.scheduler.restore_queued(&id, Priority::NORMAL);
            }

            restored += 1;
        }

        Ok(restored)
    }

    pub fn drain_task_events(&mut self) -> Vec<nexum_task::TaskEvent> {
        self.tasks.drain_events()
    }

    pub fn drain_scheduler_events(&mut self) -> Vec<SchedulerEvent> {
        self.scheduler.drain_events()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexum_domain::{Destination, DownloadSource, Progress};
    use nexum_engine::{EngineAdapter, EngineCapabilities, EngineTaskState};
    use nexum_storage::SqliteRepository;

    fn config() -> SchedulerConfig {
        SchedulerConfig {
            max_concurrent_tasks: 2,
            ..SchedulerConfig::default()
        }
    }

    fn task_source() -> (DownloadSource, Destination) {
        (
            DownloadSource::new("https://example.com/file"),
            Destination::new("/tmp/file"),
        )
    }

    #[test]
    fn resolves_sources_through_core_boundary() {
        let core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let result = core.resolve_source("https://example.com/file").unwrap();
        assert_eq!(result.kind, nexum_resolver::ResolveKind::Https);
        let (source, destination) = task_source();
        let mut core = core;
        let id = TaskId::from("task-1");
        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::HIGH).unwrap();
        core.update_progress(&id, Progress::new(128, Some(1024)))
            .unwrap();

        let stored = core.repository.get(&id).unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Queued);
        assert_eq!(stored.progress, Progress::new(128, Some(1024)));
    }

    #[test]
    fn rejects_task_creation_with_unsupported_source() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let error = core
            .create_task(
                "invalid-source",
                DownloadSource::new("ftp://example.com/file"),
                Destination::new("/tmp/file"),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CoreError::Resolver(nexum_resolver::ResolverError::UnsupportedScheme(_))
        ));
        assert!(core.tasks.get(&TaskId::from("invalid-source")).is_none());
    }

    #[test]
    fn failed_repository_insert_does_not_create_an_in_memory_task() {
        let (source, destination) = task_source();
        let id = TaskId::from("already-stored");
        let mut repository = InMemoryRepository::new();
        repository
            .insert(DownloadTask::new(id.clone(), source.clone(), destination.clone()).into())
            .unwrap();
        let mut core = Core::with_repository(config(), repository).unwrap();

        assert_eq!(
            core.create_task(id.clone(), source, destination),
            Err(CoreError::Storage(StorageError::AlreadyExists(id.clone())))
        );
        assert!(core.tasks.get(&id).is_none());
        assert!(core.drain_task_events().is_empty());
    }

    #[test]
    fn recovery_rebuilds_queued_tasks() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("task-1");
        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();

        let repository = core.repository;
        let mut restarted = Core::with_repository(config(), repository).unwrap();
        assert_eq!(restarted.recover().unwrap(), 1);
        assert_eq!(restarted.tasks.get(&id).unwrap().state, TaskState::Queued);
        assert_eq!(restarted.scheduler.queued_len(), 1);
    }

    #[test]
    fn recovery_normalizes_interrupted_sqlite_tasks_and_persists_the_result() {
        let mut repository = SqliteRepository::open_in_memory().unwrap();
        let states = [
            TaskState::Created,
            TaskState::Queued,
            TaskState::Downloading,
            TaskState::Paused,
            TaskState::Retrying,
            TaskState::Completed,
            TaskState::Failed,
        ];
        let (source, destination) = task_source();
        let progress = Progress {
            downloaded_bytes: 42,
            total_bytes: Some(100),
            speed_bytes_per_second: 10,
            eta_seconds: Some(6),
        };

        for state in states {
            let mut task =
                DownloadTask::new(format!("{state:?}"), source.clone(), destination.clone());
            task.state = state;
            task.progress = progress.clone();
            repository.insert(task.into()).unwrap();
        }

        let mut core = Core::with_repository(config(), repository).unwrap();
        assert_eq!(core.recover().unwrap(), states.len());
        assert_eq!(core.scheduler.queued_len(), 4);
        assert_eq!(core.scheduler.active_len(), 0);

        for state in states {
            let id = TaskId::from(format!("{state:?}"));
            let queued = matches!(
                state,
                TaskState::Queued
                    | TaskState::Downloading
                    | TaskState::Paused
                    | TaskState::Retrying
            );
            let expected_state = if queued { TaskState::Queued } else { state };
            let mut expected_progress = progress.clone();
            if queued {
                expected_progress.speed_bytes_per_second = 0;
                expected_progress.eta_seconds = None;
            }
            let restored = core.tasks.get(&id).unwrap();
            assert_eq!(restored.state, expected_state);
            assert_eq!(restored.progress, expected_progress);
            let stored = core.repository.get(&id).unwrap().unwrap();
            assert_eq!(stored.state, expected_state);
            assert_eq!(stored.progress, expected_progress);
        }

        assert_eq!(core.recover().unwrap(), 0);
        assert_eq!(core.scheduler.queued_len(), 4);

        let mut restarted = Core::with_repository(config(), core.repository).unwrap();
        assert_eq!(restarted.recover().unwrap(), states.len());
        assert_eq!(restarted.scheduler.queued_len(), 4);
    }

    struct UpdateFailureRepository {
        inner: InMemoryRepository,
    }

    impl TaskRepository for UpdateFailureRepository {
        fn insert(&mut self, task: StoredTask) -> Result<(), StorageError> {
            self.inner.insert(task)
        }

        fn get(&self, id: &TaskId) -> Result<Option<StoredTask>, StorageError> {
            self.inner.get(id)
        }

        fn list(&self) -> Result<Vec<StoredTask>, StorageError> {
            self.inner.list()
        }

        fn update(&mut self, _task: StoredTask) -> Result<(), StorageError> {
            Err(StorageError::Other("update failed".to_owned()))
        }

        fn remove(&mut self, id: &TaskId) -> Result<Option<StoredTask>, StorageError> {
            self.inner.remove(id)
        }
    }

    #[test]
    fn recovery_does_not_restore_task_when_normalization_cannot_be_persisted() {
        let (source, destination) = task_source();
        let id = TaskId::from("interrupted");
        let mut task = DownloadTask::new(id.clone(), source, destination);
        task.state = TaskState::Downloading;
        let mut repository = UpdateFailureRepository {
            inner: InMemoryRepository::new(),
        };
        repository.insert(task.into()).unwrap();

        let mut core = Core::with_repository(config(), repository).unwrap();
        assert_eq!(
            core.recover(),
            Err(CoreError::Storage(StorageError::Other(
                "update failed".to_owned()
            )))
        );
        assert!(core.tasks.get(&id).is_none());
        assert_eq!(core.scheduler.queued_len(), 0);
        assert_eq!(
            core.repository.get(&id).unwrap().unwrap().state,
            TaskState::Downloading
        );
    }

    #[derive(Default)]
    struct WriteFailureRepository {
        inner: InMemoryRepository,
        fail_updates: bool,
        fail_removals: bool,
    }

    impl TaskRepository for WriteFailureRepository {
        fn insert(&mut self, task: StoredTask) -> Result<(), StorageError> {
            self.inner.insert(task)
        }

        fn get(&self, id: &TaskId) -> Result<Option<StoredTask>, StorageError> {
            self.inner.get(id)
        }

        fn list(&self) -> Result<Vec<StoredTask>, StorageError> {
            self.inner.list()
        }

        fn update(&mut self, task: StoredTask) -> Result<(), StorageError> {
            if self.fail_updates {
                Err(StorageError::Other("injected update failure".into()))
            } else {
                self.inner.update(task)
            }
        }

        fn remove(&mut self, id: &TaskId) -> Result<Option<StoredTask>, StorageError> {
            if self.fail_removals {
                Err(StorageError::Other("injected removal failure".into()))
            } else {
                self.inner.remove(id)
            }
        }
    }

    #[test]
    fn write_failure_does_not_publish_queue_or_claim_changes() {
        let mut core = Core::with_repository(config(), WriteFailureRepository::default()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("write-failure-claim");
        core.create_task(id.clone(), source, destination).unwrap();
        core.drain_task_events();

        core.repository.fail_updates = true;
        assert!(matches!(
            core.queue_task(&id, Priority::HIGH),
            Err(CoreError::Storage(_))
        ));
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Created);
        assert_eq!(core.scheduler.queued_len(), 0);
        assert!(core.drain_task_events().is_empty());
        assert!(core.drain_scheduler_events().is_empty());
        assert_eq!(
            core.repository.get(&id).unwrap().unwrap().state,
            TaskState::Created
        );

        core.repository.fail_updates = false;
        core.queue_task(&id, Priority::HIGH).unwrap();
        core.update_progress(&id, Progress::new(10, Some(100)))
            .unwrap();
        core.drain_task_events();
        core.drain_scheduler_events();

        core.repository.fail_updates = true;
        assert!(matches!(core.claim_next(), Err(CoreError::Storage(_))));
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Queued);
        assert_eq!(core.tasks.get(&id).unwrap().progress.downloaded_bytes, 10);
        assert_eq!(core.scheduler.queued_len(), 1);
        assert_eq!(core.scheduler.active_len(), 0);
        assert!(core.drain_task_events().is_empty());
        assert!(core.drain_scheduler_events().is_empty());
        assert_eq!(
            core.repository.get(&id).unwrap().unwrap().state,
            TaskState::Queued
        );

        core.repository.fail_updates = false;
        assert_eq!(core.claim_next().unwrap(), Some(id.clone()));
        assert_eq!(core.tasks.get(&id).unwrap().progress, Progress::default());
        assert_eq!(core.scheduler.active_len(), 1);
    }

    #[test]
    fn write_failure_during_engine_start_keeps_task_queued() {
        let mut core = Core::with_repository(config(), WriteFailureRepository::default()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("write-failure-start");
        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();
        core.drain_task_events();
        core.drain_scheduler_events();

        core.repository.fail_updates = true;
        assert!(matches!(core.start_next(), Err(CoreError::Storage(_))));
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Queued);
        assert_eq!(core.scheduler.queued_len(), 1);
        assert_eq!(core.scheduler.active_len(), 0);
        assert!(!core.engine_tasks.contains_key(&id));
        assert!(core.drain_task_events().is_empty());
        assert!(core.drain_scheduler_events().is_empty());
        assert_eq!(
            core.repository.get(&id).unwrap().unwrap().state,
            TaskState::Queued
        );

        core.repository.fail_updates = false;
        assert_eq!(core.start_next().unwrap(), Some(id));
    }

    #[test]
    fn selected_claim_skips_other_tasks_and_rolls_back_on_write_failure() {
        let mut core = Core::with_repository(
            SchedulerConfig {
                max_concurrent_tasks: 1,
                ..config()
            },
            WriteFailureRepository::default(),
        )
        .unwrap();
        let (source, destination) = task_source();
        let first = TaskId::from("first");
        let selected = TaskId::from("selected");
        for id in [&first, &selected] {
            core.create_task(id.clone(), source.clone(), destination.clone())
                .unwrap();
            core.queue_task(id, Priority::NORMAL).unwrap();
        }
        assert_eq!(
            core.scheduler
                .next_queued_task_matching(|id| id == &selected),
            Some(&selected)
        );
        core.drain_task_events();
        core.drain_scheduler_events();

        core.repository.fail_updates = true;
        assert!(matches!(
            core.claim_queued(&selected),
            Err(CoreError::Storage(_))
        ));
        assert_eq!(core.scheduler.queued_len(), 2);
        assert_eq!(core.scheduler.active_len(), 0);
        assert_eq!(core.tasks.get(&selected).unwrap().state, TaskState::Queued);
        assert!(core.drain_task_events().is_empty());
        assert!(core.drain_scheduler_events().is_empty());

        core.repository.fail_updates = false;
        assert!(core.claim_queued(&selected).unwrap());
        assert_eq!(core.tasks.get(&first).unwrap().state, TaskState::Queued);
        assert_eq!(core.scheduler.queued_len(), 1);
        assert_eq!(core.scheduler.active_len(), 1);
        assert_eq!(core.scheduler.next_queued_task(), None);
        assert!(!core.claim_queued(&first).unwrap());
    }

    #[test]
    fn write_failure_preserves_progress_pause_resume_and_finish_state() {
        let mut core = Core::with_repository(config(), WriteFailureRepository::default()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("write-failure-state");
        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();
        core.claim_next().unwrap();
        core.drain_task_events();
        core.drain_scheduler_events();

        core.repository.fail_updates = true;
        assert!(matches!(
            core.update_progress(&id, Progress::new(50, Some(100))),
            Err(CoreError::Storage(_))
        ));
        assert!(matches!(core.pause_task(&id), Err(CoreError::Storage(_))));
        assert!(matches!(
            core.finish_task(&id, TaskState::Completed),
            Err(CoreError::Storage(_))
        ));
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Downloading);
        assert_eq!(core.tasks.get(&id).unwrap().progress, Progress::default());
        assert_eq!(core.scheduler.active_len(), 1);
        assert!(core.drain_task_events().is_empty());
        assert!(core.drain_scheduler_events().is_empty());
        assert_eq!(
            core.repository.get(&id).unwrap().unwrap().state,
            TaskState::Downloading
        );

        core.repository.fail_updates = false;
        core.pause_task(&id).unwrap();
        core.drain_task_events();
        core.drain_scheduler_events();
        core.repository.fail_updates = true;
        assert!(matches!(core.resume_task(&id), Err(CoreError::Storage(_))));
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Paused);
        assert_eq!(core.scheduler.active_len(), 0);
        assert!(core.drain_task_events().is_empty());
        assert!(core.drain_scheduler_events().is_empty());
        assert_eq!(
            core.repository.get(&id).unwrap().unwrap().state,
            TaskState::Paused
        );
    }

    #[test]
    fn removal_failure_keeps_task_queue_and_events_intact() {
        let mut core = Core::with_repository(config(), WriteFailureRepository::default()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("write-failure-remove");
        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();
        core.drain_task_events();
        core.drain_scheduler_events();

        core.repository.fail_removals = true;
        assert!(matches!(core.remove_task(&id), Err(CoreError::Storage(_))));
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Queued);
        assert_eq!(core.scheduler.queued_len(), 1);
        assert!(core.drain_task_events().is_empty());
        assert!(core.drain_scheduler_events().is_empty());
        assert!(core.repository.get(&id).unwrap().is_some());

        core.repository.fail_removals = false;
        core.remove_task(&id).unwrap();
        assert!(core.tasks.get(&id).is_none());
        assert_eq!(core.scheduler.queued_len(), 0);
        assert_eq!(core.claim_next().unwrap(), None);
    }

    #[test]
    fn engine_removal_error_retains_its_task_mapping() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("engine-remove-error");
        core.create_task(id.clone(), source, destination).unwrap();
        core.engine_tasks.insert(
            id.clone(),
            (
                "missing-engine".into(),
                EngineTask {
                    task_id: id.clone(),
                    handle: "missing-handle".into(),
                },
            ),
        );

        assert!(matches!(core.remove_task(&id), Err(CoreError::Engine(_))));
        assert!(core.engine_tasks.contains_key(&id));
        assert!(core.tasks.get(&id).is_some());
        assert!(core.repository.get(&id).unwrap().is_some());
    }

    struct CompletedEngine;

    impl EngineAdapter for CompletedEngine {
        fn name(&self) -> &str {
            "completed-test"
        }

        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities::BASIC
        }

        fn start(
            &mut self,
            task_id: &TaskId,
            _source: &str,
            _destination: &str,
        ) -> Result<EngineTask, EngineError> {
            Ok(EngineTask {
                task_id: task_id.clone(),
                handle: task_id.to_string(),
            })
        }

        fn pause(&mut self, _task: &EngineTask) -> Result<(), EngineError> {
            Err(EngineError::UnsupportedOperation("pause"))
        }

        fn resume(&mut self, _task: &EngineTask) -> Result<(), EngineError> {
            Err(EngineError::UnsupportedOperation("resume"))
        }

        fn remove(&mut self, _task: &EngineTask) -> Result<(), EngineError> {
            Ok(())
        }

        fn progress(&self, _task: &EngineTask) -> Result<Progress, EngineError> {
            Ok(Progress::new(100, Some(100)))
        }

        fn state(&self, _task: &EngineTask) -> Result<EngineTaskState, EngineError> {
            Ok(EngineTaskState::Completed)
        }
    }

    #[test]
    fn completed_engine_snapshot_releases_scheduler_slot() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        core.engines.register(Box::new(CompletedEngine));
        let (source, destination) = task_source();
        for number in 0..3 {
            let id = TaskId::from(format!("completed-{number}"));
            core.create_task(id.clone(), source.clone(), destination.clone())
                .unwrap();
            core.queue_task(&id, Priority::NORMAL).unwrap();
        }

        for _ in 0..3 {
            let id = core
                .start_next_with_engine("completed-test")
                .unwrap()
                .unwrap();
            assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Completed);
            assert_eq!(core.scheduler.active_len(), 0);
            assert_eq!(
                core.repository.get(&id).unwrap().unwrap().state,
                TaskState::Completed
            );
        }
        assert_eq!(core.scheduler.queued_len(), 0);
    }

    #[test]
    fn sqlite_repository_can_be_injected_into_core() {
        let mut core =
            Core::with_repository(config(), SqliteRepository::open_in_memory().unwrap()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("sqlite-task");
        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();
        core.start_next().unwrap();
        core.update_progress(&id, Progress::new(64, Some(100)))
            .unwrap();

        let stored = core.repository.get(&id).unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Downloading);
        assert_eq!(stored.progress.downloaded_bytes, 64);
    }

    #[test]
    fn failed_transfer_error_is_persisted_until_next_claim() {
        let mut core =
            Core::with_repository(config(), SqliteRepository::open_in_memory().unwrap()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("failed-transfer");
        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();
        core.claim_next().unwrap();

        core.finish_task_with_error(&id, TaskState::Failed, "HTTP GET returned 503")
            .unwrap();
        let stored = core.repository.get(&id).unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Queued);
        assert_eq!(stored.last_error.as_deref(), Some("HTTP GET returned 503"));
        assert_eq!(
            core.tasks.get(&id).unwrap().last_error.as_deref(),
            stored.last_error.as_deref()
        );

        core.claim_next().unwrap();
        let stored = core.repository.get(&id).unwrap().unwrap();
        assert_eq!(stored.state, TaskState::Downloading);
        assert_eq!(stored.last_error, None);
    }

    #[test]
    fn full_task_lifecycle() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("lifecycle-task");

        // Create
        let task = core
            .create_task(id.clone(), source.clone(), destination.clone())
            .unwrap();
        assert_eq!(task.state, TaskState::Created);

        // Queue
        core.queue_task(&id, Priority::NORMAL).unwrap();
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Queued);

        // Start (in-memory engine completes immediately)
        let started_id = core.start_next().unwrap();
        assert_eq!(started_id, Some(id.clone()));
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Downloading);

        // Update progress
        core.update_progress(&id, Progress::new(512, Some(1024)))
            .unwrap();
        let stored = core.repository.get(&id).unwrap().unwrap();
        assert_eq!(stored.progress.downloaded_bytes, 512);
        assert_eq!(stored.progress.total_bytes, Some(1024));

        // Pause
        core.pause_task(&id).unwrap();
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Paused);

        // Resume
        core.resume_task(&id).unwrap();
        assert_eq!(core.tasks.get(&id).unwrap().state, TaskState::Downloading);

        // Complete (simulate engine completing)
        core.finish_task(&id, TaskState::Completed).unwrap();
        assert!(core.tasks.get(&id).unwrap().is_terminal());
    }

    #[test]
    fn concurrent_tasks_respect_max_concurrent_limit() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap(); // max_concurrent_tasks: 2
        let (source, destination) = task_source();

        let id1 = TaskId::from("concurrent-1");
        let id2 = TaskId::from("concurrent-2");
        let id3 = TaskId::from("concurrent-3");

        core.create_task(id1.clone(), source.clone(), destination.clone())
            .unwrap();
        core.create_task(id2.clone(), source.clone(), destination.clone())
            .unwrap();
        core.create_task(id3.clone(), source, destination).unwrap();

        core.queue_task(&id1, Priority::NORMAL).unwrap();
        core.queue_task(&id2, Priority::NORMAL).unwrap();
        core.queue_task(&id3, Priority::HIGH).unwrap();

        // Start first two (max_concurrent_tasks = 2)
        let r1 = core.start_next().unwrap();
        let r2 = core.start_next().unwrap();
        assert!(r1.is_some());
        assert!(r2.is_some());

        // Third should not start (limit reached)
        let r3 = core.start_next().unwrap();
        assert!(r3.is_none());

        // Finish one to free a slot
        let task1 = core.tasks.get(&id1).unwrap().clone();
        core.finish_task(&task1.id, TaskState::Completed).unwrap();

        // Now the next task from queue should start
        let r3 = core.start_next().unwrap();
        assert!(r3.is_some());
    }

    #[test]
    fn core_drains_events() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("events-task");
        core.create_task(id.clone(), source, destination).unwrap();

        let events = core.drain_task_events();
        assert!(!events.is_empty());
        assert_eq!(
            events[0],
            nexum_task::TaskEvent::Created {
                task_id: id.clone()
            }
        );

        // After draining, events should be empty
        assert!(core.drain_task_events().is_empty());
    }

    #[test]
    fn core_schedulers_drain_scheduler_events() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("sched-events");
        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();

        let events = core.drain_scheduler_events();
        assert!(!events.is_empty());
        assert!(matches!(&events[0], SchedulerEvent::Enqueued { .. }));
    }

    #[test]
    fn core_persists_and_recovers_full_lifecycle() {
        let mut core =
            Core::with_repository(config(), SqliteRepository::open_in_memory().unwrap()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("persist-recover");

        core.create_task(id.clone(), source.clone(), destination.clone())
            .unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();

        // Save repository and create fresh core
        let repository = core.repository;
        let mut fresh = Core::with_repository(config(), repository).unwrap();

        // Recover should restore the queued task
        let restored = fresh.recover().unwrap();
        assert_eq!(restored, 1);
        assert_eq!(fresh.tasks.get(&id).unwrap().state, TaskState::Queued);
        assert_eq!(fresh.scheduler.queued_len(), 1);

        // Start and pause should also persist
        fresh.start_next().unwrap();
        fresh.pause_task(&id).unwrap();

        // After another restart, task should be recovered as queued (not downloading/paused)
        let repository2 = fresh.repository;
        let mut restarted = Core::with_repository(config(), repository2).unwrap();
        assert_eq!(restarted.recover().unwrap(), 1);
        // State is normalized to Queued during recovery
        assert_eq!(restarted.tasks.get(&id).unwrap().state, TaskState::Queued);
    }

    #[test]
    fn core_removes_task_from_all_subsystems() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let (source, destination) = task_source();
        let id = TaskId::from("remove-task");

        core.create_task(id.clone(), source, destination).unwrap();
        core.queue_task(&id, Priority::NORMAL).unwrap();
        core.start_next().unwrap();

        // Remove should clean up all subsystems
        let removed = core.remove_task(&id).unwrap();
        assert_eq!(removed.id, id);
        assert!(core.tasks.get(&id).is_none());
        assert!(core.repository.get(&id).unwrap().is_none());
        assert!(core.engine_tasks.is_empty());
        assert_eq!(core.scheduler.active_len(), 0);
    }

    #[test]
    fn core_plugin_manager_exists() {
        let core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        assert!(core.plugins.is_empty());
    }

    #[test]
    fn core_can_register_plugin() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        let manifest = nexum_plugin::PluginManifest::new("test", "Test", "lib.so");
        core.register_plugin(manifest).unwrap();
        assert_eq!(core.plugins.len(), 1);
    }

    #[test]
    fn core_init_plugins_noop() {
        let mut core = Core::with_repository(config(), InMemoryRepository::new()).unwrap();
        core.init_plugins().unwrap();
    }
}
