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

/// Compile an English hardware property with signal declarations.
///
/// Takes an English specification and a list of signal declarations that map
/// English proper nouns to SVA signal names. Produces a BoundedExpr with
/// correctly-mapped signal variables.
pub fn compile_hw_property(
    spec: &str,
    decls: &[HwSignalDecl],
    bound: u32,
) -> Result<TranslateResult, HwError> {
    let signal_map = SignalMap::from_decls(decls);
    logicaffeine_language::compile_kripke_with(spec, |ast, interner| {
        let mut translator = FolTranslator::new(interner, bound);
        translator.set_signal_map(&signal_map);
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
pub fn extract_kg(spec: &str) -> Result<HwKnowledgeGraph, HwError> {
    logicaffeine_language::compile_kripke_with(spec, |ast, interner| {
        logicaffeine_language::semantics::knowledge_graph::extract_from_kripke_ast(ast, interner)
    })
    .map_err(|e| HwError::ParseError(format!("{:?}", e)))
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

/// Check FOL ↔ SVA **semantic** equivalence at a bound with our pure-Rust, certified prover
/// (no Z3) — the in-browser counterpart of [`check_z3_equivalence`]. Lowers both bounded
/// obligations to `ProofExpr` and discharges `F ↔ S` through the CDCL → RUP tiers: an
/// `equivalent` verdict is RUP-certified, a non-equivalent one carries a counterexample
/// trace (`name@t` → `"1"`/`"0"`). Errors (fail-closed, never a false "equivalent") if
/// either obligation leaves the Boolean fragment — data-path needs bit-blasting, not wired
/// yet.
pub fn prove_bounded_equivalence(
    fol_bounded: &BoundedExpr,
    sva_bounded: &BoundedExpr,
    bound: u32,
) -> Result<EquivalenceResult, HwError> {
    use super::sva_to_proof::bounded_to_proof;
    use logicaffeine_proof::sat::{prove_equivalence, EquivOutcome};

    let fol = bounded_to_proof(fol_bounded).ok_or_else(|| {
        HwError::VerificationError(
            "FOL obligation is outside the Boolean fragment (bit-blasting not yet wired)".into(),
        )
    })?;
    let sva = bounded_to_proof(sva_bounded).ok_or_else(|| {
        HwError::VerificationError(
            "SVA obligation is outside the Boolean fragment (bit-blasting not yet wired)".into(),
        )
    })?;

    match prove_equivalence(&fol, &sva) {
        EquivOutcome::Equivalent => Ok(EquivalenceResult {
            equivalent: true,
            counterexample: None,
            bound,
        }),
        EquivOutcome::Differ(model) => Ok(EquivalenceResult {
            equivalent: false,
            counterexample: Some(
                model
                    .into_iter()
                    .map(|(name, v)| (name, if v { "1" } else { "0" }.to_string()))
                    .collect(),
            ),
            bound,
        }),
        EquivOutcome::Unsupported => Err(HwError::VerificationError(
            "obligation not purely propositional — escalate to bit-blasting".into(),
        )),
    }
}

/// End-to-end: an English spec and a candidate SVA string → certified semantic equivalence
/// with our prover. The Z3-free counterpart of [`check_z3_equivalence`]: the same
/// translators, our CDCL → RUP tiers instead of Z3.
pub fn prove_spec_sva_equivalence(
    spec_source: &str,
    sva_text: &str,
    bound: u32,
) -> Result<EquivalenceResult, HwError> {
    let spec_bounded = translate_spec_to_bounded(spec_source, bound)?;
    let sva_bounded = translate_sva_for_equiv(sva_text, bound)?;
    prove_bounded_equivalence(&spec_bounded.expr, &sva_bounded.expr, bound)
}

/// A node in the renderable knowledge-graph view: a signal with its role and width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KgNode {
    pub name: String,
    pub role: String,
    pub width: u32,
}

/// A directed relation between two nodes (indices into [`KgSummary::nodes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KgLink {
    pub from: usize,
    pub to: usize,
    pub relation: String,
}

/// A compact, render-ready view of the hardware knowledge graph extracted from an English
/// spec — signals as nodes, relations (drives / triggers / handshakes / …) as directed links.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KgSummary {
    pub nodes: Vec<KgNode>,
    pub links: Vec<KgLink>,
}

/// Extract a render-ready knowledge graph from an English hardware spec. Pure Rust (no Z3),
/// so it runs in the browser. Merges the legacy and typed (ontology) edge sets.
pub fn kg_summary(spec: &str) -> Result<KgSummary, HwError> {
    use logicaffeine_language::semantics::knowledge_graph::{KgRelation, SignalRole};
    use std::collections::HashMap;

    let kg = extract_kg(spec)?;
    let mut nodes = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for s in &kg.signals {
        let role = match s.role {
            SignalRole::Input => "input",
            SignalRole::Output => "output",
            SignalRole::Internal => "internal",
            SignalRole::Clock => "clock",
        }
        .to_string();
        index.insert(s.name.clone(), nodes.len());
        nodes.push(KgNode { name: s.name.clone(), role, width: s.width });
    }

    let mut links = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |from: &str, to: &str, rel: String, links: &mut Vec<KgLink>| {
        if let (Some(&f), Some(&t)) = (index.get(from), index.get(to)) {
            if f != t && seen.insert((f, t, rel.clone())) {
                links.push(KgLink { from: f, to: t, relation: rel });
            }
        }
    };
    for e in &kg.edges {
        let rel = match e.relation {
            KgRelation::Temporal => "temporal",
            KgRelation::Triggers => "triggers",
            KgRelation::Constrains => "constrains",
            KgRelation::TypeOf => "type",
        }
        .to_string();
        add(&e.from, &e.to, rel, &mut links);
    }
    for (from, to, rel) in &kg.typed_edges {
        add(from, to, typed_relation_label(rel), &mut links);
    }

    Ok(KgSummary { nodes, links })
}

fn typed_relation_label(rel: &logicaffeine_language::semantics::knowledge_graph::HwRelation) -> String {
    use logicaffeine_language::semantics::knowledge_graph::HwRelation as R;
    match rel {
        R::Drives | R::DrivesRegistered { .. } => "drives",
        R::DataFlow => "data",
        R::Reads => "reads",
        R::Writes => "writes",
        R::Controls => "controls",
        R::Selects => "selects",
        R::Enables => "enables",
        R::Resets => "resets",
        R::Triggers { .. } => "triggers",
        R::Constrains => "constrains",
        R::Follows { .. } => "follows",
        R::Precedes => "precedes",
        R::Preserves => "preserves",
        R::Contains => "contains",
        R::Instantiates => "instantiates",
        R::ConnectsTo => "connects",
        R::BelongsToDomain { .. } => "domain",
        R::HandshakesWith => "handshake",
        R::Acknowledges => "acks",
        R::Pipelines { .. } => "pipeline",
        R::MutuallyExcludes => "mutex",
        R::EventuallyFollows => "eventually",
        R::AssumedBy => "assumed",
    }
    .to_string()
}

/// One signal's value over time in a counterexample waveform. `width` is the bit width
/// (1 = a boolean control bit); each `values[t]` is the (reconstructed) register value at
/// timestep `t`, or `None` if unconstrained there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveSignal {
    pub name: String,
    pub width: u32,
    pub values: Vec<Option<u64>>,
}

/// A counterexample rendered as a waveform: signals (rows) over discrete timesteps (columns).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Waveform {
    pub timesteps: u32,
    pub signals: Vec<WaveSignal>,
}

/// Turn a counterexample's bindings into a waveform. Handles both 1-bit signals (`name@t`)
/// and bit-blasted multi-bit registers (`name@t#i`), reconstructing each register's integer
/// value per timestep from its bits. Signals are sorted by name; each row spans
/// `0..timesteps`. Malformed keys (no `@t`) are skipped.
pub fn counterexample_waveform(counterexample: &[(String, String)]) -> Waveform {
    use std::collections::BTreeMap;
    // name → (width, timestep → (bit-index → value))
    struct Acc {
        width: u32,
        per_t: BTreeMap<u32, BTreeMap<u32, bool>>,
    }
    let mut sigs: BTreeMap<String, Acc> = BTreeMap::new();
    let mut max_t = 0u32;
    let mut any = false;
    for (key, val) in counterexample {
        let bit_val = match val.as_str() {
            "1" | "true" => true,
            "0" | "false" => false,
            _ => continue,
        };
        // Optional `#<bit>` suffix marks a bit of a bit-blasted register.
        let (base, bit, multibit) = match key.rsplit_once('#') {
            Some((b, idx)) => match idx.parse::<u32>() {
                Ok(i) => (b, i, true),
                Err(_) => continue,
            },
            None => (key.as_str(), 0u32, false),
        };
        let Some((name, t_str)) = base.rsplit_once('@') else {
            continue;
        };
        let Ok(t) = t_str.parse::<u32>() else {
            continue;
        };
        let acc = sigs.entry(name.to_string()).or_insert(Acc { width: 1, per_t: BTreeMap::new() });
        if multibit {
            acc.width = acc.width.max(bit + 1);
        }
        acc.per_t.entry(t).or_default().insert(bit, bit_val);
        max_t = max_t.max(t);
        any = true;
    }
    let timesteps = if any { max_t + 1 } else { 0 };
    let signals = sigs
        .into_iter()
        .map(|(name, acc)| {
            let values = (0..timesteps)
                .map(|t| {
                    acc.per_t.get(&t).map(|bits| {
                        bits.iter().fold(0u64, |v, (&i, &b)| if b { v | (1u64 << i) } else { v })
                    })
                })
                .collect();
            WaveSignal { name, width: acc.width, values }
        })
        .collect();
    Waveform { timesteps, signals }
}

/// A vacuity verdict for a synthesized property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VacuityReport {
    /// The trigger (implication antecedent) is reachable — the property is non-vacuous.
    NonVacuous,
    /// The trigger can never fire — the property holds vacuously (a dead trigger / likely bug).
    Vacuous,
    /// The property has no antecedent (e.g. a pure safety assertion) — vacuity does not apply.
    NoTrigger,
    /// The antecedent left the Boolean fragment — escalate (bit-blasting).
    Unsupported,
}

/// The first implication antecedent reachable through the Boolean structure of a bounded
/// property — the SVA "trigger". `None` for a property with no implication (pure safety).
fn first_antecedent(e: &BoundedExpr) -> Option<&BoundedExpr> {
    match e {
        BoundedExpr::Implies(a, _) => Some(a),
        BoundedExpr::And(l, r) | BoundedExpr::Or(l, r) => {
            first_antecedent(l).or_else(|| first_antecedent(r))
        }
        BoundedExpr::Not(x) => first_antecedent(x),
        _ => None,
    }
}

/// Vacuity check (Z3-free, certified): does the property's trigger ever fire? An
/// unsatisfiable antecedent means the property passes vacuously — a dead trigger that usually
/// signals a malformed spec. Reuses [`logicaffeine_proof::bmc::check_vacuity`].
pub fn check_spec_vacuity(spec_source: &str, bound: u32) -> Result<VacuityReport, HwError> {
    use super::sva_to_proof::bounded_to_proof;
    use logicaffeine_proof::bmc::{check_vacuity, VacuityOutcome};

    let fol = translate_spec_to_bounded(spec_source, bound)?;
    let antecedent = match first_antecedent(&fol.expr) {
        Some(a) => a,
        None => return Ok(VacuityReport::NoTrigger),
    };
    let proof = match bounded_to_proof(antecedent) {
        Some(p) => p,
        None => return Ok(VacuityReport::Unsupported),
    };
    Ok(match check_vacuity(&proof) {
        VacuityOutcome::Vacuous => VacuityReport::Vacuous,
        VacuityOutcome::Reachable(_) => VacuityReport::NonVacuous,
        VacuityOutcome::Unsupported => VacuityReport::Unsupported,
    })
}

#[cfg(test)]
mod native_prove_tests {
    use super::*;

    /// A property is equivalent to itself — the load-bearing identity, RUP-certified.
    #[test]
    fn sva_self_equivalence_is_certified() {
        let a = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
        let b = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
        let r = prove_bounded_equivalence(&a.expr, &b.expr, 3).unwrap();
        assert!(r.equivalent, "self-equivalence must hold");
        assert!(r.counterexample.is_none());
    }

    /// The killer case: De Morgan duals are structurally DIFFERENT but semantically EQUAL.
    /// Structural equality (`check_bounded_equivalence`) cannot see it; our certified SAT
    /// prover can. This is the whole point of semantic equivalence.
    #[test]
    fn de_morgan_is_semantically_equivalent() {
        let a = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
        let b = translate_sva_to_bounded("!grant_a || !grant_b", 3).unwrap();
        assert!(
            !check_bounded_equivalence(&a.expr, &b.expr, 3).equivalent,
            "structural equality should NOT see De Morgan duals as equal"
        );
        let r = prove_bounded_equivalence(&a.expr, &b.expr, 3).unwrap();
        assert!(r.equivalent, "De Morgan duals must be semantically equivalent");
    }

    /// End-to-end: synthesizing SVA from an English spec must PRESERVE meaning — the
    /// synthesized SVA is certified equivalent to the spec's own FOL, by our prover.
    #[test]
    fn synthesized_sva_is_equivalent_to_its_spec() {
        let spec = "Always, if request is high, then acknowledge is high.";
        let synth = crate::codegen_sva::fol_to_sva::synthesize_sva_from_spec(spec, "clk").unwrap();
        let r = prove_spec_sva_equivalence(spec, &synth.body, 3).unwrap();
        assert!(r.equivalent, "synthesis must preserve meaning; got {:?}", r);
    }

    /// Distinct properties differ, and we recover a concrete counterexample trace.
    #[test]
    fn distinct_properties_differ_with_counterexample() {
        let a = translate_sva_to_bounded("!(grant_a && grant_b)", 2).unwrap();
        let b = translate_sva_to_bounded("grant_a |-> grant_b", 2).unwrap();
        let r = prove_bounded_equivalence(&a.expr, &b.expr, 2).unwrap();
        assert!(!r.equivalent, "a mutex and an implication are not equivalent");
        let ce = r.counterexample.expect("a counterexample trace");
        assert!(!ce.is_empty(), "counterexample must bind some signal@t");
    }

    #[test]
    fn implication_spec_has_a_reachable_trigger() {
        // The antecedent (`request is high`) is over a free signal, so it is reachable —
        // the property is non-vacuous.
        let r = check_spec_vacuity("Always, if request is high, then acknowledge is high.", 3)
            .unwrap();
        assert_eq!(r, VacuityReport::NonVacuous);
    }

    #[test]
    fn kg_summary_extracts_signals_and_relations() {
        let kg = kg_summary("Always, if request is high, then acknowledge is high.")
            .expect("extracts a knowledge graph");
        assert!(!kg.nodes.is_empty(), "the spec mentions signals: {kg:?}");
        // Every link must reference valid node indices.
        for l in &kg.links {
            assert!(l.from < kg.nodes.len() && l.to < kg.nodes.len(), "dangling link {l:?}");
            assert!(!l.relation.is_empty());
        }
        // The named signals should appear (proper nouns are capitalized in the KG).
        let names: Vec<&str> = kg.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("request")),
            "expected a Request signal among {names:?}"
        );
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("acknowledge")),
            "expected an Acknowledge signal among {names:?}"
        );
    }

    #[test]
    fn waveform_groups_boolean_signals_over_timesteps() {
        let ce = vec![
            ("req@0".to_string(), "0".to_string()),
            ("req@1".to_string(), "1".to_string()),
            ("ack@1".to_string(), "0".to_string()),
        ];
        let wf = counterexample_waveform(&ce);
        assert_eq!(wf.timesteps, 2);
        assert_eq!(wf.signals.len(), 2);
        let ack = wf.signals.iter().find(|s| s.name == "ack").unwrap();
        assert_eq!(ack.width, 1);
        assert_eq!(ack.values, vec![None, Some(0)]);
        let req = wf.signals.iter().find(|s| s.name == "req").unwrap();
        assert_eq!(req.values, vec![Some(0), Some(1)]);
    }

    #[test]
    fn waveform_reconstructs_multibit_register_values() {
        // cnt is 2 bits: at t0 = 0b01 = 1, at t1 = 0b10 = 2.
        let ce = vec![
            ("cnt@0#0".to_string(), "1".to_string()),
            ("cnt@0#1".to_string(), "0".to_string()),
            ("cnt@1#0".to_string(), "0".to_string()),
            ("cnt@1#1".to_string(), "1".to_string()),
        ];
        let wf = counterexample_waveform(&ce);
        let cnt = wf.signals.iter().find(|s| s.name == "cnt").unwrap();
        assert_eq!(cnt.width, 2);
        assert_eq!(cnt.values, vec![Some(1), Some(2)]);
    }

    #[test]
    fn waveform_skips_malformed_keys() {
        let ce = vec![("nonsense".to_string(), "1".to_string())];
        assert_eq!(counterexample_waveform(&ce), Waveform::default());
    }

    #[test]
    fn first_antecedent_extracts_the_trigger() {
        use crate::codegen_sva::sva_to_verify::BoundedExpr;
        let v = |s: &str| BoundedExpr::Var(s.to_string());
        let bx = |e: BoundedExpr| Box::new(e);
        // ⋀ over timesteps of (req@t → ack@t): the first antecedent is req@0.
        let prop = BoundedExpr::And(
            bx(BoundedExpr::Implies(bx(v("req@0")), bx(v("ack@0")))),
            bx(BoundedExpr::Implies(bx(v("req@1")), bx(v("ack@1")))),
        );
        assert_eq!(first_antecedent(&prop), Some(&v("req@0")));
        // A pure safety property (no implication) has no trigger.
        let safety = BoundedExpr::Not(bx(BoundedExpr::And(bx(v("a@0")), bx(v("b@0")))));
        assert_eq!(first_antecedent(&safety), None);
    }
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
