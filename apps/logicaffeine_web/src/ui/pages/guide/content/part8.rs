//! Part VIII: Reference — sections: quick-reference, appendix-keywords, appendix-errors, appendix-runnability.

use super::{CodeExample, ExampleMode, Runnability, Section};

pub(super) const PART_8: &[Section] = &[
    Section {
        id: "quick-reference",
        number: 40,
        title: "Quick Reference",
        part: "Part VIII: Reference",
        content: r#"
### Syntax Cheat Sheet

**Variables:**
- `Let x be 5.` — Create variable
- `Set x to 10.` — Change variable
- `Let x: Int be 5.` — With type annotation

**Control Flow:**
- `If condition:` ... `Otherwise:` — Conditional
- `While condition:` — While loop
- `Repeat for item in items:` — For-each loop
- `Return value.` — Return from function

**Functions:**
- `## To name (param: Type) -> ReturnType:` — Define function

**Structs:**
- `A TypeName has:` ... — Define struct
- `Let x be a new TypeName with field1 value1.` — Create instance
- `x's field` — Access field

**Enums:**
- `A TypeName is either:` ... — Define enum
- `Inspect x: When Variant:` ... — Pattern match

**Primitive Types:**

| Type | Description | Examples |
|------|-------------|----------|
| `Int` | Whole numbers | `5`, `-10`, `0` |
| `Bool` | True or false | `true`, `false` |
| `Text` | Strings | `"Hello"`, `""` |
| `Real` | Decimals | `3.14`, `-0.5` |
| `Char` | Single character | backtick syntax |
| `Byte` | 8-bit unsigned | `42: Byte`, `255: Byte` |

**Lists (Seq):**
- `[1, 2, 3]` — List literal
- `item 1 of items` or `items[1]` — Access (1-indexed)
- `Push value to items.` — Add to end
- `length of items` — Get length

**Maps:**
- `Map of K to V` — Map type (key-value pairs)
- `a new Map of Text to Int` — Create empty map
- `item "key" of map` or `map["key"]` — Get value by key
- `Set item "key" of map to val.` or `Set map["key"] to val.` — Set value

**Sets:**
- `Set of T` — Set type (unique elements)
- `a new Set of Int` — Create empty set
- `Add x to set.` — Add element
- `Remove x from set.` — Remove element
- `set contains x` — Check membership
- `a union b` — Elements in either set
- `a intersection b` — Elements in both sets

**Tuples:**
- `(1, "two", 3.0)` — Tuple literal (mixed types allowed)
- `t[1]` or `item 1 of t` — Access (1-indexed)
- `length of t` — Get tuple size

### Ownership Verbs

| Verb | Meaning |
|------|---------|
| `Give x to f.` | Move ownership |
| `Show x to f.` | Borrow (read) |
| `Let f modify x.` | Mutable borrow |
| `copy of x` | Clone |

### Zones

**Basic syntax:**
- `Inside a zone called "Name":` — 4KB default zone
- `Inside a zone called "Name" of size 2 MB:` — Sized heap zone
- `Inside a zone called "Name" mapped from "file.bin":` — Memory-mapped file

**Size units:** B, KB, MB, GB

### Concurrency

**Async I/O:**
- `Attempt all of the following:` — Concurrent async tasks (tokio::join!)

**Parallel CPU:**
- `Simultaneously:` — Parallel computation (rayon/threads)

**Tasks:**
- `Launch a task to f(args).` — Fire-and-forget spawn
- `Let h be Launch a task to f(args).` — Spawn with handle
- `Stop h.` — Abort a running task

**Channels/Pipes:**
- `Let p be a new Pipe of Int.` — Create bounded channel
- `Send x into p.` — Blocking send
- `Receive x from p.` — Blocking receive
- `Try to send/receive` — Non-blocking variants

**Select:**
- `Await the first of:` — Race multiple operations
- `Receive x from p:` — Channel receive branch
- `After N seconds:` — Timeout branch

### Distributed Types (CRDTs)

**Shared Structs:**
- `A Counter is Shared and has:` — CRDT-enabled struct

**CRDT Field Types:**
- `ConvergentCount` — Grow-only counter (`Increase`)
- `Tally` — Bidirectional counter (`Increase`, `Decrease`)
- `LastWriteWins of T` — Timestamp-based register (`Set`)
- `Divergent T` — Multi-value register (`Set`, `Resolve`)
- `SharedSet of T` — Add/remove set (`Add`, `Remove`, `contains`)
- `SharedSet (AddWins) of T` — Set where add wins conflicts
- `SharedSet (RemoveWins) of T` — Set where remove wins conflicts
- `SharedSequence of T` — Ordered list RGA (`Append`)
- `CollaborativeSequence of T` — Text-optimized YATA (`Append`)
- `SharedSequence (YATA) of T` — Alternate YATA syntax
- `SharedMap from K to V` — Key-value CRDT (`[]` access)
- `ORMap from K to V` — Alternate map syntax

**CRDT Operations:**
- `Increase x's field by amount.` — Increment counter
- `Decrease x's field by amount.` — Decrement Tally
- `Set x's field to value.` — Set register value
- `Resolve x's field to value.` — Resolve Divergent conflict
- `Add value to x's field.` — Add to SharedSet
- `Remove value from x's field.` — Remove from SharedSet
- `Append value to x's field.` — Append to sequence
- `Merge source into target.` — Combine two CRDT instances

**Persistence (Compiled Only):**
- `Persistent Counter` — Type with automatic journaling
- `Let x be mounted at "data.lsf".` — Load/create persistent CRDT
- `Mount x at "path".` — Mount statement for persistence

**Network Sync:**
- `Sync mutable_var on "topic".` — Subscribe to GossipSub topic for auto-sync (real peers when compiled; local single-node mode in the playground)

### P2P Networking

**Server/Client:**
- `Listen on "/ip4/0.0.0.0/tcp/8000".` — Bind to address
- `Listen on "/ip4/0.0.0.0/tcp/0".` — Listen on any available port
- `Connect to addr.` — Dial a peer
- `Let remote be a PeerAgent at addr.` — Create remote handle
- `Send msg to remote.` — Transmit message

**Portable Types:**
- `A Message is Portable and has:` — Network-serializable struct

**Automatic Discovery:**
- mDNS auto-discovers peers on local network when you Listen
- Peers are automatically connected when discovered

**GossipSub (via Sync):**
- Topics broadcast state changes to all subscribers
- Retry with exponential backoff (1s, 2s, 4s, 8s, 16s)

**File Transfer:**
- FileSipper for chunked transfers (1 MB chunks)
- FileManifest with SHA256 hashes for verification
- Enables resumable transfers

### Security

**Policy Blocks:**
- `## Policy` — Define security rules
- `A User is admin if...` — Define a predicate
- `A User can edit the Doc if...` — Define a capability

**Security Enforcement:**
- `Check that user is admin.` — Mandatory runtime check (never optimized out)
- `Assert that x > 0.` — Debug-only assertion (can be optimized out)

### Logic Mode Symbols

| English | Symbol |
|---------|--------|
| All | `∀` |
| Some | `∃` |
| and | `∧` |
| or | `∨` |
| not | `¬` |
| if...then | `→` |
| can/may | `◇` |
| must | `□` |
"#,
        hook: "",
        objectives: &[
            "identify the syntax for any construct in under a minute",
            "translate between English and bracket spellings at a glance",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[],
        recap: &[],
    },

    Section {
        id: "appendix-keywords",
        number: 41,
        title: "Appendix: Every Operator, Both Spellings",
        part: "Part VIII: Reference",
        content: r#"
The complete operator vocabulary — every English form and its symbolic twin. Neither
spelling is more "real"; they compile identically.

| English | Symbol | Meaning |
|---------|--------|---------|
| `plus` | `+` | addition / Text concat |
| `minus` | `-` | subtraction |
| `times` | `*` | multiplication |
| `divided by` | `/` | division |
| `modulo` | `%` | remainder |
| — | `//` | floor division (floors toward −∞) |
| — | `**` | exponentiation |
| `equals` | `==` | equality |
| `is not` | `!=` | inequality |
| `is less than` | `<` | less |
| `is greater than` | `>` | greater |
| `is at most` | `<=` | ≤ (chainable: `1 <= x <= 10`) |
| `is at least` | `>=` | ≥ |
| `is between a and b` | — | inclusive range test |
| `is even` / `is odd` | — | parity |
| `is divisible by` | — | divisibility |
| `is approximately` | — | tolerant float equality |
| `and` / `or` / `not` | — | logic (short-circuit) |
| `xor` | `^` | bitwise / word XOR |
| `shifted left by` / `shifted right by` | — | bit shifts |
| `followed by` | — | sequence concatenation |
| `item N of xs` | `xs[N]` | 1-based access |
| `items A through B of xs` | — | inclusive slice |
| `x's field` | — | field access |
| `copy of x` | — | deep copy |
| `length of x` | — | length |
| `xs contains v` | — | membership |
| `a union b` / `a intersection b` | — | set algebra |
"#,
        hook: "",
        objectives: &[
            "identify both spellings of every operator",
            "choose the spelling that reads better in context, knowing they compile identically",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[],
        recap: &[],
    },
    Section {
        id: "appendix-errors",
        number: 42,
        title: "Appendix: The Error Message Gallery",
        part: "Part VIII: Reference",
        content: r#"
LOGOS errors teach — each states what happened, why it matters, and what to do. These
examples fail **on purpose** so you can read the real messages; the test suite locks
every displayed diagnostic word-for-word, so this gallery can never drift from the
compiler.

*Try it: run each one, read the message, then fix the program and watch it pass.*
"#,
        hook: "",
        objectives: &[
            "read a Socratic diagnostic and name the fix it suggests",
            "run each failing exhibit, fix it, and watch it pass",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "error-undefined-variable",
                label: "Undefined Variable",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "Undefined variable: total",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Main
Show total."#,
            },
            CodeExample {
                id: "error-type-mismatch",
                label: "Type Mismatch (the Socratic form)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "This slot is typed 'Int', but the value here is 'Text'. LOGOS holds every value to its declared type — that agreement is what the later guarantees stand on. Which side is right, the annotation or the value? Change the one that is lying.",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Main
Let x: Int be "hello"."#,
            },
            CodeExample {
                id: "error-division-by-zero",
                label: "Division by Zero",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "Division by zero",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Main
Show 1 / 0."#,
            },
            CodeExample {
                id: "error-nonexhaustive-match",
                label: "Non-Exhaustive Match",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "Inspect has no arm for `Green` (a `Light`) and no Otherwise (matches must be exhaustive) — add a `When Green (…):` arm, or an `Otherwise:` to cover it.",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Definition
A Light is either:
    Red.
    Green.

## Main
Let l be Green.
Inspect l:
    When Red:
        Show "stop"."#,
            },
        ],
        exercises: &[],
        recap: &[],
    },
    Section {
        id: "appendix-runnability",
        number: 43,
        title: "Appendix: Playground vs Compiled",
        part: "Part VIII: Reference",
        content: r#"
Every example in this guide carries a **truth badge**: it either runs right here in the
browser playground, or it says exactly why it cannot. The badge, the Run button, and the
test suite all read the same declaration — and the suite executes every example on every
change, in both directions: a playground example that stops running fails the build, and
a compiled-only example that *starts* running fails the build too, until it is promoted.

This table is the complete list of examples that need the compiled runtime. Everything
not listed here runs in the playground.

| Example | Section | Why it needs `largo build` |
|---------|---------|----------------------------|
| `crdt-persistent` | Distributed Types (CRDTs) | The journal runtime (`Mount`/`Persistent`) only exists in compiled Rust |
| `network-connect` | P2P Networking | `Connect` dials a live relay — a real network transport the browser doesn't have |
| `network-peer-agent` | P2P Networking | `PeerAgent` needs the compiled networking runtime |
| `network-distributed` | P2P Networking | `Mount` and the live relay both need the compiled runtime |
| `escape-to-rust` | Interoperability | Raw Rust blocks compile under `largo build`; the playground cannot interpret them |

To run one: copy the code into a `largo new` project and `largo run`.
"#,
        hook: "",
        objectives: &[
            "identify which examples need largo build and exactly why",
            "run a compiled-only example locally with largo new and largo run",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[],
        recap: &[],
    },
];
