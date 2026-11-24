//! The bytecode side: each compiled function's control-flow graph — the pcs its entry reaches, their dominators and
//! natural loops — and the symbolic execution of the VM's own ops from a cut point to the next.
//!
//! Every op is taken as the VM's handler computes it, for every value its registers can hold, including the ones the
//! compiler's own proofs exclude: a `MagicDivU` on a negative dividend computes what the reciprocal sequence computes
//! on its `u64` reading, so an oracle fact that was wrong shows up as a disagreement instead of being assumed.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use logicaffeine_compile::ast::stmt::BinaryOpKind;
use logicaffeine_compile::vm::{CompiledProgram, Constant, Op};
use z3::ast::{Ast, Bool, Int};

use super::path::{
    all_constant, concrete_return, dead, fold_returns, loop_nest, opaque_call, Budget, CallKind, Calls, End, LoopNode, Mode, Path,
    State, CONSTANT_CALL_STEPS,
};
use super::value::{binop, magic_div, not, pow2, tdiv, truth, SymVal};
use crate::symexec::Unsupported;

/// A place the VM keeps a value: a register of the frame, or the counter or last value of an open range iterator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Loc {
    Reg(u16),
    IterAt(u8),
    IterLast(u8),
}

impl Loc {
    pub fn describe(&self) -> String {
        match self {
            Loc::Reg(r) => format!("R{r}"),
            Loc::IterAt(d) => format!("iterator{d}.at"),
            Loc::IterLast(d) => format!("iterator{d}.last"),
        }
    }
}

/// A compiled function (or `## Main`) as a graph.
#[derive(Clone, Debug)]
pub struct BFunc {
    pub name: String,
    pub entry: usize,
    pub params: u16,
    pub registers: usize,
    pub loops: Vec<LoopNode>,
    pub head_of: HashMap<usize, usize>,
    pub loop_free: bool,
    pub in_out: bool,
}

/// The pcs control may go to after `pc`.
fn successors(op: &Op, pc: usize) -> Vec<usize> {
    match op {
        Op::Jump { target } => vec![*target],
        Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => vec![pc + 1, *target],
        Op::IterNext { exit, .. } => vec![pc + 1, *exit],
        Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => vec![],
        _ => vec![pc + 1],
    }
}

/// The graph of the function entered at `entry`: the pcs it reaches and its loop nest.
pub fn analyze(program: &CompiledProgram, name: &str, entry: usize, params: u16, registers: usize, in_out: bool) -> Result<BFunc, Unsupported> {
    let code = &program.code;
    let mut seen = BTreeSet::new();
    let mut stack = vec![entry];
    let mut succ: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    while let Some(pc) = stack.pop() {
        if !seen.insert(pc) {
            continue;
        }
        let op = code.get(pc).ok_or_else(|| Unsupported(format!("`{name}` runs past the end of the code")))?;
        let next = successors(op, pc);
        stack.extend(next.iter().rev().copied());
        succ.insert(pc, next);
    }
    let loops = loop_nest(entry, &succ).map_err(|why| Unsupported(format!("`{name}` {why}")))?;
    let head_of = loops.iter().enumerate().map(|(k, l)| (l.head, k)).collect();
    Ok(BFunc {
        name: name.to_string(),
        entry,
        params,
        registers,
        loop_free: loops.is_empty(),
        loops,
        head_of,
        in_out,
    })
}

/// Every compiled function by name, analyzed, and `## Main`.
pub struct BProgram<'c> {
    pub code: &'c CompiledProgram,
    pub functions: BTreeMap<String, (u16, Result<BFunc, Unsupported>)>,
    pub main: Result<BFunc, Unsupported>,
}

/// Analyze every function of `program`; `names[i]` is the name of `program.functions[i]`.
pub fn analyze_program<'c>(program: &'c CompiledProgram, names: &[String]) -> BProgram<'c> {
    let mut functions = BTreeMap::new();
    for (i, f) in program.functions.iter().enumerate() {
        let name = names[i].clone();
        let analyzed = analyze(program, &name, f.entry_pc, f.param_count, f.register_count, !f.mutable_param_regs.is_empty());
        functions.insert(name, (i as u16, analyzed));
    }
    let main = analyze(program, "## Main", 0, 0, program.register_count, false);
    BProgram { code: program, functions, main }
}

/// A bytecode run: a position, the frame's registers and open iterators, the path condition and what it showed.
struct Walk {
    pc: usize,
    env: BTreeMap<Loc, SymVal>,
    depth: u8,
    cond: Bool,
    out: Vec<SymVal>,
    started: bool,
    concrete: bool,
}

impl Walk {
    fn reg(&self, r: u16) -> Result<SymVal, Unsupported> {
        match self.env.get(&Loc::Reg(r)) {
            Some(SymVal::Clobbered) => Err(Unsupported(format!("R{r} read after a call overwrote it"))),
            Some(v) => Ok(v.clone()),
            None => Ok(SymVal::Nothing),
        }
    }

    fn set(&mut self, r: u16, v: SymVal) {
        let v = if self.concrete { v.settled() } else { v };
        self.env.insert(Loc::Reg(r), v);
    }
}

/// Runs the compiled program's graphs.
pub struct MachineRun<'p, 'c> {
    pub program: &'p BProgram<'c>,
    pub calls: &'p Calls,
    pub mode: Mode,
    pub budget: &'p Budget,
}

impl MachineRun<'_, '_> {
    fn constant(&self, idx: u32) -> Result<SymVal, Unsupported> {
        match self.program.code.constants.get(idx as usize) {
            Some(Constant::Int(n)) => Ok(SymVal::Int(Int::from_i64(*n))),
            Some(Constant::Bool(b)) => Ok(SymVal::Bool(Bool::from_bool(*b))),
            Some(Constant::Nothing) => Ok(SymVal::Nothing),
            Some(other) => Err(Unsupported(format!("the constant {other:?}"))),
            None => Err(Unsupported(format!("constant {idx} is not in the pool"))),
        }
    }

    /// The entry state of a function called with `args`: its parameters in its first registers, the rest `nothing`.
    pub fn entry_state(&self, f: &BFunc, args: &[SymVal]) -> State<Loc> {
        let mut env = BTreeMap::new();
        for r in 0..f.registers.max(args.len()) {
            let v = args.get(r).cloned().unwrap_or(SymVal::Nothing);
            env.insert(Loc::Reg(r as u16), v);
        }
        State { env, depth: 0 }
    }

    /// Every path from pc `from` with state `state` to the next loop head (symbolic) or to the end.
    pub fn explore(&self, f: &BFunc, from: usize, state: State<Loc>) -> Result<Vec<Path<Loc>>, Unsupported> {
        let mut done = Vec::new();
        let mut work = vec![Walk {
            pc: from,
            env: state.env,
            depth: state.depth,
            cond: Bool::from_bool(true),
            out: Vec::new(),
            started: false,
            concrete: self.mode == Mode::Concrete,
        }];
        while let Some(mut w) = work.pop() {
            loop {
                if self.mode == Mode::Concrete {
                    self.budget.spend()?;
                }
                let first = !w.started;
                w.started = true;
                if !first && self.mode == Mode::Symbolic {
                    if let Some(&k) = f.head_of.get(&w.pc) {
                        done.push(stop(w, End::Cut(k)));
                        break;
                    }
                }
                let op = self.program.code.code[w.pc];
                match self.step(f, &mut w, op, &mut work, &mut done)? {
                    Flow::Next => w.pc += 1,
                    Flow::Goto(t) => w.pc = t,
                    Flow::Stop(end) => {
                        if !dead(&w.cond) {
                            done.push(stop(w, end));
                        }
                        break;
                    }
                    Flow::Gone => break,
                }
                if w.concrete {
                    w.cond = w.cond.simplify();
                }
                if dead(&w.cond) {
                    break;
                }
            }
        }
        Ok(done)
    }

    /// Record the path that raises under `raises`, and continue on the one that does not.
    fn guard(&self, w: &mut Walk, raises: &Bool, done: &mut Vec<Path<Loc>>) {
        let raised = Bool::and(&[&w.cond, raises]);
        if !dead(&raised) {
            done.push(Path {
                cond: raised,
                end: End::Raise,
                out: w.out.clone(),
                state: State { env: w.env.clone(), depth: w.depth },
            });
        }
        w.cond = Bool::and(&[&w.cond, &raises.not()]);
    }

    fn binary(&self, w: &mut Walk, done: &mut Vec<Path<Loc>>, op: BinaryOpKind, dst: u16, lhs: u16, rhs: u16) -> Result<Flow, Unsupported> {
        let o = binop(op, &w.reg(lhs)?, &w.reg(rhs)?)?;
        self.guard(w, &o.raises, done);
        w.set(dst, o.value);
        Ok(Flow::Next)
    }

    fn branch(&self, w: &mut Walk, work: &mut Vec<Walk>, cond: u16, target: usize, jump_when: bool) -> Result<Flow, Unsupported> {
        let c = truth(&w.reg(cond)?)?;
        let jump = if jump_when { c } else { c.not() };
        let stay = Bool::and(&[&w.cond, &jump.not()]);
        let go = Bool::and(&[&w.cond, &jump]);
        if !dead(&go) {
            work.push(Walk {
                pc: target,
                env: w.env.clone(),
                depth: w.depth,
                cond: go,
                out: w.out.clone(),
                started: true,
                concrete: w.concrete,
            });
        }
        if dead(&stay) {
            return Ok(Flow::Gone);
        }
        w.cond = stay;
        Ok(Flow::Next)
    }

    fn step(&self, f: &BFunc, w: &mut Walk, op: Op, work: &mut Vec<Walk>, done: &mut Vec<Path<Loc>>) -> Result<Flow, Unsupported> {
        use BinaryOpKind as K;
        match op {
            Op::LoadConst { dst, idx } => {
                let v = self.constant(idx)?;
                w.set(dst, v);
            }
            Op::Move { dst, src } => {
                let v = w.reg(src)?;
                w.set(dst, v);
            }
            Op::RevSwap { a, b } => {
                let (x, y) = (w.reg(a)?, w.reg(b)?);
                w.set(a, y);
                w.set(b, x);
            }
            Op::EnsureOwned { .. } | Op::RegionBoundsGuard { .. } => {}
            Op::Add { dst, lhs, rhs } => return self.binary(w, done, K::Add, dst, lhs, rhs),
            Op::AddAssign { dst, src } => return self.binary(w, done, K::Add, dst, dst, src),
            Op::Sub { dst, lhs, rhs } => return self.binary(w, done, K::Subtract, dst, lhs, rhs),
            Op::Mul { dst, lhs, rhs } => return self.binary(w, done, K::Multiply, dst, lhs, rhs),
            Op::Div { dst, lhs, rhs } => return self.binary(w, done, K::Divide, dst, lhs, rhs),
            Op::FloorDiv { dst, lhs, rhs } => return self.binary(w, done, K::FloorDivide, dst, lhs, rhs),
            Op::Mod { dst, lhs, rhs } => return self.binary(w, done, K::Modulo, dst, lhs, rhs),
            Op::Pow { dst, lhs, rhs } => return self.binary(w, done, K::Pow, dst, lhs, rhs),
            Op::Lt { dst, lhs, rhs } => return self.binary(w, done, K::Lt, dst, lhs, rhs),
            Op::Gt { dst, lhs, rhs } => return self.binary(w, done, K::Gt, dst, lhs, rhs),
            Op::LtEq { dst, lhs, rhs } => return self.binary(w, done, K::LtEq, dst, lhs, rhs),
            Op::GtEq { dst, lhs, rhs } => return self.binary(w, done, K::GtEq, dst, lhs, rhs),
            Op::Eq { dst, lhs, rhs } => return self.binary(w, done, K::Eq, dst, lhs, rhs),
            Op::NotEq { dst, lhs, rhs } => return self.binary(w, done, K::NotEq, dst, lhs, rhs),
            Op::BitAnd { dst, lhs, rhs } => return self.binary(w, done, K::BitAnd, dst, lhs, rhs),
            Op::BitOr { dst, lhs, rhs } => return self.binary(w, done, K::BitOr, dst, lhs, rhs),
            Op::BitXor { dst, lhs, rhs } => return self.binary(w, done, K::BitXor, dst, lhs, rhs),
            Op::Shl { dst, lhs, rhs } => return self.binary(w, done, K::Shl, dst, lhs, rhs),
            Op::Shr { dst, lhs, rhs } => return self.binary(w, done, K::Shr, dst, lhs, rhs),
            Op::DivPow2 { dst, lhs, k } => {
                let x = w.reg(lhs)?.int("a `DivPow2` dividend")?;
                w.set(dst, SymVal::Int(tdiv(&x, &pow2(k as u32))));
            }
            Op::MagicDivU {
                dst,
                lhs,
                magic,
                more,
                mul_back,
                divisor,
                wrap,
            } => {
                let x = w.reg(lhs)?.int("a `MagicDivU` dividend")?;
                let o = magic_div(&x, magic, more, mul_back, divisor, wrap);
                w.set(dst, o.value);
            }
            Op::Not { dst, src } => {
                let v = not(&w.reg(src)?)?;
                w.set(dst, v);
            }
            Op::Jump { target } => return Ok(Flow::Goto(target)),
            Op::JumpIfFalse { cond, target } => return self.branch(w, work, cond, target, false),
            Op::JumpIfTrue { cond, target } => return self.branch(w, work, cond, target, true),
            Op::Show { src } => {
                let v = w.reg(src)?;
                w.out.push(v);
            }
            Op::Return { src } => return Ok(Flow::Stop(End::Return(w.reg(src)?))),
            Op::ReturnNothing => return Ok(Flow::Stop(End::Return(SymVal::Nothing))),
            Op::Halt => return Ok(Flow::Stop(End::Halt)),
            Op::FailWith { .. } => return Ok(Flow::Stop(End::Raise)),
            Op::Call {
                dst,
                func,
                args_start,
                arg_count,
            } => {
                let callee = &self.program.code.functions[func as usize];
                let args = (0..arg_count).map(|i| w.reg(args_start + i)).collect::<Result<Vec<_>, _>>()?;
                let (value, raises, shown) = self.call(func, &args)?;
                self.guard(w, &raises, done);
                w.out.extend(shown);
                let window = args_start as usize + arg_count as usize;
                let frame = (args_start as usize + callee.register_count).min(f.registers);
                for r in args_start as usize..window {
                    w.set(r as u16, SymVal::Nothing);
                }
                for r in window..frame {
                    w.set(r as u16, SymVal::Clobbered);
                }
                w.set(dst, value);
            }
            Op::NewRange { dst, start, end } => {
                let lo = w.reg(start)?.int("a range's start")?;
                let hi = w.reg(end)?.int("a range's end")?;
                w.set(dst, SymVal::Range(lo, hi));
            }
            Op::IterPrepare { iterable } => match w.reg(iterable)? {
                SymVal::Range(lo, hi) => {
                    w.env.insert(Loc::IterAt(w.depth), SymVal::Int(lo));
                    w.env.insert(Loc::IterLast(w.depth), SymVal::Int(hi));
                    w.depth += 1;
                }
                other => return Err(Unsupported(format!("an iterator over {:?}", other.kind()))),
            },
            Op::IterNext { dst, exit } => {
                let d = w.depth.checked_sub(1).ok_or_else(|| Unsupported("`IterNext` with no iterator open".into()))?;
                let at = w.env[&Loc::IterAt(d)].int("an iterator's position")?;
                let last = w.env[&Loc::IterLast(d)].int("an iterator's end")?;
                let more = at.le(&last);
                let finished = Bool::and(&[&w.cond, &more.not()]);
                if !dead(&finished) {
                    work.push(Walk {
                        pc: exit,
                        env: w.env.clone(),
                        depth: w.depth,
                        cond: finished,
                        out: w.out.clone(),
                        started: true,
                        concrete: w.concrete,
                    });
                }
                w.cond = Bool::and(&[&w.cond, &more]);
                if dead(&w.cond) {
                    return Ok(Flow::Gone);
                }
                w.set(dst, SymVal::Int(at.clone()));
                let next = SymVal::Int(at + 1i64);
                let next = if w.concrete { next.settled() } else { next };
                w.env.insert(Loc::IterAt(d), next);
            }
            Op::IterPop => {
                let d = w.depth.checked_sub(1).ok_or_else(|| Unsupported("`IterPop` with no iterator open".into()))?;
                w.env.remove(&Loc::IterAt(d));
                w.env.remove(&Loc::IterLast(d));
                w.depth = d;
            }
            other => return Err(Unsupported(format!("the op `{}`", other.name()))),
        }
        Ok(Flow::Next)
    }

    fn call(&self, func: u16, args: &[SymVal]) -> Result<(SymVal, Bool, Vec<SymVal>), Unsupported> {
        let (name, callee) = self
            .program
            .functions
            .iter()
            .find(|(_, (i, _))| *i == func)
            .map(|(n, (_, f))| (n.as_str(), f))
            .ok_or_else(|| Unsupported(format!("a call to function {func}, which has no name")))?;
        let callee = callee.as_ref().map_err(|why| Unsupported(format!("`{name}`: {}", why.0)))?;
        if callee.in_out {
            return Err(Unsupported(format!("`{name}` copies a parameter back to its caller")));
        }
        if args.len() != callee.params as usize {
            return Err(Unsupported(format!("`{name}` called with {} arguments", args.len())));
        }
        let state = self.entry_state(callee, args);
        if self.mode == Mode::Concrete {
            let (value, raised, out) = concrete_return(name, self.explore(callee, callee.entry, state)?)?;
            return Ok((value, Bool::from_bool(raised), out));
        }
        if all_constant(args) {
            let budget = Budget::new(CONSTANT_CALL_STEPS);
            let run = MachineRun { program: self.program, calls: self.calls, mode: Mode::Concrete, budget: &budget };
            let settled: Vec<SymVal> = args.iter().map(|a| a.clone().settled()).collect();
            let entry = run.entry_state(callee, &settled);
            if let Ok((value, raised, out)) = run.explore(callee, callee.entry, entry).and_then(|ps| concrete_return(name, ps)) {
                return Ok((value, Bool::from_bool(raised), out));
            }
        }
        match self.calls.get(name) {
            Some(CallKind::Inline) if callee.loop_free => {
                let (value, raises) = fold_returns(name, self.explore(callee, callee.entry, state)?)?;
                Ok((value, raises, Vec::new()))
            }
            Some(CallKind::Inline) => Err(Unsupported(format!("`{name}` is loop-free in the source but loops in the bytecode"))),
            Some(CallKind::Opaque { ret }) => {
                let (value, raises) = opaque_call(name, *ret, args)?;
                Ok((value, raises, Vec::new()))
            }
            Some(CallKind::Refused(why)) => Err(Unsupported(format!("a call to `{name}`: {why}"))),
            None => Err(Unsupported(format!("a call to `{name}`, which has no call policy"))),
        }
    }
}

/// Where control goes after an op.
enum Flow {
    Next,
    Goto(usize),
    Stop(End),
    /// The path ended inside the op (a branch whose fall-through is dead).
    Gone,
}

fn stop(w: Walk, end: End) -> Path<Loc> {
    Path {
        cond: w.cond,
        end,
        out: w.out,
        state: State { env: w.env, depth: w.depth },
    }
}
