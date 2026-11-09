//! Bound wire bytes before UTF-8 decoding, JSON parsing or method selection.

use std::io::{self, BufRead};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BoundedLine {
    Eof,
    Data(Vec<u8>),
    TooLarge,
}

/// Retain at most `max_payload + 2` wire bytes. LF and an immediately
/// preceding CR are framing bytes; a CR at EOF remains payload.
pub(super) fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_payload: usize,
) -> io::Result<BoundedLine> {
    let mut line = Vec::new();
    loop {
        let available = match reader.fill_buf() {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if available.is_empty() {
            return Ok(if line.is_empty() {
                BoundedLine::Eof
            } else if line.len() > max_payload {
                BoundedLine::TooLarge
            } else {
                BoundedLine::Data(line)
            });
        }

        // Inspect only the remaining budget plus one byte. At the limit,
        // only a pending CR may wait for one more byte to recognize CRLF.
        let window = max_payload.saturating_sub(line.len()).saturating_add(1);
        let available = &available[..available.len().min(window)];
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);

        if newline.is_some() {
            let payload_len =
                line.len() - 1 - usize::from(line.len() >= 2 && line[line.len() - 2] == b'\r');
            return Ok(if payload_len > max_payload {
                BoundedLine::TooLarge
            } else {
                BoundedLine::Data(line)
            });
        }
        if line.len() > max_payload
            && !(line.len() - max_payload == 1 && line.last() == Some(&b'\r'))
        {
            return Ok(BoundedLine::TooLarge);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    #[test]
    fn byte_limits_hold_across_fragmentation_and_line_endings() {
        for limit in 0..=8 {
            for size in 0..=10 {
                for ending in [b"".as_slice(), b"\n", b"\r\n"] {
                    let mut bytes = vec![b'x'; size];
                    bytes.extend_from_slice(ending);
                    for capacity in 1..=5 {
                        let mut reader = BufReader::with_capacity(capacity, Cursor::new(&bytes));
                        let expected = if bytes.is_empty() {
                            BoundedLine::Eof
                        } else if size > limit {
                            BoundedLine::TooLarge
                        } else {
                            BoundedLine::Data(bytes.clone())
                        };
                        assert_eq!(
                            read_bounded_line(&mut reader, limit).unwrap(),
                            expected,
                            "limit={limit}, size={size}, ending={ending:?}, capacity={capacity}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn trailing_cr_is_only_excluded_when_followed_by_lf() {
        for (bytes, valid) in [
            (b"1234\r\n".as_slice(), true),
            (b"1234\r", false),
            (b"123\r", true),
            (b"1234\rX", false),
            (b"1234\r\r\n", false),
        ] {
            let mut reader = BufReader::with_capacity(1, Cursor::new(bytes));
            let result = read_bounded_line(&mut reader, 4).unwrap();
            assert_eq!(
                result,
                if valid {
                    BoundedLine::Data(bytes.to_vec())
                } else {
                    BoundedLine::TooLarge
                }
            );
        }
    }

    #[test]
    fn complete_frames_do_not_consume_the_next_request() {
        let mut reader = Cursor::new(b"1234\r\nnext\nlast");
        for expected in [b"1234\r\n".as_slice(), b"next\n", b"last"] {
            assert_eq!(
                read_bounded_line(&mut reader, 4).unwrap(),
                BoundedLine::Data(expected.to_vec())
            );
        }
        assert_eq!(read_bounded_line(&mut reader, 4).unwrap(), BoundedLine::Eof);
    }

    #[test]
    fn oversized_input_stops_before_draining_or_decoding() {
        let mut bytes = vec![0xff; 1024 * 1024];
        bytes.push(b'\n');
        let mut reader = Cursor::new(&bytes);
        assert_eq!(
            read_bounded_line(&mut reader, 16).unwrap(),
            BoundedLine::TooLarge
        );
        assert_eq!(reader.position(), 17);

        let mut reader = Cursor::new("éé\n".as_bytes());
        assert_eq!(
            read_bounded_line(&mut reader, 3).unwrap(),
            BoundedLine::TooLarge
        );
        let mut reader = Cursor::new("éé\n".as_bytes());
        assert_eq!(
            read_bounded_line(&mut reader, 4).unwrap(),
            BoundedLine::Data("éé\n".as_bytes().to_vec())
        );
    }

    #[test]
    fn io_failures_propagate_and_interrupted_reads_retry() {
        struct InterruptedOnce {
            inner: Cursor<Vec<u8>>,
            interrupted: bool,
        }
        impl io::Read for InterruptedOnce {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.inner.read(buf)
            }
        }
        impl BufRead for InterruptedOnce {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                if !self.interrupted {
                    self.interrupted = true;
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                } else {
                    self.inner.fill_buf()
                }
            }
            fn consume(&mut self, amount: usize) {
                self.inner.consume(amount);
            }
        }
        let mut reader = InterruptedOnce {
            inner: Cursor::new(b"ok\n".to_vec()),
            interrupted: false,
        };
        assert_eq!(
            read_bounded_line(&mut reader, 2).unwrap(),
            BoundedLine::Data(b"ok\n".to_vec())
        );

        struct Failed;
        impl io::Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::TimedOut.into())
            }
        }
        impl BufRead for Failed {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                Err(io::ErrorKind::TimedOut.into())
            }
            fn consume(&mut self, _: usize) {
                unreachable!();
            }
        }
        assert_eq!(
            read_bounded_line(&mut Failed, 2).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
}
