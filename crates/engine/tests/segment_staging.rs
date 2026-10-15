use nexum_engine::{
    HttpEngine, HttpResumeValidator, HttpTransferControl, Segment, resumable_metadata_path,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Duration;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const ETAG: &str = "\"staging-v1\"";
const BODY: &[u8] = b"abcdefghijk";
const REPLACEMENT: &[u8] = b"replacement";

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nexum-segment-staging-{}-{}",
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
    fn valid() -> Self {
        Self::new(|request, mut stream| {
            let _ = stream.write_all(&valid_response(&request, BODY));
        })
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
fn download(
    fixture: &Fixture,
    partial: &Path,
    destination: &Path,
) -> Result<nexum_domain::Progress, nexum_engine::EngineError> {
    HttpEngine::new().download_to_segments(
        &fixture.url,
        &[Segment::new(0, BODY.len() as u64 - 1)],
        destination.to_str().unwrap(),
        partial,
        BODY.len() as u64,
        Some(&HttpResumeValidator::Etag(ETAG.into())),
        &HttpTransferControl::new(),
        |_| Ok(()),
    )
}

#[test]
fn existing_regular_staging_is_preserved_without_requests() {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("destination");
    let sidecar = resumable_metadata_path(&partial);
    std::fs::write(&partial, b"previous download").unwrap();
    std::fs::write(&sidecar, b"unrelated metadata").unwrap();
    let fixture = Fixture::valid();
    assert!(download(&fixture, &partial, &destination).is_err());
    assert_eq!(std::fs::read(partial).unwrap(), b"previous download");
    assert_eq!(std::fs::read(sidecar).unwrap(), b"unrelated metadata");
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 0);
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn existing_symlink_staging_and_target_are_preserved_without_requests() {
    use std::os::unix::fs::symlink;
    let directory = Directory::new();
    let target = directory.0.join("target");
    let partial = directory.0.join("partial");
    let destination = directory.0.join("destination");
    std::fs::write(&target, b"private user bytes").unwrap();
    symlink(&target, &partial).unwrap();
    let fixture = Fixture::valid();
    assert!(download(&fixture, &partial, &destination).is_err());
    assert!(std::fs::symlink_metadata(&partial).unwrap().is_symlink());
    assert_eq!(std::fs::read(&target).unwrap(), b"private user bytes");
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 0);
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn existing_fifo_staging_is_rejected_and_preserved() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("destination");
    let c_path = std::ffi::CString::new(partial.as_os_str().as_bytes()).unwrap();
    // SAFETY: c_path is a NUL-terminated pathname and mode is a valid permission mask.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    // Keep the regression safe on the old blocking writer implementation too.
    let _reader = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&partial)
        .unwrap();
    let fixture = Fixture::valid();
    assert!(download(&fixture, &partial, &destination).is_err());
    assert!(
        std::fs::symlink_metadata(&partial)
            .unwrap()
            .file_type()
            .is_fifo()
    );
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 0);
    assert!(!destination.exists());
}

#[test]
fn existing_directory_staging_is_rejected_and_preserved() {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("destination");
    std::fs::create_dir(&partial).unwrap();
    std::fs::write(partial.join("owned"), b"user data").unwrap();
    let fixture = Fixture::valid();
    assert!(download(&fixture, &partial, &destination).is_err());
    assert_eq!(std::fs::read(partial.join("owned")).unwrap(), b"user data");
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 0);
    assert!(!destination.exists());
}

enum DelayedOutcome {
    Success,
    Failed,
    Cancelled,
}
fn assert_delayed_replacement(outcome: DelayedOutcome) {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let displaced = directory.0.join("displaced");
    let destination = directory.0.join("destination");
    let sidecar = resumable_metadata_path(&partial);
    std::fs::write(&sidecar, b"unrelated sidecar").unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let failed = matches!(outcome, DelayedOutcome::Failed);
    let fixture = Fixture::new(move |request, mut stream| {
        entered_tx.send(()).unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let response = if failed {
            b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else {
            valid_response(&request, BODY)
        };
        let _ = stream.write_all(&response);
    });
    let control = HttpTransferControl::new();
    let child_control = control.clone();
    let url = fixture.url.clone();
    let child_partial = partial.clone();
    let child_destination = destination.clone();
    let worker = thread::spawn(move || {
        HttpEngine::new().download_to_segments(
            &url,
            &[Segment::new(0, BODY.len() as u64 - 1)],
            child_destination.to_str().unwrap(),
            child_partial,
            BODY.len() as u64,
            Some(&HttpResumeValidator::Etag(ETAG.into())),
            &child_control,
            |_| Ok(()),
        )
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    std::fs::rename(&partial, &displaced).unwrap();
    std::fs::write(&partial, REPLACEMENT).unwrap();
    if matches!(outcome, DelayedOutcome::Cancelled) {
        control.cancel();
    }
    release_tx.send(()).unwrap();
    let result = worker.join().unwrap();
    assert!(result.is_err(), "replaced staging must never publish");
    assert_eq!(std::fs::read(&partial).unwrap(), REPLACEMENT);
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"unrelated sidecar");
    assert_eq!(std::fs::read(displaced).unwrap(), vec![0; BODY.len()]);
    assert!(!destination.exists());
}
#[test]
fn replacement_during_delayed_response_is_not_written_or_published() {
    assert_delayed_replacement(DelayedOutcome::Success);
}
#[test]
fn failed_response_preserves_replacement_and_unowned_sidecar() {
    assert_delayed_replacement(DelayedOutcome::Failed);
}
#[test]
fn cancellation_preserves_replacement_and_unowned_sidecar() {
    assert_delayed_replacement(DelayedOutcome::Cancelled);
}

#[test]
fn concurrent_segments_write_exact_independent_offsets() {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("destination");
    let body = Arc::new(
        (0..1024 * 1024)
            .map(|i| (i * 31 % 251) as u8)
            .collect::<Vec<_>>(),
    );
    let expected = Arc::clone(&body);
    let segments = HttpEngine::split_segments(body.len() as u64, 16);
    let ready = (Mutex::new(0), Condvar::new());
    let count = segments.len();
    let fixture = Fixture::new(move |request, mut stream| {
        let (start, end) = requested_range(&request);
        let headers = format!(
            "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Range: bytes {start}-{end}/{}\r\nETag: {ETAG}\r\nContent-Length: {}\r\n\r\n",
            body.len(),
            end - start + 1
        );
        stream.write_all(headers.as_bytes()).unwrap();
        let (lock, changed) = &ready;
        let mut entered = lock.lock().unwrap();
        *entered += 1;
        changed.notify_all();
        let (guard, timeout) = changed
            .wait_timeout_while(entered, Duration::from_secs(5), |entered| *entered < count)
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "all segment requests must reach the fixture"
        );
        drop(guard);
        for chunk in body[start..=end].chunks(257) {
            stream.write_all(chunk).unwrap();
            thread::yield_now();
        }
    });
    HttpEngine::new()
        .download_to_segments(
            &fixture.url,
            &segments,
            destination.to_str().unwrap(),
            &partial,
            expected.len() as u64,
            Some(&HttpResumeValidator::Etag(ETAG.into())),
            &HttpTransferControl::new(),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), *expected);
    assert!(!partial.exists());
}

#[test]
fn replacement_after_first_chunk_stops_later_writes() {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let displaced = directory.0.join("displaced");
    let destination = directory.0.join("destination");
    let sidecar = resumable_metadata_path(&partial);
    std::fs::write(&sidecar, b"unrelated sidecar").unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let fixture = Fixture::new(move |_, mut stream| {
        write!(stream, "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nContent-Length: 11\r\n\r\na").unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let _ = stream.write_all(&BODY[1..]);
    });
    let mut replaced = false;
    let result = HttpEngine::new().download_to_segments(
        &fixture.url,
        &[Segment::new(0, 10)],
        destination.to_str().unwrap(),
        &partial,
        11,
        Some(&HttpResumeValidator::Etag(ETAG.into())),
        &HttpTransferControl::new(),
        |progress| {
            if progress.downloaded_bytes > 0 && !replaced {
                assert_eq!(progress.downloaded_bytes, 1);
                std::fs::rename(&partial, &displaced).unwrap();
                std::fs::write(&partial, REPLACEMENT).unwrap();
                replaced = true;
                release_tx.send(()).unwrap();
            }
            Ok(())
        },
    );
    assert!(replaced);
    assert!(result.is_err());
    assert_eq!(std::fs::read(&partial).unwrap(), REPLACEMENT);
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"unrelated sidecar");
    let mut first_chunk = vec![0; BODY.len()];
    first_chunk[0] = b'a';
    assert_eq!(std::fs::read(displaced).unwrap(), first_chunk);
    assert!(!destination.exists());
}

#[test]
fn replacement_in_final_progress_callback_is_not_published() {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let displaced = directory.0.join("displaced");
    let destination = directory.0.join("destination");
    let fixture = Fixture::valid();
    let mut replaced = false;
    let result = HttpEngine::new().download_to_segments(
        &fixture.url,
        &[Segment::new(0, 10)],
        destination.to_str().unwrap(),
        &partial,
        11,
        Some(&HttpResumeValidator::Etag(ETAG.into())),
        &HttpTransferControl::new(),
        |progress| {
            if progress.downloaded_bytes == 11 && !replaced {
                std::fs::rename(&partial, &displaced).unwrap();
                std::fs::write(&partial, REPLACEMENT).unwrap();
                replaced = true;
            }
            Ok(())
        },
    );
    assert!(replaced);
    assert!(result.is_err());
    assert_eq!(std::fs::read(&partial).unwrap(), REPLACEMENT);
    assert_eq!(std::fs::read(displaced).unwrap(), BODY);
    assert!(!destination.exists());
}

#[test]
fn success_cleans_owned_partial_and_preserves_unowned_sidecar() {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("destination");
    let sidecar = resumable_metadata_path(&partial);
    std::fs::write(&sidecar, b"unrelated sidecar").unwrap();
    let fixture = Fixture::valid();
    download(&fixture, &partial, &destination).unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), BODY);
    assert!(!partial.exists());
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"unrelated sidecar");
}

#[test]
fn failed_response_cleans_owned_partial_and_preserves_unowned_sidecar() {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("destination");
    let sidecar = resumable_metadata_path(&partial);
    std::fs::write(&sidecar, b"unrelated sidecar").unwrap();
    let fixture = Fixture::new(|_, mut stream| {
        let _ = stream
            .write_all(b"HTTP/1.1 500 Error\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
    });
    assert!(download(&fixture, &partial, &destination).is_err());
    assert!(!partial.exists());
    assert!(!destination.exists());
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"unrelated sidecar");
}

#[test]
fn cancelled_transfer_cleans_owned_partial_and_preserves_unowned_sidecar() {
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("destination");
    let sidecar = resumable_metadata_path(&partial);
    std::fs::write(&sidecar, b"unrelated sidecar").unwrap();
    let control = HttpTransferControl::new();
    let child_control = control.clone();
    let fixture = Fixture::new(move |request, mut stream| {
        child_control.cancel();
        let _ = stream.write_all(&valid_response(&request, BODY));
    });
    let result = HttpEngine::new().download_to_segments(
        &fixture.url,
        &[Segment::new(0, 10)],
        destination.to_str().unwrap(),
        &partial,
        11,
        Some(&HttpResumeValidator::Etag(ETAG.into())),
        &control,
        |_| Ok(()),
    );
    assert_eq!(result.unwrap_err(), nexum_engine::EngineError::Cancelled);
    assert!(!partial.exists());
    assert!(!destination.exists());
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"unrelated sidecar");
}
