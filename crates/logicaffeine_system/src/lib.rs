#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// === Always Available (Core IO) ===
pub mod io;
pub mod temporal;

// The thin-relay wire protocol — shared by the native relay server (under
// `networking`) and the browser relay client (wasm). Target-agnostic (serde
// only), so it compiles into both the native binary and the wasm bundle without
// dragging in libp2p.
pub mod relay_proto;

// Address normalization (libp2p multiaddr → ws:// URL) so the interpreter accepts
// the same peer-address surface as the compiled path. Pure string logic, no
// libp2p, no target gating — identical on native and wasm.
pub mod addr;

// The native WebSocket relay server + client. A LIGHT capability behind its own
// `relay` feature (tokio-tungstenite, NO libp2p) so the interpreter can network
// over the relay without the mesh stack. `networking` implies `relay`.
#[cfg(all(not(target_arch = "wasm32"), feature = "relay"))]
pub mod relay;

// Browser relay client — a `web-sys` WebSocket speaking `relay_proto`, the
// browser's door into a native node's relay. wasm-only; no libp2p.
#[cfg(target_arch = "wasm32")]
pub mod relay_browser;

// The cross-target `Net` handle the interpreter holds — native RelayClient or
// browser WebSocket behind one API. Available wherever a relay client is.
#[cfg(any(all(not(target_arch = "wasm32"), feature = "relay"), target_arch = "wasm32"))]
pub mod net;

// Native-only core modules
#[cfg(not(target_arch = "wasm32"))]
pub mod time;
#[cfg(not(target_arch = "wasm32"))]
pub mod env;
#[cfg(not(target_arch = "wasm32"))]
pub mod random;
#[cfg(not(target_arch = "wasm32"))]
pub mod text;

// === Feature-Gated Modules ===

// Persistence feature: file operations, storage, VFS
#[cfg(feature = "persistence")]
pub mod file;
#[cfg(feature = "persistence")]
pub mod fs;
#[cfg(feature = "persistence")]
pub mod storage;

// Networking feature: P2P networking
#[cfg(feature = "networking")]
pub mod network;

// Concurrency feature: parallel computation
#[cfg(feature = "concurrency")]
pub mod concurrency;
#[cfg(feature = "concurrency")]
pub mod memory;

// Distributed<T> requires both networking AND persistence
#[cfg(all(feature = "networking", feature = "persistence"))]
pub mod distributed;

// CRDT sync wrapper requires networking (uses tokio + libp2p)
#[cfg(feature = "networking")]
pub mod crdt;

// Runtime support for the Word8/16/32/64 ring types in compiled LOGOS (constructors,
// rotations, Showable). Operators live on the newtypes themselves in logicaffeine_base.
pub mod word_rt;

// ML-KEM (Kyber) NTT runtime kernel (scalar + AVX2 i16×16) reached via the `mlkemNtt` stdlib fn.
pub mod ntt;

// Keccak-f[1600] + SHA-3 / SHAKE — the symmetric/hash layer (reached via sha3_256/shake128/… ).
pub mod keccak;

// ChaCha20-Poly1305 AEAD (RFC 8439) — the symmetric seal for the post-quantum channel.
pub mod aead;

// ML-KEM-768 (FIPS-203) keygen/encaps/decaps composed from the NTT + Keccak kernels — the
// post-quantum key exchange for the channel handshake.
pub mod mlkem;

// ML-DSA-65 (FIPS-204) signature kernels — the post-quantum signature complement to ML-KEM.
pub mod mldsa;

// Re-export tokio for async main support (native only)
#[cfg(not(target_arch = "wasm32"))]
pub use tokio;

// Re-export commonly used items
pub use io::{show, read_line, println, eprintln, print, Showable};
pub use temporal::{LogosDate, LogosMoment, LogosSpan, LogosTime};
pub use word_rt::{
    hsum_lanes4, int_of_word16, int_of_word32, int_of_word64, lanes16_word16, lanes4_word64,
    lanes8_word32, montmul32, mul32x32to64, mulhi16, splat4_word64, and_not4,
    rotl, rotr, word_and, word_or, word_not, seq_of_lanes16, seq_of_lanes4, word32_shr, word64_and, word64_shl, word64_shr,
    seq_of_lanes8, splat16_word16, splat8_word32, word16, word32, word64, word8, Lanes16Word16,
    Lanes4Word64, Lanes8Word32, Word16, Word32, Word64, Word8, WordRotate,
    Lanes4Word32, lanes4_word32, lanes4_of, seq_of_lanes4w32, sha1rnds4, sha1msg1, sha1msg2, sha1nexte,
    Lanes16Word8, lanes16_word8, seq_of_lanes16w8, splat16_word8, shuffle16, shr_bytes16,
    interleave_lo16, interleave_hi16, byte_add16, maddubs16, packus16,
};
pub use ntt::{
    mlkem_base_mul, mlkem_byte_decode, mlkem_byte_encode, mlkem_cbd2, mlkem_cbd3, mlkem_compress,
    mlkem_decompress, mlkem_inv_ntt, mlkem_ntt, mlkem_sample_a, mlkem_sample_ntt, mlkem_to_mont,
};
pub use keccak::{sha3_256, sha3_512, shake128, shake256};

/// Panic with a custom message (used by generated LOGOS code)
pub fn panic_with(reason: &str) -> ! {
    panic!("{}", reason);
}

/// Formatting utilities
pub mod fmt {
    pub fn format<T: std::fmt::Display>(x: T) -> String {
        format!("{}", x)
    }
}
