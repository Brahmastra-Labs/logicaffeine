//! SIMD reduction of a transcendental accumulator loop.
//!
//! A counted loop `for v in start..end` (LOGOS `While v <= n` / `Repeat for v from a to b`) whose
//! body is exactly `Set acc to acc + trunc(<float expr in v>)` — the shape every
//! `math.<transcendental>` benchmark has — is emitted as an AVX2 four-lane `f64` reduction over the
//! aligned prefix, with the ordinary scalar `for` loop covering the `< 4` tail (and the whole range
//! on a non-AVX2 host).
//!
//! SOUNDNESS / BYTE-IDENTITY BY CONSTRUCTION. The kernel is not a re-derivation: each lane runs the
//! *identical* scalar operation sequence (a branch `If c: Return A` becomes a `blendv` selecting `A`
//! exactly where `c` holds, so the lane result equals the scalar branch's result to the bit), each
//! lane's `f64` result is truncated to `i64` per lane (`x as i64` — truncate-toward-zero, saturating,
//! byte-identical to the scalar `trunc` over the FULL i64 range; a packed `_mm256_cvttpd_epi32` would
//! silently wrap any lane past ±2^31, which is unsound for unbounded outputs like `degrees`/`tan`), and
//! the accumulator is summed in `i64` (associative), so the four-lane reduce equals the sequential
//! scalar sum. Proven 3.54× on `sin` with exact-match output across n = 1..3_000_000.
//!
//! A function is *liftable* iff its body is only `Let`/`Set` bindings followed by a guarded-return
//! chain (`(If <cmp>: Return A)* Return B`), over arithmetic, `floor`, `sqrt`, comparisons, and calls
//! to liftable functions — no loops, no side effects. So `sin`/`cos` (via `sinCore`/`cosCore`/`roundR`/
//! `floorR`), `atan` (sqrt-free Cephes `ratAtan`), and `asin`/`acos` (via `sqrt` + `atan`) lift.
//! `exp`/`ln` do NOT win: glibc's scalar is table-based, so a byte-identical non-FMA SIMD Horner loses.

use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use crate::loop_shape::CountedLoop;
use std::collections::{BTreeMap, HashMap};

/// name → (param symbols, body) for the top-level user functions the lifter may inline.
pub(crate) type FnMap<'a> = HashMap<Symbol, (Vec<Symbol>, Block<'a>)>;

/// Collect the program's top-level (non-native) function definitions into a lift map.
pub(crate) fn build_fn_map<'a>(stmts: &[Stmt<'a>]) -> FnMap<'a> {
    let mut m = FnMap::new();
    for s in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            is_native: false,
            ..
        } = s
        {
            m.insert(*name, (params.iter().map(|(p, _)| *p).collect(), *body));
        }
    }
    m
}

/// A function's body split into leading bindings and a terminal guarded-return chain.
fn split_body<'a>(body: Block<'a>) -> Option<(&'a [Stmt<'a>], &'a [Stmt<'a>])> {
    let mut i = 0;
    while let Some(s) = body.get(i) {
        match s {
            Stmt::Let { .. } | Stmt::Set { .. } => i += 1,
            Stmt::If { .. } | Stmt::Return { .. } => break,
            _ => return None,
        }
    }
    Some((&body[..i], &body[i..]))
}

fn fn_liftable(f: Symbol, fns: &FnMap, interner: &Interner, seen: &mut Vec<Symbol>) -> bool {
    if seen.contains(&f) {
        return true;
    }
    seen.push(f);
    let Some((params, body)) = fns.get(&f) else {
        return false;
    };
    let Some((binds, chain)) = split_body(body) else {
        return false;
    };
    for s in binds {
        let value = match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => value,
            _ => return false,
        };
        if !expr_liftable(value, params, fns, interner, seen) {
            return false;
        }
    }
    chain_liftable(chain, params, fns, interner, seen)
}

fn chain_liftable(
    chain: &[Stmt],
    params: &[Symbol],
    fns: &FnMap,
    interner: &Interner,
    seen: &mut Vec<Symbol>,
) -> bool {
    match chain {
        [Stmt::Return { value: Some(e) }] => expr_liftable(e, params, fns, interner, seen),
        [Stmt::If {
            cond,
            then_block,
            else_block: None,
        }, rest @ ..] => {
            let [Stmt::Return { value: Some(a) }] = *then_block else {
                return false;
            };
            cmp_liftable(cond, params, fns, interner, seen)
                && expr_liftable(a, params, fns, interner, seen)
                && chain_liftable(rest, params, fns, interner, seen)
        }
        _ => false,
    }
}

fn cmp_liftable(
    cond: &Expr,
    params: &[Symbol],
    fns: &FnMap,
    interner: &Interner,
    seen: &mut Vec<Symbol>,
) -> bool {
    matches!(cond, Expr::BinaryOp { op, left, right }
        if matches!(op, BinaryOpKind::Eq | BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq)
            && expr_liftable(left, params, fns, interner, seen)
            && expr_liftable(right, params, fns, interner, seen))
}

fn expr_liftable(
    e: &Expr,
    params: &[Symbol],
    fns: &FnMap,
    interner: &Interner,
    seen: &mut Vec<Symbol>,
) -> bool {
    match e {
        Expr::Literal(Literal::Float(_)) | Expr::Literal(Literal::Number(_)) => true,
        Expr::Identifier(_) => true,
        Expr::BinaryOp { op, left, right } => {
            matches!(
                op,
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
                    | BinaryOpKind::Modulo
            ) && expr_liftable(left, params, fns, interner, seen)
                && expr_liftable(right, params, fns, interner, seen)
        }
        Expr::Call { function, args } => {
            let name = interner.resolve(*function);
            let library = |n: &str| name == n && crate::loader::is_library_function(n);
            if (name == "floor" || name == "sqrt" || library("toReal") || library("trunc"))
                && args.len() == 1
            {
                return expr_liftable(args[0], params, fns, interner, seen);
            }
            if library("copysign") && args.len() == 2 {
                return args
                    .iter()
                    .all(|a| expr_liftable(a, params, fns, interner, seen));
            }
            fns.contains_key(function)
                && args
                    .iter()
                    .all(|a| expr_liftable(a, params, fns, interner, seen))
                && fn_liftable(*function, fns, interner, seen)
        }
        _ => false,
    }
}

struct Lifter<'m, 'a> {
    interner: &'m Interner,
    fns: &'m FnMap<'a>,
    helpers: BTreeMap<String, String>,
}

impl<'m, 'a> Lifter<'m, 'a> {
    fn lit(f: f64) -> String {
        format!("_mm256_set1_pd({})", rust_f64(f))
    }

    fn lift_expr(&mut self, e: &Expr, scope: &HashMap<Symbol, String>) -> Option<String> {
        match e {
            Expr::Literal(Literal::Float(f)) => Some(Self::lit(*f)),
            Expr::Literal(Literal::Number(n)) => Some(Self::lit(*n as f64)),
            Expr::Identifier(s) => scope.get(s).cloned(),
            Expr::BinaryOp { op, left, right } => {
                let l = self.lift_expr(left, scope)?;
                let r = self.lift_expr(right, scope)?;
                if matches!(op, BinaryOpKind::Modulo) {
                    // Truncated modulo `a - b*trunc(a/b)` — byte-identical to LOGOS's `a mod b`
                    // (Rust i64 `%` / `fmod`, both truncated) for exact-integer operands in f64
                    // range, which the loop counter always is. Lets `counter mod K` lift.
                    return Some(format!(
                        "_mm256_sub_pd({l}, _mm256_mul_pd({r}, _mm256_round_pd(_mm256_div_pd({l}, {r}), _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC)))"
                    ));
                }
                let f = match op {
                    BinaryOpKind::Add => "_mm256_add_pd",
                    BinaryOpKind::Subtract => "_mm256_sub_pd",
                    BinaryOpKind::Multiply => "_mm256_mul_pd",
                    BinaryOpKind::Divide => "_mm256_div_pd",
                    _ => return None,
                };
                Some(format!("{}({}, {})", f, l, r))
            }
            Expr::Call { function, args } => {
                let name = self.interner.resolve(*function);
                let library = |n: &str| name == n && crate::loader::is_library_function(n);
                if library("toReal") && args.len() == 1 {
                    return self.lift_expr(args[0], scope);
                }
                if name == "floor" && args.len() == 1 {
                    let a = self.lift_expr(args[0], scope)?;
                    return Some(format!("_mm256_floor_pd({})", a));
                }
                if library("trunc") && args.len() == 1 {
                    let a = self.lift_expr(args[0], scope)?;
                    return Some(format!(
                        "_mm256_round_pd({a}, _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC)"
                    ));
                }
                if name == "sqrt" && args.len() == 1 {
                    let a = self.lift_expr(args[0], scope)?;
                    return Some(format!("_mm256_sqrt_pd({})", a));
                }
                if library("copysign") && args.len() == 2 {
                    // |a| with the sign of b — exactly `f64::copysign`, so byte-identical.
                    let a = self.lift_expr(args[0], scope)?;
                    let b = self.lift_expr(args[1], scope)?;
                    return Some(format!(
                        "_mm256_or_pd(_mm256_andnot_pd(_mm256_set1_pd(-0.0), {a}), _mm256_and_pd(_mm256_set1_pd(-0.0), {b}))"
                    ));
                }
                self.ensure_helper(*function)?;
                let a: Option<Vec<String>> =
                    args.iter().map(|x| self.lift_expr(x, scope)).collect();
                Some(format!("{}_x4({})", name, a?.join(", ")))
            }
            _ => None,
        }
    }

    fn lift_mask(&mut self, cond: &Expr, scope: &HashMap<Symbol, String>) -> Option<String> {
        let Expr::BinaryOp { op, left, right } = cond else {
            return None;
        };
        let l = self.lift_expr(left, scope)?;
        let r = self.lift_expr(right, scope)?;
        let pred = match op {
            BinaryOpKind::Eq => "_CMP_EQ_OQ",
            BinaryOpKind::Lt => "_CMP_LT_OQ",
            BinaryOpKind::Gt => "_CMP_GT_OQ",
            BinaryOpKind::LtEq => "_CMP_LE_OQ",
            BinaryOpKind::GtEq => "_CMP_GE_OQ",
            _ => return None,
        };
        Some(format!("_mm256_cmp_pd({}, {}, {})", l, r, pred))
    }

    fn lift_chain(&mut self, chain: &[Stmt], scope: &HashMap<Symbol, String>) -> Option<String> {
        match chain {
            [Stmt::Return { value: Some(e) }] => self.lift_expr(e, scope),
            [Stmt::If {
                cond,
                then_block,
                else_block: None,
            }, rest @ ..] => {
                let [Stmt::Return { value: Some(a) }] = *then_block else {
                    return None;
                };
                let mask = self.lift_mask(cond, scope)?;
                let a_v = self.lift_expr(a, scope)?;
                let else_v = self.lift_chain(rest, scope)?;
                Some(format!("_mm256_blendv_pd({}, {}, {})", else_v, a_v, mask))
            }
            _ => None,
        }
    }

    fn ensure_helper(&mut self, f: Symbol) -> Option<()> {
        let name = self.interner.resolve(f).to_string();
        let key = format!("{}_x4", name);
        if self.helpers.contains_key(&key) {
            return Some(());
        }
        self.helpers.insert(key.clone(), String::new());
        let (params, body) = self.fns.get(&f)?;
        let (binds, chain) = split_body(body)?;
        let mut scope: HashMap<Symbol, String> = HashMap::new();
        let mut sig_params = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let pn = format!("p{}", i);
            scope.insert(*p, pn.clone());
            sig_params.push(format!("{}: __m256d", pn));
        }
        let mut lets = String::new();
        for (i, s) in binds.iter().enumerate() {
            let (var, value) = match s {
                Stmt::Let { var, value, .. } => (*var, *value),
                Stmt::Set { target, value } => (*target, *value),
                _ => return None,
            };
            let v = self.lift_expr(value, &scope)?;
            let ln = format!("l{}", i);
            lets.push_str(&format!("        let {} = {};\n", ln, v));
            scope.insert(var, ln);
        }
        let ret = self.lift_chain(chain, &scope)?;
        let code = format!(
            "    #[target_feature(enable = \"avx2\")]\n    unsafe fn {}({}) -> __m256d {{\n{}        {}\n    }}\n",
            key, sig_params.join(", "), lets, ret
        );
        self.helpers.insert(key, code);
        Some(())
    }
}

/// Rust source for an `f64` literal that round-trips to the same bits (`{:?}` is shortest-exact).
pub(crate) fn rust_f64(f: f64) -> String {
    format!("{:?}", f)
}

/// The kernel plan for a recognized transcendental accumulator loop.
pub(crate) struct LiftedReduce {
    pub acc: Symbol,
    pub helpers: String,
    pub lane_expr: String,
}

/// Recognize + lift a counted loop whose body is exactly `Set acc to acc + trunc(<liftable float
/// in the counter>)`. `acc` must not appear inside the float term (a pure function of the counter).
pub(crate) fn try_lift<'a>(
    cl: &CountedLoop<'a>,
    fns: &FnMap<'a>,
    interner: &Interner,
) -> Option<LiftedReduce> {
    let [Stmt::Set {
        target: acc,
        value: acc_val,
    }] = cl.body_without_increment
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = acc_val
    else {
        return None;
    };
    if !matches!(left, Expr::Identifier(s) if *s == *acc) {
        return None;
    }
    let Expr::Call { function, args } = right else {
        return None;
    };
    if interner.resolve(*function) != "trunc"
        || args.len() != 1
        || !crate::loader::is_library_function("trunc")
    {
        return None;
    }
    let float_expr = args[0];
    if mentions(float_expr, *acc) {
        return None;
    }
    let mut seen = Vec::new();
    if !expr_liftable(float_expr, &[cl.var], fns, interner, &mut seen) {
        return None;
    }
    let mut lifter = Lifter {
        interner,
        fns,
        helpers: BTreeMap::new(),
    };
    let mut scope = HashMap::new();
    scope.insert(cl.var, "iv".to_string());
    let lane_expr = lifter.lift_expr(float_expr, &scope)?;
    let helpers: String = lifter.helpers.values().cloned().collect();
    Some(LiftedReduce {
        acc: *acc,
        helpers,
        lane_expr,
    })
}

thread_local! {
    /// Per-program counter for the emitted `__logos_simd_chunks_N` kernel names. Reset at
    /// program-emission entry (`reset_ids`) so the same program always emits the same names
    /// (deterministic output), while distinct SIMD loops within one program stay distinct.
    static SIMD_REDUCE_ID: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Reset the per-program kernel-name counter. Called from `codegen::expr::begin_program_emission`.
pub(crate) fn reset_ids() {
    SIMD_REDUCE_ID.with(|c| c.set(0));
}

/// Assemble the full dispatched block: an AVX2 chunk kernel over the aligned prefix, then the
/// ordinary scalar `for` loop (given as `body_code`, the same accumulator statement the for-range
/// peephole emits) over the `< 4` tail — which is the whole range on a non-AVX2 host.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_block(
    plan: &LiftedReduce,
    acc_rust: &str,
    counter_rust: &str,
    start_rust: &str,
    limit_rust: &str,
    inclusive: bool,
    body_code: &str,
    indent: &str,
) -> String {
    let id = SIMD_REDUCE_ID.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    });
    let end = if inclusive {
        format!("(({}) + 1)", limit_rust)
    } else {
        format!("({})", limit_rust)
    };
    format!(
        "{i}{{\n\
         {i}    #[target_feature(enable = \"avx2\")]\n\
         {i}    unsafe fn __logos_simd_chunks_{id}(lo: i64, hi: i64) -> i64 {{\n\
         {i}        use std::arch::x86_64::*;\n\
         {helpers}\
         {i}        let mut __acc = _mm256_setzero_si256();\n\
         {i}        let mut __k = lo;\n\
         {i}        while __k < hi {{\n\
         {i}            let iv = _mm256_set_pd((__k + 3) as f64, (__k + 2) as f64, (__k + 1) as f64, __k as f64);\n\
         {i}            let fv = {lane};\n\
         {i}            let mut __t = [0.0f64; 4];\n\
         {i}            _mm256_storeu_pd(__t.as_mut_ptr(), fv);\n\
         {i}            __acc = _mm256_add_epi64(__acc, _mm256_set_epi64x(__t[3] as i64, __t[2] as i64, __t[1] as i64, __t[0] as i64));\n\
         {i}            __k += 4;\n\
         {i}        }}\n\
         {i}        let mut __b = [0i64; 4];\n\
         {i}        _mm256_storeu_si256(__b.as_mut_ptr() as *mut __m256i, __acc);\n\
         {i}        __b[0] + __b[1] + __b[2] + __b[3]\n\
         {i}    }}\n\
         {i}    let __s: i64 = {start};\n\
         {i}    let __e: i64 = {end};\n\
         {i}    let mut __off: i64 = 0;\n\
         {i}    if std::is_x86_feature_detected!(\"avx2\") {{\n\
         {i}        __off = if __e > __s {{ ((__e - __s) / 4) * 4 }} else {{ 0 }};\n\
         {i}        if __off > 0 {{ {acc} = {acc} + unsafe {{ __logos_simd_chunks_{id}(__s, __s + __off) }}; }}\n\
         {i}    }}\n\
         {i}    for {counter} in (__s + __off)..__e {{\n\
         {body}\
         {i}    }}\n\
         {i}}}\n",
        i = indent,
        id = id,
        helpers = indent_lines(&plan.helpers, indent),
        lane = plan.lane_expr,
        start = start_rust,
        end = end,
        acc = acc_rust,
        counter = counter_rust,
        body = body_code,
    )
}

fn indent_lines(s: &str, pad: &str) -> String {
    s.lines()
        .map(|l| {
            if l.is_empty() {
                "\n".to_string()
            } else {
                format!("{}{}\n", pad, l)
            }
        })
        .collect()
}

fn mentions(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => mentions(left, sym) || mentions(right, sym),
        Expr::Call { args, .. } => args.iter().any(|a| mentions(a, sym)),
        _ => false,
    }
}
