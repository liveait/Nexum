use nexum_engine::{
    CompletionContext, EngineError, HttpEngine, HttpResumeValidator, HttpTransferControl,
    cleanup_completion_partial, clear_completion, completion_receipt_path, resumable_metadata_path,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);
const BODY: &[u8] = b"complete segmented bytes";
const ETAG: &str = "\"segment-proof-v1\"";

struct Directory {
    root: PathBuf,
    destination: PathBuf,
    partial: PathBuf,
    context: CompletionContext,
}
impl Directory {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nexum-segment-completion-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("data")).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        Self {
            destination: root.join("file"),
            partial: root.join("partial"),
            context: CompletionContext::new("segment-task", root.join("data")),
            root,
        }
    }
    fn download(
        &self,
        source: &str,
        control: &HttpTransferControl,
        progress: impl FnMut(nexum_domain::Progress) -> Result<(), EngineError>,
    ) -> Result<nexum_domain::Progress, EngineError> {
        HttpEngine::new().download_to_segments_with_completion(
            source,
            &HttpEngine::split_segments(BODY.len() as u64, 3),
            self.destination.to_str().unwrap(),
            &self.partial,
            BODY.len() as u64,
            Some(&HttpResumeValidator::Etag(ETAG.into())),
            &self.context,
            control,
            progress,
        )
    }
    fn sidecar(&self) -> PathBuf {
        resumable_metadata_path(&self.partial)
    }
    fn receipt(&self) -> PathBuf {
        completion_receipt_path(&self.context)
    }
    fn acknowledge(&self, source: &str) {
        cleanup_completion_partial(
            &self.context,
            source,
            self.destination.to_str().unwrap(),
            &self.partial,
        )
        .unwrap();
        clear_completion(&self.context).unwrap();
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Fixture {
    source: String,
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicU64>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let child_stop = Arc::clone(&stop);
        let requests = Arc::new(AtomicU64::new(0));
        let child_requests = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            let mut workers = Vec::new();
            while let Ok((mut connection, _)) = listener.accept() {
                if child_stop.load(Ordering::Acquire) {
                    break;
                }
                let requests = Arc::clone(&child_requests);
                workers.push(thread::spawn(move || {
                    connection.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    connection.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        if connection.read(&mut byte).unwrap_or(0) == 0 { return; }
                        request.push(byte[0]);
                    }
                    requests.fetch_add(1, Ordering::SeqCst);
                    let request = String::from_utf8(request).unwrap();
                    let value = request.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("range").then_some(value.trim())
                    }).unwrap();
                    let (start, end) = value.strip_prefix("bytes=").unwrap().split_once('-').unwrap();
                    let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
                    if status != 206 {
                        let _ = connection.write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        return;
                    }
                    let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nETag: {ETAG}\r\nConnection: close\r\n\r\n", BODY.len(), end-start+1);
                    let _ = connection.write_all(header.as_bytes());
                    let _ = connection.write_all(&BODY[start..=end]);
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            source: format!("http://{address}/file"),
            address,
            stop,
            requests,
            worker: Some(worker),
        }
    }
    fn close(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.stop.store(true, Ordering::Release);
            let _ = TcpStream::connect(self.address);
            worker.join().unwrap();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.close();
    }
}

#[test]
fn exact_segment_bytes_journal_identity_and_digest_until_acknowledged() {
    let directory = Directory::new();
    std::fs::write(directory.sidecar(), b"unowned metadata").unwrap();
    let fixture = Fixture::new(206);
    let control = HttpTransferControl::new();
    let result = directory
        .download(&fixture.source, &control, |_| Ok(()))
        .unwrap();
    assert_eq!(
        result,
        nexum_domain::Progress::new(BODY.len() as u64, Some(BODY.len() as u64))
    );
    assert_eq!(std::fs::read(&directory.destination).unwrap(), BODY);
    assert!(control.is_finished() && control.is_committed() && !control.is_cancelled());
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(directory.receipt()).unwrap()).unwrap();
    assert_eq!(receipt["version"], 1);
    assert_eq!(receipt["task_id"], "segment-task");
    assert_eq!(receipt["source"], fixture.source);
    assert_eq!(
        receipt["file"]["sha256"],
        format!("{:x}", Sha256::digest(BODY))
    );
    assert!(receipt["file"]["snapshot"]["identity"]["file"].is_u64());
    assert!(receipt["resume_metadata"].is_null());
    assert_eq!(
        fixture.requests.load(Ordering::SeqCst),
        HttpEngine::split_segments(BODY.len() as u64, 3).len() as u64
    );
    directory.acknowledge(&fixture.source);
    assert!(!directory.receipt().exists());
    assert!(!directory.partial.exists());
    assert_eq!(
        std::fs::read(directory.sidecar()).unwrap(),
        b"unowned metadata"
    );
    assert_eq!(std::fs::read(&directory.destination).unwrap(), BODY);
}

#[test]
fn same_entry_recovers_published_bytes_without_a_second_http_request() {
    let directory = Directory::new();
    let mut fixture = Fixture::new(206);
    directory
        .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
        .unwrap();
    fixture.close();
    let control = HttpTransferControl::new();
    let progress = directory
        .download(&fixture.source, &control, |_| {
            panic!("offline reconciliation must not invoke network progress")
        })
        .unwrap();
    assert_eq!(progress.downloaded_bytes, BODY.len() as u64);
    assert!(control.is_committed() && control.is_finished());
    assert_eq!(std::fs::read(&directory.destination).unwrap(), BODY);
    assert!(directory.receipt().exists());
}

#[test]
fn same_entry_republishes_proven_complete_staging_offline() {
    let directory = Directory::new();
    std::fs::write(directory.sidecar(), b"not owned").unwrap();
    let mut fixture = Fixture::new(206);
    directory
        .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
        .unwrap();
    fixture.close();
    std::fs::rename(&directory.destination, &directory.partial).unwrap();
    directory
        .download(&fixture.source, &HttpTransferControl::new(), |_| {
            panic!("unexpected network progress")
        })
        .unwrap();
    assert_eq!(std::fs::read(&directory.destination).unwrap(), BODY);
    directory.acknowledge(&fixture.source);
    assert_eq!(std::fs::read(directory.sidecar()).unwrap(), b"not owned");
}

#[test]
fn identical_bytes_with_another_identity_are_not_claimed() {
    let directory = Directory::new();
    let mut fixture = Fixture::new(206);
    directory
        .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
        .unwrap();
    fixture.close();
    std::fs::rename(
        &directory.destination,
        directory.root.join("actual-download"),
    )
    .unwrap();
    std::fs::write(&directory.destination, BODY).unwrap();
    assert!(
        directory
            .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
            .is_err()
    );
    assert_eq!(std::fs::read(&directory.destination).unwrap(), BODY);
    assert_eq!(
        std::fs::read(directory.root.join("actual-download")).unwrap(),
        BODY
    );
    assert!(directory.receipt().exists());
}

#[test]
fn malformed_receipt_and_source_binding_conflicts_preserve_every_file() {
    let directory = Directory::new();
    let mut fixture = Fixture::new(206);
    directory
        .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
        .unwrap();
    fixture.close();
    let receipt = std::fs::read(directory.receipt()).unwrap();
    assert!(
        directory
            .download(
                "http://127.0.0.1:1/other",
                &HttpTransferControl::new(),
                |_| Ok(())
            )
            .is_err()
    );
    assert_eq!(std::fs::read(directory.receipt()).unwrap(), receipt);
    std::fs::write(directory.receipt(), b"{invalid proof").unwrap();
    assert!(
        directory
            .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
            .is_err()
    );
    assert_eq!(std::fs::read(&directory.destination).unwrap(), BODY);
    assert_eq!(
        std::fs::read(directory.receipt()).unwrap(),
        b"{invalid proof"
    );
}

#[test]
fn publication_conflict_keeps_complete_staging_and_receipt_for_offline_retry() {
    let directory = Directory::new();
    std::fs::write(directory.sidecar(), b"not owned").unwrap();
    let mut fixture = Fixture::new(206);
    let mut occupied = false;
    let error = directory
        .download(&fixture.source, &HttpTransferControl::new(), |progress| {
            if progress.downloaded_bytes == BODY.len() as u64 && !occupied {
                std::fs::write(&directory.destination, b"foreign target").unwrap();
                occupied = true;
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert!(occupied);
    assert_eq!(std::fs::read(&directory.partial).unwrap(), BODY);
    assert_eq!(
        std::fs::read(&directory.destination).unwrap(),
        b"foreign target"
    );
    assert!(directory.receipt().exists());
    fixture.close();
    std::fs::remove_file(&directory.destination).unwrap();
    directory
        .download(&fixture.source, &HttpTransferControl::new(), |_| {
            panic!("offline retry")
        })
        .unwrap();
    directory.acknowledge(&fixture.source);
    assert_eq!(std::fs::read(directory.sidecar()).unwrap(), b"not owned");
}

#[test]
fn final_callback_cancellation_leaves_no_recoverable_receipt_or_staging() {
    let directory = Directory::new();
    std::fs::write(directory.sidecar(), b"not owned").unwrap();
    let fixture = Fixture::new(206);
    let control = HttpTransferControl::new();
    let error = directory
        .download(&fixture.source, &control, |progress| {
            if progress.downloaded_bytes == BODY.len() as u64 {
                control.cancel();
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error, EngineError::Cancelled);
    assert!(!directory.receipt().exists());
    assert!(!directory.partial.exists());
    assert!(!directory.destination.exists());
    assert_eq!(std::fs::read(directory.sidecar()).unwrap(), b"not owned");
    assert!(control.is_finished());
}

#[test]
fn response_failure_cleans_only_owned_staging_without_a_receipt() {
    let directory = Directory::new();
    std::fs::write(directory.sidecar(), b"not owned").unwrap();
    let fixture = Fixture::new(500);
    assert!(
        directory
            .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
            .is_err()
    );
    assert!(!directory.receipt().exists());
    assert!(!directory.partial.exists());
    assert!(!directory.destination.exists());
    assert_eq!(std::fs::read(directory.sidecar()).unwrap(), b"not owned");
}

#[test]
fn journal_path_failure_before_receipt_does_not_publish_or_claim_foreign_file() {
    let directory = Directory::new();
    let fixture = Fixture::new(206);
    let journal = directory.receipt().parent().unwrap().to_owned();
    let mut blocked = false;
    let error = directory
        .download(&fixture.source, &HttpTransferControl::new(), |progress| {
            if progress.downloaded_bytes == BODY.len() as u64 && !blocked {
                std::fs::write(&journal, b"foreign journal path").unwrap();
                blocked = true;
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("not a real directory"));
    assert_eq!(std::fs::read(journal).unwrap(), b"foreign journal path");
    assert!(!directory.partial.exists());
    assert!(!directory.destination.exists());
}

#[cfg(unix)]
#[test]
fn segment_receipt_and_acknowledgement_never_follow_unowned_sidecar_symlink() {
    use std::os::unix::fs::symlink;
    let directory = Directory::new();
    let victim = directory.root.join("foreign-metadata");
    std::fs::write(&victim, vec![b'u'; 100 * 1024]).unwrap();
    symlink(&victim, directory.sidecar()).unwrap();
    let fixture = Fixture::new(206);
    directory
        .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
        .unwrap();
    directory.acknowledge(&fixture.source);
    assert!(
        std::fs::symlink_metadata(directory.sidecar())
            .unwrap()
            .is_symlink()
    );
    assert_eq!(std::fs::read(victim).unwrap(), vec![b'u'; 100 * 1024]);
}

#[test]
fn offline_receipt_recovery_does_not_require_a_fresh_range_plan() {
    let directory = Directory::new();
    let mut fixture = Fixture::new(206);
    directory
        .download(&fixture.source, &HttpTransferControl::new(), |_| Ok(()))
        .unwrap();
    fixture.close();
    let recovered = HttpEngine::new()
        .download_to_segments_with_completion(
            &fixture.source,
            &[],
            directory.destination.to_str().unwrap(),
            &directory.partial,
            0,
            None,
            &directory.context,
            &HttpTransferControl::new(),
            |_| panic!("no network progress"),
        )
        .unwrap();
    assert_eq!(
        recovered,
        nexum_domain::Progress::new(BODY.len() as u64, Some(BODY.len() as u64))
    );
    assert_eq!(std::fs::read(&directory.destination).unwrap(), BODY);
}

#[test]
fn invalid_fresh_plan_without_a_receipt_never_requests_or_creates_staging() {
    let directory = Directory::new();
    let fixture = Fixture::new(206);
    let result = HttpEngine::new().download_to_segments_with_completion(
        &fixture.source,
        &[],
        directory.destination.to_str().unwrap(),
        &directory.partial,
        0,
        None,
        &directory.context,
        &HttpTransferControl::new(),
        |_| panic!("invalid plan"),
    );
    assert!(result.is_err());
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    assert!(!directory.partial.exists());
    assert!(!directory.destination.exists());
    assert!(!directory.receipt().exists());
}
