//! Single-connection plain FTP reception into a caller-owned file.

mod reply;

use std::fmt;
use std::fs::File;
use std::future::Future;
use std::io::{self, Seek, Write};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nexum_domain::Progress;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::HttpTransferControl;
use reply::{Reply, read_reply};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const DATA_TIMEOUT: Duration = Duration::from_secs(8);
const CONTROL_POLL: Duration = Duration::from_millis(100);

pub enum FtpCredentials<'a> {
    Anonymous,
    Password {
        username: &'a str,
        password: &'a str,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FtpStage {
    Connect,
    Greeting,
    Login,
    BinaryMode,
    Size,
    Passive,
    Retrieve,
    Data,
    Completion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FtpReceiveError {
    Cancelled,
    InvalidRequest,
    ControlAlreadyUsed,
    InvalidTarget,
    LocalIo(io::ErrorKind),
    NetworkIo(FtpStage, io::ErrorKind),
    TimedOut(FtpStage),
    InvalidReply(FtpStage),
    Rejected(FtpStage, u16),
    LengthMismatch,
    SourceSizeChanged,
}

impl fmt::Display for FtpReceiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("FTP receive cancelled"),
            Self::InvalidRequest => formatter.write_str("invalid FTP request"),
            Self::ControlAlreadyUsed => formatter.write_str("FTP control already used"),
            Self::InvalidTarget => formatter.write_str("FTP requires an empty regular file"),
            Self::LocalIo(kind) => write!(formatter, "FTP file I/O failed ({kind})"),
            Self::NetworkIo(stage, kind) => write!(formatter, "FTP {stage:?} I/O failed ({kind})"),
            Self::TimedOut(stage) => write!(formatter, "FTP {stage:?} timed out"),
            Self::InvalidReply(stage) => write!(formatter, "invalid FTP {stage:?} reply"),
            Self::Rejected(stage, code) => write!(formatter, "FTP {stage:?} rejected ({code})"),
            Self::LengthMismatch => formatter.write_str("FTP data length does not match SIZE"),
            Self::SourceSizeChanged => formatter.write_str("FTP source SIZE changed"),
        }
    }
}

impl std::error::Error for FtpReceiveError {}

#[derive(Default)]
struct ControlState {
    worker: HttpTransferControl,
    started: AtomicBool,
}

/// Shared pause and cancellation state for one receive attempt.
#[derive(Clone, Default)]
pub struct FtpTransferControl(Arc<ControlState>);

impl FtpTransferControl {
    /// Waits for the worker to stop writing. Returns whether it is parked.
    pub fn request_pause(&self) -> Result<bool, FtpReceiveError> {
        self.0
            .worker
            .request_pause()
            .map_err(|_| FtpReceiveError::Cancelled)?;
        Ok(self.0.worker.is_paused())
    }

    pub fn resume(&self) -> Result<(), FtpReceiveError> {
        self.0
            .worker
            .resume()
            .map_err(|_| FtpReceiveError::Cancelled)
    }

    pub fn cancel(&self) {
        self.0.worker.cancel();
    }

    pub fn progress(&self) -> Option<Progress> {
        self.0.worker.session_progress()
    }

    pub fn is_finished(&self) -> bool {
        self.0.worker.is_finished()
    }

    fn checkpoint(&self) -> Result<(), FtpReceiveError> {
        self.0
            .worker
            .wait_if_paused()
            .map_err(|_| FtpReceiveError::Cancelled)
    }

    fn finish_success(&self) -> Result<(), FtpReceiveError> {
        let (lock, wake) = &*self.0.worker.inner;
        loop {
            self.checkpoint()?;
            let mut state = lock.lock().expect("FTP transfer control mutex poisoned");
            if state.cancelled {
                return Err(FtpReceiveError::Cancelled);
            }
            if !state.pause_requested {
                state.finished = true;
                wake.notify_all();
                return Ok(());
            }
        }
    }

    async fn wait<F, T>(
        &self,
        stage: FtpStage,
        budget: Duration,
        future: F,
    ) -> Result<T, FtpReceiveError>
    where
        F: Future<Output = Result<T, FtpReceiveError>>,
    {
        let mut future = std::pin::pin!(future);
        let mut remaining = budget;
        loop {
            self.checkpoint()?;
            let started = Instant::now();
            let polled = tokio::time::timeout(remaining.min(CONTROL_POLL), future.as_mut()).await;
            remaining = remaining.saturating_sub(started.elapsed());
            self.checkpoint()?;
            if remaining.is_zero() {
                return Err(FtpReceiveError::TimedOut(stage));
            }
            if let Ok(result) = polled {
                return result;
            }
        }
    }
}

struct FinishOnDrop<'a>(&'a HttpTransferControl);

impl Drop for FinishOnDrop<'_> {
    fn drop(&mut self) {
        self.0.mark_finished();
    }
}

/// Receives one file on a blocking worker owned by the caller.
///
/// The endpoint must already be resolved. The caller owns a writable regular
/// file at offset zero, with zero length and no concurrent writer. On error or
/// cancellation the bytes already written stay in that file. The worker never
/// opens, moves, removes or publishes a path. Join the worker before releasing
/// the file or using its final result. Credentials are sent in plain FTP.
pub fn receive(
    endpoint: SocketAddr,
    path: &str,
    credentials: FtpCredentials<'_>,
    file: &mut File,
    control: &FtpTransferControl,
) -> Result<Progress, FtpReceiveError> {
    if control.0.started.swap(true, Ordering::AcqRel) {
        return Err(FtpReceiveError::ControlAlreadyUsed);
    }
    let _finish = FinishOnDrop(&control.0.worker);
    control.checkpoint()?;
    let (username, password) = match credentials {
        FtpCredentials::Anonymous => ("anonymous", "anonymous@"),
        FtpCredentials::Password { username, password } => (username, password),
    };
    if endpoint.port() == 0
        || !valid_arg(path, 32768, false)
        || !valid_arg(username, 4096, false)
        || !valid_arg(password, 65536, true)
    {
        return Err(FtpReceiveError::InvalidRequest);
    }
    let metadata = file.metadata().map_err(local_io)?;
    if !metadata.is_file() || metadata.len() != 0 || file.stream_position().map_err(local_io)? != 0
    {
        return Err(FtpReceiveError::InvalidTarget);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(network_io(FtpStage::Connect))?;
    let progress = runtime.block_on(receive_bytes(
        endpoint, path, username, password, file, control,
    ))?;
    control.checkpoint()?;
    file.sync_all().map_err(local_io)?;
    control.0.worker.record_session_progress(progress.clone());
    control.finish_success()?;
    Ok(progress)
}

fn valid_arg(value: &str, limit: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= limit
        && !value.chars().any(char::is_control)
}

fn local_io(error: io::Error) -> FtpReceiveError {
    FtpReceiveError::LocalIo(error.kind())
}

fn network_io(stage: FtpStage) -> impl FnOnce(io::Error) -> FtpReceiveError {
    move |error| FtpReceiveError::NetworkIo(stage, error.kind())
}

async fn connect(
    address: SocketAddr,
    stage: FtpStage,
    control: &FtpTransferControl,
) -> Result<TcpStream, FtpReceiveError> {
    control
        .wait(stage, CONTROL_TIMEOUT, async {
            TcpStream::connect(address).await.map_err(network_io(stage))
        })
        .await
}

struct Connection<'a> {
    stream: BufReader<TcpStream>,
    control: &'a FtpTransferControl,
}

impl Connection<'_> {
    async fn command(
        &mut self,
        stage: FtpStage,
        verb: &str,
        argument: Option<&str>,
    ) -> Result<Reply, FtpReceiveError> {
        self.control
            .wait(stage, CONTROL_TIMEOUT, async {
                let socket = self.stream.get_mut();
                socket
                    .write_all(verb.as_bytes())
                    .await
                    .map_err(network_io(stage))?;
                if let Some(argument) = argument {
                    socket.write_all(b" ").await.map_err(network_io(stage))?;
                    socket
                        .write_all(argument.as_bytes())
                        .await
                        .map_err(network_io(stage))?;
                }
                socket.write_all(b"\r\n").await.map_err(network_io(stage))?;
                read_reply(&mut self.stream, stage).await
            })
            .await
    }

    async fn login(&mut self, username: &str, password: &str) -> Result<(), FtpReceiveError> {
        let stage = FtpStage::Greeting;
        self.control
            .wait(stage, CONTROL_TIMEOUT, async {
                let mut reply = read_reply(&mut self.stream, stage).await?;
                if reply.code == 120 {
                    reply = read_reply(&mut self.stream, stage).await?;
                }
                reply.expect(stage, &[220])
            })
            .await?;
        let stage = FtpStage::Login;
        let reply = self.command(stage, "USER", Some(username)).await?;
        if reply.code == 331 {
            self.command(stage, "PASS", Some(password))
                .await?
                .expect(stage, &[230])?;
        } else {
            reply.expect(stage, &[230])?;
        }
        self.command(FtpStage::BinaryMode, "TYPE", Some("I"))
            .await?
            .expect(FtpStage::BinaryMode, &[200])
    }

    async fn size(&mut self, path: &str) -> Result<Option<u64>, FtpReceiveError> {
        let stage = FtpStage::Size;
        let reply = self.command(stage, "SIZE", Some(path)).await?;
        if matches!(reply.code, 500 | 502 | 504 | 550) {
            return Ok(None);
        }
        reply.expect(stage, &[213])?;
        reply.size().map(Some)
    }

    async fn passive_address(&mut self) -> Result<SocketAddr, FtpReceiveError> {
        let stage = FtpStage::Passive;
        let mut address = self
            .stream
            .get_ref()
            .peer_addr()
            .map_err(network_io(stage))?;
        let reply = self.command(stage, "EPSV", None).await?;
        let port = if matches!(reply.code, 500 | 502) && address.is_ipv4() {
            let reply = self.command(stage, "PASV", None).await?;
            reply.expect(stage, &[227])?;
            reply.pasv_port()?
        } else {
            reply.expect(stage, &[229])?;
            reply.epsv_port()?
        };
        address.set_port(port);
        Ok(address)
    }
}

async fn receive_bytes(
    endpoint: SocketAddr,
    path: &str,
    username: &str,
    password: &str,
    file: &mut File,
    control: &FtpTransferControl,
) -> Result<Progress, FtpReceiveError> {
    let mut connection = Connection {
        stream: BufReader::new(connect(endpoint, FtpStage::Connect, control).await?),
        control,
    };
    connection.login(username, password).await?;
    let total = connection.size(path).await?;
    control
        .0
        .worker
        .record_session_progress(Progress::new(0, total));
    let address = connection.passive_address().await?;
    let mut data = connect(address, FtpStage::Passive, control).await?;
    connection
        .command(FtpStage::Retrieve, "RETR", Some(path))
        .await?
        .expect(FtpStage::Retrieve, &[125, 150])?;

    let received = write_data(&mut data, file, control, total).await?;
    drop(data);
    let stage = FtpStage::Completion;
    control
        .wait(
            stage,
            CONTROL_TIMEOUT,
            read_reply(&mut connection.stream, stage),
        )
        .await?
        .expect(stage, &[226, 250])?;
    if let Some(expected) = total {
        if expected != received {
            return Err(FtpReceiveError::LengthMismatch);
        }
        if connection.size(path).await? != Some(expected) {
            return Err(FtpReceiveError::SourceSizeChanged);
        }
    }
    Ok(Progress::new(received, Some(received)))
}

async fn write_data(
    socket: &mut TcpStream,
    file: &mut File,
    control: &FtpTransferControl,
    total: Option<u64>,
) -> Result<u64, FtpReceiveError> {
    let mut received = 0u64;
    let mut buffer = [0u8; 32 * 1024];
    loop {
        let read = control
            .wait(FtpStage::Data, DATA_TIMEOUT, async {
                socket
                    .read(&mut buffer)
                    .await
                    .map_err(network_io(FtpStage::Data))
            })
            .await?;
        if read == 0 {
            return Ok(received);
        }
        let end = received
            .checked_add(read as u64)
            .ok_or(FtpReceiveError::LengthMismatch)?;
        if total.is_some_and(|total| end > total) {
            return Err(FtpReceiveError::LengthMismatch);
        }
        let mut pending = &buffer[..read];
        while !pending.is_empty() {
            control.checkpoint()?;
            match file.write(pending) {
                Ok(0) => return Err(FtpReceiveError::LocalIo(io::ErrorKind::WriteZero)),
                Ok(written) => {
                    received += written as u64;
                    pending = &pending[written..];
                    control
                        .0
                        .worker
                        .record_session_progress(Progress::new(received, total));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(local_io(error)),
            }
        }
    }
}
