//! Reflection symmetry-breaking for bitmask counting searches (AOT path).
//!
//! An N-Queens count enumerates first-row queen placements and sums the
//! sub-counts. The board has a left-right reflection symmetry: placing the
//! first queen in column `c` yields the same sub-count as column `n-1-c`. So
//! the total is `2 · (sum over columns [0, n/2)) + (n odd ? middle column : 0)`
//! — roughly HALF the work. gcc/LLVM can't do this (it's an algorithmic
//! symmetry, not a peephole); LOGOS can, because the reflection invariance is
//! a kernel-PROVED theorem (`logicaffeine_kernel::bitvector`), not a guess.
//!
//! Soundness rests on three facts the kernel proves for ALL `n`
//! (`bitvector::reflection_certificate`): mirroring the state mirrors the
//! available set (L1), and a per-move bitmask step commutes with the mirror for
//! both diagonal directions (LEM4/LEM5). With those, the search trees rooted at
//! columns `c` and `n-1-c` are in bijection (column choice `b ↦ rev_n(b)`), so
//! their leaf-count sums are equal — the reflection invariance. This pass fires
//! ONLY when the kernel certificate holds AND the recognized search has the
//! exact reflection structure (one column mask + a conjugate `<<1`/`>>1`
//! diagonal pair); otherwise it emits nothing (fail-closed). No runtime guard:
//! the proof is unbounded.

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

/// Kernel-LIA proof that the left-right reflection `c ↦ m-c` (m = n-1) PRESERVES
/// the N-Queens attack relation — the geometric half of the closure, proven for
/// all n by Fourier-Motzkin over symbolic `(r1,c1,r2,c2,m)`. Two queens attack
/// iff they share a column (`c1=c2`), a "/" diagonal (`r1-c1=r2-c2`), or a "\"
/// diagonal (`r1+c1=r2+c2`). Reflection fixes the column relation and SWAPS the
/// two diagonals, so the attack relation (their disjunction) is invariant. Each
/// claim is a linear-expression identity (the two difference expressions are
/// equal up to sign), discharged through `logicaffeine_kernel::lia`. Together
/// with the bit-level commutation (`bitvector::reflection_symmetry_proven`),
/// this is the kernel-checked, unbounded closure: reflection is a
/// count-preserving bijection on the search, so `subcount(c) = subcount(n-1-c)`.
fn reflection_preserves_attacks() -> bool {
    use logicaffeine_kernel::lia::{fourier_motzkin_unsat, Constraint, LinearExpr, Rational};
    let v = LinearExpr::var;
    let (r1, c1, r2, c2, m) = (v(1), v(2), v(3), v(4), v(5));
    // `e >= 0` is valid iff `{ e + 1 <= 0 }` is unsatisfiable.
    let valid_ge0 = |e: &LinearExpr| {
        fourier_motzkin_unsat(&[Constraint {
            expr: e.add(&LinearExpr::constant(Rational::from_int(1))),
            strict: false,
        }])
    };
    // `a == b` identically iff both `a-b >= 0` and `b-a >= 0` are valid.
    let eq = |a: &LinearExpr, b: &LinearExpr| {
        let d = a.sub(b);
        valid_ge0(&d) && valid_ge0(&d.neg())
    };
    // reflected column-difference == negation of the original column-difference
    let refl_col = m.sub(&c1).sub(&m.sub(&c2)); // (m-c1)-(m-c2)
    let orig_col_neg = c2.sub(&c1); // -(c1-c2)
    // reflected "/" difference == original "\" difference
    let refl_fwd = r1.sub(&m.sub(&c1)).sub(&r2.sub(&m.sub(&c2)));
    let orig_bwd = r1.add(&c1).sub(&r2.add(&c2));
    // reflected "\" difference == original "/" difference
    let refl_bwd = r1.add(&m.sub(&c1)).sub(&r2.add(&m.sub(&c2)));
    let orig_fwd = r1.sub(&c1).sub(&r2.sub(&c2));
    eq(&refl_col, &orig_col_neg) && eq(&refl_fwd, &orig_bwd) && eq(&refl_bwd, &orig_fwd)
}

/// The full closure discharge (memoised; constant): both kernel facts hold —
/// the bit-level commutation (B1) and the geometric attack-preservation (the
/// LIA proof above). Unbounded, machine-checked.
fn reflection_closure_proven() -> bool {
    use std::sync::OnceLock;
    static PROVEN: OnceLock<bool> = OnceLock::new();
    *PROVEN.get_or_init(|| {
        logicaffeine_kernel::bitvector::reflection_symmetry_proven() && reflection_preserves_attacks()
    })
}

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

fn is_binop<'a>(e: &'a Expr<'a>, op: BinaryOpKind) -> Option<(&'a Expr<'a>, &'a Expr<'a>)> {
    match e {
        Expr::BinaryOp { op: o, left, right } if *o == op => Some((left, right)),
        _ => None,
    }
}

/// `p | bit` (either operand order) → `p`.
fn match_or_with_bit(e: &Expr, bit: Symbol) -> Option<Symbol> {
    let (l, r) = is_binop(e, BinaryOpKind::Or)?;
    if as_ident(r) == Some(bit) {
        as_ident(l)
    } else if as_ident(l) == Some(bit) {
        as_ident(r)
    } else {
        None
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum ArgKind {
    Plain,
    ShiftLeft,
    ShiftRight,
}

/// Classify a recursive-call argument as a reflection-relevant state transition
/// of some parameter, given the loop's `bit` symbol → `(param, kind)`.
fn classify_state_arg(e: &Expr, bit: Symbol) -> Option<(Symbol, ArgKind)> {
    // (p | bit) << 1
    if let Some((base, amt)) = is_binop(e, BinaryOpKind::Shl) {
        if as_int(amt) == Some(1) {
            return match_or_with_bit(base, bit).map(|p| (p, ArgKind::ShiftLeft));
        }
    }
    // (p | bit) >> 1
    if let Some((base, amt)) = is_binop(e, BinaryOpKind::Shr) {
        if as_int(amt) == Some(1) {
            return match_or_with_bit(base, bit).map(|p| (p, ArgKind::ShiftRight));
        }
    }
    // p | bit
    match_or_with_bit(e, bit).map(|p| (p, ArgKind::Plain))
}

/// `k + 1` (either order).
fn is_step(e: &Expr, k: Symbol) -> bool {
    is_binop(e, BinaryOpKind::Add).is_some_and(|(l, r)| {
        (as_ident(l) == Some(k) && as_int(r) == Some(1))
            || (as_int(l) == Some(1) && as_ident(r) == Some(k))
    })
}

/// The canonical bit-iteration loop `while x != 0 { bit = x & (0-x); x = x ^ bit; ... }`.
/// Returns `(x_sym, bit_sym, acc_sym, the accumulating Set's value expr)`.
fn match_bit_loop<'a>(
    cond: &Expr,
    body: Block<'a>,
) -> Option<(Symbol, Symbol, Symbol, &'a Expr<'a>)> {
    // cond: x != 0
    let (xl, xr) = is_binop(cond, BinaryOpKind::NotEq)?;
    let x = as_ident(xl)?;
    if as_int(xr) != Some(0) {
        return None;
    }
    if body.len() != 3 {
        return None;
    }
    // Let bit = x & (0 - x)
    let bit = match &body[0] {
        Stmt::Let { var, value, .. } => {
            let (a, b) = is_binop(value, BinaryOpKind::And)?;
            let neg = |e: &Expr| is_binop(e, BinaryOpKind::Subtract)
                .is_some_and(|(z, y)| as_int(z) == Some(0) && as_ident(y) == Some(x));
            if !((as_ident(a) == Some(x) && neg(b)) || (as_ident(b) == Some(x) && neg(a))) {
                return None;
            }
            *var
        }
        _ => return None,
    };
    // Set x = x ^ bit
    match &body[1] {
        Stmt::Set { target, value } if *target == x => {
            let (a, b) = is_binop(value, BinaryOpKind::BitXor)?;
            let ok = (as_ident(a) == Some(x) && as_ident(b) == Some(bit))
                || (as_ident(a) == Some(bit) && as_ident(b) == Some(x));
            if !ok {
                return None;
            }
        }
        _ => return None,
    }
    // Set acc = acc + <value>
    match &body[2] {
        Stmt::Set { target, value } => {
            let (a, b) = is_binop(value, BinaryOpKind::Add)?;
            if as_ident(a) == Some(*target) {
                Some((x, bit, *target, b))
            } else if as_ident(b) == Some(*target) {
                Some((x, bit, *target, a))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Confirm `g` is a reflection-symmetric recursive search: base case
/// `if row == n { return CONST }`, a bit-iteration loop summing
/// `g(row+1, …)` whose state-arg transitions are exactly one Plain, one
/// ShiftLeft, one ShiftRight (the column mask + conjugate diagonal pair).
fn is_reflection_search(g: Symbol, params: &[Symbol], body: Block) -> bool {
    if body.is_empty() {
        return false;
    }
    // base case: If row == n { Return CONST }
    let (row, n) = match &body[0] {
        Stmt::If { cond, then_block, else_block: None } => {
            let Some((l, r)) = is_binop(cond, BinaryOpKind::Eq) else { return false };
            let (Some(row), Some(n)) = (as_ident(l), as_ident(r)) else { return false };
            if then_block.len() != 1 || !matches!(&then_block[0], Stmt::Return { value: Some(v) } if as_int(v).is_some())
            {
                return false;
            }
            (row, n)
        }
        _ => return false,
    };
    // find the bit-iteration loop whose accumulated value is a self-call g(row+1, …)
    for s in body {
        if let Stmt::While { cond, body: lb, .. } = s {
            if let Some((_x, bit, _acc, val)) = match_bit_loop(cond, lb) {
                if let Expr::Call { function, args } = val {
                    if *function == g && args.len() == params.len() && is_step(args[0], row) {
                        // classify the middle args (skip arg0=row+1 and the n arg)
                        let mut plain = 0;
                        let mut left = 0;
                        let mut right = 0;
                        let mut other = 0;
                        for a in &args[1..] {
                            if as_ident(a) == Some(n) {
                                continue; // size param passed through
                            }
                            match classify_state_arg(a, bit) {
                                Some((_, ArgKind::Plain)) => plain += 1,
                                Some((_, ArgKind::ShiftLeft)) => left += 1,
                                Some((_, ArgKind::ShiftRight)) => right += 1,
                                None => other += 1,
                            }
                        }
                        return plain == 1 && left == 1 && right == 1 && other == 0;
                    }
                }
            }
        }
    }
    false
}

/// A recognized first-row enumeration entry to rewrite.
struct Entry<'a> {
    n: Symbol,
    avail_let_idx: usize,
    avail_sym: Symbol,
    avail_init: &'a Expr<'a>,
    while_idx: usize,
    bit_sym: Symbol,
    call: &'a Expr<'a>,
}

/// Recognize `E(n)` whose body enumerates the first row: a bit-iteration loop
/// summing `g(1, …, n)` for a reflection-symmetric search `g`.
fn recognize_entry<'a>(
    params: &[Symbol],
    body: Block<'a>,
    searches: &HashMap<Symbol, ()>,
) -> Option<Entry<'a>> {
    if params.len() != 1 {
        return None;
    }
    let n = params[0];
    // find the enumeration loop
    let mut while_idx = None;
    let mut found_bit = None;
    let mut found_call = None;
    let mut acc_sym = None;
    for (i, s) in body.iter().enumerate() {
        if let Stmt::While { cond, body: lb, .. } = s {
            if let Some((_x, bit, acc, val)) = match_bit_loop(cond, lb) {
                if let Expr::Call { function, args } = val {
                    // first row: row arg is literal 1; g is a known reflection search; size arg is n
                    if searches.contains_key(function)
                        && !args.is_empty()
                        && as_int(args[0]) == Some(1)
                        && args.iter().any(|a| as_ident(a) == Some(n))
                    {
                        while_idx = Some(i);
                        found_bit = Some(bit);
                        found_call = Some(val);
                        acc_sym = Some(acc);
                        break;
                    }
                }
            }
        }
    }
    let while_idx = while_idx?;
    let bit_sym = found_bit?;
    let call = found_call?;
    let acc = acc_sym?;
    // `acc` initialised to 0 before the loop, `Return acc` last.
    if !body[..while_idx]
        .iter()
        .any(|s| matches!(s, Stmt::Let { var, value, .. } if *var == acc && as_int(value) == Some(0)))
    {
        return None;
    }
    if !matches!(body.last(), Some(Stmt::Return { value: Some(v) }) if as_ident(v) == Some(acc)) {
        return None;
    }
    // the loop iterates `avail`; find its defining Let (its init is masked to the low half).
    let (avail_sym, _, _, _) = match &body[while_idx] {
        Stmt::While { cond, body: lb, .. } => match_bit_loop(cond, lb)?,
        _ => return None,
    };
    let avail_let_idx = body[..while_idx]
        .iter()
        .rposition(|s| matches!(s, Stmt::Let { var, .. } if *var == avail_sym))?;
    let avail_init = match &body[avail_let_idx] {
        Stmt::Let { value, .. } => *value,
        _ => return None,
    };
    Some(Entry { n, avail_let_idx, avail_sym, avail_init, while_idx, bit_sym, call })
}

/// Substitute identifier `from` → identifier `to` throughout an expression
/// (used to turn the loop's `g(1, …bit…)` into the middle-column `g(1, …mid…)`).
fn subst_ident<'a>(e: &'a Expr<'a>, from: Symbol, to: Symbol, ea: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
    match e {
        Expr::Identifier(s) if *s == from => ea.alloc(Expr::Identifier(to)),
        Expr::Identifier(_) | Expr::Literal(_) => e,
        Expr::BinaryOp { op, left, right } => ea.alloc(Expr::BinaryOp {
            op: *op,
            left: subst_ident(left, from, to, ea),
            right: subst_ident(right, from, to, ea),
        }),
        Expr::Not { operand } => ea.alloc(Expr::Not { operand: subst_ident(operand, from, to, ea) }),
        Expr::Call { function, args } => ea.alloc(Expr::Call {
            function: *function,
            args: args.iter().map(|a| subst_ident(a, from, to, ea)).collect(),
        }),
        _ => e,
    }
}

fn int_lit<'a>(v: i64, ea: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
    ea.alloc(Expr::Literal(Literal::Number(v)))
}

/// Rewrite the entry body to the half-enumeration: restrict the first-row loop
/// to columns `[0, n/2)`, double, and add the middle column when `n` is odd.
fn rewrite_entry<'a>(
    e: &Entry<'a>,
    body: Block<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let half = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Shr,
        left: ea.alloc(Expr::Identifier(e.n)),
        right: int_lit(1, ea),
    });
    // low_half_mask = (1 << (n >> 1)) - 1
    let low_mask = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Subtract,
        left: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Shl,
            left: int_lit(1, ea),
            right: half,
        }),
        right: int_lit(1, ea),
    });
    // available init &= low_half_mask
    let new_init = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::And,
        left: e.avail_init,
        right: low_mask,
    });

    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(body.len() + 4);
    for (i, s) in body.iter().enumerate() {
        if i == e.avail_let_idx {
            // replace the available Let's init
            if let Stmt::Let { var, ty, mutable, .. } = s {
                out.push(Stmt::Let { var: *var, ty: *ty, value: new_init, mutable: *mutable });
            } else {
                out.push(s.clone());
            }
        } else if i == e.while_idx {
            // the loop, then doubling + odd-n middle column, BEFORE the return.
            out.push(s.clone());
            // find the accumulator (the loop's acc)
            let acc = match s {
                Stmt::While { cond, body: lb, .. } => match_bit_loop(cond, lb).map(|(_, _, a, _)| a),
                _ => None,
            };
            if let Some(acc) = acc {
                // Set acc to acc * 2
                out.push(Stmt::Set {
                    target: acc,
                    value: ea.alloc(Expr::BinaryOp {
                        op: BinaryOpKind::Multiply,
                        left: ea.alloc(Expr::Identifier(acc)),
                        right: int_lit(2, ea),
                    }),
                });
                // if (n % 2) != 0 { let mid = 1 << (n>>1); acc = acc + g(1, …mid…) }
                let mid = interner.intern("__sym_mid");
                let half2 = ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Shr,
                    left: ea.alloc(Expr::Identifier(e.n)),
                    right: int_lit(1, ea),
                });
                let mid_let = Stmt::Let {
                    var: mid,
                    ty: None,
                    value: ea.alloc(Expr::BinaryOp {
                        op: BinaryOpKind::Shl,
                        left: int_lit(1, ea),
                        right: half2,
                    }),
                    mutable: false,
                };
                let mid_call = subst_ident(e.call, e.bit_sym, mid, ea);
                let add_mid = Stmt::Set {
                    target: acc,
                    value: ea.alloc(Expr::BinaryOp {
                        op: BinaryOpKind::Add,
                        left: ea.alloc(Expr::Identifier(acc)),
                        right: mid_call,
                    }),
                };
                let odd = ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::NotEq,
                    left: ea.alloc(Expr::BinaryOp {
                        op: BinaryOpKind::Modulo,
                        left: ea.alloc(Expr::Identifier(e.n)),
                        right: int_lit(2, ea),
                    }),
                    right: int_lit(0, ea),
                });
                out.push(Stmt::If {
                    cond: odd,
                    then_block: sa.alloc_slice(vec![mid_let, add_mid]),
                    else_block: None,
                });
            }
        } else {
            out.push(s.clone());
        }
    }
    let _ = e.avail_sym;
    out
}

/// Apply reflection symmetry-breaking where the kernel certificate holds and an
/// entry over a reflection-symmetric search is recognized. Fail-closed.
pub fn break_symmetry_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    // Catalogue reflection-symmetric searches.
    let mut searches: HashMap<Symbol, ()> = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef {
            name, params, body, is_native: false, generics, ..
        } = s
        {
            if generics.is_empty() {
                let psyms: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
                if is_reflection_search(*name, &psyms, body) {
                    searches.insert(*name, ());
                }
            }
        }
    }
    if searches.is_empty() {
        return stmts;
    }
    // The reflection invariance is kernel-proved for all n (bit-level
    // commutation AND geometric attack-preservation); if either certificate
    // ever failed (it cannot), fire nothing — fail-closed.
    if !reflection_closure_proven() {
        return stmts;
    }

    stmts
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
            } if generics.is_empty()
                && opt_flags.is_on(crate::optimization::Opt::Symmetry)
                && !searches.contains_key(name) =>
            {
                let psyms: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
                match recognize_entry(&psyms, body, &searches) {
                    Some(entry) => {
                        let new_body = rewrite_entry(&entry, body, expr_arena, stmt_arena, interner);
                        Stmt::FunctionDef {
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
                        }
                    }
                    None => s,
                }
            }
            _ => s,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflection_attack_preservation_is_kernel_proved() {
        assert!(
            reflection_preserves_attacks(),
            "kernel LIA failed to prove the reflection preserves the attack relation"
        );
    }

    #[test]
    fn full_closure_is_kernel_proven() {
        // bit-level commutation (B1) AND geometric attack-preservation (LIA).
        assert!(reflection_closure_proven());
    }
}
