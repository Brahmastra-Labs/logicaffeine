//! FORGE WORD-LIST PIN LANE — a `Seq of Word32` read by index in a hot loop (`item idx of tbl`) tiers. This
//! is the sha256 message-schedule / round-constant read pattern (`item i of w` / `item i of kk`). A Word32
//! list's elements ARE masked i64s, so the pin REUSES the IntList buffer machinery — the only difference from
//! an `Int` list is that an indexed load yields a `Kind::Word32` (so the Word ops downstream accept it),
//! never an `Int`. Correctness = forge == the tree-walker; tiering (x86_64) = the hot read loop went native.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn eq_tw(label: &str, src: &str, want_tier: bool) {
    let tw = tw_outcome_with_args(src, &[]);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    assert!(
        tw.error.is_none(),
        "[{label}] tree-walker errored: {:?}\n{src}",
        tw.error
    );
    assert!(
        forge.error.is_none(),
        "[{label}] forge errored: {:?}\n{src}",
        forge.error
    );
    assert_eq!(
        forge.output.trim(),
        tw.output.trim(),
        "[{label}] forge diverged\n{src}"
    );
    #[cfg(target_arch = "x86_64")]
    if want_tier {
        let (_, region_ok) = tier.region_counts();
        assert!(
            region_ok >= 1,
            "[{label}] did NOT tier its hot read loop (region={region_ok}) — a `Seq of Word32` indexed read \
             declined; the forge needs a Word32-list pin (reuse the IntList buffer, tag the load Word32)\n{src}"
        );
    }
}

#[test]
fn forge_word32_list_indexed_read() {
    // Build a 64-entry Word32 table, then a hot loop that XORs `item idx of tbl` into a Word32 accumulator —
    // the sha256 `h = h xor (item i of kk)` shape. The read loop must tier over the pinned Word32 table.
    eq_tw(
        "word32 table xor-read",
        "## Main\n\
         Let mutable tbl be a new Seq of Word32.\n\
         Let mutable s be 0.\n\
         While s is less than 64:\n\
         \x20   Push word32(s * 2654435761) to tbl.\n\
         \x20   Set s to s + 1.\n\
         Let mutable h be word32(2166136261).\n\
         Let mutable k be 1.\n\
         While k is at most 4000000:\n\
         \x20   Let idx be (k mod 64) + 1.\n\
         \x20   Set h to (h xor (item idx of tbl)) * word32(16777619).\n\
         \x20   Set k to k + 1.\n\
         Show intOfWord32(h).\n",
        true,
    );
    // A Word32 table read ADDED (modular) into the accumulator — exercises the pinned-load result flowing
    // into the Word arithmetic lane.
    eq_tw(
        "word32 table add-read",
        "## Main\n\
         Let mutable tbl be a new Seq of Word32.\n\
         Let mutable s be 0.\n\
         While s is less than 16:\n\
         \x20   Push word32((s + 1) * 40503) to tbl.\n\
         \x20   Set s to s + 1.\n\
         Let mutable acc be word32(0).\n\
         Let mutable k be 1.\n\
         While k is at most 4000000:\n\
         \x20   Let idx be (k mod 16) + 1.\n\
         \x20   Set acc to acc + (item idx of tbl).\n\
         \x20   Set k to k + 1.\n\
         Show intOfWord32(acc).\n",
        true,
    );
}
