//! Tests of the private completion boundaries called by both segment entries.
use super::*;
use std::fs;
use std::sync::mpsc;

static NEXT_COMPLETION_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const SOURCE: &str = "http://127.0.0.1:1/offline-segment-proof";
const BYTES: &[u8] = b"complete segment bytes";

struct Fixture {
    root: PathBuf,
    partial: PathBuf,
    destination: PathBuf,
    context: CompletionContext,
    staging: segment_staging::SegmentStaging,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nexum-segment-finish-{}-{}",
            std::process::id(),
            NEXT_COMPLETION_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("data")).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let partial = root.join("partial");
        let staging =
            segment_staging::SegmentStaging::create(&partial, BYTES.len() as u64).unwrap();
        staging.write_all_at(BYTES, 0).unwrap();
        Self {
            destination: root.join("destination"),
            context: CompletionContext::new("segment-phase", root.join("data")),
            root,
            partial,
            staging,
        }
    }
    fn prepare(
        &self,
        control: &HttpTransferControl,
        sync: impl FnOnce(&Path) -> Result<(), EngineError>,
    ) -> Result<completion::CompletionReceipt, completion::SegmentPreparationError> {
        completion::prepare_segment_completion_with_sync(
            &self.context,
            SOURCE,
            self.destination.to_str().unwrap(),
            &self.partial,
            self.staging.writer(),
            &Progress::new(BYTES.len() as u64, Some(BYTES.len() as u64)),
            control,
            sync,
        )
    }
    fn receipt(&self) -> PathBuf {
        completion_receipt_path(&self.context)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn journal_directory_sync_failure_after_commit_preserves_proof_and_complete_bytes() {
    let fixture = Fixture::new();
    let control = HttpTransferControl::new();
    let prepared = fixture.prepare(&control, |_| {
        Err(EngineError::Failed(
            "injected journal directory sync failure".into(),
        ))
    });
    assert!(prepared.as_ref().err().unwrap().receipt.is_some());
    let result = finish_segment_prepared(&fixture.staging, &fixture.context, prepared, &control);
    assert_eq!(
        result.unwrap_err(),
        EngineError::Failed("injected journal directory sync failure".into())
    );
    assert_eq!(fs::read(&fixture.partial).unwrap(), BYTES);
    assert!(fixture.receipt().exists());
    assert!(!fixture.destination.exists());
    let recovered = HttpEngine::new()
        .download_to_segments_with_completion(
            SOURCE,
            &[Segment::new(0, BYTES.len() as u64 - 1)],
            fixture.destination.to_str().unwrap(),
            &fixture.partial,
            BYTES.len() as u64,
            Some(&HttpResumeValidator::Etag("\"proof\"".into())),
            &fixture.context,
            &HttpTransferControl::new(),
            |_| panic!("receipt reconciliation must stay offline"),
        )
        .unwrap();
    assert_eq!(recovered.downloaded_bytes, BYTES.len() as u64);
    assert_eq!(fs::read(&fixture.destination).unwrap(), BYTES);
}

#[test]
fn cancellation_after_receipt_publication_clears_only_its_proof_and_staging() {
    let fixture = Fixture::new();
    let sidecar = resumable_metadata_path(&fixture.partial);
    fs::write(&sidecar, b"unowned metadata").unwrap();
    let control = HttpTransferControl::new();
    let prepared = fixture.prepare(&control, |_| {
        control.cancel();
        Ok(())
    });
    assert!(prepared.is_ok());
    assert!(fixture.receipt().exists());
    assert_eq!(
        finish_segment_prepared(&fixture.staging, &fixture.context, prepared, &control),
        Err(EngineError::Cancelled)
    );
    assert!(!fixture.receipt().exists());
    assert!(!fixture.partial.exists());
    assert!(!fixture.destination.exists());
    assert_eq!(fs::read(sidecar).unwrap(), b"unowned metadata");
}

#[test]
fn cancellation_cannot_delete_a_replaced_receipt_or_silently_recover_it() {
    let fixture = Fixture::new();
    let control = HttpTransferControl::new();
    let prepared = fixture.prepare(&control, |_| Ok(()));
    fs::write(fixture.receipt(), b"{unrelated proof").unwrap();
    control.cancel();
    let error = finish_segment_prepared(&fixture.staging, &fixture.context, prepared, &control)
        .unwrap_err();
    assert!(control.is_cancelled() && !control.is_committed());
    assert!(
        error
            .to_string()
            .contains("cancellation cleanup incomplete")
    );
    assert_eq!(fs::read(&fixture.partial).unwrap(), BYTES);
    assert_eq!(fs::read(fixture.receipt()).unwrap(), b"{unrelated proof");
    let result = HttpEngine::new().download_to_segments_with_completion(
        SOURCE,
        &[Segment::new(0, BYTES.len() as u64 - 1)],
        fixture.destination.to_str().unwrap(),
        &fixture.partial,
        BYTES.len() as u64,
        Some(&HttpResumeValidator::Etag("\"proof\"".into())),
        &fixture.context,
        &HttpTransferControl::new(),
        |_| panic!("must fail before HTTP"),
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("completion receipt")
    );
    assert!(!fixture.destination.exists());
}

#[test]
fn cancellation_with_a_replaced_partial_preserves_receipt_and_reports_incomplete_cleanup() {
    let fixture = Fixture::new();
    let control = HttpTransferControl::new();
    let prepared = fixture.prepare(&control, |_| Ok(()));
    let receipt = fs::read(fixture.receipt()).unwrap();
    fs::rename(&fixture.partial, fixture.root.join("actual-download")).unwrap();
    fs::write(&fixture.partial, b"foreign file").unwrap();
    control.cancel();
    let error = finish_segment_prepared(&fixture.staging, &fixture.context, prepared, &control)
        .unwrap_err();
    assert!(control.is_cancelled() && !control.is_committed());
    assert!(
        error
            .to_string()
            .contains("cancellation cleanup incomplete")
    );
    assert_eq!(fs::read(fixture.receipt()).unwrap(), receipt);
    assert_eq!(fs::read(&fixture.partial).unwrap(), b"foreign file");
    assert_eq!(
        fs::read(fixture.root.join("actual-download")).unwrap(),
        BYTES
    );
}

#[test]
fn cancellation_winning_publication_lock_prevents_the_commit_callback() {
    let control = HttpTransferControl::new();
    control.cancel();
    assert_eq!(
        control.finish_publication(|_| panic!("cancelled transfer cannot publish")),
        Err(EngineError::Cancelled)
    );
    assert!(!control.is_committed());
}

#[test]
fn cancellation_queued_during_commit_cannot_change_success_into_cancelled() {
    let control = HttpTransferControl::new();
    let child = control.clone();
    let (commit_tx, commit_rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    let canceller = thread::spawn(move || {
        commit_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        cancel_tx.send(()).unwrap();
        child.cancel();
    });
    control
        .finish_publication(|committed| {
            *committed = true;
            commit_tx.send(()).unwrap();
            cancel_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(())
        })
        .unwrap();
    canceller.join().unwrap();
    assert!(control.is_committed() && control.is_finished());
    assert!(!control.is_cancelled());
}

#[test]
fn post_commit_error_preserves_commit_state_and_is_never_cancelled() {
    let control = HttpTransferControl::new();
    let error = control
        .finish_publication(|committed| {
            *committed = true;
            Err(EngineError::Failed(
                "injected post-commit sync failure".into(),
            ))
        })
        .unwrap_err();
    control.cancel();
    assert_eq!(
        error,
        EngineError::Failed("injected post-commit sync failure".into())
    );
    assert!(control.is_committed() && control.is_finished());
    assert!(!control.is_cancelled());
}
