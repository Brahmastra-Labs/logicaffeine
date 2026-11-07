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
//!    never-reused segment per message.
//! 2. **No randomness at runtime.** [`PnpSuite::seal`] / [`PnpSuite::open`] draw zero entropy — they
//!    are pure deterministic functions of `(pad, ledger, plaintext)`. All randomness is the pad.
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
//! **Two regions per half.** A MAC key is only a secret while no ciphertext has exposed it, and a
//! cover's keystream is exposed to anyone who knows its plaintext. So keystream and MAC keys never
//! share pad: each directional half hands out keystream from its bottom, upward, and 32-byte MAC keys
//! from its top, downward, one *key slot* per cover; the half is spent when the two meet. A cover names
//! its key slot and its keystream offset, both under its tag, and a receiver keys the MAC only from the
//! key region — were it to key the MAC from wherever a frame said, a cover placed over a known stretch
//! of keystream would carry a MAC key its forger knows.
//!
//! **A durable ledger.** Reissuing a byte of pad is a two-time pad, and a cursor that lived only in
//! the process would start at the bottom on every run. Every reservation is made through a
//! [`PadLedgerStore`], which reads, advances and durably records the half's position under one
//! exclusive hold before the frame exists, so a crash can only waste pad, and two holders of one ledger
//! (two threads, two processes) never draw the same byte. The ledger also keeps the receive half's
//! high-water key slot, so a cover opened before a restart is refused after it. A ledger that cannot
//! be read closes the pad: it is never taken for a fresh one.
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
//! switching.
//!
//! Frames ride the [`super::channel`] envelope:
//!
//! ```text
//! [ CHAN_MAGIC | CHAN_VER | SUITE_PNP | kind:u8 | epoch:u32 | offset:u64 | len:u32 | slot:u64 | ct(len) | tag:16 ]
//! ```

use std::sync::{Arc, Mutex};

use logicaffeine_proof::ait::{classify_bytes, CompressibilityClass};
use logicaffeine_system::aead::poly1305;
use logicaffeine_system::keccak::sha3_256_bytes;

use super::channel::{
    ActiveSession, ReplayWindow, CHAN_HEADER_LEN, CHAN_MAGIC, CHAN_VER, SUITE_PNP,
};

/// The one-time Poly1305 MAC key length: each cover takes one key slot of this many pad bytes from
/// the top of its half, on top of the `len` keystream bytes it takes from the bottom.
pub const MAC_KEY_LEN: usize = 32;

/// The authenticator tag length (Poly1305).
pub const TAG_LEN: usize = 16;

/// Body field: cover kind ([`KIND_DATA`] / [`KIND_ROLL`]).
const KIND_LEN: usize = 1;
/// Body field: pad epoch this cover draws from.
const EPOCH_LEN: usize = 4;
/// Body field: half-local keystream offset of the cover.
const OFF_LEN: usize = 8;
/// Body field: payload length.
const LEN_LEN: usize = 4;
/// Body field: the key slot the cover's MAC key is drawn from.
const SLOT_LEN: usize = 8;
/// The fixed body preamble before the payload: `kind ‖ epoch ‖ offset ‖ len ‖ slot`.
const PREAMBLE: usize = KIND_LEN + EPOCH_LEN + OFF_LEN + LEN_LEN + SLOT_LEN;

/// A data cover: the payload is application ciphertext.
const KIND_DATA: u8 = 0;
/// A roll cover: the payload is `next_epoch:u32 ‖ next_commitment:32` — the authenticated handoff.
const KIND_ROLL: u8 = 1;

/// The default receiver reorder/replay window, in covers: a cover whose key slot is this many or more
/// behind the newest one opened can no longer be told from a replay, and is refused. Forward jumps (a
/// dropped cover skipped) are always fine.
pub const RECV_WINDOW_COVERS: u64 = 4096;

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

impl Role {
    /// The role's name, as a `with pad … as <role>` clause spells it.
    pub fn name(self) -> &'static str {
        match self {
            Role::Initiator => "initiator",
            Role::Responder => "responder",
        }
    }
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

/// Where one pad's two halves stand: what the send half has handed out, and how far the receive
/// half has opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PadLedger {
    /// Keystream bytes the send half has handed out, from its bottom.
    pub keystream: u64,
    /// Key slots the send half has handed out, from its top.
    pub key_slots: u64,
    /// One past the newest key slot the receive half has opened; 0 when it has opened none.
    pub opened_through: u64,
}

/// Why a ledger could not be read or recorded. Either way the pad is closed: a position that cannot be
/// read could be a position that has already handed out every byte below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerError(pub String);

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The durable record of a pad's position — the state whose loss would be catastrophic, because
/// reissuing a byte the send half has handed out is a two-time pad.
pub trait PadLedgerStore: Send + Sync {
    /// Read the ledger, let `change` change it, and record the change durably before returning, all
    /// under one exclusive hold: no other holder of this ledger reads it between the read and the
    /// record. `change` answers `false` to leave the ledger as it was. Returns the ledger as it now
    /// stands.
    fn update(
        &self,
        change: &mut dyn FnMut(&mut PadLedger) -> bool,
    ) -> Result<PadLedger, LedgerError>;
}

/// The in-memory ledger: correct for one process's sessions over a pad no other run will use.
#[derive(Default)]
pub struct MemLedger {
    state: Mutex<PadLedger>,
}

impl PadLedgerStore for MemLedger {
    fn update(
        &self,
        change: &mut dyn FnMut(&mut PadLedger) -> bool,
    ) -> Result<PadLedger, LedgerError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        change(&mut state);
        Ok(*state)
    }
}

/// The file ledger: the position lives in a file, every update holds an exclusive lock on a sibling
/// lock file across its read and its record, and a record is written to a temporary file, synced,
/// renamed over the ledger and the directory synced, so a crash leaves the old position or the new one
/// and a restart resumes past every byte ever handed out. A missing ledger is a fresh pad; a ledger
/// of any other shape, or one that cannot be read, is an error.
#[cfg(not(target_arch = "wasm32"))]
pub struct FileLedger {
    path: std::path::PathBuf,
}

/// The first eight bytes of a file ledger: its format.
#[cfg(not(target_arch = "wasm32"))]
const LEDGER_MAGIC: &[u8; 8] = b"LGPNPLD1";

/// A file ledger's length: its magic and three little-endian `u64`s.
#[cfg(not(target_arch = "wasm32"))]
const LEDGER_LEN: usize = 8 + 3 * 8;

#[cfg(not(target_arch = "wasm32"))]
impl FileLedger {
    /// A ledger kept at `path` (created on its first record).
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The ledger a `with pad "<pad>" as <role>` session keeps beside its pad.
    pub fn beside(pad: &std::path::Path, role: Role) -> Self {
        let mut name = pad.as_os_str().to_owned();
        name.push(format!(".{}.ledger", role.name()));
        Self::new(std::path::PathBuf::from(name))
    }

    /// Where the ledger is kept.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn sibling(&self, suffix: &str) -> std::path::PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(suffix);
        std::path::PathBuf::from(name)
    }

    fn error(&self, what: &str, why: impl std::fmt::Display) -> LedgerError {
        LedgerError(format!(
            "the pad ledger '{}' {what}: {why}",
            self.path.display()
        ))
    }

    fn read(&self) -> Result<PadLedger, LedgerError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PadLedger::default()),
            Err(e) => return Err(self.error("could not be read", e)),
        };
        if bytes.len() != LEDGER_LEN || &bytes[..8] != LEDGER_MAGIC {
            return Err(self.error(
                "is not a pad ledger",
                format!(
                    "{} bytes, where a ledger is {LEDGER_LEN} beginning LGPNPLD1",
                    bytes.len()
                ),
            ));
        }
        let word = |i: usize| u64::from_le_bytes(bytes[8 + 8 * i..16 + 8 * i].try_into().unwrap());
        Ok(PadLedger {
            keystream: word(0),
            key_slots: word(1),
            opened_through: word(2),
        })
    }

    fn record(&self, ledger: &PadLedger) -> Result<(), LedgerError> {
        use std::io::Write;
        let mut bytes = Vec::with_capacity(LEDGER_LEN);
        bytes.extend_from_slice(LEDGER_MAGIC);
        for word in [ledger.keystream, ledger.key_slots, ledger.opened_through] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let temporary = self.sibling(".tmp");
        let mut file =
            std::fs::File::create(&temporary).map_err(|e| self.error("could not be written", e))?;
        file.write_all(&bytes)
            .map_err(|e| self.error("could not be written", e))?;
        file.sync_all()
            .map_err(|e| self.error("could not be synced", e))?;
        std::fs::rename(&temporary, &self.path)
            .map_err(|e| self.error("could not be replaced", e))?;
        let directory = match self.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => std::path::Path::new("."),
        };
        std::fs::File::open(directory)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| self.error("could not be made durable in its directory", e))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PadLedgerStore for FileLedger {
    fn update(
        &self,
        change: &mut dyn FnMut(&mut PadLedger) -> bool,
    ) -> Result<PadLedger, LedgerError> {
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.sibling(".lock"))
            .map_err(|e| self.error("could not be locked", e))?;
        fs2::FileExt::lock_exclusive(&lock).map_err(|e| self.error("could not be locked", e))?;
        let mut ledger = self.read()?;
        if change(&mut ledger) {
            self.record(&ledger)?;
        }
        Ok(ledger)
    }
}

/// A keyed one-time-pad channel over one directional half of a [`PadPool`] at one pad epoch. A peer
/// *seals* with the suite for its send direction and *opens* with the suite for its receive
/// direction; because the two directions are different pad halves, a peer never seals and opens over
/// the same bytes.
pub struct PnpSuite {
    pad: Arc<[u8]>,
    base: usize,
    len: usize,
    epoch: u32,
    ledger: Arc<dyn PadLedgerStore>,
    recv: Mutex<RecvState>,
}

/// The receive half's memory of what it has opened: the exact window, once it has been resumed from
/// the ledger's high-water mark.
struct RecvState {
    span: u64,
    window: Option<ReplayWindow>,
}

impl PnpSuite {
    fn over(pad: Arc<[u8]>, base: usize, len: usize, epoch: u32) -> Self {
        Self {
            pad,
            base,
            len,
            epoch,
            ledger: Arc::new(MemLedger::default()),
            recv: Mutex::new(RecvState {
                span: RECV_WINDOW_COVERS,
                window: None,
            }),
        }
    }

    /// Keep this half's position in `ledger`: a durable one makes the position survive a restart.
    pub fn with_ledger(self, ledger: Arc<dyn PadLedgerStore>) -> Self {
        Self { ledger, ..self }
    }

    /// Set the receiver reorder/replay window, in covers: at least one, the newest.
    pub fn with_recv_window(self, covers: u64) -> Self {
        *self
            .recv
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = RecvState {
            span: covers.max(1),
            window: None,
        };
        self
    }

    /// The pad epoch this suite draws from.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// How many key slots this half holds.
    fn slots(&self) -> u64 {
        (self.len / MAC_KEY_LEN) as u64
    }

    /// The half-local start of key slot `slot`, counted down from the top.
    fn key_start(&self, slot: u64) -> u64 {
        self.len as u64 - MAC_KEY_LEN as u64 * (slot + 1)
    }

    /// Pad bytes neither region has handed out yet — the low-water gauge; 0 when the ledger cannot be
    /// read. When this falls below `MAC_KEY_LEN` plus a message's length, [`PnpSuite::seal`] fails
    /// closed.
    pub fn send_remaining(&self) -> usize {
        match self.ledger.update(&mut |_| false) {
            Ok(l) => (self.len as u64)
                .saturating_sub(l.keystream)
                .saturating_sub(l.key_slots.saturating_mul(MAC_KEY_LEN as u64))
                as usize,
            Err(_) => 0,
        }
    }

    /// Whether the send pad has dropped below `threshold` bytes — the signal to provision and roll to
    /// the next epoch before exhaustion.
    pub fn is_low(&self, threshold: usize) -> bool {
        self.send_remaining() < threshold
    }

    /// Seal `blob` into a fresh data cover. `None` — fail-closed — when the pad is exhausted or its
    /// ledger cannot be read or recorded.
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

    /// Reserve the next cover of `len` keystream bytes: its key slot and its keystream offset, durably
    /// recorded before the frame exists.
    fn reserve(&self, len: u64) -> Option<(u64, u64)> {
        let mut reserved = None;
        self.ledger
            .update(&mut |l| {
                let slot = l.key_slots;
                if slot >= self.slots() {
                    return false;
                }
                match l.keystream.checked_add(len) {
                    Some(end) if end <= self.key_start(slot) => {
                        reserved = Some((slot, l.keystream));
                        l.keystream = end;
                        l.key_slots = slot + 1;
                        true
                    }
                    _ => false,
                }
            })
            .ok()?;
        reserved
    }

    fn mac_key(&self, slot: u64) -> [u8; MAC_KEY_LEN] {
        let at = self.base + self.key_start(slot) as usize;
        self.pad[at..at + MAC_KEY_LEN]
            .try_into()
            .expect("a key slot is 32 bytes")
    }

    fn seal_kind(&self, kind: u8, blob: &[u8]) -> Option<Vec<u8>> {
        let len = blob.len();
        if len > u32::MAX as usize {
            return None;
        }
        let (slot, offset) = self.reserve(len as u64)?;
        let mac_key = self.mac_key(slot);

        let mut out = Vec::with_capacity(CHAN_HEADER_LEN + PREAMBLE + len + TAG_LEN);
        out.push(CHAN_MAGIC);
        out.push(CHAN_VER);
        out.extend_from_slice(&SUITE_PNP.to_le_bytes());
        out.push(kind);
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&slot.to_le_bytes());
        // Write ciphertext = plaintext ⊕ pad straight into the frame buffer via a vectorizable slice
        // loop (the shape LLVM turns into SIMD): grow with a cheap memset, then overwrite in one pass.
        let payload_start = CHAN_HEADER_LEN + PREAMBLE;
        out.resize(payload_start + len, 0);
        let at = self.base + offset as usize;
        let xor = &self.pad[at..at + len];
        for ((d, b), p) in out[payload_start..payload_start + len]
            .iter_mut()
            .zip(blob)
            .zip(xor)
        {
            *d = b ^ p;
        }

        // MAC the contiguous body: kind ‖ epoch ‖ offset ‖ len ‖ slot ‖ ciphertext — no copy.
        let tag = poly1305(&mac_key, &out[CHAN_HEADER_LEN..payload_start + len]);
        out.extend_from_slice(&tag);
        Some(out)
    }

    /// Open a data cover, or `None` on any malformed / tampered / replayed / wrong-epoch frame, or when
    /// the ledger cannot record it.
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
        if body.len() < PREAMBLE + TAG_LEN || body[0] != want_kind {
            return None;
        }
        let epoch = u32::from_le_bytes(body[KIND_LEN..KIND_LEN + EPOCH_LEN].try_into().ok()?);
        if epoch != self.epoch {
            return None; // a cover from a different pad epoch
        }
        let off_at = KIND_LEN + EPOCH_LEN;
        let offset = u64::from_le_bytes(body[off_at..off_at + OFF_LEN].try_into().ok()?);
        let len_at = off_at + OFF_LEN;
        let len = u32::from_le_bytes(body[len_at..len_at + LEN_LEN].try_into().ok()?) as usize;
        let slot_at = len_at + LEN_LEN;
        let slot = u64::from_le_bytes(body[slot_at..slot_at + SLOT_LEN].try_into().ok()?);
        if body.len() != PREAMBLE + len + TAG_LEN || slot >= self.slots() {
            return None;
        }
        if offset.checked_add(len as u64)? > self.key_start(slot) {
            return None; // keystream reaching into the key region is no cover this half sealed
        }
        let tag = &body[PREAMBLE + len..];
        let expect = poly1305(&self.mac_key(slot), &body[..PREAMBLE + len]);
        if !ct_eq(&expect, tag) {
            return None; // tamper — nothing recorded, so the genuine cover can still arrive
        }

        let mut recv = self
            .recv
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if recv.window.is_none() {
            let through = self.ledger.update(&mut |_| false).ok()?.opened_through;
            recv.window = Some(ReplayWindow::resumed_at(recv.span, through));
        }
        if !recv.window.as_ref().is_some_and(|w| w.admits(slot)) {
            return None; // a replay, or beyond the reorder window
        }
        self.ledger
            .update(&mut |l| {
                let through = slot + 1;
                if through > l.opened_through {
                    l.opened_through = through;
                    true
                } else {
                    false
                }
            })
            .ok()?;
        if let Some(window) = recv.window.as_mut() {
            window.accept(slot);
        }
        drop(recv);

        let at = self.base + offset as usize;
        let xor = &self.pad[at..at + len];
        Some(
            body[PREAMBLE..PREAMBLE + len]
                .iter()
                .zip(xor)
                .map(|(c, p)| c ^ p)
                .collect(),
        )
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
        PadPool {
            pad: pad.into(),
            split,
            epoch: 0,
        }
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
    /// path is one-time-pad protected end-to-end. Its position lives in memory: for a pad any other run
    /// will use, [`PadPool::session_with_ledger`].
    pub fn session(&self, role: Role) -> PnpSession {
        PnpSession::new(self.send_suite(role), self.recv_suite(role))
    }

    /// A live [`PnpSession`] for `role` whose two halves keep their position in `ledger` (one ledger
    /// for the pair: the send half records its reservations and the receive half its high-water mark).
    pub fn session_with_ledger(&self, role: Role, ledger: Arc<dyn PadLedgerStore>) -> PnpSession {
        PnpSession::new(
            self.send_suite(role).with_ledger(ledger.clone()),
            self.recv_suite(role).with_ledger(ledger),
        )
    }
}

/// The cover kind of a `PNP` frame (`KIND_DATA` / `KIND_ROLL`), or `None` if it is not a
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

/// The half-local keystream offset a `PNP` frame draws from, or `None` if it is not a well-formed
/// `PNP` frame. Useful for monitoring the pad high-water mark without opening the cover.
pub fn frame_offset(frame: &[u8]) -> Option<u64> {
    frame_word(frame, KIND_LEN + EPOCH_LEN)
}

/// The key slot a `PNP` frame's MAC key is drawn from, or `None` if it is not a well-formed `PNP`
/// frame.
pub fn frame_key_slot(frame: &[u8]) -> Option<u64> {
    frame_word(frame, KIND_LEN + EPOCH_LEN + OFF_LEN + LEN_LEN)
}

fn frame_word(frame: &[u8], body_at: usize) -> Option<u64> {
    let at = CHAN_HEADER_LEN + body_at;
    if frame.len() < at + 8
        || frame[0] != CHAN_MAGIC
        || frame[1] != CHAN_VER
        || u16::from_le_bytes([frame[2], frame[3]]) != SUITE_PNP
    {
        return None;
    }
    Some(u64::from_le_bytes(frame[at..at + 8].try_into().ok()?))
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/concurrency/pnp/tests.rs"]
mod tests;
