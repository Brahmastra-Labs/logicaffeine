//! Lambda calculus transformations for Montague-style compositional semantics.
//!
//! This module provides functions for manipulating logical expressions using
//! lambda abstraction and application. It supports:
//!
//! - **Quantifier raising** via type-lifting (proper names → generalized quantifiers)
//! - **Beta reduction** for applying lambda abstractions
//! - **Scope enumeration** to generate all possible quantifier scopings
//! - **Intensional contexts** for opaque verbs (believes, seeks, wants)
//! - **Event semantics** transformation (Davidson-style)
//!
//! # Key Functions
//!
//! | Function | Purpose |
//! |----------|---------|
//! | [`lift_proper_name`] | Lifts a proper name to a generalized quantifier |
//! | [`lift_quantifier`] | Creates a quantifier lambda term |
//! | [`beta_reduce`] | Performs beta reduction on applications |
//! | [`enumerate_scopings`] | Generates all scope permutations respecting islands |
//! | [`enumerate_intensional_readings`] | Generates de re / de dicto readings |
//! | [`to_event_semantics`] | Converts predicate to neo-Davidsonian event form |
//!
//! # Scope Islands
//!
//! Quantifiers are assigned an `island_id` during parsing. The scoping algorithm
//! only permutes quantifiers within the same island. This prevents:
//! - Wh-extraction from relative clauses
//! - Quantifier raising out of adjuncts
//! - Other island constraint violations
//!
//! # Intensionality
//!
//! Opaque verbs create intensional contexts where substitution of co-referential
//! terms may change truth value. The module provides:
//! - `make_intensional`: Wraps an expression in an intensional operator
//! - `substitute_respecting_opacity`: Blocks substitution inside intensional contexts

use crate::ast::{LogicExpr, QuantifierKind, Term};
use crate::lexicon;
use crate::token::TokenType;
use logicaffeine_base::Arena;
use logicaffeine_base::{Interner, Symbol};

fn clone_term<'a>(term: &Term<'a>, arena: &'a Arena<Term<'a>>) -> Term<'a> {
    match term {
        Term::Constant(s) => Term::Constant(*s),
        Term::Variable(s) => Term::Variable(*s),
        Term::Function(name, args) => {
            let cloned_args: Vec<Term<'a>> = args.iter().map(|t| clone_term(t, arena)).collect();
            Term::Function(*name, arena.alloc_slice(cloned_args))
        }
        Term::Group(members) => {
            let cloned: Vec<Term<'a>> = members.iter().map(|t| clone_term(t, arena)).collect();
            Term::Group(arena.alloc_slice(cloned))
        }
        Term::Possessed {
            possessor,
            possessed,
        } => Term::Possessed {
            possessor: arena.alloc(clone_term(possessor, arena)),
            possessed: *possessed,
        },
        Term::Sigma(predicate) => Term::Sigma(*predicate),
        Term::Intension(predicate) => Term::Intension(*predicate),
        Term::Kind(kind) => Term::Kind(*kind),
        Term::Proposition(expr) => Term::Proposition(*expr),
        Term::Value {
            kind,
            unit,
            dimension,
        } => Term::Value {
            kind: *kind,
            unit: *unit,
            dimension: *dimension,
        },
    }
}

/// Checks if a verb creates an opaque (intensional) context.
///
/// Opaque verbs like "believes", "wants", "seeks" block substitution
/// of co-referential terms. For example:
/// - "John believes Clark Kent flies" ≠ "John believes Superman flies"
pub fn is_opaque_verb(verb: Symbol, interner: &Interner) -> bool {
    let verb_str = interner.resolve(verb);
    let lower = verb_str.to_lowercase();
    lexicon::is_opaque_verb(&lower)
}

/// Wraps an expression in an intensional operator.
///
/// Creates `operator[content]` for de dicto readings of intensional verbs.
pub fn make_intensional<'a>(
    operator: Symbol,
    content: &'a LogicExpr<'a>,
    arena: &'a Arena<LogicExpr<'a>>,
) -> &'a LogicExpr<'a> {
    arena.alloc(LogicExpr::Intensional { operator, content })
}

/// Substitutes a variable with a replacement, respecting opacity boundaries.
///
/// Unlike standard substitution, this function does NOT substitute inside
/// intensional contexts, preserving the opacity of verbs like "believes".
pub fn substitute_respecting_opacity<'a>(
    expr: &'a LogicExpr<'a>,
    var: Symbol,
    replacement: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Intensional { operator, content } => expr_arena.alloc(LogicExpr::Intensional {
            operator: *operator,
            content: *content,
        }),

        LogicExpr::Predicate { name, args, .. } => {
            let new_args: Vec<Term<'a>> = args
                .iter()
                .map(|arg| substitute_term_for_opacity(arg, var, replacement, term_arena))
                .collect();
            expr_arena.alloc(LogicExpr::Predicate {
                name: *name,
                args: term_arena.alloc_slice(new_args),
                world: None,
            })
        }

        LogicExpr::BinaryOp { left, op, right } => expr_arena.alloc(LogicExpr::BinaryOp {
            left: substitute_respecting_opacity(left, var, replacement, expr_arena, term_arena),
            op: op.clone(),
            right: substitute_respecting_opacity(right, var, replacement, expr_arena, term_arena),
        }),

        LogicExpr::UnaryOp { op, operand } => expr_arena.alloc(LogicExpr::UnaryOp {
            op: op.clone(),
            operand: substitute_respecting_opacity(
                operand,
                var,
                replacement,
                expr_arena,
                term_arena,
            ),
        }),

        LogicExpr::Quantifier {
            kind,
            variable,
            body,
            island_id,
        } => {
            if *variable == var {
                expr
            } else {
                expr_arena.alloc(LogicExpr::Quantifier {
                    kind: *kind,
                    variable: *variable,
                    body: substitute_respecting_opacity(
                        body,
                        var,
                        replacement,
                        expr_arena,
                        term_arena,
                    ),
                    island_id: *island_id,
                })
            }
        }

        LogicExpr::Lambda { variable, body } => {
            if *variable == var {
                expr
            } else {
                expr_arena.alloc(LogicExpr::Lambda {
                    variable: *variable,
                    body: substitute_respecting_opacity(
                        body,
                        var,
                        replacement,
                        expr_arena,
                        term_arena,
                    ),
                })
            }
        }

        LogicExpr::App { function, argument } => expr_arena.alloc(LogicExpr::App {
            function: substitute_respecting_opacity(
                function,
                var,
                replacement,
                expr_arena,
                term_arena,
            ),
            argument: substitute_respecting_opacity(
                argument,
                var,
                replacement,
                expr_arena,
                term_arena,
            ),
        }),

        LogicExpr::Atom(s) => {
            if *s == var {
                replacement
            } else {
                expr
            }
        }

        _ => expr,
    }
}

fn substitute_term_for_opacity<'a>(
    term: &Term<'a>,
    var: Symbol,
    replacement: &LogicExpr<'a>,
    arena: &'a Arena<Term<'a>>,
) -> Term<'a> {
    match term {
        Term::Constant(c) if *c == var => match replacement {
            LogicExpr::Atom(s) => Term::Constant(*s),
            _ => clone_term(term, arena),
        },
        Term::Variable(v) if *v == var => match replacement {
            LogicExpr::Atom(s) => Term::Constant(*s),
            _ => clone_term(term, arena),
        },
        _ => clone_term(term, arena),
    }
}

/// Converts a predicate to Neo-Davidsonian event semantics.
///
/// Transforms `P(x, y)` into `∃e(P(e) ∧ Agent(e, x) ∧ Theme(e, y))`.
/// This reification of events enables reasoning about event modification.
pub fn to_event_semantics<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Predicate { name, args, .. } => {
            let e_sym = interner.intern("e");
            let _event_var = term_arena.alloc(Term::Variable(e_sym));

            let event_pred = expr_arena.alloc(LogicExpr::Predicate {
                name: *name,
                args: term_arena.alloc_slice([Term::Variable(e_sym)]),
                world: None,
            });

            let mut body = event_pred;

            if !args.is_empty() {
                let agent_args = term_arena
                    .alloc_slice([Term::Variable(e_sym), clone_term(&args[0], term_arena)]);
                let agent_pred = expr_arena.alloc(LogicExpr::Predicate {
                    name: interner.intern("Agent"),
                    args: agent_args,
                    world: None,
                });
                body = expr_arena.alloc(LogicExpr::BinaryOp {
                    left: body,
                    op: TokenType::And,
                    right: agent_pred,
                });
            }

            if args.len() > 1 {
                let theme_args = term_arena
                    .alloc_slice([Term::Variable(e_sym), clone_term(&args[1], term_arena)]);
                let theme_pred = expr_arena.alloc(LogicExpr::Predicate {
                    name: interner.intern("Theme"),
                    args: theme_args,
                    world: None,
                });
                body = expr_arena.alloc(LogicExpr::BinaryOp {
                    left: body,
                    op: TokenType::And,
                    right: theme_pred,
                });
            }

            if args.len() > 2 {
                let goal_args = term_arena
                    .alloc_slice([Term::Variable(e_sym), clone_term(&args[2], term_arena)]);
                let goal_pred = expr_arena.alloc(LogicExpr::Predicate {
                    name: interner.intern("Goal"),
                    args: goal_args,
                    world: None,
                });
                body = expr_arena.alloc(LogicExpr::BinaryOp {
                    left: body,
                    op: TokenType::And,
                    right: goal_pred,
                });
            }

            expr_arena.alloc(LogicExpr::Quantifier {
                kind: QuantifierKind::Existential,
                variable: e_sym,
                body,
                island_id: 0,
            })
        }
        _ => expr,
    }
}

/// Adds an adverbial modifier to an event.
///
/// Transforms `∃e(P(e) ∧ ...)` into `∃e(P(e) ∧ Adverb(e) ∧ ...)`.
pub fn apply_adverb<'a>(
    expr: &'a LogicExpr<'a>,
    adverb: Symbol,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    let e_sym = interner.intern("e");
    match expr {
        LogicExpr::Quantifier {
            kind,
            variable,
            body,
            island_id,
        } if *variable == e_sym => {
            let adverb_str = interner.resolve(adverb);
            let capitalized = capitalize(adverb_str);
            let adverb_pred = expr_arena.alloc(LogicExpr::Predicate {
                name: interner.intern(&capitalized),
                args: term_arena.alloc_slice([Term::Variable(*variable)]),
                world: None,
            });

            let new_body = expr_arena.alloc(LogicExpr::BinaryOp {
                left: *body,
                op: TokenType::And,
                right: adverb_pred,
            });

            expr_arena.alloc(LogicExpr::Quantifier {
                kind: *kind,
                variable: *variable,
                body: new_body,
                island_id: *island_id,
            })
        }
        _ => expr,
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn factorial(n: usize) -> u64 {
    (1..=n as u64).product()
}

pub struct ScopeIterator<'a> {
    expr_arena: &'a Arena<LogicExpr<'a>>,
    islands: Vec<Vec<ScopalElement<'a>>>,
    core: &'a LogicExpr<'a>,
    current_index: u64,
    total: u64,
    single_result: Option<&'a LogicExpr<'a>>,
    returned_single: bool,
}

impl<'a> ScopeIterator<'a> {
    fn nth_island_aware_permutation(&self, n: u64) -> Vec<ScopalElement<'a>> {
        let mut result = Vec::new();
        let mut remainder = n;

        for island in &self.islands {
            let island_perms = factorial(island.len());
            let island_index = remainder % island_perms;
            remainder /= island_perms;

            let perm = nth_permutation_of_slice(island, island_index);
            result.extend(perm);
        }

        result
    }
}

fn nth_permutation_of_slice<T: Clone>(items: &[T], n: u64) -> Vec<T> {
    let len = items.len();
    let mut available: Vec<usize> = (0..len).collect();
    let mut result = Vec::with_capacity(len);
    let mut remainder = n;

    for i in 0..len {
        let divisor = factorial(len - i - 1);
        let index = (remainder / divisor) as usize;
        remainder %= divisor;
        result.push(items[available.remove(index)].clone());
    }
    result
}

impl<'a> Iterator for ScopeIterator<'a> {
    type Item = &'a LogicExpr<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(single) = self.single_result {
            if self.returned_single {
                return None;
            }
            self.returned_single = true;
            return Some(single);
        }

        if self.current_index >= self.total {
            return None;
        }
        let ordered = self.nth_island_aware_permutation(self.current_index);
        self.current_index += 1;
        Some(rebuild_with_scopal_elements(
            &ordered,
            self.core,
            self.expr_arena,
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.single_result.is_some() {
            let remaining = if self.returned_single { 0 } else { 1 };
            return (remaining, Some(remaining));
        }
        let remaining = (self.total - self.current_index) as usize;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for ScopeIterator<'a> {}

#[derive(Clone, Debug)]
struct QuantifierInfo<'a> {
    kind: QuantifierKind,
    variable: Symbol,
    restrictor: &'a LogicExpr<'a>,
    island_id: u32,
}

#[derive(Clone, Debug)]
enum ScopalElement<'a> {
    Quantifier(QuantifierInfo<'a>),
    Negation { island_id: u32 },
}

impl<'a> ScopalElement<'a> {
    fn island_id(&self) -> u32 {
        match self {
            ScopalElement::Quantifier(q) => q.island_id,
            ScopalElement::Negation { island_id } => *island_id,
        }
    }
}

/// Generates all possible quantifier scopings for an expression.
///
/// Returns an iterator over all scope-permuted versions of the expression,
/// respecting scope island constraints. Quantifiers with the same `island_id`
/// can be permuted; quantifiers in different islands maintain relative order.
///
/// # Example
///
/// "Every man loves a woman" has two readings:
/// - Surface scope: ∀x(Man(x) → ∃y(Woman(y) ∧ Love(x,y)))
/// - Inverse scope: ∃y(Woman(y) ∧ ∀x(Man(x) → Love(x,y)))
pub fn enumerate_scopings<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    _term_arena: &'a Arena<Term<'a>>,
) -> ScopeIterator<'a> {
    let mut elements = Vec::new();
    let core = extract_scopal_elements(expr, &mut elements, interner, expr_arena);

    if elements.is_empty() || elements.len() == 1 {
        return ScopeIterator {
            expr_arena,
            islands: Vec::new(),
            core,
            current_index: 0,
            total: 0,
            single_result: Some(expr),
            returned_single: false,
        };
    }

    let islands = group_scopal_by_island(elements);
    let total: u64 = islands
        .iter()
        .map(|island| factorial(island.len()))
        .product();

    ScopeIterator {
        expr_arena,
        islands,
        core,
        current_index: 0,
        total,
        single_result: None,
        returned_single: false,
    }
}

fn group_scopal_by_island<'a>(elements: Vec<ScopalElement<'a>>) -> Vec<Vec<ScopalElement<'a>>> {
    use std::collections::BTreeMap;

    let mut by_island: BTreeMap<u32, Vec<ScopalElement<'a>>> = BTreeMap::new();
    for elem in elements {
        by_island.entry(elem.island_id()).or_default().push(elem);
    }

    by_island.into_values().collect()
}

/// Builds the CUMULATIVE reading (Scha 1981) for a two-cardinal transitive
/// sentence — the reading irreducible to either nesting:
///   "Three boys lifted five boxes." →
///     ∃=3 x(Boy(x) ∧ ∃y(Box(y) ∧ Lift(x,y))) ∧ ∃=5 y(Box(y) ∧ ∃x(Boy(x) ∧ Lift(x,y)))
/// i.e. exactly 3 boys each lifted some box AND exactly 5 boxes were each lifted
/// by some boy. This is first-order over the Link lattice (no ⊕ term needed). It
/// is produced only when there are exactly two cardinal quantifiers in the same
/// island over a shared nucleus; otherwise returns `None`.
pub fn cumulative_reading<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
) -> Option<&'a LogicExpr<'a>> {
    let mut elements = Vec::new();
    let core = extract_scopal_elements(expr, &mut elements, interner, expr_arena);
    if elements.len() != 2 {
        return None;
    }
    let (q1, q2) = match (&elements[0], &elements[1]) {
        (ScopalElement::Quantifier(a), ScopalElement::Quantifier(b)) => (a.clone(), b.clone()),
        _ => return None,
    };
    // Both must be cardinal counts in the same island (cumulative is a relation
    // between two counted groups, not a nesting).
    if !matches!(q1.kind, QuantifierKind::Cardinal(_))
        || !matches!(q2.kind, QuantifierKind::Cardinal(_))
        || q1.island_id != q2.island_id
    {
        return None;
    }

    let and = |l: &'a LogicExpr<'a>, r: &'a LogicExpr<'a>| -> &'a LogicExpr<'a> {
        expr_arena.alloc(LogicExpr::BinaryOp {
            left: l,
            op: TokenType::And,
            right: r,
        })
    };

    // Conjunct 1: q1 x ( R1(x) ∧ ∃y( R2(y) ∧ core ) )
    let exists_y = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: q2.variable,
        body: and(q2.restrictor, core),
        island_id: q1.island_id,
    });
    let c1 = expr_arena.alloc(LogicExpr::Quantifier {
        kind: q1.kind,
        variable: q1.variable,
        body: and(q1.restrictor, exists_y),
        island_id: q1.island_id,
    });

    // Conjunct 2: q2 y ( R2(y) ∧ ∃x( R1(x) ∧ core ) )
    let exists_x = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: q1.variable,
        body: and(q1.restrictor, core),
        island_id: q2.island_id,
    });
    let c2 = expr_arena.alloc(LogicExpr::Quantifier {
        kind: q2.kind,
        variable: q2.variable,
        body: and(q2.restrictor, exists_x),
        island_id: q2.island_id,
    });

    Some(and(c1, c2))
}

fn extract_scopal_elements<'a>(
    expr: &'a LogicExpr<'a>,
    elements: &mut Vec<ScopalElement<'a>>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Quantifier {
            kind,
            variable,
            body,
            island_id,
        } => {
            if let LogicExpr::BinaryOp { left, op, right } = body {
                if matches!(op, TokenType::If | TokenType::Implies | TokenType::And) {
                    // Check if right side has a negation at the top level
                    if let LogicExpr::UnaryOp {
                        op: TokenType::Not,
                        operand,
                    } = right
                    {
                        // Pattern: ∀x(R(x) → ¬P(x)) or ∃x(R(x) ∧ ¬P(x))
                        // Extract both quantifier and negation
                        elements.push(ScopalElement::Quantifier(QuantifierInfo {
                            kind: *kind,
                            variable: *variable,
                            restrictor: *left,
                            island_id: *island_id,
                        }));
                        elements.push(ScopalElement::Negation {
                            island_id: *island_id,
                        });
                        return extract_scopal_elements(operand, elements, interner, expr_arena);
                    }
                    // No negation in right side, just extract quantifier
                    elements.push(ScopalElement::Quantifier(QuantifierInfo {
                        kind: *kind,
                        variable: *variable,
                        restrictor: *left,
                        island_id: *island_id,
                    }));
                    return extract_scopal_elements(right, elements, interner, expr_arena);
                }
            }
            // No binary op body, use a true restrictor
            elements.push(ScopalElement::Quantifier(QuantifierInfo {
                kind: *kind,
                variable: *variable,
                restrictor: expr_arena.alloc(LogicExpr::Atom(interner.intern("T"))),
                island_id: *island_id,
            }));
            extract_scopal_elements(body, elements, interner, expr_arena)
        }
        LogicExpr::UnaryOp {
            op: TokenType::Not,
            operand,
        } => {
            // Standalone negation (not inside a quantifier body)
            elements.push(ScopalElement::Negation { island_id: 0 });
            extract_scopal_elements(operand, elements, interner, expr_arena)
        }
        _ => expr,
    }
}

fn rebuild_with_scopal_elements<'a>(
    elements: &[ScopalElement<'a>],
    core: &'a LogicExpr<'a>,
    arena: &'a Arena<LogicExpr<'a>>,
) -> &'a LogicExpr<'a> {
    let mut result = core;

    for elem in elements.iter().rev() {
        match elem {
            ScopalElement::Quantifier(q) => {
                let connective = match q.kind {
                    QuantifierKind::Universal => TokenType::Implies,
                    _ => TokenType::And,
                };

                let body = arena.alloc(LogicExpr::BinaryOp {
                    left: q.restrictor,
                    op: connective,
                    right: result,
                });

                result = arena.alloc(LogicExpr::Quantifier {
                    kind: q.kind,
                    variable: q.variable,
                    body,
                    island_id: q.island_id,
                });
            }
            ScopalElement::Negation { .. } => {
                result = arena.alloc(LogicExpr::UnaryOp {
                    op: TokenType::Not,
                    operand: result,
                });
            }
        }
    }

    result
}

pub fn lift_proper_name<'a>(
    name: Symbol,
    interner: &mut Interner,
    arena: &'a Arena<LogicExpr<'a>>,
) -> &'a LogicExpr<'a> {
    let p_sym = interner.intern("P");
    let inner_app = arena.alloc(LogicExpr::App {
        function: arena.alloc(LogicExpr::Atom(p_sym)),
        argument: arena.alloc(LogicExpr::Atom(name)),
    });
    arena.alloc(LogicExpr::Lambda {
        variable: p_sym,
        body: inner_app,
    })
}

pub fn lift_quantifier<'a>(
    kind: QuantifierKind,
    restrictor: Symbol,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    let x_sym = interner.intern("x");
    let q_sym = interner.intern("Q");

    let restrictor_pred = expr_arena.alloc(LogicExpr::Predicate {
        name: restrictor,
        args: term_arena.alloc_slice([Term::Variable(x_sym)]),
        world: None,
    });

    let q_of_x = expr_arena.alloc(LogicExpr::App {
        function: expr_arena.alloc(LogicExpr::Atom(q_sym)),
        argument: expr_arena.alloc(LogicExpr::Atom(x_sym)),
    });

    let connective = match kind {
        QuantifierKind::Universal => TokenType::Implies,
        _ => TokenType::And,
    };

    let body = expr_arena.alloc(LogicExpr::BinaryOp {
        left: restrictor_pred,
        op: connective,
        right: q_of_x,
    });

    let quantifier = expr_arena.alloc(LogicExpr::Quantifier {
        kind,
        variable: x_sym,
        body,
        island_id: 0,
    });

    expr_arena.alloc(LogicExpr::Lambda {
        variable: q_sym,
        body: quantifier,
    })
}

/// Performs beta reduction on lambda applications.
///
/// Reduces `(λx.body)(arg)` to `body[x := arg]` by substituting
/// the argument for the bound variable in the body.
pub fn beta_reduce<'a>(
    expr: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::App { function, argument } => {
            if let LogicExpr::Lambda { variable, body } = function {
                substitute(body, *variable, argument, expr_arena, term_arena)
            } else {
                expr_arena.alloc(LogicExpr::App {
                    function: beta_reduce(function, expr_arena, term_arena),
                    argument: beta_reduce(argument, expr_arena, term_arena),
                })
            }
        }
        LogicExpr::Lambda { variable, body } => expr_arena.alloc(LogicExpr::Lambda {
            variable: *variable,
            body: beta_reduce(body, expr_arena, term_arena),
        }),
        _ => expr,
    }
}

fn substitute<'a>(
    expr: &'a LogicExpr<'a>,
    var: Symbol,
    replacement: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Predicate { name, args, .. } => {
            let new_args: Vec<Term<'a>> = args
                .iter()
                .map(|arg| substitute_term(arg, var, replacement, term_arena))
                .collect();
            expr_arena.alloc(LogicExpr::Predicate {
                name: *name,
                args: term_arena.alloc_slice(new_args),
                world: None,
            })
        }

        LogicExpr::Lambda { variable, body } => {
            if *variable == var {
                expr
            } else {
                expr_arena.alloc(LogicExpr::Lambda {
                    variable: *variable,
                    body: substitute(body, var, replacement, expr_arena, term_arena),
                })
            }
        }

        LogicExpr::App { function, argument } => expr_arena.alloc(LogicExpr::App {
            function: substitute(function, var, replacement, expr_arena, term_arena),
            argument: substitute(argument, var, replacement, expr_arena, term_arena),
        }),

        LogicExpr::BinaryOp { left, op, right } => expr_arena.alloc(LogicExpr::BinaryOp {
            left: substitute(left, var, replacement, expr_arena, term_arena),
            op: op.clone(),
            right: substitute(right, var, replacement, expr_arena, term_arena),
        }),

        LogicExpr::UnaryOp { op, operand } => expr_arena.alloc(LogicExpr::UnaryOp {
            op: op.clone(),
            operand: substitute(operand, var, replacement, expr_arena, term_arena),
        }),

        LogicExpr::Quantifier {
            kind,
            variable,
            body,
            island_id,
        } => {
            if *variable == var {
                expr
            } else {
                expr_arena.alloc(LogicExpr::Quantifier {
                    kind: *kind,
                    variable: *variable,
                    body: substitute(body, var, replacement, expr_arena, term_arena),
                    island_id: *island_id,
                })
            }
        }

        LogicExpr::Atom(s) => {
            if *s == var {
                replacement
            } else {
                expr
            }
        }

        _ => expr,
    }
}

fn substitute_term<'a>(
    term: &Term<'a>,
    var: Symbol,
    replacement: &LogicExpr<'a>,
    term_arena: &'a Arena<Term<'a>>,
) -> Term<'a> {
    match term {
        Term::Variable(v) if *v == var => match replacement {
            LogicExpr::Atom(s) => Term::Constant(*s),
            LogicExpr::Predicate { name, .. } => Term::Constant(*name),
            _ => clone_term(term, term_arena),
        },
        _ => clone_term(term, term_arena),
    }
}

// ═══════════════════════════════════════════════════════════════════
// Intensional Reading Generation (De Re / De Dicto)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
struct IntensionalContext {
    quantifier_var: Symbol,
    restrictor: Symbol,
}

fn find_opaque_verb_context<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &Interner,
) -> Option<IntensionalContext> {
    match expr {
        LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable,
            body,
            ..
        } => {
            if let LogicExpr::BinaryOp {
                left,
                op: TokenType::And,
                right,
            } = body
            {
                if let LogicExpr::Predicate {
                    name: restrictor,
                    args,
                    ..
                } = left
                {
                    if args.len() == 1 {
                        if let Term::Variable(v) = &args[0] {
                            if *v == *variable {
                                if find_opaque_verb_in_scope(right, *variable, interner).is_some() {
                                    return Some(IntensionalContext {
                                        quantifier_var: *variable,
                                        restrictor: *restrictor,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn find_opaque_verb_in_scope<'a>(
    expr: &'a LogicExpr<'a>,
    theme_var: Symbol,
    interner: &Interner,
) -> Option<Symbol> {
    match expr {
        LogicExpr::Quantifier { body, .. } => find_opaque_verb_in_scope(body, theme_var, interner),
        LogicExpr::BinaryOp { left, right, .. } => {
            find_opaque_verb_in_scope(left, theme_var, interner)
                .or_else(|| find_opaque_verb_in_scope(right, theme_var, interner))
        }
        LogicExpr::NeoEvent(data) => {
            if is_opaque_verb(data.verb, interner) {
                for (role, term) in data.roles.iter() {
                    if matches!(role, crate::ast::ThematicRole::Theme) {
                        if let Term::Variable(v) = term {
                            if *v == theme_var {
                                return Some(data.verb);
                            }
                        }
                    }
                }
            }
            None
        }
        LogicExpr::Predicate { name, args, .. } => {
            if is_opaque_verb(*name, interner) && args.len() >= 2 {
                if let Term::Variable(v) = &args[1] {
                    if *v == theme_var {
                        return Some(*name);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn build_de_dicto_reading<'a>(
    expr: &'a LogicExpr<'a>,
    ctx: &IntensionalContext,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    role_arena: &'a Arena<(crate::ast::ThematicRole, Term<'a>)>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable,
            body,
            ..
        } if *variable == ctx.quantifier_var => {
            if let LogicExpr::BinaryOp { right, .. } = body {
                replace_theme_with_intension(right, ctx, expr_arena, term_arena, role_arena)
            } else {
                expr
            }
        }
        _ => expr,
    }
}

fn replace_theme_with_intension<'a>(
    expr: &'a LogicExpr<'a>,
    ctx: &IntensionalContext,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    role_arena: &'a Arena<(crate::ast::ThematicRole, Term<'a>)>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Quantifier {
            kind,
            variable,
            body,
            island_id,
        } => {
            let new_body =
                replace_theme_with_intension(body, ctx, expr_arena, term_arena, role_arena);
            expr_arena.alloc(LogicExpr::Quantifier {
                kind: *kind,
                variable: *variable,
                body: new_body,
                island_id: *island_id,
            })
        }
        LogicExpr::BinaryOp { left, op, right } => {
            let new_left =
                replace_theme_with_intension(left, ctx, expr_arena, term_arena, role_arena);
            let new_right =
                replace_theme_with_intension(right, ctx, expr_arena, term_arena, role_arena);
            expr_arena.alloc(LogicExpr::BinaryOp {
                left: new_left,
                op: op.clone(),
                right: new_right,
            })
        }
        LogicExpr::NeoEvent(data) => {
            let new_roles: Vec<_> = data
                .roles
                .iter()
                .map(|(role, term)| {
                    if matches!(role, crate::ast::ThematicRole::Theme) {
                        if let Term::Variable(v) = term {
                            if *v == ctx.quantifier_var {
                                return (*role, Term::Intension(ctx.restrictor));
                            }
                        }
                    }
                    (*role, clone_term(term, term_arena))
                })
                .collect();

            expr_arena.alloc(LogicExpr::NeoEvent(Box::new(crate::ast::NeoEventData {
                event_var: data.event_var,
                verb: data.verb,
                roles: role_arena.alloc_slice(new_roles),
                modifiers: data.modifiers,
                suppress_existential: false,
                world: None,
            })))
        }
        LogicExpr::Predicate { name, args, .. } => {
            let new_args: Vec<_> = args
                .iter()
                .map(|arg| {
                    if let Term::Variable(v) = arg {
                        if *v == ctx.quantifier_var {
                            return Term::Intension(ctx.restrictor);
                        }
                    }
                    clone_term(arg, term_arena)
                })
                .collect();

            expr_arena.alloc(LogicExpr::Predicate {
                name: *name,
                args: term_arena.alloc_slice(new_args),
                world: None,
            })
        }
        _ => expr,
    }
}

pub fn enumerate_intensional_readings<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    role_arena: &'a Arena<(crate::ast::ThematicRole, Term<'a>)>,
) -> Vec<&'a LogicExpr<'a>> {
    // Check if expression already has intensional terms (de dicto from parser)
    if let Some(de_re) =
        build_de_re_from_de_dicto(expr, interner, expr_arena, term_arena, role_arena)
    {
        // Return both: de re first (existential), de dicto second (intension)
        return vec![de_re, expr];
    }

    // Clausal attitude complement with an embedded existential
    // ("believes ⟨∃x(Spy(x) ∧ Exist(x))⟩"): the de re reading raises the
    // existential over the attitude — ∃x(Spy(x) ∧ Believe(s, ⟨Exist(x)⟩)).
    if let Some(de_re) = raise_existential_from_proposition(expr, expr_arena, term_arena) {
        return vec![expr, de_re];
    }

    // Original logic: check for de re that can be converted to de dicto
    if let Some(ctx) = find_opaque_verb_context(expr, interner) {
        let de_dicto = build_de_dicto_reading(expr, &ctx, expr_arena, term_arena, role_arena);
        vec![expr, de_dicto]
    } else {
        vec![expr]
    }
}

/// De re quantifier raising out of a clausal attitude complement:
/// `V(s, ⟨∃x(R(x) ∧ P(x))⟩)` → `∃x(R(x) ∧ V(s, ⟨P(x)⟩))`. A `Temporal`
/// wrapper on the attitude stays on the attitude.
fn raise_existential_from_proposition<'a>(
    expr: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> Option<&'a LogicExpr<'a>> {
    use crate::token::TokenType;

    let (temporal_op, attitude) = match expr {
        LogicExpr::Temporal { operator, body } => (Some(*operator), *body),
        other => (None, other),
    };

    let LogicExpr::Predicate { name, args, world } = attitude else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    let Term::Proposition(LogicExpr::Quantifier {
        kind: crate::ast::QuantifierKind::Existential,
        variable,
        body,
        island_id,
    }) = &args[1]
    else {
        return None;
    };
    let LogicExpr::BinaryOp {
        left: restriction,
        op: TokenType::And,
        right: inner,
    } = body
    else {
        return None;
    };

    let raised_attitude = expr_arena.alloc(LogicExpr::Predicate {
        name: *name,
        args: term_arena.alloc_slice([args[0].clone(), Term::Proposition(inner)]),
        world: world.clone(),
    });
    let raised_attitude = match temporal_op {
        Some(operator) => expr_arena.alloc(LogicExpr::Temporal {
            operator,
            body: raised_attitude,
        }),
        None => raised_attitude,
    };
    let raised_body = expr_arena.alloc(LogicExpr::BinaryOp {
        left: restriction,
        op: TokenType::And,
        right: raised_attitude,
    });
    Some(expr_arena.alloc(LogicExpr::Quantifier {
        kind: crate::ast::QuantifierKind::Existential,
        variable: *variable,
        body: raised_body,
        island_id: *island_id,
    }))
}

fn build_de_re_from_de_dicto<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
    role_arena: &'a Arena<(crate::ast::ThematicRole, Term<'a>)>,
) -> Option<&'a LogicExpr<'a>> {
    // Find Term::Intension in NeoEvent themes and expand to existential
    match expr {
        LogicExpr::NeoEvent(data) => {
            // Check if any role has an Intension term
            for (role, term) in data.roles.iter() {
                if matches!(role, crate::ast::ThematicRole::Theme) {
                    if let Term::Intension(noun) = term {
                        // Build de re: ∃x(Noun(x) ∧ Event[Theme=x])
                        let var = interner.intern("x");

                        // Build noun predicate: Noun(x)
                        let noun_pred = expr_arena.alloc(LogicExpr::Predicate {
                            name: *noun,
                            args: term_arena.alloc_slice([Term::Variable(var)]),
                            world: None,
                        });

                        // Build new roles with variable instead of intension
                        let new_roles: Vec<_> = data
                            .roles
                            .iter()
                            .map(|(r, t)| {
                                if matches!(r, crate::ast::ThematicRole::Theme) {
                                    (*r, Term::Variable(var))
                                } else {
                                    (*r, t.clone())
                                }
                            })
                            .collect();

                        let new_event = expr_arena.alloc(LogicExpr::NeoEvent(Box::new(
                            crate::ast::NeoEventData {
                                event_var: data.event_var,
                                verb: data.verb,
                                roles: role_arena.alloc_slice(new_roles),
                                modifiers: data.modifiers,
                                suppress_existential: false,
                                world: None,
                            },
                        )));

                        // Build: ∃x(Noun(x) ∧ Event)
                        let body = expr_arena.alloc(LogicExpr::BinaryOp {
                            left: noun_pred,
                            op: crate::token::TokenType::And,
                            right: new_event,
                        });

                        return Some(expr_arena.alloc(LogicExpr::Quantifier {
                            kind: crate::ast::QuantifierKind::Existential,
                            variable: var,
                            body,
                            island_id: 0,
                        }));
                    }
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/lambda/tests.rs"]
mod tests;
