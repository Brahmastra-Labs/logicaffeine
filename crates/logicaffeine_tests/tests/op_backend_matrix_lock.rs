//! ★ THE OP × BACKEND GAUNTLET + RATCHET ★ — every LOGOS ISA op, exhaustive edge cases run
//! DIFFERENTIALLY across every AOT backend. Oracle-verified (tw==VM), per-backend MONOTONE FLOORS.
//! Only 2 ops are excluded from THIS all-backends-run corpus (BigIntWide=hw wide-lane; CrdtJoin, whose
//! `Converge` source verb + full cross-tier ≡VM proof live in `converge_crdt_lock` — a CRDT program is not
//! in THIS corpus because it runs EVERY backend and the Rust tier does not compile a CRDT `merge`/`converge`
//! call); the reversible-core Rev{Add,Sub,Xor,Neg} graduated to source via `Reversibly:`/`Uncompute:`.
//!
//! The 7th column is the spatial FABRIC. The fabric realizes 118 of 120 ISA ops (proven ≡VM in
//! `fabric_coverage_lock`, `FABRIC_TILE_FLOOR=118`), so as an *op tile set* it is the most complete
//! backend. This matrix asks a narrower, per-PROGRAM question — "does the fabric lower this whole
//! `## Main` output-tape program?" — via `whole_program_fabric_from_source` (the combinational
//! output-tape path). That path is STRAIGHT-LINE-Main-only by construction: a branch in Main (any
//! `If`/`While`/comparison-driven `Show`) is REFUSED because a conditional `Show` cannot ride the
//! combinational tape (control-flow bodies lower through the CFG engine, but that path is keyed on a
//! *named single function*, not the anonymous top-level Main these corpus programs use). So the fabric
//! genuinely lowers the straight-line whole programs — its floor here (FLOOR_FABRIC) is that honest
//! per-`## Main` count, NOT the 115-op tile coverage.
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_program_cfg_fabric_traced, try_main_cfg_fabric, whole_program_fabric_from_source,
};
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_cuda, compile_to_riscv, compile_to_rust, compile_to_tenstorrent,
    compile_to_wasm, tw_outcome, vm_outcome, vm_outcome_concurrent, vm_outcome_net,
};
use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

struct Prog {
    op: &'static str,
    label: &'static str,
    src: &'static str,
    expect_out: &'static str,
    expect_err: bool,
    oracle: &'static str,
}

// EXCLUDED FROM THIS ALL-BACKENDS-RUN CORPUS (2). BigIntWide = 256-bit hw wide-lane, lowered + ≡VM-proven
// via hand-built bytecode with no source emit site (ints use the BigInt VALUE type). CrdtJoin DOES have a
// source verb now — `Converge x into y` (idempotent per-field MAX, the sibling of `Merge`→CrdtMerge) — and
// is proven ≡VM across tw/VM/AST-C/bytecode-C/RISC-V in `converge_crdt_lock`; it is excluded HERE only
// because this matrix runs EVERY backend including Rust, and the Rust tier does not compile a CRDT
// `merge`/`converge` method call (a pre-existing Rust-backend CRDT-runtime gap, shared by `Merge`).
const HAND_BYTECODE_ONLY: &[&str] = &["BigIntWide", "CrdtJoin"];

// ★ HOST-IO ops, which cannot live in a DETERMINISTIC all-backends differential corpus: both dial a
// real socket, so there is no offline ≡VM witness to compare against. They are excluded on the same
// honest grounds as the argv/wall-clock ops elsewhere — `Tcp`'s real end-to-end ≡VM proof lives in
// `tcp_walker_lock`, and `Serve`'s in the HTTP lifecycle locks.
//
// ★ THEY USED TO BE EXCLUDED BY ACCIDENT RATHER THAN BY DECISION. The partition below asserted a
// hardcoded `115` against an ISA of 117, so these two sat in NO column of a matrix whose own doc
// claims "every LOGOS ISA op, exhaustive edge cases run DIFFERENTIALLY across every AOT backend" —
// and the assertion passed, because a hand-copied denominator only ever measures itself.
const HOST_IO_ONLY: &[&str] = &["Tcp", "Serve"];

// 116 == every source-emittable op (120 total − 2 hand-bytecode-only − 2 host-IO-only). C, Rust,
// RISC-V, and WASM cover the FULL source-emittable surface; CUDA covers 114 (Obs/TaskAwait ride other tiers),
// Tenstorrent 102. Ratcheted UP to the measured coverage (a floor set below actual is a silent regression risk).
const FLOOR_C: usize = 116;
const FLOOR_RUST: usize = 116;
const FLOOR_RISCV: usize = 116;
const FLOOR_CUDA: usize = 114;
const FLOOR_WASM: usize = 116;
const FLOOR_TENSTORRENT: usize = 102;
// The spatial fabric, measured per whole `## Main` program via `whole_program_fabric_from_source`
// (combinational output-tape lowering). The straight-line-Main-only path refuses branchy Mains, so this
// floor (83 of the 116 source-emittable ops) is what the fabric GENUINELY lowers for these output-tape
// programs — deliberately NOT the 118-op tile coverage (see the module doc + `fabric_coverage_lock`).
const FLOOR_FABRIC: usize = 83;

const CORPUS: &[Prog] = &[
    Prog { op: "LoadConst", label: "const_positive", src: "## Main\nShow 42.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "LoadConst", label: "const_add_small", src: "## Main\nShow 2 + 3.\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "LoadConst", label: "const_negative_result", src: "## Main\nShow (0 - 5) + 2.\n", expect_out: "-3", expect_err: false, oracle: "plain" },
    Prog { op: "LoadConst", label: "const_zero", src: "## Main\nShow 0.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "LoadConst", label: "const_one", src: "## Main\nShow 1.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "LoadConst", label: "const_i64_max", src: "## Main\nShow 9223372036854775807.\n", expect_out: "9223372036854775807", expect_err: false, oracle: "plain" },
    Prog { op: "LoadConst", label: "const_neg_via_zero_minus", src: "## Main\nShow 0 - 1.\n", expect_out: "-1", expect_err: false, oracle: "plain" },
    Prog { op: "Move", label: "move_copy_binding", src: "## Main\nLet x be 7.\nLet y be x.\nShow y.\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "Move", label: "move_while_accumulate", src: "## Main\nLet mutable acc be 100.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set acc to acc - i.\n    Set i to i + 1.\nShow acc.\n", expect_out: "85", expect_err: false, oracle: "plain" },
    Prog { op: "Move", label: "move_chain_of_copies", src: "## Main\nLet a be 3.\nLet b be a.\nLet c be b.\nShow c.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "Move", label: "move_copy_zero", src: "## Main\nLet x be 0.\nLet y be x.\nShow y.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Move", label: "move_copy_negative", src: "## Main\nLet x be 0 - 9.\nLet y be x.\nShow y.\n", expect_out: "-9", expect_err: false, oracle: "plain" },
    Prog { op: "Move", label: "move_while_to_negative", src: "## Main\nLet mutable acc be 50.\nLet mutable i be 1.\nWhile i is at most 10:\n    Set acc to acc - i.\n    Set i to i + 1.\nShow acc.\n", expect_out: "-5", expect_err: false, oracle: "plain" },
    Prog { op: "Add", label: "add_two_items", src: "## Main\nLet xs be [3, 5, 8].\nLet a be item 1 of xs.\nLet b be item 2 of xs.\nShow a + b.\n", expect_out: "8", expect_err: false, oracle: "plain" },
    Prog { op: "Add", label: "add_loop_with_div", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 5:\n    Let d be i + 1.\n    Set acc to acc + 100 / d.\n    Set i to i + 1.\nShow acc.\n", expect_out: "144", expect_err: false, oracle: "plain" },
    Prog { op: "Add", label: "add_identity_zero", src: "## Main\nShow 42 + 0.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "Add", label: "add_negative_operand", src: "## Main\nLet a be 5.\nLet b be 0 - 8.\nShow a + b.\n", expect_out: "-3", expect_err: false, oracle: "plain" },
    Prog { op: "Add", label: "add_overflow_to_bignum", src: "## Main\nLet a be 9223372036854775807.\nShow a + 1.\n", expect_out: "9223372036854775808", expect_err: false, oracle: "plain" },
    Prog { op: "Add", label: "add_sum_first_hundred", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 100:\n    Set acc to acc + i.\n    Set i to i + 1.\nShow acc.\n", expect_out: "5050", expect_err: false, oracle: "plain" },
    Prog { op: "AddAssign", label: "addassign_div_loop", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 30:\n    Set acc to acc + i / 7.\n    Set i to i + 1.\nShow acc.\n", expect_out: "54", expect_err: false, oracle: "plain" },
    Prog { op: "AddAssign", label: "addassign_identity_zero", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set acc to acc + (i - i).\n    Set i to i + 1.\nShow acc.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "AddAssign", label: "addassign_single_step", src: "## Main\nLet mutable acc be 41.\nLet mutable i be 1.\nWhile i is at most 1:\n    Set acc to acc + i.\n    Set i to i + 1.\nShow acc.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "AddAssign", label: "addassign_into_negative", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 10:\n    Set acc to acc - i.\n    Set i to i + 1.\nShow acc.\n", expect_out: "-55", expect_err: false, oracle: "plain" },
    Prog { op: "Sub", label: "sub_to_negative", src: "## Main\nShow 3 - 10.\n", expect_out: "-7", expect_err: false, oracle: "plain" },
    Prog { op: "Sub", label: "sub_identity_zero", src: "## Main\nShow 42 - 0.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "Sub", label: "sub_self_is_zero", src: "## Main\nLet a be 99.\nShow a - a.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Sub", label: "sub_underflow_to_bignum", src: "## Main\nLet a be 0 - 9223372036854775807.\nShow a - 2.\n", expect_out: "-9223372036854775809", expect_err: false, oracle: "plain" },
    Prog { op: "Sub", label: "sub_negative_minus_negative", src: "## Main\nLet a be 0 - 4.\nLet b be 0 - 10.\nShow a - b.\n", expect_out: "6", expect_err: false, oracle: "plain" },
    Prog { op: "Mul", label: "mul_overflow_single", src: "## Main\nShow 10000000000 * 10000000000.\n", expect_out: "100000000000000000000", expect_err: false, oracle: "plain" },
    Prog { op: "Mul", label: "mul_factorial_loop", src: "## Main\nLet mutable acc be 1.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set acc to acc * i.\n    Set i to i + 1.\nShow acc.\n", expect_out: "120", expect_err: false, oracle: "plain" },
    Prog { op: "Mul", label: "mul_by_zero", src: "## Main\nShow 999 * 0.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Mul", label: "mul_identity_one", src: "## Main\nShow 42 * 1.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "Mul", label: "mul_negative", src: "## Main\nLet a be 0 - 4.\nShow a * 3.\n", expect_out: "-12", expect_err: false, oracle: "plain" },
    Prog { op: "Mul", label: "mul_factorial_20_bignum", src: "## Main\nLet mutable acc be 1.\nLet mutable i be 1.\nWhile i is at most 20:\n    Set acc to acc * i.\n    Set i to i + 1.\nShow acc.\n", expect_out: "2432902008176640000", expect_err: false, oracle: "plain" },
    Prog { op: "Div", label: "div_by_zero_error", src: "## Main\nLet a be 5.\nLet b be 0.\nShow a / b.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Div", label: "div_zero_numerator", src: "## Main\nLet a be 0.\nLet b be 5.\nShow a / b.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Div", label: "div_identity_by_one", src: "## Main\nShow 42 / 1.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "Div", label: "div_negative_truncates_toward_zero", src: "## Main\nLet a be 0 - 7.\nShow a / 2.\n", expect_out: "-3", expect_err: false, oracle: "plain" },
    Prog { op: "ExactDiv", label: "exactdiv_seven_halves", src: "## Main\nLet x: Rational be 7 / 2.\nShow x.\n", expect_out: "7/2", expect_err: false, oracle: "plain" },
    Prog { op: "ExactDiv", label: "exactdiv_one_third", src: "## Main\nLet x: Rational be 1 / 3.\nShow x.\n", expect_out: "1/3", expect_err: false, oracle: "plain" },
    Prog { op: "ExactDiv", label: "exactdiv_by_zero_error", src: "## Main\nLet x: Rational be 1 / 0.\nShow x.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "ExactDiv", label: "exactdiv_reduces_to_whole", src: "## Main\nLet x: Rational be 8 / 4.\nShow x.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "ExactDiv", label: "exactdiv_negative_reduces", src: "## Main\nLet x: Rational be (0 - 6) / 4.\nShow x.\n", expect_out: "-3/2", expect_err: false, oracle: "plain" },
    Prog { op: "ExactDiv", label: "exactdiv_large_over_prime", src: "## Main\nLet x: Rational be 100 / 7.\nShow x.\n", expect_out: "100/7", expect_err: false, oracle: "plain" },
    Prog { op: "FloorDiv", label: "floordiv_positive", src: "## Main\nShow 7 // 2.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "FloorDiv", label: "floordiv_negative_dividend", src: "## Main\nLet a be 0 - 7.\nShow a // 2.\n", expect_out: "-4", expect_err: false, oracle: "plain" },
    Prog { op: "FloorDiv", label: "floordiv_by_zero_error", src: "## Main\nLet a be 7.\nLet b be 0.\nShow a // b.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "FloorDiv", label: "floordiv_negative_divisor", src: "## Main\nLet b be 0 - 2.\nShow 7 // b.\n", expect_out: "-4", expect_err: false, oracle: "plain" },
    Prog { op: "FloorDiv", label: "floordiv_both_negative", src: "## Main\nLet a be 0 - 7.\nLet b be 0 - 2.\nShow a // b.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "FloorDiv", label: "floordiv_left_assoc_chain", src: "## Main\nShow 10 // 3 // 2.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "FloorDiv", label: "floordiv_identity_by_one", src: "## Main\nShow 8 // 1.\n", expect_out: "8", expect_err: false, oracle: "plain" },
    Prog { op: "Mod", label: "mod_by_zero_error", src: "## Main\nLet a be 5.\nLet b be 0.\nShow a mod b.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Mod", label: "mod_positive", src: "## Main\nLet a be 17.\nLet b be 5.\nShow a mod b.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "Mod", label: "mod_loop", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 5:\n    Let d be i + 1.\n    Set acc to acc + 100 mod d.\n    Set i to i + 1.\nShow acc.\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "Mod", label: "mod_evenly_divides_zero_remainder", src: "## Main\nLet a be 10.\nLet b be 5.\nShow a mod b.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Mod", label: "mod_negative_dividend_keeps_sign", src: "## Main\nLet a be 0 - 7.\nShow a mod 2.\n", expect_out: "-1", expect_err: false, oracle: "plain" },
    Prog { op: "DivPow2", label: "divpow2_by_eight_loop", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 16:\n    Set acc to acc + i / 8.\n    Set i to i + 1.\nShow acc.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "DivPow2", label: "divpow2_by_two_loop", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 10:\n    Set acc to acc + i / 2.\n    Set i to i + 1.\nShow acc.\n", expect_out: "25", expect_err: false, oracle: "plain" },
    Prog { op: "DivPow2", label: "divpow2_by_four_loop", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 20:\n    Set acc to acc + i / 4.\n    Set i to i + 1.\nShow acc.\n", expect_out: "45", expect_err: false, oracle: "plain" },
    Prog { op: "DivPow2", label: "divpow2_by_sixteen_loop", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 32:\n    Set acc to acc + i / 16.\n    Set i to i + 1.\nShow acc.\n", expect_out: "18", expect_err: false, oracle: "plain" },
    Prog { op: "DivPow2", label: "divpow2_all_below_divisor_zero", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 7:\n    Set acc to acc + i / 8.\n    Set i to i + 1.\nShow acc.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "DivPow2", label: "divpow2_first_hit_at_boundary", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 8:\n    Set acc to acc + i / 8.\n    Set i to i + 1.\nShow acc.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "MagicDivU", label: "magicdivu_mod_by_thousand", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 30:\n    Set acc to acc + i mod 1000.\n    Set i to i + 1.\nShow acc.\n", expect_out: "465", expect_err: false, oracle: "plain" },
    Prog { op: "MagicDivU", label: "magicdivu_div_by_three", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 30:\n    Set acc to acc + i / 3.\n    Set i to i + 1.\nShow acc.\n", expect_out: "145", expect_err: false, oracle: "plain" },
    Prog { op: "MagicDivU", label: "magicdivu_mod_by_seven", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 30:\n    Set acc to acc + i mod 7.\n    Set i to i + 1.\nShow acc.\n", expect_out: "87", expect_err: false, oracle: "plain" },
    Prog { op: "MagicDivU", label: "magicdivu_div_all_below_divisor", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 6:\n    Set acc to acc + i / 7.\n    Set i to i + 1.\nShow acc.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "MagicDivU", label: "magicdivu_mod_small_modulus", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 10:\n    Set acc to acc + i mod 3.\n    Set i to i + 1.\nShow acc.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "Pow", label: "pow_2_10", src: "## Main\nShow 2 ** 10.\n", expect_out: "1024", expect_err: false, oracle: "plain" },
    Prog { op: "Pow", label: "pow_2_100_bignum", src: "## Main\nShow 2 ** 100.\n", expect_out: "1267650600228229401496703205376", expect_err: false, oracle: "plain" },
    Prog { op: "Pow", label: "pow_exponent_zero", src: "## Main\nShow 5 ** 0.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "Pow", label: "pow_negative_exponent_error", src: "## Main\nShow 2 ** -1.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Pow", label: "pow_negative_base_odd", src: "## Main\nLet b be 0 - 3.\nShow b ** 3.\n", expect_out: "-27", expect_err: false, oracle: "plain" },
    Prog { op: "Pow", label: "pow_zero_to_zero_is_one", src: "## Main\nShow 0 ** 0.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "Pow", label: "pow_ten_to_twenty_bignum", src: "## Main\nShow 10 ** 20.\n", expect_out: "100000000000000000000", expect_err: false, oracle: "plain" },
    Prog { op: "Lt", label: "lt_true", src: "## Main\n    Show 2 is less than 5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Lt", label: "lt_equal_is_false", src: "## Main\n    Show 5 is less than 5.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Lt", label: "lt_negatives", src: "## Main\n    Show -5 is less than -2.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Lt", label: "lt_equal_floats_false", src: "## Main\n    Let a be 1.5.\n    Let b be 1.5.\n    Show a is less than b.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Lt", label: "lt_zero_and_one", src: "## Main\n    Show 0 is less than 1.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Lt", label: "lt_neg_one_below_zero", src: "## Main\n    Show -1 is less than 0.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Lt", label: "lt_int_below_bignum", src: "## Main\n    Let big be 2 ** 100.\n    Show 5 is less than big.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Lt", label: "lt_drives_a_counting_loop", src: "## Main\n    Let i be 0.\n    While i is less than 5:\n        Show i.\n        Set i to i + 1.\n", expect_out: "0\n1\n2\n3\n4", expect_err: false, oracle: "plain" },
    Prog { op: "Gt", label: "gt_true", src: "## Main\n    Show 5 is greater than 2.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Gt", label: "gt_equal_is_false", src: "## Main\n    Show 5 is greater than 5.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Gt", label: "gt_negatives", src: "## Main\n    Show -2 is greater than -5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Gt", label: "gt_zero_above_negative", src: "## Main\n    Show 0 is greater than -1.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Gt", label: "gt_zero_not_above_zero", src: "## Main\n    Show 0 is greater than 0.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Gt", label: "gt_bignum_above_int", src: "## Main\n    Let big be 2 ** 100.\n    Show big is greater than 5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "LtEq", label: "lte_equal_true", src: "## Main\n    Show 5 is at most 5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "LtEq", label: "lte_over_false", src: "## Main\n    Show 6 is at most 5.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "LtEq", label: "lte_worded_equal_true", src: "## Main\n    Show 3 is less than or equal to 3.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "LtEq", label: "lte_negatives_equal_true", src: "## Main\n    Show -5 is at most -5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "LtEq", label: "lte_zero_equal_true", src: "## Main\n    Show 0 is at most 0.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "LtEq", label: "lte_negative_below_positive", src: "## Main\n    Show -3 is less than or equal to 2.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "LtEq", label: "lte_one_over_zero_false", src: "## Main\n    Show 1 is at most 0.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "GtEq", label: "gte_equal_true", src: "## Main\n    Show 5 is at least 5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "GtEq", label: "gte_under_false", src: "## Main\n    Show 4 is at least 5.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "GtEq", label: "gte_worded_equal_true", src: "## Main\n    Show 3 is greater than or equal to 3.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "GtEq", label: "gte_negatives_true", src: "## Main\n    Show -2 is at least -5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "GtEq", label: "gte_zero_equal_true", src: "## Main\n    Show 0 is at least 0.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "GtEq", label: "gte_positive_above_negative", src: "## Main\n    Show 2 is greater than or equal to -3.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "GtEq", label: "gte_negative_below_positive_false", src: "## Main\n    Show -3 is at least 2.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_worded_true", src: "## Main\n    Show 5 is equal to 5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_bare_number_true", src: "## Main\n    Let x be 3.\n    Show x is 3.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_negatives_true", src: "## Main\n    Show -5 is equal to -5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_different_false", src: "## Main\n    Show 2 is equal to 5.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_zero_true", src: "## Main\n    Show 0 is equal to 0.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_int_equals_float_exact", src: "## Main\n    Let a be 1.\n    Let b be 1.0.\n    Show a is equal to b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_text_equal_true", src: "## Main\n    Let a be \"hi\".\n    Let b be \"hi\".\n    Show a is equal to b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_bool_equal_true", src: "## Main\n    Let a be true.\n    Let b be true.\n    Show a is equal to b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_bignum_identity_true", src: "## Main\n    Let a be 2 ** 100.\n    Let b be 2 ** 100.\n    Show a is equal to b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Eq", label: "eq_int_never_equals_bignum", src: "## Main\n    Let big be 2 ** 100.\n    Show 5 is equal to big.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "NotEq", label: "neq_var_true", src: "## Main\n    Let x be 3.\n    Show x is not 5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "NotEq", label: "neq_var_equal_false", src: "## Main\n    Let x be 5.\n    Show x is not 5.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "NotEq", label: "neq_negatives_true", src: "## Main\n    Let x be -3.\n    Show x is not -5.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "NotEq", label: "neq_zero_equal_false", src: "## Main\n    Let x be 0.\n    Show x is not 0.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "NotEq", label: "neq_literal_different_true", src: "## Main\n    Show 5 is not 3.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "NotEq", label: "neq_text_different_true", src: "## Main\n    Let a be \"hi\".\n    Show a is not \"bye\".\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "NotEq", label: "neq_negative_equal_false", src: "## Main\n    Let x be -3.\n    Show x is not -3.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "ApproxEq", label: "approx_equal_true", src: "## Main\n    Let a be 0.1.\n    Let b be 0.1.\n    Show a is approximately b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "ApproxEq", label: "approx_worded_equal_true", src: "## Main\n    Let a be 1.0.\n    Let b be 1.0.\n    Show a is approximately equal to b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "ApproxEq", label: "approx_far_apart_false", src: "## Main\n    Let a be 0.1.\n    Let b be 9.9.\n    Show a is approximately b.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "ApproxEq", label: "approx_float_drift_within_tolerance_true", src: "## Main\n    Let a be 0.1 + 0.2.\n    Let b be 0.3.\n    Show a is approximately b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "ApproxEq", label: "approx_zero_true", src: "## Main\n    Let a be 0.0.\n    Let b be 0.0.\n    Show a is approximately b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "ApproxEq", label: "approx_int_operands_coerce_true", src: "## Main\n    Let a be 5.\n    Let b be 5.\n    Show a is approximately b.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "ApproxEq", label: "approx_clearly_different_false", src: "## Main\n    Let a be 1.0.\n    Let b be 2.0.\n    Show a is approximately b.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "ApproxEq", label: "approx_text_operands_error", src: "## Main\n    Let a be \"hi\".\n    Let b be \"hi\".\n    Show a is approximately b.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Not", label: "not_true_flag_false", src: "## Main\n    Let flag be 3 is less than 5.\n    Show not flag.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Not", label: "not_false_flag_true", src: "## Main\n    Let flag be 5 is less than 3.\n    Show not flag.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Not", label: "not_not_is_identity", src: "## Main\n    Let flag be 3 is less than 5.\n    Show not not flag.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Not", label: "not_not_preserves_false", src: "## Main\n    Let flag be 5 is less than 3.\n    Show not not flag.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Not", label: "not_zero_int_is_true", src: "## Main\n    Let flag be 0.\n    Show not flag.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "Not", label: "not_nonzero_int_is_false", src: "## Main\n    Let flag be 7.\n    Show not flag.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "Not", label: "not_empty_text_is_true", src: "## Main\n    Let flag be \"\".\n    Show not flag.\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "BitAnd", label: "AND with zero mask clears all bits", src: "## Main\nLet a be 255.\nLet b be 0.\nLet c be a & b.\nShow c.", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "BitAnd", label: "AND identical operands is identity", src: "## Main\nLet a be 255.\nLet b be 255.\nLet c be a & b.\nShow c.", expect_out: "255", expect_err: false, oracle: "plain" },
    Prog { op: "BitAnd", label: "AND with -1 (all ones) passes value through", src: "## Main\nLet a be -1.\nLet b be 12345.\nLet c be a & b.\nShow c.", expect_out: "12345", expect_err: false, oracle: "plain" },
    Prog { op: "BitAnd", label: "AND idempotence a & a equals a on a large value", src: "## Main\nLet a be 987654321.\nLet c be a & a.\nShow c.", expect_out: "987654321", expect_err: false, oracle: "plain" },
    Prog { op: "BitAnd", label: "AND single-bit mask isolates that bit (boundary bit 8)", src: "## Main\nLet a be 1023.\nLet b be 256.\nLet c be a & b.\nShow c.", expect_out: "256", expect_err: false, oracle: "plain" },
    Prog { op: "BitAnd", label: "AND on Bool operands (type variant, true & false)", src: "## Main\nLet a be true.\nLet b be false.\nLet c be a & b.\nShow c.", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "BitAnd", label: "AND accumulated over a loop drops to zero (many)", src: "## Main\nLet mutable acc be -1.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set acc to acc & i.\n    Set i to i + 1.\nShow acc.", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "BitOr", label: "OR with zero returns the other operand", src: "## Main\nLet a be 0.\nLet b be 170.\nLet c be a | b.\nShow c.", expect_out: "170", expect_err: false, oracle: "plain" },
    Prog { op: "BitOr", label: "OR of complementary nibbles fills the byte", src: "## Main\nLet a be 240.\nLet b be 15.\nLet c be a | b.\nShow c.", expect_out: "255", expect_err: false, oracle: "plain" },
    Prog { op: "BitOr", label: "OR with -1 (all ones) saturates to -1", src: "## Main\nLet a be -1.\nLet b be 0.\nLet c be a | b.\nShow c.", expect_out: "-1", expect_err: false, oracle: "plain" },
    Prog { op: "BitOr", label: "OR idempotence a | a equals a on a large value", src: "## Main\nLet a be 987654321.\nLet c be a | a.\nShow c.", expect_out: "987654321", expect_err: false, oracle: "plain" },
    Prog { op: "BitOr", label: "OR on Bool operands (type variant, false | true)", src: "## Main\nLet a be false.\nLet b be true.\nLet c be a | b.\nShow c.", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "BitOr", label: "OR accumulated over a loop merges low bits (many)", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set acc to acc | i.\n    Set i to i + 1.\nShow acc.", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "BitXor", label: "XOR of a value with itself is zero", src: "## Main\nLet a be 4242.\nLet c be a xor a.\nShow c.", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "BitXor", label: "XOR with caret operator on two small ints", src: "## Main\nLet a be 12.\nLet b be 10.\nLet c be a ^ b.\nShow c.", expect_out: "6", expect_err: false, oracle: "plain" },
    Prog { op: "BitXor", label: "NOT of zero is -1 (tilde lowers to xor -1)", src: "## Main\nLet a be 0.\nLet c be ~a.\nShow c.", expect_out: "-1", expect_err: false, oracle: "plain" },
    Prog { op: "BitXor", label: "NOT of 5 is -6 (two's complement)", src: "## Main\nLet a be 5.\nLet c be ~a.\nShow c.", expect_out: "-6", expect_err: false, oracle: "plain" },
    Prog { op: "BitXor", label: "double NOT is identity (~~a == a)", src: "## Main\nLet a be 42.\nLet c be ~~a.\nShow c.", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "BitXor", label: "NOT of -1 is 0 (all ones flips to zero)", src: "## Main\nLet a be -1.\nLet c be ~a.\nShow c.", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "BitXor", label: "XOR on Bool operands (type variant, true xor true)", src: "## Main\nLet a be true.\nLet b be true.\nLet c be a xor b.\nShow c.", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "BitXor", label: "XOR accumulated 1..7 cancels to zero (many)", src: "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 7:\n    Set acc to acc xor i.\n    Set i to i + 1.\nShow acc.", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Shl", label: "shift left by zero is identity", src: "## Main\nLet a be 5.\nLet b be 0.\nLet c be a shifted left by b.\nShow c.", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "Shl", label: "shift left by 2 multiplies by 4", src: "## Main\nLet a be 5.\nLet b be 2.\nLet c be a shifted left by b.\nShow c.", expect_out: "20", expect_err: false, oracle: "plain" },
    Prog { op: "Shl", label: "shift 1 left by 63 reaches the sign bit (i64::MIN)", src: "## Main\nLet a be 1.\nLet b be 63.\nLet c be a shifted left by b.\nShow c.", expect_out: "-9223372036854775808", expect_err: false, oracle: "plain" },
    Prog { op: "Shl", label: "shift count masked mod 64: 1 left by 64 wraps to count 0", src: "## Main\nLet a be 1.\nLet b be 64.\nLet c be a shifted left by b.\nShow c.", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "Shl", label: "shift 255 left by 8 moves into the next byte", src: "## Main\nLet a be 255.\nLet b be 8.\nLet c be a shifted left by b.\nShow c.", expect_out: "65280", expect_err: false, oracle: "plain" },
    Prog { op: "Shl", label: "repeated shift-left-by-1 in a loop is *32 (many)", src: "## Main\nLet mutable v be 1.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set v to v shifted left by 1.\n    Set i to i + 1.\nShow v.", expect_out: "32", expect_err: false, oracle: "plain" },
    Prog { op: "Shr", label: "shift right by zero is identity", src: "## Main\nLet a be 20.\nLet b be 0.\nLet c be a shifted right by b.\nShow c.", expect_out: "20", expect_err: false, oracle: "plain" },
    Prog { op: "Shr", label: "shift right by 2 divides by 4", src: "## Main\nLet a be 20.\nLet b be 2.\nLet c be a shifted right by b.\nShow c.", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "Shr", label: "arithmetic shift right preserves sign of a negative", src: "## Main\nLet a be -16.\nLet b be 2.\nLet c be a shifted right by b.\nShow c.", expect_out: "-4", expect_err: false, oracle: "plain" },
    Prog { op: "Shr", label: "shift i64::MIN right by 63 fills with sign bit (-1)", src: "## Main\nLet a be -9223372036854775808.\nLet b be 63.\nLet c be a shifted right by b.\nShow c.", expect_out: "-1", expect_err: false, oracle: "plain" },
    Prog { op: "Shr", label: "shift count masked mod 64: 8 right by 64 wraps to count 0", src: "## Main\nLet a be 8.\nLet b be 64.\nLet c be a shifted right by b.\nShow c.", expect_out: "8", expect_err: false, oracle: "plain" },
    Prog { op: "Shr", label: "shift -1 right by 1 stays -1 (all-ones sign fill)", src: "## Main\nLet a be -1.\nLet b be 1.\nLet c be a shifted right by b.\nShow c.", expect_out: "-1", expect_err: false, oracle: "plain" },
    Prog { op: "Shr", label: "repeated shift-right-by-1 in a loop is /32 (many)", src: "## Main\nLet mutable v be 1024.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set v to v shifted right by 1.\n    Set i to i + 1.\nShow v.", expect_out: "32", expect_err: false, oracle: "plain" },
    Prog { op: "Jump", label: "if-false-takes-otherwise", src: "## Main\n    Let x be 3.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "Jump", label: "while-counts-to-three", src: "## Main\n    Let i be 0.\n    While i is less than 3:\n        Show i.\n        Set i to i + 1.\n", expect_out: "0\n1\n2", expect_err: false, oracle: "plain" },
    Prog { op: "Jump", label: "nested-if-deep-true-branch", src: "## Main\n    Let x be 7.\n    If x is greater than 5:\n        If x is greater than 6:\n            Show 100.\n        Otherwise:\n            Show 50.\n    Otherwise:\n        Show 0.\n", expect_out: "100", expect_err: false, oracle: "plain" },
    Prog { op: "Jump", label: "while-zero-iterations-empty-body", src: "## Main\n    Let i be 5.\n    While i is less than 3:\n        Show i.\n        Set i to i + 1.\n    Show 99.\n", expect_out: "99", expect_err: false, oracle: "plain" },
    Prog { op: "Jump", label: "while-exactly-one-iteration", src: "## Main\n    Let i be 0.\n    While i is less than 1:\n        Show i.\n        Set i to i + 1.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Jump", label: "if-boundary-equal-not-greater", src: "## Main\n    Let x be 5.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "Jump", label: "while-many-thousand-sum", src: "## Main\n    Let i be 0.\n    Let total be 0.\n    While i is less than 1000:\n        Set total to total + i.\n        Set i to i + 1.\n    Show total.\n", expect_out: "499500", expect_err: false, oracle: "plain" },
    Prog { op: "Jump", label: "if-negative-value-branch", src: "## Main\n    Let x be -4.\n    If x is less than 0:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfFalse", label: "or-both-false-otherwise", src: "## Main\n    Let a be 0.\n    Let b be 0.\n    If a is greater than 0 or b is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfFalse", label: "and-one-false-short-circuits-otherwise", src: "## Main\n    Let a be 1.\n    Let b be 0.\n    If a is greater than 0 and b is greater than 0:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfFalse", label: "while-guard-false-immediately", src: "## Main\n    Let i be 10.\n    While i is less than 0:\n        Show i.\n        Set i to i - 1.\n    Show 0.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfFalse", label: "if-even-guard-false-on-odd", src: "## Main\n    Let x be 7.\n    If x is even:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfFalse", label: "if-not-equal-guard-false", src: "## Main\n    Let x be 5.\n    If x is not 5:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfTrue", label: "or-first-true-takes-branch", src: "## Main\n    Let a be 1.\n    Let b be 0.\n    If a is greater than 0 or b is greater than 0:\n        Show 1.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfTrue", label: "and-both-true-takes-branch", src: "## Main\n    Let a be 1.\n    Let b be 2.\n    If a is greater than 0 and b is greater than 0:\n        Show 1.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfTrue", label: "or-second-true", src: "## Main\n    Let a be 0.\n    Let b be 3.\n    If a is greater than 0 or b is greater than 0:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfTrue", label: "or-both-true", src: "## Main\n    Let a be 5.\n    Let b be 9.\n    If a is greater than 0 or b is greater than 0:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfTrue", label: "or-with-negative-second-true", src: "## Main\n    Let a be -1.\n    Let b be 4.\n    If a is greater than 0 or b is greater than 0:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "JumpIfTrue", label: "or-between-and-guard-true", src: "## Main\n    Let x be 5.\n    If x is between 1 and 10 or x is greater than 100:\n        Show 1.\n    Otherwise:\n        Show 2.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "Call", label: "single-arg-square", src: "## To sq (x: Int) -> Int:\n    Return x * x.\n## Main\n    Show sq(9).\n", expect_out: "81", expect_err: false, oracle: "plain" },
    Prog { op: "Call", label: "recursive-fib-ten", src: "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n", expect_out: "55", expect_err: false, oracle: "plain" },
    Prog { op: "Call", label: "nested-call-two-args", src: "## To sq (x: Int) -> Int:\n    Return x * x.\n## To sumsq (a: Int) (b: Int) -> Int:\n    Return sq(a) + sq(b).\n## Main\n    Show sumsq(3, 4).\n", expect_out: "25", expect_err: false, oracle: "plain" },
    Prog { op: "Call", label: "call-with-zero-arg", src: "## To sq (x: Int) -> Int:\n    Return x * x.\n## Main\n    Show sq(0).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Call", label: "call-with-negative-arg", src: "## To sq (x: Int) -> Int:\n    Return x * x.\n## Main\n    Show sq(-6).\n", expect_out: "36", expect_err: false, oracle: "plain" },
    Prog { op: "Call", label: "recursion-base-case-only-fib-zero", src: "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(0).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Call", label: "recursion-deep-fib-twenty", src: "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(20).\n", expect_out: "6765", expect_err: false, oracle: "plain" },
    Prog { op: "Call", label: "call-divide-by-zero-in-callee-errors", src: "## To div (a: Int) (b: Int) -> Int:\n    Return a / b.\n## Main\n    Show div(10, 0).\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "CallBuiltin", label: "sqrt-perfect-square", src: "## Main\n    Show sqrt(9.0).\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "CallBuiltin", label: "abs-of-negative", src: "## Main\n    Show abs(-4).\n", expect_out: "4", expect_err: false, oracle: "plain" },
    Prog { op: "CallBuiltin", label: "max-of-two", src: "## Main\n    Show max(3, 7).\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "CallBuiltin", label: "format-interpolation", src: "## Main\n    Let n be 5.\n    Show format(\"x={n}\").\n", expect_out: "x=5", expect_err: false, oracle: "plain" },
    Prog { op: "CallBuiltin", label: "abs-of-zero", src: "## Main\n    Show abs(0).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "CallBuiltin", label: "sqrt-of-zero", src: "## Main\n    Show sqrt(0.0).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "CallBuiltin", label: "max-with-negatives", src: "## Main\n    Show max(-3, -7).\n", expect_out: "-3", expect_err: false, oracle: "plain" },
    Prog { op: "CallBuiltin", label: "abs-of-large-value", src: "## Main\n    Show abs(-1000000).\n", expect_out: "1000000", expect_err: false, oracle: "plain" },
    Prog { op: "GlobalGet", label: "read-outer-let-in-fn", src: "## To getbase -> Int:\n    Return base.\n## Main\n    Let base be 10.\n    Show getbase().\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "GlobalGet", label: "mutate-global-across-calls", src: "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let counter be 0.\n    bump().\n    bump().\n    Show counter.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "GlobalGet", label: "read-and-accumulate-global", src: "## To step -> Int:\n    Set total to total + step_amount.\n    Return total.\n## Main\n    Let total be 0.\n    Let step_amount be 5.\n    Show step().\n    Show step().\n", expect_out: "5\n10", expect_err: false, oracle: "plain" },
    Prog { op: "GlobalGet", label: "read-global-zero", src: "## To getbase -> Int:\n    Return base.\n## Main\n    Let base be 0.\n    Show getbase().\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "GlobalGet", label: "read-global-negative", src: "## To getbase -> Int:\n    Return base.\n## Main\n    Let base be -7.\n    Show getbase().\n", expect_out: "-7", expect_err: false, oracle: "plain" },
    Prog { op: "GlobalGet", label: "read-global-in-loop-many", src: "## To getstep -> Int:\n    Return step_amount.\n## Main\n    Let step_amount be 3.\n    Let i be 0.\n    Let total be 0.\n    While i is less than 4:\n        Set total to total + getstep().\n        Set i to i + 1.\n    Show total.\n", expect_out: "12", expect_err: false, oracle: "plain" },
    Prog { op: "GlobalSet", label: "decrement-global-into-negative", src: "## To drop:\n    Set counter to counter - 1.\n## Main\n    Let counter be 0.\n    drop().\n    drop().\n    Show counter.\n", expect_out: "-2", expect_err: false, oracle: "plain" },
    Prog { op: "GlobalSet", label: "set-global-many-times-in-loop", src: "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let counter be 0.\n    Let i be 0.\n    While i is less than 100:\n        bump().\n        Set i to i + 1.\n    Show counter.\n", expect_out: "100", expect_err: false, oracle: "plain" },
    Prog { op: "Return", label: "early-return-clamp", src: "## To clamp (x: Int) -> Int:\n    If x is greater than 5:\n        Return 5.\n    Return x.\n## Main\n    Show clamp(9).\n    Show clamp(3).\n", expect_out: "5\n3", expect_err: false, oracle: "plain" },
    Prog { op: "Return", label: "early-return-on-boundary-equal", src: "## To clamp (x: Int) -> Int:\n    If x is greater than 5:\n        Return 5.\n    Return x.\n## Main\n    Show clamp(5).\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "Return", label: "return-negative", src: "## To neg (x: Int) -> Int:\n    Return 0 - x.\n## Main\n    Show neg(8).\n", expect_out: "-8", expect_err: false, oracle: "plain" },
    Prog { op: "Return", label: "return-divide-by-zero-errors", src: "## To half (x: Int) -> Int:\n    Return x / 0.\n## Main\n    Show half(10).\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "ReturnNothing", label: "show-then-bare-return", src: "## To greet (n: Int):\n    Show n.\n    Return.\n## Main\n    greet(7).\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "ReturnNothing", label: "early-bare-return-guards-rest", src: "## To maybe (n: Int):\n    If n is greater than 5:\n        Show 99.\n        Return.\n    Show 1.\n## Main\n    maybe(9).\n    maybe(2).\n", expect_out: "99\n1", expect_err: false, oracle: "plain" },
    Prog { op: "ReturnNothing", label: "implicit-return-no-keyword", src: "## To p (n: Int):\n    Show n.\n## Main\n    p(5).\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "ReturnNothing", label: "bare-return-with-zero-arg", src: "## To greet (n: Int):\n    Show n.\n    Return.\n## Main\n    greet(0).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "ReturnNothing", label: "bare-return-with-negative-arg", src: "## To greet (n: Int):\n    Show n.\n    Return.\n## Main\n    greet(-3).\n", expect_out: "-3", expect_err: false, oracle: "plain" },
    Prog { op: "ReturnNothing", label: "void-fn-called-many-times", src: "## To p (n: Int):\n    Show n.\n## Main\n    Let i be 0.\n    While i is less than 3:\n        p(i).\n        Set i to i + 1.\n", expect_out: "0\n1\n2", expect_err: false, oracle: "plain" },
    Prog { op: "Show", label: "show-single-int", src: "## Main\n    Show 42.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "Show", label: "show-three-ints-newline-joined", src: "## Main\n    Show 1.\n    Show 2.\n    Show 3.\n", expect_out: "1\n2\n3", expect_err: false, oracle: "plain" },
    Prog { op: "Show", label: "show-zero", src: "## Main\n    Show 0.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Show", label: "show-negative-int", src: "## Main\n    Show -17.\n", expect_out: "-17", expect_err: false, oracle: "plain" },
    Prog { op: "Show", label: "show-text-literal", src: "## Main\n    Show \"hello\".\n", expect_out: "hello", expect_err: false, oracle: "plain" },
    Prog { op: "Show", label: "show-bool-from-comparison", src: "## Main\n    Let x be 5.\n    If x is greater than 3:\n        Show \"yes\".\n    Otherwise:\n        Show \"no\".\n", expect_out: "yes", expect_err: false, oracle: "plain" },
    Prog { op: "Show", label: "show-float-whole-no-decimal", src: "## Main\n    Show sqrt(16.0).\n", expect_out: "4", expect_err: false, oracle: "plain" },
    Prog { op: "Show", label: "show-divide-by-zero-produces-no-output", src: "## Main\n    Show 1 / 0.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Halt", label: "halt-after-empty-main-no-output", src: "## Main\n    Let x be 5.\n", expect_out: "", expect_err: false, oracle: "plain" },
    Prog { op: "Halt", label: "halt-after-loop-completes", src: "## Main\n    Let i be 0.\n    While i is less than 3:\n        Set i to i + 1.\n    Show i.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "Halt", label: "halt-after-void-call", src: "## To p (n: Int):\n    Show n.\n## Main\n    p(9).\n", expect_out: "9", expect_err: false, oracle: "plain" },
    Prog { op: "Halt", label: "halt-after-many-shows", src: "## Main\n    Let i be 0.\n    While i is less than 5:\n        Show i.\n        Set i to i + 1.\n    Show 100.\n", expect_out: "0\n1\n2\n3\n4\n100", expect_err: false, oracle: "plain" },
    Prog { op: "FailWith", label: "assert_fails_traps", src: "## Main\n    Show 1.\n    Assert that 1 is 2.\n    Show 2.\n", expect_out: "1", expect_err: true, oracle: "plain" },
    Prog { op: "FailWith", label: "assert_passes_no_trap", src: "## Main\n    Show 1.\n    Assert that 1 is 1.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "plain" },
    Prog { op: "FailWith", label: "inspect_no_arm_traps", src: "## A Color is one of:\n    A Red.\n    A Green.\n    A Blue.\n\n## To name (c: Color) -> Text:\n    Inspect c:\n        When Red:\n            Return \"red\".\n        When Green:\n            Return \"green\".\n\n## Main\n    Let c be a new Blue.\n    Show name(c).\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "FailWith", label: "set_undefined_var_traps", src: "## Main\n    Show 1.\n    Set x to 5.\n    Show 2.\n", expect_out: "1", expect_err: true, oracle: "plain" },
    Prog { op: "FailWith", label: "assert_false_gt_traps", src: "## Main\n    Show 1.\n    Assert that 5 is greater than 10.\n    Show 2.\n", expect_out: "1", expect_err: true, oracle: "plain" },
    Prog { op: "FailWith", label: "assert_true_gt_no_trap", src: "## Main\n    Show 1.\n    Assert that 5 is greater than 2.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "plain" },
    Prog { op: "FailWith", label: "assert_first_stmt_false_no_output", src: "## Main\n    Assert that 0 is not 0.\n    Show 1.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "FailWith", label: "assert_negative_eq_boundary_passes", src: "## Main\n    Show 1.\n    Assert that 0 is 0.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "plain" },
    Prog { op: "FailWith", label: "inspect_no_arm_second_variant_traps", src: "## A Color is one of:\n    A Red.\n    A Green.\n    A Blue.\n\n## To name (c: Color) -> Text:\n    Inspect c:\n        When Red:\n            Return \"red\".\n\n## Main\n    Let c be a new Green.\n    Show name(c).\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "FailWith", label: "assert_passes_then_shows_many", src: "## Main\n    Show 1.\n    Assert that 1 is 1.\n    Show 2.\n    Assert that 3 is greater than 2.\n    Show 3.\n", expect_out: "1\n2\n3", expect_err: false, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "in_bounds_sum_three", src: "## To total (xs: Seq of Int) (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than n:\n        Set acc to acc + item i of xs.\n        Set i to i + 1.\n    Return acc.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 10 to a.\n    Push 20 to a.\n    Push 30 to a.\n    Show total(a, 3).\n", expect_out: "30", expect_err: false, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "out_of_bounds_walks_off_end", src: "## To total (xs: Seq of Int) (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than n:\n        Set acc to acc + item i of xs.\n        Set i to i + 1.\n    Return acc.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 10 to a.\n    Push 20 to a.\n    Show total(a, 99).\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "in_bounds_double_three", src: "## To dbl (xs: Seq of Int) (n: Int) -> Seq of Int:\n    Let mutable i be 1.\n    While i is less than n:\n        Set item i of xs to (item i of xs) * 2.\n        Set i to i + 1.\n    Return xs.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 5 to a.\n    Push 6 to a.\n    Push 7 to a.\n    Show dbl(a, 3).\n", expect_out: "[10, 12, 7]", expect_err: false, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "n_one_loop_never_runs", src: "## To total (xs: Seq of Int) (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than n:\n        Set acc to acc + item i of xs.\n        Set i to i + 1.\n    Return acc.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 42 to a.\n    Push 7 to a.\n    Push 9 to a.\n    Show total(a, 1).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "single_element_first_index", src: "## To total (xs: Seq of Int) (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than n:\n        Set acc to acc + item i of xs.\n        Set i to i + 1.\n    Return acc.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 42 to a.\n    Show total(a, 2).\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "empty_seq_no_iteration_safe", src: "## To total (xs: Seq of Int) (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than n:\n        Set acc to acc + item i of xs.\n        Set i to i + 1.\n    Return acc.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Show total(a, 1).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "empty_seq_first_index_traps", src: "## To total (xs: Seq of Int) (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than n:\n        Set acc to acc + item i of xs.\n        Set i to i + 1.\n    Return acc.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Show total(a, 2).\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "double_single_element_in_bounds", src: "## To dbl (xs: Seq of Int) (n: Int) -> Seq of Int:\n    Let mutable i be 1.\n    While i is less than n:\n        Set item i of xs to (item i of xs) * 2.\n        Set i to i + 1.\n    Return xs.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 3 to a.\n    Push 4 to a.\n    Show dbl(a, 2).\n", expect_out: "[6, 4]", expect_err: false, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "many_elements_full_sum", src: "## To total (xs: Seq of Int) (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than n:\n        Set acc to acc + item i of xs.\n        Set i to i + 1.\n    Return acc.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Push 2 to a.\n    Push 3 to a.\n    Push 4 to a.\n    Push 5 to a.\n    Push 6 to a.\n    Push 7 to a.\n    Push 8 to a.\n    Push 9 to a.\n    Push 10 to a.\n    Show total(a, 11).\n", expect_out: "55", expect_err: false, oracle: "plain" },
    Prog { op: "RegionBoundsGuard", label: "many_elements_one_past_end_traps", src: "## To total (xs: Seq of Int) (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than n:\n        Set acc to acc + item i of xs.\n        Set i to i + 1.\n    Return acc.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Push 2 to a.\n    Push 3 to a.\n    Show total(a, 5).\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "NewList", label: "literal-index-2", src: "## Main\nLet xs be [10, 20, 30].\nShow item 2 of xs.\n", expect_out: "20", expect_err: false, oracle: "plain" },
    Prog { op: "NewList", label: "single-element", src: "## Main\nLet xs be [42].\nShow item 1 of xs.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "NewList", label: "empty-length", src: "## Main\nLet xs be [].\nShow length of xs.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewList", label: "first-element", src: "## Main\nLet xs be [10, 20, 30].\nShow item 1 of xs.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "NewList", label: "last-via-negative", src: "## Main\nLet xs be [10, 20, 30].\nShow item -1 of xs.\n", expect_out: "30", expect_err: false, oracle: "plain" },
    Prog { op: "NewList", label: "large-literal-list-length", src: "## Main\nLet xs be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].\nShow length of xs.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "NewList", label: "text-element-list", src: "## Main\nLet xs be [\"a\", \"b\", \"c\"].\nShow item 3 of xs.\n", expect_out: "c", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyList", label: "empty-seq-length", src: "## Main\nLet mutable xs be a new Seq of Text.\nShow length of xs.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyList", label: "push-one-length", src: "## Main\nLet mutable xs be a new Seq of Text.\nPush \"a\" to xs.\nShow length of xs.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyList", label: "new-seq-int-empty", src: "## Main\nLet mutable xs be a new Seq of Int.\nShow length of xs.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyList", label: "new-seq-int-push-show", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 7 to xs.\nShow item 1 of xs.\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyList", label: "empty-seq-pop-stops", src: "## Main\nLet mutable xs be a new Seq of Int.\nShow 1.\nPop from xs into v.\nShow v.\n", expect_out: "1", expect_err: true, oracle: "plain" },
    Prog { op: "NewEmptyList", label: "empty-seq-push-pop-length", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 3 to xs.\nPop from xs into v.\nShow v.\nShow length of xs.\n", expect_out: "3\n0", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyList", label: "new-seq-real", src: "## Main\nLet mutable xs be a new Seq of Real.\nShow length of xs.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyListWithCapacity", label: "dynamic-capacity-seq", src: "## Main\nLet n be 17.\nLet mutable xs be a new Seq of Int with capacity n.\nPush 7 to xs.\nShow item 1 of xs.\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyListFixed", label: "proven-fixed-seq", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 7 to xs.\nPush 5 to xs.\nShow (length of xs) + (item 1 of xs).\n", expect_out: "9", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyListI32", label: "while-build-arith-100", src: "## Main\nLet arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 100:\n    Push (i * 7 + 3) % 1000000 to arr.\n    Set i to i + 1.\nShow length of arr.\n", expect_out: "100", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyListI32", label: "while-build-zero", src: "## Main\nLet arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 0:\n    Push (i * 7 + 3) % 1000000 to arr.\n    Set i to i + 1.\nShow length of arr.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyListI32", label: "while-build-one", src: "## Main\nLet arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 1:\n    Push (i * 7 + 3) % 1000000 to arr.\n    Set i to i + 1.\nShow length of arr.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyListI32", label: "while-build-first-value", src: "## Main\nLet arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 5:\n    Push (i * 7 + 3) % 1000000 to arr.\n    Set i to i + 1.\nShow item 1 of arr.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyListI32", label: "while-build-many-1000", src: "## Main\nLet arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 1000:\n    Push i to arr.\n    Set i to i + 1.\nShow length of arr.\n", expect_out: "1000", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyListI32", label: "while-build-then-last", src: "## Main\nLet arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 5:\n    Push i to arr.\n    Set i to i + 1.\nShow item -1 of arr.\n", expect_out: "4", expect_err: false, oracle: "plain" },
    Prog { op: "NewRange", label: "sum-1-to-5", src: "## Main\nLet mutable total be 0.\nFor x from 1 to 5:\n    Set total to total + x.\nShow total.\n", expect_out: "15", expect_err: false, oracle: "plain" },
    Prog { op: "NewRange", label: "single-point-3-to-3", src: "## Main\nLet mutable total be 0.\nFor x from 3 to 3:\n    Set total to total + x.\nShow total.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "NewRange", label: "descending-5-to-1-empty", src: "## Main\nLet mutable total be 0.\nFor x from 5 to 1:\n    Set total to total + x.\nShow total.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewRange", label: "sum-1-to-100", src: "## Main\nLet mutable total be 0.\nFor x from 1 to 100:\n    Set total to total + x.\nShow total.\n", expect_out: "5050", expect_err: false, oracle: "plain" },
    Prog { op: "NewRange", label: "count-iterations-1-to-4", src: "## Main\nLet mutable count be 0.\nFor x from 1 to 4:\n    Set count to count + 1.\nShow count.\n", expect_out: "4", expect_err: false, oracle: "plain" },
    Prog { op: "NewRange", label: "zero-to-zero-single", src: "## Main\nLet mutable total be 0.\nFor x from 0 to 0:\n    Set total to total + 1.\nShow total.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "ListPush", label: "push-two-show-first", src: "## Main\nLet mutable xs be a new Seq of Text.\nPush \"hello\" to xs.\nPush \"world\" to xs.\nShow item 1 of xs.\n", expect_out: "hello", expect_err: false, oracle: "plain" },
    Prog { op: "ListPush", label: "push-two-show-second", src: "## Main\nLet mutable xs be a new Seq of Text.\nPush \"hello\" to xs.\nPush \"world\" to xs.\nShow item 2 of xs.\n", expect_out: "world", expect_err: false, oracle: "plain" },
    Prog { op: "ListPush", label: "push-int-length", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nPush 2 to xs.\nPush 3 to xs.\nShow length of xs.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "ListPush", label: "push-loop-many", src: "## Main\nLet mutable xs be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 50:\n    Push i to xs.\n    Set i to i + 1.\nShow length of xs.\n", expect_out: "50", expect_err: false, oracle: "plain" },
    Prog { op: "ListPush", label: "push-negative-value", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush -5 to xs.\nShow item 1 of xs.\n", expect_out: "-5", expect_err: false, oracle: "plain" },
    Prog { op: "ListPop", label: "pop-last-of-three", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nPush 2 to xs.\nPush 3 to xs.\nPop from xs into last.\nShow last.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "ListPop", label: "pop-single-then-length", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 99 to xs.\nPop from xs into v.\nShow v.\nShow length of xs.\n", expect_out: "99\n0", expect_err: false, oracle: "plain" },
    Prog { op: "ListPop", label: "pop-twice-order", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nPush 2 to xs.\nPop from xs into a.\nPop from xs into b.\nShow a.\nShow b.\n", expect_out: "2\n1", expect_err: false, oracle: "plain" },
    Prog { op: "ListPop", label: "pop-past-the-last-stops", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 7 to xs.\nPop from xs into a.\nShow a.\nPop from xs into b.\nShow b.\n", expect_out: "7", expect_err: true, oracle: "plain" },
    Prog { op: "ListPop", label: "pop-behind-its-check", src: "## To dropped (xs: Seq of Int) and (n: Int) -> Int:\n    Let mutable ys be xs.\n    Repeat for i from 1 to n:\n        Pop from ys.\n    Return length of ys.\n\n## Main\nShow dropped([4, 5, 6], 2).\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "ListPop", label: "pop-text-value", src: "## Main\nLet mutable xs be a new Seq of Text.\nPush \"x\" to xs.\nPop from xs into v.\nShow v.\n", expect_out: "x", expect_err: false, oracle: "plain" },
    Prog { op: "ListPushField", label: "push-two-field-length", src: "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\nPush 5 to b's items.\nPush 9 to b's items.\nShow length of b's items.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "ListPushField", label: "push-one-field-index", src: "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\nPush 42 to b's items.\nShow item 1 of b's items.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "ListPushField", label: "field-empty-length", src: "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\nShow length of b's items.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "ListPushField", label: "field-push-loop-many", src: "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\nLet mutable i be 0.\nWhile i is less than 20:\n    Push i to b's items.\n    Set i to i + 1.\nShow length of b's items.\n", expect_out: "20", expect_err: false, oracle: "plain" },
    Prog { op: "ListPushField", label: "field-push-second-index", src: "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\nPush 5 to b's items.\nPush 9 to b's items.\nShow item 2 of b's items.\n", expect_out: "9", expect_err: false, oracle: "plain" },
    Prog { op: "ListPushField", label: "field-seq-text", src: "## A Bag has:\n    an items, which is Seq of Text.\n## Main\nLet mutable b be a new Bag with items (a new Seq of Text).\nPush \"z\" to b's items.\nShow item 1 of b's items.\n", expect_out: "z", expect_err: false, oracle: "plain" },
    Prog { op: "Index", label: "out-of-bounds-literal-9", src: "## Main\nLet xs be [10, 20, 30].\nShow item 9 of xs.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Index", label: "variable-index-2", src: "## Main\nLet xs be [10, 20, 30].\nLet k be 2.\nShow item k of xs.\n", expect_out: "20", expect_err: false, oracle: "plain" },
    Prog { op: "Index", label: "slice-then-index", src: "## Main\nLet xs be [10, 20, 30].\nLet ys be items 2 through 2 of xs.\nShow item 1 of ys.\n", expect_out: "20", expect_err: false, oracle: "plain" },
    Prog { op: "Index", label: "zero-index-via-variable-error", src: "## Main\nLet xs be [10, 20, 30].\nLet k be 0.\nShow item k of xs.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Index", label: "negative-index-first-element", src: "## Main\nLet xs be [10, 20, 30].\nShow item -3 of xs.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "Index", label: "negative-index-out-of-bounds-error", src: "## Main\nLet xs be [10, 20, 30].\nShow item -4 of xs.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Index", label: "index-empty-list-error", src: "## Main\nLet xs be [].\nLet k be 1.\nShow item k of xs.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "IndexUnchecked", label: "last-literal-index", src: "## Main\nLet xs be [10, 20, 30].\nShow item 3 of xs.\n", expect_out: "30", expect_err: false, oracle: "plain" },
    Prog { op: "IndexUnchecked", label: "single-element-index-1", src: "## Main\nLet xs be [77].\nShow item 1 of xs.\n", expect_out: "77", expect_err: false, oracle: "plain" },
    Prog { op: "IndexUnchecked", label: "computed-index-expr", src: "## Main\nLet xs be [10, 20, 30].\nLet k be 1.\nShow item (k + 1) of xs.\n", expect_out: "20", expect_err: false, oracle: "plain" },
    Prog { op: "Length", label: "four-element-literal", src: "## Main\nLet xs be [1, 2, 3, 4].\nShow length of xs.\n", expect_out: "4", expect_err: false, oracle: "plain" },
    Prog { op: "Length", label: "single-element", src: "## Main\nLet xs be [99].\nShow length of xs.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "Length", label: "length-after-push", src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nPush 2 to xs.\nShow length of xs.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "Length", label: "length-of-concat", src: "## Main\nLet xs be [1, 2].\nLet ys be [3, 4, 5].\nLet zs be xs followed by ys.\nShow length of zs.\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "Length", label: "length-large-loop", src: "## Main\nLet mutable xs be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 256:\n    Push i to xs.\n    Set i to i + 1.\nShow length of xs.\n", expect_out: "256", expect_err: false, oracle: "plain" },
    Prog { op: "Length", label: "length-of-text-bytes", src: "## Main\nLet s be \"hello\".\nShow length of s.\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "SliceOp", label: "slice-2-through-4-length", src: "## Main\nLet xs be [10, 20, 30, 40, 50].\nLet ys be items 2 through 4 of xs.\nShow length of ys.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "SliceOp", label: "full-slice-first", src: "## Main\nLet xs be [10, 20, 30].\nLet ys be items 1 through 3 of xs.\nShow item 1 of ys.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "SliceOp", label: "descending-slice-empty", src: "## Main\nLet xs be [10, 20, 30].\nLet ys be items 3 through 2 of xs.\nShow length of ys.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "SliceOp", label: "slice-past-end-clamped", src: "## Main\nLet xs be [10, 20, 30].\nLet ys be items 2 through 9 of xs.\nShow length of ys.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "SliceOp", label: "slice-1-through-1-single", src: "## Main\nLet xs be [10, 20, 30].\nLet ys be items 1 through 1 of xs.\nShow item 1 of ys.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "SeqConcat", label: "concat-with-empty-right-identity", src: "## Main\nLet xs be [1, 2, 3].\nLet ys be [].\nLet zs be xs followed by ys.\nShow length of zs.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "SeqConcat", label: "concat-with-empty-left-identity", src: "## Main\nLet xs be [].\nLet ys be [7, 8].\nLet zs be xs followed by ys.\nShow length of zs.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "SeqConcat", label: "concat-both-empty", src: "## Main\nLet xs be [].\nLet ys be [].\nLet zs be xs followed by ys.\nShow length of zs.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "SeqConcat", label: "concat-first-element-is-left", src: "## Main\nLet xs be [10, 20].\nLet ys be [30, 40].\nLet zs be xs followed by ys.\nShow item 1 of zs.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "SeqConcat", label: "concat-boundary-element-is-right", src: "## Main\nLet xs be [10, 20].\nLet ys be [30, 40].\nLet zs be xs followed by ys.\nShow item 3 of zs.\n", expect_out: "30", expect_err: false, oracle: "plain" },
    Prog { op: "SeqConcat", label: "concat-text-lists", src: "## Main\nLet xs be [\"a\"].\nLet ys be [\"b\", \"c\"].\nLet zs be xs followed by ys.\nShow item 3 of zs.\n", expect_out: "c", expect_err: false, oracle: "plain" },
    Prog { op: "IterPrepare", label: "sum-over-three", src: "## Main\nLet xs be [10, 20, 30].\nLet mutable total be 0.\nFor each x in xs:\n    Set total to total + x.\nShow total.\n", expect_out: "60", expect_err: false, oracle: "plain" },
    Prog { op: "IterPrepare", label: "sum-over-empty", src: "## Main\nLet xs be [].\nLet mutable total be 0.\nFor each x in xs:\n    Set total to total + x.\nShow total.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "IterPrepare", label: "sum-over-single", src: "## Main\nLet xs be [7].\nLet mutable total be 0.\nFor each x in xs:\n    Set total to total + x.\nShow total.\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "IterPrepare", label: "count-over-four", src: "## Main\nLet xs be [1, 2, 3, 4].\nLet mutable count be 0.\nFor each x in xs:\n    Set count to count + 1.\nShow count.\n", expect_out: "4", expect_err: false, oracle: "plain" },
    Prog { op: "IterPrepare", label: "sum-over-built-list", src: "## Main\nLet mutable xs be a new Seq of Int.\nLet mutable i be 1.\nWhile i is less than 6:\n    Push i to xs.\n    Set i to i + 1.\nLet mutable total be 0.\nFor each x in xs:\n    Set total to total + x.\nShow total.\n", expect_out: "15", expect_err: false, oracle: "plain" },
    Prog { op: "IterPrepare", label: "sum-with-negatives", src: "## Main\nLet xs be [-1, -2, -3].\nLet mutable total be 0.\nFor each x in xs:\n    Set total to total + x.\nShow total.\n", expect_out: "-6", expect_err: false, oracle: "plain" },
    Prog { op: "IterNext", label: "foreach-single", src: "## Main\nLet xs be [88].\nLet mutable total be 0.\nFor each x in xs:\n    Set total to total + x.\nShow total.\n", expect_out: "88", expect_err: false, oracle: "plain" },
    Prog { op: "IterNext", label: "range-sum-many-1-to-50", src: "## Main\nLet mutable total be 0.\nFor x from 1 to 50:\n    Set total to total + x.\nShow total.\n", expect_out: "1275", expect_err: false, oracle: "plain" },
    Prog { op: "IterNext", label: "foreach-product-count", src: "## Main\nLet xs be [2, 4, 6, 8].\nLet mutable count be 0.\nFor each x in xs:\n    Set count to count + 1.\nShow count.\n", expect_out: "4", expect_err: false, oracle: "plain" },
    Prog { op: "IterPop", label: "foreach-single-then-done", src: "## Main\nLet xs be [5].\nLet mutable total be 0.\nFor each x in xs:\n    Set total to total + x.\nShow total.\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "IterPop", label: "foreach-drain-count-many", src: "## Main\nLet mutable xs be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 30:\n    Push i to xs.\n    Set i to i + 1.\nLet mutable count be 0.\nFor each x in xs:\n    Set count to count + 1.\nShow count.\n", expect_out: "30", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptySet", label: "empty int set shows braces and zero length", src: "## Main\n    Let mutable s be a new Set of Int.\n    Show s.\n    Show length of s.\n", expect_out: "{}\n0", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptySet", label: "text set with an idempotent duplicate", src: "## Main\n    Let mutable s be a new Set of Text.\n    Add \"alice\" to s.\n    Add \"bob\" to s.\n    Add \"alice\" to s.\n    Show s.\n    Show length of s.\n", expect_out: "{alice, bob}\n2", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptySet", label: "single-element set", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add 42 to s.\n    Show s.\n    Show length of s.\n", expect_out: "{42}\n1", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptySet", label: "empty text set shows braces and zero length", src: "## Main\n    Let mutable s be a new Set of Text.\n    Show s.\n    Show length of s.\n", expect_out: "{}\n0", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptySet", label: "many elements built by a loop preserve insertion order", src: "## Main\n    Let mutable s be a new Set of Int.\n    Let mutable i be 1.\n    While i is at most 5:\n        Add i to s.\n        Set i to i plus 1.\n    Show s.\n    Show length of s.\n", expect_out: "{1, 2, 3, 4, 5}\n5", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptySet", label: "negative and zero elements in a set", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add 0 to s.\n    Add -1 to s.\n    Add -1 to s.\n    Show s.\n    Show length of s.\n", expect_out: "{0, -1}\n2", expect_err: false, oracle: "plain" },
    Prog { op: "SetAdd", label: "add then remove leaves one element and membership true", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    Add 1 to s.\n    Remove 2 from s.\n    Show s.\n    If s contains 1:\n        Show 1.\n    Otherwise:\n        Show 0.\n", expect_out: "{1}\n1", expect_err: false, oracle: "plain" },
    Prog { op: "SetAdd", label: "add to an empty set makes one element", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add 7 to s.\n    Show s.\n    Show length of s.\n", expect_out: "{7}\n1", expect_err: false, oracle: "plain" },
    Prog { op: "SetAdd", label: "same value added many times stays length one", src: "## Main\n    Let mutable s be a new Set of Int.\n    Let mutable i be 1.\n    While i is at most 100:\n        Add 5 to s.\n        Set i to i plus 1.\n    Show s.\n    Show length of s.\n", expect_out: "{5}\n1", expect_err: false, oracle: "plain" },
    Prog { op: "SetAdd", label: "add negative values preserves insertion order", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add -5 to s.\n    Add 0 to s.\n    Add -5 to s.\n    Add 3 to s.\n    Show s.\n    Show length of s.\n", expect_out: "{-5, 0, 3}\n3", expect_err: false, oracle: "plain" },
    Prog { op: "SetAdd", label: "add a large value to a set", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add 1000000000 to s.\n    Add 1000000000 to s.\n    Show s.\n    Show length of s.\n", expect_out: "{1000000000}\n1", expect_err: false, oracle: "plain" },
    Prog { op: "RemoveFrom", label: "remove absent element is a no-op", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Remove 7 from s.\n    Show s.\n", expect_out: "{1}", expect_err: false, oracle: "plain" },
    Prog { op: "RemoveFrom", label: "remove the only element yields empty set", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add 9 to s.\n    Remove 9 from s.\n    Show s.\n    Show length of s.\n", expect_out: "{}\n0", expect_err: false, oracle: "plain" },
    Prog { op: "RemoveFrom", label: "remove from an already-empty set is a no-op", src: "## Main\n    Let mutable s be a new Set of Int.\n    Remove 3 from s.\n    Show s.\n    Show length of s.\n", expect_out: "{}\n0", expect_err: false, oracle: "plain" },
    Prog { op: "RemoveFrom", label: "remove a text element from a text set", src: "## Main\n    Let mutable s be a new Set of Text.\n    Add \"alice\" to s.\n    Add \"bob\" to s.\n    Remove \"alice\" from s.\n    Show s.\n    Show length of s.\n", expect_out: "{bob}\n1", expect_err: false, oracle: "plain" },
    Prog { op: "RemoveFrom", label: "remove a negative element", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add -1 to s.\n    Add -2 to s.\n    Remove -1 from s.\n    Show s.\n", expect_out: "{-2}", expect_err: false, oracle: "plain" },
    Prog { op: "Contains", label: "set does not contain absent element", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add 5 to s.\n    If s contains 9:\n        Show 1.\n    Otherwise:\n        Show 0.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Contains", label: "map contains checks keys not values", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 10.\n    Set item 2 of m to 20.\n    If m contains 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Contains", label: "empty set contains nothing", src: "## Main\n    Let mutable s be a new Set of Int.\n    If s contains 0:\n        Show 1.\n    Otherwise:\n        Show 0.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "Contains", label: "map contains a present key", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 2 of m to 20.\n    If m contains 2:\n        Show 1.\n    Otherwise:\n        Show 0.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "Contains", label: "set contains a negative element after add", src: "## Main\n    Let mutable s be a new Set of Int.\n    Add -3 to s.\n    If s contains -3:\n        Show 1.\n    Otherwise:\n        Show 0.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "Contains", label: "text set contains a present string", src: "## Main\n    Let mutable s be a new Set of Text.\n    Add \"alice\" to s.\n    If s contains \"alice\":\n        Show 1.\n    Otherwise:\n        Show 0.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "UnionOp", label: "union and intersection of overlapping sets", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Let mutable b be a new Set of Int.\n    Add 2 to b.\n    Add 3 to b.\n    Let u be a union b.\n    Let i be a intersection b.\n    Show u.\n    Show i.\n", expect_out: "{1, 2, 3}\n{2}", expect_err: false, oracle: "plain" },
    Prog { op: "UnionOp", label: "union with an empty set is the original", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Let mutable b be a new Set of Int.\n    Let u be a union b.\n    Show u.\n    Show length of u.\n", expect_out: "{1, 2}\n2", expect_err: false, oracle: "plain" },
    Prog { op: "UnionOp", label: "union of two empty sets is empty", src: "## Main\n    Let mutable a be a new Set of Int.\n    Let mutable b be a new Set of Int.\n    Let u be a union b.\n    Show u.\n    Show length of u.\n", expect_out: "{}\n0", expect_err: false, oracle: "plain" },
    Prog { op: "UnionOp", label: "union of disjoint sets keeps left order then right", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 3 to a.\n    Add 1 to a.\n    Let mutable b be a new Set of Int.\n    Add 4 to b.\n    Add 2 to b.\n    Let u be a union b.\n    Show u.\n", expect_out: "{3, 1, 4, 2}", expect_err: false, oracle: "plain" },
    Prog { op: "UnionOp", label: "union with self is idempotent", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Let u be a union a.\n    Show u.\n    Show length of u.\n", expect_out: "{1, 2}\n2", expect_err: false, oracle: "plain" },
    Prog { op: "UnionOp", label: "union of text sets deduplicates", src: "## Main\n    Let mutable a be a new Set of Text.\n    Add \"alice\" to a.\n    Let mutable b be a new Set of Text.\n    Add \"alice\" to b.\n    Add \"bob\" to b.\n    Let u be a union b.\n    Show u.\n", expect_out: "{alice, bob}", expect_err: false, oracle: "plain" },
    Prog { op: "IntersectOp", label: "intersection of disjoint sets is empty", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Let mutable b be a new Set of Int.\n    Add 2 to b.\n    Let i be a intersection b.\n    Show i.\n", expect_out: "{}", expect_err: false, oracle: "plain" },
    Prog { op: "IntersectOp", label: "intersection with empty set is empty", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Let mutable b be a new Set of Int.\n    Let i be a intersection b.\n    Show i.\n    Show length of i.\n", expect_out: "{}\n0", expect_err: false, oracle: "plain" },
    Prog { op: "IntersectOp", label: "intersection with self is the set itself", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 5 to a.\n    Add 6 to a.\n    Let i be a intersection a.\n    Show i.\n    Show length of i.\n", expect_out: "{5, 6}\n2", expect_err: false, oracle: "plain" },
    Prog { op: "IntersectOp", label: "intersection keeps left set order", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 3 to a.\n    Add 1 to a.\n    Add 2 to a.\n    Let mutable b be a new Set of Int.\n    Add 2 to b.\n    Add 3 to b.\n    Let i be a intersection b.\n    Show i.\n", expect_out: "{3, 2}", expect_err: false, oracle: "plain" },
    Prog { op: "IntersectOp", label: "intersection of text sets", src: "## Main\n    Let mutable a be a new Set of Text.\n    Add \"alice\" to a.\n    Add \"bob\" to a.\n    Let mutable b be a new Set of Text.\n    Add \"bob\" to b.\n    Let i be a intersection b.\n    Show i.\n", expect_out: "{bob}", expect_err: false, oracle: "plain" },
    Prog { op: "SetIndex", label: "text-keyed map with an overwrite", src: "## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"alice\" of m to 30.\n    Set item \"bob\" of m to 25.\n    Set item \"alice\" of m to 31.\n    Show m.\n    Show item \"alice\" of m.\n", expect_out: "{alice: 31, bob: 25}\n31", expect_err: false, oracle: "plain" },
    Prog { op: "SetIndex", label: "int-keyed map single entry", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 7 of m to 42.\n    Show m.\n    Show item 7 of m.\n", expect_out: "{7: 42}\n42", expect_err: false, oracle: "plain" },
    Prog { op: "SetIndex", label: "reading an absent map key errors", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 10.\n    Show item 2 of m.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "SetIndex", label: "map key overwrite keeps original insertion position", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 100.\n    Set item 2 of m to 200.\n    Set item 1 of m to 999.\n    Show m.\n    Show item 1 of m.\n", expect_out: "{1: 999, 2: 200}\n999", expect_err: false, oracle: "plain" },
    Prog { op: "SetIndex", label: "many entries built by a loop preserve insertion order", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Let mutable i be 1.\n    While i is at most 4:\n        Set item i of m to i plus 10.\n        Set i to i plus 1.\n    Show m.\n    Show length of m.\n", expect_out: "{1: 11, 2: 12, 3: 13, 4: 14}\n4", expect_err: false, oracle: "plain" },
    Prog { op: "SetIndexUnchecked", label: "set two list positions by 1-based index", src: "## Main\n    Let mutable xs be [5, 6, 7].\n    Set item 1 of xs to 99.\n    Set item 3 of xs to 88.\n    Show xs.\n", expect_out: "[99, 6, 88]", expect_err: false, oracle: "plain" },
    Prog { op: "SetIndexUnchecked", label: "set the single element of a one-element list", src: "## Main\n    Let mutable xs be [10].\n    Set item 1 of xs to 42.\n    Show xs.\n", expect_out: "[42]", expect_err: false, oracle: "plain" },
    Prog { op: "SetIndexUnchecked", label: "index past the end errors", src: "## Main\n    Let mutable xs be [1, 2, 3].\n    Set item 4 of xs to 99.\n    Show xs.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "SetIndexUnchecked", label: "index zero errors", src: "## Main\n    Let mutable xs be [1, 2, 3].\n    Set item 0 of xs to 99.\n    Show xs.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "SetIndexUnchecked", label: "negative end-relative index sets the last element", src: "## Main\n    Let mutable xs be [1, 2, 3].\n    Set item -1 of xs to 99.\n    Show xs.\n", expect_out: "[1, 2, 99]", expect_err: false, oracle: "plain" },
    Prog { op: "SetIndexUnchecked", label: "set a large value into a list slot", src: "## Main\n    Let mutable xs be [1, 2, 3].\n    Set item 2 of xs to 1000000000.\n    Show xs.\n", expect_out: "[1, 1000000000, 3]", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyMap", label: "empty int-to-int map shows braces", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Show m.\n", expect_out: "{}", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyMap", label: "empty map has length zero", src: "## Main\n    Let mutable m be a new Map of Text to Int.\n    Show m.\n    Show length of m.\n", expect_out: "{}\n0", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyMap", label: "single entry map", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 5 of m to 50.\n    Show m.\n    Show length of m.\n", expect_out: "{5: 50}\n1", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyMap", label: "text-to-text map", src: "## Main\n    Let mutable m be a new Map of Text to Text.\n    Set item \"greeting\" of m to \"hello\".\n    Show m.\n    Show item \"greeting\" of m.\n", expect_out: "{greeting: hello}\nhello", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyMap", label: "many entries built by a loop", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Let mutable i be 1.\n    While i is at most 3:\n        Set item i of m to i.\n        Set i to i plus 1.\n    Show m.\n    Show length of m.\n", expect_out: "{1: 1, 2: 2, 3: 3}\n3", expect_err: false, oracle: "plain" },
    Prog { op: "NewEmptyMapWithCapacity", label: "dynamic-capacity-map", src: "## Main\n    Let n be 17.\n    Let mutable m be a new Map of Int to Int with capacity n.\n    Set item 5 of m to 50.\n    Show item 5 of m.\n", expect_out: "50", expect_err: false, oracle: "plain" },
    Prog { op: "DeepClone", label: "copy of a concatenated list", src: "## Main\n    Let a be [1, 2].\n    Let b be [3, 4].\n    Let c be a followed by b.\n    Let d be copy of c.\n    Show d.\n", expect_out: "[1, 2, 3, 4]", expect_err: false, oracle: "plain" },
    Prog { op: "DeepClone", label: "copy of a set", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Let b be copy of a.\n    Show b.\n", expect_out: "{1, 2}", expect_err: false, oracle: "plain" },
    Prog { op: "DeepClone", label: "copy is independent of later mutation of the original", src: "## Main\n    Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Let b be copy of a.\n    Add 2 to a.\n    Show a.\n    Show b.\n", expect_out: "{1, 2}\n{1}", expect_err: false, oracle: "plain" },
    Prog { op: "DeepClone", label: "copy of an empty set", src: "## Main\n    Let mutable a be a new Set of Int.\n    Let b be copy of a.\n    Show b.\n    Show length of b.\n", expect_out: "{}\n0", expect_err: false, oracle: "plain" },
    Prog { op: "DeepClone", label: "copy of a single-element list", src: "## Main\n    Let a be [7].\n    Let b be copy of a.\n    Show b.\n", expect_out: "[7]", expect_err: false, oracle: "plain" },
    Prog { op: "DeepClone", label: "copy of a map", src: "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 10.\n    Set item 2 of m to 20.\n    Let n be copy of m.\n    Show n.\n", expect_out: "{1: 10, 2: 20}", expect_err: false, oracle: "plain" },
    Prog { op: "EnsureOwned", label: "bump copies the arg before mutating, caller unchanged at other index", src: "## To bump (arr: Seq of Int, i: Int) -> Seq of Int:\n    Let mutable r be arr.\n    Set item i of r to 99.\n    Return r.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Push 2 to a.\n    Set a to bump(a, 1).\n    Show item 1 of a.\n    Show item 2 of a.\n", expect_out: "99\n2", expect_err: false, oracle: "plain" },
    Prog { op: "EnsureOwned", label: "setat returns the mutated owned list", src: "## To setat (xs: Seq of Int) (k: Int) -> Seq of Int:\n    Set item k of xs to 99.\n    Return xs.\n\n## Main\n    Let mutable a be [1, 2, 3].\n    Set a to setat(a, 2).\n    Show a.\n", expect_out: "[1, 99, 3]", expect_err: false, oracle: "plain" },
    Prog { op: "EnsureOwned", label: "callee mutation does not leak back into an unassigned caller list", src: "## To setat (xs: Seq of Int) (k: Int) -> Seq of Int:\n    Set item k of xs to 99.\n    Return xs.\n\n## Main\n    Let mutable a be [1, 2, 3].\n    Let b be setat(a, 1).\n    Show a.\n    Show b.\n", expect_out: "[1, 2, 3]\n[99, 2, 3]", expect_err: false, oracle: "plain" },
    Prog { op: "EnsureOwned", label: "setat on a single-element list", src: "## To setat (xs: Seq of Int) (k: Int) -> Seq of Int:\n    Set item k of xs to 99.\n    Return xs.\n\n## Main\n    Let mutable a be [7].\n    Set a to setat(a, 1).\n    Show a.\n", expect_out: "[99]", expect_err: false, oracle: "plain" },
    Prog { op: "EnsureOwned", label: "setat with an out-of-bounds index errors", src: "## To setat (xs: Seq of Int) (k: Int) -> Seq of Int:\n    Set item k of xs to 99.\n    Return xs.\n\n## Main\n    Let mutable a be [1, 2, 3].\n    Set a to setat(a, 4).\n    Show a.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "EnsureOwned", label: "setat with a negative end-relative index", src: "## To setat (xs: Seq of Int) (k: Int) -> Seq of Int:\n    Set item k of xs to 99.\n    Return xs.\n\n## Main\n    Let mutable a be [1, 2, 3].\n    Set a to setat(a, -1).\n    Show a.\n", expect_out: "[1, 2, 99]", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "concat with empty prefix and empty suffix", src: "## Main\n    Let g be \"\" combined with \"x\".\n    Show g.\n    Let h be \"a\" combined with \"\".\n    Show h.\n", expect_out: "x\na", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "basic two-word concat", src: "## Main\n    Let g be \"foo\" combined with \"bar\".\n    Show g.\n", expect_out: "foobar", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "interpolation plus negative-int concat coercion", src: "## Main\n    Let total be 100.\n    Show \"count: {total}\".\n    Let neg be 0 - 7.\n    Show neg combined with \" items\".\n", expect_out: "count: 100\n-7 items", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "interpolation of float and bool", src: "## Main\n    Let pi be 3.14.\n    Let flag be 1 is less than 2.\n    Show \"pi is {pi} flag is {flag}\".\n", expect_out: "pi is 3.14 flag is true", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "interpolation of a text variable", src: "## Main\n    Let name be \"world\".\n    Show \"hello {name}!\".\n", expect_out: "hello world!", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "concat of two empty strings yields empty", src: "## Main\n    Let g be \"\" combined with \"\".\n    Show g.\n", expect_out: "", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "bool operand coerces to text via combined with", src: "## Main\n    Let flag be 1 is less than 2.\n    Show flag combined with \"!\".\n", expect_out: "true!", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "integral float operand renders bare (2.0 -> 2)", src: "## Main\n    Let x be 2.0.\n    Show \"n=\" combined with x.\n", expect_out: "n=2", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "non-integral float operand keeps decimal", src: "## Main\n    Let x be 3.5.\n    Show \"v=\" combined with x.\n", expect_out: "v=3.5", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "chained three-way concat is left-associative", src: "## Main\n    Let s be \"a\" combined with \"b\" combined with \"c\".\n    Show s.\n", expect_out: "abc", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "concat with empty string is identity on a negative int", src: "## Main\n    Let n be 0 - 42.\n    Show n combined with \"\".\n", expect_out: "-42", expect_err: false, oracle: "plain" },
    Prog { op: "Concat", label: "loop accumulates a longer string (many iterations)", src: "## Main\n    Let s be \"\".\n    Let i be 0.\n    While i is less than 3:\n        Set s to s combined with \"ab\".\n        Set i to i + 1.\n    Show s.\n", expect_out: "ababab", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "precision .2 and .4 on a float", src: "## Main\n    Let x be 3.14159.\n    Show \"{x:.2}\".\n    Show \"{x:.4}\".\n", expect_out: "3.14\n3.1416", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "precision .1 on an int promotes to float display", src: "## Main\n    Let n be 7.\n    Show \"{n:.1}\".\n", expect_out: "7.0", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "bare width pads an int right-aligned", src: "## Main\n    Let n be 42.\n    Show \"[{n:5}]\".\n", expect_out: "[   42]", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "debug spec echoes name and value", src: "## Main\n    Let x be 3.14159.\n    Show \"{x=}\".\n", expect_out: "x=3.14159", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "debug spec combined with precision", src: "## Main\n    Let x be 3.14159.\n    Show \"{x=:.2}\".\n", expect_out: "x=3.14", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "precision .0 rounds to nearest integer", src: "## Main\n    Let x be 3.7.\n    Show \"{x:.0}\".\n", expect_out: "4", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "right-align width on text with delimiters", src: "## Main\n    Let s be \"hi\".\n    Show \"|{s:>10}|\".\n", expect_out: "|        hi|", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "left-align width on text with delimiters", src: "## Main\n    Let s be \"hi\".\n    Show \"|{s:<10}|\".\n", expect_out: "|hi        |", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "center-align width on text with delimiters", src: "## Main\n    Let s be \"hi\".\n    Show \"|{s:^10}|\".\n", expect_out: "|    hi    |", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "currency spec on a float", src: "## Main\n    Let price be 19.99.\n    Show \"{price:$}\".\n", expect_out: "$19.99", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "currency spec on an integer yields cents", src: "## Main\n    Let price be 42.\n    Show \"{price:$}\".\n", expect_out: "$42.00", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "currency spec rounds whole float to two decimals", src: "## Main\n    Let price be 5.0.\n    Show \"{price:$}\".\n", expect_out: "$5.00", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "debug spec on an underscore-named variable", src: "## Main\n    Let my_var be 99.\n    Show \"{my_var=}\".\n", expect_out: "my_var=99", expect_err: false, oracle: "plain" },
    Prog { op: "FormatValue", label: "empty interpolation hole is a compile error", src: "## Main\n    Show \"Hello {}!\".\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "FormatValue", label: "invalid precision format specifier is an error", src: "## Main\n    Let x be 3.14.\n    Show \"{x:.abc}\".\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "FormatValue", label: "invalid alignment format specifier is an error", src: "## Main\n    Let x be 42.\n    Show \"{x:>abc}\".\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "FormatValue", label: "unclosed interpolation brace is an error", src: "## Main\n    Show \"Hello {name\".\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "NewStruct", label: "flat_two_int_fields", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 10 and y 20.\nShow p's x.\nShow p's y.\n", expect_out: "10\n20", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "nested_struct_field", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## A Line has:\n    A p: Point.\n\n## Main\nLet a be a new Point with x 1 and y 2.\nLet l be a new Line with p a.\nShow l's p's x.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "seq_field", src: "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet b be a new Bag with items [1, 2, 3].\nShow b's items.\n", expect_out: "[1, 2, 3]", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "zero_valued_fields", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 0 and y 0.\nShow p's x.\nShow p's y.\n", expect_out: "0\n0", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "negative_field", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x -7 and y -100.\nShow p's x.\nShow p's y.\n", expect_out: "-7\n-100", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "empty_seq_field", src: "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet b be a new Bag with items [].\nShow b's items.\n", expect_out: "[]", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "large_int_field", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 9223372036854775807 and y 1.\nShow p's x.\nShow p's y.\n", expect_out: "9223372036854775807\n1", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "bool_and_text_fields", src: "## A Flag has:\n    An on: Bool.\n    A name: Text.\n\n## Main\nLet fl be a new Flag with on true and name \"x\".\nShow fl's on.\nShow fl's name.\n", expect_out: "true\nx", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "real_field", src: "## A Data has:\n    A v: Real.\n\n## Main\nLet d be a new Data with v 1.5.\nShow d's v.\n", expect_out: "1.5", expect_err: false, oracle: "plain" },
    Prog { op: "NewStruct", label: "whole_struct_show_sorted", src: "## A Person has:\n    A name: Text.\n    An age: Int.\n\n## Main\nLet p be a new Person with name \"Bob\" and age 25.\nShow p.\n", expect_out: "Person { age: 25, name: Bob }", expect_err: false, oracle: "plain" },
    Prog { op: "StructInsert", label: "single_field_boundary", src: "## A Wrap has:\n    A n: Int.\n\n## Main\nLet w be a new Wrap with n 42.\nShow w's n.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "StructInsert", label: "zero_field_insert", src: "## A Wrap has:\n    A n: Int.\n\n## Main\nLet w be a new Wrap with n 0.\nShow w's n.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "StructInsert", label: "negative_field_insert", src: "## A Wrap has:\n    A n: Int.\n\n## Main\nLet w be a new Wrap with n -256.\nShow w's n.\n", expect_out: "-256", expect_err: false, oracle: "plain" },
    Prog { op: "StructInsert", label: "deeply_nested_insert", src: "## A A has:\n    A n: Int.\n\n## A B has:\n    A a: A.\n\n## A C has:\n    A b: B.\n\n## Main\nLet x be a new A with n 7.\nLet y be a new B with a x.\nLet z be a new C with b y.\nShow z's b's a's n.\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "StructInsert", label: "text_field_insert", src: "## A Label has:\n    A name: Text.\n\n## Main\nLet l be a new Label with name \"hello\".\nShow l's name.\n", expect_out: "hello", expect_err: false, oracle: "plain" },
    Prog { op: "StructInsert", label: "scalar_next_to_seq_field", src: "## A Rec has:\n    A id: Int.\n    A tags: Seq of Text.\n\n## Main\nLet r be a new Rec with id 7 and tags [\"p\", \"q\"].\nShow r's id.\nShow r's tags.\n", expect_out: "7\n[p, q]", expect_err: false, oracle: "plain" },
    Prog { op: "GetField", label: "read_zero_field", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 0 and y 5.\nShow p's x.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "GetField", label: "read_negative_field", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x -1 and y -2.\nShow p's x.\nShow p's y.\n", expect_out: "-1\n-2", expect_err: false, oracle: "plain" },
    Prog { op: "GetField", label: "read_bool_field", src: "## A Flag has:\n    An on: Bool.\n    A name: Text.\n\n## Main\nLet fl be a new Flag with on false and name \"z\".\nShow fl's on.\n", expect_out: "false", expect_err: false, oracle: "plain" },
    Prog { op: "GetField", label: "read_real_field", src: "## A Data has:\n    A v: Real.\n\n## Main\nLet d be a new Data with v 2.5.\nShow d's v.\n", expect_out: "2.5", expect_err: false, oracle: "plain" },
    Prog { op: "GetField", label: "read_field_used_in_arithmetic", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 3 and y 4.\nShow p's x + p's y.\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "GetField", label: "read_large_int_field", src: "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 9223372036854775807 and y -9223372036854775808.\nShow p's x.\nShow p's y.\n", expect_out: "9223372036854775807\n-9223372036854775808", expect_err: false, oracle: "plain" },
    Prog { op: "NewTuple", label: "pairs_sum_in_loop", src: "## Main\nLet pairs be [(1, 2), (3, 4), (5, 6)].\nRepeat for (a, b) in pairs:\n    Show a + b.\n", expect_out: "3\n7\n11", expect_err: false, oracle: "plain" },
    Prog { op: "NewTuple", label: "triple_index_access", src: "## Main\nLet t be (7, 8, 9).\nShow item 1 of t.\nShow item 3 of t.\n", expect_out: "7\n9", expect_err: false, oracle: "plain" },
    Prog { op: "NewTuple", label: "triples_sum_in_loop", src: "## Main\nLet triples be [(1, 2, 3), (4, 5, 6)].\nRepeat for (a, b, c) in triples:\n    Show a + b + c.\n", expect_out: "6\n15", expect_err: false, oracle: "plain" },
    Prog { op: "NewTuple", label: "tuple_negative_index_last", src: "## Main\nLet t be (7, 8, 9).\nShow item -1 of t.\n", expect_out: "9", expect_err: false, oracle: "plain" },
    Prog { op: "NewTuple", label: "tuple_index_out_of_bounds", src: "## Main\nLet t be (7, 8, 9).\nShow item 4 of t.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "NewTuple", label: "tuple_index_zero_error", src: "## Main\nLet t be (7, 8, 9).\nShow item 0 of t.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "NewTuple", label: "tuple_with_negative_and_zero_values", src: "## Main\nLet t be (0, -5, 100).\nShow item 1 of t.\nShow item 2 of t.\nShow item 3 of t.\n", expect_out: "0\n-5\n100", expect_err: false, oracle: "plain" },
    Prog { op: "NewTuple", label: "tuple_of_two_index", src: "## Main\nLet t be (11, 22).\nShow item 1 of t.\nShow item 2 of t.\n", expect_out: "11\n22", expect_err: false, oracle: "plain" },
    Prog { op: "NewTuple", label: "tuple_large_int_element", src: "## Main\nLet t be (9223372036854775807, 1).\nShow item 1 of t.\n", expect_out: "9223372036854775807", expect_err: false, oracle: "plain" },
    Prog { op: "NewTuple", label: "empty_pairs_loop_no_output", src: "## Main\nLet pairs be [].\nRepeat for (a, b) in pairs:\n    Show a + b.\n", expect_out: "", expect_err: false, oracle: "plain" },
    Prog { op: "DestructureTuple", label: "destructure_single_pair", src: "## Main\nLet pairs be [(10, 20)].\nRepeat for (a, b) in pairs:\n    Show a + b.\n", expect_out: "30", expect_err: false, oracle: "plain" },
    Prog { op: "DestructureTuple", label: "destructure_negative_components", src: "## Main\nLet pairs be [(-1, -2), (-3, -4)].\nRepeat for (a, b) in pairs:\n    Show a + b.\n", expect_out: "-3\n-7", expect_err: false, oracle: "plain" },
    Prog { op: "DestructureTuple", label: "destructure_zero_components", src: "## Main\nLet pairs be [(0, 0), (0, 5)].\nRepeat for (a, b) in pairs:\n    Show a + b.\n", expect_out: "0\n5", expect_err: false, oracle: "plain" },
    Prog { op: "DestructureTuple", label: "destructure_show_each_component", src: "## Main\nLet pairs be [(1, 2), (3, 4)].\nRepeat for (a, b) in pairs:\n    Show a.\n    Show b.\n", expect_out: "1\n2\n3\n4", expect_err: false, oracle: "plain" },
    Prog { op: "DestructureTuple", label: "destructure_many_pairs", src: "## Main\nLet pairs be [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)].\nRepeat for (a, b) in pairs:\n    Show a + b.\n", expect_out: "2\n4\n6\n8\n10", expect_err: false, oracle: "plain" },
    Prog { op: "DestructureTuple", label: "destructure_triple_show_components", src: "## Main\nLet triples be [(7, 8, 9)].\nRepeat for (a, b, c) in triples:\n    Show c.\n    Show a.\n", expect_out: "9\n7", expect_err: false, oracle: "plain" },
    Prog { op: "DestructureTuple", label: "destructure_large_int_component", src: "## Main\nLet pairs be [(9223372036854775806, 1)].\nRepeat for (a, b) in pairs:\n    Show a + b.\n", expect_out: "9223372036854775807", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "nullary_variant_matches_middle_arm", src: "## Definition\nA Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Green.\nInspect c:\n    If it is a Red:\n        Show \"red\".\n    If it is a Green:\n        Show \"green\".\n    Otherwise:\n        Show \"other\".\n", expect_out: "green", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "variant_with_one_int_field", src: "## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    A Point.\n\n## Main\nLet s be a new Circle with radius 5.\nInspect s:\n    If it is a Circle (radius: r):\n        Show r.\n    If it is a Point:\n        Show \"point\".\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "variant_with_two_int_fields", src: "## Definition\nA Shape is either:\n    A Rectangle with a width, which is Int, and a height, which is Int.\n    A Point.\n\n## Main\nLet s be a new Rectangle with width 4 and height 6.\nInspect s:\n    If it is a Rectangle (width: w, height: h):\n        Show w + h.\n    If it is a Point:\n        Show 0.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "variant_with_text_field", src: "## Definition\nA Result is either:\n    A Success (value: Int).\n    A Failure (message: Text).\n\n## Main\nLet r be a new Success with value 42.\nInspect r:\n    If it is a Success (value: v):\n        Show v.\n    If it is a Failure (message: m):\n        Show m.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "variant_field_zero", src: "## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    A Point.\n\n## Main\nLet s be a new Circle with radius 0.\nInspect s:\n    If it is a Circle (radius: r):\n        Show r.\n    If it is a Point:\n        Show \"point\".\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "variant_field_negative", src: "## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    A Point.\n\n## Main\nLet s be a new Circle with radius -8.\nInspect s:\n    If it is a Circle (radius: r):\n        Show r.\n    If it is a Point:\n        Show \"point\".\n", expect_out: "-8", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "variant_field_large_int", src: "## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    A Point.\n\n## Main\nLet s be a new Circle with radius 9223372036854775807.\nInspect s:\n    If it is a Circle (radius: r):\n        Show r.\n    If it is a Point:\n        Show \"point\".\n", expect_out: "9223372036854775807", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "nullary_variant_no_fields", src: "## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    A Point.\n\n## Main\nLet s be a new Point.\nInspect s:\n    If it is a Circle (radius: r):\n        Show r.\n    If it is a Point:\n        Show \"point\".\n", expect_out: "point", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "failure_text_variant_selected", src: "## Definition\nA Result is either:\n    A Success (value: Int).\n    A Failure (message: Text).\n\n## Main\nLet r be a new Failure with message \"boom\".\nInspect r:\n    If it is a Success (value: v):\n        Show v.\n    If it is a Failure (message: m):\n        Show m.\n", expect_out: "boom", expect_err: false, oracle: "plain" },
    Prog { op: "NewInductive", label: "first_variant_selected", src: "## Definition\nA Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Red.\nInspect c:\n    If it is a Red:\n        Show \"red\".\n    If it is a Green:\n        Show \"green\".\n    Otherwise:\n        Show \"other\".\n", expect_out: "red", expect_err: false, oracle: "plain" },
    Prog { op: "TestArm", label: "otherwise_catches_unlisted", src: "## Definition\nA Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Blue.\nInspect c:\n    If it is a Red:\n        Show \"red\".\n    Otherwise:\n        Show \"other\".\n", expect_out: "other", expect_err: false, oracle: "plain" },
    Prog { op: "TestArm", label: "first_arm_of_three_selected", src: "## Definition\nA Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Red.\nInspect c:\n    If it is a Red:\n        Show \"red\".\n    If it is a Green:\n        Show \"green\".\n    If it is a Blue:\n        Show \"blue\".\n", expect_out: "red", expect_err: false, oracle: "plain" },
    Prog { op: "TestArm", label: "last_arm_of_three_selected", src: "## Definition\nA Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Blue.\nInspect c:\n    If it is a Red:\n        Show \"red\".\n    If it is a Green:\n        Show \"green\".\n    If it is a Blue:\n        Show \"blue\".\n", expect_out: "blue", expect_err: false, oracle: "plain" },
    Prog { op: "TestArm", label: "only_otherwise_present", src: "## Definition\nA Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Green.\nInspect c:\n    Otherwise:\n        Show \"any\".\n", expect_out: "any", expect_err: false, oracle: "plain" },
    Prog { op: "TestArm", label: "text_variant_arm_test", src: "## Definition\nA Result is either:\n    A Success (value: Int).\n    A Failure (message: Text).\n\n## Main\nLet r be a new Failure with message \"nope\".\nInspect r:\n    If it is a Success (value: v):\n        Show v.\n    If it is a Failure (message: m):\n        Show m.\n", expect_out: "nope", expect_err: false, oracle: "plain" },
    Prog { op: "BindArm", label: "bind_negative_field", src: "## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    A Point.\n\n## Main\nLet s be a new Circle with radius -3.\nInspect s:\n    If it is a Circle (radius: r):\n        Show r.\n    If it is a Point:\n        Show \"point\".\n", expect_out: "-3", expect_err: false, oracle: "plain" },
    Prog { op: "BindArm", label: "bind_text_field", src: "## Definition\nA Result is either:\n    A Success (value: Int).\n    A Failure (message: Text).\n\n## Main\nLet r be a new Failure with message \"oops\".\nInspect r:\n    If it is a Success (value: v):\n        Show v.\n    If it is a Failure (message: m):\n        Show m.\n", expect_out: "oops", expect_err: false, oracle: "plain" },
    Prog { op: "BindArm", label: "bind_large_int_field", src: "## Definition\nA Result is either:\n    A Success (value: Int).\n    A Failure (message: Text).\n\n## Main\nLet r be a new Success with value 9223372036854775807.\nInspect r:\n    If it is a Success (value: v):\n        Show v.\n    If it is a Failure (message: m):\n        Show m.\n", expect_out: "9223372036854775807", expect_err: false, oracle: "plain" },
    Prog { op: "BindArm", label: "bind_two_fields_first_used", src: "## Definition\nA Shape is either:\n    A Rectangle with a width, which is Int, and a height, which is Int.\n    A Point.\n\n## Main\nLet s be a new Rectangle with width 100 and height 1.\nInspect s:\n    If it is a Rectangle (width: w, height: h):\n        Show w.\n    If it is a Point:\n        Show 0.\n", expect_out: "100", expect_err: false, oracle: "plain" },
    Prog { op: "BindArm", label: "bind_two_fields_second_used", src: "## Definition\nA Shape is either:\n    A Rectangle with a width, which is Int, and a height, which is Int.\n    A Point.\n\n## Main\nLet s be a new Rectangle with width 100 and height 1.\nInspect s:\n    If it is a Rectangle (width: w, height: h):\n        Show h.\n    If it is a Point:\n        Show 0.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "BindArm", label: "bind_field_used_in_arithmetic_product", src: "## Definition\nA Shape is either:\n    A Rectangle with a width, which is Int, and a height, which is Int.\n    A Point.\n\n## Main\nLet s be a new Rectangle with width 7 and height 8.\nInspect s:\n    If it is a Rectangle (width: w, height: h):\n        Show w * h.\n    If it is a Point:\n        Show 0.\n", expect_out: "56", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "closure passed to a named higher-order helper (square then apply)", src: "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    Let g be (n: Int) -> n * n.\n    Show apply(g, 7).\n", expect_out: "49", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "inline closure literal as an argument (no let binding)", src: "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    Show apply((n: Int) -> n * 2, 10).\n", expect_out: "20", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "closure bound then called directly", src: "## Main\n    Let doubler be (n: Int) -> n * 2.\n    Show doubler(5).\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "closure applied to zero (boundary)", src: "## Main\n    Let doubler be (n: Int) -> n * 2.\n    Show doubler(0).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "closure applied to a negative argument", src: "## Main\n    Let doubler be (n: Int) -> n * 2.\n    Show doubler(-3).\n", expect_out: "-6", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "squaring closure overflows i64 and promotes to bignum", src: "## Main\n    Let doubler be (n: Int) -> n * 2.\n    Show doubler(9223372036854775807).\n", expect_out: "18446744073709551614", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "two-parameter closure (adder) passed to helper is called with both args via helper x reuse", src: "## Main\n    Let add be (a: Int, b: Int) -> a + b.\n    Show add(2, 40).\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "closure body is a comparison producing a Bool", src: "## Main\n    Let isBig be (n: Int) -> n > 100.\n    Show isBig(250).\n", expect_out: "true", expect_err: false, oracle: "plain" },
    Prog { op: "MakeClosure", label: "closure captures an outer variable (lexical capture)", src: "## Main\n    Let base be 1000.\n    Let addBase be (n: Int) -> n + base.\n    Show addBase(7).\n", expect_out: "1007", expect_err: false, oracle: "plain" },
    Prog { op: "CallValue", label: "call a closure value with a zero argument (boundary)", src: "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    Let g be (n: Int) -> n * n.\n    Show apply(g, 0).\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "CallValue", label: "call a closure value with a negative argument", src: "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    Let g be (n: Int) -> n * n.\n    Show apply(g, -4).\n", expect_out: "16", expect_err: false, oracle: "plain" },
    Prog { op: "CallValue", label: "same closure value applied twice (idempotence of the call mechanism)", src: "## Main\n    Let doubler be (n: Int) -> n * 2.\n    Show doubler(doubler(3)).\n", expect_out: "12", expect_err: false, oracle: "plain" },
    Prog { op: "CallValue", label: "call a two-parameter closure value", src: "## Main\n    Let add be (a: Int, b: Int) -> a + b.\n    Show add(15, 27).\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "CallValue", label: "call through helper where closure body overflows into bignum", src: "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    Let g be (n: Int) -> n * 2.\n    Show apply(g, 9223372036854775807).\n", expect_out: "18446744073709551614", expect_err: false, oracle: "plain" },
    Prog { op: "CheckPolicy", label: "predicate holds — admin role passes, continues to Show", src: "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"admin\".\nCheck that the u is admin.\nShow \"ok\".\n", expect_out: "ok", expect_err: false, oracle: "plain" },
    Prog { op: "CheckPolicy", label: "predicate fails — guest role denied, security check fails", src: "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"guest\".\nCheck that the u is admin.\nShow \"reached\".\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "CheckPolicy", label: "disjunctive predicate — mod satisfies the OR branch", src: "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is staff if the user's role equals \"admin\" or the user's role equals \"mod\".\n\n## Main\nLet u be a new User with role \"mod\".\nCheck that the u is staff.\nShow \"staffok\".\n", expect_out: "staffok", expect_err: false, oracle: "plain" },
    Prog { op: "CheckPolicy", label: "capability denied — non-admin non-owner cannot edit", src: "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet mallory be a new User with name \"Mallory\" and role \"guest\".\nLet doc be a new Document with owner \"Alice\".\nCheck that mallory can edit doc.\nShow \"edited\".\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "CheckPolicy", label: "capability allowed via owner-match branch of the OR", src: "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet alice be a new User with name \"Alice\" and role \"guest\".\nLet doc be a new Document with owner \"Alice\".\nCheck that alice can edit doc.\nShow \"edited\".\n", expect_out: "edited", expect_err: false, oracle: "plain" },
    Prog { op: "CheckPolicy", label: "capability allowed via admin branch of the OR (owner differs)", src: "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet root be a new User with name \"Root\" and role \"admin\".\nLet doc be a new Document with owner \"Alice\".\nCheck that root can edit doc.\nShow \"edited\".\n", expect_out: "edited", expect_err: false, oracle: "plain" },
    Prog { op: "CheckPolicy", label: "disjunctive predicate — neither branch holds, denied", src: "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is staff if the user's role equals \"admin\" or the user's role equals \"mod\".\n\n## Main\nLet u be a new User with role \"guest\".\nCheck that the u is staff.\nShow \"staffok\".\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "CheckPolicy", label: "predicate passes, then a second Show confirms flow continues past the check", src: "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"admin\".\nShow \"before\".\nCheck that the u is admin.\nShow \"after\".\n", expect_out: "before\nafter", expect_err: false, oracle: "plain" },
    Prog { op: "Args", label: "args_length_empty", src: "## To native args () -> Seq of Text\n\n## Main\nLet a be args().\nShow length of a.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "Args", label: "args_item_past_end_is_out_of_bounds", src: "## To native args () -> Seq of Text\n\n## Main\nLet a be args().\nShow item 4 of a.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "Obs", label: "note_before_show_is_transparent", src: "## Main\n    Note that starting the run.\n    Show \"hi\".\n", expect_out: "hi", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "note_with_payload_is_transparent", src: "## Main\n    Let n be 5.\n    Note that computed value with n.\n    Show n.\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "warn_before_show_is_transparent", src: "## Main\n    Warn that something looks off.\n    Show \"done\".\n", expect_out: "done", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "record_with_payload_is_transparent", src: "## Main\n    Let x be 42.\n    Record that a metric with x.\n    Show x.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "during_span_body_runs_and_is_transparent", src: "## Main\n    During processing:\n        Show \"inside\".\n    Show \"after\".\n", expect_out: "inside\nafter", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "obs_around_show_never_touches_stdout", src: "## Main\n    Note that before.\n    Show 7.\n    Warn that after.\n    Record that done.\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "nested_during_spans_are_transparent", src: "## Main\n    During outer work:\n        During inner work:\n            Show \"deep\".\n    Show \"top\".\n", expect_out: "deep\ntop", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "note_sampling_rate_is_transparent", src: "## Main\n    Note once in every 1024 times that the packet arrived.\n    Show \"ok\".\n", expect_out: "ok", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "during_body_mutation_still_runs", src: "## Main\n    Let mutable total be 0.\n    During accumulating:\n        Set total to total + 5.\n        Set total to total + 5.\n    Show total.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "obs_only_no_show_produces_no_stdout", src: "## Main\n    Note that nothing to print.\n    Warn that still nothing.\n", expect_out: "", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "warn_with_two_payloads_is_transparent", src: "## Main\n    Let attempts be 3.\n    Let size be 42.\n    Warn that the upload failed with attempts and size.\n    Show attempts.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "Obs", label: "obs_inside_loop_body_is_transparent", src: "## Main\n    Let mutable i be 0.\n    While i is less than 3:\n        Note that a step ran.\n        Set i to i + 1.\n    Show i.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "NewCrdt", label: "new_convergent_count_two_adds", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable c be a new Counter.\n    Increase c's points by 5.\n    Increase c's points by 3.\n    Show c's points.\n", expect_out: "8", expect_err: false, oracle: "plain" },
    Prog { op: "NewCrdt", label: "new_tally_inc_then_dec", src: "## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n## Main\n    Let mutable w be a new Wallet.\n    Increase w's balance by 10.\n    Decrease w's balance by 4.\n    Show w's balance.\n", expect_out: "6", expect_err: false, oracle: "plain" },
    Prog { op: "NewCrdt", label: "new_shared_sequence_one_append", src: "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Text.\n## Main\n    Let g be a new Log.\n    Append \"a\" to g's entries.\n    Show g's entries.\n", expect_out: "[a]", expect_err: false, oracle: "plain" },
    Prog { op: "NewCrdt", label: "new_fresh_convergent_count_reads_zero", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable c be a new Counter.\n    Show c's points.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewCrdt", label: "new_fresh_tally_reads_zero", src: "## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n## Main\n    Let mutable w be a new Wallet.\n    Show w's balance.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewCrdt", label: "new_empty_shared_sequence_shows_empty", src: "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Text.\n## Main\n    Let g be a new Log.\n    Show g's entries.\n", expect_out: "[]", expect_err: false, oracle: "plain" },
    Prog { op: "NewCrdt", label: "new_fresh_shared_sequence_length_zero", src: "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Text.\n## Main\n    Let mutable g be a new Log.\n    Show length of g's entries.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "NewCrdt", label: "new_divergent_int_resolved_once", src: "## Definition\nA Cell is Shared and has:\n    a value, which is a Divergent Int.\n## Main\n    Let x be a new Cell.\n    Resolve x's value to 7.\n    Show x's value.\n", expect_out: "7", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtBump", label: "bump_tally_goes_negative", src: "## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n## Main\n    Let mutable w be a new Wallet.\n    Increase w's balance by 3.\n    Decrease w's balance by 10.\n    Show w's balance.\n", expect_out: "-7", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtBump", label: "bump_convergent_count_by_zero_identity", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable c be a new Counter.\n    Increase c's points by 5.\n    Increase c's points by 0.\n    Show c's points.\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtBump", label: "bump_tally_back_to_exactly_zero", src: "## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n## Main\n    Let mutable w be a new Wallet.\n    Increase w's balance by 8.\n    Decrease w's balance by 8.\n    Show w's balance.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtBump", label: "bump_convergent_count_by_variable", src: "## Definition\nA Counter is Shared and has:\n    score: ConvergentCount.\n## Main\n    Let mutable c be a new Counter.\n    Let amount be 25.\n    Increase c's score by amount.\n    Show c's score.\n", expect_out: "25", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtBump", label: "bump_convergent_count_large_value", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable c be a new Counter.\n    Increase c's points by 1000000000000.\n    Increase c's points by 2000000000000.\n    Show c's points.\n", expect_out: "3000000000000", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtBump", label: "bump_convergent_count_negative_is_error", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable c be a new Counter.\n    Increase c's points by 5.\n    Increase c's points by -3.\n    Show c's points.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "CrdtBump", label: "bump_decrease_convergent_count_is_error", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable c be a new Counter.\n    Increase c's points by 5.\n    Decrease c's points by 2.\n    Show c's points.\n", expect_out: "", expect_err: true, oracle: "plain" },
    Prog { op: "CrdtBump", label: "bump_convergent_count_many_adds", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable c be a new Counter.\n    Increase c's points by 1.\n    Increase c's points by 2.\n    Increase c's points by 3.\n    Increase c's points by 4.\n    Show c's points.\n", expect_out: "10", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtMerge", label: "merge_convergent_count_idempotent_double", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable a be a new Counter.\n    Increase a's points by 5.\n    Let mutable b be a new Counter.\n    Increase b's points by 3.\n    Merge b into a.\n    Merge b into a.\n    Show a's points.\n", expect_out: "8", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtMerge", label: "merge_tally_two_replicas", src: "## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n## Main\n    Let mutable a be a new Wallet.\n    Increase a's balance by 7.\n    Let mutable b be a new Wallet.\n    Increase b's balance by 2.\n    Merge b into a.\n    Show a's balance.\n", expect_out: "9", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtMerge", label: "merge_empty_remote_leaves_local_unchanged", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable a be a new Counter.\n    Increase a's points by 5.\n    Let mutable b be a new Counter.\n    Merge b into a.\n    Show a's points.\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtMerge", label: "merge_two_empty_replicas_is_zero", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable a be a new Counter.\n    Let mutable b be a new Counter.\n    Merge b into a.\n    Show a's points.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtMerge", label: "merge_idempotent_repeated_three_times", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable a be a new Counter.\n    Increase a's points by 10.\n    Let mutable b be a new Counter.\n    Increase b's points by 4.\n    Merge b into a.\n    Merge b into a.\n    Merge b into a.\n    Show a's points.\n", expect_out: "14", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtMerge", label: "merge_tally_negative_replica", src: "## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n## Main\n    Let mutable a be a new Wallet.\n    Increase a's balance by 5.\n    Let mutable b be a new Wallet.\n    Decrease b's balance by 8.\n    Merge b into a.\n    Show a's balance.\n", expect_out: "-3", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtMerge", label: "merge_convergent_count_large_disjoint", src: "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable a be a new Counter.\n    Increase a's points by 1000000000.\n    Let mutable b be a new Counter.\n    Increase b's points by 2000000000.\n    Merge b into a.\n    Show a's points.\n", expect_out: "3000000000", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtAppend", label: "append_two_then_show_marker", src: "## Definition\nA Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n## Main\n    Let d be a new Document.\n    Append \"Hello\" to d's lines.\n    Append \"World\" to d's lines.\n    Show \"done\".\n", expect_out: "done", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtAppend", label: "append_two_then_show_entries", src: "## Definition\nA Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n## Main\n    Let d be a new Document.\n    Append \"Hello\" to d's lines.\n    Append \"World\" to d's lines.\n    Show d's lines.\n", expect_out: "[Hello, World]", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtAppend", label: "append_one_then_show_length", src: "## Definition\nA Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n## Main\n    Let mutable d be a new Document.\n    Append \"Line 1\" to d's lines.\n    Show length of d's lines.\n", expect_out: "1", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtAppend", label: "append_two_then_show_length", src: "## Definition\nA Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n## Main\n    Let mutable d be a new Document.\n    Append \"Line 1\" to d's lines.\n    Append \"Line 2\" to d's lines.\n    Show length of d's lines.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtAppend", label: "append_three_then_show_length", src: "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Text.\n## Main\n    Let mutable g be a new Log.\n    Append \"a\" to g's entries.\n    Append \"b\" to g's entries.\n    Append \"c\" to g's entries.\n    Show length of g's entries.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtAppend", label: "append_three_then_show_entries", src: "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Text.\n## Main\n    Let mutable g be a new Log.\n    Append \"a\" to g's entries.\n    Append \"b\" to g's entries.\n    Append \"c\" to g's entries.\n    Show g's entries.\n", expect_out: "[a, b, c]", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtResolve", label: "resolve_int_then_show", src: "## Definition\nA Cell is Shared and has:\n    a value, which is a Divergent Int.\n## Main\n    Let x be a new Cell.\n    Resolve x's value to 42.\n    Show x's value.\n", expect_out: "42", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtResolve", label: "resolve_text_last_writer_wins", src: "## Definition\nA WikiPage is Shared and has:\n    a title, which is a Divergent Text.\n## Main\n    Let p be a new WikiPage.\n    Resolve p's title to \"Draft\".\n    Resolve p's title to \"Final\".\n    Show p's title.\n", expect_out: "Final", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtResolve", label: "resolve_text_then_marker", src: "## Definition\nA WikiPage is Shared and has:\n    a title, which is a Divergent Text.\n## Main\n    Let p be a new WikiPage.\n    Resolve p's title to \"Final\".\n    Show \"done\".\n", expect_out: "done", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtResolve", label: "resolve_int_zero", src: "## Definition\nA Cell is Shared and has:\n    a value, which is a Divergent Int.\n## Main\n    Let x be a new Cell.\n    Resolve x's value to 0.\n    Show x's value.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtResolve", label: "resolve_int_negative", src: "## Definition\nA Cell is Shared and has:\n    a value, which is a Divergent Int.\n## Main\n    Let x be a new Cell.\n    Resolve x's value to -5.\n    Show x's value.\n", expect_out: "-5", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtResolve", label: "resolve_int_overwritten_last_wins", src: "## Definition\nA Cell is Shared and has:\n    a value, which is a Divergent Int.\n## Main\n    Let x be a new Cell.\n    Resolve x's value to 1.\n    Resolve x's value to 2.\n    Resolve x's value to 3.\n    Show x's value.\n", expect_out: "3", expect_err: false, oracle: "plain" },
    Prog { op: "CrdtResolve", label: "resolve_int_large_value", src: "## Definition\nA Cell is Shared and has:\n    a value, which is a Divergent Int.\n## Main\n    Let x be a new Cell.\n    Resolve x's value to 9000000000000.\n    Show x's value.\n", expect_out: "9000000000000", expect_err: false, oracle: "plain" },
    Prog { op: "ChanNew", label: "two sends then two receives sum (existing)", src: "## Main\n    Let jobs be a Pipe of Int.\n    Send 10 into jobs.\n    Send 20 into jobs.\n    Receive a from jobs.\n    Receive b from jobs.\n    Show a + b.\n", expect_out: "30", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanNew", label: "new Pipe keyword single value (existing)", src: "## Main\n    Let messages be a new Pipe of Int.\n    Send 42 into messages.\n    Receive x from messages.\n    Show x.\n", expect_out: "42", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanNew", label: "Pipe of Text carries a string (existing)", src: "## Main\n    Let ch be a Pipe of Text.\n    Send \"hi\" into ch.\n    Receive g from ch.\n    Show g.\n", expect_out: "hi", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanNew", label: "Pipe of Bool carries a boolean", src: "## Main\n    Let ch be a Pipe of Bool.\n    Send true into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "true", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanNew", label: "Pipe of Real carries a float", src: "## Main\n    Let ch be a Pipe of Real.\n    Send 3.5 into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "3.5", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanNew", label: "channel carries zero", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 0 into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "0", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanNew", label: "channel carries a negative int", src: "## Main\n    Let ch be a Pipe of Int.\n    Send -7 into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "-7", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanNew", label: "fresh channel try-receive on empty yields nothing", src: "## Main\n    Let ch be a Pipe of Int.\n    Try to receive x from ch.\n    Show x.\n", expect_out: "nothing", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanSend", label: "producer task sends three, main receives three (existing)", src: "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    Send 3 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive a from jobs.\n    Receive b from jobs.\n    Receive c from jobs.\n    Show a + b + c.\n", expect_out: "6", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanSend", label: "try-send then blocking receive (existing)", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 99 into ch.\n    Try to receive x from ch.\n    Show x.\n", expect_out: "99", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanSend", label: "single send single receive", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 1 into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "1", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanSend", label: "send a large int that overflows only after channel arithmetic", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 10000000000 into ch.\n    Send 10000000000 into ch.\n    Receive a from ch.\n    Receive b from ch.\n    Show a * b.\n", expect_out: "100000000000000000000", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanSend", label: "many sends via a producer loop, consumer loop sums 1..5", src: "## To produce (ch: Int):\n    Let i be 1.\n    While i is less than 6:\n        Send i into ch.\n        Set i to i + 1.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Let total be 0.\n    Let k be 0.\n    While k is less than 5:\n        Receive v from jobs.\n        Set total to total + v.\n        Set k to k + 1.\n    Show total.\n", expect_out: "15", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanSend", label: "send preserves FIFO order across two receives", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 7 into ch.\n    Send 8 into ch.\n    Receive a from ch.\n    Receive b from ch.\n    Show a.\n    Show b.\n", expect_out: "7\n8", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanRecv", label: "try-send then blocking receive (existing)", src: "## Main\n    Let ch be a Pipe of Int.\n    Try to send 5 into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "5", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanRecv", label: "receive rendezvous from a producer task", src: "## To produce (ch: Int):\n    Send 7 into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "7", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanRecv", label: "receive a buffered item from a closed channel", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 5 into ch.\n    Close ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "5", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanRecv", label: "receive on a closed-empty channel yields nothing", src: "## Main\n    Let ch be a Pipe of Int.\n    Close ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "nothing", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanRecv", label: "receive a negative int through the channel", src: "## Main\n    Let ch be a Pipe of Int.\n    Send -1 into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "-1", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanClose", label: "buffered item survives close and is delivered first", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 42 into ch.\n    Close ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "42", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanClose", label: "buffered item then nothing after drain on closed channel", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 5 into ch.\n    Close ch.\n    Receive a from ch.\n    Receive b from ch.\n    Show a.\n    Show b.\n", expect_out: "5\nnothing", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanClose", label: "close then try-receive on empty yields nothing", src: "## Main\n    Let ch be a Pipe of Int.\n    Close ch.\n    Try to receive x from ch.\n    Show x.\n", expect_out: "nothing", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanClose", label: "close a Text channel then receive yields nothing", src: "## Main\n    Let ch be a Pipe of Text.\n    Close ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "nothing", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanClose", label: "two buffered items drain in FIFO order after close", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 1 into ch.\n    Send 2 into ch.\n    Close ch.\n    Receive a from ch.\n    Receive b from ch.\n    Show a + b.\n", expect_out: "3", expect_err: false, oracle: "concurrent" },
    Prog { op: "Spawn", label: "worker task runs before main continues (existing)", src: "## To worker:\n    Show \"work\".\n\n## Main\n    Launch a task to worker.\n    Show \"main\".\n", expect_out: "work\nmain", expect_err: false, oracle: "concurrent" },
    Prog { op: "Spawn", label: "launch with an int argument (existing)", src: "## To greet (n: Int):\n    Show n.\n\n## Main\n    Launch a task to greet with 7.\n    Show 0.\n", expect_out: "7\n0", expect_err: false, oracle: "concurrent" },
    Prog { op: "Spawn", label: "launch with a Text argument shows raw string first", src: "## To greet (name: Text):\n    Show name.\n\n## Main\n    Launch a task to greet with \"hello\".\n    Show \"main\".\n", expect_out: "hello\nmain", expect_err: false, oracle: "concurrent" },
    Prog { op: "Spawn", label: "launch with a negative int argument", src: "## To greet (n: Int):\n    Show n.\n\n## Main\n    Launch a task to greet with -5.\n    Show 0.\n", expect_out: "-5\n0", expect_err: false, oracle: "concurrent" },
    Prog { op: "Spawn", label: "launch with a zero argument", src: "## To greet (n: Int):\n    Show n.\n\n## Main\n    Launch a task to greet with 0.\n    Show 1.\n", expect_out: "0\n1", expect_err: false, oracle: "concurrent" },
    Prog { op: "Spawn", label: "worker task computes a bignum before main continues", src: "## To worker:\n    Show 9223372036854775807 + 1.\n\n## Main\n    Launch a task to worker.\n    Show \"done\".\n", expect_out: "9223372036854775808\ndone", expect_err: false, oracle: "concurrent" },
    Prog { op: "SpawnHandle", label: "handle to a worker, worker runs before main (existing)", src: "## To worker:\n    Show \"work\".\n\n## Main\n    Let h be Launch a task to worker.\n    Show \"main\".\n", expect_out: "work\nmain", expect_err: false, oracle: "concurrent" },
    Prog { op: "SpawnHandle", label: "handle to greet with an int argument (existing)", src: "## To greet (n: Int):\n    Show n.\n\n## Main\n    Let h be Launch a task to greet with 9.\n    Show 0.\n", expect_out: "9\n0", expect_err: false, oracle: "concurrent" },
    Prog { op: "SpawnHandle", label: "handle to a producer task; main receives after handle bind", src: "## To produce (ch: Int):\n    Send 3 into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    Let h be Launch a task to produce with ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "3", expect_err: false, oracle: "concurrent" },
    Prog { op: "SpawnHandle", label: "handle to greet with a negative int", src: "## To greet (n: Int):\n    Show n.\n\n## Main\n    Let h be Launch a task to greet with -4.\n    Show 0.\n", expect_out: "-4\n0", expect_err: false, oracle: "concurrent" },
    Prog { op: "SpawnHandle", label: "handle to greet with a Text argument", src: "## To greet (name: Text):\n    Show name.\n\n## Main\n    Let h be Launch a task to greet with \"hi\".\n    Show \"main\".\n", expect_out: "hi\nmain", expect_err: false, oracle: "concurrent" },
    Prog { op: "SpawnHandle", label: "handle to greet with zero", src: "## To greet (n: Int):\n    Show n.\n\n## Main\n    Let h be Launch a task to greet with 0.\n    Show 1.\n", expect_out: "0\n1", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTrySend", label: "try-send then try-receive round-trip (existing)", src: "## Main\n    Let ch be a Pipe of Int.\n    Try to send 42 into ch.\n    Try to receive x from ch.\n    Show x.\n", expect_out: "42", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTrySend", label: "try-send never blocks the sender on an empty channel", src: "## Main\n    Let ch be a Pipe of Int.\n    Try to send 99 into ch.\n    Show \"sent\".\n", expect_out: "sent", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTrySend", label: "try-send a zero value", src: "## Main\n    Let ch be a Pipe of Int.\n    Try to send 0 into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "0", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTrySend", label: "try-send a negative value", src: "## Main\n    Let ch be a Pipe of Int.\n    Try to send -3 into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "-3", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTrySend", label: "try-send a Text value round-trips", src: "## Main\n    Let ch be a Pipe of Text.\n    Try to send \"ok\" into ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "ok", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTrySend", label: "two try-sends buffered then received in FIFO order (unbounded)", src: "## Main\n    Let ch be a Pipe of Int.\n    Try to send 1 into ch.\n    Try to send 2 into ch.\n    Receive a from ch.\n    Receive b from ch.\n    Show a + b.\n", expect_out: "3", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTryRecv", label: "try-receive drains the only buffered item, then nothing", src: "## Main\n    Let ch be a Pipe of Int.\n    Send 7 into ch.\n    Try to receive a from ch.\n    Try to receive b from ch.\n    Show a.\n    Show b.\n", expect_out: "7\nnothing", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTryRecv", label: "try-receive gets a negative value", src: "## Main\n    Let ch be a Pipe of Int.\n    Send -8 into ch.\n    Try to receive x from ch.\n    Show x.\n", expect_out: "-8", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanTryRecv", label: "try-receive gets a Text value", src: "## Main\n    Let ch be a Pipe of Text.\n    Send \"yo\" into ch.\n    Try to receive x from ch.\n    Show x.\n", expect_out: "yo", expect_err: false, oracle: "concurrent" },
    Prog { op: "Sleep", label: "sleep_basic", src: "## Main\n    Show 1.\n    Sleep 5.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "concurrent" },
    Prog { op: "Sleep", label: "sleep_var", src: "## Main\n    Let d be 3.\n    Show 1.\n    Sleep d.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "concurrent" },
    Prog { op: "Sleep", label: "sleep_zero", src: "## Main\n    Show 1.\n    Sleep 0.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "concurrent" },
    Prog { op: "Sleep", label: "sleep_one_boundary", src: "## Main\n    Show 1.\n    Sleep 1.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "concurrent" },
    Prog { op: "Sleep", label: "sleep_large", src: "## Main\n    Show 1.\n    Sleep 1000000.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "concurrent" },
    Prog { op: "Sleep", label: "sleep_many_sequential", src: "## Main\n    Show 1.\n    Sleep 2.\n    Sleep 3.\n    Sleep 4.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "concurrent" },
    Prog { op: "Sleep", label: "sleep_in_function", src: "## To pause:\n    Sleep 10.\n\n## Main\n    Show 1.\n    Call pause.\n    Show 2.\n", expect_out: "1\n2", expect_err: false, oracle: "concurrent" },
    Prog { op: "Sleep", label: "sleep_then_compute", src: "## Main\n    Let d be 7.\n    Sleep d.\n    Show d + 1.\n", expect_out: "8", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmRecv", label: "select_recv_wins", src: "## To produce (ch: Int):\n    Send 9 into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Await the first of:\n        Receive v from ch:\n            Show v.\n        After 100 seconds:\n            Show 0.\n", expect_out: "9", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmRecv", label: "select_two_recv", src: "## To produce (ch: Int):\n    Send 7 into ch.\n\n## Main\n    Let a be a Pipe of Int.\n    Let b be a Pipe of Int.\n    Launch a task to produce with b.\n    Await the first of:\n        Receive x from a:\n            Show x.\n        Receive y from b:\n            Show y.\n        After 100 seconds:\n            Show 0.\n", expect_out: "7", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmRecv", label: "select_recv_wins_zero", src: "## To producer (ch: Int):\n    Send 0 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 42.\n", expect_out: "0", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmRecv", label: "select_recv_wins_negative", src: "## To producer (ch: Int):\n    Send -5 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 42.\n", expect_out: "-5", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmRecv", label: "select_recv_wins_large", src: "## To producer (ch: Int):\n    Send 1000000 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 42.\n", expect_out: "1000000", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmRecv", label: "select_recv_wins_one", src: "## To producer (ch: Int):\n    Send 1 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 0.\n", expect_out: "1", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmRecv", label: "select_recv_computed_body", src: "## To producer (ch: Int):\n    Send 20 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x + 1.\n        After 10 seconds:\n            Show 0.\n", expect_out: "21", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmTimeout", label: "select_timeout_wins", src: "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            Show v.\n        After 5 seconds:\n            Show 42.\n", expect_out: "42", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmTimeout", label: "select_timeout_one_second", src: "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            Show v.\n        After 1 seconds:\n            Show 42.\n", expect_out: "42", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmTimeout", label: "select_timeout_only_arm", src: "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        After 1 seconds:\n            Show 99.\n", expect_out: "99", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmTimeout", label: "select_timeout_wins_zero_body", src: "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            Show v.\n        After 3 seconds:\n            Show 0.\n", expect_out: "0", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmTimeout", label: "select_timeout_wins_negative_body", src: "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            Show v.\n        After 3 seconds:\n            Show -7.\n", expect_out: "-7", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectArmTimeout", label: "select_timeout_two_recv_arms", src: "## Main\n    Let a be a Pipe of Int.\n    Let b be a Pipe of Int.\n    Await the first of:\n        Receive x from a:\n            Show x.\n        Receive y from b:\n            Show y.\n        After 2 seconds:\n            Show 55.\n", expect_out: "55", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectWait", label: "select_wait_recv_then_after", src: "## To producer (ch: Int):\n    Send 5 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 0.\n", expect_out: "5", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectWait", label: "select_wait_timeout_only", src: "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        After 4 seconds:\n            Show 7.\n", expect_out: "7", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectWait", label: "select_wait_three_arms_recv_wins", src: "## To produce (ch: Int):\n    Send 8 into ch.\n\n## Main\n    Let a be a Pipe of Int.\n    Let b be a Pipe of Int.\n    Launch a task to produce with b.\n    Await the first of:\n        Receive x from a:\n            Show x.\n        Receive y from b:\n            Show y.\n        After 100 seconds:\n            Show 0.\n", expect_out: "8", expect_err: false, oracle: "concurrent" },
    Prog { op: "SelectWait", label: "select_wait_after_shows_after", src: "## Main\n    Let a be a Pipe of Int.\n    Let b be a Pipe of Int.\n    Await the first of:\n        Receive x from a:\n            Show x.\n        Receive y from b:\n            Show y.\n        After 1 seconds:\n            Show 33.\n", expect_out: "33", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAbort", label: "abort_then_send", src: "## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n## Main\n    Let ch be a Pipe of Int.\n    Let h be Launch a task to blocker with ch.\n    Stop h.\n    Send 9 into ch.\n    Show \"main\".\n", expect_out: "main", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAbort", label: "abort_blocked", src: "## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n## Main\n    Let ch be a Pipe of Int.\n    Let h be Launch a task to blocker with ch.\n    Stop h.\n    Show \"main\".\n", expect_out: "main", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAbort", label: "abort_after_receive", src: "## To worker (ch: Int):\n    Send 7 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n", expect_out: "7", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAbort", label: "abort_after_receive_zero", src: "## To worker (ch: Int):\n    Send 0 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n", expect_out: "0", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAbort", label: "abort_after_receive_negative", src: "## To worker (ch: Int):\n    Send -3 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n", expect_out: "-3", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAbort", label: "abort_after_receive_large", src: "## To worker (ch: Int):\n    Send 1000000 into ch.\n\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n", expect_out: "1000000", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAbort", label: "abort_two_handles", src: "## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n## Main\n    Let a be a Pipe of Int.\n    Let b be a Pipe of Int.\n    Let h be Launch a task to blocker with a.\n    Let g be Launch a task to blocker with b.\n    Stop h.\n    Stop g.\n    Show \"main\".\n", expect_out: "main", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAwait", label: "await_bare_discard_then_show", src: "## To fetch -> Int:\n    Sleep 5.\n    Return 99.\n\n## Main\n    Let h be Launch a task to fetch.\n    Await h.\n    Show \"done\".\n", expect_out: "done", expect_err: false, oracle: "concurrent" },
    Prog { op: "NetConnect", label: "connect_only", src: "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Show \"connected\".\n", expect_out: "connected", expect_err: false, oracle: "net" },
    Prog { op: "NetConnect", label: "connect_scalar_roundtrip", src: "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 7 to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "7", expect_err: false, oracle: "net" },
    Prog { op: "NetConnect", label: "connect_twice_idempotent", src: "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Connect to \"ws://127.0.0.1:9\".\n    Show \"connected\".\n", expect_out: "connected", expect_err: false, oracle: "net" },
    Prog { op: "NetConnect", label: "connect_text_roundtrip", src: "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send \"hello\" to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "hello", expect_err: false, oracle: "net" },
    Prog { op: "NetConnect", label: "connect_negative_scalar_roundtrip", src: "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send -9 to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "-9", expect_err: false, oracle: "net" },
    Prog { op: "NetListen", label: "listen_only", src: "## Main\n    Listen on \"chan\".\n    Show \"listening\".\n", expect_out: "listening", expect_err: false, oracle: "net" },
    Prog { op: "NetListen", label: "listen_named_channel", src: "## Main\n    Listen on \"room-42\".\n    Show \"ok\".\n", expect_out: "ok", expect_err: false, oracle: "net" },
    Prog { op: "NetListen", label: "listen_send_two_fifo", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 1 to peer.\n    Send 2 to peer.\n    Await response from peer into a.\n    Await response from peer into b.\n    Show a.\n    Show b.\n", expect_out: "1\n2", expect_err: false, oracle: "net" },
    Prog { op: "NetListen", label: "listen_stream_roundtrip", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Let vals be [5].\n    Stream vals to peer.\n    Await stream from peer into b.\n    Show item 1 of b.\n", expect_out: "5", expect_err: false, oracle: "net" },
    Prog { op: "NetMakePeer", label: "makepeer_scalar_text_roundtrip", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send \"hi\" to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "hi", expect_err: false, oracle: "net" },
    Prog { op: "NetMakePeer", label: "makepeer_bool_roundtrip", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send true to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "true", expect_err: false, oracle: "net" },
    Prog { op: "NetMakePeer", label: "makepeer_real_roundtrip", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 2.5 to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "2.5", expect_err: false, oracle: "net" },
    Prog { op: "NetMakePeer", label: "makepeer_zero_roundtrip", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 0 to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "0", expect_err: false, oracle: "net" },
    Prog { op: "NetSend", label: "send_large_int_roundtrip", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 1000000000 to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "1000000000", expect_err: false, oracle: "net" },
    Prog { op: "NetSend", label: "send_list_value_roundtrip", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Let nums be [10, 20, 30].\n    Send nums to peer.\n    Await response from peer into got.\n    Show item 2 of got.\n", expect_out: "20", expect_err: false, oracle: "net" },
    Prog { op: "NetStream", label: "stream_roundtrip", src: "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Let vals be [1, 2, 3].\n    Stream vals to peer.\n    Await stream from peer into b.\n    Show item 2 of b.\n", expect_out: "2", expect_err: false, oracle: "net" },
    Prog { op: "NetStream", label: "stream_len", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Let vals be [10, 20].\n    Stream vals to peer.\n    Await stream from peer into b.\n    Show the length of b.\n", expect_out: "2", expect_err: false, oracle: "net" },
    Prog { op: "NetStream", label: "stream_empty_len_zero", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Let vals be [] of Int.\n    Stream vals to peer.\n    Await stream from peer into b.\n    Show the length of b.\n", expect_out: "0", expect_err: false, oracle: "net" },
    Prog { op: "NetStream", label: "stream_single_element", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Let vals be [99].\n    Stream vals to peer.\n    Await stream from peer into b.\n    Show item 1 of b.\n", expect_out: "99", expect_err: false, oracle: "net" },
    Prog { op: "NetStream", label: "stream_many_len_ten", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Let vals be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].\n    Stream vals to peer.\n    Await stream from peer into b.\n    Show the length of b.\n", expect_out: "10", expect_err: false, oracle: "net" },
    Prog { op: "NetAwait", label: "await_text_roundtrip", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send \"world\" to peer.\n    Await response from peer into got.\n    Show got.\n", expect_out: "world", expect_err: false, oracle: "net" },
    Prog { op: "NetAwait", label: "await_stream_first_element", src: "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Let vals be [7, 8, 9].\n    Stream vals to peer.\n    Await stream from peer into b.\n    Show item 1 of b.\n", expect_out: "7", expect_err: false, oracle: "net" },
    Prog { op: "NetSync", label: "sync_scalar", src: "## Main\n    Let mutable c be 5.\n    Connect to \"ws://127.0.0.1:9\".\n    Sync c on \"t\".\n    Show c.\n", expect_out: "5", expect_err: false, oracle: "net" },
    Prog { op: "NetSync", label: "sync_crdt_counter", src: "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable state be a new Tally.\n    Increase state's n by 3.\n    Sync state on \"topic\".\n    Show state's n.\n", expect_out: "3", expect_err: false, oracle: "net" },
    Prog { op: "NetSync", label: "sync_scalar_zero", src: "## Main\n    Let mutable c be 0.\n    Connect to \"ws://127.0.0.1:9\".\n    Sync c on \"t\".\n    Show c.\n", expect_out: "0", expect_err: false, oracle: "net" },
    Prog { op: "NetSync", label: "sync_scalar_negative", src: "## Main\n    Let mutable c be -3.\n    Connect to \"ws://127.0.0.1:9\".\n    Sync c on \"t\".\n    Show c.\n", expect_out: "-3", expect_err: false, oracle: "net" },
    Prog { op: "NetSync", label: "sync_crdt_counter_accumulates", src: "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable state be a new Tally.\n    Increase state's n by 5.\n    Increase state's n by 7.\n    Sync state on \"topic\".\n    Show state's n.\n", expect_out: "12", expect_err: false, oracle: "net" },
    Prog { op: "RevSwap", label: "revswap_two_ints", src: "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "2\n1", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_double_is_identity", src: "## Main\n    Let mutable a be 7.\n    Let mutable b be 9.\n    Swap a and b.\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "7\n9", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_text_values", src: "## Main\n    Let mutable a be \"hello\".\n    Let mutable b be \"world\".\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "world\nhello", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_bools", src: "## Main\n    Let mutable a be true.\n    Let mutable b be false.\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "false\ntrue", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_in_function", src: "## To exchange (x: Int, y: Int) -> Int:\n    Let mutable a be x.\n    Let mutable b be y.\n    Swap a and b.\n    Return a - b.\n## Main\n    Show exchange(3, 8).\n", expect_out: "5", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_zero_and_negative", src: "## Main\n    Let mutable a be 0.\n    Let mutable b be -1.\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "-1\n0", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_large_i64_boundary", src: "## Main\n    Let mutable a be 9223372036854775807.\n    Let mutable b be 1.\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "1\n9223372036854775807", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_real_values", src: "## Main\n    Let mutable a be 3.5.\n    Let mutable b be 2.5.\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "2.5\n3.5", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_triple_is_single_swap", src: "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Swap a and b.\n    Swap a and b.\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "2\n1", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_equal_values", src: "## Main\n    Let mutable a be 5.\n    Let mutable b be 5.\n    Swap a and b.\n    Show a.\n    Show b.\n", expect_out: "5\n5", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_many_even_loop_is_identity", src: "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Let mutable i be 1.\n    While i is at most 10:\n        Swap a and b.\n        Set i to i + 1.\n    Show a.\n    Show b.\n", expect_out: "1\n2", expect_err: false, oracle: "plain" },
    Prog { op: "RevSwap", label: "revswap_many_odd_loop_net_swap", src: "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Let mutable i be 1.\n    While i is at most 5:\n        Swap a and b.\n        Set i to i + 1.\n    Show a.\n    Show b.\n", expect_out: "2\n1", expect_err: false, oracle: "plain" },
    Prog { op: "LoadToday", label: "today_direct", src: "## Main\n    Show today.\n", expect_out: "2024-10-04", expect_err: false, oracle: "plain" },
    Prog { op: "LoadToday", label: "today_let", src: "## Main\n    Let d be today.\n    Show d.\n", expect_out: "2024-10-04", expect_err: false, oracle: "plain" },
    Prog { op: "LoadNow", label: "now_direct", src: "## Main\n    Show now.\n", expect_out: "2024-10-04 12:34", expect_err: false, oracle: "plain" },
    Prog { op: "LoadNow", label: "now_let", src: "## Main\n    Let m be now.\n    Show m.\n", expect_out: "2024-10-04 12:34", expect_err: false, oracle: "plain" },
    Prog { op: "ChanClose", label: "close_then_recv", src: "## To produce (ch: Int):\n    Send 5 into ch.\n    Close ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "5", expect_err: false, oracle: "concurrent" },
    Prog { op: "ChanClose", label: "close_the_pipe", src: "## To produce (ch: Int):\n    Send 9 into ch.\n    Close the pipe ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Receive x from ch.\n    Show x.\n", expect_out: "9", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAwait", label: "await_worker", src: "## To worker:\n    Show 7.\n\n## Main\n    Let h be Launch a task to worker.\n    Await h.\n    Show 8.\n", expect_out: "7\n8", expect_err: false, oracle: "concurrent" },
    Prog { op: "TaskAwait", label: "wait_for_worker", src: "## To worker:\n    Show 3.\n\n## Main\n    Let h be Launch a task to worker.\n    Wait for h.\n    Show 4.\n", expect_out: "3\n4", expect_err: false, oracle: "concurrent" },
    Prog { op: "RevAdd", label: "rev_add", src: "## Main\n    Let mutable x be 5.\n    Reversibly:\n        Set x to x + 3.\n    Show x.\n", expect_out: "8", expect_err: false, oracle: "plain" },
    Prog { op: "RevAdd", label: "rev_add_uncompute_alias", src: "## Main\n    Let mutable x be 10.\n    Uncompute:\n        Set x to x + 90.\n    Show x.\n", expect_out: "100", expect_err: false, oracle: "plain" },
    Prog { op: "RevSub", label: "rev_sub", src: "## Main\n    Let mutable x be 5.\n    Reversibly:\n        Set x to x - 3.\n    Show x.\n", expect_out: "2", expect_err: false, oracle: "plain" },
    Prog { op: "RevSub", label: "rev_sub_to_negative", src: "## Main\n    Let mutable x be 2.\n    Reversibly:\n        Set x to x - 9.\n    Show x.\n", expect_out: "-7", expect_err: false, oracle: "plain" },
    Prog { op: "RevXor", label: "rev_xor", src: "## Main\n    Let mutable x be 3.\n    Reversibly:\n        Set x to x xor 5.\n    Show x.\n", expect_out: "6", expect_err: false, oracle: "plain" },
    Prog { op: "RevXor", label: "rev_xor_self_zero", src: "## Main\n    Let mutable x be 12.\n    Reversibly:\n        Set x to x xor 12.\n    Show x.\n", expect_out: "0", expect_err: false, oracle: "plain" },
    Prog { op: "RevNeg", label: "rev_neg", src: "## Main\n    Let mutable x be 7.\n    Reversibly:\n        Set x to -x.\n    Show x.\n", expect_out: "-7", expect_err: false, oracle: "plain" },
    Prog { op: "RevNeg", label: "rev_neg_double_identity", src: "## Main\n    Let mutable x be 4.\n    Reversibly:\n        Set x to -x.\n        Set x to -x.\n    Show x.\n", expect_out: "4", expect_err: false, oracle: "plain" },
];

const BACKENDS: &[&str] = &[
    "c",
    "cuda",
    "riscv",
    "rust",
    "wasm",
    "tenstorrent",
    "fabric",
];
fn compiles(b: &str, src: &str) -> bool {
    match b {
        "c" => compile_to_c(src).is_ok(),
        "cuda" => compile_to_cuda(src).is_ok(),
        "riscv" => compile_to_riscv(src).is_ok(),
        // ★ `Ok` PROVES NOTHING ON THIS TIER. `compile_to_rust` returns `Ok` for a program it has
        // REFUSED — the refusal is a `compile_error!` embedded in the returned source, which is
        // well-formed Rust that fails on purpose. An `is_ok()` here counted every refused op as
        // COVERED, and this file's floors are a completeness CLAIM ("Rust covers the full
        // source-emittable surface"), so the one thing it must not do is believe a refusal.
        "rust" => matches!(compile_to_rust(src), Ok(rust) if !rust.contains("compile_error!")),
        // ⚠️ THE WASM COLUMN CANNOT SAY THE SAME YET, and pretending otherwise would be the same
        // bug in a new place. `compile_to_wasm` returns `Ok` for bytes that do not VALIDATE, so
        // this asks only "did the backend produce a module?". Validation needs an engine and this
        // file is deliberately ungated (it runs in the default pass); the wasm-gated locks
        // instantiate for real. Treat FLOOR_WASM as "emitted", not "loads".
        "wasm" => compile_to_wasm(src).is_ok(),
        "tenstorrent" => compile_to_tenstorrent(src).is_ok(),
        // The fabric's whole-program lowering PANICS on an unsupported construct (a branchy Main, an
        // unlowered op); `catch_unwind` turns that into a clean `false` so a refusal is a coverage miss,
        // never a test crash. `AssertUnwindSafe` is sound here: `src` is `&str` and no state escapes.
        // ★ TWO ROUTES ONTO THE FABRIC, AND THE SECOND IS HELD TO A HIGHER BAR THAN THE FIRST.
        //
        // The combinational output-tape path is straight-line-Main-only: a `## Main` with a branch is
        // refused, because a conditional `Show` cannot ride a straight tape. That refusal is correct and
        // it is why this column sat at 80 — not for want of tiles, but because the CFG engine that DOES
        // lower branches is addressed by function name and the top level is not a named function.
        // `try_main_cfg_fabric` addresses it.
        //
        // The tape route is accepted on "it lowered without panicking", as it always was. The CFG route
        // is accepted only if the program's lowered OUTPUT STREAM equals the VM's — value for value, in
        // order. A `## Main` is judged by what it prints, so counting one as covered because it lowered
        // would be believing a refusal in a new costume, which the Rust column above exists to warn about.
        "fabric" => {
            catch_unwind(AssertUnwindSafe(|| {
                whole_program_fabric_from_source(src);
            }))
            .is_ok()
                || fabric_main_equals_vm(src)
        }
        _ => false,
    }
}
/// Does the CFG-routed `## Main` print exactly what the VM prints? Anything unlowerable, anything that
/// errors, and any disagreement in value or order is a coverage MISS, never a pass.
fn fabric_main_equals_vm(src: &str) -> bool {
    // Defence in depth: `try_main_cfg_fabric` refuses by type, but a corpus is exactly where an
    // unconverted internal `expect` would surface, and a probe must never be the thing that crashes.
    let Ok(Ok(pf)) = catch_unwind(AssertUnwindSafe(|| try_main_cfg_fabric(src))) else {
        return false;
    };
    let Ok((_ret, shown)) = catch_unwind(AssertUnwindSafe(|| {
        eval_program_cfg_fabric_traced(&pf, &[])
    })) else {
        return false;
    };
    let vm = vm_outcome(src);
    if vm.error.is_some() {
        return false;
    }
    let want: Vec<&str> = vm.output.split_whitespace().collect();
    let got: Vec<String> = shown.iter().map(|v| v.to_string()).collect();
    got.len() == want.len() && got.iter().zip(&want).all(|(a, b)| a == b)
}

fn floor(b: &str) -> usize {
    match b {
        "c" => FLOOR_C,
        "cuda" => FLOOR_CUDA,
        "riscv" => FLOOR_RISCV,
        "rust" => FLOOR_RUST,
        "wasm" => FLOOR_WASM,
        "tenstorrent" => FLOOR_TENSTORRENT,
        "fabric" => FLOOR_FABRIC,
        _ => 0,
    }
}

#[test]
fn the_oracle_agrees_on_every_corpus_program() {
    let mut bad = Vec::new();
    for p in CORPUS {
        let tw = tw_outcome(p.src);
        let vm = match p.oracle {
            "concurrent" => vm_outcome_concurrent(p.src),
            "net" => vm_outcome_net(p.src),
            _ => vm_outcome(p.src),
        };
        if (tw.output.trim(), tw.error.is_some()) != (vm.output.trim(), vm.error.is_some()) {
            bad.push(format!("{}/{}", p.op, p.label));
        }
    }
    assert!(
        bad.is_empty(),
        "oracle disagreement ({}): {:?}",
        bad.len(),
        &bad[..bad.len().min(25)]
    );
}
#[test]
fn every_op_is_classified_and_the_partition_covers_the_whole_isa() {
    let emittable: BTreeSet<&str> = CORPUS.iter().map(|p| p.op).collect();
    let hand: BTreeSet<&str> = HAND_BYTECODE_ONLY.iter().copied().collect();
    let host: BTreeSet<&str> = HOST_IO_ONLY.iter().copied().collect();
    assert!(
        emittable.is_disjoint(&hand),
        "op both emittable+hand: {:?}",
        emittable.intersection(&hand).collect::<Vec<_>>()
    );
    assert!(
        emittable.is_disjoint(&host),
        "op both emittable+host-io: {:?}",
        emittable.intersection(&host).collect::<Vec<_>>()
    );
    assert!(
        hand.is_disjoint(&host),
        "op both hand+host-io: {:?}",
        hand.intersection(&host).collect::<Vec<_>>()
    );

    // ★ ASK THE ISA HOW BIG IT IS. The literal this replaced (`115`) was two behind the real ISA,
    // which is exactly how `Tcp` and `Serve` came to be in no column at all.
    let isa: BTreeSet<&str> = logicaffeine_compile::vm::Op::all_variants()
        .iter()
        .map(|o| o.name())
        .collect();
    let classified: BTreeSet<&str> = emittable
        .union(&hand)
        .copied()
        .collect::<BTreeSet<_>>()
        .union(&host)
        .copied()
        .collect();
    let missing: Vec<&&str> = isa.difference(&classified).collect();
    assert!(
        missing.is_empty(),
        "{} ISA op(s) are in NO column of this matrix and NO exclusion list: {missing:?}\n\
         Every op is either exercised by the corpus, documented as hand-bytecode-only, or \
         documented as host-IO. An op in none of the three is one this differential never runs.",
        missing.len()
    );
    let stale: Vec<&&str> = classified.difference(&isa).collect();
    assert!(
        stale.is_empty(),
        "classified op(s) that are not in the ISA (renamed or removed?): {stale:?}"
    );
}
#[test]
fn every_backend_covers_at_least_its_floor_of_ops() {
    let mut rep = Vec::new();
    for b in BACKENDS {
        let mut cov: BTreeSet<&str> = BTreeSet::new();
        for p in CORPUS {
            if compiles(b, p.src) {
                cov.insert(p.op);
            }
        }
        rep.push(format!("{b}={}", cov.len()));
        assert!(
            cov.len() >= floor(b),
            "backend `{b}` covers {} ops, BELOW floor {} — regression",
            cov.len(),
            floor(b)
        );
    }
    eprintln!(
        "GAUNTLET coverage: {} ({} programs)",
        rep.join("  "),
        CORPUS.len()
    );
}
#[test]
fn no_source_emittable_op_compiles_nowhere() {
    const AOT_ELIDED: &[&str] = &["Obs"];
    let ops: BTreeSet<&str> = CORPUS.iter().map(|p| p.op).collect();
    let mut nowhere = Vec::new();
    for op in ops {
        if AOT_ELIDED.contains(&op) {
            continue;
        }
        if !CORPUS
            .iter()
            .filter(|p| p.op == op)
            .any(|p| BACKENDS.iter().any(|b| compiles(b, p.src)))
        {
            nowhere.push(op);
        }
    }
    assert!(nowhere.is_empty(), "ops compiling nowhere: {nowhere:?}");
}
