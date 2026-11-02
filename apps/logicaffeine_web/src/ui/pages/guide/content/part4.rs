//! Part IV: Distributed Programming — sections: crdt, networking, security.

use super::{
    CodeExample, ExampleMode, Exercise, ExerciseKind, Misconception, RecapQuestion, Runnability,
    Section,
};

pub(super) const PART_4: &[Section] = &[
    Section {
        id: "crdt",
        number: 23,
        title: "Distributed Types (CRDTs)",
        part: "Part IV: Distributed Programming",
        content: r#"
### What are CRDTs?

CRDTs (Conflict-free Replicated Data Types) are data structures that can be replicated across multiple computers and merged without coordination. No matter what order updates arrive, the final state converges to the same result.

### Why CRDTs Matter

| Challenge | Traditional Approach | CRDT Approach |
|-----------|---------------------|---------------|
| Network partition | Data loss or conflicts | Automatic merge |
| Concurrent edits | Last-write-wins (data loss) | Semantic merge |
| Offline support | Sync conflicts | Seamless reconciliation |

### Shared Structs

Mark a struct as `Shared` to enable automatic merge support. The compiler generates a `merge` method that combines two instances.

### Built-in CRDT Types

| Type | Description | Operations |
|------|-------------|------------|
| `ConvergentCount` | Counter that only grows | `Increase` |
| `Tally` | Counter that grows and shrinks | `Increase`, `Decrease` |
| `LastWriteWins of T` | Register with timestamp-based conflict resolution | `Set` |
| `Divergent T` | Register that preserves concurrent values | `Set`, `Resolve` |
| `SharedSet of T` | Set with add/remove support | `Add`, `Remove`, `contains` |
| `SharedSequence of T` | Ordered list (RGA algorithm) | `Append`, `length of` |
| `CollaborativeSequence of T` | Text-optimized sequence (YATA) | `Append`, `length of` |
| `SharedMap from K to V` | Key-value CRDT map | `[]` access and assignment |

### ConvergentCount

A grow-only counter (G-Counter). Multiple replicas can increment independently, and when merged, the total reflects all increments. Useful for view counts, likes, or any monotonically increasing metric.

### Tally

A bidirectional counter (PN-Counter) that supports both increment and decrement. Unlike ConvergentCount, values can go up and down—even negative. Useful for scores, balances, and temperatures.

### LastWriteWins

A register that resolves conflicts by timestamp. The most recent write wins. Works with any type: `Text`, `Int`, `Bool`, etc.

### Divergent

A multi-value register that preserves all concurrent writes instead of silently picking a winner. When replicas write different values concurrently, both are kept until you explicitly `Resolve` the conflict. Useful for collaborative editing where conflicts should be visible.

### SharedSet

An observed-remove set (OR-Set) that supports both adding and removing elements. By default uses **add-wins** semantics: if one replica adds while another removes, the element stays.

**Configuring bias:**
- `SharedSet (AddWins) of T` — concurrent add beats remove (default)
- `SharedSet (RemoveWins) of T` — concurrent remove beats add

### SharedSequence

An ordered CRDT list using the RGA (Replicated Growable Array) algorithm. Elements maintain their order across replicas. Useful for ordered lists, chat history, and document lines.

### CollaborativeSequence

A text-optimized sequence using the YATA algorithm. Better conflict resolution for concurrent insertions at the same position. Ideal for collaborative text editing. Alternative syntax: `SharedSequence (YATA) of T`.

### SharedMap

A key-value CRDT map (OR-Map). Keys can be added and removed, and values are themselves CRDTs that merge recursively. Alternative syntax: `ORMap from K to V`.

### Merge Operations

Use `Merge source into target` to combine two CRDT instances. The target is updated in place with the merged state.

### Persistence

CRDTs can be persisted to disk using the `Persistent` type modifier and `Mount` statement. Data is stored in append-only journal files (`.lsf` format) with automatic compaction.

**The Persistent Type:**

`Persistent Counter` wraps a Shared struct with journaling. All mutations are durably recorded.

**The Mount Statement:**

`Mount [variable] at [path].`

or

`Let x be mounted at "path/to/data.lsf".`

This loads existing state from the journal file (if present) or creates a new one. Changes are automatically persisted.

### Network Synchronization

CRDTs earn their keep when synchronized across the network. Use `Sync` to subscribe a variable to a GossipSub topic.

**The Sync Statement:**

`Sync [variable] on [topic].`

- `variable` — A mutable variable containing a Shared struct
- `topic` — A string or variable naming the GossipSub topic

**What Sync Does:**
1. Subscribes to the topic for incoming messages
2. Spawns a background task to merge incoming updates
3. Broadcasts the full state after any mutation

### Persistence + Network

For the best of both worlds, combine `Persistent` types with `Sync`. The Distributed runtime ensures:
- Local changes are journaled before broadcast
- Remote updates are merged and persisted
- Data survives restarts

**Note:** In the playground, `Sync` runs in local single-node mode — the statement executes, but there are no real peers to gossip with. `Mount`/`Persistent` journaling needs the compiled runtime; that example is marked "(Compiled Only)".

> **Mental model:** Replicas are notebooks kept by different reporters: syncing is the
> union of FACTS — increments, adds, timestamped writes — never a fight over one pen.
> Convergence is guaranteed because a union doesn't care what order the facts arrive in.
> The limit: union needs facts designed to union — a plain overwrite degenerates to
> LastWriteWins, where the later clock wins and the earlier write is gone.
"#,
        hook: "Two phones edit the same shopping list in a tunnel, offline. When they reconnect, whose edits survive? With CRDTs the answer is BOTH — merge is mathematics, not mediation.",
        objectives: &[
            "write Shared structs with CRDT fields",
            "predict what Merge does to concurrent updates",
            "choose the right CRDT type per field",
        ],
        misconceptions: &[
            Misconception {
                id: "m-crdt-merge",
                expectation: "Two replicas both incremented the counter; merging keeps one and loses the other — someone must win.",
                reality: "ConvergentCount merges by combining per-replica contributions: 100 + 50 = 150, in any merge order. Every increment survives — that is the 'conflict-free' in CRDT, and it is why no coordinator is needed.",
                evidence: "crdt-merge",
            },
        ],
        examples: &[
            CodeExample {
                id: "crdt-basic",
                label: "Basic Shared Struct",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("10"),
                predict_first: false,
                code: r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Let c be a new Counter.
Increase c's points by 10.
Show c's points."#,
            },
            CodeExample {
                id: "crdt-merge",
                label: "Merging Replicas",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("150"),
                predict_first: true,
                code: r#"## Definition
A Stats is Shared and has:
    a views, which is ConvergentCount.

## Main
Let local be a new Stats.
Increase local's views by 100.

Let remote be a new Stats.
Increase remote's views by 50.

Merge remote into local.
Show local's views."#,
            },
            CodeExample {
                id: "crdt-lww",
                label: "Last-Write-Wins Register",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Alice\n100"),
                predict_first: false,
                code: r#"## Definition
A Profile is Shared and has:
    a name, which is LastWriteWins of Text.
    a score, which is LastWriteWins of Int.

## Main
Let p be a new Profile.
Set p's name to "Alice".
Set p's score to 100.
Show p's name.
Show p's score."#,
            },
            CodeExample {
                id: "crdt-sync-counter",
                label: "Synced Counter",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("100"),
                predict_first: false,
                code: r#"## Definition
A GameScore is Shared and has:
    a points, which is ConvergentCount.

## Main
Let mutable score be a new GameScore.
Sync score on "game-leaderboard".
Increase score's points by 100.
Show score's points."#,
            },
            CodeExample {
                id: "crdt-sync-profile",
                label: "Synced Profile",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("1"),
                predict_first: false,
                code: r#"## Definition
A Profile is Shared and has:
    a name, which is LastWriteWins of Text.
    a level, which is ConvergentCount.

## Main
Let mutable p be a new Profile.
Sync p on "player-data".
Increase p's level by 1.
Show p's level."#,
            },
            CodeExample {
                id: "crdt-persistent",
                label: "Persistent Counter",
                mode: ExampleMode::Imperative,
                runnability: Runnability::CompiledOnly {
                    reason: "the journal runtime (Mount/Persistent) only exists in compiled Rust",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Definition
A Counter is Shared and has:
    a value, which is ConvergentCount.

## Main
Let mutable c: Persistent Counter be mounted at "counter.lsf".
Increase c's value by 1.
Show c's value."#,
            },
            CodeExample {
                id: "crdt-tally",
                label: "Tally (Bidirectional Counter)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("70"),
                predict_first: false,
                code: r#"## Definition
A Score is Shared and has:
    a points, which is a Tally.

## Main
Let mutable s be a new Score.
Increase s's points by 100.
Decrease s's points by 30.
Show s's points."#,
            },
            CodeExample {
                id: "crdt-divergent",
                label: "Divergent (Multi-Value Register)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Draft\nFinal"),
                predict_first: false,
                code: r#"## Definition
A WikiPage is Shared and has:
    a title, which is Divergent Text.

## Main
Let mutable page be a new WikiPage.
Set page's title to "Draft".
Show page's title.
Resolve page's title to "Final".
Show page's title."#,
            },
            CodeExample {
                id: "crdt-sharedset",
                label: "SharedSet (Add/Remove Set)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Bob is invited\n1"),
                predict_first: false,
                code: r#"## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let mutable p be a new Party.
Add "Alice" to p's guests.
Add "Bob" to p's guests.
Remove "Alice" from p's guests.
If p's guests contains "Bob":
    Show "Bob is invited".
Show length of p's guests."#,
            },
            CodeExample {
                id: "crdt-sharedset-bias",
                label: "SharedSet with Bias",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("{safe}\n{spammer}"),
                predict_first: false,
                code: r#"## Definition
A Moderation is Shared and has:
    a tags, which is a SharedSet (AddWins) of Text.
    a blocked, which is a SharedSet (RemoveWins) of Text.

## Main
Let mutable m be a new Moderation.
Add "safe" to m's tags.
Add "spammer" to m's blocked.
Show m's tags.
Show m's blocked."#,
            },
            CodeExample {
                id: "crdt-sequence",
                label: "SharedSequence (Ordered List)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3"),
                predict_first: false,
                code: r#"## Definition
A Document is Shared and has:
    a lines, which is a SharedSequence of Text.

## Main
Let mutable doc be a new Document.
Append "Line 1" to doc's lines.
Append "Line 2" to doc's lines.
Append "Line 3" to doc's lines.
Show length of doc's lines."#,
            },
            CodeExample {
                id: "crdt-collaborative",
                label: "CollaborativeSequence (Text)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3"),
                predict_first: false,
                code: r#"## Definition
A Editor is Shared and has:
    a text, which is a CollaborativeSequence of Text.

## Main
Let mutable e be a new Editor.
Append "Hello" to e's text.
Append " " to e's text.
Append "World" to e's text.
Show length of e's text."#,
            },
            CodeExample {
                id: "crdt-sharedmap",
                label: "SharedMap (Key-Value CRDT)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("50"),
                predict_first: false,
                code: r#"## Definition
A Inventory is Shared and has:
    an items, which is a SharedMap from Text to Int.

## Main
Let mutable inv be a new Inventory.
Set inv's items["wood"] to 50.
Set inv's items["stone"] to 30.
Show inv's items["wood"]."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-crdt-hole",
                kind: ExerciseKind::Completion,
                concepts: &["Merge"],
                prompt: "Fill the hole so away's contributions fold into home — 45 survives.",
                mode: ExampleMode::Imperative,
                expected: "45",
                starter: "## Definition\nA Score is Shared and has:\n    a points, which is a Tally.\n\n## Main\nLet mutable home be a new Score.\nIncrease home's points by 30.\nLet mutable away be a new Score.\nIncrease away's points by 20.\nDecrease away's points by 5.\nMerge ??? into home.\nShow home's points.",
                hints: &[
                    "Merge reads `Merge <source> into <target>`.",
                    "The source replica is the one being folded in.",
                ],
                reference: "## Definition\nA Score is Shared and has:\n    a points, which is a Tally.\n\n## Main\nLet mutable home be a new Score.\nIncrease home's points by 30.\nLet mutable away be a new Score.\nIncrease away's points by 20.\nDecrease away's points by 5.\nMerge away into home.\nShow home's points.",
            },
            Exercise {
                id: "ex-crdt-tally",
                kind: ExerciseKind::Practice,
                concepts: &["Tally (PN-Counter)", "Merge"],
                prompt: "Home scored 30. Away scored 20, then lost 5. Merge away into home and print home's points.",
                mode: ExampleMode::Imperative,
                expected: "45",
                starter: "## Definition\nA Score is Shared and has:\n    a points, which is a Tally.\n\n## Main\n",
                hints: &[
                    "Two replicas, Increase/Decrease on each, then one Merge.",
                    "Every contribution survives: 30 + 20 − 5.",
                ],
                reference: "## Definition\nA Score is Shared and has:\n    a points, which is a Tally.\n\n## Main\nLet mutable home be a new Score.\nIncrease home's points by 30.\nLet mutable away be a new Score.\nIncrease away's points by 20.\nDecrease away's points by 5.\nMerge away into home.\nShow home's points.",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-crdt-c",
                question: "What does the 'conflict-free' in CRDT actually guarantee?",
                answer: "Merges converge to the same state in any order — concurrent updates combine instead of fighting.",
            },
            RecapQuestion {
                id: "rq-crdt-pick",
                question: "A like-counter, a set of invited guests, a profile name — which CRDT for each?",
                answer: "ConvergentCount; SharedSet; LastWriteWins of Text.",
            },
            RecapQuestion {
                id: "rq-crdt-divergent",
                question: "When would you reach for Divergent instead of LastWriteWins?",
                answer: "When concurrent writes must stay VISIBLE until someone explicitly Resolves them.",
            },
        ],
    },
    Section {
        id: "networking",
        number: 24,
        title: "P2P Networking",
        part: "Part IV: Distributed Programming",
        content: r#"
LOGOS includes built-in peer-to-peer networking primitives for building distributed applications.

**Note:** In the playground these examples run in offline single-node mode — `Listen`, `Send`, and mDNS setup execute locally with no real network. `Connect` and `PeerAgent` dial live peers, so they need a compiled program; those examples are marked "(Compiled Only)".

### Core Concepts

| Concept | Description |
|---------|-------------|
| **Address** | libp2p multiaddr format: `/ip4/127.0.0.1/tcp/8000` |
| **Listen** | Bind to an address to accept connections |
| **Connect** | Dial a peer at an address |
| **PeerAgent** | A handle to a remote peer |
| **Send** | Transmit a message to a peer |
| **Sync** | Subscribe a CRDT to a GossipSub topic |

### Portable Types

Messages sent over the network must be **Portable**. Mark your struct with `is Portable` to enable network serialization.

### Address Format

LOGOS uses libp2p multiaddresses:

| Address | Meaning |
|---------|---------|
| `/ip4/0.0.0.0/tcp/8000` | Listen on all interfaces, port 8000 |
| `/ip4/127.0.0.1/tcp/8000` | Localhost only, port 8000 |
| `/ip4/192.168.1.5/tcp/8000` | Specific IP address |
| `/ip4/0.0.0.0/tcp/0` | Listen on any available port |

### Automatic Peer Discovery (mDNS)

When you `Listen`, LOGOS automatically enables **mDNS** (multicast DNS) for local network peer discovery. Peers on the same LAN will discover each other without manual configuration.

- Works on WiFi networks, local development
- No configuration required — Listen is enough
- Peers are auto-connected when discovered

### GossipSub (Pub/Sub)

The `Sync` statement uses **GossipSub**, a pub/sub protocol for broadcasting messages to topic subscribers:

- Topics are strings (e.g., `"game-scores"`, `"player-data"`)
- When you mutate a synced variable, the full state broadcasts to all subscribers
- Incoming messages are automatically merged in the background
- Retry with exponential backoff: 1s, 2s, 4s, 8s, 16s

### File Transfer

For large file transfers, LOGOS provides **FileSipper**—a chunked transfer protocol:

| Component | Description |
|-----------|-------------|
| **FileSipper** | Zero-copy file chunker (1 MB default chunks) |
| **FileManifest** | Describes file: chunk count, SHA256 hashes |
| **FileChunk** | Individual chunk with verification hash |

This enables resumable transfers over unreliable networks.

### Building a P2P Application

1. Define Portable message types
2. Listen on an address (server)
3. Connect to peers (client)
4. Create PeerAgent handles
5. Send messages
6. Use `Sync` for automatic CRDT replication
"#,
        hook: "A LAN game finds its peers with zero configuration: Listen once, and mDNS introduces every machine on the network. The whole shape of a P2P app — addresses, handshakes, gossip — fits on this one page.",
        objectives: &[
            "name the P2P building blocks: Listen, Connect, PeerAgent, Send, Sync",
            "distinguish playground single-node mode from compiled live networking",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "network-listen",
                label: "Server: Listen for Connections",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Server listening on port 8000"),
                predict_first: false,
                code: r#"## Main
Listen on "/ip4/0.0.0.0/tcp/8000".
Show "Server listening on port 8000"."#,
            },
            CodeExample {
                id: "network-connect",
                label: "Client: Connect to Peer",
                mode: ExampleMode::Imperative,
                runnability: Runnability::CompiledOnly {
                    reason: "Connect dials a live relay — a real network transport the browser playground doesn't have",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Main
Let server_addr be "/ip4/127.0.0.1/tcp/8000".
Connect to server_addr.
Show "Connected to server"."#,
            },
            CodeExample {
                id: "network-peer-agent",
                label: "Creating a Remote Handle",
                mode: ExampleMode::Imperative,
                runnability: Runnability::CompiledOnly {
                    reason: "PeerAgent needs the compiled networking runtime",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Main
Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/8000".
Show "Remote peer handle created"."#,
            },
            CodeExample {
                id: "network-send-message",
                label: "Sending a Message",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Sending: Hello, peer!"),
                predict_first: false,
                code: r#"## Definition
A Greeting is Portable and has:
    a message (Text).

## Main
Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/8000".
Let msg be a new Greeting with message "Hello, peer!".
Show "Sending: " + msg's message.
Send msg to remote."#,
            },
            CodeExample {
                id: "network-distributed",
                label: "Persistent + Synced",
                mode: ExampleMode::Imperative,
                runnability: Runnability::CompiledOnly {
                    reason: "Mount and the live relay need the compiled runtime",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Definition
A Counter is Shared and has:
    a value, which is ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Let mutable c: Persistent Counter be mounted at "counter.lsf".
Sync c on "shared-counter".
Increase c's value by 1.
Show c's value."#,
            },
            CodeExample {
                id: "network-mdns",
                label: "Automatic Peer Discovery",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Listening... mDNS will auto-discover peers\nSynced to game-session topic"),
                predict_first: false,
                code: r#"## Definition
A GameState is Shared and has:
    a score, which is ConvergentCount.

## Main
Listen on "/ip4/0.0.0.0/tcp/0".
Show "Listening... mDNS will auto-discover peers".

Let mutable state be a new GameState.
Sync state on "game-session".
Show "Synced to game-session topic"."#,
            },
            CodeExample {
                id: "network-file-transfer",
                label: "File Transfer Pattern",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("File server ready\nSupports resumable chunked transfers"),
                predict_first: false,
                code: r#"## Definition
A FileRequest is Portable and has:
    a filename: Text.
    a chunk_index: Int.

A FileResponse is Portable and has:
    a data: Text.
    a is_last: Bool.

## Main
Listen on "/ip4/0.0.0.0/tcp/8000".
Show "File server ready".
Show "Supports resumable chunked transfers"."#,
            },
        ],
        exercises: &[],
        recap: &[
            RecapQuestion {
                id: "rq-net-portable",
                question: "What must a struct declare before it can cross the network?",
                answer: "`is Portable` — the network serialization contract.",
            },
            RecapQuestion {
                id: "rq-net-mdns",
                question: "What configuration does LAN peer discovery need?",
                answer: "None — Listen enables mDNS, and same-network peers find each other.",
            },
            RecapQuestion {
                id: "rq-net-badge",
                question: "Which networking statements run in the playground, and in what mode?",
                answer: "Listen, Send, Sync — offline single-node mode; Connect and PeerAgent dial live peers and need a compiled program.",
            },
        ],
    },
    Section {
        id: "security",
        number: 25,
        title: "Policy-Based Security",
        part: "Part IV: Distributed Programming",
        content: r#"
### Security in Natural Language

LOGOS lets you express security policies as natural English sentences. These compile into efficient runtime checks that can never be optimized away.

### Policy Blocks

Define security rules in `## Policy` blocks. Policies define **predicates** (conditions on a single entity) and **capabilities** (permissions involving multiple entities).

### Predicates

A predicate is a boolean condition on a subject:

`A User is admin if the user's role equals "admin".`

This generates a method `is_admin()` on the User type.

### Capabilities

A capability defines what a subject can do with an object:

`A User can publish the Document if the user is admin.`

This generates a method `can_publish(&Document)` on the User type.

### Check Statements

Use `Check` to enforce security at runtime. **Unlike `Assert`, Check statements are mandatory and can never be optimized away.**

| Statement | Debug Build | Release Build |
|-----------|-------------|---------------|
| `Assert` | Runs | Can be optimized out |
| `Check` | Runs | **Always runs** |

### Policy Composition

Policies can use `AND` and `OR` to combine conditions, and can reference other predicates.
"#,
        hook: "The intern's cleanup script deletes the CEO's document at 2 a.m. — through the code path everyone forgot to guard. A `Check that` line is the guard that cannot be optimized away, compiled from a policy you can read aloud.",
        objectives: &[
            "write policy predicates and capabilities in English",
            "distinguish Check from Assert by what survives a release build",
            "predict what a denied Check does to the program",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "security-predicate",
                label: "Simple Predicate",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Access granted"),
                predict_first: false,
                code: r#"## Definition
A User has:
    a role: Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "admin".
Check that u is admin.
Show "Access granted"."#,
            },
            CodeExample {
                id: "security-capability",
                label: "Capability with Object",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Edit permitted"),
                predict_first: false,
                code: r#"## Definition
A User has:
    a name: Text.
    a role: Text.

A Document has:
    an owner: Text.

## Policy
A User is admin if the user's role equals "admin".
A User can edit the Document if:
    The user is admin, OR
    The user's name equals the document's owner.

## Main
Let alice be a new User with name "Alice" and role "editor".
Let doc be a new Document with owner "Alice".
Check that alice can edit doc.
Show "Edit permitted"."#,
            },
            CodeExample {
                id: "security-denial",
                label: "A Denied Check (Fails on Purpose)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "Security Check Failed: u is admin",
                },
                expected_output: None,
                predict_first: true,
                code: r#"## Definition
A User has:
    a role: Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "guest".
Check that u is admin.
Show "unreachable"."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-sec-hole",
                kind: ExerciseKind::Completion,
                concepts: &["Check enforcement"],
                prompt: "Fill the hole with the predicate so the gate opens for the admin.",
                mode: ExampleMode::Imperative,
                expected: "Access granted",
                starter: "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"admin\".\nCheck that u is ???.\nShow \"Access granted\".",
                hints: &[
                    "The policy block already named the predicate.",
                    "Check lines read `Check that <subject> is <predicate>`.",
                ],
                reference: "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"admin\".\nCheck that u is admin.\nShow \"Access granted\".",
            },
            Exercise {
                id: "ex-sec-cap",
                kind: ExerciseKind::Practice,
                concepts: &["capabilities", "policy predicates"],
                prompt: "Write a `can delete` capability: admins, or the document's owner. Ada (a writer) owns the doc — Check she can delete it and print `delete permitted`.",
                mode: ExampleMode::Imperative,
                expected: "delete permitted",
                starter: "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\n",
                hints: &[
                    "A capability header names the actor, the verb, and the object, then an OR body on the following lines.",
                    "Ada passes through the owner leg — her name equals the document's owner.",
                ],
                reference: "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can delete the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet owner be a new User with name \"Ada\" and role \"writer\".\nLet doc be a new Document with owner \"Ada\".\nCheck that owner can delete doc.\nShow \"delete permitted\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-sec-check",
                question: "Check vs Assert — which survives a release build, and why does it matter?",
                answer: "Check always runs; Assert can be optimized out. Security gates must be Checks.",
            },
            RecapQuestion {
                id: "rq-sec-parts",
                question: "Predicate vs capability — what does each involve?",
                answer: "A predicate judges one entity (is admin); a capability relates two (can edit the Document).",
            },
            RecapQuestion {
                id: "rq-sec-deny",
                question: "What does a denied Check do?",
                answer: "Stops the program with a named security error — the line after it never runs.",
            },
        ],
    },
    Section {
        id: "part-4-checkpoint",
        number: 26,
        title: "Checkpoint: Distributed Programming",
        part: "Part IV: Distributed Programming",
        content: r#"
CRDTs, networking, and policies close the distributed story. Three challenges blend
them — two reach back to earlier parts on purpose — and the recall cards collect on
the part's promises.
"#,
        hook: "",
        objectives: &[
            "combine Shared types, merges, and policies in working programs",
            "diagnose which Part IV section to reread when a challenge stumps you",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[
            Exercise {
                id: "cp4-lww-profile",
                kind: ExerciseKind::Challenge,
                concepts: &["LastWriteWins register", "possessive field access"],
                prompt: "A Shared profile's name is set to \"Ada\", then \"Grace\". Print the surviving name.",
                mode: ExampleMode::Imperative,
                expected: "Grace",
                starter: "## Definition\n",
                hints: &[
                    "The field declares its CRDT with `which is` — a last-write-wins register of Text.",
                    "Later write wins — that is the register's whole policy.",
                ],
                reference: "## Definition\nA Profile is Shared and has:\n    a name, which is LastWriteWins of Text.\n\n## Main\nLet mutable p be a new Profile.\nSet p's name to \"Ada\".\nSet p's name to \"Grace\".\nShow p's name.",
            },
            Exercise {
                id: "cp4-views-merge",
                kind: ExerciseKind::Challenge,
                concepts: &["ConvergentCount (G-Counter)", "string interpolation"],
                prompt: "Local counted 100 views; remote counted 50. Merge and print `views: 150`.",
                mode: ExampleMode::Imperative,
                expected: "views: 150",
                starter: "## Definition\n",
                hints: &[
                    "Two Stats replicas, one Increase each, one Merge.",
                    "The interpolated Show can hold the possessive: {local's views}.",
                ],
                reference: "## Definition\nA Stats is Shared and has:\n    a views, which is ConvergentCount.\n\n## Main\nLet mutable local be a new Stats.\nIncrease local's views by 100.\nLet mutable remote be a new Stats.\nIncrease remote's views by 50.\nMerge remote into local.\nShow \"views: {local's views}\".",
            },
            Exercise {
                id: "cp4-policy-guard",
                kind: ExerciseKind::Challenge,
                concepts: &["Check enforcement", "struct definition"],
                prompt: "Define a User with a role, a policy making role \"admin\" an admin, Check it for an admin user, and print `granted`.",
                mode: ExampleMode::Imperative,
                expected: "granted",
                starter: "## Definition\n",
                hints: &[
                    "The predicate reads: a User is admin if their role field equals the admin string.",
                    "Check that u is admin — then the Show runs only if permitted.",
                ],
                reference: "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"admin\".\nCheck that u is admin.\nShow \"granted\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "cq4-merge",
                question: "Two replicas of a ConvergentCount hold 100 and 50. What does merging produce, and in what orders?",
                answer: "150 — in every order; merge is commutative union of contributions.",
            },
            RecapQuestion {
                id: "cq4-lww",
                question: "What decides the winner in LastWriteWins?",
                answer: "The timestamp — the later write survives, the earlier is gone.",
            },
            RecapQuestion {
                id: "cq4-addwins",
                question: "One replica adds a tag while another removes it. Who wins by default?",
                answer: "The add — SharedSet defaults to add-wins (RemoveWins is opt-in).",
            },
            RecapQuestion {
                id: "cq4-portable",
                question: "What marks a struct as network-serializable?",
                answer: "`is Portable`.",
            },
            RecapQuestion {
                id: "cq4-checkopt",
                question: "Why is Check the right gate for security, not Assert?",
                answer: "Check can never be optimized away; Assert can vanish in release builds.",
            },
            RecapQuestion {
                id: "cq4-sync",
                question: "What three things does `Sync x on \"topic\"` set up?",
                answer: "Topic subscription, background merging of incoming updates, and broadcast after local mutations.",
            },
        ],
    },
];
