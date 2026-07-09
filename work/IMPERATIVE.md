# LOGOS Imperative Architecture & Semantics

> "The art of programming is the art of organizing complexity, of mastering multitude and avoiding its bastard chaos as effectively as possible." — Edsger W. Dijkstra

This document outlines the **Imperative Mode** of the LOGOS system (Mode 1). While the Logical Mode handles abstract truth and possible worlds, the Imperative Mode is a fully-featured, statically-typed programming language designed to compile directly to efficient Rust.

It introduces a novel paradigm: **Natural Language Systems Programming**, enabling low-level control (ownership, concurrency, memory layout) through high-level English prose.

---

## 1. Core Syntax: The "Literate" Paradigm

LOGOS programs read like technical specifications but execute like systems code.

### 1.1 Variables & Mutation
*   **Binding:** `Let x be 5.` (Immutable by default)
*   **Mutation:** `Set x to 10.` (Requires mutable binding)
*   **Type Annotation:** `Let x: Int be 5.`

### 1.2 Control Flow
*   **Conditionals:**
    ```text
    If x is greater than 10:
        Print "Large".
    Otherwise:
        Print "Small".
    ```
*   **Loops:**
    *   `While x is positive: ...`
    *   `Repeat for item in list: ...`
    *   `Repeat for i from 1 to 10: ...`

### 1.3 Functions
Defined using the `##` header syntax or `To [verb]` phrasing.
```text
## To calculate_area (width: Real) and (height: Real) -> Real:
    Return width * height.
```

---

## 2. The Type System

LOGOS implements a robust, static type system with algebraic data types (ADTs) and generics.

### 2.1 Structural Types (Structs)
Defined by "Has-A" relationships.
```text
A Point has:
    x, which is Real.
    y, which is Real.
```

### 2.2 Sum Types (Enums)
Defined by "Is-A" relationships.
```text
A Shape is either:
    A Circle with a radius (Real).
    A Rectangle with width (Real) and height (Real).
```

### 2.3 Pattern Matching
Pattern matching is handled via the `Inspect` statement.
```text
Inspect shape:
    When Circle (radius: r): ...
    When Rectangle (width: w): ...
```

---

## 3. Ownership in English (Memory Safety)

LOGOS maps Rust's ownership model (Move vs Borrow) to intuitive English verbs. This ensures memory safety without a garbage collector.

### 3.1 Ownership Transfer (Move)
*   **Keyword:** `Give`
*   **Semantics:** Transfers ownership of the value. The original variable becomes invalid.
*   **Example:** `Give data to process_function.`

### 3.2 Immutable Reference (Borrow)
*   **Keyword:** `Show`
*   **Semantics:** Passes a read-only reference (`&T`). The original variable remains valid.
*   **Example:** `Show config to display_settings.`

### 3.3 Mutable Reference
*   **Keyword:** `Lend` (Planned) / `Show ... mutably`
*   Currently handled via `Set` on fields or specific mutable APIs.

---

## 4. Structured Concurrency & Parallelism

LOGOS distinguishes between **Waiting** (Async) and **Doing** (Parallelism).

### 4.1 Async/Await (`Attempt all`)
Maps to `tokio::join!`. Used for I/O-bound tasks.
```text
Attempt all of the following:
    Fetch data from url1.
    Fetch data from url2.
```

### 4.2 Parallelism (`Simultaneously`)
Maps to `rayon::join` or thread spawning. Used for CPU-bound tasks.
```text
Simultaneously:
    Process large_dataset_A.
    Process large_dataset_B.
```

### 4.3 CSP (Communicating Sequential Processes)
Go-style concurrency primitives for complex coordination.
*   **Channels:** `Let pipe be a new Pipe of Text.`
*   **Tasks:** `Launch a task to worker_function.`
*   **Select:** `Await the first of:` (handles races/timeouts).

---

## 5. Distributed Systems & CRDTs

LOGOS includes native support for Local-First software and P2P networking.

### 5.1 Shared Types (CRDTs)
Types marked as `Shared` automatically generate conflict-free merge logic.
*   **Counter:** `Shared Counter` (GCounter/PNCounter)
*   **Register:** `Shared Register` (LWWRegister)
*   **Sequence:** `Shared Sequence` (RGA/Logoot)

### 5.2 The `Sync` Statement
Binds a variable to a P2P topic.
*   `Sync score on "game-room-1".`
*   **Semantics:** Auto-publishes local mutations to the mesh; auto-merges incoming remote changes.

---

## 6. The Proof Bridge: Curry-Howard Correspondence

The most advanced feature of LOGOS is the bridge between **Imperative Code** (Mode 1) and **Logical Verification** (Mode 2).

### 6.1 Refinement Types (Dependent Types)
We allow types to be constrained by logical predicates.
*   **Syntax:** `Let x: Int where it > 0 be 10.`
*   **Semantics:** The compiler generates a **Proof Obligation**. It uses the Z3 solver to verify that the value `10` satisfies the predicate `it > 0`.
*   **Runtime:** If static verification is disabled, this compiles to `debug_assert!(x > 0)`.

### 6.2 The `Trust` Statement
Used to bridge the gap when the solver cannot prove a property automatically.
*   **Syntax:** `Trust that x > 0 because "I initialized it above".`
*   **Semantics:** Acts as a checked assumption. The "because" clause forces the programmer to document the justification, which is preserved in the compiled artifacts.

### 6.3 Propositions as Types
The `Refinement` variant in the type system explicitly embeds a `LogicExpr` (from Mode 2) into a `TypeExpr` (from Mode 1).
*   **Implementation:** `src/ast/type_expr.rs` -> `Refinement { base, predicate }`
*   **Verification:** `src/verification/solver.rs` extracts these predicates and proves validity before allowing compilation to proceed.

This implements a pragmatic subset of the **Curry-Howard Correspondence**:
1.  **Types** are sets of values.
2.  **Refinements** are propositions about those values.
3.  **Programs** are proofs that the values satisfy the propositions.
