//! ════════════════════════════════════════════════════════════════════════════════════════════
//! SHORTCUT QUARANTINE LOCK (Phase 6) — a Jones/Futamura certificate must come from the GENUINE
//! self-application, never from a shortcut REPRESENTATION of it.
//!
//! Three "projection" surfaces in `compile.rs` are shortcut representations — each is a real,
//! defensible transform, but none DEMONSTRATES the self-application machinery actually running, so
//! none may stand in as a certificate of Jones optimality:
//!   • `projection1_source`  — the Rust-native optimizer+decompiler transform (real, and legitimate
//!                             as a decompiler utility — but it is not interpreter specialization,
//!                             so it cannot certify Jones optimality).
//!   • `projection2_source`  — the renamed-PE representation: `PE(PE, self-interp) = PE` for the
//!                             Core self-interpreter, realized by renaming peExpr→compileExpr /
//!                             peBlock→compileBlock. A TRUE property that yields a working compiler,
//!                             but it ASSERTS the collapse by renaming instead of RUNNING it.
//!   • `projection3_source`  — likewise, the renamed-PE cogen representation.
//!
//! The GENUINE self-applicative path — which actually RUNS `PE(pe_source, pe_mini/pe_bti)` — is
//! `genuine_projection{2,3}_residual` / `*_real` / `*_real_fast`. This lock reads every gate whose
//! verdict CERTIFIES Jones optimality and forbids it from reaching that verdict through any of the
//! three shortcut representations. A certificate must be earned by the running machinery, not by a
//! rename — this test makes routing a Jones gate through the shortcut impossible.
//!
//!  ⚠️  A JONES CERTIFICATE COMES FROM THE GENUINE RUN, NOT A SHORTCUT REPRESENTATION.  ⚠️  Route
//!  gates through `decompile` (`projection1_source_real_fast`) or the genuine `_real` projections.
//!  Strictly monotone: add gate files here as they are created; never remove one.
//! ════════════════════════════════════════════════════════════════════════════════════════════

/// Every lock/gate file whose green verdict is a claim of Jones optimality.
const GATE_FILES: &[(&str, &str)] = &[
    ("phase_pe_jones.rs", include_str!("phase_pe_jones.rs")),
    ("phase_pe_jones_adversarial.rs", include_str!("phase_pe_jones_adversarial.rs")),
    ("jones_whole_language_lock.rs", include_str!("jones_whole_language_lock.rs")),
    ("futamura_tier_lock.rs", include_str!("futamura_tier_lock.rs")),
    ("futamura_ratchet.rs", include_str!("futamura_ratchet.rs")),
];

/// The three shortcut surfaces, matched as a call/`use` token. `projection1_source(` deliberately
/// does NOT match `projection1_source_real_fast(` (the genuine in-process path), because the
/// `_real` suffix sits before the `(`.
const SHORTCUT_CALL_TOKENS: &[&str] =
    &["projection1_source(", "projection2_source", "projection3_source"];

#[test]
fn no_jones_gate_certifies_through_a_shortcut() {
    let mut violations = Vec::new();
    for (name, src) in GATE_FILES {
        for token in SHORTCUT_CALL_TOKENS {
            if src.contains(token) {
                violations.push(format!(
                    "[{name}] reaches its Jones verdict through the shortcut `{}` — route it \
                     through `decompile` (projection1_source_real_fast) or a genuine `_real` \
                     projection instead.",
                    token.trim_end_matches('(')
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "shortcut leaked into a Jones gate ({}):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// Guard against a vacuous lock: the token set actually fires on shortcut usage, so a future gate
/// that adopts a shortcut is genuinely caught.
#[test]
fn quarantine_detector_is_not_vacuous() {
    let shortcut_gate = "let r = projection2_source().unwrap();";
    assert!(
        SHORTCUT_CALL_TOKENS.iter().any(|t| shortcut_gate.contains(t)),
        "the shortcut detector would not catch a gate calling projection2_source"
    );
    let genuine_gate = "let r = projection1_source_real_fast(\"\", \"\", p).unwrap();";
    assert!(
        !genuine_gate.contains("projection1_source("),
        "the detector must NOT flag the genuine projection1_source_real_fast path"
    );
}
