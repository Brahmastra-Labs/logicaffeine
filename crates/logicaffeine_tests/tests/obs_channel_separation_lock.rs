//! W6 — `obs_channel_separation_lock`: the Two-Channel Contract (D0.1).
//!
//! Program output (`Show`) and the observation channel (`Note`/`Warn`/`Record`/
//! `During`) are SEPARATE. Adding, removing, or reordering observation
//! statements must NEVER change what the program prints — on any tier. This is
//! what lets observation be compile-time elidable without any tier ever
//! disagreeing about what the program did. This lock proves it by construction:
//! a program with observation statements produces byte-identical `Show` output
//! to the same program with them stripped, on BOTH the tree-walker and the VM.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// The same program, once with a spread of observation statements and once
/// without — their `Show` output must be identical.
const WITH_OBS: &str = "## Main\n\
    \x20   Let x be 10.\n\
    \x20   Note that the computation started with x.\n\
    \x20   During doubling the value:\n\
    \x20       Let y be x * 2.\n\
    \x20       Warn that a doubling happened with y.\n\
    \x20   Record that the run finished with y.\n\
    \x20   Show y.\n";

const WITHOUT_OBS: &str = "## Main\n\
    \x20   Let x be 10.\n\
    \x20   Let y be x * 2.\n\
    \x20   Show y.\n";

#[test]
fn observation_never_changes_stdout_on_the_tree_walker() {
    let a = tw_outcome(WITH_OBS);
    let b = tw_outcome(WITHOUT_OBS);
    assert_eq!(
        a.error, None,
        "the observed program runs cleanly: {:?}",
        a.error
    );
    assert_eq!(
        a.output, b.output,
        "observation statements must not touch the program-output channel"
    );
    assert_eq!(a.output, "20", "and the actual work still prints");
}

#[test]
fn observation_never_changes_stdout_on_the_vm() {
    let a = vm_outcome(WITH_OBS);
    let b = vm_outcome(WITHOUT_OBS);
    assert_eq!(
        a.error, None,
        "the observed program runs cleanly on the VM: {:?}",
        a.error
    );
    assert_eq!(
        a.output, b.output,
        "the two-channel contract holds on the VM tier too"
    );
}

/// The span body's real work runs (`During` is scope-transparent, not a no-op).
#[test]
fn during_body_actually_executes() {
    let with_span = "## Main\n\
        \x20   Let mutable total be 0.\n\
        \x20   During accumulating:\n\
        \x20       Set total to total + 5.\n\
        \x20       Set total to total + 5.\n\
        \x20   Show total.\n";
    // Transparency: the span's body runs exactly as if the wrapper were gone —
    // proven per-tier against the un-wrapped equivalent (so any tw/vm output
    // formatting difference cancels).
    let flat = "## Main\n\
        \x20   Let mutable total be 0.\n\
        \x20   Set total to total + 5.\n\
        \x20   Set total to total + 5.\n\
        \x20   Show total.\n";
    assert_eq!(
        tw_outcome(with_span).output,
        tw_outcome(flat).output,
        "tree-walker: span is transparent"
    );
    assert_eq!(
        vm_outcome(with_span).output,
        vm_outcome(flat).output,
        "VM: span is transparent"
    );
    assert!(
        tw_outcome(with_span).output.starts_with("10"),
        "the body did the work"
    );
}
