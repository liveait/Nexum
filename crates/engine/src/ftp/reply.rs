use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;

use super::{FtpReceiveError, FtpStage, network_io};

const MAX_LINE: usize = 8 * 1024;
const MAX_REPLY: usize = 32 * 1024;

pub(super) struct Reply {
    pub(super) code: u16,
    text: Vec<u8>,
    multiline: bool,
}

impl Reply {
    pub(super) fn expect(&self, stage: FtpStage, allowed: &[u16]) -> Result<(), FtpReceiveError> {
        if allowed.contains(&self.code) {
            Ok(())
        } else {
            Err(FtpReceiveError::Rejected(stage, self.code))
        }
    }

    pub(super) fn size(&self) -> Result<u64, FtpReceiveError> {
        if self.multiline {
            return Err(FtpReceiveError::InvalidReply(FtpStage::Size));
        }
        decimal(&self.text).ok_or(FtpReceiveError::InvalidReply(FtpStage::Size))
    }

    pub(super) fn epsv_port(&self) -> Result<u16, FtpReceiveError> {
        let tuple = passive_tuple(&self.text, true).ok_or(invalid_passive())?;
        let [delimiter, second, third, port @ .., last] = tuple else {
            return Err(invalid_passive());
        };
        if !(33..=126).contains(delimiter)
            || delimiter != second
            || delimiter != third
            || delimiter != last
        {
            return Err(invalid_passive());
        }
        decimal(port)
            .filter(|port| *port != 0)
            .ok_or(invalid_passive())
    }

    pub(super) fn pasv_port(&self) -> Result<u16, FtpReceiveError> {
        let tuple = passive_tuple(&self.text, false).ok_or(invalid_passive())?;
        let mut fields = tuple.split(|byte| *byte == b',');
        let mut address = [0u8; 6];
        for field in &mut address {
            *field = fields.next().and_then(decimal).ok_or(invalid_passive())?;
        }
        let port = u16::from_be_bytes([address[4], address[5]]);
        if fields.next().is_some() || port == 0 {
            return Err(invalid_passive());
        }
        Ok(port)
    }
}

fn invalid_passive() -> FtpReceiveError {
    FtpReceiveError::InvalidReply(FtpStage::Passive)
}

fn decimal<T: std::str::FromStr>(bytes: &[u8]) -> Option<T> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn passive_tuple(text: &[u8], require_parentheses: bool) -> Option<&[u8]> {
    let text = text.trim_ascii_end();
    let text = text.strip_suffix(b".").unwrap_or(text);
    if let Some(start) = text.iter().position(|byte| *byte == b'(') {
        return text[start + 1..].strip_suffix(b")");
    }
    if require_parentheses {
        return None;
    }
    let start = text.iter().position(u8::is_ascii_digit)?;
    Some(&text[start..])
}

pub(super) async fn read_reply(
    socket: &mut BufReader<TcpStream>,
    stage: FtpStage,
) -> Result<Reply, FtpReceiveError> {
    let invalid = FtpReceiveError::InvalidReply(stage);
    let mut budget = MAX_REPLY;
    let mut line = Vec::new();
    read_line(socket, &mut line, &mut budget, stage).await?;
    let prefix: [u8; 3] = line
        .get(..3)
        .ok_or_else(|| invalid.clone())?
        .try_into()
        .unwrap();
    let code = decimal::<u16>(&prefix)
        .filter(|code| (100..600).contains(code))
        .ok_or_else(|| invalid.clone())?;
    let multiline = match line.get(3) {
        Some(b'-') => true,
        Some(b' ') => false,
        _ => return Err(invalid),
    };
    if multiline {
        loop {
            read_line(socket, &mut line, &mut budget, stage).await?;
            if line.starts_with(&prefix) && line.get(3) == Some(&b' ') {
                break;
            }
        }
    }
    Ok(Reply {
        code,
        text: line.split_off(4),
        multiline,
    })
}

async fn read_line(
    socket: &mut BufReader<TcpStream>,
    line: &mut Vec<u8>,
    budget: &mut usize,
    stage: FtpStage,
) -> Result<(), FtpReceiveError> {
    line.clear();
    let limit = MAX_LINE.min(*budget);
    loop {
        let available = socket.fill_buf().await.map_err(network_io(stage))?;
        if available.is_empty() {
            return Err(FtpReceiveError::InvalidReply(stage));
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let consumed = end.map_or(available.len(), |index| index + 1);
        if line.len() + consumed > limit {
            return Err(FtpReceiveError::InvalidReply(stage));
        }
        line.extend_from_slice(&available[..consumed]);
        socket.consume(consumed);
        if end.is_some() {
            *budget -= line.len();
            let text = line
                .strip_suffix(b"\r\n")
                .ok_or(FtpReceiveError::InvalidReply(stage))?;
            if text.iter().any(|byte| matches!(byte, b'\r' | 0 | 255)) {
                return Err(FtpReceiveError::InvalidReply(stage));
            }
            line.truncate(text.len());
            return Ok(());
        }
    }
}
