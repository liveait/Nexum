use super::resume::{binding, proof, resume_request};
use super::*;
use nexum_engine::ftp::repair_uncommitted_tail;
use std::io::{Seek, SeekFrom};

#[test]
fn repairs_only_uncommitted_bytes_and_keeps_the_original_proof() {
    let (held, mut file) = HeldFile::new();
    file.write_all(b"first").unwrap();
    let directory = File::open(&held.0).unwrap();
    let binding = binding();
    let proof = proof(&mut file, &directory, &binding, Some(8));
    let encoded = proof.encode().unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(b"unsaved tail").unwrap();
    fs::hard_link(held.0.join("file.part"), held.0.join("alias")).unwrap();
    for _ in 0..2 {
        let control = FtpTransferControl::default();
        let progress = repair_uncommitted_tail(
            &mut file,
            &directory,
            resume_request(&binding, &proof),
            &control,
        )
        .unwrap();
        assert_eq!(progress, nexum_domain::Progress::new(5, Some(8)));
        assert_eq!(control.progress(), Some(progress));
        assert!(control.is_finished());
        assert_eq!(file.stream_position().unwrap(), 0);
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), b"first");
        assert_eq!(fs::read(held.0.join("alias")).unwrap(), b"first");
        assert_eq!(proof.encode().unwrap(), encoded);
    }
}

#[test]
fn repair_rejects_untrusted_objects_and_bindings_without_mutation() {
    for case in [
        "corrupt",
        "short",
        "replacement",
        "parent",
        "generation",
        "binding",
    ] {
        let (held, mut file) = HeldFile::new();
        file.write_all(b"first").unwrap();
        let mut directory = File::open(&held.0).unwrap();
        let mut context = binding();
        let proof = proof(&mut file, &directory, &context, Some(8));
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"tail").unwrap();
        let (other, _) = HeldFile::new();
        let expected = match case {
            "corrupt" => {
                file.rewind().unwrap();
                file.write_all(b"other").unwrap();
                FtpReceiveError::LocalPrefixMismatch
            }
            "short" => {
                file.set_len(3).unwrap();
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
                file.write_all(b"firsttail").unwrap();
                FtpReceiveError::FileIdentityMismatch
            }
            "parent" => {
                directory = File::open(&other.0).unwrap();
                FtpReceiveError::FileIdentityMismatch
            }
            "generation" => FtpReceiveError::ResumeBindingMismatch,
            "binding" => {
                context.destination.push('x');
                FtpReceiveError::ResumeBindingMismatch
            }
            _ => unreachable!(),
        };
        let before = fs::read(held.0.join("file.part")).unwrap();
        let mut request = resume_request(&context, &proof);
        if case == "generation" {
            request.attempt_generation = "old-attempt";
        }
        let control = FtpTransferControl::default();
        assert_eq!(
            repair_uncommitted_tail(&mut file, &directory, request, &control),
            Err(expected)
        );
        assert!(control.is_finished());
        assert!(control.progress().is_none());
        assert_eq!(fs::read(held.0.join("file.part")).unwrap(), before);
    }
}

#[test]
fn repair_accepts_an_empty_prefix_and_rejects_pre_cancelled_or_reused_controls() {
    let (held, mut file) = HeldFile::new();
    let directory = File::open(&held.0).unwrap();
    let context = binding();
    let proof = proof(&mut file, &directory, &context, None);
    file.write_all(b"uncommitted").unwrap();
    let cancelled = FtpTransferControl::default();
    cancelled.cancel();
    assert_eq!(
        repair_uncommitted_tail(
            &mut file,
            &directory,
            resume_request(&context, &proof),
            &cancelled
        ),
        Err(FtpReceiveError::Cancelled)
    );
    assert_eq!(file.metadata().unwrap().len(), 11);
    let control = FtpTransferControl::default();
    assert_eq!(
        repair_uncommitted_tail(
            &mut file,
            &directory,
            resume_request(&context, &proof),
            &control
        )
        .unwrap(),
        nexum_domain::Progress::new(0, None)
    );
    assert_eq!(file.metadata().unwrap().len(), 0);
    assert_eq!(
        repair_uncommitted_tail(
            &mut file,
            &directory,
            resume_request(&context, &proof),
            &control
        ),
        Err(FtpReceiveError::ControlAlreadyUsed)
    );
}
