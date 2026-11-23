//! Public Engine consumer using an actual completed FTP transfer.
#![cfg(target_os = "macos")]

use nexum_engine::ftp::*;
use std::fs::{self, DirBuilder, File};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, symlink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

const DATA: &[u8] = b"complete FTP payload";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    file: File,
    parent: File,
    binding: FtpResumeBinding,
    proof: FtpCompletionProof,
    receive_control: FtpTransferControl,
}

impl Fixture {
    fn new() -> Self {
        Self::with_names("partial", "final")
    }

    fn with_names(source_leaf: &str, final_leaf: &str) -> Self {
        Self::with_prefix("nexum-publication", source_leaf, final_leaf)
    }

    fn with_prefix(prefix: &str, source_leaf: &str, final_leaf: &str) -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new().mode(0o700).create(&root).unwrap();
        let parent = File::open(&root).unwrap();
        let mut file = File::options()
            .create_new(true)
            .read(true)
            .write(true)
            .open(root.join(source_leaf))
            .unwrap();
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = server.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (socket, _) = server.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(8)))
                .unwrap();
            let mut ftp = BufReader::new(socket);
            let data = TcpListener::bind("127.0.0.1:0").unwrap();
            let pasv = format!("229 (|||{}|)", data.local_addr().unwrap().port());
            let size = format!("213 {}", DATA.len());
            writeln!(ftp.get_mut(), "220 ready\r").unwrap();
            for (command, reply) in [
                ("USER anonymous", "230 ok"),
                ("TYPE I", "200 ok"),
                ("SIZE file", &size),
                ("EPSV", &pasv),
                ("RETR file", "150 data"),
                ("SIZE file", &size),
            ] {
                let mut line = String::new();
                ftp.read_line(&mut line).unwrap();
                assert_eq!(line, format!("{command}\r\n"));
                writeln!(ftp.get_mut(), "{reply}\r").unwrap();
                if command == "RETR file" {
                    data.accept().unwrap().0.write_all(DATA).unwrap();
                    writeln!(ftp.get_mut(), "226 complete\r").unwrap();
                }
            }
        });
        let binding = FtpResumeBinding {
            server_instance_id: "instance".into(),
            task_id: "task".into(),
            operation_id: "operation".into(),
            source_origin: format!("ftp://{endpoint}"),
            command_path: "file".into(),
            path_mode: FtpPathMode::LoginRelative,
            destination: root.join(final_leaf).to_str().unwrap().into(),
            credential: None,
        };
        let receive_control = FtpTransferControl::default();
        let proof = receive_complete(
            endpoint,
            FtpCredentials::Anonymous,
            &mut file,
            &parent,
            FtpCompletionRequest {
                binding: &binding,
                attempt_generation: "producer",
            },
            &receive_control,
        )
        .unwrap();
        worker.join().unwrap();
        Self {
            root,
            file,
            parent,
            binding,
            proof,
            receive_control,
        }
    }
    fn request(&self) -> FtpPublicationRequest<'_> {
        FtpPublicationRequest {
            completion: FtpCompletionRequest {
                binding: &self.binding,
                attempt_generation: "producer",
            },
            proof: &self.proof,
            owner_generation: "current-owner",
            authorization: "current-authorization",
        }
    }
    fn plan(&self) -> FtpPublicationPlan {
        FtpPublicationPlan::prepare(&self.file, &self.parent, "partial", self.request()).unwrap()
    }
    fn publish(
        &self,
        plan: &FtpPublicationPlan,
        control: &FtpTransferControl,
    ) -> Result<FtpPublishedCompletion, FtpPublicationError> {
        publish_completion_with_guard(&self.parent, plan, self.request(), control, || Ok(()))
    }
    fn reconcile(
        &self,
        plan: &FtpPublicationPlan,
        control: &FtpTransferControl,
    ) -> Result<FtpPublishedCompletion, FtpPublicationError> {
        reconcile_publication_with_guard(&self.parent, plan, self.request(), control, || Ok(()))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn same_inode_no_clobber_hardlink_retains_staging() {
    let f = Fixture::new();
    let plan = FtpPublicationPlan::decode(&f.plan().encode().unwrap()).unwrap();
    let control = FtpTransferControl::default();
    let evidence = f.publish(&plan, &control).unwrap();
    assert_eq!(evidence.progress(), f.proof.progress());
    assert_eq!(evidence.plan().producer_generation(), "producer");
    assert_eq!(evidence.plan().owner_generation(), "current-owner");
    assert_eq!(evidence.plan().authorization(), "current-authorization");
    assert!(control.is_committed() && control.is_finished());
    let final_file = f.root.join("final");
    assert_eq!(
        fs::metadata(&final_file).unwrap().ino(),
        f.file.metadata().unwrap().ino()
    );
    assert_eq!(
        fs::metadata(f.root.join("partial")).unwrap().ino(),
        fs::metadata(&final_file).unwrap().ino()
    );
    assert_eq!(fs::read(&final_file).unwrap(), DATA);
    assert!(f.publish(&plan, &FtpTransferControl::default()).is_err());
    let recovery = FtpTransferControl::default();
    assert_eq!(
        f.reconcile(&plan, &recovery).unwrap().progress(),
        f.proof.progress()
    );
    assert!(recovery.is_committed() && recovery.is_finished());
    for debug in [
        format!("{plan:?}"),
        format!("{evidence:?}"),
        format!("{:?}", f.proof),
    ] {
        assert!(
            !debug.contains(f.root.to_str().unwrap())
                && !debug.contains("current-authorization")
                && !debug.contains("sha256")
        );
    }
}

#[test]
fn existing_regular_symlink_hardlink_and_directory_are_preserved() {
    for kind in ["regular", "symlink", "hardlink", "directory"] {
        let f = Fixture::new();
        let plan = f.plan();
        let final_file = f.root.join("final");
        let sentinel = f.root.join("sentinel");
        fs::write(&sentinel, "keep").unwrap();
        match kind {
            "regular" => fs::write(&final_file, DATA).unwrap(),
            "symlink" => symlink("dangling", &final_file).unwrap(),
            "hardlink" => fs::hard_link(&sentinel, &final_file).unwrap(),
            _ => fs::create_dir(&final_file).unwrap(),
        }
        let before = fs::symlink_metadata(&final_file).unwrap();
        let control = FtpTransferControl::default();
        assert!(f.publish(&plan, &control).is_err(), "{kind}");
        assert!(!control.is_committed());
        assert!(
            f.reconcile(&plan, &FtpTransferControl::default()).is_err(),
            "{kind}"
        );
        let after = fs::symlink_metadata(&final_file).unwrap();
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before.file_type(), after.file_type());
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(fs::read(f.root.join("partial")).unwrap(), DATA);
    }
}

#[test]
fn partial_or_parent_identity_mismatch_is_rejected() {
    let f = Fixture::new();
    let plan = f.plan();
    fs::create_dir(f.root.join("other")).unwrap();
    let wrong_parent = File::open(f.root.join("other")).unwrap();
    assert!(
        publish_completion_with_guard(
            &wrong_parent,
            &plan,
            f.request(),
            &FtpTransferControl::default(),
            || Ok(())
        )
        .is_err()
    );
    let partial = f.root.join("partial");
    fs::rename(&partial, f.root.join("original")).unwrap();
    symlink("original", &partial).unwrap();
    assert!(f.publish(&plan, &FtpTransferControl::default()).is_err());
    assert_eq!(
        fs::read_link(&partial).unwrap(),
        std::path::Path::new("original")
    );
    assert!(!f.root.join("final").exists());
}

#[test]
fn equal_bytes_replacement_and_corruption_are_retained() {
    let f = Fixture::new();
    let plan = f.plan();
    let source = f.root.join("partial");
    fs::rename(&source, f.root.join("original")).unwrap();
    fs::write(&source, DATA).unwrap();
    assert!(f.publish(&plan, &FtpTransferControl::default()).is_err());
    assert_eq!(fs::read(&source).unwrap(), DATA);
    assert!(!f.root.join("final").exists());
    fs::rename(f.root.join("original"), &source).unwrap();
    f.publish(&plan, &FtpTransferControl::default()).unwrap();
    fs::write(&source, vec![0; DATA.len()]).unwrap();
    assert!(f.reconcile(&plan, &FtpTransferControl::default()).is_err());
    assert_eq!(fs::read(f.root.join("final")).unwrap(), vec![0; DATA.len()]);
}

#[test]
fn unknown_or_malformed_publication_plan_is_rejected() {
    let f = Fixture::new();
    let bytes = f.plan().encode().unwrap();
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (field, value) in [
        ("format", "ftp-publication/v2"),
        ("extra", "x"),
        ("source_leaf", "../partial"),
        ("final_leaf", "other"),
        ("authorization", ""),
    ] {
        let mut modified = original.clone();
        modified[field] = value.into();
        assert!(
            FtpPublicationPlan::decode(&serde_json::to_vec(&modified).unwrap()).is_err(),
            "{field}"
        );
    }
    for field in ["source_leaf", "authorization", "completion"] {
        let mut modified = original.clone();
        modified.as_object_mut().unwrap().remove(field);
        assert!(FtpPublicationPlan::decode(&serde_json::to_vec(&modified).unwrap()).is_err());
    }
    let mut missing = original.clone();
    missing["completion"]["binding"]
        .as_object_mut()
        .unwrap()
        .remove("credential");
    assert!(FtpPublicationPlan::decode(&serde_json::to_vec(&missing).unwrap()).is_err());
    assert!(FtpPublicationPlan::decode(&vec![b' '; 65537]).is_err());
    assert!(FtpPublicationPlan::decode(&f.proof.encode().unwrap()).is_err());
}

#[test]
fn source_and_final_leaf_names_are_strictly_bound() {
    let f = Fixture::new();
    for invalid in [
        "",
        ".",
        "..",
        "./partial",
        "partial/",
        "part/ial",
        "/partial",
        "part\\ial",
        "part\0ial",
        "final",
    ] {
        assert!(
            FtpPublicationPlan::prepare(&f.file, &f.parent, invalid, f.request()).is_err(),
            "{invalid:?}"
        );
    }
    let plan = f.plan();
    for field in ["binding", "producer", "owner", "authorization", "proof"] {
        let other = Fixture::new();
        let mut request = f.request();
        match field {
            "binding" => request.completion.binding = &other.binding,
            "producer" => request.completion.attempt_generation = "other",
            "owner" => request.owner_generation = "other",
            "authorization" => request.authorization = "other",
            _ => request.proof = &other.proof,
        }
        assert!(
            publish_completion_with_guard(
                &f.parent,
                &plan,
                request,
                &FtpTransferControl::default(),
                || Ok(())
            )
            .is_err(),
            "{field}"
        );
        assert!(!f.root.join("final").exists());
    }
}

#[test]
fn finished_controls_are_not_reused() {
    let f = Fixture::new();
    let plan = f.plan();
    assert!(f.publish(&plan, &f.receive_control).is_err());
    assert!(!f.root.join("final").exists());
    let control = FtpTransferControl::default();
    f.publish(&plan, &control).unwrap();
    assert!(f.reconcile(&plan, &control).is_err());
}

#[test]
fn reconcile_missing_final_publishes_with_supplied_control() {
    let f = Fixture::new();
    let plan = f.plan();
    let control = FtpTransferControl::default();
    f.reconcile(&plan, &control).unwrap();
    assert!(control.is_committed() && control.is_finished());
    assert_eq!(
        fs::metadata(f.root.join("final")).unwrap().ino(),
        f.file.metadata().unwrap().ino()
    );
}

#[test]
fn existing_final_survives_missing_or_conflicting_staging() {
    let f = Fixture::new();
    let plan = f.plan();
    f.publish(&plan, &FtpTransferControl::default()).unwrap();
    let partial = f.root.join("partial");
    fs::remove_file(&partial).unwrap();

    let control = FtpTransferControl::default();
    f.reconcile(&plan, &control).unwrap();
    assert!(control.is_committed() && control.is_finished());
    assert!(!partial.exists());
    let only_final = File::open(f.root.join("final")).unwrap();
    let mut next = f.request();
    next.owner_generation = "next-owner";
    next.authorization = "new-authorization";
    let rearmed = plan.rearm(&only_final, &f.parent, next).unwrap();
    assert_eq!(rearmed.producer_generation(), "producer");
    assert_eq!(rearmed.owner_generation(), "next-owner");
    assert_eq!(rearmed.authorization(), "new-authorization");
    let mut request = f.request();
    request.owner_generation = "next-owner";
    request.authorization = "new-authorization";
    assert!(
        reconcile_publication_with_guard(
            &f.parent,
            &rearmed,
            request,
            &FtpTransferControl::default(),
            || Ok(())
        )
        .is_ok()
    );
    assert!(
        f.reconcile(&rearmed, &FtpTransferControl::default())
            .is_err()
    );

    fs::write(&partial, DATA).unwrap();
    let replacement = fs::metadata(&partial).unwrap().ino();
    let control = FtpTransferControl::default();
    assert!(f.reconcile(&plan, &control).is_err());
    assert!(control.is_committed() && control.is_finished());
    assert_eq!(fs::metadata(&partial).unwrap().ino(), replacement);
    assert_eq!(
        fs::metadata(f.root.join("final")).unwrap().ino(),
        f.file.metadata().unwrap().ino()
    );
}

#[test]
fn equivalent_leaf_spellings_cannot_adopt_one_entry_as_two() {
    for (source, target) in [("partial", "PARTIAL"), ("cafe\u{301}", "caf\u{e9}")] {
        let f = Fixture::with_names(source, target);
        let plan = FtpPublicationPlan::prepare(&f.file, &f.parent, source, f.request()).unwrap();
        let control = FtpTransferControl::default();
        if f.root.join(target).exists() {
            // This volume resolves both spellings to the one staged entry.
            assert!(f.publish(&plan, &FtpTransferControl::default()).is_err());
            assert!(f.reconcile(&plan, &control).is_err());
            assert!(!control.is_committed());
            assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
        } else {
            f.reconcile(&plan, &control).unwrap();
            assert!(control.is_committed());
            assert_eq!(fs::read_dir(&f.root).unwrap().count(), 2);
        }
        assert_eq!(fs::read(f.root.join(source)).unwrap(), DATA);
    }
}

#[test]
fn replaced_parent_and_parent_symlink_are_rejected() {
    let f = Fixture::new();
    let plan = f.plan();
    let moved = f.root.with_extension("moved");
    fs::rename(&f.root, &moved).unwrap();
    fs::create_dir(&f.root).unwrap();
    let replacement = f.publish(&plan, &FtpTransferControl::default());
    fs::remove_dir(&f.root).unwrap();
    symlink(&moved, &f.root).unwrap();
    let alias = f.publish(&plan, &FtpTransferControl::default());
    fs::remove_file(&f.root).unwrap();
    fs::rename(&moved, &f.root).unwrap();
    assert!(replacement.is_err() && alias.is_err());
    assert_eq!(fs::read(f.root.join("partial")).unwrap(), DATA);
    assert!(!f.root.join("final").exists());
}

#[test]
fn multibyte_leaves_and_ancestors_publish_and_reconcile() {
    let long = "文".repeat(86);
    for (parent, source, target) in [
        ("nexum-publication", "partial", "文".repeat(255)),
        ("nexum-publication", long.as_str(), "final".into()),
        (long.as_str(), "partial", "final".into()),
    ] {
        let f = Fixture::with_prefix(parent, source, &target);
        let plan = FtpPublicationPlan::prepare(&f.file, &f.parent, source, f.request()).unwrap();
        f.publish(&plan, &FtpTransferControl::default()).unwrap();
        f.reconcile(&plan, &FtpTransferControl::default()).unwrap();
        assert_eq!(fs::read(f.root.join(&target)).unwrap(), DATA);
        assert_eq!(
            fs::metadata(f.root.join(source)).unwrap().ino(),
            fs::metadata(f.root.join(target)).unwrap().ino()
        );
    }
}
