# ORACLE UNIFICATION — optimization-gate audit & grind-out plan

## Thesis

A six-front audit of the optimization surface found **one disease repeated ~25 times**: a
pass answers a hard question — *is this expression pure?*, *is this index in bounds?*, *does
this value fit i32?*, *is this list sole-owned?* — with a **bespoke, syntactic, single-shape
predicate**, when a strictly stronger oracle for that exact question **already exists in the
tree and is already consulted by a sibling pass**. de-Rc was just the first instance noticed
(its firing gate was narrower than its soundness requirement).

The win is therefore **not** 25 one-off gate-wideners. It is to **collapse the bespoke
predicates into the shared oracles that already exist**, so widening one place lights up many
passes at once. Every widening in this plan is "consult a stronger *sound* oracle," never
"loosen a check" — which makes it lift-and-shift (CLAUDE.md rule 12), removes duplication so
features compose, and keeps soundness as an invariant.

Two forms of the disease:

1. **Bespoke predicate shadows a stronger oracle.** (Plays A, B, C.) The compiler owns ~6
   real analyses — `effects.rs` purity, `abstract_interp` `OracleFacts`/kernel-LIA, `affine.rs`,
   the kernel prover, the de-Rc ownership analysis, the JIT fusion-soundness check — but each
   is consulted by *one* pass while the others re-roll a weaker local version.
2. **Hand-enumerated variant/stencil tables.** (Play D.) The JIT stacks a representation/op
   gate on top of an already-sufficient soundness condition: a fusion proven sound for
   int/single-buffer/Add is simply *not built* for float/dual-buffer/Eq, so it falls back.

## Sequence (dependency-correct; all of it, in this order)

```
Baseline(green+bench)  →  A (purity oracle)  →  C (affine repr)  →  B (bound oracle)  →  D (JIT fusion)  →  Closeout
```

- **A first** — highest fan-out (5 passes), lowest risk, infra already exists.
- **C before B** — B's best cases (dense-map on index-domain, graph_bfs) are *blocked* by the
  affine representation poverty C fixes; neither fires alone.
- **D last** — independent, and the purity oracle from A is exactly what tells the fusion
  framework which calls are safe to fold.

They compound: A feeds CSE/LICM → exposes more invariant work → feeds C/B induction; B's
straight-line BCE + the `i≤j` relaxation together are the quicksort lever; A's purity oracle
gates D's call-fusions.

## Non-negotiable grind rules (from CLAUDE.md)

- **TDD, RED first.** Each phase opens with a RED test that pins the widened behavior. NEVER
  edit a RED test to make it pass — fix the implementation. Stop and ask if a RED test looks wrong.
- **Start every phase from all-green** and end every phase all-green. A failing test is always
  a regression; do not advance with one red.
- **Bit-identical or prove-the-delta.** Every widening must be observationally identical for all
  inputs (sound oracle ⟹ same value), OR carry a runtime guard that makes it so. Benchmarks must
  stay bit-identical; record geomean before/after, no benchmark regresses.
- **Widen-with-a-guard, not widen-by-deleting** where soundness needs a residual obligation
  (e.g. graph_bfs `% m` needs an emitted `m ≥ 1` positivity guard, not a deleted check).
- One test suite at a time (rule 11). Full runs via `./scripts/run-all-tests-fast.sh`; targeted
  RED/GREEN via `cargo nextest run -E 'test(<name>)'`. No git, ever.

---

# PLAY A — One purity/totality oracle, consumed everywhere

**Disease:** the compiler has *two* purity notions already — `effects.rs::classify_native_function`
and `codegen/hoist.rs::is_pure_scalar_builtin` (`sqrt/abs/floor/ceil/round/pow/min/max`) — but
`effects.rs` is consumed by exactly **one** pass (PE specialization), and every other pass rolls
its own `is it a call? → bail`.

**Evidence:**
- `optimize/gvn.rs:63-66` (CSE) — `_ => None`: every call un-numbered, never CSE'd.
- `optimize/licm.rs:105-107` (LICM) — `_ => false`: every call "variant", never hoisted. LICM
  *already* has the preheader-guard machinery (`hoist_invariant_loads`, `licm.rs:376`) that makes
  hoisting a partial/panicking call sound, and still won't.
- `optimize/effects.rs:867-889` — `classify_native_function` is a **closed allowlist** with
  `_ => EffectSet::io()`; omits obviously-pure `length`, `concat`, `reverse`, `contains`,
  `exp`, `atan2`, `log2`, `cbrt`, `hypot`, `sign`, `gcd`, and any user `## To native` pure helper.
  Through the fixpoint (`effects.rs:329-376`) `io` poisons every transitive caller, so
  `function_is_specialization_safe` (`effects.rs:275-284`, used at `partial_eval.rs:676`) goes false
  for the whole chain.
- `optimize/inline_tiny.rs:73` (`arg_atomic` — Literal/Identifier only), `optimize/ctfe.rs:34-56`
  (`is_pure_stmt` has no `Repeat`/`Inspect` arm → pure loop helpers never fold), and
  `optimize/loop_carried_cse.rs:320` (literal-entry-only seed) are the same gate in 3 more spots.

**Soundness:** CSE of a pure expression is sound (same value, no observable effect; totality not
even required — it replaces a *later* eval with an *earlier* one in the same straight-line region).
LICM additionally needs totality, already handled by its preheader guard. Adding *more* pure
entries only widens safe specialization; the table stays conservative-additive (the shipped stdlib
natives `read/write/now/sleep/randomInt/args/get` are genuinely IO/nondet — keep the IO default for them).

**Tasks:** A-RED → build unified oracle → wire GVN/CSE + LICM → wire CTFE/inline_tiny/loop_carried_cse → GREEN+bench.

**Lights up:** per-iteration `sqrt`/`pow`/`abs` hoist+CSE on the float clusters (nbody/mandelbrot-shaped);
PE specialization across pure-native chains; pure-loop CTFE folding.

---

# PLAY C — Strengthen the affine representation (precondition for B's best cases)

**Disease:** representation poverty in `affine.rs` blocks the bound oracle from spelling facts it
can already *prove*.

**Evidence:**
- `optimize/affine.rs:108-131` `lin_of` has no `Expr::Length` arm (and `lin_to_rust`,
  `abstract_interp.rs:4091`, renders only identifier+constant terms) → a map sized
  `with capacity (length of input)` cannot be represented, so `gather_implicit_map_caps`
  (`abstract_interp.rs:4064-4065`) declines it — even though the per-key proof engine
  `try_prove_dense_key` (`abstract_interp.rs:3007`) reasons about `length(arr)` as a first-class
  symbolic quantity. Proof side understands lengths; capture side can't spell them.
- `length_def: HashMap<Symbol,(Symbol,i64)>` (`abstract_interp.rs:1698`) holds only `n + off`,
  never `coeff·n`; `infer_build_length` (`abstract_interp.rs:3519-3524`) explicitly skips multi-push
  loops → graph_bfs `adj` (5 pushes/iter, length `5n`) gets no length fact, leaving its inner
  adjacency store checked.

**Soundness:** a `coeff·n + off` lower bound is as sound as `n + off`; FM already handles scaled
terms. A `length(x) → x.len()` term is sound by construction. Both are representational widenings,
not check relaxations.

**Tasks:** C-RED (length-sized dense map + multi-push length fact) → extend `lin_of`/`lin_to_rust`
length term + `length_def` coeff·n → GREEN+bench.

**Lights up:** dense-map-on-index-domain (counting-sort/histogram class — the `collect` 1.42s→0.07s
lever, currently denied to length-sized maps); graph_bfs's last checked store (with B).

---

# PLAY B — `abstract_interp`'s Oracle as the *single* bound provider

**Disease:** the literal de-Rc twin — a bespoke bound prover where the full Oracle is in scope and
already used by the sibling map narrowing.

**Evidence:**
- `codegen/narrow.rs:322-397` (`classify`) runs a hand-rolled **3-source** prover (const / `e % m` /
  one accumulator idiom) and bails the whole buffer on anything else (`narrow.rs:396 w.bail = true`),
  taking **no `OracleFacts`** — while the sibling map narrowing one call below it consults the full
  oracle: `i64_map.rs:229-232` `fits_i32 = |addr| oracle.expr_int_range_addr(addr)…`, and the live
  `oracle` is in scope at the Seq call site (`codegen/program.rs:632-650`) and dropped on the floor.
- **Straight-line BCE gap:** `abstract_interp.rs` invokes the affine/LIA prover
  (`record_affine_index_bounds`) *only* from `rich_walk_loop` (`:5314`,`:5320`); every non-loop
  access falls to the weaker `index_provably_in_bounds` (`:2086`), which never consults `length_def`.
  → quicksort pivot read `arr[hi-1]` and post-partition swaps stay **checked** though the entry guard
  already proved `hi ≤ len(arr)` and seeded `length_def(arr)=(hi,0)` (`:4395`). Fix = run the existing
  `affine_walk` over straight-line statement runs, not just loop bodies.
- **`i0==j0` over-gate:** `derive_iv_le_invariants` (`abstract_interp.rs:2672-2678`) requires
  `entry.scalar_def.get(&i) == entry.scalar_def.get(&j)` when the per-iter `Δi ≤ Δj` is already
  proven, so soundness needs only `i0 ≤ j0`. Excludes Lomuto/Hoare partitions with `i=lo-1` /
  `i=lo, j=lo+1`. This is the concrete quicksort relational-BCE lever.
- `codegen/worklist.rs` presize and dense-map capture run their own bound shapes instead of asking
  the Oracle.

**Soundness:** identical facts, identical kernel prover — only the *call site* is loop-gated.
Replace `a == b` with a kernel `prove(i0 ≤ j0)` (constant case trivial; `lo-1 ≤ lo` is one FM call).
The `% m` element-bound on the VM path needs an emitted positivity guard (`vm/compiler.rs:556-568`,
decisive `:564`) — widen-with-a-guard.

**Tasks:** B-RED (Seq narrowing via oracle, straight-line quicksort BCE, `i0≤j0`, dense-map
index-domain, worklist) → `narrow.rs` → `OracleFacts` → straight-line `affine_walk` + `i0≤j0`
relaxation → dense-map capacity + worklist → Oracle → GREEN+bench.

**Lights up:** graph/cache int arrays (narrowing); quicksort/mergesort/heap_sort straight-line tails
+ partition (BCE); dense-map + worklist.

---

# PLAY D — Op/repr-agnostic JIT fusion (stop hand-enumerating variant tables)

**Disease:** a repr/op gate stacked on an already-sufficient soundness condition. Several are
self-admitted "variant table not built yet."

**Evidence (ranked):**
1. `vm/machine.rs:1969-1975` — `NewEmptyList` allocation-reuse gated `ListRepr::Ints(_)`; the real
   precondition (sole ownership, `Rc::strong_count==1 && weak==0`) is checked at `:1972-1973`.
   `Vec::clear()` is repr-agnostic. A sole-owned `Seq of Float`/`Bool`/`Boxed`/`Struct` rebuilt each
   iteration reallocates `Rc<RefCell<Vec>>` every pass (`:1983`). **de-Rc almost verbatim.**
2. Float `EqF`/`NeqF` absent from the XMM-pin whitelist — `logicaffeine_jit/src/lib.rs:6160-6216`
   (whole-region pin-drop at `:6273` `!ops.iter().all(xmm_pin_safe)`) and `forge/jit.rs:2076-2087`
   (value-form, "no variant yet" at `:1921-1933`). Scored on the *same* mem-form path as the
   whitelisted ordering compares (`jit.rs:5964-5973`). One `while x != prev` drops *every* XMM pin
   for the whole region. Mirror the shipped Lever-2a ordering additions; build a `V_FCMP` table.
3. `vm/compiler.rs:1872-1880` — `ModPow2` emits 2 ops (`LoadConst`+`AndEager`) while sibling
   `DivPow2` (`:1853-1858`) emits 1; the mask is a compile-time constant. Collapse to one
   `AndImm`/`ModPow2{dst,lhs,k}`. (VM-tier only; JIT already collapses.)
4. `logicaffeine_jit/src/lib.rs:2192` — two-buffer *float* load+binop fusion is same-buffer only
   (`coll_a != coll_b → None`); the integer twin `ArrLoad2` already carries two pointer slots
   (`:5003-5013`). Needs a dual-buffer `ArrLoad2F` stencil. Cross-array float kernels `a[i]*b[j]`.
5. `codegen/peephole.rs:1757-1761` — `is_simple_expr` excludes `BitXor`/`Shl`/`Shr` (pure total int
   ops); gate for ~20 downstream AOT lowerings (`.swap()`, `with_capacity`, presize, fill). The
   `:1786` `_ => format!("({})", l)` fallback also **silently drops the operator** — latent
   miscompile on the two guard-less callers. Fix `^`/`<<`/`>>` (keep `&`/`|` out — `&&` vs `&`).
6. Bitwise two-buffer `IOp` (`lib.rs:4925-4930`, lacks And/Or/Xor/Shl/Shr) and `MulF+SubF` FMS
   (`lib.rs:5315-5318`, Add-only) — float/bitwise analogs of proven-sound int/Add fusions.

**Soundness:** each is a repr/op gate on top of a sufficient condition (sole-ownership; op-agnostic
mem-form spill that touches only its own slot; constant mask). Generate the fusion/pin tables from
the operand-type × op cross-product behind the one soundness predicate.

**Tasks:** D-RED → NewEmptyList repr drop → V_FCMP EqF/NeqF pins (jit+forge) → dual-buffer
ArrLoad2F + ModPow2 one-op + bitwise IOp + peephole shift/xor (+ operator-drop fix) → GREEN+bench.

**Lights up:** float cluster (whole-region pin retention) + allocation cluster.

---

# Folded relaxations (Play E — done inside A/B/D where adjacent)

- `loop_carried_cse.rs:320` literal-entry → runtime `Let temp be o*o` seed at the pre-loop point
  (bit-identical for any entry value; deletes ~35 lines of literal machinery). Fold into A.
- `inline_tiny.rs:73` `arg_atomic` → `call_free(a) && param_uses[i] ≤ 1` (zero-cost rename). Fold into A.
- `closed_form.rs:459` `start==0` bail for Count/Sum (formula already correct for start ≤ 1; the
  hand-off peephole only emits an O(n) loop, so the O(1) collapse is genuinely lost). Fold into B.
- `unroll.rs` run-path scalar const-trip loop left rolled (interpreter dispatch). Low priority.
- `float_induction_sr.rs:140-148` integer-coeff-only → dyadic coeff with re-proved magnitude bound.
  Medium risk (FP magnitude reasoning); defer unless a dyadic-coeff hot loop appears.

# Deliberate gates — DO NOT "fix" unilaterally (documented so we don't re-litigate)

- Run-path recursion inlining is opt-in (mask bit 256 OFF) — measured net-negative (nqueens +159%).
- Leaf/tiny inliners run run-path-only, not AOT — AOT lets LLVM inline; justified asymmetry.
- Pair-TCE top-level-only (`tce.rs:60-68`) — load-bearing for the TCO three-tier-consistency
  invariant; widening must be a coordinated all-three-tier change, not an AOT gate flip.
- AoS regime gate (`detection.rs:2667`) — real measured nbody regression motivates it; only safe
  widening is to scope `block_has_const_member_index` to the hot loop, not the whole program.
- VM keeps `% m` checks by design ("largo run is not the benchmark path") — but EXODIA note says
  largo run *is* the VM+JIT benchmark path, so the positivity-guarded widening (D/B) is in scope.

# Closeout

Full parity run (`run-all-tests-fast.sh` + `compare-test-runs.sh` zero-missing/zero-extra),
benchmark recalibration (compute-bound clusters, not the V8 startup floor — see
benchmark-floor-inflation), update this doc with measured deltas, update memory.
