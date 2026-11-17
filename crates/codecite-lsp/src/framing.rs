//! LSP's base protocol: `Content-Length: <n>\r\n\r\n<json>` frames.
//!
//! Both caps keep a misbehaving server from making codecite allocate without
//! bound. A malformed frame leaves the stream at an unknown offset, so the
//! client treats every framing error as the end of the connection.

use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

use crate::{Error, Result};

/// The largest header block read before giving up, in bytes.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;

/// The default cap on a frame's body, in bytes.
pub const DEFAULT_MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Writes one framed message and flushes.
pub fn write_message<W: Write + ?Sized>(writer: &mut W, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    frame.extend_from_slice(&body);
    writer.write_all(&frame)?;
    writer.flush()
}

/// Reads one framed message. `Ok(None)` is a clean end of stream: EOF before
/// the first byte of a header.
pub fn read_message<R: BufRead + ?Sized>(reader: &mut R, max_body: usize) -> Result<Option<Value>> {
    let mut length: Option<u64> = None;
    let mut header_bytes = 0usize;
    let mut line = Vec::new();
    loop {
        line.clear();
        // One byte past the remaining budget tells "exactly full" from "over".
        let budget = (MAX_HEADER_BYTES - header_bytes) as u64 + 1;
        let read = reader.take(budget).read_until(b'\n', &mut line)?;
        if read == 0 {
            return if header_bytes == 0 {
                Ok(None)
            } else {
                Err(Error::TruncatedFrame)
            };
        }
        header_bytes += read;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(Error::HeaderTooLarge {
                limit: MAX_HEADER_BYTES,
            });
        }
        if line.last() != Some(&b'\n') {
            return Err(Error::TruncatedFrame);
        }
        let text = trim_line_end(&line);
        if text.is_empty() {
            break;
        }
        let text = String::from_utf8_lossy(text);
        let Some((name, value)) = text.split_once(':') else {
            return Err(Error::MalformedHeader {
                line: text.into_owned(),
            });
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            length = Some(
                value
                    .parse::<u64>()
                    .map_err(|_| Error::InvalidContentLength {
                        value: value.to_owned(),
                    })?,
            );
        }
        // Content-Type, and any other header, is accepted and ignored.
    }
    let length = length.ok_or(Error::MissingContentLength)?;
    let fits = usize::try_from(length).ok().filter(|&n| n <= max_body);
    let Some(length) = fits else {
        return Err(Error::BodyTooLarge {
            length,
            limit: max_body,
        });
    };
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => Error::TruncatedFrame,
        _ => Error::Io(e),
    })?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| Error::InvalidMessage {
            reason: e.to_string(),
        })
}

fn trim_line_end(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;
    use std::io::{BufReader, Cursor};

    fn read(bytes: &[u8]) -> Result<Option<Value>> {
        read_message(&mut Cursor::new(bytes), DEFAULT_MAX_BODY_BYTES)
    }

    #[test]
    fn round_trips_a_message() {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        let body = serde_json::to_vec(&message).unwrap();
        assert!(bytes.starts_with(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes()));
        assert_eq!(read(&bytes).unwrap(), Some(message));
    }

    #[test]
    fn accepts_and_ignores_content_type_in_any_case() {
        let bytes = b"content-type: application/vscode-jsonrpc; charset=utf-8\r\n\
                      CONTENT-LENGTH: 2\r\n\r\n{}";
        assert_eq!(read(bytes).unwrap(), Some(json!({})));
    }

    #[test]
    fn eof_before_a_header_is_a_clean_end() {
        assert_eq!(read(b"").unwrap(), None);
    }

    #[test]
    fn eof_inside_a_frame_is_truncated() {
        assert!(matches!(read(b"Content-Le"), Err(Error::TruncatedFrame)));
        assert!(matches!(
            read(b"Content-Length: 2\r\n"),
            Err(Error::TruncatedFrame)
        ));
        assert!(matches!(
            read(b"Content-Length: 5\r\n\r\n{}"),
            Err(Error::TruncatedFrame)
        ));
    }

    #[test]
    fn refuses_a_missing_length() {
        assert!(matches!(
            read(b"Content-Type: x\r\n\r\n{}"),
            Err(Error::MissingContentLength)
        ));
    }

    #[test]
    fn refuses_a_non_numeric_length() {
        for value in ["abc", "-1", "1.5", ""] {
            let bytes = format!("Content-Length: {value}\r\n\r\n{{}}");
            let err = read(bytes.as_bytes()).unwrap_err();
            assert!(
                matches!(&err, Error::InvalidContentLength { value: v } if v == value),
                "{value:?}: {err:?}"
            );
        }
    }

    #[test]
    fn refuses_a_header_line_without_a_colon() {
        assert!(matches!(
            read(b"garbage\r\n\r\n{}"),
            Err(Error::MalformedHeader { .. })
        ));
    }

    #[test]
    fn refuses_a_header_block_over_the_cap() {
        let mut bytes = b"Content-Length: 2\r\nX-Pad: ".to_vec();
        bytes.extend(std::iter::repeat_n(b'a', MAX_HEADER_BYTES));
        bytes.extend_from_slice(b"\r\n\r\n{}");
        assert!(matches!(
            read(&bytes),
            Err(Error::HeaderTooLarge {
                limit: MAX_HEADER_BYTES
            })
        ));
        // A header line with no newline at all stops at the cap too.
        let endless = vec![b'a'; MAX_HEADER_BYTES * 4];
        assert!(matches!(read(&endless), Err(Error::HeaderTooLarge { .. })));
    }

    #[test]
    fn refuses_a_body_over_the_cap_without_allocating_it() {
        let bytes = b"Content-Length: 18446744073709551615\r\n\r\n";
        assert!(matches!(
            read(bytes),
            Err(Error::BodyTooLarge {
                length: u64::MAX,
                ..
            })
        ));
        let bytes = b"Content-Length: 11\r\n\r\n";
        assert!(matches!(
            read_message(&mut Cursor::new(bytes), 10),
            Err(Error::BodyTooLarge {
                length: 11,
                limit: 10
            })
        ));
    }

    #[test]
    fn refuses_a_body_that_is_not_json() {
        assert!(matches!(
            read(b"Content-Length: 3\r\n\r\nnot"),
            Err(Error::InvalidMessage { .. })
        ));
    }

    /// Returns at most `chunk` bytes per `read`, so frames split across reads.
    struct Trickle {
        bytes: Cursor<Vec<u8>>,
        chunk: usize,
    }

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(self.chunk);
            self.bytes.read(&mut buf[..n])
        }
    }

    fn json_value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::from),
            any::<i64>().prop_map(Value::from),
            ".*".prop_map(Value::from),
        ];
        leaf.prop_recursive(3, 32, 6, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..6).prop_map(Value::from),
                prop::collection::btree_map(".*", inner, 0..6)
                    .prop_map(|m| Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn any_sequence_decodes_unchanged(
            messages in prop::collection::vec(json_value(), 0..8),
            chunk in 1usize..16,
            capacity in 1usize..64,
        ) {
            let mut bytes = Vec::new();
            for message in &messages {
                write_message(&mut bytes, message).unwrap();
            }
            let trickle = Trickle { bytes: Cursor::new(bytes), chunk };
            let mut reader = BufReader::with_capacity(capacity, trickle);
            let mut decoded = Vec::new();
            while let Some(message) = read_message(&mut reader, DEFAULT_MAX_BODY_BYTES).unwrap() {
                decoded.push(message);
            }
            prop_assert_eq!(decoded, messages);
        }
    }
}
