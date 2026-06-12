//! Phase B2 — Partially-static data (PE_IMPROVE §5, closes gap G1; THE keystone).
//!
//! Today `isStatic` is all-or-nothing: an aggregate with one dynamic field is wholly
//! dynamic, so reading a *static* field of it residualizes instead of folding. B2 makes
//! the PE represent partially-static aggregates so the static parts fold and only the
//! dynamic parts residualize — the mechanism that makes interpreter specialization
//! Jones-optimal (the interpreter's `env` becomes partially static: shape known, runtime
//! values dynamic). RED-first per CLAUDE.md.
//!
//! Dynamic operands are constructed with a range > 64 loop (not unrolled → the accumulator
//! is genuinely dynamic in the residual). NOTE: write programs as `"\` + real newlines +
//! real indentation — do NOT use `\n\` line continuation (it strips the next line's leading
//! whitespace, breaking loop-body indentation).

mod pe_support;

use pe_support::*;

/// Reading the STATIC field of a partially-static struct must fold to its constant, even
/// though the other field is dynamic. RED today: all-or-nothing isStatic makes the whole
/// struct dynamic, so `b's base` residualizes instead of folding to 5.
#[test]
fn partial_struct_static_field_folds() {
    let program = "\
## A Box has:
    A base: Int.
    A flex: Int.

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let b be a new Box with base 5 and flex d.
Show b's base.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 5"),
        "static field `base` should fold to 5 (residual still does field access):\n{}",
        residual
    );
    assert_run_equals(program, "5");
}

/// Reading the DYNAMIC field residualizes and runs correctly (flex = sum 1..100 = 5050).
#[test]
fn partial_struct_dynamic_field_residualizes() {
    let program = "\
## A Box has:
    A base: Int.
    A flex: Int.

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let b be a new Box with base 5 and flex d.
Show b's flex.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "dynamic field must residualize cleanly, no garbage:\n{}",
        residual
    );
    assert_run_equals(program, "5050");
}

/// Regression guard: an all-static struct must behave exactly as today — the static field
/// folds (this already works; B2 must not regress it).
#[test]
fn all_static_struct_field_still_folds() {
    let program = "\
## A Box has:
    A base: Int.
    A flex: Int.

## Main
Let b be a new Box with base 5 and flex 7.
Show b's base.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 5"),
        "all-static struct field should fold to 5:\n{}",
        residual
    );
    assert_run_equals(program, "5");
}

/// B2.1 — `SetField` on one field should invalidate only that field's static fact (the
/// unmutated `flex` keeps folding). DEFERRED to EXODIA Phase 1 / AliasInfo: folding struct
/// field *mutations* (tracking a partial CNewVariant across a CMapSet) is unsound without
/// alias/escape analysis — the self-interpreter's structs escape/alias in ways static
/// tracking can't follow, and enabling it regresses 176 Futamura tests. The PE currently
/// (soundly) invalidates the whole struct on mutation; partial field *access* (B2.0) folds.
/// This test documents the target precision and unblocks once the Oracle's AliasInfo domain
/// lands. See [[pe-exodia-roadmap-combo]] / task #17.
/// SOUNDNESS (not ignored): a struct field mutation is preserved — the residual runs to the
/// correct value even though the PE conservatively invalidates the whole struct on mutation
/// (it just doesn't *fold* the unmutated field; that optimization is the ignored test below).
#[test]
fn partial_struct_setfield_is_sound() {
    let program = "\
## A Box has:
    A base: Int.
    A flex: Int.

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let mutable b be a new Box with base 1 and flex 7.
Set b's base to d.
Show b's flex.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "no garbage in residual:\n{}",
        residual
    );
    assert_run_equals(program, "7");
}

/// B2.1 delivered: SetField on one field folds only the unmutated field's static fact. (The
/// earlier "176 Futamura regression" was a misdiagnosis — a codegen move bug, `Push peVal`
/// moving the value the CMapSet emit reused, fixed with `copy of peVal`. The folding is sound.)
#[test]
fn partial_struct_setfield_invalidates_only_mutated() {
    let program = "\
## A Box has:
    A base: Int.
    A flex: Int.

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let mutable b be a new Box with base 1 and flex 7.
Set b's base to d.
Show b's flex.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 7"),
        "unmutated static field `flex` should still fold to 7:\n{}",
        residual
    );
    assert_run_equals(program, "7");
}

// ===========================================================================
// B2.2 — List / Tuple / Map partial-static folding.
// A list/tuple with a static spine + one dynamic element: a static-index read folds;
// a dynamic-index read residualizes. (`d` is dynamic via a range>64 loop.)
// ===========================================================================

/// Reading a STATIC index of a partially-static list folds to the literal.
#[test]
fn partial_list_static_index_folds() {
    let program = "\
## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let xs be [1, d, 3].
Show item 1 of xs.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 1"),
        "static index 1 should fold to 1:\n{}",
        residual
    );
    assert_run_equals(program, "1");
}

/// Reading a different static index of the same partial list folds too (index 3 -> 3).
#[test]
fn partial_list_static_index_3_folds() {
    let program = "\
## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let xs be [1, d, 3].
Show item 3 of xs.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 3"),
        "static index 3 should fold to 3:\n{}",
        residual
    );
    assert_run_equals(program, "3");
}

/// Reading the DYNAMIC element residualizes and runs correctly (d = 5050).
#[test]
fn partial_list_dynamic_index_residualizes() {
    let program = "\
## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let xs be [1, d, 3].
Show item 2 of xs.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        !residual.contains("__unresolvable"),
        "dynamic element read must residualize cleanly:\n{}",
        residual
    );
    assert_run_equals(program, "5050");
}

/// A tuple with a static spine + dynamic element: static-index read folds.
#[test]
fn partial_tuple_static_index_folds() {
    let program = "\
## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let t be (1, d, 3).
Show item 1 of t.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 1"),
        "static tuple index 1 should fold to 1:\n{}",
        residual
    );
    assert_run_equals(program, "1");
}

// ===========================================================================
// B2.3 — Nested partial-static + flows-into-call.
// ===========================================================================

/// Nested struct (struct-in-struct), static spine + dynamic leaf: the static nested path
/// folds even though a sibling field is dynamic.
#[test]
fn partial_nested_struct_path_folds() {
    let program = "\
## A Inner has:
    A x: Int.

## A Outer has:
    A inner: Inner.
    A tag: Int.

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let inr be a new Inner with x 42.
Let o be a new Outer with inner inr and tag d.
Show o's inner's x.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 42"),
        "nested static path o.inner.x should fold to 42:\n{}",
        residual
    );
    assert_run_equals(program, "42");
}

/// A partially-static struct passed to a function: the call specializes on the static
/// field (the residual must not leave a dangling call, since projection emits only main).
#[test]
fn partial_flows_into_call() {
    let program = "\
## A Box has:
    A base: Int.
    A flex: Int.

## To getBase (bx: Box) -> Int:
    Return bx's base.

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let b be a new Box with base 5 and flex d.
Show getBase(b).";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 5"),
        "getBase should specialize on the static field base=5:\n{}",
        residual
    );
    assert_run_equals(program, "5");
}

// ===========================================================================
// B2 keystone demo — a partially-static record (static field names/shape, a dynamic
// value) dissolves to ZERO interpreter dispatch: the static field access folds to its
// constant even though a sibling field is dynamic. This is the mechanism that makes
// interpreter specialization Jones-optimal (the interpreter's env becomes partially
// static). The full self-interpreter-over-a-record demo is Phase D territory.
// ===========================================================================
#[test]
fn keystone_partial_record_dissolves_to_zero_dispatch() {
    let program = "\
## A Env has:
    A x: Int.
    A y: Int.

## Main
Let mutable acc be 0.
Repeat for i from 1 to 100:
    Set acc to acc + 1.
Let e be a new Env with x 10 and y acc.
Show e's x.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 10"),
        "static field x of a partially-static record must fold to 10:\n{}",
        residual
    );
    assert_eq!(
        count_dispatch(&residual),
        0,
        "residual must have ZERO interpreter dispatch (the partial-static env dissolved):\n{}",
        residual
    );
    assert_run_equals(program, "10");
}

// ===========================================================================
// B2.4 — Aliasing safety (conservative + correct).
// Seq/Set/Map are reference types (Rc<RefCell>): `Let a be s` aliases. A mutation
// through `a` must invalidate `s`'s static facts so a later read of `s` is not folded
// to a stale value. Correctness over optimization.
// ===========================================================================

/// Mutation through an alias must invalidate the source's static facts: after
/// `Let a be s. Set item 1 of a to d.`, reading `item 1 of s` must see the runtime value
/// (d), not the stale literal 1.
///
/// Reference aggregates (LogosSeq/Set are Rc<RefCell>): `Let a be s` must stay an ALIAS
/// (not be copy-propagated into a fresh literal, which would break reference semantics), and
/// the static value tracking of both names must be invalidated so a mutation through one is
/// not folded away on the other. Conservative + correct.
#[test]
fn partial_alias_mutation_invalidates_source() {
    let program = "\
## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + i.
Let s be [1, 2, 3].
Let a be s.
Set item 1 of a to d.
Show item 1 of s.";
    // s[1] was mutated to d (=5050) through the alias a; reference semantics.
    assert_run_equals(program, "5050");
}
