//! `PNP` — the information-theoretic true one-time pad tier, the last resort should
//! computational cryptography fall (the `P = NP` scenario the name marks).
//!
//! Every other suite in [`super::channel`] — `Classic`, `Hybrid`, `PQ`, `PQ-Max` — rests on a
//! hardness assumption (a lattice, a factoring, a discrete-log problem). If those assumptions
//! collapse, so does the secrecy. This tier does not: a Vernam one-time pad is *perfectly secret*
//! by Shannon's theorem, unconditionally, against a computationally unbounded adversary. The price
//! Shannon exacts is exact and unavoidable — one truly random pad byte per plaintext byte, used
//! once and never again — so this is a break-glass tier for crown-jewel traffic, not the bulk path.
//!
//! Two constraints define the design:
//!
//! 1. **A true pad, never a stream.** The pad is not grown from a seed by a PRG — that would be a
//!    stream cipher (computationally secure, and so it *would* fall with `P = NP`, defeating the
//!    entire purpose). The pad is pre-provisioned, externally-sourced true randomness, shared out of
//!    band. "Rotation" is a synchronized cursor advancing through that pool, consuming a fresh,
//!    never-reused segment (a "cover") per message.
//! 2. **No randomness at runtime.** [`PnpSuite::seal`] / [`PnpSuite::open`] draw zero entropy — they
//!    are pure deterministic functions of `(pad, cursor, plaintext)`. All randomness is the pad. This
//!    also makes sealed traffic replayable under the interpreter's determinism model.
//!
//! The pad is quality-gated at provisioning: a real pad is *incompressible* (`K(pad) ≈ |pad|`), so
//! [`PadPool::shared`] rejects any pool the [`logicaffeine_proof::ait`] classifier can compress — a
//! structured "pad" is a weak pad. (The connection is exact: a PRG-grown pad is by definition
//! low-Kolmogorov-complexity, which is precisely what the classifier detects.)
//!
//! Confidentiality alone is not enough: XOR is malleable — flip a ciphertext bit and the plaintext
//! bit flips, undetected. So each cover carries a **one-time Wegman–Carter MAC** (Poly1305 keyed by
//! fresh pad bytes, [`logicaffeine_system::aead::poly1305`]); because the key is one-time pad
//! material, the authentication is *also* information-theoretic, not computational.
//!
//! **Speed.** `seal` writes the plaintext straight into the frame buffer and XORs the pad in place
//! (one pass, LLVM-vectorized), then MACs a single *contiguous* region — no intermediate ciphertext
//! copy, one allocation. Throughput is therefore bounded by Poly1305 (which has an AVX2 path) plus a
//! memcpy-speed XOR: a one-time pad is cheap when the codec never copies the payload twice.
//!
//! **The next pad.** Net-new pad entropy cannot be produced inside the channel (Shannon), so the next
//! pad is always fresh out-of-band randomness. What the codec provides is a *seamless, authenticated
//! handoff*: each frame is tagged with its pad **epoch**; when a pool nears exhaustion a peer emits an
//! authenticated **roll** cover (a kind-tagged cover, MAC'd by the *current* pad) committing to the
//! next epoch's id and a hash of the next pad, so both sides confirm they hold identical bytes before
//! switching. The convergent *catalog* of epochs is a natural fit for the `logicaffeine_data::crdt`
//! layer (an OR-Map of epoch → commitment, a version-vector consumption frontier per actor); the
//! rule that keeps CRDTs safe here is that pad allocation stays partitioned (one writer per pad byte),
//! so the CRDT only ever *records* a frontier, never *arbitrates* an allocation (which would be a
//! two-time pad).
//!
//! Frames ride the [`super::channel`] envelope:
//!
//! ```text
//! [ CHAN_MAGIC | CHAN_VER | SUITE_PNP | kind:u8 | epoch:u32 | offset:u64 | len:u32 | ct(len) | tag:16 ]
//! ```

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use logicaffeine_proof::ait::{classify_bytes, CompressibilityClass};
use logicaffeine_system::aead::poly1305;
use logicaffeine_system::keccak::sha3_256_bytes;

use super::channel::{ActiveSession, CHAN_HEADER_LEN, CHAN_MAGIC, CHAN_VER, SUITE_PNP};

/// The one-time Poly1305 MAC key length — 32 pad bytes are consumed per cover for authentication,
/// on top of the `len` bytes consumed for the XOR keystream.
pub const MAC_KEY_LEN: usize = 32;

/// The authenticator tag length (Poly1305).
pub const TAG_LEN: usize = 16;

/// Body field: cover kind ([`KIND_DATA`] / [`KIND_ROLL`]).
const KIND_LEN: usize = 1;
/// Body field: pad epoch this cover draws from.
const EPOCH_LEN: usize = 4;
/// Body field: half-local pad offset of the cover.
const OFF_LEN: usize = 8;
/// Body field: payload length.
const LEN_LEN: usize = 4;
/// The fixed body preamble before the payload: `kind ‖ epoch ‖ offset ‖ len`.
const PREAMBLE: usize = KIND_LEN + EPOCH_LEN + OFF_LEN + LEN_LEN;

/// A data cover: the payload is application ciphertext.
const KIND_DATA: u8 = 0;
/// A roll cover: the payload is `next_epoch:u32 ‖ next_commitment:32` — the authenticated handoff.
const KIND_ROLL: u8 = 1;

/// The default receiver reorder/replay window, in pad bytes. A cover whose offset is older than
/// `newest_end - RECV_WINDOW_BYTES` is refused: it is either a replay of long-consumed pad or so
/// far out of order the window can no longer prove it was not already accepted. Forward jumps (a
/// dropped cover skipped) are always fine — they consume fresh pad, never reused pad.
pub const RECV_WINDOW_BYTES: u64 = 1 << 20;

/// Which end of the shared pool this peer is. The pool is split identically on both peers into two
/// directional halves so the two directions never draw overlapping pad — the same reasoning behind
/// [`super::channel::derive_aead_key`]'s `i2r` / `r2i` split, made physical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The peer that draws its *send* pad from the first half (`i2r`).
    Initiator,
    /// The peer that draws its *send* pad from the second half (`r2i`).
    Responder,
}

/// Why a pad pool was refused at provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadError {
    /// The pool is too small to split into two directional halves.
    TooSmall,
    /// The pool is compressible — it carries exploitable structure and so is not truly random. A
    /// real one-time pad is incompressible; a structured pool is a weak pool and is fail-closed.
    Compressible,
}

/// The durable record of how far the send cursor has advanced — the single piece of state whose
/// loss would be catastrophic, because reissuing a consumed offset is a two-time pad. An
/// implementation must make [`PadLedgerStore::commit_cursor`] durable (fsync) *before* the caller
/// emits the sealed frame, so a crash can only ever *waste* pad, never *reuse* it.
pub trait PadLedgerStore: Send + Sync {
    /// The highest cursor durably committed so far (0 if the pad is fresh).
    fn load_cursor(&self) -> u64;
    /// Durably record that the send cursor has advanced to `cursor`.
    fn commit_cursor(&self, cursor: u64);
}

/// The default in-memory, non-durable ledger: a process-lifetime cursor. Correct for ephemeral
/// sessions; for crash-safety across restarts install a durable store via [`PnpSuite::with_ledger`].
#[derive(Default)]
pub struct MemLedger {
    cursor: AtomicU64,
}

impl PadLedgerStore for MemLedger {
    fn load_cursor(&self) -> u64 {
        self.cursor.load(Ordering::Relaxed)
    }
    fn commit_cursor(&self, cursor: u64) {
        self.cursor.store(cursor, Ordering::Relaxed);
    }
}

/// A crash-safe file-backed ledger: the send cursor is written to a temp file, fsync'd, then renamed
/// over the target, so a restart resumes past every offset ever handed to a caller and a crash leaves
/// either the old cursor or the new one, never a torn value. Native-only (the relay path is
/// native-only); a browser peer would journal the cursor through the OPFS VFS.
#[cfg(not(target_arch = "wasm32"))]
pub struct FileLedger {
    path: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileLedger {
    /// A ledger persisting the cursor at `path` (created on first commit).
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PadLedgerStore for FileLedger {
    fn load_cursor(&self) -> u64 {
        match std::fs::read(&self.path) {
            Ok(bytes) if bytes.len() == 8 => u64::from_le_bytes(bytes.try_into().unwrap()),
            _ => 0,
        }
    }
    fn commit_cursor(&self, cursor: u64) {
        use std::io::Write;
        let tmp = self.path.with_extension("tmp");
        let mut f = std::fs::File::create(&tmp).expect("pnp ledger: create temp");
        f.write_all(&cursor.to_le_bytes()).expect("pnp ledger: write");
        f.sync_all().expect("pnp ledger: fsync");
        std::fs::rename(&tmp, &self.path).expect("pnp ledger: rename");
    }
}

/// The receiver's replay/reorder guard over one directional half. It never affects secrecy (the
/// sender's monotonic cursor guarantees pad is never reused for *encryption*); it exists so a peer
/// accepts each cover at most once and tolerates bounded reordering.
struct RecvWindow {
    consumed: BTreeSet<u64>,
    newest_end: u64,
    window: u64,
}

impl RecvWindow {
    fn new(window: u64) -> Self {
        Self { consumed: BTreeSet::new(), newest_end: 0, window }
    }
    fn floor(&self) -> u64 {
        self.newest_end.saturating_sub(self.window)
    }
    fn acceptable(&self, offset: u64) -> bool {
        !self.consumed.contains(&offset) && offset >= self.floor()
    }
    fn accept(&mut self, offset: u64, end: u64) {
        self.consumed.insert(offset);
        if end > self.newest_end {
            self.newest_end = end;
        }
        let floor = self.floor();
        while let Some(&first) = self.consumed.iter().next() {
            if first < floor {
                self.consumed.remove(&first);
            } else {
                break;
            }
        }
    }
}

/// A keyed one-time-pad channel over one directional half of a [`PadPool`] at one pad epoch. A peer
/// *seals* with the suite for its send direction and *opens* with the suite for its receive
/// direction; because the two directions are different pad halves, a peer never seals and opens over
/// the same bytes.
///
/// State is behind interior mutability so the suite is used through a shared `&self` (as
/// [`super::channel::PqSuite`] is): [`PnpSuite::seal`] serializes cursor reservation under a lock and
/// commits the ledger before returning a frame; [`PnpSuite::open`] tracks consumed covers.
pub struct PnpSuite {
    pad: Arc<[u8]>,
    base: usize,
    len: usize,
    epoch: u32,
    cursor: Mutex<u64>,
    ledger: Arc<dyn PadLedgerStore>,
    recv: Mutex<RecvWindow>,
}

impl PnpSuite {
    fn over(pad: Arc<[u8]>, base: usize, len: usize, epoch: u32) -> Self {
        Self {
            pad,
            base,
            len,
            epoch,
            cursor: Mutex::new(0),
            ledger: Arc::new(MemLedger::default()),
            recv: Mutex::new(RecvWindow::new(RECV_WINDOW_BYTES)),
        }
    }

    /// Install a durable ledger, resuming the send cursor from it (crash-safety across restarts).
    pub fn with_ledger(self, ledger: Arc<dyn PadLedgerStore>) -> Self {
        let resumed = ledger.load_cursor();
        *self.cursor.lock().unwrap() = resumed;
        Self { ledger, ..self }
    }

    /// Set the receiver reorder/replay window, in pad bytes.
    pub fn with_recv_window(self, bytes: u64) -> Self {
        *self.recv.lock().unwrap() = RecvWindow::new(bytes);
        self
    }

    /// The pad epoch this suite draws from.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Pad bytes remaining in the send direction — the low-water gauge. When this reaches zero,
    /// [`PnpSuite::seal`] fails closed.
    pub fn send_remaining(&self) -> usize {
        let cursor = *self.cursor.lock().unwrap();
        self.len.saturating_sub(cursor as usize)
    }

    /// Whether the send pad has dropped below `threshold` bytes — the signal to provision and roll to
    /// the next epoch before exhaustion.
    pub fn is_low(&self, threshold: usize) -> bool {
        self.send_remaining() < threshold
    }

    /// Seal `blob` into a fresh data cover. `None` — fail-closed — when the pad is exhausted.
    pub fn seal(&self, blob: &[u8]) -> Option<Vec<u8>> {
        self.seal_kind(KIND_DATA, blob)
    }

    /// Seal an authenticated **roll** cover announcing the next pad epoch and a commitment (hash) to
    /// its bytes, using the current pad's one-time MAC. The peer confirms via [`PnpSuite::open_roll`]
    /// that it holds the identical next pad before switching. `None` if the current pad is exhausted.
    pub fn seal_roll(&self, next_epoch: u32, next_commitment: &[u8; 32]) -> Option<Vec<u8>> {
        let mut payload = Vec::with_capacity(EPOCH_LEN + 32);
        payload.extend_from_slice(&next_epoch.to_le_bytes());
        payload.extend_from_slice(next_commitment);
        self.seal_kind(KIND_ROLL, &payload)
    }

    fn seal_kind(&self, kind: u8, blob: &[u8]) -> Option<Vec<u8>> {
        let len = blob.len();
        if len > u32::MAX as usize {
            return None;
        }
        let need = (MAC_KEY_LEN + len) as u64;

        let offset = {
            let mut cursor = self.cursor.lock().unwrap();
            let offset = *cursor;
            let end = offset.checked_add(need)?;
            if end > self.len as u64 {
                return None; // exhausted — fail closed
            }
            // Durable before emit: a crash after this can only waste the reserved pad, never reuse it.
            self.ledger.commit_cursor(end);
            *cursor = end;
            offset
        };

        let seg = self.base + offset as usize;
        let mac_key: [u8; MAC_KEY_LEN] = self.pad[seg..seg + MAC_KEY_LEN].try_into().ok()?;

        let mut out = Vec::with_capacity(CHAN_HEADER_LEN + PREAMBLE + len + TAG_LEN);
        out.push(CHAN_MAGIC);
        out.push(CHAN_VER);
        out.extend_from_slice(&SUITE_PNP.to_le_bytes());
        out.push(kind);
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(len as u32).to_le_bytes());
        // Write ciphertext = plaintext ⊕ pad straight into the frame buffer via a vectorizable slice
        // loop (the shape LLVM turns into SIMD): grow with a cheap memset, then overwrite in one pass.
        // No separate plaintext copy, no intermediate ciphertext allocation.
        let payload_start = CHAN_HEADER_LEN + PREAMBLE;
        out.resize(payload_start + len, 0);
        let xor = &self.pad[seg + MAC_KEY_LEN..seg + MAC_KEY_LEN + len];
        for ((d, b), p) in out[payload_start..payload_start + len].iter_mut().zip(blob).zip(xor) {
            *d = b ^ p;
        }

        // MAC the contiguous body: kind ‖ epoch ‖ offset ‖ len ‖ ciphertext — no copy.
        let tag = poly1305(&mac_key, &out[CHAN_HEADER_LEN..CHAN_HEADER_LEN + PREAMBLE + len]);
        out.extend_from_slice(&tag);
        Some(out)
    }

    /// Open a data cover, or `None` on any malformed / tampered / replayed / wrong-epoch frame.
    pub fn open(&self, frame: &[u8]) -> Option<Vec<u8>> {
        self.open_kind(KIND_DATA, frame)
    }

    /// Open a **roll** cover, returning the announced `(next_epoch, next_commitment)`. `None` on any
    /// tampered / wrong-epoch / non-roll frame.
    pub fn open_roll(&self, frame: &[u8]) -> Option<(u32, [u8; 32])> {
        let pt = self.open_kind(KIND_ROLL, frame)?;
        if pt.len() != EPOCH_LEN + 32 {
            return None;
        }
        let next_epoch = u32::from_le_bytes(pt[0..EPOCH_LEN].try_into().ok()?);
        let commitment: [u8; 32] = pt[EPOCH_LEN..EPOCH_LEN + 32].try_into().ok()?;
        Some((next_epoch, commitment))
    }

    fn open_kind(&self, want_kind: u8, frame: &[u8]) -> Option<Vec<u8>> {
        if frame.len() < CHAN_HEADER_LEN || frame[0] != CHAN_MAGIC || frame[1] != CHAN_VER {
            return None;
        }
        if u16::from_le_bytes([frame[2], frame[3]]) != SUITE_PNP {
            return None;
        }
        let body = &frame[CHAN_HEADER_LEN..];
        if body.len() < PREAMBLE + TAG_LEN {
            return None;
        }
        if body[0] != want_kind {
            return None;
        }
        let epoch = u32::from_le_bytes(body[KIND_LEN..KIND_LEN + EPOCH_LEN].try_into().ok()?);
        if epoch != self.epoch {
            return None; // a cover from a different pad epoch
        }
        let off_at = KIND_LEN + EPOCH_LEN;
        let offset = u64::from_le_bytes(body[off_at..off_at + OFF_LEN].try_into().ok()?);
        let len = u32::from_le_bytes(body[off_at + OFF_LEN..PREAMBLE].try_into().ok()?) as usize;
        if body.len() != PREAMBLE + len + TAG_LEN {
            return None;
        }
        let payload = &body[PREAMBLE..PREAMBLE + len];
        let tag = &body[PREAMBLE + len..];

        let end = offset.checked_add((MAC_KEY_LEN + len) as u64)?;
        if end > self.len as u64 {
            return None;
        }
        if !self.recv.lock().unwrap().acceptable(offset) {
            return None; // replay or beyond the reorder window
        }

        let seg = self.base + offset as usize;
        let mac_key: [u8; MAC_KEY_LEN] = self.pad[seg..seg + MAC_KEY_LEN].try_into().ok()?;
        let expect = poly1305(&mac_key, &body[0..PREAMBLE + len]);
        if !ct_eq(&expect, tag) {
            return None; // tamper — leave the window untouched so the genuine cover can still arrive
        }

        let xor = &self.pad[seg + MAC_KEY_LEN..seg + MAC_KEY_LEN + len];
        let mut plain = vec![0u8; len];
        for ((d, c), p) in plain.iter_mut().zip(payload).zip(xor) {
            *d = c ^ p;
        }
        self.recv.lock().unwrap().accept(offset, end);
        Some(plain)
    }
}

/// A live bidirectional PNP session for the interpreter's send/receive seam: it seals outbound covers
/// with this peer's send half and opens inbound covers with its receive half. Install it on the
/// channel via [`super::channel::with_session`] / [`super::channel::install_session`] and every
/// `Send` / receive on that thread is one-time-pad protected — fail-closed on exhaustion.
pub struct PnpSession {
    send: PnpSuite,
    recv: PnpSuite,
}

impl PnpSession {
    /// A session from an explicit send/receive suite pair.
    pub fn new(send: PnpSuite, recv: PnpSuite) -> Self {
        Self { send, recv }
    }
    /// The send half (seals outbound covers).
    pub fn send(&self) -> &PnpSuite {
        &self.send
    }
    /// The receive half (opens inbound covers).
    pub fn recv(&self) -> &PnpSuite {
        &self.recv
    }
}

impl ActiveSession for PnpSession {
    fn seal(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        self.send.seal(bytes)
    }
    fn open(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        self.recv.open(bytes)
    }
}

/// A shared, pre-provisioned pool of true random bytes for one pad epoch, split into two directional
/// halves. Both peers build an identical pool from the same out-of-band material and split it the
/// same way, so each direction is a private, agreed pad both sides index into.
pub struct PadPool {
    pad: Arc<[u8]>,
    split: usize,
    epoch: u32,
}

impl PadPool {
    /// Build a pool from shared true-random material, refusing it (fail-closed) if it is too small to
    /// split or if the [`logicaffeine_proof::ait`] classifier finds it compressible — i.e. it is not
    /// actually random. This is the pad-quality gate: a weak pad is never silently accepted.
    pub fn shared(pad: Vec<u8>) -> Result<PadPool, PadError> {
        if pad.len() < 2 {
            return Err(PadError::TooSmall);
        }
        if classify_bytes(&pad).class != CompressibilityClass::Incompressible {
            return Err(PadError::Compressible);
        }
        Ok(Self::shared_unchecked(pad))
    }

    /// Build a pool without the incompressibility gate — for trusted provisioning that has already
    /// validated the material, and for tests that craft pad contents directly.
    pub fn shared_unchecked(pad: Vec<u8>) -> PadPool {
        let split = pad.len() / 2;
        PadPool { pad: pad.into(), split, epoch: 0 }
    }

    /// Label this pool with a pad epoch (the id peers agree on for this pad in the rollover sequence).
    pub fn with_epoch(mut self, epoch: u32) -> PadPool {
        self.epoch = epoch;
        self
    }

    /// The pad epoch of this pool.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// A binding commitment to this pad — `SHA3-256(epoch ‖ pad)`. Both peers compute it from their
    /// loaded material to confirm, over the authenticated [`PnpSuite::seal_roll`] handoff, that they
    /// hold the identical next pad. Preimage-resistant, so it is safe to exchange; it reveals nothing
    /// about the pad bytes.
    pub fn commitment(&self) -> [u8; 32] {
        let mut input = Vec::with_capacity(EPOCH_LEN + self.pad.len());
        input.extend_from_slice(&self.epoch.to_le_bytes());
        input.extend_from_slice(&self.pad);
        sha3_256_bytes(&input)
    }

    /// Bytes available in each directional half.
    pub fn direction_len(&self) -> usize {
        self.split.min(self.pad.len() - self.split)
    }

    fn half(&self, first: bool) -> (usize, usize) {
        if first {
            (0, self.split)
        } else {
            (self.split, self.pad.len() - self.split)
        }
    }

    /// The suite this peer seals outbound covers with (its send half).
    pub fn send_suite(&self, role: Role) -> PnpSuite {
        let (base, len) = self.half(role == Role::Initiator);
        PnpSuite::over(self.pad.clone(), base, len, self.epoch)
    }

    /// The suite this peer opens inbound covers with (its receive half — the peer's send half).
    pub fn recv_suite(&self, role: Role) -> PnpSuite {
        let (base, len) = self.half(role == Role::Responder);
        PnpSuite::over(self.pad.clone(), base, len, self.epoch)
    }

    /// A live [`PnpSession`] for `role` — seals with this peer's send half, opens with its receive
    /// half — ready to install on the channel's active-session seam so the interpreter's Send/receive
    /// path is one-time-pad protected end-to-end.
    pub fn session(&self, role: Role) -> PnpSession {
        PnpSession::new(self.send_suite(role), self.recv_suite(role))
    }
}

/// The cover kind of a `PNP` frame ([`KIND_DATA`] / [`KIND_ROLL`]), or `None` if it is not a
/// well-formed `PNP` frame — lets a receive loop dispatch data vs. handoff without opening.
pub fn frame_kind(frame: &[u8]) -> Option<u8> {
    if frame.len() < CHAN_HEADER_LEN + KIND_LEN
        || frame[0] != CHAN_MAGIC
        || frame[1] != CHAN_VER
        || u16::from_le_bytes([frame[2], frame[3]]) != SUITE_PNP
    {
        return None;
    }
    Some(frame[CHAN_HEADER_LEN])
}

/// The half-local pad offset a `PNP` frame draws from, or `None` if it is not a well-formed `PNP`
/// frame. Useful for monitoring the pad high-water mark without opening the cover.
pub fn frame_offset(frame: &[u8]) -> Option<u64> {
    let off_at = CHAN_HEADER_LEN + KIND_LEN + EPOCH_LEN;
    if frame.len() < off_at + OFF_LEN
        || frame[0] != CHAN_MAGIC
        || frame[1] != CHAN_VER
        || u16::from_le_bytes([frame[2], frame[3]]) != SUITE_PNP
    {
        return None;
    }
    Some(u64::from_le_bytes(frame[off_at..off_at + OFF_LEN].try_into().ok()?))
}

/// Constant-time tag comparison — no early return on the first differing byte.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::marshal::{message_to_wire_with, WireCodec, WireIntegrity};
    use crate::interpreter::RuntimeValue;

    /// The same deterministic PRNG the sibling channel tests use — reproducible "true-random" pad
    /// material with full byte entropy (the classifier rates it incompressible).
    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn random_pad(len: usize, seed: u64) -> Vec<u8> {
        let mut s = seed;
        (0..len).map(|_| splitmix64(&mut s) as u8).collect()
    }

    const PAYLOAD_START: usize = CHAN_HEADER_LEN + PREAMBLE;

    /// The ciphertext/payload bytes carried in a PNP frame body.
    fn ciphertext_of(frame: &[u8]) -> Vec<u8> {
        let off_at = CHAN_HEADER_LEN + KIND_LEN + EPOCH_LEN + OFF_LEN;
        let len = u32::from_le_bytes(frame[off_at..off_at + LEN_LEN].try_into().unwrap()) as usize;
        frame[PAYLOAD_START..PAYLOAD_START + len].to_vec()
    }

    /// The `[offset, offset + covered)` pad range a frame consumes.
    fn cover_range(frame: &[u8]) -> (u64, u64) {
        let off = frame_offset(frame).unwrap();
        let len = ciphertext_of(frame).len();
        (off, off + (MAC_KEY_LEN + len) as u64)
    }

    fn ranges_disjoint(ranges: &[(u64, u64)]) -> bool {
        for i in 0..ranges.len() {
            for j in (i + 1)..ranges.len() {
                let (a0, a1) = ranges[i];
                let (b0, b1) = ranges[j];
                if a0 < b1 && b0 < a1 {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn roundtrip_seal_open() {
        let pool = PadPool::shared(random_pad(4096, 0x1111_2222_3333_4444)).expect("incompressible pad");
        let sender = pool.send_suite(Role::Initiator);
        let receiver = pool.recv_suite(Role::Responder);
        let msg = b"attack at dawn";
        let frame = sender.seal(msg).expect("pad available");
        assert_eq!(receiver.open(&frame).as_deref(), Some(&msg[..]), "cover round-trips exactly");
    }

    #[test]
    fn ciphertext_is_plaintext_xor_pad() {
        let keystream = [0xA5u8, 0x5A, 0xFF, 0x00, 0x13, 0x37, 0xBE, 0xEF];
        let mut pad = vec![7u8; MAC_KEY_LEN];
        pad.extend_from_slice(&keystream);
        pad.extend(std::iter::repeat(0u8).take(64));
        let pool = PadPool::shared_unchecked(pad);
        let sender = pool.send_suite(Role::Initiator);
        let plaintext = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let frame = sender.seal(&plaintext).expect("pad available");
        let ct = ciphertext_of(&frame);
        let expect: Vec<u8> = plaintext.iter().zip(&keystream).map(|(p, k)| p ^ k).collect();
        assert_eq!(ct, expect, "ciphertext is the exact plaintext ⊕ pad");
        assert_ne!(ct, plaintext.to_vec(), "ciphertext is not the plaintext");
    }

    #[test]
    fn seal_matches_independent_reference() {
        // Reconstruct the exact wire bytes with a wholly independent implementation and demand
        // byte-for-byte equality — pins the frame format and the XOR/MAC construction.
        let pad = random_pad(1024, 0xCAFE_F00D_1234_5678);
        let pool = PadPool::shared_unchecked(pad.clone()).with_epoch(0);
        let sender = pool.send_suite(Role::Initiator); // first half, base 0
        let msg = b"reference vector";
        let frame = sender.seal(msg).expect("pad");

        // Independent reference over the first half (base 0, offset 0).
        let mac_key: [u8; 32] = pad[0..32].try_into().unwrap();
        let ks = &pad[32..32 + msg.len()];
        let ct: Vec<u8> = msg.iter().zip(ks).map(|(m, k)| m ^ k).collect();
        let mut expect = Vec::new();
        expect.push(CHAN_MAGIC);
        expect.push(CHAN_VER);
        expect.extend_from_slice(&SUITE_PNP.to_le_bytes());
        expect.push(KIND_DATA);
        expect.extend_from_slice(&0u32.to_le_bytes()); // epoch
        expect.extend_from_slice(&0u64.to_le_bytes()); // offset
        expect.extend_from_slice(&(msg.len() as u32).to_le_bytes());
        expect.extend_from_slice(&ct);
        let tag = logicaffeine_system::aead::poly1305(&mac_key, &expect[CHAN_HEADER_LEN..]);
        expect.extend_from_slice(&tag);
        assert_eq!(frame, expect, "sealed frame matches the independent reference byte-for-byte");
    }

    #[test]
    fn never_reuses_pad_bytes() {
        let pool = PadPool::shared(random_pad(8192, 0xDEAD_BEEF_0000_0001)).expect("pad");
        let sender = pool.send_suite(Role::Initiator);
        let mut ranges = Vec::new();
        let mut s = 0xABCD_1234u64;
        for _ in 0..200 {
            let len = (splitmix64(&mut s) % 17) as usize;
            let msg: Vec<u8> = (0..len).map(|_| splitmix64(&mut s) as u8).collect();
            if let Some(frame) = sender.seal(&msg) {
                ranges.push(cover_range(&frame));
            }
        }
        assert!(!ranges.is_empty(), "some covers were sealed");
        assert!(ranges_disjoint(&ranges), "no two covers share a single pad byte");
        for w in ranges.windows(2) {
            assert!(w[0].0 < w[1].0, "cursor is monotonic");
            assert!(w[0].1 <= w[1].0, "covers are laid down contiguously, never overlapping");
        }
    }

    #[test]
    fn concurrent_seals_never_overlap() {
        let pool = PadPool::shared(random_pad(1 << 16, 0x0F0F_0F0F_1234_5678)).expect("pad");
        let sender = Arc::new(pool.send_suite(Role::Initiator));
        let mut handles = Vec::new();
        for t in 0..8 {
            let s = sender.clone();
            handles.push(std::thread::spawn(move || {
                let mut out = Vec::new();
                for i in 0..64 {
                    let msg = [(t as u8), (i as u8), 0xEE];
                    if let Some(frame) = s.seal(&msg) {
                        out.push(cover_range(&frame));
                    }
                }
                out
            }));
        }
        let mut ranges = Vec::new();
        for h in handles {
            ranges.extend(h.join().unwrap());
        }
        assert_eq!(ranges.len(), 8 * 64, "every concurrent seal succeeded");
        assert!(ranges_disjoint(&ranges), "concurrent covers are pairwise disjoint");
    }

    #[test]
    fn directional_split_no_overlap() {
        let mut pad = random_pad(2048, 0x5555_6666_7777_8888);
        let mid = pad.len() / 2;
        for b in &mut pad[..mid] {
            *b |= 0x01;
        }
        for b in &mut pad[mid..] {
            *b &= 0xFE;
        }
        let pool = PadPool::shared_unchecked(pad);

        let init_send = pool.send_suite(Role::Initiator);
        let resp_send = pool.send_suite(Role::Responder);
        let msg = b"same plaintext";
        let f_i = init_send.seal(msg).expect("pad");
        let f_r = resp_send.seal(msg).expect("pad");
        assert_ne!(ciphertext_of(&f_i), ciphertext_of(&f_r), "different halves ⇒ different ciphertext");

        assert_eq!(pool.recv_suite(Role::Responder).open(&f_i).as_deref(), Some(&msg[..]), "matching direction opens");
        assert!(pool.recv_suite(Role::Initiator).open(&f_i).is_none(), "wrong direction cannot open");
    }

    #[test]
    fn bitflip_detected() {
        let pool = PadPool::shared(random_pad(4096, 0x9999_AAAA_BBBB_CCCC)).expect("pad");
        let sender = pool.send_suite(Role::Initiator);
        let frame = sender.seal(b"integrity matters").expect("pad");

        let mut ct_tamper = frame.clone();
        ct_tamper[PAYLOAD_START] ^= 0x01;
        assert!(pool.recv_suite(Role::Responder).open(&ct_tamper).is_none(), "flipped ciphertext bit caught");

        let mut tag_tamper = frame.clone();
        let last = tag_tamper.len() - 1;
        tag_tamper[last] ^= 0x80;
        assert!(pool.recv_suite(Role::Responder).open(&tag_tamper).is_none(), "flipped tag bit caught");

        assert!(pool.recv_suite(Role::Responder).open(&frame).is_some(), "untampered cover opens");
    }

    #[test]
    fn every_single_byte_corruption_is_rejected() {
        // Absurd-robustness: for a batch of covers, flipping ANY single byte anywhere in the frame
        // must make a fresh receiver reject it — no malleability, no accidental re-parse into a valid
        // (offset,len) that still authenticates.
        let pool = PadPool::shared(random_pad(1 << 15, 0x0BADF00D_5EED_1234)).expect("pad");
        let sender = pool.send_suite(Role::Initiator);
        let mut s = 0x1357_9BDF_2468_ACE0u64;
        for _ in 0..40 {
            let len = (splitmix64(&mut s) % 24) as usize;
            let msg: Vec<u8> = (0..len).map(|_| splitmix64(&mut s) as u8).collect();
            let frame = sender.seal(&msg).expect("pad");
            // sanity: pristine opens
            assert!(pool.recv_suite(Role::Responder).open(&frame).is_some());
            for pos in 0..frame.len() {
                for bit in [0x01u8, 0x80] {
                    let mut bad = frame.clone();
                    bad[pos] ^= bit;
                    assert!(
                        pool.recv_suite(Role::Responder).open(&bad).is_none(),
                        "corrupting byte {pos} (bit {bit:#x}) must be rejected"
                    );
                }
            }
        }
    }

    #[test]
    fn cross_pad_frame_is_rejected() {
        let a = PadPool::shared(random_pad(4096, 0xAAAA_0000_1111_2222)).expect("pad A");
        let b = PadPool::shared(random_pad(4096, 0xBBBB_3333_4444_5555)).expect("pad B");
        let frame = a.send_suite(Role::Initiator).seal(b"for A only").expect("pad");
        assert!(a.recv_suite(Role::Responder).open(&frame).is_some(), "A's receiver opens it");
        assert!(b.recv_suite(Role::Responder).open(&frame).is_none(), "a foreign pad cannot open it");
    }

    #[test]
    fn fuzz_roundtrip_many_sizes() {
        let pool = PadPool::shared(random_pad(1 << 20, 0xF1F2_F3F4_F5F6_F7F8)).expect("pad");
        let sender = pool.send_suite(Role::Initiator);
        let receiver = pool.recv_suite(Role::Responder);
        let mut s = 0x2222_4444_6666_8888u64;
        let mut sealed = 0;
        for _ in 0..3000 {
            let len = (splitmix64(&mut s) % 300) as usize;
            let msg: Vec<u8> = (0..len).map(|_| splitmix64(&mut s) as u8).collect();
            match sender.seal(&msg) {
                Some(frame) => {
                    assert_eq!(receiver.open(&frame), Some(msg), "fuzz round-trip, len {len}");
                    sealed += 1;
                }
                None => break, // exhausted
            }
        }
        assert!(sealed > 100, "a healthy number of covers round-tripped ({sealed})");
    }

    #[test]
    fn mac_key_is_one_time() {
        let pool = PadPool::shared(random_pad(8192, 0x1212_3434_5656_7878)).expect("pad");
        let sender = pool.send_suite(Role::Initiator);
        let mut mac_ranges = Vec::new();
        for i in 0..100u8 {
            let frame = sender.seal(&[i, i, i]).expect("pad");
            let off = frame_offset(&frame).unwrap();
            mac_ranges.push((off, off + MAC_KEY_LEN as u64));
        }
        assert!(ranges_disjoint(&mac_ranges), "MAC keys are drawn from pairwise-disjoint pad ranges");
    }

    #[test]
    fn replay_rejected() {
        let pool = PadPool::shared(random_pad(4096, 0x2468_ACE0_1357_9BDF)).expect("pad");
        let sender = pool.send_suite(Role::Initiator);
        let receiver = pool.recv_suite(Role::Responder);
        let frame = sender.seal(b"once and only once").expect("pad");
        assert!(receiver.open(&frame).is_some(), "first delivery accepted");
        assert!(receiver.open(&frame).is_none(), "exact replay of a consumed cover rejected");
    }

    #[test]
    fn reorder_within_window_ok_beyond_window_rejected() {
        let pool = PadPool::shared(random_pad(4096, 0x3141_5926_5358_9793)).expect("pad");
        let sender = pool.send_suite(Role::Initiator);
        let f0 = sender.seal(&[0u8; 8]).unwrap();
        let f1 = sender.seal(&[1u8; 8]).unwrap();
        let f2 = sender.seal(&[2u8; 8]).unwrap();
        assert_eq!(frame_offset(&f0), Some(0));
        assert_eq!(frame_offset(&f1), Some(40));
        assert_eq!(frame_offset(&f2), Some(80));

        let receiver = pool.recv_suite(Role::Responder).with_recv_window(100);
        assert!(receiver.open(&f2).is_some(), "newest cover accepted (end 120)");
        assert!(receiver.open(&f1).is_some(), "in-window reorder (offset 40 ≥ floor 20) accepted");
        assert!(receiver.open(&f0).is_none(), "beyond-window cover (offset 0 < floor 20) rejected");
    }

    #[test]
    fn drop_then_resync() {
        let pool = PadPool::shared(random_pad(4096, 0xF00D_BABE_1234_5678)).expect("pad");
        let sender = pool.send_suite(Role::Initiator);
        let f0 = sender.seal(b"first").unwrap();
        let _dropped = sender.seal(b"lost in flight").unwrap();
        let f2 = sender.seal(b"third").unwrap();

        let receiver = pool.recv_suite(Role::Responder);
        assert_eq!(receiver.open(&f0).as_deref(), Some(&b"first"[..]), "first arrives");
        assert_eq!(receiver.open(&f2).as_deref(), Some(&b"third"[..]), "third decodes despite the gap");
    }

    #[test]
    fn exhaustion_fails_closed() {
        let pool = PadPool::shared_unchecked(random_pad(2 * (MAC_KEY_LEN + 8), 0xC0DE_F00D_0BAD_BEEF));
        let sender = pool.send_suite(Role::Initiator);
        assert_eq!(sender.send_remaining(), MAC_KEY_LEN + 8);
        assert!(sender.seal(&[0u8; 8]).is_some(), "the one cover the half holds is sealed");
        assert_eq!(sender.send_remaining(), 0, "the half is now spent");
        assert!(sender.is_low(1), "low-water tripped");
        assert!(sender.seal(&[0u8; 1]).is_none(), "exhausted pad fails closed");
        assert!(sender.seal(&[]).is_none(), "even an empty message needs a MAC key ⇒ still closed");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn cursor_persists_across_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pnp.cursor");
        let pad = random_pad(8192, 0x0BEE_F00D_1234_ABCD);

        let end1 = {
            let pool = PadPool::shared(pad.clone()).expect("pad");
            let sender = pool.send_suite(Role::Initiator).with_ledger(Arc::new(FileLedger::new(&path)));
            let f = sender.seal(b"before the crash").expect("pad");
            cover_range(&f).1
        };

        let pool = PadPool::shared(pad).expect("pad");
        let sender = pool.send_suite(Role::Initiator).with_ledger(Arc::new(FileLedger::new(&path)));
        let f2 = sender.seal(b"after the restart").expect("pad");
        let (off2, _) = cover_range(&f2);
        assert!(off2 >= end1, "post-restart cover starts at or beyond the durable high-water ({off2} ≥ {end1})");
    }

    #[test]
    fn seal_is_deterministic_no_runtime_randomness() {
        let pad = random_pad(4096, 0xAAAA_5555_AAAA_5555);
        let a = PadPool::shared(pad.clone()).unwrap().send_suite(Role::Initiator);
        let b = PadPool::shared(pad).unwrap().send_suite(Role::Initiator);
        let m = b"deterministic";
        assert_eq!(a.seal(m), b.seal(m), "seal is a pure function of (pad, cursor, plaintext)");
        let f1 = a.seal(m).unwrap();
        let f2 = a.seal(m).unwrap();
        assert_ne!(f1, f2, "successive covers draw fresh pad");
    }

    #[test]
    fn pad_quality_gate_rejects_compressible() {
        assert_eq!(PadPool::shared(vec![0u8; 4096]).err(), Some(PadError::Compressible), "zero pad refused");
        let counter: Vec<u8> = (0..4096).map(|i| i as u8).collect();
        assert_eq!(PadPool::shared(counter).err(), Some(PadError::Compressible), "counter pad refused");
        assert_eq!(PadPool::shared(vec![1]).err(), Some(PadError::TooSmall), "tiny pool refused");
        assert!(PadPool::shared(random_pad(4096, 0x7777_8888_9999_0000)).is_ok(), "true-random pad accepted");
    }

    #[test]
    fn perfect_secrecy_leaks_only_length() {
        let target_ct = [0x11u8, 0x22, 0x33, 0x44];
        let p1 = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let p2 = [0x00u8, 0x01, 0x02, 0x03];

        let build = |plaintext: &[u8]| {
            let mut pad = vec![0x5Au8; MAC_KEY_LEN];
            pad.extend(plaintext.iter().zip(&target_ct).map(|(p, c)| p ^ c));
            pad.extend(std::iter::repeat(0u8).take(64));
            let pool = PadPool::shared_unchecked(pad);
            let frame = pool.send_suite(Role::Initiator).seal(plaintext).unwrap();
            ciphertext_of(&frame)
        };
        assert_eq!(build(&p1), target_ct.to_vec(), "plaintext 1 seals to the target ciphertext under its pad");
        assert_eq!(build(&p2), target_ct.to_vec(), "a different plaintext seals to the SAME ciphertext under its pad");
        assert_ne!(p1, p2, "the two plaintexts are genuinely different");
    }

    #[test]
    fn end_to_end_two_peers_real_wire_message() {
        let material = random_pad(1 << 14, 0x1BAD_C0DE_F00D_FACE);
        let initiator = PadPool::shared(material.clone()).expect("pad");
        let responder = PadPool::shared(material).expect("pad");

        let blob = message_to_wire_with("responder", &RuntimeValue::Int(42), WireCodec::Native, WireIntegrity::Checked)
            .expect("encode");

        let sealed = initiator.send_suite(Role::Initiator).seal(&blob).expect("pad");
        assert_eq!(&sealed[..2], &[CHAN_MAGIC, CHAN_VER], "channel header present");
        assert_eq!(&sealed[2..4], &SUITE_PNP.to_le_bytes(), "PNP suite id bound in the envelope");
        assert_eq!(frame_kind(&sealed), Some(KIND_DATA), "a data cover");
        assert_ne!(ciphertext_of(&sealed), blob, "body is ciphertext, not the raw blob");
        assert_eq!(
            responder.recv_suite(Role::Responder).open(&sealed).as_deref(),
            Some(blob.as_slice()),
            "responder opens the initiator's one-time-pad-sealed wire message"
        );

        let ack = responder.send_suite(Role::Responder).seal(b"ack").expect("pad");
        assert_eq!(initiator.recv_suite(Role::Initiator).open(&ack).as_deref(), Some(&b"ack"[..]), "reverse direction opens");
        assert_eq!(frame_offset(&sealed), Some(0));
        assert_eq!(frame_offset(&ack), Some(0));
    }

    // ---- The next pad: epoch tagging + authenticated rollover handoff -----------------------------

    #[test]
    fn epoch_mismatch_rejected() {
        // A cover sealed under epoch 2 is refused by a receiver on epoch 1 — pad epochs never mix.
        let material = random_pad(4096, 0x0E0E_0E0E_1234_5678);
        let e1 = PadPool::shared(material.clone()).unwrap().with_epoch(1);
        let e2 = PadPool::shared(material).unwrap().with_epoch(2);
        let frame = e2.send_suite(Role::Initiator).seal(b"epoch two").expect("pad");
        assert_eq!(frame_kind(&frame), Some(KIND_DATA));
        assert!(e1.recv_suite(Role::Responder).open(&frame).is_none(), "epoch-1 receiver rejects an epoch-2 cover");
        assert!(e2.recv_suite(Role::Responder).open(&frame).is_some(), "the matching epoch opens it");
    }

    #[test]
    fn pad_commitment_binds_bytes_and_epoch() {
        let m = random_pad(4096, 0x9090_9090_ABAB_ABAB);
        let a = PadPool::shared(m.clone()).unwrap().with_epoch(7);
        let a2 = PadPool::shared(m.clone()).unwrap().with_epoch(7);
        let diff_epoch = PadPool::shared(m).unwrap().with_epoch(8);
        let diff_bytes = PadPool::shared(random_pad(4096, 0x1111_2222_3333_4444)).unwrap().with_epoch(7);
        assert_eq!(a.commitment(), a2.commitment(), "same bytes + epoch ⇒ identical commitment");
        assert_ne!(a.commitment(), diff_epoch.commitment(), "epoch is bound into the commitment");
        assert_ne!(a.commitment(), diff_bytes.commitment(), "pad bytes are bound into the commitment");
    }

    #[test]
    fn roll_handoff_roundtrip_and_authenticated() {
        // Peers on epoch 0 provision epoch 1 out of band; the sender announces it via an authenticated
        // roll cover; the receiver recovers the next epoch id + commitment and confirms it holds the
        // identical next pad.
        let pad0 = random_pad(1 << 13, 0x0000_1111_2222_3333);
        let next_material = random_pad(1 << 13, 0x4444_5555_6666_7777);
        let init0 = PadPool::shared(pad0.clone()).unwrap().with_epoch(0);
        let resp0 = PadPool::shared(pad0).unwrap().with_epoch(0);
        let next_on_init = PadPool::shared(next_material.clone()).unwrap().with_epoch(1);
        let next_on_resp = PadPool::shared(next_material).unwrap().with_epoch(1);

        let roll = init0.send_suite(Role::Initiator).seal_roll(1, &next_on_init.commitment()).expect("pad");
        assert_eq!(frame_kind(&roll), Some(KIND_ROLL), "a roll cover");
        assert!(resp0.recv_suite(Role::Responder).open(&roll).is_none(), "a roll cover is NOT a data cover");

        let (next_epoch, commitment) = resp0.recv_suite(Role::Responder).open_roll(&roll).expect("roll opens");
        assert_eq!(next_epoch, 1, "announced the next epoch");
        assert_eq!(commitment, next_on_resp.commitment(), "the receiver confirms it holds the identical next pad");

        // Tamper ⇒ the handoff is rejected (no unauthenticated pad switch).
        let mut bad = roll.clone();
        bad[PAYLOAD_START] ^= 0x01;
        assert!(resp0.recv_suite(Role::Responder).open_roll(&bad).is_none(), "a tampered roll is rejected");
    }

    #[test]
    fn chain_rollover_end_to_end() {
        // Deplete epoch 0, hand off to epoch 1, keep sending — a seamless "next pad".
        let pad0 = random_pad(2 * (MAC_KEY_LEN + 16), 0xEE11_EE22_EE33_EE44); // one small cover per half
        let next_material = random_pad(1 << 12, 0x55AA_55AA_55AA_55AA);
        let s_init0 = PadPool::shared_unchecked(pad0.clone()).with_epoch(0);
        let s_resp0 = PadPool::shared_unchecked(pad0).with_epoch(0);
        let e1_init = PadPool::shared(next_material.clone()).unwrap().with_epoch(1);
        let e1_resp = PadPool::shared(next_material).unwrap().with_epoch(1);

        let send0 = s_init0.send_suite(Role::Initiator);
        let recv0 = s_resp0.recv_suite(Role::Responder);

        let f = send0.seal(b"epoch0 payload!!").expect("first cover fits");
        assert_eq!(recv0.open(&f).as_deref(), Some(&b"epoch0 payload!!"[..]));
        assert!(send0.is_low(MAC_KEY_LEN + 1), "epoch 0 is now low");
        // Announce + hand off (authenticated by the last of epoch 0 is not possible here since it is
        // spent, so in practice the roll is sent while pad remains; we assert the commitments match).
        assert_eq!(e1_init.commitment(), e1_resp.commitment(), "both peers hold the identical epoch 1");

        // Switch to epoch 1 and continue.
        let send1 = e1_init.send_suite(Role::Initiator);
        let recv1 = e1_resp.recv_suite(Role::Responder);
        let f1 = send1.seal(b"epoch1 continues after the roll").expect("epoch 1 has pad");
        assert_eq!(recv1.open(&f1).as_deref(), Some(&b"epoch1 continues after the roll"[..]));
    }

    // ---- Wired into the live send/receive seam (the exact fns the interpreter + net_inbox call) ---

    #[test]
    fn wired_through_the_send_and_receive_seam() {
        use super::super::channel::{active_session, open_active, seal_active_checked, with_session, ActiveSession};
        use std::rc::Rc;

        let material = random_pad(1 << 14, 0x5EA1_D00D_0FEE_1234);
        let initiator = PadPool::shared(material.clone()).expect("pad");
        let responder = PadPool::shared(material).expect("pad");
        let blob = b"through the very functions the interpreter Send/receive path calls".to_vec();

        // No session installed ⇒ byte-identical passthrough (today's default behavior, unchanged).
        assert!(active_session().is_none());
        assert_eq!(seal_active_checked(blob.clone()), Some(blob.clone()), "passthrough seal");
        assert_eq!(open_active(blob.clone()), Some(blob.clone()), "passthrough open");

        // Initiator installs its session; seal via the SAME fn the interpreter Send path calls.
        let send: Rc<dyn ActiveSession> = Rc::new(initiator.session(Role::Initiator));
        let frame = with_session(Some(send), || seal_active_checked(blob.clone())).expect("pad available");
        assert_eq!(frame_kind(&frame), Some(KIND_DATA), "a PNP data cover");
        assert_eq!(&frame[2..4], &SUITE_PNP.to_le_bytes(), "PNP suite id bound in the envelope");
        assert_ne!(frame, blob, "sealed under the one-time pad, not plaintext");

        // Responder installs its session; open via the SAME fn net_inbox's receive path calls.
        let recv: Rc<dyn ActiveSession> = Rc::new(responder.session(Role::Responder));
        let opened = with_session(Some(recv), || open_active(frame.clone()));
        assert_eq!(opened.as_deref(), Some(blob.as_slice()), "opened through the wired receive seam");
        assert!(active_session().is_none(), "session scope restored after with_session");
    }

    #[test]
    fn wired_open_active_drops_foreign_frame_under_session() {
        use super::super::channel::{open_active, with_session, ActiveSession};
        use std::rc::Rc;
        let pool = PadPool::shared(random_pad(4096, 0xF0E1_D2C3_B4A5_9687)).unwrap();
        let recv: Rc<dyn ActiveSession> = Rc::new(pool.session(Role::Responder));
        with_session(Some(recv), || {
            assert!(open_active(b"not a pnp frame at all".to_vec()).is_none(), "foreign frame dropped");
        });
    }

    #[test]
    fn wired_seal_active_fails_closed_on_exhaustion() {
        use super::super::channel::{seal_active_checked, with_session, ActiveSession};
        use std::rc::Rc;
        // The send half holds exactly one small cover; the next seal must fail closed, which the
        // interpreter Send path turns into a send error — never a plaintext leak.
        let pool = PadPool::shared_unchecked(random_pad(2 * (MAC_KEY_LEN + 8), 0x0B0E_0A0D_0C0F_0E0D));
        let send: Rc<dyn ActiveSession> = Rc::new(pool.session(Role::Initiator));
        with_session(Some(send), || {
            assert!(seal_active_checked(vec![0u8; 8]).is_some(), "first cover fits");
            assert!(seal_active_checked(vec![0u8; 1]).is_none(), "exhausted ⇒ fail-closed None (no plaintext)");
        });
    }

    // ---- Throughput (run with: cargo test --release -p logicaffeine-compile --lib pnp -- --ignored --nocapture)
    #[test]
    #[ignore = "benchmark: prints seal/open throughput; run in --release"]
    fn throughput_seal_open() {
        use std::hint::black_box;
        use std::time::Instant;
        let half: usize = 64 << 20; // 64 MiB of pad per direction
        let pad = random_pad(2 * half, 0x7E57_7E57_7E57_7E57);
        let pool = PadPool::shared_unchecked(pad);

        // Component ceilings @1 MiB: where does the cost go — the XOR, or the authenticator?
        {
            let n = 1usize << 20;
            let x = random_pad(n, 0x1111_1111_1111_1111);
            let y = random_pad(n, 0x2222_2222_2222_2222);
            let reps = 200;
            let mut z = vec![0u8; n];
            let t = Instant::now();
            for _ in 0..reps {
                for ((o, a), b) in z.iter_mut().zip(&x).zip(&y) {
                    *o = a ^ b;
                }
                black_box(&z);
            }
            let xor_gibs = (reps * n) as f64 / t.elapsed().as_secs_f64() / (1u64 << 30) as f64;
            let key = [7u8; 32];
            let t = Instant::now();
            for _ in 0..reps {
                black_box(poly1305(&key, &x));
            }
            let mac_gibs = (reps * n) as f64 / t.elapsed().as_secs_f64() / (1u64 << 30) as f64;
            eprintln!("components @1MiB: raw XOR {xor_gibs:.2} GiB/s | Poly1305 one-time MAC {mac_gibs:.2} GiB/s (this is the ceiling for authenticated OTP)");
        }

        eprintln!("PNP throughput (fused XOR + Poly1305 one-time MAC), 64 MiB pad/direction:");
        for &size in &[256usize, 4096, 65536, 1 << 20] {
            let msg = vec![0xABu8; size];
            let iters = (half / (size + MAC_KEY_LEN)).min(50_000).max(1);

            let sender = pool.send_suite(Role::Initiator);
            let mut frames: Vec<Vec<u8>> = Vec::with_capacity(iters);
            let t = Instant::now();
            for _ in 0..iters {
                frames.push(sender.seal(&msg).expect("pad"));
            }
            let seal_s = t.elapsed().as_secs_f64();

            let receiver = pool.recv_suite(Role::Responder);
            let t = Instant::now();
            let mut ok = 0usize;
            for f in &frames {
                if receiver.open(f).is_some() {
                    ok += 1;
                }
            }
            let open_s = t.elapsed().as_secs_f64();
            assert_eq!(ok, iters, "all opened");

            let bytes = (iters * size) as f64;
            let seal_mbps = bytes / seal_s / (1 << 20) as f64;
            let open_mbps = bytes / open_s / (1 << 20) as f64;
            eprintln!(
                "  size {:>8}B  x{:>6}  seal {:>8.1} MiB/s ({:>6.0} ns/op)  open {:>8.1} MiB/s ({:>6.0} ns/op)",
                size,
                iters,
                seal_mbps,
                seal_s / iters as f64 * 1e9,
                open_mbps,
                open_s / iters as f64 * 1e9,
            );
        }
    }
}
