# Imperative mode — the LOGOS language

Imperative LOGOS is a statically-typed programming language written in English. The same source
runs through an interpreter, a bytecode VM, and a JIT, or compiles ahead-of-time to Rust or
directly to WebAssembly (see [Execution & performance](execution-and-performance.md)).

```rust
let rust = logicaffeine_compile::compile_to_rust(source)?;   // English → Rust
```

Source of truth: the parser and AST in
[`crates/logicaffeine_language/src/ast/stmt.rs`](../crates/logicaffeine_language/src/ast/stmt.rs),
the code generators in
[`crates/logicaffeine_compile/src/codegen/`](../crates/logicaffeine_compile/src/codegen/), and the
`e2e_*` tests in [`crates/logicaffeine_tests/tests/`](../crates/logicaffeine_tests/tests/). Every
snippet below is taken from a passing test.

## Program structure

A program is made of `##` blocks. `## Main` is the entry point; `## To <name> …` defines a function;
`## A <Name> has:` / `## A <Name> is one of:` define types.

```logos
## Main
Let x be 5.
Set x to 10.
Show x.
```
→ prints `10`.

## Variables & mutation

`Let <name> be <expr>` binds a variable; `Set <name> to <expr>` mutates it. Type annotations are
optional (`Let x: Int be 5`); types are otherwise inferred.

## Primitive types & operators

`Int` (i64), `Nat` (u64), `Real` (f64; `Float` is an alias), `Rational` (exact fractions), `Bool`,
`Text`, `Char`, `Byte` (u8), the machine words `Word8`/`Word16`/`Word32`/`Word64` (wrapping
ℤ/2ⁿ ring arithmetic — the crypto substrate), and `Uuid` (RFC 9562, every version from nil/max
through v1–v8). Operators: arithmetic `+ - * / %` (also the words `plus`, `minus`,
`times`, `divided by`); comparisons `== != < > <= >=` (also `is less than`, `is at most`, …); logical
`and` / `or` / `not`; bitwise shifts and xor. (`e2e_primitives`, `e2e_expressions`,
`e2e_comparisons`, `e2e_operators`, `phase_bitwise`, `e2e_codegen_uuid`, `e2e_md5_logos`.)

## Functions

```logos
## To classify (n: Int) -> Text:
    If n is less than 0:
        Return "negative".
    Return "non-negative".
```
compiles to:
```rust
#[inline]
fn classify(n: i64) -> String {
    if (n < 0) {
        return String::from("negative");
    }
    return String::from("non-negative");
}
```

Parameters are typed; return types are explicit (`-> T`) or inferred. Multiple parameters use
`and`: `## To add (a: Int) and (b: Int) -> Int:`. Recursion, closures, generics, and FFI
(`native "…"`, `is exported for native`/`wasm`) are all supported. (`e2e_functions`,
`phase32_functions`, `e2e_closures`, `phase34_generics`, `e2e_recursive_factorial`.)

## Collections — 1-indexed

English speakers count from one, and so does LOGOS. Slices are inclusive on both ends.

```logos
## Main
Let items be [1, 2].
Push 3 to items.
Let first be item 1 of items.        # → 1
Let middle be items 2 through 4.     # inclusive slice
Set item 2 of items to 99.
Show item 3 of items.
```

`Seq` (sequences), `Set`, `Map`, and `Tuple` are all available, with `length of`, `Push`/`Pop`,
`Add`/`Remove`, and map/tuple indexing. (`e2e_collections`, `e2e_sets`, `e2e_maps`, `e2e_tuples`.)

## Control flow

```logos
## Main
Let sum be 0.
Repeat for x in [1, 2, 3]:
    Set sum to sum + x.
Show sum.                            # → 6
```

`If … Otherwise …`, `While …` (with an optional `(decreasing <expr>)` termination measure),
`Repeat for <x> in <collection>`, `Repeat for i from 1 to n` (inclusive), `Break`, and `Return`.
(`e2e_control_flow`, `e2e_iteration`, `phase30_iteration`.)

## Structs

```logos
## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point.
Show p's x.
```

Field access and mutation use the possessive: `p's x`, `Set p's x to 100`. (`e2e_structs`,
`phase31_structs`.)

## Enums & pattern matching

```logos
## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Red.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue".
```

`Inspect … When …` matches enum variants with exhaustiveness checking. (`e2e_enums`,
`phase33_enums`.)

## Ownership & borrowing

LOGOS tracks ownership. `copy of <x>` makes an explicit copy (used throughout the
[merge-sort example](../README.md#a-complete-example-merge-sort)); move/borrow verbs and
use-after-move detection are handled by the ownership analysis. (`phase21_ownership`,
[`analysis/ownership.rs`](../crates/logicaffeine_compile/src/analysis/ownership.rs).)

## I/O & assertions

`Show <expr>` prints; `Read … from console`/`from file "…"` reads; `Write "…" to file "…"` writes.
`Require that <cond>` / `Assert that <logic>` connect to the logic side — `Assert that …` is the
**Assert Bridge** into [Proof & verification](proof-and-verification.md). (`phase10_io`,
`phase25_assertions`.)

## Standard library

A small standard library ships with the compiler
([`assets/std/`](../crates/logicaffeine_compile/assets/std/)) — modules for core types plus `io`,
`file`, `time`, `random`, `env`, `crdt`, `concurrency`, `net`, `crypto`, and `uuid`, exposing
functions like `now`, `sleep`, `randomInt`, and `randomFloat`.

The `crypto` and `uuid` modules are themselves written in LOGOS:
[`crypto.lg`](../crates/logicaffeine_compile/assets/std/crypto.lg) carries the post-quantum
ML-KEM-768 building blocks (NTT with SIMD lanes, Montgomery/Barrett reduction) and ChaCha20, and
[`uuid.lg`](../crates/logicaffeine_compile/assets/std/uuid.lg) implements RFC 9562 UUIDs with the
MD5 and SHA-1 digests as LOGOS source — proven bit-exact against FIPS and reference-crate oracles
across the execution tiers (`mlkem768_logos`, `mlkem768_oracle`, `e2e_codegen_uuid`,
`e2e_uuid_logos_gen`).

Imports are **demand-driven and invisible**:
[`apply_prelude`](../crates/logicaffeine_compile/src/loader.rs) prepends a module only when your
program references one of its names *and* doesn't define them itself ("declarer wins", so your own
`Message`/`args` is never shadowed). Programs that touch no stdlib vocabulary compile byte-for-byte
unchanged, and the same seam serves both the interpreter and the AOT compiler. Opt out entirely with
a `## NoPrelude` marker.

## Concurrency

Structured concurrency, channels, agents, and CRDTs are part of the language too — see
[Concurrency & distributed](concurrency.md).

## See also

- How it runs and how fast → [Execution & performance](execution-and-performance.md)
- The compiler crate → [`logicaffeine_compile`](../crates/logicaffeine_compile/README.md)

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
