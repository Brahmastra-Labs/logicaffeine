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
mod tests {
    use super::*;
    use crate::concurrency::marshal::GenExpr;
    use crate::interpreter::ClosureValue;
    use std::collections::HashMap;
    use std::rc::Rc;

    /// A shipped pure computation `3·x + 1` over one argument — exactly the shape a peer
    /// ships via `Send computed`.
    fn shipped_3x_plus_1() -> RuntimeValue {
        let gen = GenExpr::Add(
            Box::new(GenExpr::Mul(Box::new(GenExpr::Index), Box::new(GenExpr::Const(3)))),
            Box::new(GenExpr::Const(1)),
        );
        RuntimeValue::Function(Box::new(ClosureValue {
            body_index: usize::MAX,
            captured_env: HashMap::default(),
            param_names: vec![logicaffeine_base::Symbol::from_index(0)],
            generated: Some(Rc::new(gen)),
        }))
    }

    #[test]
    fn in_range_argument_runs_in_the_sandbox() {
        let contract = AcceptanceContract::new(0, 1000);
        // 3·5 + 1 = 16, and 5 ∈ [0, 1000].
        assert_eq!(contract.apply(&shipped_3x_plus_1(), 5).unwrap(), 16);
        // Boundaries are inclusive.
        assert_eq!(contract.apply(&shipped_3x_plus_1(), 0).unwrap(), 1);
        assert_eq!(contract.apply(&shipped_3x_plus_1(), 1000).unwrap(), 3001);
    }

    #[test]
    fn out_of_range_argument_is_refused_not_clamped() {
        let contract = AcceptanceContract::new(0, 1000);
        let above = contract.apply(&shipped_3x_plus_1(), 1001);
        let below = contract.apply(&shipped_3x_plus_1(), -1);
        assert!(above.is_err(), "an argument above the range must be refused");
        assert!(below.is_err(), "an argument below the range must be refused");
        // Refused, not clamped: the error names the offending value, and NO result is produced.
        assert!(above.unwrap_err().contains("1001"));
        assert!(below.unwrap_err().contains("refused"));
    }

    #[test]
    fn an_ordinary_closure_is_refused_at_the_signature_check() {
        // A closure with NO `generated` body (an arena-resident interpreter closure) must
        // never run under a contract — the sandbox is the only thing a contract evaluates.
        let ordinary = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: 7,
            captured_env: HashMap::default(),
            param_names: vec![logicaffeine_base::Symbol::from_index(0)],
            generated: None,
        }));
        let contract = AcceptanceContract::new(0, 1000);
        assert!(contract.apply(&ordinary, 5).is_err(), "an ordinary closure must be refused");
    }

    #[test]
    fn a_wrong_arity_computation_is_refused() {
        let gen = GenExpr::Index;
        let two_arg = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: usize::MAX,
            captured_env: HashMap::default(),
            param_names: vec![
                logicaffeine_base::Symbol::from_index(0),
                logicaffeine_base::Symbol::from_index(1),
            ],
            generated: Some(Rc::new(gen)),
        }));
        let contract = AcceptanceContract::new(0, 1000);
        assert!(contract.apply(&two_arg, 5).is_err(), "a 2-argument computation must be refused");
    }

    #[test]
    fn a_non_function_value_is_refused() {
        let contract = AcceptanceContract::new(0, 1000);
        assert!(contract.apply(&RuntimeValue::Int(5), 5).is_err(), "a non-function must be refused");
    }

    #[test]
    fn a_reversed_range_accepts_nothing() {
        // lo > hi is a vacuous contract — every argument is out of range.
        let contract = AcceptanceContract::new(1000, 0);
        assert!(contract.apply(&shipped_3x_plus_1(), 5).is_err());
        assert!(contract.apply(&shipped_3x_plus_1(), 500).is_err());
    }
}
