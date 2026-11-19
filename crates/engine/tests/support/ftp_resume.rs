use super::*;
use nexum_engine::ftp::{
    FtpCheckpointRequest, FtpCredentialBinding, FtpPathMode, FtpPrefixProof, FtpResumeBinding,
    FtpResumeRequest, checkpoint, resume,
};

pub(super) fn binding() -> FtpResumeBinding {
    FtpResumeBinding {
        server_instance_id: "server-instance".into(),
        task_id: "ftp-task".into(),
        operation_id: "task-incarnation".into(),
        source_origin: "ftp://localhost:2121".into(),
        command_path: "pub/a b.bin".into(),
        path_mode: FtpPathMode::LoginRelative,
        destination: "/test/download.bin".into(),
        credential: None,
    }
}

pub(super) fn proof(
    file: &mut File,
    directory: &File,
    binding: &FtpResumeBinding,
    total_bytes: Option<u64>,
) -> FtpPrefixProof {
    checkpoint(
        FtpCheckpointRequest {
            binding,
            attempt_generation: "old-attempt",
            trusted_bytes: file.metadata().unwrap().len(),
            total_bytes,
        },
        file,
        directory,
        &FtpTransferControl::default(),
    )
    .unwrap()
}

pub(super) fn resume_request<'a>(
    binding: &'a FtpResumeBinding,
    proof: &'a FtpPrefixProof,
) -> FtpResumeRequest<'a> {
    FtpResumeRequest {
        binding,
        proof,
        attempt_generation: "new-attempt",
    }
}

#[test]
fn verifies_saved_prefix_before_appending_on_one_retr() {
    let (held, mut file) = HeldFile::new();
    file.write_all(b"first").unwrap();
    let directory = File::open(&held.0).unwrap();
    let context = binding();
    let proof = proof(&mut file, &directory, &context, Some(8));
    let encoded = proof.encode().unwrap();
    let proof = FtpPrefixProof::decode(&encoded).unwrap();
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.anonymous_login(8);
        let mut data = origin.open_data();
        data.write_all(b"firstend").unwrap();
        drop(data);
        origin.reply("226 Complete\r\n");
        origin.size("213 8\r\n");
        origin.expect_closed();
    });
    let control = FtpTransferControl::default();
    let progress = resume(
        endpoint,
        FtpCredentials::Anonymous,
        &mut file,
        &directory,
        resume_request(&context, &proof),
        &control,
    )
    .unwrap();
    origin.join().unwrap();
    assert_eq!(progress.downloaded_bytes, 8);
    assert_eq!(control.progress().unwrap(), progress);
    assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"firstend");
}

#[test]
fn rejects_changed_local_bytes_identity_tail_and_parent_before_connecting() {
    for case in [
        "digest",
        "replacement",
        "tail",
        "short",
        "parent",
        "generation",
    ] {
        let (held, mut file) = HeldFile::new();
        file.write_all(b"first").unwrap();
        let mut directory = File::open(&held.0).unwrap();
        let context = binding();
        let proof = proof(&mut file, &directory, &context, Some(8));
        let (_other, other) = HeldFile::new();
        let expected = match case {
            "digest" => {
                file.rewind().unwrap();
                file.write_all(b"other").unwrap();
                FtpReceiveError::LocalPrefixMismatch
            }
            "replacement" => {
                fs::rename(held.0.join("file.part"), held.0.join("original")).unwrap();
                file = File::options()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(held.0.join("file.part"))
                    .unwrap();
                file.write_all(b"first").unwrap();
                FtpReceiveError::FileIdentityMismatch
            }
            "tail" => {
                file.seek(std::io::SeekFrom::End(0)).unwrap();
                file.write_all(b"uncommitted").unwrap();
                FtpReceiveError::UncommittedTail
            }
            "short" => {
                file.set_len(3).unwrap();
                FtpReceiveError::LocalPrefixMismatch
            }
            "parent" => {
                directory = File::open(&_other.0).unwrap();
                FtpReceiveError::FileIdentityMismatch
            }
            "generation" => FtpReceiveError::ResumeBindingMismatch,
            _ => unreachable!(),
        };
        drop(other);
        let before = fs::read(held.0.join("file.part")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let control = FtpTransferControl::default();
        let mut request = resume_request(&context, &proof);
        if case == "generation" {
            request.attempt_generation = "old-attempt";
        }
        assert_eq!(
            resume(
                listener.local_addr().unwrap(),
                FtpCredentials::Anonymous,
                &mut file,
                &directory,
                request,
                &control
            ),
            Err(expected)
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(control.is_finished());
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), before);
    }
}

#[test]
fn rejects_stale_source_destination_instance_task_and_credential_binding() {
    let (held, mut file) = HeldFile::new();
    file.write_all(b"first").unwrap();
    let directory = File::open(&held.0).unwrap();
    let context = binding();
    let proof = proof(&mut file, &directory, &context, Some(8));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    for case in 0..8 {
        let mut changed = context.clone();
        match case {
            0 => changed.server_instance_id.push('x'),
            1 => changed.task_id.push('x'),
            2 => changed.operation_id.push('x'),
            3 => changed.source_origin = "ftp://other-host:2121".into(),
            4 => changed.command_path.push('x'),
            5 => changed.destination.push('x'),
            6 => {
                changed.command_path.insert(0, '/');
                changed.path_mode = FtpPathMode::Absolute;
            }
            7 => {
                changed.credential = Some(FtpCredentialBinding {
                    credential_id: "credential".into(),
                    revision: 2,
                })
            }
            _ => unreachable!(),
        }
        assert_eq!(
            resume(
                listener.local_addr().unwrap(),
                FtpCredentials::Anonymous,
                &mut file,
                &directory,
                resume_request(&changed, &proof),
                &FtpTransferControl::default()
            ),
            Err(FtpReceiveError::ResumeBindingMismatch)
        );
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"first");
}

#[test]
fn rejects_unknown_or_oversized_proofs_and_secret_origins() {
    let (held, mut file) = HeldFile::new();
    file.write_all(b"first").unwrap();
    let directory = File::open(&held.0).unwrap();
    let context = binding();
    let proof = proof(&mut file, &directory, &context, None);
    let encoded = proof.encode().unwrap();
    for mutation in ["format", "unknown", "length"] {
        let mut value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        match mutation {
            "format" => value["format"] = "ftp-held-prefix/v99".into(),
            "unknown" => value["unknown"] = true.into(),
            "length" => value["trusted_bytes"] = u64::MAX.into(),
            _ => unreachable!(),
        }
        if mutation == "length" {
            value["total_bytes"] = 1.into();
        }
        assert!(matches!(
            FtpPrefixProof::decode(&serde_json::to_vec(&value).unwrap()),
            Err(FtpReceiveError::InvalidProof)
        ));
    }
    assert!(FtpPrefixProof::decode(&vec![b' '; 65537]).is_err());
    for origin in [
        "ftp://USER_SENTINEL:PASSWORD_SENTINEL@host",
        "ftp://host/?secret=TOKEN_SENTINEL",
    ] {
        let mut changed = context.clone();
        changed.source_origin = origin.into();
        let result = checkpoint(
            FtpCheckpointRequest {
                binding: &changed,
                attempt_generation: "attempt",
                trusted_bytes: 5,
                total_bytes: None,
            },
            &mut file,
            &directory,
            &FtpTransferControl::default(),
        );
        let error = result.unwrap_err();
        assert_eq!(error, FtpReceiveError::InvalidProof);
        assert!(!format!("{error:?} {error}").contains("SENTINEL"));
    }
}

#[test]
fn remote_prefix_mismatch_or_short_response_preserves_the_local_file() {
    for payload in [b"otherend".as_slice(), b"fir".as_slice()] {
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.login();
            origin.size("550 SIZE unavailable\r\n");
            let mut data = origin.open_data();
            data.write_all(payload).unwrap();
            drop(data);
            origin.expect_closed();
        });
        let (held, mut file) = HeldFile::new();
        file.write_all(b"first").unwrap();
        let directory = File::open(&held.0).unwrap();
        let context = binding();
        let proof = proof(&mut file, &directory, &context, None);
        let control = FtpTransferControl::default();
        assert_eq!(
            resume(
                endpoint,
                FtpCredentials::Anonymous,
                &mut file,
                &directory,
                resume_request(&context, &proof),
                &control
            ),
            Err(FtpReceiveError::RemotePrefixMismatch)
        );
        origin.join().unwrap();
        assert_eq!(control.progress().unwrap().downloaded_bytes, 5);
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"first");
    }
}

#[test]
fn prefix_comparison_can_pause_and_cancel_without_counting_retransmission() {
    let (release, sent) = mpsc::channel();
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.anonymous_login(8);
        let mut data = origin.open_data();
        data.write_all(b"fir").unwrap();
        release.send(()).unwrap();
        // Cancellation can drop the socket before consuming these bytes. Both
        // EOF and a reset prove closure; timeout and other I/O errors do not.
        match data.read(&mut [0]) {
            Ok(0) => {}
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            result => panic!("cancelled data connection did not close: {result:?}"),
        }
        origin.expect_closed();
    });
    let (held, mut file) = HeldFile::new();
    file.write_all(b"first").unwrap();
    let directory = File::open(&held.0).unwrap();
    let context = binding();
    let proof = proof(&mut file, &directory, &context, Some(8));
    let control = FtpTransferControl::default();
    let worker_control = control.clone();
    let worker = thread::spawn(move || {
        resume(
            endpoint,
            FtpCredentials::Anonymous,
            &mut file,
            &directory,
            resume_request(&context, &proof),
            &worker_control,
        )
    });
    sent.recv_timeout(WAIT).unwrap();
    pause(&control);
    assert_eq!(control.progress().unwrap().downloaded_bytes, 5);
    assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"first");
    control.cancel();
    assert_eq!(worker.join().unwrap(), Err(FtpReceiveError::Cancelled));
    origin.join().unwrap();
    assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"first");
}

#[test]
fn resumes_after_the_original_worker_process_exits() {
    const CHILD: &str = "NEXUM_FTP_RESUME_CHILD";
    if let Some(directory) = std::env::var_os(CHILD) {
        let directory = PathBuf::from(directory);
        let endpoint = std::env::var("NEXUM_FTP_RESUME_ENDPOINT")
            .unwrap()
            .parse()
            .unwrap();
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(directory.join("file.part"))
            .unwrap();
        let control = FtpTransferControl::default();
        let error = fetch(endpoint, &mut file, &control).unwrap_err();
        assert!(matches!(
            error,
            FtpReceiveError::InvalidReply(FtpStage::Completion)
        ));
        assert_eq!(control.progress().unwrap().downloaded_bytes, 5);
        let proof = proof(
            &mut file,
            &File::open(&directory).unwrap(),
            &binding(),
            Some(8),
        );
        let mut journal = File::options()
            .write(true)
            .create_new(true)
            .open(directory.join("prefix.json"))
            .unwrap();
        journal.write_all(&proof.encode().unwrap()).unwrap();
        journal.sync_all().unwrap();
        return;
    }
    let (held, file) = HeldFile::new();
    drop(file);
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.anonymous_login(8);
        let mut data = origin.open_data();
        data.write_all(b"first").unwrap();
        drop(data);
    });
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "resume::resumes_after_the_original_worker_process_exits",
            "--nocapture",
        ])
        .env(CHILD, &held.0)
        .env("NEXUM_FTP_RESUME_ENDPOINT", endpoint.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child output {}",
        String::from_utf8_lossy(&output.stdout)
    );
    origin.join().unwrap();
    let proof = FtpPrefixProof::decode(&fs::read(held.0.join("prefix.json")).unwrap()).unwrap();
    let (endpoint, origin) = spawn_origin(move |mut origin| {
        origin.anonymous_login(8);
        let mut data = origin.open_data();
        data.write_all(b"firstend").unwrap();
        drop(data);
        origin.reply("226 Complete\r\n");
        origin.size("213 8\r\n");
        origin.expect_closed();
    });
    let mut file = File::options()
        .read(true)
        .write(true)
        .open(held.0.join("file.part"))
        .unwrap();
    let progress = resume(
        endpoint,
        FtpCredentials::Anonymous,
        &mut file,
        &File::open(&held.0).unwrap(),
        resume_request(&binding(), &proof),
        &FtpTransferControl::default(),
    )
    .unwrap();
    origin.join().unwrap();
    assert_eq!(progress.downloaded_bytes, 8);
    let content = fs::read(held.0.join("file.part")).unwrap();
    assert_eq!(content, b"firstend");
    use sha2::{Digest, Sha256};
    assert_eq!(Sha256::digest(&content), Sha256::digest(b"firstend"));
}

#[test]
fn resumes_at_chunk_boundaries_with_known_and_unknown_size_including_a_complete_prefix() {
    let payload: Vec<u8> = (0..98317).map(|index| (index % 251) as u8).collect();
    for (prefix, known) in [
        (0, true),
        (3, false),
        (32767, true),
        (32768, false),
        (32769, true),
        (65537, false),
        (98317, true),
    ] {
        let data_bytes = payload.clone();
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.login();
            if known {
                origin.size("213 98317\r\n");
            } else {
                origin.size("550 SIZE unavailable\r\n");
            }
            let mut data = origin.open_data();
            for chunk in data_bytes.chunks(7001) {
                data.write_all(chunk).unwrap();
            }
            drop(data);
            origin.reply("226 Complete\r\n");
            if known {
                origin.size("213 98317\r\n");
            }
            origin.expect_closed();
        });
        let (held, mut file) = HeldFile::new();
        file.write_all(&payload[..prefix]).unwrap();
        let directory = File::open(&held.0).unwrap();
        let context = binding();
        let proof = proof(&mut file, &directory, &context, known.then_some(98317));
        let control = FtpTransferControl::default();
        let progress = resume(
            endpoint,
            FtpCredentials::Anonymous,
            &mut file,
            &directory,
            resume_request(&context, &proof),
            &control,
        )
        .unwrap();
        origin.join().unwrap();
        assert_eq!(progress, nexum_domain::Progress::new(98317, Some(98317)));
        assert_eq!(control.progress().unwrap(), progress);
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), payload);
    }
}

#[test]
fn checkpoint_requires_the_explicit_written_length_and_authenticated_revision_is_bound() {
    let (held, mut file) = HeldFile::new();
    file.write_all(b"first").unwrap();
    let directory = File::open(&held.0).unwrap();
    let mut context = binding();
    let error = checkpoint(
        FtpCheckpointRequest {
            binding: &context,
            attempt_generation: "old-attempt",
            trusted_bytes: 3,
            total_bytes: Some(8),
        },
        &mut file,
        &directory,
        &FtpTransferControl::default(),
    )
    .unwrap_err();
    assert_eq!(error, FtpReceiveError::UncommittedTail);
    context.credential = Some(FtpCredentialBinding {
        credential_id: "keychain-id".into(),
        revision: 1,
    });
    let proof = proof(&mut file, &directory, &context, Some(8));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut changed = context.clone();
    changed.credential.as_mut().unwrap().revision = 2;
    let result = resume(
        listener.local_addr().unwrap(),
        FtpCredentials::Password {
            username: "user",
            password: "secret",
        },
        &mut file,
        &directory,
        resume_request(&changed, &proof),
        &FtpTransferControl::default(),
    );
    assert_eq!(result, Err(FtpReceiveError::ResumeBindingMismatch));
    let result = resume(
        listener.local_addr().unwrap(),
        FtpCredentials::Anonymous,
        &mut file,
        &directory,
        resume_request(&context, &proof),
        &FtpTransferControl::default(),
    );
    assert_eq!(result, Err(FtpReceiveError::ResumeBindingMismatch));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
