//! Optional immutable storage for ordinary independent-kernel child edges.
//!
//! Sharing preserves syntax only. It supplies no typing, equality, or reduction
//! certificate. Every shared allocation retains its own persistent reservation;
//! an enclosing allocation accounts only for its exclusive owned descendants.
use super::*;
use std::boxed::Box as OwnedBox;
use std::rc::Rc;

pub(super) struct AdmittedDb {
    body: Db,
    facts: FixFacts,
    generation: usize,
    // Declared last so syntax is destroyed before its reservation is released.
    memory: FixSharedTicket,
}

pub(super) enum DbBox {
    Owned(OwnedBox<Db>),
    Shared(Rc<AdmittedDb>),
}

impl DbBox {
    pub(super) fn owned(body: OwnedBox<Db>) -> Self {
        Self::Owned(body)
    }

    pub(super) fn new(body: Db) -> Self {
        let admitted = (|| {
            let facts = fix_facts(&body)?;
            let bytes = exclusive_retained_bytes(&body, std::mem::size_of::<AdmittedDb>())?;
            // The physical walk charges its root visit before allocation;
            // no second synthetic admission unit is needed here.
            let memory = FixSharedTicket::new(bytes)?;
            let generation = FIX_FACTORY.with(|f| f.get().scope);
            Some((facts, generation, memory))
        })();
        match admitted {
            Some((facts, generation, memory)) => Self::Shared(Rc::new(AdmittedDb {
                body,
                facts,
                generation,
                memory,
            })),
            None => Self::Owned(OwnedBox::new(body)),
        }
    }

    pub(super) fn into_db(self) -> Db {
        match self {
            Self::Owned(body) => *body,
            Self::Shared(body) => match Rc::try_unwrap(body) {
                // This transfers exclusive syntax back to ordinary evaluator
                // ownership, like FixBody::into_db. Nested shared allocations
                // keep their own tickets, including when this parent disappears.
                Ok(body) => body.body,
                Err(body) => body.body.clone(),
            },
        }
    }

    pub(super) fn into_box(self) -> OwnedBox<Db> {
        match self {
            Self::Owned(body) => body,
            shared => OwnedBox::new(shared.into_db()),
        }
    }

    pub(super) fn facts(&self) -> Option<FixFacts> {
        let Self::Shared(body) = self else {
            return None;
        };
        let factory = FIX_FACTORY.with(|f| f.get());
        (factory.started && factory.scope != 0 && factory.scope == body.generation)
            .then_some(body.facts)
    }

    #[cfg(test)]
    pub(super) fn is_shared(&self) -> bool {
        matches!(self, Self::Shared(_))
    }

    /// Exclusive allocation bytes reserved by this edge, excluding independently
    /// ticketed descendants. This is not the logical unfolded size.
    #[cfg(test)]
    pub(super) fn retained_bytes(&self) -> usize {
        match self {
            Self::Owned(_) => 0,
            Self::Shared(body) => body.memory.bytes,
        }
    }

    pub(super) fn shifted(&self, delta: isize, cutoff: usize) -> Self {
        // Identity does not depend on current-generation admission facts.
        if delta == 0 {
            #[cfg(test)]
            if let Self::Owned(body) = self {
                beta_count_clone(body);
            }
            return self.clone();
        }
        if self.facts().is_some_and(|f| f.required <= cutoff) {
            return self.clone();
        }
        Self::new(shift(self, delta, cutoff))
    }

    pub(super) fn opened(&self, argument: &Db, depth: usize) -> Self {
        if self.facts().is_some_and(|f| f.required <= depth) {
            return self.clone();
        }
        Self::new(beta_open_at(self, argument, depth))
    }
}

impl Clone for DbBox {
    fn clone(&self) -> Self {
        match self {
            Self::Owned(body) => Self::Owned(body.clone()),
            Self::Shared(body) => Self::Shared(Rc::clone(body)),
        }
    }
}
impl std::fmt::Debug for DbBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_ref().fmt(f)
    }
}
impl PartialEq for DbBox {
    fn eq(&self, other: &Self) -> bool {
        // No pointer shortcut: a shared Float NaN must remain non-reflexive.
        self.as_ref() == other.as_ref()
    }
}
impl AsRef<Db> for DbBox {
    fn as_ref(&self) -> &Db {
        match self {
            Self::Owned(body) => body,
            Self::Shared(body) => &body.body,
        }
    }
}
impl std::ops::Deref for DbBox {
    type Target = Db;
    fn deref(&self) -> &Db {
        self.as_ref()
    }
}
impl std::ops::DerefMut for DbBox {
    fn deref_mut(&mut self) -> &mut Db {
        if matches!(self, Self::Shared(_)) {
            // Never expose a mutable AdmittedDb, even when its Rc is unique:
            // mutation could invalidate both its facts and reserved byte count.
            // Demotion moves unique storage or copies shared storage first.
            let old = std::mem::replace(self, Self::Owned(OwnedBox::new(Db::Var(0))));
            *self = Self::Owned(old.into_box());
        }
        match self {
            Self::Owned(body) => body,
            Self::Shared(_) => unreachable!("shared storage was demoted"),
        }
    }
}

/// Count the physical storage retained exclusively by a proposed shared root.
/// Logical admission has already bounded depth, nodes, and payload; this second
/// independently charged walk adds capacities and excludes nested reservations.
/// It allocates no traversal frontier and never follows more than PREFIX_DEPTH
/// recursive edges. All fallible work precedes the new Rc allocation.
pub(super) fn exclusive_retained_bytes(root: &Db, allocation_size: usize) -> Option<usize> {
    struct Measure {
        bytes: usize,
        limit: usize,
        remaining: usize,
    }
    impl Measure {
        fn add(&mut self, bytes: usize) -> Option<()> {
            self.bytes = self.bytes.checked_add(bytes)?;
            (self.bytes <= self.limit).then_some(())
        }
        fn visit(&mut self, depth: usize) -> Option<()> {
            if depth > PREFIX_DEPTH {
                return None;
            }
            self.remaining = self.remaining.checked_sub(1)?;
            fix_factory_spend(1)
        }
        fn string(&mut self, value: &String) -> Option<()> {
            fix_factory_spend(1)?;
            if value.len() > 256 {
                return None;
            }
            self.add(value.capacity())
        }
        fn level(&mut self, level: &Universe, depth: usize) -> Option<()> {
            self.visit(depth)?;
            match level {
                Universe::Var(name) => self.string(name)?,
                Universe::Succ(a) => {
                    self.add(std::mem::size_of::<Universe>())?;
                    self.level(a, depth + 1)?;
                }
                Universe::Max(a, b) | Universe::IMax(a, b) => {
                    self.add(2 * std::mem::size_of::<Universe>())?;
                    self.level(a, depth + 1)?;
                    self.level(b, depth + 1)?;
                }
                Universe::SProp | Universe::Prop | Universe::Type(_) => {}
            }
            Some(())
        }
        fn child(&mut self, child: &DbBox, depth: usize) -> Option<()> {
            fix_factory_spend(1)?;
            if let DbBox::Owned(body) = child {
                self.add(std::mem::size_of::<Db>())?;
                self.node(body, depth)?;
            }
            // Shared children own their Rc, metadata, payload and descendants
            // through a separate persistent ticket; their edge is inline in Db.
            Some(())
        }
        fn node(&mut self, node: &Db, depth: usize) -> Option<()> {
            self.visit(depth)?;
            match node {
                Db::Global(name) => self.string(name)?,
                Db::Sort(level) => self.level(level, depth + 1)?,
                Db::Const { name, levels } => {
                    self.string(name)?;
                    if levels.len() > PREFIX_NODES {
                        return None;
                    }
                    self.add(
                        levels
                            .capacity()
                            .checked_mul(std::mem::size_of::<Universe>())?,
                    )?;
                    for level in levels {
                        self.level(level, depth + 1)?;
                    }
                }
                Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
                    self.child(a, depth + 1)?;
                    self.child(b, depth + 1)?;
                }
                Db::Let(a, b, c) => {
                    self.child(a, depth + 1)?;
                    self.child(b, depth + 1)?;
                    self.child(c, depth + 1)?;
                }
                Db::Match {
                    disc,
                    motive,
                    cases,
                } => {
                    self.child(disc, depth + 1)?;
                    self.child(motive, depth + 1)?;
                    if cases.len() > PREFIX_NODES {
                        return None;
                    }
                    self.add(cases.capacity().checked_mul(std::mem::size_of::<Db>())?)?;
                    for case in cases {
                        self.node(case, depth + 1)?;
                    }
                }
                Db::MutualFix { defs, .. } => {
                    if defs.len() > PREFIX_NODES {
                        return None;
                    }
                    self.add(defs.capacity().checked_mul(std::mem::size_of::<Db>())?)?;
                    for body in defs {
                        self.node(body, depth + 1)?;
                    }
                }
                Db::Fix(FixBody::Owned(body)) => {
                    self.add(std::mem::size_of::<Db>())?;
                    self.node(body, depth + 1)?;
                }
                Db::Fix(FixBody::Shared(_)) => {}
                Db::Lit(Literal::Text(value)) => self.string(value)?,
                Db::Lit(Literal::Float(_)) => return None,
                // These admitted values use BigInt's canonical inline limb.
                Db::Lit(Literal::Nat(n)) => {
                    if n.is_negative() || n.to_i64().is_none() {
                        return None;
                    }
                }
                Db::Lit(Literal::BigInt(n)) => {
                    n.to_i64()?;
                }
                Db::Var(_) | Db::Lit(_) => {}
            }
            Some(())
        }
    }
    let overhead = allocation_size
        .checked_sub(std::mem::size_of::<Db>())?
        .checked_add(2 * std::mem::size_of::<usize>())?;
    let mut measure = Measure {
        bytes: std::mem::size_of::<Db>().checked_add(overhead)?,
        limit: PREFIX_TREE_BYTES.checked_add(overhead)?,
        remaining: PREFIX_NODES.checked_mul(2)?,
    };
    measure.node(root, 1)?;
    Some(measure.bytes)
}
