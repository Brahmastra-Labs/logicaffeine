//! Tier cache (HOTSWAP §P12): persist a compiled [`FnBytecode`] keyed by
//! `(source, optimization-config, tier)` so a re-run skips re-optimization.
//!
//! Soundness rests on the key: a hit means the EXACT same source was compiled at the
//! same config and tier. A `FnBytecode`'s `Op`s carry interner-relative `Symbol`
//! indices, which are only meaningful within the interner that produced them — but
//! re-parsing identical source reproduces the same interning order, so a same-source
//! hit's indices resolve to the same symbols. Changing one byte of source (or the
//! config, or the tier) changes the key → a miss → recompile. (m8: tier is in the key.)
//!
//! [`encode`]/[`decode`]/[`cache_key`] are platform-agnostic so the browser warm tier
//! (P13) can store the same wire bytes through OPFS (`Vfs`); [`store`]/[`load`] are the
//! desktop on-disk sidecar.

use super::fn_bytecode::FnBytecode;

/// The compiler-identity component of the cache key. A `FnBytecode`'s `Op`s carry
/// interner-relative `Symbol` indices, and the interning ORDER is fixed by the compiler
/// binary + the build-time-baked `lexicon.json` + the pre-seeded primitives — none of
/// which appear in the user `source`. So the key MUST fold in a compiler-version stamp,
/// or a post-upgrade run of byte-identical source would hit a stale entry whose indices
/// now resolve to different symbols (a silent miscompile). This mirrors `aot_cache_key`
/// folding in the `rustc` version. `CARGO_PKG_VERSION` is bumped in lockstep across all
/// crates on every release (and any lexicon/interning change ships in a release), so it
/// is the correct cross-release invalidator.
const COMPILER_STAMP: &str = concat!("logos-tiercache-v1-", env!("CARGO_PKG_VERSION"));

/// `(compiler, source, config, tier)` → a stable hex key. FNV-1a over the compiler
/// stamp, the source bytes, the config bitset, and the tier.
pub fn cache_key(source: &str, config_bits: u64, tier: u8) -> String {
    format!("{:016x}", key_hash(source, config_bits, tier))
}

fn key_hash(source: &str, config_bits: u64, tier: u8) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    mix(COMPILER_STAMP.as_bytes());
    mix(source.as_bytes());
    mix(&config_bits.to_le_bytes());
    mix(&[tier]);
    h
}

/// FNV-1a over the JSON payload — a content checksum so a truncated / bit-flipped entry
/// that still parses as JSON is rejected rather than installed (see [`decode`]).
fn payload_checksum(json: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in json.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Serialize a body to the cache wire format — bit-exact (`f64` by bits, `Symbol` by
/// index), so a decoded body is byte-identical to the one that was stored. A `checksum`
/// line precedes the JSON so corruption is caught on the way back in.
pub fn encode(fnbc: &FnBytecode) -> String {
    let json = serde_json::to_string(fnbc).expect("FnBytecode serializes");
    format!("{:016x}\n{json}", payload_checksum(&json))
}

/// Deserialize a body; `None` on ANY corruption — a corrupt entry is genuinely just a
/// miss (the VM recompiles). Rejects: a missing/garbled checksum line, a checksum that
/// does not match the payload (truncation / bit-flip), or non-deserializable JSON.
pub fn decode(s: &str) -> Option<FnBytecode> {
    let (sum, json) = s.split_once('\n')?;
    let expected = u64::from_str_radix(sum.trim(), 16).ok()?;
    if payload_checksum(json) != expected {
        return None;
    }
    serde_json::from_str(json).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn cache_path(dir: &std::path::Path, key: &str) -> std::path::PathBuf {
    dir.join(format!("{key}.bc"))
}

/// Store `fnbc` under `(source, config, tier)` in `dir` (created if absent).
/// Best-effort — a write failure just means the next run recompiles.
#[cfg(not(target_arch = "wasm32"))]
pub fn store(
    dir: &std::path::Path,
    source: &str,
    config_bits: u64,
    tier: u8,
    fnbc: &FnBytecode,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(cache_path(dir, &cache_key(source, config_bits, tier)), encode(fnbc))
}

/// Load the body cached for `(source, config, tier)` from `dir`; `None` on miss or
/// corruption.
#[cfg(not(target_arch = "wasm32"))]
pub fn load(
    dir: &std::path::Path,
    source: &str,
    config_bits: u64,
    tier: u8,
) -> Option<FnBytecode> {
    let s = std::fs::read_to_string(cache_path(dir, &cache_key(source, config_bits, tier))).ok()?;
    decode(&s)
}
