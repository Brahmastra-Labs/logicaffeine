# Extreme Testing Plan - Going Beyond the Basics

## User's Key Insights
1. **Deep nesting**: Test `Seq of Seq of Seq of...` to extreme levels
2. **No corner cutting**: PROPER FULL fixes, not workarounds
3. **Beyond tuples**: What other destructuring patterns do we need?

## 1. Deeply Nested Generics (Parentheses Bug)

### Current Limitation
```logos
Let matrix be a new Seq of (Seq of Int).  // ❌ Parser doesn't support parentheses
```

### Extreme Test Cases Needed
```logos
// 3-level nesting
Let cube be a new Seq of Seq of Seq of Int.

// 4-level nesting
Let hypercube be a new Seq of Seq of Seq of Seq of Int.

// Mixed nesting
Let complex be a new Map of Text to Seq of Map of Text to Int.

// Function returning deeply nested type
## To makeMatrix -> Seq of Seq of Int:
    ...

// Struct with deeply nested field
## A Tensor has:
    A data: Seq of Seq of Seq of Float.
```

### Proper Fix Strategy
Instead of accepting parentheses limitation, we should:
1. Make `consume_field_type()` fully recursive
2. Support arbitrary nesting depth
3. Test up to 5+ levels of nesting

---

## 2. Loop Destructuring - Beyond Tuples

### Current: Only Tuple (k, v) Supported
```logos
Repeat for (k, v) in map:  // Tuple destructuring
```

### What Else Could We Destructure?

#### Struct Destructuring
```logos
## A Point has x: Int, y: Int.
Let points be [Point(1, 2), Point(3, 4)].

Repeat for Point(x, y) in points:  // Destructure struct in loop
    Show x + y.
```

#### Enum Variant Matching in Loops
```logos
## A Result is one of:
    Success with value Int.
    Failure with error Text.

Let results be [Success(10), Failure("err"), Success(20)].

Repeat for Success(val) in results:  // Only iterate over Success variants
    Show val.
```

#### Nested Destructuring
```logos
Let pairs be [((1, 2), (3, 4)), ((5, 6), (7, 8))].

Repeat for ((a, b), (c, d)) in pairs:  // Nested tuple destructuring
    Show a + b + c + d.
```

#### Array/Slice Patterns
```logos
Repeat for [first, second, rest...] in chunks:  // Head/tail pattern
    ...
```

---

## 3. Give Keyword - Complete Coverage

### Current Fix: Basic Give
```logos
Call consume with Give items  // ✓ FIXED
```

### Extreme Cases to Test

#### Give with Complex Expressions
```logos
// Give field access
Call process with Give user's data

// Give nested field
Call save with Give config's settings's timeout

// Give from collection
Call handle with Give item 1 of list

// Give result of call
Call transform with Give makeData()

// Give struct literal
Call send with Give a new Message with content "hi"
```

#### Give with Ownership Chain
```logos
Let x be [1, 2, 3].
Call first with Give x.
// x is now moved - should error if used again
Show length of x.  // Should fail - use after move
```

#### Give Multiple Times (Should Fail)
```logos
Let data be [1, 2, 3].
Call first with Give data.
Call second with Give data.  // Should fail - already moved
```

---

## 4. Map Iteration - Complete Fix

### Current Issue: No Destructuring
```logos
Repeat for (k, v) in map:  // ❌ Not supported
```

### Alternative Patterns Users Might Want

#### Iterate Keys Only
```logos
Repeat for k in keys of scores:
    Show k.
```

#### Iterate Values Only
```logos
Repeat for v in values of scores:
    Show v.
```

#### Iterate with Index
```logos
Repeat for (i, x) in enumerate(items):
    Show i.
    Show x.
```

#### Iterate Pairs
```logos
Repeat for pair in entries of scores:
    Show pair's key.
    Show pair's value.
```

---

## 5. Comprehensive Test Suite Structure

### Test File: `e2e_extreme_tests.rs`

```rust
// Deep Nesting Tests
#[test] fn extreme_3_level_seq() { ... }
#[test] fn extreme_4_level_seq() { ... }
#[test] fn extreme_5_level_seq() { ... }
#[test] fn extreme_mixed_map_seq() { ... }

// Give Ownership Tests
#[test] fn give_field_access() { ... }
#[test] fn give_nested_field() { ... }
#[test] fn give_from_call() { ... }
#[test] fn give_struct_literal() { ... }
#[test] fn give_use_after_move_should_fail() { ... }

// Map Iteration Tests
#[test] fn map_iter_keys_only() { ... }
#[test] fn map_iter_values_only() { ... }
#[test] fn map_iter_with_destructure() { ... }

// Struct Destructuring Tests
#[test] fn struct_destructure_in_loop() { ... }
#[test] fn nested_destructure_in_loop() { ... }
```

---

## 6. Codegen Considerations

### Deep Nesting - Type Inference
```rust
// 3 levels
Vec<Vec<Vec<i64>>>

// 4 levels
Vec<Vec<Vec<Vec<i64>>>>

// Mixed
HashMap<String, Vec<HashMap<String, i64>>>
```

**Challenge**: Ensure proper generic parameter propagation at ALL levels

### Loop Destructuring - Match vs For
```rust
// For tuple: Can use Rust's for loop directly
for (k, v) in map {
    ...
}

// For struct: Needs destructuring in loop body
for item in items {
    let Point { x, y } = item;
    ...
}
```

---

## 7. Priority Order for Implementation

1. **HIGH**: Fix parentheses in types (enables deep nesting)
2. **HIGH**: Complete Give ownership semantics (use-after-move detection)
3. **MEDIUM**: Map iteration destructuring (tuple pattern)
4. **MEDIUM**: Deep nesting tests (3-5 levels)
5. **LOW**: Struct destructuring in loops (nice-to-have)
6. **LOW**: Enum filtering in loops (nice-to-have)

---

## Next Steps

1. ✅ Give keyword - DONE
2. ⏭️ Add extreme depth tests for current features
3. ⏭️ Fix parentheses in type syntax
4. ⏭️ Implement tuple destructuring for loops
5. ⏭️ Test ownership semantics thoroughly
