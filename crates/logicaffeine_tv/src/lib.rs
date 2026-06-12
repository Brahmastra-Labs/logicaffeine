//! # logicaffeine_tv — SMT translation validation
//!
//! Proves that the Rust emitted by the logicaffeine compiler is *observationally
//! equivalent* to the LOGOS source it was compiled from, per compile, by symbolically
//! executing both into the shared [`logicaffeine_verify`] semantic domain and
//! discharging the equivalence with Z3.
//!
//! The two encoders are *trusted but cross-validated*: the LOGOS encoder against the
//! tree-walking interpreter (the de-facto semantics), the Rust encoder against
//! compiling and running the emitted code. [`check_encoder_sound`] is the LOGOS-side
//! meta-soundness check — the load-bearing trust anchor for every downstream claim.
//!
//! This is rung 3–4 (translation validation), not rung 5 (machine-checked proof): the
//! trust boundary is the encoders + Z3 + rustc, not a mechanized meta-theorem.

pub mod equiv;
pub mod parse;
pub mod symexec;
pub mod verdict;

use logicaffeine_compile::compile::interpret_program;
use logicaffeine_verify::{BitVecOp, VerifyExpr};

pub use symexec::{SymSummary, SymValue};
pub use verdict::{SoundnessReport, TvError};

/// Parse `source` and symbolically execute it into a [`SymSummary`] over the LOGOS
/// semantics. Does not run the optimizer (the source-of-truth side).
pub fn summarize_logos(source: &str) -> Result<SymSummary, TvError> {
    match parse::with_program(source, false, symexec::execute) {
        Ok(Ok(summary)) => Ok(summary),
        Ok(Err(symexec::Unsupported(reason))) => Err(TvError::Unsupported(reason)),
        Err(e) => Err(TvError::Parse(e)),
    }
}

/// Cross-validate the LOGOS encoder against the tree-walking interpreter on one program.
///
/// Runs the program through `interpret_program` (the independent ground truth) and the
/// symbolic encoder, then proves with Z3 that they agree on the full observable behavior
/// — the ordered `Show` outputs and whether an error was raised. This is the meta-oracle
/// that makes the encoder trustworthy: a buggy encoder is caught here, not masked by a
/// downstream equivalence that "proves" two wrong things equal.
pub fn check_encoder_sound(source: &str) -> SoundnessReport {
    let summary = match summarize_logos(source) {
        Ok(s) => s,
        Err(TvError::Unsupported(reason)) => return SoundnessReport::Unsupported { reason },
        Err(TvError::Parse(e)) => {
            return SoundnessReport::ParseFailed {
                detail: format!("{e:?}"),
            }
        }
    };

    match interpret_program(source) {
        Err(e) => {
            // Interpreter raised → the encoder must prove the error condition holds.
            if equiv::is_valid(&summary.errored) {
                SoundnessReport::Agrees
            } else {
                SoundnessReport::Disagrees {
                    detail: format!(
                        "interpreter errored ({e:?}) but encoder did not prove `errored`"
                    ),
                }
            }
        }
        Ok(out) => {
            // Interpreter succeeded → the encoder must prove it does *not* error, and the
            // outputs must match position-for-position.
            if !equiv::is_valid(&VerifyExpr::not(summary.errored.clone())) {
                return SoundnessReport::Disagrees {
                    detail: "encoder admits an error on an input where the interpreter succeeded"
                        .to_string(),
                };
            }
            compare_outputs(&summary.outputs, &out)
        }
    }
}

/// What the interpreter's textual output line is expected to be.
enum Expected {
    Int(i64),
    Bool(bool),
}

fn parse_expected(line: &str) -> Option<Expected> {
    match line {
        "true" => Some(Expected::Bool(true)),
        "false" => Some(Expected::Bool(false)),
        _ => line.parse::<i64>().ok().map(Expected::Int),
    }
}

fn compare_outputs(outputs: &[SymValue], interp_out: &str) -> SoundnessReport {
    let lines: Vec<&str> = if interp_out.is_empty() {
        Vec::new()
    } else {
        interp_out.split('\n').collect()
    };

    if outputs.len() != lines.len() {
        return SoundnessReport::Disagrees {
            detail: format!(
                "output count: encoder produced {} line(s), interpreter produced {} ({:?})",
                outputs.len(),
                lines.len(),
                lines
            ),
        };
    }

    for (i, (slot, line)) in outputs.iter().zip(lines.iter()).enumerate() {
        let expected = match parse_expected(line) {
            Some(e) => e,
            None => {
                return SoundnessReport::Unsupported {
                    reason: format!("non-Int/Bool output line {i}: {line:?}"),
                }
            }
        };
        let pred = match (slot, expected) {
            (SymValue::Int(e), Expected::Int(n)) => {
                VerifyExpr::bv_binary(BitVecOp::Eq, e.clone(), VerifyExpr::bv_const(64, n as u64))
            }
            (SymValue::Bool(e), Expected::Bool(b)) => VerifyExpr::iff(e.clone(), VerifyExpr::bool(b)),
            (slot, _) => {
                return SoundnessReport::Disagrees {
                    detail: format!(
                        "output {i}: kind mismatch (encoder {} vs interpreter {line:?})",
                        kind_of(slot)
                    ),
                }
            }
        };
        if !equiv::is_valid(&pred) {
            return SoundnessReport::Disagrees {
                detail: format!("output {i}: encoder value disagrees with interpreter {line:?}"),
            };
        }
    }

    SoundnessReport::Agrees
}

fn kind_of(v: &SymValue) -> &'static str {
    match v {
        SymValue::Int(_) => "Int",
        SymValue::Bool(_) => "Bool",
    }
}
