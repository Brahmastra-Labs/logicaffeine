//! SMT-LIB2 Export
//!
//! Export VerifyExpr to standard SMT-LIB2 format readable by Z3, CVC5, Yices, etc.

use crate::ir::{VerifyExpr, VerifyOp, VerifyType, BitVecOp};
use std::collections::HashSet;

/// Export a VerifyExpr to SMT-LIB2 format.
pub fn to_smtlib2(expr: &VerifyExpr, declarations: &[(&str, VerifyType)]) -> String {
    let mut output = String::new();
    output.push_str("(set-logic ALL)\n");

    // Declarations
    for (name, ty) in declarations {
        let sort = type_to_smtlib(ty);
        output.push_str(&format!("(declare-fun {} () {})\n", name, sort));
    }

    // Auto-declare any variables not in declarations
    let mut vars = HashSet::new();
    crate::equivalence::collect_vars_pub(expr, &mut vars);
    let declared: HashSet<String> = declarations.iter().map(|(n, _)| n.to_string()).collect();
    for var in &vars {
        if !declared.contains(var) {
            output.push_str(&format!("(declare-fun {} () Int)\n", var));
        }
    }

    output.push_str(&format!("(assert {})\n", expr_to_smtlib(expr)));
    output.push_str("(check-sat)\n");
    output.push_str("(get-model)\n");
    output
}

/// Export an equivalence check to SMT-LIB2 format.
pub fn equivalence_to_smtlib2(a: &VerifyExpr, b: &VerifyExpr) -> String {
    let mut output = String::new();
    output.push_str("(set-logic ALL)\n");

    let mut vars = HashSet::new();
    crate::equivalence::collect_vars_pub(a, &mut vars);
    crate::equivalence::collect_vars_pub(b, &mut vars);
    for var in &vars {
        output.push_str(&format!("(declare-fun {} () Int)\n", var));
    }

    let a_smt = expr_to_smtlib(a);
    let b_smt = expr_to_smtlib(b);
    output.push_str(&format!("(assert (not (= {} {})))\n", a_smt, b_smt));
    output.push_str("(check-sat)\n");
    output
}

fn type_to_smtlib(ty: &VerifyType) -> String {
    match ty {
        VerifyType::Int => "Int".into(),
        VerifyType::Bool => "Bool".into(),
        VerifyType::Object => "Int".into(),
        VerifyType::Real => "Real".into(),
        VerifyType::BitVector(w) => format!("(_ BitVec {})", w),
        VerifyType::Array(idx, elem) => {
            format!("(Array {} {})", type_to_smtlib(idx), type_to_smtlib(elem))
        }
    }
}

fn expr_to_smtlib(expr: &VerifyExpr) -> String {
    match expr {
        VerifyExpr::Bool(true) => "true".into(),
        VerifyExpr::Bool(false) => "false".into(),
        VerifyExpr::Int(n) => {
            if *n < 0 { format!("(- {})", -n) } else { n.to_string() }
        }
        VerifyExpr::Var(name) => name.clone(),
        VerifyExpr::Binary { op, left, right } => {
            let l = expr_to_smtlib(left);
            let r = expr_to_smtlib(right);
            let op_str = match op {
                VerifyOp::Add => "+",
                VerifyOp::Sub => "-",
                VerifyOp::Mul => "*",
                VerifyOp::Div => "div",
                VerifyOp::Eq => "=",
                VerifyOp::Neq => return format!("(not (= {} {}))", l, r),
                VerifyOp::Gt => ">",
                VerifyOp::Lt => "<",
                VerifyOp::Gte => ">=",
                VerifyOp::Lte => "<=",
                VerifyOp::And => "and",
                VerifyOp::Or => "or",
                VerifyOp::Implies => "=>",
            };
            format!("({} {} {})", op_str, l, r)
        }
        VerifyExpr::Not(inner) => format!("(not {})", expr_to_smtlib(inner)),
        VerifyExpr::Iff(l, r) => format!("(= {} {})", expr_to_smtlib(l), expr_to_smtlib(r)),
        VerifyExpr::ForAll { vars, body } => {
            let bindings: Vec<String> = vars.iter()
                .map(|(n, t)| format!("({} {})", n, type_to_smtlib(t)))
                .collect();
            format!("(forall ({}) {})", bindings.join(" "), expr_to_smtlib(body))
        }
        VerifyExpr::Exists { vars, body } => {
            let bindings: Vec<String> = vars.iter()
                .map(|(n, t)| format!("({} {})", n, type_to_smtlib(t)))
                .collect();
            format!("(exists ({}) {})", bindings.join(" "), expr_to_smtlib(body))
        }
        VerifyExpr::Apply { name, args } => {
            let arg_strs: Vec<String> = args.iter().map(expr_to_smtlib).collect();
            if arg_strs.is_empty() {
                name.clone()
            } else {
                format!("({} {})", name, arg_strs.join(" "))
            }
        }
        VerifyExpr::BitVecConst { width, value } => {
            format!("#x{:0>width$X}", value, width = ((*width + 3) / 4) as usize)
        }
        VerifyExpr::BitVecBinary { op, left, right } => {
            let l = expr_to_smtlib(left);
            let r = expr_to_smtlib(right);
            let op_str = match op {
                BitVecOp::And => "bvand",
                BitVecOp::Or => "bvor",
                BitVecOp::Xor => "bvxor",
                BitVecOp::Not => return format!("(bvnot {})", l),
                BitVecOp::Shl => "bvshl",
                BitVecOp::Shr => "bvlshr",
                BitVecOp::AShr => "bvashr",
                BitVecOp::Add => "bvadd",
                BitVecOp::Sub => "bvsub",
                BitVecOp::Mul => "bvmul",
                BitVecOp::ULt => "bvult",
                BitVecOp::SLt => "bvslt",
                BitVecOp::ULe => "bvule",
                BitVecOp::SLe => "bvsle",
                BitVecOp::Eq => return format!("(= {} {})", l, r),
            };
            format!("({} {} {})", op_str, l, r)
        }
        VerifyExpr::BitVecExtract { high, low, operand } => {
            format!("((_ extract {} {}) {})", high, low, expr_to_smtlib(operand))
        }
        VerifyExpr::BitVecConcat(l, r) => {
            format!("(concat {} {})", expr_to_smtlib(l), expr_to_smtlib(r))
        }
        VerifyExpr::Select { array, index } => {
            format!("(select {} {})", expr_to_smtlib(array), expr_to_smtlib(index))
        }
        VerifyExpr::Store { array, index, value } => {
            format!("(store {} {} {})", expr_to_smtlib(array), expr_to_smtlib(index), expr_to_smtlib(value))
        }
        VerifyExpr::AtState { expr, .. } => expr_to_smtlib(expr),
        VerifyExpr::Transition { from, to } => {
            format!("(and {} {})", expr_to_smtlib(from), expr_to_smtlib(to))
        }
    }
}
