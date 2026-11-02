//! ★ THE IR-PRIMITIVE LOCK ★ — Phase 0.1 of the bytecode-IR-as-source-of-truth refactor.
//!
//! Pins the two new primitives that make the bytecode `CompiledProgram` an addressable source of truth:
//!   - `compile_to_program(src)` — the ONE canonical `source → CompiledProgram` path.
//!   - `program_outcome(&CompiledProgram)` — run an already-compiled program straight-line on the VM.
//!
//! The invariant they must satisfy, by construction: running the compiled IR on the VM is byte-for-byte the
//! same as running the source on the VM — `program_outcome(compile_to_program(src)) == vm_outcome(src)`. This
//! is the seam every later invariant lock stands on (Invariant A `VM(P)==VM(opt P)`, Invariant B
//! `VM(opt P)==backend(opt P)`), so it must be rock-solid first.
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{compile_to_program, program_outcome, vm_outcome};

/// A spread of straight-line programs (scalar / control flow / functions / collections / text / error path) —
/// enough to prove the primitive round-trips the VM across the value kinds.
const CORPUS: &[(&str, &str)] = &[
    ("scalar", "## Main\n    Show 42.\n"),
    ("arith", "## Main\n    Show 2 + 3 * 4.\n"),
    ("loop_sum", "## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is at most 100:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Show acc.\n"),
    ("branch", "## Main\n    Let x be 7.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
    ("function", "## To square (n: Int) -> Int:\n    Return n * n.\n\n## Main\n    Show square(9).\n"),
    ("list", "## Main\n    Let xs be [10, 20, 30].\n    Show item 2 of xs.\n"),
    ("text", "## Main\n    Show \"hello\".\n"),
    ("bigint_overflow", "## Main\n    Show 10000000000 * 10000000000.\n"),
    ("div_by_zero_error", "## Main\n    Let a be 5.\n    Let b be 0.\n    Show a / b.\n"),
];

#[test]
fn compile_to_program_succeeds_on_the_corpus() {
    for (label, src) in CORPUS {
        assert!(
            compile_to_program(src).is_ok(),
            "`{label}`: compile_to_program must produce a CompiledProgram"
        );
    }
}

#[test]
fn program_outcome_of_the_compiled_ir_equals_vm_outcome() {
    for (label, src) in CORPUS {
        let program =
            compile_to_program(src).unwrap_or_else(|e| panic!("`{label}`: compile: {e:?}"));
        let ir = program_outcome(&program);
        let vm = vm_outcome(src);
        assert_eq!(
            (ir.output.trim(), ir.error.is_some()),
            (vm.output.trim(), vm.error.is_some()),
            "`{label}`: program_outcome(compile_to_program(src)) must equal vm_outcome(src) — the IR is the \
             same thing the VM runs"
        );
    }
}
