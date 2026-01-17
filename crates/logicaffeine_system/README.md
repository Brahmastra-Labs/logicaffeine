# logicaffeine-system

Platform IO and System Services library for LOGOS.

## Overview

This crate provides platform-specific IO operations, persistence, networking, and distributed coordination. It follows a **lean by default** philosophy—the core IO module compiles to ~50KB with no heavy dependencies. Features like networking and persistence are opt-in.

### Design Principles

- **Lean Core**: Basic IO (print, read, show) works with zero feature flags
- **Cross-Platform**: Native + WASM support with platform-specific implementations
- **CRDT-First**: Persistence and networking integrate directly with `logicaffeine_data` CRDTs
- **Feature-Gated**: Heavy dependencies (libp2p, memmap2, rayon) are optional

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| (none) | Yes | Core IO only (~50KB) |
| `persistence` | No | File operations, VFS, journaled storage (~120KB) |
| `networking` | No | libp2p P2P networking, GossipSub (~2MB) |
| `concurrency` | No | Async tasks, channels, zone allocator (~80KB) |
| `distributed` | No | `networking` + `persistence` combined |
| `full` | No | All features |

## Core Modules (Always Available)

### `io` - Output and Input

The `Showable` trait provides natural formatting for LOGOS types:

```rust
use logicaffeine_system::{show, read_line, Showable};

let numbers = vec![1, 2, 3];
show(&numbers);      // Prints: [1, 2, 3]
show(&42);           // Prints: 42
show(&"hello");      // Prints: hello (no quotes)

let input = read_line();  // Read line from stdin
```

Showable is implemented for primitives, `Vec<T>`, `Option<T>`, and CRDT types like `GCounter` and `PNCounter`.

### `time` - Time Operations (Native Only)

```rust
use logicaffeine_system::time;

let timestamp = time::now();  // Milliseconds since Unix epoch
time::sleep(1000);            // Sleep for 1 second
```

### `env` - Environment Access (Native Only)

```rust
use logicaffeine_system::env;

let home = env::get("HOME".to_string());  // Get environment variable
let args = env::args();                    // Get command-line arguments
```

### `random` - Random Number Generation (Native Only)

```rust
use logicaffeine_system::random;

let n = random::randomInt(1, 100);  // Random integer in [1, 100]
let f = random::randomFloat();       // Random float in [0.0, 1.0)
```

## Persistence Feature

Requires `--features persistence`

### `file` - Simple File I/O

```rust
use logicaffeine_system::file;

file::write("data.txt".into(), "Hello".into())?;
let content = file::read("data.txt".into())?;
```

### `fs` - Virtual File System

Platform-agnostic async file operations with sandboxed paths:

```rust
use logicaffeine_system::fs::{NativeVfs, Vfs};
use std::sync::Arc;

let vfs = Arc::new(NativeVfs::new("./data"));

// All paths are relative to the base directory
vfs.write("config.json", b"{\"version\": 1}").await?;
let content = vfs.read_to_string("config.json").await?;

// Atomic writes (temp file + rename)
vfs.write("important.dat", data).await?;

// Directory operations
vfs.create_dir_all("nested/path").await?;
let entries = vfs.list_dir("nested").await?;
```

On WASM, use `OpfsVfs` for Origin Private File System access.

### `storage` - Journaled CRDT Persistence

`Persistent<T>` wraps any CRDT with crash-resilient storage:

```rust
use logicaffeine_system::storage::Persistent;
use logicaffeine_system::fs::{NativeVfs, Vfs};
use logicaffeine_data::crdt::GCounter;
use std::sync::Arc;

let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(NativeVfs::new("./data"));
let counter = Persistent::<GCounter>::mount(vfs, "counter.journal").await?;

// Mutations are automatically journaled
counter.mutate(|c| c.increment(5)).await?;

// Survives restarts - state is replayed from journal
let value = counter.get().await;

// Compact journal when it grows large
counter.maybe_compact(1000).await?;
```

**Journal Format**: `[4 bytes: length][4 bytes: CRC32][payload]`

Journal entries are append-only with CRC32 checksums. Compaction writes a snapshot and truncates the log.

## Networking Feature

Requires `--features networking`

### `network` - P2P Mesh Networking

libp2p-based peer-to-peer networking with automatic peer discovery:

```rust
use logicaffeine_system::network::{listen, connect, send, local_peer_id, MeshNode};

// Start listening for connections
listen("/ip4/0.0.0.0/tcp/9000").await?;

// Connect to a peer
connect("/ip4/192.168.1.100/tcp/9000").await?;

// Send a message to a peer
send(peer_id, message_bytes).await?;

// Get local peer ID
let my_id = local_peer_id().await?;
```

### `network::gossip` - Pub/Sub Messaging

GossipSub protocol for topic-based broadcast:

```rust
use logicaffeine_system::network::gossip;

// Subscribe to a topic
let mut rx = gossip::subscribe("game-scores").await;

// Publish to a topic
gossip::publish("game-scores", &my_score).await;

// Receive messages
while let Some(bytes) = rx.recv().await {
    let score: Score = bincode::deserialize(&bytes)?;
}
```

### `crdt` - Network-Synced CRDTs

`Synced<T>` automatically replicates CRDT changes over GossipSub:

```rust
use logicaffeine_system::crdt::Synced;
use logicaffeine_data::crdt::GCounter;

let counter = Synced::new(GCounter::new(), "game-scores").await;

// Mutations are automatically broadcast to all subscribers
counter.mutate(|c| c.increment(10)).await;

// Remote changes are automatically merged
let current = counter.get().await;
```

## Concurrency Feature

Requires `--features concurrency`

### `concurrency` - Async Task Primitives

Go-like concurrency with tasks and channels:

```rust
use logicaffeine_system::concurrency::{spawn, Pipe, check_preemption, TaskHandle};

// Spawn an async task
let handle: TaskHandle<i32> = spawn(async {
    expensive_computation().await
});

// Check completion
if handle.is_finished() {
    let result = handle.await?;
}

// Abort if needed
handle.abort();
```

**Pipe Channels** - Bounded async channels (Go-style):

```rust
let (tx, mut rx) = Pipe::<String>::new(16);

spawn(async move {
    tx.send("hello".to_string()).await.unwrap();
});

let msg = rx.recv().await;
```

**Cooperative Scheduling** - The "Nanny" function:

```rust
// In long-running loops, periodically yield to other tasks
for i in 0..1_000_000 {
    heavy_computation(i);
    check_preemption().await;  // Yields if >10ms since last yield
}
```

### `memory` - Zone-Based Allocation

Region-based memory management with O(1) bulk deallocation:

```rust
use logicaffeine_system::memory::Zone;

// Heap arena - fast allocation, bulk free on drop
let zone = Zone::new_heap(1024 * 1024);  // 1MB arena
let x = zone.alloc(42);
let slice = zone.alloc_slice(&[1, 2, 3, 4, 5]);

// Memory-mapped file (requires persistence + concurrency features)
let mapped = Zone::new_mapped("large-file.bin")?;
let bytes = mapped.as_slice();  // Zero-copy access
```

Zone implements the "Hotel California" rule: values can enter but cannot escape. When the zone is dropped, all allocations are freed at once.

## Distributed Feature

Requires `--features distributed` (enables both `networking` and `persistence`)

### `distributed` - Persistent + Network-Synced CRDTs

`Distributed<T>` combines in-memory state, journaled persistence, and network synchronization:

```rust
use logicaffeine_system::distributed::Distributed;
use logicaffeine_system::fs::{NativeVfs, Vfs};
use logicaffeine_data::crdt::GCounter;
use std::sync::Arc;

let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(NativeVfs::new("./data"));

// Disk + Network synchronization
let counter = Distributed::<GCounter>::mount(
    vfs,
    "counter.lsf",           // Journal file
    Some("scores".into())    // GossipSub topic
).await?;

// Mutations flow: RAM -> Journal -> Network
counter.mutate(|c| c.increment(5)).await?;

// Remote updates flow: Network -> RAM -> Journal
// (automatically via background receive loop)
```

**Data Flow**:
- Local mutation: RAM -> Journal -> Network
- Remote update: Network -> RAM -> Journal

This solves the "data loss on restart" problem where `Synced<T>` merges remote updates to RAM but doesn't persist them.

## Platform Support

| Platform | Core IO | Persistence | Networking | Concurrency |
|----------|---------|-------------|------------|-------------|
| Linux | Yes | Yes | Yes | Yes |
| macOS | Yes | Yes | Yes | Yes |
| Windows | Yes | Yes | Yes | Yes |
| WASM | Yes | OPFS only | No* | Partial |

*WASM networking requires WebSocket relay (future work). `Distributed<T>` on WASM behaves like `Persistent<T>`.

## Dependencies

### Always Included (Core)
- `logicaffeine-base` - Base types
- `logicaffeine-data` - CRDTs
- `serde`, `bincode` - Serialization
- `crc32fast` - Journal checksums
- `async-lock` - Cross-platform async mutex

### Native-Only
- `tokio` - Async runtime
- `rand`, `getrandom` - Random number generation
- `uuid` - Unique identifiers

### Feature-Gated
- `persistence`: `memmap2`, `sha2`
- `networking`: `libp2p`, `futures`
- `concurrency`: `rayon`, `bumpalo`

### WASM
- `wasm-bindgen`, `web-sys`, `js-sys`

## License

BUSL-1.1
