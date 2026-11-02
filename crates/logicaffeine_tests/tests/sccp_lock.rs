//! SCCP (sparse conditional constant propagation) lock.
//!
//! SCCP is additive: it prunes a branch whose guard is a PROVEN constant that
//! the iterated `fold → propagate → deadcode` pipeline structurally cannot fold
//! — `propagate` refuses any variable that is ever a `Set` target, and
//! `deadcode` prunes only on a syntactic literal condition. These locks pin:
//!
//!   1. FIRING — SCCP fires on its triggers (AOT + run-path) and gates off under
//!      `## No sccp`; `## Require sccp` compiles iff there is a site.
//!   2. SEMANTICS — on every trigger, tree-walker == VM == (SCCP-off VM) ==
//!      (SCCP-off tree-walker), byte-for-byte, so SCCP is provably
//!      semantics-preserving on both executing tiers.
//!   3. EDGE CASES — meet over branches, the no-poison property, loop-carried
//!      widening (never fold a loop variable to its pre-loop constant), the
//!      refusal to fold an `Index` read, and pruning a branch whose DROPPED body
//!      contains a runtime hazard (never evaluated).

use logicaffeine_compile::compile::{
    interpret_program, optimizations_fired, optimizations_fired_run, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

// ── Triggers (all self-contained, deterministic, no program args) ────────────

/// A mutable set to a constant, then guarded by it: `propagate` excludes
/// `threshold` (a `Set` target), `fold`/`deadcode` see a non-literal guard, so
/// only SCCP proves `threshold == 10` and takes the then arm.
const T1_DEAD_BRANCH: &str = "\
## Main
Let mutable threshold be 0.
Set threshold to 10.
If threshold is greater than 5:
    Show 100.
Otherwise:
    Show 200.
";

/// The meet: `v` is read from an `Index` (SCCP refuses to fold it → ⊥), so the
/// first `If` is NOT pruned; both arms set `x` to 8, so the join proves
/// `x == 8`, which prunes the second `If`. Exercises Index-refusal AND the meet
/// AND a meet-enabled prune in one program.
const T2_MEET_PRUNE: &str = "\
## Main
Let mutable data be a new Seq of Int.
Push 3 to data.
Let v be item 1 of data.
Let mutable x be 0.
If v is less than 5:
    Set x to 8.
Otherwise:
    Set x to 8.
If x is less than 5:
    Show 111.
Otherwise:
    Show 222.
";

/// A loop-carried variable must NEVER be folded to its pre-loop constant. `i`
/// starts at 0 but ends at 5; if SCCP wrongly kept `i == 0` it would prune the
/// guard the wrong way and print 2 instead of 1.
const T3_LOOP_CARRIED: &str = "\
## Main
Let mutable i be 0.
Let mutable total be 0.
While i is less than 5:
    Set total to total + i.
    Set i to i + 1.
If i is greater than 3:
    Show 1.
Otherwise:
    Show 2.
";

/// Pruning a branch whose dropped body holds a runtime hazard (division by zero)
/// must simply DROP it, never evaluate it. The then arm is proven live, so the
/// program prints 777 with SCCP on or off.
const T4_HAZARD_IN_DEAD_ARM: &str = "\
## Main
Let mutable mode be 9.
Set mode to 1.
If mode is less than 3:
    Show 777.
Otherwise:
    Show 100 / (mode - 1).
";

/// Every trigger with the value the program must print on every tier.
const TRIGGERS: &[(&str, &str, &str)] = &[
    ("dead_branch", T1_DEAD_BRANCH, "100"),
    ("meet_prune", T2_MEET_PRUNE, "222"),
    ("loop_carried", T3_LOOP_CARRIED, "1"),
    ("hazard_in_dead_arm", T4_HAZARD_IN_DEAD_ARM, "777"),
];

// Executors differ in whether they emit a trailing newline; that formatting
// detail is orthogonal to SCCP, so normalize it away and compare the content.
fn tw(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("tree-walker parse/run failed: {e:?}"))
        .trim_end()
        .to_string()
}
fn vm(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("VM run failed: {e}"))
        .trim_end()
        .to_string()
}

/// The correctness core: SCCP is semantics-preserving on BOTH executing tiers.
/// For every trigger, the tree-walker (which does not optimize), the VM run-path
/// (which now runs SCCP), and both of those with `## No sccp`, must all produce
/// the identical, expected output — byte for byte.
#[test]
fn sccp_preserves_semantics_across_tiers() {
    for (name, src, expected) in TRIGGERS {
        let off = decorate_source(src, &["sccp"]);
        let tw_on = tw(src);
        let vm_on = vm(src);
        let tw_off = tw(&off);
        let vm_off = vm(&off);
        assert_eq!(tw_on, *expected, "{name}: tree-walker output");
        assert_eq!(vm_on, *expected, "{name}: VM (SCCP on) output");
        assert_eq!(vm_off, *expected, "{name}: VM (SCCP off) output");
        assert_eq!(tw_off, *expected, "{name}: tree-walker (SCCP off) output");
        // The strong differential: SCCP on == SCCP off == tree-walker.
        assert_eq!(
            vm_on, vm_off,
            "{name}: SCCP changed the VM output (unsound!)"
        );
        assert_eq!(vm_on, tw_on, "{name}: VM disagrees with tree-walker");
    }
}

/// SCCP fires on the dead-branch trigger on BOTH the AOT and the run-path
/// pipelines, and gates off under `## No sccp`.
#[test]
fn sccp_fires_and_gates() {
    let aot = optimizations_fired(T1_DEAD_BRANCH);
    assert!(
        aot.contains(&"sccp"),
        "AOT must fire sccp on the dead-branch trigger; got {aot:?}"
    );

    let run = optimizations_fired_run(T1_DEAD_BRANCH);
    assert!(
        run.contains(&"sccp"),
        "run-path must fire sccp; got {run:?}"
    );

    let off = optimizations_fired(&decorate_source(T1_DEAD_BRANCH, &["sccp"]));
    assert!(
        !off.contains(&"sccp"),
        "`## No sccp` must remove it; got {off:?}"
    );
}

/// The meet-enabled prune is genuinely SCCP: it fires on the AOT trace for the
/// program whose only prunable branch depends on a value proven constant by
/// joining two arms.
#[test]
fn sccp_fires_on_meet_enabled_prune() {
    let aot = optimizations_fired(T2_MEET_PRUNE);
    assert!(
        aot.contains(&"sccp"),
        "meet-enabled prune must fire sccp; got {aot:?}"
    );
}

/// A program with no SCCP opportunity must NOT report sccp firing — the pass is
/// a true no-op there (no spurious refold that would perturb the fired set).
#[test]
fn sccp_is_a_noop_without_a_site() {
    let plain = "## Main\nShow 42.\n";
    let fired = optimizations_fired(plain);
    assert!(
        !fired.contains(&"sccp"),
        "sccp must not fire on a program with no branch to prune; got {fired:?}"
    );
    assert_eq!(tw(plain), "42");
    assert_eq!(vm(plain), "42");
}

/// `## Require sccp` compiles when there is a site, and errors when there is
/// not — the honest contract that a demanded optimization actually applied.
#[test]
fn sccp_require_gate() {
    use logicaffeine_compile::compile::compile_to_rust;
    let required = format!("## Require sccp\n{T1_DEAD_BRANCH}");
    assert!(
        compile_to_rust(&required).is_ok(),
        "`## Require sccp` must compile when sccp has a site"
    );
    let no_site = "## Require sccp\n## Main\nShow 42.\n";
    assert!(
        compile_to_rust(no_site).is_err(),
        "`## Require sccp` must error when sccp has no site"
    );
}
