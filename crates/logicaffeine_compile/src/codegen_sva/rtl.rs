//! RTL / transition-system bounded model checking — including multi-bit (bitvector) registers.
//!
//! A [`TransitionSystem`] is the classic `(registers, property)` model of synchronous
//! hardware: each [`Register`] has a width, an initial value, and a next-state expression
//! over the CURRENT state; a safety property must hold in every reachable state. 1-bit
//! registers are control/FSM bits; wider registers are datapath, handled through the
//! bit-blaster. This module time-indexes the model (`r` → `r@t`), lowers it through the
//! certified prover seam ([`super::sva_to_proof::bounded_to_proof`]), and drives
//! [`logicaffeine_proof::bmc`]. Pure Rust, certified, Z3-free.
//!
//! [`parse_transition_system`] ingests a synthesizable Verilog subset (multi-bit `reg`,
//! `initial`, one `always @(posedge clk)` block, `assert property`) with a typed (bool /
//! bitvector) expression parser. Unsupported constructs error out — never a silent
//! mis-parse.

use super::sva_to_proof::bounded_to_proof;
use super::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
use logicaffeine_proof::bmc::{self, BmcOutcome, InductionOutcome};
use logicaffeine_proof::sat::{find_model, prove_unsat, ModelOutcome, UnsatOutcome};
use logicaffeine_proof::ProofExpr;
use std::collections::HashMap;

/// A next-state function: a direct expression, or a guarded `if/else` (from RTL reset/mux
/// logic). Guards reference the current state and free inputs.
#[derive(Debug, Clone)]
pub enum NextState {
    Simple(BoundedExpr),
    Ite {
        cond: BoundedExpr,
        then_: Box<NextState>,
        else_: Box<NextState>,
    },
}

/// A hardware register: a width, an optional initial value, and its next-state function.
#[derive(Debug, Clone)]
pub struct Register {
    pub name: String,
    /// Bit width. `1` is a Boolean control bit; `>1` is a bitvector datapath register.
    pub width: u32,
    /// Initial (reset) value, masked to `width`; `None` means a FREE initial state (BMC
    /// explores all start values — the reset-as-free-input model).
    pub init: Option<u64>,
    /// Next-state function over the current (unindexed) state and free inputs.
    pub next: NextState,
}

/// A synchronous transition system over Boolean and bitvector registers.
#[derive(Debug, Clone)]
pub struct TransitionSystem {
    pub registers: Vec<Register>,
    /// The safety property that must hold in every reachable state (unindexed, boolean).
    pub property: BoundedExpr,
}

fn mask(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

impl TransitionSystem {
    /// Bounded model check: is a state violating `property` reachable within `max_k` steps?
    pub fn bmc(&self, max_k: u32) -> BmcOutcome {
        if self.lowered_init().is_none()
            || self.lowered_trans(0).is_none()
            || self.lowered_property(0).is_none()
        {
            return BmcOutcome::Unsupported;
        }
        let init = self.lowered_init().unwrap();
        bmc::find_counterexample(
            &init,
            &|t| self.lowered_trans(t).unwrap(),
            &|t| self.lowered_property(t).unwrap(),
            max_k,
        )
    }

    /// Prove (by k-induction) that `property` holds in EVERY reachable state.
    pub fn prove_invariant(&self, k: u32) -> InductionOutcome {
        if self.lowered_init().is_none()
            || self.lowered_trans(0).is_none()
            || self.lowered_property(0).is_none()
        {
            return InductionOutcome::Unsupported;
        }
        let init = self.lowered_init().unwrap();
        bmc::prove_invariant(
            &init,
            &|t| self.lowered_trans(t).unwrap(),
            &|t| self.lowered_property(t).unwrap(),
            k,
        )
    }

    /// A concrete witnessing execution: a model of `init ∧ trans(0..steps)` with NO property
    /// negation. For a deterministic controller this is the unique real run from the initial
    /// state — used to ANIMATE the proven-safe machine, which (by construction) has no
    /// counterexample to show. Returns the `signal@t` bit assignments of the run, or `None` if
    /// the obligation leaves the supported fragment.
    pub fn witness_trace(&self, steps: u32) -> Option<Vec<(String, bool)>> {
        let mut acc = self.lowered_init()?;
        for t in 0..steps {
            let tr = self.lowered_trans(t)?;
            acc = ProofExpr::And(Box::new(acc), Box::new(tr));
        }
        match find_model(&acc) {
            ModelOutcome::Sat(model) => Some(model),
            _ => None,
        }
    }

    fn lowered_init(&self) -> Option<ProofExpr> {
        // Only registers with a declared initial value are constrained; the rest are free.
        let parts: Vec<BoundedExpr> = self
            .registers
            .iter()
            .filter_map(|r| r.init.map(|v| reg_init_eq(&r.name, r.width, v)))
            .collect();
        bounded_to_proof(&conj(parts))
    }

    fn lowered_trans(&self, t: u32) -> Option<ProofExpr> {
        let parts = self.registers.iter().map(|r| reg_next_eq(r, t)).collect();
        bounded_to_proof(&conj(parts))
    }

    fn lowered_property(&self, t: u32) -> Option<ProofExpr> {
        bounded_to_proof(&time_index(&self.property, t))
    }
}

/// `reg@0 = init` as a bounded obligation (boolean equality for 1-bit, `bveq` for wider).
fn reg_init_eq(name: &str, width: u32, init: u64) -> BoundedExpr {
    let at0 = format!("{name}@0");
    if width == 1 {
        let v = BoundedExpr::Var(at0);
        if init & 1 == 1 {
            v
        } else {
            BoundedExpr::Not(Box::new(v))
        }
    } else {
        BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Eq,
            left: Box::new(BoundedExpr::BitVecVar(at0, width)),
            right: Box::new(BoundedExpr::BitVecConst {
                width,
                value: init & mask(width),
            }),
        }
    }
}

/// `reg@(t+1) = value` as a bounded obligation (boolean equality for 1-bit, `bveq` for wider).
fn reg_eq_value(name: &str, width: u32, t: u32, rhs: BoundedExpr) -> BoundedExpr {
    let at_next = format!("{name}@{}", t + 1);
    if width == 1 {
        BoundedExpr::Eq(Box::new(BoundedExpr::Var(at_next)), Box::new(rhs))
    } else {
        BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Eq,
            left: Box::new(BoundedExpr::BitVecVar(at_next, width)),
            right: Box::new(rhs),
        }
    }
}

/// `reg@(t+1) = next(state@t)` as a bounded obligation, expanding guarded updates into
/// `(cond → reg@(t+1)=then) ∧ (¬cond → reg@(t+1)=else)` (no bitvector-mux node needed).
fn reg_next_eq(r: &Register, t: u32) -> BoundedExpr {
    next_constraint(&r.name, r.width, t, &r.next)
}

fn next_constraint(name: &str, width: u32, t: u32, ns: &NextState) -> BoundedExpr {
    match ns {
        NextState::Simple(e) => reg_eq_value(name, width, t, time_index(e, t)),
        NextState::Ite { cond, then_, else_ } => {
            let c = time_index(cond, t);
            BoundedExpr::And(
                Box::new(BoundedExpr::Implies(
                    Box::new(c.clone()),
                    Box::new(next_constraint(name, width, t, then_)),
                )),
                Box::new(BoundedExpr::Implies(
                    Box::new(BoundedExpr::Not(Box::new(c))),
                    Box::new(next_constraint(name, width, t, else_)),
                )),
            )
        }
    }
}

fn conj(mut parts: Vec<BoundedExpr>) -> BoundedExpr {
    match parts.len() {
        0 => BoundedExpr::Bool(true),
        1 => parts.pop().unwrap(),
        _ => {
            let mut acc = parts.pop().unwrap();
            while let Some(p) = parts.pop() {
                acc = BoundedExpr::And(Box::new(p), Box::new(acc));
            }
            acc
        }
    }
}

/// Replace every (unindexed) register reference with its `name@t` form, recursively.
fn time_index(e: &BoundedExpr, t: u32) -> BoundedExpr {
    let ri = |x: &BoundedExpr| Box::new(time_index(x, t));
    match e {
        BoundedExpr::Var(n) => BoundedExpr::Var(format!("{n}@{t}")),
        BoundedExpr::BitVecVar(n, w) => BoundedExpr::BitVecVar(format!("{n}@{t}"), *w),
        BoundedExpr::Bool(_) | BoundedExpr::Int(_) | BoundedExpr::BitVecConst { .. } => e.clone(),
        BoundedExpr::Not(x) => BoundedExpr::Not(ri(x)),
        BoundedExpr::And(a, b) => BoundedExpr::And(ri(a), ri(b)),
        BoundedExpr::Or(a, b) => BoundedExpr::Or(ri(a), ri(b)),
        BoundedExpr::Implies(a, b) => BoundedExpr::Implies(ri(a), ri(b)),
        BoundedExpr::Eq(a, b) => BoundedExpr::Eq(ri(a), ri(b)),
        BoundedExpr::Lt(a, b) => BoundedExpr::Lt(ri(a), ri(b)),
        BoundedExpr::Gt(a, b) => BoundedExpr::Gt(ri(a), ri(b)),
        BoundedExpr::Lte(a, b) => BoundedExpr::Lte(ri(a), ri(b)),
        BoundedExpr::Gte(a, b) => BoundedExpr::Gte(ri(a), ri(b)),
        BoundedExpr::BitVecBinary { op, left, right } => BoundedExpr::BitVecBinary {
            op: op.clone(),
            left: ri(left),
            right: ri(right),
        },
        BoundedExpr::BitVecExtract { high, low, operand } => BoundedExpr::BitVecExtract {
            high: *high,
            low: *low,
            operand: ri(operand),
        },
        BoundedExpr::BitVecConcat(a, b) => BoundedExpr::BitVecConcat(ri(a), ri(b)),
        BoundedExpr::Comparison { op, left, right } => BoundedExpr::Comparison {
            op: op.clone(),
            left: ri(left),
            right: ri(right),
        },
        other => other.clone(),
    }
}

// ── Verilog → TransitionSystem ──────────────────────────────────────────────────────────

/// A Verilog parse error (kept distinct so the BMC layer can surface it verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtlParseError {
    pub message: String,
}

fn perr<T>(msg: impl Into<String>) -> Result<T, RtlParseError> {
    Err(RtlParseError {
        message: msg.into(),
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    /// A numeric literal: value plus optional explicit bit width (`4'd5` → (5, Some(4))).
    Num(u64, Option<u32>),
    Sym(String),
}

fn tokenize(src: &str) -> Result<Vec<Tok>, RtlParseError> {
    let b = src.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            toks.push(Tok::Ident(src[start..i].to_string()));
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < b.len() && (b[i] as char).is_ascii_digit() {
                i += 1;
            }
            let size_digits = &src[start..i];
            if i < b.len() && b[i] == b'\'' {
                // Sized literal `<size>'<base><value>`.
                i += 1;
                let base = if i < b.len() {
                    let ch = b[i] as char;
                    i += 1;
                    ch.to_ascii_lowercase()
                } else {
                    return perr("malformed sized literal");
                };
                let vstart = i;
                while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let vstr: String = src[vstart..i].chars().filter(|c| *c != '_').collect();
                let radix = match base {
                    'b' => 2,
                    'o' => 8,
                    'd' => 10,
                    'h' => 16,
                    _ => return perr(format!("unsupported literal base '{base}'")),
                };
                let value = u64::from_str_radix(&vstr, radix).map_err(|_| RtlParseError {
                    message: format!("bad literal value '{vstr}'"),
                })?;
                let width: u32 = size_digits.parse().map_err(|_| RtlParseError {
                    message: format!("bad literal size '{size_digits}'"),
                })?;
                toks.push(Tok::Num(value, Some(width)));
            } else {
                let value: u64 = size_digits.parse().map_err(|_| RtlParseError {
                    message: format!("bad number '{size_digits}'"),
                })?;
                toks.push(Tok::Num(value, None));
            }
            continue;
        }
        let two = if i + 1 < b.len() { &src[i..i + 2] } else { "" };
        if matches!(two, "==" | "!=" | "<=" | ">=" | "&&" | "||" | "<<" | ">>") {
            toks.push(Tok::Sym(two.to_string()));
            i += 2;
            continue;
        }
        let one = &src[i..i + 1];
        if "()[]{};:@~&|^=<>!,+-*".contains(one) {
            toks.push(Tok::Sym(one.to_string()));
            i += 1;
            continue;
        }
        return perr(format!("unexpected character '{c}'"));
    }
    Ok(toks)
}

/// A typed sub-expression: Boolean, a width-`w` bitvector, or an unsized literal whose width
/// is determined by context.
#[derive(Clone)]
enum Typed {
    Bool(BoundedExpr),
    Bv(BoundedExpr, u32),
    Num(u64),
}

impl Typed {
    /// Coerce to a `w`-bit bitvector (or a Boolean when `w == 1`).
    fn as_width(self, w: u32) -> Result<BoundedExpr, RtlParseError> {
        match self {
            Typed::Bv(e, ew) if ew == w => Ok(e),
            Typed::Bv(_, ew) => perr(format!("width mismatch: expected {w}, got {ew}")),
            Typed::Num(v) if w == 1 => Ok(BoundedExpr::Bool(v & 1 == 1)),
            Typed::Num(v) => Ok(BoundedExpr::BitVecConst {
                width: w,
                value: v & mask(w),
            }),
            Typed::Bool(e) if w == 1 => Ok(e),
            Typed::Bool(_) => perr(format!("expected a {w}-bit value, got a Boolean")),
        }
    }
    /// Coerce to a Boolean (a wider value is "non-zero").
    fn as_bool(self) -> Result<BoundedExpr, RtlParseError> {
        match self {
            Typed::Bool(e) => Ok(e),
            Typed::Num(v) => Ok(BoundedExpr::Bool(v != 0)),
            Typed::Bv(e, w) => Ok(BoundedExpr::Not(Box::new(BoundedExpr::BitVecBinary {
                op: BitVecBoundedOp::Eq,
                left: Box::new(e),
                right: Box::new(BoundedExpr::BitVecConst { width: w, value: 0 }),
            }))),
        }
    }
    fn width_hint(&self) -> Option<u32> {
        match self {
            Typed::Bv(_, w) => Some(*w),
            Typed::Bool(_) => Some(1),
            Typed::Num(_) => None,
        }
    }
}

/// Bring two operands to a common bitvector width (at least one must be sized).
fn bv_pair(a: Typed, b: Typed) -> Result<(BoundedExpr, BoundedExpr, u32), RtlParseError> {
    let w = a.width_hint().or_else(|| b.width_hint());
    let w = match w {
        Some(w) if w >= 1 => w,
        _ => return perr("ambiguous width: at least one operand must be sized"),
    };
    Ok((a.as_width(w)?, b.as_width(w)?, w))
}

fn bx(e: BoundedExpr) -> Box<BoundedExpr> {
    Box::new(e)
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
    widths: HashMap<String, u32>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Sym(x)) if x == s)
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(x)) if x == k)
    }
    fn eat_sym(&mut self, s: &str) -> Result<(), RtlParseError> {
        if self.is_sym(s) {
            self.i += 1;
            Ok(())
        } else {
            perr(format!("expected '{s}', found {:?}", self.peek()))
        }
    }
    fn eat_kw(&mut self, k: &str) -> Result<(), RtlParseError> {
        if self.is_kw(k) {
            self.i += 1;
            Ok(())
        } else {
            perr(format!("expected '{k}', found {:?}", self.peek()))
        }
    }
    fn ident(&mut self) -> Result<String, RtlParseError> {
        match self.peek() {
            Some(Tok::Ident(n)) => {
                let n = n.clone();
                self.i += 1;
                Ok(n)
            }
            other => perr(format!("expected identifier, found {other:?}")),
        }
    }
    fn num(&mut self) -> Result<u64, RtlParseError> {
        match self.peek() {
            Some(Tok::Num(v, _)) => {
                let v = *v;
                self.i += 1;
                Ok(v)
            }
            other => perr(format!("expected a number, found {other:?}")),
        }
    }

    // Typed expression grammar (Verilog precedence, low→high):
    // || , && , | , ^ , & , ==/!= , </<=/>/>= , +/- , * , unary ~/! , primary.
    fn expr(&mut self) -> Result<Typed, RtlParseError> {
        self.p_lor()
    }
    fn p_lor(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_land()?;
        while self.is_sym("||") {
            self.i += 1;
            let b = self.p_land()?;
            a = Typed::Bool(BoundedExpr::Or(bx(a.as_bool()?), bx(b.as_bool()?)));
        }
        Ok(a)
    }
    fn p_land(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_bor()?;
        while self.is_sym("&&") {
            self.i += 1;
            let b = self.p_bor()?;
            a = Typed::Bool(BoundedExpr::And(bx(a.as_bool()?), bx(b.as_bool()?)));
        }
        Ok(a)
    }
    fn p_bor(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_bxor()?;
        while self.is_sym("|") {
            self.i += 1;
            let b = self.p_bxor()?;
            a = self.bitwise(
                a,
                b,
                |x, y| BoundedExpr::Or(bx(x), bx(y)),
                BitVecBoundedOp::Or,
            )?;
        }
        Ok(a)
    }
    fn p_bxor(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_band()?;
        while self.is_sym("^") {
            self.i += 1;
            let b = self.p_band()?;
            a = self.bitwise(
                a,
                b,
                |x, y| BoundedExpr::Not(bx(BoundedExpr::Eq(bx(x), bx(y)))),
                BitVecBoundedOp::Xor,
            )?;
        }
        Ok(a)
    }
    fn p_band(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_eq()?;
        while self.is_sym("&") {
            self.i += 1;
            let b = self.p_eq()?;
            a = self.bitwise(
                a,
                b,
                |x, y| BoundedExpr::And(bx(x), bx(y)),
                BitVecBoundedOp::And,
            )?;
        }
        Ok(a)
    }
    fn p_eq(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_rel()?;
        while self.is_sym("==") || self.is_sym("!=") {
            let neq = self.is_sym("!=");
            self.i += 1;
            let b = self.p_rel()?;
            let eq = self.equality(a, b)?;
            a = Typed::Bool(if neq { BoundedExpr::Not(bx(eq)) } else { eq });
        }
        Ok(a)
    }
    fn p_rel(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_add()?;
        while self.is_sym("<") || self.is_sym(">") || self.is_sym("<=") || self.is_sym(">=") {
            let op = match self.peek() {
                Some(Tok::Sym(s)) => s.clone(),
                _ => unreachable!(),
            };
            self.i += 1;
            let b = self.p_add()?;
            let (l, r, _w) = bv_pair(a, b)?;
            // Unsigned magnitude comparisons via the bit-blaster's ULt.
            let ult = |x: BoundedExpr, y: BoundedExpr| BoundedExpr::BitVecBinary {
                op: BitVecBoundedOp::ULt,
                left: bx(x),
                right: bx(y),
            };
            let res = match op.as_str() {
                "<" => ult(l, r),
                ">" => ult(r, l),
                "<=" => BoundedExpr::Not(bx(ult(r, l))),
                ">=" => BoundedExpr::Not(bx(ult(l, r))),
                _ => unreachable!(),
            };
            a = Typed::Bool(res);
        }
        Ok(a)
    }
    fn p_add(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_mul()?;
        while self.is_sym("+") || self.is_sym("-") {
            let sub = self.is_sym("-");
            self.i += 1;
            let b = self.p_mul()?;
            let (l, r, w) = bv_pair(a, b)?;
            a = Typed::Bv(
                BoundedExpr::BitVecBinary {
                    op: if sub {
                        BitVecBoundedOp::Sub
                    } else {
                        BitVecBoundedOp::Add
                    },
                    left: bx(l),
                    right: bx(r),
                },
                w,
            );
        }
        Ok(a)
    }
    fn p_mul(&mut self) -> Result<Typed, RtlParseError> {
        let mut a = self.p_unary()?;
        while self.is_sym("*") {
            self.i += 1;
            let b = self.p_unary()?;
            let (l, r, w) = bv_pair(a, b)?;
            a = Typed::Bv(
                BoundedExpr::BitVecBinary {
                    op: BitVecBoundedOp::Mul,
                    left: bx(l),
                    right: bx(r),
                },
                w,
            );
        }
        Ok(a)
    }
    fn p_unary(&mut self) -> Result<Typed, RtlParseError> {
        if self.is_sym("!") {
            self.i += 1;
            return Ok(Typed::Bool(BoundedExpr::Not(bx(self
                .p_unary()?
                .as_bool()?))));
        }
        if self.is_sym("~") {
            self.i += 1;
            let inner = self.p_unary()?;
            return Ok(match inner {
                Typed::Bool(e) => Typed::Bool(BoundedExpr::Not(bx(e))),
                Typed::Bv(e, w) => Typed::Bv(
                    BoundedExpr::BitVecBinary {
                        op: BitVecBoundedOp::Not,
                        left: bx(e.clone()),
                        right: bx(e),
                    },
                    w,
                ),
                Typed::Num(_) => return perr("'~' needs a sized operand"),
            });
        }
        self.p_primary()
    }
    fn p_primary(&mut self) -> Result<Typed, RtlParseError> {
        if self.is_sym("(") {
            self.i += 1;
            let e = self.expr()?;
            self.eat_sym(")")?;
            return Ok(e);
        }
        match self.peek() {
            Some(Tok::Num(v, w)) => {
                let (v, w) = (*v, *w);
                self.i += 1;
                Ok(match w {
                    None => Typed::Num(v),
                    Some(1) => Typed::Bool(BoundedExpr::Bool(v & 1 == 1)),
                    Some(w) => Typed::Bv(
                        BoundedExpr::BitVecConst {
                            width: w,
                            value: v & mask(w),
                        },
                        w,
                    ),
                })
            }
            Some(Tok::Ident(n)) => {
                let n = n.clone();
                self.i += 1;
                match self.widths.get(&n).copied() {
                    Some(1) | None => Ok(Typed::Bool(BoundedExpr::Var(n))),
                    Some(w) => Ok(Typed::Bv(BoundedExpr::BitVecVar(n, w), w)),
                }
            }
            other => perr(format!("expected an expression, found {other:?}")),
        }
    }

    /// Bitwise `&`/`|`/`^`: Boolean on Boolean operands, bit-blasted on bitvectors.
    fn bitwise(
        &self,
        a: Typed,
        b: Typed,
        bool_op: fn(BoundedExpr, BoundedExpr) -> BoundedExpr,
        bv_op: BitVecBoundedOp,
    ) -> Result<Typed, RtlParseError> {
        match (&a, &b) {
            (Typed::Bv(..), _) | (_, Typed::Bv(..)) => {
                let (l, r, w) = bv_pair(a, b)?;
                Ok(Typed::Bv(
                    BoundedExpr::BitVecBinary {
                        op: bv_op,
                        left: bx(l),
                        right: bx(r),
                    },
                    w,
                ))
            }
            _ => Ok(Typed::Bool(bool_op(a.as_bool()?, b.as_bool()?))),
        }
    }

    /// `==` (the `!=` caller negates): biconditional on Booleans, `bveq` on bitvectors.
    fn equality(&self, a: Typed, b: Typed) -> Result<BoundedExpr, RtlParseError> {
        match (&a, &b) {
            (Typed::Bv(..), _) | (_, Typed::Bv(..)) => {
                let (l, r, _w) = bv_pair(a, b)?;
                Ok(BoundedExpr::BitVecBinary {
                    op: BitVecBoundedOp::Eq,
                    left: bx(l),
                    right: bx(r),
                })
            }
            _ => Ok(BoundedExpr::Eq(bx(a.as_bool()?), bx(b.as_bool()?))),
        }
    }

    fn skip_to_semi(&mut self) -> Result<(), RtlParseError> {
        while !self.is_sym(";") {
            if self.peek().is_none() {
                return perr("expected ';' before end of input");
            }
            self.i += 1;
        }
        self.i += 1;
        Ok(())
    }
    fn skip_balanced_parens(&mut self) -> Result<(), RtlParseError> {
        self.eat_sym("(")?;
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                None => return perr("unbalanced '(' in port list"),
                Some(Tok::Sym(s)) if s == "(" => depth += 1,
                Some(Tok::Sym(s)) if s == ")" => depth -= 1,
                _ => {}
            }
            self.i += 1;
        }
        Ok(())
    }

    /// Parse `[hi:lo]` and return the width `hi - lo + 1`.
    fn parse_range_width(&mut self) -> Result<u32, RtlParseError> {
        self.eat_sym("[")?;
        let hi = self.num()? as i64;
        self.eat_sym(":")?;
        let lo = self.num()? as i64;
        self.eat_sym("]")?;
        if hi < lo {
            return perr("register range must be [hi:lo] with hi >= lo");
        }
        Ok((hi - lo + 1) as u32)
    }

    /// Parse an `always` body into per-register next-state functions, honoring `if/else`
    /// (a register unassigned in a branch holds its prior value).
    fn parse_block(&mut self) -> Result<HashMap<String, NextState>, RtlParseError> {
        let mut map = HashMap::new();
        if self.is_kw("begin") {
            self.i += 1;
            while !self.is_kw("end") {
                if self.peek().is_none() {
                    return perr("missing 'end' in always block");
                }
                self.parse_stmt(&mut map)?;
            }
            self.i += 1;
        } else {
            self.parse_stmt(&mut map)?;
        }
        Ok(map)
    }

    fn parse_stmt(&mut self, map: &mut HashMap<String, NextState>) -> Result<(), RtlParseError> {
        if self.is_kw("if") {
            self.i += 1;
            self.eat_sym("(")?;
            let cond = self.expr()?.as_bool()?;
            self.eat_sym(")")?;
            let then_map = self.parse_block()?;
            let else_map = if self.is_kw("else") {
                self.i += 1;
                self.parse_block()?
            } else {
                HashMap::new()
            };
            let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            names.extend(then_map.keys().cloned());
            names.extend(else_map.keys().cloned());
            for n in names {
                let w = self.widths.get(&n).copied().unwrap_or(1);
                let hold = if w == 1 {
                    NextState::Simple(BoundedExpr::Var(n.clone()))
                } else {
                    NextState::Simple(BoundedExpr::BitVecVar(n.clone(), w))
                };
                let prior = map.get(&n).cloned().unwrap_or(hold);
                let then_ns = then_map.get(&n).cloned().unwrap_or_else(|| prior.clone());
                let else_ns = else_map.get(&n).cloned().unwrap_or(prior);
                map.insert(
                    n,
                    NextState::Ite {
                        cond: cond.clone(),
                        then_: Box::new(then_ns),
                        else_: Box::new(else_ns),
                    },
                );
            }
            Ok(())
        } else {
            let n = self.ident()?;
            let w = self.widths.get(&n).copied().unwrap_or(1);
            if self.is_sym("<=") {
                self.i += 1;
            } else {
                self.eat_sym("=")?;
            }
            let e = self.expr()?.as_width(w)?;
            self.eat_sym(";")?;
            map.insert(n, NextState::Simple(e));
            Ok(())
        }
    }
}

/// Parse a synthesizable Verilog module into a [`TransitionSystem`]. Errors (never silently
/// mis-parses) on anything outside the supported subset.
pub fn parse_transition_system(src: &str) -> Result<TransitionSystem, RtlParseError> {
    let toks = tokenize(src)?;
    // First pass: collect register widths so the expression parser is correctly typed.
    // First pass: collect the width of every declared signal (reg/input/output/wire,
    // wherever declared — including the port list) so the expression parser is correctly
    // typed. Inputs are free variables; only `reg`s become registers (second pass).
    let is_decl_kw = |s: &str| matches!(s, "reg" | "input" | "output" | "wire");
    let mut widths: HashMap<String, u32> = HashMap::new();
    {
        let mut j = 0;
        while j < toks.len() {
            if matches!(&toks[j], Tok::Ident(k) if is_decl_kw(k)) {
                j += 1;
                let mut w = 1u32;
                if matches!(toks.get(j), Some(Tok::Sym(s)) if s == "[") {
                    let mut tmp = Parser {
                        t: &toks,
                        i: j,
                        widths: HashMap::new(),
                    };
                    w = tmp.parse_range_width()?;
                    j = tmp.i;
                }
                // Comma-separated names — but stop at the next declaration keyword (port
                // lists read `input clk, input rst`, not `input clk, rst`).
                loop {
                    match toks.get(j) {
                        Some(Tok::Ident(n)) if !is_decl_kw(n) => {
                            widths.insert(n.clone(), w);
                            j += 1;
                        }
                        _ => break,
                    }
                    if matches!(toks.get(j), Some(Tok::Sym(s)) if s == ",") {
                        j += 1;
                        continue;
                    }
                    break;
                }
            } else {
                j += 1;
            }
        }
    }

    let mut p = Parser {
        t: &toks,
        i: 0,
        widths,
    };
    p.eat_kw("module")?;
    let _name = p.ident()?;
    if p.is_sym("(") {
        p.skip_balanced_parens()?;
    }
    p.eat_sym(";")?;

    let mut order: Vec<String> = Vec::new();
    let mut init: HashMap<String, u64> = HashMap::new();
    let mut next: HashMap<String, NextState> = HashMap::new();
    let mut property: Option<BoundedExpr> = None;

    loop {
        match p.peek() {
            None => return perr("unexpected end of input (missing 'endmodule')"),
            Some(Tok::Ident(k)) if k == "endmodule" => {
                p.i += 1;
                break;
            }
            Some(Tok::Ident(k)) if k == "reg" => {
                p.i += 1;
                if p.is_sym("[") {
                    let _ = p.parse_range_width()?;
                }
                loop {
                    let n = p.ident()?;
                    if !order.contains(&n) {
                        order.push(n);
                    }
                    if p.is_sym(",") {
                        p.i += 1;
                        continue;
                    }
                    break;
                }
                p.eat_sym(";")?;
            }
            Some(Tok::Ident(k)) if k == "input" || k == "output" || k == "wire" => {
                p.skip_to_semi()?;
            }
            Some(Tok::Ident(k)) if k == "initial" => {
                p.i += 1;
                let mut one = |p: &mut Parser| -> Result<(), RtlParseError> {
                    let n = p.ident()?;
                    p.eat_sym("=")?;
                    let v = p.num()?;
                    p.eat_sym(";")?;
                    init.insert(n, v);
                    Ok(())
                };
                if p.is_kw("begin") {
                    p.i += 1;
                    while !p.is_kw("end") {
                        if p.peek().is_none() {
                            return perr("missing 'end' for initial block");
                        }
                        one(&mut p)?;
                    }
                    p.i += 1;
                } else {
                    one(&mut p)?;
                }
            }
            Some(Tok::Ident(k)) if k == "always" => {
                p.i += 1;
                p.eat_sym("@")?;
                p.eat_sym("(")?;
                p.eat_kw("posedge")?;
                let _clk = p.ident()?;
                p.eat_sym(")")?;
                let block = p.parse_block()?;
                for (n, ns) in block {
                    next.insert(n, ns);
                }
            }
            Some(Tok::Ident(k)) if k == "assert" => {
                p.i += 1;
                p.eat_kw("property")?;
                p.eat_sym("(")?;
                if p.is_sym("@") {
                    p.i += 1;
                    p.eat_sym("(")?;
                    p.eat_kw("posedge")?;
                    let _ = p.ident()?;
                    p.eat_sym(")")?;
                }
                let e = p.expr()?.as_bool()?;
                p.eat_sym(")")?;
                p.eat_sym(";")?;
                property = Some(e);
            }
            Some(_) => p.skip_to_semi()?,
        }
    }

    if order.is_empty() {
        return perr("no registers found");
    }
    let property = match property {
        Some(p) => p,
        None => return perr("no 'assert property' found"),
    };

    let mut registers = Vec::with_capacity(order.len());
    for name in &order {
        let width = p.widths.get(name).copied().unwrap_or(1);
        // `initial` is optional: absent ⇒ a free initial state (reset-as-free-input).
        let init_val = init.get(name).map(|v| *v & mask(width));
        let next_ns = next.remove(name).unwrap_or_else(|| {
            // A register with no update holds its value.
            if width == 1 {
                NextState::Simple(BoundedExpr::Var(name.clone()))
            } else {
                NextState::Simple(BoundedExpr::BitVecVar(name.clone(), width))
            }
        });
        registers.push(Register {
            name: name.clone(),
            width,
            init: init_val,
            next: next_ns,
        });
    }

    Ok(TransitionSystem {
        registers,
        property,
    })
}

/// Emit a synthesizable Verilog SOFT-CORE for the reversible ALU datapath (corner 5, phase 3) — the
/// register-VM's `RevAdd` charge-recovery core as RTL: a `width`-bit register advanced by `step` each
/// clock. `q := (q + step) mod 2^width` is a PERMUTATION of the state space (a cyclic reversible
/// datapath whose reverse module is the `−step` decrement), and the emitted text is real synthesizable
/// Verilog — [`parse_transition_system`] accepts it and BMC / k-induction discharge its obligations.
/// This is the round-tripping HDL twin of the software `RevAdd`; the full VM-datapath soft-core + a
/// VM↔RTL SMT equivalence are the larger MAKE_A_CHIP build this slice opens.
pub fn emit_reversible_alu_verilog(width: u32, step: u64) -> String {
    let hi = width - 1;
    let max = if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    format!(
        "module rev_alu(input clk);\n  reg [{hi}:0] q;\n  initial q = 0;\n  always @(posedge clk) q <= q + {width}'d{step};\n  assert property (q <= {width}'d{max});\nendmodule"
    )
}

/// Emit the reversible `RevAdd` DATAPATH as synthesizable Verilog — a `width`-bit accumulator advanced
/// by a data INPUT each clock (`q := (q + delta) mod 2^width`), the genuine charge-recovery ALU slice
/// (its reverse is `q := q − delta`). For every fixed `delta` the clocked transition is a bijection in
/// `q`. Unlike the constant-step [`emit_reversible_alu_verilog`], this is the real datapath the VM's
/// `RevAdd` maps to; `benchmarks/silicon/` synthesizes + place-and-routes the 32-bit form on an ECP5 FPGA.
pub fn emit_reversible_datapath_verilog(width: u32) -> String {
    let hi = width - 1;
    format!(
        "module rev_alu(input clk, input [{hi}:0] delta, output reg [{hi}:0] q);\n  initial q = 0;\n  always @(posedge clk) q <= q + delta;\n  assert property (q == q);\nendmodule"
    )
}

/// Emit the LOGOS reversible ALU CORE as synthesizable Verilog — the abelian charge-recovery datapath
/// of the VM: two `width`-bit registers `a`,`b` and a 3-bit opcode selecting `RevAdd`/`RevSub`/`RevXor`/
/// `RevNeg`/`RevSwap`. For every fixed opcode the transition on `(a,b)` is a BIJECTION, so the whole ALU
/// runs backward with zero erased bits. The 32-bit form place-and-routes on a Lattice ECP5 (ULX3S) via
/// `scripts/synth-soft-core.sh` (`benchmarks/silicon/rev_alu_core.v`): 336 LUTs + 64 FFs at ~136 MHz.
pub fn emit_reversible_alu_core_verilog(width: u32) -> String {
    let hi = width - 1;
    format!(
        "module rev_alu_core(input clk, input [2:0] op, output reg [{hi}:0] a, output reg [{hi}:0] b);\n\
         \x20 initial begin a = 0; b = 0; end\n\
         \x20 always @(posedge clk) begin\n\
         \x20   case (op)\n\
         \x20     3'd0: a <= a + b;\n\
         \x20     3'd1: a <= a - b;\n\
         \x20     3'd2: a <= a ^ b;\n\
         \x20     3'd3: a <= (~a) + {width}'d1;\n\
         \x20     3'd4: begin a <= b; b <= a; end\n\
         \x20     default: ;\n\
         \x20   endcase\n\
         \x20 end\n\
         endmodule"
    )
}

/// Emit the ISA **opcode decoder** for the reversible core — the silicon FRONT-END that turns a wire
/// opcode BYTE (the exact byte [`crate::vm::binisa::encode_program`] puts on the wire) into the
/// reversible ALU's 3-bit control select plus a `core` valid flag. The `case` labels are DERIVED from
/// [`crate::vm::binisa::opcode`], so the decoder and the wire format share ONE source of truth: the
/// byte the network carries is the byte the chip decodes, with no translation layer. Compose with
/// [`emit_reversible_alu_core_verilog`] — wire `isa_core_decoder.sel → rev_alu_core.op`.
pub fn emit_isa_core_decoder_verilog() -> String {
    use crate::vm::binisa::opcode;
    use crate::vm::Op;
    let core = [
        (Op::RevAdd { place: 0, delta: 0 }, "RevAdd"),
        (Op::RevSub { place: 0, delta: 0 }, "RevSub"),
        (Op::RevXor { place: 0, delta: 0 }, "RevXor"),
        (Op::RevNeg { place: 0 }, "RevNeg"),
        (Op::RevSwap { a: 0, b: 0 }, "RevSwap"),
    ];
    let mut arms = String::new();
    for (op, name) in &core {
        let code = opcode(op);
        arms.push_str(&format!(
            "\x20     8'd{code}: begin sel = 3'd{code}; core = 1'b1; end  // {name}\n"
        ));
    }
    // An UNRECOGNISED wire byte decodes to an ALU-identity select (`3'd7` hits the ALU's `default: ;`)
    // and asserts `core = 0`: an illegal opcode is an inert no-op on the reversible state, never a
    // silent RevAdd. Correct ISA hygiene — a corrupt byte cannot perturb the datapath.
    format!(
        "module isa_core_decoder(input [7:0] opcode, output reg [2:0] sel, output reg core);\n\
         \x20 always @(*) begin\n\
         \x20   case (opcode)\n\
         {arms}\
         \x20     default: begin sel = 3'd7; core = 1'b0; end\n\
         \x20   endcase\n\
         \x20 end\n\
         endmodule"
    )
}

/// Emit the GENERAL INTEGER ALU datapath — the combinational functional unit that executes the
/// arithmetic / bitwise / signed-comparison ops straight off a WIRE OPCODE BYTE. The `case` labels are
/// the stable `binisa` opcodes (one source of truth with the wire format), so the ALU decodes the same
/// bytes the network carries. Each operation is proven correct by our own certified SAT (the identity
/// battery `the_general_alu_datapath_is_proven_correct_by_our_sat`); comparisons are SIGNED (matching
/// the VM's i64 semantics) and yield 0/1. This is the ratchet frontier grown from the reversible core.
pub fn emit_alu_datapath_verilog(width: u32) -> String {
    use crate::vm::binisa::opcode;
    use crate::vm::Op;
    let hi = width - 1;
    let add = opcode(&Op::Add {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let addassign = opcode(&Op::AddAssign { dst: 0, src: 0 });
    let sub = opcode(&Op::Sub {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let mul = opcode(&Op::Mul {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    // Div / ExactDiv / Mod / FloorDiv are NOT in the combinational ALU: division is inherently
    // sequential, so the soft-core issues those opcodes to the shared iterative `shared_divider`
    // (a ~26x LUT saving). DivPow2 stays here — it is a cheap round-toward-zero arithmetic shift.
    let divpow2 = opcode(&Op::DivPow2 {
        dst: 0,
        lhs: 0,
        k: 0,
    });
    let xor = opcode(&Op::BitXor {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let and = opcode(&Op::BitAnd {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let or = opcode(&Op::BitOr {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let not = opcode(&Op::Not { dst: 0, src: 0 });
    let shl = opcode(&Op::Shl {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let shr = opcode(&Op::Shr {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    // The shift COUNT is masked to the value width (`b[log2(W)-1:0]`), matching the VM's
    // `wrapping_shl/shr(b as u32)` which masks the count mod the word width. `Shr` is ARITHMETIC.
    let cnt_hi = width.trailing_zeros().saturating_sub(1);
    let lt = opcode(&Op::Lt {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let gt = opcode(&Op::Gt {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let lte = opcode(&Op::LtEq {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let gte = opcode(&Op::GtEq {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let eq = opcode(&Op::Eq {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    let neq = opcode(&Op::NotEq {
        dst: 0,
        lhs: 0,
        rhs: 0,
    });
    format!(
        "module alu(input [7:0] opcode, input [{hi}:0] a, input [{hi}:0] b, output reg [{hi}:0] y);\n\
         \x20 always @(*) begin\n\
         \x20   case (opcode)\n\
         \x20     8'd{add}: y = a + b;\n\
         \x20     8'd{addassign}: y = a + b;\n\
         \x20     8'd{sub}: y = a - b;\n\
         \x20     8'd{mul}: y = a * b;\n\
         \x20     8'd{divpow2}: y = a[{hi}] ? ($signed($signed(a) + (({width}'d1 << b[{cnt_hi}:0]) - {width}'d1)) >>> b[{cnt_hi}:0]) : ($signed(a) >>> b[{cnt_hi}:0]);\n\
         \x20     8'd{xor}: y = a ^ b;\n\
         \x20     8'd{and}: y = a & b;\n\
         \x20     8'd{or}: y = a | b;\n\
         \x20     8'd{not}: y = ~a;\n\
         \x20     8'd{shl}: y = a << b[{cnt_hi}:0];\n\
         \x20     8'd{shr}: y = $signed(a) >>> b[{cnt_hi}:0];\n\
         \x20     8'd{lt}: y = ($signed(a) < $signed(b)) ? 1 : 0;\n\
         \x20     8'd{gt}: y = ($signed(a) > $signed(b)) ? 1 : 0;\n\
         \x20     8'd{lte}: y = ($signed(a) <= $signed(b)) ? 1 : 0;\n\
         \x20     8'd{gte}: y = ($signed(a) >= $signed(b)) ? 1 : 0;\n\
         \x20     8'd{eq}: y = (a == b) ? 1 : 0;\n\
         \x20     8'd{neq}: y = (a != b) ? 1 : 0;\n\
         \x20     8'd226: y = a + b;\n\
         \x20     8'd227: y = a - b;\n\
         \x20     8'd228: y = a * b;\n\
         \x20     default: y = {width}'d0;\n\
         \x20   endcase\n\
         \x20 end\n\
         endmodule"
    )
}

/// Emit the SHARED ITERATIVE DIVIDER — one sequential restoring divider that computes signed
/// quotient AND remainder, replacing the ~24,600-cell COMBINATIONAL `$signed(a)/$signed(b)` +
/// `$signed(a)%$signed(b)` the ALU instantiates per-op (Div/Mod/FloorDiv). It costs 833 LUT / 204 FF
/// placed-and-routed on the ECP5-85K at ~100 MHz — a ~26× LUT drop vs the ~22,000-LUT combinational
/// divide, the lever that fits the full 113-op CPU under 83,640. Truncated (round-toward-zero)
/// semantics matching the VM: quotient sign = sign(a)^sign(b), remainder sign = sign(a); b==0 →
/// q=r=0. One 1-bit/cycle restoring loop over |a|/|b| (W cycles) then a sign fixup; the soft-core
/// pulses `start` and stalls until `done`. The module is Verilog-parameterized (default W=32;
/// instantiate #(.W(64)) for the BigInt lane), so this emitter is the single source for the on-disk
/// `shared_divider.v`, proven == the combinational oracle by iverilog ([`emit_shared_divider_testbench`]).
pub fn emit_shared_divider_verilog() -> String {
    include_str!("shared_divider.v").to_string()
}

/// Emit the divider's self-checking testbench: drive the sequential `shared_divider`, wait for
/// `done`, and check quotient+remainder against the combinational `$signed` div/mod oracle over
/// every sign/edge case plus a 4000-pair pseudo-random sweep. PASS iff all match.
pub fn emit_shared_divider_testbench() -> String {
    include_str!("tb_shared_divider.v").to_string()
}

/// Emit a yosys synthesis FLOW (`.ys`) that takes the reversible soft-core RTL to a gate-level netlist
/// (corner 5, phase 5). This is the tape-out-ready boundary produced IN-REPO: the synthesizable,
/// formally-verified reversible RTL ([`emit_reversible_alu_verilog`]) PLUS this flow. Executing it needs
/// the external `yosys` synthesizer (not installed here); mapping the generic gate set to an
/// adiabatic / charge-recovery PDK and running a foundry are the physical tape-out steps beyond
/// software. The flow reads the module, elaborates, techmaps, maps to a generic gate library, and
/// writes the gate-level netlist + a resource report.
pub fn emit_yosys_tapeout_flow(module: &str, rtl_file: &str) -> String {
    format!(
        "# LOGOS reversible soft-core — tape-out synthesis flow.  Run with:  yosys {module}.ys\n\
         read_verilog {rtl_file}\n\
         hierarchy -check -top {module}\n\
         proc; opt; fsm; opt; memory; opt\n\
         techmap; opt\n\
         # Generic gate mapping; a reversible/adiabatic PDK (charge-recovery cells) replaces this for silicon.\n\
         abc -g AND,OR,XOR,NAND,NOR\n\
         opt; clean\n\
         write_verilog {module}_netlist.v\n\
         stat\n"
    )
}

/// LOGICAL EQUIVALENCE CHECK (corner 5, phase 3) — discharged by OUR OWN certified SAT, not yosys's
/// SAT or Z3. Parse the soft-core RTL, extract register `reg`'s next-state, and prove it equals `spec`
/// (the VM op's datapath function) for EVERY input by refuting the miter `rtl_next ≠ spec` with
/// [`logicaffeine_proof::sat::prove_unsat`] — a bit-blasted CDCL refutation, the LEC "back-check". This
/// is the equivalence a tool like yosys `equiv` runs on its internal SAT, done here on ours. Returns
/// `Some(true)` iff the RTL datapath is equivalent to the VM spec, `Some(false)` if a differing input
/// exists, `None` if the RTL / next-state / miter can't be lowered to the bit-blaster.
pub fn lec_next_state_matches_vm(rtl: &str, reg: &str, spec: BoundedExpr) -> Option<bool> {
    let ts = parse_transition_system(rtl).ok()?;
    let r = ts.registers.iter().find(|r| r.name == reg)?;
    let rtl_next = match &r.next {
        NextState::Simple(e) => e.clone(),
        NextState::Ite { .. } => return None,
    };
    // The miter: are the RTL datapath and the VM spec EVER unequal? Equivalent iff that is UNSAT.
    let miter = BoundedExpr::Eq(Box::new(rtl_next), Box::new(spec));
    let p = super::bitblast::lower_bool(&miter)?;
    Some(prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/rtl/tests.rs"]
mod tests;
