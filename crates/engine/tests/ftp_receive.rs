use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nexum_engine::ftp::{FtpCredentials, FtpReceiveError, FtpStage, FtpTransferControl, receive};
#[path = "support/ftp_completion.rs"]
mod completion;
#[path = "support/ftp_resume.rs"]
mod resume;
#[path = "support/ftp_tail.rs"]
mod tail;

const WAIT: Duration = Duration::from_secs(12);
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct HeldFile(PathBuf);

impl HeldFile {
    fn new() -> (Self, File) {
        let directory = std::env::temp_dir().join(format!(
            "nexum-ftp-test-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let held = Self(directory);
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(held.0.join("file.part"))
            .unwrap();
        (held, file)
    }
}

impl Drop for HeldFile {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct Origin(BufReader<TcpStream>);

impl Origin {
    fn command(&mut self, expected: &str) {
        let mut command = String::new();
        self.0.read_line(&mut command).unwrap();
        assert_eq!(command, format!("{expected}\r\n"));
    }

    fn reply(&mut self, value: &str) {
        self.0.get_mut().write_all(value.as_bytes()).unwrap();
    }

    fn anonymous_login(&mut self, size: u64) {
        self.login();
        self.size(&format!("213 {size}\r\n"));
    }

    fn login(&mut self) {
        self.reply("220-Nexum fixture\r\n220 ready\r\n");
        self.command("USER anonymous");
        self.reply("331 Password required\r\n");
        self.command("PASS anonymous@");
        self.reply("230 Logged in\r\n");
        self.command("TYPE I");
        self.reply("200 Binary\r\n");
    }

    fn size(&mut self, response: &str) {
        self.command("SIZE pub/a b.bin");
        self.reply(response);
    }

    fn open_data(&mut self) -> TcpStream {
        self.command("EPSV");
        let mut endpoint = self.0.get_ref().local_addr().unwrap();
        endpoint.set_port(0);
        let listener = TcpListener::bind(endpoint).unwrap();
        self.reply(&format!(
            "229 Entering Extended Passive Mode ())){}))\r\n",
            listener.local_addr().unwrap().port()
        ));
        self.start_retr(&listener)
    }

    fn start_retr(&mut self, listener: &TcpListener) -> TcpStream {
        self.command("RETR pub/a b.bin");
        self.reply("150 Opening data\r\n");
        accept(listener)
    }

    fn expect_closed(&mut self) {
        assert_eq!(self.0.read(&mut [0]).unwrap(), 0);
    }
}

fn spawn_origin(script: impl FnOnce(Origin) + Send + 'static) -> (SocketAddr, JoinHandle<()>) {
    spawn_origin_on("127.0.0.1:0", script)
}

fn spawn_origin_on(
    address: &str,
    script: impl FnOnce(Origin) + Send + 'static,
) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind(address).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        script(Origin(BufReader::new(accept(&listener))));
    });
    (endpoint, worker)
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let expires = Instant::now() + WAIT;
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < expires,
                    "origin did not receive a connection"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("origin accept failed: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    stream.set_write_timeout(Some(WAIT)).unwrap();
    stream
}

fn fetch(
    endpoint: SocketAddr,
    file: &mut File,
    control: &FtpTransferControl,
) -> Result<nexum_domain::Progress, FtpReceiveError> {
    receive(
        endpoint,
        "pub/a b.bin",
        FtpCredentials::Anonymous,
        file,
        control,
    )
}

fn wait_progress(control: &FtpTransferControl, bytes: u64) {
    let expires = Instant::now() + Duration::from_secs(3);
    while control
        .progress()
        .map_or(0, |progress| progress.downloaded_bytes)
        < bytes
    {
        assert!(!control.is_finished() && Instant::now() < expires);
        thread::sleep(Duration::from_millis(5));
    }
}

fn pause(control: &FtpTransferControl) {
    let remote = control.clone();
    let (sender, result) = mpsc::channel();
    let waiter = thread::spawn(move || sender.send(remote.request_pause()));
    let paused = result.recv_timeout(Duration::from_secs(3));
    if paused.is_err() {
        control.cancel();
    }
    waiter.join().unwrap().unwrap();
    assert_eq!(paused.unwrap(), Ok(true));
}

#[test]
fn receives_one_binary_retr_into_the_held_file() {
    let payload = b"\x00abc\r\nxyz\xff";
    let size = payload.len() as u64;
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.anonymous_login(size);
        let mut data = origin.open_data();
        data.write_all(payload).unwrap();
        drop(data);
        origin.reply("226 Transfer complete\r\n");
        origin.command("SIZE pub/a b.bin");
        origin.reply(&format!("213 {size}\r\n"));
    });
    let (_held, mut file) = HeldFile::new();
    let progress = fetch(endpoint, &mut file, &FtpTransferControl::default()).unwrap();
    origin.join().unwrap();
    assert_eq!(progress.downloaded_bytes, size);
    assert_eq!(progress.total_bytes, Some(size));
    file.rewind().unwrap();
    let mut content = Vec::new();
    file.read_to_end(&mut content).unwrap();
    assert_eq!(content, payload);
}

#[test]
fn pauses_and_resumes_the_same_retr_using_the_original_file_handle() {
    let (send_tail, allow_tail) = mpsc::channel();
    let (sent_tail, tail_ready) = mpsc::channel();
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.anonymous_login(8);
        let mut data = origin.open_data();
        data.write_all(b"first").unwrap();
        allow_tail.recv_timeout(WAIT).unwrap();
        data.write_all(b"end").unwrap();
        drop(data);
        origin.reply("250 Data sent\r\n");
        sent_tail.send(()).unwrap();
        origin.size("213 8\r\n");
        origin.expect_closed();
    });
    let (held, mut file) = HeldFile::new();
    let source = held.0.join("file.part");
    let moved = held.0.join("held");
    let control = FtpTransferControl::default();
    let worker_control = control.clone();
    let worker = thread::spawn(move || fetch(endpoint, &mut file, &worker_control));
    wait_progress(&control, 5);
    pause(&control);
    fs::rename(&source, &moved).unwrap();
    fs::write(&source, b"unrelated replacement").unwrap();
    send_tail.send(()).unwrap();
    tail_ready.recv_timeout(WAIT).unwrap();
    assert_eq!(control.progress().unwrap().downloaded_bytes, 5);
    assert_eq!(fs::read(&moved).unwrap(), b"first");
    assert_eq!(fs::read(&source).unwrap(), b"unrelated replacement");
    control.resume().unwrap();
    assert_eq!(worker.join().unwrap().unwrap().downloaded_bytes, 8);
    origin.join().unwrap();
    assert!(control.is_finished());
    assert_eq!(control.request_pause(), Ok(false));
    assert_eq!(control.progress().unwrap().downloaded_bytes, 8);
    assert_eq!(fs::read(&moved).unwrap(), b"firstend");
    assert_eq!(fs::read(&source).unwrap(), b"unrelated replacement");
}

#[test]
fn cancels_a_stalled_data_read_or_paused_data_worker_and_closes_both_sockets() {
    for parked in [false, true] {
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.anonymous_login(8);
            let mut data = origin.open_data();
            data.write_all(b"first").unwrap();
            assert_eq!(data.read(&mut [0]).unwrap(), 0);
            origin.expect_closed();
        });
        let (held, mut file) = HeldFile::new();
        let control = FtpTransferControl::default();
        let worker_control = control.clone();
        let worker = thread::spawn(move || fetch(endpoint, &mut file, &worker_control));
        wait_progress(&control, 5);
        if parked {
            pause(&control);
        }
        let cancel = Instant::now();
        control.cancel();
        assert_eq!(worker.join().unwrap(), Err(FtpReceiveError::Cancelled));
        assert!(cancel.elapsed() < Duration::from_secs(2));
        origin.join().unwrap();
        assert!(control.is_finished());
        assert_eq!(control.progress().unwrap().downloaded_bytes, 5);
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"first");
    }
}

#[test]
fn can_pause_and_cancel_while_waiting_for_the_final_reply() {
    let (sent, eof) = mpsc::channel();
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.anonymous_login(5);
        let mut data = origin.open_data();
        data.write_all(b"first").unwrap();
        data.shutdown(std::net::Shutdown::Write).unwrap();
        assert_eq!(data.read(&mut [0]).unwrap(), 0);
        sent.send(()).unwrap();
        origin.expect_closed();
    });
    let (held, mut file) = HeldFile::new();
    let control = FtpTransferControl::default();
    let worker_control = control.clone();
    let worker = thread::spawn(move || fetch(endpoint, &mut file, &worker_control));
    eof.recv_timeout(WAIT).unwrap();
    pause(&control);
    assert_eq!(control.progress().unwrap().downloaded_bytes, 5);
    assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"first");
    control.cancel();
    assert_eq!(worker.join().unwrap(), Err(FtpReceiveError::Cancelled));
    origin.join().unwrap();
    assert!(control.is_finished());
}

#[test]
fn bounds_idle_data_and_incomplete_final_replies() {
    for (stage, budget) in [(FtpStage::Data, 8), (FtpStage::Completion, 5)] {
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.anonymous_login(5);
            let mut data = origin.open_data();
            data.write_all(b"first").unwrap();
            if stage == FtpStage::Data {
                assert_eq!(data.read(&mut [0]).unwrap(), 0);
            } else {
                drop(data);
                origin.reply("226 Incomplete reply");
            }
            origin.expect_closed();
        });
        let (_held, mut file) = HeldFile::new();
        let control = FtpTransferControl::default();
        let started = Instant::now();
        assert_eq!(
            fetch(endpoint, &mut file, &control),
            Err(FtpReceiveError::TimedOut(stage))
        );
        assert!(started.elapsed() >= Duration::from_secs(budget));
        assert!(started.elapsed() < Duration::from_secs(budget + 3));
        assert_eq!(file.metadata().unwrap().len(), 5);
        origin.join().unwrap();
    }
}

#[test]
fn ipv4_falls_back_to_pasv_once_and_pins_the_actual_control_peer() {
    for mapped in [false, true] {
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.login();
            origin.size("550 SIZE unavailable\r\n");
            origin.command("EPSV");
            origin.reply("502 Command not implemented\r\n");
            origin.command("PASV");
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let [high, low] = listener.local_addr().unwrap().port().to_be_bytes();
            origin.reply(&format!("227 Passive mode 127,0,0,2,{high},{low}.\r\n"));
            let mut data = origin.start_retr(&listener);
            data.write_all(b"yes").unwrap();
            drop(data);
            origin.reply("226 Complete\r\n");
            origin.expect_closed();
        });
        let endpoint = if mapped {
            let SocketAddr::V4(address) = endpoint else {
                unreachable!("IPv4 loopback fixture");
            };
            SocketAddr::new(address.ip().to_ipv6_mapped().into(), address.port())
        } else {
            endpoint
        };
        let (held, mut file) = HeldFile::new();
        let progress = fetch(endpoint, &mut file, &FtpTransferControl::default()).unwrap();
        origin.join().unwrap();
        assert_eq!(progress.total_bytes, Some(3));
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"yes");
    }
}

#[test]
fn ipv6_uses_epsv_and_does_not_try_pasv() {
    for supported in [true, false] {
        let (endpoint, origin) = spawn_origin_on("[::1]:0", move |mut origin| {
            origin.anonymous_login(0);
            if supported {
                drop(origin.open_data());
                origin.reply("226 Complete\r\n");
                origin.size("213 0\r\n");
            } else {
                origin.command("EPSV");
                origin.reply("500 EPSV unavailable\r\n");
            }
            origin.expect_closed();
        });
        let (_held, mut file) = HeldFile::new();
        let result = fetch(endpoint, &mut file, &FtpTransferControl::default());
        origin.join().unwrap();
        if supported {
            assert_eq!(result.unwrap().total_bytes, Some(0));
        } else {
            assert_eq!(
                result,
                Err(FtpReceiveError::Rejected(FtpStage::Passive, 500))
            );
        }
    }
}

#[test]
fn rejects_bad_epsv_replies_without_falling_back() {
    for (response, error) in [
        (
            "229 invalid (|||0|)\r\n",
            FtpReceiveError::InvalidReply(FtpStage::Passive),
        ),
        (
            "229 invalid (|||65536|)\r\n",
            FtpReceiveError::InvalidReply(FtpStage::Passive),
        ),
        (
            "421 Service unavailable\r\n",
            FtpReceiveError::Rejected(FtpStage::Passive, 421),
        ),
    ] {
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.anonymous_login(8);
            origin.command("EPSV");
            origin.reply(response);
            origin.expect_closed();
        });
        let (_held, mut file) = HeldFile::new();
        assert_eq!(
            fetch(endpoint, &mut file, &FtpTransferControl::default()),
            Err(error)
        );
        origin.join().unwrap();
        assert_eq!(file.metadata().unwrap().len(), 0);
    }
}

#[test]
fn rejects_invalid_completion_framing_and_negative_completion() {
    let oversized_line = format!("226 {}\r\n", "x".repeat(9 * 1024));
    let long_line = format!("{}\r\n", "x".repeat(7 * 1024));
    let oversized_reply = format!("226-Continue\r\n{}226 Done\r\n", long_line.repeat(5));
    for (response, error) in [
        (
            "226 Missing CRLF",
            FtpReceiveError::InvalidReply(FtpStage::Completion),
        ),
        (
            "226 Missing CR\n",
            FtpReceiveError::InvalidReply(FtpStage::Completion),
        ),
        (
            "226-Held\r\n250 Other code\r\n",
            FtpReceiveError::InvalidReply(FtpStage::Completion),
        ),
        (
            oversized_line.as_str(),
            FtpReceiveError::InvalidReply(FtpStage::Completion),
        ),
        (
            oversized_reply.as_str(),
            FtpReceiveError::InvalidReply(FtpStage::Completion),
        ),
        (
            "426 Transfer aborted\r\n",
            FtpReceiveError::Rejected(FtpStage::Completion, 426),
        ),
    ] {
        let response = response.to_owned();
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.anonymous_login(3);
            let mut data = origin.open_data();
            data.write_all(b"yes").unwrap();
            drop(data);
            origin.reply(&response);
        });
        let (_held, mut file) = HeldFile::new();
        let control = FtpTransferControl::default();
        assert_eq!(fetch(endpoint, &mut file, &control), Err(error));
        origin.join().unwrap();
        assert_eq!(file.metadata().unwrap().len(), 3);
        assert!(control.is_finished());
    }
}

#[test]
fn rejects_short_long_or_changed_size() {
    for (declared, final_reply, error) in [
        (4, None, FtpReceiveError::LengthMismatch),
        (2, None, FtpReceiveError::LengthMismatch),
        (3, Some("213 4\r\n"), FtpReceiveError::SourceSizeChanged),
        (
            3,
            Some("550 SIZE no longer available\r\n"),
            FtpReceiveError::SourceSizeChanged,
        ),
    ] {
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.anonymous_login(declared);
            let mut data = origin.open_data();
            data.write_all(b"yes").unwrap();
            drop(data);
            if declared >= 3 {
                origin.reply("226 Complete\r\n");
            }
            if let Some(reply) = final_reply {
                origin.size(reply);
            }
            origin.expect_closed();
        });
        let (_held, mut file) = HeldFile::new();
        assert_eq!(
            fetch(endpoint, &mut file, &FtpTransferControl::default()),
            Err(error)
        );
        origin.join().unwrap();
        assert!(file.metadata().unwrap().len() <= declared);
    }
}

#[test]
fn authentication_errors_omit_credentials_and_raw_origin_text() {
    const USER: &str = "FTP_USER_SENTINEL";
    const PASSWORD: &str = "FTP_PASSWORD_SENTINEL";
    const REPLY: &str = "FTP_REPLY_SENTINEL";
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.reply("220 Ready\r\n");
        origin.command(&format!("USER {USER}"));
        origin.reply("331 Continue\r\n");
        origin.command(&format!("PASS {PASSWORD}"));
        origin.reply(&format!("530 {REPLY} {USER} {PASSWORD}\r\n"));
        origin.expect_closed();
    });
    let (_held, mut file) = HeldFile::new();
    let error = receive(
        endpoint,
        "pub/a b.bin",
        FtpCredentials::Password {
            username: USER,
            password: PASSWORD,
        },
        &mut file,
        &FtpTransferControl::default(),
    )
    .unwrap_err();
    origin.join().unwrap();
    assert_eq!(error, FtpReceiveError::Rejected(FtpStage::Login, 530));
    let rendered = format!("{error:?} {error}");
    assert!(
        [USER, PASSWORD, REPLY]
            .iter()
            .all(|sentinel| !rendered.contains(sentinel))
    );
}

#[test]
fn logs_in_when_pass_is_reported_as_superfluous() {
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.reply("220 Ready\r\n");
        origin.command("USER account");
        origin.reply("331 Continue\r\n");
        origin.command("PASS sample-test-password");
        origin.reply("202 PASS superfluous\r\n");
        origin.command("TYPE I");
        origin.reply("200 Binary\r\n");
        origin.size("213 3\r\n");
        let mut data = origin.open_data();
        data.write_all(b"yes").unwrap();
        drop(data);
        origin.reply("226 Complete\r\n");
        origin.size("213 3\r\n");
        origin.expect_closed();
    });
    let (held, mut file) = HeldFile::new();
    let result = receive(
        endpoint,
        "pub/a b.bin",
        FtpCredentials::Password {
            username: "account",
            password: "sample-test-password",
        },
        &mut file,
        &FtpTransferControl::default(),
    );
    assert_eq!(result.unwrap().downloaded_bytes, 3);
    origin.join().unwrap();
    assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"yes");
}

#[test]
fn rejects_untrusted_input_and_pre_cancelled_attempts_before_connecting() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let (_held, mut file) = HeldFile::new();
    let control = FtpTransferControl::default();
    let result = receive(
        endpoint,
        "bad\r\npath",
        FtpCredentials::Anonymous,
        &mut file,
        &control,
    );
    assert_eq!(result, Err(FtpReceiveError::InvalidRequest));
    assert!(control.is_finished());
    assert_eq!(
        fetch(endpoint, &mut file, &control),
        Err(FtpReceiveError::ControlAlreadyUsed)
    );
    let control = FtpTransferControl::default();
    control.cancel();
    assert_eq!(
        fetch(endpoint, &mut file, &control),
        Err(FtpReceiveError::Cancelled)
    );
    assert!(control.is_finished());
    let control = FtpTransferControl::default();
    file.write_all(b"existing data").unwrap();
    assert_eq!(
        fetch(endpoint, &mut file, &control),
        Err(FtpReceiveError::InvalidTarget)
    );
    assert!(control.is_finished());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(file.metadata().unwrap().len(), 13);
}
