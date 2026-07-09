# Concurrency & distributed systems

LOGOS treats concurrency, message passing, replicated state, and networking as first-class language
features. They are backed by a deterministic, replayable runtime and run across native and browser
targets.

Source of truth: the statement set in
[`ast/stmt.rs`](../crates/logicaffeine_language/src/ast/stmt.rs), the runtime
([`logicaffeine_runtime`](../crates/logicaffeine_runtime/README.md)), the CRDTs
([`logicaffeine_data/src/crdt/`](../crates/logicaffeine_data/src/crdt/)), and networking
([`logicaffeine_system/src/`](../crates/logicaffeine_system/src/)). Tests live in
`phase46_agents` … `phase54_concurrency` and the `e2e_concurrency` suite.

## The runtime

[`logicaffeine_runtime`](../crates/logicaffeine_runtime/README.md) is a **deterministic** scheduler
— pure `std`, tokio-free, and WASM-safe (the **Lamport invariant** keeps the data layer IO-free).
Scheduling decisions come from a `Chooser` driven by a seed (`SchedSeed`) or a recorded trace
(`SchedTrace`), so a concurrent run is **reproducible and replayable** (`run_with_seed`,
`run_with_trace`). There are two drivers behind one model:

- **Cooperative M:1** (the WASM path) — single-threaded, deterministic.
- **Work-stealing M:N** (native, `cfg(not(wasm32))`) — genuine multicore.

The runtime is used by the interpreter/VM tiers; AOT-compiled binaries lower to host primitives
(tokio / rayon / libp2p via [`logicaffeine_system`](../crates/logicaffeine_system/README.md))
instead, and are never linked against the runtime.

## Structured concurrency

```logos
## Main
Attempt all of the following:      # async / I/O-bound — joined
    Show "fetch A".
    Show "fetch B".
Simultaneously:                    # CPU-bound — run in parallel
    Show "crunch X".
    Show "crunch Y".
```

(`Concurrent` and `Parallel` in `ast/stmt.rs`; `phase9_structured_concurrency`.)

## Tasks, channels & select

Green threads and Go-style channels (`Pipe`). A producer task feeds a consumer:

```logos
## To produce (ch: Int):
    Send 1 into ch.
    Send 2 into ch.

## Main
    Let jobs be a Pipe of Int.
    Launch a task to produce with jobs.
    Receive first from jobs.
    Receive second from jobs.
    Show first.
    Show second.
```

`Select` waits on whichever arm is ready first — a receive or a timeout:

```logos
Await the first of:
    Receive x from jobs:
        Show x.
    After 1 seconds:
        Show "timeout".
```

The full set: `LaunchTask` (`Launch a task to <fn> with <args>`), `LaunchTaskWithHandle`
(`Let h be Launch a task to <fn>`) and `StopTask` (`Stop h.`), `CreatePipe` (`Let ch be a Pipe of T`),
`SendPipe`/`ReceivePipe` and their `Try…` variants (`Try to send <v> into ch`), and `Select`. All are
wired through the interpreter, the VM (channel/spawn/select opcodes), the JIT, and AOT codegen.
(`interp_concurrency`, `vm_concurrency`, `phase54_concurrency`.)

## Agents (message passing)

```logos
Spawn a Worker called "w1".
Send Ping to "w1".
Await response from "w1" into reply.
```

Messages can be `compressed` (Lz4/Deflate/Zstd), `cached` (schema dictionary), or `unchecked`
(latency trade-off). (`Spawn`, `SendMessage`, `AwaitMessage`; `phase46_agents`.)

A `Send` can also choose a **wire layout** — a size↔speed↔resilience dial the sender picks for its
link, orthogonal to the modifiers above:

| Word | Layout | For |
|------|--------|-----|
| `fast` / `quickly` | fixed-width (memcpy) | latency-bound / fat links (LAN, datacenter, RDMA) |
| `compact` / `small` | varint (the default) | mobile / WAN / metered |
| `packed` | SIMD group-varint | the balanced middle |
| `smallest` / `best` | per-column compression menu | bandwidth-bound, CPU cheap |
| `redundant` / `tough` | Reed-Solomon FEC shards | lossy / one-way links (UDP, multicast, BLE, LoRa) — reconstruct from any *K* |

```logos
Send redundant Ping to "w1".   # survives shard loss with no retransmit
```

The `redundant` layout splits the message into Reed-Solomon shards
([`concurrency/fec.rs`](../crates/logicaffeine_compile/src/concurrency/fec.rs)). The layout menu
(`ast::SendLayout`) runs on the interpreter/VM concurrency path today; AOT codegen lowering is in
progress.

## CRDTs

[`logicaffeine_data/src/crdt/`](../crates/logicaffeine_data/src/crdt/) defines **eight** conflict-free
replicated types, all sharing a `Merge` trait and causal metadata (dots, vector clocks), with delta
support where applicable:

| CRDT | File | What it is |
|------|------|-----------|
| `GCounter` | `gcounter.rs` | Grow-only counter |
| `PNCounter` | `pncounter.rs` | Increment/decrement counter |
| `LWWRegister` | `lww.rs` | Last-write-wins register |
| `MVRegister` | `mvregister.rs` | Multi-value register (concurrent writes = conflict set) |
| `ORSet` | `orset.rs` | Observed-remove set (add-wins / remove-wins bias) |
| `ORMap` | `ormap.rs` | Observed-remove map |
| `RGA` | `sequence/rga.rs` | Replicated growable array (sequence) |
| `YATA` | `sequence/yata.rs` | Collaborative-text sequence |

### Declaring a shared type

A replicated type is declared `Shared`; each field names the CRDT it converges as. The compiler
emits a `Merge` impl for the whole struct (`phase49_crdt`, `e2e_crdt`):

```logos
## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.
    a name, which is LastWriteWins of Text.

## Main
    Let c be a new Counter.
```

The field-type keywords map onto the structs above:

| Declared as | CRDT |
|-------------|------|
| `ConvergentCount` | `GCounter` |
| `a Tally` | `PNCounter` |
| `LastWriteWins of T` | `LWWRegister<T>` |
| `a Divergent T` | `MVRegister<T>` |
| `a SharedSet of T` | `ORSet<T>` |
| `a SharedSequence of T` | `RGA` |

### Mutating and merging

Once declared, CRDT operations read naturally:

```logos
Increase local's points by 10.       # GCounter
Decrease game's score by 5.          # PNCounter
Append "Hello" to doc's lines.       # RGA sequence
Resolve page's title to "Final".     # MVRegister
Merge remote into local.
```

(`IncreaseCrdt`, `DecreaseCrdt`, `AppendToSequence`, `ResolveConflict`, `MergeCrdt`;
`phase49_crdt`, `e2e_crdt`.)

## Networking & persistence

[`logicaffeine_system`](../crates/logicaffeine_system/README.md) provides the transports, gated by
Cargo features (`relay`, `networking`, `persistence`, `concurrency`):

- **libp2p mesh** ([`network/`](../crates/logicaffeine_system/src/network/)) — TCP + QUIC, mDNS
  discovery, request/response, and **GossipSub** pub/sub for CRDT sync.
- **Thin WebSocket relay** ([`relay.rs`](../crates/logicaffeine_system/src/relay.rs) native +
  [`relay_browser.rs`](../crates/logicaffeine_system/src/relay_browser.rs) WASM, over the shared
  [`relay_proto`](../crates/logicaffeine_system/src/relay_proto.rs)) — bridges the browser to the mesh
  without libp2p.

The distributed verbs tie state to the network and disk:

```logos
Listen on "/ip4/127.0.0.1/tcp/8000".
Connect to "/ip4/127.0.0.1/tcp/8000".
Sync counter on "scores".            # subscribe to a GossipSub topic; auto-publish + auto-merge
Mount counter at "data/counter.journal".   # journal to disk (WAL + snapshots)
```

(`Listen`, `ConnectTo`, `Sync`, `Mount`; `phase48_network`, `phase51_mesh`, `phase52_sync`,
`phase53_persistence`.)

## Determinism in practice

Because scheduling is seed/trace driven, a concurrent program can be run identically on the
interpreter, the VM, and (for single-task replay) AOT — the differential tests assert the tiers agree
byte-for-byte. This is what makes "concurrent" and "reproducible" coexist.

## See also

- The execution tiers → [Execution & performance](execution-and-performance.md)
- The runtime, data, and system crates' own READMEs

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
