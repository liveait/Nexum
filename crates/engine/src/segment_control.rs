//! Private participant barriers for the disabled segmented HTTP prototype.
//!
//! All workers and the progress coordinator register before any worker starts.
//! Acknowledging pause requires every live participant to reach a checkpoint.
//! Cooperative network and pacing waits observe control at short intervals;
//! synchronous filesystem operations and user callbacks must return themselves.

use super::{EngineError, HttpTransferControl, HttpTransferControlState};
use std::future::Future;
use std::time::{Duration, Instant};

pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct Slot {
    active: bool,
    parked_epoch: Option<u64>,
}

pub(super) struct SegmentState {
    slots: Vec<Slot>,
    epoch: u64,
    failure: Option<EngineError>,
}

impl SegmentState {
    pub(super) fn start_pause(&mut self) {
        // Wrapping is harmless: live participants overwrite their parked epoch
        // under the same lock before acknowledging a fresh request.
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn all_parked(&self) -> bool {
        self.failure.is_none()
            && self
                .slots
                .iter()
                .all(|slot| !slot.active || slot.parked_epoch == Some(self.epoch))
    }
}

fn update_paused(state: &mut HttpTransferControlState) {
    if let Some(group) = &state.segments {
        state.paused = state.pause_requested && !state.cancelled && group.all_parked();
    }
}

pub(super) struct SegmentGroup {
    control: HttpTransferControl,
}

impl SegmentGroup {
    pub(super) fn begin(
        control: &HttpTransferControl,
        count: usize,
    ) -> Result<(Self, Vec<Participant>), EngineError> {
        let (lock, _) = &*control.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if state.segments.is_some() || state.finished || state.committed {
            return Err(EngineError::Failed(
                "HTTP transfer control is already in use or finished".into(),
            ));
        }
        if state.cancelled {
            return Err(EngineError::Cancelled);
        }
        state.segments = Some(SegmentState {
            slots: (0..count)
                .map(|_| Slot {
                    active: true,
                    parked_epoch: None,
                })
                .collect(),
            epoch: 1,
            failure: None,
        });
        state.paused = false;
        Ok((
            Self {
                control: control.clone(),
            },
            (0..count)
                .map(|index| Participant {
                    control: control.clone(),
                    index,
                })
                .collect(),
        ))
    }

    pub(super) fn fail(&self, error: EngineError) {
        fail(&self.control, error);
    }

    pub(super) fn error(&self) -> Option<EngineError> {
        self.control.with_state(|state| {
            state
                .segments
                .as_ref()
                .and_then(|group| group.failure.clone())
        })
    }
}

impl Drop for SegmentGroup {
    fn drop(&mut self) {
        let (lock, wake) = &*self.control.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        debug_assert!(
            state
                .segments
                .as_ref()
                .is_none_or(|group| group.slots.iter().all(|slot| !slot.active))
        );
        state.segments = None;
        state.paused = false;
        wake.notify_all();
    }
}

fn fail(control: &HttpTransferControl, error: EngineError) {
    let (lock, wake) = &*control.inner;
    let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
    if let Some(group) = &mut state.segments
        && group.failure.is_none()
    {
        group.failure = Some(error);
    }
    state.paused = false;
    wake.notify_all();
}

pub(super) fn run_runtime(
    index: usize,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<u64, EngineError>,
) -> Result<u64, EngineError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|error| {
            EngineError::Failed(format!("could not start segment-{index} runtime: {error}"))
        })?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&runtime)));
    // Request futures have exited before teardown. A running OS resolver can
    // finish independently; it has no staging/participant references and must
    // not keep this segment worker's join blocked indefinitely.
    runtime.shutdown_timeout(POLL_INTERVAL);
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Thread creation is injected only into this private helper for direct failure
/// tests. The production caller always uses `Builder::spawn` below.
pub(super) fn spawn_worker(
    index: usize,
    participant: Participant,
    sender: std::sync::mpsc::Sender<Result<u64, EngineError>>,
    run: impl FnOnce(&mut Participant) -> Result<u64, EngineError> + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    spawn_worker_with(index, participant, sender, run, |task| {
        std::thread::Builder::new()
            .name(format!("http-segment-{index}"))
            .spawn(task)
    })
}

fn spawn_worker_with(
    index: usize,
    mut participant: Participant,
    sender: std::sync::mpsc::Sender<Result<u64, EngineError>>,
    run: impl FnOnce(&mut Participant) -> Result<u64, EngineError> + Send + 'static,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<std::thread::JoinHandle<()>>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    let control = participant.control.clone();
    let result = spawn(Box::new(move || {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut participant)))
                .unwrap_or_else(|_| {
                    Err(EngineError::Failed(format!(
                        "segment worker {index} panicked"
                    )))
                });
        if let Err(error) = &result {
            participant.fail(error.clone());
        }
        let _ = sender.send(result);
    }));
    if let Err(error) = &result {
        fail(
            &control,
            EngineError::Failed(format!("could not spawn segment worker {index}: {error}")),
        );
    }
    result
}

pub(super) struct Participant {
    control: HttpTransferControl,
    index: usize,
}

impl Participant {
    pub(super) fn fail(&self, error: EngineError) {
        fail(&self.control, error);
    }

    pub(super) fn checkpoint(&mut self) -> Result<(), EngineError> {
        let (lock, wake) = &*self.control.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        loop {
            if let Some(error) = state
                .segments
                .as_ref()
                .and_then(|group| group.failure.clone())
            {
                return Err(error);
            }
            if state.cancelled {
                return Err(EngineError::Cancelled);
            }
            let pause = state.pause_requested;
            let was_paused = state.paused;
            let group = state.segments.as_mut().expect("registered segment group");
            let parked = pause.then_some(group.epoch);
            let changed = group.slots[self.index].parked_epoch != parked;
            group.slots[self.index].parked_epoch = parked;
            update_paused(&mut state);
            if changed || was_paused != state.paused {
                wake.notify_all();
            }
            if !pause {
                return Ok(());
            }
            state = wake
                .wait(state)
                .expect("HTTP transfer control mutex poisoned while waiting");
        }
    }

    /// Retain one future across control polls: a delayed header response must
    /// not cause duplicate HTTP requests. Paused time is excluded from the
    /// caller's optional inactivity budget.
    pub(super) async fn wait<F: Future>(
        &mut self,
        future: F,
        inactivity: Option<Duration>,
    ) -> Result<Option<F::Output>, EngineError> {
        let mut future = std::pin::pin!(future);
        let mut active_wait = Duration::ZERO;
        loop {
            self.checkpoint()?;
            let started = Instant::now();
            match tokio::time::timeout(POLL_INTERVAL, future.as_mut()).await {
                Ok(output) => {
                    self.checkpoint()?;
                    return Ok(Some(output));
                }
                Err(_) => {
                    active_wait = active_wait.saturating_add(started.elapsed());
                    self.checkpoint()?;
                    if inactivity.is_some_and(|bound| active_wait >= bound) {
                        return Ok(None);
                    }
                }
            }
        }
    }

    pub(super) fn wait_for_budget(&mut self, bytes: u64) -> Result<(), EngineError> {
        self.checkpoint()?;
        let (lock, _) = &*self.control.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        let Some(cap) = state.bytes_per_second else {
            state.pacer_sent = state.pacer_sent.saturating_add(bytes);
            return Ok(());
        };
        let now = Instant::now();
        let started = *state.pacer_started.get_or_insert(now);
        state.pacer_sent = state.pacer_sent.saturating_add(bytes);
        let virtual_seconds = state.pacer_sent as f64 / cap.max(1) as f64;
        let wait = virtual_seconds - now.duration_since(started).as_secs_f64();
        drop(state);
        if wait > 0.0 {
            let until = Instant::now() + Duration::from_secs_f64(wait);
            loop {
                self.checkpoint()?;
                let now = Instant::now();
                if now >= until {
                    break;
                }
                let (lock, wake) = &*self.control.inner;
                let state = lock.lock().expect("HTTP transfer control mutex poisoned");
                // Recheck under the condition-variable lock so pause/cancel/
                // failure cannot be lost between checkpoint and wait.
                if state.pause_requested
                    || state.cancelled
                    || state
                        .segments
                        .as_ref()
                        .is_some_and(|group| group.failure.is_some())
                {
                    drop(state);
                    continue;
                }
                let _ = wake
                    .wait_timeout(state, (until - now).min(POLL_INTERVAL))
                    .expect("HTTP transfer control mutex poisoned while pacing");
            }
        }
        self.checkpoint()
    }
}

impl Drop for Participant {
    fn drop(&mut self) {
        let (lock, wake) = &*self.control.inner;
        let mut state = lock.lock().expect("HTTP transfer control mutex poisoned");
        if let Some(group) = &mut state.segments {
            group.slots[self.index].active = false;
            group.slots[self.index].parked_epoch = None;
            if std::thread::panicking() && group.failure.is_none() {
                group.failure = Some(EngineError::Failed("segment participant panicked".into()));
            }
        }
        update_paused(&mut state);
        wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    fn wait_for_reservation(control: &HttpTransferControl) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !control.with_state(|state| state.pacer_sent > 0) {
            assert!(
                Instant::now() < deadline,
                "worker must reserve its pacing budget"
            );
            thread::yield_now();
        }
    }

    #[test]
    fn pacing_pause_waits_for_all_slots_and_cancel_wakes_every_slot() {
        let control = HttpTransferControl::new();
        control.set_bytes_per_second(Some(1));
        let (group, mut slots) = SegmentGroup::begin(&control, 2).unwrap();
        let mut coordinator = slots.remove(0);
        let worker = slots.remove(0);
        let (result_tx, result_rx) = mpsc::channel();
        let handle = spawn_worker(0, worker, result_tx, |worker| {
            worker.wait_for_budget(1000)?;
            Ok(1000)
        })
        .unwrap();
        wait_for_reservation(&control);
        let pause = control.clone();
        let (paused_tx, paused_rx) = mpsc::channel();
        let requester = thread::spawn(move || {
            paused_tx.send(pause.request_pause()).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while !control.is_pause_requested() {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        // The worker may park, but this coordinator has not reached a boundary.
        assert!(paused_rx.recv_timeout(Duration::from_millis(100)).is_err());
        let cancel = control.clone();
        let canceller = thread::spawn(move || {
            let result = paused_rx.recv_timeout(Duration::from_secs(2));
            cancel.cancel();
            result
        });
        assert_eq!(coordinator.checkpoint(), Err(EngineError::Cancelled));
        assert_eq!(
            result_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(EngineError::Cancelled)
        );
        handle.join().unwrap();
        requester.join().unwrap();
        canceller.join().unwrap().unwrap().unwrap();
        drop(coordinator);
        drop(group);
        assert!(control.with_state(|state| state.segments.is_none()));
    }

    #[test]
    fn original_failure_interrupts_pacing_without_setting_public_cancellation() {
        let control = HttpTransferControl::new();
        control.set_bytes_per_second(Some(1));
        let (group, mut slots) = SegmentGroup::begin(&control, 1).unwrap();
        let worker = slots.remove(0);
        let (tx, rx) = mpsc::channel();
        let handle = spawn_worker(0, worker, tx, |worker| {
            worker.wait_for_budget(1000)?;
            Ok(1000)
        })
        .unwrap();
        wait_for_reservation(&control);
        let error = EngineError::Failed("original segment failure".into());
        group.fail(error.clone());
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(error.clone())
        );
        handle.join().unwrap();
        assert_eq!(group.error(), Some(error));
        assert!(!control.is_cancelled());
        drop(group);
    }

    #[test]
    fn spawn_failure_aborts_existing_worker_and_drops_unstarted_registration() {
        let control = HttpTransferControl::new();
        control.set_bytes_per_second(Some(1));
        let (group, mut slots) = SegmentGroup::begin(&control, 2).unwrap();
        let worker = slots.remove(0);
        let unstarted = slots.remove(0);
        let (tx, rx) = mpsc::channel();
        let handle = spawn_worker(0, worker, tx.clone(), |worker| {
            worker.wait_for_budget(1000)?;
            Ok(1000)
        })
        .unwrap();
        wait_for_reservation(&control);
        let result = spawn_worker_with(
            1,
            unstarted,
            tx,
            |_| Ok(0),
            |task| {
                drop(task);
                Err(std::io::Error::other("injected spawn failure"))
            },
        );
        assert!(result.is_err());
        let error = rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err();
        handle.join().unwrap();
        assert!(error.to_string().contains("injected spawn failure"));
        assert!(control.with_state(|state| {
            state
                .segments
                .as_ref()
                .unwrap()
                .slots
                .iter()
                .all(|slot| !slot.active)
        }));
        assert!(
            !control.is_finished(),
            "only the joined transfer owner marks completion"
        );
        drop(group);
    }

    #[test]
    fn panic_is_reported_and_interrupts_peer_before_join() {
        let control = HttpTransferControl::new();
        control.set_bytes_per_second(Some(1));
        let (group, mut slots) = SegmentGroup::begin(&control, 2).unwrap();
        let worker = slots.remove(0);
        let panicking = slots.remove(0);
        let (tx, rx) = mpsc::channel();
        let handle = spawn_worker(0, worker, tx.clone(), |worker| {
            worker.wait_for_budget(1000)?;
            Ok(1000)
        })
        .unwrap();
        wait_for_reservation(&control);
        let panic_handle =
            spawn_worker(1, panicking, tx, |_| panic!("injected worker panic")).unwrap();
        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err();
        let second = rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err();
        handle.join().unwrap();
        panic_handle.join().unwrap();
        assert_eq!(first, second);
        assert!(first.to_string().contains("segment worker 1 panicked"));
        assert_eq!(group.error(), Some(first));
        assert!(!control.is_cancelled());
        drop(group);
    }

    #[test]
    fn runtime_teardown_does_not_wait_for_unrelated_blocking_resolver() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (ended_tx, ended_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = run_runtime(0, |runtime| {
                runtime.spawn_blocking(move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                    ended_tx.send(()).unwrap();
                });
                entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(0)
            });
            result_tx.send(result).unwrap();
        });
        let early = result_rx.recv_timeout(Duration::from_secs(1));
        release_tx.send(()).unwrap();
        ended_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert_eq!(early.unwrap().unwrap(), 0);
    }

    #[test]
    fn rapid_resume_and_repause_acknowledges_each_generation() {
        let control = HttpTransferControl::new();
        let (group, mut slots) = SegmentGroup::begin(&control, 2).unwrap();
        let mut coordinator = slots.remove(0);
        let mut worker = slots.remove(0);
        let worker_thread = thread::spawn(move || {
            while worker.checkpoint().is_ok() {
                thread::yield_now();
            }
        });
        let pause = control.clone();
        let (tx, rx) = mpsc::channel();
        let requester = thread::spawn(move || {
            let result = (|| {
                pause.request_pause()?;
                pause.resume()?;
                pause.request_pause()?;
                Ok::<_, EngineError>(())
            })();
            let _ = tx.send(result);
        });
        let cancel = control.clone();
        let watchdog = thread::spawn(move || {
            let result = rx.recv_timeout(Duration::from_secs(2));
            cancel.cancel();
            result
        });
        assert_eq!(
            loop {
                if let Err(error) = coordinator.checkpoint() {
                    break error;
                }
                thread::yield_now();
            },
            EngineError::Cancelled
        );
        let result = watchdog.join().unwrap();
        requester.join().unwrap();
        worker_thread.join().unwrap();
        result.unwrap().unwrap();
        assert!(!control.is_paused());
        drop(coordinator);
        drop(group);
    }
}
