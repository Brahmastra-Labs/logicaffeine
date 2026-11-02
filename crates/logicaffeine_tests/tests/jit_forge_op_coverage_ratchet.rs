//! FORGE OP-COVERAGE RATCHET — the copy-and-patch forge JIT is hand-written codegen (NOT Futamura-
//! derived / correct-by-construction), so every op-class it compiles needs an external correctness
//! net. This runs a hot program for each ISA op-class through the forge tier and asserts it is
//! byte-identical to the tree-walker (the trusted reference). One test reports EVERY divergence at
//! once — the ratchet: drive the divergence list to empty and keep it there. A new JIT-visible
//! construct adds a row here.
//!
//! Ops the forge never compiles (IO/concurrency/networking/CRDT/temporal — Show/Args/ChanSend/Spawn/
//! Net*/Crdt*/Sleep/LoadNow/FailWith) bail to the VM and are covered by the tier-differential suite;
//! this file targets the COMPUTE ops the forge actually emits native code for.

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// (label, program) — each program runs its construct in a HOT loop (≥ the tier-up threshold) so the
/// forge native body is what executes, and `Show`s a deterministic result the tree-walker also computes.
const CASES: &[(&str, &str)] = &[
    // ── integer arithmetic (Add/Sub/Mul/Div/Mod/FloorDiv/Pow + Move/Add-in-loop) ──
    ("int_arith_mix", "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 600:\n    Set acc to acc + ((k * 3) - (k mod 7) + (k / 2)).\n    Set k to k + 1.\nShow acc.\n"),
    ("int_floordiv", "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 600:\n    Set acc to acc + (k // 3).\n    Set k to k + 1.\nShow acc.\n"),
    ("int_pow", "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 600:\n    Set acc to acc + (2 ** (k mod 6)).\n    Set k to k + 1.\nShow acc.\n"),
    ("int_div_pow2", "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 600:\n    Set acc to acc + (k / 4).\n    Set k to k + 1.\nShow acc.\n"),
    // ── comparisons as branches AND as accumulated flags (Lt/Gt/LtEq/GtEq/Eq/NotEq/Not) ──
    ("cmp_branches", "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 600:\n    If k is less than 200:\n        Set acc to acc + 1.\n    If k is greater than 400:\n        Set acc to acc + 2.\n    If (k mod 5) is equal to 0:\n        Set acc to acc + 4.\n    Set k to k + 1.\nShow acc.\n"),
    // ── bitwise (BitAnd/BitOr/BitXor/Shl/Shr) ──
    ("bitwise", "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 600:\n    Set acc to acc + ((k & 6) | (k xor 3)).\n    Set k to k + 1.\nShow acc.\n"),
    ("shifts", "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 600:\n    Set acc to acc + ((k shifted left by 2) - (k shifted right by 1)).\n    Set k to k + 1.\nShow acc.\n"),
    // ── float arithmetic + comparison (folded to an int accumulator for exactness) ──
    ("float_arith", "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 600:\n    Let f be (toReal(k) * 1.5) - 0.25.\n    If f is greater than 100.0:\n        Set acc to acc + 1.\n    Set k to k + 1.\nShow acc.\n"),
    // ── list build + sum (NewEmptyList/ListPush/Index/Length in a mode-B region) ──
    ("list_build_sum", "## Main\nLet mutable xs be a new Seq of Int.\nLet mutable k be 1.\nWhile k is at most 40:\n    Push (k * k) to xs.\n    Set k to k + 1.\nLet mutable acc be 0.\nLet mutable r be 0.\nWhile r is less than 30:\n    Let mutable i be 1.\n    While i is at most (length of xs):\n        Set acc to acc + (item i of xs).\n        Set i to i + 1.\n    Set r to r + 1.\nShow acc.\n"),
    // ── list SetIndex + read-back ──
    ("list_setindex", "## Main\nLet mutable xs be a new Seq of Int.\nLet mutable k be 1.\nWhile k is at most 20:\n    Push 0 to xs.\n    Set k to k + 1.\nLet mutable acc be 0.\nLet mutable r be 0.\nWhile r is less than 40:\n    Let mutable i be 1.\n    While i is at most 20:\n        Set item i of xs to (i * 2).\n        Set i to i + 1.\n    Set acc to acc + (item 10 of xs).\n    Set r to r + 1.\nShow acc.\n"),
    // ── list-returning function (quicksort-shape: must tier via the pin machinery) ──
    ("list_return_fn", "## To dblAll (xs: Seq of Int) -> Seq of Int:\n    Let out be a new Seq of Int.\n    Repeat for v in xs:\n        Push (v * 2) to out.\n    Return out.\n## Main\nLet base be [1, 2, 3, 4, 5].\nLet mutable acc be 0.\nLet mutable k be 0.\nWhile k is less than 500:\n    Let d be dblAll(base).\n    Set acc to acc + (item 3 of d).\n    Set k to k + 1.\nShow acc.\n"),
    // ── scalar-return list-search (the shape that MISCOMPILED — now declined to VM) ──
    ("scalar_list_search", "## To firstGE (xs: Seq of Int, t: Int) -> Int:\n    Let mutable i be 1.\n    While i is at most (length of xs):\n        If (item i of xs) is at least t:\n            Return i.\n        Set i to i + 1.\n    Return 0.\n## Main\nLet xs be [3, 1, 4, 1, 5, 9, 2, 6].\nLet mutable acc be 0.\nLet mutable k be 0.\nWhile k is less than 500:\n    Set acc to acc + firstGE(xs, 5).\n    Set k to k + 1.\nShow acc.\n"),
    // ── map build + guarded lookup (NewEmptyMap/SetIndex/Contains/Index) ──
    ("map_lookup", "## Main\nLet mutable m be a new Map of Int to Int.\nSet item 1 of m to 10.\nSet item 2 of m to 20.\nSet item 3 of m to 30.\nLet mutable acc be 0.\nLet mutable k be 0.\nWhile k is less than 500:\n    Let key be (k mod 4) + 1.\n    If m contains key:\n        Set acc to acc + (item key of m).\n    Set k to k + 1.\nShow acc.\n"),
    // ── recursion (self Call + early return) ──
    ("recursion_fib", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\nLet mutable acc be 0.\nLet mutable k be 0.\nWhile k is less than 500:\n    Set acc to acc + fib(15).\n    Set k to k + 1.\nShow acc.\n"),
    // ── nested loops + break-ish accumulation ──
    ("nested_loops", "## Main\nLet mutable acc be 0.\nLet mutable a be 1.\nWhile a is at most 30:\n    Let mutable b be 1.\n    While b is at most 30:\n        Set acc to acc + (a * b).\n        Set b to b + 1.\n    Set a to a + 1.\nShow acc.\n"),
    // ── for-each over a range (NewRange/IterPrepare/IterNext) ──
    ("range_foreach", "## Main\nLet mutable acc be 0.\nLet mutable r be 0.\nWhile r is less than 40:\n    Repeat for i from 1 to 25:\n        Set acc to acc + (i * i).\n    Set r to r + 1.\nShow acc.\n"),
    // ── text bytes scan (text_bytes/Index/Length + early return) ──
    ("text_scan", "## To countByte (s: Text, b: Int) -> Int:\n    Let sb be text_bytes(s).\n    Let mutable c be 0.\n    Let mutable i be 1.\n    While i is at most (length of sb):\n        If (item i of sb) is equal to b:\n            Set c to c + 1.\n        Set i to i + 1.\n    Return c.\n## Main\nLet mutable acc be 0.\nLet mutable k be 0.\nWhile k is less than 500:\n    Set acc to acc + countByte(\"hello world wonderful\", 111).\n    Set k to k + 1.\nShow acc.\n"),
    // ── tuple make + destructure (NewTuple/DestructureTuple) ──
    ("tuple_destructure", "## To divmod (n: Int) -> (Int, Int):\n    Return (n / 7, n mod 7).\n## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 500:\n    Let (q, rem) be divmod(k).\n    Set acc to acc + (q + rem).\n    Set k to k + 1.\nShow acc.\n"),
    // ── mixed: string builder via list of bytes (SeqConcat-ish / text_from_bytes) ──
    ("bytes_reverse", "## To rev (s: Text) -> Text:\n    Let sb be text_bytes(s).\n    Let out be a new Seq of Int.\n    Let mutable i be (length of sb).\n    While i is at least 1:\n        Push item i of sb to out.\n        Set i to i - 1.\n    Return text_from_bytes(out).\n## Main\nLet mutable acc be 0.\nLet mutable k be 0.\nWhile k is less than 500:\n    Let rr be rev(\"abcdefgh\").\n    Set acc to acc + (length of text_bytes(rr)).\n    Set k to k + 1.\nShow acc.\n"),
];

#[test]
fn every_forge_op_class_matches_the_tree_walker() {
    let mut diverged: Vec<String> = Vec::new();
    for (label, src) in CASES {
        let tw = tw_outcome_with_args(src, &[]);
        let tier = ForgeTier::new();
        let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
        if tw.error.is_some() {
            diverged.push(format!("{label}: tree-walker ERRORED: {:?}", tw.error));
            continue;
        }
        if forge.error.is_some() {
            diverged.push(format!("{label}: forge ERRORED: {:?}", forge.error));
            continue;
        }
        if tw.output.trim() != forge.output.trim() {
            diverged.push(format!(
                "{label}: forge={:?} tw={:?}",
                forge.output.trim(),
                tw.output.trim()
            ));
        }
    }
    assert!(
        diverged.is_empty(),
        "forge JIT diverged from the tree-walker on {} op-class(es):\n{}",
        diverged.len(),
        diverged.join("\n")
    );
}
