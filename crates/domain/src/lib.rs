//! Foundational domain types shared by Nexum components.

use std::fmt;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for TaskId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}
impl From<&str> for TaskId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}
impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadSource(String);
impl DownloadSource {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Destination(String);
impl Destination {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Progress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_second: u64,
    pub eta_seconds: Option<u64>,
}

impl Progress {
    pub fn new(downloaded_bytes: u64, total_bytes: Option<u64>) -> Self {
        Self {
            downloaded_bytes,
            total_bytes,
            ..Self::default()
        }
    }

    /// A progress snapshot carrying an observed transfer rate. `eta_seconds`
    /// should be `Some` only when the total length is trustworthy (`total_bytes`
    /// `Some`) and `speed_bytes_per_second` is non-zero; callers keep it `None`
    /// for unknown-length (e.g. chunked) responses.
    pub fn with_rate(
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        speed_bytes_per_second: u64,
        eta_seconds: Option<u64>,
    ) -> Self {
        Self {
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_second,
            eta_seconds,
        }
    }
}
