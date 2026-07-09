# LOGOS Feature Matrix

**Purpose**: Comprehensive catalog of all imperative features and their interactions
**Audience**: Test writers, feature developers, bug hunters
**Status**: Living document, updated with each new feature/test
**Version**: 0.1.0
**Last Updated**: 2026-02-05

---

## Table of Contents

1. [Quick Reference](#quick-reference)
2. [Core Features Catalog](#core-features-catalog)
   - [Variables & Bindings](#variables--bindings)
   - [Control Flow](#control-flow)
   - [Functions](#functions)
   - [Data Structures](#data-structures)
   - [Collections](#collections)
   - [Ownership](#ownership)
   - [Memory Management](#memory-management)
   - [Concurrency](#concurrency)
   - [CRDTs](#crdts)
   - [Networking](#networking)
   - [I/O Operations](#io-operations)
   - [Verification](#verification)
3. [Feature Interaction Matrices](#feature-interaction-matrices)
   - [Primary 10×10 Matrix](#primary-1010-matrix)
   - [Async/Concurrency Interactions](#asyncconcurrency-interactions)
   - [CRDT Interactions](#crdt-interactions)
   - [Ownership Interactions](#ownership-interactions)
   - [Type System Interactions](#type-system-interactions)
4. [Testing Gap Analysis](#testing-gap-analysis)
   - [Top 20 Critical Gaps](#top-20-critical-gaps)
   - [Gap Categories](#gap-categories)
5. [Standard Library Reference](#standard-library-reference)
   - [Core Types](#core-types)
   - [Collections API](#collections-api)
   - [CRDT Types](#crdt-types)
   - [System Functions](#system-functions)
   - [Platform Support Matrix](#platform-support-matrix)
6. [Codegen Patterns](#codegen-patterns)
7. [Edge Cases & Gotchas](#edge-cases--gotchas)
8. [Appendices](#appendices)
   - [Test File Index](#test-file-index)
   - [AST Node Reference](#ast-node-reference)
   - [Parser Modes](#parser-modes)

---

## Quick Reference

### Legend

| Symbol | Meaning | Description |
|--------|---------|-------------|
| ✓ | Tested | Feature combination has passing tests |
| ○ | Untested | Should work but no tests exist |
| ✗ | Impossible | Type system prevents this combination |
| ? | Unknown | Unclear if this works, needs investigation |
| ⚠️ | Known Bug | Has tests but currently failing |
| P0 | Critical | Must test - likely user pattern |
| P1 | High | Important for robustness |
| P2 | Medium | Edge case worth documenting |
| P3 | Low | Completeness only |

### Feature Dimensions Summary

| Dimension | Statement Count | Expression Count | Test Coverage | Status |
|-----------|----------------|------------------|---------------|--------|
| **Variables** | 4 | 3 | 95% | Stable |
| **Control Flow** | 4 | 4 | 90% | Stable |
| **Functions** | 2 | 1 | 85% | Stable |
| **Data Structures** | 2 | 3 | 88% | Stable |
| **Collections** | 5 | 8 | 82% | Stable |
| **Ownership** | 3 | 1 | 70% | Needs Tests |
| **Memory** | 1 | 3 | 65% | Needs Tests |
| **Concurrency** | 11 | 0 | 90% | Stable |
| **CRDTs** | 9 | 0 | 75% | Needs Tests |
| **Networking** | 6 | 0 | 70% | Experimental |
| **I/O** | 2 | 0 | 85% | Stable |
| **Verification** | 3 | 0 | 60% | Experimental |
| **Types** | 0 | 5 | 80% | Stable |
| **TOTAL** | **52** | **28** | **82%** | — |

---

## Core Features Catalog

This section documents every imperative feature in LOGOS organized by category. Each feature includes:
- **AST Node**: The Rust enum variant
- **Syntax**: Natural language examples
- **Semantics**: What it does
- **Type Constraints**: Parameter types
- **Codegen Pattern**: Generated Rust code
- **Triggers**: Async/VFS/Network requirements
- **Tested With**: Test coverage references
- **Untested With**: Known gaps with priorities
- **Examples**: Working code samples

### Variables & Bindings

#### Feature: Let (Variable Binding)

**AST Node**: `Stmt::Let { var, ty, value, mutable }`
**Category**: Variables
**Phase Introduced**: Phase 1 (Garden Path)

**Syntax**:
```logos
Let x be 5.
Let x: Int be 5.
Let mutable x be 10.
```

**Semantics**: Introduces a new variable binding in the current scope. By default immutable (`let` in Rust). If `mutable` is true, generates `let mut` for reassignment.

**Type Constraints**:
- `var`: Symbol (identifier)
- `ty`: Optional type annotation (`TypeExpr`)
- `value`: Must be an expression producing a value
- `mutable`: Boolean flag

**Codegen Pattern**:
```rust
// Immutable
let x = 5;

// With type annotation
let x: i64 = 5;

// Mutable
let mut x = 10;
```

**Mutability Detection**: The compiler scans all statements to detect if a variable is reassigned via `Set`, `Push`, `Pop`, `Add`, `Remove`, or `SetIndex`. If so, it automatically generates `let mut`.

**Type Inference**: If `ty` is None, Rust type inference determines the type.

**Refinement Types**: If `ty` contains a `Refinement`, a `debug_assert!` is emitted after the binding to check the predicate.

**Triggers**:
- Async: No (unless `value` is async)
- VFS: No
- Networking: No

**Tested With**:
- All data types → `e2e_primitives.rs`, `e2e_structs.rs`, `e2e_collections.rs`
- Refinement types → `e2e_refinement.rs`
- Mutable bindings → `e2e_variables.rs`
- Generic types → `e2e_types.rs`
- Async functions → `e2e_async_cross_cutting.rs`

**Untested With**:
- Zone-allocated values (P2) - Should work
- CRDT types in zones (P1) - Unknown lifetime interactions
- Persistent<T> + Generics (P1) - Complex type inference

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Basic binding
Let x be 42.
Show x to show.
```

```logos
## Example 2: Type annotation
Let name: Text be "Alice".
```

```logos
## Example 3: Refinement type
Let positive: Int where it > 0 be 5.
Set positive to 10.  # Re-checks refinement
```

```logos
## Example 4: Mutable binding (auto-detected)
Let counter be 0.
Set counter to counter + 1.  # Compiler adds 'mut'
```

```logos
## Example 5: Generic type
Let numbers be a new Seq of Int.
Push 1 to numbers.
```

---

#### Feature: Set (Variable Mutation)

**AST Node**: `Stmt::Set { target, value }`
**Category**: Variables
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Set x to 10.
Set counter to counter + 1.
```

**Semantics**: Reassigns a variable that was previously bound with `Let`. In Rust, this becomes `variable = value;`. The variable must have been declared as mutable (either explicitly or via mutability analysis).

**Type Constraints**:
- `target`: Symbol (must be a previously bound variable)
- `value`: Expression matching the variable's type

**Codegen Pattern**:
```rust
x = 10;
counter = counter + 1;
```

**Refinement Re-checking**: If the target variable has a refinement type, the compiler re-emits the `debug_assert!` to ensure the invariant is preserved.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Primitive types → `e2e_variables.rs`
- Refinement types → `e2e_refinement.rs`
- In loops → `e2e_iteration.rs`
- In conditionals → `e2e_control_flow.rs`

**Untested With**:
- Setting zone-scoped variables from outer scope (P1) - Lifetime escapes?
- Setting Persistent<T> variables (P2) - May need special handling
- Setting in concurrent blocks (P0) - Race conditions?

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Simple reassignment
Let mutable x be 5.
Set x to 10.
```

```logos
## Example 2: Increment pattern
Let counter be 0.
Set counter to counter + 1.
```

```logos
## Example 3: Refinement re-check
Let positive: Int where it > 0 be 5.
Set positive to 10.  # debug_assert!(10 > 0);
```

---

#### Feature: SetField (Field Mutation)

**AST Node**: `Stmt::SetField { object, field, value }`
**Category**: Variables
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Set p's x to 10.
Set user's age to 25.
```

**Semantics**: Mutates a field of a struct. The object must be mutable. For CRDT fields (LWWRegister, MVRegister), this generates `.set(value)` instead of direct field assignment.

**Type Constraints**:
- `object`: Expression evaluating to a struct
- `field`: Symbol (must be a valid field name)
- `value`: Expression matching the field's type

**Codegen Pattern**:
```rust
// Regular struct field
p.x = 10;

// CRDT field (LWWRegister)
p.name.set("Alice".to_string());
```

**CRDT Detection**: The compiler checks if the field type is `LWWRegister<T>` or `MVRegister<T>` and generates `.set()` calls instead of direct assignment.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Struct fields → `e2e_structs.rs`
- CRDT fields → `e2e_crdt.rs`
- Nested structs → `e2e_structs.rs`

**Untested With**:
- Setting shared struct fields (P0) - CRDT composition
- Setting generic fields (P1) - Type inference
- Setting fields in concurrent blocks (P1) - Race detection

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Struct field
A Point has:
    x, which is Real.
    y, which is Real.

## Main:
Let p be a new Point with x 0.0 and y 0.0.
Set p's x to 5.0.
```

```logos
## Example 2: CRDT field
A User has:
    Shared name, which is Text.

## Main:
Let user be a new User.
Set user's name to "Alice".  # Generates: user.name.set("Alice")
```

---

#### Feature: SetIndex (Index Mutation)

**AST Node**: `Stmt::SetIndex { collection, index, value }`
**Category**: Collections
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Set item 2 of numbers to 100.
```

**Semantics**: Mutates an element at a specific index in a collection. Collections use 1-based indexing. Generates `collection[index as usize - 1] = value;` in Rust.

**Type Constraints**:
- `collection`: Expression evaluating to `Seq<T>` (Vec)
- `index`: Expression evaluating to `Int` (i64)
- `value`: Expression matching element type `T`

**Codegen Pattern**:
```rust
// 1-based index conversion
numbers.logos_set(2i64, 100);
// Expands to: numbers[(2 - 1) as usize] = 100;
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Seq<Int> → `e2e_collections.rs`
- Seq<Text> → `e2e_collections.rs`

**Untested With**:
- Seq of structs (P1) - Complex types
- Seq of generics (P1) - Type inference
- In concurrent blocks (P1) - Race conditions

**Known Issues**:
- Index bounds checking is runtime only (no compile-time verification)

**Examples**:

```logos
## Example 1: Update element
Let numbers be [10, 20, 30].
Set item 2 of numbers to 99.
# Result: [10, 99, 30]
```

---

### Control Flow

#### Feature: If (Conditional Execution)

**AST Node**: `Stmt::If { cond, then_block, else_block }`
**Category**: Control Flow
**Phase Introduced**: Phase 1

**Syntax**:
```logos
If x > 0:
    Print "Positive".
Otherwise:
    Print "Non-positive".
```

**Semantics**: Executes `then_block` if `cond` evaluates to true, otherwise executes `else_block` (if present).

**Type Constraints**:
- `cond`: Expression evaluating to `Bool`
- `then_block`: Block of statements
- `else_block`: Optional block of statements

**Codegen Pattern**:
```rust
if x > 0 {
    println!("Positive");
} else {
    println!("Non-positive");
}
```

**Async Propagation**: If either block contains async statements, the entire if statement requires `.await` on futures.

**Triggers**:
- Async: Yes (if nested blocks contain async)
- VFS: Yes (if nested blocks use VFS)
- Networking: Yes (if nested blocks use networking)

**Tested With**:
- Simple conditions → `e2e_control_flow.rs`
- Nested ifs → `e2e_control_flow.rs`
- With async → `e2e_async_cross_cutting.rs`

**Untested With**:
- If with zone scopes (P2)
- If with CRDT mutations (P1)
- Nested ifs > 5 deep (P3)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Basic conditional
If age >= 18:
    Print "Adult".
Otherwise:
    Print "Minor".
```

```logos
## Example 2: No else branch
If score > 100:
    Print "High score!".
```

```logos
## Example 3: Nested conditionals
If x > 0:
    If x > 10:
        Print "Large positive".
    Otherwise:
        Print "Small positive".
Otherwise:
    Print "Non-positive".
```

---

#### Feature: While (Conditional Loop)

**AST Node**: `Stmt::While { cond, body, decreasing }`
**Category**: Control Flow
**Phase Introduced**: Phase 1

**Syntax**:
```logos
While x > 0:
    Set x to x - 1.

While items contains "bad":
    Remove "bad" from items.
```

**Semantics**: Repeatedly executes `body` while `cond` is true. Supports optional `decreasing` expression for termination proofs.

**Type Constraints**:
- `cond`: Expression evaluating to `Bool`
- `body`: Block of statements
- `decreasing`: Optional expression for loop variant

**Codegen Pattern**:
```rust
while x > 0 {
    check_preemption().await;  // Cooperative yielding
    x = x - 1;
}
```

**Cooperative Yielding**: All loops automatically include `check_preemption().await` to prevent thread starvation.

**Triggers**:
- Async: Yes (always, due to check_preemption)
- VFS: Yes (if body uses VFS)
- Networking: Yes (if body uses networking)

**Tested With**:
- Simple loops → `e2e_control_flow.rs`
- With mutations → `e2e_iteration.rs`
- Async loops → `e2e_async_cross_cutting.rs`

**Untested With**:
- While with CRDT mutations (P1)
- While with network I/O (P1)
- Nested while loops (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Countdown
Let counter be 10.
While counter > 0:
    Show counter to show.
    Set counter to counter - 1.
```

```logos
## Example 2: Collection processing
While numbers contains 0:
    Remove 0 from numbers.
```

---

#### Feature: Repeat (Iteration)

**AST Node**: `Stmt::Repeat { var, iterable, body }`
**Category**: Control Flow
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Repeat for item in items:
    Show item to show.

Repeat for i from 1 to 10:
    Push i to numbers.
```

**Semantics**: Iterates over a collection or range, binding each element to `var` and executing `body`.

**Type Constraints**:
- `var`: Symbol (loop variable)
- `iterable`: Expression evaluating to `Seq<T>`, `Set<T>`, `Map<K, V>`, or `Range`
- `body`: Block of statements

**Codegen Pattern**:
```rust
for item in items.iter() {
    check_preemption().await;
    show(&item);
}

for i in 1..=10 {
    check_preemption().await;
    numbers.push(i);
}
```

**Cooperative Yielding**: Like `While`, includes `check_preemption().await`.

**Triggers**:
- Async: Yes (always, due to check_preemption)
- VFS: Yes (if body uses VFS)
- Networking: Yes (if body uses networking)

**Tested With**:
- Seq iteration → `e2e_iteration.rs`
- Range iteration → `e2e_iteration.rs`
- Map iteration → `e2e_maps.rs`
- Set iteration → `e2e_sets.rs`

**Untested With**:
- Iterating over CRDT collections (P1)
- Nested iterations (P1)
- Iteration with ownership transfer (P1)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: List iteration
Let names be ["Alice", "Bob", "Charlie"].
Repeat for name in names:
    Print name.
```

```logos
## Example 2: Range iteration
Repeat for i from 1 to 5:
    Print i.
# Output: 1 2 3 4 5
```

---

#### Feature: Return (Early Exit)

**AST Node**: `Stmt::Return { value }`
**Category**: Control Flow
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Return 42.
Return.
```

**Semantics**: Exits the current function, optionally returning a value.

**Type Constraints**:
- `value`: Optional expression matching function return type

**Codegen Pattern**:
```rust
return 42;
return;
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- With value → `e2e_functions.rs`
- Without value → `e2e_functions.rs`
- In conditionals → `e2e_functions.rs`

**Untested With**:
- Return from zone scope (P1) - Lifetime escapes?
- Return CRDT values (P2)
- Return Persistent<T> (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Return value
To calculate_double (x: Int) -> Int:
    Return x * 2.
```

```logos
## Example 2: Early return
To process_value (x: Int):
    If x < 0:
        Return.
    Print x.
```

---

### Functions

#### Feature: FunctionDef (Function Definition)

**AST Node**: `Stmt::FunctionDef { name, params, body, return_type, is_native }`
**Category**: Functions
**Phase Introduced**: Phase 1

**Syntax**:
```logos
To calculate_area (width: Real) and (height: Real) -> Real:
    Return width * height.

To greet (name: Text):
    Print name.
```

**Semantics**: Defines a function with parameters and optional return type. If `body` contains async operations, generates `async fn`.

**Type Constraints**:
- `name`: Symbol (function identifier)
- `params`: Vec of (name, type) pairs
- `body`: Block of statements
- `return_type`: Optional return type
- `is_native`: Boolean (true for FFI declarations)

**Codegen Pattern**:
```rust
fn calculate_area(width: f64, height: f64) -> f64 {
    width * height
}

async fn fetch_data(url: String) -> String {
    // async operations
}
```

**Async Detection**: If `body` contains any async statement (Network, VFS, Sleep, Concurrent, etc.), the function is marked `async fn`.

**Triggers**:
- Async: Yes (if body contains async)
- VFS: Yes (if body uses VFS)
- Networking: Yes (if body uses networking)

**Tested With**:
- Basic functions → `e2e_functions.rs`
- Async functions → `e2e_async_cross_cutting.rs`
- Generic functions → `e2e_types.rs`
- Recursive functions → `e2e_functions.rs`

**Untested With**:
- Functions with CRDT parameters (P1)
- Functions with zone parameters (P1)
- Functions with ownership transfer (Give) (P0) - Critical gap
- Deep generic nesting (P0)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Simple function
To add (x: Int) and (y: Int) -> Int:
    Return x + y.

## Main:
Let result be add(5, 3).
```

```logos
## Example 2: Async function
To fetch_page (url: Text) -> Text:
    # Contains async operations, auto-detected
    Read content from file url.
    Return content.
```

```logos
## Example 3: Generic function
To first_element (items: Seq of T) -> T:
    Return item 1 of items.
```

---

#### Feature: Call (Function Call)

**AST Node**: `Stmt::Call { function, args }` (statement) or `Expr::Call { function, args }` (expression)
**Category**: Functions
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Call process with data.
Let result be calculate(x, y).
```

**Semantics**: Invokes a function with arguments. Can be used as a statement (side effects) or expression (return value).

**Type Constraints**:
- `function`: Symbol (function name)
- `args`: Vec of expressions matching parameter types

**Codegen Pattern**:
```rust
// Statement form
process(data);

// Expression form
let result = calculate(x, y);

// Async function
let result = fetch_data(url).await;
```

**Triggers**:
- Async: Yes (if callee is async)
- VFS: Yes (if callee uses VFS)
- Networking: Yes (if callee uses networking)

**Tested With**:
- Simple calls → `e2e_functions.rs`
- Async calls → `e2e_async_cross_cutting.rs`
- Chained calls → `e2e_functions.rs`

**Untested With**:
- Calling with CRDT arguments (P1)
- Calling with ownership transfer (Give) (P0)
- Calling from concurrent blocks (P1)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Statement call
To log_message (msg: Text):
    Print msg.

## Main:
Call log_message with "Hello".
```

```logos
## Example 2: Expression call
To square (x: Int) -> Int:
    Return x * x.

## Main:
Let result be square(5).
Show result to show.  # 25
```

---

### Data Structures

#### Feature: StructDef (Struct Definition)

**AST Node**: `Stmt::StructDef { name, fields, is_portable }`
**Category**: Data Structures
**Phase Introduced**: Phase 1

**Syntax**:
```logos
A Point has:
    x, which is Real.
    y, which is Real.

A portable User has:
    name, which is Text.
    age, which is Int.
```

**Semantics**: Defines a product type (struct) with named fields. If `portable`, derives `Serialize` and `Deserialize` for network transmission.

**Type Constraints**:
- `name`: Symbol (struct identifier)
- `fields`: Vec of (field_name, type_name, is_public)
- `is_portable`: Boolean (serialization support)

**Codegen Pattern**:
```rust
#[derive(Debug, Clone)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    name: String,
    age: i64,
}
```

**CRDT Field Detection**: If a field type is `Shared T`, it generates a CRDT wrapper (LWWRegister, GCounter, etc.).

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Simple structs → `e2e_structs.rs`
- Nested structs → `e2e_structs.rs`
- Generic structs → `e2e_types.rs`
- Portable structs → `e2e_structs.rs`

**Untested With**:
- Structs with CRDT fields (P1) - Composition
- Structs with Persistent<T> fields (P1)
- Structs with Zone-allocated fields (P2)
- Deep generic nesting (P0)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Simple struct
A Point has:
    x, which is Real.
    y, which is Real.

## Main:
Let p be a new Point with x 1.0 and y 2.0.
Show p's x to show.
```

```logos
## Example 2: Generic struct
A Box of T has:
    value, which is T.

## Main:
Let int_box be a new Box of Int with value 42.
```

```logos
## Example 3: Shared struct (CRDT)
A Counter has:
    Shared count, which is Nat.

## Main:
Let c be a new Counter.
Increase c's count by 10.
```

---

#### Feature: EnumDef (Enum Definition)

**AST Node**: `Stmt::EnumDef { name, variants }`
**Category**: Data Structures
**Phase Introduced**: Phase 1

**Syntax**:
```logos
A Shape is either:
    A Circle with radius (Real).
    A Rectangle with width (Real) and height (Real).
    A Point.

A Result of T and E is either:
    Ok with value (T).
    Err with error (E).
```

**Semantics**: Defines a sum type (tagged union) with named variants. Variants can be unit (no data) or struct-like (with named fields). Supports generics.

**Type Constraints**:
- `name`: Symbol (enum identifier)
- `variants`: Vec of (variant_name, fields)
- Each field has (name, type)

**Codegen Pattern**:
```rust
#[derive(Debug, Clone)]
enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
    Point,
}

// Generic enum
#[derive(Debug, Clone)]
enum Result<T, E> {
    Ok { value: T },
    Err { error: E },
}
```

**Recursive Detection**: If a variant contains the enum type itself (e.g., `Tree` has `left: Tree`), the field is automatically wrapped in `Box<T>` to break the infinite size cycle.

**Codegen Example (Recursive)**:
```rust
// A Tree is either: A Leaf. A Node with left (Tree) and right (Tree).
#[derive(Debug, Clone)]
enum Tree {
    Leaf,
    Node { left: Box<Tree>, right: Box<Tree> },
}
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Simple enums → `e2e_enums.rs:15`
- Recursive enums → `e2e_enums.rs:45`
- Generic enums → `e2e_types.rs:67`
- Enums in structs → `e2e_structs.rs:89`

**Untested With**:
- Enums with CRDT fields (P1) - Possible?
- Enums with Persistent<T> (P2) - Serialization
- Deep recursive nesting (P1) - Stack safety
- Generic enums with constraints (P2) - Type bounds

**Known Issues**:
- No support for tuple variants (only struct variants)
- No support for discriminant values
- No `derive` attributes (always Debug + Clone)

**Examples**:

```logos
## Example 1: Simple enum
A Color is either:
    Red.
    Green.
    Blue.
    Custom with r (Int), g (Int), b (Int).

## Main:
Let c be a new Custom with r 255, g 128, b 0.
Inspect c:
    When Custom (r: red) and (g: green) and (b: blue):
        Show red to show.
```

```logos
## Example 2: Recursive enum (binary tree)
A Tree is either:
    A Leaf with value (Int).
    A Node with left (Tree) and right (Tree).

## Main:
Let leaf1 be a new Leaf with value 1.
Let leaf2 be a new Leaf with value 2.
Let tree be a new Node with left leaf1 and right leaf2.

Inspect tree:
    When Node (left: l) and (right: r):
        Show "Binary node" to show.
    When Leaf (value: v):
        Show v to show.
```

```logos
## Example 3: Generic enum (Option)
An Option of T is either:
    Some with value (T).
    None.

## Main:
Let maybe_num be a new Some of Int with value 42.
Inspect maybe_num:
    When Some (value: n):
        Show n to show.
    When None:
        Show "Nothing" to show.
```

```logos
## Example 4: Enum with multiple fields
A Message is either:
    Text with content (Text).
    Data with payload (Seq of Byte) and encoding (Text).
    Event with timestamp (Int), event_type (Text), data (Text).

## Main:
Let msg be a new Text with content "Hello".
```

---

#### Feature: Inspect (Pattern Matching)

**AST Node**: `Stmt::Inspect { target, arms, has_otherwise }`
**Category**: Data Structures
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Inspect shape:
    When Circle (radius: r):
        Print r.
    When Rectangle (width: w) and (height: h):
        Print w * h.
    Otherwise:
        Print "Unknown".
```

**Semantics**: Pattern matches on enum variants, binding fields to variables.

**Type Constraints**:
- `target`: Expression evaluating to an enum type
- `arms`: Vec of match arms with patterns and bodies
- `has_otherwise`: Boolean (exhaustiveness check)

**Codegen Pattern**:
```rust
match shape {
    Shape::Circle { radius } => {
        let r = radius;
        println!("{}", r);
    }
    Shape::Rectangle { width, height } => {
        let w = width;
        let h = height;
        println!("{}", w * h);
    }
    _ => {
        println!("Unknown");
    }
}
```

**Boxed Deref**: If enum fields are boxed (e.g., recursive types), bindings are automatically dereferenced with `*`.

**Triggers**:
- Async: Yes (if arm bodies contain async)
- VFS: Yes (if arm bodies use VFS)
- Networking: Yes (if arm bodies use networking)

**Tested With**:
- Simple enums → `e2e_enums.rs`
- Nested enums → `e2e_enums.rs`
- Boxed enums → `e2e_enums.rs`

**Untested With**:
- Inspect with async in arms (P1) - Known bug: sleep in arm
- Inspect with CRDT enums (P2)
- Inspect with ownership transfer (P1)

**Known Issues**:
- ⚠️ Sleep in inspect arms causes compilation issues (see e2e_async_cross_cutting.rs)

**Examples**:

```logos
## Example 1: Basic matching
A Shape is either:
    A Circle with radius (Real).
    A Square with side (Real).

## Main:
Let s be a new Circle with radius 5.0.
Inspect s:
    When Circle (radius: r):
        Print r.
    When Square (side: s):
        Print s.
```

---

### Collections

#### Feature: Push (Append to Sequence)

**AST Node**: `Stmt::Push { value, collection }`
**Category**: Collections
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Push 3 to items.
Push "text" to messages.
```

**Semantics**: Appends a value to the end of a sequence (Vec). The collection must be mutable.

**Type Constraints**:
- `value`: Expression matching element type `T`
- `collection`: Expression evaluating to `Seq<T>` (Vec)

**Codegen Pattern**:
```rust
items.push(3);
messages.push("text".to_string());
```

**Mutability Detection**: The compiler automatically marks the collection as `mut` if it detects Push operations.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Push to list → `e2e_collections.rs:20`
- Push to empty → `e2e_collections.rs:113`
- Push in loops → `e2e_iteration.rs`

**Untested With**:
- Push in concurrent blocks (P1) - Race conditions
- Push to CRDT sequences (P1) - Different semantics (Append)
- Push generic types (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Basic push
Let items be [1, 2].
Push 3 to items.
Show items.  # [1, 2, 3]
```

```logos
## Example 2: Push in loop
Let numbers be a new Seq of Int.
Repeat for i from 1 to 5:
    Push i to numbers.
Show numbers.  # [1, 2, 3, 4, 5]
```

---

#### Feature: Pop (Remove from Sequence)

**AST Node**: `Stmt::Pop { collection, into }`
**Category**: Collections
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Pop from items.
Pop from items into x.
```

**Semantics**: Removes and optionally returns the last element from a sequence. Panics if empty.

**Type Constraints**:
- `collection`: Expression evaluating to `Seq<T>`
- `into`: Optional Symbol to bind the popped value

**Codegen Pattern**:
```rust
items.pop();  // Discards value
let x = items.pop().unwrap();  // Binds value
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Pop from list → `e2e_collections.rs:33`
- Pop into variable → `e2e_collections.rs:126`
- Length after pop → `e2e_collections.rs:205`

**Untested With**:
- Pop in concurrent blocks (P1)
- Pop with error handling (P2)
- Pop from generic sequences (P2)

**Known Issues**:
- No Option return - panics on empty (design choice)

**Examples**:

```logos
## Example 1: Discard value
Let items be [1, 2, 3].
Pop from items.
Show items.  # [1, 2]
```

```logos
## Example 2: Capture value
Let items be [1, 2, 3].
Pop from items into last.
Show last.  # 3
```

---

#### Feature: Add (Insert into Set)

**AST Node**: `Stmt::Add { value, collection }`
**Category**: Collections
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Add "item" to tags.
Add 42 to numbers.
```

**Semantics**: Inserts a value into a set. If already present, no effect (set semantics).

**Type Constraints**:
- `value`: Expression matching element type `T` where `T: Hash + Eq`
- `collection`: Expression evaluating to `Set<T>` (HashSet) or CRDT SharedSet

**Codegen Pattern**:
```rust
// Regular Set
tags.insert("item".to_string());

// CRDT SharedSet (ORSet)
tags.add("item".to_string());
```

**CRDT Detection**: If collection is a SharedSet field, generates `.add()` instead of `.insert()`.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Add to HashSet → `e2e_sets.rs`
- Add to SharedSet → `e2e_crdt.rs:273`
- Add duplicates → `e2e_crdt.rs:367`

**Untested With**:
- Add in concurrent blocks (P1)
- Add with ownership transfer (P1)
- Add generic types (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Add to regular set
Let tags be an empty Set of Text.
Add "important" to tags.
Add "urgent" to tags.
Show length of tags.  # 2
```

```logos
## Example 2: Add to CRDT SharedSet
A Party is Shared and has:
    guests, which is a SharedSet of Text.

## Main:
Let p be a new Party.
Add "Alice" to p's guests.
Add "Bob" to p's guests.
```

---

#### Feature: Remove (Delete from Set)

**AST Node**: `Stmt::Remove { value, collection }`
**Category**: Collections
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Remove "item" from tags.
Remove 42 from numbers.
```

**Semantics**: Removes a value from a set. If not present, no effect.

**Type Constraints**:
- `value`: Expression matching element type `T`
- `collection`: Expression evaluating to `Set<T>` or CRDT SharedSet

**Codegen Pattern**:
```rust
// Regular Set
tags.remove(&"item".to_string());

// CRDT SharedSet
tags.remove("item".to_string());
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Remove from HashSet → `e2e_sets.rs`
- Remove from SharedSet → `e2e_crdt.rs:328`
- Remove then check contains → `e2e_crdt.rs:347`

**Untested With**:
- Remove in concurrent blocks (P1)
- Remove with complex expressions (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Remove from set
Let items be a Set containing "sword", "shield", "potion".
Remove "shield" from items.
Show length of items.  # 2
```

---

#### Expression: Index (Element Access)

**AST Node**: `Expr::Index { collection, index }`
**Category**: Collections (Expression)
**Phase Introduced**: Phase 1

**Syntax**:
```logos
item 2 of items
items at i
```

**Semantics**: Accesses an element at a specific index. LOGOS uses 1-based indexing. Panics if out of bounds.

**Type Constraints**:
- `collection`: Expression evaluating to `Seq<T>`
- `index`: Expression evaluating to `Int` (i64)
- Returns: `T`

**Codegen Pattern**:
```rust
// 1-based → 0-based conversion
items.logos_get(2i64)  // Expands to: items[(2 - 1) as usize]
```

**1-Based Indexing**: All LOGOS collections use 1-based indexing to match natural language.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Index access → `e2e_collections.rs:59`
- Last element → `e2e_collections.rs:140`
- Variable index → `e2e_collections.rs:152`

**Untested With**:
- Index with async expressions (P1)
- Index on generic sequences (P2)
- Bounds checking with refinements (P1)

**Known Issues**:
- Index 0 panics (design choice - 1-based only)
- No compile-time bounds checking

**Examples**:

```logos
## Example 1: Basic indexing
Let items be [10, 20, 30].
Let first be item 1 of items.
Show first.  # 10
```

```logos
## Example 2: Dynamic index
Let items be [100, 200, 300].
Let i be 2.
Show item i of items.  # 200
```

---

#### Expression: Slice (Range Access)

**AST Node**: `Expr::Slice { collection, start, end }`
**Category**: Collections (Expression)
**Phase Introduced**: Phase 1

**Syntax**:
```logos
items 2 through 4
data 1 through (length of data)
```

**Semantics**: Extracts a sub-sequence from start to end (inclusive, 1-based).

**Type Constraints**:
- `collection`: Expression evaluating to `Seq<T>`
- `start`: Expression evaluating to `Int`
- `end`: Expression evaluating to `Int`
- Returns: `Seq<T>` (new Vec)

**Codegen Pattern**:
```rust
// 1-based inclusive → 0-based exclusive
items.logos_slice(2i64, 4i64)
// Expands to: items[(2-1) as usize..=(4-1) as usize].to_vec()
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Slice range → `e2e_collections.rs:72`
- Full list slice → `e2e_collections.rs:165`
- Single element slice → `e2e_collections.rs:178`

**Untested With**:
- Slice with async expressions (P1)
- Slice on generic types (P2)
- Empty slices (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Middle elements
Let items be [1, 2, 3, 4, 5].
Let middle be items 2 through 4.
Show middle.  # [2, 3, 4]
```

```logos
## Example 2: Single element
Let items be [1, 2, 3, 4, 5].
Let single be items 3 through 3.
Show single.  # [3]
```

---

#### Expression: Length (Collection Size)

**AST Node**: `Expr::Length { collection }`
**Category**: Collections (Expression)
**Phase Introduced**: Phase 1

**Syntax**:
```logos
length of items
length of (items 1 through 10)
```

**Semantics**: Returns the number of elements in a collection.

**Type Constraints**:
- `collection`: Expression evaluating to `Seq<T>`, `Set<T>`, or `Map<K,V>`
- Returns: `Int` (i64)

**Codegen Pattern**:
```rust
items.len() as i64
```

**Works With**: Seq, Set, Map, and all CRDT collection types.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Length of list → `e2e_collections.rs:46`
- Length after push → `e2e_collections.rs:191`
- Length after pop → `e2e_collections.rs:205`
- Length of CRDT → `e2e_crdt.rs:283`

**Untested With**:
- Length in refinement types (P1)
- Length with zones (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: List length
Let items be [1, 2, 3, 4, 5].
Let n be length of items.
Show n.  # 5
```

```logos
## Example 2: After operations
Let items be [1, 2].
Push 3 to items.
Push 4 to items.
Show length of items.  # 4
```

---

#### Expression: Copy (Deep Clone)

**AST Node**: `Expr::Copy { expr }`
**Category**: Collections (Expression)
**Phase Introduced**: Phase 1

**Syntax**:
```logos
copy of items
copy of (items 1 through 5)
```

**Semantics**: Creates an independent deep copy of a value. Mutations to the copy don't affect the original.

**Type Constraints**:
- `expr`: Expression of any cloneable type
- Returns: Same type as input

**Codegen Pattern**:
```rust
items.clone()
```

**Use Cases**: Creating independent collections, avoiding ownership issues, defensive copying.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Copy of list → `e2e_collections.rs:85`
- Independent mutation → `e2e_collections.rs:85`

**Untested With**:
- Copy of CRDTs (P1) - May need special handling
- Copy with ownership semantics (P1)
- Copy in concurrent blocks (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Independent copy
Let original be [1, 2, 3].
Let cloned be copy of original.
Push 4 to original.
Show cloned.  # [1, 2, 3] (unchanged)
```

---

#### Expression: Contains (Membership Test)

**AST Node**: `Expr::Contains { collection, value }`
**Category**: Collections (Expression)
**Phase Introduced**: Phase 1

**Syntax**:
```logos
set contains "item"
"item" in set
```

**Semantics**: Tests whether a value is present in a collection.

**Type Constraints**:
- `collection`: Expression evaluating to `Seq<T>`, `Set<T>`, or `Text`
- `value`: Expression of type `T` (or `Text`/`Char` for strings)
- Returns: `Bool`

**Codegen Pattern**:
```rust
// Set
set.contains(&"item".to_string())

// Seq (linear search)
seq.contains(&value)

// Text (substring)
text.contains("substring")
text.contains('c')
```

**String Semantics**: For Text, tests substring or character membership.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Set contains → `e2e_sets.rs`
- CRDT contains → `e2e_crdt.rs:290`
- String contains → `e2e_primitives.rs`

**Untested With**:
- Contains in concurrent blocks (P2)
- Contains with complex expressions (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Set membership
Let tags be a Set containing "important", "urgent".
If tags contains "important":
    Show "found".
```

```logos
## Example 2: String contains
Let text be "Hello World".
If text contains "World":
    Show "match".
```

---

#### Expression: Union (Set Union)

**AST Node**: `Expr::Union { left, right }`
**Category**: Collections (Expression)
**Phase Introduced**: Phase 1

**Syntax**:
```logos
a union b
tags union categories
```

**Semantics**: Returns a new set containing all elements from both sets.

**Type Constraints**:
- `left`: Expression evaluating to `Set<T>`
- `right`: Expression evaluating to `Set<T>`
- Returns: `Set<T>`

**Codegen Pattern**:
```rust
{
    let mut result = left.clone();
    result.extend(right.iter().cloned());
    result
}
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Union of sets → `e2e_sets.rs`

**Untested With**:
- Union with CRDT sets (P1)
- Union in expressions (P2)
- Union with large sets (P3)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Combine sets
Let a be a Set containing "red", "blue".
Let b be a Set containing "blue", "green".
Let combined be a union b.
# combined contains "red", "blue", "green"
```

---

#### Expression: Intersection (Set Intersection)

**AST Node**: `Expr::Intersection { left, right }`
**Category**: Collections (Expression)
**Phase Introduced**: Phase 1

**Syntax**:
```logos
a intersection b
tags intersection allowed
```

**Semantics**: Returns a new set containing only elements present in both sets.

**Type Constraints**:
- `left`: Expression evaluating to `Set<T>`
- `right`: Expression evaluating to `Set<T>`
- Returns: `Set<T>`

**Codegen Pattern**:
```rust
left.intersection(&right).cloned().collect()
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Intersection of sets → `e2e_sets.rs`

**Untested With**:
- Intersection with CRDTs (P1)
- Intersection in refinements (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Find common elements
Let a be a Set containing "red", "blue", "green".
Let b be a Set containing "blue", "green", "yellow".
Let common be a intersection b.
# common contains "blue", "green"
```

---

### Ownership

#### Feature: Give (Ownership Transfer)

**AST Node**: `Stmt::Give { object, recipient }`
**Category**: Ownership
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Give data to process.
```

**Semantics**: Transfers ownership of `object` to `recipient`. Compiles to move semantics in Rust. The original variable becomes invalid after the Give.

**Type Constraints**:
- `object`: Expression (any owned value)
- `recipient`: Expression (function or variable accepting ownership)

**Codegen Pattern**:
```rust
process(data);  // Move
```

**Triggers**:
- Async: No (unless recipient is async)
- VFS: No
- Networking: No

**Tested With**:
- Basic ownership transfer → `e2e_variables.rs`

**Untested With**:
- Give through function parameters (P0) - Critical gap
- Give into Launch tasks (P1) - Async ownership
- Give with CRDTs (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Give to function
To consume (data: Text):
    Print data.

## Main:
Let msg be "Hello".
Give msg to consume.
# msg is now invalid
```

---

#### Feature: Show (Immutable Borrow)

**AST Node**: `Stmt::Show { object, recipient }`
**Category**: Ownership
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Show data to display.
Show value to process.
```

**Semantics**: Passes an immutable reference (`&T`) to `recipient`. The original variable remains valid and can be used again. Compiles to immutable borrow in Rust.

**Type Constraints**:
- `object`: Expression (any value)
- `recipient`: Expression (function or variable accepting `&T`)

**Codegen Pattern**:
```rust
display(&data);  // Immutable borrow
```

**Triggers**:
- Async: No (unless recipient is async)
- VFS: No
- Networking: No

**Tested With**:
- Basic borrowing → `e2e_variables.rs`
- Multiple shows → `e2e_ownership.rs`
- Show in loops → `e2e_iteration.rs`
- Show with async → `e2e_async_cross_cutting.rs`

**Untested With**:
- Show through function parameters (P1) - Reference relay
- Show with mutable data (P2) - Clarify semantics
- Show with CRDTs (P2) - Distributed state borrowing

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Show preserves ownership
To display (data: &Text):
    Show data to show.

## Main:
Let msg be "Hello".
Show msg to display.
Show msg to display.  # msg still valid, can use again
```

```logos
## Example 2: Show vs Give
To consume (data: Text):
    Show data to show.

To borrow (data: &Text):
    Show data to show.

## Main:
Let x be "test".
Show x to borrow.  # x still valid
Show x to borrow.  # OK, can borrow multiple times
Give x to consume.  # x moved, now invalid
# Show x to borrow.  # ERROR: x was moved
```

```logos
## Example 3: Show in loop
Let numbers be [1, 2, 3, 4, 5].

Repeat for n in numbers:
    Show n to show.  # Borrow each element
# numbers still valid after loop
```

```logos
## Example 4: Show with structs
A Point has:
    x, which is Real.
    y, which is Real.

To calculate_distance (p: &Point) returns Real:
    Return sqrt of ((p's x times p's x) plus (p's y times p's y)).

## Main:
Let origin be a new Point with x: 0.0, y: 0.0.
Let distance be Show origin to calculate_distance.
Show origin to show.  # origin still valid
```

---

### Memory Management

#### Feature: Zone (Memory Arena)

**AST Node**: `Stmt::Zone { name, capacity, source_file, body }`
**Category**: Memory
**Phase Introduced**: Phase 85

**Syntax**:
```logos
Inside a new zone called 'Scratch':
    # Allocations here

Inside a zone called 'Buffer' of size 1 MB:
    # Pre-sized arena

Inside a zone called 'Data' mapped from 'file.bin':
    # Memory-mapped file
```

**Semantics**: Creates a memory arena for bulk allocation and deallocation. Heap zones use bump allocation. Mapped zones use mmap for zero-copy file access.

**Type Constraints**:
- `name`: Symbol (arena identifier)
- `capacity`: Optional size in bytes (heap zones)
- `source_file`: Optional file path (mapped zones)
- `body`: Block of statements

**Codegen Pattern**:
```rust
{
    let Scratch = Zone::new_heap(1024 * 1024);
    // body
    // Zone dropped here, all allocations freed
}
```

**Hotel California Rule**: Values allocated in a zone cannot escape to outer scopes (lifetime constraints).

**Triggers**:
- Async: Yes (if body contains async)
- VFS: Yes (if mapped zone)
- Networking: No

**Tested With**:
- Heap zones → `e2e_zones.rs`
- Mapped zones → `e2e_zones.rs`
- File sipping → `phase85_zones.rs`

**Untested With**:
- Zone + Return escape (P1) - Lifetime safety
- Zone + CRDT (P2)
- Zone + Concurrent (P2)

**Known Issues**:
- None currently

---

### Concurrency

#### Feature: Concurrent (Async Block)

**AST Node**: `Stmt::Concurrent { tasks }`
**Category**: Concurrency
**Phase Introduced**: Phase 9

**Syntax**:
```logos
Attempt all of the following:
    Fetch from url1.
    Fetch from url2.
```

**Semantics**: Executes tasks concurrently using `tokio::join!`. Used for I/O-bound parallelism.

**Type Constraints**:
- `tasks`: Block of statements

**Codegen Pattern**:
```rust
tokio::join!(
    async { /* task 1 */ },
    async { /* task 2 */ },
);
```

**Triggers**:
- Async: Yes (always)
- VFS: Yes (if tasks use VFS)
- Networking: Yes (if tasks use networking)

**Tested With**:
- Network I/O → `e2e_async_cross_cutting.rs`
- Multiple async operations → `e2e_concurrency.rs`

**Untested With**:
- Concurrent + CRDT mutations (P1)
- Concurrent + ownership transfer (P1)
- Concurrent + Inspect (P1) - Known bug

**Known Issues**:
- ⚠️ Inspect inside Concurrent blocks has issues (sleep in arms)

---

### CRDTs

CRDT (Conflict-free Replicated Data Type) operations enable distributed data synchronization without conflicts.

#### Feature: Increase (GCounter Increment)

**AST Node**: `Stmt::IncreaseCrdt { object, field, amount }`
**Category**: CRDTs
**Phase Introduced**: Phase 49

**Syntax**:
```logos
Increase counter's points by 10.
Increase game's score by amount.
```

**Semantics**: Increments a GCounter (grow-only counter) field. The field must be of type ConvergentCount or GCounter.

**Type Constraints**:
- `object`: Expression evaluating to a struct with CRDT field
- `field`: Symbol (field name, must be GCounter type)
- `amount`: Expression evaluating to `Nat` or `Int`

**Codegen Pattern**:
```rust
counter.points.increment(10);
game.score.increment(amount as u64);
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- GCounter increment → `e2e_crdt.rs:18`
- By variable → `e2e_crdt.rs:34`
- Multiple increments → `e2e_crdt.rs:51`

**Untested With**:
- Increase in concurrent blocks (P1) - Race conditions
- Increase with async expressions (P2)
- Increase with refinement types (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Basic increment
A Counter is Shared and has:
    points: ConvergentCount.

## Main:
Let c be a new Counter.
Increase c's points by 10.
Show c's points.  # 10
```

```logos
## Example 2: Variable amount
A Game is Shared and has:
    score: ConvergentCount.

## Main:
Let g be a new Game.
Let bonus be 50.
Increase g's score by bonus.
```

---

#### Feature: Decrease (PNCounter Decrement)

**AST Node**: `Stmt::DecreaseCrdt { object, field, amount }`
**Category**: CRDTs
**Phase Introduced**: Phase 49

**Syntax**:
```logos
Decrease game's score by 5.
Decrease balance's amount by penalty.
```

**Semantics**: Decrements a PNCounter (positive-negative counter) field, also known as Tally.

**Type Constraints**:
- `object`: Expression evaluating to a struct with PNCounter field
- `field`: Symbol (field name, must be PNCounter/Tally type)
- `amount`: Expression evaluating to `Nat` or `Int`

**Codegen Pattern**:
```rust
game.score.decrement(5);
balance.amount.decrement(penalty as u64);
```

**Can Go Negative**: Unlike GCounter, PNCounter supports negative values.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- PNCounter decrement → `e2e_crdt.rs:198`
- Decrease to negative → `e2e_crdt.rs:215`
- Multiple operations → `e2e_crdt.rs:232`
- Decrease only → `e2e_crdt.rs:251`

**Untested With**:
- Decrease in concurrent blocks (P1)
- Decrease with CRDTs in generics (P1)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Decrement counter
A Game is Shared and has:
    score, which is a Tally.

## Main:
Let g be a new Game.
Increase g's score by 100.
Decrease g's score by 30.
Show g's score.  # 70
```

```logos
## Example 2: Negative balance
A Temperature is Shared and has:
    degrees, which is a Tally.

## Main:
Let t be a new Temperature.
Increase t's degrees by 10.
Decrease t's degrees by 25.
Show t's degrees.  # -15
```

---

#### Feature: Append (RGA Sequence Append)

**AST Node**: `Stmt::AppendToSequence { sequence, value }`
**Category**: CRDTs
**Phase Introduced**: Phase 49

**Syntax**:
```logos
Append "Line 1" to doc's lines.
Append item to log's entries.
```

**Semantics**: Appends a value to a SharedSequence (RGA - Replicated Growable Array). Unlike Push (which is for regular Seq), this is for CRDT sequences.

**Type Constraints**:
- `sequence`: Expression evaluating to a field of type SharedSequence<T> or RGA<T>
- `value`: Expression of type `T`

**Codegen Pattern**:
```rust
doc.lines.append("Line 1".to_string());
log.entries.append(item);
```

**Vs Push**: Use Append for CRDT SharedSequence, Push for regular Seq.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Append to SharedSequence → `e2e_crdt.rs:389`
- Empty sequence → `e2e_crdt.rs:407`

**Untested With**:
- Append in concurrent blocks (P1)
- Append with ownership transfer (P1)
- Append generic types (P1)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Build log
A Document is Shared and has:
    lines, which is a SharedSequence of Text.

## Main:
Let doc be a new Document.
Append "Line 1" to doc's lines.
Append "Line 2" to doc's lines.
Append "Line 3" to doc's lines.
Show length of doc's lines.  # 3
```

---

#### Feature: Resolve (MVRegister Conflict Resolution)

**AST Node**: `Stmt::ResolveConflict { object, field, value }`
**Category**: CRDTs
**Phase Introduced**: Phase 49

**Syntax**:
```logos
Resolve page's title to "Final".
Resolve config's value to selected.
```

**Semantics**: Resolves conflicts in an MVRegister (Multi-Value Register / Divergent) by selecting a winning value.

**Type Constraints**:
- `object`: Expression evaluating to a struct with MVRegister field
- `field`: Symbol (field name, must be Divergent<T> or MVRegister<T>)
- `value`: Expression of type `T`

**Codegen Pattern**:
```rust
page.title.resolve("Final".to_string());
config.value.resolve(selected);
```

**Conflict Detection**: MVRegister preserves all concurrent writes. Resolve picks one.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Resolve conflicts → `e2e_crdt.rs:460`

**Untested With**:
- Resolve in concurrent blocks (P1) - More conflicts
- Resolve with has_conflict check (P1)
- Resolve generic types (P2)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Resolve conflict
A WikiPage is Shared and has:
    title, which is a Divergent Text.

## Main:
Let page be a new WikiPage.
Set page's title to "Draft".
Set page's title to "Conflicted".
Resolve page's title to "Final".
Show page's title.  # "Final"
```

---

#### Feature: Merge (CRDT State Merge)

**AST Node**: `Stmt::MergeCrdt { source, target }`
**Category**: CRDTs
**Phase Introduced**: Phase 49

**Syntax**:
```logos
Merge remote into local.
Merge remote's field into local's field.
```

**Semantics**: Merges CRDT state from source into target. This is the core operation for distributed synchronization.

**Type Constraints**:
- `source`: Expression evaluating to a CRDT type (must implement `Merge` trait)
- `target`: Expression evaluating to same CRDT type

**Codegen Pattern**:
```rust
// Struct-level merge
local.merge(&remote);

// Field-level merge
local.field.merge(&remote.field);
```

**Commutativity**: Merge is commutative, associative, and idempotent (CRDTs guarantee).

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Struct-level merge → `e2e_crdt.rs:125`
- Field-level merge → `e2e_crdt.rs:141`
- GCounter merge → `e2e_crdt.rs:175`
- PNCounter merge → `e2e_crdt.rs:527`
- SharedSet merge → `e2e_crdt.rs:545`
- Divergent merge → `e2e_crdt.rs:563`

**Untested With**:
- Merge in concurrent blocks (P0) - Critical gap
- Merge with generics (P1)
- Merge with Persistent<T> (P1)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Struct merge
A Counter is Shared and has:
    points, which is ConvergentCount.

## Main:
Let local be a new Counter.
Let remote be a new Counter.
Increase local's points by 10.
Increase remote's points by 5.
Merge remote into local.
# local.points now contains 15 (10 + 5)
```

```logos
## Example 2: Field merge
A Profile is Shared and has:
    active, which is LastWriteWins of Bool.

## Main:
Let local be a new Profile.
Let remote be a new Profile.
Merge remote's active into local's active.
```

---

#### Feature: Sync (Network Synchronization)

**AST Node**: `Stmt::Sync { var, topic }`
**Category**: CRDTs / Networking
**Phase Introduced**: Phase 52

**Syntax**:
```logos
Sync counter on "game-scores".
Sync state on topic_name.
```

**Semantics**: Subscribes a CRDT variable to a GossipSub topic. Automatically publishes local mutations and merges incoming remote updates.

**Type Constraints**:
- `var`: Symbol (variable name, must be a CRDT type)
- `topic`: Expression evaluating to `Text`

**Codegen Pattern**:
```rust
// If ONLY Sync (no Mount), generates Synced<T>
let counter = Synced::new(GCounter::new(), "game-scores").await;

// If BOTH Mount AND Sync, generates Distributed<T> (see Mount+Sync Detection)
```

**Auto-Merge**: Background task automatically merges received updates.

**Triggers**:
- Async: Yes (network operations)
- VFS: No (unless also Mounted)
- Networking: Yes (libp2p GossipSub)

**Tested With**:
- Sync CRDT → `e2e_gossip.rs`

**Untested With**:
- Sync in concurrent blocks (P1)
- Sync with generics (P1)
- Sync + Mount combination (P1) - Should generate Distributed<T>

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Synced counter
Let counter be a new GCounter.
Sync counter on "player-scores".
Increase counter by 100.
# Mutation is automatically broadcast to other replicas
```

---

#### Feature: Mount (Persistent Storage)

**AST Node**: `Stmt::Mount { var, path }`
**Category**: CRDTs / I/O
**Phase Introduced**: Phase 53

**Syntax**:
```logos
Mount counter at "data/counter.journal".
Mount state at file_path.
```

**Semantics**: Loads or creates a CRDT from a journal file. Provides crash-resilient persistence via append-only log.

**Type Constraints**:
- `var`: Symbol (variable name, must be a CRDT type)
- `path`: Expression evaluating to `Text`

**Codegen Pattern**:
```rust
// If ONLY Mount (no Sync), generates Persistent<T>
let counter = Persistent::<GCounter>::mount(vfs.clone(), "data/counter.journal").await?;

// If BOTH Mount AND Sync, generates Distributed<T>
let counter = Distributed::<GCounter>::mount(
    vfs.clone(),
    "data/counter.journal",
    Some("game-scores".into())
).await?;
```

**Journal Format**: CRC32-checksummed append-only log with snapshots and deltas.

**Triggers**:
- Async: Yes (file I/O)
- VFS: Yes (file operations)
- Networking: No (unless also Synced)

**Tested With**:
- Mount CRDT → integration tests

**Untested With**:
- Mount in concurrent blocks (P1)
- Mount with generics (P1)
- Mount + Sync combination (P1)

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Persistent counter
Let counter be a new GCounter.
Mount counter at "data/visits.journal".
Increase counter by 1.
# State is persisted to disk
```

```logos
## Example 2: Distributed (Mount + Sync)
Let counter be a new GCounter.
Mount counter at "data/counter.journal".
Sync counter on "game-scores".
# State is both persisted AND synchronized
```

---

### Concurrency (Additional Features)

(See Concurrent block documented earlier. Adding remaining concurrency primitives:)

#### Feature: LaunchTask (Fire-and-Forget Task)

**AST Node**: `Stmt::LaunchTask { body }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Launch a task to fetch_data.
Launch a task:
    Sleep 1000.
    Show "Done" to show.
```

**Semantics**: Spawns an async task that runs concurrently without blocking. The task runs independently and cannot be awaited or stopped. Use `LaunchTaskWithHandle` if you need to await completion or stop the task.

**Type Constraints**:
- `body`: Block of statements (becomes async task body)
- Task must be `Send` (can cross thread boundaries)

**Codegen Pattern**:
```rust
tokio::spawn(async move {
    // task body
});
```

**Triggers**:
- Async: Yes (always)
- VFS: Depends on body
- Networking: Depends on body

**Tested With**:
- Basic launch → `e2e_concurrency.rs:15`
- Launch with captures → `e2e_concurrency.rs:45`
- Multiple launches → `e2e_concurrency.rs:78`
- Launch calling async functions → `e2e_concurrency.rs:102`

**Untested With**:
- Launch + Give (P1) - Ownership transfer unclear
- Launch + Zone (P2) - Lifetime conflicts

**Known Issues**:
- Tasks cannot be cancelled without handle
- No way to detect task completion without handle
- Error handling requires explicit Try/Result

**Examples**:

```logos
## Example 1: Fire-and-forget logging
Launch a task:
    Sleep 5000.
    Show "5 seconds elapsed" to show.
```

```logos
## Example 2: Multiple concurrent tasks
Launch a task:
    Call process_data with batch1.

Launch a task:
    Call process_data with batch2.

## Both tasks run concurrently
```

```logos
## Example 3: Launch calling async function
To async_operation:
    Sleep 1000.
    Return 42.

## Main:
Launch a task:
    Let result be Call async_operation.
    Show result to show.
```

---

#### Feature: LaunchWithHandle (Managed Task)

**AST Node**: `Stmt::LaunchTaskWithHandle { var, body }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Let handle be Launch a task to compute.
Let result be StopTask handle.
```

**Semantics**: Spawns an async task and binds a handle that can be used to await completion or stop the task. The handle is a `TaskHandle<T>` where `T` is the task's return type.

**Type Constraints**:
- `var`: Symbol (handle variable)
- `body`: Block of statements with optional Return
- Return type must be `Send`

**Codegen Pattern**:
```rust
let handle = tokio::spawn(async move {
    // task body
    return_value
});
```

**Triggers**:
- Async: Yes (always)
- VFS: Depends on body
- Networking: Depends on body

**Tested With**:
- Basic handle → `e2e_concurrency.rs:125`
- Await handle → `e2e_concurrency.rs:156`
- Stop task → `e2e_concurrency.rs:178`
- Return value → `e2e_concurrency.rs:201`

**Untested With**:
- Generic return types (P1) - Type inference
- CRDT return types (P2) - Distributed + async

**Examples**:

```logos
## Example 1: Await task completion
To compute_sum (n: Int) returns Int:
    Sleep 100.
    Return n + n.

## Main:
Let handle be Launch a task to compute_sum with 21.
Let result be Stop task handle.
Show result to show.  # 42
```

```logos
## Example 2: Concurrent computations
Let h1 be Launch a task to expensive_operation with data1.
Let h2 be Launch a task to expensive_operation with data2.

Let r1 be Stop task h1.
Let r2 be Stop task h2.

Show r1 combined with r2 to show.
```

---

#### Feature: CreatePipe (Channel Creation)

**AST Node**: `Stmt::CreatePipe { var }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Create a pipe called channel.
```

**Semantics**: Creates an unbounded MPMC (multi-producer, multi-consumer) channel for passing messages between tasks. The pipe is a `Pipe<T>` where `T` is inferred from usage.

**Type Constraints**:
- `var`: Symbol (pipe variable)
- Message type `T` must be `Send`

**Codegen Pattern**:
```rust
let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
let channel = Pipe { tx: tx.clone(), rx: Arc::new(Mutex::new(rx)) };
```

**Triggers**:
- Async: No (creation is sync, usage is async)
- VFS: No
- Networking: No

**Tested With**:
- Basic pipe → `e2e_concurrency.rs:234`
- Send/Receive → `e2e_concurrency.rs:267`
- Multiple senders → `e2e_concurrency.rs:289`
- Multiple receivers → `e2e_concurrency.rs:312`

**Untested With**:
- Generic message types (P1) - Type inference
- Struct message types (P0) - Common pattern
- CRDT message types (P2) - Distributed

**Examples**:

```logos
## Example 1: Producer-consumer pattern
Create a pipe called jobs.

Launch a task:
    Send 1 into jobs.
    Send 2 into jobs.
    Send 3 into jobs.

Launch a task:
    Receive x from jobs.
    Show x to show.  # 1
    Receive y from jobs.
    Show y to show.  # 2
```

```logos
## Example 2: Task coordination
Create a pipe called results.

Let h be Launch a task:
    Let sum be 10 + 20.
    Send sum into results.
    Return nothing.

Receive answer from results.
Show answer to show.  # 30
```

---

#### Feature: SendPipe (Channel Send)

**AST Node**: `Stmt::SendPipe { value, pipe }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Send 42 into channel.
Send message into pipe.
```

**Semantics**: Sends a value through a pipe (channel). Blocks (asynchronously) if the receiver is not ready. The value is moved (ownership transferred).

**Type Constraints**:
- `value`: Expression of type `T`
- `pipe`: Expression of type `Pipe<T>`
- `T` must be `Send`

**Codegen Pattern**:
```rust
pipe.tx.send(value).await?;
```

**Triggers**:
- Async: Yes (always)
- VFS: No
- Networking: No

**Tested With**:
- Basic send → `e2e_concurrency.rs:345`
- Send in loop → `e2e_concurrency.rs:378`
- Send struct → `e2e_concurrency.rs:401`
- Send multiple → `e2e_concurrency.rs:423`

**Untested With**:
- Send + Give (P1) - Ownership semantics
- Send CRDT (P2) - Distributed state
- Send in Concurrent block (P1) - Race conditions

**Examples**:

```logos
## Example 1: Send primitive
Create a pipe called nums.
Send 100 into nums.
```

```logos
## Example 2: Send struct
A Message has:
    content, which is Text.
    priority, which is Int.

Create a pipe called messages.
Let msg be a new Message with content "urgent" and priority 10.
Send msg into messages.
```

---

#### Feature: ReceivePipe (Channel Receive)

**AST Node**: `Stmt::ReceivePipe { var, pipe }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Receive x from channel.
```

**Semantics**: Receives a value from a pipe (channel), blocking (asynchronously) until a value is available. Ownership of the value is transferred to the receiver.

**Type Constraints**:
- `var`: Symbol (variable to bind received value)
- `pipe`: Expression of type `Pipe<T>`
- Result type is `T`

**Codegen Pattern**:
```rust
let x = pipe.rx.lock().await.recv().await.ok_or("channel closed")?;
```

**Triggers**:
- Async: Yes (always)
- VFS: No
- Networking: No

**Tested With**:
- Basic receive → `e2e_concurrency.rs:456`
- Receive in loop → `e2e_concurrency.rs:489`
- Multiple receivers → `e2e_concurrency.rs:512`
- Receive with pattern matching → `e2e_concurrency.rs:534`

**Untested With**:
- Receive + pattern matching (P1) - Ergonomics
- Receive timeout (P2) - Not implemented
- Receive in Concurrent (P1) - Complex interaction

**Examples**:

```logos
## Example 1: Consumer loop
Create a pipe called work.

## Producer
Launch a task:
    Repeat for i in [1, 2, 3, 4, 5]:
        Send i into work.

## Consumer
Repeat for _ in [1, 2, 3, 4, 5]:
    Receive job from work.
    Show job to show.
```

```logos
## Example 2: Receive struct
A Task has:
    id, which is Int.
    data, which is Text.

Create a pipe called tasks.

Send (a new Task with id 1 and data "first") into tasks.

Receive task from tasks.
Show task's id to show.  # 1
```

---

#### Feature: TrySendPipe (Non-Blocking Send)

**AST Node**: `Stmt::TrySendPipe { value, pipe, var }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Try to send 42 into channel storing result in success.
```

**Semantics**: Attempts to send a value through a pipe without blocking. Returns a boolean indicating success. Use this when you want to avoid blocking on a full channel.

**Type Constraints**:
- `value`: Expression of type `T`
- `pipe`: Expression of type `Pipe<T>`
- `var`: Symbol (boolean result variable)

**Codegen Pattern**:
```rust
let success = pipe.tx.try_send(value).is_ok();
```

**Triggers**:
- Async: No (non-blocking)
- VFS: No
- Networking: No

**Tested With**:
- Basic try send → `e2e_concurrency.rs:567`
- Try send loop → `e2e_concurrency.rs:589`
- Backpressure handling → `e2e_concurrency.rs:612`

**Untested With**:
- Try send + error handling (P2) - Pattern unclear
- Try send in hot loop (P2) - Performance implications

**Examples**:

```logos
## Example: Backpressure handling
Create a pipe called buffer.

Let counter be 0.
Repeat for i in [1, 2, 3]:
    Try to send i into buffer storing result in sent.
    If sent:
        Set counter to counter + 1.
    Otherwise:
        Show "Buffer full" to show.

Show counter to show.  # Number of successful sends
```

---

#### Feature: TryReceivePipe (Non-Blocking Receive)

**AST Node**: `Stmt::TryReceivePipe { var, pipe, result_var }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Try to receive x from channel storing result in got_value.
```

**Semantics**: Attempts to receive a value from a pipe without blocking. Returns a boolean indicating whether a value was received. If `true`, the variable is bound to the received value.

**Type Constraints**:
- `var`: Symbol (variable to bind if successful)
- `pipe`: Expression of type `Pipe<T>`
- `result_var`: Symbol (boolean result variable)

**Codegen Pattern**:
```rust
let (x, got_value) = match pipe.rx.lock().await.try_recv() {
    Ok(v) => (v, true),
    Err(_) => (Default::default(), false),
};
```

**Triggers**:
- Async: No (non-blocking)
- VFS: No
- Networking: No

**Tested With**:
- Basic try receive → `e2e_concurrency.rs:634`
- Polling loop → `e2e_concurrency.rs:656`
- Empty channel → `e2e_concurrency.rs:678`

**Untested With**:
- Try receive + Option (P2) - Better semantics?
- Try receive in hot loop (P2) - Performance

**Examples**:

```logos
## Example: Polling pattern
Create a pipe called events.

Launch a task:
    Sleep 1000.
    Send "event" into events.

Let received be false.
While not received:
    Try to receive evt from events storing result in received.
    If not received:
        Show "Waiting..." to show.
        Sleep 100.

Show evt to show.  # "event"
```

---

#### Feature: StopTask (Await Task Handle)

**AST Node**: `Stmt::StopTask { handle, var }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Stop task handle.
Let result be Stop task handle.
```

**Semantics**: Awaits completion of a task launched with handle, blocking until the task finishes. Returns the task's return value (or `nothing` if no return).

**Type Constraints**:
- `handle`: Expression of type `TaskHandle<T>`
- `var`: Optional symbol (variable to bind result)
- Result type is `T`

**Codegen Pattern**:
```rust
let result = handle.await??;
```

**Triggers**:
- Async: Yes (awaits task)
- VFS: No
- Networking: No

**Tested With**:
- Basic stop → `e2e_concurrency.rs:701`
- Stop with return value → `e2e_concurrency.rs:723`
- Stop multiple handles → `e2e_concurrency.rs:745`
- Stop in sequence → `e2e_concurrency.rs:767`

**Untested With**:
- Stop + error propagation (P2) - Task panic handling
- Stop in Concurrent (P1) - Nested await

**Examples**:

```logos
## Example 1: Sequential task execution
To step1 returns Int:
    Sleep 100.
    Return 10.

To step2 (x: Int) returns Int:
    Sleep 100.
    Return x + 5.

## Main:
Let h1 be Launch a task to step1.
Let r1 be Stop task h1.
Let h2 be Launch a task to step2 with r1.
Let r2 be Stop task h2.
Show r2 to show.  # 15
```

---

#### Feature: Select (Multi-Channel Wait)

**AST Node**: `Stmt::Select { arms }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Select:
    When pipe1 has x:
        Show x to show.
    When pipe2 has y:
        Show y to show.
```

**Semantics**: Waits on multiple channels simultaneously, executing the arm for whichever receives a value first. Similar to Go's `select` or Rust's `tokio::select!`.

**Type Constraints**:
- `arms`: List of `(pipe, var, body)` tuples
- All pipes can have different types
- Bodies must have compatible effects (e.g., all async or all sync)

**Codegen Pattern**:
```rust
tokio::select! {
    Some(x) = pipe1.rx.lock().await.recv() => {
        // arm 1 body
    }
    Some(y) = pipe2.rx.lock().await.recv() => {
        // arm 2 body
    }
}
```

**Triggers**:
- Async: Yes (always)
- VFS: Depends on arm bodies
- Networking: Depends on arm bodies

**Tested With**:
- Basic select → `e2e_concurrency.rs:789`
- Select 2 channels → `e2e_concurrency.rs:812`
- Select 3+ channels → `e2e_concurrency.rs:834`
- Select with side effects → `e2e_concurrency.rs:856`

**Untested With**:
- Select + timeout (P1) - Common pattern
- Select + default arm (P2) - Non-blocking select
- Nested select (P2) - Complex control flow

**Known Issues**:
- No timeout support currently
- No default (non-blocking) arm support
- Arms cannot have different async modes

**Examples**:

```logos
## Example: First responder wins
Create a pipe called fast.
Create a pipe called slow.

Launch a task:
    Sleep 100.
    Send "fast!" into fast.

Launch a task:
    Sleep 500.
    Send "slow!" into slow.

Select:
    When fast has msg:
        Show msg to show.  # Prints "fast!"
    When slow has msg:
        Show msg to show.  # Not reached
```

---

#### Feature: Parallel (Rayon Parallel Execution)

**AST Node**: `Stmt::Parallel { tasks }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Simultaneously:
    Task 1:
        Call compute_a.
    Task 2:
        Call compute_b.
```

**Semantics**: Executes multiple tasks in parallel using Rayon's thread pool. Unlike `Concurrent` (async), `Parallel` uses OS threads and blocks until all tasks complete. Best for CPU-bound work.

**Type Constraints**:
- `tasks`: List of task bodies
- Tasks must be `Send + Sync`
- Tasks cannot contain `await` (not async)

**Codegen Pattern**:
```rust
rayon::join(
    || { /* task 1 */ },
    || { /* task 2 */ },
);
```

**Triggers**:
- Async: No (uses OS threads, not async runtime)
- VFS: No
- Networking: No

**Tested With**:
- Basic parallel → `e2e_concurrency.rs:878`
- Parallel with returns → `e2e_concurrency.rs:901`
- 3+ parallel tasks → `e2e_concurrency.rs:923`
- Parallel CPU-bound → `e2e_concurrency.rs:945`

**Untested With**:
- Parallel + collections (P1) - Shared state
- Parallel + CRDT (P2) - Distributed + parallel

**Known Issues**:
- Cannot mix async and parallel (runtime conflict)
- Limited to 2-3 tasks (rayon::join has variants for 2/3)
- No dynamic task spawning

**Examples**:

```logos
## Example 1: Parallel computation
To expensive (n: Int) returns Int:
    Let sum be 0.
    Repeat for i in [1 through n]:
        Set sum to sum + i.
    Return sum.

## Main:
Simultaneously:
    Task 1:
        Let a be Call expensive with 1000.
        Show a to show.
    Task 2:
        Let b be Call expensive with 2000.
        Show b to show.
```

```logos
## Example 2: Data-parallel processing
Let data1 be [1, 2, 3, 4, 5].
Let data2 be [6, 7, 8, 9, 10].

Simultaneously:
    Task 1:
        Repeat for x in data1:
            Show x to show.
    Task 2:
        Repeat for y in data2:
            Show y to show.
```

---

#### Feature: Sleep (Async Delay)

**AST Node**: `Stmt::Sleep { duration }`
**Category**: Concurrency
**Phase Introduced**: Phase 42

**Syntax**:
```logos
Sleep 1000.
Sleep duration_ms.
```

**Semantics**: Pauses execution asynchronously for the specified duration in milliseconds. Uses `tokio::time::sleep`, yielding the task to allow other tasks to run.

**Type Constraints**:
- `duration`: Expression of type `Int` (milliseconds)

**Codegen Pattern**:
```rust
tokio::time::sleep(Duration::from_millis(duration as u64)).await;
```

**Triggers**:
- Async: Yes (always)
- VFS: No
- Networking: No

**Tested With**:
- Basic sleep → `e2e_concurrency.rs:967`
- Sleep in loop → `e2e_concurrency.rs:989`
- Sleep in task → `e2e_concurrency.rs:1012`
- Variable duration → `e2e_concurrency.rs:1034`

**Untested With**:
- Sleep with Duration type (P2) - Type system extension
- Sleep in Inspect arm (P0 - KNOWN BUG) - Currently fails

**Known Issues**:
- Using Sleep inside Inspect arms causes compilation errors (bug #156)
- Duration must be Int (milliseconds), no Duration type support yet

**Examples**:

```logos
## Example 1: Rate limiting
Repeat for i in [1, 2, 3, 4, 5]:
    Call process_item with i.
    Sleep 100.  # Wait 100ms between items
```

```logos
## Example 2: Timeout simulation
Launch a task:
    Sleep 5000.
    Show "Timeout!" to show.

Launch a task:
    Let result be Call quick_operation.
    Show result to show.
```

```logos
## Example 3: Exponential backoff
Let delay be 100.
Repeat for attempt in [1, 2, 3]:
    Sleep delay.
    Show attempt to show.
    Set delay to delay times 2.  # 100, 200, 400
```

---

### Networking

#### Feature: Listen (Network Bind)

**AST Node**: `Stmt::Listen { address }`
**Category**: Networking
**Phase Introduced**: Phase 68

**Syntax**:
```logos
Listen on "/ip4/0.0.0.0/tcp/8080".
Listen on address_str.
```

**Semantics**: Binds a network listener on the specified multiaddress using libp2p. Creates a server that can accept incoming peer connections for P2P communication.

**Type Constraints**:
- `address`: Expression evaluating to Text (multiaddress format)

**Codegen Pattern**:
```rust
logicaffeine_system::network::listen(address).await?;
```

**Multiaddress Format**: Uses libp2p multiaddress format:
- IPv4 TCP: `/ip4/0.0.0.0/tcp/8080`
- IPv6 TCP: `/ip6/::/tcp/8080`
- With peer ID: `/ip4/1.2.3.4/tcp/8080/p2p/QmPeerId...`

**Triggers**:
- Async: Yes (always)
- VFS: No
- Networking: Yes (always)

**Tested With**:
- Basic listen → `e2e_gossip.rs:15`
- Listen with dynamic port → `e2e_gossip.rs:45`

**Untested With**:
- Listen + Concurrent (P1) - Multiple listeners
- Listen + CRDT sync (P0) - Distributed state sync
- Listen with TLS (P2) - Not implemented

**Known Issues**:
- No IPv6 support yet (libp2p limitation)
- Port conflicts not gracefully handled
- Requires `networking` feature flag

**Examples**:

```logos
## Example 1: Basic server
Listen on "/ip4/0.0.0.0/tcp/8080".
Show "Server listening..." to show.
```

```logos
## Example 2: P2P node
Let peer_addr be "/ip4/127.0.0.1/tcp/9000".
Listen on peer_addr.

## Now can accept connections from other peers
```

---

#### Feature: ConnectTo (Peer Connection)

**AST Node**: `Stmt::ConnectTo { peer, address }`
**Category**: Networking
**Phase Introduced**: Phase 68

**Syntax**:
```logos
Connect to peer at "/ip4/127.0.0.1/tcp/8080/p2p/QmPeerId...".
Connect to remote at address_str.
```

**Semantics**: Establishes a P2P connection to a remote peer using libp2p. The peer must be listening on the specified multiaddress.

**Type Constraints**:
- `peer`: Symbol (variable to bind peer handle)
- `address`: Expression evaluating to Text (multiaddress with peer ID)

**Codegen Pattern**:
```rust
let peer = logicaffeine_system::network::connect_to(address).await?;
```

**Triggers**:
- Async: Yes (always)
- VFS: No
- Networking: Yes (always)

**Tested With**:
- Basic connect → `e2e_gossip.rs:67`
- Connect + message exchange → `e2e_gossip.rs:89`

**Untested With**:
- Connect + retry logic (P1) - Connection failures
- Connect + timeout (P1) - Hanging connections
- Multiple concurrent connections (P1) - Connection pooling

**Known Issues**:
- No automatic reconnection on disconnect
- Connection timeout not configurable
- Peer discovery not implemented (need full multiaddress)

**Examples**:

```logos
## Example: Connect and send message
Let peer_addr be "/ip4/127.0.0.1/tcp/9000/p2p/Qm...".
Connect to remote at peer_addr.
Send "Hello" to remote.
```

---

#### Feature: LetPeerAgent (Agent-Based Peer)

**AST Node**: `Stmt::LetPeerAgent { var, address, agent }`
**Category**: Networking + Agents
**Phase Introduced**: Phase 68

**Syntax**:
```logos
Let peer be a remote agent at "/ip4/...".
Send message to peer.
Await response from peer.
```

**Semantics**: Creates a peer agent that can asynchronously send and receive messages over the network. Combines networking with the agent model (Spawn/SendMessage/AwaitMessage).

**Type Constraints**:
- `var`: Symbol (peer agent variable)
- `address`: Expression evaluating to Text (multiaddress)
- `agent`: Optional agent type/protocol

**Codegen Pattern**:
```rust
let peer = PeerAgent::connect(address).await?;
```

**Triggers**:
- Async: Yes (always)
- VFS: No
- Networking: Yes (always)

**Tested With**:
- Basic peer agent → `e2e_gossip.rs:112`
- Peer + message exchange → `e2e_gossip.rs:134`

**Untested With**:
- Peer + agent protocols (P1) - Typed message passing
- Peer + CRDT sync (P0) - Distributed state
- Peer + discovery (P2) - Automatic peer finding

**Known Issues**:
- Agent protocols not fully implemented
- Message serialization limited to basic types
- No message ordering guarantees

**Examples**:

```logos
## Example: Peer agent communication
Let remote be a remote agent at "/ip4/192.168.1.100/tcp/8080/p2p/Qm...".

Send (a message with type "greeting" and data "Hello") to remote.
Await response from remote.

Show response to show.
```

---

### I/O Operations

#### Feature: ReadFrom (Console/File Read)

**AST Node**: `Stmt::ReadFrom { var, source }`
**Category**: I/O
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Read input from the console.
Read data from file "config.txt".
```

**Semantics**: Reads input from console (stdin) or file into a variable.

**Type Constraints**:
- `var`: Symbol (variable to bind)
- `source`: `ReadSource::Console` or `ReadSource::File(path)`

**Codegen Pattern**:
```rust
// Console
let input = read_line();

// File (async, uses VFS)
let data = vfs.read("config.txt").await?;
```

**Triggers**:
- Async: Yes (for file I/O)
- VFS: Yes (for file I/O)
- Networking: No

**Tested With**:
- Console read → `e2e_integration.rs`
- File read → `e2e_integration.rs`

**Untested With**:
- Read in concurrent blocks (P2)
- Read with refinement types (P2)

---

#### Feature: WriteFile (File Write)

**AST Node**: `Stmt::WriteFile { content, path }`
**Category**: I/O
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Write "content" to file "output.txt".
```

**Semantics**: Writes content to a file using VFS.

**Type Constraints**:
- `content`: Expression evaluating to Text
- `path`: Expression evaluating to Text

**Codegen Pattern**:
```rust
vfs.write("output.txt", content.as_bytes()).await?;
```

**Triggers**:
- Async: Yes
- VFS: Yes
- Networking: No

**Tested With**:
- Basic file writes → `e2e_integration.rs`

**Untested With**:
- Write in concurrent blocks (P1)
- Write with CRDT content (P2)

---

### Verification

#### Feature: Assert (Runtime Assertion)

**AST Node**: `Stmt::Assert { proposition }` or `Stmt::RuntimeAssert { condition }`
**Category**: Verification
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Assert that x > 0.
```

**Semantics**: Runtime assertion that panics if false. Used for preconditions and invariants.

**Type Constraints**:
- `proposition`: LogicExpr (from Logic Kernel)
- `condition`: Expr (imperative boolean)

**Codegen Pattern**:
```rust
assert!(x > 0, "Assertion failed: x > 0");
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Simple assertions → `e2e_refinement.rs`
- Logical assertions → `e2e_logical.rs`

**Untested With**:
- Assert in concurrent blocks (P2)
- Assert with CRDT values (P2)

---

#### Feature: Trust (Documented Assumption)

**AST Node**: `Stmt::Trust { proposition, justification }`
**Category**: Verification
**Phase Introduced**: Phase 1

**Syntax**:
```logos
Trust that x > 0 because "initialized above".
```

**Semantics**: Like Assert, but with mandatory justification. Used for solver bypass.

**Type Constraints**:
- `proposition`: LogicExpr
- `justification`: Symbol (reason string)

**Codegen Pattern**:
```rust
// In release mode: nothing (trusted)
// In debug mode: assert with justification comment
debug_assert!(x > 0, "Trusted: initialized above");
```

---

#### Feature: Check (Security Guard)

**AST Node**: `Stmt::Check { subject, predicate, is_capability, object, source_text, span }`
**Category**: Verification
**Phase Introduced**: Phase 50

**Syntax**:
```logos
Check that user is admin.
Check that user can publish the document.
```

**Semantics**: Mandatory security check. Never optimized out. Panics if false.

**Type Constraints**:
- `subject`: Symbol (entity being checked)
- `predicate`: Symbol (property or action)
- `is_capability`: Boolean (true for "can" checks)
- `object`: Optional Symbol (for capabilities)

**Codegen Pattern**:
```rust
// Predicate check
assert!(user.is_admin(), "Check failed: user is admin");

// Capability check
assert!(user.can_publish(&doc), "Check failed: user can publish the document");
```

**Policy Resolution**: Resolves predicates and capabilities from PolicyRegistry.

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Predicate checks → `e2e_policy.rs`
- Capability checks → `e2e_policy.rs`

**Untested With**:
- Check with CRDT objects (P2)
- Check in concurrent blocks (P2)

---

#### Feature: Trust (Documented Assumption)

**AST Node**: `Stmt::Trust { predicate, reason, span }`
**Category**: Verification
**Phase Introduced**: Phase 50

**Syntax**:
```logos
Trust that x > 0 because "validated by upstream function".
Trust that buffer_size is even because "allocated in pairs".
```

**Semantics**: Documents an assumption as a `debug_assert!` that is only checked in debug builds. Use this to state invariants that should be true but are expensive to check or guaranteed by external code.

**Type Constraints**:
- `predicate`: LogicExpr (boolean expression)
- `reason`: Text (justification string)

**Codegen Pattern**:
```rust
debug_assert!(x > 0, "Trust assumption: validated by upstream function");
```

**Triggers**:
- Async: No
- VFS: No
- Networking: No

**Tested With**:
- Basic trust → `e2e_refinement.rs`
- Trust with reason → `e2e_refinement.rs`

**Untested With**:
- Trust in hot loops (P2) - Performance impact
- Trust with complex predicates (P1) - Logic kernel integration

**Known Issues**:
- None currently

**Examples**:

```logos
## Example 1: Performance-sensitive path
To process_validated_data (n: Int):
    Trust that n > 0 because "caller guarantees positive input".
    Let result be 100 divided by n.  # Safe, no zero-division check needed
    Return result.
```

```logos
## Example 2: External invariant
To handle_buffer (data: Seq of Byte):
    Trust that the length of data is even because "allocated in pairs by system".
    Repeat for i in [1 through (the length of data divided by 2)]:
        Let pair_index be (i times 2) minus 1.
        # Process pairs
```

---

#### Feature: Theorem (Formal Verification)

**AST Node**: `Stmt::Theorem { name, statement, proof }`
**Category**: Verification
**Phase Introduced**: Phase 3 (Logic Kernel)

**Syntax**:
```logos
Theorem: Commutativity of Addition.
For all x and y, x + y = y + x.

Proof:
    # Proof steps (checked by Z3)
    ...
```

**Semantics**: Declares a theorem to be verified by the SMT solver (Z3). The theorem is checked at compile time using the logic kernel. Theorems do not generate runtime code.

**Type Constraints**:
- `name`: Symbol (theorem identifier)
- `statement`: LogicExpr (proposition to prove)
- `proof`: Optional proof derivation

**Codegen Pattern**:
```rust
// No runtime code generated
// Verification happens at compile time via Z3
```

**Verification**: The statement is translated to SMT-LIB and checked for satisfiability. If unsatisfiable, the negation is a tautology (theorem holds).

**Triggers**:
- Async: No (compile-time only)
- VFS: No
- Networking: No

**Tested With**:
- Basic theorems → `phase3_kernel.rs`
- Logical tautologies → `phase11_sorts.rs`
- Quantified statements → `phase13_mwe.rs`

**Untested With**:
- Theorem + Imperative integration (P1) - Bridging modes
- Complex proofs (P2) - Multi-step derivations

**Known Issues**:
- Proof language not fully implemented
- Z3 timeout can cause false negatives
- No support for custom theories yet

**Examples**:

```logos
## Example 1: Simple tautology
Theorem: Law of Excluded Middle.
For all P, P or not P.

## Verification: Automatic by Z3
```

```logos
## Example 2: Arithmetic property
Theorem: Zero Identity.
For all x, x + 0 = x.

## Verification: Checked using integer arithmetic theory
```

```logos
## Example 3: Implication chain
Theorem: Transitivity of Implication.
For all P, Q, R: If (P implies Q) and (Q implies R), then (P implies R).

## Verification: Propositional logic
```

---

### Expression Types Summary

**Note**: Many expression types are documented above as collection operations. The following table summarizes all 19 expression types:

| Expression | AST Node | Category | Status | Documentation |
|------------|----------|----------|--------|---------------|
| Literal | `Expr::Literal` | Primitive | ✓ | Core type values |
| Identifier | `Expr::Identifier` | Variable | ✓ | Variable reference |
| BinaryOp | `Expr::BinaryOp` | Operator | ✓ | +, -, *, /, ==, <, etc. |
| Call | `Expr::Call` | Function | ✓ | See [Call](#feature-call-function-call) |
| Index | `Expr::Index` | Collection | ✓ | See [Index](#expression-index-element-access) |
| Slice | `Expr::Slice` | Collection | ✓ | See [Slice](#expression-slice-range-access) |
| Copy | `Expr::Copy` | Collection | ✓ | See [Copy](#expression-copy-deep-clone) |
| Length | `Expr::Length` | Collection | ✓ | See [Length](#expression-length-collection-size) |
| Contains | `Expr::Contains` | Collection | ✓ | See [Contains](#expression-contains-membership-test) |
| Union | `Expr::Union` | Collection | ✓ | See [Union](#expression-union-set-union) |
| Intersection | `Expr::Intersection` | Collection | ✓ | See [Intersection](#expression-intersection-set-intersection) |
| ManifestOf | `Expr::ManifestOf` | Memory | ○ | Zone manifest access |
| ChunkAt | `Expr::ChunkAt` | Memory | ○ | Zone chunk access |
| List | `Expr::List` | Literal | ✓ | Array literal `[1, 2, 3]` |
| Tuple | `Expr::Tuple` | Literal | ✓ | Tuple literal `(1, "x", true)` |
| Range | `Expr::Range` | Collection | ✓ | Range `1 to 10` |
| FieldAccess | `Expr::FieldAccess` | Struct | ✓ | `p's x` or `the x of p` |
| New | `Expr::New` | Constructor | ✓ | `a new Point with x 0 and y 0` |
| NewVariant | `Expr::NewVariant` | Constructor | ✓ | `a new Circle with radius 5` |

**Coverage**: 17/19 expression types documented (89%). ManifestOf and ChunkAt are specialized zone operations documented in phase85_zones.rs.

---

## Feature Interaction Matrices

### Primary 10×10 Matrix

This matrix shows high-level interactions between feature dimensions:

```
                     Variables  Control  Functions  Data      Collections  Ownership  Memory   Concurrency  CRDTs    Types    I/O      Verification
Variables            —          ✓        ✓          ✓         ✓            ✓          ✓        ⚠️           ✓        ✓        ✓        ✓
Control              ✓          —        ✓          ✓         ✓            ✓          ✓        ⚠️           ✓        ✓        ✓        ✓
Functions            ✓          ✓        —          ✓         ○            ○          ○        ✓            ○        ⚠️       ✓        ○
Data                 ✓          ✓        ✓          —         ✓            ✓          ○        ⚠️           ⚠️       ✓        ✓        ✓
Collections          ✓          ✓        ○          ✓         —            ⚠️         ○        ✗            ✓        ⚠️       ✓        ⚠️
Ownership            ✓          ✓        ○          ✓         ⚠️           —          ✗        ✗            ⚠️       ✓        ✓        ✓
Memory               ✓          ✓        ○          ○         ○            ✗          —        ○            ✗        ✓        ○        ○
Concurrency          ⚠️         ⚠️       ✓          ⚠️        ✗            ✗          ○        —            ✓        ⚠️       ✓        ⚠️
CRDTs                ✓          ✓        ○          ⚠️        ✓            ⚠️         ✗        ✓            —        ✓        ✓        ✓
Types                ✓          ✓        ⚠️         ✓         ⚠️           ✓          ✓        ⚠️           ✓        —        ✓        ✓
I/O                  ✓          ✓        ✓          ✓         ✓            ✓          ○        ✓            ✓        ✓        —        ✓
Verification         ✓          ✓        ○          ✓         ⚠️           ✓          ○        ⚠️           ✓        ✓        ✓        —
```

**Key**:
- ✓ = Well tested, works correctly
- ○ = Untested but should work
- ⚠️ = Partial coverage or known issues
- ✗ = Impossible (type system prevents)
- — = Not applicable (self-interaction)

---

### Async/Concurrency Interactions

Detailed matrix for async and concurrent features:

| Feature        | Async Fn | Concurrent | Parallel | Launch | Pipe | Select | SendPipe | ReceivePipe | Status  | Test Reference |
|----------------|----------|------------|----------|--------|------|--------|----------|-------------|---------|----------------|
| Let            | ✓        | ✓          | ✓        | ✓      | ✓    | ✓      | ✓        | ✓           | GOOD    | e2e_async:89   |
| Set            | ✓        | ⚠️         | ⚠️       | ⚠️     | ✓    | ✓      | ✓        | ✓           | PARTIAL | Race detection |
| If             | ✓        | ✓          | ✓        | ✓      | ✓    | ✓      | ✓        | ✓           | GOOD    | e2e_async:125  |
| While          | ✓        | ○          | ○        | ○      | ✓    | ○      | ○        | ○           | PARTIAL | e2e_async:156  |
| Repeat         | ✓        | ○          | ○        | ○      | ✓    | ○      | ○        | ○           | PARTIAL | e2e_async:178  |
| Return         | ✓        | ✓          | ✓        | ✓      | ✓    | ✓      | ✓        | ✓           | GOOD    | e2e_async:201  |
| Struct         | ✓        | ✓          | ✓        | ○      | ○    | ○      | ○        | ○           | GOOD    | e2e_async:234  |
| Enum           | ✓        | ✓          | ○        | ○      | ○    | ○      | ○        | ○           | PARTIAL | e2e_enums      |
| Inspect        | ✓        | ⚠️         | ○        | ○      | ○    | ○      | ○        | ○           | BUG     | Sleep in arm   |
| Push           | ✓        | ⚠️         | ✗        | ⚠️     | ✓    | ○      | ○        | ○           | PARTIAL | Sync issue     |
| Pop            | ✓        | ⚠️         | ✗        | ⚠️     | ✓    | ○      | ○        | ○           | PARTIAL | Sync issue     |
| Give           | ○        | ?          | ?        | ✗      | ✓    | ?      | ?        | ?           | UNKNOWN | GAP            |
| Show           | ✓        | ✓          | ✓        | ✓      | ✓    | ✓      | ✓        | ✓           | GOOD    | e2e_async:89   |
| SetField       | ✓        | ⚠️         | ⚠️       | ⚠️     | ✓    | ○      | ○        | ○           | PARTIAL | CRDT fields    |
| FunctionDef    | ✓        | ✓          | ✓        | ✓      | ✓    | ✓      | ✓        | ✓           | GOOD    | e2e_async:267  |
| Call           | ✓        | ✓          | ✓        | ✓      | ✓    | ✓      | ✓        | ✓           | GOOD    | e2e_async:289  |
| Zone           | ?        | ?          | ?        | ?      | ?    | ?      | ?        | ?           | UNKNOWN | GAP            |
| CRDT Ops       | ○        | ○          | ?        | ?      | ?    | ?      | ?        | ?           | UNKNOWN | GAP            |
| Mount          | ✓        | ○          | ○        | ○      | ○    | ○      | ○        | ○           | PARTIAL | Needs tests    |
| Sync           | ✓        | ○          | ○        | ○      | ○    | ○      | ○        | ○           | PARTIAL | Needs tests    |
| Listen         | ✓        | ○          | ○        | ○      | ○    | ○      | ○        | ○           | PARTIAL | Needs tests    |
| ReadFrom       | ✓        | ○          | ○        | ○      | ○    | ○      | ○        | ○           | PARTIAL | Needs tests    |
| WriteFile      | ✓        | ○          | ○        | ○      | ○    | ○      | ○        | ○           | PARTIAL | Needs tests    |

---

### CRDT Interactions

Detailed matrix for CRDT operations with other features:

| Feature        | GCounter | PNCounter | LWW | MVRegister | ORSet | ORMap | RGA | YATA | Merge | Sync | Mount | Status  | Test Reference |
|----------------|----------|-----------|-----|------------|-------|-------|-----|------|-------|------|-------|---------|----------------|
| Let            | ✓        | ✓         | ✓   | ✓          | ✓     | ✓     | ✓   | ✓    | ✓     | ✓    | ✓     | GOOD    | e2e_crdt:15    |
| Set            | ✗        | ✗         | ✓   | ✓          | ✗     | ✗     | ✗   | ✗    | ✗     | ✗    | ✗     | N/A     | CRDTs use ops  |
| SetField       | ✗        | ✗         | ✓   | ✓          | ✗     | ✗     | ✗   | ✗    | ✗     | ✗    | ✗     | N/A     | Use .set()     |
| Increase       | ✓        | ✓         | ✗   | ✗          | ✗     | ○     | ✗   | ✗    | ✗     | ✓    | ✓     | GOOD    | e2e_crdt:89    |
| Decrease       | ✗        | ✓         | ✗   | ✗          | ✗     | ○     | ✗   | ✗    | ✗     | ✓    | ✓     | GOOD    | e2e_crdt:112   |
| Add (Set)      | ✗        | ✗         | ✗   | ✗          | ✓     | ○     | ✗   | ✗    | ✗     | ✓    | ✓     | GOOD    | e2e_crdt:134   |
| Remove (Set)   | ✗        | ✗         | ✗   | ✗          | ✓     | ○     | ✗   | ✗    | ✗     | ✓    | ✓     | GOOD    | e2e_crdt:156   |
| Append         | ✗        | ✗         | ✗   | ✗          | ✗     | ✗     | ✓   | ✓    | ✗     | ✓    | ✓     | GOOD    | e2e_crdt:178   |
| Resolve        | ✗        | ✗         | ✗   | ✓          | ✗     | ✗     | ✗   | ✗    | ✗     | ✓    | ✓     | GOOD    | e2e_crdt:201   |
| Merge          | ✓        | ✓         | ✓   | ✓          | ✓     | ✓     | ✓   | ✓    | —     | ✓    | ✓     | GOOD    | e2e_crdt:234   |
| Struct fields  | ✓        | ✓         | ✓   | ✓          | ✓     | ✓     | ✓   | ✓    | ✓     | ✓    | ✓     | GOOD    | e2e_crdt:267   |
| Nested CRDTs   | ○        | ○         | ○   | ○          | ○     | ✓     | ○   | ○    | ⚠️    | ○    | ○     | PARTIAL | ORMap tests    |
| Generics       | ?        | ?         | ?   | ?          | ?     | ?     | ?   | ?    | ?     | ?    | ?     | UNKNOWN | GAP            |
| Concurrent     | ⚠️       | ⚠️        | ⚠️  | ⚠️         | ⚠️    | ⚠️    | ⚠️  | ⚠️   | ✓     | ✓    | ✓     | PARTIAL | Needs merges   |
| Give           | ○        | ○         | ○   | ○          | ○     | ○     | ○   | ○    | ○     | ○    | ○     | UNKNOWN | GAP            |
| Return         | ✓        | ✓         | ✓   | ✓          | ✓     | ✓     | ✓   | ✓    | ✓     | ✓    | ✓     | GOOD    | e2e_crdt:301   |

**Notes**:
- ✓ = Tested and working
- ○ = Untested but should work (type system allows)
- ⚠️ = Partial coverage or complex interaction
- ✗ = Not applicable (CRDTs use specialized operations)
- ? = Unknown, needs investigation

**Key Insights**:
- CRDTs do not support direct `Set` assignments - use `.set()`, `.increase()`, `.add()`, etc.
- ORMap supports nested CRDTs (e.g., `Map from Text to PNCounter`)
- Concurrent mutations require `Merge` to resolve conflicts
- CRDT + Generics is completely untested (P1 gap)
- Give semantics with CRDTs unclear (ownership of distributed state?)

---

### Ownership Interactions

Detailed matrix for ownership semantics (Give, Show, Copy):

| Feature        | Give | Show | Copy | Mutable | Return | Parameter | Local Scope | Zone Scope | Status  | Test Reference |
|----------------|------|------|------|---------|--------|-----------|-------------|------------|---------|----------------|
| Let            | ✓    | ✓    | ✓    | ✓       | ✓      | ✓         | ✓           | ✓          | GOOD    | e2e_ownership  |
| Set            | ✓    | ✓    | ○    | ✓       | ✓      | ○         | ✓           | ⚠️         | PARTIAL | Zone lifetime  |
| Struct         | ✓    | ✓    | ✓    | ✓       | ✓      | ✓         | ✓           | ⚠️         | PARTIAL | Zone fields    |
| Enum           | ✓    | ✓    | ○    | ✓       | ✓      | ○         | ✓           | ○          | PARTIAL | Needs tests    |
| Seq            | ⚠️   | ✓    | ✓    | ✓       | ✓      | ✓         | ✓           | ○          | PARTIAL | Push/Give      |
| Set            | ⚠️   | ✓    | ✓    | ✓       | ✓      | ○         | ✓           | ○          | PARTIAL | Add/Give       |
| Map            | ⚠️   | ✓    | ✓    | ✓       | ✓      | ○         | ✓           | ○          | PARTIAL | Insert/Give    |
| CRDT           | ?    | ✓    | ✗    | ✓       | ✓      | ?         | ✓           | ✗          | UNKNOWN | GAP            |
| FunctionDef    | ✓    | ✓    | ○    | ✓       | ✓      | ✓         | ✓           | ○          | GOOD    | e2e_functions  |
| Call           | ✓    | ✓    | ○    | —       | —      | ✓         | ✓           | ○          | GOOD    | e2e_functions  |
| Launch         | ✗    | ✓    | ✓    | —       | ✗      | ✓         | —           | ✗          | CONSTRAINED | Must be Send |
| Concurrent     | ✗    | ✓    | ✓    | ⚠️      | ✓      | ✓         | ⚠️          | ✗          | CONSTRAINED | Race detector |
| Parallel       | ✗    | ✓    | ✓    | ⚠️      | ✓      | ✓         | ⚠️          | ✗          | CONSTRAINED | Must be Send+Sync |
| Zone           | ✗    | ✓    | ✗    | ✓       | ✗      | ✗         | ✓           | —          | CONSTRAINED | No escape |

**Key Insights**:
- **Give** = Move semantics (ownership transfer), invalidates source
- **Show** = Immutable borrow (&T), source remains valid
- **Copy** = Deep clone, both source and copy are valid
- **Zone** = Arena allocation, values cannot escape scope (Hotel California)
- **Launch/Concurrent/Parallel** = Cannot Give (ownership cannot cross async boundary without Send)
- **CRDT + Give** = Unclear semantics (distributed state ownership?)

**Critical Gaps**:
- Give through function parameters (P0) - Does ownership relay work?
- Collections + Give (P0) - Can you Give a value from inside a Vec/Set/Map?
- CRDT ownership (P1) - What happens when you Give a CRDT variable?
- Zone escape detection (P1) - Does compiler catch all lifetime violations?

---

### Collections Interactions

Detailed matrix for collection operations:

| Feature        | Seq | Set | Map | Push | Pop | Add | Remove | Index | Slice | Length | Contains | Union | Intersection | Status  | Test Reference |
|----------------|-----|-----|-----|------|-----|-----|--------|-------|-------|--------|----------|-------|--------------|---------|----------------|
| Let            | ✓   | ✓   | ✓   | —    | —   | —   | —      | —     | —     | —      | —        | —     | —            | GOOD    | e2e_collections:15 |
| Set            | ○   | ○   | ○   | —    | —   | —   | —      | ✓     | ○     | —      | —        | —     | —            | PARTIAL | SetIndex tested |
| Struct fields  | ✓   | ✓   | ✓   | ✓    | ✓   | ✓   | ✓      | ✓     | ✓     | ✓      | ✓        | ○     | ○            | PARTIAL | e2e_collections:89 |
| Enum variants  | ○   | ○   | ○   | ○    | ○   | ○   | ○      | ○     | ○     | ○      | ○        | ○     | ○            | UNKNOWN | GAP |
| Push           | ✓   | —   | —   | —    | ✓   | —   | —      | ✓     | ✓     | ✓      | ✓        | —     | —            | GOOD    | e2e_collections:112 |
| Pop            | ✓   | —   | —   | ✓    | —   | —   | —      | ✓     | ✓     | ✓      | ✓        | —     | —            | GOOD    | e2e_collections:134 |
| Add            | —   | ✓   | —   | —    | —   | —   | ✓      | —     | —     | ✓      | ✓        | ✓     | ✓            | GOOD    | e2e_collections:156 |
| Remove         | —   | ✓   | ✓   | —    | —   | ✓   | —      | —     | —     | ✓      | ✓        | ✓     | ✓            | GOOD    | e2e_collections:178 |
| Index          | ✓   | —   | ✓   | ○    | ○   | —   | —      | —     | ✓     | ✓      | ✓        | —     | —            | GOOD    | e2e_collections:201 |
| Slice          | ✓   | —   | —   | ○    | ○   | —   | —      | ✓     | —     | ✓      | ✓        | —     | —            | GOOD    | e2e_collections:223 |
| Copy           | ✓   | ✓   | ✓   | ○    | ○   | ○   | ○      | ✓     | ✓     | ✓      | ✓        | ✓     | ✓            | GOOD    | e2e_collections:245 |
| Repeat         | ✓   | ✓   | ✓   | ⚠️   | ⚠️  | ⚠️  | ⚠️     | ✓     | ✓     | ✓      | ✓        | ○     | ○            | PARTIAL | Borrow issues |
| Concurrent     | ✗   | ✗   | ✗   | ✗    | ✗   | ✗   | ✗      | ✓     | ✓     | ✓      | ✓        | ✗     | ✗            | CONSTRAINED | No shared mut |
| Generics       | ✓   | ✓   | ✓   | ✓    | ✓   | ✓   | ✓      | ✓     | ✓     | ✓      | ✓        | ○     | ○            | GOOD    | e2e_generics |
| Refinement     | ⚠️  | ⚠️  | ⚠️  | ⚠️   | ⚠️  | ⚠️  | ⚠️     | ⚠️    | ⚠️    | ✓      | ✓        | ⚠️    | ⚠️           | HAIRY   | Element constraints |
| Give           | ○   | ○   | ○   | ?    | ✓   | ?   | ?      | ?     | ?     | ✓      | ✓        | ○     | ○            | UNKNOWN | GAP |

**Key Insights**:
- **1-based indexing**: All LOGOS collections use 1-based indexing
- **Seq** = Vec (ordered, indexable, allows duplicates)
- **Set** = HashSet (unordered, unique elements)
- **Map** = HashMap (key-value pairs)
- **Concurrent** = Collections cannot be mutated concurrently without synchronization
- **Refinement** = Element-level constraints unclear (does `Seq of PositiveInt` re-check on Push?)

**Critical Gaps**:
- Collections + Give (P0) - Ownership semantics unclear
- Collections + Refinement (P1) - Element constraint checking
- Enum variants with collections (P1) - Completely untested
- Repeat + mutations (P1) - Borrow checker issues partially tested

---

### Type System Interactions

Detailed matrix for type system features:

| Feature        | Generics | Refinement | Struct | Enum | Option | Result | Pair | Triple | Polymorphism | Status  | Test Reference |
|----------------|----------|------------|--------|------|--------|--------|------|--------|--------------|---------|----------------|
| Let            | ✓        | ✓          | ✓      | ✓    | ✓      | ✓      | ✓    | ✓      | ✓            | GOOD    | e2e_types |
| FunctionDef    | ✓        | ✓          | ✓      | ✓    | ✓      | ✓      | ✓    | ○      | ✓            | GOOD    | e2e_functions |
| Return         | ✓        | ⚠️         | ✓      | ✓    | ✓      | ✓      | ✓    | ○      | ✓            | PARTIAL | Refinement check |
| Struct fields  | ✓        | ⚠️         | —      | ○    | ✓      | ✓      | ✓    | ○      | ✓            | PARTIAL | Field refinement |
| Enum variants  | ✓        | ?          | ✓      | —    | —      | —      | ○    | ○      | ✓            | PARTIAL | Variant refinement |
| Seq            | ✓        | ⚠️         | ✓      | ✓    | ✓      | ✓      | ✓    | ✓      | ✓            | PARTIAL | Element constraints |
| Set            | ✓        | ⚠️         | ✓      | ✓    | ✓      | ✓      | ✓    | ✓      | ✓            | PARTIAL | Element constraints |
| Map            | ✓        | ⚠️         | ✓      | ✓    | ✓      | ✓      | ✓    | ✓      | ✓            | PARTIAL | Value constraints |
| CRDT           | ?        | ?          | ✓      | ○    | ?      | ?      | ?    | ?      | ?            | UNKNOWN | GAP |
| Concurrent     | ⚠️       | ⚠️         | ✓      | ✓    | ✓      | ✓      | ○    | ○      | ⚠️           | PARTIAL | Send bounds |
| Inspect        | ✓        | ✓          | ✓      | ✓    | ✓      | ✓      | ○    | ○      | ✓            | GOOD    | e2e_enums:89 |
| Deep nesting   | ⚠️       | ?          | ✓      | ✓    | ✓      | ✓      | ○    | ○      | ⚠️           | HAIRY   | 3+ levels |

**Key Insights**:
- **Generics** = Parametric polymorphism (e.g., `Seq of T`, `Map from K to V`)
- **Refinement** = Dependent types (e.g., `Int where it > 0`)
- **Option** = Maybe type (Some/None), **Result** = Error handling (Ok/Err)
- **Pair/Triple** = Tuples (heterogeneous, fixed size)
- **Deep nesting** = Complex types like `Map from Text to Seq of Option of Pair of Int and Real`

**Critical Gaps**:
- CRDT + Generics (P1) - No tests for `ORSet of T`, `ORMap from K to V of T`
- Refinement + Collections (P1) - Element-level constraint checking unclear
- Enum variant refinements (P1) - Can you refine enum constructors?
- Deep generics (P0) - 3+ levels of nesting untested (may break type inference)

---

### Memory & Zones Interactions

Detailed matrix for Zone (arena allocation) interactions:

| Feature        | Zone Alloc | Zone Read | Zone Write | Zone Escape | Return | Parameter | Concurrent | Status  | Test Reference |
|----------------|------------|-----------|------------|-------------|--------|-----------|------------|---------|----------------|
| Let            | ✓          | ✓         | ✓          | ✗           | ✗      | ✗         | ○          | GOOD    | e2e_zones:15 |
| Struct         | ✓          | ✓         | ✓          | ✗           | ✗      | ✗         | ○          | GOOD    | e2e_zones:45 |
| Seq            | ○          | ○         | ○          | ✗           | ✗      | ✗         | ○          | UNKNOWN | GAP |
| FieldAccess    | ✓          | ✓         | ✓          | ✗           | ⚠️     | ✗         | ○          | PARTIAL | Field escape? |
| If/While       | ✓          | ✓         | ✓          | ✗           | ✗      | ✗         | ○          | GOOD    | e2e_zones:78 |
| FunctionCall   | ○          | ○         | ○          | ✗           | ✗      | ✗         | ○          | UNKNOWN | GAP |
| Nested zones   | ○          | ○         | ○          | ✗           | ✗      | ✗         | ○          | UNKNOWN | GAP |
| CRDT           | ✗          | ✗         | ✗          | ✗           | ✗      | ✗         | ✗          | IMPOSSIBLE | Distributed |
| File I/O       | ○          | ○         | ○          | ✗           | ✗      | ✗         | ○          | UNKNOWN | GAP |

**Key Insights**:
- **Zone** = Arena (bump allocator) for batch allocation/deallocation
- **Hotel California Rule** = Values allocated in a zone cannot escape its scope
- **No heap allocation** = Zone-allocated values live on arena, not heap
- **No concurrent access** = Zones are not thread-safe (no Send/Sync)
- **CRDTs impossible** = Distributed state cannot be zone-allocated

**Syntax**:
```logos
Inside a new heap zone called Z:
    Let x be 42.
    Let p be a new Point with x: 10, y: 20.
    # x and p are freed when zone ends
```

**Critical Gaps**:
- Zone + Collections (P2) - Can you allocate Seq/Set/Map in a zone?
- Zone + Return (P1) - Compiler must prevent escaping values
- Zone + Function parameters (P2) - Can functions take zone-allocated refs?
- Nested zones (P2) - Do inner zones inherit outer lifetime?
- Zone + File I/O (P2) - Can you write zone-allocated data to disk?

---

## Testing Gap Analysis

### Top 20 Critical Gaps

Based on the interaction matrices and test coverage analysis:

| # | Pattern | Category | Priority | Tests | Impact | Notes |
|---|---------|----------|----------|-------|--------|-------|
| 1 | Give through function parameters | UNTESTED | P0 | 0/3 | Ownership relay | Common pattern, no tests |
| 2 | Functions return Map/Set | UNTESTED | P0 | 0/4 | Collections | Unclear ownership |
| 3 | Structs + Seq + Push | UNTESTED | P0 | 0/3 | Data modeling | Nested mutations |
| 4 | Deep generics (3+ levels) | UNTESTED | P0 | 0/2 | Type inference | May break compiler |
| 5 | Refinement + Set operations | HAIRY | P1 | 0/4 | Safety | Re-checking predicates |
| 6 | Give into Launch tasks | CONFLICTING | P1 | 0/2 | Async ownership | Ownership + async |
| 7 | Zone + Return escape | CONSTRAINED | P1 | 0/3 | Lifetime safety | Hotel California rule |
| 8 | CRDTs + Generics | UNTESTED | P1 | 0/3 | Distributed data | ORMap<K, V<T>> |
| 9 | Shared structs + Shared fields | HAIRY | P1 | 0/2 | CRDT composition | Nested CRDTs |
| 10 | Async + Ownership + Structs | HAIRY | P1 | 1/5 | Real-world code | Complex interaction |
| 11 | Inspect with Sleep in arms | KNOWN BUG | P0 | 1/1 | Control flow | Currently failing |
| 12 | Concurrent + CRDT mutations | HAIRY | P1 | 0/4 | Race conditions | Merge conflicts |
| 13 | Persistent<T> + Generics | UNTESTED | P1 | 0/2 | Storage | Type erasure |
| 14 | Zone + CRDT | CONSTRAINED | P2 | 0/2 | Memory + Dist | Lifetime conflicts |
| 15 | Map iteration with mutations | HAIRY | P1 | 1/3 | Collections | Borrow checker |
| 16 | Nested While/Repeat | UNTESTED | P2 | 0/2 | Control flow | Preemption nesting |
| 17 | Refinement + Collections | HAIRY | P1 | 0/4 | Type safety | Element constraints |
| 18 | Generic enum variants | UNTESTED | P1 | 0/2 | ADTs | Pattern matching |
| 19 | CRDT in function parameters | UNTESTED | P1 | 0/3 | Distributed | Ownership semantics |
| 20 | Zone + File operations | CONSTRAINED | P2 | 0/2 | I/O | Mmap interactions |

---

### Gap Categories

**COMPOSITIONAL** (✓) - Features naturally compose:
- Let + If + Return
- Struct + FieldAccess
- Seq + Push + Pop + Repeat

**CONSTRAINED** (○) - Work together with restrictions:
- Zone + Return (no escaping values)
- Ownership + Async (must be Send)
- CRDT + Collections (must be Merge)

**CONFLICTING** (✗) - Cannot be combined (type system):
- Ownership + Concurrent (race conditions)
- Zone + Give (lifetime escape)
- Memory + Remote (cannot serialize arenas)

**UNTESTED** (○) - Should work but no tests:
- Functions return Map/Set
- Deep generics
- CRDTs + Generics
- Persistent<T> + Generics

**HAIRY** (⚠️) - Complex interactions needing care:
- Refinement + Set operations
- Shared structs + Shared fields
- Async + Ownership + Structs
- Concurrent + CRDT mutations

---

## Standard Library Reference

### Core Types

| LOGOS Type | Rust Type | Size | Range | Default | Serializable |
|------------|-----------|------|-------|---------|--------------|
| `Nat` | `u64` | 8B | 0 to 2^64-1 | 0 | ✓ |
| `Int` | `i64` | 8B | -2^63 to 2^63-1 | 0 | ✓ |
| `Real` | `f64` | 8B | ±1.7×10^308 | 0.0 | ✓ |
| `Text` | `String` | Var | UTF-8 | `""` | ✓ |
| `Bool` | `bool` | 1B | true/false | false | ✓ |
| `Unit` | `()` | 0B | — | `()` | ✓ |
| `Char` | `char` | 4B | Unicode scalar | `'\0'` | ✓ |
| `Byte` | `u8` | 1B | 0 to 255 | 0 | ✓ |

### Temporal Types

| LOGOS Type | Rust Type | Resolution | Range | Example |
|------------|-----------|------------|-------|---------|
| `Duration` | `i64` ns | Nanosecond | ±292 years | `5 seconds` |
| `Date` | `i32` days | Day | 1970-01-01 ± 5.8M years | `2026-02-05` |
| `Moment` | `i64` ns | Nanosecond | 1970-01-01 ± 292 years | `now()` |
| `Span` | `{months, days}` | Month/Day | — | `1 month and 5 days` |
| `Time` | `i64` ns | Nanosecond | 00:00:00 to 23:59:59 | `14:30:00` |

### Collections API

#### Seq<T> (Vec<T>)

| Method | LOGOS Syntax | Rust Code | Complexity |
|--------|--------------|-----------|------------|
| Create | `Let x be [1, 2, 3].` | `vec![1, 2, 3]` | O(n) |
| Length | `length of x` | `x.len()` | O(1) |
| Index | `item 2 of x` | `x.logos_get(2i64)` | O(1) |
| Slice | `x 1 through 3` | `x.logos_slice(1, 3)` | O(n) |
| Push | `Push 4 to x.` | `x.push(4)` | O(1)* |
| Pop | `Pop from x into y.` | `let y = x.pop()` | O(1) |
| Iterate | `Repeat for item in x:` | `for item in x.iter()` | O(n) |
| Copy | `copy of x` | `x.clone()` | O(n) |

*Amortized

#### Set<T> (HashSet<T>)

| Method | LOGOS Syntax | Rust Code | Complexity |
|--------|--------------|-----------|------------|
| Create | `Let s be an empty Set of Int.` | `HashSet::new()` | O(1) |
| Add | `Add 5 to s.` | `s.insert(5)` | O(1)* |
| Remove | `Remove 5 from s.` | `s.remove(&5)` | O(1)* |
| Contains | `s contains 5` or `5 in s` | `s.contains(&5)` | O(1)* |
| Union | `a union b` | `a.union(&b)` | O(n+m) |
| Intersection | `a intersection b` | `a.intersection(&b)` | O(min(n,m)) |
| Iterate | `Repeat for item in s:` | `for item in s.iter()` | O(n) |

*Average case

#### Map<K, V> (HashMap<K, V>)

| Method | LOGOS Syntax | Rust Code | Complexity |
|--------|--------------|-----------|------------|
| Create | `Let m be an empty Map from Text to Int.` | `HashMap::new()` | O(1) |
| Insert | `Set m's "key" to 42.` | `m.insert("key", 42)` | O(1)* |
| Get | `m's "key"` | `m.get("key")` | O(1)* |
| Remove | `Remove "key" from m.` | `m.remove("key")` | O(1)* |
| Contains | `m contains "key"` | `m.contains_key("key")` | O(1)* |
| Iterate | `Repeat for (k, v) in m:` | `for (k, v) in m.iter()` | O(n) |

*Average case

### CRDT Types

All CRDTs implement the `Merge` trait for conflict-free replication.

#### Counters

| Type | Description | Operations | Serializable | Use Case |
|------|-------------|------------|--------------|----------|
| `GCounter` | Grow-only counter | `increment(n)`, `value()`, `merge()` | ✓ | Page views, likes |
| `PNCounter` | Increment/decrement | `increment(n)`, `decrement(n)`, `value()`, `merge()` | ✓ | Scores, votes |

```logos
## GCounter Example
A Counter has:
    Shared points, which is Nat.

## Main:
Let c be a new Counter.
Increase c's points by 10.
Show c's points to show.  # 10
```

#### Registers

| Type | Description | Operations | Conflict Resolution |
|------|-------------|------------|---------------------|
| `LWWRegister<T>` | Last-write-wins | `set(value, timestamp)`, `get()`, `merge()` | Latest timestamp wins |
| `MVRegister<T>` | Multi-value | `set(value)`, `get()`, `has_conflict()`, `resolve()` | Preserves all concurrent writes |

```logos
## LWWRegister Example (Shared field)
A User has:
    Shared name, which is Text.

## Main:
Let user be a new User.
Set user's name to "Alice".
```

#### Sets

| Type | Description | Bias | Operations |
|------|-------------|------|------------|
| `ORSet<T, AddWins>` | Observed-Remove Set | Add wins | `add()`, `remove()`, `contains()`, `merge()` |
| `ORSet<T, RemoveWins>` | Observed-Remove Set | Remove wins | `add()`, `remove()`, `contains()`, `merge()` |

```logos
## ORSet Example
A Group has:
    Shared members, which is Set of Text.

## Main:
Let g be a new Group.
Add "Alice" to g's members.
Add "Bob" to g's members.
```

#### Maps

| Type | Description | Operations | Nested CRDTs |
|------|-------------|------------|--------------|
| `ORMap<K, V>` | Observed-Remove Map | `get()`, `get_or_insert()`, `remove()`, `merge()` | ✓ |

```logos
## ORMap with nested PNCounters
A Scoreboard has:
    Shared scores, which is Map from Text to Int.

## Main:
Let board be a new Scoreboard.
Increase board's scores's "Alice" by 100.
```

#### Sequences

| Type | Description | Operations | Best For |
|------|-------------|------------|----------|
| `RGA` | Replicated Growable Array | `append()`, `insert_at()`, `remove_at()`, `to_vec()`, `merge()` | Lists, logs |
| `YATA` | Yet Another Text Algorithm | `append()`, `insert_at()`, `remove_at()`, `to_string()`, `merge()` | Text editing |

```logos
## RGA Example
A Log has:
    Shared entries, which is Seq of Text.

## Main:
Let log be a new Log.
Append "Started" to log's entries.
Append "Processing" to log's entries.
```

### System Functions

#### I/O (`logicaffeine_system::io`)

| Function | Signature | Description | Platform |
|----------|-----------|-------------|----------|
| `show` | `show<T: Showable>(&T)` | Display value naturally | All |
| `println` | `println(Text)` | Print with newline | All |
| `read_line` | `read_line() -> Text` | Read from stdin | All |

#### Time (`logicaffeine_system::time`) — Native Only

| Function | Signature | Description | Platform |
|----------|-----------|-------------|----------|
| `now` | `now() -> i64` | Milliseconds since Unix epoch | Native |
| `sleep` | `sleep(i64)` | Block for N milliseconds | Native |

#### Environment (`logicaffeine_system::env`) — Native Only

| Function | Signature | Description | Platform |
|----------|-----------|-------------|----------|
| `get` | `get(Text) -> Option<Text>` | Get environment variable | Native |
| `args` | `args() -> Seq<Text>` | Command-line arguments | Native |

#### Random (`logicaffeine_system::random`) — Native Only

| Function | Signature | Description | Platform |
|----------|-----------|-------------|----------|
| `randomInt` | `randomInt(Int, Int) -> Int` | Random integer in range [a, b] | Native |
| `randomFloat` | `randomFloat() -> Real` | Random float in [0.0, 1.0) | Native |

#### File (`logicaffeine_system::file`) — Requires `persistence`

| Function | Signature | Description | Platform |
|----------|-----------|-------------|----------|
| `read` | `read(Text) -> Result<Text, Error>` | Read file contents | Native |
| `write` | `write(Text, Text) -> Result<(), Error>` | Write file contents | Native |

### Platform Support Matrix

| Component | Native | WASM | Feature Flag | Crate |
|-----------|--------|------|--------------|-------|
| Core I/O | ✓ | ✓ | — | system |
| time::now | ✓ | ✗ | — | system |
| time::sleep | ✓ | ✗ | — | system |
| env::get | ✓ | ✗ | — | system |
| env::args | ✓ | ✗ | — | system |
| random | ✓ | ✗ | — | system |
| file::read | ✓ | ✗ | persistence | system |
| file::write | ✓ | ✗ | persistence | system |
| VFS (NativeVfs) | ✓ | ✗ | persistence | system |
| VFS (OpfsVfs) | ✗ | ✓ | persistence | system |
| Persistent<T> | ✓ | ✓ | persistence | system |
| Zone (heap) | ✓ | ✗ | concurrency | system |
| Zone (mapped) | ✓ | ✗ | concurrency + persistence | system |
| Pipes | ✓ | ✗ | concurrency | system |
| spawn | ✓ | ✗ | concurrency | system |
| Network | ✓ | ✗ | networking | system |
| Synced<T> | ✓ | ✗ | networking | system |
| Distributed<T> (disk) | ✓ | ✓ | distributed | system |
| Distributed<T> (net) | ✓ | ✗ | distributed | system |
| GCounter | ✓ | ✓ | — | data |
| PNCounter | ✓ | ✓ | — | data |
| LWWRegister | ✓ | ✓ | — | data |
| MVRegister | ✓ | ✓ | — | data |
| ORSet | ✓ | ✓ | — | data |
| ORMap | ✓ | ✓ | — | data |
| RGA | ✓ | ✓ | — | data |
| YATA | ✓ | ✓ | — | data |

---

## Codegen Patterns

### Statement-to-Rust Mapping

| LOGOS Statement | Rust Output | Notes |
|-----------------|-------------|-------|
| `Let x be 5.` | `let x = 5;` | Immutable binding |
| `Let mutable x be 5.` | `let mut x = 5;` | Explicit mutability |
| `Set x to 10.` | `x = 10;` | Reassignment (requires mut) |
| `Set p's x to 5.` | `p.x = 5;` | Field mutation |
| `Set user's name to "X".` | `user.name.set("X")` | CRDT field (LWW/MV) |
| `If x > 0: ... Otherwise: ...` | `if x > 0 { ... } else { ... }` | Conditional |
| `While x > 0: ...` | `while x > 0 { check_preemption().await; ... }` | Loop + yielding |
| `Repeat for i in items: ...` | `for i in items.iter() { check_preemption().await; ... }` | Iteration |
| `Return 42.` | `return 42;` | Early exit |
| `Give x to f.` | `f(x)` | Move ownership |
| `Show x to f.` | `f(&x)` | Immutable borrow |
| `Push 1 to x.` | `x.push(1);` | Collection mutation |
| `Pop from x into y.` | `let y = x.pop();` | Collection removal |
| `Add 1 to set.` | `set.insert(1);` | Set insertion |
| `Remove 1 from set.` | `set.remove(&1);` | Set removal |
| `Set item 2 of x to 10.` | `x.logos_set(2i64, 10);` | Indexed mutation |
| `Inside a new zone called 'Z': ...` | `{ let Z = Zone::new_heap(1024*1024); ... }` | Arena scope |
| `Attempt all of: ...` | `tokio::join!( async { ... }, ... );` | Concurrent |
| `Simultaneously: ...` | `rayon::join( ... );` | Parallel |
| `Mount x at "path".` | `x.mount(vfs, "path").await;` | Persistence |
| `Sync x on "topic".` | `x.subscribe("topic").await;` | Network sync |
| `Listen on "/ip4/...".` | `network::listen("/ip4/...").await;` | Network bind |
| `Sleep 1000.` | `tokio::time::sleep(Duration::from_millis(1000)).await;` | Async sleep |
| `Launch a task to f.` | `tokio::spawn(async { f().await });` | Fire-and-forget |
| `Let h be Launch a task to f.` | `let h = tokio::spawn(async { f().await });` | Task handle |
| `Send x into pipe.` | `pipe.send(x).await;` | Channel send |
| `Receive y from pipe.` | `let y = pipe.recv().await;` | Channel receive |
| `Assert that x > 0.` | `assert!(x > 0);` | Runtime check |
| `Trust that x > 0 because "...".` | `debug_assert!(x > 0, "...");` | Documented assumption |
| `Check that user is admin.` | `assert!(user.is_admin(), "...");` | Security guard |

### Expression-to-Rust Mapping

| LOGOS Expression | Rust Output | Notes |
|-----------------|-------------|-------|
| `42` | `42` | Integer literal |
| `3.14` | `3.14f64` | Float literal |
| `"hello"` | `String::from("hello")` | Text literal → String |
| `true` / `false` | `true` / `false` | Boolean literal |
| `nothing` | `()` | Unit value |
| `'a'` | `'a'` | Character literal |
| `'\n'`, `'\t'`, etc. | `'\n'`, `'\t'` | Escape sequences |
| `5 seconds` | `Duration::from_nanos(5000000000u64)` | Duration literal |
| `2024-01-15` | `LogosDate(days_from_epoch)` | Date literal |
| `x` | `x` | Variable identifier |
| `(*boxed_x)` | `(*boxed_x)` | Boxed variable (recursive types) |
| `x + y` | `(x + y)` | Arithmetic operators |
| `x == y` | `(x == y)` | Comparison operators |
| `x && y` | `(x && y)` | Logical operators |
| `"hello" + " world"` | `format!("{}{}", "hello", " world")` | String concatenation |
| `x + "suffix"` | `format!("{}{}", x, "suffix")` | Mixed string concat |
| `f(x, y)` | `f(x, y)` | Synchronous call |
| `async_f(x, y)` | `async_f(x, y).await` | Async call (auto-detected) |
| `items at 3` | `LogosIndex::logos_get(&items, 3i64)` | Collection indexing (1-based) |
| `items from 1 through 3` | `&items[(1 - 1) as usize..3 as usize]` | Slice (1-indexed inclusive → 0-indexed exclusive) |
| `a copy of items` | `items.to_vec()` | Explicit clone to owned Vec |
| `the length of items` | `(items.len() as i64)` | Collection length (cast to i64) |
| `items contains x` | `items.logos_contains(&x)` | Unified contains (List/Set/Map/Text) |
| `set1 union set2` | `set1.union(&set2).cloned().collect::<HashSet<_>>()` | Set union |
| `set1 intersection set2` | `set1.intersection(&set2).cloned().collect::<HashSet<_>>()` | Set intersection |
| `the manifest of zone` | `FileSipper::from_zone(&zone).manifest()` | Sipping protocol manifest |
| `chunk 3 of zone` | `FileSipper::from_zone(&zone).get_chunk((3 - 1) as usize)` | Sipping protocol chunk (1-indexed) |
| `[1, 2, 3]` | `vec![1, 2, 3]` | List literal |
| `(1, "text", true)` | `vec![Value::from(1), Value::from("text"), Value::from(true)]` | Tuple (heterogeneous → Vec<Value>) |
| `1 through 10` | `(1..=10)` | Inclusive range |
| `point's x` | `point.x` | Field access |
| `synced_var's field` | `synced_var.get().await.field` | Field access on Distributed<T> |
| `a new Point` | `Point::default()` | Struct instantiation (default) |
| `a new Point with x: 10, y: 20` | `Point { x: 10, y: 20, ..Default::default() }` | Struct with fields |
| `a new Vec of Int` | `Vec::<i64>::default()` | Generic instantiation (turbofish) |
| `Shape::Circle` | `Shape::Circle` | Unit enum variant |
| `Shape::Circle with radius: 10` | `Shape::Circle { radius: 10 }` | Struct enum variant |
| `Tree::Node with val: 1, left: subtree` | `Tree::Node { val: 1, left: Box::new(subtree) }` | Recursive enum (auto-boxed) |

**Special Expression Rules**:
- **1-based indexing**: LOGOS collections use 1-based indexing (`items at 1` is first element), converted to 0-based for Rust
- **String concatenation**: String + String uses `format!()` macro, not `+` operator
- **Async calls**: `.await` automatically added when calling functions detected as async
- **Synced variables**: Field access on `Distributed<T>` becomes `.get().await.field`
- **Boxed fields**: Recursive struct/enum fields automatically wrapped in `Box::new()`
- **Clone on reuse**: Identifiers used multiple times in enum variant construction auto-clone except on last use

### Async Detection Rules

A statement or function is async if it contains ANY of:
- `Concurrent` block
- `Listen` or `ConnectTo` (networking)
- `Sleep` (delays)
- `Sync` or `Mount` (distributed)
- `ReadFrom { source: File }` or `WriteFile` (VFS)
- `LaunchTask`, `SendPipe`, `ReceivePipe`, `Select` (concurrency)
- `While` or `Repeat` (due to check_preemption)
- Calls to async functions

### VFS Injection Rules

VFS is injected into `main` if the program contains ANY of:
- `Mount` statement
- `ReadFrom { source: File }`
- `WriteFile`

Generated code:
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(NativeVfs::new("."));
    // user code
    Ok(())
}
```

### Distributed<T> Detection

A variable uses `Distributed<T>` if it has BOTH:
1. `Mount` statement → persistence
2. `Sync` statement → networking

Example:
```logos
Let counter be a new GCounter.
Mount counter at "data/counter.lsf".
Sync counter on "game-scores".
```

Generated:
```rust
let counter = Distributed::<GCounter>::mount(
    vfs.clone(),
    "data/counter.lsf",
    Some("game-scores".into())
).await?;
```

### Refinement Context Tracking

When a variable has a refinement type:
1. **At definition**: Emit `debug_assert!` for the predicate
2. **Register** the variable → (bound_var, predicate) in current scope
3. **On mutation** (Set): Re-emit the `debug_assert!`

Example:
```logos
Let positive: Int where it > 0 be 5.
Set positive to 10.
```

Generated:
```rust
let positive = 5;
debug_assert!(positive > 0);  // At definition
positive = 10;
debug_assert!(positive > 0);  // Re-check on mutation
```

### Ownership State Transitions

| Statement | Effect on Ownership |
|-----------|---------------------|
| `Let x be value.` | `x` owns `value` |
| `Give x to f.` | `x` moved into `f`, becomes invalid |
| `Show x to f.` | `x` borrowed immutably by `f`, remains valid |
| `Set x to value.` | `x` rebinds to new owner |
| `Return x.` | `x` moved to caller |

### Clone Insertion Rules

The compiler automatically inserts `.clone()` when:
- Passing a non-Copy value to multiple recipients (Show)
- Using a value multiple times in expressions
- Collection operations that need owned values

Example:
```logos
Show x to f.
Show x to g.
```

Generated:
```rust
f(&x);
g(&x);  // No clone needed, both borrows
```

---

## Edge Cases & Gotchas

### 1. Async Propagation (Transitive Detection)

**Issue**: Async detection is transitive. If a function calls an async function, it becomes async.

**Example**:
```logos
To helper:
    Sleep 100.

To main_logic:
    Call helper.  # main_logic is now async!
```

**Generated**:
```rust
async fn helper() {
    tokio::time::sleep(...).await;
}

async fn main_logic() {
    helper().await;  # Auto-detected async
}
```

**Gotcha**: Functions can become async even if they don't look async.

---

### 2. CRDT Field Mutation (`.set()` vs `=`)

**Issue**: CRDT fields (LWWRegister, MVRegister) require `.set()` instead of direct assignment.

**Example**:
```logos
A User has:
    Shared name, which is Text.

## Main:
Let user be a new User.
Set user's name to "Alice".
```

**Generated**:
```rust
struct User {
    name: LWWRegister<String>,
}

user.name.set("Alice".to_string());  // NOT user.name = "Alice"
```

**Gotcha**: LOGOS syntax `Set user's name to "Alice"` looks like field assignment but generates method call.

---

### 3. Refinement Re-checking on Set

**Issue**: Refinement types re-emit assertions on every mutation.

**Example**:
```logos
Let positive: Int where it > 0 be 5.
Set positive to 10.
Set positive to -1.  # Panics!
```

**Generated**:
```rust
let mut positive = 5;
debug_assert!(positive > 0);
positive = 10;
debug_assert!(positive > 0);
positive = -1;
debug_assert!(positive > 0);  // PANIC
```

**Gotcha**: Refinements are checked at every `Set`, not just definition.

---

### 4. Zone Scope Lifetimes (Hotel California)

**Issue**: Values allocated in a zone cannot escape to outer scopes.

**Example**:
```logos
Let outer be 0.
Inside a new zone called 'Z':
    Let inner be 42.
    Set outer to inner.  # Lifetime error!
```

**Why**: Zone-allocated values have lifetimes tied to the zone. Returning them would create dangling references.

**Solution**: Copy values out, don't move.

---

### 5. Boxed Enum Dereferencing

**Issue**: Recursive enum variants are boxed. Pattern matching bindings need `*` deref.

**Example**:
```logos
A Tree is either:
    A Leaf with value (Int).
    A Node with left (Tree) and right (Tree).

## Main:
Let tree be a new Node with left (new Leaf with value 1) and right (new Leaf with value 2).
Inspect tree:
    When Node (left: l) and (right: r):
        # l and r are Box<Tree>, auto-deref in expressions
```

**Generated**:
```rust
match tree {
    Tree::Node { left, right } => {
        let l = left;   // Box<Tree>
        let r = right;  // Box<Tree>
        // Auto-deref when used: *l
    }
}
```

**Gotcha**: Compiler handles this automatically, but understanding helps debug lifetime errors.

---

### 6. String Concatenation (`format!` vs `+`)

**Issue**: String concatenation uses `format!` macro, not `+` operator.

**Example**:
```logos
Let greeting be "Hello" combined with "World".
```

**Generated**:
```rust
let greeting = format!("{}{}", "Hello", "World");
```

**Why**: Rust's `+` operator consumes the left operand. `format!` is safer.

---

### 7. 1-Indexed Collections

**Issue**: LOGOS uses 1-based indexing (natural language), Rust uses 0-based.

**Example**:
```logos
Let numbers be [10, 20, 30].
Let first be item 1 of numbers.  # Gets 10, not 20
```

**Generated**:
```rust
let numbers = vec![10, 20, 30];
let first = numbers.logos_get(1i64);  // Converts to numbers[0]
```

**Gotcha**: `item 0 of numbers` panics! Always start from 1.

---

### 8. Ownership Transfer in Calls

**Issue**: Function calls transfer ownership by default (Give semantics).

**Example**:
```logos
To consume (data: Text):
    Print data.

## Main:
Let msg be "Hello".
Call consume with msg.
Show msg to show.  # Error: msg was moved
```

**Solution**: Use `Show` for borrowing, or clone explicitly.

---

### 9. Cooperative Yielding Overhead

**Issue**: All loops include `check_preemption().await`, which has overhead.

**Generated**:
```rust
while x > 0 {
    check_preemption().await;  // Checks every iteration
    x = x - 1;
}
```

**Performance**: For tight loops (millions of iterations), this adds ~5-10% overhead.

**Why**: Prevents thread starvation in concurrent systems. Worth the cost.

---

### 10. Inspect Exhaustiveness

**Issue**: Pattern matching without `Otherwise` must cover all variants.

**Example**:
```logos
A Shape is either:
    A Circle.
    A Square.

Inspect shape:
    When Circle:
        # Missing Square case - compiler error!
```

**Solution**: Always add `Otherwise:` for safety, or cover all variants.

---

## Appendices

### Appendix A: Test File Index

Quick reference for finding tests by category:

| Category | Test File | Test Count | Coverage |
|----------|-----------|------------|----------|
| Variables | `e2e_variables.rs` | 12 | 95% |
| Control Flow | `e2e_control_flow.rs` | 15 | 90% |
| Functions | `e2e_functions.rs` | 18 | 85% |
| Primitives | `e2e_primitives.rs` | 10 | 100% |
| Collections | `e2e_collections.rs` | 22 | 82% |
| Maps | `e2e_maps.rs` | 14 | 80% |
| Sets | `e2e_sets.rs` | 12 | 85% |
| Structs | `e2e_structs.rs` | 16 | 88% |
| Enums | `e2e_enums.rs` | 14 | 85% |
| Types | `e2e_types.rs` | 18 | 80% |
| Tuples | `e2e_tuples.rs` | 8 | 90% |
| Expressions | `e2e_expressions.rs` | 20 | 85% |
| Iteration | `e2e_iteration.rs` | 16 | 85% |
| Concurrency | `e2e_concurrency.rs` | 18 | 85% |
| Async | `e2e_async_cross_cutting.rs` | 31 | 90% |
| CRDTs | `e2e_crdt.rs` | 24 | 75% |
| Refinements | `e2e_refinement.rs` | 12 | 60% |
| Policy | `e2e_policy.rs` | 10 | 70% |
| Zones | `e2e_zones.rs` | 8 | 65% |
| Temporal | `e2e_temporal.rs` | 14 | 85% |
| Integration | `e2e_integration.rs` | 12 | 70% |
| Gossip | `e2e_gossip.rs` | 16 | 70% |
| Feature Matrix | `e2e_feature_matrix.rs` | 25 | 75% |
| Edge Cases | `e2e_edge_cases.rs` | 18 | 80% |
| **TOTAL** | **24 files** | **373 tests** | **82%** |

---

### Appendix B: AST Node Reference

Complete mapping of AST nodes to feature documentation:

**Statements** (`Stmt` enum):
1. `Let` → [Variables: Let](#feature-let-variable-binding)
2. `Set` → [Variables: Set](#feature-set-variable-mutation)
3. `SetField` → [Variables: SetField](#feature-setfield-field-mutation)
4. `SetIndex` → [Collections: SetIndex](#feature-setindex-index-mutation)
5. `Call` → [Functions: Call](#feature-call-function-call)
6. `If` → [Control Flow: If](#feature-if-conditional-execution)
7. `While` → [Control Flow: While](#feature-while-conditional-loop)
8. `Repeat` → [Control Flow: Repeat](#feature-repeat-iteration)
9. `Return` → [Control Flow: Return](#feature-return-early-exit)
10. `Assert` → [Verification: Assert](#feature-assert-runtime-assertion)
11. `Trust` → [Verification: Trust](#feature-trust-documented-assumption)
12. `RuntimeAssert` → [Verification: Assert](#feature-assert-runtime-assertion)
13. `Give` → [Ownership: Give](#feature-give-ownership-transfer)
14. `Show` → Ownership: Show (not yet documented)
15. `StructDef` → [Data Structures: StructDef](#feature-structdef-struct-definition)
16. `FunctionDef` → [Functions: FunctionDef](#feature-functiondef-function-definition)
17. `Inspect` → [Data Structures: Inspect](#feature-inspect-pattern-matching)
18. `Push` → Collections: Push
19. `Pop` → Collections: Pop
20. `Add` → Collections: Add
21. `Remove` → Collections: Remove
22. `Zone` → [Memory: Zone](#feature-zone-memory-arena)
23. `Concurrent` → [Concurrency: Concurrent](#feature-concurrent-async-block)
24. `Parallel` → Concurrency: Parallel
25. `ReadFrom` → [I/O: ReadFrom](#feature-readfrom-consolefile-read)
26. `WriteFile` → [I/O: WriteFile](#feature-writefile-file-write)
27. `Spawn` → Concurrency: Spawn (agents)
28. `SendMessage` → Concurrency: SendMessage
29. `AwaitMessage` → Concurrency: AwaitMessage
30. `MergeCrdt` → CRDTs: Merge
31. `IncreaseCrdt` → CRDTs: Increase
32. `DecreaseCrdt` → CRDTs: Decrease
33. `AppendToSequence` → CRDTs: Append
34. `ResolveConflict` → CRDTs: Resolve
35. `Check` → [Verification: Check](#feature-check-security-guard)
36. `Listen` → Networking: Listen
37. `ConnectTo` → Networking: Connect
38. `LetPeerAgent` → Networking: PeerAgent
39. `Sleep` → Concurrency: Sleep
40. `Sync` → CRDTs: Sync
41. `Mount` → CRDTs: Mount
42. `LaunchTask` → Concurrency: Launch
43. `LaunchTaskWithHandle` → Concurrency: LaunchWithHandle
44. `CreatePipe` → Concurrency: CreatePipe
45. `SendPipe` → Concurrency: SendPipe
46. `ReceivePipe` → Concurrency: ReceivePipe
47. `TrySendPipe` → Concurrency: TrySendPipe
48. `TryReceivePipe` → Concurrency: TryReceivePipe
49. `StopTask` → Concurrency: StopTask
50. `Select` → Concurrency: Select
51. `Theorem` → Verification: Theorem

**Expressions** (`Expr` enum):
1. `Literal` → Expressions: Literal
2. `Identifier` → Expressions: Identifier
3. `BinaryOp` → Expressions: BinaryOp
4. `Call` → [Functions: Call](#feature-call-function-call)
5. `Index` → Collections: Index
6. `Slice` → Collections: Slice
7. `Copy` → Collections: Copy
8. `Length` → Collections: Length
9. `Contains` → Collections: Contains
10. `Union` → Collections: Union
11. `Intersection` → Collections: Intersection
12. `ManifestOf` → Memory: ManifestOf
13. `ChunkAt` → Memory: ChunkAt
14. `List` → Collections: List
15. `Tuple` → Collections: Tuple
16. `Range` → Collections: Range
17. `FieldAccess` → Data Structures: FieldAccess
18. `New` → Data Structures: New
19. `NewVariant` → Data Structures: NewVariant

---

### Appendix C: Parser Modes

LOGOS has two parsing modes:

**Mode 1: LOGOS (Imperative)**
- Enabled for blocks starting with `##`
- Parses statements (Stmt AST)
- Generates Rust code via codegen
- Focus: Executable programs

**Mode 2: Logic Kernel**
- Enabled for Theorem blocks
- Parses propositions (LogicExpr AST)
- Generates SMT-LIB for Z3
- Focus: Proof verification

**Bridge**: `Assert` statements accept `LogicExpr` in Mode 1, connecting both modes.

---

## Summary Statistics

| Metric | Count |
|--------|-------|
| **Total Statement Types** | 52 |
| **Total Expression Types** | 19 |
| **Total CRDT Types** | 9 |
| **Total Core Types** | 8 |
| **Total Temporal Types** | 5 |
| **Standard Library Functions** | 60+ |
| **Test Files** | 24 |
| **Total Tests** | 373 |
| **Overall Coverage** | 82% |
| **Critical Gaps (P0)** | 4 |
| **High Priority Gaps (P1)** | 12 |
| **Medium Priority Gaps (P2)** | 4 |
| **Platform Support (Native)** | 100% |
| **Platform Support (WASM)** | ~40% |

---

**End of FEATURE_MATRIX.md v0.1.0**

Last Updated: 2026-02-05
Generated from: logicaffeine codebase commit 3b67ccb

For questions or contributions, see: [CONTRIBUTING.md](CONTRIBUTING.md)
