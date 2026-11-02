//! Closed-Form Loop Recognition.
//!
//! Detects simple accumulator loops and replaces them with closed-form formulas:
//! - `sum += i` over range [start, limit] → Gauss formula
//! - `count += 1` over range [start, limit] → `limit - start + 1`

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::optimization::Opt;
use std::cell::RefCell;

/// The program's canonical `toReal` (Int→Real cast) and `trunc` (toward zero) symbols, verified
/// by BODY so a user shadow (`## To toReal(n)->Real: Return 999.0.`) is never matched. Populated
/// per `closed_form_stmts` run. Used so the loop recognizer can treat `trunc(toReal(i))` as `i`:
/// within the pass's `start>=1` + i64-bound-guarded fast path the counter is a small positive
/// integer, so `toReal(i)` is exact and `trunc` of it is `i` — letting the Gauss reduction fire.
#[derive(Clone, Copy, Default)]
struct KnownCasts {
    to_real: Option<Symbol>,
    trunc: Option<Symbol>,
}

thread_local! {
    static KNOWN_CASTS: RefCell<KnownCasts> = const { RefCell::new(KnownCasts { to_real: None, trunc: None }) };
    /// Symbol → the affine `(a, b)` of a 1-param function whose body is `Return a·param + b`
    /// (a pure affine helper — includes the partial-eval SPECIALIZATIONS like `opAdd_s1_7(x)=x+7`
    /// that exist before inlining). Lets the affine closed-form reach through such calls.
    static AFFINE_FNS: RefCell<std::collections::HashMap<Symbol, (i64, i64)>> =
        RefCell::new(std::collections::HashMap::new());
}

/// `body` is exactly `Return <param> + 0.0` (or `<param> * 1.0`) — the Int→Real cast.
fn is_int_cast_body(param: Symbol, body: &[Stmt]) -> bool {
    let [Stmt::Return { value: Some(e) }] = body else {
        return false;
    };
    let Expr::BinaryOp { op, left, right } = &**e else {
        return false;
    };
    let left_is_param = matches!(&**left, Expr::Identifier(s) if *s == param);
    match op {
        BinaryOpKind::Add => {
            left_is_param && matches!(&**right, Expr::Literal(Literal::Float(f)) if *f == 0.0)
        }
        BinaryOpKind::Multiply => {
            left_is_param && matches!(&**right, Expr::Literal(Literal::Float(f)) if *f == 1.0)
        }
        _ => false,
    }
}

/// `body` is exactly `If <param> < 0.0: … ; Return floor(<param>)` — the canonical truncation. The
/// guarded `If <param> < 0.0` means the positive path (`Return floor(param)`) is what runs for the
/// non-negative loop counter, and `floor` of an exact non-negative integer float is that integer.
fn is_trunc_body(param: Symbol, body: &[Stmt], interner: &Interner) -> bool {
    let [Stmt::If { cond, .. }, Stmt::Return { value: Some(e) }] = body else {
        return false;
    };
    let cond_ok = matches!(&**cond, Expr::BinaryOp { op: BinaryOpKind::Lt, left, right }
        if matches!(&**left, Expr::Identifier(s) if *s == param)
            && matches!(&**right, Expr::Literal(Literal::Float(f)) if *f == 0.0));
    if !cond_ok {
        return false;
    }
    matches!(&**e, Expr::Call { function, args }
        if interner.resolve(*function) == "floor" && args.len() == 1
            && matches!(args[0], Expr::Identifier(s) if *s == param))
}

/// Scan top-level function defs for the canonical `toReal`/`trunc` (EXACTLY ONE def each with the
/// canonical body — a user shadow with a different body is not recognized) AND build the affine-fn
/// map: every 1-param function whose body is `Return <affine in param>` (pure affine helper,
/// including partial-eval specializations like `opAdd_s1_7(x)=x+7`).
fn collect_known_casts(stmts: &[Stmt], interner: &Interner) -> KnownCasts {
    let mut kc = KnownCasts::default();
    let (mut n_real, mut n_trunc) = (0u32, 0u32);
    AFFINE_FNS.with(|m| {
        let mut m = m.borrow_mut();
        m.clear();
        for s in stmts {
            if let Stmt::FunctionDef {
                name, params, body, ..
            } = s
            {
                match interner.resolve(*name) {
                    "toReal" if params.len() == 1 => {
                        n_real += 1;
                        if is_int_cast_body(params[0].0, body) {
                            kc.to_real = Some(*name);
                        }
                    }
                    "trunc" if params.len() == 1 => {
                        n_trunc += 1;
                        if is_trunc_body(params[0].0, body, interner) {
                            kc.trunc = Some(*name);
                        }
                    }
                    _ => {}
                }
                if params.len() == 1 {
                    if let [Stmt::Return { value: Some(e) }] = *body {
                        if let Some((a, b)) = affine_of(e, params[0].0) {
                            if let (Ok(a), Ok(b)) = (i64::try_from(a), i64::try_from(b)) {
                                m.insert(*name, (a, b));
                            }
                        }
                    }
                }
            }
        }
    });
    if n_real != 1 {
        kc.to_real = None;
    }
    if n_trunc != 1 {
        kc.trunc = None;
    }
    kc
}

/// `e` is `f(counter)` for a known affine 1-param helper → its `(a, b)` in the counter.
fn affine_of_1arg_call(e: &Expr, counter: Symbol) -> Option<(i128, i128)> {
    let Expr::Call { function, args } = e else {
        return None;
    };
    if args.len() != 1 || !matches!(args[0], Expr::Identifier(s) if *s == counter) {
        return None;
    }
    AFFINE_FNS.with(|m| {
        m.borrow()
            .get(function)
            .map(|&(a, b)| (a as i128, b as i128))
    })
}

/// `e` is `trunc(toReal(counter))` for the canonical casts — equal to `counter` in the guarded range.
fn is_trunc_toreal_of(e: &Expr, counter: Symbol) -> bool {
    KNOWN_CASTS.with(|kc| {
        let kc = kc.borrow();
        let (Some(trunc_sym), Some(to_real_sym)) = (kc.trunc, kc.to_real) else {
            return false;
        };
        let Expr::Call { function, args } = e else {
            return false;
        };
        if *function != trunc_sym || args.len() != 1 {
            return false;
        }
        let Expr::Call {
            function: inner,
            args: iargs,
        } = args[0]
        else {
            return false;
        };
        *inner == to_real_sym
            && iargs.len() == 1
            && matches!(iargs[0], Expr::Identifier(s) if *s == counter)
    })
}

#[derive(Debug)]
enum AccumPattern {
    SumOfCounter,
    /// `acc += counter * counter` — Faulhaber `Σ i² = n(n+1)(2n+1)/6`.
    SumOfSquares,
    /// `acc += counter * counter * counter` — Faulhaber `Σ i³ = (n(n+1)/2)²`.
    SumOfCubes,
    Count,
    MulByTwo,
    /// `acc += a·counter + b` (a linear function of the counter, a≠1∨b≠0) → `a·Σi + b·count`.
    /// The codegen overflow-guard promotes the raw formula's products to BigInt exactly as the
    /// loop would, so this is exact for any `a`,`b` (no manual bound needed).
    AffineSum {
        a: i64,
        b: i64,
    },
}

struct Candidate {
    accum: Symbol,
    counter: Symbol,
    pattern: AccumPattern,
}

fn try_extract_candidate(body: &[Stmt], while_cond: &Expr) -> Option<Candidate> {
    let sets: Vec<_> = body
        .iter()
        .filter(|s| matches!(s, Stmt::Set { .. }))
        .collect();
    if sets.len() != 2 {
        return None;
    }

    for s in body {
        match s {
            Stmt::Set { .. } | Stmt::Let { .. } => {}
            _ => return None,
        }
    }

    // Identify the loop counter from the While condition
    let cond_counter = match while_cond {
        Expr::BinaryOp { left, .. } => {
            if let Expr::Identifier(sym) = &**left {
                Some(*sym)
            } else {
                None
            }
        }
        _ => None,
    }?;

    // Find the counter increment: Set cond_counter to cond_counter + 1
    let mut counter_found = false;
    let mut accum_stmt_idx = None;

    for (idx, s) in body.iter().enumerate() {
        if let Stmt::Set { target, value } = s {
            if *target == cond_counter {
                if let Expr::BinaryOp {
                    op: BinaryOpKind::Add,
                    left,
                    right,
                } = &**value
                {
                    if let Expr::Identifier(lhs) = &**left {
                        if *lhs == cond_counter {
                            if let Expr::Literal(Literal::Number(1)) = &**right {
                                counter_found = true;
                                continue;
                            }
                        }
                    }
                }
                return None; // counter is modified but not as +1
            }
            accum_stmt_idx = Some(idx);
        }
    }

    if !counter_found {
        return None;
    }
    let counter = cond_counter;
    let accum_idx = accum_stmt_idx?;

    if let Stmt::Set {
        target: accum,
        value,
    } = &body[accum_idx]
    {
        // Additive patterns: accum = accum + counter (SumOfCounter), accum = accum + 1 (Count)
        if let Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } = &**value
        {
            if let Expr::Identifier(lhs) = &**left {
                if *lhs == *accum {
                    if let Expr::Identifier(rhs) = &**right {
                        if *rhs == counter {
                            return Some(Candidate {
                                accum: *accum,
                                counter,
                                pattern: AccumPattern::SumOfCounter,
                            });
                        }
                    }
                    if let Expr::Literal(Literal::Number(1)) = &**right {
                        return Some(Candidate {
                            accum: *accum,
                            counter,
                            pattern: AccumPattern::Count,
                        });
                    }
                    // Faulhaber polynomials: `acc += counter²` / `acc += counter³`.
                    if is_power_of_counter(&**right, counter, 2) {
                        return Some(Candidate {
                            accum: *accum,
                            counter,
                            pattern: AccumPattern::SumOfSquares,
                        });
                    }
                    if is_power_of_counter(&**right, counter, 3) {
                        return Some(Candidate {
                            accum: *accum,
                            counter,
                            pattern: AccumPattern::SumOfCubes,
                        });
                    }
                    // `acc += trunc(toReal(counter))` == `acc += counter` (the counter is a small
                    // positive integer in the pass's guarded fast path, so the Int→Real→trunc round
                    // trip is the identity). Body-verified canonical casts only — see KnownCasts.
                    if is_trunc_toreal_of(&**right, counter) {
                        return Some(Candidate {
                            accum: *accum,
                            counter,
                            pattern: AccumPattern::SumOfCounter,
                        });
                    }
                    // General affine addend `a·counter + b` (bare counter / literal-1 already
                    // handled above): closes to `a·Σi + b·count`. Exact via the codegen overflow
                    // guard (promotes to BigInt like the loop). i128→i64 keeps literals in range.
                    if let Some((a, b)) = affine_of(&**right, counter)
                        .or_else(|| affine_of_1arg_call(&**right, counter))
                    {
                        if let (Ok(a), Ok(b)) = (i64::try_from(a), i64::try_from(b)) {
                            return Some(Candidate {
                                accum: *accum,
                                counter,
                                pattern: AccumPattern::AffineSum { a, b },
                            });
                        }
                    }
                }
            }
        }
        // Multiplicative pattern: accum = accum * 2 → power-of-2 (accum << count)
        // Also handles the strength-reduced form: accum = accum << 1
        if let Expr::BinaryOp { op, left, right } = &**value {
            let is_mul_by_2 = match op {
                BinaryOpKind::Multiply => match (left, right) {
                    (Expr::Identifier(lhs), Expr::Literal(Literal::Number(2)))
                        if *lhs == *accum =>
                    {
                        true
                    }
                    (Expr::Literal(Literal::Number(2)), Expr::Identifier(rhs))
                        if *rhs == *accum =>
                    {
                        true
                    }
                    _ => false,
                },
                BinaryOpKind::Shl => {
                    // accum << 1 (strength-reduced form from fold pass)
                    match (left, right) {
                        (Expr::Identifier(lhs), Expr::Literal(Literal::Number(1)))
                            if *lhs == *accum =>
                        {
                            true
                        }
                        _ => false,
                    }
                }
                _ => false,
            };
            if is_mul_by_2 {
                return Some(Candidate {
                    accum: *accum,
                    counter,
                    pattern: AccumPattern::MulByTwo,
                });
            }
        }
    }

    None
}

/// Whether `e` is `counter` multiplied by itself `k` times, left-associated (`i`, `i*i`, `(i*i)*i`, …) — the
/// shape `Set acc to acc + i * i [* i]` parses to. Recognizes the addend of a Faulhaber power sum.
fn is_power_of_counter(e: &Expr, counter: Symbol, k: u32) -> bool {
    match k {
        0 => false,
        1 => matches!(e, Expr::Identifier(s) if *s == counter),
        _ => {
            if let Expr::BinaryOp {
                op: BinaryOpKind::Multiply,
                left,
                right,
            } = e
            {
                is_power_of_counter(left, counter, k - 1)
                    && matches!(&**right, Expr::Identifier(s) if *s == counter)
            } else {
                false
            }
        }
    }
}

/// `Σ_{i=1}^{m} i² = m(m+1)(2m+1)/6` as a compile-time literal (the product is always divisible by 6).
fn faulhaber_squares(m: i64) -> i64 {
    m * (m + 1) * (2 * m + 1) / 6
}

/// `Σ_{i=1}^{m} i³ = (m(m+1)/2)²` as a compile-time literal.
fn faulhaber_cubes(m: i64) -> i64 {
    let half = m * (m + 1) / 2;
    half * half
}

fn extract_while_limit<'a>(cond: &'a Expr<'a>, counter: Symbol) -> Option<(&'a Expr<'a>, bool)> {
    match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left,
            right,
        } => {
            if let Expr::Identifier(sym) = &**left {
                if *sym == counter {
                    return Some((right, false));
                }
            }
            None
        }
        Expr::BinaryOp {
            op: BinaryOpKind::LtEq,
            left,
            right,
        } => {
            if let Expr::Identifier(sym) = &**left {
                if *sym == counter {
                    return Some((right, true));
                }
            }
            None
        }
        _ => None,
    }
}

fn find_init_value(stmts: &[Stmt], sym: Symbol) -> Option<i64> {
    for s in stmts.iter().rev() {
        match s {
            Stmt::Let { var, value, .. } if *var == sym => {
                if let Expr::Literal(Literal::Number(n)) = &**value {
                    return Some(*n);
                }
                return None;
            }
            Stmt::Set { target, value } if *target == sym => {
                if let Expr::Literal(Literal::Number(n)) = &**value {
                    return Some(*n);
                }
                return None;
            }
            // Control flow may modify the variable — bail conservatively
            Stmt::If { .. }
            | Stmt::While { .. }
            | Stmt::Repeat { .. }
            | Stmt::Call { .. }
            | Stmt::Escape { .. }
            | Stmt::Zone { .. } => {
                return None;
            }
            _ => {}
        }
    }
    None
}

fn mk_int<'a>(n: i64, arena: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
    arena.alloc(Expr::Literal(Literal::Number(n)))
}

fn mk_binop<'a>(
    op: BinaryOpKind,
    left: &'a Expr<'a>,
    right: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    arena.alloc(Expr::BinaryOp { op, left, right })
}

fn build_formula<'a>(
    pattern: &AccumPattern,
    init: i64,
    start: i64,
    limit: &'a Expr<'a>,
    inclusive: bool,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    match pattern {
        AccumPattern::Count => {
            // count = limit - start + 1 (inclusive) or limit - start (exclusive)
            let count = if inclusive {
                mk_binop(
                    BinaryOpKind::Add,
                    mk_binop(BinaryOpKind::Subtract, limit, mk_int(start, ea), ea),
                    mk_int(1, ea),
                    ea,
                )
            } else {
                mk_binop(BinaryOpKind::Subtract, limit, mk_int(start, ea), ea)
            };
            if init == 0 {
                count
            } else {
                mk_binop(BinaryOpKind::Add, mk_int(init, ea), count, ea)
            }
        }
        AccumPattern::MulByTwo => {
            unreachable!(
                "MulByTwo folds to its exact literal in the caller (a runtime-count \
                 `<<` would wrap past 63 bits where the exact loop promotes)"
            )
        }
        AccumPattern::SumOfCounter => {
            // Gauss: effective_limit * (effective_limit + 1) / 2
            let eff_limit = if inclusive {
                limit
            } else {
                mk_binop(BinaryOpKind::Subtract, limit, mk_int(1, ea), ea)
            };
            let gauss_top = mk_binop(
                BinaryOpKind::Divide,
                mk_binop(
                    BinaryOpKind::Multiply,
                    eff_limit,
                    mk_binop(BinaryOpKind::Add, eff_limit, mk_int(1, ea), ea),
                    ea,
                ),
                mk_int(2, ea),
                ea,
            );

            let sum = if start <= 1 {
                gauss_top
            } else {
                let start_part = mk_binop(
                    BinaryOpKind::Divide,
                    mk_binop(
                        BinaryOpKind::Multiply,
                        mk_int(start - 1, ea),
                        mk_int(start, ea),
                        ea,
                    ),
                    mk_int(2, ea),
                    ea,
                );
                mk_binop(BinaryOpKind::Subtract, gauss_top, start_part, ea)
            };

            if init == 0 {
                sum
            } else {
                mk_binop(BinaryOpKind::Add, mk_int(init, ea), sum, ea)
            }
        }
        AccumPattern::SumOfSquares => {
            // Σ_{i=start}^{eff} i² = eff(eff+1)(2·eff+1)/6 − (start-1)(start)(2·start-1)/6
            let eff = if inclusive {
                limit
            } else {
                mk_binop(BinaryOpKind::Subtract, limit, mk_int(1, ea), ea)
            };
            let eff1 = mk_binop(BinaryOpKind::Add, eff, mk_int(1, ea), ea);
            let two_eff1 = mk_binop(
                BinaryOpKind::Add,
                mk_binop(BinaryOpKind::Multiply, mk_int(2, ea), eff, ea),
                mk_int(1, ea),
                ea,
            );
            let full = mk_binop(
                BinaryOpKind::Divide,
                mk_binop(
                    BinaryOpKind::Multiply,
                    mk_binop(BinaryOpKind::Multiply, eff, eff1, ea),
                    two_eff1,
                    ea,
                ),
                mk_int(6, ea),
                ea,
            );
            let sum = if start <= 1 {
                full
            } else {
                mk_binop(
                    BinaryOpKind::Subtract,
                    full,
                    mk_int(faulhaber_squares(start - 1), ea),
                    ea,
                )
            };
            if init == 0 {
                sum
            } else {
                mk_binop(BinaryOpKind::Add, mk_int(init, ea), sum, ea)
            }
        }
        AccumPattern::SumOfCubes => {
            // Σ_{i=start}^{eff} i³ = (eff(eff+1)/2)² − ((start-1)·start/2)²
            let eff = if inclusive {
                limit
            } else {
                mk_binop(BinaryOpKind::Subtract, limit, mk_int(1, ea), ea)
            };
            let half = mk_binop(
                BinaryOpKind::Divide,
                mk_binop(
                    BinaryOpKind::Multiply,
                    eff,
                    mk_binop(BinaryOpKind::Add, eff, mk_int(1, ea), ea),
                    ea,
                ),
                mk_int(2, ea),
                ea,
            );
            let full = mk_binop(BinaryOpKind::Multiply, half, half, ea);
            let sum = if start <= 1 {
                full
            } else {
                mk_binop(
                    BinaryOpKind::Subtract,
                    full,
                    mk_int(faulhaber_cubes(start - 1), ea),
                    ea,
                )
            };
            if init == 0 {
                sum
            } else {
                mk_binop(BinaryOpKind::Add, mk_int(init, ea), sum, ea)
            }
        }
        AccumPattern::AffineSum { a, b } => {
            // a·Σi + b·count + init, over [start, eff]. Raw formula; the codegen overflow guard
            // promotes products to BigInt exactly as the loop does (sound for any a, b).
            let eff = if inclusive {
                limit
            } else {
                mk_binop(BinaryOpKind::Subtract, limit, mk_int(1, ea), ea)
            };
            let gauss_top = mk_binop(
                BinaryOpKind::Divide,
                mk_binop(
                    BinaryOpKind::Multiply,
                    eff,
                    mk_binop(BinaryOpKind::Add, eff, mk_int(1, ea), ea),
                    ea,
                ),
                mk_int(2, ea),
                ea,
            );
            let gauss = if start <= 1 {
                gauss_top
            } else {
                let sp = mk_binop(
                    BinaryOpKind::Divide,
                    mk_binop(
                        BinaryOpKind::Multiply,
                        mk_int(start - 1, ea),
                        mk_int(start, ea),
                        ea,
                    ),
                    mk_int(2, ea),
                    ea,
                );
                mk_binop(BinaryOpKind::Subtract, gauss_top, sp, ea)
            };
            // count = eff - start + 1
            let count = mk_binop(
                BinaryOpKind::Add,
                mk_binop(BinaryOpKind::Subtract, eff, mk_int(start, ea), ea),
                mk_int(1, ea),
                ea,
            );
            let mut total = if *a == 1 {
                gauss
            } else {
                mk_binop(BinaryOpKind::Multiply, mk_int(*a, ea), gauss, ea)
            };
            if *b != 0 {
                let bc = mk_binop(BinaryOpKind::Multiply, mk_int(*b, ea), count, ea);
                total = mk_binop(BinaryOpKind::Add, total, bc, ea);
            }
            if init != 0 {
                total = mk_binop(BinaryOpKind::Add, mk_int(init, ea), total, ea);
            }
            total
        }
    }
}

/// True if `value` is exactly `counter + 1`.
fn is_counter_plus_one(value: &Expr, counter: Symbol) -> bool {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = value
    {
        if let (Expr::Identifier(l), Expr::Literal(Literal::Number(1))) = (&**left, &**right) {
            return *l == counter;
        }
    }
    false
}

/// O8a — modulus deferral. Rewrite a counted loop whose body is exactly
/// `Set acc to (acc + counter) % p` + `Set counter to counter + 1` into a
/// guarded chunked form that applies `% p` once per K iterations instead of
/// every iteration. Sound because, with `acc` and `counter` starting ≥ 0,
/// every partial sum is non-negative, so truncated remainder equals
/// mathematical mod and deferring the reduction is value-preserving. The
/// `If limit <= K_SAFE` guard ensures the K-deep accumulation cannot
/// overflow i64; otherwise the original loop runs unchanged.
fn try_defer_modulus<'a>(
    cond: &'a Expr<'a>,
    body: Block<'a>,
    preceding: &[Stmt<'a>],
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Option<Vec<Stmt<'a>>> {
    const K: i64 = 16;

    let counter = match cond {
        Expr::BinaryOp { left, .. } => match &**left {
            Expr::Identifier(s) => *s,
            _ => return None,
        },
        _ => return None,
    };
    let (limit_expr, inclusive) = extract_while_limit(cond, counter)?;

    // Body must be exactly the accumulate + the unit counter increment.
    if body.len() != 2 {
        return None;
    }
    let mut accum: Option<(Symbol, &'a Expr<'a>)> = None;
    let mut counter_inc = false;
    for s in body {
        match s {
            Stmt::Set { target, value } if *target == counter => {
                if is_counter_plus_one(value, counter) {
                    counter_inc = true;
                } else {
                    return None;
                }
            }
            Stmt::Set { target, value } => accum = Some((*target, value)),
            _ => return None,
        }
    }
    if !counter_inc {
        return None;
    }
    let (acc, acc_value) = accum?;

    // acc_value must be `(acc + counter) % p` with p a literal ≥ 1.
    let (add_expr, p) = match acc_value {
        Expr::BinaryOp {
            op: BinaryOpKind::Modulo,
            left,
            right,
        } => match &**right {
            Expr::Literal(Literal::Number(n)) if *n >= 1 => (&**left, *n),
            _ => return None,
        },
        _ => return None,
    };
    // add_expr must be `acc + counter` (the addend is the counter, which is
    // non-negative throughout, and is not the accumulator).
    match add_expr {
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            let acc_lhs = matches!(&**left, Expr::Identifier(s) if *s == acc);
            let counter_rhs = matches!(&**right, Expr::Identifier(s) if *s == counter);
            if !(acc_lhs && counter_rhs) {
                return None;
            }
        }
        _ => return None,
    }

    // Non-negative seeds make truncated remainder equal mathematical mod.
    if find_init_value(preceding, acc)? < 0 {
        return None;
    }
    if find_init_value(preceding, counter)? < 0 {
        return None;
    }

    // K_SAFE: largest `limit` for which (p-1) + K*limit still fits in i64.
    let k_safe = (i64::MAX - (p - 1)) / K;
    // The guard also needs a LOWER bound: `limit - (K-1)` in the chunk
    // condition must not underflow for a limit near `i64::MIN` (any bound
    // above `MIN + K` keeps every guarded-branch expression in range; loops
    // with such a limit never iterate, so the fallback is value-identical).
    let min_safe = i64::MIN + K;

    let acc_id = ea.alloc(Expr::Identifier(acc));
    let counter_id = ea.alloc(Expr::Identifier(counter));
    // One increment node per loop: the guarded loops' nodes are registered
    // as proven-raw below, the fallback's must NOT be — sharing one arena
    // node would leak the proof into the unguarded branch.
    let mk_inc =
        |ea: &'a Arena<Expr<'a>>| mk_binop(BinaryOpKind::Add, counter_id, mk_int(1, ea), ea);
    let inner_inc = mk_inc(ea);
    let remainder_inc = mk_inc(ea);
    let fallback_inc = mk_inc(ea);

    // Inner mod-free accumulation loop, run K times per chunk.
    let stop_sym = interner.intern("__defer_stop");
    let stop_id = ea.alloc(Expr::Identifier(stop_sym));
    let inner_add = mk_binop(BinaryOpKind::Add, acc_id, counter_id, ea);
    let inner_body = sa.alloc_slice(vec![
        Stmt::Set {
            target: acc,
            value: inner_add,
        },
        Stmt::Set {
            target: counter,
            value: inner_inc,
        },
    ]);
    let inner_while = Stmt::While {
        cond: mk_binop(BinaryOpKind::Lt, counter_id, stop_id, ea),
        body: inner_body,
        decreasing: None,
    };
    let stop_value = mk_binop(BinaryOpKind::Add, counter_id, mk_int(K, ea), ea);
    let chunk_body = sa.alloc_slice(vec![
        Stmt::Let {
            var: stop_sym,
            ty: None,
            value: stop_value,
            mutable: false,
        },
        inner_while,
        Stmt::Set {
            target: acc,
            value: mk_binop(BinaryOpKind::Modulo, acc_id, mk_int(p, ea), ea),
        },
    ]);
    let chunk_limit = mk_binop(BinaryOpKind::Subtract, limit_expr, mk_int(K - 1, ea), ea);
    let chunk_cmp = if inclusive {
        BinaryOpKind::LtEq
    } else {
        BinaryOpKind::Lt
    };
    let chunk_while = Stmt::While {
        cond: mk_binop(chunk_cmp, counter_id, chunk_limit, ea),
        body: chunk_body,
        decreasing: None,
    };

    // Remainder loop: a FRESH body (same shape as the source) so its nodes
    // can carry the guarded-branch proof; the fallback keeps the original
    // `acc_value` nodes, unproven.
    let remainder_add = mk_binop(BinaryOpKind::Add, acc_id, counter_id, ea);
    let remainder_val = mk_binop(BinaryOpKind::Modulo, remainder_add, mk_int(p, ea), ea);
    let remainder_body = sa.alloc_slice(vec![
        Stmt::Set {
            target: acc,
            value: remainder_val,
        },
        Stmt::Set {
            target: counter,
            value: remainder_inc,
        },
    ]);
    let fallback_body = sa.alloc_slice(vec![
        Stmt::Set {
            target: acc,
            value: acc_value,
        },
        Stmt::Set {
            target: counter,
            value: fallback_inc,
        },
    ]);
    let remainder_while = Stmt::While {
        cond,
        body: remainder_body,
        decreasing: None,
    };
    let fallback_while = Stmt::While {
        cond,
        body: fallback_body,
        decreasing: None,
    };

    // The guard IS the overflow proof for every arithmetic node in the
    // guarded branch: `min_safe <= limit <= k_safe` with `acc`/`counter`
    // seeded non-negative bounds each of them far inside i64. Register the
    // constructed nodes so codegen lowers them RAW (the interval fixpoint
    // widens accumulators to ±∞ and cannot recover this).
    for e in [
        inner_add,
        inner_inc,
        remainder_add,
        remainder_inc,
        stop_value,
        chunk_limit,
    ] {
        super::facts::mark_expr(super::facts::Fact::RawIntOp, e);
    }

    let then_block = sa.alloc_slice(vec![chunk_while, remainder_while]);
    let else_block = sa.alloc_slice(vec![fallback_while]);
    let guard = mk_binop(
        BinaryOpKind::And,
        mk_binop(BinaryOpKind::GtEq, limit_expr, mk_int(min_safe, ea), ea),
        mk_binop(BinaryOpKind::LtEq, limit_expr, mk_int(k_safe, ea), ea),
        ea,
    );
    let guarded = Stmt::If {
        cond: guard,
        then_block,
        else_block: Some(else_block),
    };
    Some(vec![guarded])
}

pub fn closed_form_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    // Verify the canonical toReal/trunc casts once (body-matched, shadow-safe) so the loop
    // recognizer can treat `trunc(toReal(i))` as `i`.
    let kc = collect_known_casts(&stmts, interner);
    KNOWN_CASTS.with(|c| *c.borrow_mut() = kc);
    // O8b first (whole-program: it reads the SOLVED forms of sibling
    // functions), then the per-statement loop collapses.
    let stmts = solve_affine_recursions(stmts, expr_arena, stmt_arena, interner);
    let out = closed_form_block(stmts, expr_arena, stmt_arena, interner);
    KNOWN_CASTS.with(|c| *c.borrow_mut() = KnownCasts::default());
    AFFINE_FNS.with(|m| m.borrow_mut().clear());
    out
}

fn closed_form_block<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                let replaced =
                    try_replace_with_closed_form(cond, body, &result, expr_arena, stmt_arena);
                if let Some(replacement_stmts) = replaced {
                    result.extend(replacement_stmts);
                    continue;
                }

                if let Some(deferred) =
                    try_defer_modulus(cond, body, &result, expr_arena, stmt_arena, interner)
                {
                    result.extend(deferred);
                    continue;
                }

                let new_body = closed_form_block(body.to_vec(), expr_arena, stmt_arena, interner);
                result.push(Stmt::While {
                    cond,
                    body: stmt_arena.alloc_slice(new_body),
                    decreasing,
                });
            }
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
            } => {
                let new_body = closed_form_block(body.to_vec(), expr_arena, stmt_arena, interner);
                result.push(Stmt::FunctionDef {
                    name,
                    generics,
                    params,
                    body: stmt_arena.alloc_slice(new_body),
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                });
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let new_then =
                    closed_form_block(then_block.to_vec(), expr_arena, stmt_arena, interner);
                let new_else = else_block.map(|eb| {
                    let processed =
                        closed_form_block(eb.to_vec(), expr_arena, stmt_arena, interner);
                    let b: Block = stmt_arena.alloc_slice(processed);
                    b
                });
                result.push(Stmt::If {
                    cond,
                    then_block: stmt_arena.alloc_slice(new_then),
                    else_block: new_else,
                });
            }
            other => result.push(other),
        }
    }

    result
}

fn try_replace_with_closed_form<'a>(
    cond: &'a Expr<'a>,
    body: Block<'a>,
    preceding: &[Stmt<'a>],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Option<Vec<Stmt<'a>>> {
    let candidate = try_extract_candidate(body, cond)?;
    let (limit_expr, inclusive) = extract_while_limit(cond, candidate.counter)?;
    let init = find_init_value(preceding, candidate.accum)?;
    let start = find_init_value(preceding, candidate.counter)?;

    // For additive patterns (Sum, Count), only fire for start >= 1;
    // the for-range codegen peephole handles start=0 loops efficiently.
    // For multiplicative patterns (MulByTwo), allow start=0 because
    // the loop body cannot be eliminated by the for-range peephole.
    match candidate.pattern {
        AccumPattern::MulByTwo => {
            if start < 0 {
                return None;
            }
        }
        _ => {
            if start < 1 {
                return None;
            }
        }
    }

    // MulByTwo: a `<<` on a runtime count wraps (or is UB) past 63 bits,
    // exactly where the tower's un-collapsed loop promotes to BigInt. A
    // literal limit folds to the exact literal; a runtime limit emits a
    // SHIFT-SAFE guarded fast path (`init << count`, provably in i64) whose
    // fallback is the original loop — exact on every tier, loop-free where
    // it matters (the binary_trees iteration pattern).
    if matches!(candidate.pattern, AccumPattern::MulByTwo) {
        if init < 0 {
            return None;
        }
        // Largest count keeping `init << count` in i64.
        let mut shift_max: i64 = -1;
        for k in 0..=62u32 {
            match (init as i128).checked_shl(k) {
                Some(v) if v <= i64::MAX as i128 => shift_max = k as i64,
                _ => break,
            }
        }
        if shift_max < 0 {
            return None;
        }
        if let Expr::Literal(Literal::Number(limit)) = limit_expr {
            let count = (*limit as i128) - (start as i128) + if inclusive { 1 } else { 0 };
            if !(0..=shift_max as i128).contains(&count) {
                return None;
            }
            let exact = i64::try_from((init as i128) << count).ok()?;
            let guard_cond = if inclusive {
                mk_binop(
                    BinaryOpKind::GtEq,
                    limit_expr,
                    mk_int(start, expr_arena),
                    expr_arena,
                )
            } else {
                mk_binop(
                    BinaryOpKind::Gt,
                    limit_expr,
                    mk_int(start, expr_arena),
                    expr_arena,
                )
            };
            let counter_final = if inclusive {
                mk_binop(
                    BinaryOpKind::Add,
                    limit_expr,
                    mk_int(1, expr_arena),
                    expr_arena,
                )
            } else {
                limit_expr
            };
            let body_stmts = vec![
                Stmt::Set {
                    target: candidate.accum,
                    value: mk_int(exact, expr_arena),
                },
                Stmt::Set {
                    target: candidate.counter,
                    value: counter_final,
                },
            ];
            return Some(vec![Stmt::If {
                cond: guard_cond,
                then_block: stmt_arena.alloc_slice(body_stmts),
                else_block: None,
            }]);
        }
        // Runtime limit: fire only when the limit is a SIMPLE variable (`while p < n`),
        // whose count `n - start` lowers to an i64 shift amount. A COMPOUND Int limit
        // (binary_trees' `maxDepth - depth + minDepth`) lowers to a `LogosInt`, and
        // `1 << LogosInt` has no `Shl` impl; narrowing it soundly would need a fresh raw
        // copy that must not share nodes with the exact version guard (an intermediate
        // overflow in a shared raw `limit` would let the guard wrongly admit the fast
        // branch), and it isn't worth that for a log-sized doubling loop — compound
        // limits fall back to the exact loop. (binary_trees' hot speedup is the
        // affine-recursion closed form on makeCheck, not this counter.)
        if !matches!(limit_expr, Expr::Identifier(_)) {
            return None;
        }
        let adj = if inclusive { 1i64 } else { 0 };
        let v_max = shift_max.checked_add(start)?.checked_sub(adj)?;
        let count_expr = {
            let diff = mk_binop(
                BinaryOpKind::Subtract,
                limit_expr,
                mk_int(start, expr_arena),
                expr_arena,
            );
            if inclusive {
                mk_binop(BinaryOpKind::Add, diff, mk_int(1, expr_arena), expr_arena)
            } else {
                diff
            }
        };
        let shifted = mk_binop(
            BinaryOpKind::Shl,
            mk_int(init, expr_arena),
            count_expr,
            expr_arena,
        );
        let guard = mk_binop(
            BinaryOpKind::And,
            mk_binop(
                BinaryOpKind::GtEq,
                limit_expr,
                mk_int(start, expr_arena),
                expr_arena,
            ),
            mk_binop(
                BinaryOpKind::LtEq,
                limit_expr,
                mk_int(v_max, expr_arena),
                expr_arena,
            ),
            expr_arena,
        );
        let counter_final = if inclusive {
            mk_binop(
                BinaryOpKind::Add,
                limit_expr,
                mk_int(1, expr_arena),
                expr_arena,
            )
        } else {
            limit_expr
        };
        let fast = vec![
            Stmt::Set {
                target: candidate.accum,
                value: shifted,
            },
            Stmt::Set {
                target: candidate.counter,
                value: counter_final,
            },
        ];
        let fallback = vec![Stmt::While {
            cond,
            body: stmt_arena.alloc_slice(body.to_vec()),
            decreasing: None,
        }];
        return Some(vec![Stmt::If {
            cond: guard,
            then_block: stmt_arena.alloc_slice(fast),
            else_block: Some(stmt_arena.alloc_slice(fallback)),
        }]);
    }
    let formula = build_formula(
        &candidate.pattern,
        init,
        start,
        limit_expr,
        inclusive,
        expr_arena,
    );

    // Build: If limit >= start (for inclusive) or limit > start (for exclusive):
    //   Set accum to formula.
    //   Set counter to limit + 1 (inclusive) or limit (exclusive).
    let guard_cond = if inclusive {
        expr_arena.alloc(Expr::BinaryOp {
            op: BinaryOpKind::GtEq,
            left: limit_expr,
            right: mk_int(start, expr_arena),
        })
    } else {
        expr_arena.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Gt,
            left: limit_expr,
            right: mk_int(start, expr_arena),
        })
    };

    let counter_final = if inclusive {
        mk_binop(
            BinaryOpKind::Add,
            limit_expr,
            mk_int(1, expr_arena),
            expr_arena,
        )
    } else {
        limit_expr
    };

    let body_stmts = vec![
        Stmt::Set {
            target: candidate.accum,
            value: formula,
        },
        Stmt::Set {
            target: candidate.counter,
            value: counter_final,
        },
    ];

    let guarded = Stmt::If {
        cond: guard_cond,
        then_block: stmt_arena.alloc_slice(body_stmts),
        else_block: None,
    };

    Some(vec![guarded])
}

// ===========================================================================
// O8b — affine-recursion closed forms (the specialized Ackermann tower)
// ===========================================================================

/// The solved form of a unary Int function.
#[derive(Clone, Copy)]
enum SolvedForm {
    /// `f(n) = a·n + b`
    Affine { a: i128, b: i128 },
    /// `f(n) = scale·2^n − t`
    Geo2 { scale: i128, t: i128 },
}

/// O8b — solve the self-recursions the specializer leaves behind
/// (`f(0) = C; f(n) = g(f(n−1))` with `g` itself already solved affine, plus
/// direct affine bodies) and rewrite each solved RECURSIVE function as a
/// version-guarded closed form over its original body:
///
/// ```text
/// If n >= 0 and n <= N_SAFE: Return <closed form>.   # ops proven-raw
/// <original recursive body>                          # exact fallback
/// ```
///
/// `N_SAFE` is the largest argument whose closed-form result provably fits
/// i64, so every guarded-branch op is registered proven-raw; outside the
/// guard the original body runs bit for bit — including the exact promotion
/// and the divergence-on-negatives the recursion defines. LLVM once derived
/// these forms itself from raw i64 ops; exact-Int checked arithmetic blocks
/// its recursion reasoning, so the derivation lives HERE, where the proof
/// obligations are explicit and the fallback is the original semantics.
fn solve_affine_recursions<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Vec<Stmt<'a>> {
    use std::collections::HashMap;

    let is_int =
        |ty: &TypeExpr| matches!(ty, TypeExpr::Primitive(s) if interner.resolve(*s) == "Int");

    let mut solved: HashMap<Symbol, SolvedForm> = HashMap::new();
    // Fixpoint: towers solve bottom-up (`s0_0 → s0_1 → …`).
    loop {
        let mut changed = false;
        for stmt in &stmts {
            let Stmt::FunctionDef {
                name,
                params,
                body,
                return_type,
                is_native: false,
                ..
            } = stmt
            else {
                continue;
            };
            if solved.contains_key(name) || params.len() != 1 {
                continue;
            }
            if !is_int(params[0].1) || !return_type.is_some_and(|t| is_int(t)) {
                continue;
            }
            let p = params[0].0;
            let form = direct_affine_form(body, p)
                .or_else(|| recursive_solved_form(body, *name, p, &solved));
            if let Some(f) = form {
                solved.insert(*name, f);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    if solved.is_empty() {
        return stmts;
    }

    stmts
        .into_iter()
        .map(|stmt| match stmt {
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
            } if !is_native && params.len() == 1 => {
                let p = params[0].0;
                // Only RECURSIVE solved bodies get the guarded rewrite — a
                // direct affine body is already its own closed form.
                let new_body = solved
                    .get(&name)
                    .copied()
                    .filter(|_| base_and_wrap(body, p).is_some())
                    .and_then(|f| guarded_closed_body(f, p, body, ea, sa));
                if new_body.is_some() {
                    super::mark_fired(Opt::ClosedForm);
                }
                Stmt::FunctionDef {
                    name,
                    generics,
                    params,
                    body: new_body.unwrap_or(body),
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                }
            }
            other => other,
        })
        .collect()
}

/// Body is exactly `Return <affine of p>` — solved for the chain, untouched.
fn direct_affine_form(body: &[Stmt], p: Symbol) -> Option<SolvedForm> {
    match body {
        [Stmt::Return { value: Some(e) }] => {
            let (a, b) = affine_of(e, p)?;
            Some(SolvedForm::Affine { a, b })
        }
        _ => None,
    }
}

/// `e` as `a·p + b`, exactly.
fn affine_of(e: &Expr, p: Symbol) -> Option<(i128, i128)> {
    match e {
        Expr::Identifier(s) if *s == p => Some((1, 0)),
        Expr::Literal(Literal::Number(k)) => Some((0, *k as i128)),
        Expr::BinaryOp { op, left, right } => {
            let (a1, b1) = affine_of(left, p)?;
            let (a2, b2) = affine_of(right, p)?;
            match op {
                BinaryOpKind::Add => Some((a1.checked_add(a2)?, b1.checked_add(b2)?)),
                BinaryOpKind::Subtract => Some((a1.checked_sub(a2)?, b1.checked_sub(b2)?)),
                BinaryOpKind::Multiply if a1 == 0 => {
                    Some((b1.checked_mul(a2)?, b1.checked_mul(b2)?))
                }
                BinaryOpKind::Multiply if a2 == 0 => {
                    Some((a1.checked_mul(b2)?, b1.checked_mul(b2)?))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Body is `If p == 0: Return C.` + `Return <wrap>` — the recursion shape.
fn base_and_wrap<'a>(body: &'a [Stmt<'a>], p: Symbol) -> Option<(i128, &'a Expr<'a>)> {
    match body {
        [Stmt::If {
            cond,
            then_block,
            else_block: None,
        }, Stmt::Return { value: Some(wrap) }] => {
            if !is_eq_zero(cond, p) {
                return None;
            }
            match then_block {
                [Stmt::Return {
                    value: Some(Expr::Literal(Literal::Number(c))),
                }] => Some((*c as i128, wrap)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn is_eq_zero(cond: &Expr, p: Symbol) -> bool {
    match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::Eq,
            left,
            right,
        } => {
            (matches!(left, Expr::Identifier(s) if *s == p)
                && matches!(right, Expr::Literal(Literal::Number(0))))
                || (matches!(right, Expr::Identifier(s) if *s == p)
                    && matches!(left, Expr::Literal(Literal::Number(0))))
        }
        _ => false,
    }
}

/// `SELF(p − 1)`, exactly.
fn is_self_dec_call(e: &Expr, selfn: Symbol, p: Symbol) -> bool {
    let Expr::Call { function, args } = e else {
        return false;
    };
    if *function != selfn || args.len() != 1 {
        return false;
    }
    let Expr::BinaryOp {
        op: BinaryOpKind::Subtract,
        left,
        right,
    } = args[0]
    else {
        return false;
    };
    matches!(left, Expr::Identifier(s) if *s == p)
        && matches!(right, Expr::Literal(Literal::Number(1)))
}

/// The recursion's step as an affine map of `SELF(p−1)`: bare `SELF(p−1)` is
/// the identity; `g(SELF(p−1))` uses `g`'s solved affine form.
fn wrap_as_affine_of_self(
    wrap: &Expr,
    selfn: Symbol,
    p: Symbol,
    solved: &std::collections::HashMap<Symbol, SolvedForm>,
) -> Option<(i128, i128)> {
    if is_self_dec_call(wrap, selfn, p) {
        return Some((1, 0));
    }
    let Expr::Call { function, args } = wrap else {
        return None;
    };
    if args.len() != 1 || !is_self_dec_call(args[0], selfn, p) {
        return None;
    }
    match solved.get(function)? {
        SolvedForm::Affine { a, b } => Some((*a, *b)),
        SolvedForm::Geo2 { .. } => None,
    }
}

/// `f(0) = C; f(n) = step(f(n−1))` with `step(x) = pc·x + q`:
/// `pc == 1 → f(n) = q·n + C`; `pc == 2 → f(n) = (C+q)·2^n − q`.
fn recursive_solved_form(
    body: &[Stmt],
    selfn: Symbol,
    p: Symbol,
    solved: &std::collections::HashMap<Symbol, SolvedForm>,
) -> Option<SolvedForm> {
    let (c0, wrap) = base_and_wrap(body, p)?;
    let (pc, q) = wrap_as_affine_of_self(wrap, selfn, p, solved)?;
    match pc {
        1 => Some(SolvedForm::Affine { a: q, b: c0 }),
        2 => Some(SolvedForm::Geo2 {
            scale: c0.checked_add(q)?,
            t: q,
        }),
        _ => None,
    }
}

/// Build `If 0 <= n <= N_SAFE: Return <closed>.` + the original body. The
/// constructed arithmetic is registered proven-raw (the guard is its proof).
fn guarded_closed_body<'a>(
    form: SolvedForm,
    p: Symbol,
    original: Block<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Option<Block<'a>> {
    let n_id = ea.alloc(Expr::Identifier(p));
    let (n_safe, formula): (i128, &Expr) = match form {
        SolvedForm::Affine { a, b } => {
            if a <= 0 || b < 0 || a > i64::MAX as i128 || b > i64::MAX as i128 {
                return None;
            }
            let n_safe = (i64::MAX as i128 - b) / a;
            let scaled: &Expr = if a == 1 {
                n_id
            } else {
                let m = mk_binop(BinaryOpKind::Multiply, mk_int(a as i64, ea), n_id, ea);
                super::facts::mark_expr(super::facts::Fact::RawIntOp, m);
                m
            };
            let expr = if b == 0 {
                scaled
            } else {
                let s = mk_binop(BinaryOpKind::Add, scaled, mk_int(b as i64, ea), ea);
                super::facts::mark_expr(super::facts::Fact::RawIntOp, s);
                s
            };
            (n_safe, expr)
        }
        SolvedForm::Geo2 { scale, t } => {
            if scale <= 0 || t < 0 || scale > i64::MAX as i128 || t > i64::MAX as i128 {
                return None;
            }
            // Largest n with scale·2^n itself in i64 (then − t certainly is).
            let mut n_safe: i128 = -1;
            for k in 0..=62u32 {
                let v = scale.checked_shl(k)?;
                if v <= i64::MAX as i128 {
                    n_safe = k as i128;
                } else {
                    break;
                }
            }
            if n_safe < 0 {
                return None;
            }
            let shl = mk_binop(BinaryOpKind::Shl, mk_int(scale as i64, ea), n_id, ea);
            let expr = if t == 0 {
                shl
            } else {
                let s = mk_binop(BinaryOpKind::Subtract, shl, mk_int(t as i64, ea), ea);
                super::facts::mark_expr(super::facts::Fact::RawIntOp, s);
                s
            };
            (n_safe, expr)
        }
    };
    if n_safe < 0 {
        return None;
    }
    let n_safe = n_safe.min(i64::MAX as i128) as i64;
    let guard = mk_binop(
        BinaryOpKind::And,
        mk_binop(BinaryOpKind::GtEq, n_id, mk_int(0, ea), ea),
        mk_binop(BinaryOpKind::LtEq, n_id, mk_int(n_safe, ea), ea),
        ea,
    );
    let then_block = sa.alloc_slice(vec![Stmt::Return {
        value: Some(formula),
    }]);
    let mut new_body = vec![Stmt::If {
        cond: guard,
        then_block,
        else_block: None,
    }];
    new_body.extend(original.iter().cloned());
    Some(sa.alloc_slice(new_body))
}
