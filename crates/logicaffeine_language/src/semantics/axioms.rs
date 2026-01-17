//! Axiom expansion for predicates using lexical semantics.
//!
//! This module enriches logical expressions by:
//!
//! - **Hypernym inference**: "Dog(x)" → "Dog(x) ∧ Animal(x)"
//! - **Verb entailments**: "Kill(e,x,y)" → "Kill(e,x,y) ∧ Cause(e,x,Dead(y))"
//! - **Privative adjectives**: "Fake gun" → "FakeGun(x) ∧ ¬Gun(x)"
//!
//! The transformations use lexicon data generated at compile time.

use logicaffeine_base::Arena;
use crate::ast::{LogicExpr, NeoEventData, Term, ThematicRole};
use logicaffeine_base::{Interner, Symbol};
use crate::lexicon::{lookup_canonical, Polarity};
use crate::token::TokenType;

use super::{is_privative_adjective, lookup_noun_entailments, lookup_noun_hypernyms, lookup_verb_entailment};

/// Apply axiom expansion to a logical expression.
pub fn apply_axioms<'a>(
    expr: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    transform_expr(expr, expr_arena, term_arena, interner)
}

fn transform_expr<'a>(
    expr: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Predicate { name, args, .. } => {
            expand_predicate(*name, args, expr_arena, term_arena, interner)
        }

        LogicExpr::Quantifier { kind, variable, body, island_id } => {
            let new_body = transform_expr(body, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Quantifier {
                kind: *kind,
                variable: *variable,
                body: new_body,
                island_id: *island_id,
            })
        }

        LogicExpr::BinaryOp { left, op, right } => {
            let new_left = transform_expr(left, expr_arena, term_arena, interner);
            let new_right = transform_expr(right, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::BinaryOp {
                left: new_left,
                op: op.clone(),
                right: new_right,
            })
        }

        LogicExpr::UnaryOp { op, operand } => {
            let new_operand = transform_expr(operand, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::UnaryOp {
                op: op.clone(),
                operand: new_operand,
            })
        }

        LogicExpr::NeoEvent(data) => {
            expand_neo_event(data, expr_arena, term_arena, interner)
        }

        LogicExpr::Modal { vector, operand } => {
            let new_operand = transform_expr(operand, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Modal {
                vector: *vector,
                operand: new_operand,
            })
        }

        LogicExpr::Temporal { operator, body } => {
            let new_body = transform_expr(body, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Temporal {
                operator: *operator,
                body: new_body,
            })
        }

        LogicExpr::Lambda { variable, body } => {
            let new_body = transform_expr(body, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Lambda {
                variable: *variable,
                body: new_body,
            })
        }

        LogicExpr::Question { wh_variable, body } => {
            let new_body = transform_expr(body, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Question {
                wh_variable: *wh_variable,
                body: new_body,
            })
        }

        LogicExpr::YesNoQuestion { body } => {
            let new_body = transform_expr(body, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::YesNoQuestion { body: new_body })
        }

        _ => expr,
    }
}

fn expand_predicate<'a>(
    name: Symbol,
    args: &'a [Term<'a>],
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    let name_str = interner.resolve(name).to_string();
    let lower_name = name_str.to_lowercase();

    // Check for canonical mapping (synonyms/antonyms)
    // E.g., Lack(x,y) -> ¬Have(x,y), Possess(x,y) -> Have(x,y)
    if let Some(mapping) = lookup_canonical(&lower_name) {
        let canonical_sym = interner.intern(mapping.lemma);
        let canonical_pred = expr_arena.alloc(LogicExpr::Predicate {
            name: canonical_sym,
            args,
            world: None,
        });

        // Wrap antonyms in negation
        return match mapping.polarity {
            Polarity::Positive => canonical_pred,
            Polarity::Negative => expr_arena.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: canonical_pred,
            }),
        };
    }

    // Check for compound predicates (e.g., Fake-Gun from non-intersective adjectives)
    if let Some(hyphen_pos) = name_str.find('-') {
        let adj_part = name_str[..hyphen_pos].to_string();
        let noun_part = name_str[hyphen_pos + 1..].to_string();

        if is_privative_adjective(&adj_part) {
            return expand_privative(&noun_part, args, expr_arena, term_arena, interner);
        }
    }

    // Check for noun entailments (Bachelor -> Unmarried + Male)
    let entailments = lookup_noun_entailments(&lower_name);
    if !entailments.is_empty() {
        return expand_noun_entailments(name, args, entailments, expr_arena, term_arena, interner);
    }

    // Check for hypernyms (Dog -> Animal)
    let hypernyms = lookup_noun_hypernyms(&lower_name);
    if !hypernyms.is_empty() {
        return expand_hypernyms(name, args, hypernyms, expr_arena, term_arena, interner);
    }

    // No expansion needed - return original
    expr_arena.alloc(LogicExpr::Predicate { name, args, world: None })
}

fn expand_privative<'a>(
    noun: &str,
    args: &'a [Term<'a>],
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    // Fake-Gun(x) => ¬Gun(x) ∧ Resembles(x, ^Gun)
    let noun_sym = interner.intern(noun);
    let resembles_sym = interner.intern("Resembles");

    // Gun(x)
    let noun_pred = expr_arena.alloc(LogicExpr::Predicate {
        name: noun_sym,
        args,
        world: None,
    });

    // ¬Gun(x)
    let negated_noun = expr_arena.alloc(LogicExpr::UnaryOp {
        op: TokenType::Not,
        operand: noun_pred,
    });

    // Resembles(x, ^Gun)
    let intension_term = Term::Intension(noun_sym);
    let mut resembles_args_vec = Vec::with_capacity(args.len() + 1);
    if !args.is_empty() {
        resembles_args_vec.push(clone_term(&args[0], term_arena));
    }
    resembles_args_vec.push(intension_term);
    let resembles_args = term_arena.alloc_slice(resembles_args_vec);

    let resembles_pred = expr_arena.alloc(LogicExpr::Predicate {
        name: resembles_sym,
        args: resembles_args,
        world: None,
    });

    // ¬Gun(x) ∧ Resembles(x, ^Gun)
    expr_arena.alloc(LogicExpr::BinaryOp {
        left: negated_noun,
        op: TokenType::And,
        right: resembles_pred,
    })
}

fn expand_noun_entailments<'a>(
    base: Symbol,
    args: &'a [Term<'a>],
    entailments: &[&str],
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    // Bachelor(x) => Bachelor(x) ∧ Unmarried(x) ∧ Male(x)
    let base_pred = expr_arena.alloc(LogicExpr::Predicate { name: base, args, world: None });

    let mut result: &LogicExpr = base_pred;
    for entailment in entailments {
        let ent_sym = interner.intern(entailment);
        let ent_pred = expr_arena.alloc(LogicExpr::Predicate {
            name: ent_sym,
            args,
            world: None,
        });
        result = expr_arena.alloc(LogicExpr::BinaryOp {
            left: result,
            op: TokenType::And,
            right: ent_pred,
        });
    }

    result
}

fn expand_hypernyms<'a>(
    base: Symbol,
    args: &'a [Term<'a>],
    hypernyms: &[&str],
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    // Dog(x) => Dog(x) ∧ Animal(x)
    let base_pred = expr_arena.alloc(LogicExpr::Predicate { name: base, args, world: None });

    let mut result: &LogicExpr = base_pred;
    for hypernym in hypernyms {
        let hyp_sym = interner.intern(hypernym);
        let hyp_pred = expr_arena.alloc(LogicExpr::Predicate {
            name: hyp_sym,
            args,
            world: None,
        });
        result = expr_arena.alloc(LogicExpr::BinaryOp {
            left: result,
            op: TokenType::And,
            right: hyp_pred,
        });
    }

    result
}

fn expand_neo_event<'a>(
    data: &NeoEventData<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    let verb_str = interner.resolve(data.verb);
    let lower_verb = verb_str.to_lowercase();

    // Check for canonical mapping (synonyms/antonyms)
    // E.g., Lack(x,y) -> ¬Have(x,y)
    if let Some(mapping) = lookup_canonical(&lower_verb) {
        let canonical_sym = interner.intern(mapping.lemma);

        // Create NeoEvent with canonical verb
        let canonical_event = expr_arena.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var: data.event_var,
            verb: canonical_sym,
            roles: data.roles,
            modifiers: data.modifiers,
            suppress_existential: data.suppress_existential,
            world: None,
        })));

        // Wrap antonyms in negation
        return match mapping.polarity {
            Polarity::Positive => canonical_event,
            Polarity::Negative => expr_arena.alloc(LogicExpr::UnaryOp {
                op: TokenType::Not,
                operand: canonical_event,
            }),
        };
    }

    if let Some((base_verb, manner_preds)) = lookup_verb_entailment(&lower_verb) {
        // Murder(e) => Murder(e) ∧ Kill(e) ∧ Intentional(Agent)
        let base_verb_sym = interner.intern(base_verb);

        // Keep original NeoEvent
        let original = expr_arena.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var: data.event_var,
            verb: data.verb,
            roles: data.roles,
            modifiers: data.modifiers,
            suppress_existential: data.suppress_existential,
            world: None,
        })));

        // Create entailed verb NeoEvent (e.g., Kill)
        let entailed_event = expr_arena.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var: data.event_var,
            verb: base_verb_sym,
            roles: data.roles,
            modifiers: data.modifiers,
            suppress_existential: data.suppress_existential,
            world: None,
        })));

        // Conjoin original with entailed
        let mut result = expr_arena.alloc(LogicExpr::BinaryOp {
            left: original,
            op: TokenType::And,
            right: entailed_event,
        });

        // Add manner predicates (e.g., Intentional(Agent))
        for manner in manner_preds {
            let manner_sym = interner.intern(manner);

            // Find the agent in roles
            let agent_term = data.roles.iter()
                .find(|(role, _)| *role == ThematicRole::Agent)
                .map(|(_, term)| term);

            if let Some(agent) = agent_term {
                let manner_args = term_arena.alloc_slice([clone_term(agent, term_arena)]);
                let manner_pred = expr_arena.alloc(LogicExpr::Predicate {
                    name: manner_sym,
                    args: manner_args,
                    world: None,
                });
                result = expr_arena.alloc(LogicExpr::BinaryOp {
                    left: result,
                    op: TokenType::And,
                    right: manner_pred,
                });
            }
        }

        result
    } else {
        // No entailment - return original unchanged
        expr_arena.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
            event_var: data.event_var,
            verb: data.verb,
            roles: data.roles,
            modifiers: data.modifiers,
            suppress_existential: data.suppress_existential,
            world: None,
        })))
    }
}

fn clone_term<'a>(term: &Term<'a>, arena: &'a Arena<Term<'a>>) -> Term<'a> {
    match term {
        Term::Constant(s) => Term::Constant(*s),
        Term::Variable(s) => Term::Variable(*s),
        Term::Function(s, args) => {
            let cloned_args: Vec<Term<'a>> = args.iter().map(|t| clone_term(t, arena)).collect();
            Term::Function(*s, arena.alloc_slice(cloned_args))
        }
        Term::Group(terms) => {
            let cloned: Vec<Term<'a>> = terms.iter().map(|t| clone_term(t, arena)).collect();
            Term::Group(arena.alloc_slice(cloned))
        }
        Term::Possessed { possessor, possessed } => {
            let cloned_possessor = arena.alloc(clone_term(possessor, arena));
            Term::Possessed { possessor: cloned_possessor, possessed: *possessed }
        }
        Term::Sigma(s) => Term::Sigma(*s),
        Term::Intension(s) => Term::Intension(*s),
        Term::Proposition(e) => Term::Proposition(*e),
        Term::Value { kind, unit, dimension } => Term::Value {
            kind: *kind,
            unit: *unit,
            dimension: *dimension,
        },
    }
}
