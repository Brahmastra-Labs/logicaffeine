//! `Opt::ProbeFuse`: the map lookups a branch reads through the binding its test already made.
//!
//! `If m contains k:` emitted as `if let Some(__probeN) = m.get(&k) {` holds `m`'s value for `k` in `__probeN`. While
//! that branch is emitted, a read `item k of m` beneath it — the same map, the same key — is `__probeN`, so the test
//! and every read of the key under it are one lookup. The read stays `item k of m` in the program, so everything
//! that reasons about it (the range oracle's element interval, the exactness of the arithmetic around it) sees what
//! it saw. [`Probe::reads`] counts what the binding served; a branch none of whose reads it served keeps its plain
//! test.

use std::cell::RefCell;

use crate::ast::stmt::{Expr, Literal};
use crate::intern::Symbol;

/// A key a probe answers for: a name or a literal, whose value cannot change between the test and a read when the
/// branch writes no name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeKey {
    Name(Symbol),
    Int(i64),
    Text(Symbol),
}

impl ProbeKey {
    /// `e` as a key a probe can answer for.
    pub(crate) fn of(e: &Expr) -> Option<Self> {
        match e {
            Expr::Identifier(s) => Some(ProbeKey::Name(*s)),
            Expr::Literal(Literal::Number(n)) => Some(ProbeKey::Int(*n)),
            Expr::Literal(Literal::Text(t)) => Some(ProbeKey::Text(*t)),
            _ => None,
        }
    }
}

/// One `if let Some(name) = map.get(&key)` binding, while its branch is emitted.
pub(crate) struct Probe {
    pub map: Symbol,
    pub key: ProbeKey,
    pub name: String,
    pub reads: usize,
}

thread_local! {
    /// The probes whose branches are being emitted, innermost last; [`within`] pushes one and pops it.
    static PROBES: RefCell<Vec<Probe>> = const { RefCell::new(Vec::new()) };
    /// The number in the next `__probeN`. Distinct within a program; [`reset_names`] starts every program at 0, so
    /// a program's bindings are named the same whatever was compiled before it.
    static PROBE_ID: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Start the probe bindings' numbering over. Called from `codegen::expr::begin_program_emission`.
pub(crate) fn reset_names() {
    PROBE_ID.with(|n| n.set(0));
}

/// A fresh binding name for a probe.
pub(crate) fn fresh_name() -> String {
    PROBE_ID.with(|n| {
        let k = n.get();
        n.set(k + 1);
        format!("__probe{k}")
    })
}

/// Runs `f` with `probe` answering the reads beneath it, and returns what `f` returned with the reads it served.
pub(crate) fn within<R>(probe: Probe, f: impl FnOnce() -> R) -> (R, usize) {
    PROBES.with(|p| p.borrow_mut().push(probe));
    let out = f();
    let served = PROBES.with(|p| p.borrow_mut().pop().map_or(0, |probe| probe.reads));
    (out, served)
}

/// The binding that answers `item index of collection`, when an enclosing probe holds that map's value for that key.
pub(crate) fn read(collection: &Expr, index: &Expr) -> Option<String> {
    let Expr::Identifier(map) = collection else {
        return None;
    };
    let key = ProbeKey::of(index)?;
    PROBES.with(|p| {
        let mut p = p.borrow_mut();
        let probe = p
            .iter_mut()
            .rev()
            .find(|probe| probe.map == *map && probe.key == key)?;
        probe.reads += 1;
        Some(probe.name.clone())
    })
}
