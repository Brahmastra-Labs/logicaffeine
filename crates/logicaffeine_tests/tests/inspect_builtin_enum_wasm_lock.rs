//! ★ LOCK — G5's `compile_inspect` fallthrough must not break `Option`/`Result` on wasm. ★
//!
//! `Option`/`Result` are language built-ins: no `## A ... is either:` declaration ever puts them
//! in the discovery-pass type registry, so `compile_inspect`'s G5 fallthrough (gauntlet_findings_
//! lock's non-exhaustive-`Inspect`-names-its-variant fix) could not find a static enum for either
//! one and fell to the DYNAMIC `Op::CallBuiltin(BuiltinId::Fail, scrutinee)` path — which wasm
//! cannot lower for an arbitrary value ("unsupported concat operand kind cannot be stringified
//! yet"). Every exhaustive `Inspect` over an `Option`/`Result` paid for a fallthrough it could
//! never reach, and wasm refused to compile it at all.
//!
//! The fix seeds `Option`→`{Some, None}` and `Result`→`{Ok, Err}` into the SAME `enum_variants`/
//! `variant_enum` tables a user-declared enum populates, keyed by one of their own constructors'
//! symbols (`Compiler::builtin_enum_display` carries the real display name back) — so an Inspect
//! over either one gets the identical static treatment: an EXHAUSTIVE one emits NO fallthrough at
//! all (nothing left to test), a PARTIAL one gets the same compile-time-constant message every
//! other static enum gets. The dynamic path is now reserved for a genuinely unknown scrutinee.

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::builtins::inspect_unhandled_message;

const EXHAUSTIVE_OPTION: &str =
    "## Main\nLet v be some 0.\nInspect v:\n    When Some (n):\n        \
                                  Show \"some\".\n    When None:\n        Show \"none\".\n";

const EXHAUSTIVE_RESULT: &str = "## Main\nLet v be a new Ok with value 0.\nInspect v:\n    \
                                  When Ok (n):\n        Show \"ok\".\n    When Err (e):\n        \
                                  Show \"err\".\n";

const PARTIAL_OPTION: &str = "## Main\nLet v be none.\nInspect v:\n    When Some (n):\n        \
                               Show \"some\".\n";

/// The coordinator's own repro: an exhaustive `Option` `Inspect` used to refuse on wasm outright.
#[test]
fn an_exhaustive_option_inspect_compiles_and_runs_on_every_tier() {
    let tw = tw_outcome(EXHAUSTIVE_OPTION);
    assert_eq!(
        tw.error, None,
        "tree-walker must run it cleanly: {:?}",
        tw.error
    );
    assert_eq!(tw.output.trim(), "some");

    let vm = vm_outcome(EXHAUSTIVE_OPTION);
    assert_eq!(vm.error, None, "VM must run it cleanly: {:?}", vm.error);
    assert_eq!(vm.output.trim_end_matches('\n'), "some");

    compile_to_wasm(EXHAUSTIVE_OPTION)
        .expect("an exhaustive Option Inspect must compile to wasm — no fallthrough is reachable");
}

/// Same shape, `Result` — the coordinator's fix explicitly covers both built-ins alike.
#[test]
fn an_exhaustive_result_inspect_compiles_and_runs_on_every_tier() {
    let tw = tw_outcome(EXHAUSTIVE_RESULT);
    assert_eq!(
        tw.error, None,
        "tree-walker must run it cleanly: {:?}",
        tw.error
    );
    assert_eq!(tw.output.trim(), "ok");

    let vm = vm_outcome(EXHAUSTIVE_RESULT);
    assert_eq!(vm.error, None, "VM must run it cleanly: {:?}", vm.error);
    assert_eq!(vm.output.trim_end_matches('\n'), "ok");

    compile_to_wasm(EXHAUSTIVE_RESULT)
        .expect("an exhaustive Result Inspect must compile to wasm — no fallthrough is reachable");
}

/// A PARTIAL `Option` `Inspect` (no `Otherwise`, arms don't cover `None`) must report the SAME
/// message on the tree-walker and the VM — the compile-time-constant path a user-declared enum
/// already gets, not the dynamic value-display fallback.
#[test]
fn a_partial_option_inspect_reports_the_same_message_on_every_tier() {
    let tw = tw_outcome(PARTIAL_OPTION);
    let vm = vm_outcome(PARTIAL_OPTION);
    assert_eq!(
        vm.error, tw.error,
        "the VM must word this identically to the tree-walker: tw={:?} vm={:?}",
        tw.error, vm.error
    );
    assert_eq!(
        tw.error.as_deref(),
        Some(inspect_unhandled_message("Option", "None").as_str()),
        "must fail with the shared constructor's own wording: {:?}",
        tw.error
    );
    // A partial Option Inspect is still statically resolvable (only one arm covers it), so it
    // must compile to wasm too — the compile-time-constant fallthrough, not the dynamic one.
    compile_to_wasm(PARTIAL_OPTION)
        .expect("a partial Option Inspect must still compile to wasm (compile-time message)");
}
