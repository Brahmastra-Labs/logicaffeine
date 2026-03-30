//! Knowledge Graph extraction for hardware verification.
//!
//! Extracts a structured Knowledge Graph from parsed hardware specifications,
//! providing LLMs with formally grounded context for SVA generation.

/// Role of a hardware signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalRole {
    Input,
    Output,
    Internal,
    Clock,
}

/// A signal node in the Knowledge Graph.
#[derive(Debug, Clone)]
pub struct KgSignal {
    pub name: String,
    pub width: u32,
    pub role: SignalRole,
}

/// A temporal property node.
#[derive(Debug, Clone)]
pub struct KgProperty {
    pub name: String,
    pub property_type: String,
    pub operator: String,
}

/// Relation type for KG edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KgRelation {
    Temporal,
    Triggers,
    Constrains,
    TypeOf,
}

/// An edge in the Knowledge Graph.
#[derive(Debug, Clone)]
pub struct KgEdge {
    pub from: String,
    pub to: String,
    pub relation: KgRelation,
    pub property: Option<String>,
}

/// Hardware Knowledge Graph — extracted from Kripke-lowered FOL.
#[derive(Debug, Clone)]
pub struct HwKnowledgeGraph {
    pub signals: Vec<KgSignal>,
    pub properties: Vec<KgProperty>,
    pub edges: Vec<KgEdge>,
}

impl HwKnowledgeGraph {
    pub fn new() -> Self {
        Self {
            signals: Vec::new(),
            properties: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Serialize the KG to JSON for LLM consumption.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");

        // Signals
        out.push_str("  \"signals\": [\n");
        for (i, sig) in self.signals.iter().enumerate() {
            let role = match &sig.role {
                SignalRole::Input => "input",
                SignalRole::Output => "output",
                SignalRole::Internal => "internal",
                SignalRole::Clock => "clock",
            };
            out.push_str(&format!(
                "    {{\"name\": \"{}\", \"width\": {}, \"role\": \"{}\"}}",
                sig.name, sig.width, role
            ));
            if i < self.signals.len() - 1 {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ],\n");

        // Properties
        out.push_str("  \"properties\": [\n");
        for (i, prop) in self.properties.iter().enumerate() {
            out.push_str(&format!(
                "    {{\"name\": \"{}\", \"type\": \"{}\", \"operator\": \"{}\"}}",
                prop.name, prop.property_type, prop.operator
            ));
            if i < self.properties.len() - 1 {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ],\n");

        // Edges
        out.push_str("  \"edges\": [\n");
        for (i, edge) in self.edges.iter().enumerate() {
            let rel = match &edge.relation {
                KgRelation::Temporal => "temporal",
                KgRelation::Triggers => "triggers",
                KgRelation::Constrains => "constrains",
                KgRelation::TypeOf => "type_of",
            };
            let prop = edge
                .property
                .as_deref()
                .map(|p| format!(", \"property\": \"{}\"", p))
                .unwrap_or_default();
            out.push_str(&format!(
                "    {{\"from\": \"{}\", \"to\": \"{}\", \"relation\": \"{}\"{}}}",
                edge.from, edge.to, rel, prop
            ));
            if i < self.edges.len() - 1 {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ]\n");

        out.push('}');
        out
    }

    pub fn add_signal(&mut self, name: impl Into<String>, width: u32, role: SignalRole) {
        self.signals.push(KgSignal {
            name: name.into(),
            width,
            role,
        });
    }

    pub fn add_property(
        &mut self,
        name: impl Into<String>,
        property_type: impl Into<String>,
        operator: impl Into<String>,
    ) {
        self.properties.push(KgProperty {
            name: name.into(),
            property_type: property_type.into(),
            operator: operator.into(),
        });
    }

    pub fn add_edge(
        &mut self,
        from: impl Into<String>,
        to: impl Into<String>,
        relation: KgRelation,
        property: Option<String>,
    ) {
        self.edges.push(KgEdge {
            from: from.into(),
            to: to.into(),
            relation,
            property,
        });
    }
}

impl Default for HwKnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract a Knowledge Graph from a Kripke-lowered LogicExpr AST.
///
/// Walks the AST to identify:
/// - Signals: predicates that appear with world arguments
/// - Properties: temporal patterns (∀w' Accessible_Temporal → ...) → safety, (∃w' Reachable → ...) → liveness
/// - Edges: implication between predicates → Triggers, negated conjunction → Constrains
pub fn extract_from_kripke_ast<'a>(
    expr: &'a crate::ast::logic::LogicExpr<'a>,
    interner: &crate::Interner,
) -> HwKnowledgeGraph {
    use crate::ast::logic::{LogicExpr, QuantifierKind};
    use std::collections::HashSet;

    let mut kg = HwKnowledgeGraph::new();
    let mut seen_signals: HashSet<String> = HashSet::new();

    fn walk<'a>(
        expr: &'a LogicExpr<'a>,
        interner: &crate::Interner,
        kg: &mut HwKnowledgeGraph,
        seen: &mut HashSet<String>,
        in_safety: bool,
        in_liveness: bool,
    ) {
        match expr {
            LogicExpr::Predicate { name, args, world } => {
                let pred_name = interner.resolve(*name).to_string();

                // Skip accessibility predicates themselves
                if pred_name.contains("Accessible") || pred_name.contains("Reachable")
                    || pred_name.contains("Next_Temporal")
                {
                    return;
                }

                // Predicates with world args are signals
                if world.is_some() {
                    for arg in args.iter() {
                        if let crate::ast::logic::Term::Constant(sym)
                            | crate::ast::logic::Term::Variable(sym) = arg
                        {
                            let arg_name = interner.resolve(*sym).to_string();
                            // Skip world variables (w0, w1, ...)
                            if !arg_name.starts_with('w') || arg_name.len() > 3 {
                                if seen.insert(arg_name.clone()) {
                                    kg.add_signal(&arg_name, 1, SignalRole::Internal);
                                }
                            }
                        }
                    }
                }
            }

            // Universal quantifier with temporal accessibility → safety property
            LogicExpr::Quantifier { kind: QuantifierKind::Universal, variable, body, .. } => {
                let var_name = interner.resolve(*variable).to_string();
                let is_temporal_world = var_name.starts_with('w');
                walk(body, interner, kg, seen, in_safety || is_temporal_world, in_liveness);
            }

            // Existential quantifier with temporal reachability → liveness property
            LogicExpr::Quantifier { kind: QuantifierKind::Existential, variable, body, .. } => {
                let var_name = interner.resolve(*variable).to_string();
                let is_temporal_world = var_name.starts_with('w');
                walk(body, interner, kg, seen, in_safety, in_liveness || is_temporal_world);
            }

            // Binary connectives: walk both sides
            LogicExpr::BinaryOp { left, right, op } => {
                walk(left, interner, kg, seen, in_safety, in_liveness);
                walk(right, interner, kg, seen, in_safety, in_liveness);

                // Implication between predicates → Triggers edge
                if matches!(op, crate::token::TokenType::If) {
                    if let (Some(left_sig), Some(right_sig)) =
                        (extract_signal_name(left, interner), extract_signal_name(right, interner))
                    {
                        if left_sig != right_sig {
                            kg.add_edge(&left_sig, &right_sig, KgRelation::Triggers, None);
                        }
                    }
                }
            }

            // Negation
            LogicExpr::UnaryOp { operand, .. } => {
                walk(operand, interner, kg, seen, in_safety, in_liveness);

                // Not(And(P, Q)) → Constrains edge (mutex pattern)
                if let LogicExpr::BinaryOp { left, right, op: crate::token::TokenType::And } = operand {
                    if let (Some(left_sig), Some(right_sig)) =
                        (extract_signal_name(left, interner), extract_signal_name(right, interner))
                    {
                        if left_sig != right_sig {
                            kg.add_edge(&left_sig, &right_sig, KgRelation::Constrains, None);
                        }
                    }
                }
            }

            // Temporal operators
            LogicExpr::Temporal { body, .. } => {
                walk(body, interner, kg, seen, in_safety, in_liveness);
            }

            LogicExpr::TemporalBinary { left, right, .. } => {
                walk(left, interner, kg, seen, in_safety, in_liveness);
                walk(right, interner, kg, seen, in_safety, in_liveness);
            }

            // Modal operators
            LogicExpr::Modal { operand, .. } => {
                walk(operand, interner, kg, seen, in_safety, in_liveness);
            }

            _ => {}
        }
    }

    walk(expr, interner, &mut kg, &mut seen_signals, false, false);

    // Determine property type from the top-level structure
    match expr {
        LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Always, .. } => {
            kg.add_property("Safety", "safety", "G(...)");
        }
        LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Eventually, .. } => {
            kg.add_property("Liveness", "liveness", "F(...)");
        }
        LogicExpr::Quantifier { kind: QuantifierKind::Universal, .. } => {
            // Kripke-lowered G produces ∀w'(...)
            kg.add_property("Safety", "safety", "G(...)");
        }
        LogicExpr::Quantifier { kind: QuantifierKind::Existential, .. } => {
            kg.add_property("Liveness", "liveness", "F(...)");
        }
        _ => {}
    }

    kg
}

/// Try to extract a signal name from a predicate expression.
fn extract_signal_name<'a>(
    expr: &'a crate::ast::logic::LogicExpr<'a>,
    interner: &crate::Interner,
) -> Option<String> {
    match expr {
        crate::ast::logic::LogicExpr::Predicate { args, .. } => {
            args.first().and_then(|arg| match arg {
                crate::ast::logic::Term::Constant(sym)
                | crate::ast::logic::Term::Variable(sym) => {
                    let name = interner.resolve(*sym).to_string();
                    if !name.starts_with('w') || name.len() > 3 {
                        Some(name)
                    } else {
                        None
                    }
                }
                _ => None,
            })
        }
        _ => None,
    }
}
