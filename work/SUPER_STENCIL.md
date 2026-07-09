# SUPER_STENCIL.md: Stencils as Compiled Semantic Facts

> Companion to [EXODIA.md](EXODIA.md). EXODIA asks "how do we derive an optimal
> compiler?"; this document asks "what is the **next level** for the copy-and-patch
> JIT EXODIA Phase 3 shipped?" — and answers it against the code we actually have, with
> feasibility ratings and per-benchmark ROI.

---

## 0. The Reframe

A stencil today is an **opcode snippet**: "given holes A, B, C, emit this machine code."
`logicaffeine_forge` extracts those snippets from Rust at build time and patches two
kinds of hole — `HoleId::Cont(n)` (a branch target) and `HoleId::ConstI64(n)` (a 64-bit
immediate) — at JIT time (`crates/logicaffeine_forge/src/stencil_model.rs`).

The next level is to stop thinking of a stencil as a snippet and start thinking of it as
a **compiled semantic fact with holes**:

```
not:  stencil ADD_INT
but:  stencil "this semantic situation"  —  types known, shapes known, aliasing
                                            known, bounds known, effects known,
                                            callee known, layout known, deopt
                                            target known, proof obligations known
```

Then the JIT stops being a code generator and becomes a **fact-to-code machine**: it
consumes the facts the Oracle already proves and emits the rawest machine code those
facts make legal, guarded by the assumptions it could not prove statically, with a deopt
edge for when a guard fails.

**The headline finding, stated up front because every priority below follows from it:**
our AST/codegen optimizer is already world-class (it emits 32×32 tiled `ikj` matrix
loops that gcc/clang don't at `-O2`), and we are at **~0.97× geomean vs C**. The
remaining losses are not optimizer gaps — they are **runtime representation tax**. Every
value is `LogosSeq<T>(Rc<RefCell<Vec<T>>>)` or `LogosMap<K,V>(Rc<RefCell<FxHashMap>>)`
(`crates/logicaffeine_data/src/types.rs:49,215`), and the worst benchmarks pay for it in
`borrow()` traffic, allocation churn, and boxing:

| Benchmark | vs C | Why we lose (from `benchmarks/POTENTIAL_OPTIMIZATIONS.md`) |
|-----------|------|------------------------------------------------------------|
| knapsack | **4.56×** | Inner DP loop: `curr.borrow_mut()[w]` + up to 3 more borrows + checked index per iteration |
| nbody | **3.70×** | 5 bodies in parallel `Seq`s; ~15 `borrow()` per (i,j) pair. C keeps a 5-struct array in registers |
| ackermann | **2.29×** | TCE fires; residual-call frame setup vs gcc's recursion-to-iteration |
| mergesort | **1.72×** | `to_vec()` + fresh result `Vec` per recursion level; C merges into one scratch buffer |
| matrix_mult | **1.54×** | OPT-TILE fires, then 3× `borrow()` + 1× `borrow_mut()` + 4 bounds checks per FMA eat the win |
| two_sum | **1.29×** | `LogosMap` borrow per access vs C's hand-rolled open-addressing table |
| counting_sort | **1.21×** | Scatter increments through `borrow_mut()` per element |

So the move is precise: **stencil the semantic escape hatches** — the moments where
managed semantics (`Rc<RefCell<Vec>>`) can collapse to raw-machine semantics *under
guard*. Not just arithmetic. Not just bytecode handlers. The collapse points.

---

## 1. The Unfair Advantage — What We Already Own

Most of the twelve ideas below are **wire-up**, not invention. We already shipped the
hard parts. This table is what makes the ROI estimates credible.

| Capability the ideas need | Where it lives | State |
|---------------------------|----------------|-------|
| Copy-patch stencil engine, holes, arm64+x86-64 patchers | `logicaffeine_forge` (`stencil_model.rs`, `build.rs`, `patch.rs`) | ✅ shipped |
| Build-time stencil extraction from Rust asm; 125 register-threading location variants per ALU family | `logicaffeine_forge/build.rs`, `stencils/int_stencils.rs` | ✅ shipped |
| VM↔JIT seam (`NativeTier` trait), `ForgeTier` adapter | `logicaffeine_jit`, `compile/src/vm/native_tier.rs` | ✅ shipped |
| **Speculative typed regions** — `ObservedKind{Int,Float,Bool,IntList,FloatList,BoolList,Map,Other}` | `vm/native_tier.rs:258` | ✅ shipped |
| **Entry guards** `(slot, expected_kind)`; **precise/region-grade deopt** (resume in bytecode, `NativeFrame` materialization) | `vm/machine.rs`, `vm/native_tier.rs` | ✅ shipped |
| **`FnTable` hot-swap seam** (EXODIA 4.7) — atomic entry pointers for self-calls | `vm/native_tier.rs:25` | ✅ shipped |
| Tier-up triggers (`NATIVE_TIER_THRESHOLD=100`, `REGION_TIER_THRESHOLD=100`) | `vm/native_tier.rs:370,373` | ✅ shipped |
| **The Oracle** — intervals (widening ladder), types, shapes, nullability, **aliases** | `optimize/abstract_interp.rs` (`OracleFacts:1528`, `AliasInfo:699`, `loop_handles_definitely_distinct:1613`, `index_provably_in_bounds:1638`) | ✅ Phase 1 COMPLETE |
| **Effects** — `EffectSet{reads, writes, allocates, io, security_check, diverges, unknown}`, Pure⊂Read⊂Write⊂IO⊂Unknown | `optimize/effects.rs:22` | ✅ shipped |
| Ownership (Give vs Show), escape, liveness, **readonly** (`&[T]`-eligibility), call-graph SCCs | `analysis/{ownership,escape,liveness,readonly,callgraph}.rs` | ✅ shipped |
| Binding-time analysis (Static/Dynamic), polyvariant | `optimize/bta.rs` | ✅ shipped |
| **Soundness certificates** — kernel e-graph mints kernel-checked certs; `ProofCertificate` | `logicaffeine_kernel/src/cc.rs`, `logicaffeine_verify/src/certificate.rs` | ✅ shipped |
| Offline Z3 synthesis + translation validation (the safety net) | `logicaffeine_synth`, `logicaffeine_verify`, `logicaffeine_tv` | ✅ shipped |
| **All three Futamura projections** (P1 Jones-optimal, P2, P3) | EXODIA §0.2, `phase_futamura.rs` (436 tests) | ✅ COMPLETE |

The losses listed in §0 are exactly the targets the codegen roadmap already scoped as
**O1 borrow hoisting, O2 de-`Rc` escape analysis, O3 small-`Seq` scalarization**
(`POTENTIAL_OPTIMIZATIONS.md` §coverage). This document is the **JIT/stencil expression
of those same levers** — done at runtime under guard, where the static optimizer cannot
prove the precondition but the speculative region can assume it.

---

## 2. The Spine — Stencils as Program Derivatives

The user's intuition — *"we have all three Futamura projections; this feels like taking
derivatives of programs"* — is the organizing principle, and it has a rigorous home.

**A fact-driven stencil is the residual of partially evaluating a fact-enriched
interpreter.** Formally, the First Futamura Projection is
`PE(interpreter, program) = compiled_program`. Enrich the interpreter's state from
`(pc, stack, locals)` to `(pc, stack, locals, facts, guards, deopt_map)` and the same
projection becomes:

```
PE(factful_interpreter, bytecode + Oracle facts + profile)  =  guarded optimized code
```

The three pieces of a proof-carrying stencil fall straight out of staging:

- **Holes** = the *dynamic residual* — the inputs the PE could not fix at specialization
  time (today: `Cont`, `ConstI64`; tomorrow: register classes, layouts).
- **Guards** = the *staged binding-time assumptions* — the facts the PE *assumed* to
  produce the fast residual (this slot is `Int`; this `Seq` is unique; refcount == 1).
- **Deopt edge** = the *fallback when a staged assumption is violated at runtime* —
  precisely the `NativeFrame`-materialization path we already ship.

Because **P2 and P3 are done**, we are not limited to hand-writing stencils:

- **P2** (`PE(pe_source, factful_interpreter) = a compiler`) means we can **derive the
  stencil-emitting JIT** from the factful interpreter rather than hand-maintaining the
  125 register-threading variants. The variants become *output*, not source.
- **P3** (`PE(pe_source, pe_source) = a compiler generator`) means we can derive a
  **family of specializers, one per fact-regime** — a unique-alias specializer, a
  frozen-object specializer, a no-escape specializer — each a different "stencil pack."

**On the word "derivative" — the honest version.** Partial evaluation is *not* automatic
differentiation, and we should not pretend it is. But there is one rigorous anchor that
earns the metaphor: when a hot region's **fact profile drifts** and a guard fails, we do
not recompile from scratch — we re-specialize against the *delta* in the facts. That is
**incremental recompilation = finite differencing over the fact-environment**. A stencil
farm (Idea 11) that re-specializes regions on fact-deltas is computing "the program's
response to a change in its facts." The deopt → re-specialize loop *is* the derivative,
applied to a delta in the assumptions. Everything else is analogy; flag it as such.

This spine ties Ideas **5** (stenciled PE), **10** (factful interpreter), **11** (stencil
farms), and **12** (whole-tower) into one mechanism.

### 2.1 The frontier — what's been done, and why this is undone

None of the pieces are new *in isolation*. **Copy-and-patch** (Xu & Kjolstad, 2021)
stitches binary stencils with holes for fast codegen — that is `forge`, and the published
work stops at arithmetic and bytecode handlers. **Truffle/Graal** specializes interpreters
by partial evaluation — that is our Futamura layer, but it residualizes into a general
compiler IR, not into copy-patchable stencils. **Souper** and **Minotaur** use SMT and
synthesis to *discover verified optimizations* — that is `synth`, but offline and for
arbitrary functions, not as a live stencil market. **STOKE** searches binary space
stochastically. **Proof-carrying code** (Necula, 1997) ships a proof alongside a binary —
but as a one-shot load-time safety check, not fused into a JIT's per-fragment stencils.

What has **not** been done, as far as we can find, is unifying all of that into one
**self-applicable, proof-aware, copy-patchable language tower** — where the *same* engine
that mints kernel-checked certificates (`logicaffeine_kernel`) also mints the stencils,
and the *same* partial evaluator that achieves all three Futamura projections also
*emits* those stencils. Each prior system holds one corner. We are the rare codebase that
holds every corner at once, which is the only reason the ideas below are wire-up rather
than PhD theses.

That unification unlocks the actual next-level reframe: **the JIT stops being a code
generator and becomes a semantic cache.** Machine code is merely *one* cached artifact.
The co-equal artifacts are proofs, guards, layouts, alias facts, effect facts, deopt
recipes, register schedules, and microarch choices — each one queryable, reusable,
invalidatable, and *teachable*. The compiler database can then answer questions no
production JIT can today: *why was this code legal? what fact enabled this raw load? what
guard protects this elided check? can this fact be reused elsewhere? can this failed guard
teach the specializer a new stencil?* A deopt stops being a failure and becomes a new fact
(§2's derivative). **That** is the thing nobody mainstream has built — and everything in
§3 is a down-payment on it.

---

## 3. The Twelve Ideas — Feasibility & ROI

Feasibility scale:  **⬤ Wire-up** (machinery exists, assemble it) · **◐ Build** (new
pass, known shape, bounded risk) · **○ Research** (genuine open problems).

Each card: *what it means here* · *have / net-new* · *feasibility* · *ROI — speedup and
where* · **Unlocks** (the capability it buys us, beyond speed) · **Frontier** (what is
genuinely undone, versus the prior art in §2.1). ROI figures are **upper bounds** (the
full representation tax recovered); real gains are net of guard and deopt cost.

### 3.1 ⬤◐ Idea 2 — Alias-state stencils (the money idea)

**What it means here.** Make alias/ownership state a first-class JIT specialization
dimension alongside type and shape. The inline-cache key becomes
`value-type + collection-shape + alias-state`. The same `seq[i]` lowers differently for
`UniqueVec` (raw load), `FrozenVec` (raw + CSE-able), `BorrowedSlice` (raw + bounds
fact), or `SharedRefCell` (full managed path). The crown jewel is **stenciled alias
collapse**: detect a hot region where a `Rc<RefCell<Vec<T>>>` is *effectively unique*
(refcount == 1, unborrowed, no escaped alias, shape stable) and compile its body as if it
were `&mut Vec<T>` — C semantics inside, managed semantics outside, a guarded bridge
between.

**Have.** `AliasInfo`, `loop_handles_definitely_distinct`, `readonly` (`&[T]`
eligibility), ownership (Give vs Show), escape analysis, the speculative region JIT
(`region_hot`), and precise region-grade deopt. **Net-new.** Region-entry guard set on
the alias facts (`DeoptIf{RefcountChanged, Borrowed, Escaped}`) and the raw-load stencil
family inside the collapsed region.

**Feasibility.** ⬤ engine / ◐ the pass. **ROI — the largest lever.** This is the JIT
form of codegen's O1/O2/O3, which the notes say "addresses nearly every loss." Directly
attacks **knapsack 4.56×, nbody 3.70×, matrix_mult 1.54×, mergesort 1.72×,
counting_sort 1.21×, two_sum 1.29×** — plausibly most to **~1.0–1.2× C**, and the geomean
"decisively below 0.97×."

**Unlocks.** A memory-safe, dynamic language gets to *temporarily wear C's clothes*. The
programmer writes the ordinary managed `Seq`; no `unsafe`, no lifetime annotations, no
ownership ceremony in user code — and the hot region still runs as `&mut Vec<T>`. Safety
is never surrendered; the runtime *borrows it back* for the duration of a guarded region
and hands it straight back on exit. That is a different deal than Rust's (prove
uniqueness forever, statically) or a tracing JIT's (never collapse the box at all).

**Frontier.** Every serious JIT specializes on `type + shape` (V8 hidden classes, Graal).
Making **alias/ownership state a third, co-equal specialization dimension** — and
collapsing managed→raw under a *temporary*-uniqueness guard rather than a global proof —
is the part we cannot find in any mainstream system. This is the money shot.

### 3.2 ◐⬤ Idea 1 — Proof-carrying stencils

**What it means here.** Pair every optimized fragment with the **theorem that makes it
legal**: stencil id, patched holes, required facts, guard set, deopt stencil,
invalidation dependencies, optional certificate. The JIT becomes a *semantic cache* that
can answer "what fact enabled this raw load? what guard protects this elided bounds
check?"

**Have.** `OracleFacts` (per-expression, arena-keyed), kernel-checked e-graph
certificates, `ProofCertificate` with claim digests. **Net-new.** A per-stencil
*precondition record* threading the relevant `OracleFacts` to the region, and reuse of
facts as IC keys.

**Feasibility.** ◐ the plumbing / ⬤ the proof sources. **ROI — an enabler, not a direct
speedup.** Its value is that it *multiplies* the others: it lets the optimizer be
aggressive (Idea 2's collapse) without losing soundness, and makes failed guards
*teachable* (feeds Idea 11).

**Unlocks.** A queryable, auditable "why" for the entire JIT. Today an optimized fragment
is opaque — you can read the machine code but not the *argument* that justified it. With
the theorem attached, every elision is introspectable and every speculation is
falsifiable; debugging a miscompile becomes "which premise was wrong," not "stare at
disassembly." It also turns soundness from a hope into a checked invariant.

**Frontier.** Proof-carrying code ships a proof *next to* a binary as a load-time gate.
**Fusing the certificate into the copy-patch stencil** — so the JIT patches the code and
the proof obligation together, from the same kernel that proves the language's logic —
has not been done. Our kernel already mints these certs for e-graph rewrites; pointing
that machinery at the JIT is the novel step.

### 3.3 ⬤ Idea 3 — Deopt-graph stencils

**What it means here.** Treat deoptimization structure as something to stencil:
`reconstruct frame`, `materialize virtual object`, `rebox scalar-replaced object`,
`resume at pc=N`, `invalidate dependent code`. Deopt is half the runtime; cheap reliable
deopt lets the optimizer be braver.

**Have.** Precise/region-grade deopt with `NativeFrame` materialization is *already*
shipped — we resume in bytecode exactly where the native path bailed, with no
mid-flight side effects. **Net-new.** Make the deopt path itself stencil-shaped with
holes (`class_shape`, `fields_from_registers`, `resume_pc`) rather than bespoke per
fragment.

**Feasibility.** ⬤. **ROI — indirect.** Unblocks the aggression that makes Idea 2 and
Idea 4 safe; near-zero standalone speedup.

**Unlocks.** Makes speculation *cheap to author and mechanically reliable*, which changes
the optimizer's risk appetite: we will speculate in places we currently wouldn't dare,
because the cost of being wrong drops from "hand-write bespoke recovery metadata" to
"emit a deopt stencil." Aggressive optimization is gated by deopt cost; lower the cost and
the whole optimizer gets braver.

**Frontier.** Deopt is half a real runtime's complexity, yet it is almost always bespoke
per-fragment metadata. Treating **deopt structure itself as a synthesizable stencil
family** (`materialize object`, `rebox scalar`, `resume at pc=N`) is rarely done — and
it is the precondition for trusting Idea 2's managed↔raw bridge.

### 3.4 ◐ Idea 4 — Layout stencils

**What it means here.** Let the runtime *choose object layouts that match available
stencils* — re-stencil the heap. A struct repeatedly accessed `obj.x; obj.y; obj.z`
migrates to a contiguous unboxed-triple layout with direct-offset load stencils. Code
templates and heap layouts co-design each other.

**Have.** Collection-shape facts, struct field information, the `with_capacity`
inference. **Net-new.** Shape-migration on profile + the contiguous/unboxed load stencil
family. This is codegen's **O3 small-`Seq` scalarization** at runtime.

**Feasibility.** ◐. **ROI — concentrated and large where it applies.** **nbody 3.70× →
near parity** is the poster child: 5 bodies in parallel `Seq`s of compile-time-known size
that should live in registers. Also struct-heavy workloads.

**Unlocks.** The heap reshapes itself to the code instead of the reverse. "Structure of
arrays," unboxed field triples, and register-resident small collections *emerge from the
profile* — the programmer never rewrites their data model to chase cache behavior. Data
representation becomes a runtime decision, not a source-level commitment.

**Frontier.** Hidden classes specialize *access* to a layout the runtime was handed.
**Letting the runtime migrate the layout to match the stencils it already has** — code
templates and heap layout *co-designing each other* — inverts the usual direction and is,
as far as we can tell, undone in production.

### 3.5 ◐ Idea 5 — Stenciled partial evaluator (the derivative engine)

**What it means here.** Make the PE residualize into a fixed menu of **residual
stencils** (`StaticBranchElim`, `KnownCalleeInline`, `KnownShapeLoad`,
`EscapeAnalyzedAllocation`, `DynamicGuardedResidual`) instead of arbitrary IR. Each PE
decision becomes: select residual form, patch holes, attach guard/proof/deopt edge.
Partial evaluation stops feeling like a compiler pass and becomes an
*interpreter-to-stencil transducer* — §2's spine, made concrete.

**Have.** The full PE (`partial_eval.rs`), BTA (`bta.rs`), and **all three Futamura
projections**. This is the one place we are *uniquely* positioned versus any other
runtime. **Net-new.** The residual-stencil menu and the binding-time → stencil mapping.

**Feasibility.** ◐, but uniquely feasible for us. **ROI — correctness and extensibility
over raw speed.** Replaces the hand-maintained 125 variants with *derived* ones (P2);
the long-term payoff is that new fact dimensions cost a specializer, not a hand-written
stencil explosion.

**Unlocks.** The JIT grows *by derivation*. Adding a new fact dimension (a new alias
state, a new effect class) stops meaning "hand-write another combinatorial stencil
family" and starts meaning "teach the specializer one rule and let P2 emit the variants."
The 125 register-threading variants we maintain today become *output*, and the
correctness of the JIT reduces to the correctness of the interpreter it was derived from.

**Frontier.** Partial evaluation deriving compilers is Futamura's result and Graal's
practice. A PE that **residualizes specifically into copy-patchable proof/deopt
stencils** — the interpreter-to-stencil *transducer* — is, per the literature, not a
standard production architecture. This is §2's spine made executable, and we are the only
codebase with all three projections already passing to attempt it.

### 3.6 ⬤ Idea 6 — Effect stencils

**What it means here.** Specialize *effects*, not just types: `PureCallKnownTarget`,
`NoThrowCall`, `AllocNoEscapeCall`, `CallWithGlobalVersionGuard`. Many optimizations are
blocked by *effect* uncertainty, not type uncertainty — e.g. reusing the first `a.x`
across `f()` in `a.x + f() + a.x` requires knowing `f` cannot mutate `a`.

**Have.** The full `EffectSet` lattice (`reads/writes/allocates/io/security_check/
diverges/unknown`). **Net-new.** Effect-keyed call stencils + using effect facts to
license redundant-load elimination across call sites in the JIT.

**Feasibility.** ⬤. **ROI — broad but modest.** Unblocks load-reuse in call-heavy hot
loops; complements Idea 2 rather than standing alone.

**Unlocks.** Reasoning that is blocked by *effect* uncertainty rather than type
uncertainty becomes mechanical. "`f()` cannot touch `a.x`, so the load is reusable across
the call" is the kind of inference a programmer makes by eye and a JIT normally cannot;
with effect-keyed stencils it falls out of the `EffectSet` we already compute.

**Frontier.** Type specialization is universal; **effect state as a first-class stencil
dimension** (pure / no-throw / alloc-no-escape / global-version-guarded calls) is rare.
Cheap, because the lattice already exists.

### 3.7 ◐○ Idea 7 — Scheduler / register-allocation stencils

**What it means here.** Stencil pre-scheduled, microarchitecture-specific *chunks*
(`Zen4_LoadPairThenFMA`, `AVX2_ReductionTree`, `AVX512_MaskedLoopTail`) with holes for
physical registers, vector width, and tail policy — so the JIT picks from a menu instead
of doing real instruction scheduling.

**Have.** The 125 register-threading location variants are the *first instance* of this
idea (operands pinned to r0–r3 vs frame). **Net-new.** SIMD/microarch schedule families —
which is exactly **offline synthesis (Minotaur-style) territory**, so it pairs with Idea
11.

**Feasibility.** ◐ for the scalar menu / ○ for verified SIMD synthesis. **ROI — kernel
workloads:** matrix_mult, nbody, spectral_norm, parsing/search loops.

**Unlocks.** Microarchitecture portability *without writing a scheduler*: ship a stencil
pack per chip family and let the JIT pick pre-scheduled chunks. The hardest, most
chip-specific part of a backend becomes data, not code.

**Frontier.** Minotaur already synthesizes verified SIMD sequences offline. The undone
half is the **runtime feeding an evolving stencil supply** (Idea 11) — the online demand
signal closing the loop with offline synthesis.

### 3.8 ⮕ Idea 8 — GC-barrier stencils → refcount-traffic stencils

**What it means here.** We have **no tracing GC** — memory is `Rc` reference counting
plus `RefCell` borrow flags. So the barrier analogue is **refcount-traffic stencils**:
`StoreNoBarrier` (proven unique), `StoreNoRefcountBump` (proven non-escaping), elide the
borrow-flag check. This is *not a separate idea* — it **is** alias collapse (Idea 2)
viewed from the memory-management side.

**Feasibility / ROI.** Fold entirely into Idea 2. Noting it separately only clarifies
that our "write barrier" cost is `Rc` inc/dec + borrow-flag set, which §0's losses pay on
every element.

**Unlocks / Frontier.** Nothing standalone — but the reframing is the useful insight: the
GC-barrier literature's `young/old`, `card-mark`, `remembered-set` taxonomy maps cleanly
onto our `unique / shared / borrowed / escaped` refcount states, so the same
"specialize the barrier by proven object state" discipline applies. We just have a
different barrier to elide.

### 3.9 ◐○ Idea 9 — Inline-cache family stencils

**What it means here.** A combinatorial IC library (`MonomorphicShapeLoad`,
`ShapeTransitionAndStore`, `PrototypeChainGuard`) where the IC itself is a mini
copy-patch compiler that patches a new cache stencil on object-model evolution.

**Have.** `ObservedKind` guards. **Net-new.** Hidden classes / shape transitions — which
we largely *don't have*, because **LOGOS dispatch is far more static than JavaScript's**.

**Feasibility.** ◐/○. **ROI — honestly lower for this language.** The polymorphic-dispatch
tax that ICs exist to kill is small here; we should say so and not over-invest. Worth it
only if/when dynamic dispatch surfaces (CRDT/closure-heavy code).

**Unlocks / Frontier.** The one genuinely new angle — *the IC as a mini copy-patch
compiler that patches a fresh cache stencil on each object-model transition* — would make
shape evolution literal binary patching. Real, but it solves a problem LOGOS mostly
doesn't have. Park it.

### 3.10 ◐ Idea 10 — Factful abstract-machine interpreter

**What it means here.** Stencil an abstract machine whose state is already
`Bytecode + TypeState + ShapeState + AliasState + EffectState` — a proof-carrying
execution engine. Specializing *this* (§2) yields guarded optimized code directly.

**Have.** All the fact-state pieces exist as Oracle outputs; they are simply not yet
*woven into the interpreter's state*. **Net-new.** The enriched interpreter
representation.

**Feasibility.** ◐. **ROI — an enabler / the substrate** for Ideas 1 and 5. Not a
standalone number; it is the principled foundation that makes the derivation in §2 real
rather than ad hoc.

**Unlocks.** It makes Futamura *dramatically* more powerful. Specializing a bare
interpreter gives you a compiler; specializing a *proof-carrying* interpreter gives you a
**guarded, optimized, self-justifying** compiler — the profile/proof/fact layer is the
weapon, not the bytecode. `PE(factful_interp, program + profile + proofs)` is a strictly
stronger machine than `PE(interp, program)`.

**Frontier.** Interpreters carry `(pc, stack, locals)`. Enriching that state to
`(…, facts, guards, dependencies, deopt-map)` *and then partially evaluating it* — turning
the interpreter into a proof-carrying execution engine before specialization — is the leap
that none of the prior systems take.

### 3.11 ◐○ Idea 11 — Stencil farms

**What it means here.** Offline, continuous synthesis: collect hot traces, canonicalize,
cluster, synthesize+verify better templates, ship a stencil pack. Programs produce
stencil *demand*; the farm produces stencil *supply*; the runtime gets smarter over
time. This is the home for SMT/superoptimization — **offline**, never on the runtime path.

**Have.** `logicaffeine_synth` (offline Z3 specs/synthesis), the e-graph, and the EXODIA
Phase 2/5 direction (Souper/Minotaur-style). **Net-new.** The demand→supply pipeline and
the "re-specialize on fact-delta" recompiler (§2's derivative).

**Feasibility.** ◐ harness / ○ at scale. **ROI — compounding long-tail**, not a single
benchmark.

**Unlocks.** A runtime that *gets faster the longer it runs and the more programs it
sees*. The deopt stream — every guard that failed on real data — becomes training data
for the farm: a failed assumption is a demand signal for a stencil that wouldn't have
failed. The runtime stops being a fixed artifact and becomes a learning one.

**Frontier.** Souper and Minotaur prove offline synthesis pays. The **closed market** —
runtime demand → offline synthesis → verified stencil pack → runtime consumption,
recurring — is the undone loop, and it is the literal realization of §2's "derivative on
a fact-delta."

### 3.12 ○ Idea 12 — Whole-language-tower stencils

**What it means here.** Every layer of the 17-crate tower emits/consumes stencils:
`source → bytecode → factful bytecode → residual stencil IR → machine stencils →
proof/deopt stencils → layout/barrier stencils`. A self-applicable stencil machine.

**Feasibility.** ○ (the boss fight). **ROI — an organizing principle, not a deliverable.**
The hard research problem is making the stencil spaces *compose without exploding*. This
is the north star §2 points at, achieved incrementally by shipping Ideas 1–6.

**Unlocks.** One mental model — *emit/consume stencils* — across every layer of the tower,
so the lexer-to-machine-code pipeline becomes a single substrate instead of seven
disjoint systems. **Frontier.** This *is* the "self-applicable stencil machine" that, per
§2.1, no one has built. We don't build it directly; it is what Ideas 1–6 *add up to*.

### Summary table

| # | Idea | Feasibility | Primary ROI |
|---|------|-------------|-------------|
| **2** | **Alias-state stencils (collapse)** | ⬤◐ | **Biggest lever** — knapsack, nbody, matrix, mergesort, counting_sort, two_sum |
| 4 | Layout stencils | ◐ | nbody 3.70× → ~parity; structs |
| 1 | Proof-carrying stencils | ◐⬤ | Enabler — multiplies 2/4; soundness |
| 3 | Deopt-graph stencils | ⬤ | Enabler — braver optimizer |
| 5 | Stenciled PE (derivative engine) | ◐ | Correctness + derived variants |
| 6 | Effect stencils | ⬤ | Cross-call load reuse |
| 10 | Factful interpreter | ◐ | Substrate for 1/5 |
| 7 | Scheduler/regalloc stencils | ◐○ | SIMD kernels |
| 11 | Stencil farms | ◐○ | Compounding long-tail |
| 8 | GC-barrier stencils | ⮕ 2 | (folds into alias collapse) |
| 9 | Inline-cache family | ◐○ | Low for this language |
| 12 | Whole-tower | ○ | North star, not a task |

---

## 4. Recommendation — Build Order

**First project: Idea 2, stenciled alias collapse.** Highest ROI, the machinery already
exists, and it attacks the measured losses head-on. At strategy altitude the region
stencil shape is:

```
EnterUniqueRegion        // guard: refcount == 1 ∧ unborrowed ∧ no escaped alias ∧ shape stable
  RawVecLoop             // body compiled against &mut Vec<T>, raw loads/stores, no borrow/bounds
ExitUniqueRegion         // restore managed invariants
  DeoptIfRefcountChanged
  DeoptIfBorrowed
  DeoptIfEscaped
```

The uniqueness is **not proven globally and forever** — it is *assumed for a hot region
and guarded*, with the existing region-grade deopt as the fallback. This is why it is
feasible now: it rides the speculative-region tier (`region_hot`), the entry-guard
mechanism, and `NativeFrame` materialization we already ship. It is backed by **Idea 1**
(the uniqueness fact + its guard, as a proof-carrying precondition) and **Idea 3** (the
guarded bridge expressed as deopt stencils).

Then, in order:

1. **Idea 4 — layout stencils** (nbody scalarization; second-worst loss).
2. **Idea 6 — effect stencils** (cheap, broad, ⬤).
3. **Idea 5 — stenciled PE** (the long-term derivative engine; pays back §2's promise of
   *derived* rather than hand-written variants).
4. **Idea 11 — stencil farm** (turn the fact-deltas from deopts into new supply).

**Deprioritize Idea 9** (low ROI for a statically-dispatched language) and treat **Idea
12** as the framing, not a task.

---

## 5. How We'd Measure

Every claim above is falsifiable against the existing harness
(`benchmarks/results/local-logos-vs-c.json`, i9-14900K, gcc 13.3.0, 10 runs). Targets:

| Benchmark | Current vs C | Target (idea) | Pass/fail signal |
|-----------|--------------|---------------|------------------|
| knapsack | 4.56× | ~1.1× (2: collapse + bounds) | `panic_bounds_check == 0` in region; borrow count → 0 in inner loop |
| nbody | 3.70× | ~1.0× (4: scalarize) | bodies in registers; `borrow()` count per (i,j) → 0 |
| mergesort | 1.72× | ~1.2× (2 + scratch reuse) | alloc count per level → O(1) |
| matrix_mult | 1.54× | ~1.05× (2: collapse) | 0 borrows / 0 bounds checks per FMA |
| counting_sort | 1.21× | ~1.05× (2) | scatter store is a raw store |
| two_sum | 1.29× | ~1.1× (2 on map handle) | map borrow hoisted out of loop |
| **geomean** | **0.97×** | **decisively < 0.90×** | overall |

---

## 6. Honesty & Risks

- **Estimates are upper bounds.** They assume the *entire* representation tax is
  recovered. Real gains are net of guard checks at region entry and deopt churn when a
  guard fails on real data. A region that deopts every iteration is *slower* than the
  managed path — the tier trigger and guard placement must be conservative.
- **"Derivative" is a metaphor with one rigorous anchor** (incremental recompilation =
  finite differencing over the fact-environment, §2). It is *not* automatic
  differentiation; do not let the slogan outrun the mechanism.
- **Representation collapse must preserve observable semantics.** Compiling
  `Rc<RefCell<Vec>>` as `&mut Vec` inside a region is only sound if the guards are
  complete — a missed aliasing path is a miscompile, not a slowdown. The safety net is
  **`logicaffeine_tv` (translation validation)**: prove the collapsed region is
  observationally equivalent to the managed body before trusting it, and gate it behind
  the same differential testing (`vm_outcome` vs `tw_outcome`) the VM already uses.
- **Stencil-space explosion** (Idea 12) is the real long-term risk. Idea 5 (derived
  stencils via P2/P3) is the mitigation: generate the variants, don't hand-write them.

---

*The next-level slogan, in three beats:*

1. *Stencil the moments where managed semantics collapse into raw-machine semantics under
   guard — so a safe, dynamic language can wear C's clothes for the length of a hot
   region without the programmer giving up a thing.*
2. *Use the Futamura tower to **derive** those stencils rather than hand-write them — the
   only codebase with all three projections passing is the one that can.*
3. *Stop treating the JIT as a code generator. It is a **semantic cache** — code is one
   cached artifact beside proofs, guards, layouts, alias facts, and deopt recipes — and a
   failed guard is not a failure but a new fact. That is the part nobody has built yet.*
