use super::*;
use logicaffeine_base::Symbol;

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
    use logicaffeine_base::Arena;
    use logicaffeine_base::Interner;

    let mut interner = Interner::new();
    let x = interner.intern("x");
    let y = interner.intern("y");

    let term_arena: Arena<Term> = Arena::new();
    let terms = term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]);

    let expr_arena: Arena<LogicExpr> = Arena::new();
    let pred = interner.intern("P");
    let expr = expr_arena.alloc(LogicExpr::Predicate {
        name: pred,
        args: terms,
        world: None,
    });

    let mut collector = VariableCollector { variables: vec![] };
    collector.visit_expr(expr);

    assert_eq!(collector.variables.len(), 2);
    assert!(collector.variables.contains(&x));
    assert!(collector.variables.contains(&y));
}

#[test]
fn expr_counter_counts_nested() {
    use crate::token::TokenType;
    use logicaffeine_base::Arena;
    use logicaffeine_base::Interner;

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
