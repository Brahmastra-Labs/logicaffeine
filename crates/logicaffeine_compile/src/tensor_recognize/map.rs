//! The pointwise map a loop the program writes means: `Repeat for i from 1 to N:` — or a nest of two, a row then its
//! columns — whose every turn computes one element of a tensor the program declared, from the elements at the same place
//! of the tensors it reads, fixed entries of others, the turn's own position and the program's constants, through
//! scalar arithmetic, the library's functions, the program's own functions and branches.
//!
//! Lifted as ONE op whose body is the turn's arithmetic, so an op nobody has written a sugar or a kernel for — a novel
//! activation, a novel attention mask, a novel score — compiles on its first compile. A turn is read by running it
//! symbolically: each `Let` binds a term, each `Set item` of the written tensor replaces the element's term, and a
//! branch joins its arms' terms under its condition. A turn that writes anything else, reads a place other than its
//! own or a fixed one, or does anything but compute is no map, and the walk lists the loop.

use std::collections::HashMap;

use logicaffeine_language::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt};

use super::{as_ident, as_index};
use crate::intern::{Interner, Symbol};

/// A comparison a branch of a turn tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Lt,
    LtEq,
    Gt,
    GtEq,
    Eq,
    NotEq,
}

/// A branch's condition.
#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Cmp(Cmp, Box<Term>, Box<Term>),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    Not(Box<Cond>),
}

/// A function a turn calls by its meaning: the library's `exp`, `ln` and `tanh`, and the builtins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fun {
    Exp,
    Ln,
    Tanh,
    Sqrt,
    Abs,
    Min,
    Max,
}

/// A turn's arithmetic over its element, as the VM computes it: an Int term is exact, a Real one an `f64`.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    /// The element at the turn's place of operand `k`.
    Here(usize),
    /// Element `at` (0-based) of operand `k`, whatever the turn: a scalar bound before the loop, an entry of a table.
    Fixed(usize, usize),
    /// The turn's position in loop `k` of the nest, from 1.
    Position(usize),
    /// The written tensor's element one turn of the outer loop before this one: the value a recurrence carries. It is
    /// read only on an arm no first turn takes.
    Earlier,
    Int(i64),
    Real(f64),
    Add(Box<Term>, Box<Term>),
    Sub(Box<Term>, Box<Term>),
    Mul(Box<Term>, Box<Term>),
    /// A quotient of Reals.
    Div(Box<Term>, Box<Term>),
    /// A quotient of Ints, truncated toward zero (`-7 / 2` is −3).
    Quot(Box<Term>, Box<Term>),
    /// A quotient of Ints, floored (`-7 // 2` is −4).
    FloorDiv(Box<Term>, Box<Term>),
    /// The remainder of a truncated quotient of Ints: `a − b·(a / b)`.
    Rem(Box<Term>, Box<Term>),
    /// `a ** b`: an exact power of Ints, `powf` once a Real is in it.
    Pow(Box<Term>, Box<Term>),
    Call(Fun, Vec<Term>),
    /// `then` where `cond` holds, `otherwise` elsewhere.
    Select { cond: Cond, then: Box<Term>, otherwise: Box<Term> },
}

impl Term {
    /// Whether the term reads the element at the turn's place of operand `k`.
    pub fn reads_here(&self, k: usize) -> bool {
        let mut found = false;
        self.visit(&mut |t| found |= *t == Term::Here(k));
        found
    }

    /// Every operand the term reads, at the turn's place or fixed, and every loop whose position it reads.
    pub fn visit(&self, f: &mut dyn FnMut(&Term)) {
        f(self);
        match self {
            Term::Here(_) | Term::Fixed(..) | Term::Position(_) | Term::Earlier | Term::Int(_) | Term::Real(_) => {}
            Term::Add(a, b)
            | Term::Sub(a, b)
            | Term::Mul(a, b)
            | Term::Div(a, b)
            | Term::Quot(a, b)
            | Term::FloorDiv(a, b)
            | Term::Rem(a, b)
            | Term::Pow(a, b) => {
                a.visit(f);
                b.visit(f);
            }
            Term::Call(_, args) => args.iter().for_each(|a| a.visit(f)),
            Term::Select { cond, then, otherwise } => {
                cond.visit(f);
                then.visit(f);
                otherwise.visit(f);
            }
        }
    }
}

impl Cond {
    fn visit(&self, f: &mut dyn FnMut(&Term)) {
        match self {
            Cond::Cmp(_, a, b) => {
                a.visit(f);
                b.visit(f);
            }
            Cond::And(p, q) | Cond::Or(p, q) => {
                p.visit(f);
                q.visit(f);
            }
            Cond::Not(p) => p.visit(f),
        }
    }
}

/// `out[p] = body(p)` for every place `p` of the loop nest, row-major: the map a counted loop the program wrote means.
#[derive(Clone, Debug, PartialEq)]
pub struct MapSpec {
    /// The tensor the loop writes, every element once a turn.
    pub out: Symbol,
    /// The tensors and scalars the turns read, in the order they are first read. `out` is among them when a turn keeps
    /// its element on some branch: the map updates it in place.
    pub ins: Vec<Symbol>,
    /// Each operand's length as the program binds it.
    pub lens: Vec<usize>,
    /// The loop nest's trip counts, outermost first.
    pub extents: Vec<usize>,
    /// The element a turn writes.
    pub body: Term,
    /// The element is a Real (an Int otherwise).
    pub real: bool,
}

impl MapSpec {
    /// The places the map writes: the product of its trip counts.
    pub fn n(&self) -> usize {
        self.extents.iter().product()
    }

    /// The position of `out` among the operands, when the map reads it.
    pub fn in_place(&self) -> Option<usize> {
        self.ins.iter().position(|s| *s == self.out).filter(|&k| self.body.reads_here(k))
    }

    /// Whether a turn reads what the turn one step before it along the outer loop wrote: a recurrence, its turns in order.
    pub fn recurs(&self) -> bool {
        let mut found = false;
        self.body.visit(&mut |t| found |= *t == Term::Earlier);
        found
    }

    /// The places one turn of the outer loop covers: what separates an element from the one it carries from.
    pub fn stride(&self) -> usize {
        self.extents[1..].iter().product()
    }
}

/// What the program bound before the loop, as the walk knows it.
pub struct MapContext<'c, 'a> {
    /// A tensor: its length and whether it holds Reals.
    pub tensor: &'c dyn Fn(Symbol) -> Option<(usize, bool)>,
    /// A one-element tensor an op computed before the loop — an arg-max's index: whether it holds a Real.
    pub scalar: &'c dyn Fn(Symbol) -> Option<bool>,
    /// A name the program bound to a literal number and never assigned again.
    pub constant: &'c dyn Fn(Symbol) -> Option<Term>,
    /// Whether a tensor is one no op has read or written yet — the only kind a map may write.
    pub fresh: &'c dyn Fn(Symbol) -> bool,
    /// The program's own functions, by name: their parameters and bodies.
    pub functions: &'c HashMap<Symbol, (Vec<Symbol>, &'a [Stmt<'a>])>,
    /// The library's functions the program has not redefined, which a turn calls by their meaning.
    pub library: &'c dyn Fn(Symbol) -> bool,
}

/// How deep a program's functions may call one another inside a turn.
const CALL_DEPTH: usize = 8;

/// A term and whether it is a Real.
type Typed = (Term, bool);

/// A turn as far as it has run: the names it has bound, and the written element's term once a statement sets it.
#[derive(Clone, Default)]
struct State {
    env: HashMap<Symbol, Typed>,
    written: Option<Typed>,
}

struct Turn<'c, 'a, 'i> {
    cx: &'c MapContext<'c, 'a>,
    interner: &'i Interner,
    /// The loop variables, outermost first, with their trip counts.
    vars: Vec<(Symbol, usize)>,
    out: Option<Symbol>,
    ins: Vec<Symbol>,
}

impl Turn<'_, '_, '_> {
    fn operand(&mut self, s: Symbol) -> usize {
        match self.ins.iter().position(|x| *x == s) {
            Some(k) => k,
            None => {
                self.ins.push(s);
                self.ins.len() - 1
            }
        }
    }

    /// `(coefficients of the loop positions, constant)` of an Int term built of positions, literals, sums, differences
    /// and products by a literal.
    fn affine(&self, t: &Term) -> Option<(Vec<i64>, i64)> {
        let zero = || vec![0i64; self.vars.len()];
        match t {
            Term::Int(v) => Some((zero(), *v)),
            Term::Position(k) => {
                let mut c = zero();
                c[*k] = 1;
                Some((c, 0))
            }
            Term::Add(a, b) | Term::Sub(a, b) => {
                let ((ca, ka), (cb, kb)) = (self.affine(a)?, self.affine(b)?);
                let sign = if matches!(t, Term::Sub(..)) { -1 } else { 1 };
                Some((ca.iter().zip(&cb).map(|(x, y)| x + sign * y).collect(), ka + sign * kb))
            }
            Term::Mul(a, b) => {
                let ((ca, ka), (cb, kb)) = (self.affine(a)?, self.affine(b)?);
                match (ca.iter().all(|c| *c == 0), cb.iter().all(|c| *c == 0)) {
                    (true, _) => Some((cb.iter().map(|c| c * ka).collect(), ka * kb)),
                    (_, true) => Some((ca.iter().map(|c| c * kb).collect(), ka * kb)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether an index term names the turn's own place: `i` for one loop, `(r − 1)·C + c` for a nest of two.
    fn is_here(&self, t: &Term) -> bool {
        self.offset_from_here(t) == Some(0)
    }

    /// How far an index term's place is from the turn's own, when the two differ by a constant.
    fn offset_from_here(&self, t: &Term) -> Option<i64> {
        let (coeffs, constant) = self.affine(t)?;
        let mut stride = 1i64;
        let mut want = vec![0i64; self.vars.len()];
        let mut base = 1i64;
        for (k, (_, extent)) in self.vars.iter().enumerate().rev() {
            want[k] = stride;
            base -= stride;
            stride *= *extent as i64;
        }
        (coeffs == want).then_some(constant - base)
    }

    fn expr(&mut self, e: &Expr, st: &State, depth: usize) -> Option<Typed> {
        match e {
            Expr::Literal(Literal::Number(n)) => Some((Term::Int(*n), false)),
            Expr::Literal(Literal::Float(f)) => Some((Term::Real(*f), true)),
            Expr::Identifier(s) => {
                if let Some(t) = st.env.get(s) {
                    return Some(t.clone());
                }
                if let Some(k) = self.vars.iter().position(|(v, _)| v == s) {
                    return Some((Term::Position(k), false));
                }
                if let Some(c) = (self.cx.constant)(*s) {
                    let real = matches!(c, Term::Real(_));
                    return Some((c, real));
                }
                if let Some(real) = (self.cx.scalar)(*s) {
                    return Some((Term::Fixed(self.operand(*s), 0), real));
                }
                // The Real constants a program that binds no such name reads bare.
                match self.interner.resolve(*s) {
                    "e" if (self.cx.tensor)(*s).is_none() => Some((Term::Real(std::f64::consts::E), true)),
                    "pi" if (self.cx.tensor)(*s).is_none() => Some((Term::Real(std::f64::consts::PI), true)),
                    _ => None,
                }
            }
            Expr::Index { .. } => {
                let (collection, index) = as_index(e)?;
                let tensor = as_ident(collection)?;
                let (len, real) = (self.cx.tensor)(tensor)?;
                let (at, int_real) = self.expr(index, st, depth)?;
                if int_real {
                    return None;
                }
                if Some(tensor) == self.out {
                    // The written tensor is read at the turn's own place, or where the turn one step before along the
                    // outer loop wrote: anywhere else a turn may or may not have written it yet.
                    let stride: usize = self.vars[1..].iter().map(|(_, e)| *e).product();
                    match self.offset_from_here(&at) {
                        Some(0) => {
                            if let Some(now) = &st.written {
                                return Some(now.clone());
                            }
                        }
                        Some(d) if d == -(stride as i64) => return Some((Term::Earlier, real)),
                        _ => return None,
                    }
                }
                if self.is_here(&at) {
                    let n: usize = self.vars.iter().map(|(_, e)| *e).product();
                    (len >= n).then(|| (Term::Here(self.operand(tensor)), real))
                } else {
                    let Term::Int(p) = at else { return None };
                    (p >= 1 && p as usize <= len).then(|| (Term::Fixed(self.operand(tensor), p as usize - 1), real))
                }
            }
            Expr::BinaryOp { op, left, right } => {
                let (a, ra) = self.expr(left, st, depth)?;
                let (b, rb) = self.expr(right, st, depth)?;
                let real = ra || rb;
                let (a, b) = (Box::new(a), Box::new(b));
                Some(match op {
                    BinaryOpKind::Add => (Term::Add(a, b), real),
                    BinaryOpKind::Subtract => (Term::Sub(a, b), real),
                    BinaryOpKind::Multiply => (Term::Mul(a, b), real),
                    BinaryOpKind::Divide if real => (Term::Div(a, b), true),
                    BinaryOpKind::Divide => (Term::Quot(a, b), false),
                    BinaryOpKind::FloorDivide if !real => (Term::FloorDiv(a, b), false),
                    BinaryOpKind::Modulo if !real => (Term::Rem(a, b), false),
                    BinaryOpKind::Pow => (Term::Pow(a, b), real),
                    _ => return None,
                })
            }
            Expr::Call { function, args } => self.call(*function, args, st, depth),
            _ => None,
        }
    }

    fn call(&mut self, function: Symbol, args: &[&Expr], st: &State, depth: usize) -> Option<Typed> {
        let args: Vec<Typed> = args.iter().map(|a| self.expr(a, st, depth)).collect::<Option<_>>()?;
        if let Some((params, body)) = self.cx.functions.get(&function) {
            if depth >= CALL_DEPTH || params.len() != args.len() {
                return None;
            }
            let mut inner = State { env: params.iter().copied().zip(args).collect(), written: None };
            return self.returned(body, &mut inner, depth + 1);
        }
        if !(self.cx.library)(function) {
            return None;
        }
        let real = args.iter().any(|(_, r)| *r);
        let (fun, arity, real) = match self.interner.resolve(function) {
            "exp" => (Fun::Exp, 1, true),
            "ln" => (Fun::Ln, 1, true),
            "tanh" => (Fun::Tanh, 1, true),
            "sqrt" => (Fun::Sqrt, 1, true),
            "abs" => (Fun::Abs, 1, real),
            "min" => (Fun::Min, 2, real),
            "max" => (Fun::Max, 2, real),
            _ => return None,
        };
        (args.len() == arity).then(|| (Term::Call(fun, args.into_iter().map(|(t, _)| t).collect()), real))
    }

    /// The value a function's body returns: its `Let`s bound in turn, then one `Return`.
    fn returned(&mut self, body: &[Stmt], st: &mut State, depth: usize) -> Option<Typed> {
        let (last, lets) = body.split_last()?;
        for s in lets {
            let Stmt::Let { var, value, .. } = s else { return None };
            let t = self.expr(value, st, depth)?;
            st.env.insert(*var, t);
        }
        let Stmt::Return { value: Some(value) } = last else { return None };
        self.expr(value, st, depth)
    }

    fn cond(&mut self, e: &Expr, st: &State) -> Option<Cond> {
        match e {
            Expr::Not { operand } => Some(Cond::Not(Box::new(self.cond(operand, st)?))),
            Expr::BinaryOp { op: op @ (BinaryOpKind::And | BinaryOpKind::Or), left, right } => {
                let (p, q) = (Box::new(self.cond(left, st)?), Box::new(self.cond(right, st)?));
                Some(if *op == BinaryOpKind::And { Cond::And(p, q) } else { Cond::Or(p, q) })
            }
            Expr::BinaryOp { op, left, right } => {
                let cmp = match op {
                    BinaryOpKind::Lt => Cmp::Lt,
                    BinaryOpKind::LtEq => Cmp::LtEq,
                    BinaryOpKind::Gt => Cmp::Gt,
                    BinaryOpKind::GtEq => Cmp::GtEq,
                    BinaryOpKind::Eq => Cmp::Eq,
                    BinaryOpKind::NotEq => Cmp::NotEq,
                    _ => return None,
                };
                let (a, _) = self.expr(left, st, 0)?;
                let (b, _) = self.expr(right, st, 0)?;
                Some(Cond::Cmp(cmp, Box::new(a), Box::new(b)))
            }
            _ => None,
        }
    }

    /// Run a turn's statements: each `Let`, and each `Set` of a name the turn bound, rebinds it; each `Set item` of the
    /// written tensor at the turn's place replaces the element's term.
    fn run(&mut self, body: &[Stmt], st: &mut State) -> Option<()> {
        for s in body {
            match s {
                Stmt::Let { var, value, .. } => {
                    let t = self.expr(value, st, 0)?;
                    st.env.insert(*var, t);
                }
                Stmt::Set { target, value } if st.env.contains_key(target) => {
                    let t = self.expr(value, st, 0)?;
                    st.env.insert(*target, t);
                }
                Stmt::SetIndex { collection, index, value } => {
                    (Some(as_ident(collection)?) == self.out).then_some(())?;
                    let (at, real) = self.expr(index, st, 0)?;
                    (!real && self.is_here(&at)).then_some(())?;
                    st.written = Some(self.expr(value, st, 0)?);
                }
                Stmt::If { cond, then_block, else_block } => {
                    let cond = self.cond(cond, st)?;
                    let mut then_st = st.clone();
                    self.run(then_block, &mut then_st)?;
                    let mut else_st = st.clone();
                    if let Some(eb) = else_block {
                        self.run(eb, &mut else_st)?;
                    }
                    // A name bound before the branch is joined under its condition; one bound in an arm ends with it.
                    let join = |a: Typed, b: Typed| -> Option<Typed> {
                        if a == b {
                            return Some(a);
                        }
                        (a.1 == b.1).then(|| (Term::Select { cond: cond.clone(), then: Box::new(a.0), otherwise: Box::new(b.0) }, a.1))
                    };
                    for k in st.env.keys().copied().collect::<Vec<_>>() {
                        let t = join(then_st.env.get(&k)?.clone(), else_st.env.get(&k)?.clone())?;
                        st.env.insert(k, t);
                    }
                    if then_st.written.is_some() || else_st.written.is_some() {
                        let a = match then_st.written {
                            Some(t) => t,
                            None => self.kept()?,
                        };
                        let b = match else_st.written {
                            Some(t) => t,
                            None => self.kept()?,
                        };
                        // A Real tensor holds an Int an arm writes as it holds a Real.
                        let real = self.out.and_then(|o| (self.cx.tensor)(o)).is_some_and(|(_, r)| r);
                        st.written = Some(if real { join((a.0, true), (b.0, true))? } else { join(a, b)? });
                    }
                }
                _ => return None,
            }
        }
        Some(())
    }

    /// The written tensor's element as the turn found it: what an arm that writes nothing keeps.
    fn kept(&mut self) -> Option<Typed> {
        let out = self.out?;
        let (_, real) = (self.cx.tensor)(out)?;
        Some((Term::Here(self.operand(out)), real))
    }
}

/// Whether every [`Term::Earlier`] in `t` lies on an arm that excludes the outer loop's first turn — `safe` when `t`
/// itself does.
fn carried_after_the_first(t: &Term, safe: bool) -> bool {
    match t {
        Term::Earlier => safe,
        Term::Select { cond, then, otherwise } => {
            let (then_safe, else_safe) = excludes_first(cond);
            cond_terms(cond).iter().all(|x| carried_after_the_first(x, safe))
                && carried_after_the_first(then, safe || then_safe)
                && carried_after_the_first(otherwise, safe || else_safe)
        }
        _ => children(t).iter().all(|c| carried_after_the_first(c, safe)),
    }
}

/// The terms a condition compares.
fn cond_terms(c: &Cond) -> Vec<&Term> {
    match c {
        Cond::Cmp(_, a, b) => vec![&**a, &**b],
        Cond::And(p, q) | Cond::Or(p, q) => cond_terms(p).into_iter().chain(cond_terms(q)).collect(),
        Cond::Not(p) => cond_terms(p),
    }
}

/// The direct sub-terms of `t`.
fn children(t: &Term) -> Vec<&Term> {
    match t {
        Term::Add(a, b)
        | Term::Sub(a, b)
        | Term::Mul(a, b)
        | Term::Div(a, b)
        | Term::Quot(a, b)
        | Term::FloorDiv(a, b)
        | Term::Rem(a, b)
        | Term::Pow(a, b) => vec![&**a, &**b],
        Term::Call(_, args) => args.iter().collect(),
        _ => Vec::new(),
    }
}

/// Whether a condition's holding — and its failing — excludes the first turn of the outer loop: `(the then arm never runs
/// at turn 1, the otherwise arm never does)`, decided by [`at_the_first`].
fn excludes_first(c: &Cond) -> (bool, bool) {
    match at_the_first(c) {
        Some(true) => (false, true),
        Some(false) => (true, false),
        None => (false, false),
    }
}

/// Whether `c` holds on the first turn of the outer loop, whatever the turn's other positions: its comparisons read on
/// that turn, where the outer position is 1, as Int arithmetic of literals — `t > 1`, `(t − 1) % 4 = 0` — and `None`
/// where they read anything else.
fn at_the_first(c: &Cond) -> Option<bool> {
    fn int(t: &Term) -> Option<i64> {
        match t {
            Term::Int(v) => Some(*v),
            Term::Position(0) => Some(1),
            Term::Add(a, b) => int(a)?.checked_add(int(b)?),
            Term::Sub(a, b) => int(a)?.checked_sub(int(b)?),
            Term::Mul(a, b) => int(a)?.checked_mul(int(b)?),
            Term::Quot(a, b) => int(a)?.checked_div(int(b)?),
            Term::Rem(a, b) => int(a)?.checked_rem(int(b)?),
            Term::FloorDiv(a, b) => {
                let (x, y) = (int(a)?, int(b)?);
                let q = x.checked_div(y)?;
                Some(if x % y != 0 && ((x < 0) != (y < 0)) { q - 1 } else { q })
            }
            _ => None,
        }
    }
    match c {
        Cond::Cmp(op, a, b) => {
            let (a, b) = (int(a)?, int(b)?);
            Some(match op {
                Cmp::Lt => a < b,
                Cmp::LtEq => a <= b,
                Cmp::Gt => a > b,
                Cmp::GtEq => a >= b,
                Cmp::Eq => a == b,
                Cmp::NotEq => a != b,
            })
        }
        Cond::And(p, q) => match (at_the_first(p), at_the_first(q)) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        Cond::Or(p, q) => match (at_the_first(p), at_the_first(q)) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
        Cond::Not(p) => at_the_first(p).map(|v| !v),
    }
}

/// The tensor the first `Set item` of a turn writes, however deep in its branches.
fn first_written(body: &[Stmt]) -> Option<Symbol> {
    body.iter().find_map(|s| match s {
        Stmt::SetIndex { collection, .. } => as_ident(collection),
        Stmt::If { then_block, else_block, .. } => first_written(then_block).or_else(|| else_block.and_then(|e| first_written(e))),
        _ => None,
    })
}

/// The map a counted loop the program wrote means ([`MapSpec`]), or `None` when its turns are not one.
pub fn recognize_map(s: &Stmt, interner: &Interner, cx: &MapContext) -> Option<MapSpec> {
    let mut vars = Vec::new();
    let mut body: &[Stmt] = std::slice::from_ref(s);
    while let [Stmt::Repeat { pattern: Pattern::Identifier(v), iterable: Expr::Range { start, end }, body: inner }] = body {
        let (Expr::Literal(Literal::Number(1)), Expr::Literal(Literal::Number(n))) = (*start, *end) else { return None };
        if *n < 1 || vars.len() == 2 {
            return None;
        }
        vars.push((*v, *n as usize));
        body = inner;
    }
    if vars.is_empty() {
        return None;
    }
    // The tensor the turn writes is known before the turn runs, so a read of it ahead of the write — the carried value
    // a recurrence reads first — is read as one.
    let out = first_written(body)?;
    (cx.fresh)(out).then_some(())?;
    let mut turn = Turn { cx, interner, vars, out: Some(out), ins: Vec::new() };
    let mut st = State::default();
    turn.run(body, &mut st)?;
    let (term, real) = st.written?;
    let out = turn.out?;
    // A turn reads the written tensor at its own place alone: a fixed entry may be another turn's.
    let k_out = turn.ins.iter().position(|s| *s == out);
    let mut elsewhere = false;
    term.visit(&mut |t| elsewhere |= matches!(t, Term::Fixed(k, _) if Some(*k) == k_out));
    if elsewhere {
        return None;
    }
    let n: usize = turn.vars.iter().map(|(_, e)| *e).product();
    let (len, out_real) = (cx.tensor)(out)?;
    if len != n || (real && !out_real) {
        return None;
    }
    // A carried value is read only on an arm the first turn of the outer loop never takes: there it lies before the
    // tensor, which every tier refuses.
    if !carried_after_the_first(&term, false) {
        return None;
    }
    let lens = turn.ins.iter().map(|s| (cx.tensor)(*s).map(|(len, _)| len).or_else(|| (cx.scalar)(*s).map(|_| 1))).collect::<Option<Vec<_>>>()?;
    Some(MapSpec {
        out,
        lens,
        ins: turn.ins,
        extents: turn.vars.iter().map(|(_, e)| *e).collect(),
        body: term,
        real: out_real,
    })
}
