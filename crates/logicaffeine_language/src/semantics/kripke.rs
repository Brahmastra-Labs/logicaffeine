//! Kripke Semantics Lowering Pass
//!
//! Transforms modal operators into explicit possible world semantics:
//! - Diamond P (force <= 0.5) → Exists w'(Accessible(w, w') And P(w'))
//! - Box P (force > 0.5) → ForAll w'(Accessible(w, w') Implies P(w'))

use logicaffeine_base::Arena;
use crate::ast::{LogicExpr, ModalDomain, ModalVector, NeoEventData, QuantifierKind, Term};
use logicaffeine_base::{Interner, Symbol};
use crate::token::TokenType;

/// Context for tracking world variables during Kripke lowering.
///
/// For hardware verification, `clock_counter` tracks discrete timesteps
/// and `domain_hint` disambiguates temporal vs modal lowering when
/// the context is ambiguous.
pub struct KripkeContext {
    world_counter: u32,
    current_world: Symbol,
    /// Discrete timestep counter for hardware clock cycles.
    /// Incremented when generating Next_Temporal worlds.
    clock_counter: u32,
    /// Hint for the current modal domain being lowered.
    /// Set to `Some(Temporal)` when processing temporal operators,
    /// enabling domain-aware disambiguation.
    domain_hint: Option<ModalDomain>,
}

impl KripkeContext {
    pub fn new(interner: &mut Interner) -> Self {
        Self {
            world_counter: 0,
            current_world: interner.intern("w0"),
            clock_counter: 0,
            domain_hint: None,
        }
    }

    pub fn fresh_world(&mut self, interner: &mut Interner) -> Symbol {
        self.world_counter += 1;
        interner.intern(&format!("w{}", self.world_counter))
    }

    /// Get the current clock counter value (discrete timestep).
    pub fn clock_counter(&self) -> u32 {
        self.clock_counter
    }

    /// Get the current domain hint.
    pub fn domain_hint(&self) -> Option<ModalDomain> {
        self.domain_hint
    }

    /// Advance the clock counter by one tick (for Next_Temporal).
    fn tick_clock(&mut self) {
        self.clock_counter += 1;
    }

    /// Set the domain hint for the current lowering context.
    fn set_domain_hint(&mut self, domain: ModalDomain) {
        self.domain_hint = Some(domain);
    }

    /// Clear the domain hint.
    fn clear_domain_hint(&mut self) {
        self.domain_hint = None;
    }
}

/// Apply Kripke lowering to transform modal operators into explicit world quantification.
///
/// This transforms surface modal operators (`◇`, `□`) into explicit First-Order Logic
/// with possible world semantics, enabling Z3 verification.
///
/// ## Example
/// Surface: `◇Fly(x)` (John can fly)
/// Deep: `∃w'(Accessible(w₀, w') ∧ Fly(x, w'))` (There exists an accessible world where John flies)
pub fn apply_kripke_lowering<'a>(
    expr: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    let mut ctx = KripkeContext::new(interner);
    lower_expr(expr, &mut ctx, expr_arena, term_arena, interner)
}

fn lower_expr<'a>(
    expr: &'a LogicExpr<'a>,
    ctx: &mut KripkeContext,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Modal { vector, operand } => {
            lower_modal(vector, operand, ctx, expr_arena, term_arena, interner)
        }

        LogicExpr::Predicate { name, args, world } => {
            if world.is_none() {
                // Add current world to predicate
                expr_arena.alloc(LogicExpr::Predicate {
                    name: *name,
                    args: *args,
                    world: Some(ctx.current_world),
                })
            } else {
                expr
            }
        }

        LogicExpr::Quantifier { kind, variable, body, island_id } => {
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Quantifier {
                kind: *kind,
                variable: *variable,
                body: new_body,
                island_id: *island_id,
            })
        }

        LogicExpr::BinaryOp { left, op, right } => {
            let new_left = lower_expr(left, ctx, expr_arena, term_arena, interner);
            let new_right = lower_expr(right, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::BinaryOp {
                left: new_left,
                op: op.clone(),
                right: new_right,
            })
        }

        LogicExpr::UnaryOp { op, operand } => {
            let new_operand = lower_expr(operand, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::UnaryOp {
                op: op.clone(),
                operand: new_operand,
            })
        }

        LogicExpr::NeoEvent(data) => {
            // Set the world on the event to the current world
            if data.world.is_none() {
                expr_arena.alloc(LogicExpr::NeoEvent(Box::new(NeoEventData {
                    event_var: data.event_var,
                    verb: data.verb,
                    roles: data.roles,
                    modifiers: data.modifiers,
                    suppress_existential: data.suppress_existential,
                    world: Some(ctx.current_world),
                })))
            } else {
                expr
            }
        }

        LogicExpr::Temporal { operator, body } => {
            use crate::ast::logic::TemporalOperator;
            // Set domain hint for temporal lowering
            ctx.set_domain_hint(ModalDomain::Temporal);
            let result = match operator {
                // LTL operators → Kripke world quantification
                TemporalOperator::Always => {
                    // G(φ) → ∀w'(Accessible_Temporal(w, w') → φ(w'))
                    lower_temporal_unary(
                        body, ctx, expr_arena, term_arena, interner,
                        "Accessible_Temporal", true,
                    )
                }
                TemporalOperator::Eventually => {
                    // F(φ) → ∃w'(Reachable_Temporal(w, w') ∧ φ(w'))
                    lower_temporal_unary(
                        body, ctx, expr_arena, term_arena, interner,
                        "Reachable_Temporal", false,
                    )
                }
                TemporalOperator::Next => {
                    // X(φ) → ∀w'(Next_Temporal(w, w') → φ(w'))
                    ctx.tick_clock();
                    lower_temporal_unary(
                        body, ctx, expr_arena, term_arena, interner,
                        "Next_Temporal", true,
                    )
                }
                // Priorian tense operators — pass through (linguistic, not hardware)
                TemporalOperator::Past | TemporalOperator::Future => {
                    let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
                    expr_arena.alloc(LogicExpr::Temporal {
                        operator: *operator,
                        body: new_body,
                    })
                }
            };
            ctx.clear_domain_hint();
            result
        }

        LogicExpr::TemporalBinary { operator, left, right } => {
            // φ U ψ → ψ(w) ∨ (φ(w) ∧ ∃w'(Next_Temporal(w,w') ∧ (φ U ψ)(w')))
            // For now: lower both operands with world threading
            let new_left = lower_expr(left, ctx, expr_arena, term_arena, interner);
            let new_right = lower_expr(right, ctx, expr_arena, term_arena, interner);

            // Generate Next_Temporal accessibility for the recursive step
            let source_world = ctx.current_world;
            let target_world = ctx.fresh_world(interner);
            let next_name = interner.intern("Next_Temporal");
            let accessibility = expr_arena.alloc(LogicExpr::Predicate {
                name: next_name,
                args: term_arena.alloc_slice([
                    Term::Variable(source_world),
                    Term::Variable(target_world),
                ]),
                world: None,
            });

            // Build: right(w) ∨ (left(w) ∧ ∃w'(Next_Temporal(w,w') ∧ ...))
            let recursive_body = expr_arena.alloc(LogicExpr::BinaryOp {
                left: accessibility,
                op: crate::token::TokenType::And,
                right: expr_arena.alloc(LogicExpr::TemporalBinary {
                    operator: *operator,
                    left: new_left,
                    right: new_right,
                }),
            });
            let existential = expr_arena.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: target_world,
                body: recursive_body,
                island_id: 0,
            });
            let left_and_next = expr_arena.alloc(LogicExpr::BinaryOp {
                left: new_left,
                op: crate::token::TokenType::And,
                right: existential,
            });
            expr_arena.alloc(LogicExpr::BinaryOp {
                left: new_right,
                op: crate::token::TokenType::Or,
                right: left_and_next,
            })
        }

        LogicExpr::Aspectual { operator, body } => {
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Aspectual {
                operator: *operator,
                body: new_body,
            })
        }

        LogicExpr::Voice { operator, body } => {
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Voice {
                operator: *operator,
                body: new_body,
            })
        }

        LogicExpr::Lambda { variable, body } => {
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Lambda {
                variable: *variable,
                body: new_body,
            })
        }

        LogicExpr::App { function, argument } => {
            let new_function = lower_expr(function, ctx, expr_arena, term_arena, interner);
            let new_argument = lower_expr(argument, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::App {
                function: new_function,
                argument: new_argument,
            })
        }

        LogicExpr::Intensional { operator, content } => {
            let new_content = lower_expr(content, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Intensional {
                operator: *operator,
                content: new_content,
            })
        }

        LogicExpr::Control { verb, subject, object, infinitive } => {
            let new_infinitive = lower_expr(infinitive, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Control {
                verb: *verb,
                subject: *subject,
                object: *object,
                infinitive: new_infinitive,
            })
        }

        LogicExpr::Scopal { operator, body } => {
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Scopal {
                operator: *operator,
                body: new_body,
            })
        }

        LogicExpr::Question { wh_variable, body } => {
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Question {
                wh_variable: *wh_variable,
                body: new_body,
            })
        }

        LogicExpr::YesNoQuestion { body } => {
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::YesNoQuestion { body: new_body })
        }

        LogicExpr::Focus { kind, focused, scope } => {
            let new_scope = lower_expr(scope, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Focus {
                kind: *kind,
                focused: *focused,
                scope: new_scope,
            })
        }

        LogicExpr::Distributive { predicate } => {
            let new_predicate = lower_expr(predicate, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Distributive {
                predicate: new_predicate,
            })
        }

        LogicExpr::Counterfactual { antecedent, consequent } => {
            let new_antecedent = lower_expr(antecedent, ctx, expr_arena, term_arena, interner);
            let new_consequent = lower_expr(consequent, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Counterfactual {
                antecedent: new_antecedent,
                consequent: new_consequent,
            })
        }

        LogicExpr::Event { predicate, adverbs } => {
            let new_predicate = lower_expr(predicate, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Event {
                predicate: new_predicate,
                adverbs: *adverbs,
            })
        }

        LogicExpr::Imperative { action } => {
            let new_action = lower_expr(action, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Imperative { action: new_action })
        }

        LogicExpr::Causal { effect, cause } => {
            let new_effect = lower_expr(effect, ctx, expr_arena, term_arena, interner);
            let new_cause = lower_expr(cause, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Causal {
                effect: new_effect,
                cause: new_cause,
            })
        }

        LogicExpr::Presupposition { assertion, presupposition } => {
            let new_assertion = lower_expr(assertion, ctx, expr_arena, term_arena, interner);
            let new_presupposition = lower_expr(presupposition, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::Presupposition {
                assertion: new_assertion,
                presupposition: new_presupposition,
            })
        }

        LogicExpr::TemporalAnchor { anchor, body } => {
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::TemporalAnchor {
                anchor: *anchor,
                body: new_body,
            })
        }

        LogicExpr::GroupQuantifier { group_var, count, member_var, restriction, body } => {
            let new_restriction = lower_expr(restriction, ctx, expr_arena, term_arena, interner);
            let new_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
            expr_arena.alloc(LogicExpr::GroupQuantifier {
                group_var: *group_var,
                count: *count,
                member_var: *member_var,
                restriction: new_restriction,
                body: new_body,
            })
        }

        // Leaf nodes that don't need transformation
        LogicExpr::Identity { .. }
        | LogicExpr::Metaphor { .. }
        | LogicExpr::Categorical(_)
        | LogicExpr::Relation(_)
        | LogicExpr::Atom(_)
        | LogicExpr::Superlative { .. }
        | LogicExpr::Comparative { .. }
        | LogicExpr::SpeechAct { .. } => expr,
    }
}

fn lower_modal<'a>(
    vector: &ModalVector,
    operand: &'a LogicExpr<'a>,
    ctx: &mut KripkeContext,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
) -> &'a LogicExpr<'a> {
    let source_world = ctx.current_world;
    let target_world = ctx.fresh_world(interner);

    // Lower operand with new current world
    let old_world = ctx.current_world;
    ctx.current_world = target_world;
    let lowered_operand = lower_expr(operand, ctx, expr_arena, term_arena, interner);
    ctx.current_world = old_world;

    // Create accessibility predicate based on modal domain
    let access_name = match vector.domain {
        ModalDomain::Alethic => interner.intern("Accessible_Alethic"),
        ModalDomain::Deontic => interner.intern("Accessible_Deontic"),
        ModalDomain::Temporal => interner.intern("Accessible_Temporal"),
    };

    let accessibility = expr_arena.alloc(LogicExpr::Predicate {
        name: access_name,
        args: term_arena.alloc_slice([
            Term::Variable(source_world),
            Term::Variable(target_world),
        ]),
        world: None, // Accessibility predicate is a meta-level relation
    });

    if vector.force > 0.5 {
        // Necessity (Box): ForAll w'(Accessible(w, w') -> P(w'))
        let implication = expr_arena.alloc(LogicExpr::BinaryOp {
            left: accessibility,
            op: TokenType::Implies,
            right: lowered_operand,
        });
        expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: target_world,
            body: implication,
            island_id: 0,
        })
    } else {
        // Possibility (Diamond): Exists w'(Accessible(w, w') ∧ P(w'))
        let conjunction = expr_arena.alloc(LogicExpr::BinaryOp {
            left: accessibility,
            op: TokenType::And,
            right: lowered_operand,
        });
        expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: target_world,
            body: conjunction,
            island_id: 0,
        })
    }
}

/// Lower a unary LTL temporal operator into Kripke world quantification.
///
/// `is_universal`: true for G/X (∀w'), false for F (∃w')
fn lower_temporal_unary<'a>(
    body: &'a LogicExpr<'a>,
    ctx: &mut KripkeContext,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    interner: &mut Interner,
    predicate_name: &str,
    is_universal: bool,
) -> &'a LogicExpr<'a> {
    let source_world = ctx.current_world;
    let target_world = ctx.fresh_world(interner);

    // Lower body with new current world
    let old_world = ctx.current_world;
    ctx.current_world = target_world;
    let lowered_body = lower_expr(body, ctx, expr_arena, term_arena, interner);
    ctx.current_world = old_world;

    // Create temporal accessibility predicate
    let access_name = interner.intern(predicate_name);
    let accessibility = expr_arena.alloc(LogicExpr::Predicate {
        name: access_name,
        args: term_arena.alloc_slice([
            Term::Variable(source_world),
            Term::Variable(target_world),
        ]),
        world: None,
    });

    if is_universal {
        // G/X: ∀w'(Accessible_Temporal(w, w') → φ(w'))
        let implication = expr_arena.alloc(LogicExpr::BinaryOp {
            left: accessibility,
            op: TokenType::Implies,
            right: lowered_body,
        });
        expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: target_world,
            body: implication,
            island_id: 0,
        })
    } else {
        // F: ∃w'(Reachable_Temporal(w, w') ∧ φ(w'))
        let conjunction = expr_arena.alloc(LogicExpr::BinaryOp {
            left: accessibility,
            op: TokenType::And,
            right: lowered_body,
        });
        expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: target_world,
            body: conjunction,
            island_id: 0,
        })
    }
}
