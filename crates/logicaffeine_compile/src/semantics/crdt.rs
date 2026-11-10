//! CRDT runtime values for the tree-walker interpreter.
//!
//! The compiled (AOT) tier lowers a `Shared` struct's collection fields to the real
//! `logicaffeine_data::crdt` types — `ORSet`, `RGA`, `MVRegister` — and runs their full
//! convergent semantics. For the interpreter to MEAN THE SAME THING as the compiler
//! (the futamura invariant the cross-tier differential tests lock), it holds those very
//! same types, not a lookalike `Set`/`List`. Merge is then concurrent-correct by
//! construction: both tiers call into one `Merge` implementation, so there is no second
//! copy of the convergence logic that could drift.
//!
//! The language's shared collections are homogeneous over a concrete element type
//! (`SharedSet of Text`, `… of Int`) exactly as the compiled tier's `ORSet<String>` is.
//! [`CrdtScalar`] is the dynamically-typed element that covers every element type the
//! surface syntax can name; it is `Hash + Eq + Ord + Clone`, so it satisfies the
//! data-crate CRDT bounds.

use crate::interpreter::RuntimeValue;
use logicaffeine_data::crdt::{
    DeltaCrdt, GCounter, LatestWrite, MVRegister, Merge, ORMap, ORSet, PNCounter, RemoveWins,
    ReplicaId, VClock, RGA,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// A monotonic source of replica ids for CRDTs the interpreter constructs. Each `new`
/// shared collection gets a DISTINCT id, so two replicas built in one program (then
/// merged) carry independent causal histories — the precondition for correct OR-Set /
/// RGA convergence. The absolute id value never affects observable output (it only tags
/// internal causal metadata), so a process-global counter keeps every run deterministic.
static REPLICA_SEQ: AtomicU64 = AtomicU64::new(1);

/// Allocate the next distinct replica id.
pub fn next_replica_id() -> ReplicaId {
    REPLICA_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// A scalar CRDT element. CRDT collections are homogeneous, so a collection only ever
/// holds one of these variants in practice; the enum exists because the interpreter is
/// dynamically typed and the element type is not threaded through `struct_defs`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum CrdtScalar {
    Int(i64),
    Text(String),
    Bool(bool),
}

/// The placeholder a map entry holds before its first write merges in: a `LatestWrite` at stamp 0,
/// which every real write (stamp ≥ 1) outranks, so the placeholder is never observed.
impl Default for CrdtScalar {
    fn default() -> Self {
        CrdtScalar::Int(0)
    }
}

impl CrdtScalar {
    /// Project a [`RuntimeValue`] onto a CRDT element. Only the scalar types the surface
    /// syntax can put in a shared collection are accepted; anything else is a type error
    /// at the seam (never a silent coercion).
    pub fn from_runtime(value: &RuntimeValue) -> Result<CrdtScalar, String> {
        match value {
            RuntimeValue::Int(n) => Ok(CrdtScalar::Int(*n)),
            RuntimeValue::Text(s) => Ok(CrdtScalar::Text((**s).clone())),
            RuntimeValue::Bool(b) => Ok(CrdtScalar::Bool(*b)),
            other => Err(format!(
                "a shared collection holds Int, Text, or Bool elements, not {}",
                other.type_name()
            )),
        }
    }

    /// Reconstitute a [`RuntimeValue`] from a CRDT element.
    pub fn to_runtime(&self) -> RuntimeValue {
        match self {
            CrdtScalar::Int(n) => RuntimeValue::Int(*n),
            CrdtScalar::Text(s) => RuntimeValue::Text(std::rc::Rc::new(s.clone())),
            CrdtScalar::Bool(b) => RuntimeValue::Bool(*b),
        }
    }

    /// The canonical `Show` rendering of an element — matches how the compiled tier
    /// prints the same scalar.
    pub fn render(&self) -> String {
        match self {
            CrdtScalar::Int(n) => n.to_string(),
            CrdtScalar::Text(s) => s.clone(),
            CrdtScalar::Bool(b) => b.to_string(),
        }
    }
}

/// Every kind of CRDT the language has — the ONE table for what each is called in source, the code a
/// `NewCrdt` op carries in bytecode, and the tag its full state / deltas carry on the wire. Both codes
/// are stable wire formats: never renumber one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrdtKind {
    Set,
    SetRemoveWins,
    Seq,
    Register,
    Counter,
    PnCounter,
    Map,
}

impl CrdtKind {
    pub const ALL: [CrdtKind; 7] = [
        CrdtKind::Set,
        CrdtKind::SetRemoveWins,
        CrdtKind::Seq,
        CrdtKind::Register,
        CrdtKind::Counter,
        CrdtKind::PnCounter,
        CrdtKind::Map,
    ];

    /// The CRDT a declared field type names, or `None` for a type that is not one.
    pub fn of_type_name(name: &str) -> Option<CrdtKind> {
        CrdtKind::TYPE_NAMES.iter().find(|(n, _)| *n == name).map(|(_, kind)| *kind)
    }

    /// Every declared type name that names a CRDT, with its kind.
    pub const TYPE_NAMES: &'static [(&'static str, CrdtKind)] = &[
        ("SharedSet", CrdtKind::Set),
        ("ORSet", CrdtKind::Set),
        ("SharedSet_AddWins", CrdtKind::Set),
        ("SharedSet_RemoveWins", CrdtKind::SetRemoveWins),
        ("SharedSequence", CrdtKind::Seq),
        ("RGA", CrdtKind::Seq),
        ("SharedSequence_YATA", CrdtKind::Seq),
        ("CollaborativeSequence", CrdtKind::Seq),
        ("ConvergentCount", CrdtKind::Counter),
        ("GCounter", CrdtKind::Counter),
        ("Tally", CrdtKind::PnCounter),
        ("PNCounter", CrdtKind::PnCounter),
        ("SharedMap", CrdtKind::Map),
        ("ORMap", CrdtKind::Map),
    ];

    /// Whether `backend` lowers this CRDT — single-replica, running identically to the interpreter and
    /// the VM. [`crate::compile::refuse_unlinkable`] refuses a program that builds a kind its backend
    /// answers `false` for, naming the type. Exhaustive over kind × backend with no wildcard, so a new
    /// kind or a new backend does not compile until every pair is decided; `crdt_backend_lock` runs
    /// every pair and holds each answer to what the backend actually does.
    pub const fn lowered_by(self, backend: crate::compile::Backend) -> bool {
        use crate::compile::Backend;
        match self {
            CrdtKind::Set => match backend {
                Backend::C => false,
                Backend::CFromAst => false,
                Backend::RiscV => false,
                Backend::WebAssembly => true,
                Backend::SoftCore => false,
            },
            CrdtKind::SetRemoveWins => match backend {
                Backend::C => false,
                Backend::CFromAst => false,
                Backend::RiscV => false,
                Backend::WebAssembly => true,
                Backend::SoftCore => false,
            },
            CrdtKind::Seq => match backend {
                Backend::C => true,
                Backend::CFromAst => true,
                Backend::RiscV => true,
                Backend::WebAssembly => true,
                Backend::SoftCore => false,
            },
            CrdtKind::Register => match backend {
                Backend::C => true,
                Backend::CFromAst => true,
                Backend::RiscV => true,
                Backend::WebAssembly => true,
                Backend::SoftCore => true,
            },
            CrdtKind::Counter => match backend {
                Backend::C => true,
                Backend::CFromAst => true,
                Backend::RiscV => true,
                Backend::WebAssembly => true,
                Backend::SoftCore => false,
            },
            CrdtKind::PnCounter => match backend {
                Backend::C => true,
                Backend::CFromAst => true,
                Backend::RiscV => true,
                Backend::WebAssembly => true,
                Backend::SoftCore => false,
            },
            CrdtKind::Map => match backend {
                Backend::C => false,
                Backend::CFromAst => false,
                Backend::RiscV => false,
                Backend::WebAssembly => true,
                Backend::SoftCore => false,
            },
        }
    }

    /// The type name a value of this kind answers ([`CrdtValue::kind`]): its declared name ([`Self::type_name`])
    /// without a set's bias.
    pub const fn value_type_name(self) -> &'static str {
        match self {
            CrdtKind::Set | CrdtKind::SetRemoveWins => "SharedSet",
            CrdtKind::Seq => "SharedSequence",
            CrdtKind::Register => "Divergent",
            CrdtKind::Counter => "ConvergentCount",
            CrdtKind::PnCounter => "Tally",
            CrdtKind::Map => "SharedMap",
        }
    }

    /// A fresh, empty replica of this kind with its own replica id.
    pub fn fresh(self) -> CrdtValue {
        let replica = next_replica_id();
        match self {
            CrdtKind::Set => CrdtValue::new_set(replica),
            CrdtKind::SetRemoveWins => CrdtValue::new_set_remove_wins(replica),
            CrdtKind::Seq => CrdtValue::new_seq(replica),
            CrdtKind::Register => CrdtValue::new_register(replica),
            CrdtKind::Counter => CrdtValue::new_counter(replica),
            CrdtKind::PnCounter => CrdtValue::new_pn_counter(replica),
            CrdtKind::Map => CrdtValue::new_map(replica),
        }
    }

    /// The `kind` a `NewCrdt` op carries in bytecode.
    pub const fn new_crdt_code(self) -> u8 {
        match self {
            CrdtKind::Set => 0,
            CrdtKind::Seq => 1,
            CrdtKind::Register => 2,
            CrdtKind::SetRemoveWins => 3,
            CrdtKind::Counter => 4,
            CrdtKind::PnCounter => 5,
            CrdtKind::Map => 6,
        }
    }

    pub fn from_new_crdt_code(code: u8) -> Option<CrdtKind> {
        Self::ALL.into_iter().find(|k| k.new_crdt_code() == code)
    }

    /// The tag leading a full-state or delta payload.
    pub const fn state_tag(self) -> u8 {
        match self {
            CrdtKind::Set => 0,
            CrdtKind::SetRemoveWins => 1,
            CrdtKind::Seq => 2,
            CrdtKind::Register => 3,
            CrdtKind::Counter => 4,
            CrdtKind::PnCounter => 5,
            CrdtKind::Map => 6,
        }
    }

    pub fn from_state_tag(tag: u8) -> Option<CrdtKind> {
        Self::ALL.into_iter().find(|k| k.state_tag() == tag)
    }

    /// The name a program declares it by.
    pub const fn type_name(self) -> &'static str {
        match self {
            CrdtKind::Set => "SharedSet",
            CrdtKind::SetRemoveWins => "SharedSet (RemoveWins)",
            CrdtKind::Seq => "SharedSequence",
            CrdtKind::Register => "Divergent",
            CrdtKind::Counter => "ConvergentCount",
            CrdtKind::PnCounter => "Tally",
            CrdtKind::Map => "SharedMap",
        }
    }

    /// Why a backend that does not implement this CRDT refuses a program that uses it.
    pub fn refusal(self, backend: &str) -> String {
        format!(
            "`{}` is a CRDT the {backend} backend does not implement — run the program on the \
             interpreter or the VM, or build it with the Rust tier (`largo build`)",
            self.type_name()
        )
    }
}

/// A live CRDT held by the interpreter. Wraps the actual `logicaffeine_data` type, so its
/// convergence behaviour is identical to the compiled tier's by construction.
#[derive(Debug, Clone)]
pub enum CrdtValue {
    /// `SharedSet of T` / `SharedSet (AddWins)` — observed-remove set, add-wins on a
    /// concurrent add/remove of the same element.
    Set(ORSet<CrdtScalar>),
    /// `SharedSet (RemoveWins)` — observed-remove set, remove-wins on a concurrent
    /// add/remove (e.g. an access-revocation / block list).
    SetRemoveWins(ORSet<CrdtScalar, RemoveWins>),
    /// `SharedSequence of T` — replicated growable array (`RGA`). Stable, convergent
    /// insertion order.
    Seq(RGA<CrdtScalar>),
    /// `Divergent T` — multi-value register (`MVRegister`). Keeps concurrent writes until
    /// one is resolved.
    Register(MVRegister<CrdtScalar>),
    /// `ConvergentCount` — grow-only counter (`GCounter`): each replica owns a slot, the value
    /// is the sum, and merge is per-replica MAX. State-based ⇒ IDEMPOTENT: redelivering the same
    /// remote state never double-counts (the very property a bare-`Int` op-based counter lacks).
    Counter(GCounter),
    /// `Tally` — increment/decrement counter (`PNCounter`): two G-Counters (increments and
    /// decrements), value = Σinc − Σdec, merge per-replica MAX on both. Idempotent like `Counter`.
    PnCounter(PNCounter),
    /// `SharedMap from K to V` — observed-remove keys (add-wins), each value a Lamport-stamped
    /// last-writer register (`ORMap<K, LatestWrite<V>>`), exactly the compiled tier's map.
    Map(ORMap<CrdtScalar, LatestWrite<CrdtScalar>>),
}

impl CrdtValue {
    pub fn new_set(replica: ReplicaId) -> CrdtValue {
        CrdtValue::Set(ORSet::new(replica))
    }

    pub fn new_set_remove_wins(replica: ReplicaId) -> CrdtValue {
        CrdtValue::SetRemoveWins(ORSet::new(replica))
    }

    pub fn new_seq(replica: ReplicaId) -> CrdtValue {
        CrdtValue::Seq(RGA::new(replica))
    }

    pub fn new_register(replica: ReplicaId) -> CrdtValue {
        CrdtValue::Register(MVRegister::new(replica))
    }

    pub fn new_counter(replica: ReplicaId) -> CrdtValue {
        CrdtValue::Counter(GCounter::with_replica_id(replica))
    }

    pub fn new_pn_counter(replica: ReplicaId) -> CrdtValue {
        CrdtValue::PnCounter(PNCounter::with_replica_id(replica))
    }

    pub fn new_map(replica: ReplicaId) -> CrdtValue {
        CrdtValue::Map(ORMap::new(replica))
    }

    /// `Set <map>[key] to value`.
    pub fn map_put(&mut self, key: &RuntimeValue, value: &RuntimeValue) -> Result<(), String> {
        match self {
            CrdtValue::Map(m) => {
                m.put(
                    CrdtScalar::from_runtime(key)?,
                    CrdtScalar::from_runtime(value)?,
                );
                Ok(())
            }
            other => Err(format!("cannot set a key of a {}", other.kind())),
        }
    }

    /// `<map>[key]` — the value under a PRESENT key; a missing key traps exactly as a `Map`'s does.
    pub fn map_get(&self, key: &RuntimeValue) -> Result<RuntimeValue, String> {
        match self {
            CrdtValue::Map(m) => m
                .value(&CrdtScalar::from_runtime(key)?)
                .map(CrdtScalar::to_runtime)
                .ok_or_else(|| {
                    logicaffeine_data::failure::key_not_found(key.to_display_string().as_str())
                }),
            other => Err(format!("cannot index a {} by key", other.kind())),
        }
    }

    /// `Increase/Decrease <counter> by N` — bump this replica's own slot. A grow-only
    /// `ConvergentCount` rejects a decrement (that is what a `Tally` is for); a `Tally` takes both.
    /// The per-replica slot is what makes a later merge idempotent.
    pub fn bump(&mut self, amount: i64) -> Result<(), String> {
        match self {
            CrdtValue::Counter(c) => {
                if amount < 0 {
                    return Err(
                        "a ConvergentCount is grow-only — use a Tally to decrement".to_string()
                    );
                }
                c.increment(amount as u64);
                Ok(())
            }
            CrdtValue::PnCounter(c) => {
                if amount >= 0 {
                    c.increment(amount as u64);
                } else {
                    c.decrement(amount.unsigned_abs());
                }
                Ok(())
            }
            other => Err(format!("cannot increment a {}", other.kind())),
        }
    }

    /// The integer value of a counter (`GCounter` sum, or `PNCounter` inc−dec); `None` for a
    /// non-counter CRDT. This is how a counter reads as a plain number for `Show` and comparison.
    pub fn counter_value(&self) -> Option<i64> {
        match self {
            CrdtValue::Counter(c) => Some(c.value() as i64),
            CrdtValue::PnCounter(c) => Some(c.value()),
            _ => None,
        }
    }

    /// Which CRDT this is.
    pub fn crdt_kind(&self) -> CrdtKind {
        match self {
            CrdtValue::Set(_) => CrdtKind::Set,
            CrdtValue::SetRemoveWins(_) => CrdtKind::SetRemoveWins,
            CrdtValue::Seq(_) => CrdtKind::Seq,
            CrdtValue::Register(_) => CrdtKind::Register,
            CrdtValue::Counter(_) => CrdtKind::Counter,
            CrdtValue::PnCounter(_) => CrdtKind::PnCounter,
            CrdtValue::Map(_) => CrdtKind::Map,
        }
    }

    /// The CRDT kind, used for type errors and `type_name`.
    pub fn kind(&self) -> &'static str {
        self.crdt_kind().value_type_name()
    }

    /// `Add element to <set>` — insert into the observed-remove set.
    pub fn insert(&mut self, value: &RuntimeValue) -> Result<(), String> {
        let e = CrdtScalar::from_runtime(value)?;
        match self {
            CrdtValue::Set(s) => Ok(s.insert(e)),
            CrdtValue::SetRemoveWins(s) => Ok(s.insert(e)),
            other => Err(format!("cannot add an element to a {}", other.kind())),
        }
    }

    /// `Remove element from <set>` / `Remove key from <map>` — observed-remove deletion.
    pub fn remove(&mut self, value: &RuntimeValue) -> Result<(), String> {
        let e = CrdtScalar::from_runtime(value)?;
        match self {
            CrdtValue::Set(s) => Ok(s.remove(&e)),
            CrdtValue::SetRemoveWins(s) => Ok(s.remove(&e)),
            CrdtValue::Map(m) => Ok(m.remove(&e)),
            other => Err(format!("cannot remove an element from a {}", other.kind())),
        }
    }

    /// `<set> contains element` / `<map> contains key` — observed-remove membership.
    pub fn contains(&self, value: &RuntimeValue) -> Result<bool, String> {
        let e = CrdtScalar::from_runtime(value)?;
        match self {
            CrdtValue::Set(s) => Ok(s.contains(&e)),
            CrdtValue::SetRemoveWins(s) => Ok(s.contains(&e)),
            CrdtValue::Map(m) => Ok(m.contains_key(&e)),
            other => Err(format!("a {} has no membership test", other.kind())),
        }
    }

    /// `Append element to <sequence>` — RGA append at the end.
    pub fn append(&mut self, value: &RuntimeValue) -> Result<(), String> {
        match self {
            CrdtValue::Seq(s) => {
                s.append(CrdtScalar::from_runtime(value)?);
                Ok(())
            }
            other => Err(format!("cannot append to a {}", other.kind())),
        }
    }

    /// `Set <register> to value` / `Resolve <register> to value` — the divergent register
    /// takes a value that dominates all concurrent ones.
    pub fn resolve(&mut self, value: &RuntimeValue) -> Result<(), String> {
        match self {
            CrdtValue::Register(r) => {
                r.resolve(CrdtScalar::from_runtime(value)?);
                Ok(())
            }
            other => Err(format!("cannot resolve a {}", other.kind())),
        }
    }

    /// `length of <collection>` — element count (set cardinality or sequence length).
    pub fn len(&self) -> usize {
        match self {
            CrdtValue::Set(s) => s.len(),
            CrdtValue::SetRemoveWins(s) => s.len(),
            CrdtValue::Seq(s) => s.len(),
            CrdtValue::Register(r) => r.values().len(),
            CrdtValue::Map(m) => m.len(),
            // A counter is a scalar, not a collection — it has no cardinality.
            CrdtValue::Counter(_) | CrdtValue::PnCounter(_) => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The elements as runtime values, in convergent order for a sequence and sorted for a
    /// set (sets have no intrinsic order — sorting makes the rendering deterministic).
    pub fn to_runtime_vec(&self) -> Vec<RuntimeValue> {
        match self {
            CrdtValue::Set(s) => {
                let mut elems: Vec<&CrdtScalar> = s.iter().collect();
                elems.sort();
                elems.into_iter().map(CrdtScalar::to_runtime).collect()
            }
            CrdtValue::SetRemoveWins(s) => {
                let mut elems: Vec<&CrdtScalar> = s.iter().collect();
                elems.sort();
                elems.into_iter().map(CrdtScalar::to_runtime).collect()
            }
            CrdtValue::Seq(s) => s.to_vec().iter().map(CrdtScalar::to_runtime).collect(),
            CrdtValue::Register(r) => {
                let mut vals: Vec<&CrdtScalar> = r.values();
                vals.sort();
                vals.into_iter().map(CrdtScalar::to_runtime).collect()
            }
            // A map's elements are its `(key, value)` pairs in key order — what `Repeat for` visits.
            CrdtValue::Map(m) => m
                .sorted_entries()
                .into_iter()
                .map(|(k, v)| {
                    RuntimeValue::Tuple(std::rc::Rc::new(vec![
                        k.to_runtime(),
                        v.get().to_runtime(),
                    ]))
                })
                .collect(),
            // A counter's "elements" view is its single integer value (used only as a fallback;
            // `render`/`crdt_values_equal` handle counters directly).
            CrdtValue::Counter(_) | CrdtValue::PnCounter(_) => {
                vec![RuntimeValue::Int(self.counter_value().unwrap_or(0))]
            }
        }
    }

    /// The resolved value of a register — the single current value, or `None` while it is
    /// empty or still divergent (multiple concurrent values).
    pub fn register_value(&self) -> Option<RuntimeValue> {
        match self {
            CrdtValue::Register(r) => {
                let vals = r.values();
                if vals.len() == 1 {
                    Some(vals[0].to_runtime())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Merge another CRDT of the SAME kind into this one (the convergent join). Mismatched
    /// kinds are a type error rather than a silent no-op.
    pub fn merge(&mut self, other: &CrdtValue) -> Result<(), String> {
        match (self, other) {
            (CrdtValue::Set(a), CrdtValue::Set(b)) => {
                a.merge(b);
                Ok(())
            }
            (CrdtValue::SetRemoveWins(a), CrdtValue::SetRemoveWins(b)) => {
                a.merge(b);
                Ok(())
            }
            (CrdtValue::Seq(a), CrdtValue::Seq(b)) => {
                a.merge(b);
                Ok(())
            }
            (CrdtValue::Register(a), CrdtValue::Register(b)) => {
                a.merge(b);
                Ok(())
            }
            // Per-replica MAX (idempotent) — the join that never double-counts a redelivered state.
            (CrdtValue::Counter(a), CrdtValue::Counter(b)) => {
                a.merge(b);
                Ok(())
            }
            (CrdtValue::PnCounter(a), CrdtValue::PnCounter(b)) => {
                a.merge(b);
                Ok(())
            }
            (CrdtValue::Map(a), CrdtValue::Map(b)) => {
                a.merge(b);
                Ok(())
            }
            (a, b) => Err(format!("cannot merge a {} with a {}", a.kind(), b.kind())),
        }
    }

    /// The current causal version (vector clock) — every update this replica has observed. The
    /// δ-CRDT `mergeable` send remembers the version it last shipped to a peer and ships only the
    /// delta SINCE that version, so a large set/sequence syncs its few NEW entries, not its whole
    /// state. Idempotent + commutative, so reordering or redelivering deltas still converges.
    pub fn version(&self) -> VClock {
        match self {
            CrdtValue::Set(s) => s.version(),
            CrdtValue::SetRemoveWins(s) => s.version(),
            CrdtValue::Seq(s) => s.version(),
            CrdtValue::Register(s) => s.version(),
            CrdtValue::PnCounter(s) => s.version(),
            CrdtValue::Map(s) => s.version(),
            // A `GCounter` keeps no vector clock; its full state is tiny, so the sync ships it whole
            // (a valid state-based delta) — always current, and idempotent to re-apply.
            CrdtValue::Counter(_) => VClock::new(),
        }
    }

    /// Serialize the changes since `since` as a KIND-TAGGED delta payload — far smaller than the
    /// full state when little changed. `None` if the variant can't produce a delta (history
    /// truncated); the caller falls back to the full-state `crdt_to_wire`.
    pub fn delta_since_bytes(&self, since: &VClock) -> Option<Vec<u8>> {
        // bincode, NOT serde_json: a delta's `entries` is a map keyed by the ELEMENT (a `CrdtScalar`
        // enum), and JSON cannot encode a non-string map key — bincode serializes maps as (key,value)
        // sequences, so any key type round-trips.
        fn tagged<D: serde::Serialize>(kind: u8, delta: &D) -> Option<Vec<u8>> {
            let mut out = vec![kind];
            out.extend(bincode::serialize(delta).ok()?);
            Some(out)
        }
        let tag = self.crdt_kind().state_tag();
        match self {
            CrdtValue::Set(s) => tagged(tag, &s.delta_since(since)?),
            CrdtValue::SetRemoveWins(s) => tagged(tag, &s.delta_since(since)?),
            CrdtValue::Seq(s) => tagged(tag, &s.delta_since(since)?),
            CrdtValue::Register(s) => tagged(tag, &s.delta_since(since)?),
            // Counters ship full state as their delta (state-based, idempotent to merge).
            CrdtValue::Counter(s) => tagged(tag, s),
            CrdtValue::PnCounter(s) => tagged(tag, s),
            CrdtValue::Map(s) => tagged(tag, &s.delta_since(since)?),
        }
    }

    /// Apply a kind-tagged delta from `delta_since_bytes` of the SAME kind. Returns false on a
    /// kind mismatch or malformed bytes, leaving self unchanged — a foreign / garbage delta never
    /// corrupts the CRDT (the network-edge safety the `mergeable` receive needs).
    pub fn apply_delta_bytes(&mut self, bytes: &[u8]) -> bool {
        let Some((&kind, body)) = bytes.split_first() else {
            return false;
        };
        if CrdtKind::from_state_tag(kind) != Some(self.crdt_kind()) {
            return false;
        }
        match self {
            CrdtValue::Set(s) => {
                match bincode::deserialize::<<ORSet<CrdtScalar> as DeltaCrdt>::Delta>(body) {
                    Ok(d) => {
                        s.apply_delta(&d);
                        true
                    }
                    Err(_) => false,
                }
            }
            CrdtValue::SetRemoveWins(s) => {
                match bincode::deserialize::<<ORSet<CrdtScalar, RemoveWins> as DeltaCrdt>::Delta>(
                    body,
                ) {
                    Ok(d) => {
                        s.apply_delta(&d);
                        true
                    }
                    Err(_) => false,
                }
            }
            CrdtValue::Seq(s) => {
                match bincode::deserialize::<<RGA<CrdtScalar> as DeltaCrdt>::Delta>(body) {
                    Ok(d) => {
                        s.apply_delta(&d);
                        true
                    }
                    Err(_) => false,
                }
            }
            CrdtValue::Register(s) => {
                match bincode::deserialize::<<MVRegister<CrdtScalar> as DeltaCrdt>::Delta>(body) {
                    Ok(d) => {
                        s.apply_delta(&d);
                        true
                    }
                    Err(_) => false,
                }
            }
            // Counter deltas are full states — deserialize and MERGE (per-replica max, idempotent).
            CrdtValue::Counter(s) => match bincode::deserialize::<GCounter>(body) {
                Ok(d) => {
                    s.merge(&d);
                    true
                }
                Err(_) => false,
            },
            CrdtValue::PnCounter(s) => match bincode::deserialize::<PNCounter>(body) {
                Ok(d) => {
                    s.merge(&d);
                    true
                }
                Err(_) => false,
            },
            CrdtValue::Map(s) => {
                match bincode::deserialize::<
                    <ORMap<CrdtScalar, LatestWrite<CrdtScalar>> as DeltaCrdt>::Delta,
                >(body)
                {
                    Ok(d) => {
                        s.apply_delta(&d);
                        true
                    }
                    Err(_) => false,
                }
            }
        }
    }

    /// Serialize the WHOLE convergent state as a KIND-TAGGED payload — the state-based delta since
    /// the empty clock (every variant produces a valid full state at `VClock::new()`). This is what
    /// crosses a task/`Sync` boundary: the far side rebuilds a fresh replica and MERGES it, so
    /// re-applying the same bytes is idempotent. Distinct from `crdt_to_wire` (which projects a
    /// counter to a plain-Int relay form); this preserves the FULL per-replica CRDT state.
    pub fn to_full_state_bytes(&self) -> Vec<u8> {
        // Every variant yields `Some` at the empty clock (a full state is always producible), but a
        // truncated-history variant could theoretically return `None`; degrade to an empty payload,
        // which `from_full_state_bytes` reconstructs as a fresh (empty) replica.
        self.delta_since_bytes(&VClock::new()).unwrap_or_default()
    }

    /// Reconstruct a CRDT from `to_full_state_bytes`: build a FRESH empty replica of the tagged kind,
    /// then apply the state (a per-replica merge). A malformed / empty payload degrades to a fresh
    /// grow-only counter (the safe default for the counter-`Sync` path). The fresh replica id keeps
    /// the rebuilt value from aliasing the sender's live cell.
    pub fn from_full_state_bytes(bytes: &[u8]) -> CrdtValue {
        let kind = bytes
            .first()
            .and_then(|&t| CrdtKind::from_state_tag(t))
            .unwrap_or(CrdtKind::Counter);
        let mut cv = kind.fresh();
        cv.apply_delta_bytes(bytes);
        cv
    }

    /// `Show` rendering: a set renders in set notation `{…}` (matching a plain `Set`), a
    /// sequence as an ordered list `[…]`, a register as its resolved value (or, while still
    /// divergent, the brace-joined set of concurrent values).
    pub fn render(&self) -> String {
        let parts = || -> String {
            self.to_runtime_vec()
                .iter()
                .map(render_runtime)
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            CrdtValue::Register(_) => match self.register_value() {
                Some(v) => render_runtime(&v),
                None => format!("{{{}}}", parts()),
            },
            CrdtValue::Set(_) | CrdtValue::SetRemoveWins(_) => format!("{{{}}}", parts()),
            CrdtValue::Seq(_) => format!("[{}]", parts()),
            // A map renders like a `Map` — `{key: value, …}` — in key order.
            CrdtValue::Map(m) => format!(
                "{{{}}}",
                m.sorted_entries()
                    .into_iter()
                    .map(|(k, v)| format!("{}: {}", k.render(), v.get().render()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            // A counter renders as its integer value — the same as the compiled tier's `Showable`.
            CrdtValue::Counter(_) | CrdtValue::PnCounter(_) => {
                self.counter_value().unwrap_or(0).to_string()
            }
        }
    }
}

/// Structural equality used by the interpreter's `values_equal` for CRDT values: two
/// CRDTs are equal when they observe the same elements (sequences also compare order).
pub fn crdt_values_equal(a: &CrdtValue, b: &CrdtValue) -> bool {
    match (a, b) {
        (CrdtValue::Set(_), CrdtValue::Set(_))
        | (CrdtValue::SetRemoveWins(_), CrdtValue::SetRemoveWins(_))
        | (CrdtValue::Seq(_), CrdtValue::Seq(_))
        | (CrdtValue::Register(_), CrdtValue::Register(_))
        | (CrdtValue::Map(_), CrdtValue::Map(_)) => a.to_runtime_vec() == b.to_runtime_vec(),
        // Two counters are equal when their integer values agree (internal per-replica slots differ).
        (CrdtValue::Counter(_), CrdtValue::Counter(_))
        | (CrdtValue::PnCounter(_), CrdtValue::PnCounter(_)) => {
            a.counter_value() == b.counter_value()
        }
        _ => false,
    }
}

/// Render a runtime scalar the way `Show` does for a CRDT element (text is unquoted).
fn render_runtime(v: &RuntimeValue) -> String {
    match v {
        RuntimeValue::Int(n) => n.to_string(),
        RuntimeValue::Text(s) => (**s).clone(),
        RuntimeValue::Bool(b) => b.to_string(),
        other => other.type_name().to_string(),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/crdt/tests.rs"]
mod tests;
