//! Positive conversion evidence scoped to one borrowed, immutable context.
use crate::term_store::{DefEqCache, TermId, TermStore};
use crate::{Context, Literal, Term, Universe};
use std::cell::RefCell;
const MAX_PAIR_NODES: usize = 4096;
const MAX_DEPTH: usize = 128;
const MAX_PAYLOAD: usize = 262_144;
const MAX_STORE_NODES: usize = 65_536;
const MAX_STORE_PAYLOAD: usize = 8 * 1024 * 1024;
const MAX_PAIRS: usize = 4096;
struct Session {
    root: usize,
    store: TermStore,
    pairs: DefEqCache,
    payload: usize,
}
impl Session {
    fn new(ctx: &Context) -> Self {
        Self {
            root: ctx as *const Context as usize,
            store: TermStore::new(),
            pairs: DefEqCache::new(),
            payload: 0,
        }
    }
    fn key(&mut self, a: &Term, b: &Term) -> Option<(TermId, TermId)> {
        let (nodes, bytes) = admit(a, b)?;
        if self.store.len() + nodes > MAX_STORE_NODES
            || self.pairs.len() >= MAX_PAIRS
            || self.payload + bytes > MAX_STORE_PAYLOAD
        {
            self.store = TermStore::new();
            self.pairs = DefEqCache::new();
            self.payload = 0;
        }
        // Charge conservatively even when interning finds an existing node.
        self.payload += bytes;
        let a = self.store.intern(a);
        let b = self.store.intern(b);
        (self.store.eq_faithful(a) && self.store.eq_faithful(b)).then_some((a, b))
    }
}
thread_local! {static ACTIVE:RefCell<Option<Session>>=const {RefCell::new(None)};}
struct Restore(Option<Session>);
impl Drop for Restore {
    fn drop(&mut self) {
        ACTIVE.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// The root remains immutably borrowed for the whole callback. Its address is
/// only an identity token, never dereferenced; other contexts and thread stacks
/// cannot reuse evidence. Nested scopes restore the outer session on unwind.
pub(crate) fn scope<R>(ctx: &Context, f: impl FnOnce() -> R) -> R {
    let _restore = Restore(ACTIVE.with(|slot| slot.replace(Some(Session::new(ctx)))));
    f()
}
pub(crate) fn known(ctx: &Context, a: &Term, b: &Term) -> bool {
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(session) = slot
            .as_mut()
            .filter(|s| s.root == ctx as *const Context as usize)
        else {
            return false;
        };
        let Some((a, b)) = session.key(a, b) else {
            return false;
        };
        session.pairs.get(a, b) == Some(true)
    })
}
fn record(ctx: &Context, a: &Term, b: &Term) {
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(session) = slot
            .as_mut()
            .filter(|s| s.root == ctx as *const Context as usize)
        else {
            return;
        };
        if let Some((a, b)) = session.key(a, b) {
            session.pairs.insert(a, b, true);
        }
    });
}
#[cfg(test)]
thread_local! {static COMPUTATIONS:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};}
pub(crate) fn convert(ctx: &Context, a: &Term, b: &Term, f: impl FnOnce() -> bool) -> bool {
    if known(ctx, a, b) {
        return true;
    }
    #[cfg(test)]
    COMPUTATIONS.with(|n| n.set(n.get() + 1));
    let result = f();
    if result {
        record(ctx, a, b);
    }
    result
}

// Admission precedes interning/copying, including wide vectors, names and
// universe payloads. Cache-ineligible inputs still use ordinary conversion.
/// Borrowed syntax/payload admission only; this returns no conversion evidence.
pub(crate) fn admission_metrics(a: &Term, b: &Term) -> Option<(usize, usize)> {
    admit(a, b)
}

fn admit(a: &Term, b: &Term) -> Option<(usize, usize)> {
    enum Visit<'a> {
        Term(&'a Term, usize),
        Level(&'a Universe, usize),
    }
    let mut pending = vec![Visit::Term(a, 0), Visit::Term(b, 0)];
    let mut nodes = 0;
    let mut bytes = 0usize;
    fn name(bytes: &mut usize, n: &str) -> Option<()> {
        *bytes = bytes.checked_add(n.len())?;
        (*bytes <= MAX_PAYLOAD).then_some(())
    }
    while let Some(item) = pending.pop() {
        nodes += 1;
        if nodes > MAX_PAIR_NODES {
            return None;
        }
        match item {
            Visit::Level(u, d) => {
                if d > MAX_DEPTH {
                    return None;
                }
                match u {
                    Universe::Var(n) => name(&mut bytes, n)?,
                    Universe::Succ(a) => pending.push(Visit::Level(a, d + 1)),
                    Universe::Max(a, b) | Universe::IMax(a, b) => {
                        pending.push(Visit::Level(a, d + 1));
                        pending.push(Visit::Level(b, d + 1));
                    }
                    Universe::SProp | Universe::Prop | Universe::Type(_) => {}
                }
            }
            Visit::Term(t, d) => {
                if d > MAX_DEPTH {
                    return None;
                }
                match t {
                    Term::Sort(u) => pending.push(Visit::Level(u, d + 1)),
                    Term::Var(n) | Term::Global(n) => name(&mut bytes, n)?,
                    Term::Const { name: n, levels } => {
                        name(&mut bytes, n)?;
                        if levels.len() > MAX_PAIR_NODES - pending.len() {
                            return None;
                        }
                        pending.extend(levels.iter().map(|u| Visit::Level(u, d + 1)));
                    }
                    Term::App(a, b) => {
                        pending.push(Visit::Term(a, d + 1));
                        pending.push(Visit::Term(b, d + 1));
                    }
                    Term::Pi {
                        param,
                        param_type,
                        body_type,
                    } => {
                        name(&mut bytes, param)?;
                        pending.push(Visit::Term(param_type, d + 1));
                        pending.push(Visit::Term(body_type, d + 1));
                    }
                    Term::Lambda {
                        param,
                        param_type,
                        body,
                    } => {
                        name(&mut bytes, param)?;
                        pending.push(Visit::Term(param_type, d + 1));
                        pending.push(Visit::Term(body, d + 1));
                    }
                    Term::Let {
                        name: n,
                        ty,
                        value,
                        body,
                    } => {
                        name(&mut bytes, n)?;
                        pending.push(Visit::Term(ty, d + 1));
                        pending.push(Visit::Term(value, d + 1));
                        pending.push(Visit::Term(body, d + 1));
                    }
                    Term::Fix { name: n, body } => {
                        name(&mut bytes, n)?;
                        pending.push(Visit::Term(body, d + 1));
                    }
                    Term::MutualFix { defs, .. } => {
                        if defs.len() > MAX_PAIR_NODES - pending.len() {
                            return None;
                        }
                        for (n, t) in defs {
                            name(&mut bytes, n)?;
                            pending.push(Visit::Term(t, d + 1));
                        }
                    }
                    Term::Match {
                        discriminant,
                        motive,
                        cases,
                    } => {
                        if cases.len() + 2 > MAX_PAIR_NODES - pending.len() {
                            return None;
                        }
                        pending.push(Visit::Term(discriminant, d + 1));
                        pending.push(Visit::Term(motive, d + 1));
                        pending.extend(cases.iter().map(|t| Visit::Term(t, d + 1)));
                    }
                    Term::Lit(Literal::Text(s)) => name(&mut bytes, s)?,
                    Term::Lit(Literal::Nat(n) | Literal::BigInt(n)) => {
                        n.to_i64()?;
                    }
                    Term::Lit(Literal::Float(_)) | Term::Hole => return None,
                    Term::Annotated { term, .. } => pending.push(Visit::Term(term, d + 1)),
                    Term::Lit(
                        Literal::Int(_)
                        | Literal::Duration(_)
                        | Literal::Date(_)
                        | Literal::Moment(_),
                    ) => {}
                }
            }
        }
        if pending.len() > MAX_PAIR_NODES {
            return None;
        }
    }
    Some((nodes, bytes))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/conversion_session/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/conversion_session/controls.rs"]
mod controls;
