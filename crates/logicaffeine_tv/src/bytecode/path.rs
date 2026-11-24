//! What an exploration produces, on either side: the paths from a cut point to the next one, and how a call is taken.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use z3::ast::{Ast, Bool, Dynamic, Int};
use z3::{FuncDecl, Sort};

use super::value::{Kind, SymVal};
use crate::symexec::Unsupported;

/// Where a path stopped.
#[derive(Clone, Debug)]
pub enum End {
    /// At the head of loop `k` (numbered in the order the loops begin).
    Cut(usize),
    /// The function returned this value (`nothing` for a bare return or falling off its end).
    Return(SymVal),
    /// The runtime raised.
    Raise,
    /// The compiled code stopped with a loud integer overflow: a value it holds in 64 bits needed more (the emitted
    /// Rust; the source and the VM promote instead).
    Overflow,
    /// `## Main` finished.
    Halt,
}

/// A state at a cut point: every variable or register with its value, and on the bytecode side how many iterators
/// are open.
#[derive(Clone, Debug)]
pub struct State<L> {
    pub env: BTreeMap<L, SymVal>,
    pub depth: u8,
}

/// One way through a segment: the condition under which it is taken, where it ends, what it showed, and the state it
/// ends in.
#[derive(Clone, Debug)]
pub struct Path<L> {
    pub cond: Bool,
    pub end: End,
    pub out: Vec<SymVal>,
    pub state: State<L>,
}

/// A loop in a function's loop nest: its head, and the loop it sits inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopNode {
    pub head: usize,
    pub parent: Option<usize>,
}

/// The loop nest of a graph given by its successors from `entry`: a loop per node that is the target of a back edge
/// (an edge to a node that dominates its source), numbered by that head's position, each inside the smallest loop
/// whose natural body holds its head. Both sides number their loops this way, so loop `k` of one is paired with loop
/// `k` of the other by the same definition. A cycle with no such head (two entries) is refused.
pub fn loop_nest(entry: usize, succ: &BTreeMap<usize, Vec<usize>>) -> Result<Vec<LoopNode>, String> {
    let nodes: BTreeSet<usize> = succ.keys().copied().collect();
    let mut preds: BTreeMap<usize, Vec<usize>> = nodes.iter().map(|&p| (p, Vec::new())).collect();
    for (&p, ns) in succ {
        for &n in ns {
            preds.get_mut(&n).ok_or("jumps outside its own code")?.push(p);
        }
    }
    let mut dom: BTreeMap<usize, BTreeSet<usize>> = nodes
        .iter()
        .map(|&p| (p, if p == entry { BTreeSet::from([entry]) } else { nodes.clone() }))
        .collect();
    let mut changed = true;
    while changed {
        changed = false;
        for &p in &nodes {
            if p == entry {
                continue;
            }
            let mut meet: Option<BTreeSet<usize>> = None;
            for q in &preds[&p] {
                meet = Some(match meet {
                    None => dom[q].clone(),
                    Some(m) => m.intersection(&dom[q]).copied().collect(),
                });
            }
            let mut next = meet.unwrap_or_default();
            next.insert(p);
            if next != dom[&p] {
                dom.insert(p, next);
                changed = true;
            }
        }
    }
    let mut bodies: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut forward: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (&u, ns) in succ {
        for &h in ns {
            if dom[&u].contains(&h) {
                let body = bodies.entry(h).or_insert_with(|| BTreeSet::from([h]));
                let mut work = vec![u];
                while let Some(x) = work.pop() {
                    if body.insert(x) {
                        work.extend(preds[&x].iter().copied());
                    }
                }
            } else {
                forward.entry(u).or_default().push(h);
            }
        }
    }
    if cyclic(&forward, &nodes) {
        return Err("has a loop with two entries".into());
    }
    let heads: Vec<usize> = bodies.keys().copied().collect();
    Ok(heads
        .iter()
        .map(|&h| LoopNode {
            head: h,
            parent: heads
                .iter()
                .enumerate()
                .filter(|&(_, &o)| o != h && bodies[&o].contains(&h))
                .min_by_key(|&(_, &o)| bodies[&o].len())
                .map(|(j, _)| j),
        })
        .collect())
}

/// The graph left once every back edge is removed still has a cycle.
fn cyclic(forward: &BTreeMap<usize, Vec<usize>>, nodes: &BTreeSet<usize>) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Closed,
    }
    let mut marks: HashMap<usize, Mark> = HashMap::new();
    for &root in nodes {
        if marks.contains_key(&root) {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        marks.insert(root, Mark::Open);
        while let Some((n, i)) = stack.pop() {
            match forward.get(&n).and_then(|ns| ns.get(i)).copied() {
                Some(m) => {
                    stack.push((n, i + 1));
                    match marks.get(&m) {
                        Some(Mark::Open) => return true,
                        Some(Mark::Closed) => {}
                        None => {
                            marks.insert(m, Mark::Open);
                            stack.push((m, 0));
                        }
                    }
                }
                None => {
                    marks.insert(n, Mark::Closed);
                }
            }
        }
    }
    false
}

/// How far a run goes. A symbolic run stops at every loop head it reaches and inlines only loop-free callees; a
/// concrete run (every value a constant) goes through loop heads and into every callee, and spends one unit of fuel
/// per step, so a program that does not finish is refused rather than followed forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Symbolic,
    Concrete,
}

/// The budget a concrete run spends.
pub struct Budget(Cell<u64>);

impl Budget {
    pub fn new(steps: u64) -> Budget {
        Budget(Cell::new(steps))
    }

    pub fn spend(&self) -> Result<(), Unsupported> {
        let left = self.0.get();
        if left == 0 {
            return Err(Unsupported("the run did not finish within its step budget".into()));
        }
        self.0.set(left - 1);
        Ok(())
    }

    pub fn left(&self) -> u64 {
        self.0.get()
    }
}

/// The steps a call with constant arguments may run concretely before it is taken symbolically instead.
pub const CONSTANT_CALL_STEPS: u64 = 100_000;

/// Every argument is a constant, so the call can be run rather than modeled.
pub fn all_constant(args: &[SymVal]) -> bool {
    args.iter().all(|a| matches!(a, SymVal::Int(_) | SymVal::Bool(_)) && a.constant().is_some())
}

/// How a symbolic run takes a call, decided once per callee and shared by both sides so they take it the same way.
#[derive(Clone, Debug)]
pub enum CallKind {
    /// Loop-free, non-recursive and silent: its body runs in place, on each side its own code.
    Inline,
    /// Recursive or looping, and silent: an uninterpreted function of its arguments, the same symbol on both sides.
    /// Sound because each side calls the function the other's proof covers; the report names it as assumed.
    Opaque { ret: Option<Kind> },
    /// Shows something, or is outside the fragment: a call to it is not modeled.
    Refused(String),
}

/// The decision for every callee by name.
pub type Calls = HashMap<String, CallKind>;

/// An opaque call's value and the condition under which it raises: the same two uninterpreted functions of the
/// arguments on both sides.
pub fn opaque_call(name: &str, ret: Option<Kind>, args: &[SymVal]) -> Result<(SymVal, Bool), Unsupported> {
    let mut sorts = Vec::new();
    let mut terms: Vec<Dynamic> = Vec::new();
    for a in args {
        match a {
            SymVal::Int(i) => {
                sorts.push(Sort::int());
                terms.push(Dynamic::from_ast(i));
            }
            SymVal::Bool(b) => {
                sorts.push(Sort::bool());
                terms.push(Dynamic::from_ast(b));
            }
            other => return Err(Unsupported(format!("an argument of kind {:?} to `{name}`", other.kind()))),
        }
    }
    let sort_refs: Vec<&Sort> = sorts.iter().collect();
    let arg_refs: Vec<&dyn Ast> = terms.iter().map(|t| t as &dyn Ast).collect();
    let signature: String = sorts.iter().map(|s| format!("{s}")).collect::<Vec<_>>().join(",");
    let raises = FuncDecl::new(format!("{name}({signature}).raises"), &sort_refs, &Sort::bool())
        .apply(&arg_refs)
        .as_bool()
        .expect("a Bool-valued function");
    let value = match ret {
        Some(Kind::Int) => SymVal::Int(
            FuncDecl::new(format!("{name}({signature})"), &sort_refs, &Sort::int())
                .apply(&arg_refs)
                .as_int()
                .expect("an Int-valued function"),
        ),
        Some(Kind::Bool) => SymVal::Bool(
            FuncDecl::new(format!("{name}({signature})"), &sort_refs, &Sort::bool())
                .apply(&arg_refs)
                .as_bool()
                .expect("a Bool-valued function"),
        ),
        None | Some(Kind::Nothing) => SymVal::Nothing,
        Some(k) => return Err(Unsupported(format!("`{name}` returns {k:?}"))),
    };
    Ok((value, raises))
}

/// The value and raise condition of a loop-free, silent callee run in place: its return paths folded into one `ite`
/// chain, and the disjunction of its raising paths. The paths partition the callee's inputs, so the chain is exact.
pub fn fold_returns<L>(name: &str, paths: Vec<Path<L>>) -> Result<(SymVal, Bool), Unsupported> {
    let mut raises = Vec::new();
    let mut returns: Vec<(Bool, SymVal)> = Vec::new();
    for p in paths {
        if !p.out.is_empty() {
            return Err(Unsupported(format!("`{name}` shows a value while run in place")));
        }
        match p.end {
            End::Raise => raises.push(p.cond),
            End::Return(v) => returns.push((p.cond, v)),
            End::Cut(_) | End::Halt | End::Overflow => {
                return Err(Unsupported(format!("`{name}` reached a loop head while run in place")))
            }
        }
    }
    let raise = Bool::or(&raises);
    let Some((_, last)) = returns.pop() else {
        return Ok((SymVal::Nothing, raise));
    };
    let mut value = last;
    while let Some((cond, v)) = returns.pop() {
        value = match (&v, &value) {
            (SymVal::Int(a), SymVal::Int(b)) => SymVal::Int(cond.ite(a, b)),
            (SymVal::Bool(a), SymVal::Bool(b)) => SymVal::Bool(cond.ite(a, b)),
            (SymVal::Nothing, SymVal::Nothing) => SymVal::Nothing,
            (a, b) => {
                return Err(Unsupported(format!(
                    "`{name}` returns {:?} on one path and {:?} on another",
                    a.kind(),
                    b.kind()
                )))
            }
        };
    }
    Ok((value, raise))
}

/// A callee run concretely: its one path's value (or that it raised) and what it showed.
pub fn concrete_return<L>(name: &str, paths: Vec<Path<L>>) -> Result<(SymVal, bool, Vec<SymVal>), Unsupported> {
    let mut live = paths.into_iter().filter(|p| !dead(&p.cond));
    let (Some(p), None) = (live.next(), live.next()) else {
        return Err(Unsupported(format!("a concrete call to `{name}` did not take exactly one path")));
    };
    match p.end {
        End::Return(v) => Ok((v, false, p.out)),
        End::Raise => Ok((SymVal::Nothing, true, p.out)),
        End::Cut(_) | End::Halt | End::Overflow => {
            Err(Unsupported(format!("a concrete call to `{name}` ended outside a return or a raise")))
        }
    }
}

/// The condition is false as written, so its path is never taken.
pub fn dead(cond: &Bool) -> bool {
    cond.simplify().as_bool() == Some(false)
}

/// A fresh value of a kind, named for the counterexample it may appear in.
pub fn fresh(kind: Kind, name: &str) -> SymVal {
    match kind {
        Kind::Int => SymVal::Int(Int::new_const(name)),
        Kind::Bool => SymVal::Bool(z3::ast::Bool::new_const(name)),
        Kind::Nothing => SymVal::Nothing,
        Kind::Range => SymVal::Range(Int::new_const(format!("{name}.from")), Int::new_const(format!("{name}.to"))),
        Kind::Clobbered => SymVal::Clobbered,
    }
}
