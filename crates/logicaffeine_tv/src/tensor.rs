//! A rewrite of the one IR, proven: the obligation `tensor_graph::smt` prints for two graphs, discharged by Z3.
//!
//! A rewrite is proven when every lemma of its obligation holds: its premises are satisfiable — the facts it assumes hold
//! of some input, so nothing follows from them vacuously — and its premises with its goal are not: no input and no
//! meaning of the ops the IR leaves open makes the lemma's two terms differ, or a read of the rewritten graph stray out
//! of range.

use logicaffeine_compile::tensor_graph::smt::{Lemma, Obligation};
use z3::{Params, SatResult, Solver};

/// What Z3 says of one obligation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The rewrite keeps the meaning.
    Proven,
    /// A counterexample: the model Z3 found, an input and a meaning of the open ops under which the graphs differ.
    Refuted(String),
    /// The premises assume what no input satisfies, so the goal would follow from them whatever it says.
    Vacuous,
    /// Z3 gave no answer within the time, and why.
    Unknown(String),
}

fn solver(text: &str, timeout_ms: u32) -> Solver {
    let s = Solver::new();
    let mut params = Params::new();
    params.set_u32("timeout", timeout_ms);
    s.set_params(&params);
    s.from_string(text.to_string());
    s
}

/// Discharge `o`: every lemma, each given up to `timeout_ms` for each of its two checks. The first lemma that is not
/// proven gives the verdict.
pub fn discharge(o: &Obligation, timeout_ms: u32) -> Verdict {
    for lemma in &o.lemmas {
        let verdict = lemma_verdict(lemma, timeout_ms);
        if verdict != Verdict::Proven {
            return verdict;
        }
    }
    Verdict::Proven
}

fn lemma_verdict(lemma: &Lemma, timeout_ms: u32) -> Verdict {
    let premises = solver(&lemma.premises, timeout_ms);
    match premises.check() {
        SatResult::Sat => {}
        SatResult::Unsat => return Verdict::Vacuous,
        SatResult::Unknown => return Verdict::Unknown(premises.get_reason_unknown().unwrap_or_default()),
    }
    let whole = solver(&lemma.text(), timeout_ms);
    match whole.check() {
        SatResult::Unsat => Verdict::Proven,
        SatResult::Sat => Verdict::Refuted(whole.get_model().map(|m| m.to_string()).unwrap_or_default()),
        SatResult::Unknown => Verdict::Unknown(whole.get_reason_unknown().unwrap_or_default()),
    }
}
