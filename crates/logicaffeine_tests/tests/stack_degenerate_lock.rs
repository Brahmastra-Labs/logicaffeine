//! ★ STACK DEGENERATE-EMBED LOCK ★ — wrapping today's 2D analog lane in a 1-tier stack
//! changes NOTHING: the compiled artifact is bit-identical to the direct path.
//!
//! This is the additive-extension covenant, stated as an equation instead of a promise:
//! every existing 2D lock stays green because `StackTarget::single(t)` and the bare `t`
//! produce EQUAL netlists, schedules, certificates, budgets, placements, and SPICE decks.
//! Refusals pass through typed, and a stack without an analog tier refuses by name.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a stack feature ever needs the degenerate
//! embed to diverge from 2D, that is a redesign conversation, not an edit here.

use logicaffeine_compile::codegen_ana::stack::{analog_artifact_on_stack, StackTarget};
use logicaffeine_compile::codegen_ana::{analog_artifact_from_source, AnaLowerError, AnalogTarget};

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

#[test]
fn the_degenerate_embed_is_bit_identical_to_the_direct_2d_path() {
    for target in [AnalogTarget::ideal(), AnalogTarget::sky130_fpaa()] {
        let direct = analog_artifact_from_source(DOT3, "dot3", &target).expect("2D lowers");
        let embedded = analog_artifact_on_stack(DOT3, "dot3", &StackTarget::single(target))
            .expect("the 1-tier stack lowers");
        assert_eq!(
            embedded, direct,
            "single-tier embed ≡ direct, field for field"
        );
    }
}

#[test]
fn the_v1_stack_compiles_through_its_analog_tier() {
    let stack = StackTarget::sky130_stack_v1();
    let via_stack = analog_artifact_on_stack(DOT3, "dot3", &stack).expect("v1 stack lowers");
    let direct =
        analog_artifact_from_source(DOT3, "dot3", &AnalogTarget::sky130_fpaa()).expect("2D");
    assert_eq!(
        via_stack, direct,
        "the stack resolves to its analog tier exactly"
    );
}

#[test]
fn refusals_pass_through_typed_and_missing_tiers_refuse_by_name() {
    // An unbounded input refuses identically through the stack.
    let unbounded =
        "## To f (x: Float) -> Float:\n    Return x * 2.0.\n\n## Main\n    Show f(0.5).\n";
    let direct = analog_artifact_from_source(unbounded, "f", &AnalogTarget::ideal());
    let embedded =
        analog_artifact_on_stack(unbounded, "f", &StackTarget::single(AnalogTarget::ideal()));
    match (direct, embedded) {
        (Err(AnaLowerError::UnboundedInput(a)), Err(AnaLowerError::UnboundedInput(b))) => {
            assert_eq!(a, b, "the refusal text passes through unchanged");
        }
        other => panic!("both paths must refuse UnboundedInput, got {other:?}"),
    }

    // A stack with no analog tier cannot compile an analog region — by name.
    let mut no_analog = StackTarget::sky130_stack_v1();
    no_analog.tiers.remove(1);
    no_analog.gaps.clear();
    match analog_artifact_on_stack(DOT3, "dot3", &no_analog) {
        Err(AnaLowerError::Unsupported(msg)) => assert!(msg.contains("no analog tier")),
        other => panic!("must refuse by name, got {other:?}"),
    }

    // A malformed stack refuses before any compilation happens.
    let mut malformed = StackTarget::sky130_stack_v1();
    malformed.gaps.clear();
    match analog_artifact_on_stack(DOT3, "dot3", &malformed) {
        Err(AnaLowerError::UnloweredOp(msg)) => assert!(msg.contains("GapCountMismatch")),
        other => panic!("shape violations refuse first, got {other:?}"),
    }
}
