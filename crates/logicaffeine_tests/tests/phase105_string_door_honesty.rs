//! =============================================================================
//! PHASE 105: STRING-DOOR HONESTY — "Proved" IFF kernel-certified
//! =============================================================================
//!
//! The `compile_theorem` String door must NEVER present an uncertified
//! derivation as "Proved". A backward-chaining success that the kernel did not
//! certify is NOT a proof, and the user-facing string must not claim it is.
//!
//! This is the anti-divergence property for the human-readable door, mirroring
//! phase102's `verified`-flag consistency for the structured doors:
//!
//!     compile_theorem(input) contains "Proved"   IFF   verified(input)
//!
//! A hostile reviewer who finds ANY input that yields a derivation the kernel
//! cannot certify must see an honest "not proved" message, not "Proved!".

use logicaffeine_compile::compile_theorem_for_ui;
use logicaffeine_language::compile_theorem;

/// The Barber paradox: the backward chainer finds a derivation, but the kernel
/// does not certify it. This is the canonical "derivation without a proof".
const BARBER: &str = r#"## Theorem: Barber_Paradox
Given: The barber is a man.
Given: The barber shaves all men who do not shave themselves.
Given: The barber does not shave any man who shaves himself.
Prove: The barber does not exist.
Proof: Auto.
"#;

const VALID_SOCRATES: &str = r#"## Theorem: Socrates_Mortality
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

const INVALID_MISSING_PREMISE: &str = r#"## Theorem: Incomplete
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

/// The core honesty invariant: the String door says "Proved" exactly when the
/// structured door reports `verified == true`. No divergence in either
/// direction.
#[test]
fn string_door_says_proved_iff_kernel_verified() {
    for input in [BARBER, VALID_SOCRATES, INVALID_MISSING_PREMISE] {
        let ui = compile_theorem_for_ui(input);
        let verified = ui.verified;
        let has_derivation = ui.derivation.is_some();

        let string = compile_theorem(input);
        let says_proved = string
            .as_ref()
            .map(|s| s.contains("Proved"))
            .unwrap_or(false);

        eprintln!(
            "[honesty] name={:<24} verified={:<5} derivation={:<5} says_proved={}",
            ui.name, verified, has_derivation, says_proved
        );

        assert_eq!(
            says_proved, verified,
            "String door divergence for '{}': says_proved={} but verified={} \
             (derivation present={}). An uncertified derivation must NOT be \
             reported as 'Proved'.",
            ui.name, says_proved, verified, has_derivation
        );
    }
}

/// Specifically pin the dangerous case: a derivation the kernel could not
/// certify must never be advertised as a proof. The Barber paradox is our
/// witness — it yields a derivation but `verified == false`.
#[test]
fn uncertified_barber_derivation_is_not_called_proved() {
    let ui = compile_theorem_for_ui(BARBER);
    assert!(
        ui.derivation.is_some(),
        "test precondition: Barber should yield a derivation"
    );
    assert!(
        !ui.verified,
        "test precondition: Barber derivation should NOT be kernel-certified"
    );

    match compile_theorem(BARBER) {
        Ok(s) => assert!(
            !s.contains("Proved"),
            "String door advertised an UNCERTIFIED Barber derivation as 'Proved':\n{}",
            s
        ),
        Err(_) => { /* honest rejection is acceptable */ }
    }
}
