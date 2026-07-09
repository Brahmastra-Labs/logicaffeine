//! Phase: CRDT Tier Parity (interpreter ↔ AOT / the Futamura lock)
//!
//! The codegen/AOT tier lowers a `Shared` struct's CRDT fields to the real
//! `logicaffeine_data::crdt` types (PNCounter / ORSet / RGA / MVRegister) and runs
//! their full convergent semantics. The interpreter (tree-walker) tier MUST agree:
//! the SAME program, run through both tiers, has to produce identical output.
//!
//! These differential tests LOCK the two tiers together. A divergence here is the
//! futamura-invariant violation CLAUDE.md calls a regression — a program that *means
//! something different* interpreted vs compiled. An OR-Set unit test against
//! `logicaffeine_data` alone cannot catch this, because it never exercises the
//! interpreter at all; only a cross-tier program does.
//!
//! AUDIT (RED-first): the counter case is already at parity (the interpreter does the
//! same signed-integer counter math). The SharedSet / SharedSequence / Divergent cases
//! are RED until the interpreter is bridged to the rich CRDTs — today the tree-walker
//! either rejects the field mutation or hard-errors ("Use compiled Rust"). Each test
//! flips GREEN as its bridge lands, closing the tier gap one CRDT at a time.

mod common;
use common::{run_interpreter, run_logos};

/// Run `source` through BOTH tiers and assert they agree. Unlike the bare
/// `assert_compiled_equals_interpreted` (which is satisfied when *both* tiers fail),
/// this REQUIRES the AOT tier to succeed first — so it can never pass vacuously by
/// both engines erroring out. The AOT tier is the oracle; the interpreter must match
/// it byte-for-byte on stdout.
fn assert_tiers_agree(source: &str) {
    let aot = run_logos(source);
    assert!(
        aot.success,
        "AOT/compiled tier failed — cannot serve as the parity oracle.\n\
         Source:\n{}\n\nstderr:\n{}\n\nGenerated Rust:\n{}",
        source, aot.stderr, aot.rust_code
    );

    let interp = run_interpreter(source);
    assert!(
        interp.success,
        "TIER INCONSISTENCY: AOT succeeded but the INTERPRETER failed.\n\
         The interpreter gives weaker CRDT semantics than the compiler.\n\
         Source:\n{}\n\ninterpreter error:\n{}",
        source, interp.error
    );

    assert_eq!(
        interp.output.trim(),
        aot.stdout.trim(),
        "TIER MISMATCH: interpreter and AOT produced different output.\n\
         Source:\n{}\n\ninterpreter:\n{:?}\n\nAOT:\n{:?}\n\nGenerated Rust:\n{}",
        source,
        interp.output.trim(),
        aot.stdout.trim(),
        aot.rust_code
    );
}

/// PN-counter (`Tally`): increase then decrease. Already at parity — the interpreter
/// performs the same signed counter math the compiled `PNCounter` does.
#[test]
fn crdt_tier_parity_counter_increase_decrease() {
    assert_tiers_agree(
        r#"
## Definition
A Game is Shared and has:
    a score, which is a Tally.

## Main
Let g be a new Game.
Increase g's score by 100.
Decrease g's score by 30.
Show g's score.
"#,
    );
}

/// OR-Set (`SharedSet of Text`): add an element, then test membership. RED until the
/// interpreter holds a real `ORSet` — today `Add E to X's field` is rejected because
/// the collection is a field access, not a bare identifier.
#[test]
fn crdt_tier_parity_shared_set_add_and_contains() {
    assert_tiers_agree(
        r#"
## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let p be a new Party.
Add "Alice" to p's guests.
If p's guests contains "Alice":
    Show "Found Alice".
Otherwise:
    Show "Not found".
"#,
    );
}

/// RGA (`SharedSequence of Text`): append two elements, read the length. RED until the
/// interpreter holds a real `RGA` — today `Append` returns "not supported in the
/// interpreter. Use compiled Rust."
#[test]
fn crdt_tier_parity_shared_sequence_append_and_length() {
    assert_tiers_agree(
        r#"
## Definition
A Document is Shared and has:
    a lines, which is a SharedSequence of Text.

## Main
Let d be a new Document.
Append "Line 1" to d's lines.
Append "Line 2" to d's lines.
Show length of d's lines.
"#,
    );
}

/// MV-Register (`Divergent Text`): set a value, read it back. RED until the interpreter
/// holds a real `MVRegister` — today the divergent field has no interpreter value model.
#[test]
fn crdt_tier_parity_divergent_set_and_show() {
    assert_tiers_agree(
        r#"
## Definition
A WikiPage is Shared and has:
    a title, which is a Divergent Text.

## Main
Let p be a new WikiPage.
Set p's title to "Draft".
Show p's title.
"#,
    );
}

// =============================================================================
// CONCURRENT MERGE — the deep lock. A single-replica program can be served by a
// plain Set/List lookalike; only a CONCURRENT merge distinguishes a real CRDT.
// These prove the interpreter's join is byte-identical to the compiled tier's.
// =============================================================================

/// Two replicas each add a distinct element, then merge: the result is the UNION. This is
/// the basic convergence both tiers must agree on.
#[test]
fn crdt_tier_parity_shared_set_concurrent_add_union() {
    assert_tiers_agree(
        r#"
## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let mutable a be a new Party.
Let mutable b be a new Party.
Add "Alice" to a's guests.
Add "Bob" to b's guests.
Merge b into a.
If a's guests contains "Alice":
    Show "alice-in".
Otherwise:
    Show "alice-out".
If a's guests contains "Bob":
    Show "bob-in".
Otherwise:
    Show "bob-out".
"#,
    );
}

/// THE distinguishing OR-Set property: replica `a` adds then removes "X"; replica `b`
/// CONCURRENTLY adds "X"; after merge, `b`'s add wins (it was never observed by `a`'s
/// remove), so "X" survives. A naive grow-set with one tombstone would drop it. Both tiers
/// must produce the SAME answer — this is what a lookalike `Set` cannot fake.
#[test]
fn crdt_tier_parity_shared_set_add_wins_over_concurrent_remove() {
    assert_tiers_agree(
        r#"
## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let mutable a be a new Party.
Let mutable b be a new Party.
Add "X" to a's guests.
Add "X" to b's guests.
Remove "X" from a's guests.
Merge b into a.
If a's guests contains "X":
    Show "present".
Otherwise:
    Show "absent".
"#,
    );
}

/// RGA merge: two replicas each append, then merge. Both tiers must agree on the converged
/// length (the elements survive the join).
#[test]
fn crdt_tier_parity_shared_sequence_concurrent_merge_length() {
    assert_tiers_agree(
        r#"
## Definition
A Document is Shared and has:
    a lines, which is a SharedSequence of Text.

## Main
Let mutable a be a new Document.
Let mutable b be a new Document.
Append "a1" to a's lines.
Append "b1" to b's lines.
Merge b into a.
Show length of a's lines.
"#,
    );
}

// =============================================================================
// §13 GUIDE EXAMPLES — the exact programs shipped in the Distributed Types (CRDTs)
// guide section, locked to their CORRECT output (not merely "runs without error").
// Promoting these out of REQUIRES_COMPILATION is only honest if they produce the
// right answer in the interpreter — and match the compiled tier where it supports them.
// =============================================================================

/// Assert the interpreter produces exactly `expected` (trimmed).
fn assert_interp_output(source: &str, expected: &str) {
    let r = run_interpreter(source);
    assert!(r.success, "interpreter failed:\n{}\nerr: {}", source, r.error);
    assert_eq!(r.output.trim(), expected, "interpreter output mismatch:\n{}", source);
}

#[test]
fn guide_crdt_divergent() {
    let src = "## Definition\nA WikiPage is Shared and has:\n    a title, which is Divergent Text.\n\n## Main\nLet mutable page be a new WikiPage.\nSet page's title to \"Draft\".\nShow page's title.\nResolve page's title to \"Final\".\nShow page's title.";
    assert_interp_output(src, "Draft\nFinal");
    assert_tiers_agree(src);
}

#[test]
fn guide_crdt_sharedset() {
    let src = "## Definition\nA Party is Shared and has:\n    a guests, which is a SharedSet of Text.\n\n## Main\nLet mutable p be a new Party.\nAdd \"Alice\" to p's guests.\nAdd \"Bob\" to p's guests.\nRemove \"Alice\" from p's guests.\nIf p's guests contains \"Bob\":\n    Show \"Bob is invited\".\nShow length of p's guests.";
    assert_interp_output(src, "Bob is invited\n1");
    assert_tiers_agree(src);
}

/// `SharedSet (AddWins)` / `SharedSet (RemoveWins)` — the interpreter honors the declared
/// bias (RemoveWins is a distinct `ORSet<_, RemoveWins>`) and renders a set as `{…}`. The
/// AOT tier cannot `Show` a raw OR-Set, so this is interpreter-locked on its output.
#[test]
fn guide_crdt_sharedset_bias() {
    let src = "## Definition\nA Moderation is Shared and has:\n    a tags, which is a SharedSet (AddWins) of Text.\n    a blocked, which is a SharedSet (RemoveWins) of Text.\n\n## Main\nLet mutable m be a new Moderation.\nAdd \"safe\" to m's tags.\nAdd \"spammer\" to m's blocked.\nShow m's tags.\nShow m's blocked.";
    assert_interp_output(src, "{safe}\n{spammer}");
}

#[test]
fn guide_crdt_sequence() {
    let src = "## Definition\nA Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n\n## Main\nLet mutable doc be a new Document.\nAppend \"Line 1\" to doc's lines.\nAppend \"Line 2\" to doc's lines.\nAppend \"Line 3\" to doc's lines.\nShow length of doc's lines.";
    assert_interp_output(src, "3");
    assert_tiers_agree(src);
}

#[test]
fn guide_crdt_collaborative() {
    let src = "## Definition\nA Editor is Shared and has:\n    a text, which is a CollaborativeSequence of Text.\n\n## Main\nLet mutable e be a new Editor.\nAppend \"Hello\" to e's text.\nAppend \" \" to e's text.\nAppend \"World\" to e's text.\nShow length of e's text.";
    assert_interp_output(src, "3");
    assert_tiers_agree(src);
}
