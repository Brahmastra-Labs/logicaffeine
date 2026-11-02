//! The machine form a bytecode backend holds a number variable in, when the definitions of the variable make numbers
//! of different forms ([`crate::analysis::numbers`] says how forms join).
//!
//! The VM keeps each number the value it is and changes a variable's kind as the program assigns it: `total` is the
//! integer 0 until a real is added to it, a counter holds an integer until it gains a fraction. A backend that holds
//! a register in one machine form across the points a value reaches joins the forms its definitions make, and each
//! definition whose own form differs lands converted into the joined one. Only a merge of control joins forms: a
//! straight run of assignments reaches each read with one definition, so each value keeps its own form there, and a
//! variable continues past a definition only through a collection changed in place, whose items stay where they are.
//! An integer and a real join to the real only where the program cannot tell the integer's phase from the real it
//! widens to: each integer definition is a constant a double holds exactly, and every op that reads the variable
//! answers that integer as it answers the real. Anything else has no machine form, and the backend refuses it by
//! name.

use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(crate) use crate::analysis::numbers::{Form, Number};
use crate::analysis::numbers::{exact_join, EXACT_IN_A_DOUBLE};
use crate::vm::variables::{changes_in_place, Flow, Site, Variables};
use crate::vm::{Constant, Op};

/// The variables of a region as a number join reads them: a definition continues the variable it reads only when it
/// changes a collection in place.
pub(crate) fn variables(code: &[Op], region: &impl Flow, entries: u16) -> Variables {
    Variables::of(code, region, entries, |op, w, r| w == r && changes_in_place(op))
}

/// How a region holds its number variables.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Joined {
    /// Each definition by an op whose own form differs from its variable's, by pc: the form it lands as.
    pub(crate) lands: BTreeMap<usize, Form>,
    /// Each register live on entry whose form differs from its variable's: the form it is converted to.
    pub(crate) entries: BTreeMap<u16, Form>,
    /// One definition (by pc, or on entry by register) of each variable no machine form holds.
    pub(crate) refused: Vec<Site>,
}

/// The joins of `region`, whose first `entries` registers are live on entry: `form(site)` is the form a definition
/// makes on its own (`None` for a value that is no number and no list of numbers), `real(pc, reg)` whether `reg` holds
/// a real as the op at `pc` reads it.
pub(crate) fn joined(
    code: &[Op],
    constants: &[Constant],
    region: &impl Flow,
    entries: u16,
    form: impl Fn(Site) -> Option<Form>,
    real: impl Fn(usize, u16) -> bool,
) -> Joined {
    let vars = variables(code, region, entries);
    let mut members: BTreeMap<usize, Vec<(usize, Site, Option<Form>)>> = BTreeMap::new();
    for (d, site) in vars.sites() {
        members.entry(vars.variable(d)).or_default().push((d, site, form(site)));
    }
    let mut joined = Joined::default();
    let mut widened: BTreeMap<usize, Form> = BTreeMap::new();
    let mut integer_real: Vec<usize> = Vec::new();
    for (&var, defs) in &members {
        let forms: BTreeSet<Form> = defs.iter().filter_map(|(_, _, f)| *f).collect();
        if forms.len() < 2 || defs.iter().any(|(_, _, f)| f.is_none()) {
            continue;
        }
        let numbers: Option<Vec<Number>> = forms
            .iter()
            .map(|f| match f {
                Form::Number(n) => Some(*n),
                Form::List(_) => None,
            })
            .collect();
        let items: Option<Vec<Number>> = forms
            .iter()
            .map(|f| match f {
                Form::List(n) => Some(*n),
                Form::Number(_) => None,
            })
            .collect();
        let exact = |ns: &[Number]| ns.iter().copied().try_fold(ns[0], exact_join);
        match (numbers, items) {
            (Some(ns), _) => match exact(&ns) {
                Some(n) => {
                    widened.insert(var, Form::Number(n));
                }
                None if ns.iter().all(|n| matches!(n, Number::Int | Number::Real)) => integer_real.push(var),
                None => joined.refused.push(defs[0].1),
            },
            (_, Some(ns)) => match exact(&ns) {
                Some(n) => {
                    widened.insert(var, Form::List(n));
                }
                None => joined.refused.push(defs[0].1),
            },
            _ => joined.refused.push(defs[0].1),
        }
    }
    for var in integer_real {
        match hidden_integer_phase(code, constants, region, &vars, &members, var, &real) {
            Some(copies) => {
                for v in copies {
                    widened.insert(v, Form::Number(Number::Real));
                }
            }
            None => joined.refused.push(members[&var][0].1),
        }
    }
    for (var, to) in widened {
        for (_, site, own) in &members[&var] {
            if *own == Some(to) {
                continue;
            }
            match *site {
                Site::Op(pc) => {
                    joined.lands.insert(pc, to);
                }
                Site::Entry(reg) => {
                    joined.entries.insert(reg, to);
                }
            }
        }
    }
    joined
}

/// Whether variable `var`, whose definitions make integers and reals, can hold its integers widened to reals without the
/// program telling: every op that reads it answers an integer as it answers the real it widens to (`+ - *` beside a
/// real, an order or equality test, a display, a copy into a variable widened with it), and each integer definition
/// of a variable so widened is a constant a double holds exactly, or a copy of one, or a copy of a widened value.
/// The variables to widen, `var` and every one it is copied into, or `None`.
fn hidden_integer_phase(
    code: &[Op],
    constants: &[Constant],
    region: &impl Flow,
    vars: &Variables,
    members: &BTreeMap<usize, Vec<(usize, Site, Option<Form>)>>,
    var: usize,
    real: &impl Fn(usize, u16) -> bool,
) -> Option<BTreeSet<usize>> {
    let mut reads: HashMap<usize, Vec<(usize, u16)>> = HashMap::new();
    for pc in region.pcs() {
        let mut read = Vec::new();
        region.read(&code[pc], &mut |r| read.push(r));
        for r in read {
            let mut of: Vec<usize> = vars.reaching(pc, r).map(|d| vars.variable(d)).collect();
            of.sort_unstable();
            of.dedup();
            for v in of {
                reads.entry(v).or_default().push((pc, r));
            }
        }
    }
    let mut widened: BTreeSet<usize> = BTreeSet::from([var]);
    let mut work = vec![var];
    while let Some(v) = work.pop() {
        for &(pc, r) in reads.get(&v).into_iter().flatten() {
            let beside_a_real = |other: u16| other != r && real(pc, other);
            let hidden = match code[pc] {
                Op::Add { lhs, rhs, .. } | Op::Sub { lhs, rhs, .. } | Op::Mul { lhs, rhs, .. } => {
                    beside_a_real(if lhs == r { rhs } else { lhs })
                }
                Op::AddAssign { dst, src } => beside_a_real(if dst == r { src } else { dst }),
                Op::Lt { .. }
                | Op::Gt { .. }
                | Op::LtEq { .. }
                | Op::GtEq { .. }
                | Op::Eq { .. }
                | Op::NotEq { .. }
                | Op::Show { .. } => true,
                Op::Move { dst, .. } => match vars.definition(pc) {
                    Some((w, d)) if w == dst => {
                        let into = vars.variable(d);
                        if widened.insert(into) {
                            work.push(into);
                        }
                        true
                    }
                    _ => false,
                },
                _ => false,
            };
            if !hidden {
                return None;
            }
        }
    }
    let exact_in_a_double = |pc: usize| -> bool {
        let mut at = pc;
        loop {
            match code[at] {
                Op::LoadConst { idx, .. } => {
                    return matches!(constants.get(idx as usize), Some(Constant::Int(c)) if c.unsigned_abs() < EXACT_IN_A_DOUBLE)
                }
                Op::Move { src, .. } => {
                    let from: Vec<usize> = vars.reaching(at, src).collect();
                    if !from.is_empty() && from.iter().all(|d| widened.contains(&vars.variable(*d))) {
                        return true;
                    }
                    let [d] = from[..] else { return false };
                    match vars.site(d) {
                        Site::Op(p) if p != at => at = p,
                        _ => return false,
                    }
                }
                _ => return false,
            }
        }
    };
    for v in &widened {
        for (_, site, own) in members.get(v).into_iter().flatten() {
            match (*own, *site) {
                (Some(Form::Number(Number::Real)), _) => {}
                (Some(Form::Number(Number::Int)), Site::Op(pc)) if exact_in_a_double(pc) => {}
                _ => return None,
            }
        }
    }
    Some(widened)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/numbers/tests.rs"]
mod tests;
