use super::*;
use nexum_engine::ftp::{
    FtpCompletionProof, FtpCompletionRequest, FtpCredentialBinding, FtpResumeBinding,
    receive_complete, resume_complete, verify_completion,
};

fn request(binding: &FtpResumeBinding) -> FtpCompletionRequest<'_> {
    FtpCompletionRequest {
        binding,
        attempt_generation: "complete-attempt",
    }
}

fn completed() -> (HeldFile, File, FtpCompletionProof) {
    let (endpoint, origin) = spawn_origin(|mut origin| {
        origin.anonymous_login(8);
        let mut data = origin.open_data();
        data.write_all(b"firstend").unwrap();
        drop(data);
        origin.reply("226 Complete\r\n");
        origin.size("213 8\r\n");
        origin.expect_closed();
    });
    let (held, mut file) = HeldFile::new();
    let proof = receive_complete(
        endpoint,
        FtpCredentials::Anonymous,
        &mut file,
        &File::open(&held.0).unwrap(),
        request(&resume::binding()),
        &FtpTransferControl::default(),
    )
    .unwrap();
    origin.join().unwrap();
    (held, file, proof)
}

#[test]
fn completes_a_real_ftp_response_into_a_held_file_proof() {
    let (endpoint, origin) = spawn_origin(|mut origin| {
        origin.anonymous_login(8);
        let mut data = origin.open_data();
        data.write_all(b"firstend").unwrap();
        drop(data);
        origin.reply("226 Complete\r\n");
        origin.size("213 8\r\n");
        origin.expect_closed();
    });
    let (held, mut file) = HeldFile::new();
    let binding = resume::binding();
    let control = FtpTransferControl::default();
    let result = receive_complete(
        endpoint,
        FtpCredentials::Anonymous,
        &mut file,
        &File::open(&held.0).unwrap(),
        FtpCompletionRequest {
            binding: &binding,
            attempt_generation: "complete-attempt",
        },
        &control,
    );
    assert!(result.is_ok(), "expected complete FTP proof: {result:?}");
    origin.join().unwrap();
    assert!(control.is_finished());
    let proof = result.unwrap();
    assert_eq!(proof.progress(), nexum_domain::Progress::new(8, Some(8)));
    assert_eq!(control.progress(), Some(proof.progress()));
    assert_eq!(proof.binding(), &binding);
    assert_eq!(proof.attempt_generation(), "complete-attempt");
    let encoded = proof.encode().unwrap();
    let proof = FtpCompletionProof::decode(&encoded).unwrap();
    assert!(
        TcpStream::connect(endpoint).is_err(),
        "origin must be offline"
    );
    let mut readonly = File::open(held.0.join("file.part")).unwrap();
    let verifier = FtpTransferControl::default();
    assert_eq!(
        verify_completion(
            &mut readonly,
            &File::open(&held.0).unwrap(),
            request(&binding),
            &proof,
            &verifier
        )
        .unwrap(),
        proof.progress()
    );
    assert_eq!(verifier.progress(), Some(proof.progress()));
    assert!(verifier.is_finished());
    assert_eq!(proof.encode().unwrap(), encoded);
    assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"firstend");
}

#[test]
fn complete_producers_cover_empty_unknown_length_and_resumed_boundaries() {
    for (length, prefix, known) in [
        (0, None, true),
        (3, None, false),
        (65537, None, true),
        (65537, Some(0), true),
        (65537, Some(3), false),
        (65537, Some(32768), true),
        (65537, Some(65537), true),
    ] {
        let payload: Vec<_> = (0..length).map(|i| (i % 251) as u8).collect();
        let sent = payload.clone();
        let (endpoint, origin) = spawn_origin(move |mut origin| {
            origin.login();
            origin.size(&if known {
                format!("213 {length}\r\n")
            } else {
                "550 Unavailable\r\n".into()
            });
            let mut data = origin.open_data();
            for chunk in sent.chunks(7001) {
                data.write_all(chunk).unwrap();
            }
            drop(data);
            origin.reply("250 Complete\r\n");
            if known {
                origin.size(&format!("213 {length}\r\n"));
            }
            origin.expect_closed();
        });
        let (held, mut file) = HeldFile::new();
        let directory = File::open(&held.0).unwrap();
        let binding = resume::binding();
        let control = FtpTransferControl::default();
        let proof = if let Some(prefix) = prefix {
            file.write_all(&payload[..prefix]).unwrap();
            let old = resume::proof(
                &mut file,
                &directory,
                &binding,
                known.then_some(length as u64),
            );
            resume_complete(
                endpoint,
                FtpCredentials::Anonymous,
                &mut file,
                &directory,
                resume::resume_request(&binding, &old),
                &control,
            )
            .unwrap()
        } else {
            receive_complete(
                endpoint,
                FtpCredentials::Anonymous,
                &mut file,
                &directory,
                request(&binding),
                &control,
            )
            .unwrap()
        };
        origin.join().unwrap();
        assert_eq!(
            proof.progress(),
            nexum_domain::Progress::new(length as u64, Some(length as u64))
        );
        assert_eq!(control.progress(), Some(proof.progress()));
        assert!(control.is_finished());
        let current = FtpCompletionRequest {
            binding: &binding,
            attempt_generation: proof.attempt_generation(),
        };
        assert_eq!(
            verify_completion(
                &mut file,
                &directory,
                current,
                &proof,
                &FtpTransferControl::default()
            )
            .unwrap(),
            proof.progress()
        );
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), payload);
    }
}

#[test]
fn full_bytes_without_valid_terminal_evidence_never_return_completion_proof() {
    for case in ["missing", "negative", "malformed", "changed_size"] {
        for resumed in [false, true] {
            let (endpoint, origin) = spawn_origin(move |mut origin| {
                origin.anonymous_login(8);
                let mut data = origin.open_data();
                data.write_all(b"firstend").unwrap();
                drop(data);
                match case {
                    "missing" => return,
                    "negative" => origin.reply("550 Completion rejected\r\n"),
                    "malformed" => origin.reply("226X malformed\r\n"),
                    "changed_size" => {
                        origin.reply("226 Complete\r\n");
                        origin.size("213 9\r\n");
                    }
                    _ => unreachable!(),
                }
                origin.expect_closed();
            });
            let (held, mut file) = HeldFile::new();
            let control = FtpTransferControl::default();
            let directory = File::open(&held.0).unwrap();
            let binding = resume::binding();
            let error = if resumed {
                file.write_all(b"firstend").unwrap();
                let prefix = resume::proof(&mut file, &directory, &binding, Some(8));
                resume_complete(
                    endpoint,
                    FtpCredentials::Anonymous,
                    &mut file,
                    &directory,
                    resume::resume_request(&binding, &prefix),
                    &control,
                )
            } else {
                receive_complete(
                    endpoint,
                    FtpCredentials::Anonymous,
                    &mut file,
                    &directory,
                    request(&binding),
                    &control,
                )
            }
            .unwrap_err();
            origin.join().unwrap();
            let expected = match case {
                "negative" => FtpReceiveError::Rejected(FtpStage::Completion, 550),
                "changed_size" => FtpReceiveError::SourceSizeChanged,
                _ => FtpReceiveError::InvalidReply(FtpStage::Completion),
            };
            assert_eq!(error, expected);
            assert!(control.is_finished());
            assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"firstend");
            assert_eq!(control.progress().unwrap().downloaded_bytes, 8);
        }
    }
}
#[test]
fn offline_verification_rejects_context_object_length_and_digest_substitution() {
    for case in [
        "instance",
        "task",
        "operation",
        "origin",
        "path",
        "destination",
        "credential",
        "generation",
        "parent",
        "replacement",
        "short",
        "tail",
        "digest",
    ] {
        let (held, mut file, proof) = completed();
        let mut binding = resume::binding();
        let mut generation = "complete-attempt";
        let mut directory = File::open(&held.0).unwrap();
        let (other, _) = HeldFile::new();
        let expected = match case {
            "instance" => {
                binding.server_instance_id.push('x');
                FtpReceiveError::ResumeBindingMismatch
            }
            "task" => {
                binding.task_id.push('x');
                FtpReceiveError::ResumeBindingMismatch
            }
            "operation" => {
                binding.operation_id.push('x');
                FtpReceiveError::ResumeBindingMismatch
            }
            "origin" => {
                binding.source_origin.push('0');
                FtpReceiveError::ResumeBindingMismatch
            }
            "path" => {
                binding.command_path.push('x');
                FtpReceiveError::ResumeBindingMismatch
            }
            "destination" => {
                binding.destination.push('x');
                FtpReceiveError::ResumeBindingMismatch
            }
            "credential" => {
                binding.credential = Some(FtpCredentialBinding {
                    credential_id: "other".into(),
                    revision: 2,
                });
                FtpReceiveError::ResumeBindingMismatch
            }
            "generation" => {
                generation = "new-attempt";
                FtpReceiveError::ResumeBindingMismatch
            }
            "parent" => {
                directory = File::open(&other.0).unwrap();
                FtpReceiveError::FileIdentityMismatch
            }
            "replacement" => {
                fs::rename(held.0.join("file.part"), held.0.join("original")).unwrap();
                file = File::options()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(held.0.join("file.part"))
                    .unwrap();
                file.write_all(b"firstend").unwrap();
                FtpReceiveError::FileIdentityMismatch
            }
            "short" => {
                file.set_len(3).unwrap();
                FtpReceiveError::LocalPrefixMismatch
            }
            "tail" => {
                file.seek(std::io::SeekFrom::End(0)).unwrap();
                file.write_all(b"tail").unwrap();
                FtpReceiveError::LocalPrefixMismatch
            }
            "digest" => {
                file.rewind().unwrap();
                file.write_all(b"otherend").unwrap();
                FtpReceiveError::LocalPrefixMismatch
            }
            _ => unreachable!(),
        };
        let before = fs::read(held.0.join("file.part")).unwrap();
        let control = FtpTransferControl::default();
        assert_eq!(
            verify_completion(
                &mut file,
                &directory,
                FtpCompletionRequest {
                    binding: &binding,
                    attempt_generation: generation
                },
                &proof,
                &control
            ),
            Err(expected),
            "{case}"
        );
        assert!(control.is_finished());
        assert!(control.progress().is_none());
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), before);
    }
}

#[test]
fn completion_encoding_rejects_prefix_unknown_extra_and_oversized_formats() {
    let (held, mut file, proof) = completed();
    let original = proof.encode().unwrap();
    for case in [
        "format",
        "unknown",
        "generation",
        "hash",
        "length",
        "credential",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match case {
            "format" => value["format"] = "ftp-held-completion/v2".into(),
            "unknown" => value["extra"] = true.into(),
            "generation" => value["attempt_generation"] = "".into(),
            "hash" => value["sha256"] = serde_json::json!([0]),
            "length" => value["length"] = serde_json::json!(-1),
            "credential" => {
                value["binding"]["source_origin"] = "ftp://user:secret@localhost".into()
            }
            _ => unreachable!(),
        }
        assert!(
            matches!(
                FtpCompletionProof::decode(&serde_json::to_vec(&value).unwrap()),
                Err(FtpReceiveError::InvalidProof)
            ),
            "{case}"
        );
    }
    assert!(FtpCompletionProof::decode(&vec![b' '; 65537]).is_err());
    let prefix = resume::proof(
        &mut file,
        &File::open(&held.0).unwrap(),
        &resume::binding(),
        Some(8),
    );
    assert!(FtpCompletionProof::decode(&prefix.encode().unwrap()).is_err());
    assert_eq!(proof.encode().unwrap(), original);
}

#[test]
fn complete_receive_rejects_invalid_or_cancelled_contexts_before_connecting() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    for case in [
        "generation",
        "credential",
        "parent",
        "target",
        "cancel",
        "oversized",
    ] {
        let (held, mut file) = HeldFile::new();
        let mut binding = resume::binding();
        let mut directory = File::open(&held.0).unwrap();
        let mut generation = "complete-attempt";
        let control = FtpTransferControl::default();
        match case {
            "generation" => generation = "",
            "credential" => {
                binding.credential = Some(FtpCredentialBinding {
                    credential_id: "credential".into(),
                    revision: 1,
                })
            }
            "parent" => directory = file.try_clone().unwrap(),
            "target" => file.write_all(b"existing").unwrap(),
            "cancel" => control.cancel(),
            "oversized" => {
                binding.command_path = "a".repeat(32768);
                binding.destination = "b".repeat(32768);
            }
            _ => unreachable!(),
        }
        let before = fs::read(held.0.join("file.part")).unwrap();
        assert!(
            receive_complete(
                listener.local_addr().unwrap(),
                FtpCredentials::Anonymous,
                &mut file,
                &directory,
                FtpCompletionRequest {
                    binding: &binding,
                    attempt_generation: generation
                },
                &control
            )
            .is_err()
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), before);
        assert!(control.is_finished());
    }
}

#[test]
fn complete_receive_binds_authenticated_credentials_without_exposing_the_password() {
    let (endpoint, origin) = spawn_origin(|mut origin| {
        origin.reply("220 Ready\r\n");
        origin.command("USER FTP_USER_SENTINEL");
        origin.reply("331 Continue\r\n");
        origin.command("PASS FTP_PASSWORD_SENTINEL");
        origin.reply("230 Logged in\r\n");
        origin.command("TYPE I");
        origin.reply("200 Binary\r\n");
        origin.size("213 8\r\n");
        let mut data = origin.open_data();
        data.write_all(b"firstend").unwrap();
        drop(data);
        origin.reply("226 Complete\r\n");
        origin.size("213 8\r\n");
        origin.expect_closed();
    });
    let (held, mut file) = HeldFile::new();
    let mut binding = resume::binding();
    binding.credential = Some(FtpCredentialBinding {
        credential_id: "credential-ref".into(),
        revision: 2,
    });
    let proof = receive_complete(
        endpoint,
        FtpCredentials::Password {
            username: "FTP_USER_SENTINEL",
            password: "FTP_PASSWORD_SENTINEL",
        },
        &mut file,
        &File::open(&held.0).unwrap(),
        request(&binding),
        &FtpTransferControl::default(),
    )
    .unwrap();
    origin.join().unwrap();
    let encoded = String::from_utf8(proof.encode().unwrap()).unwrap();
    assert!(!encoded.contains("FTP_USER_SENTINEL") && !encoded.contains("FTP_PASSWORD_SENTINEL"));
    binding.credential.as_mut().unwrap().revision += 1;
    assert_eq!(
        verify_completion(
            &mut file,
            &File::open(&held.0).unwrap(),
            request(&binding),
            &proof,
            &FtpTransferControl::default()
        ),
        Err(FtpReceiveError::ResumeBindingMismatch)
    );
}
