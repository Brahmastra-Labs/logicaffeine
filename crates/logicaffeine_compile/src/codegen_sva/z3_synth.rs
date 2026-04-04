//! Sprint 4A: Z3 Synthesis Constraint Builder
//!
//! Converts a kernel spec type (Term) into a synthesis constraint.
//! When tactics can't construct a proof term directly, we ask Z3 to find one.
//!
//! For a spec like `Pi(a:Bit). Pi(b:Bit). Bit`, this extracts:
//! - Inputs: [(a, Bit), (b, Bit)]
//! - Output: Bit
//! - Constraint: the function must map all 2^n inputs to valid outputs

use logicaffeine_kernel::Term;

/// Configuration for synthesis constraint building.
#[derive(Debug, Clone)]
pub struct SynthesisConstraintConfig {
    /// Maximum number of Bit inputs to enumerate.
    pub max_inputs: usize,
    /// Timeout for Z3 in milliseconds.
    pub timeout_ms: u64,
}

impl Default for SynthesisConstraintConfig {
    fn default() -> Self {
        Self {
            max_inputs: 8,
            timeout_ms: 10000,
        }
    }
}

/// Result of building a synthesis constraint.
#[derive(Debug, Clone)]
pub enum SynthesisConstraint {
    /// A satisfiable constraint — the spec term can be synthesized.
    /// Contains the normalized spec type with extracted IO information.
    Satisfiable(Term),
    /// The spec is unrealizable — no implementation exists.
    Unrealizable,
    /// Could not translate the spec type.
    Unsupported(String),
}

/// Convert a kernel Pi-typed specification into a synthesis constraint.
///
/// Given a spec like `Pi(a:Bit). Pi(b:Bit). Bit`,
/// this determines whether the spec can be synthesized by:
/// 1. Extracting input/output types
/// 2. Checking all types are hardware types (Bit, BVec, Unit)
/// 3. Returning Satisfiable if the spec is well-formed
pub fn build_synthesis_constraint(
    spec_type: &Term,
    config: &SynthesisConstraintConfig,
) -> SynthesisConstraint {
    let (inputs, output) = extract_io_from_spec(spec_type);

    if inputs.is_empty() && output.is_none() {
        return SynthesisConstraint::Unsupported(
            "spec has no Pi binders — not a function type".to_string(),
        );
    }

    // Check all inputs are hardware types
    for (name, ty) in &inputs {
        if !is_hardware_type(ty) {
            return SynthesisConstraint::Unsupported(format!(
                "input '{}' has non-hardware type: {:?}",
                name, ty
            ));
        }
    }

    // Check input count is within bounds
    let bit_count = inputs.iter().filter(|(_, ty)| is_bit_type(ty)).count();
    if bit_count > config.max_inputs {
        return SynthesisConstraint::Unsupported(format!(
            "too many Bit inputs ({}) exceeds max_inputs ({})",
            bit_count, config.max_inputs
        ));
    }

    // Check output is a hardware type
    if let Some(ref out_ty) = output {
        if !is_hardware_type(out_ty) {
            return SynthesisConstraint::Unsupported(format!(
                "output has non-hardware type: {:?}",
                out_ty
            ));
        }
    }

    // If we got here, the spec is synthesizable
    SynthesisConstraint::Satisfiable(spec_type.clone())
}

/// Extract input/output signal types from a kernel spec type.
///
/// Walks Pi binders to find input names and types. The final non-Pi
/// type is the output type.
///
/// Example: `Pi(a:Bit). Pi(b:Bit). Bit` → inputs=[(a,Bit),(b,Bit)], output=Some(Bit)
pub fn extract_io_from_spec(spec_type: &Term) -> (Vec<(String, Term)>, Option<Term>) {
    let mut inputs = Vec::new();
    let mut current = spec_type;

    loop {
        match current {
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                inputs.push((param.clone(), *param_type.clone()));
                current = body_type;
            }
            other => {
                // This is the output type (or the body of a dependent type)
                return (inputs, Some(other.clone()));
            }
        }
    }
}

/// Check if a Term represents a hardware type (Bit, BVec, Unit, Circuit).
fn is_hardware_type(ty: &Term) -> bool {
    match ty {
        Term::Global(name) => matches!(
            name.as_str(),
            "Bit" | "Unit" | "BVec" | "Circuit"
        ),
        // BVec n — application of BVec to a Nat
        Term::App(func, _) => {
            if let Term::Global(name) = func.as_ref() {
                name == "BVec" || name == "Circuit"
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Check if a Term is specifically the Bit type.
fn is_bit_type(ty: &Term) -> bool {
    matches!(ty, Term::Global(name) if name == "Bit")
}
