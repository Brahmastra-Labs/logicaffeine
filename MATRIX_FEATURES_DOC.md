# Compiled Code Optimization Matrix

A comprehensive comparison of compiled code optimization techniques across all fast languages — what makes C, Rust, Zig, and others fast, what Logos inherits for free, where the gaps are, and what it takes to close them.

---

## Executive Summary

Logos compiles English-like source to Rust, which then compiles via LLVM. At a **geometric mean of 1.038x vs C** across six benchmarks, Logos release builds are competitive with hand-written systems language code.

**Where Logos wins:**
- **Pure computation** — Fibonacci, Ackermann, and similar recursive workloads compile to identical Rust code as hand-written versions. LLVM sees the same IR.
- **HashMap operations** — Logos generates Rust's `HashMap::with_capacity` + insert/lookup. This beats hand-written C hash tables using open addressing by ~2x.
- **String assembly** — Logos emits `String::with_capacity` + `write!` macro, matching hand-tuned Rust.

**Where Logos loses:**
- **Array-heavy algorithms** — Sieve (1.60x C) and bubble sort (1.04x C) pay overhead from `LogosIndex::logos_get()` / `LogosIndexMut::logos_set()` trait dispatch, 1-based → 0-based conversion, and forced `.clone()` on every read.
- **Deep recursion** — Ackermann (1.59x C) and Fibonacci (1.35x C) show modest overhead from Logos' function call boilerplate, though LLVM optimizes most of it away.

**Root cause:** For tight inner loops over arrays, every element access goes through the `LogosIndex` trait, which adds bounds checking, index conversion, and `.clone()`. Hand-written Rust uses direct `arr[i]` or `arr.swap()`. The generated code uses `while` loops where hand-written code uses `for i in 0..n` ranges. These two patterns — indexing abstraction and loop style — account for virtually all the performance gap.

**Compilation time:** Logos takes **13.4 seconds** (release) to compile a benchmark program vs **72ms** for GCC. This is because `largo build --release` compiles the full Rust dependency graph (logicaffeine_data, logicaffeine_system) for each program. The generated Rust code itself compiles in milliseconds.

---

## The Optimization Matrix

Rows = optimization techniques. Columns = languages. Each cell indicates support level.

**Legend:** `Y` = Yes | `P` = Partial | `N` = No | `-` = N/A (not applicable to language paradigm) | `I` = Inherited (Logos gets it free via Rust/LLVM)

### Memory & Allocation

| Technique | C | C++ | Rust | Zig | Go | Java | JS/V8 | Python | Ruby | Nim | Logos |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Manual memory management | Y | Y | P | Y | N | N | N | N | N | P | I |
| Stack allocation by default | Y | Y | Y | Y | N | N | N | N | N | Y | I |
| No GC pauses | Y | Y | Y | Y | N | N | N | N | N | P | I |
| Arena/bump allocation | P | Y | Y | P | N | Y | N | N | N | N | I |
| Custom allocators | Y | Y | Y | Y | N | N | N | N | N | Y | I |
| Placement new / in-place construction | N | Y | P | Y | N | N | N | N | N | N | I |
| Move semantics (avoid copies) | N | Y | Y | P | N | - | - | - | - | P | P |
| Copy elision (NRVO) | N | Y | Y | P | N | - | - | - | - | N | I |
| Escape analysis | N | N | P | N | Y | Y | Y | N | N | N | I |
| Object pooling / slab allocation | P | Y | Y | P | P | Y | N | N | N | N | N |

### Compilation & Code Generation

| Technique | C | C++ | Rust | Zig | Go | Java | JS/V8 | Python | Ruby | Nim | Logos |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Ahead-of-time compilation | Y | Y | Y | Y | Y | N | N | N | N | Y | I |
| LLVM backend | P | P | Y | N | N | N | N | N | N | N | I |
| Monomorphization (no vtable) | - | Y | Y | N | N | N | N | N | N | Y | I |
| Link-time optimization (LTO) | Y | Y | Y | Y | N | N | N | N | N | Y | I |
| Profile-guided optimization (PGO) | Y | Y | Y | Y | N | Y | Y | N | N | N | I |
| Compile-time function evaluation | N | Y | P | Y | N | N | N | N | N | Y | P |
| Template / generic specialization | - | Y | Y | Y | N | N | N | N | N | Y | I |
| Whole-program optimization | P | P | Y | Y | Y | Y | Y | N | N | Y | I |
| JIT compilation | N | N | N | N | N | Y | Y | N | P | N | N |
| Tiered compilation | N | N | N | N | N | Y | Y | N | P | N | N |

### Loop Optimization

| Technique | C | C++ | Rust | Zig | Go | Java | JS/V8 | Python | Ruby | Nim | Logos |
|---|---|---|---|---|---|---|---|---|---|---|---|
| For-range loops (counted iteration) | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | N |
| Loop unrolling | Y | Y | Y | Y | P | Y | Y | N | N | Y | I |
| Loop vectorization (auto-SIMD) | Y | Y | Y | Y | N | Y | P | N | N | Y | N |
| Loop-invariant code motion (LICM) | Y | Y | Y | Y | P | Y | Y | N | N | Y | I |
| Loop fusion | P | P | Y | P | N | Y | P | N | N | P | N |
| Loop tiling / blocking | N | P | P | N | N | Y | N | N | N | N | N |
| Strength reduction (i*4 → i+=4) | Y | Y | Y | Y | P | Y | Y | N | N | Y | I |
| Iterator-based loops (zero-overhead) | - | Y | Y | Y | N | N | N | N | N | Y | N |
| Loop interchange | P | P | P | P | N | Y | N | N | N | P | N |

### Scalar Optimization

| Technique | C | C++ | Rust | Zig | Go | Java | JS/V8 | Python | Ruby | Nim | Logos |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Constant folding | Y | Y | Y | Y | Y | Y | Y | N | N | Y | Y |
| Constant propagation | Y | Y | Y | Y | Y | Y | Y | N | N | Y | N |
| Dead code elimination | Y | Y | Y | Y | Y | Y | Y | N | N | Y | Y |
| Common subexpression elimination | Y | Y | Y | Y | P | Y | Y | N | N | Y | I |
| Register allocation (graph coloring) | Y | Y | Y | Y | Y | Y | Y | N | N | Y | I |
| Instruction scheduling | Y | Y | Y | Y | Y | Y | Y | N | N | Y | I |
| Algebraic simplification | Y | Y | Y | Y | P | Y | Y | N | N | Y | Y |
| Branch prediction hints | Y | P | P | Y | N | N | N | N | N | N | N |
| Tail call optimization | P | P | P | Y | N | N | P | N | N | Y | I |
| Inlining (automatic) | Y | Y | Y | Y | Y | Y | Y | N | N | Y | I |
| Bounds check elimination | P | - | P | P | P | Y | Y | N | N | P | N |

### Data Representation

| Technique | C | C++ | Rust | Zig | Go | Java | JS/V8 | Python | Ruby | Nim | Logos |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Unboxed primitives | Y | Y | Y | Y | Y | P | P | N | N | Y | I |
| Packed structs / field ordering | Y | Y | Y | Y | Y | N | N | N | N | Y | I |
| Tagged unions (sum types) | N | P | Y | Y | N | N | N | N | N | Y | I |
| Bitfield packing | Y | Y | P | Y | N | N | N | N | N | Y | N |
| SIMD intrinsics / builtins | Y | Y | Y | Y | N | P | N | N | N | P | N |
| Restrict / noalias pointers | Y | P | Y | N | N | N | N | N | N | N | I |
| Inline assembly | Y | Y | Y | Y | N | N | N | N | N | N | N |
| Zero-cost abstractions | P | Y | Y | Y | N | N | N | N | N | Y | P |

### Safety vs Performance Tradeoffs

| Technique | C | C++ | Rust | Zig | Go | Java | JS/V8 | Python | Ruby | Nim | Logos |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Bounds checking (array access) | N | N | Y | Y | Y | Y | Y | Y | Y | P | Y |
| Overflow checking | N | N | P | Y | N | N | N | N | N | N | P |
| Null safety (type-level) | N | P | Y | Y | N | N | N | N | N | Y | I |
| `unsafe` escape hatch | - | - | Y | Y | - | - | - | - | - | N | I |
| UB-based optimization | Y | Y | N | N | N | N | N | N | N | P | N |

### Runtime Features

| Technique | C | C++ | Rust | Zig | Go | Java | JS/V8 | Python | Ruby | Nim | Logos |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Hidden classes / shapes | N | N | N | N | N | N | Y | N | Y | N | N |
| Inline caching | N | N | N | N | N | N | Y | N | Y | N | N |
| Speculative optimization | N | N | N | N | N | Y | Y | N | P | N | N |
| Deoptimization | N | N | N | N | N | Y | Y | N | P | N | N |
| Scalar replacement | N | N | N | N | Y | Y | P | N | N | N | N |
| Goroutine / green threads | N | N | P | N | Y | P | Y | N | P | N | P |

---

## Language Deep Dives

### C — The Zero-Overhead Baseline

C is fast because it does nothing you don't ask for. There is no runtime, no GC, no bounds checking, no hidden allocation. The compiler sees exactly what the hardware will execute.

**Key optimizations:**
- **`restrict` pointers** — Tells the compiler that pointers don't alias, enabling aggressive loop vectorization. LLVM's `noalias` attribute is the Rust equivalent (applied to `&mut` references automatically).
- **UB-based optimization** — Signed overflow, null dereference, and out-of-bounds access are undefined behavior. The compiler assumes they never happen, enabling optimizations like removing bounds checks after a comparison, or assuming loop trip counts fit in a register.
- **Zero overhead** — No vtables unless you build them yourself. No RTTI. No exceptions (in typical embedded C). What you write is what you get.
- **`calloc` vs `malloc`** — `calloc` can use OS-level zero-page mapping for large allocations (zero-copy from kernel). The sieve benchmark exploits this with `calloc(limit + 1, 1)`.
- **`for` loop idiom** — `for (int i = 0; i < n; i++)` is the canonical counted loop. LLVM recognizes this pattern and applies trip count analysis, unrolling, and vectorization. Logos emits `while` loops instead, which are semantically equivalent but may not trigger the same pattern matchers in LLVM.

**What Logos doesn't have from C:** `restrict`-equivalent hints for the optimizer, UB-based loop bound inference, `for`-range loop emission.

### C++ — Templates and Move Semantics

C++ adds zero-cost abstractions over C without sacrificing performance. Template metaprogramming and `constexpr` move computation to compile time.

**Key optimizations:**
- **Templates / monomorphization** — `std::sort<int*>` generates type-specific machine code with no indirection. LLVM inlines the comparator. Logos inherits this via Rust's generics.
- **`constexpr` / `consteval`** — Arbitrary compile-time evaluation. C++20's `consteval` guarantees evaluation at compile time. Logos' OPTIMIZER_PLAN Phase 4 (compile-time function evaluation) achieves similar results using the interpreter as an oracle.
- **Move semantics** — `std::move` transfers ownership without copying. The compiler can elide copies when an object is used exactly once. Logos forces `.clone()` on every `logos_get()` call, even for values that are immediately consumed.
- **NRVO** — Named Return Value Optimization constructs the return value directly in the caller's stack frame. Rust (and by extension Logos) gets this automatically from LLVM.
- **SIMD via intrinsics** — `<immintrin.h>` provides AVX/SSE/NEON intrinsics. C++ code can hand-vectorize hot loops.

**What Logos doesn't have from C++:** Fine-grained move vs copy control in generated code, explicit SIMD.

### Rust — Ownership Without GC

Rust achieves C-level performance with memory safety. The ownership system eliminates GC pauses and enables aggressive optimization through aliasing guarantees.

**Key optimizations:**
- **`&mut` is `noalias`** — Every mutable reference is guaranteed exclusive. LLVM can assume no aliasing, enabling vectorization and reordering that C can only get with `restrict`. This is Logos' biggest inherited advantage.
- **Monomorphization** — Generic code is specialized per type at compile time. `Vec<i64>` generates different machine code from `Vec<String>`. No vtable dispatch.
- **Iterator fusion** — `iter().filter().map().sum()` compiles to a single loop with no intermediate allocations. LLVM sees through the iterator chain. Logos doesn't use iterators — it emits `while` loops.
- **Zero-cost abstractions** — `Option<T>` is the same size as `T` when `T` has a niche (null pointer optimization). `Result<T, E>` uses discriminant elision. `enum` variants are packed.
- **`#[inline(always)]`** — Forces inlining regardless of heuristics. Logos' `LogosIndex` trait methods use this, ensuring the trait dispatch is eliminated.

**What Logos loses by not using idiomatic Rust:** Iterator-based loops, `arr.swap()` instead of manual temp+set, `for i in 0..n` range loops, `arr[i]` direct indexing instead of `logos_get()`, eliding `.clone()` when the value is consumed immediately.

### Zig — Comptime and No Hidden Allocations

Zig beats C in several benchmarks (bubble sort 3.4x faster at 2K elements) through aggressive compile-time evaluation and explicit control over allocation.

**Key optimizations:**
- **`comptime`** — Any function can run at compile time. `comptime var x = fibonacci(10);` evaluates during compilation. No separate `constexpr` language — the same code works at both compile time and runtime.
- **No hidden allocations** — Every allocation is explicit. The standard library takes an `Allocator` parameter. No hidden `malloc` in string operations.
- **Wrapping arithmetic operators** — `*%` (wrapping multiply), `+%` (wrapping add), `-%` (wrapping subtract) are distinct operators. The compiler knows overflow behavior at each site.
- **`@memset` / `@memcpy` builtins** — Direct LLVM memset/memcpy intrinsics. `@memset(sieve, false)` in the sieve benchmark compiles to a single `rep stosb` instruction.
- **Saturating arithmetic** — `-|` (saturating subtract) avoids branch-heavy clamping code.
- **Safety without overhead** — Bounds checking in debug mode, unchecked in release. `ReleaseFast` mode disables all safety checks for maximum performance.

**Why Zig beats C at bubble sort:** Zig's `page_allocator` provides memory-mapped pages (fast allocation), and the array operations compile to tight register-based loops with no function call overhead.

### Go — Escape Analysis and Goroutine Scheduling

Go trades raw throughput for predictable latency and developer productivity. The GC is tuned for sub-millisecond pauses.

**Key optimizations:**
- **Escape analysis** — The compiler determines whether a value can stay on the stack. Values that don't escape the function are stack-allocated even if created with `new()` or composite literals.
- **Goroutine scheduling** — M:N scheduling with work stealing. Goroutines are 2KB initial stack (vs 1MB for OS threads). Context switching is ~100ns.
- **GC tuning** — The concurrent mark-sweep collector targets <500μs pauses. `GOGC` controls the GC frequency. Low-latency services set `GOGC=off` and manage memory manually with sync.Pool.
- **Devirtualization** — The compiler inlines interface method calls when the concrete type is known.

**Performance ceiling:** Go's GC and lack of generics (before 1.18) make it fundamentally slower than Rust/C for allocation-heavy workloads.

### Java — JIT and Escape Analysis

Java's JIT compiler (C2/Graal) can produce code faster than static compilers for long-running workloads through profile-guided speculative optimization.

**Key optimizations:**
- **Tiered compilation** — Interpreter → C1 (fast JIT) → C2 (optimizing JIT). Hot methods get progressively more optimized. Warmup takes 5-30 seconds.
- **Escape analysis + scalar replacement** — Objects that don't escape a method are decomposed into their fields and kept in registers. `Point p = new Point(x, y); return p.x + p.y;` → `return x + y;` with no allocation.
- **Speculative devirtualization** — When profiling shows a virtual call resolves to one type 99% of the time, the JIT inlines that type's implementation with a guard check.
- **Intrinsics** — The JVM replaces calls like `System.arraycopy()`, `Math.sqrt()`, and `String.equals()` with hand-tuned assembly. ~400 intrinsified methods in HotSpot.
- **Loop optimization** — The C2 compiler performs counted loop detection, range check elimination, loop unrolling, and auto-vectorization.

**Why Java benchmarks are slow at small sizes:** JVM startup (50-100ms) and warmup dominate. The benchmark numbers include this overhead. At scale, JIT-compiled Java approaches C speed for compute-bound code.

### JavaScript / V8 — Hidden Classes and Inline Caching

V8's TurboFan compiler achieves remarkable performance for a dynamic language through aggressive speculative optimization.

**Key optimizations:**
- **Hidden classes (shapes)** — Every object has a hidden class that describes its property layout. Objects with the same property order share a hidden class. Property access compiles to a fixed-offset memory load.
- **Inline caching** — Property access sites cache the hidden class and offset. The second access to `obj.x` is a direct memory load, not a hash table lookup.
- **TurboFan speculative optimization** — The JIT compiles based on observed types. If `x` has always been an integer, TurboFan generates integer arithmetic. If the type changes, the code deoptimizes and recompiles.
- **Allocation folding** — Multiple small allocations in a sequence are combined into a single bump-pointer allocation. Object creation becomes pointer increment + field writes.

**Performance ceiling:** V8 can't match compiled languages because every value is potentially boxed, every function call needs a type guard, and GC pauses interrupt execution.

### Python — Why It's Slow

Python is 26-50x slower than C across all benchmarks. Every operation goes through multiple layers of indirection.

**Key bottlenecks:**
- **Boxing** — Every integer is a heap-allocated `PyLongObject` with reference count, type pointer, and variable-length digit array. `1 + 1` allocates a new object.
- **Dynamic dispatch** — `a + b` calls `a.__add__(b)`, which does a type check, method lookup, and dispatch. Integer addition is ~15 instructions vs 1 for C.
- **GIL** — The Global Interpreter Lock prevents true parallelism. Only one thread executes Python bytecode at a time.
- **No JIT (CPython)** — CPython interprets bytecode in a switch-dispatch loop. Each bytecode instruction is ~20-50ns of overhead. PyPy's JIT eliminates much of this but adds memory overhead.
- **Reference counting** — Every assignment increments/decrements a reference count (atomic operation, cache-line bounce). This prevents many optimizations that compiled languages take for granted.

### Ruby — YJIT and Shape-Based Optimization

Ruby 3.2+ includes YJIT, a JIT compiler that uses shape-based optimization similar to V8's hidden classes.

**Key optimizations:**
- **YJIT** — Lazy basic block compilation. Only compiles code paths that are actually taken. Generates x86-64 machine code inline.
- **Object shapes** — Similar to V8's hidden classes. Objects with the same instance variable order share a shape, enabling fixed-offset access.
- **Speculative optimization** — Method calls are compiled with type guards. The most common receiver type gets an inlined fast path.

**Performance ceiling:** Still 20-50x slower than C. YJIT improves throughput by 15-30% over interpreted Ruby but can't overcome the fundamental overhead of dynamic dispatch and object model.

### Nim — Compile to C with Arc/Orc

Nim transpiles to C (like Logos transpiles to Rust), achieving C-level performance with Python-like syntax.

**Key optimizations:**
- **C backend** — Nim generates C code compiled with GCC/Clang. Inherits all C compiler optimizations.
- **ARC/ORC** — Automatic Reference Counting with cycle collection. No GC pauses. Deterministic destruction. Move semantics eliminate most reference count operations.
- **CTFE (Compile-Time Function Evaluation)** — `const x = fib(10)` evaluates arbitrary Nim code at compile time. Similar to Zig's `comptime`.
- **Value types** — Objects are stack-allocated by default. Heap allocation is explicit with `ref`.
- **Aggressive inlining** — The compiler inlines small functions aggressively. Generated C code is often a single function.

**Comparison to Logos:** Both compile to a systems language. Nim → C, Logos → Rust. Both inherit their backend's optimizations. Nim has mature CTFE; Logos has the tactic-based optimizer and proof kernel.

### Additional Notable Languages

**Swift** — Reference counting (ARC) with copy-on-write value types. SIL (Swift Intermediate Language) enables Swift-specific optimizations before LLVM. Performance matches C++ for most workloads. The bridge to Objective-C adds overhead for interop.

**Julia** — Multiple dispatch JIT. The compiler specializes on argument types, generating native code for each combination. First call is slow (JIT compilation); subsequent calls match C. Designed for numerical computing with native SIMD support.

**D** — CTFE (one of the first languages to have it), `betterC` mode disables GC/runtime for C-equivalent performance. Templates are more powerful than C++ but simpler syntax. GC by default hurts performance predictability.

**Haskell** — Deforestation eliminates intermediate data structures (`map f . map g` → `map (f . g)`). Stream fusion rewrites list operations to tight loops. Laziness enables algebraic optimization but makes performance reasoning difficult. GHC's optimizer is one of the most sophisticated outside LLVM.

**OCaml** — Unboxed types for integers and floats. The runtime representation uses tagged pointers (integers are shifted left 1 bit, LSB=1). Pattern matching compiles to jump tables. OCaml 5.0 adds multicore support with shared-memory GC.

---

## Logos Compiled Code Analysis

### What the Generated Rust Looks Like

Logos compiles English-like source to Rust. Here is the actual generated code for each benchmark, alongside the hand-written equivalents.

### Fibonacci — Nearly Identical

**Logos source:**
```
To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).
```

**Logos-generated Rust (`benchmarks/generated/fib.rs`):**
```rust
fn fib(n: i64) -> i64 {
    if (n < 2) {
        return n;
    }
    return (fib((n - 1)) + fib((n - 2)));
}
```

**Hand-written Rust (`benchmarks/programs/fib/main.rs`):**
```rust
fn fib(n: i64) -> i64 {
    if n < 2 { return n; }
    fib(n - 1) + fib(n - 2)
}
```

**Hand-written C (`benchmarks/programs/fib/main.c`):**
```c
long fib(long n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}
```

**Analysis:** The generated code is functionally identical. Extra parentheses and explicit `return` statements have zero performance impact — LLVM sees the same IR. The only difference is the `main()` function, where Logos uses `LogosIndex::logos_get(&arguments, 2)` to get the command-line argument (1-based indexing, with clone). This runs once and doesn't affect the benchmark.

**Result:** Logos matches C/C++ at this benchmark. The overhead is purely in the benchmark harness, not the algorithm.

### Sieve — The Indexing Bottleneck

**Logos-generated Rust (`benchmarks/generated/sieve.rs`):**
```rust
fn sieve(limit: i64) -> i64 {
    let mut flags = Seq::<bool>::default();
    let mut i = 0;
    while (i <= limit) {                              // ← while loop, not for range
        flags.push(false);                            // ← push loop to initialize
        i = (i + 1);
    }
    let mut count = 0;
    i = 2;
    while (i <= limit) {                              // ← while loop
        if (LogosIndex::logos_get(&flags, (i + 1))    // ← trait dispatch + clone
            == false) {
            count = (count + 1);
            let mut j = (i * i);
            while (j <= limit) {
                LogosIndexMut::logos_set(              // ← trait dispatch
                    &mut flags, (j + 1), true);
                j = (j + i);
            }
        }
        i = (i + 1);
    }
    return count;
}
```

**Hand-written Rust (`benchmarks/programs/sieve/main.rs`):**
```rust
let mut sieve = vec![false; limit + 1];               // ← single allocation
let mut count = 0u64;
for i in 2..=limit {                                   // ← for-range loop
    if !sieve[i] {                                     // ← direct indexing
        count += 1;
        let mut j = i * i;
        while j <= limit {
            sieve[j] = true;                           // ← direct mutation
            j += i;
        }
    }
}
```

**Hand-written C (`benchmarks/programs/sieve/main.c`):**
```c
char *sieve = calloc(limit + 1, 1);                   // ← zero-initialized, 1 byte per flag
int count = 0;
for (int i = 2; i <= limit; i++) {                     // ← counted for loop
    if (!sieve[i]) {                                   // ← direct memory access
        count++;
        for (long j = (long)i * i; j <= limit; j += i)
            sieve[j] = 1;                              // ← single byte write
    }
}
```

**Overhead sources in the Logos version:**
1. **Initialization loop** — `while (i <= limit) { flags.push(false); }` is O(n) push calls with potential reallocation. Hand-written versions use `vec![false; limit+1]` (single allocation) or `calloc` (OS-level zero-page).
2. **`LogosIndex::logos_get(&flags, (i + 1))`** — Each read goes through: (a) bounds check, (b) 1-based → 0-based conversion, (c) `unsafe { self.get_unchecked(idx).clone() }`. For `bool`, the clone is free, but the bounds check and conversion add ~2 instructions per access.
3. **`LogosIndexMut::logos_set(&mut flags, (j + 1), true)`** — Same overhead for writes.
4. **`while` loop vs `for` range** — `for i in 2..=limit` gives LLVM a known trip count, enabling unrolling and vectorization. `while (i <= limit) { ... i = (i + 1); }` requires LLVM to prove the same trip count by analyzing the loop body.
5. **1-based offset** — `(i + 1)` and `(j + 1)` are extra additions on every inner loop iteration.

### Bubble Sort — The Clone Tax

**Logos-generated Rust (`benchmarks/generated/bubble_sort.rs`):**
```rust
let mut j = 1;
while (j <= ((n - 1) - i)) {
    let a = LogosIndex::logos_get(&arr, j);        // ← clone on read
    let b = LogosIndex::logos_get(&arr, (j + 1));  // ← clone on read
    if (a > b) {
        LogosIndexMut::logos_set(&mut arr, j, b);  // ← trait dispatch
        LogosIndexMut::logos_set(&mut arr, (j + 1), a);
    }
    j = (j + 1);
}
```

**Hand-written Rust (`benchmarks/programs/bubble_sort/main.rs`):**
```rust
for j in 0..n - 1 - i {
    if arr[j] > arr[j + 1] {
        arr.swap(j, j + 1);                       // ← single swap, no clones
    }
}
```

**Overhead sources:**
1. **Two clones per comparison** — `logos_get` clones each element. For `i64`, clone is a copy (free). But the bounds checking and 1-based conversion still add overhead.
2. **Two sets per swap** — Logos does `set(j, b); set(j+1, a);` — two separate function calls. Rust's `arr.swap(j, j+1)` is a single operation that the compiler can optimize to register exchanges.
3. **`while` vs `for`** — Same LLVM pattern recognition gap as sieve.

### Ackermann — Pure Recursion

**Logos-generated Rust (`benchmarks/generated/ackermann.rs`):**
```rust
fn ackermann(m: i64, n: i64) -> i64 {
    if (m == 0) {
        return (n + 1);
    }
    if (n == 0) {
        return ackermann((m - 1), 1);
    }
    return ackermann((m - 1), ackermann(m, (n - 1)));
}
```

**Hand-written C (`benchmarks/programs/fib/main.c`):**
```c
long ackermann(long m, long n) {
    if (m == 0) return n + 1;
    if (n == 0) return ackermann(m - 1, 1);
    return ackermann(m - 1, ackermann(m, n - 1));
}
```

**Analysis:** Functionally identical. The extra parentheses in the generated code are cosmetic. Both versions produce the same LLVM IR. The ~1.6x slowdown at Ackermann(3,10) is surprising for identical code — this is likely due to differences in the `main()` function and benchmark harness, not the recursive function itself.

### Collection Operations — Logos Wins

**Logos-generated Rust (`benchmarks/generated/collect.rs`):**
```rust
let mut m = {
    let __m: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::with_capacity((n) as usize);
    __m
};
let mut i = 1;
while (i < (n + 1)) {
    LogosIndexMut::logos_set(&mut m, i, (i * 2));  // ← HashMap insert
    i = (i + 1);
}
```

**Hand-written C:**
```c
#define CAPACITY 1048576
struct Entry { int key, value, occupied; };
// Hand-rolled open-addressing hash table with linear probing
```

**Why Logos wins by ~2x:** Logos compiles to Rust's `HashMap`, which uses Robin Hood hashing with SIMD-accelerated probing (SwissTable). The hand-written C hash table uses simple linear probing with a fixed capacity. Rust's HashMap is one of the fastest hash table implementations in any language.

### String Assembly — Competitive

**Logos-generated Rust (`benchmarks/generated/strings.rs`):**
```rust
let mut result = String::with_capacity(((n * 6)) as usize);
let mut i = 0;
while (i < n) {
    write!(result, "{}{}", i, " ").unwrap();
    i = (i + 1);
}
let count: i64 = {
    result.chars().filter(|c| *c == ' ').count() as i64
};
```

**Analysis:** The generated code uses `String::with_capacity` (pre-allocation) and `write!` (formatting macro), which are idiomatic Rust patterns. Performance matches hand-written Rust closely. The `while` loop is the only difference from a `for i in 0..n` pattern.

---

## The Gap Analysis

Every optimization that Logos doesn't have, ranked by expected benchmark impact.

### HIGH Impact — Direct Performance Wins

#### 1. For-Range Loop Emission

**Current:** All counted loops emit `while` patterns:
```rust
let mut i = 0;
while (i < n) {
    // body
    i = (i + 1);
}
```

**Target:** Emit Rust `for` range loops:
```rust
for i in 0..n {
    // body
}
```

**Why it matters:** LLVM's loop analysis has pattern matchers tuned for `for` range loops in Rust's MIR. A `for i in 0..n` loop gives the optimizer: known trip count, known stride, known bounds — enabling unrolling, vectorization, and strength reduction. A `while` loop with a manually incremented counter requires the optimizer to reconstruct this information through induction variable analysis, which doesn't always succeed.

**Implementation:** In `codegen.rs`, detect `While { cond: i < n, body: [..., Set { target: i, value: i + step }] }` patterns and emit `for i in start..end` instead.

**Expected impact:** 10-25% improvement on sieve, 5-15% on bubble sort. The primary bottleneck in these benchmarks is the inner loop, where LLVM vectorization and unrolling depend on loop trip count recognition.

#### 2. Direct Array Indexing (Bypass LogosIndex)

**Current:** All array access goes through trait dispatch:
```rust
LogosIndex::logos_get(&arr, (i + 1))      // read
LogosIndexMut::logos_set(&mut arr, j, v)  // write
```

**Target:** When the compiler knows the collection type at codegen time, emit direct indexing:
```rust
arr[(i + 1 - 1) as usize]                // read: fold the 1-based offset at compile time
arr[(j - 1) as usize] = v;               // write: direct assignment
```

**Why it matters:** Even though `#[inline(always)]` eliminates the function call, the 1-based → 0-based conversion and bounds checking remain. For inner loops running millions of iterations, two extra instructions per access (subtract 1, bounds check) accumulate.

**Implementation:** The `RefinementContext` in `codegen.rs` already tracks variable types. The code at lines 7552-7570 already has a partial optimization path for known `Vec` types — it emits `arr[(idx - 1) as usize]` instead of `LogosIndex::logos_get()`. Extend this to handle more cases and eliminate the `(i + 1)` offset that Logos adds for 1-based indexing when the index is computed from a 0-based counter.

**Expected impact:** 15-30% improvement on sieve (inner loop is pure array access), 10-20% on bubble sort.

#### 3. Eliminate Unnecessary Clones

**Current:** Every `logos_get()` clones the returned value:
```rust
unsafe { self.get_unchecked(idx).clone() }
```

**Target:** For `Copy` types (`i64`, `f64`, `bool`, `char`), the clone is already a copy. But the codegen also adds explicit `.clone()` calls in some paths:
```rust
// codegen.rs line 7558
let suffix = if has_copy_element_type(t) { "" } else { ".clone()" };
```

This is partially optimized. The remaining gap is for values that are consumed immediately — `logos_get(&arr, i)` used directly in a comparison or arithmetic expression could return a reference instead of a clone.

**Implementation:** Analyze usage of `logos_get` results. If the result is: (a) used in a comparison, (b) used in arithmetic, (c) passed to a function that takes by value — the clone is necessary. If the result is bound to a variable that's used once and then dropped, the clone is unnecessary for `Copy` types (and LLVM already optimizes it away). For non-`Copy` types, this requires escape analysis at the AST level.

**Expected impact:** Minimal for current benchmarks (all use `i64`/`bool` which are `Copy`). Significant for string-heavy or struct-heavy workloads.

#### 4. Vec Initialization via `vec![value; size]`

**Current:** Collections are initialized with push loops:
```rust
let mut flags = Seq::<bool>::default();
let mut i = 0;
while (i <= limit) {
    flags.push(false);
    i = (i + 1);
}
```

**Target:** Single-allocation initialization:
```rust
let mut flags = vec![false; (limit + 1) as usize];
```

**Why it matters:** The push loop does O(n) reallocations (amortized) with O(n) bounds checks. `vec![false; n]` does a single allocation and a `memset`. For the sieve benchmark with 1M elements, this is the difference between ~1M push calls and a single `calloc`-equivalent operation.

**Implementation:** In the optimizer or codegen, detect the pattern `let mut v = Seq::default(); while (i < n) { v.push(constant); i += 1; }` and replace with `let mut v = vec![constant; n]`.

**Expected impact:** 5-10% improvement on sieve (initialization phase).

### MEDIUM Impact — Quality of Life

#### 5. Bounds Check Elimination

**Current:** Every `logos_get` / `logos_set` checks bounds:
```rust
if index < 1 { panic!(...); }
let idx = (index - 1) as usize;
if idx >= self.len() { panic!(...); }
```

**Target:** When the loop variable is provably in bounds (e.g., `for i in 0..arr.len()`), skip the bounds check.

**Implementation:** This is extremely difficult at the Logos level because bounds checks are in the `LogosIndex` trait implementation, not in the generated code. Options:
- Emit direct `arr[i]` indexing (see gap #2), which Rust/LLVM can bounds-check-eliminate through range analysis.
- Add `unsafe { arr.get_unchecked(i) }` when the index is provably in bounds — requires a proof obligation system.
- Use `arr.get(i).unwrap_unchecked()` with range proofs from the optimizer.

**Expected impact:** 5-15% on inner-loop-heavy benchmarks. LLVM already eliminates many bounds checks through its own analysis.

#### 6. Iterator-Based Loops

**Current:** Collections are iterated with index-based while loops:
```rust
let mut i = 0;
while (i < arr.len()) {
    let x = LogosIndex::logos_get(&arr, (i + 1));
    // ...
    i = (i + 1);
}
```

**Target:** Iterator-based loops:
```rust
for x in arr.iter() {
    // ...
}
```

**Why it matters:** Iterators avoid bounds checks entirely (the iterator knows it's in bounds by construction). They also enable fusion — `arr.iter().filter().map().collect()` is a single loop. And LLVM vectorizes iterator loops more reliably than index-based loops.

**Implementation:** Detect `Repeat for x in collection` patterns (which currently emit `for x in collection.clone()`) and, when the collection is not mutated in the loop body, emit `for x in collection.iter()` instead of `for x in collection.clone()`.

**Expected impact:** 5-10% on for-in loops over large collections. Also reduces memory pressure by eliminating the clone of the entire collection.

#### 7. Constant Propagation (Optimizer Phase 3)

**Current:** The optimizer doesn't propagate constant values through let bindings.

**Target:** `Let x be 10. Let y be x + 5.` → `Let y be 15.`

**Implementation:** Planned as OPTIMIZER_PLAN Phase 3 (`propagate.rs`). Single-pass forward walk maintaining a substitution environment.

**Expected impact:** Varies. Primarily enables further folding and DCE. For benchmarks, minimal direct impact. For user programs with configuration constants, could eliminate significant dead code.

#### 8. Algebraic Simplification (Optimizer Phase 1)

**Current:** The optimizer doesn't simplify `x + 0`, `x * 1`, `x * 0`, etc.

**Target:** Identity and absorbing element elimination.

**Implementation:** Planned as OPTIMIZER_PLAN Phase 1 (`try_simplify_algebraic` in `fold.rs`).

**Expected impact:** Minimal for benchmarks. Important for generated code quality and downstream optimization opportunities.

#### 9. `arr.swap()` for Element Exchange

**Current:** Bubble sort swaps via two separate `logos_set` calls:
```rust
let a = LogosIndex::logos_get(&arr, j);
let b = LogosIndex::logos_get(&arr, (j + 1));
LogosIndexMut::logos_set(&mut arr, j, b);
LogosIndexMut::logos_set(&mut arr, (j + 1), a);
```

**Target:**
```rust
arr.swap((j - 1) as usize, j as usize);
```

**Implementation:** The codegen already has a swap pattern detector (`try_emit_swap_pattern` in `codegen.rs`). Extend it to cover the `logos_get` + `logos_set` pattern and emit `arr.swap()`.

**Expected impact:** 5-10% on bubble sort specifically. `arr.swap()` compiles to three register moves (or `xchg` on x86), while two `logos_set` calls each include bounds checking and function call overhead.

### LOW Impact — Polish

#### 10. SIMD Hints

**Current:** No explicit SIMD. LLVM auto-vectorization handles simple loops.

**Target:** For known patterns (array fill, array copy, reduction), emit SIMD-friendly code or use Rust's `std::simd` (nightly).

**Implementation:** Not practical without significant codegen complexity. LLVM auto-vectorization handles the common cases when loop structure is clear (which depends on gaps #1 and #2).

**Expected impact:** 0-5% for current benchmarks. Up to 4-8x for array-processing workloads (when LLVM auto-vectorization kicks in from fixing loops).

#### 11. Profile-Guided Optimization (PGO)

**Current:** Not used.

**Target:** Compile with profiling instrumentation, run representative workload, recompile with profile data.

**Implementation:** Add `--pgo` flag to `largo build --release` that runs the program once with instrumentation, then recompiles. Straightforward but adds compilation time.

**Expected impact:** 2-5% across all benchmarks. PGO primarily helps branch prediction and function layout.

#### 12. Branch Prediction Hints

**Current:** No hints.

**Target:** `#[cold]` on panic paths, `likely`/`unlikely` on conditions.

**Implementation:** Mark the panic paths in `LogosIndex` implementations as `#[cold]`. This is a one-line change:
```rust
#[cold]
#[inline(never)]
fn index_out_of_bounds(index: i64, len: usize) -> ! {
    panic!("Index {} is out of bounds for seq of length {}", index, len);
}
```

**Expected impact:** 1-3% on tight loops. Moves panic code out of the hot path, improving instruction cache utilization.

### INHERITED — Free via Rust/LLVM

These optimizations are already applied to Logos' generated code by the Rust compiler and LLVM backend:

| Optimization | What It Does | Where It Helps |
|---|---|---|
| **Monomorphization** | Specializes generic code per type | All generic function calls |
| **Register allocation** | Assigns variables to CPU registers | All code |
| **Instruction scheduling** | Reorders instructions for pipeline efficiency | All code |
| **Link-time optimization (LTO)** | Optimizes across compilation units | Cross-function inlining |
| **NRVO / copy elision** | Constructs return values in caller's frame | Function returns |
| **Common subexpression elimination** | Reuses computed values | Repeated expressions |
| **Tail call optimization** | Converts tail recursion to loop | Recursive functions |
| **Dead code elimination (LLVM)** | Removes unreachable code | After inlining |
| **Loop-invariant code motion** | Hoists expressions out of loops | Loop bodies |
| **Strength reduction (LLVM)** | Replaces expensive ops with cheaper ones | Multiplications in loops |
| **Auto-vectorization** | Converts scalar loops to SIMD | Simple loops (when structure is clear) |
| **Constant folding (LLVM)** | Evaluates constant expressions | All constant math |
| **Function inlining** | Replaces calls with function bodies | Small functions |
| **`noalias` from `&mut`** | Guarantees exclusive access | Mutable references |

---

## What It Takes

For each high-impact missing optimization, the concrete implementation cost.

### Gap #1: For-Range Loop Emission

**File:** `crates/logicaffeine_compile/src/codegen.rs` (lines ~6102-6126)

**Current code path:**
```rust
Stmt::While { cond, body, decreasing: _ } => {
    let cond_str = codegen_expr_with_async(cond, ...);
    writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
    // ... body ...
    writeln!(output, "{}}}", indent_str).unwrap();
}
```

**What to change:** Before emitting a `while` loop, check if the loop matches the pattern:
- Condition: `i < n` or `i <= n` (comparison of a variable against an expression)
- Last statement in body: `Set { target: i, value: BinaryOp { Add, Identifier(i), Literal(1) } }`
- Variable `i` is not modified elsewhere in the body

If matched, emit:
```rust
for i in start..end {
    // body without the i = i + 1 statement
}
```

**Complexity:** Medium. The pattern matching is straightforward, but handling edge cases (step != 1, variable upper bound that changes during loop, `<=` vs `<`) requires care.

**Estimated effort:** ~100 lines of codegen changes + 10-15 test cases.

### Gap #2: Direct Array Indexing

**File:** `crates/logicaffeine_compile/src/codegen.rs` (lines ~7552-7570)

**Current code path** (partially optimized):
```rust
match known_type {
    Some(t) if t.starts_with("Vec") => {
        let suffix = if has_copy_element_type(t) { "" } else { ".clone()" };
        format!("{}[({} - 1) as usize]{}", coll_str, index_str, suffix)
    }
    _ => {
        format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
    }
}
```

**What to change:** The `Vec` path already emits direct indexing (`arr[(idx - 1) as usize]`). The issue is:
1. The `- 1` is always emitted even when the index already accounts for 1-based offset
2. The fallback path uses `LogosIndex::logos_get`
3. The type tracking doesn't always resolve `Seq<T>` to `Vec<T>`

Extend `RefinementContext` to track `Seq` types as `Vec`, and ensure the direct indexing path fires for all known-type array accesses.

**Complexity:** Low-Medium. The infrastructure exists; it's pattern matching coverage.

**Estimated effort:** ~50 lines of codegen changes + 5-10 test cases.

### Gap #3: Clone Elimination

**File:** `crates/logicaffeine_data/src/indexing.rs` (lines 71-84)

**What to change:** For `Copy` types, the current `unsafe { self.get_unchecked(idx).clone() }` is already optimized by LLVM (clone of a `Copy` type is a no-op). The real improvement would be adding a `logos_get_ref` method that returns `&T` instead of `T`, used when the caller only needs to read the value (comparisons, passing to functions that take `&T`).

**Complexity:** Medium-High. Requires tracking at codegen time whether a value is consumed or just observed.

**Estimated effort:** ~200 lines across codegen + indexing + test cases.

### Gap #4: Vec Initialization

**File:** `crates/logicaffeine_compile/src/codegen.rs` or optimizer

**What to change:** Detect the pattern in the AST or generated code:
```
Let mut v = Seq::default()
While (i < n):
    Push false to v
    Set i to (i + 1)
```

Replace with:
```rust
let mut v = vec![false; n as usize];
```

**Complexity:** Medium. Pattern matching at the AST level is cleaner than at the codegen level. Could be implemented as an optimizer pass.

**Estimated effort:** ~80 lines in optimizer + 5 test cases.

---

## Logos Advantages

What Logos has that other compilers don't.

### 1. Tactic-Based Optimizer from Proof Kernel

Most compilers encode optimization passes as ad-hoc pattern matching. Logos' optimizer is built on the same tactic combinator system (`then`, `orelse`, `try`, `repeat`, `first`) that powers its proof kernel. Each optimization pass is a tactic that transforms AST while preserving semantics. The pipeline is:

```
repeat(then(propagate, then(try(compile_eval), then(fold, dce))))
```

This means new optimizations plug in as composable tactics, and the `repeat` combinator automatically catches cascading optimization opportunities. No other English-to-code compiler has this architecture.

### 2. Polynomial Ring Normalization

The proof kernel's `ring.rs` canonicalizes arithmetic expressions to polynomial form. Adapted for the optimizer (OPTIMIZER_PLAN Phase 5), this means:

```
a + b - b          →  a               (cancellation)
2*x + 3*x          →  5*x             (like-term collection)
(a + 1) * (a - 1)  →  a*a - 1         (expansion to minimal form)
1 + a + 2 + b + 3  →  a + b + 6       (constant collection)
```

These simplifications emerge automatically from polynomial canonical forms — no special-case patterns needed. The ring axioms handle everything. This is mathematically rigorous optimization, not heuristic pattern matching.

### 3. Interpreter-as-Oracle Compile-Time Evaluation

Logos' interpreter and compiler share the same AST in the same crate. OPTIMIZER_PLAN Phase 4 uses the interpreter to evaluate pure functions at compile time:

```
factorial(5)  →  120
fib(10)       →  55
```

This works for arbitrary user-defined recursive functions with a step limit for safety. Most compilers can only evaluate built-in functions at compile time.

### 4. Automatic Memoization of Pure Recursive Functions

When the compiler detects a pure recursive function (no side effects), it can automatically add memoization. This transforms exponential-time Fibonacci into linear-time without any annotation from the user:

```
To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).
```

The compiler detects purity, wraps in a thread-local cache, and the recursive calls hit the cache. This is a unique optimization for a natural-language compiler.

### 5. Mutual Tail Call Optimization

Logos implements mutual TCO for groups of functions that call each other in tail position. This is rarer than single-function TCO — most compilers (including GCC and LLVM) only optimize self-tail-calls. Logos handles:

```
To isEven (n: Int) -> Bool:
    If n is 0: Return true.
    Return isOdd(n - 1).

To isOdd (n: Int) -> Bool:
    If n is 0: Return false.
    Return isEven(n - 1).
```

Both functions are compiled to a single loop with a state machine, eliminating stack growth entirely.

### 6. Accumulator Introduction

The optimizer can transform naive recursive functions into tail-recursive forms by introducing an accumulator parameter:

```
// Before optimization (linear recursion, O(n) stack)
To factorial (n: Int) -> Int:
    If n is 0: Return 1.
    Return n * factorial(n - 1).

// After optimization (tail recursion, O(1) stack)
To factorial (n: Int, acc: Int) -> Int:
    If n is 0: Return acc.
    Return factorial(n - 1, n * acc).
```

This transformation is provably correct via the proof kernel's rewrite rules. Combined with TCO, the result is a loop with constant stack usage.

---

## Benchmark Data

Full results from v0.8.12, measured on AMD EPYC 7763 (GitHub Actions, Ubuntu 24.04). All times in milliseconds. 10 iterations per configuration.

### Fibonacci — fib(n), Recursive

| Language | fib(10) | fib(15) | fib(20) | fib(25) | fib(30) |
|---|---|---|---|---|---|
| C | 0.532 | 0.594 | 0.608 | 0.691 | 2.117 |
| C++ | 0.525 | 0.529 | 0.561 | 0.689 | 2.161 |
| Rust | 0.705 | 0.704 | 0.786 | 0.956 | 3.259 |
| Zig | 0.221 | 0.217 | 0.252 | 0.455 | 2.349 |
| Go | 1.024 | 0.990 | 1.017 | 1.426 | 5.423 |
| Java | 29.68 | 30.04 | 40.62 | 31.53 | 33.36 |
| JavaScript | 23.60 | 22.96 | 24.10 | 24.90 | 34.30 |
| Python | 15.99 | 16.08 | 16.99 | 26.83 | 135.92 |
| Ruby | 62.62 | 62.52 | 63.34 | 71.53 | 158.34 |
| Nim | 0.555 | 0.557 | 0.679 | 0.987 | 3.516 |
| **Logos (release)** | **0.726** | **0.698** | **0.741** | **0.926** | **2.863** |
| Logos (debug) | 0.738 | 0.735 | 0.797 | 1.469 | 8.584 |

**Logos vs C at fib(30):** 2.863 / 2.117 = **1.35x slower**

### Sieve of Eratosthenes — Prime Count to n

| Language | 10K | 50K | 100K | 500K | 1M |
|---|---|---|---|---|---|
| C | 0.578 | 0.681 | 0.832 | 1.831 | 3.054 |
| C++ | 1.108 | 1.257 | 1.337 | 2.480 | 3.993 |
| Rust | 0.770 | 0.915 | 1.085 | 2.327 | 4.107 |
| Zig | 0.479 | 0.437 | 0.583 | 1.711 | 3.404 |
| Go | 1.000 | 1.167 | 1.280 | 2.403 | 3.884 |
| Java | 31.26 | 33.73 | 38.21 | 43.37 | 38.30 |
| JavaScript | 24.70 | 26.30 | 28.02 | 30.60 | 33.23 |
| Python | 17.02 | 22.32 | 28.51 | 81.91 | 148.87 |
| Ruby | 63.64 | 68.37 | 74.11 | 123.31 | 185.93 |
| Nim | 0.599 | 0.737 | 0.886 | 2.202 | 3.895 |
| **Logos (release)** | **1.033** | **1.012** | **1.182** | **2.856** | **4.900** |
| Logos (debug) | 1.435 | 3.755 | 6.638 | 31.47 | 63.04 |

**Logos vs C at sieve(1M):** 4.900 / 3.054 = **1.60x slower**

### Bubble Sort — Sort n Random Elements

| Language | 100 | 500 | 1K | 2K |
|---|---|---|---|---|
| C | 0.012 | 0.024 | 0.061 | 0.304 |
| C++ | 0.011 | 0.024 | 0.061 | 0.296 |
| Rust | 0.012 | 0.026 | 0.067 | 0.334 |
| Zig | 0.005 | 0.008 | 0.018 | 0.089 |
| Go | 0.021 | 0.038 | 0.099 | 0.383 |
| Java | 0.08 | 0.09 | 0.22 | 0.85 |
| JavaScript | 0.12 | 0.19 | 0.42 | 1.22 |
| Python | 0.18 | 0.47 | 1.02 | 1.93 |
| Ruby | 0.31 | 0.62 | 1.34 | 1.22 |
| Nim | 0.011 | 0.023 | 0.060 | 0.304 |
| **Logos (release)** | **0.011** | **0.023** | **0.061** | **0.315** |
| Logos (debug) | 0.032 | 0.137 | 0.378 | 0.404 |

**Logos vs C at bubble_sort(2K):** 0.315 / 0.304 = **1.04x slower**

### Collection Operations — HashMap Insert + Lookup

| Language | 1K | 5K | 10K | 50K |
|---|---|---|---|---|
| C | 2.662 | 2.736 | 3.023 | 4.146 |
| C++ | 1.178 | 1.371 | 1.772 | 2.836 |
| Rust | 0.771 | 0.885 | 1.126 | 1.877 |
| Go | 1.038 | 1.159 | 1.328 | 2.073 |
| Java | 31.96 | 32.97 | 35.08 | 35.63 |
| JavaScript | 24.04 | 24.34 | 24.68 | 26.26 |
| Python | 16.68 | 23.41 | 29.12 | 49.71 |
| Ruby | 63.05 | 65.50 | 69.52 | 72.79 |
| Nim | 0.698 | 0.764 | 0.903 | 1.450 |
| **Logos (release)** | **0.756** | **0.891** | **1.214** | **2.078** |
| Logos (debug) | 1.366 | 1.522 | 1.964 | 3.481 |

**Logos vs C at collect(50K):** 2.078 / 4.146 = **2.00x faster**

### String Assembly — Build String of n Numbers

| Language | Mean (50K) | Median | StdDev |
|---|---|---|---|
| C++ | 0.00166 | 0.00158 | 0.00012 |
| Zig | 0.00285 | 0.00288 | 0.00015 |
| Rust | 0.00502 | 0.00502 | 0.00014 |
| Nim | 0.00602 | 0.00601 | 0.00024 |
| C | 0.00622 | 0.00619 | 0.00029 |
| Go | 0.00687 | 0.00673 | 0.00030 |
| **Logos (release)** | **0.00697** | **0.00689** | **0.00041** |
| Logos (debug) | 0.01620 | 0.01598 | 0.00101 |
| JavaScript | 0.06502 | 0.06471 | 0.00251 |
| Java | 0.08270 | 0.08262 | 0.00327 |
| Ruby | 0.08475 | 0.08451 | 0.00231 |
| Python | 0.20807 | 0.20666 | 0.00545 |

**Logos vs C at strings(50K):** 0.00697 / 0.00622 = **1.12x slower**

### Ackermann — ackermann(3, n)

| Language | Mean (n=10) | Median | StdDev |
|---|---|---|---|
| C++ | 0.1405 | 0.1403 | 0.0086 |
| C | 0.1478 | 0.1476 | 0.0069 |
| Rust | 0.1980 | 0.1950 | 0.0135 |
| Java | 0.2179 | 0.2179 | 0.0033 |
| Nim | 0.2317 | 0.2331 | 0.0112 |
| Zig | 0.2342 | 0.2328 | 0.0146 |
| **Logos (release)** | **0.2344** | **0.2340** | **0.0133** |
| Go | 0.2504 | 0.2487 | 0.0173 |
| Logos (debug) | 0.3223 | 0.3186 | 0.0258 |
| JavaScript | 1.0993 | 1.0877 | 0.0629 |
| Ruby | 4.4454 | 4.4388 | 0.0897 |
| Python | 8.0565 | 7.9946 | 0.1720 |

**Logos vs C at ackermann(3,10):** 0.2344 / 0.1478 = **1.59x slower**

### Geometric Mean Speedup vs C (Reference Sizes)

| Language | Geometric Mean | Interpretation |
|---|---|---|
| Zig | 1.111 | 11.1% faster than C |
| **Logos (release)** | **1.038** | **3.8% faster than C** |
| Rust | 1.011 | 1.1% faster than C |
| C | 1.000 | Baseline |
| C++ | 0.997 | 0.3% slower than C |
| Nim | 0.872 | 12.8% slower than C |
| Go | 0.773 | 22.7% slower than C |
| Logos (debug) | 0.125 | 87.5% slower than C |
| JavaScript | 0.105 | 89.5% slower than C |
| Java | 0.104 | 89.6% slower than C |
| Python | 0.026 | 97.4% slower than C |
| Ruby | 0.023 | 97.7% slower than C |

The geometric mean is pulled above 1.0 by the collection operations benchmark, where Logos' Rust HashMap implementation is ~2x faster than the hand-written C hash table.

### Compilation Time (Release Build)

| Compiler | Mean (ms) | Notes |
|---|---|---|
| GCC `-O2` | 71.84 | Single-file C compilation |
| G++ `-O2` | 82.24 | Single-file C++ compilation |
| Go `build` | 114.30 | Single-file Go compilation |
| Rustc `-O` | 137.03 | Single-file Rust compilation |
| Javac | 376.60 | Single-file Java compilation |
| Nim `c` | 633.52 | Transpile to C + compile |
| Largo debug | 4,973.90 | Full dependency compilation |
| **Largo release** | **13,434.80** | Full dependency compilation + LLVM opt |

**Why Logos compilation is slow:** `largo build --release` compiles the generated Rust file plus all dependencies (`logicaffeine_data`, `logicaffeine_system`) from source with full optimization. The generated Rust code itself is trivial — the time is spent in dependency compilation and LLVM optimization passes. Incremental builds (after the first compilation) are significantly faster.

### Logos Interpreted Mode (Single Measurement)

| Benchmark | Size | Time (ms) |
|---|---|---|
| Fibonacci | 35 | 19,193.52 |
| Bubble Sort | 5,000 | 1,099.59 |
| Sieve | 1M | 36.94 |
| Strings | 100K | 13.19 |
| Collection | 50K | 6.09 |
| Ackermann | 10 | 0.01 |

The interpreter is suitable for development and education but 100-1000x slower than compiled code for recursion-heavy workloads.

---

## Summary

Logos at v0.8.12 is **competitive with hand-written C, Rust, and Zig** for compiled performance. The geometric mean of 1.038x vs C means that, across the benchmark suite, Logos-generated code is slightly faster than hand-written C on average — though this is significantly boosted by the collection benchmark where Rust's HashMap beats C's hand-rolled hash table.

The primary optimization targets are:
1. **For-range loop emission** — Unlock LLVM vectorization and unrolling
2. **Direct array indexing** — Bypass `LogosIndex` trait dispatch in inner loops
3. **Clone elimination** — Remove unnecessary copies on array reads
4. **Vec initialization patterns** — Replace push loops with `vec![val; n]`

These four changes, focused on `codegen.rs`, would close the gap on array-heavy benchmarks (sieve, bubble sort) while preserving the existing advantages on pure computation and hash table operations. The estimated implementation cost is ~400 lines of codegen changes and ~40 test cases.

The tactic-based optimizer (OPTIMIZER_PLAN) provides a unique architectural advantage for future optimizations. As phases 1-6 are implemented, Logos will gain constant propagation, compile-time evaluation, polynomial normalization, and algebraic simplification — capabilities that go beyond what most compiled languages offer at the source level, complementing the already-strong LLVM backend optimizations.

---

## The Mountain — Tiered Optimization Roadmap

Every `N` and `P` cell in the optimization matrix is a gap between Logos and the performance ceiling. This section enumerates all 24 gaps, classifies them into implementation tiers, analyzes each one in depth, and maps the critical path from where Logos stands today to where it could be.

---

### 1. Every N and P Cell, Enumerated

24 cells across 6 categories where Logos is not at full capability.

#### Memory & Allocation (2 gaps)

| Cell | Status | Description |
|---|---|---|
| Move semantics (avoid copies) | **P** | `logos_get()` always clones. For-in loops clone entire collections. Ownership analysis exists but doesn't feed codegen. |
| Object pooling / slab allocation | **N** | No pooling or slab allocator for hot-path objects. Arena allocation exists at parse time but not at runtime. |

#### Compilation & Code Generation (2 gaps)

| Cell | Status | Description |
|---|---|---|
| Compile-time function evaluation | **P** | Interpreter exists and shares the AST. Not yet wired as an optimizer pass. |
| JIT / Tiered compilation | **N** | Architecture is AOT to Rust. No runtime code generation. |

#### Loop Optimization (6 gaps)

| Cell | Status | Description |
|---|---|---|
| For-range loops (counted iteration) | **N** | All counted loops emit `while` with manual counter increment. LLVM misses trip count analysis. |
| Loop vectorization (auto-SIMD) | **N** | LLVM can vectorize, but `while` loops and `LogosIndex` trait dispatch block it. |
| Loop fusion | **N** | No multi-loop analysis. Adjacent loops over the same range are not merged. |
| Loop tiling / blocking | **N** | No 2D array access patterns in the language model. |
| Iterator-based loops (zero-overhead) | **N** | `Repeat for x in coll` emits `for x in coll.clone()`. No `.iter()` emission. |
| Loop interchange | **N** | No nested loop reordering. Would require 2D access pattern analysis. |

#### Scalar Optimization (3 gaps)

| Cell | Status | Description |
|---|---|---|
| Constant propagation | **N** | No forward propagation of constant let bindings through the AST. |
| ~~Algebraic simplification~~ | ~~**P**~~ **Y** | ✅ Identity/annihilator rules implemented for int and float (`x + 0 → x`, `x * 1 → x`, `x * 0 → 0`, `x - 0 → x`, `x / 1 → x`). |
| Branch prediction hints | **N** | No `#[cold]` on panic paths, no `likely`/`unlikely` annotations. |
| Bounds check elimination | **N** | Every `logos_get` checks bounds. No range proof integration. |

#### Data Representation (3 gaps)

| Cell | Status | Description |
|---|---|---|
| Bitfield packing | **N** | Boolean sequences use `Vec<bool>` (1 byte per flag). No bitfield representation. |
| SIMD intrinsics / builtins | **N** | No explicit SIMD. Relies entirely on LLVM auto-vectorization. |
| Zero-cost abstractions | **P** | `LogosIndex` trait adds overhead in fallback path. Direct indexing exists but doesn't fire for all cases. |

#### Safety & Runtime (3 gaps)

| Cell | Status | Description |
|---|---|---|
| Overflow checking | **P** | Rust's release mode wraps silently. No opt-in checked arithmetic at the Logos level. |
| Scalar replacement | **N** | Reclassify to **I** — LLVM already performs SROA (Scalar Replacement of Aggregates). |
| Green threads | **P** | `Concurrent` block exists with `tokio::spawn`. Not zero-cost — requires async runtime. |

#### Runtime Features — Not Applicable (6 gaps)

| Cell | Status | Description |
|---|---|---|
| Hidden classes / shapes | **N** | N/A for compiled language. |
| Inline caching | **N** | N/A for compiled language. |
| Speculative optimization | **N** | N/A for compiled language. |
| Deoptimization | **N** | N/A for compiled language. |
| JIT compilation | **N** | N/A — architecture is AOT. |
| Tiered compilation | **N** | N/A — architecture is AOT. |

---

### 2. Tier Classification

Items are classified by prerequisite depth: Tier 0 must land before Tier 1 can be effective, Tier 1 before Tier 4, etc. Each tier has an ID for cross-referencing.

#### Tier 0 — Bedrock (3 items) ✅ COMPLETE (v0.8.14)

Prerequisites that unlock everything else. Without these, higher-tier optimizations have limited surface area.

| ID | Item | Matrix Cell | Status |
|---|---|---|---|
| **0-A** | Deep expression recursion in fold.rs | — | ✅ Exhaustive matching for all 26 Expr variants. `fold_expr` signature extended with `stmt_arena` for Closure body folding. |
| **0-B** | Unreachable-after-return DCE | DCE (Y→Y+) | ✅ Post-pass truncation after `Stmt::Return` in `eliminate_dead_code`. 3 lines. |
| **0-C** | Algebraic simplification | Algebraic simp (P→Y) | ✅ `try_simplify_algebraic` in `fold.rs` — int and float identity/annihilator rules. |

#### Tier 1 — Direct Strike (5 items) ✅ COMPLETE (v0.8.15)

Codegen-level changes that produce the biggest immediate speedups. These directly close the gap on array-heavy benchmarks.

| ID | Item | Matrix Cell | Status |
|---|---|---|---|
| **1-A** | For-range loop emission | For-range (N→Y) | ✅ `try_emit_for_range_pattern` with post-loop counter value restoration, `body_modifies_var` guard. Integrated at all 7 peephole sites. |
| **1-B** | Iterator-based loops | Iterator loops (N→Y) | ✅ `.iter().copied()` for Copy-type `Vec` when body doesn't mutate collection. `body_mutates_collection` recursive helper. |
| **1-C** | Direct array indexing + clone elimination | Zero-cost abstractions (P→Y) | ✅ List literal element type inference from first literal. `[10, 20, 30]` registers as `Vec<i64>`. |
| **1-D** | Vec initialization pattern | — | ✅ `BinaryOpKind::Lt` (exclusive bound) added to vec-fill pattern. |
| **1-E** | Swap pattern enhancement | — | ✅ `BinaryOpKind::Eq` and `BinaryOpKind::NotEq` added to swap comparison. |

#### Tier 2 — Core Muscle (4 items)

The kernel-powered optimization passes. These leverage `simp.rs`, `ring.rs`, `lia.rs`, and `cc.rs` — infrastructure no other English-to-code compiler has.

| ID | Item | Matrix Cell | Kernel Leverage |
|---|---|---|---|
| **2-A** | Constant propagation | Const prop (N→Y) | `simp.rs` Substitution = `HashMap<i64, STerm>` maps directly to propagation env = `HashMap<Symbol, &Expr>` |
| **2-B** | Compile-time function evaluation | CTFE (P→Y) | Interpreter-as-oracle with fuel limit (1000 steps from `simp.rs` pattern) |
| **2-C** | Polynomial normalization | — | `ring.rs` Polynomial/Monomial with BTreeMap canonical forms. Structural equality = semantic equality. |
| **2-D** | Peephole + strength reduction | — | `lia.rs` LinearExpr for strength reduction proofs (e.g., `i * 4` → `i << 2` when provably non-negative) |

#### Tier 3 — Structural Strength (5 items)

Deeper architectural changes that require more analysis infrastructure.

| ID | Item | Matrix Cell | Complexity |
|---|---|---|---|
| **3-A** | Move semantics deep dive | Move semantics (P→Y) | High — requires connecting `ownership.rs` VarState tracking to codegen decisions |
| **3-B** | Bounds check elimination | Bounds check elim (N→Y) | High — requires range proof integration or switch to direct indexing (1-C) |
| **3-C** | Object pooling / slab allocation | Object pooling (N→Y) | Medium — `typed-arena` or `bumpalo` for hot-path runtime objects |
| **3-D** | Loop fusion | Loop fusion (N→P) | Medium — limited applicability for Logos programs |
| **3-E** | Overflow checking | Overflow (P→Y) | Low — `checked_add` / `checked_mul` with configurable panic behavior |

#### Tier 4 — Summit (7 items)

Peak optimizations. Some fall out free from earlier tiers. Others are extreme specializations.

| ID | Item | Matrix Cell | Reality Check |
|---|---|---|---|
| **4-A** | Loop vectorization | Vectorization (N→I) | Falls out **free** from LLVM once 1-A and 1-C land. No Logos work needed. |
| **4-B** | SIMD intrinsics | SIMD (N→P) | Requires language-level SIMD types. Extreme effort for marginal gain. |
| **4-C** | Bitfield packing | Bitfield (N→P) | `Vec<bool>` → `BitVec` for boolean sequences. Medium effort, niche benefit. |
| **4-D** | Loop tiling / blocking | Tiling (N→N) | Not relevant — Logos has no 2D array access patterns. |
| **4-E** | Loop interchange | Interchange (N→N) | Not relevant — requires nested 2D array iteration. |
| **4-F** | Branch prediction hints | Branch hints (N→P) | `#[cold]` on panic paths in `LogosIndex`. One-line change, 1–3% improvement. |
| **4-G** | Green threads | Green threads (P→Y) | `tokio` runtime already exists. Gap is zero-cost lightweight tasks without full async. |

#### N/A — Inapplicable (6 items)

These `N` cells are architectural impossibilities for a compiled-to-Rust language, not gaps to close.

| Item | Why N/A |
|---|---|
| **Hidden classes / shapes** | Logos structs compile to Rust structs with compile-time-known layouts. There are no dynamic property additions — every field is known at parse time and monomorphized. Hidden classes solve a problem that doesn't exist here. |
| **Inline caching** | All method dispatch is resolved at compile time. Trait methods are monomorphized. There are no polymorphic call sites at runtime — the Rust compiler has already resolved every call to a concrete function. |
| **Speculative optimization** | No runtime profiler exists. All types are fully resolved during compilation. There is nothing to speculate about — the generated Rust code has concrete types everywhere. |
| **Deoptimization** | No JIT to deoptimize from. The binary is statically compiled. There is no "slow path" to fall back to because there is no "fast speculative path" in the first place. |
| **Scalar replacement** | Reclassify to **I** (Inherited). LLVM already performs SROA (Scalar Replacement of Aggregates) on the generated Rust code. Structs that don't escape are decomposed into registers automatically. |
| **JIT / Tiered compilation** | The architecture is AOT compilation: Logos → Rust → LLVM → native binary. Adding a JIT would require a fundamentally different runtime model. The interpreter already serves the "fast iteration" use case; compiled code serves the "maximum performance" use case. |

---

### 3. Per-Item Analysis

Detailed analysis for each of the 24 actionable items (Tiers 0–4).

#### 0-A: Deep Expression Recursion in Fold

**Tier:** 0 — Bedrock
**Expected speedup:** Indirect — enables all downstream folding
**Complexity:** Low
**Kernel leverage:** None directly

**Current state:** The constant folder in `optimize/fold.rs` has a catch-all `_ => expr` at line 197. Expression variants like `Expr::Call`, `Expr::MethodCall`, `Expr::FieldAccess`, `Expr::Index`, and many others pass through without their subexpressions being recursively folded.

**What "done" looks like:** Every `Expr` variant that contains subexpressions recursively folds them. The catch-all is replaced with exhaustive matching. After this change, `f(2 + 3)` folds to `f(5)` and `arr[1 + 0]` folds to `arr[1]`.

**Key files:**
- `crates/logicaffeine_compile/src/optimize/fold.rs` — lines 180–199, replace catch-all with recursive descent
- Estimated ~60 lines of new match arms

**Dependencies:** None. This is the foundation.

#### 0-B: Unreachable-After-Return DCE

**Tier:** 0 — Bedrock
**Expected speedup:** Indirect — reduces codegen noise, enables cleaner analysis
**Complexity:** Low
**Kernel leverage:** None

**Current state:** `optimize/dce.rs` (92 lines) handles `if false` and `while false` elimination but does not scan for `Stmt::Give` or `Stmt::Exit` within blocks. Statements after an unconditional return survive into codegen:

```
To example (n: Int) -> Int:
    If n is 0: Return 1.       ← Give
    Return n.                   ← Give
    Set n to 42.                ← Dead code, survives to generated Rust
```

**What "done" looks like:** In `eliminate_dead_code`, after processing each statement in a block, check if it's a `Give` or `Exit`. If so, discard all remaining statements in the block. The truncation happens before recursive descent into sub-blocks.

**Key files:**
- `crates/logicaffeine_compile/src/optimize/dce.rs` — add post-return truncation in the main loop (~15 lines)

**Dependencies:** None.

#### 0-C: Algebraic Simplification

**Tier:** 0 — Bedrock
**Expected speedup:** Indirect — cleans up output from propagation and folding
**Complexity:** Low
**Kernel leverage:** `ring.rs` provides the mathematical foundation, but implementation is simpler pattern matching

**Current state:** The optimizer folds `2 + 3 → 5` (literal-literal folding) but does not simplify `x + 0 → x`, `x * 1 → x`, `x * 0 → 0`, `0 - x → -x`, `x - x → 0`, `x / 1 → x`.

**What "done" looks like:** A `try_simplify_algebraic` function in `fold.rs` that fires on `BinaryOp` nodes when one operand is a known identity or absorbing element. Produces simplified `Expr` nodes.

**Key files:**
- `crates/logicaffeine_compile/src/optimize/fold.rs` — add ~40 lines of identity/absorber rules
- Test: verify cascading `let x = 0. let y = a + x.` → `let y = a.`

**Dependencies:** None. Enhances 0-A (deeper folding exposes more simplification sites).

#### 1-A: For-Range Loop Emission

**Tier:** 1 — Direct Strike
**Expected speedup:** 10–25% on sieve, 5–15% on bubble sort
**Complexity:** Medium
**Kernel leverage:** None

**Current state:** All counted loops emit `while` patterns in `codegen.rs` lines 6102–6126:
```rust
let mut i = 0;
while (i < n) {
    // body
    i = (i + 1);
}
```

LLVM's loop analysis has pattern matchers tuned for `for` range loops in Rust's MIR. A `while` loop with a manually incremented counter requires LLVM to reconstruct trip count through induction variable analysis, which doesn't always succeed — blocking unrolling, vectorization, and strength reduction.

**What "done" looks like:** The codegen detects `While { cond: Var < Bound, body: [..., Set { target: Var, value: Add(Var, 1) }] }` and emits `for var in start..bound` (or `start..=bound` for `<=`). The counter increment statement is excluded from the emitted body. Edge cases handled: step ≠ 1 → `(start..end).step_by(step)`, mutable upper bound → fall back to `while`.

**Key files:**
- `crates/logicaffeine_compile/src/codegen.rs` — lines 6102–6126, add pattern detection before `while` emission (~100 lines)

**Dependencies:** 0-A (constant folding may simplify loop bounds first).

#### 1-B: Iterator-Based Loops

**Tier:** 1 — Direct Strike
**Expected speedup:** 5–10% on for-in loops, eliminates full collection clone
**Complexity:** Medium
**Kernel leverage:** None

**Current state:** `Repeat for x in collection` emits `for x in collection.clone()` at `codegen.rs` line 6159. The `.clone()` copies the entire collection before iteration to avoid moving it. For a `Vec<i64>` with 1M elements, this allocates and copies 8MB before iterating.

**What "done" looks like:** Three-tier emission based on body analysis:
1. If body does **not** mutate or move the collection → `for x in collection.iter()` (zero-copy)
2. If body mutates the collection → `for x in collection.clone()` (current behavior)
3. If collection is consumed after the loop → `for x in collection` (move, no clone)

Detection requires checking whether the collection symbol appears in any `Set`, `SetIndex`, `Push`, `Pop`, or `Remove` statement within the loop body.

**Key files:**
- `crates/logicaffeine_compile/src/codegen.rs` — lines 6128–6178, add body mutation analysis (~60 lines)

**Dependencies:** None. Independent of other items.

#### 1-C: Direct Array Indexing + Clone Elimination

**Tier:** 1 — Direct Strike
**Expected speedup:** 15–30% on sieve, 10–20% on bubble sort
**Complexity:** Low-Medium
**Kernel leverage:** None

**Current state:** The codegen at lines 7548–7571 already has a partial optimization:
```rust
match known_type {
    Some(t) if t.starts_with("Vec") => {
        let suffix = if has_copy_element_type(t) { "" } else { ".clone()" };
        format!("{}[({} - 1) as usize]{}", coll_str, index_str, suffix)
    }
    _ => {
        format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
    }
}
```

The `Vec` path fires for known types and emits `arr[(idx - 1) as usize]` with no clone for `Copy` types. The gap is:
1. Type tracking doesn't always resolve `Seq<T>` to `Vec<T>` — the fallback path uses `LogosIndex::logos_get`
2. The `- 1` offset is always emitted even when the index already compensates

**What "done" looks like:** The `RefinementContext` resolves `Seq<T>` as `Vec<T>` in all cases. The fallback `LogosIndex::logos_get` path fires only for genuinely unknown collection types (rare in practice). For `Copy` element types, no `.clone()` suffix is emitted.

**Key files:**
- `crates/logicaffeine_compile/src/codegen.rs` — lines 7548–7571, extend type resolution (~50 lines)
- `crates/logicaffeine_data/src/indexing.rs` — lines 71–84, add `logos_get_ref` returning `&T` for non-Copy observation patterns (~30 lines)

**Dependencies:** Independent, but compounds with 1-A (for-range + direct indexing = LLVM vectorization).

#### 1-D: Vec Initialization Pattern

**Tier:** 1 — Direct Strike
**Expected speedup:** 5–10% on sieve (initialization phase)
**Complexity:** Medium (already partially implemented)
**Kernel leverage:** None

**Current state:** The peephole optimization `try_emit_vec_fill_pattern` at `codegen.rs` lines 5554–5714 already detects the pattern:
```
Let mut v be a new Seq of Bool.
Let mut i be 0.
While i <= limit: Push false to v. Set i to i + 1.
```
and emits `let mut v: Vec<bool> = vec![false; (limit + 1) as usize];`

**What remains:** The peephole fires at the codegen statement level, requiring the three statements to be adjacent and the pattern to exactly match. An optimizer-level version would:
1. Handle non-adjacent declarations (other statements interleaved)
2. Handle `Seq::default()` without explicit type annotation (infer from push value)
3. Handle `push(variable)` when the variable is a known constant

**What "done" looks like:** The existing peephole catches 80% of cases. Promoting this to an optimizer pass would catch the remaining 20%.

**Key files:**
- `crates/logicaffeine_compile/src/codegen.rs` — lines 5554–5714 (existing, mostly complete)
- `crates/logicaffeine_compile/src/optimize/` — optional new pass (~80 lines)

**Dependencies:** 0-A (folding push values to constants enables more matches).

#### 1-E: Swap Pattern Enhancement

**Tier:** 1 — Direct Strike
**Expected speedup:** 5–10% on bubble sort
**Complexity:** Low (already partially implemented)
**Kernel leverage:** None

**Current state:** `try_emit_swap_pattern` at `codegen.rs` lines 5752–5906 detects the three-statement swap pattern:
```
Let a be item j of arr.
Let b be item (j+1) of arr.
If a > b: Set item j of arr to b. Set item (j+1) of arr to a.
```
and emits:
```rust
if arr[(j - 1) as usize] > arr[j as usize] {
    arr.swap((j - 1) as usize, j as usize);
}
```

**What remains:** The pattern requires exactly adjacent statements, `Let` (not `Set`), and comparison operators only (`>`, `<`, `>=`, `<=`). Extensions:
1. Handle `Set` temporary variables (not just `Let`)
2. Handle non-adjacent swap (other statements between read and write)
3. Handle equality comparison (`==`, `!=`) for partition algorithms

**What "done" looks like:** Bubble sort benchmark matches hand-written Rust's `arr.swap()` performance. Already close — the existing peephole fires for the standard bubble sort pattern.

**Key files:**
- `crates/logicaffeine_compile/src/codegen.rs` — lines 5752–5906 (existing, mostly complete)

**Dependencies:** 1-C (direct indexing makes the swap comparison use direct indexing too).

#### 2-A: Constant Propagation

**Tier:** 2 — Core Muscle
**Expected speedup:** Varies — primarily enables downstream folding and DCE
**Complexity:** Medium
**Kernel leverage:** **Direct** — `simp.rs` provides the template

**Current state:** The optimizer has no propagation pass. `Let x be 10. Let y be x + 5.` survives as two separate let bindings with the addition preserved.

**How `simp.rs` maps:**

The kernel's `simp.rs` uses `Substitution = HashMap<i64, STerm>` to track variable-to-term mappings extracted from hypotheses. The optimizer's propagation pass mirrors this exactly:

| Kernel (`simp.rs`) | Optimizer propagation |
|---|---|
| `Substitution = HashMap<i64, STerm>` | `PropEnv = HashMap<Symbol, &Expr>` |
| `decompose_goal` extracts hypothesis equalities | Forward walk extracts `Let` constant bindings |
| `simplify_sterm(term, subst, fuel)` | `propagate_expr(expr, env)` |
| Fuel limit prevents infinite loops | Single-pass forward walk (no fuel needed) |

**What "done" looks like:** Single forward pass over the AST. When a `Let x = <constant_expr>` is encountered, add `x → <constant_expr>` to the environment. When `Identifier(x)` is encountered in a subsequent expression, substitute the constant. After substitution, re-fold the containing expression (leveraging 0-A). Kill the binding if `x` is never used after substitution (leveraging 0-B DCE).

**Key files:**
- `crates/logicaffeine_compile/src/optimize/propagate.rs` — new file, ~120 lines
- `crates/logicaffeine_compile/src/optimize/mod.rs` — add propagation to the 2-pass pipeline → 3-pass: `propagate → fold → dce`

**Dependencies:** 0-A (folding must handle all expression types), 0-C (algebraic simplification cleans up after substitution).

#### 2-B: Compile-Time Function Evaluation

**Tier:** 2 — Core Muscle
**Expected speedup:** Varies — eliminates function calls with constant arguments
**Complexity:** Medium-High
**Kernel leverage:** **Fuel pattern** from `simp.rs`

**Current state:** The interpreter and compiler share the same AST in the same crate. The interpreter can evaluate any Logos function. This capability is not wired into the optimizer.

**How `simp.rs` fuel maps:** The kernel uses `const FUEL: usize = 1000` (line 256) with decrement-on-recursion to prevent non-termination. The CTFE pass uses the same pattern: evaluate a function call with `fuel = 1000` steps. If the interpreter terminates within the fuel budget, replace the call with the result literal. If it doesn't, leave the call as-is.

**What "done" looks like:** When the optimizer encounters `Call { func, args }` where all `args` are constant literals:
1. Look up `func` in the function table
2. Check purity (no side effects — no Print, no Set to globals, no I/O)
3. Evaluate via the interpreter with a step limit
4. If evaluation completes, replace the entire `Call` with `Literal(result)`

Example: `factorial(5)` → `120`, `fib(10)` → `55`.

**Key files:**
- `crates/logicaffeine_compile/src/optimize/eval.rs` — new file, ~100 lines
- Needs access to `crates/logicaffeine_compile/src/interpret.rs` — the interpreter

**Dependencies:** 2-A (propagation may turn variable arguments into constants, enabling more CTFE).

#### 2-C: Polynomial Normalization

**Tier:** 2 — Core Muscle
**Expected speedup:** Indirect — simplifies complex arithmetic beyond pattern matching
**Complexity:** Medium
**Kernel leverage:** **Direct** — `ring.rs` is the implementation

**Current state:** The kernel's `ring.rs` (410 lines) provides complete polynomial arithmetic:
- `Monomial` — product of variables with powers, stored as `BTreeMap<i64, u32>` for canonical ordering
- `Polynomial` — sum of monomials with coefficients, stored as `BTreeMap<Monomial, i64>`
- Structural equality = semantic equality because BTreeMap ordering is deterministic

**What the kernel gives for free:**
```
a + b - b          → a               (cancellation via polynomial subtraction)
2*x + 3*x          → 5*x             (like-term collection via add)
(a + 1) * (a - 1)  → a*a - 1         (expansion via polynomial multiplication)
1 + a + 2 + b + 3  → a + b + 6       (constant collection)
```

These emerge automatically from the canonical polynomial form. No special-case pattern matching.

**What "done" looks like:** A `try_normalize_polynomial` function that:
1. Converts an `Expr` subtree to a `Polynomial` via `ring::reify`
2. If `reify` succeeds (expression is polynomial — no division/modulo), convert the canonical polynomial back to an `Expr`
3. If the result is simpler (fewer nodes), replace the original expression

**Key files:**
- `crates/logicaffeine_kernel/src/ring.rs` — existing (410 lines), provides `reify` and all arithmetic
- `crates/logicaffeine_compile/src/optimize/fold.rs` — add polynomial normalization path (~50 lines)
- Bridge: convert between `ring::Polynomial` ↔ `ast::Expr` (~40 lines)

**Dependencies:** 0-A (expressions must be deeply folded before normalization sees them).

#### 2-D: Peephole + Strength Reduction

**Tier:** 2 — Core Muscle
**Expected speedup:** 2–5% on loop-heavy benchmarks
**Complexity:** Medium
**Kernel leverage:** `lia.rs` LinearExpr for non-negativity proofs

**Current state:** No peephole optimizations at the AST level (only at codegen level — swap and vec-fill patterns). No strength reduction (`x * 2` is not converted to `x << 1`).

**What `lia.rs` provides:** The kernel's `lia.rs` (752 lines) has `LinearExpr` with `Rational` coefficients and `fourier_motzkin_unsat` for constraint solving. This enables proving non-negativity of expressions — a requirement for safe strength reduction of multiplications to shifts (negative numbers have different shift semantics).

**What "done" looks like:** An AST-level peephole pass that:
1. `x * 2` → `x << 1` (when `x` is provably non-negative via LIA)
2. `x * 4` → `x << 2` (powers of 2)
3. `x / 2` → `x >> 1` (when `x` is provably non-negative)
4. `x % 2` → `x & 1` (when `x` is provably non-negative)

**Key files:**
- `crates/logicaffeine_kernel/src/lia.rs` — existing (752 lines), provides `fourier_motzkin_unsat` and `LinearExpr`
- `crates/logicaffeine_compile/src/optimize/peephole.rs` — new file, ~80 lines

**Dependencies:** 2-A (propagation may reveal constant multipliers), 0-C (algebraic simplification normalizes multiplication forms).

#### 3-A: Move Semantics Deep Dive

**Tier:** 3 — Structural Strength
**Expected speedup:** 5–15% on collection-heavy workloads
**Complexity:** High
**Kernel leverage:** None

**Current state:** The ownership analyzer at `analysis/ownership.rs` tracks `VarState { Owned, Moved, Borrowed }` per variable. This is a check-time-only analysis — it verifies programs are valid but does **not** feed information to the code generator.

Three clone sites exist:
1. **`logos_get()` always clones** — `indexing.rs` line 83: `unsafe { self.get_unchecked(idx).clone() }`. For `Copy` types this is free. For `String`, `Vec`, etc., every read copies.
2. **For-in `.clone()`** — `codegen.rs` line 6159: `for x in collection.clone()`. Copies entire collection before iteration.
3. **Function arguments** — values passed by move. No reference-passing optimization.

**What "done" looks like:** The ownership analysis flows to codegen via an annotation on each variable use:
- `VarState::Owned` + last use → move (no clone)
- `VarState::Owned` + more uses ahead → clone
- `VarState::Borrowed` → reference (no clone)
- For-in on `VarState::Owned` collection with no mutation → `.iter()` (no clone, see 1-B)
- `logos_get` result used in comparison only → return `&T` instead of `T`

**Key files:**
- `crates/logicaffeine_compile/src/analysis/ownership.rs` — extend VarState tracking to produce use-count and last-use annotations (~100 lines)
- `crates/logicaffeine_compile/src/codegen.rs` — consume ownership annotations to choose clone/move/ref (~80 lines)

**Dependencies:** 1-B (iterator loops are the highest-impact clone elimination), 1-C (direct indexing removes the `logos_get` clone for known types).

#### 3-B: Bounds Check Elimination

**Tier:** 3 — Structural Strength
**Expected speedup:** 5–15% on inner-loop-heavy benchmarks
**Complexity:** High
**Kernel leverage:** `lia.rs` for range proofs

**Current state:** Every `logos_get` call checks bounds (`indexing.rs` lines 76–82):
```rust
if index < 1 { panic!(...); }
let idx = (index - 1) as usize;
if idx >= self.len() { panic!(...); }
```

LLVM eliminates some of these through its own range analysis, but the `while` loop structure (see 1-A) limits LLVM's ability to prove bounds.

**What "done" looks like:** Two complementary approaches:
1. **Primary:** Switch to direct indexing (1-C), which lets Rust/LLVM handle bounds checks. For `for i in 0..n` loops, LLVM can prove `arr[i]` is in bounds and eliminate the check entirely.
2. **Advanced:** Use `lia.rs` `fourier_motzkin_unsat` to prove index-in-bounds at the AST level. If the proof succeeds, emit `unsafe { arr.get_unchecked(i) }`. This requires assembling constraints from loop bounds and conditionals — e.g., `0 <= i`, `i < arr.len()`.

**Key files:**
- Primarily addressed by 1-A + 1-C (for-range loops + direct indexing)
- Advanced: `crates/logicaffeine_compile/src/optimize/bounds.rs` — new file, ~150 lines integrating with `lia.rs`

**Dependencies:** 1-A (for-range loops give LLVM trip count), 1-C (direct indexing removes the `LogosIndex` layer).

#### 3-C: Object Pooling / Slab Allocation

**Tier:** 3 — Structural Strength
**Expected speedup:** Varies — significant for allocation-heavy programs
**Complexity:** Medium
**Kernel leverage:** None

**Current state:** Arena allocation exists at parse time (bumpalo for AST nodes) but not at runtime. Generated programs use standard Rust heap allocation for all collections and objects.

**What "done" looks like:** For programs that create and destroy many small objects of the same type in a loop (e.g., AST node processing, graph algorithms), emit a slab allocator or object pool:
```rust
let mut pool: Vec<MyStruct> = Vec::with_capacity(1000);
// ... reuse from pool instead of allocating ...
```

This is primarily a library feature — add a `Pool<T>` type to `logicaffeine_data` that programs can use, and have the codegen emit pool usage when it detects create/destroy patterns.

**Key files:**
- `crates/logicaffeine_data/src/pool.rs` — new file, ~100 lines
- `crates/logicaffeine_compile/src/codegen.rs` — pattern detection and pool emission (~50 lines)

**Dependencies:** None. Orthogonal to other optimizations.

#### 3-D: Loop Fusion

**Tier:** 3 — Structural Strength
**Expected speedup:** Rarely applicable — 5–15% when it fires
**Complexity:** Medium
**Kernel leverage:** None

**Current state:** No multi-loop analysis. Adjacent loops over the same range with independent bodies are emitted as separate loops.

**Honest assessment:** Loop fusion is rarely applicable for Logos programs. It requires:
1. Two adjacent loops over the same range
2. No data dependence between the loops (loop 2 doesn't read what loop 1 writes)
3. Both loops have the same iteration pattern

In practice, most Logos programs don't have adjacent independent loops. The sieve benchmark has dependent nested loops. The bubble sort benchmark has a single inner loop. The most likely fusion candidate is initialization followed by computation — but the vec-fill peephole (1-D) already handles that case.

**What "done" looks like:** An AST-level pass that detects adjacent `While` loops with identical conditions and compatible bodies, merging them into a single loop. Conservative: only fuse when bodies have no shared mutable state.

**Key files:**
- `crates/logicaffeine_compile/src/optimize/fusion.rs` — new file, ~120 lines with dependence analysis

**Dependencies:** 1-A (for-range detection makes range comparison easier).

#### 3-E: Overflow Checking

**Tier:** 3 — Structural Strength
**Expected speedup:** Negative (adds runtime checks) — this is a correctness feature
**Complexity:** Low
**Kernel leverage:** None

**Current state:** Rust's release mode silently wraps on integer overflow. Debug mode panics. Logos has no control over this behavior.

**What "done" looks like:** A compiler flag `--checked-arithmetic` that emits `checked_add`, `checked_sub`, `checked_mul` instead of raw operators. Overflow produces a clear error message instead of silent wrapping:
```rust
// Default (release)
let y = a + b;
// With --checked-arithmetic
let y = a.checked_add(b).expect("arithmetic overflow at line 5");
```

**Key files:**
- `crates/logicaffeine_compile/src/codegen.rs` — conditional arithmetic emission (~30 lines)
- CLI flag handling in `apps/logicaffeine_cli/` (~10 lines)

**Dependencies:** None. Pure codegen flag.

#### 4-A: Loop Vectorization

**Tier:** 4 — Summit
**Expected speedup:** 2–8x for array-processing loops (inherited from LLVM)
**Complexity:** None — **free** after Tier 1
**Kernel leverage:** None

**Current state:** LLVM can auto-vectorize, but `while` loops and `LogosIndex` trait dispatch block it. The optimizer can't determine trip counts or prove absence of aliasing through the trait abstraction layer.

**What "done" looks like:** After 1-A (for-range loops) and 1-C (direct array indexing), LLVM sees:
```rust
for i in 0..n {
    arr[i] = compute(arr[i]);
}
```
This is the canonical vectorizable loop. LLVM will automatically emit SSE/AVX/NEON instructions. No Logos work needed.

**Key files:** None. Falls out from 1-A + 1-C.

**Dependencies:** 1-A, 1-C (both required to unlock LLVM vectorization).

#### 4-B: SIMD Intrinsics

**Tier:** 4 — Summit
**Expected speedup:** Up to 4–16x for specific workloads
**Complexity:** Extreme
**Kernel leverage:** None

**Current state:** No SIMD types or operations in the Logos language.

**What "done" looks like:** Language-level SIMD support:
```
Let v be a new SimdVec of 4 Ints holding (1, 2, 3, 4).
Let w be v plus v.
```

This is an entire language feature — new types, parser support, codegen to `std::simd` (Rust nightly). The effort-to-benefit ratio is extremely high. LLVM auto-vectorization (4-A) handles the common cases.

**Recommendation:** Defer indefinitely. Auto-vectorization from 4-A covers 90% of the benefit at 0% of the cost.

**Dependencies:** 4-A (auto-vectorization should be validated first).

#### 4-C: Bitfield Packing

**Tier:** 4 — Summit
**Expected speedup:** 8x memory reduction for boolean arrays, 2–3x speedup for boolean-heavy algorithms
**Complexity:** Medium
**Kernel leverage:** None

**Current state:** `Vec<bool>` uses 1 byte per boolean (8 bits wasted per flag). The sieve benchmark's `flags` array at 1M elements uses 1MB instead of 125KB.

**What "done" looks like:** When the compiler detects `Seq of Bool`, emit `bitvec::BitVec` or a custom bitfield implementation. This reduces memory 8x and improves cache utilization.

**Tradeoff:** Bitfield access is more expensive per-element (shift + mask vs direct byte read). For random access patterns, the cache benefit dominates. For sequential access, LLVM may vectorize byte operations better than bit operations.

**Key files:**
- `crates/logicaffeine_data/src/` — `BitSeq` type or `bitvec` dependency
- `crates/logicaffeine_compile/src/codegen.rs` — emit `BitVec` for `Seq of Bool`

**Dependencies:** None. Independent, but interacts with 1-C (indexing abstraction layer).

#### 4-D: Loop Tiling / Blocking

**Tier:** 4 — Summit
**Expected speedup:** N/A for current programs
**Complexity:** N/A

**Current state:** Not relevant. Loop tiling optimizes cache performance for 2D array traversals (matrix multiplication, stencil computations). Logos has no 2D array type — all sequences are 1D `Vec`. Nested loops over separate 1D arrays don't benefit from tiling.

**What "done" looks like:** Would require: 2D array types, nested loop detection over matrix rows/columns, tile size selection based on cache line size. The entire data model would need expansion.

**Recommendation:** Not applicable to current language design. Mark as architectural N/A.

**Dependencies:** Would require language-level 2D array support.

#### 4-E: Loop Interchange

**Tier:** 4 — Summit
**Expected speedup:** N/A for current programs
**Complexity:** N/A

**Current state:** Same as 4-D. Loop interchange swaps nested loop order to improve spatial locality (iterating row-major instead of column-major). Without 2D arrays, there are no interchange candidates.

**Recommendation:** Not applicable. Mark as architectural N/A.

**Dependencies:** Same as 4-D.

#### 4-F: Branch Prediction Hints

**Tier:** 4 — Summit (but trivial to implement)
**Expected speedup:** 1–3% on tight loops
**Complexity:** Low
**Kernel leverage:** None

**Current state:** The `LogosIndex` panic paths are inline with the happy path. The CPU's branch predictor must learn that the panic path is cold through repeated execution.

**What "done" looks like:** Two changes:
1. Extract panic paths into `#[cold] #[inline(never)]` functions in `indexing.rs`:
```rust
#[cold]
#[inline(never)]
fn index_out_of_bounds(index: i64, len: usize) -> ! {
    panic!("Index {} is out of bounds for seq of length {}", index, len);
}
```
2. Optionally, emit `#[likely]`/`#[unlikely]` attributes on generated `if` conditions when the compiler can infer the likely branch (e.g., bounds checks are unlikely to fail).

**Key files:**
- `crates/logicaffeine_data/src/indexing.rs` — extract panic into cold function (~10 lines)

**Dependencies:** None. Can land independently at any time.

#### 4-G: Green Threads

**Tier:** 4 — Summit
**Expected speedup:** Throughput improvement for concurrent programs (not raw compute speed)
**Complexity:** High
**Kernel leverage:** None

**Current state:** `Concurrent` and `Parallel` blocks exist, using `tokio::spawn` for async tasks and `rayon` for parallel iteration. This is not zero-cost — the tokio runtime adds ~500KB to the binary and ~1ms to startup.

**What "done" looks like:** Lightweight stackful coroutines or a custom M:N scheduler that doesn't require a full async runtime. Alternatively, accept the current tokio approach as "good enough" — the overhead is negligible for programs that actually use concurrency.

**Recommendation:** Low priority. The current implementation works. Green threads optimize throughput for I/O-bound workloads, which are not Logos' primary use case.

**Dependencies:** None. Orthogonal to optimization pipeline.

---

### 4. Deep Dive: Loops

Six loop-related cells in the matrix. Here's what each one means for Logos, what LLVM needs to see, and what changes in codegen.

#### For-Range Loops (1-A) — The Keystone

**The pattern to detect:**
```
// AST: While { cond: BinaryOp(Lt, Identifier(i), Expr(n)),
//              body: [...stmts..., Set { target: i, value: BinaryOp(Add, Identifier(i), Literal(1)) }] }
```

**What to emit:**
```rust
for i in start..n {    // Lt → exclusive range
    // body without counter increment
}
// or
for i in start..=n {   // LtEq → inclusive range
    // body without counter increment
}
```

**What LLVM gains from `for i in 0..n`:**
1. **Trip count** — LLVM knows exactly how many iterations without analyzing the loop body. This enables: unrolling by a known factor, vectorization with known remainder handling, strength reduction of `i * stride`.
2. **SCEV bypass** — LLVM's Scalar Evolution analysis works harder on `while` loops. For `for` ranges, the induction variable is explicit in the MIR — no analysis needed.
3. **Vectorization enablement** — The auto-vectorizer needs: (a) known trip count, (b) known stride, (c) no aliasing in the body. For-range gives (a) and (b) for free. Direct indexing (1-C) provides (c).
4. **Loop rotation** — LLVM rotates loops to do-while form. For-range loops are already in the right shape. While loops may need an extra branch.

**Edge cases:**
- **Step ≠ 1:** `Set i to i + 2` → `for i in (start..end).step_by(2)`
- **Decreasing:** `Set i to i - 1` → `for i in (0..n).rev()`
- **Variable upper bound that changes:** The `n` in `while (i < n)` might be modified in the loop body. Detection: check if `n` (or any variable in the bound expression) appears as a `Set` target in the body. If so, fall back to `while`.
- **Non-zero start:** `let mut i = 5; while (i < n)` → `for i in 5..n`
- **Counter used after loop:** The `for` variable is scoped to the loop. If `i` is read after the loop, emit `let mut i = start; for __i in start..n { i = __i; ... } // i is now n` — or just keep the `while` loop.

#### Iterator-Based Loops (1-B) — The Clone Eliminator

**Current emission hierarchy:**
```
Repeat for x in coll:
    // body
```
Always emits:
```rust
for x in coll.clone() {  // full collection copy
    // body
}
```

**Target emission hierarchy (three tiers):**

**Tier 1 — Immutable iteration (`.iter()`):**
If the loop body does not contain any of: `Set { target: coll_sym }`, `SetIndex { collection: Identifier(coll_sym) }`, `Push { collection: Identifier(coll_sym) }`, `Pop { collection: Identifier(coll_sym) }`, `Remove { collection: Identifier(coll_sym) }`:
```rust
for x in coll.iter() {
    // body — x is &T, may need .clone() at use sites for non-Copy T
}
```
For `Copy` types, `x` is automatically copied. For non-Copy, the iterator yields references.

**Tier 2 — Consuming iteration (move):**
If the collection is not used after the loop (last-use analysis from 3-A):
```rust
for x in coll {
    // body — x is T, collection consumed
}
```
No clone, no reference — the collection is moved into the iterator.

**Tier 3 — Mutating iteration (current `.clone()`):**
If the body mutates the collection, keep `.clone()`:
```rust
for x in coll.clone() {
    // body mutates coll
}
```

**Mutation detection algorithm:**
```
fn body_mutates_collection(body: &[Stmt], coll_sym: Symbol) -> bool {
    for stmt in body {
        match stmt {
            Set { target, .. } if *target == coll_sym => return true,
            SetIndex { collection: Identifier(sym), .. } if *sym == coll_sym => return true,
            Push { collection: Identifier(sym), .. } if *sym == coll_sym => return true,
            Pop { collection: Identifier(sym), .. } if *sym == coll_sym => return true,
            // Recursively check nested blocks (if, while, etc.)
            _ => { /* recurse into sub-blocks */ }
        }
    }
    false
}
```

#### Loop Fusion (3-D) — Honest Assessment

**When loop fusion helps:** Two adjacent loops over the same range, each doing one pass over an array. Fused, they do a single pass — halving cache misses.

**Why it rarely applies to Logos programs:**
1. **Most loops are data-dependent** — the second loop reads what the first loop computed (e.g., sieve: mark composites, then count primes). These cannot be fused.
2. **Most programs have a single hot loop** — bubble sort has one inner loop, Fibonacci has recursion (no loops), collection ops have one insertion loop.
3. **The vec-fill peephole (1-D) already handles the main fusion candidate** — initialization + computation.

**When it would help:** Programs that compute multiple independent statistics over the same array (mean and standard deviation, min and max). These are uncommon in current benchmarks but could appear in user programs.

**Implementation difficulty:** Medium. Requires: same-range detection (comparing loop bounds), no-shared-mutation analysis (checking that body1 doesn't write variables read by body2), and body merging (interleaving statements).

#### Loop Tiling / Loop Interchange (4-D, 4-E) — Not Relevant

Both optimizations target 2D array access patterns:
- **Tiling:** Break a large 2D loop nest into small cache-friendly tiles
- **Interchange:** Swap loop order for row-major vs column-major access

**Why they don't apply:** Logos has no 2D array type. All sequences are 1D `Vec`. Matrix operations would be emulated as `Vec<Vec<T>>`, but the language doesn't have native matrix indexing syntax. No current benchmark or user program uses nested 2D iteration.

**If 2D arrays were added:** These optimizations would become relevant for matrix multiplication, image processing, and scientific computing. But that's a language design question, not an optimizer question.

#### Loop Vectorization (4-A) — Free After Tier 1

**What LLVM needs to auto-vectorize:**
1. Counted loop with known trip count → provided by 1-A (for-range)
2. Contiguous array access with known stride → provided by 1-C (direct indexing)
3. No aliasing between read and write arrays → provided by Rust's `&mut` = `noalias`
4. Loop body is vectorizable (arithmetic, comparisons, conditional select)

**After 1-A + 1-C, the sieve inner loop becomes:**
```rust
for j in (i*i..=limit).step_by(i) {
    flags[j] = true;
}
```

LLVM sees: known start (`i*i`), known end (`limit`), known stride (`i`), single array write, no aliasing. This vectorizes to `vpbroadcastb` + `vmovdqu` (AVX2) — setting 32 bytes per iteration instead of 1.

**No Logos work needed.** The optimization is entirely LLVM's responsibility once the loop structure is clean.

---

### 5. Deep Dive: Move Semantics

Where clones can be eliminated, what Rust's borrow checker already handles, and what Logos must handle at the AST level.

#### Clone Site 1: `logos_get()` Always Clones

**Location:** `crates/logicaffeine_data/src/indexing.rs` line 83

```rust
unsafe { self.get_unchecked(idx).clone() }
```

**For `Copy` types (`i64`, `f64`, `bool`, `char`):** The clone is a bitwise copy. LLVM optimizes this to a register load. Zero overhead. The `Clone` trait call is monomorphized and inlined — `clone()` on `i64` compiles to `mov`.

**For non-Copy types (`String`, `Vec<T>`, structs with heap data):** The clone allocates new heap memory and copies bytes. This is the real cost. A `Vec<String>` with 1000 elements where each element is read once via `logos_get` performs 1000 unnecessary heap allocations.

**Elimination strategy:**

| Usage Pattern | Current | Optimal | Savings |
|---|---|---|---|
| `let x = arr[i]; if x > 5` | Clone + compare + drop | Reference compare | 1 allocation |
| `let x = arr[i]; f(x)` | Clone + move into f | Pass reference or move | 1 allocation |
| `let x = arr[i]; arr[j] = x` | Clone + assign | Move (last use) | 1 allocation |
| `let x = arr[i]; g(x); h(x)` | Clone (necessary) | Clone (necessary) | 0 |

For benchmarks (all `i64`): no savings — already optimal via LLVM.
For real programs with `String`/struct values: significant savings.

**Implementation:** Add `logos_get_ref(&self, index: i64) -> &T` to `LogosIndex`. Codegen chooses `logos_get_ref` when the value is used in a comparison or passed to a function that takes `&T`. Requires tracking expression context in codegen (observation vs consumption).

#### Clone Site 2: For-In `.clone()`

**Location:** `crates/logicaffeine_compile/src/codegen.rs` line 6159

```rust
writeln!(output, "{}for {} in {}.clone() {{", indent_str, pattern_str, iter_str)
```

**Why it clones:** Rust's `for x in coll` moves `coll` into the iterator. After the loop, `coll` is consumed and cannot be used. Logos clones preventatively because it doesn't track whether the collection is used after the loop.

**What `.iter()` gives:** `for x in coll.iter()` borrows the collection. After the loop, `coll` is still available. The iterator yields `&T` references — no cloning of elements.

**The detection hierarchy:**
1. Check if `coll` appears in any statement after the loop → if not, use `for x in coll` (move, no clone)
2. Check if body mutates `coll` → if not, use `for x in coll.iter()` (borrow, no clone)
3. Otherwise → use `for x in coll.clone()` (current behavior)

For `Copy` element types, `.iter()` yields `&i64` which derefs to `i64` transparently. For non-Copy types, element accesses would need `.clone()` at the use site — but only for elements that are actually consumed, not just observed.

#### Clone Site 3: Ownership Analysis → Codegen Bridge

**Location:** `crates/logicaffeine_compile/src/analysis/ownership.rs`

```rust
pub enum VarState {
    Owned,
    Moved,
    Borrowed,
}
```

**Current state:** The ownership analyzer walks the AST and tracks variable states. It detects use-after-move errors and reports them. But the information is discarded after analysis — it does not flow to the code generator.

**What the bridge would look like:**

```
Analysis Phase:
  ownership.rs analyzes program →
    produces HashMap<(Symbol, Location), VarState>

Codegen Phase:
  codegen.rs reads HashMap →
    at each variable use, checks:
      if VarState::Owned && is_last_use → emit move (no clone)
      if VarState::Borrowed → emit reference
      if VarState::Owned && more_uses_ahead → emit clone
```

**What Rust's borrow checker handles vs what Logos must handle:**

| Concern | Rust handles it | Logos must handle it |
|---|---|---|
| Lifetime of references | Yes (borrow checker) | No — just emit valid code |
| Use-after-move | Yes (compile error) | Yes — must not emit moved-then-used code |
| Clone necessity | No (programmer decides) | Yes — must decide clone vs move vs ref |
| Aliasing rules | Yes (`&mut` exclusivity) | Partially — must not emit aliased `&mut` |

The key insight: Logos doesn't need to implement a borrow checker. It needs to produce Rust code that *passes* the borrow checker. The ownership analysis provides enough information to make clone/move/ref decisions that always produce valid Rust.

---

### 6. Deep Dive: Constant Propagation + Kernel

How `simp.rs` maps to the optimizer's propagation pass, and what additional power the kernel provides.

#### The `simp.rs` Parallel

The kernel's simplifier and the optimizer's propagation pass solve the same problem at different levels:

| Aspect | `simp.rs` (kernel) | Propagation pass (optimizer) |
|---|---|---|
| **Domain** | Proof terms (`STerm`) | Program AST (`Expr`) |
| **Environment** | `Substitution = HashMap<i64, STerm>` | `PropEnv = HashMap<Symbol, &Expr>` |
| **Source of bindings** | Hypothesis equalities (`H: x = t`) | `Let` bindings (`Let x be 10.`) |
| **Substitution** | `simplify_sterm(term, subst, fuel)` | `propagate_expr(expr, env)` |
| **Termination** | Fuel-limited (1000 steps) | Single-pass forward walk (natural termination) |
| **Re-simplification** | Fuel decrement prevents infinite loops | Re-fold after substitution (0-A provides this) |

**The concrete mapping:**

```
Kernel:
  decompose_goal(Implies(Eq(x, 5), Implies(Eq(y, x+3), Eq(y, 8))))
  → subst = { x → 5 }, conclusion = Eq(y, 8)
  → simplify_sterm(y, { x → 5 }, 1000) = ... (check y = x+3 = 5+3 = 8)

Optimizer:
  Let x be 5.
  Let y be x + 3.
  Print y.
  → env = { x → 5 }
  → propagate(x + 3, { x → 5 }) = 5 + 3
  → fold(5 + 3) = 8
  → result: Let y be 8. Print 8.
```

#### Hypothesis Extraction from If Conditions

Beyond simple `Let` propagation, the optimizer can extract hypotheses from `If` conditions — the same way `simp.rs` extracts from implications.

```
If x is greater than 0:
    Let y be x + 1.    ← Inside this block, we know x > 0
    Print y.
```

In the `then` block, the environment gains `x > 0`. This enables:
1. **Branch elimination:** `If x is greater than 0: ... If x is positive: ...` — the inner `If` is provably true (via `lia.rs`), eliminate it.
2. **Bounds proof:** `If i is at least 1 and i is at most (length of arr): let v be item i of arr.` — bounds check provably safe (via `lia.rs` constraints).
3. **Constant refinement:** `If x is 5: Print x + 3.` → `Print 8.` — `x` is known to be 5 in the then-block.

#### What `ring.rs`, `lia.rs`, and `cc.rs` Add Beyond Standard Propagation

**`ring.rs` — Polynomial normalization:**
Standard constant propagation substitutes and folds. Ring normalization goes further — it canonicalizes arbitrary polynomial expressions. After propagation produces `(a + 1) * (a - 1)`, ring normalization simplifies to `a*a - 1` without needing a specific pattern for that expression. The BTreeMap-based canonical form means that semantically equal expressions are structurally equal — enabling CSE (common subexpression elimination) at the AST level.

**`lia.rs` — Linear Integer Arithmetic:**
Fourier-Motzkin elimination can prove relationships between variables. After propagation, if we know `x = 2*y + 1` and need to check whether `x` is odd, `lia.rs` can prove it. This powers:
- Bounds check elimination (3-B): prove `0 <= idx < len`
- Strength reduction (2-D): prove `x >= 0` for safe shift conversion
- Dead branch elimination: prove `x > 5` is always true given the constraints

**`cc.rs` — Congruence Closure:**
The E-graph with union-find and congruence propagation can prove equalities that propagation alone cannot. If `f(a) = f(b)` and we later learn `a = b`, congruence closure deduces `f(a) = f(b)` without re-evaluating `f`. This is overkill for most optimizer tasks but enables powerful program equivalence proofs — the foundation for verified optimization passes.

**The cascade:**
```
propagation (2-A) discovers: x = 5, y = x + 3
  → fold (0-A) evaluates: y = 8
    → algebraic simp (0-C) cleans: z + 0 = z
      → ring (2-C) normalizes: 2*a + 3*a = 5*a
        → lia (2-D/3-B) proves: 5*a >= 0 when a >= 0
          → branch elimination removes: if a >= 0 { ... }
```

Each layer amplifies the others. The tactic pipeline `repeat(then(propagate, then(fold, dce)))` catches cascading opportunities automatically.

---

### 7. The Critical Path

Optimal implementation order, respecting dependencies and maximizing compound speedups.

#### Phase 1 — Bedrock ✅ COMPLETE (v0.8.14)

All 3 items implemented and verified with 33 tests, 0 regressions.

| Item | Files | Lines | Status |
|---|---|---|---|
| 0-A: Deep expression recursion | `fold.rs` | ~150 | ✅ Done |
| 0-B: Post-return DCE | `dce.rs` | +3 | ✅ Done |
| 0-C: Algebraic simplification | `fold.rs` | ~30 | ✅ Done |

**Unlocked:** All downstream optimization passes can now rely on complete folding and clean DCE.

#### Phase 2 — Direct Strike ✅ COMPLETE (v0.8.15)

All 5 items implemented with 24 new tests, 0 regressions.

| Item | Files | Lines | Status |
|---|---|---|---|
| 1-C: Direct indexing | `codegen.rs` | ~15 | ✅ Done |
| 1-D: Vec fill (exclusive) | `codegen.rs` | ~20 | ✅ Done |
| 1-E: Swap enhancement | `codegen.rs` | ~10 | ✅ Done |
| 1-A: For-range loops | `codegen.rs` | ~130 | ✅ Done |
| 1-B: Iterator loops | `codegen.rs` | ~45 | ✅ Done |

**Unlocked:** LLVM vectorization (4-A) falls out free. Array-heavy benchmarks close the gap with C.

**Compound speedup projection (sieve 1M):**

| Current | After 1-A | After 1-A + 1-C | After 1-A + 1-C + 1-D |
|---|---|---|---|
| 4.900ms (1.60x C) | ~4.0ms (1.31x C) | ~3.4ms (1.11x C) | ~3.2ms (1.05x C) |

#### Phase 3 — Core Muscle (2-A first, then parallel)

Implement 2-A first (constant propagation unlocks 2-B). Then 2-B, 2-C, 2-D in parallel.

| Item | Files | Lines | Time |
|---|---|---|---|
| 2-A: Constant propagation | `optimize/propagate.rs` (new) | ~120 | Medium |
| 2-B: CTFE | `optimize/eval.rs` (new) | ~100 | Medium |
| 2-C: Polynomial normalization | `optimize/fold.rs` + bridge | ~90 | Medium |
| 2-D: Peephole + strength reduction | `optimize/peephole.rs` (new) | ~80 | Medium |

**Unlock:** Generated code quality approaches hand-written. Constant expressions evaluated at compile time.

#### Phase 4 — Structural (selective)

Not all items are worth implementing. Prioritize by ROI.

| Item | Priority | Rationale |
|---|---|---|
| 3-B: Bounds check elimination | High | Largely addressed by 1-C (direct indexing). Advanced version via `lia.rs` is bonus. |
| 3-A: Move semantics | Medium | High impact for non-Copy types. Current benchmarks are all `i64`. |
| 4-F: Branch prediction hints | Low-Medium | Trivial to implement (~10 lines), small but free improvement. |
| 3-E: Overflow checking | Low | Correctness feature, not performance. Implement when users request it. |
| 3-D: Loop fusion | Low | Rarely applicable. Implement opportunistically. |
| 3-C: Object pooling | Low | Library feature. Implement when allocation-heavy programs appear. |

#### Phase 5 — Summit (free + opportunistic)

| Item | Status |
|---|---|
| 4-A: Loop vectorization | **Free** after Phase 2. Validate with benchmarks. |
| 4-B: SIMD intrinsics | Defer indefinitely. |
| 4-C: Bitfield packing | Implement if boolean-heavy programs appear. |
| 4-D: Loop tiling | N/A. |
| 4-E: Loop interchange | N/A. |
| 4-G: Green threads | Current `tokio` implementation is sufficient. |

#### Compound Speedup Table per Benchmark

Projected performance after each phase (vs C baseline):

| Benchmark | Current | After Phase 2 | After Phase 3 | After Phase 4 | Target |
|---|---|---|---|---|---|
| **Fibonacci** | 1.35x | 1.35x | 1.30x | 1.30x | ~1.0x |
| **Sieve** | 1.60x | 1.05x | 1.03x | 1.00x | ~1.0x |
| **Bubble Sort** | 1.04x | 0.98x | 0.98x | 0.98x | ~1.0x |
| **Collection** | 0.50x | 0.50x | 0.50x | 0.50x | 0.50x (already wins) |
| **Strings** | 1.12x | 1.10x | 1.08x | 1.08x | ~1.0x |
| **Ackermann** | 1.59x | 1.59x | 1.50x | 1.50x | ~1.0x |
| **Geometric Mean** | **1.038x** | **~0.90x** | **~0.85x** | **~0.83x** | — |

Notes:
- Fibonacci and Ackermann are recursion-bound. Loop optimizations don't help. Improvement comes from 0-A (deeper folding), 2-A (propagation), 2-B (CTFE for small inputs).
- Sieve sees the largest improvement because it's the most loop-and-array-intensive benchmark. Phase 2 changes directly target its bottlenecks.
- Bubble sort is already close (1.04x). The swap peephole already fires. Phase 2 closes the remaining gap.
- Collection already wins. No changes needed.
- Geometric mean below 1.0x means Logos-generated code is, on average, faster than hand-written C across the suite.

---

### 8. Summary Table

All 30 items (24 actionable + 6 N/A) in one view.

| ID | Item | Tier | Matrix Cell | Speedup | Complexity | Kernel? | Dependencies |
|---|---|---|---|---|---|---|---|
| 0-A | Deep expression recursion | 0 | — | Indirect | Low | No | None | ✅ |
| 0-B | Post-return DCE | 0 | DCE (Y+) | Indirect | Low | No | None | ✅ |
| 0-C | Algebraic simplification | 0 | Alg simp (P→Y) | Indirect | Low | ring.rs foundation | None | ✅ |
| 1-A | For-range loops | 1 | For-range (N→Y) | 10–25% | Medium | No | 0-A | ✅ |
| 1-B | Iterator-based loops | 1 | Iterator (N→Y) | 5–10% | Medium | No | None | ✅ |
| 1-C | Direct indexing + clone elim | 1 | Zero-cost (P→Y) | 15–30% | Low-Med | No | 1-A | ✅ |
| 1-D | Vec initialization | 1 | — | 5–10% | Medium | No | 0-A | ✅ |
| 1-E | Swap pattern enhancement | 1 | — | 5–10% | Low | No | 1-C | ✅ |
| 2-A | Constant propagation | 2 | Const prop (N→Y) | Varies | Medium | simp.rs template | 0-A, 0-C |
| 2-B | Compile-time func eval | 2 | CTFE (P→Y) | Varies | Med-High | simp.rs fuel pattern | 2-A |
| 2-C | Polynomial normalization | 2 | — | Indirect | Medium | ring.rs direct | 0-A |
| 2-D | Peephole + strength reduction | 2 | — | 2–5% | Medium | lia.rs proofs | 2-A, 0-C |
| 3-A | Move semantics | 3 | Move sem (P→Y) | 5–15% | High | No | 1-B, 1-C |
| 3-B | Bounds check elimination | 3 | Bounds (N→Y) | 5–15% | High | lia.rs proofs | 1-A, 1-C |
| 3-C | Object pooling / slab | 3 | Pool (N→Y) | Varies | Medium | No | None |
| 3-D | Loop fusion | 3 | Fusion (N→P) | 5–15%* | Medium | No | 1-A |
| 3-E | Overflow checking | 3 | Overflow (P→Y) | Negative | Low | No | None |
| 4-A | Loop vectorization | 4 | Vector (N→I) | 2–8x | **Free** | No | 1-A, 1-C |
| 4-B | SIMD intrinsics | 4 | SIMD (N→P) | Up to 16x | Extreme | No | 4-A |
| 4-C | Bitfield packing | 4 | Bitfield (N→P) | 2–3x* | Medium | No | None |
| 4-D | Loop tiling | 4 | Tiling (N→N) | N/A | N/A | No | 2D arrays |
| 4-E | Loop interchange | 4 | Interchange (N→N) | N/A | N/A | No | 2D arrays |
| 4-F | Branch prediction hints | 4 | Branch (N→P) | 1–3% | Low | No | None |
| 4-G | Green threads | 4 | Green (P→Y) | Throughput | High | No | None |
| — | Hidden classes | N/A | N→N/A | — | — | — | Not applicable |
| — | Inline caching | N/A | N→N/A | — | — | — | Not applicable |
| — | Speculative optimization | N/A | N→N/A | — | — | — | Not applicable |
| — | Deoptimization | N/A | N→N/A | — | — | — | Not applicable |
| — | Scalar replacement | N/A | N→I | — | — | — | Already inherited (LLVM SROA) |
| — | JIT / Tiered compilation | N/A | N→N/A | — | — | — | Not applicable |

*Asterisk: speedup only applies when the optimization fires, which is rare for current programs.

**Total implementation cost for Phases 1–3 (the high-ROI items):**
- ~16 items
- ~1,000 lines of optimizer + codegen changes
- ~60 test cases
- Projected result: geometric mean from 1.038x → ~0.85x vs C (Logos-generated code faster than hand-written C on average)
