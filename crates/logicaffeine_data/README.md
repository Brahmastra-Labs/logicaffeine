# logicaffeine-data

The values LOGOS programs compute with and the CRDTs that replicate them: the dynamic `Value` universe, the collection carriers compiled programs use, 1-based indexing, the shared wire and float-display formats, and conflict-free replicated data types. Two replicas that each bump a shared counter offline agree on the total the moment they merge, with no server deciding who won, because every CRDT here merges commutatively, associatively and idempotently. It never touches the network or the filesystem, and merge logic never reads a clock.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 1. The same code builds for native targets and `wasm32-unknown-unknown`, differing only in how a replica id is seeded.

## Role in the workspace

This is the value layer shared by the rest of the workspace. `logicaffeine_compile` and `logicaffeine_system` build on its runtime types and CRDTs, and the web app links it for `wasm32-unknown-unknown`. The networking wrappers that do touch the outside world (`Synced<T>`, for one) live in `logicaffeine_system`. See [concurrency](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/concurrency.md) for how replicated state flows through the runtime.

The no-I/O rule is enforced at the dependency boundary: there is no tokio, no libp2p and no filesystem or network crate, and CRDT merge logic never reads a clock. Callers inject timestamps (`LWWRegister::new`/`set` take an explicit `u64`), and a `crdt::LatestWrite` is stamped with Lamport time by the map that holds it. The one clock read in the crate is `generate_replica_id`: native builds XOR `SystemTime::now()` with `getrandom` bytes, and `wasm32` uses `getrandom` alone (Web Crypto via the `js` feature).

## Example

Two replicas update the same key concurrently, then converge:

```rust
use logicaffeine_data::{Merge, ORMap, PNCounter};

let mut a: ORMap<String, PNCounter> = ORMap::new(1);
let mut b: ORMap<String, PNCounter> = ORMap::new(2);

a.get_or_insert("score".to_string()).increment(10);
b.get_or_insert("score".to_string()).increment(5);

a.merge(&b);
b.merge(&a);
assert_eq!(a.get(&"score".to_string()).unwrap().value(), 15);
assert_eq!(b.get(&"score".to_string()).unwrap().value(), 15);
```

## CRDTs

Every CRDT converges through the `Merge` trait (commutative, associative, idempotent) and derives `serde::Serialize`/`Deserialize`. `ReplicaId = u64`.

| Type | File | Description |
|------|------|-------------|
| `GCounter` | `crdt/gcounter.rs` | Grow-only counter; per-replica counts, value is their sum |
| `PNCounter` | `crdt/pncounter.rs` | Increment/decrement counter built from two grow-only counters (P and N) |
| `LWWRegister<T>` | `crdt/lww.rs` | Last-write-wins register; the highest caller-supplied timestamp wins on merge |
| `MVRegister<T>` | `crdt/mvregister.rs` | Multi-value register; preserves all concurrent writes until resolved |
| `ORSet<T, B = AddWins>` | `crdt/orset.rs` | Observed-remove set; `B: SetBias` is `AddWins` (default) or `RemoveWins` |
| `ORMap<K, V: Merge>` | `crdt/ormap.rs` | Observed-remove map; add-wins keys, recursively merged nested-CRDT values |
| `RGA<T>` | `crdt/sequence/rga.rs` | Replicated Growable Array; sequence CRDT for collaborative lists |
| `YATA<T>` | `crdt/sequence/yata.rs` | Origin-left/right sequence CRDT optimized for collaborative text |
| `crdt::LatestWrite<T>` | `crdt/latest.rs` | Last-writer register stamped by its holding map with Lamport time (no caller timestamp, no wall clock); merge is a join over `(stamp, value)`. The value cell of a LOGOS `SharedMap` (`ORMap<K, LatestWrite<V>>`) |

## Public API

**`Value` and runtime types** (`types`): `Value` is the dynamic enum (`Int`/`Float`/`Bool`/`Text`/`Char`/`Seq`/`Boxed`/`Nothing`) for heterogeneous tuples, where `Seq` carries a collection element and `Boxed` a program-declared record or variant (`Boxable`, `LogosBoxed`), with `Add`/`Sub`/`Mul`/`Div` (numeric promotion, text concatenation). Aliases: `Nat=u64`, `Int=i64`, `Real=f64`, `Text=String`, `Bool=bool`, `Char=char`, `Byte=u8`, `Unit=()`, plus the compiled execution tiers' carriers for the base numeric and value types: `LogosInt`, `LogosRational`, `LogosDecimal`, `LogosComplex`, `LogosComplexF`, `LogosModular`, `LogosQuantity`, `LogosMoney` (with `set_rate`/`set_rates`/`to_currency`), `LogosUuid`, and `LogosTuple`.

Collections use reference semantics: `Seq<T> = LogosSeq<T>` (`Rc<RefCell<Vec<T>>>`) and `Map<K,V> = LogosMap<K,V>` (`Rc<RefCell<FxIndexMap>>`, insertion-ordered), with `.deep_clone()` for an independent copy. `Set<T>` is a value-semantics `FxIndexSet`, iterating in first-insertion order like the tree-walker's sets. Wherever bounds and range analysis prove the swap invisible, the code generator substitutes specialized value-semantics integer collections: `LogosI64Map`/`LogosI64Set`, `LogosI32Map`/`LogosI32Set`, `LogosDenseI64Map`/`LogosDenseI64Set`, `LogosDenseI64MapNoPresence`, and the `LogosDivU64` magic-number divisor. `LogosContains<T>` unifies membership across all of these plus `Vec`/`[T]`/`String`/`ORSet`.

**The exact extension in compiled code** (`exact`): `LogosValue` is the value a compiled program holds wherever the exact extension reaches (the infinitesimal, the hyperreals and infinite integers it generates, real algebraic numbers, computable reals, ordinals, residue-dependent and provisional values, and every standard value that meets them), kept in its canonical form so it computes and prints what the interpreter does. It implements `logicaffeine_base::tower::{TowerValue, ValueAlgebra}`, so the interpreter's own glue and supertask algorithms run on it, and `logos_extension(ExtensionBuiltin, args)` calls the extension's builtins through the same dispatcher the interpreter uses. A kind outside the tower (a date, a span) rides inside it through `OtherValue`; `failure::answered` stops a program with an operation's refusal.

**A value or `nothing`** (`opt`): `LogosOpt<T>` is what a compiled program keeps in a place the VM can store `nothing` in (a binding, an item, a field, a parameter, a result), the value or `nothing`, with `is_nothing()` and the reads that stop the program in the VM's words when `nothing` is where a value is needed.

**Temporal values** (`temporal`): `LogosDate` (days since 1970-01-01), `LogosMoment` (nanoseconds since the epoch), `LogosTime` (nanoseconds since midnight) and `LogosSpan` (whole months and days, never converted into each other), with the calendar arithmetic and the display every tier shares from `logicaffeine_base::temporal`, and each an `OtherValue` a `LogosValue` holds, so a date meets the exact extension's values as it does in the interpreter. Nothing here reads a clock: `logicaffeine_system::temporal::current_moment` does.

**The C ABI** (`abi`, with the `c-abi` feature): the `logos_rt_value_*` functions a C program the exact extension reaches calls for every value it holds, so both C backends compute with `LogosValue` itself. A value crosses as a handle only `logos_rt_value_drop` frees; an operation borrows its operands and answers a fresh handle (`push`, `set_item` and `pop` change the list their first handle holds); an operation with no answer returns a null handle or a status of 0 and keeps the interpreter's sentence for `logos_rt_value_error`, a caught panic included, so nothing unwinds into C. `Counting` is the allocator a static library built over this module installs, whose total `logos_rt_value_heap_bytes` reports to the program's collector. `logicaffeine_compile::codegen_c::value_runtime` builds that library.

**`Merge` trait**: `fn merge(&mut self, other: &Self)`, the convergence contract (commutative, associative, idempotent).

**Delta support**: `DeltaCrdt: Merge` exposes `delta_since(&VClock)` / `apply_delta` / `version`, implemented by `PNCounter`, `RGA` and `YATA`; several CRDTs also carry their own `*Delta` payload structs. `DeltaBuffer<D>` retains recent deltas in a ring buffer for late joiners.

**Causal metadata** (`crdt::causal`): `Dot` (replica + counter), `VClock` (`dominates`/`concurrent`/`merge_vclock`), and `DotContext` (a clock plus an out-of-order dot cloud) track happens-before across replicas.

**Indexing** (`indexing`): `LogosIndex`/`LogosIndexMut` use 1-based indices to match natural language, and `LogosGetChar` returns a `char` without allocating. They cover `Vec`, `[T]`, `&mut [T]`, `String`, `LogosSeq`, `LogosTuple`, `LogosMap`, and `FxHashMap`.

**Shared wire codec** (`wire`): the byte format of the peer/transport codec, factored out so both the interpreter's `RuntimeValue` and AOT-generated types encode through one definition (`WireEncode`/`WireDecode` over the `T_INT`/`T_TEXT`/`T_LIST`/`T_INDUCTIVE` tagged-varint form). The bytes are identical across both value models, which is what lets a compile-once native partial evaluator receive a program as data.

**Cross-tier arithmetic and formatting**: `ops` is the exact numeric-comparison layer the code generator emits (`logos_cmp_i64_f64`/`logos_i64_eq_f64`/`logos_approx_eq`/`logos_truthy`), so a statically mixed `Int`/`Float` compare is exact rather than a lossy `as f64` cast that would call `9007199254740993` equal to `9007199254740992.0`. It also holds the exact `Int` arithmetic generated code calls:

- `logos_{add,sub,mul,div,rem,floordiv,pow}_exact` spill to `LogosInt` on overflow, the compiled mirror of the interpreter's big-integer promotion;
- `logos_{add,sub,mul,div,rem}_i64` raise the canonical overflow error on a hot path that must stay machine-width;
- `logos_{div,rem}_i128`/`logos_narrow_i128` evaluate a width-bounded chain in `i128` and narrow once.

`fmt` renders every `f64` for every execution tier (tree-walker, VM, AOT binary, direct-WASM host), so the same program prints the same decimal string however it was run.

**Backpressure** (`backpressure`): `BoundedRing<T>`, a fixed-capacity FIFO for a live-media outbound queue. A push into a full ring evicts the *oldest* item and returns it (freshest data wins), and every eviction is counted in `dropped`.

**Contract reasons** (`failure`): the report a broken contract gives on the compiled execution tiers (`Require that c, otherwise r.`). `ContractReason`/`contract_reason` turn the value after `otherwise` into its report: a Text reason is the report itself, and any other type yields the one refusal sentence every engine uses, naming the LOGOS type it got. A type with no implementation does not compile, so no tier reports in different words.

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `c-abi` | off | The `abi` module: the `logos_rt_value_*` C ABI over `LogosValue` and the `Counting` allocator, which the static library a C program links is built with. |

## Dependencies

- **Internal**: `logicaffeine-base`.
- **External**: `rustc-hash` (FxHashMap/FxHashSet), `indexmap` (the insertion-ordered `FxIndexMap`/`FxIndexSet` behind `Map` and `Set`), `serde` (derive), `getrandom` (replica-id entropy; `js` feature on wasm32). Dev-only: `bincode`.

The crate has no build script.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_data/CHANGELOG.md)
