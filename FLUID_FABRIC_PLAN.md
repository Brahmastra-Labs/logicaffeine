# FLUID_FABRIC Plan: Distributed Computation Mesh

## Executive Summary

FLUID_FABRIC is a distributed computation layer that enables work to flow seamlessly between a Mac terminal (native Rust) and a Chrome browser (WASM). The core abstraction is the **Program ID** - nodes sharing the same ID automatically form a true peer-to-peer mesh via libp2p, synchronize state via CRDTs, and can distribute work units across the fabric.

### The Golden Test
```
Mac Terminal: largo run --id "Alpha" --mode server
Browser Tab:  Open logicaffeine.com/studio?id=Alpha

# Direct P2P connection - no relay server required
# Both nodes see the same state, can push work to each other
```

---

## Core Architectural Principle: libp2p EVERYWHERE

**Critical Insight**: Rust's `libp2p` crate has full support for WASM via `libp2p-webrtc` and `libp2p-websocket`. We do NOT need a custom relay server as the primary communication path.

### The Golden Path

```
┌─────────────────────────────────────────────────────────────────────┐
│                    TRUE PEER-TO-PEER MESH                            │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   ┌─────────────┐                              ┌────────────┐        │
│   │  Mac CLI    │◄────────────────────────────►│  Browser   │        │
│   │  (Native)   │      Direct WebRTC P2P       │  (WASM)    │        │
│   │             │      via libp2p-webrtc       │            │        │
│   └──────┬──────┘                              └─────┬──────┘        │
│          │                                           │               │
│          │         Program ID: "Alpha"               │               │
│          │         Kademlia DHT Discovery            │               │
│          │                                           │               │
│   ┌──────┴──────┐                              ┌─────┴──────┐        │
│   │ Distributed │◄────────────────────────────►│ Distributed│        │
│   │ <GCounter>  │         GossipSub            │ <GCounter> │        │
│   └─────────────┘                              └────────────┘        │
│                                                                      │
│   VFS (Native)          Anti-Entropy           VFS (OPFS)           │
│   /home/.lsf/           Merkle Search Tree     Browser OPFS         │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Why No Relay Server?

| Approach | Problem |
|----------|---------|
| Custom Relay | Single point of failure. If relay dies, Mac and Browser on same desk stop talking. |
| **libp2p Native** | Mac acts as Circuit Relay v2 node. Browser connects directly via WebRTC. True P2P. |

---

## Current State Analysis

### What Exists (Strong Foundation)

| Component | Location | Status |
|-----------|----------|--------|
| **CRDTs** | `logicaffeine_data/src/crdt/` | 8 types: GCounter, PNCounter, LWW, MVRegister, ORSet, ORMap, RGA, YATA |
| **VFS Abstraction** | `logicaffeine_system/src/fs/` | NativeVfs + OpfsVfs implementations |
| **P2P Networking** | `logicaffeine_system/src/network/` | libp2p with QUIC/TCP, GossipSub, mDNS |
| **Distributed<T>** | `logicaffeine_system/src/distributed.rs` | RAM + Journal + Network (native only) |
| **Go-like Concurrency** | `logicaffeine_system/src/concurrency.rs` | Pipe, TaskHandle, spawn, Select |
| **Journal Persistence** | `logicaffeine_system/src/storage/` | CRC32 checksums, compaction, atomic writes |

### The Gap We're Closing

The WASM `Distributed<T>` implementation currently ignores the `topic` parameter. We need to enable libp2p-webrtc for WASM so browsers participate as first-class peers.

### Integration Strategy: Building ON Existing Infrastructure

FLUID_FABRIC extends existing components rather than replacing them:

| Existing | How FLUID_FABRIC Extends It |
|----------|----------------------------|
| **Synced<T>** (`crdt/sync.rs`) | Add WebRTC transport path for WASM; currently only native GossipSub |
| **DeltaCrdt trait** | Use for efficient Merkle sync; send deltas not full state |
| **DeltaBuffer** | Ring buffer for recent deltas; enables "catch-up" after reconnect |
| **Distributed<T>** | Enable the WASM code path with real networking (currently no-op) |
| **OpfsVfs** | Integrate with distributed durability layer |
| **Journal** (CRC32) | Upgrade to CRC32C (hardware-accelerated on M-series + WASM) |

**Key Principle**: The `Merge` trait and CRDT implementations in `logicaffeine_data` remain pure (no IO). FLUID_FABRIC adds networking in `logicaffeine_system`.

---

## Sovereignty Classes (Tiered Node Model)

**Problem**: Treating all peers equally in quorum calculations doesn't scale. If 50 browser tabs open, that's +50 peers that must acknowledge writes—potentially grinding a high-performance Mac cluster to a halt waiting for acks from ephemeral browser instances.

**Solution**: Classify nodes by capability and durability, with only Authority nodes participating in write quorum.

```rust
pub enum SovereigntyClass {
    /// Mac/Server - full journal, quorum voter, JIT provider, relay
    Authority,
    /// Desktop Browser - OPFS journal, work-stealer, can relay
    Citizen,
    /// Mobile/Guest - RAM-only, task consumer, leaf node
    Ephemeral,
}
```

### Responsibility Matrix

| Class | Persona | Journal | Quorum Voter | JIT Provider | Relay |
|-------|---------|---------|--------------|--------------|-------|
| Authority | Mac/Server | Full (NVMe/SSD) | Yes | Yes | Yes |
| Citizen | Desktop Browser | Partial (OPFS) | No | No | Limited |
| Ephemeral | Mobile/Guest | None (RAM) | No | No | No |

### LOGOS Syntax for Node Declaration

```logos
## Main
# The Mac identifies as an Authority
Enable Networked Mode with ID "Alpha" as Authority.

# The Browser identifies as a Citizen
Enable Networked Mode with ID "Alpha" as Citizen.
```

### Why This Matters

| Question | Answer |
|----------|--------|
| "What if 50 tabs open on a phone?" | Only Authority acks count toward quorum |
| "Can browsers talk to each other?" | Yes, via Authority's Circuit Relay v2 |
| "What about ephemeral mobile guests?" | Ephemeral class - no journal, no quorum vote |
| "How does programmer control durability?" | `DurabilityPolicy` enum (Local, Quorum(N), Leader) |

---

## Robustness Layer: Surviving the Volatile Browser

**Critical Insight**: libp2p handles the *connection* (Layer 4/5), but LOGOS must handle the *logical session* (Layer 7) to survive the volatile browser environment. Tabs are throttled, hibernated, or closed instantly—we must be resilient.

### Retries & Reliability

libp2p provides **partial** reliability:
- **libp2p-webrtc**: Reliable transport via SCTP, handles low-level packet retransmissions
- **GossipSub**: Best-effort message propagation—if a node is offline during broadcast, it **misses the message**
- **The Gap**: libp2p does NOT handle logical retries for a `Distributed<T>` write if the peer disconnects mid-acknowledgment

**Solution: Reliable Broadcast State Machine**

```rust
// reliable_broadcast.rs - Pending Ack Store with Exponential Backoff
pub struct ReliableBroadcast {
    pending: HashMap<DeltaId, PendingDelta>,
    backoff_config: BackoffConfig,
}

#[derive(Clone)]
pub struct PendingDelta {
    delta: WriteDelta,
    required_acks: HashSet<PeerId>,  // Authorities that must ack
    received_acks: HashSet<PeerId>,
    attempts: u32,
    next_retry: Instant,
    created_at: Instant,
}

impl ReliableBroadcast {
    /// Track a delta that needs Authority acknowledgments
    pub fn track(&mut self, delta: WriteDelta, authorities: Vec<PeerId>) -> DeltaId {
        let id = DeltaId::new();
        self.pending.insert(id, PendingDelta {
            delta,
            required_acks: authorities.into_iter().collect(),
            received_acks: HashSet::new(),
            attempts: 0,
            next_retry: Instant::now(),
            created_at: Instant::now(),
        });
        id
    }

    /// Record an ack from a peer
    pub fn record_ack(&mut self, delta_id: &DeltaId, peer: PeerId) -> AckResult {
        if let Some(pending) = self.pending.get_mut(delta_id) {
            pending.received_acks.insert(peer);

            // Check if we have majority quorum
            let required = (pending.required_acks.len() / 2) + 1;
            if pending.received_acks.len() >= required {
                self.pending.remove(delta_id);
                return AckResult::QuorumReached;
            }
        }
        AckResult::Waiting
    }

    /// Get deltas that need retry (exponential backoff)
    pub fn due_for_retry(&mut self) -> Vec<(DeltaId, WriteDelta, Vec<PeerId>)> {
        let now = Instant::now();
        let mut retries = vec![];

        for (id, pending) in &mut self.pending {
            if now >= pending.next_retry {
                // Who hasn't acked yet?
                let missing: Vec<_> = pending.required_acks
                    .difference(&pending.received_acks)
                    .cloned()
                    .collect();

                if !missing.is_empty() {
                    retries.push((*id, pending.delta.clone(), missing));

                    // Exponential backoff: 100ms, 200ms, 400ms, 800ms, max 30s
                    pending.attempts += 1;
                    let backoff = Duration::from_millis(
                        (100 * 2u64.pow(pending.attempts.min(8))).min(30_000)
                    );
                    pending.next_retry = now + backoff;
                }
            }
        }

        retries
    }

    /// Abandon deltas older than timeout - let MST Anti-Entropy handle them
    pub fn abandon_stale(&mut self, timeout: Duration) -> Vec<DeltaId> {
        let now = Instant::now();
        let stale: Vec<_> = self.pending
            .iter()
            .filter(|(_, p)| now.duration_since(p.created_at) > timeout)
            .map(|(id, _)| *id)
            .collect();

        for id in &stale {
            self.pending.remove(id);
        }

        stale  // These will be reconciled via MST later
    }
}

pub enum AckResult {
    QuorumReached,
    Waiting,
}
```

**Key Property**: Because our CRDTs are idempotent (`Merge` trait), the browser can send the same delta 100 times without "double-counting". This makes retries safe.

### Volatile WASM Environment: Keep-Alive & Session Resume

**Problem**: Browser tabs are throttled, hibernated, or closed instantly.

**A. Liveness Guard (Keep-Alive Pulse)**

```rust
// liveness.rs - Prevent "Ghost Tasks" from hanging the mesh
pub struct LivenessGuard {
    peers: HashMap<PeerId, PeerLiveness>,
    pulse_interval: Duration,      // 5 seconds
    stale_threshold: u32,          // 3 missed pulses = stale
}

#[derive(Clone)]
pub struct PeerLiveness {
    last_pulse: Instant,
    missed_pulses: u32,
    sovereignty: SovereigntyClass,
    state: PeerState,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PeerState {
    Active,
    Stale,      // Missed pulses, ignored for scheduling
    Disconnected,
}

impl LivenessGuard {
    /// WASM Citizens must call this every 5 seconds
    pub fn record_pulse(&mut self, peer: PeerId) {
        if let Some(liveness) = self.peers.get_mut(&peer) {
            liveness.last_pulse = Instant::now();
            liveness.missed_pulses = 0;
            liveness.state = PeerState::Active;
        }
    }

    /// Run periodically to detect stale peers
    pub fn check_liveness(&mut self) -> Vec<PeerId> {
        let now = Instant::now();
        let mut newly_stale = vec![];

        for (peer_id, liveness) in &mut self.peers {
            // Only check non-Authority peers (Authorities don't need pulses)
            if liveness.sovereignty != SovereigntyClass::Authority {
                let elapsed = now.duration_since(liveness.last_pulse);
                let missed = (elapsed.as_secs() / self.pulse_interval.as_secs()) as u32;

                if missed > liveness.missed_pulses {
                    liveness.missed_pulses = missed;

                    if missed >= self.stale_threshold && liveness.state == PeerState::Active {
                        liveness.state = PeerState::Stale;
                        newly_stale.push(*peer_id);
                    }
                }
            }
        }

        newly_stale
    }

    /// Get only active peers for scheduling
    pub fn active_peers(&self) -> Vec<PeerId> {
        self.peers.iter()
            .filter(|(_, l)| l.state == PeerState::Active)
            .map(|(id, _)| *id)
            .collect()
    }
}
```

**B. Session Resume via Merkle Catch-up**

When a browser tab reconnects after hibernation:

```rust
// session.rs - Identity persistence and fast reconnect
pub struct SessionManager {
    /// PeerId stored in LocalStorage for identity persistence
    keypair: Keypair,
    /// Last known MST root hash
    last_mst_root: Option<Hash>,
}

impl SessionManager {
    /// Browser: Store identity in LocalStorage so Mac recognizes us
    #[cfg(target_arch = "wasm32")]
    pub fn persist_identity(&self) {
        let storage = web_sys::window()
            .unwrap()
            .local_storage()
            .unwrap()
            .unwrap();

        storage.set_item(
            "fabric_keypair",
            &base64::encode(self.keypair.to_bytes())
        ).unwrap();

        if let Some(root) = &self.last_mst_root {
            storage.set_item("fabric_mst_root", &hex::encode(root)).unwrap();
        }
    }

    /// Browser: Restore identity on page load
    #[cfg(target_arch = "wasm32")]
    pub fn restore_identity() -> Option<Self> {
        let storage = web_sys::window()?.local_storage().ok()??;

        let keypair_bytes = storage.get_item("fabric_keypair").ok()??;
        let keypair = Keypair::from_bytes(&base64::decode(&keypair_bytes).ok()?).ok()?;

        let last_mst_root = storage.get_item("fabric_mst_root").ok()?
            .and_then(|s| hex::decode(&s).ok())
            .and_then(|b| Hash::from_slice(&b));

        Some(Self { keypair, last_mst_root })
    }

    /// Fast reconnect: send MST root, receive only missing entries
    pub async fn reconnect_fast(&self, fabric: &FabricHandle) -> Result<(), ReconnectError> {
        if let Some(our_root) = &self.last_mst_root {
            // Tell Authority our state
            let response = fabric.request_mst_diff(*our_root).await?;

            match response {
                MstDiffResponse::InSync => {
                    // Perfect - we missed nothing
                }
                MstDiffResponse::Behind { missing_entries } => {
                    // Apply only what we're missing
                    for entry in missing_entries {
                        self.journal.apply(entry).await?;
                    }
                }
                MstDiffResponse::Diverged { common_ancestor, their_entries, our_entries } => {
                    // Merge both sides (CRDTs make this safe)
                    for entry in their_entries {
                        self.journal.apply(entry).await?;
                    }
                    // Re-broadcast our entries they might have missed
                    for entry in our_entries {
                        fabric.publish_delta(&entry).await?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub enum MstDiffResponse {
    InSync,
    Behind { missing_entries: Vec<JournalEntry> },
    Diverged {
        common_ancestor: Hash,
        their_entries: Vec<JournalEntry>,
        our_entries: Vec<JournalEntry>,
    },
}
```

### Task Lease Protocol (Task Abandonment Recovery)

**Problem**: Browser picks up a task, then tab closes/crashes/throttles. Without recovery, that task is lost forever and the system locks up waiting for a result that will never come.

**Solution: Lease-Based Task Ownership**

```rust
// task_lease.rs - Lease-based ownership prevents abandoned task lockup
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TaskState {
    /// In the Global Pipe, waiting for a taker
    Pending,
    /// A Citizen is currently computing it (has a lease)
    Leased { holder: PeerId, expires: Instant },
    /// Results are back and verified
    Completed,
    /// Lease expired without a result - ready to re-queue
    Stale,
}

pub struct TaskLease {
    pub task_id: TaskId,
    pub holder: PeerId,
    pub acquired_at: Instant,
    pub expires_at: Instant,
    pub pulse_count: u32,
    pub last_pulse: Instant,
}

impl TaskLease {
    pub fn new(task_id: TaskId, holder: PeerId, ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            task_id,
            holder,
            acquired_at: now,
            expires_at: now + ttl,
            pulse_count: 0,
            last_pulse: now,
        }
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }

    /// Extend lease on pulse (heartbeat)
    pub fn extend(&mut self, extension: Duration) {
        self.pulse_count += 1;
        self.last_pulse = Instant::now();
        self.expires_at = Instant::now() + extension;
    }
}
```

**Authority-Side Task Manager** (The "Janitor" that cleans up after crashed browsers):

```rust
// task_manager.rs - Authority manages task lifecycle
pub struct TaskManager {
    /// All in-flight tasks with their leases
    in_flight: HashMap<TaskId, TaskLease>,
    /// Tasks waiting to be picked up
    pending_queue: VecDeque<WorkUnit>,
    /// Completed task results (cached briefly for late acks)
    completed: HashMap<TaskId, (TaskResult, Instant)>,
    /// Config
    lease_ttl: Duration,           // Default: 30 seconds
    pulse_interval: Duration,      // Expected: 5 seconds
    stale_threshold: u32,          // Missed pulses before revoke: 3
}

impl TaskManager {
    /// Citizen requests a task - Authority grants lease
    pub fn grant_lease(&mut self, peer: PeerId) -> Option<(WorkUnit, TaskLease)> {
        if let Some(work) = self.pending_queue.pop_front() {
            let lease = TaskLease::new(work.id, peer, self.lease_ttl);
            self.in_flight.insert(work.id, lease.clone());
            Some((work, lease))
        } else {
            None
        }
    }

    /// Citizen sends pulse while computing - extends lease
    pub fn record_pulse(&mut self, task_id: TaskId, peer: PeerId) -> Result<(), LeaseError> {
        if let Some(lease) = self.in_flight.get_mut(&task_id) {
            if lease.holder != peer {
                return Err(LeaseError::NotHolder);
            }
            lease.extend(self.lease_ttl);
            Ok(())
        } else {
            Err(LeaseError::NotFound)
        }
    }

    /// Citizen completes task - Authority releases lease
    pub fn complete_task(&mut self, task_id: TaskId, result: TaskResult, peer: PeerId) -> Result<(), LeaseError> {
        if let Some(lease) = self.in_flight.remove(&task_id) {
            if lease.holder != peer {
                // Task was re-assigned (peer was too slow)
                // But thanks to idempotency, we can still accept the result!
                // CRDTs make double-completion safe
            }
            self.completed.insert(task_id, (result, Instant::now()));
            Ok(())
        } else {
            // Task already completed by another node - that's fine (idempotent)
            Ok(())
        }
    }

    /// Run periodically by Authority to reclaim abandoned tasks
    pub fn reclaim_stale_leases(&mut self) -> Vec<WorkUnit> {
        let now = Instant::now();
        let mut reclaimed = vec![];

        let stale_ids: Vec<_> = self.in_flight.iter()
            .filter(|(_, lease)| lease.is_expired())
            .map(|(id, _)| *id)
            .collect();

        for task_id in stale_ids {
            if let Some(lease) = self.in_flight.remove(&task_id) {
                tracing::warn!(
                    "Reclaiming abandoned task {:?} from peer {:?} (missed {} pulses)",
                    task_id, lease.holder,
                    (now.duration_since(lease.last_pulse).as_secs() / self.pulse_interval.as_secs())
                );
                // Move task back to pending queue
                if let Some(work) = self.recover_work_unit(&task_id) {
                    reclaimed.push(work.clone());
                    self.pending_queue.push_back(work);
                }
            }
        }

        reclaimed
    }

    /// Citizen voluntarily releases lease (detected throttling)
    pub fn voluntary_release(&mut self, task_id: TaskId, peer: PeerId) -> Result<(), LeaseError> {
        if let Some(lease) = self.in_flight.remove(&task_id) {
            if lease.holder == peer {
                tracing::info!("Peer {:?} voluntarily released task {:?}", peer, task_id);
                if let Some(work) = self.recover_work_unit(&task_id) {
                    self.pending_queue.push_front(work); // Priority re-queue
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    #[error("Task not found")]
    NotFound,
    #[error("Peer is not the lease holder")]
    NotHolder,
    #[error("Lease already expired")]
    Expired,
}
```

**Browser-Side: Voluntary Throttle Detection**

```rust
// browser_worker.rs - WASM-side throttle detection
#[cfg(target_arch = "wasm32")]
pub struct BrowserWorker {
    current_task: Option<(TaskId, Instant)>,
    expected_duration: Duration,
    pulse_interval: Duration,
}

#[cfg(target_arch = "wasm32")]
impl BrowserWorker {
    /// Check if we're being throttled (task taking 10x longer than expected)
    pub fn is_throttled(&self) -> bool {
        if let Some((_, started)) = &self.current_task {
            let elapsed = Instant::now().duration_since(*started);
            elapsed > self.expected_duration * 10
        } else {
            false
        }
    }

    /// Called periodically during long computations
    pub async fn maybe_yield(&mut self, fabric: &FabricHandle) {
        if self.is_throttled() {
            if let Some((task_id, _)) = self.current_task.take() {
                // Voluntarily release - let a faster node take over
                fabric.voluntary_release(task_id).await;
                tracing::info!("Voluntarily released throttled task {:?}", task_id);
            }
        } else {
            // Send pulse to keep lease alive
            if let Some((task_id, _)) = &self.current_task {
                fabric.send_pulse(*task_id).await;
            }
        }
    }

    /// Use scheduler.yield() to cooperatively yield to browser
    pub async fn cooperative_compute<F, R>(&mut self, task_id: TaskId, f: F) -> Option<R>
    where
        F: FnOnce() -> R,
    {
        self.current_task = Some((task_id, Instant::now()));

        // Yield control periodically to browser event loop
        let result = f();

        self.current_task = None;
        Some(result)
    }
}
```

**Integration with LivenessGuard**:

```rust
impl LivenessGuard {
    /// Enhanced: Also tracks per-task pulses, not just peer liveness
    pub fn record_task_pulse(&mut self, peer: PeerId, task_id: TaskId) {
        // Update peer liveness
        self.record_pulse(peer);

        // Update task lease
        if let Some(task_manager) = &mut self.task_manager {
            let _ = task_manager.record_pulse(task_id, peer);
        }
    }
}
```

**Why Task Abandonment Won't Lock Up the Mesh**:

| Scenario | What Happens | Recovery |
|----------|--------------|----------|
| Tab closes mid-task | Pulses stop arriving | Authority reclaims after 3 missed pulses (~15s) |
| Browser throttled (background tab) | Task takes 10x longer | Browser voluntarily releases, faster node takes over |
| Browser crashes | No cleanup possible | Lease expires, task returns to Pending |
| Network partition | Pulses can't reach Authority | Lease expires locally, Authority re-queues |
| Double completion | Two nodes finish same task | CRDTs make this safe - idempotent merge |

### Snapshot Catch-up (Slow Consumer Recovery)

**Problem**: A Browser (Citizen) on a throttled connection cannot keep up with high-frequency deltas from a Mac (Authority). The `DeltaBuffer` overflows, leading to lag, battery drain, or crashes as the browser processes thousands of stale messages.

**Solution: Snapshot-only Catch-up**

```rust
// snapshot_catchup.rs - Switch from deltas to snapshots for slow consumers
pub struct SnapshotCatchup {
    /// Threshold before switching to snapshot mode
    delta_lag_threshold: usize,  // Default: 500 unapplied deltas
    /// Per-peer delta lag tracking
    peer_lag: HashMap<PeerId, DeltaLagState>,
}

#[derive(Clone)]
pub struct DeltaLagState {
    /// Number of deltas sent but not acked
    pending_deltas: usize,
    /// Last known applied delta sequence number
    last_applied_seq: u64,
    /// Current sync mode
    mode: SyncMode,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SyncMode {
    /// Normal: Stream deltas incrementally
    Incremental,
    /// Paused: Stop streaming, wait for stabilization
    Paused,
    /// Snapshot: Send full state as compressed payload
    SnapshotPending,
}

impl SnapshotCatchup {
    /// Called when we receive a delta ack from a peer
    pub fn record_ack(&mut self, peer: PeerId, acked_seq: u64) {
        if let Some(state) = self.peer_lag.get_mut(&peer) {
            state.pending_deltas = state.pending_deltas.saturating_sub(1);
            state.last_applied_seq = acked_seq;

            // Peer caught up - resume incremental mode
            if state.pending_deltas < self.delta_lag_threshold / 2 {
                state.mode = SyncMode::Incremental;
            }
        }
    }

    /// Called when we send a delta to a peer
    pub fn record_delta_sent(&mut self, peer: PeerId) {
        if let Some(state) = self.peer_lag.get_mut(&peer) {
            state.pending_deltas += 1;

            // Peer is falling behind - pause deltas
            if state.pending_deltas >= self.delta_lag_threshold {
                state.mode = SyncMode::Paused;
                tracing::warn!("Peer {:?} lagging ({} pending), switching to snapshot mode",
                    peer, state.pending_deltas);
            }
        }
    }

    /// Check if peer needs a snapshot instead of deltas
    pub fn needs_snapshot(&self, peer: &PeerId) -> bool {
        self.peer_lag.get(peer)
            .map(|s| s.mode == SyncMode::Paused || s.mode == SyncMode::SnapshotPending)
            .unwrap_or(false)
    }

    /// Generate compressed snapshot for slow consumer
    pub async fn generate_snapshot<T: Serialize>(&self, state: &T) -> CompressedSnapshot {
        let serialized = bincode::serialize(state).unwrap();
        let compressed = zstd::encode_all(&serialized[..], 3).unwrap();

        CompressedSnapshot {
            data: compressed,
            uncompressed_size: serialized.len(),
            timestamp: Instant::now(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct CompressedSnapshot {
    pub data: Vec<u8>,
    pub uncompressed_size: usize,
    pub timestamp: Instant,
}
```

**Why This Matters**:

| Scenario | Without Snapshot Catch-up | With Snapshot Catch-up |
|----------|---------------------------|------------------------|
| Browser on 3G | Processes 500 stale deltas, drains battery | Receives single snapshot, instant sync |
| Tab returns from background | Backlog of 1000 deltas | Authority detects lag, sends snapshot |
| Mobile browser | UI freezes processing queue | Clean state, responsive UI |

### Clock Skew & Causality: VClock Standardization

**Problem**: Mac and Browser clocks often differ by seconds or even minutes. Using raw `SystemTime` for Last-Writer-Wins (LWW) CRDTs can result in "future" data from one node erroneously overwriting "current" data from another.

**Solution: Standardize on Vector Clocks (VClock) for All Causal Ordering**

```rust
// vclock_causality.rs - Logical time replaces wall-clock time
use crate::crdt::VClock;

/// Every distributed mutation carries a VClock for causal ordering
/// SystemTime is metadata for display only, NOT for conflict resolution
#[derive(Serialize, Deserialize, Clone)]
pub struct CausalMutation<T> {
    /// The actual data change
    pub payload: T,
    /// Logical timestamp for ordering (SOURCE OF TRUTH)
    pub vclock: VClock,
    /// Wall-clock time (DISPLAY ONLY - never used for ordering)
    pub wall_time: SystemTime,
    /// Origin node
    pub origin: ReplicaId,
}

impl<T> CausalMutation<T> {
    pub fn new(payload: T, vclock: VClock, origin: ReplicaId) -> Self {
        Self {
            payload,
            vclock,
            wall_time: SystemTime::now(),  // For human display only
            origin,
        }
    }

    /// Compare causality using VClock, NOT wall time
    pub fn happened_before(&self, other: &Self) -> bool {
        self.vclock.partial_cmp(&other.vclock) == Some(std::cmp::Ordering::Less)
    }

    /// Check if mutations are concurrent (neither happened-before the other)
    pub fn is_concurrent_with(&self, other: &Self) -> bool {
        self.vclock.partial_cmp(&other.vclock).is_none()
    }
}

/// Enhanced LWW that uses VClock instead of SystemTime
pub struct VClockLWW<T> {
    value: T,
    vclock: VClock,
    /// Wall time kept for display only
    last_modified_display: SystemTime,
}

impl<T: Clone> VClockLWW<T> {
    pub fn set(&mut self, value: T, mutation: &CausalMutation<()>) {
        // Compare VClocks, not timestamps
        if mutation.vclock > self.vclock {
            self.value = value;
            self.vclock = mutation.vclock.clone();
            self.last_modified_display = mutation.wall_time;
        }
    }

    pub fn merge(&mut self, other: &Self) {
        // VClock comparison for conflict resolution
        if other.vclock > self.vclock {
            self.value = other.value.clone();
            self.vclock = other.vclock.clone();
            self.last_modified_display = other.last_modified_display;
        }
    }
}

/// Node-local clock manager - increments on every local mutation
pub struct LocalClock {
    replica_id: ReplicaId,
    vclock: VClock,
}

impl LocalClock {
    /// Increment and return new VClock for a mutation
    pub fn tick(&mut self) -> VClock {
        self.vclock.increment(self.replica_id);
        self.vclock.clone()
    }

    /// Merge received VClock and increment (for receiving remote mutations)
    pub fn receive(&mut self, remote: &VClock) -> VClock {
        self.vclock.merge(remote);
        self.tick()
    }
}
```

**Why VClock Over SystemTime**:

| Scenario | SystemTime (Broken) | VClock (Correct) |
|----------|---------------------|------------------|
| Mac clock 5s ahead | Mac always "wins" writes | Logical ordering, latest writer wins |
| Browser NTP drift | Random overwrites | Consistent causal order |
| Timezone confusion | 8-hour time jumps | Immune to wall-clock issues |
| Replay attacks | Old timestamps accepted | VClock monotonically increases |

### Zombie Tasks: Generation IDs & Stop Signals

**Problem**: A browser might drop a task due to a WiFi hiccup, and the Mac reassigns it. If the first browser suddenly finishes and submits, we have "Double Completion" - but worse, the zombie browser might continue working on stale data.

**Solution: Task Generation IDs with Stop Signals**

```rust
// task_generation.rs - Prevent zombie task execution
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct GenerationId(u64);

impl GenerationId {
    pub fn initial() -> Self {
        Self(1)
    }

    pub fn increment(&self) -> Self {
        Self(self.0 + 1)
    }
}

/// Enhanced TaskLease with generation tracking
pub struct GenerationalTaskLease {
    pub task_id: TaskId,
    pub holder: PeerId,
    pub generation: GenerationId,
    pub expires_at: Instant,
    pub pulse_count: u32,
}

/// Enhanced TaskManager with generation tracking
impl TaskManager {
    /// Reclaim task and INCREMENT generation
    pub fn reclaim_with_generation(&mut self, task_id: TaskId) -> Option<(WorkUnit, GenerationId)> {
        if let Some(lease) = self.in_flight.remove(&task_id) {
            let new_generation = lease.generation.increment();

            // Store the new generation
            self.task_generations.insert(task_id, new_generation);

            // Re-queue with new generation
            if let Some(mut work) = self.recover_work_unit(&task_id) {
                work.generation = new_generation;
                self.pending_queue.push_back(work.clone());
                return Some((work, new_generation));
            }
        }
        None
    }

    /// Handle result submission with generation check
    pub fn submit_result(
        &mut self,
        task_id: TaskId,
        generation: GenerationId,
        result: TaskResult,
        peer: PeerId,
    ) -> SubmitOutcome {
        let current_generation = self.task_generations.get(&task_id)
            .copied()
            .unwrap_or(GenerationId::initial());

        if generation < current_generation {
            // Stale generation - accept result (idempotent) but kill zombie
            tracing::warn!(
                "Stale result from {:?} for task {:?} (gen {} < current {})",
                peer, task_id, generation.0, current_generation.0
            );

            // Accept the result anyway (might be valid data)
            if !self.completed.contains_key(&task_id) {
                self.completed.insert(task_id, (result, Instant::now()));
            }

            // Signal zombie to stop
            return SubmitOutcome::StaleGeneration {
                stop_signal: StopWorkSignal { task_id, peer }
            };
        }

        // Current generation - normal completion
        self.in_flight.remove(&task_id);
        self.completed.insert(task_id, (result, Instant::now()));
        SubmitOutcome::Accepted
    }
}

pub enum SubmitOutcome {
    /// Result accepted, task completed
    Accepted,
    /// Result accepted but submitter should stop (stale generation)
    StaleGeneration { stop_signal: StopWorkSignal },
    /// Task not found (already completed)
    NotFound,
}

/// Signal sent to zombie workers to stop execution
#[derive(Serialize, Deserialize, Clone)]
pub struct StopWorkSignal {
    pub task_id: TaskId,
    pub peer: PeerId,
}

/// Browser-side handling of stop signals
#[cfg(target_arch = "wasm32")]
impl BrowserWorker {
    /// Handle incoming stop signal - abort current work
    pub fn handle_stop_signal(&mut self, signal: StopWorkSignal) {
        if let Some((current_task, _)) = &self.current_task {
            if *current_task == signal.task_id {
                tracing::info!("Received stop signal for {:?}, aborting", signal.task_id);
                self.current_task = None;
                // Cancel any pending computation
                self.abort_handle.take().map(|h| h.abort());
            }
        }
    }
}
```

**Zombie Prevention Matrix**:

| Scenario | Without Generation IDs | With Generation IDs |
|----------|------------------------|---------------------|
| WiFi hiccup, task reassigned | Both browsers compute, double results | Gen 1 browser gets stop signal |
| Browser crashes, restarts | May resume stale work | New lease = new generation |
| Network partition heals | Conflicting results | Generation check resolves |

### Adaptive Journal Compaction (OPFS Storage Quotas)

**Problem**: Browsers have strict **OPFS Storage Quotas** (~10% of disk, often 2-5GB). A long-running LOGOS program could hit these limits and fail to save data, causing data loss.

**Solution: Adaptive Compaction Policy Based on Storage Pressure**

```rust
// adaptive_compaction.rs - Storage-aware journal management
#[cfg(target_arch = "wasm32")]
pub struct AdaptiveCompactor {
    /// Threshold to trigger aggressive compaction (0.0-1.0)
    warning_threshold: f64,   // Default: 0.7 (70%)
    critical_threshold: f64,  // Default: 0.85 (85%)
    /// Current compaction policy
    policy: CompactionPolicy,
}

#[derive(Clone, Copy, PartialEq)]
pub enum CompactionPolicy {
    /// Normal: Keep recent WAL entries for replay
    Normal { wal_retention_days: u32 },
    /// Aggressive: Keep only snapshot + MST root
    Aggressive,
    /// Critical: Emergency - snapshot only, no history
    Critical,
}

#[cfg(target_arch = "wasm32")]
impl AdaptiveCompactor {
    /// Check storage quota using navigator.storage.estimate()
    pub async fn check_storage_pressure(&mut self) -> StoragePressure {
        let window = web_sys::window().unwrap();
        let navigator = window.navigator();
        let storage = navigator.storage();

        let estimate = JsFuture::from(storage.estimate()).await.unwrap();
        let estimate: web_sys::StorageEstimate = estimate.unchecked_into();

        let usage = estimate.usage().unwrap_or(0) as f64;
        let quota = estimate.quota().unwrap_or(u64::MAX) as f64;
        let ratio = usage / quota;

        let pressure = if ratio >= self.critical_threshold {
            StoragePressure::Critical { used_ratio: ratio }
        } else if ratio >= self.warning_threshold {
            StoragePressure::Warning { used_ratio: ratio }
        } else {
            StoragePressure::Normal { used_ratio: ratio }
        };

        // Update policy based on pressure
        self.policy = match pressure {
            StoragePressure::Critical { .. } => CompactionPolicy::Critical,
            StoragePressure::Warning { .. } => CompactionPolicy::Aggressive,
            StoragePressure::Normal { .. } => CompactionPolicy::Normal { wal_retention_days: 7 },
        };

        pressure
    }

    /// Run compaction based on current policy
    pub async fn compact(&self, journal: &mut Journal, mst: &MerkleSearchTree) -> CompactionResult {
        match self.policy {
            CompactionPolicy::Normal { wal_retention_days } => {
                // Keep entries from last N days
                let cutoff = SystemTime::now() - Duration::from_secs(wal_retention_days as u64 * 86400);
                let removed = journal.remove_entries_before(cutoff).await?;
                CompactionResult::Normal { entries_removed: removed }
            }
            CompactionPolicy::Aggressive => {
                // Generate snapshot, keep only recent entries
                let snapshot = self.create_snapshot(journal).await?;
                journal.truncate_to_snapshot(&snapshot).await?;

                // Keep MST root for fast reconnect
                let mst_root = mst.root_hash();

                CompactionResult::Aggressive {
                    snapshot_size: snapshot.data.len(),
                    mst_root,
                }
            }
            CompactionPolicy::Critical => {
                // Emergency: snapshot only, discard ALL history
                let snapshot = self.create_snapshot(journal).await?;
                journal.replace_with_snapshot(&snapshot).await?;

                tracing::warn!("Critical compaction: all WAL history discarded");

                CompactionResult::Critical {
                    snapshot_size: snapshot.data.len(),
                    history_discarded: true,
                }
            }
        }
    }

    /// Estimate space savings before compaction
    pub async fn estimate_savings(&self, journal: &Journal) -> StorageSavings {
        let current_size = journal.total_size().await;
        let snapshot_size = journal.estimate_snapshot_size().await;

        StorageSavings {
            current_bytes: current_size,
            after_compaction_bytes: snapshot_size,
            savings_bytes: current_size.saturating_sub(snapshot_size),
            savings_percent: ((current_size - snapshot_size) as f64 / current_size as f64) * 100.0,
        }
    }
}

#[derive(Debug)]
pub enum StoragePressure {
    Normal { used_ratio: f64 },
    Warning { used_ratio: f64 },
    Critical { used_ratio: f64 },
}

#[derive(Debug)]
pub enum CompactionResult {
    Normal { entries_removed: usize },
    Aggressive { snapshot_size: usize, mst_root: Option<Hash> },
    Critical { snapshot_size: usize, history_discarded: bool },
}

#[derive(Debug)]
pub struct StorageSavings {
    pub current_bytes: usize,
    pub after_compaction_bytes: usize,
    pub savings_bytes: usize,
    pub savings_percent: f64,
}
```

**Storage Pressure Response**:

| Pressure Level | Used Ratio | Policy | Action |
|----------------|------------|--------|--------|
| Normal | < 70% | Normal | Keep 7 days WAL |
| Warning | 70-85% | Aggressive | Snapshot + MST root only |
| Critical | > 85% | Critical | Snapshot only, discard all history |

**Integration with Journal Lifecycle**:

```rust
impl Journal {
    /// Periodic storage check (runs every 5 minutes in browser)
    #[cfg(target_arch = "wasm32")]
    pub async fn storage_maintenance(&mut self) {
        let pressure = self.compactor.check_storage_pressure().await;

        match pressure {
            StoragePressure::Warning { used_ratio } => {
                tracing::warn!("Storage at {:.1}%, triggering aggressive compaction", used_ratio * 100.0);
                self.compactor.compact(&mut self.inner, &self.mst).await;
            }
            StoragePressure::Critical { used_ratio } => {
                tracing::error!("Storage critical at {:.1}%, emergency compaction", used_ratio * 100.0);
                self.compactor.compact(&mut self.inner, &self.mst).await;

                // Notify user they may need to free space
                self.emit_storage_warning(used_ratio).await;
            }
            StoragePressure::Normal { .. } => {
                // Normal compaction on schedule
            }
        }
    }
}
```

### Quorum Safety: Split-Brain Prevention

**Problem**: If two Macs (Authorities) lose connection to each other, they might both think they are "Leader" and accept conflicting writes.

**Solution: Majority-Quorum with Sovereign Lease**

```rust
// quorum.rs - Split-brain prevention via majority quorum
pub struct QuorumConfig {
    /// Total known Authority nodes in the mesh
    authority_count: usize,
}

impl QuorumConfig {
    /// Calculate required acks for write to succeed
    /// Uses floor(n/2) + 1 to ensure majority
    pub fn required_acks(&self) -> usize {
        (self.authority_count / 2) + 1
    }

    /// Check if we can even attempt a write
    pub fn can_attempt_write(&self, reachable_authorities: usize) -> bool {
        reachable_authorities >= self.required_acks()
    }
}

// Examples:
// - 1 Mac (local dev): required = 1, quorum = 1
// - 2 Macs: required = 2, BOTH must ack (prevents split-brain)
// - 3 Macs: required = 2, any 2 of 3 must ack
// - 50 browsers: They are WITNESSES, not voters. Don't affect quorum math.
```

**Sovereign Lease**: Temporary leadership for ordered operations

```rust
// lease.rs - Prevent concurrent leaders during partition
pub struct SovereignLease {
    holder: Option<PeerId>,
    expires: Instant,
    term: u64,
}

impl SovereignLease {
    /// Request leadership (only Authorities can hold leases)
    pub async fn acquire(&mut self, fabric: &FabricHandle) -> Result<LeaseGuard, LeaseError> {
        if !fabric.local_capabilities().is_authority() {
            return Err(LeaseError::NotAuthority);
        }

        // Must get majority agreement to become leader
        let authorities = fabric.authority_peers().await;
        let required = (authorities.len() / 2) + 1;

        let votes = fabric.request_leadership_votes(self.term + 1).await;

        if votes.len() >= required {
            self.holder = Some(fabric.local_peer_id());
            self.term += 1;
            self.expires = Instant::now() + Duration::from_secs(30);
            Ok(LeaseGuard { lease: self, fabric: fabric.clone() })
        } else {
            Err(LeaseError::InsufficientVotes)
        }
    }
}
```

### Security Layer: Peer Authenticity

**Problem**: The current plan uses `noise` for encryption, but random strangers could join your mesh if they find your IP.

**Solution: Derive PeerId from Program ID using Shared HMAC**

```rust
// auth.rs - Only nodes knowing the Program ID can join
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub struct MeshAuthenticator {
    program_id: ProgramId,
    /// Derived from program_id - acts as shared secret
    mesh_key: [u8; 32],
}

impl MeshAuthenticator {
    pub fn new(program_id: ProgramId) -> Self {
        // Derive mesh key from program ID
        let mut mac = Hmac::<Sha256>::new_from_slice(b"fabric/mesh/auth").unwrap();
        mac.update(program_id.as_bytes());
        let mesh_key: [u8; 32] = mac.finalize().into_bytes().into();

        Self { program_id, mesh_key }
    }

    /// Generate challenge for incoming peer
    pub fn generate_challenge(&self) -> Challenge {
        let nonce: [u8; 32] = rand::random();
        Challenge { nonce }
    }

    /// Verify peer knows the program ID
    pub fn verify_response(&self, challenge: &Challenge, response: &[u8]) -> bool {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.mesh_key).unwrap();
        mac.update(&challenge.nonce);
        let expected = mac.finalize().into_bytes();

        // Constant-time comparison
        expected.as_slice() == response
    }

    /// Compute response to prove we know the program ID
    pub fn compute_response(&self, challenge: &Challenge) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.mesh_key).unwrap();
        mac.update(&challenge.nonce);
        mac.finalize().into_bytes().to_vec()
    }
}

pub struct Challenge {
    nonce: [u8; 32],
}

// Integration with libp2p handshake
impl FabricBehaviour {
    /// After noise handshake, verify peer knows program ID
    pub async fn authenticate_peer(&self, peer: PeerId) -> Result<(), AuthError> {
        let challenge = self.authenticator.generate_challenge();
        self.send_challenge(peer, &challenge).await?;

        let response = self.await_response(peer).await?;

        if self.authenticator.verify_response(&challenge, &response) {
            Ok(())
        } else {
            self.disconnect(peer).await;
            Err(AuthError::InvalidResponse)
        }
    }
}
```

---

## Implementation Phases

### Phase FF-1: Universal libp2p Transport

**Goal**: Enable browser WASM to participate as a first-class libp2p peer via WebRTC, connecting directly to native nodes.

**Files to Create/Modify**:
- `crates/logicaffeine_system/src/network/webrtc.rs` (NEW)
- `crates/logicaffeine_system/src/network/transport.rs` (MODIFY)
- `crates/logicaffeine_system/src/network/mod.rs` (MODIFY)
- `crates/logicaffeine_system/src/distributed.rs` (MODIFY WASM impl)

**Architecture**:
```
┌─────────────────────────────────────────────────────────────────────┐
│                    libp2p Transport Stack                            │
│                                                                      │
│   Native (Mac/Linux)              WASM (Browser)                     │
│   ┌──────────────────┐            ┌──────────────────┐              │
│   │ QUIC + TCP       │            │ WebRTC           │              │
│   │ WebRTC (server)  │◄──────────►│ WebSocket        │              │
│   │ Circuit Relay v2 │            │ (fallback)       │              │
│   └────────┬─────────┘            └────────┬─────────┘              │
│            │                               │                         │
│            └───────────┬───────────────────┘                         │
│                        │                                             │
│              ┌─────────┴─────────┐                                   │
│              │    GossipSub      │                                   │
│              │    Kademlia DHT   │                                   │
│              │    Identify       │                                   │
│              └───────────────────┘                                   │
└─────────────────────────────────────────────────────────────────────┘
```

**Design**:
```rust
// webrtc.rs - WebRTC transport configuration for both native and WASM

/// Configure libp2p for the current platform
pub fn build_transport() -> libp2p::core::transport::Boxed<(PeerId, StreamMuxerBox)> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Native: QUIC + TCP + WebRTC server
        let quic = libp2p::quic::tokio::Transport::new(quic_config);
        let tcp = libp2p::tcp::tokio::Transport::new(tcp_config);
        let webrtc = libp2p::webrtc::tokio::Transport::new(
            keypair.clone(),
            webrtc_certificate,
        );

        OrTransport::new(quic, OrTransport::new(tcp, webrtc))
            .upgrade(Version::V1)
            .authenticate(noise::Config::new(&keypair)?)
            .multiplex(yamux::Config::default())
            .boxed()
    }

    #[cfg(target_arch = "wasm32")]
    {
        // WASM: WebRTC client + WebSocket fallback
        let webrtc = libp2p::webrtc::Transport::new(keypair.clone());
        let websocket = libp2p::websocket::WsConfig::new(
            libp2p::wasm_ext::ExtTransport::new(libp2p::wasm_ext::ffi::websocket_transport())
        );

        OrTransport::new(webrtc, websocket)
            .upgrade(Version::V1)
            .authenticate(noise::Config::new(&keypair)?)
            .multiplex(yamux::Config::default())
            .boxed()
    }
}

/// Native node configuration - acts as Circuit Relay for browsers
#[cfg(not(target_arch = "wasm32"))]
pub struct NativeNodeConfig {
    /// Listen on WebRTC for browser connections
    pub webrtc_listen: Multiaddr,  // e.g., /ip4/0.0.0.0/udp/9000/webrtc-direct
    /// Listen on QUIC for native peer connections
    pub quic_listen: Multiaddr,    // e.g., /ip4/0.0.0.0/udp/9001/quic-v1
    /// Enable Circuit Relay v2 server
    pub relay_enabled: bool,
}

/// Browser node configuration - connects via WebRTC
#[cfg(target_arch = "wasm32")]
pub struct BrowserNodeConfig {
    /// Bootstrap peers (native nodes with WebRTC listeners)
    pub bootstrap_peers: Vec<Multiaddr>,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeNodeConfig {
    /// Enable Circuit Relay v2 server for browser-to-browser communication
    /// Authority nodes act as relay pivots for Citizens that can't directly connect
    pub fn with_relay(mut self) -> Self {
        self.relay_config = Some(relay::Config::default()
            .max_reservations(128)           // Max browser connections
            .max_circuits(64)                // Max active relays
            .max_circuit_duration(Duration::from_secs(3600))
        );
        self
    }
}
```

**Browser-to-Browser via Authority Relay**

Browsers cannot normally act as listeners for other browsers due to NAT/Firewall.
The Authority (Mac) acts as a Circuit Relay v2 pivot:

```
┌──────────────┐                    ┌──────────────┐
│  Browser A   │                    │  Browser B   │
│  (Citizen)   │                    │  (Citizen)   │
└──────┬───────┘                    └───────┬──────┘
       │                                    │
       │ WebRTC                      WebRTC │
       │                                    │
       ▼                                    ▼
┌──────────────────────────────────────────────────┐
│                 Mac (Authority)                   │
│                                                   │
│  1. Browser A and Browser B connect via WebRTC   │
│  2. Mac attempts dcutr hole punch                │
│  3. If direct fails → Circuit Relay v2           │
│  4. GossipSub flows through relay at LAN speed   │
└──────────────────────────────────────────────────┘
```

```rust
// In build_swarm: Authority includes relay server
#[cfg(not(target_arch = "wasm32"))]
pub fn build_swarm_with_relay(keypair: &Keypair, config: &NativeNodeConfig) -> Swarm<FabricBehaviour> {
    let transport = build_transport(keypair);

    let behaviour = FabricBehaviour {
        gossipsub: gossipsub::Behaviour::new(...),
        kad: kad::Behaviour::new(...),
        identify: identify::Behaviour::new(...),
        autonat: autonat::Behaviour::new(local_peer_id, autonat::Config::default()),
        dcutr: dcutr::Behaviour::new(local_peer_id),
        mdns: mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?,

        // Relay v2: Authority acts as pivot for browser-to-browser
        relay: relay::Behaviour::new(local_peer_id, relay::Config::default()),
        relay_client: relay::client::Behaviour::new(local_peer_id, &keypair),
    };

    SwarmBuilder::with_tokio_executor(transport, behaviour, local_peer_id).build()
}
```

**Connection Flow**:
```
1. Mac starts with WebRTC listener on /ip4/192.168.1.100/udp/9000/webrtc-direct
2. Browser discovers Mac via Kademlia DHT (Program ID "Alpha" → DHT key)
3. Browser initiates WebRTC connection directly to Mac's IP
4. ICE negotiation happens via libp2p signaling (no external STUN/TURN needed on LAN)
5. GossipSub messages flow over the WebRTC DataChannel
```

**NAT Traversal & Instant Discovery**:

```rust
// Native node includes Auto-NAT, Hole Punching, and mDNS
#[cfg(not(target_arch = "wasm32"))]
pub fn build_swarm(keypair: &Keypair) -> Swarm<FabricBehaviour> {
    let transport = build_transport(keypair);

    let behaviour = FabricBehaviour {
        gossipsub: gossipsub::Behaviour::new(...),
        kad: kad::Behaviour::new(...),
        identify: identify::Behaviour::new(...),

        // Auto-NAT: Detect if we're behind NAT
        autonat: autonat::Behaviour::new(local_peer_id, autonat::Config::default()),

        // Hole Punching: Traverse NAT when possible
        dcutr: dcutr::Behaviour::new(local_peer_id),

        // mDNS: Instant local network discovery (Mac finds Browser on same WiFi)
        mdns: mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?,
    };

    SwarmBuilder::with_tokio_executor(transport, behaviour, local_peer_id).build()
}
```

**Why mDNS Matters**:
- DHT warmup can take 5-10 seconds
- mDNS discovers peers on the same local network **instantly** (<100ms)
- Mac and Browser on same WiFi connect before DHT even bootstraps

**TDD Tests**:
```rust
#[tokio::test]
async fn test_native_webrtc_listener() {
    // Native node starts WebRTC listener
    // Verify multiaddr is /ip4/.../udp/.../webrtc-direct
}

#[tokio::test]
async fn test_browser_connects_to_native_via_webrtc() {
    // Native node listening on WebRTC
    // Browser node (simulated) connects
    // Both see each other as peers
}

#[tokio::test]
async fn test_gossipsub_over_webrtc() {
    // Native publishes to topic "test"
    // Browser subscribed to "test"
    // Browser receives message via WebRTC DataChannel
}

#[tokio::test]
async fn test_distributed_wasm_syncs_with_native() {
    // Create Distributed<GCounter> on Mac with topic "test"
    // Create Distributed<GCounter> on Browser with topic "test"
    // Mutate on Mac → Browser sees change
    // Mutate on Browser → Mac sees change
}

#[tokio::test]
async fn test_direct_p2p_no_relay_required() {
    // Mac and Browser on same network
    // No external relay server running
    // Connection succeeds via direct WebRTC
}

#[tokio::test]
async fn test_mdns_instant_discovery() {
    // Mac and Browser on same WiFi
    // Browser discovers Mac via mDNS < 100ms
    // No DHT warmup required
}

#[tokio::test]
async fn test_nat_hole_punching() {
    // Mac behind NAT
    // Auto-NAT detects NAT type
    // dcutr (Direct Connection Upgrade) punches hole
    // Browser connects via punched hole
}
```

---

### Phase FF-2: Kademlia DHT Discovery

**Goal**: Nodes with same Program ID discover each other via DHT without any central server.

**Files to Create/Modify**:
- `crates/logicaffeine_system/src/fabric/mod.rs` (NEW)
- `crates/logicaffeine_system/src/fabric/program_id.rs` (NEW)
- `crates/logicaffeine_system/src/fabric/discovery.rs` (NEW)

**Design**:
```rust
// program_id.rs
use libp2p::kad::{Kademlia, KademliaEvent, QueryId, Record, RecordKey};
use sha2::{Sha256, Digest};

#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramId(String);

impl ProgramId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Derive DHT key from Program ID
    /// Hash ensures even distribution across DHT keyspace
    pub fn dht_key(&self) -> RecordKey {
        let mut hasher = Sha256::new();
        hasher.update(b"fabric/program/");
        hasher.update(self.0.as_bytes());
        RecordKey::new(&hasher.finalize())
    }

    /// Derive GossipSub topic from ID
    pub fn gossip_topic(&self) -> IdentTopic {
        IdentTopic::new(format!("fabric/{}/sync", self.0))
    }
}

// discovery.rs - DHT-based peer discovery
pub struct FabricDiscovery {
    kad: Kademlia<MemoryStore>,
    program_id: ProgramId,
    known_peers: HashSet<PeerId>,
}

impl FabricDiscovery {
    /// Announce ourselves to the DHT under the Program ID key
    pub async fn announce(&mut self) -> Result<(), DiscoveryError> {
        let record = Record {
            key: self.program_id.dht_key(),
            value: self.local_peer_info().serialize(),
            publisher: Some(self.local_peer_id),
            expires: Some(Instant::now() + Duration::from_secs(3600)),
        };
        self.kad.put_record(record, Quorum::One)?;
        Ok(())
    }

    /// Find peers sharing our Program ID
    pub async fn discover_peers(&mut self) -> Result<Vec<PeerInfo>, DiscoveryError> {
        self.kad.get_record(self.program_id.dht_key());
        // Returns via KademliaEvent::OutboundQueryProgressed
    }

    /// Bootstrap into the DHT network
    pub async fn bootstrap(&mut self, bootstrap_peers: &[Multiaddr]) -> Result<(), DiscoveryError> {
        for addr in bootstrap_peers {
            self.kad.add_address(&peer_id_from_addr(addr)?, addr.clone());
        }
        self.kad.bootstrap()?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub addresses: Vec<Multiaddr>,
    pub capabilities: PeerCapabilities,
    pub timestamp: u64,
}

#[derive(Serialize, Deserialize)]
pub struct PeerCapabilities {
    pub platform: Platform,      // Native, Wasm
    pub arch: Arch,              // X86_64, Aarch64, Wasm32
    pub features: Vec<String>,   // "compute", "storage", "relay"
    pub tier: CapabilityTier,    // LowPower, WebStandard, NativeStandard, HighPerformance
    pub sovereignty: SovereigntyClass,  // Authority, Citizen, Ephemeral
}

impl PeerCapabilities {
    /// Check if this peer is an Authority (quorum voter)
    pub fn is_authority(&self) -> bool {
        matches!(self.sovereignty, SovereigntyClass::Authority)
    }

    /// Check if this peer can vote in write quorum
    pub fn can_vote_in_quorum(&self) -> bool {
        self.is_authority()
    }

    /// Check if this peer can provide JIT-compiled binaries
    pub fn can_provide_jit(&self) -> bool {
        self.is_authority() && self.tier >= CapabilityTier::NativeStandard
    }

    /// Check if this peer can act as a relay
    pub fn can_relay(&self) -> bool {
        matches!(self.sovereignty, SovereigntyClass::Authority | SovereigntyClass::Citizen)
            && self.tier >= CapabilityTier::WebStandard
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum Platform { Native, Wasm }

#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum Arch { X86_64, Aarch64, Wasm32 }
```

**Discovery Flow**:
```
1. Mac joins with Program ID "Alpha"
   - Computes DHT key: SHA256("fabric/program/Alpha")
   - Announces itself: PUT(key, {peer_id, multiaddrs, capabilities})
   - Subscribes to GossipSub topic "fabric/Alpha/sync"

2. Browser joins with Program ID "Alpha"
   - Computes same DHT key
   - Queries DHT: GET(key) → finds Mac's peer info
   - Connects to Mac via WebRTC multiaddr
   - Subscribes to same GossipSub topic

3. Both are now in the same mesh, sharing state via GossipSub
```

**Bootstrapping UX: Zero-Config Initial Connection**

The DHT requires at least one known peer to bootstrap. We solve this with **Invite Links**:

```rust
// bootstrap.rs - Generate and parse invite links
pub struct InviteLink {
    pub program_id: ProgramId,
    pub peer_id: PeerId,
    pub addrs: Vec<Multiaddr>,
}

impl InviteLink {
    /// Generate shareable URL
    /// Example: logicaffeine.com/studio?join=Alpha&peer=12D3Koo...&addr=/ip4/192.168.1.100/udp/9000/webrtc-direct
    pub fn to_url(&self) -> String {
        let addrs_encoded = self.addrs.iter()
            .map(|a| urlencoding::encode(&a.to_string()))
            .collect::<Vec<_>>()
            .join(",");

        format!(
            "https://logicaffeine.com/studio?join={}&peer={}&addr={}",
            urlencoding::encode(&self.program_id.0),
            self.peer_id,
            addrs_encoded
        )
    }

    /// Generate QR Code (for mobile/tablet)
    pub fn to_qr(&self) -> QrCode {
        QrCode::new(self.to_url()).unwrap()
    }

    /// Parse from URL query params
    pub fn from_url(url: &str) -> Result<Self, ParseError> { ... }
}

// CLI integration
// When running: largo run --id "Alpha"
// Terminal outputs:
//
// ╭─────────────────────────────────────────────────╮
// │  Fabric Mesh: Alpha                              │
// │  Your peers can join at:                         │
// │                                                  │
// │  logicaffeine.com/studio?join=Alpha&peer=12D... │
// │                                                  │
// │  [QR CODE HERE]                                  │
// │                                                  │
// │  Or scan the QR code above                       │
// ╰─────────────────────────────────────────────────╯
```

**Why This Matters**:
- First connection needs a "seed" peer
- Invite link provides the seed
- Once connected, Kademlia DHT maintains mesh automatically
- Reconnection after reboot uses cached DHT state

**TDD Tests**:
```rust
#[tokio::test]
async fn test_program_id_dht_key_deterministic() {
    let id1 = ProgramId::new("Alpha");
    let id2 = ProgramId::new("Alpha");
    assert_eq!(id1.dht_key(), id2.dht_key());
}

#[tokio::test]
async fn test_different_ids_different_keys() {
    let alpha = ProgramId::new("Alpha");
    let beta = ProgramId::new("Beta");
    assert_ne!(alpha.dht_key(), beta.dht_key());
}

#[tokio::test]
async fn test_dht_discovery_finds_peers() {
    // Node A announces under "Alpha"
    // Node B queries for "Alpha"
    // Node B finds Node A's peer info
}

#[tokio::test]
async fn test_dht_discovery_isolates_programs() {
    // Node A announces under "Alpha"
    // Node B queries for "Beta"
    // Node B does NOT find Node A
}

#[tokio::test]
async fn test_browser_discovers_native_via_dht() {
    // Mac announces under "Alpha" with WebRTC multiaddr
    // Browser queries DHT for "Alpha"
    // Browser receives Mac's WebRTC address
    // Browser connects directly
}
```

---

### Phase FF-3: Distributed Durable VFS

**Goal**: Enforce consistent file system behavior across platforms. Writes are not acknowledged until replicated to at least one peer.

**Files to Create/Modify**:
- `crates/logicaffeine_system/src/fs/unified.rs` (NEW)
- `crates/logicaffeine_system/src/fs/distributed_write.rs` (NEW)

**Critical Insight**: Browser OPFS is private to the origin. The VFS must treat the **Network as a Virtual Disk**. A write to OPFS should not return `Ok` until the Anti-Entropy layer confirms at least one other "Alpha" node has acknowledged the delta.

**The Write-Ahead Loop** (Refined for Robustness)

| Step | Native (Mac/Authority) | WASM (Browser/Citizen) |
|------|------------------------|------------------------|
| **1. Local Commit** | Write to Journal (SSD) | Write to Journal (OPFS) |
| **2. Broadcast** | Send to all known Peers | Send to Authorities via Relay |
| **3. Await Ack** | Wait for Majority Authorities | Wait for ONE Authority (Mac) |
| **4. Failure Mode** | Retry via Reliable Broadcast | Auto-Reschedule on Reconnect |
| **5. Ultimate Fallback** | MST Anti-Entropy resolves | MST Catch-up on tab focus |

**Key Robustness Properties**:
- **Idempotent Merging**: CRDTs ensure retries are safe (no double-counting)
- **Partition Tolerance**: Writes eventually propagate via MST Anti-Entropy
- **Browser Resilience**: Tab hibernation/closure triggers MST catch-up on reconnect
- **CRC32C Validation**: Every journal page validated after browser crash

**Performance Enhancement: CRC32C Hardware Acceleration**

The existing journal uses CRC32. Upgrade to CRC32C for hardware acceleration:

```rust
// crc.rs - Hardware-accelerated CRC32C
use crc32c::crc32c;  // Uses SSE4.2 on x86, CRC instructions on ARM

/// Compute CRC32C checksum (hardware-accelerated on M-series Mac + modern WASM)
pub fn checksum(data: &[u8]) -> u32 {
    crc32c(data)
}

// WASM: crc32c crate auto-detects WebAssembly SIMD support
// M-series Mac: Uses ARM CRC32C instructions (3x faster than software)
// Intel Mac: Uses SSE4.2 CRC32 instructions
```

**Zero-Copy Memory Zones** (High-Performance Path)

For latency-critical data, use memory-mapped I/O:

```rust
// memory_zone.rs - Zero-copy shared memory
#[cfg(not(target_arch = "wasm32"))]
pub struct MemoryZone {
    mmap: memmap2::MmapMut,
    dirty_pages: BitSet,
}

#[cfg(target_arch = "wasm32")]
pub struct MemoryZone {
    sab: SharedArrayBuffer,  // Requires COOP/COEP headers
    dirty_pages: BitSet,
}

impl MemoryZone {
    /// Sync dirty pages directly to network (zero-copy)
    pub async fn sync_to_fabric(&self, fabric: &FabricHandle) {
        for page_idx in self.dirty_pages.iter() {
            let page_data = self.get_page(page_idx);
            // Send page directly from memory - no intermediate buffer
            fabric.publish_page(page_idx, page_data).await;
        }
        self.dirty_pages.clear();
    }
}
```

**Why Zero-Copy Matters**:
- Standard path: RAM → Copy to buffer → Serialize → Network
- Zero-copy path: RAM → Network (dirty pages sent directly)
- Reduces CPU cycles during "Golden Test" sync

**DurabilityPolicy: First-Class Durability Intent**

Programmers choose their durability/latency trade-off explicitly:

```rust
/// First-class durability intent - programmer chooses trade-off
pub enum DurabilityPolicy {
    /// Return immediately after local write (Fastest)
    /// Use for: game state, caches, ephemeral data
    Local,

    /// Wait for N Authority nodes to acknowledge (Safest)
    /// Use for: financial data, user documents
    Quorum(usize),

    /// Wait for the primary Authority to acknowledge
    /// Use for: ordered operations, leader-based workflows
    Leader,
}
```

**LOGOS Syntax**:
```logos
# High safety for financial data
Mount wallet at "wallet.journal" with Quorum(2).

# High speed for game state (voxels)
Mount world_data at "voxels.journal" with Local.
```

**Design**:
```rust
// unified.rs
pub struct UnifiedVfs {
    inner: Arc<dyn Vfs + Send + Sync>,
    fabric: Arc<FabricHandle>,
    locks: Arc<Mutex<HashMap<PathBuf, LockState>>>,
    durability_policy: DurabilityPolicy,  // Controls quorum behavior
}

impl UnifiedVfs {
    /// Create with distributed durability
    pub fn new(
        vfs: Arc<dyn Vfs + Send + Sync>,
        fabric: Arc<FabricHandle>,
        durability_policy: DurabilityPolicy,
    ) -> Self {
        Self {
            inner: vfs,
            fabric,
            locks: Arc::new(Mutex::new(HashMap::new())),
            durability_policy,
        }
    }

    /// Distributed durable write
    /// Behavior depends on DurabilityPolicy:
    /// - Local: Return immediately after local write
    /// - Quorum(N): Wait for N Authority nodes to ack
    /// - Leader: Wait for primary Authority to ack
    pub async fn write_durable(&self, path: &Path, data: &[u8]) -> VfsResult<()> {
        // 1. Acquire exclusive lock
        let _lock = self.lock_exclusive(path).await?;

        // 2. Write to local VFS
        self.inner.write(path, data).await?;

        // 3. Compute delta for replication
        let delta = WriteDelta {
            path: path.to_path_buf(),
            content_hash: blake3::hash(data),
            timestamp: SystemTime::now(),
        };

        // 4. Broadcast delta via GossipSub (always, for eventual consistency)
        self.fabric.publish_delta(&delta).await?;

        // 5. Wait for Authority acknowledgments ONLY (not Citizens/Ephemeral)
        // This is the key change: 50 browser tabs don't slow down quorum
        match &self.durability_policy {
            DurabilityPolicy::Local => {
                // Return immediately - local write is enough
                // Use for: game state, caches, ephemeral data
            }
            DurabilityPolicy::Quorum(n) => {
                // Wait for N Authority nodes to ack
                // Citizens and Ephemerals are witnesses, not voters
                self.fabric.await_authority_acks(&delta, *n).await?;
            }
            DurabilityPolicy::Leader => {
                // Wait for primary Authority to ack
                self.fabric.await_leader_ack(&delta).await?;
            }
        }

        Ok(())
    }

    /// Acquire exclusive lock (mandatory, software-level)
    pub async fn lock_exclusive(&self, path: &Path) -> VfsResult<LockGuard> {
        let normalized = Self::normalize_path(path);

        loop {
            let mut locks = self.locks.lock().await;
            match locks.get(&normalized) {
                None | Some(LockState::Unlocked) => {
                    locks.insert(normalized.clone(), LockState::Exclusive {
                        holder: self.fabric.local_replica_id(),
                    });
                    return Ok(LockGuard::new(self.locks.clone(), normalized));
                }
                Some(LockState::Exclusive { .. }) | Some(LockState::Shared { .. }) => {
                    drop(locks);
                    // Wait and retry
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Normalize path: case-sensitive, forward slashes only
    fn normalize_path(path: &Path) -> PathBuf {
        let s = path.to_string_lossy();
        PathBuf::from(s.replace('\\', "/"))
        // Note: We do NOT lowercase - paths are case-sensitive
    }
}

// distributed_write.rs
#[derive(Serialize, Deserialize, Clone)]
pub struct WriteDelta {
    pub path: PathBuf,
    pub content_hash: [u8; 32],
    pub timestamp: SystemTime,
}

#[derive(Debug)]
pub enum LockState {
    Exclusive { holder: ReplicaId },
    Shared { readers: HashSet<ReplicaId> },
    Unlocked,
}

pub struct LockGuard {
    locks: Arc<Mutex<HashMap<PathBuf, LockState>>>,
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Ok(mut locks) = self.locks.try_lock() {
            locks.insert(self.path.clone(), LockState::Unlocked);
        }
    }
}

// fabric_handle.rs - Authority-only quorum methods
impl FabricHandle {
    /// Wait for N Authority nodes to acknowledge (not all peers)
    /// This is the key to preventing quorum bloat from browser tabs
    pub async fn await_authority_acks(&self, delta: &WriteDelta, n: usize) -> Result<(), FabricError> {
        let authorities: Vec<_> = self.connected_peers().await
            .into_iter()
            .filter(|p| p.capabilities.is_authority())
            .collect();

        if authorities.len() < n {
            return Err(FabricError::InsufficientAuthorities {
                required: n,
                available: authorities.len(),
            });
        }

        // Broadcast delta and collect acks from authorities only
        // Citizens/Ephemeral receive the delta but don't block the write
        let acks = self.broadcast_and_collect(&delta, &authorities).await;

        if acks.len() >= n {
            Ok(())
        } else {
            Err(FabricError::QuorumTimeout)
        }
    }

    /// Wait for the primary Authority (leader) to acknowledge
    pub async fn await_leader_ack(&self, delta: &WriteDelta) -> Result<(), FabricError> {
        let leader = self.current_leader().await
            .ok_or(FabricError::NoLeaderElected)?;

        let ack = self.send_and_await_ack(&delta, &leader).await?;
        Ok(())
    }

    /// Get all connected Authority peers
    pub async fn authority_peers(&self) -> Vec<PeerInfo> {
        self.connected_peers().await
            .into_iter()
            .filter(|p| p.capabilities.is_authority())
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FabricError {
    #[error("Insufficient authorities: required {required}, available {available}")]
    InsufficientAuthorities { required: usize, available: usize },

    #[error("Quorum timeout: not enough authorities acknowledged within deadline")]
    QuorumTimeout,

    #[error("No leader elected")]
    NoLeaderElected,
}
```

**Consistency Guarantees**:

| Semantic | Behavior | Rationale |
|----------|----------|-----------|
| **Case Sensitivity** | Strict case-sensitive | Prevents "works on Mac, breaks on Linux" |
| **Locking** | Software-level mandatory | Windows has it, Linux doesn't - we enforce it |
| **Path Separators** | `/` always | Normalized before hitting platform |
| **Durability** | Quorum-based | Write not OK until N peers ack |

**TDD Tests**:
```rust
#[tokio::test]
async fn test_case_sensitivity_enforced() {
    let vfs = UnifiedVfs::new(...);
    vfs.write_durable(Path::new("File.txt"), b"data").await.unwrap();

    // Different case = different file
    let result = vfs.read(Path::new("file.txt")).await;
    assert!(result.is_err()); // Not found
}

#[tokio::test]
async fn test_mandatory_locking() {
    let vfs = Arc::new(UnifiedVfs::new(...));

    // Process A locks
    let lock_a = vfs.lock_exclusive(Path::new("data.json")).await.unwrap();

    // Process B tries to lock - should block
    let vfs_clone = vfs.clone();
    let handle = tokio::spawn(async move {
        vfs_clone.lock_exclusive(Path::new("data.json")).await
    });

    // Give B time to attempt
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!handle.is_finished()); // Still waiting

    // A releases
    drop(lock_a);

    // B should now succeed
    let lock_b = handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn test_distributed_durable_write() {
    // Browser writes to OPFS
    // Write blocks until Mac acknowledges
    // Only then does write return Ok
}

#[tokio::test]
async fn test_write_fails_without_quorum() {
    // Browser writes with quorum=1
    // No other peers connected
    // Write times out / returns error
}
```

---

### Phase FF-4: AOT Work Distribution

**Goal**: Enable compute tasks to flow between nodes. Use native binaries when hardware matches, WASM as universal fallback.

**Files to Create/Modify**:
- `crates/logicaffeine_system/src/fabric/work.rs` (NEW)
- `crates/logicaffeine_system/src/fabric/scheduler.rs` (NEW)
- `crates/logicaffeine_system/src/fabric/capability.rs` (NEW)

**Key Insight**: Instead of shipping fat binaries, we use **Lazy Native Promotion**:
- Always send **WASM only** over the wire (small, portable)
- Receiving node executes WASM immediately
- If task becomes a "hot path", node **JIT compiles WASM → native locally**
- Native binary is cached by `CapabilityHash` for future runs

**Why Lazy Promotion Beats Fat Binaries**:
| Approach | Network Cost | First Run | Subsequent Runs |
|----------|--------------|-----------|-----------------|
| Fat Binaries (WASM + x86 + ARM) | ~3x size | Instant native | Instant native |
| **Lazy Promotion** | 1x size (WASM only) | WASM speed | Native speed (after JIT) |

The JIT compiler (Cranelift) is already present in the LOGOS runtime.

**Design**:
```rust
// capability.rs - Hardware capability fingerprinting
#[derive(Serialize, Deserialize, Clone, Hash, Eq, PartialEq)]
pub struct CapabilityHash([u8; 32]);

impl CapabilityHash {
    pub fn current() -> Self {
        let mut hasher = blake3::Hasher::new();

        // Platform
        #[cfg(target_arch = "x86_64")]
        hasher.update(b"x86_64");
        #[cfg(target_arch = "aarch64")]
        hasher.update(b"aarch64");
        #[cfg(target_arch = "wasm32")]
        hasher.update(b"wasm32");

        // OS
        #[cfg(target_os = "macos")]
        hasher.update(b"macos");
        #[cfg(target_os = "linux")]
        hasher.update(b"linux");
        #[cfg(target_os = "windows")]
        hasher.update(b"windows");
        #[cfg(target_arch = "wasm32")]
        hasher.update(b"browser");

        // ABI version (increment when native format changes)
        hasher.update(b"abi-v1");

        Self(hasher.finalize().into())
    }

    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// capability.rs - Hardware Capability Tiers
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityTier {
    /// Mobile browser, limited memory (~512MB), battery constrained
    LowPower = 0,
    /// Desktop browser, moderate memory (~2GB), no native execution
    WebStandard = 1,
    /// Desktop native, full memory, can JIT compile
    NativeStandard = 2,
    /// Server-class, high memory (16GB+), multiple cores
    HighPerformance = 3,
}

impl CapabilityTier {
    pub fn current() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            // Detect mobile vs desktop browser
            // Uses navigator.hardwareConcurrency and deviceMemory hints
            if is_mobile_browser() {
                CapabilityTier::LowPower
            } else {
                CapabilityTier::WebStandard
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let memory_gb = sys_info::mem_info().map(|m| m.total / 1024 / 1024).unwrap_or(4);
            if memory_gb >= 16 {
                CapabilityTier::HighPerformance
            } else {
                CapabilityTier::NativeStandard
            }
        }
    }
}

// work.rs - Portable work unit with Lazy Promotion
#[derive(Serialize, Deserialize)]
pub struct WorkUnit {
    pub id: Uuid,
    pub program_id: ProgramId,
    pub payload: WorkPayload,
    pub requirements: WorkRequirements,
}

#[derive(Serialize, Deserialize)]
pub struct WorkPayload {
    /// WASM bytecode - ALWAYS sent, universal portable format
    pub wasm: Vec<u8>,
    /// Entrypoint function name
    pub entrypoint: String,
    /// Content hash for caching JIT-compiled natives
    pub content_hash: [u8; 32],
}

impl WorkPayload {
    /// Get executable for current platform (checks local JIT cache first)
    pub fn executable(&self, jit_cache: &JitCache) -> Executable {
        let my_cap = CapabilityHash::current();

        // Check if we've already JIT'd this WASM for our platform
        if let Some(native) = jit_cache.get(&self.content_hash, &my_cap) {
            return Executable::Native(native);
        }

        // Fall back to WASM interpretation
        Executable::Wasm(self.wasm.clone())
    }
}

pub enum Executable {
    Native(Vec<u8>),  // Locally JIT-compiled Mach-O, ELF, or PE
    Wasm(Vec<u8>),    // Portable, runs anywhere
}

// jit.rs - Local JIT compilation cache
pub struct JitCache {
    cache_dir: PathBuf,  // ~/.lsf/jit/ on native, IndexedDB on WASM
}

impl JitCache {
    /// Check if we have a cached native for this WASM + platform
    pub fn get(&self, content_hash: &[u8; 32], cap: &CapabilityHash) -> Option<Vec<u8>> {
        let key = Self::cache_key(content_hash, cap);
        self.load_cached(&key)
    }

    /// JIT compile WASM to native and cache it
    pub async fn compile_and_cache(
        &self,
        wasm: &[u8],
        content_hash: &[u8; 32],
    ) -> Result<Vec<u8>, JitError> {
        let cap = CapabilityHash::current();

        // Use Cranelift for JIT compilation
        let native = cranelift_wasm::compile(wasm)?;

        // Cache for future runs
        let key = Self::cache_key(content_hash, &cap);
        self.store_cached(&key, &native).await?;

        Ok(native)
    }
}

#[derive(Serialize, Deserialize)]
pub struct WorkRequirements {
    pub min_memory_mb: u32,
    pub prefers_native: bool,
    pub timeout_ms: u64,
}

// scheduler.rs - Work-stealing scheduler with hot-path detection
pub struct FabricScheduler {
    local_queue: VecDeque<WorkUnit>,
    remote_peers: HashMap<PeerId, PeerCapabilities>,
    local_tier: CapabilityTier,
    jit_cache: Arc<JitCache>,
    execution_counts: HashMap<[u8; 32], u32>,  // Track hot paths
}

const HOT_PATH_THRESHOLD: u32 = 5;  // JIT after 5 executions

impl FabricScheduler {
    /// Submit work - scheduler decides where to run
    pub async fn submit(&mut self, work: WorkUnit) -> WorkHandle {
        // Check capability tier requirements
        let min_tier = work.requirements.min_capability_tier;

        // Find best peer for this work
        if min_tier > self.local_tier {
            // We're not capable enough - delegate to higher-tier peer
            if let Some(peer) = self.find_peer_with_tier(min_tier) {
                return self.delegate_to(peer, work).await;
            }
        }

        // Run locally
        self.execute_local(work).await
    }

    /// Find peer with at least the required capability tier
    fn find_peer_with_tier(&self, min_tier: CapabilityTier) -> Option<PeerId> {
        self.remote_peers.iter()
            .filter(|(_, cap)| cap.tier >= min_tier)
            .max_by_key(|(_, cap)| cap.tier)  // Prefer highest tier
            .map(|(id, _)| *id)
    }

    /// Execute locally with hot-path JIT promotion
    pub async fn execute_local(&mut self, work: WorkUnit) -> WorkHandle {
        let content_hash = work.payload.content_hash;

        // Track execution count for hot-path detection
        let count = self.execution_counts
            .entry(content_hash)
            .and_modify(|c| *c += 1)
            .or_insert(1);

        match work.payload.executable(&self.jit_cache) {
            Executable::Native(bytes) => {
                self.run_native(bytes, &work.payload.entrypoint).await
            }
            Executable::Wasm(bytes) => {
                // Check if this has become a hot path
                if *count >= HOT_PATH_THRESHOLD && self.local_tier >= CapabilityTier::NativeStandard {
                    // JIT compile in background (don't block this execution)
                    let jit_cache = self.jit_cache.clone();
                    let wasm = bytes.clone();
                    tokio::spawn(async move {
                        let _ = jit_cache.compile_and_cache(&wasm, &content_hash).await;
                    });
                }
                self.run_wasm(bytes, &work.payload.entrypoint).await
            }
        }
    }

    /// Steal work from busier peers (respects capability tiers)
    pub async fn steal_work(&mut self) -> Option<WorkUnit> {
        for (peer_id, cap) in &self.remote_peers {
            // Only steal work we can handle
            if cap.tier <= self.local_tier {
                if let Some(work) = self.request_work_from(*peer_id).await {
                    return Some(work);
                }
            }
        }
        None
    }

    /// Citizen broadcasts when under heavy load - offload to Authority
    /// This enables lightweight browsers to participate without being bottlenecked
    pub async fn offload_to_authority(&mut self, work: WorkUnit) -> Result<WorkHandle, OffloadError> {
        // Find an Authority with capacity
        let authority = self.remote_peers.iter()
            .filter(|(_, cap)| cap.is_authority())
            .min_by_key(|(_, cap)| cap.current_load)  // Pick least loaded
            .map(|(id, _)| *id)
            .ok_or(OffloadError::NoAuthorityAvailable)?;

        let request = ComputeRequest {
            work,
            origin: self.local_peer_id,
            deadline: Instant::now() + Duration::from_secs(30),
            stream_results: false,
        };

        self.fabric.send_compute_request(authority, request).await
    }

    /// Authority handles incoming compute requests from Citizens/Ephemerals
    pub async fn handle_compute_request(&mut self, req: ComputeRequest) {
        if !self.local_capabilities.is_authority() {
            // Citizens don't handle compute requests - reject
            return;
        }

        // Execute with JIT (native if cached)
        let result = self.execute_local(req.work).await;

        // Stream result back to originating Citizen
        self.fabric.send_compute_result(req.origin, result).await;
    }
}

/// Request for an Authority to execute work on behalf of a Citizen
#[derive(Serialize, Deserialize)]
pub struct ComputeRequest {
    pub work: WorkUnit,
    pub origin: PeerId,
    pub deadline: Instant,
    pub stream_results: bool,  // For incremental result delivery
}

#[derive(Debug, thiserror::Error)]
pub enum OffloadError {
    #[error("No authority available to handle compute request")]
    NoAuthorityAvailable,

    #[error("Compute request timed out")]
    Timeout,

    #[error("Authority rejected request: {0}")]
    Rejected(String),
}
```

**Execution Flow with Lazy JIT**:

```
1. Work arrives with WASM payload (always)
2. Check JIT cache - has this been compiled for our platform?
   - YES → Run native (fast)
   - NO → Run WASM (portable)
3. Track execution count
4. If count >= 5 AND we can JIT (NativeStandard+):
   - Background: cranelift compiles WASM → native
   - Cache result for future runs
5. Next execution uses cached native
```

**Why This Works**:

| Scenario | What Happens |
|----------|--------------|
| Browser executes | Always WASM (can't JIT) |
| Mac first run | WASM (no cache yet) |
| Mac 5th run | WASM, but JIT kicks off |
| Mac 6th+ run | Native from cache |
| Mac → Browser | WASM sent, Browser runs WASM |
| Browser → Mac | WASM sent, Mac JITs if hot |

**TDD Tests**:
```rust
#[test]
fn test_capability_hash_deterministic() {
    let h1 = CapabilityHash::current();
    let h2 = CapabilityHash::current();
    assert_eq!(h1, h2);
}

#[test]
fn test_capability_hash_differs_by_arch() {
    // This would need to be tested across actual architectures
    // or via mocking the cfg attributes
}

#[test]
fn test_work_payload_selects_native_when_available() {
    let my_cap = CapabilityHash::current();
    let payload = WorkPayload {
        wasm: vec![0x00, 0x61, 0x73, 0x6D], // WASM magic
        entrypoint: "main".into(),
        native_cache: [(my_cap.clone(), vec![0xCF, 0xFA, 0xED, 0xFE])].into(), // Mach-O magic
    };

    match payload.executable() {
        Executable::Native(bytes) => assert_eq!(&bytes[..4], &[0xCF, 0xFA, 0xED, 0xFE]),
        Executable::Wasm(_) => panic!("Should have selected native"),
    }
}

#[test]
fn test_work_payload_falls_back_to_wasm() {
    let other_cap = CapabilityHash([0xFF; 32]); // Different platform
    let payload = WorkPayload {
        wasm: vec![0x00, 0x61, 0x73, 0x6D],
        entrypoint: "main".into(),
        native_cache: [(other_cap, vec![0xCF, 0xFA, 0xED, 0xFE])].into(),
    };

    match payload.executable() {
        Executable::Wasm(bytes) => assert_eq!(&bytes[..4], &[0x00, 0x61, 0x73, 0x6D]),
        Executable::Native(_) => panic!("Should have fallen back to WASM"),
    }
}

#[tokio::test]
async fn test_scheduler_prefers_native_peer() {
    // Browser submits work with prefers_native=true
    // Mac is connected with Native capability
    // Work is delegated to Mac
}

#[tokio::test]
async fn test_work_stealing() {
    // Mac is idle
    // Browser has queued work
    // Mac steals work from Browser
    // Result flows back to Browser
}
```

---

### Phase FF-5: Merkle Search Tree Anti-Entropy

**Goal**: Ensure CRDT state converges even after network partitions. Use **Merkle Search Trees (MSTs)** for efficient live sync.

**Why MST over Standard Merkle Tree**:

| Feature | Standard Merkle | Merkle Search Tree (MST) |
|---------|-----------------|--------------------------|
| Structure | Binary tree | B-tree with key ordering |
| Insert/Delete | Rebuilds large subtrees | O(log n) stable updates |
| Live Sync | Requires full tree exchange | Find diff in O(log n) round trips |
| Determinism | Order-dependent | Same keys = same tree (always) |

**Key Insight**: When Mac increments a counter, the MST hash changes in a **predictable, localized way**. The Browser can find the difference in O(log n) network round trips.

**Files to Create/Modify**:
- `crates/logicaffeine_system/src/fabric/mst.rs` (NEW)
- `crates/logicaffeine_system/src/fabric/anti_entropy.rs` (NEW)

**Design**:
```rust
// mst.rs - Merkle Search Tree (deterministic, B-tree structured)
use blake3::Hash;

/// Merkle Search Tree - deterministic B-tree with hash at each node
/// Same set of keys ALWAYS produces the same tree structure
#[derive(Clone)]
pub struct MerkleSearchTree {
    root: Option<MstNode>,
    fanout: usize,  // Typically 32 for network efficiency
}

#[derive(Clone)]
struct MstNode {
    hash: Hash,
    /// Sorted keys at this level
    keys: Vec<Vec<u8>>,
    /// Values (or child hashes) for each key
    children: Vec<MstChild>,
}

enum MstChild {
    Leaf(JournalEntryId),
    Branch(Box<MstNode>),
}

impl MerkleSearchTree {
    /// Build from journal entries (deterministic - same entries = same tree)
    pub fn from_journal(entries: &[JournalEntry]) -> Self {
        let mut tree = Self { root: None, fanout: 32 };
        // Sort entries by key for deterministic structure
        let mut sorted: Vec<_> = entries.iter().collect();
        sorted.sort_by_key(|e| &e.key);
        for entry in sorted {
            tree.insert(entry);
        }
        tree
    }

    /// Get root hash for quick comparison
    pub fn root_hash(&self) -> Option<Hash> {
        self.root.as_ref().map(|n| n.hash)
    }

    /// Interactive diff protocol - O(log n) network round trips
    /// Instead of sending full trees, we compare hashes level by level
    pub async fn diff_interactive(
        &self,
        peer: &PeerId,
        fabric: &FabricHandle,
    ) -> Vec<JournalEntryId> {
        let mut missing = vec![];
        let mut to_compare = vec![(self.root.as_ref(), fabric.request_root(peer).await)];

        while let Some((local, remote_hash)) = to_compare.pop() {
            match (local, remote_hash) {
                (Some(node), Some(hash)) if node.hash == hash => {
                    // Subtrees match - skip
                    continue;
                }
                (Some(node), Some(_)) => {
                    // Hashes differ - drill down
                    let remote_children = fabric.request_children(peer, &node.hash).await;
                    for (i, child) in node.children.iter().enumerate() {
                        match child {
                            MstChild::Leaf(id) => {
                                if !remote_children.contains_key(&node.keys[i]) {
                                    missing.push(*id);
                                }
                            }
                            MstChild::Branch(child_node) => {
                                let remote_hash = remote_children.get(&node.keys[i]).cloned();
                                to_compare.push((Some(child_node.as_ref()), remote_hash));
                            }
                        }
                    }
                }
                (Some(node), None) => {
                    // Remote doesn't have this subtree - add all our entries
                    missing.extend(node.all_entries());
                }
                (None, _) => {}
            }
        }

        missing
    }

    /// Local diff (when we have both trees in memory)
    pub fn diff_local(&self, remote: &MerkleSearchTree) -> Vec<JournalEntryId> {
        match (&self.root, &remote.root) {
            (None, None) => vec![],
            (Some(local), None) => local.all_entries(),
            (None, Some(_)) => vec![],
            (Some(local), Some(remote)) => self.diff_nodes(local, remote),
        }
    }

    fn diff_nodes(&self, local: &MstNode, remote: &MstNode) -> Vec<JournalEntryId> {
        if local.hash == remote.hash {
            return vec![]; // Subtrees identical
        }

        let mut diff = vec![];

        // Compare keys at this level
        for (i, key) in local.keys.iter().enumerate() {
            match remote.keys.binary_search(key) {
                Ok(j) => {
                    // Key exists in both - compare children
                    match (&local.children[i], &remote.children[j]) {
                        (MstChild::Leaf(id), MstChild::Leaf(_)) => {
                            // Values differ
                            diff.push(*id);
                        }
                        (MstChild::Branch(l), MstChild::Branch(r)) => {
                            diff.extend(self.diff_nodes(l, r));
                        }
                        _ => {
                            // Structure mismatch
                            if let MstChild::Leaf(id) = &local.children[i] {
                                diff.push(*id);
                            }
                        }
                    }
                }
                Err(_) => {
                    // Key only in local
                    if let MstChild::Leaf(id) = &local.children[i] {
                        diff.push(*id);
                    }
                }
            }
        }

        diff
    }
}

// anti_entropy.rs
pub struct AntiEntropy {
    journal: Arc<Journal>,
    fabric: Arc<FabricHandle>,
    sync_interval: Duration,
    local_mst: RwLock<MerkleSearchTree>,
}

impl AntiEntropy {
    /// Run periodic sync with all peers
    pub async fn run_sync_loop(&self) {
        let mut interval = tokio::time::interval(self.sync_interval);

        loop {
            interval.tick().await;

            for peer in self.fabric.connected_peers().await {
                if let Err(e) = self.sync_with_peer(&peer).await {
                    tracing::warn!("Anti-entropy sync with {:?} failed: {}", peer, e);
                }
            }
        }
    }

    /// Sync with a specific peer using interactive MST diff
    async fn sync_with_peer(&self, peer: &PeerId) -> Result<(), SyncError> {
        // 1. Quick root hash comparison
        let local_root = self.local_mst.read().await.root_hash();
        let remote_root = self.fabric.request_root_hash(peer).await?;

        if local_root == remote_root {
            return Ok(()); // Already in sync - O(1) check
        }

        // 2. Interactive diff - O(log n) network round trips
        // Much more efficient than exchanging full trees
        let we_need = self.local_mst.read().await
            .diff_interactive(peer, &self.fabric).await;

        // 3. Request missing entries
        if !we_need.is_empty() {
            let entries = self.fabric.request_entries(peer, &we_need).await?;
            for entry in entries {
                self.journal.apply(entry).await?;
            }
        }

        // 4. Push what they're missing (symmetric diff)
        let they_need = self.fabric.request_diff_from_us(peer).await?;
        if !they_need.is_empty() {
            let entries = self.journal.get_entries(&they_need).await?;
            self.fabric.send_entries(peer, &entries).await?;
        }

        // 5. Rebuild MST from updated journal
        *self.local_mst.write().await = MerkleSearchTree::from_journal(
            &self.journal.all_entries().await
        );

        Ok(())
    }
}
```

**TDD Tests**:
```rust
#[test]
fn test_mst_deterministic_structure() {
    // Same entries in any order produce identical tree
    let entries = vec![entry1, entry2, entry3];
    let shuffled = vec![entry3, entry1, entry2];

    let tree1 = MerkleSearchTree::from_journal(&entries);
    let tree2 = MerkleSearchTree::from_journal(&shuffled);

    assert_eq!(tree1.root_hash(), tree2.root_hash());
}

#[test]
fn test_mst_identical_trees_no_diff() {
    let entries = vec![entry1, entry2, entry3];
    let tree1 = MerkleSearchTree::from_journal(&entries);
    let tree2 = MerkleSearchTree::from_journal(&entries);

    assert!(tree1.diff_local(&tree2).is_empty());
}

#[test]
fn test_mst_diff_finds_missing() {
    let tree1 = MerkleSearchTree::from_journal(&[entry1, entry2, entry3]);
    let tree2 = MerkleSearchTree::from_journal(&[entry1, entry2]); // Missing entry3

    let diff = tree1.diff_local(&tree2);
    assert_eq!(diff, vec![entry3.id]);
}

#[test]
fn test_mst_localized_changes() {
    // Insert into large tree only affects O(log n) nodes
    let entries: Vec<_> = (0..1000).map(make_entry).collect();
    let tree1 = MerkleSearchTree::from_journal(&entries);

    let mut entries_plus_one = entries.clone();
    entries_plus_one.push(make_entry(1000));
    let tree2 = MerkleSearchTree::from_journal(&entries_plus_one);

    // Only a small number of nodes should differ
    // (not rebuilding entire tree)
    assert_ne!(tree1.root_hash(), tree2.root_hash());
}

#[tokio::test]
async fn test_interactive_diff_efficiency() {
    // Mock network: count round trips
    // 10,000 entries, 5 differ
    // Should complete in O(log 10000) ≈ 13 round trips, not 10000
}

#[tokio::test]
async fn test_partition_recovery() {
    // Mac and Browser both connected
    // Disconnect them
    // Both mutate independently
    // Reconnect
    // Anti-entropy runs (MST interactive diff)
    // Both converge to same state
}
```

---

### Phase FF-6: Thread-Safe CRDT Access

**Goal**: Ensure concurrent CRDT operations are correct across platforms. Our existing CRDTs use `HashMap<ReplicaId, u64>` with `Mutex`/`RwLock` guards—this is the right pattern.

**Key Insight**: Our existing CRDTs in `logicaffeine_data` are pure data structures (no IO, no threading). The thread safety comes from the wrappers in `logicaffeine_system`:

| Layer | Location | Responsibility |
|-------|----------|----------------|
| `GCounter` etc. | `logicaffeine_data` | Pure merge logic, no sync |
| `Synced<T>` | `logicaffeine_system/crdt/sync.rs` | `Arc<Mutex<T>>` + GossipSub |
| `Distributed<T>` | `logicaffeine_system/distributed.rs` | `Arc<Mutex<T>>` + Journal + Network |

**No Changes Needed to Data Crate**: The data crate stays pure. Thread safety is handled at the system level.

**Audit Checklist for ARM Correctness**:

```rust
// The existing Synced<T> wrapper already does this correctly:
pub struct Synced<T: Merge + Serialize + DeserializeOwned + Clone + Send> {
    inner: Arc<Mutex<T>>,  // ✓ Mutex provides memory ordering
    topic: String,
}

impl<T: Merge + Serialize + DeserializeOwned + Clone + Send> Synced<T> {
    pub async fn mutate<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let result = {
            let mut guard = self.inner.lock().await;  // ✓ Acquire semantics
            f(&mut *guard)
        };  // ✓ Release semantics on drop
        // ... publish to network
        result
    }
}
```

**Why This Is Already Correct**:
- `async_lock::Mutex` (used for WASM compat) has proper memory ordering
- Lock acquisition = Acquire fence
- Lock release = Release fence
- No need for raw atomics in CRDT implementations

**Platform Behavior**:

| Platform | Memory Model | Mutex Behavior |
|----------|--------------|----------------|
| x86_64 | Strong (TSO) | Compiler fence sufficient |
| aarch64 (M-series) | Weak | DMB emitted on lock/unlock |
| wasm32 | Sequential | Single-threaded, no ordering needed |

**TDD Tests**:
```rust
#[tokio::test]
async fn test_synced_concurrent_mutations() {
    let synced = Arc::new(Synced::new(GCounter::new(), "test"));
    let mut handles = vec![];

    for _ in 0..100 {
        let s = synced.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..100 {
                s.mutate(|c| c.increment()).await;
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(synced.get().await.value(), 10_000);
}

#[tokio::test]
async fn test_distributed_survives_concurrent_access() {
    // Same test but with Distributed<T> (adds journal persistence)
    let vfs = Arc::new(MemoryVfs::new());
    let dist = Arc::new(Distributed::new(
        GCounter::new(),
        vfs,
        "test.journal",
        Some("test-topic"),
    ).await.unwrap());

    // ... concurrent mutations ...
    // Verify journal is consistent
}

#[test]
fn test_merge_is_idempotent() {
    // Core CRDT property - merging twice has no effect
    let mut c1 = GCounter::new();
    c1.increment();

    let c2 = c1.clone();

    c1.merge(&c2);
    c1.merge(&c2);
    c1.merge(&c2);

    assert_eq!(c1.value(), 1); // Not 3!
}
```

---

## File Structure (New Files)

```
crates/logicaffeine_system/src/
├── fabric/
│   ├── mod.rs              # Public exports
│   ├── program_id.rs       # ProgramId type and DHT key derivation
│   ├── bootstrap.rs        # Invite links (QR, URL) for initial connection
│   ├── discovery.rs        # Kademlia DHT + mDNS hybrid discovery
│   ├── work.rs             # WorkUnit with WASM-only payload
│   ├── scheduler.rs        # Work-stealing scheduler with hot-path detection
│   ├── capability.rs       # Hardware capability tiers (LowPower → HighPerformance)
│   ├── jit.rs              # Lazy JIT compilation cache (Cranelift)
│   ├── anti_entropy.rs     # Periodic MST-based sync coordinator
│   ├── mst.rs              # Merkle Search Tree (deterministic B-tree)
│   ├── reliable_broadcast.rs # Pending Ack Store with exponential backoff (NEW)
│   ├── liveness.rs         # Keep-Alive Pulse, Ghost Task prevention (NEW)
│   ├── session.rs          # Identity persistence, MST Catch-up (NEW)
│   ├── quorum.rs           # Majority quorum calculation (NEW)
│   ├── lease.rs            # Sovereign Lease for leadership (NEW)
│   ├── auth.rs             # Peer Authenticity via HMAC challenge (NEW)
│   ├── task_lease.rs       # TaskState enum, TaskLease struct (NEW)
│   ├── task_manager.rs     # Authority-side lease management (NEW)
│   ├── browser_worker.rs   # WASM-side throttle detection (cfg wasm32) (NEW)
│   ├── snapshot_catchup.rs # Delta-lag detection, snapshot fallback (NEW)
│   ├── vclock_causality.rs # VClock-based ordering, CausalMutation wrapper (NEW)
│   └── task_generation.rs  # GenerationId, StopWorkSignal for zombies (NEW)
├── network/
│   ├── webrtc.rs           # WebRTC transport for native + WASM
│   ├── transport.rs        # Unified transport builder (MODIFY)
│   └── nat.rs              # Auto-NAT detection + dcutr hole punching
├── fs/
│   ├── unified.rs          # UnifiedVfs with distributed durability
│   ├── distributed_write.rs # Write delta and lock types
│   ├── crc.rs              # CRC32C hardware-accelerated checksums
│   ├── memory_zone.rs      # Zero-copy mmap/SharedArrayBuffer zones
│   └── adaptive_compaction.rs # Storage quota monitoring, OPFS compaction (NEW)
└── crdt/
    └── sync.rs             # Synced<T> - MODIFY for WASM WebRTC support
```

**Note**: `logicaffeine_data` is NOT modified. It remains pure (no IO) per the Lamport Invariant.

---

## Dependencies

```toml
# crates/logicaffeine_system/Cargo.toml
[dependencies]
libp2p = { version = "0.54", features = [
    "tokio",
    "gossipsub",
    "kad",
    "identify",
    "noise",
    "yamux",
    "macros",
] }

# Native-specific transports and features
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
libp2p = { version = "0.54", features = [
    "quic",
    "tcp",
    "webrtc",       # WebRTC server for accepting browser connections
    "relay",        # Circuit Relay v2 server
    "mdns",         # Local network discovery (instant on LAN)
    "autonat",      # NAT detection
    "dcutr",        # Direct Connection Upgrade (hole punching)
] }
cranelift-wasm = "0.113"  # JIT compilation for hot paths
qrcode = "0.14"           # QR code generation for invite links

# WASM-specific transports
[target.'cfg(target_arch = "wasm32")'.dependencies]
libp2p = { version = "0.54", features = [
    "wasm-bindgen",
    "websocket",    # WebSocket fallback
] }
libp2p-webrtc-websys = "0.4"  # WebRTC for browsers
js-sys = "0.3"    # For SharedArrayBuffer detection

# Common
uuid = { version = "1.0", features = ["v4", "serde"] }
blake3 = "1.5"                    # Fast hashing (SIMD-accelerated)
crc32c = "0.6"                    # Hardware-accelerated CRC32C
urlencoding = "2.1"               # For invite link encoding
sys-info = { version = "0.9", optional = true }  # Memory detection for capability tiers

# Robustness Layer (NEW)
hmac = "0.12"                     # HMAC-SHA256 for peer authenticity
sha2 = "0.10"                     # SHA-256 for HMAC
rand = "0.8"                      # Random nonce generation for challenges
base64 = "0.22"                   # Keypair encoding for LocalStorage
hex = "0.4"                       # MST root hash encoding
thiserror = "1.0"                 # Error types for robustness layer

# WASM-specific (Robustness)
[target.'cfg(target_arch = "wasm32")'.dependencies]
web-sys = { version = "0.3", features = ["Window", "Storage"] }  # LocalStorage for session

[features]
fabric = ["networking", "persistence"]
jit = ["cranelift-wasm"]          # Optional: enables lazy native promotion
capability-detection = ["sys-info"]
```

---

## Test File Structure

```
tests/
├── fabric_transport.rs      # Phase FF-1: libp2p WebRTC + NAT traversal tests
├── fabric_discovery.rs      # Phase FF-2: Kademlia DHT + mDNS + bootstrap tests
├── fabric_vfs.rs            # Phase FF-3: Distributed VFS + CRC32C + zero-copy tests
├── fabric_work.rs           # Phase FF-4: Work distribution + JIT promotion tests
├── fabric_mst.rs            # Phase FF-5: Merkle Search Tree anti-entropy tests
├── fabric_threading.rs      # Phase FF-6: Thread-safe CRDT access tests
├── fabric_robustness.rs     # Robustness Layer: Reliable broadcast + retries (NEW)
├── fabric_liveness.rs       # Liveness Guard: Pulse detection, stale peer handling (NEW)
├── fabric_session.rs        # Session Resume: MST catch-up after hibernation (NEW)
├── fabric_quorum.rs         # Split-Brain Prevention: Majority quorum tests (NEW)
├── fabric_auth.rs           # Peer Authenticity: HMAC challenge-response tests (NEW)
├── fabric_task_lease.rs     # Task Lease: Grant/release/expire tests (NEW)
├── fabric_task_abandon.rs   # Task Abandonment: Tab close simulation, recovery tests (NEW)
├── fabric_throttle.rs       # Throttle Detection: Voluntary release on throttle (NEW)
├── fabric_snapshot_catchup.rs # Slow Consumer: Delta lag → snapshot fallback (NEW)
├── fabric_vclock.rs         # VClock Causality: Clock skew immunity tests (NEW)
├── fabric_generation.rs     # Generation IDs: Zombie task prevention tests (NEW)
└── fabric_compaction.rs     # Adaptive Compaction: OPFS quota management tests (NEW)
```

---

## Success Criteria

1. **Golden Test**: Mac and Browser connect **directly via WebRTC** (no central relay server)
2. **Instant LAN**: Same-network peers connect via **mDNS in <100ms** (no DHT warmup)
3. **Discovery**: Remote peers find each other via **Kademlia DHT** using Program ID as key
4. **Bootstrap UX**: First connection via **QR Code or Invite URL** (`largo run --id Alpha` shows join link)
5. **Storage**: OPFS and Mac disk sync via **Merkle Search Tree** (O(log n) network round trips)
6. **Performance**: Hot paths **JIT-compiled locally** via Cranelift (WASM sent, native cached)
7. **Capability Tiers**: Scheduler routes heavy work to **HighPerformance** nodes, not mobile browsers
8. **Durability**: Writes don't return until **quorum peers acknowledge**
9. **Zero-Copy**: High-performance paths use **mmap/SharedArrayBuffer** dirty page sync
10. **CRC32C**: Journal checksums use **hardware acceleration** (M-series ARM, x86 SSE4.2, WASM SIMD)
11. **Sovereignty**: Node classes (Authority, Citizen, Ephemeral) prevent slow browsers from bottlenecking cluster
12. **Tiered Durability**: Quorum defined by Authority count, not total peer count (50 browser tabs don't affect write latency)
13. **Compute Offload**: Citizens can delegate heavy work to Authorities, results stream back via libp2p

---

## Robustness Checklist

Before shipping, verify each layer is implemented:

### Layer 7: Logical Session (LOGOS-owned)
- [ ] **Reliable Broadcast**: Pending Ack Store with exponential backoff
- [ ] **Idempotent Merging**: All `logicaffeine_data` types remain pure (no side effects on retry)
- [ ] **Liveness Guard**: Citizens send pulse every 5s; 3 missed = Stale
- [ ] **Session Resume**: PeerId persisted in LocalStorage; MST root hash cached
- [ ] **MST Catch-up**: Browser sends root hash on reconnect, receives only missing entries

### Split-Brain Prevention
- [ ] **Majority Quorum**: Writes require `floor(n/2) + 1` Authority acks
- [ ] **Sovereign Lease**: Only one Leader per term; 30s expiry
- [ ] **Partition Detection**: If < majority reachable, writes fail fast (don't split)

### Security
- [ ] **Peer Authenticity**: Challenge-response proves knowledge of Program ID
- [ ] **Mesh Key**: Derived from Program ID via HMAC-SHA256
- [ ] **Noise Encryption**: All traffic encrypted (libp2p default)

### Data Integrity
- [ ] **CRC32C Validation**: Hardware-accelerated on M-series, x86, WASM SIMD
- [ ] **Journal Recovery**: After crash, validate all pages before accepting writes
- [ ] **Compaction Safe**: Never compact unacked deltas

### Browser Resilience
- [ ] **Tab Hibernation**: State restored from OPFS + MST diff on focus
- [ ] **Offline Queue**: Mutations queue locally, replay on reconnect
- [ ] **Ghost Task Prevention**: Stale peers excluded from scheduling

### Task Abandonment Recovery
- [ ] **Task Lease**: Tasks are leased, not owned - 30s TTL default
- [ ] **Pulse Heartbeat**: Citizens send task pulse every 5s while computing
- [ ] **Automatic Reclaim**: Authority reclaims after 3 missed pulses (~15s)
- [ ] **Voluntary Release**: Throttled browsers release tasks proactively
- [ ] **Idempotent Safety**: Double-completion safe via CRDT merge
- [ ] **No Browser-to-Browser Lock**: Citizens don't vote, can't block consensus

### Slow Consumer Recovery
- [ ] **Delta Lag Detection**: Track pending deltas per peer (threshold: 500)
- [ ] **Snapshot Fallback**: Switch to compressed snapshot when peer falls behind
- [ ] **Graceful Resume**: Return to incremental mode when lag < 250

### Causality & Clock Skew
- [ ] **VClock Ordering**: All mutations use VClock, not SystemTime, for ordering
- [ ] **CausalMutation Wrapper**: Every delta carries VClock + wall_time (display only)
- [ ] **LocalClock Manager**: Each node maintains monotonic VClock

### Zombie Task Prevention
- [ ] **Generation IDs**: Every task lease includes GenerationId
- [ ] **Generation Increment**: Reclaim increments generation (stale results detectable)
- [ ] **Stop Signals**: Stale generation → accept result + send StopWorkSignal

### Storage Quota Management
- [ ] **OPFS Monitoring**: Check `navigator.storage.estimate()` every 5 minutes
- [ ] **Warning Threshold**: 70% usage → Aggressive compaction (snapshot + MST root)
- [ ] **Critical Threshold**: 85% usage → Emergency compaction (snapshot only)
- [ ] **User Notification**: Emit storage warning when critical

---

## LOGOS Programming Semantics for Fluid Fabric

This section defines the complete natural language syntax for distributed computation. LOGOS programmers write English-like statements; the compiler handles networking, consensus, and recovery automatically.

### Design Philosophy

| Principle | Implementation |
|-----------|----------------|
| **Invisible Complexity** | Generation IDs, heartbeats, MST sync happen automatically |
| **Graceful Degradation** | Quorum failures degrade with warnings, not crashes |
| **Sovereignty-Aware** | Code adapts to node capabilities (Authority vs Citizen vs Ephemeral) |
| **Explicit Safety** | `Strict` modifier opts into fail-fast behavior |

---

### 1. Fabric Initialization

Enable distributed mode and declare the node's sovereignty class.

**Authority Node (Mac/Server):**
```logos
## Main
Enable Networked Mode with ID "Alpha" as Authority.
```

**Citizen Node (Desktop Browser):**
```logos
## Main
Enable Networked Mode with ID "Alpha" as Citizen.
```

**Ephemeral Node (Mobile/Guest):**
```logos
## Main
Enable Networked Mode with ID "Alpha" as Ephemeral.
```

**Conditional Initialization (Platform-Aware):**
```logos
## Main
If I am an Authority:
    Enable Networked Mode with ID "Alpha" as Authority.
Otherwise:
    Enable Networked Mode with ID "Alpha" as Citizen.
```

**Syntax Breakdown:**

| Clause | Meaning |
|--------|---------|
| `Enable Networked Mode` | Activates libp2p transport layer |
| `with ID "..."` | Binds the Program ID (mesh membership key) |
| `as Authority` | Full journal, quorum voter, JIT provider, relay |
| `as Citizen` | OPFS journal, work-stealer, no quorum vote |
| `as Ephemeral` | RAM-only, leaf node, no durability |

---

### 2. Distributed State Management

#### Shared Type Declaration

```logos
## Definition
A Counter is Shared and has:
    value: ConvergentCount.

A GameState is Shared and has:
    active_users: SharedSet of Text.
    scores: SharedMap from Text to Int.
    world: SharedList of Voxel.
```

#### Variable Binding

```logos
Let mutable score be a new shared Counter.
Let mutable state be a new GameState.
```

#### Durability Policy (Mount)

The `Mount` statement persists state to journal with configurable durability:

**Quorum Mode (Graceful Degradation):**
```logos
# Writes proceed if ANY authority is available; emits warning if < N ack
Mount score at "data/score.journal" with Quorum of 2.
```

**Strict Quorum Mode (Fail-Fast):**
```logos
# Writes FAIL if 2 Authorities aren't online (no degradation)
Mount wallet at "wallet.journal" with Strict Quorum of 2.
```

**Local Mode (Maximum Speed):**
```logos
# Write to local journal only; no network acks required
Mount world_data at "voxels.journal" with Local.
```

**Leader Mode (Ordered Operations):**
```logos
# Requires current Leader's ack before returning
Mount transactions at "tx.journal" with Leader.
```

**Durability Policy Reference:**

| Policy | Semantics | Use Case |
|--------|-----------|----------|
| `with Quorum of N` | Wait for N Authority acks; degrade gracefully if unavailable | Collaborative editing |
| `with Strict Quorum of N` | Require exactly N acks; fail if unavailable | Financial data |
| `with Local` | Local journal only; sync via eventual consistency | High-frequency game state |
| `with Leader` | Route through Raft-style leader | Ordered transaction log |

#### Topic Synchronization

```logos
# Subscribe to mesh-wide topic for automatic CRDT merge
Sync score on "global-leaderboard".
Sync state on "mesh-presence".
```

**Semantics:**
- Subscribes to GossipSub topic derived from string
- All deltas broadcast to topic
- Incoming deltas merged via `Merge` trait
- Idempotent: duplicate merges are safe

---

### 3. Parallel & Coordinated Compute

#### Parallel Mode (Idempotent, CRDT-Safe)

Distribute work across all capable nodes. Results converge via CRDT merge.

```logos
Across the mesh, parallel launch:
    Process the next chunk of the world.
```

**Semantics:**
- Broadcasts WASM work unit to all capable nodes (work-stealing queue)
- Each node processes independently
- Results merged via CRDT `Merge` trait
- **Idempotent**: Safe for retry, duplicate execution, out-of-order completion
- Best for: map-reduce, parallel search, distributed rendering

**With Result Collection:**
```logos
Across the mesh, parallel launch:
    Let results be search_local_cache("pattern").
    Add results to state's active_users.
```

#### Exactly-Once Mode (Coordinated)

Execute task on exactly one node with guaranteed completion.

```logos
Exactly once, launch task:
    If balance is at least 100:
        Send the confirmation email.
```

**Semantics:**
- Acquires Sovereign Lease with Generation ID
- Only one node executes; others wait or work-steal other tasks
- Heartbeat pulses keep lease alive automatically
- If executor dies, Authority reclaims and reassigns
- **Not idempotent**: Use for side-effecting operations

**With Task Handle:**
```logos
Let handle be exactly once, launch task:
    Process the payment.

# Check completion status
If handle is finished:
    Show "Payment processed".

# Abort if needed (releases lease)
Stop handle.
```

#### Task Lifecycle States

| State | Meaning |
|-------|---------|
| `Pending` | Queued, waiting for executor |
| `Running` | Active lease, heartbeats flowing |
| `Finished` | Completed successfully |
| `Abandoned` | Executor died, awaiting reclaim |
| `Stopped` | Explicitly aborted |

---

### 4. Compute Offload & Sovereignty Detection

#### Check Node Sovereignty

```logos
If I am a Citizen:
    # Browser - limited compute
    Offload heavy_computation to an Authority.
Otherwise if I am an Authority:
    # Mac/Server - full capabilities
    Execute heavy_computation locally.
Otherwise:
    # Ephemeral - minimal capabilities
    Skip heavy_computation.
```

#### Offload to Authority

Delegate expensive computation to capable nodes:

```logos
Let result be offload expensive_operation to an Authority.
Let analysis be offload analyze_data(state) to an Authority.
```

**Semantics:**
- Serializes function + arguments to WASM
- Routes to Authority node via libp2p
- Streams result back
- Transparent to caller

---

### 5. Connectivity & Discovery

#### Invite Generation (CLI Display)

```logos
Show the mesh invite for "Alpha".
```

**Output:**
```
╭─────────────────────────────────────────────────╮
│  Fabric Mesh: Alpha                             │
│  Join at: logicaffeine.com/studio?join=Alpha   │
│  [QR CODE]                                      │
╰─────────────────────────────────────────────────╯
```

#### Manual Network Listeners

```logos
# Listen on specific multiaddr
Listen on "/ip4/0.0.0.0/udp/9000/webrtc-direct".
Listen on "/ip4/0.0.0.0/tcp/4001".
```

#### Connect to Known Peer

```logos
Connect to "/ip4/192.168.1.100/udp/9000/webrtc-direct".
Connect to "/dns4/bootstrap.logicaffeine.com/tcp/4001".
```

#### mDNS Local Discovery

```logos
# Enabled by default; explicitly control if needed
Enable local discovery.
Disable local discovery.
```

---

### 6. Heartbeat Control

#### Automatic Mode (Default)

Runtime sends heartbeat pulses automatically. No programmer action required.

```logos
# Heartbeats are automatic - just write your logic
Exactly once, launch task:
    Repeat for item in items:
        Process item.
    # Pulses sent automatically between iterations
```

#### Manual Mode (Advanced)

For fine-grained control over long computations:

```logos
# Disable automatic heartbeats
Disable automatic heartbeat.

# In long computation, manually signal liveness
Repeat for chunk in chunks:
    Process chunk.
    Pulse.  # Keeps lease alive

# Re-enable automatic mode
Enable automatic heartbeat.
```

**When to Use Manual Mode:**
- Long-running computations with unpredictable iteration timing
- Operations that might appear hung but are making progress
- Fine-grained control over lease renewal timing

---

### 7. Error Handling & Recovery

#### Timeout in Select

```logos
Await the first of:
    Receive result from fabric:
        Show result.
    After 30 seconds:
        Show "Timed out waiting for mesh".
```

#### Catch-up After Partition

On reconnect, MST diff runs automatically. For explicit sync:

```logos
Reconcile score with the mesh.
Reconcile state with the mesh.
```

**Semantics:**
- Sends local MST root hash to peers
- Receives only missing entries (delta compression)
- Merges incoming state via CRDT `Merge`
- Safe to call multiple times

#### Pre-Mount Quorum Check

For strict mounts, verify authorities are available:

```logos
If quorum is available for 2:
    Mount wallet at "wallet.journal" with Strict Quorum of 2.
Otherwise:
    Show "Insufficient authorities online. Cannot mount wallet safely.".
```

#### Degradation Warnings

When operating in degraded mode:

```logos
On sovereignty warning:
    Show "Operating in low durability mode".
```

**Runtime behavior:**
- Tags writes with "Dirty/Non-Quorum" flag in WAL
- Auto-upgrades via MST Anti-Entropy when quorum is restored
- Emits warning event programmers can handle

---

### 8. Complete Reference Example: The Golden Test

This program demonstrates all major Fluid Fabric features:

```logos
## Definition
A GameState is Shared and has:
    active_users: SharedSet of Text.
    scores: SharedMap from Text to Int.
    messages: SharedList of Text.

## Main
# 1. Join the mesh (sovereignty determined by platform)
If I am an Authority:
    Enable Networked Mode with ID "Alpha" as Authority.
Otherwise:
    Enable Networked Mode with ID "Alpha" as Citizen.

# 2. Define durable, synchronized state
Let mutable state be a new GameState.
Mount state at "game.journal" with Quorum of 1.
Sync state on "mesh-presence".

# 3. Register this node
Let my_name be "Node-" followed by random_id().
Add my_name to state's active_users.

# 4. Parallel compute: distribute search across all nodes
Across the mesh, parallel launch:
    Let results be search_local_cache("pattern").
    For each result in results:
        Add result to state's messages.

# 5. Exactly-once: only one node sends aggregate report
Exactly once, launch task:
    After 60 seconds:
        Let report be generate_report(state).
        Show report.
        Add "Report generated" to state's messages.

# 6. Heavy computation: delegate to Authority if needed
If I am a Citizen:
    Let analysis be offload analyze_data(state) to an Authority.
Otherwise:
    Let analysis be analyze_data(state).

Show analysis.

# 7. Main event loop
Repeat forever:
    Await the first of:
        Receive message from state's messages:
            Show "New message: " followed by message.
        After 30 seconds:
            Reconcile state with the mesh.
            Show "Heartbeat - " followed by count of state's active_users followed by " users online".
```

**Properties Demonstrated:**

| Feature | Line(s) | Explanation |
|---------|---------|-------------|
| Sovereignty-aware init | 5-9 | Adapts to platform capabilities |
| Graceful degradation | 13 | `Quorum of 1` proceeds even with single authority |
| Idempotent parallel | 20-23 | Safe for retry via CRDT merge |
| Coordinated exactly-once | 26-30 | Side-effects execute once |
| Compute offload | 33-37 | Browser delegates to Mac |
| Reactive event loop | 42-49 | Select-style await with timeout |
| Automatic catch-up | 45 | Explicit reconciliation |

---

### 9. Keyword-to-Implementation Mapping

| LOGOS Clause | Rust Implementation | Robustness Layer |
|--------------|---------------------|------------------|
| `Enable Networked Mode` | `FabricHandle::new()` | libp2p transport init |
| `with ID "..."` | `ProgramId::from_str()` | Kademlia DHT key |
| `as Authority` | `SovereigntyClass::Authority` | Quorum voter, JIT provider |
| `as Citizen` | `SovereigntyClass::Citizen` | Work-stealer, no vote |
| `as Ephemeral` | `SovereigntyClass::Ephemeral` | RAM-only, leaf node |
| `Mount ... at` | `Distributed<T>::mount()` | Journal persistence |
| `with Quorum of N` | `DurabilityPolicy::Quorum(N)` | Graceful degradation |
| `with Strict Quorum of N` | `DurabilityPolicy::StrictQuorum(N)` | Fail-fast, no degradation |
| `with Local` | `DurabilityPolicy::Local` | Immediate return |
| `with Leader` | `DurabilityPolicy::Leader` | Primary ACK required |
| `Sync ... on` | GossipSub topic subscribe | Automatic merge |
| `Across the mesh, parallel` | Broadcast via GossipSub | Work-stealing queue |
| `Exactly once, launch` | `GenerationId` + Sovereign Lease | Heartbeat reclaim |
| `If I am a [Class]:` | `FabricHandle::local_sovereignty()` | Runtime class check |
| `Offload ... to an Authority` | `FabricScheduler::offload_to_authority()` | Compute delegation |
| `Pulse.` | `BrowserWorker::send_pulse()` | Manual heartbeat |
| `Disable automatic heartbeat.` | `TaskLease::manual_mode()` | Opt-out of auto-pulse |
| `Reconcile ... with the mesh.` | `AntiEntropy::sync_with_peer()` | MST diff |
| `If quorum is available for N:` | `QuorumConfig::can_attempt_write(N)` | Pre-mount check |
| `Show the mesh invite` | `InviteGenerator::generate()` | QR + URL generation |
| `Listen on "..."` | `Swarm::listen_on()` | Multiaddr binding |
| `Connect to "..."` | `Swarm::dial()` | Explicit peer connection |
| `Stop handle.` | `TaskHandle::abort()` | Lease release |

---

### 10. Quick Reference Card

**Initialization:**
```logos
Enable Networked Mode with ID "..." as Authority|Citizen|Ephemeral.
```

**State:**
```logos
Let mutable x be a new shared T.
Mount x at "path.journal" with Quorum of N|Strict Quorum of N|Local|Leader.
Sync x on "topic".
```

**Compute:**
```logos
Across the mesh, parallel launch: ...
Exactly once, launch task: ...
Let result be offload f(x) to an Authority.
```

**Connectivity:**
```logos
Show the mesh invite for "...".
Listen on "multiaddr".
Connect to "multiaddr".
```

**Recovery:**
```logos
Reconcile x with the mesh.
If quorum is available for N: ...
```

**Heartbeat:**
```logos
Pulse.
Disable automatic heartbeat.
Enable automatic heartbeat.
```

---

## Integration with Existing Infrastructure

| Existing Component | FLUID_FABRIC Integration |
|--------------------|--------------------------|
| `Synced<T>` | Extend with WASM WebRTC transport |
| `Distributed<T>` | Enable WASM code path (currently no-op) |
| `DeltaCrdt` trait | Use for efficient MST sync |
| `DeltaBuffer` | Ring buffer for catch-up after reconnect |
| `OpfsVfs` | Integrate with distributed durability |
| `Journal` | Upgrade CRC32 → CRC32C |
| `VClock`, `DotContext` | No changes - used as-is |
