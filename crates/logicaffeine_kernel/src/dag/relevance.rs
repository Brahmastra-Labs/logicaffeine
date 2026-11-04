//! Argument relevance: which arguments of a definition cannot change what it computes.
//!
//! When two applications of the same definition are compared, Lean's algorithm first tries the
//! arguments pairwise and only unfolds on failure. Comparing an argument the definition never
//! uses can cost arbitrarily much (Arena `perf/discarded-argument`: two numerals evaluated in
//! Θ(n²) only to be thrown away). An argument position `i` is *ignorable* when either
//!
//! * the definition's value, after its leading `fun`s, never mentions parameter `i`, and the rest
//!   of its type does not depend on it — unfolding substitutes into a body with no occurrence of
//!   the parameter, so `c … aᵢ …` and `c … bᵢ …` unfold to the same term whenever the other
//!   arguments agree; or
//! * the parameter's type is a proposition — two proofs of definitionally equal propositions are
//!   definitionally equal (proof irrelevance), and the propositions are equal whenever the earlier
//!   arguments are.
//!
//! Both facts are properties of the settled constant (and, for proof-ness, its universe
//! arguments), so they are computed once and shared by every declaration.

use super::error::KResult;
use super::expr::{ExprId, Node};
use super::level::LevelsId;
use super::name::NameId;
use super::tc::Kernel;
use rustc_hash::FxHashMap as FxMap;

/// Positions beyond this are always compared.
pub const MAX_TRACKED: usize = 64;

/// Relevance facts, keyed on settled constants only.
#[derive(Clone, Default)]
pub struct Cache {
    absent: FxMap<NameId, u64>,
    proof: FxMap<(NameId, LevelsId), u64>,
    signatures: FxMap<(NameId, LevelsId), Signature>,
    /// Signatures being computed: one asked for again while its own computation runs (a
    /// reduction inside the telescope's walk that compares applications of the constant) is
    /// unknown for that inner question.
    pub(crate) computing: rustc_hash::FxHashSet<(NameId, LevelsId)>,
}

/// What a settled constant's type says about its applications, at one universe instantiation.
///
/// The type's telescope `Π (x₀ : A₀) … (xₙ₋₁ : Aₙ₋₁), B`, walked once under fresh variables,
/// gives the universe `uᵢ` of every binder type and `v` of the result. Universes do not depend on
/// terms, so these are the universes of every application's parts too: an argument at position
/// `i` is a proof exactly when `uᵢ` is zero, and an application to `k ≤ n` arguments, whose type
/// is `Π (xₖ : Aₖ) … , B`, lives in `imax(uₖ, … imax(uₙ₋₁, v))` — zero exactly when `v` is. So
/// whether an application is a proof is read off its head and its argument count, without
/// inferring a type. Past the telescope — a result type that becomes a Π only once arguments are
/// substituted — nothing is known.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Signature {
    proof_args: u64,
    /// Bit `k`: an application to `k` arguments is a proof (meaningful where `known` has bit `k`).
    proofs: u64,
    known: u64,
    /// The telescope's length when it ends within tracking, and — when its result `B` is a sort —
    /// whether that sort is `Prop` (the full application is then a proposition).
    full: Option<(u32, Option<bool>)>,
}

impl Signature {
    pub(crate) fn new(
        proof_args: u64,
        proofs: u64,
        known: u64,
        full: Option<(u32, Option<bool>)>,
    ) -> Signature {
        Signature {
            proof_args,
            proofs,
            known,
            full,
        }
    }

    /// The argument positions (below [`MAX_TRACKED`]) whose binder type is a proposition.
    pub fn proof_args(&self) -> u64 {
        self.proof_args
    }

    /// Whether an application to `k` arguments is a proof, if the telescope says.
    #[inline]
    pub fn is_proof(&self, k: usize) -> Option<bool> {
        (k < MAX_TRACKED && (self.known >> k) & 1 == 1).then(|| (self.proofs >> k) & 1 == 1)
    }

    /// Whether an application to `k` arguments is a proposition, if the telescope says: only a
    /// full application whose result is a sort is a type at all.
    #[inline]
    pub fn is_proposition(&self, k: usize) -> Option<bool> {
        match self.full {
            Some((n, p)) if n as usize == k => p,
            _ => None,
        }
    }
}

impl Kernel {
    /// The argument positions of `c.{ls}` whose arguments need not be compared (see the module
    /// documentation). Zero for anything that is not a settled, unfoldable definition.
    pub fn ignorable_args(&mut self, c: NameId, ls: LevelsId) -> KResult<u64> {
        let Some(absent) = self.absent_mask(c, ls) else {
            return Ok(0);
        };
        let proof = match self.relevance.proof.get(&(c, ls)) {
            Some(m) => *m,
            None => {
                let k = self.env.get(c).cloned().expect("an unfoldable definition");
                let ty = self.instantiate_type_lparams(&k, ls);
                let m = self.proof_args(ty)?;
                self.relevance.proof.insert((c, ls), m);
                m
            }
        };
        Ok(absent | proof)
    }

    /// The positions `c`'s value never mentions, or `None` when `c.{ls}` is not a settled,
    /// unfoldable definition (whose arguments must then all be compared).
    pub fn absent_mask(&mut self, c: NameId, ls: LevelsId) -> Option<u64> {
        if !self.env.is_settled(c) {
            return None;
        }
        let k = self.env.get(c)?.clone();
        let value = k.delta_value()?;
        if self.s.levels.get_list(ls).len() != k.lparams.len() {
            return None;
        }
        Some(match self.relevance.absent.get(&c) {
            Some(m) => *m,
            None => {
                let m = self.absent_args(k.ty, value);
                self.relevance.absent.insert(c, m);
                m
            }
        })
    }

    /// The signature of `c.{ls}`, if some engine has already computed it.
    #[inline]
    pub(crate) fn known_signature(&self, c: NameId, ls: LevelsId) -> Option<Signature> {
        self.relevance.signatures.get(&(c, ls)).copied()
    }

    /// Records the signature of the settled constant `c.{ls}` (and its proof positions).
    pub(crate) fn note_signature(&mut self, c: NameId, ls: LevelsId, s: Signature) {
        self.relevance.signatures.insert((c, ls), s);
        self.relevance.proof.entry((c, ls)).or_insert(s.proof_args);
    }

    /// Parameters the value never mentions and the rest of the type does not depend on.
    fn absent_args(&mut self, ty: ExprId, value: ExprId) -> u64 {
        let mut body = value;
        let mut arity = 0usize;
        while let Node::Lam(_, b) = self.s.node(body) {
            if arity == MAX_TRACKED {
                break;
            }
            body = b;
            arity += 1;
        }
        if arity == 0 {
            return 0;
        }
        let used = loose_mask(self, body);
        let mut absent = 0u64;
        let mut rest = ty;
        for i in 0..arity {
            let Node::Pi(_, cod) = self.s.node(rest) else {
                break;
            };
            let unused = (used >> (arity - 1 - i)) & 1 == 0;
            let independent = !mentions_bvar0(self, cod);
            if unused && independent {
                absent |= 1 << i;
            }
            rest = cod;
        }
        absent
    }

    /// Parameters whose type is a proposition, over the instantiated type's telescope.
    fn proof_args(&mut self, ty: ExprId) -> KResult<u64> {
        let mut mask = 0u64;
        let mut cur = ty;
        for i in 0..MAX_TRACKED {
            let w = self.whnf(cur)?;
            let Node::Pi(d, cod) = self.s.node(w) else {
                break;
            };
            if self.is_prop(d)? {
                mask |= 1 << i;
            }
            let fv = self.push_local(d, None);
            cur = self.s.instantiate1(cod, fv);
        }
        Ok(mask)
    }
}

/// The loose de Bruijn indices below 64 occurring in `e`: the store's mask, which records them
/// exactly unless an index of 64 or more occurs somewhere in `e`, and then a walk.
fn loose_mask(k: &Kernel, e: ExprId) -> u64 {
    match k.s.loose_bvar_mask(e) {
        Some(m) => m,
        None => loose_mask_walk(k, e, 0, &mut FxMap::default()),
    }
}

/// The loose de Bruijn indices (relative to `depth`, below 64) occurring in `e`, found by
/// walking it.
fn loose_mask_walk(k: &Kernel, e: ExprId, depth: u32, memo: &mut FxMap<(ExprId, u32), u64>) -> u64 {
    if k.s.loose_bvar_range(e) <= depth {
        return 0;
    }
    if let Some(m) = memo.get(&(e, depth)) {
        return *m;
    }
    let m = match k.s.node(e) {
        Node::BVar(i) => {
            let j = i - depth;
            if (j as usize) < MAX_TRACKED {
                1u64 << j
            } else {
                0
            }
        }
        Node::App(a, b) => loose_mask_walk(k, a, depth, memo) | loose_mask_walk(k, b, depth, memo),
        Node::Lam(a, b) | Node::Pi(a, b) => {
            loose_mask_walk(k, a, depth, memo) | loose_mask_walk(k, b, depth + 1, memo)
        }
        Node::Let(a, b, c) => {
            loose_mask_walk(k, a, depth, memo)
                | loose_mask_walk(k, b, depth, memo)
                | loose_mask_walk(k, c, depth + 1, memo)
        }
        Node::Proj(_, _, a) => loose_mask_walk(k, a, depth, memo),
        _ => 0,
    };
    memo.insert((e, depth), m);
    m
}

/// Does `e` mention the innermost enclosing binder (`BVar(0)` at depth 0)?
fn mentions_bvar0(k: &Kernel, e: ExprId) -> bool {
    loose_mask(k, e) & 1 == 1
}
