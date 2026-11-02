//! `SecureChannel` envelope — the end-to-end crypto seam that wraps the opaque
//! wire blob `marshal` produces, between the codec and the transport.
//!
//! Layering: the channel owns its *own* versioned envelope
//! and never inspects or mutates the payload, exactly as the FEC layer in [`super::fec`]
//! does. `seal` frames an opaque blob under a suite id; `open` reverses it, returning
//! `None` on any malformed input — the same `None`-on-malformed contract
//! [`super::marshal::message_from_wire`] honours, so a corrupt or truncated envelope is
//! dropped, never decoded.
//!
//! ```text
//! [ CHAN_MAGIC | CHAN_VER | suite_id (LE u16) | body ]
//! ```
//!
//! The `null` suite (`SUITE_NULL`) is identity framing — no cryptography — and exists to
//! prove the embedding seam end-to-end before any primitive lands. Real suites replace
//! `body` with `handshake/sequence ‖ AEAD(blob)+tag` under their own suite id; because the
//! suite id is bound in the envelope, swapping a suite is a registry change, never a wire
//! break.

/// Envelope magic byte — distinct from [`super::fec`]'s `0xFE` so a framed blob is
/// self-identifying.
pub(crate) const CHAN_MAGIC: u8 = 0xC0;

/// Envelope format version.
pub(crate) const CHAN_VER: u8 = 1;

/// Fixed header: magic (1) + version (1) + suite id (2, little-endian).
pub(crate) const CHAN_HEADER_LEN: usize = 4;

/// The suite id of the identity suite.
pub const SUITE_NULL: u16 = 0;

/// The suite id of the post-quantum suite: an ML-KEM-768 handshake establishes the shared secret,
/// SHAKE256 derives the key, and every body is ChaCha20-Poly1305 AEAD-sealed.
pub const SUITE_PQ: u16 = 1;

/// The suite id of the `PNP` tier — the information-theoretic true one-time pad (see [`super::pnp`]).
/// It is the last resort should computational cryptography fall (the `P = NP` scenario): its secrecy
/// rests on Shannon, not on any hardness assumption. Like [`SUITE_PQ`] it is keyed and stateful, so
/// it lives outside the stateless [`suite_for`] registry and is used through [`super::pnp::PnpSuite`]
/// rather than [`seal`] / [`open`].
pub const SUITE_PNP: u16 = 2;

/// A pluggable crypto suite. Each posture level — `null`, `Classic`, `Hybrid`, `PQ`,
/// `PQ-Max` — is one `Suite` registered in [`suite_for`], so adding a primitive is a
/// registration, not a change to [`seal`] / [`open`]. The seam stays suite-agnostic; all
/// cryptography lives behind this trait (crypto-agility).
pub trait Suite: Sync {
    /// The wire suite id bound into the envelope header.
    fn id(&self) -> u16;
    /// Transform an opaque blob into the envelope body — identity for `null`; for a real
    /// suite, `handshake/sequence ‖ AEAD(blob)+tag`.
    fn seal_body(&self, blob: &[u8]) -> Vec<u8>;
    /// Reverse [`Suite::seal_body`], or `None` on a tampered / malformed body.
    fn open_body(&self, body: &[u8]) -> Option<Vec<u8>>;
}

/// The identity suite: no cryptography, `body == blob`. Proves the seam; never the shipped
/// default once a real suite exists.
pub struct NullSuite;

impl Suite for NullSuite {
    fn id(&self) -> u16 {
        SUITE_NULL
    }
    fn seal_body(&self, blob: &[u8]) -> Vec<u8> {
        blob.to_vec()
    }
    fn open_body(&self, body: &[u8]) -> Option<Vec<u8>> {
        Some(body.to_vec())
    }
}

/// Resolve a registered STATELESS suite by its wire id, or `None` for an unknown suite (so
/// [`open`] returns `None` rather than decoding under the wrong primitive). The keyed [`PqSuite`]
/// is not here — it carries a per-session key and is used via [`seal_with`] / [`open_with`].
pub fn suite_for(id: u16) -> Option<&'static dyn Suite> {
    static NULL: NullSuite = NullSuite;
    match id {
        SUITE_NULL => Some(&NULL),
        _ => None,
    }
}

/// How many sequence numbers behind the newest it has opened a [`PqSuite`] still opens a frame at: a
/// frame further back can no longer be told from a replay, and is refused.
pub const PQ_REPLAY_WINDOW: u64 = 1024;

/// The sequence numbers a receiver has opened, over a window of `span` numbers ending at the newest
/// (the anti-replay window of RFC 4303 §3.4.3): each number opens once, and a number `span` or more
/// behind the newest is refused, since the window no longer remembers whether it opened.
pub(crate) struct ReplayWindow {
    span: u64,
    newest: Option<u64>,
    seen: Vec<u64>,
}

impl ReplayWindow {
    pub(crate) fn new(span: u64) -> Self {
        assert!(span >= 1, "a replay window spans at least one number");
        Self {
            span,
            newest: None,
            seen: vec![0; span.div_ceil(64) as usize],
        }
    }

    /// A window that has opened every number below `opened_through` and none from it on: where a
    /// receiver that remembers only its high-water mark resumes, refusing all it may have opened.
    pub(crate) fn resumed_at(span: u64, opened_through: u64) -> Self {
        let mut window = Self::new(span);
        if let Some(newest) = opened_through.checked_sub(1) {
            window.newest = Some(newest);
            window.seen.fill(u64::MAX);
        }
        window
    }

    fn slot(&self, seq: u64) -> (usize, u64) {
        let at = seq % self.span;
        ((at / 64) as usize, 1u64 << (at % 64))
    }

    /// Whether `seq` would open: not opened before, and not too far behind the newest to tell.
    pub(crate) fn admits(&self, seq: u64) -> bool {
        match self.newest {
            Some(newest) if seq <= newest => {
                let (word, bit) = self.slot(seq);
                newest - seq < self.span && self.seen[word] & bit == 0
            }
            _ => true,
        }
    }

    /// Record `seq` as opened: `false`, recording nothing, when the window does not admit it.
    pub(crate) fn accept(&mut self, seq: u64) -> bool {
        if !self.admits(seq) {
            return false;
        }
        match self.newest {
            Some(newest) if seq <= newest => {}
            Some(newest) if seq - newest < self.span => {
                for skipped in newest + 1..seq {
                    let (word, bit) = self.slot(skipped);
                    self.seen[word] &= !bit;
                }
                self.newest = Some(seq);
            }
            _ => {
                self.seen.fill(0);
                self.newest = Some(seq);
            }
        }
        let (word, bit) = self.slot(seq);
        self.seen[word] |= bit;
        true
    }
}

/// The post-quantum suite: a per-session ChaCha20-Poly1305 key (from an ML-KEM-768 handshake,
/// see [`pq_handshake_initiator`] / [`pq_handshake_responder`]). Each `seal_body` draws the next
/// sequence number as its nonce, so no two frames share one, and the nonce rides in the body. The
/// opening side opens each sequence number once, in any order within [`PQ_REPLAY_WINDOW`] of the
/// newest, so a captured frame cannot be delivered twice. The suite id is bound as the AEAD
/// associated data, so a frame can't be replayed under a different suite. (Bidirectional use derives a
/// key per direction — see the handshake.)
pub struct PqSuite {
    key: [u8; 32],
    seq: std::sync::atomic::AtomicU64,
    opened: std::sync::Mutex<ReplayWindow>,
}

impl PqSuite {
    /// A PQ suite over an already-derived 32-byte AEAD key.
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key,
            seq: std::sync::atomic::AtomicU64::new(0),
            opened: std::sync::Mutex::new(ReplayWindow::new(PQ_REPLAY_WINDOW)),
        }
    }
}

impl Suite for PqSuite {
    fn id(&self) -> u16 {
        SUITE_PQ
    }
    fn seal_body(&self, blob: &[u8]) -> Vec<u8> {
        let seq = self
            .seq
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |seq| seq.checked_add(1),
            )
            .expect("a PQ session seals at most 2^64 - 1 frames: another would reuse a nonce");
        let mut nonce = [0u8; 12];
        nonce[..8].copy_from_slice(&seq.to_le_bytes());
        let sealed = logicaffeine_system::aead::chacha20poly1305_seal(
            &self.key,
            &nonce,
            &SUITE_PQ.to_le_bytes(),
            blob,
        );
        let mut body = Vec::with_capacity(12 + sealed.len());
        body.extend_from_slice(&nonce);
        body.extend_from_slice(&sealed);
        body
    }
    fn open_body(&self, body: &[u8]) -> Option<Vec<u8>> {
        if body.len() < 12 {
            return None;
        }
        let nonce: [u8; 12] = body[..12].try_into().ok()?;
        let blob = logicaffeine_system::aead::chacha20poly1305_open(
            &self.key,
            &nonce,
            &SUITE_PQ.to_le_bytes(),
            &body[12..],
        )?;
        let seq = u64::from_le_bytes(nonce[..8].try_into().ok()?);
        let first = self
            .opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accept(seq);
        first.then_some(blob)
    }
}

/// Derive a directional 32-byte AEAD key from an ML-KEM shared secret via SHAKE256, domain-separated
/// by `label` (e.g. `b"i2r"` / `b"r2i"`) so each direction has an independent key — no nonce reuse
/// across the two streams that share the handshake secret.
pub fn derive_aead_key(shared_secret: &[u8], label: &[u8]) -> [u8; 32] {
    let mut input = Vec::with_capacity(shared_secret.len() + label.len() + 24);
    input.extend_from_slice(b"logos-pq-channel-v1\x00");
    input.extend_from_slice(label);
    input.extend_from_slice(shared_secret);
    let mut k = [0u8; 32];
    k.copy_from_slice(&logicaffeine_system::keccak::shake256_bytes(&input, 32));
    k
}

/// The freshly generated handshake material an initiator publishes + retains.
pub struct PqHandshake {
    /// ML-KEM-768 encapsulation key, sent to the responder.
    pub ek: Vec<u8>,
    /// ML-KEM-768 decapsulation key, kept secret.
    dk: Vec<u8>,
}

/// Initiator step 1: generate an ML-KEM-768 keypair. Publish [`PqHandshake::ek`] to the responder.
pub fn pq_handshake_initiator(seed_d: &[u8; 32], seed_z: &[u8; 32]) -> PqHandshake {
    let (ek, dk) = logicaffeine_system::mlkem::keygen(seed_d, seed_z);
    PqHandshake { ek, dk }
}

/// Responder: encapsulate to the initiator's `ek`, returning the ciphertext to send back and the
/// two directional suites (`initiator→responder`, `responder→initiator`). An `ek` that fails the
/// FIPS 203 encapsulation-key checks is refused, never encapsulated to.
pub fn pq_handshake_responder(
    ek: &[u8],
    msg: &[u8; 32],
) -> Result<(Vec<u8>, PqSuite, PqSuite), logicaffeine_system::mlkem::KemError> {
    let (ct, ss) = logicaffeine_system::mlkem::encaps(ek, msg)?;
    Ok((
        ct,
        PqSuite::new(derive_aead_key(&ss, b"i2r")),
        PqSuite::new(derive_aead_key(&ss, b"r2i")),
    ))
}

/// Initiator step 2: decapsulate the responder's ciphertext, yielding the matching directional
/// suites (`initiator→responder`, `responder→initiator`). A ciphertext of the wrong length is
/// refused; a well-formed but forged one implicit-rejects into suites that open nothing.
pub fn pq_handshake_finish(
    hs: &PqHandshake,
    ct: &[u8],
) -> Result<(PqSuite, PqSuite), logicaffeine_system::mlkem::KemError> {
    let ss = logicaffeine_system::mlkem::decaps(&hs.dk, ct)?;
    Ok((
        PqSuite::new(derive_aead_key(&ss, b"i2r")),
        PqSuite::new(derive_aead_key(&ss, b"r2i")),
    ))
}

/// Frame a blob under a keyed [`Suite`] instance (the [`PqSuite`] path), binding the suite id in the
/// header exactly like [`seal`]. The instance carries the per-session key the static registry can't.
pub fn seal_with(suite: &dyn Suite, blob: &[u8]) -> Vec<u8> {
    let body = suite.seal_body(blob);
    let mut out = Vec::with_capacity(CHAN_HEADER_LEN + body.len());
    out.push(CHAN_MAGIC);
    out.push(CHAN_VER);
    out.extend_from_slice(&suite.id().to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Reverse [`seal_with`] under a keyed [`Suite`] instance, or `None` on a bad header, a suite-id
/// mismatch, or a body the suite rejects (a tampered AEAD tag).
pub fn open_with(suite: &dyn Suite, bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < CHAN_HEADER_LEN || bytes[0] != CHAN_MAGIC || bytes[1] != CHAN_VER {
        return None;
    }
    if u16::from_le_bytes([bytes[2], bytes[3]]) != suite.id() {
        return None;
    }
    suite.open_body(&bytes[CHAN_HEADER_LEN..])
}

/// Frame an opaque blob under `suite_id`, dispatching the body through the registered
/// [`Suite`]. Panics only if sealed with an unregistered suite — a programmer error, since
/// callers select from the registry.
pub fn seal(suite_id: u16, blob: &[u8]) -> Vec<u8> {
    let suite = suite_for(suite_id).expect("seal with a registered suite");
    let body = suite.seal_body(blob);
    let mut out = Vec::with_capacity(CHAN_HEADER_LEN + body.len());
    out.push(CHAN_MAGIC);
    out.push(CHAN_VER);
    out.extend_from_slice(&suite_id.to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Reverse [`seal`], returning the inner blob. `None` on a too-short header, a bad
/// magic/version, an unknown suite id, or a body the suite rejects — never a panic, never
/// a partial decode (mirrors [`super::marshal::message_from_wire`]).
pub fn open(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < CHAN_HEADER_LEN || bytes[0] != CHAN_MAGIC || bytes[1] != CHAN_VER {
        return None;
    }
    let suite_id = u16::from_le_bytes([bytes[2], bytes[3]]);
    let suite = suite_for(suite_id)?;
    suite.open_body(&bytes[CHAN_HEADER_LEN..])
}

thread_local! {
    /// The suite sealing outbound / opening inbound frames on this thread. `None` (the
    /// default) means the channel is disengaged: [`seal_active`] / [`open_active`] are pure
    /// pass-throughs, so a program that never activates a suite is byte-identical on the wire.
    static ACTIVE_SUITE: std::cell::Cell<Option<u16>> = const { std::cell::Cell::new(None) };
}

/// The suite active for seal/open on this thread, if any.
pub fn active_suite() -> Option<u16> {
    ACTIVE_SUITE.with(|s| s.get())
}

/// Run `f` with `suite` active for seal/open on this thread, restoring the prior suite after
/// (mirrors `marshal`'s scoped wire-knob setters).
pub fn with_suite<T>(suite: Option<u16>, f: impl FnOnce() -> T) -> T {
    let prev = ACTIVE_SUITE.with(|s| s.replace(suite));
    let out = f();
    ACTIVE_SUITE.with(|s| s.set(prev));
    out
}

/// A keyed, stateful crypto session installed for the live send/receive path. Unlike the stateless
/// [`Suite`] registry, a session carries per-direction key/pad material and may **fail closed** on
/// seal — a one-time pad can run out. Both the keyed [`PqSuite`] and the [`super::pnp`] one-time pad
/// plug in here, so [`seal_active_checked`] / [`open_active`] stay suite-agnostic.
pub trait ActiveSession {
    /// Seal outbound bytes, or `None` to fail closed — the caller must then refuse to send, never
    /// transmit the plaintext instead.
    fn seal(&self, bytes: &[u8]) -> Option<Vec<u8>>;
    /// Open inbound bytes, or `None` on a tampered / foreign / replayed frame.
    fn open(&self, bytes: &[u8]) -> Option<Vec<u8>>;
}

thread_local! {
    /// The keyed session sealing outbound / opening inbound frames on this thread, if installed. It
    /// takes precedence over [`ACTIVE_SUITE`]; `None` (the default) leaves the stateless path — and
    /// so the wire — unchanged for programs that never engage a session.
    static ACTIVE_SESSION: std::cell::RefCell<Option<std::rc::Rc<dyn ActiveSession>>> =
        const { std::cell::RefCell::new(None) };
}

/// The keyed session active for seal/open on this thread, if any.
pub fn active_session() -> Option<std::rc::Rc<dyn ActiveSession>> {
    ACTIVE_SESSION.with(|s| s.borrow().clone())
}

/// Install `session` as the active keyed session on this thread, returning the prior one. The live
/// [`seal_active_checked`] / [`open_active`] path then routes through it until it is replaced/cleared.
pub fn install_session(
    session: Option<std::rc::Rc<dyn ActiveSession>>,
) -> Option<std::rc::Rc<dyn ActiveSession>> {
    ACTIVE_SESSION.with(|s| std::mem::replace(&mut *s.borrow_mut(), session))
}

/// Run `f` with `session` active for seal/open on this thread, restoring the prior session after
/// (mirrors [`with_suite`]).
pub fn with_session<T>(
    session: Option<std::rc::Rc<dyn ActiveSession>>,
    f: impl FnOnce() -> T,
) -> T {
    let prev = install_session(session);
    let out = f();
    install_session(prev);
    out
}

/// Seal `bytes` under the active suite; with no active suite, return them unchanged so the
/// wire stays byte-identical for non-secure programs. (Stateless path only — the keyed, possibly
/// fail-closing session path is [`seal_active_checked`].)
pub fn seal_active(bytes: Vec<u8>) -> Vec<u8> {
    match active_suite() {
        Some(id) => seal(id, &bytes),
        None => bytes,
    }
}

/// Seal `bytes` for the live send path: through the active keyed [`ActiveSession`] if installed
/// (which may **fail closed**, returning `None` — e.g. a one-time pad is exhausted), else through the
/// active stateless suite, else pass-through. A `None` result is the fail-closed signal the caller
/// MUST surface as a send error — never transmit the plaintext instead.
pub fn seal_active_checked(bytes: Vec<u8>) -> Option<Vec<u8>> {
    match active_session() {
        Some(session) => session.seal(&bytes),
        None => Some(seal_active(bytes)),
    }
}

/// Open an inbound frame under the active keyed session if installed, else the active suite;
/// `None` on a tampered/foreign frame (the caller drops it). With neither engaged, pass `bytes`
/// through so the wire stays byte-identical for non-secure programs.
pub fn open_active(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if let Some(session) = active_session() {
        return session.open(&bytes);
    }
    match active_suite() {
        Some(_) => open(&bytes),
        None => Some(bytes),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/concurrency/channel/tests.rs"]
mod tests;
