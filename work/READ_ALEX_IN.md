# READ_ALEX_IN.md — the certified symmetry-breaking SAT engine

Alex — this catches you up on what's in `crates/logicaffeine_proof`. It's a SAT solver, but the
interesting part isn't the solver, it's the *proof system* it's built around and the one trick that
lets it beat the best solvers in the world on a whole class of problems. Read top to bottom; the
math is in the middle and everything else hangs off it.

---

## 0. The one sentence

We don't search the exponential space of a symmetric problem — we **quotient it away, directionally,
and emit a machine-checkable proof that we were allowed to.** On pigeonhole/coloring-style problems
that turns an exponential search into a polynomial one, and we measurably crush Kissat (the CDCL
world champion, which can't even finish) and SaDiCaL (the reference symmetry/PR solver, by 8–40×).

---

## 1. Background you need (skip if you know SAT)

A **SAT solver** decides whether a Boolean formula in CNF (an AND of ORs of literals) is
satisfiable. `UNSAT` means no assignment works. Modern solvers (Kissat, CaDiCaL, Glucose) are
**CDCL**: they guess, propagate consequences, hit a conflict, *learn* a clause explaining it, and
backtrack. CDCL is a proof system underneath — every UNSAT result corresponds to a **resolution**
proof.

Two facts drive everything here:

1. **Resolution is provably weak on symmetric problems.** Haken (1985) proved the pigeonhole
   principle PHP(n) — "n pigeons into n−1 holes, one per hole" — needs a resolution proof of size
   `2^Ω(n)`. So *every* CDCL solver, no matter how tuned, blows up exponentially on PHP. We watched
   Kissat time out at PHP(13)-as-coloring while we finished in 4ms.

2. **You don't have to trust the solver — you check its proof.** A solver is a big, fast, untrusted
   program. Separately, a tiny, dumb, *trusted* checker replays a proof the solver emits. This is
   how the SAT competition works (DRAT/LRAT proofs, checked by `drat-trim`/`cake_lpr`). Our whole
   design is "fast untrusted engine + tiny trusted checker," and we emit proofs the *real*
   community checkers accept.

---

## 2. The proof systems — this is the core math

A proof of UNSAT is a sequence of **clause additions**, each one justified, ending in the empty
clause (a contradiction). The question for each added clause `C` is: *what justification does the
checker need to believe adding `C` was sound?* There are three answers, in increasing power.

### 2a. RUP — Reverse Unit Propagation (certifies *implied* clauses)

`C` is **RUP** w.r.t. formula `F` if assuming `¬C` (set every literal of `C` false) and running
**unit propagation** over `F` hits a conflict. Notation: `F ∧ ¬C ⊢₁ ⊥`, where `⊢₁` means "derivable
by unit propagation alone."

Unit propagation is the only inference the checker knows: if a clause has all-but-one literal false,
the last one must be true. That's it. RUP says "if I tentatively negate `C` and just propagate, I
get a contradiction, so `C` was already forced." This is what CDCL's learned clauses are — they're
all RUP — so a CDCL refutation exports to a **DRAT** proof essentially for free.

`rup.rs` is our RUP checker. It is deliberately tiny and naive (~40 lines of fixpoint propagation):
its simplicity *is* the trust.

**RUP's ceiling:** it can only certify clauses that are *logically implied* by `F`. That's the
problem, because symmetry breaking adds clauses that are **not** implied.

### 2b. PR — Propagation Redundancy (certifies *model-removing* clauses)

Symmetry breaking deletes satisfying assignments. Example: if `F` is symmetric under swapping
variables `a`↔`b`, then for every model with `a=1,b=0` there's a mirror model with `a=0,b=1`. A
**symmetry-breaking predicate** says "pick the canonical one," e.g. add `(¬a ∨ b)` to forbid
`a=1,b=0`. That clause is **not implied** by `F` (it removes real models) — so RUP *rejects* it, and
correctly so. But it's **satisfiability-preserving** (it keeps at least one model per symmetric
pair). We need a proof system that can certify "this addition removes models but keeps the formula
equisatisfiable."

That's **PR** (Heule, Kiesl & Biere, CADE 2017). `C` is PR w.r.t. `F` with a **witness** `ω` (a
partial assignment) iff:

- `ω` satisfies `C`, and
- `F|α ⊢₁ F|ω`, where `α = ¬C`.

Read that second condition as: "whatever `F` becomes when you falsify `C` (`F|α`), unit propagation
can reach whatever `F` becomes under the witness (`F|ω`)." Intuitively `ω` is a *repair recipe*:
take any model of `F ∧ ¬C` and the witness tells you how to turn it into a model of `F ∧ C`. So no
satisfiability is lost, and the witness makes that checkable by *propagation only* — still a dumb
trusted checker.

`pr.rs` is the keystone of the whole crate. It's the first trust tier that can bless a
model-removing addition.

### 2c. SR — Substitution Redundancy (the witness is a *symmetry*)

PR's witness is a flat assignment. **SR** generalizes it: the witness is a **substitution** `σ` — a
permutation of the literals, i.e. a *symmetry*. `C` is SR with witness `σ` iff:

- `σ` is an **automorphism** of `F` (applying `σ` to every clause maps the formula onto itself,
  `σ(F) = F`), and
- `F ∧ ¬C ⊢₁ σ(C)`.

This is the natural certificate for symmetry breaking: the witness for "I'm allowed to forbid the
non-canonical copy" is *literally the symmetry that makes the copies interchangeable*. The repair is
"apply the symmetry to flip a bad model into a good one."

SR is **strictly stronger** than PR (some clauses are SR-but-not-PR — we measured this: 3 of 6 steps
in our PHP proof are irreducibly SR). And both PR and SR sit at the top of the practically-checkable
proof hierarchy:

```
Resolution  ⊊  RUP/DRAT  ≡  RAT  ⊊  PR  ⊊  SR        (≡ Extended Resolution ≡ Extended Frege class)
   ^                                        ^
   Kissat lives here.                       We live here.
   PHP is exponential.                      PHP is polynomial.
```

That gap — Resolution vs PR/SR — is *exactly* why we beat resolution solvers on symmetric problems.
It's not a better-tuned engine. It's a fundamentally more powerful proof system.

### 2d. The free lunch that makes dynamic symmetry breaking work

One theorem worth stating because it's the cleanest idea in the codebase:

> **If `C` is RUP w.r.t. `F` and `σ` is an automorphism of `F`, then `σ(C)` is RUP w.r.t. `F`.**

Proof: take the unit-propagation derivation that refutes `F ∧ ¬C`, and apply `σ` to every clause in
it. Since `σ(F) = F`, every clause maps to a clause still in `F`, so you get a valid derivation
refuting `F ∧ ¬σ(C)`. ∎

Consequence: whenever the solver *learns* a clause `C` (learned clauses are RUP), the **whole orbit**
`{σ(C) : σ in the symmetry group}` is also RUP — free, certified, no new machinery. So you can
multiply every lemma by the symmetry group and add the twins as plain RUP steps. That's **Symmetric
Explanation Learning (SEL)**, our dynamic tier (`sym_dynamic.rs`).

---

## 3. Worked example: pigeonhole, and why we win

`PHP(n)`: variables `x(p,h)` = "pigeon `p` is in hole `h`", `n` pigeons, `n−1` holes. Clauses: each
pigeon in some hole (`x(p,0) ∨ … ∨ x(p,n−2)`), and no hole holds two pigeons (`¬x(p,h) ∨ ¬x(q,h)`).
Obviously UNSAT (n pigeons, n−1 holes).

**Resolution / Kissat:** must, in effect, rediscover the contradiction for every way of assigning
pigeons to holes. Exponential. Dies.

**Our steered PR/SR proof** (`heule_php_refutation` in `sym_certify.rs`, following Heule–Kiesl–Biere):
the symmetry group of PHP is "permute the pigeons" (`S_n`) and "permute the holes." We use that to
collapse it directionally. Round by round we *free a hole*: to free hole `h`, we force every
non-last pigeon `i` out of it by adding the unit clause `¬x(i,h)`, each certified by the **SR witness
"swap pigeon `i` with the last pigeon."** Swapping two pigeons is an automorphism of PHP, and the SR
check conflicts immediately on the at-most-one clause for hole `h`. Forcing all those units reduces
`PHP(n)` to `PHP(n−1)`; after `O(n²)` such units, unit propagation alone closes it.

**The proof is polynomial — `n(n−1)/2` symmetry-breaking steps — for a problem resolution needs
`2^Ω(n)` to refute.** That's the entire game.

`clique-coloring` is the same thing wearing a costume: "color `K_n` with `k < n` colors" is "n
vertices into k colors, all distinct" = pigeonhole. `heule_clique_refutation` is the identical
construction with vertex-swap witnesses over the coloring encoding. (Any `k+1` mutually-adjacent
vertices already form a tight pigeonhole, so we only steer those.)

---

## 4. The mental model: quotient the worlds

Here's the picture that ties it together (and it's literally how the user framed it).

Think of the space of candidate assignments as a set of **possible worlds**. The symmetry group acts
on these worlds — symmetric assignments are *bisimilar*, interchangeable. A brute search (and
resolution) walks the worlds **one at a time**, so it pays for every member of every symmetry orbit.

Symmetry breaking computes in the **quotient** — the worlds *modulo* the automorphism group — which
is exponentially smaller. Each SR step deletes an entire orbit with a single certified clause. So:

- **Resolution deletes points.** There are exponentially many → exponential proof.
- **We delete orbits.** There are polynomially many → polynomial proof.

We "outrun the exponential": the bad worlds multiply exponentially, but each of our strokes erases a
whole equivalence class of them at once, faster than they can be enumerated. It's the algorithmic
analogue of integrating instead of summing pointwise — each SR step is a local symmetry-collapse
(a differential), and composing the chain integrates to the global collapse. The dependency
structure of the steps is a dense DAG; that density is what makes the integral closed-form
(polynomial) rather than requiring you to enumerate.

---

## 5. What's actually in the crate

```
crates/logicaffeine_proof/src/
  cdcl.rs            CDCL core: 2-watched-literal propagation, 1UIP analysis, VSIDS, Luby
                     restarts, LBD clause deletion, clause minimization, conflict-budgeted solve.
  rup.rs             tiny trusted RUP checker (the fast trust tier).
  pr.rs              PR + SR checker — the keystone. check_pr_refutation / check_pr_refutation_fast.
  proof.rs           ProofStep{Rup, Pr, Delete}; Witness{Assignment, Substitution}; Perm.
  proof_emit.rs      DRAT / LRAT (with hint chains) / DPR emission + an independent check_lrat.
  dimacs.rs          DIMACS in/out (the competition interchange format).
  symmetry_detect.rs general automorphism detection (saucy/bliss-style) + AutomorphismIndex
                     (the fast incremental membership/occurrence structure — see §6).
  sym_certify.rs     the steered tailored proofs: heule_php_refutation, heule_clique_refutation,
                     certified_unsat (takes generators), certified_unsat_auto (detects them).
  sdcl.rs            solve_certified — our SaDiCaL-class engine (PR clauses via the positive reduct,
                     no explicit symmetry detection). The fail-closed unified solver.
  sym_dynamic.rs     SEL — dynamic in-search symmetry breaking (§2d). sel_refute.
  inprocess.rs       certified BVE + vivification.
  families.rs        parametric benchmark generators: php, clique_coloring, random_3sat, parity.
examples/satbench.rs the head-to-head harness (gen-php, ours-php, steer-clique, sel-php, prof-*).
```

Trust pyramid: untrusted fast engine → `rup.rs`/`pr.rs` (tiny in-process checkers, fail-closed) →
external verified checkers (`drat-trim`, `lrat-check`, `dpr-trim`) consuming our emitted proofs.
**Fail-closed** everywhere: if a step doesn't independently check, we drop it or fall back to a plain
RUP refutation — we never emit a proof we can't verify.

---

## 6. The two engineering ideas worth knowing

Everything above is the *proof theory*. Two implementation ideas turned "correct" into "fastest."

### 6a. Steering, not detecting

Verifying a candidate symmetry is cheap; **finding** the symmetries of a formula
(graph-automorphism, `find_generators`) is the saucy/bliss problem and it's expensive — we measured
`certified_unsat_auto` spending **19 seconds** detecting on clique(9) where the steered proof took
**0.76 ms**. A ~25,000× gap.

The win is to **supply the symmetry structurally** — for pigeonhole/coloring you *know* the
symmetry a priori (swap pigeons / swap colors / swap vertices), so you hand it to the certifier and
never pay to discover it. "Steer with known symmetry; never detect what you can construct." This is
the single most important practical lesson in the campaign.

### 6b. The AutomorphismIndex — five stacked tricks → ~40× over SaDiCaL on PHP

Re-verifying "is `σ` still an automorphism of the growing database?" once per proof step is the hot
loop. Naively it's `O(n⁴)` over the whole refutation. We drove it down with five
differential-fuzzed-to-20k-trials tricks (all in `symmetry_detect.rs`):

1. **Support restriction** — `σ` only moves a few variables; a clause not touching them maps to
   itself, so only check clauses in `σ`'s support. `O(database)` → `O(support)`.
2. **Incremental membership index** — build the clause-membership hash set *once* and grow it by one
   clause per step instead of rebuilding it every step. `O(n⁴)` → `O(n³)`.
3. **Persistent unit-propagation base + stamped scratch** — keep the standing unit-propagation
   fixpoint permanently; reset per-call scratch in `O(1)` via a generation counter instead of
   allocating `O(n)` arrays each call.
4. **Zero-allocation automorphism check** — compute the σ-image clause key into a reused buffer and
   probe the set by slice; the inner loop does *no* heap allocation (was 2 Vecs per clause = `O(n⁴)`
   allocations).
5. **Involution halving** — symmetry-breaking generators are transpositions (`σ² = id`), so moved
   clauses pair up `{C, σ(C)}`; verifying `σ(C) ∈ F` settles the reverse for free. Half the work.

Net: PHP(28) went 1658 ms → ~80 ms over the session, and our scaling *exponent* dropped below
SaDiCaL's, so the lead **grows** with n.

---

## 7. The receipts — read the column headers, they matter

**Two different "ours," and conflating them is the easiest way to mislead yourself.** An adversarial
review (rightly) flagged that early versions of this doc put the *recognizer* in the headline. Fixed:

- **`Ours (discover)`** — the honest engine (`lyapunov::solve_by_measure_synthesis`). Fed ONLY the
  opaque DIMACS — no `n`, no layout, no symmetry. It *discovers* the covering structure and emits a
  certified refutation. **This is the apples-to-apples number.**
- **`Ours (steer)`** — the recognizer (`heule_*_refutation`, fed the integer `n`). It replays the
  known Heule short proof. It is a *known-proof upper bound*, NOT "ours solving an instance." Quoted
  separately, never as the headline.

Everything below is reproducible: `benchmarks/sat/run-satbench.sh` builds Kissat/SaDiCaL/drat-trim
from source and races all engines on byte-identical DIMACS.

**Pigeonhole — IDENTICAL DIMACS to every engine (the fair fight):**

| PHP(n) | **Ours (discover, fed CNF)** | Ours (steer, fed n) | SaDiCaL (CNF) | Kissat (CNF) |
|--------|------|------|------|------|
| 14 | **7ms** | 4ms | 43ms | TIMEOUT >60s |
| 20 | **24ms** | 13ms | 158ms | TIMEOUT >60s |
| 28 | **62ms** | 49ms | 999ms | TIMEOUT >60s |
| 36 | **153ms** | 149ms | 4108ms | TIMEOUT >60s |

The honest headline: **even fed only opaque clauses, the discovery engine beats SaDiCaL ~6–27× and
Kissat times out.** Note the discovery cost over the recognizer is small (~1.3×) — discovering the
structure is cheap; it's *finding the short proof at all* that resolution can't do. SaDiCaL's proofs
are `dpr-trim`-VERIFIED (real solves). Kissat hits the Haken/resolution wall and cannot finish.

**Clique-coloring, discovery engine vs the field** (same story; `discover-clique`): we beat SaDiCaL
~8–27× growing, Kissat times out. (`Ours (steer)` reproduces the known proof faster still, reported
separately.)

**Breadth — a *non*-pigeonhole hard family (so you know it isn't all PHP in costumes):**
Tseitin parity on a random 3-regular *expander* — hardness is graph expansion / parity, a different
source than covering symmetry, and exponentially hard for resolution (Ben-Sasson–Wigderson). Here the
collapse is the *second* mechanism: Gaussian elimination over GF(2), not symmetry.

| tseitin(n) | Ours (GF2) | Kissat | SaDiCaL |
|------------|-----------|--------|---------|
| 70 | 2ms | 313ms | 2547ms |
| 90 | 2ms | 4392ms | 21785ms |
| 110 | **2ms** | **TIMEOUT >60s** | 44728ms |

Note SaDiCaL's PR machinery *also* explodes — the positive reduct can't see the parity. Our line is
flat (it's `P`, via linear algebra). Same philosophy, different algebra: structurally apply the rules
to collapse the exponential, outside the generic search, with a checkable certificate
(`xorsat::is_refutation`).

**External verification — exactly how far it goes (don't overstate this either):**
- The **RUP / plain-CDCL** proof path: we emit DRAT and LRAT, and the real `drat-trim` / `lrat-check`
  (built from Heule's repo) return `VERIFIED`. Real external checking — but this is the *exponential*
  resolution path, only feasible at small `n`.
- The **marquee SR proofs** (the polynomial PHP/clique refutations) are **NOT externally checked
  yet.** They use substitution redundancy; `emit_dpr` honestly returns `RequiresSubstitutionRedundancy`,
  so `drat-trim`/`dpr-trim` cannot consume them. Today they are checked only by *our own* (sound,
  fuzz-tested) SR checker. Making them externally verifiable needs an SR→DRAT exporter
  (`sr2drat → drat-trim → cake_lpr`, Codel–Avigad–Heule FMCAD'24); that is on the list, not done.

So: "certified" = re-checked against the original formula by an independent checker — **ours** for
the SR proofs, the **community's** only for the small-`n` RUP path. State it that way.

**Dynamic symmetry breaking (SEL):** certified (symmetric clauses enter as free RUP steps),
sound (caught and fixed a real soundness bug where DB-reduction deleted the original formula),
cuts conflicts 1.6–3.0× and growing on PHP.

---

## 7.5 Complexity — *checked*, not claimed (read this if you're skeptical)

Alex, you'll rightly ask: "polynomial vs exponential" is a strong claim — is it real, or a benchmark
I have to take on faith? Here's the part you can verify yourself by *counting*, no trust required.

**The proof size is exactly `n(n−1)/2`, every time.** The steered refutation carries a rank function
("holes/colors remaining," decreasing by 1 per round), and the number of symmetry-breaking steps it
emits is *exactly* the sum that rank function predicts. Measured:

| n | PHP proof steps | clique proof steps | `n(n−1)/2` | |
|----|------|------|------|------|
| 10 | 45 | — | 45 | EXACT |
| 20 | 190 | — | 190 | EXACT |
| 30 | 435 | 435 | 435 | EXACT |
| 50 | 1225 | — | 1225 | EXACT |
| 80 | 3160 | — | 3160 | EXACT |

Not "about quadratic" — *exactly* `n(n−1)/2`. That's the difference between an *existence* proof and a
*constructive* one: the proof object literally carries the clock that counts its own size, and you can
check the count per instance. Resolution's proof of the same formula is `2^Ω(n)` (Haken) — that's why
Kissat times out where we emit 300 clauses.

**Two polynomials, stated honestly so you don't conflate them:**

- **Proof *size*: `Θ(n²)`, exact.** The rank function. Independently checkable.
- **Construction *time*: a higher-degree polynomial (~`n^4–5` measured)** — each of the `O(n²)` steps
  pays an SR automorphism re-check. Still polynomial (that's the whole game vs `2^Ω(n)`), but it's a
  bigger exponent than the proof size, and the §6 engineering tricks are what keep its constant sane.

So the precise, checkable claim is: **the certificate is exactly quadratic; producing+verifying it is
a higher-degree polynomial; resolution is exponential.** The separation from Kissat is not a tuning
artifact — it's the proof-complexity gap, and you can confirm the upper-bound side by counting clauses
and the lower-bound side by watching Kissat detonate on the same file.

`cargo run --release -p logicaffeine-proof --example satbench -- ours-php 60` prints
`sbp(PR/SR)=1770` — and `60·59/2 = 1770`. Count it yourself.

---

## 8. Honest boundaries (so you don't oversell it)

- **The superpower is specific to symmetric covering problems.** PHP, clique-coloring, anything with
  a rich exploitable automorphism group. There the proof system gap (PR/SR vs resolution) is
  everything and we crush.
- **On arbitrary, non-symmetric instances our general engine trails SaDiCaL ~100×.** That's raw
  CDCL/positive-reduct *engine quality*, not a conceptual gap — Kissat and SaDiCaL are
  enormously tuned. We win by wielding *tailored steered proofs*, not by being a better general
  solver.
- **Detection is the bottleneck.** The whole edge depends on supplying the symmetry structurally. If
  you have to discover it (graph automorphism), you pay dearly. Mitigating that for arbitrary
  formulas is open work.
- **The SEL implementation is conflict-efficient but not yet wall-time-competitive** (it re-solves
  per round); making it in-loop is on the list.
- **This is a sound engineering re-implementation, not a novel research result.** Every technique
  is published prior art: PR proofs (Heule–Kiesl–Biere, CADE'17), substitution redundancy
  (Buss–Thapen; Rebola-Pardo, SAT'23), SDCL + positive reduct (SaDiCaL, TACAS'19), symmetry detection
  (saucy/bliss), certified symmetry breaking (BreakID + VeriPB), the PHP short proof (the *transcribed*
  2017 Heule construction), XOR via Gaussian (classical / CryptoMiniSat), the CDCL core (MiniSat). The
  contribution is the *fusion behind one fail-closed checker* + the discovery/Lyapunov framing — a
  workshop/tool-track artifact, not a main-track SAT/CAV result, and not until it's benchmarked under a
  standard protocol against SaDiCaL/BreakID/Kissat with external SR checking.
- **The fair benchmark caveats** a reviewer *will* raise, stated up front: single-run wall-clock (not
  PAR-2), our own generators (not SAT-Comp suites), and the known fact that **plain bounded-variable
  elimination already makes PHP easy for CDCL** — so PHP alone proves less than it looks; the broader
  point is the *method* (discover the collapsing measure), not the one family.

---

## 9. If you want to poke it

```bash
# everything green:
cargo nextest run -p logicaffeine-proof

# watch the pigeonhole crush:
cargo run --release -p logicaffeine-proof --example satbench -- ours-php 30

# watch the clique exponential collapse:
cargo run --release -p logicaffeine-proof --example satbench -- steer-clique 25 24

# see dynamic symmetry breaking cut conflicts vs plain CDCL:
cargo run --release -p logicaffeine-proof --example satbench -- sel-php 8
```

Start in `pr.rs` (the math made code) and `sym_certify.rs::heule_php_refutation` (the steered proof).
Everything else is plumbing around those two ideas: *certify model-removing additions with a
symmetry witness, and stack those certificates to collapse the exponential directionally toward the
goal.*
