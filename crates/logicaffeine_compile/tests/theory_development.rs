//! End to end: a Tarski-geometry source program — `## Theory` with formal `Axiom` and
//! `Theorem` declarations — compiles and every theorem is kernel-certified by the
//! multi-theorem driver. The surface seam (`## Axiom`/`## Theory` → formal-formula parser
//! → library driver → kernel) proven from source text.

use logicaffeine_compile::ui_bridge::compile_theory_for_ui;

#[test]
fn tarski_theory_block_program_is_kernel_certified() {
    // The opening Tarski congruence development as a real source program: a shared axiom
    // base (A1 pseudo-reflexivity, A2 inner transitivity) and three theorems discharged
    // in citation order — reflexivity, symmetry (cites reflexivity), transitivity (cites
    // symmetry). Each independently kernel-certified.
    let program = "\
## Theory Tarski

Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem symmetry cites reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
Theorem transitivity cites symmetry: prove for all a b c d e f, if Cong(a, b, c, d) and Cong(c, d, e, f) then Cong(a, b, e, f).
";

    let result = compile_theory_for_ui(program);
    assert!(result.parse_error.is_none(), "parse error: {:?}", result.parse_error);
    assert_eq!(result.theory_name.as_deref(), Some("Tarski"));
    assert_eq!(result.axiom_count, 2, "two Tarski axioms in scope");
    assert_eq!(result.theorems.len(), 3, "three theorems");
    for t in &result.theorems {
        assert!(t.verified, "theorem '{}' must be kernel-certified: {:?}", t.name, t.error);
    }
    assert!(result.all_verified());
}

#[test]
fn standalone_axiom_blocks_form_the_base_for_a_theory() {
    // `## Axiom` blocks declared standalone form the shared base; the `## Theory` block's
    // theorem is proved against them. Exercises the merge of top-level axioms with a
    // theory development.
    let program = "\
## Axiom flip: for all a b, Cong(a, b, b, a).

## Axiom trans: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).

## Theory Congruence

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem symmetry cites reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
";
    let result = compile_theory_for_ui(program);
    assert!(result.parse_error.is_none(), "parse error: {:?}", result.parse_error);
    assert_eq!(result.axiom_count, 2, "two standalone axioms");
    assert_eq!(result.theorems.len(), 2);
    assert!(result.all_verified(), "all theorems certified: {:?}", result.theorems);
}

#[test]
fn an_unfounded_theorem_does_not_certify() {
    // Honesty: a theorem that does NOT follow from the axioms is reported as unverified —
    // the driver never rubber-stamps. (Reflexivity needs BOTH A1 and A2; with only A1 it
    // cannot be derived.)
    let program = "\
## Theory Weak

Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
";
    let result = compile_theory_for_ui(program);
    assert!(result.parse_error.is_none(), "parse error: {:?}", result.parse_error);
    assert_eq!(result.theorems.len(), 1);
    assert!(
        !result.theorems[0].verified,
        "reflexivity must NOT certify from A1 alone (needs A2)"
    );
    assert!(!result.all_verified());
}

#[test]
fn full_tarski_opening_development_congruence_and_betweenness() {
    // The opening of Tarski's geometry as ONE surface program: the congruence axioms
    // (A1 pseudo-reflexivity, A2 inner transitivity, A3 identity) and the betweenness
    // axioms (A4 segment construction, A6 identity of betweenness, A7 inner Pasch) as a
    // shared base, with seven theorems discharged against it in citation order — congruence
    // reflexivity → symmetry → transitivity, the null-segment identity, segment
    // construction, Pasch's meeting cevians, and the degenerate-betweenness collapse.
    // Every theorem independently kernel-certified, end to end from source text.
    let program = "\
## Theory Tarski

Axiom cong_pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom cong_inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).
Axiom cong_identity: for all a b c, if Cong(a, b, c, c) then a = b.
Axiom segment_construction: for all q a b c, there exists x, Bet(q, a, x) and Cong(a, x, b, c).
Axiom bet_identity: for all a b, if Bet(a, b, a) then a = b.
Axiom pasch: for all a p c b q, if Bet(a, p, c) and Bet(b, q, c) then there exists x, Bet(p, x, b) and Bet(q, x, a).
Axiom five_segment: for all a b c d ap bp cp dp, if not (a = b) and Bet(a, b, c) and Bet(ap, bp, cp) and Cong(a, b, ap, bp) and Cong(b, c, bp, cp) and Cong(a, d, ap, dp) and Cong(b, d, bp, dp) then Cong(c, d, cp, dp).

Theorem cong_reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem cong_symmetry cites cong_reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
Theorem cong_transitivity cites cong_symmetry: prove for all a b c d e f, if Cong(a, b, c, d) and Cong(c, d, e, f) then Cong(a, b, e, f).
Theorem null_segment_identity: given Cong(P, Q, R, R); prove P = Q.
Theorem construct_point: prove there exists x, Bet(Q, A, x) and Cong(A, x, B, C).
Theorem cevians_meet: given Bet(A, P, C); given Bet(B, Q, C); prove there exists x, Bet(P, x, B) and Bet(Q, x, A).
Theorem degenerate_betweenness: given Bet(P, Q, P); prove P = Q.
Theorem outer_five_segment: given not (A = B); given Bet(A, B, C); given Bet(Ap, Bp, Cp); given Cong(A, B, Ap, Bp); given Cong(B, C, Bp, Cp); given Cong(A, D, Ap, Dp); given Cong(B, D, Bp, Dp); prove Cong(C, D, Cp, Dp).
";
    // All FIVE core Tarski axiom groups in one base — including the seven-antecedent
    // five-segment A5, which previously made a `Cong` goal's search explode against the
    // recursive inner-transitivity A2. The backward-chainer's relevance ordering now tries
    // the directly-dischargeable axiom first (and the node budget guarantees termination),
    // so the whole development certifies in well under a second.
    let result = compile_theory_for_ui(program);
    assert!(result.parse_error.is_none(), "parse error: {:?}", result.parse_error);
    assert_eq!(result.theory_name.as_deref(), Some("Tarski"));
    assert_eq!(result.axiom_count, 7, "seven Tarski axioms in scope");
    assert_eq!(result.theorems.len(), 8, "eight theorems");
    for t in &result.theorems {
        assert!(t.verified, "Tarski theorem '{}' must be kernel-certified: {:?}", t.name, t.error);
    }
    assert!(result.all_verified());
}
