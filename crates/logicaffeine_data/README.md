# logicaffeine-data

WASM-safe data structures and CRDTs with no IO dependencies.

## Overview

This crate provides pure data structures that compile for both native and `wasm32-unknown-unknown` targets.

### The Lamport Invariant

This crate has **NO path to system IO**. No tokio, no libp2p, no network dependencies. Timestamps and IO operations are injected by callers (typically from `logicaffeine_system`).

This separation enables:
- Compilation to WebAssembly without modification
- Deterministic testing of CRDT logic
- Clear architectural boundaries between pure data and effectful IO

## Core Types

Scalar and collection types for the LOGOS type system:

| Type | Rust Mapping | Description |
|------|--------------|-------------|
| `Nat` | `u64` | Natural numbers (non-negative) |
| `Int` | `i64` | Integers |
| `Real` | `f64` | Floating-point numbers |
| `Text` | `String` | UTF-8 text |
| `Bool` | `bool` | Boolean values |
| `Unit` | `()` | Unit type |
| `Char` | `char` | Unicode character |
| `Byte` | `u8` | Single byte |
| `Seq<T>` | `Vec<T>` | Ordered sequence |
| `Map<K,V>` | `HashMap<K,V>` | Key-value mapping |
| `Set<T>` | `HashSet<T>` | Unordered collection |
| `Tuple` | `Vec<Value>` | Heterogeneous tuple |

### Dynamic Values

The `Value` enum provides dynamic typing for heterogeneous collections:

```rust
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Char(char),
    Nothing,
}
```

### LogosContains Trait

Unified membership checking across collection types:

```rust
pub trait LogosContains<T> {
    fn logos_contains(&self, value: &T) -> bool;
}
```

Implemented for `Vec<T>`, `HashSet<T>`, `HashMap<K,V>`, `String`, and `ORSet<T>`.

## CRDTs

Conflict-free Replicated Data Types for eventually consistent distributed state. CRDTs provide automatic conflict resolution—any two replicas can be merged to produce the same result regardless of order.

### The Merge Trait

All CRDTs implement the `Merge` trait:

```rust
pub trait Merge {
    fn merge(&mut self, other: &Self);
}
```

Merge operations are:
- **Commutative**: `a.merge(b) == b.merge(a)`
- **Associative**: `a.merge(b.merge(c)) == a.merge(b).merge(c)`
- **Idempotent**: `a.merge(a) == a`

### Simple CRDTs

#### GCounter

Grow-only counter. Each replica maintains its own count; the total is the sum across all replicas.

```rust
let mut counter = GCounter::with_replica_id(1);
counter.increment(5);
assert_eq!(counter.value(), 5);
```

#### PNCounter

Positive-negative counter supporting increment and decrement operations.

```rust
let mut counter = PNCounter::with_replica_id(1);
counter.increment(10);
counter.decrement(3);
assert_eq!(counter.value(), 7);
```

#### LWWRegister

Last-write-wins register. Concurrent writes resolved by timestamp—latest write wins.

```rust
let mut reg = LWWRegister::new(1, "initial", timestamp);
reg.set("updated", later_timestamp);
```

#### MVRegister

Multi-value register preserving all concurrent writes until explicitly resolved.

```rust
let mut reg = MVRegister::new(1, "value_a");
// Concurrent writes from different replicas are preserved
```

### Complex CRDTs

#### ORSet

Observed-remove set with configurable bias for concurrent add/remove conflicts:

```rust
// Default: AddWins (concurrent add beats remove)
let mut set: ORSet<String> = ORSet::new(1);
set.add("item".to_string());
set.remove(&"item".to_string());

// Alternative: RemoveWins
let mut set: ORSet<String, RemoveWins> = ORSet::new(1);
```

The `SetBias` trait controls resolution:
- `AddWins` (default): Concurrent add beats remove
- `RemoveWins`: Concurrent remove beats add

#### ORMap

Observed-remove map supporting nested CRDTs as values:

```rust
let mut map: ORMap<String, GCounter> = ORMap::new(1);
map.apply_entry("key".to_string(), |counter| counter.increment(1));
```

### Sequence CRDTs

#### RGA

Replicated Growable Array for ordered sequences with insert/delete operations.

#### YATA

Yet Another Text Algorithm—optimized for collaborative text editing.

## Causal Infrastructure

### ReplicaId

Unique identifier for a replica in a distributed system:

```rust
pub type ReplicaId = u64;

// Generate a unique ID (uses getrandom, WASM-compatible)
let id = generate_replica_id();
```

### Dot

Unique event identifier combining replica and sequence number:

```rust
pub struct Dot {
    pub replica: ReplicaId,
    pub counter: u64,
}
```

### VClock

Vector clock for tracking causal relationships between events:

```rust
let mut clock = VClock::new();
clock.increment(replica_id);

// Check causality
if clock_a.dominates(&clock_b) {
    // a happened after b
} else if clock_a.concurrent(&clock_b) {
    // concurrent events
}
```

### DotContext

Combines a VClock with a "cloud" of seen dots for efficient event tracking in delta-state CRDTs.

## Delta Synchronization

CRDTs implementing `DeltaCrdt` support efficient delta-state synchronization:

```rust
pub trait DeltaCrdt: Merge + Sized {
    type Delta: Serialize + DeserializeOwned + Clone + Send + 'static;

    fn delta_since(&self, version: &VClock) -> Option<Self::Delta>;
    fn apply_delta(&mut self, delta: &Self::Delta);
    fn version(&self) -> VClock;
}
```

Use `DeltaBuffer` to accumulate operations before syncing.

## Polymorphic Indexing

The `LogosIndex` and `LogosIndexMut` traits provide unified indexing:

```rust
pub trait LogosIndex<I> {
    type Output;
    fn logos_get(&self, index: I) -> Self::Output;
}

pub trait LogosIndexMut<I>: LogosIndex<I> {
    fn logos_set(&mut self, index: I, value: Self::Output);
}
```

### 1-Based Indexing

`Vec<T>` uses **1-based indexing** (LOGOS convention):

```rust
let v = vec![10, 20, 30];
assert_eq!(v.logos_get(1i64), 10);  // First element
assert_eq!(v.logos_get(2i64), 20);  // Second element
```

### Key-Based Access

`HashMap<K,V>` uses key-based indexing:

```rust
let mut map: HashMap<String, i64> = HashMap::new();
map.insert("key".to_string(), 42);
assert_eq!(map.logos_get("key"), 42);
```

## WASM Compatibility

This crate achieves WASM compatibility by:

1. **No SystemTime**: Timestamps are injected by callers
2. **No networking**: All IO is external
3. **WASM-compatible randomness**: Uses `getrandom` with the `js` feature for `wasm32-unknown-unknown`

The `Synced<T>` wrapper for networked synchronization lives in `logicaffeine_system`, which has the IO dependencies.

## License

BUSL-1.1
