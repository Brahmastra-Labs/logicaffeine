//! Proof-PRODUCING arithmetic oracle (untrusted search, kernel-checked proof).
//!
//! Given an `Int` equality goal `Eq Int lhs rhs`, [`prove_int_eq`] searches for a
//! genuine kernel proof term and returns it — or `None`. Nothing here is trusted:
//! whatever it returns is re-checked by the kernel's `infer_type`, so a wrong
//! proof is rejected, never believed. This is the Coq-`lia`/`nia` model — the fast
//! search lives outside the trusted base; a bug here can only cause a *failed*
//! proof, never a false one.
//!
//! Trust boundary: closed/literal goals are proven by `add`/`mul` **computation**
//! plus `refl` (zero axioms). Ring identities are proven from the seven registered
//! commutative-ring axioms (`add_comm`/`add_assoc`/`add_zero`/`mul_comm`/
//! `mul_assoc`/`mul_one`/`mul_distrib_add`) — the entire trusted arithmetic base.

use logicaffeine_kernel::{normalize, Context, Term};

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
    app(
        app(app(app(global("Eq_sym"), int()), x), y),
        proof,
    )
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
type Poly = Vec<(Mono, i64)>;

fn poly_canon(mut terms: Vec<(Mono, i64)>) -> Poly {
    for (m, _) in terms.iter_mut() {
        m.sort_unstable();
    }
    terms.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out: Poly = Vec::new();
    for (m, c) in terms {
        if c == 0 {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.0 == m {
                last.1 += c;
                if last.1 == 0 {
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
            t.push((m, c1 * c2));
        }
    }
    poly_canon(t)
}
fn poly_scale(k: i64, a: &Poly) -> Poly {
    poly_canon(a.iter().map(|(m, c)| (m.clone(), c * k)).collect())
}

/// Compute the canonical polynomial of an arithmetic term.
fn to_poly(p: &mut Polynomials, ctx: &Context, t: &Term) -> Poly {
    let t = normalize(ctx, t);
    if let Term::Lit(logicaffeine_kernel::Literal::Int(n)) = t {
        return if n == 0 { vec![] } else { vec![(vec![], n)] };
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
    vec![(vec![id], 1)]
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
    Term::Lit(logicaffeine_kernel::Literal::Int(n))
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
fn scaled_term(mono: &[usize], coeff: i64, atoms: &[Term]) -> Term {
    match mono_to_term(mono, atoms) {
        None => lit_t(coeff),
        Some(m) if coeff == 1 => m,
        Some(m) => ax2("mul", lit_t(coeff), m),
    }
}
/// The canonical term for a whole polynomial (left-assoc sum, sorted).
fn reify(poly: &[(Mono, i64)], atoms: &[Term]) -> Term {
    // Drop zero-coefficient monomials so the canonical form is unique (a cancelled
    // term must not linger as `add 0 …` / `mul 0 …`, which would make two equal
    // polynomials reify to syntactically different terms).
    let mut iter = poly.iter().filter(|(_, c)| *c != 0);
    let Some((m0, c0)) = iter.next() else {
        return lit_t(0);
    };
    let mut t = scaled_term(m0, *c0, atoms);
    for (m, c) in iter {
        t = ax2("add", t, scaled_term(m, *c, atoms));
    }
    t
}

/// Proof `term = mul (lit c) M`, where `term = scaled_term(m, c)` and `M = mono_to_term(m)`.
/// For `c == 1` the term is the bare monomial `M`, coerced via `mul_one`/`mul_comm`.
fn as_scaled_mul(c: i64, m_term: &Term) -> Term {
    if c == 1 {
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
        refl(ax2("mul", lit_t(c), m_term.clone()))
    }
}

/// Proof `add (mul c1 M) (mul c2 M) = mul (add c1 c2) M`  (right reverse-distribution).
fn rev_distrib(c1: i64, c2: i64, m_term: &Term) -> Term {
    let big_c = ax2("add", lit_t(c1), lit_t(c2));
    let mul_c1 = ax2("mul", lit_t(c1), m_term.clone());
    let mul_c2 = ax2("mul", lit_t(c2), m_term.clone());
    // mul C M = mul M C
    let s1 = ax2("mul_comm", big_c.clone(), m_term.clone());
    // mul M C = add (mul M c1) (mul M c2)
    let s2 = ax3("mul_distrib_add", m_term.clone(), lit_t(c1), lit_t(c2));
    // add (mul M c1)(mul M c2) = add (mul c1 M)(mul c2 M)
    let s3 = cong2(
        "add",
        &ax2("mul", m_term.clone(), lit_t(c1)),
        &mul_c1,
        &ax2("mul", m_term.clone(), lit_t(c2)),
        &mul_c2,
        ax2("mul_comm", m_term.clone(), lit_t(c1)),
        ax2("mul_comm", m_term.clone(), lit_t(c2)),
    );
    // mul C M = add (mul c1 M)(mul c2 M)
    let forward = eq_trans(
        ax2("mul", big_c.clone(), m_term.clone()),
        ax2("mul", m_term.clone(), big_c.clone()),
        ax2("add", mul_c1.clone(), mul_c2.clone()),
        s1,
        eq_trans(
            ax2("mul", m_term.clone(), big_c.clone()),
            ax2("add", ax2("mul", m_term.clone(), lit_t(c1)), ax2("mul", m_term.clone(), lit_t(c2))),
            ax2("add", mul_c1.clone(), mul_c2.clone()),
            s2,
            s3,
        ),
    );
    eq_sym(ax2("mul", big_c, m_term.clone()), ax2("add", mul_c1, mul_c2), forward)
}

/// Proof `add (scaled m c1) (scaled m c2) = scaled m (c1+c2)` for the SAME monomial `m`.
fn combine_coeff(m: &[usize], c1: i64, c2: i64, atoms: &[Term]) -> Option<Term> {
    let t1 = scaled_term(m, c1, atoms);
    let t2 = scaled_term(m, c2, atoms);
    let sum = c1 + c2;
    let result = scaled_term(m, sum, atoms);
    let _ = (&t1, &t2, &result);
    match mono_to_term(m, atoms) {
        // constant terms: `add (lit c1)(lit c2)` ≡ `lit (c1+c2)` by computation.
        None => Some(refl(lit_t(sum))),
        Some(m_term) => {
            // add t1 t2 = add (mul c1 M)(mul c2 M)  [coerce]  = mul (c1+c2) M  [rev_distrib]
            let coerce = cong2("add", &t1, &ax2("mul", lit_t(c1), m_term.clone()),
                &t2, &ax2("mul", lit_t(c2), m_term.clone()),
                as_scaled_mul(c1, &m_term), as_scaled_mul(c2, &m_term));
            let rd = rev_distrib(c1, c2, &m_term);
            let coerced = ax2("add", ax2("mul", lit_t(c1), m_term.clone()), ax2("mul", lit_t(c2), m_term.clone()));
            if sum != 1 {
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
fn merge_term(atoms: &[Term], p: &[(Mono, i64)], m: &[usize], c: i64) -> Option<(Poly, Term)> {
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
        return Some((vec![(m.to_vec(), c)], proof));
    }
    let (ml, cl) = p.last().unwrap().clone();
    let init = &p[..p.len() - 1];
    let last_t = scaled_term(&ml, cl, atoms);
    let reify_p = reify(p, atoms);

    use std::cmp::Ordering;
    match m.to_vec().cmp(&ml) {
        Ordering::Greater => {
            // already sorted: structurally reify(p ++ [term])
            let mut res = p.to_vec();
            res.push((m.to_vec(), c));
            Some((res, refl(ax2("add", reify_p, st))))
        }
        Ordering::Equal => {
            if cl + c == 0 {
                // The monomial cancels (`cl·M + c·M = 0`), so it drops from the
                // canonical form. `combine_coeff` gives `add last_t st = mul 0 M`;
                // chain `mul 0 M = mul M 0 (mul_comm) = 0 (mul_zero)`.
                let cc = combine_coeff(&ml, cl, c, atoms)?;
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
            let cc = combine_coeff(&ml, cl, c, atoms)?; // add last_t st = scaled(ml, cl+c)
            let combined = scaled_term(&ml, cl + c, atoms);
            if init.is_empty() {
                Some((vec![(ml, cl + c)], cc))
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
                    ax2("add", ri.clone(), ax2("add", scaled_term(&ml, cl, atoms), scaled_term(m, c, atoms))),
                    ax2("add", ri.clone(), combined),
                    assoc,
                    cong,
                );
                let mut res = init.to_vec();
                res.push((ml, cl + c));
                Some((res, proof))
            }
        }
        Ordering::Less => {
            if init.is_empty() {
                // add last_t st = add st last_t
                let mut res = vec![(m.to_vec(), c)];
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
                        ax2("add", ax2("add", reify(init, atoms), last_t.clone()), st.clone()),
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
fn merge_canonical(atoms: &[Term], pa: &[(Mono, i64)], pb: &[(Mono, i64)]) -> Option<(Poly, Term)> {
    let ra = reify(pa, atoms);
    if pb.is_empty() {
        // add (reify pa) 0 = reify pa
        return Some((pa.to_vec(), ax1("add_zero", ra)));
    }
    if pb.len() == 1 {
        let (m, c) = &pb[0];
        return merge_term(atoms, pa, m, *c);
    }
    let (ml, cl) = pb.last().unwrap().clone();
    let pb_init = &pb[..pb.len() - 1];
    let rbi = reify(pb_init, atoms);
    let slast = scaled_term(&ml, cl, atoms);
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
    let (m2, p2) = merge_term(atoms, &m1, &ml, cl)?; // add rm1 slast = reify(m2)
    let rm2 = reify(&m2, atoms);
    let proof = eq_trans(
        ax2("add", ra.clone(), ax2("add", rbi.clone(), slast.clone())),
        ax2("add", ax2("add", ra, rbi), slast.clone()),
        rm2,
        assoc_sym,
        eq_trans(
            ax2("add", ax2("add", reify(pa, atoms), reify(pb_init, atoms)), slast.clone()),
            ax2("add", rm1, slast),
            reify(&m2, atoms),
            cong,
            p2,
        ),
    );
    Some((m2, proof))
}

/// Distribute `mul ca cb` (canonical terms), returning
/// `(product_poly, proof : mul ca cb = reify(product))`.
fn dist_mul(ctx: &Context, polys: &mut Polynomials, ca: &Term, cb: &Term) -> Option<(Poly, Term)> {
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
            ax2("add", ax2("mul", ca.clone(), cb1.clone()), ax2("mul", ca.clone(), cb2.clone())),
            rpm,
            distrib,
            eq_trans(
                ax2("add", ax2("mul", ca.clone(), cb1), ax2("mul", ca.clone(), cb2)),
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
            eq_trans(ax2("mul", ca.clone(), cb.clone()), ax2("mul", cb.clone(), ca.clone()), rpm, comm, inner),
        ));
    }
    // both monomials: a single product; let the bounded search canonicalize it.
    let prod = ax2("mul", ca.clone(), cb.clone());
    let pp = to_poly(polys, ctx, &prod);
    let c = reify(&pp, &polys.atoms);
    let proof = prove_eq(ctx, &prod, &c, MAX_REWRITE_DEPTH)?;
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
        return Some((c.clone(), eq_trans(t.clone(), ax2("add", ca, cb), c, cong, merge)));
    }
    if let Some((a, b)) = match_bin(t, "mul") {
        let (ca, pa) = norm(ctx, polys, &a)?;
        let (cb, pb) = norm(ctx, polys, &b)?;
        let cong = cong2("mul", &a, &ca, &b, &cb, pa, pb); // mul a b = mul ca cb
        let (pm, dproof) = dist_mul(ctx, polys, &ca, &cb)?;
        let c = reify(&pm, &polys.atoms);
        return Some((c.clone(), eq_trans(t.clone(), ax2("mul", ca, cb), c, cong, dproof)));
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
    Some(eq_trans(lhs.clone(), cl, rhs.clone(), pl, eq_sym(rhs.clone(), cr, pr)))
}

/// Bound on the multi-step (Eq_trans) rewrite search. Congruence does not consume
/// it (it recurses on strictly-smaller subterms), so this only limits same-size
/// axiom-chaining — enough for the ring identities that arise, and total.
const MAX_REWRITE_DEPTH: u32 = 6;

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
            if let (Some(pa), Some(pb)) =
                (prove_eq(ctx, &la, &ra, depth), prove_eq(ctx, &lb, &rb, depth))
            {
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
                return Some(eq_trans(lhs.clone(), mid, rhs.clone(), p_lhs_mid, p_mid_rhs));
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
                eq_sym(lhs_a, rhs_a, app3(g("add_assoc"), a.clone(), b2.clone(), c2.clone())),
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
                app2(g("add"), app2(g("mul"), a.clone(), b2.clone()), app2(g("mul"), a.clone(), c2.clone())),
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
        app(app(app(app(app(global("Eq_rec"), int()), x), motive), base), y),
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
    let p1 = lam_int(eq_int_term(opab.clone(), app2(global(op), w.clone(), b.clone())));
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
mod tests {
    use super::*;
    use logicaffeine_kernel::{infer_type, prelude::StandardLibrary};

    fn ctx() -> Context {
        let mut c = Context::new();
        StandardLibrary::register(&mut c);
        c.add_declaration("x", int());
        c.add_declaration("y", int());
        c
    }

    /// The oracle must find a proof AND the kernel must accept it as `Eq Int lhs rhs`.
    fn assert_certifies(ctx: &Context, lhs: &Term, rhs: &Term) {
        let proof = prove_int_eq(ctx, lhs, rhs)
            .unwrap_or_else(|| panic!("oracle found no proof for {lhs:?} = {rhs:?}"));
        let ty = infer_type(ctx, &proof)
            .unwrap_or_else(|e| panic!("kernel rejected the proof for {lhs:?} = {rhs:?}: {e:?}"));
        let want = eq_int_term(lhs.clone(), rhs.clone());
        assert!(
            conv(ctx, &ty, &want),
            "proof types as {ty:?}, wanted Eq Int {lhs:?} {rhs:?}"
        );
    }

    fn add_t(a: Term, b: Term) -> Term {
        ax2("add", a, b)
    }
    fn mul_t(a: Term, b: Term) -> Term {
        ax2("mul", a, b)
    }

    #[test]
    fn coefficients_summing_to_one_recombine() {
        // The merge gap: like monomials whose coefficients sum to exactly 1 must
        // recombine to the bare monomial (2x + (-1)x = x), not fail the proof.
        let ctx = ctx();
        let x = global("x");
        assert_certifies(&ctx, &add_t(mul_t(lit_t(2), x.clone()), mul_t(lit_t(-1), x.clone())), &x);
        assert_certifies(&ctx, &add_t(mul_t(lit_t(-1), x.clone()), mul_t(lit_t(2), x.clone())), &x);
        assert_certifies(
            &ctx,
            &add_t(mul_t(lit_t(3), x.clone()), mul_t(lit_t(-2), x.clone())),
            &x,
        );
    }

    #[test]
    fn farkas_shape_big_l_certifies() {
        // cert_farkas's summed left side: Σ λᵢ·0 must certify equal to 0.
        let ctx = ctx();
        let big_l = add_t(mul_t(lit_t(1), lit_t(0)), mul_t(lit_t(1), lit_t(0)));
        assert_certifies(&ctx, &big_l, &lit_t(0));
    }

    #[test]
    fn farkas_shape_double_constant_big_r_certifies() {
        // The exact BigR cert_farkas builds for the double-constant system
        // x+1 ≤ y ∧ y+1 ≤ x+1 with multipliers λ = (1, 1):
        //   1·(y − (x+1)) + 1·((x+1) − (y+1))  =  −1
        // encoded sub-free as add(r, mul(−1, l)) per hypothesis.
        let ctx = ctx();
        let x = global("x");
        let y = global("y");
        let l1 = add_t(x.clone(), lit_t(1));
        let r1 = y.clone();
        let l2 = add_t(y.clone(), lit_t(1));
        let r2 = add_t(x.clone(), lit_t(1));
        let diff1 = add_t(r1, mul_t(lit_t(-1), l1));
        let diff2 = add_t(r2, mul_t(lit_t(-1), l2));
        let big_r = add_t(mul_t(lit_t(1), diff1), mul_t(lit_t(1), diff2));
        assert_certifies(&ctx, &big_r, &lit_t(-1));
    }
}
