use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, Pattern};
use crate::intern::Symbol;

#[derive(Clone, Debug, PartialEq)]
enum Bound {
    NegInf,
    Finite(i64),
    PosInf,
}

impl Bound {
    fn add(&self, other: &Bound) -> Bound {
        match (self, other) {
            (Bound::Finite(a), Bound::Finite(b)) => {
                match a.checked_add(*b) {
                    Some(r) => Bound::Finite(r),
                    None => if *a > 0 { Bound::PosInf } else { Bound::NegInf },
                }
            }
            (Bound::PosInf, Bound::NegInf) | (Bound::NegInf, Bound::PosInf) => Bound::NegInf,
            (Bound::PosInf, _) | (_, Bound::PosInf) => Bound::PosInf,
            (Bound::NegInf, _) | (_, Bound::NegInf) => Bound::NegInf,
        }
    }

    fn sub(&self, other: &Bound) -> Bound {
        match (self, other) {
            (Bound::Finite(a), Bound::Finite(b)) => {
                match a.checked_sub(*b) {
                    Some(r) => Bound::Finite(r),
                    None => if *a > 0 { Bound::PosInf } else { Bound::NegInf },
                }
            }
            (Bound::PosInf, Bound::PosInf) | (Bound::NegInf, Bound::NegInf) => Bound::NegInf,
            (Bound::PosInf, _) | (_, Bound::NegInf) => Bound::PosInf,
            (Bound::NegInf, _) | (_, Bound::PosInf) => Bound::NegInf,
        }
    }

    fn cmp_bound(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Bound::NegInf, Bound::NegInf) => std::cmp::Ordering::Equal,
            (Bound::NegInf, _) => std::cmp::Ordering::Less,
            (_, Bound::NegInf) => std::cmp::Ordering::Greater,
            (Bound::PosInf, Bound::PosInf) => std::cmp::Ordering::Equal,
            (Bound::PosInf, _) => std::cmp::Ordering::Greater,
            (_, Bound::PosInf) => std::cmp::Ordering::Less,
            (Bound::Finite(a), Bound::Finite(b)) => a.cmp(b),
        }
    }

    fn min_bound(a: &Bound, b: &Bound) -> Bound {
        if a.cmp_bound(b) == std::cmp::Ordering::Less { a.clone() } else { b.clone() }
    }

    fn max_bound(a: &Bound, b: &Bound) -> Bound {
        if a.cmp_bound(b) == std::cmp::Ordering::Greater { a.clone() } else { b.clone() }
    }
}

#[derive(Clone, Debug)]
struct Interval {
    lo: Bound,
    hi: Bound,
}

impl Interval {
    fn exact(n: i64) -> Self {
        Interval { lo: Bound::Finite(n), hi: Bound::Finite(n) }
    }

    fn top() -> Self {
        Interval { lo: Bound::NegInf, hi: Bound::PosInf }
    }

    fn non_negative() -> Self {
        Interval { lo: Bound::Finite(0), hi: Bound::PosInf }
    }

    fn is_exact(&self) -> Option<i64> {
        if let (Bound::Finite(a), Bound::Finite(b)) = (&self.lo, &self.hi) {
            if a == b { return Some(*a); }
        }
        None
    }

    fn join(&self, other: &Interval) -> Interval {
        Interval {
            lo: Bound::min_bound(&self.lo, &other.lo),
            hi: Bound::max_bound(&self.hi, &other.hi),
        }
    }

    fn add(&self, other: &Interval) -> Interval {
        Interval {
            lo: self.lo.add(&other.lo),
            hi: self.hi.add(&other.hi),
        }
    }

    fn sub(&self, other: &Interval) -> Interval {
        Interval {
            lo: self.lo.sub(&other.hi),
            hi: self.hi.sub(&other.lo),
        }
    }

    fn mul(&self, other: &Interval) -> Interval {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            if let Some(r) = a.checked_mul(b) {
                return Interval::exact(r);
            }
        }
        Interval::top()
    }

    fn div(&self, other: &Interval) -> Interval {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            if b != 0 {
                return Interval::exact(a / b);
            }
        }
        Interval::top()
    }

    fn modulo(&self, other: &Interval) -> Interval {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            if b != 0 {
                return Interval::exact(a % b);
            }
        }
        Interval::top()
    }

    fn definitely_gt(&self, other: &Interval) -> Option<bool> {
        if self.lo.cmp_bound(&other.hi) == std::cmp::Ordering::Greater {
            return Some(true);
        }
        if self.hi.cmp_bound(&other.lo) != std::cmp::Ordering::Greater {
            return Some(false);
        }
        None
    }

    fn definitely_lt(&self, other: &Interval) -> Option<bool> {
        if self.hi.cmp_bound(&other.lo) == std::cmp::Ordering::Less {
            return Some(true);
        }
        if self.lo.cmp_bound(&other.hi) != std::cmp::Ordering::Less {
            return Some(false);
        }
        None
    }

    fn definitely_gteq(&self, other: &Interval) -> Option<bool> {
        if self.lo.cmp_bound(&other.hi) != std::cmp::Ordering::Less {
            return Some(true);
        }
        if self.hi.cmp_bound(&other.lo) == std::cmp::Ordering::Less {
            return Some(false);
        }
        None
    }

    fn definitely_lteq(&self, other: &Interval) -> Option<bool> {
        if self.hi.cmp_bound(&other.lo) != std::cmp::Ordering::Greater {
            return Some(true);
        }
        if self.lo.cmp_bound(&other.hi) == std::cmp::Ordering::Greater {
            return Some(false);
        }
        None
    }

    fn definitely_eq(&self, other: &Interval) -> Option<bool> {
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            return Some(a == b);
        }
        None
    }

    fn definitely_neq(&self, other: &Interval) -> Option<bool> {
        if self.hi.cmp_bound(&other.lo) == std::cmp::Ordering::Less
            || self.lo.cmp_bound(&other.hi) == std::cmp::Ordering::Greater
        {
            return Some(true);
        }
        if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
            return Some(a != b);
        }
        None
    }
}

#[derive(Clone)]
struct AbstractState {
    vars: HashMap<Symbol, Interval>,
    lengths: HashMap<Symbol, Interval>,
}

impl AbstractState {
    fn new() -> Self {
        AbstractState {
            vars: HashMap::new(),
            lengths: HashMap::new(),
        }
    }

    fn get_var(&self, sym: &Symbol) -> Interval {
        self.vars.get(sym).cloned().unwrap_or(Interval::top())
    }

    fn set_var(&mut self, sym: Symbol, range: Interval) {
        self.vars.insert(sym, range);
    }

    fn get_length(&self, sym: &Symbol) -> Interval {
        self.lengths.get(sym).cloned().unwrap_or(Interval::non_negative())
    }

    fn set_length(&mut self, sym: Symbol, range: Interval) {
        self.lengths.insert(sym, range);
    }
}

fn eval_expr(expr: &Expr, state: &AbstractState) -> Interval {
    match expr {
        Expr::Literal(Literal::Number(n)) => Interval::exact(*n),
        Expr::Literal(Literal::Boolean(_)) => Interval::top(),
        Expr::Literal(Literal::Float(_)) => Interval::top(),
        Expr::Identifier(sym) => state.get_var(sym),
        Expr::BinaryOp { op, left, right } => {
            let l = eval_expr(left, state);
            let r = eval_expr(right, state);
            match op {
                BinaryOpKind::Add => l.add(&r),
                BinaryOpKind::Subtract => l.sub(&r),
                BinaryOpKind::Multiply => l.mul(&r),
                BinaryOpKind::Divide => l.div(&r),
                BinaryOpKind::Modulo => l.modulo(&r),
                _ => Interval::top(),
            }
        }
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                state.get_length(sym)
            } else {
                Interval::non_negative()
            }
        }
        _ => Interval::top(),
    }
}

fn eval_condition(cond: &Expr, state: &AbstractState) -> Option<bool> {
    match cond {
        Expr::Literal(Literal::Boolean(b)) => Some(*b),
        Expr::BinaryOp { op, left, right } => {
            let l = eval_expr(left, state);
            let r = eval_expr(right, state);
            match op {
                BinaryOpKind::Gt => l.definitely_gt(&r),
                BinaryOpKind::Lt => l.definitely_lt(&r),
                BinaryOpKind::GtEq => l.definitely_gteq(&r),
                BinaryOpKind::LtEq => l.definitely_lteq(&r),
                BinaryOpKind::Eq => l.definitely_eq(&r),
                BinaryOpKind::NotEq => l.definitely_neq(&r),
                _ => None,
            }
        }
        Expr::Not { operand } => eval_condition(operand, state).map(|b| !b),
        _ => None,
    }
}

fn narrow_state(cond: &Expr, state: &mut AbstractState) {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::Gt, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    if let Some(new_lo) = n.checked_add(1) {
                        state.set_var(*sym, Interval {
                            lo: Bound::max_bound(&cur.lo, &Bound::Finite(new_lo)),
                            hi: cur.hi,
                        });
                    }
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    if let Some(new_hi) = n.checked_sub(1) {
                        state.set_var(*sym, Interval {
                            lo: cur.lo,
                            hi: Bound::min_bound(&cur.hi, &Bound::Finite(new_hi)),
                        });
                    }
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::GtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    state.set_var(*sym, Interval {
                        lo: Bound::max_bound(&cur.lo, &Bound::Finite(n)),
                        hi: cur.hi,
                    });
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    state.set_var(*sym, Interval {
                        lo: cur.lo,
                        hi: Bound::min_bound(&cur.hi, &Bound::Finite(n)),
                    });
                }
            }
        }
        _ => {}
    }
}

fn narrow_state_negated(cond: &Expr, state: &mut AbstractState) {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::Gt, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    state.set_var(*sym, Interval {
                        lo: cur.lo,
                        hi: Bound::min_bound(&cur.hi, &Bound::Finite(n)),
                    });
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    state.set_var(*sym, Interval {
                        lo: Bound::max_bound(&cur.lo, &Bound::Finite(n)),
                        hi: cur.hi,
                    });
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::GtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    if let Some(new_hi) = n.checked_sub(1) {
                        state.set_var(*sym, Interval {
                            lo: cur.lo,
                            hi: Bound::min_bound(&cur.hi, &Bound::Finite(new_hi)),
                        });
                    }
                }
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                let r = eval_expr(right, state);
                if let Some(n) = r.is_exact() {
                    let cur = state.get_var(sym);
                    if let Some(new_lo) = n.checked_add(1) {
                        state.set_var(*sym, Interval {
                            lo: Bound::max_bound(&cur.lo, &Bound::Finite(new_lo)),
                            hi: cur.hi,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

pub fn abstract_interp_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut state = AbstractState::new();
    interp_block(stmts, &mut state, expr_arena, stmt_arena)
}

fn interp_block<'a>(
    stmts: Vec<Stmt<'a>>,
    state: &mut AbstractState,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::Let { var, ty, value, mutable } => {
                let range = eval_expr(value, state);
                state.set_var(var, range);
                if matches!(value, Expr::New { .. }) {
                    state.set_length(var, Interval::exact(0));
                }
                result.push(Stmt::Let { var, ty, value, mutable });
            }

            Stmt::Set { target, value } => {
                let range = eval_expr(value, state);
                state.set_var(target, range);
                result.push(Stmt::Set { target, value });
            }

            Stmt::Push { value, collection } => {
                if let Expr::Identifier(sym) = collection {
                    let cur_len = state.get_length(sym);
                    state.set_length(*sym, cur_len.add(&Interval::exact(1)));
                }
                result.push(Stmt::Push { value, collection });
            }

            Stmt::If { cond, then_block, else_block } => {
                if let Some(val) = eval_condition(cond, state) {
                    let new_cond = expr_arena.alloc(Expr::Literal(Literal::Boolean(val)));
                    if val {
                        let mut then_state = state.clone();
                        narrow_state(cond, &mut then_state);
                        let new_then = interp_nested_block(then_block, &mut then_state, expr_arena, stmt_arena);
                        *state = then_state;
                        result.push(Stmt::If {
                            cond: new_cond,
                            then_block: new_then,
                            else_block: None,
                        });
                    } else {
                        if let Some(eb) = else_block {
                            let mut else_state = state.clone();
                            narrow_state_negated(cond, &mut else_state);
                            let new_else = interp_nested_block(eb, &mut else_state, expr_arena, stmt_arena);
                            *state = else_state;
                            result.push(Stmt::If {
                                cond: new_cond,
                                then_block: stmt_arena.alloc_slice(vec![]),
                                else_block: Some(new_else),
                            });
                        } else {
                            result.push(Stmt::If {
                                cond: new_cond,
                                then_block: stmt_arena.alloc_slice(vec![]),
                                else_block: None,
                            });
                        }
                    }
                } else {
                    let mut then_state = state.clone();
                    narrow_state(cond, &mut then_state);
                    let new_then = interp_nested_block(then_block, &mut then_state, expr_arena, stmt_arena);

                    let (new_else, else_state) = if let Some(eb) = else_block {
                        let mut es = state.clone();
                        narrow_state_negated(cond, &mut es);
                        let ne = interp_nested_block(eb, &mut es, expr_arena, stmt_arena);
                        (Some(ne), Some(es))
                    } else {
                        (None, None)
                    };

                    if let Some(es) = else_state {
                        join_states(state, &then_state, &es);
                    } else {
                        let orig = state.clone();
                        join_states(state, &then_state, &orig);
                    }

                    result.push(Stmt::If { cond, then_block: new_then, else_block: new_else });
                }
            }

            Stmt::While { cond, body, decreasing } => {
                let mut loop_state = state.clone();

                let loop_writes = collect_writes(body);
                let bounded_var = extract_bounded_var(cond);

                // Widen all loop-written variables (including the counter)
                // to their full possible range before analyzing the body.
                for w in &loop_writes {
                    loop_state.set_var(*w, Interval::top());
                }

                // Now narrow the loop state based on the condition.
                // For the bounded variable (counter), this gives it the range
                // from its initial value (widened to top above) narrowed by the
                // condition, resulting in [-inf, bound].
                narrow_state(cond, &mut loop_state);

                let new_body = interp_nested_block(body, &mut loop_state, expr_arena, stmt_arena);

                // After loop: condition is false (loop exited)
                narrow_state_negated(cond, state);
                // Variables written in loop body get widened to top
                for w in &loop_writes {
                    if Some(*w) != bounded_var {
                        state.set_var(*w, Interval::top());
                    }
                }

                result.push(Stmt::While { cond, body: new_body, decreasing });
            }

            Stmt::Repeat { pattern, iterable, body } => {
                let mut loop_state = state.clone();

                if let Pattern::Identifier(var) = &pattern {
                    loop_state.set_var(*var, Interval::top());
                }

                let loop_writes = collect_writes(body);
                for w in &loop_writes {
                    loop_state.set_var(*w, Interval::top());
                }

                let new_body = interp_nested_block(body, &mut loop_state, expr_arena, stmt_arena);

                for w in &loop_writes {
                    state.set_var(*w, Interval::top());
                }

                result.push(Stmt::Repeat { pattern, iterable, body: new_body });
            }

            Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
                let mut func_state = AbstractState::new();
                let new_body = interp_nested_block(body, &mut func_state, expr_arena, stmt_arena);
                result.push(Stmt::FunctionDef {
                    name, params, generics,
                    body: new_body,
                    return_type, is_native, native_path, is_exported, export_target, opt_flags,
                });
            }

            Stmt::Inspect { target, arms, has_otherwise } => {
                let new_arms: Vec<_> = arms.into_iter().map(|arm| {
                    let mut arm_state = state.clone();
                    let new_body = interp_nested_block(arm.body, &mut arm_state, expr_arena, stmt_arena);
                    crate::ast::stmt::MatchArm {
                        enum_name: arm.enum_name,
                        variant: arm.variant,
                        bindings: arm.bindings,
                        body: new_body,
                    }
                }).collect();
                result.push(Stmt::Inspect { target, arms: new_arms, has_otherwise });
            }

            Stmt::Zone { .. } => {
                // Don't analyze inside zones — zone-scoped bindings must be
                // preserved for escape analysis (same as propagation pass).
                result.push(stmt);
            }

            Stmt::Concurrent { tasks } => {
                let mut sub_state = state.clone();
                let new_tasks = interp_nested_block(tasks, &mut sub_state, expr_arena, stmt_arena);
                result.push(Stmt::Concurrent { tasks: new_tasks });
            }

            Stmt::Parallel { tasks } => {
                let mut sub_state = state.clone();
                let new_tasks = interp_nested_block(tasks, &mut sub_state, expr_arena, stmt_arena);
                result.push(Stmt::Parallel { tasks: new_tasks });
            }

            other => {
                result.push(other);
            }
        }
    }

    result
}

fn interp_nested_block<'a>(
    block: &'a [Stmt<'a>],
    state: &mut AbstractState,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> &'a [Stmt<'a>] {
    let stmts: Vec<Stmt<'a>> = block.iter().cloned().collect();
    let result = interp_block(stmts, state, expr_arena, stmt_arena);
    stmt_arena.alloc_slice(result)
}

fn join_states(out: &mut AbstractState, a: &AbstractState, b: &AbstractState) {
    let mut all_keys: std::collections::HashSet<Symbol> = a.vars.keys().cloned().collect();
    all_keys.extend(b.vars.keys().cloned());

    for key in all_keys {
        let a_range = a.vars.get(&key).cloned().unwrap_or(Interval::top());
        let b_range = b.vars.get(&key).cloned().unwrap_or(Interval::top());
        out.set_var(key, a_range.join(&b_range));
    }

    let mut len_keys: std::collections::HashSet<Symbol> = a.lengths.keys().cloned().collect();
    len_keys.extend(b.lengths.keys().cloned());

    for key in len_keys {
        let a_len = a.lengths.get(&key).cloned().unwrap_or(Interval::non_negative());
        let b_len = b.lengths.get(&key).cloned().unwrap_or(Interval::non_negative());
        out.set_length(key, a_len.join(&b_len));
    }
}

fn collect_writes(block: &[Stmt]) -> Vec<Symbol> {
    let mut writes = Vec::new();
    for stmt in block {
        collect_writes_stmt(stmt, &mut writes);
    }
    writes
}

fn collect_writes_stmt(stmt: &Stmt, writes: &mut Vec<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => {
            if !writes.contains(target) {
                writes.push(*target);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block { collect_writes_stmt(s, writes); }
            if let Some(eb) = else_block {
                for s in *eb { collect_writes_stmt(s, writes); }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            for s in *body { collect_writes_stmt(s, writes); }
        }
        _ => {}
    }
}

fn extract_bounded_var(cond: &Expr) -> Option<Symbol> {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::Lt | BinaryOpKind::LtEq | BinaryOpKind::Gt | BinaryOpKind::GtEq, left, .. } => {
            if let Expr::Identifier(sym) = left {
                Some(*sym)
            } else {
                None
            }
        }
        _ => None,
    }
}
