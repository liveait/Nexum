//! Absolute I/O budget for TLS handshakes and the managed startup probe.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) struct DeadlineSocket {
    socket: TcpStream,
    deadline: Option<Instant>,
    enabled: Option<Arc<AtomicBool>>,
}

impl DeadlineSocket {
    pub fn new(
        socket: TcpStream,
        deadline: Instant,
        enabled: Option<Arc<AtomicBool>>,
    ) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            deadline: Some(deadline),
            enabled,
        })
    }

    pub fn check_budget(&self) -> io::Result<()> {
        if self
            .enabled
            .as_ref()
            .is_some_and(|enabled| !enabled.load(Ordering::Acquire))
        {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "local service startup was cancelled",
            ));
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "connection deadline exceeded",
            ));
        }
        Ok(())
    }

    pub fn clear_budget(&mut self) -> io::Result<()> {
        self.check_budget()?;
        self.socket.set_nonblocking(false)?;
        self.deadline = None;
        self.enabled = None;
        Ok(())
    }

    fn wait_for_io(&self) -> io::Result<()> {
        self.check_budget()?;
        let remaining = self
            .deadline
            .expect("polling requires a deadline")
            .saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(Duration::from_millis(5)));
        self.check_budget()
    }
}

impl Read for DeadlineSocket {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            self.check_budget()?;
            match self.socket.read(bytes) {
                Err(error)
                    if self.deadline.is_some() && error.kind() == io::ErrorKind::WouldBlock =>
                {
                    self.wait_for_io()?
                }
                result => return result,
            }
        }
    }
}

impl Write for DeadlineSocket {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            self.check_budget()?;
            match self.socket.write(bytes) {
                Err(error)
                    if self.deadline.is_some() && error.kind() == io::ErrorKind::WouldBlock =>
                {
                    self.wait_for_io()?
                }
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check_budget()?;
        self.socket.flush()
    }
}
