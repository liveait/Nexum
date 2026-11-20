use super::*;
use std::io::{BufRead, BufReader, Read, SeekFrom};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{atomic::Ordering, mpsc};
use std::thread;

const WAIT: Duration = Duration::from_secs(5);

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + WAIT;
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    stream.set_write_timeout(Some(WAIT)).unwrap();
    stream
}

fn origin() -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let mut socket = BufReader::new(accept(&listener));
        fn command(socket: &mut BufReader<TcpStream>, expected: &str) {
            let mut line = String::new();
            socket.read_line(&mut line).unwrap();
            assert_eq!(line, format!("{expected}\r\n"));
        }
        fn reply(socket: &mut BufReader<TcpStream>, text: &str) {
            socket.get_mut().write_all(text.as_bytes()).unwrap();
        }
        reply(&mut socket, "220 Ready\r\n");
        command(&mut socket, "USER anonymous");
        reply(&mut socket, "230 Accepted\r\n");
        command(&mut socket, "TYPE I");
        reply(&mut socket, "200 Binary\r\n");
        command(&mut socket, "SIZE file.bin");
        reply(&mut socket, "213 8\r\n");
        command(&mut socket, "EPSV");
        let data = TcpListener::bind("127.0.0.1:0").unwrap();
        reply(
            &mut socket,
            &format!(
                "229 Passive (|||{}|)\r\n",
                data.local_addr().unwrap().port()
            ),
        );
        command(&mut socket, "RETR file.bin");
        reply(&mut socket, "150 Data\r\n");
        let mut stream = accept(&data);
        stream.write_all(b"firstend").unwrap();
        drop(stream);
        reply(&mut socket, "226 Complete\r\n");
        command(&mut socket, "SIZE file.bin");
        reply(&mut socket, "213 8\r\n");
        assert_eq!(socket.read(&mut [0]).unwrap(), 0);
    });
    (endpoint, worker)
}

struct Fixture {
    root: PathBuf,
    moved_root: Option<PathBuf>,
    file: File,
    directory: File,
    binding: FtpResumeBinding,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nexum-complete-{}-{}",
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
        file.write_all(b"firstend").unwrap();
        let directory = File::open(&root).unwrap();
        let binding = FtpResumeBinding {
            server_instance_id: "instance".into(),
            task_id: "task".into(),
            operation_id: "operation".into(),
            source_origin: "ftp://localhost:21".into(),
            command_path: "file.bin".into(),
            path_mode: FtpPathMode::LoginRelative,
            destination: "/test/file.bin".into(),
            credential: None,
        };
        Self {
            root,
            moved_root: None,
            file,
            directory,
            binding,
        }
    }

    fn request(&self) -> FtpCompletionRequest<'_> {
        FtpCompletionRequest {
            binding: &self.binding,
            attempt_generation: "generation",
        }
    }

    fn proof(&mut self) -> FtpCompletionProof {
        let proof = prepare(&self.file, &self.directory, self.request()).unwrap();
        let control = FtpTransferControl::default();
        let _finish = begin(&control).unwrap();
        seal(
            &mut self.file,
            &self.directory,
            proof,
            Progress::new(8, Some(8)),
            &control,
            digest,
            File::sync_all,
        )
        .unwrap()
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

struct GatedReader<'a> {
    file: &'a mut File,
    started: Option<mpsc::Sender<()>>,
    release: mpsc::Receiver<()>,
}

impl Read for GatedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let size = self.file.read(buffer)?;
        if let Some(sender) = self.started.take() {
            sender.send(()).unwrap();
            self.release.recv_timeout(WAIT).unwrap();
        }
        Ok(size)
    }
}
impl Seek for GatedReader<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}

#[test]
fn hashing_can_pause_resume_or_cancel_without_finishing_the_producer_or_verifier() {
    for verify in [false, true] {
        for cancel in [false, true] {
            let mut fixture = Fixture::new();
            let proof = if verify {
                fixture.proof()
            } else {
                prepare(&fixture.file, &fixture.directory, fixture.request()).unwrap()
            };
            let mut file = fixture.file.try_clone().unwrap();
            let directory = fixture.directory.try_clone().unwrap();
            let binding = fixture.binding.clone();
            let peer = if verify { None } else { Some(origin()) };
            let endpoint = peer.as_ref().map(|(endpoint, _)| *endpoint);
            let control = FtpTransferControl::default();
            let worker_control = control.clone();
            let (started, ready) = mpsc::channel();
            let (release, proceed) = mpsc::channel();
            let worker = thread::spawn(move || {
                let hash = |file: &mut File, length, control: &FtpTransferControl| {
                    digest(
                        &mut GatedReader {
                            file,
                            started: Some(started),
                            release: proceed,
                        },
                        length,
                        control,
                    )
                };
                if verify {
                    verify_with(
                        &mut file,
                        &directory,
                        FtpCompletionRequest {
                            binding: &binding,
                            attempt_generation: "generation",
                        },
                        &proof,
                        &worker_control,
                        hash,
                    )
                } else {
                    let _finish = begin(&worker_control).unwrap();
                    file.set_len(0).unwrap();
                    file.rewind().unwrap();
                    let received = transfer_data(
                        endpoint.unwrap(),
                        &binding.command_path,
                        FtpCredentials::Anonymous,
                        &mut file,
                        &worker_control,
                        0,
                        None,
                    )
                    .unwrap();
                    assert_eq!(received, Progress::new(8, Some(8)));
                    assert!(
                        !worker_control.is_finished(),
                        "terminal response must leave proof hashing controllable"
                    );
                    seal(
                        &mut file,
                        &directory,
                        proof,
                        Progress::new(8, Some(8)),
                        &worker_control,
                        hash,
                        File::sync_all,
                    )
                    .map(|proof| proof.progress())
                }
            });
            ready.recv_timeout(WAIT).unwrap();
            assert!(!control.is_finished());
            let before = control.progress();
            let pause = control.clone();
            let (paused, ack) = mpsc::channel();
            let waiter = thread::spawn(move || paused.send(pause.request_pause()).unwrap());
            let (lock, wake) = &*control.0.worker.inner;
            let state = lock.lock().unwrap();
            let (state, _) = wake
                .wait_timeout_while(state, WAIT, |s| !s.pause_requested)
                .unwrap();
            let requested = state.pause_requested;
            drop(state);
            release.send(()).unwrap();
            let parked = ack.recv_timeout(WAIT);
            if parked.is_err() || !requested {
                control.cancel();
            }
            waiter.join().unwrap();
            assert!(requested);
            assert_eq!(parked.unwrap(), Ok(true));
            assert!(!control.is_finished());
            assert_eq!(control.progress(), before);
            assert_eq!(
                std::fs::read(fixture.root.join("partial")).unwrap(),
                b"firstend"
            );
            if cancel {
                control.cancel();
            } else {
                control.resume().unwrap();
            }
            let result = worker.join().unwrap();
            if let Some((_, peer)) = peer {
                peer.join().unwrap();
            }
            assert!(control.is_finished());
            if cancel {
                assert_eq!(result, Err(FtpReceiveError::Cancelled));
                assert_eq!(control.progress(), before);
            } else {
                assert_eq!(result.unwrap(), Progress::new(8, Some(8)));
            }
        }
    }
}

#[test]
fn cancellation_after_sync_prevents_a_completion_proof() {
    let mut fixture = Fixture::new();
    let proof = prepare(&fixture.file, &fixture.directory, fixture.request()).unwrap();
    let control = FtpTransferControl::default();
    let result = {
        let _finish = begin(&control).unwrap();
        seal(
            &mut fixture.file,
            &fixture.directory,
            proof,
            Progress::new(8, Some(8)),
            &control,
            digest,
            |file| {
                file.sync_all()?;
                control.cancel();
                Ok(())
            },
        )
    };
    assert!(matches!(result, Err(FtpReceiveError::Cancelled)));
    assert!(control.is_finished());
    assert_eq!(
        std::fs::read(fixture.root.join("partial")).unwrap(),
        b"firstend"
    );
}

#[test]
fn read_and_sync_failures_return_no_proof_and_retain_the_file() {
    for sync_failure in [false, true] {
        let mut fixture = Fixture::new();
        let proof = prepare(&fixture.file, &fixture.directory, fixture.request()).unwrap();
        let control = FtpTransferControl::default();
        let sync_ran = std::cell::Cell::new(false);
        let result = {
            let _finish = begin(&control).unwrap();
            seal(
                &mut fixture.file,
                &fixture.directory,
                proof,
                Progress::new(8, Some(8)),
                &control,
                |file, length, control| {
                    if sync_failure {
                        digest(file, length, control)
                    } else {
                        Err(FtpReceiveError::LocalIo(std::io::ErrorKind::Other))
                    }
                },
                |_| {
                    sync_ran.set(true);
                    Err(std::io::Error::other("injected sync failure"))
                },
            )
        };
        assert!(matches!(
            result,
            Err(FtpReceiveError::LocalIo(std::io::ErrorKind::Other))
        ));
        assert_eq!(sync_ran.get(), sync_failure);
        assert!(control.is_finished());
        assert_eq!(
            std::fs::read(fixture.root.join("partial")).unwrap(),
            b"firstend"
        );
    }
}

#[cfg(unix)]
#[test]
fn replacing_names_after_hash_keeps_proof_bound_to_the_held_objects() {
    for kind in ["regular", "symlink", "hardlink", "parent"] {
        let mut fixture = Fixture::new();
        let proof = prepare(&fixture.file, &fixture.directory, fixture.request()).unwrap();
        let root = fixture.root.clone();
        let original = root.join("partial");
        let held = root.join("held");
        let unrelated = root.join("unrelated");
        std::fs::write(&unrelated, b"unrelated content").unwrap();
        let moved = root.with_extension("held-parent");
        if kind == "parent" {
            fixture.moved_root = Some(moved.clone());
        }
        let control = FtpTransferControl::default();
        let proof = {
            let _finish = begin(&control).unwrap();
            seal(
                &mut fixture.file,
                &fixture.directory,
                proof,
                Progress::new(8, Some(8)),
                &control,
                digest,
                |file| {
                    file.sync_all()?;
                    if kind == "parent" {
                        std::fs::rename(&root, &moved).unwrap();
                        std::fs::create_dir(&root).unwrap();
                        std::fs::write(&original, b"unrelated content").unwrap();
                        std::fs::write(&unrelated, b"unrelated content").unwrap();
                    } else {
                        std::fs::rename(&original, &held).unwrap();
                        match kind {
                            "regular" => std::fs::write(&original, b"unrelated content").unwrap(),
                            "symlink" => std::os::unix::fs::symlink(&unrelated, &original).unwrap(),
                            "hardlink" => std::fs::hard_link(&unrelated, &original).unwrap(),
                            _ => unreachable!(),
                        }
                    }
                    Ok(())
                },
            )
            .unwrap()
        };
        let binding = fixture.binding.clone();
        assert_eq!(
            verify_completion(
                &mut fixture.file,
                &fixture.directory,
                FtpCompletionRequest {
                    binding: &binding,
                    attempt_generation: "generation"
                },
                &proof,
                &FtpTransferControl::default()
            )
            .unwrap(),
            Progress::new(8, Some(8))
        );
        assert_eq!(std::fs::read(&original).unwrap(), b"unrelated content");
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated content");
        if kind == "parent" {
            assert_eq!(
                std::fs::read(moved.join("unrelated")).unwrap(),
                b"unrelated content"
            );
        }
    }
}

#[test]
fn offline_verification_rechecks_length_after_hash_and_preserves_read_errors() {
    for shorten in [false, true] {
        let mut fixture = Fixture::new();
        let proof = fixture.proof();
        let binding = fixture.binding.clone();
        let control = FtpTransferControl::default();
        let result = verify_with(
            &mut fixture.file,
            &fixture.directory,
            FtpCompletionRequest {
                binding: &binding,
                attempt_generation: "generation",
            },
            &proof,
            &control,
            |file, length, control| {
                if shorten {
                    let hash = digest(file, length, control)?;
                    file.set_len(3).unwrap();
                    Ok(hash)
                } else {
                    Err(FtpReceiveError::LocalIo(std::io::ErrorKind::Other))
                }
            },
        );
        let expected = if shorten {
            FtpReceiveError::LocalPrefixMismatch
        } else {
            FtpReceiveError::LocalIo(std::io::ErrorKind::Other)
        };
        assert_eq!(result, Err(expected));
        assert!(control.is_finished());
        assert!(control.progress().is_none());
    }
}

#[test]
fn cancellation_after_offline_hash_does_not_publish_verified_progress() {
    let mut fixture = Fixture::new();
    let proof = fixture.proof();
    let binding = fixture.binding.clone();
    let control = FtpTransferControl::default();
    let result = verify_with(
        &mut fixture.file,
        &fixture.directory,
        FtpCompletionRequest {
            binding: &binding,
            attempt_generation: "generation",
        },
        &proof,
        &control,
        |file, length, control| {
            let hash = digest(file, length, control)?;
            control.cancel();
            Ok(hash)
        },
    );
    assert_eq!(result, Err(FtpReceiveError::Cancelled));
    assert!(control.is_finished());
    assert!(
        control.progress().is_none(),
        "cancelled verification must not publish verified progress"
    );
}

#[test]
fn sealing_rejects_incomplete_progress_and_verifiers_cannot_reuse_controls() {
    let mut fixture = Fixture::new();
    let initial = prepare(&fixture.file, &fixture.directory, fixture.request()).unwrap();
    let control = FtpTransferControl::default();
    let result = {
        let _finish = begin(&control).unwrap();
        seal(
            &mut fixture.file,
            &fixture.directory,
            initial,
            Progress::new(8, None),
            &control,
            digest,
            File::sync_all,
        )
    };
    assert!(matches!(result, Err(FtpReceiveError::InvalidProof)));
    assert!(control.is_finished());
    let proof = fixture.proof();
    let binding = fixture.binding.clone();
    assert_eq!(
        verify_completion(
            &mut fixture.file,
            &fixture.directory,
            FtpCompletionRequest {
                binding: &binding,
                attempt_generation: "generation"
            },
            &proof,
            &control
        ),
        Err(FtpReceiveError::ControlAlreadyUsed)
    );
    let cancelled = FtpTransferControl::default();
    cancelled.cancel();
    assert_eq!(
        verify_completion(
            &mut fixture.file,
            &fixture.directory,
            FtpCompletionRequest {
                binding: &binding,
                attempt_generation: "generation"
            },
            &proof,
            &cancelled
        ),
        Err(FtpReceiveError::Cancelled)
    );
}
