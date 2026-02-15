# Logicaffeine

**Our Mission: Compile the universes information. No, not collect, compile, like code.**

[![CI](https://github.com/Brahmastra-Labs/logicaffeine/actions/workflows/test.yml/badge.svg)](https://github.com/Brahmastra-Labs/logicaffeine/actions/workflows/test.yml)
[![Version](https://img.shields.io/badge/version-0.8.12-blue)]()
[![Tests](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/Brahmastra-Labs/logicaffeine/badges/logicaffeine_test_count.json)](https://github.com/Brahmastra-Labs/logicaffeine/actions/workflows/test.yml)
[![License](https://img.shields.io/badge/license-BSL%201.1-blue)](LICENSE.md)

**[Try LOGOS Online →](https://logicaffeine.com/guide)**

---

## Why LOGOS?

**The gap between specification and implementation is where bugs hide.**

Natural language specs get mistranslated into code. Code comments drift from reality. Documentation lies.

LOGOS closes this gap: your specification IS your program.

| Traditional | LOGOS |
|-------------|-------|
| Write spec in English | Write spec in English |
| Translate to code manually | Compiler does it |
| Spec and code diverge | They're the same thing |
| Logic bugs hide in translation | Logic is explicit and verifiable |

---

Logicaffeine is a natural language compiler with two modes:

| Mode | Input | Output |
|------|-------|--------|
| **Imperative** | English programs | Executable Rust code |
| **Logic** | English sentences | First-Order Logic (∀, ∃, →, ∧) |

**Imperative Mode** — English to Rust:
```logos
## Main
Let x be 5.
If x is less than 10:
    Return true.
Return false.
```
↓
```rust
fn main() -> bool {
    let x = 5;
    if x < 10 { return true; }
    false
}
```

**Logic Mode** — English to First-Order Logic:
```
"Every woman loves a man."  →  ∀x(Woman(x) → ∃y(Man(y) ∧ Love(x,y)))
```

The programming language is called **LOGOS**.

**Logicaffeine and LOGOS are in early-access**
Please use Github issues for bugs/issues/feature-requests.

Important note to developers from the developer:

Yes, it's true that writing code in english is actually quite tedious. Languages have lots of great syntax sugar for good reasons. You might see things that seem silly, for example...

`Set p's x to 5`

You may rightfully think that it would be much simpler to write:

`Set p.x to 5`

Or even better:

`p.x = 5`

So what's the deal? The goal is to start with english, so that someone who doesn't know anything about programming can read and understand and perhaps even write some of their own code. Right now there is a mix of syntax-sugar in some places, and a lack thereof in others. Due to the way the parser works, it is designed for ambiguity, and thus in it's final form the goal would be for ALL of the above examples to compile down to the same AST and parse just fine. Initially, in some places syntax sugar has been used that will be expanded out. The sugar will still sprinkle just fine, but even things like function calls and such ought to be able to be written in english prose.

Perhaps:

`Call func with arguments x and y and z and set the result to x`

Or maybe:

`Set x to the return of func called with arguments x and y and z`

... or maybe both. You get the idea. Because we are doing code-gen to Rust, this can all turn into the same boring code. If you think this is crazy, just wait until you hear about this language called Typescript that "compiles" to Javascript.

**Why Rust?**
First, I love Rust. Second, Rust has some of the best tooling for bundling to WASM out of the languages I've worked. There are primitives that use OPFS and I've started writing a virtual file system that will enable the distributed types to be able to sync across the browser boundary, and provide a true local-first programming language.

**Should you use LOGOS in production?**
LOGOS is in early access. It is well tested but I wouldn't yet use it for things that matter. When LOGOS leaves early access it will mark the beginning of it being production ready. The language surface is also not yet stable and subject to change with feedback from users and user studies.

**Why not just use ChatGPT?**

LLMs are probabilistic—they guess. LOGOS is deterministic—it parses. When "every woman loves a man" has two meanings, GPT picks one. LOGOS returns both.

---

## Table of Contents

- [Why LOGOS?](#why-logos)
- [Quick Start](#quick-start)
- [The Grand Challenge: Mergesort](#the-grand-challenge-mergesort)
- [Beyond Hello World](#beyond-hello-world)
- [Imperative Mode](#imperative-mode)
  - [Type System](#type-system)
  - [Concurrency](#concurrency)
  - [Distributed Systems](#distributed-systems)
  - [Security](#security)
  - [Memory Zones](#memory-zones)
  - [Static Verification](#static-verification)
- [Logic Mode](#logic-mode)
  - [Quantifiers](#quantifiers)
  - [Connectives](#connectives)
  - [Causal Connectives](#causal-connectives)
  - [Modal Operators](#modal-operators)
  - [Tense & Aspect](#tense--aspect)
  - [Comparatives & Superlatives](#comparatives--superlatives)
  - [Units & Dimensionality](#units--dimensionality)
  - [Event Adjectives](#event-adjectives)
  - [Distributive vs Collective](#distributive-vs-collective)
  - [Axioms & Entailment](#axioms--entailment)
  - [Proof Engine](#proof-engine)
    - [Decision Procedure Tactics](#decision-procedure-tactics)
    - [Tactic Combinators](#tactic-combinators)
    - [Structural Proof Tactics](#structural-proof-tactics)
    - [Hint Database](#hint-database)
    - [Derivation Constructors Reference](#derivation-constructors-reference)
    - [Safety Guarantees](#safety-guarantees)
  - [Focus Particles](#focus-particles)
  - [Morphological Rules](#morphological-rules)
  - [Intensionality](#intensionality)
  - [Wh-Questions](#wh-questions)
  - [Scope Ambiguity](#scope-ambiguity)
  - [Parse Forests](#parse-forests)
  - [Garden Path Sentences](#garden-path-sentences)
  - [Discourse & Pronouns](#discourse--pronouns)
  - [Modal Subordination](#modal-subordination)
  - [Sessions & Multi-Turn Discourse](#sessions--multi-turn-discourse)
  - [Bridging Anaphora](#bridging-anaphora)
  - [Multi-Word Expressions](#multi-word-expressions--idioms)
  - [Category Shift](#category-shift)
  - [Reciprocals](#reciprocals)
  - [Ellipsis](#ellipsis)
  - [Topicalization](#topicalization)
  - [Passive Voice](#passive-voice)
  - [Respectively](#respectively)
  - [Control & Raising Verbs](#control--raising-verbs)
  - [Presupposition Triggers](#presupposition-triggers)
  - [Negative Polarity Items](#negative-polarity-items)
  - [Semantic Sorts & Metaphor](#semantic-sorts--metaphor-detection)
  - [Counterfactual Conditionals](#counterfactual-conditionals)
  - [Weather Verbs](#weather-verbs)
  - [Imperatives](#imperatives)
  - [Reflexive Binding](#reflexive-binding)
- [The CLI: largo](#the-cli-largo)
- [API Reference](#api-reference)
- [Architecture](#architecture)
- [Testing](#testing)
- [Glossary](#glossary)
- [Theoretical Foundations](#theoretical-foundations)
- [Further Reading](#further-reading)

---

## Quick Start

### Try Online

No installation required—[launch the interactive playground at logicaffeine.com/guide →](https://logicaffeine.com/guide)

### Local Development

```bash
# Build the project
cargo build

# Run the CLI
cargo run --features cli

# Launch the web IDE (requires dioxus-cli)
dx serve --bin logicaffeine_web

# Run tests (skips slow e2e by default)
cargo test -- --skip e2e

# Run all tests including e2e
cargo test
```

### Library Usage

```rust
use logicaffeine_language::{compile, compile_to_rust, compile_all_scopes};

// Logic Mode: English → First-Order Logic
let fol = compile("All men are mortal.").unwrap();
// → ∀x(Man(x) → Mortal(x))

// Imperative Mode: English → Rust
let rust = compile_to_rust("## Main\nLet x be 5.\nReturn x.").unwrap();
// → fn main() -> i64 { let x = 5; x }

// Get all scope readings for ambiguous sentences
let readings = compile_all_scopes("Every woman loves a man.").unwrap();
// → [surface scope, inverse scope]
```

---

## The Grand Challenge: Mergesort

This is a complete, recursive mergesort algorithm written in LOGOS. It compiles to working Rust and executes correctly.

```logos
## To Merge (left: Seq of Int) and (right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be 1.
    Let j be 1.
    Let n_left be length of left.
    Let n_right be length of right.

    While i is at most n_left and j is at most n_right:
        Let l_val be item i of left.
        Let r_val be item j of right.

        If l_val is less than r_val:
            Push l_val to result.
            Set i to i + 1.
        Otherwise:
            Push r_val to result.
            Set j to j + 1.

    While i is at most n_left:
        Let v be item i of left.
        Push v to result.
        Set i to i + 1.

    While j is at most n_right:
        Let v be item j of right.
        Push v to result.
        Set j to j + 1.

    Return result.

## To MergeSort (items: Seq of Int) -> Seq of Int:
    Let n be length of items.
    If n is less than 2:
        Return copy of items.

    Let mid be n / 2.
    Let left_slice be items 1 through mid.
    Let right_slice be items (mid + 1) through n.

    Let sorted_left be MergeSort(copy of left_slice).
    Let sorted_right be MergeSort(copy of right_slice).

    Return Merge(sorted_left, sorted_right).

## Main
    Let numbers be a new Seq of Int.
    Push 3 to numbers.
    Push 1 to numbers.
    Push 4 to numbers.
    Push 1 to numbers.
    Push 5 to numbers.

    Let sorted be MergeSort(numbers).
    Show sorted.
```

**What this demonstrates:**
- Recursive function definitions
- Generic collection types (`Seq of Int`)
- Compound conditions (`and`)
- Comparison operators (`is less than`, `is at most`)
- 1-based indexing (`item 1 of items`)
- Inclusive slicing (`items 1 through mid`)
- Collection operations (`Push`, `length of`, `copy of`)
- Full compilation to executable Rust

---

## Beyond Hello World

LOGOS goes far beyond basic logic and imperative code. See the sections below for:

- **[Concurrency](#concurrency)** — Structured concurrency, Go-like channels, actor system
- **[Distributed Systems](#distributed-systems)** — P2P networking, 6 CRDT types, GossipSub, persistence
- **[Security](#security)** — Mandatory guards, capability checks, policy blocks
- **[Memory Zones](#memory-zones)** — Arena allocation for high-performance scenarios

---

## Imperative Mode

Write executable programs in natural English. LOGOS compiles to Rust.

### Hello World

```logos
## Main
Show "Hello, World!".
```

Compiles to:

```rust
fn main() {
    println!("{:?}", "Hello, World!");
}
```

### Variables & Types

```logos
## Main
Let x be 5.                      # Immutable binding
Let name be "Alice".             # Text (string)
Let flag be true.                # Boolean
Let pi be 3.14159.               # Float

Set x to 10.                     # Mutation (requires prior Let)

Let y: Int be 42.                # Explicit type annotation
```

| LOGOS Type | Rust Type | Example |
|------------|-----------|---------|
| `Int` | `i64` | `Let x be 5.` |
| `Bool` | `bool` | `Let flag be true.` |
| `Text` | `String` | `Let name be "Alice".` |
| `Float` | `f64` | `Let pi be 3.14.` |
| `Seq of T` | `Vec<T>` | `Let items be [1, 2, 3].` |

### Ownership in English

LOGOS maps Rust's ownership model to intuitive English verbs, ensuring memory safety without a garbage collector.

| Keyword | Rust Equivalent | Semantics |
|---------|-----------------|-----------|
| `Give` | Move | Transfers ownership; original variable becomes invalid |
| `Show` | `&T` | Immutable borrow; original remains valid |
| `Lend` | `&mut T` | Mutable borrow (planned) |

```logos
Give data to process_function.    # Move semantics
Show config to display_settings.  # Immutable reference
```

### Control Flow

```logos
## Main
Let x be 5.

# Conditionals
If x is less than 10:
    Show "small".
Otherwise:
    Show "large".

# While loops
Let i be 1.
While i is at most 5:
    Show i.
    Set i to i + 1.

# For-each loops
Let items be [1, 2, 3].
Repeat for item in items:
    Show item.

# Early return
If x equals 0:
    Return false.
Return true.
```

**Comparison Operators:**

| English | Symbol | Meaning |
|---------|--------|---------|
| `is less than` | `<` | Less than |
| `is greater than` | `>` | Greater than |
| `is at most` | `<=` | Less than or equal |
| `is at least` | `>=` | Greater than or equal |
| `equals` / `is` | `==` | Equality |
| `is not` | `!=` | Inequality |

**Logical Operators:**

| English | Symbol | Example |
|---------|--------|---------|
| `and` | `&&` | `If x > 0 and y > 0:` |
| `or` | `\|\|` | `If x is 0 or y is 0:` |
| `not` | `!` | `If not flag:` |

### Collections

LOGOS uses **1-based indexing** because that's how humans count.

```logos
## Main
# List literals
Let items be [10, 20, 30, 40, 50].

# Access (1-indexed)
Let first be item 1 of items.     # → 10
Let third be item 3 of items.     # → 30

# Slicing (inclusive)
Let slice be items 2 through 4.   # → [20, 30, 40]

# Length
Let n be length of items.         # → 5

# Create empty collection
Let numbers be a new Seq of Int.

# Push (append)
Push 100 to numbers.
Push 200 to numbers.

# Pop (remove last)
Pop from numbers into last.       # last = 200

# Copy (deep clone)
Let backup be copy of items.
```

### Functions

Functions are defined with `## To` blocks:

```logos
## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## To greet (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main
    Let sum be add(3, 4).           # → 7
    Let fact be factorial(5).       # → 120
    Show greet("World").            # → "Hello, World!"
```

**Function Syntax:**
- `## To verb (param: Type) -> ReturnType:`
- Multiple parameters: `(a: Int) and (b: Int)`
- No parameters: `## To greet -> Text:`
- Void return: omit `-> Type`

### Structs

Define custom data types:

```logos
## Definition
A Point has:
    an x: Int.
    a y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Show p's x.                        # → 10
Show p's y.                        # → 20

Set p's x to 15.                   # Mutation
```

### Enums & Pattern Matching

```logos
## Definition
A Shape is either:
    a Circle with radius: Int.
    a Rectangle with width: Int and height: Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle:
            Return 3 * s's radius * s's radius.
        When Rectangle:
            Return s's width * s's height.

## Main
Let c be a new Circle with radius 5.
Let r be a new Rectangle with width 4 and height 6.

Show area(c).                      # → 75
Show area(r).                      # → 24
```

### Type System

#### Primitives

| Type | Description | Example |
|------|-------------|---------|
| `Int` | 64-bit integer | `5`, `-10`, `0` |
| `Bool` | Boolean | `true`, `false` |
| `Text` | String | `"hello"` |
| `Float` | 64-bit float | `3.14` |
| `Unit` | No value | (implicit) |

#### Collections

```logos
Let ints: Seq of Int be [1, 2, 3].
Let texts: Seq of Text be ["a", "b", "c"].
Let nested: Seq of (Seq of Int) be [[1, 2], [3, 4]].
```

#### User-Defined Types

**Structs (Product Types):**
```logos
## Definition
A Person has:
    a name: Text.
    an age: Int.
```

**Enums (Sum Types):**
```logos
## Definition
A Result is either:
    an Ok with value: Int.
    an Error with message: Text.
```

**Recursive Inductive Types:**

LOGOS supports recursive data structures through inductive types. These are automatically boxed for memory safety:

```logos
## A Peano is either:
    A Zero.
    A Succ with pred Peano.

## Main
Let z be a new Zero.
Let n1 be a new Succ with pred z.
Let n2 be a new Succ with pred n1.

Inspect n2:
    When Zero: Show "zero".
    When Succ (p):
        Inspect p:
            When Zero: Show "one".
            When Succ (pp): Show "two or more".
```

This compiles to efficient Rust with `Box<T>` for recursive fields:
```rust
pub enum Peano {
    Zero,
    Succ { pred: Box<Peano> },
}
```

Recursive types enable classic functional data structures like linked lists, binary trees, and natural number representations.

#### Generics

```logos
## Definition
A Box of [T] has:
    a contents: T.

## Main
Let int_box be a new Box of Int with contents 42.
Let text_box be a new Box of Text with contents "hello".
```

#### Refinement Types

```logos
Let positive: Int where it > 0 be 5.
```

### Concurrency

LOGOS provides multiple concurrency models for different use cases.

#### Structured Concurrency

For I/O-bound operations, use `Attempt all of the following` for concurrent execution:

```logos
## Main
Attempt all of the following:
    Fetch "https://api.example.com/users".
    Fetch "https://api.example.com/posts".
    Fetch "https://api.example.com/comments".
```

For CPU-bound parallel computation:

```logos
## Main
Simultaneously:
    Process chunk 1.
    Process chunk 2.
    Process chunk 3.
```

#### Go-like Channels

LOGOS supports Go-style channels (called Pipes) for CSP-style concurrency:

```logos
## Main
Let ch be a new Pipe of Int.

Launch task:
    Send 42 into ch.
    Send 100 into ch.

Receive from ch into first.
Receive from ch into second.

Show first.    # → 42
Show second.   # → 100
```

**Channel Operations:**

| Operation | Syntax | Behavior |
|-----------|--------|----------|
| Create | `Let ch be a new Pipe of T.` | Unbuffered channel |
| Send | `Send value into ch.` | Blocks until received |
| Receive | `Receive from ch into var.` | Blocks until sent |
| Try Send | `Try to send value into ch.` | Non-blocking |
| Try Receive | `Try to receive from ch into var.` | Non-blocking |

**Select Statement:**

Wait on multiple channels:

```logos
## Main
Let ch1 be a new Pipe of Int.
Let ch2 be a new Pipe of Text.

Select first of the following:
    Receive from ch1 into num:
        Show num.
    Receive from ch2 into msg:
        Show msg.
    After 5 seconds:
        Show "timeout".
```

**Task Control:**

```logos
## Main
Let worker be launch task:
    # Long-running work...

Stop worker.    # Cancel the task
```

#### Agent System

LOGOS includes an actor-model agent system for message-passing concurrency:

```logos
## Main
Spawn worker as DataProcessor.
Send "process" to worker.
Await response from worker into result.
Show result.
```

Agents are lightweight, isolated processes that communicate only via messages—no shared state.

### Distributed Systems

LOGOS includes a P2P mesh networking layer with native CRDT support.

#### P2P Networking

```logos
## Main
Listen on "/ip4/0.0.0.0/tcp/8080".
Connect to "/ip4/192.168.1.5/tcp/8080".
```

Compiles to production-grade Rust with libp2p, GossipSub, and CRDT merge semantics.

#### CRDT Types

CRDTs (Conflict-free Replicated Data Types) guarantee eventual consistency without coordination. LOGOS provides six built-in CRDT types:

**GCounter (Grow-only Counter):**
```logos
Let votes be a new shared GCounter.
Increment votes.
Let merged be merge(votes, remote_votes).
```

**PNCounter (Tally - can increment and decrement):**
```logos
Let score be a new Tally.
Increase score by 10.
Decrease score by 3.
# Net value: 7
```

**ORSet (SharedSet - add/remove with bias):**
```logos
Let tags be a new SharedSet.
Add "important" to tags.
Remove "draft" from tags.

# With conflict resolution bias:
Let items be a new SharedSet with AddWins.
Let other be a new SharedSet with RemoveWins.
```

**RGA (SharedSequence - collaborative text):**
```logos
Let doc be a new SharedSequence.
Append "Hello" to doc.
Append " World" to doc.

# Or with YATA algorithm:
Let text be a new CollaborativeSequence using YATA.
```

**ORMap (SharedMap - key-value CRDT):**
```logos
Let config be a new SharedMap.
Set config["theme"] to "dark".
Set config["fontSize"] to "14".
```

**MVRegister (Divergent - conflict resolution):**
```logos
Let setting be a new Divergent.
Set setting to "value1".
# On conflict, access all concurrent values:
Let all_values be values of setting.
Resolve setting with first of all_values.
```

| CRDT Type | Use Case | Conflict Resolution |
|-----------|----------|---------------------|
| GCounter | Vote counts, views | Sum of all replicas |
| Tally | Scores, balances | Sum with negative support |
| SharedSet | Tags, memberships | Add-wins or remove-wins |
| SharedSequence | Collaborative text | Causal ordering |
| SharedMap | Distributed config | Last-writer-wins per key |
| Divergent | Settings, flags | Manual resolution |

#### GossipSub

The `Sync` statement subscribes a CRDT to a GossipSub topic for automatic network synchronization:

```logos
## Definition
A Score is Shared and has:
    a points: ConvergentCount.

## Main
Let mutable score be a new Score.
Sync score on "leaderboard".      # Subscribe to topic
Increase score's points by 10.    # Broadcasts to all peers
```

**How it works:**

| Step | What Happens |
|------|--------------|
| Subscribe | Awaits mesh membership before returning |
| Background Task | Spawns auto-merge loop for incoming messages |
| Local Mutation | Broadcasts full state to all topic subscribers |
| Remote Message | Automatically merged into local state |

Topics are arbitrary strings—use any naming scheme: `"game-scores"`, `"room-123"`, `"player-data"`.

#### Persistence

Mount CRDTs to disk for durability across restarts:

```logos
## Main
Mount counter at "data/counter.journal".
Increment counter.
# Survives restarts via journal replay
```

**Journal Format:**

The journal uses a WAL (Write-Ahead Log) approach with two entry types:
- **Delta** — Incremental changes appended on each mutation
- **Snapshot** — Full state checkpoint

**Compaction:** After ~1000 deltas, the system writes a snapshot and truncates old entries. This bounds journal size while preserving crash-safety. Replay on startup applies entries in order—snapshots replace state, deltas merge into it.

#### Distributed<T> — Persistence + Network

**The problem:** `Sync` alone stores remote updates in RAM—they're lost on restart.

**The solution:** Combine `Mount` and `Sync` to journal both local AND remote updates:

```logos
## Definition
A Game is Shared and has:
    a players: ConvergentCount.

## Main
Mount game at "game.journal".   # Disk persistence
Sync game on "active-games".    # Network sync

Increase game's players by 1.
# Local changes: RAM → Journal → Network
# Remote updates: Network → RAM → Journal (crash-safe!)
```

**Data Flow Comparison:**

| Mode | Local Mutation | Remote Update | Crash-Safe |
|------|----------------|---------------|------------|
| `Sync` only | RAM → Network | Network → RAM | No |
| `Mount` only | RAM → Journal | N/A | Yes |
| `Mount` + `Sync` | RAM → Journal → Network | Network → RAM → Journal | Yes |

### Security

LOGOS includes a declarative security system with mandatory runtime enforcement.

#### Policy Blocks

Define security rules in `## Policy` blocks using two constructs:

**Predicates** — Boolean conditions on a single entity:
```logos
## Policy
A User is admin if the user's role equals "admin".
A User is active if the user's status equals "enabled".
```

**Capabilities** — Permissions involving multiple entities:
```logos
## Policy
A User can publish the Document if:
    The user is admin, OR
    The user equals the document's owner.

A User can delete the Record if the user is admin.
```

#### Complete Example

```logos
## Definition
A User has:
    a role: Text.
    a status: Text.

A Document has:
    an owner: User.
    a title: Text.

## Policy
A User is admin if the user's role equals "admin".
A User is active if the user's status equals "enabled".

A User can publish the Document if:
    The user is admin, OR
    The user equals the document's owner.

A User can view the Document if the user is active.

## Main
Let alice be a new User with role "editor" and status "enabled".
Let doc be a new Document with owner alice and title "Report".

Check that alice can publish doc.  # Passes - alice is owner
Check that alice can view doc.     # Passes - alice is active
```

#### Check vs Assert

| Statement | Behavior | Use Case |
|-----------|----------|----------|
| `Check that...` | **Mandatory** - never optimized away | Security-critical guards |
| `Assert that...` | Debug-only - stripped in release | Development assertions |

```logos
## Main
Check that user is admin.    # Always enforced, even in release
Assert that x > 0.           # Only checked in debug builds
```

Check failures halt execution with a descriptive error including the source location and policy description.

### Memory Zones

LOGOS supports arena-based memory allocation for high-performance scenarios.

#### Basic Zones

```logos
## Main
Inside a new zone called "Scratch":
    Let temp be [1, 2, 3, 4, 5].
    Let processed be process(temp).
    # All allocations freed when zone exits
```

#### Heap Zones with Capacity

```logos
## Main
Inside a new zone called "WorkBuffer" with size 1024:
    # Pre-allocated buffer of 1024 bytes
    Let data be process_chunk(input).
```

#### Memory-Mapped Zones

```logos
## Main
Inside a new zone mapped from "large-file.bin":
    Let chunk be data at offset 0 with length 4096.
    Process chunk.
```

Zones provide deterministic deallocation—all memory in a zone is freed at once when the zone exits, avoiding GC pauses.

### Static Verification

LOGOS includes optional Z3-based static verification that can prove assertions at compile time. This is a premium feature requiring a Pro, Premium, Lifetime, or Enterprise license.

#### Requirements

**Install Z3** (required for the verification feature):

```bash
# macOS
brew install z3

# Ubuntu/Debian
apt install libz3-dev

# Windows
# Download from https://github.com/Z3Prover/z3/releases
```

**Set environment variables** (macOS with Homebrew):

```bash
export Z3_SYS_Z3_HEADER=/opt/homebrew/include/z3.h
export BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/include"
export LIBRARY_PATH="/opt/homebrew/lib"
```

#### Building with Verification

```bash
# Build with verification support
cargo build --features verification

# Build CLI with verification
cargo build --features cli,verification
```

#### Usage

```bash
# Verify a project (requires license)
largo verify --license sub_xxx

# Build with verification
largo build --verify --license sub_xxx

# Or use environment variable
export LOGOS_LICENSE=sub_xxx
largo build --verify
```

#### What It Verifies

The verifier uses the Z3 SMT solver to check:

- **Tautologies**: Assertions that are always true
- **Contradictions**: Assertions that can never be true
- **Integer bounds**: Constraints like `x > 5` given known values
- **Refinement types**: Values satisfy their declared predicates

When verification fails, you get **Socratic error messages** with counter-examples:

```
Verification failed.
You asserted 'x is greater than 10', but x could be 5.
```

#### License Tiers

| Plan | Verification |
|------|--------------|
| Free | No |
| Supporter | No |
| Pro | Yes |
| Premium | Yes |
| Lifetime | Yes |
| Enterprise | Yes |

Get a license at [logicaffeine.com/pricing](https://logicaffeine.com/pricing).

---

## Logic Mode

Compile English sentences to First-Order Logic with full semantic analysis. The vocabulary is defined in `assets/lexicon.json`.

LOGOS implements serious formal semantics:
- **Neo-Davidsonian event decomposition** with thematic roles (Agent, Patient, Theme, Goal)
- **Montague-style compositional semantics** via λ-calculus
- **DRS (Discourse Representation Structures)** for anaphora resolution
- **Vendler aspectual classes** (State, Activity, Accomplishment, Achievement, Semelfactive)

### Quantifiers

```
Input:  "All men are mortal."
Output: ∀x(Man(x) → Mortal(x))

Input:  "Some cats are black."
Output: ∃x(Cat(x) ∧ Black(x))

Input:  "No dogs are cats."
Output: ∀x(Dog(x) → ¬Cat(x))

Input:  "Most birds fly."
Output: MOST x(Bird(x), Fly(x))

Input:  "Three cats sleep."
Output: ∃=3x(Cat(x) ∧ Sleep(x))

Input:  "At least two dogs bark."
Output: ∃≥2x(Dog(x) ∧ Bark(x))

Input:  "Nobody runs."
Output: ∀x(Person(x) → ¬Run(x))

Input:  "Nothing matters."
Output: ∀x(Thing(x) → ¬Matter(x))

Input:  "Dogs bark."
Output: GEN x(Dog(x), Bark(x))
```

**Note on Generics:** "Dogs bark" uses the generic quantifier (GEN), not universal (∀). Generics allow exceptions—"Dogs bark" is true even if some dogs don't bark.

### Connectives

```
Input:  "John runs and Mary walks."
Output: Run(j) ∧ Walk(m)

Input:  "John runs or Mary walks."
Output: Run(j) ∨ Walk(m)

Input:  "John does not run."
Output: ¬Run(j)

Input:  "If John runs then Mary walks."
Output: Run(j) → Walk(m)

Input:  "John runs if and only if Mary walks."
Output: Run(j) ↔ Walk(m)
```

### Causal Connectives

LOGOS handles causal expressions with the "because" connective:

```
Input:  "John ran because Mary walked."
Output: Run(j) ∧ Walk(m) ∧ Because(Run(j), Walk(m))

Input:  "The ground is wet because it rained."
Output: Wet(ground) ∧ Rain() ∧ Because(Wet(ground), Rain())
```

### Modal Operators

Modal operators are parsed with correct force and flavor:

| Modal | Force | Flavor | Symbol |
|-------|-------|--------|--------|
| must | Universal | Epistemic/Deontic | □ |
| can | Existential | Dynamic | ◇ |
| may | Existential | Deontic | ◇ |
| should | Universal | Deontic | ○ |

```
Input:  "John can swim."
Output: ◇Swim(j)

Input:  "John must leave."
Output: □Leave(j)

Input:  "John may enter."
Output: ◇Enter(j)       [deontic permission]

Input:  "John should help."
Output: ○Help(j)        [deontic obligation]
```

The system distinguishes:
- **Epistemic:** "John must be home" → inference from evidence
- **Deontic:** "John must leave" → obligation
- **Dynamic:** "John can swim" → ability

### Tense & Aspect

LOGOS classifies verbs by Vendler class (aktionsart) to correctly compose tense and aspect:

| Vendler Class | Example | Property |
|---------------|---------|----------|
| State | "know", "love" | No internal structure |
| Activity | "run", "swim" | Unbounded process |
| Accomplishment | "build a house" | Bounded process with endpoint |
| Achievement | "arrive", "die" | Instantaneous change |
| Semelfactive | "knock", "cough" | Single occurrence |

```
Input:  "John ran."
Output: PAST(Run(j))

Input:  "John will run."
Output: FUT(Run(j))

Input:  "John is running."
Output: PROG(Run(j))

Input:  "John has run."
Output: PERF(Run(j))

Input:  "John had been running."
Output: PAST(PERF(PROG(Run(j))))
```

This enables correct parsing of aspectual compositions:
- "John was building a house" → progressive of accomplishment
- "John was knowing the answer" → anomalous (states resist progressive)

**Stative Progressive Anomaly:** Stative verbs (know, love, believe) typically resist progressive aspect because they lack internal temporal structure. LOGOS detects this mismatch—states describe conditions that hold, not ongoing processes.

**Reichenbachian Temporal Structure:**

LOGOS uses Reichenbach's three-point system: Event (E), Reference (R), and Speech (S) time:

```
Past Perfect:    "John had run."     E < R < S
Present Perfect: "John has run."    E < R = S
Future Perfect:  "John will have run." E < S < R
Simple Past:     "John ran."         E = R < S
Simple Future:   "John will run."    S < E = R
```

This enables precise temporal ordering constraints in the output:

```
Input:  "John had run."
Output: ∃e(Run(e) ∧ Agent(e, j) ∧ Precedes(e, r) ∧ Precedes(r, S))
```

### Comparatives & Superlatives

LOGOS handles gradable adjectives with degree semantics:

```
Input:  "John is taller than Mary."
Output: Taller(j, m)

Input:  "John is the tallest man."
Output: ∃x(Man(x) ∧ Tallest(x) ∧ x = j)

Input:  "Mary is much taller than John."
Output: Taller(m, j, MUCH)
```

The system distinguishes:
- **Comparatives**: "taller than", "more intelligent than"
- **Superlatives**: "the tallest", "the most intelligent"
- **Degree modifiers**: "much taller", "slightly faster", "far better"

### Units & Dimensionality

LOGOS supports measure phrases with explicit dimension tracking:

**Dimension Types:**

| Dimension | Units | Examples |
|-----------|-------|----------|
| Length | inch, foot, yard, meter, mile | "five feet tall" |
| Time | second, minute, hour, day, year | "three hours long" |
| Weight | ounce, pound, gram, kilogram | "ten pounds heavy" |
| Temperature | degree | "ninety degrees" |
| Cardinality | child, item, piece | "five children" |

**Measure Phrases:**

```
Input:  "John is six feet tall."
Output: Height(j) = Measure(6, Foot)

Input:  "The meeting lasted three hours."
Output: ∃e(Meeting(e) ∧ Duration(e) = Measure(3, Hour))

Input:  "The box weighs ten pounds."
Output: Weight(box) = Measure(10, Pound)
```

**Comparative Measures:**

```
Input:  "John is two inches taller than Mary."
Output: Height(j) - Height(m) = Measure(2, Inch)

Input:  "The train arrived five minutes late."
Output: ∃e(Arrive(e) ∧ Theme(e, train) ∧ Delay(e) = Measure(5, Minute))
```

**Cardinal Measures:**

```
Input:  "Five children played."
Output: ∃=5x(Child(x) ∧ Play(x))

Input:  "Many students attended."
Output: MANY x(Student(x), Attend(x))
```

For comprehensive coverage of arithmetic operators, numeric literals, quantified cardinals, and output formats, see [MATH.md](MATH.md).

### Event Adjectives

Adjectives can have multiple readings depending on whether they modify individuals or events:

```
Input: "The beautiful dancer performed."

Reading 1 (Intersective - modifies the person):
Beautiful(x) ∧ Dancer(x) ∧ Perform(x)
"The dancer who is beautiful performed"

Reading 2 (Event-modifying - modifies the dancing):
∃e(Dance(e) ∧ Agent(e, x) ∧ Beautiful(e))
"The dancer performed beautifully"
```

| Adjective Type | Example | Readings |
|----------------|---------|----------|
| Physical | tall, short | Intersective only |
| Manner | graceful, clumsy | Both readings |
| Aesthetic | beautiful, elegant | Both readings |

**Subsective vs Intersective Adjectives:**

Beyond event modification, LOGOS distinguishes intersective from subsective adjectives:

```
Intersective:  "The red ball"
Output: Red(x) ∧ Ball(x)
"Red" applies absolutely to the entity

Subsective:    "The small elephant"
Output: Small(^Elephant)(x) ∧ Elephant(x)
"Small" is relative to the comparison class (^Elephant = elephant intension)
```

| Type | Example | Semantics |
|------|---------|-----------|
| Intersective | red, wooden, dead | Adj(x) ∧ N(x) |
| Subsective | small, large, tall | Adj(^N)(x) ∧ N(x) |
| Privative | fake, former | Adj(x) ∧ ¬N(x) |

**Full Adjective Taxonomy:**

LOGOS classifies 400+ adjectives across five semantic categories:

| Category | Count | Examples | Behavior |
|----------|-------|----------|----------|
| Intersective | ~200 | red, tall, happy, dead, wooden | Absolute property: Adj(x) ∧ N(x) |
| Subsective | ~20 | small, large, good, bad, big | Comparison class: Adj(^N)(x) ∧ N(x) |
| Non-Intersective/Privative | ~20 | fake, former, alleged, counterfeit, would-be, imaginary, fictional | Negates noun: Adj(x) ∧ ¬N(x) |
| EventModifier | ~10 | fast, graceful, skillful, clumsy, elegant, quick | Modifies event, not entity |
| Weather | ~10 | hot, cold, sunny, humid, windy, rainy, foggy | Predicates of weather conditions |

**EventModifier Adjectives:**

EventModifier adjectives can modify the event rather than the entity:

```
Input: "The graceful dancer performed."

Reading 1 (Entity-modifying):
Graceful(x) ∧ Dancer(x) ∧ Perform(x)
"The dancer who is graceful performed"

Reading 2 (Event-modifying):
∃e(Perform(e) ∧ Agent(e, x) ∧ Dancer(x) ∧ Graceful(e))
"The dancer performed gracefully"
```

**Weather Adjectives:**

Weather adjectives predicate of atmospheric conditions:

```
Input: "It is cold."
Output: Cold(weather)

Input: "The day was sunny and humid."
Output: Sunny(day) ∧ Humid(day)
```

### Distributive vs Collective

LOGOS implements Link's Logic of Plurals to handle plural predication:

```
Input:  "The men slept."
Output: ∀x(x ∈ men → Sleep(x))  [Distributive]
"Each man slept individually"

Input:  "The men gathered."
Output: Gather(men)  [Collective]
"The men gathered as a group"

Input:  "The men lifted the piano."
Reading 1: ∀x(x ∈ men → Lift(x, piano))  [Distributive]
Reading 2: Lift(men, piano)  [Collective]
```

Verbs are classified in the lexicon:

| Verb Type | Examples | Behavior |
|-----------|----------|----------|
| Distributive | sleep, eat, die | Only individual application |
| Collective | gather, assemble, meet | Only group application |
| Mixed | lift, carry, push | Both readings |

**Link's Logic Operators:**

LOGOS uses two formal operators from Link's algebra:

| Operator | Name | Meaning |
|----------|------|---------|
| σ (sigma) | Maximal sum | The maximal plural individual satisfying a predicate |
| * (star) | Distributive marker | Distributes predicate over atomic parts |

```
Input: "The men who lifted the piano."
Output: σx(*Man(x) ∧ Lift(x, piano))
"The maximal sum of men such that each lifted the piano"

Input: "The cats slept."
Output: *Sleep(σx(Cat(x)))
"Sleeping distributed over the maximal sum of cats"
```

**Verb Classification Mechanics:**

The lexer provides predicates for runtime classification:
- `is_distributive_verb()` — sleep, run, eat, die, think
- `is_collective_verb()` — gather, meet, assemble, disperse, congregate
- `is_mixed_verb()` — lift, carry, push, build, write

Mixed verbs generate multiple readings during parse forest construction.

### Axioms & Entailment

The lexicon encodes meaning postulates that drive semantic inference:

**Noun Entailments:**
```
Input:  "John is a bachelor."
Output: Bachelor(j) ∧ Unmarried(j) ∧ Male(j) ∧ Adult(j)
```

**Privative Adjectives:**
```
Input:  "This is a fake gun."
Output: Fake(x) ∧ ¬Gun(x)
"A fake gun is not a gun"

Input:  "This is a former president."
Output: Former(x) ∧ ¬President(x) ∧ PAST(President(x))
```

**Verbal Entailment:**
```
Input:  "John murdered Mary."
Output: Murder(j, m) → Kill(j, m) ∧ Intentional(j)
```

**Hypernyms:**
```
Input:  "Every dog barks."
Output: ∀x(Dog(x) → Bark(x)) ∧ ∀x(Dog(x) → Animal(x))
```

### Proof Engine

LOGOS includes a native proof engine that constructs derivation trees explaining *why* something is true, not just *that* it is true.

**Curry-Howard Correspondence:**
- A Proposition is a Type
- A Proof is a Program
- Verification is Type Checking

#### BackwardChainer

The proof engine uses backward chaining: starting from the goal, it searches for inference rules whose conclusions match, then recursively proves their premises.

```rust
use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

let mut engine = BackwardChainer::new();

// Axiom 1: All humans are mortal
engine.add_axiom(ProofExpr::ForAll {
    variable: "x".into(),
    body: Box::new(ProofExpr::Implies(
        Box::new(ProofExpr::Predicate {
            name: "Human".into(),
            args: vec![ProofTerm::Variable("x".into())],
            world: None,
        }),
        Box::new(ProofExpr::Predicate {
            name: "Mortal".into(),
            args: vec![ProofTerm::Variable("x".into())],
            world: None,
        }),
    )),
});

// Axiom 2: Socrates is human
engine.add_axiom(ProofExpr::Predicate {
    name: "Human".into(),
    args: vec![ProofTerm::Constant("Socrates".into())],
    world: None,
});

// Goal: Prove Socrates is mortal
let goal = ProofExpr::Predicate {
    name: "Mortal".into(),
    args: vec![ProofTerm::Constant("Socrates".into())],
    world: None,
};

let proof = engine.prove(goal).unwrap();
println!("{}", proof.display_tree());
```

**Output:**
```
└─ [ModusPonens] Mortal(Socrates)
   └─ [UniversalInst(Socrates)] Human(Socrates) → Mortal(Socrates)
      └─ [PremiseMatch] ∀x(Human(x) → Mortal(x))
   └─ [PremiseMatch] Human(Socrates)
```

#### Inference Rules

| Rule | Logic | Description |
|------|-------|-------------|
| PremiseMatch | Γ, P ⊢ P | Direct match with knowledge base |
| ModusPonens | P → Q, P ⊢ Q | If P implies Q and P holds, then Q |
| ModusTollens | ¬Q, P → Q ⊢ ¬P | Contrapositive reasoning |
| ConjunctionIntro | P, Q ⊢ P ∧ Q | Prove both sides |
| ConjunctionElim | P ∧ Q ⊢ P | Extract from conjunction |
| DisjunctionIntro | P ⊢ P ∨ Q | Prove one side |
| DisjunctionElim | P ∨ Q, P → R, Q → R ⊢ R | Case analysis |
| UniversalInst | ∀x P(x) ⊢ P(c) | Instantiate with specific term |
| ExistentialIntro | P(c) ⊢ ∃x P(x) | Witness introduction |
| StructuralInduction | P(0), ∀k(P(k) → P(S(k))) ⊢ ∀n P(n) | Induction on inductive types |

#### Structural Induction

The proof engine supports structural induction on inductive types like Peano naturals and lists.

**Example: Proving ∀n. Add(n, 0) = n**

```rust
// Define addition axioms
engine.add_axiom(eq(app("Add", vec![zero(), var("m")]), var("m")));
engine.add_axiom(eq(
    app("Add", vec![succ(var("k")), var("m")]),
    succ(app("Add", vec![var("k"), var("m")])),
));

// Goal: ∀n:Nat. Add(n, 0) = n
let goal = eq(app("Add", vec![nat_var("n"), zero()]), nat_var("n"));

let proof = engine.prove(goal).unwrap();
// Uses StructuralInduction with base case and step case
```

The prover automatically:
1. **Base case:** Substitutes Zero for n, proves Add(Zero, 0) = Zero
2. **Step case:** Assumes Add(k, 0) = k (induction hypothesis), proves Add(Succ(k), 0) = Succ(k)

#### Unification

The engine uses Robinson's unification algorithm with occurs check to find substitutions that make terms identical:

```
Mortal(x) unifies with Mortal(Socrates)
→ {x ↦ Socrates}

f(g(x), y) unifies with f(g(a), b)
→ {x ↦ a, y ↦ b}

x unifies with f(x)
→ FAILS (occurs check prevents infinite types)
```

**Alpha-Equivalence:** Bound variable names are arbitrary. The unifier understands that `∃e P(e)` is equivalent to `∃x P(x)`. This enables event semantics where "John runs" parsed twice may generate different event variable names (`e₁` vs `e₂`) but should still unify.

Fresh constants (`#α0`, `#α1`, ...) are substituted for bound variables before comparing bodies, ensuring correct structural comparison without capture issues.

#### Beta-Reduction

The proof engine implements beta-reduction for lambda calculus—the computational engine that underpins type theory:

**Basic Reduction:**
```
(λx. Run(x))(John) → Run(John)
```

**Nested Reduction:**
```
(λx. (λy. P(x, y))(B))(A) → P(A, B)
```

The prover normalizes both goals and premises before matching, so:

```rust
// Premise: (λx. Run(x))(John)
// Goal: Run(John)
// ✓ Matches after beta-reduction
```

This enables higher-order reasoning where lambda terms appear in axioms or goals.

#### Pattern Unification

Miller Pattern Unification is the decidable fragment of higher-order unification used for:
- **Motive inference** in structural induction
- **Type inference** in dependent types
- **Implicit argument resolution**

**The Pattern:** `?F(x₁, ..., xₙ) = Body` where xᵢ are distinct bound variables
**The Solution:** `?F = λx₁...λxₙ. Body`

**Simple Example:**
```
?P(x) = x + 0 = x
→ ?P = λx. (x + 0 = x)
```

**Multi-Argument Example:**
```
?F(x, y) = x + y = y + x
→ ?F = λx.λy. (x + y = y + x)
```

**Error Cases:**
- Duplicate variables: `?P(x, x)` — not a Miller pattern (rejected)
- Scope violation: `?P(x) = y + 0` where y is not in scope (rejected)

This enables the prover to automatically infer induction motives when proving properties like `∀n. Add(n, 0) = n`.

#### Type Kernel

The proof engine includes a Calculus of Constructions (CoC) kernel—the type-theoretic foundation that makes proofs and programs the same thing.

**Universe Hierarchy:**
```
Prop : Type₁ : Type₂ : Type₃ : ...
```

**Dependent Function Types (Π):**
```
ΠA:Type. Πx:A. A    -- The type of polymorphic identity
```

**Example: Polymorphic Identity Function**
```rust
use logicaffeine_kernel::{Term, Universe, Context, infer_type};

// λA:Type. λx:A. x
let id = Term::Lambda {
    param: "A",
    param_type: Box::new(Term::Sort(Universe::Type(0))),
    body: Box::new(Term::Lambda {
        param: "x",
        param_type: Box::new(Term::Var("A")),
        body: Box::new(Term::Var("x")),
    }),
};

// Kernel infers: ΠA:Type. Πx:A. A
let ty = infer_type(&Context::new(), &id)?;
```

The kernel:
- Implements the infinite universe hierarchy (Prop : Type₁ : Type₂ : ...)
- Type-checks dependent function types (Π-types)
- Performs substitution with capture avoidance
- Checks alpha-equivalence for type equality
- Rejects type errors (mismatches, unbound variables, non-function application)

#### Inductive Types

The kernel supports inductive type definitions—the "I" in CIC (Calculus of Inductive Constructions).

**Formation & Introduction Rules:**
```
Nat : Type₀                     -- Formation: Nat is a type
Zero : Nat                      -- Introduction: nullary constructor
Succ : Nat → Nat               -- Introduction: unary constructor
```

**Example: Defining Peano Naturals**
```rust
use logicaffeine_kernel::{Term, Universe, Context};

let mut ctx = Context::new();

// Nat : Type 0
ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

// Zero : Nat
ctx.add_constructor("Zero", "Nat", Term::Global("Nat".into()));

// Succ : Nat -> Nat
ctx.add_constructor("Succ", "Nat", Term::Pi {
    param: "_".into(),
    param_type: Box::new(Term::Global("Nat".into())),
    body: Box::new(Term::Global("Nat".into())),
});

// Succ(Succ(Zero)) : Nat ✓
```

Inductive types enable:
- **Data definition**: Nat, List, Bool, Tree as first-class kernel types
- **Type-safe constructors**: Zero and Succ are the only ways to build Nat
- **Elimination via match**: Consume inductive values with pattern matching

**Elimination (Match):**
```
match n return (λ_. Nat) with
| Zero   => Zero
| Succ k => k
```

The match typing rule ensures exhaustive, type-safe case analysis:
- The discriminant must have an inductive type
- The motive `P : I → Type` determines the result type
- Each constructor gets exactly one case with the correct type

#### Polymorphic Inductive Types

The kernel supports polymorphic inductive types with type parameters:

**Syntax:**
```coq
Inductive List (A : Type) :=
  Nil : List A
  | Cons : A -> List A -> List A.

Inductive Either (A : Type) (B : Type) :=
  Left : A -> Either A B
  | Right : B -> Either A B.
```

**Type Signatures:**
- `List : Type -> Type` (or `Π(A:Type). Type`)
- `Nil : Π(A:Type). List A`
- `Cons : Π(A:Type). A -> List A -> List A`

**Instantiation:**
```
List Nat              -- List of naturals
Nil Nat               -- Empty list of naturals
Cons Nat Zero (Nil Nat)   -- [0]
```

Type parameters are prepended to constructor types, enabling polymorphic data structures like `List`, `Either`, `Option`, and `Tree`.

#### Generic Elimination (DElim)

The `DElim` construct provides a generic elimination principle for any inductive type:

**Derivation Constructors:**

| Constructor | Type | Purpose |
|-------------|------|---------|
| `DCase` | `Derivation -> Derivation -> Derivation` | Chain case proofs |
| `DCaseEnd` | `Derivation` | Terminate case chain |
| `DElim` | `Syntax -> Syntax -> Derivation -> Derivation` | Generic eliminator |

**How DElim Works:**
```
DElim(InductiveType, Motive, CaseChain)
```

1. **InductiveType**: The type to eliminate over (e.g., `Nat`, `List A`)
2. **Motive**: The goal predicate `λn:T. P(n)`
3. **CaseChain**: Proofs for each constructor via `DCase`

**Example: Induction on Nat**
```
Motive: λn:Nat. Eq Nat n n
Cases: DCase(base_proof, DCase(step_proof, DCaseEnd))
Result: ∀n:Nat. Eq Nat n n
```

DElim validates that:
- Case count matches constructor count
- Each case conclusion matches the expected goal type

#### List Operations

With polymorphic types and DElim, the kernel supports standard list operations:

**Append:**
```coq
Definition append : forall A : Type, List A -> List A -> List A :=
  fun A : Type =>
  fix rec =>
  fun xs : List A =>
  fun ys : List A =>
  match xs return List A with
  | Nil => ys
  | Cons h t => Cons A h (rec t ys)
  end.
```

**Map:**
```coq
Definition map : forall A B : Type, (A -> B) -> List A -> List B :=
  fun A B : Type =>
  fun f : A -> B =>
  fix rec =>
  fun xs : List A =>
  match xs return List B with
  | Nil => Nil B
  | Cons h t => Cons B (f h) (rec t)
  end.
```

**Length:**
```coq
Definition length : forall A : Type, List A -> Nat := ...
```

These operations compute correctly under evaluation:
- `append [0] [1]` → `[0, 1]`
- `map Succ [0, 1]` → `[1, 2]`
- `length [0, 1, 2]` → `3`

**Verified Theorems:**

The kernel can computationally verify list theorems:

| Theorem | Statement |
|---------|-----------|
| `append_nil_r` | `∀l. append l [] = l` |
| `append_assoc` | `∀x y z. append (append x y) z = append x (append y z)` |
| `map_id` | `∀l. map id l = l` |
| `length_append` | `∀x y. length (append x y) = plus (length x) (length y)` |

These are verified by computation: both sides reduce to the same normal form.

#### Universe Cumulativity

The kernel implements universe subtyping—types at lower levels can be used where higher levels are expected:

```
Prop ≤ Type₀ ≤ Type₁ ≤ Type₂ ≤ ...
```

**What This Enables:**
- A function expecting `Type₁` accepts `Nat : Type₀`
- Propositions (`Prop`) can be used where types are expected
- Pi types are contravariant in parameters, covariant in return types

**No Downward Flow:** `Type₀` cannot be used where `Prop` is expected—the hierarchy only flows upward.

#### Kernel Prelude

The kernel includes a standard library of fundamental logical types:

| Type | Universe | Constructors | Purpose |
|------|----------|--------------|---------|
| `Nat` | Type₀ | `Zero`, `Succ` | Natural numbers |
| `True` | Prop | `I` | Trivial proposition |
| `False` | Prop | (none) | Empty type (absurdity) |
| `Eq` | Π(A:Type). A → A → Prop | `refl` | Propositional equality |
| `And` | Prop → Prop → Prop | `conj` | Logical conjunction |
| `Or` | Prop → Prop → Prop | `left`, `right` | Logical disjunction |

**Example: Proving Equality**
```rust
use logicaffeine_kernel::prelude::with_prelude;

let ctx = with_prelude();

// refl Nat Zero : Eq Nat Zero Zero
// "Proof that 0 = 0"
```

The prelude enables expressing and type-checking propositions like `Eq Nat (Succ Zero) (Succ Zero)` (proof that 1 = 1).

#### Certifier

The certifier bridges the proof engine and kernel—it converts derivation trees into lambda terms that type-check in the kernel. This is the Curry-Howard correspondence made concrete:

| DerivationTree Rule | Kernel Term |
|---------------------|-------------|
| Axiom / PremiseMatch | `Term::Global(name)` |
| ModusPonens [impl, arg] | `Term::App(impl_term, arg_term)` |
| ConjunctionIntro [p, q] | `conj P Q p_term q_term` |
| UniversalInst(witness) | `Term::App(forall_proof, witness)` |
| UniversalIntro(x:T) | `Term::Lambda(x, T, body)` |
| StructuralInduction | `Term::Fix` + `Term::Match` |
| ExistentialIntro(witness) | `exist T witness proof` |

**Why It Matters:** Every proof produced by the backward chainer can now be independently verified by the kernel's type checker. The kernel audits the engine—untrusted proofs become certified terms.

**Example: The Classic Syllogism**
```
h1 : ∀x. P(x) → Q(x)    [Kernel: Π(x:Nat). P x → Q x]
h2 : P(Zero)            [Kernel: P Zero]
────────────────────────────────────────────────────
Certified: (h1 Zero) h2 : Q Zero
```

#### End-to-End Verification

The complete verification pipeline connects all components:

```
Input → Parse → Engine → Certify → Type-Check → Verified
```

**The Socrates Syllogism, Verified:**
```rust
let result = verify_theorem(
    "All men are mortal. Socrates is a man. Therefore Socrates is mortal."
);
// Produces: ((h2 Socrates) h1) : mortal(Socrates)
```

The `verify_theorem` function:
1. Parses natural language to FOL
2. Runs backward chaining proof search
3. Certifies the derivation tree to kernel terms
4. Type-checks the certified term against the goal type

If any step fails, verification fails—no false positives.

#### The Guardian: Termination & Positivity

The Guardian protects the kernel from logical inconsistency:

**Termination Checking:** Recursive functions must decrease on a structural argument.
```
fix f. λn:Nat. match n { Zero → ... | Succ k → f k }  ✓ (k < Succ k)
fix f. λn:Nat. f (Succ n)                              ✗ (infinite loop)
```

**Positivity Checking:** Inductive types cannot appear negatively in their own constructors.
```
Inductive Nat { Zero : Nat, Succ : Nat → Nat }        ✓ (positive)
Inductive Bad { MkBad : (Bad → Bool) → Bad }          ✗ (negative occurrence)
```

Negative occurrences enable Curry's paradox—the type-theoretic equivalent of Russell's paradox. The Guardian rejects them before they can break soundness.

#### Equality & Rewriting

The Mirror implements Leibniz's Law: equals can be substituted for equals.

**Core Rules:**
```
Rewrite:      a = b, P(a) ⊢ P(b)     (Leibniz's Law)
Symmetry:     a = b ⊢ b = a
Transitivity: a = b, b = c ⊢ a = c
```

**Example: The Superman Syllogism**
```
Clark = Superman ∧ mortal(Clark) ⊢ mortal(Superman)
```

The kernel provides `Eq_rec` (the equality eliminator), `Eq_sym`, and `Eq_trans` as certified primitives.

#### Full Reduction

The Calculator teaches the proof engine to compute:

**Iota Reduction:** Pattern matching on constructors
```
match (Succ k) { Zero → a | Succ n → body } → body[n := k]
```

**Fix Unfolding:** Recursive functions unfold on constructors
```
(fix f. λn. match n {...}) (Succ k) → unfolds and reduces
```

**Reflexivity by Computation:** Proves `a = b` by normalizing both sides
```
1 + 1 = 2  →  Succ (Succ Zero) = Succ (Succ Zero)  →  refl
```

This enables arithmetic proofs: `0 + n = n`, `1 + 1 = 2`, `2 + 1 = 3`.

#### Delta Reduction

The kernel now has memory—global definitions unfold during normalization:

```
Definition two : Nat := Succ(Succ(Zero))

normalize(two) → Succ(Succ(Zero))    (δ-reduction)
```

**Three kinds of globals:**
| Kind | Behavior | Example |
|------|----------|---------|
| Definition | Unfolds (transparent) | `two := Succ(Succ(Zero))` |
| Axiom | Stuck (opaque) | `human : Entity → Prop` |
| Constructor | Iota-eliminated | `Succ : Nat → Nat` |

#### Kernel Primitives

The kernel supports native hardware types for practical computation—no more stack overflows from Peano arithmetic on large numbers.

**Primitive Types:**

| Type | Representation | Example |
|------|----------------|---------|
| `Int` | 64-bit signed integer | `42`, `-100`, `1000000` |
| `Float` | 64-bit floating point | `3.14`, `-0.5` |
| `Text` | UTF-8 string | `"hello"` |

**Hardware Arithmetic:**

```
Check 10000.
→ "10000 : Int" (instant)

Definition trillion : Int := mul 1000000 1000000.
Eval trillion.
→ "1000000000000" (instant via CPU ALU)
```

**Built-in Operations:**

| Operation | Signature | Behavior |
|-----------|-----------|----------|
| `add` | `Int → Int → Int` | Addition |
| `sub` | `Int → Int → Int` | Subtraction |
| `mul` | `Int → Int → Int` | Multiplication |
| `div` | `Int → Int → Int` | Integer division |
| `mod` | `Int → Int → Int` | Modulo |

**Why This Matters:**

Before kernel primitives, verifying `10000` required building `Succ(Succ(...))` 10,000 times—causing stack overflows. Native `i64` support enables instant verification of large numbers, which is required for arithmetization of syntax (encoding logical formulas as numbers).

#### Reflection

The kernel can represent its own syntax as data, enabling verified tactics, Gödel numbering, and self-referential theorems.

**Syntax Type:**

The `Syntax` inductive type encodes kernel terms using De Bruijn indices:

| Constructor | Type | Represents |
|-------------|------|------------|
| `SVar` | `Int → Syntax` | Variable (De Bruijn index) |
| `SGlobal` | `Text → Syntax` | Global reference |
| `SSort` | `Univ → Syntax` | Universe (Prop/Type) |
| `SApp` | `Syntax → Syntax → Syntax` | Application |
| `SLam` | `Syntax → Syntax → Syntax` | Lambda abstraction |
| `SPi` | `Syntax → Syntax → Syntax` | Pi type |
| `SLit` | `Int → Syntax` | Integer literal (for quoting) |
| `SName` | `Text → Syntax` | Named reference (for quoting) |

**Universe Type:**

| Constructor | Type | Represents |
|-------------|------|------------|
| `UProp` | `Univ` | Prop universe |
| `UType` | `Int → Univ` | Type at level n |

**Derivation Type:**

The `Derivation` inductive type encodes proof trees as first-class data:

| Constructor | Type | Represents |
|-------------|------|------------|
| `DAxiom` | `Syntax → Derivation` | Introduce an axiom |
| `DModusPonens` | `Derivation → Derivation → Derivation` | Modus ponens: from P and P→Q, derive Q |
| `DUnivIntro` | `Derivation → Derivation` | Universal introduction: from P, derive ∀x.P |
| `DUnivElim` | `Derivation → Syntax → Derivation` | Universal elimination: from ∀x.P, derive P[t/x] |
| `DRefl` | `Syntax → Syntax → Derivation` | Reflexivity: prove Eq T a a |
| `DInduction` | `Syntax → Derivation → Derivation → Derivation` | Induction: motive, base case, step case |
| `DCompute` | `Syntax → Derivation` | Proof by computation: prove Eq T A B if eval(A) == eval(B) |
| `DCong` | `Syntax → Derivation → Derivation` | Congruence: from Eq T a b, derive Eq T (f a) (f b) |

**Derivation Operations:**

| Function | Type | Description |
|----------|------|-------------|
| `concludes` | `Derivation → Syntax` | Extract what a derivation proves |
| `try_refl` | `Syntax → Derivation` | Reflexivity tactic: attempt to prove goal by reflexivity |
| `try_compute` | `Syntax → Derivation` | Computation tactic: prove equality by evaluating both sides |
| `try_cong` | `Syntax → Derivation → Derivation` | Congruence tactic: wrap DCong |
| `tact_fail` | `Syntax → Derivation` | Tactic that always fails (returns error) |
| `tact_orelse` | `(Syntax → Derivation) → (Syntax → Derivation) → Syntax → Derivation` | Try first tactic; if it fails, try second |

**Syntax Operations:**

| Function | Type | Description |
|----------|------|-------------|
| `syn_size` | `Syntax → Int` | Count nodes in syntax tree |
| `syn_max_var` | `Syntax → Int` | Max free variable index (-1 if closed) |
| `syn_lift` | `Int → Int → Syntax → Syntax` | Shift free variables by amount above cutoff |
| `syn_subst` | `Syntax → Int → Syntax → Syntax` | Capture-avoiding substitution |
| `syn_beta` | `Syntax → Syntax → Syntax` | Beta reduction: substitute arg into body |
| `syn_step` | `Syntax → Syntax` | Single-step head reduction |
| `syn_eval` | `Int → Syntax → Syntax` | Bounded evaluation up to N steps |
| `syn_quote` | `Syntax → Syntax` | Quote: produce code that constructs the input |
| `syn_diag` | `Syntax → Syntax` | Diagonalization: substitute quoted self into variable 0 |

**Example: Representing the Identity Function**

```
Definition id : Syntax := SLam (SSort (UType 0)) (SVar 0).
Eval (syn_size id).      → 3
Eval (syn_max_var id).   → -1  (closed term)
```

**Example: Variable Lifting**

```
Eval (syn_lift 1 0 (SVar 0)).                        → (SVar 1)
Eval (syn_lift 1 0 (SLam (SSort UProp) (SVar 0))).   → λ.var0  (bound, unchanged)
Eval (syn_lift 1 0 (SLam (SSort UProp) (SVar 1))).   → λ.var2  (free, shifted)
```

**Example: Capture-Avoiding Substitution**

```
Eval (syn_subst (SSort UProp) 0 (SVar 0)).           → (SSort UProp)
Eval (syn_subst A 0 (SLam T (SVar 0))).              → SLam T (SVar 0)  (bound)
Eval (syn_subst A 0 (SLam T (SVar 1))).              → SLam T A         (free)
```

**Example: Beta Reduction**

```
Eval (syn_beta (SVar 0) (SSort UProp)).              → (SSort UProp)
Eval (syn_step (SApp (SLam T (SVar 0)) A)).          → A
Eval (syn_step (SApp (SApp (SLam T body) x) y)).     → (SApp (syn_beta body x) y)
```

**Why This Matters:**

`syn_step` performs single-step head reduction on embedded syntax, enabling verified computation on reflected terms. This is required for the Diagonal Lemma, where a formula must evaluate its own encoding.

**Example: Bounded Evaluation**

```
Eval (syn_eval 0 (SApp (SLam T (SVar 0)) A)).    → (SApp (SLam T (SVar 0)) A)  (no fuel)
Eval (syn_eval 1 (SApp (SLam T (SVar 0)) A)).    → A                           (one step)
Eval (syn_eval 10 ((λx.λy.x) A B)).              → A                           (multi-step)
```

**Why This Matters:**

`syn_eval` provides bounded evaluation with a fuel parameter, ensuring termination. This avoids the Halting Problem by design—computation is always total, preventing non-termination during proof checking.

**Example: Quoting**

```
Eval (syn_quote (SVar 5)).                       → (SApp (SName "SVar") (SLit 5))
Eval (syn_quote (SSort UProp)).                  → (SApp (SName "SSort") (SName "UProp"))
Eval (syn_quote (SApp f x)).                     → (SApp (SApp (SName "SApp") (syn_quote f)) (syn_quote x))
```

**Why This Matters:**

`syn_quote` produces code that constructs its input—required for Gödel's Diagonal Lemma. The diagonalization function `syn_diag x := syn_subst (syn_quote x) 0 x` substitutes the quoted representation of x into variable 0 of x itself, enabling self-referential sentences.

**Example: Inference Rules**

```
Definition ax : Derivation := DAxiom P.
Eval (concludes ax).                             → P

Definition mp : Derivation := DModusPonens (DAxiom P) (DAxiom (Implies P Q)).
Eval (concludes mp).                             → Q

Definition ui : Derivation := DUnivIntro (DAxiom P).
Eval (concludes ui).                             → (Forall T P)

Definition ue : Derivation := DUnivElim (DAxiom (Forall T P)) A.
Eval (concludes ue).                             → P[A/0]
```

**Why This Matters:**

`Derivation` and `concludes` enable reasoning about proofs as data. The kernel can now represent provability itself, which is required for constructing the Gödel sentence "This statement is unprovable."

**Example: The Diagonal Lemma**

```
syn_diag x := syn_subst (syn_quote x) 0 x

Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
Definition G : Syntax := syn_diag T.
-- G says: "I am not provable" (the Gödel sentence)

Definition quine : Syntax := SApp (SName "Print") (SVar 0).
Definition Q : Syntax := syn_diag quine.
-- Q prints its own source code
```

**Why This Matters:**

`syn_diag` enables construction of self-referential sentences: Gödel sentences ("I am unprovable"), quines (self-replicating programs), and fixed points for arbitrary predicates.

**Example: The Gödel Sentence**

```
Definition Not : Prop -> Prop := fun P => P -> False.
Definition Provable : Syntax -> Prop :=
  fun s => Ex Derivation (fun d => Eq Syntax (concludes d) s).

Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
Definition G : Syntax := syn_diag T.
-- G says: "I am not provable"
```

**The Two Levels:**

| Level | Expression | Meaning |
|-------|------------|---------|
| Deep (Syntax) | `G : Syntax` | The sentence "I am not provable" |
| Shallow (Prop) | `Provable G : Prop` | Can we prove G? |
| | `Not (Provable G) : Prop` | The claim G makes about itself |

**Why This Matters:**

If the system is consistent, it cannot prove G (because G asserts its own unprovability). Yet G is true—it correctly describes itself. This is the First Incompleteness Theorem: any consistent formal system capable of encoding arithmetic contains true statements it cannot prove.

**Example: The Incompleteness Theorems**

```
Definition Consistent : Prop := Not (Provable (SName "False")).

Definition Godel_I : Prop := Consistent -> Not (Provable G).
Check Godel_I.                           → Consistent -> (Not (Provable G)) : Prop

Definition Godel_II : Prop := Consistent -> Not (Provable ConsistentSyn).
Check Godel_II.                          → Consistent -> (Not (Provable ConsistentSyn)) : Prop
```

**The Two Theorems:**

| Theorem | Statement | Meaning |
|---------|-----------|---------|
| Gödel I | `Consistent -> Not (Provable G)` | If consistent, G is unprovable |
| Gödel II | `Consistent -> Not (Provable ConsistentSyn)` | If consistent, cannot prove own consistency |

**Why This Matters:**

The kernel can now formally state its own incompleteness. This is not a limitation of the implementation but a mathematical certainty—any sufficiently powerful consistent system has inherent boundaries.

**Example: Verified Tactics**

```
Definition goal : Syntax := Eq Nat Zero Zero.
Definition proof : Derivation := try_refl goal.
Eval (concludes proof).                  → Eq Nat Zero Zero

-- The tactic workflow:
-- 1. Define a goal (as Syntax)
-- 2. Run the tactic: proof := try_refl goal
-- 3. Verify: concludes proof == goal  ✓
```

**Why This Matters:**

`try_refl` is a tactic that inspects the goal, constructs the appropriate `DRefl` derivation, and returns a verified proof. This enables proof automation—tactics that search for proofs programmatically.

**Example: Deep Induction**

```
Definition motive : Syntax := SLam (SName "Nat") P.
Definition base : Derivation := (* proof that P(Zero) *)
Definition step : Derivation := (* proof that ∀k. P(k) → P(Succ k) *)

Definition ind_proof : Derivation := DInduction motive base step.
Eval (concludes ind_proof).              → Forall Nat motive
```

**Verification at `concludes` time:**

| Check | Requirement |
|-------|-------------|
| Base case | `concludes base == motive[Zero/n]` |
| Step case | `concludes step == ∀k. motive[k/n] → motive[Succ k/n]` |
| Result | If both pass → `Forall Nat motive`, else → `Error` |

**Why This Matters:**

`DInduction` encodes mathematical induction as a derivation constructor. The kernel verifies that the base case proves P(Zero) and the step case proves ∀k. P(k) → P(Succ k) before concluding ∀n. P(n).

**Example: Tactic Combinators**

```
Definition solve_trivial := tact_orelse try_refl tact_fail.

Eval (concludes (solve_trivial (Eq Nat Zero Zero))).  → Eq Nat Zero Zero
Eval (concludes (solve_trivial (Eq Nat Zero One))).   → Error
```

**Why This Matters:**

`tact_orelse` enables composite tactics via lazy evaluation—the second tactic is only evaluated if the first fails. This allows building tactic strategies from simpler components.

**Example: Computational Proofs**

```
-- try_refl fails: (add 1 1) and 2 are syntactically different
Eval (concludes (try_refl (Eq Int (add 1 1) 2))).     → Error

-- try_compute succeeds: eval(add 1 1) = eval(2) = 2
Eval (concludes (try_compute (Eq Int (add 1 1) 2))). → Eq Int (add 1 1) 2

-- Composite tactic for arithmetic
Definition solve_arith := tact_orelse try_refl try_compute.
```

**Why This Matters:**

`try_compute` proves equalities by evaluating both sides (with bounded fuel) and comparing results. This extends `syn_step` to handle arithmetic operations (`add`, `sub`, `mul`, `div`, `mod`) on `SLit` values, enabling proofs like `1 + 1 = 2` automatically.

**Example: Congruence**

```
-- Given: eq_proof proves (Eq Nat k k')
-- Context: SLam (SName "Nat") (SApp (SName "Succ") (SVar 0))  -- λx. Succ x
Definition cong_proof := DCong context eq_proof.
Eval (concludes cong_proof).                     → Eq Nat (Succ k) (Succ k')
```

**Why This Matters:**

`DCong` implements Leibniz's Law (substituting equals for equals) in the deep embedding. Given a context `λx. f[x]` and a proof of `a = b`, it derives `f[a] = f[b]`. This enables the step case of induction proofs: from IH `k + 0 = k`, apply congruence with `λx. Succ x` to get `Succ (k + 0) = Succ k`.

#### Decision Procedure Tactics

LOGOS includes automated decision procedures that can prove goals in specific mathematical domains without manual proof construction.

| Tactic | Domain | Algorithm | Derivation Constructor |
|--------|--------|-----------|------------------------|
| `try_ring` | Polynomial equality | Normalization to canonical form | `DRingSolve` |
| `try_lia` | Linear integer arithmetic | Fourier-Motzkin elimination | `DLiaSolve` |
| `try_omega` | Integer arithmetic | Omega test with integer semantics | `DOmegaSolve` |
| `try_cc` | Equality reasoning | Congruence closure (Union-Find) | `DCCSolve` |
| `try_simp` | Term simplification | Bottom-up rewriting | `DSimpSolve` |

**Ring Tactic (Polynomial Equality):**

The `ring` tactic proves polynomial equalities by normalizing both sides to canonical form (sum of monomials) and comparing:

```
-- Proves automatically: 3(2k+1) + 1 = 6k + 4
Definition goal : Syntax := Eq Int (mul 3 (add (mul 2 k) 1)) (add (mul 6 k) 4).
Definition proof : Derivation := try_ring goal.
Eval (concludes proof).  → goal
```

Supports: addition, subtraction, multiplication (no division).

**LIA Tactic (Linear Integer Arithmetic):**

The `lia` tactic proves linear inequalities using Fourier-Motzkin elimination with exact rational arithmetic:

```
-- Proves: x < x + 1 (successor relation)
Definition goal : Syntax := Lt x (add x 1).
Definition proof : Derivation := try_lia goal.

-- Proves: 2 ≤ 5 (constant inequality)
Definition goal2 : Syntax := Le 2 5.
Definition proof2 : Derivation := try_lia goal2.
```

Supported relations: `Lt` (<), `Le` (≤), `Gt` (>), `Ge` (≥)

Constraint: Expressions must be linear (constants, variables, c*x — no x*y).

**Omega Tactic (Integer Arithmetic):**

The `omega` tactic is a decision procedure for integer linear arithmetic with proper integer semantics:

```
-- Integer-aware: x > 1 converts to x >= 2
-- Floor division: 3x <= 10 gives x <= 3
-- Parity detection: 2x = 5 detected as unsatisfiable
```

Key difference from LIA: Uses true integer arithmetic with proper rounding, not rationals.

**CC Tactic (Congruence Closure):**

The `cc` tactic proves equalities over uninterpreted functions using Union-Find with congruence propagation:

```
-- Given: a = b and f(a) = c
-- Proves: f(b) = c (by congruence)
```

Algorithm: Downey-Sethi-Tarjan E-graph construction with path compression.

**Simp Tactic (Simplification):**

The `simp` tactic normalizes goals by bottom-up term rewriting:

```
-- Proves: Eq 5 5 (reflexivity after simplification)
-- Proves: Eq (add 2 3) 5 (constant folding)
-- Handles hypothesis substitution with nested implications
```

#### Tactic Combinators

LOGOS provides combinators for composing tactics into complex proof strategies:

| Combinator | Type | Description |
|------------|------|-------------|
| `tact_fail` | `Syntax → Derivation` | Always fails (returns Error) |
| `tact_orelse` | `Tactic → Tactic → Syntax → Derivation` | Try first; if fails, try second |
| `tact_try` | `Tactic → Syntax → Derivation` | Attempt tactic; never fails (returns identity on failure) |
| `tact_then` | `Tactic → Tactic → Syntax → Derivation` | Sequence two tactics (;) |
| `tact_repeat` | `Tactic → Syntax → Derivation` | Apply repeatedly until failure or no progress |
| `tact_first` | `TList Tactic → Syntax → Derivation` | Try list of tactics until one succeeds |
| `tact_solve` | `Tactic → Syntax → Derivation` | Enforce tactic completely solves goal |

**Example: Composite Tactic Definition:**

```
-- Define a tactic that tries reflexivity first, falls back to fail
Definition solve_trivial : Syntax -> Derivation :=
    tact_orelse try_refl tact_fail.

-- Apply to a goal
Definition d : Derivation := solve_trivial (Eq Nat Zero Zero).
Eval (concludes d).  → Eq Nat Zero Zero
```

**Example: The Nuclear Option:**

```
-- A tactic that tries everything
Definition nuclear : Syntax -> Derivation :=
    tact_first (TacCons try_refl
               (TacCons try_ring
               (TacCons try_lia
               (TacCons try_compute TacNil)))).
```

**Example: Tactic Sequencing:**

```
-- Simplify, then try reflexivity
Definition simp_then_refl : Syntax -> Derivation :=
    tact_then try_simp try_refl.
```

**Fixed-Point Detection:**

`tact_repeat` detects fixed points to prevent infinite loops — if a tactic succeeds but makes no progress, iteration terminates.

#### Structural Proof Tactics

Beyond decision procedures, LOGOS provides tactics for structural reasoning:

| Tactic | Purpose | Example Use |
|--------|---------|-------------|
| `try_auto` | Automatic selection with hint database | General-purpose proving |
| `try_induction` | Structural induction on Nat, List, etc. | `∀n. n + 0 = n` |
| `try_inversion` | Derive contradictions from impossible patterns | `Succ n = Zero → False` |
| `try_rewrite` | Rewrite goal using equality (L→R) | Substitute equals |
| `try_rewrite_rev` | Rewrite goal using equality (R→L) | Reverse substitution |
| `try_destruct` | Case analysis on inductive term | Pattern match on Nat |
| `try_apply` | Apply hypothesis or lemma to goal | Use proven theorem |

**Induction Tactic:**

```
-- Prove: ∀n. n + 0 = n
Definition motive : Syntax := λn. Eq Nat (add n Zero) n.
Definition base : Derivation := (* proof that 0 + 0 = 0 *)
Definition step : Derivation := (* proof that k + 0 = k → Succ k + 0 = Succ k *)

Definition proof : Derivation := try_induction motive base step.
```

**Inversion Tactic:**

```
-- From Succ n = Zero, derive anything (contradiction)
Definition impossible : Syntax := Eq Nat (Succ n) Zero.
Definition d : Derivation := try_inversion impossible.
-- Proves: False (the equation is impossible)
```

**Rewrite Tactics:**

```
-- Given: eq_proof proves (Eq Nat a b)
-- Goal: P(a)
-- After try_rewrite eq_proof: Goal becomes P(b)

Definition rewritten := try_rewrite eq_proof goal.
```

#### Hint Database

The `auto` tactic maintains a database of hints—previously proven theorems it can apply automatically.

**Registering Hints:**

```
-- Prove a lemma
Definition add_zero_r : ∀n. n + 0 = n := (* proof *).

-- Register as hint
Hint add_zero_r.

-- Now auto will try add_zero_r when other tactics fail
Definition goal : Syntax := Eq Nat (add k Zero) k.
Definition proof : Derivation := try_auto goal.  -- Uses hint!
```

**Auto Tactic Search Order:**

1. `try_refl` - Reflexivity
2. `try_compute` - Computational equality
3. `try_ring` - Polynomial equality
4. `try_lia` - Linear integer arithmetic
5. `try_cc` - Congruence closure
6. `try_simp` - Simplification
7. **Hint database** - Registered theorems
8. Fail if nothing works

**Why Hints Matter:**

Without hints, `auto` only knows built-in decision procedures. With hints, it learns from your proofs—each registered lemma extends its capabilities.

#### Derivation Constructors Reference

The kernel's proof system uses derivation constructors to build certified proofs:

**Core Inference Rules:**

| Constructor | Type | Rule |
|-------------|------|------|
| `DAxiom` | `Syntax → Derivation` | Introduce axiom |
| `DModusPonens` | `Derivation → Derivation → Derivation` | From P and P→Q, derive Q |
| `DUnivIntro` | `Derivation → Derivation` | From P, derive ∀x.P |
| `DUnivElim` | `Derivation → Syntax → Derivation` | From ∀x.P, derive P[t/x] |
| `DRefl` | `Syntax → Syntax → Derivation` | Prove Eq T a a |

**Computational Rules:**

| Constructor | Type | Rule |
|-------------|------|------|
| `DCompute` | `Syntax → Derivation` | Prove equality by evaluation |
| `DCong` | `Syntax → Derivation → Derivation` | From a = b, derive f(a) = f(b) |

**Structural Rules:**

| Constructor | Type | Rule |
|-------------|------|------|
| `DInduction` | `Syntax → Derivation → Derivation → Derivation` | Induction with motive, base, step |
| `DCase` | `Derivation → Derivation → Derivation` | Case analysis |
| `DElim` | `Syntax → Derivation → Derivation` | Elimination rule |
| `DInversion` | `Syntax → Derivation` | Inversion principle |
| `DDestruct` | `Syntax → Derivation` | Destructuring |

**Decision Procedure Rules:**

| Constructor | Type | Rule |
|-------------|------|------|
| `DRingSolve` | `Syntax → Derivation` | Ring tactic proof |
| `DLiaSolve` | `Syntax → Derivation` | LIA tactic proof |
| `DOmegaSolve` | `Syntax → Derivation` | Omega tactic proof |
| `DCCSolve` | `Syntax → Derivation` | Congruence closure proof |
| `DSimpSolve` | `Syntax → Derivation` | Simplification proof |
| `DAutoSolve` | `Syntax → Derivation` | Auto tactic proof |

**Rewriting Rules:**

| Constructor | Type | Rule |
|-------------|------|------|
| `DRewrite` | `Derivation → Syntax → Derivation` | Rewrite using equality |
| `DApply` | `Derivation → Derivation` | Apply hypothesis |

#### Safety Guarantees

The kernel enforces two critical safety properties that prevent logical inconsistency:

**Strict Positivity:**

Inductive types must satisfy strict positivity—the type being defined cannot appear in negative position (left of an arrow) in constructor arguments.

```
-- VALID: Nat appears only positively
Inductive Nat := Zero : Nat | Succ : Nat → Nat.  ✓

-- INVALID: Bad appears negatively (left of arrow)
Inductive Bad := MkBad : (Bad → False) → Bad.   ✗
```

Why it matters: Negative occurrences enable Curry's paradox—the type-theoretic equivalent of Russell's paradox. Without positivity checking, you could prove `False` and the entire system becomes unsound.

**Termination Checking:**

Recursive functions (fixpoints) must be structurally decreasing—each recursive call must be on a syntactically smaller argument.

```
-- VALID: Recursive call on k, which is smaller than Succ k
fix add. λm n. match m { Zero → n | Succ k → Succ (add k n) }  ✓

-- INVALID: Recursive call on same or larger argument
fix loop. λn. loop n                                            ✗
fix grow. λn. grow (Succ n)                                     ✗
```

Why it matters: Non-terminating proofs can "prove" anything. Without termination checking, `fix f. f : False` would type-check, breaking soundness.

**The Guardian:**

These checks are enforced by the Guardian—a safety layer that audits all inductive definitions and fixpoints before they enter the kernel. Rejected definitions produce clear error messages explaining the violation.

#### The Vernacular

The kernel supports a text-based command interface:

```
Definition one : Nat := Succ Zero.
Definition inc : Nat -> Nat := fun n : Nat => Succ n.
Check Zero.                              → "Zero : Nat"
Eval (inc Zero).                         → "(Succ Zero)"
Inductive MyBool := Yes : MyBool | No : MyBool.
```

**Commands:**
| Command | Purpose |
|---------|---------|
| `Definition x : T := v.` | Add named definition |
| `Check e.` | Infer and display type |
| `Eval e.` | Normalize and display |
| `Inductive T := C₁ : T₁ \| ...` | Define inductive type |

Arrow syntax `A -> B` desugars to `Π(_:A). B`.

#### Program Extraction

The Forge extracts verified kernel terms to executable Rust code:

```rust
// Kernel definition
Definition add : Nat -> Nat -> Nat := fix f => fun m n =>
  match m { Zero => n | Succ k => Succ (f k n) }.

// Extracted Rust
fn add(m: Nat, n: Nat) -> Nat {
    match m {
        Nat::Zero => n,
        Nat::Succ(k) => Nat::Succ(Box::new(add(*k, n))),
    }
}
```

**Extraction rules:**
| Kernel | Rust |
|--------|------|
| Inductive type | `enum` with `Box` for recursion |
| Fixpoint | Recursive function |
| Match | `match` with auto-deref |
| Application | Function call |

Extracted code compiles and executes—proofs become programs.

#### Oracle Fallback (Z3)

When structural proofs fail, the engine falls back to Z3 as an Oracle. This creates a hybrid architecture:

| Tier | Role | Output |
|------|------|--------|
| **Tier 1: Prover** | Explains "Why" | DerivationTree with inference rules |
| **Tier 2: Oracle** | Checks "Is this valid?" | Z3 verification (sat/unsat) |

**Requirements:** Requires the `verification` feature flag and Z3 installed.

```rust
// Goal: x > 10 → x > 5 (no axioms provided)
// Structural prover cannot derive this, but Z3 knows arithmetic.

let engine = BackwardChainer::new();
let goal = implies(gt(x, 10), gt(x, 5));

let proof = engine.prove(goal).unwrap();
// Returns: OracleVerification("Verified by Z3")
```

**Inductive Safety:** The oracle automatically skips goals containing Peano constructs (`Zero`, `Succ`, `Ctor`, `TypedVar`) since Z3 cannot reason about inductive types without explicit axioms.

#### Theorem Interface

LOGOS is a proof assistant. Write theorem blocks directly in `.logos` files:

```logos
## Theorem: Socrates_Doom
Given: All men are mortal.
Given: All mortals are doomed.
Given: Socrates is a man.
Prove: Socrates is doomed.
Proof: Auto.
```

**Output:**
```
Theorem 'Socrates_Doom' Proved!
└─ [ModusPonens] doomed(Socrates)
   └─ [UniversalInst(Socrates)] mortal(Socrates) → doomed(Socrates)
      └─ [PremiseMatch] ∀x(mortal(x) → doomed(x))
   └─ [ModusPonens] mortal(Socrates)
      └─ [UniversalInst(Socrates)] man(Socrates) → mortal(Socrates)
         └─ [PremiseMatch] ∀x(man(x) → mortal(x))
      └─ [PremiseMatch] man(Socrates)
```

**Proof Strategies:**

| Strategy | Syntax | Description |
|----------|--------|-------------|
| Auto | `Proof: Auto.` | Automatic backward chaining |
| Induction | `Proof: Induction on n.` | Structural induction on variable |
| ByRule | `Proof: ModusPonens.` | Direct rule application |

**Semantic Normalization:** Predicates are automatically lemmatized and lowercased. "men" → "man", "Mortal" = "mortal" = "mortals". This allows natural English ("All men are mortal") without manual canonicalization. Constants like "Socrates" preserve their case.

### Focus Particles

Focus particles like "only", "even", and "just" create alternative semantics:

```
Input:  "Only John ran."
Output: Run(j) ∧ ∀x(Run(x) → x = j)
"John ran, and no one else did"

Input:  "Even John ran."
Output: Run(j) ∧ UNLIKELY(Run(j))
"John ran, which was unexpected"

Input:  "Just three cats slept."
Output: ∃=3x(Cat(x) ∧ Sleep(x))
"Exactly three cats slept"
```

### Morphological Rules

LOGOS recognizes derivational morphology to connect related word forms:

**Agent Nominals (-er, -or):**
```
Input:  "The runner won."
Output: ∃x∃e(Run(x) ∧ Agent(x) ∧ Win(e) ∧ Theme(e, x))
"runner" derived from "run" + agentive -er
```

| Suffix | Meaning | Examples |
|--------|---------|----------|
| -er | Agent who V's | runner, teacher, writer, singer |
| -or | Agent who V's | actor, inventor, sailor, donor |

**Patient Nominals (-ee):**
```
Input:  "The trainee learned."
Output: ∃x∃e(Train(x) ∧ Patient(x) ∧ Learn(e) ∧ Agent(e, x))
"trainee" derived from "train" + patient -ee
```

| Suffix | Meaning | Examples |
|--------|---------|----------|
| -ee | Patient of V | trainee, employee, addressee, nominee |

**Practitioner Nominals (-ist, -ian):**

| Suffix | Meaning | Examples |
|--------|---------|----------|
| -ist | Practitioner of N | scientist, artist, pianist, linguist |
| -ian | Related to N | musician, mathematician, librarian |

**Derivation Tracking:**

The lexicon records derivational relationships:
```json
{
  "Runner": { "derived_from": "Run", "relation": "Agent" },
  "Trainee": { "derived_from": "Train", "relation": "Patient" }
}
```

This enables inference: "All runners are athletes who run."

### Intensionality

LOGOS distinguishes de dicto (narrow scope) and de re (wide scope) readings for intensional contexts:

```
Input: "John seeks a unicorn."

De Dicto (narrow scope):
Seek(j, ^λx.Unicorn(x))
"John seeks something that would be a unicorn"
(No specific unicorn need exist)

De Re (wide scope):
∃x(Unicorn(x) ∧ Seek(j, x))
"There is a specific unicorn that John seeks"
(A particular unicorn exists)
```

Intensional verbs are marked in the lexicon:
- **Opaque**: seek, want, imagine, dream, pretend
- **Factive**: know, realize, regret (presuppose truth)

**The Temperature Paradox:**

LOGOS handles intensional predicates like "rising", "changing", and "increasing" that require intensions as arguments:

```
Input:  "The temperature is ninety."
Output: Temperature = 90  [Extensional identity]

Input:  "The temperature is rising."
Output: Rising(^Temperature)  [Uses intension, not value]
```

Without intensions, substituting "90" for "temperature" would yield `Rising(90)` — absurd. LOGOS prevents this by marking intensional predicates in the lexicon and using the intension (`^Temperature`) rather than the current value.

```
Input:  "The price is changing."
Output: Changing(^Price)

Input:  "The speed is increasing."
Output: Increasing(^Speed)
```

### Wh-Questions

```
Input:  "Who loves Mary?"
Output: λx.Love(x, m)

Input:  "What does John love?"
Output: λx.Love(j, x)

Input:  "Who did John say Mary loves?"
Output: λx.Say(j, [Love(m, x)])
```

**Pied-Piping:** The preposition can move with the wh-word:

```
Input:  "To whom did John talk?"
Output: λx.Talk(j, x)

Input:  "With what did Mary cut the bread?"
Output: λx.∃e(Cut(e) ∧ Agent(e, m) ∧ Theme(e, bread) ∧ Instrument(e, x))
```

### Scope Ambiguity

Quantified sentences can have multiple readings:

```
Input: "Every woman loves a man."

Reading 1 (Surface Scope):
∀x(Woman(x) → ∃y(Man(y) ∧ Love(x, y)))
"Each woman loves some man (possibly different men)"

Reading 2 (Inverse Scope):
∃y(Man(y) ∧ ∀x(Woman(x) → Love(x, y)))
"There is one specific man whom every woman loves"
```

Use `compile_all_scopes()` to get all readings.

### Parse Forests

Ambiguous sentences produce multiple parses. The parser uses backtracking with RAII guards (`ParserGuard`) for memory-safe rollback, and arena allocation (`bumpalo`) for zero-copy AST nodes. Up to 12 distinct readings are returned.

```
Input: "I saw the man with the telescope."

Reading 1 (Instrument):
∃e(See(e) ∧ Agent(e, i) ∧ Theme(e, m) ∧ Instrument(e, t))
"I used the telescope to see him"

Reading 2 (Modifier):
∃e(See(e) ∧ Agent(e, i) ∧ Theme(e, m)) ∧ With(m, t)
"I saw the man who has the telescope"
```

```
Input: "I saw her duck."

Reading 1 (Noun):
See(i, duck)
"I saw her pet duck"

Reading 2 (Verb):
See(i, [Duck(her)])
"I saw her perform a ducking motion"
```

### Garden Path Sentences

Classic parsing challenges that trip up other parsers:

```
Input: "The horse raced past the barn fell."

Analysis:
- First parse: "The horse raced past the barn" (complete sentence?)
- Backtrack: "The horse [that was] raced past the barn" (reduced relative clause)
- Resolution: The horse that was raced past the barn... fell.

Output: ∃x∃e₁∃e₂(Horse(x) ∧ Race(e₁) ∧ Theme(e₁, x) ∧ Past(e₁, barn) ∧ Fall(e₂) ∧ Theme(e₂, x))
```

The parser recovers from initial misparse via RAII guards for memory-safe backtracking.

### Discourse & Pronouns

```
Sentence 1: "John saw Mary."
Sentence 2: "He loves her."

Output: See(j, m) ∧ Love(j, m) ∧ Precedes(e₁, e₂)
```

Pronouns are resolved using gender, number, and discourse context.

**Donkey Anaphora:**

```
Input: "Every farmer who owns a donkey beats it."
Output: ∀x∀y((Farmer(x) ∧ Donkey(y) ∧ Own(x,y)) → Beat(x,y))
```

The indefinite "a donkey" receives universal (not existential) force due to DRS accessibility constraints.

**DRS Accessibility Rules:**

Not all antecedents are accessible for pronoun resolution:

```
Blocked by negation:
"No farmer owns a donkey. *He is happy."
→ Error: "he" has no accessible antecedent

Blocked by disjunction:
"A man or a woman entered. *They left."
→ Error: disjunctive antecedents create scope islands
```

However, universal quantifiers can "telescope" their scope across sentences:
```
"Every chess game has a winner. He is happy."
→ ∀x(ChessGame(x) → ∃y(Winner(y, x) ∧ Happy(y)))
```

**DRS Box Architecture:**

LOGOS uses 8 distinct box types for tracking discourse referent scope:

| Box Type | Created By | Accessibility |
|----------|-----------|---------------|
| Main | Top-level clause | Outward to all |
| ConditionalAntecedent | "if" clause | Into consequent only |
| ConditionalConsequent | "then" clause | Blocked outward |
| NegationScope | "not", "no", "never" | Blocked outward |
| UniversalRestrictor | "every", "all" | Into scope only |
| UniversalScope | Universal body | Blocked outward |
| Disjunct | "or" alternatives | Blocked outward |
| ModalScope | "might", "would" | Into subordinate modals |

**Pronoun Case System:**

LOGOS tracks grammatical case for accurate binding:

| Case | Forms | Usage |
|------|-------|-------|
| Subject | he, she, they, I, we | Nominative position |
| Object | him, her, them, me, us | Accusative position |
| Possessive | his, her, their, my, our | Genitive position |

```
Input: "John saw her. She saw him."
Binding: her → Mary (Object case), She → Mary (Subject case), him → John (Object case)
```

**Referent Source Tracking:**

Each discourse referent records its introduction source:
- `MainClause` — Introduced in top-level assertion
- `ProperName` — Introduced as a named entity
- `ConditionalAntecedent` — Introduced in "if" clause
- `UniversalRestrictor` — Introduced in universal's restriction
- `NegationScope` — Introduced under negation
- `ModalScope` — Introduced in hypothetical world

This enables precise accessibility computation and meaningful error messages when binding fails.

### Modal Subordination

LOGOS handles anaphora across modal contexts using Kripke semantics with world arguments:

```
Input: "A wolf might walk in. It would eat you."

Output:
◇∃x(Wolf(x) ∧ WalkIn(x, w₁)) ∧ □(w₁ → Eat(x, you, w₁))
```

The pronoun "it" in the second sentence refers to the hypothetical wolf introduced in the first—even though that wolf may not exist in the actual world.

**How it works:**
- The modal "might" introduces a possible world w₁
- The pronoun "it" is resolved within that world
- The modal "would" keeps the reference in the subordinate context

```
Input: "John might buy a car. He would drive it to work."

Output:
◇∃x(Car(x) ∧ Buy(j, x, w₁)) ∧ □(w₁ → Drive(j, x, work, w₁))
```

**Modal Flavor and Force:**

LOGOS distinguishes modal flavor (semantic domain) and force (quantificational strength):

| Modal | Flavor | Force | Value |
|-------|--------|-------|-------|
| must | Epistemic/Deontic | Necessity | 1.0 |
| will | Epistemic | Necessity | 1.0 |
| should | Deontic | Weak necessity | 0.8 |
| would | Root | Conditional | 0.7 |
| can | Dynamic | Possibility | 0.5 |
| may | Deontic/Epistemic | Possibility | 0.5 |
| might | Epistemic | Weak possibility | 0.3 |
| could | Dynamic | Remote possibility | 0.3 |

**Subordination Chain Rules:**

Modal subordination follows precedence rules:

| Antecedent | Subordinate | Example |
|------------|-------------|---------|
| might | would | "A wolf might come. It would attack." |
| may | would | "A thief may enter. He would steal." |
| could | would | "John could win. He would celebrate." |
| can | will | "Mary can help. She will arrive." |

The subordinate modal must have equal or higher force to maintain the hypothetical context.

### Sessions & Multi-Turn Discourse

LOGOS supports session-based evaluation for multi-turn interactions via the `Session` API:

```rust
use logicaffeine_language::Session;

let mut session = Session::new();

// Turn 1: Introduce entities
session.eval("The boys lifted the piano.").unwrap();
// → ∃e(Lift(e) ∧ Agent(e, σBoy) ∧ Theme(e, piano))

// Turn 2: Pronoun resolves to entity from Turn 1
session.eval("They smiled.").unwrap();
// → ∃e(Smile(e) ∧ Agent(e, σBoy))
// "They" resolves to "the boys" from previous turn

// Get accumulated logic with temporal ordering
let history = session.history();
// → ... ∧ Precedes(e₁, e₂)
```

**Key Features:**

| Feature | Description |
|---------|-------------|
| Persistent DRS | Discourse referents survive across `eval()` calls |
| Cross-turn anaphora | "A man entered. He sat." — pronoun resolves across turns |
| Modal subordination | "A wolf might enter. It would eat you." — works across turns |
| Temporal ordering | `Precedes(e₁, e₂)` constraints generated automatically |
| Session history | Full accumulated logic via `session.history()` |

**Session API:**

```rust
Session::new()              // Create new session
Session::with_format(fmt)   // Create with output format (Unicode, LaTeX, SimpleFOL)
session.eval(input)         // Evaluate one sentence, returns transpiled logic
session.history()           // Get accumulated logic with Precedes relations
session.turn_count()        // Number of sentences processed
session.reset()             // Clear state, start fresh
```

**Modal Scope Barriers:**

Sessions enforce modal accessibility—hypothetical entities cannot leak into reality:

```
Turn 1: "A wolf might enter."   // Wolf exists in possible world w₁
Turn 2: "He eats you."          // Indicative mode (reality w₀)
        ↓
Error: Pronoun has no accessible antecedent
```

But modal continuation is allowed:

```
Turn 1: "A wolf might enter."   // Wolf in w₁
Turn 2: "It would eat you."     // "would" continues in w₁
        ↓
◇∃x(Wolf(x) ∧ Enter(x)) ∧ □(w₁ → Eat(x, you))
```

### Bridging Anaphora

LOGOS uses ontological knowledge to resolve definite descriptions that lack explicit antecedents:

```
Input: "I bought a car. The engine smoked."

Output:
∃x(Car(x) ∧ Buy(i, x)) ∧ ∃y(Engine(y) ∧ PartOf(y, x) ∧ Smoke(y))
```

The definite "the engine" is resolved via the `PartOf` relation in the ontology—cars have engines.

**Bridging relations:**

| Relation | Example |
|----------|---------|
| PartOf | car → engine, house → roof |
| ContainedIn | room → house, chapter → book |
| MemberOf | player → team, student → class |

```
Input: "Mary walked into the room. The chandelier sparkled."
Output: ∃x(Room(x) ∧ WalkInto(m, x)) ∧ ∃y(Chandelier(y) ∧ In(y, x) ∧ Sparkle(y))
```

### Multi-Word Expressions & Idioms

LOGOS recognizes idioms and compiles them to their semantic meaning:

```
Input:  "John kicked the bucket."
Output: Die(j)

Input:  "Mary spilled the beans."
Output: RevealSecret(m)

Input:  "The fire engine arrived."
Output: ∃e(Arrive(e) ∧ Theme(e, fire_engine))
```

The MWE pipeline uses a trie-based recognizer to merge multi-word units before parsing, handling compound nouns ("fire engine"), phrasal verbs ("give up"), and idiomatic expressions ("kick the bucket").

**Particle Movement:** Phrasal verb particles can be distanced from the verb:

```
Input:  "John gave the idea up."
Output: GiveUp(j, idea)

Input:  "Mary picked her friend up."
Output: ∃e(PickUp(e) ∧ Agent(e, m) ∧ Theme(e, friend))
```

Both "gave up the idea" and "gave the idea up" produce the same semantics.

**Phrasal Verb Equivalence Mapping:**

The lexicon maps 17+ phrasal verbs to their semantic equivalents:

| Phrasal Verb | Semantic Equivalent | Vendler Class |
|--------------|---------------------|---------------|
| give up | Surrender | Achievement |
| break down | Malfunction | Achievement |
| turn on | Activate | Achievement |
| turn off | Deactivate | Achievement |
| pick up | Collect | Achievement |
| put down | Place | Achievement |
| take off | Depart | Achievement |
| bring up | Mention | Achievement |
| find out | Discover | Achievement |
| work out | Solve | Accomplishment |
| carry out | Execute | Accomplishment |
| look up | Search | Activity |
| look after | Care | Activity |
| run into | Encounter | Achievement |
| come across | Find | Achievement |
| get along | Cooperate | Activity |
| put up with | Tolerate | State |

**Trie-Based Recognition:**

The MWE pipeline uses a trie (prefix tree) for efficient recognition:

1. **Tokenize** — Split input into tokens
2. **Trie lookup** — Match multi-token sequences
3. **Merge** — Replace matched sequences with single compound token
4. **Parse** — Process merged token stream

This enables O(n) recognition regardless of MWE dictionary size.

### Category Shift

LOGOS handles noun-to-verb conversions (denominal verbs):

```
Input:  "The committee tabled the discussion."
Output: ∃e(Table(e) ∧ Agent(e, committee) ∧ Theme(e, discussion))

Input:  "John googled the answer."
Output: ∃e(Google(e) ∧ Agent(e, j) ∧ Theme(e, answer))
```

The system recognizes when nouns are used as verbs and applies appropriate event semantics.

### Reciprocals

LOGOS expands reciprocal constructions into bidirectional predication:

```
Input:  "John and Mary love each other."
Output: Love(j, m) ∧ Love(m, j)

Input:  "John and Mary saw each other."
Output: See(j, m) ∧ See(m, j)
```

The reciprocal "each other" is never left unexpanded—it always produces symmetric predication.

### Ellipsis

LOGOS reconstructs elided material in three types of ellipsis:

**VP Ellipsis:**
```
Input:  "John runs. Mary does too."
Output: Run(j) ∧ Run(m)

Input:  "John can swim. Mary can too."
Output: ◇Swim(j) ∧ ◇Swim(m)

Input:  "John runs. Mary does not."
Output: Run(j) ∧ ¬Run(m)
```

**Gapping:**
```
Input:  "John ate an apple, and Mary a banana."
Output: Eat(j, apple) ∧ Eat(m, banana)

Input:  "John ran yesterday, and Mary today."
Output: ∃e₁(Run(e₁) ∧ Agent(e₁, j) ∧ Time(e₁, yesterday)) ∧
        ∃e₂(Run(e₂) ∧ Agent(e₂, m) ∧ Time(e₂, today))
```

**Ditransitive Gapping:**
```
Input:  "John gave Mary a book, and Sue a pen."
Output: Give(j, m, book) ∧ Give(j, s, pen)

Input:  "John walked to the park, and Mary to the school."
Output: ∃e₁(Walk(e₁) ∧ Agent(e₁, j) ∧ Goal(e₁, park)) ∧
        ∃e₂(Walk(e₂) ∧ Agent(e₂, m) ∧ Goal(e₂, school))
```

Gapping reconstructs the elided verb from the antecedent clause, with temporal modifiers replaced rather than copied.

**PP Gapping:**
```
Input:  "John spoke to Mary, and Bill to Sue."
Output: ∃e₁(Speak(e₁) ∧ Agent(e₁, j) ∧ Goal(e₁, m)) ∧
        ∃e₂(Speak(e₂) ∧ Agent(e₂, b) ∧ Goal(e₂, s))

Input:  "John put the book on the table, and Mary the pen on the desk."
Output: ∃e₁(Put(e₁) ∧ Agent(e₁, j) ∧ Theme(e₁, book) ∧ Location(e₁, table)) ∧
        ∃e₂(Put(e₂) ∧ Agent(e₂, m) ∧ Theme(e₂, pen) ∧ Location(e₂, desk))
```

**Gapping Reconstruction Mechanics:**

LOGOS uses template-based reconstruction with role preservation:

1. **Parse antecedent** — Extract verb and thematic role structure
2. **Identify remnants** — Match remaining NPs/PPs to roles
3. **Reconstruct** — Apply antecedent template with new role fillers
4. **Override temporals** — Temporal adverbs replace rather than copy

| Role Type | Behavior |
|-----------|----------|
| Agent | Replaced by remnant subject |
| Theme/Patient | Replaced by remnant object |
| Goal/Location | Replaced by remnant PP |
| Temporal | Replaced (not copied) |
| Manner | Copied from antecedent |

**Sluicing:**
```
Input:  "Someone left. I know who."
Output: ∃x(Leave(x)) ∧ Know(speaker, λy.Leave(y))

Input:  "John ate something. I know what."
Output: ∃x(Eat(j, x)) ∧ Know(speaker, λy.Eat(j, y))
```

### Topicalization

LOGOS handles filler-gap dependencies where the object moves to sentence-initial position:

```
Input:  "The apple, John ate."
Output: ∃e(Eat(e) ∧ Agent(e, j) ∧ Theme(e, apple))

Input:  "The red apple, John ate."
Output: ∃e(Eat(e) ∧ Agent(e, j) ∧ Theme(e, x)) ∧ Apple(x) ∧ Red(x)

Input:  "A book, Mary read."
Output: ∃x∃e(Book(x) ∧ Read(e) ∧ Agent(e, m) ∧ Theme(e, x))
```

The topicalized constituent is correctly identified as the object (Theme) despite appearing before the subject.

### Passive Voice

LOGOS handles passive constructions with proper thematic role reassignment:

```
Input:  "The apple was eaten."
Output: ∃e(Eat(e) ∧ Theme(e, apple))

Input:  "The apple was eaten by John."
Output: ∃e(Eat(e) ∧ Agent(e, j) ∧ Theme(e, apple))

Input:  "The apple would have been being eaten."
Output: □(Perf(Prog(∃e(Eat(e) ∧ Theme(e, apple)))))
```

In passive voice:
- The grammatical subject becomes the Theme (patient)
- The optional "by"-phrase provides the Agent
- Complex aspect chains are preserved

### Respectively

The "respectively" adverb triggers pairwise conjunction of coordinated lists:

```
Input:  "John and Mary saw Tom and Jerry respectively."
Output: ∃e₁(See(e₁) ∧ Agent(e₁, j) ∧ Theme(e₁, t)) ∧
        ∃e₂(See(e₂) ∧ Agent(e₂, m) ∧ Theme(e₂, jerry))

Input:  "Alice, Bob, and Carol love Dave, Eve, and Frank respectively."
Output: Love(a, d) ∧ Love(b, e) ∧ Love(c, f)
```

**Length mismatch produces a semantic error:**
```
Input:  "John and Mary saw Tom respectively."
Error:  "Respectively requires equal-length lists (2 subjects, 1 object)"
```

### Control & Raising Verbs

LOGOS handles control and raising constructions with proper argument sharing:

**Subject Control** — The matrix subject controls the embedded clause subject:
```
Input:  "John wants to leave."
Output: Want(j, [Leave(j)])
"John wants [PRO to leave], where PRO = John"

Input:  "John tried to swim."
Output: Try(j, [Swim(j)])
```

**Object Control** — The matrix object controls the embedded clause subject:
```
Input:  "John persuaded Mary to leave."
Output: Persuade(j, m, [Leave(m)])
"John persuaded Mary [PRO to leave], where PRO = Mary"

Input:  "John told Mary to run."
Output: Tell(j, m, [Run(m)])
```

**Raising** — The embedded subject "raises" to matrix subject position:
```
Input:  "John seems to sleep."
Output: Seem([Sleep(j)])
"[John to sleep] seems" — John is semantically the sleeper
```

### Presupposition Triggers

LOGOS detects presupposition triggers and marks presupposed content:

**Change-of-state verbs:**
```
Input:  "John stopped smoking."
Output: Stop(j, Smoke) ∧ PRESUPPOSE(PAST(Smoke(j)))
"John stopped smoking" presupposes "John used to smoke"

Input:  "Mary continued running."
Output: Continue(m, Run) ∧ PRESUPPOSE(PROG(Run(m)))
```

**Factive verbs:**
```
Input:  "John regrets leaving."
Output: Regret(j, Leave) ∧ PRESUPPOSE(Leave(j))
"Regret" presupposes the truth of its complement

Input:  "Mary realized that John left."
Output: Realize(m, [Leave(j)]) ∧ PRESUPPOSE(Leave(j))
```

| Trigger Type | Examples | Presupposition |
|--------------|----------|----------------|
| Change-of-state | stop, start, continue | Prior state held |
| Factive | know, regret, realize | Complement is true |
| Iterative | again, return | Event occurred before |

### Negative Polarity Items

LOGOS correctly interprets "any" based on its licensing context:

**NPI "any" in negative contexts (existential):**
```
Input:  "Not any dogs run."
Output: ¬∃x(Dog(x) ∧ Run(x))
"any" → existential under negation

Input:  "No one saw anything."
Output: ∀x(Person(x) → ¬∃y(Saw(x, y)))
```

**Free-choice "any" in positive contexts (universal):**
```
Input:  "Any dog runs."
Output: ∀x(Dog(x) → Run(x))
"any" → universal in positive context

Input:  "If any dog barks, John runs."
Output: ∀x((Dog(x) ∧ Bark(x)) → Run(j))
"any" → universal in conditional antecedent
```

The system tracks negative depth to determine when NPIs like "any", "anything", and "anyone" are licensed.

### Semantic Sorts & Metaphor Detection

LOGOS assigns semantic sorts to entities and detects metaphor via sort violations.

**Full Sort Hierarchy:**

LOGOS uses a 9-sort ontology with 500+ classified nouns:

| Sort | Examples | Count |
|------|----------|-------|
| Human | John, Mary, doctor, farmer, teacher, student | ~100 |
| Animal | dog, cat, bird, horse, whale, spider | ~80 |
| Physical | table, rock, apple, car, book, chair | ~150 |
| Abstract | time, love, freedom, justice, beauty | ~60 |
| Celestial | sun, moon, star, planet, galaxy | ~15 |
| Value | money, price, cost, wealth, debt | ~20 |
| Place | park, room, city, country, ocean | ~40 |
| Event | party, meeting, war, concert, game | ~30 |
| Group | team, committee, family, crowd, herd | ~25 |

**Animacy Features:**

Beyond sorts, LOGOS tracks animacy for selectional restrictions:

| Feature | Examples | Behavior |
|---------|----------|----------|
| Animate | person, dog, bird | Can be Agent of volitional verbs |
| Inanimate | rock, table, book | Cannot "want", "think", etc. |

```
Input:  "The rock thinks."
Output: SortViolation: "think" requires Animate subject
```

> *Did you know that a software developer's job is primarily to teach rocks to think? This is why "wizard" is synonymous with "programmer".*

**Predicate Sort Requirements:**

Predicates specify required sorts for their arguments:

| Predicate | Required Sort | Violation Example |
|-----------|---------------|-------------------|
| happy, sad, angry | Animate | *"The table is happy" |
| think, believe, remember | Animate | *"The rock believes" |
| melt, evaporate | Physical | *"Love melts" (metaphor) |
| shine, orbit | Celestial/Physical | "The sun shines" ✓ |

**Literal predication (sorts compatible):**
```
Input:  "John is a man."
Output: Man(j)
Human/Human — no metaphor

Input:  "The king is bald."
Output: Bald(king)
Human/Property — no metaphor
```

**Metaphor detection (sort violation):**
```
Input:  "Juliet is the sun."
Output: Metaphor(j ≈ sun)
Human/Celestial mismatch triggers metaphor

Input:  "Time is money."
Output: Metaphor(time ≈ money)
Abstract/Value mismatch triggers metaphor
```

### Counterfactual Conditionals

LOGOS distinguishes indicative from counterfactual conditionals via subjunctive mood:

**Indicative conditional:**
```
Input:  "If John runs, Mary walks."
Output: Run(j) → Walk(m)
```

**Counterfactual conditional:**
```
Input:  "If John were a bird, he would fly."
Output: □(Bird(j) →_CF Fly(j))
"were" + "would" triggers counterfactual reading

Input:  "If Mary had left, John would have stayed."
Output: □(Leave(m) →_CF Stay(j))
"had" + "would have" triggers counterfactual
```

Counterfactuals are marked with the counterfactual conditional operator (→_CF) to distinguish them from material implication.

### Weather Verbs

LOGOS handles impersonal weather constructions where "it" is an expletive (non-referential):

```
Input:  "It rains."
Output: Rain()
No argument — "it" is expletive

Input:  "It is snowing."
Output: PROG(Snow())

Input:  "It will thunder."
Output: FUT(Thunder())
```

Weather verbs are marked in the lexicon and take no true subject—the "it" is purely syntactic.

### Imperatives

Imperatives have an implicit addressee as subject:

```
Input:  "Run!"
Output: Run(addressee)

Input:  "Close the door."
Output: ∃e(Close(e) ∧ Agent(e, addressee) ∧ Theme(e, door))
```

The addressee is a deictic element bound to the hearer in context.

### Reflexive Binding

LOGOS handles reflexive pronouns with proper binding constraints:

```
Input:  "John saw himself."
Output: See(j, j)
Reflexive "himself" bound to subject

Input:  "Mary hurt herself."
Output: Hurt(m, m)

Input:  "The men helped themselves."
Output: ∀x(x ∈ men → Help(x, x))
```

Reflexives must be bound within their local domain (Binding Principle A).

---

## The CLI: largo

The `largo` command-line tool manages LOGOS projects:

```bash
# Build CLI locally
cargo build --features cli

# Or install from source
cargo install --path . --features cli

# Create a new project
largo new my-project
cd my-project

# Build and run
largo build
largo run

# Type checking only
largo check

# Publish to the registry
largo publish --token $LOGOS_TOKEN

# Verify with Z3 (Pro+ license required)
largo verify --license $LOGOS_LICENSE
```

Project structure:
```
my-project/
├── Logos.toml      # Project manifest
├── src/
│   └── main.logos  # Entry point
└── tests/
```

---

## API Reference

### Core Functions

```rust
// Logic Mode
pub fn compile(input: &str) -> Result<String, ParseError>
pub fn compile_all_scopes(input: &str) -> Result<Vec<String>, ParseError>
pub fn compile_forest(input: &str) -> Vec<String>

// Imperative Mode
pub fn compile_to_rust(input: &str) -> Result<String, ParseError>
pub fn compile_to_dir(input: &str, output: &Path) -> Result<(), CompileError>

// Output Formats
pub fn compile_with_options(input: &str, opts: CompileOptions) -> Result<String, ParseError>

// Theorem Proving
pub fn compile_theorem(input: &str) -> Result<String, ProofError>

// Session (Multi-Turn Discourse)
pub struct Session { ... }
impl Session {
    pub fn new() -> Self
    pub fn with_format(format: OutputFormat) -> Self
    pub fn eval(&mut self, input: &str) -> Result<String, ParseError>
    pub fn history(&self) -> String
    pub fn turn_count(&self) -> usize
    pub fn reset(&mut self)
}
```

### Output Formats

| Format | Example | Use Case |
|--------|---------|----------|
| `Unicode` | `∀x(Dog(x) → Bark(x))` | Display, terminals |
| `LaTeX` | `\forall x (Dog(x) \supset Bark(x))` | Academic papers |
| `SimpleFOL` | `ALL x (Dog(x) IMP Bark(x))` | ASCII-only |

```rust
let options = CompileOptions { format: OutputFormat::LaTeX };
let latex = compile_with_options("All cats sleep.", options).unwrap();
```

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Logicaffeine Pipeline                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  English Input                                                              │
│       │                                                                     │
│       ▼                                                                     │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────────┐              │
│  │  Lexer  │───▶│   MWE   │───▶│ Parser  │───▶│     AST     │              │
│  │         │    │Pipeline │    │         │    │             │              │
│  └─────────┘    └─────────┘    └─────────┘    └──────┬──────┘              │
│                                                       │                     │
│                          ┌────────────────────────────┼───────────────┐     │
│                          │                            │               │     │
│                          ▼                            ▼               │     │
│                   ┌─────────────┐              ┌─────────────┐        │     │
│                   │  Semantics  │              │   Codegen   │        │     │
│                   │  (λ-calc)   │              │   (Rust)    │        │     │
│                   └──────┬──────┘              └──────┬──────┘        │     │
│                          │                            │               │     │
│                          ▼                            ▼               │     │
│                   ┌─────────────┐              ┌─────────────┐        │     │
│                   │ Transpiler  │              │   Compile   │        │     │
│                   │  (FOL)      │              │   (cargo)   │        │     │
│                   └──────┬──────┘              └──────┬──────┘        │     │
│                          │                            │               │     │
│                          ▼                            ▼               │     │
│                   ┌─────────────┐              ┌─────────────┐        │     │
│                   │   Logic     │              │  Executable │        │     │
│                   │   Output    │              │   Binary    │        │     │
│                   └─────────────┘              └─────────────┘        │     │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Crate Architecture

Logicaffeine uses a tiered crate design:

| Tier | Crate | Purpose |
|------|-------|---------|
| 0 | `logicaffeine_base` | Arena allocation, tokens, spans |
| 0 | `logicaffeine_kernel` | Pure type theory (Calculus of Constructions) |
| 0 | `logicaffeine_data` | CRDTs, data structures (WASM-safe, no IO) |
| 1 | `logicaffeine_lexicon` | English vocabulary (compiled from lexicon.json) |
| 1 | `logicaffeine_system` | Platform IO: networking, persistence, VFS |
| 2 | `logicaffeine_language` | Parser, AST, FOL transpiler |
| 2 | `logicaffeine_compile` | Rust codegen, interpreter |
| 3 | `logicaffeine_proof` | Backward-chaining proof engine |
| 3 | `logicaffeine_verify` | Z3 verification (optional, excluded from workspace) |

**Applications:**
- `apps/logicaffeine_cli` - The `largo` command-line tool
- `apps/logicaffeine_web` - Browser-based IDE (Dioxus/WASM)

The main `logos` crate re-exports all functionality for backwards compatibility.

### Design Highlights

- **Arena Allocation**: Uses `bumpalo` for efficient AST nodes
- **Symbol Interning**: All strings interned for fast comparison
- **ParserGuard**: RAII pattern for automatic backtracking
- **Parse Forests**: Up to 12 readings for ambiguous inputs
- **Neo-Davidsonian Events**: Thematic roles (Agent, Patient, Theme)

---

## Testing

The test suite covers 2200+ tests organized by category:

| Category | Coverage |
|----------|----------|
| Core Syntax | Garden path sentences, polarity, tense, movement, wh-questions |
| Advanced Semantics | Degrees, sorts, ontology, multi-word expressions, ambiguity |
| Extended Phenomena | Negation, aspect, plurality, axioms |
| Code Generation | Blocks, scoping, types, ownership, runtime |
| Type System | Collections, structs, functions, enums, inductive types, modules |
| Formal Semantics | Event adjectives, DRS, distributivity, intensionality |
| Systems | Agents, networking, CRDTs, security, concurrency |
| Proof Engine | Backward chaining, unification, induction, kernel, certifier, extraction |
| Memory | Arena allocation, zones |
| CRDTs | Serialization, delta, stress tests, edge cases |
| Modals | Modal subordination, Kripke semantics |

**End-to-End Tests:**
- `e2e_collections.rs` - Push, pop, length, slicing
- `e2e_functions.rs` - Recursion, multi-parameter
- `e2e_structs.rs` - User-defined types
- `e2e_enums.rs` - Pattern matching
- `phase102_bridge.rs` - Recursive inductive types
- `grand_challenge_mergesort.rs` - Full algorithm compilation

```bash
# Run all tests
cargo test

# Run a specific test suite
cargo test --test e2e_functions

# Run with output
cargo test -- --nocapture
```

---

## Glossary

| Term | Definition |
|------|------------|
| **Arena Allocation** | Memory allocation strategy where objects are allocated in a contiguous region and freed all at once |
| **Backward Chaining** | Goal-directed proof search that works from the conclusion to axioms |
| **Beta-Reduction** | Lambda calculus computation: (λx.P)(a) → P[x:=a]. Reduces function application by substitution |
| **Bridging Anaphora** | Resolution of definite descriptions via world knowledge (e.g., "the engine" after mentioning "a car") |
| **Calculus of Constructions (CoC)** | Type theory unifying proofs and programs; foundation for proof assistants like Coq |
| **Certifier** | Converts derivation trees to kernel terms; proofs become type-checkable lambda terms |
| **Collective Predicate** | Predicate applying to groups as wholes ("gather", "meet"), not individuals |
| **CRDT** | Conflict-free Replicated Data Type - data structures that merge automatically without coordination |
| **Cumulativity** | Universe subtyping: Type₀ ≤ Type₁ ≤ Type₂; lower levels usable where higher expected |
| **Curry-Howard Correspondence** | Isomorphism between proofs and programs, propositions and types |
| **De Dicto / De Re** | Narrow scope (conceptual) vs. wide scope (referential) readings of intensional contexts |
| **Dependent Type** | Type that depends on a value; ΠA:Type. A→A depends on the type A |
| **Derivation Tree** | Recursive proof structure showing inference steps from axioms to conclusion |
| **Delta Reduction (δ)** | Unfolding global definitions during normalization; `two → Succ(Succ(Zero))` |
| **DElim** | Generic elimination principle for inductive types; takes motive and case chain to prove properties by structural induction |
| **Distributed<T>** | CRDT wrapper combining persistence AND network sync; journals both local and remote updates |
| **Distributive Predicate** | Predicate applying to individuals separately ("sleep"), not groups |
| **DRS** | Discourse Representation Structure - formal framework for tracking entities and relations across sentences |
| **Elimination Rule** | Rule for consuming inductive values via pattern matching (match) |
| **First-Order Logic (FOL)** | Formal system using quantifiers (∀, ∃), predicates, and logical connectives |
| **Fixpoint (Fix)** | Recursive term combinator; `fix f. body` where f refers to the whole term for recursion |
| **Focus Particle** | Words like "only", "even", "just" that invoke alternatives and presuppositions |
| **Formation Rule** | Rule declaring a type exists (e.g., Nat : Type₀) |
| **GossipSub** | Pub/sub protocol for P2P message propagation used for CRDT synchronization |
| **Inductive Type** | Type defined by its constructors; values built only via introduction rules |
| **Introduction Rule** | Rule for constructing values (e.g., Zero : Nat, Succ : Nat → Nat) |
| **Iota Reduction** | Pattern matching computation: match on constructor selects branch and substitutes bindings |
| **Kernel Primitives** | Native hardware types (Int, Float, Text) with O(1) arithmetic; enables verification of large numbers without Peano overhead |
| **Kripke Semantics** | Possible worlds framework for modal logic; used for modal subordination |
| **Lambda Calculus** | Formal system for function abstraction and application, used for compositional semantics |
| **Leibniz's Law** | Indiscernibility of identicals: if a = b, then P(a) implies P(b); implemented via Eq_rec |
| **Link's Logic of Plurals** | Framework classifying predicates as distributive, collective, or mixed |
| **Miller Pattern Unification** | Decidable fragment of higher-order unification where holes are applied to distinct bound variables |
| **Modal Subordination** | Anaphora resolution across modal contexts ("A wolf might come in. It would eat you.") |
| **MWE** | Multi-Word Expression - phrases that behave as single units ("fire engine", "kick the bucket") |
| **Neo-Davidsonian** | Event semantics using event variables with thematic roles (Agent, Patient, Theme) |
| **NPI** | Negative Polarity Item - words like "any" that require negative/downward-entailing contexts |
| **Oracle** | Z3-based fallback verification when structural proofs fail |
| **Parse Forest** | Collection of all valid parse trees for an ambiguous sentence |
| **Pipe** | Go-style channel for CSP concurrency; typed, unbuffered by default |
| **Pi Type (Π)** | Dependent function type: Πx:A. B(x) where B can mention x |
| **Polymorphic Inductive Type** | Inductive type parameterized by type variables; `List (A : Type)` creates a family of types |
| **Positivity Checking** | Ensures inductive types don't appear negatively in constructors; prevents Curry's paradox |
| **Prelude** | Standard library of fundamental types (Nat, Eq, True, False, And, Or) |
| **Privative Adjective** | Adjective negating the noun ("fake gun" → not a gun) |
| **Propositional Equality** | Type `Eq A x y` inhabited only when x equals y; proof via `refl` |
| **Program Extraction** | Translating verified kernel terms to executable code (Rust); proofs become programs |
| **Reflection** | Deep embedding of kernel syntax as data (Syntax type); enables tactics, Gödel numbering, and self-reference |
| **Scope Ambiguity** | When quantifiers can be ordered in multiple ways, yielding different meanings |
| **Structural Induction** | Proof technique for inductive types (Nat, List) using base case and step case |
| **Symbol Interning** | Storing strings once and referring to them by index for efficiency |
| **Termination Checking** | Ensures recursive functions decrease on a structural argument; prevents infinite loops |
| **Thematic Role** | Semantic relationship between verb and argument (Agent, Patient, Theme, Goal, etc.) |
| **Theorem Block** | LOGOS syntax for declaring provable statements with Given premises and Prove goal |
| **Type Parameter** | Variable ranging over types in polymorphic definitions; `A` in `List A` |
| **Alpha-Equivalence** | Principle that bound variable names are arbitrary; ∃e P(e) ≡ ∃x P(x) |
| **Unification** | Algorithm finding substitutions to make terms identical; core of proof engines |
| **Universe** | Hierarchy of types: Prop : Type₁ : Type₂ : ... preventing paradoxes |
| **Vernacular** | Human-readable command language for interacting with the kernel (Definition, Check, Eval, Inductive) |
| **Vendler Class** | Aspectual classification: State, Activity, Accomplishment, Achievement, Semelfactive |
| **Zone** | Memory arena with deterministic deallocation; all contents freed when zone exits |
| **Fun Fact** | This was built because the developer was tired of sitting around waiting for the resources to build it, so he just built it. A vision 10 years in the making. Development for the project began on December 22nd 2025. |

---

## Theoretical Foundations

LOGOS builds on decades of formal semantics research:

- **Montague Grammar** (1970s) — Compositional semantics via λ-calculus
- **Discourse Representation Theory** (Kamp, 1981) — Anaphora and presupposition
- **Neo-Davidsonian Event Semantics** (Parsons, 1990) — Thematic roles
- **Generalized Quantifier Theory** (Barwise & Cooper, 1981) — Scope ambiguity
- **Vendler Aspectual Classes** (1957) — Tense and aspect composition
- **Link's Logic of Plurals** (1983) — Distributive vs. collective predication
- **Kripke Semantics** (1959) — Possible worlds for modal logic
- **Alternative Semantics** (Rooth, 1985) — Focus particles and alternatives

The test suite includes classic examples from the formal semantics literature: donkey anaphora, garden paths, scope islands, control theory, modal subordination, and bridging.

---

## Further Reading

**Documentation**
- **[Language Guide](https://logicaffeine.com/guide)** — Interactive tutorial with live REPL
- **[SPECIFICATION.md](SPECIFICATION.md)** — Complete language specification (5000+ lines)
- **[LOGOS_DOCUMENTATION.md](LOGOS_DOCUMENTATION.md)** — Full technical documentation

**Project**
- **[CHANGELOG.md](CHANGELOG.md)** — Version history (v0.8.12: Latest release)
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — How to contribute (TDD workflow)
- **[SECURITY.md](SECURITY.md)** — Report security vulnerabilities
- **[ROADMAP.md](ROADMAP.md)** — Development direction

**For AI Contributors**
- **[CLAUDE.md](CLAUDE.md)** — AI assistant guidelines

---

## License

**Business Source License 1.1**

- **Licensor:** Brahmastra Labs LLC
- **Change Date:** 2029-12-24 (converts to MIT)
- **Additional Use Grant:** Free for individuals and organizations with fewer than 25 employees, except for commercial "Logic Service" offerings

See [LICENSE.md](LICENSE.md) for full terms.

---

**Logicaffeine** | [Try Online](https://logicaffeine.com/guide) | [Docs](SPECIFICATION.md) | [Changelog](CHANGELOG.md) | [Contribute](CONTRIBUTING.md)

*In the beginning was the Word, and the Word was with Logic, and the Word was Code.*

