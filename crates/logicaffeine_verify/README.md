# logicaffeine-verify

Z3-based static verification engine with Socratic error messages.

## The Tarski Invariant

This crate has **NO dependency on the main AST**. The Intermediate Representation (IR) is self-contained. Translation from `LogicExpr` to `VerifyExpr` happens in the compile layer, keeping verification decoupled from parsing.

## Overview

A static verification engine that uses the Z3 SMT solver to prove properties at compile time:

- Detect contradictions before runtime
- Prove bounds and refinement types
- Generate counter-examples when verification fails
- Socratic error messages for pedagogical feedback

This is a premium feature requiring a Pro, Premium, Lifetime, or Enterprise license.

### Smart Full Mapping Strategy

The verification IR uses a "smart full mapping" approach:

| Type | Z3 Encoding | Description |
|------|-------------|-------------|
| `Int` | IntSort | 64-bit integers with full arithmetic |
| `Bool` | BoolSort | Boolean values with logical operations |
| `Object` | Uninterpreted | Domain entities (opaque to Z3) |
| Predicates | `Apply` | Uninterpreted functions |
| Modals | `Apply` | Uninterpreted functions |
| Temporals | `Apply` | Uninterpreted functions |

Z3 reasons about structure without semantic knowledge. For example, given `Possible(A) -> Possible(B)` and `Possible(A)`, Z3 deduces `Possible(B)` via modus ponens.

## Quick Start

```rust
use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};

// Create a verification session
let mut session = VerificationSession::new();

// Declare variables with types
session.declare("x", VerifyType::Int);

// Add assumptions (constraints)
session.assume(&VerifyExpr::eq(
    VerifyExpr::var("x"),
    VerifyExpr::int(10),
));

// Verify assertions
session.verify(&VerifyExpr::gt(
    VerifyExpr::var("x"),
    VerifyExpr::int(5),
))?; // Succeeds: 10 > 5
```

### Uninterpreted Function Example

```rust
use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyType};

let mut session = VerificationSession::new();
session.declare("socrates", VerifyType::Object);

// Mortal(x) -> Human(x)
session.assume(&VerifyExpr::implies(
    VerifyExpr::apply("Mortal", vec![VerifyExpr::var("socrates")]),
    VerifyExpr::apply("Human", vec![VerifyExpr::var("socrates")]),
));

// Mortal(socrates)
session.assume(&VerifyExpr::apply("Mortal", vec![VerifyExpr::var("socrates")]));

// Z3 deduces: Human(socrates)
session.verify(&VerifyExpr::apply("Human", vec![VerifyExpr::var("socrates")]))?;
```

## Public API

```toml
[dependencies]
logicaffeine-verify = { path = "../logicaffeine_verify" }
```

Re-exports from `lib.rs`:

| Export | Description |
|--------|-------------|
| `VerificationError` | Error type with Socratic explanations |
| `VerificationErrorKind` | Error kind enum |
| `VerificationResult<T>` | `Result<T, VerificationError>` |
| `VerifyExpr` | Verification IR expression |
| `VerifyOp` | Binary operation enum |
| `VerifyType` | Type declarations (Int, Bool, Object) |
| `LicensePlan` | License tier enum |
| `LicenseValidator` | License validation with caching |
| `Verifier` | Low-level Z3 wrapper |
| `VerificationSession` | High-level verification API |

## Core Types

### VerifyType

| Variant | Z3 Sort | Description |
|---------|---------|-------------|
| `Int` | IntSort | 64-bit integers |
| `Bool` | BoolSort | Boolean values |
| `Object` | Uninterpreted | Domain entities |

### VerifyOp

| Variant | Category | Description |
|---------|----------|-------------|
| `Add`, `Sub`, `Mul`, `Div` | Arithmetic | Integer operations (Int -> Int) |
| `Eq`, `Neq` | Equality | Equality comparison (any -> Bool) |
| `Gt`, `Lt`, `Gte`, `Lte` | Comparison | Relational operators (Int -> Bool) |
| `And`, `Or`, `Implies` | Logic | Boolean connectives (Bool -> Bool) |

### VerifyExpr

| Variant | Description |
|---------|-------------|
| `Int(i64)` | Integer literal |
| `Bool(bool)` | Boolean literal |
| `Var(String)` | Named variable reference |
| `Binary { op, left, right }` | Binary operation |
| `Not(Box<VerifyExpr>)` | Logical negation |
| `ForAll { vars, body }` | Universal quantifier: forall x: T. P(x) |
| `Exists { vars, body }` | Existential quantifier: exists x: T. P(x) |
| `Apply { name, args }` | Uninterpreted function application |

### VerifyExpr Builder Methods

```rust
// Literals
VerifyExpr::var("x")      // Variable reference
VerifyExpr::int(42)       // Integer literal
VerifyExpr::bool(true)    // Boolean literal

// Operations
VerifyExpr::binary(op, left, right)  // Binary operation
VerifyExpr::not(expr)                // Logical negation
VerifyExpr::apply(name, args)        // Uninterpreted function

// Quantifiers
VerifyExpr::forall(vars, body)       // Universal: forall x. P(x)
VerifyExpr::exists(vars, body)       // Existential: exists x. P(x)

// Comparison shortcuts
VerifyExpr::eq(left, right)          // left == right
VerifyExpr::neq(left, right)         // left != right
VerifyExpr::gt(left, right)          // left > right
VerifyExpr::lt(left, right)          // left < right
VerifyExpr::gte(left, right)         // left >= right
VerifyExpr::lte(left, right)         // left <= right

// Logic shortcuts
VerifyExpr::and(left, right)         // left && right
VerifyExpr::or(left, right)          // left || right
VerifyExpr::implies(left, right)     // left -> right
```

## Modules

| Module | Purpose |
|--------|---------|
| `lib.rs` | Public API re-exports |
| `ir.rs` | Intermediate representation (VerifyExpr, VerifyType, VerifyOp) |
| `solver.rs` | Z3 wrapper (Verifier, VerificationSession) |
| `license.rs` | Stripe license validation with caching |
| `error.rs` | Error types with Socratic explanations |

## VerificationSession API

| Method | Description |
|--------|-------------|
| `new()` | Create a new verification session |
| `declare(name, ty)` | Declare a variable with a type |
| `assume(expr)` | Add an assumption (constraint) |
| `verify(expr)` | Verify an assertion is valid given assumptions |
| `verify_with_binding(var, ty, value, pred)` | Verify with temporary variable binding |

## Verifier (Low-Level API)

| Method | Description |
|--------|-------------|
| `new()` | Create verifier with 10-second timeout |
| `check_bool(value)` | Verify a boolean is always true |
| `check_int_greater_than(value, bound)` | Verify value > bound |
| `check_int_less_than(value, bound)` | Verify value < bound |
| `check_int_equals(left, right)` | Verify left == right |
| `context()` | Get a VerificationContext for complex proofs |

## License Validation

Verification is a premium feature gated by license.

### LicensePlan

| Plan | Can Verify |
|------|------------|
| `None` | No |
| `Free` | No |
| `Supporter` | No |
| `Pro` | Yes |
| `Premium` | Yes |
| `Lifetime` | Yes |
| `Enterprise` | Yes |

### Validation Flow

1. License key format: Stripe subscription ID (`sub_*`)
2. Validation endpoint: `https://api.logicaffeine.com/validate`
3. Request: `{ "licenseKey": "<key>" }`
4. Response: `{ valid: bool, plan: Option<String>, error: Option<String> }`
5. Results cached for 24 hours at `{cache_dir}/logos/verification_license.json`
6. Stale cache used as fallback when network unavailable

### LicenseValidator API

| Method | Description |
|--------|-------------|
| `new()` | Create validator with default cache path |
| `validate(key)` | Validate key, returns `VerificationResult<LicensePlan>` |

## Error Types

### VerificationErrorKind

| Kind | Description |
|------|-------------|
| `ContradictoryAssertion` | Assertion can never be true |
| `BoundsViolation { var, expected, found }` | Variable violates declared bounds |
| `RefinementViolation { type_name }` | Value doesn't satisfy refinement type |
| `LicenseRequired` | No license key provided |
| `LicenseInvalid { reason }` | Invalid license key format or expired |
| `LicenseInsufficientPlan { current }` | Plan doesn't include verification |
| `SolverUnknown` | Z3 timeout or undecidable |
| `SolverError { message }` | Internal Z3 error |
| `TerminationViolation { variant, reason }` | Loop termination cannot be proven |

### VerificationError Factory Methods

```rust
VerificationError::license_required()
VerificationError::license_invalid(reason)
VerificationError::insufficient_plan(current)
VerificationError::contradiction(explanation, counterexample)
VerificationError::bounds_violation(var, expected, found)
VerificationError::refinement_violation(type_name, explanation)
VerificationError::solver_unknown()
VerificationError::solver_error(message)
VerificationError::termination_violation(variant, reason)

// Builder method
error.with_span(start, end)
```

### CounterExample

```rust
pub struct CounterExample {
    pub assignments: Vec<(String, String)>,
}
```

Shows variable assignments that make an assertion false.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `z3` | SMT solver bindings |
| `serde` | Serialization for license cache |
| `serde_json` | JSON encoding for cache files |
| `ureq` | HTTP client for license API |
| `dirs` | Cross-platform cache directory |

## Z3 Setup

### macOS (Homebrew)

```bash
brew install z3

# Set environment variables for building
export Z3_SYS_Z3_HEADER=/opt/homebrew/include/z3.h
export BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/include"
export LIBRARY_PATH="/opt/homebrew/lib"
```

### Linux (apt)

```bash
sudo apt install z3 libz3-dev
```

### Building

```bash
cargo build -p logicaffeine-verify
```

## License

BUSL-1.1
