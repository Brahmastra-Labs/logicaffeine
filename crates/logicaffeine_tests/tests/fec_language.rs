//! Wave 2 — Reed-Solomon forward error correction, exposed to the Logos language.
//!
//! The pure engine (`logicaffeine_base::fec::ReedSolomon`) is proven byte-exact and
//! MDS in its own crate. Here we lock the *language surface*: two builtins that let a
//! Logos program protect data against packet loss and reconstruct it after losses —
//! the "recover without a retransmit" primitive for the lossy-network media transport.
//!
//! - `reedSolomonEncode(data, parity)` — `data` is a `Seq of Seq of Int` (each inner Seq
//!   a shard of bytes 0–255, all equal length); `parity` is the number of parity shards.
//!   Returns `n = data.count + parity` shards; the first `data.count` are the data shards
//!   unchanged (systematic), the rest are parity.
//! - `reedSolomonRecover(received, k)` — `received` is a `Seq` of `n` slots, each either a
//!   shard (`Seq of Int`) or `nothing` (an erased shard); `k` is the data-shard count.
//!   Returns all `n` shards byte-exact when at least `k` survived; fails otherwise.
//!
//! These tests are the spec. They must fail (RED) until the builtins exist, and — proven
//! by mutation in the engine crate — they bite: recovery is verified byte-exact, not by
//! shape alone.

mod common;
use common::{assert_all_tiers, assert_interpreter_fails};

/// The code is systematic: the first `k` encoded shards are the data shards, unchanged,
/// and `parity` more are appended. A conferencing sender can therefore transmit the data
/// shards immediately and the parity shards as pure redundancy.
#[test]
fn reed_solomon_encode_is_systematic_and_appends_parity() {
    assert_all_tiers(
        "## Main\n\
         Let d1 be [10, 20, 30].\n\
         Let d2 be [40, 50, 60].\n\
         Let data be [d1, d2].\n\
         Let encoded be reedSolomonEncode(data, 2).\n\
         Show length of encoded.\n\
         Show item 1 of encoded.\n\
         Show item 2 of encoded.\n",
        "4\n[10, 20, 30]\n[40, 50, 60]",
    );
}

/// The money test: encode 2 data + 2 parity shards, then LOSE two shards (a data shard
/// and a parity shard). Any two survivors must reconstruct the original data byte-exact —
/// the whole point of forward error correction on a lossy link.
#[test]
fn reed_solomon_recovers_the_data_after_losing_up_to_parity_many_shards() {
    assert_all_tiers(
        "## Main\n\
         Let d1 be [65, 66, 67].\n\
         Let d2 be [68, 69, 70].\n\
         Let data be [d1, d2].\n\
         Let encoded be reedSolomonEncode(data, 2).\n\
         Let a be item 2 of encoded.\n\
         Let b be item 4 of encoded.\n\
         Let received be [nothing, a, nothing, b].\n\
         Let recovered be reedSolomonRecover(received, 2).\n\
         Show item 1 of recovered.\n\
         Show item 2 of recovered.\n",
        "[65, 66, 67]\n[68, 69, 70]",
    );
}

/// Losing more shards than the code can bear (3 of 4, leaving fewer than `k` survivors)
/// must fail loudly — never silently return corrupt data.
#[test]
fn reed_solomon_recover_declines_when_more_than_parity_shards_are_lost() {
    assert_interpreter_fails(
        "## Main\n\
         Let d1 be [1, 2].\n\
         Let d2 be [3, 4].\n\
         Let data be [d1, d2].\n\
         Let encoded be reedSolomonEncode(data, 2).\n\
         Let b be item 4 of encoded.\n\
         Let received be [nothing, nothing, nothing, b].\n\
         Let recovered be reedSolomonRecover(received, 2).\n\
         Show recovered.\n",
        "insufficient surviving shards",
    );
}

/// The strongest FEC claim: lose BOTH data shards and reconstruct them purely from the
/// parity shards. A pattern-specific bug (only recovering when some data survives) dies here.
#[test]
fn reed_solomon_rebuilds_both_data_shards_from_parity_alone() {
    assert_all_tiers(
        "## Main\n\
         Let d1 be [11, 22, 33, 44].\n\
         Let d2 be [55, 66, 77, 88].\n\
         Let data be [d1, d2].\n\
         Let encoded be reedSolomonEncode(data, 2).\n\
         Let p1 be item 3 of encoded.\n\
         Let p2 be item 4 of encoded.\n\
         Let received be [nothing, nothing, p1, p2].\n\
         Let recovered be reedSolomonRecover(received, 2).\n\
         Show item 1 of recovered.\n\
         Show item 2 of recovered.\n",
        "[11, 22, 33, 44]\n[55, 66, 77, 88]",
    );
}

/// A larger code (4 data + 3 parity) recovers from a scattered loss of the full parity budget
/// (3 shards gone, at positions 1, 4 and 6). Proves the language surface scales past the toy case.
#[test]
fn reed_solomon_recovers_a_larger_code_after_a_scattered_maximal_loss() {
    assert_all_tiers(
        "## Main\n\
         Let s1 be [1, 2].\n\
         Let s2 be [3, 4].\n\
         Let s3 be [5, 6].\n\
         Let s4 be [7, 8].\n\
         Let data be [s1, s2, s3, s4].\n\
         Let encoded be reedSolomonEncode(data, 3).\n\
         Let a be item 2 of encoded.\n\
         Let b be item 3 of encoded.\n\
         Let c be item 5 of encoded.\n\
         Let e be item 7 of encoded.\n\
         Let received be [nothing, a, b, nothing, c, nothing, e].\n\
         Let recovered be reedSolomonRecover(received, 4).\n\
         Show item 1 of recovered.\n\
         Show item 2 of recovered.\n\
         Show item 3 of recovered.\n\
         Show item 4 of recovered.\n",
        "[1, 2]\n[3, 4]\n[5, 6]\n[7, 8]",
    );
}
