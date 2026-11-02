//! Universe levels: hash-consed, with a COMPLETE decision procedure for `≤` and `≡`.
//!
//! Lean's C++ kernel decides level equality by a normalisation that is sound but incomplete, and
//! two of the Arena's soundness tests (`level-imax-leq`, `level-imax-normalization`) exist because
//! an earlier version of it was unsound. Here `u ≤ v` is decided exactly — "for every assignment
//! of natural numbers to the parameters" — by case analysis on the only non-monotone operator:
//! `imax u v` is `0` when `v = 0` and `max u v` otherwise. A parameter that can make an `imax`
//! right side vanish is split into `p = 0` and `p = p' + 1`; once no `imax` can vanish, both
//! sides are `max(c, p₁+k₁, …)` and comparison is a closed-form check.

use super::name::NameId;
use rustc_hash::FxHashMap as FxMap;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct LevelId(pub u32);

impl LevelId {
    pub const ZERO: LevelId = LevelId(0);
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum LevelNode {
    Zero,
    Succ(LevelId),
    Max(LevelId, LevelId),
    IMax(LevelId, LevelId),
    Param(NameId),
    /// LOGOS's `SProp`, the definitionally proof-irrelevant sort below `Prop`. It is only ever a
    /// whole level: `succ`, `max` and `imax` absorb it (`succ SProp = 1`, `max SProp u = u`,
    /// `imax u SProp = SProp`), it is `≤` every level, and only `SProp` is `≤` it.
    SProp,
}

/// An interned list of levels (the universe arguments of a constant).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LevelsId(pub u32);

impl LevelsId {
    pub const EMPTY: LevelsId = LevelsId(0);
}

/// A universe substitution — a constant's level parameters replaced by the levels of one use of
/// it — interned, so that a value can carry it as one id.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct SubstId(pub u32);

impl SubstId {
    /// Replaces nothing.
    pub const IDENTITY: SubstId = SubstId(0);
}

#[derive(Clone)]
struct Subst {
    ps: Box<[NameId]>,
    us: Box<[LevelId]>,
}

#[derive(Clone)]
pub struct Levels {
    nodes: Vec<LevelNode>,
    has_param: Vec<bool>,
    index: FxMap<LevelNode, LevelId>,
    lists: Vec<Box<[LevelId]>>,
    list_index: FxMap<Box<[LevelId]>, LevelsId>,
    leq_memo: FxMap<(LevelId, LevelId), bool>,
    substs: Vec<Subst>,
    subst_index: FxMap<(LevelsId, LevelsId), SubstId>,
    subst_memo: FxMap<(LevelId, SubstId), LevelId>,
    subst_list_memo: FxMap<(LevelsId, SubstId), LevelsId>,
    /// `SProp`'s id, once a LOGOS term has used it: until then no level is `SProp`.
    sprop: Option<LevelId>,
}

impl Default for Levels {
    fn default() -> Self {
        Self::new()
    }
}

impl Levels {
    pub fn new() -> Levels {
        let mut ls = Levels {
            nodes: Vec::new(),
            has_param: Vec::new(),
            index: FxMap::default(),
            lists: Vec::new(),
            list_index: FxMap::default(),
            leq_memo: FxMap::default(),
            substs: vec![Subst {
                ps: Box::new([]),
                us: Box::new([]),
            }],
            subst_index: FxMap::default(),
            subst_memo: FxMap::default(),
            subst_list_memo: FxMap::default(),
            sprop: None,
        };
        let z = ls.intern(LevelNode::Zero);
        debug_assert_eq!(z, LevelId::ZERO);
        let e = ls.list(&[]);
        debug_assert_eq!(e, LevelsId::EMPTY);
        ls
    }

    pub fn intern(&mut self, node: LevelNode) -> LevelId {
        if let Some(&id) = self.index.get(&node) {
            return id;
        }
        let hp = match node {
            LevelNode::Zero => false,
            LevelNode::Succ(a) => self.has_param(a),
            LevelNode::Max(a, b) | LevelNode::IMax(a, b) => self.has_param(a) || self.has_param(b),
            LevelNode::Param(_) => true,
            LevelNode::SProp => false,
        };
        let id = LevelId(self.nodes.len() as u32);
        self.nodes.push(node);
        self.has_param.push(hp);
        self.index.insert(node, id);
        id
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[inline]
    pub fn node(&self, l: LevelId) -> LevelNode {
        self.nodes[l.0 as usize]
    }

    #[inline]
    pub fn has_param(&self, l: LevelId) -> bool {
        self.has_param[l.0 as usize]
    }

    pub fn list(&mut self, ls: &[LevelId]) -> LevelsId {
        if let Some(&id) = self.list_index.get(ls) {
            return id;
        }
        let id = LevelsId(self.lists.len() as u32);
        let boxed: Box<[LevelId]> = ls.into();
        self.lists.push(boxed.clone());
        self.list_index.insert(boxed, id);
        id
    }

    #[inline]
    pub fn get_list(&self, id: LevelsId) -> &[LevelId] {
        &self.lists[id.0 as usize]
    }

    pub fn succ(&mut self, l: LevelId) -> LevelId {
        let l = if self.is_sprop(l) { LevelId::ZERO } else { l };
        self.intern(LevelNode::Succ(l))
    }

    /// LOGOS's `SProp` (see [`LevelNode::SProp`]).
    pub fn sprop(&mut self) -> LevelId {
        let l = self.intern(LevelNode::SProp);
        self.sprop = Some(l);
        l
    }

    #[inline]
    pub fn is_sprop(&self, l: LevelId) -> bool {
        self.sprop == Some(l)
    }

    pub fn param(&mut self, n: NameId) -> LevelId {
        self.intern(LevelNode::Param(n))
    }

    pub fn max_raw(&mut self, a: LevelId, b: LevelId) -> LevelId {
        self.intern(LevelNode::Max(a, b))
    }

    pub fn imax_raw(&mut self, a: LevelId, b: LevelId) -> LevelId {
        self.intern(LevelNode::IMax(a, b))
    }

    pub fn of_nat(&mut self, n: u64) -> LevelId {
        (0..n).fold(LevelId::ZERO, |l, _| self.succ(l))
    }

    /// Syntactic: `true` only when `l` is nonzero under every assignment (`is_never_zero`).
    pub fn is_never_zero(&self, l: LevelId) -> bool {
        match self.node(l) {
            LevelNode::Zero | LevelNode::Param(_) | LevelNode::SProp => false,
            LevelNode::Succ(_) => true,
            LevelNode::Max(a, b) => self.is_never_zero(a) || self.is_never_zero(b),
            LevelNode::IMax(_, b) => self.is_never_zero(b),
        }
    }

    fn to_offset(&self, mut l: LevelId) -> (LevelId, u64) {
        let mut k = 0;
        while let LevelNode::Succ(a) = self.node(l) {
            l = a;
            k += 1;
        }
        (l, k)
    }

    /// Lean's `mkLevelMax'`: `max` with the cheap simplifications the kernel applies.
    pub fn max(&mut self, a: LevelId, b: LevelId) -> LevelId {
        if a == b {
            return a;
        }
        if self.is_sprop(a) {
            return b;
        }
        if self.is_sprop(b) {
            return a;
        }
        if a == LevelId::ZERO {
            return b;
        }
        if b == LevelId::ZERO {
            return a;
        }
        if let LevelNode::Max(_, b2) = self.node(a) {
            if b2 == b {
                return a;
            }
        }
        let (ab, ak) = self.to_offset(a);
        let (bb, bk) = self.to_offset(b);
        if ab == bb {
            return if ak >= bk { a } else { b };
        }
        self.max_raw(a, b)
    }

    /// Lean's `mkLevelIMax'`.
    pub fn imax(&mut self, a: LevelId, b: LevelId) -> LevelId {
        if self.is_sprop(b) {
            return b;
        }
        let a = if self.is_sprop(a) { LevelId::ZERO } else { a };
        if self.is_never_zero(b) {
            return self.max(a, b);
        }
        if b == LevelId::ZERO || a == LevelId::ZERO || a == b {
            return b;
        }
        if a == self.of_nat(1) {
            return b;
        }
        self.imax_raw(a, b)
    }

    /// Replaces each `Param(ps[i])` by `us[i]`.
    pub fn instantiate(&mut self, l: LevelId, ps: &[NameId], us: &[LevelId]) -> LevelId {
        if !self.has_param(l) {
            return l;
        }
        match self.node(l) {
            LevelNode::Zero | LevelNode::SProp => l,
            LevelNode::Param(n) => match ps.iter().position(|p| *p == n) {
                Some(i) => us[i],
                None => l,
            },
            LevelNode::Succ(a) => {
                let a2 = self.instantiate(a, ps, us);
                self.succ(a2)
            }
            LevelNode::Max(a, b) => {
                let (a2, b2) = (self.instantiate(a, ps, us), self.instantiate(b, ps, us));
                if (a2, b2) == (a, b) {
                    l
                } else {
                    self.max(a2, b2)
                }
            }
            LevelNode::IMax(a, b) => {
                let (a2, b2) = (self.instantiate(a, ps, us), self.instantiate(b, ps, us));
                if (a2, b2) == (a, b) {
                    l
                } else {
                    self.imax(a2, b2)
                }
            }
        }
    }

    /// The substitution of the levels `us` for the parameters `ps` (pairwise; a parameter without
    /// a level is left as it is). Replacing each parameter by itself is the identity.
    pub fn substitution(&mut self, ps: &[NameId], us: LevelsId) -> SubstId {
        if ps.is_empty() {
            return SubstId::IDENTITY;
        }
        let own: Vec<LevelId> = ps.iter().map(|p| self.param(*p)).collect();
        let own = self.list(&own);
        if own == us {
            return SubstId::IDENTITY;
        }
        if let Some(&s) = self.subst_index.get(&(own, us)) {
            return s;
        }
        let n = ps.len().min(self.get_list(us).len());
        let id = SubstId(self.substs.len() as u32);
        self.substs.push(Subst {
            ps: ps[..n].into(),
            us: self.get_list(us)[..n].into(),
        });
        self.subst_index.insert((own, us), id);
        id
    }

    /// `l` under the substitution `s` — exactly [`Levels::instantiate`] with its parameters and
    /// levels, remembered.
    pub fn subst(&mut self, l: LevelId, s: SubstId) -> LevelId {
        if s == SubstId::IDENTITY || !self.has_param(l) {
            return l;
        }
        if let Some(&r) = self.subst_memo.get(&(l, s)) {
            return r;
        }
        let sub = &self.substs[s.0 as usize];
        let (ps, us) = (sub.ps.clone(), sub.us.clone());
        let r = self.instantiate(l, &ps, &us);
        self.subst_memo.insert((l, s), r);
        r
    }

    /// Every level of `ls` under the substitution `s`.
    pub fn subst_list(&mut self, ls: LevelsId, s: SubstId) -> LevelsId {
        if s == SubstId::IDENTITY {
            return ls;
        }
        if let Some(&r) = self.subst_list_memo.get(&(ls, s)) {
            return r;
        }
        let old = self.get_list(ls).to_vec();
        let new: Vec<LevelId> = old.iter().map(|l| self.subst(*l, s)).collect();
        let r = self.list(&new);
        self.subst_list_memo.insert((ls, s), r);
        r
    }

    /// Whether `SProp` occurs anywhere in `l`.
    pub fn mentions_sprop(&self, l: LevelId) -> bool {
        if self.sprop.is_none() {
            return false;
        }
        match self.node(l) {
            LevelNode::SProp => true,
            LevelNode::Zero | LevelNode::Param(_) => false,
            LevelNode::Succ(a) => self.mentions_sprop(a),
            LevelNode::Max(a, b) | LevelNode::IMax(a, b) => {
                self.mentions_sprop(a) || self.mentions_sprop(b)
            }
        }
    }

    /// Whether `l` mentions the parameter `p`.
    pub fn mentions_param(&self, l: LevelId, p: NameId) -> bool {
        if !self.has_param(l) {
            return false;
        }
        match self.node(l) {
            LevelNode::Zero | LevelNode::SProp => false,
            LevelNode::Param(n) => n == p,
            LevelNode::Succ(a) => self.mentions_param(a, p),
            LevelNode::Max(a, b) | LevelNode::IMax(a, b) => {
                self.mentions_param(a, p) || self.mentions_param(b, p)
            }
        }
    }

    /// The first parameter of `l` not listed in `ps`.
    pub fn undef_param(&self, l: LevelId, ps: &[NameId]) -> Option<NameId> {
        if !self.has_param(l) {
            return None;
        }
        match self.node(l) {
            LevelNode::Zero | LevelNode::SProp => None,
            LevelNode::Param(n) => (!ps.contains(&n)).then_some(n),
            LevelNode::Succ(a) => self.undef_param(a, ps),
            LevelNode::Max(a, b) | LevelNode::IMax(a, b) => {
                self.undef_param(a, ps).or_else(|| self.undef_param(b, ps))
            }
        }
    }

    /// `u ≤ v` under every assignment of the parameters.
    pub fn leq(&mut self, u: LevelId, v: LevelId) -> bool {
        if u == v {
            return true;
        }
        if self.sprop.is_some() {
            if self.is_sprop(u) {
                return true;
            }
            if self.is_sprop(v) {
                return false;
            }
        }
        if u == LevelId::ZERO {
            return true;
        }
        if let Some(&r) = self.leq_memo.get(&(u, v)) {
            return r;
        }
        let mut vars = Vec::new();
        let a = self.tree(u, &mut vars);
        let b = self.tree(v, &mut vars);
        let r = decide_leq(&a, &b);
        self.leq_memo.insert((u, v), r);
        r
    }

    pub fn equiv(&mut self, u: LevelId, v: LevelId) -> bool {
        u == v || (self.leq(u, v) && self.leq(v, u))
    }

    pub fn equiv_lists(&mut self, a: LevelsId, b: LevelsId) -> bool {
        if a == b {
            return true;
        }
        let (xs, ys) = (self.get_list(a).to_vec(), self.get_list(b).to_vec());
        xs.len() == ys.len() && xs.iter().zip(&ys).all(|(x, y)| self.equiv(*x, *y))
    }

    /// `l` is zero under every assignment of its parameters. Exact, and structural: a successor
    /// never is, a parameter may be one, `max a b` is exactly when both are, `imax a b` exactly
    /// when `b` is (it is `0` when `b` is and at least `b` otherwise).
    pub fn is_always_zero(&self, l: LevelId) -> bool {
        match self.node(l) {
            LevelNode::Zero | LevelNode::SProp => true,
            LevelNode::Succ(_) | LevelNode::Param(_) => false,
            LevelNode::Max(a, b) => self.is_always_zero(a) && self.is_always_zero(b),
            LevelNode::IMax(_, b) => self.is_always_zero(b),
        }
    }

    fn tree(&self, l: LevelId, vars: &mut Vec<NameId>) -> T {
        match self.node(l) {
            LevelNode::Zero | LevelNode::SProp => T::Zero,
            LevelNode::Succ(a) => T::Succ(Box::new(self.tree(a, vars))),
            LevelNode::Max(a, b) => {
                T::Max(Box::new(self.tree(a, vars)), Box::new(self.tree(b, vars)))
            }
            LevelNode::IMax(a, b) => {
                T::IMax(Box::new(self.tree(a, vars)), Box::new(self.tree(b, vars)))
            }
            LevelNode::Param(n) => {
                let i = vars.iter().position(|v| *v == n).unwrap_or_else(|| {
                    vars.push(n);
                    vars.len() - 1
                });
                T::Var(i as u32)
            }
        }
    }
}

/// A level over numbered variables, the working form of the decision procedure.
#[derive(Clone, Debug, PartialEq)]
enum T {
    Zero,
    Succ(Box<T>),
    Max(Box<T>, Box<T>),
    IMax(Box<T>, Box<T>),
    Var(u32),
}

impl T {
    fn never_zero(&self) -> bool {
        match self {
            T::Zero | T::Var(_) => false,
            T::Succ(_) => true,
            T::Max(a, b) => a.never_zero() || b.never_zero(),
            T::IMax(_, b) => b.never_zero(),
        }
    }

    fn is_zero(&self) -> bool {
        match self {
            T::Zero => true,
            T::Succ(_) | T::Var(_) => false,
            T::Max(a, b) => a.is_zero() && b.is_zero(),
            T::IMax(_, b) => b.is_zero(),
        }
    }

    /// Resolves every `imax` whose right side is decidedly zero or nonzero.
    fn simp(self) -> T {
        match self {
            T::Succ(a) => T::Succ(Box::new(a.simp())),
            T::Max(a, b) => T::Max(Box::new(a.simp()), Box::new(b.simp())),
            T::IMax(a, b) => {
                let b = b.simp();
                if b.is_zero() {
                    T::Zero
                } else if b.never_zero() {
                    T::Max(Box::new(a.simp()), Box::new(b))
                } else {
                    T::IMax(Box::new(a.simp()), Box::new(b))
                }
            }
            t => t,
        }
    }

    /// A variable whose vanishing would make some remaining `imax` vanish.
    fn split_candidate(&self) -> Option<u32> {
        match self {
            T::Zero | T::Var(_) => None,
            T::Succ(a) => a.split_candidate(),
            T::Max(a, b) => a.split_candidate().or_else(|| b.split_candidate()),
            T::IMax(a, b) => b
                .zero_relevant()
                .or_else(|| a.split_candidate())
                .or_else(|| b.split_candidate()),
        }
    }

    fn zero_relevant(&self) -> Option<u32> {
        match self {
            T::Var(v) => Some(*v),
            T::Zero | T::Succ(_) => None,
            T::Max(a, b) => a.zero_relevant().or_else(|| b.zero_relevant()),
            T::IMax(_, b) => b.zero_relevant(),
        }
    }

    fn subst(&self, v: u32, to_zero: bool) -> T {
        match self {
            T::Var(w) if *w == v => {
                if to_zero {
                    T::Zero
                } else {
                    T::Succ(Box::new(T::Var(v)))
                }
            }
            T::Succ(a) => T::Succ(Box::new(a.subst(v, to_zero))),
            T::Max(a, b) => T::Max(Box::new(a.subst(v, to_zero)), Box::new(b.subst(v, to_zero))),
            T::IMax(a, b) => T::IMax(Box::new(a.subst(v, to_zero)), Box::new(b.subst(v, to_zero))),
            t => t.clone(),
        }
    }

    /// `max(c, vᵢ + kᵢ …)` for an `imax`-free level: (constant, per-variable offsets).
    fn flatten(&self, k: u64, c: &mut u64, vars: &mut Vec<(u32, u64)>) {
        match self {
            T::Zero => *c = (*c).max(k),
            T::Succ(a) => a.flatten(k + 1, c, vars),
            T::Max(a, b) => {
                a.flatten(k, c, vars);
                b.flatten(k, c, vars);
            }
            T::Var(v) => {
                *c = (*c).max(k);
                match vars.iter_mut().find(|(w, _)| w == v) {
                    Some(e) => e.1 = e.1.max(k),
                    None => vars.push((*v, k)),
                }
            }
            T::IMax(..) => unreachable!("flatten requires an imax-free level"),
        }
    }
}

fn decide_leq(a: &T, b: &T) -> bool {
    let (a, b) = (a.clone().simp(), b.clone().simp());
    if let Some(v) = a.split_candidate().or_else(|| b.split_candidate()) {
        return decide_leq(&a.subst(v, true), &b.subst(v, true))
            && decide_leq(&a.subst(v, false), &b.subst(v, false));
    }
    let (mut ca, mut va) = (0, Vec::new());
    a.flatten(0, &mut ca, &mut va);
    let (mut cb, mut vb) = (0, Vec::new());
    b.flatten(0, &mut cb, &mut vb);
    let floor_b = vb.iter().fold(cb, |m, (_, k)| m.max(*k));
    ca <= floor_b
        && va
            .iter()
            .all(|(v, k)| vb.iter().any(|(w, j)| w == v && j >= k))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/level/tests.rs"]
mod tests;
