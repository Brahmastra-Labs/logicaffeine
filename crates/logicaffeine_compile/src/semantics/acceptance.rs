//! C2 Layer C — the receiver's typed, bounded **acceptance contract** for shipped
//! computation.
//!
//! `Send computed f` (Layer B) lets a peer ship a *pure function* — its `GenExpr` body,
//! not its data — which the receiver evaluates in a bounded sandbox. That is safe in that
//! the sandbox can only do total integer arithmetic over one argument; it is NOT safe in
//! that the receiver would run *whatever shape* arrived on *whatever argument*.
//!
//! An [`AcceptanceContract`] closes that gap the way a web form's validator does: the
//! receiver writes down **exactly** the interface it will run — a single integer argument
//! within a declared inclusive range — and every invocation is validated against it
//! *before* evaluation. A function of the wrong shape is refused at the signature check; an
//! argument outside the range is **refused, never silently clamped**. The attack surface is
//! precisely what the receiver wrote down, and nothing more.
//!
//! The check is O(1) — two integer comparisons over a value the sandbox already bounds — so
//! the safety costs essentially nothing on the hot path.

use crate::concurrency::marshal::gen_eval;
use crate::interpreter::RuntimeValue;

/// A receiver-declared acceptance contract: a single integer parameter accepted only within
/// `[lo, hi]` (inclusive), returning an integer. The bound is what the receiver promises to
/// honor; anything outside is refused at the seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptanceContract {
    pub lo: i64,
    pub hi: i64,
}

impl AcceptanceContract {
    pub fn new(lo: i64, hi: i64) -> Self {
        // A reversed range accepts nothing — normalize so the bound reads as the user wrote
        // it but still rejects every argument (lo > hi ⇒ no `arg` satisfies lo ≤ arg ≤ hi).
        AcceptanceContract { lo, hi }
    }

    /// Validate `function` and `arg` against this contract, then evaluate. The two failure
    /// modes are distinct and both surface as `Err`:
    ///
    /// * **signature** — `function` must be a *shipped* pure computation (`generated`) of
    ///   exactly one argument. An ordinary closure (arena body) or a wrong arity is refused;
    ///   the contract only ever runs the bounded sandbox, never interpreter-resident code.
    /// * **range** — `arg` must satisfy `lo ≤ arg ≤ hi`. Out-of-range is refused, NOT
    ///   clamped: the receiver asked for a bounded domain, so an out-of-domain input is an
    ///   error at the edge, not a quietly-different computation.
    pub fn apply(&self, function: &RuntimeValue, arg: i64) -> Result<i64, String> {
        let closure = match function {
            RuntimeValue::Function(c) => c,
            other => {
                return Err(format!(
                    "acceptance contract: expected a shipped computation, got {}",
                    other.type_name()
                ))
            }
        };
        let gen = closure.generated.as_ref().ok_or_else(|| {
            "acceptance contract: an ordinary closure is refused — only a `Send computed` \
             shipped pure computation may be run under a contract"
                .to_string()
        })?;
        if closure.param_names.len() != 1 {
            return Err(format!(
                "acceptance contract: a shipped computation must take exactly one argument, \
                 this one takes {}",
                closure.param_names.len()
            ));
        }
        if arg < self.lo || arg > self.hi {
            return Err(format!(
                "acceptance contract: argument {arg} is outside the accepted range \
                 {}..={} — refused (the contract is not satisfied; the value is not clamped)",
                self.lo, self.hi
            ));
        }
        Ok(gen_eval(gen, arg))
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/acceptance/tests.rs"]
mod tests;
