# Potential Optimizations — What We Have, What We're Missing, What It's Worth

Companion to [BENCHMARK_IDEAS.md](BENCHMARK_IDEAS.md). That document asks
"what should we measure?"; this one asks "what should we build?" — grounded
in three sources:

1. **What largo already does** — read from
   `crates/logicaffeine_compile/src/optimize/` and `src/codegen/`.
2. **What LLVM does for us** (and can't do for us) — the class taxonomy in
   BENCHMARK_IDEAS §1.
3. **Where we actually lose** — `results/local-logos-vs-c.json`
   (2026-06-12, commit 4d31c18, i9-14900K, gcc 13.3.0, 10 runs).

The headline finding, stated up front because every priority below follows
from it: **our AST/codegen optimizer is already sophisticated — the losses
are dominated by the runtime representation.** `matrix_mult` is the proof:
largo emits a 32×32 *tiled, ikj-reordered* loop nest (OPT-TILE — an
optimization gcc/clang don't even do at -O2), and we still lose 1.54x,
because the innermost statement performs three `borrow()` calls, one
`borrow_mut()`, and four bounds checks per multiply-add. The optimizer won
the algorithm and the representation gave it back.

## 1. Inventory — what we have today

### 1a. AST optimizer (`optimize/mod.rs` pipeline, in order)

| Pass | File | What it does |
|------|------|--------------|
| Fold | `fold.rs` | Constant folding, algebraic identities, double-negation, `x * 2^k → x << k`; deliberately refuses NaN-unsafe identities on untyped identifiers |
| Propagate | `propagate.rs` | Constant propagation through immutable `Let`s |
| Partial eval (×16 fixpoint with fold+propagate) | `partial_eval.rs`, `bta.rs` | Polyvariant specialization with binding-time analysis; the Futamura-projection machinery |
| CTFE | `ctfe.rs` | Compile-time evaluation of pure calls, step-limited |
| GVN/CSE | `gvn.rs` | Structural common-subexpression elimination |
| LICM | `licm.rs` | Hoists invariant immutable `Let`s out of loops; conservative — no calls, no allocs, bails on anything effectful |
| Closed-form | `closed_form.rs` | `sum += i` → Gauss, `count += 1` → trip count, `x *= 2` patterns — three patterns exactly |
| Deforestation | `deforest.rs` | Producer-consumer loop fusion over intermediate Seqs, up to 4 chained stages |
| Abstract interpretation | `abstract_interp.rs` (2.4k lines) | Interval domain with ±∞ bounds; dead-branch elimination; feeds range facts |
| DCE | `dce.rs` | Dead store/code elimination |
| Supercompile | `supercompile.rs` | Inline+propagate+fold driver with homeomorphic-embedding termination |

### 1b. Codegen layer (`codegen/peephole.rs`, `tce.rs`, `program.rs`)

This layer is *shape-aware Rust emission* — its job is to hand LLVM code it
can chew:

- **OPT-1a/1b** — `While` + counter → `for` ranges (unlocks LLVM trip-count
  analysis, unrolling, vectorization).
- **OPT-TILE** — triple-nested loop tiling (fires on matrix_mult: 32-wide
  tiles, ikj order).
- **OPT-8** — zero-based counter normalization (eliminates `-1` index
  arithmetic from 1-indexed surface code).
- **OPT-4** — `assert_unchecked(bound <= seq.len())` hints in loop
  preheaders so LLVM can erase bounds checks (verified working:
  `_logos_main` in `prefix_sum_logos.ll` contains **zero**
  `panic_bounds_check` — all 117 whole-file hits are stdlib).
- **OPT-7** — counter-overwrite fusion.
- **`with_capacity` inference** — proves deterministic push counts and
  pre-sizes collections.
- **TCE** (`tce.rs`) — self- and nested-tail-call → `loop`/`continue`
  (verified: ackermann compiles to a loop with one residual recursive call).
- **Auto-memoization** (`program.rs`) — purity analysis + thread-local
  `FxHashMap` memo tables (verified: fib runs in 2.7ms vs C's 415ms).
- **Slice idioms** — tail copies become `extend_from_slice` (seen in
  mergesort's merge).

### 1c. What LLVM then does for us

Verified in the dumps: magic-number division for `% 1000000007` with 8×
unroll (`loop_sum_logos.s` — *more* aggressive than clang's 4×),
vectorization with `llvm.vector.reduce` (spectral_norm), `llvm.memset`
formation (array_fill), full bounds-check elimination where OPT-4's hints
reach (prefix_sum). Inlining, GVN at the IR level, instcombine — all free.

**Division of labor that emerges:** largo should never compete with LLVM on
B1/B2/C1-class work (it demonstrably arrives), and should spend all its
effort on what LLVM *cannot see through*: the `Rc<RefCell<Vec>>` wrapper,
allocation strategy, and semantic facts (purity, ranges, aliasing of
LogosSeq handles) that die in translation to Rust.

## 2. Scoreboard — where we lose, and why (local-logos-vs-c.json)

Losses, worst first, each diagnosed from `generated/<b>.rs` + dumps:

| Benchmark | Ratio | Diagnosis |
|-----------|-------|-----------|
| knapsack | **4.56x** | Inner DP loop: `curr.borrow_mut()[w] = prev.borrow()[w]` then up to 3 more borrows + checked indexing per iteration. Plus a **hint bug**: loop runs `w in 0..(capacity+1)` (max index `capacity`) but OPT-4 emits `assert_unchecked(capacity <= prev.len())` — off by one, so the hint cannot license check removal. RefCell traffic also blocks vectorization of the max-fold. |
| nbody | **3.70x** | 5 bodies in parallel Seqs; the force loop does ~15 `borrow()` calls per (i,j) pair. C keeps a 5-struct array in registers/L1. The sizes are compile-time constants — this could scalarize entirely. |
| ackermann | **2.29x** | TCE works (loop + one residual call). Open diagnosis: gcc's call-heavy codegen vs ours needs an asm diff; candidates are the residual call's frame setup and gcc's deeper recursion-to-iteration conversion. **Action: diff `ackermann_{c,logos}.s` hot frames before building anything.** |
| graph_bfs | 1.74x | Per-access borrows in the BFS relaxation; `queue.len()` re-evaluated (borrow + load) every iteration of the drain loop. |
| mergesort | 1.72x | Allocation churn: every recursion level does `to_vec()` slice copies + fresh result Vec; C merges into one preallocated scratch buffer. Push-based merge also pays a capacity check per element. |
| matrix_mult | 1.54x | OPT-TILE fires (tiled ikj!) but the inner statement performs 3× `borrow()` + 1× `borrow_mut()` + 4 bounds checks per FMA-shaped op. The representation tax eats the tiling win. |
| string_search | 1.50x | Per-character compare does two checked `as_bytes()[..]` accesses; OPT-4 hints the wrong bound (`needleLen <= text.len()`, but the access index reaches `textLen-1`). No `bcmp` idiom forms. |
| fannkuch | 1.38x | Dense small-array permutation shuffling through borrows. |
| two_sum | 1.29x | LogosMap (FxHashMap) vs C's hand-rolled open-addressing table; per-op borrow on the map handle. |
| counting_sort | 1.21x | Scatter increments through `borrow_mut()` per element. |
| loop_sum | 1.19x | **Not a missing optimization**: rustc/LLVM emits the same magic-number mod, unrolled 8× vs clang's 4×. The gap is gcc 13's scheduling of the serial mod dependence chain. The *largo-level* counter exists though — see Mod deferral (§3, O6). |
| quicksort | 1.11x | Borrow + checks in partition; close enough that O1 likely closes it. |

And two "wins" that are really measure-integrity findings:

- **fib 0.01x, binary_trees ~0.00x** — auto-memoization changed the
  complexity class. Legitimate compiler behavior, but those two benchmarks
  no longer measure call overhead — they measure a hash lookup. This is
  exactly the scaling-exponent tripwire case from BENCHMARK_IDEAS §5.4;
  `tak` (idea 41) and memo-resistant variants restore the measurement.
  The C-side comparisons on those two rows should be read as "largo found
  an algorithmic win," not "largo is 415x faster at recursion."

The pattern across every real loss: **RefCell borrow traffic + residual
bounds checks + allocation churn** — D1/D3/D5 in the taxonomy. Not one loss
is caused by LLVM missing a classical optimization on clean input.

## 3. Potential optimizations

Ordered within each tier by expected geomean impact. Each entry: what ·
where it lives · which losses it addresses · which benchmark (existing or
BENCHMARK_IDEAS candidate) measures it · risk.

### Tier 1 — Representation: stop paying the Rc<RefCell> tax

**O1. Borrow hoisting (scoped slice extraction).**
When a loop body indexes Seqs without pushing/popping/rebinding them, emit
the borrow once in the preheader and index plain slices inside:

```rust
{
    let prev_s = &*prev.borrow();
    let curr_s = &mut *curr.borrow_mut();
    for w in 0..=capacity { curr_s[w] = ...prev_s[w]...; }
}
```

One RefCell op per loop instead of per access — and LLVM receives `&[i64]`
/ `&mut [i64]` with **noalias**, which unlocks vectorization, GVN across
iterations, and D2-class wins over C. Requires alias analysis: LogosSeq has
reference semantics (`Let b be a.` aliases), so two handles may only be
simultaneously hoisted if at most one is `&mut` *or* they're proven
distinct (distinct fresh allocations that never flow into each other — the
abstract interpreter is the natural home for this fact). Fall back to
per-access borrows when aliasing is possible.
· Codegen + abstract_interp. · Addresses: knapsack, nbody, matrix_mult,
graph_bfs, fannkuch, counting_sort, quicksort — i.e. nearly every loss.
· Measured by: the whole loss table, plus `bc_sequential`, `dot_product`,
`saxpy`, `noalias_inc` from BENCHMARK_IDEAS. · Risk: alias soundness —
needs the absurdly-robust-test treatment (aliased handles, conditional
aliasing, handles passed to functions, handles captured then mutated).

**O2. Escape-analysis ownership inference (de-Rc).**
The deeper fix: a Seq created in a function that never escapes (not
returned, not stored into another structure, not captured, not passed to
an opaque call) doesn't need `Rc<RefCell<...>>` at all — emit `Vec<i64>`
and lend `&`/`&mut` per use. Every hot Seq in knapsack, nbody, matrix_mult,
graph_bfs, and the sorts qualifies. O1 becomes mostly redundant where O2
applies, but O1 is simpler and catches escaping Seqs too — build O1 first,
keep both.
· New analysis pass + codegen type policy. · Measured by: same set as O1,
plus `alloc_churn`, `seq_of_seqs`. · Risk: the type split (`Vec` vs
`LogosSeq`) ripples through codegen signatures; functions taking Seq
parameters need a borrowed-slice calling convention (see O4).

**O3. Small fixed-size Seq scalarization (largo-SROA).**
A Seq with compile-time-known constant size that's never resized (nbody's
5 bodies) becomes a fixed array `[f64; 5]` — or, aggressively, individual
locals. This is D1 done at the level LLVM can't reach (LLVM cannot SROA
through `Rc`).
· Codegen, gated on abstract_interp shape facts. · Addresses: nbody 3.70x
almost entirely. · Measured by: nbody; `complex_mul`, `sqrt_norm` ideas.
· Risk: low — narrow precondition, big payoff.

**O4. Slice-borrowing calling convention for Seq parameters.**
mergeSort already receives `&[i64]` (the codegen does this today for some
shapes) but then immediately `to_vec()`s. Generalize: pure readers take
`&[T]`, writers take `&mut [T]`, only escape/store forces the handle.
· Codegen signature policy. · Addresses: mergesort, any `## To` helper in
hot loops. · Measured by: mergesort; `inline_ladder`.

### Tier 2 — Bounds checks: finish what OPT-4 started

**O5. Interval-driven hint generalization (and the off-by-one fix).**
Two concrete defects found in this investigation, then the general version:
(a) inclusive loops emit `assert_unchecked(bound <= len)` where the max
index *equals* `bound` — must be `bound < len` (knapsack); (b) hints only
cover counter-indexed arrays, so window/offset accesses (`text[i+j-1]` in
string_search, `prev[w-wi]` in knapsack) get nothing. The general version:
abstract_interp already computes the interval of every index expression —
emit one `assert_unchecked(max_interval < len)` per (array, loop) pair for
*any* index whose interval is loop-derivable.
· abstract_interp → codegen plumbing. · Addresses: knapsack, string_search,
stencil-shaped code. · Measured by: `bc_sequential`/`bc_window` (the
pass/fail grep is `panic_bounds_check == 0` in `_logos_main`), knapsack
ratio. · Risk: an unsound hint is UB — every emitted hint needs a paired
debug-build `assert!` mode and adversarial tests.

### Tier 3 — Allocation: churn and reuse

**O6. Scratch-buffer reuse for divide-and-conquer.**
mergesort's `to_vec()`-per-level is the textbook case: detect
recursive-helper + fresh-buffer patterns and rewrite to a caller-allocated
scratch buffer threaded through the recursion (or, simpler first step:
in-place merge into a preallocated output with explicit ranges).
· Deforestation-family AST rewrite. · Addresses: mergesort 1.72x.
· Measured by: mergesort; `alloc_churn`, `grow_push`. · Risk: medium —
transformation is intricate; supercompiler infrastructure may host it.

**O7. Allocation sinking/hoisting.**
A Seq allocated inside a loop, fully consumed within the iteration, gets
hoisted and `clear()`ed instead of reallocated.
· AST pass. · Measured by: `alloc_churn` (idea 51). · Risk: low.

### Tier 4 — Semantic arithmetic (things LLVM will never do)

**O8. Modulus deferral via interval analysis.**
`sum = (sum + x) % p` in a loop: when intervals prove `k` additions fit in
i64 before overflow (loop_sum: ~18 iterations of values < 2³¹ against
p ≈ 2³⁰), apply `% p` every `k` iterations. LLVM cannot reassociate `%`;
gcc cannot either — this beats *both* baselines, converts our only
"clean-code" loss (loop_sum 1.19x) into a likely win, and is a pure
AST-level rewrite sitting directly on infrastructure we already have
(abstract_interp intervals + closed_form's loop-pattern matching).
· closed_form.rs sibling pattern. · Measured by: loop_sum, fib_iterative,
`mod_weighted_sum`, with the scaling tripwire confirming O(n) is preserved.
· Risk: low; the arithmetic argument is mechanical and testable to
absurdity.

**O9. Loop-invariant divisor strength reduction (runtime libdivide).**
`x % m` / `x / m` where `m` is loop-invariant but runtime (histogram's
`% n`-style patterns): precompute the magic reciprocal once in the
preheader. Neither gcc nor clang can do this (divisor unknown at compile
time) — another both-baselines beat.
· Codegen preheader emission + a small runtime helper in logicaffeine_data.
· Measured by: `licm_div` (idea 2), `base_mix`. · Risk: low-medium
(correctness of the reciprocal algorithm; well-trodden territory).

**O10. Closed-form generalization — only behind the honesty rules.**
Extending Gauss to `sum += i*i`, `sum += c`, affine recurrences is cheap
wins on synthetic code, but per BENCHMARK_IDEAS anti-gaming rules each new
pattern must fire on ≥2 structurally distinct programs and survive the
scaling tripwire. Lower priority than everything above because it optimizes
benchmarks more than programs.

### Tier 5 — Diagnosis-first items (measure before building)

**O11. ackermann (2.29x)** — diff the hot frames in
`asm/ackermann_{c,logos}.s`. Hypotheses: residual-call frame size, gcc's
specific iteration conversion of the m-loop, or the dispatch-into-TCE-loop
shape confusing LLVM's branch layout. No optimization should be designed
until the asm says which.

**O12. two_sum (1.29x)** — LogosMap is FxHashMap; C is a hand-rolled
open-addressing table per the fairness rules. Decide: either a specialized
open-addressing i64→i64 map in logicaffeine_data (real engineering, helps
all Map workloads: two_sum, collect, `word_freq`), or accept the gap as a
stdlib-vs-bespoke delta and document it. `hash_open_addr` (idea 75) makes
the comparison apples-to-apples either way.

**O13. Memoization policy note** — keep the auto-memoizer (it's a
language feature, and a spectacular one), but: (a) add the
BENCHMARK_IDEAS scaling-exponent metadata so complexity-class changes are
*visible* rather than silent; (b) add `tak`/memo-resistant recursion
benchmarks so call overhead stays measured; (c) consider a memo-table size
policy for unbounded-domain functions.

## 4. Optimization ↔ benchmark cross-reference

| Optimization | Existing benchmarks that move | BENCHMARK_IDEAS candidates that isolate it |
|--------------|-------------------------------|--------------------------------------------|
| O1 borrow hoisting | knapsack, matrix_mult, graph_bfs, fannkuch, counting_sort, quicksort | bc_sequential, dot_product, saxpy, stencil_3pt |
| O2 de-Rc escape analysis | all of the above + mergesort | alloc_churn, seq_of_seqs, noalias_inc |
| O3 small-Seq scalarization | nbody | complex_mul, sqrt_norm |
| O4 slice calling convention | mergesort | inline_ladder, licm_pure_call |
| O5 interval hints | knapsack, string_search | bc_sequential/bc_random/bc_window (the pass/fail greps) |
| O6 scratch reuse | mergesort | alloc_churn |
| O7 alloc sinking | — | alloc_churn, grow_push |
| O8 mod deferral | loop_sum, fib_iterative, prefix_sum | mod_weighted_sum, lcg_chain (control: must NOT move) |
| O9 runtime divisors | histogram-family | licm_div, base_mix |
| O10 closed-form ext. | (synthetic only) | guarded by scaling tripwire |
| O12 specialized map | two_sum, collect | word_freq, hash_open_addr, string_pool |

Control benchmarks matter as much as target benchmarks: `lcg_chain` and
`pointer_chase` must **not** move under any of these — if they do, an
optimization is leaking somewhere it shouldn't.

## 5. Suggested order of attack

1. **O5a — the hint off-by-one fix** (small, sound, immediate knapsack
   relief; a bug, not a feature).
2. **O11 — ackermann asm diff** (an afternoon of reading; informs whether
   a 2.29x loss is one weird trick or structural).
3. **O1 — borrow hoisting** (the single biggest lever on the geomean;
   touches every loss).
4. **O5 — interval-driven hints, general form** (rides on O1's loop
   analysis; finishes D3).
5. **O3 — small-Seq scalarization** (nbody's 3.70x is the second-worst
   loss and has the narrowest fix).
6. **O8 — mod deferral** (turns our only clean-code loss into a win; pure
   AST; great test story).
7. **O2 — escape analysis** (the deep fix; do it after O1 proves the alias
   analysis).
8. **O6/O7 — allocation work** (mergesort + churn class).
9. **O9, O12** as the diagnostics dictate.

Expected end state: of the twelve losses, ten trace to representation and
bounds (O1/O2/O3/O5) — the geomean should move from 0.97x decisively below
0.9x before any Tier-4 cleverness is needed, and the BENCHMARK_IDEAS
diagnostic tier gives every step a mechanical pass/fail check.

## 6. Appendix — every benchmark, one by one

All 32 from `local-logos-vs-c.json`, worst ratio first. Each entry: where we
come in, what the generated code actually does today, and what would make it
faster — including the ones we already win, because a win with known
headroom is still headroom. Ratios are LOGOS/C mean time at the calibrated
size (lower is better for us); O-numbers reference §3.

### Losing

**knapsack — 4.56x (C 550ms, LOGOS 2506ms, n=18284)**
Rolling-array DP. Today: the inner `w` loop does `curr.borrow_mut()[w] =
prev.borrow()[w]`, a conditional `prev.borrow()[w-wi]` read, a
`curr.borrow()[w]` re-read, and a second `borrow_mut()` write — five RefCell
flag operations per cell — and the OPT-4 hint is off by one for the
inclusive bound (`capacity <= len` where index reaches `capacity`), so the
bounds checks all survive. The `prev`/`curr` handle swap is already smart
(`std::mem::swap` on the Rc handles). Speedups: O5a (hint fix — immediate),
O1 (hoist `prev_s`/`curr_s` slices around the inner loop; the two never
alias because both are fresh local allocations), after which LLVM should
if-convert the max into `cmov`/`max` and possibly vectorize the
unconditional copy half. Expected: this collapses toward parity; it's pure
representation tax.

**nbody — 3.70x (C 498ms, LOGOS 1844ms, n=18.4M steps)**
5-body simulation in 7 parallel Seqs of f64. Today: the advance loop does
~15 `borrow()`s plus checked indexing per (i,j) pair, every step. The body
count is a compile-time constant and the Seqs are never resized. Speedups:
O3 (scalarize to `[f64; 5]` locals — C's exact representation) is the whole
fix; O1 is the halfway version. FP semantics unchanged (same adds/muls,
strict order preserved). Note for later: when flags ever enable FMA (§5.8
of BENCHMARK_IDEAS), this benchmark moves again on both sides.

**ackermann — 2.29x (C 153ms, LOGOS 351ms, n=12)**
Today: TCE already converts two of the three calls into `loop`/`continue`;
one true recursive call remains (`ackermann(m, n-1)` as an argument). The
generated Rust is structurally identical to the C. Speedups: none to build
yet — O11 says diff the asm first. Hypotheses to check: gcc may be
performing a deeper recursion-to-iteration transform on the `m`-pattern,
frame size of the residual call, or branch layout of the dispatch loop.
This is the one loss where the answer isn't already known.

**graph_bfs — 1.74x (C 506ms, LOGOS 882ms, n=5.4M nodes)**
CSR-style adjacency + BFS queue. Today: graph construction and the BFS
drain both go through per-access borrows (`adjStarts.borrow()[i]`,
`adj.borrow_mut()[start+cnt]`); the drain re-evaluates `queue.len()` (a
borrow + load) every iteration as its loop condition. Speedups: O1 across
construction and relaxation loops; hoist the front/len comparison into a
locally tracked length (the queue only grows — abstract_interp can prove
monotonicity); O2 makes all five Seqs plain Vecs since none escape. The
random-access pattern itself is cache-bound (D6) — expect convergence to
slightly above C, not below.

**mergesort — 1.72x (C 501ms, LOGOS 863ms, n=6.2M)**
Today: each recursion level allocates `left`/`right` via `to_vec()` slice
copies plus a fresh `result` Vec; the merge loop pushes element-by-element
with per-element `borrow()`s on both inputs (though tail runs already use
`extend_from_slice` — good). C sorts into one preallocated scratch buffer.
Speedups: O6 (scratch-buffer reuse — the textbook case), O1 for the merge
loop borrows, O4 so the recursive calls stay in slice-land instead of
re-wrapping into LogosSeq. Allocation is the dominant term; O6 alone should
take most of the gap.

**matrix_mult — 1.54x (C 503ms, LOGOS 776ms, n=965)**
Today: OPT-TILE emits a 32×32×32 tiled ikj nest — better loop structure
than the C side — but the inner statement performs 3 `borrow()`s, 1
`borrow_mut()`, and 4 checked indexes per multiply-add, plus a `% 1000000007`
per element. Speedups: O1 (hoist `a_s`, `b_s`, `c_s` around the tile body —
all three are distinct fresh allocations), O5 for the residual checks, then
O8 (defer the mod out of the k-loop: products are < p² … too big — but
deferring with i128 accumulation or k-chunked mod both work and intervals
prove which). With slices + deferred mod the inner loop becomes a clean
integer dot product LLVM can vectorize; this could flip to a win since we
*keep* the tiling advantage.

**string_search — 1.50x (C 506ms, LOGOS 761ms, n=462M chars)**
Naive substring scan. Today: the per-character compare does two checked
`as_bytes()[..]` reads, and the OPT-4 hint guards the wrong bound
(`needleLen <= text.len()`; the actual index reaches `textLen-1`).
Speedups: O5 (interval-derived hint on `i+j-1`), or better, the slice idiom
from §3-O1's family: emit the inner equality loop as
`&text.as_bytes()[i-1..i-1+needleLen] == needle.as_bytes()` and let LLVM
form `bcmp` (BENCHMARK_IDEAS A3/G). The `equal_ranges`/`kmp_search`
candidates measure both forms.

**fannkuch — 1.38x (C 138ms, LOGOS 190ms, n=10)**
Today: the permutation loop allocates a fresh `perm` via
`perm1.borrow()[..].to_vec()` **per permutation** (3.6M allocations at
n=10); flip and rotate loops go through per-access borrows. C copies into a
fixed stack array. Speedups: O7 (hoist `perm`, refill with
`copy_from_slice`), O1 for the flip loop, O2/O3 since all arrays are small,
fixed-size, non-escaping. Allocation is the headline; this should land
near parity from O7 alone.

**two_sum — 1.29x (C 504ms, LOGOS 650ms, n=39.8M)**
Today: LogosMap (FxHashMap under Rc<RefCell>) vs C's hand-rolled
open-addressing array; each `contains`/insert pays a borrow plus FxHashMap's
general-purpose probing. Speedups: O12 — either a specialized i64→i64
open-addressing map in logicaffeine_data, or accept and document the
stdlib-vs-bespoke delta. O1-style borrow hoisting on the map handle helps
marginally. The `hash_open_addr` candidate (BENCHMARK_IDEAS idea 75) makes
this comparison apples-to-apples by hand-rolling both sides.

**counting_sort — 1.21x (C 504ms, LOGOS 612ms, n=70.6M)**
Today: count phase does `arr.borrow()[i]` + `counts.borrow()[v]` +
`counts.borrow_mut()[v]` per element (the scatter index `v` is
data-dependent, so its bounds check is legitimately hard to erase — though
`v`'s interval is provably `[0,1000)` from the `% 1000`); the rebuild phase
pushes element-by-element. Speedups: O1 (slices for arr/counts — the
borrows, not the checks, are most of the tax), O5 using the modulo-derived
interval to license even the scatter check, and emitting the rebuild's
inner repeat-push as a `fill`-style idiom. Histogram (0.62x win, same
scatter shape but no rebuild) shows the count phase can win; the rebuild is
the difference.

**loop_sum — 1.19x (C 493ms, LOGOS 588ms, n=240M)**
Today: already optimal *code* — plain i64 `for`, and LLVM emits the same
magic-number `% 1000000007` as clang with 8× unroll. The gap is gcc 13's
scheduling of the serial mod chain (the timed C binary is gcc, not the
clang in our dumps). Speedups: O8 — defer the mod ~18 iterations (intervals
prove safety), turning one magic-mul chain per element into one per 18;
this beats both gcc and clang because neither may reassociate `%`. The
clean-code loss becomes a clean win.

**quicksort — 1.11x (C 505ms, LOGOS 561ms, n=6.5M)**
Today: already good — `qs` receives `&mut *arr.borrow_mut()` once (the
slice calling convention exists for helpers!), so partition runs on a plain
slice; remaining cost is checked indexing in partition plus the final
checksum loop's per-access borrow. Speedups: O5 (partition indices `lo..hi`
are interval-provable), O1 on the checksum loop. Small gap, small fixes.

**prefix_sum — 1.06x (C 500ms, LOGOS 529ms, n=86.6M)**
Today: bounds checks already eliminated (verified: zero
`panic_bounds_check` in `_logos_main`); the loop-carried dependence forbids
vectorization on both sides equally. Remaining: two `borrow()`s + one
`borrow_mut()` per element. Speedups: O1 — one `borrow_mut` slice around
the scan loop; that's the whole remaining 6%.

**sieve — 1.05x (C 492ms, LOGOS 517ms, n=77.5M)**
Today: helper takes `limit`, allocates `LogosSeq<bool>`, marks composites
through `flags.borrow_mut()[j]` per write. Speedups: O1/O2 (the flags Vec
never escapes `sieve()`), giving LLVM a `&mut [bool]` it can strength-reduce
into pointer arithmetic. The `bitset_sieve` candidate then measures the
bit-packed variant on both sides.

**array_fill — 1.05x (C 504ms, LOGOS 531ms, n=111.6M)**
Today: fill via `push` of a computed value (capacity pre-reserved), then a
mod-armored sum with per-access borrow. Speedups: O1 on the sum loop, O8
mod deferral; the push-fill could emit as `extend((0..n).map(...))` to
drop the per-push capacity check. All three are small; this is nearly
converged.

**heap_sort — 1.03x (C 505ms, LOGOS 521ms, n=4.8M)**
Today: `siftDown` already receives `&mut *arr.borrow_mut()` (slice
convention), swap fusion emits single-borrow swap blocks. Remaining: a
fresh `borrow_mut()` per while-iteration in the pop loop and checked
indexing inside siftDown (data-dependent child indices — hard checks).
Speedups: O1 around the pop loop; O5 can prove `child < end` from the loop
guard. Marginal by design — this one is nearly done.

**coins — 1.03x (C 504ms, LOGOS 520ms, n=50.2M)**
Today: classic DP, inner loop `dp.borrow()[j] + dp.borrow()[j-coin]` then
`borrow_mut()[j]` — three RefCell ops per cell; OPT-4's hint uses `n <= len`
for an inclusive `j..n+1` loop (same off-by-one family as knapsack).
Speedups: O5a + O1; identical prescription to knapsack, milder dose (fewer
borrows per cell). Should flip to a win.

### Winning — and what's still on the table

**mandelbrot — 0.98x (C 507ms, LOGOS 498ms, n=3756)**
Pure scalar f64 escape-time loop; LLVM if-converts the early-exit flag
trick well. Headroom: none worth chasing at the source level — the
`isInside`/`iter = 50` early-exit encoding could emit as a real `break` for
cleaner control flow, and SIMD-across-pixels is a (fast-math-free)
vectorization LLVM won't do with the data-dependent exit. Leave it; the
BENCHMARK_IDEAS `conditional_sum` candidate covers the pattern in isolable
form.

**nqueens — 0.98x (C 125ms, LOGOS 123ms, n=14)**
Bitboard backtracking compiles to clean shifts/masks; recursion is the
honest cost on both sides. Headroom: TCE doesn't apply (the call is summed,
not tail); only O11-style asm reading if we ever care. Effectively tied —
working as intended.

**fib_iterative — 0.96x (C 500ms, LOGOS 478ms, n=242M)**
Serial add-mod chain, already winning (LLVM's 8× unroll + magic mod beats
gcc here). Headroom: O8 mod deferral applies (defer across ~9 iterations
for pair-sums), widening the win for free alongside loop_sum.

**array_reverse — 0.86x (C 499ms, LOGOS 431ms, n=122M)**
Swap fusion emits one `borrow_mut()` block per swap and LLVM turns the
two-pointer loop into wide moves. Headroom: O1 would lift even that borrow
out of the loop; LLVM might then recognize the full reverse idiom. Already
a solid win.

**spectral_norm — 0.81x (C 503ms, LOGOS 408ms, n=4258)**
`mulAtav` receives `&*u.borrow()` slices (convention again) and LLVM
vectorizes — `llvm.vector.reduce` confirmed in the dump. We win because the
division-heavy inner kernel runs identically and our outer plumbing is
fine. Headroom: `tmp.clone()` per call is an Rc clone (cheap, but O4 could
pass slices throughout); marginal.

**gcd — 0.79x (C 496ms, LOGOS 392ms, n=6637)**
The `gcd` helper compiles to a tight `while` over `%` — LLVM's codegen for
the Euclid chain simply beats gcc's. Headroom: none needed; nothing largo
adds here.

**collect — 0.77x (C 513ms, LOGOS 394ms, n=11.9M)**
Map insert/lookup with `with_capacity` pre-sizing (the inference pass
earning its keep) — FxHashMap beats the C implementation's hand-rolled
table here. Headroom: O12's specialized map would widen it; borrow hoisting
on the map handle is the cheap version.

**pi_leibniz — 0.76x (C 498ms, LOGOS 378ms, n=729M)**
Serial FP reduction, unvectorizable without reassociation on both sides —
and LLVM schedules the divide chain better than gcc. Headroom: none legal
(fast-math is off by policy; the `kahan_sum` candidate guards exactly
this). Enjoy the win.

**collatz — 0.75x (C 496ms, LOGOS 374ms, n=3.6M)**
Pure scalar branchy loop; LLVM's `cmov`/shift codegen wins. Headroom: the
`k % 2 / k / 2` pair already strength-reduces; nothing to add.

**histogram — 0.62x (C 503ms, LOGOS 312ms, n=298M)**
LCG + scatter increments; despite per-access borrows we win big — LLVM
keeps the hot 1000-entry counts array access tight while gcc's codegen
lags. Headroom: O1 anyway (it's the same shape as counting_sort's count
phase, which loses — the difference between these two benchmarks is itself
diagnostic), and O5 can use the `% 1000` interval to erase the scatter
check on principle.

**primes — 0.62x (C 498ms, LOGOS 309ms, n=2.8M)**
Trial division, pure scalar; LLVM emits a better division-test loop than
gcc -O2. Headroom: none needed. (If we ever want sport: `d*d <= i` could
strength-reduce the multiply to an addition chain — LLVM already does.)

**strings — 0.53x (C 505ms, LOGOS 269ms, n=18M)**
Text building with the `with capacity n*6` hint honored, then a byte scan.
Rust's String machinery beats C's manual buffer code. Headroom: the scan
loop's `item i of result` goes through `as_bytes()` cleanly already; O1
n/a (String, not Seq). Done.

**bubble_sort — 0.50x (C 500ms, LOGOS 251ms, n=19756)**
The surprise star: swap fusion emits a single `borrow_mut()` block per
compare-swap, and LLVM if-converts the swap into branchless `cmov`s where
gcc keeps a branch — on random data that's the whole 2x. Headroom: O1 would
hoist the remaining per-iteration borrow; academic at this margin.
(`branch_sorted`/`branch_shuffled` from BENCHMARK_IDEAS will prove the
cmov story explicitly.)

### Complexity-class outliers (wins, with an asterisk)

**fib — 0.01x (C 415ms, LOGOS 2.7ms, n=43)**
Auto-memoization turned O(φⁿ) into O(n) hash lookups. Legitimate language
feature, not a codegen win — this row measures the memoizer's existence,
not speed. Keep, but per O13: add the scaling metadata and a
memo-resistant sibling (`tak`) so call overhead stays measured. Residual
real cost if we care: the thread-local + FxHashMap per call could become a
flat Vec memo for dense integer domains (intervals can prove density) —
that would speed up *memoized* programs generally.

**binary_trees — ~0.00x (C 294ms, LOGOS 0.7ms, n=23)**
Same story squared: `makeCheck` is allocation-free recursion on our side
and the memoizer/CTFE machinery collapses it; C builds actual trees. The
benchmark no longer compares like with like — BENCHMARK_IDEAS Group H
(linked_list_sum, bst_insert_walk) restores a real heap-structure
measurement. Until then, treat this row as a language-feature demo.

### Reading the appendix as a whole

Counting prescriptions: **O1 appears in 14 entries, O5 in 7, O2/O3 in 6,
allocation work (O6/O7) in 3, O8 in 3.** The five benchmarks where the
slice calling convention already exists for helpers (quicksort, heap_sort,
spectral_norm — all ≤1.11x) versus the five worst losses (all
per-access-borrow in `_logos_main` bodies) is the cleanest possible
natural experiment: where slices reach the hot loop we're at parity or
winning; where borrows reach it we lose 1.5–4.6x. O1/O2 is the suite
telling us, twelve different ways, what to build first.
