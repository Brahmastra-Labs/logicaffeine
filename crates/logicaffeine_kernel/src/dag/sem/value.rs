//! Semantic values and the arena that interns them.
//!
//! A value is what an expression means once its bound variables have values: a closure for a
//! function or a Π, a neutral (a head and the arguments applied to it) for anything whose head
//! cannot reduce further by β, a sort, or a literal. Nothing in a value is ever substituted.
//!
//! Every value is interned. A neutral is built once per head and argument list, a variable once
//! per (level, type), a constant once per universe instantiation, an environment once per
//! (parent, value): so two values built the same way from the same parts are the same id, and a
//! result remembered for an id — its weak-head normal form, its type, a conversion answer —
//! serves every place that builds it again. The per-value memos are plain arrays indexed by id,
//! not hash maps.
//!
//! A neutral's arguments lie contiguously in the arena's spine slots: an application of a head to
//! all its arguments is built and found with one probe, and the `i`-th argument is read in place.

use super::table::Table;
use crate::dag::expr::{ExprId, NatId, StrId};
use crate::dag::level::{LevelId, LevelsId, SubstId};
use crate::dag::name::NameId;
use crate::dag::slot;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct V(pub u32);

/// An environment: the values of the enclosing bound variables, innermost first.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct EnvId(pub u32);

impl EnvId {
    pub const EMPTY: EnvId = EnvId(0);
}

/// The types of the enclosing bound variables, innermost first (for inference under binders).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct CtxId(pub u32);

impl CtxId {
    pub const EMPTY: CtxId = CtxId(0);
}

/// The arguments applied to a neutral head: `len` values stored contiguously from `start` in the
/// arena's spine slots. Every empty spine is [`Sp::NIL`].
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Sp {
    pub start: u32,
    pub len: u32,
}

impl Sp {
    pub const NIL: Sp = Sp { start: 0, len: 0 };

    #[inline]
    pub fn len(self) -> usize {
        self.len as usize
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct HeadId(pub u32);

/// What a neutral's head does under reduction.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    /// A variable bound by a binder being checked or compared.
    Local,
    /// A stuck projection `x.idx`.
    Proj,
    /// A definition or theorem: δ-reduces.
    Delta,
    /// An axiom, opaque definition or inductive type.
    Opaque,
    Ctor,
    Rec,
    Quot,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Head {
    Local { level: u32, ty: V },
    Const { name: NameId, levels: LevelsId },
    Proj { s: NameId, idx: u32, x: V },
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Val {
    Neu {
        kind: Kind,
        head: HeadId,
        spine: Sp,
    },
    /// `fun (x : dom) => body` closed over `env`; the domain is evaluated on demand.
    Lam {
        dom: ExprId,
        env: EnvId,
        body: ExprId,
    },
    Pi {
        dom: V,
        env: EnvId,
        body: ExprId,
    },
    /// The type of a lambda: `Π (x : dom), T` where `T` is the type inferred for `body` in `env`
    /// and `ctx` extended by `x` — inferred only when something asks for the codomain.
    PiLam {
        dom: V,
        env: EnvId,
        ctx: CtxId,
        body: ExprId,
    },
    Sort(LevelId),
    Nat(NatId),
    Str(StrId),
}

/// The value mentions a variable bound by a binder being checked or compared.
pub const OPEN: u8 = 1;
/// Whether the value is a proof has been decided; [`PROOF`] holds the answer.
const PROOF_KNOWN: u8 = 2;
const PROOF: u8 = 4;

const NONE: u32 = u32::MAX;

/// An environment node: either one entry consed onto a parent environment, or a *frame* — a
/// pruned environment of `len` entries holding only the entries an expression can see, stored
/// contiguously from `val` (an index into the frame slots). A narrow frame (every live index
/// below 64) finds an entry by its rank in `live`; a *wide* frame keeps its live indices as a
/// sorted list (`wide`, an index into the wide-index table) and finds an entry by search.
///
/// Every environment also carries the universe substitution its terms are read under (`lsub`):
/// a universe-polymorphic constant's type and body are evaluated from its own terms, under the
/// substitution of one use's levels for its parameters, without building an instantiated copy.
/// An extension inherits its parent's; the empty environment of each substitution is its base.
#[derive(Copy, Clone)]
pub struct EnvNode {
    pub parent: EnvId,
    pub val: V,
    pub len: u32,
    pub open: bool,
    pub frame: bool,
    /// Which of the innermost 64 entries are not [`DEAD`].
    pub live: u64,
    /// No entry at all is [`DEAD`].
    pub dense: bool,
    /// The two latest prunings of this environment, the latest first: the masks asked for (0
    /// for none: a pruning's mask is never empty) and the results.
    pub pmask: [u64; 2],
    pub pres: [u32; 2],
    /// The last extension of this environment: the value it was extended by and the result.
    pub xval: V,
    pub xres: EnvId,
    /// For a wide frame, where its index list starts in the wide-index table (count first).
    pub wide: u32,
    /// The universe substitution this environment's terms are read under.
    pub lsub: SubstId,
}

/// The placeholder for an environment entry an expression cannot see: value 0 of every session,
/// closed and inert (a neutral headed by the anonymous constant, which no declaration can be).
pub const DEAD: V = V(0);

#[derive(Copy, Clone)]
pub struct CtxNode {
    pub parent: CtxId,
    pub ty: V,
    pub open: bool,
}

/// A value and what is remembered about it, in one record: a value is created by one push, and
/// the memos reduction and inference consult sit beside it. A memo is [`NONE`] until set.
#[derive(Copy, Clone)]
struct Rec {
    val: Val,
    flags: u8,
    whnf: u32,
    whnf_core: u32,
    delta: u32,
    ty: u32,
    /// A lambda's evaluated domain.
    dom: u32,
}

/// All values of a checking session, their memos, and the tables that intern them.
#[derive(Clone)]
pub struct Arena {
    recs: Vec<Rec>,
    heads: Vec<Head>,
    spine_slots: Vec<V>,
    envs: Vec<EnvNode>,
    frame_slots: Vec<V>,
    frame_index: ContentTable,
    wide_idx: Vec<u32>,
    base_index: Table<(u32, u32)>,
    ctxs: Vec<CtxNode>,
    neutral_index: ContentTable,
    head_index: Table<(u32, u32, u32, u32)>,
    env_index: Table<(u32, u32)>,
    ctx_index: Table<(u32, u32)>,
    atom_index: Table<(u32, u32)>,
    lam_index: Table<(u32, u32, u32)>,
    pi_index: Table<(u32, u32, u32)>,
    pilam_index: Table<(u32, u32, u32, u32)>,
}

/// A head as a table key.
#[inline]
fn head_key(h: &Head) -> (u32, u32, u32, u32) {
    match *h {
        Head::Local { level, ty } => (0, level, ty.0, 0),
        Head::Const { name, levels } => (1, name.0, levels.0, 0),
        Head::Proj { s, idx, x } => (2, s.0, idx, x.0),
    }
}

#[inline]
fn frame_hash(len: u32, mask: u64, vals: &[V]) -> u64 {
    let k = 0x9E37_79B9_7F4A_7C15u64;
    let mut h = (mask ^ ((len as u64) << 58)).wrapping_mul(k);
    for v in vals {
        h = (h ^ v.0 as u64).wrapping_mul(k).rotate_left(29);
    }
    h.wrapping_mul(k)
}

/// What a universe substitution adds to a frame's hash.
#[inline]
fn lsub_hash(lsub: SubstId) -> u64 {
    (lsub.0 as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93)
}

/// The hash of a neutral: its head and its arguments.
#[inline]
fn neutral_hash(head: HeadId, args: &[V]) -> u64 {
    let k = 0x9E37_79B9_7F4A_7C15u64;
    let mut h = ((head.0 as u64) ^ ((args.len() as u64) << 40)).wrapping_mul(k);
    for v in args {
        h = (h ^ v.0 as u64).wrapping_mul(k).rotate_left(29);
    }
    h.wrapping_mul(k)
}

/// Ids interned by content (frames, neutrals): open addressing over ids, probed by a caller's
/// equality test (the content lives in the arena, not the table). A slot keeps the top half of
/// its entry's hash: all that places the entry (the table indexes by the hash's top bits, and
/// never outgrows 2^32 slots), and a filter that leaves the content compare to entries whose
/// hash agrees in 32 bits.
#[derive(Clone)]
struct ContentTable {
    slots: Vec<(u32, u32)>,
    shift: u32,
    len: usize,
}

impl ContentTable {
    /// A vacant slot: every bit set, so that emptying the table is one byte fill.
    const VACANT: (u32, u32) = (u32::MAX, NONE);

    fn new() -> ContentTable {
        ContentTable {
            slots: vec![Self::VACANT; 1 << 10],
            shift: 64 - 10,
            len: 0,
        }
    }

    fn clear(&mut self) {
        if self.len > 0 {
            self.slots.fill(Self::VACANT);
            self.len = 0;
        }
    }

    #[inline]
    fn find(&self, h: u64, same: impl Fn(u32) -> bool) -> Option<u32> {
        let mask = self.slots.len() - 1;
        let top = (h >> 32) as u32;
        let mut i = (h >> self.shift) as usize;
        loop {
            let (t, e) = *slot::at(&self.slots, i);
            if e == NONE {
                return None;
            }
            if t == top && same(e) {
                return Some(e);
            }
            i = (i + 1) & mask;
        }
    }

    fn insert(&mut self, h: u64, e: u32) {
        if (self.len + 1) * 2 > self.slots.len() {
            let cap = self.slots.len() * 2;
            assert!(cap as u64 <= 1 << 32, "a content table outgrew the hash half it keeps");
            let old = std::mem::replace(&mut self.slots, vec![Self::VACANT; cap]);
            self.shift = 64 - cap.trailing_zeros();
            for (ot, oe) in old {
                if oe != NONE {
                    self.place((ot as u64) << 32, oe);
                }
            }
        }
        self.place(h, e);
        self.len += 1;
    }

    fn place(&mut self, h: u64, e: u32) {
        let mask = self.slots.len() - 1;
        let mut i = (h >> self.shift) as usize;
        while slot::at(&self.slots, i).1 != NONE {
            i = (i + 1) & mask;
        }
        *slot::at_mut(&mut self.slots, i) = ((h >> 32) as u32, e);
    }

    fn bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<(u32, u32)>()
    }
}

/// The arguments of `s` among the spine slots.
#[inline]
fn spine_of(slots: &[V], s: Sp) -> &[V] {
    slot::run(slots, s.start as usize, s.len())
}

const ATOM_SORT: u32 = 0;
const ATOM_NAT: u32 = 1;
const ATOM_STR: u32 = 2;

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    pub fn new() -> Arena {
        let mut a = Arena {
            recs: Vec::new(),
            heads: Vec::new(),
            spine_slots: Vec::new(),
            envs: vec![EnvNode {
                parent: EnvId::EMPTY,
                val: V(NONE),
                len: 0,
                open: false,
                frame: false,
                live: 0,
                dense: true,
                pmask: [0; 2],
                pres: [NONE; 2],
                xval: V(NONE),
                xres: EnvId::EMPTY,
                wide: NONE,
                lsub: SubstId::IDENTITY,
            }],
            frame_slots: Vec::new(),
            frame_index: ContentTable::new(),
            wide_idx: Vec::new(),
            base_index: Table::new(),
            ctxs: vec![CtxNode {
                parent: CtxId::EMPTY,
                ty: V(NONE),
                open: false,
            }],
            neutral_index: ContentTable::new(),
            head_index: Table::new(),
            env_index: Table::new(),
            ctx_index: Table::new(),
            atom_index: Table::new(),
            lam_index: Table::new(),
            pi_index: Table::new(),
            pilam_index: Table::new(),
        };
        a.reserve_dead();
        a
    }

    fn reserve_dead(&mut self) {
        let dead = self.head_value(
            Kind::Opaque,
            Head::Const {
                name: NameId::ANON,
                levels: LevelsId::EMPTY,
            },
        );
        debug_assert_eq!(dead, DEAD);
    }

    /// Drops every value (the end of a session), keeping the storage for the next session.
    pub fn reset(&mut self) {
        self.recs.clear();
        self.heads.clear();
        self.spine_slots.clear();
        self.envs.truncate(1);
        let root = slot::at_mut(&mut self.envs, 0);
        root.xval = V(NONE);
        root.xres = EnvId::EMPTY;
        self.frame_slots.clear();
        self.frame_index.clear();
        self.wide_idx.clear();
        self.base_index.clear();
        self.ctxs.truncate(1);
        self.neutral_index.clear();
        self.head_index.clear();
        self.env_index.clear();
        self.ctx_index.clear();
        self.atom_index.clear();
        self.lam_index.clear();
        self.pi_index.clear();
        self.pilam_index.clear();
        self.reserve_dead();
    }

    pub fn len(&self) -> usize {
        self.recs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.recs.is_empty()
    }

    fn push(&mut self, v: Val, flags: u8) -> V {
        let id = V(self.recs.len() as u32);
        self.recs.push(Rec {
            val: v,
            flags,
            whnf: NONE,
            whnf_core: NONE,
            delta: NONE,
            ty: NONE,
            dom: NONE,
        });
        id
    }

    #[inline]
    pub fn get(&self, v: V) -> Val {
        slot::at(&self.recs, v.0 as usize).val
    }

    #[inline]
    pub fn is_open(&self, v: V) -> bool {
        slot::at(&self.recs, v.0 as usize).flags & OPEN != 0
    }

    #[inline]
    pub fn head(&self, h: HeadId) -> Head {
        *slot::at(&self.heads, h.0 as usize)
    }

    // ---- memos ----

    #[inline]
    fn memo(r: u32) -> Option<V> {
        (r != NONE).then_some(V(r))
    }

    #[inline]
    pub fn whnf_memo(&self, v: V) -> Option<V> {
        Self::memo(slot::at(&self.recs, v.0 as usize).whnf)
    }
    #[inline]
    pub fn set_whnf_memo(&mut self, v: V, r: V) {
        slot::at_mut(&mut self.recs, v.0 as usize).whnf = r.0;
    }
    #[inline]
    pub fn whnf_core_memo(&self, v: V) -> Option<V> {
        Self::memo(slot::at(&self.recs, v.0 as usize).whnf_core)
    }
    #[inline]
    pub fn set_whnf_core_memo(&mut self, v: V, r: V) {
        slot::at_mut(&mut self.recs, v.0 as usize).whnf_core = r.0;
    }
    #[inline]
    pub fn delta_memo(&self, v: V) -> Option<V> {
        Self::memo(slot::at(&self.recs, v.0 as usize).delta)
    }
    #[inline]
    pub fn set_delta_memo(&mut self, v: V, r: V) {
        slot::at_mut(&mut self.recs, v.0 as usize).delta = r.0;
    }
    #[inline]
    pub fn type_memo(&self, v: V) -> Option<V> {
        Self::memo(slot::at(&self.recs, v.0 as usize).ty)
    }
    #[inline]
    pub fn set_type_memo(&mut self, v: V, r: V) {
        slot::at_mut(&mut self.recs, v.0 as usize).ty = r.0;
    }
    /// Whether the value is a proof, once decided.
    #[inline]
    pub fn proof_memo(&self, v: V) -> Option<bool> {
        let f = slot::at(&self.recs, v.0 as usize).flags;
        (f & PROOF_KNOWN != 0).then_some(f & PROOF != 0)
    }
    #[inline]
    pub fn set_proof_memo(&mut self, v: V, proof: bool) {
        slot::at_mut(&mut self.recs, v.0 as usize).flags |=
            PROOF_KNOWN | if proof { PROOF } else { 0 };
    }
    /// A lambda's evaluated domain.
    #[inline]
    pub fn dom_memo(&self, v: V) -> Option<V> {
        Self::memo(slot::at(&self.recs, v.0 as usize).dom)
    }
    #[inline]
    pub fn set_dom_memo(&mut self, v: V, r: V) {
        slot::at_mut(&mut self.recs, v.0 as usize).dom = r.0;
    }

    // ---- construction ----

    #[inline]
    pub fn find_head(&self, h: &Head) -> Option<V> {
        self.head_index.get(&head_key(h)).map(V)
    }

    /// The bare value of a head (no arguments).
    pub fn head_value(&mut self, kind: Kind, h: Head) -> V {
        if let Some(v) = self.head_index.get(&head_key(&h)) {
            return V(v);
        }
        let flags = match h {
            Head::Local { .. } => OPEN,
            Head::Const { .. } => 0,
            Head::Proj { x, .. } => slot::at(&self.recs, x.0 as usize).flags & OPEN,
        };
        let hid = HeadId(self.heads.len() as u32);
        self.heads.push(h);
        let v = self.push(
            Val::Neu {
                kind,
                head: hid,
                spine: Sp::NIL,
            },
            flags,
        );
        self.head_index.insert(head_key(&h), v.0);
        v
    }

    /// The neutral `f a` for a neutral `f`.
    #[inline]
    pub fn neutral_app(&mut self, f: V, a: V) -> V {
        self.neutral_apply(f, &[a])
    }

    /// The neutral `f a₁ … aₙ` for a neutral `f`: its arguments followed by `args`, one probe.
    pub fn neutral_apply(&mut self, f: V, args: &[V]) -> V {
        let Val::Neu { kind, head, spine } = self.get(f) else {
            unreachable!("neutral_apply of a non-neutral")
        };
        if args.is_empty() {
            return f;
        }
        let flags = slot::at(&self.recs, f.0 as usize).flags;
        if spine.is_empty() {
            return self.neutral(kind, head, flags, args);
        }
        let mut all: smallvec::SmallVec<[V; 16]> = smallvec::SmallVec::new();
        all.extend_from_slice(self.spine(spine));
        all.extend_from_slice(args);
        self.neutral(kind, head, flags, &all)
    }

    /// The neutral of `kind` headed by `head` and applied to the non-empty `args`, interned by
    /// content. A new one is open when the head's `flags` or an argument say so; an interned one
    /// already knows.
    fn neutral(&mut self, kind: Kind, head: HeadId, flags: u8, args: &[V]) -> V {
        let h = neutral_hash(head, args);
        let (recs, slots) = (&self.recs, &self.spine_slots);
        let found = self
            .neutral_index
            .find(h, |v| match slot::at(recs, v as usize).val {
                Val::Neu {
                    head: h2, spine, ..
                } => h2 == head && spine.len() == args.len() && spine_of(slots, spine) == args,
                _ => false,
            });
        if let Some(v) = found {
            return V(v);
        }
        let flags = args
            .iter()
            .fold(flags, |fl, a| fl | slot::at(&self.recs, a.0 as usize).flags)
            & OPEN;
        let start = self.spine_slots.len() as u32;
        self.spine_slots.extend_from_slice(args);
        let spine = Sp {
            start,
            len: args.len() as u32,
        };
        let v = self.push(Val::Neu { kind, head, spine }, flags);
        self.neutral_index.insert(h, v.0);
        v
    }

    fn atom(&mut self, kind: u32, id: u32, make: impl FnOnce() -> Val) -> V {
        if let Some(v) = self.atom_index.get(&(kind, id)) {
            return V(v);
        }
        let v = self.push(make(), 0);
        self.atom_index.insert((kind, id), v.0);
        v
    }

    pub fn sort(&mut self, l: LevelId) -> V {
        self.atom(ATOM_SORT, l.0, || Val::Sort(l))
    }

    pub fn nat(&mut self, n: NatId) -> V {
        self.atom(ATOM_NAT, n.0, || Val::Nat(n))
    }

    pub fn str(&mut self, s: StrId) -> V {
        self.atom(ATOM_STR, s.0, || Val::Str(s))
    }

    pub fn lam(&mut self, dom: ExprId, env: EnvId, body: ExprId) -> V {
        let key = (dom.0, env.0, body.0);
        if let Some(v) = self.lam_index.get(&key) {
            return V(v);
        }
        let flags = if self.env_open(env) { OPEN } else { 0 };
        let v = self.push(Val::Lam { dom, env, body }, flags);
        self.lam_index.insert(key, v.0);
        v
    }

    pub fn pi(&mut self, dom: V, env: EnvId, body: ExprId) -> V {
        let key = (dom.0, env.0, body.0);
        if let Some(v) = self.pi_index.get(&key) {
            return V(v);
        }
        let flags = if self.is_open(dom) || self.env_open(env) {
            OPEN
        } else {
            0
        };
        let v = self.push(Val::Pi { dom, env, body }, flags);
        self.pi_index.insert(key, v.0);
        v
    }

    pub fn pi_lam(&mut self, dom: V, env: EnvId, ctx: CtxId, body: ExprId) -> V {
        let key = (dom.0, env.0, ctx.0, body.0);
        if let Some(v) = self.pilam_index.get(&key) {
            return V(v);
        }
        let flags = if self.is_open(dom) || self.env_open(env) || self.ctx_open(ctx) {
            OPEN
        } else {
            0
        };
        let v = self.push(
            Val::PiLam {
                dom,
                env,
                ctx,
                body,
            },
            flags,
        );
        self.pilam_index.insert(key, v.0);
        v
    }

    // ---- spines ----

    #[inline]
    pub fn spine_len(&self, s: Sp) -> usize {
        s.len()
    }

    /// The arguments of a spine, first to last, in place.
    #[inline]
    pub fn spine(&self, s: Sp) -> &[V] {
        spine_of(&self.spine_slots, s)
    }

    /// The arguments of a spine, first to last.
    #[inline]
    pub fn args(&self, s: Sp) -> smallvec::SmallVec<[V; 8]> {
        smallvec::SmallVec::from_slice(self.spine(s))
    }

    /// The `i`-th argument (0-based, first to last) of a spine of more than `i` arguments.
    #[inline]
    pub fn spine_arg(&self, s: Sp, i: usize) -> V {
        debug_assert!(i < s.len());
        *slot::at(&self.spine_slots, s.start as usize + i)
    }

    /// The `i`-th argument (0-based, first to last) of a spine.
    #[inline]
    pub fn arg(&self, s: Sp, i: usize) -> Option<V> {
        (i < s.len()).then(|| *slot::at(&self.spine_slots, s.start as usize + i))
    }

    // ---- environments and contexts ----

    #[inline]
    pub fn env(&self, e: EnvId) -> EnvNode {
        *slot::at(&self.envs, e.0 as usize)
    }

    /// The environment `e` was pruned to with the non-empty `mask`, if that was one of the two
    /// latest prunings asked of it.
    #[inline]
    pub fn last_prune(&self, e: EnvId, mask: u64) -> Option<EnvId> {
        debug_assert!(mask != 0);
        let n = slot::at(&self.envs, e.0 as usize);
        if n.pmask[0] == mask {
            Some(EnvId(n.pres[0]))
        } else if n.pmask[1] == mask {
            Some(EnvId(n.pres[1]))
        } else {
            None
        }
    }

    #[inline]
    pub fn env_open(&self, e: EnvId) -> bool {
        slot::at(&self.envs, e.0 as usize).open
    }

    /// The universe substitution `e`'s terms are read under.
    #[inline]
    pub fn lsub_of(&self, e: EnvId) -> SubstId {
        slot::at(&self.envs, e.0 as usize).lsub
    }

    /// The empty environment of the substitution `s`, if there is one yet.
    #[inline]
    pub fn find_lsub_base(&self, s: SubstId) -> Option<EnvId> {
        if s == SubstId::IDENTITY {
            return Some(EnvId::EMPTY);
        }
        self.base_index.get(&(s.0, 0)).map(EnvId)
    }

    /// The empty environment of the substitution `s` (for `s` the identity, [`EnvId::EMPTY`]).
    pub fn lsub_base(&mut self, s: SubstId) -> EnvId {
        if s == SubstId::IDENTITY {
            return EnvId::EMPTY;
        }
        if let Some(e) = self.base_index.get(&(s.0, 0)) {
            return EnvId(e);
        }
        let id = EnvId(self.envs.len() as u32);
        self.envs.push(EnvNode {
            parent: EnvId::EMPTY,
            val: V(NONE),
            len: 0,
            open: false,
            frame: false,
            live: 0,
            dense: true,
            pmask: [0; 2],
            pres: [NONE; 2],
            xval: V(NONE),
            xres: EnvId::EMPTY,
            wide: NONE,
            lsub: s,
        });
        self.base_index.insert((s.0, 0), id.0);
        id
    }

    /// Whether two environments have the same length and substitution: all that tells them apart
    /// besides their entries.
    #[inline]
    pub fn same_shape(&self, a: EnvId, b: EnvId) -> bool {
        let (x, y) = (
            slot::at(&self.envs, a.0 as usize),
            slot::at(&self.envs, b.0 as usize),
        );
        x.len == y.len && x.lsub == y.lsub
    }

    pub fn extend(&mut self, parent: EnvId, val: V) -> EnvId {
        let last = slot::at(&self.envs, parent.0 as usize);
        if last.xval == val {
            return last.xres;
        }
        let e = self.extension(parent, val);
        let p = slot::at_mut(&mut self.envs, parent.0 as usize);
        p.xval = val;
        p.xres = e;
        e
    }

    /// [`Arena::extend`] past the parent's last extension: interned by (parent, value).
    fn extension(&mut self, parent: EnvId, val: V) -> EnvId {
        if let Some(e) = self.env_index.get(&(parent.0, val.0)) {
            return EnvId(e);
        }
        let p = *slot::at(&self.envs, parent.0 as usize);
        let id = EnvId(self.envs.len() as u32);
        let open = p.open || self.is_open(val);
        let live = (p.live << 1) | (val != DEAD) as u64;
        let dense = p.dense && val != DEAD;
        self.envs.push(EnvNode {
            parent,
            val,
            len: p.len + 1,
            open,
            frame: false,
            live,
            dense,
            pmask: [0; 2],
            pres: [NONE; 2],
            xval: V(NONE),
            xres: EnvId::EMPTY,
            wide: NONE,
            lsub: p.lsub,
        });
        self.env_index.insert((parent.0, val.0), id.0);
        id
    }

    /// Entries stored in frames so far this session: what pruning has copied.
    pub fn frame_slot_count(&self) -> usize {
        self.frame_slots.len()
    }

    /// Arguments stored in neutral spines so far this session.
    pub fn spine_slot_count(&self) -> usize {
        self.spine_slots.len()
    }

    /// The value of de Bruijn index `i`.
    #[inline]
    pub fn lookup(&self, mut e: EnvId, mut i: u32) -> Option<V> {
        loop {
            let n = slot::at(&self.envs, e.0 as usize);
            if i >= n.len {
                return None;
            }
            if n.frame {
                return Some(self.frame_entry(n, i));
            }
            if i == 0 {
                return Some(n.val);
            }
            i -= 1;
            e = n.parent;
        }
    }

    /// The entries of an environment, innermost first ([`DEAD`] for those pruned away), in one
    /// walk.
    pub fn entries(&self, e: EnvId) -> impl Iterator<Item = V> + '_ {
        let mut cur = e;
        let mut offset = 0u32;
        (0..slot::at(&self.envs, e.0 as usize).len).map(move |i| {
            let n = *slot::at(&self.envs, cur.0 as usize);
            if n.frame {
                self.frame_entry(&n, i - offset)
            } else {
                cur = n.parent;
                offset += 1;
                n.val
            }
        })
    }

    /// Entry `i` (below the frame's length) of a frame: its value, or [`DEAD`].
    #[inline]
    fn frame_entry(&self, n: &EnvNode, i: u32) -> V {
        if n.wide == NONE {
            if i >= 64 || (n.live >> i) & 1 == 0 {
                return DEAD;
            }
            let rank = (n.live & ((1u64 << i) - 1)).count_ones();
            return *slot::at(&self.frame_slots, (n.val.0 + rank) as usize);
        }
        self.wide_frame_entry(n, i)
    }

    /// [`Arena::frame_entry`] of a wide frame, found by searching its index list.
    #[cold]
    #[inline(never)]
    fn wide_frame_entry(&self, n: &EnvNode, i: u32) -> V {
        match self.wide_indices(n).binary_search(&i) {
            Ok(rank) => *slot::at(&self.frame_slots, n.val.0 as usize + rank),
            Err(_) => DEAD,
        }
    }

    #[inline]
    fn wide_indices(&self, n: &EnvNode) -> &[u32] {
        let start = n.wide as usize;
        let count = *slot::at(&self.wide_idx, start) as usize;
        &self.wide_idx[start + 1..start + 1 + count]
    }

    /// `e` pruned to the entries `mask` marks, the highest of them entry `len - 1`: the frame of
    /// their values (innermost first), read under `e`'s substitution, interned by content, and
    /// remembered as `e`'s latest pruning.
    #[cold]
    #[inline(never)]
    pub fn prune(&mut self, e: EnvId, len: u32, mask: u64) -> EnvId {
        let r = self.picked_frame(e, len, mask);
        let n = slot::at_mut(&mut self.envs, e.0 as usize);
        n.pmask = [mask, n.pmask[0]];
        n.pres = [r.0, n.pres[0]];
        r
    }

    /// The frame of [`Arena::prune`]. The values are picked straight into the frame store's
    /// tail, in one walk, and left there only if no frame holds them yet.
    #[cfg_attr(debug_assertions, inline)]
    #[cfg_attr(not(debug_assertions), inline(always))]
    fn picked_frame(&mut self, mut e: EnvId, len: u32, mask: u64) -> EnvId {
        debug_assert!(mask != 0 && 64 - mask.leading_zeros() == len);
        let lsub = slot::at(&self.envs, e.0 as usize).lsub;
        let start = self.frame_slots.len();
        let count = mask.count_ones() as usize;
        self.frame_slots.reserve(count);
        // `m` is relative to the node reached: bit 0 is that node's entry.
        let mut m = mask;
        while m != 0 {
            let n = slot::at(&self.envs, e.0 as usize);
            debug_assert!(n.len > 0, "a referenced entry is in range");
            if n.frame {
                let mut r = m;
                while r != 0 {
                    let v = self.frame_entry(n, r.trailing_zeros());
                    self.frame_slots.push(v);
                    r &= r - 1;
                }
                break;
            }
            if m & 1 == 1 {
                self.frame_slots.push(n.val);
            }
            m >>= 1;
            e = n.parent;
        }
        debug_assert_eq!(self.frame_slots.len(), start + count);
        let vals = slot::run(&self.frame_slots, start, count);
        let h = frame_hash(len, mask, vals) ^ lsub_hash(lsub);
        if let Some(f) = self.find_frame(h, len, mask, lsub, vals) {
            self.frame_slots.truncate(start);
            return EnvId(f);
        }
        self.seal_frame(h, len, mask, NONE, start, lsub)
    }

    /// The values at `indices` (ascending, innermost first), in one walk.
    pub fn select_indices(
        &self,
        mut e: EnvId,
        indices: &[u32],
        out: &mut smallvec::SmallVec<[V; 16]>,
    ) {
        let mut pos = 0u32;
        let mut k = 0;
        while k < indices.len() {
            let n = slot::at(&self.envs, e.0 as usize);
            debug_assert!(n.len > 0, "a referenced entry is in range");
            if n.frame {
                if n.wide == NONE {
                    for &i in &indices[k..] {
                        out.push(self.frame_entry(n, i - pos));
                    }
                } else {
                    // Both ascending: one walk over the frame's own indices.
                    let (wide, mut r) = (self.wide_indices(n), 0);
                    for &i in &indices[k..] {
                        let want = i - pos;
                        while r < wide.len() && wide[r] < want {
                            r += 1;
                        }
                        let hit = r < wide.len() && wide[r] == want;
                        out.push(if hit {
                            *slot::at(&self.frame_slots, n.val.0 as usize + r)
                        } else {
                            DEAD
                        });
                    }
                }
                return;
            }
            if indices[k] == pos {
                out.push(n.val);
                k += 1;
            }
            pos += 1;
            e = n.parent;
        }
    }

    /// The frame of `len` entries whose live entries are `vals` at the positions `mask` marks
    /// (innermost first), read under the substitution `lsub`, interned by content.
    pub fn frame(&mut self, len: u32, mask: u64, vals: &[V], lsub: SubstId) -> EnvId {
        debug_assert_eq!(mask.count_ones() as usize, vals.len());
        let h = frame_hash(len, mask, vals) ^ lsub_hash(lsub);
        if let Some(f) = self.find_frame(h, len, mask, lsub, vals) {
            return EnvId(f);
        }
        let start = self.frame_slots.len();
        self.frame_slots.extend_from_slice(vals);
        self.seal_frame(h, len, mask, NONE, start, lsub)
    }

    /// The narrow frame of `len` entries holding `vals` at the positions `mask` marks, under
    /// `lsub`, if there is one; `h` is its hash.
    #[inline]
    fn find_frame(&self, h: u64, len: u32, mask: u64, lsub: SubstId, vals: &[V]) -> Option<u32> {
        let (envs, slots) = (&self.envs, &self.frame_slots);
        self.frame_index.find(h, |f| {
            let n = slot::at(envs, f as usize);
            n.wide == NONE
                && n.len == len
                && n.live == mask
                && n.lsub == lsub
                && slot::run(slots, n.val.0 as usize, vals.len()) == vals
        })
    }

    /// The frame of `len` entries whose live entries are `vals` at `indices` (ascending, some of
    /// them 64 or more), read under the substitution `lsub`, interned by content.
    pub fn wide_frame(&mut self, len: u32, indices: &[u32], vals: &[V], lsub: SubstId) -> EnvId {
        debug_assert_eq!(indices.len(), vals.len());
        let mask = indices
            .iter()
            .filter(|&&i| i < 64)
            .fold(0u64, |m, &i| m | (1 << i));
        let h = indices.iter().fold(frame_hash(len, mask, vals), |h, &i| {
            (h ^ i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        }) ^ lsub_hash(lsub);
        if let Some(e) = self.frame_index.find(h, |e| {
            let n = slot::at(&self.envs, e as usize);
            n.wide != NONE
                && n.len == len
                && n.lsub == lsub
                && self.wide_indices(n) == indices
                && &self.frame_slots[n.val.0 as usize..n.val.0 as usize + vals.len()] == vals
        }) {
            return EnvId(e);
        }
        let wide = self.wide_idx.len() as u32;
        self.wide_idx.push(indices.len() as u32);
        self.wide_idx.extend_from_slice(indices);
        let start = self.frame_slots.len();
        self.frame_slots.extend_from_slice(vals);
        self.seal_frame(h, len, mask, wide, start, lsub)
    }

    /// The new frame whose values are the frame store's tail from `start`, entered under its
    /// hash `h`.
    fn seal_frame(
        &mut self,
        h: u64,
        len: u32,
        live: u64,
        wide: u32,
        start: usize,
        lsub: SubstId,
    ) -> EnvId {
        let vals = &self.frame_slots[start..];
        let open = vals.iter().any(|v| self.is_open(*v));
        let dense = vals.len() == len as usize;
        let id = EnvId(self.envs.len() as u32);
        self.envs.push(EnvNode {
            parent: EnvId::EMPTY,
            val: V(start as u32),
            len,
            open,
            frame: true,
            live,
            dense,
            pmask: [0; 2],
            pres: [NONE; 2],
            xval: V(NONE),
            xres: EnvId::EMPTY,
            wide,
            lsub,
        });
        // Frames are only ever found again through their hash, which is kept in the table.
        self.frame_index.insert(h, id.0);
        id
    }

    pub fn ctx_extend(&mut self, parent: CtxId, ty: V) -> CtxId {
        if let Some(c) = self.ctx_index.get(&(parent.0, ty.0)) {
            return CtxId(c);
        }
        let id = CtxId(self.ctxs.len() as u32);
        let open = slot::at(&self.ctxs, parent.0 as usize).open || self.is_open(ty);
        self.ctxs.push(CtxNode { parent, ty, open });
        self.ctx_index.insert((parent.0, ty.0), id.0);
        id
    }

    pub fn ctx_lookup(&self, mut c: CtxId, mut i: u32) -> Option<V> {
        loop {
            if c == CtxId::EMPTY {
                return None;
            }
            let n = *slot::at(&self.ctxs, c.0 as usize);
            if i == 0 {
                return Some(n.ty);
            }
            i -= 1;
            c = n.parent;
        }
    }

    #[inline]
    fn ctx_open(&self, c: CtxId) -> bool {
        slot::at(&self.ctxs, c.0 as usize).open
    }

    /// Bytes held by each part of the arena (capacity: what the process pays).
    pub fn memory_report(&self) -> Vec<(&'static str, usize)> {
        vec![
            (
                "value engine: values",
                self.recs.capacity() * std::mem::size_of::<Val>(),
            ),
            (
                "value engine: value memos and flags",
                self.recs.capacity() * (std::mem::size_of::<Rec>() - std::mem::size_of::<Val>()),
            ),
            (
                "value engine: heads",
                self.heads.capacity() * std::mem::size_of::<Head>(),
            ),
            (
                "value engine: spines",
                self.spine_slots.capacity() * std::mem::size_of::<V>(),
            ),
            (
                "value engine: environments and frames",
                self.envs.capacity() * std::mem::size_of::<EnvNode>()
                    + self.frame_slots.capacity() * std::mem::size_of::<V>()
                    + self.wide_idx.capacity() * 4,
            ),
            (
                "value engine: contexts",
                self.ctxs.capacity() * std::mem::size_of::<CtxNode>(),
            ),
            ("value engine: neutral index", self.neutral_index.bytes()),
            ("value engine: environment index", self.env_index.bytes()),
            ("value engine: frame index", self.frame_index.bytes()),
            (
                "value engine: other indexes",
                self.base_index.bytes()
                    + self.head_index.bytes()
                    + self.ctx_index.bytes()
                    + self.atom_index.bytes()
                    + self.lam_index.bytes()
                    + self.pi_index.bytes()
                    + self.pilam_index.bytes(),
            ),
        ]
    }

    pub fn bytes(&self) -> usize {
        self.memory_report().iter().map(|(_, b)| b).sum()
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/sem/value/content_controls.rs"]
mod content_controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/sem/value/memory_controls.rs"]
mod memory_controls;
