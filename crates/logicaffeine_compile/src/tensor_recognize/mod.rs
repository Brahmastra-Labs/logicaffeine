//! Backend-agnostic tensor-op **recognition** — LOGOS AST -> typed op specs, shared by every codegen backend.
//!
//! The recognizers here read an ordinary (desugared) LOGOS program and lift it into a typed spec — a matmul, an
//! elementwise op, a reduction / fused-row normalization, or a SwiGLU map — or REFUSE (typed). They are
//! **backend-neutral**: the CUDA ([`crate::codegen_cuda`]), Tenstorrent ([`crate::codegen_tt`]), and RISC-V
//! ([`crate::codegen_riscv`]) tiers all consume the SAME spec and each emit their own kernel from it. The
//! per-backend EMITTERS (`lower_*_to_tensix`, the CUDA `emit_*` fns, ...) live in their backend module; only the
//! recognition + spec types are shared, and they live HERE rather than under any one backend.

use crate::intern::{Interner, Symbol};
use logicaffeine_language::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use std::collections::HashMap;

pub mod map;
pub mod qwen;

/// A recognized matmul: `C(M×N) = A(M×K) · B(K×N)`, row-major, over the named `Seq of Int` arrays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatmulSpec {
    pub m: usize,
    pub n: usize,
    pub k: usize,
    pub a: Symbol,
    pub b: Symbol,
    pub c: Symbol,
}

impl MatmulSpec {
    /// Output tiles high (32×32 Tensix tiles, ceil).
    pub fn mt(&self) -> usize {
        self.m.div_ceil(32)
    }
    /// Output tiles wide.
    pub fn nt(&self) -> usize {
        self.n.div_ceil(32)
    }
    /// Contraction tiles.
    pub fn kt(&self) -> usize {
        self.k.div_ceil(32)
    }
}

/// A recognized per-head matmul, `Y = X matmul W across H heads`: each head's slice of every row of `X` times its own
/// block of `W`'s rows. `X` is `m × (heads·a)`, `W` `(heads·a) × b`, `Y` `m × (heads·b)`, and
/// `Y[t][h·b + r] = Σ_e X[t][h·a + e]·W[h·a + e][r]` — a block-diagonal product, one `a × b` block per head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadMatmulSpec {
    pub x: Symbol,
    pub w: Symbol,
    pub out: Symbol,
    pub m: usize,
    pub heads: usize,
    pub a: usize,
    pub b: usize,
}

/// Why a program is not a recognizable tile-matmul — a typed refusal, never a miscompile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatmulReject {
    /// No `While` loop at the top of the scanned region.
    NoLoopNest,
    /// Fewer than three nested counted loops.
    NotTripleNested,
    /// No accumulating indexed store `Set item _ of C to (item _ of C) + …`.
    NoAccumulatingStore,
    /// An index expression is not affine in the induction variables.
    NonAffineIndex,
    /// The affine indices do not match the row-major GEMM layout (against the loop bounds).
    NotGemmLayout,
}

/// An affine form `Σ coeff·iv + cst` over induction/loop variables (keyed by their `Symbol`).
#[derive(Clone, Default, Debug, PartialEq, Eq)]
struct Affine {
    coeff: HashMap<Symbol, i64>,
    cst: i64,
}

impl Affine {
    fn constant(c: i64) -> Self {
        Affine {
            coeff: HashMap::new(),
            cst: c,
        }
    }
    fn var(s: Symbol) -> Self {
        let mut coeff = HashMap::new();
        coeff.insert(s, 1);
        Affine { coeff, cst: 0 }
    }
    fn add(&self, o: &Affine) -> Affine {
        let mut coeff = self.coeff.clone();
        for (k, v) in &o.coeff {
            *coeff.entry(*k).or_default() += v;
        }
        coeff.retain(|_, v| *v != 0);
        Affine {
            coeff,
            cst: self.cst + o.cst,
        }
    }
    fn sub(&self, o: &Affine) -> Affine {
        let mut coeff = self.coeff.clone();
        for (k, v) in &o.coeff {
            *coeff.entry(*k).or_default() -= v;
        }
        coeff.retain(|_, v| *v != 0);
        Affine {
            coeff,
            cst: self.cst - o.cst,
        }
    }
    fn scale(&self, f: i64) -> Affine {
        Affine {
            coeff: self.coeff.iter().map(|(k, v)| (*k, v * f)).collect(),
            cst: self.cst * f,
        }
    }
    fn is_const(&self) -> bool {
        self.coeff.is_empty()
    }
    fn coeff_of(&self, s: Symbol) -> i64 {
        *self.coeff.get(&s).unwrap_or(&0)
    }
}

/// Evaluate an expression to an affine form over the induction variables, resolving top-level integer
/// constants and inner `Let`-bound index expressions. Returns `None` if the expression is non-affine
/// (e.g. iv×iv) or references an unsupported construct.
fn eval_affine(
    e: &Expr,
    consts: &HashMap<Symbol, i64>,
    lets: &HashMap<Symbol, Affine>,
) -> Option<Affine> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(Affine::constant(*n)),
        Expr::Identifier(s) => {
            if let Some(c) = consts.get(s) {
                Some(Affine::constant(*c))
            } else if let Some(a) = lets.get(s) {
                Some(a.clone())
            } else {
                Some(Affine::var(*s))
            }
        }
        Expr::BinaryOp { op, left, right } => {
            let l = eval_affine(left, consts, lets)?;
            let r = eval_affine(right, consts, lets)?;
            match op {
                BinaryOpKind::Add => Some(l.add(&r)),
                BinaryOpKind::Subtract => Some(l.sub(&r)),
                BinaryOpKind::Multiply => {
                    if l.is_const() {
                        Some(r.scale(l.cst))
                    } else if r.is_const() {
                        Some(l.scale(r.cst))
                    } else {
                        None // iv×iv — non-affine
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The first `While` in a block, as `(condition, body)`.
fn first_while<'a>(stmts: &'a [Stmt<'a>]) -> Option<(&'a Expr<'a>, &'a [Stmt<'a>])> {
    stmts.iter().find_map(|s| match s {
        Stmt::While { cond, body, .. } => Some((*cond, *body)),
        _ => None,
    })
}

/// A counted loop's `(induction variable, trip count)` from `While iv is less than BOUND` (or `at most`).
fn loop_iv_bound(cond: &Expr, consts: &HashMap<Symbol, i64>) -> Option<(Symbol, i64)> {
    let empty = HashMap::new();
    if let Expr::BinaryOp { op, left, right } = cond {
        if let Expr::Identifier(iv) = left {
            let b = eval_affine(right, consts, &empty)?;
            if b.is_const() {
                return match op {
                    BinaryOpKind::Lt => Some((*iv, b.cst)),
                    BinaryOpKind::LtEq => Some((*iv, b.cst + 1)),
                    _ => None,
                };
            }
        }
    }
    None
}

fn as_ident(e: &Expr) -> Option<Symbol> {
    if let Expr::Identifier(s) = e {
        Some(*s)
    } else {
        None
    }
}

fn as_index<'a>(e: &'a Expr<'a>) -> Option<(&'a Expr<'a>, &'a Expr<'a>)> {
    if let Expr::Index { collection, index } = e {
        Some((collection, index))
    } else {
        None
    }
}

fn as_binop<'a>(e: &'a Expr<'a>, want: BinaryOpKind) -> Option<(&'a Expr<'a>, &'a Expr<'a>)> {
    if let Expr::BinaryOp { op, left, right } = e {
        if *op == want {
            return Some((left, right));
        }
    }
    None
}

/// Every integer constant `Let x be <number>` anywhere in the tree, so loop bounds / index strides that
/// reference a named size (`n`) resolve regardless of scope.
fn collect_consts_rec(stmts: &[Stmt], out: &mut HashMap<Symbol, i64>) {
    for s in stmts {
        match s {
            // Only IMMUTABLE bindings are dimension constants; a `Let mutable i be 0` is a loop counter and
            // must remain a free induction variable, not get frozen to its initial value.
            Stmt::Let {
                var,
                value,
                mutable: false,
                ..
            } => {
                if let Expr::Literal(Literal::Number(n)) = value {
                    out.insert(*var, *n);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                collect_consts_rec(body, out)
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_consts_rec(then_block, out);
                if let Some(eb) = else_block {
                    collect_consts_rec(eb, out);
                }
            }
            _ => {}
        }
    }
}

fn reject_rank(r: &MatmulReject) -> u8 {
    match r {
        MatmulReject::NoLoopNest => 1,
        MatmulReject::NotTripleNested => 2,
        MatmulReject::NoAccumulatingStore => 3,
        MatmulReject::NonAffineIndex => 4,
        MatmulReject::NotGemmLayout => 5,
    }
}

/// Recognize a row-major GEMM idiom anywhere in a LOGOS statement list and extract its `MatmulSpec`.
///
/// Structure required: three nested counted `While` loops over induction variables `i, k, j`, with an
/// accumulating store `Set item _ of C to (item _ of C) + (item _ of A) * (item _ of B)` in the innermost
/// body. The three index expressions must be affine and match the row-major layout — verified against the
/// loop bounds — i.e. `C` indexed `i·N + j`, `A` indexed `i·K + k`, `B` indexed `k·N + j`. Loops that are
/// not the matmul nest (e.g. an array-build loop) are skipped; the search returns the most-specific refusal
/// if nothing matches.
pub fn recognize_matmul(stmts: &[Stmt], _interner: &Interner) -> Result<MatmulSpec, MatmulReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    search_matmul(stmts, &consts)
}

/// Recognize `Let Y be X matmul W across H heads` — the matmul's nest with a head loop between the row loop and the
/// contraction: `Y[t·(H·b) + h·b + r + 1] += X[t·(H·a) + h·a + e + 1] · W[(h·a + e)·b + r + 1]`, the loops `t`, `h`, `e`,
/// `r` over `m`, `H`, `a`, `b`.
pub fn recognize_head_matmul(stmts: &[Stmt]) -> Option<HeadMatmulSpec> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    search_head_matmul(stmts, &consts)
}

fn search_head_matmul(stmts: &[Stmt], consts: &HashMap<Symbol, i64>) -> Option<HeadMatmulSpec> {
    for s in stmts {
        let found = match s {
            Stmt::While { cond, body, .. } => {
                head_matmul_at(cond, body, consts).or_else(|| search_head_matmul(body, consts))
            }
            Stmt::Splice { body } => search_head_matmul(body, consts),
            Stmt::If { then_block, else_block, .. } => search_head_matmul(then_block, consts)
                .or_else(|| else_block.and_then(|eb| search_head_matmul(eb, consts))),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The per-head nest with the given `While` as its row loop.
fn head_matmul_at(cond_t: &Expr, body_t: &[Stmt], consts: &HashMap<Symbol, i64>) -> Option<HeadMatmulSpec> {
    let (t, m) = loop_iv_bound(cond_t, consts)?;
    let (cond_h, body_h) = first_while(body_t)?;
    let (h, heads) = loop_iv_bound(cond_h, consts)?;
    let (cond_e, body_e) = first_while(body_h)?;
    let (e, a) = loop_iv_bound(cond_e, consts)?;
    let (cond_r, body_r) = first_while(body_e)?;
    let (r, b) = loop_iv_bound(cond_r, consts)?;
    let none = HashMap::new();
    let (out, idx, value) = body_r.iter().find_map(|s| match s {
        Stmt::SetIndex { collection, index, value } => Some((as_ident(collection)?, *index, *value)),
        _ => None,
    })?;
    let y = eval_affine(idx, consts, &none)?;
    let (acc, prod) = as_binop(value, BinaryOpKind::Add)?;
    let (acc_c, acc_i) = as_index(acc)?;
    if as_ident(acc_c)? != out || eval_affine(acc_i, consts, &none)? != y {
        return None;
    }
    let (xr, wr) = as_binop(prod, BinaryOpKind::Multiply)?;
    let (xc, xi) = as_index(xr)?;
    let (wc, wi) = as_index(wr)?;
    let (x, w) = (as_ident(xc)?, as_ident(wc)?);
    let (xi, wi) = (eval_affine(xi, consts, &none)?, eval_affine(wi, consts, &none)?);
    let shaped = |f: &Affine, coeffs: [i64; 4]| {
        [t, h, e, r].iter().zip(coeffs).all(|(v, c)| f.coeff_of(*v) == c) && f.coeff.len() == coeffs.iter().filter(|c| **c != 0).count() && f.cst == 1
    };
    let ok = shaped(&y, [heads * b, b, 0, 1]) && shaped(&xi, [heads * a, a, 1, 0]) && shaped(&wi, [0, a * b, b, 1]);
    (ok && m >= 1 && heads >= 1 && a >= 1 && b >= 1).then(|| HeadMatmulSpec {
        x,
        w,
        out,
        m: m as usize,
        heads: heads as usize,
        a: a as usize,
        b: b as usize,
    })
}

/// How an array is FILLED by its build loop — recognized from `While iv is less than N: Push <expr> to xs`,
/// so a backend host can reproduce the source's data and a numerics claim can bound the operand range.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Fill {
    /// Every element is the same constant.
    Const(i64),
    /// `element[p] = (p % modulus) + shift` over the loop counter `p`.
    Pattern { modulus: i64, shift: i64 },
    /// A recognized base fill followed by constant element overrides `Set item i of xs to v` (0-based here).
    Overrides {
        base: Box<Fill>,
        sets: Vec<(usize, i64)>,
    },
    /// Every element is the same float constant (its bits, so the fill stays hashable).
    ConstF(u64),
    /// A float base constant (bits) followed by float element overrides (0-based index, value bits).
    FloatOverrides { base: u64, sets: Vec<(usize, u64)> },
    /// `element[p] = ((p % modulus) + shift) / divisor` over the loop counter `p`, a `Real` (`divisor` by its bits).
    RealPattern { modulus: i64, shift: i64, divisor: u64 },
    /// Not a recognized fill shape (data is external to the compiler).
    Unknown,
}

impl Fill {
    /// The float element overrides of a float fill, as values.
    pub fn float_sets(&self) -> Option<(f64, Vec<(usize, f64)>)> {
        match self {
            Fill::ConstF(b) => Some((f64::from_bits(*b), Vec::new())),
            Fill::FloatOverrides { base, sets } => Some((
                f64::from_bits(*base),
                sets.iter().map(|(i, b)| (*i, f64::from_bits(*b))).collect(),
            )),
            _ => None,
        }
    }

    /// Element `p` of a fill that is the same at every length — `None` for an override list, whose length the fill
    /// does not carry past its last override, and for data the compiler cannot see.
    pub fn element(&self, p: i64) -> Option<Scalar> {
        match self {
            Fill::Const(c) => Some(Scalar::Int(*c)),
            Fill::Pattern { modulus, shift } => Some(Scalar::Int(p % modulus + shift)),
            Fill::ConstF(b) => Some(Scalar::Real(f64::from_bits(*b))),
            Fill::RealPattern { modulus, shift, divisor } => {
                Some(Scalar::Real((p % modulus + shift) as f64 / f64::from_bits(*divisor)))
            }
            Fill::Overrides { .. } | Fill::FloatOverrides { .. } | Fill::Unknown => None,
        }
    }
}

/// Recognize the fill of `tensor` anywhere in the statement list: the FIRST loop whose body pushes onto it.
pub fn recognize_fill(stmts: &[Stmt], tensor: Symbol) -> Fill {
    Fills::of(stmts).fill(tensor)
}

/// Every tensor's fill in a statement list at once ([`recognize_fill`] of each), from one walk of the list: for each
/// tensor the first loop pushing onto it — a loop's own pushes met before the loops nested in it, as `recognize_fill`
/// meets them — and its top-level literal writes, in order.
pub struct Fills<'s, 'a> {
    first: std::collections::HashMap<Symbol, Fill>,
    writes: std::collections::HashMap<Symbol, Vec<&'s Stmt<'a>>>,
}

impl<'s, 'a> Fills<'s, 'a> {
    pub fn of(stmts: &'s [Stmt<'a>]) -> Self {
        fn scan(stmts: &[Stmt], first: &mut std::collections::HashMap<Symbol, Fill>) {
            for s in stmts {
                match s {
                    Stmt::While { cond, body, .. } => {
                        let iv = as_binop_any(cond).and_then(|(l, _)| as_ident(l));
                        for b in body.iter() {
                            if let Stmt::Push { value, collection } = b {
                                if let Some(t) = as_ident(collection) {
                                    first.entry(t).or_insert_with(|| fill_of(value, iv));
                                }
                            }
                        }
                        scan(body, first);
                    }
                    Stmt::If { then_block, else_block, .. } => {
                        scan(then_block, first);
                        if let Some(eb) = else_block {
                            scan(eb, first);
                        }
                    }
                    Stmt::Splice { body } => scan(body, first),
                    _ => {}
                }
            }
        }
        let mut first = std::collections::HashMap::new();
        scan(stmts, &mut first);
        let mut writes: std::collections::HashMap<Symbol, Vec<&Stmt>> = std::collections::HashMap::new();
        for s in stmts {
            if let Stmt::SetIndex { collection, .. } = s {
                if let Some(t) = as_ident(collection) {
                    writes.entry(t).or_default().push(s);
                }
            }
        }
        Fills { first, writes }
    }

    /// `tensor`'s fill: its first fill loop's, folded with its literal writes.
    pub fn fill(&self, tensor: Symbol) -> Fill {
        let base = self.first.get(&tensor).cloned().unwrap_or(Fill::Unknown);
        if base == Fill::Unknown {
            return base;
        }
        let writes: &[&Stmt] = self.writes.get(&tensor).map_or(&[], Vec::as_slice);
        if !writes.is_empty() && matches!(base, Fill::RealPattern { .. }) {
            return Fill::Unknown;
        }
        let mut sets = Vec::new();
        let mut float_sets: Vec<(usize, u64)> = Vec::new();
        for s in writes {
            if let Stmt::SetIndex {
                index: Expr::Literal(Literal::Number(i)),
                value,
                ..
            } = s
            {
                if *i >= 1 {
                    match value {
                        Expr::Literal(Literal::Number(v)) => sets.push((*i as usize - 1, *v)),
                        Expr::Literal(Literal::Float(v)) => float_sets.push((*i as usize - 1, v.to_bits())),
                        _ => {}
                    }
                }
            }
        }
        let float_base = match &base {
            Fill::ConstF(b) => Some(*b),
            Fill::Const(c) if !float_sets.is_empty() => Some((*c as f64).to_bits()),
            _ => None,
        };
        if let Some(fb) = float_base {
            let mut all: Vec<(usize, u64)> = sets
                .iter()
                .map(|(i, v)| (*i, (*v as f64).to_bits()))
                .collect();
            all.extend(float_sets);
            return if all.is_empty() {
                Fill::ConstF(fb)
            } else {
                Fill::FloatOverrides {
                    base: fb,
                    sets: all,
                }
            };
        }
        if sets.is_empty() {
            base
        } else {
            Fill::Overrides {
                base: Box::new(base),
                sets,
            }
        }
    }
}

fn as_binop_any<'a>(e: &'a Expr<'a>) -> Option<(&'a Expr<'a>, &'a Expr<'a>)> {
    if let Expr::BinaryOp { left, right, .. } = e {
        Some((left, right))
    } else {
        None
    }
}

/// The fill an expression over the loop counter `iv` denotes: a number literal, `iv % m`, `(iv % m) ± c`, or either
/// of the last two over a `Real` divisor.
fn fill_of(value: &Expr, iv: Option<Symbol>) -> Fill {
    match value {
        Expr::BinaryOp {
            op: BinaryOpKind::Divide,
            left,
            right: Expr::Literal(Literal::Float(d)),
        } if d.is_finite() && *d != 0.0 => match fill_of(left, iv) {
            Fill::Pattern { modulus, shift } => Fill::RealPattern { modulus, shift, divisor: d.to_bits() },
            _ => Fill::Unknown,
        },
        Expr::Literal(Literal::Number(n)) => Fill::Const(*n),
        Expr::Literal(Literal::Float(f)) => Fill::ConstF(f.to_bits()),
        Expr::BinaryOp {
            op: BinaryOpKind::Modulo,
            left,
            right,
        } => match (as_ident(left), right) {
            (Some(v), Expr::Literal(Literal::Number(m))) if Some(v) == iv && *m > 0 => {
                Fill::Pattern {
                    modulus: *m,
                    shift: 0,
                }
            }
            _ => Fill::Unknown,
        },
        Expr::BinaryOp { op, left, right }
            if matches!(op, BinaryOpKind::Add | BinaryOpKind::Subtract) =>
        {
            match (fill_of(left, iv), right) {
                (Fill::Pattern { modulus, shift }, Expr::Literal(Literal::Number(c))) => {
                    let c = if *op == BinaryOpKind::Add { *c } else { -*c };
                    Fill::Pattern {
                        modulus,
                        shift: shift + c,
                    }
                }
                _ => Fill::Unknown,
            }
        }
        _ => Fill::Unknown,
    }
}

/// An elementwise tensor op — `C[p] = A[p] ⊕ B[p]` over a shared flat length.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum ElementwiseOp {
    Add,
    Mul,
}

impl ElementwiseOp {
    /// The C/CUDA infix operator token.
    pub fn c_op(self) -> &'static str {
        match self {
            ElementwiseOp::Add => "+",
            ElementwiseOp::Mul => "*",
        }
    }
}

/// ★ A BLOCK THE PROGRAM WROTE BY HAND IS NOT THE OP IT HOLDS. Every sugar desugars into a `Splice`, so a `Repeat` or a
/// `While` with a desugar inside is the program's own loop, running the op once a turn beside whatever else it holds,
/// and an `If` with a desugar or a loop inside runs it on one arm. Neither is lifted as the op — the recognizers would
/// find it inside and drop the rest — so the walk lists the block and the program is refused by naming it. A loop the
/// program wrote that IS an op (a counted loop nest of products) holds no desugar and is lifted as the op it is.
pub fn holds_an_op_by_hand(s: &Stmt) -> bool {
    fn holds(stmts: &[Stmt], loops: bool) -> bool {
        stmts.iter().any(|s| match s {
            Stmt::Splice { .. } => true,
            Stmt::While { body, .. } => loops || holds(body, loops),
            Stmt::Repeat { body, .. } => holds(body, loops),
            Stmt::If { then_block, else_block, .. } => holds(then_block, loops) || else_block.is_some_and(|e| holds(e, loops)),
            _ => false,
        })
    }
    match s {
        Stmt::If { then_block, else_block, .. } => holds(then_block, true) || else_block.is_some_and(|e| holds(e, true)),
        Stmt::Repeat { body, .. } | Stmt::While { body, .. } => holds(body, false),
        _ => false,
    }
}

/// The recognized shape of an elementwise tensor op: two source tensors `a`, `b`, a destination `c`, the
/// operator, and the shared element count `n`.
#[derive(Debug, Clone, Copy)]
pub struct ElementwiseSpec {
    pub a: Symbol,
    pub b: Symbol,
    pub c: Symbol,
    pub op: ElementwiseOp,
    pub n: usize,
    /// `B` is one row of this width, combined with every row of `A` (a bias over a block); `None` when the two
    /// operands have one shape.
    pub row: Option<usize>,
}

/// Why a program is not a recognizable elementwise op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementwiseReject {
    NotElementwise,
}

/// Recognize an elementwise idiom `Push (item p of A) ⊕ (item p of B) to C` inside a counted loop — the
/// exact shape the `plus`/`hadamard` sugar desugars to — anywhere in the statement list (through `Splice`).
pub fn recognize_elementwise(
    stmts: &[Stmt],
    _interner: &Interner,
) -> Result<ElementwiseSpec, ElementwiseReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    search_elementwise(stmts, &consts, None).ok_or(ElementwiseReject::NotElementwise)
}

fn search_elementwise(
    stmts: &[Stmt],
    consts: &HashMap<Symbol, i64>,
    binder: Option<Symbol>,
) -> Option<ElementwiseSpec> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                if let Some(spec) = try_elementwise_at(cond, body, consts, binder) {
                    return Some(spec);
                }
                if let Some(spec) = search_elementwise(body, consts, binder) {
                    return Some(spec);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if let Some(spec) = search_elementwise(then_block, consts, binder) {
                    return Some(spec);
                }
                if let Some(eb) = else_block {
                    if let Some(spec) = search_elementwise(eb, consts, binder) {
                        return Some(spec);
                    }
                }
            }
            Stmt::Splice { body } => {
                if let Some(spec) = search_elementwise(body, consts, splice_binder(body).or(binder))
                {
                    return Some(spec);
                }
            }
            _ => {}
        }
    }
    None
}

/// `1..=N` counted loop over `iv` → `(iv, N)`; the elementwise sugar counts `p` from 1 with `p <= N`.
fn ew_loop_bound(cond: &Expr, consts: &HashMap<Symbol, i64>) -> Option<(Symbol, i64)> {
    let empty = HashMap::new();
    if let Expr::BinaryOp { op, left, right } = cond {
        if let Expr::Identifier(iv) = left {
            let b = eval_affine(right, consts, &empty)?;
            if b.is_const() {
                return match op {
                    BinaryOpKind::LtEq => Some((*iv, b.cst)), // p <= N (p from 1) → N elements
                    BinaryOpKind::Lt => Some((*iv, b.cst - 1)), // p <  N (p from 1) → N-1 elements
                    _ => None,
                };
            }
        }
    }
    None
}

fn try_elementwise_at(
    cond: &Expr,
    body: &[Stmt],
    consts: &HashMap<Symbol, i64>,
    binder: Option<Symbol>,
) -> Option<ElementwiseSpec> {
    let (iv, n) = ew_loop_bound(cond, consts)?;
    if n <= 0 {
        return None;
    }
    for st in body {
        let Stmt::Push { value, collection } = st else {
            continue;
        };
        let c = as_ident(collection)?;
        if !pushes_into_binder(c, binder) {
            continue;
        }
        let Expr::BinaryOp { op, left, right } = value else {
            continue;
        };
        let ewop = match op {
            BinaryOpKind::Add => ElementwiseOp::Add,
            BinaryOpKind::Multiply => ElementwiseOp::Mul,
            _ => continue,
        };
        let (a_coll, a_idx) = as_index(left)?;
        let (b_coll, b_idx) = as_index(right)?;
        // `A` reads at the loop variable; `B` at it too (one shape), or at its column `((p − 1) mod W) + 1` (a row).
        let row = match as_ident(b_idx) {
            Some(i) if i == iv => Some(None),
            _ => broadcast_width(b_idx, iv).map(Some),
        };
        if let (Some(row), true) = (row, as_ident(a_idx) == Some(iv)) {
            return Some(ElementwiseSpec {
                a: as_ident(a_coll)?,
                b: as_ident(b_coll)?,
                c,
                op: ewop,
                n: n as usize,
                row,
            });
        }
    }
    None
}

/// `((iv − 1) mod W) + 1` — the column of a row `W` wide that element `iv` (1-based) of a block falls in. Returns `W`.
fn broadcast_width(e: &Expr, iv: Symbol) -> Option<usize> {
    let (m, one) = as_binop(e, BinaryOpKind::Add)?;
    let (shifted, w) = as_binop(m, BinaryOpKind::Modulo)?;
    let (i, one2) = as_binop(shifted, BinaryOpKind::Subtract)?;
    let lit = |e: &Expr| match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    };
    (lit(one)? == 1 && lit(one2)? == 1 && as_ident(i)? == iv).then_some(())?;
    lit(w).filter(|w| *w > 0).map(|w| w as usize)
}

/// A float reduction tensor op — `rmsnorm` (unit-RMS rescale) or `softmax` (probability distribution).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum ReductionKind {
    Rmsnorm,
    Softmax,
}

impl ReductionKind {
    pub fn name(self) -> &'static str {
        match self {
            ReductionKind::Rmsnorm => "rmsnorm",
            ReductionKind::Softmax => "softmax",
        }
    }
}

/// The recognized shape of a reduction: source tensor `x`, destination `y`, element count `n`, and (an rmsnorm's) the
/// divisor its normalize step divides by, whose assignment holds its epsilon.
#[derive(Debug, Clone, Copy)]
pub struct ReductionSpec {
    pub kind: ReductionKind,
    pub x: Symbol,
    pub y: Symbol,
    pub n: usize,
    pub denom: Option<Symbol>,
}

/// The reduction performed per row in a **fused row op** — the extensible core of the fused-row family
/// (LayerNorm / fused-add-RMSNorm join here as they land). Each is a shared-memory reduction over one row.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum RowReduceKind {
    /// `Y[r,i] = X[r,i] / sqrt(mean_i(X[r,·]²) + eps)` (× `weight[i]` when present) — unit-RMS rescale.
    RmsNorm,
    /// `Y[r,i] = exp(X[r,i]-max) / Σ exp(X[r,·]-max)` — a per-row probability distribution.
    Softmax,
    /// `Y[r,i] = (X[r,i]-mean_i)/sqrt(var_i+eps) * W[i] + B[i]` — per-row mean/variance normalize, scale + shift.
    LayerNorm,
}

impl RowReduceKind {
    pub fn name(self) -> &'static str {
        match self {
            RowReduceKind::RmsNorm => "rmsnorm",
            RowReduceKind::Softmax => "softmax",
            RowReduceKind::LayerNorm => "layernorm",
        }
    }
}

/// A norm's epsilon — the `E` of `sqrt(… + E)` — kept as its bits, so a spec holding one stays `Eq` and hashable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Epsilon(u64);

impl Epsilon {
    /// The epsilon a norm sugar takes when the program gives none.
    pub const DEFAULT: Epsilon = Epsilon(1e-6f64.to_bits());

    pub fn new(v: f64) -> Epsilon {
        Epsilon(v.to_bits())
    }

    pub fn get(self) -> f64 {
        f64::from_bits(self.0)
    }
}

/// A constant factor an op multiplies by, kept as its bits, so a spec holding one stays `Eq` and hashable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Factor(u64);

impl Factor {
    pub fn new(v: f64) -> Factor {
        Factor(v.to_bits())
    }

    pub fn get(self) -> f64 {
        f64::from_bits(self.0)
    }
}

/// The epsilon of the norm whose divisor is `denom`: the `E` of the `sqrt(… + E)` the norm's desugar assigns to it.
pub fn epsilon_of(stmts: &[Stmt], denom: Symbol) -> Option<Epsilon> {
    for s in stmts {
        let found = match s {
            Stmt::Let { var, value, .. } if *var == denom => sqrt_epsilon(value),
            Stmt::Set { target, value } if *target == denom => sqrt_epsilon(value),
            Stmt::While { body, .. } | Stmt::Splice { body } => epsilon_of(body, denom),
            Stmt::If { then_block, else_block, .. } => {
                epsilon_of(then_block, denom).or_else(|| else_block.and_then(|e| epsilon_of(e, denom)))
            }
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The `E` of `sqrt(… + E)`.
fn sqrt_epsilon(value: &Expr) -> Option<Epsilon> {
    let Expr::Call { args, .. } = value else { return None };
    let [arg] = args.as_slice() else { return None };
    let (_, eps) = as_binop(arg, BinaryOpKind::Add)?;
    match eps {
        Expr::Literal(Literal::Float(f)) if *f > 0.0 => Some(Epsilon::new(*f)),
        _ => None,
    }
}

/// A recognized **fused row op** — a per-row reduction over a `rows × cols` tensor: one output row per input
/// row, each normalized independently over its `cols` entries. This is the transformer-normalization shape
/// (RMSNorm / softmax / LayerNorm over the hidden dim, batched across rows). The flat single-vector reduction
/// [`ReductionSpec`] is exactly the `rows == 1` special case; this spec is what the one-block-per-row CUDA
/// emitter consumes, and it grows `weight`/`residual` as the fused variants land — the durable foundation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FusedRowSpec {
    pub rows: usize,
    pub cols: usize,
    pub reduce: RowReduceKind,
    pub x: Symbol,
    pub y: Symbol,
    /// Per-column weight applied after the reduction (`None` for the weightless sugar).
    pub weight: Option<Symbol>,
    /// How many rows of `cols` the weight holds: 1, one row shared by every row; `L`, the lanes of a hyper-connected
    /// residual, row `r` taking the weight's row `r mod L` (`rmsnorm of H in L lanes with weight W`).
    pub weight_rows: usize,
    /// A fused residual added element-wise BEFORE the reduction — the reduced value is `X[idx] + residual[idx]`
    /// (`None` for plain RMSNorm). This is the "add" of fused-add-RMSNorm (FIB `001_fused_add_rmsnorm`).
    pub residual: Option<Symbol>,
    /// Per-column bias ADDED after the scale (`Y = … * weight + bias`) — the shift of LayerNorm (`None` for
    /// RMSNorm, which has no bias).
    pub bias: Option<Symbol>,
    /// The norm's epsilon, as the program gives it ([`Epsilon::DEFAULT`] for a softmax, which has none).
    pub eps: Epsilon,
}

/// Recognize the fused row-op idiom. First it lifts the PER-ROW WEIGHTED RMSNorm shape (a row loop whose
/// normalize body `Push (X[(r − 1)·cols + j] / denom) * W[j]` gives `x`, `weight`, `y`, and whose enclosing loop bounds
/// give `rows`/`cols`; a weight read at `((r − 1) mod L)·cols + j` gives `weight_rows` `L`, and an operand read anywhere
/// else is no norm); failing that it falls back to the weightless flat `rmsnorm`/`softmax` sugar
/// (`rows == 1`, `cols == n`) via [`recognize_reduction`]. Backend-agnostic (CUDA + Tenstorrent).
pub fn recognize_fused_row(
    stmts: &[Stmt],
    interner: &Interner,
) -> Result<FusedRowSpec, ReductionReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    if let Some((mut spec, denom)) = search_weighted_rmsnorm(stmts, &consts, &mut Vec::new(), None) {
        if let Some(denom) = denom {
            spec.eps = epsilon_of(stmts, denom).ok_or(ReductionReject::NotReduction)?;
        }
        return Ok(spec);
    }
    let r = recognize_reduction(stmts, interner)?;
    let eps = match r.kind {
        ReductionKind::Rmsnorm => r.denom.and_then(|d| epsilon_of(stmts, d)).ok_or(ReductionReject::NotReduction)?,
        ReductionKind::Softmax => Epsilon::DEFAULT,
    };
    Ok(FusedRowSpec {
        rows: 1,
        cols: r.n,
        reduce: match r.kind {
            ReductionKind::Rmsnorm => RowReduceKind::RmsNorm,
            ReductionKind::Softmax => RowReduceKind::Softmax,
        },
        x: r.x,
        y: r.y,
        weight: None,
        weight_rows: 1,
        residual: None,
        bias: None,
        eps,
    })
}

/// How a row op's normalize loop reads its operands: the row counter (`None` with no row loop), the column counter
/// and the row's width, each counter counting from 1.
struct RowReads<'c> {
    row: Option<Symbol>,
    col: Symbol,
    cols: i64,
    consts: &'c HashMap<Symbol, i64>,
}

impl RowReads<'_> {
    fn affine(&self, index: &Expr) -> Option<Affine> {
        eval_affine(index, self.consts, &HashMap::new())
    }

    /// `(row − 1)·cols + col` — the entry of the row and column — or `col` with no row loop.
    fn entry(&self, index: &Expr) -> bool {
        let mut want = Affine::var(self.col);
        if let Some(r) = self.row {
            want = want.add(&Affine::var(r).sub(&Affine::constant(1)).scale(self.cols));
        }
        self.affine(index) == Some(want)
    }

    /// `col` — the column, one value per column shared by every row.
    fn column(&self, index: &Expr) -> bool {
        self.affine(index) == Some(Affine::var(self.col))
    }

    /// How many rows of the row's width a weight read at `index` holds: 1 read at the column, `L` read at
    /// `((row − 1) mod L)·cols + col` (row `r` taking the weight's row `r mod L`), `None` read anywhere else.
    fn weight_rows(&self, index: &Expr) -> Option<usize> {
        if self.column(index) {
            return Some(1);
        }
        let row = self.row?;
        let (a, b) = as_binop(index, BinaryOpKind::Add)?;
        let strided = if self.column(b) { a } else if self.column(a) { b } else { return None };
        let (p, q) = as_binop(strided, BinaryOpKind::Multiply)?;
        let constant = |e: &Expr| self.affine(e).filter(Affine::is_const).map(|a| a.cst);
        let (lane, stride) = match (constant(p), constant(q)) {
            (None, Some(s)) => (p, s),
            (Some(s), None) => (q, s),
            _ => return None,
        };
        let (counted, lanes) = as_binop(lane, BinaryOpKind::Modulo)?;
        let lanes = constant(lanes).filter(|l| *l >= 1)?;
        (stride == self.cols && self.affine(counted) == Some(Affine::var(row).sub(&Affine::constant(1))))
            .then_some(lanes as usize)
    }
}

/// A LayerNorm-normalize body `((X[idx] - mean) / denom) * W[col] + B[col]`, every operand read at its row's entry or
/// its column. Returns `(x, weight, bias, denom)`; `None` for any other shape. The top-level `+ B[col]` (bias shift) is
/// what distinguishes it from RMSNorm.
fn match_layernorm_push(value: &Expr, reads: &RowReads) -> Option<(Symbol, Symbol, Symbol, Symbol)> {
    let (scaled, b_index) = as_binop(value, BinaryOpKind::Add)?;
    let (b_coll, b_at) = as_index(b_index)?;
    let (div, w_index) = as_binop(scaled, BinaryOpKind::Multiply)?;
    let (w_coll, w_at) = as_index(w_index)?;
    let (sub, denom) = as_binop(div, BinaryOpKind::Divide)?;
    // The divisor is the row's `sqrt(var + eps)` scalar — a plain identifier, never a per-element expression.
    let denom = as_ident(denom)?;
    let (x_read, mean) = as_binop(sub, BinaryOpKind::Subtract)?;
    as_ident(mean)?;
    let (x_coll, x_at) = as_index(x_read)?;
    (reads.entry(x_at) && reads.column(w_at) && reads.column(b_at)).then_some(())?;
    Some((as_ident(x_coll)?, as_ident(w_coll)?, as_ident(b_coll)?, denom))
}

/// A weighted RMSNorm's normalize body.
struct WeightedPush {
    x: Symbol,
    residual: Option<Symbol>,
    weight: Symbol,
    weight_rows: usize,
    denom: Symbol,
}

/// A weighted-normalize body `(REDUCED / denom) * W[..]`, where `REDUCED` is `X[idx]` (plain RMSNorm) or
/// `X[idx] + residual[idx]` (fused-add-RMSNorm), each read at its row's entry, and the weight read at the column or a
/// lane's column ([`RowReads::weight_rows`]). `None` for any other shape: specific enough that only the (fused-add)
/// weighted-RMSNorm desugar matches.
fn match_weighted_push(value: &Expr, reads: &RowReads) -> Option<WeightedPush> {
    let (div, w_index) = as_binop(value, BinaryOpKind::Multiply)?;
    let (reduced, denom) = as_binop(div, BinaryOpKind::Divide)?;
    // The divisor must be the scalar `denom` binding (a plain identifier), NOT a per-element expression — this
    // rejects e.g. SwiGLU's `(gate / (1 + e^-gate)) * up`, whose divisor is a binop, not the row's rsqrt scalar.
    let denom = as_ident(denom)?;
    let (w_coll, w_at) = as_index(w_index)?;
    let weight = as_ident(w_coll)?;
    let weight_rows = reads.weight_rows(w_at)?;
    // Fused-add: the reduced value is `X[idx] + residual[idx]`.
    let (x, residual) = match as_binop(reduced, BinaryOpKind::Add) {
        Some((a, b)) => {
            let (x_coll, x_at) = as_index(a)?;
            let (r_coll, r_at) = as_index(b)?;
            (reads.entry(x_at) && reads.entry(r_at)).then_some(())?;
            (as_ident(x_coll)?, Some(as_ident(r_coll)?))
        }
        None => {
            let (x_coll, x_at) = as_index(reduced)?;
            reads.entry(x_at).then_some(())?;
            (as_ident(x_coll)?, None)
        }
    };
    Some(WeightedPush { x, residual, weight, weight_rows, denom })
}

/// Search for the weighted per-row RMSNorm shape (and the LayerNorm and per-row softmax shapes beside it), tracking
/// the stack of enclosing counted loops so the innermost is the column's and the next-out the row's. Returns the spec
/// and the norm's divisor, whose assignment holds its epsilon (none for a softmax).
fn search_weighted_rmsnorm(
    stmts: &[Stmt],
    consts: &HashMap<Symbol, i64>,
    bounds: &mut Vec<(Symbol, i64)>,
    binder: Option<Symbol>,
) -> Option<(FusedRowSpec, Option<Symbol>)> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                let pushed = ew_loop_bound(cond, consts)
                    .map(|counted| bounds.push(counted))
                    .is_some();
                if let Some(spec) = search_weighted_rmsnorm(body, consts, bounds, binder) {
                    return Some(spec);
                }
                if pushed {
                    bounds.pop();
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if let Some(spec) = search_weighted_rmsnorm(then_block, consts, bounds, binder) {
                    return Some(spec);
                }
                if let Some(eb) = else_block {
                    if let Some(spec) = search_weighted_rmsnorm(eb, consts, bounds, binder) {
                        return Some(spec);
                    }
                }
            }
            Stmt::Splice { body } => {
                if let Some(spec) =
                    search_weighted_rmsnorm(body, consts, bounds, splice_binder(body).or(binder))
                {
                    return Some(spec);
                }
            }
            Stmt::Push { value, collection } => {
                let y = as_ident(collection)?;
                if !pushes_into_binder(y, binder) {
                    continue;
                }
                let (col, cols) = *bounds.last()?;
                let row = bounds.len().checked_sub(2).map(|at| bounds[at]);
                let rows = row.map_or(1, |(_, n)| n);
                if cols <= 0 || rows <= 0 {
                    continue;
                }
                let reads = RowReads { row: row.map(|(r, _)| r), col, cols, consts };
                // LayerNorm first (its `+ B[col]` shift is the most specific shape), then weighted RMSNorm.
                if let Some((x, weight, bias, denom)) = match_layernorm_push(value, &reads) {
                    let spec = FusedRowSpec {
                        rows: rows as usize,
                        cols: cols as usize,
                        reduce: RowReduceKind::LayerNorm,
                        x,
                        y,
                        weight: Some(weight),
                        weight_rows: 1,
                        residual: None,
                        bias: Some(bias),
                        eps: Epsilon::DEFAULT,
                    };
                    return Some((spec, Some(denom)));
                }
                if let Some(x) = match_softmax_push(value, &reads) {
                    let spec = FusedRowSpec {
                        rows: rows as usize,
                        cols: cols as usize,
                        reduce: RowReduceKind::Softmax,
                        x,
                        y,
                        weight: None,
                        weight_rows: 1,
                        residual: None,
                        bias: None,
                        eps: Epsilon::DEFAULT,
                    };
                    return Some((spec, None));
                }
                if let Some(p) = match_weighted_push(value, &reads) {
                    let spec = FusedRowSpec {
                        rows: rows as usize,
                        cols: cols as usize,
                        reduce: RowReduceKind::RmsNorm,
                        x: p.x,
                        y,
                        weight: Some(p.weight),
                        weight_rows: p.weight_rows,
                        residual: p.residual,
                        bias: None,
                        eps: Epsilon::DEFAULT,
                    };
                    return Some((spec, Some(p.denom)));
                }
            }
            _ => {}
        }
    }
    None
}

/// A per-row softmax normalize body `e^(X[idx] - m) / s` — the value the row loop pushes, `X` read at its row's
/// entry. Returns the source tensor. `m` (the row max) and `s` (the row's sum of exponentials) must be plain
/// identifiers: they are the row scalars, and requiring them rejects any per-element divisor.
fn match_softmax_push(value: &Expr, reads: &RowReads) -> Option<Symbol> {
    let (expo, s_denom) = as_binop(value, BinaryOpKind::Divide)?;
    as_ident(s_denom)?;
    let (base, shifted) = as_binop(expo, BinaryOpKind::Pow)?;
    if !is_euler(base) {
        return None;
    }
    let (x_index, m) = as_binop(shifted, BinaryOpKind::Subtract)?;
    as_ident(m)?;
    let (x_coll, x_at) = as_index(x_index)?;
    reads.entry(x_at).then_some(())?;
    as_ident(x_coll)
}

/// A recognized **SwiGLU** map: `Y[i] = silu(gate[i]) * up[i]`, `silu(g) = g / (1 + e^-g)`, over `total`
/// elements — the gated MLP activation (not a row-reduction; a pure two-input elementwise map).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwiGluSpec {
    pub total: usize,
    pub gate: Symbol,
    pub up: Symbol,
    pub y: Symbol,
}

/// Recognize the SwiGLU idiom the sugar desugars to — `Push (gate[iv] / (1 + E^(0 - gate[iv]))) * up[iv]` in a
/// counted loop. Backend-agnostic (CUDA + Tenstorrent).
pub fn recognize_swiglu(
    stmts: &[Stmt],
    _interner: &Interner,
) -> Result<SwiGluSpec, ReductionReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    search_swiglu(stmts, &consts, None).ok_or(ReductionReject::NotReduction)
}

fn search_swiglu(
    stmts: &[Stmt],
    consts: &HashMap<Symbol, i64>,
    binder: Option<Symbol>,
) -> Option<SwiGluSpec> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                if let Some((iv, n)) = ew_loop_bound(cond, consts) {
                    if n > 0 {
                        for st in *body {
                            if let Stmt::Push { value, collection } = st {
                                let y = as_ident(collection)?;
                                if !pushes_into_binder(y, binder) {
                                    continue;
                                }
                                if let Some((gate, up)) = match_swiglu_push(value, iv) {
                                    return Some(SwiGluSpec {
                                        total: n as usize,
                                        gate,
                                        up,
                                        y,
                                    });
                                }
                            }
                        }
                    }
                }
                if let Some(spec) = search_swiglu(body, consts, binder) {
                    return Some(spec);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if let Some(spec) = search_swiglu(then_block, consts, binder) {
                    return Some(spec);
                }
                if let Some(eb) = else_block {
                    if let Some(spec) = search_swiglu(eb, consts, binder) {
                        return Some(spec);
                    }
                }
            }
            Stmt::Splice { body } => {
                if let Some(spec) = search_swiglu(body, consts, splice_binder(body).or(binder)) {
                    return Some(spec);
                }
            }
            _ => {}
        }
    }
    None
}

/// `Push (gate[iv] / (1 + E^(0 - gate[iv]))) * up[iv]` → `(gate, up)`; both element reads index by `iv`.
fn match_swiglu_push(value: &Expr, iv: Symbol) -> Option<(Symbol, Symbol)> {
    let (silu, up_index) = as_binop(value, BinaryOpKind::Multiply)?;
    let (up_coll, up_idx) = as_index(up_index)?;
    if as_ident(up_idx) != Some(iv) {
        return None;
    }
    // silu(g) = g / (1 + e^(0 - g))
    let (g_num, denom) = as_binop(silu, BinaryOpKind::Divide)?;
    let (g_coll, g_idx) = as_index(g_num)?;
    if as_ident(g_idx) != Some(iv) {
        return None;
    }
    // denom = 1 + Pow(E, Subtract(0, gate[iv])) — confirm the sigmoid shape (an Add whose RHS is a Pow).
    let (_one, powe) = as_binop(denom, BinaryOpKind::Add)?;
    as_binop(powe, BinaryOpKind::Pow)?;
    Some((as_ident(g_coll)?, as_ident(up_coll)?))
}

/// Why a program is not a recognizable reduction op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReductionReject {
    NotReduction,
}

/// Recognize the `rmsnorm` / `softmax` normalize-loop idiom the sugar desugars to — a counted loop that
/// `Push`es `X[p] / denom` (rmsnorm) or `e^(X[p]-m) / s` (softmax) into `Y` — anywhere (through `Splice`).
pub fn recognize_reduction(
    stmts: &[Stmt],
    _interner: &Interner,
) -> Result<ReductionSpec, ReductionReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    search_reduction(stmts, &consts, None).ok_or(ReductionReject::NotReduction)
}

fn search_reduction(
    stmts: &[Stmt],
    consts: &HashMap<Symbol, i64>,
    binder: Option<Symbol>,
) -> Option<ReductionSpec> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                if let Some(spec) = try_reduction_at(cond, body, consts, stmts, binder) {
                    return Some(spec);
                }
                if let Some(spec) = search_reduction(body, consts, binder) {
                    return Some(spec);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if let Some(spec) = search_reduction(then_block, consts, binder) {
                    return Some(spec);
                }
                if let Some(eb) = else_block {
                    if let Some(spec) = search_reduction(eb, consts, binder) {
                        return Some(spec);
                    }
                }
            }
            Stmt::Splice { body } => {
                if let Some(spec) = search_reduction(body, consts, splice_binder(body).or(binder)) {
                    return Some(spec);
                }
            }
            _ => {}
        }
    }
    None
}

/// The flat normalize loop of `rmsnorm` / `softmax`, with the divisor checked for the form it claims: an
/// rmsnorm divides by the root-mean-square scalar this block binds (`Let denom be sqrt(ss / n + eps)`), a
/// softmax by the row's exponential sum — never by a per-element expression, which is how `silu`'s and
/// `gelu`'s `x / (1 + e^…)` loops read otherwise. `scope` is the statement list the loop lives in.
fn try_reduction_at(
    cond: &Expr,
    body: &[Stmt],
    consts: &HashMap<Symbol, i64>,
    scope: &[Stmt],
    binder: Option<Symbol>,
) -> Option<ReductionSpec> {
    let (iv, n) = ew_loop_bound(cond, consts)?;
    if n <= 0 {
        return None;
    }
    for st in body {
        let Stmt::Push { value, collection } = st else {
            continue;
        };
        let y = as_ident(collection)?;
        if !pushes_into_binder(y, binder) {
            continue;
        }
        let (num, den) = as_binop(value, BinaryOpKind::Divide)?;
        // rmsnorm: X[iv] / denom, the divisor bound in this block to `sqrt(mean + eps)`.
        if let Some((coll, idx)) = as_index(num) {
            if as_ident(idx) == Some(iv)
                && as_ident(den).is_some_and(|d| binds_rms_denominator(scope, d))
            {
                return Some(ReductionSpec {
                    kind: ReductionKind::Rmsnorm,
                    x: as_ident(coll)?,
                    y,
                    n: n as usize,
                    denom: as_ident(den),
                });
            }
        }
        // softmax: e^(X[iv] - m) / s, with `m` (the row max) and `s` (the exponential sum) the row scalars.
        if let Some((base, exponent)) = as_binop(num, BinaryOpKind::Pow) {
            if is_euler(base) && as_ident(den).is_some() {
                if let Some((subl, m)) = as_binop(exponent, BinaryOpKind::Subtract) {
                    if as_ident(m).is_some() {
                        if let Some((coll, idx)) = as_index(subl) {
                            if as_ident(idx) == Some(iv) {
                                return Some(ReductionSpec {
                                    kind: ReductionKind::Softmax,
                                    x: as_ident(coll)?,
                                    y,
                                    n: n as usize,
                                    denom: None,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// The tensor a desugar binds — the first `Let var be a new Seq` of its `Splice` body. Every Push-based
/// recognizer lifts only pushes into that binder while inside the block, so a loop that is a fragment of a
/// larger sugar (the router softmax inside `moe`) is never taken for the whole program.
fn splice_binder(body: &[Stmt]) -> Option<Symbol> {
    body.iter().find_map(|s| match s {
        Stmt::Let { var, value, .. } if matches!(value, Expr::New { .. }) => Some(*var),
        _ => None,
    })
}

fn pushes_into_binder(collection: Symbol, binder: Option<Symbol>) -> bool {
    binder.map(|b| b == collection).unwrap_or(true)
}

fn is_euler(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::Float(v)) if (*v - std::f64::consts::E).abs() < 1e-9)
}

/// Does `scope` bind `d` to the rmsnorm denominator `sqrt(<sum> / <n> + <eps>)`?
fn binds_rms_denominator(scope: &[Stmt], d: Symbol) -> bool {
    scope.iter().any(|s| match s {
        Stmt::Let {
            var,
            value: Expr::Call { args, .. },
            ..
        } if *var == d => match args.as_slice() {
            [arg] => as_binop(arg, BinaryOpKind::Add)
                .and_then(|(mean, eps)| Some((as_binop(mean, BinaryOpKind::Divide)?, eps)))
                .map(|((ss, _n), eps)| {
                    as_ident(ss).is_some() && matches!(eps, Expr::Literal(Literal::Float(_)))
                })
                .unwrap_or(false),
            _ => false,
        },
        _ => false,
    })
}

/// Try every loop nest (and nested blocks) as a matmul root; return the first match, else the most-advanced
/// refusal seen (so a triple-loop-with-wrong-indices reports `NotGemmLayout`, not `NoLoopNest`).
fn search_matmul(
    stmts: &[Stmt],
    consts: &HashMap<Symbol, i64>,
) -> Result<MatmulSpec, MatmulReject> {
    let mut best = MatmulReject::NoLoopNest;
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                match try_matmul_at(cond, body, consts) {
                    Ok(spec) => return Ok(spec),
                    Err(e) => {
                        if reject_rank(&e) > reject_rank(&best) {
                            best = e;
                        }
                    }
                }
                if let Ok(spec) = search_matmul(body, consts) {
                    return Ok(spec);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if let Ok(spec) = search_matmul(then_block, consts) {
                    return Ok(spec);
                }
                if let Some(eb) = else_block {
                    if let Ok(spec) = search_matmul(eb, consts) {
                        return Ok(spec);
                    }
                }
            }
            // Parser desugars (tensor sugar, tuple destructure, …) emit their statements inside a transparent
            // `Stmt::Splice`; look through it so a matmul written as `Let C be A matmul B` is recognized.
            Stmt::Splice { body } => {
                if let Ok(spec) = search_matmul(body, consts) {
                    return Ok(spec);
                }
            }
            _ => {}
        }
    }
    Err(best)
}

/// Try to recognize a matmul with the given `While` as the outer (row / `i`) loop.
fn try_matmul_at(
    cond_i: &Expr,
    body_i: &[Stmt],
    consts: &HashMap<Symbol, i64>,
) -> Result<MatmulSpec, MatmulReject> {
    let (iv_i, m) = loop_iv_bound(cond_i, consts).ok_or(MatmulReject::NoLoopNest)?;
    let (cond_k, body_k) = first_while(body_i).ok_or(MatmulReject::NotTripleNested)?;
    let (iv_k, k) = loop_iv_bound(cond_k, consts).ok_or(MatmulReject::NotTripleNested)?;
    let (cond_j, body_j) = first_while(body_k).ok_or(MatmulReject::NotTripleNested)?;
    let (iv_j, n) = loop_iv_bound(cond_j, consts).ok_or(MatmulReject::NotTripleNested)?;

    // Inner `Let` bindings (e.g. `Let idx be i*n + j + 1.`) captured as affine forms so the store's index
    // expressions resolve even when they are named.
    let mut lets: HashMap<Symbol, Affine> = HashMap::new();
    for s in body_j {
        if let Stmt::Let { var, value, .. } = s {
            if let Some(a) = eval_affine(value, consts, &lets) {
                lets.insert(*var, a);
            }
        }
    }

    // The accumulating indexed store into C.
    let (store_coll, store_idx, store_val) = body_j
        .iter()
        .find_map(|s| match s {
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => Some((*collection, *index, *value)),
            _ => None,
        })
        .ok_or(MatmulReject::NoAccumulatingStore)?;

    let c = as_ident(store_coll).ok_or(MatmulReject::NotGemmLayout)?;
    let c_idx = eval_affine(store_idx, consts, &lets).ok_or(MatmulReject::NonAffineIndex)?;

    // value = (item c_idx of C) + (item a_idx of A) * (item b_idx of B)
    let (add_l, add_r) =
        as_binop(store_val, BinaryOpKind::Add).ok_or(MatmulReject::NoAccumulatingStore)?;
    let (acc_coll, acc_idx) = as_index(add_l).ok_or(MatmulReject::NoAccumulatingStore)?;
    if as_ident(acc_coll) != Some(c) {
        return Err(MatmulReject::NoAccumulatingStore);
    }
    let acc_idx_aff = eval_affine(acc_idx, consts, &lets).ok_or(MatmulReject::NonAffineIndex)?;
    if acc_idx_aff != c_idx {
        return Err(MatmulReject::NoAccumulatingStore); // must be C[idx] += … (same index read + written)
    }

    let (mul_l, mul_r) =
        as_binop(add_r, BinaryOpKind::Multiply).ok_or(MatmulReject::NotGemmLayout)?;
    let (a_coll, a_idx_e) = as_index(mul_l).ok_or(MatmulReject::NotGemmLayout)?;
    let (b_coll, b_idx_e) = as_index(mul_r).ok_or(MatmulReject::NotGemmLayout)?;
    let a = as_ident(a_coll).ok_or(MatmulReject::NotGemmLayout)?;
    let b = as_ident(b_coll).ok_or(MatmulReject::NotGemmLayout)?;
    let a_idx = eval_affine(a_idx_e, consts, &lets).ok_or(MatmulReject::NonAffineIndex)?;
    let b_idx = eval_affine(b_idx_e, consts, &lets).ok_or(MatmulReject::NonAffineIndex)?;

    // Row-major GEMM layout, verified against the loop bounds:
    //   C[i·N + j]  (i-stride N, j-stride 1, no k)
    //   A[i·K + k]  (i-stride K, k-stride 1, no j)
    //   B[k·N + j]  (k-stride N, j-stride 1, no i)
    let n_i = n;
    let k_i = k;
    let gemm_c =
        c_idx.coeff_of(iv_i) == n_i && c_idx.coeff_of(iv_j) == 1 && c_idx.coeff_of(iv_k) == 0;
    let gemm_a =
        a_idx.coeff_of(iv_i) == k_i && a_idx.coeff_of(iv_k) == 1 && a_idx.coeff_of(iv_j) == 0;
    let gemm_b =
        b_idx.coeff_of(iv_k) == n_i && b_idx.coeff_of(iv_j) == 1 && b_idx.coeff_of(iv_i) == 0;
    if !(gemm_c && gemm_a && gemm_b) {
        return Err(MatmulReject::NotGemmLayout);
    }

    Ok(MatmulSpec {
        m: m as usize,
        n: n as usize,
        k: k as usize,
        a,
        b,
        c,
    })
}

/// A recognized dot product `acc = Σ_{t<n} A[t]·B[t]`: one counted loop accumulating the product of two
/// declared `Seq of Int` arrays read at the same unit-stride index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotSpec {
    pub n: usize,
    pub a: Symbol,
    pub b: Symbol,
    pub acc: Symbol,
}

/// Why a program is not a recognizable dot product — a typed refusal, never a miscompile. Ordered from the
/// least to the most advanced, so a search reports the refusal of the loop that came closest: what a loop's
/// body computes ranks before whether its counter is provable, so a dot product with an unprovable counter
/// outranks a loop that accumulates nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DotReject {
    /// No `While counter is less than BOUND` loop.
    NoLoop,
    /// No `Set s to s + …` in the loop body.
    NoAccumulation,
    /// The accumulated term is not `item _ of A * item _ of B`.
    NoProduct,
    /// A product, but not of two declared `Seq of Int` arrays — the integer dot product the app computes.
    NotInt,
    /// An index is not affine in the counter.
    NonAffineIndex,
    /// The two operands are not read at one unit-stride index of the counter.
    NotAligned,
    /// The counter is not set to a constant right before the loop, or is not stepped by exactly one.
    NotCounted,
    /// The loop body does more than the dot product.
    ExtraWork,
    /// The accumulator is not set to zero right before the loop.
    NotFromZero,
}

/// A recognized outer product `C[i·n + j] = A[i]·B[j]` over `i < m`, `j < n`: a counted nest storing (not
/// accumulating) the product of two `Seq of Int` arrays into a row-major `m × n` array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OuterSpec {
    pub m: usize,
    pub n: usize,
    pub a: Symbol,
    pub b: Symbol,
    pub c: Symbol,
}

/// Why a program is not a recognizable outer product, least to most advanced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OuterReject {
    /// No `While counter is less than BOUND` loop.
    NoLoopNest,
    /// A counted loop with no counted loop directly inside it.
    NotDoubleNested,
    /// No `Set item _ of C to …` in the inner loop.
    NoStore,
    /// The stored value is not `item _ of A * item _ of B`.
    NoProduct,
    /// A product, but `A`, `B` and `C` are not all declared `Seq of Int` arrays — the integer product the app computes.
    NotInt,
    /// `C[idx] := C[idx] + A·B` — a rank-1 update, not an outer product.
    Accumulates,
    /// `C[idx] := A·B` where a rank-1 update (`C[idx] := C[idx] + A·B`) is read.
    Stores,
    /// An index is not affine in the counters.
    NonAffineIndex,
    /// A counter not set to a constant right before its loop, or not stepped by exactly one.
    NotCounted,
    /// Not `C[i·n + j]`, `A[i]`, `B[j]` with unit strides against the loop bounds.
    NotOuterLayout,
    /// A loop body does more than the outer product.
    ExtraWork,
}

/// A counted loop: its counter and how many times its body runs.
struct Counted {
    iv: Symbol,
    trips: i64,
}

/// The constant each variable holds as the statement at `stmts[at]` begins, read from the contiguous run of
/// `Let`/`Set` statements right before it: the nearest write wins, and a non-constant write hides every
/// earlier one.
fn entry_values(
    stmts: &[Stmt],
    at: usize,
    consts: &HashMap<Symbol, i64>,
) -> HashMap<Symbol, Option<i64>> {
    let empty = HashMap::new();
    let mut out: HashMap<Symbol, Option<i64>> = HashMap::new();
    for s in stmts[..at].iter().rev() {
        let (var, value) = match s {
            Stmt::Let { var, value, .. } => (*var, *value),
            Stmt::Set { target, value } => (*target, *value),
            _ => break,
        };
        out.entry(var).or_insert_with(|| {
            eval_affine(value, consts, &empty)
                .filter(Affine::is_const)
                .map(|a| a.cst)
        });
    }
    out
}

/// `Set iv to iv + 1` (either operand order).
fn is_unit_step(s: &Stmt, iv: Symbol) -> bool {
    match s {
        Stmt::Set { target, value } if *target == iv => as_binop(value, BinaryOpKind::Add)
            .map_or(false, |(l, r)| {
                let one = |e: &Expr| matches!(e, Expr::Literal(Literal::Number(1)));
                (as_ident(l) == Some(iv) && one(r)) || (one(l) && as_ident(r) == Some(iv))
            }),
        _ => false,
    }
}

/// The loop `While iv is less than BOUND` at `stmts[at]` as a counted loop: `iv` set to a constant right
/// before it, and its one top-level write in the body `Set iv to iv + 1`.
fn counted_at(
    stmts: &[Stmt],
    at: usize,
    (iv, bound): (Symbol, i64),
    body: &[Stmt],
    consts: &HashMap<Symbol, i64>,
) -> Option<Counted> {
    let start = entry_values(stmts, at, consts).get(&iv).copied().flatten()?;
    let writes = |s: &&Stmt| {
        matches!(s, Stmt::Let { var, .. } if *var == iv)
            || matches!(s, Stmt::Set { target, .. } if *target == iv)
    };
    let steps = body.iter().filter(|s| is_unit_step(s, iv)).count();
    (steps == 1 && body.iter().filter(writes).count() == 1 && bound > start).then(|| Counted {
        iv,
        trips: bound - start,
    })
}

/// Try `at` on every `While` of a block and of the blocks nested in it (loop bodies, an `If`'s arms, a
/// `Splice`): the first match wins, else the most advanced refusal seen (`floor` when no loop was tried).
fn search_loops<'a, T, R: Ord + Copy>(
    stmts: &'a [Stmt<'a>],
    floor: R,
    at: &impl Fn(&'a [Stmt<'a>], usize, &'a Expr<'a>, &'a [Stmt<'a>]) -> Result<T, R>,
) -> Result<T, R> {
    let mut best = floor;
    for (k, s) in stmts.iter().enumerate() {
        let nested: Vec<&'a [Stmt<'a>]> = match s {
            Stmt::While { cond, body, .. } => {
                match at(stmts, k, cond, body) {
                    Ok(spec) => return Ok(spec),
                    Err(e) => best = best.max(e),
                }
                vec![*body]
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => std::iter::once(*then_block).chain(*else_block).collect(),
            Stmt::Splice { body } => vec![*body],
            _ => Vec::new(),
        };
        for block in nested {
            match search_loops(block, floor, at) {
                Ok(spec) => return Ok(spec),
                Err(e) => best = best.max(e),
            }
        }
    }
    Err(best)
}

/// The immutable `Let` bindings in scope, in order — an outer loop's before its inner loop's — as affine
/// forms (for indices) and as expressions (for an operand or product named before it is used).
fn body_lets<'a>(
    scope: impl IntoIterator<Item = &'a Stmt<'a>>,
    consts: &HashMap<Symbol, i64>,
) -> (HashMap<Symbol, Affine>, HashMap<Symbol, &'a Expr<'a>>) {
    let mut affine = HashMap::new();
    let mut exprs = HashMap::new();
    for s in scope {
        if let Stmt::Let {
            var,
            value,
            mutable: false,
            ..
        } = s
        {
            if let Some(a) = eval_affine(value, consts, &affine) {
                affine.insert(*var, a);
            }
            exprs.insert(*var, *value);
        }
    }
    (affine, exprs)
}

/// `e` with a body-`Let` name replaced by the expression it was bound to.
fn through_lets<'a>(e: &'a Expr<'a>, exprs: &HashMap<Symbol, &'a Expr<'a>>) -> &'a Expr<'a> {
    as_ident(e)
        .and_then(|s| exprs.get(&s).copied())
        .unwrap_or(e)
}

/// `item IA of A * item IB of B` (through body `Let`s), as `(A, IA, B, IB)`.
fn indexed_product<'a>(
    e: &'a Expr<'a>,
    exprs: &HashMap<Symbol, &'a Expr<'a>>,
) -> Option<(Symbol, &'a Expr<'a>, Symbol, &'a Expr<'a>)> {
    let (l, r) = as_binop(through_lets(e, exprs), BinaryOpKind::Multiply)?;
    let (ac, ai) = as_index(through_lets(l, exprs))?;
    let (bc, bi) = as_index(through_lets(r, exprs))?;
    Some((as_ident(ac)?, ai, as_ident(bc)?, bi))
}

/// Whether a body statement is an index or product name: an immutable `Let` that rebinds none of `kept`.
fn is_body_let(s: &Stmt, kept: &[Symbol]) -> bool {
    matches!(s, Stmt::Let { var, mutable: false, .. } if !kept.contains(var))
}

/// Recognize a dot product anywhere in a LOGOS statement list:
///
/// ```text
/// Let mutable s be 0.
/// Let mutable i be 0.
/// While i is less than n:
///     Set s to s + item (i + 1) of a * item (i + 1) of b.
///     Set i to i + 1.
/// ```
///
/// A dot product is the [zip sum](recognize_zip_sum) whose term is the product of two reads: the loop is
/// counted, the accumulator starts at zero right before it, both operands are read at the same unit-stride
/// index, and the body holds nothing else but immutable index or product names. The search returns the
/// refusal of the loop that came closest when nothing matches.
pub fn recognize_dot(stmts: &[Stmt], interner: &Interner) -> Result<DotSpec, DotReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    let program = Program::of(stmts, interner);
    search_loops(stmts, DotReject::NoLoop, &|block, at, cond, body| {
        let zip = try_zip_at(block, at, cond, body, &consts, &program).map_err(DotReject::from)?;
        match zip.term {
            ZipTerm::Op(ZipOp::Mul, ref l, ref r) => match (l.as_ref(), r.as_ref()) {
                (ZipTerm::Read(x), ZipTerm::Read(y)) => Ok(DotSpec {
                    n: zip.n,
                    a: zip.operands[*x],
                    b: zip.operands[*y],
                    acc: zip.acc,
                }),
                _ => Err(DotReject::NoProduct),
            },
            _ => Err(DotReject::NoProduct),
        }
    })
}

impl From<ZipReject> for DotReject {
    fn from(z: ZipReject) -> DotReject {
        match z {
            ZipReject::NoLoop => DotReject::NoLoop,
            ZipReject::NoAccumulation => DotReject::NoAccumulation,
            ZipReject::UnsupportedTerm => DotReject::NoProduct,
            ZipReject::NotInt => DotReject::NotInt,
            ZipReject::NonAffineIndex => DotReject::NonAffineIndex,
            ZipReject::NotAligned => DotReject::NotAligned,
            ZipReject::NotCounted => DotReject::NotCounted,
            ZipReject::ExtraWork => DotReject::ExtraWork,
            ZipReject::NotFromZero => DotReject::NotFromZero,
        }
    }
}

/// An operator of a [`ZipTerm`]: arithmetic, a comparison (a boolean), or a boolean connective. On two `Int`s
/// arithmetic is `Int` (`/` truncating toward zero, `**` exact); with a `Real` operand it is `Real` — the VM's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZipOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

impl ZipOp {
    fn of(op: BinaryOpKind) -> Option<ZipOp> {
        Some(match op {
            BinaryOpKind::Add => ZipOp::Add,
            BinaryOpKind::Subtract => ZipOp::Sub,
            BinaryOpKind::Multiply => ZipOp::Mul,
            BinaryOpKind::Divide => ZipOp::Div,
            BinaryOpKind::Pow => ZipOp::Pow,
            BinaryOpKind::Lt => ZipOp::Lt,
            BinaryOpKind::Gt => ZipOp::Gt,
            BinaryOpKind::LtEq => ZipOp::Le,
            BinaryOpKind::GtEq => ZipOp::Ge,
            BinaryOpKind::Eq => ZipOp::Eq,
            BinaryOpKind::NotEq => ZipOp::Ne,
            BinaryOpKind::And => ZipOp::And,
            BinaryOpKind::Or => ZipOp::Or,
            _ => return None,
        })
    }

    /// Whether the operator compares two numbers.
    pub fn compares(self) -> bool {
        matches!(self, ZipOp::Lt | ZipOp::Gt | ZipOp::Le | ZipOp::Ge | ZipOp::Eq | ZipOp::Ne)
    }

    /// Whether the operator joins two booleans.
    pub fn connects(self) -> bool {
        matches!(self, ZipOp::And | ZipOp::Or)
    }
}

/// The type of a [`ZipTerm`], or of an array element it reads: LOGOS's 64-bit `Int`, its `Real` (a double) and `Bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ty {
    Int,
    Real,
    Bool,
}

/// A scalar a [`ZipTerm`] takes, as the VM holds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scalar {
    Int(i64),
    Real(f64),
    Bool(bool),
}

/// Why [`ZipTerm::value`] has no value: an `Int` left the range asked for, a `Real` is not finite or left its bound,
/// a divisor is zero, an exponent negative, or an operator met an operand it does not take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    IntRange,
    RealRange,
    ZeroDivisor,
    NegativeExponent,
    Mistyped,
}

/// The term a zip loop computes from the elements it reads at the counter's index: arithmetic, comparisons and
/// selects over array elements and constants. A guarded accumulation (`If P: Set s to s + E`) is the select
/// `P ? E : 0`; a call of one of the program's pure functions is its body, inlined.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ZipTerm {
    /// The element of operand `k` (the `k`-th distinct array the loop reads, in reading order).
    Read(usize),
    Const(i64),
    /// A `Real` constant, by its bits.
    Real(u64),
    Op(ZipOp, Box<ZipTerm>, Box<ZipTerm>),
    /// `cond ? then : else`, `cond` a boolean.
    Select(Box<ZipTerm>, Box<ZipTerm>, Box<ZipTerm>),
    /// `abs(x)`.
    Abs(Box<ZipTerm>),
}

impl ZipTerm {
    /// The `Real` constant `v`.
    pub fn real(v: f64) -> ZipTerm {
        ZipTerm::Real(v.to_bits())
    }

    /// The term's type when operand `k` holds a `reads[k]`; `None` when an operator sees an operand it does not
    /// take, a select's guard is not a boolean or its arms differ in type, or the term is a boolean select.
    pub fn ty(&self, reads: &[Ty]) -> Option<Ty> {
        match self {
            ZipTerm::Read(k) => reads.get(*k).copied(),
            ZipTerm::Const(_) => Some(Ty::Int),
            ZipTerm::Real(_) => Some(Ty::Real),
            ZipTerm::Abs(x) => abs_ty(x.ty(reads)?),
            ZipTerm::Op(op, l, r) => op_ty(*op, l.ty(reads)?, r.ty(reads)?),
            ZipTerm::Select(c, a, b) => select_ty(c.ty(reads)?, a.ty(reads)?, b.ty(reads)?),
        }
    }

    /// The term's value at one element, every operand an `Int` (`row[k]` the element of operand `k`), booleans
    /// as 0 and 1; `None` when an intermediate value leaves 64 bits or it has none.
    pub fn eval(&self, row: &[i64]) -> Option<i64> {
        self.eval_within(row, i64::MIN as i128, i64::MAX as i128)
    }

    /// [`Self::eval`] with every intermediate value held to `lo..=hi` (`None` when one leaves it).
    pub fn eval_within(&self, row: &[i64], lo: i128, hi: i128) -> Option<i64> {
        let row: Vec<Scalar> = row.iter().map(|v| Scalar::Int(*v)).collect();
        match self.value(&row, (lo, hi), f64::MAX).ok()? {
            Scalar::Int(v) => Some(v),
            Scalar::Bool(b) => Some(b as i64),
            Scalar::Real(_) => None,
        }
    }

    /// The term's value at one element (`row[k]` the element of operand `k`) as the VM computes it, every `Int`
    /// held to `ints` and every `Real` finite and within `±reals`. Only the arm a select takes is evaluated, as
    /// the VM's `If` does (and a `mux` keeps).
    pub fn value(&self, row: &[Scalar], ints: (i128, i128), reals: f64) -> Result<Scalar, Stop> {
        match self {
            ZipTerm::Read(k) => bounded(*row.get(*k).ok_or(Stop::Mistyped)?, ints, reals),
            ZipTerm::Const(c) => bounded(Scalar::Int(*c), ints, reals),
            ZipTerm::Real(b) => bounded(Scalar::Real(f64::from_bits(*b)), ints, reals),
            ZipTerm::Abs(x) => abs_of(x.value(row, ints, reals)?, ints, reals),
            ZipTerm::Select(c, a, b) => match c.value(row, ints, reals)? {
                Scalar::Bool(true) => a.value(row, ints, reals),
                Scalar::Bool(false) => b.value(row, ints, reals),
                _ => Err(Stop::Mistyped),
            },
            ZipTerm::Op(op, l, r) => apply(*op, l.value(row, ints, reals)?, r.value(row, ints, reals)?, ints, reals),
        }
    }
}

/// The type `op` gives operands of types `a` and `b`, by the VM's rules; `None` when it does not take them.
fn op_ty(op: ZipOp, a: Ty, b: Ty) -> Option<Ty> {
    let num = |t: Ty| t != Ty::Bool;
    if op.connects() {
        (a == Ty::Bool && b == Ty::Bool).then_some(Ty::Bool)
    } else if !(num(a) && num(b)) {
        None
    } else if op.compares() {
        Some(Ty::Bool)
    } else if a == Ty::Real || b == Ty::Real {
        Some(Ty::Real)
    } else {
        Some(Ty::Int)
    }
}

/// A select's type: its arms', when its guard is a boolean and its arms are one numeric type.
fn select_ty(c: Ty, a: Ty, b: Ty) -> Option<Ty> {
    (c == Ty::Bool && a == b && a != Ty::Bool).then_some(a)
}

fn abs_ty(x: Ty) -> Option<Ty> {
    (x != Ty::Bool).then_some(x)
}

/// `v` when it is an `Int` within `ints`, a finite `Real` within `±reals`, or a boolean.
fn bounded(v: Scalar, ints: (i128, i128), reals: f64) -> Result<Scalar, Stop> {
    match v {
        Scalar::Int(i) if !(ints.0 <= i as i128 && i as i128 <= ints.1) => Err(Stop::IntRange),
        Scalar::Real(r) if !(r.is_finite() && r.abs() <= reals) => Err(Stop::RealRange),
        v => Ok(v),
    }
}

fn abs_of(v: Scalar, ints: (i128, i128), reals: f64) -> Result<Scalar, Stop> {
    match v {
        Scalar::Int(i) => {
            let a = (i as i128).abs();
            if ints.0 <= a && a <= ints.1 {
                Ok(Scalar::Int(a as i64))
            } else {
                Err(Stop::IntRange)
            }
        }
        Scalar::Real(r) => bounded(Scalar::Real(r.abs()), ints, reals),
        Scalar::Bool(_) => Err(Stop::Mistyped),
    }
}

/// `a op b` as the VM computes it: two `Int`s exactly (in 128 bits, then held to `ints`), `/` truncating toward
/// zero and refusing a zero divisor, `**` refusing a negative exponent; an `Int` beside a `Real` promoted to it.
fn apply(op: ZipOp, a: Scalar, b: Scalar, ints: (i128, i128), reals: f64) -> Result<Scalar, Stop> {
    let int = |v: i128| {
        if ints.0 <= v && v <= ints.1 {
            Ok(Scalar::Int(v as i64))
        } else {
            Err(Stop::IntRange)
        }
    };
    let cmp = |o: std::cmp::Ordering| {
        use std::cmp::Ordering::*;
        Scalar::Bool(match op {
            ZipOp::Lt => o == Less,
            ZipOp::Gt => o == Greater,
            ZipOp::Le => o != Greater,
            ZipOp::Ge => o != Less,
            ZipOp::Eq => o == Equal,
            _ => o != Equal,
        })
    };
    match (a, b) {
        (Scalar::Bool(x), Scalar::Bool(y)) => match op {
            ZipOp::And => Ok(Scalar::Bool(x && y)),
            ZipOp::Or => Ok(Scalar::Bool(x || y)),
            _ => Err(Stop::Mistyped),
        },
        (Scalar::Bool(_), _) | (_, Scalar::Bool(_)) => Err(Stop::Mistyped),
        _ if op.connects() => Err(Stop::Mistyped),
        (Scalar::Int(x), Scalar::Int(y)) => {
            let (x, y) = (x as i128, y as i128);
            match op {
                ZipOp::Add => int(x + y),
                ZipOp::Sub => int(x - y),
                ZipOp::Mul => int(x * y),
                ZipOp::Div if y == 0 => Err(Stop::ZeroDivisor),
                ZipOp::Div => int(x / y),
                ZipOp::Pow if y < 0 => Err(Stop::NegativeExponent),
                ZipOp::Pow => u32::try_from(y).ok().and_then(|e| x.checked_pow(e)).ok_or(Stop::IntRange).and_then(int),
                _ => Ok(cmp(x.cmp(&y))),
            }
        }
        (a, b) => {
            let f = |v: Scalar| match v {
                Scalar::Int(i) => i as f64,
                Scalar::Real(r) => r,
                Scalar::Bool(_) => unreachable!("booleans are refused above"),
            };
            let (x, y) = (f(a), f(b));
            let real = |v: f64| bounded(Scalar::Real(v), ints, reals);
            match op {
                ZipOp::Add => real(x + y),
                ZipOp::Sub => real(x - y),
                ZipOp::Mul => real(x * y),
                ZipOp::Div => real(x / y),
                ZipOp::Pow => real(x.powf(y)),
                _ => x.partial_cmp(&y).map(cmp).ok_or(Stop::RealRange),
            }
        }
    }
}

/// A node of a [`TermDag`]: a [`ZipTerm`] node whose operands are nodes, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DagNode {
    Read(usize),
    Const(i64),
    Real(u64),
    Op(ZipOp, usize, usize),
    Select(usize, usize, usize),
    Abs(usize),
}

/// A term as a DAG: each distinct subterm once, every node after its operands — how a large term (a function
/// inlined at each call) is evaluated per element and printed with each shared subterm bound once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermDag {
    pub nodes: Vec<DagNode>,
    /// How many times each node is an operand of another.
    pub uses: Vec<usize>,
    pub root: usize,
}

impl TermDag {
    pub fn of(term: &ZipTerm) -> TermDag {
        fn intern(t: &ZipTerm, nodes: &mut Vec<DagNode>, index: &mut HashMap<DagNode, usize>) -> usize {
            let mut child = |c: &ZipTerm| intern(c, nodes, index);
            let node = match t {
                ZipTerm::Read(k) => DagNode::Read(*k),
                ZipTerm::Const(c) => DagNode::Const(*c),
                ZipTerm::Real(b) => DagNode::Real(*b),
                ZipTerm::Abs(x) => DagNode::Abs(child(x)),
                ZipTerm::Op(op, l, r) => {
                    let l = child(l);
                    DagNode::Op(*op, l, child(r))
                }
                ZipTerm::Select(c, a, b) => {
                    let (c, a) = (child(c), child(a));
                    DagNode::Select(c, a, child(b))
                }
            };
            *index.entry(node).or_insert_with(|| {
                nodes.push(node);
                nodes.len() - 1
            })
        }
        let (mut nodes, mut index) = (Vec::new(), HashMap::new());
        let root = intern(term, &mut nodes, &mut index);
        let mut uses = vec![0; nodes.len()];
        for n in &nodes {
            match *n {
                DagNode::Abs(x) => uses[x] += 1,
                DagNode::Op(_, l, r) => {
                    uses[l] += 1;
                    uses[r] += 1;
                }
                DagNode::Select(c, a, b) => {
                    uses[c] += 1;
                    uses[a] += 1;
                    uses[b] += 1;
                }
                _ => {}
            }
        }
        TermDag { nodes, uses, root }
    }

    /// Every node's type when operand `k` holds a `reads[k]` ([`ZipTerm::ty`] node by node); `None` when the term
    /// is ill-typed.
    pub fn types(&self, reads: &[Ty]) -> Option<Vec<Ty>> {
        let mut tys: Vec<Ty> = Vec::with_capacity(self.nodes.len());
        for n in &self.nodes {
            let t = match *n {
                DagNode::Read(k) => reads.get(k).copied(),
                DagNode::Const(_) => Some(Ty::Int),
                DagNode::Real(_) => Some(Ty::Real),
                DagNode::Abs(x) => abs_ty(tys[x]),
                DagNode::Op(op, l, r) => op_ty(op, tys[l], tys[r]),
                DagNode::Select(c, a, b) => select_ty(tys[c], tys[a], tys[b]),
            }?;
            tys.push(t);
        }
        Some(tys)
    }

    /// The term's value at one element, as [`ZipTerm::value`] computes it — each node at most once, and only the
    /// arm a select takes.
    pub fn value(&self, row: &[Scalar], ints: (i128, i128), reals: f64) -> Result<Scalar, Stop> {
        self.value_of(self.root, row, ints, reals)
    }

    /// The value of node `node` at one element, as [`Self::value`] computes the root's.
    pub fn value_of(&self, node: usize, row: &[Scalar], ints: (i128, i128), reals: f64) -> Result<Scalar, Stop> {
        fn at(
            d: &TermDag,
            i: usize,
            row: &[Scalar],
            ints: (i128, i128),
            reals: f64,
            memo: &mut Vec<Option<Result<Scalar, Stop>>>,
        ) -> Result<Scalar, Stop> {
            if let Some(v) = memo[i] {
                return v;
            }
            let v = match d.nodes[i] {
                DagNode::Read(k) => row.get(k).copied().ok_or(Stop::Mistyped).and_then(|v| bounded(v, ints, reals)),
                DagNode::Const(c) => bounded(Scalar::Int(c), ints, reals),
                DagNode::Real(b) => bounded(Scalar::Real(f64::from_bits(b)), ints, reals),
                DagNode::Abs(x) => at(d, x, row, ints, reals, memo).and_then(|v| abs_of(v, ints, reals)),
                DagNode::Op(op, l, r) => at(d, l, row, ints, reals, memo)
                    .and_then(|a| Ok((a, at(d, r, row, ints, reals, memo)?)))
                    .and_then(|(a, b)| apply(op, a, b, ints, reals)),
                DagNode::Select(c, a, b) => match at(d, c, row, ints, reals, memo) {
                    Ok(Scalar::Bool(true)) => at(d, a, row, ints, reals, memo),
                    Ok(Scalar::Bool(false)) => at(d, b, row, ints, reals, memo),
                    Ok(_) => Err(Stop::Mistyped),
                    Err(e) => Err(e),
                },
            };
            memo[i] = Some(v);
            v
        }
        at(self, node, row, ints, reals, &mut vec![None; self.nodes.len()])
    }
}

/// What a loop's term may draw on beyond the loop: the program's pure functions (inlined where they are called), the
/// builtin `abs`, and the element type of every array it declares (`a new Seq of Int`, `Seq of Real`, `Seq of Bool`).
pub struct Program<'a> {
    fns: HashMap<Symbol, PureFn<'a>>,
    abs: Option<Symbol>,
    /// Each array's element type; `None` for a name declared twice with different element types.
    elems: HashMap<Symbol, Option<Ty>>,
    prims: [(Option<Symbol>, Ty); 3],
}

/// A function a term may call: typed parameters and result, a body of immutable `Let`s, `If`s and `Return`s.
struct PureFn<'a> {
    params: Vec<(Symbol, Ty)>,
    ret: Ty,
    body: &'a [Stmt<'a>],
}

impl<'a> Program<'a> {
    pub fn of(stmts: &'a [Stmt<'a>], interner: &Interner) -> Program<'a> {
        let prims = [
            (interner.lookup("Int"), Ty::Int),
            (interner.lookup("Real"), Ty::Real),
            (interner.lookup("Bool"), Ty::Bool),
        ];
        let mut program = Program { fns: HashMap::new(), abs: interner.lookup("abs"), elems: HashMap::new(), prims };
        let seqs = [interner.lookup("Seq"), interner.lookup("List")];
        for s in stmts {
            if let Stmt::FunctionDef { name, generics, params, body, return_type: Some(ret), is_native: false, .. } = s {
                let typed: Option<Vec<(Symbol, Ty)>> =
                    params.iter().map(|(p, t)| program.ty_of(t).map(|ty| (*p, ty))).collect();
                if let (true, Some(params), Some(ret)) = (generics.is_empty(), typed, program.ty_of(ret)) {
                    program.fns.insert(*name, PureFn { params, ret, body });
                }
            }
        }
        fn declare<'a>(program: &mut Program<'a>, seqs: &[Option<Symbol>; 2], stmts: &'a [Stmt<'a>]) {
            for s in stmts {
                match s {
                    Stmt::Let { var, ty, value, .. } => {
                        let elem = match (ty, value) {
                            (Some(TypeExpr::Generic { base, params: [t] }), _) if seqs.contains(&Some(*base)) => {
                                program.ty_of(t)
                            }
                            (_, Expr::New { type_name, type_args, .. }) if seqs.contains(&Some(*type_name)) => {
                                match type_args.as_slice() {
                                    [t] => program.ty_of(t),
                                    _ => None,
                                }
                            }
                            _ => continue,
                        };
                        let entry = program.elems.entry(*var).or_insert(elem);
                        if *entry != elem {
                            *entry = None;
                        }
                    }
                    Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                        declare(program, seqs, body)
                    }
                    Stmt::If { then_block, else_block, .. } => {
                        declare(program, seqs, then_block);
                        declare(program, seqs, else_block.unwrap_or(&[]));
                    }
                    _ => {}
                }
            }
        }
        declare(&mut program, &seqs, stmts);
        program
    }

    /// The type a `TypeExpr` names, when it is `Int`, `Real` or `Bool`.
    fn ty_of(&self, t: &TypeExpr) -> Option<Ty> {
        match t {
            TypeExpr::Primitive(s) | TypeExpr::Named(s) => {
                self.prims.iter().find(|(p, _)| *p == Some(*s)).map(|(_, ty)| *ty)
            }
            _ => None,
        }
    }

    /// The element type of the array `array`, when it is declared once with one.
    pub fn elem(&self, array: Symbol) -> Option<Ty> {
        self.elems.get(&array).copied().flatten()
    }
}

/// Why an expression is not a term: it reads something a term cannot (or is ill-typed), an index is not affine, or
/// a read is not where the recognizer's layout puts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermReject {
    Unsupported,
    NonAffine,
    Layout,
}

/// How many nodes a term may grow to before it is refused — inlining copies a function's body at every call.
const MAX_TERM_NODES: usize = 1 << 20;

/// Builds [`ZipTerm`]s from a loop body's expressions: the body's immutable `Let`s read through, named constants
/// folded to their values, a call of one of the program's pure functions inlined (its parameters bound to the
/// call's argument terms, its `If`s and `Return`s a chain of selects), `abs` as itself. An array element becomes
/// the operand the recognizer's `read(array, index)` numbers it as, typed by the array's declared element type.
struct Terms<'c, 'a> {
    program: &'c Program<'a>,
    consts: &'c HashMap<Symbol, i64>,
    exprs: &'c HashMap<Symbol, &'a Expr<'a>>,
    /// Names a term may not read as constants: the loop counters and what the loop writes.
    hidden: Vec<Symbol>,
    /// Each operand's element type, by operand number (an operand the term has not read yet has none).
    types: Vec<Option<Ty>>,
    nodes: usize,
}

type Read<'r, 'a> = dyn FnMut(Symbol, &'a Expr<'a>) -> Result<usize, TermReject> + 'r;

impl<'c, 'a> Terms<'c, 'a> {
    fn new(
        program: &'c Program<'a>,
        consts: &'c HashMap<Symbol, i64>,
        exprs: &'c HashMap<Symbol, &'a Expr<'a>>,
        hidden: Vec<Symbol>,
    ) -> Self {
        Terms { program, consts, exprs, hidden, types: Vec::new(), nodes: 0 }
    }

    fn term(&mut self, e: &'a Expr<'a>, read: &mut Read<'_, 'a>) -> Result<ZipTerm, TermReject> {
        self.within(e, None, read, &mut Vec::new())
    }

    /// Every operand's element type, once each has been read.
    fn types(&self) -> Option<Vec<Ty>> {
        self.types.iter().copied().collect()
    }

    /// The type of `t` over the operands read so far.
    fn ty(&self, t: &ZipTerm) -> Option<Ty> {
        t.ty(&self.types.iter().map(|t| t.unwrap_or(Ty::Bool)).collect::<Vec<_>>())
    }

    /// `e` as a term. Inside an inlined function `env` binds its parameters and `Let`s, and nothing else is in scope.
    fn within(
        &mut self,
        e: &'a Expr<'a>,
        env: Option<&HashMap<Symbol, ZipTerm>>,
        read: &mut Read<'_, 'a>,
        calls: &mut Vec<Symbol>,
    ) -> Result<ZipTerm, TermReject> {
        self.nodes += 1;
        if self.nodes > MAX_TERM_NODES {
            return Err(TermReject::Unsupported);
        }
        let e = match env {
            Some(env) => match as_ident(e) {
                Some(s) => return env.get(&s).cloned().ok_or(TermReject::Unsupported),
                None => e,
            },
            None => through_lets(e, self.exprs),
        };
        match e {
            Expr::Literal(Literal::Number(n)) => Ok(ZipTerm::Const(*n)),
            Expr::Literal(Literal::Float(f)) => Ok(ZipTerm::real(*f)),
            Expr::Identifier(s) => self
                .consts
                .get(s)
                .filter(|_| !self.hidden.contains(s))
                .map(|c| ZipTerm::Const(*c))
                .ok_or(TermReject::Unsupported),
            Expr::Index { collection, index } if env.is_none() => {
                let array = as_ident(collection).ok_or(TermReject::Unsupported)?;
                let ty = self.program.elem(array).ok_or(TermReject::Unsupported)?;
                let k = read(array, index)?;
                if self.types.len() <= k {
                    self.types.resize(k + 1, None);
                }
                self.types[k] = Some(ty);
                Ok(ZipTerm::Read(k))
            }
            Expr::BinaryOp { op, left, right } => {
                let op = ZipOp::of(*op).ok_or(TermReject::Unsupported)?;
                let l = self.within(left, env, read, calls)?;
                let r = self.within(right, env, read, calls)?;
                Ok(ZipTerm::Op(op, Box::new(l), Box::new(r)))
            }
            Expr::Call { function, args } => self.call(*function, args, env, read, calls),
            _ => Err(TermReject::Unsupported),
        }
    }

    /// A call: one of the program's pure functions inlined, or `abs`. A function that calls itself, however
    /// indirectly, is refused.
    fn call(
        &mut self,
        function: Symbol,
        args: &[&'a Expr<'a>],
        env: Option<&HashMap<Symbol, ZipTerm>>,
        read: &mut Read<'_, 'a>,
        calls: &mut Vec<Symbol>,
    ) -> Result<ZipTerm, TermReject> {
        let program = self.program;
        if let Some(f) = program.fns.get(&function) {
            if calls.contains(&function) || args.len() != f.params.len() {
                return Err(TermReject::Unsupported);
            }
            let mut bound = HashMap::new();
            for ((p, want), a) in f.params.iter().zip(args) {
                let t = self.within(a, env, read, calls)?;
                if self.ty(&t) != Some(*want) {
                    return Err(TermReject::Unsupported);
                }
                bound.insert(*p, t);
            }
            calls.push(function);
            let body = self.body(f.body.iter().collect(), bound, read, calls);
            calls.pop();
            let body = body?;
            if self.ty(&body) != Some(f.ret) {
                return Err(TermReject::Unsupported);
            }
            Ok(body)
        } else if Some(function) == program.abs && args.len() == 1 {
            Ok(ZipTerm::Abs(Box::new(self.within(args[0], env, read, calls)?)))
        } else {
            Err(TermReject::Unsupported)
        }
    }

    /// A function body from `stmts[0]` on, as the term it returns: a `Let` binds, an `If` is the select of its two
    /// continuations (each arm followed by the statements after the `If`), a `Return` ends the path. A path that
    /// ends without a `Return`, or any other statement, is refused.
    fn body(
        &mut self,
        stmts: Vec<&'a Stmt<'a>>,
        mut env: HashMap<Symbol, ZipTerm>,
        read: &mut Read<'_, 'a>,
        calls: &mut Vec<Symbol>,
    ) -> Result<ZipTerm, TermReject> {
        for (k, s) in stmts.iter().enumerate() {
            match s {
                Stmt::Let { var, ty, value, mutable: false } => {
                    let t = self.within(value, Some(&env), read, calls)?;
                    if let Some(declared) = ty {
                        if self.program.ty_of(declared).is_none() || self.program.ty_of(declared) != self.ty(&t) {
                            return Err(TermReject::Unsupported);
                        }
                    }
                    env.insert(*var, t);
                }
                Stmt::Return { value: Some(v) } => return self.within(v, Some(&env), read, calls),
                Stmt::If { cond, then_block, else_block } => {
                    let c = self.within(cond, Some(&env), read, calls)?;
                    let rest = &stmts[k + 1..];
                    let path = |arm: &'a [Stmt<'a>]| arm.iter().chain(rest.iter().copied()).collect::<Vec<_>>();
                    let a = self.body(path(then_block), env.clone(), read, calls)?;
                    let b = self.body(path(else_block.unwrap_or(&[])), env.clone(), read, calls)?;
                    return Ok(ZipTerm::Select(Box::new(c), Box::new(a), Box::new(b)));
                }
                _ => return Err(TermReject::Unsupported),
            }
        }
        Err(TermReject::Unsupported)
    }
}

/// The reads of a zip loop: every array at one unit-stride index of the counter, numbered in reading order.
struct Aligned<'c> {
    iv: Symbol,
    consts: &'c HashMap<Symbol, i64>,
    lets: &'c HashMap<Symbol, Affine>,
    operands: Vec<Symbol>,
    index: Option<Affine>,
}

impl<'c> Aligned<'c> {
    fn read(&mut self, array: Symbol, index: &Expr) -> Result<usize, TermReject> {
        let at = eval_affine(index, self.consts, self.lets).ok_or(TermReject::NonAffine)?;
        if at.coeff.len() != 1 || at.coeff_of(self.iv) != 1 || self.index.as_ref().map_or(false, |i| *i != at) {
            return Err(TermReject::Layout);
        }
        self.index = Some(at);
        Ok(match self.operands.iter().position(|o| *o == array) {
            Some(k) => k,
            None => {
                self.operands.push(array);
                self.operands.len() - 1
            }
        })
    }
}

/// A recognized zip sum `acc = Σ_{t<n} f(A₀[t], A₁[t], …)`: one counted loop accumulating an integer term
/// of several `Seq of Int` arrays, every one read at the same unit-stride index — a dot product, TPC-H Q6's
/// filtered revenue, any map-reduce over aligned streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipSum {
    pub n: usize,
    /// The arrays read, in reading order; [`ZipTerm::Read`]`(k)` is the element of `operands[k]`.
    pub operands: Vec<Symbol>,
    pub term: ZipTerm,
    pub acc: Symbol,
}

/// Why a program is not a recognizable zip sum, least to most advanced (what a loop's body computes ranks
/// before whether its counter is provable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZipReject {
    /// No `While counter is less than BOUND` loop.
    NoLoop,
    /// No `Set s to s + …`, plain or under an `If`, in the loop body.
    NoAccumulation,
    /// The accumulated term reads something other than array elements and constants (the counter, the
    /// accumulator, a variable the loop does not fix), uses an operator or calls a function a term does not have,
    /// or is ill-typed.
    UnsupportedTerm,
    /// A well-typed term, but not an `Int` over `Int` arrays — the integer sum the zip-sum app computes.
    NotInt,
    /// An index is not affine in the counter.
    NonAffineIndex,
    /// The arrays are not all read at one unit-stride index of the counter.
    NotAligned,
    /// The counter is not set to a constant right before the loop, or is not stepped by exactly one.
    NotCounted,
    /// The loop body does more than the zip sum.
    ExtraWork,
    /// The accumulator is not set to zero right before the loop.
    NotFromZero,
}

impl From<TermReject> for ZipReject {
    fn from(r: TermReject) -> ZipReject {
        match r {
            TermReject::Unsupported => ZipReject::UnsupportedTerm,
            TermReject::NonAffine => ZipReject::NonAffineIndex,
            TermReject::Layout => ZipReject::NotAligned,
        }
    }
}

/// Recognize a zip sum anywhere in a LOGOS statement list:
///
/// ```text
/// Let mutable s be 0.
/// Let mutable i be 0.
/// While i is less than n:
///     Let date be item (i + 1) of dates.
///     If date is greater than 0 and date is less than 9999:
///         Set s to s + item (i + 1) of prices * item (i + 1) of discts.
///     Set i to i + 1.
/// ```
///
/// The loop is counted (its counter set to a constant right before it, stepped by one), the accumulator
/// starts at zero right before it, every array is a declared `Seq of Int` read at the same unit-stride index, the
/// accumulation is `Set s to s + E` or an `If` whose arms are each exactly one such accumulation (a missing `Else`
/// adds 0), and the body holds nothing else but immutable names for elements and subterms. The first loop that is
/// one is returned; otherwise the refusal of the loop that came closest.
pub fn recognize_zip_sum(stmts: &[Stmt], interner: &Interner) -> Result<ZipSum, ZipReject> {
    recognize_zip_sum_where(stmts, interner, &|_| true)
}

/// [`recognize_zip_sum`] for the first loop whose zip sum `wanted` accepts; a zip sum it turns away counts as
/// [`ZipReject::UnsupportedTerm`] (its term is not the one asked for).
pub fn recognize_zip_sum_where(
    stmts: &[Stmt],
    interner: &Interner,
    wanted: &dyn Fn(&ZipSum) -> bool,
) -> Result<ZipSum, ZipReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    let program = Program::of(stmts, interner);
    search_loops(stmts, ZipReject::NoLoop, &|block, at, cond, body| {
        try_zip_at(block, at, cond, body, &consts, &program)
            .and_then(|z| if wanted(&z) { Ok(z) } else { Err(ZipReject::UnsupportedTerm) })
    })
}

/// `Set acc to acc + E` (either operand order), `acc` not `iv`: `(acc, E)`.
fn accumulation_into<'a>(s: &'a Stmt<'a>, iv: Symbol) -> Option<(Symbol, &'a Expr<'a>)> {
    match s {
        Stmt::Set { target, value } if *target != iv => {
            let (l, r) = as_binop(value, BinaryOpKind::Add)?;
            if as_ident(l) == Some(*target) {
                Some((*target, r))
            } else if as_ident(r) == Some(*target) {
                Some((*target, l))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// A loop body's accumulation statement: plain, or an `If` whose arms each hold exactly one accumulation into
/// the same register — `(acc, guard, then-term, else-term)`.
fn accumulation_of<'a>(
    s: &'a Stmt<'a>,
    iv: Symbol,
) -> Option<(Symbol, Option<&'a Expr<'a>>, &'a Expr<'a>, Option<&'a Expr<'a>>)> {
    if let Some((acc, e)) = accumulation_into(s, iv) {
        return Some((acc, None, e, None));
    }
    let Stmt::If { cond, then_block, else_block } = s else { return None };
    let [then] = then_block else { return None };
    let (acc, e1) = accumulation_into(then, iv)?;
    let e2 = match else_block {
        None => None,
        Some([other]) => match accumulation_into(other, iv)? {
            (a2, e2) if a2 == acc => Some(e2),
            _ => return None,
        },
        Some(_) => return None,
    };
    Some((acc, Some(*cond), e1, e2))
}

fn try_zip_at<'a>(
    stmts: &'a [Stmt<'a>],
    at: usize,
    cond: &'a Expr<'a>,
    body: &'a [Stmt<'a>],
    consts: &HashMap<Symbol, i64>,
    program: &Program<'a>,
) -> Result<ZipSum, ZipReject> {
    let bound = loop_iv_bound(cond, consts).ok_or(ZipReject::NoLoop)?;
    let iv = bound.0;
    let (lets, exprs) = body_lets(body, consts);
    let (pos, (acc, guard, then, otherwise)) = body
        .iter()
        .enumerate()
        .find_map(|(k, s)| accumulation_of(s, iv).map(|a| (k, a)))
        .ok_or(ZipReject::NoAccumulation)?;
    // Converted in source order — the guard before its arms — so the operands are numbered as they are read.
    let mut aligned = Aligned { iv, consts, lets: &lets, operands: Vec::new(), index: None };
    let mut terms = Terms::new(program, consts, &exprs, vec![iv, acc]);
    let mut read = |a, i| aligned.read(a, i);
    let term = match guard {
        None => terms.term(then, &mut read)?,
        Some(g) => {
            let g = terms.term(g, &mut read)?;
            let value = terms.term(then, &mut read)?;
            let e = match otherwise {
                Some(e) => terms.term(e, &mut read)?,
                None => ZipTerm::Const(0),
            };
            ZipTerm::Select(Box::new(g), Box::new(value), Box::new(e))
        }
    };
    let types = terms.types().ok_or(ZipReject::UnsupportedTerm)?;
    match term.ty(&types) {
        None | Some(Ty::Bool) => return Err(ZipReject::UnsupportedTerm),
        Some(Ty::Real) => return Err(ZipReject::NotInt),
        Some(Ty::Int) if types.iter().any(|t| *t != Ty::Int) => return Err(ZipReject::NotInt),
        Some(Ty::Int) => {}
    }
    let operands = aligned.operands;
    let loop_ = counted_at(stmts, at, bound, body, consts).ok_or(ZipReject::NotCounted)?;
    let mut kept = vec![iv, acc];
    kept.extend(&operands);
    if body
        .iter()
        .enumerate()
        .any(|(k, s)| !(k == pos || is_unit_step(s, iv) || is_body_let(s, &kept)))
    {
        return Err(ZipReject::ExtraWork);
    }
    if entry_values(stmts, at, consts).get(&acc).copied().flatten() != Some(0) {
        return Err(ZipReject::NotFromZero);
    }
    Ok(ZipSum {
        n: loop_.trips as usize,
        operands,
        term,
        acc,
    })
}

/// A recognized zip map `out[t] = f(A₀[t], A₁[t], …)` for `t < n`: one counted loop storing a term of several
/// arrays into another, every one read at the index it stores at — Black-Scholes over its option columns, any
/// elementwise map over aligned streams. The term's type is the output's element type, `Int` or `Real`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipMap {
    pub n: usize,
    /// The arrays read, in reading order; [`ZipTerm::Read`]`(k)` is the element of `operands[k]`.
    pub operands: Vec<Symbol>,
    /// Each operand's element type.
    pub types: Vec<Ty>,
    pub term: ZipTerm,
    /// The term's type: the element type of `out`.
    pub ty: Ty,
    pub out: Symbol,
}

/// Why a program is not a recognizable zip map, least to most advanced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZipMapReject {
    /// No `While counter is less than BOUND` loop.
    NoLoop,
    /// No `Set item _ of out to …` in the loop body.
    NoStore,
    /// The stored term reads something other than array elements and constants (the counter, the output, a
    /// variable the loop does not fix), reads no array, uses an operator or calls a function a term does not have,
    /// is ill-typed or a boolean, or is not of the output's element type.
    UnsupportedTerm,
    /// An index is not affine in the counter.
    NonAffineIndex,
    /// The arrays are not all read at one unit-stride index of the counter, the index the output is written at.
    NotAligned,
    /// The counter is not set to a constant right before the loop, or is not stepped by exactly one.
    NotCounted,
    /// The loop body does more than the zip map.
    ExtraWork,
}

impl From<TermReject> for ZipMapReject {
    fn from(r: TermReject) -> ZipMapReject {
        match r {
            TermReject::Unsupported => ZipMapReject::UnsupportedTerm,
            TermReject::NonAffine => ZipMapReject::NonAffineIndex,
            TermReject::Layout => ZipMapReject::NotAligned,
        }
    }
}

/// Recognize a zip map anywhere in a LOGOS statement list:
///
/// ```text
/// Let mutable i be 0.
/// While i is less than n:
///     Set item (i + 1) of out to blackScholes(item (i + 1) of prices, item (i + 1) of strikes, …).
///     Set i to i + 1.
/// ```
///
/// The loop is counted; it writes one array, `out`, at a unit-stride index of the counter, and every array the
/// stored term reads is a declared `Seq` read at that same index; the term calls only the program's pure functions
/// (each inlined) and `abs`; and the body holds nothing else but immutable names. The first loop that is one is
/// returned; otherwise the refusal of the loop that came closest.
pub fn recognize_zip_map(stmts: &[Stmt], interner: &Interner) -> Result<ZipMap, ZipMapReject> {
    recognize_zip_map_where(stmts, interner, &|_| true)
}

/// [`recognize_zip_map`] for the first loop whose zip map `wanted` accepts; one it turns away counts as
/// [`ZipMapReject::UnsupportedTerm`] (its term is not the one asked for).
pub fn recognize_zip_map_where(
    stmts: &[Stmt],
    interner: &Interner,
    wanted: &dyn Fn(&ZipMap) -> bool,
) -> Result<ZipMap, ZipMapReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    let program = Program::of(stmts, interner);
    search_loops(stmts, ZipMapReject::NoLoop, &|block, at, cond, body| {
        try_map_at(block, at, cond, body, &consts, &program)
            .and_then(|z| if wanted(&z) { Ok(z) } else { Err(ZipMapReject::UnsupportedTerm) })
    })
}

fn try_map_at<'a>(
    stmts: &'a [Stmt<'a>],
    at: usize,
    cond: &'a Expr<'a>,
    body: &'a [Stmt<'a>],
    consts: &HashMap<Symbol, i64>,
    program: &Program<'a>,
) -> Result<ZipMap, ZipMapReject> {
    let bound = loop_iv_bound(cond, consts).ok_or(ZipMapReject::NoLoop)?;
    let iv = bound.0;
    let (lets, exprs) = body_lets(body, consts);
    let (pos, out, index, value) = body
        .iter()
        .enumerate()
        .find_map(|(k, s)| match s {
            Stmt::SetIndex { collection, index, value } => Some((k, as_ident(collection)?, *index, *value)),
            _ => None,
        })
        .ok_or(ZipMapReject::NoStore)?;
    let mut aligned = Aligned { iv, consts, lets: &lets, operands: Vec::new(), index: None };
    let mut terms = Terms::new(program, consts, &exprs, vec![iv, out]);
    let term = terms.term(value, &mut |a, i| aligned.read(a, i))?;
    let types = terms.types().ok_or(ZipMapReject::UnsupportedTerm)?;
    let ty = term.ty(&types).filter(|t| *t != Ty::Bool).ok_or(ZipMapReject::UnsupportedTerm)?;
    if aligned.operands.is_empty() || aligned.operands.contains(&out) || program.elem(out) != Some(ty) {
        return Err(ZipMapReject::UnsupportedTerm);
    }
    let stored = eval_affine(index, consts, &lets).ok_or(ZipMapReject::NonAffineIndex)?;
    if aligned.index.as_ref() != Some(&stored) {
        return Err(ZipMapReject::NotAligned);
    }
    let loop_ = counted_at(stmts, at, bound, body, consts).ok_or(ZipMapReject::NotCounted)?;
    let mut kept = vec![iv, out];
    kept.extend(&aligned.operands);
    if body
        .iter()
        .enumerate()
        .any(|(k, s)| !(k == pos || is_unit_step(s, iv) || is_body_let(s, &kept)))
    {
        return Err(ZipMapReject::ExtraWork);
    }
    Ok(ZipMap {
        n: loop_.trips as usize,
        operands: aligned.operands,
        types,
        term,
        ty,
        out,
    })
}

/// Recognize an outer product anywhere in a LOGOS statement list:
///
/// ```text
/// Let mutable i be 0.
/// While i is less than m:
///     Let mutable j be 0.
///     While j is less than n:
///         Let idx be i * n + j + 1.
///         Set item idx of c to item (i + 1) of a * item (j + 1) of b.
///         Set j to j + 1.
///     Set i to i + 1.
/// ```
///
/// Both loops are counted; the store is `C[i·n + j]` (row-major against the inner trip count), `A` is read
/// at a unit stride of `i` alone and `B` of `j` alone; and the bodies hold nothing else but immutable index
/// or product names. `C[idx] := C[idx] + A·B` is refused by name: it is a rank-1 update.
pub fn recognize_outer(stmts: &[Stmt], interner: &Interner) -> Result<OuterSpec, OuterReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    let program = Program::of(stmts, interner);
    search_loops(stmts, OuterReject::NoLoopNest, &|block, at, cond, body| {
        outer_nest_at(block, at, cond, body, &consts, &program, false)
    })
}

/// The index of `e` when it reads `item _ of array`.
fn index_read_of<'a>(e: &'a Expr<'a>, array: Symbol) -> Option<&'a Expr<'a>> {
    as_index(e).filter(|(coll, _)| as_ident(coll) == Some(array)).map(|(_, i)| i)
}

/// The counted nest `i < m, j < n` at `stmts[at]` writing `C[i·n + j]` from `A[i]·B[j]`: stored (an outer product)
/// or, when `accumulate`, added to `C[i·n + j]` (a rank-1 update).
fn outer_nest_at(
    stmts: &[Stmt],
    at: usize,
    cond_i: &Expr,
    body_i: &[Stmt],
    consts: &HashMap<Symbol, i64>,
    program: &Program,
    accumulate: bool,
) -> Result<OuterSpec, OuterReject> {
    let bound_i = loop_iv_bound(cond_i, consts).ok_or(OuterReject::NoLoopNest)?;
    let (at_j, cond_j, body_j) = body_i
        .iter()
        .enumerate()
        .find_map(|(k, s)| match s {
            Stmt::While { cond, body, .. } => Some((k, *cond, *body)),
            _ => None,
        })
        .ok_or(OuterReject::NotDoubleNested)?;
    let bound_j = loop_iv_bound(cond_j, consts).ok_or(OuterReject::NotDoubleNested)?;
    let (lets, exprs) = body_lets(body_i[..at_j].iter().chain(body_j), consts);

    let (store_coll, store_idx, value) = body_j
        .iter()
        .find_map(|s| match s {
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => Some((*collection, *index, *value)),
            _ => None,
        })
        .ok_or(OuterReject::NoStore)?;
    let c = as_ident(store_coll).ok_or(OuterReject::NoStore)?;
    let value = through_lets(value, &exprs);
    // `C[idx] + A·B` (either order): the read of C and the product beside it.
    let accumulated = as_binop(value, BinaryOpKind::Add).and_then(|(l, r)| {
        match (index_read_of(l, c), index_read_of(r, c)) {
            (Some(ri), _) if indexed_product(r, &exprs).is_some() => Some((ri, r)),
            (_, Some(ri)) if indexed_product(l, &exprs).is_some() => Some((ri, l)),
            _ => None,
        }
    });
    let value = match (accumulated, accumulate) {
        (Some(_), false) => return Err(OuterReject::Accumulates),
        (None, true) => {
            return Err(if indexed_product(value, &exprs).is_some() { OuterReject::Stores } else { OuterReject::NoProduct })
        }
        (Some((read_index, product)), true) => {
            let ri = eval_affine(read_index, consts, &lets).ok_or(OuterReject::NonAffineIndex)?;
            if Some(ri) != eval_affine(store_idx, consts, &lets) {
                return Err(OuterReject::NotOuterLayout);
            }
            product
        }
        (None, false) => value,
    };
    let (a, ai, b, bi) = indexed_product(value, &exprs).ok_or(OuterReject::NoProduct)?;
    if [a, b, c].iter().any(|x| program.elem(*x) != Some(Ty::Int)) {
        return Err(OuterReject::NotInt);
    }
    let ci = eval_affine(store_idx, consts, &lets).ok_or(OuterReject::NonAffineIndex)?;
    let ai = eval_affine(ai, consts, &lets).ok_or(OuterReject::NonAffineIndex)?;
    let bi = eval_affine(bi, consts, &lets).ok_or(OuterReject::NonAffineIndex)?;
    let li = counted_at(stmts, at, bound_i, body_i, consts).ok_or(OuterReject::NotCounted)?;
    let lj = counted_at(body_i, at_j, bound_j, body_j, consts).ok_or(OuterReject::NotCounted)?;
    let (iv_i, iv_j, n) = (li.iv, lj.iv, lj.trips);
    let only = |x: &Affine, pairs: &[(Symbol, i64)]| {
        x.coeff.len() == pairs.len() && pairs.iter().all(|(s, k)| x.coeff_of(*s) == *k)
    };
    if !(only(&ci, &[(iv_i, n), (iv_j, 1)]) && only(&ai, &[(iv_i, 1)]) && only(&bi, &[(iv_j, 1)])) {
        return Err(OuterReject::NotOuterLayout);
    }
    let kept = [iv_i, iv_j, a, b, c];
    let store = |s: &Stmt| matches!(s, Stmt::SetIndex { .. });
    let inner_ok = body_j
        .iter()
        .all(|s| is_unit_step(s, iv_j) || store(s) || is_body_let(s, &kept))
        && body_j.iter().filter(|s| store(s)).count() == 1;
    let j_init = |k: usize| {
        k < at_j
            && body_i[k..at_j]
                .iter()
                .all(|s| matches!(s, Stmt::Let { .. } | Stmt::Set { .. }))
            && matches!(&body_i[k], Stmt::Let { var, .. } | Stmt::Set { target: var, .. } if *var == iv_j)
    };
    let outer_ok = body_i.iter().enumerate().all(|(k, s)| {
        k == at_j || is_unit_step(s, iv_i) || j_init(k) || is_body_let(s, &kept)
    }) && body_i
        .iter()
        .filter(|s| matches!(s, Stmt::While { .. }))
        .count()
        == 1;
    if !(inner_ok && outer_ok) {
        return Err(OuterReject::ExtraWork);
    }
    Ok(OuterSpec {
        m: li.trips as usize,
        n: n as usize,
        a,
        b,
        c,
    })
}

/// `term` with every select whose two arms apply one operator to a common operand rewritten to apply it once, to the
/// select of what differs — `c ? x − a : x − b` becomes `x − (c ? a : b)`, and the same for `+`, `×` and a common
/// right operand. Exact for every value: both forms apply the same operator to the same operands of the arm taken.
pub fn hoist_common_operand(term: ZipTerm) -> ZipTerm {
    match term {
        ZipTerm::Op(op, l, r) => ZipTerm::Op(op, Box::new(hoist_common_operand(*l)), Box::new(hoist_common_operand(*r))),
        ZipTerm::Abs(x) => ZipTerm::Abs(Box::new(hoist_common_operand(*x))),
        ZipTerm::Select(c, a, b) => {
            let (c, a, b) = (hoist_common_operand(*c), hoist_common_operand(*a), hoist_common_operand(*b));
            match (a, b) {
                (ZipTerm::Op(o1, l1, r1), ZipTerm::Op(o2, l2, r2))
                    if o1 == o2 && matches!(o1, ZipOp::Add | ZipOp::Sub | ZipOp::Mul) && (l1 == l2 || r1 == r2) =>
                {
                    let select = |x: ZipTerm, y: ZipTerm| hoist_common_operand(ZipTerm::Select(Box::new(c.clone()), Box::new(x), Box::new(y)));
                    if l1 == l2 {
                        ZipTerm::Op(o1, l1, Box::new(select(*r1, *r2)))
                    } else {
                        ZipTerm::Op(o1, Box::new(select(*l1, *l2)), r1)
                    }
                }
                (a, b) => ZipTerm::Select(Box::new(c), Box::new(a), Box::new(b)),
            }
        }
        leaf => leaf,
    }
}

/// How a scatter row's term reads an array: at the row (`y[r]`), at the column (`mu[c]`), or at both, row-major
/// (`x[r·C + c]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    Row,
    Col,
    RowCol,
}

/// A recognized scatter-matrix accumulation `S = Σ_{r<R} d_r d_rᵀ` (`C × C`, row-major, from zero): each row's vector
/// `d_r[c] = term` computed into a scratch array by an elementwise loop over the row, then added to `S` as a rank-1
/// update of itself — the covariance step of Gaussian discriminant analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScatterSpec {
    pub rows: usize,
    pub cols: usize,
    /// The scratch row and the accumulated matrix.
    pub sub: Symbol,
    pub sigma: Symbol,
    /// The arrays the row term reads, in reading order, each with its axis; [`ZipTerm::Read`]`(k)` is
    /// `operands[k]` at that axis.
    pub operands: Vec<(Symbol, Axis)>,
    /// The row term, a select's common operand hoisted out of its arms ([`hoist_common_operand`]).
    pub term: ZipTerm,
}

/// Why a program is not a recognizable scatter accumulation, least to most advanced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScatterReject {
    /// No `While counter is less than BOUND` loop.
    NoLoop,
    /// A loop with no elementwise loop over the row inside it.
    NoRowLoop,
    /// The row loop does not write one array at its counter (`Set item (c + k) of sub to …`, or an `If` whose two
    /// arms each write it).
    NoRowStore,
    /// The row term reads something other than array elements and constants, uses an operator or calls a function a
    /// term does not have, or is ill-typed.
    UnsupportedTerm,
    /// A well-typed row term, but not an `Int` over `Int` arrays — the integer covariance the app computes.
    NotInt,
    /// An index is not affine in the counters.
    NonAffineIndex,
    /// A read that is not at the row, the column, or the row-major element of both.
    NotScatterLayout,
    /// The row's vector is not added to a `C × C` matrix as a rank-1 update of itself (`S[i·C + j] += sub[i]·sub[j]`).
    NoRank1Update,
    /// A counter not set to a constant right before its loop, or not stepped by exactly one.
    NotCounted,
    /// A loop body does more than the scatter accumulation.
    ExtraWork,
    /// The matrix is not filled with zeros before the rows are added.
    NotFromZero,
}

/// Recognize a scatter accumulation anywhere in a LOGOS statement list:
///
/// ```text
/// Let mutable r be 0.
/// While r is less than rows:
///     Let label be item (r + 1) of y.
///     Let mutable c be 0.
///     While c is less than cols:
///         If label is 1:
///             Set item (c + 1) of sub to item (r * cols + c + 1) of x - item (c + 1) of mu1.
///         Otherwise:
///             Set item (c + 1) of sub to item (r * cols + c + 1) of x - item (c + 1) of mu0.
///         Set c to c + 1.
///     Let mutable i be 0.
///     While i is less than cols:
///         Let mutable j be 0.
///         While j is less than cols:
///             Let idx be i * cols + j + 1.
///             Set item idx of sigma to item idx of sigma + item (i + 1) of sub * item (j + 1) of sub.
///             Set j to j + 1.
///         Set i to i + 1.
///     Set r to r + 1.
/// ```
///
/// Every loop is counted; the row loop writes the scratch row at its counter from reads at the row, the column, or
/// the row-major element; the rank-1 update adds the scratch row's outer product with itself to a zero-filled
/// `C × C` matrix; and every body holds nothing else but immutable names.
pub fn recognize_scatter(stmts: &[Stmt], interner: &Interner) -> Result<ScatterSpec, ScatterReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    let program = Program::of(stmts, interner);
    let spec = search_loops(stmts, ScatterReject::NoLoop, &|block, at, cond, body| {
        try_scatter_at(block, at, cond, body, &consts, &program)
    })?;
    if recognize_fill(stmts, spec.sigma) != Fill::Const(0) {
        return Err(ScatterReject::NotFromZero);
    }
    Ok(spec)
}

/// The reads of one scatter row term: each array with its axis, against the row counter `r`, the column counter
/// `c` and the row length `cols`.
struct Rows<'c> {
    r: Symbol,
    c: Symbol,
    cols: i64,
    consts: &'c HashMap<Symbol, i64>,
    lets: &'c HashMap<Symbol, Affine>,
    operands: Vec<(Symbol, Axis)>,
}

impl<'c> Rows<'c> {
    fn read(&mut self, array: Symbol, index: &Expr) -> Result<usize, TermReject> {
        let at = eval_affine(index, self.consts, self.lets).ok_or(TermReject::NonAffine)?;
        let (cr, cc) = (at.coeff_of(self.r), at.coeff_of(self.c));
        let axis = match (at.coeff.len(), cr, cc) {
            (1, 1, 0) => Axis::Row,
            (1, 0, 1) => Axis::Col,
            (2, r, 1) if r == self.cols => Axis::RowCol,
            _ => return Err(TermReject::Layout),
        };
        match self.operands.iter().position(|(o, _)| *o == array) {
            Some(k) if self.operands[k].1 == axis => Ok(k),
            Some(_) => Err(TermReject::Layout),
            None => {
                self.operands.push((array, axis));
                Ok(self.operands.len() - 1)
            }
        }
    }
}

impl From<TermReject> for ScatterReject {
    fn from(r: TermReject) -> ScatterReject {
        match r {
            TermReject::Unsupported => ScatterReject::UnsupportedTerm,
            TermReject::NonAffine => ScatterReject::NonAffineIndex,
            TermReject::Layout => ScatterReject::NotScatterLayout,
        }
    }
}

/// The row loop's write: `Set item IDX of sub to T` alone, or an `If P` whose two arms each write `sub` at the same
/// index — `(sub, IDX, guard, T, else-T)`.
fn row_store<'a>(
    s: &'a Stmt<'a>,
) -> Option<(Symbol, &'a Expr<'a>, Option<&'a Expr<'a>>, &'a Expr<'a>, Option<&'a Expr<'a>>)> {
    let store = |s: &'a Stmt<'a>| match s {
        Stmt::SetIndex { collection, index, value } => Some((as_ident(collection)?, *index, *value)),
        _ => None,
    };
    if let Some((sub, idx, value)) = store(s) {
        return Some((sub, idx, None, value, None));
    }
    let Stmt::If { cond, then_block, else_block: Some(else_block) } = s else { return None };
    let ([then], [other]) = (then_block, else_block) else { return None };
    let ((sub, idx, a), (sub2, _, b)) = (store(then)?, store(other)?);
    (sub == sub2).then_some((sub, idx, Some(*cond), a, Some(b)))
}

fn try_scatter_at<'a>(
    stmts: &'a [Stmt<'a>],
    at: usize,
    cond: &'a Expr<'a>,
    body: &'a [Stmt<'a>],
    consts: &HashMap<Symbol, i64>,
    program: &Program<'a>,
) -> Result<ScatterSpec, ScatterReject> {
    let bound_r = loop_iv_bound(cond, consts).ok_or(ScatterReject::NoLoop)?;
    let loops: Vec<(usize, &Expr, &[Stmt])> = body
        .iter()
        .enumerate()
        .filter_map(|(k, s)| match s {
            Stmt::While { cond, body, .. } => Some((k, *cond, *body)),
            _ => None,
        })
        .collect();
    let (at_c, cond_c, body_c) = *loops.first().ok_or(ScatterReject::NoRowLoop)?;
    let bound_c = loop_iv_bound(cond_c, consts).ok_or(ScatterReject::NoRowLoop)?;
    let (pos, (sub, idx, guard, then, otherwise)) = body_c
        .iter()
        .enumerate()
        .find_map(|(k, s)| row_store(s).map(|w| (k, w)))
        .ok_or(ScatterReject::NoRowStore)?;
    let lr = counted_at(stmts, at, bound_r, body, consts).ok_or(ScatterReject::NotCounted)?;
    let lc = counted_at(body, at_c, bound_c, body_c, consts).ok_or(ScatterReject::NotCounted)?;
    let (lets, exprs) = body_lets(body[..at_c].iter().chain(body_c), consts);
    let at_sub = eval_affine(idx, consts, &lets).ok_or(ScatterReject::NonAffineIndex)?;
    if at_sub.coeff.len() != 1 || at_sub.coeff_of(lc.iv) != 1 {
        return Err(ScatterReject::NoRowStore);
    }
    let mut rows = Rows { r: lr.iv, c: lc.iv, cols: lc.trips, consts, lets: &lets, operands: Vec::new() };
    let mut terms = Terms::new(program, consts, &exprs, vec![lr.iv, lc.iv, sub]);
    let mut read = |a, i| rows.read(a, i);
    let term = match guard {
        None => terms.term(then, &mut read)?,
        Some(g) => {
            let g = terms.term(g, &mut read)?;
            let (a, b) = (terms.term(then, &mut read)?, terms.term(otherwise.expect("both arms"), &mut read)?);
            ZipTerm::Select(Box::new(g), Box::new(a), Box::new(b))
        }
    };
    let types = terms.types().ok_or(ScatterReject::UnsupportedTerm)?;
    match term.ty(&types) {
        None | Some(Ty::Bool) => return Err(ScatterReject::UnsupportedTerm),
        Some(Ty::Int) if types.iter().all(|t| *t == Ty::Int) => {}
        Some(_) => return Err(ScatterReject::NotInt),
    }
    let (at_i, cond_i, body_i) = *loops.get(1).ok_or(ScatterReject::NoRank1Update)?;
    let nest = outer_nest_at(body, at_i, cond_i, body_i, consts, program, true).map_err(|e| match e {
        OuterReject::NotCounted => ScatterReject::NotCounted,
        OuterReject::ExtraWork => ScatterReject::ExtraWork,
        _ => ScatterReject::NoRank1Update,
    })?;
    if (nest.a, nest.b) != (sub, sub) || (nest.m as i64, nest.n as i64) != (lc.trips, lc.trips) {
        return Err(ScatterReject::NoRank1Update);
    }
    let mut kept = vec![lr.iv, lc.iv, sub, nest.c];
    kept.extend(rows.operands.iter().map(|(o, _)| *o));
    let init_of = |block: &[Stmt], at_loop: usize, iv: Symbol, k: usize| {
        k < at_loop
            && block[k..at_loop].iter().all(|s| matches!(s, Stmt::Let { .. } | Stmt::Set { .. }))
            && matches!(&block[k], Stmt::Let { var, .. } | Stmt::Set { target: var, .. } if *var == iv)
    };
    let i_iv = loop_iv_bound(cond_i, consts).map(|(iv, _)| iv);
    let row_ok = body.iter().enumerate().all(|(k, s)| {
        k == at_c
            || k == at_i
            || is_unit_step(s, lr.iv)
            || init_of(body, at_c, lc.iv, k)
            || i_iv.map_or(false, |iv| init_of(body, at_i, iv, k))
            || is_body_let(s, &kept)
    }) && loops.len() == 2;
    let col_ok = body_c.iter().enumerate().all(|(k, s)| k == pos || is_unit_step(s, lc.iv) || is_body_let(s, &kept));
    if !(row_ok && col_ok) {
        return Err(ScatterReject::ExtraWork);
    }
    Ok(ScatterSpec {
        rows: lr.trips as usize,
        cols: lc.trips as usize,
        sub,
        sigma: nest.c,
        operands: rows.operands,
        term: hoist_common_operand(term),
    })
}

/// Which of several equally near centroids a point joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ties {
    /// The first nearest (`If c is 0 or dist is less than bestDist`).
    First,
    /// The last nearest (`If c is 0 or dist is at most bestDist`).
    Last,
}

/// A recognized k-means clustering (Lloyd's algorithm): `n` points of `f` integer features, row-major in `points`,
/// around `k` centroids, row-major in `centroids`, which start as the first `k` points. Each of `iters` epochs
/// assigns every point to its nearest centroid by the distance `Σ_d term(point[d], centroid[d])` (ties as `ties`
/// says), sums each centroid's points and counts them, then sets every centroid to the integer mean of its points —
/// 0 for a centroid with none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KmeansSpec {
    pub n: usize,
    pub k: usize,
    pub f: usize,
    pub iters: usize,
    pub points: Symbol,
    pub centroids: Symbol,
    /// The distance a feature adds: [`ZipTerm::Read`]`(0)` the point's, `Read(1)` the centroid's.
    pub distance: ZipTerm,
    pub ties: Ties,
}

impl KmeansSpec {
    /// The final centroids (`k × f`, row-major) for the points `points` (`n × f`, row-major), as the program computes
    /// them; refused (with why) when a distance, a sum of distance terms in any order, or a feature sum over the
    /// points in any order leaves `ints`.
    pub fn run(&self, points: &[i64], ints: (i128, i128)) -> Result<Vec<i64>, Stop> {
        let (n, k, f) = (self.n, self.k, self.f);
        if points.len() < n * f || n < k {
            return Err(Stop::Mistyped);
        }
        let within = |v: i128| if ints.0 <= v && v <= ints.1 { Ok(v as i64) } else { Err(Stop::IntRange) };
        for d in 0..f {
            within((0..n).map(|p| (points[p * f + d] as i128).abs()).sum())?;
        }
        within(n as i128)?;
        let dag = TermDag::of(&self.distance);
        let mut cts: Vec<i64> = points[..k * f].to_vec();
        for _ in 0..self.iters {
            let (mut sums, mut counts) = (vec![0i64; k * f], vec![0i64; k]);
            for p in 0..n {
                let (mut best, mut best_dist) = (0usize, 0i64);
                for c in 0..k {
                    let (mut dist, mut magnitude) = (0i128, 0i128);
                    for d in 0..f {
                        let row = [Scalar::Int(points[p * f + d]), Scalar::Int(cts[c * f + d])];
                        let Scalar::Int(v) = dag.value(&row, ints, f64::MAX)? else { return Err(Stop::Mistyped) };
                        dist += v as i128;
                        magnitude += (v as i128).abs();
                        within(magnitude)?;
                    }
                    let dist = within(dist)?;
                    let nearer = match self.ties {
                        Ties::First => dist < best_dist,
                        Ties::Last => dist <= best_dist,
                    };
                    if c == 0 || nearer {
                        best = c;
                        best_dist = dist;
                    }
                }
                for d in 0..f {
                    sums[best * f + d] += points[p * f + d];
                }
                counts[best] += 1;
            }
            for c in 0..k {
                for d in 0..f {
                    cts[c * f + d] = if counts[c] == 0 { 0 } else { sums[c * f + d] / counts[c] };
                }
            }
        }
        Ok(cts)
    }
}

/// Why a program is not a recognizable k-means clustering, least to most advanced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KmeansReject {
    /// No `While counter is less than BOUND` loop.
    NoLoop,
    /// No loop over the points whose body finds each point's nearest centroid: a loop over the centroids around a
    /// loop over the features.
    NoAssignment,
    /// The loop over the features does not add a term into one register.
    NoDistance,
    /// The distance term reads something other than the point's and the centroid's feature and constants, uses an
    /// operator or calls a function a term does not have, or is ill-typed.
    UnsupportedTerm,
    /// A distance, but not an `Int` over `Int` arrays — the integer clustering the app computes.
    NotInt,
    /// An index is not affine in the counters.
    NonAffineIndex,
    /// A read that is not the point's or the centroid's feature, row-major (`row · f + feature`).
    NotRowLayout,
    /// The loop over the centroids does not keep the nearest one (`If c is 0 or dist is at most bestDist:` setting
    /// `best` to `c` and `bestDist` to `dist`).
    NoArgmin,
    /// The point is not added to its nearest centroid's row of sums and counted once.
    NoAccumulation,
    /// No loop over the centroids setting each to its sum over its count, or to 0 when it has no points.
    NoUpdate,
    /// The sums and counts are not set to zero at the start of every epoch.
    NotZeroed,
    /// The centroids do not start as the first `k` points.
    NotFromFirstPoints,
    /// A counter not set to a constant right before its loop, or not stepped by exactly one.
    NotCounted,
    /// A loop body does more than the clustering.
    ExtraWork,
}

impl From<TermReject> for KmeansReject {
    fn from(r: TermReject) -> KmeansReject {
        match r {
            TermReject::Unsupported => KmeansReject::UnsupportedTerm,
            TermReject::NonAffine => KmeansReject::NonAffineIndex,
            TermReject::Layout => KmeansReject::NotRowLayout,
        }
    }
}

/// Recognize a k-means clustering anywhere in a LOGOS statement list — the epoch loop of Lloyd's algorithm:
///
/// ```text
/// While epoch is less than iters:
///     (zero `sums` (k · f) and `counts` (k), each with a loop setting every item to 0)
///     Let mutable p be 0.
///     While p is less than n:
///         Let mutable best be 0.
///         Let mutable bestDist be 0.
///         Let mutable c be 0.
///         While c is less than k:
///             Let mutable dist be 0.
///             Let mutable d be 0.
///             While d is less than f:
///                 Set dist to dist + (item (p * f + d + 1) of pts - item (c * f + d + 1) of cts) ** 2.
///                 Set d to d + 1.
///             If c is 0 or dist is at most bestDist:
///                 Set best to c.
///                 Set bestDist to dist.
///             Set c to c + 1.
///         (a loop over the features adding the point's row to row `best` of `sums`)
///         Set item (best + 1) of counts to item (best + 1) of counts + 1.
///         Set p to p + 1.
///     (a loop over the centroids, and in it over the features: `If cnt is 0:` the centroid's feature set to 0,
///      `Otherwise:` to its sum over `cnt`, `cnt` the centroid's count)
///     Set epoch to epoch + 1.
/// ```
///
/// where `cts` starts as the first `k · f` items of `pts` (a loop pushing `item (q + 1) of pts` onto it). Every loop
/// is counted and every body holds nothing else but immutable names and the counters' resets. `dist is less than
/// bestDist` keeps the first of equally near centroids, `at most` the last.
pub fn recognize_kmeans(stmts: &[Stmt], interner: &Interner) -> Result<KmeansSpec, KmeansReject> {
    let mut consts: HashMap<Symbol, i64> = HashMap::new();
    collect_consts_rec(stmts, &mut consts);
    let program = Program::of(stmts, interner);
    let spec = search_loops(stmts, KmeansReject::NoLoop, &|block, at, cond, body| {
        try_kmeans_at(block, at, cond, body, &consts, &program)
    })?;
    match copied_prefix(stmts, spec.centroids, &consts) {
        Some((src, len)) if src == spec.points && len == (spec.k * spec.f) as i64 && spec.n >= spec.k => Ok(spec),
        _ => Err(KmeansReject::NotFromFirstPoints),
    }
}

/// The `While` at `block[at]`: its condition and body.
fn while_at<'a>(block: &'a [Stmt<'a>], at: usize) -> Option<(&'a Expr<'a>, &'a [Stmt<'a>])> {
    match &block[at] {
        Stmt::While { cond, body, .. } => Some((*cond, *body)),
        _ => None,
    }
}

/// A counted loop at `block[at]` with its body: `Ok((counter, trips, body))`, `Err(NotCounted)` when its counter is not
/// counted, `None` when `block[at]` is not a `While` over a counter.
fn counted_while<'a>(
    block: &'a [Stmt<'a>],
    at: usize,
    consts: &HashMap<Symbol, i64>,
) -> Option<Result<(Symbol, i64, &'a [Stmt<'a>]), KmeansReject>> {
    let (cond, body) = while_at(block, at)?;
    let bound = loop_iv_bound(cond, consts)?;
    Some(counted_at(block, at, bound, body, consts).map(|c| (c.iv, c.trips, body)).ok_or(KmeansReject::NotCounted))
}

/// The array `dst` is filled from, and how many items, when its first fill loop pushes `item (q + 1) of SRC` for its
/// counter `q` from 0 — a copy of `SRC`'s first items.
fn copied_prefix(stmts: &[Stmt], dst: Symbol, consts: &HashMap<Symbol, i64>) -> Option<(Symbol, i64)> {
    let at = stmts.iter().position(|s| {
        matches!(s, Stmt::While { body, .. }
            if body.iter().any(|b| matches!(b, Stmt::Push { collection, .. } if as_ident(collection) == Some(dst))))
    })?;
    let (iv, trips, body) = counted_while(stmts, at, consts)?.ok()?;
    let value = body.iter().find_map(|b| match b {
        Stmt::Push { value, collection } if as_ident(collection) == Some(dst) => Some(*value),
        _ => None,
    })?;
    let src = indexed(value, &[(iv, 1)], consts, &HashMap::new())?;
    (entry_values(stmts, at, consts).get(&iv).copied().flatten() == Some(0)).then_some((src, trips))
}

/// `item IDX of A` with `IDX` affine exactly as `pairs` (each counter's coefficient) plus 1: `A`.
fn indexed(e: &Expr, pairs: &[(Symbol, i64)], consts: &HashMap<Symbol, i64>, lets: &HashMap<Symbol, Affine>) -> Option<Symbol> {
    let (coll, index) = as_index(e)?;
    let at = eval_affine(index, consts, lets)?;
    let shaped = at.cst == 1 && at.coeff.len() == pairs.len() && pairs.iter().all(|(s, k)| at.coeff_of(*s) == *k);
    shaped.then(|| as_ident(coll)).flatten()
}

/// `Set target to target + E` (either order): `(target, E)`.
fn accumulation<'a>(s: &'a Stmt<'a>) -> Option<(Symbol, &'a Expr<'a>)> {
    let Stmt::Set { target, value } = s else { return None };
    let (l, r) = as_binop(value, BinaryOpKind::Add)?;
    match (as_ident(l) == Some(*target), as_ident(r) == Some(*target)) {
        (true, _) => Some((*target, r)),
        (_, true) => Some((*target, l)),
        _ => None,
    }
}

/// `Set item IDX of A to item IDX of A + E` (either order) at `IDX` affine as `pairs` plus 1: `(A, E)`.
fn indexed_accumulation<'a>(
    s: &'a Stmt<'a>,
    pairs: &[(Symbol, i64)],
    consts: &HashMap<Symbol, i64>,
) -> Option<(Symbol, &'a Expr<'a>)> {
    let Stmt::SetIndex { collection, index, value } = s else { return None };
    let a = as_ident(collection)?;
    let at = eval_affine(index, consts, &HashMap::new())?;
    if !(at.cst == 1 && at.coeff.len() == pairs.len() && pairs.iter().all(|(v, k)| at.coeff_of(*v) == *k)) {
        return None;
    }
    let (l, r) = as_binop(value, BinaryOpKind::Add)?;
    let own = |x: &Expr| indexed(x, pairs, consts, &HashMap::new()) == Some(a);
    if own(l) {
        Some((a, r))
    } else if own(r) {
        Some((a, l))
    } else {
        None
    }
}

/// A loop setting every item of one array to 0, its counter from 0: `(array, items)`.
fn zeroing(block: &[Stmt], at: usize, consts: &HashMap<Symbol, i64>) -> Option<(Symbol, i64)> {
    let (iv, trips, body) = counted_while(block, at, consts)?.ok()?;
    let store = |s: &Stmt| match s {
        Stmt::SetIndex { collection, index, value: Expr::Literal(Literal::Number(0)) } => {
            let at = eval_affine(index, consts, &HashMap::new())?;
            (at.cst == 1 && at.coeff.len() == 1 && at.coeff_of(iv) == 1).then(|| as_ident(collection)).flatten()
        }
        _ => None,
    };
    let array = match body {
        [a, b] if is_unit_step(b, iv) => store(a)?,
        [a, b] if is_unit_step(a, iv) => store(b)?,
        _ => return None,
    };
    (entry_values(block, at, consts).get(&iv).copied().flatten() == Some(0)).then_some((array, trips))
}

/// Whether `block[k]` resets the counter of a later loop of `block` to a number, or names something immutably
/// without rebinding `kept`: what a body may hold beside the clustering.
fn is_scaffold(block: &[Stmt], k: usize, kept: &[Symbol]) -> bool {
    let resets = |iv: Symbol| {
        matches!(&block[k], Stmt::Let { var, mutable: true, value: Expr::Literal(Literal::Number(_)), .. }
            | Stmt::Set { target: var, value: Expr::Literal(Literal::Number(_)) } if *var == iv)
    };
    is_body_let(&block[k], kept)
        || block[k + 1..].iter().any(|later| match later {
            Stmt::While { cond, .. } => as_binop_any(cond).and_then(|(l, _)| as_ident(l)).map_or(false, resets),
            _ => false,
        })
}

/// `Let mutable v be N.` for a number `N` (or exactly 0, when `zero`).
fn starts_at(s: &Stmt, v: Symbol, zero: bool) -> bool {
    matches!(s, Stmt::Let { var, mutable: true, value: Expr::Literal(Literal::Number(n)), .. } if *var == v && (!zero || *n == 0))
}

/// The assignment step of an epoch: the loop over the points, in it the loop over the centroids keeping the nearest
/// by the distance its loop over the features sums, then the point added to that centroid's sums and counted.
struct Assignment {
    at: usize,
    n: i64,
    k: i64,
    f: i64,
    kept: Vec<Symbol>,
    points: Symbol,
    centroids: Symbol,
    sums: Symbol,
    counts: Symbol,
    distance: ZipTerm,
    ties: Ties,
}

impl Assignment {
    /// The assignment at `body[at]`; `None` when `body[at]` is not a loop with a loop around a loop inside it.
    fn at<'a>(
        body: &'a [Stmt<'a>],
        at: usize,
        consts: &HashMap<Symbol, i64>,
        program: &Program<'a>,
    ) -> Option<Result<Assignment, KmeansReject>> {
        let (cond_p, body_p) = while_at(body, at)?;
        let (p, _) = loop_iv_bound(cond_p, consts)?;
        let at_c = body_p.iter().position(|s| matches!(s, Stmt::While { body, .. } if body.iter().any(|t| matches!(t, Stmt::While { .. }))))?;
        let (cond_c, body_c) = while_at(body_p, at_c)?;
        let (c, _) = loop_iv_bound(cond_c, consts)?;
        let at_d = body_c.iter().position(|s| matches!(s, Stmt::While { .. }))?;
        let (cond_d, body_d) = while_at(body_c, at_d)?;
        let (d, _) = loop_iv_bound(cond_d, consts)?;
        Some(Self::read(body, at, (p, body_p), (c, at_c, body_c), (d, at_d, body_d), consts, program))
    }

    #[allow(clippy::too_many_arguments)]
    fn read<'a>(
        body: &'a [Stmt<'a>],
        at: usize,
        (p, body_p): (Symbol, &'a [Stmt<'a>]),
        (c, at_c, body_c): (Symbol, usize, &'a [Stmt<'a>]),
        (d, at_d, body_d): (Symbol, usize, &'a [Stmt<'a>]),
        consts: &HashMap<Symbol, i64>,
        program: &Program<'a>,
    ) -> Result<Assignment, KmeansReject> {
        // The distance: one register summing a term of the point's and the centroid's feature.
        let (pos_dist, (dist, term)) = body_d
            .iter()
            .enumerate()
            .find_map(|(k, s)| accumulation(s).filter(|(t, _)| *t != d).map(|a| (k, a)))
            .ok_or(KmeansReject::NoDistance)?;
        let (lets, exprs) = body_lets(body_p[..at_c].iter().chain(&body_c[..at_d]).chain(body_d), consts);
        let mut roles: [Option<Symbol>; 2] = [None, None];
        let mut width: Option<i64> = None;
        let mut read = |array: Symbol, index: &'a Expr<'a>| -> Result<usize, TermReject> {
            let at = eval_affine(index, consts, &lets).ok_or(TermReject::NonAffine)?;
            let role = [p, c].iter().position(|r| at.coeff_of(*r) != 0).ok_or(TermReject::Layout)?;
            let w = at.coeff_of([p, c][role]);
            if at.coeff.len() != 2 || at.coeff_of(d) != 1 || at.cst != 1 || width.map_or(false, |x| x != w) {
                return Err(TermReject::Layout);
            }
            width = Some(w);
            match roles[role] {
                Some(a) if a != array => Err(TermReject::Layout),
                _ => {
                    roles[role] = Some(array);
                    Ok(role)
                }
            }
        };
        let mut terms = Terms::new(program, consts, &exprs, vec![p, c, d, dist]);
        let distance = terms.term(term, &mut read)?;
        let ([Some(points), Some(centroids)], Some(f)) = (roles, width) else { return Err(KmeansReject::NotRowLayout) };
        let types = terms.types().ok_or(KmeansReject::NotRowLayout)?;
        match distance.ty(&types) {
            None | Some(Ty::Bool) => return Err(KmeansReject::UnsupportedTerm),
            Some(Ty::Int) if types == [Ty::Int, Ty::Int] => {}
            Some(_) => return Err(KmeansReject::NotInt),
        }
        if points == centroids {
            return Err(KmeansReject::NotRowLayout);
        }
        // The nearest centroid kept.
        let (pos_if, best, best_dist, ties) = body_c
            .iter()
            .enumerate()
            .find_map(|(k, s)| argmin_update(s, c, dist).map(|(b, bd, t)| (k, b, bd, t)))
            .ok_or(KmeansReject::NoArgmin)?;
        // The point's row added to row `best` of the sums, and the point counted.
        let row_sum = |k: usize| -> Option<(Symbol, i64)> {
            let (e, trips, body_e) = counted_while(body_p, k, consts)?.ok()?;
            let [a, b] = body_e else { return None };
            let add = if is_unit_step(b, e) { a } else if is_unit_step(a, e) { b } else { return None };
            let (sums, x) = indexed_accumulation(add, &[(best, f), (e, 1)], consts)?;
            (indexed(x, &[(p, f), (e, 1)], consts, &HashMap::new()) == Some(points)).then_some((sums, trips))
        };
        let (at_sum, (sums, sum_trips)) = (0..body_p.len())
            .filter(|k| *k != at_c)
            .find_map(|k| row_sum(k).map(|r| (k, r)))
            .ok_or(KmeansReject::NoAccumulation)?;
        let counted = |s: &Stmt| {
            indexed_accumulation(s, &[(best, 1)], consts)
                .filter(|(_, x)| matches!(x, Expr::Literal(Literal::Number(1))))
                .map(|(a, _)| a)
        };
        let (at_count, counts) = body_p
            .iter()
            .enumerate()
            .find_map(|(k, s)| counted(s).map(|a| (k, a)))
            .ok_or(KmeansReject::NoAccumulation)?;
        if sum_trips != f || sums == counts || [sums, counts].contains(&points) || [sums, counts].contains(&centroids) {
            return Err(KmeansReject::NoAccumulation);
        }
        // Every loop counted, and the features as wide as a row.
        let (p, n, _) = counted_while(body, at, consts).expect("a loop over a counter")?;
        let (c, k, _) = counted_while(body_p, at_c, consts).expect("a loop over a counter")?;
        let (d, f_trips, _) = counted_while(body_c, at_d, consts).expect("a loop over a counter")?;
        if f_trips != f {
            return Err(KmeansReject::NotRowLayout);
        }
        // Nothing else in the three bodies.
        let kept = vec![p, c, d, dist, best, best_dist, points, centroids, sums, counts];
        let point_ok = body_p.iter().enumerate().all(|(j, s)| {
            [at_c, at_sum, at_count].contains(&j)
                || is_unit_step(s, p)
                || starts_at(s, best, false)
                || starts_at(s, best_dist, false)
                || is_scaffold(body_p, j, &kept)
        }) && body_p.iter().any(|s| starts_at(s, best, false))
            && body_p.iter().any(|s| starts_at(s, best_dist, false));
        let centroid_ok = body_c.iter().enumerate().all(|(j, s)| {
            j == at_d || j == pos_if || is_unit_step(s, c) || starts_at(s, dist, true) || is_scaffold(body_c, j, &kept)
        }) && body_c[..at_d].iter().any(|s| starts_at(s, dist, true));
        let feature_ok =
            body_d.iter().enumerate().all(|(j, s)| j == pos_dist || is_unit_step(s, d) || is_body_let(s, &kept));
        if !(point_ok && centroid_ok && feature_ok) {
            return Err(KmeansReject::ExtraWork);
        }
        Ok(Assignment { at, n, k, f, kept, points, centroids, sums, counts, distance, ties })
    }
}

/// `If c is 0 or dist is at most bestDist: Set best to c. Set bestDist to dist.` (the disjuncts in either order, the
/// sets in either order; `less than` for the first of equally near centroids): `(best, bestDist, ties)`.
fn argmin_update(s: &Stmt, c: Symbol, dist: Symbol) -> Option<(Symbol, Symbol, Ties)> {
    let Stmt::If { cond, then_block: [a, b], else_block: None } = s else { return None };
    let (l, r) = as_binop(cond, BinaryOpKind::Or)?;
    let first = |e: &Expr| {
        as_binop(e, BinaryOpKind::Eq)
            .map_or(false, |(x, z)| as_ident(x) == Some(c) && matches!(z, Expr::Literal(Literal::Number(0))))
    };
    let nearer = |e: &Expr| -> Option<(Symbol, Ties)> {
        let Expr::BinaryOp { op, left, right } = e else { return None };
        let ties = match op {
            BinaryOpKind::LtEq => Ties::Last,
            BinaryOpKind::Lt => Ties::First,
            _ => return None,
        };
        (as_ident(left) == Some(dist)).then_some((as_ident(right)?, ties))
    };
    let (best_dist, ties) = if first(l) { nearer(r)? } else if first(r) { nearer(l)? } else { return None };
    let set = |s: &Stmt| match s {
        Stmt::Set { target, value } => Some((*target, as_ident(value)?)),
        _ => None,
    };
    let (sa, sb) = (set(a)?, set(b)?);
    let best = [sa, sb].iter().find(|(_, v)| *v == c).map(|(t, _)| *t)?;
    ([sa, sb].contains(&(best_dist, dist)) && best != best_dist && best_dist != dist).then_some((best, best_dist, ties))
}

/// The loop at `block[at]` over the centroids setting each to its mean — in a loop over the features, `If cnt is 0:`
/// the feature 0, `Otherwise:` `sums[j·f + e] / cnt`, `cnt` the centroid's count — as its trip count; `None` when it
/// is not a loop around a loop.
fn centroid_update(
    block: &[Stmt],
    at: usize,
    a: &Assignment,
    consts: &HashMap<Symbol, i64>,
) -> Option<Result<i64, KmeansReject>> {
    let (_, body_j) = while_at(block, at)?;
    let at_e = body_j.iter().position(|s| matches!(s, Stmt::While { .. }))?;
    Some((|| {
        let (j, trips, body_j) = counted_while(block, at, consts).expect("a loop")?;
        let (e, f_trips, body_e) = counted_while(body_j, at_e, consts).ok_or(KmeansReject::NoUpdate)??;
        let (lets, exprs) = body_lets(&body_j[..at_e], consts);
        let count = |x: &Expr| indexed(through_lets(x, &exprs), &[(j, 1)], consts, &lets) == Some(a.counts);
        let feature = |x: &Expr| indexed(x, &[(j, a.f), (e, 1)], consts, &lets);
        let set_feature = |s: &Stmt| -> Option<bool> {
            let Stmt::SetIndex { collection, index, value } = s else { return None };
            let at = eval_affine(index, consts, &lets)?;
            let ok = as_ident(collection) == Some(a.centroids)
                && at.cst == 1
                && at.coeff.len() == 2
                && at.coeff_of(j) == a.f
                && at.coeff_of(e) == 1;
            ok.then(|| match value {
                Expr::Literal(Literal::Number(0)) => true,
                v => as_binop(v, BinaryOpKind::Divide).map_or(false, |(n, d)| feature(n) == Some(a.sums) && count(d)),
            })
        };
        let rule = |s: &Stmt| {
            let Stmt::If { cond, then_block: [t], else_block: Some([o]) } = s else { return false };
            let empty =
                as_binop(cond, BinaryOpKind::Eq).map_or(false, |(x, z)| count(x) && matches!(z, Expr::Literal(Literal::Number(0))));
            let zero = matches!(t, Stmt::SetIndex { value: Expr::Literal(Literal::Number(0)), .. }) && set_feature(t) == Some(true);
            let mean = !matches!(o, Stmt::SetIndex { value: Expr::Literal(_), .. }) && set_feature(o) == Some(true);
            empty && zero && mean
        };
        let pos = body_e.iter().position(rule).ok_or(KmeansReject::NoUpdate)?;
        if f_trips != a.f {
            return Err(KmeansReject::NotRowLayout);
        }
        let mut kept = a.kept.clone();
        kept.extend([j, e]);
        let only = body_e.iter().enumerate().all(|(k, s)| k == pos || is_unit_step(s, e))
            && body_j.iter().enumerate().all(|(k, s)| k == at_e || is_unit_step(s, j) || is_scaffold(body_j, k, &kept));
        if only {
            Ok(trips)
        } else {
            Err(KmeansReject::ExtraWork)
        }
    })())
}

fn try_kmeans_at<'a>(
    stmts: &'a [Stmt<'a>],
    at: usize,
    cond: &'a Expr<'a>,
    body: &'a [Stmt<'a>],
    consts: &HashMap<Symbol, i64>,
    program: &Program<'a>,
) -> Result<KmeansSpec, KmeansReject> {
    let bound = loop_iv_bound(cond, consts).ok_or(KmeansReject::NoLoop)?;
    let a = (0..body.len())
        .find_map(|k| Assignment::at(body, k, consts, program))
        .ok_or(KmeansReject::NoAssignment)??;
    let (at_u, trips) = (0..body.len())
        .filter(|k| *k != a.at)
        .find_map(|k| centroid_update(body, k, &a, consts).map(|r| r.map(|t| (k, t))))
        .ok_or(KmeansReject::NoUpdate)??;
    let zeroed: Vec<(usize, (Symbol, i64))> = (0..a.at).filter_map(|k| zeroing(body, k, consts).map(|z| (k, z))).collect();
    let has = |arr: Symbol, len: i64| zeroed.iter().any(|(_, z)| *z == (arr, len));
    if trips != a.k || !(has(a.sums, a.k * a.f) && has(a.counts, a.k)) {
        return Err(KmeansReject::NotZeroed);
    }
    let epoch = counted_at(stmts, at, bound, body, consts).ok_or(KmeansReject::NotCounted)?;
    let mut kept = a.kept.clone();
    kept.push(epoch.iv);
    let epoch_ok = body.iter().enumerate().all(|(k, s)| {
        k == a.at
            || k == at_u
            || zeroed.iter().any(|(z, _)| *z == k)
            || is_unit_step(s, epoch.iv)
            || is_scaffold(body, k, &kept)
    });
    if !epoch_ok {
        return Err(KmeansReject::ExtraWork);
    }
    if [a.points, a.centroids, a.sums, a.counts].iter().any(|x| program.elem(*x) != Some(Ty::Int)) {
        return Err(KmeansReject::NotInt);
    }
    Ok(KmeansSpec {
        n: a.n as usize,
        k: a.k as usize,
        f: a.f as usize,
        iters: epoch.trips as usize,
        points: a.points,
        centroids: a.centroids,
        distance: a.distance,
        ties: a.ties,
    })
}
