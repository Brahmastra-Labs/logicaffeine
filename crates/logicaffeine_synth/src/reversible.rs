//! Offline Z3 bijectivity proofs for the reversible zero-garbage (abelian) core.
//!
//! The register-VM opcode set is the chip ISA, and the `Rev*` abelian
//! ops are its zero-garbage adiabatic datapath: each reverses with no history tape because it
//! self-inverts algebraically. This module *proves* that — for each op's forward map `F` and its
//! declared inverse `G` (the pairing locked structurally by the
//! `reverse_op_lock` test), Z3 discharges `∀s. G(F(s)) = s` and `∀s. F(G(s)) = s` over the exact BV64
//! domain the VM computes in (`i64` wrapping arithmetic == 64-bit two's-complement bitvectors).
//!
//! The proof is by refutation: assert a counterexample `G(F(s)) ≠ s` exists and require `UNSAT`.
//! A deliberate-wrong **canary** (an inverse that does not invert) must be rejected, so the harness
//! is proven able to fail rather than rubber-stamp. This is the semantic complement to the static
//! catalog lock: that proves every op *names* an inverse; this proves the inverse *inverts*.

use z3::ast::BV;
use z3::{SatResult, Solver};

/// One reversible abelian op as a BV64 bijectivity obligation: its forward map and declared inverse,
/// each a function of the state word `x` and an independent group element `d` (the immediate delta;
/// ignored by the unary involutions). Independence of `d` from `x` is an invariant of the surface
/// lowering; here `d` is a free variable, so the proof is universal over it.
pub struct RevSpec {
    pub name: &'static str,
    pub forward: fn(&BV, &BV) -> BV,
    pub inverse: fn(&BV, &BV) -> BV,
}

fn add(x: &BV, d: &BV) -> BV {
    x.bvadd(d)
}
fn sub(x: &BV, d: &BV) -> BV {
    x.bvsub(d)
}
fn xor(x: &BV, d: &BV) -> BV {
    x.bvxor(d)
}
/// `RevNeg` — the VM computes `0 - x` (wrapping `i64` negation), so does this.
fn neg(x: &BV, _d: &BV) -> BV {
    BV::from_i64(0, 64).bvsub(x)
}

/// Every parameterized abelian bijection, paired with the inverse the ISA lock declares for it.
pub fn all_rev_specs() -> Vec<RevSpec> {
    vec![
        // (ℤ/2⁶⁴, +): RevAdd and RevSub are a mutually-inverse pair.
        RevSpec {
            name: "RevAdd",
            forward: add,
            inverse: sub,
        },
        RevSpec {
            name: "RevSub",
            forward: sub,
            inverse: add,
        },
        // 𝔽₂⁶⁴: RevXor is its own inverse.
        RevSpec {
            name: "RevXor",
            forward: xor,
            inverse: xor,
        },
        // Additive involution: RevNeg is its own inverse.
        RevSpec {
            name: "RevNeg",
            forward: neg,
            inverse: neg,
        },
    ]
}

/// Prove `spec` is a bijection with the declared inverse: BOTH `∀x,d. G(F(x,d),d) = x` and
/// `∀x,d. F(G(x,d),d) = x`. Returns `Ok(())` iff Z3 finds NO counterexample to either round-trip.
pub fn prove_bijective(spec: &RevSpec) -> Result<(), String> {
    for (label, first, second) in [
        ("G∘F", spec.forward, spec.inverse),
        ("F∘G", spec.inverse, spec.forward),
    ] {
        let solver = Solver::new();
        let x = BV::new_const("x", 64);
        let d = BV::new_const("d", 64);
        // Round-trip the state through F then its inverse; a bijection returns exactly `x`.
        let round = second(&first(&x, &d), &d);
        // Refutation: assert the round-trip DIFFERS from `x`. UNSAT ⟹ the identity holds for all x,d.
        solver.assert(&round.eq(&x).not());
        match solver.check() {
            SatResult::Unsat => {}
            SatResult::Sat => {
                return Err(format!(
                    "{} {label}: a counterexample exists — not a bijection",
                    spec.name
                ))
            }
            SatResult::Unknown => {
                return Err(format!("{} {label}: Z3 returned unknown", spec.name))
            }
        }
    }
    Ok(())
}

/// `RevSwap` is a permutation on TWO state words `(a, b) ↦ (b, a)`; prove it is an involution —
/// applying it twice is the identity on the pair. (Its own inverse, so `NamedOp` is itself.)
pub fn prove_swap_involution() -> Result<(), String> {
    let solver = Solver::new();
    let a = BV::new_const("a", 64);
    let b = BV::new_const("b", 64);
    // swap once → (b, a); swap again → (a, b).
    let (a1, b1) = (b.clone(), a.clone());
    let (a2, b2) = (b1, a1);
    // Refutation: assert the double-swap is NOT the original pair. UNSAT ⟹ it always is.
    solver.assert(&(a2.eq(&a) & b2.eq(&b)).not());
    match solver.check() {
        SatResult::Unsat => Ok(()),
        _ => Err("RevSwap is not an involution".to_string()),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_synth/reversible/tests.rs"]
mod tests;
