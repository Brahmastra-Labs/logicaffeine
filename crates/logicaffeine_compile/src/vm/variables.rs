//! The variables of one region of bytecode. A register is reused for unrelated values; a variable is what it holds
//! across the points one value reaches. Its definitions are a register live on entry and each op that gives a
//! register a value of its own, and two definitions are one variable when a single read is reached by both
//! (reaching definitions over the region's control flow), or when an op continues a variable it reads rather than
//! starting one (`continues`). A backend that holds a register in one form across the points a variable
//! reaches decides that form per variable: a value of the exact extension ([`super::tower::webs`]), a number whose
//! definitions make numbers of different kinds ([`super::numbers`]).

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::vm::Op;

/// One region of bytecode as [`Variables`] reads it: its pcs that run, in order from its entry, and how control and
/// registers flow through them on the backend that lowers it.
pub(crate) trait Flow {
    /// The region's pcs that run, in execution order; the first is where control enters.
    fn pcs(&self) -> Vec<usize>;
    /// The pcs control can pass to from `pc`, within the region.
    fn successors(&self, pc: usize) -> Vec<usize>;
    /// Every register `op` writes, a `mutable` argument a call hands back included.
    fn written(&self, op: &Op, f: &mut dyn FnMut(u16));
    /// Every register `op` reads.
    fn read(&self, op: &Op, f: &mut dyn FnMut(u16));
}

/// Where a definition is made: on entry (a register live into the region) or by the op at a pc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Site {
    Entry(u16),
    Op(usize),
}

/// The one register `op` gives a new value of its own, a list it changes in place (a push, an item stored) included.
pub(crate) fn defined(op: &Op) -> Option<u16> {
    let mut written = Vec::new();
    op.for_each_written_reg(|w| written.push(w));
    match (written.as_slice(), op) {
        ([w], _) => Some(*w),
        ([], Op::ListPush { list, .. }) => Some(*list),
        ([], Op::SetIndex { collection, .. } | Op::SetIndexUnchecked { collection, .. }) => Some(*collection),
        _ => None,
    }
}

/// Whether `op` changes a collection in place: what it defines is the collection it read, still the same value.
pub(crate) fn changes_in_place(op: &Op) -> bool {
    matches!(op, Op::ListPush { .. } | Op::SetIndex { .. } | Op::SetIndexUnchecked { .. })
}

/// The definitions of a region and the variables they make.
#[derive(Debug, Clone)]
pub(crate) struct Variables {
    sites: Vec<Site>,
    variable: Vec<usize>,
    op_site: HashMap<usize, (u16, usize)>,
    at: HashMap<usize, usize>,
    reach_in: Vec<HashMap<u16, BTreeSet<usize>>>,
}

impl Variables {
    /// The variables of `region`, whose first `entries` registers are live on entry. An op that defines register `w`
    /// and reads register `r` continues the variable of that read when `continues(op, w, r)`.
    pub(crate) fn of(
        code: &[Op],
        region: &impl Flow,
        entries: u16,
        continues: impl Fn(&Op, u16, u16) -> bool,
    ) -> Variables {
        let pcs = region.pcs();
        let at: HashMap<usize, usize> = pcs.iter().enumerate().map(|(i, pc)| (*pc, i)).collect();
        let mut sites: Vec<Site> = (0..entries).map(Site::Entry).collect();
        let mut op_site: HashMap<usize, (u16, usize)> = HashMap::new();
        for &pc in &pcs {
            if let Some(w) = defined(&code[pc]) {
                op_site.insert(pc, (w, sites.len()));
                sites.push(Site::Op(pc));
            }
        }
        let mut reach_in: Vec<HashMap<u16, BTreeSet<usize>>> = vec![HashMap::new(); pcs.len()];
        if let Some(&first) = pcs.first() {
            for (d, site) in sites.iter().enumerate() {
                if let Site::Entry(reg) = site {
                    reach_in[at[&first]].entry(*reg).or_default().insert(d);
                }
            }
        }
        let mut work: Vec<usize> = pcs.iter().rev().copied().collect();
        let mut queued: HashSet<usize> = work.iter().copied().collect();
        while let Some(pc) = work.pop() {
            queued.remove(&pc);
            let mut out = reach_in[at[&pc]].clone();
            region.written(&code[pc], &mut |w| {
                out.remove(&w);
            });
            if let Some(&(w, d)) = op_site.get(&pc) {
                out.insert(w, BTreeSet::from([d]));
            }
            for s in region.successors(pc) {
                let Some(&si) = at.get(&s) else {
                    continue;
                };
                let into = &mut reach_in[si];
                let mut changed = false;
                for (reg, ds) in &out {
                    let slot = into.entry(*reg).or_default();
                    for d in ds {
                        changed |= slot.insert(*d);
                    }
                }
                if changed && queued.insert(s) {
                    work.push(s);
                }
            }
        }

        let mut parent: Vec<usize> = (0..sites.len()).collect();
        for &pc in &pcs {
            let mut read = Vec::new();
            region.read(&code[pc], &mut |r| read.push(r));
            for r in read {
                let mut ds: Vec<usize> = reach_in[at[&pc]]
                    .get(&r)
                    .map(|ds| ds.iter().copied().collect())
                    .unwrap_or_default();
                if let Some(&(w, d)) = op_site.get(&pc) {
                    if continues(&code[pc], w, r) {
                        ds.push(d);
                    }
                }
                for pair in ds.windows(2) {
                    let (a, b) = (find(&mut parent, pair[0]), find(&mut parent, pair[1]));
                    parent[a] = b;
                }
            }
        }
        let variable: Vec<usize> = (0..sites.len()).map(|d| find(&mut parent, d)).collect();
        Variables {
            sites,
            variable,
            op_site,
            at,
            reach_in,
        }
    }

    /// Every definition: its index and where it is made.
    pub(crate) fn sites(&self) -> impl Iterator<Item = (usize, Site)> + '_ {
        self.sites.iter().copied().enumerate()
    }

    /// Where definition `site` is made.
    pub(crate) fn site(&self, site: usize) -> Site {
        self.sites[site]
    }

    /// The variable definition `site` makes, named by one of its definitions.
    pub(crate) fn variable(&self, site: usize) -> usize {
        self.variable[site]
    }

    /// The register the op at `pc` defines and its definition, when it defines one.
    pub(crate) fn definition(&self, pc: usize) -> Option<(u16, usize)> {
        self.op_site.get(&pc).copied()
    }

    /// The definitions of register `reg` that reach the op at `pc`.
    pub(crate) fn reaching(&self, pc: usize, reg: u16) -> impl Iterator<Item = usize> + '_ {
        self.at
            .get(&pc)
            .and_then(|i| self.reach_in[*i].get(&reg))
            .into_iter()
            .flatten()
            .copied()
    }

    /// The register a definition defines.
    pub(crate) fn register(&self, site: usize) -> u16 {
        match self.sites[site] {
            Site::Entry(reg) => reg,
            Site::Op(pc) => self.op_site[&pc].0,
        }
    }
}

fn find(parent: &mut [usize], x: usize) -> usize {
    let mut root = x;
    while parent[root] != root {
        root = parent[root];
    }
    let mut at = x;
    while parent[at] != root {
        let next = parent[at];
        parent[at] = root;
        at = next;
    }
    root
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/variables/tests.rs"]
mod tests;
