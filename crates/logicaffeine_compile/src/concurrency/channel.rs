//! `SecureChannel` envelope — the end-to-end crypto seam that wraps the opaque
//! wire blob `marshal` produces, between the codec and the transport.
//!
//! Layering (work/QUANTUM_MAP.md §1, §L): the channel owns its *own* versioned envelope
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
/// cryptography lives behind this trait (work/QUANTUM_MAP.md §3 — crypto-agility).
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

/// The post-quantum suite: a per-session ChaCha20-Poly1305 key (from an ML-KEM-768 handshake,
/// see [`pq_handshake_initiator`] / [`pq_handshake_responder`]). Each `seal_body` draws a fresh
/// counter nonce, so every frame is unique; the nonce is carried in the body, so the peer opens
/// statelessly. The suite id is bound as the AEAD associated data, so a frame can't be replayed
/// under a different suite. (Bidirectional use derives a key per direction — see the handshake.)
pub struct PqSuite {
    key: [u8; 32],
    seq: std::sync::atomic::AtomicU64,
}

impl PqSuite {
    /// A PQ suite over an already-derived 32-byte AEAD key.
    pub fn new(key: [u8; 32]) -> Self {
        Self { key, seq: std::sync::atomic::AtomicU64::new(0) }
    }
}

impl Suite for PqSuite {
    fn id(&self) -> u16 {
        SUITE_PQ
    }
    fn seal_body(&self, blob: &[u8]) -> Vec<u8> {
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut nonce = [0u8; 12];
        nonce[..8].copy_from_slice(&seq.to_le_bytes());
        let sealed =
            logicaffeine_system::aead::chacha20poly1305_seal(&self.key, &nonce, &SUITE_PQ.to_le_bytes(), blob);
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
        logicaffeine_system::aead::chacha20poly1305_open(&self.key, &nonce, &SUITE_PQ.to_le_bytes(), &body[12..])
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
/// two directional suites (`initiator→responder`, `responder→initiator`).
pub fn pq_handshake_responder(ek: &[u8], msg: &[u8; 32]) -> (Vec<u8>, PqSuite, PqSuite) {
    let (ct, ss) = logicaffeine_system::mlkem::encaps(ek, msg);
    (ct, PqSuite::new(derive_aead_key(&ss, b"i2r")), PqSuite::new(derive_aead_key(&ss, b"r2i")))
}

/// Initiator step 2: decapsulate the responder's ciphertext, yielding the matching directional
/// suites (`initiator→responder`, `responder→initiator`).
pub fn pq_handshake_finish(hs: &PqHandshake, ct: &[u8]) -> (PqSuite, PqSuite) {
    let ss = logicaffeine_system::mlkem::decaps(&hs.dk, ct);
    (PqSuite::new(derive_aead_key(&ss, b"i2r")), PqSuite::new(derive_aead_key(&ss, b"r2i")))
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
pub fn with_session<T>(session: Option<std::rc::Rc<dyn ActiveSession>>, f: impl FnOnce() -> T) -> T {
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
mod tests {
    use super::*;
    use super::super::marshal::{message_to_wire_with, WireCodec, WireIntegrity};
    use crate::interpreter::RuntimeValue;

    #[test]
    fn null_envelope_round_trips_and_rejects_malformed() {
        let blob = message_to_wire_with("alice", &RuntimeValue::Int(7), WireCodec::Native, WireIntegrity::Checked)
            .expect("encode");

        let sealed = seal(SUITE_NULL, &blob);

        // Round-trip: the null envelope returns the exact blob.
        assert_eq!(open(&sealed).as_deref(), Some(blob.as_slice()), "null suite round-trip");

        // Truncated header → None.
        assert!(open(&sealed[..2]).is_none(), "truncated envelope rejected");

        // Unknown suite id (0xFFFF) → None.
        let mut bad = sealed.clone();
        bad[2] = 0xFF;
        bad[3] = 0xFF;
        assert!(open(&bad).is_none(), "unknown suite rejected");
    }

    /// A small deterministic PRNG so the fuzz lock is reproducible (house style).
    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[test]
    fn null_round_trips_arbitrary_blobs() {
        let mut s = 0x1234_5678_9ABC_DEF0u64;
        for _ in 0..2000 {
            let len = (splitmix64(&mut s) % 257) as usize; // 0..=256, incl. empty
            let blob: Vec<u8> = (0..len).map(|_| splitmix64(&mut s) as u8).collect();
            let sealed = seal(SUITE_NULL, &blob);
            assert_eq!(open(&sealed), Some(blob), "null suite round-trip, len {len}");
        }
    }

    #[test]
    fn rejects_bad_magic_version_and_empty() {
        let sealed = seal(SUITE_NULL, b"payload");
        assert!(open(&[]).is_none(), "empty input rejected");
        assert!(open(&sealed[..CHAN_HEADER_LEN - 1]).is_none(), "header-short rejected");

        let mut bad_magic = sealed.clone();
        bad_magic[0] ^= 0xFF;
        assert!(open(&bad_magic).is_none(), "bad magic rejected");

        let mut bad_ver = sealed.clone();
        bad_ver[1] = bad_ver[1].wrapping_add(1);
        assert!(open(&bad_ver).is_none(), "bad version rejected");
    }

    #[test]
    fn seal_is_framed_not_raw() {
        let blob = b"the relay sees only the envelope";
        let sealed = seal(SUITE_NULL, blob);
        assert_eq!(&sealed[..2], &[CHAN_MAGIC, CHAN_VER], "header magic+version present");
        assert_eq!(&sealed[2..4], &SUITE_NULL.to_le_bytes(), "suite id bound little-endian");
        assert_eq!(sealed.len(), CHAN_HEADER_LEN + blob.len(), "null body length-exact");
        assert_ne!(sealed.as_slice(), blob.as_slice(), "sealed bytes differ from the raw blob");
    }

    #[test]
    fn null_suite_trait_dispatch() {
        let suite = suite_for(SUITE_NULL).expect("null suite registered");
        assert_eq!(suite.id(), SUITE_NULL);
        let body = suite.seal_body(b"abc");
        assert_eq!(suite.open_body(&body).as_deref(), Some(&b"abc"[..]), "identity body");
        assert!(suite_for(0xFFFF).is_none(), "unknown suite id is not registered");
    }

    #[test]
    fn no_active_suite_is_passthrough() {
        assert_eq!(active_suite(), None, "default: channel disengaged");
        let blob = b"plain".to_vec();
        assert_eq!(seal_active(blob.clone()), blob, "seal is identity when off");
        assert_eq!(open_active(blob.clone()), Some(blob), "open is identity when off");
    }

    #[test]
    fn with_suite_scopes_and_restores() {
        assert_eq!(active_suite(), None);
        with_suite(Some(SUITE_NULL), || {
            assert_eq!(active_suite(), Some(SUITE_NULL), "suite active inside scope");
            let blob = b"secret payload".to_vec();
            let sealed = seal_active(blob.clone());
            assert_ne!(sealed, blob, "sealed under an active suite is framed");
            assert_eq!(open_active(sealed), Some(blob), "round-trip under the active suite");
        });
        assert_eq!(active_suite(), None, "prior suite restored after scope");
    }

    #[test]
    fn open_active_drops_foreign_frame_under_active_suite() {
        with_suite(Some(SUITE_NULL), || {
            assert!(open_active(b"not an envelope".to_vec()).is_none(), "foreign frame dropped");
        });
    }

    #[test]
    fn pq_channel_seals_a_real_wire_message_end_to_end() {
        // Full post-quantum handshake + AEAD seal over an actual marshalled wire blob.
        // Initiator generates an ML-KEM-768 keypair and publishes ek; responder encapsulates to it
        // and returns the ciphertext; both derive the same directional ChaCha20-Poly1305 suites.
        let hs = pq_handshake_initiator(&[0xA1; 32], &[0xA2; 32]);
        let (ct, resp_i2r, resp_r2i) = pq_handshake_responder(&hs.ek, &[0xB0; 32]);
        let (init_i2r, init_r2i) = pq_handshake_finish(&hs, &ct);

        // The actual payload is a real codec-encoded wire blob, exactly what the transport carries.
        let blob = message_to_wire_with(
            "responder",
            &RuntimeValue::Int(42),
            WireCodec::Native,
            WireIntegrity::Checked,
        )
        .expect("encode");

        // Responder→initiator: responder seals, initiator opens. End-to-end post-quantum secrecy.
        let sealed = seal_with(&resp_r2i, &blob);
        assert_eq!(&sealed[..2], &[CHAN_MAGIC, CHAN_VER], "channel header present");
        assert_eq!(&sealed[2..4], &SUITE_PQ.to_le_bytes(), "PQ suite id bound in the envelope");
        assert_ne!(sealed[CHAN_HEADER_LEN..].to_vec(), blob, "body is ciphertext, not the raw blob");
        assert_eq!(
            open_with(&init_r2i, &sealed).as_deref(),
            Some(blob.as_slice()),
            "initiator opens the responder's PQ-sealed wire message"
        );

        // Initiator→responder direction works independently (distinct key).
        let s2 = seal_with(&init_i2r, b"ack");
        assert_eq!(open_with(&resp_i2r, &s2).as_deref(), Some(&b"ack"[..]), "i2r direction opens");

        // The counter nonce makes every seal unique, and each still opens.
        let sealed_again = seal_with(&resp_r2i, &blob);
        assert_ne!(sealed, sealed_again, "fresh counter nonce ⇒ a distinct frame each time");
        assert_eq!(open_with(&init_r2i, &sealed_again).as_deref(), Some(blob.as_slice()));

        // A tampered tag is rejected; a foreign session key (failed handshake) cannot open.
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(open_with(&init_r2i, &tampered).is_none(), "tampered PQ frame rejected");
        let eve = PqSuite::new(derive_aead_key(&[0u8; 32], b"r2i"));
        assert!(open_with(&eve, &sealed).is_none(), "wrong session key cannot open");
    }

    #[test]
    fn pq_handshake_disagrees_on_a_corrupted_ciphertext() {
        // If the KEM ciphertext is corrupted in flight, ML-KEM's implicit reject makes the two
        // sides derive DIFFERENT secrets, so the initiator cannot open the responder's frames.
        let hs = pq_handshake_initiator(&[1; 32], &[2; 32]);
        let (ct, _resp_i2r, resp_r2i) = pq_handshake_responder(&hs.ek, &[3; 32]);
        let mut bad_ct = ct.clone();
        bad_ct[0] ^= 1;
        let (_init_i2r, init_r2i) = pq_handshake_finish(&hs, &bad_ct);
        let sealed = seal_with(&resp_r2i, b"top secret");
        assert!(
            open_with(&init_r2i, &sealed).is_none(),
            "a corrupted handshake yields divergent keys ⇒ frames don't open"
        );
    }
}
