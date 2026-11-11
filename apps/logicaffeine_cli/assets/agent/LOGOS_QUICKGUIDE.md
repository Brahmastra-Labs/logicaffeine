# LOGOS Quick Guide

How to write and work with every construct in LOGOS, in one place. Each table
shows the **canonical** form, the **ambiguous alternatives that also parse** (LOGOS
supports many surfaces for one meaning), and what it **lowers to** internally.

Forms marked **(proposed)** are designed but not yet implemented — they are
listed here so this guide doubles as the target spec. Everything not so
marked works in-tree today.

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
| Increment | `Set i to i + 1.` | `Increase i by 1.` · `i += 1.` | `Stmt::Set` |
| Decrement | `Set i to i - 1.` | `Decrease i by 1.` · `i -= 1.` | `Stmt::Set` |

---

## 3. Arithmetic, comparison, logic, bitwise

| Meaning | Symbolic | English | Lowers to |
|---------|----------|---------|-----------|
| Add / sub | `x + y` · `x - y` | — | `BinaryOp Add/Subtract` |
| Mul / div / mod | `x * y` · `x / y` · `x % y` | — | `Multiply/Divide/Modulo` |
| Concatenate (Text) | `"a" + "b"` | `"a" combined with "b"` | `BinaryOp Concat` |
| Equal | `x == y` | `x equals y` · `x is equal to y` | `Eq` |
| Not equal | `x != y` | `x is not y` | `NotEq` |
| Greater / less | `x > y` · `x < y` | `x is greater than y` · `x is less than y` | `Gt/Lt` |
| ≥ / ≤ | `x >= y` · `x <= y` | `x is at least y` · `x is at most y` | `GtEq/LtEq` |
| Chained | `lo <= x <= hi` | — | `(lo <= x) and (x <= hi)` |
| Range test | — | `x is between lo and hi` | inclusive `lo <= x and x <= hi` |
| Parity / divisibility | — | `x is even` · `x is odd` · `x is divisible by n` | `x % 2 == 0` · `== 1` · `x % n == 0` |
| Logical | — | `a and b` · `a or b` · `not a` | `And/Or/Not` (truthiness → Bool, short-circuit) |
| Bitwise and/or | `x & y` · `x \| y` | — | `BitAnd/BitOr` (Set: intersection/union) |
| Bitwise xor | `x ^ y` | `x xor y` | `BitXor` (Set: symmetric difference) |
| Bitwise not | `~x` | — | lowers to `x ^ -1` |
| Shift | — | `x shifted left by n` · `x shifted right by n` | `Shl/Shr` |
| Negate | `-x` | — | `0 - x` |
| Popcount | — | `count_ones(x)` | builtin |
| Infinitesimal | — | `x is infinitesimal` · `x is limited` · `x is infinitely close to y` (each with `not`, each optionally `relative to dt`) | `isInfinitesimal`/`isLimited`/`infinitelyClose`; `relative to dt` asks from `dt`'s standpoint, where every value of `dt` is an ordinary number |
| Standard part | — | `the standard part of x` | `standardPart` over the whole arithmetic after it |
| Exact functions | `sqrt(x)` · `exp(x)` · `ln(x)` · `pi` · `π` · `e` · `x ** q` | — | in an exact context (a declared `Rational`/`Algebraic`/`ExactReal`/`Hyperreal` binding, parameter or result, or an exact operand): `exactSqrt`/`exactExp`/`exactLn`/`exactPi`/`exactE`; elsewhere `Real`. `sqrt` of a hyperreal is its root (`sqrt(dt)` is `√dt`); `exp` and `ln` of a hyperreal are its series in `dt` (`exp(dt)` is `1 + dt + O(dt^2)`), the exponential of an infinite value and the logarithm of an infinitesimal refused; `x ** q` to an exact exponent is `exactPower`, a root for a fraction (`2 ** half` is `√2`) |
| First infinite ordinal | `omega` · `ω` | — | in an `Ordinal` context: `exactOmega` (`1 + ω` is `ω`) |
| Defined real | — | `the real number whose digits in base k are f` · `the real number approximated by f` | `definedReal`: an `ExactReal` reading `f(i)`, the digit worth `k^-i`, or `f(n)`, within `1/2^n`, only as a comparison, floor or printed digit asks; `f` takes one `Int`, reads only it and only answers |

Note: `and`/`or`/`not` are LOGICAL — truthiness in, `Bool` out, short-circuit. Falsy:
`false`, `0`, `0.0`, `nothing`, empty Text/Seq/Map/Set (so `If items:` tests emptiness).
The bitwise spellings are the symbols `&`/`|`/`^`/`~`; on Sets, `&`/`|`/`^`/`-` are
intersection/union/symmetric-difference/difference (`a without b` is the English difference).

---

## 4. Strings

| Meaning | Canonical | Also works | Lowers to |
|---------|-----------|------------|-----------|
| Concatenate | `a combined with b` | `a + b` | `Concat` |
| Interpolate | `"Hello, {name}!"` | — | `InterpolatedString` |
| Format precision | `"{pi:.2}"` | — | `StringPart{format_spec}` |
| Align | `"{s:>10}"` · `"{s:<10}"` · `"{s:^10}"` | — | format spec |
| Debug | `"{v=}"` | — | format spec `debug` |
| Currency | `"{price:$}"` | — | format spec |
| Multiline | `"""…"""` | — | literal |
| split/join/trim/case/replace | `split(s, ",")` · `join(parts, ", ")` · `trim(s)` · `upper(s)` · `replace(s, "a", "b")` | `s.split(",")` (UFCS) | `strings.lg`, imported on use |

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
| Map, literal | `{ "a": 1, "b": 2 }` · empty `{} of Text to Int` | — | `New{Map}` + inserts |
| Set, empty | `a new Set of Int` | `a new HashSet of Int` | `Expr::New{Set}` |
| Set, literal | `{1, 2, 3}` (no colon ⇒ set) | — | `New{Set}` + adds |

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
> `item 0 of xs` is a compile error; `xs[0]` currently underflows.
> A project may opt into 0-based indexing
> **(proposed)** via `logos.toml`.

### 5.3 Mutate (lists & sets)

| Meaning | Form | Lowers to |
|---------|------|-----------|
| Append | `Push v to xs.` | `Stmt::Push` |
| Pop | `Pop from xs.` · `Pop from xs into y.` | `Stmt::Pop`; a list with no items stops the program |
| Set add | `Add v to s.` | `Stmt::Add` |
| Set remove | `Remove v from s.` | `Stmt::Remove` |
| Union / intersection | `a union b` · `a intersection b` | `Union`/`Intersection` |

### 5.4 Iterate & transform

| Meaning | Canonical | Also works | Lowers to |
|---------|-----------|------------|-----------|
| For-each | `Repeat for x in xs:` | `for x in xs:` · `Repeat x in xs:` | `Stmt::Repeat` |
| Counted | `for i from 1 to n:` | `Repeat for i from 1 to n:` | `Repeat` over `Range` |
| Pairs (map) | `Repeat for (k, v) in m:` | — | `Repeat` + tuple `Pattern` |
| Filter (a new list) | `Let small be every n in xs where n is less than 4.` | `Set small to every n in xs where …` | desugar to `Repeat`+`Push` (`Stmt::Splice`) |
| Filter (in place) | `Remove every t from todos where t's done.` | — | desugar to `Repeat`+`Push`, written back (`Stmt::Splice`) |
| Map / comprehension | **(proposed)** `[f(x) for x in xs if p(x)]` · `xs.map(f)` · `each x in xs mapped to f(x)` | — | desugar to `Repeat`+`Push` |
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
| Infinitesimal | `Let dt be an infinitesimal [whose reciprocal is even \| odd \| divisible by k \| congruent to r modulo k …].` | facts joined by `and`, each after an optional `not`; a contradiction refused |
| Several infinitesimals | `Let dx be an infinitesimal much smaller than dt.` · `… much larger than dt.` · `… much smaller than dt and much larger than dy.` | smaller than every positive value of `dt`; the bounds of one between must be ordered; a bare `smaller than` and values of infinitesimals of no stated order refused; a value of several residues depends on the coarsest outside; `⌊1/dt + 1/dx⌋` is `1/dx + 1/dt` |
| Supertask | `While t < 1:` … `Set t to t + dt.` | runs `⌈1/dt⌉` times; ends in its exact state (affine closed form, cycle, flow, or a flow with jumps — `If guard:` — run as its hybrid system, Zeno tails included); a supertask in each pass of another ends past every stage `ω·k` (`1/dt^2`), and one that reads what the passes change is written as its map — an iterated limit, over any infinitesimals; a polynomial body whose monomials close ends exactly (`Σ t²·dt`); a count infinite in several (`⌈1/(dt·dx)⌉`) is read by the closed form, the cycle and `exp(N·log A)` |
| Limit rule | `At the limit, take the limit superior.` | also `the limit inferior`, `the Cesàro mean`, `the limit`, `the standard part`; for the supertasks after it in its block |
| Search | `Repeat for every natural n[, checking up to K cases]:` | decided exactly for a linear condition, else provisional with its evidence |
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
| Field read | `p's x` | `p.x` | `FieldAccess` |
| Nested field | `b's location's x` | `b.location.x` | chained `FieldAccess` |
| Field write | `Set p's x to 5.` | `Set p.x to 5.` · `p.x = 5.` | `SetField` |
| Method (UFCS) | `f(xs, a)` | `xs.f(a)` · `xs.f().g()` · `(5).double()` | same `Call` (receiver is arg 0) |
| Enum def | `## A Shape is one of:` then `A Circle with radius Int.` | — | enum `StructDef` |
| Variant construct | `a new Circle with radius 10` | `a new Failure from Outcome with message "x"` (names the enum when two share a variant) | `NewVariant` |
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
| CRDT join (idempotent) | `Converge remote into local.` | lattice join: a per-field MAX, so applying it twice equals once |
| Shared set | `Add "Alice" to p's guests.` | ORSet semantics |
| Shared sequence | `Append "Line 1" to doc's lines.` | RGA semantics |
| Spawn agent | `Spawn an EchoAgent called "echo".` | actor |
| Zone (arena) | `Inside a zone called "Scratch":` | scoped allocation |
| Listen | `Listen on "/ip4/0.0.0.0/tcp/8000".` | raw multiaddr (low-level) |

---

## 13. Output

| Meaning | Form |
|---------|------|
| Print | `Show x.` · `Show "Hello, World!".` |
| Print formatted | `Show "{result:.15}".` |
| Move into a sink | `Give x to consume.` (ownership move) · `Show x to display.` (borrow) |

---

## 14. The web

A page is `## Page /path`: real HTML with `{…}` holes, and English sentences saying what happens.
`State:` is what it knows, `Actions:` what it does, `View:` the markup, `Style:` scoped CSS. A page
mounts itself — there is no `## Main` and no `Mount`. One event is one turn: the action runs,
derived fields recompute, the page repaints once, and if anything fails the state goes back to what
the turn started from. `largo new --web` scaffolds a site, `largo serve` runs it with live reload,
and every page is server-rendered first. The guides: [Frontend](docs/frontend.md) and
[Web](docs/web.md).

| Construct | Form | Notes |
|-----------|------|-------|
| Page | `## Page /about` | owns a URL and mounts itself |
| Page with params | `## Page /todos/:id (id: Int)` | the segment is bound, typed |
| Component | `## Component Card (title: Text) and (done: Bool)` | props in the header, read-only inside |
| Placing one | `<Card title="Inbox" done={false}/>` | attributes are props |
| Callback prop | `(remove: fn(Int) -> Unit)` · `remove={drop}` | the child calls the parent's action |
| Children | `{children}` | where a caller's markup lands |
| State | `Let draft be "".` | one plain record per instance; `mutable` is optional here and never enforced |
| Derived | `Let left always be length of todos.` | recomputed after every turn that changes what it reads |
| Actions | `To add:` · `To remove (id: Int):` | the only place state changes |
| Handler | `on-click={add}` · `on-click={remove(t's id)}` · `on-submit.prevent={save}` | `.prevent` cancels the browser's default |
| Two-way binding | `bind={draft}` | input, textarea, select, checkbox, radio |
| Hole | `<p>Hi {name}</p>` | escaped everywhere, attribute values included |
| Filter | `{title \| truncate 20}` | `truncate(title, 20)` |
| Conditional | `{if c:}…{else if d:}…{else:}…{end}` | |
| List | `{for each t in todos, keyed by t's id, at index i:}…{else:}…{end}` | `i` counts from 1; `{else:}` renders for an empty list |
| Class toggle | `class:done={t's done}` | composes with `class="…"` |
| Scoped style | `Style:` | real CSS, scoped to the component |
| Head | `Head:` | a page's `<title>`, `<meta>`, `<link>` |
| Layout | `{extends Site}` + `{block content:}…{end}` | a layout is a component with `{block}` slots |
| Lifecycle | `When the page loads:` · `When this component disappears:` | |
| Event sentence | `When "#add" is submitted:` · `When the user presses Escape:` | |
| Data sentence | `When query changes:` · `When Cart's count changes:` | |
| Timer | `Every 5 seconds as tick:` · `After 2 seconds:` · `Stop the timer tick.` | stops with its component |
| Failure | `When an action fails (error: Text):` | after the turn rolls back |
| Store | `## Store Cart persisted as "cart"` · `Use Cart.` · `Call Cart's add.` | `preserved across reloads` keeps it for the tab; a store holds any value a page's own `State:` can (`Text`, `Int`, `Real`, `Bool`, a list, or a record); `shared on "topic"` converges a plain field across replicas on the in-process tiers, verified by `shared_store_lock`'s tree-walker/VM parity check |
| Navigation | `Navigate to "/about".` · `Go back.` | `When the route changes:` hears both |
| Typed fetch | `Get "/api/todos" as Seq of Todo, then loaded.` | the action receives `Result of Seq of Todo and HttpError` |
| Directive | `## Directive tooltip (el: Element) and (text: Text):` | a custom attribute behaviour |
| Error page | `## Error page 404` | served for a URL nothing matches |
| Route | `## Route GET /api/todos -> Seq of Todo:` · `Serve on port 8080.` | returns a value; the JSON codec is derived |
| Session | `## Session` + `a user, which is Text.` | `the session's user` reads and writes it, in one signed cookie |
| Server render | `html of Card` | a component's markup as text |
| Page test | `## Test: …` · `Click "#add".` · `Expect that the text of "#n" is equal to "1".` | the page is already mounted when the test starts |
| Component test | `Mount Card.` inside a `## Test` | renders one component to drive it |

---

## 15. Tensors

A tensor has a shape the parser knows; every tensor statement desugars to the loops every tier runs, and the
accelerator backends recognize those loops: a backend with no kernel for one (the CUDA tiers for a soft cap or a
residual's lanes) refuses it by name.

| Construct | Form | Notes |
|-----------|------|-------|
| Declare | `Let A be a 2 by 3 tensor of Real.` | zero-filled; `Set item i of A to v.` fills it |
| Products | `A matmul B` · `X matmul W across H heads` | the second multiplies each head's slice of `X` by its own block of `W` |
| Elementwise | `A plus B` · `A hadamard B` · `X scaled by C` · `X capped at C` | a one-row `B` combines with every row of `A`; a cap is `C·tanh(x/C)` |
| Norms | `rmsnorm of X with weight W` · `layernorm of X with weight W and bias B` · `l2norm of X` | `with epsilon E` names the model's epsilon |
| Activations | `silu of X` · `sigmoid of X` · `gelu of X` · `swiglu of G and U` | |
| Rotation | `rope of Q with cos C and sin S across H heads` | `from position P` reads a cache of angles |
| Attention | `attention of Q over K and V with H heads KV groups up to P` · `causal attention of Q over K and V with H heads KV groups from P` | then `within a window of W`, `scaled by S`, `capped at C`, `with sinks S`; values may be narrower or wider than keys |
| Heads | `the first K of every N of X` · `the last K of every N of X` · `A joined with B across H heads` | one part of every head; each head's parts side by side |
| Mixture | `moe of X with router Wr gate Wg up Wu down Wd choosing K of E` | `plus B` after a weight; `without renormalizing`; `using a swiglu clamped at L with slope A` |
| Rows | `row i of T` · `rows I of T` · `Set row i of T to V.` · `Set rows from P of T to X.` | a lookup and a cache write, checked at run time |
| Recurrence | `causal conv of X with weight WC state CS over T taps` · `gated delta of Q over K and V with …` · `decay of Ar with bias Dt rate R` | carried state |
| Other | `softmax of X` · `argmax of X` · `transpose of X` · `full attention of Q over K and V with H heads` | |
| Integer | `X times M shifted right by S` · `X clamped between LO and HI` · `relu of X` · `lut attention of Q over K and V with H heads KV groups up to P with table E and scale M shifted right by S` | an int8 model's requant (an arithmetic shift), saturation, ReLU and table-softmax attention, every operand `Int` |

---

### Legend

- **(proposed)** — designed but not yet implemented.
- "Lowers to" names the AST node in `crates/logicaffeine_language/src/ast/stmt.rs`.
- Everything unmarked parses in-tree today.
