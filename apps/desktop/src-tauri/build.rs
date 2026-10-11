use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

fn main() {
    let resources = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("resources");
    // Workspace checks do not build the Server. An empty resource directory lets
    // those debug checks run; an actual release must carry a prepared executable.
    fs::create_dir_all(&resources).expect("create Desktop resource staging directory");
    let executable = resources.join("nexum-server");
    let metadata_path = resources.join("nexum-server-build.json");
    println!("cargo:rerun-if-changed={}", resources.display());
    println!("cargo:rerun-if-changed={}", executable.display());
    println!("cargo:rerun-if-changed={}", metadata_path.display());

    if executable.exists() || metadata_path.exists() {
        let metadata: serde_json::Value = serde_json::from_slice(
            &fs::read(&metadata_path).expect("read prepared Server metadata"),
        )
        .expect("parse prepared Server metadata");
        assert_eq!(
            metadata["target"].as_str(),
            Some(env::var("TARGET").unwrap().as_str()),
            "bundled Server target differs from Desktop; run prepare-desktop-server.mjs"
        );
        assert_eq!(
            metadata["server_version"].as_str(),
            Some(env::var("CARGO_PKG_VERSION").unwrap().as_str()),
            "bundled Server version differs from Desktop"
        );
        let digest = format!(
            "{:x}",
            Sha256::digest(fs::read(&executable).expect("read prepared Server executable"))
        );
        assert_eq!(
            metadata["sha256"].as_str(),
            Some(digest.as_str()),
            "prepared Server digest changed; rebuild it before packaging"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                fs::metadata(&executable).unwrap().permissions().mode() & 0o111,
                0,
                "prepared Server is not executable"
            );
        }
        println!("cargo:rustc-env=NEXUM_SERVER_SHA256={digest}");
        println!(
            "cargo:rustc-env=NEXUM_SERVER_DEV_PATH={}",
            executable.canonicalize().unwrap().display()
        );
    } else {
        assert_ne!(
            env::var("PROFILE").unwrap(),
            "release",
            "release Desktop requires a bundled Server; use pnpm exec tauri build"
        );
        println!("cargo:rustc-env=NEXUM_SERVER_SHA256=");
        println!("cargo:rustc-env=NEXUM_SERVER_DEV_PATH=");
    }
    tauri_build::build()
}
