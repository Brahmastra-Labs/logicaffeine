//! Native rotate must stay a one-instruction, trace-visible JIT lowering.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_jit::optimization_trace_jit;

const HOT_ROTATES: &str = r#"
## Main
Let mutable a be word32(2147483649).
Let mutable b be word64(0 - 3).
Let mutable i be 0.
While i is less than 500:
    Set a to rotr(rotl(a, i), i + 32).
    Set b to rotl(rotr(b, i + 64), i).
    Set i to i + 1.
Show intOfWord32(a).
Show intOfWord64(b).
"#;

#[test]
fn word_rotates_are_native_traceable_and_exact() {
    let traced = optimization_trace_jit(HOT_ROTATES, &[]);
    let tree = tw_outcome_with_args(HOT_ROTATES, &[]);

    assert_eq!(traced.outcome.error, tree.error);
    assert_eq!(traced.outcome.output.trim(), tree.output.trim());
    assert!(
        traced.native.keywords().contains(&"rotate_instruction"),
        "native code compiled without exposing the rotate lowering: {traced:?}"
    );
    assert!(
        traced.function_successes + traced.region_successes > 0,
        "the rotate program never reached native code: {traced:?}"
    );
}
