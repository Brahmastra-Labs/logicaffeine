//! W13 — `obs_aot_emission_lock`: the AOT-to-Rust backend EMITS observation.
//!
//! Wave 13 joins the AOT tier to the observation channel. In the OBSERVED
//! profile (`compile_to_rust_observed`) the emitted Rust calls
//! `logicaffeine_system::obs::{record, span_enter, span_exit}` for
//! `Note`/`Warn`/`Record`/`During` — the same events the tree-walker and VM
//! emit, now baked into the compiled binary. In the default ELIDED (release)
//! profile the emitted Rust carries NONE of them and is byte-identical to the
//! observation-free program (the §0.7 zero-cost guarantee). Two profiles, one
//! program: the tier emits when asked and costs nothing when not.

use logicaffeine_compile::compile::{compile_to_rust, compile_to_rust_observed};

const OBSERVED: &str = "## Main\n\
    \x20   Let mutable total be 0.\n\
    \x20   Note that the run began with total.\n\
    \x20   During accumulating:\n\
    \x20       Repeat for i from 1 to 4:\n\
    \x20           Set total to total + i.\n\
    \x20           Record that a step ran with i and total.\n\
    \x20   Warn that the fold finished with total.\n\
    \x20   Show total.\n";

const STRIPPED: &str = "## Main\n\
    \x20   Let mutable total be 0.\n\
    \x20   Repeat for i from 1 to 4:\n\
    \x20       Set total to total + i.\n\
    \x20   Show total.\n";

/// OBSERVED profile: the emitted Rust emits every observation, through the
/// `logicaffeine_system::obs` channel — messages AND balanced span boundaries.
#[test]
fn observed_profile_emits_obs_calls() {
    let rust = compile_to_rust_observed(OBSERVED).expect("compiles");
    // The three speech-act categories, each a real emitted call.
    assert!(
        rust.contains("logicaffeine_system::obs::record(\"note\""),
        "Note emits:\n{rust}"
    );
    assert!(
        rust.contains("logicaffeine_system::obs::record(\"warn\""),
        "Warn emits"
    );
    assert!(
        rust.contains("logicaffeine_system::obs::record(\"record\""),
        "Record emits"
    );
    // The `During` span brackets its body.
    assert!(
        rust.contains("logicaffeine_system::obs::span_enter("),
        "During opens a span"
    );
    assert!(
        rust.contains("logicaffeine_system::obs::span_exit("),
        "and closes it"
    );
    // A payload value is captured (the message + the live variable render).
    assert!(
        rust.contains("format!(\"{}\", total)"),
        "payloads carry the live value:\n{rust}"
    );
    // Balanced: as many span exits as enters.
    let enters = rust.matches("obs::span_enter(").count();
    let exits = rust.matches("obs::span_exit(").count();
    assert_eq!(enters, exits, "spans are balanced in the emitted Rust");
    assert!(enters >= 1);
}

/// The real program work is still emitted alongside the observation (emission is
/// additive, not a replacement) — the loop and the final `show` survive.
#[test]
fn observed_profile_keeps_the_program() {
    let rust = compile_to_rust_observed(OBSERVED).expect("compiles");
    assert!(rust.contains("for i in"), "the loop is still compiled");
    assert!(rust.contains("show(&total)"), "and the program's output");
}

/// ELIDED (default) profile: the emitted Rust carries NO observation calls and
/// is byte-identical to the statement-deleted program — zero cost by erasure
/// (§0.7), exactly as `obs_zero_cost_codegen_lock` requires.
#[test]
fn elided_profile_emits_nothing_and_is_byte_identical() {
    let elided = compile_to_rust(OBSERVED).expect("compiles");
    let stripped = compile_to_rust(STRIPPED).expect("compiles");
    assert!(
        !elided.contains("obs::record"),
        "the release profile emits no observation"
    );
    assert!(!elided.contains("obs::span_enter"), "and no spans");
    assert_eq!(
        elided, stripped,
        "byte-identical to the observation-free program (§0.7)"
    );
}

/// The two profiles are the SAME program (semantically): the observed emission
/// rides on top of identical computation — the emitted `show` line is the same.
#[test]
fn both_profiles_compile_the_same_computation() {
    let observed = compile_to_rust_observed(OBSERVED).expect("compiles");
    let elided = compile_to_rust(OBSERVED).expect("compiles");
    assert!(observed.contains("show(&total)"));
    assert!(elided.contains("show(&total)"));
    // The observed one is strictly larger (it added the obs calls), never smaller.
    assert!(
        observed.len() > elided.len(),
        "observation is additive over the elided baseline"
    );
}
