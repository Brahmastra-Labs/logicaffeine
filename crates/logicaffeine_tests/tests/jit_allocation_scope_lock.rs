//! Recursive native functions reclaim their list allocations at each return,
//! rather than retaining the recursion tree until the outer VM boundary.

#![cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

const MERGESORT: &str = r#"
## To mergeSort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is less than 2:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable right be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Set left to mergeSort(left).
    Set right to mergeSort(right).
    Let mutable result be arr.
    Let mutable li be 1.
    Let mutable ri be 1.
    Let mutable k be 1.
    While li is at most length of left and ri is at most length of right:
        If item li of left is at most item ri of right:
            Set item k of result to item li of left.
            Set li to li + 1.
        Otherwise:
            Set item k of result to item ri of right.
            Set ri to ri + 1.
        Set k to k + 1.
    While li is at most length of left:
        Set item k of result to item li of left.
        Set li to li + 1.
        Set k to k + 1.
    While ri is at most length of right:
        Set item k of result to item ri of right.
        Set ri to ri + 1.
        Set k to k + 1.
    Return result.

## Main
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than 512:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 32768 to arr.
    Set i to i + 1.
Set arr to mergeSort(arr).
Let mutable checksum be 0.
Set i to 1.
While i is at most 512:
    Set checksum to (checksum * 31 + item i of arr) & 268435455.
    Set i to i + 1.
Show checksum.
"#;

fn norm(s: &str) -> String {
    s.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn recursive_native_allocations_are_stack_scoped() {
    let source = MERGESORT.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let before = logicaffeine_jit::native_alloc_scope_counts();
            let tier = ForgeTier::new();
            let vm = vm_outcome_with_args(&source, &[], Some(&tier as &dyn NativeTier));
            let tw = tw_outcome_with_args(&source, &[]);
            assert_eq!((norm(&vm.output), &vm.error), (norm(&tw.output), &tw.error));
            assert_eq!(vm.error, None);
            assert!(
                tier.regalloc_function_count() >= 1,
                "mergeSort must run natively"
            );

            assert!(
                tier.runtime_stats().1 > 0,
                "native functions must complete: {:?}",
                tier.runtime_stats()
            );

            let after = logicaffeine_jit::native_alloc_scope_counts();
            let entries = after.0 - before.0;
            let leaves = after.1 - before.1;
            assert!(
                entries > 20,
                "recursive calls did not enter allocation scopes: {entries}"
            );
            assert_eq!(
                leaves, entries,
                "every successful native frame must release its scope"
            );
            assert_eq!(logicaffeine_jit::native_alloc_scope_depth(), 0);
            assert_eq!(logicaffeine_jit::native_alloc_registry_len(), 0);
        })
        .expect("spawn large-stack JIT test")
        .join()
        .expect("JIT test thread panicked");
}

#[test]
fn native_copy_on_write_without_a_list_constructor_has_a_scope() {
    let source = r#"
## To changed (xs: Seq of Int) (value: Int) -> Int:
    Let changedValues be xs.
    Set item 1 of changedValues to value.
    Return item 1 of changedValues + item 1 of xs.

## Main
Let input be [11, 13, 17].
Let mutable total be 0.
Repeat for i from 1 to 800:
    Set total to total + changed(input, i).
Show total.
Show input.
"#;
    let before = logicaffeine_jit::native_alloc_scope_counts();
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(source, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(source, &[]);
    assert_eq!(
        vm.output.lines().collect::<Vec<_>>(),
        tw.output.lines().collect::<Vec<_>>()
    );
    assert_eq!(vm.error, tw.error);
    assert_eq!(vm.error, None);
    assert!(
        tier.runtime_stats().1 > 0,
        "copy-on-write must complete natively: {:?}",
        tier.runtime_stats()
    );
    let after = logicaffeine_jit::native_alloc_scope_counts();
    assert!(
        after.0 - before.0 > 20,
        "implicit list allocation needs a scope"
    );
    assert_eq!(after.0 - before.0, after.1 - before.1);
    assert_eq!(logicaffeine_jit::native_alloc_registry_len(), 0);
    assert_eq!(logicaffeine_jit::native_alloc_scope_depth(), 0);
}
