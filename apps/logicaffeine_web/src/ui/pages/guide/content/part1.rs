//! Part I: First Steps — sections: introduction, reading-english, getting-started, tutorial-project.

use super::{
    CodeExample, ExampleMode, Exercise, ExerciseKind, Misconception, RecapQuestion, Runnability,
    Section,
};

pub(super) const PART_1: &[Section] = &[
    Section {
        id: "introduction",
        number: 1,
        title: "Introduction",
        part: "Part I: First Steps",
        content: r#"
### What is LOGOS?

LOGOS is a programming language where you write code in natural English. Instead of cryptic symbols and arcane syntax, you express your ideas in sentences that read like plain prose—and those sentences compile into efficient, executable programs.

LOGOS has two modes:

| Mode | What It Does | Output |
|------|--------------|--------|
| **Imperative Mode** | Write executable programs | Rust code (compiled to native binaries) |
| **Logic Mode** | Translate English to formal logic | First-Order Logic notation |

This guide focuses primarily on **Imperative Mode**—using LOGOS as a programming language. Part VI covers Logic Mode and verification for those interested in formal semantics.

### The Vision

The name LOGOS comes from the Greek λόγος, meaning "word," "reason," and "principle." In LOGOS, these concepts unify:

- **Words** become executable code
- **Reason** becomes verifiable logic
- **Principles** become formal proofs

When you write LOGOS, you're not writing comments that describe code—you're writing sentences that *are* the code.

### How to Read This Guide

**If you're new to programming:**
- Read each section in order
- Try every example yourself
- Don't skip ahead—each concept builds on the previous

**If you're an experienced programmer:**
- Use the Table of Contents to jump to what interests you
- The Quick Reference section provides rapid lookup
- The Complete Examples show real-world patterns
"#,
        hook: "",
        objectives: &[
            "decide which mode — Imperative or Logic — fits what you want to build",
            "choose your reading path: front-to-back as a beginner, jump-around as a veteran",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[],
        recap: &[],
    },
    Section {
        id: "reading-english",
        number: 2,
        title: "How LOGOS Reads English",
        part: "Part I: First Steps",
        content: r#"
### The phrasing you would guess

LOGOS is a deterministic grammar, not an AI guessing your intent. Every sentence the
parser accepts has exactly one meaning, and the design rule is least surprise: the
phrasing you would naturally write is the phrasing that works. Where several natural
phrasings exist, they are synonyms for the same operation — `2 + 3` and `2 plus 3`
compile to the same addition, and neither is more "real" than the other.

### English first, symbols beside

Every operator has an English form and a symbolic twin: `plus` and `+`, `is at least`
and `>=`, `equals` and `==`. Use whichever reads better in context; they are the same
program. The full table lives in Operators and Expressions.

### The deliberate exceptions

A few places where LOGOS chooses the human reading over programmer tradition — learn
these and the rest of the language follows your intuition:

1. **Counting starts at 1.** `item 1 of items` is the first element, and slices are
   inclusive on both ends: `items 1 through 2` is two elements.
2. **Equality has a canonical spelling: `equals`** (or `==`). Bare `is` also
   compares — `x is 5` runs and is true — but `is` wears many hats in LOGOS
   (descriptions, definitions, predicates like `is even`), so this guide writes
   `equals` wherever equality is meant: the dedicated spelling keeps intent
   unambiguous.
3. **Fields read as possessives.** A struct's field is `alice's age` — the apostrophe
   is grammar, not decoration.

The examples below preview indexing, comparisons, and `If` — each is taught fully in
Part II; here they carry the mental model only.

*Try it: in the first example below, change `plus` to `times` — then make the symbolic
line disagree and watch the last line turn false.*
"#,
        hook: "A Python programmer reads `item 1 of scores` and expects the second element. A C programmer reads `items 1 through 2` and expects one element. Both are wrong — and the reasons will make the rest of LOGOS feel obvious.",
        objectives: &[
            "predict what `item 1 of xs` and `items 1 through 2 of xs` return",
            "translate any symbolic operator to its English twin and back",
            "choose the equality spelling that keeps intent unambiguous",
            "spot the three deliberate exceptions to programmer tradition",
        ],
        misconceptions: &[
            Misconception {
                id: "m-zero-based",
                expectation: "In Python or C, `scores[1]` is the SECOND element — counting starts at zero, and slices exclude their end.",
                reality: "LOGOS counts the way humans do: `item 1` is the first element, and slices include both ends — `items 1 through 2` is exactly two items. One convention, applied everywhere, so there are no off-by-one gymnastics to rehearse.",
                evidence: "reading-one-based",
            },
        ],
        examples: &[
            CodeExample {
                id: "reading-twins",
                label: "English and Symbols Are Twins",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5\n5\ntrue"),
                predict_first: false,
                code: r#"## Main
Let total be 2 + 3.
Let same be 2 plus 3.
Show total.
Show same.
Show total equals same."#,
            },
            CodeExample {
                id: "reading-guess",
                label: "The Phrasing You Would Guess",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("First score is an A\nSame check, symbol style"),
                predict_first: false,
                code: r#"## Main
Let scores be [90, 75, 60].
If item 1 of scores is at least 80:
    Show "First score is an A".
If scores[1] >= 80:
    Show "Same check, symbol style"."#,
            },
            CodeExample {
                id: "reading-one-based",
                label: "Counting Starts at 1 (Inclusive)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("a\n[a, b]\n4"),
                predict_first: true,
                code: r#"## Main
Let letters be ["a", "b", "c", "d"].
Show item 1 of letters.
Show items 1 through 2 of letters.
Show length of letters."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-reading-twins",
                kind: ExerciseKind::Completion,
                concepts: &["equals-not-is rule"],
                prompt: "Fill the hole so the program prints `true` — use the canonical equality spelling.",
                mode: ExampleMode::Imperative,
                expected: "true",
                starter: "## Main\nLet total be 2 plus 3.\nShow total ??? 5.",
                hints: &[
                    "The comparison the guide prefers is spelled as one English word.",
                    "Its symbolic twin is `==` — either passes, but write the word.",
                ],
                reference: "## Main\nLet total be 2 plus 3.\nShow total equals 5.",
            },
            Exercise {
                id: "ex-reading-third",
                kind: ExerciseKind::Practice,
                concepts: &["1-based inclusive indexing exception"],
                prompt: "Print the third letter twice: once with `item N of`, once with brackets.",
                mode: ExampleMode::Imperative,
                expected: "c\nc",
                starter: "## Main\nLet letters be [\"a\", \"b\", \"c\", \"d\"].",
                hints: &[
                    "Counting starts at 1 — the third letter sits at position 3.",
                    "The bracket twin of `item 3 of letters` is `letters[3]`.",
                ],
                reference: "## Main\nLet letters be [\"a\", \"b\", \"c\", \"d\"].\nShow item 3 of letters.\nShow letters[3].",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-reading-index",
                question: "What does `item 1 of xs` return — and is `items 2 through 3 of xs` one item or two?",
                answer: "The first element (counting starts at 1); two items — slices include both ends.",
            },
            RecapQuestion {
                id: "rq-reading-is",
                question: "Which spelling does the guide prefer for equality, and why?",
                answer: "`equals` (or `==`). Bare `is` also compares, but `is` carries many other roles — the dedicated spelling keeps intent unambiguous.",
            },
            RecapQuestion {
                id: "rq-reading-twins",
                question: "Name the two spellings of the ≥ comparison — which one is more \"real\"?",
                answer: "`is at least` and `>=`. Neither — they compile to the same program.",
            },
        ],
    },
    Section {
        id: "getting-started",
        number: 3,
        title: "Getting Started",
        part: "Part I: First Steps",
        content: r#"
### Installing largo

Everything on this site runs in your browser — but for real projects you want `largo`,
the LOGOS build tool, on your machine. One line, no toolchain required:

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

On Windows:

```powershell
irm https://logicaffeine.com/install.ps1 | iex
```

The installer fetches the prebuilt binary for your platform (Linux/macOS x64 + arm64,
Windows x64), verifies its SHA-256 checksum, and installs it to `~/.local/bin` — no sudo,
no shell-config edits. Add `--full` (`sh -s -- --full`) for the build with Z3 verification
bundled. Then:

```bash
largo new hello && cd hello && largo run
```

### Hello World

Every programming journey begins with Hello World. In LOGOS:

The Program Structure example below is a deliberate preview — a struct and a function
before either is taught. Every piece gets its full section in Part II; here it shows
the shape of a real program.
"#,
        hook: "",
        objectives: &[
            "run your first LOGOS program in the playground",
            "write a `## Main` block that prints text",
            "name the one-line installer for your platform",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "hello-world",
                label: "Hello World",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Hello, World!"),
                predict_first: false,
                code: r#"## Main
Show "Hello, World!"."#,
            },
            CodeExample {
                id: "program-structure",
                label: "Program Structure",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Hello, World!"),
                predict_first: false,
                code: r#"## Definition
A Point has:
    an x: Int.
    a y: Int.

## To greet (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main
Let p be a new Point with x 10 and y 20.
Let message be greet("World").
Show message."#,
            },
            CodeExample {
                id: "first-program",
                label: "Your First Real Program",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Name: Alice\nAge: 25"),
                predict_first: false,
                code: r#"## Main
Let name be "Alice".
Let age be 25.
Show "Name: " + name.
Show "Age: " + age."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-greet-logos",
                kind: ExerciseKind::Practice,
                concepts: &["hello world"],
                prompt: "Change the greeting: make the program print exactly `Hello, LOGOS!`.",
                mode: ExampleMode::Imperative,
                expected: "Hello, LOGOS!",
                starter: "## Main\nShow \"Hello, World!\".",
                hints: &[
                    "Only the text inside the quotes needs to change.",
                    "The target greeting names the language, not the world.",
                ],
                reference: "## Main\nShow \"Hello, LOGOS!\".",
            },
        ],
        recap: &[],
    },
    Section {
        id: "tutorial-project",
        number: 4,
        title: "Tutorial: Teach the Computer to Guess",
        part: "Part I: First Steps",
        content: r#"
One small program, built in five stages — each stage runs, and each adds exactly one idea.
The game: you think of a number between 1 and 100; the *program* finds it by binary
search, and it never needs more than 7 tries.

**Stage 1** sets the scene with variables and `Show`. **Stage 2** makes one guess: the
midpoint, and an `If` to say which way it missed. **Stage 3** turns that into the whole
game with `While` and `Break`, halving the range each round. **Stage 4** extracts a
function so any secret can be found. **Stage 5** packages the result in a struct and
announces it with interpolation.

The secret is written into the program so every stage runs right here — change it at any
stage and watch the search adapt. Every construct the game uses — `While`, `Break`,
functions, structs, interpolation — gets its full treatment in Part II; the tutorial
shows them working together first.

*Try it: in Stage 4, try secrets 1 and 100 — the edges. Does it ever need more than 7?*

> **Look back:** the halving move generalizes to any sorted search — dictionaries,
> version bisecting, guess-the-bug. What breaks if the range starts at 0? Nothing —
> but if the secret is OUTSIDE the range, the loop never ends: bounds are the
> contract. Try counting comparisons instead of tries.
"#,
        hook: "You think of a number between 1 and 100; the program finds it in at most 7 guesses — every time, guaranteed. By stage five you will have written the whole thing: the loop that narrows the range, the function that plays any secret, and the struct that reports the win.",
        objectives: &[
            "build a complete program in five runnable stages",
            "trace how halving the range guarantees at most 7 guesses",
            "extend a working program without breaking its earlier stages",
        ],
        misconceptions: &[
            Misconception {
                id: "m-floor-div",
                expectation: "The midpoint of 1 and 100 is 50.5, so the program's first guess should print 50.5.",
                reality: "The guess uses `//` — floor division. `(1 + 100) // 2` is exactly 50: integer arithmetic stays in the integers unless you ask otherwise, which is precisely what a guessing game needs.",
                evidence: "tutorial-stage-2",
            },
        ],
        examples: &[
            CodeExample {
                id: "tutorial-stage-1",
                label: "Stage 1: The Scene",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Think of a number. Mine is between 1 and 100."),
                predict_first: false,
                code: r#"## Main
Let secret be 37.
Let low be 1.
Let high be 100.
Show "Think of a number. Mine is between {low} and {high}."."#,
            },
            CodeExample {
                id: "tutorial-stage-2",
                label: "Stage 2: One Guess",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("I guess 50.\nToo high!"),
                predict_first: true,
                code: r#"## Main
Let secret be 37.
Let guess be (1 + 100) // 2.
Show "I guess {guess}.".
If guess is greater than secret:
    Show "Too high!".
If guess is less than secret:
    Show "Too low!"."#,
            },
            CodeExample {
                id: "tutorial-stage-3",
                label: "Stage 3: The Whole Game",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Got 37 in 3 tries."),
                predict_first: false,
                code: r#"## Main
Let secret be 37.
Let mutable low be 1.
Let mutable high be 100.
Let mutable tries be 0.
While true:
    Let guess be (low + high) // 2.
    Set tries to tries + 1.
    If guess equals secret:
        Break.
    If guess is greater than secret:
        Set high to guess - 1.
    Otherwise:
        Set low to guess + 1.
Show "Got {secret} in {tries} tries."."#,
            },
            CodeExample {
                id: "tutorial-stage-4",
                label: "Stage 4: Extract a Function",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("37 found in 3 tries.\n99 found in 6 tries."),
                predict_first: false,
                code: r#"## To find (secret: Int) -> Int:
    Let mutable low be 1.
    Let mutable high be 100.
    Let mutable tries be 0.
    While true:
        Let guess be (low + high) // 2.
        Set tries to tries + 1.
        If guess equals secret:
            Return tries.
        If guess is greater than secret:
            Set high to guess - 1.
        Otherwise:
            Set low to guess + 1.
    Return tries.

## Main
Show "37 found in {find(37)} tries.".
Show "99 found in {find(99)} tries."."#,
            },
            CodeExample {
                id: "tutorial-stage-5",
                label: "Stage 5: A Struct Finale",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Found 37 in 3 tries — never more than 7."),
                predict_first: false,
                code: r#"## Definition
A Game has:
    a secret: Int.
    a tries: Int.

## To play (secret: Int) -> Game:
    Let mutable low be 1.
    Let mutable high be 100.
    Let mutable tries be 0.
    While true:
        Let guess be (low + high) // 2.
        Set tries to tries + 1.
        If guess equals secret:
            Break.
        If guess is greater than secret:
            Set high to guess - 1.
        Otherwise:
            Set low to guess + 1.
    Return a new Game with secret secret and tries tries.

## Main
Let g be play(37).
Show "Found {g's secret} in {g's tries} tries — never more than 7."."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-tutorial-holes",
                kind: ExerciseKind::Completion,
                concepts: &["staged build-one-program tutorial"],
                prompt: "Stage 3 with two holes: count each try, and move the bottom of the range after a low guess.",
                mode: ExampleMode::Imperative,
                expected: "Got 37 in 3 tries.",
                starter: "## Main\nLet secret be 37.\nLet mutable low be 1.\nLet mutable high be 100.\nLet mutable tries be 0.\nWhile true:\n    Let guess be (low + high) // 2.\n    Set tries to ???.\n    If guess equals secret:\n        Break.\n    If guess is greater than secret:\n        Set high to guess - 1.\n    Otherwise:\n        Set low to ???.\nShow \"Got {secret} in {tries} tries.\".",
                hints: &[
                    "Each pass through the loop is one more try.",
                    "When the guess is too low, the answer must be above it — move the bottom of the range past the guess.",
                ],
                reference: "## Main\nLet secret be 37.\nLet mutable low be 1.\nLet mutable high be 100.\nLet mutable tries be 0.\nWhile true:\n    Let guess be (low + high) // 2.\n    Set tries to tries + 1.\n    If guess equals secret:\n        Break.\n    If guess is greater than secret:\n        Set high to guess - 1.\n    Otherwise:\n        Set low to guess + 1.\nShow \"Got {secret} in {tries} tries.\".",
            },
            Exercise {
                id: "ex-tutorial-thousand",
                kind: ExerciseKind::Practice,
                concepts: &["staged build-one-program tutorial"],
                prompt: "Widen the search to 1 through 1000 and report the hunt for 999 in the stage-4 style. Predict first: can it still finish in 10 tries?",
                mode: ExampleMode::Imperative,
                expected: "999 found in 9 tries.",
                starter: "## To find (secret: Int) -> Int:\n    Let mutable low be 1.\n    Let mutable high be 100.\n    Let mutable tries be 0.\n    While true:\n        Let guess be (low + high) // 2.\n        Set tries to tries + 1.\n        If guess equals secret:\n            Return tries.\n        If guess is greater than secret:\n            Set high to guess - 1.\n        Otherwise:\n            Set low to guess + 1.\n    Return tries.\n\n## Main\nShow \"999 found in {find(999)} tries.\".",
                hints: &[
                    "Only the top of the range changes — the algorithm doesn't.",
                    "Ten halvings cover 1024 numbers, so 1000 fits inside 10 tries.",
                ],
                reference: "## To find (secret: Int) -> Int:\n    Let mutable low be 1.\n    Let mutable high be 1000.\n    Let mutable tries be 0.\n    While true:\n        Let guess be (low + high) // 2.\n        Set tries to tries + 1.\n        If guess equals secret:\n            Return tries.\n        If guess is greater than secret:\n            Set high to guess - 1.\n        Otherwise:\n            Set low to guess + 1.\n    Return tries.\n\n## Main\nShow \"999 found in {find(999)} tries.\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-tutorial-guesses",
                question: "Why can the program always find the secret in at most 7 tries?",
                answer: "Each guess halves the remaining range, and seven halvings cover a hundred numbers (2^7 = 128).",
            },
            RecapQuestion {
                id: "rq-tutorial-midpoint",
                question: "What does `(low + high) // 2` print when low is 1 and high is 100 — and why not 50.5?",
                answer: "50 — `//` is floor division; integer arithmetic stays whole.",
            },
            RecapQuestion {
                id: "rq-tutorial-loop",
                question: "Which statement ends the `While true:` loop when the guess is right?",
                answer: "`Break.` — it exits the loop immediately.",
            },
        ],
    },
    Section {
        id: "part-1-checkpoint",
        number: 5,
        title: "Checkpoint: First Steps",
        part: "Part I: First Steps",
        content: r#"
Part I is done — prove it to yourself. The three challenges below blend what you
met across the part: the program shape, the twin spellings, and the tutorial's
midpoint arithmetic. Then the recall cards collect on the part's promises.

Nothing here is new. If a challenge stumps you, the section to reread is named
in its hints — that is the point of a checkpoint.
"#,
        hook: "",
        objectives: &[
            "combine the twin spellings, midpoint arithmetic, and printing in one program",
            "diagnose which Part I section to reread when a challenge stumps you",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[
            Exercise {
                id: "cp1-twins-hello",
                kind: ExerciseKind::Challenge,
                concepts: &["hello world", "English/symbolic twins"],
                prompt: "Print exactly `Hello, LOGOS!`, then print whether `2 plus 3` equals `2 + 3`.",
                mode: ExampleMode::Imperative,
                expected: "Hello, LOGOS!\ntrue",
                starter: "## Main\n",
                hints: &[
                    "Two lines: a greeting, then a comparison.",
                    "English and symbols are twins — compare the two spellings of addition with `equals`.",
                ],
                reference: "## Main\nShow \"Hello, LOGOS!\".\nShow 2 plus 3 equals 2 + 3.",
            },
            Exercise {
                id: "cp1-midpoint",
                kind: ExerciseKind::Challenge,
                concepts: &["staged build-one-program tutorial", "English/symbolic twins"],
                prompt: "Compute the game's first guess for the range 1–100, print it, then print whether it is at least 50 — English spelling.",
                mode: ExampleMode::Imperative,
                expected: "50\ntrue",
                starter: "## Main\n",
                hints: &[
                    "The tutorial's midpoint line is the one you want.",
                    "`>=` has an English twin.",
                ],
                reference: "## Main\nLet guess be (1 + 100) // 2.\nShow guess.\nShow guess is at least 50.",
            },
            Exercise {
                id: "cp1-adapt-guess",
                kind: ExerciseKind::Challenge,
                concepts: &["staged build-one-program tutorial", "equals-not-is rule"],
                prompt: "The secret is 88 and the range is 1–175. Print the first midpoint guess, then print whether it already equals the secret.",
                mode: ExampleMode::Imperative,
                expected: "88\ntrue",
                starter: "## Main\n",
                hints: &[
                    "Midpoint = (low + high) // 2.",
                    "Compare guess and secret with `equals` — the printed value is the comparison's Bool.",
                ],
                reference: "## Main\nLet secret be 88.\nLet guess be (1 + 175) // 2.\nShow guess.\nShow guess equals secret.",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "cq1-main-block",
                question: "What does `## Main` mark?",
                answer: "The program's entry point — the block that runs.",
            },
            RecapQuestion {
                id: "cq1-twins",
                question: "What are the English twins of `>=` and of `==`?",
                answer: "`is at least`; `equals`.",
            },
            RecapQuestion {
                id: "cq1-slice-count",
                question: "`items 2 through 4 of xs` — how many items?",
                answer: "Three — both ends are included.",
            },
            RecapQuestion {
                id: "cq1-seven",
                question: "Why does the guessing game never need more than 7 tries for 1–100?",
                answer: "Each guess halves the range, and 2^7 = 128 covers 100.",
            },
            RecapQuestion {
                id: "cq1-let-set",
                question: "Which statement creates a variable, and which changes one?",
                answer: "`Let x be ...` creates; `Set x to ...` changes.",
            },
            RecapQuestion {
                id: "cq1-undefined",
                question: "What happens if you run `Show total.` without a `Let total` line first?",
                answer: "A Socratic error: `Undefined variable: total` — LOGOS never invents a value.",
            },
        ],
    },
];
