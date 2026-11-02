//! Regions: the functions an accelerator runs, as control-flow graphs over checked cells.
//!
//! A region is lowered from a function of the SAME optimized `CompiledProgram` the host is lowered from, so
//! its function index is the host's. The lowering is typed: every register's kind (`Int`, `Float`, `Bool`)
//! is inferred by a forward dataflow over the function's blocks from its declared parameter kinds, and each
//! op becomes the cell whose checked semantics is the VM kernel's for those kinds. A function the lowering
//! cannot type, or that does anything besides compute a scalar from scalars (a `Show`, a global, a
//! collection, a call, a closure), stays on the host with the reason named.
//!
//! A region is executed two ways: [`RegionCfg::eval_checked`] runs the cells' 64-bit checked semantics (the
//! reference), and [`RegionCfg::eval_on_lanes`] runs every cell as its lane routine on a device's lanes (the
//! device model). Control flow is real, not predicated: only the cells on the path the VM takes execute, so
//! a divergence is reported exactly where the VM would promote or fail.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::codegen_sva::fabric_rtl::{AluOp, CmpKind, Divergence, FuncUnit};
use crate::semantics::builtins::BuiltinId;
use crate::vm::{CompiledProgram, Constant, Op, ParamKind, SlotKind};

use super::cells::{routine, CellOp};
use super::lane::{join_u64, run, split_u64, LaneCounts, Lanes};

/// A region register: one 64-bit value (an `Int`, a `Bool` as 0/1, a `Float` as its bit pattern).
pub type VReg = u16;

/// The value kinds a region carries across its boundary and between its cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Int,
    Float,
    Bool,
}

impl Kind {
    /// The native-boundary slot this kind rides in.
    pub fn slot(self) -> SlotKind {
        match self {
            Kind::Int => SlotKind::Int,
            Kind::Float => SlotKind::Float,
            Kind::Bool => SlotKind::Bool,
        }
    }
}

/// One statement of a region block.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RStmt {
    /// `dst = value`.
    Const { dst: VReg, value: i64 },
    /// `dst = src`.
    Copy { dst: VReg, src: VReg },
    /// `dst = op(a, b)` under the op's checked semantics; a divergence ends the region.
    Cell { dst: VReg, op: CellOp, a: VReg, b: VReg },
    /// `dst = cmp(a, b) ? t : e` over integers.
    Select { dst: VReg, cmp: CmpKind, a: VReg, b: VReg, t: VReg, e: VReg },
}

/// Where a block goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RTerm {
    Jump(u32),
    /// `cond != 0 ? then_ : else_`.
    Branch { cond: VReg, then_: u32, else_: u32 },
    Return(VReg),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RBlock {
    pub stmts: Vec<RStmt>,
    pub term: RTerm,
}

/// A function as a region: its ports, its register file size, its blocks (block 0 is the entry).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RegionCfg {
    pub name: String,
    pub func: u16,
    pub params: Vec<Kind>,
    pub ret: Kind,
    pub regs: u16,
    pub blocks: Vec<RBlock>,
}

/// Why a region run stopped short of a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Halt {
    /// A cell diverged from the VM: the call goes back to the host. The op and its operands are the evidence.
    Declined { divergence: Divergence, op: CellOp, a: i64, b: i64 },
    /// The run exceeded the block budget it was given.
    OutOfFuel,
}

/// A function the region lowering keeps on the host, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostReason {
    pub func: u16,
    pub name: String,
    pub reason: String,
}

/// What discovery made of one function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Discovered {
    Region(RegionCfg),
    Host(HostReason),
}

/// A register's inferred kind at a program point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ak {
    Int,
    Float,
    Bool,
    /// A range built by the `NewRange` at this pc.
    Range(usize),
    /// Different kinds on different paths.
    Conflict,
}

impl Ak {
    fn of(k: Kind) -> Ak {
        match k {
            Kind::Int => Ak::Int,
            Kind::Float => Ak::Float,
            Kind::Bool => Ak::Bool,
        }
    }
    fn scalar(self) -> Option<Kind> {
        match self {
            Ak::Int => Some(Kind::Int),
            Ak::Float => Some(Kind::Float),
            Ak::Bool => Some(Kind::Bool),
            _ => None,
        }
    }
}

/// The dataflow state at a block boundary: each VM register's kind (absent = no path defines it) and the
/// stack of live range iterators (by the pc of the `IterPrepare` that pushed each).
#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    kinds: BTreeMap<u16, Ak>,
    iters: Vec<usize>,
}

fn join(a: &State, b: &State) -> Result<State, String> {
    if a.iters != b.iters {
        return Err("paths reach one point with different live loops".to_string());
    }
    let mut kinds = a.kinds.clone();
    for (r, k) in &b.kinds {
        kinds
            .entry(*r)
            .and_modify(|cur| {
                if *cur != *k {
                    *cur = Ak::Conflict
                }
            })
            .or_insert(*k);
    }
    // A register only one path defines is undefined on the other: absent, not its kind.
    let both: BTreeSet<u16> = a.kinds.keys().filter(|r| b.kinds.contains_key(r)).copied().collect();
    kinds.retain(|r, _| both.contains(r));
    Ok(State { kinds, iters: a.iters.clone() })
}

/// The pcs of `[entry, end)` that start a block.
fn leaders(code: &[Op], entry: usize, end: usize) -> Result<BTreeSet<usize>, String> {
    let mut set = BTreeSet::new();
    set.insert(entry);
    for pc in entry..end {
        let within = |t: usize| -> Result<usize, String> {
            if t < entry || t >= end {
                Err(format!("a jump at pc {pc} leaves the function (to {t})"))
            } else {
                Ok(t)
            }
        };
        match code[pc] {
            Op::Jump { target } => {
                set.insert(within(target)?);
                set.insert(pc + 1);
            }
            Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
                set.insert(within(target)?);
                set.insert(pc + 1);
            }
            Op::IterNext { exit, .. } => {
                set.insert(within(exit)?);
                set.insert(pc + 1);
            }
            Op::Return { .. } | Op::ReturnNothing => {
                set.insert(pc + 1);
            }
            _ => {}
        }
    }
    set.retain(|p| *p < end);
    Ok(set)
}

/// The kinds of a function's parameters, or why one is not a region port.
fn param_kinds(program: &CompiledProgram, fi: usize) -> Result<Vec<Kind>, String> {
    let f = &program.functions[fi];
    (0..f.param_count as usize)
        .map(|i| match f.param_kinds.get(i).copied().flatten() {
            Some(ParamKind::Scalar(SlotKind::Int)) => Ok(Kind::Int),
            Some(ParamKind::Scalar(SlotKind::Float)) => Ok(Kind::Float),
            Some(ParamKind::Scalar(SlotKind::Bool)) => Ok(Kind::Bool),
            other => Err(format!("parameter {} is not an Int, Float or Bool ({other:?})", i + 1)),
        })
        .collect()
}

/// The code range a function occupies: its entry up to the next function's entry, or the end of the code.
fn function_range(program: &CompiledProgram, fi: usize) -> (usize, usize) {
    let entry = program.functions[fi].entry_pc;
    let end = program.functions.iter().map(|g| g.entry_pc).filter(|&e| e > entry).min().unwrap_or(program.code.len());
    (entry, end)
}

/// Lower function `fi` of `program` to a region, or name why it stays on the host.
pub fn lower(program: &CompiledProgram, fi: usize, name: &str) -> Result<RegionCfg, HostReason> {
    let host = |reason: String| HostReason { func: fi as u16, name: name.to_string(), reason };
    let f = &program.functions[fi];
    if !f.captures.is_empty() {
        return Err(host("a closure body (it captures variables)".to_string()));
    }
    if program.code.iter().any(|op| matches!(op, Op::MakeClosure { func, .. } if *func as usize == fi)) {
        return Err(host("a function the program also uses as a closure value".to_string()));
    }
    let params = param_kinds(program, fi).map_err(host)?;
    let (entry, end) = function_range(program, fi);
    let code = &program.code;
    let starts = leaders(code, entry, end).map_err(host)?;
    let starts: Vec<usize> = starts.into_iter().collect();
    let block_of: BTreeMap<usize, usize> = starts.iter().enumerate().map(|(i, p)| (*p, i)).collect();
    let block_end = |i: usize| starts.get(i + 1).copied().unwrap_or(end);
    // ── Kinds: a forward dataflow to a fixpoint. ──
    let mut entry_state: Vec<Option<State>> = vec![None; starts.len()];
    entry_state[0] = Some(State {
        kinds: params.iter().enumerate().map(|(i, k)| (i as u16, Ak::of(*k))).collect(),
        iters: Vec::new(),
    });
    let mut work: Vec<usize> = vec![0];
    let mut guard = 0usize;
    while let Some(bi) = work.pop() {
        guard += 1;
        if guard > 100_000 {
            return Err(host("the kind analysis did not settle".to_string()));
        }
        let mut st = entry_state[bi].clone().expect("a reached block has a state");
        let mut succ: Vec<usize> = Vec::new();
        let mut falls = true;
        for pc in starts[bi]..block_end(bi) {
            match transfer(program, pc, &mut st).map_err(host)? {
                Flow::Next => {}
                Flow::Jump(t) => {
                    succ.push(t);
                    falls = false;
                }
                Flow::Branch(t) => succ.push(t),
                Flow::End => falls = false,
            }
        }
        if falls && block_end(bi) < end {
            succ.push(block_end(bi));
        }
        for t in succ {
            let ti = *block_of.get(&t).ok_or_else(|| host(format!("pc {t} does not start a block")))?;
            let merged = match &entry_state[ti] {
                None => st.clone(),
                Some(cur) => join(cur, &st).map_err(host)?,
            };
            if entry_state[ti].as_ref() != Some(&merged) {
                entry_state[ti] = Some(merged);
                work.push(ti);
            }
        }
    }
    // ── Translation. ──
    let mut tr = Translator::new(program, f.register_count.max(params.len()));
    let region_blocks: Vec<u32> = (0..starts.len()).map(|_| tr.new_block()).collect();
    let mut ret: Option<Kind> = None;
    for bi in 0..starts.len() {
        let Some(mut st) = entry_state[bi].clone() else { continue };
        let at = region_blocks[bi];
        tr.at = at;
        let mut ended = false;
        for pc in starts[bi]..block_end(bi) {
            let before = st.clone();
            transfer(program, pc, &mut st).map_err(host)?;
            let term = tr.op(pc, &before, &region_blocks, &block_of, &mut ret).map_err(host)?;
            if term {
                ended = true;
                break;
            }
        }
        if !ended {
            let next = block_end(bi);
            if next >= end {
                return Err(host("control falls off the end of the function".to_string()));
            }
            tr.terminate(RTerm::Jump(region_blocks[block_of[&next]]));
        }
    }
    let ret = ret.ok_or_else(|| host("no path returns a value".to_string()))?;
    let regs = u16::try_from(tr.next_reg).map_err(|_| host("more registers than a region holds".to_string()))?;
    Ok(RegionCfg { name: name.to_string(), func: fi as u16, params, ret, regs, blocks: tr.finish() })
}

/// Every function of `program`, each a region or a host reason; `names[i]` is function `i`'s name.
pub fn discover(program: &CompiledProgram, names: &[String]) -> Vec<Discovered> {
    (0..program.functions.len())
        .map(|fi| match lower(program, fi, &names[fi]) {
            Ok(r) => Discovered::Region(r),
            Err(h) => Discovered::Host(h),
        })
        .collect()
}

/// What an op does to control.
enum Flow {
    Next,
    /// Unconditional transfer (no fall-through).
    Jump(usize),
    /// A possible transfer beside the fall-through.
    Branch(usize),
    /// A return.
    End,
}

fn scalar(st: &State, r: u16, pc: usize) -> Result<Kind, String> {
    match st.kinds.get(&r) {
        Some(k) => k.scalar().ok_or_else(|| format!("pc {pc}: register {r} is not a scalar here ({k:?})")),
        None => Err(format!("pc {pc}: register {r} is read where no path defines it")),
    }
}

/// The kind an arithmetic op on `(a, b)` produces: Int × Int stays Int, a Float operand makes it Float.
fn numeric(a: Kind, b: Kind, pc: usize, what: &str) -> Result<Kind, String> {
    match (a, b) {
        (Kind::Int, Kind::Int) => Ok(Kind::Int),
        (Kind::Float, Kind::Float) | (Kind::Int, Kind::Float) | (Kind::Float, Kind::Int) => Ok(Kind::Float),
        _ => Err(format!("pc {pc}: {what} of {a:?} and {b:?}")),
    }
}

/// Advance the dataflow state over the op at `pc`.
fn transfer(program: &CompiledProgram, pc: usize, st: &mut State) -> Result<Flow, String> {
    let set = |st: &mut State, r: u16, k: Ak| {
        st.kinds.insert(r, k);
    };
    match program.code[pc] {
        Op::LoadConst { dst, idx } => {
            let k = match &program.constants[idx as usize] {
                Constant::Int(_) => Ak::Int,
                Constant::Float(_) => Ak::Float,
                Constant::Bool(_) => Ak::Bool,
                other => return Err(format!("pc {pc}: a {} constant", constant_kind(other))),
            };
            set(st, dst, k);
        }
        Op::Move { dst, src } => {
            let k = *st.kinds.get(&src).ok_or_else(|| format!("pc {pc}: register {src} is read where no path defines it"))?;
            set(st, dst, k);
        }
        Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs } | Op::Mul { dst, lhs, rhs } | Op::Div { dst, lhs, rhs } => {
            let k = numeric(scalar(st, lhs, pc)?, scalar(st, rhs, pc)?, pc, program.code[pc].name())?;
            set(st, dst, Ak::of(k));
        }
        Op::AddAssign { dst, src } => {
            let k = numeric(scalar(st, dst, pc)?, scalar(st, src, pc)?, pc, "AddAssign")?;
            set(st, dst, Ak::of(k));
        }
        Op::FloorDiv { dst, lhs, rhs } | Op::Mod { dst, lhs, rhs } | Op::Pow { dst, lhs, rhs } | Op::Shl { dst, lhs, rhs } | Op::Shr { dst, lhs, rhs } => {
            let (a, b) = (scalar(st, lhs, pc)?, scalar(st, rhs, pc)?);
            if (a, b) != (Kind::Int, Kind::Int) {
                return Err(format!("pc {pc}: {} of {a:?} and {b:?}", program.code[pc].name()));
            }
            set(st, dst, Ak::Int);
        }
        Op::DivPow2 { dst, lhs, .. } | Op::MagicDivU { dst, lhs, .. } => {
            if scalar(st, lhs, pc)? != Kind::Int {
                return Err(format!("pc {pc}: {} of a non-Int", program.code[pc].name()));
            }
            set(st, dst, Ak::Int);
        }
        Op::Lt { dst, lhs, rhs } | Op::Gt { dst, lhs, rhs } | Op::LtEq { dst, lhs, rhs } | Op::GtEq { dst, lhs, rhs } => {
            let (a, b) = (scalar(st, lhs, pc)?, scalar(st, rhs, pc)?);
            if a != b || a == Kind::Bool {
                return Err(format!("pc {pc}: an ordering of {a:?} and {b:?}"));
            }
            set(st, dst, Ak::Bool);
        }
        Op::Eq { dst, lhs, rhs } | Op::NotEq { dst, lhs, rhs } => {
            let (a, b) = (scalar(st, lhs, pc)?, scalar(st, rhs, pc)?);
            if a != b {
                return Err(format!("pc {pc}: an equality of {a:?} and {b:?}"));
            }
            set(st, dst, Ak::Bool);
        }
        Op::ApproxEq { dst, lhs, rhs } => {
            let (a, b) = (scalar(st, lhs, pc)?, scalar(st, rhs, pc)?);
            if (a, b) != (Kind::Int, Kind::Int) {
                return Err(format!("pc {pc}: `is approximately` of {a:?} and {b:?}"));
            }
            set(st, dst, Ak::Bool);
        }
        Op::Not { dst, src } => {
            scalar(st, src, pc)?;
            set(st, dst, Ak::Bool);
        }
        Op::BitXor { dst, lhs, rhs } | Op::BitAnd { dst, lhs, rhs } | Op::BitOr { dst, lhs, rhs } => {
            let (a, b) = (scalar(st, lhs, pc)?, scalar(st, rhs, pc)?);
            let k = match (a, b) {
                (Kind::Int, Kind::Int) => Ak::Int,
                (Kind::Bool, Kind::Bool) => Ak::Bool,
                _ => return Err(format!("pc {pc}: {} of {a:?} and {b:?}", program.code[pc].name())),
            };
            set(st, dst, k);
        }
        Op::CallBuiltin { dst, builtin, args_start, arg_count } => {
            let k = builtin_kind(st, builtin, args_start, arg_count, pc)?;
            set(st, dst, Ak::of(k));
        }
        Op::Jump { target } => return Ok(Flow::Jump(target)),
        Op::JumpIfFalse { cond, target } | Op::JumpIfTrue { cond, target } => {
            scalar(st, cond, pc)?;
            return Ok(Flow::Branch(target));
        }
        Op::Return { src } => {
            scalar(st, src, pc)?;
            return Ok(Flow::End);
        }
        Op::ReturnNothing => return Err(format!("pc {pc}: a path returns nothing")),
        Op::NewRange { dst, start, end } => {
            if scalar(st, start, pc)? != Kind::Int || scalar(st, end, pc)? != Kind::Int {
                return Err(format!("pc {pc}: a range over non-Int bounds"));
            }
            set(st, dst, Ak::Range(pc));
        }
        Op::IterPrepare { iterable } => match st.kinds.get(&iterable) {
            Some(Ak::Range(_)) => st.iters.push(pc),
            other => return Err(format!("pc {pc}: iterating over {other:?}, not a range")),
        },
        Op::IterNext { dst, exit } => {
            if st.iters.is_empty() {
                return Err(format!("pc {pc}: IterNext with no live loop"));
            }
            set(st, dst, Ak::Int);
            return Ok(Flow::Branch(exit));
        }
        Op::IterPop => {
            st.iters.pop().ok_or_else(|| format!("pc {pc}: IterPop with no live loop"))?;
        }
        ref other => return Err(format!("pc {pc}: `{}` is not scalar region computation", other.name())),
    }
    Ok(Flow::Next)
}

fn constant_kind(c: &Constant) -> &'static str {
    match c {
        Constant::Int(_) => "Int",
        Constant::Float(_) => "Float",
        Constant::Bool(_) => "Bool",
        Constant::Text(_) => "Text",
        Constant::Char(_) => "Char",
        Constant::Nothing => "Nothing",
        Constant::Duration(_) => "Duration",
        Constant::Date(_) => "Date",
        Constant::Moment(_) => "Moment",
        Constant::Span { .. } => "Span",
        Constant::Time(_) => "Time",
    }
}

/// The builtins a region computes: `sqrt` (a Float), `floor` (an Int), `count_ones` (an Int).
fn builtin_kind(st: &State, b: BuiltinId, start: u16, count: u16, pc: usize) -> Result<Kind, String> {
    if count != 1 {
        return Err(format!("pc {pc}: builtin {b:?} with {count} arguments"));
    }
    let a = scalar(st, start, pc)?;
    match (b, a) {
        (BuiltinId::Sqrt, Kind::Int | Kind::Float) => Ok(Kind::Float),
        (BuiltinId::Floor, Kind::Int | Kind::Float) => Ok(Kind::Int),
        (BuiltinId::CountOnes, Kind::Int) => Ok(Kind::Int),
        _ => Err(format!("pc {pc}: builtin {b:?} of {a:?}")),
    }
}

/// The region under construction.
struct Translator<'a> {
    program: &'a CompiledProgram,
    next_reg: usize,
    blocks: Vec<RBlock>,
    open: Vec<Vec<RStmt>>,
    terms: Vec<Option<RTerm>>,
    at: u32,
    consts: BTreeMap<i64, VReg>,
    const_stmts: Vec<RStmt>,
    /// `NewRange` pc → the registers holding its start and end.
    ranges: BTreeMap<usize, (VReg, VReg)>,
    /// `IterPrepare` pc → its cursor, end and done registers.
    iters: BTreeMap<usize, (VReg, VReg, VReg)>,
}

impl<'a> Translator<'a> {
    fn new(program: &'a CompiledProgram, vm_regs: usize) -> Self {
        Translator {
            program,
            next_reg: vm_regs,
            blocks: Vec::new(),
            open: Vec::new(),
            terms: Vec::new(),
            at: 0,
            consts: BTreeMap::new(),
            const_stmts: Vec::new(),
            ranges: BTreeMap::new(),
            iters: BTreeMap::new(),
        }
    }

    fn reg(&mut self) -> VReg {
        let r = self.next_reg as VReg;
        self.next_reg += 1;
        r
    }

    fn new_block(&mut self) -> u32 {
        self.open.push(Vec::new());
        self.terms.push(None);
        (self.open.len() - 1) as u32
    }

    fn push(&mut self, s: RStmt) {
        self.open[self.at as usize].push(s);
    }

    fn terminate(&mut self, t: RTerm) {
        self.terms[self.at as usize] = Some(t);
    }

    /// A register holding `v`, set once at the region's entry.
    fn k(&mut self, v: i64) -> VReg {
        if let Some(r) = self.consts.get(&v) {
            return *r;
        }
        let r = self.reg();
        self.const_stmts.push(RStmt::Const { dst: r, value: v });
        self.consts.insert(v, r);
        r
    }

    fn cell(&mut self, op: CellOp, a: VReg, b: VReg) -> VReg {
        let dst = self.reg();
        self.push(RStmt::Cell { dst, op, a, b });
        dst
    }

    /// `x` as a Float: an Int is promoted by the kernel's `as f64`.
    fn float(&mut self, x: VReg, k: Kind) -> VReg {
        match k {
            Kind::Float => x,
            _ => {
                let zero = self.k(0);
                self.cell(CellOp::Unit(FuncUnit::IntToF), x, zero)
            }
        }
    }

    /// A branch condition's truth as a 0/1 register: `Int`/`Bool` are truthy when nonzero, a `Float` when
    /// it is not ±0 (NaN is truthy).
    fn truthy(&mut self, c: VReg, k: Kind) -> VReg {
        match k {
            Kind::Int | Kind::Bool => c,
            Kind::Float => {
                let zero = self.k(0);
                self.cell(CellOp::Unit(FuncUnit::FNeF), c, zero)
            }
        }
    }

    fn finish(mut self) -> Vec<RBlock> {
        let mut entry_consts = std::mem::take(&mut self.const_stmts);
        let mut blocks = Vec::with_capacity(self.open.len());
        for (i, stmts) in self.open.into_iter().enumerate() {
            let term = self.terms[i].unwrap_or(RTerm::Jump(i as u32));
            let stmts = if i == 0 {
                let mut s = std::mem::take(&mut entry_consts);
                s.extend(stmts);
                s
            } else {
                stmts
            };
            blocks.push(RBlock { stmts, term });
        }
        self.blocks = blocks;
        self.blocks
    }

    /// Translate the op at `pc`; `before` is the dataflow state entering it. Returns whether the op
    /// terminated the block.
    fn op(
        &mut self,
        pc: usize,
        before: &State,
        region_blocks: &[u32],
        block_of: &BTreeMap<usize, usize>,
        ret: &mut Option<Kind>,
    ) -> Result<bool, String> {
        let blk = |p: usize| -> Result<u32, String> {
            block_of.get(&p).map(|i| region_blocks[*i]).ok_or_else(|| format!("pc {p} does not start a block"))
        };
        let kind = |r: u16| scalar(before, r, pc);
        match self.program.code[pc] {
            Op::LoadConst { dst, idx } => {
                let v = match &self.program.constants[idx as usize] {
                    Constant::Int(n) => *n,
                    Constant::Bool(b) => i64::from(*b),
                    Constant::Float(f) => f.to_bits() as i64,
                    other => return Err(format!("pc {pc}: a {} constant", constant_kind(other))),
                };
                let c = self.k(v);
                self.push(RStmt::Copy { dst, src: c });
            }
            Op::Move { dst, src } => match before.kinds.get(&src) {
                Some(Ak::Range(_)) => {}
                _ => self.push(RStmt::Copy { dst, src }),
            },
            Op::Add { dst, lhs, rhs } => self.arith(dst, lhs, rhs, kind(lhs)?, kind(rhs)?, AluOp::Add, FuncUnit::FAddF),
            Op::AddAssign { dst, src } => self.arith(dst, dst, src, kind(dst)?, kind(src)?, AluOp::Add, FuncUnit::FAddF),
            Op::Sub { dst, lhs, rhs } => self.arith(dst, lhs, rhs, kind(lhs)?, kind(rhs)?, AluOp::Sub, FuncUnit::FSubF),
            Op::Mul { dst, lhs, rhs } => self.arith(dst, lhs, rhs, kind(lhs)?, kind(rhs)?, AluOp::Mul, FuncUnit::FMulF),
            Op::Div { dst, lhs, rhs } => {
                let (a, b) = (kind(lhs)?, kind(rhs)?);
                if (a, b) == (Kind::Int, Kind::Int) {
                    let v = self.cell(CellOp::Unit(FuncUnit::DivS), lhs, rhs);
                    self.push(RStmt::Copy { dst, src: v });
                } else {
                    let (x, y) = (self.float(lhs, a), self.float(rhs, b));
                    let v = self.cell(CellOp::Unit(FuncUnit::FDivF), x, y);
                    self.push(RStmt::Copy { dst, src: v });
                }
            }
            Op::FloorDiv { dst, lhs, rhs } => self.int_unit(dst, lhs, rhs, FuncUnit::FloorDivS),
            Op::Mod { dst, lhs, rhs } => self.int_unit(dst, lhs, rhs, FuncUnit::ModS),
            Op::Pow { dst, lhs, rhs } => self.int_unit(dst, lhs, rhs, FuncUnit::Pow),
            Op::Shl { dst, lhs, rhs } => self.alu(dst, lhs, rhs, AluOp::Shl),
            Op::Shr { dst, lhs, rhs } => self.alu(dst, lhs, rhs, AluOp::Shr),
            Op::BitXor { dst, lhs, rhs } => self.alu(dst, lhs, rhs, AluOp::Xor),
            Op::BitAnd { dst, lhs, rhs } => self.alu(dst, lhs, rhs, AluOp::And),
            Op::BitOr { dst, lhs, rhs } => self.alu(dst, lhs, rhs, AluOp::Or),
            Op::DivPow2 { dst, lhs, k } => {
                let d = self.k(1i64 << k);
                self.int_unit(dst, lhs, d, FuncUnit::DivS);
            }
            Op::MagicDivU { dst, lhs, mul_back, divisor, .. } => {
                let d = self.k(divisor);
                self.int_unit(dst, lhs, d, if mul_back == 0 { FuncUnit::DivS } else { FuncUnit::ModS });
            }
            Op::Lt { dst, lhs, rhs } => self.compare(dst, lhs, rhs, kind(lhs)?, CmpKind::Lt, FuncUnit::FLtF),
            Op::Gt { dst, lhs, rhs } => self.compare(dst, lhs, rhs, kind(lhs)?, CmpKind::Gt, FuncUnit::FGtF),
            Op::LtEq { dst, lhs, rhs } => self.compare(dst, lhs, rhs, kind(lhs)?, CmpKind::Le, FuncUnit::FLeF),
            Op::GtEq { dst, lhs, rhs } => self.compare(dst, lhs, rhs, kind(lhs)?, CmpKind::Ge, FuncUnit::FGeF),
            Op::Eq { dst, lhs, rhs } => self.compare(dst, lhs, rhs, kind(lhs)?, CmpKind::Eq, FuncUnit::FEqF),
            Op::NotEq { dst, lhs, rhs } => self.compare(dst, lhs, rhs, kind(lhs)?, CmpKind::Ne, FuncUnit::FNeF),
            Op::ApproxEq { dst, lhs, rhs } => {
                let v = self.cell(CellOp::Unit(FuncUnit::ApproxEqF), lhs, rhs);
                self.push(RStmt::Copy { dst, src: v });
            }
            Op::Not { dst, src } => {
                let t = self.truthy(src, kind(src)?);
                let (zero, one) = (self.k(0), self.k(1));
                self.push(RStmt::Select { dst, cmp: CmpKind::Eq, a: t, b: zero, t: one, e: zero });
            }
            Op::CallBuiltin { dst, builtin, args_start, .. } => {
                let a = kind(args_start)?;
                match builtin {
                    BuiltinId::Sqrt => {
                        let x = self.float(args_start, a);
                        let zero = self.k(0);
                        let v = self.cell(CellOp::Unit(FuncUnit::FSqrtF), x, zero);
                        self.push(RStmt::Copy { dst, src: v });
                    }
                    BuiltinId::Floor if a == Kind::Int => self.push(RStmt::Copy { dst, src: args_start }),
                    BuiltinId::Floor => {
                        let zero = self.k(0);
                        let v = self.cell(CellOp::Unit(FuncUnit::FFloorF), args_start, zero);
                        self.push(RStmt::Copy { dst, src: v });
                    }
                    BuiltinId::CountOnes => {
                        let zero = self.k(0);
                        let v = self.cell(CellOp::Unit(FuncUnit::PopCount), args_start, zero);
                        self.push(RStmt::Copy { dst, src: v });
                    }
                    other => return Err(format!("pc {pc}: builtin {other:?}")),
                }
            }
            Op::Jump { target } => {
                self.terminate(RTerm::Jump(blk(target)?));
                return Ok(true);
            }
            Op::JumpIfFalse { cond, target } => {
                let c = self.truthy(cond, kind(cond)?);
                self.terminate(RTerm::Branch { cond: c, then_: blk(pc + 1)?, else_: blk(target)? });
                return Ok(true);
            }
            Op::JumpIfTrue { cond, target } => {
                let c = self.truthy(cond, kind(cond)?);
                self.terminate(RTerm::Branch { cond: c, then_: blk(target)?, else_: blk(pc + 1)? });
                return Ok(true);
            }
            Op::Return { src } => {
                let k = kind(src)?;
                match ret {
                    None => *ret = Some(k),
                    Some(r) if *r == k => {}
                    Some(r) => return Err(format!("pc {pc}: returns a {k:?} where another path returns a {r:?}")),
                }
                self.terminate(RTerm::Return(src));
                return Ok(true);
            }
            Op::NewRange { start, end, .. } => {
                let (s, e) = (self.reg(), self.reg());
                self.push(RStmt::Copy { dst: s, src: start });
                self.push(RStmt::Copy { dst: e, src: end });
                self.ranges.insert(pc, (s, e));
            }
            Op::IterPrepare { iterable } => {
                let Some(Ak::Range(rp)) = before.kinds.get(&iterable).copied() else {
                    return Err(format!("pc {pc}: iterating over a non-range"));
                };
                let (s, e) = *self.ranges.get(&rp).ok_or_else(|| format!("pc {pc}: the range's NewRange was not translated first"))?;
                let (cur, end, done) = (self.reg(), self.reg(), self.reg());
                let (zero, one) = (self.k(0), self.k(1));
                self.push(RStmt::Copy { dst: cur, src: s });
                self.push(RStmt::Copy { dst: end, src: e });
                self.push(RStmt::Select { dst: done, cmp: CmpKind::Gt, a: cur, b: end, t: one, e: zero });
                self.iters.insert(pc, (cur, end, done));
            }
            Op::IterNext { dst, exit } => {
                let q = *before.iters.last().ok_or_else(|| format!("pc {pc}: IterNext with no live loop"))?;
                let (cur, end, done) = *self.iters.get(&q).ok_or_else(|| format!("pc {pc}: the loop's IterPrepare was not translated first"))?;
                let body = blk(pc + 1)?;
                let exit_b = blk(exit)?;
                let step = self.new_block();
                let last = self.new_block();
                let inc = self.new_block();
                self.terminate(RTerm::Branch { cond: done, then_: exit_b, else_: step });
                let (zero, one) = (self.k(0), self.k(1));
                self.at = step;
                self.push(RStmt::Copy { dst, src: cur });
                let is_last = self.reg();
                self.push(RStmt::Select { dst: is_last, cmp: CmpKind::Eq, a: cur, b: end, t: one, e: zero });
                self.terminate(RTerm::Branch { cond: is_last, then_: last, else_: inc });
                self.at = last;
                self.push(RStmt::Copy { dst: done, src: one });
                self.terminate(RTerm::Jump(body));
                self.at = inc;
                let next = self.cell(CellOp::Alu(AluOp::Add), cur, one);
                self.push(RStmt::Copy { dst: cur, src: next });
                self.terminate(RTerm::Jump(body));
                return Ok(true);
            }
            Op::IterPop => {}
            ref other => return Err(format!("pc {pc}: `{}` is not scalar region computation", other.name())),
        }
        Ok(false)
    }

    fn arith(&mut self, dst: VReg, lhs: VReg, rhs: VReg, a: Kind, b: Kind, int: AluOp, float: FuncUnit) {
        let v = if (a, b) == (Kind::Int, Kind::Int) {
            self.cell(CellOp::Alu(int), lhs, rhs)
        } else {
            let (x, y) = (self.float(lhs, a), self.float(rhs, b));
            self.cell(CellOp::Unit(float), x, y)
        };
        self.push(RStmt::Copy { dst, src: v });
    }

    fn int_unit(&mut self, dst: VReg, lhs: VReg, rhs: VReg, unit: FuncUnit) {
        let v = self.cell(CellOp::Unit(unit), lhs, rhs);
        self.push(RStmt::Copy { dst, src: v });
    }

    fn alu(&mut self, dst: VReg, lhs: VReg, rhs: VReg, op: AluOp) {
        let v = self.cell(CellOp::Alu(op), lhs, rhs);
        self.push(RStmt::Copy { dst, src: v });
    }

    fn compare(&mut self, dst: VReg, lhs: VReg, rhs: VReg, k: Kind, cmp: CmpKind, float: FuncUnit) {
        if k == Kind::Float {
            let v = self.cell(CellOp::Unit(float), lhs, rhs);
            self.push(RStmt::Copy { dst, src: v });
        } else {
            let (zero, one) = (self.k(0), self.k(1));
            self.push(RStmt::Select { dst, cmp, a: lhs, b: rhs, t: one, e: zero });
        }
    }
}

impl RegionCfg {
    /// Run the region under the cells' 64-bit checked semantics: the reference every device model is held
    /// to. `fuel` bounds the blocks executed.
    pub fn eval_checked(&self, args: &[i64], fuel: u64) -> Result<i64, Halt> {
        assert_eq!(args.len(), self.params.len(), "{}: arity", self.name);
        let mut regs = vec![0i64; self.regs as usize];
        regs[..args.len()].copy_from_slice(args);
        let mut block = 0usize;
        let mut left = fuel;
        loop {
            if left == 0 {
                return Err(Halt::OutOfFuel);
            }
            left -= 1;
            let b = &self.blocks[block];
            for s in &b.stmts {
                match *s {
                    RStmt::Const { dst, value } => regs[dst as usize] = value,
                    RStmt::Copy { dst, src } => regs[dst as usize] = regs[src as usize],
                    RStmt::Cell { dst, op, a, b } => {
                        let (x, y) = (regs[a as usize], regs[b as usize]);
                        let r = match op {
                            CellOp::Alu(alu) => alu.eval_checked(x, y),
                            CellOp::Unit(u) => u.eval_checked(x, y),
                            CellOp::Select(c) => Ok(i64::from(c.eval(x, y))),
                        };
                        regs[dst as usize] = r.map_err(|d| Halt::Declined { divergence: d, op, a: x, b: y })?;
                    }
                    RStmt::Select { dst, cmp, a, b, t, e } => {
                        regs[dst as usize] = if cmp.eval(regs[a as usize], regs[b as usize]) { regs[t as usize] } else { regs[e as usize] };
                    }
                }
            }
            match b.term {
                RTerm::Jump(t) => block = t as usize,
                RTerm::Branch { cond, then_, else_ } => block = if regs[cond as usize] != 0 { then_ } else { else_ } as usize,
                RTerm::Return(v) => return Ok(regs[v as usize]),
            }
        }
    }

    /// Run the region on a device's lanes: every value in the device's words, every cell its lane routine.
    /// The result, and the lane operations it cost.
    pub fn eval_on_lanes(&self, lanes: &mut dyn Lanes, args: &[i64], fuel: u64) -> Result<(i64, LaneCounts), Halt> {
        assert_eq!(args.len(), self.params.len(), "{}: arity", self.name);
        let shape = lanes.shape();
        let n = shape.words_for(64);
        let mut regs: Vec<Vec<u32>> = vec![vec![0; n]; self.regs as usize];
        for (i, a) in args.iter().enumerate() {
            regs[i] = split_u64(shape, *a as u64, n);
        }
        let mut counts = LaneCounts::default();
        let mut block = 0usize;
        let mut left = fuel;
        let lane_fault = |e: super::lane::LaneFault| -> Halt { panic!("{}: a lane routine refused its inputs: {e}", self.name) };
        loop {
            if left == 0 {
                return Err(Halt::OutOfFuel);
            }
            left -= 1;
            let b = &self.blocks[block];
            for s in &b.stmts {
                match *s {
                    RStmt::Const { dst, value } => regs[dst as usize] = split_u64(shape, value as u64, n),
                    RStmt::Copy { dst, src } => regs[dst as usize] = regs[src as usize].clone(),
                    RStmt::Cell { dst, op, a, b } => {
                        let r = routine(shape, op);
                        let mut input = regs[a as usize].clone();
                        input.extend_from_slice(&regs[b as usize]);
                        let (out, c) = run(&r, lanes, &input).map_err(lane_fault)?;
                        counts.absorb(&c);
                        let value = out[..n].to_vec();
                        if op.faults() {
                            let code = join_u64(shape, &out[n..]) as u32;
                            if code != 0 {
                                let divergence = Divergence::from_code(code).unwrap_or_else(|| panic!("{}: fault code {code}", self.name));
                                let (x, y) = (join_u64(shape, &regs[a as usize]) as i64, join_u64(shape, &regs[b as usize]) as i64);
                                return Err(Halt::Declined { divergence, op, a: x, b: y });
                            }
                        }
                        regs[dst as usize] = value;
                    }
                    RStmt::Select { dst, cmp, a, b, t, e } => {
                        let r = routine(shape, CellOp::Select(cmp));
                        let mut input = regs[a as usize].clone();
                        input.extend_from_slice(&regs[b as usize]);
                        input.extend_from_slice(&regs[t as usize]);
                        input.extend_from_slice(&regs[e as usize]);
                        let (out, c) = run(&r, lanes, &input).map_err(lane_fault)?;
                        counts.absorb(&c);
                        regs[dst as usize] = out;
                    }
                }
            }
            match b.term {
                RTerm::Jump(t) => block = t as usize,
                RTerm::Branch { cond, then_, else_ } => {
                    block = if regs[cond as usize].iter().any(|w| *w != 0) { then_ } else { else_ } as usize
                }
                RTerm::Return(v) => return Ok((join_u64(shape, &regs[v as usize]) as i64, counts)),
            }
        }
    }
}
