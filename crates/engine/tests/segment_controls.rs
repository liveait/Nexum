use nexum_engine::{HttpEngine, HttpResumeValidator, HttpTransferControl, resumable_metadata_path};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const ETAG: &str = "\"staging-v1\"";
const BODY: &[u8] = b"abcdefghijk";

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nexum-segment-controls-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    url: String,
    address: std::net::SocketAddr,
    requests: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(handler: impl Fn(String, TcpStream) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            let mut workers = Vec::new();
            while let Ok((mut stream, _)) = listener.accept() {
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let handler = Arc::clone(&handler);
                let observed = Arc::clone(&observed);
                workers.push(thread::spawn(move || {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        if stream.read(&mut byte).unwrap_or(0) == 0 {
                            return;
                        }
                        request.push(byte[0]);
                    }
                    observed.fetch_add(1, Ordering::Relaxed);
                    handler(String::from_utf8(request).unwrap(), stream);
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            url: format!("http://{address}/file"),
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn requested_range(request: &str) -> (usize, usize) {
    let range = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("range").then_some(value.trim())
        })
        .unwrap();
    let (start, end) = range
        .strip_prefix("bytes=")
        .unwrap()
        .split_once('-')
        .unwrap();
    (start.parse().unwrap(), end.parse().unwrap())
}
fn valid_response(request: &str, body: &[u8]) -> Vec<u8> {
    let (start, end) = requested_range(request);
    let mut response = format!("HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Range: bytes {start}-{end}/{}\r\nETag: {ETAG}\r\nContent-Length: {}\r\n\r\n", body.len(), end-start+1).into_bytes();
    response.extend_from_slice(&body[start..=end]);
    response
}

#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);
impl Gate {
    fn wait(&self) {
        let (lock, changed) = &*self.0;
        let (open, timeout) = changed
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(8), |open| !*open)
            .unwrap();
        assert!(*open && !timeout.timed_out(), "fixture gate timed out");
    }
    fn open(&self) {
        let (lock, changed) = &*self.0;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}

struct Download {
    control: HttpTransferControl,
    result: mpsc::Receiver<Result<nexum_domain::Progress, nexum_engine::EngineError>>,
    thread: Option<thread::JoinHandle<()>>,
    directory: Directory,
}
impl Download {
    fn start(
        fixture: &Fixture,
        count: usize,
        control: HttpTransferControl,
        mut progress: impl FnMut(nexum_domain::Progress) -> Result<(), nexum_engine::EngineError>
        + Send
        + 'static,
    ) -> Self {
        let directory = Directory::new();
        let destination = directory.0.join("destination");
        let partial = directory.0.join("partial");
        std::fs::write(resumable_metadata_path(&partial), b"unowned sidecar").unwrap();
        let url = fixture.url.clone();
        let child = control.clone();
        let (tx, result) = mpsc::channel();
        let thread = thread::spawn(move || {
            let value = HttpEngine::new().download_to_segments(
                &url,
                &HttpEngine::split_segments(BODY.len() as u64, count),
                destination.to_str().unwrap(),
                &partial,
                BODY.len() as u64,
                Some(&HttpResumeValidator::Etag(ETAG.into())),
                &child,
                &mut progress,
            );
            tx.send(value).unwrap();
        });
        Self {
            control,
            result,
            thread: Some(thread),
            directory,
        }
    }
    fn assert_failed_cleanup(&self) {
        assert!(self.control.is_finished());
        assert!(!self.directory.0.join("partial").exists());
        assert!(!self.directory.0.join("destination").exists());
        assert_eq!(
            std::fs::read(resumable_metadata_path(self.directory.0.join("partial"))).unwrap(),
            b"unowned sidecar"
        );
    }
    fn join(&mut self) {
        self.thread.take().unwrap().join().unwrap();
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        self.control.cancel();
        if let Some(worker) = self.thread.take() {
            // Explicit success paths assert join results. During an existing
            // test failure, join without causing a second unwinding panic.
            let _ = worker.join();
        }
    }
}

#[test]
fn cancellation_interrupts_waiting_response_headers() {
    let release = Gate::default();
    let child_release = release.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let fixture = Fixture::new(move |request, mut stream| {
        entered_tx.send(()).unwrap();
        child_release.wait();
        let _ = stream.write_all(&valid_response(&request, BODY));
    });
    let mut download = Download::start(&fixture, 1, HttpTransferControl::new(), |_| Ok(()));
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    download.control.cancel();
    let early = download.result.recv_timeout(Duration::from_secs(2));
    release.open();
    let responsive = early.is_ok();
    let result = early.unwrap_or_else(|_| {
        download
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    });
    download.join();
    assert_eq!(result.unwrap_err(), nexum_engine::EngineError::Cancelled);
    download.assert_failed_cleanup();
    assert!(
        responsive,
        "cancel must terminate before the server releases response headers"
    );
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 1);
}

#[test]
fn worker_failure_interrupts_peer_waiting_response_headers() {
    let release = Gate::default();
    let child_release = release.clone();
    let entered = Arc::new((Mutex::new(0usize), Condvar::new()));
    let fixture_entered = Arc::clone(&entered);
    let fixture = Fixture::new(move |request, mut stream| {
        let (lock, changed) = &*fixture_entered;
        let mut count = lock.lock().unwrap();
        *count += 1;
        changed.notify_all();
        let (count, timeout) = changed
            .wait_timeout_while(count, Duration::from_secs(3), |n| *n < 2)
            .unwrap();
        assert!(!timeout.timed_out());
        drop(count);
        if requested_range(&request).0 == 0 {
            child_release.wait();
            let _ = stream.write_all(&valid_response(&request, BODY));
        } else {
            let _ = stream
                .write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    });
    let mut download = Download::start(&fixture, 2, HttpTransferControl::new(), |_| Ok(()));
    wait_for_requests(&entered, 2);
    let early = download.result.recv_timeout(Duration::from_secs(2));
    release.open();
    let responsive = early.is_ok();
    let result = early.unwrap_or_else(|_| {
        download
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    });
    download.join();
    assert!(result.unwrap_err().to_string().contains("500"));
    download.assert_failed_cleanup();
    assert!(
        responsive,
        "failure must interrupt another worker's header wait"
    );
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 2);
}

#[test]
fn progress_error_interrupts_peer_waiting_response_headers() {
    let release = Gate::default();
    let child_release = release.clone();
    let entered = Arc::new((Mutex::new(0usize), Condvar::new()));
    let fixture_entered = Arc::clone(&entered);
    let fixture = Fixture::new(move |request, mut stream| {
        let (lock, changed) = &*fixture_entered;
        let mut count = lock.lock().unwrap();
        *count += 1;
        changed.notify_all();
        let (count, timeout) = changed
            .wait_timeout_while(count, Duration::from_secs(3), |n| *n < 2)
            .unwrap();
        assert!(!timeout.timed_out());
        drop(count);
        if requested_range(&request).0 == 0 {
            child_release.wait();
        }
        let _ = stream.write_all(&valid_response(&request, BODY));
    });
    let calls = Arc::new(AtomicU64::new(0));
    let child_calls = Arc::clone(&calls);
    let mut download = Download::start(&fixture, 2, HttpTransferControl::new(), move |progress| {
        child_calls.fetch_add(1, Ordering::SeqCst);
        if progress.downloaded_bytes > 0 {
            Err(nexum_engine::EngineError::Failed(
                "specific callback failure".into(),
            ))
        } else {
            Ok(())
        }
    });
    wait_for_requests(&entered, 2);
    let early = download.result.recv_timeout(Duration::from_secs(2));
    release.open();
    let responsive = early.is_ok();
    let result = early.unwrap_or_else(|_| {
        download
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    });
    download.join();
    assert_eq!(
        result.unwrap_err(),
        nexum_engine::EngineError::Failed("specific callback failure".into())
    );
    download.assert_failed_cleanup();
    assert!(
        responsive,
        "callback failure must interrupt another worker's header wait"
    );
}

#[test]
fn pause_freezes_all_writes_and_progress_until_resume() {
    let release = Gate::default();
    let child_release = release.clone();
    let fixture = Fixture::new(move |_, mut stream| {
        write!(stream, "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nContent-Length: 11\r\n\r\na").unwrap();
        child_release.wait();
        let _ = stream.write_all(&BODY[1..]);
    });
    let (progress_tx, progress_rx) = mpsc::channel();
    let mut download = Download::start(&fixture, 1, HttpTransferControl::new(), move |progress| {
        progress_tx.send(progress.downloaded_bytes).unwrap();
        Ok(())
    });
    loop {
        if progress_rx.recv_timeout(Duration::from_secs(3)).unwrap() > 0 {
            break;
        }
    }
    let pause = download.control.clone();
    let (paused_tx, paused_rx) = mpsc::channel();
    let requester = thread::spawn(move || {
        paused_tx.send(pause.request_pause()).unwrap();
    });
    let paused = paused_rx.recv_timeout(Duration::from_secs(2));
    if paused.is_err() {
        download.control.cancel();
        release.open();
    }
    assert!(paused.is_ok(), "pause must acknowledge an idle body read");
    paused.unwrap().unwrap();
    requester.join().unwrap();
    while progress_rx.try_recv().is_ok() {}
    let before = std::fs::read(download.directory.0.join("partial")).unwrap();
    release.open();
    let extra_progress = progress_rx.recv_timeout(Duration::from_millis(400)).is_ok();
    let after = std::fs::read(download.directory.0.join("partial")).unwrap();
    download.control.resume().unwrap();
    let result = download
        .result
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    download.join();
    result.unwrap();
    assert!(
        !extra_progress,
        "coordinator must not emit progress after pause acknowledgement"
    );
    assert_eq!(
        before, after,
        "all file writes must stay stopped while paused"
    );
    assert_eq!(
        std::fs::read(download.directory.0.join("destination")).unwrap(),
        BODY
    );
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 1);
}

#[test]
fn callback_error_wakes_paused_workers_without_changing_error() {
    let release_body = Gate::default();
    let body_gate = release_body.clone();
    let fixture = Fixture::new(move |_, mut stream| {
        write!(stream, "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nContent-Length: 11\r\n\r\na").unwrap();
        body_gate.wait();
        let _ = stream.write_all(&BODY[1..]);
    });
    let callback_release = Gate::default();
    let callback_gate = callback_release.clone();
    let (callback_tx, callback_rx) = mpsc::channel();
    let mut download = Download::start(&fixture, 1, HttpTransferControl::new(), move |progress| {
        if progress.downloaded_bytes > 0 {
            callback_tx.send(()).unwrap();
            callback_gate.wait();
            return Err(nexum_engine::EngineError::Failed(
                "paused callback failure".into(),
            ));
        }
        Ok(())
    });
    callback_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let pause = download.control.clone();
    let (paused_tx, paused_rx) = mpsc::channel();
    let requester = thread::spawn(move || {
        let _ = paused_tx.send(pause.request_pause());
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !download.control.is_pause_requested() && Instant::now() < deadline {
        thread::yield_now();
    }
    // The callback is deliberately in flight. A pause cannot be acknowledged
    // until it returns, even if the network worker is already parked.
    let acknowledged_early = paused_rx.recv_timeout(Duration::from_millis(1200)).is_ok();
    callback_release.open();
    let early = download.result.recv_timeout(Duration::from_secs(2));
    let responsive = early.is_ok();
    download.control.cancel();
    release_body.open();
    let result = early.unwrap_or_else(|_| {
        download
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    });
    requester.join().unwrap();
    download.join();
    assert_eq!(
        result.unwrap_err(),
        nexum_engine::EngineError::Failed("paused callback failure".into())
    );
    download.assert_failed_cleanup();
    assert!(
        !acknowledged_early,
        "in-flight progress callback participates in the pause barrier"
    );
    assert!(
        responsive,
        "callback failure must wake workers parked for pause"
    );
}

fn wait_for_requests(entered: &Arc<(Mutex<usize>, Condvar)>, count: usize) {
    let (lock, changed) = &**entered;
    let (actual, timeout) = changed
        .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(3), |actual| {
            *actual < count
        })
        .unwrap();
    assert!(!timeout.timed_out() && *actual >= count);
}

#[test]
fn mixed_header_and_body_workers_pause_together_and_resume_one_get_each() {
    let release = Gate::default();
    let child_release = release.clone();
    let entered = Arc::new((Mutex::new(0usize), Condvar::new()));
    let fixture_entered = Arc::clone(&entered);
    let fixture = Fixture::new(move |request, mut stream| {
        let (lock, changed) = &*fixture_entered;
        *lock.lock().unwrap() += 1;
        changed.notify_all();
        let (start, end) = requested_range(&request);
        if start == 0 {
            write!(stream, "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Range: bytes {start}-{end}/11\r\nETag: {ETAG}\r\nContent-Length: {}\r\n\r\na", end-start+1).unwrap();
            child_release.wait();
            let _ = stream.write_all(&BODY[1..=end]);
        } else {
            child_release.wait();
            let _ = stream.write_all(&valid_response(&request, BODY));
        }
    });
    let (progress_tx, progress_rx) = mpsc::channel();
    let mut download = Download::start(&fixture, 2, HttpTransferControl::new(), move |progress| {
        progress_tx.send(progress.downloaded_bytes).unwrap();
        Ok(())
    });
    wait_for_requests(&entered, 2);
    loop {
        if progress_rx.recv_timeout(Duration::from_secs(3)).unwrap() > 0 {
            break;
        }
    }
    let pause = download.control.clone();
    let (paused_tx, paused_rx) = mpsc::channel();
    let requester = thread::spawn(move || {
        let _ = paused_tx.send(pause.request_pause());
    });
    let paused = paused_rx.recv_timeout(Duration::from_secs(2));
    if paused.is_err() {
        release.open();
        download.control.cancel();
    }
    assert!(
        paused.is_ok(),
        "all participants must acknowledge pause while headers and body are idle"
    );
    paused.unwrap().unwrap();
    requester.join().unwrap();
    while progress_rx.try_recv().is_ok() {}
    let before = std::fs::read(download.directory.0.join("partial")).unwrap();
    release.open();
    let extra_progress = progress_rx.recv_timeout(Duration::from_millis(400)).is_ok();
    let after = std::fs::read(download.directory.0.join("partial")).unwrap();
    download.control.resume().unwrap();
    download
        .result
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    download.join();
    assert!(
        !extra_progress,
        "mixed-phase pause must freeze the coordinator too"
    );
    assert_eq!(before, after);
    assert_eq!(
        std::fs::read(download.directory.0.join("destination")).unwrap(),
        BODY
    );
    assert_eq!(
        fixture.requests.load(Ordering::Relaxed),
        2,
        "pause must retain each original request future"
    );
}

#[test]
fn cancellation_interrupts_waiting_response_body() {
    let release = Gate::default();
    let child_release = release.clone();
    let fixture = Fixture::new(move |_, mut stream| {
        write!(stream, "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nContent-Length: 11\r\n\r\na").unwrap();
        child_release.wait();
        let _ = stream.write_all(&BODY[1..]);
    });
    let (progress_tx, progress_rx) = mpsc::channel();
    let mut download = Download::start(&fixture, 1, HttpTransferControl::new(), move |progress| {
        let _ = progress_tx.send(progress.downloaded_bytes);
        Ok(())
    });
    loop {
        if progress_rx.recv_timeout(Duration::from_secs(3)).unwrap() > 0 {
            break;
        }
    }
    download.control.cancel();
    let early = download.result.recv_timeout(Duration::from_secs(2));
    release.open();
    let responsive = early.is_ok();
    let result = early.unwrap_or_else(|_| {
        download
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    });
    download.join();
    assert_eq!(result.unwrap_err(), nexum_engine::EngineError::Cancelled);
    download.assert_failed_cleanup();
    assert!(responsive, "body cancellation must not wait for the server");
}

#[test]
fn cancellation_wakes_all_paused_workers() {
    let release = Gate::default();
    let child_release = release.clone();
    let entered = Arc::new((Mutex::new(0usize), Condvar::new()));
    let fixture_entered = Arc::clone(&entered);
    let fixture = Fixture::new(move |request, mut stream| {
        let (lock, changed) = &*fixture_entered;
        *lock.lock().unwrap() += 1;
        changed.notify_all();
        child_release.wait();
        let _ = stream.write_all(&valid_response(&request, BODY));
    });
    let mut download = Download::start(&fixture, 2, HttpTransferControl::new(), |_| Ok(()));
    wait_for_requests(&entered, 2);
    let pause = download.control.clone();
    let (paused_tx, paused_rx) = mpsc::channel();
    let requester = thread::spawn(move || {
        let _ = paused_tx.send(pause.request_pause());
    });
    let paused = paused_rx.recv_timeout(Duration::from_secs(2));
    download.control.cancel();
    let early = download.result.recv_timeout(Duration::from_secs(2));
    release.open();
    let responsive = early.is_ok();
    let result = early.unwrap_or_else(|_| {
        download
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    });
    requester.join().unwrap();
    download.join();
    assert!(paused.is_ok(), "all waiting-header participants must park");
    assert_eq!(result.unwrap_err(), nexum_engine::EngineError::Cancelled);
    download.assert_failed_cleanup();
    assert!(responsive, "cancel must wake all parked participants");
}

#[test]
fn final_progress_pause_prevents_publication_until_resume() {
    let callback_release = Gate::default();
    let callback_gate = callback_release.clone();
    let fixture = Fixture::new(|request, mut stream| {
        let _ = stream.write_all(&valid_response(&request, BODY));
    });
    let (final_tx, final_rx) = mpsc::channel();
    let mut download = Download::start(&fixture, 2, HttpTransferControl::new(), move |progress| {
        if progress.downloaded_bytes == BODY.len() as u64 {
            final_tx.send(()).unwrap();
            callback_gate.wait();
        }
        Ok(())
    });
    final_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let pause = download.control.clone();
    let (paused_tx, paused_rx) = mpsc::channel();
    let requester = thread::spawn(move || {
        let _ = paused_tx.send(pause.request_pause());
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !download.control.is_pause_requested() && Instant::now() < deadline {
        thread::yield_now();
    }
    callback_release.open();
    let paused = paused_rx.recv_timeout(Duration::from_secs(2));
    if paused.is_err() {
        download.control.cancel();
    }
    let published_while_paused = download.directory.0.join("destination").exists();
    download.control.resume().unwrap();
    let result = download
        .result
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    requester.join().unwrap();
    download.join();
    paused.unwrap().unwrap();
    result.unwrap();
    assert!(
        !published_while_paused,
        "final coordinator pause must precede publication"
    );
    assert_eq!(
        std::fs::read(download.directory.0.join("destination")).unwrap(),
        BODY
    );
}

#[test]
fn worker_failure_interrupts_peer_bandwidth_wait() {
    let release_error = Gate::default();
    let error_gate = release_error.clone();
    let entered = Arc::new((Mutex::new(0usize), Condvar::new()));
    let fixture_entered = Arc::clone(&entered);
    let fixture = Fixture::new(move |request, mut stream| {
        let (lock, changed) = &*fixture_entered;
        *lock.lock().unwrap() += 1;
        changed.notify_all();
        if requested_range(&request).0 == 0 {
            let _ = stream.write_all(&valid_response(&request, BODY));
        } else {
            error_gate.wait();
            let _ = stream
                .write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    });
    let control = HttpTransferControl::new();
    control.set_bytes_per_second(Some(1));
    let ready = Arc::new(AtomicBool::new(false));
    let child_ready = Arc::clone(&ready);
    let callback_gate = release_error.clone();
    let mut download = Download::start(&fixture, 2, control, move |progress| {
        if progress.downloaded_bytes == 0 && !child_ready.swap(true, Ordering::SeqCst) {
            callback_gate.open();
        }
        Ok(())
    });
    wait_for_requests(&entered, 2);
    let early = download.result.recv_timeout(Duration::from_secs(2));
    let responsive = early.is_ok();
    // Rescue the old uninterruptible pacer without changing the first error.
    download.control.cancel();
    release_error.open();
    let result = early.unwrap_or_else(|_| {
        download
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    });
    download.join();
    assert!(result.unwrap_err().to_string().contains("500"));
    download.assert_failed_cleanup();
    assert!(
        responsive,
        "peer failure must interrupt the reserved bandwidth wait"
    );
}

#[test]
fn cancellation_interrupts_bandwidth_wait() {
    let fixture = Fixture::new(|request, mut stream| {
        let _ = stream.write_all(&valid_response(&request, BODY));
    });
    let control = HttpTransferControl::new();
    control.set_bytes_per_second(Some(1));
    let (tx, rx) = mpsc::channel();
    let mut download = Download::start(&fixture, 1, control, move |progress| {
        let _ = tx.send(progress.downloaded_bytes);
        Ok(())
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), 0);
    download.control.cancel();
    let result = download
        .result
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    download.join();
    assert_eq!(result.unwrap_err(), nexum_engine::EngineError::Cancelled);
    download.assert_failed_cleanup();
}

#[test]
fn progress_callback_panic_joins_peers_and_cleans_owned_file() {
    let release = Gate::default();
    let child_release = release.clone();
    let entered = Arc::new((Mutex::new(0usize), Condvar::new()));
    let fixture_entered = Arc::clone(&entered);
    let fixture = Fixture::new(move |request, mut stream| {
        let (lock, changed) = &*fixture_entered;
        *lock.lock().unwrap() += 1;
        changed.notify_all();
        if requested_range(&request).0 == 0 {
            child_release.wait();
        }
        let _ = stream.write_all(&valid_response(&request, BODY));
    });
    let mut download = Download::start(&fixture, 2, HttpTransferControl::new(), |progress| {
        if progress.downloaded_bytes > 0 {
            panic!("injected progress panic");
        }
        Ok(())
    });
    wait_for_requests(&entered, 2);
    let early = download.result.recv_timeout(Duration::from_secs(2));
    release.open();
    let responsive = early.is_ok();
    let result = early.unwrap_or_else(|_| {
        download
            .result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    });
    download.join();
    assert_eq!(
        result.unwrap_err(),
        nexum_engine::EngineError::Failed("segment progress callback panicked".into())
    );
    download.assert_failed_cleanup();
    assert!(responsive);
}
