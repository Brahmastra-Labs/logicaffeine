//! Core runtime type definitions.
//!
//! This module defines the primitive types used by LOGOS programs at runtime.
//! These are type aliases that map LOGOS types to their Rust equivalents.
//!
//! ## Type Mappings
//!
//! | LOGOS Type | Rust Type | Description |
//! |------------|-----------|-------------|
//! | `Nat` | `u64` | Natural numbers (non-negative) |
//! | `Int` | `i64` | Signed integers |
//! | `Real` | `f64` | Floating-point numbers |
//! | `Text` | `String` | UTF-8 strings |
//! | `Bool` | `bool` | Boolean values |
//! | `Unit` | `()` | The unit type |
//! | `Char` | `char` | Unicode scalar values |
//! | `Byte` | `u8` | Raw bytes |
//! | `Seq<T>` | `LogosSeq<T>` | Ordered sequences (reference semantics) |
//! | `Set<T>` | `HashSet<T>` | Unordered unique elements |
//! | `Map<K,V>` | `LogosMap<K,V>` | Key-value mappings (reference semantics) |

use std::cell::RefCell;
use std::hash::Hash;
use std::rc::Rc;

/// Non-negative integers. Maps to Peano `Nat` in the kernel.
pub type Nat = u64;
/// Signed integers.
pub type Int = i64;
/// IEEE 754 floating-point numbers.
pub type Real = f64;
/// UTF-8 encoded text strings.
pub type Text = String;
/// Boolean truth values.
pub type Bool = bool;
/// The unit type (single value).
pub type Unit = ();
/// Unicode scalar values.
pub type Char = char;
/// Raw bytes (0-255).
pub type Byte = u8;

/// Ordered sequence with reference semantics.
///
/// `LogosSeq<T>` wraps `Rc<RefCell<Vec<T>>>` to provide shared mutable access.
/// Cloning a `LogosSeq` produces a shallow copy (shared reference), not a deep copy.
/// Use `.deep_clone()` for an independent copy (LOGOS `copy of`).
#[derive(Debug)]
pub struct LogosSeq<T>(pub Rc<RefCell<Vec<T>>>);

impl<T> LogosSeq<T> {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    pub fn from_vec(v: Vec<T>) -> Self {
        Self(Rc::new(RefCell::new(v)))
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(Rc::new(RefCell::new(Vec::with_capacity(cap))))
    }

    pub fn push(&self, value: T) {
        self.0.borrow_mut().push(value);
    }

    pub fn pop(&self) -> Option<T> {
        self.0.borrow_mut().pop()
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    pub fn remove(&self, index: usize) -> T {
        self.0.borrow_mut().remove(index)
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, Vec<T>> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, Vec<T>> {
        self.0.borrow_mut()
    }
}

/// A FILL-SLOT copy: collections copy DEEP, so every slot of a fill
/// (`n copies of x`, `[x] * n`) is an independent row — never `n` aliases of
/// one row (the classic `[[0]] * 3` footgun is designed out). Scalars copy
/// plain. Mirrors the tree-walker's recursive `RuntimeValue::deep_clone`.
pub trait FillClone {
    fn fill_clone(&self) -> Self;
}

macro_rules! fill_by_clone {
    ($($t:ty),* $(,)?) => {
        $(impl FillClone for $t {
            #[inline]
            fn fill_clone(&self) -> Self {
                self.clone()
            }
        })*
    };
}

fill_by_clone!(i8, i16, i32, i64, i128, u8, u16, u32, u64, usize, f32, f64, bool, char, String);

impl<T: FillClone> FillClone for LogosSeq<T> {
    fn fill_clone(&self) -> Self {
        Self(Rc::new(RefCell::new(
            self.0.borrow().iter().map(|e| e.fill_clone()).collect(),
        )))
    }
}

impl<K: Clone + Eq + Hash, V: FillClone> FillClone for LogosMap<K, V> {
    fn fill_clone(&self) -> Self {
        Self(Rc::new(RefCell::new(
            self.0
                .borrow()
                .iter()
                .map(|(k, v)| (k.clone(), v.fill_clone()))
                .collect(),
        )))
    }
}

impl<T: Clone> LogosSeq<T> {
    pub fn deep_clone(&self) -> Self {
        Self(Rc::new(RefCell::new(self.0.borrow().clone())))
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.0.borrow().clone()
    }

    pub fn extend_from_slice(&self, other: &[T]) {
        self.0.borrow_mut().extend_from_slice(other);
    }

    pub fn iter(&self) -> LogosSeqIter<T> {
        LogosSeqIter {
            data: self.to_vec(),
            pos: 0,
        }
    }
}

pub struct LogosSeqIter<T> {
    data: Vec<T>,
    pos: usize,
}

impl<T: Clone> Iterator for LogosSeqIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.pos < self.data.len() {
            let val = self.data[self.pos].clone();
            self.pos += 1;
            Some(val)
        } else {
            None
        }
    }
}

impl<T: Ord> LogosSeq<T> {
    pub fn sort(&self) {
        self.0.borrow_mut().sort();
    }
}

impl<T> LogosSeq<T> {
    pub fn reverse(&self) {
        self.0.borrow_mut().reverse();
    }
}

/// Whether Mutable Value Semantics is enabled for compiled (AOT) programs. The
/// AOT binary is a separate process from the interpreter, so it reads the
/// `LOGOS_VALUE_SEMANTICS` env var itself (inherited from the parent process),
/// cached once. When on, collection `Clone` deep-copies (value semantics); when
/// off it shares the `Rc` (the historical reference semantics — unchanged).
pub fn value_semantics_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    // Value semantics is the DEFAULT; `LOGOS_VALUE_SEMANTICS=0` restores the
    // historical reference semantics. Matches the compile-crate gate.
    *ON.get_or_init(|| std::env::var("LOGOS_VALUE_SEMANTICS").as_deref() != Ok("0"))
}

impl<T: Clone> Clone for LogosSeq<T> {
    fn clone(&self) -> Self {
        // Both semantics now SHARE the allocation on `Clone` — a cheap `Rc`
        // bump. Under value semantics, isolation is preserved LAZILY by `cow()`
        // (below), which codegen/the engines call before an in-place mutation:
        // clone-on-write instead of the historical clone-on-copy. Reference
        // semantics never calls `cow()`, so the share is permanent (its
        // historical behavior). This is the "best of all worlds" model: aliases
        // share until one of them diverges via mutation.
        Self(Rc::clone(&self.0))
    }
}

impl<T: Clone> LogosSeq<T> {
    /// Copy-on-write: make this handle uniquely own its buffer before an
    /// in-place mutation, so the mutation cannot leak into a value-semantics
    /// sibling that shares the same allocation. A no-op when already unique
    /// (`strong_count == 1`), so the common case pays only a count read.
    /// Mirrors the tree-walker's `ensure_collection_owned` and the VM's
    /// `ensure_reg_owned`.
    #[inline]
    pub fn cow(&mut self) {
        if Rc::strong_count(&self.0) > 1 {
            // Bind the clone first so the `borrow()` temporary is released before the reassignment
            // (edition-2024 extends the temporary's lifetime to the end of the statement otherwise).
            let cloned = self.0.borrow().clone();
            self.0 = Rc::new(RefCell::new(cloned));
        }
    }
}

/// A no-op `cow()` for an OWNED `Vec` — a scalarized (non-shared) sequence never aliases, so the
/// copy-on-write barrier the codegen emits before an in-place mutation is unnecessary. This keeps the
/// codegen's `cow()` emission TOTAL: whether a `Seq` lowered to a shared `LogosSeq` (real copy-on-write)
/// or a scalarized owned `Vec<T>` (a proven-non-aliased buffer — e.g. a `Seq of Real` → `Vec<f64>`),
/// `<seq>.cow()` always type-checks. Without this, a scalarized float/int vector whose `variable_types`
/// entry still read `LogosSeq` made the emitter write `vec.cow()` on a `Vec`, which has no such method
/// (`error[E0599]: no method named `cow``), breaking every float-sequence program on the compiled tiers.
/// `LogosSeq`'s inherent `cow()` shadows this trait method, so shared handles keep the real barrier.
pub trait VecCowNoop {
    #[inline]
    fn cow(&mut self) {}
}
impl<T> VecCowNoop for Vec<T> {}

impl<T: Clone> LogosSeq<LogosSeq<T>> {
    /// Value-semantic nested write `Set item i of item k of grid to v` (the through-write fast path):
    /// copy-on-write the ROW `k` names (deep-copy only if its buffer is shared with a value-semantics
    /// sibling), then set the item `i` names in that row in place. Both are 1-based LOGOS indices under
    /// [`crate::resolve_logos_index`], the row's first. The compiled mirror of the parser's
    /// clone-modify-writeback place desugar, but O(1) when the row is uniquely owned — no full-row clone.
    /// The caller `cow()`s the OUTER handle first (as the codegen emits), so a shared `grid` is never
    /// mutated in place.
    #[inline]
    pub fn set_nested(&self, k: i64, i: i64, v: T) {
        use crate::LogosItems;
        let mut outer = self.borrow_mut();
        let row = outer.logos_item_mut(k);
        row.cow();
        *row.borrow_mut().logos_item_mut(i) = v;
    }
}

impl<T> Default for LogosSeq<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: PartialEq> PartialEq for LogosSeq<T> {
    fn eq(&self, other: &Self) -> bool {
        *self.0.borrow() == *other.0.borrow()
    }
}

impl<T: std::fmt::Display> std::fmt::Display for LogosSeq<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.0.borrow();
        write!(f, "[")?;
        for (i, item) in inner.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", item)?;
        }
        write!(f, "]")
    }
}

impl<T> From<Vec<T>> for LogosSeq<T> {
    fn from(v: Vec<T>) -> Self {
        Self::from_vec(v)
    }
}

impl<T: serde::Serialize> serde::Serialize for LogosSeq<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.borrow().serialize(serializer)
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for LogosSeq<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let vec = Vec::<T>::deserialize(deserializer)?;
        Ok(Self::from_vec(vec))
    }
}

impl<T: PartialEq> LogosContains<T> for LogosSeq<T> {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.0.borrow().contains(value)
    }
}

impl<T: Clone> IntoIterator for LogosSeq<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.to_vec().into_iter()
    }
}

/// The insertion-ordered map behind every LOGOS `Map`: `IndexMap` for the
/// order contract (iteration, display, and serialization follow insertion,
/// like a Python dict), Fx hashing for speed on the small keys LOGOS programs
/// use (no DoS-resistance requirement).
pub type FxIndexMap<K, V> =
    indexmap::IndexMap<K, V, std::hash::BuildHasherDefault<rustc_hash::FxHasher>>;

/// Key-value mapping with reference semantics.
///
/// `LogosMap<K, V>` wraps `Rc<RefCell<FxIndexMap<K, V>>>` to provide shared mutable access.
/// Cloning a `LogosMap` produces a shallow copy (shared reference), not a deep copy.
/// Use `.deep_clone()` for an independent copy (LOGOS `copy of`).
#[derive(Debug)]
pub struct LogosMap<K, V>(pub Rc<RefCell<FxIndexMap<K, V>>>);

impl<K: Eq + Hash, V> LogosMap<K, V> {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(FxIndexMap::default())))
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(Rc::new(RefCell::new(FxIndexMap::with_capacity_and_hasher(
            cap,
            Default::default(),
        ))))
    }

    pub fn from_map(m: FxIndexMap<K, V>) -> Self {
        Self(Rc::new(RefCell::new(m)))
    }

    pub fn insert(&self, key: K, value: V) -> Option<V> {
        self.0.borrow_mut().insert(key, value)
    }

    /// Removes a key while PRESERVING the insertion order of the remaining
    /// entries (`shift_remove` — Python `del` semantics, O(n)).
    pub fn remove(&self, key: &K) -> Option<V> {
        self.0.borrow_mut().shift_remove(key)
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.0.borrow().contains_key(key)
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, FxIndexMap<K, V>> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, FxIndexMap<K, V>> {
        self.0.borrow_mut()
    }
}

impl<K: Eq + Hash + Clone, V: Clone> LogosMap<K, V> {
    pub fn deep_clone(&self) -> Self {
        Self(Rc::new(RefCell::new(self.0.borrow().clone())))
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.0.borrow().get(key).cloned()
    }

    pub fn values(&self) -> Vec<V> {
        self.0.borrow().values().cloned().collect()
    }

    pub fn keys(&self) -> Vec<K> {
        self.0.borrow().keys().cloned().collect()
    }
}

impl<K: Clone, V: Clone> Clone for LogosMap<K, V> {
    fn clone(&self) -> Self {
        // Share on `Clone` under both semantics; value-semantics isolation is
        // preserved lazily by `cow()` before an in-place mutation. See
        // `LogosSeq::clone`/`cow` for the full rationale.
        Self(Rc::clone(&self.0))
    }
}

impl<K: Clone, V: Clone> LogosMap<K, V> {
    /// Copy-on-write: make this handle uniquely own its map before an in-place
    /// mutation. See [`LogosSeq::cow`].
    #[inline]
    pub fn cow(&mut self) {
        if Rc::strong_count(&self.0) > 1 {
            // Bind the clone first so the `borrow()` temporary is released before the reassignment
            // (edition-2024 extends the temporary's lifetime to the end of the statement otherwise).
            let cloned = self.0.borrow().clone();
            self.0 = Rc::new(RefCell::new(cloned));
        }
    }
}

impl<K: Eq + Hash, V> Default for LogosMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: PartialEq + Eq + Hash, V: PartialEq> PartialEq for LogosMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        *self.0.borrow() == *other.0.borrow()
    }
}

impl<K: std::fmt::Display + Eq + Hash, V: std::fmt::Display> std::fmt::Display for LogosMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.0.borrow();
        write!(f, "{{")?;
        for (i, (k, v)) in inner.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: {}", k, v)?;
        }
        write!(f, "}}")
    }
}

impl<K: serde::Serialize + Eq + Hash, V: serde::Serialize> serde::Serialize for LogosMap<K, V> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.borrow().serialize(serializer)
    }
}

impl<'de, K: serde::Deserialize<'de> + Eq + Hash, V: serde::Deserialize<'de>>
    serde::Deserialize<'de> for LogosMap<K, V>
{
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = FxIndexMap::<K, V>::deserialize(deserializer)?;
        Ok(Self::from_map(map))
    }
}

/// Iterating a map yields `(key, value)` pairs in INSERTION order — the same
/// order the tree-walker and VM iterate. Snapshot semantics (like
/// [`LogosSeq`]'s `IntoIterator`): the entries are collected up front, so
/// mutating the map inside the loop never invalidates the iteration.
impl<K: Clone, V: Clone> IntoIterator for LogosMap<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;

    fn into_iter(self) -> Self::IntoIter {
        let entries: Vec<(K, V)> = self
            .0
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        entries.into_iter()
    }
}

impl<K: Eq + Hash, V> LogosContains<K> for LogosMap<K, V> {
    #[inline(always)]
    fn logos_contains(&self, key: &K) -> bool {
        self.0.borrow().contains_key(key)
    }
}

/// The most an open-addressed Int table ([`LogosI64Map`], [`LogosI64Set`], [`LogosI32Map`], [`LogosI32Set`]) is filled:
/// half. A linear-probing miss walks (1 + 1/(1 - α)²) / 2 slots on average, 1.6 at a load of 0.3 and 3.6 at 0.6, and
/// in a table larger than the cache every slot walked is a cache miss: two_sum's set spent a fifth fewer cycles at half
/// full than at three-quarters.
const INT_TABLE_MAX_LOAD: (usize, usize) = (1, 2);

/// The power-of-two slot count (at least 8) that holds `cap` entries within [`INT_TABLE_MAX_LOAD`].
fn int_table_slots(cap: usize) -> usize {
    let (num, den) = INT_TABLE_MAX_LOAD;
    (cap.saturating_mul(den) / num + 1).next_power_of_two().max(8)
}

/// The slot of an Int key in an open-addressed table of `2^(64 - shift)` slots: Fibonacci hashing, the TOP bits of the
/// key times `2^64/φ` (Knuth). The top bits are the ones the multiply mixes from every bit of the key, so keys that
/// differ only high (multiples of `2^32`) spread as sequential ones do, in two instructions, the table C hand-rolls.
#[inline(always)]
fn fib_slot(key: u64, shift: u32) -> usize {
    (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> shift) as usize
}

/// Whether a table of `slots` holding `len` probed entries must grow before it takes another.
fn int_table_full(len: usize, slots: usize) -> bool {
    let (num, den) = INT_TABLE_MAX_LOAD;
    (len + 1) * den > slots * num
}

/// A specialized open-addressing `i64 → i64` map with VALUE semantics and no
/// `Rc<RefCell>` indirection.
///
/// The code generator emits this in place of `LogosMap<i64, i64>` for
/// non-aliased local `Map of Int to Int` variables (see `codegen::i64_map`):
/// linear probing over two flat `Vec`s with `Copy` keys and values — no
/// per-operation `RefCell` borrow, no key/value clone, `&mut self` mutation
/// that LLVM can keep in registers. This is the C open-addressing shape, and it
/// is only selected where the alias analysis proves the map never escapes or is
/// shared, so the loss of reference semantics is invisible to the program.
///
/// `0` is the empty-slot sentinel; the real key `0` is tracked separately so the
/// map is correct for the entire `i64` key space. The sentinel is `0`, not
/// `i64::MIN`, so the probe table allocates via `vec![[0, 0]; slots]` — the bit
/// pattern Rust's `IsZero` specialization lowers to `alloc_zeroed` (calloc):
/// lazily-zeroed pages, no eager memset/page-fault storm up front.
#[derive(Debug, Clone)]
pub struct LogosI64Map {
    /// AoS probe table of `[key, value]` slots; `key == EMPTY` marks empty.
    /// Interleaving key and value keeps both on the SAME cache line, so a lookup
    /// hit reads ONE line, not two (the SoA tax) — matching C's `struct Entry`.
    /// `[i64; 2]` (not a tuple) so the zero fill hits `alloc_zeroed`.
    entries: Vec<[i64; 2]>,
    mask: usize,
    /// `64 - log2(slots)`: a key's slot is the top bits of its Fibonacci product ([`fib_slot`]).
    shift: u32,
    len: usize,
    has_zero_key: bool,
    zero_key_val: i64,
}

impl LogosI64Map {
    const EMPTY: i64 = 0;

    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            mask: 0,
            shift: 64,
            len: 0,
            has_zero_key: false,
            zero_key_val: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut m = Self::new();
        if cap > 0 {
            // Within the most a table is filled, rounded up to a power of two
            // (so the probe mask is `slots - 1`), with a floor of 8 slots.
            let slots = int_table_slots(cap);
            // `[Self::EMPTY, 0] == [0, 0]` → `alloc_zeroed` (calloc): lazily
            // zeroed, no eager memset of the whole table.
            m.entries = vec![[Self::EMPTY, 0]; slots];
            m.mask = slots - 1;
            m.shift = 64 - slots.trailing_zeros();
        }
        m
    }

    /// Live entries stored in the probe table (excludes the zero key, which
    /// lives outside the table).
    #[inline]
    fn table_len(&self) -> usize {
        self.len - self.has_zero_key as usize
    }

    /// See [`LogosI64Set::assume_table_invariant`]: `entries.len() == mask + 1`
    /// once `mask != 0`, so masked probe indices need no bounds check.
    #[inline(always)]
    fn assume_table_invariant(&self) {
        unsafe {
            std::hint::assert_unchecked(self.mask + 1 == self.entries.len());
        }
    }

    #[inline]
    fn slot(&self, key: i64) -> usize {
        fib_slot(key as u64, self.shift)
    }

    #[inline]
    pub fn insert(&mut self, key: i64, value: i64) {
        if key == Self::EMPTY {
            if !self.has_zero_key {
                self.has_zero_key = true;
                self.len += 1;
            }
            self.zero_key_val = value;
            return;
        }
        if self.mask == 0 || int_table_full(self.table_len(), self.mask + 1) {
            self.grow();
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.entries[i][0];
            if k == Self::EMPTY {
                self.entries[i] = [key, value];
                self.len += 1;
                return;
            }
            if k == key {
                self.entries[i][1] = value;
                return;
            }
            i = (i + 1) & self.mask;
        }
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let new_slots = if self.mask == 0 {
            8
        } else {
            (self.mask + 1) * 2
        };
        let old = std::mem::replace(&mut self.entries, vec![[Self::EMPTY, 0]; new_slots]);
        self.mask = new_slots - 1;
        self.shift = 64 - new_slots.trailing_zeros();
        self.assume_table_invariant();
        for &[k, v] in old.iter() {
            if k != Self::EMPTY {
                let mut i = self.slot(k);
                while self.entries[i][0] != Self::EMPTY {
                    i = (i + 1) & self.mask;
                }
                self.entries[i] = [k, v];
            }
        }
    }

    #[inline]
    pub fn get(&self, key: &i64) -> Option<i64> {
        let key = *key;
        if key == Self::EMPTY {
            return if self.has_zero_key {
                Some(self.zero_key_val)
            } else {
                None
            };
        }
        if self.mask == 0 {
            return None;
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let [k, v] = self.entries[i];
            if k == key {
                return Some(v);
            }
            if k == Self::EMPTY {
                return None;
            }
            i = (i + 1) & self.mask;
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosI64Map {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosI64Map {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.get(key).is_some()
    }
}

/// A `Seq of Bool` held one bit per item: the representation a local sequence of truth values takes when every use
/// of it is an item read, an item store, a push or its length. A table of `n` flags takes `n / 8` bytes where a
/// `Vec<bool>` takes `n`, so a sieve over 10^8 numbers fits in the cache it used to stream past.
///
/// Items are counted from 1 as in every sequence, through [`crate::resolve_logos_index`]: an index counted from the
/// back reaches the same item, and one that names no item fails with the sentence every tier reports. The unchecked
/// accessors take a 0-based offset the caller has proven below [`LogosBits::len`].
#[derive(Debug, Clone, Default)]
pub struct LogosBits {
    /// Item `k` (0-based) is bit `k % 64` of word `k / 64`; bits past `len` are zero.
    words: Vec<u64>,
    len: usize,
}

impl LogosBits {
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty sequence with room for `items` items before it grows.
    pub fn with_capacity(items: usize) -> Self {
        LogosBits {
            words: Vec::with_capacity(items.div_ceil(64)),
            len: 0,
        }
    }

    /// `len` items, each `value`.
    pub fn filled(len: usize, value: bool) -> Self {
        let mut words = vec![if value { u64::MAX } else { 0 }; len.div_ceil(64)];
        if value && len % 64 != 0 {
            if let Some(last) = words.last_mut() {
                *last = (1u64 << (len % 64)) - 1;
            }
        }
        LogosBits { words, len }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn push(&mut self, value: bool) {
        if self.len % 64 == 0 {
            self.words.push(0);
        }
        let at = self.len;
        self.len += 1;
        // SAFETY: `at` is below the new `len`, and its word exists.
        unsafe { self.set_unchecked(at, value) }
    }

    /// The item `index` names (counted from 1, or from the back when negative).
    #[inline]
    pub fn item(&self, index: i64) -> bool {
        let at = crate::resolve_logos_index(index, self.len);
        // SAFETY: `resolve_logos_index` returns an offset below `len` or does not return.
        unsafe { self.get_unchecked(at) }
    }

    /// Store `value` as the item `index` names.
    #[inline]
    pub fn set_item(&mut self, index: i64, value: bool) {
        let at = crate::resolve_logos_index(index, self.len);
        // SAFETY: as in `item`.
        unsafe { self.set_unchecked(at, value) }
    }

    /// The item at 0-based `offset`.
    ///
    /// # Safety
    /// `offset` is below [`LogosBits::len`].
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, offset: usize) -> bool {
        debug_assert!(offset < self.len);
        (*self.words.get_unchecked(offset / 64) >> (offset % 64)) & 1 == 1
    }

    /// Store `value` at 0-based `offset`.
    ///
    /// # Safety
    /// `offset` is below [`LogosBits::len`].
    #[inline(always)]
    pub unsafe fn set_unchecked(&mut self, offset: usize, value: bool) {
        debug_assert!(offset < self.len);
        let word = self.words.get_unchecked_mut(offset / 64);
        let bit = 1u64 << (offset % 64);
        *word = (*word & !bit) | ((value as u64) << (offset % 64));
    }

    /// Store `true` at 0-based `offset`: one `or` into its word.
    ///
    /// # Safety
    /// `offset` is below [`LogosBits::len`].
    #[inline(always)]
    pub unsafe fn set_true_unchecked(&mut self, offset: usize) {
        debug_assert!(offset < self.len);
        *self.words.get_unchecked_mut(offset / 64) |= 1u64 << (offset % 64);
    }

    /// Store `value` at the `count` offsets `first`, `first + step`, `first + 2 * step`, ….
    ///
    /// A step of a word or more puts each offset in a word of its own, and a handful of offsets cost no more one at a
    /// time; both store in place. Otherwise [`Self::fill_stride_words`] writes each word the progression crosses once,
    /// out of line, so a loop that fills now and then keeps none of the walk's state in its own registers.
    ///
    /// # Safety
    /// `step` is at least 1 and, when `count` is not 0, `first + (count - 1) * step` is below [`LogosBits::len`].
    #[inline]
    pub unsafe fn fill_stride_unchecked(&mut self, first: usize, count: usize, step: usize, value: bool) {
        debug_assert!(step >= 1);
        debug_assert!(count == 0 || first + (count - 1) * step < self.len);
        if step >= 64 || count < 8 {
            let mut at = first;
            for _ in 0..count {
                let word = self.words.get_unchecked_mut(at / 64);
                let bit = 1u64 << (at % 64);
                *word = if value { *word | bit } else { *word & !bit };
                at += step;
            }
        } else {
            self.fill_stride_words(first, count, step, value);
        }
    }

    /// Store `value` at `first`, `first + step`, … up to `through`, and return how many steps past `first` that took:
    /// a loop's stores from its counter's value on entry to its bound, the bound given as the offset it would store at.
    /// Out of line, so the loop that calls it keeps no arithmetic of the progression in its own registers: what it
    /// computes from its counter is the counter.
    ///
    /// # Safety
    /// `step` is at least 1, `first <= through`, and `first + (through - first) / step * step` is below
    /// [`LogosBits::len`].
    #[inline(never)]
    pub unsafe fn fill_stride_through_unchecked(
        &mut self,
        first: usize,
        through: usize,
        step: usize,
        value: bool,
    ) -> usize {
        debug_assert!(step >= 1 && first <= through);
        let more = (through - first) / step;
        self.fill_stride_unchecked(first, more + 1, step, value);
        more
    }

    /// [`Self::fill_stride_unchecked`] for a step shorter than a word: the offsets in one word are a fixed pattern,
    /// the multiples of `step` below 64, shifted to where the progression enters the word, so each word is written
    /// once. The progression enters the next word `64 % step` bits earlier in the pattern, wrapping by `step`.
    ///
    /// # Safety
    /// As [`Self::fill_stride_unchecked`], with `1 <= step < 64` and `count >= 1`.
    #[inline(never)]
    unsafe fn fill_stride_words(&mut self, first: usize, count: usize, step: usize, value: bool) {
        let last = first + (count - 1) * step;
        let words = self.words.as_mut_ptr();
        let store = |w: usize, mask: u64| {
            let word = words.add(w);
            *word = if value { *word | mask } else { *word & !mask };
        };
        let mut pattern = 0u64;
        let mut k = 0;
        while k < 64 {
            pattern |= 1u64 << k;
            k += step;
        }
        let back = 64 % step;
        let (first_word, last_word) = (first / 64, last / 64);
        let through_last = u64::MAX >> (63 - last % 64);
        let mut phase = first % 64;
        if first_word == last_word {
            store(first_word, (pattern << phase) & through_last);
            return;
        }
        store(first_word, pattern << phase);
        phase = (phase + step - back) % step;
        for w in first_word + 1..last_word {
            store(w, pattern << phase);
            phase = if phase >= back { phase - back } else { phase + step - back };
        }
        store(last_word, (pattern << phase) & through_last);
    }

    /// The items, first to last.
    pub fn iter(&self) -> impl Iterator<Item = bool> + '_ {
        // SAFETY: every offset below `len`.
        (0..self.len).map(move |k| unsafe { self.get_unchecked(k) })
    }
}

impl PartialEq for LogosBits {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.words == other.words
    }
}

impl Eq for LogosBits {}

/// Open-addressing `i64` SET — the keys-only sibling of [`LogosI64Map`], emitted
/// for a `Map of Int to Int` the alias analysis proves is used ONLY as a set
/// (insert + contains; the value is never read — two_sum's `seen`). ONE flat
/// `Vec<i64>` with linear probing and a `0` empty-slot sentinel (the real key
/// `0` tracked separately for full key-space correctness). With no value array
/// it has HALF the store traffic and footprint of the map — 8 bytes/slot,
/// smaller than C's `struct Entry { long key; int occupied; }`.
///
/// The sentinel is `0`, not `i64::MIN`, on purpose: `vec![0; slots]` is the bit
/// pattern Rust's `IsZero` specialization lowers to `alloc_zeroed` (calloc), so
/// the table comes from the OS lazily-zeroed — no eager 1 GiB memset/page-fault
/// storm up front (the kernel-time gap that made two_sum lose to C's `calloc`).
#[derive(Debug, Clone)]
pub struct LogosI64Set {
    keys: Vec<i64>,
    mask: usize,
    /// `64 - log2(slots)`: a key's slot is the top bits of its Fibonacci product ([`fib_slot`]).
    shift: u32,
    len: usize,
    has_zero_key: bool,
}

impl LogosI64Set {
    const EMPTY: i64 = 0;

    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            mask: 0,
            shift: 64,
            len: 0,
            has_zero_key: false,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut s = Self::new();
        if cap > 0 {
            let slots = int_table_slots(cap);
            // `Self::EMPTY == 0` → `alloc_zeroed` (calloc): lazily-zeroed pages,
            // no eager memset/fault of the whole table.
            s.keys = vec![Self::EMPTY; slots];
            s.mask = slots - 1;
            s.shift = 64 - slots.trailing_zeros();
        }
        s
    }

    #[inline]
    fn table_len(&self) -> usize {
        self.len - self.has_zero_key as usize
    }

    /// The probe table is a power-of-two `Vec` whose length is exactly
    /// `mask + 1` (maintained by `with_capacity`/`grow`); every probe index is
    /// `… & mask`, so once `mask != 0` every `self.keys[i]` is in bounds. Telling
    /// LLVM the invariant lets it drop the per-probe bounds check (matching C's
    /// raw indexed loads). Caller must have established `mask != 0`.
    #[inline(always)]
    fn assume_table_invariant(&self) {
        unsafe {
            std::hint::assert_unchecked(self.mask + 1 == self.keys.len());
        }
    }

    #[inline]
    fn slot(&self, key: i64) -> usize {
        fib_slot(key as u64, self.shift)
    }

    /// Insert a key. The `_value` parameter mirrors [`LogosI64Map::insert`]'s
    /// call shape so codegen needs no special-casing at the insert site — by the
    /// set-usage proof the value is never read.
    #[inline]
    pub fn insert(&mut self, key: i64, _value: i64) {
        if key == Self::EMPTY {
            if !self.has_zero_key {
                self.has_zero_key = true;
                self.len += 1;
            }
            return;
        }
        if self.mask == 0 || int_table_full(self.table_len(), self.mask + 1) {
            self.grow();
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == Self::EMPTY {
                self.keys[i] = key;
                self.len += 1;
                return;
            }
            if k == key {
                return;
            }
            i = (i + 1) & self.mask;
        }
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let new_slots = if self.mask == 0 {
            8
        } else {
            (self.mask + 1) * 2
        };
        let old_keys = std::mem::replace(&mut self.keys, vec![Self::EMPTY; new_slots]);
        self.mask = new_slots - 1;
        self.shift = 64 - new_slots.trailing_zeros();
        self.assume_table_invariant();
        for &k in old_keys.iter() {
            if k != Self::EMPTY {
                let mut i = self.slot(k);
                while self.keys[i] != Self::EMPTY {
                    i = (i + 1) & self.mask;
                }
                self.keys[i] = k;
            }
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        let key = *key;
        if key == Self::EMPTY {
            return self.has_zero_key;
        }
        if self.mask == 0 {
            return false;
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == key {
                return true;
            }
            if k == Self::EMPTY {
                return false;
            }
            i = (i + 1) & self.mask;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosI64Set {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosI64Set {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.contains_key(key)
    }
}

/// i32-narrowed open-addressing `i64 → i64` map — the same probe-table shape as
/// [`LogosI64Map`] but with 8-byte `(i32, i32)` slots, emitted when the compiler
/// PROVES every key AND value stays in `i32` range. Halving the slot width halves
/// the table's memory traffic — the dominant cost of this random-access workload —
/// for maps the dense gate cannot capture (no proven contiguous key window). The
/// call surface is `i64` (keys/values widen at the boundary; the proof makes the
/// narrowing cast lossless), so codegen emits it identically to `LogosI64Map`.
#[derive(Debug, Clone)]
pub struct LogosI32Map {
    entries: Vec<[i32; 2]>,
    mask: usize,
    /// `64 - log2(slots)`: a key's slot is the top bits of its Fibonacci product ([`fib_slot`]).
    shift: u32,
    len: usize,
    has_zero_key: bool,
    zero_key_val: i32,
}

impl LogosI32Map {
    const EMPTY: i32 = 0;

    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            mask: 0,
            shift: 64,
            len: 0,
            has_zero_key: false,
            zero_key_val: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut m = Self::new();
        if cap > 0 {
            let slots = int_table_slots(cap);
            // `[0, 0]` → `alloc_zeroed` (calloc): lazily zeroed, no eager memset.
            m.entries = vec![[Self::EMPTY, 0]; slots];
            m.mask = slots - 1;
            m.shift = 64 - slots.trailing_zeros();
        }
        m
    }

    #[inline]
    fn table_len(&self) -> usize {
        self.len - self.has_zero_key as usize
    }

    /// See [`LogosI64Set::assume_table_invariant`]: `entries.len() == mask + 1`
    /// once `mask != 0`, so masked probe indices need no bounds check.
    #[inline(always)]
    fn assume_table_invariant(&self) {
        unsafe {
            std::hint::assert_unchecked(self.mask + 1 == self.entries.len());
        }
    }

    #[inline]
    fn slot(&self, key: i32) -> usize {
        fib_slot(key as u32 as u64, self.shift)
    }

    #[inline]
    pub fn insert(&mut self, key: i64, value: i64) {
        let key = key as i32;
        let value = value as i32;
        if key == Self::EMPTY {
            if !self.has_zero_key {
                self.has_zero_key = true;
                self.len += 1;
            }
            self.zero_key_val = value;
            return;
        }
        if self.mask == 0 || int_table_full(self.table_len(), self.mask + 1) {
            self.grow();
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.entries[i][0];
            if k == Self::EMPTY {
                self.entries[i] = [key, value];
                self.len += 1;
                return;
            }
            if k == key {
                self.entries[i][1] = value;
                return;
            }
            i = (i + 1) & self.mask;
        }
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let new_slots = if self.mask == 0 {
            8
        } else {
            (self.mask + 1) * 2
        };
        let old = std::mem::replace(&mut self.entries, vec![[Self::EMPTY, 0]; new_slots]);
        self.mask = new_slots - 1;
        self.shift = 64 - new_slots.trailing_zeros();
        self.assume_table_invariant();
        for &[k, v] in old.iter() {
            if k != Self::EMPTY {
                let mut i = self.slot(k);
                while self.entries[i][0] != Self::EMPTY {
                    i = (i + 1) & self.mask;
                }
                self.entries[i] = [k, v];
            }
        }
    }

    #[inline]
    pub fn get(&self, key: &i64) -> Option<i64> {
        let key = *key as i32;
        if key == Self::EMPTY {
            return if self.has_zero_key {
                Some(self.zero_key_val as i64)
            } else {
                None
            };
        }
        if self.mask == 0 {
            return None;
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let [k, v] = self.entries[i];
            if k == key {
                return Some(v as i64);
            }
            if k == Self::EMPTY {
                return None;
            }
            i = (i + 1) & self.mask;
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosI32Map {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosI32Map {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.get(key).is_some()
    }
}

/// i32-narrowed open-addressing set — the keys-only sibling of [`LogosI32Map`]
/// (the i32 analogue of [`LogosI64Set`]). One flat `Vec<i32>`: 4 bytes per slot,
/// a quarter of `LogosI64Map`'s footprint. Emitted for a set-usage `Int → Int`
/// map whose keys are all proven to fit `i32`. The empty-slot sentinel is `0`
/// (the real key `0` tracked separately) so `vec![0; slots]` allocates via
/// `alloc_zeroed` — lazily-zeroed, no eager memset of the table.
#[derive(Debug, Clone)]
pub struct LogosI32Set {
    keys: Vec<i32>,
    mask: usize,
    /// `64 - log2(slots)`: a key's slot is the top bits of its Fibonacci product ([`fib_slot`]).
    shift: u32,
    len: usize,
    has_zero_key: bool,
}

impl LogosI32Set {
    const EMPTY: i32 = 0;

    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            mask: 0,
            shift: 64,
            len: 0,
            has_zero_key: false,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut s = Self::new();
        if cap > 0 {
            let slots = int_table_slots(cap);
            s.keys = vec![Self::EMPTY; slots];
            s.mask = slots - 1;
            s.shift = 64 - slots.trailing_zeros();
        }
        s
    }

    #[inline]
    fn table_len(&self) -> usize {
        self.len - self.has_zero_key as usize
    }

    /// See [`LogosI64Set::assume_table_invariant`]: `keys.len() == mask + 1` once
    /// `mask != 0`, so masked probe indices need no bounds check.
    #[inline(always)]
    fn assume_table_invariant(&self) {
        unsafe {
            std::hint::assert_unchecked(self.mask + 1 == self.keys.len());
        }
    }

    #[inline]
    fn slot(&self, key: i32) -> usize {
        fib_slot(key as u32 as u64, self.shift)
    }

    #[inline]
    pub fn insert(&mut self, key: i64, _value: i64) {
        let key = key as i32;
        if key == Self::EMPTY {
            if !self.has_zero_key {
                self.has_zero_key = true;
                self.len += 1;
            }
            return;
        }
        if self.mask == 0 || int_table_full(self.table_len(), self.mask + 1) {
            self.grow();
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == Self::EMPTY {
                self.keys[i] = key;
                self.len += 1;
                return;
            }
            if k == key {
                return;
            }
            i = (i + 1) & self.mask;
        }
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let new_slots = if self.mask == 0 {
            8
        } else {
            (self.mask + 1) * 2
        };
        let old_keys = std::mem::replace(&mut self.keys, vec![Self::EMPTY; new_slots]);
        self.mask = new_slots - 1;
        self.shift = 64 - new_slots.trailing_zeros();
        self.assume_table_invariant();
        for &k in old_keys.iter() {
            if k != Self::EMPTY {
                let mut i = self.slot(k);
                while self.keys[i] != Self::EMPTY {
                    i = (i + 1) & self.mask;
                }
                self.keys[i] = k;
            }
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        let key = *key as i32;
        if key == Self::EMPTY {
            return self.has_zero_key;
        }
        if self.mask == 0 {
            return false;
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == key {
                return true;
            }
            if k == Self::EMPTY {
                return false;
            }
            i = (i + 1) & self.mask;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosI32Set {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosI32Set {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.contains_key(key)
    }
}

/// Direct-addressed `i64 → i64` map — the densest representation of a
/// `Map of Int to Int`, emitted when the compiler PROVES every key ever inserted
/// or queried lands in a statically bounded window `[lo, lo + slots)` whose width
/// is ≤ the map's own `with capacity` hint. Keys index a flat value array
/// (`data[key - lo]`); a parallel presence bitset distinguishes a stored value
/// from an absent slot, so `get` returns `None` for an in-range key that was
/// never inserted. No hashing, no probing, no sparse table — both build and scan
/// phases are sequential array accesses, which the backend vectorizes and the
/// prefetcher loves. This is perfect hashing for dense integer keys: it removes
/// the random-access hash table that costs `LogosI64Map` its cache locality.
#[derive(Debug, Clone)]
pub struct LogosDenseI64Map {
    /// Value slots, indexed by `key - lo`. Zero-initialized; a zero is only a
    /// real value when its presence bit is set.
    data: Vec<i64>,
    /// One bit per slot: set ⇔ the slot holds an inserted value.
    present: Vec<u64>,
    /// The window's lower bound; `data[key - lo]` rebases the key space (so a
    /// 1-based or negative key domain maps onto `[0, slots)`).
    lo: i64,
    len: usize,
}

impl LogosDenseI64Map {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            present: Vec::new(),
            lo: 0,
            len: 0,
        }
    }

    /// A window `[0, cap)` — the offset-free form. Mirrors the `LogosI64Map`
    /// constructor name so a non-offset dense map reuses the same emission.
    pub fn with_capacity(cap: usize) -> Self {
        Self::with_bounds(0, cap)
    }

    /// A window `[lo, lo + slots)`. `slots` is sized to the proven capacity hint;
    /// the soundness gate guarantees every key falls inside, so `key - lo` is a
    /// valid index for every insert and get the program performs.
    pub fn with_bounds(lo: i64, slots: usize) -> Self {
        Self {
            data: vec![0; slots],
            present: vec![0; slots.div_ceil(64)],
            lo,
            len: 0,
        }
    }

    #[inline]
    pub fn insert(&mut self, key: i64, value: i64) {
        let idx = (key - self.lo) as usize;
        debug_assert!(
            key >= self.lo && idx < self.data.len(),
            "dense map key {key} outside proven window [{}, {})",
            self.lo,
            self.lo + self.data.len() as i64
        );
        let bit = 1u64 << (idx & 63);
        let word = &mut self.present[idx >> 6];
        if *word & bit == 0 {
            *word |= bit;
            self.len += 1;
        }
        self.data[idx] = value;
    }

    #[inline]
    pub fn get(&self, key: &i64) -> Option<i64> {
        let idx = (*key - self.lo) as usize;
        debug_assert!(
            *key >= self.lo && idx < self.data.len(),
            "dense map key {key} outside proven window [{}, {})",
            self.lo,
            self.lo + self.data.len() as i64
        );
        if self.present[idx >> 6] & (1u64 << (idx & 63)) != 0 {
            Some(self.data[idx])
        } else {
            None
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        let idx = (*key - self.lo) as usize;
        debug_assert!(*key >= self.lo && idx < self.data.len());
        self.present[idx >> 6] & (1u64 << (idx & 63)) != 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosDenseI64Map {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosDenseI64Map {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.contains_key(key)
    }
}

/// The presence-elided sibling of [`LogosDenseI64Map`], emitted ONLY when the
/// compiler additionally proves the get/contains key domain is a subset of a
/// CONTIGUOUSLY FULLY-COVERED insert range — i.e. every key the program reads was
/// definitely written first. With that proof the presence bit is invariably set,
/// so it is dropped: `get` is a bare `Some(data[key - lo])` load, byte-identical
/// to a C array read, and no presence bitset is allocated. The gate forbids this
/// type for any map with a `contains` use (there is no way to answer membership
/// without presence), so `contains_key`/`LogosContains` are intentionally absent
/// — a generated `contains` on this type would fail to compile, surfacing a gate
/// bug loudly rather than silently miscompiling.
#[derive(Debug, Clone)]
pub struct LogosDenseI64MapNoPresence {
    data: Vec<i64>,
    lo: i64,
    /// Insert count. Exact under the proven regime (each covered key written
    /// once); never emitted (a `length of m` use disqualifies the map upstream).
    len: usize,
}

impl LogosDenseI64MapNoPresence {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            lo: 0,
            len: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self::with_bounds(0, cap)
    }

    pub fn with_bounds(lo: i64, slots: usize) -> Self {
        Self {
            data: vec![0; slots],
            lo,
            len: 0,
        }
    }

    #[inline]
    pub fn insert(&mut self, key: i64, value: i64) {
        let idx = (key - self.lo) as usize;
        debug_assert!(
            key >= self.lo && idx < self.data.len(),
            "dense map key {key} outside proven window [{}, {})",
            self.lo,
            self.lo + self.data.len() as i64
        );
        self.data[idx] = value;
        self.len += 1;
    }

    #[inline]
    pub fn get(&self, key: &i64) -> Option<i64> {
        let idx = (*key - self.lo) as usize;
        debug_assert!(
            *key >= self.lo && idx < self.data.len(),
            "dense map key {key} outside proven window [{}, {})",
            self.lo,
            self.lo + self.data.len() as i64
        );
        Some(self.data[idx])
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosDenseI64MapNoPresence {
    fn default() -> Self {
        Self::new()
    }
}

/// Direct-addressed `i64` SET — the keys-only, value-free sibling of
/// [`LogosDenseI64Map`] (the dense analogue of [`LogosI64Set`]). Membership lives
/// in a presence bitset over the proven window `[lo, lo + slots)`; `insert` sets
/// a bit, `contains` tests one. No value array → 1 bit per key, the smallest
/// footprint of any map/set representation.
#[derive(Debug, Clone)]
pub struct LogosDenseI64Set {
    present: Vec<u64>,
    lo: i64,
    len: usize,
}

impl LogosDenseI64Set {
    pub fn new() -> Self {
        Self {
            present: Vec::new(),
            lo: 0,
            len: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self::with_bounds(0, cap)
    }

    pub fn with_bounds(lo: i64, slots: usize) -> Self {
        Self {
            present: vec![0; slots.div_ceil(64)],
            lo,
            len: 0,
        }
    }

    /// Insert a key. The `_value` mirrors [`LogosDenseI64Map::insert`]'s call
    /// shape so codegen needs no special-casing at the insert site.
    #[inline]
    pub fn insert(&mut self, key: i64, _value: i64) {
        let idx = (key - self.lo) as usize;
        debug_assert!(
            key >= self.lo && idx < self.present.len() * 64,
            "dense set key {key} outside proven window starting at {}",
            self.lo
        );
        let bit = 1u64 << (idx & 63);
        let word = &mut self.present[idx >> 6];
        if *word & bit == 0 {
            *word |= bit;
            self.len += 1;
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        let idx = (*key - self.lo) as usize;
        debug_assert!(*key >= self.lo && idx < self.present.len() * 64);
        self.present[idx >> 6] & (1u64 << (idx & 63)) != 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosDenseI64Set {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosDenseI64Set {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.contains_key(key)
    }
}

/// Precomputed unsigned division by a **loop-invariant runtime divisor** — the
/// magic-multiply that replaces a hardware `div`/`idiv` once the divisor is
/// pinned. Codegen emits `LogosDivU64::new(n)` in a loop's preheader (computed
/// ONCE) and rewrites each in-loop `x % n` / `x / n` to `.rem(x)` / `.div(x)`,
/// turning a ~20–40-cycle division into a multiply-high plus a shift. gcc and
/// rustc both leave a runtime-invariant divisor as a real `div` (neither
/// synthesizes this), so it is a strict win over the C baseline on division-hot
/// loops (graph_bfs's `% n` adjacency build).
///
/// The construction is the standard Granlund–Montgomery / libdivide unsigned
/// algorithm, exact for every divisor in `1..=u64::MAX` (power-of-two fast path,
/// the 64-bit-magic path, and the 65-bit "add marker" path). Codegen only emits
/// it where the dividend is proven non-negative and `n > 0` (the existing
/// positivity guard), so the `i64`→`u64` reinterpretation is value-preserving.
#[derive(Debug, Clone, Copy)]
pub struct LogosDivU64 {
    magic: u64,
    /// Low 6 bits: shift amount. `ADD_MARKER` (0x40): the 65-bit-magic path.
    /// `SHIFT_PATH` (0x80): the divisor is a power of two (pure shift).
    more: u8,
    d: u64,
}

impl LogosDivU64 {
    const SHIFT_MASK: u8 = 0x3F;
    const ADD_MARKER: u8 = 0x40;
    const SHIFT_PATH: u8 = 0x80;

    /// Build the magic numbers for divisor `d` (must be non-zero). Runs once per
    /// loop in the preheader, so its cost (a single 128/64 division) amortizes
    /// over the whole loop.
    #[inline]
    pub fn new(d: u64) -> Self {
        debug_assert!(d != 0, "LogosDivU64: divisor must be non-zero");
        if d & (d - 1) == 0 {
            // Power of two (including d == 1, shift 0): the division is a shift.
            return Self {
                magic: 0,
                more: (d.trailing_zeros() as u8) | Self::SHIFT_PATH,
                d,
            };
        }
        let floor_log_2_d = 63 - d.leading_zeros();
        // proposed_m = floor(2^(64 + floor_log_2_d) / d); rem the remainder.
        let numer = 1u128 << (64 + floor_log_2_d);
        let proposed_m = (numer / d as u128) as u64;
        let rem = (numer % d as u128) as u64;
        let e = d - rem;
        let (magic, more) = if e < (1u64 << floor_log_2_d) {
            // The 64-bit magic suffices.
            (proposed_m + 1, floor_log_2_d as u8)
        } else {
            // Need the 65th bit — fold it into an extra add at division time.
            let twice_rem = rem.wrapping_mul(2);
            let bump = (twice_rem >= d || twice_rem < rem) as u64;
            (
                proposed_m.wrapping_mul(2).wrapping_add(bump) + 1,
                (floor_log_2_d as u8) | Self::ADD_MARKER,
            )
        };
        Self { magic, more, d }
    }

    #[inline(always)]
    pub fn div(&self, numer: u64) -> u64 {
        if self.more & Self::SHIFT_PATH != 0 {
            return numer >> (self.more & Self::SHIFT_MASK);
        }
        let q = (((self.magic as u128) * (numer as u128)) >> 64) as u64;
        if self.more & Self::ADD_MARKER != 0 {
            let t = ((numer - q) >> 1).wrapping_add(q);
            t >> (self.more & Self::SHIFT_MASK)
        } else {
            q >> self.more
        }
    }

    #[inline(always)]
    pub fn rem(&self, numer: u64) -> u64 {
        numer - self.div(numer).wrapping_mul(self.d)
    }

    /// The raw `(magic, more)` pair, for backends (the VM / JIT interpreter
    /// tiers) that bake the precomputed constants into a single fused op instead
    /// of holding a `LogosDivU64` struct. The encoding of `more` is exactly the
    /// one `div`/`rem` consume (low 6 bits = shift; `0x40` = the 65-bit
    /// add-marker path; `0x80` = the pure-shift power-of-two path).
    #[inline]
    pub fn parts(&self) -> (u64, u8) {
        (self.magic, self.more)
    }
}

/// Ordered sequences with reference semantics.
pub type Seq<T> = LogosSeq<T>;

/// Key-value mappings with reference semantics.
pub type Map<K, V> = LogosMap<K, V>;

/// Unordered collections of unique elements with FxHash.
/// The insertion-ordered set behind every LOGOS `Set`: display and iteration
/// follow first-insertion order, identical to the tree-walker's Vec-backed
/// sets and the direct-WASM linear sets. Fx hashing for speed. NOTE: removal
/// must go through `shift_remove` (order-preserving), never `swap_remove`.
pub type FxIndexSet<T> = indexmap::IndexSet<T, std::hash::BuildHasherDefault<rustc_hash::FxHasher>>;
pub type Set<T> = FxIndexSet<T>;

/// Unified containment testing for all collection types.
///
/// This trait provides a consistent `logos_contains` method across Logos's
/// collection types, abstracting over the different containment semantics
/// of vectors (by value), sets (by membership), maps (by key), and
/// strings (by substring or character).
///
/// # Implementations
///
/// - [`Vec<T>`]: Tests if the vector contains an element equal to the value
/// - `HashSet<T>`: Tests if the element is a member of the set
/// - `HashMap<K, V>`: Tests if a key exists in the map
/// - [`String`]: Tests for substring (`&str`) or character (`char`) presence
/// - `ORSet<T, B>`: Tests if the element is in the CRDT set
///
/// # Examples
///
/// ```
/// use logicaffeine_data::LogosContains;
///
/// // Vector: contains by value equality
/// let v = vec![1, 2, 3];
/// assert!(v.logos_contains(&2));
/// assert!(!v.logos_contains(&5));
///
/// // String: contains by substring
/// let s = String::from("hello world");
/// assert!(s.logos_contains(&"world"));
///
/// // String: contains by character
/// assert!(s.logos_contains(&'o'));
/// ```
pub trait LogosContains<T> {
    /// Check if this collection contains the given value.
    fn logos_contains(&self, value: &T) -> bool;
}

impl<T: PartialEq> LogosContains<T> for Vec<T> {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<T: PartialEq> LogosContains<T> for [T] {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<T: Eq + Hash, S: std::hash::BuildHasher> LogosContains<T> for indexmap::IndexSet<T, S> {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<T: Eq + Hash> LogosContains<T> for rustc_hash::FxHashSet<T> {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<K: Eq + Hash, V> LogosContains<K> for rustc_hash::FxHashMap<K, V> {
    #[inline(always)]
    fn logos_contains(&self, key: &K) -> bool {
        self.contains_key(key)
    }
}

impl LogosContains<&str> for String {
    #[inline(always)]
    fn logos_contains(&self, value: &&str) -> bool {
        self.contains(*value)
    }
}

impl LogosContains<String> for String {
    #[inline(always)]
    fn logos_contains(&self, value: &String) -> bool {
        self.contains(value.as_str())
    }
}

impl LogosContains<char> for String {
    #[inline(always)]
    fn logos_contains(&self, value: &char) -> bool {
        self.contains(*value)
    }
}

impl<T: Eq + Hash + Clone, B: crate::crdt::SetBias> LogosContains<T> for crate::crdt::ORSet<T, B> {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

/// Dynamic value type for heterogeneous collections.
///
/// `Value` enables tuples and other heterogeneous data structures in Logos.
/// It supports basic arithmetic between compatible types and provides
/// runtime type coercion where sensible.
///
/// # Variants
///
/// - `Int(i64)` - Integer values
/// - `Float(f64)` - Floating-point values
/// - `Bool(bool)` - Boolean values
/// - `Text(String)` - String values
/// - `Char(char)` - Single character values
/// - `Nothing` - Unit/null value
///
/// # Arithmetic
///
/// Arithmetic operations are supported between numeric types:
/// - `Int op Int` → `Int`
/// - `Float op Float` → `Float`
/// - `Int op Float` or `Float op Int` → `Float` (promotion)
/// - `Text + Text` → `Text` (concatenation)
///
/// # Panics
///
/// Arithmetic on incompatible variants panics at runtime.
///
/// # Examples
///
/// ```
/// use logicaffeine_data::Value;
///
/// let a = Value::Int(10);
/// let b = Value::Int(3);
/// assert_eq!(a + b, Value::Int(13));
///
/// let x = Value::Float(2.5);
/// let y = Value::Int(2);
/// assert_eq!(x * y, Value::Float(5.0));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Integer values.
    Int(i64),
    /// Floating-point values.
    Float(f64),
    /// Boolean values.
    Bool(bool),
    /// String values.
    Text(String),
    /// Single character values.
    Char(char),
    /// A boxed homogeneous sequence — the carrier that lets a `LogosTuple` hold a COLLECTION element
    /// (`(Seq of Text, Seq of Int)`), not just a scalar. The elements are themselves boxed `Value`s, so
    /// a `Seq of Seq of _` nests naturally. Constructed by the `From<LogosSeq<T>>` impls and read back by
    /// the `logos_seq_*` unboxers, exactly as the scalar variants pair with `From`/`logos_*`.
    Seq(LogosSeq<Value>),
    /// A value of a type the program declares (a record, a variant), boxed so it rides a tuple element
    /// beside scalars and lists. Built by the `From<T: Boxable>` impl, read back by
    /// [`Value::logos_boxed`], rendered by the type's own `Show`.
    Boxed(LogosBoxed),
    /// Unit/null value.
    Nothing,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Text(s) => write!(f, "{}", s),
            Value::Char(c) => write!(f, "{}", c),
            Value::Seq(xs) => write!(f, "{}", xs),
            Value::Boxed(b) => write!(f, "{}", b),
            Value::Nothing => write!(f, "nothing"),
        }
    }
}

/// A type a compiled program declares that can ride a tuple element as a [`Value::Boxed`]. The
/// program implements it for each record and variant it declares; `render` is the type's own `Show`
/// (this crate has no IO, so the rendering is supplied by the implementor).
pub trait Boxable: std::any::Any + Clone + PartialEq + std::fmt::Debug {
    fn render(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}

/// The object-safe face of a [`Boxable`]: what a [`LogosBoxed`] needs from the value it holds.
trait BoxedValue: std::any::Any {
    fn render(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
    fn equals(&self, other: &dyn BoxedValue) -> bool;
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: Boxable> BoxedValue for T {
    fn render(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Boxable::render(self, f)
    }
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
    fn equals(&self, other: &dyn BoxedValue) -> bool {
        other
            .as_any()
            .downcast_ref::<T>()
            .is_some_and(|other| self == other)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`Boxable`] value behind a shared, immutable pointer. Reading it back clones the value out, so
/// sharing the allocation between clones of a tuple never lets one copy observe another's writes.
/// Two boxes are equal when they hold the same type and equal values.
#[derive(Clone)]
pub struct LogosBoxed(Rc<dyn BoxedValue>);

impl LogosBoxed {
    #[inline]
    pub fn new<T: Boxable>(value: T) -> Self {
        LogosBoxed(Rc::new(value))
    }

    /// The held value as a `T`. Holding any other type is a program-type error, and panics like the
    /// scalar unboxers.
    #[inline]
    pub fn get<T: Boxable>(&self) -> T {
        match self.0.as_any().downcast_ref::<T>() {
            Some(value) => value.clone(),
            None => panic!(
                "expected a {} tuple/record element, found {}",
                std::any::type_name::<T>(),
                self
            ),
        }
    }
}

impl PartialEq for LogosBoxed {
    fn eq(&self, other: &Self) -> bool {
        self.0.equals(&*other.0)
    }
}

impl std::fmt::Debug for LogosBoxed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.debug(f)
    }
}

impl std::fmt::Display for LogosBoxed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.render(f)
    }
}

impl<T: Boxable> From<T> for Value {
    #[inline]
    fn from(value: T) -> Self {
        Value::Boxed(LogosBoxed::new(value))
    }
}

/// A LOGOS tuple `(a, b, …)` — a positional, heterogeneous value. A newtype over `Vec<Value>` that
/// `Deref`s to it, so every existing indexing / destructuring / iteration / return path keeps working
/// unchanged; it exists ONLY to carry a distinct display: a tuple renders with PARENS `(a, b)` and — the
/// whole point — stays distinguishable from a `Seq` even when nested (`[(1, 2), (3, 4)]`), which a bare
/// `Vec<Value>` (shown as `[…]`, identical to a Seq) could not.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LogosTuple(pub Vec<Value>);

impl std::ops::Deref for LogosTuple {
    type Target = Vec<Value>;
    fn deref(&self) -> &Vec<Value> {
        &self.0
    }
}

impl std::ops::DerefMut for LogosTuple {
    fn deref_mut(&mut self) -> &mut Vec<Value> {
        &mut self.0
    }
}

impl From<Vec<Value>> for LogosTuple {
    fn from(v: Vec<Value>) -> Self {
        LogosTuple(v)
    }
}

impl std::fmt::Display for LogosTuple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(")?;
        for (i, v) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", v)?;
        }
        write!(f, ")")
    }
}

// Conversion traits for Value
impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Int(n)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Float(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}

impl From<char> for Value {
    fn from(c: char) -> Self {
        Value::Char(c)
    }
}

/// BOX a homogeneous sequence into a `Value` so it can ride as a tuple element. Each element is boxed
/// via its own `Into<Value>` (so `LogosSeq<i64>` → `Value::Seq` of `Value::Int`, and a `LogosSeq<LogosSeq<_>>`
/// nests by boxing each inner seq the same way; a `LogosSeq<Value>` uses the reflexive `Value: Into<Value>`).
/// Pairs with the `logos_seq_*` unboxers.
impl<T: Into<Value> + Clone> From<LogosSeq<T>> for Value {
    fn from(xs: LogosSeq<T>) -> Self {
        Value::Seq(LogosSeq::from_vec(
            xs.0.borrow().iter().cloned().map(Into::into).collect(),
        ))
    }
}

/// Tuple type: Vec of heterogeneous Values (uses LogosIndex from indexing module)
pub type Tuple = Vec<Value>;

impl Value {
    /// UNBOX a heterogeneous-tuple / named-record element (a boxed `Value` read out of the positional
    /// `Vec<Value>`) back to its concrete scalar so it can be USED in a typed position — `upper(item 2 of
    /// t)` where element 2 is a `Text`, or `r's ratio * 2.0` where `ratio` is a `Real`. Extraction alone
    /// yields a `Value` (fine for `Show`, which takes any `Display`); these turn it back into the exact
    /// scalar the AOT-Rust code's type context demands. A wrong-variant unbox is a genuine program-type
    /// error, so it panics loudly (mirrors the interpreter/VM, which would surface the same mismatch).
    #[inline]
    pub fn logos_i64(self) -> i64 {
        match self {
            Value::Int(n) => n,
            other => panic!("expected an Int tuple/record element, found {other}"),
        }
    }
    #[inline]
    pub fn logos_f64(self) -> f64 {
        match self {
            Value::Float(x) => x,
            Value::Int(n) => n as f64,
            other => panic!("expected a Real tuple/record element, found {other}"),
        }
    }
    #[inline]
    pub fn logos_bool(self) -> bool {
        match self {
            Value::Bool(b) => b,
            other => panic!("expected a Bool tuple/record element, found {other}"),
        }
    }
    #[inline]
    pub fn logos_string(self) -> String {
        match self {
            Value::Text(s) => s,
            Value::Char(c) => c.to_string(),
            other => panic!("expected a Text tuple/record element, found {other}"),
        }
    }
    #[inline]
    pub fn logos_char(self) -> char {
        match self {
            Value::Char(c) => c,
            other => panic!("expected a Char tuple/record element, found {other}"),
        }
    }
    /// UNBOX a [`Value::Boxed`] element back to the record or variant it holds.
    #[inline]
    pub fn logos_boxed<T: Boxable>(self) -> T {
        match self {
            Value::Boxed(b) => b.get(),
            other => panic!(
                "expected a {} tuple/record element, found {other}",
                std::any::type_name::<T>()
            ),
        }
    }

    /// UNBOX the [`Value::Seq`] carrier back to a typed `LogosSeq<T>`, unboxing each element with `f`
    /// (`Value::logos_i64` for a `Seq of Int`, `Value::logos_seq_string` for a `Seq of Seq of Text`, …).
    /// The scalar-per-type wrappers below name the common element unboxers; nesting composes them. A
    /// non-`Seq` value is a genuine program-type error, so it panics loudly like the scalar unboxers.
    #[inline]
    pub fn logos_seq<T>(self, f: impl Fn(Value) -> T) -> LogosSeq<T> {
        match self {
            Value::Seq(xs) => LogosSeq::from_vec(xs.0.borrow().iter().cloned().map(f).collect()),
            other => panic!("expected a Seq tuple/record element, found {other}"),
        }
    }
    #[inline]
    pub fn logos_seq_i64(self) -> LogosSeq<i64> {
        self.logos_seq(Value::logos_i64)
    }
    #[inline]
    pub fn logos_seq_f64(self) -> LogosSeq<f64> {
        self.logos_seq(Value::logos_f64)
    }
    #[inline]
    pub fn logos_seq_bool(self) -> LogosSeq<bool> {
        self.logos_seq(Value::logos_bool)
    }
    #[inline]
    pub fn logos_seq_string(self) -> LogosSeq<String> {
        self.logos_seq(Value::logos_string)
    }
    #[inline]
    pub fn logos_seq_seq_i64(self) -> LogosSeq<LogosSeq<i64>> {
        self.logos_seq(Value::logos_seq_i64)
    }
    #[inline]
    pub fn logos_seq_seq_f64(self) -> LogosSeq<LogosSeq<f64>> {
        self.logos_seq(Value::logos_seq_f64)
    }
    #[inline]
    pub fn logos_seq_seq_bool(self) -> LogosSeq<LogosSeq<bool>> {
        self.logos_seq(Value::logos_seq_bool)
    }
    #[inline]
    pub fn logos_seq_seq_string(self) -> LogosSeq<LogosSeq<String>> {
        self.logos_seq(Value::logos_seq_string)
    }
}

// NOTE: Showable impl for Value is in logicaffeine_system (io module)
// This crate (logicaffeine_data) has NO IO dependencies.

// Arithmetic operations for Value
impl std::ops::Add for Value {
    type Output = Value;

    #[inline]
    fn add(self, other: Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a + b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
            (Value::Int(a), Value::Float(b)) => Value::Float(a as f64 + b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a + b as f64),
            (Value::Text(a), Value::Text(b)) => Value::Text(format!("{}{}", a, b)),
            _ => panic!("Cannot add these value types"),
        }
    }
}

impl std::ops::Sub for Value {
    type Output = Value;

    #[inline]
    fn sub(self, other: Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a - b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a - b),
            (Value::Int(a), Value::Float(b)) => Value::Float(a as f64 - b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a - b as f64),
            _ => panic!("Cannot subtract these value types"),
        }
    }
}

impl std::ops::Mul for Value {
    type Output = Value;

    #[inline]
    fn mul(self, other: Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a * b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a * b),
            (Value::Int(a), Value::Float(b)) => Value::Float(a as f64 * b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a * b as f64),
            _ => panic!("Cannot multiply these value types"),
        }
    }
}

impl std::ops::Div for Value {
    type Output = Value;

    #[inline]
    fn div(self, other: Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a / b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a / b),
            (Value::Int(a), Value::Float(b)) => Value::Float(a as f64 / b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a / b as f64),
            _ => panic!("Cannot divide these value types"),
        }
    }
}

/// Exact rational number for compiled LOGOS programs — the AOT counterpart of the
/// interpreter's `RuntimeValue::Rational`.
///
/// Wraps the always-reduced [`logicaffeine_base::Rational`] (a `BigInt` numerator over a
/// positive `BigInt` denominator) so a `Let x: Rational be 7 / 2` compiles to an exact
/// `7/2` instead of flooring to `3`. The type-directed `resolve_divisions` pass only ever
/// produces these in a `Rational`-typed context, so the integer floor default is untouched.
/// `Display` reduces a whole value to a bare integer (`6 / 2 → "3"`), matching the interpreter.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogosRational(pub logicaffeine_base::Rational);

impl LogosRational {
    /// A whole integer as an exact rational (`5 → 5/1`, shown `5`).
    #[inline]
    pub fn from_i64(n: i64) -> Self {
        LogosRational(logicaffeine_base::Rational::from_i64(n))
    }

    /// The exact quotient `n / d`. Panics on a zero denominator, mirroring integer `/ 0`.
    #[inline]
    pub fn from_ratio(n: i64, d: i64) -> Self {
        LogosRational(
            logicaffeine_base::Rational::from_ratio_i64(n, d)
                .expect(crate::failure::DIVISION_BY_ZERO),
        )
    }

    #[inline]
    pub fn add(&self, other: &LogosRational) -> LogosRational {
        LogosRational(self.0.add(&other.0))
    }

    #[inline]
    pub fn sub(&self, other: &LogosRational) -> LogosRational {
        LogosRational(self.0.sub(&other.0))
    }

    #[inline]
    pub fn mul(&self, other: &LogosRational) -> LogosRational {
        LogosRational(self.0.mul(&other.0))
    }

    /// Exact division. Panics on a zero divisor, mirroring integer `/ 0`.
    #[inline]
    pub fn div_exact(&self, other: &LogosRational) -> LogosRational {
        LogosRational(
            self.0
                .div(&other.0)
                .expect(crate::failure::DIVISION_BY_ZERO),
        )
    }

    /// The exact absolute value (a rational stays a rational: `|-7/2| = 7/2`).
    #[inline]
    pub fn abs(&self) -> LogosRational {
        LogosRational(self.0.abs())
    }

    /// The greatest integer ≤ this rational (toward −∞), computed EXACTLY on the
    /// BigInt numerator/denominator — never through a lossy `f64`.
    #[inline]
    pub fn floor(&self) -> i64 {
        self.0
            .floor()
            .to_i64()
            .expect("LOGOS runtime error: floor exceeds i64")
    }

    /// The least integer ≥ this rational (toward +∞), computed exactly.
    #[inline]
    pub fn ceil(&self) -> i64 {
        self.0
            .ceil()
            .to_i64()
            .expect("LOGOS runtime error: ceiling exceeds i64")
    }

    /// The nearest integer, ties away from zero (matching `f64::round`), computed exactly.
    #[inline]
    pub fn round(&self) -> i64 {
        self.0
            .round()
            .to_i64()
            .expect("LOGOS runtime error: round exceeds i64")
    }

    /// The exact number the language holds this as: the integer it equals when it is whole, so every
    /// operator below takes the rule the interpreter takes for the value (a whole value the integer's).
    fn exact(&self) -> crate::LogosValue {
        crate::LogosValue::from_rational(self.0.clone())
    }

    /// The rational an operator over two exact numbers answers.
    fn of_exact(v: crate::LogosValue) -> LogosRational {
        match v {
            crate::LogosValue::Int(n) => LogosRational::from_i64(n),
            crate::LogosValue::BigInt(b) => LogosRational(logicaffeine_base::Rational::from_bigint((*b).clone())),
            crate::LogosValue::Rational(r) => LogosRational((*r).clone()),
            other => unreachable!("an operator over exact numbers answered a {}", other.type_name()),
        }
    }

    fn exact_op(
        &self,
        other: &LogosRational,
        op: fn(&crate::LogosValue, &crate::LogosValue) -> Result<crate::LogosValue, String>,
    ) -> LogosRational {
        LogosRational::of_exact(crate::failure::answered(op(&self.exact(), &other.exact())))
    }

    fn integer_op(
        &self,
        other: &LogosRational,
        op: fn(&crate::LogosValue, &crate::LogosValue) -> Result<crate::LogosValue, String>,
    ) -> i64 {
        match crate::failure::answered(op(&self.exact(), &other.exact())) {
            crate::LogosValue::Int(n) => n,
            other => unreachable!("an integer operator answered a {}", other.type_name()),
        }
    }

    /// `self / other`: two whole values truncate as integers do, and otherwise the quotient is exact.
    pub fn quotient(&self, other: &LogosRational) -> LogosRational {
        self.exact_op(other, crate::LogosValue::div)
    }

    /// `self % other`: the remainder of two whole values, its sign the dividend's; a fraction has none.
    pub fn remainder(&self, other: &LogosRational) -> LogosRational {
        self.exact_op(other, crate::LogosValue::rem)
    }

    /// `self // other`: the floor of the exact quotient.
    pub fn floor_quotient(&self, other: &LogosRational) -> LogosRational {
        self.exact_op(other, crate::LogosValue::floor_div)
    }

    /// `self ** other`: a whole base takes an integer's power, a fraction a rational's.
    pub fn power(&self, other: &LogosRational) -> LogosRational {
        self.exact_op(other, crate::LogosValue::pow)
    }

    /// `self & other` of two whole values.
    pub fn bit_and(&self, other: &LogosRational) -> i64 {
        self.integer_op(other, crate::LogosValue::bit_and)
    }

    /// `self | other` of two whole values.
    pub fn bit_or(&self, other: &LogosRational) -> i64 {
        self.integer_op(other, crate::LogosValue::bit_or)
    }

    /// `self ^ other` of two whole values.
    pub fn bit_xor(&self, other: &LogosRational) -> i64 {
        self.integer_op(other, crate::LogosValue::bit_xor)
    }

    /// `self << other` of two whole values.
    pub fn shl(&self, other: &LogosRational) -> i64 {
        self.integer_op(other, crate::LogosValue::shl)
    }

    /// `self >> other` of two whole values.
    pub fn shr(&self, other: &LogosRational) -> i64 {
        self.integer_op(other, crate::LogosValue::shr)
    }

    /// The real the language computes with where this meets one.
    #[inline]
    pub fn to_f64(&self) -> f64 {
        self.0.to_f64()
    }

    /// How this compares with the real `other`, by that real's exact value; `None` against NaN.
    #[inline]
    pub fn order_real(&self, other: f64) -> Option<std::cmp::Ordering> {
        logicaffeine_base::numeric::cmp_rational_f64_exact(&self.0, other)
    }

    /// The machine integer this is where the language takes one (an index, a bound): stopped with
    /// `past_word` when it is whole but outside `i64`, and with `fraction` when it is not whole.
    pub fn integer(&self, past_word: &str, fraction: &str) -> i64 {
        if !self.0.is_integer() {
            return crate::failure::answered(Err(fraction.to_string()));
        }
        match self.0.to_bigint().and_then(|b| b.to_i64()) {
            Some(n) => n,
            None => crate::failure::answered(Err(past_word.to_string())),
        }
    }
}

impl std::fmt::Display for LogosRational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl From<i64> for LogosRational {
    #[inline]
    fn from(n: i64) -> Self {
        LogosRational::from_i64(n)
    }
}

/// The placeholder `#[derive(Default)]` on a record needs. The language holds nothing in a rational field a record is
/// built without, which a `LogosRational` cannot hold, so a compiled construction that leaves one unset is refused
/// and this value is never observed.
impl Default for LogosRational {
    fn default() -> Self {
        LogosRational::from_i64(0)
    }
}

impl From<LogosInt> for LogosRational {
    #[inline]
    fn from(n: LogosInt) -> Self {
        match n {
            LogosInt::Small(n) => LogosRational::from_i64(n),
            LogosInt::Big(b) => LogosRational(logicaffeine_base::Rational::from_bigint(*b)),
        }
    }
}

/// A value of integers held where the language holds rationals: an integer as the rational it equals, a list item
/// by item, at any depth.
pub trait ToRationals {
    /// The same value with each integer a rational.
    type Rationals;
    fn to_rationals(&self) -> Self::Rationals;
}

impl ToRationals for i64 {
    type Rationals = LogosRational;
    #[inline]
    fn to_rationals(&self) -> LogosRational {
        LogosRational::from_i64(*self)
    }
}

impl ToRationals for LogosRational {
    type Rationals = LogosRational;
    #[inline]
    fn to_rationals(&self) -> LogosRational {
        self.clone()
    }
}

impl<T: ToRationals> ToRationals for [T] {
    type Rationals = LogosSeq<T::Rationals>;
    fn to_rationals(&self) -> LogosSeq<T::Rationals> {
        LogosSeq::from_vec(self.iter().map(ToRationals::to_rationals).collect())
    }
}

impl<T: ToRationals> ToRationals for Vec<T> {
    type Rationals = LogosSeq<T::Rationals>;
    fn to_rationals(&self) -> LogosSeq<T::Rationals> {
        self.as_slice().to_rationals()
    }
}

impl<T: ToRationals> ToRationals for LogosSeq<T> {
    type Rationals = LogosSeq<T::Rationals>;
    fn to_rationals(&self) -> LogosSeq<T::Rationals> {
        self.0.borrow().as_slice().to_rationals()
    }
}

// Arithmetic OPERATORS so codegen can emit `a + b` for rationals (a `decimal ÷` widens to
// `LogosRational`, and a following `+`/`−`/`×` composed raw would otherwise be E0369). They delegate
// to the wrapped exact `Rational`, so they stay byte-identical to `.add()`/`.sub()`/`.mul()`. `/`
// panics on a zero divisor, mirroring integer `/ 0`.
impl std::ops::Add for LogosRational {
    type Output = LogosRational;
    #[inline]
    fn add(self, other: LogosRational) -> LogosRational {
        LogosRational(self.0.add(&other.0))
    }
}
impl std::ops::Sub for LogosRational {
    type Output = LogosRational;
    #[inline]
    fn sub(self, other: LogosRational) -> LogosRational {
        LogosRational(self.0.sub(&other.0))
    }
}
impl std::ops::Mul for LogosRational {
    type Output = LogosRational;
    #[inline]
    fn mul(self, other: LogosRational) -> LogosRational {
        LogosRational(self.0.mul(&other.0))
    }
}
impl std::ops::Div for LogosRational {
    type Output = LogosRational;
    #[inline]
    fn div(self, other: LogosRational) -> LogosRational {
        LogosRational(
            self.0
                .div(&other.0)
                .expect(crate::failure::DIVISION_BY_ZERO),
        )
    }
}

/// An exact base-10 fixed-point number — money's runtime type on the compiled-to-Rust
/// path, the AOT mirror of the interpreter's `Decimal`. `decimal("19.99")` compiles to
/// `LogosDecimal::parse("19.99")`; `+ − ×` stay exact `Decimal` (scale preserved), and `÷`
/// widens to the exact `LogosRational` (base-10 division need not terminate). `Display`
/// shows the scale faithfully (`19.99`, `20.00`), matching the interpreter.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogosDecimal(pub logicaffeine_base::Decimal);

/// A value `decimal(…)` takes: Text is read as a base-10 number, an Int widens exactly, a Decimal is
/// itself, and any other kind stops the program with [`crate::failure::decimal_needs_text_or_int`],
/// naming it as the engines do.
pub trait DecimalArg {
    fn into_decimal(self) -> LogosDecimal;
}

impl DecimalArg for &str {
    fn into_decimal(self) -> LogosDecimal {
        LogosDecimal::parse(self)
    }
}

impl DecimalArg for String {
    fn into_decimal(self) -> LogosDecimal {
        LogosDecimal::parse(&self)
    }
}

impl DecimalArg for &String {
    fn into_decimal(self) -> LogosDecimal {
        LogosDecimal::parse(self)
    }
}

impl DecimalArg for i64 {
    fn into_decimal(self) -> LogosDecimal {
        LogosDecimal::from_i64(self)
    }
}

impl DecimalArg for LogosInt {
    fn into_decimal(self) -> LogosDecimal {
        match self {
            LogosInt::Small(n) => LogosDecimal::from_i64(n),
            LogosInt::Big(b) => LogosDecimal::parse(&b.to_string()),
        }
    }
}

impl DecimalArg for LogosDecimal {
    fn into_decimal(self) -> LogosDecimal {
        self
    }
}

impl DecimalArg for &LogosDecimal {
    fn into_decimal(self) -> LogosDecimal {
        self.clone()
    }
}

impl DecimalArg for f64 {
    fn into_decimal(self) -> LogosDecimal {
        panic!("{}", crate::failure::decimal_needs_text_or_int("Float"))
    }
}

impl DecimalArg for bool {
    fn into_decimal(self) -> LogosDecimal {
        panic!("{}", crate::failure::decimal_needs_text_or_int("Bool"))
    }
}

impl DecimalArg for char {
    fn into_decimal(self) -> LogosDecimal {
        panic!("{}", crate::failure::decimal_needs_text_or_int("Char"))
    }
}

/// `decimal(v)`: see [`DecimalArg`].
pub fn decimal_of(v: impl DecimalArg) -> LogosDecimal {
    v.into_decimal()
}

impl LogosDecimal {
    /// A whole integer as a scale-0 decimal (`5 → "5"`).
    #[inline]
    pub fn from_i64(n: i64) -> Self {
        LogosDecimal(logicaffeine_base::Decimal::from_i64(n))
    }

    /// The exact value `decimal(text)` names: the text, surrounding whitespace aside, read as a
    /// base-10 number (`"19.99"`; [`logicaffeine_base::Decimal::parse`] owns the rule). Any other text
    /// stops the program with [`crate::failure::not_a_decimal`], as on every tier.
    #[inline]
    pub fn parse(s: &str) -> Self {
        match logicaffeine_base::Decimal::parse(s) {
            Some(d) => LogosDecimal(d),
            None => panic!("{}", crate::failure::not_a_decimal(s)),
        }
    }

    #[inline]
    pub fn add(&self, other: &LogosDecimal) -> LogosDecimal {
        LogosDecimal(self.0.add(&other.0))
    }

    #[inline]
    pub fn sub(&self, other: &LogosDecimal) -> LogosDecimal {
        LogosDecimal(self.0.sub(&other.0))
    }

    #[inline]
    pub fn mul(&self, other: &LogosDecimal) -> LogosDecimal {
        LogosDecimal(self.0.mul(&other.0))
    }

    /// The exact rational view (`coeff / 10^scale`) — the bridge `÷` widens through.
    #[inline]
    pub fn to_rational(&self) -> LogosRational {
        LogosRational(self.0.to_rational())
    }

    /// Exact division, widening to a `LogosRational` (base-10 division need not terminate).
    /// Panics on a zero divisor, mirroring integer `/ 0`.
    #[inline]
    pub fn div_exact(&self, other: &LogosDecimal) -> LogosRational {
        self.to_rational().div_exact(&other.to_rational())
    }

    /// The exact absolute value (scale preserved).
    #[inline]
    pub fn abs(&self) -> LogosDecimal {
        LogosDecimal(self.0.abs())
    }
}

impl std::fmt::Display for LogosDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl From<i64> for LogosDecimal {
    #[inline]
    fn from(n: i64) -> Self {
        LogosDecimal::from_i64(n)
    }
}

/// An exact complex number `re + im·i` — the AOT mirror of the interpreter's `Complex`.
/// `complex(0, 1)` compiles to `LogosComplex::new(..)`; `+ − × ÷` stay exact and closed
/// (`i·i = −1`). NOT ordered. `Display` shows `3+4i` / `i` / `-2i`, matching the interpreter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogosComplex(pub logicaffeine_base::Complex);

impl LogosComplex {
    /// A real integer as `re + 0i`.
    #[inline]
    pub fn from_i64(n: i64) -> Self {
        LogosComplex(logicaffeine_base::Complex::from_i64(n))
    }

    /// `re + im·i` from two exact rationals.
    #[inline]
    pub fn new(re: LogosRational, im: LogosRational) -> Self {
        LogosComplex(logicaffeine_base::Complex::new(re.0, im.0))
    }

    #[inline]
    pub fn add(&self, other: &LogosComplex) -> LogosComplex {
        LogosComplex(self.0.add(&other.0))
    }

    #[inline]
    pub fn sub(&self, other: &LogosComplex) -> LogosComplex {
        LogosComplex(self.0.sub(&other.0))
    }

    #[inline]
    pub fn mul(&self, other: &LogosComplex) -> LogosComplex {
        LogosComplex(self.0.mul(&other.0))
    }

    /// Exact division (the complex field is closed). Panics on a zero divisor, mirroring `/ 0`.
    #[inline]
    pub fn div_exact(&self, other: &LogosComplex) -> LogosComplex {
        LogosComplex(
            self.0
                .div(&other.0)
                .expect(crate::failure::DIVISION_BY_ZERO),
        )
    }

    /// The complex conjugate `re − im·i` — stays exact. The Hermitian-transpose primitive.
    #[inline]
    pub fn conjugate(&self) -> LogosComplex {
        LogosComplex(self.0.conjugate())
    }
    /// The real part as an `f64` (`re(z)`).
    #[inline]
    pub fn re(&self) -> f64 {
        self.0.re().to_f64()
    }
    /// The imaginary part as an `f64` (`im(z)`).
    #[inline]
    pub fn im(&self) -> f64 {
        self.0.im().to_f64()
    }
    /// The modulus `|z| = √(re² + im²)` as an `f64` — delegates to the wrapped `Complex`, so it is
    /// byte-identical to the interpreter's `abs(Complex)` (`RuntimeValue::Float(c.abs_f64())`).
    #[inline]
    pub fn abs_f64(&self) -> f64 {
        self.0.abs_f64()
    }
}

impl std::fmt::Display for LogosComplex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl From<i64> for LogosComplex {
    #[inline]
    fn from(n: i64) -> Self {
        LogosComplex::from_i64(n)
    }
}

/// A floating-point complex `re + im·i` (each part `f64`) — the AOT mirror of the interpreter's
/// `ComplexF` (LAPACK `Z`). `complex(3.0, 4.0)` compiles to `LogosComplexF::new(..)`; `+ − × ÷` and
/// `sqrt` are inexact `f64` (the reason it exists over the exact `LogosComplex`). NOT ordered.
/// `Display` matches the interpreter (`2+i`, `4+6i`) via `fmt_f64`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogosComplexF(pub logicaffeine_base::ComplexF);

impl LogosComplexF {
    /// A real `f64` as `re + 0i`.
    #[inline]
    pub fn from_f64(re: f64) -> Self {
        LogosComplexF(logicaffeine_base::ComplexF::from_f64(re))
    }
    /// `re + im·i` from two `f64`s.
    #[inline]
    pub fn new(re: f64, im: f64) -> Self {
        LogosComplexF(logicaffeine_base::ComplexF::new(re, im))
    }
    #[inline]
    pub fn add(&self, other: &LogosComplexF) -> LogosComplexF {
        LogosComplexF(self.0.add(&other.0))
    }
    #[inline]
    pub fn sub(&self, other: &LogosComplexF) -> LogosComplexF {
        LogosComplexF(self.0.sub(&other.0))
    }
    #[inline]
    pub fn mul(&self, other: &LogosComplexF) -> LogosComplexF {
        LogosComplexF(self.0.mul(&other.0))
    }
    /// Division; panics on a zero divisor, mirroring `/ 0`.
    #[inline]
    pub fn div_exact(&self, other: &LogosComplexF) -> LogosComplexF {
        LogosComplexF(
            self.0
                .div(&other.0)
                .expect(crate::failure::DIVISION_BY_ZERO),
        )
    }
    /// The principal square root (via the real `sqrt`) — the operation the exact `LogosComplex` lacks.
    #[inline]
    pub fn sqrt(&self) -> LogosComplexF {
        LogosComplexF(self.0.sqrt())
    }
    #[inline]
    pub fn conjugate(&self) -> LogosComplexF {
        LogosComplexF(self.0.conjugate())
    }
    #[inline]
    pub fn abs(&self) -> f64 {
        self.0.abs()
    }
    /// The real part as an `f64` (`re(z)`).
    #[inline]
    pub fn re(&self) -> f64 {
        self.0.re()
    }
    /// The imaginary part as an `f64` (`im(z)`).
    #[inline]
    pub fn im(&self) -> f64 {
        self.0.im()
    }
}

impl std::fmt::Display for LogosComplexF {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Match the interpreter's `ComplexF` display: `fmt_f64` parts with the unit coefficient omitted.
        let (re, im) = (self.0.re(), self.0.im());
        let g = crate::fmt::fmt_f64;
        if im == 0.0 {
            return write!(f, "{}", g(re));
        }
        let mag = im.abs();
        let icoef = if mag == 1.0 { String::new() } else { g(mag) };
        if re == 0.0 {
            write!(f, "{}{}i", if im < 0.0 { "-" } else { "" }, icoef)
        } else {
            write!(f, "{}{}{}i", g(re), if im < 0.0 { "-" } else { "+" }, icoef)
        }
    }
}

impl From<f64> for LogosComplexF {
    #[inline]
    fn from(re: f64) -> Self {
        LogosComplexF::from_f64(re)
    }
}

/// An element of the ring ℤ/nℤ — the AOT mirror of the interpreter's `Modular`.
/// `modular(value, modulus)` compiles to `LogosModular::new(..)`; `+ − ×` wrap in the ring,
/// `pow` is fast modular exponentiation, and `÷` multiplies by the modular inverse. NOT ordered.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogosModular(pub logicaffeine_base::Modular);

impl LogosModular {
    /// `value mod modulus`. Panics on a non-positive modulus (a LOGOS runtime error).
    #[inline]
    pub fn new(value: i64, modulus: i64) -> Self {
        LogosModular(
            logicaffeine_base::Modular::from_i64(value, modulus)
                .unwrap_or_else(|| panic!("{}", crate::failure::MODULUS_NOT_POSITIVE)),
        )
    }

    #[inline]
    pub fn add(&self, other: &LogosModular) -> LogosModular {
        LogosModular(
            self.0
                .add(&other.0)
                .unwrap_or_else(|| panic!("{}", crate::failure::modular_rings_differ("add"))),
        )
    }

    #[inline]
    pub fn sub(&self, other: &LogosModular) -> LogosModular {
        LogosModular(
            self.0
                .sub(&other.0)
                .unwrap_or_else(|| panic!("{}", crate::failure::modular_rings_differ("subtract"))),
        )
    }

    #[inline]
    pub fn mul(&self, other: &LogosModular) -> LogosModular {
        LogosModular(
            self.0
                .mul(&other.0)
                .unwrap_or_else(|| panic!("{}", crate::failure::modular_rings_differ("multiply"))),
        )
    }

    /// Division by the modular inverse. Stops the program when the moduli differ or the divisor is not
    /// coprime to the modulus.
    #[inline]
    pub fn div_exact(&self, other: &LogosModular) -> LogosModular {
        if self.0.modulus() != other.0.modulus() {
            panic!("{}", crate::failure::modular_rings_differ("divide"));
        }
        LogosModular(
            self.0
                .div(&other.0)
                .unwrap_or_else(|| panic!("{}", crate::failure::NO_MODULAR_INVERSE)),
        )
    }

    /// Fast modular exponentiation `self^exp`.
    #[inline]
    pub fn pow(&self, exp: u64) -> LogosModular {
        LogosModular(self.0.pow(exp))
    }
}

impl std::fmt::Display for LogosModular {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// A physical quantity on the compiled-to-Rust path — the AOT mirror of the interpreter's
/// `Quantity` value. The magnitude rides the exact rational tower (no float drift) and the display
/// unit travels with it so `Show` renders faithfully (`42/127 ft`). `+ −` require the same
/// dimension — the front-end's type checker proves this, so a mismatch is a *compile* error, not a
/// runtime one; the runtime check here is a defensive backstop. `× ÷` combine dimensions, and a
/// quantity may be scaled by a dimensionless number (unit preserved).
#[derive(Clone, Debug)]
pub struct LogosQuantity {
    pub q: logicaffeine_base::Quantity,
    pub unit: logicaffeine_base::Unit,
}

impl LogosQuantity {
    /// `value` of the named unit (`LogosQuantity::of(2, "inch")`).
    #[inline]
    pub fn of(value: i64, unit_name: &str) -> Self {
        Self::from_rational(LogosRational::from_i64(value), unit_name)
    }

    /// An exact-rational magnitude of the named unit. Panics on an unknown unit, which the
    /// front-end has already rejected before codegen.
    pub fn from_rational(value: LogosRational, unit_name: &str) -> Self {
        let unit = logicaffeine_base::quantity::units::by_name(unit_name)
            .unwrap_or_else(|| panic!("{}", crate::failure::unknown_unit(unit_name)));
        LogosQuantity {
            q: logicaffeine_base::Quantity::of(value.0, &unit),
            unit,
        }
    }

    #[inline]
    pub fn add(&self, other: &LogosQuantity) -> LogosQuantity {
        let q = self.q.add(&other.q).unwrap_or_else(|| {
            panic!(
                "{}",
                crate::failure::quantities_differ("add", self.q.dimension(), other.q.dimension())
            )
        });
        LogosQuantity {
            q,
            unit: self.unit.clone(),
        }
    }

    #[inline]
    pub fn sub(&self, other: &LogosQuantity) -> LogosQuantity {
        let q = self.q.sub(&other.q).unwrap_or_else(|| {
            panic!(
                "{}",
                crate::failure::quantities_differ(
                    "subtract",
                    self.q.dimension(),
                    other.q.dimension()
                )
            )
        });
        LogosQuantity {
            q,
            unit: self.unit.clone(),
        }
    }

    /// `× ÷` combine dimensions; the result is shown in SI/dimension form (empty display unit).
    #[inline]
    pub fn mul(&self, other: &LogosQuantity) -> LogosQuantity {
        let q = self.q.mul(&other.q);
        let unit = Self::si_unit(&q);
        LogosQuantity { q, unit }
    }

    #[inline]
    pub fn div_exact(&self, other: &LogosQuantity) -> LogosQuantity {
        let q = self
            .q
            .div(&other.q)
            .unwrap_or_else(|| panic!("{}", crate::failure::DIVIDE_BY_ZERO_QUANTITY));
        let unit = Self::si_unit(&q);
        LogosQuantity { q, unit }
    }

    /// Scale by a dimensionless number (unit preserved): `q * k`.
    #[inline]
    pub fn scale(&self, k: &LogosRational) -> LogosQuantity {
        let mag = self.q.magnitude_si().mul(&k.0);
        LogosQuantity {
            q: logicaffeine_base::Quantity::si(mag, self.q.dimension()),
            unit: self.unit.clone(),
        }
    }

    /// Scale by a dimensionless integer (unit preserved) — the common `q * 3` case from codegen.
    #[inline]
    pub fn scale_int(&self, k: i64) -> LogosQuantity {
        self.scale(&LogosRational::from_i64(k))
    }

    /// Divide by a dimensionless integer (unit preserved) — the common `q / 2` case from codegen.
    #[inline]
    pub fn div_int(&self, k: i64) -> LogosQuantity {
        self.div_scalar(&LogosRational::from_i64(k))
    }

    /// Divide by a dimensionless number (unit preserved): `q / k`.
    #[inline]
    pub fn div_scalar(&self, k: &LogosRational) -> LogosQuantity {
        let mag = self
            .q
            .magnitude_si()
            .div(&k.0)
            .unwrap_or_else(|| panic!("{}", crate::failure::DIVIDE_QUANTITY_BY_ZERO));
        LogosQuantity {
            q: logicaffeine_base::Quantity::si(mag, self.q.dimension()),
            unit: self.unit.clone(),
        }
    }

    /// Re-express in another unit of the SAME dimension. A different dimension is the forbidden
    /// cast — the type checker rejects it, so this panic is a defensive backstop.
    pub fn convert(&self, unit_name: &str) -> LogosQuantity {
        let unit = logicaffeine_base::quantity::units::by_name(unit_name)
            .unwrap_or_else(|| panic!("{}", crate::failure::unknown_unit(unit_name)));
        if self.q.dimension() != unit.dimension {
            panic!(
                "{}",
                crate::failure::cannot_convert(&self.unit.symbol, &unit.symbol)
            );
        }
        LogosQuantity {
            q: self.q.clone(),
            unit,
        }
    }

    /// A synthetic SI-base unit (empty symbol) for a combined dimension.
    fn si_unit(q: &logicaffeine_base::Quantity) -> logicaffeine_base::Unit {
        logicaffeine_base::Unit::linear("", q.dimension(), logicaffeine_base::Rational::one())
    }
}

// Equality is PHYSICAL (SI magnitude + dimension); the display unit is presentation only, so
// `100 cm` equals `1 m`. Ordering is by magnitude within a shared dimension (the type checker
// guarantees comparisons are same-dimension).
impl PartialEq for LogosQuantity {
    fn eq(&self, other: &Self) -> bool {
        self.q == other.q
    }
}
impl Eq for LogosQuantity {}

impl PartialOrd for LogosQuantity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.q.dimension() != other.q.dimension() {
            return None;
        }
        self.q.magnitude_si().partial_cmp(other.q.magnitude_si())
    }
}

impl std::fmt::Display for LogosQuantity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let magnitude = self
            .q
            .in_unit(&self.unit)
            .expect("a Quantity's display unit always shares its dimension");
        if self.unit.symbol.is_empty() {
            write!(f, "{} {}", magnitude, self.q.dimension())
        } else {
            write!(f, "{} {}", magnitude, self.unit.symbol)
        }
    }
}

/// Money for the compiled tier — an exact amount in a currency, mirroring `base::Money`. `+ −`
/// require the SAME currency (a runtime-panic backstop; the type checker rejects mismatches first),
/// `× ÷` scale by an exact number, and a same-currency `÷` is the dimensionless ratio.
#[derive(Clone, Debug)]
pub struct LogosMoney(pub logicaffeine_base::Money);

// `Default` for the exact value types — required so a user struct with a value-type field can
// `#[derive(Default)]` (the AOT-Rust struct codegen always derives Default and constructs via
// `Struct { field: v, ..Default::default() }`, which needs Default even when every field is given).
// These are zero/nil/dimensionless placeholders, only ever materialized for an UNINITIALIZED field
// (which the construction immediately overwrites), so the degenerate currency/modulus/unit is never observed.
impl Default for LogosDecimal {
    fn default() -> Self {
        LogosDecimal(logicaffeine_base::Decimal::zero())
    }
}
impl Default for LogosComplex {
    fn default() -> Self {
        LogosComplex(logicaffeine_base::Complex::zero())
    }
}
impl Default for LogosModular {
    fn default() -> Self {
        LogosModular(
            logicaffeine_base::Modular::from_i64(0, 1).expect("0 mod 1 is a valid modular value"),
        )
    }
}
impl Default for LogosUuid {
    fn default() -> Self {
        LogosUuid(logicaffeine_base::Uuid::NIL)
    }
}
impl Default for LogosMoney {
    fn default() -> Self {
        LogosMoney::from_i64(0, "USD")
    }
}
impl Default for LogosQuantity {
    fn default() -> Self {
        LogosQuantity {
            q: logicaffeine_base::Quantity::scalar(logicaffeine_base::Rational::zero()),
            unit: logicaffeine_base::Unit::linear(
                "",
                logicaffeine_base::Dimension::DIMENSIONLESS,
                logicaffeine_base::Rational::one(),
            ),
        }
    }
}

impl LogosMoney {
    /// Money of a `Decimal` amount in the named currency. Panics on an unknown currency, which the
    /// front-end has already rejected before codegen.
    pub fn of(amount: LogosDecimal, code: &str) -> Self {
        let currency = logicaffeine_base::money::currency::by_code(code)
            .unwrap_or_else(|| panic!("{}", crate::failure::unknown_currency(code)));
        LogosMoney(logicaffeine_base::Money::of(amount.0, currency))
    }

    /// Money of an integer amount — the `5 USD` case (codegen passes the literal directly).
    pub fn from_i64(amount: i64, code: &str) -> Self {
        Self::of(
            LogosDecimal(logicaffeine_base::Decimal::from_i64(amount)),
            code,
        )
    }

    #[inline]
    pub fn add(&self, other: &LogosMoney) -> LogosMoney {
        LogosMoney(self.0.add(&other.0).unwrap_or_else(|| {
            panic!(
                "{}",
                crate::failure::currencies_differ(
                    "add",
                    self.0.currency.code,
                    other.0.currency.code
                )
            )
        }))
    }

    #[inline]
    pub fn sub(&self, other: &LogosMoney) -> LogosMoney {
        LogosMoney(self.0.sub(&other.0).unwrap_or_else(|| {
            panic!(
                "{}",
                crate::failure::currencies_differ(
                    "subtract",
                    self.0.currency.code,
                    other.0.currency.code
                )
            )
        }))
    }

    /// Scale by an integer (`19.99 USD × 3`), re-quantised to the currency.
    #[inline]
    pub fn scale_int(&self, k: i64) -> LogosMoney {
        LogosMoney(self.0.scale_int(k))
    }

    /// Scale by an exact decimal (`price × 1.5`), re-quantised to the currency.
    #[inline]
    pub fn scale_decimal(&self, k: &LogosDecimal) -> LogosMoney {
        LogosMoney(logicaffeine_base::Money::of(
            self.0.amount.mul(&k.0),
            self.0.currency,
        ))
    }

    /// Divide by an integer (split a bill), re-quantised to the currency.
    #[inline]
    pub fn div_int(&self, k: i64) -> LogosMoney {
        let d = self
            .0
            .amount
            .div(
                &logicaffeine_base::Decimal::from_i64(k),
                self.0.currency.scale,
                logicaffeine_base::RoundingMode::HalfEven,
            )
            .unwrap_or_else(|| panic!("{}", crate::failure::DIVIDE_MONEY_BY_ZERO));
        LogosMoney(logicaffeine_base::Money::of(d, self.0.currency))
    }

    /// The exact dimensionless ratio of two same-currency amounts.
    #[inline]
    pub fn ratio(&self, other: &LogosMoney) -> LogosRational {
        match self.0.ratio(&other.0) {
            Some(r) => LogosRational(r),
            None if self.0.currency != other.0.currency => panic!(
                "{}",
                crate::failure::currencies_differ(
                    "compare",
                    self.0.currency.code,
                    other.0.currency.code
                )
            ),
            None => panic!("{}", crate::failure::DIVIDE_MONEY_BY_ZERO_AMOUNT),
        }
    }
}

impl PartialEq for LogosMoney {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for LogosMoney {}

impl PartialOrd for LogosMoney {
    /// Ordered by amount within a currency; across currencies it is incomparable (`None`).
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.0.currency != other.0.currency {
            return None;
        }
        self.0
            .amount
            .to_rational()
            .partial_cmp(&other.0.amount.to_rational())
    }
}

impl std::fmt::Display for LogosMoney {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A 128-bit UUID for the compiled tier (RFC 9562), mirroring `logicaffeine_base::Uuid`. A `Copy` newtype over
/// `[u8; 16]` — no allocation, byte-ordered comparison (so v6/v7 sort by time), canonical text form.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LogosUuid(pub logicaffeine_base::Uuid);

impl LogosUuid {
    /// Parse from canonical/simple/braced/urn text. Panics on a malformed id (the front-end has
    /// already validated literal forms; a runtime parse of bad data is a clean program error).
    pub fn parse(s: &str) -> Self {
        LogosUuid(
            logicaffeine_base::Uuid::parse(s)
                .unwrap_or_else(|| panic!("{}", crate::failure::invalid_uuid(s))),
        )
    }
    /// The nil (all-zero) UUID.
    pub fn nil() -> Self {
        LogosUuid(logicaffeine_base::Uuid::NIL)
    }
    /// The max (all-one) UUID.
    pub fn max() -> Self {
        LogosUuid(logicaffeine_base::Uuid::MAX)
    }
    /// The version nibble (0 for nil, 1–8 for the defined versions, 15 for max).
    pub fn version(&self) -> i64 {
        self.0.version() as i64
    }
    /// The well-known DNS / URL / OID / X.500 namespaces.
    pub fn namespace_dns() -> Self {
        LogosUuid(logicaffeine_base::Uuid::NAMESPACE_DNS)
    }
    pub fn namespace_url() -> Self {
        LogosUuid(logicaffeine_base::Uuid::NAMESPACE_URL)
    }
    pub fn namespace_oid() -> Self {
        LogosUuid(logicaffeine_base::Uuid::NAMESPACE_OID)
    }
    pub fn namespace_x500() -> Self {
        LogosUuid(logicaffeine_base::Uuid::NAMESPACE_X500)
    }
}

impl LogosUuid {
    /// The UUID's 16 bytes as a `Seq of Int` — the byte view the Logos-written version constructors
    /// (uuid.lg) prepend as the namespace.
    pub fn byte_seq(&self) -> LogosSeq<i64> {
        LogosSeq::from_vec(self.0.as_bytes().iter().map(|&b| b as i64).collect())
    }
    /// Build a UUID from the first 16 bytes of a `Seq of Int` (the compiled mirror of the
    /// `uuid_from_bytes` builtin). Panics on fewer than 16 bytes — the front-end feeds a digest.
    pub fn from_byte_seq(seq: &LogosSeq<i64>) -> LogosUuid {
        let v = seq.0.borrow();
        if v.len() < 16 {
            panic!("{}", crate::failure::uuid_needs_16_bytes(v.len()));
        }
        let mut b = [0u8; 16];
        for (i, slot) in b.iter_mut().enumerate() {
            *slot = (v[i] & 0xff) as u8;
        }
        LogosUuid(logicaffeine_base::Uuid::from_bytes(b))
    }
}

/// A Text's UTF-8 bytes as a `Seq of Int` (the `text_bytes` builtin, AOT).
pub fn text_bytes(s: &str) -> LogosSeq<i64> {
    LogosSeq::from_vec(s.as_bytes().iter().map(|&b| b as i64).collect())
}

/// Rebuild a Text from a `Seq of Int` of UTF-8 bytes (the `text_from_bytes` builtin, AOT —
/// the exact inverse of [`text_bytes`]).
///
/// ★ INVALID BYTES ARE A FAILURE, NOT A SUBSTITUTION. This used to fall back to
/// `from_utf8_lossy`, justified by a premise written in this very comment: *"inputs originate
/// from `text_bytes`, so the bytes are always valid UTF-8"*. **Our own stdlib falsifies it** —
/// `fromHex` (encoding.lg) decodes arbitrary hex and hands the result straight in, so
/// `fromHex("ff")` arrives here with byte 255.
///
/// The consequence was three answers to one question: the tree-walker and the VM raised
/// `text_from_bytes(): invalid UTF-8`, this returned `U+FFFD`, and the wasm lowering checked
/// nothing at all. A panic is what the AOT tier means by "the reference tier errored" —
/// `readBytes` already answers that way — so the tiers now agree.
///
/// A premise asserted in a doc comment is enforced by nothing, and the caller that violates it
/// is usually in a different file from the one that states it.
pub fn text_from_bytes(bytes: &LogosSeq<i64>) -> String {
    let b: Vec<u8> = bytes.iter().map(|v| v as u8).collect();
    match String::from_utf8(b) {
        Ok(s) => s,
        Err(e) => panic!("{}", crate::failure::invalid_utf8(e)),
    }
}

impl std::fmt::Display for LogosUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An exchange rate accepted by [`set_rate`] — a plain integer (`1`), an exact `LogosDecimal`
/// (`1.10`), or a `LogosRational`. All widen losslessly onto the Rational tower so normalisation
/// stays exact, matching the interpreter's `set_rate` builtin.
pub trait IntoRate {
    fn into_rate(self) -> logicaffeine_base::Rational;
}
impl IntoRate for i64 {
    fn into_rate(self) -> logicaffeine_base::Rational {
        logicaffeine_base::Rational::from_i64(self)
    }
}
impl IntoRate for LogosDecimal {
    fn into_rate(self) -> logicaffeine_base::Rational {
        self.0.to_rational()
    }
}
impl IntoRate for LogosRational {
    fn into_rate(self) -> logicaffeine_base::Rational {
        self.0
    }
}

/// Install (or replace) one exchange rate in the ambient rate context — `1 <code> = rate` reference
/// units. The compiled-tier mirror of the `set_rate` builtin; a fresh process starts with no rates,
/// so a program installs them before any `<money> in <currency>` conversion.
pub fn set_rate<R: IntoRate>(code: String, rate: R) {
    logicaffeine_base::money::set_ambient_rate(&code, rate.into_rate());
}

/// Bulk-install a whole exchange-rate table from a `Map of Text to <number>` (currency code → rate
/// vs the reference) into the ambient rate context. The compiled-tier mirror of the `set_rates`
/// builtin and the bridge a literal, network-synced, or fetched table feeds. Order-independent.
pub fn set_rates<V: IntoRate + Clone>(map: LogosMap<String, V>) {
    for (code, rate) in map.0.borrow().iter() {
        logicaffeine_base::money::set_ambient_rate(code, rate.clone().into_rate());
    }
}

/// Convert money to another currency via the ambient rate context — the `<money> in <currency>`
/// surface. Panics (the typed-error backstop, like a currency-mismatch `+`) when no rates are in
/// scope or the currency lacks a rate; the front-end / interpreter surface these as clean errors.
pub fn to_currency(money: LogosMoney, code: String) -> LogosMoney {
    let to = logicaffeine_base::money::currency::by_code(&code)
        .unwrap_or_else(|| panic!("{}", crate::failure::unknown_currency(&code)));
    let converted = logicaffeine_base::money::ambient_convert(&money.0, to).unwrap_or_else(|| {
        if logicaffeine_base::money::has_ambient_rates() {
            panic!(
                "{}",
                crate::failure::no_exchange_rate(money.0.currency.code, to.code)
            )
        } else {
            panic!("{}", crate::failure::NO_EXCHANGE_RATES)
        }
    });
    LogosMoney(converted)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/types/logos_money_tests.rs"]
mod logos_money_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/types/logos_quantity_tests.rs"]
mod logos_quantity_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/types/logos_rational_tests.rs"]
mod logos_rational_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/types/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/types/logos_bits_tests.rs"]
mod logos_bits_tests;

// ===========================================================================
// LogosInt — the EXACT compiled integer (overflow ruling v2, stage 2)
// ===========================================================================

/// The exact integer for GENERATED code: `i64` on the fast path, spilling to a
/// heap [`logicaffeine_base::BigInt`] the moment a value escapes 64 bits — the
/// AOT's mirror of the interpreter's Int→BigInt promotion (and of the JIT's
/// overflow side-exit). Every operation NORMALIZES: a big result that fits
/// `i64` downsizes to `Small`, so representation never leaks into `==`/`Ord`.
#[derive(Clone, Debug)]
pub enum LogosInt {
    Small(i64),
    Big(Box<logicaffeine_base::BigInt>),
}

/// Zero, so a struct holding a promoted `Int` can still `#[derive(Default)]` — the AOT-Rust
/// struct codegen always derives it and constructs with `..Default::default()`.
impl Default for LogosInt {
    fn default() -> Self {
        LogosInt::Small(0)
    }
}

impl LogosInt {
    #[inline]
    pub fn from_i64(x: i64) -> Self {
        LogosInt::Small(x)
    }

    pub fn from_big(b: logicaffeine_base::BigInt) -> Self {
        match b.to_i64() {
            Some(x) => LogosInt::Small(x),
            None => LogosInt::Big(Box::new(b)),
        }
    }

    /// Parse a (possibly > 64-bit) decimal literal the codegen emitted.
    pub fn from_literal(s: &str) -> Self {
        if let Ok(x) = s.parse::<i64>() {
            return LogosInt::Small(x);
        }
        let (neg, digits) = match s.strip_prefix('-') {
            Some(d) => (true, d),
            None => (false, s),
        };
        let ten = logicaffeine_base::BigInt::from_i64(10);
        let mut acc = logicaffeine_base::BigInt::from_i64(0);
        for ch in digits.bytes() {
            debug_assert!(ch.is_ascii_digit(), "LogosInt literal digit");
            acc = acc
                .mul(&ten)
                .add(&logicaffeine_base::BigInt::from_i64((ch - b'0') as i64));
        }
        if neg {
            acc = acc.negated();
        }
        LogosInt::from_big(acc)
    }

    fn to_bigint(&self) -> logicaffeine_base::BigInt {
        match self {
            LogosInt::Small(x) => logicaffeine_base::BigInt::from_i64(*x),
            LogosInt::Big(b) => (**b).clone(),
        }
    }

    /// Borrow the value as a `&BigInt` WITHOUT cloning a heap `Big`. A `Small`
    /// materializes a cheap single-limb `BigInt` (no heap); a `Big` borrows its box.
    /// This is the allocation-avoiding counterpart to [`to_bigint`] for the by-ref
    /// `BigInt` arithmetic below — a growing accumulator (`factorial`, `pow_exact`)
    /// was being cloned once per operation purely to pass it by reference.
    #[inline]
    fn as_bigint(&self) -> std::borrow::Cow<'_, logicaffeine_base::BigInt> {
        match self {
            LogosInt::Small(x) => std::borrow::Cow::Owned(logicaffeine_base::BigInt::from_i64(*x)),
            LogosInt::Big(b) => std::borrow::Cow::Borrowed(&**b),
        }
    }

    /// The value as `i64` when it fits.
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            LogosInt::Small(x) => Some(*x),
            LogosInt::Big(b) => b.to_i64(),
        }
    }

    /// The low 64 bits of this integer, reduced modulo 2^64.
    #[inline]
    pub fn wrapping_u64(&self) -> u64 {
        match self {
            Self::Small(value) => *value as u64,
            Self::Big(value) => value.wrapping_u64(),
        }
    }

    /// The value as `i64`, or a LOUD canonical error — for sinks that are
    /// structurally 64-bit (indices, sizes, native call arguments).
    #[inline]
    pub fn expect_i64(&self, what: &str) -> i64 {
        match self.to_i64() {
            Some(x) => x,
            None => panic!("Integer overflow: {self} does not fit a 64-bit {what}"),
        }
    }

    #[inline]
    pub fn add(&self, rhs: &LogosInt) -> LogosInt {
        if let (LogosInt::Small(a), LogosInt::Small(b)) = (self, rhs) {
            if let Some(s) = a.checked_add(*b) {
                return LogosInt::Small(s);
            }
        }
        let (a, b) = (self.as_bigint(), rhs.as_bigint());
        LogosInt::from_big(a.add(&b))
    }

    #[inline]
    pub fn sub(&self, rhs: &LogosInt) -> LogosInt {
        if let (LogosInt::Small(a), LogosInt::Small(b)) = (self, rhs) {
            if let Some(s) = a.checked_sub(*b) {
                return LogosInt::Small(s);
            }
        }
        let (a, b) = (self.as_bigint(), rhs.as_bigint());
        LogosInt::from_big(a.sub(&b))
    }

    #[inline]
    pub fn mul(&self, rhs: &LogosInt) -> LogosInt {
        if let (LogosInt::Small(a), LogosInt::Small(b)) = (self, rhs) {
            if let Some(p) = a.checked_mul(*b) {
                return LogosInt::Small(p);
            }
        }
        let (a, b) = (self.as_bigint(), rhs.as_bigint());
        LogosInt::from_big(a.mul(&b))
    }

    /// In-place multiply by a machine-integer scalar — the exact-value equivalent of
    /// `*self = self.mul(&LogosInt::from(rhs))`, but a `Big` accumulator mutates its heap
    /// buffer in place instead of allocating a fresh product each call (the `Set p to p * i`
    /// factorial hot loop). A `Small` stays on the checked fast path, promoting to `Big` only
    /// on overflow. A `Big` never shrinks below the i64 range under a nonzero scalar, so it
    /// stays a canonical `Big`; a zero scalar collapses to `Small(0)`.
    #[inline]
    pub fn mul_assign_i64(&mut self, rhs: i64) {
        match self {
            LogosInt::Small(a) => match a.checked_mul(rhs) {
                Some(p) => *a = p,
                None => {
                    let big = logicaffeine_base::BigInt::from_i64(*a)
                        .mul(&logicaffeine_base::BigInt::from_i64(rhs));
                    *self = LogosInt::from_big(big);
                }
            },
            LogosInt::Big(b) => {
                if rhs == 0 {
                    *self = LogosInt::Small(0);
                } else {
                    b.mul_assign_i64(rhs);
                }
            }
        }
    }

    /// Truncating division; `Err` on a zero divisor (the interp's catchable
    /// "Division by zero"). `i64::MIN / -1` promotes exactly.
    #[inline]
    pub fn div(&self, rhs: &LogosInt) -> Result<LogosInt, String> {
        if let (LogosInt::Small(a), LogosInt::Small(b)) = (self, rhs) {
            if *b == 0 {
                return Err(crate::failure::DIVISION_BY_ZERO.to_string());
            }
            if let Some(q) = a.checked_div(*b) {
                return Ok(LogosInt::Small(q));
            }
        }
        match self.to_bigint().div_rem(&rhs.to_bigint()) {
            Some((q, _)) => Ok(LogosInt::from_big(q)),
            None => Err(crate::failure::DIVISION_BY_ZERO.to_string()),
        }
    }

    /// Floor division — the quotient rounded toward NEGATIVE INFINITY (`-7 // 2 → -4`),
    /// the semantics of the `//` operator. Distinct from [`div`](Self::div), which
    /// truncates toward zero (`-7 / 2 → -3`); the two agree when the operands share a
    /// sign. Exact (promotes to `BigInt`); a zero divisor is a loud error.
    pub fn div_floor(&self, rhs: &LogosInt) -> Result<LogosInt, String> {
        if let (LogosInt::Small(a), LogosInt::Small(b)) = (self, rhs) {
            if *b == 0 {
                return Err(crate::failure::DIVISION_BY_ZERO.to_string());
            }
            // `i64::MIN / -1` overflows the Small path → fall through to the exact BigInt
            // branch. Otherwise floor = truncate, minus one when a nonzero remainder means
            // the true quotient sits below the truncated one (operands of opposite sign).
            if let (Some(q), Some(r)) = (a.checked_div(*b), a.checked_rem(*b)) {
                let floored = if r != 0 && (r < 0) != (*b < 0) {
                    q - 1
                } else {
                    q
                };
                return Ok(LogosInt::Small(floored));
            }
        }
        match self.to_bigint().div_rem(&rhs.to_bigint()) {
            Some((q, r)) => {
                let floored = if !r.is_zero() && r.is_negative() != rhs.to_bigint().is_negative() {
                    q.sub(&logicaffeine_base::BigInt::from_i64(1))
                } else {
                    q
                };
                Ok(LogosInt::from_big(floored))
            }
            None => Err(crate::failure::DIVISION_BY_ZERO.to_string()),
        }
    }

    /// Truncating remainder (sign of the dividend); `i64::MIN % -1` is 0.
    #[inline]
    pub fn rem(&self, rhs: &LogosInt) -> Result<LogosInt, String> {
        if let (LogosInt::Small(a), LogosInt::Small(b)) = (self, rhs) {
            if *b == 0 {
                return Err(crate::failure::MODULO_BY_ZERO.to_string());
            }
            if let Some(r) = a.checked_rem(*b) {
                return Ok(LogosInt::Small(r));
            }
            return Ok(LogosInt::Small(0)); // MIN % -1
        }
        match self.to_bigint().div_rem(&rhs.to_bigint()) {
            Some((_, r)) => Ok(LogosInt::from_big(r)),
            None => Err(crate::failure::MODULO_BY_ZERO.to_string()),
        }
    }

    /// Exponentiation with an integer exponent. `Err` on a negative exponent
    /// (an Int can't hold the fractional result) or one too large to be a
    /// `u32` power; overflow of the i64 fast path promotes to `BigInt`
    /// exactly. Mirrors the interpreter's `int_power`.
    pub fn pow(&self, exp: &LogosInt) -> Result<LogosInt, String> {
        let e = match exp.to_i64() {
            Some(e) if e < 0 => {
                return Err("negative exponent on an integer (an Int can't hold a fraction — use a Float base)".to_string());
            }
            Some(e) => e,
            None => return Err("exponent too large".to_string()),
        };
        let e = u32::try_from(e).map_err(|_| "exponent too large".to_string())?;
        if let LogosInt::Small(b) = self {
            if let Some(r) = b.checked_pow(e) {
                return Ok(LogosInt::Small(r));
            }
        }
        Ok(LogosInt::from_big(self.to_bigint().pow(e)))
    }

    pub fn neg(&self) -> LogosInt {
        match self {
            LogosInt::Small(x) => match x.checked_neg() {
                Some(n) => LogosInt::Small(n),
                None => LogosInt::from_big(self.to_bigint().negated()),
            },
            LogosInt::Big(b) => LogosInt::from_big(b.negated()),
        }
    }
}

impl From<i64> for LogosInt {
    #[inline]
    fn from(x: i64) -> Self {
        LogosInt::Small(x)
    }
}

impl PartialEq for LogosInt {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // Normalization invariant: a Big never holds an i64-fitting value.
            (LogosInt::Small(a), LogosInt::Small(b)) => a == b,
            (LogosInt::Big(a), LogosInt::Big(b)) => **a == **b,
            _ => false,
        }
    }
}
impl Eq for LogosInt {}

impl PartialEq<i64> for LogosInt {
    fn eq(&self, other: &i64) -> bool {
        matches!(self, LogosInt::Small(a) if a == other)
    }
}
impl PartialEq<LogosInt> for i64 {
    fn eq(&self, other: &LogosInt) -> bool {
        other == self
    }
}

impl Ord for LogosInt {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match (self, other) {
            (LogosInt::Small(a), LogosInt::Small(b)) => a.cmp(b),
            _ => self.to_bigint().cmp(&other.to_bigint()),
        }
    }
}
impl PartialOrd for LogosInt {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialOrd<i64> for LogosInt {
    fn partial_cmp(&self, other: &i64) -> Option<core::cmp::Ordering> {
        self.partial_cmp(&LogosInt::Small(*other))
    }
}
impl PartialOrd<LogosInt> for i64 {
    fn partial_cmp(&self, other: &LogosInt) -> Option<core::cmp::Ordering> {
        LogosInt::Small(*self).partial_cmp(other)
    }
}

impl core::hash::Hash for LogosInt {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // The unified numeric hash (mod 2^61−1) — coherent with equal i64s.
        match self {
            LogosInt::Small(x) => logicaffeine_base::numeric::numeric_hash_i64(*x).hash(state),
            LogosInt::Big(b) => logicaffeine_base::numeric::numeric_hash_bigint(b).hash(state),
        }
    }
}

impl core::fmt::Display for LogosInt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LogosInt::Small(x) => write!(f, "{x}"),
            LogosInt::Big(b) => write!(f, "{b}"),
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/types/logos_int_spec.rs"]
mod logos_int_spec;
