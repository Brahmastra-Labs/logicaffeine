# logicaffeine-base

Pure structural atoms for the Logicaffeine ecosystem.

## Overview

`logicaffeine-base` provides the foundational types used throughout Logicaffeine:

- **Arena allocation** for stable AST references
- **Symbol interning** for efficient string comparison
- **Span tracking** for error reporting
- **Base error types** with source locations

### Design Philosophy

This crate is intentionally minimal:

- **Zero vocabulary knowledge** - no lexicon, no English parsing
- **No IO operations** - pure computational types only
- **Single dependency** - only `bumpalo` for arena allocation
- **WASM-compatible** - no platform-specific code

## Core Types

### Arena\<T\>

Bump allocator wrapper providing stable references for AST nodes during parsing.

```rust
use logicaffeine_base::Arena;

// Create an arena for AST nodes
let arena: Arena<MyNode> = Arena::new();

// Allocate values - references remain valid for arena's lifetime
let node = arena.alloc(MyNode { value: 42 });
let slice = arena.alloc_slice([1, 2, 3]);

// Reset for REPL loops (invalidates all references, keeps capacity)
// arena.reset();
```

**Methods:**
- `new()` - Create a new arena
- `alloc(value)` - Allocate a single value, returns `&T`
- `alloc_slice(iter)` - Allocate a slice from an iterator, returns `&[T]`
- `reset()` - Reset arena for reuse (invalidates all references)

### Symbol & Interner

String interning for O(1) equality comparison. Strings are stored once and referenced by integer handles.

```rust
use logicaffeine_base::{Interner, Symbol, SymbolEq};

let mut interner = Interner::new();

// Intern strings - same string returns same symbol
let hello = interner.intern("hello");
let hello2 = interner.intern("hello");
assert_eq!(hello, hello2);  // O(1) comparison

// Resolve back to string
assert_eq!(interner.resolve(hello), "hello");

// Check equality with SymbolEq trait
assert!(hello.is(&interner, "hello"));

// Lookup without creating new entry
assert!(interner.lookup("hello").is_some());
assert!(interner.lookup("unknown").is_none());
```

**Symbol methods:**
- `EMPTY` - Constant for the empty string symbol
- `index()` - Get the internal index
- `is(interner, str)` - Check if symbol equals a string (via `SymbolEq` trait)

**Interner methods:**
- `new()` - Create a new interner
- `intern(str)` - Intern a string, returns `Symbol`
- `resolve(symbol)` - Get the string for a symbol
- `lookup(str)` - Look up existing interned string without creating entry
- `len()` / `is_empty()` - Query interned count

### Span

Byte offset ranges for source location tracking and error reporting.

```rust
use logicaffeine_base::Span;

// Create a span from byte offsets
let span = Span::new(10, 25);
assert_eq!(span.start, 10);
assert_eq!(span.end, 25);
assert_eq!(span.len(), 15);

// Merge spans (e.g., for compound expressions)
let first = Span::new(0, 10);
let second = Span::new(15, 25);
let merged = first.merge(second);
assert_eq!(merged, Span::new(0, 25));

// Check if empty
assert!(!span.is_empty());
assert!(Span::new(5, 5).is_empty());
```

**Methods:**
- `new(start, end)` - Create span from byte offsets
- `merge(other)` - Combine two spans into one covering both
- `len()` - Length in bytes
- `is_empty()` - Check if span has zero length

### SpannedError & Result

Base error type with source location for precise error reporting.

```rust
use logicaffeine_base::{SpannedError, Span, Result};

fn parse_number(input: &str, span: Span) -> Result<i32> {
    input.parse().map_err(|_| {
        SpannedError::new("invalid number", span)
    })
}

// Error displays with location
let err = SpannedError::new("unexpected token", Span::new(5, 10));
println!("{}", err);  // "unexpected token at 5..10"
```

**SpannedError fields:**
- `message: String` - The error message
- `span: Span` - Source location

**Methods:**
- `new(message, span)` - Create a new error

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
logicaffeine-base = "0.6"
```

Basic usage combining types:

```rust
use logicaffeine_base::{Arena, Interner, Span, SpannedError, Result};

struct Token<'a> {
    text: &'a str,
    span: Span,
}

fn tokenize<'a>(arena: &'a Arena<Token<'a>>, input: &str) -> Result<&'a [Token<'a>]> {
    // ... tokenization logic using arena allocation
    Ok(arena.alloc_slice(vec![]))
}
```

## Testing

```bash
cargo test -p logicaffeine-base
```

## License

BUSL-1.1
