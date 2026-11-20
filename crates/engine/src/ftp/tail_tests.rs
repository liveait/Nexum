use super::*;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{atomic::Ordering, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    root: PathBuf,
    moved_root: Option<PathBuf>,
    file: File,
    directory: File,
    binding: FtpResumeBinding,
    proof: FtpPrefixProof,
}

fn binding() -> FtpResumeBinding {
    FtpResumeBinding {
        server_instance_id: "tail-server".into(),
        task_id: "tail-task".into(),
        operation_id: "tail-operation".into(),
        source_origin: "ftp://localhost:21".into(),
        command_path: "file.bin".into(),
        path_mode: FtpPathMode::LoginRelative,
        destination: "/test/file.bin".into(),
        credential: None,
    }
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nexum-tail-unit-{}-{}",
            std::process::id(),
            crate::NEXT_TEMP_DOWNLOAD.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.join("partial"))
            .unwrap();
        file.write_all(b"first").unwrap();
        let directory = File::open(&root).unwrap();
        let binding = binding();
        let proof = checkpoint(
            FtpCheckpointRequest {
                binding: &binding,
                attempt_generation: "old-attempt",
                trusted_bytes: 5,
                total_bytes: Some(8),
            },
            &mut file,
            &directory,
            &FtpTransferControl::default(),
        )
        .unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"uncommitted").unwrap();
        Self {
            root,
            moved_root: None,
            file,
            directory,
            binding,
            proof,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
        if let Some(root) = &self.moved_root {
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

fn request<'a>(binding: &'a FtpResumeBinding, proof: &'a FtpPrefixProof) -> FtpResumeRequest<'a> {
    FtpResumeRequest {
        binding,
        proof,
        attempt_generation: "new-attempt",
    }
}

#[test]
fn entry_replacements_after_validation_cannot_redirect_the_truncation() {
    for kind in ["regular", "symlink", "hardlink", "parent"] {
        let mut fixture = Fixture::new();
        let original = fixture.root.join("partial");
        let held = fixture.root.join("held");
        let decoy = fixture.root.join("unrelated");
        std::fs::write(&decoy, b"unrelated content").unwrap();
        let root = fixture.root.clone();
        let moved_parent = root.with_extension("held-parent");
        if kind == "parent" {
            fixture.moved_root = Some(moved_parent.clone());
        }
        let progress = repair_tail(
            &mut fixture.file,
            &fixture.directory,
            request(&fixture.binding, &fixture.proof),
            &FtpTransferControl::default(),
            || {
                if kind == "parent" {
                    std::fs::rename(&root, &moved_parent).unwrap();
                    std::fs::create_dir(&root).unwrap();
                    std::fs::write(&original, b"unrelated content").unwrap();
                    std::fs::write(&decoy, b"unrelated content").unwrap();
                } else {
                    std::fs::rename(&original, &held).unwrap();
                    match kind {
                        "regular" => std::fs::write(&original, b"unrelated content").unwrap(),
                        "symlink" => std::os::unix::fs::symlink(&decoy, &original).unwrap(),
                        "hardlink" => std::fs::hard_link(&decoy, &original).unwrap(),
                        _ => unreachable!(),
                    }
                }
            },
            File::sync_all,
        )
        .unwrap();
        assert_eq!(progress, Progress::new(5, Some(8)));
        let repaired = if kind == "parent" {
            moved_parent.join("partial")
        } else {
            held
        };
        assert_eq!(std::fs::read(&repaired).unwrap(), b"first");
        assert_eq!(std::fs::read(&original).unwrap(), b"unrelated content");
        assert_eq!(std::fs::read(&decoy).unwrap(), b"unrelated content");
        if kind == "parent" {
            assert_eq!(
                std::fs::read(moved_parent.join("unrelated")).unwrap(),
                b"unrelated content"
            );
        }
    }
}

#[test]
fn cancellation_winning_the_final_decision_prevents_truncation() {
    let mut fixture = Fixture::new();
    let control = FtpTransferControl::default();
    let before = std::fs::read(fixture.root.join("partial")).unwrap();
    let result = repair_tail(
        &mut fixture.file,
        &fixture.directory,
        request(&fixture.binding, &fixture.proof),
        &control,
        || control.cancel(),
        File::sync_all,
    );
    assert_eq!(result, Err(FtpReceiveError::Cancelled));
    assert_eq!(std::fs::read(fixture.root.join("partial")).unwrap(), before);
    assert!(control.progress().is_none());
    assert!(control.is_finished());
}

#[test]
fn a_pause_after_validation_preserves_bytes_until_resumed_or_cancelled() {
    for cancelled in [false, true] {
        let fixture = Fixture::new();
        let path = fixture.root.join("partial");
        let before = std::fs::read(&path).unwrap();
        let mut file = fixture.file.try_clone().unwrap();
        let directory = fixture.directory.try_clone().unwrap();
        let proof = fixture.proof.clone();
        let binding = fixture.binding.clone();
        let control = FtpTransferControl::default();
        let worker_control = control.clone();
        let (validated, ready) = mpsc::channel();
        let (proceed, allowed) = mpsc::channel();
        let worker = thread::spawn(move || {
            repair_tail(
                &mut file,
                &directory,
                request(&binding, &proof),
                &worker_control,
                || {
                    validated.send(()).unwrap();
                    allowed.recv_timeout(WAIT).unwrap();
                },
                File::sync_all,
            )
        });
        ready.recv_timeout(WAIT).unwrap();
        let pause_control = control.clone();
        let (acknowledged, ack) = mpsc::channel();
        let waiter =
            thread::spawn(move || acknowledged.send(pause_control.request_pause()).unwrap());
        let (lock, wake) = &*control.0.worker.inner;
        let state = lock.lock().unwrap();
        let (state, _) = wake
            .wait_timeout_while(state, WAIT, |state| !state.pause_requested)
            .unwrap();
        let requested = state.pause_requested;
        drop(state);
        proceed.send(()).unwrap();
        let paused = ack.recv_timeout(WAIT);
        if paused.is_err() || !requested {
            control.cancel();
        }
        waiter.join().unwrap();
        assert!(requested);
        assert_eq!(paused.unwrap(), Ok(true));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(control.progress().is_none());
        if cancelled {
            control.cancel();
        } else {
            control.resume().unwrap();
        }
        let result = worker.join().unwrap();
        if cancelled {
            assert_eq!(result, Err(FtpReceiveError::Cancelled));
            assert_eq!(std::fs::read(&path).unwrap(), before);
        } else {
            assert_eq!(result.unwrap(), Progress::new(5, Some(8)));
            assert_eq!(std::fs::read(&path).unwrap(), b"first");
        }
    }
}

#[test]
fn repair_winning_the_decision_completes_before_late_controls() {
    for cancel in [false, true] {
        let fixture = Fixture::new();
        let mut file = fixture.file.try_clone().unwrap();
        let directory = fixture.directory.try_clone().unwrap();
        let binding = fixture.binding.clone();
        let proof = fixture.proof.clone();
        let control = FtpTransferControl::default();
        let worker_control = control.clone();
        let (entered, ready) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        let worker = thread::spawn(move || {
            repair_tail(
                &mut file,
                &directory,
                request(&binding, &proof),
                &worker_control,
                || {},
                |file| {
                    entered.send(()).unwrap();
                    proceed.recv_timeout(WAIT).unwrap();
                    file.sync_all()
                },
            )
        });
        ready.recv_timeout(WAIT).unwrap();
        let late_control = control.clone();
        let (started, start) = mpsc::channel();
        let late = thread::spawn(move || {
            started.send(()).unwrap();
            if cancel {
                late_control.cancel();
                Ok(false)
            } else {
                late_control.request_pause()
            }
        });
        start.recv_timeout(WAIT).unwrap();
        release.send(()).unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), Progress::new(5, Some(8)));
        assert_eq!(late.join().unwrap(), Ok(false));
        assert!(control.is_finished());
        assert_eq!(
            std::fs::read(fixture.root.join("partial")).unwrap(),
            b"first"
        );
    }
}

#[test]
fn truncate_and_sync_failures_preserve_proof_and_allow_an_explicit_retry() {
    let mut fixture = Fixture::new();
    let encoded = fixture.proof.encode().unwrap();
    let original = std::fs::read(fixture.root.join("partial")).unwrap();
    let mut readonly = File::open(fixture.root.join("partial")).unwrap();
    let failed = repair_uncommitted_tail(
        &mut readonly,
        &fixture.directory,
        request(&fixture.binding, &fixture.proof),
        &FtpTransferControl::default(),
    );
    assert!(matches!(failed, Err(FtpReceiveError::LocalIo(_))));
    assert_eq!(
        std::fs::read(fixture.root.join("partial")).unwrap(),
        original
    );
    let failed = repair_tail(
        &mut fixture.file,
        &fixture.directory,
        request(&fixture.binding, &fixture.proof),
        &FtpTransferControl::default(),
        || {},
        |_| Err(io::Error::from(io::ErrorKind::Other)),
    );
    assert_eq!(failed, Err(FtpReceiveError::LocalIo(io::ErrorKind::Other)));
    assert_eq!(
        std::fs::read(fixture.root.join("partial")).unwrap(),
        b"first"
    );
    assert_eq!(fixture.proof.encode().unwrap(), encoded);
    repair_uncommitted_tail(
        &mut fixture.file,
        &fixture.directory,
        request(&fixture.binding, &fixture.proof),
        &FtpTransferControl::default(),
    )
    .unwrap();
    assert_eq!(fixture.proof.encode().unwrap(), encoded);
}

#[test]
fn child_termination_around_truncation_allows_idempotent_repair() {
    const CHILD: &str = "NEXUM_TAIL_REPAIR_CHILD";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(root.join("partial"))
            .unwrap();
        let directory = File::open(&root).unwrap();
        let proof =
            FtpPrefixProof::decode(&std::fs::read(root.join("proof.json")).unwrap()).unwrap();
        let binding = binding();
        let phase = std::env::var("NEXUM_TAIL_REPAIR_PHASE").unwrap();
        let gate = |file: &File| {
            let mut connection =
                std::net::TcpStream::connect(std::env::var("NEXUM_TAIL_REPAIR_GATE").unwrap())
                    .unwrap();
            connection
                .write_all(&file.metadata().unwrap().len().to_be_bytes())
                .unwrap();
            loop {
                thread::park();
            }
        };
        let marker_file = file.try_clone().unwrap();
        repair_tail(
            &mut file,
            &directory,
            request(&binding, &proof),
            &FtpTransferControl::default(),
            || {
                if phase == "before" {
                    gate(&marker_file);
                }
            },
            |file| {
                gate(file);
                unreachable!()
            },
        )
        .unwrap();
        return;
    }
    for phase in ["before", "after"] {
        let mut fixture = Fixture::new();
        let encoded = fixture.proof.encode().unwrap();
        std::fs::write(fixture.root.join("proof.json"), &encoded).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ftp::resume::tail_tests::child_termination_around_truncation_allows_idempotent_repair", "--nocapture"])
            .env(CHILD, &fixture.root).env("NEXUM_TAIL_REPAIR_PHASE", phase)
            .env("NEXUM_TAIL_REPAIR_GATE", listener.local_addr().unwrap().to_string())
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        let began = Instant::now();
        let connection = loop {
            match listener.accept() {
                Ok((connection, _)) => break Some(connection),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && began.elapsed() < WAIT =>
                {
                    thread::sleep(Duration::from_millis(5));
                }
                _ => break None,
            }
        };
        if connection.is_none() {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "child did not reach repair gate: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let mut connection = connection.unwrap();
        connection.set_nonblocking(false).unwrap();
        connection.set_read_timeout(Some(WAIT)).unwrap();
        let mut length = [0; 8];
        connection.read_exact(&mut length).unwrap();
        child.kill().unwrap();
        let status = child.wait().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9));
        assert_eq!(
            u64::from_be_bytes(length),
            if phase == "before" { 16 } else { 5 }
        );
        fixture.file = File::options()
            .read(true)
            .write(true)
            .open(fixture.root.join("partial"))
            .unwrap();
        repair_uncommitted_tail(
            &mut fixture.file,
            &fixture.directory,
            request(&fixture.binding, &fixture.proof),
            &FtpTransferControl::default(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(fixture.root.join("partial")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(fixture.root.join("proof.json")).unwrap(),
            encoded
        );
    }
}
