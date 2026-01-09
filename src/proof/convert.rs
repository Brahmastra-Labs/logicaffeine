// =============================================================================
// PROOF ENGINE - AST CONVERSION (PHASE 63)
// =============================================================================
//
// This module bridges the parser's arena-allocated AST (LogicExpr<'a>) to the
// proof engine's owned representation (ProofExpr).
//
// The conversion clones all data into owned Strings, enabling proof trees
// to persist beyond the arena's lifetime.

use crate::ast::logic::{
    LogicExpr, ModalDomain, ModalFlavor, QuantifierKind, TemporalOperator, Term, ThematicRole,
};
use crate::intern::Interner;
use crate::lexicon::get_canonical_noun;
use crate::proof::{ProofExpr, ProofTerm};
use crate::token::TokenType;

// =============================================================================
// PUBLIC API
// =============================================================================

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
            let normalized = get_canonical_noun(&name_str.to_lowercase())
                .map(|lemma| lemma.to_lowercase())
                .unwrap_or_else(|| name_str.to_lowercase());

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
                QuantifierKind::Cardinal(n) => {
                    // Cardinal(n) is existential in proof context
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
                TokenType::If | TokenType::Then => ProofExpr::Implies(l, r),
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
            };
            let flavor = match vector.flavor {
                ModalFlavor::Root => "Root",
                ModalFlavor::Epistemic => "Epistemic",
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
            };
            ProofExpr::Temporal {
                operator: op_name.to_string(),
                body: body_expr,
            }
        }

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
            // Counterfactuals become implications in classical logic
            ProofExpr::Implies(
                Box::new(logic_expr_to_proof_expr(antecedent, interner)),
                Box::new(logic_expr_to_proof_expr(consequent, interner)),
            )
        }

        // --- Unsupported constructs (return Unsupported variant) ---
        LogicExpr::Categorical(_) => ProofExpr::Unsupported("Categorical (legacy)".into()),
        LogicExpr::Relation(_) => ProofExpr::Unsupported("Relation (legacy)".into()),
        LogicExpr::Metaphor { .. } => ProofExpr::Unsupported("Metaphor".into()),
        LogicExpr::Question { .. } => ProofExpr::Unsupported("Question".into()),
        LogicExpr::YesNoQuestion { .. } => ProofExpr::Unsupported("YesNoQuestion".into()),
        LogicExpr::Intensional { .. } => ProofExpr::Unsupported("Intensional".into()),
        LogicExpr::Event { .. } => ProofExpr::Unsupported("Event (legacy)".into()),
        LogicExpr::Imperative { .. } => ProofExpr::Unsupported("Imperative".into()),
        LogicExpr::SpeechAct { .. } => ProofExpr::Unsupported("SpeechAct".into()),
        LogicExpr::Causal { .. } => ProofExpr::Unsupported("Causal".into()),
        LogicExpr::Comparative { .. } => ProofExpr::Unsupported("Comparative".into()),
        LogicExpr::Superlative { .. } => ProofExpr::Unsupported("Superlative".into()),
        LogicExpr::Scopal { .. } => ProofExpr::Unsupported("Scopal".into()),
        LogicExpr::Control { .. } => ProofExpr::Unsupported("Control".into()),
        LogicExpr::Presupposition { .. } => ProofExpr::Unsupported("Presupposition".into()),
        LogicExpr::Focus { .. } => ProofExpr::Unsupported("Focus".into()),
        LogicExpr::TemporalAnchor { .. } => ProofExpr::Unsupported("TemporalAnchor".into()),
        LogicExpr::Distributive { .. } => ProofExpr::Unsupported("Distributive".into()),
        LogicExpr::GroupQuantifier { .. } => ProofExpr::Unsupported("GroupQuantifier".into()),
        // Aspectual wrappers (Imperfective, Perfective, etc.) are transparent to proof.
        // "John runs" -> Aspectual(Imperfective, ∃e(Run(e) ∧ Agent(e, John)))
        // We pass through to the inner event structure.
        LogicExpr::Aspectual { body, .. } => logic_expr_to_proof_expr(body, interner),
        LogicExpr::Voice { .. } => ProofExpr::Unsupported("Voice".into()),
    }
}

/// Convert a Term to ProofTerm.
pub fn term_to_proof_term<'a>(term: &Term<'a>, interner: &Interner) -> ProofTerm {
    match term {
        Term::Constant(s) => ProofTerm::Constant(interner.resolve(*s).to_string()),

        Term::Variable(s) => ProofTerm::Variable(interner.resolve(*s).to_string()),

        Term::Function(name, args) => ProofTerm::Function(
            interner.resolve(*name).to_string(),
            args.iter().map(|t| term_to_proof_term(t, interner)).collect(),
        ),

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
}
