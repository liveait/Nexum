//! Bounded, offline v1 metainfo inspection. No files or network connections.

use caseless::Caseless;
use serde::Serialize;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::{
    collections::BTreeSet,
    fmt,
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use unicode_normalization::UnicodeNormalization;

pub const MAX_METAINFO_BYTES: usize = 512 * 1024;
pub const MAX_FILES: usize = 4096;
pub const MAX_PIECES: usize = 25_000;
pub const MAX_DEPTH: usize = 64;
pub const MAX_NODES: usize = 65_536;
pub const MAX_PATH_DEPTH: usize = 32;
pub const MAX_COMPONENT_BYTES: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TorrentFile {
    pub index: usize,
    pub path: Vec<String>,
    pub length: u64,
    pub padding: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MetainfoSummary {
    pub info_hash: String,
    pub metainfo_sha256: String,
    pub name: String,
    pub private: bool,
    pub piece_length: u64,
    pub piece_count: usize,
    pub total_bytes: u64,
    pub files: Vec<TorrentFile>,
}

/// Never contains raw metainfo, tracker URLs or parser excerpts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetainfoError {
    TooLarge,
    InvalidEncoding,
    ResourceLimit,
    InvalidMetainfo,
    UnsupportedVersion,
    UnsafePath,
    Cancelled,
    DeadlineExceeded,
}

impl MetainfoError {
    pub fn code(self) -> &'static str {
        match self {
            Self::TooLarge => "metainfo_too_large",
            Self::InvalidEncoding => "invalid_bencode",
            Self::ResourceLimit => "metainfo_resource_limit",
            Self::InvalidMetainfo => "invalid_metainfo",
            Self::UnsupportedVersion => "unsupported_torrent_version",
            Self::UnsafePath => "unsafe_torrent_path",
            Self::Cancelled => "cancelled",
            Self::DeadlineExceeded => "inspection_deadline_exceeded",
        }
    }
}
impl fmt::Display for MetainfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for MetainfoError {}

struct Budget<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
}
impl Budget<'_> {
    fn check(&self) -> Result<(), MetainfoError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(MetainfoError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(MetainfoError::DeadlineExceeded);
        }
        Ok(())
    }
}

enum Value<'a> {
    Bytes(&'a [u8]),
    Int(i64),
    List(Vec<Node<'a>>),
    Dict(Vec<(&'a [u8], Node<'a>)>),
}
struct Node<'a> {
    value: Value<'a>,
    span: Range<usize>,
}
impl<'a> Node<'a> {
    fn field(&self, key: &[u8]) -> Option<&Node<'a>> {
        if let Value::Dict(entries) = &self.value {
            entries
                .binary_search_by(|(name, _)| name.cmp(&key))
                .ok()
                .map(|index| &entries[index].1)
        } else {
            None
        }
    }
    fn bytes(&self) -> Result<&'a [u8], MetainfoError> {
        match self.value {
            Value::Bytes(bytes) => Ok(bytes),
            _ => Err(MetainfoError::InvalidMetainfo),
        }
    }
    fn number(&self) -> Result<u64, MetainfoError> {
        match self.value {
            Value::Int(number) if number >= 0 => Ok(number as u64),
            _ => Err(MetainfoError::InvalidMetainfo),
        }
    }
    fn list(&self) -> Result<&[Node<'a>], MetainfoError> {
        match &self.value {
            Value::List(entries) => Ok(entries),
            _ => Err(MetainfoError::InvalidMetainfo),
        }
    }
    fn required(&self, key: &[u8]) -> Result<&Node<'a>, MetainfoError> {
        self.field(key).ok_or(MetainfoError::InvalidMetainfo)
    }
}

struct Parser<'a, 'b> {
    bytes: &'a [u8],
    offset: usize,
    nodes: usize,
    budget: &'b Budget<'b>,
}
impl<'a> Parser<'a, '_> {
    fn bump(&mut self) -> Result<(), MetainfoError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(MetainfoError::ResourceLimit);
        }
        // Power-of-two polling avoids division on every parsed node.
        if self.nodes & 1023 == 0 {
            self.budget.check()?;
        }
        Ok(())
    }
    fn string(&mut self) -> Result<&'a [u8], MetainfoError> {
        self.bump()?;
        let start = self.offset;
        while self.bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
            self.offset += 1;
            if self.offset - start > 20 {
                return Err(MetainfoError::InvalidEncoding);
            }
        }
        let digits = &self.bytes[start..self.offset];
        if digits.is_empty()
            || (digits.len() > 1 && digits[0] == b'0')
            || self.bytes.get(self.offset) != Some(&b':')
        {
            return Err(MetainfoError::InvalidEncoding);
        }
        let length = std::str::from_utf8(digits)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or(MetainfoError::InvalidEncoding)?;
        self.offset += 1;
        let end = self
            .offset
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(MetainfoError::InvalidEncoding)?;
        self.budget.check()?;
        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
    fn node(&mut self, depth: usize) -> Result<Node<'a>, MetainfoError> {
        if depth > MAX_DEPTH {
            return Err(MetainfoError::ResourceLimit);
        }
        let start = self.offset;
        let value = match self.bytes.get(self.offset) {
            Some(b'0'..=b'9') => Value::Bytes(self.string()?),
            Some(b'i') => {
                self.bump()?;
                self.offset += 1;
                let number_start = self.offset;
                if self.bytes.get(self.offset) == Some(&b'-') {
                    self.offset += 1;
                }
                let digit_start = self.offset;
                while self.bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                    self.offset += 1;
                    if self.offset - digit_start > 19 {
                        return Err(MetainfoError::InvalidEncoding);
                    }
                }
                let digits = &self.bytes[digit_start..self.offset];
                if digits.is_empty()
                    || (digits.len() > 1 && digits[0] == b'0')
                    || (number_start != digit_start && digits == b"0")
                    || self.bytes.get(self.offset) != Some(&b'e')
                {
                    return Err(MetainfoError::InvalidEncoding);
                }
                let number = std::str::from_utf8(&self.bytes[number_start..self.offset])
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .ok_or(MetainfoError::InvalidEncoding)?;
                self.offset += 1;
                Value::Int(number)
            }
            Some(b'l') => {
                self.bump()?;
                self.offset += 1;
                let mut entries = Vec::new();
                while self.bytes.get(self.offset) != Some(&b'e') {
                    entries.push(self.node(depth + 1)?);
                }
                self.offset += 1;
                Value::List(entries)
            }
            Some(b'd') => {
                self.bump()?;
                self.offset += 1;
                let mut entries: Vec<(&[u8], Node<'a>)> = Vec::new();
                while self.bytes.get(self.offset) != Some(&b'e') {
                    let key = self.string()?;
                    if entries.last().is_some_and(|(previous, _)| *previous >= key) {
                        return Err(MetainfoError::InvalidEncoding);
                    }
                    entries.push((key, self.node(depth + 1)?));
                }
                self.offset += 1;
                Value::Dict(entries)
            }
            _ => return Err(MetainfoError::InvalidEncoding),
        };
        Ok(Node {
            value,
            span: start..self.offset,
        })
    }
}

fn component(bytes: &[u8]) -> Result<String, MetainfoError> {
    if bytes.is_empty() || bytes.len() > MAX_COMPONENT_BYTES {
        return Err(MetainfoError::UnsafePath);
    }
    let value = std::str::from_utf8(bytes).map_err(|_| MetainfoError::UnsafePath)?;
    if matches!(value, "." | "..")
        || value.ends_with(['.', ' '])
        || value.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        })
    {
        return Err(MetainfoError::UnsafePath);
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
    {
        return Err(MetainfoError::UnsafePath);
    }
    Ok(value.to_owned())
}

fn padding(node: &Node<'_>) -> Result<bool, MetainfoError> {
    if node.field(b"symlink path").is_some() {
        return Err(MetainfoError::UnsafePath);
    }
    let Some(attr) = node.field(b"attr") else {
        return Ok(false);
    };
    let attr = attr.bytes()?;
    if attr.contains(&b'l') {
        return Err(MetainfoError::UnsafePath);
    }
    Ok(attr.contains(&b'p'))
}

pub fn inspect_metainfo(
    bytes: &[u8],
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<MetainfoSummary, MetainfoError> {
    if bytes.len() > MAX_METAINFO_BYTES {
        return Err(MetainfoError::TooLarge);
    }
    let budget = Budget { cancel, deadline };
    budget.check()?;
    let mut parser = Parser {
        bytes,
        offset: 0,
        nodes: 0,
        budget: &budget,
    };
    let root = parser.node(1)?;
    if parser.offset != bytes.len() {
        return Err(MetainfoError::InvalidEncoding);
    }
    let info = root.required(b"info")?;
    if info.field(b"meta version").is_some() || info.field(b"file tree").is_some() {
        return Err(MetainfoError::UnsupportedVersion);
    }
    let name = component(info.required(b"name")?.bytes()?)?;
    let private = match info.field(b"private").map(Node::number).transpose()? {
        None | Some(0) => false,
        Some(1) => true,
        _ => return Err(MetainfoError::InvalidMetainfo),
    };
    let piece_length = info.required(b"piece length")?.number()?;
    if piece_length == 0 {
        return Err(MetainfoError::InvalidMetainfo);
    }
    let pieces = info.required(b"pieces")?.bytes()?;
    if pieces.len() % 20 != 0 {
        return Err(MetainfoError::InvalidMetainfo);
    }
    let piece_count = pieces.len() / 20;
    if piece_count > MAX_PIECES {
        return Err(MetainfoError::ResourceLimit);
    }
    let mut files = Vec::new();
    match (info.field(b"length"), info.field(b"files")) {
        (Some(length), None) => files.push(TorrentFile {
            index: 0,
            path: vec![name.clone()],
            length: length.number()?,
            padding: padding(info)?,
        }),
        (None, Some(entries)) => {
            let entries = entries.list()?;
            if entries.is_empty() {
                return Err(MetainfoError::InvalidMetainfo);
            }
            if entries.len() > MAX_FILES {
                return Err(MetainfoError::ResourceLimit);
            }
            for (index, entry) in entries.iter().enumerate() {
                budget.check()?;
                let path = entry.required(b"path")?.list()?;
                if path.is_empty() || path.len() > MAX_PATH_DEPTH {
                    return Err(MetainfoError::UnsafePath);
                }
                files.push(TorrentFile {
                    index,
                    path: path
                        .iter()
                        .map(|node| component(node.bytes()?))
                        .collect::<Result<_, _>>()?,
                    length: entry.required(b"length")?.number()?,
                    padding: padding(entry)?,
                });
            }
        }
        _ => return Err(MetainfoError::InvalidMetainfo),
    }
    let mut total_bytes = 0_u64;
    let mut aliases = BTreeSet::<Vec<String>>::new();
    for file in &files {
        budget.check()?;
        total_bytes = total_bytes
            .checked_add(file.length)
            .filter(|size| *size <= i64::MAX as u64)
            .ok_or(MetainfoError::InvalidMetainfo)?;
        let mut alias = Vec::with_capacity(file.path.len());
        for part in &file.path {
            budget.check()?;
            alias.push(
                part.chars()
                    .nfd()
                    .default_case_fold()
                    .nfd()
                    .collect::<String>(),
            );
        }
        if aliases
            .range(..=alias.clone())
            .next_back()
            .is_some_and(|previous| alias.starts_with(previous))
            || aliases
                .range(alias.clone()..)
                .next()
                .is_some_and(|next| next.starts_with(&alias))
        {
            return Err(MetainfoError::UnsafePath);
        }
        aliases.insert(alias);
    }
    if total_bytes.div_ceil(piece_length) != piece_count as u64 {
        return Err(MetainfoError::InvalidMetainfo);
    }
    let mut info_hash = Sha1::new();
    for chunk in bytes[info.span.clone()].chunks(16 * 1024) {
        budget.check()?;
        info_hash.update(chunk);
    }
    let mut metainfo_hash = Sha256::new();
    for chunk in bytes.chunks(16 * 1024) {
        budget.check()?;
        metainfo_hash.update(chunk);
    }
    budget.check()?;
    Ok(MetainfoSummary {
        info_hash: format!("{:x}", info_hash.finalize()),
        metainfo_sha256: format!("{:x}", metainfo_hash.finalize()),
        name,
        private,
        piece_length,
        piece_count,
        total_bytes,
        files,
    })
}
