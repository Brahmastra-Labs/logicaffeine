//! Bridge from the optimizer's AST to the kernel's linear-integer-arithmetic
//! decision procedure (`logicaffeine_kernel::lia`).
//!
//! The interval domain proves `x ∈ [lo, hi]` but cannot express the
//! cross-variable relations a loop nest establishes: `i + j < n` (nested
//! affine, string_search's `text[i+j]`), `w - wi + 1 <= cap + 1` under a
//! guard `w >= wi` (multi-variable relational, knapsack's `prev[w-wi+1]`),
//! or `u < length(dist)` where `u` is an element of an array whose values
//! are all `< n` and `length(dist) = n` (symbolic element bound,
//! graph_bfs's `dist[u]`). These are not three problems — they are one
//! decision: **is a goal inequality implied by a conjunction of affine facts
//! over the integers?**
//!
//! The kernel already answers it. `logicaffeine_kernel::lia` carries a
//! complete Fourier–Motzkin solver (`LinearExpr`, `Constraint`,
//! `fourier_motzkin_unsat`) that was, until now, only wired to the kernel's
//! own theorem-proving tactics — orphaned from the optimizer one crate over.
//! This module is the missing wire: it reifies AST index/bound expressions
//! into the kernel's `LinearExpr`, gathers loop/path guards as kernel
//! `Constraint`s, and discharges `1 <= idx <= length` through the kernel
//! prover. No second engine — the theorem prover IS the bounds prover.
//!
//! The single-variable relational recognizer (`affine_of`/`guard_proves` in
//! `abstract_interp`) is the special case of this with one variable and one
//! guard; this generalizes it to an arbitrary affine system.

use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
use crate::intern::Symbol;
use logicaffeine_kernel::lia::{fourier_motzkin_unsat, LinearExpr, Rational};

/// The kernel's linear-arithmetic types, re-exported under the optimizer's
/// names so callers (the abstract interpreter's `scalar_def` map, the bounds
/// prover) need not reach into the kernel crate directly. `LinExpr` aliases
/// `LinearExpr`.
pub(crate) use logicaffeine_kernel::lia::{Constraint, LinearExpr as LinExpr};

/// The kernel's `LinearExpr` indexes variables by `i64`; an interned symbol's
/// dense index is a stable, collision-free choice (we only ever build
/// variables this way, so the kernel's name-hash indices never clash).
fn vidx(s: Symbol) -> i64 {
    s.index() as i64
}

/// The linear expression `x` for a symbol.
pub(crate) fn var(s: Symbol) -> LinearExpr {
    LinearExpr::var(vidx(s))
}

/// The constant linear expression `n`.
pub(crate) fn konst(n: i64) -> LinearExpr {
    LinearExpr::constant(Rational::from_i64(n))
}

/// `a <= b` as a kernel constraint (`a - b <= 0`).
pub(crate) fn le(a: &LinearExpr, b: &LinearExpr) -> Constraint {
    Constraint { expr: a.sub(b), strict: false }
}

/// `a < b` as a kernel constraint (`a - b < 0`).
pub(crate) fn lt(a: &LinearExpr, b: &LinearExpr) -> Constraint {
    Constraint { expr: a.sub(b), strict: true }
}

/// `a >= b` as a kernel constraint.
pub(crate) fn ge(a: &LinearExpr, b: &LinearExpr) -> Constraint {
    le(b, a)
}

/// `a > b` as a kernel constraint.
pub(crate) fn gt(a: &LinearExpr, b: &LinearExpr) -> Constraint {
    lt(b, a)
}

/// `e >= 0` as a kernel constraint (`-e <= 0`).
pub(crate) fn nonneg(e: &LinearExpr) -> Constraint {
    Constraint { expr: e.neg(), strict: false }
}

/// Are `facts` (each a kernel `expr <= 0` / `< 0`) mutually SATISFIABLE? A
/// contradictory fact set proves every goal vacuously, so a bounds proof must
/// refuse to fire on one — a defensive net against any false hypothesis
/// (a stale or self-referential fact) slipping into the system.
pub(crate) fn consistent(facts: &[Constraint]) -> bool {
    !fourier_motzkin_unsat(facts)
}

/// Is `goal >= 0` implied by `facts` (each already a kernel `expr <= 0` / `< 0`)
/// over the integers? The integer negation of `goal >= 0` is `goal <= -1`, i.e.
/// `goal + 1 <= 0`; if `facts ∧ (goal + 1 <= 0)` is unsatisfiable, the goal is
/// valid. Soundness is the kernel's (rational-unsat ⟹ integer-unsat) and is
/// additionally certified against Z3 by the differential below.
pub(crate) fn prove(facts: &[Constraint], goal: &LinearExpr) -> bool {
    let neg = goal.add(&LinearExpr::constant(Rational::from_i64(1)));
    let mut system: Vec<Constraint> = facts.to_vec();
    system.push(Constraint { expr: neg, strict: false });
    fourier_motzkin_unsat(&system)
}

/// Reify an AST expression into a kernel `LinearExpr`, or `None` if it is not
/// affine over symbols (a product of two variables, a modulo, a call, …). The
/// multi-variable generalization of `abstract_interp::affine_of`.
pub(crate) fn lin_of(e: &Expr) -> Option<LinearExpr> {
    // Cap literals well below i64: the callers render coefficients back to
    // `i64` program constants, and a larger literal is not a bounds
    // expression worth proving, so decline it. The `LinearExpr` arithmetic
    // itself is exact at any magnitude.
    const LIT_CAP: u64 = 1 << 50;
    match e {
        Expr::Identifier(s) => Some(var(*s)),
        Expr::Literal(Literal::Number(n)) => {
            (n.unsigned_abs() <= LIT_CAP).then(|| konst(*n))
        }
        Expr::BinaryOp { op, left, right } => match op {
            BinaryOpKind::Add => Some(lin_of(left)?.add(&lin_of(right)?)),
            BinaryOpKind::Subtract => Some(lin_of(left)?.sub(&lin_of(right)?)),
            BinaryOpKind::Multiply => {
                let (l, r) = (lin_of(left)?, lin_of(right)?);
                if l.is_constant() {
                    Some(r.scale(&l.constant))
                } else if r.is_constant() {
                    Some(l.scale(&r.constant))
                } else {
                    None // nonlinear (x·y)
                }
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intern::Interner;

    struct Vars {
        i: Interner,
    }
    impl Vars {
        fn new() -> Self {
            Vars { i: Interner::new() }
        }
        fn s(&mut self, name: &str) -> Symbol {
            self.i.intern(name)
        }
    }

    /// `e + n`.
    fn plus(e: &LinearExpr, n: i64) -> LinearExpr {
        e.add(&konst(n))
    }
    /// `c·x`.
    fn term(x: Symbol, c: i64) -> LinearExpr {
        var(x).scale(&Rational::from_i64(c))
    }

    // ---- prove: the three hard-loss relations the engine exists to discharge ----

    #[test]
    fn prove_knapsack_lower_bound() {
        // Guard w >= wi ⊢ (w - wi + 1) >= 1, i.e. (w - wi) >= 0.
        let mut v = Vars::new();
        let (w, wi) = (v.s("w"), v.s("wi"));
        let guard = ge(&var(w), &var(wi)); // w >= wi
        let goal = var(w).sub(&var(wi)); // w - wi >= 0
        assert!(prove(&[guard], &goal));
    }

    #[test]
    fn prove_knapsack_upper_bound() {
        // Loop w <= cap, element wi >= 0, length(prev) = cap+1 ⊢
        // (w - wi + 1) <= cap + 1, i.e. cap - w + wi >= 0.
        let mut v = Vars::new();
        let (w, wi, cap) = (v.s("w"), v.s("wi"), v.s("cap"));
        let facts = vec![le(&var(w), &var(cap)), nonneg(&var(wi))];
        let goal = var(cap).sub(&var(w)).add(&var(wi)); // cap - w + wi
        assert!(prove(&facts, &goal));
    }

    #[test]
    fn knapsack_upper_unprovable_without_element_bound() {
        // Without wi >= 0 the upper bound does NOT hold (wi could be negative).
        let mut v = Vars::new();
        let (w, wi, cap) = (v.s("w"), v.s("wi"), v.s("cap"));
        let facts = vec![le(&var(w), &var(cap))];
        let goal = var(cap).sub(&var(w)).add(&var(wi));
        assert!(!prove(&facts, &goal));
    }

    #[test]
    fn prove_string_search_window() {
        // Outer i <= T - P, inner j <= P - 1, i >= 0, j >= 0 ⊢ i + j <= T - 1,
        // i.e. (T - 1) - (i + j) >= 0.
        let mut v = Vars::new();
        let (i, j, t, p) = (v.s("i"), v.s("j"), v.s("t"), v.s("p"));
        let facts = vec![
            le(&var(i), &var(t).sub(&var(p))), // i <= T - P
            le(&var(j), &plus(&var(p), -1)),   // j <= P - 1
            nonneg(&var(i)),
            nonneg(&var(j)),
        ];
        let ij = var(i).add(&var(j));
        let goal = plus(&var(t), -1).sub(&ij); // (T - 1) - (i + j)
        assert!(prove(&facts, &goal));
    }

    #[test]
    fn prove_graph_bfs_element_bound() {
        // u is an element of adj, all values in [0, n-1]; length(dist) = n.
        // Prove dist[u] (1-based u+1): lower u >= 0 and upper n - (u+1) >= 0.
        let mut v = Vars::new();
        let (u, n) = (v.s("u"), v.s("n"));
        let facts = vec![nonneg(&var(u)), le(&var(u), &plus(&var(n), -1))]; // 0 <= u <= n-1
        assert!(prove(&facts, &var(u))); // lower: (u+1) - 1 = u >= 0
        let upper = var(n).sub(&plus(&var(u), 1)); // n - (u + 1)
        assert!(prove(&facts, &upper));
    }

    #[test]
    fn prove_is_not_overeager() {
        // i <= n is NOT enough to prove i < n (the strict goal n - i - 1 >= 0).
        let mut v = Vars::new();
        let (i, n) = (v.s("i"), v.s("n"));
        let weak = le(&var(i), &var(n)); // i <= n
        let strict_goal = plus(&var(n).sub(&var(i)), -1); // n - i - 1
        assert!(!prove(&[weak], &strict_goal));
        // But i < n DOES prove i <= n (n - i >= 0).
        let strong = lt(&var(i), &var(n)); // i < n
        let nonstrict_goal = var(n).sub(&var(i)); // n - i
        assert!(prove(&[strong], &nonstrict_goal));
    }

    #[test]
    fn prove_empty_facts_only_proves_trivial() {
        let mut v = Vars::new();
        let x = v.s("x");
        assert!(!prove(&[], &var(x))); // cannot prove x >= 0 with no facts
        assert!(prove(&[], &konst(5))); // 5 >= 0 holds unconditionally
        assert!(!prove(&[], &konst(-1))); // -1 >= 0 never holds
    }

    #[test]
    fn prove_flattened_2d_constant_stride() {
        // arr[i*10 + j] with i,j ∈ [0,9], length(arr) = 100. The constant-stride
        // flattened access the single-var recognizer cannot match: prove
        // 1 <= i*10 + j + 1 <= 100.
        let mut v = Vars::new();
        let (i, j) = (v.s("i"), v.s("j"));
        let facts = vec![
            nonneg(&var(i)),
            le(&var(i), &konst(9)),
            nonneg(&var(j)),
            le(&var(j), &konst(9)),
        ];
        let idx0 = term(i, 10).add(&var(j)); // i*10 + j (0-based)
        assert!(prove(&facts, &idx0)); // lower: idx0 >= 0
        let upper = konst(99).sub(&idx0); // 99 - (i*10 + j) >= 0  (≤ len-1)
        assert!(prove(&facts, &upper));
    }

    // ---- lin_of: AST reification ----

    fn num<'a>(arena: &'a crate::arena::Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
        arena.alloc(Expr::Literal(Literal::Number(n)))
    }
    fn ident<'a>(arena: &'a crate::arena::Arena<Expr<'a>>, s: Symbol) -> &'a Expr<'a> {
        arena.alloc(Expr::Identifier(s))
    }
    fn binop<'a>(
        arena: &'a crate::arena::Arena<Expr<'a>>,
        op: BinaryOpKind,
        l: &'a Expr<'a>,
        r: &'a Expr<'a>,
    ) -> &'a Expr<'a> {
        arena.alloc(Expr::BinaryOp { op, left: l, right: r })
    }

    #[test]
    fn lin_of_extracts_affine_forms() {
        let arena = crate::arena::Arena::new();
        let mut v = Vars::new();
        let (w, wi) = (v.s("w"), v.s("wi"));
        // w - wi + 1
        let e = binop(
            &arena,
            BinaryOpKind::Add,
            binop(&arena, BinaryOpKind::Subtract, ident(&arena, w), ident(&arena, wi)),
            num(&arena, 1),
        );
        let lin = lin_of(e).unwrap();
        assert_eq!(lin, plus(&var(w).sub(&var(wi)), 1));
    }

    #[test]
    fn lin_of_scales_by_constant_factor() {
        let arena = crate::arena::Arena::new();
        let mut v = Vars::new();
        let i = v.s("i");
        let e1 = binop(&arena, BinaryOpKind::Multiply, num(&arena, 3), ident(&arena, i));
        let e2 = binop(&arena, BinaryOpKind::Multiply, ident(&arena, i), num(&arena, 3));
        assert_eq!(lin_of(e1).unwrap(), term(i, 3));
        assert_eq!(lin_of(e2).unwrap(), term(i, 3));
    }

    #[test]
    fn lin_of_rejects_nonlinear() {
        let arena = crate::arena::Arena::new();
        let mut v = Vars::new();
        let (i, j) = (v.s("i"), v.s("j"));
        let prod = binop(&arena, BinaryOpKind::Multiply, ident(&arena, i), ident(&arena, j));
        assert!(lin_of(prod).is_none()); // i·j nonlinear
        let m = binop(&arena, BinaryOpKind::Modulo, ident(&arena, i), num(&arena, 3));
        assert!(lin_of(m).is_none()); // i % 3 not affine
    }
}

/// Z3 certifier — the kernel's Fourier–Motzkin engine, checked against the SMT
/// solver. The fast `prove` ships on the compile hot path (Z3 is heavyweight
/// and optional; the oracle runs inside the measured run). This module is the
/// SOUNDNESS GROUND TRUTH: under the `verification` feature it re-encodes the
/// exact same affine `facts ⊢ goal >= 0` query to `logicaffeine-verify` (Z3
/// linear integer arithmetic) and asserts that every proof the kernel
/// discharges, Z3 confirms — the LIA tactic never claims what the solver
/// refutes. This is how the kernel's theorem-prover "ties into the math
/// solver" without putting Z3 on the budget-bounded run path.
#[cfg(all(test, feature = "verification"))]
mod z3_certifier {
    use super::*;
    use crate::intern::Interner;
    use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyOp, VerifyType};

    fn z3_name(idx: i64) -> String {
        format!("v{}", idx)
    }

    /// A kernel `Rational` that is integer-valued (every fact/goal we build is)
    /// as an `i64`.
    fn as_int(r: &Rational) -> i64 {
        r.to_i64()
            .expect("certifier only encodes integer constraints")
    }

    /// Encode a kernel `LinearExpr` as a Z3 integer term (deterministic order:
    /// `LinearExpr` already stores coefficients in a `BTreeMap`).
    fn encode(e: &LinearExpr) -> VerifyExpr {
        let mut acc = VerifyExpr::int(as_int(&e.constant));
        for (idx, coeff) in &e.coefficients {
            let term = VerifyExpr::binary(
                VerifyOp::Mul,
                VerifyExpr::int(as_int(coeff)),
                VerifyExpr::var(z3_name(*idx)),
            );
            acc = VerifyExpr::binary(VerifyOp::Add, acc, term);
        }
        acc
    }

    /// Does Z3 confirm `goal >= 0` is implied by `facts` (each kernel
    /// `expr <= 0` / `< 0`)? Mirrors `super::prove`, discharged by the solver.
    fn z3_confirms(facts: &[Constraint], goal: &LinearExpr) -> bool {
        let mut session = VerificationSession::new();
        let mut idxs: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for c in facts {
            for k in c.expr.coefficients.keys() {
                idxs.insert(*k);
            }
        }
        for k in goal.coefficients.keys() {
            idxs.insert(*k);
        }
        for idx in idxs {
            session.declare(&z3_name(idx), VerifyType::Int);
        }
        for c in facts {
            // expr <= 0 (or < 0).
            let lhs = encode(&c.expr);
            let zero = VerifyExpr::int(0);
            let assertion = if c.strict {
                VerifyExpr::lt(lhs, zero)
            } else {
                VerifyExpr::lte(lhs, zero)
            };
            session.assume(&assertion);
        }
        session.verify(&VerifyExpr::gte(encode(goal), VerifyExpr::int(0))).is_ok()
    }

    fn s(i: &mut Interner, name: &str) -> Symbol {
        i.intern(name)
    }

    /// The three hard-loss relations the engine exists to prove must each be
    /// confirmed by Z3 — the whole point of the build, certified end to end.
    #[test]
    fn z3_confirms_the_hard_loss_relations() {
        let mut i = Interner::new();
        let (w, wi, cap) = (s(&mut i, "w"), s(&mut i, "wi"), s(&mut i, "cap"));
        // knapsack lower: {w >= wi} ⊢ w - wi >= 0
        let f = vec![ge(&var(w), &var(wi))];
        let g = var(w).sub(&var(wi));
        assert!(prove(&f, &g) && z3_confirms(&f, &g));
        // knapsack upper: {w <= cap, wi >= 0} ⊢ cap - w + wi >= 0
        let f = vec![le(&var(w), &var(cap)), nonneg(&var(wi))];
        let g = var(cap).sub(&var(w)).add(&var(wi));
        assert!(prove(&f, &g) && z3_confirms(&f, &g));
        // string_search: {i <= T-P, j <= P-1, i>=0, j>=0} ⊢ (T-1)-(i+j) >= 0
        let (i2, j, t, p) = (s(&mut i, "i"), s(&mut i, "j"), s(&mut i, "t"), s(&mut i, "p"));
        let f = vec![
            le(&var(i2), &var(t).sub(&var(p))),
            le(&var(j), &var(p).add(&konst(-1))),
            nonneg(&var(i2)),
            nonneg(&var(j)),
        ];
        let g = var(t).add(&konst(-1)).sub(&var(i2).add(&var(j)));
        assert!(prove(&f, &g) && z3_confirms(&f, &g));
        // graph_bfs: {u >= 0, u <= n-1} ⊢ n - (u+1) >= 0
        let (u, n) = (s(&mut i, "u"), s(&mut i, "n"));
        let f = vec![nonneg(&var(u)), le(&var(u), &var(n).add(&konst(-1)))];
        let g = var(n).sub(&var(u).add(&konst(1)));
        assert!(prove(&f, &g) && z3_confirms(&f, &g));
    }

    /// Deterministic integer mixer (no `rand` — reproducible).
    fn mix(n: u64) -> u64 {
        let mut x = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51afd7ed558ccd);
        x ^= x >> 33;
        x
    }
    fn pick(seed: u64, k: u64, lo: i64, hi: i64) -> i64 {
        let span = (hi - lo + 1) as u64;
        lo + (mix(seed.wrapping_add(k.wrapping_mul(0x9E37_79B9_7F4A_7C15))) % span) as i64
    }

    /// THE soundness theorem, exhaustively sampled: over a large deterministic
    /// battery of small 3-variable affine systems and goals, every system the
    /// kernel's Fourier–Motzkin engine proves, Z3 also proves. A single
    /// counterexample (`prove` true while Z3 refutes) would be an unsound
    /// bounds elision — UB in both the AOT binary and the VM — and fails
    /// loudly. We also assert the engine isn't trivially weak (it proves a real
    /// fraction of what Z3 proves), so soundness isn't bought with uselessness.
    #[test]
    fn fm_proofs_are_always_z3_valid() {
        let mut i = Interner::new();
        let (x, y, z) = (s(&mut i, "x"), s(&mut i, "y"), s(&mut i, "z"));
        let vars = [x, y, z];
        let mk = |seed: u64, base: u64| -> LinearExpr {
            // a·x + b·y + c·z + d, coeffs ∈ [-2,2], const ∈ [-4,4]
            let mut e = konst(pick(seed, base + 3, -4, 4));
            for (vi, vv) in vars.iter().enumerate() {
                let coeff = pick(seed, base + vi as u64, -2, 2);
                e = e.add(&term_of(*vv, coeff));
            }
            e
        };
        let mut fm_proved = 0u32;
        let mut z3_proved = 0u32;
        let mut both = 0u32;
        let trials = 600u64;
        for seed in 0..trials {
            // 3 facts (each `expr <= 0`) + 1 goal.
            let facts: Vec<Constraint> = (0..3)
                .map(|fi| Constraint { expr: mk(seed, 10 + fi * 4), strict: false })
                .collect();
            let goal = mk(seed, 100);
            let fm = prove(&facts, &goal);
            let z3 = z3_confirms(&facts, &goal);
            if fm {
                fm_proved += 1;
                assert!(
                    z3,
                    "UNSOUND: kernel Fourier–Motzkin proved a goal Z3 refutes (seed {}). \
                     Coefficients are deterministic — reproduce with this seed.",
                    seed
                );
            }
            if z3 {
                z3_proved += 1;
            }
            if fm && z3 {
                both += 1;
            }
        }
        assert!(fm_proved > 0, "engine proved nothing across {} trials", trials);
        assert!(
            both * 10 >= z3_proved * 4,
            "Fourier–Motzkin recall too low: proved {}/{} of Z3-provable goals",
            both,
            z3_proved
        );
    }

    fn term_of(x: Symbol, c: i64) -> LinearExpr {
        var(x).scale(&Rational::from_i64(c))
    }
}
