use crate::ast::{LogicExpr, ModalDomain, ThematicRole, Term};
use crate::arena::Arena;
use crate::intern::Interner;

pub fn apply_pragmatics<'a>(
    expr: &'a LogicExpr<'a>,
    expr_arena: &'a Arena<LogicExpr<'a>>,
    interner: &Interner,
) -> &'a LogicExpr<'a> {
    match expr {
        LogicExpr::Modal { vector, operand } => {
            if vector.domain == ModalDomain::Alethic && vector.force > 0.0 && vector.force < 1.0 {
                if is_addressee_agent(operand, interner) {
                    return expr_arena.alloc(LogicExpr::Imperative { action: *operand });
                }
            }
            expr
        }
        LogicExpr::Question { body, .. } => {
            if let LogicExpr::Modal { vector, operand } = body {
                if vector.domain == ModalDomain::Alethic && vector.force > 0.0 && vector.force < 1.0 {
                    if is_addressee_agent(operand, interner) {
                        return expr_arena.alloc(LogicExpr::Imperative { action: *operand });
                    }
                }
            }
            expr
        }
        LogicExpr::YesNoQuestion { body } => {
            if let LogicExpr::Modal { vector, operand } = body {
                if vector.domain == ModalDomain::Alethic && vector.force > 0.0 && vector.force < 1.0 {
                    if is_addressee_agent(operand, interner) {
                        return expr_arena.alloc(LogicExpr::Imperative { action: *operand });
                    }
                }
            }
            expr
        }
        _ => expr,
    }
}

fn is_addressee_agent(expr: &LogicExpr, interner: &Interner) -> bool {
    match expr {
        LogicExpr::NeoEvent(data) => {
            for (role, term) in data.roles.iter() {
                if *role == ThematicRole::Agent {
                    if let Term::Constant(sym) = term {
                        let name = interner.resolve(*sym);
                        if name == "Addressee" {
                            return true;
                        }
                    }
                }
            }
            false
        }
        LogicExpr::Predicate { args, .. } => {
            if let Some(Term::Constant(sym)) = args.first() {
                let name = interner.resolve(*sym);
                return name == "Addressee";
            }
            false
        }
        _ => false,
    }
}
