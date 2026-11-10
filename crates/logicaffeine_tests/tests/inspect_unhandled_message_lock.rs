//! ★ LOCK — gauntlet finding G5: a non-exhaustive `Inspect` names the value's own variant and type,
//! byte-identical on the tree-walker and the VM. ★
//!
//! The lang3 lane is making a `When` arm naming an unknown variant a COMPILE-TIME refusal (its own
//! RED test, `gauntlet_findings_lock::an_inspect_arm_naming_the_wrong_variant_is_refused_at_compile_
//! time`) — the more valuable fix, since it catches the mistake before either tier ever runs it. This
//! lock holds the RUNTIME half: the message a value still reachable dynamically (a variant the
//! `Otherwise`-free arm list never anticipated, reached through a value the compile-time check cannot
//! see coming — e.g. a value built from untrusted input) produces when no arm matches.
//!
//! `semantics::builtins::inspect_unhandled_message(type_name, variant_name)` is the ONE shared
//! constructor both tiers call.

mod common;

use common::{assert_compiled_equals_interpreted_eq, assert_panics};
use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::builtins::inspect_unhandled_message;

const SRC: &str = "## A Choice is either:\n    A First (v: Int).\n    A Second (v: Int).\n\n\
## Main\nLet c be a new Second with v 1.\nInspect c:\n    When First (v):\n        Show v.\n";

#[test]
fn the_tree_walker_names_the_actual_variant_and_type() {
    let out = tw_outcome(SRC);
    assert_eq!(
        out.error.as_deref(),
        Some(inspect_unhandled_message("Choice", "Second").as_str()),
        "must fail with the shared constructor's own wording, byte-identical: output={:?}",
        out.output
    );
}

/// The VM half: needs NO runtime tag extraction and no new `Op`/`BuiltinId` (LANES.md rule 16),
/// because the compiler already knows every OTHER constructor the matched enum has, statically, from
/// the discovery-pass type registry — `compile_inspect` tests each UNCOVERED one and fails with a
/// COMPILE-TIME-CONSTANT message built by calling this SAME Rust function, so the wording is
/// byte-identical to the tree-walker by construction.
#[test]
fn the_vm_names_the_same_variant_and_type() {
    let out = logicaffeine_compile::compile::vm_outcome(SRC);
    assert_eq!(
        out.error.as_deref(),
        Some(inspect_unhandled_message("Choice", "Second").as_str()),
        "the VM must word this identically to the tree-walker"
    );
}

/// The "no statically known enum" fallback, locked SEPARATELY per the coordinator's own instruction
/// — NOT required to match the tree-walker's wording here. `Compiler::compile`'s plain entry (unlike
/// `vm_outcome`, which threads the discovery pass's type registry through) gets no type registry at
/// all, so `compile_inspect` cannot look up the matched enum's other constructors; it falls back to
/// the EXISTING `BuiltinId::Fail` dynamic-message mechanism on the scrutinee's own display text.
#[test]
fn the_vm_falls_back_to_the_values_display_text_with_no_static_enum_known() {
    use logicaffeine_compile::vm::{Compiler, Vm};
    logicaffeine_compile::ui_bridge::with_parsed_program(SRC, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("the fixture must parse");
        // Deliberately NOT threading `_types` — `Compiler::compile` takes none, simulating the
        // "no static enum knowledge" case even though a registry is available here.
        let program = Compiler::compile(stmts, interner).expect("the fixture must compile");
        let mut vm = Vm::new(&program);
        let result = vm.run();
        let err = result.expect_err("the mismatched arm must still fail");
        assert!(
            err.contains("Second"),
            "the value's own display text must still name its actual constructor: {err}"
        );
    });
}

/// The compiled half: the binary built from the program stops with the same words, so no tier hands its author a
/// compiler's complaint about generated code in place of the language's own refusal.
#[test]
fn the_compiled_binary_names_the_same_variant_and_type() {
    assert_panics(SRC, &inspect_unhandled_message("Choice", "Second"));
}

/// An `Inspect` with no `Otherwise` is a program: it runs, on every tier, whenever an arm covers the value it meets.
#[test]
fn the_compiled_binary_runs_an_inspect_whose_arms_cover_the_values_it_meets() {
    let src = "## A Shape is one of:\n    A Circle with r Int.\n    A Square with s Int.\n    A Dot.\n\n\
               ## Main\nLet mutable total be 0.\nRepeat for sh in [a new Circle with r 2, a new Circle with r 5]:\n    Inspect sh:\n        When Circle (r):\n            Set total to total + r.\nShow total.\n";
    assert_compiled_equals_interpreted_eq(src, "7");
}
