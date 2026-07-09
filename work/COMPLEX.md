# COMPLEX.md — Static Complexity Analysis for LOGOS

> Status: specification. This document describes a feature to be built; it does not
> describe code that exists yet. Every reference of the form `file:line` points at
> infrastructure that *does* exist today and that the feature reuses. The analyzer
> is the new layer; the layer beneath it is already in the tree.

## 1. Overview & motivation

LOGOS can already tell you *what it did* to a program: `largo opts <file>` reports
which optimizations actually fired, what blocked what, and which firings depended on
others (`apps/logicaffeine_cli/src/cli.rs:258` → `cmd_opts` at `:678` →
`compile::optimization_graph`). Optimizations became an inspectable, queryable
*property of the code*.

This feature makes **asymptotic complexity** the same kind of property. You ask the
compiler, and it answers — per function — the **time** cost (operation count), the
**space** cost (peak auxiliary memory and stack), and the **structure** behind those
numbers (which loop nest, which branch path, which recurrence produced the bound).
The headline surface is a *complexity flamegraph*: not a runtime profile, but a
**static, structural** picture where depth is loop/branch nesting, width is cost
share, and color is cost class.

The load-bearing observation: **the optimizer already does the hard half.** To
vectorize, hoist bounds checks, and eliminate redundant work, LOGOS already computes
symbolic loop trip counts, induction-variable bounds, and recursion structure, and it
already has a *sound* linear-arithmetic prover to certify them. Complexity analysis is
mostly a matter of *collecting and composing* facts the compiler already derives,
plus a recurrence solver on top. We are assembling an engine from proven parts, not
inventing the parts.

### How this differs from a profiler

A profiler measures one run. This analyzer reasons about *all* runs, symbolically, in
terms of input sizes (`n`, `m`, `|V|`, `|E|`, `length(arg)`). It is closer to a
mechanized version of the Big-O reasoning a competent engineer does by hand — except
it is **sound by construction** (its upper bounds are LIA-certified, never guessed)
and it is **honest** (when a bound is undecidable, it says so, with a reason, rather
than inventing a number).

### Locked design decisions

These four decisions were made with the project owner and shape the whole spec:

1. **Full stack.** Five waves, in order: cost engine → `largo complexity` CLI →
   kernel-certified bounds → Studio complexity-flamegraph → static-vs-measured
   cross-check.
2. **Sound + certify-what-fits.** Every stated bound is a *proven consequence* of the
   loop/recurrence facts via the kernel LIA prover internally. For the cases the
   kernel can already express (linear/polynomial bounds, structural-recursion
   recurrences) the engine additionally emits a kernel-checked `DerivationTree`
   certificate. Logs and exponentials are *derived* but marked
   "certificate pending" until the asymptotic layer (§9) lands. No Z3 in the trusted
   path.
3. **Time + space + opt-delta.** Report operation-count time, peak/auxiliary space,
   **and** the before-vs-after-optimization complexity class, attributing any class
   change to the pass that caused it.
4. **Empirical cross-check.** Wire the static Big-O to the benchmark harness's
   empirically-fitted scaling exponents and flag mismatches.

## 2. Design principles

- **Sound by construction, never a guess.** A reported upper bound is only stated when
  the underlying nonnegativity / dominance obligation is discharged by
  `optimize::affine::prove` (`crates/logicaffeine_compile/src/optimize/affine.rs:92`),
  which is backed by the overflow-safe Fourier-Motzkin decision procedure
  (`crates/logicaffeine_kernel/src/lia.rs:588`). The prover **fails closed**: on
  overflow or non-derivability it declines, and the engine widens to `Unknown` rather
  than fabricating.
- **Honest leaves.** The cost type carries two distinct "I don't have a number"
  leaves: `Unknown(reason)` (analyzable in principle, undetermined here) and
  `Unbounded(reason)` (proven to have no finite bound, e.g. a data-dependent `while`
  with no ranking function). They are never collapsed into each other, and every
  widening records *what* was dropped and *why* (the project's "report what was
  dropped" / no-silent-caps culture).
- **Symbolic in input sizes.** Costs are expressions over `SizeVar`s, not constants.
  A constant fallback is used only when nothing symbolic is provable, and it is marked
  as coarsened.
- **Certify where the kernel can; admit where it can't.** The `certified: bool` flag
  is true iff *all* dominant-path proof obligations were discharged. Honesty about the
  boundary is a feature, not a defect.
- **Reuse, don't reinvent.** The engine consumes `loop_shape`, `affine`,
  `abstract_interp`, `callgraph`, `tail_call`, `inline_recursive`, and `closed_form`.
  It re-implements none of them. The one new piece of *infrastructure* it needs is a
  read accessor on `OracleFacts` (§15); everything else is composition.

## 3. Architecture & data flow

The analyzer is a **read-only diagnostic**, a sibling of `analysis/callgraph.rs` and
`analysis/liveness`, living at:

```
crates/logicaffeine_compile/src/analysis/complexity/
    mod.rs      // analyze_complexity, ComplexityReport, FunctionComplexity
    cost.rs     // CostExpr, Monomial, normalize, asymptotic comparator, big_o()
    loops.rs    // trip-count + loop cost (loop_shape + affine + oracle)
    recur.rs    // recurrence extraction + Master theorem (callgraph + tail_call)
    space.rs    // allocation / peak / stack model
    report.rs   // BigO rendering, certified flag, unknown_reason, dropped list
```

It is **not** an `Opt` pass. It transforms nothing, so gating it behind an
optimization flag would be a category error. It runs at the existing compile site
where both the un-optimized and optimized statement vectors are in scope —
`compile.rs:818` holds the resolved un-optimized `stmts`; `compile.rs:824` produces
the optimized vector via `optimize_program`. The analyzer runs **twice**, once per
snapshot, to produce the opt-delta (§10).

Top-level entry:

```rust
pub fn analyze_complexity(
    stmts: &[Stmt],
    interner: &Interner,
    facts: &OracleFacts,          // pre-run via optimize::oracle_analyze_with
) -> ComplexityReport;

pub struct ComplexityReport {
    pub functions: Vec<FunctionComplexity>,
}

pub struct FunctionComplexity {
    pub name: Symbol,
    pub time:  CostExpr,          // exact symbolic operation count
    pub time_bigo: BigO,          // dominant-term projection
    pub space: CostExpr,          // peak auxiliary + stack
    pub space_bigo: BigO,
    pub certified: bool,          // all dominant-path LIA obligations discharged
    pub unknown_reason: Option<UnknownReason>,
    pub dropped: Vec<DroppedTerm>,// every sub-cost widened to Unknown, with cause
    pub size_params: Vec<(Symbol, SizeVar)>, // which inputs the cost is over
}
```

`OracleFacts` is produced by `optimize::oracle_analyze_with(stmts, interner)`
(already used at `compile.rs:532`). **Critical:** `OracleFacts` keys facts by AST
arena address. The pre-opt and post-opt snapshots live in different arenas, so each
`analyze_complexity` call must be paired with the `OracleFacts` computed from the
*same* slice (§Pitfalls).

Processing order: build the call graph once
(`CallGraph::build`, `analysis/callgraph.rs:26`), condense to its SCC DAG
(`callgraph.rs:242`), and walk SCCs in reverse-topological order (callees first) so a
caller's non-recursive callees already have memoized `Symbol → CostExpr` costs.

## 4. The Cost IR (`CostExpr`)

A symbolic cost algebra whose normal form is a sum of monomials, with `Max`,
`Unknown`, and `Unbounded` pushed to the top.

```rust
// analysis/complexity/cost.rs
use logicaffeine_kernel::lia::Rational;   // exact coefficients, reused wholesale

/// An input dimension the cost is expressed over. Backed by the SAME i64 index
/// space as affine::vidx(Symbol) so a linear CostExpr round-trips losslessly into
/// a kernel LinearExpr for proof obligations.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SizeVar(pub i64);

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CostExpr {
    Const(Rational),
    Monomial(Monomial),     // coeff · Π vᵢ^aᵢ · Π (log vⱼ)^bⱼ · Π baseₖ^varₖ
    Sum(Vec<CostExpr>),     // sequencing / additive composition
    Max(Vec<CostExpr>),     // branch worst-case; irreducible
    Unknown(UnknownReason),
    Unbounded(UnboundedReason),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Monomial {
    pub coeff: Rational,
    pub powers: BTreeMap<SizeVar, u32>,  // polynomial part
    pub logs:   BTreeMap<SizeVar, u32>,  // (log v)^b
    pub exps:   BTreeMap<ExpBase, u32>,  // base^(rational · v)
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ExpBase { pub base: Rational, pub coeff: Rational, pub var: SizeVar } // base^(coeff·var)
```

Why these choices:

- **Exact `Rational` coefficients** reuse `kernel::lia::Rational`
  (`crates/logicaffeine_kernel/src/lia.rs:48`). No floats anywhere — the linear core
  of a monomial reifies directly into a `LinearExpr` (`lia.rs:220`) for proof.
- **`SizeVar` shares `affine::vidx`'s index space**
  (`crates/logicaffeine_compile/src/optimize/affine.rs`), so the degree-1 part of any
  monomial is exactly a kernel `LinearExpr` — that is the bridge that makes the engine
  sound-by-construction rather than a separate, unverified arithmetic.
- **`Max` is irreducible** and distinct from `Sum`: asymptotically
  `O(max(f,g)) = O(f+g)`, but a branch *produces* a `Max`, and the exact-count report
  must keep the distinction; the Big-O projection collapses it.
- **`ExpBase` tracks the multiplier on the exponent variable** so `2^n`, `2^{n/2}`,
  and `b^n` are distinguishable (`coeff=1`, `coeff=1/2`, different `base`). Collapsing
  these is the classic way `fib` and `mergesort` wrongly land in the same class.

Operations:

- `add` (sequencing): concatenate into `Sum`, then `normalize`.
- `mul` (loop bound × body): distribute over `Sum`; multiply monomials by merging the
  `powers`/`logs`/`exps` maps and multiplying `coeff`. `Unknown · x = Unknown`;
  `Unbounded · (nonzero) = Unbounded`; `x · 0 = 0`.
- `max`: collect into `Max`, drop members strictly dominated by another under the
  comparator below.
- `subst(SizeVar → CostExpr)`: bind a callee formal parameter's size to the caller's
  argument size at a call site. Substituting into a `^a` power raises the substituted
  expression to `a`; defined cleanly for a substituted `Const`/`Monomial`, expanded by
  binomial for tiny `a` when the argument is a `Sum`, else widened to
  `Unknown(SubstNonMonomial)` (recorded in `dropped`, never silently dropped).

Normalization:

1. Flatten nested `Sum`/`Max`.
2. Combine like monomials (identical `powers`/`logs`/`exps`) by adding `coeff`.
3. Drop zero-coeff monomials; fold pure `Const`.
4. **Asymptotic pruning** is applied *only when projecting to Big-O* — the exact
   `CostExpr` retains every monomial (so the precise operation count, e.g. the Gauss
   `n(n+1)/2`, survives for the exact-count report).
5. Push `Unknown`/`Unbounded` to the top (absorbing).

`normalize` must be **idempotent and order-insensitive** — leaned on `BTreeMap`
key ordering exactly as `LinearExpr` relies on sorted elimination (`lia.rs:594`).

The **asymptotic comparator** is a total preorder on monomials:

1. **Exponential rank** dominates: any monomial with a non-empty `exps` map beats any
   poly-log monomial. Among exponentials compare `(base, coeff)` lexicographically, so
   `3^n ≻ 2^n ≻ 2^{n/2}`.
2. **Polynomial degree** `Σ powers[v]` next: `n^3 ≻ n^2 ≻ n`.
3. **Log degree** `Σ logs[v]` last: `n·log n ≻ n`.
4. Equal classes ⇒ same Big-O (coefficients ignored for Landau, retained for exact).

Cross-variable terms (`|V|·|E|` vs `|V|^2`, `n·m`) are **incomparable** without a
supplied relation and are kept as an irreducible `Sum` — this is how BFS reports
`O(V+E)` honestly rather than guessing an ordering (§Pitfalls).

`big_o()` extracts the comparator-maximal element(s) of the normalized sum and renders
to a `BigO` enum (`O(1)`, `O(log n)`, `O(n)`, `O(n log n)`, `O(n^2)`, `O(n^2 log n)`,
`O(n^3)`, `O(2^n)`, `O(b^n)`, and literal sums like `O(V+E)`).

## 5. Time-cost derivation

### Counted loops (the common case)

For a `CountedLoop { var, start, limit, inclusive, body_without_increment }`
(`crates/logicaffeine_compile/src/loop_shape.rs:29`), recognized by
`extract_counted_while` (`loop_shape.rs:45`), the exact trip count is
`limit − start (+1 if inclusive)`. The extractor already rejects loops whose limit
symbols are mutated in the body, so a recognized counted loop has a **loop-invariant**
trip count — exactly what we need.

`trip_count(loop) -> CostExpr`:

1. `affine::lin_of(loop.limit)` and `lin_of(loop.start)` (`affine.rs:102`). If both are
   affine, `trip_lin = limit_lin − start_lin (+1)`.
2. **Resolve each symbol in `trip_lin` to an input-size `SizeVar`**, in three tiers:
   - (a) the symbol *is* a function parameter (integer) or `length(param)` → identity.
   - (b) the symbol has a symbolic `scalar_upper` / `length_def` in terms of inputs →
     substitute. These maps already exist inside the oracle's working state
     (`abstract_interp.rs:1718` `scalar_upper`, `:1698` `length_def`); exposing them is
     the §15 prerequisite.
   - (c) only a concrete `expr_int_range` is known → use the finite constant as
     `Const` (correct but coarsened — flagged).
   - else → `Unknown(UnresolvedLimitSymbol(sym))`.
3. The resolved `trip_lin` becomes a degree-1 `Monomial`. **Soundness obligation
   discharged here:** the loop/path guards are reified via `lin_of` and the claimed
   bound's nonnegativity is checked with `affine::prove` (`affine.rs:92`). If the trip
   can go negative under the guards (possibly-empty loop), the cost is
   `Max(Const(0), monomial)` — mirroring the `If limit >= start` guard `closed_form`
   already emits (`optimize/closed_form.rs`).

### Non-affine counted limits

If `lin_of` returns `None` (e.g. `limit = m*k`), attempt **synthesis-by-verification**:
enumerate a small ladder of candidate dominating monomials from the in-scope size vars
(`n`, `n^2`, `n·m`, …) and ask `prove(guards, candidate − limit)` for each; take the
least that certifies. None proves → `Unknown(NonAffineLimit)`. (Note `affine::prove`
is a *verifier*, not a *synthesizer* — it confirms a candidate, it does not produce
one; the engine must supply candidates. See §Pitfalls.)

### Geometric / strided loops

`extract_counted_while` recognizes unit stride only. For `i *= b` (b > 1) bounded by
`lim`, reuse the `MulByTwo` recognizer shape (`closed_form.rs:109`) lifted to a
predicate: trip = `log_b(lim)` → a monomial with `logs[size_of(lim)] = 1`. This is how
binary search and heap sift-down get `O(log n)`.

### `while` with a ranking variant

`Stmt::While` carries an optional `decreasing` measure (`ast/stmt.rs:206`, field at
`:210`). When present, treat it as a ranking function `r`: if `r` is affine in the
inputs and the body provably decreases it by ≥ 1 per iteration (a `Set d to d − k`
with `k ≥ 1`, or `prove(body_effect, r_pre − r_post − 1) ≥ 0`), then
`trip ≤ r_entry` → `CostExpr(r_entry)`. Otherwise `Unknown(RankingNotAffine)`.

### `Repeat` over a collection

`extract_counted_repeat` (`loop_shape.rs:136`) only matches numeric ranges
(`Repeat for v from a to b`). The collection form `Repeat for v in coll`
(`ast/stmt.rs:214`) is handled directly: trip = `length(coll)`, resolved via
`length_def` (`abstract_interp.rs:1698`) for locally-built collections or
`expr_len_range` / parameter identity otherwise. Map iteration → map length.

### Nesting and calls

`loop_cost(loop) = trip_count(loop) * body_cost(loop.body)` via `CostExpr::mul`.
Nested loops compose by multiplication, yielding `O(n^2)` / `O(n^3)` naturally. A
non-recursive call contributes the memoized callee cost with `subst` binding callee
params to caller argument sizes. `Break` on a guard the prover shows fires within
`f(n)` iterations may lower the trip (optional precision; default ignores `Break`,
which is sound as an upper bound).

## 6. Recurrence analysis

Driven by `CallGraph.sccs` (`callgraph.rs:18`) and `is_recursive` (`callgraph.rs:79`),
processed callees-first with memoization.

### Self-recursion → recurrence extraction

For a function `f` that is its own singleton SCC with a self-edge, walk the body once:

- **`a` = recursive call count** on the worst-case path —
  `inline_recursive::count_self_calls` (`optimize/inline_recursive.rs:265`); for tree
  recursion this is the per-invocation fan-out (2 for `fib`'s `f(n-1)+f(n-2)`).
- **size-change** per recursive call, by diffing the call's size argument against the
  parameter, classified into:
  - `n − k` (k constant ≥ 1) → **subtractive** `T(n) = a·T(n−k) + g(n)`.
  - `n / b` (b ≥ 2) → **divide-and-conquer** `T(n) = a·T(n/b) + g(n)`.
  - a sliced argument (mergesort's two halves) where `prove(guards, n − 2·sub_size) ≥ 0`
    certifies the half-split → divide-and-conquer with `b = 2`.
  - anything else → `Unknown(UnrecognizedSizeChange)`.
- **`g(n)` = local non-recursive work** = body cost (§5 + §7) excluding the recursive
  call subtrees.

### Solver

- **Subtractive** `T(n) = a·T(n−k) + g(n)`:
  - `a = 1`: `T(n) = Σ g(n − i·k) = Θ(n · g)` (`a=1,g=O(1) ⇒ O(n)`;
    `a=1,g=O(n) ⇒ O(n^2)`).
  - `a ≥ 2`: `Θ(a^{n/k})` exponential (`fib`: `a=2,k=1 ⇒ O(2^n)`), built as an
    `ExpBase`.
- **Divide-and-conquer** `T(n) = a·T(n/b) + g(n)` → **Master theorem**, with
  `c = log_b a`:
  - `g ≺ n^c` ⇒ `Θ(n^c)`.
  - `g = Θ(n^c log^k n)` ⇒ `Θ(n^c log^{k+1} n)` (mergesort `a=b=2,c=1,g=Θ(n) ⇒
    Θ(n log n)`; binary search `a=1,b=2,c=0,g=Θ(1) ⇒ Θ(log n)`; naive recursive matmul
    `a=8,b=2,c=3 ⇒ Θ(n^3)`).
  - `g ≻ n^c` (regularity holds) ⇒ `Θ(g)`.
  - `g` vs `n^c` compared with the §4 comparator. `c = log_b a` may be irrational
    (Strassen `log_2 7`); keep exponents `Rational` and **widen to
    `Unknown(MasterTheoremGap)`** when the comparison is undecidable rather than
    rounding.

### Tail / accumulator recursion — recognized *first*

Before generic recursion, check `tail_call::direct_self_tail_args`
(`tail_call.rs:33`), `tail_pair_args` (`:50`), and `detect_accumulator_pattern`
(`:225`). A match means linear single-call recursion equivalent to a loop:
`T(n) = T(n−k) + O(1) ⇒ O(n)` time **and** `O(1)` stack (the language guarantees
constant-stack self-tail-calls). Missing this reports `O(n)` stack for functions the
language makes `O(1)` — so the order matters.

### Mutual recursion (SCC size > 1)

For an SCC of size > 1 (use `reachable_from`, `callgraph.rs:50`, to confirm calls stay
in the cycle), recognize only the case that reduces to a single shared `(a, b)` or
`(a, k)` across members and solve the combined recurrence. Heterogeneous size-changes
that don't reduce → `Unknown(MutualRecurrenceUnsupported)` uniformly for **every**
member of the SCC. A wrong vector-recurrence collapse is silently unsound, so we
decline it.

## 7. Space / memory cost

A second, independent `CostExpr` per function, computed in the same body walk with
different accumulation rules.

### Allocating nodes

- `Expr::New` (`ast/stmt.rs:782`): one object, `O(1)` unless a sized constructor.
- `Expr::WithCapacity { capacity }` (`ast/stmt.rs:813`): allocation of
  `cost_of(capacity)`, resolved like a loop limit (§5) — the explicit size signal.
- `Stmt::Push` (`ast/stmt.rs:307`) / `Stmt::Add` **inside a loop**: one element per
  iteration; executed `trip` times ⇒ collection grows to `O(trip)`. This is the key
  rule: a push per iteration of an `O(n)` loop ⇒ `O(n)` space.
- `Expr::List(items)`: `O(len(items))`, typically constant.

### Peak, not sum

Space is a **max over program points**, because scratch frees:

- **persistent** (escapes scope / is returned): accumulates additively along the path
  — detected via the existing escape analysis (`analysis/escape.rs`).
- **scratch** (local, dies at scope end): contributes to the peak while live; modeled
  as `Max` across sibling scopes and `Sum` across nested *live* scopes.

For the loop case: a `Push` to `C` in loop `L` sets `C`'s size to `trip_count(L)`;
`space(L) = Max(body scratch peak, size_growth(C))`.

### Recursion stack = recurrence *depth*, not call *count*

The stack term is the longest root-to-leaf recursion path, i.e. the recurrence depth:

- subtractive linear ⇒ depth `O(n/k) = O(n)` ⇒ stack `O(n)`.
- divide ⇒ depth `O(log n)` ⇒ stack `O(log n)` (mergesort: array `O(n)` dominates ⇒
  total space `O(n)`).
- tree recursion (`fib`) ⇒ depth `O(n)` ⇒ stack `O(n)` — **even though time is
  `O(2^n)`**. Using call count here would overestimate catastrophically.
- tail/accumulator (detected via `tail_call.rs`) ⇒ `O(1)` stack.

`space(f) = Max(persistent allocations, peak scratch, stack frames)`.

## 8. Honesty model

The `certified: bool` flag is true iff every dominant-path `prove` obligation was
discharged — the bound is LIA-certified to be an upper bound, not merely structurally
derived.

The engine returns a **precise bound** only when every contributing loop/recursion
produced a non-`Unknown` `CostExpr` *and* every soundness obligation discharged.

It returns **`Unknown(reason)`** with a specific, machine-readable cause:
`UnresolvedLimitSymbol(Symbol)`, `NonAffineLimit`, `RankingNotAffine`,
`MasterTheoremGap`, `UnrecognizedSizeChange`, `MutualRecurrenceUnsupported`,
`SubstNonMonomial`, `CalleeUnknown(Symbol)` (the reason chains through callers).

It returns **`Unbounded(reason)`** only when proven: `DataDependentWhile` (no counted
shape, no usable ranking variant — e.g. `while x != target: x = f(x)`), or a recursion
with no provable size decrease (the prover cannot show the argument strictly shrinks,
so termination — and thus any finite cost — is not established).

**No silent caps.** Anywhere the analysis would otherwise pick a number it emits
`Unknown`/`Unbounded` instead, and appends a `DroppedTerm` to `dropped` naming the
construct and the cause. This mirrors the existing fail-closed posture of
`affine::prove` and `lia::fourier_motzkin_unsat` (`lia.rs:588`).

## 9. Certification (no Z3)

The proof path turns a derived bound into a kernel-checkable claim. A polynomial bound
`∀n. T(n) ≤ c·n^k` is expressed as a `ProofExpr` (universally-quantified inequality
over the `logicaffeine_proof` arithmetic vocabulary) and discharged via the proof
engine — `BackwardChainer::prove` for structure plus the ring/LIA arithmetic oracle
(`logicaffeine_proof/src/arith.rs`) for the inequality — yielding a `DerivationTree`
the kernel type-checks. No Z3 sits in this path; Z3 appears only as a *differential
test oracle* under the `verification` feature (§14).

What the kernel certifies **today**: linear and polynomial bounds, and
structural-recursion recurrences (via `StructuralInduction` over `Nat`). What needs a
**thin new layer** (documented as future, §16): a Landau/asymptotic calculus so that
`O(log n)` and `O(2^n)` claims carry first-class certificates. Until that layer lands,
log and exponential bounds are reported as **derived** with a "certificate pending"
badge — the engine never claims a certificate it does not have.

## 10. Optimization delta

Because optimizations can *change complexity class* (e.g. `closed_form`
(`optimize/closed_form.rs`) collapsing an `O(n)` Gauss accumulator loop to `O(1)`),
the analyzer runs on both snapshots and diffs:

```rust
let facts_pre  = oracle_analyze_with(&stmts_unopt, interner); // compile.rs:818 vector
let pre  = analyze_complexity(&stmts_unopt, interner, &facts_pre);
let facts_post = oracle_analyze_with(&stmts_opt, interner);   // compile.rs:824 vector
let post = analyze_complexity(&stmts_opt, interner, &facts_post);
// report.opt_delta[f] = (pre.bigo, post.bigo)
```

To attribute a class change to a *specific* pass, wrap the optimizer in the existing
firing trace — `begin_fired_trace` (`optimize/mod.rs:107`), `mark_fired`
(`:126`), `end_fired_trace` (`:113`) — so a delta like `O(n) → O(1)` is annotated
`[closed_form fired]`. Each snapshot needs its **own** `OracleFacts` (arena-address
keying — §Pitfalls).

## 11. CLI surface

A new subcommand mirroring `largo opts` (`apps/logicaffeine_cli/src/cli.rs:258`):

```bash
largo complexity <file> [--function <name>] [--json]
```

Definition: a `Commands::Complexity { file, function, json }` variant alongside
`Commands::Opts` (`cli.rs:258`), a `cmd_complexity` handler modeled on `cmd_opts`
(`cli.rs:678`), and a new `compile::complexity_graph(source) -> ComplexityReport`
entry mirroring `compile::optimization_graph` (`cli.rs:686`).

Human-readable output, per function: time Big-O + exact count, space Big-O,
`certified` / `derived` badge, opt-delta, and the `unknown_reason` when imprecise:

```
fib(n):
  time   O(2^n)     [derived from T(n)=T(n-1)+T(n-2)+O(1); certificate pending]
  space  O(n)       stack depth (tree recursion); O(1) heap
  after opts: unchanged
sum_to(n):
  time   O(n)       exact: n          ✓ certified
  space  O(1)                          ✓ certified
  after opts: O(1)  [closed_form fired: Gauss n(n+1)/2]
```

JSON shape parallels the `{fired, blockers, dependencies}` form, e.g.
`{"functions":[{"name":"fib","time":"O(2^n)","space":"O(n)","certified":false,
"reason":"CertificatePending","opt_delta":["O(2^n)","O(2^n)"]}, ...]}`.

## 12. Web complexity-flamegraph

The Studio already runs the compiler **in-WASM**, calling `logicaffeine_compile`
directly (no JSON round-trip), and renders trees in a hand-built CSS/SVG idiom
(`apps/logicaffeine_web/src/ui/components/ast_tree.rs`,
`.../components/proof_panel.rs`). The complexity surface mirrors that idiom:

- A new Studio panel calling `analyze_complexity` on the current buffer in-WASM.
- An **icicle/flamegraph**: vertical depth = loop/branch nesting, horizontal width =
  cost share of the parent, color = cost class (`O(1)` → `O(2^n)` ramp). Branch paths
  are rendered explicitly (each `If` arm a sibling), since the structural picture —
  *which path costs what* — is the point.
- Per-node badges: certified ✓ vs derived ~, and the Big-O label.
- A **pre/post-opt toggle** that swaps between the two snapshots from §10, so a user
  watches a subtree collapse when an optimization changes its class.

No new charting dependency; the rendering is Dioxus + CSS/SVG in the established style.

## 13. Empirical cross-check

The benchmark harness already declares Big-O strings (`benchmarks/run.sh`
`bench_complexity()`), records them into `benchmarks/results/latest.json`
(`complexity: {time, space}`), and empirically fits scaling exponents from hyperfine
runs across input sizes, surfaced on the benchmarks page. The cross-check:

- For each benchmark, compare the analyzer's static Big-O against the measured
  exponent. Consistent (`O(n^2)` vs exponent ≈ 2.0) → ✓; mismatch (`O(n log n)` vs
  exponent ≈ 1.0, or static `O(n^2)` vs measured ≈ 3) → ⚠ with a "recheck model" flag.
- **Replace** the hardcoded `bench_complexity()` strings with the synthesized bounds,
  validated against the curve — the declared complexity becomes *derived* and
  *checked*, not hand-maintained.

This is the soundness conscience for the static layer: a static bound that grows slower
than the measured operation count is an unsound underestimate and must fail loudly.

## 14. Testing strategy

TDD, robust to absurdity — the corpus is the spec.

**Known-complexity corpus** (`tests/complexity/`), each a golden assertion on
`(time_bigo, space_bigo, certified)`:

| Program | time | space | exercises |
|---|---|---|---|
| linear scan / sum | `O(n)` | `O(1)` | counted loop, `length(param)` resolution |
| nested double loop | `O(n^2)` | `O(1)` | nesting via `mul` |
| triple-loop matmul | `O(n^3)` | `O(n^2)` | nesting + persistent alloc |
| binary search | `O(log n)` | `O(1)` | geometric induction (`MulByTwo` shape) |
| mergesort | `O(n log n)` | `O(n)` | Master theorem, `prove`-certified half-split, stack `O(log n)` + array `O(n)` |
| fib (tree) | `O(2^n)` | `O(n)` stack | tree recursion, depth≠count trap |
| factorial (tail) | `O(n)` | `O(1)` | `detect_accumulator_pattern` ⇒ O(1) stack |
| BFS | `O(V+E)` | `O(V)` | incomparable-atom `Sum` |
| data-dependent `while` | `Unbounded(DataDependentWhile)` | — | honesty leaf |
| `while` w/ `decreasing` | `O(n)` | — | ranking-function path |

**Golden assertions**: on the rendered `BigO` for the headline, plus a structural
assertion on the normalized `CostExpr` for exact-count cases (Gauss ⇒ monomial set
`{½n², ½n}`). For pre/post, assert the *delta* (`closed_form`: `pre=O(n), post=O(1)`).

**Soundness / never-underestimate**: for each corpus program, instrument the existing
VM to count actual operations at `n ∈ {16,32,64,128,256}`, fit the growth, and assert
the static `big_o` **dominates** the measured curve. A single static bound growing
slower than the measured count is unsound and fails — the project's differential-test
culture.

**LIA obligation certification**: under the `verification` feature, every upper-bound
`prove` obligation the engine discharges is re-confirmed by Z3 (the existing
FM-vs-Z3 differential harness), making "sound via the kernel LIA" machine-checked
end to end.

**Unknown is explicit**: adversarial programs (`i < m*k`, data-dependent loop,
unsupported mutual recursion) each assert the *specific* `Unknown`/`Unbounded` reason
variant — not merely "imprecise" — so the analyzer cannot regress into guessing.

**Normalization canonicity**: property tests that `normalize` is idempotent and
order-insensitive (`normalize(a+b) == normalize(b+a)`,
`normalize(normalize(x)) == normalize(x)`) over random `CostExpr`s.

## 15. Wave plan

Five waves, in order. One **prerequisite** gates the foundation.

- **Prerequisite — expose symbolic bounds from `OracleFacts`.** The symbolic
  `scalar_upper` / `length_def` / `scalar_lower` maps live in the oracle's internal
  working state (`abstract_interp.rs:1718`, `:1698`, `:1737`) and are dropped when the
  state is consumed; the public `OracleFacts` exposes only concrete, arena-keyed
  facts. Add a recorder + accessor
  `OracleFacts::symbolic_upper(&Expr) -> Option<&LinExpr>` (and `symbolic_len`),
  populated at the loop-limit / size-defining occurrences, reusing the existing
  path-merge join semantics. *Fallback for the foundation wave:* `loops.rs` builds a
  self-contained local size-environment from `Let x be E` / `Let x be length(arr)`
  definitions via `lin_of`, swapped for the exposed accessor later.

- **Wave 1 — cost engine.** `cost.rs` (`CostExpr`, normalize, comparator, `big_o`),
  `loops.rs` (trip counts), `recur.rs` (recurrences + Master theorem), `space.rs`,
  `report.rs`; the corpus and soundness tests (§14). Sound by construction; `certified`
  reflects discharged obligations.

- **Wave 2 — `largo complexity` CLI** (§11) + `compile::complexity_graph`.

- **Wave 3 — kernel-certified bounds** (§9): polynomial/linear and
  structural-recursion certificates; the differential Z3 re-check.

- **Wave 4 — Studio complexity-flamegraph** (§12), in-WASM, pre/post-opt toggle.

- **Wave 5 — static-vs-measured cross-check** (§13): wire to the benchmark scaling
  fits; replace the hardcoded `bench_complexity()` strings.

## 16. Open questions & future work

- **Real-time / structural tracing.** An instrumented mode that counts operations per
  region at runtime and overlays the measured shares on the static flamegraph (the
  owner's "maybe cool too").
- **Landau certification layer** (§9): first-class kernel certificates for `O(log n)`
  and `O(b^n)`, retiring the "certificate pending" badge.
- **Cross-variable relation hints**: let the user (or the analyzer) supply `|E| ≤ |V|^2`
  so the comparator can order otherwise-incomparable terms, rather than keeping the
  irreducible `Sum`.
- **Amortized analysis**: potential-function reasoning for structures whose per-op cost
  is uneven (dynamic-array growth, union-find).

## 17. Grounding appendix

Every reused component, with its real location:

| Concern | Path / anchor |
|---|---|
| Counted-loop shape, extractors, const-eval | `crates/logicaffeine_compile/src/loop_shape.rs:29` (`CountedLoop`), `:45` (`extract_counted_while`), `:136` (`extract_counted_repeat`, numeric ranges only), `:159` (`const_eval_i64`) |
| Sound LIA bridge | `crates/logicaffeine_compile/src/optimize/affine.rs:92` (`prove`), `:102` (`lin_of`) |
| Fourier-Motzkin + exact arithmetic | `crates/logicaffeine_kernel/src/lia.rs:48` (`Rational`), `:220` (`LinearExpr`), `:374` (`Constraint`), `:588` (`fourier_motzkin_unsat`) |
| Symbolic bounds (to be exposed) | `crates/logicaffeine_compile/src/optimize/abstract_interp.rs:1698` (`length_def`), `:1718` (`scalar_upper`), `:1737` (`scalar_lower`); `oracle_analyze_with` used at `compile.rs:532` |
| Recursion structure | `crates/logicaffeine_compile/src/analysis/callgraph.rs:12` (`CallGraph`), `:18` (`sccs`), `:50` (`reachable_from`), `:79` (`is_recursive`) |
| Recursion-shape classifiers | `crates/logicaffeine_compile/src/tail_call.rs:33` (`direct_self_tail_args`), `:50` (`tail_pair_args`), `:225` (`detect_accumulator_pattern`); `optimize/inline_recursive.rs:265` (`count_self_calls`) |
| Closed-form recognizers | `crates/logicaffeine_compile/src/optimize/closed_form.rs:13` (`SumOfCounter`), `:109` (`MulByTwo`), `:135` (`find_init_value`, constant bounds only), `:174` (`build_formula`) |
| AST nodes | `crates/logicaffeine_language/src/ast/stmt.rs:206` (`While` + `decreasing` at `:210`), `:214` (`Repeat`), `:307` (`Push`), `:782` (`New`), `:813` (`WithCapacity`) |
| Optimizer driver + fired-trace | `crates/logicaffeine_compile/src/optimize/mod.rs:505` (`optimize_program`), `:107`/`:113`/`:126` (`begin`/`end`/`mark` fired trace) |
| Integration point | `crates/logicaffeine_compile/src/compile.rs:818` (un-optimized `stmts`), `:824` (`optimize_program` → optimized) |
| CLI template | `apps/logicaffeine_cli/src/cli.rs:258` (`Commands::Opts`), `:678` (`cmd_opts`), `:686` (`optimization_graph`) |
| Certification | `crates/logicaffeine_proof/` (`ProofExpr`, `BackwardChainer`, `arith.rs` ring/LIA oracle, `StructuralInduction`) |
| Web rendering idiom | `apps/logicaffeine_web/src/ui/components/ast_tree.rs`, `.../proof_panel.rs`, `.../pages/studio.rs` |
| Empirical cross-check | `benchmarks/run.sh` (`bench_complexity`), `benchmarks/results/latest.json`, `apps/logicaffeine_web/src/ui/pages/benchmarks.rs` |
