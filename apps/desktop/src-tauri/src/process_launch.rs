//! Serialize this process's native launch windows, including stdio pipe creation.
//! Child execution, protocol traffic and waits happen after releasing the lock.

use std::io;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, TryLockError};
use std::time::{Duration, Instant};

static PROCESS_LAUNCH: Mutex<()> = Mutex::new(());
const WAIT_INTERVAL: Duration = Duration::from_millis(5);

pub fn spawn(command: &mut Command) -> io::Result<Child> {
    let _launch = PROCESS_LAUNCH
        .lock()
        .map_err(|_| io::Error::other("native process launch synchronization is unavailable"))?;
    command.spawn()
}

pub fn spawn_managed(
    command: &mut Command,
    deadline: Instant,
    enabled: &AtomicBool,
) -> io::Result<Child> {
    spawn_managed_with_lock(&PROCESS_LAUNCH, command, deadline, enabled)
}

fn spawn_managed_with_lock(
    launch: &Mutex<()>,
    command: &mut Command,
    deadline: Instant,
    enabled: &AtomicBool,
) -> io::Result<Child> {
    loop {
        check_budget(deadline, enabled)?;
        match launch.try_lock() {
            Ok(_launch) => {
                // Cancellation/deadline may change between the first check and acquisition.
                check_budget(deadline, enabled)?;
                return command.spawn();
            }
            Err(TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                std::thread::sleep(remaining.min(WAIT_INTERVAL));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(io::Error::other(
                    "native process launch synchronization is unavailable",
                ));
            }
        }
    }
}

fn check_budget(deadline: Instant, enabled: &AtomicBool) -> io::Result<()> {
    if !enabled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "local service startup was cancelled",
        ));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "local service startup deadline exceeded",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contended_launch_expires_before_invoking_the_command() {
        let launch = Mutex::new(());
        let _reservation = launch.lock().unwrap();
        let enabled = AtomicBool::new(true);
        let before = Instant::now();
        // An empty executable would produce a different error if spawn were reached.
        let error = spawn_managed_with_lock(
            &launch,
            &mut Command::new(""),
            before + Duration::from_millis(80),
            &enabled,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(before.elapsed() >= Duration::from_millis(80));
        assert!(before.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn cancellation_interrupts_a_contended_launch_before_invoking_the_command() {
        let launch = Mutex::new(());
        let _reservation = launch.lock().unwrap();
        let enabled = AtomicBool::new(true);
        let before = Instant::now();
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                spawn_managed_with_lock(
                    &launch,
                    &mut Command::new(""),
                    before + Duration::from_secs(2),
                    &enabled,
                )
            });
            enabled.store(false, Ordering::Release);
            let error = waiter.join().unwrap().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
            assert!(before.elapsed() < Duration::from_millis(750));
        });
    }
}
