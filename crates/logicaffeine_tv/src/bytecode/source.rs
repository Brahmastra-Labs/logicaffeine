//! The source side: each function's statements as a small control-flow graph whose meaning is the tree-walker's, and
//! its symbolic execution from a cut point to the next.
//!
//! The graph keeps the source's own structure — one loop head per `While`/`Repeat`, its loop nest computed from the
//! graph as the bytecode's is — and names every variable by the binding it refers to, so a `Let` inside a block
//! shadows rather than overwrites. A `Repeat for i from a to b` evaluates its bounds once into two hidden variables, its
//! counter and its last value, and binds `i` from the counter each time round, so a `Set` of `i` in the body does not
//! move the walk. A bound may be any Int, as the runtime's range takes it.
//!
//! A function that returns a call to itself has a second graph, used when its bytecode loops where the source recurses:
//! the body begins at a loop head, and `Return f(args)` evaluates the arguments, rebinds the parameters and goes back
//! to it. The two graphs mean the same thing, up to how deep the calls nest; call depth is a resource limit, not
//! something either graph models.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::path::loop_nest;

use logicaffeine_compile::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt, TypeExpr};
use logicaffeine_compile::{Interner, Symbol};
use z3::ast::{Ast, Bool};

use super::path::{
    all_constant, concrete_return, dead, fold_returns, opaque_call, Budget, CallKind, Calls, End, LoopNode, Mode, Path, State,
    CONSTANT_CALL_STEPS,
};
use super::value::{binop, not, truth, Kind, Lit, SymVal};
use crate::symexec::Unsupported;

/// A variable: one binding in one function.
pub type VarId = u32;

/// An expression, its names resolved to bindings.
#[derive(Clone, Debug)]
pub enum SExpr {
    Lit(Lit),
    Var(VarId),
    Bin(BinaryOpKind, Box<SExpr>, Box<SExpr>),
    Not(Box<SExpr>),
    Call(String, Vec<SExpr>),
}

/// One step of a function's graph.
#[derive(Clone, Debug)]
pub enum SInstr {
    Assign(VarId, SExpr),
    Show(SExpr),
    Eval(SExpr),
    /// A runtime assertion: raises when the condition is false.
    Assert(SExpr),
    Branch(SExpr, usize, usize),
    Jump(usize),
    /// A loop head.
    Head,
    Return(Option<SExpr>),
    /// The end of the body: a function returns `nothing`, `## Main` halts.
    Finish,
}

/// A function (or `## Main`) as a graph.
#[derive(Clone, Debug)]
pub struct SFunc {
    pub name: String,
    pub params: Vec<(VarId, Kind)>,
    pub ret: Option<Kind>,
    pub code: Vec<SInstr>,
    pub loops: Vec<LoopNode>,
    pub head_of: HashMap<usize, usize>,
    pub names: Vec<String>,
    pub main: bool,
    pub shows: bool,
    pub calls: BTreeSet<String>,
}

/// Every function the program defines, each lowered or refused, `## Main`, and the second graph of each function that
/// returns a call to itself.
pub struct SProgram {
    pub functions: BTreeMap<String, Result<SFunc, Unsupported>>,
    pub tail: BTreeMap<String, SFunc>,
    pub main: Result<SFunc, Unsupported>,
}

/// The kind a declared type names, for the two the fragment admits.
fn declared_kind(ty: &TypeExpr, interner: &Interner) -> Result<Kind, Unsupported> {
    match ty {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => match interner.resolve(*s) {
            "Int" => Ok(Kind::Int),
            "Bool" => Ok(Kind::Bool),
            other => Err(Unsupported(format!("a value of type {other}"))),
        },
        _ => Err(Unsupported("a parameter or return of a compound type".into())),
    }
}

/// Lower a parsed program: every non-native function and `## Main`.
pub fn lower_program(stmts: &[Stmt], interner: &Interner) -> SProgram {
    let mut functions = BTreeMap::new();
    let mut tail = BTreeMap::new();
    let mut main = Vec::new();
    for s in stmts {
        match s {
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body,
                return_type,
                is_native,
                ..
            } => {
                if *is_native {
                    continue;
                }
                let fname = interner.resolve(*name).to_string();
                let lower = |looped: bool| -> Result<(SFunc, usize), Unsupported> {
                    if !generics.is_empty() {
                        return Err(Unsupported("a generic function".into()));
                    }
                    let mut l = Lower::new(interner, &fname, false);
                    let mut ps = Vec::new();
                    for (p, ty) in params {
                        let kind = declared_kind(ty, interner)?;
                        ps.push((l.bind(*p), kind));
                    }
                    if looped {
                        l.code.push(SInstr::Head);
                        l.tail = Some(ps.iter().map(|(v, _)| *v).collect());
                    }
                    let ret = match return_type {
                        Some(t) => Some(declared_kind(t, interner)?),
                        None => None,
                    };
                    l.block(body)?;
                    l.code.push(SInstr::Finish);
                    let tail_calls = l.tail_calls;
                    Ok((l.finish(ps, ret)?, tail_calls))
                };
                let plain = lower(false).map(|(f, _)| f);
                if let Ok((looped, n)) = lower(true) {
                    if n > 0 {
                        tail.insert(fname.clone(), looped);
                    }
                }
                functions.insert(fname, plain);
            }
            Stmt::Site(_) | Stmt::TestDef { .. } | Stmt::StructDef { .. } => {}
            other => main.push(other),
        }
    }
    let main = (|| {
        let mut l = Lower::new(interner, "## Main", true);
        for s in main {
            l.stmt(s)?;
        }
        l.code.push(SInstr::Finish);
        l.finish(Vec::new(), None)
    })();
    SProgram { functions, tail, main }
}

struct Lower<'i> {
    interner: &'i Interner,
    name: String,
    main: bool,
    code: Vec<SInstr>,
    scopes: Vec<HashMap<Symbol, VarId>>,
    names: Vec<String>,
    open: Vec<Vec<usize>>,
    shows: bool,
    calls: BTreeSet<String>,
    tail: Option<Vec<VarId>>,
    tail_calls: usize,
}

impl<'i> Lower<'i> {
    fn new(interner: &'i Interner, name: &str, main: bool) -> Self {
        Lower {
            interner,
            name: name.to_string(),
            main,
            code: Vec::new(),
            scopes: vec![HashMap::new()],
            names: Vec::new(),
            open: Vec::new(),
            shows: false,
            calls: BTreeSet::new(),
            tail: None,
            tail_calls: 0,
        }
    }

    fn finish(self, params: Vec<(VarId, Kind)>, ret: Option<Kind>) -> Result<SFunc, Unsupported> {
        let mut succ = BTreeMap::new();
        let mut work = vec![0usize];
        while let Some(i) = work.pop() {
            if succ.contains_key(&i) {
                continue;
            }
            let next = match &self.code[i] {
                SInstr::Branch(_, t, f) => vec![*t, *f],
                SInstr::Jump(t) => vec![*t],
                SInstr::Return(_) | SInstr::Finish => vec![],
                _ => vec![i + 1],
            };
            work.extend(next.iter().copied());
            succ.insert(i, next);
        }
        let loops = loop_nest(0, &succ).map_err(|why| Unsupported(format!("`{}` {why}", self.name)))?;
        let head_of = loops.iter().enumerate().map(|(k, l)| (l.head, k)).collect();
        Ok(SFunc {
            name: self.name,
            params,
            ret,
            code: self.code,
            loops,
            head_of,
            names: self.names,
            main: self.main,
            shows: self.shows,
            calls: self.calls,
        })
    }

    fn hidden(&mut self, name: String) -> VarId {
        self.names.push(name);
        (self.names.len() - 1) as VarId
    }

    fn bind(&mut self, sym: Symbol) -> VarId {
        let id = self.hidden(self.interner.resolve(sym).to_string());
        self.scopes.last_mut().expect("a scope is open").insert(sym, id);
        id
    }

    fn lookup(&self, sym: Symbol) -> Result<VarId, Unsupported> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(&sym).copied())
            .ok_or_else(|| Unsupported(format!("`{}` names nothing this function binds", self.interner.resolve(sym))))
    }

    fn block(&mut self, stmts: &[Stmt]) -> Result<(), Unsupported> {
        self.scopes.push(HashMap::new());
        for s in stmts {
            self.stmt(s)?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn here(&self) -> usize {
        self.code.len()
    }

    fn patch(&mut self, at: usize, target: usize) {
        match &mut self.code[at] {
            SInstr::Jump(t) => *t = target,
            SInstr::Branch(_, _, f) => *f = target,
            _ => unreachable!("only jumps and branches are patched"),
        }
    }

    /// Open a loop at the next instruction, its head; returns the head.
    fn open_loop(&mut self) -> usize {
        let head = self.here();
        self.code.push(SInstr::Head);
        self.open.push(Vec::new());
        head
    }

    fn close_loop(&mut self, exit: usize) {
        let breaks = self.open.pop().expect("a loop is open");
        for b in breaks {
            self.patch(b, exit);
        }
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), Unsupported> {
        match s {
            Stmt::Let { var, value, ty, .. } => {
                if let Some(t) = ty {
                    declared_kind(t, self.interner)?;
                }
                let e = self.expr(value)?;
                let v = self.bind(*var);
                self.code.push(SInstr::Assign(v, e));
            }
            Stmt::Set { target, value } => {
                let e = self.expr(value)?;
                let v = self.lookup(*target)?;
                self.code.push(SInstr::Assign(v, e));
            }
            Stmt::Show { object, recipient } => match recipient {
                Expr::Identifier(r) if self.interner.resolve(*r) == "show" => {
                    let e = self.expr(object)?;
                    self.shows = true;
                    self.code.push(SInstr::Show(e));
                }
                _ => return Err(Unsupported("`Show` to a recipient other than the console".into())),
            },
            Stmt::Call { function, args } => {
                let e = self.call(*function, args)?;
                self.code.push(SInstr::Eval(e));
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let c = self.expr(cond)?;
                let branch = self.here();
                self.code.push(SInstr::Branch(c, branch + 1, usize::MAX));
                self.block(then_block)?;
                match else_block {
                    Some(eb) => {
                        let skip = self.here();
                        self.code.push(SInstr::Jump(usize::MAX));
                        let else_at = self.here();
                        self.patch(branch, else_at);
                        self.block(eb)?;
                        let end = self.here();
                        self.patch(skip, end);
                    }
                    None => {
                        let end = self.here();
                        self.patch(branch, end);
                    }
                }
            }
            Stmt::While { cond, body, decreasing } => {
                if decreasing.is_some() {
                    return Err(Unsupported("a `While` with a `decreasing` measure".into()));
                }
                let head = self.open_loop();
                let c = self.expr(cond)?;
                let branch = self.here();
                self.code.push(SInstr::Branch(c, branch + 1, usize::MAX));
                self.block(body)?;
                self.code.push(SInstr::Jump(head));
                let exit = self.here();
                self.patch(branch, exit);
                self.close_loop(exit);
            }
            Stmt::Repeat {
                pattern: Pattern::Identifier(var),
                iterable: Expr::Range { start, end },
                body,
            } => {
                let label = self.interner.resolve(*var).to_string();
                let lo = self.expr(start)?;
                let hi = self.expr(end)?;
                let at = self.hidden(format!("{label}·at"));
                let last = self.hidden(format!("{label}·last"));
                self.code.push(SInstr::Assign(at, lo));
                self.code.push(SInstr::Assign(last, hi));
                self.scopes.push(HashMap::new());
                let head = self.open_loop();
                let branch = self.here();
                self.code.push(SInstr::Branch(
                    SExpr::Bin(BinaryOpKind::LtEq, Box::new(SExpr::Var(at)), Box::new(SExpr::Var(last))),
                    branch + 1,
                    usize::MAX,
                ));
                let item = self.bind(*var);
                self.code.push(SInstr::Assign(item, SExpr::Var(at)));
                self.block(body)?;
                self.code.push(SInstr::Assign(
                    at,
                    SExpr::Bin(BinaryOpKind::Add, Box::new(SExpr::Var(at)), Box::new(SExpr::Lit(Lit::Int(1)))),
                ));
                self.code.push(SInstr::Jump(head));
                let exit = self.here();
                self.patch(branch, exit);
                self.close_loop(exit);
                self.scopes.pop();
            }
            Stmt::Repeat { .. } => return Err(Unsupported("a `Repeat` over anything but a range".into())),
            Stmt::Break => {
                let at = self.here();
                self.code.push(SInstr::Jump(usize::MAX));
                self.open
                    .last_mut()
                    .ok_or_else(|| Unsupported("`Break` outside a loop".into()))?
                    .push(at);
            }
            Stmt::Return { value } => {
                if self.main {
                    return Err(Unsupported("`Return` in `## Main`".into()));
                }
                if let (Some(params), Some(Expr::Call { function, args })) = (self.tail.clone(), value) {
                    if self.interner.resolve(*function) == self.name && args.len() == params.len() {
                        let mut temps = Vec::new();
                        for (i, a) in args.iter().enumerate() {
                            let e = self.expr(a)?;
                            let t = self.hidden(format!("{}·arg{i}", self.name));
                            self.code.push(SInstr::Assign(t, e));
                            temps.push(t);
                        }
                        for (p, t) in params.iter().zip(temps) {
                            self.code.push(SInstr::Assign(*p, SExpr::Var(t)));
                        }
                        self.code.push(SInstr::Jump(0));
                        self.tail_calls += 1;
                        return Ok(());
                    }
                }
                let e = value.map(|v| self.expr(v)).transpose()?;
                self.code.push(SInstr::Return(e));
            }
            Stmt::RuntimeAssert {
                condition,
                dim: None,
                reason: None,
                ..
            } => {
                let c = self.expr(condition)?;
                self.code.push(SInstr::Assert(c));
            }
            Stmt::Site(_) => {}
            other => return Err(Unsupported(format!("the statement {}", stmt_name(other)))),
        }
        Ok(())
    }

    fn call(&mut self, function: Symbol, args: &[&Expr]) -> Result<SExpr, Unsupported> {
        let name = self.interner.resolve(function).to_string();
        let args = args.iter().map(|a| self.expr(a)).collect::<Result<Vec<_>, _>>()?;
        self.calls.insert(name.clone());
        Ok(SExpr::Call(name, args))
    }

    fn expr(&mut self, e: &Expr) -> Result<SExpr, Unsupported> {
        Ok(match e {
            Expr::Literal(Literal::Number(n)) => SExpr::Lit(Lit::Int(*n)),
            Expr::Literal(Literal::Boolean(b)) => SExpr::Lit(Lit::Bool(*b)),
            Expr::Literal(Literal::Nothing) => SExpr::Lit(Lit::Nothing),
            Expr::Literal(_) => return Err(Unsupported("a literal that is not an Int or a Bool".into())),
            Expr::Identifier(s) => SExpr::Var(self.lookup(*s)?),
            Expr::BinaryOp { op, left, right } => match op {
                BinaryOpKind::ExactDivide
                | BinaryOpKind::Concat
                | BinaryOpKind::SeqConcat
                | BinaryOpKind::ApproxEq => return Err(Unsupported(format!("the operator {op:?}"))),
                _ => SExpr::Bin(*op, Box::new(self.expr(left)?), Box::new(self.expr(right)?)),
            },
            Expr::Not { operand } => SExpr::Not(Box::new(self.expr(operand)?)),
            Expr::Call { function, args } => self.call(*function, args)?,
            _ => return Err(Unsupported("an expression outside the integer fragment".into())),
        })
    }
}

fn stmt_name(s: &Stmt) -> &'static str {
    match s {
        Stmt::Swap { .. } => "Swap",
        Stmt::Push { .. } => "Push",
        Stmt::SetIndex { .. } => "SetIndex",
        Stmt::SetField { .. } => "SetField",
        Stmt::Inspect { .. } => "Inspect",
        Stmt::Fail { .. } => "Fail",
        Stmt::Give { .. } => "Give",
        Stmt::FunctionDef { .. } => "a nested FunctionDef",
        _ => "outside the integer fragment",
    }
}

/// A source function run: each state is a position, its bindings, its path condition and what it showed so far.
struct Walk {
    pc: usize,
    env: BTreeMap<VarId, SymVal>,
    cond: Bool,
    out: Vec<SymVal>,
    started: bool,
}

/// An expression's value, the condition under which evaluating it raises, and (concretely) what a callee showed.
struct Ev {
    value: SymVal,
    raises: Bool,
    out: Vec<SymVal>,
}

/// Runs the source program's graphs.
pub struct SourceRun<'p> {
    pub program: &'p SProgram,
    pub calls: &'p Calls,
    pub mode: Mode,
    pub budget: &'p Budget,
}

impl SourceRun<'_> {
    fn function(&self, name: &str) -> Result<&SFunc, Unsupported> {
        match self.program.functions.get(name) {
            Some(Ok(f)) => Ok(f),
            Some(Err(why)) => Err(Unsupported(format!("`{name}`: {}", why.0))),
            None => Err(Unsupported(format!("a call to `{name}`, which the program does not define"))),
        }
    }

    /// Every path from instruction `from` with bindings `state` to the next loop head (symbolic) or to the end.
    pub fn explore(&self, f: &SFunc, from: usize, state: State<VarId>) -> Result<Vec<Path<VarId>>, Unsupported> {
        let mut done = Vec::new();
        let mut work = vec![Walk {
            pc: from,
            env: state.env,
            cond: Bool::from_bool(true),
            out: Vec::new(),
            started: false,
        }];
        while let Some(mut w) = work.pop() {
            loop {
                if self.mode == Mode::Concrete {
                    self.budget.spend()?;
                }
                let first = !w.started;
                w.started = true;
                let stop = |w: Walk, end: End| Path {
                    cond: w.cond,
                    end,
                    out: w.out,
                    state: State { env: w.env, depth: 0 },
                };
                match &f.code[w.pc] {
                    SInstr::Assign(v, e) => {
                        let ev = self.eval(f, e, &w.env)?;
                        if let Some(raised) = self.raise(&w, &ev) {
                            done.push(raised);
                        }
                        w.cond = Bool::and(&[&w.cond, &ev.raises.not()]);
                        w.out.extend(ev.out);
                        w.env.insert(*v, self.settle(ev.value));
                        w.pc += 1;
                    }
                    SInstr::Show(e) => {
                        let ev = self.eval(f, e, &w.env)?;
                        if let Some(raised) = self.raise(&w, &ev) {
                            done.push(raised);
                        }
                        w.cond = Bool::and(&[&w.cond, &ev.raises.not()]);
                        w.out.extend(ev.out);
                        w.out.push(self.settle(ev.value));
                        w.pc += 1;
                    }
                    SInstr::Eval(e) => {
                        let ev = self.eval(f, e, &w.env)?;
                        if let Some(raised) = self.raise(&w, &ev) {
                            done.push(raised);
                        }
                        w.cond = Bool::and(&[&w.cond, &ev.raises.not()]);
                        w.out.extend(ev.out);
                        w.pc += 1;
                    }
                    SInstr::Assert(e) => {
                        let ev = self.eval(f, e, &w.env)?;
                        let fails = Bool::or(&[ev.raises.clone(), truth(&ev.value)?.not()]);
                        let failed = Ev { value: SymVal::Nothing, raises: fails.clone(), out: ev.out.clone() };
                        if let Some(raised) = self.raise(&w, &failed) {
                            done.push(raised);
                        }
                        w.cond = Bool::and(&[&w.cond, &fails.not()]);
                        w.out.extend(ev.out);
                        w.pc += 1;
                    }
                    SInstr::Branch(e, t, el) => {
                        let ev = self.eval(f, e, &w.env)?;
                        if let Some(raised) = self.raise(&w, &ev) {
                            done.push(raised);
                        }
                        let ok = Bool::and(&[&w.cond, &ev.raises.not()]);
                        w.out.extend(ev.out);
                        let c = truth(&ev.value)?;
                        let taken = Bool::and(&[&ok, &c]);
                        let not_taken = Bool::and(&[&ok, &c.not()]);
                        if !dead(&not_taken) {
                            work.push(Walk {
                                pc: *el,
                                env: w.env.clone(),
                                cond: not_taken,
                                out: w.out.clone(),
                                started: true,
                            });
                        }
                        if dead(&taken) {
                            break;
                        }
                        w.cond = taken;
                        w.pc = *t;
                    }
                    SInstr::Jump(t) => w.pc = *t,
                    SInstr::Head => {
                        if first || self.mode == Mode::Concrete {
                            w.pc += 1;
                        } else {
                            let k = f.head_of[&w.pc];
                            done.push(stop(w, End::Cut(k)));
                            break;
                        }
                    }
                    SInstr::Return(e) => {
                        let value = match e {
                            Some(e) => {
                                let ev = self.eval(f, e, &w.env)?;
                                if let Some(raised) = self.raise(&w, &ev) {
                                    done.push(raised);
                                }
                                w.cond = Bool::and(&[&w.cond, &ev.raises.not()]);
                                w.out.extend(ev.out);
                                ev.value
                            }
                            None => SymVal::Nothing,
                        };
                        if !dead(&w.cond) {
                            done.push(stop(w, End::Return(value)));
                        }
                        break;
                    }
                    SInstr::Finish => {
                        let fin = if f.main { End::Halt } else { End::Return(SymVal::Nothing) };
                        done.push(stop(w, fin));
                        break;
                    }
                }
                if self.mode == Mode::Concrete {
                    w.cond = w.cond.simplify();
                }
                if dead(&w.cond) {
                    break;
                }
            }
        }
        Ok(done)
    }

    /// A value as the run keeps it: as built when symbolic, a numeral when concrete.
    fn settle(&self, v: SymVal) -> SymVal {
        match self.mode {
            Mode::Symbolic => v,
            Mode::Concrete => v.settled(),
        }
    }

    /// The path that raises while evaluating, when it can.
    fn raise(&self, w: &Walk, ev: &Ev) -> Option<Path<VarId>> {
        let cond = Bool::and(&[&w.cond, &ev.raises]);
        if dead(&cond) {
            return None;
        }
        let mut out = w.out.clone();
        out.extend(ev.out.iter().cloned());
        Some(Path {
            cond,
            end: End::Raise,
            out,
            state: State { env: w.env.clone(), depth: 0 },
        })
    }

    fn eval(&self, f: &SFunc, e: &SExpr, env: &BTreeMap<VarId, SymVal>) -> Result<Ev, Unsupported> {
        Ok(match e {
            SExpr::Lit(l) => Ev { value: l.value(), raises: Bool::from_bool(false), out: Vec::new() },
            SExpr::Var(v) => match env.get(v) {
                Some(SymVal::Clobbered) | None => {
                    return Err(Unsupported(format!("`{}` read before it is bound", f.names[*v as usize])))
                }
                Some(val) => Ev { value: val.clone(), raises: Bool::from_bool(false), out: Vec::new() },
            },
            SExpr::Not(x) => {
                let ev = self.eval(f, x, env)?;
                Ev { value: not(&ev.value)?, raises: ev.raises, out: ev.out }
            }
            SExpr::Bin(op @ (BinaryOpKind::And | BinaryOpKind::Or), l, r) => {
                let is_and = matches!(op, BinaryOpKind::And);
                let lv = self.eval(f, l, env)?;
                let lt = truth(&lv.value)?;
                let decided = if is_and { lt.not() } else { lt.clone() };
                if self.mode == Mode::Concrete && decided.simplify().as_bool() == Some(true) {
                    return Ok(Ev { value: SymVal::Bool(Bool::from_bool(!is_and)), raises: lv.raises, out: lv.out });
                }
                let rv = self.eval(f, r, env)?;
                let rt = truth(&rv.value)?;
                let value = if is_and { Bool::and(&[&lt, &rt]) } else { Bool::or(&[&lt, &rt]) };
                let mut out = lv.out;
                out.extend(rv.out);
                Ev {
                    value: SymVal::Bool(value),
                    raises: Bool::or(&[lv.raises.clone(), Bool::and(&[lv.raises.not(), decided.not(), rv.raises])]),
                    out,
                }
            }
            SExpr::Bin(op, l, r) => {
                let lv = self.eval(f, l, env)?;
                let rv = self.eval(f, r, env)?;
                let o = binop(*op, &lv.value, &rv.value)?;
                let mut out = lv.out;
                out.extend(rv.out);
                Ev { value: o.value, raises: Bool::or(&[lv.raises, rv.raises, o.raises]), out }
            }
            SExpr::Call(name, args) => {
                let mut raises = Vec::new();
                let mut out = Vec::new();
                let mut vals = Vec::new();
                for a in args {
                    let ev = self.eval(f, a, env)?;
                    raises.push(ev.raises);
                    out.extend(ev.out);
                    vals.push(ev.value);
                }
                let (value, raised, shown) = self.call(name, vals)?;
                raises.push(raised);
                out.extend(shown);
                Ev { value, raises: Bool::or(&raises), out }
            }
        })
    }

    fn call(&self, name: &str, args: Vec<SymVal>) -> Result<(SymVal, Bool, Vec<SymVal>), Unsupported> {
        let callee = self.function(name)?;
        if callee.params.len() != args.len() {
            return Err(Unsupported(format!("`{name}` called with {} arguments", args.len())));
        }
        let mut env = BTreeMap::new();
        for ((v, kind), a) in callee.params.iter().zip(args.iter()) {
            if a.kind() != *kind {
                return Err(Unsupported(format!("`{name}` called with a {:?} for a {kind:?}", a.kind())));
            }
            env.insert(*v, a.clone());
        }
        let state = State { env, depth: 0 };
        if self.mode == Mode::Concrete {
            let (value, raised, out) = concrete_return(name, self.explore(callee, 0, state)?)?;
            return Ok((value, Bool::from_bool(raised), out));
        }
        if all_constant(&args) {
            let budget = Budget::new(CONSTANT_CALL_STEPS);
            let run = SourceRun { program: self.program, calls: self.calls, mode: Mode::Concrete, budget: &budget };
            let settled = State { env: state.env.iter().map(|(k, v)| (*k, v.clone().settled())).collect(), depth: 0 };
            if let Ok((value, raised, out)) = run.explore(callee, 0, settled).and_then(|ps| concrete_return(name, ps)) {
                return Ok((value, Bool::from_bool(raised), out));
            }
        }
        match self.calls.get(name) {
            Some(CallKind::Inline) => {
                let (value, raises) = fold_returns(name, self.explore(callee, 0, state)?)?;
                Ok((value, raises, Vec::new()))
            }
            Some(CallKind::Opaque { ret }) => {
                let (value, raises) = opaque_call(name, *ret, &args)?;
                Ok((value, raises, Vec::new()))
            }
            Some(CallKind::Refused(why)) => Err(Unsupported(format!("a call to `{name}`: {why}"))),
            None => Err(Unsupported(format!("a call to `{name}`, which has no call policy"))),
        }
    }
}
