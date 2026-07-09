# PROOF_SKETCH — the instrument map

Companion to `PAPER.md`. The paper is the study; this document is the map between its results and the
P vs NP question: which results touch the question, through which theorem, where the connection stops,
and which directions remain open to these instruments. **The core focus is §7b — the attack
outline**: the question decomposed into named, certified cells (existence PROVEN, checkability
PROVEN, per-family polynomial bounds CERTIFIED by construction, plant recovery EXACT, the size
obligation localized as the one remaining cell), so that every future step lands somewhere
specific. §5 states the formal barriers; §7 the program that survives them. Nothing here resolves
P vs NP in either direction.

---

## 1. What a proof of P = NP would actually require

P = NP means: some NP-complete problem (say SAT) has a **polynomial-time algorithm**. A proof would have to
exhibit such an algorithm (or a non-constructive argument that one exists) and prove its running time is
`O(n^k)` for a fixed `k`, on *worst-case* inputs, as `n → ∞`.

Three things follow immediately, all of which matter for us:

- **It is asymptotic.** P vs NP is a statement about how cost scales as `n → ∞`. Any statement about a *fixed*
  finite `n` is vacuous (a fixed finite problem is `O(1)` by table lookup). Several of our results are about
  finite `n`; none of them says anything about the asymptotic question. "P = NP for finite N" is not a theorem,
  it is a category error.
- **It is worst-case.** A fast algorithm for *structured* or *symmetric* instances says nothing; NP-hardness
  lives in the worst case.
- **We produce no algorithm.** Nothing in this repository is a candidate polynomial-time SAT algorithm.

## 2. The proof-complexity connection (the only bridge our tools touch)

Our tools are about **proof complexity**, which connects to P vs NP through one theorem:

> **Cook–Reckhow (1979).** NP = coNP if and only if there is a *polynomially bounded* propositional proof
> system — one in which every tautology has a polynomial-size proof.

Since P is closed under complement, `P = NP ⟹ NP = coNP`. So proving `NP ≠ coNP` (by showing *every* proof
system has a family of tautologies with only super-polynomial proofs) would give `P ≠ NP`. This is the
**proof-complexity program**, and it is a program toward **P ≠ NP**, pursued by proving lower bounds for
stronger and stronger proof systems. It is *not* a route to P = NP.

Our tools operate at **two rungs** of the ladder. The lower-bound instruments work on
**Nullstellensatz (NS)** and **Polynomial Calculus (PC)** over `GF(2)` — a relatively weak algebraic
system where lower bounds are actually provable, now with re-checkable dual witnesses, certified
resolution-width certificates, and the stabilized symmetric machinery that decides invariant-witness
existence at every scale. The upper-bound engine works in the **PR/SR (substitution-redundancy)
fragment** — `Resolution ⊊ RUP/DRAT ⊊ PR ⊊ SR`, the Extended-Resolution/**Extended-Frege class** —
where `pr.rs`/`sdcl.rs`/`sym_certify.rs` search for and check proofs, externally verified through
`sr2drat → drat-trim` (pigeonhole to `PHP(18)`, a 591k-line expanded DRAT). Where the *program* is
stuck is proving **lower bounds** for Frege/EF — nobody has those, and nothing here changes that; but
it is wrong to say these tools sit "far below" the frontier: the search side operates *in* the
EF class, and the census/witness side is what makes its findings checkable.

## 3. What our tools actually establish

| Artifact | What it is | Which direction |
|---|---|---|
| The census (`census.rs`) | Exhaustive proof-complexity map of every minimal-UNSAT formula for small `n`, up to symmetry | Measurement instrument |
| Constructive NS completeness (`build_ns_certificate`) | Every UNSAT formula over `n` vars has a degree-`≤ n` `GF(2)` NS refutation, ∀n, kernel-checked | **Upper bound**, but the certificate space is `2ⁿ` |
| The AIT kernel theorems (`ait_kolmogorov.rs`) | Invariance, Chaitin incompleteness, Gödel corollary as kernel terms — incompressibility is real and unprovable | The other pole of the completeness theorem |
| The degree wall (`max_ns_degree = n`) | Minimum NS/PC degree grows to `n` — reproduces the `Ω(n)` expander degree lower bounds | **Lower bound**, for a weak system |
| Degree lower-bound certificates (`ns_lower_bound_witness{,_polys}`) | A re-checkable dual witness (pseudo-expectation) that a family has *no* degree-`d` refutation — clause and polynomial-generator (linear-encoding, 63-var) engines | **Lower bound**, now certified |
| Resolution-width certificates (`res_width.rs`) | The width-`w` closure as a re-checkable "no width-`w` refutation" certificate, both width conventions | **Lower bound**, certified |
| The stabilized collapsed dual (`orbit_stability.rs`) | Fixed-degree invariant-witness existence decided **for every scale** by one finite computation (entry polynomials + Lucas, kernel-laddered), with the measured char-2 gap | **Lower bound** machinery, ∀-scale |
| The SR/PR engine (`pr.rs`, `sdcl.rs`, `sym_certify.rs`) | Proof *search and checking* in the EF-class fragment; PHP in exactly `m(m−1)/2` SR steps; externally verified via `sr2drat → drat-trim` | **Upper bound** (short proofs), certified + externally checked |
| The separations atlas (`tests/separations_atlas.rs`) | Two-sided rows: certified impossibility + certified proof per family (marquee: `Count_3` GF(2)-hard / GF(3)-easy) | Both directions, per row |
| The characteristic axis (`polycalc_gfp.rs`, `gf3_ring_kernel.rs`) | General `GF(p)`/`GF(4)` NS engine anchored at `p = 2`; prime incomparability certified both ways; `GF(4)` collapses constructively to `GF(2)`; PHP bound characteristic-invariant, witness support on a `p ≥ m` threshold; `p \| \|G\|` annihilation dichotomy with the Reynolds branch verified; `ModCount` ladder rung | **Lower bounds** + measurement, per characteristic |
| The ring axis (`polycalc_zm.rs`) | Howell-form NS over `ℤ/m` (zero divisors), oracle-anchored; completeness over EVERY modulus (structurelessness has no witness over any `ℤ/m`); coprime composite = conjunction of its parts; ring witnesses with `L(1) ≠ 0` + prime-witness lift; `ℤ/4` strictly weaker than `GF(2)` at fixed degree (the nilpotent/Hensel tax); the field composer builds `GF(4)`/`GF(9)` from pieces and exhaustively certifies no field of order 6 or 10 exists (coprime pieces force `e₁·e₂ = 0`) | **Upper bound** (existence pole) + **lower bounds** (ring witnesses) + classification |
| Symmetry cost-cut (`2ⁿ → n+1`) | Under `Sₙ`, the certificate basis collapses to one column per degree | Algorithmic speedup for *symmetric* instances only |
| Kernel-formalized `GF(2)` ring (`mpoly_ring_kernel`) | The `n`-variable multilinear ring and `p·1 = p` proven in the CoC kernel | Verification / rigor |

## 4. Why none of this proves P = NP — and why it points the other way

- The **only** results that touch the P vs NP axis are *lower bounds* (§3, rows 3–4). Lower bounds are the
  **P ≠ NP** direction. They cut *against* a collapse, not toward it.
- The **completeness** result is an *upper bound*, but the certificate lives in a `2ⁿ`-dimensional space
  (`nullstellensatz_basis_size(n, n) = 2ⁿ`). *Existence ≠ efficiency.* An exponential-size proof is not a
  polynomial algorithm. This is the honest one-liner: **structure always exists at finite `n`, and it always
  costs `2ⁿ` for the hard cores.**
- **"Hardness has no witness" — the exact sense in which that is a theorem, and the exact sense in which
  it is not.** Define hardness in its *existence* form: a formula is structureless if no certificate at any
  degree `≤ n` exists over any coefficient ring. That predicate is now machine-refuted **universally**: no
  finite formula fulfills it over any `ℤ/m`, `m ≥ 2` (`no_finite_formula_is_structureless_over_any_modulus`
  — the completeness pole, carried past the fields to every ring). That is a genuine, certified theorem,
  and it is the precise content of "nothing finite is random." But NP-hardness is not the existence
  predicate — it is the **cost** predicate: certificate *size/degree growth along a family, worst-case, as
  `n → ∞`*. That predicate is fulfilled by every family we have certified lower bounds for (PHP's growing
  degree at every characteristic, `Count_p` across the mismatched fields, the ring lifts) — our own
  instruments are the evidence FOR the cost-form of hardness, and the kernel incompressibility theorems
  (§3) prove the cost-form cannot be certified away uniformly. Conflating the two forms is the "P = NP for
  finite n" category error of §1. What we can honestly say, with artifacts: **hardness-as-structurelessness
  provably has no witness; hardness-as-cost provably has witnesses; and the asymptotic question lives
  entirely in the second form, behind the §5 barriers.** The split is now itself machine-checked, rung by
  rung — H_exist unfulfillable over every modulus, H_max (NS-degree exactly `n`) fulfilled at every size
  over `GF(2)`/`GF(3)`/`ℤ/6` simultaneously (by an object that also carries its structure certificate —
  the two poles in one formula), H_grow fulfilled by pigeonhole characteristic-invariantly
  (`tests/hardness_witness_ladder.rs`). H_max is furthermore uniform-with-reason (the Cost-Pole
  Attainment Theorem, PAPER §5.13): kill-or-absorb — every multiplier either dies on `x·(1−x) = 0` or
  absorbs into the clause polynomial by `x·x = x` — verified on every product per ring with the
  per-variable identities kernel-seeded at all four ring classes (`GF(2)`, `GF(3)`, `ℤ/4`, `ℤ/6`;
  `tests/cost_pole_kernel_seeds.rs`), the two branch INVARIANTS kernel-certified `∀n` on the same
  ladder as the completeness pole (`tests/cost_pole_kernel_ladder.rs` — dead-stays-dead and
  absorbed-prefix-is-the-clause-polynomial, with a negative control), and the "nothing finite is
  random" upper half kernel-certified `∀n` with its atom proven where the field axioms fail. The
  P vs NP boundary itself is executable (`tests/pvnp_gunsight.rs`, PAPER §8.2): the weak-system
  collapse routes certifiably closed on PHP (degree + width, growing, re-checked), the same family
  certified CHEAP at the EF-class frontier (`m(m−1)/2` SR steps, zero-trust re-checked) — so the
  named open cell is exactly a family with certified superpolynomial EF-class proof size, and every
  weaker route to a poly-bounded system ends at one of our certificates.
- The **symmetry cut** is exponential → linear *only* for fully-symmetric (`Sₙ`) families — a vanishing slice.
  The generic, `Bₙ`-rigid cores stay at `2ⁿ`. That residue is exactly where worst-case hardness lives, located
  precisely and quantified. Again: consistent with P ≠ NP.

There is no step here — and no combination of these steps — that yields a polynomial worst-case algorithm.

## 5. The barriers that say algebraic tools *cannot* settle P vs NP

Three formal barriers explain why our specific tools cannot resolve P vs NP in *either* direction.
Stating them precisely is part of the result.

- **Relativization** (Baker–Gill–Solovay, 1975): any technique that holds relative to an oracle cannot settle
  P vs NP.
- **Natural Proofs** (Razborov–Rudich, 1997): a "natural" (constructive + large) property of Boolean functions
  cannot prove strong circuit lower bounds, assuming pseudorandom generators exist. Our census-style *structural*
  and *symmetry* arguments are exactly the kind of natural, large, constructive properties this barrier warns
  about — and our own finding that the generic cores are irreducibly `2ⁿ` is the empirical shadow of it.
- **Algebrization** (Aaronson–Wigderson, 2008): techniques that "algebrize" (survive an algebraic oracle
  extension) cannot resolve P vs NP. **Our tools are finite-field algebra — NS/PC over `GF(2)`, and now over
  every `GF(p)` and `GF(4)` (the characteristic axis) — squarely inside the algebrizing class.** Widening the
  field does not move past the barrier: the certified prime incomparability says no single low-degree
  algebraic system suffices, which *sharpens* the hardness picture without touching the boundary. This is the
  sharpest reason our algebraic machinery cannot, on its own, prove P vs NP either way. A resolution needs
  non-algebrizing, non-naturalizing techniques we do not have.

## 6. The genuine result — and how we just made it stronger

The honest, publishable contribution is an **experimental / machine-certified proof-complexity study**:

1. **The exhaustive census** of minimal-UNSAT structure for small `n`, classified by cheapest certified proof.
2. **Constructive NS completeness** (no finite randomness) proven `∀n` by kernel induction, with the honest
   `2ⁿ` cost made explicit.
3. **The `Bₙ`-vs-`AGL` lens-narrowness finding** — the standard symmetry lens under-reports structure; ~92% of
   "rigid" cores carry hidden affine symmetry.
4. **The symmetry cost-cut** `2ⁿ → n+1` under `Sₙ`, with a *sound* reduced certificate.
5. **A kernel formalization** of the `GF(2)` multilinear ring — a verified artifact, not a claim.

**What we built to strengthen it:** re-checkable **degree lower-bound certificates**
(`ns_lower_bound_witness` / `check_ns_lower_bound`). Previously our degree lower bounds were "our solver found no
degree-`d` proof." Now each is a *dual witness* — a degree-`d` `GF(2)` pseudo-expectation `L` with `L(1) = 1`
and `L(m·p_C) = 0` for every generator — which an independent checker verifies, certifying `NS-degree(F) > d`
with zero trust in the solver. That is the standard object on the lower-bound side (the linear-programming /
SoS dual), and it is what makes the lower-bound claims of the paper referee-proof. On top of it we have, all
machine-checked (`parametric_family_has_machine_checked_degree_growth`):

- **Parametric linear growth:** the all-corners cube `F_n` has `NS-degree = n` at each `n`, certified by a
  dual witness that no degree-`(n-1)` refutation exists. (Honest: this bound is *width-driven* — a width-`n`
  family admits no generator below degree `n` — so it is clean but "easy.")
- **A genuine, growing, non-width degree lower bound (the honest core).** Pigeonhole PHP(m) (m pigeons,
  m−1 holes, clause width `≤ m−1`) is a *counting* principle, incomparable to `GF(2)` algebra, so it is
  algebraically hard for Nullstellensatz. We **certify its exact `GF(2)` NS degree**: `NS-degree(PHP(3)) = 4`
  and `NS-degree(PHP(4)) = 6` — each a re-checkable dual witness that there is no degree-`(2m−3)` refutation,
  plus a refutation at `2(m−1)`. The certified degree strictly *exceeds the clause width* (not the trivial
  width bound) and strictly *grows* with `m`. This is the classical `Θ(√n)` pigeonhole degree bound
  (`2(m−1) = Θ(√vars)`), machine-certified per-`m` with independent witnesses — a real proof-complexity lower
  bound, honestly for a weak system. The `∀m` closed form is the Razborov-style theorem; we certify it at
  each computable `m` and measure the pattern continuing (`PHP(5) > 6`).
- **A structural finding the tools surfaced.** Restricting the witness search to a sub-basis
  (`ns_lower_bound_witness_on_basis`), we found that over `GF(2)` the PHP pseudo-expectation is **not**
  supported on partial matchings — the classical characteristic-0 structure fails, blocked by a parity
  obstruction (the pigeon-clause constraint on the matching indicator reduces to `1 + (holes − |m|) ≡ 0
  (mod 2)`). So the `GF(2)` degree lower bound genuinely differs from the field-of-char-0 case and needs a
  parity-aware witness — an honest caution for porting classical proof-complexity lower bounds to `GF(2)`.

## 7. Honest roadmap (what would genuinely advance the paper — none of it is "prove P = NP")

- **Parametric lower bounds with proven growth — DELIVERED, including the uniform `∀m` bound.** Pigeonhole
  PHP has exact `GF(2)` NS degree `2(m−1) = Θ(√n)`, certified per-`m` with dual witnesses (`m = 3, 4`); and we
  now have the **uniform, closed-form, parity-aware witness**: `L(M) = [M hole-injective]` is a valid
  degree-`(2m−3)` `GF(2)` pseudo-expectation for **every** `m`, proving `NS-degree(PHP(m)) ≥ 2(m−1)` by one
  argument — the at-most-one clauses vanish on hole collisions, and each pigeon clause contributes
  `Σ_{S⊆U} 1 = 2^{|U|} ≡ 0 (mod 2)` with `|U| ≥ 1` (the parity that holds over `GF(2)` and fails over char 0).
  Machine-verified at `m = 3, 4`; the argument gives all `m`. This is a genuine uniform proof-complexity lower
  bound.

  **What this is, unambiguously:** a *lower* bound — a **hardness** result, in the P ≠ NP direction. It says a
  weak proof system *cannot* efficiently refute pigeonhole. It is the opposite of an algorithm, and it moves
  us *further* from any P = NP collapse, not closer. Sharper lower bounds are more evidence for P ≠ NP.
- **External cross-checking — DELIVERED.** DRAT/LRAT through `drat-trim`/`lrat-check`; SR through
  `sr2drat → drat-trim` (`PHP(18)` end-to-end); the atlas walks every row through its independent
  checker.
- **A second certified family — DELIVERED.** `Count_3` on the linear encoding: exact degree 2 in the
  dense regime (`n = 4, 5`, proven degenerate), certified degree ≥ 3 at `n = 7, 8` (exact 3), the
  partial-partition witness valid exactly on `n ≡ 3 (mod 4)` (Lucas schedule), and the off-schedule
  scales where **every invariant candidate dies while asymmetric witnesses survive** — the char-2 gap
  as a theorem-grade phenomenon, not a caveat.
- **∀-scale decision machinery — DELIVERED (the stabilization theorem, executable).** Fixed-degree
  invariant-witness existence decided for every scale by one finite computation
  (`decide_invariant_witness_for_all_scales`): PHP-clause `d=2` true at every `m`; PHP-linear `d=2`
  false at every `m` (symmetric reasoning provably blind); `Count_3` `d=2` exactly the mod-4 class.
  Next hardening: kernel-internalize Pascal's recurrence (the two arithmetic lemmas currently ride
  the kernel ladder with computational rungs).
- **Certified resolution-width lower bounds — DELIVERED** (`res_width.rs`), completing the
  three-system PHP row (NS degree / resolution width / SR size).
- **The characteristic axis — DELIVERED** (`polycalc_gfp.rs`, PAPER §§5.6–5.10). A general
  `GF(p)`/`GF(4)` NS engine, differentially pinned to the `GF(2)` engine at `p = 2`, certifies:
  prime incomparability in *both* directions (`Count_3` degree-1/`GF(3)` vs growing/`GF(2)`;
  `Count_2` degree-1/`GF(2)` vs exact `2, 3, 3` staircase/`GF(3)`); the constructive extension-field
  collapse (`GF(4)` certificates λ-project to `GF(2)` — the field ladder is the prime ladder); the
  characteristic-*invariance* of the PHP bound (the hole-injective indicator telescopes
  `(1−1)^{|U|} = 0` in every field — the parity argument is its char-2 shadow; exact `GF(3)` degree
  4 at `m = 3`) against the characteristic-*threshold* law for its support (partial matchings carry
  the witness iff `p ≥ m` — measured across eight (prime, m) points; `GF(2)` is the deepest failure,
  the same small-prime-divides-a-binomial mechanism as the Lucas schedule); the `p | |G|`
  annihilation dichotomy with the Reynolds branch machine-verified (the averaging classical proof
  complexity takes for granted, exhibited at every characteristic coprime to the group); and the
  census ladder's `ModCount { p }` rung, closing the `router_beats_ladder` audit gap with the legacy
  cascade regression-pinned. All lower bounds and measurements — the P ≠ NP direction, still inside
  algebrization (§5).
- **The ring axis — DELIVERED** (`polycalc_zm.rs`, PAPER §5.11). Howell-form NS over `ℤ/m`
  (gcd pivots + annihilator completion, proven against an exhaustive all-combinations oracle and
  against the field engine at prime `m`): completeness certified over **every** modulus `2..12` —
  the "no structureless object" theorem ring-completed (see §4 for its exact scope) — plus the CRT
  conjunction theorem (`ℤ/6 = GF(2) ∧ GF(3)`, `ℤ/12 = ℤ/4 ∧ ℤ/3`: composite moduli intersect, they
  do not add), ring dual witnesses with the `L(1) ≠ 0` zero-divisor normalization and the
  `(m/p)·L_p` prime-witness lift, and the measured nilpotent tax (`ℤ/4` strictly weaker than
  `GF(2)` at fixed degree — parity: degree 2 vs 3 — the Hensel `≤ 2d` lift made concrete).
- **The EF-class probe — instrumented.** `tests/ef_class_probe.rs` points the automatic SR search at
  the mutilated chessboard and threshold random 3-CNF, externally verified where the checkers apply;
  results are measurements either way.
- **The hardness dichotomy — DELIVERED** (`tests/hardness_dichotomy.rs`): pointwise hardness is
  either vacuous (certificate-hardness has no instances — the completeness pole, over every ring) or
  unprovable (incompressibility-hardness is never certifiable of a named object — the Chaitin pole),
  as one kernel-certified conjunction. The companion **retreat ladder — DELIVERED**
  (`tests/hardness_retreat.rs`): six families, each certified hard for one system and certified
  dissolved one rung up — no family in the corpus is hard at its top rung. The one sentence the pair
  leaves open — whether the retreat continues forever — is NP vs coNP itself.
- **No randomness at infinity — DELIVERED in ladder form** (`tests/no_randomness_at_infinity.rs`):
  König rungs + the composition infinite-UNSAT → finite fragment → certified refutation; the `∀k`
  leap kernel-certified. Full internalization of the tree argument is the next hardening.
- **The exact-cover crush route — DELIVERED** (`Route::ExactCover`): exactly-one groups harvested
  from opaque clauses, `Σx = 1` over `GF(2)/GF(3)/GF(5)`, fail-closed re-checked refutations —
  modular counting and the mutilated chessboard fall with zero search.
- **The cofactor-DAG lens — DELIVERED** (`cofactor.rs`, `tests/cofactor_lens.rs`, PAPER §4.3). Symmetry
  *above* the instance: quotient the Shannon cofactor DAG by a congruence that need not fix the formula.
  The collapse measure `quotient_class_count` is monotone and floored by the distinct-cofactor count as
  theorems (`iso ≤ rename ≤ raw = distinct`), and the zero-trust `quotient_dag` certificate realizes the
  load-bearing lemma (poly cofactor classes ⟹ poly, output-sensitive certificate). Two-sided data:
  structured families collapse (XOR linear, PHP `≤ 4m²`, re-checked); **387 of 828 (46.7%) `n = 4`
  residue cores rigid under exhaustive `B₄` *and* every shear nonetheless collapse under CofactorIso** —
  a widespread but small (order-1) symmetry the instance lens misses; and random 3-CNF resists CofactorIso
  entirely (iso tracks the distinct floor, both exponential — the wall). The reframed open cell: **a
  poly-index SR-definable Shannon congruence on the residue** (extension variables relating cofactors
  CNF-isomorphism cannot) — its existence ⟺ `3-SAT ∈ coNP`. Every decidable congruence rung below it is
  certified exhausted.
- **The growth-root law — DELIVERED** (`tests/cofactor_family_counting.rs`, `tests/cofactor_mirror.rs`,
  PAPER §4.4). The cofactor DAG is a finite automaton; its level width is the Myhill–Nerode index of the
  cofactor language (= minimal automaton = best compression of the prefix's "carry"). The growth root is
  set by the carry's structure: a **count / group** carry (poly-size monoid) forces root 1, a **set /
  matching** carry (exponential monoid) forces root > 1, with the exact dial the carry's maximum set
  cardinality `s` via the Lucas sum `Σ_{j≤s} C(⌊n/2⌋, j)` (finite-difference degree `= s`). The root
  depends on the order+quotient, not the function (inner-product: root 1 paired, root 2 separated), and
  the solver dispatcher *is* the library of carry-monoid recognizers (each route a format with a poly
  carry; the residue alone routes `Incompressible`). The residue is incompressible under every *decidable*
  lever — exhaustive `n!` reordering (`~1.46×`, width still grows), the CofactorIso quotient stacked on the
  best order (adds `~0`), and the `GF(2)` incidence rank (an artifact matching random 3-CNF) — each a
  constant factor, none changing the root. So `3-SAT ∈ coNP ⟺` random 3-SAT's cofactor carry bounds to
  `s = O(\mathrm{polylog})` under some order+quotient — the same open cell, now with a named quantity. The
  honest ceiling is enumeration scale: the `s = Θ(n)` dense/expander regime is beyond `distinct_width`'s
  reach, so the shape of the answer is proven, not its asymptotics.
- **Scale the census** with the `AGL` collapse to push the empirical map to larger `n`, and quantify how the
  rigid-core fraction behaves.
- **Frame it as what it is.** An experimental + formally-verified study of proof complexity between
  the two kernel poles — structure always exists; incompressibility is real — with certified
  instruments on both the lower-bound and the EF-class upper-bound side. Legitimate, interesting, and
  true. **Not** titled or framed as a P vs NP result.

## 7b. The attack outline — the exact shape of a P vs NP proof from here, and where every piece stands

**The program's declared target: prove `3-SAT ∈ coNP`.** That is the angle every instrument here
takes — equivalent to NP = coNP by Cook–Levin, decomposed below into cells, with the existence and
checkability cells already closed and the size cell localized. The hypercube form of the same
target: the census classifies ALL minimal-UNSAT structure at small `n` (42,263 orbits at `n = 4`,
collapsing to ~403 structural types — the "base families," with the vast majority of orbits
mutants of a few hundred types); the family tower covers every `n` (proven); the growth law is
pinned (exactly one forced new family per scale — the degree-`n` rung); and what stands between
that classification and the target is certificate COST along the tower's top rungs plus
certificate TRANSFER along the morphs — if a type's certificate template transfers to its mutants
at bounded overhead and the type count stays controlled, that construction IS the generator, IS
`3-SAT ∈ coNP`. **The Uniform Transfer Theorem is now PROVEN in its finite form**
(`tests/uniform_transfer_theorem.rs`): certificates push forward along every refinement morph
(`g′_{C′} = Σ_{ψ(C)=C′} p_C·g_C`, sound by the one-line absorption identity `p_C·p_{ψ(C)} = p_C`,
valid over every ring — verified for all 43 orbit families at `n = 3` × `ℤ/2, ℤ/3, ℤ/6` × three
named morphs, plus one COMPLETE exhaustive morph sweep); the all-corners cube refines everything
(the super-family: exactly ONE true family per `n`, all others its mutants, any two families
within a two-step span of each other through the cube); and transfer is FUNCTORIAL (two-hop
pushforward equals the composite one-hop, bit-exact). The kernel already certifies the cube's
certificate `∀n`, so the whole hypercube is covered by kernel-∀n source + finite transfer. What
remains open is exactly the TOLL: transfer from the cube costs its `2ⁿ` basis; `3-SAT ∈ coNP` =
the existence of morph decompositions with polynomial toll — ANY fixed exponent suffices (an
`n²⁰` bound would do; generosity in the exponent is mathematically free), but no polynomial
escapes the resolution-class exponential (Chvátal–Szemerédi), so the cheap chains must be found
at SR strength or above.

The instruments have compressed the distance between the question and executable objects to zero;
what remains is the theorem, localized. The outline, cell by cell (PAPER §§8.2–8.4):

1. **The only bridge**: Cook–Reckhow. NP = coNP ⟺ one proof system is polynomially bounded.
   Since 3-SAT is NP-complete (Cook–Levin), this is equivalent to `3-SAT ∈ coNP`.
2. **coNP membership splits into three obligations — two are PROVEN.** Certificates for UNSAT
   3-CNF *exist* (the no-finite-randomness theorem, per instance, every ring — certified) and are
   *poly-time checkable* (the zero-trust checkers — certified). Only polynomial *size* remains.
   ("3-SAT is coNP up to size.")
3. **The size obligation is asymmetrically open.** CLOSED NEGATIVELY for resolution/RUP
   (Chvátal–Szemerédi: random 3-CNF needs exponential resolution — the system CDCL emits can
   never witness the swap). OPEN for SR/EF-class and above — no superpolynomial lower bound is
   known for SR (the candidacy is real), and PHP is the certified precedent that Haken-exponential
   families collapse to polynomial one rung up (`m(m−1)/2`, fitted).
4. **The = direction's ledger** (`np_conp_attack_ledger.rs`): per-family polynomial SR/specialist
   bounds certified with fitted exponents (PHP quadratic, Tseitin linear, `Count_p` linear); open
   cells named (threshold 3-CNF; the `REF`-mirrors, which by the §8.3 equivalence ARE the swap).
   Filling every cell = NP = coNP. Every new family certified polynomial is genuine =-direction
   progress; the lane (proof search, upper bounds) is blocked by NO formal barrier.
   **The hunt half is closed**: wherever a generator is planted, the zero-hint hunter finds it
   through syntactic disguise (parity, Horn, pigeonhole-in-a-haystack — certified), and with core
   isolation it recovers the plant EXACTLY, holding the generator's constant (`[24,24,24,24]`
   across a tripling haystack — the tax theorem). "Structure ⟹ found" is proven; what remains of
   the missing lemma is only "does cheap structure exist to find" — the missing lemma in hunt
   form: `3-SAT ∈ coNP ⟺ every UNSAT 3-CNF is secretly planted`.
5. **The ≠ direction's evidence**: every certified lower bound here (growing NS degree at every
   characteristic, growing width, the growing mirror dial `[(2,1),(3,2),(4,15)]`) — and the
   barriers say our algebraic instruments cannot finish that direction either.
6. **The self-referential pivot** (the mirror, §8.3): "has a short proof" is itself NP-complete
   (Atserias–Müller, run constructively — models compile to refutations of `REF`); the swap is
   equivalent to the mirror's own lower-bound certificates staying short. P vs NP now has a
   measured dial in this repository; extending the curve is the experiment, deciding its growth is
   the theorem.

What we will not do is claim the theorem: no honest instrument in this repository proves NP = coNP
or its negation, and the measured evidence leans ≠. The outline's value is that every future step —
ours or anyone's — lands in a named, certified cell.

## 8. Bottom line

We can prove, machine-check, and re-verify a lot: no finite randomness AND its incompressibility
pole, the family tower, the symmetry cut, degree and width lower bounds with independent
certificates, ∀-scale invariant-witness verdicts from one finite computation, the kernel-formalized
ring, and an EF-class proof-search engine whose outputs the community's own checkers accept. We
**cannot** prove P = NP — the lower-bound tools are algebraic (inside the algebrization barrier), and
everything they show points toward hardness, not collapse. The one direction with no formal barrier —
automated search for short EF-class proofs — is instrumented and treated as an experimental subject,
its failures reported as data. The strongest, most defensible thing we can publish is the honest
two-poles study (`PAPER.md`), with the certified atlas as its backbone.
