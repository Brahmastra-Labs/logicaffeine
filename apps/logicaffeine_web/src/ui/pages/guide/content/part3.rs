//! Part III: Systems Programming — sections: memory-ownership, zones, concurrency.

use super::{
    CodeExample, ExampleMode, Exercise, ExerciseKind, Misconception, RecapQuestion, Runnability,
    Section,
};

pub(super) const PART_3: &[Section] = &[
    Section {
        id: "memory-ownership",
        number: 17,
        title: "Memory and Ownership",
        part: "Part III: Systems Programming",
        content: r#"
LOGOS provides memory safety through an ownership system expressed in natural English. Instead of cryptic symbols, you use verbs that describe what you're doing with data.

### The Three Verbs

| Verb | Meaning | What Happens |
|------|---------|--------------|
| `Give` | Transfer ownership | The original variable can no longer be used |
| `Show` | Temporary read access | The function can look but not modify |
| `Let modify` | Temporary write access | The function can change the data |

### Ownership Rules

1. **Single Owner:** Every value has exactly one owner at a time
2. **Move Semantics:** `Give` transfers ownership—you can't use it after
3. **Borrow Checking:** References (`Show`) can't outlive the owner
4. **Exclusive Mutation:** Only one `Let modify` at a time

### Common Patterns

- Copy first, then give
- Show multiple times (all OK — reads never conflict)
- Sequential mutation

### The `copy of` Expression

Use `copy of` to create a deep clone of a value. This lets you keep using the original while giving away the copy.

> **Mental model:** A value is a physical object. `Give` hands it over — your hand is
> empty afterwards, and the compiler says so if you forget. `Show x to f` holds the
> object up to the glass: f looks, nobody keeps. `copy of` runs the photocopier first.
> The analogy's limit: copies are honest duplicates, not originals — changing one
> never touches the other.
"#,
        hook: "Two functions both want the invoice: one to display it, one to file it away for good. If both held full control, whose changes would win? LOGOS makes the question unaskable — every value has exactly one owner, and the verbs say who.",
        objectives: &[
            "distinguish Give, Show-to, and copy of by what the caller keeps",
            "predict the error a use-after-Give raises",
            "choose the cheapest verb that still does the job",
        ],
        misconceptions: &[
            Misconception {
                id: "m-give-use",
                expectation: "Passing a variable to a function shares it — I can keep using it afterwards, like every garbage-collected language.",
                reality: "`Give` transfers ownership: the name has nothing left to hold, and the error names both fixes — lend with `Show x to f`, or hand over `a copy of x`. One owner at a time is why data races are unrepresentable.",
                evidence: "ownership-give-use",
            },
        ],
        examples: &[
            CodeExample {
                id: "ownership-show",
                label: "Show (Borrow)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Displaying: User Profile Data\nUser Profile Data"),
                predict_first: false,
                code: r#"## To display (data: Text):
    Show "Displaying: " + data.

## Main
Let profile be "User Profile Data".
Show profile to display.
Show profile."#,
            },
            CodeExample {
                id: "ownership-give",
                label: "Give (Move Ownership)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Consumed: Important data\nMessage was transferred"),
                predict_first: false,
                code: r#"## To consume (data: Text):
    Show "Consumed: " + data.

## Main
Let message be "Important data".
Give message to consume.
Show "Message was transferred"."#,
            },
            CodeExample {
                id: "ownership-give-use",
                label: "Use After Give (Fails on Purpose)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "Cannot use 'message' after giving it away. 'Give message to ...' transferred ownership — 'message' belongs to the receiver now, and this later use has nothing left to hold. Who really needs to own 'message' here? To lend it and keep it, 'Show message'; to hand over a duplicate, give 'a copy of message'.",
                },
                expected_output: None,
                predict_first: true,
                code: r#"## To consume (data: Text):
    Show "Consumed: " + data.

## Main
Let message be "secret".
Give message to consume.
Show message."#,
            },
            CodeExample {
                id: "ownership-copy",
                label: "Copy Before Giving",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Processing: Keep this\nOriginal still here: Keep this"),
                predict_first: false,
                code: r#"## To process (data: Text):
    Show "Processing: " + data.

## Main
Let original be "Keep this".
Let duplicate be copy of original.
Give duplicate to process.
Show "Original still here: " + original."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-own-hole",
                kind: ExerciseKind::Completion,
                concepts: &["Give (move)"],
                prompt: "Fill the hole with the ownership verb that hands the copy to the archive.",
                mode: ExampleMode::Imperative,
                expected: "Archived: Q3 numbers\nQ3 numbers",
                starter: "## To archive (data: Text):\n    Show \"Archived: \" + data.\n\n## Main\nLet report be \"Q3 numbers\".\nLet keeper be copy of report.\n??? keeper to archive.\nShow report.",
                hints: &[
                    "The verb that transfers ownership outright.",
                    "The copy departs; the original still prints.",
                ],
                reference: "## To archive (data: Text):\n    Show \"Archived: \" + data.\n\n## Main\nLet report be \"Q3 numbers\".\nLet keeper be copy of report.\nGive keeper to archive.\nShow report.",
            },
            Exercise {
                id: "ex-own-fix",
                kind: ExerciseKind::Practice,
                concepts: &["Show-to (borrow)"],
                prompt: "Broken on purpose: it Gives, then uses. Change ONE line so the function still sees the text and the last Show works — lend, don't give.",
                mode: ExampleMode::Imperative,
                expected: "Consumed: secret\nsecret",
                starter: "## To consume (data: Text):\n    Show \"Consumed: \" + data.\n\n## Main\nLet message be \"secret\".\nGive message to consume.\nShow message.",
                hints: &[
                    "Only the transfer line changes.",
                    "Lending reads `Show <name> to <function>`.",
                ],
                reference: "## To consume (data: Text):\n    Show \"Consumed: \" + data.\n\n## Main\nLet message be \"secret\".\nShow message to consume.\nShow message.",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-own-verbs",
                question: "Which verb transfers ownership, which lends read access, which duplicates?",
                answer: "`Give` transfers; `Show x to f` lends; `copy of` duplicates.",
            },
            RecapQuestion {
                id: "rq-own-after",
                question: "What can you do with a name after Give-ing it away?",
                answer: "Nothing — ownership moved. Lend with Show, or give a copy instead.",
            },
            RecapQuestion {
                id: "rq-own-races",
                question: "What does single ownership buy in concurrent code?",
                answer: "Data races become unrepresentable — no two writers can own one value.",
            },
        ],
    },
    Section {
        id: "zones",
        number: 18,
        title: "The Zone System",
        part: "Part III: Systems Programming",
        content: r#"
For high-performance scenarios, LOGOS provides **Zones**—memory regions where allocations are fast and cleanup is instant.

### Why Zones?

| Operation | Normal Heap | Zone |
|-----------|-------------|------|
| Allocate | O(log n) | O(1) |
| Deallocate individual | O(log n) | N/A |
| Free everything | O(n) | O(1) |

### The Hotel California Rule

**"What happens in the Zone, stays in the Zone."**

References to zone-allocated data cannot escape. To get data out of a zone, make an explicit copy.

### Zone Configuration

**Default size:** 4 KB (4096 bytes) when not specified.

**Specifying size:** Use `of size` with units:

| Unit | Example | Bytes |
|------|---------|-------|
| B | `of size 256 B` | 256 |
| KB | `of size 64 KB` | 65,536 |
| MB | `of size 2 MB` | 2,097,152 |
| GB | `of size 1 GB` | 1,073,741,824 |

### Zone Types

| Zone Type | Syntax | Access | Use Case |
|-----------|--------|--------|----------|
| Heap | `Inside a zone called "X":` | Read/Write | Temporary data |
| Heap (sized) | `Inside a zone called "X" of size 2 MB:` | Read/Write | Large temporary data |
| Mapped | `Inside a zone called "X" mapped from "file.bin":` | Read-only | Large file processing |

### When to Use Zones

Use zones when:
- Processing large amounts of temporary data
- Performance is critical (games, simulations)
- Memory allocation patterns are predictable
- You want instant cleanup

> **Mental model:** The zone is a whiteboard room: allocate by writing anywhere on the
> board, free by erasing the whole board on the way out — one wipe, no matter how much
> was written. What happens in the zone stays in the zone: to keep a result, copy it
> out before the wipe. The board is not magic memory — oversized data needs a bigger
> board, asked for with `of size`.
"#,
        hook: "A simulation allocates a million temporaries every frame; freeing them one at a time costs more than making them did. What if cleanup were one instruction — the whole room swept at once?",
        objectives: &[
            "write zone blocks, sized and default",
            "explain the Hotel California rule and how results leave a zone",
            "choose zones for burst allocation patterns",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "zone-basic",
                label: "Basic Zone (4KB default)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[1, 2, 3, 4, 5]\nZone freed!"),
                predict_first: false,
                code: r#"## Main
Inside a zone called "WorkSpace":
    Let temp_data be [1, 2, 3, 4, 5].
    Show temp_data.
Show "Zone freed!"."#,
            },
            CodeExample {
                id: "zone-sized-mb",
                label: "Zone with Size (MB)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[1, 2, 3, 4, 5]"),
                predict_first: false,
                code: r#"## Main
Inside a zone called "LargeBuffer" of size 2 MB:
    Let data be [1, 2, 3, 4, 5].
    Show data."#,
            },
            CodeExample {
                id: "zone-sized-kb",
                label: "Zone with Size (KB)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("142"),
                predict_first: false,
                code: r#"## Main
Inside a zone called "SmallArena" of size 64 KB:
    Let x be 42.
    Let y be 100.
    Show x + y."#,
            },
            CodeExample {
                id: "zone-mapped",
                label: "Memory-Mapped Zone",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Processing: config.bin\nProcessing: assets.bin"),
                predict_first: false,
                code: r#"## To process_file (path: Text):
    Inside a zone called "Data" mapped from path:
        Show "Processing: " + path.

## Main
process_file("config.bin").
process_file("assets.bin")."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-zone-hole",
                kind: ExerciseKind::Completion,
                concepts: &["sized zones"],
                prompt: "Fill the hole with the zone's size so the burst work runs in 64 KB.",
                mode: ExampleMode::Imperative,
                expected: "100",
                starter: "## Main\nLet mutable total be 0.\nInside a zone called \"Work\" of size ??? KB:\n    Repeat for n in [10, 20, 30, 40]:\n        Set total to total + n.\nShow total.",
                hints: &[
                    "Sizes read `of size <number> <unit>`.",
                    "Sixty-four kilobytes.",
                ],
                reference: "## Main\nLet mutable total be 0.\nInside a zone called \"Work\" of size 64 KB:\n    Repeat for n in [10, 20, 30, 40]:\n        Set total to total + n.\nShow total.",
            },
            Exercise {
                id: "ex-zone-sum",
                kind: ExerciseKind::Practice,
                concepts: &["zone blocks"],
                prompt: "Inside a 64 KB zone, sum [10, 20, 30, 40] into a total declared OUTSIDE the zone, and show it after the zone closes.",
                mode: ExampleMode::Imperative,
                expected: "100",
                starter: "## Main\nLet mutable total be 0.",
                hints: &[
                    "The zone line reads `Inside a zone called \"...\" of size 64 KB:`.",
                    "The total lives outside, so it survives the wipe.",
                ],
                reference: "## Main\nLet mutable total be 0.\nInside a zone called \"Work\" of size 64 KB:\n    Repeat for n in [10, 20, 30, 40]:\n        Set total to total + n.\nShow total.",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-zone-cost",
                question: "What does freeing a zone cost, regardless of how much it allocated?",
                answer: "One wipe — O(1). That is the whole point.",
            },
            RecapQuestion {
                id: "rq-zone-escape",
                question: "How does a result leave a zone?",
                answer: "By explicit copy to something outside — what happens in the zone stays in the zone.",
            },
            RecapQuestion {
                id: "rq-zone-when",
                question: "What kind of workload does a zone beat the normal heap on?",
                answer: "Burst allocations with one shared lifetime — per-frame simulation temporaries, parse scratch, batch processing.",
            },
        ],
    },
    Section {
        id: "machine-words",
        number: 19,
        title: "Bits, Bytes, and Machine Words",
        part: "Part III: Systems Programming",
        content: r#"
`Word8`, `Word16`, `Word32`, and `Word64` are fixed-width machine words: arithmetic on
them **wraps** (the ring ℤ/2ⁿ) instead of promoting. That is not a limitation — it is the
substrate hashing, checksums, and ciphers are built on. The LOGOS standard library's
SHA-3, ChaCha20, and ML-KEM are written in LOGOS itself on exactly these words.

### The word vocabulary

| Operation | Form |
|-----------|------|
| Construct | `word32(n)`, `word64(n)` |
| Back to Int | `intOfWord32(w)`, `intOfWord64(w)` |
| Bitwise | `w xor v`, `word_and(a, b)`, `word_or(a, b)`, `word_not(a)` |
| Rotate | `rotl(w, n)`, `rotr(w, n)` |
| Shift | `w shifted left by n`, `w shifted right by n` |

### Wrapping is the point

`word32(4294967295) + word32(1)` is exactly `0` — the carry falls off the 32nd bit. Name
the behavior and it becomes a tool: rotation, mixing, and diffusion in ciphers all depend
on it.

### Plain Int bit tools

`Int` has bit operations too: `shifted left by` / `shifted right by`, `xor`, and
`count_ones(n)` (the number of set bits).

> **Mental model:** A word is a car odometer with a fixed number of digits: 99999 + 1
> rolls to 00000 — the roll-over is the mechanism, not a malfunction. Rotation spins the
> digits around the dial without losing any. The odometer's limit: it counts patterns,
> not meaning — words are raw bit patterns; what they stand for is yours to assign.

*Try it: in the xorshift example, run twice — same seed, same output, every time. Change
the seed and watch the whole sequence change.*
"#,
        hook: "Add 1 to 4294967295 in a Word32 and you get 0 — and SHA-3, ChaCha20, and every checksum you trust are built on exactly that. The craft is knowing when wrap-around is the bug and when it is the tool.",
        objectives: &[
            "predict wrapping arithmetic on fixed-width words",
            "use xor, rotate, and shifts as cipher building blocks",
            "distinguish Word types from auto-promoting Int",
        ],
        misconceptions: &[
            Misconception {
                id: "m-word-wrap",
                expectation: "Integer overflow is a bug: a panic (Rust debug), undefined behavior (C), or impossible (Python).",
                reality: "On Word types the wrap IS the semantics — ℤ/2ⁿ ring arithmetic, the substrate ciphers and hashes are built on. Plain Int auto-promotes instead; wrapping happens exactly where you asked for it.",
                evidence: "word-wrapping",
            },
        ],
        examples: &[
            CodeExample {
                id: "word-wrapping",
                label: "Wrapping: The Carry Falls Off",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("0"),
                predict_first: true,
                code: r#"## Main
Show word32(4294967295) + word32(1)."#,
            },
            CodeExample {
                id: "word-bitwise",
                label: "XOR and Rotate",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5\n256"),
                predict_first: false,
                code: r#"## Main
Show word32(6) xor word32(3).
Show rotl(word32(1), 8)."#,
            },
            CodeExample {
                id: "bits-toolbox",
                label: "Int Bit Tools",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("16\n16\n8"),
                predict_first: false,
                code: r#"## Main
Show 1 shifted left by 4.
Show 256 shifted right by 4.
Show count_ones(255)."#,
            },
            CodeExample {
                id: "xorshift-prng",
                label: "A PRNG in Four Lines (xorshift)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("8748534153485358512"),
                predict_first: false,
                code: r#"## Main
Let mutable s be word64(88172645463325252).
Set s to s xor (s shifted left by 13).
Set s to s xor (s shifted right by 7).
Set s to s xor (s shifted left by 17).
Show intOfWord64(s)."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-word-hole",
                kind: ExerciseKind::Completion,
                concepts: &["xorshift PRNG"],
                prompt: "The classic xorshift shifts by 13, 7, 17 — fill the first hole.",
                mode: ExampleMode::Imperative,
                expected: "8748534153485358512",
                starter: "## Main\nLet mutable s be word64(88172645463325252).\nSet s to s xor (s shifted left by ???).\nSet s to s xor (s shifted right by 7).\nSet s to s xor (s shifted left by 17).\nShow intOfWord64(s).",
                hints: &[
                    "Three constants make the classic triple: 13, 7, 17.",
                    "The first shift goes left by the first constant.",
                ],
                reference: "## Main\nLet mutable s be word64(88172645463325252).\nSet s to s xor (s shifted left by 13).\nSet s to s xor (s shifted right by 7).\nSet s to s xor (s shifted left by 17).\nShow intOfWord64(s).",
            },
            Exercise {
                id: "ex-word-rotl",
                kind: ExerciseKind::Practice,
                concepts: &["word bitwise + rotate"],
                prompt: "Rotate word32(1) left by 4 and print it; then print intOfWord64(word64(255)).",
                mode: ExampleMode::Imperative,
                expected: "16\n255",
                starter: "## Main\n",
                hints: &[
                    "`rotl(w, n)` spins bits left without losing any.",
                    "The round-trip through word64 and back is lossless for small values.",
                ],
                reference: "## Main\nShow rotl(word32(1), 4).\nShow intOfWord64(word64(255)).",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-word-wrap",
                question: "What does `word32(4294967295) + word32(1)` print, and why is that not a bug?",
                answer: "0 — the carry falls off the 32nd bit by design; wrapping is the ring arithmetic ciphers are built on.",
            },
            RecapQuestion {
                id: "rq-word-vs-int",
                question: "Same sum on plain Int — what happens instead of wrapping?",
                answer: "Automatic BigInt promotion; Int never wraps silently.",
            },
            RecapQuestion {
                id: "rq-word-crypto",
                question: "Which three word operations does xorshift combine?",
                answer: "xor, shift left, shift right — mixing until the bits look random.",
            },
        ],
    },
    Section {
        id: "concurrency",
        number: 20,
        title: "Concurrency",
        part: "Part III: Systems Programming",
        content: r#"
LOGOS provides safe concurrency through structured patterns. No data races, no deadlocks.

### Concurrent Patterns Overview

| Pattern | Syntax | Use For | Compiles To |
|---------|--------|---------|-------------|
| **Async Join** | `Attempt all of the following:` | Wait for all I/O tasks | tokio::join! |
| **Parallel CPU** | `Simultaneously:` | CPU-bound computation | rayon::join / threads |
| **Spawn Task** | `Launch a task to...` | Fire-and-forget work | tokio::spawn |
| **Channels** | `Pipe of Type` | Message passing | tokio::mpsc |
| **Select** | `Await the first of:` | Race operations | tokio::select! |

### Attempt All (Async I/O)

Use `Attempt all of the following:` for I/O operations that wait on external resources. All operations run concurrently, and the program waits until all complete.

Variables declared in concurrent blocks are captured and returned as a tuple.

### Simultaneously (Parallel CPU)

Use `Simultaneously:` for CPU-intensive work. Computations run in parallel on different CPU cores.

- 2 tasks → uses `rayon::join` (work-stealing thread pool)
- 3+ tasks → uses `std::thread::spawn` (dedicated threads)

### Tasks (Green Threads)

Use `Launch a task to...` to spawn a green thread that runs concurrently. For fire-and-forget work, launch and move on:

`Launch a task to process(data).`

To control the task later (cancel, await), capture a handle:

`Let worker be Launch a task to process(data).`

Stop a running task with:

`Stop worker.`

### Channels (Pipes)

Pipes are Go-style channels for message passing between tasks.

**Create a channel:**

`Let jobs be a new Pipe of Int.`

**Send into a channel (blocking):**

`Send value into jobs.`

**Receive from a channel (blocking):**

`Receive item from jobs.`

**Non-blocking variants:**

`Try to send value into jobs.`

`Try to receive item from jobs.`

### Select (Racing Operations)

Use `Await the first of:` to race multiple operations. The first one to complete wins:

```logos
Await the first of:
    Receive msg from inbox:
        Show msg.
    After 5 seconds:
        Show "timeout".
```

**Branch types:**
- `Receive var from pipe:` — wait for channel message
- `After N seconds:` — timeout branch

### Ownership and Concurrency

The ownership system prevents data races. Multiple reads are OK, but concurrent writes are prevented.

**Note:** Everything above — tasks, pipes, and select — runs right here in the browser playground on the deterministic scheduler. Compiled programs run the same code on the real multi-threaded runtime.

### Deterministic replay

A concurrent LOGOS run is a pure function of the program and a seed: the scheduler's every
choice derives from `LOGOS_SEED`, so a race you saw once is a race you can replay forever —
same seed, same interleaving, byte-identical output. Concurrency bugs stop being
heisenbugs.

> **Mental model:** Tasks are relay runners; pipes are the baton-passing lanes between
> them. The scheduler is a referee working from a script: every "who runs next" choice
> is written by the seed, so re-running the race replays every hand-off exactly. The
> script covers scheduling only — a wrong program replays its wrongness deterministically.
"#,
        hook: "Three workers pull jobs from one queue. In most languages the output depends on which thread won which race — today. A LOGOS run is a pure function of the program and a seed: the race you saw is the race you can replay.",
        objectives: &[
            "write tasks, pipes, and select blocks",
            "predict pipe buffering behavior before running it",
            "explain why concurrent runs replay deterministically",
        ],
        misconceptions: &[
            Misconception {
                id: "m-pipe-buffer",
                expectation: "Send blocks until someone receives — an unbuffered Go channel would deadlock right here.",
                reality: "Pipes buffer: a task can send several values and receive them later itself. Blocking bites at RECEIVE on an empty pipe — and `Try to receive` is the non-blocking probe.",
                evidence: "pipe-send-receive",
            },
        ],
        examples: &[
            CodeExample {
                id: "concurrent-async",
                label: "Async Concurrent (Attempt All)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("a = 10\nb = 20\nSum: 30"),
                predict_first: false,
                code: r#"## Main
Attempt all of the following:
    Let a be 10.
    Let b be 20.
Show "a = " + a.
Show "b = " + b.
Show "Sum: " + (a + b)."#,
            },
            CodeExample {
                id: "parallel-cpu",
                label: "Parallel CPU (Simultaneously)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("x = 100\ny = 200\nProduct: 20000"),
                predict_first: false,
                code: r#"## Main
Simultaneously:
    Let x be 100.
    Let y be 200.
Show "x = " + x.
Show "y = " + y.
Show "Product: " + (x * y)."#,
            },
            CodeExample {
                id: "parallel-three-tasks",
                label: "Three Parallel Tasks",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Sum: 6"),
                predict_first: false,
                code: r#"## Main
Simultaneously:
    Let a be 1.
    Let b be 2.
    Let c be 3.
Show "Sum: " + (a + b + c)."#,
            },
            CodeExample {
                id: "concurrent-in-function",
                label: "Concurrency in Functions",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Result: 15"),
                predict_first: false,
                code: r#"## To compute_parallel -> Int:
    Simultaneously:
        Let x be 5.
        Let y be 10.
    Return x + y.

## Main
Let result be compute_parallel().
Show "Result: " + result."#,
            },
            CodeExample {
                id: "launch-task",
                label: "Launch Task",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Worker 1 started\nWorker 2 started\nTasks launched"),
                predict_first: false,
                code: r#"## To worker (id: Int):
    Show "Worker " + id + " started".

## Main
Launch a task to worker(1).
Launch a task to worker(2).
Show "Tasks launched"."#,
            },
            CodeExample {
                id: "task-with-handle",
                label: "Task with Handle",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Working...\nTask spawned\nTask cancelled"),
                predict_first: false,
                code: r#"## To long_running:
    Show "Working...".

## Main
Let job be Launch a task to long_running.
Show "Task spawned".
Stop job.
Show "Task cancelled"."#,
            },
            CodeExample {
                id: "pipe-send-receive",
                label: "Pipe Communication",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Got: 42"),
                predict_first: true,
                code: r#"## Main
Let messages be a new Pipe of Int.
Send 42 into messages.
Send 100 into messages.
Receive x from messages.
Show "Got: " + x."#,
            },
            CodeExample {
                id: "select-timeout",
                label: "Select with Timeout",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("No message received"),
                predict_first: false,
                code: r#"## Main
Let inbox be a new Pipe of Text.

Await the first of:
    Receive msg from inbox:
        Show "Message: " + msg.
    After 2 seconds:
        Show "No message received"."#,
            },
            CodeExample {
                id: "try-send-receive",
                label: "Non-Blocking Send and Receive",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5"),
                predict_first: false,
                code: r#"## Main
Let p be a new Pipe of Int.
Try to send 5 into p.
Try to receive x from p.
Show x."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-conc-hole",
                kind: ExerciseKind::Completion,
                concepts: &["Send/Receive"],
                prompt: "Fill the hole so all three values reach the pipe and the sum prints 60.",
                mode: ExampleMode::Imperative,
                expected: "sum: 60",
                starter: "## Main\nLet jobs be a new Pipe of Int.\nSend 10 into jobs.\nSend 20 into jobs.\nSend ??? into jobs.\nLet mutable total be 0.\nReceive a from jobs.\nReceive b from jobs.\nReceive c from jobs.\nSet total to a + b + c.\nShow \"sum: {total}\".",
                hints: &[
                    "Three receives need three sends.",
                    "10 + 20 + what makes 60?",
                ],
                reference: "## Main\nLet jobs be a new Pipe of Int.\nSend 10 into jobs.\nSend 20 into jobs.\nSend 30 into jobs.\nLet mutable total be 0.\nReceive a from jobs.\nReceive b from jobs.\nReceive c from jobs.\nSet total to a + b + c.\nShow \"sum: {total}\".",
            },
            Exercise {
                id: "ex-conc-fanin",
                kind: ExerciseKind::Practice,
                concepts: &["pipes (channels)", "Send/Receive"],
                prompt: "Send 10, 20, 30 into a pipe, receive all three, and print `sum: 60`. Your answer is gradable because runs are reproducible.",
                mode: ExampleMode::Imperative,
                expected: "sum: 60",
                starter: "## Main\nLet jobs be a new Pipe of Int.",
                hints: &[
                    "Three Sends buffer; three Receives drain in order.",
                    "Receive into three names, then one interpolated Show.",
                ],
                reference: "## Main\nLet jobs be a new Pipe of Int.\nSend 10 into jobs.\nSend 20 into jobs.\nSend 30 into jobs.\nLet mutable total be 0.\nReceive a from jobs.\nReceive b from jobs.\nReceive c from jobs.\nSet total to a + b + c.\nShow \"sum: {total}\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-conc-replay",
                question: "What two things fully determine a concurrent LOGOS run?",
                answer: "The program and the seed — every scheduler choice derives from LOGOS_SEED, so runs replay byte-identically.",
            },
            RecapQuestion {
                id: "rq-conc-buffer",
                question: "Which end of a pipe can block — Send or Receive?",
                answer: "Receive, on an empty pipe. Sends buffer; `Try to` variants never block.",
            },
            RecapQuestion {
                id: "rq-conc-pick",
                question: "Attempt all vs Simultaneously — which for I/O waits, which for CPU work?",
                answer: "Attempt all for I/O (async join); Simultaneously for CPU parallelism.",
            },
        ],
    },
    Section {
        id: "interop",
        number: 21,
        title: "Interoperability: Rust, Native, and WASM",
        part: "Part III: Systems Programming",
        content: r#"
When you need to reach outside the language, three doors open — all in compiled programs.

### Escape to Rust

An `Escape to Rust:` block embeds raw Rust in place; `largo build` compiles it into the
binary as-is. The playground shows the honest boundary: it cannot interpret raw Rust, and
says so.

### Native functions

`## To native <name> (…) -> <Type>` declares a function whose body is provided by the
host — the mechanism behind the standard library's `now`, `read`, and `randomInt`.

### Exporting to WASM

Mark a function `is exported for wasm` and `largo build --emit wasm` produces a module
whose exports call straight into your LOGOS code — no JavaScript shims, no rustc in the
loop. The decorator parses and runs everywhere — it *matters* at `largo build --emit wasm`.
"#,
        hook: "Sooner or later you need something LOGOS doesn't ship: a vendor SDK, a syscall, a hand-tuned kernel. Three doors open outward — embed Rust in place, bind a native host function, or export your LOGOS to WASM callers.",
        objectives: &[
            "name the three interop doors and what each is for",
            "distinguish what runs in the playground from what needs largo build",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "escape-to-rust",
                label: "Escape to Rust",
                mode: ExampleMode::Imperative,
                runnability: Runnability::CompiledOnly {
                    reason: "Escape blocks contain raw Rust — largo build compiles them; the playground cannot interpret them",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Main
Escape to Rust:
    println!("from rust");
Show "back in LOGOS"."#,
            },
            CodeExample {
                id: "interop-wasm-export",
                label: "Exporting to WASM (Decorator Runs Everywhere)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("36"),
                predict_first: false,
                code: r#"## To square (n: Int) -> Int is exported for wasm:
    Return n * n.

## Main
Show square(6)."#,
            },
        ],
        exercises: &[],
        recap: &[
            RecapQuestion {
                id: "rq-interop-doors",
                question: "What are the three interop doors?",
                answer: "`Escape to Rust:` blocks, `## To native` host bindings, and `is exported for wasm` exports.",
            },
            RecapQuestion {
                id: "rq-interop-playground",
                question: "Why can't the playground run an Escape block?",
                answer: "The block is raw Rust — largo build compiles it; the playground interprets LOGOS only, and its badge says so.",
            },
            RecapQuestion {
                id: "rq-interop-wasm",
                question: "Where does `is exported for wasm` change behavior — the playground or the build?",
                answer: "The build: `largo build --emit wasm` exports the function; in the playground the decorator parses and the function runs normally.",
            },
        ],
    },
    Section {
        id: "part-3-checkpoint",
        number: 22,
        title: "Checkpoint: Systems Programming",
        part: "Part III: Systems Programming",
        content: r#"
Ownership, zones, machine words, concurrency — the systems toolkit. Three challenges
below blend them, and one reaches back to Part II on purpose. The recall cards then
collect on the part's promises.
"#,
        hook: "",
        objectives: &[
            "combine ownership verbs, zones, and pipes in working programs",
            "diagnose which Part III section to reread when a challenge stumps you",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[
            Exercise {
                id: "cp3-word-mask",
                kind: ExerciseKind::Challenge,
                concepts: &["word bitwise + rotate", "English arithmetic"],
                prompt: "Print word32(6) xor word32(3), then the remainder of 10 divided by 3.",
                mode: ExampleMode::Imperative,
                expected: "5\n1",
                starter: "## Main\n",
                hints: &[
                    "Two Shows: a word xor, then a modulo.",
                    "6 xor 3 flips exactly the bits where they differ.",
                ],
                reference: "## Main\nShow word32(6) xor word32(3).\nShow 10 % 3.",
            },
            Exercise {
                id: "cp3-own-zone",
                kind: ExerciseKind::Challenge,
                concepts: &["Give (move)", "zone blocks"],
                prompt: "Build \"Q3\" + \" report\" inside a zone, copy it out to a variable declared outside, Give that to an archive function, and print `done`.",
                mode: ExampleMode::Imperative,
                expected: "Archived: Q3 report\ndone",
                starter: "## To archive (data: Text):\n    Show \"Archived: \" + data.\n\n## Main\nLet mutable keep be \"\".",
                hints: &[
                    "Results leave a zone by copy — set the outside variable to `copy of` the draft.",
                    "After the zone closes, the transfer verb hands `keep` to the archive; then the final Show.",
                ],
                reference: "## To archive (data: Text):\n    Show \"Archived: \" + data.\n\n## Main\nLet mutable keep be \"\".\nInside a zone called \"Scratch\":\n    Let draft be \"Q3\" + \" report\".\n    Set keep to copy of draft.\nGive keep to archive.\nShow \"done\".",
            },
            Exercise {
                id: "cp3-pipe-total",
                kind: ExerciseKind::Challenge,
                concepts: &["pipes (channels)", "string interpolation"],
                prompt: "Send 1 and 2 into a pipe, receive both, and print `total: 3` with interpolation.",
                mode: ExampleMode::Imperative,
                expected: "total: 3",
                starter: "## Main\n",
                hints: &[
                    "Sends buffer; receive into two names.",
                    "Braces can hold the sum: `{a + b}`.",
                ],
                reference: "## Main\nLet results be a new Pipe of Int.\nSend 1 into results.\nSend 2 into results.\nReceive a from results.\nReceive b from results.\nShow \"total: {a + b}\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "cq3-verbs",
                question: "Give, Show-to, copy of — which keeps the original usable?",
                answer: "Show-to (lends) and copy of (duplicates). Give empties the name.",
            },
            RecapQuestion {
                id: "cq3-zone",
                question: "What is O(1) about a zone?",
                answer: "Freeing — the whole zone is wiped at once, however much it allocated.",
            },
            RecapQuestion {
                id: "cq3-wrap",
                question: "Which type wraps on overflow — Int or Word32 — and what does the other do?",
                answer: "Word32 wraps (by design); Int promotes to BigInt.",
            },
            RecapQuestion {
                id: "cq3-seed",
                question: "Why is a LOGOS data race replayable?",
                answer: "The scheduler's every choice derives from LOGOS_SEED — same seed, same interleaving.",
            },
            RecapQuestion {
                id: "cq3-blockend",
                question: "Which pipe operation can block, and what is its non-blocking twin?",
                answer: "Receive on an empty pipe; `Try to receive` never blocks.",
            },
            RecapQuestion {
                id: "cq3-doors",
                question: "Which interop door embeds raw Rust in place?",
                answer: "`Escape to Rust:` — compiled by largo build, honestly refused by the playground.",
            },
        ],
    },
];
