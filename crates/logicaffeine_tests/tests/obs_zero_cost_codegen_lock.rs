//! W13/W7 — `obs_zero_cost_codegen_lock`: zero cost proven by IDENTITY (§0.7).
//!
//! The Observability Constitution's performance doctrine (D0.5, §0.7):
//! "Elision is erasure, not a branch. A statement elided by the active profile
//! does not exist in the compiled artifact — no flag check, no dead argument
//! evaluation, nothing." Zero cost is not measured by a benchmark; it is proven
//! by BYTE-IDENTITY — the compiled artifact of a program carrying observation
//! statements is byte-for-byte the artifact of the same program with those
//! statements deleted.
//!
//! Wave 7 locked this for VM bytecode; Wave 13 gains the **emitted-Rust and
//! WASM rows** (constitution Part 7, Wave 13). Because the compiled tiers elide
//! observation at lowering — before any codegen — the artifacts are identical by
//! construction. This lock nails that shut on both tiers: if anyone ever made
//! observation cost a single instruction in a compiled program, a row goes RED.

use logicaffeine_compile::compile::{compile_to_rust, compile_to_wasm};

/// A program threaded with every observation shape — messages, a payload, and a
/// nested `During` span around real work.
const OBSERVED: &str = "## Main\n\
    \x20   Let mutable total be 0.\n\
    \x20   Note that the run began with total.\n\
    \x20   During accumulating:\n\
    \x20       Repeat for i from 1 to 4:\n\
    \x20           Set total to total + i.\n\
    \x20           Record that a step ran with i and total.\n\
    \x20   Warn that the fold finished with total.\n\
    \x20   Show total.\n";

/// The SAME program with every observation statement and span deleted — only the
/// arithmetic and the `Show` remain.
const STRIPPED: &str = "## Main\n\
    \x20   Let mutable total be 0.\n\
    \x20   Repeat for i from 1 to 4:\n\
    \x20       Set total to total + i.\n\
    \x20   Show total.\n";

/// Guard against a vacuous test: the two sources really are different text.
#[test]
fn the_fixtures_are_genuinely_different_source() {
    assert_ne!(
        OBSERVED, STRIPPED,
        "the point is DIFFERENT source that compiles IDENTICAL"
    );
    assert!(
        OBSERVED.contains("Note that"),
        "the observed fixture carries observation"
    );
    assert!(OBSERVED.contains("During"), "and a span");
    assert!(
        !STRIPPED.contains("Note"),
        "the stripped fixture carries none"
    );
}

/// **Emitted-Rust row (Wave 13).** The AOT backend's Rust for the observed
/// program is byte-identical to its Rust for the statement-deleted program —
/// observation contributes not one character of generated code.
#[test]
fn emitted_rust_is_byte_identical() {
    let observed = compile_to_rust(OBSERVED).expect("observed compiles to Rust");
    let stripped = compile_to_rust(STRIPPED).expect("stripped compiles to Rust");
    assert_eq!(
        observed, stripped,
        "observation must add ZERO bytes to the emitted Rust (elision is erasure, not a branch)"
    );
}

/// **WASM row (Wave 13).** The direct-WASM backend's module for the observed
/// program is byte-identical to the module for the statement-deleted program.
/// WASM is assembled straight from the VM bytecode, so this also witnesses that
/// the bytecode itself carries no observation op (the Wave-7 bytecode row).
#[test]
fn emitted_wasm_is_byte_identical() {
    let observed = compile_to_wasm(OBSERVED).expect("observed compiles to WASM");
    let stripped = compile_to_wasm(STRIPPED).expect("stripped compiles to WASM");
    assert_eq!(
        observed, stripped,
        "observation must add ZERO bytes to the emitted WASM module"
    );
    // And the module is real, non-empty wasm (\0asm magic), not two empty vecs.
    assert!(
        observed.starts_with(b"\0asm"),
        "a genuine wasm module, not a vacuous match"
    );
}

/// Sanity: the two fixtures are semantically the same program — identical output
/// — so the byte-identity above is zero-COST, not zero-MEANING.
#[test]
fn the_programs_are_semantically_identical() {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
    assert_eq!(
        tw_outcome(OBSERVED).output.trim_end(),
        tw_outcome(STRIPPED).output.trim_end()
    );
    assert_eq!(
        vm_outcome(OBSERVED).output.trim_end(),
        vm_outcome(STRIPPED).output.trim_end()
    );
    assert_eq!(
        tw_outcome(OBSERVED).output.trim_end(),
        "10",
        "1+2+3+4 = 10, the real work"
    );
}
