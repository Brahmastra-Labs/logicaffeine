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
use logicaffeine_data::crdt::{Merge, MVRegister, ORSet, RemoveWins, ReplicaId, RGA};
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CrdtScalar {
    Int(i64),
    Text(String),
    Bool(bool),
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

    /// The CRDT kind, used for type errors and `type_name`.
    pub fn kind(&self) -> &'static str {
        match self {
            CrdtValue::Set(_) | CrdtValue::SetRemoveWins(_) => "SharedSet",
            CrdtValue::Seq(_) => "SharedSequence",
            CrdtValue::Register(_) => "Divergent",
        }
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

    /// `Remove element from <set>` — observed-remove deletion.
    pub fn remove(&mut self, value: &RuntimeValue) -> Result<(), String> {
        let e = CrdtScalar::from_runtime(value)?;
        match self {
            CrdtValue::Set(s) => Ok(s.remove(&e)),
            CrdtValue::SetRemoveWins(s) => Ok(s.remove(&e)),
            other => Err(format!("cannot remove an element from a {}", other.kind())),
        }
    }

    /// `<set> contains element` — observed-remove membership.
    pub fn contains(&self, value: &RuntimeValue) -> Result<bool, String> {
        let e = CrdtScalar::from_runtime(value)?;
        match self {
            CrdtValue::Set(s) => Ok(s.contains(&e)),
            CrdtValue::SetRemoveWins(s) => Ok(s.contains(&e)),
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
            (a, b) => Err(format!("cannot merge a {} with a {}", a.kind(), b.kind())),
        }
    }

    /// `Show` rendering: a set renders in set notation `{…}` (matching a plain `Set`), a
    /// sequence as an ordered list `[…]`, a register as its resolved value (or, while still
    /// divergent, the brace-joined set of concurrent values).
    pub fn render(&self) -> String {
        let parts = || -> String {
            self.to_runtime_vec().iter().map(render_runtime).collect::<Vec<_>>().join(", ")
        };
        match self {
            CrdtValue::Register(_) => match self.register_value() {
                Some(v) => render_runtime(&v),
                None => format!("{{{}}}", parts()),
            },
            CrdtValue::Set(_) | CrdtValue::SetRemoveWins(_) => format!("{{{}}}", parts()),
            CrdtValue::Seq(_) => format!("[{}]", parts()),
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
        | (CrdtValue::Register(_), CrdtValue::Register(_)) => {
            a.to_runtime_vec() == b.to_runtime_vec()
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
mod tests {
    use super::*;

    /// A tiny deterministic PRNG (SplitMix64) so the property tests are exhaustively random
    /// yet perfectly reproducible — no `rand`, no flakiness.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn int(n: u64) -> RuntimeValue {
        RuntimeValue::Int(n as i64)
    }

    /// A random OR-Set: a fresh replica with a random add/remove op stream over a small
    /// element domain (so collisions and removals actually happen).
    fn rand_set(rng: &mut Rng, domain: u64, ops: usize) -> CrdtValue {
        let mut s = CrdtValue::new_set(next_replica_id());
        for _ in 0..ops {
            let e = int(rng.below(domain));
            if rng.below(3) == 0 {
                let _ = s.remove(&e);
            } else {
                let _ = s.insert(&e);
            }
        }
        s
    }

    /// A random RGA: a fresh replica appending random elements.
    fn rand_seq(rng: &mut Rng, domain: u64, ops: usize) -> CrdtValue {
        let mut s = CrdtValue::new_seq(next_replica_id());
        for _ in 0..ops {
            let _ = s.append(&int(rng.below(domain)));
        }
        s
    }

    /// `a ⊔ b` as a fresh value (never aliases an operand).
    fn join(a: &CrdtValue, b: &CrdtValue) -> CrdtValue {
        let mut out = a.clone();
        out.merge(b).unwrap();
        out
    }

    #[test]
    fn orset_merge_obeys_the_crdt_laws() {
        let mut rng = Rng(0xDEAD_BEEF);
        for _ in 0..500 {
            let a = rand_set(&mut rng, 6, 8);
            let b = rand_set(&mut rng, 6, 8);
            let c = rand_set(&mut rng, 6, 8);
            assert!(crdt_values_equal(&join(&a, &b), &join(&b, &a)), "commutative");
            assert!(
                crdt_values_equal(&join(&join(&a, &b), &c), &join(&a, &join(&b, &c))),
                "associative"
            );
            assert!(crdt_values_equal(&join(&a, &a), &a), "idempotent");
        }
    }

    #[test]
    fn rga_merge_obeys_the_crdt_laws() {
        let mut rng = Rng(0x0123_4567);
        for _ in 0..500 {
            let a = rand_seq(&mut rng, 6, 6);
            let b = rand_seq(&mut rng, 6, 6);
            let c = rand_seq(&mut rng, 6, 6);
            assert!(crdt_values_equal(&join(&a, &b), &join(&b, &a)), "commutative");
            assert!(
                crdt_values_equal(&join(&join(&a, &b), &c), &join(&a, &join(&b, &c))),
                "associative"
            );
            assert!(crdt_values_equal(&join(&a, &a), &a), "idempotent");
        }
    }

    /// The distinguishing OR-Set property, at the unit level: replica `a` observes-and-
    /// removes "X", replica `b` adds "X" CONCURRENTLY; after the join "X" survives, because
    /// `b`'s add carries a tag `a`'s remove never saw. A grow-set-with-one-tombstone fails.
    #[test]
    fn orset_add_wins_over_concurrent_remove() {
        let x = int(42);
        let mut a = CrdtValue::new_set(next_replica_id());
        let mut b = CrdtValue::new_set(next_replica_id());
        a.insert(&x).unwrap();
        b.insert(&x).unwrap();
        a.remove(&x).unwrap();
        let m = join(&a, &b);
        assert!(m.contains(&x).unwrap(), "the concurrent add must survive the remove");
    }

    #[test]
    fn scalar_conversions_round_trip() {
        for v in [
            RuntimeValue::Int(-7),
            RuntimeValue::Text(std::rc::Rc::new("héllo".to_string())),
            RuntimeValue::Bool(true),
        ] {
            let s = CrdtScalar::from_runtime(&v).unwrap();
            assert_eq!(s.to_runtime(), v);
        }
        // A non-scalar is refused at the seam, not silently coerced.
        assert!(CrdtScalar::from_runtime(&RuntimeValue::Float(1.5)).is_err());
    }
}
