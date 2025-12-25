use std::fmt::Write;

use crate::ast::logic::{LogicExpr, NumberKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::Interner;
use crate::token::TokenType;

pub fn codegen_program(stmts: &[Stmt], interner: &Interner) -> String {
    let mut output = String::new();
    writeln!(output, "fn main() {{").unwrap();
    for stmt in stmts {
        output.push_str(&codegen_stmt(stmt, interner, 1));
    }
    writeln!(output, "}}").unwrap();
    output
}

pub fn codegen_stmt(stmt: &Stmt, interner: &Interner, indent: usize) -> String {
    let indent_str = "    ".repeat(indent);
    let mut output = String::new();

    match stmt {
        Stmt::Let { var, value, mutable } => {
            let var_name = interner.resolve(*var);
            let value_str = codegen_expr(value, interner);
            if *mutable {
                writeln!(output, "{}let mut {} = {};", indent_str, var_name, value_str).unwrap();
            } else {
                writeln!(output, "{}let {} = {};", indent_str, var_name, value_str).unwrap();
            }
        }

        Stmt::Set { target, value } => {
            let target_name = interner.resolve(*target);
            let value_str = codegen_expr(value, interner);
            writeln!(output, "{}{} = {};", indent_str, target_name, value_str).unwrap();
        }

        Stmt::Call { function, args } => {
            let func_name = interner.resolve(*function);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, interner)).collect();
            writeln!(output, "{}{}({});", indent_str, func_name, args_str.join(", ")).unwrap();
        }

        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr(cond, interner);
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            for stmt in *then_block {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1));
            }
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                for stmt in *else_stmts {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 1));
                }
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::While { cond, body } => {
            let cond_str = codegen_expr(cond, interner);
            writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1));
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Return { value } => {
            if let Some(v) = value {
                let value_str = codegen_expr(v, interner);
                writeln!(output, "{}return {};", indent_str, value_str).unwrap();
            } else {
                writeln!(output, "{}return;", indent_str).unwrap();
            }
        }

        Stmt::Assert { proposition } => {
            let condition = codegen_assertion(proposition, interner);
            writeln!(output, "{}debug_assert!({});", indent_str, condition).unwrap();
        }
    }

    output
}

pub fn codegen_expr(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Literal(lit) => codegen_literal(lit, interner),

        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),

        Expr::BinaryOp { op, left, right } => {
            let left_str = codegen_expr(left, interner);
            let right_str = codegen_expr(right, interner);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::GtEq => ">=",
            };
            format!("({} {} {})", left_str, op_str, right_str)
        }

        Expr::Call { function, args } => {
            let func_name = interner.resolve(*function);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, interner)).collect();
            format!("{}({})", func_name, args_str.join(", "))
        }

        Expr::Index { collection, index } => {
            let coll_str = codegen_expr(collection, interner);
            format!("{}[{}]", coll_str, index - 1)
        }

        Expr::Slice { collection, start, end } => {
            let coll_str = codegen_expr(collection, interner);
            // 1-indexed to 0-indexed: items 2 through 5 → &list[1..5]
            format!("&{}[{}..{}]", coll_str, start - 1, end)
        }
    }
}

fn codegen_literal(lit: &Literal, interner: &Interner) -> String {
    match lit {
        Literal::Number(n) => n.to_string(),
        Literal::Text(sym) => format!("\"{}\"", interner.resolve(*sym)),
        Literal::Boolean(b) => b.to_string(),
        Literal::Nothing => "()".to_string(),
    }
}

pub fn codegen_assertion(expr: &LogicExpr, interner: &Interner) -> String {
    match expr {
        LogicExpr::Atom(sym) => interner.resolve(*sym).to_string(),

        LogicExpr::Identity { left, right } => {
            let left_str = codegen_term(left, interner);
            let right_str = codegen_term(right, interner);
            format!("({} == {})", left_str, right_str)
        }

        LogicExpr::Predicate { name, args } => {
            let pred_name = interner.resolve(*name).to_lowercase();
            match pred_name.as_str() {
                "greater" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} > {})", left, right)
                }
                "less" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} < {})", left, right)
                }
                "equal" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} == {})", left, right)
                }
                "greaterequal" | "greaterorequal" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} >= {})", left, right)
                }
                "lessequal" | "lessorequal" if args.len() == 2 => {
                    let left = codegen_term(&args[0], interner);
                    let right = codegen_term(&args[1], interner);
                    format!("({} <= {})", left, right)
                }
                "positive" if args.len() == 1 => {
                    let arg = codegen_term(&args[0], interner);
                    format!("({} > 0)", arg)
                }
                "negative" if args.len() == 1 => {
                    let arg = codegen_term(&args[0], interner);
                    format!("({} < 0)", arg)
                }
                "zero" if args.len() == 1 => {
                    let arg = codegen_term(&args[0], interner);
                    format!("({} == 0)", arg)
                }
                _ => {
                    let args_str: Vec<String> = args.iter()
                        .map(|a| codegen_term(a, interner))
                        .collect();
                    format!("{}({})", interner.resolve(*name), args_str.join(", "))
                }
            }
        }

        LogicExpr::BinaryOp { left, op, right } => {
            let left_str = codegen_assertion(left, interner);
            let right_str = codegen_assertion(right, interner);
            let op_str = match op {
                TokenType::And => "&&",
                TokenType::Or => "||",
                TokenType::Iff => "==",
                _ => "/* unknown op */",
            };
            format!("({} {} {})", left_str, op_str, right_str)
        }

        LogicExpr::UnaryOp { op, operand } => {
            let operand_str = codegen_assertion(operand, interner);
            match op {
                TokenType::Not => format!("(!{})", operand_str),
                _ => format!("/* unknown unary op */({})", operand_str),
            }
        }

        LogicExpr::Comparative { adjective, subject, object, .. } => {
            let adj_name = interner.resolve(*adjective).to_lowercase();
            let subj_str = codegen_term(subject, interner);
            let obj_str = codegen_term(object, interner);
            match adj_name.as_str() {
                "great" | "big" | "large" | "tall" | "old" | "high" => {
                    format!("({} > {})", subj_str, obj_str)
                }
                "small" | "little" | "short" | "young" | "low" => {
                    format!("({} < {})", subj_str, obj_str)
                }
                _ => format!("({} > {})", subj_str, obj_str), // default to greater-than
            }
        }

        _ => "/* unsupported LogicExpr */true".to_string(),
    }
}

pub fn codegen_term(term: &Term, interner: &Interner) -> String {
    match term {
        Term::Constant(sym) => interner.resolve(*sym).to_string(),
        Term::Variable(sym) => interner.resolve(*sym).to_string(),
        Term::Value { kind, .. } => match kind {
            NumberKind::Integer(n) => n.to_string(),
            NumberKind::Real(f) => f.to_string(),
            NumberKind::Symbolic(sym) => interner.resolve(*sym).to_string(),
        },
        Term::Function(name, args) => {
            let args_str: Vec<String> = args.iter()
                .map(|a| codegen_term(a, interner))
                .collect();
            format!("{}({})", interner.resolve(*name), args_str.join(", "))
        }
        Term::Possessed { possessor, possessed } => {
            let poss_str = codegen_term(possessor, interner);
            format!("{}.{}", poss_str, interner.resolve(*possessed))
        }
        Term::Group(members) => {
            let members_str: Vec<String> = members.iter()
                .map(|m| codegen_term(m, interner))
                .collect();
            format!("({})", members_str.join(", "))
        }
        _ => "/* unsupported Term */".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_number() {
        let interner = Interner::new();
        let expr = Expr::Literal(Literal::Number(42));
        assert_eq!(codegen_expr(&expr, &interner), "42");
    }

    #[test]
    fn test_literal_boolean() {
        let interner = Interner::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(true)), &interner), "true");
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(false)), &interner), "false");
    }

    #[test]
    fn test_literal_nothing() {
        let interner = Interner::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Nothing), &interner), "()");
    }
}
