use nexum_engine::{HttpEngine, HttpResumeValidator, HttpTransferControl, Segment};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const BODY: &[u8] = b"abcdefghijk";
const ETAG: &str = "\"segments-v1\"";

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nexum-segment-validation-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

struct Fixture {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new(response: impl Fn(&str) -> Vec<u8> + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_request(&mut stream);
                        observed.lock().unwrap().push(request.clone());
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let _ = stream.write_all(&response(&request));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            break;
        }
        request.push(byte[0]);
    }
    String::from_utf8(request).unwrap()
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

fn response(status: &str, headers: &str, bytes: &[u8]) -> Vec<u8> {
    let mut response =
        format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n\r\n").into_bytes();
    response.extend_from_slice(bytes);
    response
}

fn valid_response(request: &str) -> Vec<u8> {
    let (start, end) = requested_range(request);
    response(
        "206 Partial Content",
        &format!(
            "Content-Range: bytes {start}-{end}/{}\r\nETag: {ETAG}\r\nContent-Length: {}",
            BODY.len(),
            end - start + 1
        ),
        &BODY[start..=end],
    )
}

#[test]
fn segment_api_reconstructs_exact_file_and_sends_validator() {
    let fixture = Fixture::new(valid_response);
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("download");
    let mut progress = Vec::new();
    let result = HttpEngine::new()
        .download_to_segments(
            &fixture.url,
            &HttpEngine::split_segments(BODY.len() as u64, 3),
            destination.to_str().unwrap(),
            &partial,
            BODY.len() as u64,
            Some(&HttpResumeValidator::Etag(ETAG.into())),
            &HttpTransferControl::new(),
            |value| {
                progress.push(value);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), BODY);
    assert_eq!(result.downloaded_bytes, BODY.len() as u64);
    assert!(!partial.exists());
    assert!(
        progress
            .iter()
            .all(|value| value.downloaded_bytes <= BODY.len() as u64)
    );
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| {
        request
            .to_ascii_lowercase()
            .contains(&format!("if-range: {ETAG}"))
    }));
    assert!(requests.iter().all(|request| {
        request
            .lines()
            .any(|line| line.eq_ignore_ascii_case("accept-encoding: identity"))
    }));
}

#[test]
fn chunked_probe_and_segments_reconstruct_exact_file_through_eof() {
    let fixture = Fixture::new(|request| {
        let (start, end) = requested_range(request);
        let mut chunks = Vec::new();
        for chunk in BODY[start..=end].chunks(2) {
            write!(&mut chunks, "{:x}\r\n", chunk.len()).unwrap();
            chunks.extend_from_slice(chunk);
            chunks.extend_from_slice(b"\r\n");
        }
        chunks.extend_from_slice(b"0\r\n\r\n");
        response(
            "206 Partial Content",
            &format!(
                "Content-Range: bytes {start}-{end}/{}\r\nETag: {ETAG}\r\nContent-Encoding: identity\r\nTransfer-Encoding: chunked",
                BODY.len()
            ),
            &chunks,
        )
    });
    let engine = HttpEngine::new();
    let capability = engine.probe_range(&fixture.url).unwrap();
    assert!(capability.supports_ranges);
    assert_eq!(capability.length, Some(BODY.len() as u64));
    assert_eq!(
        capability.validator,
        Some(HttpResumeValidator::Etag(ETAG.into()))
    );
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    let destination = directory.0.join("download");
    let result = engine
        .download_to_segments(
            &fixture.url,
            &HttpEngine::split_segments(BODY.len() as u64, 3),
            destination.to_str().unwrap(),
            &partial,
            BODY.len() as u64,
            capability.validator.as_ref(),
            &HttpTransferControl::new(),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), BODY);
    assert_eq!(result.downloaded_bytes, BODY.len() as u64);
    assert_eq!(result.total_bytes, Some(BODY.len() as u64));
    assert!(!partial.exists());
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requested_range(&requests[0]), (0, 0));
    assert!(requests.iter().all(|request| {
        request
            .lines()
            .any(|line| line.eq_ignore_ascii_case("accept-encoding: identity"))
    }));
}

fn assert_invalid_input(segments: &[Segment], total: u64, validator: Option<HttpResumeValidator>) {
    let fixture = Fixture::new(valid_response);
    let directory = Directory::new();
    let partial = directory.0.join("partial");
    std::fs::write(&partial, b"preexisting partial").unwrap();
    let destination = directory.0.join("uncreated-parent/download");
    let result = HttpEngine::new().download_to_segments(
        &fixture.url,
        segments,
        destination.to_str().unwrap(),
        &partial,
        total,
        validator.as_ref(),
        &HttpTransferControl::new(),
        |_| Ok(()),
    );
    assert!(result.is_err(), "invalid segment input must fail");
    assert_eq!(std::fs::read(&partial).unwrap(), b"preexisting partial");
    assert!(
        !destination.parent().unwrap().exists(),
        "must validate before creating directories"
    );
    assert!(
        fixture.requests.lock().unwrap().is_empty(),
        "must validate before requests"
    );
}

#[test]
fn segment_api_rejects_invalid_coverage_before_side_effects() {
    for segments in [
        vec![],
        vec![Segment::new(1, 10)],
        vec![Segment::new(0, 9)],
        vec![Segment::new(0, 3), Segment::new(5, 10)],
        vec![Segment::new(0, 5), Segment::new(5, 10)],
        vec![Segment::new(0, 5), Segment::new(8, 7)],
        vec![Segment::new(0, 11)],
        vec![Segment::new(6, 10), Segment::new(0, 5)],
        vec![Segment::new(0, 10), Segment::new(0, 10)],
        vec![Segment::new(0, u64::MAX)],
    ] {
        assert_invalid_input(
            &segments,
            BODY.len() as u64,
            Some(HttpResumeValidator::Etag(ETAG.into())),
        );
    }
    assert_invalid_input(&[], 0, Some(HttpResumeValidator::Etag(ETAG.into())));
}

#[test]
fn segment_api_requires_strong_etag_before_side_effects() {
    for validator in [
        None,
        Some(HttpResumeValidator::Etag("W/\"weak\"".into())),
        Some(HttpResumeValidator::Etag("".into())),
        Some(HttpResumeValidator::Etag("unquoted".into())),
        Some(HttpResumeValidator::Etag("\"bad\"quote\"".into())),
        Some(HttpResumeValidator::LastModified(
            "Wed, 21 Oct 2015 07:28:00 GMT".into(),
        )),
    ] {
        assert_invalid_input(&[Segment::new(0, 10)], BODY.len() as u64, validator);
    }
}

fn assert_rejected_response(headers: &str, status: &str, bytes: &[u8], error_text: &str) {
    let raw = response(status, headers, bytes);
    let fixture = Fixture::new(move |_| raw.clone());
    let directory = Directory::new();
    let destination = directory.0.join("download");
    let result = HttpEngine::new().download_to_segments(
        &fixture.url,
        &[Segment::new(0, 10)],
        destination.to_str().unwrap(),
        directory.0.join("partial"),
        BODY.len() as u64,
        Some(&HttpResumeValidator::Etag(ETAG.into())),
        &HttpTransferControl::new(),
        |_| Ok(()),
    );
    let error = result
        .expect_err("malformed range response must fail")
        .to_string();
    assert!(
        error.contains(error_text),
        "expected {error_text:?}, got {error:?}"
    );
    assert!(!destination.exists());
}

#[test]
fn segment_api_validates_content_range_identity_and_status() {
    for range in [
        "bytes 1-11/12",
        "bytes 0-9/11",
        "bytes 0-10/12",
        "bytes */11",
        "garbage",
        "bytes +0-10/11",
        "bytes 0-+10/11",
        "bytes 0-10/+11",
    ] {
        assert_rejected_response(
            &format!("Content-Range: {range}\r\nETag: {ETAG}\r\nContent-Length: 11"),
            "206 Partial Content",
            BODY,
            "Content-Range",
        );
    }
    assert_rejected_response(
        &format!("ETag: {ETAG}\r\nContent-Length: 11"),
        "206 Partial Content",
        BODY,
        "Content-Range",
    );
    for etag in ["\"different\"", "W/\"segments-v1\"", ""] {
        assert_rejected_response(
            &format!("Content-Range: bytes 0-10/11\r\nETag: {etag}\r\nContent-Length: 11"),
            "206 Partial Content",
            BODY,
            "validator",
        );
    }
    assert_rejected_response(
        "Content-Range: bytes 0-10/11\r\nContent-Length: 11",
        "206 Partial Content",
        BODY,
        "validator",
    );
    assert_rejected_response(
        &format!(
            "Content-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nContent-Length: 11\r\nContent-Encoding: gzip"
        ),
        "206 Partial Content",
        BODY,
        "encoding",
    );
    assert_rejected_response("Content-Length: 11", "200 OK", BODY, "range");
}

#[test]
fn segment_api_rejects_ambiguous_response_headers() {
    let base = format!("Content-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nContent-Length: 11");
    for extra in [
        "Content-Range: bytes 0-10/11".to_owned(),
        format!("ETag: {ETAG}"),
        "Content-Encoding: identity\r\nContent-Encoding: identity".to_owned(),
    ] {
        assert_rejected_response(
            &format!("{base}\r\n{extra}"),
            "206 Partial Content",
            BODY,
            "ambiguous",
        );
    }
}

#[test]
fn segment_api_rejects_declared_short_and_chunked_extra_bodies() {
    assert_rejected_response(
        &format!("Content-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nContent-Length: 10"),
        "206 Partial Content",
        &BODY[..10],
        "length",
    );
    assert_rejected_response(
        &format!("Content-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nTransfer-Encoding: chunked"),
        "206 Partial Content",
        b"A\r\nabcdefghij\r\n0\r\n\r\n",
        "incomplete",
    );
    assert_rejected_response(
        &format!("Content-Range: bytes 0-10/11\r\nETag: {ETAG}\r\nTransfer-Encoding: chunked"),
        "206 Partial Content",
        b"B\r\nabcdefghijk\r\n1\r\nx\r\n0\r\n\r\n",
        "length",
    );
}

#[test]
fn split_segments_handles_empty_and_largest_total_without_overflow() {
    assert!(HttpEngine::split_segments(0, 4).is_empty());
    let segments = HttpEngine::split_segments(u64::MAX, 3);
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].start, 0);
    assert_eq!(segments.last().unwrap().end, u64::MAX - 1);
    for pair in segments.windows(2) {
        assert_eq!(pair[0].end + 1, pair[1].start);
    }
    assert_eq!(segments.iter().map(Segment::length).sum::<u64>(), u64::MAX);
    assert_eq!(
        HttpEngine::split_segments(u64::MAX, 1),
        vec![Segment::new(0, u64::MAX - 1)]
    );
    assert_eq!(
        HttpEngine::split_segments(1, usize::MAX),
        vec![Segment::new(0, 0)]
    );
    assert_eq!(Segment::new(10, 9).length(), 0);
    assert_eq!(Segment::new(0, u64::MAX).length(), 0);
}

#[test]
fn probe_accepts_only_exact_single_byte_range_with_strong_identity() {
    let fixture = Fixture::new(valid_response);
    let capability = HttpEngine::new().probe_range(&fixture.url).unwrap();
    assert!(capability.supports_ranges);
    assert_eq!(capability.length, Some(BODY.len() as u64));
    assert_eq!(
        capability.validator,
        Some(HttpResumeValidator::Etag(ETAG.into()))
    );
    {
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requested_range(&requests[0]), (0, 0));
        assert!(
            requests[0]
                .lines()
                .any(|line| line.eq_ignore_ascii_case("accept-encoding: identity"))
        );
    }
    for (headers, bytes) in [
        (
            format!("Content-Range: bytes 1-1/11\r\nETag: {ETAG}\r\nContent-Length: 1"),
            b"a".as_slice(),
        ),
        (
            format!("Content-Range: bytes 0-1/11\r\nETag: {ETAG}\r\nContent-Length: 1"),
            b"a".as_slice(),
        ),
        (
            format!("Content-Range: bytes 0-0/11\r\nETag: W/{ETAG}\r\nContent-Length: 1"),
            b"a".as_slice(),
        ),
        (
            "Content-Range: bytes 0-0/11\r\nLast-Modified: Wed, 21 Oct 2015 07:28:00 GMT\r\nContent-Length: 1".to_owned(),
            b"a".as_slice(),
        ),
        (
            format!(
                "Content-Range: bytes 0-0/11\r\nETag: {ETAG}\r\nContent-Length: 1\r\nContent-Encoding: gzip"
            ),
            b"a".as_slice(),
        ),
        (
            format!("Content-Range: bytes 0-0/11\r\nETag: {ETAG}\r\nTransfer-Encoding: chunked"),
            b"1\r\na\r\n1\r\nb\r\n0\r\n\r\n".as_slice(),
        ),
        (
            format!("Content-Range: bytes 0-0/11\r\nETag: {ETAG}\r\nContent-Length: 0"),
            b"".as_slice(),
        ),
        (
            format!("Content-Range: bytes +0-0/11\r\nETag: {ETAG}\r\nContent-Length: 1"),
            b"a".as_slice(),
        ),
        (
            format!("Content-Range: bytes 0-0/+11\r\nETag: {ETAG}\r\nContent-Length: 1"),
            b"a".as_slice(),
        ),
        (
            format!(
                "Content-Range: bytes 0-0/11\r\nContent-Range: bytes 0-0/11\r\nETag: {ETAG}\r\nContent-Length: 1"
            ),
            b"a".as_slice(),
        ),
        (
            format!(
                "Content-Range: bytes 0-0/11\r\nETag: {ETAG}\r\nETag: {ETAG}\r\nContent-Length: 1"
            ),
            b"a".as_slice(),
        ),
        (
            format!(
                "Content-Range: bytes 0-0/11\r\nETag: {ETAG}\r\nContent-Length: 1\r\nContent-Encoding: identity\r\nContent-Encoding: identity"
            ),
            b"a".as_slice(),
        ),
    ] {
        let raw = response("206 Partial Content", &headers, bytes);
        let fixture = Fixture::new(move |_| raw.clone());
        assert!(
            !HttpEngine::new()
                .probe_range(&fixture.url)
                .is_some_and(|cap| cap.supports_ranges),
            "accepted malformed probe: {headers}"
        );
    }
}

#[test]
fn probe_has_bounded_header_and_body_wait() {
    for send_headers in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            if send_headers {
                let _ = write!(
                    stream,
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/11\r\nETag: {ETAG}\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na\r\n"
                );
            }
            thread::sleep(Duration::from_secs(4));
        });
        let started = Instant::now();
        let result = HttpEngine::new().probe_range(&url);
        let elapsed = started.elapsed();
        worker.join().unwrap();
        assert!(!result.is_some_and(|cap| cap.supports_ranges));
        assert!(
            elapsed < Duration::from_millis(3500),
            "probe waited {elapsed:?}"
        );
    }
}
