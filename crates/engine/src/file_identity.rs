//! Stable identity shared by completion proofs and held segment staging files.

use crate::EngineError;
use serde::{Deserialize, Serialize};
use std::fs::{File, Metadata};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileIdentity {
    device: u64,
    file: u64,
    created: Option<(u64, u32)>,
}

#[cfg(unix)]
pub(super) fn identity(_file: &File, metadata: &Metadata) -> Result<FileIdentity, EngineError> {
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity {
        device: metadata.dev(),
        file: metadata.ino(),
        created: metadata.created().ok().and_then(|value| {
            value
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|value| (value.as_secs(), value.subsec_nanos()))
        }),
    })
}

#[cfg(windows)]
pub(super) fn identity(file: &File, _metadata: &Metadata) -> Result<FileIdentity, EngineError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
    };
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: The file owns a live handle and info points to sufficient storage.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) } == 0 {
        return Err(EngineError::Failed(format!(
            "file identity: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: A successful call initialized the structure.
    let info = unsafe { info.assume_init() };
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(EngineError::Failed(
            "file identity: file is a reparse point".into(),
        ));
    }
    let created = ((info.ftCreationTime.dwHighDateTime as u64) << 32)
        | info.ftCreationTime.dwLowDateTime as u64;
    Ok(FileIdentity {
        device: info.dwVolumeSerialNumber as u64,
        file: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        created: Some((created, 0)),
    })
}

#[cfg(not(any(unix, windows)))]
pub(super) fn identity(_file: &File, _metadata: &Metadata) -> Result<FileIdentity, EngineError> {
    Err(EngineError::Failed(
        "stable file identities are unsupported on this platform".into(),
    ))
}
