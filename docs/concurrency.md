# Concurrency & distributed systems

A concurrency bug that shows up once in a thousand runs is a bug you cannot fix, because you cannot
watch it happen twice. LOGOS treats concurrency, message passing, replicated state, and networking
as first-class language features, and backs them with a deterministic runtime: a seed or a recorded
trace fixes every scheduling choice, so a failing run replays exactly. They run across native and
browser targets.

After this guide you can launch tasks and join them, pass messages through pipes and agents,
declare replicated types that merge without conflicts, sync them over the network, and replay a
failing concurrent run.

## The runtime

[`logicaffeine_runtime`](../crates/logicaffeine_runtime/README.md) is a **deterministic** scheduler:
pure `std`, tokio-free, and WASM-safe (the **Lamport invariant** keeps the data layer IO-free).
Scheduling decisions come from a `Chooser` driven by a seed (`SchedSeed`) or a recorded trace
(`SchedTrace`), so a concurrent run is **reproducible and replayable** (`run_with_seed`,
`run_with_trace`). There are two drivers behind one model:

- **Cooperative M:1** (the WASM path): single-threaded, deterministic.
- **Work-stealing M:N** (native, `cfg(not(wasm32))`): genuine multicore.

The runtime is used by the interpreter/VM tiers; an AOT-compiled binary is never linked against it.
A default `largo build` lowers tasks, pipes and select to host primitives through
[`logicaffeine_system`](../crates/logicaffeine_system/README.md) (`tokio::spawn`, tokio channels,
`tokio::select!`, rayon or OS threads for `Simultaneously:`, libp2p for the mesh) and runs them
free. A deterministic build, through the library entry points `compile_to_rust_deterministic` and
`compile_program_full_deterministic`, instead runs every task, pipe, timer and select on
[`logicaffeine_system::replay`](../crates/logicaffeine_system/src/replay.rs). That is the runtime's
scheduler ported step for step onto Rust futures, drawing each choice from the same seeded chooser,
so the binary run at `LOGOS_SEED=s` prints what the interpreter prints at seed `s`.

The C family carries a port too. Host C, RISC-V, the bytecode walker and the CUDA tier carry the
runtime's scheduler with them, ported to C (`codegen_c/sched_core.c`) one operation at a time, so a
compiled binary interleaves its tasks exactly as the interpreter does:

- a launched task runs before its parent resumes;
- every pipe operation is a scheduling point;
- a default pipe holds 32 values;
- a task left waiting forever is a deadlock.

Each binary draws its choices from the seed in `LOGOS_SEED`, read the same way the interpreter
reads it.
Host C and RISC-V run each task as a coroutine on a stack of its own; the bytecode walker and the
GPU run it as a step function over a heap frame, the form a GPU thread can run. A GPU thread cannot
unwind, so on the GPU a failure inside a runtime helper (a zero divisor, an index out of range)
stops the whole run; a failing `Require` fails only its task, as everywhere else.

The direct WASM backend carries no scheduler: a launched task runs to completion at its launch, a pipe
is a FIFO queue, `Sleep` advances nothing and `Close` has no observable effect. That is the
scheduler's order for a program whose tasks never block.

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

The two blocks are the `Concurrent` and `Parallel` statements in `ast/stmt.rs`.

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

A task gets a copy of each value it is launched with, as a pipe's receiver gets a copy of what was sent:
a list the task changes is its own. A task may run on a worker thread of its own, so a value that
cannot leave the program's thread — a function, or a real number a function of the program defines —
stops the program where it is launched with it or sent.

`Select` waits on whichever arm is ready first, a receive or a timeout:

```logos
Await the first of:
    Receive x from jobs:
        Show x.
    After 1 seconds:
        Show "timeout".
```

A closed pipe keeps what was sent to it: its buffered values are still received, in order, and a
receive after the last one yields `nothing`:

```logos
## Main
Let ch be a Pipe of Int.
Send 5 into ch.
Close ch.
Receive a from ch.
Receive b from ch.
Show a.
Show b.
```

```output
5
nothing
```

A task handle is joined with `Await h.` (or `Wait for h.`), which parks until the task finishes:

```logos
## To slow:
    Sleep 4.
    Show "slow done".

## Main
Let h be Launch a task to slow.
Show "awaiting".
Await h.
Show "joined".
```

```output
awaiting
slow done
joined
```

The full set: `LaunchTask` (`Launch a task to <fn> with <args>`), `LaunchTaskWithHandle`
(`Let h be Launch a task to <fn>`), `StopTask` (`Stop h.`), `AwaitTask` (`Await h.` / `Wait for h.`,
optionally `… into r`), `CreatePipe` (`Let ch be a Pipe of T`), `SendPipe`/`ReceivePipe` and their
`Try…` variants (`Try to send <v> into ch`), `CloseChannel` (`Close ch.` / `Close the pipe ch.`), and
`Select`. The scheduler does not carry a task's return value back, so `Await h into r.` binds `r` to
`nothing`. Every one runs on the tree-walker and the VM (channel, spawn, await and select opcodes),
on the C family, and in a deterministic build.

## Agents (message passing)

```logos
Spawn a Worker called "w1".
Send Ping to "w1".
Await response from "w1" into reply.
```

Messages can be `compressed` (Lz4/Deflate/Zstd), `cached` (schema dictionary), or `unchecked`
(latency trade-off). `shared` ships a struct as a type id both ends derive from the same program,
`indexed` (`addressable`) lays a record list out for O(1) random field reads, `deduped` sends a
repeated subtree once with back-references, and `computed` ships a pure one-argument function as a
sandboxed computation. `Send unreliably <msg> to <peer>` marks the message a best-effort, unordered
datagram (`SendMessage.unreliable`); every tier delivers it as a reliable send. The three verbs are
the `Spawn`, `SendMessage` and `AwaitMessage` statements.

A `Send` can also choose a **wire layout**: a size↔speed↔resilience dial the sender picks for its
link, orthogonal to the modifiers above.

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
([`concurrency/fec.rs`](../crates/logicaffeine_compile/src/concurrency/fec.rs)). The modifiers and
the layout menu (`ast::SendLayout`) are applied by the tree-walker's relay send; the VM, AOT-Rust and
C lower `Send` to a plain message send and ignore them. The wire is self-describing, so a peer
decodes either form.

## CRDTs

[`logicaffeine_data/src/crdt/`](../crates/logicaffeine_data/src/crdt/) defines the conflict-free
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
| `LatestWrite` | `latest.rs` | Last-writer register stamped with its map's Lamport time (a shared map's values) |
| `RGA` | `sequence/rga.rs` | Replicated growable array (sequence) |
| `YATA` | `sequence/yata.rs` | Collaborative-text sequence |

### Declaring a shared type

A replicated type is declared `Shared`; each field names the CRDT it converges as. The compiler
emits a `Merge` impl for the whole struct:

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
| `a SharedMap from K to V` | `ORMap<K, LatestWrite<V>>` |

A shared map's keys are an observed-remove set (a concurrent write beats a remove) and each value a
last-writer register ordered by the map's own Lamport time: a write that has seen another write beats
it, and two concurrent writes resolve to the larger value. With no wall clock and no replica id, two
replicas that have seen the same writes hold the same map on the tree-walker, the VM and the compiled
binary; a merged map renders and iterates by key:

```logos
## Definition
A Warehouse is Shared and has:
    stock: a SharedMap from Text to Int.

## Main
Let mutable a be a new Warehouse.
Set a's stock["wood"] to 50.
Let mutable b be a new Warehouse.
Set b's stock["wood"] to 99.
Merge b into a.
Set a's stock["wood"] to 1.
Merge a into b.
Show a's stock["wood"].
Show b's stock["wood"].
```

```output
1
1
```

### Mutating and merging

Once declared, CRDT operations read naturally:

```logos
Increase local's points by 10.       # GCounter
Decrease game's score by 5.          # PNCounter
Append "Hello" to doc's lines.       # RGA sequence
Resolve page's title to "Final".     # MVRegister
Merge remote into local.
```

These are the `IncreaseCrdt`, `DecreaseCrdt`, `AppendToSequence`, `ResolveConflict` and
`MergeCrdt` statements.

`Converge remote into local.` is the idempotent lattice join (`JoinCrdt`). It differs from `Merge` on
one kind of field: a plain `Int` field takes the maximum where `Merge` adds, so a redelivered state
applied twice equals applying it once. Counter fields take the per-replica maximum under both, and
every other field folds as under `Merge`:

```logos
## Definition
A Mixed is Shared and has:
    points: ConvergentCount.
    tag: Int.

## Main
    Let mutable a be a new Mixed with tag 7.
    Increase a's points by 5.
    Let mutable b be a new Mixed with tag 2.
    Increase b's points by 3.
    Converge b into a.
    Show a's points.
    Show a's tag.
```

```output
8
7
```

With `Merge b into a.` in its place the program shows `8` and `9`.

## Networking & persistence

[`logicaffeine_system`](../crates/logicaffeine_system/README.md) provides the transports, gated by
Cargo features (`relay`, `networking`, `persistence`, `concurrency`, `http` for the `Serve` server,
the Linux-only `io-uring` engine, and `full` for every native capability but `io-uring`):

- **libp2p mesh** ([`network/`](../crates/logicaffeine_system/src/network/)): TCP + QUIC, mDNS
  discovery, request/response, and **GossipSub** pub/sub for CRDT sync.
- **Thin WebSocket relay** ([`relay.rs`](../crates/logicaffeine_system/src/relay.rs) native +
  [`relay_browser.rs`](../crates/logicaffeine_system/src/relay_browser.rs) WASM, over the shared
  [`relay_proto`](../crates/logicaffeine_system/src/relay_proto.rs)): bridges the browser to the mesh
  without libp2p. `relay::RelayHub` is the same pub/sub with no listener of its own: `largo serve` and
  a compiled site whose pages use the relay answer it at `/_relay`, the path a page dials for
  `Publish`, `When a message arrives` and a `shared on` store.

The distributed verbs tie state to the network and disk:

```logos
Listen on "/ip4/127.0.0.1/tcp/8000".
Connect to "/ip4/127.0.0.1/tcp/8000".
Sync counter on "scores".            # subscribe to a GossipSub topic; auto-publish + auto-merge
Mount counter at "data/counter.journal".   # journal to disk (WAL + snapshots)
```

These are the `Listen`, `ConnectTo`, `Sync` and `Mount` statements.

## Determinism in practice

Because scheduling is seed/trace driven, a concurrent program runs identically at a given seed on the
interpreter, the VM, the C-family tiers (host C, RISC-V, the bytecode walker, CUDA) and a
deterministic AOT build. `c_family_scheduler_conformance_lock` and `e2e_deterministic_replay` assert
that every tier prints what the tree-walker prints, and that the seed-sensitive programs show more
than one outcome across seeds, so the seed reaches the binary. A default `largo build` runs free on
tokio and is outside this guarantee. This is what makes "concurrent" and "reproducible" coexist.

A failing `largo run --interpret` is recorded as a `.replay` bundle under `target/replays/` (the
seed, the scheduling trace, the embedded source and the failure), and `largo replay <file>`
re-executes it.

## See also

- The execution tiers → [Execution & performance](execution-and-performance.md)
- The runtime, data, and system crates' own READMEs

## Where this is tested

The statement set is in [`ast/stmt.rs`](../crates/logicaffeine_language/src/ast/stmt.rs), the
runtime in [`logicaffeine_runtime`](../crates/logicaffeine_runtime/README.md), the CRDTs in
[`logicaffeine_data/src/crdt/`](../crates/logicaffeine_data/src/crdt/), and networking in
[`logicaffeine_system/src/`](../crates/logicaffeine_system/src/). Every `logos` block above is
compiled by `doc_examples`, and every output shown is what the tree-walker prints.

- Structured concurrency: `phase9_structured_concurrency`.
- Tasks, pipes and select: `interp_concurrency`, `vm_concurrency`, `phase54_concurrency`,
  `op_backend_matrix_lock`, the `e2e_concurrency` suite, and `c_family_scheduler_conformance_lock`
  (which also pins `Await h into r.` binding `nothing`).
- Agents: `phase46_agents`; unreliable sends: `send_reliability`.
- CRDTs: `phase49_crdt` and `e2e_crdt` (declaring, mutating and merging), `shared_map_all_tiers`
  (the shared map on every tier), `converge_crdt_lock` (`Converge`), and `rust_crdt_lock`
  (compiled Rust's `Merge` and `Converge` against the VM).
- Networking and persistence: `phase48_network`, `phase51_mesh`, `phase52_sync`,
  `phase53_persistence`.
- Determinism: `replay_executor_port_lock` holds the deterministic build's executor to the
  runtime's scheduler, `e2e_deterministic_replay` holds those binaries to the tree-walker and the VM,
  and `replay_lock` holds `.replay` capture and `largo replay`.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
