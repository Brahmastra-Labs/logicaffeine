//! Context-driven coreference for MODIFIED definite descriptions.
//!
//! Two definite descriptions corefer — denote the SAME discourse referent —
//! when (a) their distinguishing MODIFIER matches AND (b) their head nouns are
//! of a COMPATIBLE occasion sort. The modifier does the referring; the head
//! noun is a soft type ("the hunting vacation" / "the hunting trip" / "the
//! hunting holiday" all pick out the one hunting occasion).
//!
//! This is NOT synonymy. No `Vacation ↔ Trip` axiom is asserted; the FOL keeps
//! `Vacation(x)` and `Trip(x)` literally. Coreference is a referent-resolution
//! step: the later description reuses the earlier referent's VARIABLE.
//!
//! GUARDRAILS pinned here:
//! 1. MUST corefer — shared modifier + compatible occasion head nouns share a
//!    variable across the discourse.
//! 2. MUST NOT corefer — a shared modifier on NON-occasion heads of different
//!    sorts ("the red box" / "the red ball") never coreferes.
//! 3. MUST NOT corefer — differing modifiers ("the hunting trip" / "the
//!    skydiving trip") stay distinct even with identical heads.

use logicaffeine_language::compile;

fn fol(s: &str) -> String {
    compile(s).unwrap_or_else(|e| panic!("expected OK for {s:?}, got {e:?}"))
}

/// Extract the single bound variable introduced by the first clause's leading
/// existential, e.g. `∃x(...)` → "x". Used to prove the later clause reuses it.
fn leading_existential_var(fol: &str) -> String {
    let after = fol
        .split_once('∃')
        .unwrap_or_else(|| panic!("no existential in {fol:?}"))
        .1;
    after
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}

/// Split the numbered multi-clause discourse output into its clauses.
/// `compile` renders a discourse as "1) …\n2) …".
fn clauses(fol: &str) -> Vec<String> {
    if !fol.contains("1)") {
        return vec![fol.to_string()];
    }
    fol.lines()
        .map(|l| {
            // strip a leading "N) " marker
            match l.find(") ") {
                Some(i) if l[..i].chars().all(|c| c.is_ascii_digit()) => l[i + 2..].to_string(),
                _ => l.to_string(),
            }
        })
        .collect()
}

// ── GUARDRAIL 1: MUST COREFER ────────────────────────────────────────────────

#[test]
fn hunting_vacation_and_hunting_trip_corefer() {
    // "The hunting vacation arrived. The hunting trip was fun."
    // `Hunt` matches; Trip and Vacation are sort-compatible occasions, so the
    // second description reuses the first's variable.
    let out = fol("The hunting vacation arrived. The hunting trip was fun.");
    let cs = clauses(&out);
    assert_eq!(cs.len(), 2, "expected two clauses, got: {out}");

    // Clause 1 keeps the faithful FOL: Vacation(x) ∧ Hunt(x), bound by ∃x.
    assert!(
        cs[0].contains("Vacation(") && cs[0].contains("Hunt("),
        "clause 1 must keep faithful Vacation+Hunt: {}",
        cs[0]
    );
    let var = leading_existential_var(&cs[0]);

    // Clause 2 must reuse that SAME variable — it does not re-introduce a fresh
    // existential, it predicates Fun over the shared referent.
    assert!(
        !cs[1].contains('∃'),
        "clause 2 must not open a fresh existential (would mean no coreference): {}",
        cs[1]
    );
    assert_eq!(
        cs[1].trim(),
        format!("Fun({var})"),
        "clause 2 must be Fun({var}) — reusing the hunting-vacation referent; full output:\n{out}"
    );
}

#[test]
fn hunting_holiday_also_corefers_to_hunting_vacation() {
    // A third occasion head — holiday — joins the same referent.
    let out = fol("The hunting vacation arrived. The hunting holiday was fun.");
    let cs = clauses(&out);
    assert_eq!(cs.len(), 2, "expected two clauses, got: {out}");
    let var = leading_existential_var(&cs[0]);
    assert_eq!(
        cs[1].trim(),
        format!("Fun({var})"),
        "the hunting holiday must reuse the hunting-vacation referent; full output:\n{out}"
    );
}

// ── GUARDRAIL 2: MUST NOT COREFER (non-occasion heads, different sorts) ───────

#[test]
fn red_box_and_red_ball_do_not_corefer() {
    // `Red` matches, but box and ball are concrete physical objects (NOT
    // occasions) of distinct identity — the modifier does NOT do the referring.
    // The second description must introduce its OWN referent.
    let out = fol("The red box arrived. The red ball was fun.");
    let cs = clauses(&out);
    assert_eq!(cs.len(), 2, "expected two clauses, got: {out}");

    let box_var = leading_existential_var(&cs[0]);

    // Clause 2 must NOT collapse to `Fun(box_var)`. It must assert its own ball
    // entity (its own existential / its own variable).
    assert!(
        cs[1].contains('∃'),
        "the red ball must open its OWN existential — never reuse the box referent: {}",
        cs[1]
    );
    let ball_var = leading_existential_var(&cs[1]);
    assert_ne!(
        box_var, ball_var,
        "red box and red ball must NOT share a variable; full output:\n{out}"
    );
    assert!(
        cs[1].contains("Ball("),
        "the red ball clause must keep Ball(...): {}",
        cs[1]
    );
    // And it must categorically NOT be the bare reuse form.
    assert_ne!(
        cs[1].trim(),
        format!("Fun({box_var})"),
        "red ball wrongly coreferred to red box; full output:\n{out}"
    );
}

// ── GUARDRAIL 3: MUST NOT COREFER (differing modifier) ───────────────────────

#[test]
fn hunting_trip_and_skydiving_trip_do_not_corefer() {
    // Same head (trip) but the distinguishing modifier differs (Hunt vs
    // Skydive) — these are different occasions and must stay distinct.
    let out = fol("The hunting trip arrived. The skydiving trip was fun.");
    let cs = clauses(&out);
    assert_eq!(cs.len(), 2, "expected two clauses, got: {out}");

    let hunting_var = leading_existential_var(&cs[0]);

    assert!(
        cs[1].contains('∃'),
        "the skydiving trip must open its OWN existential — never reuse the hunting trip: {}",
        cs[1]
    );
    let skydiving_var = leading_existential_var(&cs[1]);
    assert_ne!(
        hunting_var, skydiving_var,
        "differing modifiers (Hunt vs Skydive) must NOT corefer; full output:\n{out}"
    );
    assert!(
        cs[1].contains("Skydive("),
        "the skydiving trip clause must keep Skydive(...): {}",
        cs[1]
    );
    assert_ne!(
        cs[1].trim(),
        format!("Fun({hunting_var})"),
        "skydiving trip wrongly coreferred to hunting trip; full output:\n{out}"
    );
}

// ── Faithfulness: single-sentence output is unchanged (no synonymy axiom) ─────

#[test]
fn single_sentence_modified_definite_is_faithful() {
    // One sentence in isolation: full Russell expansion, no coreference, and
    // crucially NO `Vacation ↔ Trip` synonymy axiom injected.
    let out = fol("The hunting vacation was fun.");
    assert_eq!(
        out,
        "∃x((((Vacation(x) ∧ Hunt(x)) ∧ ∀y(((Vacation(y) ∧ Hunt(y)) → y = x))) ∧ Fun(x)))"
    );
    assert!(
        !out.contains("Trip"),
        "no synonymy with Trip may leak into a vacation-only sentence: {out}"
    );
}
