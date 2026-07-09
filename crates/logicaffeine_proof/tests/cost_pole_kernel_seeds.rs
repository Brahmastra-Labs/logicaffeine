//! **The kernel seeds of "nothing finite is random" — at every ring class.**
//!
//! The partition-of-unity induction (the `∀n` engine of constructive Nullstellensatz completeness)
//! rests on one per-variable identity, the atom `(1 − x) + x = 1`, and the kill-or-absorb attainment
//! argument rests on the cube-point identities `b·b = b` and `b·(1 − b) = 0`. The atom is already a
//! kernel theorem over the fields — `gf2_ring_kernel.rs` (characteristic 2) and `gf3_ring_kernel.rs`
//! (characteristic 3, signed). This file completes the ring classes: **`ℤ/4` (a nilpotent) and
//! `ℤ/6` (idempotents and zero divisors) are built as kernel inductives**, their addition, negation
//! and multiplication defined as computable `Match` terms, and the kernel then certifies:
//!
//!   - **the atom** `∀x. (1 + (−x)) + x = 1` over `ℤ/4` AND `ℤ/6` — so the completeness engine of
//!     "no finite formula is structureless" is kernel-anchored beyond the fields, exactly where
//!     Gaussian elimination dies;
//!   - **the cube-point seeds** of kill-or-absorb — `0·0 = 0`, `1·1 = 1`, `0·(1−0) = 0`,
//!     `1·(1−1) = 0` — over both rings (on the cube, `x² = x` *is* cube-point idempotence, since a
//!     multilinear polynomial is exactly its function on `{0,1}ⁿ`);
//!   - **that these rings genuinely have zero divisors** — `2·2 = 0` in `ℤ/4` (the nilpotent),
//!     `2·3 = 0` and the idempotents `3·3 = 3`, `4·4 = 4` in `ℤ/6` — kernel-witnessed, so the
//!     completeness theorems above are certified to hold *despite* the failure of field axioms,
//!     not because some hidden field structure survived;
//!   - and a **negative control**: the characteristic-2 law `x + x = 0`, false in both rings, is
//!     rejected by `infer_type` — the kernel is not rubber-stamping ring-shaped statements.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Term, Universe};

// ── kernel term helpers ──────────────────────────────────────────────────────────────────────────
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn kvar(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: param.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn mtch(disc: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match { discriminant: Box::new(disc), motive: Box::new(motive), cases }
}

/// A cyclic ring `ℤ/n` as a kernel inductive: constructors `{prefix}0 .. {prefix}{n−1}`, with
/// `add`/`neg`/`mul` defined by double case analysis (computable `Match` tables).
struct KernelRing {
    ty: &'static str,
    n: usize,
    prefix: &'static str,
}

impl KernelRing {
    fn elem(&self, k: usize) -> Term {
        g(&format!("{}{}", self.prefix, k % self.n))
    }
    fn tyt(&self) -> Term {
        g(self.ty)
    }
    fn op(&self, name: &str) -> String {
        format!("{}_{}", name, self.ty)
    }
    fn add(&self, a: Term, b: Term) -> Term {
        app2(g(&self.op("add")), a, b)
    }
    fn neg(&self, a: Term) -> Term {
        app(g(&self.op("neg")), a)
    }
    fn mul(&self, a: Term, b: Term) -> Term {
        app2(g(&self.op("mul")), a, b)
    }
    fn eq(&self, a: Term, b: Term) -> Term {
        app(app2(g("Eq"), self.tyt(), a), b)
    }
    fn refl(&self, x: Term) -> Term {
        app2(g("refl"), self.tyt(), x)
    }

    /// `match b { j ⇒ elem(f(j)) }` — one inner row of a binary operation table.
    fn row(&self, f: impl Fn(usize) -> usize) -> Term {
        mtch(
            kvar("b"),
            lam("_", self.tyt(), self.tyt()),
            (0..self.n).map(|j| self.elem(f(j))).collect(),
        )
    }

    /// Register the ring: the inductive, its constructors, and computable add/neg/mul.
    fn register(&self, ctx: &mut Context) {
        ctx.add_inductive(self.ty, Term::Sort(Universe::Type(0)));
        for k in 0..self.n {
            ctx.add_constructor(&format!("{}{}", self.prefix, k), self.ty, self.tyt());
        }
        let binop = pi("a", self.tyt(), pi("b", self.tyt(), self.tyt()));
        let table = |f: &dyn Fn(usize, usize) -> usize| -> Term {
            lam(
                "a",
                self.tyt(),
                lam(
                    "b",
                    self.tyt(),
                    mtch(
                        kvar("a"),
                        lam("_", self.tyt(), self.tyt()),
                        (0..self.n).map(|k| self.row(|j| f(k, j))).collect(),
                    ),
                ),
            )
        };
        ctx.add_definition(self.op("add"), binop.clone(), table(&|k, j| (k + j) % self.n));
        ctx.add_definition(self.op("mul"), binop, table(&|k, j| (k * j) % self.n));
        let neg_body = lam(
            "a",
            self.tyt(),
            mtch(
                kvar("a"),
                lam("_", self.tyt(), self.tyt()),
                (0..self.n).map(|k| self.elem((self.n - k) % self.n)).collect(),
            ),
        );
        ctx.add_definition(self.op("neg"), pi("a", self.tyt(), self.tyt()), neg_body);
    }

    /// A universally-quantified law with the statement as the match motive, `refl` per case.
    fn law(
        &self,
        v: &str,
        lhs: impl Fn(Term) -> Term,
        rhs: impl Fn(Term) -> Term,
        case_values: &[usize],
    ) -> (Term, Term) {
        let stmt = pi(v, self.tyt(), self.eq(lhs(kvar(v)), rhs(kvar(v))));
        let motive = lam(v, self.tyt(), self.eq(lhs(kvar(v)), rhs(kvar(v))));
        let cases = case_values.iter().map(|&k| self.refl(self.elem(k))).collect();
        let proof = lam(v, self.tyt(), mtch(kvar(v), motive, cases));
        (stmt, proof)
    }
}

fn proves(ctx: &Context, proof: &Term, law: &Term) -> bool {
    match infer_type(ctx, proof) {
        Ok(ty) => is_subtype(ctx, &ty, law) && is_subtype(ctx, law, &ty),
        Err(_) => false,
    }
}

/// A closed equation `lhs = rhs` certified by `refl rhs` — the kernel must REDUCE `lhs` to `rhs`
/// definitionally for the proof to type-check.
fn closed_fact(ctx: &Context, ring: &KernelRing, lhs: Term, rhs_k: usize) -> bool {
    let rhs = ring.elem(rhs_k);
    proves(ctx, &ring.refl(rhs.clone()), &ring.eq(lhs, rhs))
}

fn z4() -> KernelRing {
    KernelRing { ty: "Z4", n: 4, prefix: "Z4_" }
}
fn z6() -> KernelRing {
    KernelRing { ty: "Z6", n: 6, prefix: "Z6_" }
}

fn seeded_context() -> (Context, KernelRing, KernelRing) {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx); // Eq, refl
    let r4 = z4();
    let r6 = z6();
    r4.register(&mut ctx);
    r6.register(&mut ctx);
    (ctx, r4, r6)
}

#[test]
fn the_pou_atom_and_cube_point_seeds_are_kernel_theorems_over_z4_and_z6() {
    let (ctx, r4, r6) = seeded_context();
    for ring in [&r4, &r6] {
        // The construction itself type-checks.
        assert!(matches!(infer_type(&ctx, &g(&ring.op("add"))), Ok(Term::Pi { .. })));
        assert!(matches!(infer_type(&ctx, &g(&ring.op("mul"))), Ok(Term::Pi { .. })));
        assert!(matches!(infer_type(&ctx, &g(&ring.op("neg"))), Ok(Term::Pi { .. })));

        // THE ATOM: ∀x. (1 + (−x)) + x = 1 — the per-variable engine of the partition of unity,
        // now a kernel theorem over a ring with zero divisors. Every case reduces to 1.
        let one_cases: Vec<usize> = vec![1; ring.n];
        let (atom, p) = ring.law(
            "x",
            |x| ring.add(ring.add(ring.elem(1), ring.neg(x.clone())), x),
            |_| ring.elem(1),
            &one_cases,
        );
        assert!(
            proves(&ctx, &p, &atom),
            "{}: the atom (1 − x) + x = 1 is a kernel theorem — completeness needs no field",
            ring.ty
        );

        // Additive identity and inverses — the only ring facts the signed indicators use.
        let id_cases: Vec<usize> = (0..ring.n).collect();
        let (add_zero, p) = ring.law("a", |a| ring.add(a, ring.elem(0)), |a| a, &id_cases);
        assert!(proves(&ctx, &p, &add_zero), "{}: a + 0 = a", ring.ty);
        let zero_cases: Vec<usize> = vec![0; ring.n];
        let (inv, p) = ring.law("a", |a| ring.add(a.clone(), ring.neg(a)), |_| ring.elem(0), &zero_cases);
        assert!(proves(&ctx, &p, &inv), "{}: a + (−a) = 0", ring.ty);

        // The cube-point seeds of kill-or-absorb: b·b = b and b·(1 − b) = 0 at b ∈ {0, 1} — on the
        // cube a multilinear polynomial IS its function, so these four closed facts are the
        // pointwise content of x² = x and x·(1 − x) = 0.
        assert!(closed_fact(&ctx, ring, ring.mul(ring.elem(0), ring.elem(0)), 0));
        assert!(closed_fact(&ctx, ring, ring.mul(ring.elem(1), ring.elem(1)), 1));
        assert!(closed_fact(
            &ctx,
            ring,
            ring.mul(ring.elem(0), ring.add(ring.elem(1), ring.neg(ring.elem(0)))),
            0
        ));
        assert!(closed_fact(
            &ctx,
            ring,
            ring.mul(ring.elem(1), ring.add(ring.elem(1), ring.neg(ring.elem(1)))),
            0
        ));
    }

    // The rings are GENUINELY degenerate — kernel-witnessed zero divisors and idempotents — so the
    // theorems above are certified to hold despite the failure of the field axioms.
    assert!(closed_fact(&ctx, &r4, r4.mul(r4.elem(2), r4.elem(2)), 0), "ℤ/4: 2·2 = 0 — nilpotent");
    assert!(closed_fact(&ctx, &r6, r6.mul(r6.elem(2), r6.elem(3)), 0), "ℤ/6: 2·3 = 0 — zero divisors");
    assert!(closed_fact(&ctx, &r6, r6.mul(r6.elem(3), r6.elem(3)), 3), "ℤ/6: 3 is idempotent");
    assert!(closed_fact(&ctx, &r6, r6.mul(r6.elem(4), r6.elem(4)), 4), "ℤ/6: 4 is idempotent");
}

#[test]
fn a_false_ring_law_is_rejected_by_the_kernel_over_z4_and_z6() {
    let (ctx, r4, r6) = seeded_context();
    for ring in [&r4, &r6] {
        // The characteristic-2 law ∀x. x + x = 0 — true over GF(2), FALSE here (1 + 1 = 2 ≠ 0).
        // The refl-per-case proof shape cannot type-check at the x = 1 case.
        let zero_cases: Vec<usize> = vec![0; ring.n];
        let (char2, p) = ring.law("a", |a| ring.add(a.clone(), a), |_| ring.elem(0), &zero_cases);
        assert!(
            !proves(&ctx, &p, &char2),
            "{}: the characteristic-2 law must NOT be provable",
            ring.ty
        );
        // And the honest sibling IS provable: ∀x. x + x = 2·x (doubling is multiplication by 2).
        let dbl_cases: Vec<usize> = (0..ring.n).map(|k| (2 * k) % ring.n).collect();
        let (dbl, p) = ring.law(
            "a",
            |a| ring.add(a.clone(), a),
            |a| ring.mul(ring.elem(2), a),
            &dbl_cases,
        );
        assert!(proves(&ctx, &p, &dbl), "{}: x + x = 2x is the true doubling law", ring.ty);
    }
}
