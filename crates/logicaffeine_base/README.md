# logicaffeine-base

Foundational infrastructure for the [Logicaffeine](https://logicaffeine.com) ecosystem. This crate provides the low-level building blocks—arena allocation, string interning, source spans, and error handling—that all other logicaffeine crates depend on.

## Overview

| Module | Purpose |
|--------|---------|
| `Arena<T>` | Bump allocation for AST nodes with stable references |
| `Interner`/`Symbol` | String interning with O(1) equality comparison |
| `Span` | Source location tracking (byte offsets) |
| `SpannedError`/`Result<T>` | Error handling with source positions |

## Installation

```toml
[dependencies]
logicaffeine-base = "0.6"
```

## Quick Start

```rust
use logicaffeine_base::{Arena, Interner, Symbol, Span, SpannedError, Result};

// Arena for bump allocation
let arena: Arena<&str> = Arena::new();
let value = arena.alloc("hello");
assert_eq!(*value, "hello");

// Interner for string deduplication
let mut interner = Interner::new();
let sym1 = interner.intern("hello");
let sym2 = interner.intern("hello");
assert_eq!(sym1, sym2);  // O(1) comparison

// Span for source tracking
let span = Span::new(0, 5);
assert_eq!(span.len(), 5);

// Error with location
let err = SpannedError::new("unexpected token", span);
assert_eq!(err.to_string(), "unexpected token at 0..5");
```

## Module Reference

### Arena

Bump allocation for stable AST references. All values live until the arena is dropped or reset.

```rust
use logicaffeine_base::Arena;

let arena: Arena<String> = Arena::new();

// Allocate single value
let s = arena.alloc("hello".to_string());

// Allocate slice from iterator
let nums = arena.alloc_slice([1, 2, 3]);
```

| Method | Description |
|--------|-------------|
| `Arena::new()` | Create empty arena |
| `arena.alloc(value)` | Allocate value, return stable reference |
| `arena.alloc_slice(iter)` | Allocate slice from `ExactSizeIterator` |
| `arena.reset()` | Clear arena, reuse capacity (REPL-friendly) |

### Interner / Symbol

String interning for O(1) equality. Each unique string is stored once; comparing symbols is just comparing integers.

```rust
use logicaffeine_base::{Interner, Symbol, SymbolEq};

let mut interner = Interner::new();

let hello = interner.intern("hello");
let world = interner.intern("world");

// O(1) equality
assert_ne!(hello, world);

// Resolve back to string
assert_eq!(interner.resolve(hello), "hello");

// SymbolEq trait for convenience
assert!(hello.is(&interner, "hello"));
```

| Method | Description |
|--------|-------------|
| `Interner::new()` | Create interner (empty string pre-interned) |
| `interner.intern(s)` | Get or create symbol for string |
| `interner.resolve(sym)` | Get original string from symbol |
| `interner.lookup(s)` | Non-interning lookup, returns `Option<Symbol>` |
| `interner.len()` | Count of interned strings |
| `Symbol::EMPTY` | Pre-interned empty string constant |
| `symbol.index()` | Internal index for dense storage |

### Span

Byte-offset range in source text. Matches Rust's string slicing: `&source[span.start..span.end]`.

```rust
use logicaffeine_base::Span;

let source = "hello world";
let hello = Span::new(0, 5);
let world = Span::new(6, 11);

assert_eq!(&source[hello.start..hello.end], "hello");

// Merge spans for compound expressions
let full = hello.merge(world);
assert_eq!(full.start, 0);
assert_eq!(full.end, 11);
```

| Method | Description |
|--------|-------------|
| `Span::new(start, end)` | Create from byte offsets |
| `span.merge(other)` | Combine two spans (min start, max end) |
| `span.len()` | Length in bytes |
| `span.is_empty()` | True if zero-length |
| `span.start` / `span.end` | Public fields for direct access |

### SpannedError / Result

Errors annotated with source location. Implements `std::error::Error`.

```rust
use logicaffeine_base::{SpannedError, Span, Result};

fn parse_number(s: &str) -> Result<i32> {
    s.parse().map_err(|_| SpannedError::new(
        format!("invalid number: '{}'", s),
        Span::new(0, s.len()),
    ))
}

let err = parse_number("abc").unwrap_err();
// Display: "invalid number: 'abc' at 0..3"
```

| Type | Description |
|------|-------------|
| `SpannedError` | Error with `message: String` and `span: Span` |
| `Result<T>` | Alias for `std::result::Result<T, SpannedError>` |

## Design Principles

- **No vocabulary knowledge**: This crate knows nothing about English or natural language
- **No I/O**: Pure data structures only
- **Minimal dependencies**: Only `bumpalo` for arena allocation
- **Foundation layer**: All other logicaffeine crates build on these types

## Dependencies

```toml
bumpalo = "3.19"
```

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.
