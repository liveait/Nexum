//! Bounded v1 Magnet identity parsing. Ancillary parameters grant no network or
//! file-selection authority and are not retained in the parsed value.

use std::fmt;

pub const MAX_MAGNET_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MagnetInfo {
    info_hash: [u8; 20],
}

impl MagnetInfo {
    pub fn info_hash(&self) -> &[u8; 20] {
        &self.info_hash
    }

    pub fn info_hash_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(40);
        for byte in self.info_hash {
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 15) as usize] as char);
        }
        encoded
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MagnetError {
    TooLong,
    InvalidUri,
    InvalidParameter,
    InvalidEncoding,
    MissingInfoHash,
    InvalidInfoHash,
    ConflictingInfoHashes,
    UnsupportedV2,
    UnsupportedIndexedTopic,
}

impl fmt::Display for MagnetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLong => "Magnet URI exceeds the byte limit",
            Self::InvalidUri => "invalid Magnet URI",
            Self::InvalidParameter => "invalid Magnet parameter",
            Self::InvalidEncoding => "invalid Magnet parameter encoding",
            Self::MissingInfoHash => "Magnet URI is missing a v1 info hash",
            Self::InvalidInfoHash => "invalid Magnet info hash",
            Self::ConflictingInfoHashes => "conflicting Magnet info hashes",
            Self::UnsupportedV2 => "v2 Magnet topics are not supported",
            Self::UnsupportedIndexedTopic => "indexed Magnet topics are not supported",
        })
    }
}

impl std::error::Error for MagnetError {}

pub(crate) fn checked_source(source: &str) -> Result<&str, MagnetError> {
    if source.len() > MAX_MAGNET_BYTES {
        return Err(MagnetError::TooLong);
    }
    if source.chars().any(char::is_control) {
        return Err(MagnetError::InvalidUri);
    }
    Ok(source.trim())
}

/// Extract one v1 identity without resolving trackers, peers, metadata or paths.
/// The original input is bounded before outer trimming. Decode parameter keys
/// and values once after splitting delimiters; a plus remains a literal plus.
pub fn parse_magnet(source: &str) -> Result<MagnetInfo, MagnetError> {
    let source = checked_source(source)?;
    let (scheme, rest) = source.split_once(':').ok_or(MagnetError::InvalidUri)?;
    if !scheme.eq_ignore_ascii_case("magnet") {
        return Err(MagnetError::InvalidUri);
    }
    let query = rest.strip_prefix('?').ok_or(MagnetError::InvalidUri)?;
    if !query.chars().all(query_character) {
        return Err(MagnetError::InvalidUri);
    }
    if query.is_empty() {
        return Err(MagnetError::MissingInfoHash);
    }
    let mut hash = None;
    let mut v2 = false;
    for parameter in query.split('&') {
        let (key, value) = parameter
            .split_once('=')
            .ok_or(MagnetError::InvalidParameter)?;
        let key = decode(key)?;
        let value = decode(value)?;
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(MagnetError::InvalidParameter);
        }
        if key.starts_with("xt.") {
            return Err(MagnetError::UnsupportedIndexedTopic);
        }
        if key != "xt" {
            continue;
        }
        let (urn, topic) = value.split_once(':').ok_or(MagnetError::InvalidInfoHash)?;
        let (namespace, encoded) = topic.split_once(':').ok_or(MagnetError::InvalidInfoHash)?;
        if !urn.eq_ignore_ascii_case("urn") || encoded.is_empty() {
            return Err(MagnetError::InvalidInfoHash);
        }
        if namespace.eq_ignore_ascii_case("btmh") {
            v2 = true;
            continue;
        }
        if !namespace.eq_ignore_ascii_case("btih") {
            return Err(MagnetError::InvalidInfoHash);
        }
        let current = decode_hash(encoded)?;
        if hash.is_some_and(|previous| previous != current) {
            return Err(MagnetError::ConflictingInfoHashes);
        }
        hash = Some(current);
    }
    if v2 {
        return Err(MagnetError::UnsupportedV2);
    }
    Ok(MagnetInfo {
        info_hash: hash.ok_or(MagnetError::MissingInfoHash)?,
    })
}

fn query_character(character: char) -> bool {
    if character.is_ascii() {
        character.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:@/?%".contains(character)
    } else {
        !character.is_control() && !character.is_whitespace()
    }
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode(value: &str) -> Result<String, MagnetError> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes
                .next()
                .and_then(hex)
                .ok_or(MagnetError::InvalidEncoding)?;
            let low = bytes
                .next()
                .and_then(hex)
                .ok_or(MagnetError::InvalidEncoding)?;
            decoded.push(high * 16 + low);
        } else {
            decoded.push(byte);
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| MagnetError::InvalidEncoding)?;
    if decoded.chars().any(char::is_control) {
        return Err(MagnetError::InvalidEncoding);
    }
    Ok(decoded)
}

fn decode_hash(encoded: &str) -> Result<[u8; 20], MagnetError> {
    let mut hash = [0; 20];
    match encoded.len() {
        40 => {
            for (out, pair) in hash.iter_mut().zip(encoded.as_bytes().as_chunks::<2>().0) {
                *out = hex(pair[0]).ok_or(MagnetError::InvalidInfoHash)? * 16
                    + hex(pair[1]).ok_or(MagnetError::InvalidInfoHash)?;
            }
        }
        32 => {
            let mut accumulator = 0u16;
            let mut bits = 0;
            let mut output = 0;
            for byte in encoded.bytes() {
                let value = match byte.to_ascii_uppercase() {
                    b'A'..=b'Z' => byte.to_ascii_uppercase() - b'A',
                    b'2'..=b'7' => byte - b'2' + 26,
                    _ => return Err(MagnetError::InvalidInfoHash),
                };
                accumulator = (accumulator << 5) | u16::from(value);
                bits += 5;
                if bits >= 8 {
                    bits -= 8;
                    hash[output] = (accumulator >> bits) as u8;
                    output += 1;
                    accumulator &= (1 << bits) - 1;
                }
            }
        }
        _ => return Err(MagnetError::InvalidInfoHash),
    }
    Ok(hash)
}
