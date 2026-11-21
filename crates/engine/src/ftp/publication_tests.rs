use super::*;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    root: PathBuf,
    dir: File,
    proof: FtpCompletionProof,
    binding: FtpResumeBinding,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "nexum-pub-unit-{}-{}",
            std::process::id(),
            crate::NEXT_TEMP_DOWNLOAD.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new().mode(0o700).create(&root).unwrap();
        let dir = File::open(&root).unwrap();
        let binding = FtpResumeBinding {
            server_instance_id: "instance".into(),
            task_id: "task".into(),
            operation_id: "operation".into(),
            source_origin: "ftp://localhost".into(),
            command_path: "file".into(),
            path_mode: FtpPathMode::LoginRelative,
            destination: root.join("final").to_str().unwrap().into(),
            credential: None,
        };
        let mut file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.join("part"))
            .unwrap();
        file.write_all(b"payload").unwrap();
        let proof = prepare(
            &file,
            &dir,
            FtpCompletionRequest {
                binding: &binding,
                attempt_generation: "producer",
            },
        )
        .unwrap();
        let control = FtpTransferControl::default();
        let _finish = begin(&control).unwrap();
        let proof = seal(
            &mut file,
            &dir,
            proof,
            Progress::new(7, Some(7)),
            &control,
            digest,
            File::sync_all,
        )
        .unwrap();
        Self {
            root,
            dir,
            binding,
            proof,
        }
    }

    fn request(&self) -> FtpPublicationRequest<'_> {
        FtpPublicationRequest {
            completion: FtpCompletionRequest {
                binding: &self.binding,
                attempt_generation: "producer",
            },
            proof: &self.proof,
            owner_generation: "owner",
            authorization: "token",
        }
    }
    fn plan(&self) -> FtpPublicationPlan {
        FtpPublicationPlan::prepare(
            &File::open(self.root.join("part")).unwrap(),
            &self.dir,
            "part",
            self.request(),
        )
        .unwrap()
    }
    fn operation<'a>(
        &'a self,
        plan: &'a FtpPublicationPlan,
        control: &'a FtpTransferControl,
        reconcile: bool,
    ) -> Publication<'a> {
        Publication {
            directory: &self.dir,
            plan,
            request: self.request(),
            control,
            reconcile,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn cancel_winning_before_syscall_prevents_publication() {
    let f = Fixture::new();
    let plan = f.plan();
    for reconcile in [false, true] {
        let control = FtpTransferControl::default();
        let result = f.operation(&plan, &control, reconcile).run(
            || -> std::result::Result<(), _> { panic!("guard after cancellation") },
            |file, len, control| {
                let hash = digest(file, len, control)?;
                control.cancel();
                Ok(hash)
            },
            File::sync_all,
        );
        assert_eq!(
            result.unwrap_err(),
            FtpPublicationError::Transfer(FtpReceiveError::Cancelled)
        );
        assert!(control.is_finished() && !control.is_committed());
        assert!(!f.root.join("final").exists());
    }
}

#[test]
fn publication_winning_before_cancel_remains_committed() {
    let f = Fixture::new();
    let plan = f.plan();
    let control = FtpTransferControl::default();
    let result = f.operation(&plan, &control, false).run(
        || Ok(()),
        digest,
        |dir| {
            assert!(control.is_committed() && !control.is_finished());
            control.cancel();
            dir.sync_all()
        },
    );
    assert!(result.is_ok());
    assert!(control.is_finished() && control.is_committed());
    assert_eq!(fs::read(f.root.join("final")).unwrap(), b"payload");
}

#[test]
fn pause_and_cancel_during_hash_preserve_every_entry() {
    // Gate the first read of the actual chunked digest. The real control
    // acknowledges pause at the checkpoint after this read.
    struct GatedRead<'a> {
        file: &'a mut File,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl Read for GatedRead<'_> {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let n = self.file.read(b)?;
            self.entered.send(()).unwrap();
            self.release.recv_timeout(WAIT).unwrap();
            Ok(n)
        }
    }
    impl Seek for GatedRead<'_> {
        fn seek(&mut self, p: io::SeekFrom) -> io::Result<u64> {
            self.file.seek(p)
        }
    }
    let f = Fixture::new();
    let plan = f.plan();
    for reconcile in [false, true] {
        let control = FtpTransferControl::default();
        let op = f.operation(&plan, &control, reconcile);
        let (entered, ready) = mpsc::channel();
        let (release, go) = mpsc::channel();
        thread::scope(|scope| {
            let worker = scope.spawn(move || {
                op.run(
                    || Ok(()),
                    |file, len, c| {
                        digest(
                            &mut GatedRead {
                                file,
                                entered,
                                release: go,
                            },
                            len,
                            c,
                        )
                    },
                    File::sync_all,
                )
            });
            ready.recv_timeout(WAIT).unwrap();
            let waiter = scope.spawn(|| control.request_pause());
            let (lock, wake) = &*control.0.worker.inner;
            let (state, _) = wake
                .wait_timeout_while(lock.lock().unwrap(), WAIT, |s| !s.pause_requested)
                .unwrap();
            let requested = state.pause_requested;
            drop(state);
            release.send(()).unwrap();
            assert!(requested);
            assert_eq!(waiter.join().unwrap(), Ok(true));
            assert!(!f.root.join("final").exists() && !control.is_finished());
            control.cancel();
            assert_eq!(
                worker.join().unwrap().unwrap_err(),
                FtpPublicationError::Transfer(FtpReceiveError::Cancelled)
            );
        });
        assert!(control.is_finished() && !control.is_committed());
        assert_eq!(fs::read(f.root.join("part")).unwrap(), b"payload");
        assert!(!f.root.join("final").exists());
    }
}

struct Releaser<F>(F);
impl<F: FnOnce() -> std::result::Result<(), FtpReceiveError>> FtpCompletionGuard for Releaser<F> {
    fn release(self) -> std::result::Result<(), FtpReceiveError> {
        self.0()
    }
}

#[test]
fn release_validation_and_sync_failures_preserve_committed_intent() {
    for failure in ["release", "validation", "sync"] {
        for existing in [false, true] {
            let f = Fixture::new();
            let plan = f.plan();
            let final_entry = f.root.join("final");
            if existing {
                fs::hard_link(f.root.join("part"), &final_entry).unwrap();
            }
            let control = FtpTransferControl::default();
            let mut synced = false;
            let result = f.operation(&plan, &control, existing).run(
                || {
                    Ok(Releaser(|| {
                        if failure == "release" {
                            return Err(FtpReceiveError::LocalIo(io::ErrorKind::Other));
                        }
                        if failure == "validation" {
                            fs::rename(&final_entry, f.root.join("old")).unwrap();
                            fs::write(&final_entry, b"replace").unwrap();
                        }
                        Ok(())
                    }))
                },
                digest,
                |dir| {
                    synced = true;
                    assert!(control.is_committed() && !control.is_finished());
                    if failure == "sync" {
                        Err(io::Error::other("sync"))
                    } else {
                        dir.sync_all()
                    }
                },
            );
            assert!(synced && control.is_committed() && control.is_finished());
            assert!(result.is_err());
            assert_eq!(fs::read(f.root.join("part")).unwrap(), b"payload");
            assert_eq!(
                fs::read(final_entry).unwrap(),
                if failure == "validation" {
                    b"replace"
                } else {
                    b"payload"
                }
            );
        }
    }
}

#[test]
fn rechecks_after_guard_acquisition_and_releases_on_identity_failure() {
    let f = Fixture::new();
    let plan = f.plan();
    let control = FtpTransferControl::default();
    let mut released = false;
    let result = f.operation(&plan, &control, false).run(
        || {
            fs::rename(f.root.join("part"), f.root.join("original")).unwrap();
            fs::write(f.root.join("part"), b"payload").unwrap();
            Ok(Releaser(|| {
                released = true;
                Ok(())
            }))
        },
        digest,
        File::sync_all,
    );
    assert!(result.is_err() && released);
    assert!(control.is_finished() && !control.is_committed());
    assert!(!f.root.join("final").exists());
}

#[test]
fn rejects_aliased_and_non_normalized_destinations() {
    let f = Fixture::new();
    let file = File::open(f.root.join("part")).unwrap();
    let plan = f.plan();
    for bad in [
        format!("{}/./final", f.root.display()),
        format!("{}//final", f.root.display()),
        format!("{}/final/", f.root.display()),
        "relative".into(),
    ] {
        let mut data = plan.0.clone();
        data.completion.binding.destination = bad;
        assert!(FtpPublicationPlan::decode(&serde_json::to_vec(&data).unwrap()).is_err());
    }
    if f.root.join("PART").exists() {
        assert!(FtpPublicationPlan::prepare(&file, &f.dir, "PART", f.request()).is_err());
    }
}

#[test]
fn denied_guard_prevents_link_and_adoption() {
    let f = Fixture::new();
    let plan = f.plan();
    for existing in [false, true] {
        if existing {
            fs::hard_link(f.root.join("part"), f.root.join("final")).unwrap();
        }
        let control = FtpTransferControl::default();
        let error = f
            .operation(&plan, &control, existing)
            .run(
                || Err::<(), _>(FtpReceiveError::ResumeBindingMismatch),
                digest,
                |_| panic!("directory sync after denied guard"),
            )
            .unwrap_err();
        assert_eq!(
            error,
            FtpPublicationError::Transfer(FtpReceiveError::ResumeBindingMismatch)
        );
        assert_eq!(f.root.join("final").exists(), existing);
        assert!(control.is_finished() && !control.is_committed());
    }
}

#[test]
fn directory_sync_failure_finishes_and_wakes_pending_pause() {
    let f = Fixture::new();
    let plan = f.plan();
    let control = FtpTransferControl::default();
    let op = f.operation(&plan, &control, false);
    let (started, ready) = mpsc::channel();
    let (release, go) = mpsc::channel();
    thread::scope(|scope| {
        let worker = scope.spawn(move || {
            op.run(
                || Ok(()),
                digest,
                |_| {
                    started.send(()).unwrap();
                    go.recv_timeout(WAIT).unwrap();
                    Err(io::Error::other("directory sync"))
                },
            )
        });
        ready.recv_timeout(WAIT).unwrap();
        assert!(control.is_committed() && !control.is_finished());
        let waiter = scope.spawn(|| control.request_pause());
        let (lock, wake) = &*control.0.worker.inner;
        let (state, _) = wake
            .wait_timeout_while(lock.lock().unwrap(), WAIT, |s| !s.pause_requested)
            .unwrap();
        let requested = state.pause_requested;
        drop(state);
        release.send(()).unwrap();
        assert!(requested);
        assert_eq!(
            worker.join().unwrap().unwrap_err(),
            FtpPublicationError::Namespace(io::ErrorKind::Other)
        );
        assert_eq!(waiter.join().unwrap(), Ok(false));
    });
    assert!(control.is_committed() && control.is_finished());
    assert_eq!(fs::read(f.root.join("final")).unwrap(), b"payload");
}
