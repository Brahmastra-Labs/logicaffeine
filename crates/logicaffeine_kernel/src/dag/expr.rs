//! The kernel core's expression store: a maximally shared DAG with de Bruijn indices.
//!
//! Every structurally distinct expression is one [`ExprId`]. Construction goes through
//! [`Store::intern`], so building the same node twice yields the same id and structural equality
//! is an integer compare. Each node carries its loose-bound-variable range and whether it contains
//! free variables or universe parameters, computed once at construction; substitution skips any
//! subterm those flags prove it cannot touch.
//!
//! Binder names and binder annotations are not part of a node: they are irrelevant to typing and
//! conversion, and dropping them lets `fun (x : A) => x` and `fun (y : A) => y` share one id.
//!
//! Nodes created while a declaration is checked live in a separate scratch region, marked by the
//! top bit of their id. When the declaration is done, [`Store::end_scratch`] drops them in O(1),
//! so transient terms never accumulate across a hundred thousand declarations — while a result
//! computed purely from persistent terms (a definition's body at given universe levels) can
//! still be interned persistently mid-declaration ([`Store::with_persistent`]) and cached for
//! every later declaration.

use super::level::{LevelId, Levels, LevelsId};
use super::name::{NameId, Names};
use super::nat::Nat;
use super::slot;
use rustc_hash::FxHashMap as FxMap;
use std::rc::Rc;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ExprId(pub u32);

/// The arguments of an application spine; most spines fit inline.
pub type Args = smallvec::SmallVec<[ExprId; 8]>;

/// The longest spine [`Store::short_app_args`] collects into a caller's buffer.
pub const SHORT_SPINE: usize = 16;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct NatId(pub u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct StrId(pub u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LitId(pub u32);

/// A LOGOS literal with no Lean counterpart. `Nat` and `Text` literals are [`Node::NatLit`] and
/// [`Node::StrLit`]. An integer is canonical: one that fits `i64` is `Int`, any other `BigInt`.
/// A float is held by its bits, so equal literals are one id and `0.0`/`-0.0` stay apart.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Lit {
    Int(i64),
    BigInt(logicaffeine_base::BigInt),
    Float(u64),
    Duration(i64),
    Date(i32),
    Moment(i64),
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Node {
    BVar(u32),
    FVar(u32),
    Sort(LevelId),
    Const(NameId, LevelsId),
    App(ExprId, ExprId),
    Lam(ExprId, ExprId),
    Pi(ExprId, ExprId),
    Let(ExprId, ExprId, ExprId),
    NatLit(NatId),
    StrLit(StrId),
    Proj(NameId, u32, ExprId),
    /// LOGOS: `match disc return motive with cases`, the cases a [`Node::Seq`] chain in
    /// constructor order.
    Match(ExprId, ExprId, ExprId),
    /// LOGOS: `fix f => body`, with `f` bound as `BVar 0` in `body`.
    Fix(ExprId),
    /// LOGOS: the `index`-th member of a mutual fixpoint block whose members (a [`Node::Seq`]
    /// chain) are each bound in every body, the first member outermost.
    MutualFix(ExprId, u32),
    /// A non-empty list of expressions: its first element and the rest ([`Node::Nil`] at the end).
    /// Only [`Node::Match`] and [`Node::MutualFix`] hold one.
    Seq(ExprId, ExprId),
    Nil,
    /// A LOGOS literal with no Lean counterpart.
    Lit(LitId),
}

const HAS_FVAR: u8 = 1;
const HAS_LPARAM: u8 = 2;
/// A bound variable with index 64 or more occurs (bound or loose): the loose-variable mask may
/// have lost bits and is not exact.
const WIDE_BVAR: u8 = 4;

impl Node {
    /// A well-mixed hash of the node's fields. The table indexes with the TOP bits (Fibonacci
    /// hashing), which a multiply-xorshift chain fills with entropy from every field.
    #[inline]
    fn hash(&self) -> u64 {
        let (tag, a, b, c) = match *self {
            Node::BVar(i) => (1u64, i as u64, 0, 0),
            Node::FVar(i) => (2, i as u64, 0, 0),
            Node::Sort(l) => (3, l.0 as u64, 0, 0),
            Node::Const(n, ls) => (4, n.0 as u64, ls.0 as u64, 0),
            Node::App(f, a) => (5, f.0 as u64, a.0 as u64, 0),
            Node::Lam(t, b) => (6, t.0 as u64, b.0 as u64, 0),
            Node::Pi(t, b) => (7, t.0 as u64, b.0 as u64, 0),
            Node::Let(t, v, b) => (8, t.0 as u64, v.0 as u64, b.0 as u64),
            Node::NatLit(n) => (9, n.0 as u64, 0, 0),
            Node::StrLit(s) => (10, s.0 as u64, 0, 0),
            Node::Proj(n, i, e) => (11, n.0 as u64, i as u64, e.0 as u64),
            Node::Match(d, m, c) => (12, d.0 as u64, m.0 as u64, c.0 as u64),
            Node::Fix(b) => (13, b.0 as u64, 0, 0),
            Node::MutualFix(ds, i) => (14, ds.0 as u64, i as u64, 0),
            Node::Seq(h, t) => (15, h.0 as u64, t.0 as u64, 0),
            Node::Nil => (16, 0, 0, 0),
            Node::Lit(l) => (17, l.0 as u64, 0, 0),
        };
        let mut h = (tag ^ (a << 8)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h = (h ^ b ^ (c << 32)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^ (h >> 29)
    }
}

/// The scratch region's table. Open addressing with linear probing. A slot holds
/// `(generation << 32) | (id + 1)`; a slot from an older generation reads as empty, so clearing
/// the table is one increment, not a memset.
///
/// (A hash tag in the slot would spare node-array reads on mismatching probes, fewer cache
/// misses, but measured +1% instructions on `init`; instructions are the Arena's metric.)
#[derive(Clone)]
struct Table {
    slots: Vec<u64>,
    shift: u32,
    len: usize,
    generation: u64,
}

impl Table {
    fn new(cap: usize) -> Table {
        let cap = cap.next_power_of_two().max(16);
        Table {
            slots: vec![0; cap],
            shift: 64 - cap.trailing_zeros(),
            len: 0,
            generation: 1,
        }
    }

    /// A slot is live exactly when its generation is current (slots are only written whole).
    #[inline]
    fn is_live(&self, s: u64) -> bool {
        s >> 32 == self.generation
    }

    /// The node's id, or the vacant slot where it would go (valid until the table changes).
    #[inline]
    fn find_or_vacant(&self, entries: &[Entry], node: &Node, h: u64) -> Result<ExprId, usize> {
        let mask = self.slots.len() - 1;
        let mut i = (h >> self.shift) as usize;
        loop {
            let s = *slot::at(&self.slots, i);
            if !self.is_live(s) {
                return Err(i);
            }
            let id = s as u32 - 1;
            if slot::at(entries, id as usize).node == *node {
                return Ok(ExprId(id));
            }
            i = (i + 1) & mask;
        }
    }

    /// Inserts `id` at a slot `find_or_vacant` returned, unless the table must grow first.
    #[inline]
    fn insert_at(&mut self, entries: &[Entry], slot: Option<usize>, id: ExprId, h: u64) {
        match slot {
            Some(i) if (self.len + 1) * 2 <= self.slots.len() => {
                *slot::at_mut(&mut self.slots, i) = (self.generation << 32) | (id.0 as u64 + 1);
                self.len += 1;
            }
            _ => self.insert(entries, id, h),
        }
    }

    fn insert(&mut self, entries: &[Entry], id: ExprId, h: u64) {
        if (self.len + 1) * 2 > self.slots.len() {
            self.grow(entries);
        }
        self.place(id.0, h);
        self.len += 1;
    }

    #[inline]
    fn place(&mut self, id: u32, h: u64) {
        let mask = self.slots.len() - 1;
        let mut i = (h >> self.shift) as usize;
        while self.is_live(*slot::at(&self.slots, i)) {
            i = (i + 1) & mask;
        }
        *slot::at_mut(&mut self.slots, i) = (self.generation << 32) | (id as u64 + 1);
    }

    fn grow(&mut self, entries: &[Entry]) {
        let cap = (self.slots.len() * 2).max(16);
        self.shift = 64 - cap.trailing_zeros();
        let old = std::mem::replace(&mut self.slots, vec![0; cap]);
        let old_gen = self.generation;
        self.generation = 1;
        for s in old {
            if s >> 32 == old_gen {
                let id = s as u32 - 1;
                self.place(id, entries[id as usize].node.hash());
            }
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.generation += 1;
        if self.generation >= u32::MAX as u64 {
            self.slots.iter_mut().for_each(|s| *s = 0);
            self.generation = 1;
        }
    }
}

/// The persistent region's table. It is never cleared, so it needs no generation: a slot is
/// `id + 1`, and `0` is vacant — 4 bytes per slot, half the scratch table's.
#[derive(Clone)]
struct PermTable {
    slots: Vec<u32>,
    shift: u32,
    len: usize,
}

impl PermTable {
    fn new(cap: usize) -> PermTable {
        let cap = cap.next_power_of_two().max(16);
        PermTable {
            slots: vec![0; cap],
            shift: 64 - cap.trailing_zeros(),
            len: 0,
        }
    }

    /// Room for `n` entries in all: one rehash, straight to the capacity that holds them.
    fn reserve(&mut self, entries: &[Entry], n: usize) {
        let cap = ((n + 1) * 2).next_power_of_two();
        if cap > self.slots.len() {
            self.rehash(entries, cap);
        }
    }

    #[inline]
    fn find_or_vacant(&self, entries: &[Entry], node: &Node, h: u64) -> Result<ExprId, usize> {
        let mask = self.slots.len() - 1;
        let mut i = (h >> self.shift) as usize;
        loop {
            let s = *slot::at(&self.slots, i);
            if s == 0 {
                return Err(i);
            }
            if slot::at(entries, (s - 1) as usize).node == *node {
                return Ok(ExprId(s - 1));
            }
            i = (i + 1) & mask;
        }
    }

    #[inline]
    fn insert_at(&mut self, entries: &[Entry], slot: Option<usize>, id: ExprId, h: u64) {
        match slot {
            Some(i) if (self.len + 1) * 2 <= self.slots.len() => {
                *slot::at_mut(&mut self.slots, i) = id.0 + 1
            }
            _ => {
                if (self.len + 1) * 2 > self.slots.len() {
                    self.grow(entries);
                }
                self.place(id.0, h);
            }
        }
        self.len += 1;
    }

    #[inline]
    fn place(&mut self, id: u32, h: u64) {
        let mask = self.slots.len() - 1;
        let mut i = (h >> self.shift) as usize;
        while *slot::at(&self.slots, i) != 0 {
            i = (i + 1) & mask;
        }
        *slot::at_mut(&mut self.slots, i) = id + 1;
    }

    fn grow(&mut self, entries: &[Entry]) {
        self.rehash(entries, (self.slots.len() * 2).max(16));
    }

    fn rehash(&mut self, entries: &[Entry], cap: usize) {
        self.shift = 64 - cap.trailing_zeros();
        let old = std::mem::replace(&mut self.slots, vec![0; cap]);
        for s in old {
            if s != 0 {
                self.place(s - 1, entries[(s - 1) as usize].node.hash());
            }
        }
    }
}

/// A memo for one traversal (substitution, lifting, abstraction), keyed on `(node, depth)`.
/// Open addressing with generation stamps: starting a new traversal is one increment, and a
/// probe is a couple of instructions, where a general hash map pays for hashing, insertion and
/// clearing on every call.
#[derive(Clone)]
struct Memo {
    keys: Vec<u64>,
    vals: Vec<u32>,
    gens: Vec<u32>,
    generation: u32,
    len: usize,
    shift: u32,
}

impl Memo {
    fn new() -> Memo {
        let cap = 1 << 10;
        Memo {
            keys: vec![0; cap],
            vals: vec![0; cap],
            gens: vec![0; cap],
            generation: 1,
            len: 0,
            shift: 64 - 10,
        }
    }

    #[inline]
    fn slot(&self, key: u64) -> usize {
        (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> self.shift) as usize
    }

    /// Starts a fresh traversal.
    #[inline]
    fn reset(&mut self) {
        self.len = 0;
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.gens.iter_mut().for_each(|g| *g = 0);
            self.generation = 1;
        }
    }

    #[inline]
    fn get(&self, e: ExprId, off: u32) -> Option<ExprId> {
        let key = ((off as u64) << 32) | e.0 as u64;
        let mask = self.keys.len() - 1;
        let mut i = self.slot(key);
        while self.gens[i] == self.generation {
            if self.keys[i] == key {
                return Some(ExprId(self.vals[i]));
            }
            i = (i + 1) & mask;
        }
        None
    }

    #[inline]
    fn insert(&mut self, e: ExprId, off: u32, r: ExprId) {
        if (self.len + 1) * 2 > self.keys.len() {
            self.grow();
        }
        let key = ((off as u64) << 32) | e.0 as u64;
        self.place(key, r.0);
        self.len += 1;
    }

    #[inline]
    fn place(&mut self, key: u64, val: u32) {
        let mask = self.keys.len() - 1;
        let mut i = self.slot(key);
        while self.gens[i] == self.generation {
            i = (i + 1) & mask;
        }
        self.keys[i] = key;
        self.vals[i] = val;
        self.gens[i] = self.generation;
    }

    fn grow(&mut self) {
        let cap = self.keys.len() * 2;
        let keys = std::mem::replace(&mut self.keys, vec![0; cap]);
        let vals = std::mem::replace(&mut self.vals, vec![0; cap]);
        let gens = std::mem::replace(&mut self.gens, vec![0; cap]);
        self.shift = 64 - cap.trailing_zeros();
        let g = self.generation;
        for i in 0..keys.len() {
            if gens[i] == g {
                self.place(keys[i], vals[i]);
            }
        }
    }
}

/// The id bit marking a scratch node.
const SCRATCH: u32 = 1 << 31;

impl ExprId {
    #[inline]
    pub fn is_scratch(self) -> bool {
        self.0 & SCRATCH != 0
    }
}

#[inline]
fn has_scratch_child(node: &Node) -> bool {
    match *node {
        Node::App(a, b) | Node::Lam(a, b) | Node::Pi(a, b) | Node::Seq(a, b) => {
            (a.0 | b.0) & SCRATCH != 0
        }
        Node::Let(a, b, c) | Node::Match(a, b, c) => (a.0 | b.0 | c.0) & SCRATCH != 0,
        Node::Proj(_, _, a) | Node::Fix(a) | Node::MutualFix(a, _) => a.0 & SCRATCH != 0,
        _ => false,
    }
}

/// A node and its cached metadata, together: whatever reads one usually reads the others.
#[derive(Copy, Clone)]
struct Entry {
    node: Node,
    /// Which loose de Bruijn indices below 64 occur (exact only without [`WIDE_BVAR`]).
    mask: u64,
    lbr: u32,
    flags: u8,
    /// How many applications lead down the function side to the head ([`SPINE_UNKNOWN`] once
    /// that no longer fits): the argument count, known without walking the spine. For a
    /// [`Node::Seq`], its length.
    spine: u16,
}

const SPINE_UNKNOWN: u16 = u16::MAX;

const _: () = assert!(
    std::mem::size_of::<Entry>() == 32,
    "an entry stays half a cache line"
);

/// One region's nodes.
#[derive(Clone, Default)]
struct Region {
    entries: Vec<Entry>,
}

impl Region {
    fn clear(&mut self) {
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Clone)]
pub struct Store {
    pub names: Names,
    pub levels: Levels,
    perm: Region,
    temp: Region,
    persistent: PermTable,
    scratch: Table,
    in_scratch: bool,
    force_persistent: bool,
    /// Set when a [`Store::with_persistent`] computation met a node that already lives in scratch.
    tainted: bool,
    /// The largest scratch region any one declaration needed, in nodes.
    temp_peak: usize,
    /// Persistent nodes created by [`Store::with_persistent`] (cached instantiations), rather than
    /// read from the input.
    cached_nodes: usize,
    nats: Vec<Nat>,
    nat_index: FxMap<Nat, NatId>,
    strs: Vec<Rc<str>>,
    str_index: FxMap<Rc<str>, StrId>,
    lits: Vec<Lit>,
    lit_index: FxMap<Lit, LitId>,
    inst_memo: Memo,
    lift_memo: Memo,
    abs_memo: Memo,
    lp_memo: Memo,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Store {
        Store {
            names: Names::new(),
            levels: Levels::new(),
            perm: Region::default(),
            temp: Region::default(),
            persistent: PermTable::new(1 << 12),
            scratch: Table::new(1 << 12),
            in_scratch: false,
            force_persistent: false,
            tainted: false,
            temp_peak: 0,
            cached_nodes: 0,
            nats: Vec::new(),
            nat_index: FxMap::default(),
            strs: Vec::new(),
            str_index: FxMap::default(),
            lits: Vec::new(),
            lit_index: FxMap::default(),
            inst_memo: Memo::new(),
            lift_memo: Memo::new(),
            abs_memo: Memo::new(),
            lp_memo: Memo::new(),
        }
    }

    pub fn expr_count(&self) -> usize {
        self.perm.len() + self.temp.len()
    }

    pub fn persistent_count(&self) -> usize {
        self.perm.len()
    }

    /// Bytes held by each part of the store (capacity, not length: what the process pays).
    pub fn memory_report(&self) -> Vec<(&'static str, usize)> {
        let region = |r: &Region| r.entries.capacity() * std::mem::size_of::<Entry>();
        vec![
            ("persistent nodes (count)", self.perm.len()),
            (
                "persistent nodes from cached instantiations (count)",
                self.cached_nodes,
            ),
            (
                "largest scratch region (nodes)",
                self.temp_peak.max(self.temp.len()),
            ),
            ("persistent region bytes", region(&self.perm)),
            ("scratch region bytes", region(&self.temp)),
            (
                "persistent table bytes",
                self.persistent.slots.capacity() * 4,
            ),
            ("scratch table bytes", self.scratch.slots.capacity() * 8),
            ("names (count)", self.names.len()),
            ("levels (count)", self.levels.len()),
            ("nat literals (count)", self.nats.len()),
            ("string literals (count)", self.strs.len()),
            ("LOGOS literals (count)", self.lits.len()),
            (
                "memo bytes",
                [
                    &self.inst_memo,
                    &self.lift_memo,
                    &self.abs_memo,
                    &self.lp_memo,
                ]
                .iter()
                .map(|m| m.keys.capacity() * 16)
                .sum(),
            ),
        ]
    }

    /// Room for `n` more persistent nodes without growing (a reader that knows its input size
    /// avoids rehashing a table of millions of entries several times over).
    pub fn reserve_persistent(&mut self, n: usize) {
        let total = self.perm.len() + n;
        self.perm.entries.reserve(n);
        self.persistent.reserve(&self.perm.entries, total);
    }

    /// Everything interned from now on is transient until [`Store::end_scratch`].
    pub fn begin_scratch(&mut self) {
        debug_assert!(!self.in_scratch);
        self.in_scratch = true;
    }

    pub fn end_scratch(&mut self) {
        if self.in_scratch {
            self.temp_peak = self.temp_peak.max(self.temp.len());
            self.in_scratch = false;
            self.temp.clear();
            self.scratch.clear();
        }
    }

    pub fn in_scratch(&self) -> bool {
        self.in_scratch
    }

    /// Runs `f`, a computation over persistent terms only, interning what it builds persistently
    /// so the result can be cached across declarations. Returns the result and whether it is
    /// persistent.
    ///
    /// A structure is only ever created when it is in neither region, so each structure has one
    /// id at any time. If `f` meets a node that already lives in scratch (built earlier in this
    /// declaration), the computation continues in scratch with that node rather than duplicate it,
    /// and the result is reported as not persistent — correct for this declaration, not cacheable.
    pub fn with_persistent<T>(&mut self, f: impl FnOnce(&mut Store) -> T) -> (T, bool) {
        let saved_force = std::mem::replace(&mut self.force_persistent, true);
        let saved_taint = std::mem::replace(&mut self.tainted, false);
        let r = f(self);
        let persistent = !self.tainted;
        self.force_persistent = saved_force;
        self.tainted = saved_taint || self.tainted;
        (r, persistent)
    }

    pub fn intern(&mut self, node: Node) -> ExprId {
        let h = node.hash();
        let scratch_child = has_scratch_child(&node);
        let mut perm_slot = None;
        if !scratch_child {
            match self.persistent.find_or_vacant(&self.perm.entries, &node, h) {
                Ok(id) => return id,
                Err(slot) => perm_slot = Some(slot),
            }
        }
        let mut temp_slot = None;
        if self.in_scratch {
            match self.scratch.find_or_vacant(&self.temp.entries, &node, h) {
                Ok(i) => {
                    self.tainted |= self.force_persistent;
                    return ExprId(i.0 | SCRATCH);
                }
                Err(slot) => temp_slot = Some(slot),
            }
        }
        let entry = self.entry_of(node);
        if self.in_scratch && (!self.force_persistent || scratch_child) {
            self.tainted |= self.force_persistent;
            let i = ExprId(self.temp.len() as u32);
            self.temp.entries.push(entry);
            self.scratch.insert_at(&self.temp.entries, temp_slot, i, h);
            ExprId(i.0 | SCRATCH)
        } else {
            debug_assert!(!scratch_child);
            let id = ExprId(self.perm.len() as u32);
            assert!(id.0 & SCRATCH == 0, "persistent expression store is full");
            self.cached_nodes += self.force_persistent as usize;
            self.perm.entries.push(entry);
            self.persistent
                .insert_at(&self.perm.entries, perm_slot, id, h);
            id
        }
    }

    #[inline]
    fn entry(&self, e: ExprId) -> &Entry {
        if e.0 & SCRATCH != 0 {
            slot::at(&self.temp.entries, (e.0 & !SCRATCH) as usize)
        } else {
            slot::at(&self.perm.entries, e.0 as usize)
        }
    }

    #[inline]
    fn lbr_of(&self, e: ExprId) -> u32 {
        self.entry(e).lbr
    }

    #[inline]
    fn flags_of(&self, e: ExprId) -> u8 {
        self.entry(e).flags
    }

    #[inline]
    fn mask_of(&self, e: ExprId) -> u64 {
        self.entry(e).mask
    }

    /// The node with its metadata: loose-variable range and mask, flags, spine length — each
    /// child's entry read once.
    #[inline(always)]
    fn entry_of(&self, node: Node) -> Entry {
        let (lbr, mask, flags, spine) = match node {
            Node::BVar(i) => (
                i + 1,
                if i < 64 { 1 << i } else { 0 },
                if i < 64 { 0 } else { WIDE_BVAR },
                0,
            ),
            Node::FVar(_) => (0, 0, HAS_FVAR, 0),
            Node::Sort(l) => (
                0,
                0,
                if self.levels.has_param(l) {
                    HAS_LPARAM
                } else {
                    0
                },
                0,
            ),
            Node::Const(_, ls) => {
                let lp = self
                    .levels
                    .get_list(ls)
                    .iter()
                    .any(|l| self.levels.has_param(*l));
                (0, 0, if lp { HAS_LPARAM } else { 0 }, 0)
            }
            Node::App(f, a) => {
                let (x, y) = (self.entry(f), self.entry(a));
                (
                    x.lbr.max(y.lbr),
                    x.mask | y.mask,
                    x.flags | y.flags,
                    x.spine.saturating_add(1),
                )
            }
            Node::Lam(t, b) | Node::Pi(t, b) => {
                let (x, y) = (self.entry(t), self.entry(b));
                (
                    x.lbr.max(y.lbr.saturating_sub(1)),
                    x.mask | (y.mask >> 1),
                    x.flags | y.flags,
                    0,
                )
            }
            Node::Let(t, v, b) => {
                let (x, y, z) = (self.entry(t), self.entry(v), self.entry(b));
                let lbr = x.lbr.max(y.lbr).max(z.lbr.saturating_sub(1));
                (
                    lbr,
                    x.mask | y.mask | (z.mask >> 1),
                    x.flags | y.flags | z.flags,
                    0,
                )
            }
            Node::NatLit(_) | Node::StrLit(_) | Node::Lit(_) | Node::Nil => (0, 0, 0, 0),
            Node::Proj(_, _, e) => {
                let x = self.entry(e);
                (x.lbr, x.mask, x.flags, 0)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Seq(..) => {
                self.logos_entry(node)
            }
        };
        Entry {
            node,
            mask,
            lbr,
            flags,
            spine,
        }
    }

    /// [`Store::entry_of`] for LOGOS's nodes, out of line: a Lean term never reaches these arms, and
    /// inlined into `intern` they would cost every node either theory interns.
    #[inline(never)]
    fn logos_entry(&self, node: Node) -> (u32, u64, u8, u16) {
        match node {
            Node::Match(d, m, c) => {
                let (x, y, z) = (self.entry(d), self.entry(m), self.entry(c));
                (
                    x.lbr.max(y.lbr).max(z.lbr),
                    x.mask | y.mask | z.mask,
                    x.flags | y.flags | z.flags,
                    0,
                )
            }
            Node::Fix(b) => {
                let x = self.entry(b);
                (x.lbr.saturating_sub(1), x.mask >> 1, x.flags, 0)
            }
            Node::MutualFix(ds, _) => {
                // The `spine` of a `Seq` is its length: the binders every member is under.
                let x = self.entry(ds);
                let n = x.spine as u32;
                let mask = if n >= 64 { 0 } else { x.mask >> n };
                let wide = if n >= 64 && x.lbr > n { WIDE_BVAR } else { 0 };
                (x.lbr.saturating_sub(n), mask, x.flags | wide, 0)
            }
            Node::Seq(h, t) => {
                let (x, y) = (self.entry(h), self.entry(t));
                (
                    x.lbr.max(y.lbr),
                    x.mask | y.mask,
                    x.flags | y.flags,
                    y.spine.saturating_add(1),
                )
            }
            other => unreachable!("not a LOGOS node: {other:?}"),
        }
    }

    #[inline]
    pub fn node(&self, e: ExprId) -> Node {
        self.entry(e).node
    }

    /// One more than the largest loose de Bruijn index (0 when closed).
    #[inline]
    pub fn loose_bvar_range(&self, e: ExprId) -> u32 {
        self.lbr_of(e)
    }

    /// [`Store::loose_bvar_range`] and [`Store::loose_bvar_mask`] in one read.
    #[inline]
    pub fn loose_bvars(&self, e: ExprId) -> (u32, Option<u64>) {
        let x = self.entry(e);
        (
            x.lbr,
            if x.flags & WIDE_BVAR != 0 {
                None
            } else {
                Some(x.mask)
            },
        )
    }

    /// [`Store::loose_bvars`] and [`Store::has_lparam`] in one read: everything outside a term
    /// that its meaning can depend on.
    #[inline]
    pub fn scope_of(&self, e: ExprId) -> (u32, Option<u64>, bool) {
        let x = self.entry(e);
        (
            x.lbr,
            if x.flags & WIDE_BVAR != 0 {
                None
            } else {
                Some(x.mask)
            },
            x.flags & HAS_LPARAM != 0,
        )
    }

    /// Which loose de Bruijn indices occur — bit `i` for `BVar(i)` — when that is known exactly:
    /// `None` if a bound variable with index 64 or more occurs anywhere in `e` (its bit could not
    /// be recorded, and may have come into range under binders).
    #[inline]
    pub fn loose_bvar_mask(&self, e: ExprId) -> Option<u64> {
        if self.flags_of(e) & WIDE_BVAR != 0 {
            None
        } else {
            Some(self.mask_of(e))
        }
    }

    #[inline]
    pub fn has_loose_bvars(&self, e: ExprId) -> bool {
        self.lbr_of(e) > 0
    }

    #[inline]
    pub fn has_fvar(&self, e: ExprId) -> bool {
        self.flags_of(e) & HAS_FVAR != 0
    }

    #[inline]
    pub fn has_lparam(&self, e: ExprId) -> bool {
        self.flags_of(e) & HAS_LPARAM != 0
    }

    // ---- constructors ----

    pub fn bvar(&mut self, i: u32) -> ExprId {
        self.intern(Node::BVar(i))
    }
    pub fn fvar(&mut self, i: u32) -> ExprId {
        self.intern(Node::FVar(i))
    }
    pub fn sort(&mut self, l: LevelId) -> ExprId {
        self.intern(Node::Sort(l))
    }
    pub fn prop(&mut self) -> ExprId {
        self.sort(LevelId::ZERO)
    }
    pub fn cnst(&mut self, n: NameId, ls: LevelsId) -> ExprId {
        self.intern(Node::Const(n, ls))
    }
    pub fn cnst_levels(&mut self, n: NameId, ls: &[LevelId]) -> ExprId {
        let l = self.levels.list(ls);
        self.cnst(n, l)
    }
    pub fn app(&mut self, f: ExprId, a: ExprId) -> ExprId {
        self.intern(Node::App(f, a))
    }
    pub fn lam(&mut self, t: ExprId, b: ExprId) -> ExprId {
        self.intern(Node::Lam(t, b))
    }
    pub fn pi(&mut self, t: ExprId, b: ExprId) -> ExprId {
        self.intern(Node::Pi(t, b))
    }
    pub fn let_(&mut self, t: ExprId, v: ExprId, b: ExprId) -> ExprId {
        self.intern(Node::Let(t, v, b))
    }
    pub fn proj(&mut self, s: NameId, i: u32, e: ExprId) -> ExprId {
        self.intern(Node::Proj(s, i, e))
    }
    pub fn nat_lit(&mut self, n: Nat) -> ExprId {
        let id = self.nat_id(n);
        self.intern(Node::NatLit(id))
    }
    /// The interned id of a natural number (the table is append-only: ids never go stale).
    pub fn nat_id(&mut self, n: Nat) -> NatId {
        match self.nat_index.get(&n) {
            Some(id) => *id,
            None => {
                let id = NatId(self.nats.len() as u32);
                self.nats.push(n.clone());
                self.nat_index.insert(n, id);
                id
            }
        }
    }
    pub fn str_lit(&mut self, s: &str) -> ExprId {
        let id = match self.str_index.get(s) {
            Some(id) => *id,
            None => {
                let rc: Rc<str> = Rc::from(s);
                let id = StrId(self.strs.len() as u32);
                self.strs.push(rc.clone());
                self.str_index.insert(rc, id);
                id
            }
        };
        self.intern(Node::StrLit(id))
    }
    pub fn nat(&self, n: NatId) -> &Nat {
        &self.nats[n.0 as usize]
    }
    pub fn lit(&mut self, l: Lit) -> ExprId {
        let id = match self.lit_index.get(&l) {
            Some(id) => *id,
            None => {
                let id = LitId(self.lits.len() as u32);
                self.lits.push(l.clone());
                self.lit_index.insert(l, id);
                id
            }
        };
        self.intern(Node::Lit(id))
    }
    pub fn lit_value(&self, l: LitId) -> &Lit {
        &self.lits[l.0 as usize]
    }
    pub fn seq(&mut self, items: &[ExprId]) -> ExprId {
        let nil = self.intern(Node::Nil);
        items
            .iter()
            .rev()
            .fold(nil, |tail, h| self.intern(Node::Seq(*h, tail)))
    }
    /// The elements of a [`Node::Seq`] chain, first to last.
    pub fn seq_items(&self, mut e: ExprId) -> Args {
        let mut out = Args::new();
        while let Node::Seq(h, t) = self.node(e) {
            out.push(h);
            e = t;
        }
        debug_assert!(matches!(self.node(e), Node::Nil), "a sequence ends in Nil");
        out
    }
    pub fn match_(&mut self, disc: ExprId, motive: ExprId, cases: &[ExprId]) -> ExprId {
        let cs = self.seq(cases);
        self.intern(Node::Match(disc, motive, cs))
    }
    pub fn fix(&mut self, body: ExprId) -> ExprId {
        self.intern(Node::Fix(body))
    }
    pub fn mutual_fix(&mut self, defs: &[ExprId], index: u32) -> ExprId {
        let ds = self.seq(defs);
        self.intern(Node::MutualFix(ds, index))
    }
    pub fn string(&self, s: StrId) -> Rc<str> {
        self.strs[s.0 as usize].clone()
    }

    pub fn mk_app_n(&mut self, f: ExprId, args: &[ExprId]) -> ExprId {
        args.iter().fold(f, |acc, a| self.app(acc, *a))
    }

    /// The head of an application spine.
    pub fn app_fn(&self, mut e: ExprId) -> ExprId {
        while let Node::App(f, _) = self.node(e) {
            e = f;
        }
        e
    }

    /// Head and arguments (in application order), filled last to first into a buffer of the
    /// recorded spine length.
    pub fn app_args(&self, e: ExprId) -> (ExprId, Args) {
        let n = self.entry(e).spine;
        if n == SPINE_UNKNOWN {
            return self.app_args_walk(e);
        }
        let mut args: Args = smallvec::smallvec![ExprId(0); n as usize];
        let mut cur = e;
        for slot in args.iter_mut().rev() {
            let Node::App(f, a) = self.node(cur) else {
                unreachable!("a recorded spine is made of applications")
            };
            *slot = a;
            cur = f;
        }
        (cur, args)
    }

    /// Head and argument count of a spine of at most [`SHORT_SPINE`] applications, its arguments
    /// filled last to first into `out`: no allocation and no copy out. `None` for a longer spine,
    /// whose arguments [`Store::app_args`] collects.
    #[inline]
    pub fn short_app_args(
        &self,
        e: ExprId,
        out: &mut [ExprId; SHORT_SPINE],
    ) -> Option<(ExprId, usize)> {
        let n = self.entry(e).spine as usize;
        if n > SHORT_SPINE {
            return None;
        }
        let mut cur = e;
        for slot in out[..n].iter_mut().rev() {
            let Node::App(f, a) = self.node(cur) else {
                unreachable!("a recorded spine is made of applications")
            };
            *slot = a;
            cur = f;
        }
        Some((cur, n))
    }

    /// [`Store::app_args`] for a spine too long to record.
    #[cold]
    fn app_args_walk(&self, mut e: ExprId) -> (ExprId, Args) {
        let mut args = Args::new();
        while let Node::App(f, a) = self.node(e) {
            args.push(a);
            e = f;
        }
        args.reverse();
        (e, args)
    }

    pub fn app_num_args(&self, mut e: ExprId) -> usize {
        let n = self.entry(e).spine;
        if n != SPINE_UNKNOWN {
            return n as usize;
        }
        let mut n = 0;
        while let Node::App(f, _) = self.node(e) {
            n += 1;
            e = f;
        }
        n
    }

    pub fn const_head(&self, e: ExprId) -> Option<(NameId, LevelsId)> {
        match self.node(self.app_fn(e)) {
            Node::Const(n, ls) => Some((n, ls)),
            _ => None,
        }
    }

    // ---- substitution ----

    /// Replaces loose `BVar(i)`, `i < subst.len()`, by `subst[i]` (`rev = false`) or by
    /// `subst[len - 1 - i]` (`rev = true`), and lowers the remaining loose indices by `len`.
    pub fn instantiate_core(&mut self, e: ExprId, subst: &[ExprId], rev: bool) -> ExprId {
        if subst.is_empty() || !self.has_loose_bvars(e) {
            return e;
        }
        self.inst_memo.reset();
        self.inst_go(e, 0, subst, rev)
    }

    pub fn instantiate(&mut self, e: ExprId, subst: &[ExprId]) -> ExprId {
        self.instantiate_core(e, subst, false)
    }

    pub fn instantiate_rev(&mut self, e: ExprId, subst: &[ExprId]) -> ExprId {
        self.instantiate_core(e, subst, true)
    }

    pub fn instantiate1(&mut self, e: ExprId, v: ExprId) -> ExprId {
        self.instantiate_core(e, &[v], false)
    }

    fn inst_go(&mut self, e: ExprId, off: u32, subst: &[ExprId], rev: bool) -> ExprId {
        if self.lbr_of(e) <= off {
            return e;
        }
        if let Some(r) = self.inst_memo.get(e, off) {
            return r;
        }
        let n = subst.len() as u32;
        let r = match self.node(e) {
            Node::BVar(i) => {
                if i < off + n {
                    let k = (i - off) as usize;
                    let v = if rev {
                        subst[subst.len() - 1 - k]
                    } else {
                        subst[k]
                    };
                    self.lift_loose(v, 0, off)
                } else {
                    self.bvar(i - n)
                }
            }
            Node::App(f, a) => {
                let (f2, a2) = (
                    self.inst_go(f, off, subst, rev),
                    self.inst_go(a, off, subst, rev),
                );
                self.app(f2, a2)
            }
            Node::Lam(t, b) => {
                let (t2, b2) = (
                    self.inst_go(t, off, subst, rev),
                    self.inst_go(b, off + 1, subst, rev),
                );
                self.lam(t2, b2)
            }
            Node::Pi(t, b) => {
                let (t2, b2) = (
                    self.inst_go(t, off, subst, rev),
                    self.inst_go(b, off + 1, subst, rev),
                );
                self.pi(t2, b2)
            }
            Node::Let(t, v, b) => {
                let t2 = self.inst_go(t, off, subst, rev);
                let v2 = self.inst_go(v, off, subst, rev);
                let b2 = self.inst_go(b, off + 1, subst, rev);
                self.let_(t2, v2, b2)
            }
            Node::Proj(s, i, x) => {
                let x2 = self.inst_go(x, off, subst, rev);
                self.proj(s, i, x2)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Seq(..) => {
                self.inst_logos(e, off, subst, rev)
            }
            _ => e,
        };
        self.inst_memo.insert(e, off, r);
        r
    }

    // The walks' arms for LOGOS's nodes, out of line. A Lean term never reaches them, and the closure
    // each builds would otherwise sit in the frame of every call of the hottest recursions in the
    // checker, for both theories.
    #[inline(never)]
    fn inst_logos(&mut self, e: ExprId, off: u32, subst: &[ExprId], rev: bool) -> ExprId {
        self.map_logos(e, &mut |st, c, k| st.inst_go(c, off + k, subst, rev))
    }
    #[inline(never)]
    fn lift_logos(&mut self, e: ExprId, s: u32, d: u32) -> ExprId {
        self.map_logos(e, &mut |st, c, k| st.lift_go(c, s + k, d))
    }
    #[inline(never)]
    fn abs_logos(&mut self, e: ExprId, off: u32, fvars: &[u32]) -> ExprId {
        self.map_logos(e, &mut |st, c, k| st.abs_go(c, off + k, fvars))
    }
    #[inline(never)]
    fn lp_logos(&mut self, e: ExprId, ps: &[NameId], us: &[LevelId]) -> ExprId {
        self.map_logos(e, &mut |st, c, _| st.lp_go(c, ps, us))
    }

    /// Rebuilds a LOGOS node ([`Node::Match`], [`Node::Fix`], [`Node::MutualFix`], [`Node::Seq`])
    /// from its children, each mapped by `f` given the number of binders the node puts above it.
    fn map_logos(
        &mut self,
        e: ExprId,
        f: &mut dyn FnMut(&mut Store, ExprId, u32) -> ExprId,
    ) -> ExprId {
        match self.node(e) {
            Node::Match(d, m, c) => {
                let (d2, m2, c2) = (f(self, d, 0), f(self, m, 0), f(self, c, 0));
                self.intern(Node::Match(d2, m2, c2))
            }
            Node::Fix(b) => {
                let b2 = f(self, b, 1);
                self.intern(Node::Fix(b2))
            }
            Node::MutualFix(ds, i) => {
                let n = self.entry(ds).spine as u32;
                let ds2 = f(self, ds, n);
                self.intern(Node::MutualFix(ds2, i))
            }
            Node::Seq(h, t) => {
                let (h2, t2) = (f(self, h, 0), f(self, t, 0));
                self.intern(Node::Seq(h2, t2))
            }
            other => unreachable!("not a LOGOS node with children: {other:?}"),
        }
    }

    /// Adds `d` to every loose index `≥ s`.
    pub fn lift_loose(&mut self, e: ExprId, s: u32, d: u32) -> ExprId {
        if d == 0 || self.lbr_of(e) <= s {
            return e;
        }
        self.lift_memo.reset();
        self.lift_go(e, s, d)
    }

    fn lift_go(&mut self, e: ExprId, s: u32, d: u32) -> ExprId {
        if self.lbr_of(e) <= s {
            return e;
        }
        if let Some(r) = self.lift_memo.get(e, s) {
            return r;
        }
        let r = match self.node(e) {
            Node::BVar(i) => self.bvar(i + d),
            Node::App(f, a) => {
                let (f2, a2) = (self.lift_go(f, s, d), self.lift_go(a, s, d));
                self.app(f2, a2)
            }
            Node::Lam(t, b) => {
                let (t2, b2) = (self.lift_go(t, s, d), self.lift_go(b, s + 1, d));
                self.lam(t2, b2)
            }
            Node::Pi(t, b) => {
                let (t2, b2) = (self.lift_go(t, s, d), self.lift_go(b, s + 1, d));
                self.pi(t2, b2)
            }
            Node::Let(t, v, b) => {
                let t2 = self.lift_go(t, s, d);
                let v2 = self.lift_go(v, s, d);
                let b2 = self.lift_go(b, s + 1, d);
                self.let_(t2, v2, b2)
            }
            Node::Proj(n, i, x) => {
                let x2 = self.lift_go(x, s, d);
                self.proj(n, i, x2)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Seq(..) => {
                self.lift_logos(e, s, d)
            }
            _ => e,
        };
        self.lift_memo.insert(e, s, r);
        r
    }

    /// Replaces `FVar(fvars[i])` by the bound variable of the `i`-th of `fvars.len()` enclosing
    /// binders (the last free variable becomes `BVar(0)`), as `mkForall`/`mkLambda` require.
    pub fn abstract_fvars(&mut self, e: ExprId, fvars: &[u32]) -> ExprId {
        if fvars.is_empty() || !self.has_fvar(e) {
            return e;
        }
        self.abs_memo.reset();
        self.abs_go(e, 0, fvars)
    }

    fn abs_go(&mut self, e: ExprId, off: u32, fvars: &[u32]) -> ExprId {
        if !self.has_fvar(e) {
            return e;
        }
        if let Some(r) = self.abs_memo.get(e, off) {
            return r;
        }
        let r = match self.node(e) {
            Node::FVar(x) => match fvars.iter().rposition(|f| *f == x) {
                Some(i) => self.bvar(off + (fvars.len() - 1 - i) as u32),
                None => e,
            },
            Node::App(f, a) => {
                let (f2, a2) = (self.abs_go(f, off, fvars), self.abs_go(a, off, fvars));
                self.app(f2, a2)
            }
            Node::Lam(t, b) => {
                let (t2, b2) = (self.abs_go(t, off, fvars), self.abs_go(b, off + 1, fvars));
                self.lam(t2, b2)
            }
            Node::Pi(t, b) => {
                let (t2, b2) = (self.abs_go(t, off, fvars), self.abs_go(b, off + 1, fvars));
                self.pi(t2, b2)
            }
            Node::Let(t, v, b) => {
                let t2 = self.abs_go(t, off, fvars);
                let v2 = self.abs_go(v, off, fvars);
                let b2 = self.abs_go(b, off + 1, fvars);
                self.let_(t2, v2, b2)
            }
            Node::Proj(n, i, x) => {
                let x2 = self.abs_go(x, off, fvars);
                self.proj(n, i, x2)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Seq(..) => {
                self.abs_logos(e, off, fvars)
            }
            _ => e,
        };
        self.abs_memo.insert(e, off, r);
        r
    }

    /// Substitutes universe parameters `ps[i] := us[i]` throughout `e`.
    pub fn instantiate_lparams(&mut self, e: ExprId, ps: &[NameId], us: &[LevelId]) -> ExprId {
        if ps.is_empty() || !self.has_lparam(e) {
            return e;
        }
        self.lp_memo.reset();
        self.lp_go(e, ps, us)
    }

    fn lp_go(&mut self, e: ExprId, ps: &[NameId], us: &[LevelId]) -> ExprId {
        if !self.has_lparam(e) {
            return e;
        }
        if let Some(r) = self.lp_memo.get(e, 0) {
            return r;
        }
        let r = match self.node(e) {
            Node::Sort(l) => {
                let l2 = self.levels.instantiate(l, ps, us);
                self.sort(l2)
            }
            Node::Const(n, ls) => {
                let old: Vec<LevelId> = self.levels.get_list(ls).to_vec();
                let new: Vec<LevelId> = old
                    .iter()
                    .map(|l| self.levels.instantiate(*l, ps, us))
                    .collect();
                self.cnst_levels(n, &new)
            }
            Node::App(f, a) => {
                let (f2, a2) = (self.lp_go(f, ps, us), self.lp_go(a, ps, us));
                self.app(f2, a2)
            }
            Node::Lam(t, b) => {
                let (t2, b2) = (self.lp_go(t, ps, us), self.lp_go(b, ps, us));
                self.lam(t2, b2)
            }
            Node::Pi(t, b) => {
                let (t2, b2) = (self.lp_go(t, ps, us), self.lp_go(b, ps, us));
                self.pi(t2, b2)
            }
            Node::Let(t, v, b) => {
                let t2 = self.lp_go(t, ps, us);
                let v2 = self.lp_go(v, ps, us);
                let b2 = self.lp_go(b, ps, us);
                self.let_(t2, v2, b2)
            }
            Node::Proj(n, i, x) => {
                let x2 = self.lp_go(x, ps, us);
                self.proj(n, i, x2)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Seq(..) => {
                self.lp_logos(e, ps, us)
            }
            _ => e,
        };
        self.lp_memo.insert(e, 0, r);
        r
    }

    /// Does any subterm satisfy `pred`? Each distinct node is visited once.
    pub fn any_node(&self, e: ExprId, pred: &mut dyn FnMut(&Store, ExprId) -> bool) -> bool {
        let mut seen = rustc_hash::FxHashSet::default();
        let mut stack = vec![e];
        while let Some(x) = stack.pop() {
            if !seen.insert(x) {
                continue;
            }
            if pred(self, x) {
                return true;
            }
            match self.node(x) {
                Node::App(a, b) | Node::Lam(a, b) | Node::Pi(a, b) | Node::Seq(a, b) => {
                    stack.push(a);
                    stack.push(b);
                }
                Node::Let(a, b, c) | Node::Match(a, b, c) => {
                    stack.push(a);
                    stack.push(b);
                    stack.push(c);
                }
                Node::Proj(_, _, a) | Node::Fix(a) | Node::MutualFix(a, _) => stack.push(a),
                _ => {}
            }
        }
        false
    }

    /// Does `e` mention constant `n`?
    pub fn mentions_const(&self, e: ExprId, names: &[NameId]) -> bool {
        self.any_node(
            e,
            &mut |s, x| matches!(s.node(x), Node::Const(c, _) if names.contains(&c)),
        )
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/expr/tests.rs"]
mod tests;
