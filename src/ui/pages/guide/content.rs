//! Embedded guide content for the Programmer's Guide page.
//!
//! Contains all 22 sections from PROGRAMMERS_LANGUAGE_STARTER.md as Rust constants.
//! WASM cannot read files at runtime, so we embed the content at compile time.

/// Mode for code examples - determines how "Run" executes them
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ExampleMode {
    /// Logic mode: compile to First-Order Logic (FOL)
    Logic,
    /// Imperative mode: execute via WASM interpreter
    Imperative,
}

/// A code example within a section
#[derive(Clone, Debug)]
pub struct CodeExample {
    pub id: &'static str,
    pub label: &'static str,
    pub mode: ExampleMode,
    pub code: &'static str,
}

/// A section of the guide
#[derive(Clone, Debug)]
pub struct Section {
    pub id: &'static str,
    pub number: u8,
    pub title: &'static str,
    pub part: &'static str,
    pub content: &'static str,
    pub examples: &'static [CodeExample],
}

/// All guide sections organized by part
pub const SECTIONS: &[Section] = &[
    // ============================================================
    // Part I: Programming in LOGOS (Sections 1-14)
    // ============================================================

    Section {
        id: "introduction",
        number: 1,
        title: "Introduction",
        part: "Part I: Programming in LOGOS",
        content: r#"
### What is LOGOS?

LOGOS is a programming language where you write code in natural English. Instead of cryptic symbols and arcane syntax, you express your ideas in sentences that read like plain prose—and those sentences compile into efficient, executable programs.

LOGOS has two modes:

| Mode | What It Does | Output |
|------|--------------|--------|
| **Imperative Mode** | Write executable programs | Rust code (compiled to native binaries) |
| **Logic Mode** | Translate English to formal logic | First-Order Logic notation |

This guide focuses primarily on **Imperative Mode**—using LOGOS as a programming language. Part III covers Logic Mode for those interested in formal semantics.

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
        examples: &[],
    },

    Section {
        id: "getting-started",
        number: 2,
        title: "Getting Started",
        part: "Part I: Programming in LOGOS",
        content: r#"
### Hello World

Every programming journey begins with Hello World. In LOGOS:
"#,
        examples: &[
            CodeExample {
                id: "hello-world",
                label: "Hello World",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Show "Hello, World!"."#,
            },
            CodeExample {
                id: "program-structure",
                label: "Program Structure",
                mode: ExampleMode::Imperative,
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
                code: r#"## Main
Let name be "Alice".
Let age be 25.
Show "Name: " + name.
Show "Age: " + age."#,
            },
        ],
    },

    Section {
        id: "variables-and-types",
        number: 3,
        title: "Variables and Types",
        part: "Part I: Programming in LOGOS",
        content: r#"
Variables are containers that hold values. In LOGOS, you create and modify variables using natural English sentences.

### Creating Variables

Use `Let` to create a new variable. The word `be` assigns a value to the variable.

### Changing Values

Use `Set` to change an existing variable. The difference between `Let` and `Set`:
- `Let` creates a *new* variable
- `Set` modifies an *existing* variable

### Primitive Types

| Type | Description | Examples |
|------|-------------|----------|
| `Int` | Whole numbers | `5`, `-10`, `0`, `1000000` |
| `Bool` | True or false | `true`, `false` |
| `Text` | Strings of characters | `"Hello"`, `"LOGOS"`, `""` |
| `Float` | Decimal numbers | `3.14`, `-0.5`, `98.6` |

### Type Annotations

Usually, LOGOS infers the type from the value you assign. But you can be explicit with `: Type`.
"#,
        examples: &[
            CodeExample {
                id: "creating-variables",
                label: "Creating Variables",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let x be 5.
Let name be "Bob".
Let is_active be true.
Let temperature be 98.6.
Show x.
Show name."#,
            },
            CodeExample {
                id: "changing-variables",
                label: "Changing Variables",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let x be 5.
Show x.

Set x to 10.
Show x.

Set x to x + 1.
Show x."#,
            },
            CodeExample {
                id: "text-concat",
                label: "Text Concatenation",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let first be "Hello".
Let second be "World".
Let message be first + ", " + second + "!".
Show message."#,
            },
        ],
    },

    Section {
        id: "operators",
        number: 4,
        title: "Operators and Expressions",
        part: "Part I: Programming in LOGOS",
        content: r#"
Operators let you combine values into expressions. LOGOS supports both symbolic operators (like `+`) and English words (like `plus`).

### Arithmetic

| Operation | Symbol | English |
|-----------|--------|---------|
| Addition | `+` | `plus` |
| Subtraction | `-` | `minus` |
| Multiplication | `*` | `times` |
| Division | `/` | `divided by` |
| Modulo | `%` | `modulo` |

### Comparisons

| Operation | Symbol | English |
|-----------|--------|---------|
| Less than | `<` | `is less than` |
| Greater than | `>` | `is greater than` |
| Less or equal | `<=` | `is at most` |
| Greater or equal | `>=` | `is at least` |
| Equal | `==` | `equals` |
| Not equal | `!=` | `is not` |

### Logical Operators

| Operation | Keyword | Meaning |
|-----------|---------|---------|
| AND | `and` | Both must be true |
| OR | `or` | At least one must be true |
| NOT | `not` | Inverts true/false |
"#,
        examples: &[
            CodeExample {
                id: "arithmetic",
                label: "Arithmetic Operations",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let a be 10.
Let b be 3.

Let sum be a + b.
Let diff be a - b.
Let prod be a * b.
Let quot be a / b.
Let rem be a % b.

Show "Sum: " + sum.
Show "Difference: " + diff.
Show "Product: " + prod.
Show "Quotient: " + quot.
Show "Remainder: " + rem."#,
            },
            CodeExample {
                id: "comparisons",
                label: "Comparisons",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let x be 5.
Let y be 10.

Show x is less than y.
Show x is greater than y.
Show x equals 5.
Show x is at most 5.
Show x is at least 5."#,
            },
            CodeExample {
                id: "logical",
                label: "Logical Operators",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let a be true.
Let b be false.

Show a and b.
Show a or b.
Show not a."#,
            },
        ],
    },

    Section {
        id: "control-flow",
        number: 5,
        title: "Control Flow",
        part: "Part I: Programming in LOGOS",
        content: r#"
Control flow determines which code runs and in what order. LOGOS provides conditionals and loops using natural English syntax.

### Conditionals

Use `If` to execute code only when a condition is true. The colon (`:`) after the condition opens an indented block.

### If/Otherwise

Use `Otherwise` to handle the false case.

### While Loops

Use `While` to repeat code as long as a condition is true.

### For-Each Loops

Use `Repeat for` to iterate over collections.
"#,
        examples: &[
            CodeExample {
                id: "if-otherwise",
                label: "If/Otherwise",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let temperature be 72.

If temperature is greater than 80:
    Show "It's hot!".
Otherwise:
    Show "It's comfortable."."#,
            },
            CodeExample {
                id: "while-loop",
                label: "While Loop",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let count be 1.

While count is at most 5:
    Show count.
    Set count to count + 1."#,
            },
            CodeExample {
                id: "for-each",
                label: "For-Each Loop",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let numbers be [1, 2, 3, 4, 5].

Repeat for n in numbers:
    Show n."#,
            },
            CodeExample {
                id: "grading",
                label: "Grading Example",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let score be 85.

If score is at least 90:
    Show "Grade: A".
If score is at least 80 and score is less than 90:
    Show "Grade: B".
If score is at least 70 and score is less than 80:
    Show "Grade: C".
If score is less than 70:
    Show "Grade: F"."#,
            },
        ],
    },

    Section {
        id: "functions",
        number: 6,
        title: "Functions",
        part: "Part I: Programming in LOGOS",
        content: r#"
Functions are reusable blocks of code. In LOGOS, you define functions using natural English headers that describe what the function does.

### Defining Functions

A function definition starts with `## To` followed by the function name.

### Parameters

Functions can accept parameters—values passed in when the function is called. Use `and` to separate multiple parameters.

### Return Values

Use `-> Type` to specify what the function returns.

### Recursion

Functions can call themselves. This is called recursion. Every recursive function needs:
1. A **base case** — when to stop recursing
2. A **recursive case** — calling itself with a "smaller" problem
"#,
        examples: &[
            CodeExample {
                id: "simple-function",
                label: "Simple Function",
                mode: ExampleMode::Imperative,
                code: r#"## To greet (name: Text):
    Show "Hello, " + name + "!".

## Main
greet("Alice").
greet("Bob")."#,
            },
            CodeExample {
                id: "function-return",
                label: "Function with Return",
                mode: ExampleMode::Imperative,
                code: r#"## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
Let sum be add(3, 5).
Show sum."#,
            },
            CodeExample {
                id: "factorial",
                label: "Recursive Factorial",
                mode: ExampleMode::Imperative,
                code: r#"## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show factorial(5)."#,
            },
        ],
    },

    Section {
        id: "collections",
        number: 7,
        title: "Collections",
        part: "Part I: Programming in LOGOS",
        content: r#"
Collections hold multiple values. The primary collection type in LOGOS is `Seq` (sequence), similar to lists or arrays in other languages.

### Creating Lists

Create a list with square brackets, or create an empty list with a type.

### Accessing Elements

LOGOS uses **1-based indexing**. The first element is at position 1, not 0. Why? Because that's how humans count.

### Modifying Collections

- `Push` to add an element to the end
- `Pop` to remove and get the last element
- `copy of` to create a deep copy

### Slicing

Extract a portion of a list with `through`. Slicing is **inclusive** on both ends.
"#,
        examples: &[
            CodeExample {
                id: "creating-lists",
                label: "Creating Lists",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let numbers be [1, 2, 3, 4, 5].
Let names be ["Alice", "Bob", "Charlie"].
Show numbers.
Show names."#,
            },
            CodeExample {
                id: "accessing-elements",
                label: "Accessing Elements (1-indexed)",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let fruits be ["apple", "banana", "cherry"].

Let first be item 1 of fruits.
Let second be item 2 of fruits.
Let third be item 3 of fruits.

Show first.
Show second.
Show third."#,
            },
            CodeExample {
                id: "push-pop",
                label: "Push and Pop",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let numbers be [1, 2, 3].
Push 4 to numbers.
Push 5 to numbers.
Show numbers."#,
            },
            CodeExample {
                id: "list-iteration",
                label: "Iterating and Accumulating",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let numbers be [10, 20, 30, 40, 50].
Let total be 0.

Repeat for n in numbers:
    Set total to total + n.

Show "Total: " + total."#,
            },
        ],
    },

    Section {
        id: "user-types",
        number: 8,
        title: "User-Defined Types",
        part: "Part I: Programming in LOGOS",
        content: r#"
Beyond primitive types and collections, LOGOS lets you define your own types to model your problem domain.

### Structs

A struct (structure) groups related values together. Define one in a `## Definition` block using `A [TypeName] has:` syntax.

### Creating Instances

Use `a new [Type] with [fields]` to create instances.

### Accessing Fields

Use `'s` (possessive) to access fields.

### Enums

An enum (enumeration) defines a type that can be one of several variants using `A [TypeName] is either:` syntax.

### Pattern Matching

Use `Inspect` to handle different enum variants with `When` clauses.
"#,
        examples: &[
            CodeExample {
                id: "struct-basic",
                label: "Basic Struct",
                mode: ExampleMode::Imperative,
                code: r#"## Definition
A Point has:
    an x: Int.
    a y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Show p's x.
Show p's y."#,
            },
            CodeExample {
                id: "struct-person",
                label: "Person Struct",
                mode: ExampleMode::Imperative,
                code: r#"## Definition
A Person has:
    a name: Text.
    an age: Int.

## Main
Let alice be a new Person with name "Alice" and age 25.
Show alice's name.
Show alice's age."#,
            },
            CodeExample {
                id: "enum-direction",
                label: "Simple Enum",
                mode: ExampleMode::Imperative,
                code: r#"## Definition
A Direction is either:
    North.
    South.
    East.
    West.

## Main
Let heading be North.
Show heading."#,
            },
        ],
    },

    Section {
        id: "generics",
        number: 9,
        title: "Generics",
        part: "Part I: Programming in LOGOS",
        content: r#"
Generics let you write types and functions that work with any type, not just specific ones.

### Generic Types

Define a generic type with `[T]` in the type name. The `[T]` is a placeholder that gets replaced with a real type when you use it.

### Multiple Type Parameters

You can have multiple type parameters like `[A]` and `[B]`.

### Generic Collections

Collections are generic types. `Seq of Int` is a sequence of integers.

### Nested Generics

You can nest generic types like `Seq of (Seq of Int)` for a matrix.
"#,
        examples: &[
            CodeExample {
                id: "generic-box",
                label: "Generic Box",
                mode: ExampleMode::Imperative,
                code: r#"## Definition
A Box of [T] has:
    a contents: T.

## Main
Let int_box be a new Box of Int with contents 42.
Let text_box be a new Box of Text with contents "Hello".

Show int_box's contents.
Show text_box's contents."#,
            },
            CodeExample {
                id: "generic-pair",
                label: "Generic Pair",
                mode: ExampleMode::Imperative,
                code: r#"## Definition
A Pair of [A] and [B] has:
    a first: A.
    a second: B.

## Main
Let p be a new Pair of Int and Text with first 1 and second "one".
Show p's first.
Show p's second."#,
            },
        ],
    },

    Section {
        id: "memory-ownership",
        number: 10,
        title: "Memory and Ownership",
        part: "Part I: Programming in LOGOS",
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
- Show multiple times (all OK - just reading)
- Sequential mutation
"#,
        examples: &[
            CodeExample {
                id: "ownership-show",
                label: "Show (Borrow)",
                mode: ExampleMode::Imperative,
                code: r#"## To display (data: Text):
    Show "Displaying: " + data.

## Main
Let profile be "User Profile Data".
Show profile to display.
Show profile."#,
            },
        ],
    },

    Section {
        id: "zones",
        number: 11,
        title: "The Zone System",
        part: "Part I: Programming in LOGOS",
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

### Zone Types

| Zone Type | Allocation | Access | Use Case |
|-----------|------------|--------|----------|
| Heap | O(1) bump | Read/Write | Temporary data |
| Mapped | OS page fault | Read-only | Large file processing |

### When to Use Zones

Use zones when:
- Processing large amounts of temporary data
- Performance is critical (games, simulations)
- Memory allocation patterns are predictable
- You want instant cleanup
"#,
        examples: &[
            CodeExample {
                id: "zone-basic",
                label: "Basic Zone",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Inside a zone called "WorkSpace":
    Let temp_data be [1, 2, 3, 4, 5].
    Show temp_data.
Show "Zone freed!"."#,
            },
        ],
    },

    Section {
        id: "concurrency",
        number: 12,
        title: "Concurrency",
        part: "Part I: Programming in LOGOS",
        content: r#"
LOGOS provides safe concurrency through structured patterns. No data races, no deadlocks.

### Two Kinds of Concurrent Work

| Pattern | Keyword | Use For | Compiles To |
|---------|---------|---------|-------------|
| **Async** | `Attempt all` | I/O-bound tasks (network, files) | tokio::join! |
| **Parallel** | `Simultaneously` | CPU-bound tasks (computation) | rayon/threads |

### Attempt All (Async I/O)

Use `Attempt all of the following:` for I/O operations that wait on external resources. All operations run concurrently, and the program waits until all complete.

### Simultaneously (Parallel CPU)

Use `Simultaneously:` for CPU-intensive work. Both computations run in parallel on different CPU cores.

### Race: First to Finish

Use `Await the first success of:` when you want the fastest response. Both start, the first to succeed wins.

### Ownership and Concurrency

The ownership system prevents data races. Multiple reads are OK, but concurrent writes are prevented.
"#,
        examples: &[
            CodeExample {
                id: "concurrency-example",
                label: "Concurrency Patterns",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Show "Concurrency in LOGOS uses:".
Show "- Attempt all: for async I/O".
Show "- Simultaneously: for parallel CPU".
Show "- Await first success: for racing"."#,
            },
        ],
    },

    Section {
        id: "error-handling",
        number: 13,
        title: "Error Handling",
        part: "Part I: Programming in LOGOS",
        content: r#"
LOGOS uses **Socratic error messages**—friendly, educational feedback that teaches while it corrects.

### The Philosophy

Instead of cryptic compiler errors, LOGOS explains:
1. **What** went wrong
2. **Where** it happened
3. **Why** it's a problem
4. **How** to fix it

### The Failure Type

Functions that might fail return a `Result`. Use pattern matching to handle success and failure cases.

### Error Propagation

Errors propagate naturally through return values. Handle them where appropriate.
"#,
        examples: &[
            CodeExample {
                id: "result-type",
                label: "Using Result Type",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Show "Error handling in LOGOS:".
Show "- Functions return Result type".
Show "- Pattern match with Inspect".
Show "- Socratic messages explain errors"."#,
            },
        ],
    },

    Section {
        id: "advanced-features",
        number: 14,
        title: "Advanced Features",
        part: "Part I: Programming in LOGOS",
        content: r#"
### Refinement Types

Refinement types add constraints to base types. The constraint is checked at runtime or compile time with Z3.

### Assertions

Use `Assert` to verify conditions in your code. If the assertion fails, the program stops with an error message.

### Trust with Reason

Use `Trust` when you know something is true but the compiler can't verify it. The `because` clause documents why you believe the condition holds.

### Modules

Organize code across multiple files with `Use`.
"#,
        examples: &[
            CodeExample {
                id: "refinement-types",
                label: "Refinement Types",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let positive: Int where it > 0 be 5.
Let percentage: Int where it >= 0 and it <= 100 be 85.
Show positive.
Show percentage."#,
            },
            CodeExample {
                id: "assertions",
                label: "Assertions",
                mode: ExampleMode::Imperative,
                code: r#"## To divide_safe (a: Int) and (b: Int) -> Int:
    Assert that b is not 0.
    Return a / b.

## Main
Let result be divide_safe(10, 2).
Show result."#,
            },
        ],
    },

    // ============================================================
    // Part II: Project Structure (Sections 15-17)
    // ============================================================

    Section {
        id: "modules",
        number: 15,
        title: "Modules",
        part: "Part II: Project Structure",
        content: r#"
Organize large programs across multiple files using the module system.

### Importing Modules

Use `Use` to import a module.

### Qualified Access

Access module contents with the possessive `'s`.

### Creating Modules

Each `.md` file is a module. The filename becomes the module name.

### Visibility

By default, all definitions are public. Mark fields private with no `public` modifier.
"#,
        examples: &[
            CodeExample {
                id: "module-import",
                label: "Importing Modules",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Show "Module syntax:".
Show "Use Math.".
Show "Let x be Math's square(5)."."#,
            },
        ],
    },

    Section {
        id: "cli-largo",
        number: 16,
        title: "The CLI: largo",
        part: "Part II: Project Structure",
        content: r#"
LOGOS projects are built with `largo`, the LOGOS build tool.

### Creating a Project

`largo new myproject` creates a new project with a `Largo.toml` manifest and `src/main.md`.

### Build Commands

| Command | Description |
|---------|-------------|
| `largo build` | Compile the project |
| `largo build --release` | Compile with optimizations |
| `largo run` | Build and run |
| `largo check` | Type check without compiling |
| `largo test` | Run tests |
| `largo audit` | List Trust statements |

### Project Manifest

The `Largo.toml` file defines package metadata and dependencies.
"#,
        examples: &[
            CodeExample {
                id: "largo-commands",
                label: "largo Commands",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Show "largo commands:".
Show "- largo new <name>".
Show "- largo build".
Show "- largo run".
Show "- largo check".
Show "- largo test"."#,
            },
        ],
    },

    Section {
        id: "stdlib",
        number: 17,
        title: "Standard Library",
        part: "Part II: Project Structure",
        content: r#"
LOGOS provides built-in functions for common operations.

### Currently Available

These built-ins work in both the playground and compiled programs:

- `Show x.` — Output values to the console
- `length of x` — Get the length of a list or text
- `format(x)` — Convert any value to text
- `abs(n)` — Absolute value of a number
- `min(a, b)` — Minimum of two integers
- `max(a, b)` — Maximum of two integers

### Coming Soon

Additional modules are planned for future releases:

- **File** — `read`, `write`, `exists` for file operations
- **Time** — `now`, `sleep` for timing and delays
- **Random** — `randomInt`, `randomFloat`, `choice`
- **Env** — Environment variables and command-line arguments

These will be available in compiled programs. Some features may have limited support in the browser playground due to WASM constraints.
"#,
        examples: &[
            CodeExample {
                id: "stdlib-example",
                label: "Standard Library",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let nums be [5, -3, 8, -1, 4].
Let text be "Hello".

Show "Built-in functions:".
Show "length of nums = " + format(length of nums).
Show "length of text = " + format(length of text).
Show "abs(-42) = " + format(abs(-42)).
Show "min(10, 3) = " + format(min(10, 3)).
Show "max(10, 3) = " + format(max(10, 3))."#,
            },
        ],
    },

    // ============================================================
    // Part III: Logic Mode (Section 18)
    // ============================================================

    Section {
        id: "logic-mode",
        number: 18,
        title: "Logic Mode",
        part: "Part III: Logic Mode",
        content: r#"
LOGOS can translate English sentences into First-Order Logic (FOL). This is useful for formal verification, knowledge representation, and understanding the logical structure of natural language.

### Quantifiers

| English | Symbol | Output |
|---------|--------|--------|
| All X are Y | `∀` | `∀x(X(x) → Y(x))` |
| Some X is Y | `∃` | `∃x(X(x) ∧ Y(x))` |
| No X is Y | `¬∃` | `¬∃x(X(x) ∧ Y(x))` |

### Connectives

| English | Symbol |
|---------|--------|
| and | `∧` |
| or | `∨` |
| not | `¬` |
| if...then | `→` |
| if and only if | `↔` |

### Modals

| English | Symbol |
|---------|--------|
| can, may, might | `◇` (possibility) |
| must | `□` (necessity) |

### Tense and Aspect

- `PAST(P)` — past tense
- `FUT(P)` — future tense
- `PROG(P)` — progressive aspect
- `PERF(P)` — perfect aspect
"#,
        examples: &[
            CodeExample {
                id: "logic-universal",
                label: "Universal Quantifier",
                mode: ExampleMode::Logic,
                code: "All birds fly.",
            },
            CodeExample {
                id: "logic-existential",
                label: "Existential Quantifier",
                mode: ExampleMode::Logic,
                code: "Some cats sleep.",
            },
            CodeExample {
                id: "logic-negative",
                label: "Negative Quantifier",
                mode: ExampleMode::Logic,
                code: "No fish fly.",
            },
            CodeExample {
                id: "logic-conditional",
                label: "Conditional",
                mode: ExampleMode::Logic,
                code: "If John runs, then Mary walks.",
            },
            CodeExample {
                id: "logic-modal",
                label: "Modal Operators",
                mode: ExampleMode::Logic,
                code: "John can swim.",
            },
        ],
    },

    // ============================================================
    // Part IV: Proofs and Verification (Sections 19-20)
    // ============================================================

    Section {
        id: "assertions-trust",
        number: 19,
        title: "Assertions and Trust",
        part: "Part IV: Proofs and Verification",
        content: r#"
LOGOS bridges imperative programming with formal verification through assertions and proof statements.

### Assert

Use `Assert` to verify conditions at runtime. If an assertion fails, the program stops with a clear error message.

### Trust with Justification

Use `Trust` for conditions the compiler can't verify automatically. The `because` clause is **mandatory**—it documents your reasoning.

### Trust Generates Debug Assertions

In development builds, `Trust` becomes a `debug_assert!`. In release builds, it generates no code—the trust is assumed.

### Auditing Trust Statements

Find all trust statements in your codebase with `largo audit`.

### Proof Blocks (Advanced)

For formal verification, use theorem blocks with proofs documented in comments.
"#,
        examples: &[
            CodeExample {
                id: "assert-example",
                label: "Assert",
                mode: ExampleMode::Imperative,
                code: r#"## To withdraw (amount: Int) from (balance: Int) -> Int:
    Assert that amount is greater than 0.
    Assert that amount is at most balance.
    Return balance - amount.

## Main
Let result be withdraw(50, 100).
Show result."#,
            },
            CodeExample {
                id: "trust-example",
                label: "Trust with Justification",
                mode: ExampleMode::Imperative,
                code: r#"## To process_positive (n: Int) -> Int:
    Trust that n is greater than 0 because "caller guarantees positive input".
    Return n * 2.

## Main
Let result be process_positive(5).
Show result."#,
            },
        ],
    },

    Section {
        id: "z3-verification",
        number: 20,
        title: "Z3 Static Verification",
        part: "Part IV: Proofs and Verification",
        content: r#"
LOGOS can use the Z3 SMT solver to verify refinement types at compile time.

### What is Z3?

Z3 is a theorem prover. Instead of checking constraints at runtime, Z3 proves (or disproves) them at compile time.

| Approach | When Checked | If Violated |
|----------|--------------|-------------|
| Runtime assertion | When code runs | Program crashes |
| Z3 verification | At compile time | Compilation fails |

### Variable Tracking

Z3 tracks constraints through variable assignments.

### Compound Predicates

Multiple constraints can be combined.

### Function Preconditions

Z3 verifies function contracts.

### Enabling Z3 Verification

Enable with `largo build --verify` or in `Largo.toml`.

### What Z3 Can Prove

| Constraint Type | Example | Z3 Support |
|-----------------|---------|------------|
| Integer bounds | `it > 0`, `it < 100` | Full |
| Equality | `it == 5` | Full |
| Arithmetic | `it * 2 < 100` | Full |
| Boolean logic | `it > 0 and it < 10` | Full |
"#,
        examples: &[
            CodeExample {
                id: "z3-refinement",
                label: "Z3 Refinement Types",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let positive: Int where it > 0 be 5.
Let bounded: Int where it >= 0 and it <= 100 be 85.
Show "Positive: " + positive.
Show "Bounded: " + bounded."#,
            },
        ],
    },

    // ============================================================
    // Part V: Reference (Sections 21-22)
    // ============================================================

    Section {
        id: "complete-examples",
        number: 21,
        title: "Complete Examples",
        part: "Part V: Reference",
        content: r#"
This section contains complete, runnable programs demonstrating various LOGOS features.

### Mergesort

A complete, recursive sorting algorithm.

### Factorial

Classic recursive example.

### Working with Structs

A complete example with custom types.

### Collection Processing

Common patterns for working with collections.
"#,
        examples: &[
            CodeExample {
                id: "example-factorial",
                label: "Factorial",
                mode: ExampleMode::Imperative,
                code: r#"## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Let result be factorial(5).
Show "5! = " + result."#,
            },
            CodeExample {
                id: "example-fibonacci",
                label: "Fibonacci",
                mode: ExampleMode::Imperative,
                code: r#"## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show "Fibonacci sequence:".
Let i be 0.
While i is less than 10:
    Show fib(i).
    Set i to i + 1."#,
            },
            CodeExample {
                id: "example-filter",
                label: "Filter Positive Numbers",
                mode: ExampleMode::Imperative,
                code: r#"## Main
Let data be [-2, 5, -1, 8, 3, -4, 7].
Let positives be a new Seq of Int.

Repeat for n in data:
    If n is greater than 0:
        Push n to positives.

Show "Positives: " + positives."#,
            },
        ],
    },

    Section {
        id: "quick-reference",
        number: 22,
        title: "Quick Reference",
        part: "Part V: Reference",
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

**Collections:**
- `[1, 2, 3]` — List literal
- `item 1 of items` — Access (1-indexed)
- `Push value to items.` — Add to end
- `length of items` — Get length

### Ownership Verbs

| Verb | Meaning |
|------|---------|
| `Give x to f.` | Move ownership |
| `Show x to f.` | Borrow (read) |
| `Let f modify x.` | Mutable borrow |
| `copy of x` | Clone |

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
        examples: &[],
    },
];

/// Get all sections
pub fn get_all_sections() -> &'static [Section] {
    SECTIONS
}

/// Get sections by part
pub fn get_sections_by_part(part: &str) -> Vec<&'static Section> {
    SECTIONS.iter().filter(|s| s.part == part).collect()
}

/// Get a section by ID
pub fn get_section_by_id(id: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|s| s.id == id)
}

/// Get all unique part names in order
pub fn get_parts() -> Vec<&'static str> {
    let mut parts = Vec::new();
    for section in SECTIONS {
        if parts.last() != Some(&section.part) {
            parts.push(section.part);
        }
    }
    parts
}
