//! An independent proof-term re-checker (the de Bruijn criterion taken seriously).
//!
//! The kernel's [`infer_type`](crate::infer_type) is the entire trust base. This module
//! is a SECOND, deliberately separate implementation of the CIC type checker: trust then
//! rests on two small independent kernels agreeing, not on one. For the independence to
//! be meaningful the two must not share their checking logic — so this re-checker uses a
//! DIFFERENT term representation (de Bruijn indices, no names) with its own reduction and
//! its own cumulative conversion. A variable-capture or substitution bug in the main
//! kernel's name-based machinery would surface here as a type error rather than slipping
//! through identically — Logos's analog of `lean4checker`/`nanoda`.
//!
//! Scope: the CIC logical core — `Sort`/`Var`/`Global`/`Pi`/`Lambda`/`App`/`Lit` (incl.
//! `BigInt`/`Nat` literal arithmetic and the Nat↔Peano bridge) with β/ι/ζ/δ-reduction and
//! cumulative conversion — PLUS the inductive eliminator `Match` (coverage, per-constructor
//! case typing via de Bruijn telescope instantiation, ι-reduction, large-elimination
//! restriction) AND both single (`Fix`) and MUTUAL (`MutualFix`) fixpoints, each with an
//! INDEPENDENT structural-termination guard. Transparent definitions ARE δ-unfolded during
//! whnf; only axioms/constructors/inductives stay opaque. The only fragment reported
//! HONESTLY as [`ReCheckError::Unsupported`] — rather than silently passed — is `Hole`
//! (an unelaborated metavariable) and a `match` on an inductive this checker cannot
//! resolve: such a proof is single-checked by the main kernel and [`double_check`] says so.
//! The re-checker never reports a term as independently verified unless it actually
//! verified every part of it.

use std::collections::HashMap;

use crate::term::{int_lit, lit_bigint, Literal, Term, Universe};
use crate::Context;
#[path = "recheck_equality.rs"]
mod equality;
#[path = "recheck_storage.rs"]
mod storage;
use storage::DbBox;
#[path = "recheck_definitions.rs"]
mod definitions;

/// Why the re-checker could not produce a verdict, split by soundness meaning.
#[derive(Debug, Clone, PartialEq)]
pub enum ReCheckError {
    /// No verdict: the re-checker did not finish checking the term. NOT a soundness signal: the
    /// term is still fully checked by the main kernel, and [`double_check`] flags it as
    /// single-checked rather than claiming agreement. [`Incomplete::kind`] says why — a construct
    /// outside the covered fragment, or a resource bound that stopped the check.
    Unsupported(Incomplete),
    /// A genuine ill-typedness the re-checker is confident about (a non-function in
    /// application position, a sort/type mismatch, wrong case count, unbound variable).
    /// If the main kernel accepted the same term, the two kernels DISAGREE — an alarm.
    Ill(String),
}

impl ReCheckError {
    fn ill(msg: impl Into<String>) -> Self {
        ReCheckError::Ill(msg.into())
    }
    fn unsupported(msg: impl Into<String>) -> Self {
        ReCheckError::Unsupported(Incomplete {
            kind: IncompleteKind::Unsupported,
            message: msg.into(),
        })
    }
    /// The check was stopped by one of this kernel's resource bounds.
    fn resource_limit(msg: impl Into<String>) -> Self {
        ReCheckError::Unsupported(Incomplete {
            kind: IncompleteKind::ResourceLimit {
                steps: MAX_RECHECK_STEPS,
                depth: MAX_RECHECK_WHNF_DEPTH,
                nodes: MAX_RECHECK_TERM_NODES,
            },
            message: msg.into(),
        })
    }
    /// The incompleteness, when this is not a refutation.
    pub fn incomplete(&self) -> Option<&Incomplete> {
        match self {
            ReCheckError::Unsupported(incomplete) => Some(incomplete),
            ReCheckError::Ill(_) => None,
        }
    }
}

/// Why the independent kernel reached no verdict, and its account of the stop.
///
/// It reads as the message it carries (`Deref<Target = str>`, and `Debug` prints the message as a
/// string), so every existing reading of an incompleteness as text is unchanged; the typed
/// [`IncompleteKind`] is what a consumer decides on.
#[derive(Clone, PartialEq, Eq)]
pub struct Incomplete {
    kind: IncompleteKind,
    message: String,
}

/// What stopped the independent kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncompleteKind {
    /// A construct outside the covered fragment: a `Hole` (an unelaborated metavariable), a
    /// `match` on an inductive whose head this kernel does not recognize, or a reduction rule of
    /// the first kernel that this one does not compute. More resources cannot decide it.
    Unsupported,
    /// One of this kernel's resource bounds stopped the check before it decided: its reduction
    /// step budget, its nesting depth, its term size, its share of the native stack, or its input
    /// admission bounds. The bounds in force are recorded; more of the resource could decide it.
    ResourceLimit {
        steps: usize,
        depth: usize,
        nodes: usize,
    },
}

impl Incomplete {
    fn unsupported(message: impl Into<String>) -> Self {
        Incomplete {
            kind: IncompleteKind::Unsupported,
            message: message.into(),
        }
    }
    /// The same stop, its account prefixed with where it surfaced.
    fn prefixed(&self, prefix: impl std::fmt::Display) -> Self {
        Incomplete {
            kind: self.kind,
            message: format!("{prefix}: {}", self.message),
        }
    }
    pub fn kind(&self) -> IncompleteKind {
        self.kind
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn is_resource_limit(&self) -> bool {
        matches!(self.kind, IncompleteKind::ResourceLimit { .. })
    }
}

impl std::ops::Deref for Incomplete {
    type Target = str;
    fn deref(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for Incomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::fmt::Debug for Incomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.message, f)
    }
}

impl From<Incomplete> for String {
    fn from(incomplete: Incomplete) -> String {
        incomplete.message
    }
}

type RResult<T> = Result<T, ReCheckError>;

/// A locally-nameless term: every bound variable is a de Bruijn INDEX (0 = innermost
/// binder), so α-equivalence is syntactic identity and there are no names to capture.
/// This is the independence-bearing representation — distinct from the main kernel's
/// named [`Term`].
#[derive(Debug, Clone, PartialEq)]
enum Db {
    Sort(Universe),
    /// de Bruijn index of a `λ`/`Π`-bound variable (0 = nearest enclosing binder).
    Var(usize),
    /// A reference into the global environment (inductive, constructor, definition,
    /// or declaration) — kept opaque.
    Global(String),
    /// A universe-polymorphic global at explicit levels — `name.{ℓ…}`.
    Const {
        name: String,
        levels: Vec<Universe>,
    },
    /// `Π(_:dom). body` — `body` is one binder deeper than `dom`.
    Pi(DbBox, DbBox),
    /// `λ(_:dom). body`.
    Lam(DbBox, DbBox),
    App(DbBox, DbBox),
    /// `match disc return motive with cases` — the node binds nothing itself; each case
    /// and the motive are `λ`s that introduce their own binders.
    Match {
        disc: DbBox,
        motive: DbBox,
        cases: Vec<Db>,
    },
    /// `fix rec. body` — binds the recursive self-reference (index 0 inside `body`).
    Fix(FixBody),
    /// `mutualfix { b₀, …, b_{n-1} }.index` — a block of `n` mutually-recursive bodies.
    /// ALL `n` names bind in EVERY body: entering a body pushes `n` levels, with def `j`
    /// at level `base + j` (`base` = enclosing depth). The node reduces to the
    /// `index`-th body once an argument is constructor-headed.
    MutualFix {
        defs: Vec<Db>,
        index: usize,
    },
    /// `let _:ty := value in body` — `body` is one binder deeper than `ty`/`value`.
    Let(DbBox, DbBox, DbBox),
    Lit(Literal),
}

// Optional immutable Fix bodies. No semantic reduction or typing certificate.
#[derive(Clone, Copy, Default)]
struct FixFactory {
    scope: usize,
    started: bool,
    remaining: usize,
    spent: usize,
}
thread_local! {
    static FIX_FACTORY:std::cell::Cell<FixFactory> = const {
        std::cell::Cell::new(FixFactory {scope:0,started:false,remaining:0,spent:0})
    };
    // This ledger is NEVER restored/reset with a prefix scope: Rc owners may escape it.
    static FIX_SHARED_LIVE:std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
}
#[cfg(test)]
thread_local! {static FIX_BODY_STATS:std::cell::Cell<[usize;6]> = const {std::cell::Cell::new([0;6])};}
fn fix_body_stat(i: usize) {
    #[cfg(test)]
    FIX_BODY_STATS.with(|s| {
        let mut v = s.get();
        v[i] += 1;
        s.set(v);
    });
    #[cfg(not(test))]
    let _ = i;
}
fn prefix_combined_fits(prefix: usize) -> bool {
    FIX_SHARED_LIVE.with(|s| {
        prefix
            .checked_add(s.get())
            .is_some_and(|n| n <= PREFIX_LIVE_BYTES)
    })
}
fn fix_factory_begin() -> bool {
    FIX_FACTORY.with(|s| {
        let mut f = s.get();
        if f.scope == 0 || f.started {
            return false;
        }
        f.started = true;
        f.remaining = PREFIX_ADMISSION_WORK;
        f.spent = 0;
        s.set(f);
        true
    })
}
fn fix_factory_spend(n: usize) -> Option<()> {
    let f = FIX_FACTORY.with(|s| s.get());
    if !f.started || f.scope == 0 || f.remaining < n {
        return None;
    }
    if recheck_budget_active() {
        if !prefix_work_available(n) || !nat_order_spend(n) {
            return None;
        }
    }
    FIX_FACTORY.with(|s| {
        let mut current = s.get();
        if current.scope != f.scope || !current.started {
            return None;
        }
        current.remaining = current.remaining.checked_sub(n)?;
        current.spent = current.spent.checked_add(n)?;
        s.set(current);
        Some(())
    })
}
struct FixSharedTicket {
    bytes: usize,
}
impl FixSharedTicket {
    fn new(bytes: usize) -> Option<Self> {
        FIX_SHARED_LIVE.with(|s| {
            let live = s.get().checked_add(bytes)?;
            let prefix = PREFIX_MEMORY.with(|p| p.get().live);
            if live.checked_add(prefix)? > PREFIX_LIVE_BYTES {
                return None;
            }
            s.set(live);
            Some(Self { bytes })
        })
    }
}
impl Drop for FixSharedTicket {
    fn drop(&mut self) {
        FIX_SHARED_LIVE.with(|s| {
            s.set(
                s.get()
                    .checked_sub(self.bytes)
                    .expect("each shared allocation releases its own reservation once"),
            )
        });
    }
}
#[derive(Clone, Copy, Debug)]
struct FixFacts {
    metrics: PrefixMetrics,
    required: usize,
}
struct AdmittedFixBody {
    body: Db,
    facts: FixFacts,
    generation: usize,
    _memory: FixSharedTicket,
}
enum FixBody {
    Owned(Box<Db>),
    Shared(std::rc::Rc<AdmittedFixBody>),
}
impl std::fmt::Debug for FixBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_ref().fmt(f)
    }
}
#[cfg(test)]
thread_local! {
    static OWNED_FIX_CLONES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
impl Clone for FixBody {
    fn clone(&self) -> Self {
        match self {
            Self::Owned(b) => {
                #[cfg(test)]
                OWNED_FIX_CLONES.with(|c| {
                    if let Some(n) = c.get() {
                        c.set(Some(n + 1));
                    }
                });
                Self::Owned(b.clone())
            }
            Self::Shared(b) => {
                fix_body_stat(1);
                Self::Shared(b.clone())
            }
        }
    }
}
impl PartialEq for FixBody {
    fn eq(&self, other: &Self) -> bool {
        self.same_current(other) || self.as_ref() == other.as_ref()
    }
}
impl std::ops::Deref for FixBody {
    type Target = Db;
    fn deref(&self) -> &Db {
        self.as_ref()
    }
}
impl AsRef<Db> for FixBody {
    fn as_ref(&self) -> &Db {
        match self {
            Self::Owned(b) => b,
            Self::Shared(b) => &b.body,
        }
    }
}
impl FixBody {
    fn into_db(self) -> Db {
        match self {
            Self::Owned(body) => *body,
            Self::Shared(body) => match std::rc::Rc::try_unwrap(body) {
                Ok(body) => body.body,
                Err(body) => body.body.clone(),
            },
        }
    }
    fn owned(body: Box<Db>) -> Self {
        Self::Owned(body)
    }
    fn new(body: Db) -> Self {
        let Some(facts) = fix_facts(&body) else {
            return Self::Owned(Box::new(body));
        };
        if facts.required > 1 {
            return Self::Owned(Box::new(body));
        }
        // Each immutable allocation reserves only its exclusive storage;
        // nested shared children keep their own persistent reservations.
        let memory =
            storage::exclusive_retained_bytes(&body, std::mem::size_of::<AdmittedFixBody>())
                .and_then(FixSharedTicket::new);
        let Some(memory) = memory else {
            return Self::Owned(Box::new(body));
        };
        let generation = FIX_FACTORY.with(|s| s.get().scope);
        fix_body_stat(0);
        Self::Shared(std::rc::Rc::new(AdmittedFixBody {
            body,
            facts,
            generation,
            _memory: memory,
        }))
    }
    fn facts(&self) -> Option<FixFacts> {
        let Self::Shared(b) = self else {
            return None;
        };
        let f = FIX_FACTORY.with(|s| s.get());
        (f.started && f.scope != 0 && f.scope == b.generation).then_some(b.facts)
    }
    fn same_current(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Shared(a), Self::Shared(b)) => {
                std::rc::Rc::ptr_eq(a, b) && self.facts().is_some()
            }
            _ => false,
        }
    }
    fn shifted(&self, delta: isize, cutoff: usize) -> Self {
        // Zero shift is unconditional syntax identity, including after the
        // admission generation expires. It needs no new admission facts.
        if delta == 0 {
            #[cfg(test)]
            if let Self::Owned(body) = self {
                beta_count_clone(body);
            }
            return self.clone();
        }
        if self.facts().is_some_and(|s| s.required <= cutoff) {
            return self.clone();
        }
        Self::new(shift(self, delta, cutoff))
    }
    fn opened(&self, arg: &Db, depth: usize) -> Self {
        if self.facts().is_some_and(|s| s.required <= depth) {
            return self.clone();
        }
        Self::new(beta_open_at(self, arg, depth))
    }
}
fn fix_facts(t: &Db) -> Option<FixFacts> {
    fn bounded(s: &FixFacts) -> bool {
        let m = s.metrics;
        m.nodes <= PREFIX_NODES
            && m.levels <= PREFIX_NODES
            && m.depth <= PREFIX_DEPTH
            && m.payload <= 16384
            && m.bytes <= PREFIX_TREE_BYTES
    }
    fn payload(m: &mut PrefixMetrics, s: &String) -> Option<()> {
        if s.len() > 256 {
            return None;
        }
        fix_factory_spend(s.len().checked_add(1)?)?;
        m.payload = m.payload.checked_add(s.len())?;
        m.bytes = m.bytes.checked_add(s.capacity())?;
        Some(())
    }
    fn level(m: &mut PrefixMetrics, u: &Universe, depth: usize) -> Option<()> {
        fix_factory_spend(1)?;
        if depth > PREFIX_DEPTH {
            return None;
        }
        m.levels = m.levels.checked_add(1)?;
        if m.levels > PREFIX_NODES {
            return None;
        }
        m.depth = m.depth.max(depth);
        match u {
            Universe::SProp | Universe::Prop | Universe::Type(_) => {}
            Universe::Var(n) => payload(m, n)?,
            Universe::Succ(a) => {
                m.bytes = m.bytes.checked_add(std::mem::size_of::<Universe>())?;
                level(m, a, depth + 1)?;
            }
            Universe::Max(a, b) | Universe::IMax(a, b) => {
                m.bytes = m.bytes.checked_add(2 * std::mem::size_of::<Universe>())?;
                level(m, a, depth + 1)?;
                level(m, b, depth + 1)?;
            }
        }
        (m.payload <= 16384 && m.bytes <= PREFIX_TREE_BYTES).then_some(())
    }
    fn merge(out: &mut FixFacts, c: FixFacts, binders: usize, boxed: bool) -> Option<()> {
        out.metrics.nodes = out.metrics.nodes.checked_add(c.metrics.nodes)?;
        out.metrics.levels = out.metrics.levels.checked_add(c.metrics.levels)?;
        out.metrics.depth = out.metrics.depth.max(c.metrics.depth.checked_add(1)?);
        out.metrics.payload = out.metrics.payload.checked_add(c.metrics.payload)?;
        let bytes = if boxed {
            c.metrics.bytes
        } else {
            c.metrics.bytes.checked_sub(std::mem::size_of::<Db>())?
        };
        out.metrics.bytes = out.metrics.bytes.checked_add(bytes)?;
        out.required = out.required.max(c.required.saturating_sub(binders));
        bounded(out).then_some(())
    }
    fn child(t: &DbBox, depth: usize) -> Option<FixFacts> {
        if let Some(facts) = t.facts() {
            fix_factory_spend(1)?;
            if depth.checked_sub(1)?.checked_add(facts.metrics.depth)? > PREFIX_DEPTH {
                return None;
            }
            Some(facts)
        } else {
            walk(t, depth)
        }
    }
    fn walk(t: &Db, depth: usize) -> Option<FixFacts> {
        fix_factory_spend(1)?;
        fix_body_stat(4);
        if depth > PREFIX_DEPTH {
            return None;
        }
        let mut out = FixFacts {
            metrics: PrefixMetrics {
                nodes: 1,
                depth: 1,
                bytes: std::mem::size_of::<Db>(),
                ..PrefixMetrics::default()
            },
            required: 0,
        };
        match t {
            Db::Var(i) => out.required = i.checked_add(1)?,
            Db::Global(n) => payload(&mut out.metrics, n)?,
            Db::Const { name, levels } => {
                if levels.len() > PREFIX_NODES {
                    return None;
                }
                payload(&mut out.metrics, name)?;
                out.metrics.bytes = out.metrics.bytes.checked_add(
                    levels
                        .capacity()
                        .checked_mul(std::mem::size_of::<Universe>())?,
                )?;
                for u in levels {
                    level(&mut out.metrics, u, 2)?;
                }
            }
            Db::Sort(u) => level(&mut out.metrics, u, 2)?,
            Db::Lit(Literal::Float(_)) => return None,
            Db::Lit(Literal::Text(s)) => payload(&mut out.metrics, s)?,
            Db::Lit(Literal::Nat(n)) => {
                if n.is_negative() || n.to_i64().is_none() {
                    return None;
                }
            }
            Db::Lit(Literal::BigInt(n)) => {
                n.to_i64()?;
            }
            Db::Lit(_) => {}
            Db::Pi(a, b) | Db::Lam(a, b) => {
                merge(&mut out, child(a, depth + 1)?, 0, true)?;
                merge(&mut out, child(b, depth + 1)?, 1, true)?;
            }
            Db::App(a, b) => {
                merge(&mut out, child(a, depth + 1)?, 0, true)?;
                merge(&mut out, child(b, depth + 1)?, 0, true)?;
            }
            Db::Fix(b) => {
                let facts = if let Some(f) = b.facts() {
                    fix_factory_spend(1)?;
                    f
                } else {
                    walk(b, depth + 1)?
                };
                merge(&mut out, facts, 1, true)?;
            }
            Db::Let(a, b, c) => {
                merge(&mut out, child(a, depth + 1)?, 0, true)?;
                merge(&mut out, child(b, depth + 1)?, 0, true)?;
                merge(&mut out, child(c, depth + 1)?, 1, true)?;
            }
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                if cases.len() > PREFIX_NODES {
                    return None;
                }
                out.metrics.bytes = out
                    .metrics
                    .bytes
                    .checked_add(cases.capacity().checked_mul(std::mem::size_of::<Db>())?)?;
                merge(&mut out, child(disc, depth + 1)?, 0, true)?;
                merge(&mut out, child(motive, depth + 1)?, 0, true)?;
                for c in cases {
                    merge(&mut out, walk(c, depth + 1)?, 0, false)?;
                }
            }
            Db::MutualFix { defs, .. } => {
                if defs.len() > PREFIX_NODES {
                    return None;
                }
                out.metrics.bytes = out
                    .metrics
                    .bytes
                    .checked_add(defs.capacity().checked_mul(std::mem::size_of::<Db>())?)?;
                for c in defs {
                    merge(&mut out, walk(c, depth + 1)?, defs.len(), false)?;
                }
            }
        }
        if !bounded(&out) {
            return None;
        }
        out.metrics.closed = out.required == 0;
        Some(out)
    }
    walk(t, 1)
}

// ---------------------------------------------------------------------------
// Named `Term` ⟷ de Bruijn `Db`
// ---------------------------------------------------------------------------

/// Lower a named [`Term`] to a de Bruijn [`Db`], resolving each `Var` to the index of
/// its nearest enclosing binder. `scope` holds the binder names, innermost LAST. A
/// `Var` with no matching binder is a free local — illegal in a closed certificate
/// term, so it is rejected. `Fix`/`Hole` are outside the covered fragment.
fn to_db(term: &Term, scope: &mut Vec<String>) -> RResult<Db> {
    match term {
        Term::Sort(u) => Ok(Db::Sort(u.clone())),
        Term::Var(name) => {
            for (depth_from_inner, bound) in scope.iter().rev().enumerate() {
                if bound == name {
                    return Ok(Db::Var(depth_from_inner));
                }
            }
            // A `?`-named free variable is an elaborator METAVARIABLE that was never solved, not
            // a typo — a different event needing a different repair. The second kernel reaches
            // the same diagnosis independently (from the shared convention, not from the first
            // kernel's answer), so `double_check` still reports agreement on the REJECTION.
            Err(ReCheckError::ill(
                crate::type_checker::unbound_diagnosis(name)
                    .unwrap_or_else(|| format!("unbound local variable '{}'", name)),
            ))
        }
        Term::Global(name) => Ok(Db::Global(name.clone())),
        Term::Const { name, levels } => Ok(Db::Const {
            name: name.clone(),
            levels: levels.clone(),
        }),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let dom = to_db(param_type, scope)?;
            scope.push(param.clone());
            let body = to_db(body_type, scope);
            scope.pop();
            Ok(Db::Pi(DbBox::new(dom), DbBox::new(body?)))
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let dom = to_db(param_type, scope)?;
            scope.push(param.clone());
            let inner = to_db(body, scope);
            scope.pop();
            Ok(Db::Lam(DbBox::new(dom), DbBox::new(inner?)))
        }
        Term::App(f, a) => Ok(Db::App(
            DbBox::new(to_db(f, scope)?),
            DbBox::new(to_db(a, scope)?),
        )),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let disc = to_db(discriminant, scope)?;
            let mot = to_db(motive, scope)?;
            let cs = cases
                .iter()
                .map(|c| to_db(c, scope))
                .collect::<RResult<Vec<_>>>()?;
            Ok(Db::Match {
                disc: DbBox::new(disc),
                motive: DbBox::new(mot),
                cases: cs,
            })
        }
        Term::Fix { name, body } => {
            scope.push(name.clone());
            let b = to_db(body, scope);
            scope.pop();
            Ok(Db::Fix(FixBody::new(b?)))
        }
        // Mutual fixpoint: all `n` names bind in every body, so push all `n` before
        // converting any body (innermost-last, so def `j` sits at level `base + j`).
        Term::MutualFix { defs, index } => {
            let names: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
            for n in &names {
                scope.push(n.clone());
            }
            let mut dbs = Vec::with_capacity(defs.len());
            let mut err = None;
            for (_, body) in defs {
                match to_db(body, scope) {
                    Ok(d) => dbs.push(d),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }
            for _ in &names {
                scope.pop();
            }
            match err {
                Some(e) => Err(e),
                None => Ok(Db::MutualFix {
                    defs: dbs,
                    index: *index,
                }),
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let d_ty = to_db(ty, scope)?;
            let d_val = to_db(value, scope)?;
            scope.push(name.clone());
            let d_body = to_db(body, scope);
            scope.pop();
            Ok(Db::Let(
                DbBox::new(d_ty),
                DbBox::new(d_val),
                DbBox::new(d_body?),
            ))
        }
        Term::Lit(l) => Ok(Db::Lit(l.clone())),
        Term::Hole => Err(ReCheckError::unsupported("Hole (unelaborated implicit)")),
        // `Db` is kernel meaning and has no annotation: this kernel erases its own, at the one
        // boundary every term it checks crosses.
        Term::Annotated { term, .. } => to_db(term, scope),
    }
}

/// Raise a de Bruijn [`Db`] back to a named [`Term`], naming binders `v{depth}` by their
/// binding depth (used only to report an inferred type back; comparisons stay in
/// de Bruijn).
fn from_db(t: &Db, depth: usize) -> Term {
    from_db_with_external(t, depth, &[])
}

// External levels retain their exact names. Internal names use the existing vN
// spelling unless it could capture an external local; distinct levels still
// have distinct numeric stems. Own domains are raised before entering a binder.
fn raised_name(level: usize, external: &[String]) -> String {
    if let Some(name) = external.get(level) {
        return name.clone();
    }
    let mut name = format!("v{level}");
    while external.iter().any(|outer| outer == &name) {
        name.push('_');
    }
    name
}

#[cfg(test)]
thread_local! {
    static RAISED_DB_NODES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
fn from_db_with_external(t: &Db, depth: usize, external: &[String]) -> Term {
    #[cfg(test)]
    RAISED_DB_NODES.with(|c| {
        if let Some(n) = c.get() {
            c.set(Some(n + 1));
        }
    });
    match t {
        Db::Sort(u) => Term::Sort(u.clone()),
        Db::Var(k) => {
            let binder = depth.saturating_sub(1).saturating_sub(*k);
            Term::Var(raised_name(binder, external))
        }
        Db::Global(n) => Term::Global(n.clone()),
        Db::Const { name, levels } => Term::Const {
            name: name.clone(),
            levels: levels.clone(),
        },
        Db::Pi(a, b) => Term::Pi {
            param: raised_name(depth, external),
            param_type: Box::new(from_db_with_external(a, depth, external)),
            body_type: Box::new(from_db_with_external(b, depth + 1, external)),
        },
        Db::Lam(a, b) => Term::Lambda {
            param: raised_name(depth, external),
            param_type: Box::new(from_db_with_external(a, depth, external)),
            body: Box::new(from_db_with_external(b, depth + 1, external)),
        },
        Db::App(f, a) => Term::App(
            Box::new(from_db_with_external(f, depth, external)),
            Box::new(from_db_with_external(a, depth, external)),
        ),
        Db::Match {
            disc,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(from_db_with_external(disc, depth, external)),
            motive: Box::new(from_db_with_external(motive, depth, external)),
            cases: cases
                .iter()
                .map(|c| from_db_with_external(c, depth, external))
                .collect(),
        },
        // A `Fix` binder occupies ONE de Bruijn level, exactly as a `λ` does — `to_db` pushes
        // its name onto the same scope stack — so it must be named by the SAME scheme its own
        // bound occurrences get. Naming it anything else (it used to be `rec{depth}`) leaves
        // the recursive self-reference reconstructed as `v{depth}`, a variable NOTHING binds:
        // `to_db` then refuses the round trip, and `double_check` reports "inferred type
        // outside re-checker fragment" for every type that mentions a fixpoint.
        Db::Fix(body) => Term::Fix {
            name: raised_name(depth, external),
            body: Box::new(from_db_with_external(body, depth + 1, external)),
        },
        // The same rule for a mutual block: all `n` names bind in every body, innermost-LAST,
        // so def `j` sits at level `depth + j` and its occurrences reconstruct as `v{depth+j}`.
        Db::MutualFix { defs, index } => {
            let n = defs.len();
            Term::MutualFix {
                defs: defs
                    .iter()
                    .enumerate()
                    .map(|(j, b)| {
                        (
                            raised_name(depth + j, external),
                            from_db_with_external(b, depth + n, external),
                        )
                    })
                    .collect(),
                index: *index,
            }
        }
        Db::Let(ty, value, body) => Term::Let {
            name: raised_name(depth, external),
            ty: Box::new(from_db_with_external(ty, depth, external)),
            value: Box::new(from_db_with_external(value, depth, external)),
            body: Box::new(from_db_with_external(body, depth + 1, external)),
        },
        Db::Lit(l) => Term::Lit(l.clone()),
    }
}

// ---------------------------------------------------------------------------
// de Bruijn shifting and substitution (the capture-free core)
// ---------------------------------------------------------------------------

#[cfg(test)]
thread_local! {
    // Counts visited source nodes and constructed output Db nodes, including subtree clones.
    static BETA_WORK: std::cell::Cell<Option<(usize, usize)>> = const {
        std::cell::Cell::new(None)
    };
    static MOTIVE_CHECKS: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
    static K_PROBE_ATTEMPTS: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
fn beta_count_work(constructed: usize) {
    BETA_WORK.with(|work| {
        if let Some((visits, copies)) = work.get() {
            work.set(Some((visits + 1, copies + constructed)));
        }
    });
}

/// Account for actual Db construction by Clone, not unfolded logical size.
/// This extra walk exists only while a test explicitly enables BETA_WORK.
#[cfg(test)]
fn beta_count_clone(t: &Db) {
    fn edge(t: &DbBox) -> usize {
        match t {
            DbBox::Owned(t) => count(t),
            DbBox::Shared(_) => 0,
        }
    }
    fn count(t: &Db) -> usize {
        1 + match t {
            Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => edge(a) + edge(b),
            Db::Let(a, b, c) => edge(a) + edge(b) + edge(c),
            Db::Match {
                disc,
                motive,
                cases,
            } => edge(disc) + edge(motive) + cases.iter().map(count).sum::<usize>(),
            Db::Fix(body) => match body {
                FixBody::Owned(t) => count(t),
                FixBody::Shared(_) => 0,
            },
            Db::MutualFix { defs, .. } => defs.iter().map(count).sum(),
            _ => 0,
        }
    }
    BETA_WORK.with(|work| {
        if let Some((visits, copies)) = work.get() {
            let nodes = count(t);
            work.set(Some((visits + nodes, copies + nodes)));
        }
    });
}

#[cfg(test)]
fn db_node_count(t: &Db) -> usize {
    match t {
        Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => 1 + db_node_count(a) + db_node_count(b),
        Db::Let(a, b, c) => 1 + db_node_count(a) + db_node_count(b) + db_node_count(c),
        Db::Fix(b) => 1 + db_node_count(b),
        Db::MutualFix { defs, .. } => 1 + defs.iter().map(db_node_count).sum::<usize>(),
        Db::Match {
            disc,
            motive,
            cases,
        } => {
            1 + db_node_count(disc)
                + db_node_count(motive)
                + cases.iter().map(db_node_count).sum::<usize>()
        }
        _ => 1,
    }
}

/// Shift every free index `≥ cutoff` by `d` (which may be negative). Binders raise the
/// cutoff so bound indices are untouched (the classic TAPL shift).
fn shift(t: &Db, d: isize, cutoff: usize) -> Db {
    // No binder or index can change when the offset is zero. Clone retains
    // immutable child allocations without consulting their admission epoch.
    if d == 0 {
        #[cfg(test)]
        beta_count_clone(t);
        return t.clone();
    }
    #[cfg(test)]
    beta_count_work(1);
    match t {
        Db::Var(k) => {
            if *k >= cutoff {
                Db::Var((*k as isize + d) as usize)
            } else {
                Db::Var(*k)
            }
        }
        Db::Pi(a, b) => Db::Pi(a.shifted(d, cutoff), b.shifted(d, cutoff + 1)),
        Db::Lam(a, b) => Db::Lam(a.shifted(d, cutoff), b.shifted(d, cutoff + 1)),
        Db::App(f, a) => Db::App(f.shifted(d, cutoff), a.shifted(d, cutoff)),
        Db::Match {
            disc,
            motive,
            cases,
        } => Db::Match {
            disc: disc.shifted(d, cutoff),
            motive: motive.shifted(d, cutoff),
            cases: cases.iter().map(|c| shift(c, d, cutoff)).collect(),
        },
        Db::Fix(body) => Db::Fix(body.shifted(d, cutoff + 1)),
        Db::MutualFix { defs, index } => Db::MutualFix {
            // Every body lives beneath ALL `n` mutual binders, so the cutoff rises by `n`.
            defs: defs
                .iter()
                .map(|b| shift(b, d, cutoff + defs.len()))
                .collect(),
            index: *index,
        },
        Db::Let(ty, value, body) => Db::Let(
            ty.shifted(d, cutoff),
            value.shifted(d, cutoff),
            body.shifted(d, cutoff + 1),
        ),
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => t.clone(),
    }
}

/// Substitute the variable with index `j` by `s` (capture-free: `s` is shifted as it
/// descends under binders, `j` rises in step).
#[cfg(test)]
fn subst(t: &Db, j: usize, s: &Db) -> Db {
    #[cfg(test)]
    beta_count_work(if matches!(t, Db::Var(k) if *k == j) {
        db_node_count(s)
    } else {
        1
    });
    match t {
        Db::Var(k) => {
            if *k == j {
                s.clone()
            } else {
                Db::Var(*k)
            }
        }
        Db::Pi(a, b) => Db::Pi(
            DbBox::new(subst(a, j, s)),
            DbBox::new(subst(b, j + 1, &shift(s, 1, 0))),
        ),
        Db::Lam(a, b) => Db::Lam(
            DbBox::new(subst(a, j, s)),
            DbBox::new(subst(b, j + 1, &shift(s, 1, 0))),
        ),
        Db::App(f, a) => Db::App(DbBox::new(subst(f, j, s)), DbBox::new(subst(a, j, s))),
        Db::Match {
            disc,
            motive,
            cases,
        } => Db::Match {
            disc: DbBox::new(subst(disc, j, s)),
            motive: DbBox::new(subst(motive, j, s)),
            cases: cases.iter().map(|c| subst(c, j, s)).collect(),
        },
        Db::Fix(body) => Db::Fix(FixBody::new(subst(body, j + 1, &shift(s, 1, 0)))),
        Db::MutualFix { defs, index } => {
            // Under `n` mutual binders: the substituted index rises by `n` and `s` is
            // shifted by `n` to keep its free variables aligned.
            let n = defs.len();
            Db::MutualFix {
                defs: defs
                    .iter()
                    .map(|b| subst(b, j + n, &shift(s, n as isize, 0)))
                    .collect(),
                index: *index,
            }
        }
        Db::Let(ty, value, body) => Db::Let(
            DbBox::new(subst(ty, j, s)),
            DbBox::new(subst(value, j, s)),
            DbBox::new(subst(body, j + 1, &shift(s, 1, 0))),
        ),
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => t.clone(),
    }
}

/// β-substitution of the argument into a binder body: `(λ. body) arg ⤳ body[0 := arg]`.
fn beta_open(body: &Db, arg: &Db) -> Db {
    beta_open_at(body, arg, 0)
}

/// Fuse the initial argument shift, substitution, and final binder removal.
/// At depth d, index d names the opened binder; larger indices lose that binder,
/// while smaller indices belong to binders encountered inside the body. Shift
/// the argument only at occurrences, so unused arguments require no traversal.
fn beta_open_at(body: &Db, arg: &Db, depth: usize) -> Db {
    #[cfg(test)]
    beta_count_work(usize::from(!matches!(body, Db::Var(k) if *k == depth)));
    match body {
        Db::Var(k) if *k == depth => shift(arg, depth as isize, 0),
        Db::Var(k) if *k > depth => Db::Var(k - 1),
        Db::Var(k) => Db::Var(*k),
        Db::Pi(a, b) => Db::Pi(a.opened(arg, depth), b.opened(arg, depth + 1)),
        Db::Lam(a, b) => Db::Lam(a.opened(arg, depth), b.opened(arg, depth + 1)),
        Db::App(f, a) => Db::App(f.opened(arg, depth), a.opened(arg, depth)),
        Db::Match {
            disc,
            motive,
            cases,
        } => Db::Match {
            disc: disc.opened(arg, depth),
            motive: motive.opened(arg, depth),
            cases: cases.iter().map(|c| beta_open_at(c, arg, depth)).collect(),
        },
        Db::Fix(body) => Db::Fix(body.opened(arg, depth + 1)),
        Db::MutualFix { defs, index } => Db::MutualFix {
            defs: defs
                .iter()
                .map(|b| beta_open_at(b, arg, depth + defs.len()))
                .collect(),
            index: *index,
        },
        Db::Let(ty, value, body) => Db::Let(
            ty.opened(arg, depth),
            value.opened(arg, depth),
            body.opened(arg, depth + 1),
        ),
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => body.clone(),
    }
}

/// Unfold the `index`-th body of a mutual block: replace each of the `n` mutual names by
/// its own `MutualFix` projection and drop the `n` binders. The mutual analog of
/// `beta_open` — the innermost binder (index 0) is name `n-1`, so it receives projection
/// `n-1`, down to name `0`.
///
/// A projection is NOT closed in general: a recursor applied to motives and minors that
/// mention local variables yields a block whose bodies refer to them. The projection lives
/// in the block's OUTER context, while the binder opened at step `k` still sits under the
/// `n-1-k` mutual binders not yet opened, so the projection is shifted over exactly those
/// before it is substituted. Without the shift its free variables would name the wrong
/// binders.
fn open_mutual(defs: &[Db], index: usize) -> Db {
    let n = defs.len();
    let mut result = defs[index].clone();
    for k in 0..n {
        let proj = Db::MutualFix {
            defs: defs.to_vec(),
            index: n - 1 - k,
        };
        result = beta_open(&result, &shift(&proj, (n - 1 - k) as isize, 0));
    }
    result
}

/// Decompose an application spine into its head and arguments (in source order).
fn spine(t: &Db) -> (Db, Vec<Db>) {
    let mut args = Vec::new();
    let mut cur = t.clone();
    while let Db::App(f, a) = cur {
        args.push(a.into_db());
        cur = f.into_db();
    }
    args.reverse();
    (cur, args)
}

/// Whether `t` reduces to an application headed by a constructor — the guard that lets a
/// fixpoint unfold (its decreasing argument is a value) without risking a loop.
/// Whether the re-checker's fix-unfolding guard sees a constructor application.
///
/// A `Nat` LITERAL is morally one — `Nat(0)` IS `Zero` and `Nat(n)` IS `Succ (n-1)`, and this
/// kernel's `Match` arm already expands it that way — so admitting it here closes a real
/// `double_check` disagreement: a structural recursion over a literal is stuck HERE while it
/// computes in the main kernel, which leaves proofs resting on `decide (2 = 2) ⤳ true`
/// single-kernel-verified. The one-clause fix is written out below and is NOT enabled, because
/// it trades a safe-direction disagreement for an UNSAFE one: once the guard lets go, the
/// re-checker actually runs the recursion, and a string-as-character-list recursion over a
/// literal overflows the native stack and ABORTS THE PROCESS. It is blocked on the same stack
/// residue as the depth door in `type_checker.rs` — a larger kernel stack, not a clause here.
///
/// ```text
/// if matches!(whnf(genv, t), Db::Lit(Literal::Nat(_))) { return true; }
/// ```
fn ctor_headed(genv: &Context, t: &Db) -> bool {
    constructor_value(genv, &whnf(genv, t))
}

/// Constructor admission on an already computed independent weak-head value.
fn constructor_value(genv: &Context, reduced: &Db) -> bool {
    if db_int_constructor(genv, reduced).is_some() {
        return true;
    }
    // A `Nat` LITERAL is a constructor application: `Nat(0)` IS `Zero` and `Nat(n)` IS
    // `Succ (n-1)`. This kernel's `Match` arm and `def_eq` already bridge the two, so refusing
    // it HERE is the one place the bridge was missing — and it made every structural recursion
    // over a literal stuck in this kernel while it computed in the main one, which reports as a
    // `double_check` DISAGREEMENT on a term the main kernel is right to accept. Enabling it is
    // safe only because `whnf` is now BOUNDED (step budget + depth door, the same discipline
    // `reduction::normalize_checked` gives the main kernel): the guard letting go means the
    // recursion actually runs, and an unbounded reducer running it aborts the process.
    if matches!(reduced, Db::Lit(Literal::Nat(_))) {
        return true;
    }
    matches!(spine_head(reduced), Db::Global(n) | Db::Const { name: n, .. } if genv.is_constructor(n))
}

// ---------------------------------------------------------------------------
// Universe-polymorphic references — this kernel's own lookup over the SHARED spec
// ---------------------------------------------------------------------------

/// A name's DECLARED universe parameters (present only where a declaration STATED one) and its
/// stored type. The registries are consulted in order, never merged: `get_global` must not see a
/// universe-polymorphic definition.
///
/// [`Context::poly_declaration`] answers for a polymorphic TYPE declaration (a body-less
/// inductive, structure or class) and for a polymorphic DEFINITION alike, so this kernel and the
/// main one cannot come to disagree about which constants carry a level telescope. The stated
/// list is what gets zipped: see the main kernel's `lookup_const` for why a telescope DERIVED
/// from the stored type silently exchanges the parameters of a declaration whose arity mentions
/// them out of order — a wrong answer both kernels reach identically, which is the one shape
/// `double_check` cannot see.
fn const_declaration<'a>(
    genv: &'a Context,
    name: &str,
) -> Option<(Option<&'a [String]>, &'a Term)> {
    if let Some((params, ty)) = genv.poly_declaration(name) {
        return Some((Some(params), ty));
    }
    // A monomorphic declaration has no level telescope, so `name.{}` is NOT a second spelling of
    // `Global(name)` — see the main kernel's `lookup_const` for the argument and the lock.
    let ty = genv.get_global(name)?;
    if !db_mentions_level_term(ty) {
        return None;
    }
    Some((None, ty))
}

/// Whether a named type mentions a free universe variable — the telescope test `const_declaration`
/// gates on, written over `Term` because that is what the environment stores.
fn db_mentions_level_term(ty: &Term) -> bool {
    crate::term::mentions_level_var(ty)
}

#[cfg(test)]
thread_local! {
    static DEFINITION_LOWERINGS:std::cell::Cell<Option<usize>> = const {std::cell::Cell::new(None)};
}
fn definition_lowered() {
    #[cfg(test)]
    DEFINITION_LOWERINGS.with(|s| {
        if let Some(n) = s.get() {
            s.set(Some(n + 1));
        }
    });
}
/// The δ-reduct of `name.{levels}` — its body with the universe parameters instantiated — or
/// `None` when there is no body (a polymorphic inductive, constructor or axiom is a VALUE) or
/// the reference is ill-formed (in which case it is inert here and refused by `infer`).
fn definition_body(genv: &Context, name: &str) -> Option<Db> {
    let body = genv.get_definition_body(name)?;
    definitions::lower(genv, body, None, || {
        definition_lowered();
        to_db(body, &mut Vec::new()).ok()
    })
}

fn const_body(genv: &Context, name: &str, levels: &[Universe]) -> Option<Db> {
    let (declared, ty, body) = match genv.get_universe_poly(name) {
        Some((params, ty, body)) => (Some(params.as_slice()), ty, body),
        None => {
            let ty = genv.get_global(name)?;
            if !db_mentions_level_term(ty) {
                return None;
            }
            (None, ty, genv.get_definition_body(name)?)
        }
    };
    let subst = crate::term::resolve_const_levels(name, declared, ty, levels).ok()?;
    definitions::lower(genv, body, Some(levels), || {
        definition_lowered();
        to_db(
            &crate::term::instantiate_universes(body, &subst),
            &mut Vec::new(),
        )
        .ok()
    })
}

/// The free universe variables of a de Bruijn term, first-occurrence order — this kernel's own
/// walk, over the same definition `term::free_level_vars` states.
fn db_level_vars(t: &Db, out: &mut Vec<String>) {
    let push = |u: &Universe, out: &mut Vec<String>| {
        for v in crate::term::free_level_vars(&Term::Sort(u.clone())) {
            if !out.iter().any(|x| *x == v) {
                out.push(v);
            }
        }
    };
    match t {
        Db::Sort(u) => push(u, out),
        Db::Var(_) | Db::Global(_) | Db::Lit(_) => {}
        Db::Const { levels, .. } => {
            for l in levels {
                push(l, out);
            }
        }
        Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
            db_level_vars(a, out);
            db_level_vars(b, out);
        }
        Db::Match {
            disc,
            motive,
            cases,
        } => {
            db_level_vars(disc, out);
            db_level_vars(motive, out);
            for c in cases {
                db_level_vars(c, out);
            }
        }
        Db::Fix(b) => db_level_vars(b, out),
        Db::MutualFix { defs, .. } => {
            for d in defs {
                db_level_vars(d, out);
            }
        }
        Db::Let(a, b, c) => {
            db_level_vars(a, out);
            db_level_vars(b, out);
            db_level_vars(c, out);
        }
    }
}

/// Whether a de Bruijn term mentions a universe variable satisfying `pred` — recursive,
/// short-circuiting and ALLOCATION-FREE. The guard in front of `db_level_params`: the application
/// rule asks this at every node, almost always about a monomorphic declaration, so learning "no"
/// must cost nothing — not a `Vec`, and certainly not a cloned `Universe` per `Sort`.
fn db_mentions_level(t: &Db, pred: &dyn Fn(&str) -> bool) -> bool {
    match t {
        Db::Sort(u) => crate::term::universe_mentions(u, pred),
        Db::Var(_) | Db::Global(_) | Db::Lit(_) => false,
        Db::Const { levels, .. } => levels
            .iter()
            .any(|l| crate::term::universe_mentions(l, pred)),
        Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
            db_mentions_level(a, pred) || db_mentions_level(b, pred)
        }
        Db::Match {
            disc,
            motive,
            cases,
        } => {
            db_mentions_level(disc, pred)
                || db_mentions_level(motive, pred)
                || cases.iter().any(|c| db_mentions_level(c, pred))
        }
        Db::Fix(b) => db_mentions_level(b, pred),
        Db::MutualFix { defs, .. } => defs.iter().any(|d| db_mentions_level(d, pred)),
        Db::Let(a, b, c) => {
            db_mentions_level(a, pred) || db_mentions_level(b, pred) || db_mentions_level(c, pred)
        }
    }
}

/// The free universe variables of a de Bruijn term, first-occurrence order.
fn db_level_params(t: &Db) -> Vec<String> {
    let mut out = Vec::new();
    db_level_vars(t, &mut out);
    out
}

/// Substitute universe variables throughout a de Bruijn term.
fn db_instantiate_universes(t: &Db, subst: &HashMap<String, Universe>) -> Db {
    match t {
        Db::Sort(u) => Db::Sort(u.substitute(subst)),
        Db::Var(k) => Db::Var(*k),
        Db::Global(n) => Db::Global(n.clone()),
        Db::Lit(l) => Db::Lit(l.clone()),
        Db::Const { name, levels } => Db::Const {
            name: name.clone(),
            levels: levels.iter().map(|l| l.substitute(subst)).collect(),
        },
        Db::Pi(a, b) => Db::Pi(
            DbBox::new(db_instantiate_universes(a, subst)),
            DbBox::new(db_instantiate_universes(b, subst)),
        ),
        Db::Lam(a, b) => Db::Lam(
            DbBox::new(db_instantiate_universes(a, subst)),
            DbBox::new(db_instantiate_universes(b, subst)),
        ),
        Db::App(a, b) => Db::App(
            DbBox::new(db_instantiate_universes(a, subst)),
            DbBox::new(db_instantiate_universes(b, subst)),
        ),
        Db::Match {
            disc,
            motive,
            cases,
        } => Db::Match {
            disc: DbBox::new(db_instantiate_universes(disc, subst)),
            motive: DbBox::new(db_instantiate_universes(motive, subst)),
            cases: cases
                .iter()
                .map(|c| db_instantiate_universes(c, subst))
                .collect(),
        },
        Db::Fix(b) => Db::Fix(FixBody::new(db_instantiate_universes(b, subst))),
        Db::MutualFix { defs, index } => Db::MutualFix {
            defs: defs
                .iter()
                .map(|d| db_instantiate_universes(d, subst))
                .collect(),
            index: *index,
        },
        Db::Let(a, b, c) => Db::Let(
            DbBox::new(db_instantiate_universes(a, subst)),
            DbBox::new(db_instantiate_universes(b, subst)),
            DbBox::new(db_instantiate_universes(c, subst)),
        ),
    }
}

/// Solve the level metavariables of an application's domain from the argument's actual type
/// (the shared specification's rule 4), and propagate the solution into the codomain.
///
/// A domain with no metavariables is returned untouched, so the ordinary path never pays for
/// this. Choosing an instantiation is not accepting one: `check` still runs afterwards.
fn solve_domain_levels(
    genv: &Context,
    lctx: &dyn LocalTypes,
    dom: &Db,
    body: &Db,
    arg: &Db,
) -> (Db, Db) {
    match db_domain_level_solution(genv, lctx, dom, arg) {
        Some(subst) => (
            db_instantiate_universes(dom, &subst),
            db_instantiate_universes(body, &subst),
        ),
        None => (dom.clone(), body.clone()),
    }
}

/// The levels `arg` solves for the metavariables of the domain `dom` — this kernel's application
/// rule's one decision, applied by [`solve_domain_levels_owned`] and recorded by
/// `db_implicit_instance_levels`. `None` when the domain has no metavariables, the argument has
/// no type, or nothing was solved.
fn db_domain_level_solution(
    genv: &Context,
    lctx: &dyn LocalTypes,
    dom: &Db,
    arg: &Db,
) -> Option<HashMap<String, Universe>> {
    if !db_mentions_level(dom, &|v| crate::term::is_level_meta(v)) {
        return None;
    }
    let arg_ty = whnf_in(genv, lctx, &infer(genv, lctx, arg).ok()?);
    let mut subst = HashMap::new();
    db_unify_levels(dom, &arg_ty, &mut subst);
    (!subst.is_empty()).then_some(subst)
}

/// A bare reference to a universe-polymorphic declaration carries no levels, so at its
/// application they become fresh metavariables — the one place with an argument to solve them
/// from. Restricted to a bare `Db::Global` head for the same reason as in the main kernel: a
/// level variable anywhere else is bound by its own declaration.
fn db_implicit_levels_for_head(f: &Db, f_ty: Db) -> Db {
    match f {
        Db::Global(name) if db_mentions_level(&f_ty, &|_| true) => {
            let params = db_level_params(&f_ty);
            if params.is_empty() {
                f_ty
            } else {
                db_instantiate_universes(&f_ty, &crate::term::fresh_level_metas(name, &params))
            }
        }
        _ => f_ty,
    }
}

/// The de Bruijn half of level unification: match a pattern type (which may carry
/// metavariables) against an actual one, solving bare metavariables only.
fn db_unify_levels(pattern: &Db, actual: &Db, subst: &mut HashMap<String, Universe>) {
    match (pattern, actual) {
        (Db::Sort(p), Db::Sort(a)) => {
            crate::term::unify_levels(&Term::Sort(p.clone()), &Term::Sort(a.clone()), subst);
        }
        (Db::Pi(p1, p2), Db::Pi(a1, a2))
        | (Db::Lam(p1, p2), Db::Lam(a1, a2))
        | (Db::App(p1, p2), Db::App(a1, a2)) => {
            db_unify_levels(p1, a1, subst);
            db_unify_levels(p2, a2, subst);
        }
        (
            Db::Const {
                name: n1,
                levels: l1,
            },
            Db::Const {
                name: n2,
                levels: l2,
            },
        ) if n1 == n2 && l1.len() == l2.len() => {
            for (p, a) in l1.iter().zip(l2.iter()) {
                crate::term::unify_levels(&Term::Sort(p.clone()), &Term::Sort(a.clone()), subst);
            }
        }
        _ => {}
    }
}

/// An application spine's HEAD alone — no vector, no clones. The cheapest possible question, and
/// the one the fix guard asks at every reduction step.
fn spine_head(t: &Db) -> &Db {
    let mut cur = t;
    while let Db::App(f, _) = cur {
        cur = f.as_ref();
    }
    cur
}

/// An application spine's head and arguments BY REFERENCE, in source order — the non-cloning
/// counterpart of [`spine`], for the hot pre-checks that only need to look.
fn spine_ref(t: &Db) -> (&Db, Vec<&Db>) {
    let mut args = Vec::new();
    let mut cur = t;
    while let Db::App(f, a) = cur {
        args.push(a.as_ref());
        cur = f.as_ref();
    }
    args.reverse();
    (cur, args)
}

// Optional per-check sharing of independently observed closed Nat prefixes.
// No evaluator result or Context state is shared with the main checker.
const PREFIX_SLOTS: usize = 4;
const PREFIX_NODES: usize = 2048;
const PREFIX_DEPTH: usize = 64;
const PREFIX_TREE_BYTES: usize = 128 * 1024;
const PREFIX_LIVE_BYTES: usize = 512 * 1024;
const PREFIX_PENDING: usize = 196;
fn prefix_reuse_enabled() -> bool {
    true
}
// Bounds cover every node/level/payload operation in the optional helpers.
// They are availability guards, not debits/refunds: actual operations debit fuel.
const PREFIX_ADMISSION_WORK: usize = 8 * (2 * PREFIX_NODES) + 16384 + 2;
const PREFIX_COPY_WORK: usize = 3 * (2 * PREFIX_NODES) + 16384;
fn prefix_work_available(work: usize) -> bool {
    !WHNF_EXHAUSTED.with(|v| v.get()) && WHNF_FUEL.with(|v| v.get()) >= work
}

#[derive(Clone, Copy, Default, Debug)]
struct PrefixMetrics {
    nodes: usize,
    levels: usize,
    depth: usize,
    payload: usize,
    bytes: usize,
    closed: bool,
}
impl PrefixMetrics {
    fn comparison_cost(self) -> usize {
        2 * (self.nodes + self.levels + self.payload) + 1
    }
}
#[derive(Clone, Copy, Default)]
struct PrefixMemory {
    scope: usize,
    live: usize,
    peak: usize,
}
thread_local! {
    static PREFIX_MEMORY: std::cell::Cell<PrefixMemory> =
        const { std::cell::Cell::new(PrefixMemory { scope:0,live:0,peak:0 }) };
    // Tickets released while a nested table is active debit their original
    // ledger when that table is restored. Until then the aggregate overcounts.
    static PREFIX_RELEASES: std::cell::RefCell<Option<std::rc::Rc<std::cell::Cell<usize>>>> =
        const { std::cell::RefCell::new(None) };
    static PREFIX_GENERATION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREFIX_TABLE: std::cell::RefCell<PrefixTable> =
        std::cell::RefCell::new(PrefixTable::default());
}
#[cfg(test)]
thread_local! {
    static PREFIX_METRICS_NODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREFIX_STATS: std::cell::Cell<[usize;5]> = const { std::cell::Cell::new([0;5]) };
}
fn prefix_stat(index: usize) {
    #[cfg(test)]
    PREFIX_STATS.with(|v| {
        let mut a = v.get();
        a[index] += 1;
        v.set(a);
    });
    #[cfg(not(test))]
    let _ = index;
}
struct PrefixTicket {
    scope: usize,
    bytes: usize,
    releases: std::rc::Rc<std::cell::Cell<usize>>,
}
impl PrefixTicket {
    fn new(bytes: usize) -> Option<Self> {
        let releases = PREFIX_RELEASES.with(|v| v.borrow().clone())?;
        let mut ticket = Self {
            scope: PREFIX_MEMORY.with(|m| m.get().scope),
            bytes: 0,
            releases,
        };
        if ticket.scope == 0 {
            return None;
        }
        ticket.add(bytes)?;
        Some(ticket)
    }
    fn add(&mut self, bytes: usize) -> Option<()> {
        PREFIX_MEMORY.with(|m| {
            let mut state = m.get();
            if state.scope != self.scope {
                return None;
            }
            let live = state.live.checked_add(bytes)?;
            if !prefix_combined_fits(live) {
                return None;
            }
            state.live = live;
            state.peak = state.peak.max(live);
            self.bytes = self.bytes.checked_add(bytes)?;
            m.set(state);
            Some(())
        })
    }
}
impl Drop for PrefixTicket {
    fn drop(&mut self) {
        PREFIX_MEMORY.with(|m| {
            let mut state = m.get();
            if state.scope == self.scope {
                state.live = state.live.saturating_sub(self.bytes);
                m.set(state);
            } else {
                // Each ticket releases once; total reservations are bounded.
                self.releases.set(
                    self.releases
                        .get()
                        .checked_add(self.bytes)
                        .expect("bounded outstanding prefix reservations"),
                );
            }
        });
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrefixStatus {
    Available,
    Busy,
    Vacant,
}
struct PrefixEntry {
    key: Db,
    key_metrics: PrefixMetrics,
    _key_memory: PrefixTicket,
    observed: usize,
    tail: Option<Db>,
    tail_memory: Option<PrefixTicket>,
    tail_metrics: Option<PrefixMetrics>,
    status: std::rc::Rc<std::cell::Cell<PrefixStatus>>,
    _status_memory: PrefixTicket,
}
struct PrefixTable {
    context: usize,
    scope: usize,
    entries: [Option<PrefixEntry>; PREFIX_SLOTS],
    definitions: definitions::Cache,
}
impl Default for PrefixTable {
    fn default() -> Self {
        Self {
            context: 0,
            scope: 0,
            entries: std::array::from_fn(|_| None),
            definitions: definitions::Cache::default(),
        }
    }
}
#[must_use]
struct PrefixScope<'ctx> {
    previous: Option<PrefixTable>,
    memory: PrefixMemory,
    previous_factory: FixFactory,
    releases: Option<std::rc::Rc<std::cell::Cell<usize>>>,
    // The check's own positive conversion evidence (`conversion_memo`), for this context only.
    _evidence: conversion_memo::Scope,
    context: std::marker::PhantomData<&'ctx Context>,
    not_send: std::marker::PhantomData<*const ()>,
}
fn prefix_table_bytes() -> usize {
    std::mem::size_of::<PrefixTable>()
        + std::mem::size_of::<FixFactory>()
        + std::mem::size_of::<std::cell::Cell<usize>>()
        + 2 * std::mem::size_of::<usize>()
}
impl<'ctx> PrefixScope<'ctx> {
    fn enter(ctx: &'ctx Context) -> Self {
        let memory = PREFIX_MEMORY.with(|v| v.get());
        let scope = PREFIX_GENERATION.with(|v| {
            let next = v.get().checked_add(1)?;
            v.set(next);
            Some(next)
        });
        let scope = scope
            .filter(|_| {
                memory
                    .live
                    .checked_add(prefix_table_bytes())
                    .is_some_and(prefix_combined_fits)
            })
            .unwrap_or(0);
        // A failed nested admission suppresses the outer cache, including for
        // the very same Context. The previous scope is restored by Drop.
        let previous = PREFIX_TABLE.with(|v| {
            v.replace(PrefixTable {
                context: if scope == 0 {
                    0
                } else {
                    ctx as *const Context as usize
                },
                scope,
                ..PrefixTable::default()
            })
        });
        let releases = PREFIX_RELEASES.with(|v| {
            v.replace(if scope == 0 {
                None
            } else {
                Some(std::rc::Rc::new(std::cell::Cell::new(0)))
            })
        });
        let fixed = if scope == 0 { 0 } else { prefix_table_bytes() };
        let previous_factory = FIX_FACTORY.with(|f| {
            f.replace(FixFactory {
                scope,
                ..FixFactory::default()
            })
        });
        PREFIX_MEMORY.with(|v| {
            v.set(PrefixMemory {
                scope,
                live: memory.live + fixed,
                peak: memory.peak.max(memory.live + fixed),
            })
        });
        Self {
            previous: Some(previous),
            memory,
            previous_factory,
            releases,
            _evidence: conversion_memo::Scope::enter(ctx),
            context: std::marker::PhantomData,
            not_send: std::marker::PhantomData,
        }
    }
}
impl Drop for PrefixScope<'_> {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            let completed = PREFIX_TABLE.with(|v| v.replace(previous));
            drop(completed); // release this scope's tickets before restoring the outer ledger
            FIX_FACTORY.with(|f| f.set(self.previous_factory));
            let releases = self.releases.take();
            let deferred = releases.as_ref().map_or(0, |r| r.replace(0));
            PREFIX_RELEASES.with(|v| {
                v.replace(releases);
            });
            PREFIX_MEMORY.with(|v| {
                let peak = v.get().peak;
                let mut old = self.memory;
                old.live = old
                    .live
                    .checked_sub(deferred)
                    .expect("original scope owns released tickets");
                if old.scope != 0 {
                    old.peak = old.peak.max(peak);
                }
                v.set(old);
            });
        }
    }
}

/// Bounded, equality-faithful closure/capacity admission. Wide children are
/// visited through slices; no frontier proportional to an untrusted width.
fn prefix_metrics(t: &Db) -> Option<PrefixMetrics> {
    enum Visit<'a> {
        Node(&'a Db, usize, usize),
        Slice(&'a [Db], usize, usize),
        Level(&'a Universe, usize),
        Levels(&'a [Universe], usize),
    }
    fn push<'a>(v: &mut Vec<Visit<'a>>, x: Visit<'a>) -> Option<()> {
        if !nat_order_tick() || v.len() == PREFIX_PENDING {
            return None;
        }
        v.push(x);
        Some(())
    }
    fn payload(m: &mut PrefixMetrics, s: &str, capacity: usize) -> Option<()> {
        if s.len() > 256 {
            return None;
        }
        m.payload = m.payload.checked_add(s.len())?;
        m.bytes = m.bytes.checked_add(capacity)?;
        if m.payload > 16384 || !nat_order_spend(s.len()) {
            return None;
        }
        Some(())
    }
    let requested = PREFIX_PENDING.checked_mul(std::mem::size_of::<Visit<'_>>())?;
    let mut memory = PrefixTicket::new(requested)?;
    if !nat_order_tick() {
        return None;
    }
    let mut pending = Vec::with_capacity(PREFIX_PENDING);
    let actual = pending
        .capacity()
        .checked_mul(std::mem::size_of::<Visit<'_>>())?;
    if actual > requested {
        memory.add(actual - requested)?;
    }
    let mut m = PrefixMetrics {
        closed: true,
        bytes: std::mem::size_of::<Db>(),
        ..PrefixMetrics::default()
    };
    push(&mut pending, Visit::Node(t, 1, 0))?;
    while let Some(item) = pending.pop() {
        if !nat_order_tick() {
            return None;
        }
        match item {
            Visit::Slice(a, d, b) => {
                if let Some((head, rest)) = a.split_first() {
                    push(&mut pending, Visit::Slice(rest, d, b))?;
                    push(&mut pending, Visit::Node(head, d, b))?;
                }
            }
            Visit::Levels(a, d) => {
                if let Some((head, rest)) = a.split_first() {
                    push(&mut pending, Visit::Levels(rest, d))?;
                    push(&mut pending, Visit::Level(head, d))?;
                }
            }
            Visit::Level(u, d) => {
                m.levels += 1;
                m.depth = m.depth.max(d);
                if d > PREFIX_DEPTH || m.levels > PREFIX_NODES {
                    return None;
                }
                match u {
                    Universe::SProp | Universe::Prop | Universe::Type(_) => {}
                    Universe::Var(n) => payload(&mut m, n, n.capacity())?,
                    Universe::Succ(a) => {
                        m.bytes = m.bytes.checked_add(std::mem::size_of::<Universe>())?;
                        push(&mut pending, Visit::Level(a, d + 1))?;
                    }
                    Universe::Max(a, b) | Universe::IMax(a, b) => {
                        m.bytes = m.bytes.checked_add(2 * std::mem::size_of::<Universe>())?;
                        push(&mut pending, Visit::Level(b, d + 1))?;
                        push(&mut pending, Visit::Level(a, d + 1))?;
                    }
                }
            }
            Visit::Node(t, d, b) => {
                #[cfg(test)]
                PREFIX_METRICS_NODES.with(|v| v.set(v.get() + 1));
                m.nodes += 1;
                m.depth = m.depth.max(d);
                if d > PREFIX_DEPTH || m.nodes > PREFIX_NODES {
                    return None;
                }
                match t {
                    Db::Var(i) => m.closed &= *i < b,
                    Db::Global(n) => payload(&mut m, n, n.capacity())?,
                    Db::Const { name, levels } => {
                        payload(&mut m, name, name.capacity())?;
                        if levels.len() > PREFIX_NODES {
                            return None;
                        }
                        m.bytes = m.bytes.checked_add(
                            levels
                                .capacity()
                                .checked_mul(std::mem::size_of::<Universe>())?,
                        )?;
                        push(&mut pending, Visit::Levels(levels, d + 1))?;
                    }
                    Db::Sort(u) => push(&mut pending, Visit::Level(u, d + 1))?,
                    Db::Lit(Literal::Float(_)) => return None,
                    Db::Lit(Literal::Text(s)) => payload(&mut m, s, s.capacity())?,
                    Db::Lit(Literal::Nat(n)) => {
                        if n.to_i64().is_none() || n.is_negative() {
                            return None;
                        }
                    }
                    Db::Lit(Literal::BigInt(n)) => {
                        n.to_i64()?;
                    }
                    Db::Lit(_) => {}
                    Db::Pi(a, c) | Db::Lam(a, c) => {
                        m.bytes = m.bytes.checked_add(2 * std::mem::size_of::<Db>())?;
                        push(&mut pending, Visit::Node(c, d + 1, b.checked_add(1)?))?;
                        push(&mut pending, Visit::Node(a, d + 1, b))?;
                    }
                    Db::App(a, c) => {
                        m.bytes = m.bytes.checked_add(2 * std::mem::size_of::<Db>())?;
                        push(&mut pending, Visit::Node(c, d + 1, b))?;
                        push(&mut pending, Visit::Node(a, d + 1, b))?;
                    }
                    Db::Fix(c) => {
                        if let Some(facts) = c.facts() {
                            if !nat_order_tick() {
                                return None;
                            }
                            fix_body_stat(2);
                            m.nodes = m.nodes.checked_add(facts.metrics.nodes)?;
                            m.levels = m.levels.checked_add(facts.metrics.levels)?;
                            m.depth = m.depth.max(d.checked_add(facts.metrics.depth)?);
                            m.payload = m.payload.checked_add(facts.metrics.payload)?;
                            m.bytes = m.bytes.checked_add(facts.metrics.bytes)?;
                            m.closed &= facts.required <= b.checked_add(1)?;
                            if m.nodes > PREFIX_NODES
                                || m.levels > PREFIX_NODES
                                || m.depth > PREFIX_DEPTH
                                || m.payload > 16384
                            {
                                return None;
                            }
                        } else {
                            m.bytes = m.bytes.checked_add(std::mem::size_of::<Db>())?;
                            push(&mut pending, Visit::Node(c, d + 1, b.checked_add(1)?))?;
                        }
                    }
                    Db::Let(a, c, e) => {
                        m.bytes = m.bytes.checked_add(3 * std::mem::size_of::<Db>())?;
                        push(&mut pending, Visit::Node(e, d + 1, b.checked_add(1)?))?;
                        push(&mut pending, Visit::Node(c, d + 1, b))?;
                        push(&mut pending, Visit::Node(a, d + 1, b))?;
                    }
                    Db::Match {
                        disc,
                        motive,
                        cases,
                    } => {
                        if cases.len() > PREFIX_NODES {
                            return None;
                        }
                        m.bytes = m
                            .bytes
                            .checked_add(2 * std::mem::size_of::<Db>())?
                            .checked_add(
                                cases.capacity().checked_mul(std::mem::size_of::<Db>())?,
                            )?;
                        push(&mut pending, Visit::Slice(cases, d + 1, b))?;
                        push(&mut pending, Visit::Node(motive, d + 1, b))?;
                        push(&mut pending, Visit::Node(disc, d + 1, b))?;
                    }
                    Db::MutualFix { defs, .. } => {
                        if defs.len() > PREFIX_NODES {
                            return None;
                        }
                        m.bytes = m
                            .bytes
                            .checked_add(defs.capacity().checked_mul(std::mem::size_of::<Db>())?)?;
                        push(
                            &mut pending,
                            Visit::Slice(defs, d + 1, b.checked_add(defs.len())?),
                        )?;
                    }
                }
            }
        }
        if m.bytes > PREFIX_TREE_BYTES {
            return None;
        }
    }
    Some(m)
}

// Explicit copying only after prefix_metrics admission. Each allocation request
// is reserved before construction; any actual excess capacity is charged before
// filling. Tickets release on every failed partial construction.
fn prefix_string(s: &str, m: &mut PrefixTicket) -> Option<String> {
    if !nat_order_spend(s.len() + 1) {
        return None;
    }
    m.add(s.len())?;
    let mut out = String::with_capacity(s.len());
    if out.capacity() > s.len() {
        m.add(out.capacity() - s.len())?;
    }
    out.push_str(s);
    Some(out)
}
fn prefix_vec<T>(len: usize, m: &mut PrefixTicket) -> Option<Vec<T>> {
    if !nat_order_tick() {
        return None;
    }
    let bytes = len.checked_mul(std::mem::size_of::<T>())?;
    m.add(bytes)?;
    let out = Vec::with_capacity(len);
    let actual = out.capacity().checked_mul(std::mem::size_of::<T>())?;
    if actual > bytes {
        m.add(actual - bytes)?;
    }
    Some(out)
}
fn prefix_level(u: &Universe, m: &mut PrefixTicket) -> Option<Universe> {
    if !nat_order_tick() {
        return None;
    }
    fn boxed(u: &Universe, m: &mut PrefixTicket) -> Option<Box<Universe>> {
        m.add(std::mem::size_of::<Universe>())?;
        Some(Box::new(prefix_level(u, m)?))
    }
    Some(match u {
        Universe::SProp => Universe::SProp,
        Universe::Prop => Universe::Prop,
        Universe::Type(n) => Universe::Type(*n),
        Universe::Var(n) => Universe::Var(prefix_string(n, m)?),
        Universe::Succ(a) => Universe::Succ(boxed(a, m)?),
        Universe::Max(a, b) => Universe::Max(boxed(a, m)?, boxed(b, m)?),
        Universe::IMax(a, b) => Universe::IMax(boxed(a, m)?, boxed(b, m)?),
    })
}
fn prefix_copy_node(t: &Db, m: &mut PrefixTicket) -> Option<Db> {
    if !nat_order_tick() {
        return None;
    }
    prefix_stat(2); // actual Db nodes copied, not reserved copies
    fn boxed(t: &Db, m: &mut PrefixTicket) -> Option<DbBox> {
        m.add(std::mem::size_of::<Db>())?;
        Some(DbBox::owned(Box::new(prefix_copy_node(t, m)?)))
    }
    Some(match t {
        Db::Var(i) => Db::Var(*i),
        Db::Global(n) => Db::Global(prefix_string(n, m)?),
        Db::Sort(u) => Db::Sort(prefix_level(u, m)?),
        Db::Const { name, levels } => {
            let name = prefix_string(name, m)?;
            let mut out = prefix_vec(levels.len(), m)?;
            for u in levels {
                out.push(prefix_level(u, m)?);
            }
            Db::Const { name, levels: out }
        }
        Db::Pi(a, b) => Db::Pi(boxed(a, m)?, boxed(b, m)?),
        Db::Lam(a, b) => Db::Lam(boxed(a, m)?, boxed(b, m)?),
        Db::App(a, b) => Db::App(boxed(a, m)?, boxed(b, m)?),
        Db::Fix(b) => Db::Fix(match b {
            FixBody::Shared(_) => b.clone(), // existing persistent ticket owns the immutable body
            FixBody::Owned(_) => FixBody::owned(boxed(b, m)?.into_box()),
        }),
        Db::Let(a, b, c) => Db::Let(boxed(a, m)?, boxed(b, m)?, boxed(c, m)?),
        Db::Match {
            disc,
            motive,
            cases,
        } => {
            let disc = boxed(disc, m)?;
            let motive = boxed(motive, m)?;
            let mut out = prefix_vec(cases.len(), m)?;
            for c in cases {
                out.push(prefix_copy_node(c, m)?);
            }
            Db::Match {
                disc,
                motive,
                cases: out,
            }
        }
        Db::MutualFix { defs, index } => {
            let mut out = prefix_vec(defs.len(), m)?;
            for d in defs {
                out.push(prefix_copy_node(d, m)?);
            }
            Db::MutualFix {
                defs: out,
                index: *index,
            }
        }
        Db::Lit(l) => Db::Lit(match l {
            Literal::Int(n) => Literal::Int(*n),
            Literal::Duration(n) => Literal::Duration(*n),
            Literal::Date(n) => Literal::Date(*n),
            Literal::Moment(n) => Literal::Moment(*n),
            Literal::Text(s) => Literal::Text(prefix_string(s, m)?),
            Literal::Nat(n) => Literal::Nat(n.to_i64()?.into()),
            Literal::BigInt(n) => Literal::BigInt(n.to_i64()?.into()),
            Literal::Float(_) => return None,
        }),
    })
}
fn prefix_copy(t: &Db) -> Option<(Db, PrefixTicket)> {
    let mut memory = PrefixTicket::new(std::mem::size_of::<Db>())?;
    let copy = prefix_copy_node(t, &mut memory)?;
    Some((copy, memory))
}

// The cursor owns the tail first and this ticket last, so abandoning a cursor
// destroys its owned syntax before releasing its auxiliary capacity reservation.
struct PrefixOwned {
    metrics: Option<PrefixMetrics>,
    memory: PrefixTicket,
    clean: bool,
}
struct PrefixSeed<'a> {
    key: &'a Db,
    metrics: PrefixMetrics,
    slot: Option<usize>,
    scope: usize,
    observed: usize,
    publishable: bool,
    lease: Option<std::rc::Rc<std::cell::Cell<PrefixStatus>>>,
}
impl Drop for PrefixSeed<'_> {
    fn drop(&mut self) {
        // This exact slot's shared state survives an inner table being active.
        // No table borrow, lookup, allocation, or evaluator call occurs here.
        if let Some(status) = self.lease.take() {
            if status.get() == PrefixStatus::Busy {
                status.set(PrefixStatus::Vacant);
            }
        }
    }
}
fn prefix_move_enabled() -> bool {
    true
}
// Exact, metered structural comparison. Shared identity skips only immutable bytes
// actually owned by both operands; no historical pointer/hash table is consulted.
fn prefix_exact_equal(a: &Db, b: &Db) -> Option<bool> {
    fn text(a: &str, b: &str) -> Option<bool> {
        if a.len() != b.len() {
            return Some(false);
        }
        if !nat_order_spend(a.len().checked_mul(2)?) {
            return None;
        }
        Some(a == b)
    }
    fn level(a: &Universe, b: &Universe, depth: usize) -> Option<bool> {
        if depth > PREFIX_DEPTH || !nat_order_spend(2) {
            return None;
        }
        Some(match (a, b) {
            (Universe::SProp, Universe::SProp) | (Universe::Prop, Universe::Prop) => true,
            (Universe::Type(x), Universe::Type(y)) => x == y,
            (Universe::Var(x), Universe::Var(y)) => text(x, y)?,
            (Universe::Succ(x), Universe::Succ(y)) => level(x, y, depth + 1)?,
            (Universe::Max(a, b), Universe::Max(c, d))
            | (Universe::IMax(a, b), Universe::IMax(c, d)) => {
                level(a, c, depth + 1)? && level(b, d, depth + 1)?
            }
            _ => false,
        })
    }
    fn visit(a: &Db, b: &Db, depth: usize) -> Option<bool> {
        if depth > PREFIX_DEPTH || !nat_order_spend(2) {
            return None;
        }
        fix_body_stat(5);
        Some(match (a, b) {
            (Db::Var(x), Db::Var(y)) => x == y,
            (Db::Global(x), Db::Global(y)) => text(x, y)?,
            (Db::Sort(x), Db::Sort(y)) => level(x, y, depth + 1)?,
            (Db::Const { name: a, levels: x }, Db::Const { name: b, levels: y }) => {
                if x.len() != y.len() || !text(a, b)? {
                    false
                } else {
                    let mut same = true;
                    for (a, b) in x.iter().zip(y) {
                        if !level(a, b, depth + 1)? {
                            same = false;
                            break;
                        }
                    }
                    same
                }
            }
            (Db::Pi(a, b), Db::Pi(c, d))
            | (Db::Lam(a, b), Db::Lam(c, d))
            | (Db::App(a, b), Db::App(c, d)) => visit(a, c, depth + 1)? && visit(b, d, depth + 1)?,
            (Db::Fix(a), Db::Fix(b)) => {
                if a.same_current(b) {
                    if !nat_order_spend(2) {
                        return None;
                    }
                    fix_body_stat(3);
                    true
                } else {
                    visit(a, b, depth + 1)?
                }
            }
            (Db::Let(a, b, c), Db::Let(d, e, f)) => {
                visit(a, d, depth + 1)? && visit(b, e, depth + 1)? && visit(c, f, depth + 1)?
            }
            (
                Db::Match {
                    disc: a,
                    motive: b,
                    cases: x,
                },
                Db::Match {
                    disc: c,
                    motive: d,
                    cases: y,
                },
            ) => {
                if x.len() != y.len() || !visit(a, c, depth + 1)? || !visit(b, d, depth + 1)? {
                    false
                } else {
                    let mut same = true;
                    for (a, b) in x.iter().zip(y) {
                        if !visit(a, b, depth + 1)? {
                            same = false;
                            break;
                        }
                    }
                    same
                }
            }
            (Db::MutualFix { defs: x, index: i }, Db::MutualFix { defs: y, index: j }) => {
                if i != j || x.len() != y.len() {
                    false
                } else {
                    let mut same = true;
                    for (a, b) in x.iter().zip(y) {
                        if !visit(a, b, depth + 1)? {
                            same = false;
                            break;
                        }
                    }
                    same
                }
            }
            (Db::Lit(a), Db::Lit(b)) => match (a, b) {
                (Literal::Int(a), Literal::Int(b))
                | (Literal::Duration(a), Literal::Duration(b))
                | (Literal::Moment(a), Literal::Moment(b)) => a == b,
                (Literal::Date(a), Literal::Date(b)) => a == b,
                (Literal::Text(a), Literal::Text(b)) => text(a, b)?,
                (Literal::Nat(a), Literal::Nat(b)) | (Literal::BigInt(a), Literal::BigInt(b)) => {
                    a.to_i64()? == b.to_i64()?
                }
                // An admitted stored key cannot contain Float.
                (Literal::Float(_), _) => return None,
                _ => false,
            },
            _ => false,
        })
    }
    if !nat_order_tick() {
        return None;
    }
    visit(a, b, 1)
}

fn prefix_lookup<'a>(
    ctx: &Context,
    key: &'a Db,
) -> Option<(PrefixSeed<'a>, Option<NatOrderCursor<'a>>)> {
    if !prefix_reuse_enabled() {
        return None;
    }
    PREFIX_TABLE.with(|state| {
        let mut table = state.borrow_mut();
        if table.scope == 0 || table.context != ctx as *const Context as usize {
            return None;
        }
        let scope = table.scope;
        let comparisons = table
            .entries
            .iter()
            .flatten()
            .map(|e| e.key_metrics.comparison_cost())
            .sum::<usize>();
        if !prefix_work_available(comparisons + PREFIX_ADMISSION_WORK + PREFIX_COPY_WORK) {
            return None;
        }
        for (slot, entry) in table.entries.iter_mut().enumerate() {
            let Some(entry) = entry else {
                continue;
            };
            prefix_stat(0);
            if !prefix_exact_equal(&entry.key, key)? {
                continue;
            }
            if entry.status.get() == PrefixStatus::Busy {
                return None;
            }
            if entry.status.get() == PrefixStatus::Vacant {
                entry.status.set(PrefixStatus::Busy);
                return Some((
                    PrefixSeed {
                        key,
                        metrics: entry.key_metrics,
                        slot: Some(slot),
                        scope,
                        observed: 0,
                        publishable: true,
                        lease: Some(entry.status.clone()),
                    },
                    None,
                ));
            }
            // Retain the prior conservative capacity gate. A move allocates
            // nothing and debits no copying work; full-ledger hits still refuse.
            let bytes = entry.tail_memory.as_ref().map_or(0, |m| m.bytes);
            if PREFIX_MEMORY.with(|m| {
                m.get()
                    .live
                    .checked_add(bytes)
                    .is_none_or(|n| !prefix_combined_fits(n))
            }) {
                return None;
            }
            let (tail, owned, lease) = if prefix_move_enabled() {
                let memory = entry
                    .tail_memory
                    .take()
                    .expect("available prefix owns reservation");
                let tail = entry.tail.take().map(NatOrderTail::Owned);
                entry.status.set(PrefixStatus::Busy);
                (
                    tail,
                    Some(PrefixOwned {
                        metrics: entry.tail_metrics,
                        memory,
                        clean: true,
                    }),
                    Some(entry.status.clone()),
                )
            } else {
                // Compiling RED seam: preserve old copies/admission on hits.
                let (tail, owned) = match &entry.tail {
                    Some(t) => {
                        let (t, memory) = prefix_copy(t)?;
                        (
                            Some(NatOrderTail::Owned(t)),
                            Some(PrefixOwned {
                                metrics: None,
                                memory,
                                clean: false,
                            }),
                        )
                    }
                    None => (None, None),
                };
                (tail, owned, None)
            };
            prefix_stat(1);
            return Some((
                PrefixSeed {
                    key,
                    metrics: entry.key_metrics,
                    slot: Some(slot),
                    scope,
                    observed: entry.observed,
                    publishable: true,
                    lease,
                },
                Some(NatOrderCursor {
                    count: logicaffeine_base::BigInt::from_i64(entry.observed as i64),
                    tail,
                    prefix: owned,
                }),
            ));
        }
        if table.entries.iter().all(Option::is_some) {
            return None;
        }
        let metrics = prefix_metrics(key)?;
        if !metrics.closed {
            return None;
        }
        Some((
            PrefixSeed {
                key,
                metrics,
                slot: None,
                scope: table.scope,
                observed: 0,
                publishable: true,
                lease: None,
            },
            None,
        ))
    })
}
fn prefix_observe(seed: &mut Option<PrefixSeed<'_>>, count: &logicaffeine_base::BigInt) {
    if let Some(seed) = seed {
        let next = count
            .to_i64()
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| seed.observed.checked_add(n));
        match next {
            Some(n) if n <= MAX_RECHECK_STEPS => seed.observed = n,
            _ => seed.publishable = false,
        }
    }
}
fn prefix_publish(
    ctx: &Context,
    seed: &mut Option<PrefixSeed<'_>>,
    cursor: &mut NatOrderCursor<'_>,
) {
    let Some(seed) = seed.as_mut().filter(|s| s.publishable) else {
        return;
    };
    if WHNF_EXHAUSTED.with(|e| e.get()) || matches!(cursor.tail, Some(NatOrderTail::Borrowed(_))) {
        return;
    }
    // Return replaced owned values so they are dropped after the table borrow.
    let retired = PREFIX_TABLE.with(|state| {
        let mut table = state.borrow_mut();
        if table.scope != seed.scope || table.context != ctx as *const Context as usize {
            return None;
        }
        let mut slot = seed.slot;
        let comparisons = if slot.is_some() {
            0
        } else {
            table
                .entries
                .iter()
                .flatten()
                .map(|e| e.key_metrics.comparison_cost())
                .sum()
        };
        if !prefix_work_available(comparisons + PREFIX_ADMISSION_WORK + PREFIX_COPY_WORK) {
            return None;
        }
        if slot.is_none() {
            for (i, entry) in table.entries.iter().enumerate() {
                let Some(entry) = entry else {
                    continue;
                };
                if !nat_order_spend(entry.key_metrics.comparison_cost()) {
                    return None;
                }
                prefix_stat(0);
                if entry.key == *seed.key {
                    slot = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = slot {
            let entry = table.entries[i].as_ref()?;
            if let Some(lease) = &seed.lease {
                if !std::rc::Rc::ptr_eq(lease, &entry.status) || lease.get() != PrefixStatus::Busy {
                    return None;
                }
            } else if entry.status.get() == PrefixStatus::Busy
                || (entry.status.get() == PrefixStatus::Available
                    && entry.observed >= seed.observed)
            {
                return None;
            }
        }
        let i = slot.or_else(|| table.entries.iter().position(Option::is_none))?;
        let clean = cursor.prefix.as_ref().is_some_and(|p| p.clean);
        let tail_metrics = if clean {
            // Certified direct projection only removes nonbinding wrappers.
            // Its old capacity/depth/closedness bounds remain conservative.
            cursor.prefix.as_ref().and_then(|p| p.metrics)
        } else {
            match cursor.tail.as_ref() {
                Some(NatOrderTail::Owned(t)) => match prefix_metrics(t) {
                    Some(m) if m.closed => Some(m),
                    _ => return None,
                },
                None => None,
                Some(NatOrderTail::Borrowed(_)) => return None,
            }
        };
        let new_memory = if clean {
            None
        } else {
            Some(PrefixTicket::new(tail_metrics.map_or(0, |m| m.bytes))?)
        };
        let new_key = if table.entries[i].is_none() {
            let status_memory = PrefixTicket::new(std::mem::size_of::<(
                usize,
                usize,
                std::cell::Cell<PrefixStatus>,
            )>())?;
            let (key, key_memory) = prefix_copy(seed.key)?;
            let status = std::rc::Rc::new(std::cell::Cell::new(PrefixStatus::Available));
            Some((key, key_memory, status, status_memory))
        } else {
            None
        };
        if WHNF_EXHAUSTED.with(|e| e.get()) {
            return None;
        }
        let tail = match cursor.tail.take() {
            Some(NatOrderTail::Owned(t)) => Some(t),
            None => None,
            Some(NatOrderTail::Borrowed(_)) => unreachable!(),
        };
        let old_owned = cursor.prefix.take();
        let (tail_memory, retired_memory) = if clean {
            (old_owned.expect("clean owned certificate").memory, None)
        } else {
            (
                new_memory.expect("fresh reservation"),
                old_owned.map(|p| p.memory),
            )
        };
        let retired = if let Some((key, key_memory, status, status_memory)) = new_key {
            table.entries[i] = Some(PrefixEntry {
                key,
                key_metrics: seed.metrics,
                _key_memory: key_memory,
                observed: seed.observed,
                tail,
                tail_memory: Some(tail_memory),
                tail_metrics,
                status,
                _status_memory: status_memory,
            });
            (None, None, retired_memory)
        } else {
            let entry = table.entries[i]
                .as_mut()
                .expect("existing immutable slot key");
            entry.observed = seed.observed;
            let old_tail = std::mem::replace(&mut entry.tail, tail);
            let old_memory = entry.tail_memory.replace(tail_memory);
            entry.tail_metrics = tail_metrics;
            entry.status.set(PrefixStatus::Available);
            (old_tail, old_memory, retired_memory)
        };
        seed.lease = None;
        prefix_stat(3);
        Some(retired)
    });
    drop(retired);
}

// Optional work telemetry counts executed cursor/admission/WHNF dispatches,
// independently of fuel reservation and the existing actual clone-node counter.
#[cfg(test)]
thread_local! {
    static NAT_ORDER_WORK: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}
fn nat_order_observed_work() {
    #[cfg(test)]
    NAT_ORDER_WORK.with(|v| {
        if let Some(n) = v.get() {
            v.set(Some(n + 1));
        }
    });
}
fn nat_order_spend(n: usize) -> bool {
    let enough = WHNF_FUEL.with(|f| {
        let left = f.get();
        if left < n {
            f.set(0);
            false
        } else {
            f.set(left - n);
            true
        }
    });
    if !enough {
        WHNF_EXHAUSTED.with(|v| v.set(true));
    }
    enough
}
fn nat_order_tick() -> bool {
    if !nat_order_spend(1) {
        return false;
    }
    nat_order_observed_work();
    true
}

/// Check a borrowed computed tail before cloning it. Slice continuations keep
/// wide Match/MutualFix nodes from allocating a frontier proportional to width.
fn nat_order_admit(t: &Db) -> Option<usize> {
    enum Visit<'a> {
        Node(&'a Db, usize),
        Slice(std::slice::Iter<'a, Db>, usize),
    }
    fn push<'a>(pending: &mut Vec<Visit<'a>>, v: Visit<'a>) -> Option<()> {
        if !nat_order_tick() {
            return None;
        }
        // At most two pending binary siblings per admitted depth, plus a slice
        // continuation. Bound auxiliary storage before Vec can grow.
        if pending.len() >= MAX_RECHECK_WHNF_DEPTH * 3 + 3 {
            WHNF_EXHAUSTED.with(|e| e.set(true));
            return None;
        }
        pending.push(v);
        Some(())
    }
    let mut pending = Vec::new();
    push(&mut pending, Visit::Node(t, 1))?;
    let mut nodes = 0usize;
    while let Some(item) = pending.pop() {
        if !nat_order_tick() {
            return None;
        }
        match item {
            Visit::Slice(mut children, depth) => {
                if let Some(child) = children.next() {
                    push(&mut pending, Visit::Slice(children, depth))?;
                    push(&mut pending, Visit::Node(child, depth))?;
                }
            }
            Visit::Node(t, depth) => {
                nodes += 1;
                if depth > MAX_RECHECK_WHNF_DEPTH || nodes > MAX_RECHECK_TERM_NODES {
                    WHNF_EXHAUSTED.with(|e| e.set(true));
                    return None;
                }
                match t {
                    Db::Sort(_) | Db::Var(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => {}
                    Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
                        push(&mut pending, Visit::Node(b, depth + 1))?;
                        push(&mut pending, Visit::Node(a, depth + 1))?;
                    }
                    Db::Fix(body) => push(&mut pending, Visit::Node(body, depth + 1))?,
                    Db::Let(a, b, c) => {
                        push(&mut pending, Visit::Node(c, depth + 1))?;
                        push(&mut pending, Visit::Node(b, depth + 1))?;
                        push(&mut pending, Visit::Node(a, depth + 1))?;
                    }
                    Db::MutualFix { defs, .. } => {
                        push(&mut pending, Visit::Slice(defs.iter(), depth + 1))?
                    }
                    Db::Match {
                        disc,
                        motive,
                        cases,
                    } => {
                        push(&mut pending, Visit::Slice(cases.iter(), depth + 1))?;
                        push(&mut pending, Visit::Node(motive, depth + 1))?;
                        push(&mut pending, Visit::Node(disc, depth + 1))?;
                    }
                }
            }
        }
    }
    Some(nodes)
}

enum NatOrderTail<'a> {
    Borrowed(&'a Db),
    Owned(Db),
}
struct NatOrderCursor<'a> {
    count: logicaffeine_base::BigInt,
    tail: Option<NatOrderTail<'a>>,
    prefix: Option<PrefixOwned>,
}
impl<'a> NatOrderCursor<'a> {
    fn new(t: &'a Db, strict: bool) -> Self {
        Self {
            count: if strict { 1.into() } else { 0.into() },
            tail: Some(NatOrderTail::Borrowed(t)),
            prefix: None,
        }
    }
    fn recognize(t: NatOrderTail<'a>) -> Result<Self, NatOrderTail<'a>> {
        use NatOrderTail::{Borrowed, Owned};
        match t {
            Borrowed(Db::Global(n)) if n == "Zero" => Ok(Self {
                count: 0.into(),
                tail: None,
                prefix: None,
            }),
            Borrowed(Db::Lit(Literal::Nat(n))) => Ok(Self {
                count: if n.is_negative() { 0.into() } else { n.clone() },
                tail: None,
                prefix: None,
            }),
            Borrowed(Db::App(f, a)) if matches!(f.as_ref(),Db::Global(n) if n=="Succ") => {
                Ok(Self {
                    count: 1.into(),
                    tail: Some(Borrowed(a)),
                    prefix: None,
                })
            }
            Owned(Db::Global(n)) if n == "Zero" => Ok(Self {
                count: 0.into(),
                tail: None,
                prefix: None,
            }),
            Owned(Db::Lit(Literal::Nat(n))) => Ok(Self {
                count: if n.is_negative() { 0.into() } else { n },
                tail: None,
                prefix: None,
            }),
            Owned(Db::App(f, a)) if matches!(f.as_ref(),Db::Global(n) if n=="Succ") => Ok(Self {
                count: 1.into(),
                tail: Some(Owned(a.into_db())),
                prefix: None,
            }),
            other => Err(other),
        }
    }
    fn expose(&mut self, genv: &Context, lctx: &dyn LocalTypes) -> Option<()> {
        if !nat_order_tick() {
            return None;
        }
        let Some(tail) = self.tail.take() else {
            return Some(());
        };
        let mut next = match Self::recognize(tail) {
            Ok(next) => next,
            Err(tail) => {
                // Every computed exposure invalidates the certificate, even
                // when WHNF returns syntactically unchanged syntax.
                if let Some(owned) = self.prefix.as_mut() {
                    owned.clean = false;
                }
                let reduced = match tail {
                    NatOrderTail::Borrowed(t) => {
                        let nodes = nat_order_admit(t)?;
                        // Reserve actual clone work before taking ownership.
                        // Keep a dispatch available before invoking WHNF.
                        if !nat_order_spend(nodes.checked_add(1)?) {
                            return None;
                        }
                        whnf_in(genv, lctx, t)
                    }
                    NatOrderTail::Owned(t) => {
                        if !nat_order_tick() {
                            return None;
                        }
                        whnf_in_owned(genv, lctx, t)
                    }
                };
                if WHNF_EXHAUSTED.with(|e| e.get()) {
                    return None;
                }
                Self::recognize(NatOrderTail::Owned(reduced)).ok()?
            }
        };
        next.prefix = self.prefix.take();
        *self = next;
        Some(())
    }
}

/// An independent lazy prefix decision for the exactly attested Nat order
/// program. Unknown tails retain ordinary reduction; no proof fields are erased.
fn try_nat_order_db_in(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    let Db::App(f1, right) = t else {
        return None;
    };
    let Db::App(f0, left) = f1.as_ref() else {
        return None;
    };
    let Db::Global(name) = f0.as_ref() else {
        return None;
    };
    let strict = match name.as_str() {
        "natLe" => false,
        "natLt" => true,
        _ => return None,
    };
    if WHNF_EXHAUSTED.with(|e| e.get()) {
        return None;
    }
    if !nat_order_tick() {
        return None;
    }
    let parent = WHNF_DEPTH.with(|d| d.get());
    if parent == 0 || parent >= MAX_RECHECK_WHNF_DEPTH {
        WHNF_EXHAUSTED.with(|e| e.set(true));
        return None;
    }
    let _depth_guard = WhnfDepthGuard(parent);
    WHNF_DEPTH.with(|d| d.set(parent + 1));
    if !genv.simp_rules().is_empty()
        || genv.constructor_inventory_upper_bound("Nat") > 4096
        || genv.constructor_inventory_upper_bound("Bool") > 4096
    {
        return None;
    }
    let names: &[&str] = if strict {
        &["natLe", "natLt"]
    } else {
        &["natLe"]
    };
    let admitted = |t: &Term| {
        matches!(crate::reduction::bounded_conversion_metrics(t),
        Some((_,depth,true)) if depth<=crate::type_checker::MAX_TERM_DEPTH)
    };
    if !names.iter().all(|name| {
        genv.poly_declaration(name).is_none()
            && !genv.is_constructor(name)
            && !genv.is_inductive(name)
            && genv.get_global(name) == genv.get_definition_type(name)
            && genv.get_definition_type(name).is_some_and(admitted)
            && genv.get_definition_body(name).is_some_and(admitted)
    }) || !crate::reduction::accelerates_nat_order(genv, name)
    {
        return None;
    }
    run_nat_order_db(
        genv,
        lctx,
        left,
        right,
        strict,
        ZeroCase::Uniform,
        "true",
        "false",
    )
}

/// How a certified order answers once its left operand is exhausted. `Uniform`: its equation
/// `f 0 n = yes` was proved for a free `n`, so it holds by reduction whatever the right operand
/// is. `Split`: only `f 0 0 = yes` and `f 0 (n+1) = yes` were proved (Lean's `Nat.ble` matches on
/// both operands), so the answer holds by reduction only once the right operand is observed to be
/// `Zero` or a successor — for a right operand stuck on a local variable the program is stuck too.
#[derive(Clone, Copy, PartialEq)]
enum ZeroCase {
    Uniform,
    Split,
}

fn run_nat_order_db(
    genv: &Context,
    lctx: &dyn LocalTypes,
    left: &Db,
    right: &Db,
    strict: bool,
    zero_case: ZeroCase,
    yes: &str,
    no: &str,
) -> Option<Db> {
    let mut a = NatOrderCursor::new(left, false);
    if strict {
        a.count = 1.into();
    }
    let mut b = NatOrderCursor::new(right, false);
    let mut prefix_started = false;
    let mut prefix_seed = None;
    loop {
        if !nat_order_tick() {
            return None;
        }
        let left_exhausted = |a: &NatOrderCursor| a.count == 0.into() && a.tail.is_none();
        if a.count == 0.into() {
            if a.tail.is_some() {
                a.expose(genv, lctx)?;
            }
            let right_observed =
                zero_case == ZeroCase::Uniform || b.count > 0.into() || b.tail.is_none();
            if left_exhausted(&a) && right_observed {
                prefix_publish(genv, &mut prefix_seed, &mut b);
                if WHNF_EXHAUSTED.with(|e| e.get()) {
                    return None;
                }
                return Some(Db::Global(yes.into()));
            }
        }
        if b.count == 0.into() {
            // Lookup only when the original left-first machine demands right.
            if !prefix_started {
                prefix_started = true;
                if let Some((seed, cached)) = prefix_lookup(genv, right) {
                    prefix_seed = Some(seed);
                    if let Some(cursor) = cached {
                        b = cursor;
                    }
                }
                if WHNF_EXHAUSTED.with(|e| e.get()) {
                    return None;
                }
            }
            if b.count == 0.into() && b.tail.is_some() {
                b.expose(genv, lctx)?;
                prefix_observe(&mut prefix_seed, &b.count);
            }
            // An exhausted left operand answers from the observed right one: `f 0 0` is `yes`.
            if left_exhausted(&a) {
                continue;
            }
            if b.count == 0.into() && b.tail.is_none() {
                prefix_publish(genv, &mut prefix_seed, &mut b);
                if WHNF_EXHAUSTED.with(|e| e.get()) {
                    return None;
                }
                return Some(Db::Global(no.into()));
            }
        }
        let common = std::cmp::min(a.count.clone(), b.count.clone());
        a.count = a.count.sub(&common);
        b.count = b.count.sub(&common);
    }
}

// This kernel earns its own certificate. The three recurrence equations are
// checked with de Bruijn conversion, under the existing shared fuel budget;
// neither the named kernel's equality function nor its certificate is used.
fn try_certified_nat_order_db_in(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    if genv.checking_order_recurrence
        || !genv.simp_rules().is_empty()
        || WHNF_EXHAUSTED.with(|e| e.get())
    {
        return None;
    }
    let Db::App(f, right) = t else {
        return None;
    };
    let Db::App(head, left) = f.as_ref() else {
        return None;
    };
    let Db::Global(name) = head.as_ref() else {
        return None;
    };
    if genv.poly_declaration(name).is_some() || genv.is_inductive(name) || genv.is_constructor(name)
    {
        return None;
    }
    let signature = genv.get_definition_type(name)?;
    if genv.get_global(name) != Some(signature) {
        return None;
    }
    let mut result = signature;
    for _ in 0..2 {
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = result
        else {
            return None;
        };
        if !matches!(&**param_type,Term::Global(n) if n=="Nat") {
            return None;
        }
        result = body_type;
    }
    let Term::Global(carrier) = result else {
        return None;
    };
    if genv.constructor_inventory_upper_bound("Nat") > 4096
        || !crate::int_inductive::canonical_nat_environment(genv)
        || !genv.is_inductive(carrier)
        || genv.get_global(carrier) != Some(&Term::Sort(Universe::Type(0)))
        || genv.get_definition_body(carrier).is_some()
        || genv.poly_declaration(carrier).is_some()
        || genv.inductive_num_params(carrier) != 0
        || genv.constructor_inventory_upper_bound(carrier) != 2
    {
        return None;
    }
    let constructors = genv.get_constructors(carrier);
    if constructors.len() != 2
        || constructors.iter().any(|(n, ty)| {
            *ty != result
                || genv.get_global(n) != Some(result)
                || genv.constructor_inductive(n) != Some(carrier.as_str())
                || genv.poly_declaration(n).is_some()
                || genv.get_definition_body(n).is_some()
        })
    {
        return None;
    }
    let body = genv.get_definition_body(name)?;
    // Bound annotations and universe trees before recursive de Bruijn lowering.
    // Admission spends the caller's fuel even when the optional probe refuses.
    let limit = WHNF_FUEL.with(|fuel| fuel.get().min(4096));
    let mut admission = LocalAdmission::new(limit);
    let admitted = admission.free_names(body);
    if !nat_order_spend(limit - admission.remaining + 1) {
        return None;
    }
    if admission.max_depth > MAX_RECHECK_WHNF_DEPTH || !admitted.ok()?.is_empty() {
        return None;
    }
    // Independent lowering rejects free variables and holes before a program
    // can be considered closed. Retain no body across calls or context changes.
    to_db(body, &mut Vec::new()).ok()?;
    let parent = WHNF_DEPTH.with(|d| d.get());
    if parent == 0 || parent >= MAX_RECHECK_WHNF_DEPTH {
        return None;
    }
    let _depth = WhnfDepthGuard(parent);
    WHNF_DEPTH.with(|d| d.set(parent + 1));
    let probe = genv.order_recurrence_context();
    let variables = vec![Db::Global("Nat".into()), Db::Global("Nat".into())];
    let apply = |f: Db, a: Db| Db::App(DbBox::new(f), DbBox::new(a));
    let call = |a: Db, b: Db| apply(apply(Db::Global(name.clone()), a), b);
    let succ = |a: Db| apply(Db::Global("Succ".into()), a);
    let base = call(Db::Global("Zero".into()), Db::Var(0));
    // `f 0 n = yes` for every n, or, for a program matching on both arguments (Lean's
    // `Nat.ble`), its two cases `f 0 0 = yes` and `f 0 (n+1) = yes`.
    let (yes, zero_case) = match constructors
        .iter()
        .position(|(n, _)| def_eq(&probe, &variables, &base, &Db::Global((*n).into())))
    {
        Some(yes) => (yes, ZeroCase::Uniform),
        None => {
            let at_zero = call(Db::Global("Zero".into()), Db::Global("Zero".into()));
            let at_successor = call(Db::Global("Zero".into()), succ(Db::Var(0)));
            (
                constructors.iter().position(|(n, _)| {
                    def_eq(&probe, &variables, &at_zero, &Db::Global((*n).into()))
                        && def_eq(&probe, &variables, &at_successor, &Db::Global((*n).into()))
                })?,
                ZeroCase::Split,
            )
        }
    };
    let no = 1 - yes;
    if !def_eq(
        &probe,
        &variables,
        &call(succ(Db::Var(1)), Db::Global("Zero".into())),
        &Db::Global(constructors[no].0.into()),
    ) || !def_eq(
        &probe,
        &variables,
        &call(succ(Db::Var(1)), succ(Db::Var(0))),
        &call(Db::Var(1), Db::Var(0)),
    ) || WHNF_EXHAUSTED.with(|e| e.get())
    {
        return None;
    }
    run_nat_order_db(
        genv,
        lctx,
        left,
        right,
        false,
        zero_case,
        constructors[yes].0,
        constructors[no].0,
    )
}

/// The EUCLIDEAN division pair on `i64` — this kernel's own implementation of the shared
/// specification, written from the rule rather than copied from `reduction::ediv_emod_i64`.
///
/// **The specification, restated.** For a dividend `a` and a divisor `b ≠ 0`, `div a b` and
/// `mod a b` are the unique `q` and `r` with
///
/// * `a = b·q + r`, and
/// * `0 ≤ r < |b|` — the remainder is NEVER negative, whatever the sign of either operand.
///
/// That second clause is the whole content: truncating division (Rust's `/` and `%`, and
/// `BigInt::div_rem`) instead gives the remainder the DIVIDEND's sign, so the two conventions
/// agree on every non-negative dividend and part company on every negative one
/// (`-7 / 2` is `-4` here and `-3` there). Lean totalises the rule at a zero divisor with
/// `a / 0 = 0` and `a % 0 = a`, which is the only pair that keeps `b·q + r = a` when `b` is
/// zero — and the reason a zero divisor computes here rather than sticking.
///
/// **Why this is not the main kernel's function.** `reduction::ediv_emod_i64` reaches the pair
/// by taking the TRUNCATED quotient and correcting it by one when the remainder came out
/// negative. This one reaches it from the other end: it takes the residue of `a` against the
/// divisor's MAGNITUDE — the `0 ≤ r < |b|` clause read literally — and then recovers `q` by an
/// exact division of `a - r`, a quantity that is divisible by `b` by construction and so rounds
/// nowhere and needs no correction. Two routes to one specification is the entire point of a
/// second kernel; a copied function would make one wrong correction wrong in both, which is the
/// failure mode this campaign has already hit seven times.
///
/// `None` means the narrow type cannot hold an intermediate (`|i64::MIN|`, or the `a - r` it
/// would need), and the caller redoes the operation exactly in `BigInt` — so the narrow path
/// and the wide path answer identically on every value both can represent.
fn euclidean_pair_i64(a: i64, b: i64) -> Option<(i64, i64)> {
    if b == 0 {
        return Some((0, a));
    }
    let magnitude = b.checked_abs()?;
    // `a % magnitude` lands in `(-magnitude, magnitude)`; one addition moves a negative
    // residue into `[0, magnitude)` and a non-negative one is already there.
    let residue = a % magnitude;
    let r = if residue < 0 {
        residue.checked_add(magnitude)?
    } else {
        residue
    };
    let q = a.checked_sub(r)?.checked_div(b)?;
    Some((q, r))
}

/// The arbitrary-precision half of the SAME specification, by the same residue-first route, so
/// the two widths cannot disagree about a value both can represent. Total at a zero divisor in
/// exactly the way the narrow path is.
fn euclidean_pair_big(
    a: &logicaffeine_base::BigInt,
    b: &logicaffeine_base::BigInt,
) -> Option<(logicaffeine_base::BigInt, logicaffeine_base::BigInt)> {
    use logicaffeine_base::BigInt;
    if b.is_zero() {
        return Some((BigInt::zero(), a.clone()));
    }
    let magnitude = b.abs();
    let (_, residue) = a.div_rem(&magnitude)?;
    let r = if residue.is_negative() {
        residue.add(&magnitude)
    } else {
        residue
    };
    let (q, _) = a.sub(&r).div_rem(b)?;
    Some((q, r))
}

/// A `Bool` constructor as this kernel spells it.
fn db_bool(b: bool) -> Db {
    Db::Global(if b { "true" } else { "false" }.to_string())
}

/// Whether `op` names a rule in the BINARY primitive table.
///
/// This question is asked at every application node of every reduction step and the answer is
/// almost always "no", so it must be one name test and no allocation.
fn is_binary_primitive(op: &str) -> bool {
    matches!(
        op,
        "add"
            | "sub"
            | "mul"
            | "div"
            | "mod"
            | "le"
            | "lt"
            | "ge"
            | "gt"
            | "Text_append"
            | "Text_beq"
            | "Text_isPrefixOf"
    )
}

/// Whether `op` names a rule in the UNARY primitive table.
fn is_unary_primitive(op: &str) -> bool {
    matches!(op, "Text_length")
}

/// Whether a DEFINITION of `op` outranks the primitive rule of the same name.
///
/// A primitive fires on a NAME, so it must never beat a body the term was type-checked against:
/// a context that defines `Text_append` has to be reduced by what it defined, or the checker
/// disagrees with the very declaration it accepted. The question is asked only once a name has
/// already matched the table, so the common path never pays for it.
fn shadowed_by_a_definition(genv: &Context, op: &str) -> bool {
    genv.get_definition_body(op).is_some()
}

/// Whether the context's DECLARATION of `op` agrees with the type the rule's answer inhabits.
///
/// SUBJECT REDUCTION, the other half of the same gate. A rule fires on a NAME and a pair of
/// literal kinds; nothing here used to look at the declared TYPE. So a context declaring
/// `Text_append : Text → Text → Nat` got a kernel where `infer` answered `Nat` and `whnf`
/// produced a `Text` literal — a term and its reduct with different types, which is precisely
/// what a conversion check may not have, since every use of a reduct in place of its term
/// assumes the two are interchangeable.
///
/// The DECISION is shared with the first kernel on purpose — `reduction::declaration_agrees_with_the_rule`
/// reads the same signature table — for the same reason `term::resolve_const_levels` is shared:
/// whether a rule APPLIES is a fact about the context, and two kernels that answered it
/// differently would be computing two different programs for one closed term. The arithmetic
/// each rule then performs stays this kernel's own, which is where independence buys something.
fn declaration_agrees_with_the_rule(genv: &Context, op: &str) -> bool {
    crate::reduction::declaration_agrees_with_the_rule(genv, op)
}

/// The `Int` computation rules: a machine fast path that PROMOTES on overflow rather than
/// wrapping, and order decided directly. `None` means "no rule for this name at these literal
/// kinds", which is how a `Nat`, `Text` or `Float` operand leaves the term stuck instead of
/// being silently read as an integer.
fn int_binary_db(op: &str, xl: &Literal, yl: &Literal) -> Option<Db> {
    if let (Literal::Int(x), Literal::Int(y)) = (xl, yl) {
        let fast = match op {
            "add" => x.checked_add(*y),
            "sub" => x.checked_sub(*y),
            "mul" => x.checked_mul(*y),
            "div" => euclidean_pair_i64(*x, *y).map(|(q, _)| q),
            "mod" => euclidean_pair_i64(*x, *y).map(|(_, r)| r),
            _ => None,
        };
        if let Some(r) = fast {
            return Some(Db::Lit(Literal::Int(r)));
        }
        match op {
            "le" => return Some(db_bool(*x <= *y)),
            "lt" => return Some(db_bool(*x < *y)),
            "ge" => return Some(db_bool(*x >= *y)),
            "gt" => return Some(db_bool(*x > *y)),
            _ => {}
        }
    }
    // EXACT PATH: arbitrary precision, reached when an operand is a `BigInt` or an `i64` op
    // overflowed. Canonicalised by `int_lit` (demoted to `Int` when it fits), so a value has ONE
    // literal and definitional equality of literals stays sound.
    let (xb, yb) = (lit_bigint(xl)?, lit_bigint(yl)?);
    let big = match op {
        "add" => Some(xb.add(&yb)),
        "sub" => Some(xb.sub(&yb)),
        "mul" => Some(xb.mul(&yb)),
        "div" => euclidean_pair_big(&xb, &yb).map(|(q, _)| q),
        "mod" => euclidean_pair_big(&xb, &yb).map(|(_, r)| r),
        _ => None,
    };
    if let Some(r) = big {
        return Some(Db::Lit(int_lit(r)));
    }
    match op {
        "le" => Some(db_bool(xb <= yb)),
        "lt" => Some(db_bool(xb < yb)),
        "ge" => Some(db_bool(xb >= yb)),
        "gt" => Some(db_bool(xb > yb)),
        _ => None,
    }
}

/// The `Text` computation rules, from the same specification the main kernel implements:
/// `Text_append` concatenates, `Text_beq` decides equality of the two strings, and
/// `Text_isPrefixOf a b` holds exactly when `a` is an initial segment of `b`. Anything that is
/// not two `Text` literals has no rule at all, so an `Int` operand leaves the term stuck rather
/// than being read as text.
fn text_binary_db(op: &str, xl: &Literal, yl: &Literal) -> Option<Db> {
    let (Literal::Text(a), Literal::Text(b)) = (xl, yl) else {
        return None;
    };
    match op {
        "Text_append" => Some(Db::Lit(Literal::Text(format!("{a}{b}")))),
        "Text_beq" => Some(db_bool(a == b)),
        "Text_isPrefixOf" => Some(db_bool(b.starts_with(a.as_str()))),
        _ => None,
    }
}

/// `Text_length` counts CHARACTERS, as Lean's `String.length` does — not bytes, so one
/// multi-byte character counts once. The only unary rule in either kernel's table.
fn text_unary_db(op: &str, xl: &Literal) -> Option<Db> {
    match (op, xl) {
        ("Text_length", Literal::Text(s)) => Some(Db::Lit(Literal::Nat(
            logicaffeine_base::BigInt::from_u64(s.chars().count() as u64),
        ))),
        _ => None,
    }
}

/// Reduce a fully-applied primitive on literal arguments — the main kernel's built-in
/// computation rules, implemented here INDEPENDENTLY so the re-checker reaches the same value
/// by its own arithmetic (`le 2 3 ⤳ true`, `add 2 4 ⤳ 6`, `Text_append "ab" "cd" ⤳ "abcd"`).
/// The arguments are themselves reduced first, so `le (add 1 1) 3` works. `None` if `t` is not
/// such an application — the answer at almost every node, reached by one pointer walk and one
/// name test.
///
/// **The table is part of the shared specification, not a per-kernel convenience.** A rule the
/// main kernel has and this one does not is NOT a safe shortfall: the term stays stuck here, the
/// conversion check fails on one side only, and `double_check` reports `Disagree` — a soundness
/// alarm — about a proof that is perfectly valid. That is the same argument `MAX_RECHECK_STEPS`
/// makes about the step budget, and it applies to every row of this table: a new primitive in
/// one engine is a new row in ALL of them, each written from the rule rather than copied.
fn try_builtin(genv: &Context, t: &Db) -> Option<Db> {
    // The SHAPE and the operation NAME are checked by reference first, before any argument is
    // reduced and before any map is consulted.
    let Db::App(head, last) = t else { return None };
    match head.as_ref() {
        // Binary: `(op x) y`.
        Db::App(f0, first) => {
            let Db::Global(name) = f0.as_ref() else {
                return None;
            };
            let op = name.as_str();
            if !is_binary_primitive(op)
                || (shadowed_by_a_definition(genv, op)
                    && !crate::int_inductive::accelerates_integer_alias(genv, op))
                || !declaration_agrees_with_the_rule(genv, op)
            {
                return None;
            }
            let operand = |t: &Db| match whnf(genv, t) {
                Db::Lit(literal) => Some(literal),
                constructor => db_int_literal(genv, &constructor),
            };
            let (x, y) = (operand(first)?, operand(last)?);
            int_binary_db(op, &x, &y).or_else(|| text_binary_db(op, &x, &y))
        }
        // Unary: `op x`.
        Db::Global(name) => {
            let op = name.as_str();
            if !is_unary_primitive(op)
                || shadowed_by_a_definition(genv, op)
                || !declaration_agrees_with_the_rule(genv, op)
            {
                return None;
            }
            let Db::Lit(x) = whnf(genv, last.as_ref()) else {
                return None;
            };
            text_unary_db(op, &x)
        }
        _ => None,
    }
}

/// ι for a fixpoint, asked of the WHOLE spine: `(fix f. λx₀ … λxₙ. e) a₀ … aₙ` unrolls when the
/// argument at the STRUCTURAL column is constructor-headed.
///
/// Both the inner Fix application and the restored outer application spine
/// ask this helper. An inner application may not yet supply the recursive
/// column; a constructor in another position must not unfold a neutral fixpoint.
/// This preserves the defining recursion boundary while later arguments arrive.
///
/// WHICH COLUMN is a shared DECISION, not shared arithmetic: `termination::structural_position`
/// is the guard's own answer about this body, and its doc comment names `reduction.rs` and this
/// module as its two callers for exactly this reason. Two kernels that picked different columns
/// would unroll different fixpoints — one of them past its guard, which is how a normaliser
/// diverges. The unrolling below is this kernel's own, over de Bruijn levels.
///
/// No demand exists when the head is not a `Fix`, the guard admits no column,
/// or a partial application has not supplied that column. Constructor admission
/// occurs only after the scheduled argument has independently reached weak head.
fn fix_structural_column(genv: &Context, t: &Db) -> Option<usize> {
    let Db::Fix(body) = spine_head(t) else {
        return None;
    };
    let mut domains = Vec::new();
    let mut core = body.as_ref();
    while let Db::Lam(domain, inner) = core {
        domains.push(domain.as_ref());
        core = inner;
    }
    // The fix binder precedes the lambda telescope. An index outside that
    // telescope names the recursive binder or an external local, never a column.
    // The innermost `match` is read through an application spine, as the guard reads it.
    let preferred = match spine_head(core) {
        Db::Match { disc, .. } => match disc.as_ref() {
            Db::Var(index) => index
                .checked_add(1)
                .and_then(|i| domains.len().checked_sub(i)),
            _ => None,
        },
        _ => None,
    };
    let candidates = domains.iter().enumerate().map(|(position, domain)| {
        let inductive = matches!(spine_head(domain),
            Db::Global(name) | Db::Const { name, .. } if genv.is_inductive(name));
        (position, inductive)
    });
    let column = crate::termination::select_structural_position(candidates, preferred)?;
    let mut supplied = 0usize;
    let mut head = t;
    while let Db::App(function, _) = head {
        supplied += 1;
        head = function;
    }
    (column < supplied).then_some(column)
}

/// The quotient computation rule in de Bruijn form: `Quot_lift A r B f h (Quot_mk A r a) ⤳
/// f a`, and `Quot.ind`'s. Returns the reduct when `t` is exactly that spine, else `None`. The
/// major premise is weak-head normalized in `t`'s own local context, so a representative that
/// only δ exposes (`Quotient.mk s a := Quot.mk Setoid.r a`) is recognized.
fn try_quot_lift_db(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    let (arity, universe) = match spine_head(t) {
        Db::Global(n) => {
            let arity = match crate::quotient::prelude_kind(genv, n)? {
                crate::quotient::PreludeKind::Lift => 6,
                crate::quotient::PreludeKind::Induction => 5,
                _ => return None,
            };
            (arity, None)
        }
        Db::Const { name, levels } => {
            let arity = match crate::quotient::kind(genv, name, levels)? {
                crate::quotient::Kind::Lift => 6,
                crate::quotient::Kind::Induction => 5,
                _ => return None,
            };
            (arity, Some(&levels[0]))
        }
        _ => return None,
    };
    let (_, args) = spine_ref(t);
    if args.len() != arity {
        return None;
    }
    let q = whnf_in(genv, lctx, args[arity - 1]);
    let constructor = match (spine_head(&q), universe) {
        (Db::Global(n), None) => {
            crate::quotient::prelude_kind(genv, n)
                == Some(crate::quotient::PreludeKind::Constructor)
        }
        (Db::Const { name, levels }, Some(u)) => {
            crate::quotient::kind(genv, name, levels) == Some(crate::quotient::Kind::Constructor)
                && levels[0].equiv(u)
        }
        _ => false,
    };
    let (_, qa) = spine_ref(&q);
    if !constructor || qa.len() != 3 {
        return None;
    }
    Some(Db::App(
        DbBox::new(args[3].clone()),
        DbBox::new(qa[2].clone()),
    ))
}

// ---------------------------------------------------------------------------
// THE PRIMITIVE RULE TABLE
// ---------------------------------------------------------------------------

/// What THIS kernel does with one of the first kernel's registered reduction rules.
///
/// The prelude DECLARES the reflection and tactic primitives, so they are inside the trusted
/// base and a proof may cite what they compute. A rule the main kernel computes and this one
/// walks past is NOT a safe shortfall: the redex stays a value here, the conversion check fails
/// on one side only, and [`double_check`] reports `Disagree` — a SOUNDNESS ALARM — about a proof
/// that is perfectly valid. An alarm that fires on valid proofs is an alarm nobody reads.
///
/// There are exactly TWO honest answers for a primitive this kernel is handed, and this table
/// says which one each rule gets. "Quietly treat it as stuck and then disagree" is neither.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RuleKind {
    /// This kernel computes the rule ITSELF, from the written specification, over its own de
    /// Bruijn representation — so an agreement is corroboration rather than an echo.
    Checked,
    /// This kernel does NOT compute the rule, and says so: reaching one sets the exhaustion
    /// flag, exactly as a spent step budget does, so a later rejection is reported as
    /// [`ReCheckError::Unsupported`] — honestly single-checked — instead of as a disagreement.
    ///
    /// Every row marked this way runs an ENGINE in the first kernel (ring normalisation,
    /// Fourier–Motzkin, congruence closure, a derivation checker). A second kernel that
    /// transcribed one of those would be the same implementation twice, which corroborates
    /// nothing while looking exactly like corroboration — the failure mode this campaign found
    /// six times in the universe algebra, where both kernels call one function.
    Unchecked,
}

/// Every reduction rule the FIRST kernel dispatches on, with the number of arguments it
/// consumes and what this kernel does with it.
///
/// `U_universes_extra::the_second_kernels_primitive_table_names_every_primitive_the_first_kernel_dispatches_on`
/// reads `reduction.rs`'s own dispatcher and fails if a name is missing here, so a rule added to
/// one kernel cannot stay silently absent from the other.
const PRIMITIVE_RULES: &[(&str, usize, RuleKind)] = &[
    // THE REFLECTION ALGEBRA's evaluator — a recursion over the `Syntax` grammar, so its
    // specification is written down in full and this kernel implements it independently. The
    // operations `reflection` defines are not rules here: this kernel computes them by their
    // definitions, which checks the first kernel's accelerators of them.
    ("syn_step", 1, RuleKind::Checked),
    ("syn_eval", 2, RuleKind::Checked),
    // Certificate-backed successes are checked independently. Unsupported goal/derivation
    // shapes still return Unchecked, including the remaining derivation constructors.
    ("concludes", 1, RuleKind::Checked),
    ("try_refl", 1, RuleKind::Unchecked),
    ("try_compute", 1, RuleKind::Unchecked),
    ("try_ring", 1, RuleKind::Checked),
    ("try_lia", 1, RuleKind::Checked),
    ("try_cc", 1, RuleKind::Checked),
    ("try_simp", 1, RuleKind::Unchecked),
    ("try_omega", 1, RuleKind::Unchecked),
    ("try_auto", 1, RuleKind::Unchecked),
    ("try_decide", 1, RuleKind::Unchecked),
    ("try_assumption", 1, RuleKind::Unchecked),
    ("try_split", 1, RuleKind::Unchecked),
    ("try_left", 1, RuleKind::Unchecked),
    ("try_right", 1, RuleKind::Unchecked),
    ("try_bitblast", 1, RuleKind::Unchecked),
    ("try_tabulate", 1, RuleKind::Unchecked),
    ("try_hw_auto", 1, RuleKind::Unchecked),
    ("try_inversion", 1, RuleKind::Unchecked),
    ("tact_fail", 1, RuleKind::Unchecked),
    ("induction_num_cases", 1, RuleKind::Unchecked),
    ("try_cong", 2, RuleKind::Unchecked),
    ("try_rewrite", 2, RuleKind::Unchecked),
    ("try_rewrite_rev", 2, RuleKind::Unchecked),
    ("try_script", 2, RuleKind::Unchecked),
    ("induction_base_goal", 2, RuleKind::Unchecked),
    ("tact_try", 2, RuleKind::Unchecked),
    ("tact_repeat", 2, RuleKind::Unchecked),
    ("tact_solve", 2, RuleKind::Unchecked),
    ("tact_first", 2, RuleKind::Unchecked),
    ("tact_orelse", 3, RuleKind::Unchecked),
    ("tact_then", 3, RuleKind::Unchecked),
    ("induction_step_goal", 3, RuleKind::Unchecked),
    ("try_induction", 3, RuleKind::Unchecked),
    ("try_destruct", 3, RuleKind::Unchecked),
    ("try_apply", 3, RuleKind::Unchecked),
];

/// The names in `PRIMITIVE_RULES` — the reading surface the cross-kernel ratchet needs, so the
/// table can be compared against the first kernel's dispatcher from a test rather than by eye.
pub fn primitive_rule_names() -> Vec<&'static str> {
    PRIMITIVE_RULES.iter().map(|(n, _, _)| *n).collect()
}

/// The arity and treatment of a primitive rule, or `None` when the name is not one.
fn primitive_rule(name: &str) -> Option<(usize, RuleKind)> {
    PRIMITIVE_RULES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, arity, kind)| (*arity, *kind))
}

// ---------------------------------------------------------------------------
// THE REFLECTION ALGEBRA, IMPLEMENTED INDEPENDENTLY
// ---------------------------------------------------------------------------
//
// `Syntax` is a first-order grammar — `SVar n`, `SGlobal n`, `SSort u`, `SLit n`, `SName s`,
// `SApp f x`, `SLam T b`, `SPi T B`. The operations `reflection` defines over it are checked
// definitions, which this kernel computes by unfolding; the evaluator (`syn_step`, `syn_eval`) is
// a rule whose answer the grammar fixes, written down in full (in `reduction.rs`'s doc comments
// and in this module's), so this kernel computes it from the rule rather than by transcribing a
// function, with its own substitution below.
//
// Tactic success cases use the independent certificate module. The remaining primitives
// explicitly report incomplete checking.

/// A `Syntax` node with ONE argument: `(Ctor arg)`.
fn s_unary(t: &Db) -> Option<(&str, &Db)> {
    let Db::App(f, a) = t else { return None };
    let Db::Global(n) = f.as_ref() else {
        return None;
    };
    Some((n.as_str(), a.as_ref()))
}

/// A `Syntax` node with TWO arguments: `((Ctor a) b)`.
fn s_binary(t: &Db) -> Option<(&str, &Db, &Db)> {
    let Db::App(f, b) = t else { return None };
    let Db::App(c, a) = f.as_ref() else {
        return None;
    };
    let Db::Global(n) = c.as_ref() else {
        return None;
    };
    Some((n.as_str(), a.as_ref(), b.as_ref()))
}

/// The machine integer a reflected numeral carries — the ONLY literal kind the `Syntax` layer's
/// `SVar`/`SLit`/`SGlobal` indices are written at, so a `Nat` or `BigInt` operand leaves the
/// rule stuck instead of being read as one.
fn s_int(t: &Db) -> Option<i64> {
    match t {
        Db::Lit(Literal::Int(n)) => Some(*n),
        _ => None,
    }
}

fn s_mk1(ctor: &str, a: Db) -> Db {
    Db::App(DbBox::new(Db::Global(ctor.to_string())), DbBox::new(a))
}

fn s_mk2(ctor: &str, a: Db, b: Db) -> Db {
    Db::App(DbBox::new(s_mk1(ctor, a)), DbBox::new(b))
}

fn s_lit(n: i64) -> Db {
    s_mk1("SLit", Db::Lit(Literal::Int(n)))
}

fn s_var(n: i64) -> Db {
    s_mk1("SVar", Db::Lit(Literal::Int(n)))
}

fn s_name(s: &str) -> Db {
    s_mk1("SName", Db::Lit(Literal::Text(s.to_string())))
}

/// `syn_lift amount cutoff t` — shift every free index at or above `cutoff` by `amount`.
///
/// Specification: `SVar k` becomes `SVar (k + amount)` when `k >= cutoff` and is untouched
/// otherwise; other leaves are untouched; `SApp` lifts both children at the same cutoff;
/// `SLam`/`SPi` lift the annotation at `cutoff` and the body at `cutoff + 1`, because the body
/// is one binder deeper.
fn syn_lift_db(amount: i64, cutoff: i64, t: &Db) -> Option<Db> {
    let _nest = nest(stack_mark())?;
    if let Some((c, a)) = s_unary(t) {
        match c {
            "SVar" => {
                let k = s_int(a)?;
                return Some(if k >= cutoff {
                    s_var(k.checked_add(amount)?)
                } else {
                    t.clone()
                });
            }
            "SGlobal" | "SSort" | "SLit" | "SName" => return Some(t.clone()),
            _ => {}
        }
    }
    if let Some((c, a, b)) = s_binary(t) {
        match c {
            "SApp" => {
                return Some(s_mk2(
                    "SApp",
                    syn_lift_db(amount, cutoff, a)?,
                    syn_lift_db(amount, cutoff, b)?,
                ));
            }
            "SLam" | "SPi" => {
                return Some(s_mk2(
                    c,
                    syn_lift_db(amount, cutoff, a)?,
                    syn_lift_db(amount, cutoff + 1, b)?,
                ));
            }
            _ => {}
        }
    }
    None
}

/// `t` with `replacement` written at `SVar index` and the binder `index` counted removed: the
/// substitution a β step of `syn_step` performs. Every surviving `SVar k` with `k > index` drops
/// to `k - 1`, because the binder it counted is gone; going under a binder lifts the replacement
/// by one at cutoff 0, so its own free variables keep pointing where they did.
fn syn_beta_db(replacement: &Db, index: i64, t: &Db) -> Option<Db> {
    let _nest = nest(stack_mark())?;
    if let Some((c, a)) = s_unary(t) {
        match c {
            "SVar" => {
                let k = s_int(a)?;
                return Some(if k == index {
                    replacement.clone()
                } else if k > index {
                    s_var(k - 1)
                } else {
                    t.clone()
                });
            }
            "SGlobal" | "SSort" | "SLit" | "SName" => return Some(t.clone()),
            _ => {}
        }
    }
    if let Some((c, a, b)) = s_binary(t) {
        match c {
            "SApp" => {
                return Some(s_mk2(
                    "SApp",
                    syn_beta_db(replacement, index, a)?,
                    syn_beta_db(replacement, index, b)?,
                ));
            }
            "SLam" | "SPi" => {
                let head = syn_beta_db(replacement, index, a)?;
                let lifted = syn_lift_db(1, 0, replacement)?;
                let body = syn_beta_db(&lifted, index + 1, b)?;
                return Some(s_mk2(c, head, body));
            }
            _ => {}
        }
    }
    None
}

/// Arithmetic on two reflected integer literals: `SApp (SApp (SName op) (SLit n)) (SLit m)`.
///
/// The reflected object language MODELS this kernel's own arithmetic — a proof door accepts
/// what it computes — so `div`/`mod` are the same TOTAL Euclidean pair the primitive table
/// uses, reached through this module's own [`euclidean_pair_i64`]. A reflected `div` that
/// truncated while the kernel's rounded the other way would certify `Eq Int (div (-7) 2) (-3)`,
/// a false equation with a kernel-checked proof in front of it.
fn syn_arith_db(func: &Db, arg: &Db) -> Option<Db> {
    let (sapp, op_node, n_node) = s_binary(func)?;
    if sapp != "SApp" {
        return None;
    }
    let (sname, op_lit) = s_unary(op_node)?;
    if sname != "SName" {
        return None;
    }
    let Db::Lit(Literal::Text(op)) = op_lit else {
        return None;
    };
    let (nc, n_lit) = s_unary(n_node)?;
    let (mc, m_lit) = s_unary(arg)?;
    if nc != "SLit" || mc != "SLit" {
        return None;
    }
    let (n, m) = (s_int(n_lit)?, s_int(m_lit)?);
    let r = match op.as_str() {
        "add" => n.checked_add(m),
        "sub" => n.checked_sub(m),
        "mul" => n.checked_mul(m),
        "div" => euclidean_pair_i64(n, m).map(|(q, _)| q),
        "mod" => euclidean_pair_i64(n, m).map(|(_, r)| r),
        _ => None,
    }?;
    Some(s_lit(r))
}

/// `syn_step` — ONE leftmost-outermost step of the reflected evaluator.
///
/// Specification, in order: reflected arithmetic on two literals; β on `SApp (SLam T b) a`;
/// δ/ι by REIFYING the application and reducing it with this kernel's own reducer; then the
/// congruence rules (step the function, else step the argument); and finally the term
/// unchanged, which is how a stuck term reports that it is a value.
///
/// The δ hop is where the two kernels could part company for a reason that is NOT a
/// disagreement: reduction here is metered, so a budget spent mid-way would hand back a PARTIAL
/// reduct and this kernel would compute a different `Syntax` value from a valid one. So the
/// meter is read: a hop whose reduction was cut short reports `Unchecked` instead of an answer.
fn syn_step_db(genv: &Context, t: &Db) -> PrimStep {
    let Some(_nest) = nest(stack_mark()) else {
        return PrimStep::Unchecked;
    };
    let Some((c, func, arg)) = s_binary(t) else {
        return PrimStep::Reduced(t.clone());
    };
    if c != "SApp" {
        return PrimStep::Reduced(t.clone());
    }
    if let Some(r) = syn_arith_db(func, arg) {
        return PrimStep::Reduced(r);
    }
    if let Some((lam, _annotation, body)) = s_binary(func) {
        if lam == "SLam" {
            return match syn_beta_db(arg, 0, body) {
                Some(r) => PrimStep::Reduced(r),
                None => PrimStep::Unchecked,
            };
        }
    }
    if let Some(reified) = syntax_to_db(t) {
        match db_normalize_checked(genv, &reified) {
            None => return PrimStep::Unchecked,
            Some(nf) if nf != reified => {
                if let Some(s) = db_to_syntax(&nf) {
                    return PrimStep::Reduced(s);
                }
            }
            Some(_) => {}
        }
    }
    match syn_step_db(genv, func) {
        PrimStep::Unchecked => return PrimStep::Unchecked,
        PrimStep::Reduced(f2) if f2 != *func => {
            return PrimStep::Reduced(s_mk2("SApp", f2, arg.clone()));
        }
        _ => {}
    }
    match syn_step_db(genv, arg) {
        PrimStep::Unchecked => return PrimStep::Unchecked,
        PrimStep::Reduced(a2) if a2 != *arg => {
            return PrimStep::Reduced(s_mk2("SApp", func.clone(), a2));
        }
        _ => {}
    }
    PrimStep::Reduced(t.clone())
}

/// `syn_eval fuel t` — iterate [`syn_step_db`] until nothing changes or the fuel runs out.
///
/// The fuel is written in the TERM, so an exhausted evaluation is a legitimate answer for the
/// object-language expression (the caller can see the bound it asked for) and the partial
/// reduct is returned, exactly as the first kernel returns it.
fn syn_eval_db(genv: &Context, fuel: i64, t: &Db) -> PrimStep {
    let mut cur = t.clone();
    let mut left = fuel;
    while left > 0 {
        match syn_step_db(genv, &cur) {
            PrimStep::Reduced(next) => {
                if next == cur {
                    return PrimStep::Reduced(cur);
                }
                cur = next;
            }
            other => return other,
        }
        left -= 1;
    }
    PrimStep::Reduced(cur)
}

/// Reify a reflected term into an ordinary de Bruijn term, so the δ/ι rules can run on it.
///
/// A reflected de Bruijn INDEX becomes an opaque global (`v0`, `g0`) rather than a `Db::Var`:
/// the index counts binders of the OBJECT language, which the reflected `SLam` does not open in
/// this one, so reading it as a real variable would bind it to the wrong thing. Opaque is the
/// faithful reading — it is exactly what the first kernel's reification does with its own named
/// representation, and it is why the δ hop only ever fires on variable-free reflected terms.
fn syntax_to_db(t: &Db) -> Option<Db> {
    let _nest = nest(stack_mark())?;
    if let Some((c, a)) = s_unary(t) {
        match c {
            "SName" => {
                let Db::Lit(Literal::Text(s)) = a else {
                    return None;
                };
                return Some(Db::Global(s.clone()));
            }
            "SLit" => return Some(Db::Lit(Literal::Int(s_int(a)?))),
            "SVar" => return Some(Db::Global(format!("v{}", s_int(a)?))),
            "SGlobal" => return Some(Db::Global(format!("g{}", s_int(a)?))),
            _ => {}
        }
    }
    if let Some((c, a, b)) = s_binary(t) {
        let (l, r) = (syntax_to_db(a)?, syntax_to_db(b)?);
        match c {
            "SApp" => return Some(Db::App(DbBox::new(l), DbBox::new(r))),
            "SLam" => return Some(Db::Lam(DbBox::new(l), DbBox::new(r))),
            "SPi" => return Some(Db::Pi(DbBox::new(l), DbBox::new(r))),
            _ => {}
        }
    }
    None
}

/// Reflect an ordinary term back into `Syntax`. The inverse of [`syntax_to_db`] on the fragment
/// both cover; `None` for anything the `Syntax` grammar cannot spell, so a reduct that left the
/// fragment leaves the rule stuck instead of being mis-reflected.
fn db_to_syntax(t: &Db) -> Option<Db> {
    let _nest = nest(stack_mark())?;
    Some(match t {
        Db::Global(n) => s_name(n),
        Db::Const { name, .. } => s_name(name),
        Db::App(f, x) => s_mk2("SApp", db_to_syntax(f)?, db_to_syntax(x)?),
        Db::Pi(d, b) => s_mk2("SPi", db_to_syntax(d)?, db_to_syntax(b)?),
        Db::Lam(d, b) => s_mk2("SLam", db_to_syntax(d)?, db_to_syntax(b)?),
        Db::Sort(u) => s_mk1("SSort", univ_to_syntax_db(u)),
        Db::Lit(Literal::Int(n)) => s_lit(*n),
        Db::Lit(Literal::Text(s)) => s_name(s),
        _ => return None,
    })
}

/// The reflected form of a universe. Deeply embedded and TOTAL, so a polymorphic level reflects
/// rather than stopping the walk.
fn univ_to_syntax_db(u: &Universe) -> Db {
    match u {
        Universe::SProp => Db::Global("USProp".to_string()),
        Universe::Prop => Db::Global("UProp".to_string()),
        Universe::Type(n) => s_mk1("UType", Db::Lit(Literal::Int(*n as i64))),
        Universe::Var(v) => s_mk1("UVar", Db::Global(v.clone())),
        Universe::Succ(l) => s_mk1("USucc", univ_to_syntax_db(l)),
        Universe::Max(a, b) => s_mk2("UMax", univ_to_syntax_db(a), univ_to_syntax_db(b)),
        Universe::IMax(a, b) => s_mk2("UIMax", univ_to_syntax_db(a), univ_to_syntax_db(b)),
    }
}

/// Full normalisation by descent: weak-head-normalise, then normalise the children.
///
/// `None` only when a nesting guard tripped, which the exhaustion flag has already recorded.
fn db_normalize(genv: &Context, t: &Db) -> Option<Db> {
    let _nest = nest(stack_mark())?;
    Some(match whnf(genv, t) {
        Db::App(f, a) => Db::App(
            DbBox::new(db_normalize(genv, &f)?),
            DbBox::new(db_normalize(genv, &a)?),
        ),
        Db::Lam(d, b) => Db::Lam(
            DbBox::new(db_normalize(genv, &d)?),
            DbBox::new(db_normalize(genv, &b)?),
        ),
        Db::Pi(d, b) => Db::Pi(
            DbBox::new(db_normalize(genv, &d)?),
            DbBox::new(db_normalize(genv, &b)?),
        ),
        other => other,
    })
}

/// [`db_normalize`], reporting whether the reduction FINISHED.
///
/// `None` means a meter tripped somewhere inside, so the answer is a partial reduct and not the
/// value of the expression. The caller must then report `Unchecked` rather than hand a partial
/// reduct to a rule whose answer a proof may cite — the same argument the step budget makes
/// about a rejection. The flag is saved and restored so this question does not erase an
/// exhaustion the surrounding reduction had already recorded.
fn db_normalize_checked(genv: &Context, t: &Db) -> Option<Db> {
    let outer = WHNF_EXHAUSTED.with(|e| e.replace(false));
    let out = db_normalize(genv, t);
    let tripped = WHNF_EXHAUSTED.with(|e| e.get());
    WHNF_EXHAUSTED.with(|e| e.set(outer || tripped));
    if tripped {
        None
    } else {
        out
    }
}

/// What one primitive rule did with its operands.
#[derive(Debug)]
enum PrimStep {
    /// The rule fired and this is its value.
    Reduced(Db),
    /// The rule's operands are outside its grammar, so there is nothing to compute — a VALUE,
    /// exactly as the first kernel leaves it.
    Stuck,
    /// This kernel did not compute the rule, and has set the exhaustion flag to say so.
    Unchecked,
}

/// Whether a name COULD be a primitive rule — a few byte comparisons, short-circuiting on the
/// first character. Asked at every application node of every reduction step, where the answer is
/// almost always "no", so it must cost nothing before the table is consulted.
fn looks_like_a_primitive_rule(name: &str) -> bool {
    name.starts_with("syn_")
        || name.starts_with("try_")
        || name.starts_with("tact_")
        || name.starts_with("induction_")
        || name == "concludes"
}

/// Read the certificate grammar directly from this checker's representation.
fn certificate_syntax(t: &Db) -> Option<crate::tactic_cert::Syn> {
    use crate::tactic_cert::Syn;
    let (head, args) = spine_ref(t);
    let Db::Global(name) = head else { return None };
    match (name.as_str(), args.as_slice()) {
        ("SLit", [Db::Lit(Literal::Int(n))]) => Some(Syn::Lit(*n)),
        ("SVar", [Db::Lit(Literal::Int(n))]) => Some(Syn::Var(*n)),
        ("SName", [Db::Lit(Literal::Text(n))]) => Some(Syn::Name(n.clone())),
        ("SApp", [f, x]) => Some(Syn::App(
            Box::new(certificate_syntax(f)?),
            Box::new(certificate_syntax(x)?),
        )),
        _ => None,
    }
}

/// A successful certificate corroborates a result; an absent certificate proves nothing.
fn apply_certificate_rule(name: &str, operand: &Db) -> PrimStep {
    use crate::tactic_cert::CertifiablePrimitive;
    let (primitive, goal) = if name == "concludes" {
        let (head, args) = spine_ref(operand);
        let Db::Global(constructor) = head else {
            return PrimStep::Unchecked;
        };
        let [goal] = args.as_slice() else {
            return PrimStep::Unchecked;
        };
        let Some(primitive) = CertifiablePrimitive::ALL
            .into_iter()
            .find(|p| p.success_constructor() == constructor)
        else {
            return PrimStep::Unchecked;
        };
        (primitive, *goal)
    } else {
        let Some(primitive) = CertifiablePrimitive::of_rule(name) else {
            return PrimStep::Unchecked;
        };
        (primitive, operand)
    };
    let Some(syntax) = certificate_syntax(goal) else {
        return PrimStep::Unchecked;
    };
    if !primitive.verify(&syntax) {
        return PrimStep::Unchecked;
    }
    if name == "concludes" {
        PrimStep::Reduced(goal.clone())
    } else {
        PrimStep::Reduced(Db::App(
            DbBox::new(Db::Global(primitive.success_constructor().to_string())),
            DbBox::new(goal.clone()),
        ))
    }
}

/// Apply one rule of the reflection algebra to its ALREADY-NORMALISED operands.
fn apply_reflection_rule(genv: &Context, name: &str, args: &[Db]) -> PrimStep {
    let value = |o: Option<Db>| match o {
        Some(d) => PrimStep::Reduced(d),
        None => PrimStep::Stuck,
    };
    match name {
        "try_ring" | "try_lia" | "try_cc" | "concludes" => apply_certificate_rule(name, &args[0]),
        "syn_step" => syn_step_db(genv, &args[0]),
        "syn_eval" => match s_int(&args[0]) {
            Some(fuel) => syn_eval_db(genv, fuel, &args[1]),
            None => PrimStep::Stuck,
        },
        // Every `Checked` row above is handled. A `Checked` row with no arm would silently
        // become "stuck", i.e. exactly the cry-wolf this table exists to end, so the default
        // is the honest one instead.
        _ => PrimStep::Unchecked,
    }
}

/// Dispatch one of the first kernel's registered reduction rules over this kernel's own
/// representation. `None` when `t` is not a saturated application of a table name — the answer
/// at almost every node, reached by one pointer walk and a few byte comparisons.
///
/// A DEFINITION of the name outranks the rule, in both kernels: a primitive fires on a NAME and
/// must never beat a body the term was type-checked against.
fn try_primitive_rule(genv: &Context, t: &Db) -> Option<PrimStep> {
    let Db::Global(head) = spine_head(t) else {
        return None;
    };
    if !looks_like_a_primitive_rule(head) {
        return None;
    }
    let (arity, kind) = primitive_rule(head)?;
    if shadowed_by_a_definition(genv, head)
        || !crate::reduction::reflection_declaration_agrees(genv, head)
    {
        return None;
    }
    let (_, args) = spine_ref(t);
    if args.len() < arity {
        return None;
    }
    if kind == RuleKind::Unchecked {
        return Some(PrimStep::Unchecked);
    }
    // The first kernel normalises each operand before walking the grammar, so this one does too
    // — and it reads the meter while doing it, because a rule fed a PARTIAL reduct would answer
    // a different question from the rule fed the value.
    let mut operands = Vec::with_capacity(arity);
    for a in &args[..arity] {
        match db_normalize_checked(genv, a) {
            Some(n) => operands.push(n),
            None => return Some(PrimStep::Unchecked),
        }
    }
    // Over-application: the rule consumes its own arguments and the surplus stays applied to the
    // result, which is where the first kernel's innermost-first descent leaves them too.
    let surplus: Vec<Db> = args[arity..].iter().map(|a| (*a).clone()).collect();
    Some(match apply_reflection_rule(genv, head, &operands) {
        PrimStep::Reduced(r) => PrimStep::Reduced(
            surplus
                .into_iter()
                .fold(r, |acc, x| Db::App(DbBox::new(acc), DbBox::new(x))),
        ),
        other => other,
    })
}

/// The re-checker's reduction step budget, per outermost `whnf`.
///
/// It is deliberately the SAME constant `reduction::normalize_checked` gives the main kernel. A
/// second kernel with a DIFFERENT budget is worse than no second kernel on exactly the terms
/// that matter: one runs out of room where the other does not, the conversion check fails on one
/// side only, and `double_check` reports a soundness alarm about a valid proof. The budget is
/// part of the shared specification, not an implementation detail.
const MAX_RECHECK_STEPS: usize = 1 << 20;

/// How deep the re-checker will nest NATIVE recursion inside `whnf`.
///
/// Limit on logical head-reduction demands, including suspended application,
/// Match and fix-argument frames. Moving these demands off the native stack
/// does not remove this independent resource boundary.
const MAX_RECHECK_WHNF_DEPTH: usize = 512;

/// The largest term the re-checker will carry through a reduction, in nodes.
///
/// A step budget alone does NOT bound memory. Each fix-unfolding substitutes the fixpoint's whole
/// body at every recursive occurrence, so a reduction that does not converge grows the term
/// GEOMETRICALLY — measured at 11 GB of resident term inside a hundred seconds, with the step
/// budget nowhere near spent. A checker that dies of OOM is exactly as useless as one that dies of
/// `SIGABRT`, so size is metered too.
const MAX_RECHECK_TERM_NODES: usize = 1 << 19;

/// Whether `t` has more than `limit` nodes — measured with an EXPLICIT stack and an early exit,
/// so asking the question about a pathological term neither overflows nor costs more than the
/// limit itself.
fn db_forest_size_exceeds(mut work: Vec<&Db>, limit: usize) -> bool {
    let mut seen = 0usize;
    while let Some(node) = work.pop() {
        seen += 1;
        if seen > limit {
            return true;
        }
        match node {
            Db::Sort(_) | Db::Var(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => {}
            Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
                work.push(a);
                work.push(b);
            }
            Db::Fix(b) => work.push(b),
            Db::MutualFix { defs, .. } => work.extend(defs.iter()),
            Db::Let(a, b, c) => {
                work.push(a);
                work.push(b);
                work.push(c);
            }
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                work.push(disc);
                work.push(motive);
                work.extend(cases.iter());
            }
        }
    }
    false
}

/// How much of the thread's stack the re-checker will spend descending before it stops.
///
/// A DEPTH cap is a proxy for the thing that actually runs out, and a bad one: a frame of this
/// module's `whnf` costs kilobytes in a debug build and a few hundred bytes optimised, so any
/// single number is either far below what a real proof needs or far above what a 2 MiB thread can
/// hold. So the meter measures the stack ITSELF, by the distance from the outermost re-checker
/// frame — which adapts to the build, to the thread, and to how much the caller had already used.
/// HALF the stack this thread actually has, so the other half is left to unwind and report
/// through. Derived rather than fixed: the two kernels now SIZE their stack from the term's own
/// depth (`term_store::deepen_stack`), and a hard-coded budget would give up with tens of
/// megabytes unused — the second kernel answering `Unsupported` about a term the first kernel
/// checks happily, which is single-checking wearing a two-kernel badge. `kernel_stack_bytes()`
/// reports the stack the kernel arranged, or the 2 MiB default when nobody sized it, so an
/// un-wired call behaves byte-identically to the constant this replaced.
fn recheck_stack_budget() -> usize {
    crate::term_store::kernel_stack_bytes() / 2
}

/// The address of a local in the CALLER's frame — a monotone proxy for how deep the native stack
/// has grown. Always inlined, so the local belongs to the guarded function rather than to this one.
#[inline(always)]
fn stack_mark() -> usize {
    let probe = 0u8;
    std::ptr::addr_of!(probe) as usize
}

/// Restores the nesting count on the way out, including on a panic.
struct NestGuard;

impl Drop for NestGuard {
    fn drop(&mut self) {
        RECHECK_NEST.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// A ticket for one native frame of the re-checker's recursion, or `None` when the stack budget
/// is spent — in which case the caller must return its SAFE answer (an unreduced term, "not
/// equal", or `Unsupported`) and the exhaustion flag makes the whole verdict honestly incomplete.
///
/// Called as `nest(stack_mark())` so the measured address is the CALLER's.
fn nest(here: usize) -> Option<NestGuard> {
    let depth = RECHECK_NEST.with(|n| n.get());
    if depth == 0 {
        RECHECK_STACK_BASE.with(|b| b.set(here));
    } else if RECHECK_STACK_BASE.with(|b| b.get()).abs_diff(here) > recheck_stack_budget() {
        WHNF_EXHAUSTED.with(|e| e.set(true));
        return None;
    }
    RECHECK_NEST.with(|n| n.set(depth + 1));
    Some(NestGuard)
}

thread_local! {
    /// Native nesting count across every recursive function in this module.
    static RECHECK_NEST: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Stack address of the outermost re-checker frame.
    static RECHECK_STACK_BASE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Logical nesting depth of `whnf`, including suspended iterative demands.
    static WHNF_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Steps remaining for the current outermost `whnf`.
    static WHNF_FUEL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Whether the budget or the depth door stopped a reduction since the flag was cleared.
    /// A reduction that was CUT SHORT did not compute a normal form, and a conversion check
    /// that compares two abandoned reductions has certified nothing — so the re-checker reports
    /// `Unsupported` (honestly single-checked) rather than a verdict it did not earn.
    static WHNF_EXHAUSTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    /// Whether `infer` is typing a term conversion or reduction PRODUCED — see [`infer_reduct`].
    static REDUCT_INFERENCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Types reducts until dropped, including on a panic; scopes nest.
struct ReductInference(bool);

impl ReductInference {
    fn enter() -> Self {
        ReductInference(REDUCT_INFERENCE.with(|m| m.replace(true)))
    }
}

impl Drop for ReductInference {
    fn drop(&mut self) {
        REDUCT_INFERENCE.with(|m| m.set(self.0));
    }
}

/// The type of a term that conversion or reduction produced — a REDUCT. Its depth is set by how
/// far a computation ran, not by how deep the input was written, so the door at `recheck`'s entry
/// never measured it and the stack sized from the input need not hold it: a reduction the budget
/// cut short leaves exactly such a term, a tower of pending eliminations as deep as the reduction
/// got. Its inference is therefore metered as the reduction that made it is. Past the meter it
/// stops, the exhaustion flag records the stop, and the caller's "no" becomes an honest resource
/// limit rather than a native overflow.
fn infer_reduct(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> RResult<Db> {
    let _reduct = ReductInference::enter();
    infer(genv, lctx, t)
}

/// Clear the exhaustion flag and report whether anything tripped it since the last clear.
fn take_budget_exhausted() -> bool {
    WHNF_EXHAUSTED.with(|e| e.replace(false))
}

thread_local! {
    /// This check met a reduction rule of the first kernel that this one does not compute (an
    /// unchecked primitive, a `@[simp]` rewrite). Such a stop is not a resource limit: no amount
    /// of fuel decides it.
    static UNCHECKED_RULE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Start a check's record: no stop and no unchecked rule yet.
fn begin_check() {
    take_budget_exhausted();
    UNCHECKED_RULE.with(|u| u.set(false));
}

/// Stop here: this kernel does not compute the rule it has reached. Reported through the same
/// exhaustion channel as a spent budget, and recorded so the stop is classified as unsupported.
fn mark_unchecked_rule() {
    WHNF_EXHAUSTED.with(|e| e.set(true));
    UNCHECKED_RULE.with(|u| u.set(true));
}

/// A stop outside any rejection: unsupported if the check met a rule this kernel does not
/// compute, otherwise a resource limit.
fn stopped(what: impl Into<String>) -> Incomplete {
    let what = what.into();
    match UNCHECKED_RULE.with(|u| u.get()) {
        true => Incomplete::unsupported(format!("{what}: the re-checker reached a rule it does not compute")),
        false => match ReCheckError::resource_limit(format!(
            "{what}: the re-checker's budget ({MAX_RECHECK_STEPS} steps, depth {MAX_RECHECK_WHNF_DEPTH}, \
             {MAX_RECHECK_TERM_NODES} nodes) ran out"
        )) {
            ReCheckError::Unsupported(incomplete) => incomplete,
            ReCheckError::Ill(_) => unreachable!("a resource limit is an incompleteness"),
        },
    }
}

/// The honest account of a rejection reached after a stop: no verdict. An unchecked rule met
/// anywhere in the check makes it unsupported, since more resources could never decide it;
/// otherwise a resource bound stopped it.
fn cut_short(rejection: &ReCheckError, what: &str) -> ReCheckError {
    if UNCHECKED_RULE.with(|u| u.get()) {
        ReCheckError::unsupported(format!(
            "the re-checker's {what} reached a rule it does not compute, so its rejection \
             ({rejection:?}) is not an independent verdict"
        ))
    } else {
        ReCheckError::resource_limit(format!(
            "the re-checker's {what} was cut short by its budget ({MAX_RECHECK_STEPS} steps, \
             depth {MAX_RECHECK_WHNF_DEPTH}, {MAX_RECHECK_TERM_NODES} nodes), so its rejection \
             ({rejection:?}) is not an independent verdict"
        ))
    }
}

/// Restores the nesting depth on the way out, including on a panic.
struct WhnfDepthGuard(usize);

impl Drop for WhnfDepthGuard {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|d| d.set(self.0));
    }
}

fn whnf(genv: &Context, t: &Db) -> Db {
    whnf_in(genv, &[], t)
}

#[cfg(test)]
thread_local! {
    static WHNF_INPUT_CLONES: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

fn clone_whnf_input(t: &Db) -> Db {
    #[cfg(test)]
    WHNF_INPUT_CLONES.with(|counter| {
        if let Some(count) = counter.get() {
            counter.set(Some(count + db_node_count(t)));
        }
    });
    t.clone()
}

fn whnf_in_owned(genv: &Context, lctx: &dyn LocalTypes, t: Db) -> Db {
    whnf_from(genv, lctx, || t)
}

thread_local! {
    /// The `whnf_from` depth at which δ of the outermost spine's head is withheld; 0 for none.
    static CORE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Weak-head normal form WITHOUT δ at the head — Lean's `whnf_core` — computed by this kernel's own
/// weak-head machine, so every rule it applies before δ (the stuck-head rules, the certified `Nat`
/// order and arithmetic, the equality recurrence), its fuel, its depth door and its size check all
/// hold exactly as in [`whnf_in`]. Only the head constant of the result is left unexpanded.
fn whnf_core_in(genv: &Context, lctx: &dyn LocalTypes, t: Db) -> Db {
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            CORE_DEPTH.with(|c| c.set(self.0));
        }
    }
    let depth = WHNF_DEPTH.with(|d| d.get()) + 1;
    let _restore = Restore(CORE_DEPTH.with(|c| c.replace(depth)));
    whnf_in_owned(genv, lctx, t)
}

fn whnf_in(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Db {
    whnf_from(genv, lctx, || clone_whnf_input(t))
}

/// Delay ownership acquisition until the original depth/stack guard locations.
/// Borrowed inputs retain their existing clone/return contract; fresh owned
/// inputs move into precisely the same independent reduction loop.
fn whnf_from(genv: &Context, lctx: &dyn LocalTypes, input: impl FnOnce() -> Db) -> Db {
    let depth = WHNF_DEPTH.with(|d| {
        let next = d.get() + 1;
        d.set(next);
        next
    });
    let _depth_guard = WhnfDepthGuard(depth - 1);
    // The outermost call establishes the step budget; every nested one SHARES it, so the total
    // work of one reduction is bounded rather than the work of each frame.
    if depth == 1 && LOCAL_CONTEXT_BUDGET_DEPTH.with(|d| d.get() == 0) {
        WHNF_FUEL.with(|f| f.set(MAX_RECHECK_STEPS));
    }
    if depth > MAX_RECHECK_WHNF_DEPTH {
        WHNF_EXHAUSTED.with(|e| e.set(true));
        return input();
    }
    // Remaining recursive primitive and quotient probes still need the native-stack
    // guard; ordinary application, Match and structural-argument demands use frames.
    let Some(_nest) = nest(stack_mark()) else {
        return input();
    };
    let mut cur = input();
    let mut pending: Vec<WhnfFrame> = Vec::new();
    let mut steps = 0usize;
    let mut next_size_check = 1024usize;
    loop {
        // Explicit demands occupy the same logical depth as recursive whnf calls.
        // Recursive primitive/quotient probes also see this depth and shared fuel.
        WHNF_DEPTH.with(|d| d.set(depth + pending.len()));
        let spent = WHNF_FUEL.with(|f| {
            let left = f.get();
            if left == 0 {
                return false;
            }
            f.set(left - 1);
            true
        });
        if !spent {
            WHNF_EXHAUSTED.with(|e| e.set(true));
            return rebuild_whnf(cur, pending);
        }
        nat_order_observed_work();
        prefix_stat(4);
        steps += 1;
        if steps == next_size_check {
            next_size_check *= 2;
            if whnf_size_exceeds(&cur, &pending, MAX_RECHECK_TERM_NODES) {
                WHNF_EXHAUSTED.with(|e| e.set(true));
                return rebuild_whnf(cur, pending);
            }
        }
        // Head reduction without δ AT THE HEAD (`whnf_core_in`): the outermost spine's head constant
        // is left folded; a major premise, a structural argument and every nested reduction still
        // unfold as usual.
        let head_delta_withheld = CORE_DEPTH.with(|c| c.get()) == depth
            && matches!(cur, Db::Global(_) | Db::Const { .. })
            && pending
                .iter()
                .all(|frame| matches!(frame, WhnfFrame::Apply(_)));
        let mut step = if head_delta_withheld {
            WhnfStep::Ready(cur)
        } else {
            whnf_step(genv, lctx, cur)
        };
        loop {
            match step {
                WhnfStep::Reduce(next) => {
                    cur = next;
                    break;
                }
                WhnfStep::Ready(value) => match pending.pop() {
                    None => return value,
                    Some(frame) => {
                        WHNF_DEPTH.with(|d| d.set(depth + pending.len()));
                        step = resume_whnf(genv, lctx, frame, value);
                    }
                },
                WhnfStep::Demand { term, frame } => {
                    if depth + pending.len() + 1 > MAX_RECHECK_WHNF_DEPTH {
                        WHNF_EXHAUSTED.with(|e| e.set(true));
                        return rebuild_whnf(frame.rebuild(term), pending);
                    }
                    pending.push(frame);
                    cur = term;
                    break;
                }
            }
        }
    }
}

/// Suspended independent reductions. Every frame owns only the context around
/// its hole, so resource refusal can reconstruct the complete outer expression.
enum WhnfFrame {
    // An undemanded argument retains its original owned/shared edge.
    Apply(DbBox),
    Match {
        motive: DbBox,
        cases: Vec<Db>,
    },
    FixArgument {
        head: Db,
        before: Vec<DbBox>,
        after: Vec<DbBox>,
    },
}
enum WhnfStep {
    Reduce(Db),
    Ready(Db),
    Demand { term: Db, frame: WhnfFrame },
}
fn apply_db_edges(head: Db, args: impl IntoIterator<Item = DbBox>) -> Db {
    args.into_iter()
        .fold(head, |f, a| Db::App(DbBox::new(f), a))
}
fn apply_fix_arguments(head: Db, before: Vec<DbBox>, value: Db, after: Vec<DbBox>) -> Db {
    let prefix = apply_db_edges(head, before);
    // Only the demanded result is a new argument edge. Retain the original
    // admission order at its application node and all undemanded side edges.
    let applied = Db::App(DbBox::new(prefix), DbBox::new(value));
    apply_db_edges(applied, after)
}
impl WhnfFrame {
    fn rebuild(self, term: Db) -> Db {
        match self {
            Self::Apply(a) => Db::App(DbBox::new(term), a),
            Self::Match { motive, cases } => Db::Match {
                disc: DbBox::new(term),
                motive,
                cases,
            },
            Self::FixArgument {
                head,
                before,
                after,
            } => apply_fix_arguments(head, before, term, after),
        }
    }
    fn size_roots<'a>(&'a self, roots: &mut Vec<&'a Db>) -> usize {
        match self {
            Self::Apply(a) => {
                roots.push(a);
                1
            }
            Self::Match { motive, cases } => {
                roots.push(motive);
                roots.extend(cases);
                1
            }
            Self::FixArgument {
                head,
                before,
                after,
            } => {
                roots.push(head);
                roots.extend(before.iter().map(|edge| edge.as_ref()));
                roots.extend(after.iter().map(|edge| edge.as_ref()));
                before.len() + 1 + after.len()
            }
        }
    }
}
fn rebuild_whnf(mut term: Db, mut pending: Vec<WhnfFrame>) -> Db {
    while let Some(frame) = pending.pop() {
        term = frame.rebuild(term);
    }
    term
}
fn whnf_size_exceeds(cur: &Db, pending: &[WhnfFrame], limit: usize) -> bool {
    let mut roots = vec![cur];
    let mut context_nodes = 0usize;
    for frame in pending {
        context_nodes += frame.size_roots(&mut roots);
    }
    context_nodes > limit || db_forest_size_exceeds(roots, limit - context_nodes)
}
fn demand_fix_argument(genv: &Context, full: Db) -> WhnfStep {
    if let Some(column) = fix_structural_column(genv, &full) {
        let mut head = full;
        let mut args = Vec::new();
        while let Db::App(f, a) = head {
            args.push(a);
            head = f.into_db();
        }
        args.reverse();
        let after = args.split_off(column + 1);
        let term = args
            .pop()
            .expect("checked structural column exists")
            .into_db();
        WhnfStep::Demand {
            term,
            frame: WhnfFrame::FixArgument {
                head,
                before: args,
                after,
            },
        }
    } else if let Some(reduced) = try_builtin(genv, &full) {
        WhnfStep::Reduce(reduced)
    } else {
        WhnfStep::Ready(full)
    }
}
fn resume_whnf(genv: &Context, lctx: &dyn LocalTypes, frame: WhnfFrame, value: Db) -> WhnfStep {
    match frame {
        WhnfFrame::Apply(a) => match value {
            Db::Lam(_, body) => WhnfStep::Reduce(beta_open_consuming(body.into_db(), a.into_db())),
            // Keep the existing mutual-fix guard and substitution semantics.
            Db::MutualFix { defs, index } => {
                if ctor_headed(genv, &a) {
                    WhnfStep::Reduce(Db::App(DbBox::new(open_mutual(&defs, index)), a))
                } else {
                    WhnfStep::Ready(Db::App(DbBox::new(Db::MutualFix { defs, index }), a))
                }
            }
            // The head is now in weak-head normal form, and this application may be a redex no
            // spine on the way down contained: offer it the rules of a stuck head first.
            other => {
                let full = Db::App(DbBox::new(other), a);
                match stuck_head_rule(genv, lctx, &full) {
                    Some(StuckHead::Reduce(reduced)) => WhnfStep::Reduce(reduced),
                    Some(StuckHead::Unchecked) => {
                        mark_unchecked_rule();
                        WhnfStep::Ready(full)
                    }
                    None => demand_fix_argument(genv, full),
                }
            }
        },
        WhnfFrame::FixArgument {
            head,
            before,
            after,
        } => {
            let ready = constructor_value(genv, &value);
            if ready {
                let Db::Fix(body) = head else {
                    unreachable!("checked Fix head")
                };
                let recursive = Db::Fix(body.clone());
                let unfolded = beta_open_consuming(body.into_db(), recursive);
                WhnfStep::Reduce(apply_fix_arguments(unfolded, before, value, after))
            } else {
                WhnfStep::Ready(apply_fix_arguments(head, before, value, after))
            }
        }
        WhnfFrame::Match { motive, cases } => resume_match(genv, lctx, value, motive, cases),
    }
}
fn db_motive_admits(genv: &Context, motive: &Db, discriminant: &Db) -> bool {
    let mut ctor = discriminant;
    while let Db::App(f, _) = ctor {
        ctor = f;
    }
    let Some(actual) = (match ctor {
        Db::Global(c) | Db::Const { name: c, .. } => genv.constructor_inductive(c),
        _ => None,
    }) else {
        return true;
    };
    let mut domain = None;
    let mut binders = 0usize;
    let mut current = motive;
    while let Db::Lam(param_type, body) = current {
        domain = Some(param_type.as_ref());
        binders += 1;
        current = body;
    }
    // An eta-short motive's last lambda binds an index, not the major premise.
    if binders <= genv.inductive_num_indices(actual) {
        return true;
    }
    let Some(domain) = domain else { return true };
    let mut head = domain;
    while let Db::App(f, _) = head {
        head = f;
    }
    let named = match head {
        Db::Global(n) | Db::Const { name: n, .. } if genv.is_inductive(n) => n,
        _ => return true,
    };
    actual == named.as_str()
}

fn resume_match(
    genv: &Context,
    lctx: &dyn LocalTypes,
    mut d: Db,
    motive: DbBox,
    mut cases: Vec<Db>,
) -> WhnfStep {
    // Peano bridge (K6): a `Nat(n)` literal is `Zero`/`Succ(Nat(n-1))` — expand
    // one step so the constructor-selection below fires (the recursor computes
    // on Nat literals, peeling one `Succ` per match), where the literal's reading holds
    // (`int_inductive::reads_nat_literal`).
    if matches!(&d, Db::Lit(Literal::Nat(n)) if crate::int_inductive::reads_nat_literal(genv, n)) {
        d = db_nat_peano_step(&d);
    }
    if let Some(constructor) = db_int_constructor(genv, &d) {
        d = constructor;
    }
    // Inspect only the spine and registration headers before consuming it.
    // Preserve the constructor's field application boxes; replace just the
    // constructor and uniform-parameter prefix with the selected branch.
    let plan = {
        let mut head = &d;
        let mut arguments = 0usize;
        while let Db::App(function, _) = head {
            arguments += 1;
            head = function;
        }
        match head {
            Db::Global(name) | Db::Const { name, .. } => {
                genv.constructor_inductive(name).and_then(|inductive| {
                    let index = genv
                        .constructor_headers(inductive)
                        .position(|(constructor, _)| constructor == name)?;
                    if index >= cases.len() {
                        return None;
                    }
                    let Some(fields) = arguments.checked_sub(genv.inductive_num_params(inductive))
                    else {
                        return Some((index, None));
                    };
                    Some((index, Some(fields)))
                })
            }
            _ => None,
        }
    };
    // A recursor fires only on its own inductive's constructors: a motive whose last binder
    // names a different registered inductive makes this match ill-typed, and it must not compute
    // (the main kernel's `motive_admits`).
    let plan = plan.filter(|_| db_motive_admits(genv, &motive, &d));
    if let Some((index, fields)) = plan {
        let Some(fields) = fields else {
            return WhnfStep::Ready(Db::Match {
                disc: DbBox::new(d),
                motive,
                cases,
            });
        };
        let branch = cases.swap_remove(index);
        let mut prefix = &mut d;
        for _ in 0..fields {
            let Db::App(function, _) = prefix else {
                unreachable!("counted constructor spine")
            };
            prefix = function;
        }
        *prefix = branch;
        return WhnfStep::Reduce(d);
    }
    // K can select a branch only for a fieldless singleton.
    // Its checked full or reachable pruned inventory has one case.
    // Avoid re-inferring a neutral discriminant when this necessary
    // condition already rules out singleton reduction.
    if cases.len() == 1 {
        if let Some(constructor) = k_constructor_db(genv, lctx, &d) {
            return WhnfStep::Reduce(Db::Match {
                disc: DbBox::new(constructor),
                motive,
                cases,
            });
        }
    }
    return WhnfStep::Ready(Db::Match {
        disc: DbBox::new(d),
        motive,
        cases,
    });
}
#[path = "recheck_arithmetic.rs"]
mod arithmetic;
#[path = "recheck_conversion_memo.rs"]
mod conversion_memo;
#[path = "recheck_division.rs"]
mod division;
#[path = "recheck_lazy_delta.rs"]
mod lazy_delta;
#[path = "recheck_observational.rs"]
mod observational;
#[cfg(test)]
thread_local! {
    /// How many conversions reached the reduction-based comparison (`def_eq_by_whnf`).
    static WHNF_CONVERSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What an application headed by a constant with no δ of its own reduces by.
enum StuckHead {
    Reduce(Db),
    /// A primitive rule this kernel does not compute: the application is a value here, and the
    /// exhaustion flag says so.
    Unchecked,
}

/// The rules of an application headed by a constant that has no δ of its own: the quotient
/// computation rules (mirroring the main kernel, so quotient proofs are two-kernel verified), the
/// first kernel's registered primitive rules, and the native-reduction hook.
///
/// Every application whose head is in weak-head normal form is offered them — the spine
/// [`whnf_step`] is handed, AND the spine [`resume_whnf`] rebuilds once δ and β have reduced the
/// head. The second is not a corner: Lean's `Quotient.lift` unfolds to the PARTIAL primitive
/// `Quot.lift f`, its respect proof and its major premise arrive from the enclosing application,
/// and the quotient redex exists only in the rebuilt spine. A rule offered only on descent leaves
/// that redex a value here while the main kernel computes it — a disagreement about a valid proof
/// (Lean core's `Quotient.lift₂._proof_2`).
fn stuck_head_rule(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<StuckHead> {
    let Db::App(function, argument) = t else {
        return None;
    };
    if let Some(reduced) = try_quot_lift_db(genv, lctx, t) {
        return Some(StuckHead::Reduce(reduced));
    }
    if let Some(reduced) = observational::reduce(genv, lctx, t) {
        return Some(StuckHead::Reduce(reduced));
    }
    // The first kernel's registered reduction rules — the reflection algebra computed here
    // independently, the tactic family declared unchecked. A rule only ONE kernel has is not a
    // safe shortfall: the redex would stay a value here, the conversion check would fail on one
    // side only, and `double_check` would report `Disagree` about a proof that is perfectly valid.
    match try_primitive_rule(genv, t) {
        Some(PrimStep::Reduced(reduced)) => return Some(StuckHead::Reduce(reduced)),
        // Honest incompleteness, through the SAME channel a spent step budget uses: a later
        // rejection is reported as `Unsupported` — single-checked and labelled — instead of as a
        // disagreement this kernel did not earn.
        Some(PrimStep::Unchecked) => return Some(StuckHead::Unchecked),
        Some(PrimStep::Stuck) | None => {}
    }
    // Native-reduction hook: the main kernel reduces `reduceBool t` with its trusted evaluator;
    // the re-checker reduces `t` with its OWN machinery (δ/ι/builtins) — an independent
    // computation of the same value, so a `native_decide` proof is genuinely two-kernel verified.
    if matches!(function.as_ref(), Db::Global(n) if n == "reduceBool") {
        let value = whnf_in(genv, lctx, argument);
        if matches!(&value, Db::Global(n) if n == "true" || n == "false") {
            return Some(StuckHead::Reduce(value));
        }
    }
    None
}

/// The accelerations of a DEFINED operation applied to values: each computes, with this kernel's
/// own arithmetic, what δ followed by ι would compute one step at a time. Every one must run BEFORE
/// δ, because δ erases the operation's name from the spine.
fn accelerated_application(genv: &Context, lctx: &dyn LocalTypes, cur: &Db) -> Option<Db> {
    // Checked public Int aliases must use their independently implemented
    // literal arithmetic before delta erases the attested alias head.
    if let Db::App(function, _) = cur {
        if let Db::App(head, _) = function.as_ref() {
            if let Db::Global(operation) = head.as_ref() {
                if crate::int_inductive::accelerates_integer_alias(genv, operation) {
                    if let Some(reduced) = try_builtin(genv, cur) {
                        return Some(reduced);
                    }
                }
            }
        }
    }
    if let Some(reduced) = arithmetic::reduce(genv, lctx, cur) {
        return Some(reduced);
    }
    // The `Nat` order on two closed naturals, by arithmetic rather than by `min(m, n)`
    // ι-steps. Runs BEFORE δ, because `natLt`'s body unfolds to `natLe (Succ m) n` and
    // `natLe`'s to a `Fix` — after either, the spine no longer names the operation.
    if let Some(reduced) = try_nat_order_db_in(genv, lctx, cur) {
        return Some(reduced);
    }
    if let Some(reduced) = try_certified_nat_order_db_in(genv, lctx, cur) {
        return Some(reduced);
    }
    if let Some(reduced) = division::reduce(genv, lctx, cur) {
        return Some(reduced);
    }
    equality::reduce(genv, lctx, cur)
}

fn whnf_step(genv: &Context, lctx: &dyn LocalTypes, cur: Db) -> WhnfStep {
    if matches!(cur, Db::App(..)) {
        match stuck_head_rule(genv, lctx, &cur) {
            Some(StuckHead::Reduce(reduced)) => return WhnfStep::Reduce(reduced),
            Some(StuckHead::Unchecked) => {
                mark_unchecked_rule();
                return WhnfStep::Ready(cur);
            }
            None => {}
        }
        if let Some(reduced) = accelerated_application(genv, lctx, &cur) {
            return WhnfStep::Reduce(reduced);
        }
        // A `@[simp]` rewrite is a rule the FIRST kernel's conversion now takes and this one
        // does not. Making a registered PROPOSITIONAL equation DEFINITIONAL is a decision
        // about the trusted base, not an algorithm — a second kernel that re-took it would
        // be the same decision twice, which corroborates nothing while looking exactly like
        // corroboration. So this kernel says what is true: it did not carry that rewrite
        // through. The flag is set and reduction CONTINUES, so where δ/ι reach the same
        // normal form anyway the verdict is still `Agreed`; only where they do not does the
        // rejection become `Unsupported` — honestly single-checked — instead of a soundness
        // alarm about a proof the first kernel checked with a rule of its own.
        //
        // The `is_empty` test comes first so a context with no simp lemma — every context
        // until a user registers one — pays nothing at all for this at every application
        // node of every reduction step.
        if !genv.simp_rules().is_empty() {
            if let Db::Global(head) = spine_head(&cur) {
                if genv.simp_rules_for(head).next().is_some() {
                    mark_unchecked_rule();
                }
            }
        }
    }

    match cur {
        Db::Global(name) => {
            if theorem_folded(genv, &name) {
                return WhnfStep::Ready(Db::Global(name));
            }
            if let Some(db) = definition_body(genv, &name) {
                return WhnfStep::Reduce(db);
            }
            WhnfStep::Ready(Db::Global(name))
        }
        Db::Const { name, levels } => {
            if theorem_folded(genv, &name) {
                return WhnfStep::Ready(Db::Const { name, levels });
            }
            if let Some(body) = const_body(genv, &name, &levels) {
                return WhnfStep::Reduce(body);
            }
            WhnfStep::Ready(Db::Const { name, levels })
        }
        Db::App(f, a) => WhnfStep::Demand {
            term: f.into_db(),
            frame: WhnfFrame::Apply(a),
        },
        Db::Match {
            disc,
            motive,
            cases,
        } => WhnfStep::Demand {
            term: disc.into_db(),
            frame: WhnfFrame::Match { motive, cases },
        },
        Db::Let(_ty, value, body) => {
            WhnfStep::Reduce(beta_open_consuming(body.into_db(), value.into_db()))
        }
        other => WhnfStep::Ready(other),
    }
}

// ---------------------------------------------------------------------------
// Conversion (definitional equality + cumulativity)
// ---------------------------------------------------------------------------

/// Independently reconstruct a nullary proof constructor and compare every index.
fn k_constructor_db(genv: &Context, lctx: &dyn LocalTypes, proof: &Db) -> Option<Db> {
    if definitely_fieldful_k_family(genv, lctx, proof) {
        return None;
    }
    #[cfg(test)]
    K_PROBE_ATTEMPTS.with(|n| {
        if let Some(count) = n.get() {
            n.set(Some(count + 1));
        }
    });
    let proof_type = whnf_in_owned(genv, lctx, infer_reduct(genv, lctx, proof).ok()?);
    let (head, args) = spine_ref(&proof_type);
    let (name, levels) = match head {
        Db::Global(name) => (name, None),
        Db::Const { name, levels } => (name, Some(levels)),
        _ => return None,
    };
    if !genv.is_inductive(&name)
        || genv
            .mutual_block_of(&name)
            .is_some_and(|block| block.len() != 1)
    {
        return None;
    }
    let parameters = genv.inductive_num_params(&name);
    let mut declared_sort = genv.get_global(&name)?;
    let mut arity = 0;
    while let Term::Pi { body_type, .. } = declared_sort {
        arity += 1;
        declared_sort = body_type;
    }
    if !matches!(declared_sort, Term::Sort(_)) {
        return None;
    }
    let unit = arity == parameters;
    if !unit
        && !matches!(declared_sort, Term::Sort(u) if *u != Universe::SProp && u.equiv(&Universe::Prop))
    {
        return None;
    }
    let constructors = genv.get_constructors(&name);
    if constructors.len() != 1 {
        return None;
    }
    let mut constructor_type = constructors[0].1;
    let mut fields = 0;
    while let Term::Pi { body_type, .. } = constructor_type {
        fields += 1;
        constructor_type = body_type;
    }
    if fields != parameters || args.len() < parameters {
        return None;
    }
    let sort = whnf_in_owned(genv, lctx, infer_reduct(genv, lctx, &proof_type).ok()?);
    if !unit && !matches!(sort, Db::Sort(u) if u != Universe::SProp && u.equiv(&Universe::Prop)) {
        return None;
    }
    let mut constructor = match levels {
        Some(levels) => Db::Const {
            name: constructors[0].0.into(),
            levels: levels.clone(),
        },
        None => Db::Global(constructors[0].0.into()),
    };
    for arg in args.into_iter().take(parameters) {
        constructor = Db::App(DbBox::new(constructor), DbBox::new(arg.clone()));
    }
    let constructor_type = infer_reduct(genv, lctx, &constructor).ok()?;
    def_eq(genv, lctx, &proof_type, &constructor_type).then_some(constructor)
}

/// One Peano-unfolding step of a de Bruijn `Nat` literal: `Nat(0) → Zero`,
/// `Nat(n) → Succ (Nat(n-1))` (K6). Non-Nat terms are returned unchanged.
fn db_nat_peano_step(t: &Db) -> Db {
    match t {
        // `n ≤ 0` collapses to `Zero` so the peel TERMINATES on any (incl. malformed
        // negative) literal — critical here, where the re-checker consumes untrusted input.
        Db::Lit(Literal::Nat(n)) if *n <= logicaffeine_base::BigInt::from_i64(0) => {
            Db::Global("Zero".to_string())
        }
        Db::Lit(Literal::Nat(n)) => Db::App(
            DbBox::new(Db::Global("Succ".to_string())),
            DbBox::new(Db::Lit(Literal::Nat(
                n.sub(&logicaffeine_base::BigInt::from_i64(1)),
            ))),
        ),
        other => other.clone(),
    }
}

/// True if `t` heads a `Nat` Peano value — `Zero` or `Succ _` — the shape a `Nat` literal
/// bridges against.
fn db_nat_peano_headed(t: &Db) -> bool {
    match t {
        Db::Global(n) => n == "Zero",
        Db::App(f, _) => matches!(f.as_ref(), Db::Global(n) if n == "Succ"),
        _ => false,
    }
}

/// Positive-only conversion probe. It uses the independent de Bruijn syntax and
/// beta substitution, never the main checker. A failed/exhausted probe proves
/// nothing; the ordinary conversion path remains responsible for the verdict.
/// Keeping shared heads folded avoids copying and reducing a common function
/// just to compare two small convertible arguments.
#[path = "recheck_delta.rs"]
mod delta;
#[path = "recheck_folded.rs"]
mod folded;
#[path = "recheck_head_closure.rs"]
mod head_closure;
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/proof_reference_boundary_controls.rs"]
mod proof_reference_boundary_controls;
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/proof_reference_controls.rs"]
mod proof_reference_controls;

fn borrowed_congruence(a: &Db, b: &Db, fuel: &mut usize) -> bool {
    borrowed_congruence_in(None, a, b, fuel)
}

fn borrowed_congruence_in(ctx: Option<&Context>, a: &Db, b: &Db, fuel: &mut usize) -> bool {
    borrowed_congruence_with_limit(ctx, a, b, fuel, 64, *fuel)
}

// A wider sequence of small reductions does not widen a single recursive
// substitution. Charge every attempted traversal to its enclosing allowance.
fn bounded_beta_zeta_probe(t: &Db, fuel: &mut usize, candidate: usize) -> Option<Db> {
    let allowance = if candidate > 64 {
        (*fuel).min(256)
    } else {
        *fuel
    };
    let mut remaining = allowance;
    let result = bounded_beta_zeta(t, &mut remaining);
    *fuel -= allowance - remaining;
    result
}

fn borrowed_congruence_with_limit(
    ctx: Option<&Context>,
    a: &Db,
    b: &Db,
    fuel: &mut usize,
    candidate: usize,
    depth: usize,
) -> bool {
    if depth == 0 {
        return false;
    }
    let _nest = if candidate > 64 {
        let Some(guard) = nest(stack_mark()) else {
            return false;
        };
        Some(guard)
    } else {
        None
    };
    if *fuel == 0 {
        return false;
    }
    *fuel -= 1;
    let congruent = match (a, b) {
        (Db::Var(x), Db::Var(y)) => x == y,
        (Db::Global(x), Db::Global(y)) => x == y,
        (Db::Lit(x), Db::Lit(y)) => x == y,
        (Db::Sort(x), Db::Sort(y)) => x.equiv(y),
        (Db::Const { name: x, levels: u }, Db::Const { name: y, levels: v }) => {
            x == y && u.len() == v.len() && u.iter().zip(v).all(|(x, y)| x.equiv(y))
        }
        (Db::App(f, x), Db::App(g, y)) => {
            borrowed_congruence_with_limit(ctx, f, g, fuel, candidate, depth - 1)
                && borrowed_congruence_with_limit(ctx, x, y, fuel, candidate, depth - 1)
        }
        (Db::Pi(d, x), Db::Pi(e, y)) | (Db::Lam(d, x), Db::Lam(e, y)) => {
            borrowed_congruence_with_limit(ctx, d, e, fuel, candidate, depth - 1)
                && borrowed_congruence_with_limit(ctx, x, y, fuel, candidate, depth - 1)
        }
        _ => false,
    };
    if congruent {
        return true;
    }
    if let Some(reduced) = bounded_beta_zeta_probe(a, fuel, candidate) {
        return borrowed_congruence_with_limit(ctx, &reduced, b, fuel, candidate, depth - 1);
    }
    if let Some(reduced) = bounded_beta_zeta_probe(b, fuel, candidate) {
        return borrowed_congruence_with_limit(ctx, a, &reduced, fuel, candidate, depth - 1);
    }
    if let Some(ctx) = ctx {
        if let Some(reduced) = delta::reduce_with_limit(ctx, a, fuel, candidate) {
            return borrowed_congruence_with_limit(
                Some(ctx),
                &reduced,
                b,
                fuel,
                candidate,
                depth - 1,
            );
        }
        if let Some(reduced) = delta::reduce_with_limit(ctx, b, fuel, candidate) {
            return borrowed_congruence_with_limit(
                Some(ctx),
                a,
                &reduced,
                fuel,
                candidate,
                depth - 1,
            );
        }
    }
    false
}

/// Count with the probe's shared work allowance. No traversal allocates a
/// frontier larger than the remaining allowance.
fn congruence_nodes(t: &Db, fuel: &mut usize) -> Option<usize> {
    let mut work = vec![t];
    let mut count = 0usize;
    while let Some(t) = work.pop() {
        if *fuel == 0 {
            return None;
        }
        *fuel -= 1;
        count += 1;
        let children = match t {
            Db::Pi(..) | Db::Lam(..) | Db::App(..) => 2,
            Db::Let(..) => 3,
            Db::Fix(..) => 1,
            Db::MutualFix { defs, .. } => defs.len(),
            Db::Match { cases, .. } => cases.len().checked_add(2)?,
            _ => 0,
        };
        if children > fuel.saturating_sub(work.len()) {
            return None;
        }
        match t {
            Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
                work.push(a);
                work.push(b);
            }
            Db::Let(a, b, c) => {
                work.push(a);
                work.push(b);
                work.push(c);
            }
            Db::Fix(b) => work.push(b),
            Db::MutualFix { defs, .. } => work.extend(defs),
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                work.push(disc);
                work.push(motive);
                work.extend(cases);
            }
            _ => {}
        }
    }
    Some(count)
}

fn bounded_beta_zeta(t: &Db, fuel: &mut usize) -> Option<Db> {
    let (body, arg) = match t {
        Db::Let(_, value, body) => (body.as_ref(), value.as_ref()),
        Db::App(f, arg) => match f.as_ref() {
            Db::Lam(_, body) => (body.as_ref(), arg.as_ref()),
            _ => return None,
        },
        _ => return None,
    };
    let body_nodes = congruence_nodes(body, fuel)?;
    let arg_nodes = congruence_nodes(arg, fuel)?;
    // At most every body node is a substituted occurrence. This conservative
    // bound covers output Db nodes and index-shifting node visits before allocating.
    // Literal/name/universe payload sizes retain their existing representation costs.
    let maximum_work = body_nodes.checked_mul(arg_nodes.checked_add(1)?)?;
    if maximum_work > *fuel {
        return None;
    }
    *fuel -= maximum_work;
    Some(beta_open(body, arg))
}

/// Result of discharging only the canonical natural constructor prefix.
/// Unknown tails return to the existing general conversion rules.
enum NatPrefixComparison {
    Decided(bool),
    Residual(Db, Db),
    Exhausted,
}
fn natural_prefix_value(t: &Db) -> bool {
    matches!(t, Db::Lit(Literal::Nat(_))) || db_nat_peano_headed(t)
}
fn compare_nat_prefix(
    genv: &Context,
    lctx: &dyn LocalTypes,
    mut a: Db,
    mut b: Db,
) -> NatPrefixComparison {
    if let (Db::Lit(Literal::Nat(x)), Db::Lit(Literal::Nat(y))) = (&a, &b) {
        return NatPrefixComparison::Decided(x == y);
    }
    // This is a representation-level rule, not an arithmetic operation-name hook.
    // The cheap shape gate avoids scanning Nat metadata in unrelated conversion.
    if !natural_prefix_value(&a)
        || !natural_prefix_value(&b)
        || !genv.simp_rules().is_empty()
        || !crate::int_inductive::canonical_nat_environment(genv)
    {
        return NatPrefixComparison::Residual(a, b);
    }
    // A discharged successor head leaves one tail obligation, not a pending
    // recursive call. Keep one logical ticket and share the unchanged reduction
    // fuel with every whnf used to expose the next constructor.
    let parent_depth = WHNF_DEPTH.with(|d| d.get());
    if parent_depth >= MAX_RECHECK_WHNF_DEPTH {
        WHNF_EXHAUSTED.with(|e| e.set(true));
        return NatPrefixComparison::Exhausted;
    }
    // A previous independent probe may already have recorded incompleteness.
    // Preserve that flag while distinguishing a fresh refusal in this loop.
    struct ExhaustionScope(bool);
    impl Drop for ExhaustionScope {
        fn drop(&mut self) {
            WHNF_EXHAUSTED.with(|e| e.set(self.0 || e.get()));
        }
    }
    let _exhaustion_scope = ExhaustionScope(WHNF_EXHAUSTED.with(|e| e.replace(false)));
    let _depth_guard = WhnfDepthGuard(parent_depth);
    WHNF_DEPTH.with(|d| d.set(parent_depth + 1));
    if parent_depth == 0 && LOCAL_CONTEXT_BUDGET_DEPTH.with(|d| d.get() == 0) {
        WHNF_FUEL.with(|f| f.set(MAX_RECHECK_STEPS));
    }
    let mut steps = 0usize;
    let mut next_size_check = 1024usize;
    loop {
        let spent = WHNF_FUEL.with(|f| {
            let n = f.get();
            if n == 0 {
                false
            } else {
                f.set(n - 1);
                true
            }
        });
        if !spent {
            WHNF_EXHAUSTED.with(|e| e.set(true));
            return NatPrefixComparison::Exhausted;
        }
        steps += 1;
        if steps == next_size_check {
            next_size_check *= 2;
            if db_forest_size_exceeds(vec![&a, &b], MAX_RECHECK_TERM_NODES) {
                WHNF_EXHAUSTED.with(|e| e.set(true));
                return NatPrefixComparison::Exhausted;
            }
        }
        if a == b {
            return NatPrefixComparison::Decided(true);
        }
        match (&a, &b) {
            (Db::Lit(Literal::Nat(x)), Db::Lit(Literal::Nat(y))) => {
                return NatPrefixComparison::Decided(x == y)
            }
            (Db::Lit(Literal::Nat(n)), Db::Global(z))
            | (Db::Global(z), Db::Lit(Literal::Nat(n)))
                if z == "Zero" =>
            {
                return NatPrefixComparison::Decided(*n <= 0.into())
            }
            (Db::Global(z), Db::App(f, _)) | (Db::App(f, _), Db::Global(z))
                if z == "Zero" && matches!(f.as_ref(), Db::Global(s) if s == "Succ") =>
            {
                return NatPrefixComparison::Decided(false)
            }
            // Use the existing independently implemented literal→Peano rule.
            (Db::Lit(Literal::Nat(_)), _) if db_nat_peano_headed(&b) => a = db_nat_peano_step(&a),
            (_, Db::Lit(Literal::Nat(_))) if db_nat_peano_headed(&a) => b = db_nat_peano_step(&b),
            _ => {}
        }
        // Cancel only the same registered unary constructor, after each head
        // has independently reached weak head. In particular neutral tails,
        // explicit-universe references and unrelated families are not guessed.
        let both_successors = matches!((&a, &b),
            (Db::App(f, _), Db::App(g, _))
                if matches!(f.as_ref(), Db::Global(s) if s == "Succ")
                && matches!(g.as_ref(), Db::Global(s) if s == "Succ"));
        if !both_successors {
            return NatPrefixComparison::Residual(a, b);
        }
        let (Db::App(_, x), Db::App(_, y)) = (a, b) else {
            unreachable!()
        };
        a = whnf_in_owned(genv, lctx, x.into_db());
        b = whnf_in_owned(genv, lctx, y.into_db());
        if WHNF_EXHAUSTED.with(|e| e.get()) {
            return NatPrefixComparison::Exhausted;
        }
    }
}

// LAZY THEOREM DELTA, the re-checker's copy of the main kernel's rule (Lean's
// `lazy_delta_reduction`: a theorem has the lowest unfolding priority).
//
// While `FOLDING` is set, weak-head reduction treats a checked transparent theorem as a
// constant it may not unfold, and records in `FOLDED` that it declined one. That record is the
// whole soundness argument: a reduction that recorded nothing performed exactly the steps the
// unrestricted one performs, so its result IS the weak-head normal form; a reduction that
// recorded a refusal returned a reduct of the input, which can prove equalities but never
// refute them. Every conversion starts unrestricted (`def_eq` saves and clears both cells),
// so folding never leaks into a nested conversion or into what it infers.
thread_local! {
    /// Weak-head reduction keeps checked transparent theorems folded.
    static FOLDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A folded reduction declined to unfold a theorem.
    static FOLDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn theorem_folded(genv: &Context, name: &str) -> bool {
    if !FOLDING.with(|folding| folding.get())
        || !crate::type_checker::is_transparent_theorem(genv, name)
    {
        return false;
    }
    FOLDED.with(|folded| folded.set(true));
    true
}

/// Weak-head normalize with theorems folded. The flag says whether a theorem was declined, i.e.
/// whether the result may still reduce further under full transparency.
fn whnf_theorems_folded(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> (Db, bool) {
    FOLDING.with(|folding| folding.set(true));
    FOLDED.with(|folded| folded.set(false));
    let result = whnf_in(genv, lctx, t);
    FOLDING.with(|folding| folding.set(false));
    (result, FOLDED.with(|folded| folded.replace(false)))
}

struct TheoremFoldingScope(bool, bool);
impl TheoremFoldingScope {
    fn enter() -> Self {
        TheoremFoldingScope(
            FOLDING.with(|folding| folding.replace(false)),
            FOLDED.with(|folded| folded.replace(false)),
        )
    }
}
impl Drop for TheoremFoldingScope {
    fn drop(&mut self) {
        FOLDING.with(|folding| folding.set(self.0));
        FOLDED.with(|folded| folded.set(self.1));
    }
}

#[cfg(test)]
thread_local! {
    /// The conversions this thread has asked this kernel for, nested ones included.
    static DEF_EQ_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Definitional equality in the independent de Bruijn kernel.
fn def_eq(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    #[cfg(test)]
    DEF_EQ_CALLS.with(|n| n.set(n.get() + 1));
    if folded::exhausted() {
        return false;
    }
    let _folding = TheoremFoldingScope::enter();
    let _numeric_observation = arithmetic::ConversionScope::enter();
    // Syntactic identity is α-equality in de Bruijn form, and α-equal terms are definitionally
    // equal — so the two reductions below are not merely redundant, they are the whole cost. The
    // main kernel's `is_subtype` has had this door since the certified-grid work; the re-checker
    // did not, and it is what made the second kernel run a recursion the first one never had to.
    // A bound proof compares a term against ITSELF: `natLe (Succ i) 18446744073709551616` is
    // decided here in one comparison, or peeled one `Succ` at a time 2^64 times.
    if a == b {
        return true;
    }
    // Conversion recurses over the two terms' structure as well as reducing them, so it gets the
    // same stack meter. "Not equal" is the safe answer: a conversion the re-checker could not
    // finish must never be reported as one it decided, and the exhaustion flag turns the
    // resulting rejection into an honest `Unsupported`.
    let Some(_nest) = nest(stack_mark()) else {
        return false;
    };
    if borrowed_congruence(a, b, &mut 256) {
        return true;
    }
    if conversion_memo::known(genv, lctx, a, b) {
        return true;
    }
    let proved = def_eq_proved(genv, lctx, a, b);
    if proved {
        conversion_memo::record(genv, lctx, a, b);
    }
    proved
}

/// [`def_eq`] past its identity, stack and borrowed-congruence doors.
fn def_eq_proved(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    if folded::proof_references(genv, lctx, a, b) {
        return true;
    }
    if folded::equivalent(genv, lctx, a, b) {
        return true;
    }
    if borrowed_congruence_in(Some(genv), a, b, &mut 256) {
        return true;
    }
    if folded::align_definitions(genv, a, b) {
        return true;
    }
    // LAZY δ (Lean's `lazy_delta_reduction`, this kernel's own): reduce without unfolding the
    // heads, compare, and unfold one side at a time. Undecided pairs fall to the reduction-based
    // conversion below, unchanged.
    match lazy_delta::conversion(genv, lctx, a, b) {
        lazy_delta::Lazy::Decided(answer) => {
            #[cfg(debug_assertions)]
            lazy_delta::differential(genv, lctx, a, b, answer);
            answer
        }
        lazy_delta::Lazy::Open => def_eq_by_whnf(genv, lctx, a, b),
    }
}

/// Conversion by weak-head normal forms: both sides reduced (theorems folded first), then
/// compared by the literal bridges, η, structure η, congruence, singleton types and proof
/// irrelevance. The lazy loop's fallback, and its differential oracle.
fn def_eq_by_whnf(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    #[cfg(test)]
    WHNF_CONVERSIONS.with(|n| n.set(n.get() + 1));
    let (a, a_folded) = whnf_theorems_folded(genv, lctx, a);
    let (b, b_folded) = whnf_theorems_folded(genv, lctx, b);
    if a == b {
        return true;
    }
    // A side stuck on a folded theorem: first the comparisons that need no proof — the same
    // head applied to convertible arguments, then proof irrelevance — and only then unfold.
    let mut irrelevance_tried = false;
    let (a, b) = if a_folded || b_folded {
        if let (Db::App(f1, x1), Db::App(f2, x2)) = (&a, &b) {
            if def_eq(genv, lctx, f1, f2) && def_eq(genv, lctx, x1, x2) {
                return true;
            }
        }
        if proof_irrel(genv, lctx, &a, &b) {
            return true;
        }
        irrelevance_tried = true;
        let a = if a_folded { whnf_in(genv, lctx, &a) } else { a };
        let b = if b_folded { whnf_in(genv, lctx, &b) } else { b };
        if a == b {
            return true;
        }
        (a, b)
    } else {
        (a, b)
    };

    let (a, b) = match compare_nat_prefix(genv, lctx, a, b) {
        NatPrefixComparison::Decided(equal) => return equal,
        NatPrefixComparison::Residual(a, b) => (a, b),
        NatPrefixComparison::Exhausted => return false,
    };

    if let Some(a_ctor) = db_int_constructor(genv, &a) {
        let b_ctor = db_int_constructor(genv, &b);
        return def_eq(genv, lctx, &a_ctor, b_ctor.as_ref().unwrap_or(&b));
    }
    if let Some(b_ctor) = db_int_constructor(genv, &b) {
        return def_eq(genv, lctx, &a, &b_ctor);
    }

    // Peano bridge (K6): a `Nat(n)` literal is definitionally `Succ^n Zero`. Two Nat
    // literals are equal iff their counts are; a Nat literal against a `Zero`/`Succ`-headed
    // term is compared by peeling one `Succ` (terminating at `Nat(0) ≡ Zero`), where the
    // literal's reading holds (`int_inductive::reads_nat_literal`).
    match (&a, &b) {
        (Db::Lit(Literal::Nat(x)), Db::Lit(Literal::Nat(y))) => return x == y,
        (Db::Lit(Literal::Nat(n)), _)
            if db_nat_peano_headed(&b) && crate::int_inductive::reads_nat_literal(genv, n) =>
        {
            return def_eq(genv, lctx, &db_nat_peano_step(&a), &b);
        }
        (_, Db::Lit(Literal::Nat(n)))
            if db_nat_peano_headed(&a) && crate::int_inductive::reads_nat_literal(genv, n) =>
        {
            return def_eq(genv, lctx, &a, &db_nat_peano_step(&b));
        }
        _ => {}
    }

    // η-conversion: `λ. f (Var 0) ≡ g`. When exactly one side is a `λ`, compare its body
    // against the other side (shifted under the binder) applied to `Var 0`.
    if let Db::Lam(dom, body) = &a {
        if !matches!(b, Db::Lam(..)) {
            let mut ext = extend_local(lctx);
            ext.push((**dom).clone());
            let bx = whnf_in(
                genv,
                &ext,
                &Db::App(DbBox::new(shift(&b, 1, 0)), DbBox::new(Db::Var(0))),
            );
            return def_eq(genv, &ext, body, &bx);
        }
    }
    if let Db::Lam(dom, body) = &b {
        if !matches!(a, Db::Lam(..)) {
            let mut ext = extend_local(lctx);
            ext.push((**dom).clone());
            let ax = whnf_in(
                genv,
                &ext,
                &Db::App(DbBox::new(shift(&a, 1, 0)), DbBox::new(Db::Var(0))),
            );
            return def_eq(genv, &ext, &ax, body);
        }
    }

    // Structure η (the second kernel's copy of the main kernel's rule): a
    // fully-applied structure constructor is convertible with any term whose
    // projections match it field-wise.
    if let Some(eq) = try_struct_eta_db(genv, lctx, &a, &b) {
        return eq;
    }
    if let Some(eq) = try_struct_eta_db(genv, lctx, &b, &a) {
        return eq;
    }
    if db_implicit_instance_conversion(genv, lctx, &a, &b) {
        return true;
    }

    let congruent = match (&a, &b) {
        (Db::Sort(x), Db::Sort(y)) => x.equiv(y),
        (Db::Var(i), Db::Var(j)) => i == j,
        (Db::Global(m), Db::Global(n)) => m == n,
        (Db::Lit(x), Db::Lit(y)) => x == y,
        (Db::App(f1, a1), Db::App(f2, a2)) => {
            !db_bare_against_explicit(&a, &b)
                && def_eq(genv, lctx, f1, f2)
                && def_eq(genv, lctx, a1, a2)
        }
        (Db::Pi(d1, b1), Db::Pi(d2, b2)) => {
            def_eq(genv, lctx, d1, d2) && {
                let mut ext = extend_local(lctx);
                ext.push((**d1).clone());
                def_eq(genv, &ext, b1, b2)
            }
        }
        (Db::Lam(d1, b1), Db::Lam(d2, b2)) => {
            def_eq(genv, lctx, d1, d2) && {
                let mut ext = extend_local(lctx);
                ext.push((**d1).clone());
                def_eq(genv, &ext, b1, b2)
            }
        }
        (
            Db::Match {
                disc: d1,
                motive: m1,
                cases: c1,
            },
            Db::Match {
                disc: d2,
                motive: m2,
                cases: c2,
            },
        ) => {
            def_eq(genv, lctx, d1, d2)
                && def_eq(genv, lctx, m1, m2)
                && c1.len() == c2.len()
                && c1
                    .iter()
                    .zip(c2.iter())
                    .all(|(x, y)| def_eq(genv, lctx, x, y))
        }
        (Db::Fix(b1), Db::Fix(b2)) => {
            let mut ext = extend_local(lctx);
            ext.push(Db::Sort(Universe::Prop)); // `rec` placeholder (its type is never consulted)
            def_eq(genv, &ext, b1, b2)
        }
        (
            Db::MutualFix {
                defs: d1,
                index: i1,
            },
            Db::MutualFix {
                defs: d2,
                index: i2,
            },
        ) => {
            i1 == i2 && d1.len() == d2.len() && {
                let mut ext = extend_local(lctx);
                for _ in 0..d1.len() {
                    ext.push(Db::Sort(Universe::Prop)); // the `n` recursive-name placeholders
                }
                d1.iter()
                    .zip(d2.iter())
                    .all(|(a, b)| def_eq(genv, &ext, a, b))
            }
        }
        (
            Db::Const {
                name: n1,
                levels: l1,
            },
            Db::Const {
                name: n2,
                levels: l2,
            },
        ) => n1 == n2 && l1.len() == l2.len() && l1.iter().zip(l2.iter()).all(|(a, b)| a.equiv(b)),
        _ => false,
    };
    if congruent {
        return true;
    }

    // Independently recognize fieldless, non-indexed singleton types.
    if let Ok(a_type) = infer_reduct(genv, lctx, &a) {
        let a_type = whnf_in(genv, lctx, &a_type);
        let (head, _) = spine(&a_type);
        if let Db::Global(name) | Db::Const { name, .. } = head {
            let mut arity = genv.get_global(&name);
            let mut count = 0;
            while let Some(Term::Pi { body_type, .. }) = arity {
                count += 1;
                arity = Some(body_type);
            }
            let ctors = genv.get_constructors(&name);
            let params = genv.inductive_num_params(&name);
            if genv.is_inductive(&name)
                && matches!(arity, Some(Term::Sort(_)))
                && count == params
                && ctors.len() == 1
                && genv.mutual_block_of(&name).is_none_or(|b| b.len() == 1)
            {
                let mut cty = ctors[0].1;
                let mut fields = 0;
                while let Term::Pi { body_type, .. } = cty {
                    fields += 1;
                    cty = body_type;
                }
                if fields == params {
                    if let Ok(b_type) = infer_reduct(genv, lctx, &b) {
                        if def_eq(genv, lctx, &a_type, &b_type) {
                            return true;
                        }
                    }
                }
            }
        }
    }

    // Proof irrelevance: two proofs of the same proposition are equal.
    !irrelevance_tried && proof_irrel(genv, lctx, &a, &b)
}

/// A bare reference is its own implicit instantiation — this kernel's copy of the main kernel's
/// `implicit_instance_conversion`, over its own application rule: `n a₁ … aₖ ≡ n.{ℓ̄} b₁ … bₖ`
/// exactly when the levels this kernel solves for the bare spine are equivalent to `ℓ̄`
/// position by position and `aᵢ ≡ bᵢ`. An unsolved level concludes nothing.
fn db_implicit_instance_conversion(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    if !db_bare_against_explicit(a, b) {
        return false;
    }
    let (ah, aa) = spine_ref(a);
    let (bh, ba) = spine_ref(b);
    if aa.len() != ba.len() {
        return false;
    }
    let (name, levels, bare_args) = match (ah, bh) {
        (Db::Global(n), Db::Const { name, levels }) if n == name => (name, levels, &aa),
        (Db::Const { name, levels }, Db::Global(n)) if n == name => (name, levels, &ba),
        _ => return false,
    };
    // With no arguments the bare reference stands at its declaration's own levels (`term`'s
    // rule 4).
    if bare_args.is_empty() {
        return const_declaration(genv, name).is_some_and(|(declared, const_type)| {
            let own = crate::term::level_telescope(declared, const_type);
            own.len() == levels.len()
                && own
                    .iter()
                    .zip(levels.iter())
                    .all(|(p, l)| l.equiv(&Universe::Var(p.clone())))
        });
    }
    // The spine is a term conversion reached, so typing it is typing a reduct.
    let solved = {
        let _reduct = ReductInference::enter();
        db_implicit_instance_levels(genv, lctx, name, bare_args)
    };
    solved.is_some_and(|solved| {
        solved.len() == levels.len() && solved.iter().zip(levels.iter()).all(|(x, y)| x.equiv(y))
    }) && aa
        .iter()
        .zip(ba.iter())
        .all(|(x, y)| def_eq(genv, lctx, x, y))
}

/// Whether the spines of `a` and `b` are headed by the same name, bare in one and explicit in the
/// other. Such a pair converts only by [`db_implicit_instance_conversion`]: congruence never
/// compares the two heads, since a bare head's levels come from its arguments.
fn db_bare_against_explicit(a: &Db, b: &Db) -> bool {
    matches!(
        (spine_head(a), spine_head(b)),
        (Db::Global(n), Db::Const { name, .. }) | (Db::Const { name, .. }, Db::Global(n))
            if n == name
    )
}

/// The levels this kernel's application rule solves for the bare spine `name args`, zipped in
/// the positional order of a `Db::Const` reference (`term::resolve_const_levels`).
///
/// The spine is walked with this kernel's own application steps — `infer` of the head, `whnf`,
/// `db_implicit_levels_for_head`, and `db_domain_level_solution` per argument — so the instance
/// read here is the one the typing of the same spine used.
fn db_implicit_instance_levels(
    genv: &Context,
    lctx: &dyn LocalTypes,
    name: &str,
    args: &[&Db],
) -> Option<Vec<Universe>> {
    let (declared, const_type) = const_declaration(genv, name)?;
    let positional = crate::term::level_telescope(declared, const_type);
    let head = Db::Global(name.to_string());
    let mut current = infer(genv, lctx, &head).ok()?;
    let mut solution = HashMap::new();
    for (i, arg) in args.iter().enumerate() {
        let exposed = whnf_in_owned(genv, lctx, current);
        let exposed = if i == 0 {
            db_implicit_levels_for_head(&head, exposed)
        } else {
            exposed
        };
        let Db::Pi(dom, body) = exposed else {
            return None;
        };
        let body = match db_domain_level_solution(genv, lctx, &dom, arg) {
            Some(subst) => {
                let body = db_instantiate_universes(&body, &subst);
                solution.extend(subst);
                body
            }
            None => body.into_db(),
        };
        current = beta_open_owned(body, arg);
    }
    let metas = crate::term::fresh_level_metas(name, &positional);
    positional
        .iter()
        .map(|p| {
            let solved = metas.get(p)?.substitute(&solution);
            (!crate::term::universe_mentions(&solved, &crate::term::is_level_meta))
                .then_some(solved)
        })
        .collect()
}

/// Structure η in the de Bruijn re-checker, ONE direction (see the main kernel's
/// `try_structure_eta`). `None` when `mk_term` is not a fully-applied registered
/// structure constructor, or `other` is the same constructor (ordinary congruence).
fn try_struct_eta_db(
    genv: &Context,
    lctx: &dyn LocalTypes,
    mk_term: &Db,
    other: &Db,
) -> Option<bool> {
    let (head, args) = spine_ref(mk_term);
    let (hname, levels) = match &head {
        Db::Global(n) => (n, None),
        Db::Const { name, levels } => (name, Some(levels)),
        _ => return None,
    };
    let (_sname, info) = genv.struct_of_constructor(hname)?;
    let nfields = info.projections.len();
    if args.len() != info.num_params + nfields {
        return None;
    }
    let ohead = spine_head(other);
    if matches!(&ohead, Db::Global(n) | Db::Const {name:n, ..} if n == hname) {
        return None;
    }
    let params = &args[..info.num_params];
    let field_args = &args[info.num_params..];
    Some(info.projections.iter().enumerate().all(|(i, proj)| {
        // `proj params… other` as a Db application.
        let mut app = match levels {
            Some(levels) => Db::Const {
                name: proj.clone(),
                levels: levels.clone(),
            },
            None => Db::Global(proj.clone()),
        };
        for p in params {
            app = Db::App(DbBox::new(app), DbBox::new((*p).clone()));
        }
        app = Db::App(DbBox::new(app), DbBox::new(other.clone()));
        def_eq(genv, lctx, &field_args[i], &app)
    }))
}

/// Proof irrelevance for the re-checker, from the same specification as the main kernel's:
/// `a ≡ b` when `a : A`, `b : B`, `A ≡ B`, and that common type is a `Prop` (or the
/// definitionally-irrelevant `SProp`).
///
/// The premise is a property of the pair, not of the left element — `A ≡ B` makes one
/// propositional exactly when the other is — so the check is written over BOTH inferred types.
/// Interrogating only `a` costs nothing when both types infer and silently breaks symmetry
/// when one does not; conversion that depends on argument order is not conversion.
fn proof_irrel(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> bool {
    let (Ok(ta), Ok(tb)) = (infer_reduct(genv, lctx, a), infer_reduct(genv, lctx, b)) else {
        return false;
    };
    let propositional = |t: &Db| {
        matches!(
            infer_reduct(genv, lctx, t).map(|s| whnf_in_owned(genv, lctx, s)),
            Ok(Db::Sort(level)) if level == Universe::SProp || level.equiv(&Universe::Prop)
        )
    };
    if !propositional(&ta) && !propositional(&tb) {
        return false;
    }
    def_eq(genv, lctx, &ta, &tb)
}

/// Cumulative subtyping `sub ≤ sup`: sorts follow `Prop ≤ Type i ≤ Type j`; a `Π` is
/// covariant in its codomain and invariant in its domain; everything else is `def_eq`.
fn is_sub(genv: &Context, lctx: &dyn LocalTypes, sub: &Db, sup: &Db) -> bool {
    if folded::exhausted() {
        return false;
    }
    // As in def_eq, de Bruijn identity proves reflexivity without reducing the
    // already inferred types. Inference still checks every term and annotation.
    if sub == sup {
        return true;
    }
    if borrowed_congruence(sub, sup, &mut 256) {
        return true;
    }
    if folded::equivalent(genv, lctx, sub, sup) {
        return true;
    }
    if borrowed_congruence_in(Some(genv), sub, sup, &mut 256) {
        return true;
    }
    if folded::align_definitions(genv, sub, sup) {
        return true;
    }
    let s = whnf_in(genv, lctx, sub);
    let t = whnf_in(genv, lctx, sup);
    match (&s, &t) {
        (Db::Sort(x), Db::Sort(y)) => x.is_subtype_of(y),
        (Db::Pi(d1, b1), Db::Pi(d2, b2)) => {
            def_eq(genv, lctx, d1, d2) && {
                let mut ext = extend_local(lctx);
                ext.push((**d1).clone());
                is_sub(genv, &ext, b1, b2)
            }
        }
        _ => def_eq(genv, lctx, &s, &t),
    }
}

// ---------------------------------------------------------------------------
// Inference
// ---------------------------------------------------------------------------

/// The type of de Bruijn variable `k`, read from `lctx` (binder types, innermost LAST,
/// each stored in the scope of ITS binding point) and lifted by `k+1` to current depth.
fn var_type(lctx: &dyn LocalTypes, k: usize) -> RResult<Db> {
    if k >= lctx.len() {
        return Err(ReCheckError::ill(format!(
            "de Bruijn index {} out of range",
            k
        )));
    }
    let stored = lctx
        .binding(lctx.len() - 1 - k)
        .expect("index checked against telescope length");
    Ok(shift(stored, (k + 1) as isize, 0))
}

/// Infer the type of `t` under global env `genv` and local context `lctx`.
fn infer(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> RResult<Db> {
    // The INPUT's inference is NOT metered. It recurses over the term's own structure, whose
    // depth the door at `recheck`'s entry already bounds by `MAX_TERM_DEPTH`; a 500-deep Peano
    // numeral is legitimate work that a caller with stack to spare must be allowed to finish. The
    // meter guards what the door did not measure: REDUCTION — `whnf` and `def_eq` — where the
    // recursion is driven by how far a computation runs, and the inference of the reducts it
    // produces (`infer_reduct`). Metering the input as well would make the re-checker refuse work
    // the thread had room for, which
    // `kernel_core_extra::the_kernel_is_correct_at_depths_a_default_thread_cannot_afford` locks
    // against: the limit is the ambient stack, not the logic.
    let _metered = match REDUCT_INFERENCE.with(|m| m.get()) {
        false => None,
        true => match nest(stack_mark()) {
            Some(guard) => Some(guard),
            None => {
                return Err(ReCheckError::resource_limit(
                    "the type of a reduct nests past the re-checker's stack meter",
                ))
            }
        },
    };
    match t {
        Db::Sort(u) => Ok(Db::Sort(u.succ())),
        Db::Var(k) => var_type(lctx, *k),
        Db::Global(name) => {
            let ty = genv.get_global(name).ok_or_else(|| {
                ReCheckError::ill(
                    crate::type_checker::unbound_diagnosis(name)
                        .unwrap_or_else(|| format!("unknown global '{}'", name)),
                )
            })?;
            // The stored type is returned VERBATIM. A declaration whose type mentions level
            // variables is universe-polymorphic even with no levels written, but the
            // metavariables are introduced by the APPLICATION rule, where there is an argument
            // to solve them from — never here, where it would rename the level variables a
            // polymorphic definition's own body is checked against.
            to_db(ty, &mut Vec::new())
        }
        // A universe-polymorphic reference. The telescope, the closure rule and the positional
        // zip are the SHARED SPECIFICATION in `term::resolve_const_levels`; the lookup below is
        // this kernel's own, so the two kernels agree about what a polymorphic reference MEANS
        // without either one borrowing the other's arithmetic.
        Db::Const { name, levels } => {
            let (declared, ty) = const_declaration(genv, name).ok_or_else(|| {
                ReCheckError::ill(
                    crate::type_checker::unbound_diagnosis(name)
                        .unwrap_or_else(|| format!("unknown universe-poly global '{}'", name)),
                )
            })?;
            let subst = crate::term::resolve_const_levels(name, declared, ty, levels)
                .map_err(|e| ReCheckError::ill(e.to_string()))?;
            to_db(
                &crate::term::instantiate_universes(ty, &subst),
                &mut Vec::new(),
            )
        }
        Db::Lit(Literal::Nat(n)) if *n < logicaffeine_base::BigInt::from_i64(0) => Err(
            ReCheckError::ill("a `Nat` literal must be non-negative".to_string()),
        ),
        Db::Lit(l) => Ok(Db::Global(lit_type_name(l).to_string())),
        Db::Pi(dom, body) => {
            let dom_sort = infer_sort(genv, lctx, dom)?;
            let mut ext = extend_local(lctx);
            ext.push((**dom).clone());
            let body_sort = infer_sort(genv, &ext, body)?;
            // Impredicative product formation via `imax` (see the main kernel's
            // `Pi` arm) — the second kernel decides the same level algebra.
            let pi_sort = dom_sort.imax(&body_sort);
            Ok(Db::Sort(pi_sort))
        }
        Db::Lam(dom, body) => {
            let _ = infer_sort(genv, lctx, dom)?;
            let mut ext = extend_local(lctx);
            ext.push((**dom).clone());
            let body_ty = infer(genv, &ext, body)?;
            Ok(Db::Pi(dom.clone(), DbBox::new(body_ty)))
        }
        Db::App(f, a) => {
            let f_ty = whnf_in_owned(genv, lctx, infer(genv, lctx, f)?);
            let f_ty = db_implicit_levels_for_head(f.as_ref(), f_ty);
            match f_ty {
                Db::Pi(dom, body) => {
                    // LEVEL SOLVING (shared specification, independent implementation): a
                    // domain carrying level metavariables is solved against the argument's
                    // ACTUAL type before anything is checked. Choosing an instantiation is not
                    // accepting one — `check` still runs against the substituted domain.
                    let (dom, body) =
                        solve_domain_levels_owned(genv, lctx, dom.into_db(), body.into_db(), a);
                    check(genv, lctx, a, &dom)?;
                    // The main kernel SHARES a large argument behind a ζ-binding rather than
                    // copying it; this kernel substitutes eagerly. That is deliberate, not a
                    // divergence: `let x := a in B` and `B[x := a]` are ζ-equal and BOTH kernels
                    // implement ζ, so the two routes meet in `def_eq`. Sharing here would buy
                    // nothing — this kernel `whnf`s the function type at every application, which
                    // ζ-reduces the binding away immediately.
                    Ok(beta_open_owned(body, a))
                }
                other => Err(ReCheckError::ill(format!(
                    "application of a non-function (type {})",
                    from_db(&other, lctx.len())
                ))),
            }
        }
        Db::Match {
            disc,
            motive,
            cases,
        } => infer_match(genv, lctx, disc, motive, cases),
        Db::Fix(body) => infer_fix(genv, lctx, body),
        Db::MutualFix { defs, index } => infer_mutual_fix(genv, lctx, defs, *index),
        // Let: check `ty` is a sort and `value : ty`, then type the body with the
        // value substituted in (zeta) — exactly the main kernel's by-substitution
        // rule, so both kernels agree.
        Db::Let(ty, value, body) => {
            let _ = infer_sort(genv, lctx, ty)?;
            check(genv, lctx, value, ty)?;
            std::thread_local! { static LET_PROBE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
            struct Probe(bool);
            impl Drop for Probe {
                fn drop(&mut self) {
                    LET_PROBE.with(|flag| flag.set(self.0));
                }
            }
            let probe = Probe(LET_PROBE.with(|flag| flag.replace(true)));
            let nested = probe.0;
            let mut ext = extend_local(lctx);
            ext.push((**ty).clone());
            let result = infer(genv, &ext, body);
            drop(probe);
            match result {
                // Independently apply the substitution lemma in de Bruijn form.
                Ok(body_type) => Ok(beta_open_owned(body_type, value)),
                Err(error) if nested => Err(error),
                Err(_) => infer(genv, lctx, &beta_open(body, value)),
            }
        }
    }
}

/// The effective motive of a `match`: a function motive `λx:I.T` (checked: its domain
/// matches the discriminant type, its codomain is a type) is used as-is; a raw type `T`
/// (a `Sort`-typed motive) is wrapped as `λ_:I. T` (shifting `T` under the new binder).
fn effective_motive(
    genv: &Context,
    lctx: &dyn LocalTypes,
    motive: &Db,
    disc_ty: &Db,
) -> RResult<Db> {
    #[cfg(test)]
    MOTIVE_CHECKS.with(|n| {
        if let Some(count) = n.get() {
            n.set(Some(count + 1));
        }
    });
    let motive_ty = whnf_in_owned(genv, lctx, infer(genv, lctx, motive)?);
    match &motive_ty {
        Db::Pi(dom, cod) => {
            if !def_eq(genv, lctx, dom, disc_ty) {
                return Err(ReCheckError::ill(format!(
                    "motive domain {} does not match discriminant type {}",
                    from_db(dom, lctx.len()),
                    from_db(disc_ty, lctx.len())
                )));
            }
            let mut ext = extend_local(lctx);
            ext.push(disc_ty.clone());
            if !matches!(whnf_in(genv, &ext, cod), Db::Sort(_)) {
                return Err(ReCheckError::ill("motive type codomain is not a sort"));
            }
            Ok(motive.clone())
        }
        Db::Sort(_) => Ok(Db::Lam(
            DbBox::new(disc_ty.clone()),
            DbBox::new(shift(motive, 1, 0)),
        )),
        other => Err(ReCheckError::ill(format!(
            "motive is neither a function nor a type (type {})",
            from_db(other, lctx.len())
        ))),
    }
}

/// Type a `fix rec. body`: compute its (structural) type, INDEPENDENTLY verify the
/// termination guard, sanity-check the body under `rec : T`, and return `T`. The guard
/// is the load-bearing soundness check — without it `fix f. f` inhabits every type.
fn infer_fix(genv: &Context, lctx: &dyn LocalTypes, body: &Db) -> RResult<Db> {
    let fix_level = lctx.len();

    // 1. The fixpoint's type, read from the body's λ-telescope and its `match` codomain
    //    (which does not depend on `rec`). The body's indices are relative to a context
    //    that INCLUDES the `rec` binder, so `fix_type` must run with that slot present
    //    (a placeholder — its type is never consulted), or every index is off by one.
    let mut inner = extend_local(lctx);
    inner.push(Db::Sort(Universe::Prop));
    let fix_ty_inner = fix_type(genv, &inner, body)?;
    // The type may not reference `rec` (level `fix_level`, index 0 at this base); it does not, so
    // that slot is dropped to express the type in the enclosing context.
    if mentions_index(&fix_ty_inner, 0) {
        return Err(ReCheckError::ill(
            "a fixpoint's type mentions the fixpoint itself".to_string(),
        ));
    }
    let fix_ty = shift(&fix_ty_inner, -1, 0);

    // 2. THE GUARDIAN — an independent structural-decrease check. The body lives one
    //    binder (`rec`, at level `fix_level`) beneath the current depth.
    check_terminates(genv, body, fix_level)?;

    // 3. Sanity: the body type-checks with `rec : T` in scope.
    let mut ext = extend_local(lctx);
    ext.push(fix_ty.clone());
    let _ = infer(genv, &ext, body)?;

    Ok(fix_ty)
}

/// Type a `mutualfix { b₀ … b_{n-1} }.index` — the mutual analog of [`infer_fix`]. Each
/// body's structural type is read from its λ-telescope (independent of the recursive
/// names), the MUTUAL termination guard is verified over the whole block, the bodies are
/// sanity-checked with all `n` names bound, and the `index`-th type is returned.
fn infer_mutual_fix(
    genv: &Context,
    lctx: &dyn LocalTypes,
    defs: &[Db],
    index: usize,
) -> RResult<Db> {
    let n = defs.len();
    if n == 0 || index >= n {
        return Err(ReCheckError::ill("malformed mutual fixpoint".to_string()));
    }
    let base = lctx.len();

    // 1. Structural type of each body, computed with `n` placeholder binders in scope
    //    (the bodies' indices assume all `n` mutual names are present). The types do not
    //    mention those names, so drop the `n` slots (shift down by `n`) into the
    //    enclosing context — each `types[j]` is then expressed relative to depth `base`.
    let mut inner = extend_local(lctx);
    for _ in 0..n {
        inner.push(Db::Sort(Universe::Prop));
    }
    let mut types: Vec<Db> = Vec::with_capacity(n);
    for body in defs {
        let ty_inner = fix_type(genv, &inner, body)?;
        types.push(shift(&ty_inner, -(n as isize), 0));
    }

    // 2. THE GUARDIAN — the mutual structural-decrease check.
    check_terminates_mutual(genv, defs, base)?;

    // 3. Sanity: every body type-checks with all `n` names bound. Binder `j` sits at level
    //    `base + j`, so its type must be expressed relative to depth `base + j` (shift up
    //    by `j` from the `base`-relative `types[j]`).
    let mut ext = extend_local(lctx);
    for (j, ty) in types.iter().enumerate() {
        ext.push(shift(ty, j as isize, 0));
    }
    for body in defs {
        let _ = infer(genv, &ext, body)?;
    }

    Ok(types[index].clone())
}

/// The structural type of a fixpoint body: each leading `λ(_:A). …` contributes a
/// `Π(_:A). …`, and the innermost `match` contributes its return type `motive(disc)`
/// (independent of `rec`). A non-`match` tail is typed directly.
fn fix_type(genv: &Context, lctx: &dyn LocalTypes, body: &Db) -> RResult<Db> {
    match body {
        Db::Lam(dom, inner) => {
            let _ = infer_sort(genv, lctx, dom)?;
            let mut ext = extend_local(lctx);
            ext.push((**dom).clone());
            let inner_ty = fix_type(genv, &ext, inner)?;
            Ok(Db::Pi(dom.clone(), DbBox::new(inner_ty)))
        }
        Db::Match { disc, motive, .. } => {
            let disc_ty = whnf_in_owned(genv, lctx, infer(genv, lctx, disc)?);
            match_return_type(genv, lctx, motive, disc, &disc_ty)
        }
        // A convoy: a `match` applied to further arguments, typed as the `match` is and then
        // applied to them.
        Db::App(..) if matches!(spine_head(body), Db::Match { .. }) => {
            let (head, args) = spine_ref(body);
            let mut ty = fix_type(genv, lctx, head)?;
            for arg in args {
                match whnf_in_owned(genv, lctx, ty) {
                    Db::Pi(_, codomain) => ty = beta_open(&codomain, arg),
                    other => {
                        return Err(ReCheckError::ill(format!(
                            "a convoy applies a match whose type is {:?}, not a function",
                            other
                        )))
                    }
                }
            }
            Ok(ty)
        }
        other => infer(genv, lctx, other),
    }
}

/// Whether the de Bruijn index `k`, counted from `t`'s root, occurs in `t`.
fn mentions_index(t: &Db, k: usize) -> bool {
    match t {
        Db::Var(i) => *i == k,
        Db::Pi(a, b) | Db::Lam(a, b) => mentions_index(a, k) || mentions_index(b, k + 1),
        Db::App(a, b) => mentions_index(a, k) || mentions_index(b, k),
        Db::Let(a, b, body) => {
            mentions_index(a, k) || mentions_index(b, k) || mentions_index(body, k + 1)
        }
        Db::Match {
            disc,
            motive,
            cases,
        } => {
            mentions_index(disc, k)
                || mentions_index(motive, k)
                || cases.iter().any(|c| mentions_index(c, k))
        }
        Db::Fix(body) => mentions_index(body.as_ref(), k + 1),
        Db::MutualFix { defs, .. } => defs.iter().any(|d| mentions_index(d, k + defs.len())),
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => false,
    }
}

// ---------------------------------------------------------------------------
// The termination guard (Giménez 1995 / the Coq guard) — independently re-derived.
//
// A fixpoint is sound only if every recursive call decreases a STRUCTURAL argument.
// Without this, `fix f. f` (and the higher-order escape `(λg. g Zero) f`) inhabit every
// type, including `False`. This is a SEPARATE implementation of the main kernel's
// `termination` module. Crucially, the main kernel tracks shadowing with explicit
// `struct_param_live`/`fix_name_live` flags because it works with NAMES; here, working
// in de Bruijn LEVELS, shadowing is automatic — an inner binder gets a fresh level, so a
// reference can never be mistaken for the recursive name or the structural parameter.
// That structural simplicity is the point: the two guards cannot share a shadowing bug.
// ---------------------------------------------------------------------------

/// The de Bruijn LEVEL referred to by index `k` at the given `depth` (`None` if the
/// index is out of range). A level is absolute, so it identifies a binder independently
/// of how deep the reference is — which is what makes shadowing a non-issue.
fn level_of(depth: usize, k: usize) -> Option<usize> {
    depth.checked_sub(1)?.checked_sub(k)
}

/// The inductive each of a (named) constructor's VALUE FIELDS is typed by, in order, skipping
/// the inductive's `num_type_params` leading parameter binders. The empty string where a field's
/// type does not head an inductive — such a field is still a structural subterm, but there is
/// nothing to match on it.
fn named_field_heads(genv: &Context, ctor_type: &Term, num_type_params: usize) -> Vec<String> {
    let mut heads = Vec::new();
    let mut cur = ctor_type;
    let mut i = 0usize;
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        if i >= num_type_params {
            let mut head = param_type.as_ref();
            while let Term::App(f, _) = head {
                head = f;
            }
            heads.push(match head {
                Term::Global(n) | Term::Const { name: n, .. } if genv.is_inductive(n) => n.clone(),
                _ => String::new(),
            });
        }
        cur = body_type;
        i += 1;
    }
    heads
}

/// Locate a fixpoint body's structural parameter: peel the λ-telescope from `base_depth`,
/// pick the scrutinee of the innermost `match` if inductive-typed (else the first
/// inductive-typed binder). Returns the parameter's LEVEL, its inductive name, the body
/// just past it, and its argument POSITION in the telescope.
fn locate_struct<'a>(
    genv: &Context,
    body: &'a Db,
    base_depth: usize,
) -> RResult<(usize, String, &'a Db, usize)> {
    let mut chain: Vec<(usize, Option<String>, &Db)> = Vec::new();
    let mut cur = body;
    let mut depth = base_depth;
    while let Db::Lam(dom, inner) = cur {
        let dom_h = whnf(genv, dom);
        chain.push((
            depth,
            extract_inductive(genv, &dom_h).map(|(n, _)| n),
            inner,
        ));
        cur = inner;
        depth += 1;
    }
    // The innermost `match`, read through an application spine: a convoy `(match v … ) w …`
    // recurses on `v` as a bare `match` does.
    let scrutinee = match spine_head(cur) {
        Db::Match { disc, .. } => match &**disc {
            Db::Var(k) => {
                level_of(depth, *k).and_then(|lvl| chain.iter().position(|(l, ..)| *l == lvl))
            }
            _ => None,
        },
        _ => None,
    };
    let idx = scrutinee
        .filter(|&i| chain[i].1.is_some())
        .or_else(|| chain.iter().position(|(_, ind, _)| ind.is_some()))
        .ok_or_else(|| {
            ReCheckError::ill(
                "fixpoint has no inductive parameter for structural recursion".to_string(),
            )
        })?;
    Ok((
        chain[idx].0,
        chain[idx].1.clone().unwrap(),
        chain[idx].2,
        idx,
    ))
}

/// Verify the fixpoint `body` (with `rec` bound at level `fix_level`) terminates: locate
/// the structural parameter and check that every recursive call decreases it. A single
/// fixpoint is the one-entry case of the block guard.
fn check_terminates(genv: &Context, body: &Db, fix_level: usize) -> RResult<()> {
    let (struct_level, ind, guard_body, struct_pos) = locate_struct(genv, body, fix_level + 1)?;
    let mut fix_positions = HashMap::new();
    fix_positions.insert(fix_level, struct_pos);
    guard(
        genv,
        guard_body,
        &fix_positions,
        struct_level,
        &ind,
        &HashMap::new(),
        struct_level + 1,
    )
}

/// Verify a MUTUAL block of `n` fixpoint bodies terminates. The `n` names occupy levels
/// `base .. base+n-1`, so each body lives beneath all of them. Every member's structural
/// position is found first (assembling `level → position`); then each body is guarded so
/// that a call to ANY member decreases the CURRENT body's structural parameter — the
/// mutual Giménez guard, independently re-derived alongside the main kernel's.
fn check_terminates_mutual(genv: &Context, defs: &[Db], base: usize) -> RResult<()> {
    let n = defs.len();
    let base_depth = base + n;
    let mut fix_positions = HashMap::new();
    let mut located: Vec<(usize, String, &Db)> = Vec::with_capacity(n);
    for (j, body) in defs.iter().enumerate() {
        let (struct_level, ind, guard_body, struct_pos) = locate_struct(genv, body, base_depth)?;
        fix_positions.insert(base + j, struct_pos);
        located.push((struct_level, ind, guard_body));
    }
    for (struct_level, ind, guard_body) in &located {
        guard(
            genv,
            guard_body,
            &fix_positions,
            *struct_level,
            ind,
            &HashMap::new(),
            struct_level + 1,
        )?;
    }
    Ok(())
}

/// Walk `term` (at `depth`) checking every recursive call — to ANY block member whose
/// level → structural-position mapping is in `fix_positions` — applies a structurally-
/// smaller argument (a variable whose level is in `smaller`, which maps each constructor-bound
/// variable's level to the inductive its own type heads — matching on the structural parameter
/// `struct_level` of `struct_type` seeds it, and matching on an ENTRY of it extends it, because
/// the structural-subterm relation is transitive).
fn guard(
    genv: &Context,
    term: &Db,
    fix_positions: &HashMap<usize, usize>,
    struct_level: usize,
    struct_type: &str,
    smaller: &HashMap<usize, String>,
    depth: usize,
) -> RResult<()> {
    match term {
        Db::App(..) => {
            let (head, args) = spine(term);
            if let Db::Var(k) = &head {
                if let Some(&pos) = level_of(depth, *k).and_then(|lvl| fix_positions.get(&lvl)) {
                    // A recursive call to some member: the argument at THAT member's
                    // structural position must be structurally smaller — a constructor-
                    // bound variable, or an APPLICATION `h a…` whose head `h` is one (the
                    // applied-smaller / Giménez rule, sound by strict positivity — see the
                    // main kernel's `verify_structural_arg_smaller`).
                    match args.get(pos) {
                        Some(arg) => {
                            let mut h: &Db = arg;
                            while let Db::App(f, _) = h {
                                h = f;
                            }
                            match h {
                                Db::Var(j)
                                    if level_of(depth, *j)
                                        .is_some_and(|l| smaller.contains_key(&l)) => {}
                                Db::Var(_) => {
                                    return Err(ReCheckError::ill(
                                        "recursive call on an argument not headed by a \
                                         structurally-smaller variable"
                                            .to_string(),
                                    ))
                                }
                                _ => {
                                    return Err(ReCheckError::ill(
                                        "recursive call whose structural argument is not a \
                                         variable or an application of one — cannot certify it \
                                         decreases"
                                            .to_string(),
                                    ))
                                }
                            }
                        }
                        None => {
                            return Err(ReCheckError::ill(
                                "recursive call is missing its structural argument".to_string(),
                            ))
                        }
                    }
                    for a in &args {
                        guard(
                            genv,
                            a,
                            fix_positions,
                            struct_level,
                            struct_type,
                            smaller,
                            depth,
                        )?;
                    }
                    return Ok(());
                }
            }
            guard(
                genv,
                &head,
                fix_positions,
                struct_level,
                struct_type,
                smaller,
                depth,
            )?;
            for a in &args {
                guard(
                    genv,
                    a,
                    fix_positions,
                    struct_level,
                    struct_type,
                    smaller,
                    depth,
                )?;
            }
            Ok(())
        }
        Db::Match {
            disc,
            motive,
            cases,
        } => {
            // The return motive is an ordinary subterm and MUST be guarded too (a recursive
            // occurrence in the return predicate would otherwise evade the check).
            guard(
                genv,
                motive,
                fix_positions,
                struct_level,
                struct_type,
                smaller,
                depth,
            )?;
            // A match on the (un-shadowed) structural parameter guards the recursive
            // calls in its cases: each constructor argument is a structural subterm.
            // A match guards when its scrutinee's fields are structural subterms of the
            // structural parameter: the parameter ITSELF, or a variable already known smaller,
            // since the subterm relation is transitive. The second case is what admits
            // `match n with Succ m => match m with Succ k => ... f k` - every `| n+2 => ...`
            // definition there is.
            if let Db::Var(k) = &**disc {
                if level_of(depth, *k) == Some(struct_level) {
                    return guard_match_cases(
                        genv,
                        cases,
                        struct_type,
                        fix_positions,
                        struct_level,
                        smaller,
                        depth,
                    );
                }
                if let Some(ind) = level_of(depth, *k)
                    .and_then(|lvl| smaller.get(&lvl))
                    .filter(|i| !i.is_empty())
                {
                    let ind = ind.clone();
                    return guard_match_cases(
                        genv,
                        cases,
                        &ind,
                        fix_positions,
                        struct_level,
                        smaller,
                        depth,
                    );
                }
            }
            guard(
                genv,
                disc,
                fix_positions,
                struct_level,
                struct_type,
                smaller,
                depth,
            )?;
            for c in cases {
                guard(
                    genv,
                    c,
                    fix_positions,
                    struct_level,
                    struct_type,
                    smaller,
                    depth,
                )?;
            }
            Ok(())
        }
        // Guard a binder's DOMAIN annotation (current depth) as well as its body (one
        // deeper). `Fix` has no domain.
        Db::Lam(dom, inner) | Db::Pi(dom, inner) => {
            guard(
                genv,
                dom,
                fix_positions,
                struct_level,
                struct_type,
                smaller,
                depth,
            )?;
            guard(
                genv,
                inner,
                fix_positions,
                struct_level,
                struct_type,
                smaller,
                depth + 1,
            )
        }
        Db::Fix(inner) => guard(
            genv,
            inner,
            fix_positions,
            struct_level,
            struct_type,
            smaller,
            depth + 1,
        ),
        // A nested mutual block introduces `n` fresh levels; its own termination is checked
        // when it is type-checked. Descend into each body, past those `n` binders — an
        // outer recursive call from within it must still decrease.
        Db::MutualFix { defs, .. } => {
            for b in defs {
                guard(
                    genv,
                    b,
                    fix_positions,
                    struct_level,
                    struct_type,
                    smaller,
                    depth + defs.len(),
                )?;
            }
            Ok(())
        }
        Db::Let(ty, value, body) => {
            // `ty`/`value` at the current depth; `body` one binder deeper.
            //
            // A let whose VALUE is an already-smaller variable binds the SAME VALUE, so its own
            // level inherits that smallness - zeta makes `let x := v in b` and `b[x := v]` the
            // same term, and a guard that disagrees rejects a definition for how it was spelled.
            guard(
                genv,
                ty,
                fix_positions,
                struct_level,
                struct_type,
                smaller,
                depth,
            )?;
            guard(
                genv,
                value,
                fix_positions,
                struct_level,
                struct_type,
                smaller,
                depth,
            )?;
            let mut inner = smaller.clone();
            if let Db::Var(k) = &**value {
                if let Some(ind) = level_of(depth, *k).and_then(|lvl| smaller.get(&lvl)) {
                    inner.insert(depth, ind.clone());
                }
            }
            guard(
                genv,
                body,
                fix_positions,
                struct_level,
                struct_type,
                &inner,
                depth + 1,
            )
        }
        Db::Var(k) => {
            // A bare member name (not the head of a guarded call) is the higher-order
            // escape — a recursive name used as a first-class value. Reject it.
            if level_of(depth, *k).is_some_and(|lvl| fix_positions.contains_key(&lvl)) {
                Err(ReCheckError::ill(
                    "recursive name occurs as a first-class value, not applied to a \
                     structurally-smaller argument"
                        .to_string(),
                ))
            } else {
                Ok(())
            }
        }
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => Ok(()),
    }
}

/// Guard each case of a match on the structural parameter: mark every constructor
/// argument it binds as structurally smaller, then check the case body.
fn guard_match_cases(
    genv: &Context,
    cases: &[Db],
    struct_type: &str,
    fix_positions: &HashMap<usize, usize>,
    struct_level: usize,
    smaller: &HashMap<usize, String>,
    depth: usize,
) -> RResult<()> {
    // Only the constructor's VALUE FIELDS are bound by a case — its Π-telescope past the
    // inductive's parameters, which the discriminant fixes. Counting the whole telescope
    // over-peels on any parameterised inductive, and when the motive is a function type the
    // case's extra λ (an ordinary function parameter, a subterm of nothing) was marked
    // structurally smaller. `termination.rs` miscounted identically, so BOTH kernels admitted a
    // non-terminating fixpoint and `False` became inhabited by a closed term built from nothing
    // but the standard library. The two-kernel guarantee is worth nothing against a shared blind
    // spot, which is why this arithmetic is spelled out in both places.
    let num_type_params = genv.inductive_num_params(struct_type);
    let fields: Vec<Vec<String>> = genv
        .get_constructors(struct_type)
        .iter()
        .map(|(_, t)| named_field_heads(genv, t, num_type_params))
        .collect();
    for (i, case) in cases.iter().enumerate() {
        let heads: &[String] = fields.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
        let mut smaller2 = smaller.clone();
        let mut cur = case;
        let mut d = depth;
        // Peel one λ per VALUE FIELD (a parametric inductive's case binds fewer than its
        // constructor's total parameter count — type parameters are fixed), recording each
        // bound variable's level as structurally smaller together with the inductive its own
        // type heads, so a nested match on it knows which constructors to expect.
        for head in heads {
            if let Db::Lam(_, inner) = cur {
                smaller2.insert(d, head.clone());
                cur = inner;
                d += 1;
            } else {
                break;
            }
        }
        guard(
            genv,
            cur,
            fix_positions,
            struct_level,
            struct_type,
            &smaller2,
            d,
        )?;
    }
    Ok(())
}

/// Independently validate an indexed motive's complete generic telescope.
/// The de Bruijn representation enters each binder directly; only the retained
/// family application is shifted as its scope grows. No named-kernel typing or
/// telescope validation is reused.
fn check_indexed_motive(
    genv: &Context,
    lctx: &dyn LocalTypes,
    motive: &Db,
    disc_ty: &Db,
    parameters: usize,
    arguments: &[Db],
) -> RResult<()> {
    let (mut family, _) = spine(disc_ty);
    let mut arity = infer(genv, lctx, &family)?;
    for parameter in &arguments[..parameters] {
        let Db::Pi(_, body) = whnf_in_owned(genv, lctx, arity) else {
            return Err(ReCheckError::ill(
                "inductive parameter telescope is too short",
            ));
        };
        arity = beta_open_owned(body.into_db(), parameter);
        family = Db::App(DbBox::new(family), DbBox::new(parameter.clone()));
    }
    let mut motive_type = infer(genv, lctx, motive)?;
    let mut inner = extend_local(lctx);
    for _ in parameters..arguments.len() {
        let Db::Pi(domain, body) = whnf_in_owned(genv, &inner, arity) else {
            return Err(ReCheckError::ill("inductive index telescope is too short"));
        };
        let Db::Pi(actual_domain, actual_body) = whnf_in_owned(genv, &inner, motive_type) else {
            return Err(ReCheckError::ill("indexed motive lacks an index binder"));
        };
        if !def_eq(genv, &inner, &actual_domain, &domain) {
            return Err(ReCheckError::ill("indexed motive index domain mismatch"));
        }
        inner.push(domain.into_db());
        family = Db::App(DbBox::new(shift(&family, 1, 0)), DbBox::new(Db::Var(0)));
        arity = body.into_db();
        motive_type = actual_body.into_db();
    }
    if !matches!(whnf_in_owned(genv, &inner, arity), Db::Sort(_)) {
        return Err(ReCheckError::ill(
            "discriminant leaves unapplied inductive indices",
        ));
    }
    let Db::Pi(domain, codomain) = whnf_in_owned(genv, &inner, motive_type) else {
        return Err(ReCheckError::ill(
            "indexed motive lacks a major-premise binder",
        ));
    };
    if !def_eq(genv, &inner, &domain, &family) {
        return Err(ReCheckError::ill("indexed motive major domain mismatch"));
    }
    inner.push(domain.into_db());
    if !matches!(whnf_in_owned(genv, &inner, codomain.into_db()), Db::Sort(_)) {
        return Err(ReCheckError::ill("indexed motive codomain is not a sort"));
    }
    Ok(())
}

/// Type a `match`: discriminant must be inductive; the motive (function `λx:I.T` or a
/// raw type) gives the return type; case count must equal constructor count; each case
/// is checked against the type derived from its constructor's signature; and a `Prop`
/// discriminant may only large-eliminate into `Type` if it is a subsingleton.
fn infer_match(
    genv: &Context,
    lctx: &dyn LocalTypes,
    disc: &Db,
    motive: &Db,
    cases: &[Db],
) -> RResult<Db> {
    // 1. Discriminant's inductive type and its type arguments.
    let disc_ty = whnf_in_owned(genv, lctx, infer(genv, lctx, disc)?);
    let (ind_name, type_args) = extract_inductive(genv, &disc_ty).ok_or_else(|| {
        ReCheckError::unsupported(format!(
            "match discriminant of unrecognized type {}",
            from_db(&disc_ty, lctx.len())
        ))
    })?;

    // An empty `match` is `ex falso`, which requires an UNINHABITED discriminant type. A
    // zero-constructor inductive this kernel can produce a literal of is not one.
    if inhabited_without_a_constructor(genv, &ind_name) {
        return Err(ReCheckError::ill(format!(
            "'{ind_name}' has no constructors but is inhabited by a literal, so a match on it \
             is not ex falso"
        )));
    }

    // 2. Parameter/index split: the first `p` type arguments are uniform parameters; any
    // remaining ones are INDICES the motive abstracts over (an indexed family like `Eq`).
    // `indexed == false` is the ordinary eliminator, handled exactly as before.
    let p = genv.inductive_num_params(&ind_name).min(type_args.len());
    let indexed = type_args.len() > p;
    let eff_motive = if indexed {
        check_indexed_motive(genv, lctx, motive, &disc_ty, p, &type_args)?;
        None
    } else {
        Some(effective_motive(genv, lctx, motive, &disc_ty)?)
    };

    // 3. Coverage. For a NON-indexed inductive this is one case per constructor. For an INDEXED
    // one it is one case per constructor the discriminant's indices still ALLOW (SPEC F): a
    // constructor whose declared result index provably clashes with the scrutinee's could not
    // have produced it, so it owes no branch. Supplying all of them stays legal.
    let all_names: Vec<String> = genv
        .get_constructors(&ind_name)
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    let ctor_names: Vec<String> = if !indexed || cases.len() == all_names.len() {
        all_names.clone()
    } else {
        let allowed = db_constructors_the_indices_allow(genv, lctx, &ind_name, p, &type_args);
        allowed.iter().map(|&i| all_names[i].clone()).collect()
    };
    if cases.len() != ctor_names.len() {
        return Err(ReCheckError::ill(format!(
            "match on {} has {} cases but {} constructors the indices allow",
            ind_name,
            cases.len(),
            ctor_names.len()
        )));
    }

    // 4. Each case against its constructor-derived type.
    for (case, cname) in cases.iter().zip(ctor_names.iter()) {
        let case_ty = match &eff_motive {
            Some(eff) => case_type(genv, eff, cname, &type_args, &disc_ty)?,
            None => case_type_indexed(genv, motive, cname, &type_args[..p], &disc_ty)?,
        };
        check_case(genv, lctx, case, &case_ty)?;
    }

    // 5. Reuse the non-indexed motive already checked above in this exact
    // immutable environment and local scope. Standalone fixpoint return-type
    // discovery still validates its own motive through match_return_type.
    let ret = match eff_motive {
        Some(eff) => whnf_in_owned(
            genv,
            lctx,
            Db::App(DbBox::new(eff), DbBox::new(disc.clone())),
        ),
        None => match_return_type(genv, lctx, motive, disc, &disc_ty)?,
    };

    // 6. The elimination restrictions. SProp eliminates only into SProp: definitional irrelevance
    // equates every inhabitant, so a match out of one into a relevant sort equates its branches.
    // A family that is a proposition at some instance of its universe levels eliminates into a
    // larger sort only if it is a subsingleton (empty, or one constructor with purely propositional
    // arguments); for a family in `Sort u`, which is `Prop` at `u := 0`, a result whose level is not
    // at most the family's is such an elimination at that instance. The main kernel states the full
    // argument; both kernels must agree here or a `0 = 1` survives in one of them.
    let disc_sort = match whnf_in_owned(genv, lctx, infer(genv, lctx, &disc_ty)?) {
        Db::Sort(u) => u,
        other => {
            return Err(ReCheckError::ill(format!(
                "the scrutinee's type lives in {other:?}, not a sort"
            )))
        }
    };
    if disc_sort.is_never_prop() {
        return Ok(ret);
    }
    let ret_sort = match whnf_in_owned(genv, lctx, infer(genv, lctx, &ret)?) {
        Db::Sort(u) => u,
        other => {
            return Err(ReCheckError::ill(format!(
                "the match's result lives in {other:?}, not a sort"
            )))
        }
    };
    if matches!(disc_sort, Universe::SProp) && !matches!(ret_sort, Universe::SProp) {
        return Err(ReCheckError::ill(format!(
            "elimination of the definitionally-irrelevant proposition '{ind_name}' into a relevant sort"
        )));
    }
    let large = if disc_sort.equiv(&Universe::Prop) {
        !ret_sort.equiv(&Universe::Prop)
    } else {
        !ret_sort.is_subtype_of(&disc_sort)
    };
    if large && !is_subsingleton(genv, &ind_name)? {
        return Err(ReCheckError::ill(format!(
            "large elimination of non-subsingleton proposition '{}' into a larger sort",
            ind_name
        )));
    }

    Ok(ret)
}

/// Extract the inductive name and its type arguments from a (head-normal) type: peel the
/// application spine; the head must be a `Global` registered as an inductive.
fn extract_inductive(genv: &Context, ty: &Db) -> Option<(String, Vec<Db>)> {
    let (head, args) = spine(ty);
    match head {
        Db::Global(name) | Db::Const { name, .. } if genv.is_inductive(&name) => Some((name, args)),
        _ => None,
    }
}

/// Resolve the constructor at the same universe arguments as its discriminant.
/// Type inference remains this checker's own de Bruijn implementation.
fn constructor_at_levels(genv: &Context, name: &str, disc_ty: &Db) -> RResult<(Db, Db)> {
    let (head, _) = spine(disc_ty);
    let constructor = match head {
        Db::Const { levels, .. } if genv.poly_declaration(name).is_some() => Db::Const {
            name: name.to_string(),
            levels,
        },
        _ => Db::Global(name.to_string()),
    };
    let ty = infer(genv, &[], &constructor)?;
    Ok((constructor, ty))
}

/// The expected type of the case for `ctor_name`, given the `motive` and the
/// discriminant's `type_args`. The constructor's type `Π(p₀:U₀)…Π(pₘ₋₁:Uₘ₋₁). Ind …`
/// has its leading `|type_args|` parameters instantiated by the type arguments
/// (β-style), and its final codomain replaced by `motive (Ctor type_args value_args)`,
/// leaving `Π(value params). motive (Ctor …)`. Pure de Bruijn — no names to capture.
fn case_type(
    genv: &Context,
    motive: &Db,
    ctor_name: &str,
    type_args: &[Db],
    disc_ty: &Db,
) -> RResult<Db> {
    let (ctor_head, ctor_db) = constructor_at_levels(genv, ctor_name, disc_ty)?;

    // Instantiate the type parameters (each `type_args[i]` is closed over the telescope).
    let mut body = ctor_db;
    for ta in type_args {
        match body {
            Db::Pi(_dom, rest) => body = beta_open_owned(rest.into_db(), ta),
            _ => {
                return Err(ReCheckError::ill(format!(
                    "constructor '{}' has fewer parameters than the discriminant's type arguments",
                    ctor_name
                )))
            }
        }
    }

    // Peel the remaining value-parameter telescope.
    let mut value_doms = Vec::new();
    let mut cur = body;
    while let Db::Pi(dom, rest) = cur {
        value_doms.push(dom.into_db());
        cur = rest.into_db();
    }
    let num_val = value_doms.len();
    let lift = num_val as isize;

    // Build `Ctor type_args value_vars` at the bottom of the value telescope.
    let mut applied = ctor_head;
    for ta in type_args {
        applied = Db::App(DbBox::new(applied), DbBox::new(shift(ta, lift, 0)));
    }
    for p in 0..num_val {
        applied = Db::App(DbBox::new(applied), DbBox::new(Db::Var(num_val - 1 - p)));
    }

    let motive_bot = shift(motive, lift, 0);
    let cod = whnf_in_owned(
        genv,
        &[],
        Db::App(DbBox::new(motive_bot), DbBox::new(applied)),
    );

    // Re-wrap the value parameters.
    let mut case_ty = cod;
    for dom in value_doms.into_iter().rev() {
        case_ty = Db::Pi(DbBox::new(dom), DbBox::new(case_ty));
    }
    Ok(case_ty)
}

/// A match's return type, indexed-aware. For an ordinary (non-indexed) inductive it is the
/// motive applied to the discriminant. For an INDEXED family the motive abstracts over the
/// indices too, so it is applied to the discriminant's own index arguments and THEN the
/// discriminant — `Eq.rec`'s motive `P` yields `P y h` for a scrutinee `h : Eq A x y`.
fn match_return_type(
    genv: &Context,
    lctx: &dyn LocalTypes,
    motive: &Db,
    disc: &Db,
    disc_ty: &Db,
) -> RResult<Db> {
    let (ind_name, type_args) = extract_inductive(genv, disc_ty).ok_or_else(|| {
        ReCheckError::unsupported(format!(
            "match discriminant of unrecognized type {}",
            from_db(disc_ty, lctx.len())
        ))
    })?;
    let p = genv.inductive_num_params(&ind_name).min(type_args.len());
    if type_args.len() > p {
        let mut ret = motive.clone();
        for idx in &type_args[p..] {
            ret = Db::App(DbBox::new(ret), DbBox::new(idx.clone()));
        }
        ret = Db::App(DbBox::new(ret), DbBox::new(disc.clone()));
        Ok(whnf_in_owned(genv, lctx, ret))
    } else {
        let eff = effective_motive(genv, lctx, motive, disc_ty)?;
        Ok(whnf_in_owned(
            genv,
            lctx,
            Db::App(DbBox::new(eff), DbBox::new(disc.clone())),
        ))
    }
}

/// The expected case type for `ctor_name` in an INDEXED match. Only the inductive's
/// `params` are instantiated (not the trailing indices); the constructor's remaining value
/// arguments become the case's `Π` binders; and the codomain is the `motive` applied to
/// the constructor's RESULT indices (its declared return-type arguments past the
/// parameters) and then the constructor value. Pure de Bruijn — mirrors [`case_type`].
fn case_type_indexed(
    genv: &Context,
    motive: &Db,
    ctor_name: &str,
    params: &[Db],
    disc_ty: &Db,
) -> RResult<Db> {
    let (ctor_head, ctor_db) = constructor_at_levels(genv, ctor_name, disc_ty)?;

    // Instantiate the inductive's PARAMETERS only (the leading `params.len()` binders).
    let mut body = ctor_db;
    for pa in params {
        match body {
            Db::Pi(_dom, rest) => body = beta_open_owned(rest.into_db(), pa),
            _ => {
                return Err(ReCheckError::ill(format!(
                    "constructor '{}' has fewer parameters than the inductive",
                    ctor_name
                )))
            }
        }
    }

    // Peel the value-parameter telescope; the residual is the constructor's result type.
    let mut value_doms = Vec::new();
    let mut cur = body;
    while let Db::Pi(dom, rest) = cur {
        value_doms.push(dom.into_db());
        cur = rest.into_db();
    }
    let num_val = value_doms.len();
    let lift = num_val as isize;

    // The constructor's RESULT indices: its result-type spine arguments past the params
    // (already expressed under the value binders, so no shift).
    let (_head, res_args) = spine(&cur);
    let result_indices: Vec<Db> = res_args.into_iter().skip(params.len()).collect();

    // Build `Ctor params value_vars` at the bottom of the value telescope.
    let mut applied = ctor_head;
    for pa in params {
        applied = Db::App(DbBox::new(applied), DbBox::new(shift(pa, lift, 0)));
    }
    for i in 0..num_val {
        applied = Db::App(DbBox::new(applied), DbBox::new(Db::Var(num_val - 1 - i)));
    }

    // `motive result_indices… applied`, all under the value binders.
    let mut cod = shift(motive, lift, 0);
    for ri in &result_indices {
        cod = Db::App(DbBox::new(cod), DbBox::new(ri.clone()));
    }
    cod = Db::App(DbBox::new(cod), DbBox::new(applied));
    let cod = whnf_in_owned(genv, &[], cod);

    // Re-wrap the value parameters.
    let mut case_ty = cod;
    for dom in value_doms.into_iter().rev() {
        case_ty = Db::Pi(DbBox::new(dom), DbBox::new(case_ty));
    }
    Ok(case_ty)
}

/// Whether the re-checker can produce a value of `ind` without any constructor — i.e. whether
/// `ind` is one of the carriers its own `Db::Lit` typing rule inhabits.
///
/// An empty `match` passes coverage for any inductive with zero constructors, and that is only
/// `ex falso` if nothing inhabits the type. These carriers have no constructors and are inhabited
/// anyway, so an empty match on one proves its motive from a value that exists. The main kernel
/// states the same rule on `empty_match_would_be_a_lie`; both kernels need it, because counting
/// constructors is exactly the step they would otherwise get identically wrong.
fn inhabited_without_a_constructor(genv: &Context, ind: &str) -> bool {
    genv.get_constructors(ind).is_empty()
        && matches!(
            ind,
            "Int" | "Nat" | "Float" | "Text" | "Duration" | "Date" | "Moment"
        )
}

/// Decompose a `Db` into a CONSTRUCTOR head and its arguments, bridging a `Nat` literal to its
/// Peano spelling so `Vect A 0` and `Vect A Zero` are the same index to the unifier below.
fn db_constructor_head(genv: &Context, t: &Db) -> Option<(String, Vec<Db>)> {
    let bridged = if let Db::Lit(Literal::Nat(n)) = t {
        if !crate::int_inductive::reads_nat_literal(genv, n) {
            return None;
        }
        db_nat_peano_step(t)
    } else {
        db_int_constructor(genv, t).unwrap_or_else(|| t.clone())
    };
    let (head, args) = spine(&bridged);
    match head {
        Db::Global(name) | Db::Const { name, .. } if genv.is_constructor(&name) => {
            Some((name, args))
        }
        _ => None,
    }
}

/// Demand a scrutinee index head in its actual local context. Declared
/// constructor indices have a different de Bruijn scope and remain syntactic.
fn db_demanded_index_constructor_head(
    genv: &Context,
    lctx: &dyn LocalTypes,
    t: &Db,
) -> Option<(String, Vec<Db>)> {
    if WHNF_EXHAUSTED.with(|flag| flag.get()) {
        return None;
    }
    if let Some(head) = db_constructor_head(genv, t) {
        return Some(head);
    }
    let exposed = whnf_in(genv, lctx, t);
    // Keep the surrounding exhaustion flag set and refuse to infer a clash
    // from a reduction that did not finish.
    if WHNF_EXHAUSTED.with(|flag| flag.get()) {
        return None;
    }
    db_constructor_head(genv, &exposed)
}

/// Whether constructor disjointness may be used for `ctor`'s inductive.
///
/// Only at `Type` level. `Prop` is proof-irrelevant: two distinct constructors of a
/// propositional inductive ARE definitionally equal, so their difference proves nothing and a
/// clash there would discharge a branch that is in fact reachable. A family in `Sort u` is a
/// proposition at `u := 0`, so the arity must end in a sort no instance of its levels makes
/// `Prop`; an arity ending in anything but a written sort grants nothing.
fn db_disjointness_available(genv: &Context, ctor: &str) -> bool {
    let Some(ind) = genv.constructor_inductive(ctor) else {
        return false;
    };
    let Some(ind_ty) = genv.get_global(ind) else {
        return false;
    };
    let Ok(mut cur) = to_db(ind_ty, &mut Vec::new()) else {
        return false;
    };
    while let Db::Pi(_, rest) = cur {
        cur = rest.into_db();
    }
    matches!(cur, Db::Sort(level) if level.is_never_prop())
}

/// Whether two index terms PROVABLY cannot be made equal. `flexible` holds the de Bruijn
/// indices — as seen from the constructor's RESULT — of the binders still free to be anything.
///
/// The only ground ever given is constructor disjointness. The two sides live in different
/// scopes, which is harmless precisely because nothing is ever concluded from a VARIABLE: a
/// variable on either side means "could be anything", and that is the safe answer.
fn db_indices_clash(
    genv: &Context,
    lctx: &dyn LocalTypes,
    scrutinee: &Db,
    declared: &Db,
    flexible: &[usize],
) -> bool {
    if let Db::Var(k) = declared {
        if flexible.contains(k) {
            return false;
        }
    }
    // The declared side stays in its own scope. If it has no constructor head,
    // there is no disjointness question to demand from the scrutinee.
    let Some((cd, args_d)) = db_constructor_head(genv, declared) else {
        return false;
    };
    let Some((cs, args_s)) = db_demanded_index_constructor_head(genv, lctx, scrutinee) else {
        return false;
    };
    if !db_disjointness_available(genv, &cs) || !db_disjointness_available(genv, &cd) {
        return false;
    }
    if cs != cd {
        return genv.constructor_inductive(&cs) == genv.constructor_inductive(&cd);
    }
    args_s
        .iter()
        .zip(args_d.iter())
        .any(|(a, b)| db_indices_clash(genv, lctx, a, b, flexible))
}

/// **SPEC F** for the re-checker — which of `ind`'s constructors the discriminant's INDICES
/// still allow, as positions in registration order. The re-checker's own reading of the rule
/// the main kernel states on `constructors_the_indices_allow`: a `Match` on an indexed family
/// owes a branch only for the constructors that could have produced the scrutinee.
///
/// De Bruijn makes "still free to be anything" a set of INDICES rather than a set of names, so
/// this side cannot be fooled by a shadowed binder — a different arithmetic for the same rule,
/// which is the point of having two kernels.
fn db_constructors_the_indices_allow(
    genv: &Context,
    lctx: &dyn LocalTypes,
    ind: &str,
    num_params: usize,
    disc_args: &[Db],
) -> Vec<usize> {
    let (params, disc_indices) = disc_args.split_at(num_params.min(disc_args.len()));
    let ctors: Vec<(String, Term)> = genv
        .get_constructors(ind)
        .iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect();
    (0..ctors.len())
        .filter(|&i| {
            let Ok(mut cur) = to_db(&ctors[i].1, &mut Vec::new()) else {
                return true;
            };
            // The constructor's parameters are the scrutinee's own.
            for value in params {
                match cur {
                    Db::Pi(_, rest) => cur = beta_open(&rest.into_db(), value),
                    _ => return true,
                }
            }
            // What is left binds the fields, each free to be anything: from the result, the
            // variables below the field count.
            let mut fields = 0usize;
            while let Db::Pi(_, rest) = cur {
                cur = rest.into_db();
                fields += 1;
            }
            let flexible: Vec<usize> = (0..fields).collect();
            let (_head, result_args) = spine(&cur);
            let declared: &[Db] = if result_args.len() > num_params {
                &result_args[num_params..]
            } else {
                &[]
            };
            !disc_indices
                .iter()
                .zip(declared.iter())
                .any(|(s, d)| db_indices_clash(genv, lctx, s, d, &flexible))
        })
        .collect()
}

/// Whether an inductive is a subsingleton `Prop` — the re-checker's independent reading of the
/// same CIC criterion the main kernel states on [`crate::type_checker::is_subsingleton_prop`]:
/// zero constructors, or exactly one whose every binder past the inductive's **parameters** is
/// propositional **or occurs among the indices of the constructor's result**. Only these may be
/// large-eliminated into `Type`.
///
/// De Bruijn makes the index clause exact where names cannot: a field bound at telescope
/// position `p` is, from under all `depth` binders of the result, the variable `depth-1-p`.
/// There is no shadowing to reason about — two distinct binders are two distinct indices — so
/// this side needs no conservative repeat-name refusal, and the two kernels reaching the same
/// verdict by different arithmetic is the point of having two.
fn is_subsingleton(genv: &Context, ind: &str) -> RResult<bool> {
    let ctors: Vec<(String, Term)> = genv
        .get_constructors(ind)
        .iter()
        .map(|(n, t)| (n.to_string(), (*t).clone()))
        .collect();
    match ctors.len() {
        0 => Ok(true),
        1 => {
            let num_params = genv.inductive_num_params(ind);
            let ctor_db = to_db(&ctors[0].1, &mut Vec::new())?;
            let mut lctx: Vec<Db> = Vec::new();
            let mut cur = ctor_db;
            let mut depth = 0usize;
            // Telescope positions of the fields that are not propositional.
            let mut owed: Vec<usize> = Vec::new();
            while let Db::Pi(dom, rest) = cur {
                if depth >= num_params && !infer_sort(genv, &lctx, &dom)?.equiv(&Universe::Prop) {
                    owed.push(depth);
                }
                lctx.push(dom.into_db());
                cur = rest.into_db();
                depth += 1;
            }
            if owed.is_empty() {
                return Ok(true);
            }
            let (_head, args) = spine(&cur);
            let indices: &[Db] = if args.len() > num_params {
                &args[num_params..]
            } else {
                &[]
            };
            Ok(owed.iter().all(|&position| {
                let as_seen_from_the_result = depth - 1 - position;
                indices
                    .iter()
                    .any(|a| matches!(a, Db::Var(k) if *k == as_seen_from_the_result))
            }))
        }
        _ => Ok(false),
    }
}

/// Infer `t`'s type and require it to be a sort; return that universe.
fn infer_sort(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> RResult<Universe> {
    let ty = whnf_in_owned(genv, lctx, infer(genv, lctx, t)?);
    match ty {
        Db::Sort(u) => Ok(u),
        other => Err(ReCheckError::ill(format!(
            "expected a type, got something of type {}",
            from_db(&other, lctx.len())
        ))),
    }
}

/// Check a match branch against its constructor-derived telescope. Each explicit
/// binder annotation must be a type and definitionally equal to the corresponding
/// field domain before the branch body is checked in that extended context.
fn check_case(genv: &Context, lctx: &dyn LocalTypes, case: &Db, case_ty: &Db) -> RResult<()> {
    match (case, case_ty) {
        (Db::Lam(annotation, body), Db::Pi(dom, cod)) => {
            if !matches!(
                whnf_in_owned(genv, lctx, infer(genv, lctx, annotation)?),
                Db::Sort(_)
            ) || !def_eq(genv, lctx, annotation, dom)
            {
                return Err(ReCheckError::ill(
                    "match branch binder annotation differs from its constructor field type"
                        .to_string(),
                ));
            }
            let mut ext = extend_local(lctx);
            ext.push((**dom).clone());
            check_case(genv, &ext, body, cod)
        }
        _ => check(genv, lctx, case, case_ty),
    }
}

fn check(genv: &Context, lctx: &dyn LocalTypes, t: &Db, expected: &Db) -> RResult<()> {
    // A fixpoint checked against a type: bound at that type, a terminating fixpoint's body must
    // have it, as the main kernel's `check_type` asks. A fixpoint this does not type falls to
    // inference, which reads the type off the body.
    if let Db::Fix(body) = t {
        if check_terminates(genv, body, lctx.len()).is_ok() {
            let mut ext = extend_local(lctx);
            ext.push(expected.clone());
            if let Ok(got) = infer(genv, &ext, body) {
                if is_sub(genv, &ext, &got, &shift(expected, 1, 0)) {
                    return Ok(());
                }
            }
        }
    }
    let inferred = infer(genv, lctx, t)?;
    if is_sub(genv, lctx, &inferred, expected) {
        Ok(())
    } else {
        Err(ReCheckError::ill(format!(
            "type mismatch: have {}, expected {}",
            from_db(&whnf_in(genv, lctx, &inferred), lctx.len()),
            from_db(&whnf_in(genv, lctx, expected), lctx.len())
        )))
    }
}

fn lit_type_name(l: &Literal) -> &'static str {
    match l {
        Literal::Int(_) | Literal::BigInt(_) => "Int",
        Literal::Nat(_) => "Nat",
        Literal::Float(_) => "Float",
        Literal::Text(_) => "Text",
        Literal::Duration(_) => "Duration",
        Literal::Date(_) => "Date",
        Literal::Moment(_) => "Moment",
    }
}

// An active Context is a name map, not a historical telescope. Recover only the
// demanded dependency closure, then independently check its predecessor-scoped types.
struct LocalPlan<'a> {
    bindings: Vec<(&'a str, &'a Term)>,
    issue: Option<ReCheckError>,
    max_depth: usize,
}

struct LocalAdmission {
    remaining: usize,
    max_depth: usize,
    #[cfg(test)]
    high_water: usize,
}

enum LocalAdmissionFrame<'a> {
    Node(&'a Term, usize),
    Db(&'a Db, usize),
    Dbs(&'a [Db], usize),
    Level(&'a Universe, usize),
    Levels(&'a [Universe], usize),
    Bind(&'a str, &'a Term, usize),
    Restore(usize),
    Cases(&'a [Term], usize),
    Bodies(&'a [(String, Term)], usize),
}

impl LocalAdmission {
    fn new(limit: usize) -> Self {
        Self {
            remaining: limit,
            max_depth: 0,
            #[cfg(test)]
            high_water: 0,
        }
    }

    // Slice continuations keep scratch bounded by depth, not the arity of a
    // malicious Match. Binder names are borrowed; no annotation trees are cloned.
    fn free_names<'a>(&mut self, term: &'a Term) -> RResult<Vec<&'a str>> {
        self.scan(LocalAdmissionFrame::Node(term, 0))
    }

    fn admit_db(&mut self, term: &Db) -> RResult<()> {
        self.scan(LocalAdmissionFrame::Db(term, 0)).map(|_| ())
    }

    fn scan<'a>(&mut self, root: LocalAdmissionFrame<'a>) -> RResult<Vec<&'a str>> {
        use LocalAdmissionFrame as Frame;
        let mut frames = vec![root];
        let mut bound: Vec<&str> = Vec::new();
        let mut free = std::collections::BTreeSet::new();
        while let Some(frame) = frames.pop() {
            #[cfg(test)]
            {
                self.high_water = self.high_water.max(frames.len() + 1);
            }
            match frame {
                Frame::Restore(len) => bound.truncate(len),
                Frame::Bind(name, body, depth) => {
                    frames.push(Frame::Restore(bound.len()));
                    bound.push(name);
                    frames.push(Frame::Node(body, depth));
                }
                Frame::Cases(cases, depth) => {
                    if let Some((first, rest)) = cases.split_first() {
                        frames.push(Frame::Cases(rest, depth));
                        frames.push(Frame::Node(first, depth));
                    }
                }
                Frame::Bodies(defs, depth) => {
                    if let Some(((_, first), rest)) = defs.split_first() {
                        frames.push(Frame::Bodies(rest, depth));
                        frames.push(Frame::Node(first, depth));
                    }
                }
                Frame::Levels(levels, depth) => {
                    if let Some((first, rest)) = levels.split_first() {
                        frames.push(Frame::Levels(rest, depth));
                        frames.push(Frame::Level(first, depth));
                    }
                }
                Frame::Level(level, depth) => {
                    if self.remaining == 0 || depth > crate::type_checker::MAX_TERM_DEPTH {
                        return Err(ReCheckError::resource_limit(
                            "active local universe exceeds the independent input admission limit",
                        ));
                    }
                    self.remaining -= 1;
                    self.max_depth = self.max_depth.max(depth);
                    match level {
                        Universe::Succ(a) => frames.push(Frame::Level(a, depth + 1)),
                        Universe::Max(a, b) | Universe::IMax(a, b) => {
                            frames.push(Frame::Level(b, depth + 1));
                            frames.push(Frame::Level(a, depth + 1));
                        }
                        Universe::SProp | Universe::Prop | Universe::Type(_) | Universe::Var(_) => {
                        }
                    }
                }
                Frame::Dbs(terms, depth) => {
                    if let Some((first, rest)) = terms.split_first() {
                        frames.push(Frame::Dbs(rest, depth));
                        frames.push(Frame::Db(first, depth));
                    }
                }
                Frame::Db(t, depth) => {
                    if self.remaining == 0 || depth > crate::type_checker::MAX_TERM_DEPTH {
                        return Err(ReCheckError::resource_limit(
                            "inferred local type exceeds input admission limit",
                        ));
                    }
                    self.remaining -= 1;
                    self.max_depth = self.max_depth.max(depth);
                    match t {
                        Db::Sort(level) => frames.push(Frame::Level(level, depth)),
                        Db::Const { levels, .. } => frames.push(Frame::Levels(levels, depth)),
                        Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
                            frames.push(Frame::Db(b, depth + 1));
                            frames.push(Frame::Db(a, depth + 1));
                        }
                        Db::Fix(body) => frames.push(Frame::Db(body, depth + 1)),
                        Db::Let(a, b, c) => {
                            frames.push(Frame::Db(c, depth + 1));
                            frames.push(Frame::Db(b, depth + 1));
                            frames.push(Frame::Db(a, depth + 1));
                        }
                        Db::MutualFix { defs, .. } => frames.push(Frame::Dbs(defs, depth + 1)),
                        Db::Match {
                            disc,
                            motive,
                            cases,
                        } => {
                            frames.push(Frame::Dbs(cases, depth + 1));
                            frames.push(Frame::Db(motive, depth + 1));
                            frames.push(Frame::Db(disc, depth + 1));
                        }
                        Db::Var(_) | Db::Global(_) | Db::Lit(_) => {}
                    }
                }
                Frame::Node(t, depth) => {
                    if self.remaining == 0 || depth > crate::type_checker::MAX_TERM_DEPTH {
                        return Err(ReCheckError::resource_limit(
                            "active local telescope exceeds the independent input admission limit",
                        ));
                    }
                    self.remaining -= 1;
                    self.max_depth = self.max_depth.max(depth);
                    match t {
                        Term::Var(name) => {
                            if !bound.iter().rev().any(|b| *b == name) {
                                free.insert(name.as_str());
                            }
                        }
                        Term::Pi {
                            param,
                            param_type,
                            body_type,
                        } => {
                            frames.push(Frame::Bind(param, body_type, depth + 1));
                            frames.push(Frame::Node(param_type, depth + 1));
                        }
                        Term::Lambda {
                            param,
                            param_type,
                            body,
                        } => {
                            frames.push(Frame::Bind(param, body, depth + 1));
                            frames.push(Frame::Node(param_type, depth + 1));
                        }
                        Term::App(f, a) => {
                            frames.push(Frame::Node(a, depth + 1));
                            frames.push(Frame::Node(f, depth + 1));
                        }
                        Term::Let {
                            name,
                            ty,
                            value,
                            body,
                        } => {
                            frames.push(Frame::Bind(name, body, depth + 1));
                            frames.push(Frame::Node(value, depth + 1));
                            frames.push(Frame::Node(ty, depth + 1));
                        }
                        Term::Fix { name, body } => frames.push(Frame::Bind(name, body, depth + 1)),
                        Term::MutualFix { defs, .. } => {
                            // Each body requires at least one admitted node. Check before
                            // retaining the names, not after allocating for arbitrary arity.
                            if bound.len().saturating_add(defs.len()) > self.remaining {
                                return Err(ReCheckError::resource_limit(
                                    "active local mutual block exceeds input admission limit",
                                ));
                            }
                            frames.push(Frame::Restore(bound.len()));
                            bound.extend(defs.iter().map(|(name, _)| name.as_str()));
                            frames.push(Frame::Bodies(defs, depth + 1));
                        }
                        Term::Match {
                            discriminant,
                            motive,
                            cases,
                        } => {
                            frames.push(Frame::Cases(cases, depth + 1));
                            frames.push(Frame::Node(motive, depth + 1));
                            frames.push(Frame::Node(discriminant, depth + 1));
                        }
                        // Universe leaves occupy their Term's existing depth; only
                        // nested level edges add depth. This preserves the ordinary
                        // zero-based Term depth boundary while sizing recursive level clones.
                        Term::Sort(level) => frames.push(Frame::Level(level, depth)),
                        Term::Const { levels, .. } => frames.push(Frame::Levels(levels, depth)),
                        Term::Global(_) | Term::Lit(_) | Term::Hole => {}
                        Term::Annotated { term, .. } => frames.push(Frame::Node(term, depth + 1)),
                    }
                }
            }
        }
        Ok(free.into_iter().collect())
    }
}

impl<'a> LocalPlan<'a> {
    fn prepare(ctx: &'a Context, term: &'a Term, limit: usize) -> RResult<Self> {
        struct Pending<'a> {
            binding: Option<(&'a str, &'a Term)>,
            dependencies: std::vec::IntoIter<&'a str>,
        }
        let mut admission = LocalAdmission::new(limit);
        let roots = admission.free_names(term)?;
        let mut pending = vec![Pending {
            binding: None,
            dependencies: roots.into_iter(),
        }];
        let mut state = HashMap::<&str, bool>::new(); // false = gray, true = complete
        let mut bindings = Vec::new();
        let mut issue = None;
        while let Some(frame) = pending.last_mut() {
            if let Some(name) = frame.dependencies.next() {
                match state.get(name) {
                    Some(true) => continue,
                    Some(false) => {
                        issue.get_or_insert_with(|| {
                            ReCheckError::ill(format!(
                                "cyclic active local type dependency at '{name}'"
                            ))
                        });
                        continue;
                    }
                    None => {}
                }
                let Some(ty) = ctx.get(name) else {
                    issue.get_or_insert_with(|| {
                        ReCheckError::ill(
                            crate::type_checker::unbound_diagnosis(name)
                                .unwrap_or_else(|| format!("unbound local variable '{name}'")),
                        )
                    });
                    state.insert(name, true);
                    continue;
                };
                let dependencies = admission.free_names(ty)?;
                state.insert(name, false);
                pending.push(Pending {
                    binding: Some((name, ty)),
                    dependencies: dependencies.into_iter(),
                });
            } else {
                let finished = pending.pop().expect("pending frame");
                if let Some((name, ty)) = finished.binding {
                    state.insert(name, true);
                    bindings.push((name, ty));
                }
            }
        }
        // Finish admission even after a missing/cyclic dependency: double_check
        // may need the main verdict, whose clones must also fit the admitted stack.
        Ok(Self {
            bindings,
            issue,
            max_depth: admission.max_depth,
        })
    }

    fn check(&self, ctx: &Context) -> RResult<(Vec<String>, Vec<Db>)> {
        if let Some(issue) = &self.issue {
            return Err(issue.clone());
        }
        let mut names = Vec::with_capacity(self.bindings.len());
        let mut lctx = Vec::with_capacity(self.bindings.len());
        for (name, ty) in &self.bindings {
            let lowered = to_db(ty, &mut names)?;
            infer_sort(ctx, &lctx, &lowered)?;
            // Stored in precisely the predecessor scope expected by var_type.
            lctx.push(lowered);
            names.push((*name).to_string());
        }
        Ok((names, lctx))
    }
}

thread_local! {
    static LOCAL_CONTEXT_BUDGET_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// A read-only ownership predicate also usable by optional metadata admission:
// logical depth zero does not mean the scoped telescope allowance is inactive.
fn recheck_budget_active() -> bool {
    WHNF_DEPTH.with(|d| d.get() != 0) || LOCAL_CONTEXT_BUDGET_DEPTH.with(|d| d.get() != 0)
}

#[cfg(test)]
thread_local! {
    static LOCAL_BUDGET_ENTRIES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

struct LocalContextBudget;
impl LocalContextBudget {
    fn enter() -> Self {
        #[cfg(test)]
        LOCAL_BUDGET_ENTRIES.with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
        if !recheck_budget_active() {
            WHNF_FUEL.with(|f| f.set(MAX_RECHECK_STEPS));
            begin_check();
        }
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|d| d.set(d.get() + 1));
        Self
    }
}
impl Drop for LocalContextBudget {
    fn drop(&mut self) {
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|d| d.set(d.get() - 1));
    }
}

fn local_outcome<T>(outcome: RResult<T>) -> RResult<T> {
    match outcome {
        Ok(value) => Ok(value),
        Err(e) if WHNF_EXHAUSTED.with(|e| e.get()) => Err(cut_short(&e, "active-local reduction")),
        Err(e) => Err(e),
    }
}

// Read the existing entitlement; never migrate an owned TLS budget to a new
// thread, increase a ceiling, or recursively clone an output to size its stack.
fn local_stack_admitted(depth: usize) -> RResult<()> {
    if depth <= crate::type_checker::MAX_TERM_DEPTH
        && (depth <= crate::term_store::INLINE_DEPTH
            || crate::term_store::stack_bytes_for_depth(depth)
                <= crate::term_store::kernel_stack_bytes())
    {
        Ok(())
    } else {
        Err(ReCheckError::resource_limit(
            "open inferred type exceeds the current kernel stack entitlement",
        ))
    }
}

fn recheck_open(ctx: &Context, term: &Term, plan: &LocalPlan<'_>) -> RResult<Term> {
    if !crate::type_checker::inside_kernel() {
        if let Some(r) = crate::term_store::deepen_stack_with_depth(term, plan.max_depth, || {
            recheck_open(ctx, term, plan)
        }) {
            return r;
        }
    }
    local_stack_admitted(plan.max_depth)?;
    let _prefix_scope = PrefixScope::enter(ctx);
    let _budget = LocalContextBudget::enter();
    fix_factory_begin();
    local_outcome((|| {
        let (mut names, lctx) = plan.check(ctx)?;
        let db = to_db(term, &mut names)?;
        let ty = infer(ctx, &lctx, &db)?;
        let mut output = LocalAdmission::new(MAX_RECHECK_TERM_NODES);
        output.admit_db(&ty)?;
        local_stack_admitted(output.max_depth)?;
        Ok(from_db_with_external(&ty, names.len(), &names))
    })())
}

fn double_check_open(ctx: &Context, term: &Term, plan: &LocalPlan<'_>) -> DoubleCheck {
    if !crate::type_checker::inside_kernel() {
        if let Some(r) = crate::term_store::deepen_stack_with_depth(term, plan.max_depth, || {
            double_check_open(ctx, term, plan)
        }) {
            return r;
        }
    }
    if let Err(ReCheckError::Unsupported(why)) = local_stack_admitted(plan.max_depth) {
        return DoubleCheck::MainOnlyReCheckerIncomplete(why);
    }
    let _prefix_scope = PrefixScope::enter(ctx);
    let _budget = LocalContextBudget::enter();
    fix_factory_begin();
    let independent = local_outcome((|| {
        let (mut names, lctx) = plan.check(ctx)?;
        let db = to_db(term, &mut names)?;
        let ty = infer(ctx, &lctx, &db)?;
        let mut output = LocalAdmission::new(MAX_RECHECK_TERM_NODES);
        output.admit_db(&ty)?;
        local_stack_admitted(output.max_depth)?;
        Ok((names, lctx, ty))
    })());
    // Unsupported is no verdict whatever main would decide. Avoid asking main
    // to clone an unsupported deep/ill-formed assumption merely for diagnostics.
    if let Err(ReCheckError::Unsupported(why)) = &independent {
        return DoubleCheck::MainOnlyReCheckerIncomplete(why.clone());
    }
    let main = crate::infer_type(ctx, term);
    match (main, independent) {
        (Ok(main_ty), Ok((mut names, lctx, re_ty))) => {
            // Main output is not an assumption. Admit it before lowering, and
            // reject additional unaccounted names rather than enlarging lctx.
            let mut admission = LocalAdmission::new(MAX_RECHECK_TERM_NODES);
            if let Err(e) = admission.free_names(&main_ty) {
                let why = match e.incomplete() {
                    Some(incomplete) => incomplete.prefixed("main inferred type cannot be lowered"),
                    None => Incomplete::unsupported(format!(
                        "main inferred type cannot be lowered: {e:?}"
                    )),
                };
                return DoubleCheck::MainOnlyReCheckerIncomplete(why);
            }
            if let Err(ReCheckError::Unsupported(why)) = local_stack_admitted(admission.max_depth) {
                return DoubleCheck::MainOnlyReCheckerIncomplete(why);
            }
            match to_db(&main_ty, &mut names) {
                Ok(m) if def_eq(ctx, &lctx, &m, &re_ty) => DoubleCheck::Agreed,
                Ok(_) if WHNF_EXHAUSTED.with(|e| e.get()) => {
                    DoubleCheck::MainOnlyReCheckerIncomplete(stopped(
                        "comparing inferred open types was cut short",
                    ))
                }
                Ok(_) => DoubleCheck::Disagree(format!(
                    "kernels inferred different types: main={}, recheck={}",
                    main_ty,
                    from_db_with_external(&re_ty, names.len(), &names)
                )),
                Err(_) => DoubleCheck::MainOnlyReCheckerIncomplete(Incomplete::unsupported(
                    "inferred type outside the checked active local telescope",
                )),
            }
        }
        (Ok(_), Err(ReCheckError::Ill(why))) => DoubleCheck::Disagree(format!(
            "main kernel accepted but re-checker rejected: {why}"
        )),
        (Err(e), Ok(_)) => DoubleCheck::Disagree(format!(
            "re-checker accepted but main kernel rejected: {e:?}"
        )),
        (Err(_), Err(ReCheckError::Ill(_))) => DoubleCheck::Agreed,
        (_, Err(ReCheckError::Unsupported(why))) => DoubleCheck::MainOnlyReCheckerIncomplete(why),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Independently re-check `term` in `ctx`, returning its inferred type on success.
///
/// A SEPARATE implementation from [`infer_type`](crate::infer_type): a de Bruijn core
/// (now including the `Match` eliminator) with its own reduction and cumulative
/// conversion, so a bug in one checker is unlikely to be shared by the other. Returns
/// [`ReCheckError::Unsupported`] for the fragment it does not yet cover (`Hole`,
/// or an unrecognized inductive), or when resource admission refuses a demand.
/// Active local assumptions are dependency-ordered and independently sort-checked;
/// unsupported demands never become a false pass.
pub fn recheck(ctx: &Context, term: &Term) -> RResult<Term> {
    if let Some(r) = crate::annotation::erased(term, |term| recheck(ctx, term)) {
        return r;
    }
    if !ctx.has_local_bindings() {
        return recheck_closed(ctx, term);
    }
    let plan = LocalPlan::prepare(ctx, term, MAX_RECHECK_TERM_NODES)?;
    if plan.bindings.is_empty() && plan.issue.is_none() {
        return recheck_closed(ctx, term);
    }
    recheck_open(ctx, term, &plan)
}

fn recheck_closed(ctx: &Context, term: &Term) -> RResult<Term> {
    // ONE STACK DISCIPLINE, NOT TWO. The main kernel sizes its stack from the term's own depth
    // at its outermost entry; this one used to meter against an absolute byte budget unrelated
    // to the stack it actually had. Two kernels that run out of room in different places do not
    // corroborate each other across the gap — `double_check` reports `MainOnlyReCheckerIncomplete`
    // on an ordinary telescope. Sizing here, through the same door, is what makes the band one
    // band. `deepen_stack` returns `None` on the re-entrant call inside the closure, which falls
    // through to the body below, so the measurement is paid exactly once.
    if !crate::type_checker::inside_kernel() {
        if let Some(r) = crate::term_store::deepen_stack(term, || recheck_closed(ctx, term)) {
            return r;
        }
    }
    // The re-checker descends untrusted input exactly as the main kernel does, so it refuses
    // at the same door — the two kernels run out of room on the same terms rather than one
    // reporting while the other aborts the process.
    if crate::type_checker::depth_exceeds(term, crate::type_checker::MAX_TERM_DEPTH) {
        return Err(ReCheckError::resource_limit(format!(
            "term nests deeper than the kernel's limit of {}",
            crate::type_checker::MAX_TERM_DEPTH
        )));
    }
    let _prefix_scope = PrefixScope::enter(ctx);
    fix_factory_begin(); // sole factory creation for this public check
    begin_check();
    let outcome = to_db(term, &mut Vec::new())
        .and_then(|db| infer(ctx, &[], &db))
        .map(|ty| from_db(&ty, 0));
    match outcome {
        // An ACCEPTANCE stands, even if a reduction along the way was cut short. Cutting a
        // reduction short can only LOSE an equality, never invent one: if `a ⟶* a'` and
        // `b ⟶* b'` and the two partial reducts match, `a` and `b` have a common reduct, which
        // IS definitional equality. A conversion that succeeded, succeeded.
        Ok(ty) => Ok(ty),
        // A REJECTION after an abandoned reduction is evidence of nothing: the two sides may
        // differ only in how far they got. Reported as an incompleteness, which makes the term
        // honestly single-checked instead of manufacturing a soundness alarm.
        Err(e) if take_budget_exhausted() => Err(cut_short(&e, "reduction")),
        Err(e) => Err(e),
    }
}

/// The verdict of cross-checking a term with BOTH kernels.
#[derive(Debug, Clone, PartialEq)]
pub enum DoubleCheck {
    /// Both kernels accepted with definitionally-equal types, or both rejected as ill-typed.
    /// Check the inference result as well when agreement must establish acceptance.
    Agreed,
    /// The independent checker reached no verdict, for example because a construct
    /// or resource requirement is unsupported. This does not establish acceptance
    /// or rejection by the main kernel. Open-input admission can stop before main
    /// inference runs; callers needing a main verdict must check it separately.
    /// An incomplete independent check never establishes two-kernel agreement.
    MainOnlyReCheckerIncomplete(Incomplete),
    /// The two kernels DISAGREE: one accepted and the other rejected, or they inferred
    /// non-equal types. A soundness alarm that must never fire on a valid proof.
    Disagree(String),
}

/// Cross-check `term` against both [`infer_type`](crate::infer_type) and `recheck`.
///
/// The de Bruijn criterion in action: a proof term is most trustworthy when two
/// independently-written kernels, on different representations, agree on its type.
/// Disagreement is surfaced loudly; an incomplete re-check (for example an unresolved `Hole`) is
/// surfaced honestly rather than dressed up as agreement.
pub fn double_check(ctx: &Context, term: &Term) -> DoubleCheck {
    if let Some(r) = crate::annotation::erased(term, |term| double_check(ctx, term)) {
        return r;
    }
    if !ctx.has_local_bindings() {
        return double_check_closed(ctx, term);
    }
    let plan = match LocalPlan::prepare(ctx, term, MAX_RECHECK_TERM_NODES) {
        Ok(plan) => plan,
        Err(ReCheckError::Unsupported(why)) => {
            return DoubleCheck::MainOnlyReCheckerIncomplete(why)
        }
        Err(ReCheckError::Ill(why)) => {
            return DoubleCheck::MainOnlyReCheckerIncomplete(Incomplete::unsupported(format!(
                "active local admission did not produce a checked telescope: {why}"
            )))
        }
    };
    if plan.bindings.is_empty() && plan.issue.is_none() {
        return double_check_closed(ctx, term);
    }
    double_check_open(ctx, term, &plan)
}

fn double_check_closed(ctx: &Context, term: &Term) -> DoubleCheck {
    // Sized ONCE here, so both kernels run on the same stack rather than each arranging its own
    // and the comparison of their two answers running on whatever is left of the caller's.
    if !crate::type_checker::inside_kernel() {
        if let Some(r) = crate::term_store::deepen_stack(term, || double_check_closed(ctx, term)) {
            return r;
        }
    }
    let main = crate::infer_type(ctx, term);
    let recheck_result = recheck_closed(ctx, term);
    match (main, recheck_result) {
        (Ok(main_ty), Ok(re_ty)) => {
            take_budget_exhausted();
            match (to_db(&main_ty, &mut Vec::new()), to_db(&re_ty, &mut Vec::new())) {
                (Ok(m), Ok(r)) if def_eq(ctx, &[], &m, &r) => DoubleCheck::Agreed,
                // A comparison whose reduction was CUT SHORT decided nothing, so the two types
                // being "different" is not evidence the kernels disagree.
                (Ok(_), Ok(_)) if take_budget_exhausted() => {
                    DoubleCheck::MainOnlyReCheckerIncomplete(stopped(format!(
                        "comparing the two inferred types was cut short (main={main_ty}, recheck={re_ty})",
                    )))
                }
                (Ok(_), Ok(_)) => DoubleCheck::Disagree(format!(
                    "kernels inferred different types: main={}, recheck={}",
                    main_ty, re_ty
                )),
                _ => DoubleCheck::MainOnlyReCheckerIncomplete(Incomplete::unsupported(
                    "inferred type outside re-checker fragment",
                )),
            }
        }
        (Ok(_), Err(ReCheckError::Unsupported(why))) => {
            DoubleCheck::MainOnlyReCheckerIncomplete(why)
        }
        (Ok(_), Err(ReCheckError::Ill(why))) => DoubleCheck::Disagree(format!(
            "main kernel accepted but re-checker rejected: {}",
            why
        )),
        (Err(e), Ok(_)) => DoubleCheck::Disagree(format!(
            "re-checker accepted but main kernel rejected: {:?}",
            e
        )),
        // Both kernels examined the term and both rejected it: that IS agreement, and the
        // strongest rejection the API can report.
        (Err(_), Err(ReCheckError::Ill(_))) => DoubleCheck::Agreed,
        // The main kernel rejected and the re-checker DECLINED TO LOOK. A checker that never
        // examined the term has not concurred with anything, so this is single-checked and
        // flagged — not agreement. Collapsing it into `Agreed` is how a `Hole`, which the
        // re-checker refuses to parse at all, could be reported as corroborated by both.
        (Err(_), Err(ReCheckError::Unsupported(why))) => {
            DoubleCheck::MainOnlyReCheckerIncomplete(why)
        }
    }
}

// Independent Int representation bridge. Consult shared declaration metadata, but
// check its types and construct/read de Bruijn values here, without the main reducer.
fn db_canonical_int_environment(genv: &Context) -> bool {
    let global = |name: &str| Db::Global(name.to_string());
    for family in ["Nat", "Int"] {
        if !genv.is_inductive(family)
            || genv.inductive_num_params(family) != 0
            || genv.poly_declaration(family).is_some()
            || genv.get_definition_body(family).is_some()
        {
            return false;
        }
        let Some(ty) = genv.get_global(family) else {
            return false;
        };
        if to_db(ty, &mut Vec::new()).ok() != Some(Db::Sort(Universe::Type(0))) {
            return false;
        }
    }
    for (family, names) in [
        ("Nat", ["Zero", "Succ"]),
        ("Int", ["Int_ofNat", "Int_negSucc"]),
    ] {
        let constructors = genv.get_constructors(family);
        if constructors.len() != 2 {
            return false;
        }
        for ((name, ty), expected) in constructors.iter().zip(names) {
            if *name != expected
                || genv.constructor_inductive(expected) != Some(family)
                || genv.poly_declaration(expected).is_some()
                || genv.get_definition_body(expected).is_some()
            {
                return false;
            }
            let expected_type = if expected == "Zero" {
                global("Nat")
            } else {
                Db::Pi(DbBox::new(global("Nat")), DbBox::new(global(family)))
            };
            let Some(current) = genv.get_global(expected) else {
                return false;
            };
            if to_db(ty, &mut Vec::new()).ok() != Some(expected_type.clone())
                || to_db(current, &mut Vec::new()).ok() != Some(expected_type)
            {
                return false;
            }
        }
    }
    true
}

fn db_int_constructor(genv: &Context, t: &Db) -> Option<Db> {
    let integer = match t {
        Db::Lit(Literal::Int(n)) => logicaffeine_base::BigInt::from_i64(*n),
        Db::Lit(Literal::BigInt(n)) => n.clone(),
        _ => return None,
    };
    if !db_canonical_int_environment(genv) {
        return None;
    }
    let (name, field) = if integer.is_negative() {
        (
            "Int_negSucc",
            integer.abs().sub(&logicaffeine_base::BigInt::from_i64(1)),
        )
    } else {
        ("Int_ofNat", integer)
    };
    Some(Db::App(
        DbBox::new(Db::Global(name.to_string())),
        DbBox::new(Db::Lit(Literal::Nat(field))),
    ))
}

fn db_int_literal(genv: &Context, t: &Db) -> Option<Literal> {
    let Db::App(head, field) = t else { return None };
    let negative = match head.as_ref() {
        Db::Global(n) if n == "Int_ofNat" => false,
        Db::Global(n) if n == "Int_negSucc" => true,
        _ => return None,
    };
    if !db_canonical_int_environment(genv) {
        return None;
    }
    let one = logicaffeine_base::BigInt::from_i64(1);
    let mut count = logicaffeine_base::BigInt::from_i64(0);
    let mut field = whnf(genv, field);
    loop {
        match field {
            Db::Lit(Literal::Nat(n)) if !n.is_negative() => {
                count = count.add(&n);
                break;
            }
            Db::Global(n) if n == "Zero" => break,
            Db::App(f, a) if matches!(f.as_ref(), Db::Global(n) if n == "Succ") => {
                count = count.add(&one);
                field = whnf(genv, &a);
            }
            _ => return None,
        }
    }
    if negative {
        count = count.add(&one).negated();
    }
    Some(crate::term::int_lit(count))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/recheck_differential_tests.rs"]
mod recheck_differential_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/beta_open_tests.rs"]
mod beta_open_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/reflexive_subtyping_tests.rs"]
mod reflexive_subtyping_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/fixed_point_structural_column_controls.rs"]
mod fixed_point_structural_column_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/borrowed_conversion_controls.rs"]
mod borrowed_conversion_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/bounded_borrowed_conversion_controls.rs"]
mod bounded_borrowed_conversion_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/checked_motive_reuse_controls.rs"]
mod checked_motive_reuse_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/owned_whnf_controls.rs"]
mod owned_whnf_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/k_probe_admission_controls.rs"]
mod k_probe_admission_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/whnf_scheduler_controls.rs"]
mod whnf_scheduler_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/nat_prefix_controls.rs"]
mod nat_prefix_controls;

// Negative-only admission: never infer a type or accept a proof here.
fn definitely_fieldful_k_family(genv: &Context, lctx: &dyn LocalTypes, proof: &Db) -> bool {
    fn spend(left: &mut usize) -> Option<()> {
        if *left == 0 {
            return None;
        }
        *left -= 1;
        Some(())
    }
    fn rigid_motive_tail<'a>(mut tail: &'a Db, left: &mut usize) -> Option<(&'a Db, usize)> {
        let mut pending = 0usize;
        loop {
            spend(left)?;
            match tail {
                Db::App(f, _) => {
                    pending += 1;
                    tail = f;
                }
                Db::Lam(_, inner) => {
                    pending = pending.saturating_sub(1);
                    tail = inner;
                }
                Db::Global(_) | Db::Const { .. } => return Some((tail, pending)),
                _ => return None,
            }
        }
    }
    #[derive(Clone, Copy)]
    enum Header<'a> {
        Named(&'a Term),
        Indexed(&'a Db),
    }
    fn observe<'a>(
        genv: &'a Context,
        lctx: &'a dyn LocalTypes,
        proof: &Db,
        left: &mut usize,
    ) -> Option<()> {
        if !genv.simp_rules().is_empty() {
            return None;
        }
        let mut head = proof;
        let mut supplied = 0usize;
        loop {
            spend(left)?;
            match head {
                Db::App(f, _) => {
                    head = f;
                    supplied += 1;
                }
                _ => break,
            }
        }
        let mut family_arguments = 0usize;
        let mut header = match head {
            Db::Var(k) => {
                Header::Indexed(lctx.binding(lctx.len().checked_sub(k.checked_add(1)?)?)?)
            }
            Db::Global(name) => Header::Named(genv.get_global(name)?),
            Db::Const { name, levels } => {
                // Inferred legacy telescopes stay on the complete old path.
                let (params, ty) = genv.poly_declaration(name)?;
                if params.len() != levels.len() {
                    return None;
                }
                Header::Named(ty)
            }
            Db::Match { motive, .. } if supplied == 0 => {
                let (tail, arguments) = rigid_motive_tail(motive, left)?;
                family_arguments = arguments;
                Header::Indexed(tail)
            }
            Db::Fix(body) if supplied != 0 => {
                // This is only a necessary shape of a SUCCESSFUL inference.
                // It neither validates the motive nor bypasses fix termination.
                let mut tail = body.as_ref();
                for _ in 0..supplied {
                    spend(left)?;
                    let Db::Lam(_, inner) = tail else {
                        return None;
                    };
                    tail = inner;
                }
                spend(left)?;
                // Exact application only; another Lambda, computed body, or
                // mutual fix retains the full old K probe.
                let Db::Match { motive, .. } = tail else {
                    return None;
                };
                let (result, arguments) = rigid_motive_tail(motive, left)?;
                family_arguments = arguments;
                // match_return_type can fail, leave Lambda(s), or apply this
                // fixed result head. None can turn a fieldful family into a
                // fieldless K family. All ordinary validation stays in infer.
                supplied = 0;
                Header::Indexed(result)
            }
            _ => return None,
        };
        for _ in 0..supplied {
            spend(left)?;
            header = match header {
                Header::Named(Term::Pi { body_type, .. }) => Header::Named(body_type),
                Header::Indexed(Db::Pi(_, body)) => Header::Indexed(body),
                _ => return None,
            };
        }
        // Pi substitution/De Bruijn shifting cannot change a fixed Global/Const
        // result head. No residual variable, alias, Let or Match is guessed.
        loop {
            spend(left)?;
            header = match header {
                Header::Named(Term::App(f, _)) => {
                    family_arguments += 1;
                    Header::Named(f)
                }
                Header::Indexed(Db::App(f, _)) => {
                    family_arguments += 1;
                    Header::Indexed(f)
                }
                _ => break,
            };
        }
        let family = match header {
            Header::Named(Term::Global(name)) | Header::Indexed(Db::Global(name)) => name.as_str(),
            Header::Named(Term::Const { name, levels })
            | Header::Indexed(Db::Const { name, levels }) => {
                let (params, _) = genv.poly_declaration(name)?;
                if params.len() != levels.len() {
                    return None;
                }
                name.as_str()
            }
            _ => return None,
        };
        if !genv.is_inductive(family)
            || genv.get_definition_body(family).is_some()
            || genv.get_universe_poly(family).is_some()
            || genv
                .mutual_block_of(family)
                .is_some_and(|block| block.len() != 1)
        {
            return None;
        }
        // Explicit parameter metadata avoids the legacy inferred-split fallback.
        let parameters = genv.inductive_declared_params(family)?;
        let mut arity = genv.get_global(family)?;
        let mut arity_count = 0usize;
        loop {
            spend(left)?;
            match arity {
                Term::Pi { body_type, .. } => {
                    arity_count += 1;
                    arity = body_type;
                }
                _ => break,
            }
        }
        if !matches!(arity, Term::Sort(_))
            || family_arguments != arity_count
            || parameters > arity_count
        {
            return None;
        }
        let (name, mut constructor_type) = genv.unique_constructor_header(family)?;
        if genv.constructor_inductive(name) != Some(family) {
            return None;
        }
        // Observing parameters+1 syntactic Pis is enough to rule out a
        // fieldless constructor; do not walk an unnecessary residual body.
        for _ in 0..=parameters {
            spend(left)?;
            match constructor_type {
                Term::Pi { body_type, .. } => constructor_type = body_type,
                _ => return None,
            }
        }
        Some(())
    }
    observe(genv, lctx, proof, &mut 256).is_some()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/k_fieldful_admission_controls.rs"]
mod k_fieldful_admission_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/k_fix_result_shape_controls.rs"]
mod k_fix_result_shape_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/k_direct_match_result_shape_controls.rs"]
mod k_direct_match_result_shape_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/k_motive_beta_head_controls.rs"]
mod k_motive_beta_head_controls;
// SOURCE DRAFT ONLY. Not installed or run. Review/freeze before runtime RED.
// Scaffold seam: try_nat_order_db_in(ctx, locals, term) forwards to the current
// literal-only try_nat_order_db(ctx,term), ignoring locals, until the candidate.
// NAT_ORDER_WORK is cfg(test) optional TLS, incremented at actual WHNF dispatch
// and cursor transition sites. It does not debit/refund fuel itself.
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/independent_nat_order_prefix_controls.rs"]
mod independent_nat_order_prefix_controls;
// Additive source draft; no runtime assertions have executed.
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/independent_observed_prefix_sharing_controls.rs"]
mod independent_observed_prefix_sharing_controls;
// Additive lease controls. Unrun; prior 125 assertions are retained separately.
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/independent_prefix_lease_controls.rs"]
mod independent_prefix_lease_controls;
// Separate additive, never-run control. Frozen13 remain unchanged.
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/prefix_lease_partial_exhaustion_control.rs"]
mod prefix_lease_partial_exhaustion_control;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/open_context_controls.rs"]
mod open_context_controls;

#[cfg(test)]
fn reference_resume_match(
    genv: &Context,
    lctx: &dyn LocalTypes,
    mut d: Db,
    motive: DbBox,
    cases: Vec<Db>,
) -> WhnfStep {
    // Peano bridge (K6): a `Nat(n)` literal is `Zero`/`Succ(Nat(n-1))` — expand
    // one step so the constructor-selection below fires (the recursor computes
    // on Nat literals, peeling one `Succ` per match), where the literal's reading holds
    // (`int_inductive::reads_nat_literal`).
    if matches!(&d, Db::Lit(Literal::Nat(n)) if crate::int_inductive::reads_nat_literal(genv, n)) {
        d = db_nat_peano_step(&d);
    }
    if let Some(constructor) = db_int_constructor(genv, &d) {
        d = constructor;
    }
    let (head, args) = spine(&d);
    if let Db::Global(cname) | Db::Const { name: cname, .. } = &head {
        if let Some(ind) = genv.constructor_inductive(cname) {
            let ctor_names: Vec<String> = genv
                .get_constructors(ind)
                .iter()
                .map(|(n, _)| n.to_string())
                .collect();
            if let Some(idx) = ctor_names.iter().position(|n| n == cname) {
                if idx < cases.len() {
                    // Indices are part of the result type, not constructor
                    // arguments. Drop only the uniform parameter prefix.
                    let parameters = genv.inductive_num_params(ind);
                    let Some(val_args) = args.get(parameters..) else {
                        return WhnfStep::Ready(Db::Match {
                            disc: DbBox::new(d),
                            motive,
                            cases,
                        });
                    };
                    let mut res = cases[idx].clone();
                    for a in val_args {
                        res = Db::App(DbBox::new(res), DbBox::new(a.clone()));
                    }
                    return WhnfStep::Reduce(res);
                }
            }
        }
    }
    // K can select a branch only for a fieldless singleton.
    // Its checked full or reachable pruned inventory has one case.
    // Avoid re-inferring a neutral discriminant when this necessary
    // condition already rules out singleton reduction.
    if cases.len() == 1 {
        if let Some(constructor) = k_constructor_db(genv, lctx, &d) {
            return WhnfStep::Reduce(Db::Match {
                disc: DbBox::new(constructor),
                motive,
                cases,
            });
        }
    }
    return WhnfStep::Ready(Db::Match {
        disc: DbBox::new(d),
        motive,
        cases,
    });
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/owned_db_match_controls.rs"]
mod owned_db_match_controls;

/// Open an owned de Bruijn body with the same index rules as beta_open_at.
/// Existing body boxes/vectors survive; only substituted argument occurrences
/// need fresh shifted terms. Unused arguments are never traversed.
fn beta_open_owned(mut body: Db, arg: &Db) -> Db {
    fn child(body: &mut DbBox, arg: &Db, depth: usize) {
        if !body.facts().is_some_and(|f| f.required <= depth) {
            open(body, arg, depth);
        }
    }
    fn open(body: &mut Db, arg: &Db, depth: usize) {
        #[cfg(test)]
        beta_count_work(0);
        match body {
            Db::Var(k) if *k == depth => *body = shift(arg, depth as isize, 0),
            Db::Var(k) if *k > depth => *k -= 1,
            Db::Var(_) | Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => {}
            Db::Pi(a, b) | Db::Lam(a, b) => {
                child(a, arg, depth);
                child(b, arg, depth + 1);
            }
            Db::App(f, a) => {
                child(f, arg, depth);
                child(a, arg, depth);
            }
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                child(disc, arg, depth);
                child(motive, arg, depth);
                for case in cases {
                    open(case, arg, depth);
                }
            }
            Db::Fix(body) => match body {
                FixBody::Owned(inner) => open(inner, arg, depth + 1),
                FixBody::Shared(_) => {
                    if !body.facts().is_some_and(|f| f.required <= depth + 1) {
                        *body = body.opened(arg, depth + 1);
                    }
                }
            },
            Db::MutualFix { defs, .. } => {
                let inner = depth + defs.len();
                for body in defs {
                    open(body, arg, inner);
                }
            }
            Db::Let(ty, value, body) => {
                child(ty, arg, depth);
                child(value, arg, depth);
                child(body, arg, depth + 1);
            }
        }
    }
    open(&mut body, arg, 0);
    body
}

fn solve_domain_levels_owned(
    genv: &Context,
    lctx: &dyn LocalTypes,
    dom: Db,
    body: Db,
    arg: &Db,
) -> (Db, Db) {
    match db_domain_level_solution(genv, lctx, &dom, arg) {
        Some(subst) => (
            db_instantiate_universes(&dom, &subst),
            db_instantiate_universes(&body, &subst),
        ),
        None => (dom, body),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/owned_db_beta_controls.rs"]
mod owned_db_beta_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/immutable_fix_body_controls.rs"]
mod immutable_fix_body_controls;

/// An immutable telescope view. Each stored type remains in its original binding
/// scope. Extensions own only new bindings and borrow the unchanged predecessor.
trait LocalTypes {
    fn len(&self) -> usize;
    fn parts(&self) -> (Option<&dyn LocalTypes>, &[Db]);
    /// Where this context remembers its number in the current conversion-evidence session.
    fn evidence(&self) -> Option<&std::cell::Cell<Option<(u64, u32)>>> {
        None
    }
    fn binding(&self, index: usize) -> Option<&Db> {
        let (mut parent, mut bindings) = self.parts();
        loop {
            let base = parent.map_or(0, LocalTypes::len);
            if index >= base {
                return bindings.get(index - base);
            }
            (parent, bindings) = parent?.parts();
        }
    }
}
impl LocalTypes for Vec<Db> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn parts(&self) -> (Option<&dyn LocalTypes>, &[Db]) {
        (None, self.as_slice())
    }
}
impl<const N: usize> LocalTypes for [Db; N] {
    fn len(&self) -> usize {
        N
    }
    fn parts(&self) -> (Option<&dyn LocalTypes>, &[Db]) {
        (None, self.as_slice())
    }
}
struct LocalExtension<'a> {
    parent: &'a dyn LocalTypes,
    base: usize,
    bindings: Vec<Db>,
    evidence: std::cell::Cell<Option<(u64, u32)>>,
}
impl LocalTypes for LocalExtension<'_> {
    fn len(&self) -> usize {
        self.base + self.bindings.len()
    }
    fn parts(&self) -> (Option<&dyn LocalTypes>, &[Db]) {
        (Some(self.parent), &self.bindings)
    }
    fn evidence(&self) -> Option<&std::cell::Cell<Option<(u64, u32)>>> {
        Some(&self.evidence)
    }
}
impl LocalExtension<'_> {
    fn push(&mut self, ty: Db) {
        self.bindings.push(ty);
        self.evidence.set(None);
    }
}
fn extend_local(lctx: &dyn LocalTypes) -> LocalExtension<'_> {
    LocalExtension {
        parent: lctx,
        base: lctx.len(),
        bindings: Vec::new(),
        evidence: std::cell::Cell::new(None),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/borrowed_local_context_controls.rs"]
mod borrowed_local_context_controls;

// Limit speculative occurrence inspection independently of expression size. This
// does not admit a larger term or grant reduction fuel: failure uses the existing
// substitution path. Current immutable Fix facts may prove a subtree unaffected.
const OWNED_BETA_SCAN_NODES: usize = 256;
#[derive(Clone, Copy)]
struct BetaScan {
    occurrences: usize,
    needs_rebase: bool,
}
impl BetaScan {
    const UNAFFECTED: Self = Self {
        occurrences: 0,
        needs_rebase: false,
    };
    fn merge(self, other: Self) -> Self {
        Self {
            occurrences: self.occurrences + other.occurrences,
            needs_rebase: self.needs_rebase || other.needs_rebase,
        }
    }
}
// The same bounded occurrence walk also records outer indices affected by
// removing this binder. Count zero alone cannot justify identity: Var(1), for
// example, contains no Var(0) but still has to become Var(0).
fn beta_scan(body: &Db, depth: usize, left: &mut usize) -> Option<BetaScan> {
    fn child(body: &DbBox, depth: usize, left: &mut usize) -> Option<BetaScan> {
        if body.facts().is_some_and(|f| f.required <= depth) {
            *left = left.checked_sub(1)?;
            #[cfg(test)]
            beta_count_work(0);
            Some(BetaScan::UNAFFECTED)
        } else {
            beta_scan(body, depth, left)
        }
    }
    *left = left.checked_sub(1)?;
    #[cfg(test)]
    beta_count_work(0);
    match body {
        Db::Var(k) => Some(BetaScan {
            occurrences: usize::from(*k == depth),
            needs_rebase: *k > depth,
        }),
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => Some(BetaScan::UNAFFECTED),
        Db::Pi(a, b) | Db::Lam(a, b) => {
            Some(child(a, depth, left)?.merge(child(b, depth + 1, left)?))
        }
        Db::App(a, b) => Some(child(a, depth, left)?.merge(child(b, depth, left)?)),
        Db::Let(a, b, c) => Some(
            child(a, depth, left)?
                .merge(child(b, depth, left)?)
                .merge(child(c, depth + 1, left)?),
        ),
        Db::Match {
            disc,
            motive,
            cases,
        } => {
            let mut scan = child(disc, depth, left)?.merge(child(motive, depth, left)?);
            for case in cases {
                scan = scan.merge(beta_scan(case, depth, left)?);
            }
            Some(scan)
        }
        Db::MutualFix { defs, .. } => {
            let mut scan = BetaScan::UNAFFECTED;
            for body in defs {
                scan = scan.merge(beta_scan(body, depth + defs.len(), left)?);
            }
            Some(scan)
        }
        Db::Fix(body) => match body {
            FixBody::Owned(body) => beta_scan(body, depth + 1, left),
            FixBody::Shared(_) => body
                .facts()
                .filter(|f| f.required <= depth + 1)
                .map(|_| BetaScan::UNAFFECTED),
        },
    }
}

// Preserve the existing occurrence-only test interface without an extra walk.
#[cfg(test)]
fn beta_occurrences(body: &Db, depth: usize, left: &mut usize) -> Option<usize> {
    beta_scan(body, depth, left).map(|scan| scan.occurrences)
}

/// Shift exclusively owned syntax in place. Shared immutable Fix bodies retain
/// the existing copy/metadata rules; no shared syntax is mutated.
fn shift_owned(mut term: Db, delta: isize, cutoff: usize) -> Db {
    fn child(t: &mut DbBox, d: isize, c: usize) {
        if !t.facts().is_some_and(|f| f.required <= c) {
            walk(t, d, c);
        }
    }
    fn walk(t: &mut Db, d: isize, c: usize) {
        #[cfg(test)]
        beta_count_work(0);
        match t {
            Db::Var(k) => {
                if *k >= c {
                    *k = (*k as isize + d) as usize;
                }
            }
            Db::Pi(a, b) | Db::Lam(a, b) => {
                child(a, d, c);
                child(b, d, c + 1);
            }
            Db::App(a, b) => {
                child(a, d, c);
                child(b, d, c);
            }
            Db::Let(a, b, body) => {
                child(a, d, c);
                child(b, d, c);
                child(body, d, c + 1);
            }
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                child(disc, d, c);
                child(motive, d, c);
                for case in cases {
                    walk(case, d, c);
                }
            }
            Db::MutualFix { defs, .. } => {
                let inner = c + defs.len();
                for body in defs {
                    walk(body, d, inner);
                }
            }
            Db::Fix(body) => match body {
                FixBody::Owned(body) => walk(body, d, c + 1),
                FixBody::Shared(_) => {
                    *body = body.shifted(d, c + 1);
                }
            },
            Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => {}
        }
    }
    if delta != 0 {
        walk(&mut term, delta, cutoff);
    }
    term
}

/// Copy the argument only for uses before the final one, which takes ownership.
/// The count and rewrite follow exactly the same binder depths. Unknown shared
/// metadata and oversized bodies fall back to the borrowed-argument operation.
fn beta_open_consuming(mut body: Db, arg: Db) -> Db {
    let mut scan_left = OWNED_BETA_SCAN_NODES;
    let Some(scan) = beta_scan(&body, 0, &mut scan_left) else {
        return beta_open_deferred(body, arg);
    };
    // Only a complete scan can prove both substitution and rebasing absent.
    // Return the owned syntax intact; shared tickets and child storage survive.
    if scan.occurrences == 0 && !scan.needs_rebase {
        return body;
    }
    let mut remaining = scan.occurrences;
    fn child(body: &mut DbBox, arg: &mut Option<Db>, remaining: &mut usize, depth: usize) {
        if !body.facts().is_some_and(|f| f.required <= depth) {
            open(body, arg, remaining, depth);
        }
    }
    fn open(body: &mut Db, arg: &mut Option<Db>, remaining: &mut usize, depth: usize) {
        #[cfg(test)]
        beta_count_work(0);
        match body {
            Db::Var(k) if *k == depth => {
                *remaining -= 1;
                *body = if *remaining == 0 {
                    shift_owned(
                        arg.take().expect("last counted occurrence owns argument"),
                        depth as isize,
                        0,
                    )
                } else {
                    shift(
                        arg.as_ref().expect("earlier occurrence borrows argument"),
                        depth as isize,
                        0,
                    )
                };
            }
            Db::Var(k) if *k > depth => *k -= 1,
            Db::Var(_) | Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => {}
            Db::Pi(a, b) | Db::Lam(a, b) => {
                child(a, arg, remaining, depth);
                child(b, arg, remaining, depth + 1);
            }
            Db::App(a, b) => {
                child(a, arg, remaining, depth);
                child(b, arg, remaining, depth);
            }
            Db::Let(a, b, c) => {
                child(a, arg, remaining, depth);
                child(b, arg, remaining, depth);
                child(c, arg, remaining, depth + 1);
            }
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                child(disc, arg, remaining, depth);
                child(motive, arg, remaining, depth);
                for case in cases {
                    open(case, arg, remaining, depth);
                }
            }
            Db::MutualFix { defs, .. } => {
                let inner = depth + defs.len();
                for body in defs {
                    open(body, arg, remaining, inner);
                }
            }
            Db::Fix(body) => match body {
                FixBody::Owned(body) => open(body, arg, remaining, depth + 1),
                FixBody::Shared(_) => {
                    debug_assert!(body.facts().is_some_and(|f| f.required <= depth + 1))
                }
            },
        }
    }
    open(&mut body, &mut Some(arg), &mut remaining, 0);
    debug_assert_eq!(remaining, 0);
    body
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/owned_beta_argument_controls.rs"]
mod owned_beta_argument_controls;

/// A single traversal can retain the final substitution slot as an exclusive
/// borrow. Encountering another occurrence fills the previous slot by copying;
/// after the walk, the retained slot takes ownership of the original argument.
/// Only disjoint descendants are borrowed, so no pointer or unsafe mutation is
/// required and the enclosing syntax cannot move while a slot is retained.
fn beta_open_deferred(mut body: Db, arg: Db) -> Db {
    fn flush(last: &mut Option<(&mut Db, usize)>, arg: &Db) {
        if let Some((slot, depth)) = last.take() {
            *slot = shift(arg, depth as isize, 0);
        }
    }
    fn child<'a>(
        body: &'a mut DbBox,
        arg: &Db,
        last: &mut Option<(&'a mut Db, usize)>,
        depth: usize,
    ) {
        if !body.facts().is_some_and(|f| f.required <= depth) {
            open(body, arg, last, depth);
        }
    }
    fn open<'a>(body: &'a mut Db, arg: &Db, last: &mut Option<(&'a mut Db, usize)>, depth: usize) {
        #[cfg(test)]
        beta_count_work(0);
        match body {
            Db::Var(k) if *k == depth => {
                flush(last, arg);
                *last = Some((body, depth));
            }
            Db::Var(k) if *k > depth => *k -= 1,
            Db::Var(_) | Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => {}
            Db::Pi(a, b) | Db::Lam(a, b) => {
                child(a, arg, last, depth);
                child(b, arg, last, depth + 1);
            }
            Db::App(a, b) => {
                child(a, arg, last, depth);
                child(b, arg, last, depth);
            }
            Db::Let(a, b, c) => {
                child(a, arg, last, depth);
                child(b, arg, last, depth);
                child(c, arg, last, depth + 1);
            }
            Db::Match {
                disc,
                motive,
                cases,
            } => {
                child(disc, arg, last, depth);
                child(motive, arg, last, depth);
                for case in cases {
                    open(case, arg, last, depth);
                }
            }
            Db::MutualFix { defs, .. } => {
                let inner = depth + defs.len();
                for body in defs {
                    open(body, arg, last, inner);
                }
            }
            Db::Fix(body) => match body {
                FixBody::Owned(body) => open(body, arg, last, depth + 1),
                FixBody::Shared(_) => {
                    if !body.facts().is_some_and(|f| f.required <= depth + 1) {
                        // The old immutable-body operation owns its metadata
                        // policy. Finish any pending slot before delegating;
                        // a subsequent owned occurrence can again be deferred.
                        flush(last, arg);
                        *body = body.opened(arg, depth + 1);
                    }
                }
            },
        }
    }
    let mut last = None;
    open(&mut body, &arg, &mut last, 0);
    if let Some((slot, depth)) = last {
        *slot = shift_owned(arg, depth as isize, 0);
    }
    body
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/deferred_beta_argument_controls.rs"]
mod deferred_beta_argument_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/borrowed_inspection_controls.rs"]
mod borrowed_inspection_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/borrowed_structural_decision_controls.rs"]
mod borrowed_structural_decision_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/certified_order_recheck_controls.rs"]
mod certified_order_recheck_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/independent_power_controls.rs"]
mod independent_power_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/bounded_alias_conversion_controls.rs"]
mod bounded_alias_conversion_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/owned_match_conversion_controls.rs"]
mod owned_match_conversion_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/storage_controls.rs"]
mod storage_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/storage_accounting_controls.rs"]
mod storage_accounting_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/storage_differential_controls.rs"]
mod storage_differential_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/definition_cache_controls.rs"]
mod definition_cache_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/zero_shift_controls.rs"]
mod zero_shift_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/dead_binder_controls.rs"]
mod dead_binder_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/argument_edge_controls.rs"]
mod argument_edge_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/head_closure_controls.rs"]
mod head_closure_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/head_closure_adversarial_controls.rs"]
mod head_closure_adversarial_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/head_closure_level_copy_controls.rs"]
mod head_closure_level_copy_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/head_closure_binder_gate_controls.rs"]
mod head_closure_binder_gate_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/fix_argument_edge_controls.rs"]
mod fix_argument_edge_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/open_mutual_controls.rs"]
mod open_mutual_controls;
