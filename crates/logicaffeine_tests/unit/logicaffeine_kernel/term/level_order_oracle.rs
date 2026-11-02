// Test-only frozen pre-refactor decision core. Production has one shared solver.
use crate::Universe;
/// A level in the ℕ-ENCODING used by the decision core: `Prop = 0`, `Type n = n+1`,
/// and every universe VARIABLE ranges over all of ℕ (so a variable may be `Prop`,
/// which is what closes the `Sort u := Nat` unsoundness — `Type 0 ≤ u` is false, since
/// `u` could be `Prop`). This mirrors Lean's level model.
#[derive(Clone, Debug)]
enum LNat {
    Const(u64),
    Var(String),
    Succ(Box<LNat>),
    Max(Box<LNat>, Box<LNat>),
    /// `imax(a, b) = 0` when `b = 0`, else `max(a, b)`.
    IMax(Box<LNat>, Box<LNat>),
}

fn to_lnat(u: &Universe) -> LNat {
    match u {
        // SProp is handled directly in `level_leq` and never survives inside a compound
        // level (max/imax/succ collapse it), so this is a never-hit fallback.
        Universe::SProp => LNat::Const(0),
        Universe::Prop => LNat::Const(0),
        Universe::Type(n) => LNat::Const(*n as u64 + 1),
        Universe::Var(v) => LNat::Var(v.clone()),
        Universe::Succ(l) => LNat::Succ(Box::new(to_lnat(l))),
        Universe::Max(a, b) => LNat::Max(Box::new(to_lnat(a)), Box::new(to_lnat(b))),
        Universe::IMax(a, b) => LNat::IMax(Box::new(to_lnat(a)), Box::new(to_lnat(b))),
    }
}

/// Simplify an [`LNat`], resolving every `imax` whose right argument's Prop-ness is
/// determined and pushing `imax` down over `max`/`imax` until its right argument is a
/// bare variable (or a constant). After this, an unresolved `imax` has the form
/// `imax(_, Var v)`, so the remaining case analysis is on those `v`.
fn simp_lnat(t: &LNat) -> LNat {
    match t {
        LNat::Const(_) | LNat::Var(_) => t.clone(),
        LNat::Succ(l) => LNat::Succ(Box::new(simp_lnat(l))),
        LNat::Max(a, b) => LNat::Max(Box::new(simp_lnat(a)), Box::new(simp_lnat(b))),
        LNat::IMax(a, b) => {
            let a = simp_lnat(a);
            let b = simp_lnat(b);
            match &b {
                // b = 0 ⇒ imax = 0.
                LNat::Const(0) => LNat::Const(0),
                // b ≥ 1 (a positive constant or a successor) ⇒ imax = max(a, b).
                LNat::Const(_) | LNat::Succ(_) => simp_lnat(&LNat::Max(Box::new(a), Box::new(b))),
                // imax(a, max(b1,b2)) = max(imax(a,b1), imax(a,b2)).
                LNat::Max(b1, b2) => simp_lnat(&LNat::Max(
                    Box::new(LNat::IMax(Box::new(a.clone()), b1.clone())),
                    Box::new(LNat::IMax(Box::new(a), b2.clone())),
                )),
                // imax(a, imax(b1,b2)) = imax(max(a,b1), b2).
                LNat::IMax(b1, b2) => simp_lnat(&LNat::IMax(
                    Box::new(LNat::Max(Box::new(a), b1.clone())),
                    b2.clone(),
                )),
                // imax(a, v): stays symbolic (v may be 0 or ≥ 1).
                LNat::Var(_) => LNat::IMax(Box::new(a), Box::new(b)),
            }
        }
    }
}

/// A variable that still appears as the right argument of an `imax` — the pivot to
/// case-split on. `None` when the term is imax-free (a pure `max`/`succ`/`const`/`var`).
fn imax_pivot(t: &LNat) -> Option<String> {
    match t {
        LNat::Const(_) | LNat::Var(_) => None,
        LNat::Succ(l) => imax_pivot(l),
        LNat::Max(a, b) => imax_pivot(a).or_else(|| imax_pivot(b)),
        LNat::IMax(a, b) => match &**b {
            LNat::Var(v) => Some(v.clone()),
            _ => imax_pivot(a).or_else(|| imax_pivot(b)),
        },
    }
}

/// Substitute `Var(v)` by `repl` throughout an [`LNat`].
fn subst_lnat(t: &LNat, v: &str, repl: &LNat) -> LNat {
    match t {
        LNat::Const(_) => t.clone(),
        LNat::Var(x) => {
            if x == v {
                repl.clone()
            } else {
                t.clone()
            }
        }
        LNat::Succ(l) => LNat::Succ(Box::new(subst_lnat(l, v, repl))),
        LNat::Max(a, b) => LNat::Max(
            Box::new(subst_lnat(a, v, repl)),
            Box::new(subst_lnat(b, v, repl)),
        ),
        LNat::IMax(a, b) => LNat::IMax(
            Box::new(subst_lnat(a, v, repl)),
            Box::new(subst_lnat(b, v, repl)),
        ),
    }
}

/// A flat atom of an imax-free level: `var + offset` (`var = None` ⇒ a constant
/// `offset`). A level is a `max` of atoms.
fn lnat_atoms(t: &LNat, off: u64, out: &mut Vec<(Option<String>, u64)>) {
    match t {
        LNat::Const(c) => out.push((None, c + off)),
        LNat::Var(v) => out.push((Some(v.clone()), off)),
        LNat::Succ(l) => lnat_atoms(l, off + 1, out),
        LNat::Max(a, b) => {
            lnat_atoms(a, off, out);
            lnat_atoms(b, off, out);
        }
        // An imax-free term never contains IMax (guaranteed by the pivot loop).
        LNat::IMax(..) => unreachable!("lnat_atoms on an unresolved imax"),
    }
}

/// Decide `a ≤ b` for an IMAX-FREE pair, over ALL variable assignments (each variable
/// ranges over ℕ ≥ 0). `max(A) ≤ max(B)` holds iff every atom of `A` is dominated by
/// `B`: a constant `c` needs `c ≤ min max(B)` (all variables at 0); a `var+off` atom
/// needs `B` to contain the SAME variable with offset `≥ off` (else driving it to ∞
/// breaks the bound).
fn leq_linear(a: &LNat, b: &LNat) -> bool {
    let mut atoms_a = Vec::new();
    lnat_atoms(a, 0, &mut atoms_a);
    let mut atoms_b = Vec::new();
    lnat_atoms(b, 0, &mut atoms_b);
    // `min max(B)`: every variable at 0, so each atom contributes its offset.
    let b_min = atoms_b.iter().map(|(_, off)| *off).max().unwrap_or(0);
    atoms_a.iter().all(|(v, off)| match v {
        None => *off <= b_min,
        Some(name) => atoms_b
            .iter()
            .any(|(bv, boff)| bv.as_deref() == Some(name.as_str()) && *boff >= *off),
    })
}

#[cfg(test)]
thread_local! { static LEVEL_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Decide cumulative `a ≤ b`, SOUND over all instantiations of the universe variables
/// (each ranges over ℕ, `Prop = 0`). Unresolved `imax(_, v)` is handled by splitting
/// `v` into the `Prop` case (`v := 0`) and the positive case (`v := succ v′`); each
/// split removes a pivot, so the recursion terminates.
fn level_leq(a: &Universe, b: &Universe) -> bool {
    #[cfg(test)]
    LEVEL_COMPARISONS.with(|n| n.set(n.get() + 1));
    // SProp is the bottom sort — `SProp ≤ everything`, and only `SProp ≤ SProp`. Handled
    // BEFORE the `Prop=0` level encoding, which never sees `SProp` (max/imax/succ collapse
    // it away, so it only ever appears bare here).
    if matches!(a, Universe::SProp) {
        return true;
    }
    if matches!(b, Universe::SProp) {
        return false;
    }
    lnat_leq(&simp_lnat(&to_lnat(a)), &simp_lnat(&to_lnat(b)))
}

/// Every variable occurring in an [`LNat`], appended without duplicates — the occurs set the
/// `imax` case split must avoid.
fn lnat_vars(t: &LNat, out: &mut Vec<String>) {
    match t {
        LNat::Const(_) => {}
        LNat::Var(v) => {
            if !out.iter().any(|x| x == v) {
                out.push(v.clone());
            }
        }
        LNat::Succ(l) => lnat_vars(l, out),
        LNat::Max(a, b) | LNat::IMax(a, b) => {
            lnat_vars(a, out);
            lnat_vars(b, out);
        }
    }
}

/// The witness variable for the `v ≥ 1` branch of the `imax` case split: `v` decorated until it
/// occurs in NEITHER side.
///
/// Freshness here is soundness, not hygiene decoration. The branch stands for "`v` is some
/// value ≥ 1", written `v := succ(v′)`, and that reading is only valid while `v′` is a variable
/// nothing else constrains. A `v′` that already occurs TIES the case-split witness to a real
/// variable, so the branch checks the DIAGONAL instead of every independent pair — and
/// `imax(Prop, u) ≤ v′ + 2`, which is false at `u = 10, v′ = 0`, comes out TRUE. A kernel whose
/// level ordering accepts a judgement that does not hold is unsound at exactly the place
/// cumulativity is read off.
fn fresh_split_var(v: &str, a: &LNat, b: &LNat) -> String {
    let mut used = Vec::new();
    lnat_vars(a, &mut used);
    lnat_vars(b, &mut used);
    let mut candidate = format!("{v}✦");
    while used.iter().any(|x| *x == candidate) {
        candidate.push('✦');
    }
    candidate
}

fn lnat_leq(a: &LNat, b: &LNat) -> bool {
    match imax_pivot(a).or_else(|| imax_pivot(b)) {
        Some(v) => {
            // v = Prop (0).
            let zero = LNat::Const(0);
            let a0 = simp_lnat(&subst_lnat(a, &v, &zero));
            let b0 = simp_lnat(&subst_lnat(b, &v, &zero));
            // v ≥ 1: v := succ(v′) for a v′ fresh in BOTH sides (bakes the `≥ 1` into an
            // offset). See `fresh_split_var` for why the freshness is load-bearing.
            let vpos = LNat::Succ(Box::new(LNat::Var(fresh_split_var(&v, a, b))));
            let ap = simp_lnat(&subst_lnat(a, &v, &vpos));
            let bp = simp_lnat(&subst_lnat(b, &v, &vpos));
            lnat_leq(&a0, &b0) && lnat_leq(&ap, &bp)
        }
        None => leq_linear(a, b),
    }
}

pub(super) fn leq(a: &Universe, b: &Universe) -> bool {
    level_leq(a, b)
}
