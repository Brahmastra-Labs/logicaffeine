# Imperative mode — the LOGOS language

A program is read far more often than it is written, so LOGOS writes it in the language its
readers already know. Imperative LOGOS is a statically-typed programming language written in
English. The same source runs through an interpreter, a bytecode VM, and a JIT, or compiles
ahead-of-time to Rust or C, or directly to WebAssembly (see
[Execution & performance](execution-and-performance.md)).

After this guide you can write a complete program: values and types, collections, functions,
records and enums, contracts and tests, observation, tensors, modules and the standard library.
Every `logos` block below compiles, and every output shown is what it prints.

```rust
let rust = logicaffeine_compile::compile_to_rust(source)?;   // English → Rust
```

## Program structure

A program is made of `##` blocks. `## Main` is the entry point; `## To <name> …` defines a function;
`## A <Name> has:` / `## A <Name> is one of:` define types; `## Test: <sentence>` holds a test.

```logos
## Main
Let x be 5.
Set x to 10.
Show x.
```
→ prints `10`.

## Variables & mutation

`Let <name> be <expr>` binds a variable; `Set <name> to <expr>` mutates it. Type annotations are
optional (`Let x: Int be 5`); types are otherwise inferred, and a whole-number initializer for a
`Real` slot is widened (after `Let rate: Real be 3.`, `rate / 2` is `1.5`). Everyday words the language also uses
as keywords (`size`, `launch`, `note`, `record`, …) work as names in every position. The exceptions
are the literal words (`true`, `false`, `nothing`, `none`, number words), the prefix operators
(`length`, `copy`, `item`, `items`, `some`) and the statement skeleton (`let`, `set`, `be`, `to`,
`if`, `while`, …).

`Increase n by 3.` and `Decrease n by 1.` step a number in place; `Swap a and b.` exchanges two
variables of the same type. A `Reversibly:` block (alias `Uncompute:`) runs its body exactly as if it
were written inline; the bytecode VM encodes the body's in-place updates of a scalar (`+`, `-`,
`xor`, negation) as reversible instructions (`RevAdd`/`RevSub`/`RevXor`/`RevNeg`), which its reverse
executor undoes without recording history.

```logos
## Main
Let mutable n be 5.
Increase n by 3.
Decrease n by 1.
Show n.
Let mutable a be 1.
Let mutable b be 2.
Swap a and b.
Show a.
Show b.
Let mutable x be 5.
Reversibly:
    Set x to x + 3.
    Set x to x xor 1.
Show x.
```

```output
7
2
1
9
```

## Primitive types & operators

`Int` (exact: a 64-bit integer that promotes to an arbitrary-precision integer on overflow),
`Nat` (u64), `Real` (f64; `Float` is an alias), `Rational` (exact fractions), `Bool`,
`Text`, `Char`, `Byte` (u8), the machine words `Word8`/`Word16`/`Word32`/`Word64` (wrapping
ℤ/2ⁿ ring arithmetic, each at its own width: the crypto substrate), `Complex` (exact) and
`ComplexF` (floating: `complex(3.0, 4.0)`), and `Uuid` (RFC 9562, every version from nil/max
through v1–v8).

Operators: arithmetic `+ - * / %` (also the words `plus`, `minus`, `times`, `divided by`);
comparisons `== != < > <= >=` (also `is less than`, `is at most`, `is greater than or equal to`,
`is not equal to`, `is approximately x [within eps]`, …); logical `and` / `or` / `not`; bitwise
shifts and xor. A `Text` literal interpolates `{expr}` holes (any expression, read whole or
refused), and `{{`/`}}` are literal braces; `r"…"` is a raw string with no holes.

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
        return ::std::string::String::from("negative");
    }
    return ::std::string::String::from("non-negative");
}
```

Parameters are typed; return types are explicit (`-> T`) or inferred. Multiple parameters use
`and` (`## To add (a: Int) and (b: Int) -> Int:`) or commas
(`## To withdraw (balance: Int, amount: Int) -> Int:`). Recursion, closures, generics, and FFI
(`native "…"`, `is exported for native`/`wasm`) are all supported.

## Collections — 1-indexed

English speakers count from one, and so does LOGOS. Slices are inclusive on both ends.

```logos
## Main
Let items be [1, 2].
Push 3 to items.
Let first be item 1 of items.        # → 1
Let middle be items 2 through 3.     # inclusive slice
Set item 2 of items to 99.
Show item 3 of items.
```

`Seq` (sequences), `Set`, `Map`, and `Tuple` are all available, with `length of` (and
`the count of`, on every collection), `Push`/`Pop`, `Add`/`Remove`, and map/tuple indexing.
`Pop from xs into y.` takes the last item off `xs`; popping a list with no items stops the program
with `Cannot pop from xs: it is empty`.
`every x in xs where <condition>` builds a new list and leaves `xs` untouched;
`Remove every x from xs where <condition>.` removes in place and keeps the order of the rest.

```logos
## Main
Let xs be [5, 1, 4, 2, 3].
Let small be every x in xs where x is less than 4.
Show small.
Let mutable ys be [5, 1, 4, 2, 3].
Remove every y from ys where y is greater than 2.
Show ys.
Show the count of xs.
```

```output
[1, 2, 3]
[1, 2]
5
```

## Tuples & multiple return

A function takes and returns anonymous tuples `(Int, Int)` and named records
`(quotient: Int, remainder: Int)`; `Let (a, b) be <expr>.` destructures a tuple. Both round-trip
through parameters and returns identically on the tree-walker, the VM, AOT-Rust, C and WASM.

```logos
## To divmod (n: Int) -> (Int, Int):
    Return (n / 5, n mod 5).

## To parts (n: Int) -> (quotient: Int, remainder: Int):
    Return (n / 5, n mod 5).

## Main
Let (q, rem) be divmod(17).
Show q.
Show rem.
Let r be parts(17).
Show r's quotient.
Show item 2 of divmod(23).
```

```output
3
2
3
3
```

## Control flow

```logos
## Main
Let sum be 0.
Repeat for x in [1, 2, 3]:
    Set sum to sum + x.
Show sum.                            # → 6
```

`If … Otherwise …`, `While …` (with an optional `(decreasing <expr>)` termination measure),
`Repeat for <x> in <collection>` (also `Repeat for each <x> in …`), `Repeat for i from 1 to n`
(inclusive), `Break`, and `Return`.

## Structs

```logos
## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point.
Show p's x.
```

Field access and mutation use the possessive: `p's x`, `Set p's x to 100`.

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

`Inspect … When …` matches enum variants with exhaustiveness checking.

An enum is a namespace of its own variants. Two enums sharing a variant name refuse a bare
`a new <Variant>` (nothing says which one is meant); name the enum with `from`:

```logos
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Status is either:
    Up.
    Failure (code: Int).

## Main
Let a be a new Failure from Outcome with message "timeout".
Let b be a new Failure from Status with code 500.
Inspect a:
    When Success (v): Show "wrong".
    When Failure (m): Show m.
Inspect b:
    When Up: Show "wrong".
    When Failure (c): Show c.
```

```output
timeout
500
```

`from` works with or without `with …` fields, and on the bare variant word too: the same three
construction sites as an unqualified `a new <Variant>`.

`Option` and `Result` come from the standard library's `core` module. `some x` and `none` build an
`Option` (as do `a new Some with value x` and `a new None`), `When Some (v):` / `When None:` match
it, and `x is none` tests it:

```logos
## To find (xs: Seq of Int, target: Int) -> Option of Int:
    Repeat for i from 1 to length of xs:
        If item i of xs equals target:
            Return some i.
    Return none.

## Main
Inspect find([4, 7, 9], 7):
    When Some (at): Show at.
    When None: Show "absent".
Let missing be find([4, 7, 9], 5).
If missing is none:
    Show "absent".
```

```output
2
absent
```

## Ownership & borrowing

LOGOS tracks ownership. `copy of <x>` makes an explicit copy (used throughout the
[merge-sort example](../README.md#a-complete-example-merge-sort)); move/borrow verbs and
use-after-move detection are handled by the ownership analysis
([`analysis/ownership.rs`](../crates/logicaffeine_compile/src/analysis/ownership.rs)).

## I/O & assertions

`Show <expr>` prints; `Read … from console`/`from file "…"` reads; `Write "…" to file "…"` writes.
`readReals("…")` is the `Seq of Real` a file of little-endian binary64 values holds, eight bytes a value: numbers a
program reads at run time rather than carries as code, such as a model's weights (`Set W to readReals("w/W.f64").`
fills a declared tensor). A file it cannot read, or one that is not a whole number of values, stops the program.
`Assert that <logic>` with a logic proposition is the **Assert Bridge** into
[Proof & verification](proof-and-verification.md).

## Contracts & failure

A program states what must hold and stops, with its reason, when it does not:

- `Fail with <reason>.` stops the program with that reason; output already shown is kept.
- `Require that <condition>.` is an enforced invariant, checked in every build.
- `Assume [that] <condition>.` is an input contract with the same enforcement, written as an
  assumption; its range form is `Assume x is between -1.0 and 1.0.`
- `Assert that <condition>.` is a development check: the interpreters always check it, and AOT-Rust
  checks it only in a debug build.
- `, otherwise <reason>` after any of the three contracts gives the report a broken contract prints
  instead of `Assertion failed`. The reason is evaluated only when the contract breaks, and must be
  `Text`.

The report reads identically on the tree-walker, the VM, AOT-Rust and C.

```logos
## To checked (n: Int) -> Int:
    If n is less than 0:
        Fail with "negative: " + format(n).
    Return n.

## To withdraw (balance: Int, amount: Int) -> Int:
    Require that amount is at most balance, otherwise "cannot withdraw " + format(amount) + " from " + format(balance).
    Return balance - amount.

## Main
Let t be 20.
Assume t is at most 100, otherwise "t out of range: " + format(t).
Show checked(3).
Show withdraw(10, 3).

## Test: an overdraft stops with its reason
Expect this to fail with "cannot withdraw 30 from 10":
    Let left be withdraw(10, 30).
```

```output
3
7
```

`withdraw(10, 30)` in `## Main` stops the run with `cannot withdraw 30 from 10`.

## Testing

A `## Test: <sentence>` block is a test; the sentence is its name, and two tests may not share one.
`largo test` runs every test in a fresh, sealed world (a virtual clock, an in-memory filesystem and
seeded randomness) and never runs `## Main`; the definitions beside it stay callable. The flags,
exit codes and report are in [the CLI guide](cli.md#test).

```logos
## To triple (n: Int) -> Int:
    Return n times 3.

## Main
Show triple(2).

## Test: triple multiplies by three
Expect that triple(5) is 15.

## Test: triple over a table of cases

For each of these cases:

| n | product |
|---|---------|
| 0 | 0       |
| 4 | 12      |

Expect that triple(n) is product.

## Test: tripling a positive number grows it
For all integers n where n is greater than 0:
    Expect that triple(n) is greater than n.

## Test: division by zero stops the block
Let a be 10.
Let b be 0.
Expect this to fail with "Division by zero":
    Let x be a / b.

## Test: the answer prints
Show triple(3).
Expect output:
    9

## Test: time and files are sealed
Let t1 be now().
Advance time by 3 seconds.
Let t2 be now().
Expect that t2 minus t1 is 3000.
Given a file "notes.txt" containing "hello".
Read contents from file "notes.txt".
Expect that contents equals "hello".
```

| Statement | Meaning |
|-----------|---------|
| `Expect that <condition>.` | Records a failure and continues, so one run reports every broken expectation. |
| `Expect this to fail [with "<text>"]:` | The indented block must fail; the test fails if it succeeds. |
| `Expect output:` | The test's `Show` output, byte for byte, against the indented lines; `Expect output as in "<file>".` reads them from a file. |
| `For each of these cases:` | The Markdown table after it binds each column to a name; every row runs as its own named test. |
| `For all integers\|naturals\|booleans <names> [where <condition>]:` | A property: 256 seeded cases, and a failure is shrunk to a minimal counterexample. `largo test --prove` checks `booleans` exhaustively (up to 16 variables). |
| `Advance time by <duration>.` | Moves the virtual clock; nothing waits. |
| `Given a file "<path>" containing "<text>".` | Seeds the in-memory filesystem. |
| `Skip this test because "<reason>".` | Skips loudly; the reason is required. |
| `This test may use the real filesystem.` | Grants a real capability (`filesystem`, `network`, `environment`, `clock`); such a test runs only under `largo test --live`. |
| `This test may run up to <n> steps.` | Raises the deterministic step budget (default 500,000,000); a runaway test fails at the same step on every run. |

The test statements are refused outside a test. A test that checks nothing (no `Expect` and no
`For all`) fails as vacuous. A failed `Expect` prints its condition back with the value of every
name in it, as in `test_runner_lock`'s golden report:

```text
FAIL  Alpha expectations both report
      Expected that y is 2.
          y → 1
```

## Observation

`Note that …` (developer), `Warn that …` (operator) and `Record that …` (audit) are statements, not
library calls. `with <name> and <name>` attaches values, `Note once in every <n> times that …`
samples deterministically per site, and `During <gerund phrase>:` wraps a block in a trace span.
They are legal anywhere, and they never change what a program prints. The tree-walker emits each
one as an event on a channel separate from `Show`, which a host drains
(`logicaffeine_compile::observe`); built in the observed profile (`vm_outcome_observed`,
`compile_to_rust_observed` through `logicaffeine_system::obs`, `compile_to_wasm_observed`), the VM,
AOT-Rust and WASM emit the same stream. `largo` installs no sink, so a command-line run prints
nothing for them. Plain `compile_to_rust` and `compile_to_wasm` erase them before optimization: the
artifact is byte-identical to the one built from the program without them.

```logos
## Main
Let attempts be 3.
Let size be 42.
During uploading the file:
    Note that the cache missed.
    Warn that the upload failed with attempts and size.
    Note once in every 1024 times that the packet arrived.
Record that the payment settled.
Show attempts.
```

```output
3
```

## Tensors

`Let A be a <R> by <C> tensor of Int.` (or `of Real`) declares a zero-filled `R×C` tensor whose shape
the compiler knows. The parser rewrites each tensor statement into ordinary row-major `Seq` code and
loops, so every tier runs it, and the accelerator backends recognize the same loop idioms, all but a soft cap's, which
they refuse.

```logos
## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A matmul B.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
```

```output
19
22
43
50
```

Shapes are checked when the program is parsed. `A matmul B`, `A plus B` and `A hadamard B` combine
two tensors; the model operators each take a declared tensor: `softmax of X` (per row of a 2-D
tensor), `rmsnorm of X with weight W`, `layernorm of X with weight W and bias B`, `l2norm of X`,
`silu of X`, `sigmoid of X`, `gelu of X`, `swiglu of Gate and Up`, `rope of Q with cos RC and sin RS`,
`attention of Q over K and V with H heads KV groups`, `causal attention of …`,
`full attention of Q over K and V with H heads`, `gated delta of …`, `moe of X with router Wr gate Wg
up Wu down Wd choosing K of E`, `causal conv of X with weight WC state CS over T taps`,
`decay of Ar with bias Dt rate R`, `argmax of X` and `transpose of X`. `row i of T` and `rows I of T`
read rows (an embedding lookup); `Set row i of T to V.` and `Set rows from P of T to X.` write them.
A row index or position is checked at run time, with one message on every tier. The norms (`rmsnorm`,
`layernorm`, `l2norm` and the output norm of `gated delta`) divide by `sqrt(… + 1e-6)`; a model trained
with another epsilon names it after the operands: `rmsnorm of X with weight W with epsilon 1e-5`.
An attention keeps a sliding window when the program gives one after its length: `attention of Q over K
and V with 8 heads 2 groups up to pos within a window of 512` attends position `pos` and the 511 before
it, `capped at C` before the sinks takes each score `s` as `C·tanh(s/C)` before the softmax (Gemma 2's soft cap), and
`with sinks S` last gives each head a logit that joins its softmax and takes no value. `X capped at C` caps every element
of a tensor the same way. `X scaled by
C` multiplies every element of a tensor by the number `C`, and `A plus B` with `B` one row as wide as
`A`'s rows adds that row to each of them (a bias over a block). A mixture takes a bias after any of its
weights (`router Wr plus Br gate Wg plus Bg …`) and `using a swiglu clamped at 7.0 with slope 1.702`
after `choosing K of E` for gpt-oss's gated activation; `choosing 6 of 64 without renormalizing` weighs
each kept expert by its softmax over every expert, as DeepSeek-V2 does, instead of renormalizing the kept
weights to sum to one.

Heads are parts of a row, and four more forms take them apart and put them together.
`the first K of every N of X` (or `the last K`) keeps one part of every group of `N` elements, `A joined
with B` puts the rows of two tensors side by side (`across H heads`, each head's parts), and `X matmul W
across H heads` multiplies each head's slice of `X` by its own block of `W`'s rows. An attention's values
may be narrower or wider than its keys, and `scaled by S` after its window replaces the score scale `1/√HD`.
Together they write DeepSeek's latent attention with each head's query absorbed into the cached latent:

```logos
## Main
Let Q be a 1 by 6 tensor of Real.
Set item 1 of Q to 1.0.
Set item 2 of Q to 0.5.
Set item 3 of Q to 2.0.
Set item 4 of Q to -1.0.
Set item 5 of Q to 0.25.
Set item 6 of Q to 1.5.
Let W be a 4 by 2 tensor of Real.
Set item 1 of W to 1.0.
Set item 3 of W to 0.5.
Set item 6 of W to 1.0.
Set item 7 of W to 2.0.
Let C be a 1 by 3 tensor of Real.
Set item 1 of C to 1.0.
Set item 2 of C to 2.0.
Set item 3 of C to 0.5.
Let qn be the first 2 of every 3 of Q.
Let qr be the last 1 of every 3 of Q.
Let ql be qn matmul W across 2 heads.
Let qa be ql joined with qr across 2 heads.
Let V be the first 2 of every 3 of C.
Let O be attention of qa over C and V with 2 heads 1 groups scaled by 0.5.
Show round(item 1 of ql * 100).
Show round(item 3 of ql * 100).
Show round(item 5 of qa * 100).
Show round(item 3 of O * 100).
```

```output
125
50
-100
100
```

An int8 model moves between its layers in integers, and four forms say its steps exactly, every operand a
tensor of `Int`. `X times M shifted right by S` requantizes, `(x · M) >> S` with an arithmetic shift, so a
negative product rounds toward −∞; `X clamped between LO and HI` saturates; `relu of X` keeps what is not
negative. `lut attention of Q over K and V with H heads KV groups up to P with table E and scale M shifted
right by S` is one query's attention over the first `P` cached rows, its softmax read from a table of `n`
exponent weights: each head's scores `s_j = q · k_j` and their maximum `mx`, the weights
`w_j = E[((mx − s_j) · M) >> S]` with the index held to `0 … n − 1`, and the context
`(Σ w_j · v_j) // (Σ w_j)`, divided toward −∞. The sums are exact, so every tier and every accelerator
computes the same integers:

```logos
## Main
Let A be a 1 by 4 tensor of Int.
Set item 1 of A to 300.
Set item 2 of A to -300.
Set item 3 of A to 5000.
Set item 4 of A to -7.
Let R be A times 104593 shifted right by 16.
Let Q be R clamped between -128 and 127.
Let P be relu of Q.
Show item 1 of R.
Show item 2 of R.
Show item 4 of R.
Show item 3 of Q.
Show item 2 of P.
Let K be a 3 by 2 tensor of Int.
Set item 1 of K to 3.
Set item 2 of K to 1.
Set item 3 of K to -2.
Set item 5 of K to 4.
Set item 6 of K to 2.
Let V be a 3 by 2 tensor of Int.
Set item 1 of V to 10.
Set item 2 of V to -20.
Set item 3 of V to 30.
Set item 4 of V to 40.
Set item 5 of V to -50.
Set item 6 of V to 60.
Let E be a 1 by 4 tensor of Int.
Set item 1 of E to 64.
Set item 2 of E to 24.
Set item 3 of E to 9.
Set item 4 of E to 3.
Let G be a 1 by 2 tensor of Int.
Set item 1 of G to 2.
Set item 2 of G to 1.
Let at be 3.
Let C be lut attention of G over K and V with 1 heads 1 groups up to at with table E and scale 16 shifted right by 6.
Show item 1 of C.
Show item 2 of C.
```

```output
478
-479
-12
127
0
-19
20
```

## Infinitesimals, infinite numbers and supertasks

Beyond `Int`, `Rational` and `Real`, the exact number types reach further: `Algebraic` (real
algebraic numbers, ordered exactly), `ExactReal` (computable reals such as `π` and `e`, to any
precision), `Hyperreal`, `Hypernatural` and `Ordinal`.

Where a program declares one of these types — a binding, a parameter, a function's result —
`sqrt`, `exp` and `ln` are exact and `pi` (`π`) and `e` are the constants themselves; an operation,
a comparison or a call with an exact operand is exact the same way. In an `Ordinal` context `omega`
(`ω`) is the first infinite ordinal. Elsewhere they keep their meaning (`sqrt(2.0)` is a `Real`,
`pi(digits)` the digits of π), a bare `pi` (`π`) or `e` is the `Real` nearest it, and a function the
program defines, or a name it binds, is never replaced:

```logos
## Main
Let r: Algebraic be sqrt(2).
Show r.
Show r * r.
Let x: ExactReal be ln(exp(2)) + pi.
Show x.
Show x is greater than 5.
Let w: Ordinal be omega + 1.
Show w.
Show sqrt(2.0).
Show 2.0 * pi.
```

```output
√2
2
π + 2
true
ω + 1
1.4142135623730951
6.283185307179586
```

A program declares an infinitesimal, reads it with `is infinitesimal`, `is limited`, `is infinitely
close to` and `the standard part of`, and a loop whose clock steps by it runs `⌈1/dt⌉` times —
infinitely many — and still ends, in its exact state:

```logos
## Main
Let dt be an infinitesimal.
Show dt is infinitesimal.
Show 1 / dt is limited.
Show the standard part of 3 + 2 * dt.
Let t be 0.
Let lamp be 0.
While t < 1:
    Set lamp to 1 - lamp.
    Set t to t + dt.
Show lamp.
```

```output
true
false
3
0 when ⌈1/dt⌉ is even, 1 when it is odd
```

The lamp ends on or off as `⌈1/dt⌉` is even or odd, and says so. What a program states about the
reciprocal after `whose reciprocal is` — `even`, `odd`, `divisible by k`, `a multiple of k`,
`congruent to r modulo k`, each optionally after `not`, joined by `and` — fixes its residues, and a
contradiction is refused before the program runs:

```logos
## Main
Let dt be an infinitesimal whose reciprocal is divisible by 6.
Let n be 1 / dt.
Show n % 3.
Show n % 4.
```

```output
0
0 when ⌈1/dt⌉ ≡ 0 (mod 4), 2 when ⌈1/dt⌉ ≡ 2 (mod 4)
```

`At the limit, take <rule>.` resolves what depends on the residue for the supertasks after it in
its block: `the limit superior`, `the limit inferior`, `the Cesàro mean`, `the limit` (only when
every residue agrees) or `the standard part`.

```logos
## Main
Let dt be an infinitesimal.
At the limit, take the Cesàro mean.
Let t be 0.
Let lamp be 0.
While t < 1:
    Set lamp to 1 - lamp.
    Set t to t + dt.
Show lamp.
```

```output
1/2
```

These are values like any other: a function takes and returns them, a list holds them, and text
joins one as it prints.

```logos
## To halve (x: Hyperreal) -> Hyperreal:
    Return x / 2.

## Main
Let dt be an infinitesimal.
Let steps be [dt, halve(dt), 3 + dt].
Repeat for s in steps:
    Show "the standard part of " + s + " is " + the standard part of s.
```

```output
the standard part of dt is 0
the standard part of (1/2)·dt is 0
the standard part of 3 + dt is 3
```

The hyperreals take roots. `sqrt(dt)` is `√dt`, an infinitesimal larger than every multiple of
`dt`, and the square root of any positive hyperreal is exact: its square is the value again, and
two routes to one value are equal. A power to an exact exponent takes a root — `dt ** half` is
`√dt` and `2 ** half` is `√2`, `half` a `Rational` one half. A root prints as a rational function
of a root of `dt` when it is one, as a radical when a power of it is, and otherwise by its two
leading terms and the order of the rest:

```logos
## Main
Let dt be an infinitesimal.
Let r: Hyperreal be sqrt(dt).
Show r.
Show (1 + r) ** 2.
Show r > dt.
Let s: Hyperreal be sqrt(1 + dt).
Show s.
Show s * s.
Show sqrt(2 + dt) + sqrt(2 - dt).
Let half: Rational be 1 / 2.
Show 2 ** half.
```

```output
√dt
1 + 2·√dt + dt
true
√(1 + dt)
1 + dt
2√2 - (√2/16)·dt^2 + O(dt^3)
√2
```

The exponential and the logarithm of a hyperreal are known by their series in `dt`, with exact
coefficients: `exp(dt)` is `1 + dt + O(dt^2)` and `ln(1 + dt)` is `dt - (1/2)·dt^2 + O(dt^3)`. Such a
value compares by the first order at which two series differ, takes roots and rational powers, and
meets a computable real or a root of `dt` in a series of the finer root. The exponential of an
infinite value lies beyond every power of `dt`, and the logarithm of a positive infinitesimal carries
`ln(dt)`; neither is a value here, and the program stops with that reason:

```logos
## Main
Let dt be an infinitesimal.
Let x: Hyperreal be exp(dt).
Show x.
Show x > 1 + dt.
Show ln(1 + dt).
Show exp(sqrt(dt)).
Show sqrt(dt) + pi.
Let half: Rational be 1 / 2.
Show x ** half.
```

```output
1 + dt + O(dt^2)
true
dt - (1/2)·dt^2 + O(dt^3)
1 + √dt + O(dt)
π + √dt + O(dt)
1 + (1/2)·dt + O(dt^2)
```

A body that is an affine update ends in its closed form, and one whose state is finite ends where
the cycle it enters puts it. A body that is an Euler step of a linear differential equation is a
flow: it ends infinitely close to the equation's solution, and the standard part is that solution,
exactly:

```logos
## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1.
Let v: Hyperreal be 0.
Let t: Hyperreal be 0.
While t < 1:
    Let a be 0 - x.
    Set x to x + v * dt.
    Set v to v + a * dt.
    Set t to t + dt.
Show the standard part of x.
Show the standard part of v.
```

```output
cos(1)
-sin(1)
```

A flow may start from a root of `dt`, and its step may hold one past `dt` itself: its state is then a
series in that root, `e^2 + e^2·√dt + O(dt)` for `⌈1/dt⌉` steps of `x′ = (2 + √dt)·x` from 1. A step
with a term between `dt⁰` and `dt` is no flow, and is refused.

A polynomial update whose monomials close under it is an affine update of them: `x ← x + t·t·dt,
t ← t + dt` acts on `x`, `t` and `t²`, and the next `t²` holds nothing new, so `⌈1/dt⌉` steps sum
`t²·dt` to exactly `1/3 - (1/2)·dt + (1/6)·dt^2`. A polynomial Euler step whose powers grow without
end, `x ← x + x·x·dt`, is a flow too. Its solution has no closed form in general, so the standard
part is the solution itself, enclosed by validated Taylor integration: every digit printed is
certified. A solution that leaves every bound before the loop's time ends leaves a value that is not
limited.

```logos
## Main
Let dt be an infinitesimal.
Let x: Hyperreal be 1 / 2.
Let t: Hyperreal be 0.
While t < 1:
    Set x to x + x * (1 - x) * dt.
    Set t to t + dt.
Show the standard part of x.
Let y: Hyperreal be 1.
Let s: Hyperreal be 0.
While s < 2:
    Set y to y + y * y * dt.
    Set s to s + dt.
Show y is limited.
```

```output
0.731058578630004879251159241822
false
```

A body whose flow is interrupted by jumps — `If guard:` statements that set values when the flow
crosses into the guard — runs as the hybrid system it follows. Between jumps the state flows along
the equation, each jump fires where the flow crosses its guard, and the infinitely many bounces a
ball makes as it comes to rest end at the sum of their series, after which it rests. A value a jump
records is infinitely close to its value at the jump. A guard the flow only touches, two jumps at
once, a jump in the loop's last step, or a condition comparing values infinitely close to each
other depends on which infinitesimal `dt` is, and stops the program with the reason:

```logos
## Main
Let dt be an infinitesimal.
Let h: Hyperreal be 0.
Let v: Hyperreal be 10.
Let time: Hyperreal be 0.
Let last: Hyperreal be 0.
While time < 5:
    Set h to h + v * dt.
    Set v to v - 10 * dt.
    If h <= 0 and v < 0:
        Set h to 0.
        Set v to 0 - v / 2.
        If v is not infinitesimal:
            Set last to time.
    Set time to time + dt.
Show the standard part of h.
Show the standard part of last.
```

```output
0
4
```

A supertask can run in each pass of another. The part of a pass that reads nothing the passes carry
from one to the next — an inner supertask over locals the pass binds afresh — gives the same values
at every pass, so it is resolved once and the outer loop ends in its own exact state: a counter that
counts to `1/dt` in each of `1/dt` passes ends at `1/dt^2`, the stage past every stage `ω·k`. An inner
supertask that reads what the passes change — the outer clock, a value the pass computes from it — is
written as the map it is, a polynomial in what it reads, so the outer loop still ends exactly: one that
adds `t·dt` for every `dt` makes the outer loop sum `∫₀¹ t dt`, `1/2 - (1/2)·dt`. A pass whose value
depends on the residue of `⌈1/dt⌉` is followed on each class:

```logos
## Main
Let dt be an infinitesimal.
Let t be 0.
Let total be 0.
Let lit be 0.
While t < 1:
    Let s be 0.
    Let n be 0.
    Let lamp be 0.
    While s < 1:
        Set n to n + 1.
        Set lamp to 1 - lamp.
        Set s to s + dt.
    Set total to total + n.
    Set lit to lit + lamp.
    Set t to t + dt.
Show total.
Show lit.
```

```output
1/dt^2
0 when ⌈1/dt⌉ is even, 1/dt when it is odd
```

A loop that counts toward a value that can be infinite runs as many times as it must and ends. Its
guard — `and`, `or` and `not` over comparisons of terms linear in the counter — is read where its
truth can change, on each residue class of `⌈1/dt⌉` that point depends on. This is the floor function
on the hyperreals, as Imamura writes it:

```logos
## To floorOf (x: Hyperreal) -> Hyperreal:
    Let n be 0.
    While not ((n <= x and x < n + 1) or (0 - n <= x and x < 0 - n + 1)):
        Set n to n + 1.
    If x >= 0:
        Return n.
    Return 0 - n.

## Main
Let dt be an infinitesimal.
Show floorOf(7 / 2).
Show floorOf(3 - dt).
Show floorOf(1 / dt + 1 / 2).
Show floorOf(1 / (2 * dt)).
```

```output
3
2
1/dt
1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) - 1/2 when it is odd
```

A program may declare several infinitesimals, ordered by how they compare. `Let dx be an infinitesimal
much smaller than dt.` makes `dx` smaller than every positive value made from `dt` — `dt^1000` among
them — so the values of both are exact and every comparison between them is decided; `much larger than`
declares one above, and `much smaller than dt and much larger than dy` one between two already ordered.
A bare `smaller than` decides too little — that `dx < dt`, but not whether `dx < dt^2` — and is refused
before the program runs, as is a value that combines two infinitesimals whose order the program never
states. A question may take a standpoint: `relative to dt`, every value made from `dt` is an ordinary
number, so the standard part of `dt + dx` relative to `dt` is `dt`, the inner limit with `dt` held fixed:

```logos
## Main
Let dt be an infinitesimal.
Let dx be an infinitesimal much smaller than dt.
Let h be an infinitesimal much larger than dt.
Let dy be an infinitesimal much smaller than dt and much larger than dx.
Show dx < dt ** 1000.
Show dy < dt ** 1000 and dx < dy ** 1000.
Show h + dt + dy + dx.
Show dx / dt.
Show (dt + dx) ** 2.
Show sqrt(dt + dx) * sqrt(dt + dx).
Show exp(dt + dx).
Show the standard part of dt + dx relative to dt.
Show dx is infinitesimal relative to dt.
Show 1 / dt is limited relative to dt.
```

```output
true
true
h + dt + dy + dx
dx/dt
dt^2 + 2·dt·dx + dx^2
dt + dx
(1 + dt + O(dt^2)) + (1 + dt + O(dt^2))·dx + O(dx^2)
dt
true
true
```

A loop stepped by `dx` is a limit inside `dt`'s, and its rates may be values of `dt`: `⌈1/dx⌉` steps of
`x ← x + x·dt·dx` end at `e^dt`, read relative to `dt` by its series. A supertask in each pass of another
is an iterated limit, written as the map it is — over the same infinitesimal or a smaller one, nested to
any depth, several in one pass — so the outer loop ends in its exact state:

```logos
## Main
Let dt be an infinitesimal.
Let dx be an infinitesimal much smaller than dt.
Let x: Hyperreal be 1.
Let s: Hyperreal be 0.
While s < 1:
    Set x to x + x * dt * dx.
    Set s to s + dx.
Show the standard part of x relative to dt.
Let total: Hyperreal be 0.
Let t: Hyperreal be 0.
While t < 1:
    Let y: Hyperreal be 0.
    Let r: Hyperreal be 0.
    While r < 1:
        Set y to y + t * t * dx.
        Set r to r + dx.
    Set total to total + y * dt.
    Set t to t + dt.
Show total.
Show the standard part of total.
```

```output
1 + dt + O(dt^2)
1/3 - (1/2)·dt + (1/6)·dt^2
1/3
```

A value may depend on the residues of several reciprocals: it depends on the coarsest outside and the
finer inside, and every limit rule reads them all. The integer part of a value infinite in several is an
infinite integer of several on the classes its fraction needs, printed as one of one infinitesimal is;
one whose fraction no residue decides, `⌊dt/dx⌋`, is refused by name. A supertask whose count is
infinite in several — `⌈1/(dt·dx)⌉` steps of `dt·dx` — ends as exactly:

```logos
## Main
Let dt be an infinitesimal.
Let dx be an infinitesimal much smaller than dt.
Show floor(1 / dt) % 2 + floor(1 / dx) % 2.
Let n be floor(1 / dt + 1 / dx).
Show n.
Show n % 2.
Show floor(1 / (2 * dt) + 1 / dx).
Let t: Hyperreal be 0.
Let x: Hyperreal be 1.
While t < 1:
    Set x to x + x * dt * dx.
    Set t to t + dt * dx.
Show the standard part of x.
```

```output
(0 when ⌈1/dx⌉ is even, 1 when it is odd) when ⌈1/dt⌉ is even, (1 when ⌈1/dx⌉ is even, 2 when it is odd) when it is odd
1/dx + 1/dt
(0 when ⌈1/dx⌉ is even, 1 when it is odd) when ⌈1/dt⌉ is even, (1 when ⌈1/dx⌉ is even, 0 when it is odd) when it is odd
1/dx + 1/(2·dt) when ⌈1/dt⌉ is even, 1/dx + 1/(2·dt) - 1/2 when it is odd
e
```

A real number can also be what a function of the program says it is. `the real number whose digits
in base k are f` is the number whose digit in place `i` is `f(i)`, worth `k^-i` (place `0` holds the
units), and `the real number approximated by f` is the number `f(n)` lies within `1/2^n` of. Either is
an `ExactReal` that calls `f` only when a comparison, a floor or a printed digit asks for more of it.
The function takes one `Int` and answers an `Int` (a digit) or a `Rational` (an approximation); it may
read nothing but its argument and do nothing but answer, and a function that does more is refused
before the program runs. A digit outside its base, or approximations no one number satisfies, stop
the program and name where. The number stays with the program that defines it: a parallel block reads
it, but a task may run where the function is not, so handing it to a task or sending it through a pipe
stops the program. One real holds a whole set of naturals in its digits, and a loop reads a member
back, as in Imamura's Proposition 3:

```logos
## To isPrime (n: Int) -> Bool:
    If n < 2:
        Return false.
    Let d be 2.
    While d * d <= n:
        If n % d = 0:
            Return false.
        Set d to d + 1.
    Return true.

## To primeDigit (i: Int) -> Int:
    If isPrime(i):
        Return 1.
    Return 0.

## To rootTwoWithin (n: Int) -> Rational:
    Let lo: Rational be 1.
    Let hi: Rational be 2.
    Let k be 0.
    While k < n + 1:
        Let mid: Rational be (lo + hi) / 2.
        If mid * mid <= 2:
            Set lo to mid.
        Otherwise:
            Set hi to mid.
        Set k to k + 1.
    Return lo.

## Main
Let primes be the real number whose digits in base 3 are primeDigit.
Show floor(primes * 3 ** 8).
Let r be the real number approximated by rootTwoWithin.
Show r.
Show r * r.
```

```output
1002
1.41421356237309504880168872421
2.00000000000000000000000000000
```

`Repeat for every natural n:` searches every natural. A condition built from linear terms in `n`
and their remainders is decided exactly; any other is run on the first ten million naturals, or the
number `checking up to K cases` names, and a search that finds no witness answers with its
evidence:

```logos
## Main
Let found be false.
Repeat for every natural n:
    If n * 3 = 12:
        Set found to true.
Show found.
Let square be false.
Repeat for every natural n, checking up to 500 cases:
    If n * n = 723:
        Set square to true.
Show square.
```

```output
true
false (provisional: checked 500 cases, 0 revisions; not proved)
```

A supertask the engine has no exact answer for stops with the reason instead of running forever.

## Modules

A program may span files. A file names the modules it uses with `Import` lines anywhere in the prose
above its first `##` header; an `Import` written below a header imports nothing and is refused,
naming the line to move it above.

```text
Import Geometry.                             the whole module
Import Geometry and Colors.                  several (also: Import Lexer, Parser, Ast.)
Import Geometry from "shapes/geometry.lg".   an explicit path, relative to this file
Import Util as U.                            an alias: U::double(21)
Import double from Util.                     one name (also: Import double, triple from Util.)
Import double as dbl from Util.              one name, renamed
```

A bare name is resolved in this order:

1. a lower-case file beside the importing file (`geometry.lg`, then `geometry.md`);
2. a directory module (`lexer/lexer.lg`, then `lexer/mod.lg`), where a file beats a directory of
   the same name;
3. the project's `src/` and the directory holding `Largo.toml`;
4. a dependency the manifest declares (a `path` dependency, or a `git` one pinned to an exact
   40-hex `rev`).

A mistyped name gets a "did you mean". A module that resolves outside the project and
the declared dependency roots is refused before it is read. A Markdown link in a file's opening
paragraph, `[Geometry](file:./geo.md)`, also imports.

- **Everything is public unless marked.** `## To private helper (…)` and `## A private Secret has:`
  are usable only inside their own module; a bare or qualified (`Util::helper`) reference from another
  file is refused, and two modules may each keep a private helper of the same name.
- **Each file sees what it imports.** A name reached only through another module's import is refused
  where it is used, naming the module to import.
- **Qualified names.** `Alias::name` calls a function and `Alias::Type` names a type
  (`Geometry::Point`); the bare and qualified spellings of a type are one type.
- **One program.** Imports are followed transitively and may be mutual; only the entry file's
  `## Main` runs. Two modules that define the same public name with different bodies are refused,
  naming both files; identical definitions merge.
- **Tooling.** `largo check` warns about an unused, duplicated or self import; `largo fmt` sorts and
  de-duplicates the import block.

The assembled program is one compilation unit, so an imported function reaches the parser, the call
graph and every tier exactly as a function written in the entry file does.

## Standard library

A standard library ships with the compiler
([`assets/std/`](../crates/logicaffeine_compile/assets/std/)), grouped by what it is for:

| Area | Modules |
|------|---------|
| Core types | `core` (`Result`, `Option`) |
| Text and data | `strings`, `chars`, `unicode`, `bytes`, `bits`, `regex`, `encoding`, `convert`, `json` |
| Numbers | `math`, `statistics`, `random`, `rng`, `operator` |
| Collections | `collections`, `iter`, `sort`, `heap`, `builder` |
| Linear algebra | `blas`, `lapack`, `clapack` |
| The machine | `io`, `file`, `env`, `time`, `calendar`, `error`, `compress`, `media`, `vision` |
| The web | `http`, `static`, `session`, `web`, `forms`, `ui`, `postgres`, `vdom` |
| The network | `tcp`, `net` |
| Distribution | `crdt`, `concurrency` |
| Identity | `crypto`, `uuid` |

The web modules are the subject of two guides: [Web](web.md) for the server and
[Frontend](frontend.md) for pages in the browser. `lapack` also backs two statements:
`Solve M times x equals v for x.` binds `x` to `solveLinear(M, v)`, and
`Minimize the quadratic Q with linear b over x.` binds it to `minimizeQuadratic(Q, b)`.

The `crypto` and `uuid` modules are themselves written in LOGOS.
[`crypto.lg`](../crates/logicaffeine_compile/assets/std/crypto.lg) carries:

- ChaCha20 with a Poly1305 AEAD, SHA-256, HMAC and PBKDF2;
- the Keccak-f permutation behind SHA3-256, SHA3-512, SHAKE128 and SHAKE256 (with a four-lane form);
- the post-quantum ML-KEM-768 and ML-DSA building blocks (NTTs with SIMD lanes, Montgomery/Barrett
  reduction);
- Ding's ring-LWE key exchange and Rainbow signatures over GF(2⁸), for study: Rainbow is broken (Beullens,
  2022), and Ding's exchange leaks a reused secret, so keys are agreed with ML-KEM-768 and messages
  signed with ML-DSA-65.

Not all of the crypto layer is LOGOS source. The `sha1` and `md5` hash functions are native tools.
The `## To native` declarations in `crypto.lg` (the ML-KEM arithmetic, sampling and serialization
kernels, and the ML-KEM/ML-DSA key generation, encapsulation, decapsulation, signing and
verification entry points) run one shared kernel on the tree-walker, the VM and AOT-Rust. A kernel takes
its arguments at their word: the items of a byte string must be bytes and a seed must be 32 of them,
so `mldsaSign` under a 256-byte context stops with `mldsaSign(): the context is 256 bytes; FIPS 204
allows at most 255` on every tier. An input from the other side of a protocol is answered instead:
`mlkemEncapsKem` returns an empty sequence for an encapsulation key that fails FIPS 203's checks,
`mlkemDecapsKem` returns one for a ciphertext of the wrong length, and `mldsaVerify` answers 0 for
anything malformed. The ML-DSA written in LOGOS takes a FIPS 204 context through `mldsaSignCtxLogos`
and `mldsaVerifyCtxLogos`, and `pqAuthHandshake` signs both of its transcripts under the context it
is given. The native kernels keep their running time apart from the secrets they hold: no division
by the modulus and no branch on a secret coefficient. The LOGOS source is the reference the tiers
are checked against, and an interpreter's arithmetic takes time that depends on its operands, so a
program that holds a real secret calls the native kernels.

Symmetric sealing is native too. `sealBox(key, bytes)` seals with XChaCha20-Poly1305 under a 24-byte
nonce drawn from the system's entropy for that seal and carried in front of the box, so one 32-byte
key seals any number of boxes; `openBox(key, box)` verifies and opens one, and `sealValueBox` and
`openValueBox` do the same for any value. `sealBytes(key, nonce, bytes)` and `sealFrame(key, nonce,
value)` take the caller's 12-byte ChaCha20-Poly1305 nonce, which must never repeat under a key: a
repeated nonce gives away both plaintexts and lets a forger sign. None of them compresses first, so
a sealed message's length tells nothing of it but its length.
[`uuid.lg`](../crates/logicaffeine_compile/assets/std/uuid.lg) implements RFC 9562 UUIDs with the
MD5 and SHA-1 digests as LOGOS source. Both modules are checked bit-exact against FIPS and
reference-crate oracles across the execution tiers.

Every module in the table imports itself by name.
[`apply_prelude`](../crates/logicaffeine_compile/src/loader.rs) splices a module into a program that
uses one of its names: a call, a type in an annotation, a `When` arm on one of its variants. It
also splices one into a program whose constructs the compiler lowers into calls to it (a `## Route`
with a `:param` segment reaches `web`, a page reaches `vdom`, `Solve … for x.` reaches `lapack`).
Selection is transitive: a module brings the modules its own functions call (`statistics` brings
`math`). Modules are spliced after the program's leading prose, before its first `##` section.

"Declarer wins", per name. A program that defines its own `trim` calls its own at every site it
writes, still receives the rest of `strings`, and the library's copy is renamed so the library keeps
calling its own. A local value named like a library function neither keeps the module out nor
replaces the function. Only a program that declares one of a module's native or builtin-backed names
keeps that whole module out.

```logos
## Main
Let s be "hello big world".
Let words be split(s, " ").
Show length of words.
Show item 2 of words.
```

```output
3
big
```

A program that uses no library name compiles byte-for-byte unchanged, the compiled tiers drop the
library functions a program never reaches, and the interpreters and every compiler go through the
same seam. Opt out entirely with a `## NoPrelude` marker.

## Concurrency

Structured concurrency, channels, agents, and CRDTs are part of the language too: tasks and their
`Await`, pipes and `Close`, CRDT `Merge` and `Converge`. See
[Concurrency & distributed](concurrency.md).

## See also

- How it runs and how fast → [Execution & performance](execution-and-performance.md)
- The `largo test`, `check` and `fmt` commands → [The largo CLI](cli.md)
- The compiler crate → [`logicaffeine_compile`](../crates/logicaffeine_compile/README.md)

## Where this is tested

The grammar and AST are in
[`crates/logicaffeine_language/src/ast/stmt.rs`](../crates/logicaffeine_language/src/ast/stmt.rs),
the code generators in
[`crates/logicaffeine_compile/src/codegen/`](../crates/logicaffeine_compile/src/codegen/), and the
tests in [`crates/logicaffeine_tests/tests/`](../crates/logicaffeine_tests/tests/). Every `logos`
block in this guide is compiled by `doc_examples`, which also runs the `## Test` blocks it holds
and checks any output shown against the tree-walker. Each construct is locked by its own suites:

| Section | Suites |
|---------|--------|
| Variables & mutation | `identifier_freedom`, `float_real_unify`, `increase_decrease_plain_lock`, `op_backend_matrix_lock`, `tier_parity_lock` |
| Primitive types & operators | `e2e_primitives`, `e2e_expressions`, `e2e_comparisons`, `phase_bitwise`, `correctness_exact_arith`, `word_narrow_all_tiers`, `complexf_arith`, `interpolation_hole_total_lock`, `e2e_codegen_uuid`, `e2e_md5_logos` |
| Functions | `e2e_functions`, `phase32_functions`, `e2e_closures`, `phase34_generics`, `phase_ffi_requires` |
| Collections | `e2e_collections`, `e2e_sets`, `e2e_maps`, `e2e_tuples`, `list_where_lock`, `count_of_collection_lock` |
| Tuples & multiple return | `param_return_tier_coverage_lock` |
| Control flow | `e2e_control_flow`, `e2e_iteration`, `phase30_iteration` |
| Structs | `e2e_structs`, `phase31_structs` |
| Enums & pattern matching | `e2e_enums`, `phase33_enums`, `variant_resolution_lock`, `option_tier_lock`, `option_of_record_lock` |
| Ownership & borrowing | `phase21_ownership` |
| I/O & assertions | `phase10_io`, `phase25_assertions` |
| Contracts & failure | `error_fail_and_contracts`, `failure_all_tiers` |
| Testing | `test_runner_lock`, `test_surface_catalog`, `property_test_lock`, `hermeticity_redteam`, `failure_message_goldens` |
| Observation | `obs_grammar_catalog`, `obs_channel_separation_lock`, `obs_vm_emission_lock`, `obs_aot_emission_lock`, `obs_wasm_lock`, `obs_zero_cost_codegen_lock` |
| Tensors | `tensor_sugar_tier_lock`, `tensor_op_coverage_lock`, `tensor_sugar_all_backends_lock`, `int_tensor_sugar_lock` |
| Infinitesimals, infinite numbers and supertasks | `nonstandard_surface`, `supertask_strategies`, `dt_flows`, `dt_nonlinear_flows`, `supertask_hybrid`, `supertask_imamura_paper`, `tier_deadline_lock` |
| Modules | `multimodule_functions`, `multimodule_types`, `multimodule_selective`, `multimodule_encapsulation`, `multimodule_directory`, `multimodule_audit`, `project_module_lock`, `local_deps_lock`, `import_scan_lock`, `imported_record_fetch_lock`, `import_hygiene` |
| Standard library | `analog_relax_surface_lock` (the `lapack` statements); `mlkem768_logos`, `mlkem768_oracle`, `crypto_sha2_lock`, `sha3_native`, `mldsa_module`, `ding_ke_logos`, `rainbow_logos`, `native_kernel_lock`, `e2e_codegen_uuid`, `e2e_uuid_logos_gen` (the crypto and UUID oracles); `transitive_prelude_lock`, `stdlib_user_name_hygiene`, `user_name_shadows_stdlib_lock`, `rustc_check_prune_lock`, `stdlib_no_name_collisions` (the prelude) |

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
