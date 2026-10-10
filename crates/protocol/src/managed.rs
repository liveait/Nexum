//! Private parent/child control frames for an App-owned Server.
//!
//! These messages belong on inherited stdin/stdout pipes, never on the public
//! RPC socket. Each JSON payload has a four-byte big-endian length prefix.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::SocketAddr;

pub const MANAGED_CONTROL_VERSION: u32 = 1;
pub const MANAGED_FRAME_MAX_BYTES: usize = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedStartup {
    pub control_version: u32,
    pub server_version: String,
    pub protocol_version: String,
    /// A fresh 32-byte secret represented as 64 ASCII hexadecimal characters.
    pub bearer_token: String,
}

impl fmt::Debug for ManagedStartup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedStartup")
            .field("control_version", &self.control_version)
            .field("server_version", &self.server_version)
            .field("protocol_version", &self.protocol_version)
            .field("bearer_token", &"<redacted>")
            .finish()
    }
}

impl ManagedStartup {
    pub fn validate(
        &self,
        expected_server_version: &str,
        expected_protocol_version: &str,
    ) -> io::Result<()> {
        let error = if self.control_version != MANAGED_CONTROL_VERSION {
            Some("unsupported managed control version")
        } else if self.server_version != expected_server_version {
            Some("managed server version mismatch")
        } else if self.protocol_version != expected_protocol_version {
            Some("managed RPC protocol version mismatch")
        } else if self.bearer_token.len() != 64
            || !self
                .bearer_token
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            Some("managed Bearer token must be 32 bytes encoded as hexadecimal")
        } else {
            None
        };
        match error {
            Some(message) => Err(io::Error::new(io::ErrorKind::InvalidInput, message)),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedControl {
    Start(ManagedStartup),
    Shutdown { control_version: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedReady {
    pub control_version: u32,
    pub server_version: String,
    pub protocol_version: String,
    pub process_id: u32,
    pub endpoint: SocketAddr,
}

/// Read one bounded frame. `None` means EOF at a frame boundary; partial frames
/// and invalid JSON are errors. Parser errors intentionally omit input values.
pub fn read_managed_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<Option<T>> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header[..1]) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    reader.read_exact(&mut header[1..])?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MANAGED_FRAME_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid managed frame length",
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid managed frame payload"))
}

pub fn write_managed_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "invalid managed frame payload")
    })?;
    if payload.is_empty() || payload.len() > MANAGED_FRAME_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid managed frame length",
        ));
    }
    writer.write_all(&(payload.len() as u32).to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn startup() -> ManagedStartup {
        ManagedStartup {
            control_version: MANAGED_CONTROL_VERSION,
            server_version: "0.1.0".to_owned(),
            protocol_version: "1".to_owned(),
            bearer_token: "0123456789abcdef".repeat(4),
        }
    }

    #[test]
    fn compatible_startup_requires_a_complete_hexadecimal_secret() {
        let mut value = startup();
        assert!(value.validate("0.1.0", "1").is_ok());
        for token in ["short".to_owned(), "x".repeat(64), "é".repeat(32)] {
            value.bearer_token = token;
            assert!(value.validate("0.1.0", "1").is_err());
        }
    }

    #[test]
    fn startup_rejects_each_incompatible_version() {
        let mut value = startup();
        assert!(value.validate("0.2.0", "1").is_err());
        assert!(value.validate("0.1.0", "2").is_err());
        value.control_version += 1;
        assert!(value.validate("0.1.0", "1").is_err());
    }

    #[test]
    fn startup_debug_does_not_expose_the_token() {
        let value = startup();
        let secret = value.bearer_token.clone();
        let debug = format!("{:?}", ManagedControl::Start(value));
        assert!(!debug.contains(&secret));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn framing_preserves_message_boundaries_and_clean_eof() {
        let mut wire = Vec::new();
        write_managed_frame(&mut wire, &ManagedControl::Start(startup())).unwrap();
        write_managed_frame(&mut wire, &ManagedControl::Shutdown { control_version: 1 }).unwrap();
        let mut reader = Cursor::new(wire);
        assert!(matches!(
            read_managed_frame::<ManagedControl>(&mut reader).unwrap(),
            Some(ManagedControl::Start(_))
        ));
        assert!(matches!(
            read_managed_frame::<ManagedControl>(&mut reader).unwrap(),
            Some(ManagedControl::Shutdown { control_version: 1 })
        ));
        assert!(
            read_managed_frame::<ManagedControl>(&mut reader)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn frame_length_is_checked_before_reading_the_payload() {
        for length in [0, MANAGED_FRAME_MAX_BYTES as u32 + 1, u32::MAX] {
            let mut reader = Cursor::new(length.to_be_bytes());
            let error = read_managed_frame::<ManagedControl>(&mut reader).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn partial_headers_and_payloads_are_not_clean_eof() {
        for wire in [vec![0], vec![0, 0, 0], vec![0, 0, 0, 2, b'{']] {
            let error = read_managed_frame::<ManagedControl>(&mut Cursor::new(wire)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        }
    }

    #[test]
    fn parser_errors_do_not_echo_sensitive_or_unknown_fields() {
        for payload in [
            br#"{"type":"unknown-sensitive-value"}"#.as_slice(),
            br#"{"type":"shutdown","control_version":1,"bearer_token":"sensitive-value"}"#,
            br#"{"type":"shutdown","control_version":"sensitive-value"}"#,
        ] {
            let mut wire = (payload.len() as u32).to_be_bytes().to_vec();
            wire.extend(payload);
            let error = read_managed_frame::<ManagedControl>(&mut Cursor::new(wire)).unwrap_err();
            assert_eq!(error.to_string(), "invalid managed frame payload");
        }
    }

    #[test]
    fn writer_rejects_oversized_frames_without_writing_any_bytes() {
        let mut wire = Vec::new();
        let value = "x".repeat(MANAGED_FRAME_MAX_BYTES);
        assert!(write_managed_frame(&mut wire, &value).is_err());
        assert!(wire.is_empty());
    }
}
