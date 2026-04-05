//! Spec Encoding: BoundedExpr → Kernel Term
//!
//! Translates hardware verification expressions from the bounded timestep model
//! into kernel Terms. Under Curry-Howard, these Terms are types — inhabiting
//! them constitutes a proof that the hardware satisfies the specification.
//!
//! This is the bridge between the SVA verification pipeline and the CoIC kernel.

use logicaffeine_kernel::{Term, Literal};
use super::sva_to_verify::BoundedExpr;

/// Encode a BoundedExpr as a kernel Term.
///
/// The encoding follows Curry-Howard:
/// - Bool(true) → Global("True") (trivially inhabited proposition)
/// - Bool(false) → Global("False") (uninhabited proposition)
/// - And(l, r) → App(App(Global("And"), l'), r')
/// - Or(l, r) → App(App(Global("Or"), l'), r')
/// - Not(e) → App(Global("Not"), e')
/// - Implies(l, r) → Pi("_", l', r') (function type = implication)
/// - Eq(l, r) → App(App(App(Global("Eq"), Hole), l'), r')
/// - Var("sig@t") → App(Global("sig"), nat_literal(t))
/// - Int(n) → Lit(Int(n))
pub fn encode_bounded_expr(expr: &BoundedExpr) -> Term {
    match expr {
        BoundedExpr::Bool(true) => Term::Global("True".to_string()),
        BoundedExpr::Bool(false) => Term::Global("False".to_string()),

        BoundedExpr::Int(n) => Term::Lit(Literal::Int(*n)),

        BoundedExpr::Var(name) => {
            if let Some(at_pos) = name.find('@') {
                let sig = &name[..at_pos];
                let t: i64 = name[at_pos + 1..].parse().unwrap_or(0);
                // Signal as function of time: sig(t)
                Term::App(
                    Box::new(Term::Global(sig.to_string())),
                    Box::new(Term::Lit(Literal::Int(t))),
                )
            } else {
                Term::Var(name.clone())
            }
        }

        BoundedExpr::And(left, right) => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("And".to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }

        BoundedExpr::Or(left, right) => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Or".to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }

        BoundedExpr::Not(inner) => {
            let e = encode_bounded_expr(inner);
            Term::App(
                Box::new(Term::Global("Not".to_string())),
                Box::new(e),
            )
        }

        BoundedExpr::Implies(left, right) => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            // Under Curry-Howard: implication = function type (Pi)
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(l),
                body_type: Box::new(r),
            }
        }

        BoundedExpr::Eq(left, right) => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            // Eq _ l r (type inferred via Hole)
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("Eq".to_string())),
                        Box::new(Term::Hole),
                    )),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }

        BoundedExpr::Lt(left, right) => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("lt".to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }

        BoundedExpr::Gt(left, right) => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("gt".to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }

        BoundedExpr::Lte(left, right) => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("le".to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }

        BoundedExpr::Gte(left, right) => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("ge".to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }

        BoundedExpr::Unsupported(msg) => {
            // Unsupported constructs become False — fail closed
            let _ = msg;
            Term::Global("False".to_string())
        }

        // Multi-sorted extensions: encode as appropriate kernel terms
        BoundedExpr::BitVecConst { value, .. } => Term::Lit(Literal::Int(*value as i64)),
        BoundedExpr::BitVecVar(name, _) => Term::Var(name.clone()),
        BoundedExpr::BitVecBinary { op, left, right } => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            let op_name = match op {
                super::sva_to_verify::BitVecBoundedOp::And => "bit_and",
                super::sva_to_verify::BitVecBoundedOp::Or => "bit_or",
                super::sva_to_verify::BitVecBoundedOp::Xor => "bit_xor",
                super::sva_to_verify::BitVecBoundedOp::Add => "bv_add",
                super::sva_to_verify::BitVecBoundedOp::Sub => "bv_sub",
                _ => "bv_op",
            };
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global(op_name.to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }
        BoundedExpr::BitVecExtract { operand, .. } => encode_bounded_expr(operand),
        BoundedExpr::BitVecConcat(l, r) => {
            let left = encode_bounded_expr(l);
            let right = encode_bounded_expr(r);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("bv_concat".to_string())),
                    Box::new(left),
                )),
                Box::new(right),
            )
        }
        BoundedExpr::ArraySelect { array, index } => {
            let a = encode_bounded_expr(array);
            let i = encode_bounded_expr(index);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("select".to_string())),
                    Box::new(a),
                )),
                Box::new(i),
            )
        }
        BoundedExpr::ArrayStore { array, index, value } => {
            let a = encode_bounded_expr(array);
            let i = encode_bounded_expr(index);
            let v = encode_bounded_expr(value);
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("store".to_string())),
                        Box::new(a),
                    )),
                    Box::new(i),
                )),
                Box::new(v),
            )
        }
        BoundedExpr::IntBinary { op, left, right } => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            let op_name = match op {
                super::sva_to_verify::ArithBoundedOp::Add => "plus",
                super::sva_to_verify::ArithBoundedOp::Sub => "minus",
                super::sva_to_verify::ArithBoundedOp::Mul => "mult",
                super::sva_to_verify::ArithBoundedOp::Div => "div",
            };
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global(op_name.to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }
        BoundedExpr::Comparison { op, left, right } => {
            let l = encode_bounded_expr(left);
            let r = encode_bounded_expr(right);
            let op_name = match op {
                super::sva_to_verify::CmpBoundedOp::Gt => "gt",
                super::sva_to_verify::CmpBoundedOp::Lt => "lt",
                super::sva_to_verify::CmpBoundedOp::Gte => "ge",
                super::sva_to_verify::CmpBoundedOp::Lte => "le",
            };
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global(op_name.to_string())),
                    Box::new(l),
                )),
                Box::new(r),
            )
        }
        BoundedExpr::ForAll { var, body, .. } => {
            let b = encode_bounded_expr(body);
            Term::Pi {
                param: var.clone(),
                param_type: Box::new(Term::Hole),
                body_type: Box::new(b),
            }
        }
        BoundedExpr::Exists { var, body, .. } => {
            let b = encode_bounded_expr(body);
            Term::App(
                Box::new(Term::Global("Ex".to_string())),
                Box::new(Term::Lambda {
                    param: var.clone(),
                    param_type: Box::new(Term::Hole),
                    body: Box::new(b),
                }),
            )
        }
        BoundedExpr::Apply { name, args } => {
            // Uninterpreted function: encode as nested application f(a1)(a2)...
            let mut term = Term::Global(name.clone());
            for arg in args {
                term = Term::App(
                    Box::new(term),
                    Box::new(encode_bounded_expr(arg)),
                );
            }
            term
        }
    }
}

/// Helper: build a Nat literal (Zero, Succ Zero, Succ (Succ Zero), ...)
#[allow(dead_code)]
fn nat_literal(n: usize) -> Term {
    let mut term = Term::Global("Zero".to_string());
    for _ in 0..n {
        term = Term::App(
            Box::new(Term::Global("Succ".to_string())),
            Box::new(term),
        );
    }
    term
}
