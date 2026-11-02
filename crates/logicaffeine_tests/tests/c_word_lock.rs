//! ★ LOCK — a machine word means on C what it means on the VM. ★
//!
//! `word8`, `word16`, `word32` and `word64` make a value of the ring ℤ/2ᵏℤ: its `+ − ×` wrap at the width, its
//! `/` and `%` divide unsigned, it orders and shows by its unsigned value, a shift moves its bits by a count
//! modulo the width, and it never combines with an `Int` or with a word of another width — the VM refuses
//! both, in its own words. The direct C backends hold a word in an `int64_t` as its zero-extended bits, so
//! every one of those rules is a place C could print the signed reading of the bits, or let a sum run past
//! the width. Each program here pins one rule and runs on both C backends against the VM, the whole outcome
//! compared: every line shown, the failure's words, the way the run ends.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use tier_parity::{Backend, Program};

const PROGRAMS: &[(&str, &str)] = &[
    (
        "show/a_word64_past_the_signed_range",
        "## Main\nLet w be word64(0 - 3).\nShow w.\nShow word64(0 - 1).\n",
    ),
    (
        "show/a_word64_in_text",
        "## Main\nLet w be word64(0 - 1).\nShow \"w=\" + w.\nShow \"[{w}]\".\nShow \"[{w:>22}]\".\n",
    ),
    (
        "show/every_width_of_a_wide_integer",
        "## No Inline\n## To emit (n: Int):\n    Show word8(n).\n    Show word16(n).\n    Show word32(n).\n    Show word64(n).\n\n## Main\nLet large be 2 ** 100 + 19.\nCall emit with large.\nCall emit with 0 - large.\n",
    ),
    (
        "ring/addition_wraps_at_each_width",
        "## Main\nShow word8(250) + word8(10).\nShow word16(65535) + word16(2).\nShow word32(4294967295) + word32(1).\nShow word64(0 - 1) + word64(5).\n",
    ),
    (
        "ring/subtraction_wraps_below_zero",
        "## Main\nShow word8(3) - word8(5).\nShow word16(5) - word16(6).\nShow word32(0) - word32(1).\nShow word64(1) - word64(2).\n",
    ),
    (
        "ring/multiplication_wraps",
        "## Main\nShow word8(16) * word8(17).\nShow word32(65536) * word32(65537).\nShow word64(0 - 1) * word64(3).\n",
    ),
    (
        "ring/a_running_sum_wraps_in_a_loop",
        "## Main\nLet mutable w be word32(4294967290).\nRepeat for i from 1 to 10:\n    Set w to w + word32(1).\nShow w.\n",
    ),
    (
        "divide/unsigned_quotient_and_remainder",
        "## Main\nLet c be word64(0 - 1).\nShow c / word64(2).\nShow c % word64(10).\nShow word8(200) / word8(7).\n",
    ),
    (
        "divide/by_a_zero_word_fails",
        "## Main\nLet z be word32(0).\nShow word32(7).\nShow word32(7) / z.\n",
    ),
    (
        "divide/remainder_by_a_zero_word_fails",
        "## Main\nLet z be word64(0).\nShow word64(7) % z.\n",
    ),
    (
        "order/unsigned_comparison",
        "## Main\nLet big be word64(0 - 1).\nLet one be word64(1).\nShow big is greater than one.\nShow one is less than big.\nShow big is at least big.\nShow word8(200) is greater than word8(100).\n",
    ),
    (
        "equal/same_width_and_value",
        "## Main\nShow word32(5) equals word32(5).\nShow word32(5) equals word32(6).\nShow word64(0 - 1) is not equal to word64(0 - 1).\n",
    ),
    (
        "bits/bitwise_logic_stays_in_the_ring",
        "## Main\nLet a be word32(4042322160).\nLet b be word32(4278255360).\nShow a & b.\nShow a | b.\nShow a ^ b.\n",
    ),
    (
        "bits/shifts_move_bits_within_the_width",
        "## Main\nShow word32(1) shifted left by 31.\nShow word32(1) shifted left by 32.\nShow word8(129) shifted left by 1.\nShow word64(0 - 1) shifted right by 60.\nShow word64(1) shifted left by 63.\n",
    ),
    (
        "convert/a_word_read_back_as_an_int",
        "## Main\nShow intOfWord8(word8(300)).\nShow intOfWord16(word16(70000)).\nShow intOfWord32(word32(0 - 1)).\nShow intOfWord64(word64(0 - 1)).\nShow intOfWord64(word64(0 - 1)) + 1.\n",
    ),
    (
        "boundary/a_word_parameter_and_result",
        "## To bump (w: Word64) -> Word64:\n    Return w + word64(1).\n\n## To halve (w: Word32) -> Word32:\n    Return w / word32(2).\n\n## Main\nShow bump(word64(0 - 1)).\nShow bump(word64(0 - 3)).\nShow halve(word32(0 - 1)).\n",
    ),
    (
        "boundary/a_word_crosses_into_a_task",
        "## To report (w: Word64):\n    Show w.\n    Show w + word64(1).\n\n## Main\nLaunch a task to report with word64(0 - 1).\nShow \"main\".\n",
    ),
    (
        "boundary/a_word_crosses_a_pipe",
        "## Main\nLet ch be a Pipe of Word64.\nLet w be word64(0 - 2).\nSend w into ch.\nReceive a from ch.\nShow a.\nShow a + word64(3).\n",
    ),
    (
        "map/a_word_keys_a_map",
        "## Main\nLet m be a new Map of Word64 to Text.\nLet top be word64(0 - 1).\nLet one be word64(1).\nSet item top of m to \"top\".\nSet item one of m to \"one\".\nShow item top of m.\nShow item one of m.\nShow length of m.\nShow m.\n",
    ),
    (
        "map/a_word_fills_a_text_keyed_map",
        "## Main\nLet m be a new Map of Text to Word32.\nSet item \"top\" of m to word32(0 - 1).\nShow item \"top\" of m.\n",
    ),
    (
        "seq/a_word_read_back_is_the_word",
        "## Main\nLet mutable ws be a new Seq of Word32.\nPush word32(4294967295) to ws.\nPush word32(7) to ws.\nShow item 1 of ws + word32(1).\nShow item 2 of ws * word32(3).\n",
    ),
    (
        "seq/a_list_of_word64_shows_unsigned",
        "## Main\nLet mutable ws be a new Seq of Word64.\nPush word64(0 - 1) to ws.\nPush word64(5) to ws.\nShow ws.\nShow item 1 of ws.\nShow \"{ws}\".\n",
    ),
    (
        "seq/a_loop_over_words_wraps",
        "## Main\nLet ws be [word32(4294967295), word32(2)].\nLet mutable total be word32(0).\nRepeat for w in ws:\n    Set total to total + w.\nShow total.\n",
    ),
    (
        "seq/a_word_list_crosses_a_call",
        "## To first (ws: Seq of Word64) -> Word64:\n    Return item 1 of ws + word64(1).\n\n## Main\nLet mutable ws be a new Seq of Word64.\nPush word64(0 - 1) to ws.\nShow first(ws).\n",
    ),
    (
        "set/a_set_of_word64_shows_unsigned",
        "## Main\nLet mutable s be a new Set of Word64.\nAdd word64(0 - 1) to s.\nAdd word64(3) to s.\nShow s.\nShow length of s.\n",
    ),
    (
        "set/iterating_a_set_of_words",
        "## Main\nLet mutable s be a new Set of Word32.\nAdd word32(4294967295) to s.\nAdd word32(6) to s.\nRepeat for w in s:\n    Show w + word32(1).\n",
    ),
    (
        "map/word_values_read_back_as_words",
        "## Main\nLet m be a new Map of Text to Word32.\nSet item \"a\" of m to word32(4294967295).\nShow item \"a\" of m + word32(1).\nShow m.\n",
    ),
    (
        "map/a_map_of_word64_to_word64_shows_unsigned",
        "## Main\nLet n be a new Map of Word64 to Word64.\nLet top be word64(0 - 1).\nSet item top of n to word64(0 - 2).\nShow n.\nShow item top of n + word64(2).\n",
    ),
    (
        "map/iterating_word_entries",
        "## Main\nLet n be a new Map of Word32 to Word32.\nLet top be word32(4294967295).\nSet item top of n to word32(1).\nRepeat for (k, v) in n:\n    Show k + v.\n",
    ),
    (
        "refuse/a_word_and_an_int",
        "## Main\nLet a be word32(5).\nShow \"before\".\nShow a + 1.\n",
    ),
    (
        "refuse/two_widths",
        "## Main\nLet a be word32(5).\nLet b be word64(5).\nShow \"before\".\nShow a + b.\n",
    ),
    (
        "refuse/ordering_two_widths",
        "## Main\nLet a be word32(5).\nLet b be word64(5).\nShow a is less than b.\n",
    ),
];

/// A program the front end refuses fails alike on every backend, so the sweep would count it a match: each
/// program here must reach the backends to pin anything.
/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn every_word_program_reaches_the_backends() {
    let refused: Vec<&str> = PROGRAMS
        .iter()
        .filter(|(_, source)| tier_parity::rejection(source).is_some())
        .map(|(id, _)| *id)
        .collect();
    assert!(
        refused.is_empty(),
        "the front end refuses {} word program(s), which pin nothing: {refused:?}",
        refused.len()
    );
}

#[test]
fn every_word_rule_runs_on_both_c_backends_as_on_the_vm() {
    let programs: Vec<Program> = PROGRAMS
        .iter()
        .map(|(id, source)| Program::new(format!("word/{id}"), *source))
        .collect();
    let rows = tier_parity::sweep(
        &programs,
        &[Backend::TreeWalker, Backend::C, Backend::CBytecode],
        4,
    )
    .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| {
            format!(
                "{} on {}: {} {}",
                row.program,
                row.backend.name(),
                row.status(),
                row.detail()
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{} (program, backend) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
