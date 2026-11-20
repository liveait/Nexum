//! Metadata-only BEP 9 reception from one explicitly authorized IP endpoint.
//! No discovery, payload transfer, listeners, files or background tasks.

use super::{
    Budget, MAX_METAINFO_BYTES, MetainfoError, MetainfoSummary, Node, Parser, Value,
    inspect_metainfo,
};
use crate::magnet::MagnetInfo;
use sha1::{Digest, Sha1};
use std::{
    fmt,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

pub const METADATA_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_METADATA_BYTES: usize = MAX_METAINFO_BYTES - 8;
pub const MAX_FRAME_BYTES: usize = 20 * 1024;
pub const MAX_HEADER_BYTES: usize = 1024;
pub const MAX_FRAMES: usize = 256;
const BLOCK: usize = 16 * 1024;
const CONTROL_WINDOW: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataError {
    Cancelled,
    DeadlineExceeded,
    InvalidPeer,
    Connect,
    Io,
    InvalidHandshake,
    UnsupportedMetadata,
    TooLarge,
    FrameLimit,
    InvalidMessage,
    Rejected,
    IdentityMismatch,
    Metainfo(MetainfoError),
}
impl MetadataError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::DeadlineExceeded => "inspection_deadline_exceeded",
            Self::InvalidPeer => "invalid_metadata_peer",
            Self::Connect => "metadata_connect_failed",
            Self::Io => "metadata_io_failed",
            Self::InvalidHandshake => "invalid_metadata_handshake",
            Self::UnsupportedMetadata => "metadata_extension_unavailable",
            Self::TooLarge => "metadata_size_limit",
            Self::FrameLimit => "metadata_frame_limit",
            Self::InvalidMessage => "invalid_metadata_message",
            Self::Rejected => "metadata_request_rejected",
            Self::IdentityMismatch => "metadata_identity_mismatch",
            Self::Metainfo(error) => error.code(),
        }
    }
}
impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for MetadataError {}
impl From<MetainfoError> for MetadataError {
    fn from(error: MetainfoError) -> Self {
        match error {
            MetainfoError::Cancelled => Self::Cancelled,
            MetainfoError::DeadlineExceeded => Self::DeadlineExceeded,
            _ => Self::Metainfo(error),
        }
    }
}

pub struct FetchedMetadata {
    metainfo: Vec<u8>,
    summary: MetainfoSummary,
}
impl FetchedMetadata {
    pub fn summary(&self) -> &MetainfoSummary {
        &self.summary
    }
    /// The bytes are private, synthesized metainfo, not an original .torrent.
    pub fn into_parts(self) -> (Vec<u8>, MetainfoSummary) {
        (self.metainfo, self.summary)
    }
}
impl fmt::Debug for FetchedMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchedMetadata")
            .field("info_hash", &self.summary.info_hash)
            .field("metainfo", &"<private>")
            .finish()
    }
}

pub fn valid_peer(peer: SocketAddr) -> bool {
    let ip = peer.ip().to_canonical();
    peer.port() != 0
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !matches!(ip, std::net::IpAddr::V4(ip) if ip.is_broadcast())
}

fn window(budget: &Budget<'_>) -> Result<Duration, MetadataError> {
    budget.check()?;
    Ok(budget
        .deadline
        .saturating_duration_since(Instant::now())
        .min(CONTROL_WINDOW)
        .max(Duration::from_nanos(1)))
}
fn retry(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}
fn read(s: &mut TcpStream, mut bytes: &mut [u8], budget: &Budget<'_>) -> Result<(), MetadataError> {
    while !bytes.is_empty() {
        s.set_read_timeout(Some(window(budget)?))
            .map_err(|_| MetadataError::Io)?;
        match s.read(bytes) {
            Ok(0) => return Err(MetadataError::Io),
            Ok(n) => bytes = &mut bytes[n..],
            Err(error) if retry(&error) => {}
            Err(_) => return Err(MetadataError::Io),
        }
    }
    budget.check()?;
    Ok(())
}
fn write(s: &mut TcpStream, mut bytes: &[u8], budget: &Budget<'_>) -> Result<(), MetadataError> {
    while !bytes.is_empty() {
        s.set_write_timeout(Some(window(budget)?))
            .map_err(|_| MetadataError::Io)?;
        match s.write(bytes) {
            Ok(0) => return Err(MetadataError::Io),
            Ok(n) => bytes = &bytes[n..],
            Err(error) if retry(&error) => {}
            Err(_) => return Err(MetadataError::Io),
        }
    }
    budget.check()?;
    Ok(())
}
fn send(s: &mut TcpStream, bytes: &[u8], budget: &Budget<'_>) -> Result<(), MetadataError> {
    write(s, &(bytes.len() as u32).to_be_bytes(), budget)?;
    write(s, bytes, budget)
}
fn receive(
    s: &mut TcpStream,
    remaining: &mut usize,
    budget: &Budget<'_>,
) -> Result<Vec<u8>, MetadataError> {
    if *remaining == 0 {
        return Err(MetadataError::FrameLimit);
    }
    *remaining -= 1;
    let mut length = [0; 4];
    read(s, &mut length, budget)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(MetadataError::FrameLimit);
    }
    let mut bytes = vec![0; length];
    read(s, &mut bytes, budget)?;
    Ok(bytes)
}
fn header<'a>(bytes: &'a [u8], budget: &Budget<'_>) -> Result<(Node<'a>, usize), MetadataError> {
    // Parse only a bounded prefix. Raw metadata bytes after the dictionary are
    // not bencode and must not expand the parser's input/resource allowance.
    let mut parser = Parser {
        bytes: &bytes[..bytes.len().min(MAX_HEADER_BYTES)],
        offset: 0,
        nodes: 0,
        budget,
    };
    let node = parser.node(1).map_err(|error| match error {
        MetainfoError::Cancelled | MetainfoError::DeadlineExceeded => MetadataError::from(error),
        _ => MetadataError::InvalidMessage,
    })?;
    if !matches!(&node.value, Value::Dict(_)) {
        return Err(MetadataError::InvalidMessage);
    }
    Ok((node, parser.offset))
}
fn number(node: &Node<'_>, key: &[u8]) -> Result<u64, MetadataError> {
    node.required(key)
        .and_then(Node::number)
        .map_err(|_| MetadataError::InvalidMessage)
}

/// Fetch from exactly `peer`. The caller supplies a fresh, independent random
/// peer ID (never an inspection ID or credential). This synchronous operation
/// owns its only socket until return; an owner may cancel and join its thread.
pub fn fetch_magnet_metadata(
    identity: MagnetInfo,
    peer: SocketAddr,
    peer_id: [u8; 20],
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<FetchedMetadata, MetadataError> {
    let budget = Budget {
        cancel,
        deadline: deadline.min(Instant::now() + METADATA_TIMEOUT),
    };
    budget.check()?;
    if !valid_peer(peer) {
        return Err(MetadataError::InvalidPeer);
    }
    let mut stream = loop {
        match TcpStream::connect_timeout(&peer, window(&budget)?) {
            Ok(stream) => break stream,
            Err(error) if retry(&error) => {}
            Err(_) => return Err(MetadataError::Connect),
        }
    };
    let mut handshake = [0_u8; 68];
    handshake[..20].copy_from_slice(b"\x13BitTorrent protocol");
    handshake[25] = 0x10;
    handshake[28..48].copy_from_slice(identity.info_hash());
    handshake[48..].copy_from_slice(&peer_id);
    write(&mut stream, &handshake, &budget)?;
    read(&mut stream, &mut handshake, &budget)?;
    if &handshake[..20] != b"\x13BitTorrent protocol"
        || handshake[25] & 0x10 == 0
        || &handshake[28..48] != identity.info_hash()
    {
        return Err(MetadataError::InvalidHandshake);
    }
    send(&mut stream, b"\x14\x00d1:md11:ut_metadatai1eee", &budget)?;
    let mut remaining = MAX_FRAMES;
    let (remote_id, size) = loop {
        let bytes = receive(&mut stream, &mut remaining, &budget)?;
        if !bytes.starts_with(&[20, 0]) {
            continue;
        }
        let (node, used) = header(&bytes[2..], &budget)?;
        if used != bytes.len() - 2 {
            return Err(MetadataError::InvalidMessage);
        }
        let remote = node
            .required(b"m")
            .and_then(|m| m.required(b"ut_metadata"))
            .and_then(Node::number)
            .ok()
            .filter(|id| (1..=255).contains(id))
            .ok_or(MetadataError::UnsupportedMetadata)? as u8;
        let size = number(&node, b"metadata_size")?;
        if size == 0 || size > MAX_METADATA_BYTES as u64 {
            return Err(MetadataError::TooLarge);
        }
        break (remote, size as usize);
    };
    // No allocation or block request precedes the advertised-size check.
    let mut info = vec![0_u8; size];
    for (piece, block) in info.chunks_mut(BLOCK).enumerate() {
        let request = format!("d8:msg_typei0e5:piecei{piece}ee");
        send(
            &mut stream,
            &[&[20, remote_id], request.as_bytes()].concat(),
            &budget,
        )?;
        loop {
            let bytes = receive(&mut stream, &mut remaining, &budget)?;
            // ID 1 is OUR advertised incoming ID, not the peer's request ID.
            if !bytes.starts_with(&[20, 1]) {
                continue;
            }
            let (node, used) = header(&bytes[2..], &budget)?;
            let kind = match &node
                .required(b"msg_type")
                .map_err(|_| MetadataError::InvalidMessage)?
                .value
            {
                Value::Int(kind) => *kind,
                _ => return Err(MetadataError::InvalidMessage),
            };
            match kind {
                0 => {
                    let requested = number(&node, b"piece")?;
                    let reject = format!("d8:msg_typei2e5:piecei{requested}ee");
                    send(
                        &mut stream,
                        &[&[20, remote_id], reject.as_bytes()].concat(),
                        &budget,
                    )?;
                }
                1 | 2 => {
                    if number(&node, b"piece")? != piece as u64 {
                        return Err(MetadataError::InvalidMessage);
                    }
                    if kind == 2 {
                        return Err(MetadataError::Rejected);
                    }
                    if number(&node, b"total_size")? != size as u64
                        || bytes.len() - 2 - used != block.len()
                    {
                        return Err(MetadataError::InvalidMessage);
                    }
                    block.copy_from_slice(&bytes[2 + used..]);
                    break;
                }
                _ => {} // BEP 9 requires ignoring unknown message types.
            }
        }
    }
    // Close the peer before CPU validation or returning any completed result.
    drop(stream);
    let mut digest = Sha1::new();
    for chunk in info.chunks(BLOCK) {
        budget.check()?;
        digest.update(chunk);
    }
    if digest.finalize().as_slice() != identity.info_hash() {
        return Err(MetadataError::IdentityMismatch);
    }
    budget.check()?;
    let mut metainfo = Vec::with_capacity(size + 8);
    metainfo.extend_from_slice(b"d4:info");
    metainfo.extend_from_slice(&info);
    metainfo.push(b'e');
    let parse_deadline = budget.deadline.min(Instant::now() + Duration::from_secs(5));
    let summary = inspect_metainfo(&metainfo, cancel, parse_deadline)?;
    budget.check()?;
    Ok(FetchedMetadata { metainfo, summary })
}
