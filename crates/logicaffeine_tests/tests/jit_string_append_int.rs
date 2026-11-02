//! Ratchet for mutable `Text += Int` in hot JIT regions. The tier must lower
//! integer concatenation to the allocation-free decimal append helper, report
//! that lowering by name, and remain bit-identical to the tree-walker.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::{ForgeTier, JitOptimization};

fn norm(s: &str) -> String {
    s.lines()
        .map(|line| line.trim_end())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tiered(src: &str) -> (String, ForgeTier) {
    let tier = ForgeTier::new();
    let jit = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tree = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&jit.output), &jit.error),
        (norm(&tree.output), &tree.error),
        "integer text append diverged from the tree-walker:\n{src}"
    );
    assert_eq!(jit.error, None);
    (norm(&jit.output), tier)
}

#[test]
fn strings_benchmark_build_loop_tiers_and_traces_integer_append() {
    let src = "## Main\n\
               Let mutable result be \"\" with capacity 24000.\n\
               Let mutable i be 0.\n\
               While i is less than 4000:\n\
               \x20   Set result to result + i + \" \".\n\
               \x20   Set i to i + 1.\n\
               Show length of result.\n";
    let (out, tier) = tiered(src);
    assert_eq!(out, "18890");
    assert_eq!(
        tier.region_counts(),
        (1, 1),
        "the build loop must compile natively"
    );
    assert!(
        tier.optimization_trace()
            .count(JitOptimization::StringAppendInt)
            > 0,
        "the JIT trace must name the integer string-append lowering"
    );
    assert!(
        tier.optimization_trace()
            .keywords()
            .contains(&"stringappendint"),
        "the stable trace keyword must be exposed"
    );
}

#[test]
fn integer_append_handles_i64_min_without_formatter_or_overflow() {
    let src = "## Main\n\
               Let edge be -9223372036854775808.\n\
               Let mutable result be \"\".\n\
               Let mutable pass be 0.\n\
               While pass is less than 200:\n\
               \x20   Set result to result + edge + \",\".\n\
               \x20   Set pass to pass + 1.\n\
               Show length of result.\n";
    let (out, tier) = tiered(src);
    assert_eq!(out, "4200");
    assert_eq!(tier.region_counts(), (1, 1));
    assert!(
        tier.optimization_trace()
            .count(JitOptimization::StringAppendInt)
            > 0
    );
}

#[test]
fn integer_append_preserves_copy_on_write_aliases() {
    let src = "## Main\n\
               Let mutable result be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 1000:\n\
               \x20   Set result to result + i.\n\
               \x20   Set i to i + 1.\n\
               Let saved be result.\n\
               While i is less than 2000:\n\
               \x20   Set result to result + i.\n\
               \x20   Set i to i + 1.\n\
               Show length of saved.\n\
               Show length of result.\n";
    let (out, tier) = tiered(src);
    assert_eq!(out, "2890\n6890");
    assert_eq!(
        tier.region_counts(),
        (2, 2),
        "both build loops must compile natively"
    );
    assert!(
        tier.optimization_trace()
            .count(JitOptimization::StringAppendInt)
            >= 2
    );
}
