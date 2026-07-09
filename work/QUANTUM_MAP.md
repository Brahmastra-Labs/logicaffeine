# QUANTUM_MAP.md — Crypto-Agile, Fully-Post-Quantum, Substrate-Independent Messaging for LOGOS

This is the build map for LOGOS networking: a **crypto-agile** end-to-end messaging substrate that
dials from a plaintext-fast suite up to a **fully post-quantum, self-healing (Triple-Ratchet) suite**,
carried over **any** transport — QUIC, WebTransport, iroh, libp2p, Bluetooth, LoRa — and running
**byte-identically native and in a browser tab**, with **deterministic replay**.

The governing principle is **best of all worlds, nothing cut.** Crypto-agility means we never choose
one primitive and discard the rest. Every primitive, suite, and transport is a first-class, selectable
option; this document explains each option, its tradeoffs, and the recommended default. You dial in the
posture per context — fast/classical when you want it, fully post-quantum + self-healing when you want
it — without ever giving an option up.

> **Status:** design map (no code yet). Every cryptographic claim cites a primary source; every build
> phase names a falsifiable test. Sources are collected at the end.

## Contents
0. [Thesis](#0-thesis) · 1. [The one move](#1-the-one-move) · 2. [Audit](#2-audit-every-quantum-weak-touchpoint) ·
3. [Crypto-agility](#3-crypto-agility-the-spine) · 4. [Options & choices (A–L)](#4-options--choices-the-decision-records) ·
5. [The two layers](#5-the-two-crypto-layers) · 6. [Forward secrecy](#6-forward-secrecy-the-post-quantum-triple-ratchet) ·
7. [Identity & addressing](#7-identity--addressing) · 8. [Symmetric/hash hygiene + libraries](#8-symmetrichash-hygiene--library-strategy) ·
9. [Transports](#9-transports--first-class-and-beyond-libp2p) · 10. [SOTA bar](#10-the-sota-bar--how-we-know-and-prove-it) ·
11. [Roadmap](#11-roadmap) · 12. [Honest limits](#12-honest-limits--risks) · 13. [Critical files](#13-critical-files)

---

## 0. Thesis

> **The first and only *language* whose built-in messaging is crypto-agile from a plaintext-fast suite
> up to a fully post-quantum Triple-Ratchet suite (PQ3 / SPQR-grade), carried over a substrate-independent
> mesh (QUIC / WebTransport / iroh / libp2p / Bluetooth / LoRa), running byte-identically native and
> in-browser, with deterministic, replayable execution.**

Every primitive we adopt is already the deployed state of the art — we invent no crypto. The novelty is
the *combination*, on axes no existing system holds at once (§10). "Fully post-quantum" is a true,
selectable maximum: with a `PQ`/`PQ-Max` suite there is **no classical primitive in the confidentiality
or authentication path**, the parameters meet **CNSA 2.0**, and forward secrecy is continuous, not one-shot.

---

## 1. The one move

Two asks — "connect and speak however it can, mesh it up" and "layer in quantum-safe transmission" — are
one architectural decision: **put encryption at the message layer (end-to-end) and put a thin `Transport`
trait under it.**

- The wire codec (`crates/logicaffeine_compile/src/concurrency/marshal.rs`, `message_to_wire` /
  `message_from_wire`) is **already** the transport-agnostic payload — a self-framed, checksummed,
  optionally compressed, SIMD-accelerated `Vec<u8>`. The relay's `RelayFrame::Publish { topic, data }`
  already proves transports never inspect payloads.
- A `SecureChannel` placed **between `marshal` and `Net`** seals those blobs. Because it is end-to-end, it
  rides over **any** transport — including a plaintext relay or a LoRa radio. "Encrypted" and "quantum-safe"
  reduce to *which suite the channel negotiates*; the transport never changes.
- Identity = **hash of a public key**. The same `DestAddr` is reachable over every interface; it is the
  cryptographic identity the channel binds to. This is the shared spine of both halves.

```
 Language:  Listen / Connect / Send / Sync   (+ optional `over <transport>`, `securely`, suite selector)
        │
   Net handle (system/src/net.rs)        — generalized: identity-addressed, transport-agnostic
        │
   SecureChannel  [NEW]                   — crypto-agile E2E: negotiate suite, seal/open, ratchet
        │   (new pure, WASM-safe `logicaffeine_crypto` crate)
   Wire codec (marshal.rs)               — the opaque payload format (already exists, UNCHANGED)
        │
   Transport trait  [NEW]                — interfaces: WS, QUIC, WebTransport, iroh, libp2p, BLE, LoRa
        │
   Router / mesh  [NEW]                  — announce path-discovery + store-and-forward (ours, not gossipsub-bound)
```

Two invariants are load-bearing and never violated: **(1)** `drain()` stays non-blocking / pull-based (the
deterministic sync-point model — `Net::drain()` already uses `try_recv`); **(2)** transports are dumb
byte-movers — framing, encryption, fragmentation, and routing all live above them.

---

## 2. Audit — every quantum-weak touchpoint

A full read of the workspace. Today **all** security-critical crypto is classical, and there is **no E2E
encryption at all** (the relay is plaintext); zero post-quantum crates are present anywhere.

| Where | Primitive(s) | Security-critical? | Quantum status | Action |
|---|---|---|---|---|
| libp2p Noise (TCP) — `network/mesh.rs:275` | X25519 + SHA-256 + ChaCha20-Poly1305 | yes (hop confidentiality) | **vulnerable** — Shor breaks X25519 | PQ transport handshake, or rely on E2E for content |
| libp2p QUIC — `mesh.rs:279` | rustls: P-256 / X25519 KEX + AES-256-GCM | yes (hop confidentiality) | **vulnerable** — Shor breaks the KEX | swap to rustls PQ KEX (`X25519MLKEM768`) |
| gossipsub signing — `behaviour.rs:83` | Ed25519 (ephemeral PeerId) | yes (hop metadata auth) | **vulnerable** — Shor | PQ / hybrid identity signature |
| **E2E content** — `interpreter.rs:1874 / 2400` | **none today — plaintext over the relay** | **yes — the actual prize** | **plaintext** | **`SecureChannel`: this is where the suite lives** |
| wire header — `marshal.rs:294-298` | FNV-1a (bits 0-4 used, **5-7 reserved**) | no (corruption only) | n/a | keep; the channel owns its **own** envelope |
| proposed `Dest = trunc(SHA-256)` | truncated hash | yes (address forgery) | **weak if truncated to 128-bit → ~64-bit under quantum BHT** | SHAKE256, **≥256-bit** output |
| symmetric AEAD | ChaCha20-Poly1305 / AES-**256**-GCM | yes | **PQ-fine at 256-bit** (Grover → 128) | reuse; **ban AES-128** |
| `sipping.rs` SHA-256 (file-chunk integrity), `marshal` FNV, `data` FxHash, `random.rs` rand_chacha | — | no (integrity / hashtable / non-crypto RNG) | n/a | unchanged |

**Conclusion:** the prize is the missing E2E layer. Everything classical is either *below* it (transport
hop crypto — redundant for content once E2E is PQ) or *replaceable* by the suite. Clean slate for PQ.

---

## 3. Crypto-agility — the spine

The `SecureChannel` wraps the opaque wire blob in its **own versioned envelope** that carries a **suite id**
from a registry:

```
[ magic+version | suite_id | handshake-or-sequence | AEAD ciphertext + tag ]
```

(We do **not** steal `marshal.rs` header bits 5-7 — clean layering wins; those bits remain a noted fallback.)

Four named profiles on the computational ladder, plus one information-theoretic break-glass tier —
**all shipped, all selectable**:

| Suite | KEM (key agreement) | Signature (identity / auth) | AEAD | Posture |
|---|---|---|---|---|
| `Classic` | X25519 | Ed25519 | ChaCha20-Poly1305 | fast / max compat — *explicitly not PQ* |
| `Hybrid` *(recommended default)* | **X-Wing** (X25519 + ML-KEM-768) | Ed25519 **+** ML-DSA-65 | ChaCha20-Poly1305 | defense-in-depth — safe if *either* half holds |
| `PQ` | **ML-KEM-768** (pure) | **ML-DSA-65** (pure) | ChaCha20-Poly1305 / AES-256-GCM | **fully post-quantum — no classical in path** |
| `PQ-Max` | **ML-KEM-1024** (CNSA 2.0) | **ML-DSA-87** + **SLH-DSA** root of trust | AES-256-GCM | paranoid / CNSA-2.0-grade |
| `PNP` *(break-glass)* | pre-shared true-random pad — *no KEM* | one-time Wegman–Carter (Poly1305) | **XOR one-time pad** | **information-theoretic — secrecy by Shannon, no hardness assumption; survives `P = NP`** |

**The `PNP` tier sits outside the negotiated ladder.** It is not a suite an attacker can downgrade *to* or
*from*: it carries no key agreement and no algorithm choice, only a pad provisioned out of band. It is the one
tier whose secrecy is *unconditional* — a Vernam one-time pad is perfectly secret by Shannon's theorem, so it
holds even if every computational assumption above it (lattice, factoring, discrete-log — and with them every
other suite) falls. The price is Shannon's price: one truly-random pad byte per plaintext byte, consumed once.
See Decision Record M and `crates/logicaffeine_compile/src/concurrency/pnp.rs`.

**Downgrade-attack resistance** (the property that makes agility *correct*, not a hole). An active
attacker must not be able to force a weaker suite than both peers permit:
1. The full *offered* and *selected* suite lists are bound into the handshake transcript and signed by the
   long-term identity (so tampering changes the transcript hash and breaks the signature).
2. Each identity advertises a **signed minimum-suite floor**.
3. A peer policy may refuse anything below a floor (e.g. "never accept `Classic`," "require `PQ` or above").

This is the formal correctness backbone of the whole system: agility without authenticated negotiation is a
downgrade vulnerability; agility *with* it is strictly more secure than any fixed suite, because a broken
primitive becomes a policy-rejected suite rather than a wire break.

---

## 4. Options & Choices — the decision records

For every fork: the **options**, the **tradeoffs**, the **default**, and the standing rule that *all stay
selectable*. Sizes are listed because "fully explain the options" means showing what each costs on the wire.

### A. Key agreement (KEM)
| Option | Sizes (pk / ct / ss) | Property |
|---|---|---|
| `X25519` | 32 / 32 / 32 B | classical, tiny, fast — **Shor-broken** |
| `ML-KEM-768` | 1184 / 1088 / 32 B | PQ, NIST **Cat 3** (FIPS 203) |
| `ML-KEM-1024` | 1568 / 1568 / 32 B | PQ, **Cat 5 / CNSA 2.0** |
| **`X-Wing`** | 1216 / 1120 / 32 B | hybrid X25519+ML-KEM-768, SHA3-256 combiner, IETF-standardized, **"secure if either holds"** |
| `HQC` | large | **code-based** backup KEM (different math family) — agility hedge if lattices fall |

**Default:** `Hybrid` = X-Wing; `PQ` = ML-KEM-768; `PQ-Max` = ML-KEM-1024. HQC reserved as a registry
escape hatch — if lattice cryptanalysis advances, we add an `HQC` suite without touching the wire format.
*Tradeoff:* pure-PQ is unconditionally quantum-safe but younger and ~1 KB larger per handshake; X-Wing pays
~30 B + one extra scalar-mult for a classical safety net; classical is tiny but broken.

### B. Signature / identity — *the best-of-all-worlds pick*
| Option | Sig size | Assumption | Use |
|---|---|---|---|
| `Ed25519` | 64 B | classical ECC | `Classic` only |
| `ML-DSA-65 / -87` | 3309 / 4627 B | **lattice** (fast) | **per-message / announce signing** |
| `SLH-DSA` | 7–50 KB | **hash only** — most conservative, outlives any lattice break | **long-term root of trust** |
| `FN-DSA` (Falcon) | ~1 KB | lattice, compact | parked — float sampling is side-channel-treacherous, FIPS 206 not final |

**Pick:** ML-DSA for the hot path (fast, signed constantly); **SLH-DSA for the root identity** (signed
rarely, where a catastrophic compromise must be hardest). A hash-based root + a lattice workhorse cuts no
corner on *either* speed or assumption-robustness — exactly the "best of all worlds" the design demands.

### C. Hybrid combiner
Options: roll-our-own concat-KDF · IETF generic KEM combiner · **X-Wing** (specific, optimized,
security-proven). **Pick: X-Wing.** Standing rule: we never invent crypto constructions; we compose
standardized, proven ones.

### D. AEAD (symmetric bulk)
| Option | Property |
|---|---|
| **`ChaCha20-Poly1305`** | 256-bit, constant-time in software, no AES-NI dependency — ideal for WASM |
| `AES-256-GCM` | hardware-fast on native, FIPS / CNSA-preferred |
| `XChaCha20-Poly1305` | 192-bit nonce → trivially safe nonce management |

All 256-bit, which is **PQ-fine** (Grover only halves symmetric strength: 256 → 128, per NIST/CNSA — we do
**not** overstate AEAD-tag weakness). **Ban AES-128.** **Default ChaCha20-Poly1305** (WASM + our
deterministic sequence-nonce); AES-256-GCM in `PQ-Max` (CNSA). Both selectable.

### E. KDF / hashing
Options: HKDF-SHA-256 · HKDF-SHA-512 · SHA-3 / SHAKE256 · BLAKE3. **Pick:** SHA3-256 for the X-Wing combiner
(spec-mandated); HKDF-SHA-512 for our own transcript / session KDF (margin); **SHAKE256 XOF for `DestAddr`
at ≥256-bit** (see K).

### F. Forward-secrecy depth (the corner the old map cut)
Apple's level framing: **L0** none · **L1** classical E2E · **L2** PQ key-establishment one-shot (PQXDH-like)
· **L3** PQ key-establishment **+ continuous PQ ratchet** (PQ3 / SPQR — self-healing post-compromise).
**Target: L3.** We ship L2 first (`PQ` one-shot) as a green intermediate, then the ratchet. We do **not**
stop at session-granularity FS — see §6.

### G. Handshake pattern
Options: **PQXDH-style** (Signal — async signed prekey bundles, fits our publish-to-inbox / await + store-and-forward
model) · PQ-Noise (interactive, fits live QUIC streams) · TLS-style. **Pick:** PQXDH-style primary; PQ-Noise
available for live stream transports.

### H. PQ library
| Option | Property |
|---|---|
| **RustCrypto** `ml-kem` / `ml-dsa` / `slh-dsa` | pure-Rust, compiles to wasm32, no C — **the WASM mandate**; but **unaudited**, and `ml-dsa` shipped timing CVE **GHSA-hcp2-x6j4-29j7 (2026-01-10)** |
| **libcrux / Cryspen** | **formally-verified** ML-KEM — higher assurance |
| `aws-lc-rs` | native; drives rustls PQ KEX |
| `liboqs` / `oqs` | broad C coverage, but no clean WASM |

**Pick:** RustCrypto baseline (browser-capable); **libcrux verified ML-KEM** where assurance matters;
aws-lc-rs on native via rustls for the transport KEX. *Agility is the safety net:* because suites are
negotiated and versioned, a broken or patched implementation is a **suite/version bump, not a wire break** —
this is precisely what lets us run pure-PQ with no classical fallback responsibly.

### I. Transport substrate
See §9 — the user's explicit "beyond libp2p" ask. All pluggable behind the `Transport` trait.

### J. Mesh / routing backend
Options: libp2p gossipsub (mature, but ties us to libp2p) · **our own Reticulum-style router** reusing the
`logicaffeine_data` CRDT seen-set (substrate-independent) · DHT. **Pick:** our own router as the spine;
libp2p gossipsub kept as one optional backend.

### K. Address width
Options: 128-bit truncation (Reticulum — only ~64-bit quantum collision resistance via BHT → forgery hazard)
· **≥256-bit SHAKE256** (≥128-bit quantum collision resistance) · full hash. **Pick: ≥256-bit.** We refuse
to inherit Reticulum's truncation weakness.

### L. Crypto envelope placement
Options: steal `marshal.rs` header bits 5-7 (couples layers) · **SecureChannel owns its own versioned
envelope** wrapping the opaque blob (clean layering). **Pick:** own envelope.

### M. Information-theoretic last resort — the `P = NP` tier
Options: nothing beyond the computational ladder (A–D) · **a true Vernam one-time pad tier** drawn from a
pre-shared pool. **Pick: ship it as `PNP`, break-glass.** Rationale and the rules that make it correct:
- **Why it exists.** Every suite A–D rests on a hardness assumption; `PNP` rests on Shannon. Perfect secrecy is
  unconditional against an unbounded adversary — the *only* thing that still holds if `P = NP` collapses the rest.
- **No shrinking the pad.** Perfect secrecy requires key ≥ message, truly random, used once (Shannon). Growing a
  small pad from a seed is a PRG = stream cipher = computational, and so it *would* fall with `P = NP`, defeating
  the purpose. The pad is therefore real, pre-distributed, and **the runtime does zero randomness** — it only
  *consumes* the pad. A real pad is incompressible (`K(pad) ≈ |pad|`), so the pool is quality-gated by the
  `logicaffeine_proof::ait` classifier and a compressible ("fake random") pool is refused (`PadError::Compressible`).
- **Rotation = a synchronized cursor.** Each message consumes a fresh, never-reused segment (a "cover") at one pad
  byte per plaintext byte, plus 32 pad bytes for the MAC key. The cover's offset rides the frame, so a
  lossy/reordered transport resyncs and a replayed offset is refused; the send cursor is fsync-committed *before* a
  frame is emitted, so a crash can only waste pad, never reuse it — a two-time pad is catastrophic.
- **Integrity is information-theoretic too.** XOR is malleable, so each cover carries a one-time Wegman–Carter MAC
  (Poly1305 keyed by fresh pad bytes): unconditional authentication, not computational.
- **Directional split + fail-closed.** The pool splits into `i2r` / `r2i` halves so the two directions never draw
  overlapping pad; when a half is spent, sealing refuses — no fallback to a weaker primitive.
- **Wired (shipped).** Surface: `Connect to <addr> with pad "<path>" as initiator` / `Listen on <addr> with pad
  "<path>" as responder`. The interpreter (the real-relay-networking tier) reads the pad, quality-gates it, and
  installs a directional `PnpSession` on the channel's `ActiveSession` seam, so every subsequent `Send`/receive is
  one-time-pad sealed and **fail-closed** (`seal_active_checked` → send error on exhaustion / unreadable pad, never
  plaintext). Non-interpreter tiers (bytecode VM, native-AOT transpile) **fail loud** on a `with pad` clause rather
  than silently drop the pad. Files: `concurrency/pnp.rs`, `concurrency/channel.rs`, `interpreter.rs`
  (`activate_pnp_session`), `logicaffeine_language` AST `SecurePad`/`SecureRole` + parser `parse_secure_clause`.

---

## 5. The two crypto layers

The old map blurred these; perfection separates them precisely.

- **(A) E2E content channel** — the `SecureChannel`; the suite lives here. With a `PQ` / `PQ-Max` suite the
  **message content is fully post-quantum over any transport**, including a plaintext relay or a LoRa link.
  This is unconditional: the transport cannot weaken content confidentiality, because it only sees ciphertext.
- **(B) Transport-hop handshake** — optional PQ for *metadata* (which topics, which peers, timing): rustls PQ
  KEX (`X25519MLKEM768`) on QUIC, or PQ-Noise. This protects the link between two hops, not the content.

**Honest statement:** E2E `PQ` protects *content* unconditionally; PQ-protecting *hop metadata* additionally
requires a PQ transport (§9). Relay/mesh routing metadata is a documented limit (§12).

---

## 6. Forward secrecy — the post-quantum Triple Ratchet

This is the centerpiece, and the corner the previous map cut. A one-shot handshake protects the session key
once; if that key is later compromised, all messages fall. The state of the art — **Apple PQ3** and **Signal
SPQR (Sparse Post-Quantum Ratchet, the "Triple Ratchet")** — instead **ratchets a post-quantum KEM
continuously**, so the channel *self-heals*: a compromise at time *t* does not expose messages before *t*
(forward secrecy) and the channel recovers secrecy after *t* (post-compromise security).

**Our design:** initial PQ key establishment (X-Wing or pure ML-KEM, per suite) establishes the root; then a
**sparse ML-KEM ratchet** performs a fresh encapsulation every *N* messages (sparse to amortize ML-KEM's ~1 KB
ciphertext cost), mixing the new shared secret into the chain through HKDF. In `Hybrid`, this runs *alongside*
a classical DH ratchet and both feed the KDF (the SPQR "triple" construction); in `PQ` / `PQ-Max`, the PQ
ratchet stands alone. Per-message symmetric keys advance every message; the KEM ratchet advances sparsely.

This is what moves us from "PQ handshake" to **L3 / global-SOTA**. It is phased (a working `PQ` one-shot lands
first), but it is a **named, committed goal — not a footnote.**

---

## 7. Identity & addressing

Today identity is an *ephemeral* Ed25519 PeerId (new per run). We replace it with a **persistent hybrid/PQ
identity keypair** as the stable root, and a self-certifying address:

```
DestAddr = SHAKE256( "logos-dest-v1" ‖ identity_public_keys )   truncated to ≥256 bits
```

- **≥256-bit** so quantum (BHT) collision resistance is ≥128-bit — explicitly **wider than Reticulum's
  128-bit truncation**, whose ~64-bit quantum collision resistance is an address-forgery hazard we refuse to
  inherit (§4-K).
- **Self-certifying:** anyone can verify a claimed identity hashes to its address; the same `DestAddr` is
  reachable over relay, QUIC, libp2p, Bluetooth, or LoRa — the cryptographic name is transport-independent.
- **Storage:** native = key file via the `persistence` VFS (`crates/.../system/src/storage/`, 0600); browser
  = the existing OPFS / IndexedDB VFS. First-run generation via `getrandom` (js feature on wasm).
- **Migration:** `addr.rs::canonical_topic` is extended to resolve a name/key to a dest-hash topic while still
  normalizing legacy `ws://` / multiaddr strings — both remain valid topic strings, so the relay, `RelayFrame`,
  and the merge path are untouched. Additive; existing programs are unaffected.

---

## 8. Symmetric/hash hygiene + library strategy

- **Symmetric is already PQ.** Mandate 256-bit AEAD (ChaCha20-Poly1305 or AES-256-GCM). Grover only halves
  symmetric strength (256 → 128), which NIST and CNSA accept as PQ-secure. **Ban AES-128.** We state this
  accurately and do *not* claim catastrophic AEAD-tag weakness.
- **Hashing:** SHA-3 / SHAKE256 for the combiner, transcript, and addresses (collision margin); HKDF for key
  derivation. Non-crypto hashes (FNV in `marshal`, FxHash in `data`, CRC) are untouched — they are corruption /
  hashtable utilities, not security primitives.
- **Crate:** a new **`logicaffeine_crypto`** crate — pure, no-IO, WASM-safe, mirroring the `data`-vs-`system`
  split. It keeps heavy PQ dependencies out of the libp2p/tokio graph and is unit-testable with NIST KATs in
  isolation. The *IO* parts (key storage) stay in `logicaffeine_system` behind `persistence`.

```toml
# logicaffeine_crypto/Cargo.toml — default builds stay crypto-free
[features]
default   = []
classical = ["dep:chacha20poly1305","dep:hkdf","dep:sha3","dep:x25519-dalek","dep:ed25519-dalek","dep:rand_core"]
pq        = ["classical","dep:ml-kem","dep:ml-dsa","dep:slh-dsa","dep:x-wing"]   # adds the PQ + hybrid suites
verified  = ["pq","dep:libcrux-ml-kem"]                                          # swap in formally-verified ML-KEM

# logicaffeine_system/Cargo.toml — opt-in, default []
secure-channel = ["dep:logicaffeine-crypto","logicaffeine-crypto/classical","relay"]
quantum-safe   = ["secure-channel","logicaffeine-crypto/pq"]
```

- **Library posture (honest):** RustCrypto `ml-kem`/`ml-dsa`/`slh-dsa` are pure-Rust and NIST-KAT-correct but
  **unaudited**, and `ml-dsa` disclosed a timing side-channel (GHSA-hcp2-x6j4-29j7) on 2026-01-10. We label
  pure-PQ "experimental until audited," default to `Hybrid`, offer libcrux verified ML-KEM via the `verified`
  feature, and rely on agility to swap/patch without a wire break.

---

## 9. Transports — first-class, and beyond libp2p

The `Transport` trait makes us **substrate-independent.** Because the E2E channel carries confidentiality, a
transport may have *no* crypto (LoRa), *classical* crypto (libp2p Noise), or *PQ* crypto (rustls-PQ QUIC) —
an E2E `PQ` suite holds regardless. **libp2p is demoted from "the foundation" to one optional impl among many.**

```rust
pub type Frame = Vec<u8>;
pub struct Capabilities { mtu: Option<usize>, reliable: bool, ordered: bool, kind: LinkKind, broadcast: bool }

pub trait Transport {                     // #[async_trait]; defined twice under #[cfg] (Send native / !Send wasm)
    fn capabilities(&self) -> Capabilities;
    async fn connect(&mut self, addr: &str) -> Result<(), String>;
    async fn listen(&mut self, addr: &str) -> Result<(), String>;   // Err(Unsupported) for rx-only links
    fn send(&self, dest: &Dest, frame: Frame) -> Result<(), String>;
    async fn subscribe(&mut self, dest: &Dest) -> Result<(), String>;
    fn drain(&mut self) -> Vec<(Dest, Frame)>;                       // NON-BLOCKING — the load-bearing contract
}
```

| Transport | Crate / API | WASM? | Transport crypto | Role |
|---|---|---|---|---|
| WebSocket relay (exists) | tokio-tungstenite / web-sys | both | none (wss via proxy) | browser + simplest path |
| Loopback / in-process | std channels | both | n/a | deterministic tests; build first |
| **QUIC** *(recommended internet)* | **quinn + rustls** | native (+ WebTransport in browser) | **rustls PQ KEX (`X25519MLKEM768`)** — *we* control it | the substrate libp2p hides from us |
| **WebTransport / WebRTC** | web-sys / webtransport | browser | DTLS / QUIC | browser-native P2P + datagrams |
| **iroh** | iroh | native | QUIC + managed NAT hole-punching | turnkey P2P without the libp2p weight |
| libp2p mesh (exists) | libp2p | native | Noise / QUIC (classical) | *one* option; reuse `gossip.rs` |
| Bluetooth LE | btleplug / Web Bluetooth | both\* | link-layer | radio mesh, small MTU |
| LoRa / serial | serialport | native | **none → E2E mandatory** | long-range radio mesh |

\* Web Bluetooth is experimental/gated; native BLE is the primary path.

**Routing/mesh is ours:** a Reticulum-style router does announce-based path discovery + store-and-forward,
reusing the `logicaffeine_data` CRDT seen-set for deterministic dedup — *not* bound to libp2p gossipsub. So a
single mesh composes across any mix of the above: a message can hop relay → QUIC → Bluetooth → LoRa.
Per-transport `Capabilities` (MTU, reliability, ordering, datagram-vs-stream) drive router-level fragmentation
and ACK/retry for small-MTU lossy links.

**Language surface (additive):** default is auto-mesh (try whatever can carry it); an optional `over <transport>`
hint and a `securely` / suite selector mirror the existing `Send compressed with <codec>` modifier. Per-program
posture via a `## Quantum-Safe` / `## Crypto Suite PQ-Max` decorator (the `## <Decorator>`-before-`## Main`
machinery — a config flag, **not** the optimization `REGISTRY`). Every current program parses unchanged.

**Determinism carve-out:** crypto draws real entropy from `getrandom`, never the seeded `Chooser` (routing
that randomness through the deterministic RNG would both break security and falsely imply reproducibility);
sealed traffic is treated as opaque external input under seeded replay.

---

## 10. The SOTA bar — how we know, and prove, it

We do not assert SOTA; we **measure** it. The honest superlative is narrow and true: every primitive is the
deployed best, and the *combination* is held by no one else.

### Competitor map (who is best on each single axis, and why none combine them)
| System | Any-transport mesh | Agile up to **pure**-PQ | **PQ ratchet** (self-heal) | Browser-tab (same stack) | Deterministic replay | Language-native |
|---|:--:|:--:|:--:|:--:|:--:|:--:|
| **Reticulum (RNS)** | ✅ (the gold standard) | ❌ classical, 128-bit addrs | ❌ | ❌ | ❌ | ❌ |
| **libp2p** | ✅ | ⚠️ nascent | ❌ | ⚠️ different impl (js-libp2p) | ❌ | ❌ |
| **Signal SPQR** | ❌ server-mediated | ⚠️ hybrid (keeps classical) | ✅ (defines the bar) | ⚠️ app | ❌ | ❌ |
| **Apple PQ3** | ❌ server-mediated | ⚠️ hybrid | ✅ (defines the bar) | ❌ | ❌ | ❌ |
| **TLS 1.3 X25519MLKEM768** | ❌ point-to-point | ⚠️ hybrid only | ❌ | ✅ | ❌ | ❌ |
| **CNSA 2.0 stacks** | ❌ | ✅ pure ML-KEM-1024 | ❌ | ❌ | ❌ | ❌ |
| **LOGOS (this map)** | ✅ | ✅ (`PQ` / `PQ-Max`) | ✅ (L3) | ✅ same stack | ✅ | ✅ |

Reading: Signal/Apple own the PQ ratchet but stay hybrid and aren't meshes or language primitives; CNSA stacks
own pure-PQ parameters but aren't ratcheted messengers; Reticulum owns any-transport but is classical and
never runs in a browser. The row filled across every column has one occupant.

### Falsifiable bars (binary pass/fail — when all green, the superlative is the test report)
1. **NIST KATs** — ML-KEM-768/1024, ML-DSA-65/87, SLH-DSA pass the published ACVP/KAT vectors bit-for-bit.
2. **X-Wing combiner** — derived key changes iff *either* the X25519 *or* the ML-KEM secret changes.
3. **Downgrade-attack test** — an active MITM cannot force a suite below either peer's signed floor.
4. **Ratchet self-healing test** — after a simulated key compromise, secrecy recovers within *N* messages
   (post-compromise security), and pre-compromise messages stay sealed (forward secrecy).
5. **Pure-PQ purity** — the `PQ`/`PQ-Max` code path makes **zero** calls into X25519/Ed25519 (grep-asserted in CI).
6. **Transport × suite matrix** — one sealed blob round-trips, byte-identical, over every transport × every suite.
7. **Browser parity** — `wasm-pack test --headless` shows the same seal → transport → open path byte-identical
   native ↔ browser (extends `scripts/test-wasm-relay.sh`).
8. **Determinism** — a full multi-node exchange replays byte-for-byte under `LOGOS_SEED` (crypto-entropy carve-out).
9. **Overhead** — seal/open throughput within a small constant of raw AEAD (extends `marshal.rs`
   `bench_wire_throughput`); per-suite `.wasm` size delta measured and reported (no silent bloat).

### Where we are *not* yet ahead (candor keeps the claim credible)
- **Per-transport maturity:** libp2p has years of hardening per transport; ours start fresh. We win on
  agility-to-pure-PQ × breadth × browser × determinism, not yet on per-transport battle-testing.
- **PQ library audit:** RustCrypto is unaudited (the `ml-dsa` 2026 CVE is the proof) — mitigated by hybrid
  default, agility, the libcrux verified option, and an "experimental" label on pure-PQ.

---

## 11. Roadmap

Interleaved, agility-first; TDD RED → GREEN; each phase independently shippable with the suite green.

| Phase | Goal | The test that proves it |
|---|---|---|
| **0** | `Transport` trait; WS implements it — byte-identical | existing relay tests pass unchanged; `Net`-over-trait `drain()` == old output |
| **1** | Loopback transport + Router skeleton | two loopback `Net`s exchange a `message_to_wire` blob, deterministic, no sockets |
| **2** | `logicaffeine_crypto` crate + `SecureChannel` envelope + suite-id registry (null suite) | frame round-trips; `open`→`None` on truncated/tampered; unknown suite rejected |
| **3** | `Classic` + `Hybrid(X-Wing)` suites + authenticated negotiation w/ **downgrade resistance**, over the relay | seal/open → identical `RuntimeValue`; **downgrade-attack test**; relay sees only ciphertext; **wasm headless** |
| **4** | `PQ` suite (pure ML-KEM-768 / ML-DSA-65) | **NIST KATs**; pure-PQ purity grep; classical↔PQ negotiation |
| **5** | **PQ Triple Ratchet** (L3 self-healing FS) | post-compromise recovery test; per-message advance; deterministic carve-out |
| **6** | Persistent identity + SHAKE256 `DestAddr` (≥256-bit) | stable across restarts; forged identity ≠ address; collision margin documented |
| **7** | `PQ-Max` (ML-KEM-1024 / ML-DSA-87 / SLH-DSA root) + libcrux `verified` swap | KATs; CNSA-2.0 profile; per-suite `.wasm` size measured |
| **8** | QUIC (quinn + rustls **PQ KEX**) + WebTransport | one sealed blob round-trips; transport handshake is PQ (metadata layer) |
| **9** | iroh + libp2p-as-transport (reuse `gossip.rs`) | mesh composes across mixed substrates |
| **10** | Announce routing + `over <transport>` grammar + radios (BLE/LoRa) + fragmentation | route w/o static config; > MTU frame fragments through a simulated lossy link |

---

## 12. Honest limits & risks

1. **Metadata.** E2E protects content, not metadata (topic, timing, size). Only a PQ *transport* (§5-B) hides
   hop metadata; the relay still routes by topic. Stated plainly, never hidden.
2. **Unaudited PQ libraries.** RustCrypto is KAT-correct but unaudited; `ml-dsa` shipped a timing CVE in
   2026. Mitigated by hybrid default, crypto-agility (swap/bump), the libcrux verified option, and an
   "experimental until audited" label on the pure-PQ suites. **#1 risk.**
3. **Ratchet complexity.** L3 PQ self-healing FS is the hardest phase; staged strictly behind a working `PQ`
   one-shot, with adversarial post-compromise tests before it ships.
4. **Datagram handshakes** over lossy radios (BLE/LoRa) need fragmentation + retransmit; the multi-message
   PQXDH handshake assumes reliable/ordered delivery (true for relay/QUIC/libp2p). Simulate a lossy small-MTU
   link before any hardware.
5. **Per-transport maturity** is not yet libp2p-grade (see §10).
6. **`PNP` pad economics & distribution.** The one-time pad is unconditionally secret but costs one pre-shared
   true-random byte per plaintext byte, so it is a break-glass tier for crown-jewel traffic, not the bulk path;
   and it leaks length + timing metadata like every other content channel. It also inherits the classic OTP
   provisioning problem: the pad must be generated from real entropy and distributed out of band (courier disk,
   hardware-RNG dump, QKD) — the runtime deliberately manufactures no randomness. Pool exhaustion fails closed.

---

## 13. Critical files

Reuse, don't reinvent:
- `crates/logicaffeine_system/src/net.rs` — the seam that becomes transport-parameterized.
- `crates/logicaffeine_system/src/relay.rs`, `relay_proto.rs`, `relay_browser.rs` — first `impl Transport`;
  `serve_bridged` becomes router-driven.
- `crates/logicaffeine_system/src/addr.rs` — extend `canonical_topic` to resolve `DestAddr`.
- `crates/logicaffeine_compile/src/concurrency/marshal.rs` — the opaque blob the `SecureChannel` wraps; the
  `None`-on-malformed contract `open` mirrors. **Unchanged.**
- `crates/logicaffeine_compile/src/concurrency/channel.rs` — the `SecureChannel` envelope + `Suite` registry;
  `SUITE_PNP` is registered here.
- `crates/logicaffeine_compile/src/concurrency/pnp.rs` — **shipped:** the `PNP` information-theoretic one-time
  pad tier (Decision Record M) — `PadPool` (quality-gated, directional split), `PnpSuite` (cursor-rotated
  covers, one-time MAC, resync, fail-closed), crash-safe `PadLedgerStore`.
- `crates/logicaffeine_compile/src/interpreter.rs` — Send/Await/Sync/drain seams (~1874 seal, ~2400 open).
- `crates/logicaffeine_language/src/ast/stmt.rs` + `parser/mod.rs` — additive `secure` / suite + `over`
  transport fields (mirror the `compressed` precedent).
- **New:** `crates/logicaffeine_system/src/transport/{mod,router,loopback,quic,webtransport,iroh,libp2p,btle,serial}.rs`,
  `identity.rs`; new crate `crates/logicaffeine_crypto/` (`channel.rs`, `suite.rs`, `ratchet.rs`, `identity.rs`, `kat.rs`).

---

## Sources

- NIST FIPS 203 (ML-KEM), 204 (ML-DSA), 205 (SLH-DSA), final 2024-08-13; FIPS 206 (FN-DSA) pending; HQC selected
  as code-based backup KEM, 2025-03 — <https://en.wikipedia.org/wiki/NIST_Post-Quantum_Cryptography_Standardization>
- NSA **CNSA 2.0** (ML-KEM-1024 + ML-DSA-87, Category 5, no hybrid) — see the NIST FIPS overview above.
- **X-Wing** hybrid KEM (X25519 + ML-KEM-768, SHA3-256 combiner), IETF — <https://www.ietf.org/archive/id/draft-connolly-cfrg-xwing-kem-09.html>
- **Apple PQ3** — <https://security.apple.com/blog/imessage-pq3/>
- **Signal SPQR / Triple Ratchet** — <https://signal.org/blog/spqr/>
- **RustCrypto PQ status** (unaudited; `ml-dsa` timing CVE GHSA-hcp2-x6j4-29j7) — <https://www.projecteleven.com/blog/the-state-of-post-quantum-cryptography-in-rust-the-belt-is-vacant>
- **libcrux / Cryspen** formally-verified ML-KEM — <https://cryspen.com/post/ml-kem-implementation/>
