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
    assert!(
        result.parse_error.is_none(),
        "parse error: {:?}",
        result.parse_error
    );
    assert_eq!(result.theory_name.as_deref(), Some("Tarski"));
    assert_eq!(result.axiom_count, 2, "two Tarski axioms in scope");
    assert_eq!(result.theorems.len(), 3, "three theorems");
    for t in &result.theorems {
        assert!(
            t.verified,
            "theorem '{}' must be kernel-certified: {:?}",
            t.name, t.error
        );
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
    assert!(
        result.parse_error.is_none(),
        "parse error: {:?}",
        result.parse_error
    );
    assert_eq!(result.axiom_count, 2, "two standalone axioms");
    assert_eq!(result.theorems.len(), 2);
    assert!(
        result.all_verified(),
        "all theorems certified: {:?}",
        result.theorems
    );
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
    assert!(
        result.parse_error.is_none(),
        "parse error: {:?}",
        result.parse_error
    );
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
    assert!(
        result.parse_error.is_none(),
        "parse error: {:?}",
        result.parse_error
    );
    assert_eq!(result.theory_name.as_deref(), Some("Tarski"));
    assert_eq!(result.axiom_count, 7, "seven Tarski axioms in scope");
    assert_eq!(result.theorems.len(), 8, "eight theorems");
    for t in &result.theorems {
        assert!(
            t.verified,
            "Tarski theorem '{}' must be kernel-certified: {:?}",
            t.name, t.error
        );
    }
    assert!(result.all_verified());
}

/// Andersen points-to as a development: allocation, assignment, store and load as four Horn rules,
/// and the facts of one program as axioms. Every name in a fact is lowercase, so each is a free
/// variable of the development — a fixed, arbitrary value (a context local), never instantiated —
/// and several (`h1`, `h2`, `f`) share their spelling with the rules' bound variables. A rule's
/// binder and a local of the same name are different variables: the search must keep them apart,
/// prove what the rules derive, and refuse what they do not.
fn points_to_development(extra: &str) -> String {
    format!(
        "\
## Theory PointsTo

Axiom new_x: VP0(x, h1).
Axiom new_y: VP0(y, h2).
Axiom copy_z: Assign(z, x).
Axiom store_xf: Store(x, f, y).
Axiom load_wzf: Load(z, f, w).

Axiom alloc: for all v h, if VP0(v, h) then VP(v, h).
Axiom assign: for all v1 v2 h, if Assign(v1, v2) and VP(v2, h) then VP(v1, h).
Axiom store: for all v1 f v2 h1 h2, if Store(v1, f, v2) and VP(v1, h1) and VP(v2, h2) then HP(h1, f, h2).
Axiom load: for all v1 f v2 h1 h2, if Load(v1, f, v2) and VP(v1, h1) and HP(h1, f, h2) then VP(v2, h2).

{extra}"
    )
}

#[test]
fn a_rule_binder_never_captures_a_local_of_the_same_name() {
    let result = compile_theory_for_ui(&points_to_development(
        "\
Theorem z_points_to_h1: prove VP(z, h1).
Theorem field_holds_h2: prove HP(h1, f, h2).
Theorem w_points_to_h2: prove VP(w, h2).
",
    ));
    assert!(result.parse_error.is_none(), "parse error: {:?}", result.parse_error);
    assert_eq!(result.axiom_count, 9);
    assert_eq!(result.theorems.len(), 3);
    for t in &result.theorems {
        assert!(t.verified, "'{}' follows from the rules and must certify: {:?}", t.name, t.error);
    }
}

#[test]
fn a_fact_the_rules_do_not_derive_is_refused_not_a_panic() {
    let result = compile_theory_for_ui(&points_to_development(
        "\
Theorem y_points_to_h1: prove VP(y, h1).
Theorem w_points_to_h1: prove VP(w, h1).
Theorem field_holds_h1: prove HP(h1, f, h1).
",
    ));
    assert!(result.parse_error.is_none(), "parse error: {:?}", result.parse_error);
    assert_eq!(result.theorems.len(), 3);
    for t in &result.theorems {
        assert!(!t.verified, "'{}' is not derivable and must be refused", t.name);
        assert!(t.error.is_some(), "a refusal names its reason: '{}'", t.name);
    }
}

#[test]
fn the_same_development_with_constants_agrees_with_the_locals() {
    // The constants reading (uppercase names) is the control: the verdicts must not depend on
    // whether a fact's names are constants or locals.
    let lower = points_to_development(
        "\
Theorem z_points_to_h1: prove VP(z, h1).
Theorem y_points_to_h1: prove VP(y, h1).
",
    );
    let upper = lower
        .replace("(x, h1)", "(X, H1)")
        .replace("(y, h2)", "(Y, H2)")
        .replace("Assign(z, x)", "Assign(Z, X)")
        .replace("Store(x, f, y)", "Store(X, F, Y)")
        .replace("Load(z, f, w)", "Load(Z, F, W)")
        .replace("VP(z, h1)", "VP(Z, H1)")
        .replace("VP(y, h1)", "VP(Y, H1)");
    let verdicts = |src: &str| -> Vec<bool> {
        compile_theory_for_ui(src).theorems.iter().map(|t| t.verified).collect()
    };
    assert_eq!(verdicts(&upper), vec![true, false]);
    assert_eq!(verdicts(&lower), verdicts(&upper));
}

#[test]
fn a_rule_instantiated_at_a_local_certifies() {
    // The witness of the instantiation is the local `x`: the certifier must apply the rule to the
    // context local, not to a declaration of that spelling.
    let result = compile_theory_for_ui(
        "\
## Theory Locals

Axiom fact: P(x).
Axiom rule: for all v, if P(v) then Q(v).

Theorem direct: prove P(x).
Theorem chained: prove Q(x).
",
    );
    assert!(result.parse_error.is_none(), "parse error: {:?}", result.parse_error);
    for t in &result.theorems {
        assert!(t.verified, "'{}' must certify: {:?}", t.name, t.error);
    }
}

#[test]
fn a_local_spelled_like_a_hypothesis_still_rechecks() {
    // `h1` and `h2` are locals here and also the names the trust door gives its first two
    // hypotheses. The independent re-checker promotes locals to declarations; each must get a
    // name of its own, or a hypothesis's type comes to mention itself.
    let result = compile_theory_for_ui(
        "\
## Theory Locals

Axiom f1: VP0(x, h1).
Axiom f2: VP0(y, h2).
Axiom f3: Store(x, f, y).
Axiom alloc: for all p q, if VP0(p, q) then VP(p, q).
Axiom store: for all s1 g s2 k1 k2, if Store(s1, g, s2) and VP(s1, k1) and VP(s2, k2) then HP(k1, g, k2).

Theorem t1: prove HP(h1, f, h2).
Theorem t2: prove HP(h2, f, h1).
",
    );
    assert!(result.parse_error.is_none(), "parse error: {:?}", result.parse_error);
    let verdicts: Vec<(&str, bool)> = result.theorems.iter().map(|t| (t.name.as_str(), t.verified)).collect();
    assert_eq!(verdicts, vec![("t1", true), ("t2", false)]);
}
