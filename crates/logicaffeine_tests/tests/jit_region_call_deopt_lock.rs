//! A precise loop region compiled on the register-allocating backend may call another function through its table slot.
//! When the callee leaves native code partway through, its exit reaches the region through the shared status channel
//! and the VM resumes at the faulting op with the region's and the callee's frames as native code left them. These
//! loops make a native callee leave in each way a callee can: an integer that outgrows a machine word, so the run
//! continues on exact integers, and a read past a list's end, so the run fails with the VM's sentence. Each prints the
//! tree-walker's output, the region holding the call ran on the precise contiguous backend, and the callee's exits
//! were taken.

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

/// The callee's sum outgrows a machine word on its 251st call, long after the loop and the callee run natively.
const OVERFLOW_IN_THE_CALLEE: &str = r#"
## To next (xs: Seq of Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push (item 1 of xs) + 1 to out.
    Return out.

## Main
Let mutable state be a new Seq of Int.
Push 9223372036854775557 to state.
Let mutable i be 0.
While i is less than 400:
    Set state to next(state).
    Set i to i + 1.
Show item 1 of state.
"#;

/// The callee reads past its list's end on the 150th call, inside native code.
const FAILURE_IN_THE_CALLEE: &str = r#"
## To next (xs: Seq of Int) (i: Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    If i is 150:
        Push item 5 of xs to out.
    Otherwise:
        Push (item 1 of xs) + 1 to out.
    Return out.

## Main
Let mutable state be a new Seq of Int.
Push 1 to state.
Let mutable i be 0.
While i is less than 200:
    Set state to next(state, i).
    Set i to i + 1.
Show item 1 of state.
"#;

/// Runs `src` on the VM with the forge tier and on the tree-walker; both must agree, and the call's region must have
/// run on the precise contiguous backend and taken a side exit from the callee.
fn agrees_through_a_callee_exit(label: &str, src: &str) -> String {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "[{label}] the JIT's run differs from the tree-walker's"
    );
    assert!(
        tier.regalloc_precise_region_count() >= 1,
        "[{label}] the loop calling `next` never compiled as a precise contiguous region: regions {}, functions {:?}",
        tier.regalloc_region_count(),
        tier.function_counts()
    );
    let (entries, completions, exits) = tier.region_runtime_stats();
    assert!(
        entries > 0 && exits > 0,
        "[{label}] the region never left native code through its call: entries {entries}, completions {completions}, exits {exits}"
    );
    let (_, _, deopts, precise_deopts) = tier.runtime_stats();
    assert!(
        deopts + precise_deopts > 0,
        "[{label}] the callee never left native code: {:?}",
        tier.runtime_stats()
    );
    assert_eq!(logicaffeine_jit::native_alloc_scope_depth(), 0);
    assert_eq!(logicaffeine_jit::native_alloc_registry_len(), 0);
    norm(&vm.output)
}

#[test]
fn an_integer_outgrowing_a_word_inside_the_callee_continues_exact() {
    let output = agrees_through_a_callee_exit("overflow", OVERFLOW_IN_THE_CALLEE);
    assert_eq!(output, "9223372036854775957");
}

#[test]
fn a_failure_inside_the_callee_reads_as_the_vms() {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(FAILURE_IN_THE_CALLEE, &[], Some(&tier as &dyn NativeTier));
    assert!(
        vm.error.is_some(),
        "reading past the list's end inside `next` must fail the run: {:?}",
        vm.output
    );
    agrees_through_a_callee_exit("failure", FAILURE_IN_THE_CALLEE);
}

#[test]
fn a_callee_exit_inside_a_function_preserves_the_callers_frame() {
    let (functions, main) = OVERFLOW_IN_THE_CALLEE.split_once("## Main\n").unwrap();
    let body = main.replace("Show item 1 of state.", "Return item 1 of state.");
    let body = body
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!("{functions}\n## To drive () -> Int:\n{body}\n\n## Main\nShow drive().");
    let output = agrees_through_a_callee_exit("nonzero caller depth", &source);
    assert_eq!(output, "9223372036854775957");
}

#[test]
fn recursive_callee_frames_preserve_borrowed_lists_and_return_values() {
    let source = r#"
## No Inline
## To next (xs: Seq of Int) (level: Int) -> Seq of Int:
    If level is greater than 0:
        Let result be next(xs, level - 1).
        If item 1 of result is less than 0:
            Return xs.
        Return result.
    Let mutable out be a new Seq of Int.
    Push (item 1 of xs) + 1 to out.
    Return out.

## Main
Let mutable state be a new Seq of Int.
Push 9223372036854775557 to state.
Let mutable i be 0.
While i is less than 400:
    Set state to next(state, 2).
    Set i to i + 1.
Show state.
"#;
    // Recursive list calls conservatively retain the function tier: their
    // list-parameter stability has not been proved for region adoption.
    // Require real native recursive execution and its precise exit here;
    // the three region controls above separately require native call regions.
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(source, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(source, &[]);
    assert_eq!(
        (vm.output.lines().collect::<Vec<_>>(), &vm.error),
        (tw.output.lines().collect::<Vec<_>>(), &tw.error)
    );
    assert_eq!(vm.error, None);
    assert_eq!(norm(&vm.output), "[9223372036854775957]");
    assert!(
        tier.function_counts().1 >= 1,
        "recursive function must run natively"
    );
    assert!(
        tier.runtime_stats().3 > 0,
        "recursive function must take a precise native exit"
    );
    assert_eq!(logicaffeine_jit::native_alloc_scope_depth(), 0);
    assert_eq!(logicaffeine_jit::native_alloc_registry_len(), 0);
}
