//! Popcount-leaf recognizer for the AOT path.
//!
//! A recursive bitmask counting search whose base case returns a constant has a
//! degenerate second-to-last level: when the recursion counter `k` is one step
//! from its limit, every child immediately hits the base case and returns the
//! constant, so the whole loop reduces to `CONST * popcount(x)` over the set
//! `x` it iterates. N-queens spends most of its time exactly there (`row==n-1`),
//! and collapsing that level to a single `count_ones` is a win gcc/LLVM don't
//! get (they can't see the constant-return depth relationship).
//!
//! This pass is a pure STRUCTURAL rewrite — no proof kernel. It fires on the
//! shape (fail-closed on any deviation):
//! ```text
//! To f(…, k, …, LIMIT) -> Int:
//!     If k == LIMIT { Return CONST }              # constant base case, FIRST stmt
//!     …                                           # pure setup
//!     Let mutable x be <set>                      # the value iterated
//!     Let mutable acc be 0
//!     While x is not 0:
//!         Let bit be x and (0 - x)                # lowest set bit
//!         Set x to x xor bit                      # clear it
//!         Set acc to acc + f(…, k + 1, …)         # sole accumulation, steps k→k+1
//!     Return acc.
//! ```
//! and splices, right after `x`'s defining `Let`:
//! ```text
//!     If k == LIMIT - 1 { Return CONST * count_ones(x) }
//! ```
//! Soundness is structural: at `k == LIMIT-1` the call `f(…, k+1=LIMIT, …)` hits
//! the base case immediately and returns `CONST` regardless of its other
//! arguments; the loop runs exactly `popcount(x)` times; `acc` starts at 0; so
//! the function returns `CONST * popcount(x)`. We never reason about the other
//! state parameters. Commuted operands (`x & (0-x)` vs `(0-x) & x`, etc.) are
//! accepted to survive egraph canonicalization. Runs BEFORE `inline_recursive`
//! so the fast path is carried into every unrolled clone; `LOGOS_POPCOUNT_LEAF=0`
//! disables it.

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use crate::optimization::{Opt, OptimizationConfig};

fn as_ident(e: &Expr) -> Option<Symbol> {
    match e {
        Expr::Identifier(s) => Some(*s),
        _ => None,
    }
}

fn as_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    }
}

/// `0 - x`
fn is_neg_of(e: &Expr, x: Symbol) -> bool {
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right }
        if as_int(left) == Some(0) && as_ident(right) == Some(x))
}

/// `x & (0 - x)` (either operand order)
fn is_lowest_bit(e: &Expr, x: Symbol) -> bool {
    if let Expr::BinaryOp {
        op: BinaryOpKind::BitAnd,
        left,
        right,
    } = e
    {
        (as_ident(left) == Some(x) && is_neg_of(right, x))
            || (as_ident(right) == Some(x) && is_neg_of(left, x))
    } else {
        false
    }
}

/// `x ^ bit` (either order)
fn is_xor_of(e: &Expr, x: Symbol, bit: Symbol) -> bool {
    if let Expr::BinaryOp {
        op: BinaryOpKind::BitXor,
        left,
        right,
    } = e
    {
        (as_ident(left) == Some(x) && as_ident(right) == Some(bit))
            || (as_ident(left) == Some(bit) && as_ident(right) == Some(x))
    } else {
        false
    }
}

/// `k + 1` (either order)
fn is_k_plus_one(e: &Expr, k: Symbol) -> bool {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = e
    {
        (as_ident(left) == Some(k) && as_int(right) == Some(1))
            || (as_int(left) == Some(1) && as_ident(right) == Some(k))
    } else {
        false
    }
}

/// A self-call to `fname` that steps the counter `k` to `k + 1`.
fn is_self_step_call(e: &Expr, fname: Symbol, k: Symbol) -> bool {
    matches!(e, Expr::Call { function, args }
        if *function == fname && args.iter().any(|a| is_k_plus_one(a, k)))
}

/// `If k == LIMIT { Return CONST }` (else-less) as the first statement →
/// `(k, LIMIT, CONST)`.
fn match_base_case(s: &Stmt) -> Option<(Symbol, Symbol, i64)> {
    if let Stmt::If {
        cond,
        then_block,
        else_block: None,
    } = s
    {
        if let Expr::BinaryOp {
            op: BinaryOpKind::Eq,
            left,
            right,
        } = cond
        {
            let k = as_ident(left)?;
            let limit = as_ident(right)?;
            if then_block.len() == 1 {
                if let Stmt::Return { value: Some(v) } = &then_block[0] {
                    return Some((k, limit, as_int(v)?));
                }
            }
        }
    }
    None
}

/// `While x is not 0` → `x`.
fn match_loop_x(cond: &Expr) -> Option<Symbol> {
    if let Expr::BinaryOp {
        op: BinaryOpKind::NotEq,
        left,
        right,
    } = cond
    {
        if as_int(right) == Some(0) {
            return as_ident(left);
        }
    }
    None
}

/// The loop body is exactly `[Let bit = x&(0-x)][Set x = x^bit][Set acc = acc + f(…k+1…)]`
/// → the accumulator symbol.
fn match_loop_body(body: Block, x: Symbol, fname: Symbol, k: Symbol) -> Option<Symbol> {
    if body.len() != 3 {
        return None;
    }
    let bit = match &body[0] {
        Stmt::Let { var, value, .. } if is_lowest_bit(value, x) => *var,
        _ => return None,
    };
    match &body[1] {
        Stmt::Set { target, value } if *target == x && is_xor_of(value, x, bit) => {}
        _ => return None,
    }
    match &body[2] {
        Stmt::Set { target, value } => {
            let acc = *target;
            if let Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } = value
            {
                let acc_left = as_ident(left) == Some(acc) && is_self_step_call(right, fname, k);
                let acc_right = as_ident(right) == Some(acc) && is_self_step_call(left, fname, k);
                if acc_left || acc_right {
                    return Some(acc);
                }
            }
            None
        }
        _ => None,
    }
}

fn block_has_count_ones(block: Block, count_ones: Symbol) -> bool {
    fn in_expr(e: &Expr, c: Symbol) -> bool {
        match e {
            Expr::Call { function, args } => *function == c || args.iter().any(|a| in_expr(a, c)),
            Expr::BinaryOp { left, right, .. } => in_expr(left, c) || in_expr(right, c),
            Expr::Not { operand } => in_expr(operand, c),
            Expr::Index { collection, index } => in_expr(collection, c) || in_expr(index, c),
            Expr::Length { collection } => in_expr(collection, c),
            _ => false,
        }
    }
    block.iter().any(|s| match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, count_ones),
        Stmt::Return { value } => value.map_or(false, |e| in_expr(e, count_ones)),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            in_expr(cond, count_ones)
                || block_has_count_ones(then_block, count_ones)
                || else_block.map_or(false, |b| block_has_count_ones(b, count_ones))
        }
        Stmt::While { cond, body, .. } => {
            in_expr(cond, count_ones) || block_has_count_ones(body, count_ones)
        }
        _ => false,
    })
}

/// Build `If k == LIMIT - 1 { Return CONST * count_ones(x) }`.
fn build_fast_path<'a>(
    k: Symbol,
    limit: Symbol,
    cnst: i64,
    x: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    count_ones: Symbol,
) -> Stmt<'a> {
    let cond = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Eq,
        left: ea.alloc(Expr::Identifier(k)),
        right: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left: ea.alloc(Expr::Identifier(limit)),
            right: ea.alloc(Expr::Literal(Literal::Number(1))),
        }),
    });
    let popcount = ea.alloc(Expr::Call {
        function: count_ones,
        args: vec![ea.alloc(Expr::Identifier(x))],
    });
    // CONST == 1 collapses to just the popcount; fold would do this anyway.
    let ret_val: &Expr = if cnst == 1 {
        popcount
    } else {
        ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left: ea.alloc(Expr::Literal(Literal::Number(cnst))),
            right: popcount,
        })
    };
    Stmt::If {
        cond,
        then_block: sa.alloc_slice(vec![Stmt::Return {
            value: Some(ret_val),
        }]),
        else_block: None,
    }
}

/// If `body` matches the popcount-leaf shape, return the body with the fast path
/// spliced in after `x`'s defining `Let`; otherwise `None`.
fn try_rewrite_body<'a>(
    fname: Symbol,
    body: Block<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    count_ones: Symbol,
) -> Option<Vec<Stmt<'a>>> {
    if body.len() < 4 {
        return None;
    }
    // Idempotence: never fire twice.
    if block_has_count_ones(body, count_ones) {
        return None;
    }
    let (k, limit, cnst) = match_base_case(&body[0])?;

    // Find the bit-iteration loop and its accumulator.
    let mut found = None;
    for (i, s) in body.iter().enumerate() {
        if let Stmt::While { cond, body: lb, .. } = s {
            if let Some(x) = match_loop_x(cond) {
                if let Some(acc) = match_loop_body(lb, x, fname, k) {
                    found = Some((i, x, acc));
                    break;
                }
            }
        }
    }
    let (while_idx, x, acc) = found?;

    // `acc` initialised to 0 before the loop, and the function ends in `Return acc`.
    let acc_init_ok = body[..while_idx].iter().any(
        |s| matches!(s, Stmt::Let { var, value, .. } if *var == acc && as_int(value) == Some(0)),
    );
    if !acc_init_ok {
        return None;
    }
    match body.last()? {
        Stmt::Return { value: Some(v) } if as_ident(v) == Some(acc) => {}
        _ => return None,
    }

    // `x`'s defining `Let` (the value it iterates) — splice the guard right after.
    let x_let_idx = body[..while_idx]
        .iter()
        .rposition(|s| matches!(s, Stmt::Let { var, .. } if *var == x))?;

    let fast = build_fast_path(k, limit, cnst, x, ea, sa, count_ones);
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(body.len() + 1);
    out.extend(body[..=x_let_idx].iter().cloned());
    out.push(fast);
    out.extend(body[x_let_idx + 1..].iter().cloned());
    Some(out)
}

// ---------------------------------------------------------------------------
// General popcount-loop recognizer.
//
// The stdlib (and hand-written code) spell a population count as the classic
// shift-and-mask loop:
// ```text
//     While v is greater than 0:
//         Set total to total + (v & 1).
//         Set v to v shifted right by 1.
// ```
// which is O(bit-width) where the hardware does it in one `POPCNT`. This
// recognizer replaces that loop, wherever it appears, with the guarded
// intrinsic:
// ```text
//     If v is greater than 0:
//         Set total to total + count_ones(v).
//         Set v to 0.
// ```
// The rewrite is behavior-identical for EVERY `v`: for `v > 0` the loop counts
// exactly `count_ones(v)` set bits and leaves `v = 0`; for `v <= 0` neither the
// loop nor the guard runs, so `total` and `v` are unchanged. Only fires under
// `Opt::Popcount` (AOT path) and only where the accumulator (`total`) and the
// scanned value (`v`) are distinct locals — a pure structural match, no proof
// kernel. `count_ones` lowers to `POPCNT` on AOT-Rust, `i64.popcnt` on WASM, and
// `__builtin_popcountll` on C; the interpreter/VM keep the loop, all producing
// the identical count.
// ---------------------------------------------------------------------------

/// `v > 0` → `v`.
fn match_gt_zero(cond: &Expr) -> Option<Symbol> {
    match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::Gt,
            left,
            right,
        } if as_int(right) == Some(0) => as_ident(left),
        _ => None,
    }
}

/// `v & 1` (either operand order).
fn is_low_bit_mask(e: &Expr, v: Symbol) -> bool {
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::BitAnd, left, right }
        if (as_ident(left) == Some(v) && as_int(right) == Some(1))
            || (as_int(left) == Some(1) && as_ident(right) == Some(v)))
}

/// `total + (v & 1)` (either operand order) accumulating into `total`.
fn is_low_bit_accum(e: &Expr, total: Symbol, v: Symbol) -> bool {
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
        if (as_ident(left) == Some(total) && is_low_bit_mask(right, v))
            || (as_ident(right) == Some(total) && is_low_bit_mask(left, v)))
}

/// `v shifted right by 1`.
fn is_shr_one(e: &Expr, v: Symbol) -> bool {
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::Shr, left, right }
        if as_ident(left) == Some(v) && as_int(right) == Some(1))
}

/// `While v > 0 { Set total = total + (v & 1); Set v = v >> 1 }` → `(v, total)`.
fn match_general_popcount(cond: &Expr, body: Block) -> Option<(Symbol, Symbol)> {
    let v = match_gt_zero(cond)?;
    if body.len() != 2 {
        return None;
    }
    let total = match &body[0] {
        Stmt::Set { target, value } if is_low_bit_accum(value, *target, v) => *target,
        _ => return None,
    };
    if total == v {
        return None;
    }
    match &body[1] {
        Stmt::Set { target, value } if *target == v && is_shr_one(value, v) => {}
        _ => return None,
    }
    Some((v, total))
}

/// `If v > 0 { Set total = total + count_ones(v); Set v = 0 }`.
fn build_general_fast<'a>(
    v: Symbol,
    total: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    count_ones: Symbol,
    v_live: bool,
) -> Stmt<'a> {
    let cond = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Gt,
        left: ea.alloc(Expr::Identifier(v)),
        right: ea.alloc(Expr::Literal(Literal::Number(0))),
    });
    let popc = ea.alloc(Expr::Call {
        function: count_ones,
        args: vec![ea.alloc(Expr::Identifier(v))],
    });
    let add = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: ea.alloc(Expr::Identifier(total)),
        right: popc,
    });
    let mut body = vec![Stmt::Set {
        target: total,
        value: add,
    }];
    if v_live {
        // The loop leaves v = 0; only store it back if something reads it.
        body.push(Stmt::Set {
            target: v,
            value: ea.alloc(Expr::Literal(Literal::Number(0))),
        });
    }
    Stmt::If {
        cond,
        then_block: sa.alloc_slice(body),
        else_block: None,
    }
}

/// Does `v` appear (read or written) anywhere in `e`? Conservative: an unknown
/// expression shape is assumed to mention `v` (so a live-range is never missed).
fn expr_mentions(e: &Expr, v: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == v,
        Expr::Literal(_) => false,
        Expr::BinaryOp { left, right, .. } => expr_mentions(left, v) || expr_mentions(right, v),
        Expr::Not { operand } => expr_mentions(operand, v),
        Expr::Call { args, .. } => args.iter().any(|a| expr_mentions(a, v)),
        Expr::Index { collection, index } => {
            expr_mentions(collection, v) || expr_mentions(index, v)
        }
        Expr::Length { collection } => expr_mentions(collection, v),
        _ => true,
    }
}

/// Does `v` appear anywhere in `s`? Conservative on unknown statement shapes.
fn stmt_mentions(s: &Stmt, v: Symbol) -> bool {
    match s {
        Stmt::Let { var, value, .. } => *var == v || expr_mentions(value, v),
        Stmt::Set { target, value } => *target == v || expr_mentions(value, v),
        Stmt::Return { value } => value.map_or(false, |e| expr_mentions(e, v)),
        Stmt::Show { object, .. } => expr_mentions(object, v),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_mentions(cond, v)
                || then_block.iter().any(|s| stmt_mentions(s, v))
                || else_block.map_or(false, |b| b.iter().any(|s| stmt_mentions(s, v)))
        }
        Stmt::While { cond, body, .. } => {
            expr_mentions(cond, v) || body.iter().any(|s| stmt_mentions(s, v))
        }
        _ => true,
    }
}

/// Is `v` live in the statements following the collapsed loop? If not, the loop's
/// final write to `v` is dead and can be dropped (letting the collapse match C's
/// flat loop, which never stores back to the scanned value).
fn v_live_after(rest: &[Stmt], v: Symbol) -> bool {
    rest.iter().any(|s| stmt_mentions(s, v))
}

/// `total + 1` accumulating into `total`.
fn is_inc_accum(e: &Expr, total: Symbol) -> bool {
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
        if (as_ident(left) == Some(total) && as_int(right) == Some(1))
            || (as_int(left) == Some(1) && as_ident(right) == Some(total)))
}

/// `While v > 0 { Set total = total + 1; Set v = v >> 1 }` → `(v, total)`. The
/// bit-length idiom: for `v > 0` the loop runs `bitLength(v)` = `64 -
/// leading_zeros(v)` times; for `v <= 0` it never runs.
fn match_bitlength(cond: &Expr, body: Block) -> Option<(Symbol, Symbol)> {
    let v = match_gt_zero(cond)?;
    if body.len() != 2 {
        return None;
    }
    let total = match &body[0] {
        Stmt::Set { target, value } if is_inc_accum(value, *target) => *target,
        _ => return None,
    };
    if total == v {
        return None;
    }
    match &body[1] {
        Stmt::Set { target, value } if *target == v && is_shr_one(value, v) => {}
        _ => return None,
    }
    Some((v, total))
}

/// `(v & 1) == 0` → `v`.
fn match_low_bit_zero(cond: &Expr) -> Option<Symbol> {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Eq,
        left,
        right,
    } = cond
    {
        if as_int(right) == Some(0) {
            if let Expr::BinaryOp {
                op: BinaryOpKind::BitAnd,
                left: l,
                right: r,
            } = left
            {
                if as_int(r) == Some(1) {
                    return as_ident(l);
                }
                if as_int(l) == Some(1) {
                    return as_ident(r);
                }
            }
        }
    }
    None
}

/// `While (v & 1) == 0 { Set total = total + 1; Set v = v >> 1 }` → `(v, total)`.
/// The trailing-zeros idiom: for `v != 0` the loop runs `trailing_zeros(v)` times.
fn match_trailing_zeros(cond: &Expr, body: Block) -> Option<(Symbol, Symbol)> {
    let v = match_low_bit_zero(cond)?;
    if body.len() != 2 {
        return None;
    }
    let total = match &body[0] {
        Stmt::Set { target, value } if is_inc_accum(value, *target) => *target,
        _ => return None,
    };
    if total == v {
        return None;
    }
    match &body[1] {
        Stmt::Set { target, value } if *target == v && is_shr_one(value, v) => {}
        _ => return None,
    }
    Some((v, total))
}

/// `If v > 0 { Set total = total + (64 - leading_zeros(v)); Set v = 0 }`.
fn build_bitlength_fast<'a>(
    v: Symbol,
    total: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    leading_zeros: Symbol,
    v_live: bool,
) -> Stmt<'a> {
    let cond = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Gt,
        left: ea.alloc(Expr::Identifier(v)),
        right: ea.alloc(Expr::Literal(Literal::Number(0))),
    });
    let lz = ea.alloc(Expr::Call {
        function: leading_zeros,
        args: vec![ea.alloc(Expr::Identifier(v))],
    });
    let bitlen = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Subtract,
        left: ea.alloc(Expr::Literal(Literal::Number(64))),
        right: lz,
    });
    let add = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: ea.alloc(Expr::Identifier(total)),
        right: bitlen,
    });
    let mut body = vec![Stmt::Set {
        target: total,
        value: add,
    }];
    if v_live {
        body.push(Stmt::Set {
            target: v,
            value: ea.alloc(Expr::Literal(Literal::Number(0))),
        });
    }
    Stmt::If {
        cond,
        then_block: sa.alloc_slice(body),
        else_block: None,
    }
}

/// `If v != 0 { Set total = total + trailing_zeros(v); Set v = v >> trailing_zeros(v) }`.
fn build_trailing_fast<'a>(
    v: Symbol,
    total: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    trailing_zeros: Symbol,
    v_live: bool,
) -> Stmt<'a> {
    let cond = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::NotEq,
        left: ea.alloc(Expr::Identifier(v)),
        right: ea.alloc(Expr::Literal(Literal::Number(0))),
    });
    let tz_total = ea.alloc(Expr::Call {
        function: trailing_zeros,
        args: vec![ea.alloc(Expr::Identifier(v))],
    });
    let add = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: ea.alloc(Expr::Identifier(total)),
        right: tz_total,
    });
    let mut body = vec![Stmt::Set {
        target: total,
        value: add,
    }];
    if v_live {
        // The loop leaves v = v >> trailing_zeros(v); only store it if read.
        let tz_shift = ea.alloc(Expr::Call {
            function: trailing_zeros,
            args: vec![ea.alloc(Expr::Identifier(v))],
        });
        let shr = ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Shr,
            left: ea.alloc(Expr::Identifier(v)),
            right: tz_shift,
        });
        body.push(Stmt::Set {
            target: v,
            value: shr,
        });
    }
    Stmt::If {
        cond,
        then_block: sa.alloc_slice(body),
        else_block: None,
    }
}

/// The interned symbols for the three bit-count intrinsics inserted by this pass.
#[derive(Clone, Copy)]
struct BitIntrinsics {
    count_ones: Symbol,
    leading_zeros: Symbol,
    trailing_zeros: Symbol,
}

/// Recursively replace general popcount / bit-length / trailing-zeros loops with
/// the matching hardware intrinsic, descending into function bodies (honoring each
/// function's `## No Popcount`), `If` arms and nested loops.
fn rewrite_general<'a>(
    stmts: Block<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    ops: BitIntrinsics,
) -> Vec<Stmt<'a>> {
    stmts
        .iter()
        .enumerate()
        .map(|(i, s)| match s {
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                let rest = &stmts[i + 1..];
                if let Some((v, total)) = match_general_popcount(cond, body) {
                    build_general_fast(v, total, ea, sa, ops.count_ones, v_live_after(rest, v))
                } else if let Some((v, total)) = match_bitlength(cond, body) {
                    build_bitlength_fast(v, total, ea, sa, ops.leading_zeros, v_live_after(rest, v))
                } else if let Some((v, total)) = match_trailing_zeros(cond, body) {
                    build_trailing_fast(v, total, ea, sa, ops.trailing_zeros, v_live_after(rest, v))
                } else {
                    Stmt::While {
                        cond,
                        body: sa.alloc_slice(rewrite_general(body, ea, sa, ops)),
                        decreasing: *decreasing,
                    }
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => Stmt::If {
                cond,
                then_block: sa.alloc_slice(rewrite_general(then_block, ea, sa, ops)),
                else_block: else_block.map(|b| &*sa.alloc_slice(rewrite_general(b, ea, sa, ops))),
            },
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            } if opt_flags.is_on(Opt::Popcount) => Stmt::FunctionDef {
                name: *name,
                generics: generics.clone(),
                params: params.clone(),
                body: sa.alloc_slice(rewrite_general(body, ea, sa, ops)),
                return_type: *return_type,
                is_native: *is_native,
                native_path: *native_path,
                is_exported: *is_exported,
                export_target: *export_target,
                opt_flags: opt_flags.clone(),
            },
            other => other.clone(),
        })
        .collect()
}

/// Insert a popcount fast path into every eligible bitmask counting search.
pub fn popcount_leaf_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    cfg: &OptimizationConfig,
) -> Vec<Stmt<'a>> {
    if !cfg.is_on(Opt::Popcount) {
        return stmts;
    }
    let count_ones = interner.intern("count_ones");
    let sat: Vec<Stmt<'a>> = stmts
        .into_iter()
        .map(|s| match &s {
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body,
                return_type,
                is_native: false,
                native_path,
                is_exported: false,
                export_target,
                opt_flags,
            } if generics.is_empty() && opt_flags.is_on(crate::optimization::Opt::Popcount) => {
                match try_rewrite_body(*name, body, expr_arena, stmt_arena, count_ones) {
                    Some(new_body) => Stmt::FunctionDef {
                        name: *name,
                        generics: generics.clone(),
                        params: params.clone(),
                        body: stmt_arena.alloc_slice(new_body),
                        return_type: *return_type,
                        is_native: false,
                        native_path: *native_path,
                        is_exported: false,
                        export_target: *export_target,
                        opt_flags: opt_flags.clone(),
                    },
                    None => s,
                }
            }
            _ => s,
        })
        .collect();
    // Then collapse every general shift-and-mask bit loop (stdlib `bits.lg` and
    // hand-written) to the matching hardware intrinsic, program-wide: popcount →
    // count_ones, bit-length → leading_zeros, trailing-zeros → trailing_zeros.
    let ops = BitIntrinsics {
        count_ones,
        leading_zeros: interner.intern("leading_zeros"),
        trailing_zeros: interner.intern("trailing_zeros"),
    };
    rewrite_general(stmt_arena.alloc_slice(sat), expr_arena, stmt_arena, ops)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/popcount_leaf/tests.rs"]
mod tests;
