//! ════════════════════════════════════════════════════════════════════════════════════════════
//! P2/P3 SELF-APPLICATION RATCHET (Phase 3 progress gate).
//!
//! Genuine `pe(pe, int)` specializes the PE's meta-dispatch away, leaving a compiler; genuine
//! `pe(pe, pe)` leaves a cogen. The FEWER `Inspect` dispatches survive in that residual, the more
//! of the interpreter has been dissolved — the self-application is doing real work.
//!
//! These ceilings capture the CURRENT specialization and may only SHRINK. The deep engine work
//! (mixed-arg memoization + "The Trick") lowers them toward the theoretical minimum; a change that
//! regresses specialization (adds surviving dispatch) turns this red. This strictly STRENGTHENS the
//! old relative `jones_p{2,3}_fewer_inspects_than_pe_source` check (which only asserted `< PE`'s 114)
//! into a monotone absolute target on the road to Jones-optimal P2/P3 output.
//!
//!  ⚠️  LOWER these constants as the engine improves; NEVER raise them.  ⚠️
//! ════════════════════════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::compile::{genuine_projection2_residual, genuine_projection3_residual};

/// Surviving `Inspect` dispatches in the genuine P2 compiler. Measured baseline: 9 (down from the
/// un-specialized PE's 114). May only shrink.
const P2_INSPECT_CEIL: usize = 9;
/// Surviving `Inspect` dispatches in the genuine P3 cogen. Measured baseline: 16. May only shrink.
const P3_INSPECT_CEIL: usize = 16;

/// Count surviving interpreter DISPATCH: an `Inspect <value>` on a runtime value. The keyword is
/// matched at a token boundary so that a Core variant NAME ending in `Inspect` — e.g. `CInspect`,
/// which appears as a `When CInspect` *case-arm label* in the residualized compiler, NOT as a
/// surviving dispatch — is never miscounted. (A naive `matches("Inspect ")` counts the substring
/// inside `CInspect `, inflating the true dispatch total; the ceilings below are true-dispatch
/// counts.) A byte immediately before the keyword that is a letter/digit/`_` means it is the tail
/// of an identifier, not the dispatch keyword.
fn inspect_count(src: &str) -> usize {
    src.match_indices("Inspect ")
        .filter(|(i, _)| {
            *i == 0 || {
                let prev = src.as_bytes()[i - 1];
                !(prev.is_ascii_alphanumeric() || prev == b'_')
            }
        })
        .count()
}

#[test]
fn genuine_p2_specialization_only_shrinks() {
    let p2 = genuine_projection2_residual().expect("genuine P2 must succeed");
    let n = inspect_count(&p2);
    assert!(
        n <= P2_INSPECT_CEIL,
        "P2 compiler dispatch GREW to {n} Inspects (ceil {P2_INSPECT_CEIL}) — a self-application \
         change regressed specialization. Lower the ceiling as the engine improves; never raise it."
    );
}

#[test]
fn genuine_p3_specialization_only_shrinks() {
    let p3 = genuine_projection3_residual().expect("genuine P3 must succeed");
    let n = inspect_count(&p3);
    assert!(
        n <= P3_INSPECT_CEIL,
        "P3 cogen dispatch GREW to {n} Inspects (ceil {P3_INSPECT_CEIL}) — regressed specialization."
    );
}
