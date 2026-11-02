//! Proof-PRODUCING arithmetic oracle (untrusted search, kernel-checked proof).
//!
//! Given an `Int` equality goal `Eq Int lhs rhs`, [`prove_int_eq`] searches for a
//! genuine kernel proof term and returns it — or `None`. Nothing here is trusted:
//! whatever it returns is re-checked by the kernel's `infer_type`, so a wrong
//! proof is rejected, never believed. This is the Coq-`lia`/`nia` model — the fast
//! search lives outside the trusted base; a bug here can only cause a *failed*
//! proof, never a false one.
//!
//! Trust boundary: closed/literal goals use exact kernel computation and
//! reflexivity. Symbolic ring proofs apply the registered law names. The standard
//! prelude now gives checked proof bodies to add_comm, add_zero, mul_comm,
//! mul_one, mul_zero, mul_assoc, add_assoc, mul_distrib_add and sub_elim.
//! These ring proofs now have empty transitive axiom dependencies.
//! sub_elim states a - b = a + (-1)*b, exposing subtraction to this vocabulary.
//! The oracle itself remains an untrusted proof producer.

use logicaffeine_kernel::{
    int_lit, lit_bigint, normalize, normalize_at, normalize_checked, BigInt, Context, Literal,
    Term, Transparency,
};

fn global(name: &str) -> Term {
    Term::Global(name.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn app3(f: Term, x: Term, y: Term, z: Term) -> Term {
    app(app2(f, x, y), z)
}
fn int() -> Term {
    global("Int")
}

/// `refl Int t`
fn refl(t: Term) -> Term {
    app2(global("refl"), int(), t)
}

/// `Eq_sym Int x y proof` : turns a proof of `Eq Int x y` into `Eq Int y x`.
fn eq_sym(x: Term, y: Term, proof: Term) -> Term {
    app(app(app(app(global("Eq_sym"), int()), x), y), proof)
}

/// Match `op a b` (i.e. `App(App(Global op, a), b)`), returning `(a, b)`.
fn match_bin(t: &Term, op: &str) -> Option<(Term, Term)> {
    if let Term::App(f, b) = t {
        if let Term::App(g, a) = f.as_ref() {
            if let Term::Global(name) = g.as_ref() {
                if name == op {
                    return Some(((**a).clone(), (**b).clone()));
                }
            }
        }
    }
    None
}

/// Definitional equality check: do `a` and `b` share a normal form?
fn conv(ctx: &Context, a: &Term, b: &Term) -> bool {
    normalize(ctx, a) == normalize(ctx, b)
}

/// Prove an `Int` equality `Eq Int lhs rhs`, or return `None`.
///
/// The returned term, when it exists, has type `Eq Int lhs rhs` (the kernel will
/// confirm). `None` means "this oracle found no proof" — never "it is false."
pub fn prove_int_eq(ctx: &Context, lhs: &Term, rhs: &Term) -> Option<Term> {
    if !logicaffeine_kernel::int_inductive::supports_integer_polynomial_operations(ctx) {
        return None;
    }
    // Expose ordinary wrappers without erasing the polynomial vocabulary into
    // constructor matches. These are definitionally equal views of the original
    // goals; the eventual proof is still checked against the original goal.
    let lhs = polynomial_view(ctx, lhs);
    let rhs = polynomial_view(ctx, rhs);
    let (lhs, rhs) = (&lhs, &rhs);

    // Closed arithmetic is a computation problem, regardless of literal width.
    // Reuse checked normalization before invoking symbolic proof search;
    // exhaustion declines rather than treating a partial result as a value.
    if ground_integer_expression(lhs) && ground_integer_expression(rhs) {
        let left_value = normalize_checked(ctx, lhs).ok()?;
        let right_value = normalize_checked(ctx, rhs).ok()?;
        return match (&left_value, &right_value) {
            (
                Term::Lit(Literal::Int(_) | Literal::BigInt(_)),
                Term::Lit(Literal::Int(_) | Literal::BigInt(_)),
            ) if left_value == right_value => Some(refl(left_value)),
            _ => None,
        };
    }

    // Complete negative decision: two terms are a *formal* ring identity iff they
    // have the same canonical polynomial. If they differ, no proof exists — bail
    // fast (and never waste the search on a non-identity).
    let mut polys = Polynomials { atoms: Vec::new() };
    let pl = to_poly(&mut polys, ctx, lhs);
    let pr = to_poly(&mut polys, ctx, rhs);
    if pl != pr {
        return None;
    }

    // Positive proof: bounded rewrite search first…
    if let Some(p) = prove_eq(ctx, lhs, rhs, MAX_REWRITE_DEPTH) {
        return Some(p);
    }
    // …then the proof-producing normalizer (handles coefficient collection / FOIL
    // that the bounded search can't reach). Additive & sound: returns None if it
    // can't build a proof, and every proof it returns is kernel-checked.
    prove_by_normalization(ctx, lhs, rhs)
}

// =============================================================================
// Polynomial decision layer — canonical multivariate polynomials over opaque
// atoms (any non-add/mul/sub/literal subterm). Used for the fast, complete
// negative decision and as the target of the proof-producing normalizer.
// =============================================================================

/// Atom interner: distinct non-arithmetic subterms get stable ids.
struct Polynomials {
    atoms: Vec<Term>,
}
impl Polynomials {
    fn atom_id(&mut self, t: &Term) -> usize {
        if let Some(i) = self.atoms.iter().position(|a| a == t) {
            i
        } else {
            self.atoms.push(t.clone());
            self.atoms.len() - 1
        }
    }
}

/// A monomial: a sorted multiset of atom ids (`[]` = the constant monomial).
type Mono = Vec<usize>;
/// A polynomial: monomials with nonzero coefficients, sorted, like terms combined.
type Poly = Vec<(Mono, BigInt)>;

fn poly_canon(mut terms: Poly) -> Poly {
    for (m, _) in terms.iter_mut() {
        m.sort_unstable();
    }
    terms.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out: Poly = Vec::new();
    for (m, c) in terms {
        if c.is_zero() {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.0 == m {
                last.1 = last.1.add(&c);
                if last.1.is_zero() {
                    out.pop();
                }
                continue;
            }
        }
        out.push((m, c));
    }
    out
}

fn poly_add(a: &Poly, b: &Poly) -> Poly {
    let mut t = a.clone();
    t.extend(b.iter().cloned());
    poly_canon(t)
}
fn poly_mul(a: &Poly, b: &Poly) -> Poly {
    let mut t = Vec::new();
    for (m1, c1) in a {
        for (m2, c2) in b {
            let mut m = m1.clone();
            m.extend(m2.iter().cloned());
            t.push((m, c1.mul(c2)));
        }
    }
    poly_canon(t)
}
fn poly_scale(k: i64, a: &Poly) -> Poly {
    let k = BigInt::from_i64(k);
    poly_canon(a.iter().map(|(m, c)| (m.clone(), c.mul(&k))).collect())
}

/// Recognize the closed literal fragment without unfolding symbolic operations.
/// An explicit work stack keeps this grammar check independent of native depth.
fn ground_integer_expression(term: &Term) -> bool {
    let mut pending = vec![term];
    while let Some(term) = pending.pop() {
        match term {
            Term::Lit(Literal::Int(_) | Literal::BigInt(_)) => {}
            Term::App(function, right) => {
                let Term::App(head, left) = function.as_ref() else {
                    return false;
                };
                let Term::Global(name) = head.as_ref() else {
                    return false;
                };
                if !matches!(name.as_str(), "add" | "mul" | "sub") {
                    return false;
                }
                pending.push(left);
                pending.push(right);
            }
            _ => return false,
        }
    }
    true
}

/// The ring oracle owns this local transparency choice. The trusted kernel
/// retains full unfolding when checking the resulting proof.
fn polynomial_view(ctx: &Context, term: &Term) -> Term {
    normalize_at(
        ctx,
        term,
        Transparency::Excluding(&|name| matches!(name, "add" | "mul" | "sub")),
    )
}

/// Compute the canonical polynomial of an arithmetic term.
fn to_poly(p: &mut Polynomials, ctx: &Context, t: &Term) -> Poly {
    let t = polynomial_view(ctx, t);
    if let Term::Lit(literal) = &t {
        if let Some(n) = lit_bigint(literal) {
            return if n.is_zero() {
                vec![]
            } else {
                vec![(vec![], n)]
            };
        }
    }
    if let Some((a, b)) = match_bin(&t, "add") {
        return poly_add(&to_poly(p, ctx, &a), &to_poly(p, ctx, &b));
    }
    if let Some((a, b)) = match_bin(&t, "mul") {
        return poly_mul(&to_poly(p, ctx, &a), &to_poly(p, ctx, &b));
    }
    if let Some((a, b)) = match_bin(&t, "sub") {
        return poly_add(&to_poly(p, ctx, &a), &poly_scale(-1, &to_poly(p, ctx, &b)));
    }
    let id = p.atom_id(&t);
    vec![(vec![id], BigInt::from_i64(1))]
}

// =============================================================================
// Proof-producing canonical normalizer.
//
// `norm(t)` returns `(c, proof : Eq Int t c)` where `c = reify(to_poly t)` is the
// deterministic canonical term. Because the negative guard already established
// `to_poly(lhs) == to_poly(rhs)`, the two canonical terms are identical, so the
// goal follows by transitivity. Every proof is built from the ring axioms and is
// re-checked by the kernel — the normalizer is untrusted.
// =============================================================================

fn lit_t(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}

/// Emit one canonical literal for an exact coefficient, demoting small results.
fn coefficient_term(n: &BigInt) -> Term {
    Term::Lit(int_lit(n.clone()))
}
fn ax1(name: &str, a: Term) -> Term {
    app(global(name), a)
}
fn ax2(name: &str, a: Term, b: Term) -> Term {
    app2(global(name), a, b)
}
fn ax3(name: &str, a: Term, b: Term, c: Term) -> Term {
    app3(global(name), a, b, c)
}

/// The product term for a monomial (left-assoc), or `None` for the empty monomial.
fn mono_to_term(mono: &[usize], atoms: &[Term]) -> Option<Term> {
    let mut iter = mono.iter();
    let first = *iter.next()?;
    let mut t = atoms[first].clone();
    for &id in iter {
        t = ax2("mul", t, atoms[id].clone());
    }
    Some(t)
}
/// The canonical term for one `(monomial, coeff)`.
fn scaled_term(mono: &[usize], coeff: &BigInt, atoms: &[Term]) -> Term {
    match mono_to_term(mono, atoms) {
        None => coefficient_term(coeff),
        Some(m) if coeff.to_i64() == Some(1) => m,
        Some(m) => ax2("mul", coefficient_term(coeff), m),
    }
}
/// The canonical term for a whole polynomial (left-assoc sum, sorted).
fn reify(poly: &[(Mono, BigInt)], atoms: &[Term]) -> Term {
    // Drop zero-coefficient monomials so the canonical form is unique (a cancelled
    // term must not linger as `add 0 …` / `mul 0 …`, which would make two equal
    // polynomials reify to syntactically different terms).
    let mut iter = poly.iter().filter(|(_, c)| !c.is_zero());
    let Some((m0, c0)) = iter.next() else {
        return lit_t(0);
    };
    let mut t = scaled_term(m0, c0, atoms);
    for (m, c) in iter {
        t = ax2("add", t, scaled_term(m, c, atoms));
    }
    t
}

/// Proof `term = mul (lit c) M`, where `term = scaled_term(m, c)` and `M = mono_to_term(m)`.
/// For `c == 1` the term is the bare monomial `M`, coerced via `mul_one`/`mul_comm`.
fn as_scaled_mul(c: &BigInt, m_term: &Term) -> Term {
    if c.to_i64() == Some(1) {
        // M = mul 1 M  via  sym (mul 1 M = mul M 1 = M)
        let mul1m = ax2("mul", lit_t(1), m_term.clone());
        let chain = eq_trans(
            mul1m.clone(),
            ax2("mul", m_term.clone(), lit_t(1)),
            m_term.clone(),
            ax2("mul_comm", lit_t(1), m_term.clone()),
            ax1("mul_one", m_term.clone()),
        );
        eq_sym(mul1m, m_term.clone(), chain)
    } else {
        refl(ax2("mul", coefficient_term(c), m_term.clone()))
    }
}

/// Proof `add (mul c1 M) (mul c2 M) = mul (add c1 c2) M`  (right reverse-distribution).
fn rev_distrib(c1: &BigInt, c2: &BigInt, m_term: &Term) -> Term {
    let big_c = ax2("add", coefficient_term(c1), coefficient_term(c2));
    let mul_c1 = ax2("mul", coefficient_term(c1), m_term.clone());
    let mul_c2 = ax2("mul", coefficient_term(c2), m_term.clone());
    // mul C M = mul M C
    let s1 = ax2("mul_comm", big_c.clone(), m_term.clone());
    // mul M C = add (mul M c1) (mul M c2)
    let s2 = ax3(
        "mul_distrib_add",
        m_term.clone(),
        coefficient_term(c1),
        coefficient_term(c2),
    );
    // add (mul M c1)(mul M c2) = add (mul c1 M)(mul c2 M)
    let s3 = cong2(
        "add",
        &ax2("mul", m_term.clone(), coefficient_term(c1)),
        &mul_c1,
        &ax2("mul", m_term.clone(), coefficient_term(c2)),
        &mul_c2,
        ax2("mul_comm", m_term.clone(), coefficient_term(c1)),
        ax2("mul_comm", m_term.clone(), coefficient_term(c2)),
    );
    // mul C M = add (mul c1 M)(mul c2 M)
    let forward = eq_trans(
        ax2("mul", big_c.clone(), m_term.clone()),
        ax2("mul", m_term.clone(), big_c.clone()),
        ax2("add", mul_c1.clone(), mul_c2.clone()),
        s1,
        eq_trans(
            ax2("mul", m_term.clone(), big_c.clone()),
            ax2(
                "add",
                ax2("mul", m_term.clone(), coefficient_term(c1)),
                ax2("mul", m_term.clone(), coefficient_term(c2)),
            ),
            ax2("add", mul_c1.clone(), mul_c2.clone()),
            s2,
            s3,
        ),
    );
    eq_sym(
        ax2("mul", big_c, m_term.clone()),
        ax2("add", mul_c1, mul_c2),
        forward,
    )
}

/// Proof `add (scaled m c1) (scaled m c2) = scaled m (c1+c2)` for the SAME monomial `m`.
fn combine_coeff(m: &[usize], c1: &BigInt, c2: &BigInt, atoms: &[Term]) -> Option<Term> {
    let t1 = scaled_term(m, c1, atoms);
    let t2 = scaled_term(m, c2, atoms);
    let sum = c1.add(c2);
    let result = scaled_term(m, &sum, atoms);
    let _ = (&t1, &t2, &result);
    match mono_to_term(m, atoms) {
        // constant terms: `add (lit c1)(lit c2)` ≡ `lit (c1+c2)` by computation.
        None => Some(refl(coefficient_term(&sum))),
        Some(m_term) => {
            // add t1 t2 = add (mul c1 M)(mul c2 M)  [coerce]  = mul (c1+c2) M  [rev_distrib]
            let coerce = cong2(
                "add",
                &t1,
                &ax2("mul", coefficient_term(c1), m_term.clone()),
                &t2,
                &ax2("mul", coefficient_term(c2), m_term.clone()),
                as_scaled_mul(c1, &m_term),
                as_scaled_mul(c2, &m_term),
            );
            let rd = rev_distrib(c1, c2, &m_term);
            let coerced = ax2(
                "add",
                ax2("mul", coefficient_term(c1), m_term.clone()),
                ax2("mul", coefficient_term(c2), m_term.clone()),
            );
            if sum.to_i64() != Some(1) {
                Some(eq_trans(ax2("add", t1, t2), coerced, result, coerce, rd))
            } else {
                // c1+c2 == 1: the result is the bare monomial, so extend the chain
                // past `mul 1 M` (what rev_distrib's RHS reduces to) with
                // `mul 1 M = mul M 1 = M`.
                let mul1m = ax2("mul", lit_t(1), m_term.clone());
                let to_bare = eq_trans(
                    mul1m.clone(),
                    ax2("mul", m_term.clone(), lit_t(1)),
                    m_term.clone(),
                    ax2("mul_comm", lit_t(1), m_term.clone()),
                    ax1("mul_one", m_term.clone()),
                );
                let inner = eq_trans(coerced.clone(), mul1m, result.clone(), rd, to_bare);
                Some(eq_trans(ax2("add", t1, t2), coerced, result, coerce, inner))
            }
        }
    }
}

/// Proof `add (add X Y) Z = add (add X Z) Y` (move `Z` past `Y`).
fn swap_top(x: Term, y: Term, z: Term) -> Term {
    // add(add X Y)Z = add X (add Y Z) = add X (add Z Y) = add(add X Z)Y
    let s1 = ax3("add_assoc", x.clone(), y.clone(), z.clone());
    let s2 = cong2(
        "add",
        &x,
        &x,
        &ax2("add", y.clone(), z.clone()),
        &ax2("add", z.clone(), y.clone()),
        refl(x.clone()),
        ax2("add_comm", y.clone(), z.clone()),
    );
    let s3 = eq_sym(
        ax2("add", ax2("add", x.clone(), z.clone()), y.clone()),
        ax2("add", x.clone(), ax2("add", z.clone(), y.clone())),
        ax3("add_assoc", x.clone(), z.clone(), y.clone()),
    );
    eq_trans(
        ax2("add", ax2("add", x.clone(), y.clone()), z.clone()),
        ax2("add", x.clone(), ax2("add", y.clone(), z.clone())),
        ax2("add", ax2("add", x.clone(), z.clone()), y.clone()),
        s1,
        eq_trans(
            ax2("add", x.clone(), ax2("add", y.clone(), z.clone())),
            ax2("add", x.clone(), ax2("add", z.clone(), y.clone())),
            ax2("add", ax2("add", x.clone(), z.clone()), y),
            s2,
            s3,
        ),
    )
}

/// Insert one `(mono, coeff)` term into a canonical poly `p`, returning
/// `(result_poly, proof : add (reify p) (scaled term) = reify(result))`.
fn merge_term(
    atoms: &[Term],
    p: &[(Mono, BigInt)],
    m: &[usize],
    c: &BigInt,
) -> Option<(Poly, Term)> {
    let st = scaled_term(m, c, atoms);
    if p.is_empty() {
        // add (lit 0) st = add st 0 = st
        let proof = eq_trans(
            ax2("add", lit_t(0), st.clone()),
            ax2("add", st.clone(), lit_t(0)),
            st.clone(),
            ax2("add_comm", lit_t(0), st.clone()),
            ax1("add_zero", st.clone()),
        );
        return Some((vec![(m.to_vec(), c.clone())], proof));
    }
    let (ml, cl) = p.last().unwrap().clone();
    let init = &p[..p.len() - 1];
    let last_t = scaled_term(&ml, &cl, atoms);
    let reify_p = reify(p, atoms);

    use std::cmp::Ordering;
    match m.to_vec().cmp(&ml) {
        Ordering::Greater => {
            // already sorted: structurally reify(p ++ [term])
            let mut res = p.to_vec();
            res.push((m.to_vec(), c.clone()));
            Some((res, refl(ax2("add", reify_p, st))))
        }
        Ordering::Equal => {
            if cl.add(c).is_zero() {
                // The monomial cancels (`cl·M + c·M = 0`), so it drops from the
                // canonical form. `combine_coeff` gives `add last_t st = mul 0 M`;
                // chain `mul 0 M = mul M 0 (mul_comm) = 0 (mul_zero)`.
                let cc = combine_coeff(&ml, &cl, c, atoms)?;
                let cancel = match mono_to_term(&ml, atoms) {
                    None => cc, // constant monomial: `scaled(ml, 0)` is already `lit 0`
                    Some(m_term) => eq_trans(
                        ax2("add", last_t.clone(), st.clone()),
                        ax2("mul", lit_t(0), m_term.clone()),
                        lit_t(0),
                        cc,
                        eq_trans(
                            ax2("mul", lit_t(0), m_term.clone()),
                            ax2("mul", m_term.clone(), lit_t(0)),
                            lit_t(0),
                            ax2("mul_comm", lit_t(0), m_term.clone()),
                            ax1("mul_zero", m_term),
                        ),
                    ),
                };
                // cancel : add last_t st = 0
                if init.is_empty() {
                    return Some((vec![], cancel));
                }
                let ri = reify(init, atoms);
                let assoc = ax3("add_assoc", ri.clone(), last_t.clone(), st.clone());
                let cong = cong2(
                    "add",
                    &ri,
                    &ri,
                    &ax2("add", last_t.clone(), st.clone()),
                    &lit_t(0),
                    refl(ri.clone()),
                    cancel,
                );
                let azero = ax1("add_zero", ri.clone());
                let proof = eq_trans(
                    ax2("add", ax2("add", ri.clone(), last_t.clone()), st.clone()),
                    ax2("add", ri.clone(), ax2("add", last_t.clone(), st.clone())),
                    ri.clone(),
                    assoc,
                    eq_trans(
                        ax2("add", ri.clone(), ax2("add", last_t.clone(), st.clone())),
                        ax2("add", ri.clone(), lit_t(0)),
                        ri.clone(),
                        cong,
                        azero,
                    ),
                );
                return Some((init.to_vec(), proof));
            }
            let cc = combine_coeff(&ml, &cl, c, atoms)?; // add last_t st = scaled(ml, cl+c)
            let combined = scaled_term(&ml, &cl.add(c), atoms);
            if init.is_empty() {
                Some((vec![(ml, cl.add(c))], cc))
            } else {
                let ri = reify(init, atoms);
                let assoc = ax3("add_assoc", ri.clone(), last_t.clone(), st.clone());
                let cong = cong2(
                    "add",
                    &ri,
                    &ri,
                    &ax2("add", last_t.clone(), st.clone()),
                    &combined,
                    refl(ri.clone()),
                    cc,
                );
                let proof = eq_trans(
                    ax2("add", ax2("add", ri.clone(), last_t), st),
                    ax2(
                        "add",
                        ri.clone(),
                        ax2(
                            "add",
                            scaled_term(&ml, &cl, atoms),
                            scaled_term(m, c, atoms),
                        ),
                    ),
                    ax2("add", ri.clone(), combined),
                    assoc,
                    cong,
                );
                let mut res = init.to_vec();
                res.push((ml, cl.add(c)));
                Some((res, proof))
            }
        }
        Ordering::Less => {
            if init.is_empty() {
                // add last_t st = add st last_t
                let mut res = vec![(m.to_vec(), c.clone())];
                res.push((ml, cl));
                Some((res, ax2("add_comm", last_t, st)))
            } else {
                let ri = reify(init, atoms);
                let swap = swap_top(ri.clone(), last_t.clone(), st.clone());
                let (init2, inner) = merge_term(atoms, init, m, c)?; // add ri st = reify(init2)
                let ri2 = reify(&init2, atoms);
                let cong = cong2(
                    "add",
                    &ax2("add", ri.clone(), st.clone()),
                    &ri2,
                    &last_t,
                    &last_t,
                    inner,
                    refl(last_t.clone()),
                );
                let proof = eq_trans(
                    ax2("add", ax2("add", ri.clone(), last_t.clone()), st.clone()),
                    ax2("add", ax2("add", ri, st.clone()), last_t.clone()),
                    ax2("add", ri2.clone(), last_t.clone()),
                    swap,
                    cong,
                );
                // If the merge cancelled all of `init`, the result reifies to the
                // bare `last_t` — eliminate the `add 0 last_t` residue so the
                // proof's conclusion IS the canonical form.
                let proof = if init2.is_empty() {
                    let zfix = eq_trans(
                        ax2("add", lit_t(0), last_t.clone()),
                        ax2("add", last_t.clone(), lit_t(0)),
                        last_t.clone(),
                        ax2("add_comm", lit_t(0), last_t.clone()),
                        ax1("add_zero", last_t.clone()),
                    );
                    eq_trans(
                        ax2(
                            "add",
                            ax2("add", reify(init, atoms), last_t.clone()),
                            st.clone(),
                        ),
                        ax2("add", ri2, last_t.clone()),
                        last_t.clone(),
                        proof,
                        zfix,
                    )
                } else {
                    proof
                };
                let mut res = init2;
                res.push((ml, cl));
                Some((res, proof))
            }
        }
    }
}

/// Merge two canonical polynomials, returning
/// `(merged, proof : add (reify pa)(reify pb) = reify(merged))`.
fn merge_canonical(
    atoms: &[Term],
    pa: &[(Mono, BigInt)],
    pb: &[(Mono, BigInt)],
) -> Option<(Poly, Term)> {
    let ra = reify(pa, atoms);
    if pb.is_empty() {
        // add (reify pa) 0 = reify pa
        return Some((pa.to_vec(), ax1("add_zero", ra)));
    }
    if pb.len() == 1 {
        let (m, c) = &pb[0];
        return merge_term(atoms, pa, m, c);
    }
    let (ml, cl) = pb.last().unwrap().clone();
    let pb_init = &pb[..pb.len() - 1];
    let rbi = reify(pb_init, atoms);
    let slast = scaled_term(&ml, &cl, atoms);
    // add ra (add rbi slast) = add (add ra rbi) slast
    let assoc_sym = eq_sym(
        ax2("add", ax2("add", ra.clone(), rbi.clone()), slast.clone()),
        ax2("add", ra.clone(), ax2("add", rbi.clone(), slast.clone())),
        ax3("add_assoc", ra.clone(), rbi.clone(), slast.clone()),
    );
    let (m1, p1) = merge_canonical(atoms, pa, pb_init)?; // add ra rbi = reify(m1)
    let rm1 = reify(&m1, atoms);
    let cong = cong2(
        "add",
        &ax2("add", ra.clone(), rbi.clone()),
        &rm1,
        &slast,
        &slast,
        p1,
        refl(slast.clone()),
    );
    let (m2, p2) = merge_term(atoms, &m1, &ml, &cl)?; // add rm1 slast = reify(m2)
    let rm2 = reify(&m2, atoms);
    let proof = eq_trans(
        ax2("add", ra.clone(), ax2("add", rbi.clone(), slast.clone())),
        ax2("add", ax2("add", ra, rbi), slast.clone()),
        rm2,
        assoc_sym,
        eq_trans(
            ax2(
                "add",
                ax2("add", reify(pa, atoms), reify(pb_init, atoms)),
                slast.clone(),
            ),
            ax2("add", rm1, slast),
            reify(&m2, atoms),
            cong,
            p2,
        ),
    );
    Some((m2, proof))
}

/// The id of an ALREADY-interned atom (non-inserting); `None` for a literal / non-atom, which the
/// constructive monomial sort declines (the caller falls back to the bounded search).
fn atom_index(polys: &Polynomials, t: &Term) -> Option<usize> {
    polys.atoms.iter().position(|a| a == t)
}

/// Swap the last two factors of a left-assoc product: proof of `mul(mul(A,b),x) = mul(mul(A,x),b)`,
/// built from `mul_assoc` (twice) and one `mul_comm` under a congruence.
fn swap_last(a: &Term, b: &Term, x: &Term) -> Term {
    let m_ab_x = ax2("mul", ax2("mul", a.clone(), b.clone()), x.clone());
    let m_a_bx = ax2("mul", a.clone(), ax2("mul", b.clone(), x.clone()));
    let m_a_xb = ax2("mul", a.clone(), ax2("mul", x.clone(), b.clone()));
    let m_ax_b = ax2("mul", ax2("mul", a.clone(), x.clone()), b.clone());
    // mul(mul(A,b),x) = mul(A, mul(b,x))
    let s1 = ax3("mul_assoc", a.clone(), b.clone(), x.clone());
    // mul(A, mul(b,x)) = mul(A, mul(x,b))    (mul_comm under A)
    let s2 = cong2(
        "mul",
        a,
        a,
        &ax2("mul", b.clone(), x.clone()),
        &ax2("mul", x.clone(), b.clone()),
        refl(a.clone()),
        ax2("mul_comm", b.clone(), x.clone()),
    );
    // mul(A, mul(x,b)) = mul(mul(A,x), b)
    let s3 = eq_sym(
        m_ax_b.clone(),
        m_a_xb.clone(),
        ax3("mul_assoc", a.clone(), x.clone(), b.clone()),
    );
    let inner = eq_trans(m_a_bx.clone(), m_a_xb, m_ax_b.clone(), s2, s3); // mul(A,mul(b,x)) = mul(mul(A,x),b)
    eq_trans(m_ab_x, m_a_bx, m_ax_b, s1, inner)
}

/// Insert atom `x` into the sorted product `p`. Returns `(result, proof : mul(p,x) = result)` where
/// `result` is `p` with `x` in its sorted position. `None` if a factor is not an interned atom.
fn insert_atom(polys: &Polynomials, p: &Term, x: &Term) -> Option<(Term, Term)> {
    let xid = atom_index(polys, x)?;
    let prod = ax2("mul", p.clone(), x.clone());
    match match_bin(p, "mul") {
        None => {
            let pid = atom_index(polys, p)?;
            if pid <= xid {
                Some((prod.clone(), refl(prod)))
            } else {
                Some((
                    ax2("mul", x.clone(), p.clone()),
                    ax2("mul_comm", p.clone(), x.clone()),
                ))
            }
        }
        Some((pinit, plast)) => {
            let lastid = atom_index(polys, &plast)?;
            if lastid <= xid {
                Some((prod.clone(), refl(prod)))
            } else {
                // swap x past plast, then insert into pinit
                let swap = swap_last(&pinit, &plast, x); // mul(mul(pinit,plast),x) = mul(mul(pinit,x),plast)
                let (inner, pin) = insert_atom(polys, &pinit, x)?; // mul(pinit,x) = inner
                let cong = cong2(
                    "mul",
                    &ax2("mul", pinit.clone(), x.clone()),
                    &inner,
                    &plast,
                    &plast,
                    pin,
                    refl(plast.clone()),
                );
                let result = ax2("mul", inner, plast.clone());
                let proof = eq_trans(
                    prod,
                    ax2("mul", ax2("mul", pinit.clone(), x.clone()), plast.clone()),
                    result.clone(),
                    swap,
                    cong,
                );
                Some((result, proof))
            }
        }
    }
}

/// Merge two sorted atom-products `ca`, `cb` into one sorted product.
/// Returns `(result, proof : mul(ca,cb) = result)`, or `None` (any non-atom factor).
fn merge_sorted(polys: &Polynomials, ca: &Term, cb: &Term) -> Option<(Term, Term)> {
    match match_bin(cb, "mul") {
        None => insert_atom(polys, ca, cb),
        Some((cbinit, cblast)) => {
            // mul(ca, mul(cbinit,cblast)) = mul(mul(ca,cbinit), cblast)   (eq_sym mul_assoc)
            let assoc_rev = eq_sym(
                ax2(
                    "mul",
                    ax2("mul", ca.clone(), cbinit.clone()),
                    cblast.clone(),
                ),
                ax2(
                    "mul",
                    ca.clone(),
                    ax2("mul", cbinit.clone(), cblast.clone()),
                ),
                ax3("mul_assoc", ca.clone(), cbinit.clone(), cblast.clone()),
            );
            let (merged1, p1) = merge_sorted(polys, ca, &cbinit)?; // mul(ca,cbinit) = merged1
            let cong = cong2(
                "mul",
                &ax2("mul", ca.clone(), cbinit.clone()),
                &merged1,
                &cblast,
                &cblast,
                p1,
                refl(cblast.clone()),
            );
            let (merged2, p2) = insert_atom(polys, &merged1, &cblast)?; // mul(merged1,cblast) = merged2
            let start = ax2("mul", ca.clone(), cb.clone());
            let mid = ax2(
                "mul",
                ax2("mul", ca.clone(), cbinit.clone()),
                cblast.clone(),
            );
            let proof = eq_trans(
                start,
                mid.clone(),
                merged2.clone(),
                assoc_rev,
                eq_trans(
                    mid,
                    ax2("mul", merged1.clone(), cblast.clone()),
                    merged2.clone(),
                    cong,
                    p2,
                ),
            );
            Some((merged2, proof))
        }
    }
}

/// Split a canonical monomial into `(coeff, atom_product)`: `lit(n)` → `(n, None)`;
/// `mul(lit n, M)` → `(n, Some(M))`; a bare atom product → `(1, Some(t))`. The coefficient of a
/// `scaled_term` always sits at the outermost-left, so this is exact.
fn split_coeff(t: &Term) -> (BigInt, Option<Term>) {
    if let Term::Lit(literal) = t {
        if let Some(n) = lit_bigint(literal) {
            return (n, None);
        }
    }
    if let Some((a, b)) = match_bin(t, "mul") {
        if let Term::Lit(literal) = a {
            if let Some(n) = lit_bigint(&literal) {
                return (n, Some(b));
            }
        }
    }
    (BigInt::from_i64(1), Some(t.clone()))
}

/// Swap the two middle factors of a product-of-products: proof of
/// `mul(mul(P,Q), mul(R,S)) = mul(mul(P,R), mul(Q,S))`, a fixed 5-step assoc/comm chain
/// (independent of the degree of P,Q,R,S). Used to hoist coefficients out of a monomial product.
fn swap_middle(p: &Term, q: &Term, r: &Term, s: &Term) -> Term {
    let rs = ax2("mul", r.clone(), s.clone());
    let qs = ax2("mul", q.clone(), s.clone());
    let qr = ax2("mul", q.clone(), r.clone());
    let rq = ax2("mul", r.clone(), q.clone());
    let w0 = ax2("mul", ax2("mul", p.clone(), q.clone()), rs.clone());
    let w1 = ax2("mul", p.clone(), ax2("mul", q.clone(), rs.clone()));
    let w2 = ax2("mul", p.clone(), ax2("mul", qr.clone(), s.clone()));
    let w3 = ax2("mul", p.clone(), ax2("mul", rq.clone(), s.clone()));
    let w4 = ax2("mul", p.clone(), ax2("mul", r.clone(), qs.clone()));
    let w5 = ax2("mul", ax2("mul", p.clone(), r.clone()), qs.clone());
    // w0 = w1 : mul_assoc P Q (mul R S)
    let p01 = ax3("mul_assoc", p.clone(), q.clone(), rs.clone());
    // w1 = w2 : under P, sym(mul_assoc Q R S) turns mul(Q, mul(R,S)) into mul(mul(Q,R), S)
    let p12 = cong2(
        "mul",
        p,
        p,
        &ax2("mul", q.clone(), rs.clone()),
        &ax2("mul", qr.clone(), s.clone()),
        refl(p.clone()),
        eq_sym(
            ax2("mul", qr.clone(), s.clone()),
            ax2("mul", q.clone(), rs.clone()),
            ax3("mul_assoc", q.clone(), r.clone(), s.clone()),
        ),
    );
    // w2 = w3 : under P·(_·S), mul_comm Q R
    let p23 = cong2(
        "mul",
        p,
        p,
        &ax2("mul", qr.clone(), s.clone()),
        &ax2("mul", rq.clone(), s.clone()),
        refl(p.clone()),
        cong2(
            "mul",
            &qr,
            &rq,
            s,
            s,
            ax2("mul_comm", q.clone(), r.clone()),
            refl(s.clone()),
        ),
    );
    // w3 = w4 : under P, mul_assoc R Q S
    let p34 = cong2(
        "mul",
        p,
        p,
        &ax2("mul", rq.clone(), s.clone()),
        &ax2("mul", r.clone(), qs.clone()),
        refl(p.clone()),
        ax3("mul_assoc", r.clone(), q.clone(), s.clone()),
    );
    // w4 = w5 : sym(mul_assoc P R (mul Q S))
    let p45 = eq_sym(
        w5.clone(),
        w4.clone(),
        ax3("mul_assoc", p.clone(), r.clone(), qs.clone()),
    );
    eq_trans(
        w0,
        w1.clone(),
        w5.clone(),
        p01,
        eq_trans(
            w1,
            w2.clone(),
            w5.clone(),
            p12,
            eq_trans(
                w2,
                w3.clone(),
                w5.clone(),
                p23,
                eq_trans(w3, w4, w5, p34, p45),
            ),
        ),
    )
}

/// Constructive coefficient-aware monomial product: `mul(ca,cb) = scaled_term(sort(atoms), c1·c2)`.
/// Handles two signed monomials (`±·atoms`) — the case `merge_sorted` declines because of the literal
/// coefficient. Hoists both coefficients out with one `swap_middle`, folds them by computation, sorts
/// the atom parts with the (fast, any-degree) `merge_sorted`, and drops a `·1` coefficient. Returns
/// `None` unless BOTH factors carry an atom product (the pure-literal / already-sorted cases the caller
/// discharges by `refl`). No search: the proof is built directly, so it is fast at any degree.
fn merge_scaled(polys: &Polynomials, ca: &Term, cb: &Term) -> Option<(Term, Term)> {
    let (c1, oa) = split_coeff(ca);
    let (c2, ob) = split_coeff(cb);
    let a = oa?;
    let b = ob?;
    let coeff = c1.mul(&c2);
    let (merged, p_atoms) = merge_sorted(polys, &a, &b)?; // mul(a,b) = merged   (pure atoms, fast)
    let (lc1, lc2, lcoeff) = (
        coefficient_term(&c1),
        coefficient_term(&c2),
        coefficient_term(&coeff),
    );
    let ca_u = ax2("mul", lc1.clone(), a.clone());
    let cb_u = ax2("mul", lc2.clone(), b.clone());
    let cvt_a = if c1.to_i64() == Some(1) {
        as_scaled_mul(&c1, &a)
    } else {
        refl(ca_u.clone())
    };
    let cvt_b = if c2.to_i64() == Some(1) {
        as_scaled_mul(&c2, &b)
    } else {
        refl(cb_u.clone())
    };
    let step_cvt = cong2("mul", ca, &ca_u, cb, &cb_u, cvt_a, cvt_b); // mul(ca,cb) = mul(ca_u,cb_u)
    let step_swap = swap_middle(&lc1, &a, &lc2, &b); // = mul(mul(lc1,lc2), mul(a,b))
    let ab = ax2("mul", a.clone(), b.clone());
    let coeff_redex = ax2("mul", lc1.clone(), lc2.clone());
    // fold coefficient (mul(lc1,lc2) = lcoeff by computation) and atoms (p_atoms) together
    let step_fold = cong2(
        "mul",
        &coeff_redex,
        &lcoeff,
        &ab,
        &merged,
        refl(lcoeff.clone()),
        p_atoms,
    ); // mul(mul(lc1,lc2), mul(a,b)) = mul(lcoeff, merged)
    let prod = ax2("mul", ca.clone(), cb.clone());
    let mid1 = ax2("mul", ca_u, cb_u);
    let mid2 = ax2("mul", coeff_redex, ab);
    let scaled_lit = ax2("mul", lcoeff, merged.clone());
    let up_to_lit = eq_trans(
        prod.clone(),
        mid1.clone(),
        scaled_lit.clone(),
        step_cvt,
        eq_trans(mid1, mid2, scaled_lit.clone(), step_swap, step_fold),
    ); // mul(ca,cb) = mul(lcoeff, merged)
    if coeff.to_i64() == Some(1) {
        // canonical form of a coeff-1 monomial is the bare product: strip the `·1`.
        let elim = eq_sym(
            merged.clone(),
            scaled_lit.clone(),
            as_scaled_mul(&coeff, &merged),
        );
        Some((
            merged.clone(),
            eq_trans(prod, scaled_lit, merged, up_to_lit, elim),
        ))
    } else {
        Some((scaled_lit, up_to_lit))
    }
}

/// Is `t` the literal integer `v`?
fn is_lit(t: &Term, v: i64) -> bool {
    matches!(t, Term::Lit(logicaffeine_kernel::Literal::Int(n)) if *n == v)
}

/// Distribute `mul ca cb` (canonical terms), returning
/// `(product_poly, proof : mul ca cb = reify(product))`.
fn dist_mul(ctx: &Context, polys: &mut Polynomials, ca: &Term, cb: &Term) -> Option<(Poly, Term)> {
    // Literal annihilator / identity factors (e.g. an identity matrix's 0/1 entries in `det·I`):
    // collapse them directly. Distributing a literal 0 or 1 across a sum instead would strand a
    // `mul(0, monomial)` / `mul(1, monomial)` that the bounded search can't close (`mul_zero`/`mul_one`
    // are right-sided, so the left forms need a `mul_comm` flip the search doesn't chain to).
    if is_lit(cb, 0) {
        return Some((vec![], ax1("mul_zero", ca.clone()))); // mul(ca, 0) = 0
    }
    if is_lit(ca, 0) {
        let prod = ax2("mul", ca.clone(), cb.clone());
        let flip = ax2("mul_comm", ca.clone(), cb.clone()); // mul(0, cb) = mul(cb, 0)
        let zero = ax1("mul_zero", cb.clone()); //             mul(cb, 0) = 0
        return Some((
            vec![],
            eq_trans(
                prod,
                ax2("mul", cb.clone(), ca.clone()),
                lit_t(0),
                flip,
                zero,
            ),
        ));
    }
    if is_lit(cb, 1) {
        let pca = to_poly(polys, ctx, ca); // ca is canonical, so reify(pca) == ca
        return Some((pca, ax1("mul_one", ca.clone()))); // mul(ca, 1) = ca
    }
    if is_lit(ca, 1) {
        let pcb = to_poly(polys, ctx, cb);
        let prod = ax2("mul", ca.clone(), cb.clone());
        let flip = ax2("mul_comm", ca.clone(), cb.clone()); // mul(1, cb) = mul(cb, 1)
        let one = ax1("mul_one", cb.clone()); //               mul(cb, 1) = cb
        return Some((
            pcb,
            eq_trans(
                prod,
                ax2("mul", cb.clone(), ca.clone()),
                cb.clone(),
                flip,
                one,
            ),
        ));
    }
    if let Some((cb1, cb2)) = match_bin(cb, "add") {
        // mul ca (add cb1 cb2) = add (mul ca cb1)(mul ca cb2)
        let distrib = ax3("mul_distrib_add", ca.clone(), cb1.clone(), cb2.clone());
        let (pp1, d1) = dist_mul(ctx, polys, ca, &cb1)?;
        let (pp2, d2) = dist_mul(ctx, polys, ca, &cb2)?;
        let rp1 = reify(&pp1, &polys.atoms);
        let rp2 = reify(&pp2, &polys.atoms);
        let cong = cong2(
            "add",
            &ax2("mul", ca.clone(), cb1.clone()),
            &rp1,
            &ax2("mul", ca.clone(), cb2.clone()),
            &rp2,
            d1,
            d2,
        );
        let (pm, mproof) = merge_canonical(&polys.atoms, &pp1, &pp2)?;
        let rpm = reify(&pm, &polys.atoms);
        let proof = eq_trans(
            ax2("mul", ca.clone(), cb.clone()),
            ax2(
                "add",
                ax2("mul", ca.clone(), cb1.clone()),
                ax2("mul", ca.clone(), cb2.clone()),
            ),
            rpm,
            distrib,
            eq_trans(
                ax2(
                    "add",
                    ax2("mul", ca.clone(), cb1),
                    ax2("mul", ca.clone(), cb2),
                ),
                ax2("add", rp1, rp2),
                reify(&pm, &polys.atoms),
                cong,
                mproof,
            ),
        );
        return Some((pm, proof));
    }
    if let Some((_ca1, _ca2)) = match_bin(ca, "add") {
        // mul (sum) cb = mul cb (sum) then distribute
        let comm = ax2("mul_comm", ca.clone(), cb.clone());
        let (pm, inner) = dist_mul(ctx, polys, cb, ca)?; // mul cb ca = reify(pm)
        let rpm = reify(&pm, &polys.atoms);
        return Some((
            pm,
            eq_trans(
                ax2("mul", ca.clone(), cb.clone()),
                ax2("mul", cb.clone(), ca.clone()),
                rpm,
                comm,
                inner,
            ),
        ));
    }
    // both monomials: canonicalize the product. The constructive merge sorts the atom factors by
    // comm/assoc swaps for ANY degree (no bounded search), so degree-4+ monomial reorderings like
    // (df·dg)·(f·g) = (df·f)·(dg·g) go through; it falls back to the bounded search when a factor is
    // not an interned atom (e.g. a literal coefficient).
    let prod = ax2("mul", ca.clone(), cb.clone());
    let pp = to_poly(polys, ctx, &prod);
    let c = reify(&pp, &polys.atoms);
    // Two constructive sorts build the proof directly (no search, fast at any degree): `merge_sorted`
    // for pure atom-products, and `merge_scaled` for signed/coefficient-carrying monomials (a factor
    // like `mul(-1, …)` from a subtraction — the det case). The bounded search is only a last-resort
    // net for whatever neither constructs (already-sorted / pure-literal cases, which it closes by refl).
    let proof = merge_sorted(polys, ca, cb)
        .filter(|(result, _)| *result == c)
        .or_else(|| merge_scaled(polys, ca, cb).filter(|(result, _)| *result == c))
        .map(|(_, pf)| pf)
        .or_else(|| prove_eq(ctx, &prod, &c, MONO_FALLBACK_DEPTH))?;
    Some((pp, proof))
}

/// `norm(t)` → `(canonical_term, proof : Eq Int t canonical_term)`, or `None`.
fn norm(ctx: &Context, polys: &mut Polynomials, t: &Term) -> Option<(Term, Term)> {
    if let Some((a, b)) = match_bin(t, "add") {
        let (ca, pa) = norm(ctx, polys, &a)?;
        let (cb, pb) = norm(ctx, polys, &b)?;
        let pa_poly = to_poly(polys, ctx, &a);
        let pb_poly = to_poly(polys, ctx, &b);
        let cong = cong2("add", &a, &ca, &b, &cb, pa, pb); // add a b = add ca cb
        let (merged, merge) = merge_canonical(&polys.atoms, &pa_poly, &pb_poly)?;
        let c = reify(&merged, &polys.atoms);
        return Some((
            c.clone(),
            eq_trans(t.clone(), ax2("add", ca, cb), c, cong, merge),
        ));
    }
    if let Some((a, b)) = match_bin(t, "mul") {
        let (ca, pa) = norm(ctx, polys, &a)?;
        let (cb, pb) = norm(ctx, polys, &b)?;
        let cong = cong2("mul", &a, &ca, &b, &cb, pa, pb); // mul a b = mul ca cb
        let (pm, dproof) = dist_mul(ctx, polys, &ca, &cb)?;
        let c = reify(&pm, &polys.atoms);
        return Some((
            c.clone(),
            eq_trans(t.clone(), ax2("mul", ca, cb), c, cong, dproof),
        ));
    }
    if let Some((a, b)) = match_bin(t, "sub") {
        // sub a b = add a (mul (-1) b)  [sub_elim], then normalize the add/mul form. Without this the
        // proof path leaves a symbolic `sub` stuck (only `to_poly` decides it), so a `sub` identity
        // would be decided-true yet unprovable.
        let elimmed = ax2("add", a.clone(), ax2("mul", lit_t(-1), b.clone()));
        let elim = ax2("sub_elim", a, b);
        let (c, p) = norm(ctx, polys, &elimmed)?;
        return Some((c.clone(), eq_trans(t.clone(), elimmed, c, elim, p)));
    }
    // atoms, literals: canonical form via the bounded search (handles nothing
    // for a bare atom — refl — and is here for robustness).
    let c = reify(&to_poly(polys, ctx, t), &polys.atoms);
    let proof = prove_eq(ctx, t, &c, MAX_REWRITE_DEPTH)?;
    Some((c, proof))
}

/// Prove `Eq Int lhs rhs` by normalizing both sides to the shared canonical form.
fn prove_by_normalization(ctx: &Context, lhs: &Term, rhs: &Term) -> Option<Term> {
    let mut polys = Polynomials { atoms: Vec::new() };
    let (cl, pl) = norm(ctx, &mut polys, lhs)?; // lhs = cl
    let (cr, pr) = norm(ctx, &mut polys, rhs)?; // rhs = cr
                                                // The guard guarantees the polynomials match; their canonical terms are equal.
    if cl != cr {
        return None;
    }
    // lhs = cl = cr = rhs  ⇒  lhs = rhs
    Some(eq_trans(
        lhs.clone(),
        cl,
        rhs.clone(),
        pl,
        eq_sym(rhs.clone(), cr, pr),
    ))
}

/// Bound on the multi-step (Eq_trans) rewrite search. Congruence does not consume
/// it (it recurses on strictly-smaller subterms), so this only limits same-size
/// axiom-chaining — enough for the ring identities that arise, and total.
const MAX_REWRITE_DEPTH: u32 = 6;

/// Last-resort bound for the monomial-product fallback. Both the pure-atom (`merge_sorted`) and the
/// coefficient-aware (`merge_scaled`) sorts build their proofs constructively, so this search is only
/// a net for the residual already-sorted / pure-literal cases it closes by `refl` — a shallow bound
/// suffices, and nothing high-degree reaches it (that was the source of the exponential blow-up).
const MONO_FALLBACK_DEPTH: u32 = 6;

fn prove_eq(ctx: &Context, lhs: &Term, rhs: &Term, depth: u32) -> Option<Term> {
    // 1. Computation: if both sides reduce to the same term, `refl` closes it.
    //    Covers all closed/literal arithmetic — zero axioms.
    let nlhs = normalize(ctx, lhs);
    let nrhs = normalize(ctx, rhs);
    if nlhs == nrhs {
        return Some(refl(nlhs));
    }

    // 2. A single oriented ring-axiom step (try both orientations).
    if let Some(p) = match_axiom(ctx, lhs, rhs) {
        return Some(p);
    }
    if let Some(p) = match_axiom(ctx, rhs, lhs) {
        // proof : Eq Int rhs lhs  ⇒  Eq_sym … : Eq Int lhs rhs
        return Some(eq_sym(rhs.clone(), lhs.clone(), p));
    }

    // 3. Congruence: `op a b = op a' b'` when `a=a'` and `b=b'` are each provable.
    //    Recurses on strictly-smaller subterms, so it terminates.
    for op in ["add", "mul", "sub"] {
        if let (Some((la, lb)), Some((ra, rb))) = (match_bin(lhs, op), match_bin(rhs, op)) {
            if let (Some(pa), Some(pb)) = (
                prove_eq(ctx, &la, &ra, depth),
                prove_eq(ctx, &lb, &rb, depth),
            ) {
                return Some(cong2(op, &la, &ra, &lb, &rb, pa, pb));
            }
        }
    }

    // 4. Multi-step: rewrite lhs → mid by one forward axiom, prove `mid = rhs`,
    //    and compose with `Eq_trans`. Handles identities needing a rewrite plus a
    //    congruence (e.g. (x+y)+z = z+(y+x)). Depth-bounded ⇒ total.
    if depth > 0 {
        for (mid, p_lhs_mid) in forward_rewrites(lhs) {
            if let Some(p_mid_rhs) = prove_eq(ctx, &mid, rhs, depth - 1) {
                return Some(eq_trans(
                    lhs.clone(),
                    mid,
                    rhs.clone(),
                    p_lhs_mid,
                    p_mid_rhs,
                ));
            }
        }
    }

    None
}

/// `Eq_trans Int x y z p1 p2` : from `p1 : x=y` and `p2 : y=z`, prove `x=z`.
fn eq_trans(x: Term, y: Term, z: Term, p1: Term, p2: Term) -> Term {
    app(
        app(app(app(app(app(global("Eq_trans"), int()), x), y), z), p1),
        p2,
    )
}

/// Single-step forward ring rewrites of `l`: each `(l', proof : Eq Int l l')`.
/// Only top-level rewrites; sub-term rewriting is covered by congruence.
fn forward_rewrites(l: &Term) -> Vec<(Term, Term)> {
    let g = global;
    let mut out = Vec::new();
    // add_comm : add a b → add b a
    if let Some((a, b)) = match_bin(l, "add") {
        out.push((
            app2(g("add"), b.clone(), a.clone()),
            app2(g("add_comm"), a.clone(), b.clone()),
        ));
        // add_assoc fwd : add (add a b) c → add a (add b c)
        if let Some((a2, b2)) = match_bin(&a, "add") {
            let c = b.clone();
            out.push((
                app2(g("add"), a2.clone(), app2(g("add"), b2.clone(), c.clone())),
                app3(g("add_assoc"), a2.clone(), b2.clone(), c.clone()),
            ));
        }
        // add_assoc rev : add a (add b c) → add (add a b) c
        if let Some((b2, c2)) = match_bin(&b, "add") {
            let lhs_a = app2(g("add"), app2(g("add"), a.clone(), b2.clone()), c2.clone());
            let rhs_a = app2(g("add"), a.clone(), app2(g("add"), b2.clone(), c2.clone()));
            out.push((
                lhs_a.clone(),
                eq_sym(
                    lhs_a,
                    rhs_a,
                    app3(g("add_assoc"), a.clone(), b2.clone(), c2.clone()),
                ),
            ));
        }
    }
    // mul_comm : mul a b → mul b a
    if let Some((a, b)) = match_bin(l, "mul") {
        out.push((
            app2(g("mul"), b.clone(), a.clone()),
            app2(g("mul_comm"), a.clone(), b.clone()),
        ));
        // mul_assoc fwd : mul (mul a b) c → mul a (mul b c)
        if let Some((a2, b2)) = match_bin(&a, "mul") {
            let c = b.clone();
            out.push((
                app2(g("mul"), a2.clone(), app2(g("mul"), b2.clone(), c.clone())),
                app3(g("mul_assoc"), a2.clone(), b2.clone(), c.clone()),
            ));
        }
        // mul_distrib_add fwd : mul a (add b c) → add (mul a b) (mul a c)
        if let Some((b2, c2)) = match_bin(&b, "add") {
            out.push((
                app2(
                    g("add"),
                    app2(g("mul"), a.clone(), b2.clone()),
                    app2(g("mul"), a.clone(), c2.clone()),
                ),
                app3(g("mul_distrib_add"), a.clone(), b2.clone(), c2.clone()),
            ));
        }
    }
    out
}

/// `Eq Int l r` as a term.
fn eq_int_term(l: Term, r: Term) -> Term {
    app(app(app(global("Eq"), int()), l), r)
}

/// `Eq_rec Int x P base y eqp` : rewrites `x` to `y` in `P` using `eqp : Eq Int x y`.
fn eq_rec(x: Term, motive: Term, base: Term, y: Term, eqp: Term) -> Term {
    app(
        app(
            app(app(app(app(global("Eq_rec"), int()), x), motive), base),
            y,
        ),
        eqp,
    )
}

/// `λ(__w : Int). body`
fn lam_int(body: Term) -> Term {
    Term::Lambda {
        param: "__w".to_string(),
        param_type: Box::new(int()),
        body: Box::new(body),
    }
}

/// Congruence for a binary op: from `pa : a = a'` and `pb : b = b'`, build a
/// proof of `Eq Int (op a b) (op a' b')` by two `Eq_rec` rewrites.
fn cong2(op: &str, a: &Term, a2: &Term, b: &Term, b2: &Term, pa: Term, pb: Term) -> Term {
    let opab = app2(global(op), a.clone(), b.clone());
    let w = Term::Var("__w".to_string());

    // step1 : Eq Int (op a b) (op a' b)   — rewrite a → a'
    //   motive P1 = λw. Eq Int (op a b) (op w b)
    let p1 = lam_int(eq_int_term(
        opab.clone(),
        app2(global(op), w.clone(), b.clone()),
    ));
    let step1 = eq_rec(a.clone(), p1, refl(opab.clone()), a2.clone(), pa);

    // step2 : Eq Int (op a b) (op a' b')  — rewrite b → b'
    //   motive P2 = λw. Eq Int (op a b) (op a' w)
    let p2 = lam_int(eq_int_term(opab.clone(), app2(global(op), a2.clone(), w)));
    eq_rec(b.clone(), p2, step1, b2.clone(), pb)
}

/// One oriented ring-axiom application proving `Eq Int l r`, if `(l, r)` matches.
fn match_axiom(ctx: &Context, l: &Term, r: &Term) -> Option<Term> {
    // add_comm : l = add a b,  r = add b a
    if let (Some((la, lb)), Some((ra, rb))) = (match_bin(l, "add"), match_bin(r, "add")) {
        if conv(ctx, &la, &rb) && conv(ctx, &lb, &ra) {
            return Some(app2(global("add_comm"), la, lb));
        }
    }
    // mul_comm : l = mul a b,  r = mul b a
    if let (Some((la, lb)), Some((ra, rb))) = (match_bin(l, "mul"), match_bin(r, "mul")) {
        if conv(ctx, &la, &rb) && conv(ctx, &lb, &ra) {
            return Some(app2(global("mul_comm"), la, lb));
        }
    }
    // add_assoc : l = add (add a b) c,  r = add a (add b c)
    if let Some((lab, lc)) = match_bin(l, "add") {
        if let Some((la, lb)) = match_bin(&lab, "add") {
            if let Some((ra, rbc)) = match_bin(r, "add") {
                if let Some((rb, rc)) = match_bin(&rbc, "add") {
                    if conv(ctx, &la, &ra) && conv(ctx, &lb, &rb) && conv(ctx, &lc, &rc) {
                        return Some(app3(global("add_assoc"), la, lb, lc));
                    }
                }
            }
        }
    }
    // mul_assoc : l = mul (mul a b) c,  r = mul a (mul b c)
    if let Some((lab, lc)) = match_bin(l, "mul") {
        if let Some((la, lb)) = match_bin(&lab, "mul") {
            if let Some((ra, rbc)) = match_bin(r, "mul") {
                if let Some((rb, rc)) = match_bin(&rbc, "mul") {
                    if conv(ctx, &la, &ra) && conv(ctx, &lb, &rb) && conv(ctx, &lc, &rc) {
                        return Some(app3(global("mul_assoc"), la, lb, lc));
                    }
                }
            }
        }
    }
    // add_zero : l = add a 0,  r = a
    if let Some((la, lb)) = match_bin(l, "add") {
        if conv(ctx, &lb, &Term::Lit(logicaffeine_kernel::Literal::Int(0))) && conv(ctx, &la, r) {
            return Some(app(global("add_zero"), la));
        }
    }
    // mul_one : l = mul a 1,  r = a
    if let Some((la, lb)) = match_bin(l, "mul") {
        if conv(ctx, &lb, &Term::Lit(logicaffeine_kernel::Literal::Int(1))) && conv(ctx, &la, r) {
            return Some(app(global("mul_one"), la));
        }
    }
    // mul_distrib_add : l = mul a (add b c),  r = add (mul a b) (mul a c)
    if let Some((a, bc)) = match_bin(l, "mul") {
        if let Some((b, c)) = match_bin(&bc, "add") {
            if let Some((rab, rac)) = match_bin(r, "add") {
                if let (Some((ra1, rb1)), Some((ra2, rc1))) =
                    (match_bin(&rab, "mul"), match_bin(&rac, "mul"))
                {
                    if conv(ctx, &a, &ra1)
                        && conv(ctx, &a, &ra2)
                        && conv(ctx, &b, &rb1)
                        && conv(ctx, &c, &rc1)
                    {
                        return Some(app3(global("mul_distrib_add"), a, b, c));
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/arith/tests.rs"]
mod tests;
