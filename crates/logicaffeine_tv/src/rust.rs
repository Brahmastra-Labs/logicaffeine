//! Translation validation of the emitted Rust: for every input, each function `compile_to_rust` emits computes what
//! its source computes, or stops with a loud integer overflow where the source holds a value past 64 bits.
//!
//! The emitted program is parsed with `syn`, so its layout never matters, and each function is lowered to a control
//! flow graph like the source's: one loop head per `while`, `loop` and `for`, every `let` its own binding. Its
//! operations are Rust's in a release build: `+ - *` on `i64` wrap, `/` and `%` raise on a zero divisor and stop on
//! `i64::MIN / -1`, shifts mask their amount. The runtime helpers are taken as `logicaffeine_data::ops` defines them:
//! `logos_add_i64` and its kin compute exactly and stop with "Integer overflow" when the result leaves `i64`, the
//! `*_exact` helpers compute exactly, `expect_i64` stops when its value leaves `i64`, `logos_cold_path` runs its
//! closure. A stop is a raise when the source would raise too (a zero divisor) and an overflow when only the width of
//! a machine word caused it, kept in the order the code evaluates, so a raise never passes for an overflow.
//!
//! The proof is [`crate::bytecode::product`]'s, with the Rust as the target. A function whose Rust can stop on an
//! overflow is `ProvenUpToOverflow`: it never computes a wrong answer, and where it cannot hold a value it says so.

use std::collections::{BTreeMap, HashMap};

use logicaffeine_compile::ast::stmt::BinaryOpKind;
use syn::{BinOp, Expr, FnArg, Item, Lit as SynLit, Pat, ReturnType, Stmt, Type, UnOp};

/// A Rust binary operator of the fragment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ROp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

fn rop(op: &BinOp) -> Result<ROp, Unsupported> {
    Ok(match op {
        BinOp::Add(_) => ROp::Add,
        BinOp::Sub(_) => ROp::Sub,
        BinOp::Mul(_) => ROp::Mul,
        BinOp::Div(_) => ROp::Div,
        BinOp::Rem(_) => ROp::Rem,
        BinOp::BitAnd(_) => ROp::BitAnd,
        BinOp::BitOr(_) => ROp::BitOr,
        BinOp::BitXor(_) => ROp::BitXor,
        BinOp::Shl(_) => ROp::Shl,
        BinOp::Shr(_) => ROp::Shr,
        BinOp::Eq(_) => ROp::Eq,
        BinOp::Ne(_) => ROp::Ne,
        BinOp::Lt(_) => ROp::Lt,
        BinOp::Le(_) => ROp::Le,
        BinOp::Gt(_) => ROp::Gt,
        BinOp::Ge(_) => ROp::Ge,
        BinOp::And(_) => ROp::And,
        BinOp::Or(_) => ROp::Or,
        _ => return refuse("an assignment operator used as a value"),
    })
}
use z3::ast::{Ast, Bool, Int, BV};

use crate::bytecode::path::{
    all_constant, dead, loop_nest, opaque_call, Budget, CallKind, Calls, End, LoopNode, Mode, Path, State,
    CONSTANT_CALL_STEPS,
};
use crate::bytecode::product::{Finding, Prover, Side};
use crate::bytecode::source::{SFunc, SProgram, SourceRun};
use crate::bytecode::value::{binop, in_i64, tdiv, tmod, truth, wrap64, Kind, Lit, SymVal};
use crate::bytecode::{FunctionReport, SourceSide, Verdict};
use crate::symexec::Unsupported;

/// A binding in one emitted function.
pub type RVar = u32;

/// A Rust expression of the fragment, its names resolved to bindings.
#[derive(Clone, Debug)]
pub enum RExpr {
    Lit(Lit),
    Var(RVar),
    Neg(Box<RExpr>),
    Not(Box<RExpr>),
    /// A Rust operator on `i64`s or `bool`s, as a release build computes it.
    Op(ROp, Box<RExpr>, Box<RExpr>),
    /// `logos_add_i64` and its kin: exact, stopping with an overflow when the result leaves `i64`.
    Checked(BinaryOpKind, Box<RExpr>, Box<RExpr>),
    /// The `*_exact` helpers and `LogosInt`'s methods: exact.
    Exact(BinaryOpKind, Box<RExpr>, Box<RExpr>),
    /// `.expect_i64(..)`: the value, stopping with an overflow when it leaves `i64`.
    Expect(Box<RExpr>),
    /// `(lo..=hi).contains(&x)` (`inclusive`) or `(lo..hi).contains(&x)`.
    InRange(Box<RExpr>, Box<RExpr>, Box<RExpr>, bool),
    /// `x.wrapping_shl(n)` (`left`) or `.wrapping_shr(n)`.
    Shift(bool, Box<RExpr>, Box<RExpr>),
    If(Box<RExpr>, Box<RExpr>, Box<RExpr>),
    Call(String, Vec<RExpr>),
}

/// One step of an emitted function's graph.
#[derive(Clone, Debug)]
pub enum RInstr {
    Assign(RVar, RExpr),
    Show(RExpr),
    Eval(RExpr),
    Branch(RExpr, usize, usize),
    Jump(usize),
    Head,
    Return(Option<RExpr>),
    Finish,
}

/// An emitted function as a graph.
#[derive(Clone, Debug)]
pub struct RFunc {
    pub name: String,
    pub params: Vec<(RVar, Kind)>,
    /// Which parameters are machine words (`i64`, not `LogosInt`): a caller can only pass a value in `i64`.
    pub words: Vec<bool>,
    pub code: Vec<RInstr>,
    pub loops: Vec<LoopNode>,
    pub head_of: HashMap<usize, usize>,
    pub names: Vec<String>,
    pub main: bool,
}

/// Every function the emitted program defines, each lowered or refused, and its `## Main` (`_logos_main`).
pub struct RProgram {
    pub functions: BTreeMap<String, Result<RFunc, Unsupported>>,
    pub main: Result<RFunc, Unsupported>,
}

fn refuse<T>(why: impl Into<String>) -> Result<T, Unsupported> {
    Err(Unsupported(why.into()))
}

/// Parse an emitted program and lower each function.
pub fn lower_rust(text: &str) -> Result<RProgram, String> {
    let file = syn::parse_file(text).map_err(|e| format!("the emitted Rust does not parse: {e}"))?;
    let mut functions = BTreeMap::new();
    let mut main = Err(Unsupported("the emitted Rust has no `_logos_main`".into()));
    for item in &file.items {
        let Item::Fn(f) = item else { continue };
        let name = f.sig.ident.to_string();
        if name == "main" {
            continue;
        }
        let lowered = lower_fn(f);
        if name == "_logos_main" {
            main = lowered.map(|mut f| {
                f.main = true;
                f
            });
        } else {
            functions.insert(name, lowered);
        }
    }
    Ok(RProgram { functions, main })
}

fn type_kind(ty: &Type) -> Result<Kind, Unsupported> {
    match ty {
        Type::Path(p) if p.qself.is_none() => match p.path.segments.last().map(|s| s.ident.to_string()).as_deref() {
            Some("i64") | Some("LogosInt") => Ok(Kind::Int),
            Some("bool") => Ok(Kind::Bool),
            other => refuse(format!("a value of type {other:?}")),
        },
        Type::Paren(p) => type_kind(&p.elem),
        _ => refuse("a value of a compound type"),
    }
}

fn lower_fn(f: &syn::ItemFn) -> Result<RFunc, Unsupported> {
    let mut l = Lower::new(f.sig.ident.to_string());
    let mut params = Vec::new();
    let mut words = Vec::new();
    for arg in &f.sig.inputs {
        let FnArg::Typed(pt) = arg else { return refuse("a method") };
        let Pat::Ident(id) = &*pt.pat else { return refuse("a destructured parameter") };
        let kind = type_kind(&pt.ty)?;
        params.push((l.bind(&id.ident.to_string()), kind));
        words.push(is_word(&pt.ty));
    }
    if let ReturnType::Type(_, ty) = &f.sig.output {
        type_kind(ty)?;
    }
    l.body(&f.block)?;
    Ok(RFunc { words, ..l.finish(params)? })
}

/// The type is `i64`, a machine word.
fn is_word(ty: &Type) -> bool {
    match ty {
        Type::Path(p) if p.qself.is_none() => p.path.segments.last().is_some_and(|s| s.ident == "i64"),
        Type::Paren(p) => is_word(&p.elem),
        _ => false,
    }
}

struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

struct Lower {
    name: String,
    code: Vec<RInstr>,
    scopes: Vec<HashMap<String, RVar>>,
    names: Vec<String>,
    open: Vec<LoopCtx>,
}

impl Lower {
    fn new(name: String) -> Self {
        Lower { name, code: Vec::new(), scopes: vec![HashMap::new()], names: Vec::new(), open: Vec::new() }
    }

    fn finish(self, params: Vec<(RVar, Kind)>) -> Result<RFunc, Unsupported> {
        let mut succ = BTreeMap::new();
        let mut work = vec![0usize];
        while let Some(i) = work.pop() {
            if succ.contains_key(&i) {
                continue;
            }
            let next = match &self.code[i] {
                RInstr::Branch(_, t, f) => vec![*t, *f],
                RInstr::Jump(t) => vec![*t],
                RInstr::Return(_) | RInstr::Finish => vec![],
                _ => vec![i + 1],
            };
            work.extend(next.iter().copied());
            succ.insert(i, next);
        }
        let loops = loop_nest(0, &succ).map_err(|why| Unsupported(format!("`{}` {why}", self.name)))?;
        let head_of = loops.iter().enumerate().map(|(k, l)| (l.head, k)).collect();
        let words = Vec::new();
        Ok(RFunc { name: self.name, params, words, code: self.code, loops, head_of, names: self.names, main: false })
    }

    fn bind(&mut self, name: &str) -> RVar {
        self.names.push(name.to_string());
        let id = (self.names.len() - 1) as RVar;
        self.scopes.last_mut().expect("a scope is open").insert(name.to_string(), id);
        id
    }

    fn lookup(&self, name: &str) -> Result<RVar, Unsupported> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name).copied())
            .ok_or_else(|| Unsupported(format!("`{name}` names nothing this function binds")))
    }

    fn here(&self) -> usize {
        self.code.len()
    }

    fn patch(&mut self, at: usize, target: usize) {
        match &mut self.code[at] {
            RInstr::Jump(t) => *t = target,
            RInstr::Branch(_, _, f) => *f = target,
            _ => unreachable!("only jumps and branches are patched"),
        }
    }

    /// A function body: its statements, then its tail value returned.
    fn body(&mut self, block: &syn::Block) -> Result<(), Unsupported> {
        self.scopes.push(HashMap::new());
        let (stmts, tail) = split_tail(&block.stmts);
        for s in stmts {
            self.stmt(s)?;
        }
        match tail {
            Some(e) => self.returns(e)?,
            None => self.code.push(RInstr::Finish),
        }
        self.scopes.pop();
        Ok(())
    }

    /// A value the function returns: an `if` in return position returns from each branch.
    fn returns(&mut self, e: &Expr) -> Result<(), Unsupported> {
        match strip(e) {
            Expr::If(i) if i.else_branch.is_some() && block_has_statements(&i.then_branch) => {
                let c = self.expr(&i.cond)?;
                let branch = self.here();
                self.code.push(RInstr::Branch(c, branch + 1, usize::MAX));
                self.tail_block(&i.then_branch)?;
                let else_at = self.here();
                self.patch(branch, else_at);
                match &i.else_branch.as_ref().expect("checked").1.as_ref() {
                    Expr::Block(b) => self.tail_block(&b.block),
                    other => self.returns(other),
                }
            }
            Expr::Block(b) => self.tail_block(&b.block),
            other => {
                let v = self.expr(other)?;
                self.code.push(RInstr::Return(Some(v)));
                Ok(())
            }
        }
    }

    fn tail_block(&mut self, block: &syn::Block) -> Result<(), Unsupported> {
        self.scopes.push(HashMap::new());
        let (stmts, tail) = split_tail(&block.stmts);
        for s in stmts {
            self.stmt(s)?;
        }
        match tail {
            Some(e) => self.returns(e)?,
            None => self.code.push(RInstr::Finish),
        }
        self.scopes.pop();
        Ok(())
    }

    fn block(&mut self, block: &syn::Block) -> Result<(), Unsupported> {
        self.scopes.push(HashMap::new());
        for s in &block.stmts {
            self.stmt(s)?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), Unsupported> {
        match s {
            Stmt::Local(local) => {
                let init = local.init.as_ref().ok_or_else(|| Unsupported("a `let` without a value".into()))?;
                if init.diverge.is_some() {
                    return refuse("a `let … else`");
                }
                let value = self.expr(&init.expr)?;
                match &local.pat {
                    Pat::Wild(_) => self.code.push(RInstr::Eval(value)),
                    Pat::Ident(id) => {
                        let v = self.bind(&id.ident.to_string());
                        self.code.push(RInstr::Assign(v, value));
                    }
                    Pat::Type(pt) => {
                        type_kind(&pt.ty)?;
                        match &*pt.pat {
                            Pat::Ident(id) => {
                                let v = self.bind(&id.ident.to_string());
                                self.code.push(RInstr::Assign(v, value));
                            }
                            Pat::Wild(_) => self.code.push(RInstr::Eval(value)),
                            _ => return refuse("a destructuring `let`"),
                        }
                    }
                    _ => return refuse("a destructuring `let`"),
                }
                Ok(())
            }
            Stmt::Expr(e, _) => self.effect(e),
            Stmt::Item(_) => refuse("an item inside a function"),
            Stmt::Macro(_) => refuse("a macro"),
        }
    }

    /// An expression run for its effect.
    fn effect(&mut self, e: &Expr) -> Result<(), Unsupported> {
        match strip(e) {
            Expr::Assign(a) => {
                let target = self.place(&a.left)?;
                let value = self.expr(&a.right)?;
                self.code.push(RInstr::Assign(target, value));
            }
            Expr::Binary(b) if compound(&b.op).is_some() => {
                let target = self.place(&b.left)?;
                let op = compound(&b.op).expect("checked");
                let value = RExpr::Op(op, Box::new(RExpr::Var(target)), Box::new(self.expr(&b.right)?));
                self.code.push(RInstr::Assign(target, value));
            }
            Expr::If(i) => {
                let c = self.expr(&i.cond)?;
                let branch = self.here();
                self.code.push(RInstr::Branch(c, branch + 1, usize::MAX));
                self.block(&i.then_branch)?;
                match &i.else_branch {
                    Some((_, els)) => {
                        let skip = self.here();
                        self.code.push(RInstr::Jump(usize::MAX));
                        let else_at = self.here();
                        self.patch(branch, else_at);
                        self.effect(els)?;
                        let end = self.here();
                        self.patch(skip, end);
                    }
                    None => {
                        let end = self.here();
                        self.patch(branch, end);
                    }
                }
            }
            Expr::Block(b) => {
                if b.label.is_some() {
                    return refuse("a labeled block");
                }
                self.block(&b.block)?;
            }
            Expr::While(w) => {
                if w.label.is_some() {
                    return refuse("a labeled loop");
                }
                let head = self.here();
                self.code.push(RInstr::Head);
                let c = self.expr(&w.cond)?;
                let branch = self.here();
                self.code.push(RInstr::Branch(c, branch + 1, usize::MAX));
                self.open.push(LoopCtx { breaks: Vec::new(), continues: Vec::new() });
                self.block(&w.body)?;
                self.code.push(RInstr::Jump(head));
                let exit = self.here();
                self.patch(branch, exit);
                self.close(exit, head);
            }
            Expr::Loop(l) => {
                if l.label.is_some() {
                    return refuse("a labeled loop");
                }
                let head = self.here();
                self.code.push(RInstr::Head);
                self.open.push(LoopCtx { breaks: Vec::new(), continues: Vec::new() });
                self.block(&l.body)?;
                self.code.push(RInstr::Jump(head));
                let exit = self.here();
                self.close(exit, head);
            }
            Expr::ForLoop(f) => {
                if f.label.is_some() {
                    return refuse("a labeled loop");
                }
                let Expr::Range(r) = strip(&f.expr) else { return refuse("a `for` over anything but a range") };
                let (Some(lo), Some(hi)) = (&r.start, &r.end) else { return refuse("an open range") };
                let inclusive = matches!(r.limits, syn::RangeLimits::Closed(_));
                let lo = self.expr(lo)?;
                let hi = self.expr(hi)?;
                self.scopes.push(HashMap::new());
                let at = self.bind("for·at");
                let last = self.bind("for·last");
                self.code.push(RInstr::Assign(at, lo));
                self.code.push(RInstr::Assign(last, hi));
                let head = self.here();
                self.code.push(RInstr::Head);
                let more = RExpr::Op(
                    if inclusive { ROp::Le } else { ROp::Lt },
                    Box::new(RExpr::Var(at)),
                    Box::new(RExpr::Var(last)),
                );
                let branch = self.here();
                self.code.push(RInstr::Branch(more, branch + 1, usize::MAX));
                self.scopes.push(HashMap::new());
                match &*f.pat {
                    Pat::Ident(id) => {
                        let v = self.bind(&id.ident.to_string());
                        self.code.push(RInstr::Assign(v, RExpr::Var(at)));
                    }
                    Pat::Wild(_) => {}
                    _ => return refuse("a destructuring `for`"),
                }
                self.open.push(LoopCtx { breaks: Vec::new(), continues: Vec::new() });
                self.block(&f.body)?;
                let step = self.here();
                self.code.push(RInstr::Assign(
                    at,
                    RExpr::Exact(BinaryOpKind::Add, Box::new(RExpr::Var(at)), Box::new(RExpr::Lit(Lit::Int(1)))),
                ));
                self.code.push(RInstr::Jump(head));
                let exit = self.here();
                self.patch(branch, exit);
                self.close(exit, step);
                self.scopes.pop();
                self.scopes.pop();
            }
            Expr::Break(b) => {
                if b.label.is_some() || b.expr.is_some() {
                    return refuse("a labeled `break` or one with a value");
                }
                let at = self.here();
                self.code.push(RInstr::Jump(usize::MAX));
                self.open.last_mut().ok_or_else(|| Unsupported("`break` outside a loop".into()))?.breaks.push(at);
            }
            Expr::Continue(c) => {
                if c.label.is_some() {
                    return refuse("a labeled `continue`");
                }
                let at = self.here();
                self.code.push(RInstr::Jump(usize::MAX));
                self.open.last_mut().ok_or_else(|| Unsupported("`continue` outside a loop".into()))?.continues.push(at);
            }
            Expr::Return(r) => {
                let v = r.expr.as_ref().map(|e| self.expr(e)).transpose()?;
                self.code.push(RInstr::Return(v));
            }
            Expr::Call(c) if path_name(&c.func).as_deref() == Some("show") => {
                let [arg] = &c.args.iter().collect::<Vec<_>>()[..] else { return refuse("`show` of several values") };
                let v = self.expr(arg)?;
                self.code.push(RInstr::Show(v));
            }
            other => {
                let v = self.expr(other)?;
                self.code.push(RInstr::Eval(v));
            }
        }
        Ok(())
    }

    fn close(&mut self, exit: usize, continue_to: usize) {
        let ctx = self.open.pop().expect("a loop is open");
        for b in ctx.breaks {
            self.patch(b, exit);
        }
        for c in ctx.continues {
            self.patch(c, continue_to);
        }
    }

    fn place(&self, e: &Expr) -> Result<RVar, Unsupported> {
        match path_name(strip(e)) {
            Some(n) => self.lookup(&n),
            None => refuse("an assignment to something other than a variable"),
        }
    }

    fn expr(&mut self, e: &Expr) -> Result<RExpr, Unsupported> {
        Ok(match strip(e) {
            Expr::Lit(l) => match &l.lit {
                SynLit::Int(i) => {
                    let n: i128 = i.base10_parse().map_err(|_| Unsupported("an integer literal".into()))?;
                    let n = i64::try_from(n).map_err(|_| Unsupported("a literal past i64".into()))?;
                    RExpr::Lit(Lit::Int(n))
                }
                SynLit::Bool(b) => RExpr::Lit(Lit::Bool(b.value)),
                _ => return refuse("a literal that is not an integer or a Boolean"),
            },
            Expr::Path(_) => {
                let n = path_name(strip(e)).ok_or_else(|| Unsupported("a qualified path".into()))?;
                RExpr::Var(self.lookup(&n)?)
            }
            Expr::Reference(r) => self.expr(&r.expr)?,
            Expr::Unary(u) => match u.op {
                UnOp::Neg(_) => RExpr::Neg(Box::new(self.expr(&u.expr)?)),
                UnOp::Not(_) => RExpr::Not(Box::new(self.expr(&u.expr)?)),
                _ => return refuse("a dereference"),
            },
            Expr::Binary(b) => {
                if compound(&b.op).is_some() {
                    return refuse("an assignment used as a value");
                }
                RExpr::Op(rop(&b.op)?, Box::new(self.expr(&b.left)?), Box::new(self.expr(&b.right)?))
            }
            Expr::If(i) => {
                let Some((_, els)) = &i.else_branch else { return refuse("an `if` value without `else`") };
                RExpr::If(Box::new(self.expr(&i.cond)?), Box::new(self.value_block(&i.then_branch)?), Box::new(self.expr(els)?))
            }
            Expr::Block(b) => self.value_block(&b.block)?,
            Expr::Call(c) if full_path(&c.func).as_deref() == Some("std::hint::black_box") && c.args.len() == 1 => {
                self.expr(&c.args[0])?
            }
            Expr::Call(c) => {
                let name = path_name(&c.func).ok_or_else(|| Unsupported("a call through an expression".into()))?;
                let args: Vec<&Expr> = c.args.iter().collect();
                let two = |l: &mut Self| -> Result<(Box<RExpr>, Box<RExpr>), Unsupported> {
                    match args[..] {
                        [a, b] => Ok((Box::new(l.expr(a)?), Box::new(l.expr(b)?))),
                        _ => refuse(format!("`{name}` with {} arguments", args.len())),
                    }
                };
                match name.as_str() {
                    "logos_add_i64" => {
                        let (a, b) = two(self)?;
                        RExpr::Checked(BinaryOpKind::Add, a, b)
                    }
                    "logos_sub_i64" => {
                        let (a, b) = two(self)?;
                        RExpr::Checked(BinaryOpKind::Subtract, a, b)
                    }
                    "logos_mul_i64" => {
                        let (a, b) = two(self)?;
                        RExpr::Checked(BinaryOpKind::Multiply, a, b)
                    }
                    "logos_div_i64" => {
                        let (a, b) = two(self)?;
                        RExpr::Checked(BinaryOpKind::Divide, a, b)
                    }
                    "logos_rem_i64" => {
                        let (a, b) = two(self)?;
                        RExpr::Checked(BinaryOpKind::Modulo, a, b)
                    }
                    "logos_add_exact" | "logos_sub_exact" | "logos_mul_exact" | "logos_div_exact" | "logos_rem_exact"
                    | "logos_floordiv_exact" => {
                        let op = match name.as_str() {
                            "logos_add_exact" => BinaryOpKind::Add,
                            "logos_sub_exact" => BinaryOpKind::Subtract,
                            "logos_mul_exact" => BinaryOpKind::Multiply,
                            "logos_div_exact" => BinaryOpKind::Divide,
                            "logos_rem_exact" => BinaryOpKind::Modulo,
                            _ => BinaryOpKind::FloorDivide,
                        };
                        let (a, b) = two(self)?;
                        RExpr::Exact(op, a, b)
                    }
                    "logos_cold_path" => match args[..] {
                        [Expr::Closure(c)] if c.inputs.is_empty() => self.expr(&c.body)?,
                        _ => return refuse("`logos_cold_path` of anything but a closure"),
                    },
                    _ => RExpr::Call(name, args.into_iter().map(|a| self.expr(a)).collect::<Result<_, _>>()?),
                }
            }
            Expr::MethodCall(m) => {
                let method = m.method.to_string();
                let args: Vec<&Expr> = m.args.iter().collect();
                match (method.as_str(), &args[..]) {
                    ("expect_i64", [_]) => RExpr::Expect(Box::new(self.expr(&m.receiver)?)),
                    ("contains", [x]) => match strip(&m.receiver) {
                        Expr::Range(r) => {
                            let (Some(lo), Some(hi)) = (&r.start, &r.end) else { return refuse("an open range") };
                            RExpr::InRange(
                                Box::new(self.expr(lo)?),
                                Box::new(self.expr(hi)?),
                                Box::new(self.expr(x)?),
                                matches!(r.limits, syn::RangeLimits::Closed(_)),
                            )
                        }
                        _ => return refuse("`contains` on anything but a range"),
                    },
                    ("wrapping_shl" | "wrapping_shr", [n]) => {
                        let n = match strip(n) {
                            Expr::Cast(c) => &*c.expr,
                            other => other,
                        };
                        RExpr::Shift(method == "wrapping_shl", Box::new(self.expr(&m.receiver)?), Box::new(self.expr(n)?))
                    }
                    ("add" | "sub" | "mul", [x]) => {
                        let op = match method.as_str() {
                            "add" => BinaryOpKind::Add,
                            "sub" => BinaryOpKind::Subtract,
                            _ => BinaryOpKind::Multiply,
                        };
                        RExpr::Exact(op, Box::new(self.expr(&m.receiver)?), Box::new(self.expr(x)?))
                    }
                    _ => return refuse(format!("the method `{method}`")),
                }
            }
            _ => return refuse("an expression outside the integer fragment"),
        })
    }

    /// A block in value position: `let`s of plain values, then its tail.
    fn value_block(&mut self, block: &syn::Block) -> Result<RExpr, Unsupported> {
        let (stmts, tail) = split_tail(&block.stmts);
        if !stmts.is_empty() {
            return refuse("a block with statements in value position");
        }
        match tail {
            Some(t) => self.expr(t),
            None => refuse("a block without a value"),
        }
    }
}

/// An expression without the parentheses and groups around it.
fn strip(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(p) => strip(&p.expr),
        Expr::Group(g) => strip(&g.expr),
        other => other,
    }
}

/// A path without a `Self` qualifier, its segments joined by `::`: `std::hint::black_box`, the identity the emitter
/// hides a loop bound behind so the optimizer keeps the loop.
fn full_path(e: &Expr) -> Option<String> {
    match strip(e) {
        Expr::Path(p) if p.qself.is_none() => {
            Some(p.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::"))
        }
        _ => None,
    }
}

/// A path that is one plain name.
fn path_name(e: &Expr) -> Option<String> {
    match strip(e) {
        Expr::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => Some(p.path.segments[0].ident.to_string()),
        _ => None,
    }
}

/// The statements of a block and its tail value, if it ends in an expression without a semicolon.
fn split_tail(stmts: &[Stmt]) -> (&[Stmt], Option<&Expr>) {
    match stmts.split_last() {
        Some((Stmt::Expr(e, None), rest)) if !is_control(e) => (rest, Some(e)),
        _ => (stmts, None),
    }
}

/// A statement-like expression that ends a block without being its value.
fn is_control(e: &Expr) -> bool {
    match strip(e) {
        Expr::While(_) | Expr::Loop(_) | Expr::ForLoop(_) => true,
        Expr::If(i) => i.else_branch.is_none(),
        _ => false,
    }
}

fn block_has_statements(b: &syn::Block) -> bool {
    split_tail(&b.stmts).0.len() > 0 || split_tail(&b.stmts).1.is_none()
}

/// The operator a compound assignment applies.
fn compound(op: &BinOp) -> Option<ROp> {
    Some(match op {
        BinOp::AddAssign(_) => ROp::Add,
        BinOp::SubAssign(_) => ROp::Sub,
        BinOp::MulAssign(_) => ROp::Mul,
        BinOp::DivAssign(_) => ROp::Div,
        BinOp::RemAssign(_) => ROp::Rem,
        BinOp::BitAndAssign(_) => ROp::BitAnd,
        BinOp::BitOrAssign(_) => ROp::BitOr,
        BinOp::BitXorAssign(_) => ROp::BitXor,
        BinOp::ShlAssign(_) => ROp::Shl,
        BinOp::ShrAssign(_) => ROp::Shr,
        _ => return None,
    })
}

/// How an evaluation can stop: by raising, or by overflowing, each excluding the other, in the order the code runs.
#[derive(Clone)]
struct Stop {
    raise: Bool,
    overflow: Bool,
}

impl Stop {
    fn none() -> Stop {
        Stop { raise: Bool::from_bool(false), overflow: Bool::from_bool(false) }
    }

    fn any(&self) -> Bool {
        Bool::or(&[&self.raise, &self.overflow])
    }

    /// `self`, then `next` when `self` did not stop.
    fn then(&self, next: &Stop) -> Stop {
        let goes = self.any().not();
        Stop {
            raise: Bool::or(&[self.raise.clone(), Bool::and(&[&goes, &next.raise])]),
            overflow: Bool::or(&[self.overflow.clone(), Bool::and(&[&goes, &next.overflow])]),
        }
    }

    /// `self`, guarded: it can stop only where `when` holds.
    fn under(&self, when: &Bool) -> Stop {
        Stop { raise: Bool::and(&[when, &self.raise]), overflow: Bool::and(&[when, &self.overflow]) }
    }

    fn either(c: &Bool, a: &Stop, b: &Stop) -> Stop {
        Stop { raise: c.ite(&a.raise, &b.raise), overflow: c.ite(&a.overflow, &b.overflow) }
    }
}

struct Ev {
    value: SymVal,
    stop: Stop,
    out: Vec<SymVal>,
}

fn int(v: &SymVal, what: &str) -> Result<Int, Unsupported> {
    v.int(what)
}

fn word(x: &Int) -> BV {
    BV::from_int(x, 64)
}

/// One operand a constant `2^k - 1` (k < 63) and the other `x`: `x & (2^k - 1)` is `x mod 2^k` for every `i64`, in
/// two's complement, which keeps the query in integer arithmetic.
fn low_mask<'v>(a: &'v Int, b: &'v Int) -> Option<(&'v Int, i64)> {
    let mask = |v: &Int| v.simplify().as_i64().filter(|&m| m >= 0 && m < i64::MAX && (m + 1).count_ones() == 1);
    match (mask(b), mask(a)) {
        (Some(m), _) => Some((a, m)),
        (None, Some(m)) => Some((b, m)),
        _ => None,
    }
}

/// A Rust operator on two values, as a release build computes it.
fn rust_op(op: &ROp, l: &SymVal, r: &SymVal) -> Result<(SymVal, Stop), Unsupported> {
    use SymVal::{Bool as B, Int as I};
    let zero = Int::from_i64(0);
    let min = Int::from_i64(i64::MIN);
    let minus_one = Int::from_i64(-1);
    Ok(match (op, l, r) {
        (ROp::Add, I(a), I(b)) => (I(wrap64(&(a + b))), Stop::none()),
        (ROp::Sub, I(a), I(b)) => (I(wrap64(&(a - b))), Stop::none()),
        (ROp::Mul, I(a), I(b)) => (I(wrap64(&(a * b))), Stop::none()),
        (ROp::Div | ROp::Rem, I(a), I(b)) => {
            let by_zero = b.eq(&zero);
            let wide = Bool::and(&[a.eq(&min), b.eq(&minus_one)]);
            let value = if *op == ROp::Div { tdiv(a, b) } else { tmod(a, b) };
            (I(value), Stop { raise: by_zero.clone(), overflow: Bool::and(&[by_zero.not(), wide]) })
        }
        (ROp::BitAnd, I(a), I(b)) => match low_mask(a, b) {
            Some((x, m)) => (I(x.modulo(Int::from_i64(m) + 1i64)), Stop::none()),
            None => (I(Int::from_bv(&(word(a) & word(b)), true)), Stop::none()),
        },
        (ROp::BitOr, I(a), I(b)) => (I(Int::from_bv(&(word(a) | word(b)), true)), Stop::none()),
        (ROp::BitXor, I(a), I(b)) => (I(Int::from_bv(&(word(a) ^ word(b)), true)), Stop::none()),
        (ROp::Shl, I(a), I(b)) => (I(Int::from_bv(&word(a).bvshl(word(b).bvurem(BV::from_i64(64, 64))), true)), Stop::none()),
        (ROp::Shr, I(a), I(b)) => (I(Int::from_bv(&word(a).bvashr(word(b).bvurem(BV::from_i64(64, 64))), true)), Stop::none()),
        (ROp::BitAnd, B(a), B(b)) => (B(Bool::and(&[a, b])), Stop::none()),
        (ROp::BitOr, B(a), B(b)) => (B(Bool::or(&[a, b])), Stop::none()),
        (ROp::BitXor, B(a), B(b)) => (B(a.xor(b)), Stop::none()),
        (ROp::Eq, I(a), I(b)) => (B(a.eq(b)), Stop::none()),
        (ROp::Ne, I(a), I(b)) => (B(a.eq(b).not()), Stop::none()),
        (ROp::Eq, B(a), B(b)) => (B(a.eq(b)), Stop::none()),
        (ROp::Ne, B(a), B(b)) => (B(a.eq(b).not()), Stop::none()),
        (ROp::Lt, I(a), I(b)) => (B(a.lt(b)), Stop::none()),
        (ROp::Le, I(a), I(b)) => (B(a.le(b)), Stop::none()),
        (ROp::Gt, I(a), I(b)) => (B(a.gt(b)), Stop::none()),
        (ROp::Ge, I(a), I(b)) => (B(a.ge(b)), Stop::none()),
        (op, l, r) => return refuse(format!("the Rust operator {op:?} on {:?} and {:?}", l.kind(), r.kind())),
    })
}

/// A runtime helper's exact operation: the value, and how it stops. A checked helper stops with an overflow when the
/// exact result leaves `i64`; the exact helpers raise only on a zero divisor.
fn helper(op: BinaryOpKind, l: &SymVal, r: &SymVal, checked: bool) -> Result<(SymVal, Stop), Unsupported> {
    let o = binop(op, l, r)?;
    let value = o.value;
    let wide = match (&value, checked) {
        (SymVal::Int(v), true) => Bool::and(&[o.raises.not(), in_i64(v).not()]),
        _ => Bool::from_bool(false),
    };
    Ok((value, Stop { raise: o.raises, overflow: wide }))
}

/// Runs the emitted program's graphs.
pub struct RustRun<'p> {
    pub program: &'p RProgram,
    pub calls: &'p Calls,
    pub mode: Mode,
    pub budget: &'p Budget,
}

struct Walk {
    pc: usize,
    env: BTreeMap<RVar, SymVal>,
    cond: Bool,
    out: Vec<SymVal>,
    started: bool,
}

impl RustRun<'_> {
    fn settle(&self, v: SymVal) -> SymVal {
        match self.mode {
            Mode::Symbolic => v,
            Mode::Concrete => v.settled(),
        }
    }

    /// Every path from instruction `from` with bindings `state` to the next loop head (symbolic) or to the end.
    pub fn explore(&self, f: &RFunc, from: usize, state: State<RVar>) -> Result<Vec<Path<RVar>>, Unsupported> {
        let mut done = Vec::new();
        let mut work = vec![Walk { pc: from, env: state.env, cond: Bool::from_bool(true), out: Vec::new(), started: false }];
        while let Some(mut w) = work.pop() {
            loop {
                if self.mode == Mode::Concrete {
                    self.budget.spend()?;
                }
                let first = !w.started;
                w.started = true;
                let stop = |w: Walk, end: End| Path { cond: w.cond, end, out: w.out, state: State { env: w.env, depth: 0 } };
                match &f.code[w.pc] {
                    RInstr::Assign(v, e) => {
                        let ev = self.eval(f, e, &w.env)?;
                        self.stops(&mut w, &ev, &mut done);
                        w.env.insert(*v, self.settle(ev.value));
                        w.pc += 1;
                    }
                    RInstr::Show(e) => {
                        let ev = self.eval(f, e, &w.env)?;
                        self.stops(&mut w, &ev, &mut done);
                        w.out.push(self.settle(ev.value));
                        w.pc += 1;
                    }
                    RInstr::Eval(e) => {
                        let ev = self.eval(f, e, &w.env)?;
                        self.stops(&mut w, &ev, &mut done);
                        w.pc += 1;
                    }
                    RInstr::Branch(e, t, el) => {
                        let ev = self.eval(f, e, &w.env)?;
                        self.stops(&mut w, &ev, &mut done);
                        let c = truth(&ev.value)?;
                        let taken = Bool::and(&[&w.cond, &c]);
                        let not_taken = Bool::and(&[&w.cond, &c.not()]);
                        if !dead(&not_taken) {
                            work.push(Walk { pc: *el, env: w.env.clone(), cond: not_taken, out: w.out.clone(), started: true });
                        }
                        if dead(&taken) {
                            break;
                        }
                        w.cond = taken;
                        w.pc = *t;
                    }
                    RInstr::Jump(t) => w.pc = *t,
                    RInstr::Head => {
                        if first || self.mode == Mode::Concrete {
                            w.pc += 1;
                        } else {
                            let k = f.head_of[&w.pc];
                            done.push(stop(w, End::Cut(k)));
                            break;
                        }
                    }
                    RInstr::Return(e) => {
                        let value = match e {
                            Some(e) => {
                                let ev = self.eval(f, e, &w.env)?;
                                self.stops(&mut w, &ev, &mut done);
                                self.settle(ev.value)
                            }
                            None => SymVal::Nothing,
                        };
                        if !dead(&w.cond) {
                            done.push(stop(w, End::Return(value)));
                        }
                        break;
                    }
                    RInstr::Finish => {
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

    /// Record the paths on which evaluating stopped, and continue on the one that did not.
    fn stops(&self, w: &mut Walk, ev: &Ev, done: &mut Vec<Path<RVar>>) {
        let mut out = w.out.clone();
        out.extend(ev.out.iter().cloned());
        for (c, end) in [(&ev.stop.raise, End::Raise), (&ev.stop.overflow, End::Overflow)] {
            let cond = Bool::and(&[&w.cond, c]);
            if !dead(&cond) {
                done.push(Path { cond, end, out: out.clone(), state: State { env: w.env.clone(), depth: 0 } });
            }
        }
        w.cond = Bool::and(&[&w.cond, &ev.stop.any().not()]);
        w.out.extend(ev.out.iter().cloned());
    }

    fn eval(&self, f: &RFunc, e: &RExpr, env: &BTreeMap<RVar, SymVal>) -> Result<Ev, Unsupported> {
        let pure = |value: SymVal| Ev { value, stop: Stop::none(), out: Vec::new() };
        Ok(match e {
            RExpr::Lit(l) => pure(l.value()),
            RExpr::Var(v) => match env.get(v) {
                Some(SymVal::Clobbered) | None => {
                    return refuse(format!("`{}` read before it is bound", f.names[*v as usize]))
                }
                Some(val) => pure(val.clone()),
            },
            RExpr::Neg(x) => {
                let ev = self.eval(f, x, env)?;
                Ev { value: SymVal::Int(wrap64(&-int(&ev.value, "a negated value")?)), ..ev }
            }
            RExpr::Not(x) => {
                let ev = self.eval(f, x, env)?;
                let value = match &ev.value {
                    SymVal::Bool(b) => SymVal::Bool(b.not()),
                    SymVal::Int(i) => SymVal::Int(-i - 1i64),
                    other => return refuse(format!("`!` on {:?}", other.kind())),
                };
                Ev { value, ..ev }
            }
            RExpr::Op(op @ (ROp::And | ROp::Or), l, r) => {
                let is_and = *op == ROp::And;
                let lv = self.eval(f, l, env)?;
                let lt = truth(&lv.value)?;
                let decided = if is_and { lt.not() } else { lt.clone() };
                if self.mode == Mode::Concrete && decided.simplify().as_bool() == Some(true) {
                    return Ok(Ev { value: SymVal::Bool(Bool::from_bool(!is_and)), ..lv });
                }
                let rv = self.eval(f, r, env)?;
                let rt = truth(&rv.value)?;
                let value = if is_and { Bool::and(&[&lt, &rt]) } else { Bool::or(&[&lt, &rt]) };
                let mut out = lv.out;
                out.extend(rv.out);
                Ev { value: SymVal::Bool(value), stop: lv.stop.then(&rv.stop.under(&decided.not())), out }
            }
            RExpr::Op(op, l, r) => {
                let (lv, rv) = (self.eval(f, l, env)?, self.eval(f, r, env)?);
                let (value, stop) = rust_op(op, &lv.value, &rv.value)?;
                self.join(lv, rv, value, stop)
            }
            RExpr::Checked(op, l, r) | RExpr::Exact(op, l, r) => {
                let (lv, rv) = (self.eval(f, l, env)?, self.eval(f, r, env)?);
                let (value, stop) = helper(*op, &lv.value, &rv.value, matches!(e, RExpr::Checked(..)))?;
                self.join(lv, rv, value, stop)
            }
            RExpr::Expect(x) => {
                let ev = self.eval(f, x, env)?;
                let v = int(&ev.value, "an `expect_i64` value")?;
                let wide = Stop { raise: Bool::from_bool(false), overflow: in_i64(&v).not() };
                Ev { value: ev.value.clone(), stop: ev.stop.then(&wide), out: ev.out }
            }
            RExpr::InRange(lo, hi, x, inclusive) => {
                let (a, b, c) = (self.eval(f, lo, env)?, self.eval(f, hi, env)?, self.eval(f, x, env)?);
                let (lo, hi, x) = (int(&a.value, "a range's start")?, int(&b.value, "a range's end")?, int(&c.value, "a value")?);
                let upper = if *inclusive { x.le(&hi) } else { x.lt(&hi) };
                let mut out = a.out;
                out.extend(b.out);
                out.extend(c.out);
                Ev { value: SymVal::Bool(Bool::and(&[x.ge(&lo), upper])), stop: a.stop.then(&b.stop).then(&c.stop), out }
            }
            RExpr::Shift(left, x, n) => {
                let (xv, nv) = (self.eval(f, x, env)?, self.eval(f, n, env)?);
                let op = if *left { ROp::Shl } else { ROp::Shr };
                let (value, stop) = rust_op(&op, &xv.value, &nv.value)?;
                self.join(xv, nv, value, stop)
            }
            RExpr::If(c, a, b) => {
                let cv = self.eval(f, c, env)?;
                let ct = truth(&cv.value)?;
                let (av, bv) = (self.eval(f, a, env)?, self.eval(f, b, env)?);
                if !av.out.is_empty() || !bv.out.is_empty() {
                    return refuse("a value branch that shows");
                }
                let value = match (&av.value, &bv.value) {
                    (SymVal::Int(x), SymVal::Int(y)) => SymVal::Int(ct.ite(x, y)),
                    (SymVal::Bool(x), SymVal::Bool(y)) => SymVal::Bool(ct.ite(x, y)),
                    (x, y) => return refuse(format!("an `if` of {:?} and {:?}", x.kind(), y.kind())),
                };
                Ev { value, stop: cv.stop.then(&Stop::either(&ct, &av.stop, &bv.stop)), out: cv.out }
            }
            RExpr::Call(name, args) => {
                let mut stop = Stop::none();
                let mut out = Vec::new();
                let mut vals = Vec::new();
                for a in args {
                    let ev = self.eval(f, a, env)?;
                    stop = stop.then(&ev.stop);
                    out.extend(ev.out);
                    vals.push(ev.value);
                }
                let (value, call_stop, shown) = self.call(name, vals)?;
                out.extend(shown);
                Ev { value, stop: stop.then(&call_stop), out }
            }
        })
    }

    fn join(&self, l: Ev, r: Ev, value: SymVal, stop: Stop) -> Ev {
        let mut out = l.out;
        out.extend(r.out);
        Ev { value, stop: l.stop.then(&r.stop).then(&stop), out }
    }

    fn call(&self, name: &str, args: Vec<SymVal>) -> Result<(SymVal, Stop, Vec<SymVal>), Unsupported> {
        let callee = match self.program.functions.get(name) {
            Some(Ok(f)) => f,
            Some(Err(why)) => return refuse(format!("`{name}`: {}", why.0)),
            None => return refuse(format!("a call to `{name}`, which the emitted Rust does not define")),
        };
        if callee.params.len() != args.len() {
            return refuse(format!("`{name}` called with {} arguments", args.len()));
        }
        let state = State { env: callee.params.iter().map(|(v, _)| *v).zip(args.iter().cloned()).collect(), depth: 0 };
        let concrete = |budget: &Budget| -> Result<(SymVal, Stop, Vec<SymVal>), Unsupported> {
            let run = RustRun { program: self.program, calls: self.calls, mode: Mode::Concrete, budget };
            let settled = State { env: state.env.iter().map(|(k, v)| (*k, v.clone().settled())).collect(), depth: 0 };
            let paths = run.explore(callee, 0, settled)?;
            let mut live = paths.into_iter().filter(|p| !dead(&p.cond));
            let (Some(p), None) = (live.next(), live.next()) else {
                return refuse(format!("a concrete call to `{name}` did not take exactly one path"));
            };
            let t = Bool::from_bool(true);
            let f = Bool::from_bool(false);
            match p.end {
                End::Return(v) => Ok((v, Stop::none(), p.out)),
                End::Raise => Ok((SymVal::Nothing, Stop { raise: t, overflow: f }, p.out)),
                End::Overflow => Ok((SymVal::Nothing, Stop { raise: f, overflow: t }, p.out)),
                _ => refuse(format!("a concrete call to `{name}` ended outside a return")),
            }
        };
        if self.mode == Mode::Concrete {
            return concrete(self.budget);
        }
        if all_constant(&args) {
            if let Ok(r) = concrete(&Budget::new(CONSTANT_CALL_STEPS)) {
                return Ok(r);
            }
        }
        match self.calls.get(name) {
            Some(CallKind::Inline) if callee.loops.is_empty() => {
                let mut raises = Vec::new();
                let mut overflows = Vec::new();
                let mut returns: Vec<(Bool, SymVal)> = Vec::new();
                for p in self.explore(callee, 0, state)? {
                    if !p.out.is_empty() {
                        return refuse(format!("`{name}` shows a value while run in place"));
                    }
                    match p.end {
                        End::Raise => raises.push(p.cond),
                        End::Overflow => overflows.push(p.cond),
                        End::Return(v) => returns.push((p.cond, v)),
                        _ => return refuse(format!("`{name}` reached a loop head while run in place")),
                    }
                }
                let stop = Stop { raise: Bool::or(&raises), overflow: Bool::or(&overflows) };
                let Some((_, last)) = returns.pop() else { return Ok((SymVal::Nothing, stop, Vec::new())) };
                let mut value = last;
                while let Some((c, v)) = returns.pop() {
                    value = match (&v, &value) {
                        (SymVal::Int(a), SymVal::Int(b)) => SymVal::Int(c.ite(a, b)),
                        (SymVal::Bool(a), SymVal::Bool(b)) => SymVal::Bool(c.ite(a, b)),
                        (SymVal::Nothing, SymVal::Nothing) => SymVal::Nothing,
                        _ => return refuse(format!("`{name}` returns values of two kinds")),
                    };
                }
                Ok((value, stop, Vec::new()))
            }
            Some(CallKind::Inline) => refuse(format!("`{name}` is loop-free in the source but loops in the Rust")),
            Some(CallKind::Opaque { ret }) => {
                let (value, raises) = opaque_call(name, *ret, &args)?;
                let (overflows, _) = opaque_call(&format!("{name}·overflows"), Some(Kind::Bool), &args)?;
                let SymVal::Bool(overflows) = overflows else { unreachable!("a Bool-valued function") };
                Ok((value, Stop { raise: raises, overflow: overflows }.ordered(), Vec::new()))
            }
            Some(CallKind::Refused(why)) => refuse(format!("a call to `{name}`: {why}")),
            None => refuse(format!("a call to `{name}`, which has no call policy")),
        }
    }
}

impl Stop {
    /// The two conditions made exclusive, a raise first.
    fn ordered(self) -> Stop {
        Stop { overflow: Bool::and(&[self.raise.not(), self.overflow]), raise: self.raise }
    }
}

/// The Rust side of a proof: one emitted function.
pub struct RustSide<'a, 'p> {
    pub run: &'a RustRun<'p>,
    pub f: &'a RFunc,
}

impl Side for RustSide<'_, '_> {
    type Loc = RVar;

    fn loops(&self) -> &[LoopNode] {
        &self.f.loops
    }

    fn entry(&self, params: &[SymVal]) -> State<RVar> {
        State { env: self.f.params.iter().zip(params).map(|((v, _), p)| (*v, p.clone())).collect(), depth: 0 }
    }

    fn assumes(&self, params: &[SymVal]) -> Bool {
        let fits: Vec<Bool> = self
            .f
            .words
            .iter()
            .zip(params)
            .filter_map(|(word, p)| match (word, p) {
                (true, SymVal::Int(i)) => Some(in_i64(i)),
                _ => None,
            })
            .collect();
        Bool::and(&fits)
    }

    fn explore(&self, from: Option<usize>, state: State<RVar>) -> Result<Vec<Path<RVar>>, Unsupported> {
        self.run.explore(self.f, from.map_or(0, |k| self.f.loops[k].head), state)
    }

    fn describe(&self, l: &RVar) -> String {
        format!("rust.{}", self.f.names[*l as usize])
    }
}

/// Every function's verdict against the emitted Rust, `## Main` first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustReport {
    pub functions: Vec<FunctionReport>,
}

impl RustReport {
    pub fn verdict(&self, name: &str) -> Option<&Verdict> {
        self.functions.iter().find(|f| f.name == name).map(|f| &f.verdict)
    }
}

/// Validate every function of `source` and `## Main` against the Rust `compile_to_rust` emits for it.
pub fn validate(source: &str) -> Result<RustReport, String> {
    validate_edited(source, |_| {})
}

/// [`validate`] against the Rust `edit` leaves of the emitted program — a seeded miscompile, which the validator must
/// refute. The proofs run in a Z3 context of their own, as `bytecode::validate_edited`'s do.
pub fn validate_edited(source: &str, edit: impl FnOnce(&mut String)) -> Result<RustReport, String> {
    let sp = crate::bytecode::lower_source(source)?;
    let mut text = logicaffeine_compile::compile::compile_to_rust(source).map_err(|e| format!("{e:?}"))?;
    edit(&mut text);
    let rp = lower_rust(&text)?;
    let calls = crate::bytecode::call_policy(&sp);
    let functions = z3::with_z3_config(&z3::Config::new(), || {
        let budget = Budget::new(0);
        let src = SourceRun { program: &sp, calls: &calls, mode: Mode::Symbolic, budget: &budget };
        let run = RustRun { program: &rp, calls: &calls, mode: Mode::Symbolic, budget: &budget };
        let mut functions = vec![FunctionReport {
            name: "## Main".into(),
            verdict: prove_one(&sp, &rp, &src, &run, &calls, sp.main.as_ref(), None, rp.main.as_ref()),
        }];
        for (name, sf) in &sp.functions {
            let verdict = match rp.functions.get(name) {
                Some(rf) => prove_one(&sp, &rp, &src, &run, &calls, sf.as_ref(), sp.tail.get(name), rf.as_ref()),
                None => Verdict::Unsupported { reason: "the emitted Rust has no such function".into() },
            };
            functions.push(FunctionReport { name: name.clone(), verdict });
        }
        functions
    });
    Ok(RustReport { functions })
}

#[allow(clippy::too_many_arguments)]
fn prove_one(
    sp: &SProgram,
    rp: &RProgram,
    src: &SourceRun,
    run: &RustRun,
    calls: &Calls,
    sf: Result<&SFunc, &Unsupported>,
    looped: Option<&SFunc>,
    rf: Result<&RFunc, &Unsupported>,
) -> Verdict {
    let (sf, rf) = match (sf, rf) {
        (Ok(s), Ok(r)) => (s, r),
        (Err(why), _) | (_, Err(why)) => return Verdict::Unsupported { reason: why.0.clone() },
    };
    let nest = |ls: &[LoopNode]| ls.iter().map(|l| l.parent).collect::<Vec<_>>();
    let sf = match looped {
        Some(l) if nest(&sf.loops) != nest(&rf.loops) && nest(&l.loops) == nest(&rf.loops) => l,
        _ => sf,
    };
    let params: Vec<SymVal> = sf
        .params
        .iter()
        .map(|(v, k)| crate::bytecode::path::fresh(*k, &format!("{}.{}", sf.name, sf.names[*v as usize])))
        .collect();
    let assumed: Vec<String> =
        sf.calls.iter().filter(|c| matches!(calls.get(*c), Some(CallKind::Opaque { .. }))).cloned().collect();
    let (source, target) = (SourceSide { run: src, f: sf }, RustSide { run, f: rf });
    let prover = Prover { source: &source, target: &target, what: "the Rust" };
    match prover.prove(&params, assumed) {
        Ok(Finding::Proven { invariants, assumed, overflow: false }) => Verdict::Proven { invariants, assumed },
        Ok(Finding::Proven { invariants, assumed, overflow: true }) => Verdict::ProvenUpToOverflow { invariants, assumed },
        Ok(Finding::Unknown { reason }) => Verdict::Unknown { reason },
        Ok(Finding::Failed { at, state, values }) => match replay(sp, rp, calls, sf, rf, &values) {
            Some((input, source, rust)) => Verdict::Refuted { input, source, bytecode: rust, at },
            None => Verdict::Unproven { at, state },
        },
        Err(why) => Verdict::Unsupported { reason: why.0 },
    }
}

/// The first input on which the source and the Rust, run concretely, part ways: different values or shows, or the
/// Rust raising or ending where the source does not. The Rust stopping on an overflow is never a counterexample.
fn replay(
    sp: &SProgram,
    rp: &RProgram,
    calls: &Calls,
    sf: &SFunc,
    rf: &RFunc,
    hints: &[String],
) -> Option<(String, String, String)> {
    let inputs = crate::bytecode::replay_inputs(sf, hints);
    let mut spent = 0u64;
    for args in inputs {
        if spent >= crate::bytecode::REPLAY_TOTAL {
            break;
        }
        let budget = Budget::new(crate::bytecode::REPLAY_STEPS);
        let src = SourceRun { program: sp, calls, mode: Mode::Concrete, budget: &budget };
        let entry = State { env: sf.params.iter().map(|(v, _)| *v).zip(args.iter().cloned()).collect(), depth: 0 };
        let source = src.explore(sf, 0, entry).ok().and_then(crate::bytecode::outcome);
        spent += crate::bytecode::REPLAY_STEPS - budget.left();
        let budget = Budget::new(crate::bytecode::REPLAY_STEPS);
        let run = RustRun { program: rp, calls, mode: Mode::Concrete, budget: &budget };
        let entry = State { env: rf.params.iter().map(|(v, _)| *v).zip(args.iter().cloned()).collect(), depth: 0 };
        let rust = run.explore(rf, 0, entry).ok().and_then(crate::bytecode::outcome);
        spent += crate::bytecode::REPLAY_STEPS - budget.left();
        if let (Some(s), Some(r)) = (source, rust) {
            if s != r && !r.ends_with("stops with an integer overflow") {
                let input = sf
                    .params
                    .iter()
                    .zip(&args)
                    .map(|((v, _), a)| format!("{} = {}", sf.names[*v as usize], a.shown().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some((input, s, r));
            }
        }
    }
    None
}

/// Run the Rust executor concretely on `## Main` and hold it to the tree-walker: what it shows, unless it stops on an
/// overflow, must be what the tree-walker shows.
pub fn check_executor(source: &str) -> crate::bytecode::ExecutorCheck {
    use crate::bytecode::ExecutorCheck;
    let sp = match crate::bytecode::lower_source(source) {
        Ok(sp) => sp,
        Err(e) => return ExecutorCheck::Unsupported(e),
    };
    let text = match logicaffeine_compile::compile::compile_to_rust(source) {
        Ok(t) => t,
        Err(e) => return ExecutorCheck::Unsupported(format!("{e:?}")),
    };
    let rp = match lower_rust(&text) {
        Ok(rp) => rp,
        Err(e) => return ExecutorCheck::Unsupported(e),
    };
    let calls = crate::bytecode::call_policy(&sp);
    let budget = Budget::new(2_000_000);
    let run = RustRun { program: &rp, calls: &calls, mode: Mode::Concrete, budget: &budget };
    let main = match &rp.main {
        Ok(m) => m,
        Err(why) => return ExecutorCheck::Unsupported(why.0.clone()),
    };
    let paths = match run.explore(main, 0, State { env: BTreeMap::new(), depth: 0 }) {
        Ok(p) => p,
        Err(why) => return ExecutorCheck::Unsupported(why.0),
    };
    let Some(outcome) = crate::bytecode::outcome(paths) else {
        return ExecutorCheck::Unsupported("the Rust executor did not take one path".into());
    };
    let walker = logicaffeine_compile::compile::interpret_program(source);
    let expected = match walker {
        Ok(out) if out.is_empty() => "finishes".to_string(),
        Ok(out) => format!("shows {} and finishes", out.lines().collect::<Vec<_>>().join(", ")),
        Err(_) => return ExecutorCheck::Unsupported("the tree-walker raised".into()),
    };
    if outcome == expected || outcome.ends_with("stops with an integer overflow") {
        ExecutorCheck::Agree
    } else {
        ExecutorCheck::Disagree(format!("the Rust executor {outcome}; the tree-walker {expected}"))
    }
}
