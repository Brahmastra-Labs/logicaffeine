use crate::ast::{LogicExpr, NounPhrase, Term};

pub trait Visitor<'a>: Sized {
    fn visit_expr(&mut self, expr: &'a LogicExpr<'a>) {
        walk_expr(self, expr);
    }

    fn visit_term(&mut self, term: &'a Term<'a>) {
        walk_term(self, term);
    }

    fn visit_np(&mut self, np: &'a NounPhrase<'a>) {
        walk_np(self, np);
    }
}

pub fn walk_expr<'a, V: Visitor<'a>>(v: &mut V, expr: &'a LogicExpr<'a>) {
    match expr {
        LogicExpr::Predicate { args, .. } => {
            for arg in *args {
                v.visit_term(arg);
            }
        }

        LogicExpr::Identity { left, right } => {
            v.visit_term(left);
            v.visit_term(right);
        }

        LogicExpr::Metaphor { tenor, vehicle } => {
            v.visit_term(tenor);
            v.visit_term(vehicle);
        }

        LogicExpr::Quantifier { body, .. } => {
            v.visit_expr(body);
        }

        LogicExpr::Categorical(data) => {
            v.visit_np(&data.subject);
            v.visit_np(&data.predicate);
        }

        LogicExpr::Relation(data) => {
            v.visit_np(&data.subject);
            v.visit_np(&data.object);
        }

        LogicExpr::Modal { operand, .. } => {
            v.visit_expr(operand);
        }

        LogicExpr::Temporal { body, .. } => {
            v.visit_expr(body);
        }

        LogicExpr::Aspectual { body, .. } => {
            v.visit_expr(body);
        }

        LogicExpr::Voice { body, .. } => {
            v.visit_expr(body);
        }

        LogicExpr::BinaryOp { left, right, .. } => {
            v.visit_expr(left);
            v.visit_expr(right);
        }

        LogicExpr::UnaryOp { operand, .. } => {
            v.visit_expr(operand);
        }

        LogicExpr::Question { body, .. } => {
            v.visit_expr(body);
        }

        LogicExpr::YesNoQuestion { body } => {
            v.visit_expr(body);
        }

        LogicExpr::Atom(_) => {}

        LogicExpr::Lambda { body, .. } => {
            v.visit_expr(body);
        }

        LogicExpr::App { function, argument } => {
            v.visit_expr(function);
            v.visit_expr(argument);
        }

        LogicExpr::Intensional { content, .. } => {
            v.visit_expr(content);
        }

        LogicExpr::Event { predicate, .. } => {
            v.visit_expr(predicate);
        }

        LogicExpr::NeoEvent(data) => {
            for (_, term) in data.roles.iter() {
                v.visit_term(term);
            }
        }

        LogicExpr::Imperative { action } => {
            v.visit_expr(action);
        }

        LogicExpr::SpeechAct { content, .. } => {
            v.visit_expr(content);
        }

        LogicExpr::Counterfactual { antecedent, consequent } => {
            v.visit_expr(antecedent);
            v.visit_expr(consequent);
        }

        LogicExpr::Causal { effect, cause } => {
            v.visit_expr(cause);
            v.visit_expr(effect);
        }

        LogicExpr::Comparative { subject, object, .. } => {
            v.visit_term(subject);
            v.visit_term(object);
        }

        LogicExpr::Superlative { subject, .. } => {
            v.visit_term(subject);
        }

        LogicExpr::Scopal { body, .. } => {
            v.visit_expr(body);
        }

        LogicExpr::Control { subject, object, infinitive, .. } => {
            v.visit_term(subject);
            if let Some(obj) = object {
                v.visit_term(obj);
            }
            v.visit_expr(infinitive);
        }

        LogicExpr::Presupposition { assertion, presupposition } => {
            v.visit_expr(assertion);
            v.visit_expr(presupposition);
        }

        LogicExpr::Focus { focused, scope, .. } => {
            v.visit_term(focused);
            v.visit_expr(scope);
        }

        LogicExpr::TemporalAnchor { body, .. } => {
            v.visit_expr(body);
        }

        LogicExpr::Distributive { predicate } => {
            v.visit_expr(predicate);
        }

        LogicExpr::GroupQuantifier { restriction, body, .. } => {
            v.visit_expr(restriction);
            v.visit_expr(body);
        }
    }
}

pub fn walk_term<'a, V: Visitor<'a>>(v: &mut V, term: &'a Term<'a>) {
    match term {
        Term::Constant(_) | Term::Variable(_) | Term::Sigma(_) | Term::Intension(_) | Term::Value { .. } => {}

        Term::Function(_, args) => {
            for arg in *args {
                v.visit_term(arg);
            }
        }

        Term::Group(members) => {
            for m in *members {
                v.visit_term(m);
            }
        }

        Term::Possessed { possessor, .. } => {
            v.visit_term(possessor);
        }

        Term::Proposition(expr) => {
            v.visit_expr(expr);
        }
    }
}

pub fn walk_np<'a, V: Visitor<'a>>(v: &mut V, np: &'a NounPhrase<'a>) {
    if let Some(poss) = np.possessor {
        v.visit_np(poss);
    }
    for pp in np.pps.iter() {
        v.visit_expr(pp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intern::Symbol;

    struct VariableCollector {
        variables: Vec<Symbol>,
    }

    impl<'a> Visitor<'a> for VariableCollector {
        fn visit_term(&mut self, term: &'a Term<'a>) {
            if let Term::Variable(sym) = term {
                self.variables.push(*sym);
            }
            walk_term(self, term);
        }
    }

    struct ExprCounter {
        count: usize,
    }

    impl<'a> Visitor<'a> for ExprCounter {
        fn visit_expr(&mut self, expr: &'a LogicExpr<'a>) {
            self.count += 1;
            walk_expr(self, expr);
        }
    }

    #[test]
    fn variable_collector_finds_variables() {
        use crate::arena::Arena;
        use crate::intern::Interner;

        let mut interner = Interner::new();
        let x = interner.intern("x");
        let y = interner.intern("y");

        let term_arena: Arena<Term> = Arena::new();
        let terms = term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]);

        let expr_arena: Arena<LogicExpr> = Arena::new();
        let pred = interner.intern("P");
        let expr = expr_arena.alloc(LogicExpr::Predicate { name: pred, args: terms });

        let mut collector = VariableCollector { variables: vec![] };
        collector.visit_expr(expr);

        assert_eq!(collector.variables.len(), 2);
        assert!(collector.variables.contains(&x));
        assert!(collector.variables.contains(&y));
    }

    #[test]
    fn expr_counter_counts_nested() {
        use crate::arena::Arena;
        use crate::intern::Interner;
        use crate::token::TokenType;

        let mut interner = Interner::new();
        let p = interner.intern("P");
        let q = interner.intern("Q");

        let expr_arena: Arena<LogicExpr> = Arena::new();

        let left = expr_arena.alloc(LogicExpr::Atom(p));
        let right = expr_arena.alloc(LogicExpr::Atom(q));
        let binary = expr_arena.alloc(LogicExpr::BinaryOp {
            left,
            op: TokenType::And,
            right,
        });

        let mut counter = ExprCounter { count: 0 };
        counter.visit_expr(binary);

        assert_eq!(counter.count, 3);
    }
}
