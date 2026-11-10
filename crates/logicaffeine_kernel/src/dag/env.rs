//! The environment: every declaration the kernel has admitted, indexed by name.

use super::expr::ExprId;
use super::name::NameId;
use super::wellknown::QuotNames;
use std::rc::Rc;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Hint {
    Opaque,
    Abbrev,
    Regular(u32),
}

impl Hint {
    /// Lean's `ReducibilityHints.lt'`: `a` unfolds after `b`.
    pub fn lt(self, other: Hint) -> bool {
        match (self, other) {
            (_, Hint::Opaque) => false,
            (Hint::Abbrev, _) => false,
            (Hint::Opaque, _) => true,
            (_, Hint::Abbrev) => true,
            (Hint::Regular(a), Hint::Regular(b)) => a < b,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Safety {
    Safe,
    Unsafe,
    Partial,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum QuotKind {
    Type,
    Ctor,
    Lift,
    Ind,
}

impl QuotKind {
    pub const ALL: [QuotKind; 4] = [QuotKind::Type, QuotKind::Ctor, QuotKind::Lift, QuotKind::Ind];

    /// What the constant is to its quotient, as a refusal names it.
    pub fn role(self) -> &'static str {
        match self {
            QuotKind::Type => "type",
            QuotKind::Ctor => "constructor",
            QuotKind::Lift => "lift",
            QuotKind::Ind => "induction principle",
        }
    }

    /// The members of its quotient the constant's canonical type names, which are declared first.
    pub fn named_members(self) -> &'static [QuotKind] {
        match self {
            QuotKind::Type => &[],
            QuotKind::Ctor | QuotKind::Lift => &[QuotKind::Type],
            QuotKind::Ind => &[QuotKind::Type, QuotKind::Ctor],
        }
    }
}

/// A quotient eliminator's computation rule, `lift f h (mk a) ⟶ f a` and `ind h (mk a) ⟶ h a`: the
/// position of the major premise, the position of the function the rule applies to the constructor's
/// argument, and the constructor the major premise must be headed by.
#[derive(Copy, Clone, Debug)]
pub struct QuotRule {
    pub major: usize,
    pub fun: usize,
    pub mk: NameId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecRule {
    pub ctor: NameId,
    pub nfields: u32,
    pub rhs: ExprId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InductiveInfo {
    pub num_params: u32,
    pub num_indices: u32,
    pub all: Rc<[NameId]>,
    pub ctors: Rc<[NameId]>,
    pub num_nested: u32,
    pub is_rec: bool,
    pub is_reflexive: bool,
    pub is_unsafe: bool,
    /// The inductive's own recursor, `<name>.rec`.
    pub rec: NameId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CtorInfo {
    pub induct: NameId,
    pub cidx: u32,
    pub num_params: u32,
    pub num_fields: u32,
    pub is_unsafe: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecInfo {
    pub all: Rc<[NameId]>,
    pub num_params: u32,
    pub num_indices: u32,
    pub num_motives: u32,
    pub num_minors: u32,
    pub rules: Rc<[RecRule]>,
    pub k: bool,
    pub is_unsafe: bool,
    /// The inductive type of the major premise (for a nested auxiliary recursor this is the
    /// nested type, e.g. `List`).
    pub major_induct: NameId,
}

impl RecInfo {
    pub fn major_idx(&self) -> usize {
        (self.num_params + self.num_motives + self.num_minors + self.num_indices) as usize
    }
    pub fn first_index_idx(&self) -> usize {
        (self.num_params + self.num_motives + self.num_minors) as usize
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Axiom {
        is_unsafe: bool,
    },
    Defn {
        value: ExprId,
        hint: Hint,
        safety: Safety,
    },
    Thm {
        value: ExprId,
    },
    Opaque {
        value: ExprId,
        is_unsafe: bool,
    },
    Quot(QuotKind),
    Induct(InductiveInfo),
    Ctor(CtorInfo),
    Rec(RecInfo),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Const {
    pub name: NameId,
    pub lparams: Rc<[NameId]>,
    pub ty: ExprId,
    pub kind: Kind,
}

impl Const {
    pub fn is_unsafe(&self) -> bool {
        match &self.kind {
            Kind::Axiom { is_unsafe } | Kind::Opaque { is_unsafe, .. } => *is_unsafe,
            Kind::Defn { safety, .. } => *safety == Safety::Unsafe,
            Kind::Thm { .. } | Kind::Quot(_) => false,
            Kind::Induct(i) => i.is_unsafe,
            Kind::Ctor(c) => c.is_unsafe,
            Kind::Rec(r) => r.is_unsafe,
        }
    }

    pub fn is_partial(&self) -> bool {
        matches!(
            self.kind,
            Kind::Defn {
                safety: Safety::Partial,
                ..
            }
        )
    }

    /// The value δ-reduction unfolds to: definitions and theorems (as in Lean's kernel).
    pub fn delta_value(&self) -> Option<ExprId> {
        match self.kind {
            Kind::Defn { value, .. } | Kind::Thm { value } => Some(value),
            _ => None,
        }
    }

    pub fn hint(&self) -> Hint {
        match self.kind {
            Kind::Defn { hint, .. } => hint,
            _ => Hint::Opaque,
        }
    }
}

/// Constants indexed densely by [`NameId`].
#[derive(Clone, Default)]
pub struct Env {
    consts: Vec<Option<Rc<Const>>>,
    pub count: usize,
    pub quot_init: bool,
    /// Every quotient installed whole, each member admitted as its own kind, in the order its last
    /// constant arrived.
    pub quotients: Vec<QuotNames>,
    /// Declarations present only while another is checked (a recursive definition's own header,
    /// an inductive family's working declarations). Nothing derived from them may be cached
    /// beyond that check: a nested family's auxiliary name can recur with another meaning.
    provisional: Vec<NameId>,
}

impl Env {
    #[inline]
    pub fn get(&self, n: NameId) -> Option<&Rc<Const>> {
        self.consts.get(n.0 as usize).and_then(|c| c.as_ref())
    }

    pub fn contains(&self, n: NameId) -> bool {
        self.get(n).is_some()
    }

    /// Every admitted constant, in name-id order.
    pub fn constants(&self) -> impl Iterator<Item = &Rc<Const>> {
        self.consts.iter().flatten()
    }

    pub fn add(&mut self, c: Const) {
        let i = c.name.0 as usize;
        if self.consts.len() <= i {
            self.consts.resize(i + 1, None);
        }
        debug_assert!(self.consts[i].is_none());
        self.consts[i] = Some(Rc::new(c));
        self.count += 1;
    }

    /// Adds a declaration that will be withdrawn with [`Env::remove`] once the check needing it
    /// is done.
    pub fn add_provisional(&mut self, c: Const) {
        self.provisional.push(c.name);
        self.add(c);
    }

    /// Withdraws a provisional declaration.
    pub fn remove(&mut self, n: NameId) {
        if let Some(slot) = self.consts.get_mut(n.0 as usize) {
            if slot.take().is_some() {
                self.count -= 1;
            }
        }
        self.provisional.retain(|p| *p != n);
    }

    /// Whether results derived from `n` may outlive the current declaration.
    #[inline]
    pub fn is_settled(&self, n: NameId) -> bool {
        self.provisional.is_empty() || !self.provisional.contains(&n)
    }

    pub fn inductive(&self, n: NameId) -> Option<&InductiveInfo> {
        match &self.get(n)?.kind {
            Kind::Induct(i) => Some(i),
            _ => None,
        }
    }

    pub fn ctor(&self, n: NameId) -> Option<&CtorInfo> {
        match &self.get(n)?.kind {
            Kind::Ctor(c) => Some(c),
            _ => None,
        }
    }

    pub fn rec(&self, n: NameId) -> Option<&RecInfo> {
        match &self.get(n)?.kind {
            Kind::Rec(r) => Some(r),
            _ => None,
        }
    }

    /// The kind of `n`, when a quotient declaration admitted it.
    #[inline]
    pub fn quot_kind(&self, n: NameId) -> Option<QuotKind> {
        match self.get(n)?.kind {
            Kind::Quot(k) => Some(k),
            _ => None,
        }
    }

    /// Whether `q`'s member of kind `kind` is admitted, as that member.
    pub fn has_quot_member(&self, q: &QuotNames, kind: QuotKind) -> bool {
        self.quot_kind(q.member(kind)) == Some(kind)
    }

    /// The computation rule of `n`, when `n` is the lift or the induction principle of a quotient
    /// installed whole, admitted as that eliminator.
    #[inline]
    pub fn quot_rule(&self, n: NameId) -> Option<QuotRule> {
        let q = self.quotients.iter().find(|q| q.lift == n || q.ind == n)?;
        let (kind, major) = if n == q.lift {
            (QuotKind::Lift, 5)
        } else {
            (QuotKind::Ind, 4)
        };
        (self.quot_kind(n) == Some(kind)).then_some(QuotRule {
            major,
            fun: 3,
            mk: q.mk,
        })
    }

    /// Whether `m` is the constructor `rule` computes on: its quotient's own, admitted as the
    /// constructor.
    #[inline]
    pub fn is_quot_mk(&self, rule: &QuotRule, m: NameId) -> bool {
        m == rule.mk && self.quot_kind(m) == Some(QuotKind::Ctor)
    }

    /// A non-recursive inductive with one constructor and no indices: the types with struct-η.
    pub fn is_non_rec_structure(&self, n: NameId) -> bool {
        matches!(self.inductive(n), Some(i) if !i.is_rec && i.ctors.len() == 1 && i.num_indices == 0)
    }

    /// Whether the inductive `n` eliminates into every universe: its own recursor takes one universe parameter, its
    /// motive's, beyond the inductive's. A proposition that eliminates only into `Prop` takes none, and a proposition
    /// that eliminates into every universe has only proofs for fields.
    pub fn eliminates_into_every_sort(&self, n: NameId) -> bool {
        match (self.get(n), self.inductive(n).and_then(|i| self.get(i.rec))) {
            (Some(i), Some(r)) => r.lparams.len() == i.lparams.len() + 1,
            _ => false,
        }
    }
}
