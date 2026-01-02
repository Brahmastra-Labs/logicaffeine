use crate::arena::Arena;
use crate::ast::{LogicExpr, QuantifierKind, Term};
use crate::intern::{Interner, Symbol};
use crate::lexicon;
use crate::token::TokenType;

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
        Term::Possessed { possessor, possessed } => Term::Possessed {
            possessor: arena.alloc(clone_term(possessor, arena)),
            possessed: *possessed,
        },
        Term::Sigma(predicate) => Term::Sigma(*predicate),
        Term::Intension(predicate) => Term::Intension(*predicate),
        Term::Proposition(expr) => Term::Proposition(*expr),
        Term::Value { kind, unit, dimension } => Term::Value {
            kind: *kind,
            unit: *unit,
            dimension: *dimension,
        },
    }
}

pub fn is_opaque_verb(verb: Symbol, interner: &Interner) -> bool {
    let verb_str = interner.resolve(verb);
    let lower = verb_str.to_lowercase();
    lexicon::is_opaque_verb(&lower)
}

pub fn make_intensional<'a>(
    operator: Symbol,
    content: &'a LogicExpr<'a>,
    arena: &'a Arena<LogicExpr<'a>>,
) -> &'a LogicExpr<'a> {
    arena.alloc(LogicExpr::Intensional { operator, content })
}

pub fn substitute_respecting_opacity<'a>(
    expr: &'a LogicExpr<'a>,
    var: Symbol,
    replacement: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Intensional { operator, content } => {
            expr_arena.alloc(LogicExpr::Intensional {
                operator: *operator,
                content: *content,
            })
        }

        LogicExpr::Predicate { name, args } => {
            let new_args: Vec<Term<'a>> = args
                .iter()
                .map(|arg| substitute_term_for_opacity(arg, var, replacement, term_arena))
                .collect();
            expr_arena.alloc(LogicExpr::Predicate {
                name: *name,
                args: term_arena.alloc_slice(new_args),
            })
        }

        LogicExpr::BinaryOp { left, op, right } => expr_arena.alloc(LogicExpr::BinaryOp {
            left: substitute_respecting_opacity(left, var, replacement, expr_arena, term_arena),
            op: op.clone(),
            right: substitute_respecting_opacity(right, var, replacement, expr_arena, term_arena),
        }),

        LogicExpr::UnaryOp { op, operand } => expr_arena.alloc(LogicExpr::UnaryOp {
            op: op.clone(),
            operand: substitute_respecting_opacity(operand, var, replacement, expr_arena, term_arena),
        }),

        LogicExpr::Quantifier { kind, variable, body, island_id } => {
            if *variable == var {
                expr
            } else {
                expr_arena.alloc(LogicExpr::Quantifier {
                    kind: *kind,
                    variable: *variable,
                    body: substitute_respecting_opacity(body, var, replacement, expr_arena, term_arena),
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
                    body: substitute_respecting_opacity(body, var, replacement, expr_arena, term_arena),
                })
            }
        }

        LogicExpr::App { function, argument } => expr_arena.alloc(LogicExpr::App {
            function: substitute_respecting_opacity(function, var, replacement, expr_arena, term_arena),
            argument: substitute_respecting_opacity(argument, var, replacement, expr_arena, term_arena),
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
        Term::Constant(c) if *c == var => {
            match replacement {
                LogicExpr::Atom(s) => Term::Constant(*s),
                _ => clone_term(term, arena),
            }
        }
        Term::Variable(v) if *v == var => {
            match replacement {
                LogicExpr::Atom(s) => Term::Constant(*s),
                _ => clone_term(term, arena),
            }
        }
        _ => clone_term(term, arena),
    }
}

pub fn to_event_semantics<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Predicate { name, args } => {
            let e_sym = interner.intern("e");
            let _event_var = term_arena.alloc(Term::Variable(e_sym));

            let event_pred = expr_arena.alloc(LogicExpr::Predicate {
                name: *name,
                args: term_arena.alloc_slice([Term::Variable(e_sym)]),
            });

            let mut body = event_pred;

            if !args.is_empty() {
                let agent_args = term_arena.alloc_slice([Term::Variable(e_sym), clone_term(&args[0], term_arena)]);
                let agent_pred = expr_arena.alloc(LogicExpr::Predicate {
                    name: interner.intern("Agent"),
                    args: agent_args,
                });
                body = expr_arena.alloc(LogicExpr::BinaryOp {
                    left: body,
                    op: TokenType::And,
                    right: agent_pred,
                });
            }

            if args.len() > 1 {
                let theme_args = term_arena.alloc_slice([Term::Variable(e_sym), clone_term(&args[1], term_arena)]);
                let theme_pred = expr_arena.alloc(LogicExpr::Predicate {
                    name: interner.intern("Theme"),
                    args: theme_args,
                });
                body = expr_arena.alloc(LogicExpr::BinaryOp {
                    left: body,
                    op: TokenType::And,
                    right: theme_pred,
                });
            }

            if args.len() > 2 {
                let goal_args = term_arena.alloc_slice([Term::Variable(e_sym), clone_term(&args[2], term_arena)]);
                let goal_pred = expr_arena.alloc(LogicExpr::Predicate {
                    name: interner.intern("Goal"),
                    args: goal_args,
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

pub fn apply_adverb<'a>(
    expr: &'a LogicExpr<'a>,
    adverb: Symbol,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    term_arena: &'a Arena<Term<'a>>,
) -> &'a LogicExpr<'a> {
    let e_sym = interner.intern("e");
    match expr {
        LogicExpr::Quantifier { kind, variable, body, island_id } if *variable == e_sym => {
            let adverb_str = interner.resolve(adverb);
            let capitalized = capitalize(adverb_str);
            let adverb_pred = expr_arena.alloc(LogicExpr::Predicate {
                name: interner.intern(&capitalized),
                args: term_arena.alloc_slice([Term::Variable(*variable)]),
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
        Some(rebuild_with_scopal_elements(&ordered, self.core, self.expr_arena))
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
    let total: u64 = islands.iter().map(|island| factorial(island.len())).product();

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

fn group_by_island<'a>(quantifiers: Vec<QuantifierInfo<'a>>) -> Vec<Vec<QuantifierInfo<'a>>> {
    use std::collections::BTreeMap;

    let mut by_island: BTreeMap<u32, Vec<QuantifierInfo<'a>>> = BTreeMap::new();
    for q in quantifiers {
        by_island.entry(q.island_id).or_default().push(q);
    }

    by_island.into_values().collect()
}

fn group_scopal_by_island<'a>(elements: Vec<ScopalElement<'a>>) -> Vec<Vec<ScopalElement<'a>>> {
    use std::collections::BTreeMap;

    let mut by_island: BTreeMap<u32, Vec<ScopalElement<'a>>> = BTreeMap::new();
    for elem in elements {
        by_island.entry(elem.island_id()).or_default().push(elem);
    }

    by_island.into_values().collect()
}

fn extract_scopal_elements<'a>(
    expr: &'a LogicExpr<'a>,
    elements: &mut Vec<ScopalElement<'a>>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Quantifier { kind, variable, body, island_id } => {
            if let LogicExpr::BinaryOp { left, op, right } = body {
                if matches!(op, TokenType::If | TokenType::And) {
                    // Check if right side has a negation at the top level
                    if let LogicExpr::UnaryOp { op: TokenType::Not, operand } = right {
                        // Pattern: ∀x(R(x) → ¬P(x)) or ∃x(R(x) ∧ ¬P(x))
                        // Extract both quantifier and negation
                        elements.push(ScopalElement::Quantifier(QuantifierInfo {
                            kind: *kind,
                            variable: *variable,
                            restrictor: *left,
                            island_id: *island_id,
                        }));
                        elements.push(ScopalElement::Negation { island_id: *island_id });
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
        LogicExpr::UnaryOp { op: TokenType::Not, operand } => {
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
                    QuantifierKind::Universal => TokenType::If,
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

fn extract_quantifiers<'a>(
    expr: &'a LogicExpr<'a>,
    quantifiers: &mut Vec<QuantifierInfo<'a>>,
    interner: &mut Interner,
    expr_arena: &'a Arena<LogicExpr<'a>>,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Quantifier { kind, variable, body, island_id } => {
            if let LogicExpr::BinaryOp { left, op, right } = body {
                if matches!(op, TokenType::If | TokenType::And) {
                    quantifiers.push(QuantifierInfo {
                        kind: *kind,
                        variable: *variable,
                        restrictor: *left,
                        island_id: *island_id,
                    });
                    return extract_quantifiers(right, quantifiers, interner, expr_arena);
                }
            }
            quantifiers.push(QuantifierInfo {
                kind: *kind,
                variable: *variable,
                restrictor: expr_arena.alloc(LogicExpr::Atom(interner.intern("T"))),
                island_id: *island_id,
            });
            extract_quantifiers(body, quantifiers, interner, expr_arena)
        }
        _ => expr,
    }
}

fn rebuild_with_scope_order<'a>(
    quantifiers: &[QuantifierInfo<'a>],
    core: &'a LogicExpr<'a>,
    arena: &'a Arena<LogicExpr<'a>>,
) -> &'a LogicExpr<'a> {
    let mut result = core;

    for q in quantifiers.iter().rev() {
        let connective = match q.kind {
            QuantifierKind::Universal => TokenType::If,
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
    });

    let q_of_x = expr_arena.alloc(LogicExpr::App {
        function: expr_arena.alloc(LogicExpr::Atom(q_sym)),
        argument: expr_arena.alloc(LogicExpr::Atom(x_sym)),
    });

    let connective = match kind {
        QuantifierKind::Universal => TokenType::If,
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
        LogicExpr::Predicate { name, args } => {
            let new_args: Vec<Term<'a>> = args
                .iter()
                .map(|arg| substitute_term(arg, var, replacement, term_arena))
                .collect();
            expr_arena.alloc(LogicExpr::Predicate {
                name: *name,
                args: term_arena.alloc_slice(new_args),
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

        LogicExpr::Quantifier { kind, variable, body, island_id } => {
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
        Term::Variable(v) if *v == var => {
            match replacement {
                LogicExpr::Atom(s) => Term::Constant(*s),
                LogicExpr::Predicate { name, .. } => Term::Constant(*name),
                _ => clone_term(term, term_arena),
            }
        }
        _ => clone_term(term, term_arena),
    }
}

// ═══════════════════════════════════════════════════════════════════
// Intensional Reading Generation (De Re / De Dicto)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
struct IntensionalContext {
    verb: Symbol,
    quantifier_var: Symbol,
    restrictor: Symbol,
}

fn find_opaque_verb_context<'a>(
    expr: &'a LogicExpr<'a>,
    interner: &Interner,
) -> Option<IntensionalContext> {
    match expr {
        LogicExpr::Quantifier { kind: QuantifierKind::Existential, variable, body, .. } => {
            if let LogicExpr::BinaryOp { left, op: TokenType::And, right } = body {
                if let LogicExpr::Predicate { name: restrictor, args } = left {
                    if args.len() == 1 {
                        if let Term::Variable(v) = &args[0] {
                            if *v == *variable {
                                if let Some(verb) = find_opaque_verb_in_scope(right, *variable, interner) {
                                    return Some(IntensionalContext {
                                        verb,
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
        LogicExpr::Predicate { name, args } => {
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
        LogicExpr::Quantifier { kind: QuantifierKind::Existential, variable, body, .. }
            if *variable == ctx.quantifier_var =>
        {
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
        LogicExpr::Quantifier { kind, variable, body, island_id } => {
            let new_body = replace_theme_with_intension(body, ctx, expr_arena, term_arena, role_arena);
            expr_arena.alloc(LogicExpr::Quantifier {
                kind: *kind,
                variable: *variable,
                body: new_body,
                island_id: *island_id,
            })
        }
        LogicExpr::BinaryOp { left, op, right } => {
            let new_left = replace_theme_with_intension(left, ctx, expr_arena, term_arena, role_arena);
            let new_right = replace_theme_with_intension(right, ctx, expr_arena, term_arena, role_arena);
            expr_arena.alloc(LogicExpr::BinaryOp {
                left: new_left,
                op: op.clone(),
                right: new_right,
            })
        }
        LogicExpr::NeoEvent(data) => {
            let new_roles: Vec<_> = data.roles.iter().map(|(role, term)| {
                if matches!(role, crate::ast::ThematicRole::Theme) {
                    if let Term::Variable(v) = term {
                        if *v == ctx.quantifier_var {
                            return (*role, Term::Intension(ctx.restrictor));
                        }
                    }
                }
                (*role, clone_term(term, term_arena))
            }).collect();

            expr_arena.alloc(LogicExpr::NeoEvent(Box::new(crate::ast::NeoEventData {
                event_var: data.event_var,
                verb: data.verb,
                roles: role_arena.alloc_slice(new_roles),
                modifiers: data.modifiers,
                suppress_existential: false,
            })))
        }
        LogicExpr::Predicate { name, args } => {
            let new_args: Vec<_> = args.iter().map(|arg| {
                if let Term::Variable(v) = arg {
                    if *v == ctx.quantifier_var {
                        return Term::Intension(ctx.restrictor);
                    }
                }
                clone_term(arg, term_arena)
            }).collect();

            expr_arena.alloc(LogicExpr::Predicate {
                name: *name,
                args: term_arena.alloc_slice(new_args),
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
    if let Some(de_re) = build_de_re_from_de_dicto(expr, interner, expr_arena, term_arena, role_arena) {
        // Return both: de re first (existential), de dicto second (intension)
        return vec![de_re, expr];
    }

    // Original logic: check for de re that can be converted to de dicto
    if let Some(ctx) = find_opaque_verb_context(expr, interner) {
        let de_dicto = build_de_dicto_reading(expr, &ctx, expr_arena, term_arena, role_arena);
        vec![expr, de_dicto]
    } else {
        vec![expr]
    }
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
                        });

                        // Build new roles with variable instead of intension
                        let new_roles: Vec<_> = data.roles.iter().map(|(r, t)| {
                            if matches!(r, crate::ast::ThematicRole::Theme) {
                                (*r, Term::Variable(var))
                            } else {
                                (*r, t.clone())
                            }
                        }).collect();

                        let new_event = expr_arena.alloc(LogicExpr::NeoEvent(Box::new(crate::ast::NeoEventData {
                            event_var: data.event_var,
                            verb: data.verb,
                            roles: role_arena.alloc_slice(new_roles),
                            modifiers: data.modifiers,
                            suppress_existential: false,
                        })));

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
mod tests {
    use super::*;
    use crate::ast::{LogicExpr, Term};
    use crate::intern::Interner;
    use crate::registry::SymbolRegistry;
    use crate::OutputFormat;

    #[test]
    fn test_lambda_formatting_unicode() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let sleep = interner.intern("Sleep");

        let body = expr_arena.alloc(LogicExpr::Predicate {
            name: sleep,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });

        let mut registry = SymbolRegistry::new();
        let output = lambda.transpile(&mut registry, &interner, OutputFormat::Unicode);
        assert!(output.contains("λx"), "Unicode should use λ: {}", output);
    }

    #[test]
    fn test_lambda_formatting_latex() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let sleep = interner.intern("Sleep");

        let body = expr_arena.alloc(LogicExpr::Predicate {
            name: sleep,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });

        let mut registry = SymbolRegistry::new();
        let output = lambda.transpile(&mut registry, &interner, OutputFormat::LaTeX);
        assert!(output.contains("\\lambda"), "LaTeX should use \\lambda: {}", output);
    }

    #[test]
    fn test_application_formatting() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();

        let p = interner.intern("P");
        let j = interner.intern("j");

        let func = expr_arena.alloc(LogicExpr::Atom(p));
        let arg = expr_arena.alloc(LogicExpr::Atom(j));
        let app = expr_arena.alloc(LogicExpr::App { function: func, argument: arg });

        let mut registry = SymbolRegistry::new();
        let output = app.transpile(&mut registry, &interner, OutputFormat::Unicode);
        assert!(output.contains("(") && output.contains(")"), "App should have parens: {}", output);
    }

    #[test]
    fn test_nested_lambda() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");

        let inner_body = expr_arena.alloc(LogicExpr::Atom(x));
        let inner_lambda = expr_arena.alloc(LogicExpr::Lambda { variable: y, body: inner_body });
        let outer_lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body: inner_lambda });

        let mut registry = SymbolRegistry::new();
        let output = outer_lambda.transpile(&mut registry, &interner, OutputFormat::Unicode);
        assert!(output.contains("λx") && output.contains("λy"), "Nested lambdas: {}", output);
    }

    #[test]
    fn test_lambda_app_helper_functions() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let _term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let p = interner.intern("P");

        let body = expr_arena.alloc(LogicExpr::Atom(x));
        let lambda = LogicExpr::lambda(x, body, &expr_arena);

        let arg = expr_arena.alloc(LogicExpr::Atom(p));
        let app = LogicExpr::app(lambda, arg, &expr_arena);

        assert!(matches!(app, LogicExpr::App { .. }));
    }

    #[test]
    fn lift_proper_name_returns_lambda() {
        let mut interner = Interner::new();
        let arena: Arena<LogicExpr> = Arena::new();

        let john = interner.intern("John");
        let lifted = lift_proper_name(john, &mut interner, &arena);

        assert!(matches!(lifted, LogicExpr::Lambda { .. }), "Should return Lambda");
    }

    #[test]
    fn lift_proper_name_applies_predicate() {
        let mut interner = Interner::new();
        let arena: Arena<LogicExpr> = Arena::new();

        let john = interner.intern("John");
        let lifted = lift_proper_name(john, &mut interner, &arena);

        if let LogicExpr::Lambda { body, .. } = lifted {
            assert!(matches!(body, LogicExpr::App { .. }), "Body should be App");
        } else {
            panic!("Expected Lambda");
        }
    }

    #[test]
    fn lift_quantifier_universal_returns_lambda() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let woman = interner.intern("woman");
        let lifted = lift_quantifier(QuantifierKind::Universal, woman, &mut interner, &expr_arena, &term_arena);

        assert!(matches!(lifted, LogicExpr::Lambda { .. }), "Should return Lambda");
    }

    #[test]
    fn lift_quantifier_universal_structure() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let woman = interner.intern("woman");
        let lifted = lift_quantifier(QuantifierKind::Universal, woman, &mut interner, &expr_arena, &term_arena);

        if let LogicExpr::Lambda { body, .. } = lifted {
            assert!(
                matches!(body, LogicExpr::Quantifier { kind: QuantifierKind::Universal, .. }),
                "Body should contain ∀, got {:?}",
                body
            );
        } else {
            panic!("Expected Lambda, got {:?}", lifted);
        }
    }

    #[test]
    fn lift_quantifier_existential_returns_lambda() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let man = interner.intern("man");
        let lifted = lift_quantifier(QuantifierKind::Existential, man, &mut interner, &expr_arena, &term_arena);

        assert!(matches!(lifted, LogicExpr::Lambda { .. }), "Should return Lambda");
    }

    #[test]
    fn lift_quantifier_existential_structure() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let man = interner.intern("man");
        let lifted = lift_quantifier(QuantifierKind::Existential, man, &mut interner, &expr_arena, &term_arena);

        if let LogicExpr::Lambda { body, .. } = lifted {
            assert!(
                matches!(body, LogicExpr::Quantifier { kind: QuantifierKind::Existential, .. }),
                "Body should contain ∃, got {:?}",
                body
            );
        } else {
            panic!("Expected Lambda, got {:?}", lifted);
        }
    }

    #[test]
    fn beta_reduce_simple_predicate() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let john = interner.intern("John");
        let run = interner.intern("Run");

        let body = expr_arena.alloc(LogicExpr::Predicate {
            name: run,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });
        let arg = expr_arena.alloc(LogicExpr::Atom(john));
        let app = expr_arena.alloc(LogicExpr::App { function: lambda, argument: arg });

        let reduced = beta_reduce(app, &expr_arena, &term_arena);

        let mut registry = SymbolRegistry::new();
        let output = reduced.transpile(&mut registry, &interner, OutputFormat::Unicode);
        assert!(output.contains("R(J)") || output.contains("Run(John)"), "Should substitute: {}", output);
    }

    #[test]
    fn beta_reduce_with_constant() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let c = interner.intern("c");

        let body = expr_arena.alloc(LogicExpr::Atom(c));
        let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });
        let arg = expr_arena.alloc(LogicExpr::Atom(interner.intern("anything")));
        let app = expr_arena.alloc(LogicExpr::App { function: lambda, argument: arg });

        let reduced = beta_reduce(app, &expr_arena, &term_arena);
        assert!(matches!(reduced, LogicExpr::Atom(s) if *s == c), "Constant should remain");
    }

    #[test]
    fn beta_reduce_nested_lambda() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");

        let inner_body = expr_arena.alloc(LogicExpr::Atom(x));
        let inner_lambda = expr_arena.alloc(LogicExpr::Lambda { variable: y, body: inner_body });
        let outer_lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body: inner_lambda });

        let reduced = beta_reduce(outer_lambda, &expr_arena, &term_arena);
        assert!(matches!(reduced, LogicExpr::Lambda { .. }), "Should still be lambda");
    }

    #[test]
    fn beta_reduce_non_application_unchanged() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let p = interner.intern("P");
        let atom = expr_arena.alloc(LogicExpr::Atom(p));

        let reduced = beta_reduce(atom, &expr_arena, &term_arena);
        assert!(matches!(reduced, LogicExpr::Atom(s) if *s == p), "Atom unchanged");
    }

    #[test]
    fn beta_reduce_preserves_unbound_variables() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");
        let john = interner.intern("John");
        let loves = interner.intern("Loves");

        let body = expr_arena.alloc(LogicExpr::Predicate {
            name: loves,
            args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        });
        let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });
        let arg = expr_arena.alloc(LogicExpr::Atom(john));
        let app = expr_arena.alloc(LogicExpr::App { function: lambda, argument: arg });

        let reduced = beta_reduce(app, &expr_arena, &term_arena);

        let mut registry = SymbolRegistry::new();
        let output = reduced.transpile(&mut registry, &interner, OutputFormat::Unicode);
        assert!(output.contains("y"), "y should remain unbound: {}", output);
    }

    #[test]
    fn enumerate_scopings_single_quantifier() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let dog = interner.intern("Dog");
        let bark = interner.intern("Bark");

        let left = expr_arena.alloc(LogicExpr::Predicate {
            name: dog,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let right = expr_arena.alloc(LogicExpr::Predicate {
            name: bark,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let body = expr_arena.alloc(LogicExpr::BinaryOp {
            left,
            op: TokenType::If,
            right,
        });
        let expr = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body,
            island_id: 0,
        });

        let scopings = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
        assert_eq!(scopings.len(), 1, "Single quantifier should have 1 reading");
    }

    #[test]
    fn enumerate_scopings_no_quantifier() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let run = interner.intern("Run");
        let john = interner.intern("John");

        let expr = expr_arena.alloc(LogicExpr::Predicate {
            name: run,
            args: term_arena.alloc_slice([Term::Constant(john)]),
        });

        let scopings = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
        assert_eq!(scopings.len(), 1, "No quantifiers should have 1 reading");
    }

    #[test]
    fn is_opaque_verb_believes() {
        let mut interner = Interner::new();
        let believes = interner.intern("believes");
        let believes_cap = interner.intern("Believes");
        assert!(is_opaque_verb(believes, &interner), "believes should be opaque");
        assert!(is_opaque_verb(believes_cap, &interner), "Believes should be opaque");
    }

    #[test]
    fn is_opaque_verb_seeks() {
        let mut interner = Interner::new();
        let seeks = interner.intern("seeks");
        let wants = interner.intern("wants");
        assert!(is_opaque_verb(seeks, &interner), "seeks should be opaque");
        assert!(is_opaque_verb(wants, &interner), "wants should be opaque");
    }

    #[test]
    fn is_opaque_verb_normal_verbs() {
        let mut interner = Interner::new();
        let runs = interner.intern("runs");
        let loves = interner.intern("loves");
        assert!(!is_opaque_verb(runs, &interner), "runs should NOT be opaque");
        assert!(!is_opaque_verb(loves, &interner), "loves should NOT be opaque");
    }

    #[test]
    fn make_intensional_creates_wrapper() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let weak = interner.intern("Weak");
        let clark = interner.intern("Clark");
        let believes = interner.intern("believes");

        let content = expr_arena.alloc(LogicExpr::Predicate {
            name: weak,
            args: term_arena.alloc_slice([Term::Constant(clark)]),
        });

        let intensional = make_intensional(believes, content, &expr_arena);

        assert!(
            matches!(intensional, LogicExpr::Intensional { operator, .. } if *operator == believes),
            "Should create Intensional wrapper, got {:?}",
            intensional
        );
    }

    #[test]
    fn intensional_transpiles_with_brackets() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let weak = interner.intern("Weak");
        let clark = interner.intern("Clark");
        let believes = interner.intern("Believes");

        let content = expr_arena.alloc(LogicExpr::Predicate {
            name: weak,
            args: term_arena.alloc_slice([Term::Constant(clark)]),
        });

        let intensional = expr_arena.alloc(LogicExpr::Intensional {
            operator: believes,
            content,
        });

        let mut registry = SymbolRegistry::new();
        let output = intensional.transpile(&mut registry, &interner, OutputFormat::Unicode);

        assert!(
            output.contains("[") && output.contains("]"),
            "Intensional should use brackets: got {}",
            output
        );
    }

    #[test]
    fn substitute_respecting_opacity_blocks_inside_intensional() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let weak = interner.intern("Weak");
        let clark = interner.intern("Clark");
        let believes = interner.intern("Believes");
        let superman = interner.intern("Superman");

        let inner = expr_arena.alloc(LogicExpr::Predicate {
            name: weak,
            args: term_arena.alloc_slice([Term::Constant(clark)]),
        });
        let expr = expr_arena.alloc(LogicExpr::Intensional {
            operator: believes,
            content: inner,
        });

        let replacement = expr_arena.alloc(LogicExpr::Atom(superman));
        let result = substitute_respecting_opacity(expr, clark, replacement, &expr_arena, &term_arena);

        let mut registry = SymbolRegistry::new();
        let output = result.transpile(&mut registry, &interner, OutputFormat::Unicode);

        assert!(
            output.contains("C") && !output.contains("S"),
            "Should NOT substitute inside intensional context: got {}",
            output
        );
    }

    #[test]
    fn substitute_respecting_opacity_allows_outside() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let weak = interner.intern("Weak");
        let clark = interner.intern("Clark");
        let superman = interner.intern("Superman");

        let expr = expr_arena.alloc(LogicExpr::Predicate {
            name: weak,
            args: term_arena.alloc_slice([Term::Constant(clark)]),
        });

        let replacement = expr_arena.alloc(LogicExpr::Atom(superman));
        let result = substitute_respecting_opacity(expr, clark, replacement, &expr_arena, &term_arena);

        let mut registry = SymbolRegistry::new();
        let output = result.transpile(&mut registry, &interner, OutputFormat::Unicode);

        assert!(
            output.contains("S"),
            "Should substitute outside intensional context: got {}",
            output
        );
    }

    #[test]
    fn factorial_basic() {
        assert_eq!(factorial(0), 1);
        assert_eq!(factorial(1), 1);
        assert_eq!(factorial(2), 2);
        assert_eq!(factorial(3), 6);
        assert_eq!(factorial(4), 24);
        assert_eq!(factorial(5), 120);
    }

    #[test]
    fn scope_iterator_two_quantifiers_yields_two() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");
        let man = interner.intern("Man");
        let woman = interner.intern("Woman");
        let loves = interner.intern("Loves");

        let man_x = expr_arena.alloc(LogicExpr::Predicate {
            name: man,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let woman_y = expr_arena.alloc(LogicExpr::Predicate {
            name: woman,
            args: term_arena.alloc_slice([Term::Variable(y)]),
        });
        let loves_xy = expr_arena.alloc(LogicExpr::Predicate {
            name: loves,
            args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        });

        let inner = expr_arena.alloc(LogicExpr::BinaryOp {
            left: woman_y,
            op: TokenType::And,
            right: loves_xy,
        });
        let inner_q = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: y,
            body: inner,
            island_id: 0,
        });

        let outer = expr_arena.alloc(LogicExpr::BinaryOp {
            left: man_x,
            op: TokenType::If,
            right: inner_q,
        });
        let expr = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body: outer,
            island_id: 0,
        });

        let scopings: Vec<_> = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena).collect();
        assert_eq!(scopings.len(), 2, "Two quantifiers should have 2! = 2 readings");
    }

    #[test]
    fn scope_iterator_three_quantifiers_yields_six() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");
        let z = interner.intern("z");
        let man = interner.intern("Man");
        let woman = interner.intern("Woman");
        let book = interner.intern("Book");
        let gives = interner.intern("Gives");

        let man_x = expr_arena.alloc(LogicExpr::Predicate {
            name: man,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let woman_y = expr_arena.alloc(LogicExpr::Predicate {
            name: woman,
            args: term_arena.alloc_slice([Term::Variable(y)]),
        });
        let book_z = expr_arena.alloc(LogicExpr::Predicate {
            name: book,
            args: term_arena.alloc_slice([Term::Variable(z)]),
        });
        let gives_xyz = expr_arena.alloc(LogicExpr::Predicate {
            name: gives,
            args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y), Term::Variable(z)]),
        });

        let inner_z = expr_arena.alloc(LogicExpr::BinaryOp {
            left: book_z,
            op: TokenType::And,
            right: gives_xyz,
        });
        let q_z = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: z,
            body: inner_z,
            island_id: 0,
        });

        let inner_y = expr_arena.alloc(LogicExpr::BinaryOp {
            left: woman_y,
            op: TokenType::And,
            right: q_z,
        });
        let q_y = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: y,
            body: inner_y,
            island_id: 0,
        });

        let outer = expr_arena.alloc(LogicExpr::BinaryOp {
            left: man_x,
            op: TokenType::If,
            right: q_y,
        });
        let expr = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body: outer,
            island_id: 0,
        });

        let scopings: Vec<_> = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena).collect();
        assert_eq!(scopings.len(), 6, "Three quantifiers should have 3! = 6 readings");
    }

    #[test]
    fn scope_iterator_no_duplicates() {
        use std::collections::HashSet;

        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");
        let man = interner.intern("Man");
        let woman = interner.intern("Woman");
        let loves = interner.intern("Loves");

        let man_x = expr_arena.alloc(LogicExpr::Predicate {
            name: man,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let woman_y = expr_arena.alloc(LogicExpr::Predicate {
            name: woman,
            args: term_arena.alloc_slice([Term::Variable(y)]),
        });
        let loves_xy = expr_arena.alloc(LogicExpr::Predicate {
            name: loves,
            args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        });

        let inner = expr_arena.alloc(LogicExpr::BinaryOp {
            left: woman_y,
            op: TokenType::And,
            right: loves_xy,
        });
        let inner_q = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: y,
            body: inner,
            island_id: 0,
        });

        let outer = expr_arena.alloc(LogicExpr::BinaryOp {
            left: man_x,
            op: TokenType::If,
            right: inner_q,
        });
        let expr = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body: outer,
            island_id: 0,
        });

        let mut registry = SymbolRegistry::new();
        let outputs: HashSet<String> = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena)
            .map(|e| e.transpile(&mut registry, &interner, OutputFormat::Unicode))
            .collect();

        assert_eq!(outputs.len(), 2, "All scopings should be unique");
    }

    #[test]
    fn scope_iterator_exact_size() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");
        let man = interner.intern("Man");
        let woman = interner.intern("Woman");
        let loves = interner.intern("Loves");

        let man_x = expr_arena.alloc(LogicExpr::Predicate {
            name: man,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let woman_y = expr_arena.alloc(LogicExpr::Predicate {
            name: woman,
            args: term_arena.alloc_slice([Term::Variable(y)]),
        });
        let loves_xy = expr_arena.alloc(LogicExpr::Predicate {
            name: loves,
            args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        });

        let inner = expr_arena.alloc(LogicExpr::BinaryOp {
            left: woman_y,
            op: TokenType::And,
            right: loves_xy,
        });
        let inner_q = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: y,
            body: inner,
            island_id: 0,
        });

        let outer = expr_arena.alloc(LogicExpr::BinaryOp {
            left: man_x,
            op: TokenType::If,
            right: inner_q,
        });
        let expr = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body: outer,
            island_id: 0,
        });

        let mut iter = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
        assert_eq!(iter.len(), 2);
        iter.next();
        assert_eq!(iter.len(), 1);
        iter.next();
        assert_eq!(iter.len(), 0);
    }

    #[test]
    fn island_constraints_reduce_permutations() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");
        let man = interner.intern("Man");
        let woman = interner.intern("Woman");
        let loves = interner.intern("Loves");

        let man_x = expr_arena.alloc(LogicExpr::Predicate {
            name: man,
            args: term_arena.alloc_slice([Term::Variable(x)]),
        });
        let woman_y = expr_arena.alloc(LogicExpr::Predicate {
            name: woman,
            args: term_arena.alloc_slice([Term::Variable(y)]),
        });
        let loves_xy = expr_arena.alloc(LogicExpr::Predicate {
            name: loves,
            args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        });

        let inner = expr_arena.alloc(LogicExpr::BinaryOp {
            left: woman_y,
            op: TokenType::And,
            right: loves_xy,
        });
        let inner_q = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: y,
            body: inner,
            island_id: 1,
        });

        let outer = expr_arena.alloc(LogicExpr::BinaryOp {
            left: man_x,
            op: TokenType::If,
            right: inner_q,
        });
        let expr = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body: outer,
            island_id: 0,
        });

        let scopings = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
        assert_eq!(
            scopings.len(),
            1,
            "Two quantifiers in different islands: 1! × 1! = 1 reading (no cross-island scoping)"
        );
    }

    #[test]
    fn multiple_quantifiers_per_island() {
        let mut interner = Interner::new();
        let expr_arena: Arena<LogicExpr> = Arena::new();
        let term_arena: Arena<Term> = Arena::new();

        let x = interner.intern("x");
        let y = interner.intern("y");
        let z = interner.intern("z");
        let w = interner.intern("w");
        let pred = interner.intern("P");

        let core = expr_arena.alloc(LogicExpr::Predicate {
            name: pred,
            args: term_arena.alloc_slice([
                Term::Variable(x),
                Term::Variable(y),
                Term::Variable(z),
                Term::Variable(w),
            ]),
        });

        let true_sym = interner.intern("T");
        let t = expr_arena.alloc(LogicExpr::Atom(true_sym));

        let q_w = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: w,
            body: expr_arena.alloc(LogicExpr::BinaryOp { left: t, op: TokenType::And, right: core }),
            island_id: 1,
        });
        let q_z = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: z,
            body: expr_arena.alloc(LogicExpr::BinaryOp { left: t, op: TokenType::And, right: q_w }),
            island_id: 1,
        });
        let q_y = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Existential,
            variable: y,
            body: expr_arena.alloc(LogicExpr::BinaryOp { left: t, op: TokenType::And, right: q_z }),
            island_id: 0,
        });
        let expr = expr_arena.alloc(LogicExpr::Quantifier {
            kind: QuantifierKind::Universal,
            variable: x,
            body: expr_arena.alloc(LogicExpr::BinaryOp { left: t, op: TokenType::If, right: q_y }),
            island_id: 0,
        });

        let scopings = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
        assert_eq!(
            scopings.len(),
            4,
            "4 quantifiers split 2+2 across islands: 2! × 2! = 4 (not 4! = 24)"
        );
    }
}
