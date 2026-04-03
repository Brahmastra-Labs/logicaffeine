//! Constraint-based type inference for VerifyExpr.
//!
//! Walks the expression tree, collects type constraints from operators,
//! unifies constraints per variable, detects conflicts, and propagates
//! bitvector widths.

use crate::ir::{BitVecOp, VerifyExpr, VerifyOp, VerifyType};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum TypeError {
    Conflict {
        var: String,
        expected: VerifyType,
        found: VerifyType,
    },
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeError::Conflict { var, expected, found } => {
                write!(f, "Type conflict for '{}': expected {:?}, found {:?}", var, expected, found)
            }
        }
    }
}

impl std::error::Error for TypeError {}

/// Infer types for all variables in a VerifyExpr.
///
/// Returns a map from variable name to inferred type.
/// Returns TypeError if a variable is used with conflicting types.
pub fn infer_types(expr: &VerifyExpr) -> Result<HashMap<String, VerifyType>, TypeError> {
    let mut constraints: HashMap<String, VerifyType> = HashMap::new();
    collect_constraints(expr, &mut constraints)?;
    Ok(constraints)
}

fn collect_constraints(
    expr: &VerifyExpr,
    constraints: &mut HashMap<String, VerifyType>,
) -> Result<(), TypeError> {
    match expr {
        VerifyExpr::Bool(_) | VerifyExpr::Int(_) => Ok(()),

        VerifyExpr::Var(_) => Ok(()),

        VerifyExpr::Binary { op, left, right } => {
            match op {
                // Boolean operators: operands must be Bool
                VerifyOp::And | VerifyOp::Or | VerifyOp::Implies => {
                    constrain_as_bool(left, constraints)?;
                    constrain_as_bool(right, constraints)?;
                }
                // Arithmetic operators: operands must be Int
                VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div => {
                    constrain_as_int(left, constraints)?;
                    constrain_as_int(right, constraints)?;
                }
                // Comparison operators: operands must be Int
                VerifyOp::Gt | VerifyOp::Lt | VerifyOp::Gte | VerifyOp::Lte => {
                    constrain_as_int(left, constraints)?;
                    constrain_as_int(right, constraints)?;
                }
                // Equality: operands must match but could be any sort
                VerifyOp::Eq | VerifyOp::Neq => {
                    // If either side is clearly Int, constrain both
                    if expr_suggests_int(left) || expr_suggests_int(right) {
                        constrain_as_int(left, constraints)?;
                        constrain_as_int(right, constraints)?;
                    }
                }
            }
            collect_constraints(left, constraints)?;
            collect_constraints(right, constraints)?;
            Ok(())
        }

        VerifyExpr::Not(inner) => {
            constrain_as_bool(inner, constraints)?;
            collect_constraints(inner, constraints)
        }

        VerifyExpr::Iff(l, r) => {
            collect_constraints(l, constraints)?;
            collect_constraints(r, constraints)
        }

        VerifyExpr::ForAll { body, vars } | VerifyExpr::Exists { body, vars } => {
            for (name, ty) in vars {
                add_constraint(name, ty.clone(), constraints)?;
            }
            collect_constraints(body, constraints)
        }

        VerifyExpr::Apply { args, .. } => {
            for arg in args {
                collect_constraints(arg, constraints)?;
            }
            Ok(())
        }

        // Bitvector operations
        VerifyExpr::BitVecConst { .. } => Ok(()),

        VerifyExpr::BitVecBinary { op: _, left, right } => {
            let width = bv_width_hint(left).or_else(|| bv_width_hint(right));
            if let Some(w) = width {
                constrain_as_bv(left, w, constraints)?;
                constrain_as_bv(right, w, constraints)?;
            }
            collect_constraints(left, constraints)?;
            collect_constraints(right, constraints)
        }

        VerifyExpr::BitVecExtract { high, low, operand } => {
            // Operand must be a BV with width > high
            let min_width = high + 1;
            constrain_as_bv(operand, min_width, constraints)?;
            collect_constraints(operand, constraints)
        }

        VerifyExpr::BitVecConcat(l, r) => {
            collect_constraints(l, constraints)?;
            collect_constraints(r, constraints)
        }

        // Array operations
        VerifyExpr::Select { array, index } => {
            if let VerifyExpr::Var(name) = array.as_ref() {
                let idx_ty = infer_sort_of(index);
                let elem_ty = VerifyType::Int; // default
                add_constraint(name, VerifyType::Array(Box::new(idx_ty), Box::new(elem_ty)), constraints)?;
            }
            collect_constraints(array, constraints)?;
            collect_constraints(index, constraints)
        }

        VerifyExpr::Store { array, index, value } => {
            if let VerifyExpr::Var(name) = array.as_ref() {
                let idx_ty = infer_sort_of(index);
                let val_ty = infer_sort_of(value);
                add_constraint(name, VerifyType::Array(Box::new(idx_ty), Box::new(val_ty)), constraints)?;
            }
            collect_constraints(array, constraints)?;
            collect_constraints(index, constraints)?;
            collect_constraints(value, constraints)
        }

        VerifyExpr::AtState { state, expr } => {
            collect_constraints(state, constraints)?;
            collect_constraints(expr, constraints)
        }

        VerifyExpr::Transition { from, to } => {
            collect_constraints(from, constraints)?;
            collect_constraints(to, constraints)
        }
    }
}

fn constrain_as_bool(
    expr: &VerifyExpr,
    constraints: &mut HashMap<String, VerifyType>,
) -> Result<(), TypeError> {
    if let VerifyExpr::Var(name) = expr {
        add_constraint(name, VerifyType::Bool, constraints)?;
    }
    Ok(())
}

fn constrain_as_int(
    expr: &VerifyExpr,
    constraints: &mut HashMap<String, VerifyType>,
) -> Result<(), TypeError> {
    if let VerifyExpr::Var(name) = expr {
        add_constraint(name, VerifyType::Int, constraints)?;
    }
    // Also handle nested binary expressions with Var leaves
    if let VerifyExpr::Binary { left, right, .. } = expr {
        constrain_as_int(left, constraints)?;
        constrain_as_int(right, constraints)?;
    }
    Ok(())
}

fn constrain_as_bv(
    expr: &VerifyExpr,
    width: u32,
    constraints: &mut HashMap<String, VerifyType>,
) -> Result<(), TypeError> {
    if let VerifyExpr::Var(name) = expr {
        add_constraint(name, VerifyType::BitVector(width), constraints)?;
    }
    Ok(())
}

fn add_constraint(
    name: &str,
    ty: VerifyType,
    constraints: &mut HashMap<String, VerifyType>,
) -> Result<(), TypeError> {
    if let Some(existing) = constraints.get(name) {
        if !types_compatible(existing, &ty) {
            return Err(TypeError::Conflict {
                var: name.to_string(),
                expected: existing.clone(),
                found: ty,
            });
        }
        // If new type is more specific, update
        if type_specificity(&ty) > type_specificity(existing) {
            constraints.insert(name.to_string(), ty);
        }
    } else {
        constraints.insert(name.to_string(), ty);
    }
    Ok(())
}

fn types_compatible(a: &VerifyType, b: &VerifyType) -> bool {
    match (a, b) {
        (VerifyType::Int, VerifyType::Int) => true,
        (VerifyType::Bool, VerifyType::Bool) => true,
        (VerifyType::Object, _) | (_, VerifyType::Object) => true,
        (VerifyType::BitVector(w1), VerifyType::BitVector(w2)) => w1 == w2,
        (VerifyType::Array(i1, e1), VerifyType::Array(i2, e2)) => {
            types_compatible(i1, i2) && types_compatible(e1, e2)
        }
        _ => false,
    }
}

fn type_specificity(ty: &VerifyType) -> u8 {
    match ty {
        VerifyType::Object => 0,
        VerifyType::Bool => 1,
        VerifyType::Int => 1,
        VerifyType::BitVector(_) => 2,
        VerifyType::Array(_, _) => 2,
    }
}

fn bv_width_hint(expr: &VerifyExpr) -> Option<u32> {
    match expr {
        VerifyExpr::BitVecConst { width, .. } => Some(*width),
        VerifyExpr::BitVecBinary { left, right, .. } => {
            bv_width_hint(left).or_else(|| bv_width_hint(right))
        }
        VerifyExpr::BitVecExtract { high, low, .. } => Some(high - low + 1),
        _ => None,
    }
}

fn expr_suggests_int(expr: &VerifyExpr) -> bool {
    matches!(
        expr,
        VerifyExpr::Int(_)
            | VerifyExpr::Binary {
                op: VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div,
                ..
            }
    )
}

fn infer_sort_of(expr: &VerifyExpr) -> VerifyType {
    match expr {
        VerifyExpr::Int(_) => VerifyType::Int,
        VerifyExpr::Bool(_) => VerifyType::Bool,
        VerifyExpr::BitVecConst { width, .. } => VerifyType::BitVector(*width),
        VerifyExpr::Binary { op: VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div, .. } => VerifyType::Int,
        _ => VerifyType::Int, // default
    }
}
