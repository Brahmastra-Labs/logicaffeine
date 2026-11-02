//! The enum lane: functions that build, inspect and return the program's own enum values run as
//! native code.
//!
//! A node lives in a frame register as `Rc::as_ptr` of the VM's own `Rc<InductiveValue>`, so a
//! node crosses the boundary in both directions without being copied. What a register holds is
//! tracked statically, per program point:
//!
//! - A **borrowed** node is kept alive by someone else for the whole call: a parameter (the caller's
//!   register holds it) or a field read out of a borrowed node (nodes are immutable, so the parent
//!   keeps it). Borrowed nodes cost nothing: no count is taken or dropped.
//! - An **owned** node carries one strong count of the register's own: a node the function built, a
//!   call's result, or a field read out of an owned node. The count is given away exactly once, into
//!   a new node's field or as the return value, or released on the control-flow edge along which its
//!   register dies (right after its last read, or on the branch edge that never reads it again). A
//!   move at a last use hands the count over and empties the source register.
//!
//! The analysis is a forward dataflow over the bytecode. Where a borrowed node and an owned one meet
//! in the same register, every borrowed node written to that register takes a count of its own, so
//! the paths agree. A point where an owned node meets a value that is not one cannot be released
//! correctly, so such a function stays on the VM. A side exit hands every node back to the VM: an
//! owned one transfers its count, and a borrowed one is shared (a count of its own is taken).

use std::rc::Rc;

use logicaffeine_compile::interpreter::{InductiveValue, RuntimeValue};
use logicaffeine_compile::vm::{BoundaryType, Constant, EnumTypeDef, Op};

use super::Kind;

/// What a frame register holds, as far as node ownership goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Hold {
    /// Zero: the prologue zeroed it and nothing wrote it since, or a move emptied it.
    Zero,
    /// Nothing this frame owns: a scalar, or a value no path agrees on.
    Junk,
    /// A node someone else keeps alive for the whole call.
    Borrowed,
    /// A node this register holds one strong count of.
    Owned,
    /// `Owned` on some paths to here and `Zero` on the rest.
    Maybe,
}

impl Hold {
    /// The register holds a count that must be given away or released.
    pub(crate) fn holds_count(self) -> bool {
        matches!(self, Hold::Owned | Hold::Maybe)
    }

    /// The register holds a node a side exit hands back to the VM.
    pub(crate) fn is_node(self) -> bool {
        matches!(self, Hold::Owned | Hold::Maybe | Hold::Borrowed)
    }
}

/// Why two paths cannot meet in one register.
enum Clash {
    /// A borrowed node meets an owned one: the borrowed side can take a count of its own.
    Promote,
    /// An owned node meets a value that is not one.
    Fatal,
}

/// Two paths meet.
fn join(a: Hold, b: Hold) -> Result<Hold, Clash> {
    use Hold::*;
    match (a, b) {
        (x, y) if x == y => Ok(x),
        (Zero | Owned | Maybe, Zero | Owned | Maybe) => Ok(Maybe),
        (Owned | Maybe, Borrowed) | (Borrowed, Owned | Maybe) => Err(Clash::Promote),
        (Owned | Maybe, _) | (_, Owned | Maybe) => Err(Clash::Fatal),
        (Zero, Borrowed) | (Borrowed, Zero) => Ok(Borrowed),
        _ => Ok(Junk),
    }
}

/// How one field slot enters a node under construction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FieldIn {
    Int,
    Bool,
    Float,
    /// A node the field takes a count of its own of.
    Share,
    /// An owned node at its last use: the field takes the register's count.
    Take,
}

/// A constructor call's fixed shape, owned by the compiled function.
pub(crate) struct Construct {
    pub(crate) type_name: String,
    pub(crate) ctor: String,
    pub(crate) fields: Vec<FieldIn>,
}

/// One op's ownership work, beyond the op itself.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Step {
    /// Nothing beyond the releases.
    Plain,
    /// A node copied between registers: `take` empties the source (a move at its last use), and
    /// `retain` gives the destination a count of its own.
    Move { take: bool, retain: bool },
    /// `NewInductive`, with each field's entry.
    Construct(Vec<FieldIn>),
    /// `BindArm`: the field's speculated kind, and whether a node read out of an owned parent takes
    /// a count of its own.
    Field { kind: Kind, retain: bool },
    /// `Return` of a node: a borrowed one takes the count it hands the caller.
    Return { retain: bool },
}

/// The ownership plan for one function body.
pub(crate) struct Ownership {
    /// Per op (None = unreachable): the holds when the op starts, after its releases — what a side
    /// exit at this op hands the VM.
    pub(crate) pause: Vec<Option<Vec<Hold>>>,
    /// Per op: registers whose count is released before the op runs (an overwrite the op does not
    /// read, or a return).
    pub(crate) release_before: Vec<Vec<u16>>,
    /// Per op: a register the op reads and then overwrites. Its old count waits in the hold slot
    /// and is released after the op.
    pub(crate) hold_old: Vec<Option<u16>>,
    /// Per op: the counts released on each edge out of it, `(successor, registers)`: every register
    /// that holds one after the op and is dead at that successor.
    pub(crate) edges: Vec<Vec<(usize, Vec<u16>)>>,
    pub(crate) steps: Vec<Step>,
    /// Registers the prologue zeroes: every non-parameter register that ever holds a node.
    pub(crate) zero_at_entry: Vec<u16>,
    /// Node parameters the prologue takes a count of (promoted: they meet owned nodes).
    pub(crate) retain_at_entry: Vec<u16>,
}

/// Whether a function body needs the enum lane: an enum parameter or an enum op.
pub(crate) fn uses_nodes(ops: &[Op], params_inductive: bool) -> bool {
    params_inductive
        || ops.iter().any(|op| {
            matches!(
                op,
                Op::NewInductive { .. } | Op::TestArm { .. } | Op::BindArm { .. }
            )
        })
}

/// The kind each `BindArm` binds, from the declared payload types: a bind sits in the block its
/// arm's `TestArm` guards, so the latest `TestArm` on the same register names the variant. Every
/// variant of that name across the program's enums must agree on the field. The kind is a
/// speculation the field read checks at run time, so a wrong guess side-exits, never misreads.
pub(crate) fn bind_kinds(ops: &[Op], constants: &[Constant], enums: &[EnumTypeDef]) -> Vec<Kind> {
    let mut arm: std::collections::HashMap<u16, &str> = std::collections::HashMap::new();
    let mut out = vec![Kind::Unknown; ops.len()];
    for (i, op) in ops.iter().enumerate() {
        match *op {
            Op::TestArm {
                target, variant, ..
            } => match constants.get(variant as usize) {
                Some(Constant::Text(v)) => {
                    arm.insert(target, v.as_str());
                }
                _ => {
                    arm.remove(&target);
                }
            },
            Op::BindArm { target, index, .. } => {
                let Some(variant) = arm.get(&target) else {
                    out[i] = Kind::Mixed;
                    continue;
                };
                let mut kind: Option<Kind> = None;
                for v in enums
                    .iter()
                    .flat_map(|e| &e.variants)
                    .filter(|v| v.name == *variant)
                {
                    let k = match v.field_types.get(index as usize) {
                        Some(BoundaryType::Enum(_)) => Kind::Inductive,
                        Some(BoundaryType::Int) => Kind::Int,
                        Some(BoundaryType::Bool) => Kind::Bool,
                        Some(BoundaryType::Float) => Kind::Float,
                        _ => Kind::Mixed,
                    };
                    kind = Some(match kind {
                        None => k,
                        Some(prev) if prev == k => k,
                        Some(_) => Kind::Mixed,
                    });
                }
                out[i] = kind.unwrap_or(Kind::Mixed);
            }
            _ => {}
        }
    }
    out
}

/// Successors of op `i` inside the body.
fn successors(ops: &[Op], i: usize, base_pc: usize) -> Vec<usize> {
    let n = ops.len();
    let inside = |t: usize| (t >= base_pc && t < base_pc + n).then(|| t - base_pc);
    match ops[i] {
        Op::Jump { target } => inside(target).into_iter().collect(),
        Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
            let mut s: Vec<usize> = (i + 1 < n).then_some(i + 1).into_iter().collect();
            s.extend(inside(target).filter(|&t| t != i + 1));
            s
        }
        Op::Return { .. } | Op::ReturnNothing => Vec::new(),
        _ => (i + 1 < n).then_some(i + 1).into_iter().collect(),
    }
}

/// What one op does to the holds.
struct Effect {
    /// The holds a side exit at this op hands back: the state on entry, after `release_before`.
    pause: Vec<Hold>,
    release_before: Vec<u16>,
    hold_old: Option<u16>,
    step: Step,
    out: Vec<Hold>,
}

/// The facts one function body's analysis reads.
struct Body<'a> {
    ops: &'a [Op],
    base_pc: usize,
    kin: &'a [Option<Vec<Kind>>],
    live: &'a [Vec<bool>],
    binds: &'a [Kind],
    returns_nodes: bool,
}

impl Body<'_> {
    /// What op `i` does to `held`, the holds on entry. `promote[r]` makes every borrowed node
    /// written to `r` take a count of its own. `None` rejects the function.
    fn effect(&self, i: usize, held: &[Hold], promote: &[bool]) -> Option<Effect> {
        let ops = self.ops;
        let k = self.kin[i].as_ref()?;
        let node = |r: u16| k[r as usize] == Kind::Inductive;
        let succs = successors(ops, i, self.base_pc);
        let live_out = |r: u16| succs.iter().any(|&s| self.live[s][r as usize]);
        let (uses, defs) = super::region_use_def(&ops[i])?;
        let mut s = held.to_vec();
        let mut release_before: Vec<u16> = Vec::new();
        let mut hold_old: Option<u16> = None;
        // An overwrite releases the old count: before the op when the op does not read it, after it
        // when it does. A swap exchanges what two registers hold and releases nothing.
        if !matches!(
            ops[i],
            Op::RevSwap { .. } | Op::Move { .. } | Op::Return { .. } | Op::ReturnNothing
        ) {
            for &d in &defs {
                if !s[d as usize].holds_count() {
                    continue;
                }
                if uses.contains(&d) {
                    if hold_old.replace(d).is_some() {
                        return None;
                    }
                } else {
                    release_before.push(d);
                    s[d as usize] = Hold::Zero;
                }
            }
        }
        // A value read as a node must be one on every path here.
        let readable = |r: u16| matches!(s[r as usize], Hold::Owned | Hold::Borrowed);
        let mut out;
        let step;
        match ops[i] {
            Op::Move { dst, src } if dst == src => {
                out = s.clone();
                step = Step::Plain;
            }
            Op::Move { dst, src } => {
                if s[dst as usize].holds_count() {
                    release_before.push(dst);
                    s[dst as usize] = Hold::Zero;
                }
                out = s.clone();
                if node(src) {
                    match s[src as usize] {
                        Hold::Owned if !live_out(src) => {
                            out[dst as usize] = Hold::Owned;
                            out[src as usize] = Hold::Zero;
                            step = Step::Move {
                                take: true,
                                retain: false,
                            };
                        }
                        Hold::Owned => {
                            out[dst as usize] = Hold::Owned;
                            step = Step::Move {
                                take: false,
                                retain: true,
                            };
                        }
                        Hold::Borrowed if promote[dst as usize] => {
                            out[dst as usize] = Hold::Owned;
                            step = Step::Move {
                                take: false,
                                retain: true,
                            };
                        }
                        Hold::Borrowed => {
                            out[dst as usize] = Hold::Borrowed;
                            step = Step::Move {
                                take: false,
                                retain: false,
                            };
                        }
                        _ => return None,
                    }
                } else {
                    out[dst as usize] = Hold::Junk;
                    step = Step::Plain;
                }
            }
            Op::RevSwap { a, b } => {
                out = s.clone();
                out.swap(a as usize, b as usize);
                step = Step::Plain;
            }
            Op::NewInductive {
                dst,
                args_start,
                count,
                ..
            } => {
                out = s.clone();
                let mut fields = Vec::with_capacity(count as usize);
                for a in args_start..args_start + count {
                    fields.push(match k[a as usize] {
                        Kind::Int => FieldIn::Int,
                        Kind::Bool => FieldIn::Bool,
                        Kind::Float => FieldIn::Float,
                        Kind::Inductive => match s[a as usize] {
                            Hold::Owned if a == dst || !live_out(a) => {
                                out[a as usize] = Hold::Zero;
                                FieldIn::Take
                            }
                            Hold::Owned | Hold::Borrowed => FieldIn::Share,
                            _ => return None,
                        },
                        _ => return None,
                    });
                }
                // A destination that is also a field gave its old count to that field.
                if hold_old == Some(dst) && fields[(dst - args_start) as usize] == FieldIn::Take {
                    hold_old = None;
                }
                out[dst as usize] = Hold::Owned;
                step = Step::Construct(fields);
            }
            Op::TestArm { dst, target, .. } => {
                if !node(target) || !readable(target) {
                    return None;
                }
                out = s.clone();
                out[dst as usize] = Hold::Junk;
                step = Step::Plain;
            }
            Op::BindArm { dst, target, .. } => {
                if !node(target) || !readable(target) {
                    return None;
                }
                out = s.clone();
                let kind = self.binds[i];
                match kind {
                    Kind::Inductive => {
                        let retain = s[target as usize] == Hold::Owned || promote[dst as usize];
                        out[dst as usize] = if retain { Hold::Owned } else { Hold::Borrowed };
                        step = Step::Field { kind, retain };
                    }
                    Kind::Int | Kind::Bool | Kind::Float => {
                        out[dst as usize] = Hold::Junk;
                        step = Step::Field {
                            kind,
                            retain: false,
                        };
                    }
                    _ => return None,
                }
            }
            Op::Call {
                dst,
                args_start,
                arg_count,
                ..
            } => {
                if (args_start..args_start + arg_count).any(|a| node(a) && !readable(a)) {
                    return None;
                }
                out = s.clone();
                out[dst as usize] = if self.returns_nodes {
                    Hold::Owned
                } else {
                    Hold::Junk
                };
                step = Step::Plain;
            }
            Op::Return { src } => {
                let retain = if node(src) {
                    match s[src as usize] {
                        Hold::Owned => false,
                        Hold::Borrowed => true,
                        _ => return None,
                    }
                } else {
                    false
                };
                for (r, h) in s.iter_mut().enumerate() {
                    if h.holds_count() && (r != src as usize || !node(src)) {
                        release_before.push(r as u16);
                        *h = Hold::Zero;
                    }
                }
                out = s.clone();
                step = if node(src) {
                    Step::Return { retain }
                } else {
                    Step::Plain
                };
            }
            Op::ReturnNothing => {
                for (r, h) in s.iter_mut().enumerate() {
                    if h.holds_count() {
                        release_before.push(r as u16);
                        *h = Hold::Zero;
                    }
                }
                out = s.clone();
                step = Step::Plain;
            }
            _ => {
                if uses.iter().any(|&u| node(u)) || hold_old.is_some() {
                    return None;
                }
                out = s.clone();
                for &d in &defs {
                    out[d as usize] = Hold::Junk;
                }
                step = Step::Plain;
            }
        }
        Some(Effect {
            pause: s,
            release_before,
            hold_old,
            step,
            out,
        })
    }

    /// The counts `out` (the holds after op `i`) loses along the edge to `s`: every register that holds
    /// one and is dead there.
    fn dying(&self, out: &[Hold], s: usize) -> Vec<u16> {
        (0..out.len())
            .filter(|&r| out[r].holds_count() && !self.live[s][r])
            .map(|r| r as u16)
            .collect()
    }
}

/// The ownership plan for a function body whose kinds `kin` and live-in sets `live` are known.
/// `params[r]` says parameter `r` is a node; every other register starts zeroed. `returns_nodes`
/// says the function's own calls return nodes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ownership(
    ops: &[Op],
    base_pc: usize,
    kin: &[Option<Vec<Kind>>],
    live: &[Vec<bool>],
    binds: &[Kind],
    params: &[bool],
    nregs: usize,
    returns_nodes: bool,
) -> Option<Ownership> {
    let body = Body {
        ops,
        base_pc,
        kin,
        live,
        binds,
        returns_nodes,
    };
    let n = ops.len();
    // A register where a borrowed node meets an owned one is PROMOTED: every borrowed node written
    // to it takes a count of its own (a parameter, at entry), so the paths agree. Promotion only
    // grows, so the search ends.
    let mut promote = vec![false; nregs];
    let held_in = 'search: loop {
        let mut start = vec![Hold::Zero; nregs];
        for (r, &is_node) in params.iter().enumerate() {
            start[r] = match (is_node, promote[r]) {
                (true, true) => Hold::Owned,
                (true, false) => Hold::Borrowed,
                (false, _) => Hold::Junk,
            };
        }
        let mut held_in: Vec<Option<Vec<Hold>>> = vec![None; n];
        held_in[0] = Some(start);
        let mut work = vec![0usize];
        while let Some(i) = work.pop() {
            let Some(cur) = held_in[i].clone() else {
                continue;
            };
            // An op refused on a state that later joins wider is retried when that state changes;
            // the final pass below judges every op on its converged state.
            let Some(e) = body.effect(i, &cur, &promote) else {
                continue;
            };
            for s in successors(ops, i, base_pc) {
                // A count whose register dies on this edge is released on it.
                let mut arriving = e.out.clone();
                for r in body.dying(&e.out, s) {
                    arriving[r as usize] = Hold::Zero;
                }
                let Some(prev) = &mut held_in[s] else {
                    held_in[s] = Some(arriving);
                    work.push(s);
                    continue;
                };
                let mut changed = false;
                for (r, (p, &o)) in prev.iter_mut().zip(&arriving).enumerate() {
                    match join(*p, o) {
                        Ok(j) => {
                            changed |= j != *p;
                            *p = j;
                        }
                        Err(Clash::Promote) if !promote[r] => {
                            promote[r] = true;
                            continue 'search;
                        }
                        Err(_) => return None,
                    }
                }
                if changed {
                    work.push(s);
                }
            }
        }
        break held_in;
    };
    let mut plan = Ownership {
        pause: vec![None; n],
        release_before: vec![Vec::new(); n],
        hold_old: vec![None; n],
        edges: vec![Vec::new(); n],
        steps: vec![Step::Plain; n],
        zero_at_entry: Vec::new(),
        retain_at_entry: (0..params.len())
            .filter(|&r| params[r] && promote[r])
            .map(|r| r as u16)
            .collect(),
    };
    let mut ever_node = vec![false; nregs];
    for i in 0..n {
        let (Some(held), Some(_)) = (&held_in[i], &kin[i]) else {
            continue;
        };
        let e = body.effect(i, held, &promote)?;
        for state in [held, &e.out] {
            for (r, h) in state.iter().enumerate() {
                ever_node[r] |= h.is_node();
            }
        }
        for s in successors(ops, i, base_pc) {
            let dying = body.dying(&e.out, s);
            if !dying.is_empty() {
                plan.edges[i].push((s, dying));
            }
        }
        plan.pause[i] = Some(e.pause);
        plan.release_before[i] = e.release_before;
        plan.hold_old[i] = e.hold_old;
        plan.steps[i] = e.step;
    }
    plan.zero_at_entry = (params.len()..nregs)
        .filter(|&r| ever_node[r])
        .map(|r| r as u16)
        .collect();
    Some(plan)
}

/// Field-read operand: the kind code in the low nibble, the retain flag above it, the field index
/// from bit 8.
pub(crate) fn field_operand(index: u16, kind: Kind, retain: bool) -> Option<i64> {
    let code = match kind {
        Kind::Inductive => FIELD_NODE,
        Kind::Int => FIELD_INT,
        Kind::Bool => FIELD_BOOL,
        Kind::Float => FIELD_FLOAT,
        _ => return None,
    };
    Some(((index as i64) << 8) | ((retain as i64) << 4) | code)
}

const FIELD_NODE: i64 = 1;
const FIELD_INT: i64 = 2;
const FIELD_BOOL: i64 = 3;
const FIELD_FLOAT: i64 = 4;
const FIELD_RETAIN: i64 = 1 << 4;

/// `frame[dst] = a new node` of the shape `*imm` (a [`Construct`]) from the slots `src..src + count`.
///
/// # Safety
/// `imm` is a live `Construct` whose field count is `count`; every `Share`/`Take` slot holds a live
/// node pointer, and a `Take` slot's count is handed over.
pub(crate) unsafe extern "C" fn logos_rt_node_new(
    base: *mut i64,
    dst: i64,
    src: i64,
    count: i64,
    imm: i64,
) -> i64 {
    let shape = &*(imm as *const Construct);
    let mut args = Vec::with_capacity(count as usize);
    for (k, entry) in shape.fields.iter().enumerate() {
        let raw = *base.add(src as usize + k);
        args.push(match entry {
            FieldIn::Int => RuntimeValue::Int(raw),
            FieldIn::Bool => RuntimeValue::Bool(raw != 0),
            FieldIn::Float => RuntimeValue::Float(f64::from_bits(raw as u64)),
            FieldIn::Share => {
                Rc::increment_strong_count(raw as *const InductiveValue);
                RuntimeValue::Inductive(Rc::from_raw(raw as *const InductiveValue))
            }
            FieldIn::Take => RuntimeValue::Inductive(Rc::from_raw(raw as *const InductiveValue)),
        });
    }
    let node = Rc::new(InductiveValue {
        inductive_type: shape.type_name.clone(),
        constructor: shape.ctor.clone(),
        args,
    });
    *base.add(dst as usize) = Rc::into_raw(node) as i64;
    1
}

/// `frame[dst] = the node in frame[src] is the variant named *imm` (a `String`).
///
/// # Safety
/// `frame[src]` holds a live node pointer and `imm` a live `String`.
pub(crate) unsafe extern "C" fn logos_rt_node_test(
    base: *mut i64,
    dst: i64,
    src: i64,
    _count: i64,
    imm: i64,
) -> i64 {
    let node = &*(*base.add(src as usize) as *const InductiveValue);
    let variant = &*(imm as *const String);
    *base.add(dst as usize) = (node.constructor == *variant) as i64;
    1
}

/// `frame[dst] = field imm>>8 of the node in frame[src]`, read as the kind `imm` names; answers 0 (a
/// side exit) when the field is missing or holds another kind.
///
/// # Safety
/// `frame[src]` holds a live node pointer.
pub(crate) unsafe extern "C" fn logos_rt_node_field(
    base: *mut i64,
    dst: i64,
    src: i64,
    _count: i64,
    imm: i64,
) -> i64 {
    let node = &*(*base.add(src as usize) as *const InductiveValue);
    let Some(value) = node.args.get((imm >> 8) as usize) else {
        return 0;
    };
    let raw = match (imm & 0xF, value) {
        (FIELD_NODE, RuntimeValue::Inductive(field)) => {
            let ptr = Rc::as_ptr(field);
            if imm & FIELD_RETAIN != 0 {
                Rc::increment_strong_count(ptr);
            }
            ptr as i64
        }
        (FIELD_INT, RuntimeValue::Int(n)) => *n,
        (FIELD_BOOL, RuntimeValue::Bool(b)) => *b as i64,
        (FIELD_FLOAT, RuntimeValue::Float(f)) => f.to_bits() as i64,
        _ => return 0,
    };
    *base.add(dst as usize) = raw;
    1
}

/// Take one more count of the node in `frame[src]` (none when the register is zero).
///
/// # Safety
/// `frame[src]` is zero or a live node pointer.
pub(crate) unsafe extern "C" fn logos_rt_node_retain(
    base: *mut i64,
    _dst: i64,
    src: i64,
    _count: i64,
    _imm: i64,
) -> i64 {
    let raw = *base.add(src as usize);
    if raw != 0 {
        Rc::increment_strong_count(raw as *const InductiveValue);
    }
    1
}

/// Release the count `frame[src]` holds (none when the register is zero).
///
/// # Safety
/// `frame[src]` is zero or a node pointer carrying a count this frame owns.
pub(crate) unsafe extern "C" fn logos_rt_node_release(
    base: *mut i64,
    _dst: i64,
    src: i64,
    _count: i64,
    _imm: i64,
) -> i64 {
    let raw = *base.add(src as usize);
    if raw != 0 {
        Rc::decrement_strong_count(raw as *const InductiveValue);
    }
    1
}

/// The VM value a side exit hands back for a register holding `raw` in state `hold`: an owned node
/// transfers its count, a borrowed one takes a count of its own. `None` for an empty register.
///
/// # Safety
/// `raw` is zero or a live node pointer; an owned one carries a count this frame gives up.
pub(crate) unsafe fn rebox(raw: i64, hold: Hold) -> Option<logicaffeine_compile::vm::Value> {
    if raw == 0 {
        return None;
    }
    let ptr = raw as *const InductiveValue;
    let node = match hold {
        Hold::Owned | Hold::Maybe => Rc::from_raw(ptr),
        Hold::Borrowed => {
            Rc::increment_strong_count(ptr);
            Rc::from_raw(ptr)
        }
        Hold::Zero | Hold::Junk => return None,
    };
    Some(logicaffeine_compile::vm::Value::from_runtime(
        RuntimeValue::Inductive(node),
    ))
}
