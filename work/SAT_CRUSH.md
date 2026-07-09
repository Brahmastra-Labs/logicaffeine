# SAT_CRUSH — plan to win the SAT Competition (and the adjacent comps)

Living campaign doc. The scoreboard is `benchmarks/arena/` (PAR-2 vs Kissat 4.0.4 + CaDiCaL 3.0.0 on
byte-identical DIMACS, in-process model verification, verdict cross-check). Every lever is flag-gated,
A/B'd on the harness, re-validated against the brute-force oracle (cdcl.rs:1005–1147) and `drat-trim`.
Rule: **never trade correctness for speed** — the cross-check stays green.

## STATUS — EGaussian watch layer + carry-the-contradiction DPLL(XOR) (2026-06-30)

Built and validated the incremental GF(2) engine end-to-end — the "watch layer" for live XOR
reasoning during search:
- **Occurrence index** (`xor_engine.rs` `IncXor.occ`): variable → rows-containing-it, so each
  `assign`/`re_pivot`/`unassign` touches only the rows mentioning the variable (par32 RREF is sparse,
  avg row weight ~17 of 3176), not all rows. Validated by the differential oracle + a direct
  `check_occ` invariant (80k random ops, exact match).
- **Incremental unit/conflict detection** (`weight` + `low` queue): `state()` reads only the rows that
  became units/conflicts since the last call — O(touched), not O(rows). Same differential gate, with
  `weight`/`low` superset invariants asserted every step.
- **Carry-the-contradiction theory integration** (`cdcl.rs` `solve_with` + shared `after_conflict`):
  every clause a theory derives is a globally-valid no-good, so it is CARRIED into the learned DB —
  a theory conflict drives a normal 1-UIP backjump, a theory propagation enqueues with the carried
  clause as its reason. This REPLACED the old path that injected theory clauses as *permanent*
  originals (an unbounded clause-DB death-spiral: par16 hung >30s). Gated by a new 400-instance
  differential test (`solve_with([IncXor])` verdict ≡ plain `solve()`), brute-force oracle still green.
- Live DPLL(XOR) wired into `hybrid_xor` behind `LOGOS_XOR_LIVE` (with `LOGOS_XOR_KERNEL` /
  `LOGOS_XOR_SEED` A/B knobs). After the fix it SOLVES par16 live (was a hang); 504 lib tests green.

**par32 — measured, honest verdict: no symmetry shortcut exists.** par32-1-c = 372 width-2 + 4882
width-3 clauses → `extract_xor` recovers all 1158 XOR equations (~186 width-2 equivalences + ~972
width-3 parity gadgets), leaving ~994 *residual* ternary clauses over a **157-dimensional GF(2)
kernel** (par32-1: 412-dim). The kernel is **linearly rigid**: depth-1 probing of every kernel
variable (full BCP + Gaussian) collapses it 47→47 (par16) and 157→156 (par32-1-c) — the 39
"equivalences" probing finds are already Gaussian-implied, so folding them back adds nothing. par is
also automorphism-rigid. So par32's hardness is a genuine search over ~994 ternary constraints on the
kernel (minimal-disagreement parity = parity + an at-most-k counting core), NOT a hidden symmetry.
Live DPLL(XOR) does the parity half but does not yet beat clause-mining on it; the remaining lever is
CryptoMiniSat-grade search tuning + native cardinality reasoning on the residual, not a one-shot
symmetry break. Kept `LOGOS_XOR_LIVE` flag-gated as the validated foundation; mining stays the default.

## STATUS — structural dispatcher live (2026-06-29)

`crates/logicaffeine_proof/src/solve.rs::solve_structured` is wired into the `logos-sat` binary: a
battery of CHEAP O(clauses) fail-closed recognizers (2-SAT → Horn → pigeonhole/Hall → cutting-planes
→ parity → covering-collapse) then CDCL. Generic graph-automorphism `find_generators` + SEL are OFF
the hot path (they don't scale: 57s on a 1359-var circuit; SEL spins on easy SAT) — they are
DISCOVERY tools only. Net: **never-worse than plain CDCL by construction**, and a **two-speed solver**:
- **CRUSH** (ms vs competitors' timeout) on structured families with matching/parity/counting/covering
  structure — confirmed in-arena: `php22` ours 0.01s, Kissat & CaDiCaL TIMEOUT. (`gen-structured.sh`
  adds PHP/clique/Tseitin to the corpus so the arena shows this.)
- **COMPETITIVE** (≈CDCL≈Kissat) on everything without that structure.

**Classes we genuinely lose / don't crush, and the honest handling:**
1. **`par32` (parity-LEARNING, SAT, hard for everyone incl. Kissat)** — WINNABLE: build a hybrid
   XOR-SAT handler (`lyapunov::extract_xor` recovers the linear system from clauses → `gf2::solve_gf2`
   gives the GF(2) solution space → search the affine coset for a model of the residual clauses).
   This is a "pick a class we lose and win it" — everyone times out, so cracking it is an outright win.
   Safe to build (no touching the sibling's `pr.rs`/`symmetry_detect.rs` churn).
2. **Random 3-SAT (uf/uuf250)** — NO structure to exploit (rigid, asymmetric); the *only* lever is
   raw CDCL search quality (rephasing / adaptive restarts / VSIDS heap). Reaches PARITY, not crush —
   stated honestly. This is the engine track.

**Measurement honesty:** committed page numbers (and this session's arena) are box-CONTENTION-inflated
~10× (clean PHP(40)=154ms vs committed 1485ms → real lead over SaDiCaL ~45×, not ~4×). Re-run both on
a quiet box before publishing. External validation column = DPR→`dpr-trim`, but honestly labeled: ✓
only where the proof reduces to PR; the SR marquee proofs need `sr2drat`→drat-trim→cake_lpr.

## Where we stand (measured 2026-06-29, i9-14900K)

- **97 real SATLIB instances: ours solved 95/97 — the identical set to Kissat AND CaDiCaL.** The 2
  misses (par32) time out for everyone at 20s. Zero verdict conflicts; all 46 SAT models verified.
- Solved-only time: **ours 1.48× Kissat, 1.08× CaDiCaL** — already basically tied with CaDiCaL.
- **We BEAT Kissat on every application + crafted family** (ratios 0.34–0.99: blocksworld 0.36×,
  logistics 0.34×, jnh 0.38×, aim 0.48×, ssa/bf 0.8×).
- **The entire gap is random-3SAT** — uf250 2.74×, uuf250 1.34× — and it dominates the total
  (39.6 of our 40.6 solved-seconds).

## What we lose to, and why

**PRIMARY — satisfiable instances, where the game is FINDING a model, not refuting one.**
The SAT-random slowdown is far worse than the UNSAT-random slowdown, and it explodes with scale:

| | uf250 (SAT) | uuf250 (UNSAT) |
|---|---|---|
| n=250 ours/kissat | 2.74× | 1.34× |
| n=300 (sample) | **60×** (6.07s vs 0.10s) | ~2× (9.96s vs 5.11s) |

Refutation (1UIP + LBD clause learning) is healthy — that's why UNSAT stays ~2×. **Solution-directed
search is the weakness.** We are missing exactly the machinery modern solvers use to converge on a
model:
- no **rephasing / target phases / best-phase tracking** (the single biggest SAT-instance lever);
- **VSIDS via an O(n) linear scan** (`pick_branch` cdcl.rs:639), no heap, no EVSIDS/VMTF;
- **Luby-only restarts** (cdcl.rs:705) — no adaptive LBD restarts, no restart blocking;
- no **stable/focused mode switching** (the Kissat architecture that pairs restart+phase regimes).

**SECONDARY — per-conflict throughput + formula size (bites at scale / industrial).** Not yet the
bottleneck at ≤300 vars, but it will dominate at the 10⁴–10⁶-var competition scale:
- `Vec<Vec<Lit>>` clause DB (cache-poor) vs a packed clause arena + 32-bit refs;
- **no live-DB inprocessing** — we search the raw formula while Kissat shrinks it first
  (subsumption, SSR, BVE, vivification, probing);
- single-tier LBD clause deletion (no CaDiCaL-style tiers/used flags).

## The plan — ranked by measured leverage

**P1 — Solution-directed search (kills the primary, measured gap).** Highest ROI; each lever is a
self-contained change, oracle-validated, A/B'd on uf/uuf + the scale ladder.
1. **Rephasing + target/best-phase saving** — track the best partial assignment, periodically reset/
   flip saved phases. Expected to collapse the SAT-random gap most.
2. **VSIDS heap** — replace the O(n) `pick_branch` scan with a d-ary heap (also a throughput win).
3. **Adaptive LBD/Glucose restarts + restart blocking on trail size** — replace Luby-only.
4. **EVSIDS/VMTF + stable/focused mode switching** — the regime that ties it together.

**P2 — Per-conflict throughput.** Packed clause arena (flat `Vec<u32>` + 32-bit refs), reworking
`watches`/`reason`/`reduce_db`. The dominant industrial-scale lever once P1 lands.

**P3 — DRAT-certified inprocessing (also our differentiator).** Promote `inprocess.rs` to a scheduled
inprocessor over occurrence lists: subsumption + SSR, scheduled BVE, live-DB vivification,
failed-literal probing + equivalent-literal substitution (reuse `twosat.rs` SCC), blocked-clause
elimination — every step emitting DRAT (we already do this for BVE/vivify; competitors struggle to).

**P4 — Clause-DB tiers + scale tuning.** CaDiCaL-style core/mid/disposable + used flags; tuned reduce
schedule; memory budget + arena compaction; continuous runs on larger + real competition instances.

**P5 — Proof completeness, then expand to all arenas.** `sr2drat` so our structured wins (PHP/parity/
symmetry) count in proof-required tracks. Then the other comps where assets already exist: Incremental
(IPASIR shim over `solve_under_assumptions`), No-Limits, Pseudo-Boolean (`pseudo_boolean.rs`), MaxSAT
(incremental + `cardinality.rs`), then Model Counting / SMT-QF_BV (`bitblast.rs`) / QBF.

## How we measure "crushing"
Grow the arena corpus toward the competition's: add larger SATLIB + real SAT-Competition application
instances, the random scale ladder (`gen-scale.sh`), and PAR-2 + cactus. A lever ships only when it
(a) keeps the oracle + cross-check green, (b) keeps every UNSAT DRAT `drat-trim`-valid, and (c) moves
PAR-2 the right way. Target the random ladder to parity first (P1), then hold parity as instances scale.
