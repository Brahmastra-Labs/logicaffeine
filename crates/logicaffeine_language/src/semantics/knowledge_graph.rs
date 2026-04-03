//! Knowledge Graph extraction for hardware verification.
//!
//! Extracts a structured Knowledge Graph from parsed hardware specifications,
//! providing LLMs with formally grounded context for SVA generation.
//!
//! ## Ontology (Sprint 0E)
//!
//! The formal hardware ontology provides 28+ entity types and 24+ relation types,
//! replacing AssertionForge's 35 LLM-prompt labels and 59 string labels with
//! formally grounded, parameterized, serializable types.

use serde::{Serialize, Deserialize};

// ═══════════════════════════════════════════════════════════════════════════
// HELPER ENUMS FOR ENTITY PARAMETERS
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortDirection { Input, Output, Inout }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalType { Wire, Reg, Logic }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetPolarity { ActiveHigh, ActiveLow }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CounterDirection { Up, Down, UpDown }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArbitrationScheme { RoundRobin, Priority, WeightedRoundRobin }

// ═══════════════════════════════════════════════════════════════════════════
// FORMAL HARDWARE ONTOLOGY — 28+ ENTITY TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Formal hardware entity types with parameterized attributes.
///
/// 28 variants organized into 6 categories:
/// - **Structural (8)**: Module, Port, Signal, Register, Memory, Fifo, Bus, Parameter
/// - **Control (5)**: Fsm, Counter, Arbiter, Decoder, Mux
/// - **Temporal (3)**: Clock, Reset, Interrupt
/// - **Protocol (3)**: Handshake, Pipeline, Transaction
/// - **Data (3)**: DataPath, Address, Configuration
/// - **Property (6)**: SafetyProperty, LivenessProperty, FairnessProperty,
///   ResponseProperty, MutexProperty, StabilityProperty
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HwEntityType {
    // Structural (8)
    Module { name: String, is_top: bool },
    Port { direction: PortDirection, width: u32, domain: Option<String> },
    Signal { width: u32, signal_type: SignalType, domain: Option<String> },
    Register { width: u32, reset_value: Option<u64>, clock: Option<String> },
    Memory { depth: u32, width: u32, ports: u8 },
    Fifo { depth: u32, width: u32 },
    Bus { width: u32, protocol: Option<String> },
    Parameter { value: String },

    // Control (5)
    Fsm { states: Vec<String>, initial: Option<String> },
    Counter { width: u32, direction: CounterDirection },
    Arbiter { scheme: ArbitrationScheme, ports: u8 },
    Decoder { input_width: u32, output_width: u32 },
    Mux { inputs: u8, select_width: u32 },

    // Temporal (3)
    Clock { frequency: Option<String>, domain: String },
    Reset { polarity: ResetPolarity, synchronous: bool },
    Interrupt { priority: Option<u8>, edge_triggered: bool },

    // Protocol (3)
    Handshake { valid_signal: String, ready_signal: String },
    Pipeline { stages: u32, stall_signal: Option<String> },
    Transaction { request: String, response: String },

    // Data (3)
    DataPath { width: u32, signed: bool },
    Address { width: u32, base: Option<u64>, range: Option<u64> },
    Configuration { fields: Vec<String> },

    // Property (6)
    SafetyProperty { formula: String },
    LivenessProperty { formula: String },
    FairnessProperty { formula: String },
    ResponseProperty { trigger: String, response: String, bound: Option<u32> },
    MutexProperty { signals: Vec<String> },
    StabilityProperty { signal: String, condition: String },
}

// ═══════════════════════════════════════════════════════════════════════════
// FORMAL HARDWARE ONTOLOGY — 24+ RELATION TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Formal hardware relation types with parameterized attributes.
///
/// 24 variants organized into 6 categories:
/// - **Data Flow (5)**: Drives, DrivesRegistered, DataFlow, Reads, Writes
/// - **Control Flow (4)**: Controls, Selects, Enables, Resets
/// - **Temporal (5)**: Triggers, Constrains, Follows, Precedes, Preserves
/// - **Structural (4)**: Contains, Instantiates, ConnectsTo, BelongsToDomain
/// - **Protocol (3)**: HandshakesWith, Acknowledges, Pipelines
/// - **Specification (3)**: MutuallyExcludes, EventuallyFollows, AssumedBy
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HwRelation {
    // Data Flow (5)
    Drives,
    DrivesRegistered { clock: String },
    DataFlow,
    Reads,
    Writes,
    // Control Flow (4)
    Controls,
    Selects,
    Enables,
    Resets,
    // Temporal (5)
    Triggers { delay: Option<u32> },
    Constrains,
    Follows { min: u32, max: u32 },
    Precedes,
    Preserves,
    // Structural (4)
    Contains,
    Instantiates,
    ConnectsTo,
    BelongsToDomain { domain: String },
    // Protocol (3)
    HandshakesWith,
    Acknowledges,
    Pipelines { stages: u32 },
    // Specification (3)
    MutuallyExcludes,
    EventuallyFollows,
    AssumedBy,
}

// ═══════════════════════════════════════════════════════════════════════════
// LEGACY TYPES (kept for backward compatibility with existing KG pipeline)
// ═══════════════════════════════════════════════════════════════════════════

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
    /// Typed entities from the formal ontology (Sprint 0E).
    pub entities: Vec<(String, HwEntityType)>,
    /// Typed relations from the formal ontology (Sprint 0E).
    pub typed_edges: Vec<(String, String, HwRelation)>,
}

impl HwKnowledgeGraph {
    pub fn new() -> Self {
        Self {
            signals: Vec::new(),
            properties: Vec::new(),
            edges: Vec::new(),
            entities: Vec::new(),
            typed_edges: Vec::new(),
        }
    }

    pub fn add_entity(&mut self, name: impl Into<String>, entity: HwEntityType) {
        self.entities.push((name.into(), entity));
    }

    pub fn add_typed_edge(&mut self, from: impl Into<String>, to: impl Into<String>, relation: HwRelation) {
        self.typed_edges.push((from.into(), to.into(), relation));
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
        out.push_str("  ],\n");

        // Typed entities (Sprint 0E ontology)
        out.push_str("  \"entities\": [\n");
        for (i, (name, entity)) in self.entities.iter().enumerate() {
            let entity_json = serde_json::to_string(entity).unwrap_or_else(|_| "{}".to_string());
            out.push_str(&format!(
                "    {{\"name\": \"{}\", \"entity_type\": {}}}",
                name, entity_json
            ));
            if i < self.entities.len() - 1 {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ],\n");

        // Typed edges (Sprint 0E ontology)
        out.push_str("  \"typed_edges\": [\n");
        for (i, (from, to, rel)) in self.typed_edges.iter().enumerate() {
            let rel_json = serde_json::to_string(rel).unwrap_or_else(|_| "{}".to_string());
            out.push_str(&format!(
                "    {{\"from\": \"{}\", \"to\": \"{}\", \"relation\": {}}}",
                from, to, rel_json
            ));
            if i < self.typed_edges.len() - 1 {
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

                    // Triggers edge — use hw-aware extraction that prefers restriction
                    // type names (Request, Grant) over verb predicates (Hold, Have)
                    if let (Some(left_sig), Some(right_sig)) =
                        (extract_hw_signal_name(left, interner), extract_hw_signal_name(right, interner))
                    {
                        if left_sig != right_sig {
                            kg.add_edge(&left_sig, &right_sig, KgRelation::Triggers, None);

                            // Sprint D: detect response pattern G(P → X(Q))
                            let is_next = matches!(right,
                                LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Next, .. }
                            ) || is_kripke_next(right, interner);
                            if is_next && in_safety {
                                kg.add_entity(
                                    format!("{}_responds_to_{}", right_sig, left_sig),
                                    HwEntityType::ResponseProperty {
                                        trigger: left_sig.clone(),
                                        response: right_sig.clone(),
                                        bound: Some(1),
                                    },
                                );
                                kg.add_typed_edge(&left_sig, &right_sig, HwRelation::Triggers { delay: Some(1) });
                            }

                            // Sprint D: detect eventually-follows G(P → F(Q))
                            let is_eventually = matches!(right,
                                LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Eventually, .. }
                            ) || is_kripke_eventually(right, interner);
                            if is_eventually && in_safety {
                                kg.add_typed_edge(&left_sig, &right_sig, HwRelation::EventuallyFollows);
                            }
                        }
                    }
                } else if matches!(op, crate::token::TokenType::Implies) {
                    // Compiler-generated Implies — could be accessibility unwrap OR user conditional.
                    // If left is an accessibility predicate, this is Kripke structure (skip edge extraction).
                    // If left is a real predicate, this is a user conditional (extract edges).
                    let left_is_accessibility = if let LogicExpr::Predicate { name, .. } = left {
                        let pn = interner.resolve(*name).to_string();
                        pn.contains("Accessible") || pn.contains("Reachable") || pn.contains("Next_Temporal")
                    } else { false };

                    if !left_is_accessibility {
                        // User conditional lowered to Implies — extract edges but preserve
                        // position as Neutral to avoid misclassifying restriction types
                        // (Animal, Mammal) as antecedent/consequent signals.
                        walk(left, interner, kg, seen, antecedent, consequent, pred_names,
                             in_safety, in_liveness, position, impl_depth);
                        walk(right, interner, kg, seen, antecedent, consequent, pred_names,
                             in_safety, in_liveness, position, impl_depth);

                        if let (Some(left_sig), Some(right_sig)) =
                            (extract_hw_signal_name(left, interner), extract_hw_signal_name(right, interner))
                        {
                            if left_sig != right_sig {
                                kg.add_edge(&left_sig, &right_sig, KgRelation::Triggers, None);

                                // Detect response pattern G(P → X(Q))
                                let is_next = matches!(right,
                                    LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Next, .. }
                                ) || is_kripke_next(right, interner);
                                if is_next && in_safety {
                                    kg.add_entity(
                                        format!("{}_responds_to_{}", right_sig, left_sig),
                                        HwEntityType::ResponseProperty {
                                            trigger: left_sig.clone(),
                                            response: right_sig.clone(),
                                            bound: Some(1),
                                        },
                                    );
                                    kg.add_typed_edge(&left_sig, &right_sig, HwRelation::Triggers { delay: Some(1) });
                                }

                                // Detect eventually-follows G(P → F(Q))
                                let is_eventually = matches!(right,
                                    LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Eventually, .. }
                                ) || is_kripke_eventually(right, interner);
                                if is_eventually && in_safety {
                                    kg.add_typed_edge(&left_sig, &right_sig, HwRelation::EventuallyFollows);
                                }
                            }
                        }
                    } else {
                        // Accessibility predicate — just walk both sides
                        walk(left, interner, kg, seen, antecedent, consequent, pred_names,
                             in_safety, in_liveness, position, impl_depth);
                        walk(right, interner, kg, seen, antecedent, consequent, pred_names,
                             in_safety, in_liveness, position, impl_depth);
                    }
                } else {
                    // Other connectives — preserve position
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

                // Not(And(P, Q)) → Constrains edge + MutexProperty entity
                if let LogicExpr::BinaryOp { left, right, op: crate::token::TokenType::And } = operand {
                    if let (Some(left_sig), Some(right_sig)) =
                        (extract_signal_name(left, interner), extract_signal_name(right, interner))
                    {
                        if left_sig != right_sig {
                            kg.add_edge(&left_sig, &right_sig, KgRelation::Constrains, None);
                            // Sprint D: add MutexProperty entity
                            kg.add_entity(
                                format!("mutex_{}_{}", left_sig, right_sig),
                                HwEntityType::MutexProperty {
                                    signals: vec![left_sig.clone(), right_sig.clone()],
                                },
                            );
                        }
                    }
                }
            }

            // Temporal operators
            LogicExpr::Temporal { body, .. } => {
                walk(body, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness, position, impl_depth);
            }

            LogicExpr::TemporalBinary { operator, left, right } => {
                walk(left, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness, position, impl_depth);
                walk(right, interner, kg, seen, antecedent, consequent, pred_names,
                     in_safety, in_liveness, position, impl_depth);

                // Sprint D: Until → Precedes typed edge
                if matches!(operator, crate::ast::logic::BinaryTemporalOp::Until) {
                    if let (Some(left_sig), Some(right_sig)) =
                        (extract_hw_signal_name(left, interner), extract_hw_signal_name(right, interner))
                    {
                        if left_sig != right_sig {
                            kg.add_typed_edge(&left_sig, &right_sig, HwRelation::Precedes);
                        }
                    }
                }
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

    // Sprint D: detect handshake pairs from signal naming patterns
    let signal_names: Vec<String> = seen_signals.iter().cloned().collect();
    let handshake_pairs: Vec<(&str, &[&str])> = vec![
        ("valid", &["ready", "rdy"]),
        ("req", &["ack", "gnt", "grant"]),
        ("request", &["acknowledge", "acknowledgment", "response", "grant"]),
        ("cmd", &["resp", "response"]),
        ("start", &["done", "complete"]),
    ];
    for (trigger_pattern, response_patterns) in &handshake_pairs {
        let trigger_match = signal_names.iter().find(|s| s.to_lowercase().contains(trigger_pattern));
        if let Some(trigger_sig) = trigger_match {
            for resp_pattern in *response_patterns {
                let resp_match = signal_names.iter().find(|s| {
                    let lower = s.to_lowercase();
                    lower.contains(resp_pattern) && *s != trigger_sig
                });
                if let Some(resp_sig) = resp_match {
                    kg.add_entity(
                        format!("handshake_{}_{}", trigger_sig, resp_sig),
                        HwEntityType::Handshake {
                            valid_signal: trigger_sig.clone(),
                            ready_signal: resp_sig.clone(),
                        },
                    );
                    kg.add_typed_edge(trigger_sig, resp_sig, HwRelation::HandshakesWith);
                    break; // Only match the first response pattern
                }
            }
        }
    }

    // Assign roles based on position inference + populate typed entities
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

        kg.add_signal(sig_name, 1, role.clone());

        // Typed entity extraction: derive HwEntityType from signal role
        if name_lower.contains("clk") || name_lower.contains("clock") {
            kg.add_entity(sig_name, HwEntityType::Clock {
                frequency: None,
                domain: sig_name.clone(),
            });
        }
    }

    // Populate typed edges from legacy edges
    let mut mutex_entities: Vec<(String, HwEntityType)> = Vec::new();
    for edge in &kg.edges {
        let typed_rel = match &edge.relation {
            KgRelation::Triggers => HwRelation::Triggers { delay: None },
            KgRelation::Constrains => HwRelation::Constrains,
            KgRelation::Temporal => HwRelation::Triggers { delay: None },
            KgRelation::TypeOf => HwRelation::Contains,
        };
        kg.typed_edges.push((edge.from.clone(), edge.to.clone(), typed_rel));

        // Sprint D: Constrains edge → MutexProperty entity
        if edge.relation == KgRelation::Constrains {
            mutex_entities.push((
                format!("mutex_{}_{}", edge.from, edge.to),
                HwEntityType::MutexProperty {
                    signals: vec![edge.from.clone(), edge.to.clone()],
                },
            ));
        }
    }
    // Add mutex entities from Constrains edges (deferred to avoid borrow conflict)
    let already_has_mutex = kg.entities.iter().any(|(_, e)| matches!(e, HwEntityType::MutexProperty { .. }));
    if !already_has_mutex {
        for (name, entity) in mutex_entities {
            kg.add_entity(name, entity);
        }
    }

    // Sprint D: detect mutex from signal naming patterns
    // Signals with same base and different suffixes (e.g., grant_a, grant_b) → mutex
    if !kg.entities.iter().any(|(_, e)| matches!(e, HwEntityType::MutexProperty { .. })) {
        let sig_names: Vec<String> = kg.signals.iter().map(|s| s.name.clone()).collect();
        let mut mutex_groups: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for name in &sig_names {
            let lower = name.to_lowercase();
            // Check for patterns like "grant_a" → base "grant"
            if let Some(underscore_pos) = lower.rfind('_') {
                let suffix = &lower[underscore_pos+1..];
                if suffix.len() <= 2 { // Single char/digit suffix
                    let base = lower[..underscore_pos].to_string();
                    mutex_groups.entry(base).or_default().push(name.clone());
                }
            }
        }
        for (base, group) in &mutex_groups {
            if group.len() >= 2 && (base.contains("grant") || base.contains("sel") || base.contains("enable")) {
                kg.add_entity(
                    format!("mutex_{}", base),
                    HwEntityType::MutexProperty { signals: group.clone() },
                );
                // Add Constrains edges between pairs
                for i in 0..group.len() {
                    for j in (i+1)..group.len() {
                        kg.add_edge(&group[i], &group[j], KgRelation::Constrains, None);
                    }
                }
            }
        }
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

    // Build a formula description from predicate names
    let formula_desc = predicate_names.iter()
        .filter(|n| {
            let lower = n.to_lowercase();
            !lower.contains("accessible") && !lower.contains("reachable")
                && !lower.contains("next_temporal")
        })
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");

    match expr {
        LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Always, .. } => {
            let name = prop_name.unwrap_or_else(|| "Safety".to_string());
            kg.add_property(name.clone(), "safety", "G(...)");
            kg.add_entity(&name, HwEntityType::SafetyProperty {
                formula: format!("G({})", formula_desc),
            });
        }
        LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Eventually, .. } => {
            let name = prop_name.unwrap_or_else(|| "Liveness".to_string());
            kg.add_property(name.clone(), "liveness", "F(...)");
            kg.add_entity(&name, HwEntityType::LivenessProperty {
                formula: format!("F({})", formula_desc),
            });
        }
        LogicExpr::Quantifier { kind: QuantifierKind::Universal, .. } => {
            // Kripke-lowered G produces ∀w'(...)
            let name = prop_name.unwrap_or_else(|| "Safety".to_string());
            kg.add_property(name.clone(), "safety", "G(...)");
            kg.add_entity(&name, HwEntityType::SafetyProperty {
                formula: format!("G({})", formula_desc),
            });
        }
        LogicExpr::Quantifier { kind: QuantifierKind::Existential, .. } => {
            let name = prop_name.unwrap_or_else(|| "Liveness".to_string());
            kg.add_property(name.clone(), "liveness", "F(...)");
            kg.add_entity(&name, HwEntityType::LivenessProperty {
                formula: format!("F({})", formula_desc),
            });
        }
        _ => {}
    }

    kg
}

/// Check if an expression contains a Kripke-lowered Next: ∀w'(Next_Temporal(w,w') → P(w'))
/// Recurses through restriction quantifiers (non-world variables) to find the temporal structure.
fn is_kripke_next<'a>(
    expr: &'a crate::ast::logic::LogicExpr<'a>,
    interner: &crate::Interner,
) -> bool {
    use crate::ast::logic::{LogicExpr, QuantifierKind};
    match expr {
        LogicExpr::Quantifier { kind: QuantifierKind::Universal, body, variable, .. } => {
            let var_name = interner.resolve(*variable).to_string();
            if var_name.starts_with('w') {
                // World quantifier — check for Next_Temporal in the body
                if let LogicExpr::BinaryOp { left, op, .. } = body {
                    if matches!(op, crate::token::TokenType::Implies | crate::token::TokenType::If) {
                        if let LogicExpr::Predicate { name, .. } = left {
                            let pred_name = interner.resolve(*name).to_string();
                            if pred_name == "Next_Temporal" {
                                return true;
                            }
                        }
                    }
                }
            }
            // Non-world quantifier (restriction) — recurse into body
            is_kripke_next(body, interner)
        }
        LogicExpr::BinaryOp { right, op, .. } if matches!(op, crate::token::TokenType::Implies) => {
            // Restriction: ∀x(Type(x) → Body) — recurse into right side
            is_kripke_next(right, interner)
        }
        LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Next, .. } => true,
        _ => false,
    }
}

/// Check if an expression contains a Kripke-lowered Eventually: ∃w'(Reachable_Temporal(w,w') ∧ P(w'))
/// Recurses through restriction quantifiers to find the temporal structure.
fn is_kripke_eventually<'a>(
    expr: &'a crate::ast::logic::LogicExpr<'a>,
    interner: &crate::Interner,
) -> bool {
    use crate::ast::logic::{LogicExpr, QuantifierKind};
    match expr {
        LogicExpr::Quantifier { kind: QuantifierKind::Existential, body, variable, .. } => {
            let var_name = interner.resolve(*variable).to_string();
            if var_name.starts_with('w') {
                if let LogicExpr::BinaryOp { left, op: crate::token::TokenType::And, .. } = body {
                    if let LogicExpr::Predicate { name, .. } = left {
                        let pred_name = interner.resolve(*name).to_string();
                        if pred_name == "Reachable_Temporal" {
                            return true;
                        }
                    }
                }
            }
            // Recurse into body for restriction quantifiers
            is_kripke_eventually(body, interner)
        }
        LogicExpr::Quantifier { kind: QuantifierKind::Universal, body, .. } => {
            // Universal restriction quantifier — recurse
            is_kripke_eventually(body, interner)
        }
        LogicExpr::BinaryOp { right, op, .. } if matches!(op, crate::token::TokenType::Implies) => {
            is_kripke_eventually(right, interner)
        }
        LogicExpr::Temporal { operator: crate::ast::logic::TemporalOperator::Eventually, .. } => true,
        _ => false,
    }
}

/// Extract a hardware signal name from Kripke-lowered AST, preferring
/// restriction predicate names (the TYPE, e.g., "Request") over verb predicates
/// (e.g., "Hold/Have"). Used for edge extraction in the walk handler.
fn extract_hw_signal_name<'a>(
    expr: &'a crate::ast::logic::LogicExpr<'a>,
    interner: &crate::Interner,
) -> Option<String> {
    use crate::ast::logic::LogicExpr;
    match expr {
        // Quantifier with restriction: ∀x(Request(x) → Hold(x,w')) → "Request"
        LogicExpr::Quantifier { body, .. } => extract_hw_signal_name(body, interner),
        LogicExpr::BinaryOp { left, right, op } => {
            if matches!(op, crate::token::TokenType::Implies) {
                // Restriction pattern: left is the type predicate, prefer it
                let left_name = extract_signal_name(left, interner);
                if left_name.is_some() {
                    return left_name;
                }
                extract_hw_signal_name(right, interner)
            } else {
                extract_hw_signal_name(left, interner)
                    .or_else(|| extract_hw_signal_name(right, interner))
            }
        }
        // Fall through to regular extraction
        _ => extract_signal_name(expr, interner),
    }
}

/// Try to extract a signal name from an expression by recursing into
/// quantifiers, binary ops, and event structures to find the first
/// meaningful predicate argument.
fn extract_signal_name<'a>(
    expr: &'a crate::ast::logic::LogicExpr<'a>,
    interner: &crate::Interner,
) -> Option<String> {
    use crate::ast::logic::LogicExpr;
    match expr {
        LogicExpr::Predicate { name, args, .. } => {
            let pred_name = interner.resolve(*name).to_string();
            // Skip accessibility/meta predicates
            if pred_name.contains("Accessible") || pred_name.contains("Reachable")
                || pred_name.contains("Next_Temporal")
                || pred_name == "Agent" || pred_name == "Theme"
            {
                return None;
            }
            // Prefer non-world arguments (signal names) over predicate name (verb)
            let arg_name = args.iter().find_map(|arg| match arg {
                crate::ast::logic::Term::Constant(sym)
                | crate::ast::logic::Term::Variable(sym) => {
                    let aname = interner.resolve(*sym).to_string();
                    if (!aname.starts_with('w') || aname.len() > 3) && aname.len() > 1 {
                        Some(aname)
                    } else {
                        None
                    }
                }
                _ => None,
            });
            if let Some(an) = arg_name {
                return Some(an);
            }
            // Fall back to predicate name if no useful arguments
            if pred_name.len() > 1 && pred_name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                return Some(pred_name);
            }
            None
        }
        // Recurse into quantifiers to find the core predicate
        LogicExpr::Quantifier { body, .. } => extract_signal_name(body, interner),
        // Recurse into binary ops — for restrictions (Implies), look at the right side (body);
        // for conjunctions, try both sides
        LogicExpr::BinaryOp { left, right, op } => {
            if matches!(op, crate::token::TokenType::Implies) {
                // Restriction: ∀x(Type(x) → Body(x)) — signal is in the body
                extract_signal_name(right, interner)
                    .or_else(|| extract_signal_name(left, interner))
            } else {
                // Try left first, then right
                extract_signal_name(left, interner)
                    .or_else(|| extract_signal_name(right, interner))
            }
        }
        // Recurse into existentials (event structures: ∃e(Run(e) ∧ Agent(e,x)))
        LogicExpr::NeoEvent(data) => {
            let verb_name = interner.resolve(data.verb).to_string();
            Some(verb_name)
        }
        _ => None,
    }
}
