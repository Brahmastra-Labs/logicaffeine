//! Conversion from parser AST to proof engine representation.
//!
//! This module bridges the parser's arena-allocated AST ([`LogicExpr<'a>`]) to the
//! proof engine's owned representation ([`ProofExpr`]).
//!
//! The conversion clones all data into owned Strings, enabling proof trees
//! to persist beyond the arena's lifetime. Symbols are resolved using the
//! interner at conversion time.
//!
//! # Key Function
//!
//! [`logic_expr_to_proof_expr`] is the main entry point for converting
//! parsed expressions to the format expected by the proof search engine.

use crate::ast::logic::{
    BinaryTemporalOp, LogicExpr, ModalDomain, ModalFlavor, QuantifierKind, TemporalOperator, Term,
    ThematicRole,
};
use crate::intern::Interner;
use crate::lexicon::get_canonical_noun;
use logicaffeine_proof::{ProofExpr, ProofTerm};
use crate::token::TokenType;

// =============================================================================
// PUBLIC API
// =============================================================================

/// Map the parser's comparison-predicate names onto the proof oracle's canonical,
/// case-sensitive vocabulary (`Gt`/`Lt`/`Gte`/`Lte`/`Eq`/`Neq`; see oracle.rs:879
/// and modal_translation.rs:124). Returns `None` for any non-comparison name,
/// which then falls through to ordinary noun normalization. The parser only ever
/// emits these names for binary degree comparisons, so the caller gates on arity 2.
fn canonical_comparison_name(name: &str) -> Option<&'static str> {
    match name {
        "Greater" | "Gt" => Some("Gt"),
        "Less" | "Lt" => Some("Lt"),
        "GreaterEqual" | "Gte" => Some("Gte"),
        "LessEqual" | "Lte" => Some("Lte"),
        "Equal" | "Eq" => Some("Eq"),
        "NotEqual" | "Neq" => Some("Neq"),
        _ => None,
    }
}

/// Map the parser's arithmetic-function names onto the oracle's canonical,
/// case-sensitive `Add`/`Sub`/`Mul`/`Div` (see oracle.rs:1034). Returns `None`
/// for every other function name so measure functions (`Score`, `Ord`, …) stay
/// verbatim as uninterpreted integer functions.
fn canonical_arithmetic_fn(name: &str) -> Option<&'static str> {
    match name {
        "add" | "Add" => Some("Add"),
        "sub" | "Sub" => Some("Sub"),
        "mul" | "Mul" => Some("Mul"),
        "div" | "Div" => Some("Div"),
        _ => None,
    }
}

/// Rename a free variable `from` → `to` inside a [`ProofTerm`].
fn subst_proof_term(t: &ProofTerm, from: &str, to: &str) -> ProofTerm {
    match t {
        ProofTerm::Variable(v) if v == from => ProofTerm::Variable(to.to_string()),
        ProofTerm::BoundVarRef(v) if v == from => ProofTerm::BoundVarRef(to.to_string()),
        ProofTerm::Function(name, args) => ProofTerm::Function(
            name.clone(),
            args.iter().map(|a| subst_proof_term(a, from, to)).collect(),
        ),
        ProofTerm::Group(args) => {
            ProofTerm::Group(args.iter().map(|a| subst_proof_term(a, from, to)).collect())
        }
        other => other.clone(),
    }
}

/// Rename a free variable `from` → `to` inside a [`ProofExpr`], stopping at a
/// quantifier that re-binds `from` (capture avoidance). Used to build the
/// uniqueness clause of an "exactly one" expansion.
fn subst_proof_var(e: &ProofExpr, from: &str, to: &str) -> ProofExpr {
    match e {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(|a| subst_proof_term(a, from, to)).collect(),
            world: world.clone(),
        },
        ProofExpr::Identity(a, b) => {
            ProofExpr::Identity(subst_proof_term(a, from, to), subst_proof_term(b, from, to))
        }
        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(subst_proof_var(l, from, to)),
            Box::new(subst_proof_var(r, from, to)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(subst_proof_var(l, from, to)),
            Box::new(subst_proof_var(r, from, to)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(subst_proof_var(l, from, to)),
            Box::new(subst_proof_var(r, from, to)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(subst_proof_var(l, from, to)),
            Box::new(subst_proof_var(r, from, to)),
        ),
        ProofExpr::Not(x) => ProofExpr::Not(Box::new(subst_proof_var(x, from, to))),
        ProofExpr::ForAll { variable, body } if variable != from => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(subst_proof_var(body, from, to)),
        },
        ProofExpr::Exists { variable, body } if variable != from => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(subst_proof_var(body, from, to)),
        },
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(subst_proof_var(body, from, to)),
        },
        ProofExpr::Term(t) => ProofExpr::Term(subst_proof_term(t, from, to)),
        other => other.clone(),
    }
}

fn subst_term_with(t: &ProofTerm, from: &str, to: &ProofTerm) -> ProofTerm {
    match t {
        ProofTerm::Variable(v) if v == from => to.clone(),
        ProofTerm::BoundVarRef(v) if v == from => to.clone(),
        ProofTerm::Function(name, args) => ProofTerm::Function(
            name.clone(),
            args.iter().map(|a| subst_term_with(a, from, to)).collect(),
        ),
        ProofTerm::Group(args) => {
            ProofTerm::Group(args.iter().map(|a| subst_term_with(a, from, to)).collect())
        }
        other => other.clone(),
    }
}

/// Substitute a free variable `from` with the CONSTANT `to` inside a [`ProofExpr`]
/// — the operation that turns a wh-question body φ(x) into a candidate goal φ(c),
/// so "who/what is …?" is answered by enumerating domain individuals and proving
/// each candidate. Stops at a quantifier that re-binds `from` (capture avoidance).
pub fn instantiate_var_with_constant(e: &ProofExpr, from: &str, to: &str) -> ProofExpr {
    let c = ProofTerm::Constant(to.to_string());
    subst_expr_with(e, from, &c)
}

fn subst_expr_with(e: &ProofExpr, from: &str, to: &ProofTerm) -> ProofExpr {
    match e {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(|a| subst_term_with(a, from, to)).collect(),
            world: world.clone(),
        },
        ProofExpr::Identity(a, b) => {
            ProofExpr::Identity(subst_term_with(a, from, to), subst_term_with(b, from, to))
        }
        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(subst_expr_with(l, from, to)),
            Box::new(subst_expr_with(r, from, to)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(subst_expr_with(l, from, to)),
            Box::new(subst_expr_with(r, from, to)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(subst_expr_with(l, from, to)),
            Box::new(subst_expr_with(r, from, to)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(subst_expr_with(l, from, to)),
            Box::new(subst_expr_with(r, from, to)),
        ),
        ProofExpr::Not(x) => ProofExpr::Not(Box::new(subst_expr_with(x, from, to))),
        ProofExpr::ForAll { variable, body } if variable != from => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(subst_expr_with(body, from, to)),
        },
        ProofExpr::Exists { variable, body } if variable != from => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(subst_expr_with(body, from, to)),
        },
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(subst_expr_with(body, from, to)),
        },
        ProofExpr::Term(t) => ProofExpr::Term(subst_term_with(t, from, to)),
        other => other.clone(),
    }
}

/// Convert a LogicExpr to ProofExpr.
///
/// This is the main entry point for bridging the parser to the proof engine.
/// All Symbols are resolved to owned Strings using the interner.
pub fn logic_expr_to_proof_expr<'a>(expr: &LogicExpr<'a>, interner: &Interner) -> ProofExpr {
    match expr {
        // --- Core FOL ---
        LogicExpr::Predicate { name, args, world } => {
            // Semantic Normalization:
            // 1. Lemmatize: "cats" → "Cat", "men" → "Man" (canonical noun form)
            // 2. Lowercase: "Cat" → "cat", "Mortal" → "mortal"
            // This ensures "Mortal" (noun) == "mortal" (adj) == "mortals" (plural noun)
            let name_str = interner.resolve(*name);
            // A binary comparison predicate carries arithmetic meaning the oracle
            // recognises only under its canonical name; it bypasses noun
            // normalization (which would lowercase "Greater" → "greater" and strand
            // the comparison as an uninterpreted function the solver cannot use).
            let normalized = match (args.len() == 2)
                .then(|| canonical_comparison_name(name_str))
                .flatten()
            {
                Some(canon) => canon.to_string(),
                None => get_canonical_noun(&name_str.to_lowercase())
                    .map(|lemma| lemma.to_lowercase())
                    .unwrap_or_else(|| name_str.to_lowercase()),
            };

            ProofExpr::Predicate {
                name: normalized,
                args: args.iter().map(|t| term_to_proof_term(t, interner)).collect(),
                world: world.map(|w| interner.resolve(w).to_string()),
            }
        }

        LogicExpr::Identity { left, right } => ProofExpr::Identity(
            term_to_proof_term(left, interner),
            term_to_proof_term(right, interner),
        ),

        LogicExpr::Atom(s) => ProofExpr::Atom(interner.resolve(*s).to_string()),

        // --- Quantifiers ---
        LogicExpr::Quantifier {
            kind,
            variable,
            body,
            ..
        } => {
            let var_name = interner.resolve(*variable).to_string();
            let body_expr = Box::new(logic_expr_to_proof_expr(body, interner));

            match kind {
                QuantifierKind::Universal => ProofExpr::ForAll {
                    variable: var_name,
                    body: body_expr,
                },
                QuantifierKind::Existential => ProofExpr::Exists {
                    variable: var_name,
                    body: body_expr,
                },
                // Map other quantifiers to existential with a note
                QuantifierKind::Most => ProofExpr::Unsupported("Most quantifier".into()),
                QuantifierKind::Few => ProofExpr::Unsupported("Few quantifier".into()),
                QuantifierKind::Many => ProofExpr::Unsupported("Many quantifier".into()),
                QuantifierKind::Generic => ProofExpr::ForAll {
                    variable: var_name,
                    body: body_expr,
                },
                QuantifierKind::Cardinal(1) => {
                    // "Exactly one x: φ(x)" = ∃x(φ(x) ∧ ∀y(φ(y) → y = x)) — existence
                    // PLUS uniqueness, the form the entailment oracle reasons over.
                    // Dropping the count (plain ∃) loses the constraint a logic-grid
                    // bijection depends on.
                    let x = var_name;
                    let y = format!("{x}_uniq");
                    let phi_y = Box::new(subst_proof_var(&body_expr, &x, &y));
                    let uniqueness = ProofExpr::ForAll {
                        variable: y.clone(),
                        body: Box::new(ProofExpr::Implies(
                            phi_y,
                            Box::new(ProofExpr::Identity(
                                ProofTerm::Variable(y),
                                ProofTerm::Variable(x.clone()),
                            )),
                        )),
                    };
                    ProofExpr::Exists {
                        variable: x,
                        body: Box::new(ProofExpr::And(body_expr, Box::new(uniqueness))),
                    }
                }
                QuantifierKind::Cardinal(n) => {
                    // n ≥ 2: existence is a sound (if incomplete) weakening of the
                    // exact count for the proof path.
                    ProofExpr::Exists {
                        variable: format!("{}_{}", var_name, n),
                        body: body_expr,
                    }
                }
                QuantifierKind::AtLeast(_) | QuantifierKind::AtMost(_) => {
                    ProofExpr::Unsupported("Counting quantifier".into())
                }
            }
        }

        // --- Logical Connectives ---
        LogicExpr::BinaryOp { left, op, right } => {
            let l = Box::new(logic_expr_to_proof_expr(left, interner));
            let r = Box::new(logic_expr_to_proof_expr(right, interner));

            match op {
                TokenType::And => ProofExpr::And(l, r),
                TokenType::Or => ProofExpr::Or(l, r),
                TokenType::If | TokenType::Implies | TokenType::Then => ProofExpr::Implies(l, r),
                TokenType::Iff => ProofExpr::Iff(l, r),
                _ => ProofExpr::Unsupported(format!("Binary operator {:?}", op)),
            }
        }

        LogicExpr::UnaryOp { op, operand } => {
            let inner = Box::new(logic_expr_to_proof_expr(operand, interner));
            match op {
                TokenType::Not => ProofExpr::Not(inner),
                _ => ProofExpr::Unsupported(format!("Unary operator {:?}", op)),
            }
        }

        // --- Modal Logic ---
        LogicExpr::Modal { vector, operand } => {
            let body = Box::new(logic_expr_to_proof_expr(operand, interner));
            let domain = match vector.domain {
                ModalDomain::Alethic => "Alethic",
                ModalDomain::Deontic => "Deontic",
                ModalDomain::Temporal => "Temporal",
            };
            let flavor = match vector.flavor {
                ModalFlavor::Root => "Root",
                ModalFlavor::Epistemic => "Epistemic",
                ModalFlavor::Evidential => "Evidential",
                ModalFlavor::Bouletic => "Bouletic",
            };
            ProofExpr::Modal {
                domain: domain.to_string(),
                force: vector.force,
                flavor: flavor.to_string(),
                body,
            }
        }

        // --- Temporal Logic ---
        LogicExpr::Temporal { operator, body } => {
            let body_expr = Box::new(logic_expr_to_proof_expr(body, interner));
            let op_name = match operator {
                TemporalOperator::Past => "Past",
                TemporalOperator::Future => "Future",
                TemporalOperator::Always => "Always",
                TemporalOperator::Eventually
                | TemporalOperator::BoundedEventually(_) => "Eventually",
                TemporalOperator::Next => "Next",
            };
            ProofExpr::Temporal {
                operator: op_name.to_string(),
                body: body_expr,
            }
        }

        LogicExpr::TemporalBinary { operator, left, right } => ProofExpr::TemporalBinary {
            operator: format!("{:?}", operator),
            left: Box::new(logic_expr_to_proof_expr(left, interner)),
            right: Box::new(logic_expr_to_proof_expr(right, interner)),
        },

        // --- Lambda Calculus ---
        LogicExpr::Lambda { variable, body } => ProofExpr::Lambda {
            variable: interner.resolve(*variable).to_string(),
            body: Box::new(logic_expr_to_proof_expr(body, interner)),
        },

        LogicExpr::App { function, argument } => ProofExpr::App(
            Box::new(logic_expr_to_proof_expr(function, interner)),
            Box::new(logic_expr_to_proof_expr(argument, interner)),
        ),

        // --- Event Semantics ---
        LogicExpr::NeoEvent(data) => {
            let roles: Vec<(String, ProofTerm)> = data
                .roles
                .iter()
                .map(|(role, term)| {
                    let role_name = match role {
                        ThematicRole::Agent => "Agent",
                        ThematicRole::Patient => "Patient",
                        ThematicRole::Theme => "Theme",
                        ThematicRole::Recipient => "Recipient",
                        ThematicRole::Goal => "Goal",
                        ThematicRole::Source => "Source",
                        ThematicRole::Instrument => "Instrument",
                        ThematicRole::Location => "Location",
                        ThematicRole::Time => "Time",
                        ThematicRole::Manner => "Manner",
                        ThematicRole::Result => "Result",
                        ThematicRole::Depictive => "Depictive",
                    };
                    (role_name.to_string(), term_to_proof_term(term, interner))
                })
                .collect();

            ProofExpr::NeoEvent {
                event_var: interner.resolve(data.event_var).to_string(),
                verb: interner.resolve(data.verb).to_string(),
                roles,
            }
        }

        // --- Counterfactual ---
        LogicExpr::Counterfactual {
            antecedent,
            consequent,
        } => {
            // □→ keeps closest-world semantics: the consequent is quantified
            // over the similarity-closest antecedent-worlds, never lowered to
            // material implication (§4.5).
            ProofExpr::Counterfactual {
                antecedent: Box::new(logic_expr_to_proof_expr(antecedent, interner)),
                consequent: Box::new(logic_expr_to_proof_expr(consequent, interner)),
            }
        }

        // --- Unsupported constructs (return Unsupported variant) ---
        LogicExpr::Categorical(_) => ProofExpr::Unsupported("Categorical (legacy)".into()),
        LogicExpr::Relation(_) => ProofExpr::Unsupported("Relation (legacy)".into()),
        LogicExpr::Metaphor { .. } => ProofExpr::Unsupported("Metaphor".into()),
        // A wh-question "Who is a lawyer?" is the GOAL ∃x.φ(x): proving it means
        // SOMEONE satisfies φ; the ANSWER is the witness (extracted by enumerating
        // the domain in `answer_question`). Carrying the variable + body lets the
        // answer layer recover both.
        LogicExpr::Question { wh_variable, body } => ProofExpr::Exists {
            variable: interner.resolve(*wh_variable).to_string(),
            body: Box::new(logic_expr_to_proof_expr(body, interner)),
        },
        LogicExpr::YesNoQuestion { .. } => ProofExpr::Unsupported("YesNoQuestion".into()),
        LogicExpr::Intensional { .. } => ProofExpr::Unsupported("Intensional".into()),
        LogicExpr::Event { .. } => ProofExpr::Unsupported("Event (legacy)".into()),
        LogicExpr::Imperative { action } => {
            // Directive(h, p) → O_g p: the commanded action becomes a bouletic
            // obligation over the addressee's action worlds (§1.4). The action
            // itself is NOT asserted — commanding is not doing.
            ProofExpr::Modal {
                domain: "Deontic".to_string(),
                force: 1.0,
                flavor: "Bouletic".to_string(),
                body: Box::new(logic_expr_to_proof_expr(action, interner)),
            }
        }
        LogicExpr::Exclamative { body, .. } => {
            // The presupposed/asserted content is the body predication.
            logic_expr_to_proof_expr(body, interner)
        }
        LogicExpr::Optative { wish } => {
            // Wish(speaker, ⟨p⟩): a bouletic necessity over the speaker's
            // preference-ideal worlds (§1.2); the complement is not entailed.
            ProofExpr::Modal {
                domain: "Deontic".to_string(),
                force: 1.0,
                flavor: "Bouletic".to_string(),
                body: Box::new(logic_expr_to_proof_expr(wish, interner)),
            }
        }
        LogicExpr::Implicature { assertion, .. } => {
            // Truth-conditional content is the literal assertion; the implicature is
            // defeasible/cancellable and not part of the entailment core.
            logic_expr_to_proof_expr(assertion, interner)
        }
        LogicExpr::SpeechAct { performer, act_type, .. } => {
            // A performative asserts that the act is performed at the utterance
            // world (the saying is the doing): `act_type(performer)`. The
            // propositional content is NOT asserted — promising to φ does not make
            // φ true — so it is deliberately not conjoined here.
            ProofExpr::Predicate {
                name: interner.resolve(*act_type).to_lowercase(),
                args: vec![term_to_proof_term(&Term::Constant(*performer), interner)],
                world: None,
            }
        }
        LogicExpr::Causal { .. } => ProofExpr::Unsupported("Causal".into()),
        LogicExpr::Concessive { main, .. } => {
            // The main clause is asserted; the concession is backgrounded (a defeated
            // expectation), so the truth-conditional content reduces to the main.
            logic_expr_to_proof_expr(main, interner)
        }
        LogicExpr::Comparative { .. } => ProofExpr::Unsupported("Comparative".into()),
        LogicExpr::Superlative { .. } => ProofExpr::Unsupported("Superlative".into()),
        LogicExpr::Scopal { .. } => ProofExpr::Unsupported("Scopal".into()),
        LogicExpr::Control { .. } => ProofExpr::Unsupported("Control".into()),
        LogicExpr::Presupposition {
            assertion,
            presupposition,
        } => {
            // A surviving (projected/accommodated) presupposition is real
            // content: "Mary doesn't regret lying" carries both ¬Regret and
            // the projected Lied(mary). Bound/filtered presuppositions are
            // rewritten away before this point (Van der Sandt pass), so the
            // conjunction is monotonically sound.
            ProofExpr::And(
                Box::new(logic_expr_to_proof_expr(assertion, interner)),
                Box::new(logic_expr_to_proof_expr(presupposition, interner)),
            )
        }
        LogicExpr::Focus { scope, .. } => {
            // Focus marking is information structure; the truth-conditional
            // content is the scope. Cleft exhaustivity is already a separate
            // conjunct built by the parser.
            logic_expr_to_proof_expr(scope, interner)
        }
        LogicExpr::TemporalAnchor { .. } => ProofExpr::Unsupported("TemporalAnchor".into()),
        LogicExpr::Distributive { predicate } => {
            // *P(σN) — atomic distribution over the plural sum. Members of
            // σN are exactly the Ns (Link lattice atoms), so the first-order
            // form is ∀x(N(x) → P(x)).
            let base = logic_expr_to_proof_expr(predicate, interner);
            match find_sigma_symbol(predicate) {
                Some(noun) => {
                    let noun_str = interner.resolve(noun).to_string();
                    let noun_pred = get_canonical_noun(&noun_str.to_lowercase())
                        .map(|lemma| lemma.to_lowercase())
                        .unwrap_or_else(|| noun_str.to_lowercase());
                    let var = format!("each_{}", noun_pred);
                    let sigma_term = ProofTerm::Variable(noun_str);
                    let member = ProofTerm::Variable(var.clone());
                    let body = replace_proof_term(&base, &sigma_term, &member);
                    ProofExpr::ForAll {
                        variable: var.clone(),
                        body: Box::new(ProofExpr::Implies(
                            Box::new(ProofExpr::Predicate {
                                name: noun_pred,
                                args: vec![ProofTerm::Variable(var)],
                                world: None,
                            }),
                            Box::new(body),
                        )),
                    }
                }
                None => base,
            }
        }
        LogicExpr::GroupQuantifier {
            group_var,
            count,
            member_var,
            restriction,
            body,
        } => {
            // ∃g(group(g) ∧ count(g, n) ∧ ∀x(member(x, g) → R(x)) ∧ B(g))
            let g = interner.resolve(*group_var).to_string();
            let x = interner.resolve(*member_var).to_string();
            let group_pred = ProofExpr::Predicate {
                name: "group".to_string(),
                args: vec![ProofTerm::Variable(g.clone())],
                world: None,
            };
            let count_pred = ProofExpr::Predicate {
                name: "count".to_string(),
                args: vec![
                    ProofTerm::Variable(g.clone()),
                    ProofTerm::Constant(count.to_string()),
                ],
                world: None,
            };
            let member_pred = ProofExpr::Predicate {
                name: "member".to_string(),
                args: vec![ProofTerm::Variable(x.clone()), ProofTerm::Variable(g.clone())],
                world: None,
            };
            let members = ProofExpr::ForAll {
                variable: x,
                body: Box::new(ProofExpr::Implies(
                    Box::new(member_pred),
                    Box::new(logic_expr_to_proof_expr(restriction, interner)),
                )),
            };
            ProofExpr::Exists {
                variable: g,
                body: Box::new(ProofExpr::And(
                    Box::new(ProofExpr::And(
                        Box::new(ProofExpr::And(Box::new(group_pred), Box::new(count_pred))),
                        Box::new(members),
                    )),
                    Box::new(logic_expr_to_proof_expr(body, interner)),
                )),
            }
        }
        // Aspectual wrappers (Imperfective, Perfective, etc.) are transparent to proof.
        // "John runs" -> Aspectual(Imperfective, ∃e(Run(e) ∧ Agent(e, John)))
        // We pass through to the inner event structure.
        LogicExpr::Aspectual { body, .. } => logic_expr_to_proof_expr(body, interner),
        LogicExpr::Voice { .. } => ProofExpr::Unsupported("Voice".into()),
    }
}

/// Convert a Term to ProofTerm.
/// A defeasible default extracted by [`logic_expr_to_proof_expr_defeasible`]:
/// the abnormality predicate guarding one GEN rule or implicature.
#[derive(Debug, Clone)]
pub struct DefaultRule {
    /// The abnormality predicate name (`ab_1`, `ab_2`, …).
    pub ab_name: String,
    /// The generic's restrictor predicate ("penguin" in GEN x(Penguin → …)),
    /// used for specificity ordering. `None` for implicatures.
    pub restriction_pred: Option<String>,
    /// Unary (per-individual, generics) vs propositional (implicatures).
    pub unary: bool,
}

/// Convert with DEFEASIBLE semantics: a generic `GEN x(R(x) → N(x))` becomes
/// the abnormality-guarded `∀x((R(x) ∧ ¬ab_k(x)) → N(x))`, and an implicature
/// is asserted under its own guard (`assertion ∧ (¬ab_k → implicature)`).
/// The circumscription itself — minimizing each `ab_k` — happens in the
/// defeasible reasoner; this conversion only preserves what the strict
/// export (`Generic → ∀`, implicature dropped) erases.
pub fn logic_expr_to_proof_expr_defeasible<'a>(
    expr: &LogicExpr<'a>,
    interner: &Interner,
    defaults: &mut Vec<DefaultRule>,
) -> ProofExpr {
    match expr {
        LogicExpr::Quantifier {
            kind: QuantifierKind::Generic,
            variable,
            body,
            ..
        } => {
            let var = interner.resolve(*variable).to_string();
            if let LogicExpr::BinaryOp {
                left: restriction,
                op: TokenType::Implies | TokenType::If,
                right: nucleus,
            } = body
            {
                let ab_name = format!("ab_{}", defaults.len() + 1);
                let restriction_pred = match restriction {
                    LogicExpr::Predicate { name, .. } => {
                        let s = interner.resolve(*name);
                        Some(
                            get_canonical_noun(&s.to_lowercase())
                                .map(|l| l.to_lowercase())
                                .unwrap_or_else(|| s.to_lowercase()),
                        )
                    }
                    _ => None,
                };
                defaults.push(DefaultRule {
                    ab_name: ab_name.clone(),
                    restriction_pred,
                    unary: true,
                });
                let guarded = ProofExpr::And(
                    Box::new(logic_expr_to_proof_expr(restriction, interner)),
                    Box::new(ProofExpr::Not(Box::new(ProofExpr::Predicate {
                        name: ab_name,
                        args: vec![ProofTerm::Variable(var.clone())],
                        world: None,
                    }))),
                );
                return ProofExpr::ForAll {
                    variable: var,
                    body: Box::new(ProofExpr::Implies(
                        Box::new(guarded),
                        Box::new(logic_expr_to_proof_expr(nucleus, interner)),
                    )),
                };
            }
            logic_expr_to_proof_expr(expr, interner)
        }
        LogicExpr::Implicature {
            assertion,
            implicature,
        } => {
            let ab_name = format!("ab_{}", defaults.len() + 1);
            defaults.push(DefaultRule {
                ab_name: ab_name.clone(),
                restriction_pred: None,
                unary: false,
            });
            ProofExpr::And(
                Box::new(logic_expr_to_proof_expr(assertion, interner)),
                Box::new(ProofExpr::Implies(
                    Box::new(ProofExpr::Not(Box::new(ProofExpr::Atom(ab_name)))),
                    Box::new(logic_expr_to_proof_expr(implicature, interner)),
                )),
            )
        }
        // Containers recurse so defaults nested under connectives or
        // presuppositions are still found.
        LogicExpr::BinaryOp { left, op, right } => {
            let l = logic_expr_to_proof_expr_defeasible(left, interner, defaults);
            let r = logic_expr_to_proof_expr_defeasible(right, interner, defaults);
            match op {
                TokenType::And => ProofExpr::And(Box::new(l), Box::new(r)),
                TokenType::Or => ProofExpr::Or(Box::new(l), Box::new(r)),
                TokenType::If | TokenType::Implies => {
                    ProofExpr::Implies(Box::new(l), Box::new(r))
                }
                TokenType::Iff => ProofExpr::Iff(Box::new(l), Box::new(r)),
                _ => logic_expr_to_proof_expr(expr, interner),
            }
        }
        LogicExpr::UnaryOp {
            op: TokenType::Not,
            operand,
        } => ProofExpr::Not(Box::new(logic_expr_to_proof_expr_defeasible(
            operand, interner, defaults,
        ))),
        LogicExpr::Presupposition {
            assertion,
            presupposition,
        } => ProofExpr::And(
            Box::new(logic_expr_to_proof_expr_defeasible(
                assertion, interner, defaults,
            )),
            Box::new(logic_expr_to_proof_expr_defeasible(
                presupposition,
                interner,
                defaults,
            )),
        ),
        _ => logic_expr_to_proof_expr(expr, interner),
    }
}

/// Find the first `Term::Sigma` symbol in an expression (the plural sum a
/// `Distributive` operator ranges over).
fn find_sigma_symbol<'a>(expr: &LogicExpr<'a>) -> Option<crate::Symbol> {
    fn in_term<'a>(term: &Term<'a>) -> Option<crate::Symbol> {
        match term {
            Term::Sigma(s) => Some(*s),
            Term::Function(_, args) | Term::Group(args) => args.iter().find_map(in_term),
            Term::Possessed { possessor, .. } => in_term(possessor),
            _ => None,
        }
    }
    match expr {
        LogicExpr::Predicate { args, .. } => args.iter().find_map(in_term),
        LogicExpr::NeoEvent(data) => data.roles.iter().find_map(|(_, t)| in_term(t)),
        LogicExpr::Quantifier { body, .. } => find_sigma_symbol(body),
        LogicExpr::BinaryOp { left, right, .. } => {
            find_sigma_symbol(left).or_else(|| find_sigma_symbol(right))
        }
        LogicExpr::UnaryOp { operand, .. } => find_sigma_symbol(operand),
        LogicExpr::Modal { operand, .. } => find_sigma_symbol(operand),
        LogicExpr::Temporal { body, .. } => find_sigma_symbol(body),
        LogicExpr::Aspectual { body, .. } => find_sigma_symbol(body),
        LogicExpr::Distributive { predicate } => find_sigma_symbol(predicate),
        LogicExpr::Presupposition { assertion, .. } => find_sigma_symbol(assertion),
        _ => None,
    }
}

/// Replace every occurrence of `from` with `to` in the term positions of a
/// proof expression (used to instantiate a plural sum by its members).
fn replace_proof_term(expr: &ProofExpr, from: &ProofTerm, to: &ProofTerm) -> ProofExpr {
    fn in_term(term: &ProofTerm, from: &ProofTerm, to: &ProofTerm) -> ProofTerm {
        if term == from {
            return to.clone();
        }
        match term {
            ProofTerm::Function(name, args) => ProofTerm::Function(
                name.clone(),
                args.iter().map(|t| in_term(t, from, to)).collect(),
            ),
            ProofTerm::Group(terms) => {
                ProofTerm::Group(terms.iter().map(|t| in_term(t, from, to)).collect())
            }
            other => other.clone(),
        }
    }
    match expr {
        ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
            name: name.clone(),
            args: args.iter().map(|t| in_term(t, from, to)).collect(),
            world: world.clone(),
        },
        ProofExpr::Identity(l, r) => {
            ProofExpr::Identity(in_term(l, from, to), in_term(r, from, to))
        }
        ProofExpr::And(l, r) => ProofExpr::And(
            Box::new(replace_proof_term(l, from, to)),
            Box::new(replace_proof_term(r, from, to)),
        ),
        ProofExpr::Or(l, r) => ProofExpr::Or(
            Box::new(replace_proof_term(l, from, to)),
            Box::new(replace_proof_term(r, from, to)),
        ),
        ProofExpr::Implies(l, r) => ProofExpr::Implies(
            Box::new(replace_proof_term(l, from, to)),
            Box::new(replace_proof_term(r, from, to)),
        ),
        ProofExpr::Iff(l, r) => ProofExpr::Iff(
            Box::new(replace_proof_term(l, from, to)),
            Box::new(replace_proof_term(r, from, to)),
        ),
        ProofExpr::Not(inner) => {
            ProofExpr::Not(Box::new(replace_proof_term(inner, from, to)))
        }
        ProofExpr::ForAll { variable, body } => ProofExpr::ForAll {
            variable: variable.clone(),
            body: Box::new(replace_proof_term(body, from, to)),
        },
        ProofExpr::Exists { variable, body } => ProofExpr::Exists {
            variable: variable.clone(),
            body: Box::new(replace_proof_term(body, from, to)),
        },
        ProofExpr::Modal {
            domain,
            force,
            flavor,
            body,
        } => ProofExpr::Modal {
            domain: domain.clone(),
            force: *force,
            flavor: flavor.clone(),
            body: Box::new(replace_proof_term(body, from, to)),
        },
        ProofExpr::Counterfactual {
            antecedent,
            consequent,
        } => ProofExpr::Counterfactual {
            antecedent: Box::new(replace_proof_term(antecedent, from, to)),
            consequent: Box::new(replace_proof_term(consequent, from, to)),
        },
        ProofExpr::Temporal { operator, body } => ProofExpr::Temporal {
            operator: operator.clone(),
            body: Box::new(replace_proof_term(body, from, to)),
        },
        ProofExpr::NeoEvent {
            event_var,
            verb,
            roles,
        } => ProofExpr::NeoEvent {
            event_var: event_var.clone(),
            verb: verb.clone(),
            roles: roles
                .iter()
                .map(|(r, t)| (r.clone(), in_term(t, from, to)))
                .collect(),
        },
        other => other.clone(),
    }
}

pub fn term_to_proof_term<'a>(term: &Term<'a>, interner: &Interner) -> ProofTerm {
    match term {
        Term::Constant(s) => ProofTerm::Constant(interner.resolve(*s).to_string()),

        Term::Variable(s) => ProofTerm::Variable(interner.resolve(*s).to_string()),

        Term::Function(name, args) => {
            // An arithmetic offset ("add"/"sub") must reach the oracle under its
            // canonical name (`Add`/`Sub`) or the equality degrades to an
            // uninterpreted function and the offset never forces a value. Measure
            // functions (Score, Ord, …) are left verbatim as uninterpreted ints.
            let name_str = interner.resolve(*name);
            let canon = (args.len() == 2)
                .then(|| canonical_arithmetic_fn(name_str))
                .flatten()
                .map(|s| s.to_string())
                .unwrap_or_else(|| name_str.to_string());
            ProofTerm::Function(
                canon,
                args.iter().map(|t| term_to_proof_term(t, interner)).collect(),
            )
        }

        Term::Group(terms) => {
            ProofTerm::Group(terms.iter().map(|t| term_to_proof_term(t, interner)).collect())
        }

        Term::Possessed { possessor, possessed } => {
            // Convert possession to function application: has(possessor, possessed)
            ProofTerm::Function(
                "has".to_string(),
                vec![
                    term_to_proof_term(possessor, interner),
                    ProofTerm::Constant(interner.resolve(*possessed).to_string()),
                ],
            )
        }

        Term::Sigma(s) => {
            // Sigma variables become regular variables
            ProofTerm::Variable(interner.resolve(*s).to_string())
        }

        Term::Intension(s) => {
            // Intensions become constants with ^ prefix
            ProofTerm::Constant(format!("^{}", interner.resolve(*s)))
        }

        Term::Kind(s) => {
            // Kind terms are reified entities; like intensions they become a
            // ^-prefixed constant so kind predication reasons over a fixed object.
            ProofTerm::Constant(format!("^{}", interner.resolve(*s)))
        }

        Term::Proposition(expr) => {
            // Embedded propositions - convert recursively but wrap as constant
            // This is a simplification; full handling would need reification
            let proof_expr = logic_expr_to_proof_expr(expr, interner);
            ProofTerm::Constant(format!("[{}]", proof_expr))
        }

        Term::Value { kind, unit, .. } => {
            // Convert numeric values to constants
            use crate::ast::logic::NumberKind;
            match kind {
                NumberKind::Integer(n) => {
                    if let Some(u) = unit {
                        ProofTerm::Constant(format!("{}{}", n, interner.resolve(*u)))
                    } else {
                        ProofTerm::Constant(n.to_string())
                    }
                }
                NumberKind::Real(f) => {
                    if let Some(u) = unit {
                        ProofTerm::Constant(format!("{}{}", f, interner.resolve(*u)))
                    } else {
                        ProofTerm::Constant(f.to_string())
                    }
                }
                NumberKind::Symbolic(s) => ProofTerm::Constant(interner.resolve(*s).to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Arena;

    #[test]
    fn test_convert_predicate() {
        let mut interner = Interner::new();
        let name = interner.intern("Man");
        let arg = interner.intern("socrates");

        let arena: Arena<Term> = Arena::new();
        let args = arena.alloc_slice([Term::Constant(arg)]);

        let expr = LogicExpr::Predicate {
            name,
            args,
            world: None,
        };

        let result = logic_expr_to_proof_expr(&expr, &interner);

        match result {
            ProofExpr::Predicate { name, args, world } => {
                // Predicate names are normalized to lowercase
                assert_eq!(name, "man");
                assert_eq!(args.len(), 1);
                // Terms (constants) preserve their case
                assert!(matches!(&args[0], ProofTerm::Constant(s) if s == "socrates"));
                assert!(world.is_none());
            }
            _ => panic!("Expected Predicate, got {:?}", result),
        }
    }

    #[test]
    fn test_convert_universal() {
        let mut interner = Interner::new();
        let var = interner.intern("x");
        let pred = interner.intern("P");

        let arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let body = arena.alloc(LogicExpr::Predicate {
            name: pred,
            args: term_arena.alloc_slice([Term::Variable(var)]),
            world: None,
        });

        let expr = LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: var,
            body,
            island_id: 0,
        };

        let result = logic_expr_to_proof_expr(&expr, &interner);

        match result {
            ProofExpr::ForAll { variable, body } => {
                assert_eq!(variable, "x");
                assert!(matches!(*body, ProofExpr::Predicate { .. }));
            }
            _ => panic!("Expected ForAll, got {:?}", result),
        }
    }

    #[test]
    fn test_convert_implication() {
        let mut interner = Interner::new();
        let p = interner.intern("P");
        let q = interner.intern("Q");

        let arena: Arena<LogicExpr> = Arena::new();

        let left = arena.alloc(LogicExpr::Atom(p));
        let right = arena.alloc(LogicExpr::Atom(q));

        let expr = LogicExpr::BinaryOp {
            left,
            op: TokenType::If,
            right,
        };

        let result = logic_expr_to_proof_expr(&expr, &interner);

        match result {
            ProofExpr::Implies(l, r) => {
                assert!(matches!(*l, ProofExpr::Atom(ref s) if s == "P"));
                assert!(matches!(*r, ProofExpr::Atom(ref s) if s == "Q"));
            }
            _ => panic!("Expected Implies, got {:?}", result),
        }
    }

    // ---- Solver-canonical arithmetic vocabulary (the parser↔oracle bridge) ----
    //
    // The parser names comparison directions "Greater"/"Less"/… and arithmetic
    // offsets "add"/"sub", but the proof oracle recognises ONLY the canonical
    // "Gt"/"Lt"/"Gte"/"Lte"/"Eq"/"Neq" predicates and "Add"/"Sub"/"Mul"/"Div"
    // functions, matched case-sensitively (oracle.rs:879, 1034;
    // modal_translation.rs:124). If the bridge does not translate to the canonical
    // vocabulary, every arithmetic/comparison clue silently degrades to an
    // uninterpreted function and never constrains the model — a parse that compiles
    // to FOL the prover cannot use. These pin the translation.

    #[test]
    fn arithmetic_offset_uses_canonical_add() {
        // "Tara scored 3 points higher than Bessie." → Score(tara) = add(Score(bessie), 3)
        let mut interner = Interner::new();
        let score = interner.intern("Score");
        let add = interner.intern("add");
        let tara = interner.intern("tara");
        let bessie = interner.intern("bessie");

        let terms: Arena<Term> = Arena::new();

        let score_tara = Term::Function(score, terms.alloc_slice([Term::Constant(tara)]));
        let score_bessie = Term::Function(score, terms.alloc_slice([Term::Constant(bessie)]));
        let offset = Term::Value {
            kind: crate::ast::logic::NumberKind::Integer(3),
            unit: None,
            dimension: None,
        };
        let rhs = Term::Function(add, terms.alloc_slice([score_bessie, offset]));
        let expr = LogicExpr::Identity {
            left: terms.alloc(score_tara),
            right: terms.alloc(rhs),
        };

        match logic_expr_to_proof_expr(&expr, &interner) {
            ProofExpr::Identity(_, ProofTerm::Function(name, args)) => {
                assert_eq!(name, "Add", "offset function must be canonical Add; got {name}");
                assert!(
                    matches!(&args[1], ProofTerm::Constant(s) if s == "3"),
                    "offset constant must be the bare integer 3; got {:?}",
                    args[1]
                );
            }
            other => panic!("expected Identity with an Add rhs, got {:?}", other),
        }
    }

    #[test]
    fn arithmetic_offset_uses_canonical_sub() {
        // "… 2 years before …" / "lower than" → ord(a) = sub(ord(b), 2)
        let mut interner = Interner::new();
        let ord = interner.intern("Ord");
        let sub = interner.intern("sub");
        let a = interner.intern("a");
        let b = interner.intern("b");
        let terms: Arena<Term> = Arena::new();
        let ord_a = Term::Function(ord, terms.alloc_slice([Term::Constant(a)]));
        let ord_b = Term::Function(ord, terms.alloc_slice([Term::Constant(b)]));
        let offset = Term::Value {
            kind: crate::ast::logic::NumberKind::Integer(2),
            unit: None,
            dimension: None,
        };
        let rhs = Term::Function(sub, terms.alloc_slice([ord_b, offset]));
        let expr = LogicExpr::Identity {
            left: terms.alloc(ord_a),
            right: terms.alloc(rhs),
        };
        match logic_expr_to_proof_expr(&expr, &interner) {
            ProofExpr::Identity(_, ProofTerm::Function(name, _)) => {
                assert_eq!(name, "Sub", "offset function must be canonical Sub; got {name}");
            }
            other => panic!("expected Identity with a Sub rhs, got {:?}", other),
        }
    }

    fn convert_binary_predicate(raw_name: &str) -> String {
        let mut interner = Interner::new();
        let name = interner.intern(raw_name);
        let a = interner.intern("a");
        let b = interner.intern("b");
        let terms: Arena<Term> = Arena::new();
        let args = terms.alloc_slice([Term::Constant(a), Term::Constant(b)]);
        let expr = LogicExpr::Predicate { name, args, world: None };
        match logic_expr_to_proof_expr(&expr, &interner) {
            ProofExpr::Predicate { name, .. } => name,
            other => panic!("expected Predicate, got {:?}", other),
        }
    }

    #[test]
    fn comparison_predicates_use_canonical_names() {
        // The parser's comparison vocabulary must reach the oracle's exact names,
        // and must NOT be lowercased into oblivion ("greater" ≠ "Gt").
        assert_eq!(convert_binary_predicate("Greater"), "Gt");
        assert_eq!(convert_binary_predicate("Less"), "Lt");
        assert_eq!(convert_binary_predicate("GreaterEqual"), "Gte");
        assert_eq!(convert_binary_predicate("LessEqual"), "Lte");
        assert_eq!(convert_binary_predicate("Equal"), "Eq");
        assert_eq!(convert_binary_predicate("NotEqual"), "Neq");
    }

    #[test]
    fn ordinary_binary_predicate_is_not_remapped_to_a_comparison() {
        // Regression: a genuine relational predicate is unaffected by the
        // comparison-name mapping — it keeps the noun-normalised lowercase form
        // and is never hijacked into an arithmetic comparison symbol.
        let name = convert_binary_predicate("Loves");
        assert!(
            !["Gt", "Lt", "Gte", "Lte", "Eq", "Neq"].contains(&name.as_str()),
            "ordinary predicate must not become a comparison op; got {name}"
        );
        assert_eq!(
            name,
            name.to_lowercase(),
            "ordinary predicate keeps the noun-normalised lowercase form; got {name}"
        );
    }
}
