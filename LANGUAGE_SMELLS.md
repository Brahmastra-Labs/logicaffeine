# LOGOS Language Smells

The pre-1.0 audit of everything a working programmer — Python, Rust, JS, GraphQL,
whatever they came from — would type, expect, and find broken in LOGOS. Read it
as three columns: **what you'd type**, **what LOGOS does today**, **the dream**.
Every row is grounded in a real `file:line`; the audit read the parser, AST,
interpreter, the bytecode VM, the AOT codegen, and the whole benchmark/studio
corpus.

LOGOS's bet is the dual register: the same meaning has an English spelling *and* a
programmer spelling, and both lower to the same primitive. So a *smell* is a form
a reasonable person types that **fails, surprises, or silently does the wrong
thing** — and the fix is almost always *additive*. Companion: `LOGOS_QUICKGUIDE.md`
(how to write every construct that works today).

### First: LOGOS runs a program three ways

Every feature has to behave identically across **all three execution backends**, or
the same program produces different answers depending on how you ran it (that is
exactly the Part I bug class):

1. **Tree-walker interpreter** — `interpreter.rs` (note: *two* paths, async +
   sync), used for the REPL / Studio / `largo run --interpret`.
2. **Bytecode VM** — `compiler.rs` → `machine.rs` → `vm/value.rs`, with the
   JIT/forge tier on top. `largo run`.
3. **AOT Rust codegen** — `codegen/*.rs` emits Rust that rustc compiles. `largo build`.

So a cost tag is about *how many of those backends a fix touches*, not one engine.

### How to read the cost tag

| Tag | What it really costs |
|-----|----------------------|
| `[parser]` | **Surface-only, truly cheap.** The new spelling lowers to an AST node *all three backends already run* — so zero interpreter/VM/codegen work (e.g. `x += 1` → `Set x to x+1`; `k in m` → existing `Contains`; `{k:v}` → `New{Map}`+`SetIndex`; dot field access → existing `FieldAccess`). |
| `[runtime]` | **New behavior — implement in all three backends in lockstep.** A new AST node, a new builtin, or new value semantics has to be added to the tree-walker *and* the bytecode VM *and* the AOT codegen (and stay in sync, or you ship a ⚠ divergence). This is the expensive tag; "runtime" never means "just the VM." |
| `[codegen]` | **AOT-only.** A static-typing concern the dynamically-typed interpreter/VM don't have (return-type inference, derive control, emitting a real Rust tuple). Only the emitted Rust changes. |
| `[lang]` | A real **semantics decision** — what a program *means* (indexing base, overflow, equality, truthiness). Needs *your* call first; then it's implemented across all three backends. |
| ⚠ | **Correctness bug today**: the backends already *disagree*, or a nonzero/valid value silently becomes wrong. These are usually "one engine implements it, the others don't" — the fix is bringing the lagging backends into line. Pre-1.0 blockers. |

---

# Part I — The dangerous ones (correctness-grade, fix before 1.0)

These are not ergonomics. They are silently-wrong results, or the interpreter and
the compiled binary producing **different output for the same program**. A
reviewer who finds any one of these stops trusting the language.

| Symptom | What happens now | Should be | Cost |
|---------|------------------|-----------|------|
| `Show 1.0 / 3.0.` | `0.333333` (interp+VM) but `0.3333333333333333` (compiled) | shortest round-trip float, identical on all engines | `[runtime]` ⚠ |
| `Show 0.0000001.` | prints **`0`** (interp) — a nonzero float vanishes; `1e-7` (compiled) | never render a nonzero value as `0` | `[runtime]` ⚠ |
| `Show 3.141592653589793.` | `3.141593` (interp) — silent 6-digit truncation of the value you typed | echo the stored value | `[runtime]` ⚠ |
| `i64::MAX + 1` | wraps to `i64::MIN` silently (interp/VM) but **panics** in a debug-compiled build (release wraps) | one defined behavior across all three engines | `[lang]` ⚠ |
| `a / 0` | interp returns a catchable error; compiled **panics** | one rule, ideally catchable | `[lang]` ⚠ |
| `[1,2,3] == [1,2,3]` | **`False`** — collections never compare equal (`compare.rs:9,117`) | structural equality | `[runtime]` ⚠ |
| `p == p` (a struct) | **`False`** in interp, **`True`** compiled (`compare.rs:31` vs derived `PartialEq`) | structural, both engines | `[runtime]` ⚠ |
| `1 == 1.0` | **`False`** — yet `1 <= 1.0` is `True`. `a<=b && b>=a` no longer implies `a==b` | numeric `==` coerces like `<` already does | `[runtime/lang]` ⚠ |
| `0.1 + 0.2 == 0.3` | **`True`** interp (epsilon `compare.rs:12`), **`False`** compiled (bit `==`) | pick one; give approx its own spelling | `[lang]` ⚠ |
| `If items:` / `While queue:` | **always true** — empty list/map/set/str are truthy (`interpreter.rs:571`) | empty container is falsy (Python) | `[runtime/lang]` |
| `6 and 3` | **`2`** — `and`/`or` silently mean bitwise `&`/`\|` on ints (`arith.rs:34`) | `and`/`or` stay logical; `&`/`\|` for bits | `[lang]` |
| `not 0` | **`-1`** — `not` is bitwise complement on ints | `not` logical only | `[lang]` |
| `Show ~5.` | **`5`** — `~` isn't a lexer char, silently dropped (`lexer.rs:1168`) | bitwise complement (`-6`) | `[lang]` |
| `Let xs be [1,2,3].` | **one element `[123]`** — the thousands-separator rule glues `1,2,3` | three elements | `[parser]` ⚠ |
| `Let n be 1_000_000.` / `0xff` | parse to **`0`** (`unwrap_or(0)`, `parser/mod.rs:6007`) | underscores + hex/bin/oct, or a hard error | `[parser]` ⚠ |
| `item k of m` (missing key) | **panics** (compiled `.expect`) / errors (interp); no safe read | a `.get(k)` / `m at k or default` form | `[runtime]` |
| struct/tuple as a Set/Map key | hashes by **stub** (Set by length, Struct by type-name) → silent key collisions (`interpreter.rs:477`) | hash over contents, or reject at compile | `[runtime]` ⚠ |
| `Repeat for (k,v) in m:` order | **nondeterministic** (FxHashMap) — diverges run-to-run, unlike Python/JS insertion order | insertion-ordered map | `[lang]` |
| `xs[-1]` | parses, then **OOB error** (negative wraps to huge usize, `collections.rs:74`) | last element | `[runtime/lang]` |
| `"a\nb"` | three chars `a n b` — the backslash escape is **dropped** (`lexer.rs:759`) | decode `\n \t \\ \u{}` | `[parser]` |
| `## Mian` (typo) | **silently swallowed**; whole program runs to empty output (`lexer.rs:2090`) | error: "did you mean `## Main`?" | `[parser]` |
| `a \| b` on two Sets | returns a **Bool** (truthiness OR), not the union (`arith.rs:38`) | set union | `[lang]` ⚠ |
| `Repeat for (a,b,c) in pairs:` | silently **drops `c`** (zip-based bind, `interpreter.rs:2759`) | arity-mismatch error | `[runtime]` |
| `Inspect c:` missing a variant | **silently does nothing** for the unmatched case (`interpreter.rs:1899`) | exhaustiveness error | `[runtime]` |
| `## To greet (n: Text): Return "hi".` | return type **hardcoded to i64** (`types.rs:92`) → wrong compiled type | infer from the returned expression | `[codegen]` ⚠ |
| `Let b be a new Body.` (struct w/ nested+Real fields) | interp fills `Nothing`/`Float`; compiled fills real defaults — **different object** | default from the declared field type, both engines | `[runtime]` ⚠ |
| `x = 5.` then `x = x + 1.` | the second `=` **shadows** (new binding), not mutates (`parser/mod.rs:2971`) | `=` mutates an existing binding | `[lang]` |
| `today` / `now` as a variable name | a builtin **overrides** the user's binding (`interpreter.rs:1909`) | local binding wins | `[runtime]` |
| `Read x from the console.` | returns **`""`** in interp, real blocking read compiled | same on both engines (or loud "unsupported") | `[runtime]` ⚠ |

---

# Part II — The ergonomic catalog, by surface

Each table: **what you'd type** (with the language a programmer brings it from) ·
**today** in LOGOS · **the dream** (programmer form + English form) · cost.

## Lists / arrays

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `xs = [0] * n` (Py) · `vec![0; n]` (Rust) | `new Seq` + a `While`/`Push` fill loop (every benchmark does this) | `[0] * n` · `a Seq of n zeros` | `[runtime]` |
| `grid = [[0]*c for _ in range(r)]` | three flat 1-D seqs + manual `item (i*n+j)` indexing (matrix_mult) | `a grid of r rows by c cols, filled with 0` | `[runtime]` |
| `grid[i].append(x)` · `grid[i][j] = v` | fails — mutation targets must be a bare identifier | allow any place-expr as an l-value | `[runtime]` |
| `xs + ys` (concat) | error "Cannot add List and List" | `xs + ys` · `xs followed by ys` | `[runtime]` |
| `xs * 3` (repeat) | error | `xs * 3` · `xs repeated 3 times` | `[runtime]` |
| `x = xs.pop()` / `xs.pop(i)` | `Pop` is a statement, tail-only, no value | pop-as-expression, pop-at-index | `[runtime]` |
| `xs.remove(v)` / `del xs[i]` | error — `Remove` is Set/Map only | remove by value / by index on lists | `[runtime]` |
| `xs.insert(i, x)` / `xs.extend(ys)` | only tail `Push` exists | insert-at / extend | `[runtime]` |
| `sum(xs)` `min(xs)` `max(xs)` `any` `all` | `min/max` need *exactly 2* args; rest absent | `the sum of xs`, `the largest of xs`, … | `[runtime]` |
| `xs.sort()` `sorted(xs)` `xs.reverse()` | none at all | `Sort xs` · `xs sorted by (x) -> x's age` | `[runtime]` |
| `xs.index(v)` / `xs.count(v)` | none | `the position of v in xs` / `the number of v in xs` | `[runtime]` |
| `xs[1:5:2]` · `xs[::-1]` · `xs[-2:]` | slice has no step, no negative, no reverse | extend `Slice` | `[runtime]` |
| `first, *rest = xs` · `f(*xs)` | none (`first`/`last` aren't keywords for lists) | `the first/last of xs`, splat | `[runtime]` |
| `Push 0, 0, 0 to adj.` (graph_bfs writes 5 separate `Push`es) | one element per statement | multi-push / `extend` | `[parser]` |

## Maps / dicts

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `{"a": 1, "b": 2}` | none — one `Set` per entry | `{"a": 1, "b": 2}` (a `:` ⇒ map) | `[parser]` |
| `m.get(k, 0)` / `m.get(k)` | missing key **panics** (Part I) | `m.get(k, 0)` · `the value for k in m, or 0` | `[runtime]` |
| `k in m` / `k not in m` | only `m contains k` (collection-first) | key-first `in` operator | `[parser]` |
| `m[k] += 1` (Counter / defaultdict) | reads key twice **and panics first time** | auto-vivifying `Increase m at k by 1` | `[runtime]` |
| `m.keys()` `m.values()` `m.items()` | none (iteration is half-built, unreachable) | `the keys of m`, `the values of m` | `[runtime]` |
| `for k in m:` | binds the whole `(k,v)` **tuple** to `k` | single-var iterates keys | `[runtime]` |
| `{**a, **b}` · `a \| b` · `m.update(o)` | manual loop (`Merge` is CRDT-only) | `base updated with other` | `[runtime]` |
| insertion order | **nondeterministic** (Part I) | insertion-ordered | `[lang]` |

## Sets

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `{1, 2, 3}` | none — `new Set` + `Add` loop | `{1, 2, 3}` (no `:` ⇒ set) | `[parser]` |
| `a \| b` · `a & b` (symbolic) | return a **Bool**, not union/intersection (Part I) | dispatch `\|`/`&`/`^`/`-` on Sets | `[lang]` ⚠ |
| `a - b` (difference) | error | `a difference b` · `a without b` | `[runtime]` |
| `a ^ b` (symmetric difference) | none | `a symmetric difference b` | `[runtime]` |
| `a <= b` (subset) · `isdisjoint` | none | `a is a subset of b`, `a is disjoint from b` | `[runtime]` |
| `x in s` / `x not in s` | only `s contains x` | subject-first `in` | `[parser]` |
| `set(xs)` (dedup) · `list(set(xs))` | manual loop | `the distinct items of xs` | `[runtime]` |
| `s.discard(x)` vs `s.remove(x)` | only the silent discard exists | add an erroring `remove` | `[runtime]` |
| `len(s)` | only `length of s` | `len(s)` / `the size of s` | `[parser]` |
| `{x for x in xs if …}` (set comp) | none | set comprehension | `[runtime]` |
| `frozenset(...)` (hashable) | every Set is mutable + stub-hashed | a frozen, content-hashed set | `[runtime]` |

## Tuples

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `for x in t` · `x in t` · `t[0:2]` | all error — tuple is a second-class collection (no iterate/contains/slice arms) | mirror the List read-arms | `[runtime]` |
| `a, b = b, a` (swap) · `x = y = 0` | none — `Set` takes one target | parallel/chained bind, `Swap a and b` | `[runtime]` |
| `Let (x, y) be f()` | tuple destructure only works in `Repeat`, not `Let` | destructuring bind | `[runtime]` |
| `Coord = tuple[int, int]` (type) | no tuple **type** and no type alias | `TypeExpr::Tuple`, `A UserId is just an Int` | `[parser]` |
| `{(1,2), (3,4)}` (tuple keys) | boxed `Vec`, hash-trapped | real Rust tuples (derive Hash/Eq) | `[codegen]` |

## Strings & formatting

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `s.split(",")` `"-".join(xs)` `s.strip()` `s.upper()` `s.replace()` | **phantom** — names known to the optimizer (`effects.rs:692`), no runtime | real builtins, `s.split(",")` + English | `[runtime]` |
| `"=" * 40` (rule line) · `s[1:4]` | error — `*` and slice reject Text | repeat + char-slice on Text | `[runtime]` |
| `"a" + "b"` | error — `+` is numeric `Add`; concat is `combined with` | make `+` join when either side is Text | `[runtime]` |
| `5 combined with 3` | `"53"` — `combined with` force-stringifies numbers, unlike `+` | type-aware, agree with `+` | `[runtime]` |
| `"a\nb"` `"\t"` `"\u{1F600}"` `r"C:\path"` | escapes **dropped** (Part I); no raw strings | decode escapes + `r"..."` | `[parser]` |
| `f"{x:x}"` `{x:b}` `{n:,}` `{x:e}` `{x:03}` `{x:>{w}}` | hex/bin/oct/comma/sci/zero-pad/dynamic-width **silently ignored** | extend the format-spec engine; error on unknown spec | `[runtime]` |
| `ord(c)` | none (only `chr`) | `ord(c)` · `the code of c` | `[runtime]` |
| `print(['a','b'])` → `['a', 'b']` | prints `[a, b]` — strings unquoted inside containers | a `repr`/debug rendering | `[runtime]` |
| `len('héllo')` vs `s[1]` | `length` counts **bytes**, index counts **chars** — they disagree | one code-point model + `byte length` | `[lang]` |
| `''.join(parts)` builder | repeated `Set s to s + piece` (realloc each time) | a mutable string builder | `[runtime]` |

## Numbers & math

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `2 ** 10` · `side ** 2` | parse error — no power op | `2 ** 10` · `2 to the power of 10` · `n squared` | `[runtime]` |
| `7 // 2` (floor div) | none | `//` · `the floor of x divided by y` | `[parser]` |
| `~mask` | silently dropped (Part I) | bitwise complement | `[lang]` |
| `flags & MASK` / `\| MASK` | `and`/`or` pun to bitwise on ints (Part I) | symbolic `&`/`\|`, English `bitand` | `[lang]` |
| `-7 % 3 == 2` (Euclidean) | `-1` — `%` is truncated | keep `%`, add `mod` (Euclidean) | `[lang]` |
| `1_000_000` · `0xff` · `0b1010` | parse to `0` (Part I) | separators + radix literals | `[parser]` |
| `float('inf')` · `float('nan')` | no literal (only via `1.0/0.0`) | `infinity` / `nan` keywords | `[parser]` |
| `gcd(a,b)` `sign(x)` `clamp(v,lo,hi)` `log(x)` `sin(x)` | `log/sin/cos/tan` phantom; `gcd/sign/clamp` absent | implement them; variadic `max(a,b,c)` | `[runtime]` |
| `3 plus 4` · `the product of a and b` | only symbolic `+ - * /` — English arithmetic missing though comparison words exist | add the arithmetic word register | `[parser]` |
| `-1.16…` (negative float literal) | lowered as `Int(0) - x` (nbody writes `0.0 - lit` everywhere) | a real negative float literal | `[parser]` |
| `1 < x < 10` · `lo <= x <= hi` | parse error at the second operator | chained compare · `x is between lo and hi` | `[parser]` |
| `i % 15 == 0` · `n.is_even()` | showcase teaches the `i/15*15 equals i` trunc trick | `is divisible by` · `is even` · `is odd` | `[parser]` |

## Conversions / casts

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `int(x)` `float(x)` `str(x)` | **phantom** — in the purity table, no runtime | `Int(x)` / `x as Text` constructors | `[runtime]` |
| `int("42")` (no setup) | must declare `## To native parseInt …` first | `Int(s)` parses Text with no FFI line | `[parser]` |
| `int(2.9)` (truncate) | only floor/ceil/round | a truncate-toward-zero cast | `[runtime]` |
| `int(True)` → `1` · `sum(p>0 for …)` | no Bool→Int path | `Int(predicate)` | `[runtime]` |
| `i64::try_from` / saturating cast | float→int silently **saturates** | choose policy: `Int(x)` vs `Int(x) exactly` (Option) | `[lang]` |

## Iteration

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `for i, x in enumerate(xs):` | manual counter — **the reason every benchmark abandons `for-in`** for a `While`+index | `for (i, x) in enumerate(xs)` · `for each x at index i in xs` | `[runtime]` |
| `for a, b in zip(xs, ys):` | dual manual index (knapsack does this) | `for (a, b) in zip(xs, ys)` · `… alongside …` | `[runtime]` |
| `range(n, 0, -1)` · `range(0, n, 2)` | `from a to b` only; descending range runs **zero times silently** | `from n down to 1` · `from 0 to n by 2` | `[runtime/lang]` |
| `continue` | none — only `Break` | `Continue` · `Skip to the next one` | `[runtime]` |
| `break 'outer` (labeled) | unit `Break` only; inner loops can't exit outer | labeled break · `Stop scanning` | `[runtime]` |
| `for … else:` (no-break epilogue) | none | a loop completion block | `[runtime]` |
| `while True:` / `loop {}` | `Repeat` requires `in`/`from`; `While` needs a cond | `Repeat forever:` | `[parser]` |
| `do { } while (c)` | pre-tested only | trailing-condition repeat | `[parser]` |
| `pass` (no-op) | none | `Pass` · `Do nothing` | `[runtime]` |
| `with open(p) as f:` | only memory `Zone` | a general acquire/release block | `[runtime]` |
| `zip` / `pairwise` / `windows` / `step_by` | none | iterator adapters | `[runtime]` |

## Control flow & statements

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `if n <= 1: return 1` (one-line guard) | requires `:` + indented block → two lines | inline `If c: Return 1.` · `Return 1 if c.` | `[parser]` |
| `a if c else b` (ternary) | none — declare a temp then `If`/`Otherwise` | `b if c else a` · `a, but b if c` | `[runtime]` |
| `match code: 200 => … 404 => …` | `Inspect` matches enum **variants only**, no literals, no guards | literal patterns + `When n where …` guards | `[runtime]` |
| `x := f()` (walrus) · `x += 1` | none · compound assignment missing | augmented family + `Increase x by 1` | `[runtime/parser]` |
| `a, b = b, a` | none (also Tuples) | parallel bind · `Swap a and b` | `[runtime]` |

## Functions & closures

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `map(double, xs)` · `let f = double` | a `## To` function **isn't a value** — `Let g be double` fails | a bare function name evaluates to its callable | `[runtime]` |
| `lambda x: x*2` · `\|x\| x*2` | every closure param needs `: Type` — `(x) -> x` is a parse error | optional annotations, inferred from context | `[parser]` |
| `def` / `fn` / `function` | only `## To`; `## Fn`/`## Def` silently become prose | accept synonyms; warn on typo'd header | `[parser]` |
| `def f(x, y=1)` (defaults) | arity is exact; no default slot | trailing `= expr` defaults | `[runtime]` |
| `connect(host=…, port=…)` (named args) | strictly positional, both call forms | labeled args · `with host being …` | `[parser]` |
| `def f(*args)` | none (yet `Show` is variadic internally) | a trailing variadic param | `[runtime]` |
| `fn inc(n) { n + 1 }` (implicit return) | bare trailing expr is a parse error; must `Return` | allow last-expression return | `[parser]` |
| `x \|> f \|> g` · `partial(add, 5)` | no pipe, no composition, no partial application | `take x, then f, then g` · `add with 5` | `[parser]` |
| `yield` / generators | none — must build the whole list eagerly | `Yield i` · `Produce i` | `[runtime]` |
| `point.distance_to(other)` | behavior is free functions only (`distance(p, q)`) | methods under a type, `p.distance to(q)` | `[parser]` |

## Types, structs, enums, generics *(the GraphQL-schema lens)*

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `retries: Int = 3` (default field, like a GraphQL input default) | none — omitted fields get type-zeros | `a retries: Int = 3` · `… defaulting to 3` | `[runtime]` |
| omitted fields agree run vs compile | interp gives `Nothing`/`Float`, compiled gives real defaults (Part I) | one default rule, both engines | `[runtime]` ⚠ |
| `match` covers all variants | non-exhaustive `Inspect` silently no-ops (Part I) | exhaustiveness diagnostic | `[runtime]` |
| `#[derive(Hash, Eq, Ord)]` | structs derive only `PartialEq`; can't be a Set/Map key or sorted | opt-in `A Point is hashable and ordered` | `[codegen]` |
| `Some(0) != 0` · `match opt {…}` | `some 0` evaluates to `0` — Option is **erased at runtime** | a real `Option` runtime value | `[runtime]` |
| `return Ok(x)` / `Err(e)` | `Result` type exists but no `Ok`/`Err` constructors or match | `Succeed with x` / `Fail with e` + arms | `[runtime]` |
| `String!` vs `String` (nullability discipline) | Option/None unwrapping is `Inspect`-only; no `?.`/`??`/`if let` | optional chaining + coalesce | `[runtime]` |
| `MyList<T>` / `MyList[T]` | type params require literal `[T]` brackets | bare `of T` binder · `for any type T` | `[parser]` |
| `Person(name="Alice", age=30)` | `new Person` + per-field `Set`, or `with x 10 and y 20` (no `:`/`=`) | `Person { name: "Alice", age: 30 }` | `[parser]` |
| `Circle(10)` / `match { Circle(r) }` | enum payloads are named-only; no positional construct/match | positional variants | `[parser]` |

## Errors, optionals, contracts

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `try: … except E as e: … finally:` | none (`Try` is pipe-channel only) | `Try: … Catch e: … Finally:` | `[runtime]` |
| `let v = f()?` | no `?` propagation | `Let v be f()?` · `… or return that failure` | `[runtime]` |
| `x ?? default` · `a?.b?.c` | none — only statement-level `Otherwise` | coalesce + optional chain | `[runtime]` |
| `assert x > 0, "msg"` | assert carries no message | optional `, otherwise "msg"` | `[runtime]` |
| `panic!("…")` / `raise` | no deliberate-abort statement | `Fail with "…"` | `[runtime]` |
| `int(s)` that returns None on bad input | `parseInt` **aborts**, no Option variant | Option-returning parse | `[runtime]` |
| `Require` enforced everywhere | `hard` flag honored compiled, **ignored** in interp | one meaning per verb across engines | `[runtime]` ⚠ |
| `assume(x)` (unchecked hint) vs `assert` | `Trust` is `debug_assert` (stripped in release), no-op in interp | split enforced vs hint, consistently | `[runtime]` |

## Modules, structure, I/O, comments

| What you'd type | Today | Dream | Cost |
|---|---|---|---|
| `import geometry` / `use geometry::Point` | a **Markdown hyperlink** buried in the first prose paragraph | a real `Import "geo.md" as Geometry` statement | `[parser]` |
| call an imported function | imports merge **types only**, never functions | merge a function registry too | `[runtime]` |
| `pub` / private helper | no visibility concept; everything is exposed | a `private` modifier / `_` convention | `[lang]` |
| top-level script (no `main`) | no `## Main` ⇒ parsed as logic prose, **runs empty** (Part I) | implicit Main, or error | `[parser]` |
| `size` / `length` as a variable name | rejected (keywords) though `read` is allowed — inconsistent | admit all keywords as identifiers in id position | `[parser]` |
| `//` or `/* */` comments | only `#` (undocumented); no doc-comments | document `#`, add `//`, capture doc-paragraphs | `[parser]` |
| a sentence between code lines | parse error inside `## Main` | tolerate prose as an attached comment | `[parser]` |
| `input()` / `read_line()` | not implemented — interp returns `""` (Part I) | a real stdin builtin | `[runtime]` |
| `print(x, file=sys.stderr)` | no stderr stream | `Show x to the error stream` | `[runtime]` |
| `print(x, end='')` | every `Show` forces a newline | a no-newline print | `[runtime]` |
| `Write x to the console` | rejected — `Write` needs the `file` keyword | unify the output target | `[parser]` |
| `s = f"{x:.2f}"` (capture, don't print) | `Show` can't yield a value | `Let s be the text of x` (interpolation already builds it) | `[runtime]` |
| one naming convention | benchmarks mix `makeCheck`, `mulAtv`, `is_prime`, `take_collatz_step` | pick one, normalize the corpus | `[docs]` |
| `n = int(sys.argv[1])` | the `args()`+`parseInt(item 2 …)` prologue, declared by hand every file | `the integer argument 1` | `[parser]` |

---

# Part III — The keystone unlocks (highest leverage)

A handful of changes each dissolve a whole column of the tables above:

1. **The dot `.`** `[parser]` — `.` is free (lexer uses it only between digits).
   One conditional token enables both `person.id` field access *and* the entire
   `xs.map()` / `m.get(k)` / `s.split(",")` / `xs.sort()` method register. Nearly
   every "Dream" cell with a `.method()` rides on this.
2. **`{…}` map & set literals + spaceless `[…]`** `[parser]` — fixes the
   list/map/set literal asymmetry and the `[1,2,3]`→`123` correctness bug at once.
3. **Generalize `Increase X by N`** `[parser]` — already exists for CRDT fields;
   open it to plain vars and indexed targets and you get English `+=` for free,
   plus auto-vivifying Counter maps.
4. **Real runtimes for the phantom builtins** `[runtime]` — `split/join/trim/upper/
   int/float/str/log/sin/gcd` are already declared pure; the optimizer trusts
   names the interpreter can't run. Implement them and a dozen rows go green.
5. **One equality + one float-display path** `[runtime]` — fixes the bulk of Part I's
   ⚠ divergences in two functions (`compare.rs`, `to_display_string`).
6. **Comprehensions desugar to the benchmark loops** `[runtime]` — `[f(x) for x in xs]`
   lowers to the exact `Repeat`+`Push` the corpus hand-writes, so the sugar is
   AOT-parity by construction.

---

# Part IV — Nothing is removed; load-bearing ambiguities stay

**Every form that works today keeps working.** All sugar in this doc is *additive*
— a new spelling that lowers to the same primitive — so no existing program breaks.
The forms below are the ones LOGOS already reads *well*; they are kept verbatim and
the new programmer-spellings sit beside them as alternatives, never replacements:

- Possessive field access `p's x` (dot `.x` is added *alongside* it).
- Refinement types `Let x: Int where x > 0 be 5`.
- String interpolation with format specs `{pi:.2}`, `{s:>10}`, `{v=}`, `{price:$}`.
- Closures `(x: Int) -> expr` and the block form `(x) ->:`.
- CRDT verbs `Increase c's points by 10` / `Merge remote into local`.
- Pattern matching `Inspect … When … Otherwise`.
- Temporal literals `500ms`, `2026-05-20`, `9:30am`.
- The English comparison words (`is at least`, `is less than`, …) **and** the
  symbolic operators (`<=`, `==`, `+`, `*`) — both already coexist; both stay.
- Slicing `items a through b of xs` and the typed empty `[] of Int`.
- Quantified FOL and the `Given/Prove/Auto` theorem form.

These existing **ambiguities are intentional and must be threaded *through*, never
flattened** — new sugar was chosen specifically not to collide with them:

- `item`/`items` keyword-vs-variable lookahead (`parser/mod.rs:5796`).
- `set` tri-modal: assignment keyword vs `Set of Int` type vs noun.
- the `Ambiguous { primary, alternatives }` token mechanism for word-class clashes.
- `[]` (and the new `{}`) require `of T` when empty — prevents type ambiguity.
- optional `of` in slices (`items 1 through mid`).

When normalizing the showcase, *rewrite* programs to the cleaner spelling but never
delete the spelling itself from the grammar — old corpora and user code must still
parse. Every desugaring ships with an equivalence test proving the old and new
forms produce identical evaluation and codegen.

---

# Part V — The index-base decision & the showcase

**Indexing** is the one place a `[lang]` choice is unavoidable. Keep **1-based**
default (it matches the English "the first item"), but (a) extend the `ZeroIndex`
guard to the bracket and variable forms so `arr[0]` is a clean compile error like
`item 0 of x` already is, and (b) add an opt-in **0-based** project setting
(`logos.toml` → `[language] index_base`, threaded through `CompileOptions` into
the parser, which is not wired today). Negative indexing (`xs[-1]`) folds in here.

**The showcase contradicts itself** and must be normalized: benchmarks avoid
`for-in`/comprehensions, FizzBuzz ships three else-if spellings, map writes appear
three ways, function names mix camelCase and snake_case, and PRNG constants are
unreadable because digit separators are broken. After the sugar lands, rewrite
`benchmarks/programs/*/main.lg` and `apps/logicaffeine_web/src/ui/examples.rs` to
one canonical style that *demonstrates* the new forms — switching to `for-in` /
comprehensions only where measured AOT parity holds. Golden output must stay
byte-identical; every desugaring carries an equivalence test.

**Suggested order:** Part I ⚠ correctness fixes first (they are bugs, not
features) → the keystones (Part III) → the per-surface ergonomics → the index-base
and showcase pass. Strict TDD per `CLAUDE.md`; start and end all-green.

---

# Part VI — Type less (verbosity & ceremony)

Not missing features — these already work. The cost is **keystrokes**: forms you
type a lot, for a little, over and over. Measured against the 30+ benchmark corpus.
Most collapse onto the Part III keystones (dot/bracket, `{}` literals, the `[]`
empty-literal, the generalized `Increase` verb).

| Today — you must type | Could be | Cost |
|---|---|---|
| `Let mutable arr be a new Seq of Int.` — ~9 words to make an empty list | `Let mutable arr be [].` · `arr = []` | `[parser]` |
| `Let mutable` on *every* reassignable var (30+ per program) | `Let arr be []` + infer mutable on first reassignment · bare `arr = 0` | `[lang]` |
| `## To native args () -> Seq of Text` + `## To native parseInt …` + `Let n be parseInt(item 2 of arguments).` — the identical CLI prologue in **every** benchmark (~30 tokens × ~30 files) | `Let n be the integer argument 1.` | `[parser]` |
| `Set item (v+1) of counts to (item (v+1) of counts) + 1.` — the index path typed **twice** | `Increase item (v+1) of counts by 1.` · `counts[v] += 1` | `[parser]` |
| `item (i*n+k+1) of a` repeated; nested `item j of (item i of grid)` (matrix_mult's ~95-token line) | `a[i*n+k]` · `grid[i][j]` | `[parser]` |
| `a public name, which is Text.` — ~7 words per struct field | `name: Text.` (article + `which is` become optional) | `[parser]` |
| `Show "" + item 1 of x + " " + item 2 of x.` — `""` hack + manual spaces | `Show a, b, c.` (variadic, space-joined; `Show` is already variadic internally) | `[runtime]` |
| `Return n * n.` on a one-liner; `Return` ending every branch | `n * n` (last-expression return — see *Functions*, Part II) | `[parser]` |
| the showcase writes `is less than` / `is at most` / `equals` everywhere though `<` / `<=` / `==` already exist | prefer symbols in tight loops, words in human-facing guards — a corpus normalization, not a new feature (Part V) | `[docs]` |

---

# Part VII — Missing basics a 1.0 needs

Whole capabilities a programmer assumes are present, grounded in-tree as absent.

| What's absent | Why it bites | The dream | Cost |
|---|---|---|---|
| **Named constants** (`const`) | magic numbers (`1103515245`, `1000000007`, π) repeat across files — unreadable, copy-paste-fragile | `Let PRNG_MULT be 1103515245.` at module scope, visible to all functions | `[runtime]` |
| **Multi-word identifiers** | `is prime` as a name lexes as `is` (copula) + `prime` (adjective) and fails — forcing `is_prime`/`isPrime`, fighting LOGOS's own English brand (`lexer.rs:599`, `mod.rs:7183`) | fold a name-shaped word run into one identifier | `[parser]` |
| **Line continuation** | newline ends a statement; a long bitwise/boolean expression (nqueens, matrix_mult) must be one physical line | continue inside unclosed `()`/`[]`, or after a trailing operator | `[parser]` |
| **Type-inference burden** | params, struct fields, *and* closure params all force `: Type` even when context fixes it (`mod.rs:6509`, `5350`) | optional annotations, inferred from the call/field context | `[parser]`+`[runtime]` |
| **Indentation rules** | tab = 4 spaces; mixing tabs/spaces is silent (`lexer.rs:131`) | fix the unit on first indent and enforce it; clear error on mismatch | `[parser]` |
| **RNG** (`random`) | a `std/random.lg` stub exists but no builtin — no simulations, games, shuffles | `Let r be a random Int from 1 to n.` | `[runtime]` |
| **Environment variables** | `std/env.lg` stub, not wired | `the environment variable "PATH"` | `[runtime]` |
| **JSON** encode/decode | no way to round-trip a data structure | `Parse JSON s.` · `the JSON of x` | `[runtime]` |
| **Regex / string patterns** | no match/extract beyond manual index loops | `s matches "[0-9]+"` · `the matches of … in s` | `[runtime]` |
| **Clock / time access** | temporal *literals* exist, but no read of the wall/monotonic clock for timing | `the current time` · `monotonic now` | `[runtime]` |
| **Test / spec construct** | assertions live inline in `## Main`; no named cases that report by name | `Test "square of 5": Assert square(5) equals 25.` | `[parser]`+`[runtime]` |
| **Enum / type methods** | behavior is free functions only (cross-ref *methods on types*, Part II) | `## A Shape … To area …` + `s.area()` | `[parser]` |
| **Type aliases** | `TypeDef::Alias` exists but is **unreachable** — no surface syntax reaches it | `A UserId is just an Int.` (cross-ref *tuple type*, Part II) | `[parser]` |

---

# Part VIII — The literate & proof surface

The Markdown/FOL writing layer the programmer-focused passes skipped.

| What's the deal | The dream | Cost |
|---|---|---|
| Every proof repeats `## Theorem:` / `Given:` / `Prove:` / `Proof: Auto.`; the Simon puzzle needs 25+ identical `Given: X is Y.` lines | a batch `Given: each of …` form; default `Proof: Auto.` when the `Proof:` line is omitted | `[parser]` |
| Header punctuation is inconsistent — `## Main` (no colon) vs `## Theorem:` vs `## Definition` vs `## To f …:` | one punctuation rule across all `##` headers | `[parser]` |
| `## Main` is an H2 — it collides with a user's own prose `##` headings, and an unknown header silently becomes a `Note` (the same root as the Part I typo-header bug) | a code-header namespace distinct from prose Markdown; error on an unknown code header | `[parser]` |
| a sentence between code lines is a parse error inside `## Main` (cross-ref *prose between code lines*, Part II) | tolerate a prose line as an attached doc-comment | `[parser]` |

---

*Cross-references:* compound assignment, methods-on-types, tuple/alias types, the
negative-float literal, and the phantom string methods already have rows in Parts
I–II — the sections above point to them rather than duplicating them.
