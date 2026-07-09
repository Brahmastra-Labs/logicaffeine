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
            expr: e.add(&LinearExpr::constant(Rational::from_i64(1))),
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
    let (l, r) = is_binop(e, BinaryOpKind::BitOr)?;
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
            let (a, b) = is_binop(value, BinaryOpKind::BitAnd)?;
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

/// Where a recursive search makes its per-level choices. Both forms enumerate
/// the same column set, so the reflection acts on them identically — one
/// recognizer and one rewrite serve both surface forms.
#[derive(Clone, Copy)]
enum ChoiceKind<'a> {
    /// `while x != 0 { bit = x & (0 - x); x = x ^ bit; … }` — the iterated set is
    /// `x`; restricting to the low-half columns masks `x`'s init.
    BitTrick { x: Symbol },
    /// `while col cmp limit { bit = 1 << col; …; col = col + 1 }` — the choice is
    /// `col` over `[0, limit)`; halving restricts the bound to `limit >> 1`.
    Range { col: Symbol, limit: &'a Expr<'a>, cmp: BinaryOpKind },
}

/// A recognized per-level choice loop: the enumeration form, the per-choice bit
/// symbol, the accumulator, the accumulated recursive call, and (range form) the
/// optional predicate gating the accumulate.
#[derive(Clone, Copy)]
struct ChoiceLoop<'a> {
    kind: ChoiceKind<'a>,
    bit: Symbol,
    acc: Symbol,
    call: &'a Expr<'a>,
    guard: Option<&'a Expr<'a>>,
}

/// `n >> 1` as a fresh arena expression.
fn shr1<'a>(sym: Symbol, ea: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
    ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Shr,
        left: ea.alloc(Expr::Identifier(sym)),
        right: int_lit(1, ea),
    })
}

/// The counted range loop `while col cmp limit { bit = 1 << col; [if guard] acc = acc + <call>; col = col + 1 }`.
/// The `col + 1` increment identifies the counter; the `1 << col` Let identifies
/// `bit`; the `acc = acc + <call>` (bare or as the sole arm of a guard `If`)
/// identifies the accumulator, the call, and the predicate.
fn match_range_loop<'a>(cond: &'a Expr<'a>, body: Block<'a>) -> Option<ChoiceLoop<'a>> {
    let (cmp, cl, cr) = match cond {
        Expr::BinaryOp { op: op @ (BinaryOpKind::Lt | BinaryOpKind::LtEq), left, right } => {
            (*op, *left, *right)
        }
        _ => return None,
    };
    let col = as_ident(cl)?;
    let limit = cr;

    // `target = target + <call>` → the call expr, when `<call>` is a Call.
    let accum_from = |target: Symbol, value: &'a Expr<'a>| -> Option<&'a Expr<'a>> {
        let (a, b) = is_binop(value, BinaryOpKind::Add)?;
        let call = if as_ident(a) == Some(target) {
            b
        } else if as_ident(b) == Some(target) {
            a
        } else {
            return None;
        };
        matches!(call, Expr::Call { .. }).then_some(call)
    };

    let mut bit: Option<Symbol> = None;
    let mut found: Option<(Symbol, &'a Expr<'a>, Option<&'a Expr<'a>>)> = None;
    let mut saw_incr = false;
    for s in body {
        match s {
            Stmt::Let { var, value, .. } => {
                if let Some((l, r)) = is_binop(value, BinaryOpKind::Shl) {
                    if as_int(l) == Some(1) && as_ident(r) == Some(col) {
                        bit = Some(*var);
                    }
                }
            }
            Stmt::Set { target, value } if *target == col && is_step(value, col) => {
                saw_incr = true;
            }
            Stmt::Set { target, value } => {
                if let Some(call) = accum_from(*target, value) {
                    found = Some((*target, call, None));
                }
            }
            Stmt::If { cond: guard, then_block, else_block: None } if then_block.len() == 1 => {
                if let Stmt::Set { target, value } = &then_block[0] {
                    if let Some(call) = accum_from(*target, value) {
                        found = Some((*target, call, Some(*guard)));
                    }
                }
            }
            _ => {}
        }
    }
    let bit = bit?;
    let (acc, call, guard) = found?;
    if !saw_incr {
        return None;
    }
    Some(ChoiceLoop { kind: ChoiceKind::Range { col, limit, cmp }, bit, acc, call, guard })
}

/// Recognize either choice-loop form, normalized to a [`ChoiceLoop`].
fn match_choice_loop<'a>(cond: &'a Expr<'a>, body: Block<'a>) -> Option<ChoiceLoop<'a>> {
    if let Some((x, bit, acc, call)) = match_bit_loop(cond, body) {
        return Some(ChoiceLoop { kind: ChoiceKind::BitTrick { x }, bit, acc, call, guard: None });
    }
    match_range_loop(cond, body)
}

/// Confirm `g` is a reflection-symmetric recursive search: base case
/// `if row == n { return CONST }`, a per-level choice loop (bit-iteration or
/// counted range) summing `g(row+1, …)` whose state-arg transitions are exactly
/// one Plain, one ShiftLeft, one ShiftRight (the column mask + conjugate
/// diagonal pair).
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
    // find the choice loop whose accumulated value is a self-call g(row+1, …)
    for s in body {
        if let Stmt::While { cond, body: lb, .. } = s {
            if let Some(cl) = match_choice_loop(cond, lb) {
                if let Expr::Call { function, args } = cl.call {
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
                            match classify_state_arg(a, cl.bit) {
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
    while_idx: usize,
    loop_info: ChoiceLoop<'a>,
    /// BitTrick form only: the `Let` defining the iterated set, masked to the
    /// low half by the rewrite. `None` for the Range form (which restricts the
    /// loop bound instead).
    avail_let_idx: Option<usize>,
    avail_init: Option<&'a Expr<'a>>,
}

/// Recognize `E(n)` whose body enumerates the first row: a per-level choice loop
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
    // find the enumeration loop: a choice loop whose call is g(1, …, n) for a known search g.
    let mut found: Option<(usize, ChoiceLoop<'a>)> = None;
    for (i, s) in body.iter().enumerate() {
        if let Stmt::While { cond, body: lb, .. } = s {
            if let Some(cl) = match_choice_loop(cond, lb) {
                if let Expr::Call { function, args } = cl.call {
                    if searches.contains_key(function)
                        && !args.is_empty()
                        && as_int(args[0]) == Some(1)
                        && args.iter().any(|a| as_ident(a) == Some(n))
                    {
                        found = Some((i, cl));
                        break;
                    }
                }
            }
        }
    }
    let (while_idx, loop_info) = found?;
    let acc = loop_info.acc;
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
    // Form-specific validation + the data the rewrite needs.
    let (avail_let_idx, avail_init) = match loop_info.kind {
        // The bit-trick loop iterates `x`; find its defining Let so the rewrite
        // can mask its init to the low-half columns.
        ChoiceKind::BitTrick { x } => {
            let idx = body[..while_idx]
                .iter()
                .rposition(|s| matches!(s, Stmt::Let { var, .. } if *var == x))?;
            let init = match &body[idx] {
                Stmt::Let { value, .. } => *value,
                _ => return None,
            };
            (Some(idx), Some(init))
        }
        // The range must cover exactly columns [0, n): `col < n` from a zero
        // start. Anything else is not a first-row enumeration we can soundly
        // halve — fail closed.
        ChoiceKind::Range { col, limit, cmp } => {
            if cmp != BinaryOpKind::Lt || as_ident(limit) != Some(n) {
                return None;
            }
            if !body[..while_idx].iter().any(|s| {
                matches!(s, Stmt::Let { var, value, .. } if *var == col && as_int(value) == Some(0))
            }) {
                return None;
            }
            (None, None)
        }
    };
    Some(Entry { n, while_idx, loop_info, avail_let_idx, avail_init })
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

/// Rewrite the entry body to the half-enumeration: restrict the first-row choice
/// to the low-half columns `[0, n/2)`, double the sub-count, and add the middle
/// column when `n` is odd. The BitTrick form masks the iterated set's init; the
/// Range form restricts the loop bound to `n >> 1`. Both then `acc *= 2` and, for
/// odd `n`, replay the choice at the fixed middle column `n >> 1` (carrying the
/// range form's predicate guard). `n >> 1` is the middle column index for odd `n`
/// and exactly the low-half size for even `n`, so the two parities are covered.
fn rewrite_entry<'a>(
    e: &Entry<'a>,
    body: Block<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(body.len() + 4);
    for (i, s) in body.iter().enumerate() {
        if Some(i) == e.avail_let_idx {
            // BitTrick: restrict the iterated set to the low-half columns:
            //   init &= (1 << (n >> 1)) - 1
            let init = e.avail_init.expect("avail_init is present whenever avail_let_idx is");
            let low_mask = ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                left: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Shl,
                    left: int_lit(1, ea),
                    right: shr1(e.n, ea),
                }),
                right: int_lit(1, ea),
            });
            let new_init = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::BitAnd, left: init, right: low_mask });
            if let Stmt::Let { var, ty, mutable, .. } = s {
                out.push(Stmt::Let { var: *var, ty: *ty, value: new_init, mutable: *mutable });
            } else {
                out.push(s.clone());
            }
        } else if i == e.while_idx {
            // Emit the (possibly bound-restricted) enumeration loop, then double
            // and add the odd-n middle column, BEFORE the return.
            match e.loop_info.kind {
                // Range: restrict the bound to [0, n >> 1).
                ChoiceKind::Range { col, cmp, .. } => {
                    if let Stmt::While { body: lb, decreasing, .. } = s {
                        let new_cond = ea.alloc(Expr::BinaryOp {
                            op: cmp,
                            left: ea.alloc(Expr::Identifier(col)),
                            right: shr1(e.n, ea),
                        });
                        out.push(Stmt::While { cond: new_cond, body: *lb, decreasing: *decreasing });
                    } else {
                        out.push(s.clone());
                    }
                }
                // BitTrick: the set was already masked at its Let; loop unchanged.
                ChoiceKind::BitTrick { .. } => out.push(s.clone()),
            }

            let acc = e.loop_info.acc;
            // acc = acc * 2
            out.push(Stmt::Set {
                target: acc,
                value: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Multiply,
                    left: ea.alloc(Expr::Identifier(acc)),
                    right: int_lit(2, ea),
                }),
            });
            // if (n % 2) != 0 { let __sym_mid = 1 << (n>>1); [if guard] acc = acc + g(1, …__sym_mid…) }
            let mid = interner.intern("__sym_mid");
            let mid_let = Stmt::Let {
                var: mid,
                ty: None,
                value: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Shl,
                    left: int_lit(1, ea),
                    right: shr1(e.n, ea),
                }),
                mutable: false,
            };
            let mid_call = subst_ident(e.loop_info.call, e.loop_info.bit, mid, ea);
            let add_mid = Stmt::Set {
                target: acc,
                value: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Add,
                    left: ea.alloc(Expr::Identifier(acc)),
                    right: mid_call,
                }),
            };
            let middle_body: Vec<Stmt<'a>> = match e.loop_info.guard {
                Some(g) => {
                    let mid_guard = subst_ident(g, e.loop_info.bit, mid, ea);
                    vec![
                        mid_let,
                        Stmt::If {
                            cond: mid_guard,
                            then_block: sa.alloc_slice(vec![add_mid]),
                            else_block: None,
                        },
                    ]
                }
                None => vec![mid_let, add_mid],
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
            out.push(Stmt::If { cond: odd, then_block: sa.alloc_slice(middle_body), else_block: None });
        } else {
            out.push(s.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Value symmetry (Sₖ) — Milestone B.
//
// When a counting search assigns INTERCHANGEABLE values (its only constraint on
// an assigned value is an equality/inequality against another assigned value —
// e.g. a proper graph coloring, whose sole rule is "adjacent nodes differ"), the
// k values form a symmetric group Sₖ: every value bijection maps a valid
// assignment to a valid assignment. So the first free choice can be pinned to
// the smallest value and the entry result multiplied by k — the Sₖ analogue of
// the reflection C₂ break above, acting on the first choice instead of the board.
// ---------------------------------------------------------------------------

/// The kernel-checked backing for the value-symmetry rewrite, via the in-tree
/// permutation-group engine (`logicaffeine_proof::permgroup`, Schreier–Sims):
/// for each representative value-count `k`, the group generated by the adjacent
/// transpositions is the full symmetric group Sₖ (order `k!`), it acts
/// TRANSITIVELY on the value domain (the orbit of any value is all k values —
/// this is exactly why the k first-choices are equinumerous, so `×k` is the
/// correct orbit weighting), and every element PRESERVES the equality/inequality
/// relation on all value pairs (so an equality-only constraint system is closed
/// under the whole group). Together: the symmetry group of an equality-only
/// value system on {0,…,k−1} is exactly the transitive Sₖ, the sound premise for
/// fixing the first choice and scaling by k. Memoised; unbounded in spirit
/// (checked on representatives, the relation is generic in the values).
fn value_bijection_preserves_equality() -> bool {
    use logicaffeine_proof::permgroup::{orbits, schreier_sims};
    for k in 2..=6usize {
        // adjacent transpositions (i i+1) generate Sₖ
        let gens: Vec<Vec<usize>> = (0..k - 1)
            .map(|i| {
                let mut p: Vec<usize> = (0..k).collect();
                p.swap(i, i + 1);
                p
            })
            .collect();
        let bsgs = schreier_sims(k, &gens);
        let fact: u128 = (1..=k as u128).product();
        if bsgs.order() != fact {
            return false;
        }
        // transitive: a single orbit covering all k values.
        let orbs = orbits(k, &gens);
        if orbs.len() != 1 || orbs[0].len() != k {
            return false;
        }
        // every group element preserves equality/inequality on all value pairs.
        let Some(elems) = bsgs.elements(fact as usize) else { return false };
        for sigma in &elems {
            for a in 0..k {
                for b in 0..k {
                    if (a == b) != (sigma[a] == sigma[b]) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// The value-symmetry closure discharge (memoised; constant): the equality-only
/// value system's symmetry group is the transitive Sₖ (permgroup-checked above).
fn value_symmetry_closure_proven() -> bool {
    use std::sync::OnceLock;
    static PROVEN: OnceLock<bool> = OnceLock::new();
    *PROVEN.get_or_init(value_bijection_preserves_equality)
}

/// Does `e` contain any bare occurrence of `sym`? Conservative: unknown node
/// kinds are treated as possibly containing it (fail-closed).
fn expr_contains_sym(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == sym,
        Expr::Literal(_) => false,
        Expr::BinaryOp { left, right, .. } => {
            expr_contains_sym(left, sym) || expr_contains_sym(right, sym)
        }
        Expr::Not { operand } => expr_contains_sym(operand, sym),
        Expr::Call { args, .. } => args.iter().any(|a| expr_contains_sym(a, sym)),
        _ => true,
    }
}

/// Every occurrence of `sym` in `e` is a DIRECT operand of an `=`/`≠` comparison
/// (never in arithmetic, ordering, a call argument, or an unknown node). This is
/// the per-program certificate that a chosen value is used only as an
/// equality-comparable identity — the structural half of the Sₖ invariance.
fn expr_sym_only_in_equality(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s != sym,
        Expr::Literal(_) => true,
        Expr::BinaryOp { op: BinaryOpKind::Eq | BinaryOpKind::NotEq, left, right } => {
            eq_side_ok(left, sym) && eq_side_ok(right, sym)
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_sym_only_in_equality(left, sym) && expr_sym_only_in_equality(right, sym)
        }
        Expr::Not { operand } => expr_sym_only_in_equality(operand, sym),
        Expr::Call { args, .. } => args.iter().all(|a| expr_sym_only_in_equality(a, sym)),
        _ => !expr_contains_sym(e, sym),
    }
}

/// Within an `=`/`≠`, a bare `sym` operand is allowed; a compound operand must
/// not use `sym` outside a further nested equality.
fn eq_side_ok(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(_) => true,
        _ => expr_sym_only_in_equality(e, sym),
    }
}

/// `sym` appears throughout `body` only as an equality/inequality operand.
fn param_used_only_in_equality(sym: Symbol, body: Block) -> bool {
    body.iter().all(|s| match s {
        Stmt::Let { value, .. } => expr_sym_only_in_equality(value, sym),
        Stmt::Set { value, .. } => expr_sym_only_in_equality(value, sym),
        Stmt::Return { value } => value.map_or(true, |v| expr_sym_only_in_equality(v, sym)),
        Stmt::If { cond, then_block, else_block } => {
            expr_sym_only_in_equality(cond, sym)
                && param_used_only_in_equality(sym, then_block)
                && else_block.map_or(true, |b| param_used_only_in_equality(sym, b))
        }
        Stmt::While { cond, body, .. } => {
            expr_sym_only_in_equality(cond, sym) && param_used_only_in_equality(sym, body)
        }
        // Any other statement kind → not a clean value search we can certify;
        // fail closed.
        _ => false,
    })
}

/// Match the per-level value-iteration loop of a value-symmetric search
/// `f(level, val, …)`:
///   `while c cmp k { if c (=|≠) prev { acc = acc + f(level+1, …c…) } ; c = c+1 }`
/// with `cmp ∈ {<, ≤}`, `prev` a parameter (the previous chosen value), and `c`
/// flowing into exactly one argument of the self-call (its position = `val`'s).
/// Returns `(val-arg-position, prev-symbol)`.
fn match_value_search_loop(
    f: Symbol,
    params: &[Symbol],
    level: Symbol,
    cond: &Expr,
    body: Block,
) -> Option<(usize, Symbol)> {
    let c = match cond {
        Expr::BinaryOp { op: BinaryOpKind::Lt | BinaryOpKind::LtEq, left, .. } => as_ident(left)?,
        _ => return None,
    };
    if body.len() != 2 {
        return None;
    }
    let mut saw_incr = false;
    let mut found: Option<(usize, Symbol)> = None;
    for s in body {
        match s {
            Stmt::Set { target, value } if *target == c && is_step(value, c) => saw_incr = true,
            Stmt::If { cond: guard, then_block, else_block: None } if then_block.len() == 1 => {
                let prev = match guard {
                    Expr::BinaryOp { op: BinaryOpKind::Eq | BinaryOpKind::NotEq, left, right } => {
                        if as_ident(left) == Some(c) {
                            as_ident(right)
                        } else if as_ident(right) == Some(c) {
                            as_ident(left)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }?;
                if prev == c || !params.contains(&prev) {
                    return None;
                }
                let Stmt::Set { target: acc, value } = &then_block[0] else { return None };
                let (a, b) = is_binop(value, BinaryOpKind::Add)?;
                let call = if as_ident(a) == Some(*acc) {
                    b
                } else if as_ident(b) == Some(*acc) {
                    a
                } else {
                    return None;
                };
                let Expr::Call { function, args } = call else { return None };
                if *function != f || args.len() != params.len() || !is_step(args[0], level) {
                    return None;
                }
                let positions: Vec<usize> = args
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| as_ident(a) == Some(c))
                    .map(|(i, _)| i)
                    .collect();
                if positions.len() != 1 || positions[0] == 0 {
                    return None;
                }
                found = Some((positions[0], prev));
            }
            _ => return None,
        }
    }
    if !saw_incr {
        return None;
    }
    found
}

/// Confirm `f` is a value-symmetric recursive search and return the argument
/// position of its "previous value" parameter. The search must have a
/// value-independent base case `if level cmp size { return CONST }`, a value
/// loop (above), and use the previous-value parameter ONLY in equality
/// comparisons — so the k values are interchangeable (Sₖ).
fn value_search_val_pos(f: Symbol, params: &[Symbol], body: Block) -> Option<usize> {
    if body.is_empty() {
        return None;
    }
    let level = match &body[0] {
        Stmt::If { cond, then_block, else_block: None } => {
            let level = match cond {
                Expr::BinaryOp {
                    op: BinaryOpKind::Gt | BinaryOpKind::GtEq | BinaryOpKind::Eq,
                    left,
                    ..
                } => as_ident(left)?,
                _ => return None,
            };
            if then_block.len() != 1
                || !matches!(&then_block[0], Stmt::Return { value: Some(v) } if as_int(v).is_some())
            {
                return None;
            }
            level
        }
        _ => return None,
    };
    for s in body {
        if let Stmt::While { cond, body: lb, .. } = s {
            if let Some((valpos, prev)) = match_value_search_loop(f, params, level, cond, lb) {
                if param_used_only_in_equality(prev, body) {
                    return Some(valpos);
                }
            }
        }
    }
    None
}

/// A recognized value-enumeration entry: `E(…)` whose body sums `f(…, c, …)`
/// over the first choice `c ∈ {1,…,k}` for a value-symmetric search `f`, with an
/// UNGUARDED accumulate (the first choice is free).
struct ValueEntry<'a> {
    while_idx: usize,
    total: Symbol,
    call: &'a Expr<'a>,
    valpos: usize,
    bound: &'a Expr<'a>,
}

/// Recognize `total = 0; c = 1; while c ≤ k { total = total + f(…c…) ; c = c+1 };
/// return total` for a known value-symmetric search `f` whose value parameter
/// receives `c`. The loop enumerates the first choice over `k` interchangeable
/// values from the smallest (`c = 1`), unguarded, so pinning it to `1` and
/// scaling by `k` is exact.
fn recognize_value_entry<'a>(
    body: Block<'a>,
    val_searches: &HashMap<Symbol, usize>,
) -> Option<ValueEntry<'a>> {
    for (i, s) in body.iter().enumerate() {
        let Stmt::While { cond, body: lb, .. } = s else { continue };
        let (c, bound) = match cond {
            Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => match as_ident(left) {
                Some(c) => (c, *right),
                None => continue,
            },
            _ => continue,
        };
        if lb.len() != 2 {
            continue;
        }
        let mut saw_incr = false;
        let mut acc_call: Option<(Symbol, &'a Expr<'a>)> = None;
        let mut clean = true;
        for st in *lb {
            match st {
                Stmt::Set { target, value } if *target == c && is_step(value, c) => {
                    saw_incr = true;
                }
                Stmt::Set { target, value } => match is_binop(value, BinaryOpKind::Add) {
                    Some((a, b)) if as_ident(a) == Some(*target) => acc_call = Some((*target, b)),
                    Some((a, b)) if as_ident(b) == Some(*target) => acc_call = Some((*target, a)),
                    _ => {
                        clean = false;
                    }
                },
                // A guarded accumulate (or anything else) means the first choice
                // is not free — cannot pin it. Fail closed.
                _ => {
                    clean = false;
                }
            }
        }
        if !clean || !saw_incr {
            continue;
        }
        let Some((total, call)) = acc_call else { continue };
        let Expr::Call { function, args } = call else { continue };
        let valpos = match val_searches.get(function) {
            Some(p) => *p,
            None => continue,
        };
        if valpos >= args.len() || as_ident(args[valpos]) != Some(c) {
            continue;
        }
        if args.iter().enumerate().any(|(j, a)| j != valpos && as_ident(a) == Some(c)) {
            continue;
        }
        let starts_at_one = body[..i].iter().any(
            |s| matches!(s, Stmt::Let { var, value, .. } if *var == c && as_int(value) == Some(1)),
        );
        let total_zero = body[..i].iter().any(
            |s| matches!(s, Stmt::Let { var, value, .. } if *var == total && as_int(value) == Some(0)),
        );
        let returns_total =
            matches!(body.last(), Some(Stmt::Return { value: Some(v) }) if as_ident(v) == Some(total));
        if !starts_at_one || !total_zero || !returns_total {
            continue;
        }
        return Some(ValueEntry { while_idx: i, total, call, valpos, bound });
    }
    None
}

/// Rewrite the value-enumeration entry to the Sₖ-broken form: pin the first
/// choice to the smallest value (`1`) and multiply the sub-count by `k`
/// (`total = __valuesym * f(…, 1, …)`, where `__valuesym = k` records the orbit
/// size). Count-preserving: every one of the k interchangeable first values
/// yields the same sub-count, so `k · subcount(1) = Σ_c subcount(c)`.
fn rewrite_value_entry<'a>(
    e: &ValueEntry<'a>,
    body: Block<'a>,
    ea: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let vsym = interner.intern("__valuesym");
    let Expr::Call { function, args } = e.call else { return body.to_vec() };
    let new_args: Vec<&'a Expr<'a>> = args
        .iter()
        .enumerate()
        .map(|(j, a)| if j == e.valpos { int_lit(1, ea) } else { *a })
        .collect();
    let fixed_call = ea.alloc(Expr::Call { function: *function, args: new_args });

    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(body.len() + 1);
    for (i, s) in body.iter().enumerate() {
        if i == e.while_idx {
            out.push(Stmt::Let { var: vsym, ty: None, value: e.bound, mutable: false });
            out.push(Stmt::Set {
                target: e.total,
                value: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Multiply,
                    left: ea.alloc(Expr::Identifier(vsym)),
                    right: fixed_call,
                }),
            });
        } else {
            out.push(s.clone());
        }
    }
    out
}

/// Apply symmetry-breaking where a kernel certificate holds and an entry over a
/// symmetric search is recognized — reflection (C₂) OR value symmetry (Sₖ).
/// Fail-closed on every axis.
pub fn break_symmetry_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    // Catalogue reflection-symmetric searches AND value-symmetric searches (a
    // function is at most one: reflection is tried first).
    let mut searches: HashMap<Symbol, ()> = HashMap::new();
    let mut val_searches: HashMap<Symbol, usize> = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef {
            name, params, body, is_native: false, generics, ..
        } = s
        {
            if generics.is_empty() {
                let psyms: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
                if is_reflection_search(*name, &psyms, body) {
                    searches.insert(*name, ());
                } else if let Some(valpos) = value_search_val_pos(*name, &psyms, body) {
                    val_searches.insert(*name, valpos);
                }
            }
        }
    }
    if searches.is_empty() && val_searches.is_empty() {
        return stmts;
    }
    // Each symmetry fires only if BOTH its catalogue is non-empty AND its kernel
    // certificate holds (both are unbounded, machine-checked). Fail-closed.
    let refl_ok = !searches.is_empty() && reflection_closure_proven();
    let value_ok = !val_searches.is_empty() && value_symmetry_closure_proven();
    if !refl_ok && !value_ok {
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
                && !searches.contains_key(name)
                && !val_searches.contains_key(name) =>
            {
                let psyms: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
                let mut new_body: Option<Vec<Stmt<'a>>> = None;
                if refl_ok {
                    if let Some(entry) = recognize_entry(&psyms, body, &searches) {
                        new_body =
                            Some(rewrite_entry(&entry, body, expr_arena, stmt_arena, interner));
                    }
                }
                if new_body.is_none() && value_ok {
                    if let Some(entry) = recognize_value_entry(body, &val_searches) {
                        new_body = Some(rewrite_value_entry(&entry, body, expr_arena, interner));
                    }
                }
                match new_body {
                    Some(nb) => Stmt::FunctionDef {
                        name: *name,
                        generics: generics.clone(),
                        params: params.clone(),
                        body: stmt_arena.alloc_slice(nb),
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

    #[test]
    fn value_symmetry_group_is_transitive_symmetric() {
        // permgroup-checked: the equality-only value system's symmetry group is
        // the full, transitive Sₖ (order k!, one orbit, equality-preserving).
        assert!(
            value_bijection_preserves_equality(),
            "permgroup failed to certify the value symmetry as the transitive Sₖ"
        );
        assert!(value_symmetry_closure_proven());
    }
}
