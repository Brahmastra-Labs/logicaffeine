//! Hardware Verification Pipeline
//!
//! Public API for the LOGOS hardware verification pipeline:
//! English spec → Kripke FOL → Knowledge Graph → SVA → Z3 Equivalence.

use super::sva_model::{SvaExpr, parse_sva, sva_expr_to_string, sva_exprs_structurally_equivalent};
use super::sva_to_verify::{SvaTranslator, BoundedExpr, TranslateResult};
use super::fol_to_verify::FolTranslator;
use super::{SvaProperty, SvaAssertionKind, emit_sva_property, sanitize_property_name};
use logicaffeine_language::semantics::knowledge_graph::HwKnowledgeGraph;

/// Result of checking semantic equivalence between FOL and SVA.
#[derive(Debug)]
pub struct EquivalenceResult {
    /// Whether the properties are equivalent at the given bound.
    pub equivalent: bool,
    /// If not equivalent, counterexample signal assignments (name@timestep → value).
    pub counterexample: Option<Vec<(String, String)>>,
    /// The BMC bound used for checking.
    pub bound: u32,
}

/// A compiled hardware specification.
#[derive(Debug)]
pub struct HwSpec {
    /// Kripke-lowered FOL as formatted text.
    pub fol_text: String,
    /// Knowledge graph extracted from the spec.
    pub kg: HwKnowledgeGraph,
}

/// Full pipeline result.
#[derive(Debug)]
pub struct PipelineResult {
    /// Property name.
    pub property_name: String,
    /// Equivalence result.
    pub result: EquivalenceResult,
    /// Generated SVA text.
    pub sva_text: String,
    /// FOL text from the spec.
    pub fol_text: String,
}

/// Error type for hardware verification pipeline.
#[derive(Debug)]
pub enum HwError {
    ParseError(String),
    SvaParseError(String),
    VerificationError(String),
}

impl std::fmt::Display for HwError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HwError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            HwError::SvaParseError(msg) => write!(f, "SVA parse error: {}", msg),
            HwError::VerificationError(msg) => write!(f, "Verification error: {}", msg),
        }
    }
}

/// Check structural equivalence between two SVA expressions.
/// This is the non-Z3 version — compares AST structure, not Z3 semantics.
pub fn check_structural_equivalence(sva_a: &str, sva_b: &str) -> Result<bool, HwError> {
    let expr_a = parse_sva(sva_a).map_err(|e| HwError::SvaParseError(e.message))?;
    let expr_b = parse_sva(sva_b).map_err(|e| HwError::SvaParseError(e.message))?;
    Ok(sva_exprs_structurally_equivalent(&expr_a, &expr_b))
}

/// Check bounded equivalence between two BoundedExpr trees.
/// Returns EquivalenceResult with structural comparison.
/// For true Z3 semantic equivalence, use the verification-feature-gated version.
pub fn check_bounded_equivalence(
    fol_bounded: &BoundedExpr,
    sva_bounded: &BoundedExpr,
    bound: u32,
) -> EquivalenceResult {
    // Structural comparison of bounded expressions
    let equivalent = bounded_exprs_equal(fol_bounded, sva_bounded);
    EquivalenceResult {
        equivalent,
        counterexample: None,
        bound,
    }
}

/// Translate an SVA string to bounded verification IR.
pub fn translate_sva_to_bounded(sva_text: &str, bound: u32) -> Result<TranslateResult, HwError> {
    let sva_expr = parse_sva(sva_text).map_err(|e| HwError::SvaParseError(e.message))?;
    let mut translator = SvaTranslator::new(bound);
    let result = translator.translate_property(&sva_expr);
    Ok(result)
}

/// Translate a LOGOS spec to bounded verification IR using compile_kripke_with.
pub fn translate_spec_to_bounded(
    spec: &str,
    bound: u32,
) -> Result<TranslateResult, HwError> {
    logicaffeine_language::compile_kripke_with(spec, |ast, interner| {
        let mut translator = FolTranslator::new(interner, bound);
        translator.translate_property(ast)
    })
    .map_err(|e| HwError::ParseError(format!("{:?}", e)))
}

/// Compile an English hardware spec to FOL text.
pub fn compile_hw_spec(source: &str) -> Result<String, HwError> {
    logicaffeine_language::compile_kripke(source)
        .map_err(|e| HwError::ParseError(format!("{:?}", e)))
}

/// Emit SVA from a property specification.
pub fn emit_hw_sva(name: &str, clock: &str, body: &str, kind: SvaAssertionKind) -> String {
    let prop = SvaProperty {
        name: sanitize_property_name(name),
        clock: clock.to_string(),
        body: body.to_string(),
        kind,
    };
    emit_sva_property(&prop)
}

/// Extract a Knowledge Graph from an English hardware spec (one call).
///
/// Combines compile_kripke_with + extract_from_kripke_ast into a single
/// convenient API for the hardware verification pipeline.
pub fn extract_kg(spec: &str) -> Result<HwKnowledgeGraph, HwError> {
    logicaffeine_language::compile_kripke_with(spec, |ast, interner| {
        logicaffeine_language::semantics::knowledge_graph::extract_from_kripke_ast(ast, interner)
    })
    .map_err(|e| HwError::ParseError(format!("{:?}", e)))
}

/// Check Z3 semantic equivalence between an English spec and an SVA string.
///
/// This is the core contribution — nobody else does this.
/// Takes an English hardware specification and an SVA assertion, translates
/// both to bounded verification IR, and asks Z3 whether they're semantically
/// equivalent. Returns a counterexample trace if they diverge.
///
/// # Example
///
/// ```ignore
/// let result = check_z3_equivalence(
///     "Always, if every request holds, then every acknowledgment holds.",
///     "req |-> ack",
///     5,
/// ).unwrap();
/// match result {
///     EquivalenceResult::Equivalent => println!("SVA matches spec"),
///     EquivalenceResult::NotEquivalent { counterexample } => {
///         println!("Mismatch at cycle {}", counterexample.cycles[0].cycle);
///     }
///     EquivalenceResult::Unknown => println!("Z3 timeout"),
/// }
/// ```
#[cfg(feature = "verification")]
pub fn check_z3_equivalence(
    spec_source: &str,
    sva_text: &str,
    bound: u32,
) -> Result<logicaffeine_verify::equivalence::EquivalenceResult, HwError> {
    use super::sva_to_verify::{bounded_to_verify, extract_signal_names};

    // 1. Translate spec (English → FOL → BoundedExpr → VerifyExpr)
    let spec_bounded = translate_spec_to_bounded(spec_source, bound)?;
    let spec_verify = bounded_to_verify(&spec_bounded.expr);

    // 2. Translate SVA (SVA text → SvaExpr → BoundedExpr → VerifyExpr)
    let sva_bounded = translate_sva_to_bounded(sva_text, bound)?;
    let sva_verify = bounded_to_verify(&sva_bounded.expr);

    // 3. Collect all signal names from both sides
    let mut all_signals = extract_signal_names(&spec_bounded);
    let sva_signals = extract_signal_names(&sva_bounded);
    for sig in sva_signals {
        if !all_signals.contains(&sig) {
            all_signals.push(sig);
        }
    }

    // 4. Ask Z3: ¬(spec ↔ sva) satisfiable?
    Ok(logicaffeine_verify::equivalence::check_equivalence(
        &spec_verify, &sva_verify, &all_signals, bound as usize,
    ))
}

/// Check if two BoundedExpr trees are structurally equal.
fn bounded_exprs_equal(a: &BoundedExpr, b: &BoundedExpr) -> bool {
    match (a, b) {
        (BoundedExpr::Var(va), BoundedExpr::Var(vb)) => va == vb,
        (BoundedExpr::Bool(va), BoundedExpr::Bool(vb)) => va == vb,
        (BoundedExpr::Int(va), BoundedExpr::Int(vb)) => va == vb,
        (BoundedExpr::And(la, ra), BoundedExpr::And(lb, rb)) => {
            bounded_exprs_equal(la, lb) && bounded_exprs_equal(ra, rb)
        }
        (BoundedExpr::Or(la, ra), BoundedExpr::Or(lb, rb)) => {
            bounded_exprs_equal(la, lb) && bounded_exprs_equal(ra, rb)
        }
        (BoundedExpr::Not(ia), BoundedExpr::Not(ib)) => bounded_exprs_equal(ia, ib),
        (BoundedExpr::Implies(la, ra), BoundedExpr::Implies(lb, rb)) => {
            bounded_exprs_equal(la, lb) && bounded_exprs_equal(ra, rb)
        }
        (BoundedExpr::Eq(la, ra), BoundedExpr::Eq(lb, rb)) => {
            bounded_exprs_equal(la, lb) && bounded_exprs_equal(ra, rb)
        }
        _ => false,
    }
}
