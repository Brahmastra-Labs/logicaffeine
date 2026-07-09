# logicaffeine-data

WASM-safe runtime values and conflict-free replicated data types (CRDTs): the
dynamic value universe LOGOS programs manipulate, the specialized integer
collections the code generator emits for proven-safe hot paths, the 1-based
indexing traits that make values subscriptable, and the eight CRDTs that converge
those values across replicas — all with no path to system IO.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 1 — depends on
logicaffeine_base. **Lamport invariant**: no IO dependencies, so these structures
stay WASM-safe and clock-agnostic.

## Role in the workspace

This is the value layer shared by the rest of the workspace. `logicaffeine_compile`
and `logicaffeine_system` build on its runtime types and CRDTs; the web app links
it for `wasm32-unknown-unknown`. Everything compiles identically for native and
WASM because nothing here touches the clock, the network, or the filesystem. The
networking wrappers that *do* (e.g. `Synced<T>`) live one tier up in
`logicaffeine_system`. See [concurrency](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/concurrency.md) for how
replicated state flows through the runtime.

The Lamport invariant is enforced at the dependency boundary: no tokio, no libp2p,
no `std::time::SystemTime` in CRDT logic. Timestamps are injected by callers —
`LWWRegister::new`/`set` take an explicit `u64`. The single clock touch is
`generate_replica_id`: native XORs `SystemTime::now()` with `getrandom` bytes;
wasm32 uses `getrandom` alone (Web Crypto via the `js` feature), keeping the WASM
build pure.

## CRDTs

Eight CRDT types. Every one converges through the `Merge` trait (commutative,
associative, idempotent) and derives `serde::Serialize`/`Deserialize`.
`ReplicaId = u64`.

| Type | File | Description |
|------|------|-------------|
| `GCounter` | `crdt/gcounter.rs` | Grow-only counter; per-replica counts, value is their sum |
| `PNCounter` | `crdt/pncounter.rs` | Increment/decrement counter built from two G-Counters (P and N) |
| `LWWRegister<T>` | `crdt/lww.rs` | Last-write-wins register; highest caller-supplied timestamp wins on merge |
| `MVRegister<T>` | `crdt/mvregister.rs` | Multi-value register; preserves all concurrent writes until resolved |
| `ORSet<T, B = AddWins>` | `crdt/orset.rs` | Observed-remove set; `B: SetBias` = `AddWins` (default) or `RemoveWins` |
| `ORMap<K, V: Merge>` | `crdt/ormap.rs` | Observed-remove map; add-wins keys, recursively merged nested-CRDT values |
| `RGA<T>` | `crdt/sequence/rga.rs` | Replicated Growable Array; sequence CRDT for collaborative lists |
| `YATA<T>` | `crdt/sequence/yata.rs` | Origin-left/right sequence CRDT optimized for collaborative text |

## Public API

**`Value` and runtime types** (`types`) — `Value` is the dynamic enum
(`Int`/`Float`/`Bool`/`Text`/`Char`/`Nothing`) for heterogeneous tuples, with
`Add`/`Sub`/`Mul`/`Div` (numeric promotion, text concat). Aliases: `Nat=u64`,
`Int=i64`, `Real=f64`, `Text=String`, `Bool=bool`, `Char=char`, `Byte=u8`,
`Unit=()`, plus `LogosRational`. Collections use reference semantics:
`Seq<T> = LogosSeq<T>` (`Rc<RefCell<Vec<T>>>`) and `Map<K,V> = LogosMap<K,V>`
(`Rc<RefCell<FxHashMap>>`); `.deep_clone()` gives an independent copy. `Set<T>`
is a value-semantics `FxHashSet`. The code generator substitutes specialized
value-semantics integer collections — `LogosI64Map`/`LogosI64Set`,
`LogosI32Map`/`LogosI32Set`, `LogosDenseI64Map`/`LogosDenseI64Set`,
`LogosDenseI64MapNoPresence`, and the `LogosDivU64` magic-divisor — wherever
bounds/range analysis proves the swap invisible. `LogosContains<T>` unifies
membership across all of these plus `Vec`/`[T]`/`String`/`ORSet`.

**`Merge` trait** — `fn merge(&mut self, other: &Self)`, the convergence contract
(commutative, associative, idempotent).

**Delta support** — `DeltaCrdt: Merge` exposes `delta_since(&VClock)` /
`apply_delta` / `version`, implemented by `PNCounter`, `RGA`, and `YATA`; several
CRDTs additionally carry their own `*Delta` payload structs. `DeltaBuffer<D>`
retains recent deltas in a ring buffer for late joiners.

**Causal metadata** (`crdt::causal`) — `Dot` (replica + counter), `VClock`
(`dominates`/`concurrent`/`merge_vclock`), and `DotContext` (clock plus an
out-of-order dot cloud) track happens-before across replicas.

**Indexing** (`indexing`) — `LogosIndex`/`LogosIndexMut` use 1-based indices to
match natural language; `LogosGetChar` returns a `char` without allocating. They
cover `Vec`, `[T]`, `&mut [T]`, `String`, `LogosSeq`, `LogosMap`, and `FxHashMap`.

**Shared wire codec** (`wire`) — the byte format of the peer/transport codec,
factored out so both the interpreter's `RuntimeValue` and AOT-generated types
encode through one definition (`WireEncode`/`WireDecode` over the `T_INT`/`T_TEXT`/
`T_LIST`/`T_INDUCTIVE` tagged-varint form). Byte-identical across value models —
what lets a compile-once native partial evaluator receive a program as data.

**Cross-tier arithmetic and formatting** — `ops` is the exact numeric-comparison
layer the code generator emits (`logos_cmp_i64_f64`/`logos_i64_eq_f64`/
`logos_approx_eq`/`logos_truthy`), so a statically-mixed `Int`/`Float` compare is
exact — never a lossy `as f64` cast that would call `9007199254740993` equal to
`9007199254740992.0`. `fmt` is the single float-display authority: every tier
(tree-walker, VM, AOT binary, direct-WASM host) renders an `f64` through it, so
the same program prints the same decimal string however it was run.

```rust
use logicaffeine_data::{ORMap, PNCounter, Merge};

let mut a: ORMap<String, PNCounter> = ORMap::new(1);
a.get_or_insert("score".into()).increment(100);

let mut b: ORMap<String, PNCounter> = ORMap::new(2);
b.get_or_insert("score".into()).increment(50);

a.merge(&b);
b.merge(&a);
// Both replicas converge to the same state regardless of merge order.
```

## Dependencies

- **Internal**: `logicaffeine-base`.
- **External**: `rustc-hash` (FxHashMap/FxHashSet), `serde` (derive),
  `getrandom` (replica-id entropy; `js` feature on wasm32). Dev-only: `bincode`.

No tokio, no libp2p, no `SystemTime` — the Lamport invariant is part of the
dependency graph, not just convention. The crate has no Cargo features and no
build script.

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
