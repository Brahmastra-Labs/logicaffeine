//! Semantic values: what an expression means once its variables are bound.
//!
//! A value never contains a substitution. A lambda is a *closure* — its body expression paired
//! with the environment its free de Bruijn indices refer to — and applying it extends that
//! environment. An application whose head cannot reduce is a *neutral*: a head and a spine of
//! eliminations. A delta-reducible constant applied to arguments is *glued*: it stays a neutral
//! with a known head (so two applications of the same definition can be compared argument by
//! argument, the way Lean's lazy delta reduction does), while its unfolding is computed only on
//! demand and then remembered. Arguments are *thunks* — evaluated at most once, and only if
//! something looks at them.
//!
//! A binder is crossed with a *local*: a variable named by the binder depth it is bound at and its
//! type. Crossing a binder of the same type at the same depth again yields the same local, so
//! everything built beneath it — values, weak-head normal forms, conversion answers — is shared by
//! every crossing, as the substitution engine shares the terms built under a binder by
//! hash-consing them. Two locals in scope at once have different depths; a value's meaning depends
//! only on the locals it mentions and their types, so a cached fact about it holds wherever it is
//! rebuilt.
//!
//! A closure, a thunk or a cached evaluation keeps of its environment exactly what its expression
//! can see: the entries at the loose indices the expression mentions, every other entry replaced
//! by [`DEAD`]. So two occurrences of one expression under binders whose other variables differ
//! are one value, as they are one term to the substitution engine (a term mentions no variable it
//! does not use).
//!
//! All values of one declaration live in an arena addressed by `u32` ids and are dropped together
//! when the declaration is done.

use crate::dag::expr::{ExprId, NatId, StrId};
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;
use rustc_hash::FxHashMap as FxMap;
use std::rc::Rc;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct VId(pub u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Sp(pub u32);

impl Sp {
    pub const NIL: Sp = Sp(0);
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct EnvId(pub u32);

impl EnvId {
    pub const EMPTY: EnvId = EnvId(0);
}

/// A checking context: the types of the enclosing bound variables, innermost first.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct CtxId(pub u32);

impl CtxId {
    pub const EMPTY: CtxId = CtxId(0);
}

#[derive(Copy, Clone, Debug)]
struct CtxNode {
    parent: CtxId,
    ty: VId,
    len: u32,
}

/// What a constant is, as far as reduction is concerned.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ConstKind {
    /// Axioms, opaque definitions, inductive types: never reduce as a head.
    Opaque,
    Ctor,
    Rec,
    Quot,
}

/// The head of a neutral value.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Head {
    /// A variable bound during conversion or type computation: an index into [`Arena::locals`].
    Local(u32),
    /// A free variable of the classic checker's local context.
    FVar(u32),
    Const(ConstKind, NameId, LevelsId),
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Elim {
    App(VId),
    Proj(NameId, u32),
}

#[derive(Copy, Clone, Debug)]
pub enum Val {
    Neutral {
        head: Head,
        spine: Sp,
    },
    /// A δ-reducible constant (definition or theorem) applied to `spine`.
    Glued {
        name: NameId,
        levels: LevelsId,
        spine: Sp,
    },
    Lam {
        dom: ExprId,
        env: EnvId,
        body: ExprId,
    },
    Pi {
        dom: VId,
        env: EnvId,
        body: ExprId,
    },
    /// The type of a lambda: `Π (x : dom), type_of(body[x])`, its codomain computed on demand.
    PiOfLam {
        dom: VId,
        env: EnvId,
        body: ExprId,
    },
    Sort(LevelId),
    Nat(NatId),
    Str(StrId),
    Thunk {
        env: EnvId,
        expr: ExprId,
    },
}

/// A local: the binder depth it is bound at, its type, and its value.
#[derive(Copy, Clone, Debug)]
pub struct Local {
    pub level: u32,
    pub ty: VId,
    pub val: VId,
}

#[derive(Copy, Clone, Debug)]
pub struct EnvNode {
    pub parent: EnvId,
    pub val: VId,
    pub len: u32,
    pub lsub: u32,
}

/// A universe-level substitution carried by an environment (a polymorphic constant's body is
/// evaluated with its level parameters bound, instead of being copied with them replaced).
#[derive(Clone, Debug)]
pub struct LSub {
    pub params: Rc<[NameId]>,
    pub levels: LevelsId,
}

const UNFORCED: u32 = u32::MAX;

/// The placeholder for an environment entry (or a context type) that the expression it is kept
/// for never mentions: value 0 of every arena, closed and inert — a neutral headed by the
/// anonymous constant, which no declaration can be.
pub const DEAD: VId = VId(0);

fn dead() -> Val {
    Val::Neutral {
        head: Head::Const(ConstKind::Opaque, NameId::ANON, LevelsId::EMPTY),
        spine: Sp::NIL,
    }
}

/// The arena of one declaration's values, spines and environments.
///
/// Environments, spines, thunks and neutral applications are hash-consed: building the same one
/// twice yields the same id. Two values with the same id are the same value, so every cache keyed
/// on ids (evaluation, weak-head normal forms, types, conversion answers) is shared between all
/// the places a value is rebuilt, the way the term store shares terms.
#[derive(Clone)]
pub struct Arena {
    vals: Vec<Val>,
    /// A thunk's forced value / a glued value's unfolding, once computed.
    memo: Vec<u32>,
    spines: Vec<(Sp, Elim)>,
    envs: Vec<EnvNode>,
    ctxs: Vec<CtxNode>,
    pub lsubs: Vec<LSub>,
    /// Every `Head::Local`, one per (depth, type).
    pub locals: Vec<Local>,
    local_index: FxMap<(u32, VId), u32>,
    env_index: FxMap<(EnvId, VId), EnvId>,
    lsub_index: FxMap<(Box<[NameId]>, LevelsId), EnvId>,
    scope_index: FxMap<(EnvId, u32, u64), EnvId>,
    ctx_index: FxMap<(CtxId, VId), CtxId>,
    ctx_scope_index: FxMap<(CtxId, u32, u64), CtxId>,
    spine_index: FxMap<(Sp, Elim), Sp>,
    thunk_index: FxMap<(EnvId, ExprId), VId>,
    app_index: FxMap<(VId, VId), VId>,
    /// The entries of the environment or context being scoped, innermost first.
    scope_scratch: Vec<VId>,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    pub fn new() -> Arena {
        Arena {
            vals: vec![dead()],
            memo: vec![UNFORCED],
            spines: vec![(Sp::NIL, Elim::Proj(NameId::ANON, 0))],
            envs: vec![EnvNode {
                parent: EnvId::EMPTY,
                val: VId(u32::MAX),
                len: 0,
                lsub: 0,
            }],
            ctxs: vec![CtxNode {
                parent: CtxId::EMPTY,
                ty: VId(u32::MAX),
                len: 0,
            }],
            lsubs: vec![LSub {
                params: Rc::from(Vec::new()),
                levels: LevelsId::EMPTY,
            }],
            locals: Vec::new(),
            local_index: FxMap::default(),
            env_index: FxMap::default(),
            lsub_index: FxMap::default(),
            scope_index: FxMap::default(),
            ctx_index: FxMap::default(),
            ctx_scope_index: FxMap::default(),
            spine_index: FxMap::default(),
            thunk_index: FxMap::default(),
            app_index: FxMap::default(),
            scope_scratch: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.vals.truncate(1);
        self.memo.truncate(1);
        self.spines.truncate(1);
        self.envs.truncate(1);
        self.ctxs.truncate(1);
        self.lsubs.truncate(1);
        self.locals.clear();
        self.local_index.clear();
        self.env_index.clear();
        self.lsub_index.clear();
        self.scope_index.clear();
        self.ctx_index.clear();
        self.ctx_scope_index.clear();
        self.spine_index.clear();
        self.thunk_index.clear();
        self.app_index.clear();
    }

    pub fn len(&self) -> usize {
        self.vals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vals.is_empty()
    }

    #[inline]
    pub fn alloc(&mut self, v: Val) -> VId {
        let id = VId(self.vals.len() as u32);
        self.vals.push(v);
        self.memo.push(UNFORCED);
        id
    }

    #[inline]
    pub fn get(&self, v: VId) -> Val {
        self.vals[v.0 as usize]
    }

    #[inline]
    pub fn memo(&self, v: VId) -> Option<VId> {
        let m = self.memo[v.0 as usize];
        (m != UNFORCED).then_some(VId(m))
    }

    #[inline]
    pub fn set_memo(&mut self, v: VId, r: VId) {
        self.memo[v.0 as usize] = r.0;
    }

    #[inline]
    pub fn snoc(&mut self, prev: Sp, e: Elim) -> Sp {
        if let Some(s) = self.spine_index.get(&(prev, e)) {
            return *s;
        }
        let id = Sp(self.spines.len() as u32);
        self.spines.push((prev, e));
        self.spine_index.insert((prev, e), id);
        id
    }

    /// The local bound at depth `level` with type `ty`.
    pub fn local(&mut self, level: u32, ty: VId) -> VId {
        if let Some(i) = self.local_index.get(&(level, ty)) {
            return self.locals[*i as usize].val;
        }
        let i = self.locals.len() as u32;
        let val = self.alloc(Val::Neutral {
            head: Head::Local(i),
            spine: Sp::NIL,
        });
        self.locals.push(Local { level, ty, val });
        self.local_index.insert((level, ty), i);
        val
    }

    /// The thunk of `expr` in `env` (`env` already scoped to what `expr` can see).
    pub fn thunk(&mut self, env: EnvId, expr: ExprId) -> VId {
        if let Some(v) = self.thunk_index.get(&(env, expr)) {
            return *v;
        }
        let v = self.alloc(Val::Thunk { env, expr });
        self.thunk_index.insert((env, expr), v);
        v
    }

    /// A neutral or glued application `f a`, shared with every other construction of it.
    pub fn app_value(&mut self, f: VId, a: VId, make: impl FnOnce(&mut Arena) -> Val) -> VId {
        if let Some(v) = self.app_index.get(&(f, a)) {
            return *v;
        }
        let val = make(self);
        let v = self.alloc(val);
        self.app_index.insert((f, a), v);
        v
    }

    #[inline]
    pub fn spine_node(&self, s: Sp) -> (Sp, Elim) {
        self.spines[s.0 as usize]
    }

    /// The eliminations of a spine, first to last.
    pub fn spine_elims(&self, mut s: Sp) -> Vec<Elim> {
        let mut out = Vec::new();
        while s != Sp::NIL {
            let (p, e) = self.spines[s.0 as usize];
            out.push(e);
            s = p;
        }
        out.reverse();
        out
    }

    /// The spine's arguments if it consists only of applications.
    pub fn spine_args(&self, s: Sp) -> Option<Vec<VId>> {
        let mut out = Vec::new();
        let mut s = s;
        while s != Sp::NIL {
            let (p, e) = self.spines[s.0 as usize];
            match e {
                Elim::App(a) => out.push(a),
                Elim::Proj(..) => return None,
            }
            s = p;
        }
        out.reverse();
        Some(out)
    }

    pub fn spine_len(&self, mut s: Sp) -> usize {
        let mut n = 0;
        while s != Sp::NIL {
            n += 1;
            s = self.spines[s.0 as usize].0;
        }
        n
    }

    #[inline]
    pub fn env(&self, e: EnvId) -> EnvNode {
        self.envs[e.0 as usize]
    }

    pub fn extend(&mut self, parent: EnvId, val: VId) -> EnvId {
        if let Some(e) = self.env_index.get(&(parent, val)) {
            return *e;
        }
        let p = self.envs[parent.0 as usize];
        let id = EnvId(self.envs.len() as u32);
        self.envs.push(EnvNode {
            parent,
            val,
            len: p.len + 1,
            lsub: p.lsub,
        });
        self.env_index.insert((parent, val), id);
        id
    }

    /// An empty environment carrying a universe substitution (one per distinct substitution).
    pub fn with_lsub(&mut self, params: Rc<[NameId]>, levels: LevelsId) -> EnvId {
        let key: (Box<[NameId]>, LevelsId) = (params.as_ref().into(), levels);
        if let Some(e) = self.lsub_index.get(&key) {
            return *e;
        }
        let li = self.lsubs.len() as u32;
        self.lsubs.push(LSub { params, levels });
        let id = EnvId(self.envs.len() as u32);
        self.envs.push(EnvNode {
            parent: EnvId::EMPTY,
            val: VId(u32::MAX),
            len: 0,
            lsub: li,
        });
        self.lsub_index.insert(key, id);
        id
    }

    /// `env` as an expression with loose-bound-variable range `n` and loose indices `mask` sees
    /// it: its innermost `n` entries, those at indices outside `mask` replaced by [`DEAD`], over
    /// its universe substitution. With no exact set (an index of 64 or more occurs), the innermost
    /// `n` entries as they are.
    pub fn scope(&mut self, env: EnvId, n: u32, mask: Option<u64>) -> EnvId {
        let node = self.envs[env.0 as usize];
        let mask = mask.unwrap_or(u64::MAX);
        let whole = n >= node.len && (n >= 64 || mask | !((1u64 << n) - 1) == u64::MAX);
        if whole || n > node.len {
            return env;
        }
        if let Some(e) = self.scope_index.get(&(env, n, mask)) {
            return *e;
        }
        let mut vals = std::mem::take(&mut self.scope_scratch);
        vals.clear();
        let mut cur = env;
        for i in 0..n {
            let c = self.envs[cur.0 as usize];
            vals.push(if i >= 64 || (mask >> i) & 1 == 1 {
                c.val
            } else {
                DEAD
            });
            cur = c.parent;
        }
        let base = if node.lsub == 0 {
            EnvId::EMPTY
        } else {
            self.lsub_base(node.lsub)
        };
        let mut r = base;
        for v in vals.iter().rev() {
            r = self.extend(r, *v);
        }
        self.scope_scratch = vals;
        self.scope_index.insert((env, n, mask), r);
        r
    }

    fn lsub_base(&mut self, li: u32) -> EnvId {
        let l = self.lsubs[li as usize].clone();
        self.with_lsub(l.params, l.levels)
    }

    /// `ctx` with one more binder, of type `ty` (one context per (parent, type)).
    pub fn ctx_extend(&mut self, parent: CtxId, ty: VId) -> CtxId {
        if let Some(c) = self.ctx_index.get(&(parent, ty)) {
            return *c;
        }
        let len = self.ctxs[parent.0 as usize].len + 1;
        let id = CtxId(self.ctxs.len() as u32);
        self.ctxs.push(CtxNode { parent, ty, len });
        self.ctx_index.insert((parent, ty), id);
        id
    }

    /// The type of de Bruijn index `i` in `ctx`.
    pub fn ctx_lookup(&self, mut ctx: CtxId, mut i: u32) -> Option<VId> {
        loop {
            let n = self.ctxs[ctx.0 as usize];
            if n.len == 0 {
                return None;
            }
            if i == 0 {
                return Some(n.ty);
            }
            i -= 1;
            ctx = n.parent;
        }
    }

    /// `ctx` as an expression with loose-bound-variable range `n` and loose indices `mask` sees
    /// it (see [`Arena::scope`]).
    pub fn ctx_scope(&mut self, ctx: CtxId, n: u32, mask: Option<u64>) -> CtxId {
        let len = self.ctxs[ctx.0 as usize].len;
        let mask = mask.unwrap_or(u64::MAX);
        let whole = n >= len && (n >= 64 || mask | !((1u64 << n) - 1) == u64::MAX);
        if whole || n > len {
            return ctx;
        }
        if let Some(c) = self.ctx_scope_index.get(&(ctx, n, mask)) {
            return *c;
        }
        let mut tys = std::mem::take(&mut self.scope_scratch);
        tys.clear();
        let mut cur = ctx;
        for i in 0..n {
            let c = self.ctxs[cur.0 as usize];
            tys.push(if i >= 64 || (mask >> i) & 1 == 1 {
                c.ty
            } else {
                DEAD
            });
            cur = c.parent;
        }
        let r = tys
            .iter()
            .rev()
            .fold(CtxId::EMPTY, |c, t| self.ctx_extend(c, *t));
        self.scope_scratch = tys;
        self.ctx_scope_index.insert((ctx, n, mask), r);
        r
    }

    /// The value bound to de Bruijn index `i`.
    pub fn lookup(&self, mut e: EnvId, i: u32) -> Option<VId> {
        let mut i = i;
        loop {
            let n = self.envs[e.0 as usize];
            if n.len == 0 {
                return None;
            }
            if i == 0 {
                return Some(n.val);
            }
            i -= 1;
            e = n.parent;
        }
    }
}
