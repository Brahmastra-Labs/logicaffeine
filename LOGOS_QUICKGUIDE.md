# LOGOS Quick Guide

How to write and work with every construct in LOGOS, in one place. Each table
shows the **canonical** form, the **ambiguous alternatives that also parse** (LOGOS
supports many surfaces for one meaning), and what it **lowers to** internally.

Forms marked **(proposed)** are designed in `LANGUAGE_SMELLS.md` but not yet
implemented — they are listed here so this guide doubles as the target spec.
Everything not so marked works in-tree today.

---

## 1. Program structure

LOGOS source is literate Markdown; `##` headers introduce definitions.

| Construct | Form | Notes |
|-----------|------|-------|
| Entry point | `## Main` | the program body |
| Function | `## To f (n: Int) -> Int:` | return type optional |
| Procedure | `## To greet (name: Text):` | no return |
| Native import | `## To native parseInt (s: Text) -> Int` | FFI declaration |
| Struct | `## A Point has:` | fields indented below |
| Enum | `## A Shape is one of:` | variants indented below |
| Theorem | `## Theorem: Socrates` | with `Given/Prove/Proof: Auto.` |

---

## 2. Variables & mutation

| Meaning | Canonical | Also works | Lowers to |
|---------|-----------|------------|-----------|
| Bind | `Let x be 5.` | `Let x: Int be 5.` | `Stmt::Let` |
| Mutable bind | `Let mutable x be 5.` | `Let mut x be 5.` | `Stmt::Let{mutable}` |
| Reassign | `Set x to 10.` | — | `Stmt::Set` |
| Increment | `Set i to i + 1.` | **(proposed)** `Increase i by 1.` · `i += 1.` | `Stmt::Set` |
| Decrement | `Set i to i - 1.` | **(proposed)** `Decrease i by 1.` · `i -= 1.` | `Stmt::Set` |

---

## 3. Arithmetic, comparison, logic, bitwise

| Meaning | Symbolic | English | Lowers to |
|---------|----------|---------|-----------|
| Add / sub | `x + y` · `x - y` | — | `BinaryOp Add/Subtract` |
| Mul / div / mod | `x * y` · `x / y` · `x % y` | — | `Multiply/Divide/Modulo` |
| Concatenate (Text) | **(proposed)** `"a" + "b"` | `"a" combined with "b"` | `BinaryOp Concat` |
| Equal | `x == y` | `x equals y` · `x is equal to y` | `Eq` |
| Not equal | `x != y` | `x is not y` | `NotEq` |
| Greater / less | `x > y` · `x < y` | `x is greater than y` · `x is less than y` | `Gt/Lt` |
| ≥ / ≤ | `x >= y` · `x <= y` | `x is at least y` · `x is at most y` | `GtEq/LtEq` |
| Logical | — | `a and b` · `a or b` · `not a` | `And/Or/Not` (type-aware) |
| Bitwise xor | — | `x xor y` | `BitXor` |
| Shift | — | `x shifted left by n` · `x shifted right by n` | `Shl/Shr` |
| Negate | `-x` | — | `0 - x` |
| Popcount | — | `count_ones(x)` | builtin |

Note: `and`/`or` are logical for `Bool` and bitwise for `Int` (resolved in codegen).

---

## 4. Strings

| Meaning | Canonical | Also works | Lowers to |
|---------|-----------|------------|-----------|
| Concatenate | `a combined with b` | **(proposed)** `a + b` | `Concat` |
| Interpolate | `"Hello, {name}!"` | — | `InterpolatedString` |
| Format precision | `"{pi:.2}"` | — | `StringPart{format_spec}` |
| Align | `"{s:>10}"` · `"{s:<10}"` · `"{s:^10}"` | — | format spec |
| Debug | `"{v=}"` | — | format spec `debug` |
| Currency | `"{price:$}"` | — | format spec |
| Multiline | `"""…"""` | — | literal |
| split/join/trim/case/replace | **(proposed)** `s.split(",")` · `join(parts, ", ")` | — | new builtins (phantom today) |

---

## 5. Collections

### 5.1 Create

| Type | Canonical | Also works | Lowers to |
|------|-----------|------------|-----------|
| List, empty | `a new Seq of Int` | `a new List of Int` · `a new Vec of Int` · `[] of Int` · **(proposed)** `a new Array of Int` | `Expr::New{Seq}` |
| List, literal | `[1, 2, 3]` | — | `Expr::List` |
| List, pre-sized | `a new Seq of Int with capacity n` | — | `Expr::WithCapacity` |
| Map, empty | `a new Map of Text to Int` | `a new HashMap of Text to Int` · **(proposed)** `a new Dictionary of …` | `Expr::New{Map}` |
| Map, with capacity | `a new Map of Int to Int with capacity n` | — | `WithCapacity` |
| Map, literal | **(proposed)** `{ "a": 1, "b": 2 }` · empty `{} of Text to Int` | — | `New{Map}` + inserts |
| Set, empty | `a new Set of Int` | `a new HashSet of Int` | `Expr::New{Set}` |
| Set, literal | **(proposed)** `{1, 2, 3}` (no `:` ⇒ set) | — | `New{Set}` + adds |

### 5.2 Read, write, slice

| Meaning | Canonical | Also works | Lowers to |
|---------|-----------|------------|-----------|
| Index read (1-based) | `item i of xs` | `xs[i]` · **(proposed)** `xs at i` | `Expr::Index` |
| Map lookup | `item k of m` | `m[k]` | `Expr::Index` |
| Index write | `Set item i of xs to v` | `Set xs[i] to v` | `Stmt::SetIndex` |
| Map insert | `Set m at k to v` | `Set item k of m to v` · `Set m[k] to v` | `SetIndex` |
| Slice (inclusive) | `items a through b of xs` | `items a through b` (collection inferred) · **(proposed)** `xs[a:b]` | `Expr::Slice` |
| Length | `length of xs` | `length(xs)` · **(proposed)** `xs.length` | `Expr::Length` |
| Membership | `xs contains v` | `v in xs` | `Expr::Contains` |
| Copy | `copy of xs` | `copy(xs)` | `Expr::Copy` |

> ⚠️ Indexing is **1-based**: `item 1 of xs` and `xs[1]` are the *first* element.
> `item 0 of xs` is a compile error; `xs[0]` currently underflows
> (see `LANGUAGE_SMELLS.md` §I-1). A project may opt into 0-based indexing
> **(proposed)** via `logos.toml`.

### 5.3 Mutate (lists & sets)

| Meaning | Form | Lowers to |
|---------|------|-----------|
| Append | `Push v to xs.` | `Stmt::Push` |
| Pop | `Pop from xs.` · `Pop from xs into y.` | `Stmt::Pop` |
| Set add | `Add v to s.` | `Stmt::Add` |
| Set remove | `Remove v from s.` | `Stmt::Remove` |
| Union / intersection | `a union b` · `a intersection b` | `Union`/`Intersection` |

### 5.4 Iterate & transform

| Meaning | Canonical | Also works | Lowers to |
|---------|-----------|------------|-----------|
| For-each | `Repeat for x in xs:` | `for x in xs:` · `Repeat x in xs:` | `Stmt::Repeat` |
| Counted | `for i from 1 to n:` | `Repeat for i from 1 to n:` | `Repeat` over `Range` |
| Pairs (map) | `Repeat for (k, v) in m:` | — | `Repeat` + tuple `Pattern` |
| Map / filter | **(proposed)** `[f(x) for x in xs if p(x)]` · `xs.map(f)` · `each x in xs mapped to f(x)` | — | desugar to `Repeat`+`Push` |
| Reduce / sum | **(proposed)** `the sum of xs` · `xs.reduce(...)` | — | new builtins |
| Sort | **(proposed)** `xs sorted` · `xs sorted by (x) -> x's age` · `xs.sort()` | — | new builtins |
| any / all / count | **(proposed)** `every x in xs satisfies p` · `xs.any(p)` | — | new builtins |

---

## 6. Control flow

| Construct | Form | Notes |
|-----------|------|-------|
| If / else | `If c:` … `Otherwise:` … | `Stmt::If` |
| Else-if | `elif c:` · `Else If c:` · `Otherwise If c:` | all valid; **canonical: `elif`** |
| While | `While c:` | `Stmt::While` |
| While + variant | `While c (decreasing e):` | termination proof |
| Break | `Break.` | innermost loop |
| Return | `Return x.` · `Return.` | `Stmt::Return` |
| Conditional value | **(proposed)** `a if c else b` | `Expr::IfExpr` |
| Match | `Inspect t:` `When V (a, b):` … `Otherwise:` … | `Stmt::Inspect` |

---

## 7. Functions & closures

| Construct | Form | Notes |
|-----------|------|-------|
| Define | `## To add (a: Int, b: Int) -> Int:` | comma params work today |
| Define (prose) | `## To add (a: Int) and (b: Int) -> Int:` | `and` between groups optional |
| Define (prepositional) | `## To withdraw (amount: Int) from (balance: Int):` | prepositions allowed |
| Call | `add(3, 7)` | `Expr::Call` |
| Call (statement) | `Call process with data.` · `Call f with a and b.` | `Stmt::Call` |
| Closure (expr) | `(x: Int) -> x + 1` | `Expr::Closure` |
| Closure (block) | `(x: Int) ->:` then indented body with `Return` | `Closure{Block}` |
| HOF parameter | `## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:` | function-typed param |
| Return a closure | `Return (x: Int) -> x + n.` | closures are first-class |
| Call a closure value | `f(42)` | `Expr::CallExpr` |

---

## 8. Structs, enums & field access

| Construct | Form | Also works | Lowers to |
|-----------|------|------------|-----------|
| Struct def | `## A Point has:` then `An x: Int.` `A y: Int.` | — | `StructDef` |
| Construct | `a new Point` | `a new Point with x 10 and y 20` | `Expr::New` |
| Field read | `p's x` | **(proposed)** `p.x` | `FieldAccess` |
| Nested field | `b's location's x` | **(proposed)** `b.location.x` | chained `FieldAccess` |
| Field write | `Set p's x to 5.` | **(proposed)** `p.x = 5.` · `Set p.x to 5.` | `SetField` |
| Enum def | `## A Shape is one of:` then `A Circle with radius Int.` | — | enum `StructDef` |
| Variant construct | `a new Circle with radius 10` | — | `NewVariant` |
| Match variant | `Inspect s:` `When Circle (r):` … | — | `Inspect` |

---

## 9. Options & pattern matching

| Meaning | Form | Lowers to |
|---------|------|-----------|
| Some | `some 30` | `OptionSome` |
| None | `none` | `OptionNone` |
| Match | `Inspect maybe:` `When OptionSome (v):` … `When OptionNone:` … | `Inspect` |
| Optional chaining | **(proposed)** `p?.address?.city` | future |

---

## 10. Contracts: refinement, assert, trust, check

| Meaning | Form | Notes |
|---------|------|-------|
| Refinement type | `Let x: Int where x > 0 be 5.` | constraint binds the declared name |
| Compound refinement | `Let x: Int where x > 0 and x < 100 be 50.` | — |
| Assert (debug) | `Assert that x is equal to 42.` | `RuntimeAssert` |
| Trust (justified) | `Trust that x is greater than 0 because "set to 10".` | carries a reason |
| Check (mandatory) | `Check that balance is at least amount.` | security gate |

---

## 11. Temporal literals

| Kind | Examples |
|------|----------|
| Duration | `50ns` · `100us` · `500ms` · `2s` · `5min` · `1h` |
| Date | `2026-05-20` |
| Time of day | `9am` · `4pm` · `noon` · `9:30am` |
| Calendar span | `2 weeks` · `3 months` · `1 year and 2 months and 5 days` |
| Combined | `2026-05-20 at 4pm` |

---

## 12. Distributed: CRDT, concurrency, networking, zones

| Construct | Form | Notes |
|-----------|------|-------|
| Shared struct | `A Counter is Shared and has:` then `points: ConvergentCount.` | CRDT field types |
| CRDT increment | `Increase c's points by 10.` | `IncreaseCrdt` (CRDT fields only today) |
| CRDT decrement | `Decrease g's score by 30.` | `DecreaseCrdt` |
| CRDT merge | `Merge remote into local.` | conflict-free |
| Shared set | `Add "Alice" to p's guests.` | ORSet semantics |
| Shared sequence | `Append "Line 1" to doc's lines.` | RGA semantics |
| Spawn agent | `Spawn an EchoAgent called "echo".` | actor |
| Zone (arena) | `Inside a zone called "Scratch":` | scoped allocation |
| Listen | `Listen on "/ip4/0.0.0.0/tcp/8000".` | raw multiaddr (leaky — see smells §I-5) |

---

## 13. Output

| Meaning | Form |
|---------|------|
| Print | `Show x.` · `Show "Hello, World!".` |
| Print formatted | `Show "{result:.15}".` |
| Move into a sink | `Give x to consume.` (ownership move) · `Show x to display.` (borrow) |

---

### Legend

- **(proposed)** — designed in `LANGUAGE_SMELLS.md`, not yet implemented.
- "Lowers to" names the AST node in `crates/logicaffeine_language/src/ast/stmt.rs`.
- Everything unmarked parses in-tree today.
