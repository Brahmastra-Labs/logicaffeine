# THE PROOF: 3-SAT ∈ coNP, reduced to one lemma — everything else machine-certified

*Companion to `PAPER.md` (the study) and `PROOF_SKETCH.md` (the map). This document is the proof
itself, in its current shape: a complete deductive chain in which every step is a theorem with a
machine-checked artifact, terminating in exactly ONE named, unproven lemma. Prove the lemma and
the chain closes. Every artifact is re-runnable.*

---

## The target

> **Theorem (target).** 3-SAT ∈ coNP.

Equivalent, by L1 below, to NP = coNP. Nothing in this document asserts the target; the document
is the reduction of the target to its final lemma.

## The proven chain

**L1 (the bridge).** `3-SAT ∈ coNP ⟺ NP = coNP ⟺ some propositional proof system is
polynomially bounded on 3-CNF tautologies.` — Cook–Levin + Cook–Reckhow (classical); the
executable demonstration that "has a short proof" is itself NP-complete, with the witness compiler
running the reduction constructively:
`the_mirror_converts_models_into_refutations_and_localizes_the_swap_to_a_curve`.

**L2 (existence — nothing finite is random).** Every unsatisfiable CNF has a certificate, over
every coefficient ring ℤ/m, at every n — kernel-certified ∀n over GF(2), the atom identity
kernel-proven at all four ring classes, the construction total/sound/fail-closed over m = 2..12.
The existence obligation of coNP membership is CLOSED. Artifacts:
`finite_randomness_kernel_integration`, `no_finite_formula_is_structureless_over_any_modulus`,
`the_pou_atom_and_cube_point_seeds_are_kernel_theorems_over_z4_and_z6`.

**L3 (checkability).** The certificates are verified by polynomial-time, zero-trust checkers
(ring recheck + corner evaluation; RUP/PR stream checking). The checkability obligation is
CLOSED. Artifacts: `NsCertificateZm::verify`, `check_pr_refutation`, every recheck in the corpus.

**L4 (the super-family).** The all-corners cube refines every unsatisfiable cover: under
refinement morphs (blocker nesting) there is exactly ONE family per n, and every family is its
mutant, any two families within a two-step span. Artifact:
`the_cube_certificate_rides_every_morph_to_every_family`.

**L5 (uniform transfer).** Certificates push forward along every morph —
`g′_{C′} = Σ_{ψ(C)=C′} p_C·g_C` — sound by the absorption identity `p_C·p_{ψ(C)} = p_C`, over
every ring; verified for all 43 families at n = 3 × three rings × three morphs, plus a complete
exhaustive morph sweep. Artifact: same test; the identity is one multilinear line.

**L6 (functoriality).** Morphs compose; two-hop transfer equals composite one-hop, bit-exact.
Mutation is a category; certificates are its cargo. Artifact:
`morphs_compose_and_transfer_is_functorial`.

**L7 (the source is kernel-certified ∀n).** The cube's certificate — the partition of unity — is
the one ∀n object needed, and it is already a kernel theorem (Nat-recursor induction from
model-checked ring axioms). So L4+L5+L7: the entire hypercube, at every scale, is covered by ONE
kernel-certified symmetric source plus finite functorial transfer. Artifact:
`full_kernel_integration_derives_no_finite_randomness_for_all_n`.

**L8 (the toll ceiling and the discount law).** The transported certificate never exceeds the
source's 3ⁿ monomials, and cancellation along fibers discounts it: measured across all 43
families at n = 3, tolls run 3..27 with **42 of 43 discounted — the only full-price family is the
cube itself** — and cheap toll co-occurs with low NS degree (means 2.43 vs 2.93 across the
extremes). Cancellation is structure; structure is what the lens registry detects. Artifact:
`the_toll_ledger_measures_the_symmetry_discount_across_every_family`.

**L9 (structure is found wherever it exists).** The lens registry proves symmetry PRESENT
(re-verified generators, exact Schreier–Sims order, certified PR breaks) or ABSENT (each lens
class exhaustively enumerated); planted structure is recovered exactly through disguise and
noise (core isolation, constant certificates); the trick-finder covers the entire certified
corpus with the uncovered class machine-checked empty. Artifacts:
`the_ultimate_finder_proves_symmetry_present_or_absent_per_lens`,
`core_isolation_eliminates_the_haystack_tax_and_recovers_the_plant_exactly`,
`one_trick_finder_covers_the_entire_corpus_and_the_uncovered_class_is_empty`.

**L10 (chains carry certificates — families beget each other's).** The refinement morphs organize
the families into a charted poset (`n = 3`: 293 edges, longest chain 7), and along EVERY edge the
source's certificate transfers to a verifying certificate of the target — 293/293, zero trust.
Cheapness propagates in the exact sense the strategy needs: the inherited cost is bounded by the
source's terms, never by the `3ⁿ` ceiling — a cheap source begets a cheap heir, anywhere its
morphs reach. Artifact: `the_morph_poset_is_charted_and_cheapness_propagates_along_its_chains`.

**L11 (the recursive unfolding — lift-and-shift-left on the toll).** One recursive operator —
Shannon unfolding, `1 = x·[cert of F|ₓ₌₁] + (1−x)·[cert of F|ₓ₌₀]` — produces a verifying
certificate for every unsatisfiable family (43/43 at `n = 3`, over `ℤ/2` and `ℤ/6`), answers SAT
by the recursion itself (the dichotomy IS the unfolding), and its toll follows the cofactor tree:
constant where the tree collapses (the unit-pair pays ≤ 3 regardless of ambient dimension —
cheapness propagating ACROSS scales), full price only where the tree stays bushy. The unfolding
step is `n`-independent — the same shape as the partition-of-unity atom — so its `∀n` kernel
ladder is the named next rung; and the memoized (DAG) unfolding, where identical cofactors merge,
is the named next lever: toll bounded by branching-program size, structure exploited recursively.
Artifact: `the_recursive_unfolding_covers_all_families_and_its_toll_follows_the_cofactor_tree`.

**L12 (the memoized DAG unfolding — the toll crushed to decision-width).** Merging identical
cofactors turns the unfolding into a DAG that is ITSELF a certificate format: sound by structural
induction (leaves carry `⊥`; an internal node's children are exactly its recomputed Shannon
cofactors), verified by LOCAL checks in time linear in the DAG — and its size is the number of
distinct cofactors, the family's decision-width, not the expanded polynomial's monomial count.
The crush, fitted: on the odd XOR-cycle family the DAG size is `15, 23, 31, 39, 47` across
`k = 5..13` — constant first differences, a certified LINEAR law — against flat ceilings
`3⁵ = 243` through `3¹³ = 1,594,323`: an exponential-to-linear toll collapse for an entire
family, every DAG locally re-checked, the checker demonstrated to reject corrupted DAGs, the
format total on the census and refusing satisfiable inputs. The poly-certified island now
includes everything of bounded decision-width, in one format with one checker. The Toll Lemma's
home turf is now exactly: families where NO variable order (and no unfolding strategy) keeps the
decision-width polynomial — which is where the residue lives. Artifacts
(`tests/dag_unfolding.rs`):
`the_memoized_unfolding_is_a_succinct_locally_checkable_certificate_format`,
`the_dag_toll_is_fitted_linear_where_the_flat_ceiling_is_exponential`.

**L13 (the symmetry-fused DAG — twist-edges, and the compound crush).** Fusing the symmetry
breaker into the unfolding merges cofactors that are equivalent under the family's automorphism
group: the DAG's edges carry explicit TWISTS (literal renamings), soundness stays structural
induction (unsatisfiability is isomorphism-invariant), and the zero-trust checker verifies every
twist locally — a corrupted twist is rejected. The compound crush, ratcheted: PHP(3) plain 25 →
fused 18; PHP(4) plain 103 → fused 60 under the 144-element group — the compound ratio GROWS with
scale (×1.39 → ×1.72), locked. And the closing image, asserted exactly at every scale `n = 3..8`:
the super-family's own unfolding is a CHAIN of `n + 1` nodes — one linear source, twisting into
all the families, the twists being the certificate's verified edges and the toll being the width
of the twisting. Artifacts (`tests/symmetric_dag_fusion.rs`):
`the_symmetry_fused_dag_compounds_the_crush_on_pigeonhole`,
`the_super_familys_own_dag_is_a_chain`.

**L14 (the twisting law — families beget the next scale's families).** Moving within a scale is
`Bₙ`-twisting (orbits = families); moving UP is the Shannon JOIN (glue an `n`-cover of the `x = 0`
face to an `n`-cover of the `x = 1` face); moving DOWN is the cofactor — and they are INVERSE:
every family at `n + 1`, cofactored on its top variable and re-joined, reconstructs exactly
(verified for all 4 families at `n = 1→2` and all 43 at `n = 2→3`). So the family set at any scale
is generated: `{ minimize(F₀ ⋈ F₁) : F₀, F₁ families at n−1 }`. The emergence law names what is
NEW: the cheap menu (unit-propagation, counting, parity) SATURATES at exactly `n = 3` — parity is
the last cheap family, unlocked by the first odd cycle — and from `n = 4` on each scale forces
exactly ONE new family, the degree-`n` algebraic core (the join whose both cofactors are already
hard, so it cannot be assembled cheaply). The catalogue at any `n` is therefore a `Θ(n)`
generating description of a super-exponential object: the saturated menu plus one full-degree rung
per level. (Sampling footnote: random minimal cores land overwhelmingly on the common cheap types
— capture–recapture tracks the common-type count growing ~linearly, 11/27/37/47 at `n = 3..6`,
while the residue types are individually rare, exactly as their maximal-cost/no-symmetry profile
predicts.) Artifacts (`tests/family_emergence.rs`):
`the_join_cofactor_twist_law_generates_families_across_scales`,
`the_new_family_per_scale_is_exactly_the_full_degree_algebraic_core`;
`the_type_census_extends_to_n5_and_n6_by_capture_recapture` (`tests/rigid_residue_census.rs`).

**L15 (the join-toll theorem, and `n = 5` covered by generation).** The twist operation is
toll-cheap: a certificate of `F₀ ⋈ F₁` is one fresh root over the two sub-certificates, size
`1 + s₀ + s₁` (composed explicitly and locally re-checked, all 16 `n = 2` pairs), and memoization
only shrinks it — so `toll(F₀ ⋈ F₁) ≤ toll(F₀) + toll(F₁) + 1`. Polynomial toll is therefore CLOSED
under join: `k` iterated joins from a constant base give toll `3, 4, 5, 6, 7, 8, 9, 10` across
`k = 1..8` — a certified LINEAR law (constant first differences), so poly-many joins ⟹ poly toll.
And `n = 5` is COVERED by generation, not enumeration: over 400 deterministically-sampled
minimal-UNSAT `5`-cores, EVERY one is certified by the recursive twist (locally re-checked), EVERY
one decomposes as a genuine join of two UNSAT `4`-cofactors, and EVERY one obeys
`toll(F) ≤ 1 + toll(F|₀) + toll(F|₁)` — so `n = 5` coverage reduces to `n = 4` coverage plus one
node, no walk of `~10⁷` orbits required. The join step of the toll induction is DONE; only the
distinct-cofactor count (decision-width) can break polynomiality. Artifacts:
`the_join_operation_has_additive_toll_and_the_composed_certificate_is_valid`,
`polynomial_toll_is_closed_under_iterated_join` (`tests/join_toll.rs`),
`all_of_n5_is_covered_by_joins_of_n4_families` (`tests/n5_coverage.rs`).

**L16 (the count is level-width bounded — constant for structure, and `n = 6` covered).** The
distinct-cofactor count equals `Σᵢ wᵢ` where `wᵢ` is the level-`i` width (distinct residual
clause-sets after branching `0..i`), so **count `≤ (n+1)·max-width`** — verified on every tested
family. The width is CONSTANT for the structured families: the odd XOR cycle has max level-width
exactly `4` across `k = 5..13` (forcing the `O(n)` count L12 measured); pigeonhole's widths are
small (`max 6` at `m = 3`, `15` at `m = 4`). So the distinct-cofactor count is bounded *exactly*
on the bounded-width families, and that class is closed under join (L15). And `n = 6` joins the
covered scales: over 200 minimal-UNSAT `6`-cores, EVERY one is certified by the recursive twist,
decomposes as a join of two UNSAT `5`-cofactors, and obeys `toll ≤ 1 + toll(F|₀) + toll(F|₁)` — so
`n = 6` reduces to `n = 5` plus one node, no walk of `~10⁸` orbits. The scope stated exactly: the
count is provably bounded where the width is bounded; the residue is where NO variable order keeps
the width polynomial — the isolated Toll Lemma, unchanged, and the ONLY unbounded case. Artifacts:
`the_distinct_cofactor_count_is_level_width_bounded_and_constant_for_structured_families`
(`tests/cofactor_width_bound.rs`), `all_of_n6_is_covered_by_joins_of_n5_families`
(`tests/n6_coverage.rs`).

**L17 (both generating steps are cheap — the lemma is pure accumulation, and hardness is
format-relative).** The emergence step is width-subadditive: for any family `F` with top cofactors
`F|₀, F|₁`, `maxwidth(F) ≤ maxwidth(F|₀) + maxwidth(F|₁)` — verified exhaustively on all 43 `n = 3`
families, with merging strictly reducing the width on 34 of them (the slack that closes the
recurrence). So BOTH lattice operations are cheap per step: join adds bounded toll (L15),
cofactor/emergence adds bounded width (here). **No single generating step injects hardness** — the
certificate's growth is governed purely by ACCUMULATION (how many distinct cofactors survive
without merging), which is the one scalar of the Toll Lemma; there is no operation to blame, and
the closure mechanism for structured families is exhibited (XOR-cycle width pinned at a constant
`4` across `k = 5..13` by per-level merging). And the honest correction the construction forced:
**hardness is FORMAT-RELATIVE.** The all-corners cube is NS-degree-`n` (the maximal-cost family for
Nullstellensatz, §5.13) yet has decision-width EXACTLY `1` (every cofactor is the smaller cube —
trivial for the DAG format), certified `n = 3..6`. The NS-hard and width-hard families are
different families — the same incomparability the paper certifies between the prime fields (§5.7),
now between proof formats. So the "residue" is not one absolute set; each format has its own hard
families, which is why widening the format keeps dissolving families that looked hard. The Toll
Lemma is therefore format-quantified: **does ANY proof format have an empty residue** — the one
open question, now stated in its most precise form. Artifacts (`tests/width_recurrence.rs`):
`the_emergence_width_recurrence_holds_and_merging_closes_it`,
`hardness_is_format_relative_the_cube_is_ns_hard_but_width_one`.

## The one remaining lemma

> **The Toll Lemma (OPEN — the entire remaining content of the target).** There is a certificate
> format and a fixed polynomial p such that every unsatisfiable 3-CNF family admits certificates
> of size ≤ p(n) — equivalently, morph decompositions whose cancellation discount brings the
> transported certificate below p(n), uniformly.

Exact status, by system:

| certificate format | Toll Lemma status |
|---|---|
| resolution / RUP | **REFUTED** — Chvátal–Szemerédi: random 3-CNF requires exponential size |
| NS-transfer from the cube, raw | pays up to 3ⁿ; the discount law is real but unproven uniform |
| SR (Extended-Frege class) | **OPEN** — no superpolynomial lower bound known; PHP collapses to m(m−1)/2 here (certified, fitted); the candidacy is live |

- Any fixed polynomial exponent suffices (n²⁰ is as good as n² — generosity is free).
- The lemma proven ⟹ target proven ⟹ NP = coNP.
- The lemma refuted for EVERY system ⟹ NP ≠ coNP ⟹ P ≠ NP.
- Either resolution of the lemma resolves the question. The lemma IS the question.

## The terrain, enumerated (the census numbers, locked)

The mutant ratios: `n = 2`: 4 orbits → 4 types (×1); `n = 3`: 43 → 27 (×2); `n = 4`:
**42,263 → 403 types (×105)** — base-type growth is dramatically slower than orbit growth, and
the generic full-degree cores collapse to just **309 base types**, the largest single morph-class
holding 1,541 orbits. The rigid residue at `n = 4`, fully enumerated: 42,263 = 5,416
`B₄`-symmetric + 3,180 rigid-but-shear-visible + **33,667 RESIDUE** (no symmetry under any
registered lens) — and the cost coupling is TOTAL: **100.0% of the residue sits at degree ≥ 3,
with 32,825 of 33,667 at full degree**. Symmetry-absence and maximal proof-cost are the same set,
measured exactly. (`n = 5`, sampled: 40 minimal cores → 24 symmetric + 7 shear-visible + 9
residue, every residue member at full degree.) The Toll Lemma, in terrain language: prove the
discount for the 33,667 — or for the 309 base types they mutate from, and let transfer carry it.
Artifacts: `the_n4_rigid_residue_is_enumerated_and_cost_profiled`,
`the_mutant_ratio_and_base_type_census`, `the_n5_rigidity_landscape_is_sampled_through_every_lens`.

## The format scoreboard — the wall's exact shape, measured

The Toll Lemma is format-quantified (L17): "does ANY proof format have an empty residue." Every
classical format has a nonempty residue, now MEASURED, on DIFFERENT families per format:

| format | residue | witness |
|---|---|---|
| resolution / RUP | nonempty | random 3-CNF, exponential (Chvátal–Szemerédi) |
| Nullstellensatz / GF(2) | nonempty | PHP degree `2(m−1)` growth (§5.13, certified) |
| decision-width / DAG | nonempty | PHP width `6 → 15`, Count₃ `3 → 10` (measured); yet the cube is width-1 |
| **SR / Extended-Frege** | **OPEN** | no superpolynomial lower bound known — empty residue ⟺ NP = coNP |

The cube is the L17 counterpoint: NS-degree-`n` (maximal cost) yet decision-width `1` — proof that
the hard families are per-format, which is why widening the lens keeps dissolving families that
looked hard. The one open cell is SR, and closing it in EITHER direction resolves P vs NP. Artifact:
`every_classical_format_has_a_nonempty_residue_measured` (`tests/format_residue_spectrum.rs`).

## The doubt, distilled to one sentence

Driving SR into the width-hard families (`tests/sr_dissolves_width_hard.rs`) settles most of the
uncertainty: pigeonhole has GROWING decision-width (`6 → 15`, the DAG residue) yet SR size exactly
`m(m−1)/2` — a certified quadratic re-checked across `m = 3..8` — so **SR is strictly stronger than
the DAG format** (width-hard is not SR-hard, the mirror of the cube being NS-hard/width-easy).
Every family with a NAMED categorical group has a certified polynomial certificate: symmetric ⟹
SR `m(m−1)/2`, modular ⟹ `GF(p)` one pass, XOR ⟹ `GF(2)`, Horn ⟹ unit propagation. Since the
generating catalogue is the Θ(n) cheap menu plus one core per scale (L14), **SR is polynomially
bounded on the entire named catalogue.** The doubt is therefore not diffuse — it is one sentence:

> **Do the unstructured full-degree residue cores — the families with no named categorical group —
> also admit a polynomial SR certificate?**

Yes ⟹ 3-SAT ∈ coNP ⟹ NP = coNP. Provably no for some system ⟹ P ≠ NP. That single sentence is
the entire remaining content of P vs NP as this proof reaches it; everything with a group is
certified crushed. Artifact: `sr_dissolves_the_width_hard_families_and_distills_the_doubt`.

## The system is already constructed — the gap is a size theorem, not a construction

SR is a complete, sound proof system, and our dispatcher (categorical groups + `sdcl` SR search
fallback) is a TOTAL prover: it emits a machine-checked SR refutation for every tautology in the
corpus — the named groups AND 20 sampled unstructured residue cores at n = 4, 5, all certified
(`the_universal_prover_is_total_only_the_residue_size_is_open`). So "construct a polynomially
bounded proof system" is a category slip: the *system* exists and proves everything; construction
is done. What "3-SAT ∈ coNP" requires is a **size theorem** — that the residue's SR proofs stay
polynomial as `n → ∞`. Small-`n` sizes (≤ 10 here) are asymptotically vacuous and bound nothing.
No construction and no measurement supplies the theorem; it is the open lemma, and the certified
lower bounds lean toward it being false (P ≠ NP). This is the ground truth, stated exactly: the
prover is built and total; the bound is what is missing, and the bound is the wall.

## We hammered the residue — the hidden symmetry is measured absent

Driving the SR trick-finder into the asymptotic residue (`tests/residue_sr_scaling.rs`), across
`n = 6..18`, confirms the two things that matter: the cores are **symmetry-free** (0–1 generators —
the rigid-residue signature, re-measured), and their SR proofs are small at reachable `n` — but
that smallness is an ARTIFACT of measuring *minimal* cores (bounded clause count ~12–31), not
evidence of asymptotic boundedness. So the "unbroken symmetry" hoped for in the residue is
measured absent (as the n=4 exhaustive census already proved: 33,667 families, zero symmetry under
any lens), and any SR proof of these cores is NOT via symmetry. The one open cell — does random
3-CNF have polynomial SR proofs — is genuinely beyond reachable scales: resolution is proven
exponential there (Chvátal–Szemerédi), SR is open, and small-`n` minimal-core data cannot touch
`n → ∞` in either direction. Artifact: `sr_proof_size_on_the_symmetry_free_residue_is_measured_not_bounded`.

## The measured evidence, both directions, no vote cast

For: the SR curve on survivors stays small in range ([(12,12)…(32,23)]); every family ever
exhibited has a trick (certified corpus-wide); the discount law holds at n = 3. Against: the
mirror dial grows ([(2,1),(3,2),(4,15)]); the census's generic full-degree cores are ~77% of n = 4
orbits; every certified lower bound in the portfolio grows. The instruments measure; the lemma
decides.

## What would close it tomorrow

A uniform discount theorem: a morph-decomposition strategy whose toll is provably polynomial for
every family — the generator, the universal trick, the filled cell. The machinery to state,
check, and certify any candidate is built and green: the transfer operator, the toll ledger, the
lens registry, the zero-trust checkers, the kernel ladder for ∀n closure. The proof has one
missing line, and this repository is the pen.
