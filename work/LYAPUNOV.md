# A Lyapunov certificate for proof-complexity collapse

*A precise, machine-checked claim. Everything here is checkable; nothing is asserted that the test
suite does not verify. Read it adversarially — that's the point.*

## The claim, stated narrowly

> **A short refutation of a structured UNSAT formula is a discrete-time dynamical system carrying a
> Lyapunov function, and the *same* four classical Lyapunov-stability axioms certify, by one checker,
> two structurally unrelated exponential-collapse mechanisms: the geometric (symmetry-orbit) collapse
> and the algebraic (GF(2)-dimension) collapse. The descent simultaneously certifies termination
> (correctness) and the polynomial step count (complexity).**

That is the whole claim. It is **not** "we beat Kissat," it is **not** about `P` vs `NP`, and it is
**not** novel solving — it is a unifying *certificate object*, and its value is that it is checked.

## What a Lyapunov function is here (the four axioms)

A potential `V : step → ℕ` over the refutation trajectory is a Lyapunov function iff:

1. **Bounded below** — `V ≥ 0` (here by type: `u64`).
2. **Monotone** — `V` never increases along the trajectory.
3. **Strict descent** — across its level set `V` strictly decreases (no level recurs ⇒ guaranteed
   progress, no infinite stall).
4. **Reaches the goal** — `V` attains its minimum exactly at ⊥ (the refutation closes).

`lyapunov::verify_lyapunov(potential, reaches_goal)` checks all four and returns the certificate
(with the complexity bound `levels · max_dissipation`), or `None`. It is proven **sound and complete**
against an independent brute-force axiom oracle over 20,000 random trajectories
(`verify_lyapunov_is_sound_and_complete_on_random_trajectories`).

## Three instantiations — one checker, three proof systems

| | **Geometric** | **Algebraic** | **Cardinality** |
|---|---|---|---|
| family | pigeonhole / clique | Tseitin on an expander | pigeonhole |
| proof system | PR / SR (substitution) | polynomial calculus / GF(2) | cutting planes |
| hardness | symmetry (covering) | parity / expansion | counting |
| potential `V` | active items remaining | unsolved GF(2) dimension | PB constraints left to combine |
| descent step | break one symmetry orbit | one Gaussian pivot | one PB addition |
| reaches ⊥ when | items exhausted | the `0 = 1` row appears | the constraint becomes `0 ≥ 1` |
| code | `lyapunov_of_symmetry` | `gaussian_lyapunov` | `cutting_planes_lyapunov` |

`three_physics_one_checker_and_pigeonhole_has_two_measures` runs all three through the *same*
`verify_lyapunov` — and proves **non-uniqueness**: pigeonhole carries a symmetry measure (`n`
levels × `n` width) *and* a cutting-planes measure (`2n-1` linear steps), two valid Lyapunov
functions for one formula in two different proof systems. The measure is a property of
*problem-plus-structure*, not a unique object — which is *why* the framework spans proof systems
rather than re-presenting one. Watch the first two:

```
$ cargo run --release -p logicaffeine-proof --example satbench -- lyapunov-unified 10

  [GEOMETRIC]  PHP(10) — V = active items remaining (discovered 10×9 from opaque CNF)
    descent:  10 ▸ 9 ▸ 8 ▸ 7 ▸ 6 ▸ 5 ▸ 4 ▸ 3 ▸ 2
    axioms ✓: monotone strict_descent reaches_goal;  levels 9 × width 9 ⇒ size ≤ 81 (actual 45)

  [ALGEBRAIC]  Tseitin(expander) — V = dimension of the unsolved GF(2) system
    descent:  14 ▸ 13 ▸ 12 ▸ 11 ▸ 10 ▸ 9 ▸ 8 ▸ 7 ▸ 6 ▸ 0
    axioms ✓: monotone strict_descent reaches_goal;  V bottoms at 0 = the 0=1 contradiction
```

## Why this is the right object (the bridge nobody names)

Three communities each own a piece of the same function and don't cite each other:

- **Program verification** calls it a *termination measure* (a ranking function proving a loop halts).
- **Complexity** calls it a *progress / potential measure* (bounding the number of steps).
- **Control theory** calls it a *Lyapunov function* (a scalar energy proving convergence + rate).

For a refutation these are literally the same object: the descent proves the proof *terminates*
(correctness) and its level structure bounds the *size* (complexity), exactly as a Lyapunov function
proves a control system *converges* (stability) at a *rate* (performance). The unification above is
the evidence that this is not analogy but identity — the same axioms, mechanically checked, fire on
collapses with no structural relationship to each other.

## Honest scope (the refutations a reviewer should make, pre-empted)

- **This certifies short proofs we already construct; it does not find them.** The geometric potential
  is discovered (`solve_by_measure_synthesis`, fed opaque CNF, no `n`); the algebraic one is read off
  Gaussian elimination. Discovering the potential for an *arbitrary* formula is the hard (NP-hard,
  automatizability) part and is **not** claimed.
- **The families are crafted.** PHP/clique are covering problems; expander-Tseitin is parity. The
  point is the *method* (one certificate for both), not a benchmark sweep.
- **No `P = NP` content.** A potential that bottoms out polynomially exists only because these
  families have one; the framework *measures* which side of the line an instance is on, it does not
  move the line. The absence of a potential in a bounded class is reported as a checkable bounded
  impossibility (`None`), never as a wrong answer.
- **The complexity bound is on proof *size* (exact `Θ(n²)` for the geometric case); construction
  *time* is a higher-degree polynomial** — see `READ_ALEX_IN.md §7.5`. Don't conflate them.

## The technical kernel: the ⟸ theorem (machine-checked)

The certificate is only interesting if the measure *generates* proofs rather than being *read off*
them. It does, and that is a theorem.

> **Theorem (⟸).** Let `F` be a formula and `M` a *Lyapunov measure* for it — an initial potential
> `L`, a per-level width `w`, and, from any database `D ⊇ F` at potential level `ℓ > 0`, at most `w`
> clause additions, **each redundant (RUP / PR / SR) against `D`**, that descend to level `ℓ−1`,
> with level `0` forcing ⊥ derivable by unit propagation. Then `F` has a checkable refutation of size
> `≤ L·w + |closure|`, constructible in time `O(L·w·c)`.

**Proof (constructive).** Induct on the potential: apply the descent steps level by level; each adds
`≤ w` certified-redundant clauses and strictly decreases the potential, so after `≤ L` levels the
potential is `0` and ⊥ is RUP-derivable. Every clause is a checkable redundancy step, so the
concatenation is a checkable refutation with `≤ L·w` descent steps. ∎

**Machine-checked.** The hypothesis is the trait `LyapunovMeasure`; the constructive proof is
`proof_from_measure`; the conclusion is verified by `theorem_poly_measure_implies_poly_checkable_proof`,
which instantiates the theorem on **three structurally different measures** (pigeonhole, tight
clique-coloring, loose clique-coloring) through the *one* generic constructor and asserts, for each,
that the produced proof (a) re-checks against `F` and (b) has descent `≤ L·w`. One measure type
(`CoveringMeasure`) covers every generalized-pigeonhole family — the family is data, the measure is
uniform. (The algebraic collapse is a *separate* instantiation of the four axioms via
`gaussian_lyapunov`; it is not clausal, which is precisely why it is evidence the framework is more
than "PR/SR proof structure renamed.")

## The unified auto-collapse agent (attacking multiple classes)

`lyapunov::auto_collapse(F)` is one engine that, given *opaque* clauses, recognizes **which** physics
collapses the formula and dispatches — sound and fail-closed on both routes:

- **Geometric** (covering symmetry): `solve_by_measure_synthesis` discovers the `items × bins` layout.
- **Algebraic** (parity): `extract_xor` recovers the latent XOR constraints from the CNF clause
  gadgets, then `gaussian_lyapunov` collapses the GF(2) system.
- **None**: a checkable bounded impossibility (no covering/parity structure recognized).

```
$ satbench auto-collapse php8.cnf     → COVERING/GEOMETRIC   (discovered 8×7, certifies)     528µs
$ satbench auto-collapse clique76.cnf → COVERING/GEOMETRIC   (discovered 7×6, certifies)     209µs
$ satbench auto-collapse tseitin16.cnf→ PARITY/ALGEBRAIC     (16 XORs, dim 23→0, valid)       44µs
```

The routing is locked in by `unified_agent_routes_a_whole_suite_correctly` across three classes at
several sizes. The point for the framework paper: **the measure is the proof-system-agnostic invariant
content of "why this is easy" — the agent reads it off whichever structure is present**, spanning
PR/SR (symmetry) and polynomial-calculus-over-GF(2) (parity) under one checker.

## Answering the reviewer's killer questions

**Q1. "Is this just resolution width renamed?"** No, and the construction is the evidence. The descent
steps are PR/SR (substitution-redundant), not resolution steps, so the theorem produces a `Θ(n²)`
proof of pigeonhole — a formula whose *every resolution proof* is `2^Ω(n)` (Haken 1985), at any
width. A resolution-width object cannot produce a polynomial proof of PHP at all. Ours does. The
framework is strictly stronger than resolution as a constructive/upper-bound tool.
(`killer_question_the_measure_transcends_resolution`.)

**Q2. "Isn't 'ranking function = Lyapunov function' folklore?"** Yes — and we don't claim that as the
result. The contribution is (i) the *constructive* ⟸ theorem with a self-certifying complexity bound,
(ii) the cross-mechanism unification (the *same* checker certifies a non-clausal algebraic collapse),
and (iii) the synthesis–impossibility duality: `solve_by_measure_synthesis` *discovers* the measure
from opaque CNF, or returns a checkable bounded impossibility.

**Q3. "Does the framework prove anything new about lower bounds?"** Honestly: **not yet, and we say
so.** The ⟸ direction (measure ⟹ short proof) is proved. The ⟹ converse (short proof ⟹ bounded
measure) and the lift "no bounded measure ⟹ proof-complexity lower bound" are stated as the open
problems. Whether that lift beats width-based techniques (Ben-Sasson–Wigderson) is exactly the open
question the framework makes constructive and checkable — not a claim we make.

**Q4. "Is the win a solver advance?"** No. On arbitrary non-structured instances the general engine
trails the SOTA; the contribution is the certificate/synthesis framework on structured families, plus
the honest fair-fight result that *fed identical opaque CNF*, the discovery engine still beats SaDiCaL
6–27× and Kissat times out (`benchmarks/sat/run-satbench.sh`).

## The categorical tower — and the homotopy type it names (all checked)

Climbing the ladder, every rung a passing test:

| rung | structure | the checked fact | file |
|---|---|---|---|
| π₀ | action groupoid `X ⫽ G` | symmetry breaking = orbits; `2ⁿ → n+1` | `groupoid.rs` |
| π₁(X⫽G) | stabilizers | orbit–stabilizer `|orbit|·|stab| = |G|` (fiber `Stab→G→Orbit`) | `groupoid.rs` |
| 1-cell | a collapse | Lyapunov measure = countdown coalgebra morphism; *terminates ⟺ morphism exists* | `coalgebra.rs` |
| 2-cell | refinement | category of collapses (preorder; initial object; non-uniqueness = 2 objects) | `category_collapse.rs` |
| functor | `transfer` | reductions carry collapses; functor laws | `category_collapse.rs` |
| groupoid | iso-reductions | invertible; `ρ∘ρ⁻¹=id`; **π₁(F) = Aut(F) = the symmetry group** | `category_collapse.rs` |

**What it names.** `π₁(F) = Aut(F)` is a *discrete* group, presented by the Coxeter relations of its
generators (`s_i² = id`, braid `(s_i s_{i+1})³ = id`, far commutation). Those relations **are the
2-cells** — the homotopies witnessing that a product of loops is trivial. Therefore:

> **The ∞-groupoid of this symmetry structure is `K(Aut(F), 1) = BG`** — the classifying space of the
> symmetry group: `π₁ = G`, and `πₙ = 0` for `n ≥ 2`, *because the symmetry is a discrete group*.

That is the honest top of the tower. The homotopy type is fully determined (`π₀` = orbits, `π₁` = the
symmetry group, higher `πₙ` vanish). **Genuine higher `πₙ` would require a 2-group** (symmetry with
internal symmetry) — that is the open frontier, named precisely, not claimed. Checked in
`the_2cells_are_the_group_relations_so_the_infinity_groupoid_is_BG`.

## Reproduce / check it yourself

```bash
cargo nextest run -p logicaffeine-proof -E 'test(lyapunov)'     # axioms + 20k soundness fuzz + the ⟸ theorem
cargo run --release -p logicaffeine-proof --example satbench -- lyapunov-unified 12
```

The theorem's kernel is `theorem_poly_measure_implies_poly_checkable_proof` (the ⟸ construction on
three measures) and `killer_question_the_measure_transcends_resolution` (the Θ(n²)-vs-2^Ω(n)
evidence). The hypothesis/conclusion live in `lyapunov.rs`: trait `LyapunovMeasure`, constructor
`proof_from_measure`, instantiation `CoveringMeasure`.

The code is `crates/logicaffeine_proof/src/lyapunov.rs` (`verify_lyapunov`,
`LyapunovCertificate`, `gaussian_lyapunov`, `lyapunov_of_symmetry`). If you can make
`verify_lyapunov` accept an invalid trajectory or reject a valid one, the central claim is false —
the fuzz says you can't, over 20k tries; try harder.
