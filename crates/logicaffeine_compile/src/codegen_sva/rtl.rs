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
use logicaffeine_proof::sat::{find_model, ModelOutcome};
use logicaffeine_proof::ProofExpr;
use std::collections::HashMap;

/// A next-state function: a direct expression, or a guarded `if/else` (from RTL reset/mux
/// logic). Guards reference the current state and free inputs.
#[derive(Debug, Clone)]
pub enum NextState {
    Simple(BoundedExpr),
    Ite { cond: BoundedExpr, then_: Box<NextState>, else_: Box<NextState> },
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
            right: Box::new(BoundedExpr::BitVecConst { width, value: init & mask(width) }),
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
    Err(RtlParseError { message: msg.into() })
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
                let value = u64::from_str_radix(&vstr, radix)
                    .map_err(|_| RtlParseError { message: format!("bad literal value '{vstr}'") })?;
                let width: u32 = size_digits
                    .parse()
                    .map_err(|_| RtlParseError { message: format!("bad literal size '{size_digits}'") })?;
                toks.push(Tok::Num(value, Some(width)));
            } else {
                let value: u64 = size_digits
                    .parse()
                    .map_err(|_| RtlParseError { message: format!("bad number '{size_digits}'") })?;
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
            Typed::Num(v) => Ok(BoundedExpr::BitVecConst { width: w, value: v & mask(w) }),
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
            a = self.bitwise(a, b, |x, y| BoundedExpr::Or(bx(x), bx(y)), BitVecBoundedOp::Or)?;
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
            a = self.bitwise(a, b, |x, y| BoundedExpr::And(bx(x), bx(y)), BitVecBoundedOp::And)?;
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
                    op: if sub { BitVecBoundedOp::Sub } else { BitVecBoundedOp::Add },
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
                BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Mul, left: bx(l), right: bx(r) },
                w,
            );
        }
        Ok(a)
    }
    fn p_unary(&mut self) -> Result<Typed, RtlParseError> {
        if self.is_sym("!") {
            self.i += 1;
            return Ok(Typed::Bool(BoundedExpr::Not(bx(self.p_unary()?.as_bool()?))));
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
                    Some(w) => Typed::Bv(BoundedExpr::BitVecConst { width: w, value: v & mask(w) }, w),
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
                Ok(Typed::Bv(BoundedExpr::BitVecBinary { op: bv_op, left: bx(l), right: bx(r) }, w))
            }
            _ => Ok(Typed::Bool(bool_op(a.as_bool()?, b.as_bool()?))),
        }
    }

    /// `==` (the `!=` caller negates): biconditional on Booleans, `bveq` on bitvectors.
    fn equality(&self, a: Typed, b: Typed) -> Result<BoundedExpr, RtlParseError> {
        match (&a, &b) {
            (Typed::Bv(..), _) | (_, Typed::Bv(..)) => {
                let (l, r, _w) = bv_pair(a, b)?;
                Ok(BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Eq, left: bx(l), right: bx(r) })
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
                    let mut tmp = Parser { t: &toks, i: j, widths: HashMap::new() };
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

    let mut p = Parser { t: &toks, i: 0, widths };
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
        registers.push(Register { name: name.clone(), width, init: init_val, next: next_ns });
    }

    Ok(TransitionSystem { registers, property })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(n: &str) -> BoundedExpr {
        BoundedExpr::Var(n.to_string())
    }
    fn not(e: BoundedExpr) -> BoundedExpr {
        BoundedExpr::Not(Box::new(e))
    }
    fn reg(name: &str, init: u64, next: BoundedExpr) -> Register {
        Register {
            name: name.to_string(),
            width: 1,
            init: Some(init),
            next: NextState::Simple(next),
        }
    }

    #[test]
    fn bmc_finds_toggle_violation() {
        let ts = TransitionSystem {
            registers: vec![reg("q", 0, not(var("q")))],
            property: not(var("q")),
        };
        match ts.bmc(5) {
            BmcOutcome::CounterexampleAt { k, trace } => {
                assert_eq!(k, 1);
                assert!(trace.iter().any(|(n, v)| n == "q@1" && *v), "trace: {trace:?}");
            }
            other => panic!("expected a counterexample, got {other:?}"),
        }
    }

    #[test]
    fn k_induction_proves_latched_invariant() {
        let ts = TransitionSystem {
            registers: vec![reg("x", 1, var("x"))],
            property: var("x"),
        };
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
        assert_eq!(ts.bmc(6), BmcOutcome::NoneWithin(6));
    }

    #[test]
    fn k_induction_proves_two_register_mutex() {
        let ts = TransitionSystem {
            registers: vec![reg("a", 1, var("b")), reg("b", 0, var("a"))],
            property: not(BoundedExpr::And(Box::new(var("a")), Box::new(var("b")))),
        };
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    // ── Real Verilog ingestion ──────────────────────────────────────────────────────────

    #[test]
    fn verilog_toggle_violation_found() {
        let src = r#"
            module toggle(input clk);
              reg q;
              initial q = 0;
              always @(posedge clk) q <= ~q;
              assert property (@(posedge clk) ~q);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        match ts.bmc(5) {
            BmcOutcome::CounterexampleAt { k, trace } => {
                assert_eq!(k, 1);
                assert!(trace.iter().any(|(n, v)| n == "q@1" && *v));
            }
            other => panic!("expected a counterexample, got {other:?}"),
        }
    }

    #[test]
    fn verilog_latched_invariant_proven() {
        let src = r#"
            module latch(input clk);
              reg x;
              initial x = 1;
              always @(posedge clk) x <= x;
              assert property (x);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    #[test]
    fn verilog_mutex_swap_invariant_proven() {
        let src = r#"
            module mutex(input clk);
              reg a;
              reg b;
              initial begin a = 1; b = 0; end
              always @(posedge clk) begin
                a <= b;
                b <= a;
              end
              assert property (~(a & b));
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    // ── Multi-bit (bitvector) datapath registers ────────────────────────────────────────

    #[test]
    fn verilog_two_bit_counter_reaches_three() {
        // A 2-bit counter cnt++ reaches 3 at step 3, violating "cnt != 3".
        let src = r#"
            module counter(input clk);
              reg [1:0] cnt;
              initial cnt = 0;
              always @(posedge clk) cnt <= cnt + 1;
              assert property (cnt != 2'd3);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        assert_eq!(ts.registers[0].width, 2);
        match ts.bmc(6) {
            BmcOutcome::CounterexampleAt { k, .. } => assert_eq!(k, 3),
            other => panic!("expected a counterexample, got {other:?}"),
        }
    }

    #[test]
    fn verilog_four_bit_counter_bound_is_invariant() {
        // A 4-bit counter never exceeds 15 — a genuine (trivial) invariant, PROVEN.
        let src = r#"
            module counter(input clk);
              reg [3:0] cnt;
              initial cnt = 0;
              always @(posedge clk) cnt <= cnt + 1;
              assert property (cnt <= 4'd15);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        assert_eq!(ts.registers[0].width, 4);
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    #[test]
    fn verilog_three_bit_counter_violates_a_false_bound() {
        // A 3-bit counter DOES reach 5 — "cnt < 5" is violated at step 5.
        let src = r#"
            module counter(input clk);
              reg [2:0] cnt;
              initial cnt = 0;
              always @(posedge clk) cnt <= cnt + 1;
              assert property (cnt < 3'd5);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        match ts.bmc(8) {
            BmcOutcome::CounterexampleAt { k, .. } => assert_eq!(k, 5),
            other => panic!("expected a counterexample, got {other:?}"),
        }
    }

    #[test]
    fn verilog_datapath_register_equality_invariant() {
        // Two 4-bit regs initialised equal and updated identically stay equal — PROVEN.
        let src = r#"
            module mirror(input clk);
              reg [3:0] x;
              reg [3:0] y;
              initial begin x = 3; y = 3; end
              always @(posedge clk) begin
                x <= x + 1;
                y <= y + 1;
              end
              assert property (x == y);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    // ── Rejections (no silent mis-parse) ────────────────────────────────────────────────

    // ── Reset-as-free-input (the reset INPUT is a free variable each cycle) ──────────────

    #[test]
    fn verilog_reset_mirrored_registers_stay_equal() {
        // Two registers with identical reset+update logic over a FREE reset input stay equal
        // — a genuine invariant, PROVEN by k-induction regardless of when reset fires.
        let src = r#"
            module mirror(input clk, input rst);
              reg a;
              reg b;
              initial begin a = 0; b = 0; end
              always @(posedge clk) begin
                if (rst) a <= 0; else a <= ~a;
                if (rst) b <= 0; else b <= ~b;
              end
              assert property (a == b);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses reset logic");
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    #[test]
    fn verilog_multibit_reset_clears_counter() {
        // A counter that holds unless reset; with reset a free input, "cnt == 0" is NOT an
        // invariant (cnt grows when reset stays low), so BMC must find a violation.
        let src = r#"
            module counter(input clk, input rst);
              reg [3:0] cnt;
              initial cnt = 0;
              always @(posedge clk) if (rst) cnt <= 0; else cnt <= cnt + 1;
              assert property (cnt == 4'd0);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        match ts.bmc(4) {
            BmcOutcome::CounterexampleAt { k, .. } => assert_eq!(k, 1, "rst low for one cycle ⇒ cnt=1"),
            other => panic!("expected a counterexample, got {other:?}"),
        }
    }

    #[test]
    fn verilog_free_initial_state_is_explored() {
        // No `initial`: the register's start value is FREE, so "q is low" is violated
        // immediately (q@0 may be high). Proves we explore all initial states soundly.
        let src = r#"
            module m(input clk);
              reg q;
              always @(posedge clk) q <= q;
              assert property (~q);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses without initial");
        assert!(ts.registers[0].init.is_none(), "q must have a free initial state");
        match ts.bmc(3) {
            BmcOutcome::CounterexampleAt { k, .. } => assert_eq!(k, 0, "a free q can start high"),
            other => panic!("expected a step-0 counterexample, got {other:?}"),
        }
    }

    // ── Real hardware: arbiters, flow control, one-hot FSMs ──────────────────────────────

    #[test]
    fn verilog_arbiter_mutual_exclusion_proven() {
        // A 2-master round-robin arbiter. Grants are set in mutually-exclusive branches, so
        // "never two grants at once" is a genuine safety invariant — PROVEN by k-induction.
        let src = r#"
            module arbiter(input clk, input r0, input r1);
              reg g0;
              reg g1;
              reg turn;
              initial begin g0 = 0; g1 = 0; turn = 0; end
              always @(posedge clk) begin
                if (r0 && (!r1 || turn == 0)) begin
                  g0 <= 1;
                  g1 <= 0;
                end else if (r1) begin
                  g0 <= 0;
                  g1 <= 1;
                end else begin
                  g0 <= 0;
                  g1 <= 0;
                end
                turn <= ~turn;
              end
              assert property (~(g0 & g1));
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    #[test]
    fn verilog_naive_arbiter_double_grant_bug_found() {
        // The classic arbiter bug: granting each request independently. If both masters
        // request at once, BOTH are granted — a mutual-exclusion violation BMC must catch.
        let src = r#"
            module bad_arbiter(input clk, input r0, input r1);
              reg g0;
              reg g1;
              initial begin g0 = 0; g1 = 0; end
              always @(posedge clk) begin
                if (r0) g0 <= 1; else g0 <= 0;
                if (r1) g1 <= 1; else g1 <= 0;
              end
              assert property (~(g0 & g1));
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        match ts.bmc(4) {
            BmcOutcome::CounterexampleAt { k, trace } => {
                assert_eq!(k, 1, "both requested at t0 ⇒ double grant at t1");
                assert!(trace.iter().any(|(n, v)| n == "g0@1" && *v));
                assert!(trace.iter().any(|(n, v)| n == "g1@1" && *v));
            }
            other => panic!("expected a double-grant counterexample, got {other:?}"),
        }
    }

    #[test]
    fn verilog_fifo_occupancy_never_overflows_proven() {
        // A FIFO occupancy counter (depth 8) over free push/pop inputs: increment only when
        // not full, decrement only when not empty. "count never exceeds 8" is a genuine
        // datapath invariant — PROVEN for every push/pop sequence, even though `count` is a
        // 4-bit register that could otherwise reach 15.
        let src = r#"
            module fifo(input clk, input push, input pop);
              reg [3:0] count;
              initial count = 0;
              always @(posedge clk)
                if (push && (count < 4'd8) && !(pop && (count > 4'd0)))
                  count <= count + 1;
                else if (pop && (count > 4'd0) && !(push && (count < 4'd8)))
                  count <= count - 1;
                else
                  count <= count;
              assert property (count <= 4'd8);
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        assert_eq!(ts.registers[0].width, 4);
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    // A COMPLEX signalized intersection: NS/EW through (ns/ew) PLUS protected left turns
    // (nsl/ewl) PLUS a pedestrian phase (ped), sequenced by a 9-state controller with a
    // per-phase down-counter. Lights: 0=red, 1=green, 2=yellow; ped: 0=don't-walk, 1=walk.
    // Each phase fully sets ALL movements to a non-conflicting configuration, which keeps the
    // safety property 1-inductive.
    const TRAFFIC_SAFE: &str = r#"
        module traffic(input clk);
          reg [1:0] ns;
          reg [1:0] ew;
          reg [1:0] nsl;
          reg [1:0] ewl;
          reg ped;
          reg [3:0] phase;
          reg [2:0] timer;
          initial begin ns=2'd0; ew=2'd0; nsl=2'd1; ewl=2'd0; ped=1'd0; phase=4'd0; timer=3'd1; end
          always @(posedge clk)
            if (timer == 3'd0) begin
              timer <= 3'd1;
              if (phase == 4'd0) begin phase<=4'd1; nsl<=2'd2; ns<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end
              else if (phase == 4'd1) begin phase<=4'd2; nsl<=2'd0; ns<=2'd1; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end
              else if (phase == 4'd2) begin phase<=4'd3; ns<=2'd2; nsl<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end
              else if (phase == 4'd3) begin phase<=4'd4; ewl<=2'd1; ns<=2'd0; ew<=2'd0; nsl<=2'd0; ped<=1'd0; end
              else if (phase == 4'd4) begin phase<=4'd5; ewl<=2'd2; ns<=2'd0; ew<=2'd0; nsl<=2'd0; ped<=1'd0; end
              else if (phase == 4'd5) begin phase<=4'd6; ewl<=2'd0; ew<=2'd1; ns<=2'd0; nsl<=2'd0; ped<=1'd0; end
              else if (phase == 4'd6) begin phase<=4'd7; ew<=2'd2; ns<=2'd0; nsl<=2'd0; ewl<=2'd0; ped<=1'd0; end
              else if (phase == 4'd7) begin phase<=4'd8; ped<=1'd1; ns<=2'd0; ew<=2'd0; nsl<=2'd0; ewl<=2'd0; end
              else begin phase<=4'd0; nsl<=2'd1; ns<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end
            end else
              timer <= timer - 1;
          assert property (~((ns != 2'd0) & (ew != 2'd0)) & ~((ped == 1'd1) & ((ns != 2'd0) | (ew != 2'd0) | (nsl != 2'd0) | (ewl != 2'd0))));
        endmodule
    "#;

    #[test]
    fn verilog_complex_traffic_controller_is_proven_safe() {
        // No two conflicting movements are ever active together, and pedestrians only get a
        // WALK when EVERY vehicle movement is red — PROVEN by k-induction.
        let ts = parse_transition_system(TRAFFIC_SAFE).expect("parses");
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }

    #[test]
    fn verilog_buggy_traffic_controller_crashes() {
        // A subtle, dangerous bug: the pedestrian phase raises WALK but leaves NS GREEN, so
        // pedestrians are sent into live cross traffic. BMC finds the exact cycle.
        let buggy = TRAFFIC_SAFE.replace(
            "phase<=4'd8; ped<=1'd1; ns<=2'd0;",
            "phase<=4'd8; ped<=1'd1; ns<=2'd1;",
        );
        assert_ne!(buggy, TRAFFIC_SAFE, "the buggy variant must differ");
        let ts = parse_transition_system(&buggy).expect("parses");
        match ts.bmc(40) {
            BmcOutcome::CounterexampleAt { .. } => {}
            other => panic!("expected a pedestrian-vs-traffic conflict, got {other:?}"),
        }
    }

    #[test]
    fn witness_trace_of_safe_controller_is_a_conflict_free_run() {
        // The proven-safe controller has no counterexample to animate, so the Studio shows a
        // WITNESS execution instead. That witness must be a real run from the declared initial
        // state in which the safety property holds at EVERY step.
        let ts = parse_transition_system(TRAFFIC_SAFE).expect("parses");
        let steps = 16u32;
        let model = ts.witness_trace(steps).expect("a witness run exists");
        let m: HashMap<String, bool> = model.into_iter().collect();
        let bv = |name: &str, t: u32, width: u32| -> u64 {
            (0..width)
                .filter(|i| *m.get(&format!("{name}@{t}#{i}")).unwrap_or(&false))
                .fold(0u64, |acc, i| acc | (1 << i))
        };
        let bit = |name: &str, t: u32| -> bool { *m.get(&format!("{name}@{t}")).unwrap_or(&false) };
        // The run starts in the declared initial state: NS-left green, everything else red.
        assert_eq!(bv("nsl", 0, 2), 1, "init: NS-left should be green");
        assert_eq!(bv("ns", 0, 2), 0, "init: NS-through should be red");
        for t in 0..steps {
            let (ns, ew, nsl, ewl) = (bv("ns", t, 2), bv("ew", t, 2), bv("nsl", t, 2), bv("ewl", t, 2));
            let ped = bit("ped", t);
            let cross = ns != 0 && ew != 0;
            let ped_conflict = ped && (ns != 0 || ew != 0 || nsl != 0 || ewl != 0);
            assert!(
                !cross && !ped_conflict,
                "witness conflicts at step {t}: ns={ns} ew={ew} nsl={nsl} ewl={ewl} ped={ped}"
            );
        }
    }

    #[test]
    fn verilog_onehot_fsm_stays_one_hot_proven() {
        // A 3-state one-hot ring FSM. Exactly one of a/b/c is high — PROVEN invariant: the
        // rotation a<=c, b<=a, c<=b permutes the bits, preserving one-hotness.
        let src = r#"
            module onehot(input clk);
              reg a;
              reg b;
              reg c;
              initial begin a = 1; b = 0; c = 0; end
              always @(posedge clk) begin
                a <= c;
                b <= a;
                c <= b;
              end
              assert property ((a | b | c) & ~(a & b) & ~(a & c) & ~(b & c));
            endmodule
        "#;
        let ts = parse_transition_system(src).expect("parses");
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven);
    }
}
