//! M10c RED gate: LIST parameters cross the function-JIT boundary, and
//! function deopt becomes PRECISE-STATE (region-grade) instead of
//! replay-from-call.
//!
//! Why precise: a list-mutating body (quicksort's partition) lands writes
//! through the shared Rc BEFORE any potential side exit — replaying the
//! whole call on bytecode would re-run those writes from a now-mutated
//! array and diverge from the kernel. The sound contract is the one
//! regions already honor: every effect up to the exit point stands, the
//! VM materializes the native call chain as real frames and resumes
//! interpreting AT the failing op, so the kernel raises the exact error
//! from the exact state.

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

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Run tiered, assert full outcome parity with the tree-walker, return
/// (output, error, fn_ok).
fn tiered(src: &str) -> (String, Option<String>, u32) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "tiered VM diverged from tree-walker on:\n{src}"
        );
        let (_, fn_ok) = tier.function_counts();
        (norm(&vm.output), vm.error, fn_ok)
    })
}

/// The quicksort kernel: a `Seq of Int` parameter mutated IN PLACE through
/// recursion, passed to self-calls verbatim (through a Let alias), and
/// returned. Must tier and produce the exact checksum.
#[test]
fn quicksort_with_list_param_tiers() {
    let src = "## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:\n\
               \x20   If lo is at least hi:\n\
               \x20       Return arr.\n\
               \x20   Let pivot be item hi of arr.\n\
               \x20   Let mutable result be arr.\n\
               \x20   Let mutable i be lo.\n\
               \x20   Let mutable j be lo.\n\
               \x20   While j is less than hi:\n\
               \x20       If item j of result is at most pivot:\n\
               \x20           Let tmp be item i of result.\n\
               \x20           Set item i of result to item j of result.\n\
               \x20           Set item j of result to tmp.\n\
               \x20           Set i to i + 1.\n\
               \x20       Set j to j + 1.\n\
               \x20   Let tmp be item i of result.\n\
               \x20   Set item i of result to item hi of result.\n\
               \x20   Set item hi of result to tmp.\n\
               \x20   Set result to qs(result, lo, i - 1).\n\
               \x20   Set result to qs(result, i + 1, hi).\n\
               \x20   Return result.\n\
               \n\
               ## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable seed be 42.\n\
               Let mutable i be 0.\n\
               While i is less than 600:\n\
               \x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
               \x20   Push (seed / 65536) % 32768 to arr.\n\
               \x20   Set i to i + 1.\n\
               Set arr to qs(arr, 1, 600).\n\
               Let mutable checksum be 0.\n\
               Set i to 1.\n\
               While i is at most 600:\n\
               \x20   Set checksum to (checksum + item i of arr) % 1000000007.\n\
               \x20   Set i to i + 1.\n\
               Show checksum.\n";
    let (_, err, fn_ok) = tiered(src);
    assert_eq!(err, None);
    assert!(fn_ok >= 1, "quicksort must JIT with its list parameter (got {fn_ok})");
}

/// A read-only `Seq of Float` parameter (the nbody energy shape): bit-exact
/// dyadic sums, function must tier.
#[test]
fn float_list_reader_fn_tiers() {
    let src = "## To energy (vs: Seq of Float, n: Int) -> Float:\n\
               \x20   Let mutable e be 0.0.\n\
               \x20   Let mutable i be 1.\n\
               \x20   While i is at most n:\n\
               \x20       Set e to e + item i of vs * item i of vs.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return e.\n\
               \n\
               ## Main\n\
               Let mutable vs be a new Seq of Float.\n\
               Let mutable i be 0.\n\
               While i is less than 64:\n\
               \x20   Push i * 0.25 to vs.\n\
               \x20   Set i to i + 1.\n\
               Let mutable acc be 0.0.\n\
               Set i to 0.\n\
               While i is less than 200:\n\
               \x20   Set acc to acc + energy(vs, 64).\n\
               \x20   Set i to i + 1.\n\
               Show \"{acc:.1}\".\n";
    let (out, err, fn_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "1066800.0", "200 × Σ (i/4)² for i in 0..63");
    assert!(fn_ok >= 1, "the float-list reader must JIT (got {fn_ok})");
}

/// Mutations through the list parameter are visible to EVERY alias after
/// the call (the Rc is shared, not copied at the native boundary).
#[test]
fn list_param_mutation_visible_through_aliases() {
    let src = "## To stamp (xs: Seq of Int, n: Int) -> Int:\n\
               \x20   Let mutable i be 1.\n\
               \x20   While i is at most n:\n\
               \x20       Set item i of xs to i * 7.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return n.\n\
               \n\
               ## Main\n\
               Let mutable xs be [0, 0, 0, 0, 0].\n\
               Let ys be xs.\n\
               Let mutable i be 0.\n\
               While i is less than 200:\n\
               \x20   Let r be stamp(xs, 5).\n\
               \x20   Set i to i + r - 5.\n\
               \x20   Set i to i + 1.\n\
               Show item 3 of xs.\n\
               Show item 5 of ys.\n";
    let (out, err, fn_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "21\n35");
    assert!(fn_ok >= 1, "stamp must JIT (got {fn_ok})");
}

/// Returning the list parameter preserves IDENTITY: the caller's result is
/// the same Rc, so mutating the returned binding mutates the original.
#[test]
fn list_return_preserves_identity() {
    let src = "## To pass (xs: Seq of Int) -> Seq of Int:\n\
               \x20   Set item 1 of xs to item 1 of xs + 1.\n\
               \x20   Return xs.\n\
               \n\
               ## Main\n\
               Let mutable xs be [100, 200].\n\
               Let mutable i be 0.\n\
               Let mutable r be xs.\n\
               While i is less than 300:\n\
               \x20   Set r to pass(xs).\n\
               \x20   Set i to i + 1.\n\
               Set item 2 of r to 999.\n\
               Show item 1 of xs.\n\
               Show item 2 of xs.\n";
    let (out, err, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "400\n999", "r and xs must be the same list");
}

/// THE precise-state gate: a recursive function lands writes through its
/// list parameter at EVERY level, then indexes out of bounds at the
/// bottom — long after tier-up. Every write up to the failing op must
/// stand, the error must be the kernel's, and the partial output must be
/// bit-identical to the tree-walker. Replay-style deopt cannot pass this
/// (it would re-run 301 increments from an already-incremented array).
#[test]
fn oob_after_writes_inside_native_recursion_is_precise() {
    // The recursion is NON-tail: a direct `Return wreck(...)` is a self-tail-call
    // that TCO loops in constant stack, so `wreck` would never recurse natively
    // and the precise-deopt-through-native-recursion this test exists to check
    // would not run. Binding the call (`Let r be wreck(...)`) then returning a
    // non-identity expression (`r + bad`) keeps it out of tail position AND out
    // of the `Let x; Return x` pair, so real native frames stack and tier.
    // `+ bad` is value-preserving on the success path (bad = 0 there) and only
    // the deep base case (bad = 1) errors, before any unwind runs.
    let src = "## To wreck (xs: Seq of Int, n: Int, bad: Int) -> Int:\n\
               \x20   Set item 1 of xs to item 1 of xs + 1.\n\
               \x20   If n is at most 0:\n\
               \x20       If bad equals 1:\n\
               \x20           Return item 99 of xs.\n\
               \x20       Return item 1 of xs.\n\
               \x20   Let r be wreck(xs, n - 1, bad).\n\
               \x20   Return r + bad.\n\
               \n\
               ## Main\n\
               Let mutable xs be [0, 7].\n\
               Let mutable i be 0.\n\
               While i is less than 80:\n\
               \x20   Show wreck(xs, 5, 0).\n\
               \x20   Set i to i + 1.\n\
               Show wreck(xs, 300, 1).\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "precise-state deopt diverged from the kernel"
        );
        assert!(vm.error.is_some(), "item 99 of a 2-list must error");
        let (_, fn_ok) = tier.function_counts();
        assert!(fn_ok >= 1, "wreck must have tiered before the bad call (got {fn_ok})");
    });
}

/// Depth-limit parity WITH list-param writes: the cap is crossed mid-native-
/// recursion after thousands of landed writes; the error and the array
/// state behind the partial output must match the kernel exactly.
#[test]
fn depth_limit_with_list_writes_is_precise() {
    // NON-tail recursion (`sink(...) - 1`) so real frames stack and depth 5000
    // crosses the call-depth cap: a direct `Return sink(...)` is a self-tail-call
    // that TCO loops in constant stack, which would never hit the cap. The `- 1`
    // runs only after each call returns, so it never executes before the deep
    // deopt — the landed writes this test checks are unaffected.
    let src = "## To sink (xs: Seq of Int, n: Int) -> Int:\n\
               \x20   Set item 1 of xs to item 1 of xs + 1.\n\
               \x20   If n equals 0:\n\
               \x20       Return item 1 of xs.\n\
               \x20   Return sink(xs, n - 1) - 1.\n\
               \n\
               ## Main\n\
               Let mutable xs be [0].\n\
               Let mutable i be 0.\n\
               While i is less than 80:\n\
               \x20   Show sink(xs, 4).\n\
               \x20   Set i to i + 1.\n\
               Show sink(xs, 5000).\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "depth-limit precise deopt diverged"
        );
        assert!(vm.error.is_some(), "depth 5000 must exceed the cap");
    });
}

/// Self-recursion that alternates between TWO distinct lists: pass-through
/// identity does not hold, so the tier may decline — but the OUTCOME must
/// stay exact either way.
#[test]
fn alternating_list_identities_stay_correct() {
    let src = "## To pingpong (a: Seq of Int, b: Seq of Int, n: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return item 1 of a + item 1 of b.\n\
               \x20   Set item 1 of a to item 1 of a + n.\n\
               \x20   Return pingpong(b, a, n - 1).\n\
               \n\
               ## Main\n\
               Let mutable xs be [10].\n\
               Let mutable ys be [20].\n\
               Let mutable i be 0.\n\
               While i is less than 150:\n\
               \x20   Let r be pingpong(xs, ys, 6).\n\
               \x20   Set i to i + 1.\n\
               Show item 1 of xs.\n\
               Show item 1 of ys.\n\
               Show pingpong(xs, ys, 3).\n";
    let (_, err, _) = tiered(src);
    assert_eq!(err, None);
}

/// Pushing to the list parameter (growth across the boundary): exactness
/// is the contract whether or not the tier accepts it yet.
#[test]
fn push_through_list_param_stays_correct() {
    let src = "## To grow (xs: Seq of Int, n: Int) -> Int:\n\
               \x20   Let mutable i be 0.\n\
               \x20   While i is less than n:\n\
               \x20       Push i to xs.\n\
               \x20       Set i to i + 1.\n\
               \x20   Return length of xs.\n\
               \n\
               ## Main\n\
               Let mutable xs be a new Seq of Int.\n\
               Let mutable total be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 150:\n\
               \x20   Set total to total + grow(xs, 3).\n\
               \x20   Set i to i + 1.\n\
               Show total.\n\
               Show length of xs.\n";
    let (out, err, _) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "33975\n450", "Σ 3k for k in 1..150, then 450 elements");
}
