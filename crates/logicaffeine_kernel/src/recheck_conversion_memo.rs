//! CONVERSION EVIDENCE for one outermost check of the independent kernel.
//!
//! A conversion this kernel has proved — `a ≡ b` in the local context Γ — holds wherever the same
//! pair is compared again in the same context under the same environment: its derivation reads
//! nothing else. An acceptance stands even where a reduction under it was cut short (a common
//! reduct of partial reducts is a common reduct), so no allowance enters the evidence. A refuted
//! pair is never retained: it may be a comparison some allowance cut short. The one negative
//! record is Lean's failure cache (`failed_before`): two applications of one head whose arguments
//! this kernel compared to the end, nothing cut short, and refuted — asked again, the same
//! comparison refutes them again, so the argument comparison is skipped, never the pair's own
//! conversion.
//!
//! Γ is identified by its content, never by an address: each distinct sequence of binder types gets
//! its own number within the session, so the same bodies compared again under the same binders —
//! the motive of every nested `match` of a reflected proof is one such pair — share their
//! evidence, and two contexts that differ in any binder type never do. Every lookup compares the
//! context's types and the pair exactly; a hash only selects the bucket.
//!
//! The session belongs to the environment the check runs in and is consulted for no other. The
//! recurrence probes run in environments of their own (`order_recurrence_context`): they establish
//! the certified accelerations, and an equality proved WITH an acceleration must never be evidence
//! for the probe that certifies it. The lazy loop's differential oracle decides every pair
//! afresh, independently of the loop, and neither reads nor writes the evidence.
use super::*;
use std::hash::{Hash, Hasher};

/// Nodes one side of a retained pair (or one retained binder type) may have; larger ones are
/// compared afresh.
const MAX_SIDE_NODES: usize = 4096;
/// Nodes each store retains before it starts over; the numbered contexts have their own bound.
const MAX_RETAINED_NODES: usize = 1 << 18;
/// Distinct contexts one session numbers.
const MAX_CONTEXTS: u32 = 1 << 14;
/// Nodes of binder types the numbered contexts retain.
const MAX_CONTEXT_NODES: usize = 1 << 17;

struct Session {
    root: usize,
    serial: u64,
    contexts: HashMap<(u32, u64), Vec<(Db, u32)>>,
    next_context: u32,
    context_nodes: usize,
    pairs: Evidence,
    // Applications of one head whose arguments this kernel refuted definitively: comparing those
    // arguments again refutes them again.
    refuted: Evidence,
}

/// One store of term pairs, keyed by context and fingerprints, with the nodes it retains.
#[derive(Default)]
struct Evidence {
    entries: HashMap<(u32, u64, u64), Vec<(Db, Db)>>,
    retained: usize,
}

impl Evidence {
    /// Room for `nodes` more, starting this store over when it is full.
    fn admit(&mut self, nodes: usize) -> bool {
        if self.retained + nodes > MAX_RETAINED_NODES {
            self.entries.clear();
            self.retained = 0;
        }
        if nodes > MAX_RETAINED_NODES {
            return false;
        }
        self.retained += nodes;
        true
    }
}

thread_local! {
    static SESSION: std::cell::RefCell<Option<Session>> = const { std::cell::RefCell::new(None) };
    static SERIAL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Pairs answered from the evidence on this thread.
    pub(super) static HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One outermost check's evidence; the enclosing session (if any) is restored on exit.
#[must_use]
pub(super) struct Scope(Option<Session>);

impl Scope {
    pub(super) fn enter(genv: &Context) -> Scope {
        let serial = SERIAL.with(|s| {
            s.set(s.get() + 1);
            s.get()
        });
        let session = Session {
            root: genv as *const Context as usize,
            serial,
            contexts: HashMap::new(),
            next_context: 1,
            context_nodes: 0,
            pairs: Evidence::default(),
            refuted: Evidence::default(),
        };
        Scope(SESSION.with(|slot| slot.replace(Some(session))))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        SESSION.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// A structural hash of `t`, and its node count, when it has at most `limit` nodes. Equal terms
/// hash equally; unequal terms may collide, and every lookup compares exactly.
fn fingerprint(t: &Db, limit: usize) -> Option<(u64, usize)> {
    let mut h = rustc_hash::FxHasher::default();
    let mut nodes = 0usize;
    let mut pending = vec![t];
    while let Some(t) = pending.pop() {
        nodes += 1;
        if nodes > limit {
            return None;
        }
        std::mem::discriminant(t).hash(&mut h);
        match t {
            Db::Sort(u) => universe(u, &mut h),
            Db::Var(i) => i.hash(&mut h),
            Db::Global(name) => name.hash(&mut h),
            Db::Const { name, levels } => {
                name.hash(&mut h);
                levels.len().hash(&mut h);
                for u in levels {
                    universe(u, &mut h);
                }
            }
            Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
                pending.push(b);
                pending.push(a);
            }
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                cases.len().hash(&mut h);
                pending.extend(cases.iter().rev());
                pending.push(motive);
                pending.push(disc);
            }
            Db::Fix(body) => pending.push(body),
            Db::MutualFix { defs, index } => {
                index.hash(&mut h);
                defs.len().hash(&mut h);
                pending.extend(defs.iter().rev());
            }
            Db::Let(ty, value, body) => {
                pending.push(body);
                pending.push(value);
                pending.push(ty);
            }
            Db::Lit(literal) => match literal {
                Literal::Nat(n) | Literal::BigInt(n) => match n.to_i64() {
                    Some(small) => small.hash(&mut h),
                    None => n.to_le_bytes().hash(&mut h),
                },
                Literal::Int(n) => n.hash(&mut h),
                _ => {}
            },
        }
    }
    Some((h.finish(), nodes))
}

fn universe(u: &Universe, h: &mut rustc_hash::FxHasher) {
    std::mem::discriminant(u).hash(h);
    match u {
        Universe::Var(name) => name.hash(h),
        Universe::Type(n) => n.hash(h),
        Universe::Succ(a) => universe(a, h),
        Universe::Max(a, b) | Universe::IMax(a, b) => {
            universe(a, h);
            universe(b, h);
        }
        Universe::Prop | Universe::SProp => {}
    }
}

impl Session {
    /// The number of the context `parent` extended by one binder of type `ty`.
    fn extend(&mut self, parent: u32, ty: &Db) -> Option<u32> {
        let (hash, nodes) = fingerprint(ty, MAX_SIDE_NODES)?;
        let bucket = self.contexts.entry((parent, hash)).or_default();
        if let Some((_, id)) = bucket.iter().find(|(known, _)| known == ty) {
            return Some(*id);
        }
        if self.next_context >= MAX_CONTEXTS || self.context_nodes + nodes > MAX_CONTEXT_NODES {
            return None;
        }
        let id = self.next_context;
        self.next_context += 1;
        self.context_nodes += nodes;
        bucket.push((ty.clone(), id));
        Some(id)
    }

    /// The number of `lctx`'s binder types, level by level. A borrowed extension remembers its own
    /// number for this session; any other context is numbered from its bindings.
    fn context(&mut self, lctx: &dyn LocalTypes) -> Option<u32> {
        if let Some(cell) = lctx.evidence() {
            if let Some((serial, id)) = cell.get() {
                if serial == self.serial {
                    return Some(id);
                }
            }
        }
        let (parent, bindings) = lctx.parts();
        let mut id = match parent {
            Some(parent) => self.context(parent)?,
            None => 0,
        };
        for ty in bindings {
            id = self.extend(id, ty)?;
        }
        if let Some(cell) = lctx.evidence() {
            cell.set(Some((self.serial, id)));
        }
        Some(id)
    }
}

/// Whether the evidence is kept for this comparison's environment.
fn active(session: &Session, genv: &Context) -> bool {
    session.root == genv as *const Context as usize
        && !genv.checking_order_recurrence
        && !lazy_delta::oracle_running()
}

/// Which store a lookup reads.
#[derive(Clone, Copy)]
enum Store {
    /// `a ≡ b`, proved.
    Proved,
    /// `a` and `b` apply one head to arguments refuted definitively.
    RefutedArguments,
}

impl Session {
    fn store(&mut self, store: Store) -> &mut Evidence {
        match store {
            Store::Proved => &mut self.pairs,
            Store::RefutedArguments => &mut self.refuted,
        }
    }
}

fn lookup(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db, store: Store) -> bool {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(session) = slot.as_mut().filter(|session| active(session, genv)) else {
            return false;
        };
        let (Some((ha, _)), Some((hb, _))) = (
            fingerprint(a, MAX_SIDE_NODES),
            fingerprint(b, MAX_SIDE_NODES),
        ) else {
            return false;
        };
        let Some(context) = session.context(lctx) else {
            return false;
        };
        let pairs = &session.store(store).entries;
        let held = |x: u64, y: u64, l: &Db, r: &Db| {
            pairs
                .get(&(context, x, y))
                .is_some_and(|bucket| bucket.iter().any(|(p, q)| p == l && q == r))
        };
        held(ha, hb, a, b) || held(hb, ha, b, a)
    })
}

fn retain(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db, store: Store) {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(session) = slot.as_mut().filter(|session| active(session, genv)) else {
            return;
        };
        let (Some((ha, na)), Some((hb, nb))) = (
            fingerprint(a, MAX_SIDE_NODES),
            fingerprint(b, MAX_SIDE_NODES),
        ) else {
            return;
        };
        let Some(context) = session.context(lctx) else {
            return;
        };
        let evidence = session.store(store);
        if evidence.admit(na + nb) {
            evidence
                .entries
                .entry((context, ha, hb))
                .or_default()
                .push((a.clone(), b.clone()));
        }
    })
}

/// Whether `a ≡ b` in `lctx` was already proved in this check.
pub(super) fn known(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    let found = lookup(genv, lctx, a, b, Store::Proved);
    #[cfg(test)]
    if found {
        HITS.with(|n| n.set(n.get() + 1));
    }
    found
}

/// Retain the proof that `a ≡ b` in `lctx`.
pub(super) fn record(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) {
    retain(genv, lctx, a, b, Store::Proved)
}

/// Whether the arguments of `a` and `b`, two applications of one head, were already refuted
/// definitively in `lctx` in this check — Lean's `failed_before`: they are not compared again.
pub(super) fn arguments_refuted(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    lookup(genv, lctx, a, b, Store::RefutedArguments)
}

/// Retain that the arguments of `a` and `b` do not convert, as a comparison that nothing cut
/// short concluded — Lean's `cache_failure`.
pub(super) fn record_refuted_arguments(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) {
    retain(genv, lctx, a, b, Store::RefutedArguments)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/conversion_memo/controls.rs"]
mod controls;
