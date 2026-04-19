//! Hardware Verification Pipeline
//!
//! Public API for the LOGOS hardware verification pipeline:
//! English spec → Kripke FOL → Knowledge Graph → SVA → Z3 Equivalence.

use super::sva_model::{SvaExpr, parse_sva, sva_expr_to_string, sva_exprs_structurally_equivalent};
use super::sva_to_verify::{SvaTranslator, BoundedExpr, TranslateResult};
use super::fol_to_verify::FolTranslator;
use super::{SvaProperty, SvaAssertionKind, emit_sva_property, sanitize_property_name};
use logicaffeine_language::semantics::knowledge_graph::{HwKnowledgeGraph, SignalRole};
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════════════════
// SIGNAL MAP
// ═══════════════════════════════════════════════════════════════════════════

/// Maps FOL argument names (from English proper nouns) to SVA signal names.
///
/// When the English spec says "Req is valid", LOGOS produces `Valid(Req, w)`.
/// The signal map translates `Req` → `req` so the bounded variable becomes
/// `req@t` instead of `Valid_Req_@t`, matching the SVA side.
#[derive(Debug, Clone)]
pub struct SignalMap {
    map: HashMap<String, String>,
}

impl SignalMap {
    pub fn new() -> Self {
        Self { map: HashMap::new() }
    }

    /// Add a mapping from FOL argument name to SVA signal name.
    pub fn add(&mut self, fol_arg: &str, sva_signal: &str) {
        self.map.insert(fol_arg.to_string(), sva_signal.to_string());
    }

    /// Resolve a FOL argument name to its SVA signal name.
    pub fn resolve(&self, fol_arg: &str) -> Option<&str> {
        self.map.get(fol_arg).map(|s| s.as_str())
    }

    /// Build a signal map from hardware signal declarations.
    pub fn from_decls(decls: &[HwSignalDecl]) -> Self {
        let mut map = Self::new();
        for decl in decls {
            map.add(&decl.english_name, &decl.sva_name);
        }
        map
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HARDWARE SIGNAL DECLARATION
// ═══════════════════════════════════════════════════════════════════════════

/// Declares a hardware signal with its English name and SVA name.
///
/// The `english_name` should be a capitalized proper noun used in the English spec
/// (e.g., "Req", "Ack", "Awvalid"). The `sva_name` is the corresponding SVA signal
/// (e.g., "req", "ack", "AWVALID").
#[derive(Debug, Clone)]
pub struct HwSignalDecl {
    pub english_name: String,
    pub sva_name: String,
    pub width: u32,
    pub role: SignalRole,
}

impl HwSignalDecl {
    pub fn new(english_name: &str, sva_name: &str, width: u32, role: SignalRole) -> Self {
        Self {
            english_name: english_name.to_string(),
            sva_name: sva_name.to_string(),
            width,
            role,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PIPELINE TYPES
// ═══════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════
// PIPELINE FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════

/// Check structural equivalence between two SVA expressions.
pub fn check_structural_equivalence(sva_a: &str, sva_b: &str) -> Result<bool, HwError> {
    let expr_a = parse_sva(sva_a).map_err(|e| HwError::SvaParseError(e.message))?;
    let expr_b = parse_sva(sva_b).map_err(|e| HwError::SvaParseError(e.message))?;
    Ok(sva_exprs_structurally_equivalent(&expr_a, &expr_b))
}

/// Check bounded equivalence between two BoundedExpr trees.
pub fn check_bounded_equivalence(
    fol_bounded: &BoundedExpr,
    sva_bounded: &BoundedExpr,
    bound: u32,
) -> EquivalenceResult {
    let equivalent = bounded_exprs_equal(fol_bounded, sva_bounded);
    EquivalenceResult {
        equivalent,
        counterexample: None,
        bound,
    }
}

/// Translate an SVA string to bounded verification IR.
///
/// Uses translate_property (G-wrapping) to model `assert property` semantics.
pub fn translate_sva_to_bounded(sva_text: &str, bound: u32) -> Result<TranslateResult, HwError> {
    let sva_expr = parse_sva(sva_text).map_err(|e| HwError::SvaParseError(e.message))?;
    let mut translator = SvaTranslator::new(bound);
    let result = translator.translate_property(&sva_expr);
    Ok(result)
}

/// Translate SVA with smart G-wrapping for equivalence checking.
/// If the outermost SVA node is already temporal (s_eventually, nexttime),
/// translates at t=0 without G-wrapping. Otherwise uses translate_property.
fn translate_sva_for_equiv(sva_text: &str, bound: u32) -> Result<TranslateResult, HwError> {
    let sva_expr = parse_sva(sva_text).map_err(|e| HwError::SvaParseError(e.message))?;
    let mut translator = SvaTranslator::new(bound);
    if sva_has_outermost_temporal(&sva_expr) {
        let expr = translator.translate(&sva_expr, 0);
        let declarations: Vec<String> = translator.declarations.iter().cloned().collect();
        Ok(TranslateResult { expr, declarations })
    } else {
        Ok(translator.translate_property(&sva_expr))
    }
}

/// Check if the outermost SVA node is a temporal operator that already encodes
/// the temporal unrolling (s_eventually, nexttime). These should NOT be wrapped
/// in an additional G (conjunction over all timesteps).
fn sva_has_outermost_temporal(expr: &SvaExpr) -> bool {
    matches!(expr, SvaExpr::SEventually(_) | SvaExpr::Nexttime(_, _) | SvaExpr::SAlways(_))
}

/// Translate a LOGOS spec to bounded verification IR.
///
/// Phase 4b migration: routes through `parse_hw_spec_with`. For multi-
/// sentence inputs, only the first property is translated — matching
/// the legacy `compile_kripke`-based behavior which was implicitly
/// single-assertion. Use [`translate_spec_to_bounded_from_hwspec`] for
/// multi-property translation when callers already hold an `HwSpec`.
pub fn translate_spec_to_bounded(
    spec: &str,
    bound: u32,
) -> Result<TranslateResult, HwError> {
    use logicaffeine_language::hw_spec::parse_hw_spec_with;

    parse_hw_spec_with(spec, |hw_spec, interner| {
        if hw_spec.properties.is_empty() {
            return Err(HwError::ParseError(
                "no property sentences in spec".to_string(),
            ));
        }
        let mut translator = FolTranslator::new(interner, bound);
        Ok(translator.translate_property(hw_spec.properties[0]))
    })
    .map_err(|e| HwError::ParseError(format!("{:?}", e)))?
}

/// Translate the first property of an already-parsed [`HwSpec`] to
/// bounded verification IR. Returns `None` if the spec has no properties.
pub fn translate_spec_to_bounded_from_hwspec(
    spec: &logicaffeine_language::hw_spec::HwSpec<'_>,
    interner: &logicaffeine_base::Interner,
    bound: u32,
) -> Option<TranslateResult> {
    let first = spec.properties.first()?;
    let mut translator = FolTranslator::new(interner, bound);
    Some(translator.translate_property(first))
}

/// Compile an English hardware property with signal declarations.
///
/// Phase 4b migration: routes through `parse_hw_spec_with`. The signal
/// map is built from `HwSignalDecl`s; phase 4 already exposes
/// `HwSymbolTable::from_decls` as the canonical parity bridge.
pub fn compile_hw_property(
    spec: &str,
    decls: &[HwSignalDecl],
    bound: u32,
) -> Result<TranslateResult, HwError> {
    use logicaffeine_language::hw_spec::parse_hw_spec_with;

    let signal_map = SignalMap::from_decls(decls);
    parse_hw_spec_with(spec, |hw_spec, interner| {
        if hw_spec.properties.is_empty() {
            return Err(HwError::ParseError(
                "no property sentences in spec".to_string(),
            ));
        }
        let mut translator = FolTranslator::new(interner, bound);
        translator.set_signal_map(&signal_map);
        Ok(translator.translate_property(hw_spec.properties[0]))
    })
    .map_err(|e| HwError::ParseError(format!("{:?}", e)))?
}

/// Compile an already-parsed [`HwSpec`] with a signal map. Returns `None`
/// if the spec has no properties.
pub fn compile_hw_property_from_hwspec(
    spec: &logicaffeine_language::hw_spec::HwSpec<'_>,
    interner: &logicaffeine_base::Interner,
    signal_map: &SignalMap,
    bound: u32,
) -> Option<TranslateResult> {
    let first = spec.properties.first()?;
    let mut translator = FolTranslator::new(interner, bound);
    translator.set_signal_map(signal_map);
    Some(translator.translate_property(first))
}

/// Compile an English hardware spec to FOL text.
///
/// Phase 4b migration: routes through `parse_hw_spec_with` so the parse
/// boundary is unified with the typed `HwSpec` IR. Each property sentence
/// is transpiled in Kripke format; multi-property specs produce a numbered
/// list matching the shape `transpile_discourse` emits.
pub fn compile_hw_spec(source: &str) -> Result<String, HwError> {
    use logicaffeine_language::hw_spec::parse_hw_spec_with;
    use logicaffeine_language::OutputFormat;
    use logicaffeine_language::SymbolRegistry;

    parse_hw_spec_with(source, |spec, interner| {
        if spec.properties.is_empty() {
            return String::new();
        }
        let mut registry = SymbolRegistry::new();
        if spec.properties.len() == 1 {
            return spec.properties[0].transpile(&mut registry, interner, OutputFormat::Kripke);
        }
        let mut out = String::new();
        for (i, expr) in spec.properties.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!(
                "{}) {}",
                i + 1,
                expr.transpile(&mut registry, interner, OutputFormat::Kripke)
            ));
        }
        out
    })
    .map_err(|e| HwError::ParseError(format!("{:?}", e)))
}

/// Compile an already-parsed [`HwSpec`] directly to FOL text. Skips the
/// text→IR step for callers that already have a typed spec from
/// [`logicaffeine_language::hw_spec::parse_hw_spec_with`].
pub fn compile_hw_spec_from_hwspec(
    spec: &logicaffeine_language::hw_spec::HwSpec<'_>,
    interner: &logicaffeine_base::Interner,
) -> String {
    use logicaffeine_language::OutputFormat;
    use logicaffeine_language::SymbolRegistry;

    if spec.properties.is_empty() {
        return String::new();
    }
    let mut registry = SymbolRegistry::new();
    if spec.properties.len() == 1 {
        return spec.properties[0].transpile(&mut registry, interner, OutputFormat::Kripke);
    }
    let mut out = String::new();
    for (i, expr) in spec.properties.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}) {}",
            i + 1,
            expr.transpile(&mut registry, interner, OutputFormat::Kripke)
        ));
    }
    out
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

/// Extract a Knowledge Graph from an English hardware spec.
///
/// Phase 4b migration: routes through `parse_hw_spec_with`. Each parsed
/// property sentence contributes to the merged KG, matching the behavior
/// of the legacy `compile_kripke_with`-based path on single- and multi-
/// sentence inputs.
pub fn extract_kg(spec: &str) -> Result<HwKnowledgeGraph, HwError> {
    use logicaffeine_language::hw_spec::parse_hw_spec_with;
    use logicaffeine_language::semantics::knowledge_graph::extract_from_kripke_ast;

    parse_hw_spec_with(spec, |hw_spec, interner| {
        merge_property_kgs(hw_spec.properties.iter().copied(), interner)
    })
    .map_err(|e| HwError::ParseError(format!("{:?}", e)))
}

/// Extract a Knowledge Graph from an already-parsed [`HwSpec`] directly.
pub fn extract_kg_from_hwspec(
    spec: &logicaffeine_language::hw_spec::HwSpec<'_>,
    interner: &logicaffeine_base::Interner,
) -> HwKnowledgeGraph {
    merge_property_kgs(spec.properties.iter().copied(), interner)
}

fn merge_property_kgs<'a, I>(
    properties: I,
    interner: &logicaffeine_base::Interner,
) -> HwKnowledgeGraph
where
    I: Iterator<Item = &'a logicaffeine_language::ast::LogicExpr<'a>>,
{
    use logicaffeine_language::semantics::knowledge_graph::extract_from_kripke_ast;

    let mut signal_names: Vec<String> = Vec::new();
    let mut merged = HwKnowledgeGraph::default();
    for expr in properties {
        let sub = extract_from_kripke_ast(expr, interner);
        for signal in sub.signals {
            if !signal_names.iter().any(|n| n == &signal.name) {
                signal_names.push(signal.name.clone());
                merged.signals.push(signal);
            }
        }
        merged.properties.extend(sub.properties);
        merged.edges.extend(sub.edges);
        merged.entities.extend(sub.entities);
        merged.typed_edges.extend(sub.typed_edges);
    }
    merged
}

/// Check Z3 semantic equivalence between an English spec and an SVA string.
///
/// This is the core contribution — nobody else does this.
#[cfg(feature = "verification")]
pub fn check_z3_equivalence(
    spec_source: &str,
    sva_text: &str,
    bound: u32,
) -> Result<logicaffeine_verify::equivalence::EquivalenceResult, HwError> {
    use super::sva_to_verify::{bounded_to_verify, extract_signal_names};

    let spec_bounded = translate_spec_to_bounded(spec_source, bound)?;
    let spec_verify = bounded_to_verify(&spec_bounded.expr);

    let sva_bounded = translate_sva_for_equiv(sva_text, bound)?;
    let sva_verify = bounded_to_verify(&sva_bounded.expr);

    let mut all_signals = extract_signal_names(&spec_bounded);
    let sva_signals = extract_signal_names(&sva_bounded);
    for sig in sva_signals {
        if !all_signals.contains(&sig) {
            all_signals.push(sig);
        }
    }

    Ok(logicaffeine_verify::equivalence::check_equivalence(
        &spec_verify, &sva_verify, &all_signals, bound as usize,
    ))
}

/// Check Z3 semantic equivalence with hardware signal declarations.
///
/// This is the signal-bridge version that maps English proper nouns to SVA signal
/// names via HwSignalDecl. Both sides translate to the same variable namespace.
#[cfg(feature = "verification")]
pub fn check_z3_hw_equivalence(
    spec: &str,
    sva_text: &str,
    decls: &[HwSignalDecl],
    bound: u32,
) -> Result<logicaffeine_verify::equivalence::EquivalenceResult, HwError> {
    use super::sva_to_verify::{bounded_to_verify, extract_signal_names};

    // 1. Compile English with signal map
    let spec_bounded = compile_hw_property(spec, decls, bound)?;
    let spec_verify = bounded_to_verify(&spec_bounded.expr);

    // 2. Translate SVA with G-wrapping to match FOL temporal structure
    let sva_bounded = translate_sva_for_equiv(sva_text, bound)?;
    let sva_verify = bounded_to_verify(&sva_bounded.expr);

    // 3. Signal list from declarations + any extra signals from either side
    let mut all_signals: Vec<String> = decls.iter().map(|d| d.sva_name.clone()).collect();
    let spec_signals = extract_signal_names(&spec_bounded);
    let sva_signals = extract_signal_names(&sva_bounded);
    for sig in spec_signals.into_iter().chain(sva_signals.into_iter()) {
        if !all_signals.contains(&sig) {
            all_signals.push(sig);
        }
    }

    // 4. Z3 equivalence check
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
        (BoundedExpr::Lt(la, ra), BoundedExpr::Lt(lb, rb)) => {
            bounded_exprs_equal(la, lb) && bounded_exprs_equal(ra, rb)
        }
        (BoundedExpr::Gt(la, ra), BoundedExpr::Gt(lb, rb)) => {
            bounded_exprs_equal(la, lb) && bounded_exprs_equal(ra, rb)
        }
        (BoundedExpr::Lte(la, ra), BoundedExpr::Lte(lb, rb)) => {
            bounded_exprs_equal(la, lb) && bounded_exprs_equal(ra, rb)
        }
        (BoundedExpr::Gte(la, ra), BoundedExpr::Gte(lb, rb)) => {
            bounded_exprs_equal(la, lb) && bounded_exprs_equal(ra, rb)
        }
        _ => false,
    }
}
