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
mod tests {
    use super::*;

    fn frame_all(bodies: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for b in bodies {
            frame_for_stream(b, &mut out);
        }
        out
    }

    fn collect(deframer: &mut StreamDeframer) -> Vec<Vec<u8>> {
        let mut got = Vec::new();
        deframer.drain_frames(|body| got.push(body.to_vec()));
        got
    }

    #[test]
    fn whole_stream_at_once() {
        let stream = frame_all(&[b"hello", b"", b"world!!", &[0u8, 255, 7]]);
        let mut d = StreamDeframer::new();
        d.push(&stream);
        let got = collect(&mut d);
        assert_eq!(got, vec![b"hello".to_vec(), b"".to_vec(), b"world!!".to_vec(), vec![0, 255, 7]]);
        assert!(d.is_empty(), "fully consumed at a frame boundary");
    }

    #[test]
    fn byte_at_a_time_arrival() {
        // The hardest streaming case: one byte per chunk. Frames must still emerge, in order, the
        // instant each is complete — and never before.
        let bodies: Vec<Vec<u8>> = (0..20).map(|i| vec![i as u8; (i % 5) + 1]).collect();
        let refs: Vec<&[u8]> = bodies.iter().map(|b| b.as_slice()).collect();
        let stream = frame_all(&refs);

        let mut d = StreamDeframer::new();
        let mut got = Vec::new();
        for &byte in &stream {
            d.push(&[byte]);
            d.drain_frames(|body| got.push(body.to_vec()));
        }
        assert_eq!(got, bodies, "every frame reassembled exactly, in order");
        assert!(d.is_empty());
    }

    #[test]
    fn frame_split_across_chunks_mid_body_and_mid_length() {
        // A long body whose length prefix is itself multi-byte, split at adversarial points.
        let big = vec![0xABu8; 5000]; // length prefix = 2 bytes (varint)
        let stream = frame_all(&[b"a", big.as_slice(), b"z"]);

        let mut d = StreamDeframer::new();
        // Split 1: only the first length byte of the big frame (after "a"'s frame + big's 1st len byte).
        let cut1 = 2 + 1; // "a" frame is [len=1][a] = 2 bytes; +1 byte into big's length varint
        d.push(&stream[..cut1]);
        assert_eq!(collect(&mut d), vec![b"a".to_vec()], "only the complete first frame");
        // Split 2: partway into the big body.
        let cut2 = cut1 + 2500;
        d.push(&stream[cut1..cut2]);
        assert_eq!(collect(&mut d), Vec::<Vec<u8>>::new(), "big body incomplete → nothing yet");
        assert!(d.pending() > 0, "the partial big frame is buffered");
        // The rest.
        d.push(&stream[cut2..]);
        assert_eq!(collect(&mut d), vec![big.clone(), b"z".to_vec()], "big + trailing frame complete");
        assert!(d.is_empty());
    }

    #[test]
    fn empty_and_no_complete_frame() {
        let mut d = StreamDeframer::new();
        assert_eq!(collect(&mut d), Vec::<Vec<u8>>::new(), "no bytes → nothing");
        d.push(&[0x80]); // an incomplete length varint (continuation bit, no follow byte)
        assert_eq!(collect(&mut d), Vec::<Vec<u8>>::new(), "incomplete length → wait");
        assert_eq!(d.pending(), 1);
        d.push(&[0x01, b'X']); // completes len = 0x80|0x01<<7 = 128 → but body is 1 byte → still incomplete
        assert_eq!(collect(&mut d), Vec::<Vec<u8>>::new(), "length completes but body not arrived");
    }

    #[test]
    fn end_to_end_zero_copy_stream_read() {
        // The point: stream real wire messages and read a field of each IN PLACE (view_message over
        // the borrowed frame body — no per-frame decode, no whole-stream buffer).
        use crate::concurrency::marshal::{message_to_wire, view_message};
        use crate::interpreter::RuntimeValue;

        let m1 = message_to_wire("p", &RuntimeValue::Int(11)).unwrap();
        let m2 = message_to_wire("p", &RuntimeValue::Int(22)).unwrap();
        let m3 = message_to_wire("p", &RuntimeValue::Int(33)).unwrap();
        let stream = frame_all(&[&m1, &m2, &m3]);

        let mut d = StreamDeframer::new();
        let mut ints = Vec::new();
        // Deliver in awkward 7-byte chunks to force cross-frame splits.
        for chunk in stream.chunks(7) {
            d.push(chunk);
            d.drain_frames(|frame| {
                let v = view_message(frame).expect("frame opens as a view in place");
                ints.push(v.as_int().expect("reads the int with no full decode"));
            });
        }
        assert_eq!(ints, vec![11, 22, 33], "every streamed message read zero-copy, in order");
    }
}
