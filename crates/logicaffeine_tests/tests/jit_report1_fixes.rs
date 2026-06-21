//! Regression pins for Bug Report #1 — JIT (forge / jit) tier.
//!
//! These drive the bytecode VM with a PRIVATE [`ForgeTier`] so the JIT
//! compile/tier-up paths are exercised, and assert the tiered VM agrees with
//! the independent tree-walker oracle.

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

/// BUG-001 (Critical): a Main-loop REGION compiled by `adapt_region` runs under
/// a discard-and-replay-from-head deopt contract that is only sound for
/// replay-idempotent effects. `ListPush` APPENDS — it is not idempotent — yet
/// the region path admits it with no guard. A Map-of-Int-to-Float read inside
/// the loop side-exits (the int fast lane misses on a Float value), forcing a
/// deopt AFTER the push already landed in the real pinned list; the VM then
/// replays the iteration on bytecode and pushes again, duplicating elements.
#[test]
fn region_listpush_is_not_double_applied_on_deopt() {
    let src = "## Main\n\
               Let mutable m be a new Map of Int to Float.\n\
               Let mutable k be 0.\n\
               While k is less than 500:\n\
               \x20   Set item k of m to 1.5.\n\
               \x20   Set k to k + 1.\n\
               Let mutable results be a new Seq of Int.\n\
               Let mutable junk be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 500:\n\
               \x20   Push i to results.\n\
               \x20   Set junk to item i of m.\n\
               \x20   Set i to i + 1.\n\
               Show length of results.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker: a region ListPush was double-applied on deopt"
    );
    assert_eq!(norm(&vm.output), "500", "exactly 500 elements must be pushed (no replay duplicates)");
}

/// BUG-001 follow-up — the deopt path now ROLLS BACK every pinned buffer to its
/// entry length before discard-and-replay, so a `ListPush` may legally coexist
/// with a PURE side-exit. This drives a push loop whose pinned int-list is built
/// alongside a map read that MISSES the int fast lane every iteration (the map
/// holds Floats): the region TIERS, enters native, pushes, then side-exits — and
/// the rollback truncates the pushed buffer back to its entry length so the
/// bytecode replay re-pushes cleanly. A Float accumulator keeps the write-back
/// kinds clean so the region is admissible. Without the truncate the replay
/// would double-apply the native pushes and `results` would exceed 500; the
/// assertions below catch exactly that, AND require the region to have tiered
/// (so the rollback path — not the old blanket guard-reject — is what holds).
#[test]
fn region_listpush_with_pure_deopt_tiers_and_rolls_back() {
    let src = "## Main\n\
               Let mutable m be a new Map of Int to Float.\n\
               Let mutable k be 0.\n\
               While k is less than 500:\n\
               \x20   Set item k of m to 1.5.\n\
               \x20   Set k to k + 1.\n\
               Let mutable results be a new Seq of Int.\n\
               Let mutable total be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 500:\n\
               \x20   Push i to results.\n\
               \x20   Set total to total + item i of m.\n\
               \x20   Set i to i + 1.\n\
               Show length of results.\n\
               Show total.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on a push + pure-deopt region (rollback bug)"
    );
    assert_eq!(
        norm(&vm.output),
        "500\n750",
        "exactly 500 elements + total 750 — the deopt rollback must re-push cleanly, no duplicates"
    );
    let (_, region_successes) = tier.region_counts();
    assert!(
        region_successes >= 1,
        "the push + pure-deopt region must TIER (narrowed guard); got {region_successes} \
         region compiles — the fix regressed to blanket guard-reject, so the rollback path is untested"
    );
}

/// XMM-mem-form regression — a mode-B FUNCTION (list param) whose body has a
/// NESTED loop with a float `item j of v` load in the inner loop and an array
/// store in the outer loop. With XMM float pinning this SIGSEGV'd (the float
/// ArrLoad variant in a function context corrupted a pointer); spectral_norm
/// crashed on exactly this shape. The fix must keep it bit-exact with the
/// tree-walker (and not crash) under the full tier.
#[test]
fn fn_nested_float_arrload_and_store_no_segv() {
    let src = "## To fill (n: Int) (out: Seq of Float) -> Seq of Float:\n\
               \x20   Let mutable result be out.\n\
               \x20   Let mutable i be 0.\n\
               \x20   While i is less than n:\n\
               \x20       Let mutable sum be 0.0.\n\
               \x20       Let mutable j be 0.\n\
               \x20       While j is less than n:\n\
               \x20           Set sum to sum + item (j + 1) of result.\n\
               \x20           Set j to j + 1.\n\
               \x20       Set item (i + 1) of result to sum.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return result.\n\
               \n\
               ## Main\n\
               Let mutable out be a new Seq of Float.\n\
               Let mutable i be 0.\n\
               While i is less than 20:\n\
               \x20   Push 1.5 to out.\n\
               \x20   Set i to i + 1.\n\
               Set i to 0.\n\
               While i is less than 5000:\n\
               \x20   Set out to fill(20, out).\n\
               \x20   Set i to i + 1.\n\
               Show item 1 of out.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on a nested float ArrLoad+store function (XMM bug)"
    );
}

/// Lever A — deopt-time FULL-CONTENT array rollback. A region with `ListPush`
/// PLUS in-place `SetIndex` is sound only if, on a mid-region side-exit, the
/// VM restores the mutated array to its entry contents before replaying (not
/// just truncating the pushed buffer). This drives the graph_bfs shape: each
/// iteration `if marks[i+1]==0 { marks[i+1]=1; push i }`, then a Float map-read
/// that MISSES the int fast lane → a non-fatal deopt AFTER the mark+push landed
/// natively. Without rollback, the persisted `marks[i+1]=1` makes the bytecode
/// replay see a non-zero mark → skip the push → `results` ends SHORT of 500.
/// With full-content rollback, `marks` is restored, replay re-marks + re-pushes,
/// and the count is exact. The Float accumulator keeps write-back kinds clean so
/// the region is admissible; the assertions require it to have TIERED.
#[test]
fn region_listpush_plus_setindex_rolls_back_on_deopt() {
    let src = "## Main\n\
               Let mutable m be a new Map of Int to Float.\n\
               Let mutable k be 0.\n\
               While k is less than 500:\n\
               \x20   Set item k of m to 1.5.\n\
               \x20   Set k to k + 1.\n\
               Let mutable marks be a new Seq of Int.\n\
               Set k to 0.\n\
               While k is less than 500:\n\
               \x20   Push 0 to marks.\n\
               \x20   Set k to k + 1.\n\
               Let mutable results be a new Seq of Int.\n\
               Let mutable total be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 500:\n\
               \x20   If item (i + 1) of marks equals 0:\n\
               \x20       Set item (i + 1) of marks to 1.\n\
               \x20       Push i to results.\n\
               \x20   Set total to total + item i of m.\n\
               \x20   Set i to i + 1.\n\
               Show length of results.\n";
    use std::sync::atomic::Ordering;
    // The TARGET push+SetIndex loop is the only region in this program that
    // side-exits (its Float map-read misses the int fast lane every iteration);
    // the other loops either bail (map SetIndex) or complete (push-only init).
    // So a non-zero REGION_DEOPTS delta proves the TARGET region tiered AND
    // entered native — i.e. the rollback path actually ran (nextest isolates
    // each test in its own process, so this global counter is local to us).
    let deopts_before = logicaffeine_jit::REGION_DEOPTS.load(Ordering::SeqCst);
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    let deopts_after = logicaffeine_jit::REGION_DEOPTS.load(Ordering::SeqCst);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on a push + SetIndex region (array rollback bug)"
    );
    assert_eq!(
        norm(&vm.output),
        "500",
        "every index marked exactly once → 500 pushes; a missing SetIndex rollback would skip some"
    );
    assert!(
        deopts_after > deopts_before,
        "the push + SetIndex region must TIER and side-exit (Lever A guard relaxation); \
         REGION_DEOPTS did not advance — the region is still guard-rejected, so the \
         array-rollback path is untested"
    );
}

/// BUG-012 (High): the register-threading (pinned) compiler cannot lower a
/// fused float compare-and-branch (`BranchF`). `compile_region` guards against
/// this, but `compile_function` does not — a mode-A function containing a float
/// `>`/`>=` comparison plus a hot integer slot gets pinned and panics
/// (`emit_mem_form` unreachable!) at tier-up, aborting a valid program.
#[test]
fn function_with_float_branch_tiers_up_without_panicking() {
    // `countgt` is mode A (float params, no list params): it has hot integer
    // slots (count, i) AND a float compare-branch (`if a > b`). Calling it 600
    // times tiers it up through `compile_function`.
    let src = "## To countgt (a: Float) (b: Float) -> Int:\n\
               \x20   Let mutable count be 0.\n\
               \x20   Let mutable i be 0.\n\
               \x20   While i is less than 20:\n\
               \x20       If a is greater than b:\n\
               \x20           Set count to count + 1.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return count.\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable j be 0.\n\
               While j is less than 600:\n\
               \x20   Set total to total + countgt(3.5, 2.5).\n\
               \x20   Set j to j + 1.\n\
               Show total.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from / panicked vs the tree-walker on a float-branch function"
    );
    assert_eq!(norm(&vm.output), "12000", "countgt(3.5,2.5)=20, called 600 times => 12000");
}

/// ARRAY READ-MODIFY-WRITE FUSION (dispatch-reduction lever): the histogram
/// idiom `Set item idx of counts to (item idx of counts) + 1` lowers to
/// ArrLoad + Add + ArrStore over the SAME cell; `fuse_array_rmw` collapses it to
/// one ArrRMW stencil. This drives the real bytecode→micro→fuse→native pipeline
/// and asserts the tiered VM stays bit-identical to the tree-walker — the
/// soundness net for the new stencil family. A NAMED index slot guarantees the
/// load and store share an index even before CSE.
#[test]
fn array_rmw_named_index_matches_tree_walker() {
    let src = "## Main\n\
               Let mutable counts be a new Seq of Int.\n\
               Let mutable j be 0.\n\
               While j is less than 16:\n\
               \x20   Push 0 to counts.\n\
               \x20   Set j to j + 1.\n\
               Let mutable i be 0.\n\
               While i is less than 4000:\n\
               \x20   Let bucket be (i % 16) + 1.\n\
               \x20   Set item bucket of counts to (item bucket of counts) + 1.\n\
               \x20   Set i to i + 1.\n\
               Let mutable total be 0.\n\
               Let mutable k be 1.\n\
               While k is less than 17:\n\
               \x20   Set total to total + item k of counts.\n\
               \x20   Set k to k + 1.\n\
               Show total.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on an array read-modify-write region (ArrRMW bug)"
    );
    assert_eq!(norm(&vm.output), "4000", "every iteration increments one bucket → total 4000");
    let (_, region_successes) = tier.region_counts();
    assert!(region_successes >= 1, "the RMW loop must TIER; got {region_successes}");
}

/// THE REAL HISTOGRAM SHAPE: the index `(v + 1)` is recomputed for both the load
/// and the store (separate slots in the bytecode). The pipeline must value-number
/// the two `v + 1` to one slot (`copy_propagate`) so `fuse_array_rmw` can collapse
/// the idiom — and the result must stay bit-identical to the tree-walker. Mixes
/// in a bitwise-OR RMW (`marks[x] = marks[x] | 1`) to exercise a second stencil.
#[test]
fn array_rmw_recomputed_index_and_bitor_match_tree_walker() {
    let src = "## Main\n\
               Let mutable counts be a new Seq of Int.\n\
               Let mutable marks be a new Seq of Int.\n\
               Let mutable j be 0.\n\
               While j is less than 32:\n\
               \x20   Push 0 to counts.\n\
               \x20   Push 0 to marks.\n\
               \x20   Set j to j + 1.\n\
               Let mutable seed be 7.\n\
               Let mutable i be 0.\n\
               While i is less than 5000:\n\
               \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
               \x20   Let v be (seed / 65536) % 32.\n\
               \x20   Set item (v + 1) of counts to (item (v + 1) of counts) + 1.\n\
               \x20   Set item (v + 1) of marks to (item (v + 1) of marks) | 1.\n\
               \x20   Set i to i + 1.\n\
               Let mutable distinct be 0.\n\
               Let mutable total be 0.\n\
               Let mutable k be 1.\n\
               While k is less than 33:\n\
               \x20   Set total to total + item k of counts.\n\
               \x20   Set distinct to distinct + item k of marks.\n\
               \x20   Set k to k + 1.\n\
               Show \"\" + total + \" \" + distinct.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on the recomputed-index + bit-or RMW histogram"
    );
}

/// FLOAT array RMW (the nbody velocity/position-update shape): `Set item i of v
/// to (item i of v) + step` over a Seq of Float lowers to ArrLoad + AddF +
/// ArrStore on an f64 buffer; the float ArrRMW stencil must reproduce the exact
/// IEEE result of the tree-walker (copy-and-patch never reassociates). Also
/// exercises a `* factor` (MulF) accumulation to cover a second float stencil.
#[test]
fn float_array_rmw_matches_tree_walker() {
    let src = "## Main\n\
               Let mutable v be a new Seq of Float.\n\
               Let mutable acc be a new Seq of Float.\n\
               Let mutable j be 0.\n\
               While j is less than 8:\n\
               \x20   Push 0.0 to v.\n\
               \x20   Push 1.0 to acc.\n\
               \x20   Set j to j + 1.\n\
               Let mutable i be 0.\n\
               While i is less than 4000:\n\
               \x20   Let slot be (i % 8) + 1.\n\
               \x20   Set item slot of v to (item slot of v) + 0.5.\n\
               \x20   Set item slot of acc to (item slot of acc) * 1.0000001.\n\
               \x20   Set i to i + 1.\n\
               Let mutable total be 0.0.\n\
               Let mutable k be 1.\n\
               While k is less than 9:\n\
               \x20   Set total to total + item k of v.\n\
               \x20   Set total to total + item k of acc.\n\
               \x20   Set k to k + 1.\n\
               Show total.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on a FLOAT array read-modify-write (float ArrRMW bug)"
    );
}

/// FLOAT MULTIPLY-ADD FUSION (the nbody force-chain shape): a hot loop computing
/// `d2 = dx*dx + dy*dy + dz*dz` is a stream of MulF feeding AddF — `fuse_fma`
/// merges each product into its consuming add (FmaF). The fused op rounds the
/// product and the add separately, so the accumulated f64 result must match the
/// tree-walker BIT-for-BIT (a single-rounding hardware FMA would diverge here).
#[test]
fn float_multiply_add_chain_matches_tree_walker() {
    let src = "## Main\n\
               Let mutable acc be 0.0.\n\
               Let mutable t be 1.0.\n\
               Let mutable i be 0.\n\
               While i is less than 4000:\n\
               \x20   Set t to t + 0.000001.\n\
               \x20   Let dx be t * 1.5.\n\
               \x20   Let dy be t * 2.5.\n\
               \x20   Let dz be t * 0.5.\n\
               \x20   Let d2 be dx * dx + dy * dy + dz * dz.\n\
               \x20   Set acc to acc + d2.\n\
               \x20   Set i to i + 1.\n\
               Show acc.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on a float multiply-add chain (FmaF rounding bug)"
    );
}
