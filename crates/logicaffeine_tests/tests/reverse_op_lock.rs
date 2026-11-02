//! ════════════════════════════════════════════════════════════════════════════════════════════
//! REVERSIBILITY LOCK (static half) — every bytecode `Op` must carry a reversibility STRATEGY, so
//! the whole ISA can be run backward (`forward ; reverse ≡ identity`). Because the register-VM
//! opcode set IS the chip ISA, classifying every op reversible is the
//! software prerequisite for the adiabatic / sub-Landauer hardware (§7 lever 3; `FUTURE_TELEPORT.md`
//! Phase IV). This is the reversibility analog of the WASM-AOT `op_support` lock.
//!
//! The classifier [`reversibility`] is an EXHAUSTIVE `match` on `Op` with NO wildcard arm, so the
//! COMPILER refuses to build the crate the moment a new `Op` is added until it is given a strategy:
//!
//!   • `Bijection` — injective on machine state with a named inverse; reverses with ZERO garbage
//!     (the abelian / involution core — the adiabatic charge-recovery path).
//!   • `Journaled` — information-losing; reversed by a minimal Bennett delta on the history tape.
//!   • `Effectful` — an observable side effect; reversed via the effect-log + a named barrier.
//!   • `Control`   — control flow / value-preserving; reversed on the recorded trace, no data inverse.
//!   • `NonReversible` — the honest "no story yet" census. It MUST stay EMPTY (see the census test).
//!
//! THE GOAL is `NonReversible == ∅` (already met) AND a strictly-growing `Bijection` set: the
//! zero-garbage core is what drives the tape — and the erased-bit energy cost — toward zero. The
//! behavioural `forward ; reverse ≡ initial-state` roundtrip lock lands in R1+ (it needs the reverse
//! executor); THIS file is the static catalog that no op may silently escape.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS FILE TO MAKE A RED CASE PASS.  ⚠️
//!  A RED means an op lost its reversibility strategy or a new op slipped past the catalog. The fix
//!  is a real strategy in `vm/reverse.rs`, NEVER a `NonReversible` bucket, a lowered floor, or a
//!  wildcard arm.

use logicaffeine_compile::vm::reverse::{reversibility, InverseSpec, RevClass};
use logicaffeine_compile::vm::Op;

/// Monotone floor of the ZERO-GARBAGE (`Bijection`) op set. RAISE it as each op is proven reversible
/// with a green `forward ; reverse` roundtrip that leaves NO tape entry — NEVER lower it to green a
/// red. R1 landed the abelian core: `RevSwap` (Sₙ involution), `RevXor` (𝔽₂ⁿ self-inverse), `RevNeg`
/// (additive involution), and the `RevAdd`/`RevSub` mutual-inverse pair over (ℤ/2ⁿ, +) — all proven
/// zero-garbage in `machine::reverse_execution`.
const BIJECTION_FLOOR: usize = 5;

/// Every ISA instruction paired with its reversibility strategy.
fn classified() -> Vec<(&'static str, RevClass)> {
    Op::all_variants()
        .iter()
        .map(|op| (op.name(), reversibility(op)))
        .collect()
}

#[test]
fn every_opcode_has_a_reversibility_classification() {
    // The classifier is an exhaustive match with NO wildcard arm (see `vm/reverse.rs`), so a new
    // `Op` fails to COMPILE until classified. This proves the enumeration is total and that the
    // classifier never panics on any instruction in the language.
    let all = classified();
    assert_eq!(all.len(), Op::all_variants().len());
    assert!(!all.is_empty(), "the ISA must have instructions");
}

#[test]
fn nonreversible_census_is_empty() {
    // ★ THE CENSUS ★ — every instruction has a reversibility STRATEGY. `NonReversible` is the
    // honest "no story yet" bucket and MUST stay ∅. A non-empty census is a RED to fix in the
    // classifier, never to accept.
    let offenders: Vec<&str> = classified()
        .into_iter()
        .filter_map(|(name, class)| matches!(class, RevClass::NonReversible(_)).then_some(name))
        .collect();
    assert!(
        offenders.is_empty(),
        "these ops have no reversibility strategy: {offenders:?}"
    );
}

#[test]
fn bijection_count_never_regresses() {
    let bijections = classified()
        .iter()
        .filter(|(_, c)| matches!(c, RevClass::Bijection(_)))
        .count();
    assert!(
        bijections >= BIJECTION_FLOOR,
        "zero-garbage (Bijection) coverage regressed: {bijections} < floor {BIJECTION_FLOOR}. \
         Raise the floor as ops are proven reversible; never lower it to green a red."
    );
}

#[test]
fn every_bijection_inverse_is_well_formed() {
    // A `Bijection`'s declared inverse must be resolvable: an `Involution` is its own inverse, a
    // `SelfFamily` inverts by flipping a flag on the same variant, and a `NamedOp` must name a REAL
    // op variant in the ISA. (Vacuous until R1 populates the `Bijection` set.)
    let names: std::collections::BTreeSet<&str> =
        Op::all_variants().iter().map(|op| op.name()).collect();
    for (name, class) in classified() {
        if let RevClass::Bijection(spec) = class {
            match spec {
                InverseSpec::Involution | InverseSpec::SelfFamily => {}
                InverseSpec::NamedOp(inv) => assert!(
                    names.contains(inv),
                    "op `{name}` declares inverse `{inv}`, which is not a real ISA op"
                ),
            }
        }
    }
}
