//! WS-C array-fusion gate: affine-index array loads (`arr[a*i + b]`,
//! `arr[w - wi + 1]`, `arr[i*n + j + 1]`) and computed-index conditional
//! swaps (`arr[2*root+1]`) must run native, fuse the index arithmetic into
//! the load/swap stencil, and stay BIT-IDENTICAL to the tree-walker — including
//! the out-of-bounds side-exit, which must deopt BEFORE any effect and replay on
//! bytecode where the kernel raises the exact index error.

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

/// Run on the tiered VM (with the JIT installed) and assert bit-identical to the
/// tree-walker. Returns (output, error, regions_that_tiered).
fn tiered(src: &str) -> (String, Option<String>, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on:\n{src}"
    );
    let (_, region_ok) = tier.region_counts();
    (norm(&vm.output), vm.error, region_ok)
}

/// THE MATMUL DOT-PRODUCT SHAPE — a 2D `c[i*n+j] = (c[i*n+j] + a[i*n+k]*b[k*n+j])
/// % M` inner loop. The read-only operand loads carry affine `i*n+k+1` /
/// `k*n+j+1` indices; the affine fusion must fold those index Adds into the
/// fused load. Bit-identical and the inner loop must tier as a region.
#[test]
fn matmul_affine_index_tiers_and_matches() {
    let src = "## Main\n\
        Let n be 40.\n\
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
    let (out, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "158944000", "matmul checksum (n=40)");
    assert!(region_ok >= 1, "the matmul inner loop must tier (got {region_ok})");
}

/// THE KNAPSACK DP SHAPE — `prev[w+1]` and `prev[w - wi + 1]` are read-only
/// affine-indexed loads in the hot DP row. The affine fusion must fold the
/// `w - wi + 1` chain (Sub then +1) into the load. Bit-identical, tiers.
#[test]
fn knapsack_affine_index_tiers_and_matches() {
    let src = "## Main\n\
        Let n be 200.\n\
        Let capacity be n * 5.\n\
        Let mutable weights be a new Seq of Int.\n\
        Let mutable vals be a new Seq of Int.\n\
        Let mutable i be 0.\n\
        While i is less than n:\n\
        \x20   Push (i * 17 + 3) % 50 + 1 to weights.\n\
        \x20   Push (i * 31 + 7) % 100 + 1 to vals.\n\
        \x20   Set i to i + 1.\n\
        Let cols be capacity + 1.\n\
        Let mutable prev be a new Seq of Int.\n\
        Set i to 0.\n\
        While i is less than cols:\n\
        \x20   Push 0 to prev.\n\
        \x20   Set i to i + 1.\n\
        Set i to 0.\n\
        While i is less than n:\n\
        \x20   Let mutable curr be a new Seq of Int.\n\
        \x20   Let wi be item (i + 1) of weights.\n\
        \x20   Let vi be item (i + 1) of vals.\n\
        \x20   Let mutable w be 0.\n\
        \x20   While w is at most capacity:\n\
        \x20       Let mutable best be item (w + 1) of prev.\n\
        \x20       If w is at least wi:\n\
        \x20           Let take be item (w - wi + 1) of prev + vi.\n\
        \x20           If take is greater than best:\n\
        \x20               Set best to take.\n\
        \x20       Push best to curr.\n\
        \x20       Set w to w + 1.\n\
        \x20   Set prev to curr.\n\
        \x20   Set i to i + 1.\n\
        Show item (capacity + 1) of prev.\n";
    let (out, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "4928", "knapsack value (n=200)");
    assert!(region_ok >= 1, "the knapsack DP loop must tier (got {region_ok})");
}

/// THE HEAP-SIFT SHAPE — a region carrying `2*root+1` / `2*root+2` indexed reads
/// and an adjacent conditional swap on a computed index. The `2*root+k` index
/// arithmetic must not block either the affine load fusion or the cond-swap
/// fusion. Bit-identical, tiers.
#[test]
fn heap_sift_computed_index_tiers_and_matches() {
    // A self-contained sift loop over a flat array, the heap_sort kernel shape
    // inlined into Main so it region-tiers on its own back-edge.
    let src = "## Main\n\
        Let n be 4000.\n\
        Let mutable arr be a new Seq of Int.\n\
        Let mutable seed be 42.\n\
        Let mutable i be 0.\n\
        While i is less than n:\n\
        \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
        \x20   Push (seed / 65536) % 32768 to arr.\n\
        \x20   Set i to i + 1.\n\
        Let mutable root be 0.\n\
        Let end be n - 1.\n\
        Let mutable guard be 0.\n\
        While 2 * root + 1 is at most end:\n\
        \x20   Let child be 2 * root + 1.\n\
        \x20   Let mutable swapIdx be root.\n\
        \x20   If item (swapIdx + 1) of arr is less than item (child + 1) of arr:\n\
        \x20       Set swapIdx to child.\n\
        \x20   If child + 1 is at most end:\n\
        \x20       If item (swapIdx + 1) of arr is less than item (child + 2) of arr:\n\
        \x20           Set swapIdx to child + 1.\n\
        \x20   If swapIdx equals root:\n\
        \x20       Set root to end.\n\
        \x20   Otherwise:\n\
        \x20       Let tmp be item (root + 1) of arr.\n\
        \x20       Set item (root + 1) of arr to item (swapIdx + 1) of arr.\n\
        \x20       Set item (swapIdx + 1) of arr to tmp.\n\
        \x20       Set root to swapIdx.\n\
        \x20   Set guard to guard + 1.\n\
        Show \"\" + item 1 of arr + \" \" + guard.\n";
    let (out, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    // Correctness is anchored by the tree-walker equality in `tiered`; the exact
    // value is whatever both engines compute, but it must be non-empty.
    assert!(!out.is_empty(), "sift loop produced output");
    assert!(region_ok >= 1, "the sift loop must tier (got {region_ok})");
}

/// THE BUBBLE-SWAP SHAPE — an adjacent compare-and-swap on a flat array where
/// the swap indices are a base plus an offset (`j+1`, `j+2`), so the cond-swap
/// fusion must tolerate the index `Add`s sitting between the two loads.
/// Bit-identical, tiers.
#[test]
fn bubble_swap_offset_index_tiers_and_matches() {
    let src = "## Main\n\
        Let n be 600.\n\
        Let mutable arr be a new Seq of Int.\n\
        Let mutable seed be 7.\n\
        Let mutable i be 0.\n\
        While i is less than n:\n\
        \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
        \x20   Push seed % 100000 to arr.\n\
        \x20   Set i to i + 1.\n\
        Let mutable outer be 0.\n\
        While outer is less than n:\n\
        \x20   Let mutable j be 0.\n\
        \x20   While j is less than n - 1:\n\
        \x20       If item (j + 1) of arr is greater than item (j + 2) of arr:\n\
        \x20           Let tmp be item (j + 1) of arr.\n\
        \x20           Set item (j + 1) of arr to item (j + 2) of arr.\n\
        \x20           Set item (j + 2) of arr to tmp.\n\
        \x20       Set j to j + 1.\n\
        \x20   Set outer to outer + 1.\n\
        Let mutable sorted be 1.\n\
        Set i to 1.\n\
        While i is less than n:\n\
        \x20   If item i of arr is greater than item (i + 1) of arr:\n\
        \x20       Set sorted to 0.\n\
        \x20   Set i to i + 1.\n\
        Show sorted.\n";
    let (out, err, region_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "1", "bubble sort must fully sort the array");
    assert!(region_ok >= 1, "the bubble inner loop must tier (got {region_ok})");
}

/// OOB-INDEX DEOPT PARITY — an affine-indexed load whose computed index walks
/// off the end mid-loop must side-exit the fused stencil BEFORE any effect and
/// replay on bytecode, where the kernel raises the exact out-of-bounds error —
/// bit-identical (same error string) to the tree-walker.
#[test]
fn affine_index_oob_deopts_identically() {
    // Build a long array, then a hot loop that reads `arr[i*2 + 1]` — once `i`
    // grows past half the length the affine index exceeds the bounds and BOTH
    // engines must raise the same index error at the same logical point.
    let src = "## Main\n\
        Let n be 5000.\n\
        Let mutable arr be a new Seq of Int.\n\
        Let mutable i be 0.\n\
        While i is less than n:\n\
        \x20   Push i to arr.\n\
        \x20   Set i to i + 1.\n\
        Let mutable acc be 0.\n\
        Let mutable k be 0.\n\
        While k is less than n:\n\
        \x20   Set acc to acc + item (k * 2 + 1) of arr.\n\
        \x20   Set k to k + 1.\n\
        Show acc.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "affine OOB must deopt and raise the identical index error as the tree-walker"
    );
    assert!(vm.error.is_some(), "the affine index must walk off the end and error");
}

/// KILL-SWITCH PARITY — with the affine fusion disabled, every shape still runs
/// (the un-fused index arithmetic + plain loads) bit-identically. Proves the
/// fusion is a pure optimization, not a correctness crutch, and gives a clean
/// bisection point.
#[test]
fn affine_fusion_killswitch_preserves_results() {
    let src = "## Main\n\
        Let n be 60.\n\
        Let mutable a be a new Seq of Int.\n\
        Let mutable i be 0.\n\
        While i is less than n:\n\
        \x20   Push (i * 7 + 1) % 97 to a.\n\
        \x20   Set i to i + 1.\n\
        Let mutable acc be 0.\n\
        Let mutable pass be 0.\n\
        While pass is less than 50:\n\
        \x20   Let mutable j be 0.\n\
        \x20   While j is less than n - 1:\n\
        \x20       Set acc to (acc + item (j + 1) of a + item (j + 2) of a) % 1000000007.\n\
        \x20       Set j to j + 1.\n\
        \x20   Set pass to pass + 1.\n\
        Show acc.\n";
    let baseline = {
        std::env::set_var("LOGOS_AFFINE", "0");
        let tier = ForgeTier::new();
        let out = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
        std::env::remove_var("LOGOS_AFFINE");
        out
    };
    let fused = {
        let tier = ForgeTier::new();
        vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier))
    };
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(norm(&baseline.output), norm(&tw.output), "kill-switch path matches tree-walker");
    assert_eq!(norm(&fused.output), norm(&tw.output), "fused path matches tree-walker");
    assert_eq!(norm(&baseline.output), norm(&fused.output), "fusion changes no observable result");
}
