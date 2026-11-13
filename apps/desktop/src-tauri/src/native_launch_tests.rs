use super::{Configuration, binary_digest, spawn_owned, tests::FixtureDirectory};
use crate::{about_links, process_launch};
use serde_json::{Value, json};
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

const PARENT_ROOT: &str = "NEXUM_NATIVE_LAUNCH_PARENT_ROOT";
const HELPER_ROOT: &str = "NEXUM_NATIVE_LAUNCH_HELPER_ROOT";
const SERVER_BINARY: &str = "NEXUM_NATIVE_LAUNCH_SERVER_BINARY";
const SERVER_DIGEST: &str = "NEXUM_NATIVE_LAUNCH_SERVER_DIGEST";
const REAL_SERVER: &str = "NEXUM_NATIVE_LAUNCH_REAL_SERVER";
const RESULT_FILE: &str = "NEXUM_NATIVE_LAUNCH_RESULT_FILE";

struct TestProcess(Child);
impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct HelperRelease(PathBuf);
impl Drop for HelperRelease {
    fn drop(&mut self) {
        let _ = fs::write(self.0.join("helper-release"), b"release");
    }
}

fn publish(path: &Path, bytes: &[u8]) {
    let temporary = path.with_extension("pending");
    fs::write(&temporary, bytes).unwrap();
    fs::rename(temporary, path).unwrap();
}

fn wait_json(path: &Path, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "missing native launch marker {path:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn ping_helper(root: &Path, phase: &str) {
    publish(&root.join("helper-ping"), phase.as_bytes());
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if fs::read_to_string(root.join("helper-pong")).ok().as_deref() == Some(phase) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native helper stopped responding"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn process_running(pid: u32) -> bool {
    assert!(pid > 0 && pid <= i32::MAX as u32);
    // Signal zero inspects the known test process without changing its state.
    if unsafe { libc::kill(pid as i32, 0) } != 0 {
        return std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
    }
    // After parent death, the init process owns reaping. A zombie has exited and
    // already released its descriptors and data-directory lock.
    let output = process_launch::spawn(
        Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "stat="])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap()
    .wait_with_output()
    .unwrap();
    if !output.status.success() {
        return false;
    }
    let state = String::from_utf8(output.stdout).unwrap();
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}

fn wait_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while process_running(pid) {
        assert!(
            Instant::now() < deadline,
            "native test process {pid} did not exit"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// This role reports descriptor metadata only, then stays alive across parent death.
#[test]
fn native_helper_fixture() {
    let Some(root) = std::env::var_os(HELPER_ROOT).map(PathBuf::from) else {
        return;
    };
    let mut descriptors = Vec::new();
    for entry in fs::read_dir("/dev/fd").unwrap() {
        let entry = entry.unwrap();
        let fd: i32 = entry.file_name().to_str().unwrap().parse().unwrap();
        if fd < 3 {
            continue;
        }
        if let Ok(metadata) = fs::metadata(entry.path()) {
            let kind = metadata.file_type();
            let kind = if kind.is_fifo() {
                "pipe"
            } else if kind.is_socket() {
                "socket"
            } else if kind.is_file() {
                "file"
            } else {
                "other"
            };
            descriptors.push(json!({"fd":fd,"kind":kind}));
        }
    }
    descriptors.sort_by_key(|descriptor| descriptor["fd"].as_i64().unwrap());
    publish(
        &root.join("helper-ready.json"),
        &serde_json::to_vec(&json!({"pid":std::process::id(),"descriptors":descriptors})).unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if !root.is_dir() || root.join("helper-release").exists() {
            return;
        }
        if let Ok(phase) = fs::read(root.join("helper-ping")) {
            publish(&root.join("helper-pong"), &phase);
        }
        assert!(
            Instant::now() < deadline,
            "native helper fixture exceeded its lifetime"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Self-dispatch keeps the real Desktop consumer in this test binary.
#[test]
fn native_bootstrap_parent_fixture() {
    let Some(root) = std::env::var_os(PARENT_ROOT).map(PathBuf::from) else {
        return;
    };
    let config = Configuration {
        binary: std::env::var_os(SERVER_BINARY).map(PathBuf::from).unwrap(),
        expected_digest: std::env::var(SERVER_DIGEST).unwrap(),
        data_dir: root.clone(),
    };
    let barrier = Arc::new(Barrier::new(2));
    let helper_barrier = Arc::clone(&barrier);
    let helper_root = root.clone();
    let helper = std::thread::spawn(move || {
        helper_barrier.wait();
        about_links::open_project_link_command(about_links::ProjectLink::Website, |url| {
            assert_eq!(url, "https://liveait.github.io/Nexum/");
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "managed_server::native_launch_tests::native_helper_fixture",
                    "--nocapture",
                ])
                .env(HELPER_ROOT, &helper_root);
            command
        })
        .unwrap();
    });
    barrier.wait();
    let (mut owned, route) = spawn_owned(&config, &Arc::new(AtomicBool::new(true))).unwrap();
    assert_eq!(route.call("server.version", None, 1000).unwrap(), "1");
    let helper_record = wait_json(&root.join("helper-ready.json"), Duration::from_secs(3));
    let bootstrap_fds = [
        owned.stdin.as_ref().unwrap().as_raw_fd(),
        owned._stdout.as_ref().unwrap().as_raw_fd(),
    ];
    assert!(bootstrap_fds.iter().all(|fd| *fd >= 3));
    let record = json!({"server_pid":owned.child.id(),"helper":helper_record,"bootstrap_fds":bootstrap_fds,"fd_probe":"all /dev/fd entries"});
    publish(
        &root.join("parent-ready.json"),
        &serde_json::to_vec(&record).unwrap(),
    );
    helper.join().unwrap();
    owned.stop();
}

#[test]
fn abnormal_parent_exit_closes_bootstrap_while_native_helper_stays_alive() {
    let fixture = FixtureDirectory::new("normal");
    let _release_helper = HelperRelease(fixture.0.clone());
    // The binary is a test dependency, allowing the same consumer to be verified
    // against the bundled production Server after building the native candidate.
    let config = if let Some(binary) = std::env::var_os(REAL_SERVER).map(PathBuf::from) {
        Configuration {
            expected_digest: binary_digest(&binary).unwrap(),
            binary,
            data_dir: fixture.0.clone(),
        }
    } else {
        fixture.config()
    };
    let mut parent = TestProcess(
        process_launch::spawn(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "managed_server::native_launch_tests::native_bootstrap_parent_fixture",
                    "--nocapture",
                ])
                .env(PARENT_ROOT, &fixture.0)
                .env(SERVER_BINARY, &config.binary)
                .env(SERVER_DIGEST, &config.expected_digest)
                .stdin(Stdio::null())
                .stdout(fs::File::create(fixture.0.join("parent-stdout.log")).unwrap())
                .stderr(fs::File::create(fixture.0.join("parent-stderr.log")).unwrap()),
        )
        .unwrap(),
    );
    let record = wait_json(&fixture.0.join("parent-ready.json"), Duration::from_secs(9));
    let helper_pid = record["helper"]["pid"].as_u64().unwrap() as u32;
    let server_pid = record["server_pid"].as_u64().unwrap() as u32;
    let descriptors = record["helper"]["descriptors"].as_array().unwrap();
    assert!(
        descriptors
            .iter()
            .all(|descriptor| descriptor["kind"] != "pipe"),
        "native helper inherited an extra pipe: {descriptors:?}"
    );
    ping_helper(&fixture.0, "before-parent-death");
    parent.0.kill().unwrap();
    assert!(!parent.0.wait().unwrap().success());
    wait_exit(server_pid);
    ping_helper(&fixture.0, "after-server-eof");
    assert!(process_running(helper_pid));
    let (mut replacement, route) = spawn_owned(&config, &Arc::new(AtomicBool::new(true))).unwrap();
    assert_ne!(replacement.child.id(), server_pid);
    assert_eq!(route.call("server.version", None, 1000).unwrap(), "1");
    ping_helper(&fixture.0, "after-same-directory-restart");
    let replacement_pid = replacement.child.id();
    replacement.stop();
    publish(&fixture.0.join("helper-release"), b"release");
    wait_exit(helper_pid);
    if let Some(result) = std::env::var_os(RESULT_FILE) {
        let result = PathBuf::from(result);
        let record = json!({"initial":record,"binary":config.binary,"binary_sha256":config.expected_digest,"replacement_pid":replacement_pid,"server_exited_after_parent_kill":true,"same_directory_restart":true,"helper_responded_across_both_phases":true,"helper_exited_after_release":true});
        publish(&result, &serde_json::to_vec_pretty(&record).unwrap());
    }
}
