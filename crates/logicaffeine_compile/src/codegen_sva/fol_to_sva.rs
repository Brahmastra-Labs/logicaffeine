//! FOL → SVA Formal Synthesis
//!
//! Pattern-matches Kripke-lowered FOL structures to synthesize
//! SystemVerilog Assertions. The key patterns:
//!
//! | Kripke Pattern | SVA Output |
//! |---|---|
//! | `∀w'(Accessible_Temporal → P(w'))` | `assert property(@(posedge clk) P)` |
//! | `∃w'(Reachable_Temporal ∧ P(w'))` | `cover property(s_eventually(P))` |
//! | `∀w'(Next_Temporal → P(w'))` | `nexttime(P)` |
//! | User `If`: `P → Q` with worlds | `P \|-> Q` |
//! | `¬(P ∧ Q)` with worlds | `!(P && Q)` |

use logicaffeine_language::ast::logic::{LogicExpr, QuantifierKind, TemporalOperator, ThematicRole, Term};
use logicaffeine_language::token::TokenType;
use logicaffeine_language::Interner;

/// Result of SVA synthesis from a specification.
#[derive(Debug)]
pub struct SynthesizedSva {
    /// Full SVA text including property wrapper and clock.
    pub sva_text: String,
    /// The SVA body expression (without property/assert wrapper).
    pub body: String,
    /// Signal names extracted from the specification.
    pub signals: Vec<String>,
    /// The assertion kind (assert/cover/assume).
    pub kind: String,
}

/// Synthesize an SVA property from an English specification.
///
/// Single-property convenience entry. Rejects multi-property specs with an
/// explicit error so callers do not silently lose properties after index 0 —
/// use [`synthesize_sva_from_hwspec`] when multiple properties need SVA.
///
/// The synthesized SVA uses the EXACT same signal names as the FOL
/// translator so Z3 equivalence checking works.
pub fn synthesize_sva_from_spec(spec: &str, clock: &str) -> Result<SynthesizedSva, String> {
    use logicaffeine_language::hw_spec::parse_hw_spec_with;

    parse_hw_spec_with(spec, |hw_spec, interner| {
        match hw_spec.properties.len() {
            0 => Err("No property sentence in spec".to_string()),
            1 => synthesize_sva_from_logic_expr(hw_spec.properties[0], interner, clock),
            n => Err(format!(
                "Spec has {} properties; synthesize_sva_from_spec only handles \
                 single-property specs. Call synthesize_sva_from_hwspec per \
                 property index instead.",
                n
            )),
        }
    })
    .map_err(|e| format!("Parse error: {:?}", e))?
}

/// Synthesize SVA from an already-parsed [`HwSpec`] at a specific property index.
///
/// Use this when a caller parsed a multi-property spec via
/// [`parse_hw_spec_with`] and needs SVA for each property. Returns an error
/// if `property_index` is out of range.
pub fn synthesize_sva_from_hwspec(
    hw_spec: &logicaffeine_language::hw_spec::HwSpec,
    interner: &Interner,
    property_index: usize,
    clock: &str,
) -> Result<SynthesizedSva, String> {
    let ast = hw_spec
        .properties
        .get(property_index)
        .copied()
        .ok_or_else(|| format!(
            "Property index {} out of bounds (spec has {} properties)",
            property_index,
            hw_spec.properties.len()
        ))?;
    synthesize_sva_from_logic_expr(ast, interner, clock)
}

fn synthesize_sva_from_logic_expr<'a>(
    ast: &'a LogicExpr<'a>,
    interner: &Interner,
    clock: &str,
) -> Result<SynthesizedSva, String> {
    use logicaffeine_language::semantics::knowledge_graph::extract_from_kripke_ast;
    use super::fol_to_verify::FolTranslator;
    use super::sva_to_verify::extract_signal_names;

    let mut fol_translator = FolTranslator::new(interner, 5);
    let fol_result = fol_translator.translate_property(ast);
    let fol_signals = extract_signal_names(&fol_result);

    let kg = extract_from_kripke_ast(ast, interner);
    let kg_signals: Vec<String> = kg.signals.iter().map(|s| s.name.clone()).collect();

    let body = synthesize_from_ast(ast, interner, clock, &fol_signals);

    // Reject degenerate synthesis results — these indicate the spec is not
    // a temporal property (e.g., bare action sentences like "The bus acknowledges the request.")
    if body.trim() == "0" {
        return Err("Not a temporal property: this sentence describes an action or event, \
            not a verifiable hardware property. Wrap in a temporal operator \
            (e.g., \"Always, ...\") or restructure as a conditional.".to_string());
    }

    let kind = if body.contains("s_eventually") || body.contains("cover") {
        "cover"
    } else {
        "assert"
    };

    let sva_text = format!("{} property (@(posedge {}) {});", kind, clock, body);

    Ok(SynthesizedSva {
        sva_text,
        body,
        signals: if kg_signals.is_empty() { fol_signals } else { kg_signals },
        kind: kind.to_string(),
    })
}

/// Synthesize SVA body from a Kripke-lowered AST node.
/// Uses `fol_signals` (the signal names the FOL translator produces) to ensure
/// the synthesized SVA uses matching variable names for Z3 equivalence.
fn synthesize_from_ast<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &Interner,
    clock: &str,
    fol_signals: &[String],
) -> String {
    match expr {
        // Temporal unary: G(P) → P, F(P) → s_eventually(P), X(P) → nexttime(P)
        LogicExpr::Temporal { operator, body } => {
            let inner = synthesize_from_ast(body, interner, clock, fol_signals);
            match operator {
                TemporalOperator::Always => inner, // G is implicit in assert property
                TemporalOperator::Eventually => format!("s_eventually({})", inner),
                TemporalOperator::Next => format!("nexttime({})", inner),
                TemporalOperator::BoundedEventually(n) => format!("##[0:{}] {}", n, inner),
                _ => inner,
            }
        }

        // Kripke-lowered G: ∀w'(Accessible_Temporal(w,w') → P(w'))
        // Kripke-lowered X: ∀w'(Next_Temporal(w,w') → P(w'))
        LogicExpr::Quantifier { kind: QuantifierKind::Universal, body, variable, .. } => {
            let var_name = interner.resolve(*variable).to_string();
            if var_name.starts_with('w') {
                if let LogicExpr::BinaryOp { left, right, op: TokenType::Implies } = body {
                    if is_accessibility_predicate(left, interner) {
                        let inner = synthesize_from_ast(right, interner, clock, fol_signals);
                        // Distinguish Next_Temporal → nexttime(P) vs Accessible → P
                        if is_next_temporal_predicate(left, interner) {
                            return format!("nexttime({})", inner);
                        }
                        return inner;
                    }
                }
            }
            // Regular quantifier — synthesize body
            synthesize_from_ast(body, interner, clock, fol_signals)
        }

        // Kripke-lowered F: ∃w'(Reachable_Temporal(w,w') ∧ P(w'))
        LogicExpr::Quantifier { kind: QuantifierKind::Existential, body, variable, .. } => {
            let var_name = interner.resolve(*variable).to_string();
            if var_name.starts_with('w') {
                if let LogicExpr::BinaryOp { left, right, op: TokenType::And } = body {
                    if is_accessibility_predicate(left, interner) {
                        return format!("s_eventually({})", synthesize_from_ast(right, interner, clock, fol_signals));
                    }
                }
            }
            synthesize_from_ast(body, interner, clock, fol_signals)
        }

        // Counting quantifiers: AtMost(n), AtLeast(n), Cardinal(n)
        LogicExpr::Quantifier { kind: QuantifierKind::AtMost(n), body, .. } => {
            let inner = synthesize_from_ast(body, interner, clock, fol_signals);
            if *n == 1 {
                format!("$onehot0({})", inner)
            } else {
                format!("($countones({}) <= {})", inner, n)
            }
        }

        LogicExpr::Quantifier { kind: QuantifierKind::AtLeast(n), body, .. } => {
            let inner = synthesize_from_ast(body, interner, clock, fol_signals);
            if *n == 1 {
                inner // at least one → signal is high (OR-reduction implicit)
            } else {
                format!("($countones({}) >= {})", inner, n)
            }
        }

        LogicExpr::Quantifier { kind: QuantifierKind::Cardinal(n), body, .. } => {
            let inner = synthesize_from_ast(body, interner, clock, fol_signals);
            if *n == 1 {
                format!("$onehot({})", inner)
            } else {
                format!("($countones({}) == {})", inner, n)
            }
        }

        // Other quantifier kinds (Most, Few, Many, Generic) — synthesize body
        LogicExpr::Quantifier { body, .. } => {
            synthesize_from_ast(body, interner, clock, fol_signals)
        }

        // User conditional: P → Q (TokenType::If from parser)
        LogicExpr::BinaryOp { left, right, op: TokenType::If } => {
            let ante = synthesize_from_ast(left, interner, clock, fol_signals);
            let cons = synthesize_from_ast(right, interner, clock, fol_signals);
            format!("{} |-> {}", ante, cons)
        }

        // Compiler-generated implication (restriction): synthesize as SVA implication
        // ∀x(Restriction(x) → Body(x)) → restriction |-> body
        // This preserves the full semantic content for Z3 equivalence checking.
        LogicExpr::BinaryOp { left, right, op: TokenType::Implies } => {
            let ante = synthesize_from_ast(left, interner, clock, fol_signals);
            let cons = synthesize_from_ast(right, interner, clock, fol_signals);
            // If the antecedent is just "1" (vacuous), skip the implication
            if ante == "1" {
                cons
            } else {
                format!("(!({}) || ({}))", ante, cons)
            }
        }

        // Conjunction
        LogicExpr::BinaryOp { left, right, op: TokenType::And } => {
            let l = synthesize_from_ast(left, interner, clock, fol_signals);
            let r = synthesize_from_ast(right, interner, clock, fol_signals);
            format!("({} && {})", l, r)
        }

        // Disjunction
        LogicExpr::BinaryOp { left, right, op: TokenType::Or } => {
            let l = synthesize_from_ast(left, interner, clock, fol_signals);
            let r = synthesize_from_ast(right, interner, clock, fol_signals);
            format!("({} || {})", l, r)
        }

        // Negation
        LogicExpr::UnaryOp { operand, .. } => {
            let inner = synthesize_from_ast(operand, interner, clock, fol_signals);
            format!("!({})", inner)
        }

        // Predicate: map to the FOL signal name so Z3 sees matching variables
        LogicExpr::Predicate { name, args, .. } => {
            let pred_name = interner.resolve(*name).to_string();
            // Skip meta-predicates
            if pred_name.contains("Accessible") || pred_name.contains("Reachable")
                || pred_name.contains("Next_Temporal")
                || pred_name == "Agent" || pred_name == "Theme"
            {
                return "1".to_string(); // vacuously true
            }
            // Build precise candidate: PredName_argName_ (matches FolTranslator naming)
            let arg_name = args.first().map(|a| term_to_string_helper(a, interner));
            if let Some(ref arg) = arg_name {
                let candidate = format!("{}_{}_", pred_name, arg);
                if let Some(fol_sig) = fol_signals.iter().find(|s| {
                    s.to_lowercase() == candidate.to_lowercase()
                }) {
                    return fol_sig.clone();
                }
            }
            // Fallback: fuzzy match on predicate name
            if let Some(fol_sig) = fol_signals.iter().find(|s| {
                let s_lower = s.to_lowercase();
                s_lower.contains(&pred_name.to_lowercase())
                    || pred_name.to_lowercase().contains(&s_lower)
            }) {
                fol_sig.clone()
            } else {
                pred_name.to_lowercase()
            }
        }

        // NeoEvent: extract verb + agent as signal (matching FolTranslator naming)
        LogicExpr::NeoEvent(data) => {
            let verb_name = interner.resolve(data.verb).to_string();
            let agent_name = data.roles.iter()
                .find(|(role, _)| matches!(role, ThematicRole::Agent))
                .map(|(_, term)| term_to_string_helper(term, interner));

            let candidate = if let Some(ref arg) = agent_name {
                format!("{}_{}_", verb_name, arg)
            } else {
                verb_name.clone()
            };

            // Match against fol_signals for consistency
            if let Some(fol_sig) = fol_signals.iter().find(|s| {
                s.to_lowercase() == candidate.to_lowercase()
            }) {
                fol_sig.clone()
            } else if let Some(fol_sig) = fol_signals.iter().find(|s| {
                let s_lower = s.to_lowercase();
                s_lower.contains(&verb_name.to_lowercase())
            }) {
                fol_sig.clone()
            } else {
                candidate
            }
        }

        // Temporal binary
        LogicExpr::TemporalBinary { operator, left, right } => {
            let l = synthesize_from_ast(left, interner, clock, fol_signals);
            let r = synthesize_from_ast(right, interner, clock, fol_signals);
            use logicaffeine_language::ast::logic::BinaryTemporalOp;
            match operator {
                BinaryTemporalOp::Until => format!("({} until {})", l, r),
                BinaryTemporalOp::Release => format!("({} release {})", l, r),
                BinaryTemporalOp::WeakUntil => format!("({} weak_until {})", l, r),
            }
        }

        // Modal: unwrap
        LogicExpr::Modal { operand, .. } => {
            synthesize_from_ast(operand, interner, clock, fol_signals)
        }

        // Aspectual: HAB(P), PROG(P), PERF(P), ITER(P) → unwrap to body
        // In hardware context, habitual aspect means "P holds generally"
        LogicExpr::Aspectual { body, .. } => {
            synthesize_from_ast(body, interner, clock, fol_signals)
        }

        // Voice: PASSIVE(P) → unwrap to body
        LogicExpr::Voice { body, .. } => {
            synthesize_from_ast(body, interner, clock, fol_signals)
        }

        // Relation: S-V-O → map verb and subject/object to signal names
        LogicExpr::Relation(data) => {
            let verb_name = interner.resolve(data.verb).to_string();
            let subj_name = interner.resolve(data.subject.noun).to_string();
            let obj_name = interner.resolve(data.object.noun).to_string();
            let candidate = format!("{}_{}_", verb_name, subj_name);
            if let Some(fol_sig) = fol_signals.iter().find(|s| {
                s.to_lowercase() == candidate.to_lowercase()
            }) {
                fol_sig.clone()
            } else if let Some(fol_sig) = fol_signals.iter().find(|s| {
                let s_lower = s.to_lowercase();
                s_lower.contains(&verb_name.to_lowercase())
                    || s_lower.contains(&subj_name.to_lowercase())
                    || s_lower.contains(&obj_name.to_lowercase())
            }) {
                fol_sig.clone()
            } else {
                format!("{}_{}_", verb_name, obj_name).to_lowercase()
            }
        }

        // Categorical: Aristotelian A/E/I/O → synthesize subject and predicate
        LogicExpr::Categorical(data) => {
            let subj_name = interner.resolve(data.subject.noun).to_string().to_lowercase();
            let pred_name = interner.resolve(data.predicate.noun).to_string().to_lowercase();
            if data.copula_negative {
                format!("({} && !({}))", subj_name, pred_name)
            } else {
                format!("(!({}) || ({}))", subj_name, pred_name)
            }
        }

        // Scopal: "only X", "always X" as scopal adverb → unwrap to body
        LogicExpr::Scopal { body, .. } => {
            synthesize_from_ast(body, interner, clock, fol_signals)
        }

        // Causal: "effect because cause" → both sides as conjunction
        LogicExpr::Causal { effect, cause } => {
            let e = synthesize_from_ast(effect, interner, clock, fol_signals);
            let c = synthesize_from_ast(cause, interner, clock, fol_signals);
            format!("({} && {})", c, e)
        }

        // Atom: bare symbol → treat as signal name
        LogicExpr::Atom(sym) => {
            let name = interner.resolve(*sym).to_string();
            if let Some(fol_sig) = fol_signals.iter().find(|s| {
                s.to_lowercase() == name.to_lowercase()
            }) {
                fol_sig.clone()
            } else {
                name.to_lowercase()
            }
        }

        // Identity: t1 = t2 → equality check
        LogicExpr::Identity { left, right } => {
            let l = term_to_string_helper(left, interner).to_lowercase();
            let r = term_to_string_helper(right, interner).to_lowercase();
            format!("({} == {})", l, r)
        }

        // Default: fail closed. Unhandled FOL patterns must NOT silently
        // become vacuously true in synthesized SVA (Sprint 0A consistency).
        _ => "0".to_string(),
    }
}

/// Check if an expression is an accessibility predicate (Accessible_Temporal, Reachable_Temporal, etc.).
fn is_accessibility_predicate<'a>(expr: &'a LogicExpr<'a>, interner: &Interner) -> bool {
    if let LogicExpr::Predicate { name, .. } = expr {
        let pred_name = interner.resolve(*name).to_string();
        pred_name.contains("Accessible") || pred_name.contains("Reachable") || pred_name.contains("Next_Temporal")
    } else {
        false
    }
}

/// Check if an expression is specifically Next_Temporal (not Accessible or Reachable).
fn is_next_temporal_predicate<'a>(expr: &'a LogicExpr<'a>, interner: &Interner) -> bool {
    if let LogicExpr::Predicate { name, .. } = expr {
        let pred_name = interner.resolve(*name).to_string();
        pred_name.contains("Next_Temporal")
    } else {
        false
    }
}

/// Helper to extract a string from a Term for signal naming.
fn term_to_string_helper<'a>(term: &'a Term<'a>, interner: &Interner) -> String {
    match term {
        Term::Constant(sym) | Term::Variable(sym) => interner.resolve(*sym).to_string(),
        Term::Function(sym, _) => interner.resolve(*sym).to_string(),
        _ => "unknown".to_string(),
    }
}
