//! The native messaging codec. Chrome and Firefox frame every message as a
//! 4-byte length in the host's native byte order followed by that many bytes of
//! JSON.
//!
//! The pure half of this module (the length helpers, the request-id scan and
//! the response builders) is shared by the proxy's blocking std IO and the
//! app's async socket server. The two IO adapters are thin: the proxy calls
//! [`read_frame`]/[`write_frame`], and the server reuses the same pure
//! functions with `AsyncReadExt`/`AsyncWriteExt` so there is exactly one
//! definition of the framing.

use std::io::{self, Read, Write};

/// Largest request frame the extension may send.
pub const MAX_REQUEST_FRAME: usize = 64 * 1024;
/// Largest response frame the app may send back.
pub const MAX_RESPONSE_FRAME: usize = 1024 * 1024;
/// Chrome's own hard limit on one native message; a frame larger than this
/// cannot be drained, so the caller must close the channel.
pub const MAX_STREAM_FRAME: usize = 64 * 1024 * 1024;

/// Bytes read from an over-cap frame to recover the request id before the
/// remainder is drained or the channel is abandoned.
const ID_PREFIX_LEN: usize = 4 * 1024;

/// The length prefix of a frame, in native endian order.
pub fn decode_len(header: [u8; 4]) -> u32 {
    u32::from_ne_bytes(header)
}

/// Encode a frame length in native endian order.
pub fn encode_len(len: u32) -> [u8; 4] {
    len.to_ne_bytes()
}

/// The `id` of a request frame, or `""` when absent or unparseable.
///
/// Tolerant on purpose: an over-cap frame is answered from a bounded prefix,
/// which is not valid JSON, so a full parse is attempted first and a key scan
/// is the fallback for the truncated case.
pub fn request_id(payload: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(payload) {
        return value
            .get("id")
            .and_then(|id| id.as_str())
            .unwrap_or("")
            .to_string();
    }
    scan_id(payload)
}

/// Find the first `"id"` key and read its string value, tolerating a value the
/// prefix cut short. The ids this channel carries are near the front of every
/// frame, so the first occurrence is the request id.
fn scan_id(payload: &[u8]) -> String {
    let mut i = 0;
    while i + 4 <= payload.len() {
        if &payload[i..i + 4] == b"\"id\"" {
            let mut j = i + 4;
            while j < payload.len() && payload[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < payload.len() && payload[j] == b':' {
                j += 1;
                while j < payload.len() && payload[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < payload.len() && payload[j] == b'"' {
                    j += 1;
                    let mut out = Vec::new();
                    while j < payload.len() && payload[j] != b'"' {
                        if payload[j] == b'\\' && j + 1 < payload.len() {
                            j += 1;
                        }
                        out.push(payload[j]);
                        j += 1;
                    }
                    return String::from_utf8_lossy(&out).into_owned();
                }
            }
        }
        i += 1;
    }
    String::new()
}

/// A complete response frame carrying `code`/`message`. Response envelopes go
/// through here so the version and the `ok` discriminant have one author.
pub fn error_frame(id: &str, code: &str, message: &str) -> Vec<u8> {
    let value = serde_json::json!({
        "v": 1u32,
        "id": id,
        "ok": false,
        "error": { "code": code, "message": message },
    });
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&encode_len(body.len() as u32));
    out.extend_from_slice(&body);
    out
}

/// The response to a frame that declared a length over the caller's cap.
pub fn warn_too_large(id: &str) -> Vec<u8> {
    error_frame(id, "too-large", "request too large")
}

/// What one frame read produced.
#[derive(Debug)]
pub enum FrameRead {
    /// A complete frame within the caller's cap.
    Frame(Vec<u8>),
    /// Declared length over the caller's cap. `id` was recovered from the
    /// prefix; `desynced` means the remainder could not be consumed, so the
    /// caller MUST close (it is only false when the frame is small enough to
    /// drain, i.e. `len <= MAX_STREAM_FRAME`).
    OverCap { id: String, desynced: bool },
    /// Clean EOF before any byte of a new frame.
    Eof,
}

/// Blocking frame read for the proxy. `max` is the caller's cap; a larger
/// declared length never allocates it, only the recovery prefix.
pub fn read_frame<R: Read>(r: &mut R, max: usize) -> io::Result<FrameRead> {
    let mut header = [0u8; 4];
    let mut read = 0;
    while read < 4 {
        match r.read(&mut header[read..]) {
            Ok(0) => {
                if read == 0 {
                    return Ok(FrameRead::Eof);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated frame header",
                ));
            }
            Ok(n) => read += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    let len = decode_len(header) as usize;
    if len <= max {
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf)?;
        Ok(FrameRead::Frame(buf))
    } else {
        let prefix_len = len.min(ID_PREFIX_LEN);
        let mut prefix = vec![0u8; prefix_len];
        r.read_exact(&mut prefix)?;
        let id = request_id(&prefix);
        if len <= MAX_STREAM_FRAME {
            let mut remaining = len - prefix_len;
            let mut scratch = [0u8; 8192];
            while remaining > 0 {
                let take = remaining.min(scratch.len());
                r.read_exact(&mut scratch[..take])?;
                remaining -= take;
            }
            Ok(FrameRead::OverCap {
                id,
                desynced: false,
            })
        } else {
            Ok(FrameRead::OverCap { id, desynced: true })
        }
    }
}

/// Blocking framed write for the proxy.
pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    w.write_all(&encode_len(payload.len() as u32))?;
    w.write_all(payload)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_round_trips_native_endian() {
        let bytes = encode_len(0x1234_5678);
        assert_eq!(bytes, 0x1234_5678u32.to_ne_bytes());
        assert_eq!(decode_len(bytes), 0x1234_5678);
    }

    #[test]
    fn request_id_reads_a_complete_frame() {
        assert_eq!(
            request_id(br#"{"v":1,"id":"abc","action":"status"}"#),
            "abc"
        );
        assert_eq!(request_id(br#"{"v":1,"action":"status"}"#), "");
        assert_eq!(request_id(b"not json"), "");
    }

    #[test]
    fn request_id_survives_a_truncated_prefix() {
        // A prefix cut mid-frame, before the id's closing quote.
        assert_eq!(request_id(br#"{"v":1,"id":"abc"#), "abc");
        assert_eq!(request_id(br#"{"v":1, "id" : "x-y" ,"a"#), "x-y");
    }

    #[test]
    fn read_frame_returns_a_frame_and_then_eof() {
        let mut wire = Vec::new();
        write_frame(&mut wire, b"{\"id\":\"1\"}").unwrap();
        let mut cursor = std::io::Cursor::new(wire);
        match read_frame(&mut cursor, MAX_REQUEST_FRAME).unwrap() {
            FrameRead::Frame(b) => assert_eq!(b, b"{\"id\":\"1\"}"),
            other => panic!("expected frame, got {other:?}"),
        }
        assert!(matches!(
            read_frame(&mut cursor, MAX_REQUEST_FRAME).unwrap(),
            FrameRead::Eof
        ));
    }

    #[test]
    fn read_frame_reports_over_cap_and_recovers_the_id() {
        let body = serde_json::to_vec(&serde_json::json!({
            "v": 1, "id": "big", "action": "x", "params": { "pad": "y".repeat(70 * 1024) }
        }))
        .unwrap();
        let mut wire = encode_len(body.len() as u32).to_vec();
        wire.extend_from_slice(&body);
        // Drainable: 70 KiB is over the 64 KiB cap but under the stream limit.
        let mut cursor = std::io::Cursor::new(wire.clone());
        match read_frame(&mut cursor, MAX_REQUEST_FRAME).unwrap() {
            FrameRead::OverCap { id, desynced } => {
                assert_eq!(id, "big");
                assert!(!desynced);
            }
            other => panic!("expected over-cap, got {other:?}"),
        }
        // The stream is still in sync: the next read is clean EOF.
        assert!(matches!(
            read_frame(&mut cursor, MAX_REQUEST_FRAME).unwrap(),
            FrameRead::Eof
        ));
    }

    #[test]
    fn read_frame_marks_a_desynced_frame_past_the_stream_limit() {
        // A frame past the stream limit cannot be drained, so only the 4 KiB
        // recovery prefix is read and the rest is left in the channel.
        let mut wire = encode_len((MAX_STREAM_FRAME + 1) as u32).to_vec();
        let mut prefix = br#"{"v":1,"id":"gone","action":"x"}"#.to_vec();
        prefix.resize(ID_PREFIX_LEN, b' ');
        wire.extend_from_slice(&prefix);
        let mut cursor = std::io::Cursor::new(wire);
        match read_frame(&mut cursor, MAX_REQUEST_FRAME).unwrap() {
            FrameRead::OverCap { id, desynced } => {
                assert_eq!(id, "gone");
                assert!(desynced);
            }
            other => panic!("expected over-cap, got {other:?}"),
        }
    }

    #[test]
    fn error_frame_carries_the_envelope() {
        let bytes = error_frame("id-1", "app-not-running", "Subclave is not running");
        let len = decode_len(bytes[..4].try_into().unwrap()) as usize;
        let value: serde_json::Value = serde_json::from_slice(&bytes[4..4 + len]).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["id"], "id-1");
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "app-not-running");
        assert_eq!(value["error"]["message"], "Subclave is not running");
    }
}
