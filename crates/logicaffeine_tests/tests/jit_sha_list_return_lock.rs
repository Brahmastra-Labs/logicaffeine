//! SHA-256's hot loop builds two local Word32 lists, calls a list-owning
//! compressor, and rebinds its state from a fresh list return. All of that must
//! stay native without retaining one schedule/output allocation per iteration.

#![cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn sha_source(iterations: usize) -> String {
    include_str!("../../../benchmarks/programs/sha256/main.lg").replace(
        "Let arguments be args().\nLet n be parseInt(item 2 of arguments).",
        &format!("Let n be {iterations}."),
    )
}

#[test]
fn sha_list_return_loop_and_scalar_helpers_stay_native() {
    let source = sha_source(200);
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let before = logicaffeine_jit::native_alloc_scope_counts();
            let resets_before = logicaffeine_jit::native_region_alloc_reset_count();
            let tier = ForgeTier::new();
            let vm = vm_outcome_with_args(&source, &[], Some(&tier as &dyn NativeTier));
            let tw = tw_outcome_with_args(&source, &[]);

            assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
            assert_eq!(vm.error, None);
            assert!(
                tier.regalloc_function_count() >= 7,
                "every SHA helper, including sha256Compress, should compile: {:?}",
                tier.function_counts()
            );
            assert!(
                tier.regalloc_region_count() >= 1,
                "the list-push + fresh-list-return outer loop never compiled"
            );

            let after = logicaffeine_jit::native_alloc_scope_counts();
            let entries = after.0 - before.0;
            let leaves = after.1 - before.1;
            let resets = logicaffeine_jit::native_region_alloc_reset_count() - resets_before;
            assert!(
                entries > 50,
                "SHA compression never entered scoped native allocation: {entries}"
            );
            assert_eq!(
                leaves, entries,
                "every successful native frame must leave its allocation scope"
            );
            assert!(
                resets > 50,
                "region-local SHA buffers were not reclaimed per iteration: {resets}"
            );
            assert_eq!(logicaffeine_jit::native_alloc_scope_depth(), 0);
            assert_eq!(logicaffeine_jit::native_alloc_registry_len(), 0);
        })
        .expect("spawn large-stack SHA JIT test")
        .join()
        .expect("SHA JIT test thread panicked");
}

const ALIAS_LOCK: &str = r#"
## To next (xs: Seq of Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push (item 1 of xs) + 1 to out.
    Return out.

## Main
Let mutable state be a new Seq of Int.
Push 1 to state.
Let alias be state.
Let mutable i be 0.
While i is less than 200:
    Set state to next(state).
    Set i to i + 1.
Show item 1 of alias.
Show item 1 of state.
"#;

#[test]
fn adopted_list_return_preserves_value_semantics_for_aliases() {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(ALIAS_LOCK, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(ALIAS_LOCK, &[]);
    assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
    assert_eq!(norm(&vm.output), "1\n201");
    assert!(
        tier.regalloc_region_count() >= 1,
        "fresh list return stayed on bytecode"
    );
    assert!(
        tier.region_runtime_stats().1 > 0,
        "the adopted-return region compiled but never completed: {:?}",
        tier.region_runtime_stats()
    );
    assert_eq!(logicaffeine_jit::native_alloc_scope_depth(), 0);
    assert_eq!(logicaffeine_jit::native_alloc_registry_len(), 0);
}
