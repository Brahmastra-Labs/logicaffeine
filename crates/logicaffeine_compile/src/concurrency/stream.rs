//! Streaming framing / deframing — process a byte STREAM of length-delimited messages
//! INCREMENTALLY, as bytes arrive, without ever buffering the whole stream. This is the basis of
//! every streaming protocol (gRPC, Arrow Flight, Kafka): a producer frames each message as
//! `[uvarint length][body]`; a consumer feeds arriving chunks — ANY chunking, from byte-at-a-time
//! to many frames at once to a frame split across chunks — to a [`StreamDeframer`], which hands
//! back each complete frame the instant it is buffered, ZERO-COPY (the body is a borrowed slice, so
//! pair it with [`crate::concurrency::marshal::view_message`] for a fully in-place stream read).
//!
//! Paired with the zero-copy `WireView` receive, this closes Cap'n Proto's streaming claim:
//! incremental reads with no whole-stream buffering and no decode of untouched fields.

/// Frame `body` for the stream: a LEB128 length prefix, then the bytes. Append-only into `out`, so
/// a producer can concatenate many frames into one stream buffer.
pub fn frame_for_stream(body: &[u8], out: &mut Vec<u8>) {
    let mut len = body.len() as u64;
    loop {
        let mut byte = (len & 0x7f) as u8;
        len >>= 7;
        if len != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
    out.extend_from_slice(body);
}

/// An incremental deframer. Feed it arriving bytes with [`push`](Self::push); pull complete frames
/// with [`drain_frames`](Self::drain_frames). Holds only the bytes not yet delivered (a partial
/// frame at most), so memory stays bounded regardless of stream length.
#[derive(Default)]
pub struct StreamDeframer {
    buf: Vec<u8>,
    start: usize,
}

impl StreamDeframer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a newly-arrived chunk (any size, including empty).
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// No un-delivered bytes remain (the stream is fully consumed up to a frame boundary).
    pub fn is_empty(&self) -> bool {
        self.start >= self.buf.len()
    }

    /// Bytes buffered but not yet delivered (a partial frame awaiting more input).
    pub fn pending(&self) -> usize {
        self.buf.len() - self.start
    }

    /// Hand every COMPLETE frame currently buffered, in arrival order, to `f` as a borrowed body
    /// slice (zero-copy). Stops at the first incomplete frame — keeping its partial bytes for the
    /// next [`push`](Self::push) — then compacts the consumed prefix so the buffer never grows
    /// unbounded. Returns how many frames were delivered.
    pub fn drain_frames(&mut self, mut f: impl FnMut(&[u8])) -> usize {
        let mut count = 0;
        loop {
            let Some((len, header)) = read_uvarint(&self.buf[self.start..]) else {
                break; // length prefix not fully arrived
            };
            let body_start = self.start + header;
            let Some(body_end) = body_start.checked_add(len as usize) else {
                break;
            };
            if body_end > self.buf.len() {
                break; // body not fully arrived
            }
            f(&self.buf[body_start..body_end]);
            self.start = body_end;
            count += 1;
        }
        if self.start > 0 {
            self.buf.drain(0..self.start);
            self.start = 0;
        }
        count
    }
}

/// LEB128 uvarint from the front of `buf`: `Some((value, bytes_consumed))`, or `None` if the varint
/// is truncated mid-encoding (wait for more bytes — never a panic, never a misread).
fn read_uvarint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (i, &b) in buf.iter().enumerate() {
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None; // malformed over-long varint — treat as "wait" (fail-closed)
        }
    }
    None
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/concurrency/stream/tests.rs"]
mod tests;
