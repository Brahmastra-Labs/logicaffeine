# logicaffeine-data

WASM-safe data structures and CRDTs for distributed systems.

Part of the [Logicaffeine](https://logicaffeine.com) project.

## The Lamport Invariant

This crate enforces a strict boundary: **no IO, no system time, no network access**. It compiles cleanly for both native and `wasm32-unknown-unknown` targets.

All timestamps must be injected by callers. This means:
- `LWWRegister` requires explicit timestamp parameters
- Replica IDs are generated using `getrandom` (works in WASM)
- The `Synced<T>` networking wrapper lives in `logicaffeine_system`, not here

## Features

- **WASM-compatible**: Compiles for native and WebAssembly targets
- **Pure data structures**: No tokio, no libp2p, no SystemTime dependencies
- **Serializable**: All types implement `serde::Serialize` and `Deserialize`
- **Delta sync**: Efficient incremental synchronization via `DeltaCrdt` trait

## CRDT Types

CRDTs (Conflict-free Replicated Data Types) provide automatic conflict resolution. Any two replicas can merge to produce the same result, regardless of message order.

### Counters

| Type | Description | Use Case |
|------|-------------|----------|
| `GCounter` | Grow-only counter | View counts, page hits |
| `PNCounter` | Positive-negative counter | Bidirectional counters (upvotes/downvotes) |

```rust
use logicaffeine_data::{GCounter, PNCounter, Merge};

let mut counter = GCounter::new();
counter.increment(5);
assert_eq!(counter.value(), 5);

let mut pn = PNCounter::new();
pn.increment(10);
pn.decrement(3);
assert_eq!(pn.value(), 7);
```

### Registers

| Type | Description | Use Case |
|------|-------------|----------|
| `LWWRegister<T>` | Last-write-wins | Single values where latest update should win |
| `MVRegister<T>` | Multi-value | Track conflicts for manual resolution |

```rust
use logicaffeine_data::{LWWRegister, MVRegister, Merge};

// LWW: Timestamp determines winner
let mut reg = LWWRegister::new("initial", 100);
reg.set("updated", 200);  // Higher timestamp wins

// MV: Preserves concurrent writes for conflict detection
let mut mv: MVRegister<String> = MVRegister::new(1);
mv.set("value".into());
if mv.has_conflict() {
    mv.resolve("resolved".into());
}
```

### Sets

| Type | Description | Use Case |
|------|-------------|----------|
| `ORSet<T, AddWins>` | Concurrent add beats remove | Collaborative collections (default) |
| `ORSet<T, RemoveWins>` | Concurrent remove beats add | Access revocation, cleanup operations |

```rust
use logicaffeine_data::{ORSet, AddWins, RemoveWins, Merge};

let mut set: ORSet<String, AddWins> = ORSet::new(1);
set.add("item".into());
assert!(set.contains(&"item".into()));

// With remove-wins bias
let mut strict: ORSet<String, RemoveWins> = ORSet::new(1);
```

### Maps

| Type | Description | Use Case |
|------|-------------|----------|
| `ORMap<K, V>` | Key-value map with nested CRDTs | Structured data, nested counters/sets |

```rust
use logicaffeine_data::{ORMap, PNCounter, Merge};

let mut scores: ORMap<String, PNCounter> = ORMap::new(1);
scores.get_or_insert("player1".into()).increment(100);
assert_eq!(scores.get(&"player1".into()).unwrap().value(), 100);
```

### Sequences

| Type | Description | Use Case |
|------|-------------|----------|
| `RGA` | Replicated Growable Array | Collaborative lists |
| `YATA` | Yet Another Text Algorithm | Collaborative text editing |

```rust
use logicaffeine_data::{RGA, YATA, Merge};

let mut list: RGA<String> = RGA::new(1);
list.append("first".into());
list.append("second".into());
assert_eq!(list.to_vec(), vec!["first", "second"]);

let mut text: YATA<char> = YATA::new(1);
text.append('H');
text.append('i');
```

## Causal Infrastructure

These types track causality and enable conflict detection.

| Type | Description |
|------|-------------|
| `VClock` | Vector clock for causal ordering |
| `Dot` | Unique event identifier (replica ID + sequence number) |
| `DotContext` | Combines clock + cloud for out-of-order message handling |
| `DeltaBuffer<D>` | Ring buffer for recent deltas (efficient sync) |

```rust
use logicaffeine_data::{VClock, Dot, DotContext};

let mut clock = VClock::new();
let seq = clock.increment(42);  // Returns sequence number

let dot = Dot::new(42, seq);

let mut ctx = DotContext::new();
let next_dot = ctx.next(42);  // Generate and track
assert!(ctx.has_seen(&next_dot));
```

## Runtime Types

Type aliases for LOGOS programs:

| LOGOS Type | Rust Type | Description |
|------------|-----------|-------------|
| `Nat` | `u64` | Natural numbers |
| `Int` | `i64` | Signed integers |
| `Real` | `f64` | Floating-point |
| `Text` | `String` | UTF-8 strings |
| `Bool` | `bool` | Boolean values |
| `Unit` | `()` | Unit type |
| `Char` | `char` | Unicode scalar |
| `Byte` | `u8` | Raw bytes |
| `Seq<T>` | `Vec<T>` | Ordered sequences |
| `Set<T>` | `HashSet<T>` | Unique elements |
| `Map<K,V>` | `HashMap<K,V>` | Key-value pairs |
| `Tuple` | `Vec<Value>` | Heterogeneous tuples |
| `Value` | enum | Dynamic type for mixed collections |

## Key Traits

### `Merge`

The core CRDT trait. Must satisfy:
- **Commutative**: `a.merge(b) == b.merge(a)`
- **Associative**: `a.merge(b.merge(c)) == a.merge(b).merge(c)`
- **Idempotent**: `a.merge(a) == a`

```rust
use logicaffeine_data::Merge;

// All CRDT types implement Merge
fn sync<T: Merge>(local: &mut T, remote: &T) {
    local.merge(remote);
}
```

### `DeltaCrdt`

For efficient incremental synchronization:

```rust
use logicaffeine_data::{DeltaCrdt, VClock};

// Extract changes since a known version
fn get_updates<T: DeltaCrdt>(crdt: &T, since: &VClock) -> Option<T::Delta> {
    crdt.delta_since(since)
}
```

### `LogosIndex` / `LogosIndexMut`

1-based indexing for natural language conventions:

```rust
use logicaffeine_data::{LogosIndex, LogosIndexMut};

let v = vec![10, 20, 30];
assert_eq!(v.logos_get(1i64), 10);  // Index 1 = first element
assert_eq!(v.logos_get(3i64), 30);  // Index 3 = third element
// Index 0 panics!
```

### `LogosContains`

Unified containment testing:

```rust
use logicaffeine_data::LogosContains;

let v = vec![1, 2, 3];
assert!(v.logos_contains(&2));

let s = String::from("hello");
assert!(s.logos_contains(&"ell"));  // Substring
assert!(s.logos_contains(&'o'));    // Character
```

## Usage Example

```rust
use logicaffeine_data::{ORSet, PNCounter, ORMap, Merge, AddWins};

// Create state on replica 1
let mut replica1: ORMap<String, PNCounter> = ORMap::new(1);
replica1.get_or_insert("score".into()).increment(100);

// Create state on replica 2
let mut replica2: ORMap<String, PNCounter> = ORMap::new(2);
replica2.get_or_insert("score".into()).increment(50);

// Merge - order doesn't matter, result is always the same
replica1.merge(&replica2);
assert_eq!(replica1.get(&"score".into()).unwrap().value(), 150);
```

## Important Constraints

1. **No IO** - Timestamps and network sync are the caller's responsibility
2. **1-based indexing** - `LogosIndex` panics on index 0 or out-of-bounds
3. **For networking** - Use `logicaffeine_system`'s `Synced<T>` wrapper

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.
