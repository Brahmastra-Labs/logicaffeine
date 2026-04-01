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
///
/// Signal roles are inferred from structural position:
/// - Antecedent-only → Input
/// - Consequent-only → Output
/// - Both positions → Internal
/// - Name contains "clk"/"clock" → Clock (overrides position)
pub fn extract_from_kripke_ast<'a>(
    expr: &'a crate::ast::logic::LogicExpr<'a>,
    interner: &crate::Interner,
) -> HwKnowledgeGraph {
    use crate::ast::logic::{LogicExpr, QuantifierKind};
    use std::collections::{HashSet, HashMap};

    let mut kg = HwKnowledgeGraph::new();
    let mut seen_signals: HashSet<String> = HashSet::new();
    // Track signal positions for role inference
    let mut antecedent_signals: HashSet<String> = HashSet::new();
    let mut consequent_signals: HashSet<String> = HashSet::new();
    // Track predicate names for property naming
    let mut predicate_names: Vec<String> = Vec::new();

    /// Position context for signal role inference.
    #[derive(Clone, Copy, PartialEq)]
    enum Position {
        Neutral,
        Antecedent,
        Consequent,
    }

    fn walk<'a>(
        expr: &'a LogicExpr<'a>,
        interner: &crate::Interner,
        kg: &mut HwKnowledgeGraph,
        seen: &mut HashSet<String>,
        antecedent: &mut HashSet<String>,
        consequent: &mut HashSet<String>,
        pred_names: &mut Vec<String>,
        in_safety: bool,
        in_liveness: bool,
        position: Position,
        impl_depth: u32,
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

                // Collect predicate names for property naming
                if !pred_name.starts_with('w') {
                    pred_names.push(pred_name.clone());
                }

                // Predicates with world args are signals.
                // Use both the predicate name and non-world arguments as signal names.
                if world.is_some() {
                    // The predicate name itself is a useful signal identifier
                    let pred_lower = pred_name.to_lowercase();
                    if !pred_lower.is_empty() {
                        seen.insert(pred_name.clone());
                        match position {
                            Position::Antecedent => { antecedent.insert(pred_name.clone()); }
                            Position::Consequent => { consequent.insert(pred_name.clone()); }
                            Position::Neutral => {}
                        }
                    }

                    for arg in args.iter() {
                        if let crate::ast::logic::Term::Constant(sym)
                            | crate::ast::logic::Term::Variable(sym) = arg
                        {
                            let arg_name = interner.resolve(*sym).to_string();
                            // Skip world variables (w0, w1, ...) and single-letter bound vars
                            if (!arg_name.starts_with('w') || arg_name.len() > 3)
                                && arg_name.len() > 1
                            {
                                seen.insert(arg_name.clone());
                                match position {
                                    Position::Antecedent => { antecedent.insert(arg_name); }
                                    Position::Consequent => { consequent.insert(arg_name); }
                                    Position::Neutral => {}
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
                walk(body, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety || is_temporal_world, in_liveness, position, impl_depth);
            }

            // Existential quantifier with temporal reachability → liveness property
            LogicExpr::Quantifier { kind: QuantifierKind::Existential, variable, body, .. } => {
                let var_name = interner.resolve(*variable).to_string();
                let is_temporal_world = var_name.starts_with('w');
                walk(body, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness || is_temporal_world, position, impl_depth);
            }

            // Binary connectives: walk both sides
            LogicExpr::BinaryOp { left, right, op } => {
                // TokenType::If = user-written conditional (provenance tag).
                // TokenType::Implies = compiler-generated restriction/accessibility.
                // Only user conditionals determine antecedent/consequent roles.
                if matches!(op, crate::token::TokenType::If) {
                    // User's explicit if...then — sets signal positions
                    walk(left, interner, kg, seen, antecedent, consequent, pred_names,
                         in_safety, in_liveness, Position::Antecedent, impl_depth);
                    walk(right, interner, kg, seen, antecedent, consequent, pred_names,
                         in_safety, in_liveness, Position::Consequent, impl_depth);

                    // Triggers edge
                    if let (Some(left_sig), Some(right_sig)) =
                        (extract_signal_name(left, interner), extract_signal_name(right, interner))
                    {
                        if left_sig != right_sig {
                            kg.add_edge(&left_sig, &right_sig, KgRelation::Triggers, None);
                        }
                    }
                } else {
                    // Compiler-generated Implies or other connectives — preserve position
                    walk(left, interner, kg, seen, antecedent, consequent, pred_names,
                         in_safety, in_liveness, position, impl_depth);
                    walk(right, interner, kg, seen, antecedent, consequent, pred_names,
                         in_safety, in_liveness, position, impl_depth);
                }
            }

            // Negation
            LogicExpr::UnaryOp { operand, .. } => {
                walk(operand, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness, position, impl_depth);

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
                walk(body, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness, position, impl_depth);
            }

            LogicExpr::TemporalBinary { left, right, .. } => {
                walk(left, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness, position, impl_depth);
                walk(right, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness, position, impl_depth);
            }

            // Modal operators
            LogicExpr::Modal { operand, .. } => {
                walk(operand, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness, position, impl_depth);
            }

            _ => {}
        }
    }

    walk(expr, interner, &mut kg, &mut seen_signals,
         &mut antecedent_signals, &mut consequent_signals, &mut predicate_names,
         false, false, Position::Neutral, 0);

    // Assign roles based on position inference
    for sig_name in &seen_signals {
        let in_ante = antecedent_signals.contains(sig_name);
        let in_cons = consequent_signals.contains(sig_name);
        let name_lower = sig_name.to_lowercase();

        let role = if name_lower.contains("clk") || name_lower.contains("clock") {
            SignalRole::Clock
        } else if in_ante && !in_cons {
            SignalRole::Input
        } else if in_cons && !in_ante {
            SignalRole::Output
        } else {
            SignalRole::Internal
        };

        kg.add_signal(sig_name, 1, role);
    }

    // Determine property type and name from the top-level structure
    // Use predicate names for descriptive property naming
    let prop_name = predicate_names.iter()
        .find(|n| {
            let lower = n.to_lowercase();
            !lower.contains("accessible") && !lower.contains("reachable")
                && !lower.contains("next_temporal") && lower != "and" && lower != "or"
        })
        .cloned();

    match expr {
        LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Always, .. } => {
            let name = prop_name.unwrap_or_else(|| "Safety".to_string());
            kg.add_property(name, "safety", "G(...)");
        }
        LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Eventually, .. } => {
            let name = prop_name.unwrap_or_else(|| "Liveness".to_string());
            kg.add_property(name, "liveness", "F(...)");
        }
        LogicExpr::Quantifier { kind: QuantifierKind::Universal, .. } => {
            let name = prop_name.unwrap_or_else(|| "Safety".to_string());
            kg.add_property(name, "safety", "G(...)");
        }
        LogicExpr::Quantifier { kind: QuantifierKind::Existential, .. } => {
            let name = prop_name.unwrap_or_else(|| "Liveness".to_string());
            kg.add_property(name, "liveness", "F(...)");
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
