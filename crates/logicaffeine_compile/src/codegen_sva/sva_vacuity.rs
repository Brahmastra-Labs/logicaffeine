//! SVA Nonvacuous Evaluation Tracking (IEEE 16.14.8)
//!
//! Implements the 33 rules (a-ag) for determining when an assertion
//! evaluation is nonvacuous. A vacuous success means the antecedent
//! was never triggered — the property never actually tested the design.
//! If ALL evaluation attempts are vacuous, the assertion is dead.

use super::sva_model::SvaExpr;

/// Result of vacuity analysis for a property.
#[derive(Debug, Clone, PartialEq)]
pub enum VacuityStatus {
    /// The evaluation is definitely nonvacuous
    Nonvacuous,
    /// The evaluation is vacuous (property never tested)
    Vacuous,
    /// Cannot determine statically — depends on runtime values
    Unknown,
}

/// Analyze whether an SVA property evaluation can be nonvacuous.
///
/// Returns `Nonvacuous` if there exists at least one evaluation attempt
/// where the property is exercised (antecedent triggered, etc.).
/// Returns `Vacuous` if the property can NEVER be exercised.
/// Returns `Unknown` for runtime-dependent cases.
///
/// IEEE 16.14.8 rules a-ag.
pub fn analyze_vacuity(expr: &SvaExpr) -> VacuityStatus {
    match expr {
        // Rule (a): A sequence is always nonvacuous
        SvaExpr::Delay { .. } | SvaExpr::Repetition { .. } |
        SvaExpr::GotoRepetition { .. } | SvaExpr::NonConsecRepetition { .. } => {
            VacuityStatus::Nonvacuous
        }

        // Rule (b): strong(seq) is always nonvacuous
        SvaExpr::Strong(_) => VacuityStatus::Nonvacuous,

        // Rule (c): weak(seq) is always nonvacuous
        SvaExpr::Weak(_) => VacuityStatus::Nonvacuous,

        // Rule (d): not p is nonvacuous iff p is nonvacuous
        SvaExpr::PropertyNot(inner) => analyze_vacuity(inner),

        // Rule (e): p or q is nonvacuous if either is
        SvaExpr::Or(l, r) | SvaExpr::SequenceOr(l, r) | SvaExpr::PropertyIff(l, r) => {
            match (analyze_vacuity(l), analyze_vacuity(r)) {
                (VacuityStatus::Nonvacuous, _) | (_, VacuityStatus::Nonvacuous) => {
                    VacuityStatus::Nonvacuous
                }
                (VacuityStatus::Vacuous, VacuityStatus::Vacuous) => VacuityStatus::Vacuous,
                _ => VacuityStatus::Unknown,
            }
        }

        // Rule (f): p and q is nonvacuous if either is
        SvaExpr::And(l, r) | SvaExpr::SequenceAnd(l, r) => {
            match (analyze_vacuity(l), analyze_vacuity(r)) {
                (VacuityStatus::Nonvacuous, _) | (_, VacuityStatus::Nonvacuous) => {
                    VacuityStatus::Nonvacuous
                }
                (VacuityStatus::Vacuous, VacuityStatus::Vacuous) => VacuityStatus::Vacuous,
                _ => VacuityStatus::Unknown,
            }
        }

        // Rule (g): if(cond) p else q — nonvacuous if either branch can be
        SvaExpr::IfElse { then_expr, else_expr, .. } => {
            match (analyze_vacuity(then_expr), analyze_vacuity(else_expr)) {
                (VacuityStatus::Nonvacuous, _) | (_, VacuityStatus::Nonvacuous) => {
                    VacuityStatus::Nonvacuous
                }
                _ => VacuityStatus::Unknown,
            }
        }

        // Rule (h): seq |-> prop is nonvacuous when seq has an endpoint match
        // Rule (i): seq |=> prop is nonvacuous when seq has a match point
        SvaExpr::Implication { antecedent, .. } => {
            // The antecedent must be able to match for the property to be nonvacuous
            match analyze_vacuity(antecedent) {
                VacuityStatus::Nonvacuous => VacuityStatus::Unknown, // depends on runtime
                VacuityStatus::Vacuous => VacuityStatus::Vacuous,
                VacuityStatus::Unknown => VacuityStatus::Unknown,
            }
        }

        // Rule (j-k): followed-by operators
        SvaExpr::FollowedBy { antecedent, .. } => {
            match analyze_vacuity(antecedent) {
                VacuityStatus::Nonvacuous => VacuityStatus::Unknown,
                VacuityStatus::Vacuous => VacuityStatus::Vacuous,
                VacuityStatus::Unknown => VacuityStatus::Unknown,
            }
        }

        // Rule (l-n): nexttime/s_nexttime — nonvacuous if next tick exists and body nonvacuous
        SvaExpr::Nexttime(inner, _) | SvaExpr::SNexttime(inner, _) => {
            analyze_vacuity(inner)
        }

        // Rule (p): always p — nonvacuous when p is nonvacuous at some tick
        SvaExpr::Always(inner) | SvaExpr::SAlways(inner) => {
            analyze_vacuity(inner)
        }

        // Rule (q-r): always [m:n] p — nonvacuous at some tick in range
        SvaExpr::AlwaysBounded { body, .. } | SvaExpr::SAlwaysBounded { body, .. } => {
            analyze_vacuity(body)
        }

        // Rule (s): s_eventually p — nonvacuous if p holds at some tick
        SvaExpr::SEventually(inner) => analyze_vacuity(inner),

        // Rule (u): eventually [m:n] p — nonvacuous
        SvaExpr::EventuallyBounded { body, .. } | SvaExpr::SEventuallyBounded { body, .. } => {
            analyze_vacuity(body)
        }

        // Rule (v): p until q — nonvacuous
        SvaExpr::Until { .. } => VacuityStatus::Nonvacuous,

        // Rule (z): p implies q — nonvacuous when p is true
        SvaExpr::PropertyImplies(lhs, _) => {
            match analyze_vacuity(lhs) {
                VacuityStatus::Nonvacuous => VacuityStatus::Unknown, // depends on p being true
                VacuityStatus::Vacuous => VacuityStatus::Vacuous,
                VacuityStatus::Unknown => VacuityStatus::Unknown,
            }
        }

        // Rule (ag): disable iff (rst) p — nonvacuous when rst is not always active
        SvaExpr::DisableIff { body, .. } => {
            analyze_vacuity(body)
        }

        // PropertyCase: vacuity depends on whether the case expression matches
        // any item at runtime. Without a default, unmatched cases are vacuously true.
        // This is runtime-dependent, so return Unknown.
        SvaExpr::PropertyCase { default, .. } => {
            if default.is_some() {
                // With a default, some branch always fires
                VacuityStatus::Nonvacuous
            } else {
                // Without a default, whether any case matches is runtime-dependent
                VacuityStatus::Unknown
            }
        }

        // Atomic signals, constants, system functions — nonvacuous (they always evaluate)
        SvaExpr::Signal(_) | SvaExpr::Const(_, _) |
        SvaExpr::Rose(_) | SvaExpr::Fell(_) | SvaExpr::Past(_, _) |
        SvaExpr::Stable(_) | SvaExpr::Changed(_) |
        SvaExpr::Not(_) | SvaExpr::Eq(_, _) | SvaExpr::NotEq(_, _) |
        SvaExpr::LessThan(_, _) | SvaExpr::GreaterThan(_, _) |
        SvaExpr::LessEqual(_, _) | SvaExpr::GreaterEqual(_, _) |
        SvaExpr::Ternary { .. } |
        SvaExpr::OneHot0(_) | SvaExpr::OneHot(_) | SvaExpr::CountOnes(_) |
        SvaExpr::IsUnknown(_) | SvaExpr::Sampled(_) | SvaExpr::Bits(_) | SvaExpr::Clog2(_) |
        SvaExpr::CountBits(_, _) | SvaExpr::IsUnbounded(_) |
        SvaExpr::AcceptOn { .. } | SvaExpr::RejectOn { .. } |
        SvaExpr::SyncAcceptOn { .. } | SvaExpr::SyncRejectOn { .. } |
        SvaExpr::FirstMatch(_) | SvaExpr::Throughout { .. } | SvaExpr::Within { .. } |
        SvaExpr::Intersect { .. } |
        SvaExpr::BitAnd(_, _) | SvaExpr::BitOr(_, _) | SvaExpr::BitXor(_, _) |
        SvaExpr::BitNot(_) | SvaExpr::ReductionAnd(_) | SvaExpr::ReductionOr(_) |
        SvaExpr::ReductionXor(_) | SvaExpr::BitSelect { .. } | SvaExpr::PartSelect { .. } |
        SvaExpr::Concat(_) | SvaExpr::ConstCast(_) |
        SvaExpr::FieldAccess { .. } | SvaExpr::EnumLiteral { .. } |
        SvaExpr::Triggered(_) | SvaExpr::Matched(_) |
        SvaExpr::ImmediateAssert { .. } |
        SvaExpr::SequenceAction { .. } |
        SvaExpr::Clocked { .. } |
        SvaExpr::LocalVar(_) => {
            VacuityStatus::Nonvacuous
        }
    }
}

/// Check if ALL evaluation attempts of a property are vacuous.
/// If so, the assertion is dead and provides no verification value.
pub fn is_dead_assertion(expr: &SvaExpr) -> bool {
    matches!(analyze_vacuity(expr), VacuityStatus::Vacuous)
}
