//! QA — error paths. The scariest bugs in a codec are the SILENT ones: bad inputs that return
//! plausible-but-wrong bytes. Every guard here proves the stack fails LOUDLY instead — a lost-cause
//! recovery, a degenerate code, or a malformed shard set is rejected, never quietly corrupted.

mod common;
use common::assert_interpreter_fails;

/// Losing more than the parity budget leaves fewer than k packets — restore must refuse, never
/// hand back garbage that looks like a frame.
#[test]
fn fec_restore_refuses_when_too_many_packets_are_lost() {
    assert_interpreter_fails(
        "## Main\n\
         Let payload be [1, 2, 3, 4, 5, 6].\n\
         Let packets be fecProtect(payload, 3, 2).\n\
         Let a be item 5 of packets.\n\
         Let received be [nothing, nothing, nothing, nothing, a].\n\
         Let restored be fecRestore(received, 3).\n\
         Show restored.\n",
        "insufficient surviving",
    );
}

/// A zero parity count is not an erasure code — reject it rather than build a degenerate one.
#[test]
fn fec_protect_rejects_a_zero_parity_count() {
    assert_interpreter_fails(
        "## Main\n\
         Let payload be [1, 2, 3].\n\
         Let packets be fecProtect(payload, 2, 0).\n\
         Show length of packets.\n",
        "positive",
    );
}

/// `reedSolomonRecover` with fewer than k survivors declines rather than fabricating shards.
#[test]
fn reed_solomon_recover_declines_below_k_survivors() {
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
        "insufficient surviving",
    );
}

/// `reedSolomonEncode` with ragged (unequal-length) shards must be rejected, never silently
/// truncated to the shortest.
#[test]
fn reed_solomon_encode_rejects_ragged_shards() {
    assert_interpreter_fails(
        "## Main\n\
         Let d1 be [1, 2, 3].\n\
         Let d2 be [4, 5].\n\
         Let data be [d1, d2].\n\
         Let encoded be reedSolomonEncode(data, 2).\n\
         Show length of encoded.\n",
        "equal length",
    );
}
