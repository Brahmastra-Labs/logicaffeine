# Benchmark Ideas — Making the Suite a Great Measure

The 32-benchmark suite is about to become the optimization target for largo.
Goodhart's law says a measure that becomes a target stops being a good
measure — *unless* the measure is comprehensive enough that gaming it is
indistinguishable from genuinely improving the compiler. This document is the
spec for getting there: a taxonomy of what LLVM actually does at `-O2`, a map
of which of those behaviors the current 32 benchmarks exercise, and ~75
candidate programs that fill the gaps, each with a mechanical pass/fail check
against the `asm/` dumps.

Everything here is grounded in this repo's own artifacts: the `.lg` sources in
`programs/`, the Rust largo emits into `generated/`, and the `DUMPS=1`
assembly/IR in `asm/`.

## 0. Ground rules — keeping the measure honest

Three observations from inspecting the current pipeline frame everything
below:

1. **Scalar LOGOS benchmarks already compile to C-shaped code.**
   `generated/loop_sum.rs` is a plain `for` over `i64`. The fight there is
   LLVM-vs-LLVM: clang answers `% 1000000007` with a magic-number multiply
   (`movabsq $0x89705F3112A28FE5`) and a 4× unroll (see `asm/loop_sum_c.s`).
2. **Seq benchmarks are the real battleground.** Every `item i of arr`
   becomes `arr.borrow()[(i-1) as usize]` — a RefCell borrow-flag check plus
   a bounds check per element. largo already emits
   `unsafe { assert_unchecked(n <= arr.len()) }` hints (see
   `generated/prefix_sum.rs`); whether LLVM can hoist the borrow, erase the
   checks, and vectorize *through* that wrapper is the single biggest
   determinant of our ratios.
3. **The dumps and the timings come from different compilers.** Timing
   baselines are `gcc -O2`; the `asm/*_c.{s,ll}` dumps are `clang -O2`
   (`run-logos-vs-c.sh:260` vs `:265`). gcc does not auto-vectorize at `-O2`;
   clang does. Neither build passes `-march=native`, so baseline x86-64 means
   **no FMA and no AVX on either side's timed binary**. Several "mysteries"
   in ratio diffs trace to exactly this. See §5.8.

### Anti-gaming rules (binding on largo development)

- **No shape-keying.** No largo transform may match a benchmark's exact
  program shape. Any new transform must demonstrably fire on at least two
  structurally distinct programs in the suite (and ideally on code outside
  it).
- **Checksums must carry the work.** Every benchmark prints one line whose
  value data-flows from every element it claims to process. If deforestation,
  partial evaluation, or closed-form recognition can legally delete the work
  *for all n*, the optimization is real and the win counts; if it can only do
  so because the checksum ignored part of the computation, the benchmark is
  broken — fix the benchmark.
- **Paired variants are tripwires.** Several candidates below come in pairs
  (eliminable/non-eliminable bounds checks, sorted/shuffled branches,
  naive/blocked traversal). A largo "win" that moves one half of a pair
  without the corresponding physical reason moving the other half is evidence
  of overfitting. Report intra-pair deltas, not just per-benchmark ratios.
- **Multiple sizes catch complexity cheating.** A compiler that turns O(n)
  into O(1) shows up instantly in the scaling exponent across `sizes.txt`.
  §5.4 makes this a CI check.

### Two tiers

Adding a benchmark today costs 11 implementations (README, "Adding a
benchmark"). Most of the candidates below are *diagnostic* — they exist to
isolate one LLVM behavior for the LOGOS-vs-C optimization loop, and forcing a
Ruby port of each would be pure overhead. Proposal:

- **Tier 1 (T1)** — full suite: all 11 languages, registered in `run.sh` /
  `run-quick.sh` / `run-logos-vs-c.sh`, CI-tracked, frontend-visible. Reserve
  for benchmarks with cross-language narrative value.
- **Tier 2 (T2)** — diagnostic tier: `main.c` + `main.lg` only, registered
  only in `run-logos-vs-c.sh` (e.g. a `DIAG_BENCHMARKS` array appended when
  `DIAG=1`), full `DUMPS=1` machinery, same sizing/verification conventions.
  Two files instead of eleven. A T2 benchmark that proves interesting gets
  promoted by writing the other nine implementations.

## 1. Taxonomy — what LLVM does at -O2, and what to grep for

Class IDs are referenced throughout the rest of the document.

| ID | Class | Key LLVM passes | Signatures in `.ll` / `.s` | LOGOS-specific risk/opportunity |
|----|-------|-----------------|----------------------------|---------------------------------|
| A1 | Loop-invariant code motion | `licm`, GVN | invariant loads/computation above the loop header; `.lcssa` phis | RefCell borrow + `len()` loads may not hoist out of loops |
| A2 | Induction variables, SCEV, strength reduction | `indvars`, `loop-reduce`, SCEV | closed-form replacement of exit values; `mul`→`add` recurrences | collides with largo's own Gauss/closed-form pass — benchmarks must be mod-armored |
| A3 | Loop idiom recognition | `loop-idiom` | `llvm.memset`, `llvm.memcpy`, `llvm.ctpop`, `bcmp` calls | bounds-check branches inside the loop body defeat idiom matching |
| A4 | Auto-vectorization + SLP | `loop-vectorize`, `slp-vectorizer` | `<4 x i64>` / `<2 x double>` types, `llvm.vector.reduce.*`; `xmm/ymm` arithmetic in `.s` | the vectorizer bails on loops with side-exits (panics); confirmed present in `spectral_norm_logos.ll` (12 `llvm.vector.reduce` hits) |
| A5 | If-conversion / branchless select | `simplifycfg`, instcombine | `select i1` in `.ll`; `cmov`, `maxsd/minsd` in `.s` | data-dependent `If` statements in surface LOGOS |
| A6 | Loop unswitching | `simple-loop-unswitch` | loop body duplicated under an invariant guard | invariant flags threaded through locals must be provably invariant after codegen |
| A7 | Unrolling, interleaving, dependence-chain breaking | `loop-unroll`, vectorizer interleaving | repeated loop bodies; multiple accumulator phis | does largo's emitted Rust permit accumulator splitting? (FP: no, without fast-math — see E2) |
| A8 | Cache-order effects (interchange/blocking) | mostly *not* done by LLVM at -O2 | stride pattern visible in addressing | a measurement-honesty class: both sides should suffer equally; deviation = per-access overhead |
| B1 | Division/modulo by constant | instcombine, DAGCombine | magic-constant `movabsq` + `imulq`/`shr` instead of `idiv` | constants must reach LLVM *as* constants through largo's pipeline |
| B2 | Integer instcombine, overflow semantics | `instcombine`, `reassociate` | `nsw`/`nuw` flags; fused expression trees | C exploits signed-overflow UB (`nsw`); Rust release wraps — a permanent, documentable semantic delta |
| B3 | Bit-manipulation idiom recognition | instcombine, DAGCombine | `llvm.ctpop`, `llvm.cttz`, `llvm.bswap`, `llvm.fshl`; `popcnt`, `tzcnt` in `.s` | LOGOS bit verbs exist (`shifted left by`, `xor`, `or` — see `nqueens/main.lg`) |
| C1 | Inlining | `inline` | small functions absent from `.ll`; no `call` in hot loops | every `## To` function is a real Rust `fn`; the inliner must see through largo's emission |
| C2 | Tail-call elimination, recursion→iteration | `tailcallelim` | recursive `call` replaced by `br` to entry block | LOGOS encourages recursion; accumulator-style recursion must become a loop |
| C3 | Switch lowering, jump tables | `simplifycfg` | `switch i64` in `.ll`; `.LJTI` tables + `jmpq *` in `.s` | LOGOS has only `If` chains — does LLVM reconstruct the switch? (this decides interpreter-class workloads) |
| D1 | SROA / scalar replacement | `sroa`, `mem2reg` | `alloca`s gone; struct fields living in registers | the `Rc<RefCell<Vec>>` box can never fully scalarize; measure how close LLVM gets |
| D2 | Alias analysis | BasicAA, TBAA, `noalias` | `noalias` attrs on parameters in `.ll`; loads kept in registers across stores | **Rust advantage**: `&mut` noalias lets LLVM cache values where C must reload — a class LOGOS should *win* |
| D3 | Bounds-check elimination | `indvars`, GVN, range analysis | count of `panic_bounds_check` callsites reachable from `_logos_main` | the headline LOGOS tax; `assert_unchecked` hints are the current mitigation |
| D4 | Dead store elimination, store-to-load forwarding | `dse`, GVN | first-pass stores deleted; loads folded into prior stores | stores through `borrow_mut()` may look escaped |
| D5 | Allocation behavior, refcount/drop glue | (runtime, not a pass) | `__rust_alloc/realloc/dealloc` calls inside loops; `memcpy` from Vec growth | Rc refcount traffic and drop glue in hot paths |
| D6 | Cache hierarchy & memory latency | n/a — hardware | diagnose with `perf stat`, not asm diffs | the language-neutral floor: LOGOS must tie C on pure pointer-chasing or the runtime taxes plain loads |
| E1 | FMA contraction | DAGCombine, `-ffp-contract` | `vfmadd*` in `.s`; `llvm.fmuladd` in `.ll` | **currently moot on x86-64**: no `-march=native`, so neither side has FMA (verified: 0 hits in both `nbody_*.s`). Becomes real on aarch64 (clang contracts to `fmadd`; rustc does not) and the moment flags change |
| E2 | FP reassociation / fast-math | `reassociate` (fast-math only) | accumulator phi count in vectorized FP loops | neither gcc, clang, nor rustc reassociates at plain -O2 — keep it that way and document it as a fairness invariant |
| E3 | libm calls and intrinsics | `simplify-libcalls` | `call double @sin` vs `llvm.sqrt.f64` | Rust routes some math through intrinsics, some through libm; which, per operation? |
| E4 | FP↔int conversion | DAGCombine | `cvttsd2si`, `roundsd` | LOGOS conversion verbs' lowering |
| F | Branch predictability | n/a — hardware | compare branch vs `cmov` codegen between sides; `perf stat` branch-misses | exposes whether the two compilers made different branch/branchless choices for identical sources |
| G | Strings / Text / Map runtime | mixed: `memchr`, `bcmp`, SIMD compare in stdlib | `memcmp`/`bcmp`/`memchr` calls | LOGOS `Text` vs `char*`; Map hashing vs hand-rolled C hash |

## 2. Coverage map — the existing 32 on the taxonomy

What each current benchmark *primarily* exercises (● primary, ◐ incidental):

| Benchmark | Primary classes | Notes |
|-----------|----------------|-------|
| loop_sum | A2● B1● | mod-armored induction sum; the strength-reduction showcase |
| fib_iterative | A2● B1◐ | serial dependence chain + mod |
| prefix_sum | A2● D3● | in-place sequential update; loop-carried dependence forbids vectorization |
| fib, ackermann | C1● C2◐ | non-tail recursion, call overhead |
| binary_trees | C1● C2◐ | **pure recursion in the LOGOS version — no node allocation**, so it does not measure heap data structures on our side |
| nqueens | C1● B3◐ | backtracking; bit ops incidental |
| fannkuch | D3● A7◐ | permutation shuffling, dense small-array traffic |
| bubble/heap/merge/quick/counting_sort | D3● F◐ D6◐ | data movement + branchy compares, entangled |
| coins, knapsack | D3● A1◐ | 2D-as-1D DP indexing |
| collatz, gcd | A5● F● | branchy scalar loops, unpredictable trip counts |
| sieve, primes | D3● A3◐ | byte-array flag setting |
| array_fill | A3● | memset-idiom candidate (LOGOS `.ll` shows `llvm.memset`; the clang C dump does not — computed values) |
| array_reverse | A3◐ D3● | reverse loop |
| histogram, counting_sort | D3● D6◐ | data-dependent scatter writes |
| two_sum, collect | G● D5◐ | Map insert/lookup at scale |
| graph_bfs | D6● D3◐ | irregular traversal |
| matrix_mult | A8● D3● | naive ijk order — cache-hostile by design |
| nbody, spectral_norm | A4◐ E1◐ E3◐ | FP kernels; spectral_norm's LOGOS side does vectorize (verified) but division dominates |
| mandelbrot | A5● E4◐ | escape-time inner loop with data-dependent exit |
| pi_leibniz | E2● | serial FP reduction — unvectorizable without reassociation, on both sides, by design |
| strings, string_search, collect | G● D5◐ | Text append + scan |
| loop classes A1, A6 | — | **never isolated** |

### Confirmed gaps (nothing in the 32 isolates these)

1. **A4 as a headline number** — no dot product, no saxpy, no vectorized
   reduction where the *entire* benchmark is "did it vectorize".
2. **E1 FMA** — invisible today (baseline x86-64), but present-and-unmeasured
   on aarch64 builds and the moment anyone adds `-march=native`. No benchmark
   makes it a number.
3. **B1 division by small constants** — `% 1000000007` is everywhere, but no
   hot `/10`, `%10` digit work where divmod-pairing and magic numbers
   dominate.
4. **C3 jump tables / dispatch** — nothing interpreter-shaped. Given EXODIA
   is itself a bytecode VM, "how well does our language run an interpreter"
   is arguably the most self-relevant workload we don't have.
5. **C2 tail recursion** — all existing recursion is non-tail.
6. **A1 LICM and A6 unswitching** — only ever incidental, never isolated.
7. **D2 noalias** — no benchmark where Rust's aliasing rules let LOGOS *beat*
   C. A great measure needs both directions.
8. **D3 as a pair** — bounds checks tax everything and are isolated by
   nothing; there is no eliminable/non-eliminable control pair.
9. **A3 beyond memset** — no copy-loop (memcpy), popcount-loop (ctpop),
   find-loop (memchr), or compare-loop (bcmp) idiom probe.
10. **D6 pure memory latency** — no pointer-chase; nothing separates
    "compiler lost" from "cache missed".
11. **F predictability pair** — sorts entangle branch prediction with data
    movement; no controlled sorted/shuffled pair.
12. **B3 bit idioms** — bit ops only appear inside nqueens backtracking.
13. **Heap data structures** — the LOGOS binary_trees is allocation-free;
    nothing measures building/walking linked structures.
14. **G algorithmic pairs** — string_search is naive scan only; no KMP, no
    word-frequency "real program" shape.

## 3. Candidate benchmarks

Format: **name** — algorithm · isolates · dump check · tier.

All candidates follow suite conventions: `n` from the first CLI argument,
inputs from the standard LCG (`seed = 42; seed = (seed*1103515245 + 12345) %
2147483648; value = (seed/65536) % 32768`), one printed checksum line that
data-flows from every element, integer accumulators armored with
`% 1000000007`, and structural identity across implementations.

### Group A — Loop optimizations

**A1 — LICM**

1. **licm_len** — inner loop bounded by `length of s` written so the length
   is re-queried each iteration in surface code; sum the elements.
   · Isolates: hoisting the RefCell-borrowed `len()` load — risk #1 from §0.
   · Check: exactly one length load above the loop in `_logos.ll`.
   · T2.
2. **licm_div** — `sum += a[i] / d` where `d` is computed from `n` before the
   loop. · Isolates: hoisting the invariant divisor and its magic-number
   setup. · Check: division setup outside the loop body. · T2.
3. **licm_addr** — 2D access `a[i*cols + j]` with the `i*cols` term invariant
   in the inner loop. · Isolates: address-computation hoisting + strength
   reduction working together. · Check: inner loop increments a pointer
   instead of re-multiplying. · T2.
4. **licm_pure_call** — invariant call to a small pure `## To` function
   inside a hot loop. · Isolates: inline-then-hoist through largo's function
   emission. · Check: no `call` in the loop; computation above the header.
   · T2.

**A2 — SCEV / strength reduction (mod-armored against largo's own
closed-form pass)**

5. **mod_weighted_sum** — `sum = (sum + i*w) % p` with `w` and prime `p`
   derived from input. · Isolates: induction arithmetic with no closed form
   available (p non-constant). · T2.
6. **lcg_chain** — iterate the suite's standard LCG n times, print the final
   state. · Isolates: nothing eliminable, a pure serial mul-add-mod chain —
   this is the **scalar latency floor**: any LOGOS-vs-C gap here is pure
   per-iteration codegen overhead. · Check: loop body is exactly the LCG ops.
   · T2, cheap and high-value.
7. **geom_mod** — iterated `x = x*3 % p`. · Isolates: strength reduction on
   the multiply; closed form requires modular number theory neither compiler
   has. · T2.
8. **triangle_addr** — fill a packed lower-triangular array via
   `k = i*(i-1)/2 + j`. · Isolates: SCEV on quadratic index expressions plus
   the `/2`. · T2.

**A3 — Loop idiom recognition**

9. **array_copy** — element-wise copy of one Seq into another, checksum the
   copy. · Isolates: memcpy idiom formation through bounds checks (array_fill
   covers memset-ish writes; nothing covers copies). · Check: `llvm.memcpy`
   in `_logos.ll`. · T2.
10. **array_zero_refill** — alternately zero and refill a window, k rounds.
    · Isolates: memset idiom + its interaction with DSE. · Check:
    `llvm.memset` in the hot region. · T2.
11. **popcount_words** — generate n words with the LCG, count set bits with
    the shift-and-mask loop. · Isolates: `llvm.ctpop` idiom recognition (the
    loop literally disappears into one instruction when it fires). · Check:
    `ctpop` in `.ll` / `popcnt` in `.s` — note baseline x86-64 lacks POPCNT,
    so expect the libcall/SWAR lowering; the `.ll` check is the real one.
    · **T1 candidate** — classic, portable to all 11 languages.
12. **find_sentinel** — repeatedly scan for the first element equal to a
    rotating sentinel. · Isolates: memchr-style early-exit loops — the
    vectorizer's hardest common case (side exits). · Check: vector compare +
    movemask, or a `memchr` call. · T2.
13. **equal_ranges** — compare two arrays for equality over many sliding
    windows. · Isolates: `bcmp`/`memcmp` formation. · T2.

**A4 — Auto-vectorization / SLP**

14. **dot_product** — `sum += a[i]*b[i]` over LCG-filled Real arrays,
    repeated k passes. · Isolates: THE vectorized reduction; also stacks
    D3 (two bounds checks per iteration) and E1. The single most glaring gap
    in the suite. · Check: `llvm.vector.reduce.fadd` in `_logos.ll`.
    · **T1, priority 1.**
15. **saxpy** — `y[i] = a*x[i] + y[i]`. · Isolates: vectorized streaming
    read-modify-write, and D2: Rust's noalias means LLVM needn't fear
    `x`/`y` overlap, while C without `restrict` must check or version the
    loop. A benchmark LOGOS can legitimately *win*. · Check: `noalias` attrs
    in `_logos.ll`; runtime overlap check in `_c.ll`. · **T1 candidate.**
16. **int_sum_squares_mod** — `sum += a[i]*a[i]`, mod-armored. · Isolates:
    integer vectorized reduction with the mod outside the SIMD-able part
    (accumulate in chunks, reduce mod per block — keep both sides
    structurally identical). · T2.
17. **stencil_3pt** — `b[i] = (a[i-1] + a[i] + a[i+1]) / 3`, k sweeps.
    · Isolates: vectorization with overlapping window loads and a 3-fold
    bounds-check cluster per iteration. · T2.
18. **conditional_sum** — sum elements above a CLI-derived threshold.
    · Isolates: masked/predicated vectorization (select-based reduction).
    Pairs with F-group variants below. · T2.
19. **minmax_scan** — one pass computing both min and max. · Isolates:
    multi-reduction vectorization. · Check: `llvm.vector.reduce.smin/smax`.
    · T2.
20. **slp_quad** — process elements in hand-unrolled blocks of 4 in the
    surface code. · Isolates: SLP (straight-line) vectorization, distinct
    from loop vectorization. · T2.

**A5 — If-conversion / branchless**

21. **clamp_stream** — clamp LCG values into `[lo, hi]`, sum. · Isolates:
    select/cmov formation from `If` chains. · Check: `select` in `.ll`, no
    conditional branch in the loop body in `.s`. · T2.
22. **abs_delta_sum** — sum `|a[i] - a[i-1]|`. · Isolates: the abs idiom
    (`llvm.abs`). · T2.
23. **count_matches** — count elements satisfying a compound predicate
    (`x % 3 == 0 and x > t`). · Isolates: branchless compare-and-add.
    Pairs with conditional_sum. · T2.

**A6 — Loop unswitching**

24. **unswitch_mode** — one loop whose body applies operation A or B
    depending on a flag parsed from the CLI. · Isolates: unswitching an
    invariant condition out of a hot loop. · Check: two specialized loop
    bodies in `.ll`. · T2.
25. **unswitch_nested** — same, with the invariant test buried under two
    loop levels. · T2.

**A7 — Unrolling / instruction-level parallelism**

26. **acc_split_sum** — the same integer reduction written twice: one
    accumulator vs four accumulators (two phases, checksums combined).
    · Isolates: whether the compiler breaks serial dependence chains itself
    (integers: legal; the 4-acc version shows the headroom). · Check: phi
    count in the vectorized loop. · T2.
27. **hash_fnv** — FNV-1a over an LCG-generated byte array. · Isolates: an
    *unbreakable* serial mul-xor chain — upper-bounds what unrolling can do,
    and exercises B2 wrapping semantics. · **T1 candidate** (classic,
    portable).

**A8 — Cache order (measurement-honesty class)**

28. **transpose_naive** — out-of-place matrix transpose, row-major read /
    column-major write. · Isolates: pure stride behavior; LLVM won't fix it
    at -O2, so both sides should tie — any deviation is per-access overhead.
    · T2.
29. **matmul_ikj** — matrix multiply in cache-friendly ikj order.
    · Isolates: paired with the existing (deliberately naive ijk)
    matrix_mult, the *pair* separates compiler quality from access order.
    · T2.
30. **row_col_sums** — sum each row, then each column, of one matrix.
    · Isolates: stride-1 vs stride-n in one program. · T2.

### Group B — Scalar optimizations

31. **digit_sum** — sum of decimal digits of 1..n via `/10` and `%10`.
    · Isolates: division/modulo by a small constant in a hot loop — magic
    numbers and divmod pairing (one multiply serving both results). A total
    gap today. · Check: no `idiv` in the hot loop of `_logos.s`; magic
    `movabsq` constant present. · **T1, priority high** — trivial in all 11
    languages.
32. **base_mix** — convert each of n numbers through bases 3, 7, and 10 by
    repeated divmod, checksum the digit streams. · Isolates: B1 across
    several constants at once. · T2.
33. **xorshift_mix** — xorshift64 PRNG iterated n times. · Isolates: B3
    shift/xor codegen quality on a serial chain (bit-heavy sibling of
    lcg_chain). · T2.
34. **bit_reverse32** — reverse the bits of each element the
    shift-and-or way. · Isolates: `llvm.bitreverse`/`llvm.bswap` idiom
    recognition. · Check: `bswap`/`bitreverse` in `.ll`. · T2.
35. **gray_stream** — Gray-code encode then decode n values, verify round
    trip. · Isolates: light bit idioms (`x xor (x shifted right by 1)`).
    · T2.
36. **bitset_sieve** — sieve of Eratosthenes packed 64 flags per word using
    shifts and masks. · Isolates: bit-addressing codegen; paired with the
    existing byte-array sieve, the delta is the cost of bit-packing in each
    language. · **T1 candidate.**
37. **saturating_acc** — accumulate with a clamp at a CLI-derived bound.
    · Isolates: instcombine folding if-then-clamp into `llvm.smin`/`smax`.
    · T2.
38. **nsw_probe** — loop arithmetic where C's signed-overflow UB licenses an
    optimization Rust's defined wrapping forbids (e.g. `i*2 < n*2` bound
    reasoning). · Isolates: the permanent semantic delta between the two
    sides — an *expected, documented* C win, kept so we never misread it as
    a largo bug. · Check: `nsw` flags in `_c.ll` absent from `_logos.ll`.
    · T2.

### Group C — Calls, recursion, dispatch

39. **tail_fact_mod** — tail-recursive accumulator factorial mod p.
    · Isolates: tail-call elimination (recursion → loop). Zero tail
    recursion exists in the suite. · Check: no recursive `call` in
    `_logos.ll` — the function body is a loop. · **T1 candidate, priority
    high** (trivially portable; sizes must note the stack-overflow canary if
    TCO fails).
40. **mutual_parity** — mutually recursive isEven/isOdd to depth n.
    · Isolates: mutual tail-call merging — much harder than self-tail-calls.
    · T2.
41. **tak** — the Takeuchi function. · Isolates: call overhead with heavier
    argument traffic than fib; naturally memoization-resistant.
    · **T1 candidate.**
42. **inline_ladder** — hot loop calling f1→f2→f3→f4, each a trivial
    one-liner `## To` function. · Isolates: inlining depth through largo's
    emission. · Check: zero `call` instructions in the loop. · T2.
43. **bytecode_interp** — a tiny 8-opcode stack VM (push, add, mul, mod,
    dup, swap, jnz, halt) running an LCG-generated-but-valid program for n
    steps. · Isolates: C3 dispatch — does LLVM rebuild a jump table from a
    LOGOS `If` chain where C uses `switch`? — plus indirect-branch
    prediction. Also the most *self-relevant* workload possible: EXODIA is
    an interpreter; "can a LOGOS program interpret fast" is our own story.
    · Check: `switch i64` in `_logos.ll`; `.LJTI` jump table in `.s`.
    · **T1, priority 1.**
44. **state_machine** — a ~10-state DFA classifying an LCG-generated byte
    stream. · Isolates: switch lowering without the VM's stack traffic;
    smaller cousin of 43. · T2.
45. **dispatch_table** — select among k behaviors via Map lookup vs an
    if-chain (paired phases). · Isolates: a devirtualization proxy until
    LOGOS has function values (see Appendix B). · T2.

### Group D — Memory, aliasing, bounds checks

46. **bc_sequential / bc_random** — the bounds-check pair: identical
    sum-over-array, one indexed by ascending `i`, one by an LCG-shuffled
    permutation. · Isolates: D3 exactly. Sequential: LLVM (helped by largo's
    `assert_unchecked`) should erase every check. Random: checks must stay.
    The C-ratio *difference* between the two halves is the irreducible
    bounds-check tax, measured. · Check: `panic_bounds_check` callsites
    reachable from `_logos_main` — 0 for sequential. · **T2, priority 1 for
    the diagnostic tier.**
47. **bc_window** — sliding-window `a[i-1], a[i], a[i+1]` access.
    · Isolates: range inference proving three checks at once. · T2.
48. **noalias_inc** — alternating passes `b[i] += a[i]` then `a[i] += b[i]`
    over distinct arrays. · Isolates: D2 — Rust's noalias lets LLVM keep
    values in registers across the stores; C without `restrict` must reload.
    The flagship *expected-LOGOS-win*; a credible measure needs wins in both
    directions. · Check: `noalias` attrs in `_logos.ll`; extra loads in
    `_c.s`. · T2, promote if it tells a good story.
49. **store_forward** — write `a[i]`, immediately read it back into the
    accumulator. · Isolates: store-to-load forwarding / GVN through
    `borrow_mut()`. · Check: the reload folded away in `.ll`. · T2.
50. **dse_overwrite** — fill an array, immediately overwrite it, checksum
    only the final contents. · Isolates: DSE deleting the first pass
    entirely (legal: the checksum ignores it — *deliberately*, this one
    inverts the checksum rule to test DSE; document it as the exception).
    · Check: one fill loop in `.ll`, not two. · T2.
51. **alloc_churn** — create, fill (small k), sum, and discard a fresh Seq
    per outer iteration. · Isolates: D5 — allocator pressure, Rc refcount,
    drop glue vs C's stack array. Quantifies the `Rc<RefCell<Vec>>`
    construction cost directly. · Check: `__rust_alloc` inside the loop.
    · T2.
52. **seq_of_seqs** — build and traverse a ragged 2D Seq of Seq.
    · Isolates: double indirection — the representation's worst case,
    measured honestly. · T2.
53. **pointer_chase** — build a random single-cycle permutation
    (Sattolo's algorithm with the LCG), then follow `i = p[i]` n times.
    · Isolates: D6 — pure dependent-load latency; compiler-proof by
    construction. The language-neutral anchor: LOGOS must tie C here, or
    plain loads carry runtime overhead. · **T1 candidate, priority high.**
54. **stride_sweep** — sum an array at stride k (k from CLI: 1, 4, 16, 64
    via `sizes.txt`-style variants). · Isolates: the cache hierarchy as a
    function of stride; LOGOS-vs-C deviation at stride 1 but not 64 means
    compute overhead, deviation everywhere means per-access overhead. · T2.
55. **binary_search_batch** — m binary searches over a sorted LCG array.
    · Isolates: cache-hostile branchy reads + whether either compiler makes
    the compare branchless (A5 crossover). · **T1 candidate.**
56. **grow_push** — push n elements with no capacity hint. · Isolates:
    realloc/growth-memcpy behavior (the existing collect benchmark
    pre-sizes its Map; this isolates growth). · Check: `__rust_realloc`
    frequency. · T2.

### Group E — Floating point

57. **fma_poly** — evaluate a degree-8 polynomial (Horner) at n LCG points.
    · Isolates: E1. Today this measures *non*-contraction symmetrically
    (baseline x86-64 has no FMA — verified in the nbody dumps); on aarch64
    or with `-march=native` it becomes the clang-contracts/rustc-doesn't gap
    as a single number. Either way it pins the policy down. FP checksum:
    print with fixed precision (`{:.6}`); if contraction policies ever
    diverge between sides, expected-output comparison must come from the
    same-side reference (§5.2). · Check: `llvm.fmuladd` in `.ll`, `vfmadd`
    in `.s` (arch-dependent). · **T1 or T2, priority 1.**
58. **kahan_sum** — compensated summation of LCG floats. · Isolates: E2 as a
    canary — the compensation term is *deleted* by fast-math reassociation,
    changing the output. If this benchmark's checksum ever changes, someone
    turned on fast-math somewhere. · T2.
59. **libm_mix** — `sum += sin(x) * exp(-x)` over n points. · Isolates: E3 —
    libm call strategy, intrinsic vs PLT call, and (absent) vector-libm.
    · Check: `@sin` / `@exp` call form in both `.ll` files.
    · **T1 candidate.**
60. **sqrt_norm** — normalize n 3-vectors (sqrt + divide). · Isolates:
    `llvm.sqrt` lowering and div-vs-reciprocal choices at strict FP. · T2.
61. **float_quantize** — round n floats to ints, histogram the buckets.
    · Isolates: E4 conversion codegen feeding integer work. · T2.
62. **complex_mul** — multiply n complex pairs held in two parallel Seqs.
    · Isolates: mul/add interleave ripe for SLP and (where available) FMA.
    · T2.

### Group F — Branch predictability (paired-variant methodology)

63. **branch_sorted / branch_shuffled** — identical threshold-counting loop
    over the same LCG values, once sorted (predictable) and once shuffled
    (hostile). · Isolates: the pair separates codegen choice (did either
    compiler emit cmov? then both halves run alike) from hardware prediction
    (branches: sorted wins big). Diagnoses several existing sort-benchmark
    mysteries for free. · Check: compare branch vs `cmov` in both `.s`
    files; `perf stat` branch-misses divergence between halves. · **T2,
    priority 1 — near-zero implementation cost.**
64. **collatz_range** — total Collatz steps over [1, n]. · Isolates:
    loop-exit misprediction with data-dependent trip counts at scale (the
    existing collatz covers the chain; coverage map should confirm whether
    range behavior differs before adding). · T2.
65. **interp_straightline / interp_branchy** — bytecode_interp (43) running
    a straight-line vs a jump-heavy program. · Isolates: indirect-branch
    prediction isolated from dispatch codegen. · T2.

### Group G — Strings, Text, Maps

66. **word_freq** — generate space-separated pseudo-words from the LCG,
    count frequencies in a `Map of Text to Int`, print a fold of the counts.
    · Isolates: Text hashing, Map growth, short-string allocation — the most
    "real program"-shaped workload missing from the suite.
    · **T1 candidate, priority high.**
67. **kmp_search** — KMP over the same haystack as string_search.
    · Isolates: paired with naive string_search, shows whether a smarter
    algorithm pays equally in both languages (complexity-honesty probe).
    · **T1 candidate.**
68. **levenshtein** — DP edit distance between two LCG-generated strings.
    · Isolates: 2D DP with per-char compares (D3 beyond knapsack's integer
    weights). · **T1 candidate.**
69. **rle_codec** — run-length encode then decode generated text, verify the
    round trip. · Isolates: append-heavy Text building + branchy scanning.
    · T2.
70. **csv_fields** — scan generated comma/newline text counting fields per
    record. · Isolates: delimiter scanning (memchr-able) and Text slicing
    cost. · T2.
71. **string_pool** — intern n generated strings through a Map, count
    uniques. · Isolates: hash + compare + allocation composite. · T2.
72. **palindrome_windows** — count palindromic windows of length k.
    · Isolates: two-pointer char compares; a `bcmp`-formation candidate.
    · T2.

### Group H — Heap data structures (language-capability frontier)

The LOGOS binary_trees is allocation-free recursion, so the suite has zero
linked-structure benchmarks on our side. These double as language probes —
if one can't be expressed naturally, that's a finding in itself.

73. **linked_list_sum** — build an n-node list (Seq-of-next-index encoding
    if no recursive types in benchmarks yet), traverse twice. · Isolates:
    allocation locality + dependent loads; sibling of pointer_chase with
    construction cost included. · T2.
74. **bst_insert_walk** — insert n LCG keys into an index-encoded BST,
    in-order walk, checksum. · Isolates: pointer-heavy branchy construction.
    · T2.
75. **hash_open_addr** — hand-rolled open-addressing hash table (linear
    probing) in a flat Seq, n inserts + m probes. · Isolates: the suite's
    fairness rule says "hand-rolled hash patterns" — this makes the hash
    table itself the benchmark rather than the stdlib Map, directly
    comparable across all 11 languages. · **T1 candidate.**

## 4. Coverage after additions

Every taxonomy class with its post-addition population (existing ◦ /
proposed •):

| Class | Coverage after §3 | Singleton risk? |
|-------|-------------------|-----------------|
| A1 LICM | •licm_len •licm_div •licm_addr •licm_pure_call | no |
| A2 SCEV | ◦loop_sum ◦fib_iterative ◦prefix_sum •mod_weighted_sum •lcg_chain •geom_mod •triangle_addr | no |
| A3 idioms | ◦array_fill •array_copy •array_zero_refill •popcount_words •find_sentinel •equal_ranges | no |
| A4 vectorization | ◦spectral_norm(◐) •dot_product •saxpy •int_sum_squares_mod •stencil_3pt •conditional_sum •minmax_scan •slp_quad | no |
| A5 if-conversion | ◦collatz ◦mandelbrot •clamp_stream •abs_delta_sum •count_matches | no |
| A6 unswitching | •unswitch_mode •unswitch_nested | no |
| A7 ILP | •acc_split_sum •hash_fnv •lcg_chain | no |
| A8 cache order | ◦matrix_mult •matmul_ikj •transpose_naive •row_col_sums | no |
| B1 div-by-const | ◦loop_sum(◐) •digit_sum •base_mix | no |
| B2 overflow semantics | •hash_fnv •nsw_probe | no |
| B3 bit idioms | ◦nqueens(◐) •popcount_words •bit_reverse32 •gray_stream •bitset_sieve •xorshift_mix | no |
| C1 inlining | ◦fib ◦ackermann •inline_ladder •tak | no |
| C2 tail calls | •tail_fact_mod •mutual_parity | no |
| C3 dispatch | •bytecode_interp •state_machine •dispatch_table | no |
| D1 SROA | (implicit in all Seq benchmarks) | **yes — flagged**: no isolated probe; revisit once a struct-like LOGOS feature exists to scalarize |
| D2 noalias | •saxpy •noalias_inc | no |
| D3 bounds checks | ◦(everything) •bc_sequential/bc_random •bc_window | no |
| D4 DSE/forwarding | •store_forward •dse_overwrite | no |
| D5 allocation | ◦binary_trees(C-side only) ◦collect •alloc_churn •grow_push •seq_of_seqs | no |
| D6 memory latency | ◦graph_bfs(◐) •pointer_chase •stride_sweep •binary_search_batch | no |
| E1 FMA | •fma_poly •complex_mul | no |
| E2 reassociation | ◦pi_leibniz •kahan_sum | no |
| E3 libm | ◦nbody(◐) •libm_mix •sqrt_norm | no |
| E4 conversions | ◦mandelbrot(◐) •float_quantize | no |
| F predictability | ◦sorts(entangled) •branch_sorted/shuffled •interp pair | no |
| G strings/maps | ◦strings ◦string_search ◦two_sum ◦collect •word_freq •kmp_search •levenshtein •rle_codec •csv_fields •string_pool | no |
| H heap structures | •linked_list_sum •bst_insert_walk •hash_open_addr | no |

Rule going forward: a class with a single benchmark is a Goodhart liability —
one program shape is one pattern-match away from being gamed. D1 is the only
accepted singleton (capability-gated), and it is flagged.

## 5. Methodology for new benchmarks

1. **Sizing.** Calibrate so the C side runs 100–2000 ms (matches the
   `run-logos-vs-c.sh` baseline practice). `sizes.txt` gets 3–4 sizes
   spanning at least 16× so scaling is observable. Memory-class benchmarks
   (D6, A8) choose sizes whose working sets deliberately straddle L2 and
   LLC; document target bytes per size in a comment in `sizes.txt`'s
   directory.
2. **Checksums.** One printed line per program. Integer benchmarks fold
   every computed value mod 1e9+7. FP benchmarks print a reduction at fixed
   precision; benchmarks that *measure* value-changing optimizations (FMA
   contraction) must generate expected outputs from a designated reference
   side and document the tolerance policy in the program directory.
   Exception class: DSE probes (dse_overwrite) deliberately exclude work
   from the checksum — mark these in their README line so nobody "fixes"
   them.
3. **Closed-form resistance.** All loop bounds and foldable constants pass
   through CLI `n` or the standard LCG. Summations get mod-prime armor.
   Acceptance rule: if `largo build` at a fixed `n` could constant-fold the
   printed answer, the benchmark is invalid — verify by reading
   `generated/<bench>.rs` before first merge.
4. **Scaling-exponent tripwire.** Each benchmark records its expected
   complexity (e.g. `scaling: n^1`) alongside `sizes.txt`. CI fits
   log-time vs log-n across the sweep that `run.sh` already performs and
   alarms when any language's fitted exponent deviates from spec by more
   than ~0.15. This is the cheapest possible detector for a compiler
   "solving" a benchmark (O(n) → O(1)) and for accidental algorithmic drift
   between the 11 implementations.
5. **Paired-variant discipline.** Pairs (bc_sequential/bc_random,
   branch_sorted/branch_shuffled, sieve/bitset_sieve, matrix_mult/matmul_ikj,
   string_search/kmp_search, interp pair) are first-class objects: report
   the intra-pair delta per language next to the LOGOS/C ratio. An
   optimization that improves one half beyond its physical justification is
   the Goodhart alarm going off.
6. **Grep-able acceptance criteria.** Every T2 diagnostic ships with its
   dump assertions written down (one line per check, e.g.
   "`_logos.ll`: `panic_bounds_check` reachable from `_logos_main` == 0",
   "`_logos.ll` contains `llvm.memcpy`"). "Done" is mechanical: run
   `ONLY=<bench> DUMPS=1 bash benchmarks/run-logos-vs-c.sh`, run the greps.
   Caveat learned from the current dumps: `*_logos.{s,ll}` are whole-binary
   dumps (~340k lines including the Rust stdlib), so naive whole-file grep
   counts are polluted — scope checks to the `_logos_main` region (e.g.
   `awk '/_logos_main/,/^}/'` on the `.ll`) or count deltas against a
   no-op baseline program.
7. **Hardware-class benchmarks need counters, not asm.** D6 and F classes
   are diagnosed with `perf stat -e branches,branch-misses,LLC-load-misses`,
   not IR diffs. Note it per benchmark so nobody stares at identical
   assembly wondering where the ratio came from.
8. **Compiler/flag symmetry.** Two standing asymmetries to keep in view:
   (a) timing uses `gcc -O2` while dumps use `clang -O2` — when a dump
   contradicts a timing, check the gcc asm (`gcc -O2 -S`) before concluding
   anything; consider adding a gcc `.s` dump alongside the clang one in
   `DUMPS=1`. (b) No `-march=native` anywhere: baseline x86-64 = no FMA, no
   AVX, no POPCNT on the timed binaries. That is a *fairness feature* (both
   sides equally constrained) but it silently caps A4/B3/E1 upside; if flags
   ever change, they change on both sides in the same commit, and fma_poly /
   popcount_words / dot_product are the benchmarks that will move.
9. **Diagnostic-tier mechanics.** `programs-diag/<bench>/` containing
   `main.c`, `main.lg`, `sizes.txt`, `expected_<n>.txt`; a `DIAG_BENCHMARKS`
   array in `run-logos-vs-c.sh` appended to `BENCHMARKS` when `DIAG=1`;
   excluded from `run-quick.sh`, `run.sh`, and CI. Promotion to T1 = move
   the directory, add the nine other implementations, register in the other
   scripts per the README checklist.

## 6. Prioritized shortlist

**Phase 1 — biggest blind spots, cheapest wins:**

1. **dot_product** (T1) — the vectorized-reduction flagship; A4+D3+E1 in
   one number.
2. **bc_sequential / bc_random** (T2) — turns the #1 known LOGOS tax into a
   measured delta with a pass/fail grep.
3. **branch_sorted / branch_shuffled** (T2) — near-zero cost, immediately
   reclassifies several sort-benchmark mysteries as hardware-vs-codegen.
4. **bytecode_interp** (T1) — jump tables + the most self-relevant workload
   in the list; hardest of phase 1, highest narrative value.
5. **digit_sum** (T1) — div-by-constant magic; an afternoon across all 11
   languages.

**Phase 2 — structural costs and credibility:**
tail_fact_mod, pointer_chase, fma_poly, popcount_words, noalias_inc (the
expected-LOGOS-win exhibit), word_freq, lcg_chain.

**Phase 3 — fill remaining classes:**
stencil_3pt, unswitch_mode, alloc_churn, kahan_sum, licm_len, bitset_sieve,
levenshtein, kmp_search, stride_sweep, hash_open_addr, then the rest by
§4's coverage table.

Ordering principles: (a) zero-coverage classes before partially-covered
ones; (b) benchmarks that quantify *known* LOGOS structural costs (bounds
checks, Rc indirection, allocation) before speculative ones; (c) at least
one expected-LOGOS-win per phase, because a measure that can only move in
one direction teaches us nothing about regressions.

## 7. Appendices

### A. Lineage

| Candidate | Ancestry |
|-----------|----------|
| dot_product, saxpy, stencil_3pt | Livermore loops K1/K3/K12, PolyBench |
| matmul_ikj, transpose_naive | PolyBench (gemm, atax variants) |
| bytecode_interp, state_machine | SPEC perlbench/gcc dispatch kernels, embench statemate |
| hash_fnv, xorshift_mix | embench md5/crc lineage |
| nbody, spectral_norm, fannkuch (existing) + libm_mix, tak | Computer Language Benchmarks Game |
| kahan_sum | numerical-analysis canon (Higham) |
| pointer_chase, stride_sweep | lmbench / memory-mountain methodology |

### B. LOGOS feature dependencies

- Bit verbs (`shifted left by`, `shifted right by`, `xor`, `or`, `and`) are
  confirmed in use (`programs/nqueens/main.lg`) — Groups B3 and bitset_sieve
  are expressible today.
- Function values / closures do not exist in benchmark-surface LOGOS, so
  true devirtualization (indirect-call promotion) is deferred;
  dispatch_table (45) is the proxy. Revisit when the language grows
  first-class functions.
- Recursive inductive types exist in the kernel but no benchmark uses
  heap-linked nodes; Group H starts with index-encoded structures so it is
  expressible now, and upgrades to real linked nodes when we decide the
  surface syntax for them is benchmark-ready.
- `Text` indexing cost model is unverified — before trusting Group G ratios,
  read `generated/strings.rs` and confirm `item i of result` on Text is O(1)
  byte access, not a char-boundary scan.

### C. Grep cookbook

All against `DUMPS=1` artifacts. Remember §5.6: LOGOS dumps are whole
binaries — scope to `_logos_main` (and the functions it calls) or compare
counts against a minimal baseline program.

```bash
# Vectorization fired? (A4)
grep -c 'llvm.vector.reduce' asm/<b>_logos.ll
grep -cE '<(2|4|8) x (i64|double)>' asm/<b>_logos.ll

# Bounds checks left in the hot path? (D3)
awk '/define.*_logos_main/,/^}/' asm/<b>_logos.ll | grep -c panic_bounds_check

# Idiom recognition? (A3, B3)
grep -c 'llvm.memcpy\|llvm.memset' asm/<b>_logos.ll
grep -c 'llvm.ctpop\|llvm.bswap\|llvm.fshl' asm/<b>_logos.ll

# Magic-number division? (B1) — look for the movabs+imul pattern, no idiv
grep -c 'idivq' asm/<b>_logos.s        # want 0 in the hot loop
grep -c 'movabsq' asm/<b>_logos.s

# Jump table built? (C3)
grep -c 'switch i64' asm/<b>_logos.ll
grep -c '\.LJTI' asm/<b>_logos.s

# Tail call eliminated? (C2) — recursive fn body should contain no self-call
awk '/define.*<fnname>/,/^}/' asm/<b>_logos.ll | grep -c 'call.*<fnname>'

# Branchless? (A5)
grep -c 'select i1' asm/<b>_logos.ll
grep -cE 'cmov|maxsd|minsd' asm/<b>_logos.s

# noalias in play? (D2)
grep -c 'noalias' asm/<b>_logos.ll

# FMA contracted? (E1 — arch/flag dependent, see §5.8)
grep -c 'llvm.fmuladd' asm/<b>_logos.ll
grep -cE 'vfmadd|fmadd' asm/<b>_logos.s

# Allocation in the hot loop? (D5)
grep -c '__rust_alloc\|__rust_realloc' asm/<b>_logos.ll
```
