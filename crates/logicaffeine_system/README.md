# logicaffeine-system

Platform IO and system services for LOGOS.

This crate provides platform-agnostic IO operations, persistence, networking, and concurrency primitives with feature-gated heavy dependencies. The core is lean by default—only enable what you need.

## Design Philosophy

- **Lean by default**: No network, no persistence, no parallelism in the default build
- **Feature-gated capabilities**: Heavy dependencies (libp2p, rayon, memmap2) are opt-in
- **Dual platform support**: Native Rust and WASM with platform-specific implementations
- **CRDT-first persistence**: Journal-based storage designed for conflict-free data types

## Feature Flags

| Feature | Dependencies | Description |
|---------|--------------|-------------|
| (default) | — | Lean core with basic IO only |
| `persistence` | memmap2, sha2 | File I/O, VFS abstraction, journal-based storage |
| `networking` | libp2p, futures | P2P networking via libp2p with mDNS discovery |
| `concurrency` | rayon, bumpalo | Parallel computation, channels, zone-based memory |
| `full` | all above | All features enabled |
| `distributed` | networking + persistence | Combined features for `Distributed<T>` |

## Module Overview

### Core (Always Available)

| Module | Description |
|--------|-------------|
| `io` | Console I/O: `show()`, `println()`, `read_line()`, `Showable` trait |
| `fmt` | String formatting utilities |

### Native-Only (Not on WASM)

| Module | Description |
|--------|-------------|
| `time` | Timestamps and delays: `now()`, `sleep()` |
| `env` | Environment variables and args: `get()`, `args()` |
| `random` | Random number generation: `randomInt()`, `randomFloat()` |

### Feature-Gated

| Module | Feature | Description |
|--------|---------|-------------|
| `file` | `persistence` | Simple synchronous file I/O |
| `fs` | `persistence` | Async VFS abstraction with atomic writes |
| `storage` | `persistence` | `Persistent<T>` - journal-based CRDT storage |
| `network` | `networking` | P2P mesh networking with GossipSub |
| `crdt` | `networking` | `Synced<T>` - auto-replicated CRDT wrapper |
| `concurrency` | `concurrency` | Task spawning, channels, cooperative yielding |
| `memory` | `concurrency` | Zone-based memory allocation |
| `distributed` | `distributed` | `Distributed<T>` - persistent + networked CRDT |

## Public API

### I/O (`io`)

```rust
use logicaffeine_system::{show, println, read_line, Showable};

show(&42);              // Prints: 42
show(&"hello");         // Prints: hello (no quotes)
show(&vec![1, 2, 3]);   // Prints: [1, 2, 3]

println("Enter name:");
let name = read_line();
```

The `Showable` trait provides natural formatting—primitives display without decoration, collections with brackets, and CRDTs show their logical values.

### Time (`time`) — Native Only

```rust
use logicaffeine_system::time;

let start = time::now();       // Milliseconds since Unix epoch
time::sleep(1000);             // Block for 1 second
let elapsed = time::now() - start;
```

### Environment (`env`) — Native Only

```rust
use logicaffeine_system::env;

if let Some(home) = env::get("HOME".to_string()) {
    println!("Home: {}", home);
}

for arg in env::args() {
    println!("Arg: {}", arg);
}
```

### Random (`random`) — Native Only

```rust
use logicaffeine_system::random;

let dice = random::randomInt(1, 6);    // Inclusive range [1, 6]
let chance = random::randomFloat();    // Range [0.0, 1.0)
```

### File I/O (`file`) — Requires `persistence`

```rust
use logicaffeine_system::file;

file::write("data.txt".to_string(), "Hello!".to_string())?;
let content = file::read("data.txt".to_string())?;
```

### Virtual File System (`fs`) — Requires `persistence`

Platform-agnostic async file operations:

```rust
use logicaffeine_system::fs::{Vfs, NativeVfs};
use std::sync::Arc;

let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(NativeVfs::new("/data"));

vfs.write("config.json", b"{}").await?;
let data = vfs.read("config.json").await?;
vfs.append("log.txt", b"entry\n").await?;  // Atomic append

let entries = vfs.list_dir("").await?;
```

**Platform implementations:**
- `NativeVfs`: Native filesystem via tokio with atomic writes
- `OpfsVfs`: Browser Origin Private File System (WASM)

### Persistent Storage (`storage`) — Requires `persistence`

Journal-based crash-resilient storage for CRDTs:

```rust
use logicaffeine_system::storage::Persistent;
use logicaffeine_data::crdt::GCounter;

let vfs = Arc::new(NativeVfs::new("/data"));
let counter = Persistent::<GCounter>::mount(vfs, "counter.lsf").await?;

counter.mutate(|c| c.increment(5)).await?;
counter.compact().await?;  // Reduce journal size
```

**Journal format:**
```
┌─────────────┬─────────────┬─────────────────┐
│ Length (4B) │ CRC32 (4B)  │ Payload (N B)   │
└─────────────┴─────────────┴─────────────────┘
```

### Concurrency (`concurrency`) — Requires `concurrency`

Go-like concurrency primitives:

```rust
use logicaffeine_system::concurrency::{spawn, Pipe, check_preemption};

// Spawn async task
let handle = spawn(async { expensive_computation().await });
if handle.is_finished() {
    let result = handle.await?;
}
handle.abort();  // Cancel if needed

// Bounded channel (Go-like)
let (tx, mut rx) = Pipe::<String>::new(16);
tx.send("hello".to_string()).await?;
let msg = rx.recv().await;

// Cooperative yielding (10ms threshold)
for i in 0..1_000_000 {
    heavy_work(i);
    check_preemption().await;
}
```

### Memory Zones (`memory`) — Requires `concurrency`

Arena allocation with bulk deallocation:

```rust
use logicaffeine_system::memory::Zone;

// Heap zone for temporary allocations
let zone = Zone::new_heap(1024 * 1024);  // 1 MB arena
let x = zone.alloc(42);
let slice = zone.alloc_slice(&[1, 2, 3]);

// Mapped zone for zero-copy file access (requires persistence)
let zone = Zone::new_mapped("data.bin")?;
let bytes = zone.as_slice();
```

### Networking (`network`) — Requires `networking`

P2P mesh networking with libp2p:

```rust
use logicaffeine_system::network::{listen, connect, send, PeerAgent};
use logicaffeine_system::network::{gossip_publish, gossip_subscribe};

// Start listening
listen("/ip4/0.0.0.0/tcp/8000").await?;

// Connect to peer
connect("/ip4/192.168.1.100/tcp/8000").await?;

// Point-to-point messaging
let peer = PeerAgent::new("/ip4/192.168.1.100/tcp/8000/p2p/12D3Koo...")?;
send(&peer, &MyMessage { data: 42 }).await?;

// Pub/sub broadcast
gossip_subscribe("my-topic").await;
gossip_publish("my-topic", &data).await?;
```

### Synced CRDTs (`crdt`) — Requires `networking`

Auto-replicated CRDT wrapper (ephemeral, no persistence):

```rust
use logicaffeine_system::crdt::Synced;
use logicaffeine_data::crdt::GCounter;

let synced = Synced::new(GCounter::new(), "game-scores").await;

synced.mutate(|c| c.increment(5)).await;
let value = synced.get().await;
```

### Distributed CRDTs (`distributed`) — Requires `distributed`

Persistent + networked CRDT (survives restarts):

```rust
use logicaffeine_system::distributed::Distributed;
use logicaffeine_data::crdt::GCounter;

// Disk-only (same as Persistent<T>)
let counter = Distributed::<GCounter>::mount(vfs, "counter.lsf", None).await?;

// Disk + Network sync
let counter = Distributed::<GCounter>::mount(
    vfs,
    "counter.lsf",
    Some("game-scores".into())
).await?;

// Mutations are persisted AND broadcast
counter.mutate(|c| c.increment(1)).await?;
```

**Data flow:**
```
Local mutation:   RAM → Journal → Network
Remote update:    Network → RAM → Journal
```

## Usage Examples

### Cargo.toml

```toml
# Lean build (default)
[dependencies]
logicaffeine-system = "0.6"

# With persistence
[dependencies]
logicaffeine-system = { version = "0.6", features = ["persistence"] }

# Full features
[dependencies]
logicaffeine-system = { version = "0.6", features = ["full"] }
```

### Persistent Counter

```rust
use logicaffeine_system::storage::Persistent;
use logicaffeine_system::fs::NativeVfs;
use logicaffeine_data::crdt::GCounter;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let vfs = Arc::new(NativeVfs::new("./data"));
    let counter = Persistent::<GCounter>::mount(vfs, "visits.lsf").await?;

    counter.mutate(|c| c.increment(1)).await?;
    println!("Visits: {}", counter.get().await.value());

    Ok(())
}
```

### Producer-Consumer with Pipes

```rust
use logicaffeine_system::concurrency::{spawn, Pipe, check_preemption};

#[tokio::main]
async fn main() {
    let (tx, mut rx) = Pipe::<i32>::new(16);

    let producer = spawn(async move {
        for i in 0..100 {
            tx.send(i).await.unwrap();
            check_preemption().await;
        }
    });

    while let Some(value) = rx.recv().await {
        println!("Received: {}", value);
    }

    producer.await.unwrap();
}
```

## Platform Support

| Feature | Native | WASM |
|---------|--------|------|
| `io` (show, println, read_line) | ✓ | ✓ |
| `fmt` | ✓ | ✓ |
| `time` | ✓ | ✗ |
| `env` | ✓ | ✗ |
| `random` | ✓ | ✗ |
| `file` | ✓ | ✗ |
| `fs` (NativeVfs) | ✓ | — |
| `fs` (OpfsVfs) | — | ✓ |
| `storage` (Persistent) | ✓ | ✓ |
| `concurrency` | ✓ | ✗ |
| `memory` | ✓ | ✗ |
| `network` | ✓ | ✗ |
| `crdt` (Synced) | ✓ | ✗ |
| `distributed` (disk-only) | ✓ | ✓ |
| `distributed` (networked) | ✓ | ✗ |

## Key Design Patterns

### Journal-Based Crash-Resilient Storage

All persistent state uses append-only journals with CRC32 checksums:
- **Snapshots** replace state during compaction
- **Deltas** record incremental updates
- **Truncated entries** are ignored (WAL semantics)
- **Auto-compaction** when entry count exceeds threshold

### Showable Trait

Natural formatting for LOGOS values:
- Primitives: displayed as-is (`42`, `true`, `hello`)
- Collections: bracket notation (`[1, 2, 3]`)
- Options: `nothing` for None, value for Some
- CRDTs: logical value (`GCounter` shows count, not internal state)

### Zone-Based Memory

"Hotel California" rule—values enter but don't escape:
- **Heap zones**: Fast bump allocation, O(1) bulk deallocation
- **Mapped zones**: Zero-copy file access via mmap

### Cooperative Multitasking

The `check_preemption()` function yields after 10ms of computation, balancing responsiveness against context-switch overhead.

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.
