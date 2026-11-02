//! The benchmark-visible JIT has the same stage-separated trace contract as
//! Rust, direct C, bytecode C and the VM.  Its source optimizer evidence stays
//! in the shared registry; forge-native lowering evidence keeps forge's own
//! vocabulary, and every refusal names the VM fallback explicitly.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

use logicaffeine_compile::compile::{optimization_trace_run, tw_outcome_with_args};
use logicaffeine_compile::optimization::OptimizationBackend;
use logicaffeine_jit::optimization_trace_jit;

fn norm(s: &str) -> String {
    s.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

const HOT_SCALAR: &str = r#"
## To mix (a: Int, b: Int) -> Int:
    Let mutable x be a.
    Let mutable j be 0.
    While j is less than 8:
        Set x to (x * 31 + b + j) % 1000003.
        Set j to j + 1.
    Return x.

## No inline
## Main
Let mutable h be 7.
Let mutable i be 0.
While i is less than 1000:
    Set h to mix(h, i).
    Set i to i + 1.
Show h.
"#;

#[test]
fn jit_trace_separates_shared_optimizer_from_native_lowering() {
    let traced = optimization_trace_jit(HOT_SCALAR, &[]);
    let tree = tw_outcome_with_args(HOT_SCALAR, &[]);

    assert_eq!(
        (norm(&traced.outcome.output), &traced.outcome.error),
        (norm(&tree.output), &tree.error),
        "tracing changed the JIT program outcome"
    );
    assert_eq!(traced.shared.backend, OptimizationBackend::Jit);
    assert_eq!(
        traced.shared.optimizer,
        optimization_trace_run(HOT_SCALAR).optimizer,
        "JIT silently diverged from the shared run-path optimizer"
    );
    assert!(
        traced.function_successes + traced.region_successes > 0,
        "the hot program never reached native code: {traced:?}"
    );
    assert!(
        !traced.native.fired().is_empty(),
        "forge compiled code but exposed no native-lowering evidence: {traced:?}"
    );
}

const UNSUPPORTED_PARAM: &str = r#"
## To lookupTwice (m: Map of Int to Int, k: Int) -> Int:
    Let mutable total be 0.
    Let mutable j be 0.
    While j is less than 2:
        If m contains k:
            Set total to total + item k of m.
        Set j to j + 1.
    Return total.

## No inline
## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 9.
Let mutable total be 0.
Let mutable i be 0.
While i is less than 150:
    Set total to total + lookupTwice(m, 1).
    Set i to i + 1.
Show total.
"#;

#[test]
fn jit_ineligible_function_reports_the_vm_fallback() {
    let traced = optimization_trace_jit(UNSUPPORTED_PARAM, &[]);
    let tree = tw_outcome_with_args(UNSUPPORTED_PARAM, &[]);

    assert_eq!(
        (norm(&traced.outcome.output), &traced.outcome.error),
        (norm(&tree.output), &tree.error),
        "fallback changed the program outcome"
    );
    assert!(
        traced.shared.fallbacks.iter().any(|fallback| {
            fallback.from == OptimizationBackend::Jit
                && fallback.to == OptimizationBackend::Vm
                && fallback.reason.contains("forge representation")
        }),
        "the JIT-to-VM fallback was silent: {:?}",
        traced.shared.fallbacks
    );
}
