//! P2 — the `Word32` ring computes correctly through the AOT-native tier. A program using
//! wrapping add, `xor`, `rotl`, and a `Seq of Word32` is compiled the full way (LOGOS →
//! generated Rust → `rustc`/`cargo` → native binary) and its output asserted byte-identical to
//! the tree-walker (LOGOS's semantic ground truth). This proves the emitted `a + b` / `a ^ b` /
//! `rotl(..)` over the `logicaffeine_base::Word32` newtype is ring-correct, not panicking-`+`.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;
use logicaffeine_compile::compile::tw_outcome;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Wrapping add (2³²−1 + 1 → 0), bit `xor`, `rotl`, and a mutating `Seq of Word32` whose stored
/// sum wraps (4294967200 + 200 → 104). Every value is a `Word32`, so the ops must wrap mod 2³².
const PROG: &str = "## Main\n\
    Let a be word32(4294967295).\n\
    Let b be word32(1).\n\
    Let c be a + b.\n\
    Show c.\n\
    Let d be word32(305419896).\n\
    Show rotl(d xor b, 8).\n\
    Let mutable xs be [word32(4294967200), word32(200)].\n\
    Set item 1 of xs to (item 1 of xs) + (item 2 of xs).\n\
    Show item 1 of xs.\n";

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT Word32 ring gate; run on demand"]
fn word32_aot_matches_treewalker_and_wraps() {
    let tw = tw_outcome(PROG);
    assert_eq!(tw.error, None, "tree-walker runs clean: {:?}", tw.error);
    // Sanity anchor independent of the tier: 2³²−1 + 1 wraps to 0.
    assert!(
        norm(&tw.output).starts_with('0'),
        "wrapping add must give 0, got {:?}",
        tw.output
    );

    let aot = run_logos_with_args(PROG, &[]);
    assert!(
        aot.success,
        "AOT compile+run failed:\n--- stderr ---\n{}\n--- generated rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    assert_eq!(
        norm(&aot.stdout),
        norm(&tw.output),
        "compiled Word32 must equal the tree-walker — the ℤ/2³² ring carries through AOT"
    );
}

/// A VERB-named Word32 helper function over `Seq of Word32`, exercised through the full optimizer
/// (function codegen + inlining + partial evaluation) compiled native, asserted byte-identical to
/// the tree-walker. Proves "all the way through the PE" AND that verb-stem function names work end
/// to end (`mix` is an English verb) — the discovery pass now recognizes `Word32` as a primitive,
/// so `## To <name>` resolves as a function definition regardless of the name's word-class.
// NOTE: written as ONE literal with explicit `\n    ` indentation — a `\`-continuation would
// strip the leading spaces and de-indent the `## To` body (Logos is indentation-sensitive).
const PROG_FN: &str = "## To mix (xs: Seq of Word32) -> Seq of Word32:\n    Let a be item 1 of xs.\n    Let b be item 2 of xs.\n    Set item 1 of xs to rotl(a xor b, 8).\n    Return xs.\n## Main\nLet mutable xs be [word32(305419896), word32(1)].\nSet xs to mix(xs).\nShow item 1 of xs.\n";

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — AOT verb-named Word32 function/PE gate; run on demand"]
fn word32_verb_function_through_optimizer_matches_treewalker() {
    let tw = tw_outcome(PROG_FN);
    assert_eq!(tw.error, None, "tree-walker runs clean: {:?}", tw.error);
    let aot = run_logos_with_args(PROG_FN, &[]);
    assert!(
        aot.success,
        "AOT compile+run failed:\n--- stderr ---\n{}\n--- generated rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    assert_eq!(
        norm(&aot.stdout),
        norm(&tw.output),
        "a verb-named Word32 function must survive the optimizer (inlining + PE) through AOT"
    );
}

#[test]
fn verb_and_word32_function_names_all_parse_via_tw() {
    // After the discovery-pass Word-primitive fix, user `## To <name>` functions over Word32 —
    // INCLUDING verb-stem names — parse + run via tw_outcome. These are the exact shapes that
    // failed against the pre-fix binary; each is checked with a fresh single tw_outcome call.
    let cases: &[(&str, &str)] = &[
        ("verb mix", "## To mix (xs: Seq of Word32) -> Seq of Word32:\n    Let a be item 1 of xs.\n    Let b be item 2 of xs.\n    Set item 1 of xs to rotl(a xor b, 8).\n    Return xs.\n## Main\nLet mutable xs be [word32(305419896), word32(1)].\nSet xs to mix(xs).\nShow item 1 of xs.\n"),
        ("nonverb qfn", "## To qfn (xs: Seq of Word32) -> Seq of Word32:\n    Let a be item 1 of xs.\n    Let b be item 2 of xs.\n    Set item 1 of xs to rotl(a xor b, 8).\n    Return xs.\n## Main\nLet mutable xs be [word32(305419896), word32(1)].\nSet xs to qfn(xs).\nShow item 1 of xs.\n"),
        ("digit-suffix kdf32", "## To kdf32 (xs: Seq of Word32) -> Seq of Word32:\n    Set item 1 of xs to rotl(item 1 of xs, 8).\n    Return xs.\n## Main\nLet mutable xs be [word32(5), word32(1)].\nSet xs to kdf32(xs).\nShow item 1 of xs.\n"),
        ("verb scalar Word32", "## To mix (x: Word32, y: Word32) -> Word32:\n    Let z be x xor y.\n    Set z to rotl(z, 8).\n    Return z.\n## Main\nLet r be mix(word32(305419896), word32(1)).\nShow r.\n"),
    ];
    for (label, src) in cases {
        let e = tw_outcome(src).error;
        assert_eq!(e, None, "case '{label}' must parse + run via tw_outcome, got: {e:?}");
    }
}

#[test]
fn control_nonword_function_then_main_via_tw() {
    // A plain Int function + Main through `tw_outcome` — the baseline that the Word32 ring builds on.
    let src = "## To dbl (x: Int, y: Int) -> Int:\n    Let z be x + y.\n    Return z.\n## Main\nLet r be dbl(5, 7).\nShow r.\n";
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "plain Int function + Main via tw_outcome must parse: {:?}", tw.error);
    assert_eq!(tw.output.trim(), "12", "dbl(5,7) = 12");
}
