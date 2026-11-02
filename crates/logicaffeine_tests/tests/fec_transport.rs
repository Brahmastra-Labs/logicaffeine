//! FEC payload transport — the media-chunk-over-a-lossy-link primitive.
//!
//! `reedSolomonEncode`/`Recover` (Wave 2) operate on pre-split equal shards. A real media
//! chunk is a flat byte buffer of arbitrary length. `fecProtect`/`fecRestore` close that gap:
//!
//! - `fecProtect(payload, k, m)` — frame the byte payload (`Seq of Int`) with a length header,
//!   zero-pad, split into `k` equal shards, and Reed-Solomon encode into `k + m` packets. Any
//!   `k` of the `k + m` packets reconstruct the payload, so up to `m` may be lost in transit.
//! - `fecRestore(received, k)` — from `received` (a `Seq` of `Seq of Int` packets or `nothing`
//!   for a lost one), recover the data shards, reassemble, and strip the header + padding to
//!   return the EXACT original payload — byte-for-byte, any length.
//!
//! These are the spec; they must fail until the builtins exist, and they bite (a length/padding
//! bug or a wrong-shard recovery changes the restored bytes).

mod common;
use common::assert_all_tiers;

/// The headline claim: a 6-byte chunk ("Hello!") is sent as 5 packets and survives losing 2 of
/// them (a data packet and a parity packet) — reconstructed byte-exact with no retransmit.
#[test]
fn fec_protect_then_restore_survives_losing_parity_many_packets() {
    assert_all_tiers(
        "## Main\n\
         Let payload be [72, 101, 108, 108, 111, 33].\n\
         Let packets be fecProtect(payload, 3, 2).\n\
         Show length of packets.\n\
         Let a be item 2 of packets.\n\
         Let b be item 3 of packets.\n\
         Let c be item 5 of packets.\n\
         Let received be [nothing, a, b, nothing, c].\n\
         Let restored be fecRestore(received, 3).\n\
         Show restored.\n",
        "5\n[72, 101, 108, 108, 111, 33]",
    );
}

/// A payload whose length (7) is NOT a multiple of the shard count (3) exercises the framing
/// header and zero-padding: restore must strip both and return exactly the 7 original bytes,
/// never the padding.
#[test]
fn fec_restore_strips_framing_and_padding_for_a_ragged_length_payload() {
    assert_all_tiers(
        "## Main\n\
         Let payload be [1, 2, 3, 4, 5, 6, 7].\n\
         Let packets be fecProtect(payload, 3, 2).\n\
         Let a be item 2 of packets.\n\
         Let b be item 3 of packets.\n\
         Let c be item 5 of packets.\n\
         Let received be [nothing, a, b, nothing, c].\n\
         Let restored be fecRestore(received, 3).\n\
         Show restored.\n",
        "[1, 2, 3, 4, 5, 6, 7]",
    );
}

/// A single-byte payload — the degenerate small chunk — round-trips through protect/restore.
#[test]
fn fec_protects_and_restores_a_single_byte_payload() {
    assert_all_tiers(
        "## Main\n\
         Let payload be [200].\n\
         Let packets be fecProtect(payload, 2, 2).\n\
         Let a be item 1 of packets.\n\
         Let b be item 4 of packets.\n\
         Let received be [a, nothing, nothing, b].\n\
         Let restored be fecRestore(received, 2).\n\
         Show restored.\n",
        "[200]",
    );
}
