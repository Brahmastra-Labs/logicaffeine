//! EXODIA Phase 4 gate — the ARCHITECT (equality saturation).
//!
//! Sprint 17: `UnionFind` extracted to `logicaffeine_base` (the kernel's
//! congruence closure and the compiler's e-graph share one implementation).
//! Sprint 18: `CompilerEGraph` — hash-consing, multi-arity congruence,
//! worklist propagation. Sprint 19: round-trip conversion. Sprints 20–21:
//! rewrite Groups 1–3, with the conditional rules gated on ORACLE FACTS
//! (interval non-negativity, scalar kinds) and float operands FAIL CLOSED.
//! Sprint 22: `optimize_program_v2` wired behind the pipeline.
//!
//! D11b: every rewrite rule carries a soundness certificate checked through
//! `logicaffeine_kernel` — the compiler's e-graph is a sibling of the proof
//! engine, not a stranger.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_base::union_find::UnionFind;
use logicaffeine_compile::optimize::egraph::extract::{best_tree, ExtractTree};
use logicaffeine_compile::optimize::egraph::rules;
use logicaffeine_compile::optimize::egraph::{CompilerEGraph, CompilerENode, NodeId};
use logicaffeine_compile::optimize::ScalarKind;

// =====================================================================
// Sprint 17 — UnionFind in base
// =====================================================================

#[test]
fn union_find_basics_in_base() {
    let mut uf = UnionFind::new();
    let a = uf.make_set();
    let b = uf.make_set();
    let c = uf.make_set();
    assert_ne!(uf.find(a), uf.find(b));
    assert!(uf.union(a, b));
    assert!(!uf.union(a, b), "re-union of the same class reports no merge");
    assert_eq!(uf.find(a), uf.find(b));
    assert_ne!(uf.find(a), uf.find(c));
    assert!(uf.union(b, c));
    let root = uf.find(a);
    assert_eq!(uf.find(b), root);
    assert_eq!(uf.find(c), root);
}

// =====================================================================
// Sprint 18 — CompilerEGraph core
// =====================================================================

/// Hash-consing: adding the same node twice yields the same id; adding a
/// node whose children were UNIONED afterwards is found congruent on rebuild.
#[test]
fn egraph_hash_consing_dedupes() {
    let mut eg = CompilerEGraph::new();
    let one_a = eg.add(CompilerENode::Int(1));
    let one_b = eg.add(CompilerENode::Int(1));
    assert_eq!(one_a, one_b, "identical leaves must hash-cons");

    let x = eg.add(CompilerENode::Var(0, 0));
    let add_a = eg.add(CompilerENode::Add(x, one_a));
    let add_b = eg.add(CompilerENode::Add(x, one_b));
    assert_eq!(add_a, add_b, "identical interior nodes must hash-cons");
}

/// Congruence: from x ≡ y, the e-graph must derive Add(x, 1) ≡ Add(y, 1)
/// (worklist propagation, not just direct unions).
#[test]
fn egraph_congruence_propagates() {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    let y = eg.add(CompilerENode::Var(1, 0));
    let one = eg.add(CompilerENode::Int(1));
    let fx = eg.add(CompilerENode::Add(x, one));
    let fy = eg.add(CompilerENode::Add(y, one));
    assert_ne!(eg.find(fx), eg.find(fy));

    eg.union(x, y);
    eg.rebuild();
    assert_eq!(
        eg.find(fx),
        eg.find(fy),
        "congruence must propagate x ≡ y into Add(x,1) ≡ Add(y,1)"
    );
}

/// Nested congruence: two levels deep, through distinct operators.
#[test]
fn egraph_congruence_propagates_transitively() {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    let y = eg.add(CompilerENode::Var(1, 0));
    let two = eg.add(CompilerENode::Int(2));
    let mx = eg.add(CompilerENode::Mul(x, two));
    let my = eg.add(CompilerENode::Mul(y, two));
    let nx = eg.add(CompilerENode::Not(mx));
    let ny = eg.add(CompilerENode::Not(my));
    eg.union(x, y);
    eg.rebuild();
    assert_eq!(eg.find(nx), eg.find(ny), "congruence must chain through two levels");
}

// =====================================================================
// Sprints 20–21 — rewrite rules
// =====================================================================

/// `x + 0 → x` for a PROVEN-INT x, both operand orders.
#[test]
fn add_zero_fires_for_proven_int() {
    for flip in [false, true] {
        let mut eg = CompilerEGraph::new();
        let x = eg.add(CompilerENode::Var(0, 0));
        eg.set_scalar(x, ScalarKind::Int);
        let zero = eg.add(CompilerENode::Int(0));
        let sum = if flip {
            eg.add(CompilerENode::Add(zero, x))
        } else {
            eg.add(CompilerENode::Add(x, zero))
        };
        eg.saturate(&rules::all());
        assert_eq!(
            eg.find(sum),
            eg.find(x),
            "x + 0 must unify with x (flip = {flip})"
        );
    }
}

/// `x + 0` must NOT fire when x is a proven FLOAT (-0.0 + 0.0 == +0.0, so
/// the identity is unsound bit-for-bit) and must NOT fire when x carries no
/// scalar fact at all (fail closed).
#[test]
fn add_zero_fails_closed_for_floats_and_unknowns() {
    for fact in [Some(ScalarKind::Float), None] {
        let mut eg = CompilerEGraph::new();
        let x = eg.add(CompilerENode::Var(0, 0));
        if let Some(k) = fact {
            eg.set_scalar(x, k);
        }
        let zero = eg.add(CompilerENode::Int(0));
        let sum = eg.add(CompilerENode::Add(x, zero));
        eg.saturate(&rules::all());
        assert_ne!(
            eg.find(sum),
            eg.find(x),
            "x + 0 must NOT rewrite under fact {fact:?}"
        );
    }
}

/// Group 1 family over proven ints: x*1→x, x-0→x, x-x→0, x/1→x, x*0→0.
#[test]
fn group1_identities_fire_for_proven_ints() {
    // (build, expected-partner-is-x)
    let cases: Vec<(&str, fn(&mut CompilerEGraph, NodeId) -> (NodeId, NodeId))> = vec![
        ("mul-one", |eg, x| {
            let one = eg.add(CompilerENode::Int(1));
            (eg.add(CompilerENode::Mul(x, one)), x)
        }),
        ("sub-zero", |eg, x| {
            let zero = eg.add(CompilerENode::Int(0));
            (eg.add(CompilerENode::Sub(x, zero)), x)
        }),
        ("div-one", |eg, x| {
            let one = eg.add(CompilerENode::Int(1));
            (eg.add(CompilerENode::Div(x, one)), x)
        }),
        ("sub-self", |eg, x| {
            let zero = eg.add(CompilerENode::Int(0));
            (eg.add(CompilerENode::Sub(x, x)), zero)
        }),
        ("mul-zero", |eg, x| {
            let zero = eg.add(CompilerENode::Int(0));
            (eg.add(CompilerENode::Mul(x, zero)), zero)
        }),
    ];
    for (name, build) in cases {
        let mut eg = CompilerEGraph::new();
        let x = eg.add(CompilerENode::Var(0, 0));
        eg.set_scalar(x, ScalarKind::Int);
        let (built, expect) = build(&mut eg, x);
        eg.saturate(&rules::all());
        assert_eq!(eg.find(built), eg.find(expect), "{name} must fire for proven ints");
    }
}

/// Group 2 boolean simplification over a proven-Bool x:
/// true ∧ x → x, false ∧ x → false, true ∨ x → true, false ∨ x → x,
/// x ∧ x → x, x ∨ x → x, x ∧ ¬x → false, x ∨ ¬x → true.
#[test]
fn group2_boolean_simplification() {
    type Build = fn(&mut CompilerEGraph, NodeId) -> (NodeId, NodeId);
    let cases: Vec<(&str, Build)> = vec![
        ("true-and", |eg, x| {
            let t = eg.add(CompilerENode::Bool(true));
            (eg.add(CompilerENode::And(t, x)), x)
        }),
        ("false-and", |eg, x| {
            let f = eg.add(CompilerENode::Bool(false));
            (eg.add(CompilerENode::And(f, x)), f)
        }),
        ("true-or", |eg, x| {
            let t = eg.add(CompilerENode::Bool(true));
            (eg.add(CompilerENode::Or(t, x)), t)
        }),
        ("false-or", |eg, x| {
            let f = eg.add(CompilerENode::Bool(false));
            (eg.add(CompilerENode::Or(f, x)), x)
        }),
        ("and-self", |eg, x| (eg.add(CompilerENode::And(x, x)), x)),
        ("or-self", |eg, x| (eg.add(CompilerENode::Or(x, x)), x)),
        ("and-not-self", |eg, x| {
            let n = eg.add(CompilerENode::Not(x));
            let f = eg.add(CompilerENode::Bool(false));
            (eg.add(CompilerENode::And(x, n)), f)
        }),
        ("or-not-self", |eg, x| {
            let n = eg.add(CompilerENode::Not(x));
            let t = eg.add(CompilerENode::Bool(true));
            (eg.add(CompilerENode::Or(x, n)), t)
        }),
    ];
    for (name, build) in cases {
        let mut eg = CompilerEGraph::new();
        let x = eg.add(CompilerENode::Var(0, 0));
        eg.set_scalar(x, ScalarKind::Bool);
        let (built, expect) = build(&mut eg, x);
        eg.saturate(&rules::all());
        assert_eq!(eg.find(built), eg.find(expect), "{name} must simplify");
    }
}

/// Boolean rules must NOT treat `x and y` over INTS as logical conjunction:
/// `And` is type-aware in this language (bitwise on Int) — `x ∧ x → x` IS
/// still sound bitwise, but `x ∧ ¬x → false` is NOT (it is 0, not false).
/// The bool-only rules fail closed without a Bool proof.
#[test]
fn group2_fails_closed_for_int_operands() {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    let n = eg.add(CompilerENode::Not(x));
    let xn = eg.add(CompilerENode::And(x, n));
    let f = eg.add(CompilerENode::Bool(false));
    eg.saturate(&rules::all());
    assert_ne!(
        eg.find(xn),
        eg.find(f),
        "x ∧ ¬x → false must NOT fire on Int operands (bitwise: x & !x == 0)"
    );
}

/// Group 3: `x * 2^n → x << n` is unconditional over wrapping ints.
#[test]
fn mul_pow2_becomes_shl() {
    // `x * 2^k → x << k` is now ORACLE-GATED (exact `*` promotes on overflow, `<<`
    // wraps), so the rewrite fires only when the proven interval of `x` makes the
    // product fit i64. Bounded `x ∈ [0, 1000]` ⇒ x*4 ≤ 4000 fits ⇒ the optimization
    // still fires (we kept it for the sound case; only the unsound unbounded case
    // is refused — there the backend keeps a checked multiply / LLVM shifts it).
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    eg.set_int_range(x, 0, 1000);
    let four = eg.add(CompilerENode::Int(4));
    let mul = eg.add(CompilerENode::Mul(x, four));
    let two = eg.add(CompilerENode::Int(2));
    let shl = eg.add(CompilerENode::Shl(x, two));
    eg.saturate(&rules::all());
    assert_eq!(eg.find(mul), eg.find(shl), "proven-bounded x * 4 must unify with x << 2");
}

/// Group 3 CONDITIONAL: `x / 2^n → x >> n` requires an Oracle proof x ≥ 0.
/// Positive polarity: interval [0, 100] → fires. Negative polarity:
/// interval [-5, 100] or no fact at all → must NOT fire (truncating ÷ and
/// arithmetic shift disagree on negatives).
#[test]
fn div_pow2_to_shr_requires_nonneg_proof() {
    // fires with the proof
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    eg.set_int_range(x, 0, 100);
    let four = eg.add(CompilerENode::Int(4));
    let div = eg.add(CompilerENode::Div(x, four));
    let two = eg.add(CompilerENode::Int(2));
    let shr = eg.add(CompilerENode::Shr(x, two));
    eg.saturate(&rules::all());
    assert_eq!(eg.find(div), eg.find(shr), "x/4 with x ∈ [0,100] must become x >> 2");

    // refuses without it
    for range in [Some((-5i64, 100i64)), None] {
        let mut eg = CompilerEGraph::new();
        let x = eg.add(CompilerENode::Var(0, 0));
        eg.set_scalar(x, ScalarKind::Int);
        if let Some((lo, hi)) = range {
            eg.set_int_range(x, lo, hi);
        }
        let four = eg.add(CompilerENode::Int(4));
        let div = eg.add(CompilerENode::Div(x, four));
        let two = eg.add(CompilerENode::Int(2));
        let shr = eg.add(CompilerENode::Shr(x, two));
        eg.saturate(&rules::all());
        assert_ne!(
            eg.find(div),
            eg.find(shr),
            "x/4 must NOT become x >> 2 under range {range:?}"
        );
    }
}

/// Group 3 CONDITIONAL: `x % 2^n → x & (2^n − 1)`, same non-negativity gate.
#[test]
fn mod_pow2_to_and_requires_nonneg_proof() {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    eg.set_int_range(x, 0, 1_000_000);
    let eight = eg.add(CompilerENode::Int(8));
    let md = eg.add(CompilerENode::Mod(x, eight));
    let seven = eg.add(CompilerENode::Int(7));
    let masked = eg.add(CompilerENode::BitAnd(x, seven));
    eg.saturate(&rules::all());
    assert_eq!(eg.find(md), eg.find(masked), "x % 8 with x ≥ 0 must become x & 7");

    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    let eight = eg.add(CompilerENode::Int(8));
    let md = eg.add(CompilerENode::Mod(x, eight));
    let seven = eg.add(CompilerENode::Int(7));
    let masked = eg.add(CompilerENode::BitAnd(x, seven));
    eg.saturate(&rules::all());
    assert_ne!(eg.find(md), eg.find(masked), "x % 8 must NOT mask without the proof");
}

/// THE case GVN misses: associativity. (a + b) + c and a + (b + c) are
/// different trees with different value numbers; equality saturation must
/// put them in one class.
#[test]
fn associativity_unifies_what_gvn_misses() {
    let mut eg = CompilerEGraph::new();
    let a = eg.add(CompilerENode::Var(0, 0));
    let b = eg.add(CompilerENode::Var(1, 0));
    let c = eg.add(CompilerENode::Var(2, 0));
    for v in [a, b, c] {
        eg.set_scalar(v, ScalarKind::Int);
    }
    let ab = eg.add(CompilerENode::Add(a, b));
    let left = eg.add(CompilerENode::Add(ab, c));
    let bc = eg.add(CompilerENode::Add(b, c));
    let right = eg.add(CompilerENode::Add(a, bc));
    eg.saturate(&rules::all());
    assert_eq!(
        eg.find(left),
        eg.find(right),
        "(a+b)+c must unify with a+(b+c) under saturation"
    );
}

/// Commutativity composes with the identity rules: 0 + (x * 1) → x.
#[test]
fn rules_compose_through_classes() {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    let zero = eg.add(CompilerENode::Int(0));
    let one = eg.add(CompilerENode::Int(1));
    let mul = eg.add(CompilerENode::Mul(x, one));
    let sum = eg.add(CompilerENode::Add(zero, mul));
    eg.saturate(&rules::all());
    assert_eq!(eg.find(sum), eg.find(x), "0 + (x * 1) must collapse to x");
}

/// Constant folding inside the e-graph: 2 + 3 lands in the class of 5,
/// with kernel-exact wrapping semantics at the i64 rim.
#[test]
fn egraph_constant_folds_with_exact_overflow_semantics() {
    let mut eg = CompilerEGraph::new();
    let two = eg.add(CompilerENode::Int(2));
    let three = eg.add(CompilerENode::Int(3));
    let sum = eg.add(CompilerENode::Add(two, three));
    let five = eg.add(CompilerENode::Int(5));
    eg.saturate(&rules::all());
    assert_eq!(eg.find(sum), eg.find(five), "2 + 3 must fold to 5");

    // Integer math is EXACT: `i64::MAX + 1` overflows i64, so its exact value is a
    // BigInt the Int e-node cannot represent. The Architect must NOT fold it to the
    // wrapped `i64::MIN` (that rewrite is unsound) — it stays an Add for the
    // runtime/exact tier to promote.
    let mut eg = CompilerEGraph::new();
    let max = eg.add(CompilerENode::Int(i64::MAX));
    let one = eg.add(CompilerENode::Int(1));
    let sum = eg.add(CompilerENode::Add(max, one));
    eg.saturate(&rules::all());
    let class = eg.find(sum);
    for k in [i64::MIN, i64::MAX, 0] {
        let lit = eg.add(CompilerENode::Int(k));
        assert_ne!(eg.find(lit), class, "i64::MAX + 1 must not fold to the wrapped {k}");
    }
}

/// Division folding must NOT fold a divide-by-zero (the runtime error is
/// the program's meaning; the Architect cannot erase it).
#[test]
fn egraph_never_folds_division_by_zero() {
    let mut eg = CompilerEGraph::new();
    let ten = eg.add(CompilerENode::Int(10));
    let zero = eg.add(CompilerENode::Int(0));
    let div = eg.add(CompilerENode::Div(ten, zero));
    eg.saturate(&rules::all());
    let class = eg.find(div);
    for k in [0i64, 10] {
        let lit = eg.add(CompilerENode::Int(k));
        assert_ne!(eg.find(lit), class, "10 / 0 must not fold to {k}");
    }
}

// =====================================================================
// Saturation discipline
// =====================================================================

/// A long associativity chain must terminate within the saturation budget
/// and stay under the node cap (no e-graph blow-up).
#[test]
fn saturation_respects_budget() {
    let mut eg = CompilerEGraph::new();
    let mut acc = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(acc, ScalarKind::Int);
    for i in 1..=24u32 {
        let v = eg.add(CompilerENode::Var(i, 0));
        eg.set_scalar(v, ScalarKind::Int);
        acc = eg.add(CompilerENode::Add(acc, v));
    }
    eg.saturate(&rules::all());
    assert!(
        eg.node_count() <= 10_000,
        "saturation must respect the node cap (got {})",
        eg.node_count()
    );
}

/// Saturation is deterministic: the same input program yields the same
/// final class structure on every run (deterministic rule order).
#[test]
fn saturation_is_deterministic() {
    let build = || {
        let mut eg = CompilerEGraph::new();
        let x = eg.add(CompilerENode::Var(0, 0));
        eg.set_scalar(x, ScalarKind::Int);
        eg.set_int_range(x, 0, 50);
        let two = eg.add(CompilerENode::Int(2));
        let four = eg.add(CompilerENode::Int(4));
        let m = eg.add(CompilerENode::Mul(x, two));
        let d = eg.add(CompilerENode::Div(m, four));
        eg.saturate(&rules::all());
        (eg.node_count(), {
            let mut eg = eg;
            eg.find(d)
        })
    };
    assert_eq!(build(), build(), "saturation must be deterministic");
}

// =====================================================================
// Cost extraction
// =====================================================================

fn tree_uses_mul(t: &ExtractTree) -> bool {
    matches!(t.node, CompilerENode::Mul(..)) || t.children.iter().any(tree_uses_mul)
}

fn tree_uses_div(t: &ExtractTree) -> bool {
    matches!(t.node, CompilerENode::Div(..)) || t.children.iter().any(tree_uses_div)
}

/// After saturation, extraction must select a cheaper form than Mul for
/// x * 2 (either x + x or x << 1 — both beat multiply in LogosCost).
#[test]
fn extraction_prefers_cheaper_than_mul() {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    let two = eg.add(CompilerENode::Int(2));
    let mul = eg.add(CompilerENode::Mul(x, two));
    eg.saturate(&rules::all());
    let tree = best_tree(&mut eg, mul);
    assert!(
        !tree_uses_mul(&tree),
        "extraction must not pick Mul when Add/Shl are in the class: {tree:?}"
    );
}

/// Extraction with a proven-non-negative dividend must drop the division.
#[test]
fn extraction_drops_proven_division() {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    eg.set_int_range(x, 0, 9_999);
    let four = eg.add(CompilerENode::Int(4));
    let div = eg.add(CompilerENode::Div(x, four));
    eg.saturate(&rules::all());
    let tree = best_tree(&mut eg, div);
    assert!(!tree_uses_div(&tree), "x/4 with x ≥ 0 must extract shift, not divide");
}

/// Extraction is total even with cycles in the class graph (x ≡ x + 0
/// creates a self-referential class; extraction must still terminate and
/// pick the finite representative).
#[test]
fn extraction_tolerates_cycles() {
    let mut eg = CompilerEGraph::new();
    let x = eg.add(CompilerENode::Var(0, 0));
    eg.set_scalar(x, ScalarKind::Int);
    let zero = eg.add(CompilerENode::Int(0));
    let sum = eg.add(CompilerENode::Add(x, zero));
    eg.union(x, sum);
    eg.rebuild();
    let tree = best_tree(&mut eg, sum);
    assert!(
        matches!(tree.node, CompilerENode::Var(..)),
        "the cyclic class must extract the Var leaf, got {tree:?}"
    );
}

// =====================================================================
// D11b — kernel-proven rule soundness
// =====================================================================

/// Every registered rewrite rule must carry a soundness certificate that
/// CHECKS through logicaffeine_kernel (ring for polynomial identities, lia
////omega for ordered facts, exhaustive Bool case analysis for Group 2).
/// No rule ships unproven.
#[test]
fn every_rule_is_kernel_certified() {
    let all = rules::all();
    assert!(
        all.len() >= 15,
        "Groups 1–3 must register at least 15 rules, got {}",
        all.len()
    );
    let verified = rules::verify_all_with_kernel()
        .unwrap_or_else(|e| panic!("kernel certification failed: {e}"));
    assert_eq!(
        verified,
        all.len(),
        "every rule must be certified — no unproven rewrites"
    );
}

/// The registry must contain the named EXODIA gate rules.
#[test]
fn registry_contains_the_exodia_gate_rules() {
    let names: Vec<&'static str> = rules::all().iter().map(|r| r.name).collect();
    for required in [
        "add-zero",
        "mul-one",
        "mul-zero",
        "sub-zero",
        "sub-self",
        "div-one",
        "mul-pow2-shl",
        "div-pow2-shr",
        "mod-pow2-and",
        "add-comm",
        "add-assoc",
        "mul-comm",
    ] {
        assert!(names.contains(&required), "missing rule '{required}' in {names:?}");
    }
}

// =====================================================================
// Sprint 22 — optimize_program_v2 wired (source-level, behavioral +
// structural)
// =====================================================================

mod v2_pipeline {
    use logicaffeine_compile::ast::stmt::{BinaryOpKind, Expr, Stmt};
    use logicaffeine_compile::compile::tw_outcome_with_args;
    use logicaffeine_compile::ui_bridge::with_v2_optimized_program;
    use logicaffeine_compile::vm::NativeTier;
    use logicaffeine_jit::ForgeTier;

    fn norm(s: &str) -> String {
        s.lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn v2_outcome(src: &str, argv: &[String]) -> (String, Option<String>) {
        let tier = ForgeTier::new();
        with_v2_optimized_program(src, |parsed, interner| match parsed {
            Ok((stmts, types, policies)) => logicaffeine_compile::vm::run_to_outcome_with_args(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                argv,
                Some(&tier as &dyn NativeTier),
            ),
            Err(advice) => (String::new(), Some(advice)),
        })
    }

    fn assert_v2_matches_raw(src: &str, argv: &[String]) {
        let (out, err) = v2_outcome(src, argv);
        let tw = tw_outcome_with_args(src, argv);
        assert_eq!(
            (norm(&out), &err),
            (norm(&tw.output), &tw.error),
            "v2-optimized VM diverged from the raw tree-walker on:\n{src}"
        );
    }

    fn expr_has_shl(e: &Expr) -> bool {
        match e {
            Expr::BinaryOp { op, left, right } => {
                *op == BinaryOpKind::Shl || expr_has_shl(left) || expr_has_shl(right)
            }
            Expr::Not { operand } => expr_has_shl(operand),
            _ => false,
        }
    }

    fn any_stmt_expr(stmts: &[Stmt], pred: &dyn Fn(&Expr) -> bool) -> bool {
        fn walk(s: &Stmt, pred: &dyn Fn(&Expr) -> bool) -> bool {
            match s {
                Stmt::Let { value, .. } | Stmt::Set { value, .. } => pred(value),
                Stmt::Show { object, .. } => pred(object),
                Stmt::While { cond, body, .. } => {
                    pred(cond) || body.iter().any(|b| walk(b, pred))
                }
                Stmt::If { cond, then_block, else_block } => {
                    pred(cond)
                        || then_block.iter().any(|b| walk(b, pred))
                        || else_block
                            .map(|eb| eb.iter().any(|b| walk(b, pred)))
                            .unwrap_or(false)
                }
                _ => false,
            }
        }
        stmts.iter().any(|s| walk(s, pred))
    }

    /// A dynamic `i * 8` whose multiplicand is bounded only by a RUNTIME `n`
    /// must NOT residualize as a wrapping shift: integer `*` is EXACT (it
    /// promotes to BigInt on overflow) while `<<` wraps, so without an Oracle
    /// proof that `i * 8` stays within i64 the strength reduction is unsound.
    /// The e-graph must keep the exact multiply (the backend still lowers it to
    /// a checked native `imul`) and the output must match the tree-walker.
    #[test]
    fn v2_refuses_unsound_mul_pow2_shift_when_overflow_possible() {
        let src = "## To native args () -> Seq of Text\n\
                   ## To native parseInt (s: Text) -> Int\n\
                   \n\
                   ## Main\n\
                   Let arguments be args().\n\
                   Let n be parseInt(item 2 of arguments).\n\
                   Let mutable total be 0.\n\
                   Let mutable i be 1.\n\
                   While i is at most n:\n\
                   \x20   Set total to total + i * 8.\n\
                   \x20   Set i to i + 1.\n\
                   Show total.\n";
        let argv = vec!["bench".to_string(), "1000".to_string()];
        let tier = ForgeTier::new();
        let (out, err, saw_shl) = with_v2_optimized_program(src, |parsed, interner| {
            let (stmts, types, policies) = parsed.expect("v2 parse");
            let saw_shl = any_stmt_expr(stmts, &expr_has_shl);
            let (out, err) = logicaffeine_compile::vm::run_to_outcome_with_args(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                &argv,
                Some(&tier as &dyn NativeTier),
            );
            (out, err, saw_shl)
        });
        assert_eq!(err, None);
        assert_eq!(norm(&out), "4004000");
        assert!(
            !saw_shl,
            "i * 8 with a runtime-bounded i must stay an EXACT multiply, not a wrapping shift \
             (the gated `r_mul_pow2_shl` only fires when the Oracle proves the product fits i64)"
        );
    }

    /// Mutation versioning: `x + 0` before and after `Set x` are DIFFERENT
    /// versions — simplifying both must not conflate them.
    #[test]
    fn v2_respects_mutation_versions() {
        assert_v2_matches_raw(
            "## Main\n\
             Let mutable x be 5.\n\
             Let a be x + 0.\n\
             Set x to x + 1.\n\
             Let b be x + 0.\n\
             Show a.\n\
             Show b.\n",
            &[],
        );
        let (out, _) = v2_outcome(
            "## Main\n\
             Let mutable x be 5.\n\
             Let a be x + 0.\n\
             Set x to x + 1.\n\
             Let b be x + 0.\n\
             Show a.\n\
             Show b.\n",
            &[],
        );
        assert_eq!(norm(&out), "5\n6");
    }

    /// Effectful calls inside arithmetic survive as OPAQUE leaves: the
    /// identity still simplifies AROUND the call, the call still happens
    /// exactly once.
    #[test]
    fn v2_preserves_opaque_calls() {
        assert_v2_matches_raw(
            "## To noisy (n: Int) -> Int:\n\
             \x20   Show n.\n\
             \x20   Return n * 2.\n\
             \n\
             ## Main\n\
             Let r be noisy(21) + 0.\n\
             Show r.\n",
            &[],
        );
    }

    /// Error semantics survive v2: a division by zero at iteration k still
    /// errors with identical partial output.
    #[test]
    fn v2_preserves_error_outcomes() {
        assert_v2_matches_raw(
            "## Main\nShow 1.\nLet mutable i be 1.\nWhile i is at most 500:\n\
             \x20   Let d be 150 - i.\n\
             \x20   Let q be 1000 / d.\n\
             \x20   Set i to i + 1 + q / 10000.\n\
             Show i.\n",
            &[],
        );
    }

    /// A mini-corpus of shapes (loops, conditionals, lists, recursion)
    /// through v2 must match the raw tree-walker bit-for-bit.
    #[test]
    fn v2_mini_corpus_matches_raw() {
        let programs = [
            // conditional accumulation with div/mod
            "## Main\n\
             Let mutable s be 0.\n\
             Let mutable i be 1.\n\
             While i is at most 200:\n\
             \x20   If i % 3 equals 0:\n\
             \x20       Set s to s + i / 4.\n\
             \x20   Set i to i + 1.\n\
             Show s.\n",
            // list build + index + mutation
            "## Main\n\
             Let mutable xs be [].\n\
             Let mutable i be 1.\n\
             While i is at most 50:\n\
             \x20   Push i * 2 to xs.\n\
             \x20   Set i to i + 1.\n\
             Set item 7 of xs to 999.\n\
             Let mutable t be 0.\n\
             Set i to 1.\n\
             While i is at most 50:\n\
             \x20   Set t to t + item i of xs.\n\
             \x20   Set i to i + 1.\n\
             Show t.\n",
            // recursion
            "## To fib (n: Int) -> Int:\n\
             \x20   If n is less than 2:\n\
             \x20       Return n.\n\
             \x20   Return fib(n - 1) + fib(n - 2).\n\
             \n\
             ## Main\n\
             Show fib(15).\n",
            // floats stay bit-exact
            "## Main\n\
             Let mutable acc be 0.0.\n\
             Let mutable i be 1.\n\
             While i is at most 100:\n\
             \x20   Set acc to acc + 1.0 / 3.0.\n\
             \x20   Set i to i + 1.\n\
             Show acc.\n",
        ];
        for src in programs {
            assert_v2_matches_raw(src, &[]);
        }
    }
}

// =====================================================================
// Sprint 23a — Group 5: deforestation / fusion (Len/Slice/Copy algebra)
// =====================================================================
//
// The modeled fragment grows Copy / Slice / Contains. The algebra removes
// INTERMEDIATE collections (the O(n) copy that exists only to be measured
// or indexed), never an error: Slice is non-total (bounds), so any rewrite
// that would DELETE a Slice needs in-bounds proofs from class facts.

mod fusion {
    use logicaffeine_compile::optimize::egraph::extract::best_tree;
    use logicaffeine_compile::optimize::egraph::rules;
    use logicaffeine_compile::optimize::egraph::{CompilerEGraph, CompilerENode};

    /// `len(copy(xs))` → `len(xs)` for a PROVEN collection: copy
    /// preserves length and cannot raise — the O(n) copy vanishes.
    #[test]
    fn len_of_copy_drops_the_copy() {
        let mut eg = CompilerEGraph::new();
        let xs = eg.add(CompilerENode::Var(0, 0));
        eg.set_collection(xs);
        let copy = eg.add(CompilerENode::Copy(xs));
        let len = eg.add(CompilerENode::Len(copy));
        eg.saturate(&rules::all());
        let tree = best_tree(&mut eg, len);
        assert!(
            matches!(tree.node, CompilerENode::Len(_)),
            "must stay a Len, got {:?}",
            tree.node
        );
        assert!(
            matches!(tree.children[0].node, CompilerENode::Var(..)),
            "the Copy must be gone, got {:?}",
            tree.children[0].node
        );
    }

    /// WITHOUT the collection proof the same shape must fail closed —
    /// `copy of` over a non-collection raises, and deleting it would
    /// erase that error.
    #[test]
    fn len_of_copy_fails_closed_without_collection_proof() {
        let mut eg = CompilerEGraph::new();
        let xs = eg.add(CompilerENode::Var(0, 0));
        let copy = eg.add(CompilerENode::Copy(xs));
        let len = eg.add(CompilerENode::Len(copy));
        eg.saturate(&rules::all());
        let tree = best_tree(&mut eg, len);
        assert!(
            matches!(tree.children[0].node, CompilerENode::Copy(_)),
            "unproven operand: the Copy must survive, got {:?}",
            tree.children[0].node
        );
    }

    /// `index(copy(xs), i)` → `index(xs, i)`: same values, same bounds,
    /// same error — the copy is unobservable through one read.
    #[test]
    fn index_of_copy_reads_through() {
        let mut eg = CompilerEGraph::new();
        let xs = eg.add(CompilerENode::Var(0, 0));
        eg.set_collection(xs);
        let i = eg.add(CompilerENode::Var(1, 0));
        let copy = eg.add(CompilerENode::Copy(xs));
        let idx = eg.add(CompilerENode::Index(copy, i));
        eg.saturate(&rules::all());
        let tree = best_tree(&mut eg, idx);
        assert!(matches!(tree.node, CompilerENode::Index(..)));
        assert!(
            matches!(tree.children[0].node, CompilerENode::Var(..)),
            "the Copy must be gone, got {:?}",
            tree.children[0].node
        );
    }

    /// `copy(copy(xs))` → `copy(xs)`: both produce a fresh unaliased
    /// value with identical contents, and an erroring operand raises the
    /// same FIRST error on both sides — unconditional.
    #[test]
    fn copy_of_copy_collapses() {
        let mut eg = CompilerEGraph::new();
        let xs = eg.add(CompilerENode::Var(0, 0));
        let inner = eg.add(CompilerENode::Copy(xs));
        let outer = eg.add(CompilerENode::Copy(inner));
        eg.saturate(&rules::all());
        let tree = best_tree(&mut eg, outer);
        assert!(matches!(tree.node, CompilerENode::Copy(_)));
        assert!(
            matches!(tree.children[0].node, CompilerENode::Var(..)),
            "nested Copy must collapse, got {:?}",
            tree.children[0].node
        );
    }

    /// `slice(xs, 1, len(xs))` and `copy(xs)` are the same value for a
    /// PROVEN LIST: the full slice's clamps are no-ops, both are fresh.
    #[test]
    fn full_slice_unifies_with_copy() {
        let mut eg = CompilerEGraph::new();
        let xs = eg.add(CompilerENode::Var(0, 0));
        eg.set_list(xs);
        let one = eg.add(CompilerENode::Int(1));
        let len = eg.add(CompilerENode::Len(xs));
        let slice = eg.add(CompilerENode::Slice(xs, one, len));
        let copy = eg.add(CompilerENode::Copy(xs));
        eg.saturate(&rules::all());
        assert_eq!(
            eg.find(slice),
            eg.find(copy),
            "the full slice and the copy must share one class"
        );
    }

    /// `len(slice(xs, 2, 5))` → 4, but ONLY under proofs: 1 ≤ a,
    /// a ≤ b + 1, b ≤ len(xs), and xs a proven list. With
    /// len(xs) ∈ [10, 20] every check passes and the length is b − a + 1.
    #[test]
    fn len_of_slice_folds_under_bounds_proofs() {
        let mut eg = CompilerEGraph::new();
        let xs = eg.add(CompilerENode::Var(0, 0));
        eg.set_list(xs);
        let a = eg.add(CompilerENode::Int(2));
        let b = eg.add(CompilerENode::Int(5));
        let len_xs = eg.add(CompilerENode::Len(xs));
        eg.set_int_range(len_xs, 10, 20);
        let slice = eg.add(CompilerENode::Slice(xs, a, b));
        let len_slice = eg.add(CompilerENode::Len(slice));
        eg.saturate(&rules::all());
        let tree = best_tree(&mut eg, len_slice);
        assert!(
            matches!(tree.node, CompilerENode::Int(4)),
            "len(slice(xs,2,5)) must fold to 4 under proofs, got {:?}",
            tree.node
        );
    }

    /// The same fold MUST fail closed without the length proof — the
    /// clamps could engage on a short list and the value would be wrong.
    #[test]
    fn len_of_slice_fails_closed_without_proofs() {
        let mut eg = CompilerEGraph::new();
        let xs = eg.add(CompilerENode::Var(0, 0));
        eg.set_list(xs);
        let a = eg.add(CompilerENode::Int(2));
        let b = eg.add(CompilerENode::Int(5));
        let slice = eg.add(CompilerENode::Slice(xs, a, b));
        let len_slice = eg.add(CompilerENode::Len(slice));
        eg.saturate(&rules::all());
        let tree = best_tree(&mut eg, len_slice);
        assert!(
            matches!(tree.node, CompilerENode::Len(_)),
            "without a len(xs) bound the fold must not fire, got {:?}",
            tree.node
        );
        assert!(
            matches!(tree.children[0].node, CompilerENode::Slice(..)),
            "the Slice must survive, got {:?}",
            tree.children[0].node
        );
    }

    /// The fusion rules ship certified like every other rule (D11b): the
    /// collection algebra carries executable LOGOS property certificates.
    #[test]
    fn fusion_rules_registered_and_certified() {
        let names: Vec<&'static str> = rules::all().iter().map(|r| r.name).collect();
        for required in
            ["len-copy", "index-copy", "copy-copy", "slice-full-copy", "len-slice-bounds"]
        {
            assert!(names.contains(&required), "missing rule '{required}' in {names:?}");
        }
        let all = rules::all();
        let verified = rules::verify_all_with_kernel()
            .unwrap_or_else(|e| panic!("certification failed: {e}"));
        assert_eq!(verified, all.len(), "every rule (fusion included) must be certified");
    }

    /// Pipeline-level: `length of (copy of xs)` in a loop must residualize
    /// WITHOUT the copy (the e-graph fired through the real v2 pipeline),
    /// with exact behavioral parity.
    #[test]
    fn v2_residualizes_len_of_copy_without_the_copy() {
        use logicaffeine_compile::ast::stmt::{Expr, Stmt};
        use logicaffeine_compile::compile::tw_outcome_with_args;
        use logicaffeine_compile::ui_bridge::with_v2_optimized_program;

        let src = "## Main\n\
                   Let mutable xs be [].\n\
                   Let mutable i be 1.\n\
                   While i is at most 200:\n\
                   \x20   Push i to xs.\n\
                   \x20   Set i to i + 1.\n\
                   Let mutable t be 0.\n\
                   Set i to 1.\n\
                   While i is at most 200:\n\
                   \x20   Set t to t + length of (copy of xs).\n\
                   \x20   Set i to i + 1.\n\
                   Show t.\n";

        fn expr_has_len_of_copy(e: &Expr) -> bool {
            match e {
                Expr::Length { collection } => matches!(collection, Expr::Copy { .. }),
                Expr::BinaryOp { left, right, .. } => {
                    expr_has_len_of_copy(left) || expr_has_len_of_copy(right)
                }
                Expr::Not { operand } => expr_has_len_of_copy(operand),
                _ => false,
            }
        }
        fn stmt_has_len_of_copy(s: &Stmt) -> bool {
            match s {
                Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_has_len_of_copy(value),
                Stmt::Show { object, .. } => expr_has_len_of_copy(object),
                Stmt::While { cond, body, .. } => {
                    expr_has_len_of_copy(cond) || body.iter().any(stmt_has_len_of_copy)
                }
                Stmt::If { cond, then_block, else_block } => {
                    expr_has_len_of_copy(cond)
                        || then_block.iter().any(stmt_has_len_of_copy)
                        || else_block
                            .map(|b| b.iter().any(stmt_has_len_of_copy))
                            .unwrap_or(false)
                }
                _ => false,
            }
        }

        let (out, err, survived) = with_v2_optimized_program(src, |parsed, interner| {
            let (stmts, types, policies) = parsed.expect("v2 parse");
            let survived = stmts.iter().any(stmt_has_len_of_copy);
            let (out, err) = logicaffeine_compile::vm::run_to_outcome_with_args(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                &[],
                None,
            );
            (out, err, survived)
        });
        assert_eq!(err, None);
        let tw = tw_outcome_with_args(src, &[]);
        assert_eq!(out.trim(), tw.output.trim(), "v2 diverged from raw");
        assert!(!survived, "length of (copy of xs) must residualize without the Copy");
    }
}

// =====================================================================
// Sprint 23b — cross-statement e-graph runs (the GVN replacement)
// =====================================================================
//
// One e-graph per STRAIGHT-LINE statement run: a Let merges its variable
// with its defining expression's class, so later statements extract
// against everything proven so far. Versioning makes mutation kills
// structural: Set bumps the target's version, effectful statements bump
// everything — equality can never leak across a write.

mod cross_statement {
    use logicaffeine_compile::ast::stmt::{BinaryOpKind, Expr, Stmt};
    use logicaffeine_compile::compile::tw_outcome_with_args;
    use logicaffeine_compile::ui_bridge::with_v2_optimized_program;

    fn norm(s: &str) -> String {
        s.lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn count_muls(e: &Expr) -> usize {
        match e {
            Expr::BinaryOp { op, left, right } => {
                usize::from(*op == BinaryOpKind::Multiply) + count_muls(left) + count_muls(right)
            }
            Expr::Not { operand } => count_muls(operand),
            _ => 0,
        }
    }

    fn count_adds(e: &Expr) -> usize {
        match e {
            Expr::BinaryOp { op, left, right } => {
                usize::from(*op == BinaryOpKind::Add) + count_adds(left) + count_adds(right)
            }
            Expr::Not { operand } => count_adds(operand),
            _ => 0,
        }
    }

    /// Runs `src` through v2 with `argv`; returns (output, error, per-Show
    /// metric) where the metric applies `f` to every top-level Show object.
    fn v2_with_show_metric(
        src: &str,
        argv: &[String],
        f: fn(&Expr) -> usize,
    ) -> (String, Option<String>, Vec<usize>) {
        with_v2_optimized_program(src, |parsed, interner| {
            let (stmts, types, policies) = parsed.expect("v2 parse");
            let metrics: Vec<usize> = stmts
                .iter()
                .filter_map(|s| match s {
                    Stmt::Show { object, .. } => Some(f(object)),
                    _ => None,
                })
                .collect();
            let (out, err) = logicaffeine_compile::vm::run_to_outcome_with_args(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                argv,
                None,
            );
            (out, err, metrics)
        })
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    /// Behavioral + metric harness: v2 output must equal the raw
    /// tree-walker's on the same argv.
    fn v2_metric_with_parity(
        src: &str,
        args: &[String],
        f: fn(&Expr) -> usize,
    ) -> (String, Vec<usize>) {
        let (out, err, metrics) = v2_with_show_metric(src, args, f);
        let tw = tw_outcome_with_args(src, args);
        assert_eq!(
            (norm(&out), &err),
            (norm(&tw.output), &tw.error),
            "v2 diverged from raw tree-walker on:\n{src}"
        );
        assert_eq!(err, None);
        (norm(&out), metrics)
    }

    /// GVN-parity through value reuse: `x * y` is already bound to `a`,
    /// so the Show extracts `a + a` — zero multiplies left. Inputs come
    /// from argv so no constant-folding pass can trivialize the pin.
    #[test]
    fn cse_reuses_a_prior_let_binding() {
        let src = "## To native args () -> Seq of Text\n\
                   ## To native parseInt (s: Text) -> Int\n\
                   \n\
                   ## Main\n\
                   Let arguments be args().\n\
                   Let x be parseInt(item 2 of arguments).\n\
                   Let y be x + 3.\n\
                   Let z be x * y.\n\
                   Show z + x * y.\n";
        let (out, muls) = v2_metric_with_parity(src, &argv(&["bench", "11"]), count_muls);
        assert_eq!(out, "308", "11·14 + 11·14");
        assert_eq!(
            muls,
            vec![0],
            "the Show must reuse `z` for x * y (got {muls:?} multiplies)"
        );
    }

    /// THE case GVN misses: associativity. `z` binds (p + q) + r; the
    /// Show's p + (q + r) is the same value only through add-assoc — the
    /// e-graph bridges it, syntactic value numbering cannot.
    #[test]
    fn associativity_reuse_across_statements() {
        let src = "## To native args () -> Seq of Text\n\
                   ## To native parseInt (s: Text) -> Int\n\
                   \n\
                   ## Main\n\
                   Let arguments be args().\n\
                   Let p be parseInt(item 2 of arguments).\n\
                   Let q be p + 1.\n\
                   Let r be p + 2.\n\
                   Let z be (p + q) + r.\n\
                   Show z + (p + (q + r)).\n";
        let (out, adds) = v2_metric_with_parity(src, &argv(&["bench", "11"]), count_adds);
        assert_eq!(out, "72", "(11+12+13) doubled");
        assert_eq!(
            adds,
            vec![1],
            "the Show must extract z + z — one Add, no rebuilt sum (got {adds:?})"
        );
    }

    /// Soundness pin: a `Set` between the binding and the use KILLS the
    /// equality — the Show must keep its own multiply and the values
    /// must match the raw tree-walker exactly. Both p versions arrive
    /// via argv, so no fold can erase the multiplies either way.
    #[test]
    fn mutation_kills_cross_statement_reuse() {
        let src = "## To native args () -> Seq of Text\n\
                   ## To native parseInt (s: Text) -> Int\n\
                   \n\
                   ## Main\n\
                   Let arguments be args().\n\
                   Let mutable p be parseInt(item 2 of arguments).\n\
                   Let q be parseInt(item 3 of arguments).\n\
                   Let z be p * q.\n\
                   Set p to parseInt(item 4 of arguments).\n\
                   Show z + p * q.\n";
        let (out, muls) =
            v2_metric_with_parity(src, &argv(&["bench", "6", "7", "9"]), count_muls);
        assert_eq!(out, "105", "42 + 63");
        assert_eq!(
            muls,
            vec![1],
            "p changed: the Show must recompute p * q (got {muls:?})"
        );
    }

    /// Soundness pin: an effectful statement (a call that Shows) between
    /// binding and use kills reuse of anything it may touch; behavior
    /// stays exact — bump's output appears exactly once.
    #[test]
    fn effectful_statement_kills_reuse_conservatively() {
        let src = "## To native args () -> Seq of Text\n\
                   ## To native parseInt (s: Text) -> Int\n\
                   \n\
                   ## To bump (n: Int) -> Int:\n\
                   \x20   Show n.\n\
                   \x20   Return n + 1.\n\
                   \n\
                   ## Main\n\
                   Let arguments be args().\n\
                   Let mutable p be parseInt(item 2 of arguments).\n\
                   Let z be p * 7.\n\
                   Set p to bump(p).\n\
                   Show z + p * 7.\n";
        let (out, _) = v2_metric_with_parity(src, &argv(&["bench", "6"]), count_muls);
        assert_eq!(out, "6\n91", "bump shows 6; 42 + 49 = 91");
    }

    /// Runs stop at control-flow boundaries: a binding before a While is
    /// reusable inside only if sound — the loop mutates x, so the use
    /// after the loop must NOT collapse to the stale binding.
    #[test]
    fn loop_mutation_respects_run_boundaries() {
        let src = "## To native args () -> Seq of Text\n\
                   ## To native parseInt (s: Text) -> Int\n\
                   \n\
                   ## Main\n\
                   Let arguments be args().\n\
                   Let mutable x be parseInt(item 2 of arguments).\n\
                   Let z be x * 10.\n\
                   Let mutable i be 0.\n\
                   While i is less than 3:\n\
                   \x20   Set x to x + 1.\n\
                   \x20   Set i to i + 1.\n\
                   Show z + x * 10.\n";
        let (out, _) = v2_metric_with_parity(src, &argv(&["bench", "2"]), count_muls);
        assert_eq!(out, "70", "20 + 50");
    }

    /// SOUNDNESS (regression): a variable MUTATED inside a loop must not have
    /// its LOOP-GUARD occurrence constant-folded to its entry value. `d` starts
    /// at 2 and is incremented in the body, so the guard `d * d <= i` must stay
    /// symbolic — folding it to `4 <= i` (the e-graph applying the Oracle's
    /// first-iteration range for `d` as a UNIVERSAL rewrite) collapsed primes
    /// to a constant 2. Caught by the benchmark differential, never the unit
    /// suite until now — pinned here forever.
    #[test]
    fn loop_mutated_var_in_guard_is_not_const_folded() {
        let src = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable count be 0.
Let mutable i be 2.
While i is at most n:
    Let mutable isPrime be 1.
    Let mutable d be 2.
    While d * d is at most i:
        If i % d equals 0:
            Set isPrime to 0.
            Break.
        Set d to d + 1.
    If isPrime equals 1:
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
        let (out, _) = v2_metric_with_parity(src, &argv(&["bench", "30"]), count_muls);
        assert_eq!(out, "10", "primes <= 30: 2,3,5,7,11,13,17,19,23,29");
    }
}
