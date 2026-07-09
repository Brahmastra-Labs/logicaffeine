//! WS-C (array/index cluster) codegen-quality gate. The JIT is per-op DISPATCH
//! bound, so the lever for the array benchmarks (matrix_mult, histogram,
//! knapsack, graph_bfs) is collapsing hot-loop op COUNTS while staying
//! BIT-IDENTICAL to the tree-walker. These differential tests pin the target
//! shapes — a two-buffer integer dot-product (matrix_mult), an indexed RMW
//! histogram, and a dependent-index DP — and the WINNERS that must never
//! regress (counting_sort, two_sum, a pure loop-sum). Every test asserts the
//! tiered VM output equals the tree-walker's and that the hot region tiers.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run the program on the tiered VM and the tree-walker; assert bit-identical
/// output/error and return `(output, region_tier_count)`.
fn tiered(src: &str) -> (String, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on:\n{src}"
    );
    let (_, region_ok) = tier.region_counts();
    (norm(&vm.output), region_ok)
}

/// MATRIX MULTIPLY (the two-buffer integer dot-product). The inner loop reads
/// `a[i*n+k]` and `b[k*n+j]` from two DISTINCT pinned int buffers, multiplies,
/// and accumulates into `c[i*n+j]` — exactly the shape the fused two-load
/// `ArrLoad2` stencil targets. The reduced result must equal the tree-walker's
/// and the inner region must tier.
#[test]
fn matrix_mult_two_buffer_dot_product_tiers_bit_identical() {
    let src = "## Main\n\
               Let n be 24.\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable b be a new Seq of Int.\n\
               Let mutable c be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Let mutable j be 0.\n\
               \x20   While j is less than n:\n\
               \x20       Push (i * n + j) % 100 to a.\n\
               \x20       Push (j * n + i) % 100 to b.\n\
               \x20       Push 0 to c.\n\
               \x20       Set j to j + 1.\n\
               \x20   Set i to i + 1.\n\
               Set i to 0.\n\
               While i is less than n:\n\
               \x20   Let mutable k be 0.\n\
               \x20   While k is less than n:\n\
               \x20       Let mutable j be 0.\n\
               \x20       While j is less than n:\n\
               \x20           Let idx be i * n + j + 1.\n\
               \x20           Set item idx of c to (item idx of c + item (i * n + k + 1) of a * item (k * n + j + 1) of b) % 1000000007.\n\
               \x20           Set j to j + 1.\n\
               \x20       Set k to k + 1.\n\
               \x20   Set i to i + 1.\n\
               Let mutable checksum be 0.\n\
               Set i to 1.\n\
               While i is at most n * n:\n\
               \x20   Set checksum to (checksum + item i of c) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show checksum.\n";
    let (_out, region_ok) = tiered(src);
    assert!(
        region_ok >= 1,
        "the matmul inner dot-product loop must tier as a region (got {region_ok})"
    );
}

/// A pure two-buffer integer dot product summed into a scalar (no array store):
/// `sum += a[i] * b[i]`. This is the minimal `ArrLoad(a); ArrLoad(b); Mul`
/// idiom; its checksum must match the tree-walker bit-for-bit.
#[test]
fn scalar_dot_product_two_buffers_bit_identical() {
    let src = "## Main\n\
               Let n be 4000.\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable b be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push (i % 7) to a.\n\
               \x20   Push (i % 13) to b.\n\
               \x20   Set i to i + 1.\n\
               Let mutable sum be 0.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Set sum to (sum + item i of a * item i of b) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show sum.\n";
    let (_out, region_ok) = tiered(src);
    assert!(region_ok >= 1, "the dot-product loop must tier (got {region_ok})");
}

/// OUT-OF-BOUNDS on a FUSED two-load: the dot-product loop runs one element
/// past buffer `b`, so the fused `ArrLoad2`'s b-index goes OOB. The bounds
/// side-exit must fire BEFORE any effect and the replay must reproduce the
/// tree-walker's exact error and partial output (Show 5 happens first).
#[test]
fn fused_two_load_oob_deopt_replay_parity() {
    let src = "## Main\n\
               Show 5.\n\
               Let n be 300.\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable b be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Push (i % 9) to a.\n\
               \x20   Set i to i + 1.\n\
               Set i to 0.\n\
               While i is less than 10:\n\
               \x20   Push (i % 5) to b.\n\
               \x20   Set i to i + 1.\n\
               Let mutable sum be 0.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Set sum to sum + item i of a * item i of b.\n\
               \x20   Set i to i + 1.\n\
               Show sum.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "fused two-load OOB deopt replay diverged"
    );
    assert!(vm.error.is_some(), "indexing b past its end must error");
    assert_eq!(norm(&vm.output), "5", "the pre-loop Show must survive the deopt");
}

/// HISTOGRAM (indexed read-modify-write). The hot loop's `count[v+1] += 1`
/// already collapses to a fused `ArrRMW`; this differential must stay green
/// (the dot-product fusion must not perturb the RMW idiom).
#[test]
fn histogram_indexed_rmw_bit_identical() {
    let src = "## Main\n\
               Let mutable counts be a new Seq of Int.\n\
               Let mutable idx be 0.\n\
               While idx is less than 1000:\n\
               \x20   Push 0 to counts.\n\
               \x20   Set idx to idx + 1.\n\
               Let mutable seed be 42.\n\
               Let mutable i be 0.\n\
               While i is less than 200000:\n\
               \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
               \x20   Let v be ((seed / 65536) % 32768) % 1000.\n\
               \x20   Set item (v + 1) of counts to (item (v + 1) of counts) + 1.\n\
               \x20   Set i to i + 1.\n\
               Let mutable maxFreq be 0.\n\
               Let mutable distinct be 0.\n\
               Set i to 0.\n\
               While i is less than 1000:\n\
               \x20   If item (i + 1) of counts is greater than 0:\n\
               \x20       Set distinct to distinct + 1.\n\
               \x20   If item (i + 1) of counts is greater than maxFreq:\n\
               \x20       Set maxFreq to item (i + 1) of counts.\n\
               \x20   Set i to i + 1.\n\
               Show \"\" + maxFreq + \" \" + distinct.\n";
    let (_out, region_ok) = tiered(src);
    assert!(region_ok >= 1, "the histogram loop must tier (got {region_ok})");
}

/// DEPENDENT-INDEX DP (the knapsack shape). The reduction reads `dp[w]` and
/// `dp[w - wt]` (a data-dependent index) inside the hot loop. The result must
/// be bit-identical to the tree-walker; the loop must tier.
#[test]
fn knapsack_dependent_index_dp_bit_identical() {
    let src = "## Main\n\
               Let cap be 600.\n\
               Let items be 40.\n\
               Let mutable dp be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is at most cap:\n\
               \x20   Push 0 to dp.\n\
               \x20   Set i to i + 1.\n\
               Let mutable it be 1.\n\
               While it is at most items:\n\
               \x20   Let wt be (it * 7) % 50 + 1.\n\
               \x20   Let val be (it * 13) % 90 + 1.\n\
               \x20   Let mutable w be cap.\n\
               \x20   While w is at least wt:\n\
               \x20       Let cand be item (w - wt + 1) of dp + val.\n\
               \x20       If cand is greater than item (w + 1) of dp:\n\
               \x20           Set item (w + 1) of dp to cand.\n\
               \x20       Set w to w - 1.\n\
               \x20   Set it to it + 1.\n\
               Show item (cap + 1) of dp.\n";
    let (_out, region_ok) = tiered(src);
    assert!(region_ok >= 1, "the knapsack DP loop must tier (got {region_ok})");
}

/// WINNER — COUNTING SORT (dense indexed counts + cumulative scan). Must stay
/// bit-identical; the new fusion must not change its tiering or result.
#[test]
fn winner_counting_sort_unregressed() {
    let src = "## Main\n\
               Let n be 5000.\n\
               Let mutable data be a new Seq of Int.\n\
               Let mutable seed be 7.\n\
               Let mutable i be 0.\n\
               While i is less than n:\n\
               \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
               \x20   Push (seed % 256) to data.\n\
               \x20   Set i to i + 1.\n\
               Let mutable counts be a new Seq of Int.\n\
               Set i to 0.\n\
               While i is less than 256:\n\
               \x20   Push 0 to counts.\n\
               \x20   Set i to i + 1.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Let v be item i of data.\n\
               \x20   Set item (v + 1) of counts to (item (v + 1) of counts) + 1.\n\
               \x20   Set i to i + 1.\n\
               Let mutable checksum be 0.\n\
               Set i to 1.\n\
               While i is at most 256:\n\
               \x20   Set checksum to checksum + item i of counts * i.\n\
               \x20   Set i to i + 1.\n\
               Show checksum.\n";
    let (_out, region_ok) = tiered(src);
    assert!(region_ok >= 1, "counting_sort must keep tiering (got {region_ok})");
}

/// WINNER — TWO-SUM style dense probe (`seen[x]` reads + writes). Must stay
/// bit-identical.
#[test]
fn winner_dense_probe_unregressed() {
    let src = "## Main\n\
               Let n be 4000.\n\
               Let mutable seen be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 1000:\n\
               \x20   Push 0 to seen.\n\
               \x20   Set i to i + 1.\n\
               Let mutable hits be 0.\n\
               Set i to 1.\n\
               While i is at most n:\n\
               \x20   Let x be (i * 37) % 1000.\n\
               \x20   If item (x + 1) of seen is greater than 0:\n\
               \x20       Set hits to hits + 1.\n\
               \x20   Set item (x + 1) of seen to 1.\n\
               \x20   Set i to i + 1.\n\
               Show hits.\n";
    let (_out, region_ok) = tiered(src);
    assert!(region_ok >= 1, "the dense-probe loop must keep tiering (got {region_ok})");
}

/// WINNER — pure scalar loop sum (no arrays): the cleanest tiering case, must
/// be unaffected by the array-fusion work.
#[test]
fn winner_loop_sum_unregressed() {
    let src = "## Main\n\
               Let n be 5000000.\n\
               Let mutable sum be 0.\n\
               Let mutable i be 1.\n\
               While i is at most n:\n\
               \x20   Set sum to sum + i.\n\
               \x20   Set i to i + 1.\n\
               Show sum.\n";
    let (out, region_ok) = tiered(src);
    assert_eq!(out, "12500002500000");
    assert!(region_ok >= 1, "loop_sum must tier (got {region_ok})");
}
