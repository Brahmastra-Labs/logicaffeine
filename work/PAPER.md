# Structure and Incompressibility, Machine-Certified: an Atlas of Propositional Proof Complexity from Nullstellensatz to the Extended-Frege Class

*Draft. Every numbered result names a machine-checked artifact (a test under
`crates/logicaffeine_proof/{src,tests}` or `crates/logicaffeine_kernel`); the claims are re-runnable,
not asserted. §9 gives the reproduce commands.*

**Positioning (read first).** This is an *experimental and formally-verified* study. Where a result
is classical (the pigeonhole degree bound, Haken's resolution bound, short PR proofs of symmetric
principles, the `Count_p`-vs-`GF(q)` incomparability) we claim machine-certification and
reproducibility, not priority. Where a result appears new (the stabilized collapsed dual and its
∀-scale verdicts, the `GF(2)` witness-support schedules, the encoding-dependence of symmetric
visibility, the two-sided certified atlas, the characteristic-threshold law for the pigeonhole
witness support, the `p | |G|` annihilation dichotomy, and the ring-`ℤ/m` conjunction/nilpotent-tax
theorems of §5.6–5.11), it is stated with its verification chain and its trust boundary. **The
paper's core focus is the attack program of §§8.2–8.4**: the two kernel poles sharpened into an
executable assault on the NP vs coNP question — the gunsight, the mirror and its witness
compiler, the attack ledger with its per-family certified polynomial bounds, the
by-construction standard, and the generator hunt with exact plant recovery. Every cell of that
program that today's mathematics can fill is filled and certified; the cells that remain ARE the
question. §8 states plainly what none of this is: a P vs NP result.

## Abstract

Two theorems, both certified in the same Calculus-of-Constructions kernel, frame everything here.
**Structure always exists**: every unsatisfiable CNF over `n` variables has a degree-`≤ n` `GF(2)`
Nullstellensatz refutation, proven constructively for *all* `n` by kernel induction — no finite
object is "structureless." **Incompressibility is real and systematically unprovable**: the
invariance theorem, Chaitin's incompleteness, and its Gödel corollary are kernel terms — almost every
object *has* structure only at full cost, and no fixed proof system can certify that cost case by
case. Propositional proof complexity is the measurement science of the boundary between those poles,
and this repository is an instrument for it: an exhaustive small-`n` census; a certified
structure-detection ladder with an honest `Incompressible` verdict; re-checkable dual witnesses for
degree lower bounds; certified resolution-width lower bounds; a working, externally-verified proof
*search* engine in the substitution-redundancy (Extended-Frege-class) fragment; and a **two-sided
separations atlas** in which every row carries a certified impossibility on one side and a certified
proof on the other. The centerpiece is a **stabilization theorem made executable**: for a symmetric
family at fixed degree, the orbit-collapsed dual system's entries are bounded-degree integer
polynomials in the scale, so — by finite-difference interpolation and Lucas periodicity, both on the
kernel ladder — the existence of an invariant pseudo-expectation is decided **for every scale by one
finite computation**, with every in-window verdict differentially validated and every positive lifted
to a zero-trust-re-checked witness. Its verdicts are new data: modular counting carries its invariant
witness exactly on a mod-4 schedule; linear-encoded pigeonhole carries *none at any scale* while
asymmetric witnesses exist — over `GF(2)`, whether symmetry can *see* a lower bound depends on the
encoding, a characteristic-2 phenomenon with no analogue over ℚ. Finally the field itself is made a
dial: a general `GF(p)`/`GF(4)` engine — differentially anchored to the `GF(2)` engine at `p = 2` —
certifies that the primes are pairwise *incomparable* proof systems (each crushes at degree 1 a
counting family the other refutes only with growing certified degree), that extension fields
collapse (a `GF(4)` certificate projects constructively to a `GF(2)` one — the field ladder is the
prime ladder), that the pigeonhole bound is characteristic-*invariant* while its witness support
obeys a characteristic *threshold* (`p ≥ m`), and that the symmetrization obstruction is exactly
`p | |G|` — with the Reynolds operator machine-verified on the coprime branch. Past the fields, a
Howell-form engine takes Nullstellensatz onto the **rings** `ℤ/m`: completeness survives every
modulus (structurelessness has *no witness* over any `ℤ/m` — the existence pole, ring-completed,
with the cost pole untouched), coprime composite moduli are the *conjunction* of their parts (mod-6
reasoning adds nothing to bounded-degree ideal membership), and nilpotents exact a measured degree
tax (`ℤ/4` strictly weaker than `GF(2)` at fixed degree).

## 1. Setting

A clause over `n` variables is a subcube of `{0,1}ⁿ`; a CNF is unsatisfiable iff its clause-subcubes
cover the cube. Encoding a clause `C` as its false-indicator polynomial `p_C` over
`R_n = GF(2)[x_1,…,x_n]/(x_i²−x_i)`, a **degree-`d` Nullstellensatz refutation** is a certificate
`Σ_C g_C·p_C = 1` with `deg(g_C·p_C) ≤ d`; `NS-degree(F)` is the least such `d`; NS is complete at
`d = n`. Two generator conventions matter. The **clause encoding** takes the `p_C` themselves. The
**linear encoding** of an exactly-one constraint over a group `G` takes the degree-1 generator
`1 + Σ_{v∈G} x_v` plus the pairwise products — the literature-standard object for counting families.
The two interreduce, and the direction is *asymmetric*: clause-refutable at `d` ⟹ linear-refutable
at `d` (degree-preserving — every `≥2`-subset monomial of the wide clause is a pair multiple), while
the reverse costs `d + k−1`. **Bounds proven against the linear encoding are therefore the stronger
statements.** Artifact: `the_linear_and_clause_encodings_interreduce_at_bounded_degree`.

The engines: the clause engine (`nullstellensatz_refutes`, `ns_lower_bound_witness`,
`check_ns_lower_bound`) enumerates the cube and stops at 20 variables; the polynomial-generator
engine (`ns_refutes_polys` and friends) walks only the `C(n, ≤d)` degree-bounded monomials
(`monomials_up_to_degree`), reaching 63 variables, and agrees with the clause engine exactly on CNFs
(differential + duality: `ns_over_polynomial_generators_agrees_with_the_clause_encoding_on_cnfs`,
`degree_bounded_monomial_enumeration_scales_past_twenty_variables`).

## 2. The two kernel poles

**2.1 Structure always exists (no finite randomness).** Every unsatisfiable CNF over `n` variables
has a degree-`≤ n` `GF(2)` NS refutation, via the partition of unity `Σ_a δ_a = 1` — which *factors*
as `Π_i ((1+x_i)+x_i)`, a product of `n` unit atoms, so it holds at every `n` by induction. The
induction is kernel-certified (Nat recursor), and in the full integration the base and step are
themselves derived from ring axioms inside the kernel. Artifacts: `polycalc::build_ns_certificate`
(the certificate carries a re-checkable witness),
`partition_of_unity_is_one_for_all_n_by_the_atom_factorization`,
`tests/no_finite_randomness_infinity.rs`, `tests/finite_randomness_kernel_integration.rs`. The honest
cost: the degree-`n` certificate lives in a basis of size `2ⁿ`
(`no_finite_randomness_but_the_certificate_is_exponentially_large`). Existence is
information-theoretic, not efficient.

**2.2 Incompressibility is real and unprovable (the other pole).** In the same kernel: the
**invariance theorem** (`K_U(x) ≤ |pref(V)·prog(V,x)|`), **Chaitin incompleteness** via the Berry
program (no sufficiently strong system proves `K(x) > C_F` for its own constant), and the **Gödel
corollary** (a true, unprovable sentence) — assembled as kernel terms and re-checked by type
inference, with a negative control (a derivation skipping the simulation step is rejected).
Artifacts: `invariance_is_kernel_certified`, `chaitin_incompleteness_via_berry_is_kernel_certified`,
`godel_incompleteness_corollary_is_kernel_certified` (`tests/ait_kolmogorov.rs`).

Together: structure always exists as an *object*; incompressibility exists as a *cost*; and no proof
system certifies the cost uniformly. Everything below measures where, between those poles, concrete
families and concrete proof systems sit — with certificates, never trust.

**2.3 The poles at infinity.** §2.1 is already a `∀n` theorem — there is no larger finite `n` to
escape to.

*First, the counting argument is finite, not asymptotic.* "Almost every object has structure only at
full cost" (§2.2) is easily misread as a statement in a limit, or about infinite objects we can never
reach; it is neither. For every **fixed finite** `n`, of the `2ⁿ` strings of length `n` at most
`2^{n−c} − 1` admit a description shorter than `n − c` — there are only that many shorter descriptions —
so at least a `1 − 2^{−c}` fraction are `c`-incompressible. This is a pure pigeonhole count (the same
principle §5.2 studies as a proof-complexity object), holding at each finite `n` with an explicit
fraction and certified per-`n` by the very engine the SAT side already trusts:
`ait::incompressible_string_exists` builds `2ⁿ` strings against `2ⁿ − 1` shorter programs and hands them
to `pigeonhole::certify_pigeonhole_unsat` (`incompressibility_lemma_is_certified_by_counting`). No
infinity is quantified over; every `n` a human ever writes down is covered exactly. The infinity that
*does* enter is **not** "individual objects are secretly infinite" — it is **uniform-in-`n`**: whether
the cost stays polynomial *along* a family as `n → ∞`. That is Cook–Reckhow (the third bullet below),
and the finite→uniform gap — not a finite→infinite one — is where P vs NP lives.

Three genuine infinity extensions, each with its exact status:

- **Infinite formulas (compactness).** A countably infinite CNF is unsatisfiable iff some finite
  subset is (propositional compactness; König's lemma in the countable case). Composed with §2.1:
  *every unsatisfiable infinite system has a finite fragment carrying a certified refutation* — no
  randomness at infinity, literally. Status: **delivered in the ladder architecture** — the König
  rungs (level-nonemptiness, the parent projection, the limit path) and the composition
  infinite-UNSAT → finite fragment → re-checked certificate are verified on concrete infinite
  families (`tests/no_randomness_at_infinity.rs`), and — **the tree argument is now internalized**,
  not merely laddered — the infinite path is an explicit kernel function `path : Nat → Node` and
  König's conclusion, *the path reaches every level* (`∀n. mark(path n) = lvl n`), is a
  kernel theorem with derived base and step: the step consumes the induction hypothesis by Leibniz
  substitution, the whole derivation certifies to a `Fix`/`Match` the kernel type-checks, and a
  negative control (skip the IH rewrite) is rejected. The opaque `LevelNonempty` premise of the
  ladder is gone. Artifacts: `the_konig_path_is_level_faithful_at_every_depth_kernel_certified`,
  `skipping_the_induction_hypothesis_fails_the_kernel_type_check`
  (`tests/konig_compactness_kernel.rs`).
- **Infinite sequences (the poles split).** For infinite 0/1 sequences the two poles become: every
  finite *prefix* of every sequence has structure at some cost (§2.1, kernel-done at every `n`),
  while almost every infinite sequence is **random in the limit** (Martin-Löf; Chaitin's `Ω` is a
  concrete witness, and §2.2's kernel Chaitin theorem is exactly its finite engine). Both poles are
  theorems, and their coexistence is now certified **on the concrete limit object Ω**: Ω is
  Martin-Löf random (Levin–Schnorr fed by the Ω incompressibility bound) and Ω is not computable
  (computability forces bounded prefix complexity, contradicting incompressibility past the scale
  threshold — the same `le_trans`/antisymmetry contradiction as the Berry theorem), while
  simultaneously every finite prefix of Ω carries a structure certificate — the two poles assembled
  as one kernel conjunction `And (MLRandom Ω) (∀n. HasStructureCert Ω n)`. This is the limit form of
  "structure exists pointwise, not uniformly," on one object. Artifacts:
  `omega_is_martin_lof_random_is_kernel_certified`, `omega_is_not_computable_is_kernel_certified`,
  `the_two_poles_coexist_on_omega_is_kernel_certified` (`tests/martin_lof_omega_kernel.rs`).
- **Uniform cost (the open one).** The extension "certificates stay *polynomial* along families" is
  neither claimed nor refuted here: by Cook–Reckhow it is equivalent to `NP = coNP`. What these
  instruments contribute is a working prototype of the only move that goes around the standard
  asymptotic front door: §7 decides infinitely many scales by **one finite computation**, because
  the *family* — not the instance — is the object (its constraint systems are bounded-degree
  polynomial data in the scale). Turning more infinite questions into finite objects of that kind is
  the program.

**2.4 The hardness dichotomy.** The two poles compose into one statement about *pointwise* hardness,
assembled and kernel-certified as a single conjunction: hardness of an individual object, however
formalized, is **either vacuous or unprovable**. Certificate-hardness ("unsatisfiable with no
structured refutation") has *no instances* — conjunct A is the §2.1 pole, and the ring extension
closes every modulus `ℤ/m`. Incompressibility-hardness has instances — most objects, by counting —
but **no system ever proves it of any named object** — conjunct B is the §2.2 Chaitin pole. So at
the object level, hardness can be defined but never exhibited: point at an object and the claim
"this one is hard" is either false or beyond proof. What the dichotomy deliberately does *not* cover
is family-hardness at a growth rate — a different type, real and exhibitable per proof system
(§8.2's retreat ladder exhibits it, certified on both sides). Artifact:
`pointwise_hardness_is_either_vacuous_or_unprovable_as_one_kernel_statement`
(`tests/hardness_dichotomy.rs`).

## 3. The census: an exhaustive structural map (`n ≤ 4`)

One representative per hyperoctahedral (`Bₙ`) orbit of every minimal unsatisfiable formula, with its
invariant battery: symmetry, face vector, minimum resolution width, and its weakest *certified* rung
(unit-propagation ≺ counting ≺ parity ≺ NS-by-degree). Orbit counts `1, 4, 43, 42263` for
`n = 1..4`; the family tower is `Θ(n)` and provably complete (no formula beyond budget — §2.1 is the
proof); the `42263` orbits collapse to `403` structural signatures; ~77% are generic full-degree
cores. Artifacts: `census_orbit_counts_are_locked`, `census::family_tower`,
`the_family_tower_is_provably_complete_and_finite_without_enumeration`, `menu_split`. The dispatcher
(`solve.rs`, 31 routes from `TwoSat` through `ExactCover`/`ModP`/`ModM`/`Sos` to `Incompressible` and `Cdcl`)
is the operational form of the ladder; `ait.rs`'s `LinearRigidityCert` and the par32 measurements
(157-dimensional linearly-rigid kernel) are its honest "no shortcut here" instrument.

## 4. Symmetry = compression

**4.1 The `Bₙ`-vs-`AGL` lens.** The clause-level symmetry lens under-reports structure: widening to
`AGL(n,2)` reveals hidden affine symmetry in ~92% of `Bₙ`-rigid cores, and for *any* linear family
the affine symmetry has a closed form `|GL(k,2)|·|GL(n−k,2)|·2^{k(n−k)}·2^k` — a `2^{Θ(n²)}` factor
the clause lens is blind to, at every `n`. A polynomial-time transvection finder recovers depth-1
shears of arbitrary CNFs at any `n`; parity's symmetries hide at depth 2 and the composite-depth
finder catches them — **the depth a formula's symmetries require is itself a complexity measure.**
Artifacts: `affine::affine_subspace_agl_order`, `census::affine_transvection_generators`,
`census::affine_composite_shear_generators`,
`the_parity_wall_is_a_depth_not_a_wall_composite_finder_catches_it`,
`affine_symmetry_dwarfs_permutation_symmetry_on_parity_at_every_scale`.

**4.2 The `Sₙ` cost-cut.** Under full symmetry the `2ⁿ` monomial basis collapses to `n+1`
orbit-columns and the reduced refutation is sound
(`symmetry_cuts_the_full_ns_basis_from_exponential_to_linear`,
`nullstellensatz_refutes_symmetric` — fail-closed if a passed generator is not a genuine symmetry).

**4.3 The cofactor-DAG lens: symmetry above the instance.** §§4.1–4.2 measure symmetry *in the
instance* — automorphisms of the cover. The residue (§3, §5.12) is rigid to all of them, and that
rigidity is the wall. A third lens sits strictly above: the symmetry need not fix the formula, only
relate its **cofactors**. The Shannon cofactor DAG of an unsatisfiable CNF takes as its nodes the
distinct residual sub-formulas `F|ρ` (Shannon expansion `F = x·F|_{x=1} + x̄·F|_{x=0}`); a hard core
has exponentially many distinct cofactors — the incompressibility pole of §2.2, one level up and
finite at every `n`. The lens quotients that fixed set by a congruence `~` and counts the classes.
Because it quotients *one fixed set*, `iso ≤ rename ≤ raw = distinct` are theorems for every formula:
the collapse measure `quotient_class_count` and the zero-trust certificate
`quotient_dag`/`check_quotient_dag` live in `cofactor.rs`, with the congruence ladder
`Raw ⊂ Rename ⊂ GroupInduced(Bₙ,AGL) ⊂ CofactorIso` — the last full CNF-isomorphism (any relabeling and
polarity flip), *not* required to be an instance automorphism, the strongest **decidable** rung.

The measurement is two-sided and honest. On structured families the cofactors collapse to
polynomially-many classes and the quotient certificate is polynomial and re-checked (odd XOR cycles:
linear distinct count, CofactorIso linear; pigeonhole: the group-cofactor certificate is `≤ 4m²` nodes,
output-sensitive, re-checked — recovering the paper's poly upper bounds through the new lens;
`poly_cofactor_quotient_is_a_poly_time_checkable_certificate`, `the_cofactor_quotient_ladder_collapses_structured_families`).
On the residue the lens finds genuine symmetry the instance lens is blind to: across the `n = 4`
minimal-UNSAT cores rigid under **every** registered instance lens (exhaustive `B₄` *and* every shear
to depth 3 — 828 of a stride-41 sample), **387 (46.7%) collapse under CofactorIso** — cofactor symmetry
above every instance automorphism, each carrying a re-checked certificate
(`the_residue_cofactor_dag_rigidity_is_measured_and_the_collapse_count_reported`). But the collapse is
*small* (order-1 merges at this scale), and on the worst-case archetype — random 3-CNF above threshold —
CofactorIso does not collapse the class count at all
(`(n, distinct, iso) = (8,66,63), (10,121,120), (12,340,336)` — iso tracks distinct, both exponential;
`the_scaling_wall_random_3cnf_cofactors_explode_and_iso_does_not_collapse`). The strongest decidable
congruence is exhausted at the wall.

The reading is exact. Symmetry-above-the-instance is a real, widespread phenomenon on rigid cores — the
`Bₙ`/`AGL` lens genuinely under-reports, as §4.1 already found for affine symmetry, now one level
deeper. It is not, at any rung a decidable congruence reaches, a polynomial collapse of the worst case.
By the load-bearing lemma (polynomially-many cofactor classes ⟹ a polynomial certificate, the
class-sharing being the extension-variable mechanism), a poly-index congruence on the residue's cofactor
DAG would be a polynomial SR/EF certificate — `3-SAT ∈ coNP` (§8.4). So the open cell of §8, reframed in
this language, is exactly **a poly-index SR-definable Shannon congruence on the residue** — one using
extension variables to relate cofactors that CNF-isomorphism cannot. Every decidable rung below it is
certified exhausted here. Artifacts: `cofactor.rs`, `tests/cofactor_lens.rs`.

**4.4 The growth-root law: the cofactor DAG is an automaton, and the carry is the root.** §4.3 counts
cofactor classes at a fixed `n`; the sharper object is how that count *grows*. Reading the variables in
order, the cofactor DAG is a finite automaton whose level-`i` states are the distinct residuals `F|ρ`
over length-`i` prefixes — the Myhill–Nerode classes of the cofactor language, the minimal automaton,
the best lossless compression of the prefix's *sufficient statistic* (the "carry"). The width sequence
`w(n)` therefore has a growth root, and that root — computed from the carry's structure, not from
enumerating the DAG — is the polynomial/exponential dividing line. The law
(`the_carry_monoid_size_is_the_growth_root_discriminant`, `the_carry_monoid_law_is_myhill_nerode_compression_over_all_families`):
a carry that is a **count / group element** — a poly-size monoid closed under the transition — forces
**root 1** (parity `ℤ/2`, width 2; mod-`q` `ℤ/q`, width `q`; majority, width `O(n)`); a carry that is a
**set / matching** — an exponential monoid whose transition branches — forces **root > 1** (pigeonhole
must remember the partial matching, width `2^{Θ(n)}`). The exact dial is the carry's maximum *set
cardinality* `s`: the widest-level width is the Lucas/binomial partial sum `Σ_{j≤s} C(⌊n/2⌋, j)`, whose
finite-difference degree is exactly `s`, so `s = O(1) ⟹` degree-`s` polynomial (root 1) and `s ∝ n ⟹`
`2^{Θ(n)}` (root > 1) — one knob between the two regimes
(`the_carry_set_cardinality_is_the_exact_root_knob`). This is not only a closed form: a real boolean family
— `⊕_{i∈X} yᵢ` gated on `|X| ≤ s`, `X` the true-`x` set — *realizes* it, and its measured OBDD width equals
`Σ_{j≤s} C(m, j)` term for term across the whole dial (`s = 1 → 3,4,5,6`; `s = 2 → 7,11,16,22`; `s = m → 2^m`),
so the structural feature forcing the root is pinned to ground truth: a bounded-size *subset* carry, its
cardinality bound `s` the knob (`the_bounded_subset_carry_realizes_the_binomial_width_on_a_real_family`).
The knob is an integer, and it is the *degree*, not merely "poly vs exp": sweeping `s = 0, 1, 2, 3`, the
measured width's finite-difference degree equals `s` exactly (`1,1,1,1`; `2,3,4,5,6`; `4,7,11,16,22`;
`8,15,26,42,64`), so the growth root's polynomial degree *is* the dimension of the carry's minimal sufficient
statistic — parity dimension 0, a counter dimension 1, a bounded-`s` subset dimension `s`, a full subset
dimension `Θ(n)` — the single integer behind count-vs-set (`the_carry_dimension_ladder_is_exactly_the_polynomial_degree`).
And the `> 1` side is not the single value `2` but a continuous algebraic spectrum: gating the same carry on
independent-sets-of-a-path (the `no-11` adjacency rule) instead of a cardinality bound makes the width count
independent sets — the Fibonacci number `F_{m+2}` (`3,5,8,13,21,34,55`) — so its recurrence is `s_n =
s_{n-1}+s_{n-2}` and Binet's dominant root is the golden ratio `φ = (1+\sqrt5)/2 ≈ 1.618`, an intermediate
root super-polynomial yet sub-full-exponential, read off the recurrence exactly and non-polynomial by finite
difference (`the_independent_set_carry_has_golden_ratio_root_binet_on_a_real_family`). The real dividing line
is `root = 1` versus `root > 1` (polynomial versus super-polynomial); the `> 1` side is a continuum whose
endpoint `2` is merely the full-set carry. That continuum is itself tunable by one structural knob — the
carry's memory depth. Forbidding a run of `k` consecutive elements makes the width the `k`-step Fibonacci
sequence, whose dominant root climbs `φ ≈ 1.618` (`k=2`), tribonacci `≈ 1.839` (`k=3`), tetranacci `≈ 1.928`
(`k=4`), toward `2` as `k → ∞` (`the_forbidden_run_length_tunes_the_growth_root_across_the_spectrum`), each
read off its order-`k` all-ones recurrence — the root's position in `(1, 2)` is exactly how far back the
local constraint reaches. A complementary knob fills the spectrum from the other side: the minimum gap
between set bits, `f(n) = f(n-1) + f(n-g)`, whose root *descends* `φ` (`g=2`), `≈1.466` (`g=3`), `≈1.380`
(`g=4`) toward `1` as `g → ∞` (`the_minimum_gap_between_ones_tunes_the_growth_root_from_below`). Two
independent local features — maximum run and minimum gap — tune the root across `(1, 2)` from opposite
directions, so the root is a property of the *specific* constraint, not its coarse size. Composition is a
genuine operation on the root, not a product of per-feature contributions: intersecting "no two adjacent 1s"
(`φ`) with "no three adjacent 0s" lands on the **plastic number** `ρ ≈ 1.3247` — the smallest Pisot number,
root of `x³ = x + 1`, verified as the exact order-3 Padovan recurrence on the width
(`the_composed_constraints_yield_the_plastic_number_root`) — *lower* than either constraint alone, so
over-constraining lowers the root. The growth root is a functional of the whole constraint set. Nor is the
map confined to one dimension: a 2×`m` grid strip (treewidth 2) has a 3-state transfer matrix with
characteristic polynomial `(λ+1)(λ² − 2λ − 1)`, so its independent-set carry grows at the **silver ratio**
`1 + √2 ≈ 2.414` per column (`√(1+√2) ≈ 1.554` per variable), the exact recurrence `f(m) = 2f(m-1) + f(m-2)`
verified (`the_grid_strip_independent_set_root_is_the_silver_ratio`) — the 1D golden ratio lifting to a metallic
one as the structure gains a dimension. The residue's `Θ(n)`-treewidth expander is where this dimensional
progression maxes out. In every case the root is not merely read off a measured width sequence — it is
*derived* from the combinatorial structure: the Perron eigenvalue of the carry's transfer matrix, computed by
power iteration on the constraint automaton with no cofactor enumeration, agrees with the width-sequence root
to `10⁻³` for run-length `φ`/tribonacci/tetranacci and the grid's silver ratio alike
(`the_growth_root_is_derived_from_the_transfer_matrix_eigenvalue`) — the two computations are one number.
And these roots are **Pisot numbers**: algebraic integers `> 1` whose conjugates lie strictly inside the unit
disk — `φ`'s conjugate is `1−φ ≈ −0.618`, silver's is `1−√2 ≈ −0.414`
(`the_metallic_growth_roots_are_pisot_conjugate_inside_unit_disk`) — which is exactly why Binet's closed form
`w(n) = C·ρⁿ + (conjugate terms)` locks the width to `ρⁿ`: the conjugate terms vanish. The honest scope: by
Lind's theorem the growth rates of regular / subshift-of-finite-type languages are the **Perron** numbers
(spectral radii of non-negative integer matrices), a class strictly broader than Pisot — a Perron number may
have a conjugate of modulus between 1 and itself. These simple metallic/`k`-bonacci families realize the Pisot
*subset*, whose conjugates lie strictly inside the disk, which is exactly why their Binet conjugate term
vanishes and the width locks cleanly to `ρⁿ`. So "Binet's roots" for the analyzable families is a Pisot
dominant root with a vanishing conjugate, sitting inside the general Perron class — and that class is genuinely
larger: the companion matrix of `x³ − 3x − 1` (recurrence `a(n) = 3a(n-2)+a(n-3)`) has Perron root `≈ 1.879`
with a conjugate `≈ −1.532` *outside* the unit disk, Perron but not Pisot
(`the_growth_roots_are_perron_not_only_pisot`), so the growth-root class is the Perron numbers, dense in
`[1, ∞)`, with Pisot the special subset where Binet's conjugate term vanishes. A single real family makes the
descent visible: the min-gap-`g` carry (1s at least `g` apart, root the dominant zero of `x^g − x^{g-1} − 1`)
gives `φ` at `g=2`, descends through the Pisot numbers, and lands *exactly* on the plastic number at `g=5`
(the factorization `x⁵ − x⁴ − 1 = (x³ − x − 1)(x² − x + 1)` pins it to the smallest Pisot), then falls below it
— `1.285` at `g=6`, `1.255` at `g=7` — into Perron-non-Pisot roots
(`the_min_gap_roots_descend_through_plastic_into_perron_non_pisot`), the family walking the growth-root
spectrum down past the Pisot floor.
This makes the entire poly-vs-exp dividing line a single spectral fact: `root = 1` iff the transfer matrix's
Perron eigenvalue is **exactly 1**, the polynomial growth `n^d` coming from a size-`(d+1)` **Jordan block** at
eigenvalue 1, not from a spectral radius above it — the bounded-subset-`s` carry (lower-bidiagonal, all
eigenvalues 1) gives Perron eigenvalue `1` for every `s`, while run-length gives `φ, ` tribonacci, tetranacci
above 1 (`the_polynomial_carry_has_perron_eigenvalue_one_the_dividing_line`). So "which structural feature
forces root 1 vs `> 1`" is, exactly, spectral radius 1 of the carry's transfer matrix, and the polynomial
degree is the Jordan-block size; the coNP side is the spectral disc and the wall is everything outside it.
The word *carry* here is not a metaphor borrowed from arithmetic — it is the arithmetic carry. **Kummer's
theorem** makes the identification exact: the 2-adic valuation `v₂(C(a+b, a))` equals the number of carries
when `a` and `b` are added in base 2 (equivalently `s₂(a) + s₂(b) − s₂(a+b)` by Legendre), and **Lucas's
theorem** makes `C(n, k)` odd exactly when `k`'s binary digits are a submask of `n`'s — a carry-free
addition — so the number of odd entries in row `n` is `2^{s₂(n)}` (Sierpiński). The binomial magnitudes that
set the width and the base-2 carry structure Kummer counts are therefore one object, verified against both
theorems over the full small grid (`the_growth_root_carry_is_kummers_base_p_carry_via_lucas`): a Lucas-sparse,
bounded-`s` carry keeps `Σ_{j≤s} C(⌊n/2⌋, j)` polynomial (root 1); a carry that ripples `Θ(n)` times pushes
it to `2^{Θ(n)}` (root > 1). Binet reads the root off the recurrence's characteristic polynomial; Lucas and
Kummer read the same root off the base-`p` digits — one growth law, two lenses.
Tracking the scale-free exponent `log₂(width)/log₂ n` as `s(n)` grows, `s = 2` holds it flat at the
constant `s` (polynomial), `s = ⌈log₂ n⌉` makes it climb *linearly in `log n`* — quasi-polynomial
`n^{Θ(log n)}`, the boundary — and `s = ⌈√n⌉` or `⌊n/4⌋` make it explode (stretched- then full-exponential)
(`the_carry_set_cardinality_phase_transition`). The phase line between an efficiently-certifiable carry and
the wall is exactly `s ≈ log n`, one threshold on one scalar — and it is the *growth rate*, not a constant:
every constant multiple `s = c·log n` stays quasi-polynomial `n^{Θ(log n)}` (the scale-free exponent grows
`~c·log n` for `c = 0.5, 1, 2` alike, `the_carry_set_phase_line_constant_is_pinned`), so the whole
`Θ(log n)` band is sub-exponential and the wall is `s = ω(log n)`. The expander lower bound puts
near-threshold 3-SAT's carry at `s = Θ(n)`, far on the wall side.

That abstract scalar is a concrete, classical graph parameter. For a Tseitin formula the carry, read edge
by edge, is the set of vertices whose parity is still pending across the cut — so the carry-set cardinality
is the constraint graph's cut-width, i.e. its treewidth. A path (treewidth 1) has a linearly-bounded
cofactor DAG at every length (`12, 16, 20, 24`), while a grid (treewidth `≈ w`) is already wider at equal
variable count (`63` vs `24` at twelve variables), and an expander (treewidth `Θ(n)`) is exponential
(`the_carry_set_cardinality_is_the_constraint_graph_treewidth`). So the growth-root dial is treewidth, in
the exact sense parameterized complexity means it — bounded treewidth is the tractable island — and the
three threads meet on one object: the residue's carry-set cardinality, the constraint hypergraph's
treewidth, and its spectral/boundary expansion are the same determinant, and random 3-SAT maxes all three.
Nor is there a decomposition escape past treewidth: rank-width and clique-width can be bounded where
treewidth is `Θ(n)` (a clique has treewidth `n`, clique-width 2), but random 3-SAT closes that door too —
the `GF(2)` cut-rank across a balanced variable partition is essentially full, `11, 17, 23, 30` of a
possible `≤ n/2` at `n = 24..60`, versus `2` for a path-structured control
(`the_cut_rank_rankwidth_is_also_high_for_random_3sat`). The full-rank cut is the algebraic fingerprint of
the expander; random 3-SAT is maximal for every width parameter of the constraint graph, not just treewidth.
And the same determinant reads as the symmetry group itself: a group acting on the prefix positions
collapses the `2^i` raw cofactors into orbits, and the growth root is how far they collapse — the full
symmetric group sends `2^i → i+1` (the Hamming-weight orbits, Burnside), so a symmetric function's width is
linear (root 1), while the trivial group of a rigid instance collapses nothing (root 2)
(`the_orbit_collapse_under_symmetry_is_the_growth_root_reducer`). In the spectral picture this is exact:
the rigid full-branching carry `[[1,1],[1,1]]` has spectral radius `2` (the wall), the orbit-collapse into the
Hamming-weight chain has spectral radius `1` (the coNP side), so §7's symmetry quotient is precisely the
operation that drops the transfer-matrix spectral radius from `2` across the dividing line to `1`
(`the_symmetry_orbit_collapse_is_the_spectral_radius_drop`) — symmetry is what pushes the carry into the
spectral disc, and the residue, rigid, never enters it. And this is why *no decidable quotient* rescues it: a
sound congruence on the cofactor DAG is an **equitable partition** of the transfer matrix, and by the
equitable-partition theorem its quotient has the *same* Perron eigenvalue — the 2×`m` grid's 3-state matrix
(silver `1+√2`) and its 2-state equitable quotient `[[1,1],[2,1]]` share the eigenvalue exactly
(`the_equitable_quotient_preserves_the_growth_root_only_semantic_can_cross`). So isomorphism, Weisfeiler–Leman,
and orbit quotients compress the *size* of the residue's representation but leave its growth *root* untouched;
its spectral radius is invariant under every local/structural quotient. Only a *non-equitable* congruence —
one merging states of different local structure but equal refutability — could cross the spectral-radius-1
line, and deciding whether such a semantic congruence exists is exactly the open cell, now stated spectrally:
the residue's spectral radius is `≈ 2` under every decidable lens, and pulling it to `1` is `NP = coNP`.
This sharpens *where* that question lives. No quotient — equitable or not — can even lower the root: the
equitable partition `{0},{1,2}` of the silver matrix preserves the eigenvalue `2.414`, while the non-equitable
lump `{0,1},{2}` gives `[[3,2],[2,0]]` with eigenvalue `4`, *over*-counting rather than reducing
(`the_quotient_cannot_reduce_the_root_only_extension_can`); lowering the root would require dropping reachable
cofactors, which misses refutation paths and is unsound. So the cofactor-quotient program is spectrally
blocked on the residue — the load-bearing "poly quotient width" is unattainable when the original root is `2`,
because a sound quotient preserves it. The one lever that changes the space rather than partitioning it is an
**extension** (adding variables — the `ER`/Frege move); the residue's open cell lives there, not in any
quotient of its cofactor DAG, which is exactly why a single extension variable moved nothing and the full
extension hierarchy remains the frontier. The mechanism is visible at the matrix level: an extension's
definitional constraint *forbids transitions* (prunes the automaton), and forbidding one transition of the
full-branching carry `[[1,1],[1,1]]` drops its Perron eigenvalue from `2` to `φ`, whereas partitioning its
states leaves the divisor `[[2]]` at eigenvalue `2` (`the_forbidding_a_transition_reduces_the_root_unlike_a_partition`).
Constraints move the root; partitions preserve it — so crossing the spectral-radius-1 line for the residue is
a search for the right transition-forbidding extension structure, not a partition, which is the open cell in
transfer-matrix terms. That "a single extension moved nothing" is sharpened by the state-of-the-art
*constructive* extension move. Bounded Variable Addition (Manthey–Heule–Biere) is extended resolution in
practice: it detects a clause grid — literals `LS` and remainders `RS` with every `(l ∨ R)` present, `|LS|·|RS|`
clauses — and replaces it with the `|LS|+|RS|` clauses `{(l ∨ ¬e)} ∪ {(e ∨ R)}` for a fresh extension `e`, the
equisatisfiable definition that shrinks structured formulas exponentially. Run as an oracle it earns its
validation (a `4×4` grid collapses `16 → 8`, pigeonhole `81 → 66` at `m = 6`, UNSAT preserved), yet on the
residue's `Incompressible` cores it introduces **zero** extension variables — the clause count is unchanged at
`n = 6, 7, 8`, and where bounded-width resolution already fails to refute (only `1/5` cores at `n = 7`) the
BVA-extended formula fails identically, because there was nothing to add
(`the_bva_extension_construction_vs_the_residue_width_barrier`). The expander has no dense repeated
substructure for even optimal extension *placement* to exploit — which is the structural reason the residue
resists extended resolution, now certified against the real algorithm rather than one hand-placed variable.
Underlying all of this is a uniformity gap that *is* Cook–Reckhow. The whole
spectral/growth-root apparatus is a uniform-family tool: one transfer matrix — a single Perron root, a
deterministic recurrence, zero variance — generates the width at every `n`. The analyzable families have that;
the residue does not. Its cofactor width is a *distribution*: twelve `Incompressible` cores at `n = 6` give
widths spread over `[16, 30]` with variance `13.9`, no single generating automaton
(`the_residue_cofactor_width_is_a_distribution_not_a_uniform_family`). That non-uniformity is why no single
spectral root, order, or quotient captures the residue, and whether random 3-SAT admits a *uniform* poly
certificate — one construction for all `n` — is exactly Cook–Reckhow, the open cell. Carry-set cardinality, Nerode index,
treewidth, rank-width, spectral/boundary expansion, and symmetry-orbit collapse are one object seen six
ways; the structured families are compressible along it and the residue is rigid, maximal on every reading.
This is quantitative, not narrative: computed on the same graded-structure instances, three of the readings
agree — a path Tseitin gives cofactor width `24`, cut-rank `1`, automorphism group `2`; a grid `63`, `3`,
`128`; a random 3-CNF `394`, `4`, and automorphism group `1` — rigid
(`the_determinant_readings_correlate_on_individual_instances`). Width and rank rise with structurelessness
while the symmetry group collapses to the trivial one for the random instance; the views point the same way
on individual instances, not merely each report "hard" in isolation.

A seventh reading reaches a different algorithmic family entirely. The minimal UNSAT core of near-threshold
random 3-SAT grows linearly — `19.6, 29.4, 38.8, 47.0` clauses at `n = 12..24`
(`the_local_consistency_radius_minimal_core_is_large_for_random_3sat`) — so the contradiction is not
localizable: every sub-`2n` subset is satisfiable, and one must inspect `Θ(n)` clauses to see UNSAT. That
local-consistency radius is the Sherali–Adams / LP-hierarchy rank, so the LP and SDP relaxation hierarchies —
not resolution, not polynomial calculus, not Frege — also need `Θ(n)` rounds and also fail. The determinant
is one object across combinatorics, proof complexity, and convex relaxation alike, and random 3-SAT maxes it
on every axis measured.

Two consequences pin the residue. First, the root is a property of the *order and quotient*, not the
function alone: inner-product `⊕_i x_i y_i` has a running-parity carry of width 4 under the paired order
`x_1 y_1 x_2 y_2 …` (root 1) but a width-`2^m` carry under the separated order (root 2) — the same
function, both roots (`the_carry_monoid_law_is_myhill_nerode_compression_over_all_families`). Second, the
solver dispatcher *is* the library of carry-monoid recognizers: the `GF(2)`/mod-`q` routes solve group
carries by linear algebra over a field, the symmetry routes exploit the `Bₙ` action, cutting-planes/SoS
the ordered-field carry — each route a certificate format with a poly-size carry, so every structured
family lands in some format and the residue alone falls through to `Incompressible`
(`the_solver_routes_are_the_carry_monoid_recognizer_library`). The carry dimension is thus *format-relative*,
and pigeonhole is the quantitative witness: its cofactor-DAG width is exponential (`20, 81` at `m = 3, 4` —
the OBDD format is root > 1 for PHP), while its auto-discovered lex-leader symmetry certificate is polynomial
(`2, 7, 19, 49` PR steps at `m = 3..6`, each re-checked by `check_pr_refutation` — the `Bₙ` format is root 1),
the same family carrying two different dimensions and the smaller one making it easy
(`the_carry_dimension_is_format_relative_php_exp_cofactor_poly_symmetry`). coNP-easiness is the minimum carry
dimension over all formats the library holds, and the residue is `Θ(n)` in every one of them. In this frame
`3-SAT ∈ coNP` is precisely
the statement that random 3-SAT's cofactor carry can be bounded to `s = O(\mathrm{polylog})` under some
order and quotient — a poly-index Nerode/SR congruence, the §4.3 open cell.

The residue's incompressibility is then measured against *every decidable lever*, and each moves only a
constant factor. Exhaustive variable reordering (all `n!` orders at `n ≤ 7`) buys a `~1.46×` factor while
the best-order width still grows `8.6 → 21.3` across `n = 4..7`
(`the_residue_variable_order_search_is_exhaustive_at_n6`, `the_residue_best_order_width_scaling`); stacking
the CofactorIso quotient on the best order adds essentially nothing (`16.2 → 16.2` at `n = 6` — order and
iso are redundant, `the_order_and_quotient_levers_stack_but_dont_bound_the_residue`); the `GF(2)`
incidence rank of the residual formulas grows far slower than their count but matches a random 3-CNF
control (`0.55` vs `0.53`), so that gap is a generic small-formula artifact, not residue structure, and in
any case incidence-XOR is not a sound proof step (`the_incidence_rank_gap_is_artifact_or_structure`). None
of these changes the growth root. The honest ceiling is the enumeration scale — the `s = Θ(n)` degree wall
of the dense/expander regime lives beyond `distinct_width`'s reach, so the small sparse cores measured here
report a slowly-growing effective `s` that is *not* an asymptotic claim. What is proven is the shape of the
answer: the root is the carry-set cardinality, the decidable levers are constant factors, and the one lever
that could bound `s` is the open cell.

One lever does move the residue where the others fail, and it sharpens the open cell into two measurable
quantities. `GF(2)` Nullstellensatz degree is invariant under an *invertible* linear change of basis (200
random bases lower none of it — `the_linear_basis_search_for_low_ns_degree_on_the_residue`, on a
substituted-polynomial checker calibrated to reproduce the standard NS exactly), but it *drops* under a
linear *projection* — the identification `x_a = x_b`, a dimension-reducing case-split that restricts to a
lower-degree subformula. Recursing sound identification-dichotomies until every leaf has bounded NS degree
turns each residue core into a small tree of bounded-degree Positivstellensatz-calculus leaves: `2–8` leaves,
every leaf degree `≤ 3`, so the certificate is `O(\text{tree} \cdot n^{\max\deg})` — polynomial *in the
accessible regime* (`the_sound_projection_tree_to_bounded_ns_degree`). Across `n = 5,6,7` both metrics stay
flat — tree size `5.0 → 4.2 → 3.7`, max leaf degree `2.6 → 2.6 → 3.0` (`the_projection_tree_scaling`). This
is the sharpest positive statement the instruments reach: the residue *has* a small bounded-degree PCR
certificate wherever we can compute one, and `3-SAT ∈ coNP` reduces to whether the projection tree size and
the max leaf degree both stay polynomial as `n` grows into the dense regime — the same open cell, now two
concrete numbers rather than one.

That positive statement is regime-specific, and pushing it onto a provably-hard family exposes the wall.
The rigid cores at `n ≤ 7` are the resolution-*easy* regime; near-threshold random 3-SAT (ratio `4.26`)
has `GF(2)` Positivstellensatz-calculus degree `Θ(n)` (Ben-Sasson–Impagliazzo). Run against it, projection
*fails to move it* — no identification lowers its NS degree, so the tree is a single leaf at the formula's
own degree — and that degree **climbs**: median `4, 4, 5, 5, 5, 5, 5` across `n = 8..14`, incrementing
`4 → 5` by `n = 10` and then held flat only by the enumeration ceiling
(`the_projection_tree_on_near_threshold_random_3sat`, `the_near_threshold_ns_degree_scaling`). This is the
first *growth* the instruments see — everything prior was flat under the small-core cap — and it lands
exactly where theory says the wall is: on the hard family the projection certificate's leaf degree grows
with `n`, so `n^{\max\deg}` is not polynomial, and the bounded-degree certificate of the previous paragraph
is a property of the easy regime, not of `3-SAT`.

The enumeration ceiling itself is then pushed. A degree-`d` refutation lives in the degree-`≤ d` monomial
basis — `\sum_{k ≤ d} \binom{n}{k} = \mathrm{poly}(n^d)`, not `2^n` — so a bounded-degree checker (built on
the substituted-polynomial layer, calibrated to agree with the full NS degree exactly,
`the_bounded_ns_calibrates_against_the_full_ns_degree`) certifies "NS degree `≤ d`?" at `n` well past the
full-enumeration wall. On near-threshold random 3-SAT it flips exactly as a `Θ(n)` degree must: degree `> 4`
for every `n ≥ 12`, and degree `≤ 5` at `n = 12, 15` but `> 5` at `n = 18`
(`the_bounded_ns_degree_threshold_past_the_ceiling`). Stitched to the full-enumeration medians, the NS
degree of the hard family provably climbs `4 → 5 → ≥ 6` across `n = 8, 12, 18` — a machine-checked
refutation existence/non-existence at each step, now *past* the scale full enumeration can reach. The
`degree > d` half is not merely a failed search: each such lower bound carries its Positivstellensatz dual,
a pseudo-expectation `L` with `L(1)=1` and `L(m·p_C)=0` on every admitted generator, extracted from the
null space and re-checked by direct dot product against an independent verifier
(`the_degree_lower_bound_has_a_certified_pseudo_expectation`) — the wall is a checkable object, not the
absence of one.

The `for-all-n` statement is then reached structurally, where per-instance certificates cannot go. The
degree lower bound of Ben-Sasson–Impagliazzo is driven by boundary expansion, and expansion has a poly-time
spectral proxy: the second singular value of the *degree-normalized* clause–variable incidence operator. It
is bounded — `σ₂ = 0.70, 0.75, 0.78, 0.78` at `n = 20, 50, 100, 200`, converging to `≈ 0.78 < 1`, with the
top value pinned at `1` (`the_spectral_expansion_of_the_hard_family_scales`; the raw un-normalized ratio
drifts and is the wrong measure). A normalized spectral gap bounded away from `1` along the family is the
signature of a family of boundary expanders, and boundary expanders have PC/NS degree `Θ(n)` for *every*
`n`. So the two instruments meet: per-instance certificates exhibit the degree climbing `4 → 5 → ≥ 6` to
`n = 18`, and a poly-time structural quantity — computed to `n = 200`, far past any refutation-enumeration —
stays in the expander regime that forces `Θ(n)` degree at all scales. This is evidence *through* the
theorem, not a from-scratch `∀n` degree certificate; but it places the wall on a provably-hard family with a
quantity that does not stop at the enumeration ceiling. The bounded-degree projection certificate is a
property of the resolution-easy regime; the hard family's certificate degree grows, computed both ways.

The spectral proxy is confirmed by the combinatorial expansion it stands for, and that closes the section on
a single unifying quantity. Sampling the boundary `∂S = \{` variables in exactly one clause of `S \}` over
random clause-subsets, the minimum ratio `|∂S|/|S|` is bounded below and *grows* with `n` — `0.31, 0.92,
1.50` at `n = 50, 100, 200` (small subsets spread over more variables expand better,
`the_boundary_expansion_gives_both_proof_walls`). Spectral gap `≈ 0.78` and combinatorial ratio `≳ 1.5`
agree: near-threshold 3-SAT is a boundary expander at scale. And boundary expansion is the *single* hypothesis
behind two classical walls — PC/NS degree `Θ(n)` (Ben-Sasson–Impagliazzo) **and** resolution width `Ω(n)`,
hence size `2^{Ω(n)}` (Ben-Sasson–Wigderson). One computed quantity, to `n = 200`, places both the algebraic
and the logical proof-system wall on the hard family for all `n`. This is exactly why the open cell is where
it is: a certificate that stays polynomial on this family must beat *both* resolution and polynomial calculus
at once — it must live in Extended Resolution / Frege, the regime where no superpolynomial lower bound is
known for any family, and where by Cook–Reckhow a polynomially bounded system for all of `UNSAT` is `NP =
coNP`. The instruments here map the cell precisely and populate it with certified data on every side; they do
not, and cannot on their own, fill it.

One contrast makes the cell concrete. Pigeonhole wears its resolution wall on the surface — CDCL conflicts
`6, 27, 140, 757, 2921` for `m = 4..8`, a clean `≈ 4.5×`-per-pigeon exponential, where random 3-SAT hides
the same wall behind the accessible-scale ceiling — yet the dispatcher routes every `PHP(m)` to its symmetry
format and closes it in a polynomial certificate (`the_php_resolution_blowup_vs_the_symmetry_format`). That
is the whole thesis in one line: a format beating resolution *exists* precisely when the instance carries
structure a recognizer can name — pigeonhole's counting symmetry is a bounded carry in the symmetry format,
and the `Pigeonhole` route is its recognizer. The residue and random 3-SAT are the complementary case: rigid
under every instance symmetry, structureless, a bounded carry in no format the library holds. In the language
this section has built, `3-SAT ∈ coNP` is the statement that even *those* have a bounded carry in *some*
format — and the open cell is exactly the search for a recognizer the dispatcher does not yet contain.

The contrast is not a single example but a clean partition of the whole zoo. Run the dispatcher across the
classic resolution-hard families and every one is recognized by a format that beats resolution:
pigeonhole, weak-PHP and clique-coloring route to the symmetry recognizer; functional- and onto-PHP,
mutilated-chessboard and modular counting to the counting recognizer; the ordering principle to `GF(2)`;
pebbling to Horn (`the_structured_hard_families_the_dispatcher_covers_or_misses`). Not one structured
resolution-hard instance falls through to `Incompressible` — the recognizer library has no gap on
structured families. And recognition is not the end of it: each such family yields an actual short proof
stream — `19` steps for `PHP(5)`, `49` for `PHP(6)`, `2` for the mutilated chessboard — that an
*independent* verifier accepts (`certified_unsat_auto` composed then re-checked by `check_pr_refutation`,
`the_structured_families_have_zero_trust_verified_certificates`). The `coNP` certificate is real and
zero-trust, not merely a routing decision. What falls through is exactly and only the structureless pole: the `Bₙ`/`AGL`-rigid
residue and near-threshold random 3-SAT. So the boundary this whole paper circles is drawn sharply here —
in-coNP-with-a-recognized-certificate on one side, the open cell on the other, and the line between them is
precisely structure versus its absence. There is no further structured recognizer to add; the only cell
left open is the incompressibility pole, and it is open for the reason §2 makes exact — most strings are
incompressible — now carrying the full weight of the `NP = coNP` question.

That open cell is narrower still than "random 3-SAT," and the last measurement pins its width. Refutation
cost is not flat across clause density: at fixed `n` the median CDCL refutation collapses monotonically as
the ratio climbs — `15, 13, 13, 11, 8, 6, 5` conflicts for ratios `4.0 … 16` at `n = 22`
(`the_refutation_hardness_peaks_at_threshold_and_over_constrained_is_easy`). Over-constrained instances are
efficiently refutable — fast in practice, and above `ratio = Ω(\sqrt n)` a *polynomial* spectral certificate
exists (Feige–Ofek; Goerdt–Krivelevich) — and below the threshold the instances are satisfiable, an `NP`
witness. So certificates are known on both sides, and the open cell contracts to the narrow constant-ratio
band at the satisfiability threshold: the structureless, `Bₙ`-rigid, expander core, where and only where no
polynomial certificate is known. That band is also exactly the *barely-unsatisfiable* one: the criticality —
the fraction of clauses whose single deletion restores satisfiability — is `0.054, 0.023, 0, 0, 0` for
ratios `4.5, 5, 6, 8, 12` at `n = 24`, nonzero only just above the threshold and identically zero once the
formula is robustly over-constrained (`the_open_cell_band_is_barely_unsat`). The exact distance to
satisfiability confirms it: the MaxSAT deficiency — the minimum over all `2ⁿ` assignments of the number of
violated clauses — is `1.00, 1.50, 4.25, 8.25, 13.75` for ratios `4.5 … 16` at `n = 16`, precisely `1` at
the threshold and climbing with density (`the_distance_to_sat_grows_with_density`). At the threshold the
instance is a single clause short of satisfiable, so a refutation must certify that *no* assignment satisfies
all when assignments satisfying all-but-one exist; above the threshold the many redundant, overlapping
reasons for unsatisfiability give a solver an easy target, and every reason at the threshold is load-bearing.
Barely-unsatisfiability is necessary for the cell but not sufficient, and pigeonhole draws the line: it is
*also* barely UNSAT — `PHP(4)` and `PHP(5)` have MaxSAT deficiency exactly `1`, a single clause short — yet
easy, recognized and certified through automorphism groups of size `144` and `2880`
(`the_pigeonhole_is_barely_unsat_but_easy`). The same razor-thin gap is trivial when a rich `Bₙ` symmetry
collapses it and open only when no symmetry can. Here honesty requires care about *which* structurelessness:
automorphism-rigidity (`aut = 1`) is not enough. A `barely`-unsatisfiable `aut = 1` random instance at
`n = 14` still routes to the semantic-symmetry recognizer and is easy
(`the_hardness_2x2_is_barely_unsat_times_rigidity`); the load-bearing notion is the stronger
`Incompressible` — no local, semantic, *or* algebraic symmetry of any kind. Honesty demands a second
refinement here, because at *accessible* scale this notion is not exhibitable: measured over automorphism-rigid
near-threshold minimal cores at `n = 6`, **0 of 30** route to `Incompressible`
(`the_incompressible_cores_exist_among_aut1_near_threshold`) — every syntactically rigid core still carries
a *semantic* symmetry the recognizer library exploits, so `Route::Incompressible` essentially never fires on
finite random 3-SAT one can enumerate. The arsenal's `Incompressible` verdict and the census residue
(`Bₙ`/`AGL`-rigid, full-NS-degree) are therefore *distinct* structurelessness notions that diverge at finite
`n`: the census isolates the proof-complexity-hard family that `Incompressible` names only in the limit. The
cell is the conjunction — barely-unsatisfiable *and* structureless in that strong sense — but it is an
**asymptotic** object: the two kernel poles meet on one family, and neither an automorphism count, a single
density statistic, nor a finite arsenal verdict captures it alone. The instruments locate the cell to that
band and populate every side of it with certified data; the band itself is the `NP = coNP` question.

This also refines what "one determinant" means, and honesty demands the refinement be stated carefully,
because a first attempt at it over-reached. The whole-formula expansion *strengthens* with density — the
normalized spectral gap `σ₂` runs `0.785, 0.763, 0.741, 0.705, 0.678` for ratios `4 … 16` at `n = 80`, a
denser random graph being a better expander (`the_spectral_expansion_vs_density_is_distinct_from_hardness`) —
the opposite direction to the *practical* refutation cost, which drops as the ratio climbs. It is tempting to
attribute the drop to the contradiction localizing into a smaller core, but that is false: the minimal-UNSAT
core size is essentially flat in density — `40.4, 39.2, 37.2, 39.8, 39.8, 36.6` at ratios `4 … 16` for
`n = 20`, `≈ 2n` throughout (`the_minimal_core_is_threshold_peaked`). What actually eases with density is
*search*: more clauses mean more immediate propagations and conflicts, so a solver *finds* a refutation
faster even though the minimal one is no smaller, and above `ratio = Ω(\sqrt n)` an outright spectral
certificate appears. The clean asymptotic statement lives at the threshold and comes from the certified
degree growth and the expander bound, not from core size or from any single-density expansion number. The
determinant is one object in that asymptotic sense at the threshold; the density sweep is a caution against
reading a fixed-`n`, single-instance number as the invariant. Artifacts: `tests/cofactor_family_counting.rs`,
`tests/cofactor_mirror.rs`.

## 5. Certified degree lower bounds

**5.1 Dual witnesses.** A degree-`d` lower bound is a `GF(2)` pseudo-expectation `L` (`L(1)=1`,
`L(m·p_C)=0` for every admitted generator), produced by the engines and re-checked independently —
zero trust in the solver. Artifacts: `ns_lower_bound_witness` / `check_ns_lower_bound` (clause),
`ns_lower_bound_witness_polys` / `check_ns_lower_bound_polys` (any generator system, 63 variables),
`ns_lower_bound_witness_polys_on_basis` (the support probe).

**5.2 Pigeonhole (clause encoding): exact, growing, non-width degree, with a uniform witness.**
`NS-degree(PHP_m) = 2(m−1)`, certified exactly at `m = 3, 4`; the uniform parity-aware witness
`L(M) = [M hole-injective]` proves the bound for **every** `m` — the at-most-one generators vanish on
hole collisions, and each pigeon clause contributes `2^{|U|} ≡ 0 (mod 2)`, a parity that fails over
characteristic 0 (the classical partial-matching support does *not* carry the `GF(2)` witness).
Artifacts: `pigeonhole_has_certified_growing_non_width_ns_degree`,
`uniform_parity_aware_witness_proves_php_degree_bound_for_all_m`,
`pigeonhole_witness_structure_differs_over_gf2`.

**5.3 The characteristic-2 obstruction.** Over ℚ the Reynolds operator makes "the extremal witness is
symmetric" free; over `GF(2)` the unnormalized sum annihilates `L(1)=1` whenever `|G|` is even — so
symmetric witnesses must be exhibited *natively*, and (new, §7) sometimes cannot exist at all while
asymmetric ones do. Artifact: `over_gf2_symmetrizing_a_proof_annihilates_when_the_group_is_even`.

**5.4 Modular counting (linear encoding): two provably distinct regimes.** `Count_3(n)` (blocks of 3,
UNSAT iff `3 ∤ n`):

- **Dense degenerate regime `n < 2q` (`n = 4, 5`): exact degree 2.** Below `n = 6` every two blocks
  intersect; `x_f·P_i ≡ x_f` mod the pairs for `i ∉ f`, and `Σ_i P_i = n + Σ_e x_e (mod 2)` closes
  it. Degree 1 is impossible (no point subset meets every block evenly). Both halves certified.
- **Genuine regime (`n = 7, 8` — 35 and 56 variables): exact degree 3.** The certified dual witness
  at degree 2 (non-width: it exceeds every generator's degree) plus refutation at 3.

Artifacts: `count_two_is_char_matched_and_falls_to_low_degree_gf2_ns` (the char-matched control:
`Count_2` falls at degree **1** — summing the point generators cancels every edge `q = 2` times),
`count_three_has_certified_growing_non_width_ns_degree_over_gf2`,
`count_three_scale_probe_measures_the_degree_growth`.

**5.5 The witness-support schedule — Lucas arithmetic in the certificate.** The `Sₙ`-invariant
degree-2 pseudo-expectation for `Count_3` on the partial-partition support (pairwise-disjoint
blocks) is forced onto type values, and the point constraint contributes `1 + C(n−4,2)·b₀` — so the
invariant witness exists **iff binomial parities align: `n ≡ 3 (mod 4)`**. On schedule (`n = 7`) the
explicit indicator `L(M) = [M pairwise disjoint]` is valid; off schedule (`n = 8`) *all four*
invariant candidates on that support are machine-refuted while the sub-basis search still finds a —
necessarily asymmetric — witness. Artifact:
`count_three_witness_support_structure_is_probed_on_sub_bases`.

**5.6 The characteristic axis: a general `GF(p)`/`GF(4)` engine, anchored at `p = 2`.** Everything
above fixes the coefficient field at `GF(2)`. The general engine (`polycalc_gfp`) makes it a runtime
parameter — every prime field, plus `GF(4)` (the 2-bit *extension* `GF(2)[ω]/(ω²+ω+1)`; the ring
`ℤ/4` is not a field, `2·2 = 0`) — with the arithmetic the `GF(2)` engine silently conflates now
explicit: the clause false-indicator of a positive literal is the *signed* `1 − x` (over `GF(3)`:
`1 + 2x`, pinned corner-by-corner), and the partition-of-unity atom `(1 − x) + x = 1` holds in every
field — so the §2.1 completeness construction is field-generic: total, sound, and fail-closed over
`GF(3)`, and the witness↔refutation duality carries over verbatim, with zero-trust checkers that
reject adversarially corrupted witnesses. The `GF(2)` engine stays as the specialized fast path
(bit-packed rows, 64 basis columns per word); the general engine is pinned to it by a `p = 2`
differential — identical verdicts at every degree, witness existence both ways, and each engine's
witnesses passing the *other's* checker. The `GF(3)` coefficient field is itself kernel-formalized
(`tests/gf3_ring_kernel.rs`): `add3`/`mul3`/`neg3` as computable terms over a three-constructor
inductive, the identities, characteristic 3, additive inverses, and the signed atom as kernel
theorems — with the characteristic-2 law `a + a = 0` *rejected* over `GF(3)` while the true doubling
law `2a = −a` is accepted, so the kernel genuinely separates characteristics. Artifacts:
`gf3_clause_polynomial_is_the_signed_false_indicator_on_every_corner`,
`the_partition_of_unity_atom_is_one_over_every_prime_field`,
`the_general_engine_at_p_two_agrees_with_the_specialized_gf2_engine`,
`build_ns_certificate_gfp_is_total_sound_and_fail_closed_over_gf3`,
`gf3_degree_lower_bounds_are_certifiable_and_dual_to_refutation`,
`gf3_ring_laws_are_kernel_theorems`, `a_false_gf3_law_is_rejected_by_the_kernel`.

**5.7 Prime incomparability, certified in both directions.** Each characteristic is a *different*
proof system, and they are pairwise incomparable — the classical fact, now a pair of re-checkable
certificates on one family pair. Side 1: `Count_3`'s linear point generators telescope over `GF(3)`
(every 3-block meets exactly 3 points, so `Σ_i P_i = −n`, a nonzero constant whenever `3 ∤ n`,
asserted as polynomial algebra) — a degree-**1** refutation at every scale checked (`n = 4, 5, 7`,
the last at 35 variables) — while over `GF(2)` the same family carries certified exact degree 2
growing to `≥ 3` (§5.4). Side 2, the mirror: `Count_2` falls at degree 1 over `GF(2)` (§5.4's
char-matched control) but has certified exact `GF(3)` degree `2 (n=3), 3 (n=5), 3 (n=7)` — dual
witness below, refutation at, every point — the same wide-tread staircase as the `GF(2)`-side
`Count_3` profile (`2, 2, 3, 3`), climbed from the opposite characteristic. And the audit datum: on
the mod-3 Tseitin expander (total charge `2 ≡ 0 (mod 2)`, so the parity cut is *structurally* blind),
the entire certified `GF(2)` ladder reports `BeyondBudget` while the recovered mod-3 system refutes
by one `GF(3)` Gaussian pass with a re-checkable combination. Artifacts:
`count_three_falls_to_degree_one_over_gf3_but_has_certified_growing_degree_over_gf2`,
`count_two_falls_to_degree_one_over_gf2_but_has_certified_growing_degree_over_gf3`,
`count_two_scale_probe_measures_the_gf3_degree_at_scale`,
`mod3_tseitin_is_gf3_easy_and_its_gf2_route_is_the_audit_gap`.

**5.8 Extension fields collapse: `GF(4)` is `GF(2)`, constructively.** Shifting `GF(2) → GF(3)`
changes everything (§5.7); shifting `GF(2) → GF(4)` changes *nothing* — NS degree depends only on
the characteristic, so the "field ladder" is really the **prime ladder**. Measured exhaustively:
`GF(4)` arithmetic field-checked over every tuple (including Frobenius `x⁴ = x` and the `ℤ/4`
zero-divisor contrast), then identical refutation verdicts and minimum degrees across every
minimal-UNSAT orbit at `n ≤ 3` plus PHP(3) and `Count_3(4)` — 50 covers, every degree. And
constructively: a provenance-tracked `GF(4)` certificate at the minimal degree projects
coefficient-wise through the `GF(2)`-linear `λ : a + b·ω ↦ a` (clause polynomials have
prime-subfield coefficients, so `λ` slides through `Σ_C p_C·g_C = 1`) to a `GF(2)` certificate of no
larger degree that re-checks — ring and corner-wise. Artifacts:
`gf4_arithmetic_satisfies_the_field_axioms_exhaustively`,
`ns_degree_over_gf4_equals_ns_degree_over_gf2_across_the_small_census`,
`a_gf4_certificate_projects_coefficientwise_to_a_rechecking_gf2_certificate`.

**5.9 Pigeonhole across characteristics: the bound is invariant, the support is threshold-graded.**
The general engine refines §5.2 twice. First, the *bound*: the hole-injective indicator is a valid
pseudo-expectation at **every** characteristic — with signed indicators the pigeon-clause pairing
telescopes as `(1−1)^{|A|}·(1−1)^{|U|} = 0` in any field, and §5.2's parity identity
`Σ 1 = 2^{|U|} ≡ 0 (mod 2)` is exactly its characteristic-2 shadow (the signs are invisible there).
Checked over `GF(2)`, `GF(3)`, `GF(5)` at `m = 3, 4`; and PHP(3)'s `GF(3)` NS degree is *exactly*
`4 = 2(m−1)` — measured, both halves, duality pinned at the threshold — equal to `GF(2)`. Counting
is orthogonal to linear algebra over every field: pigeonhole's hardness is protected by its
permutation symmetry, not by any characteristic (the §4 dichotomy, deepened). Second, the *support*:
where the characteristic does bite is *which* sub-bases carry the witness. The classical
partial-matching support fails over `GF(2)` (§5.2), fails over `GF(3)` at `m = 4`, and works over
`GF(5)`, `GF(7)` — across all eight measured (prime, `m`) points the law is a **threshold**: the
classical support survives iff `p ≥ m`, with `GF(2)` merely the deepest failure. Small primes
annihilate the counts the classical argument divides by — the same small-prime-divides-a-binomial
mechanism as §5.5's Lucas schedule. The hole-injective support carries the witness everywhere (the
control). Artifacts: `the_hole_injective_indicator_is_a_pseudo_expectation_at_every_characteristic`,
`php_gf3_ns_degree_is_measured_and_its_lower_half_certified`,
`the_php_witness_support_structure_differs_by_characteristic`.

**5.10 Annihilation is `p | |G|`, Reynolds is real elsewhere — and the ladder gains its
characteristic rung.** §5.3's obstruction generalizes exactly: the unnormalized group-sum evaluates
on the constant monomial to `|G| · L(1)`, so symmetrizing kills a witness iff `char | |G|` — "even
groups over `GF(2)`" was the `p = 2` case. On PHP(3), crossing the pigeon-transposition group `C₂`
and the pigeon-3-cycle group `C₃` with `GF(2)` and `GF(3)` annihilates in precisely the two
divisible cells and survives in the other two — where the **Reynolds operator** (`|G|⁻¹ · Σ_g g·L`)
is machine-verified to average a solver-found witness into a valid, `G`-invariant one: the
constructive branch a `GF(2)`-only study can never exhibit (every even group kills it there), now
exhibited in both directions (`C₂` over `GF(3)`, `C₃` over `GF(2)`). Finally, the census's certified
ladder (§3) gains the rung its `router_beats_ladder` audit flagged as missing:
`ProofRung::ModCount { p }` — a recognized one-hot mod-`p` CNF whose `GF(p)` Gaussian refutation
re-checks — via `weakest_crushing_rung_with_char`, with the legacy cascade regression-pinned
byte-identical, and, across the census at `n ≤ 3`, the extended ladder differing from the legacy
only by independently re-verified `ModCount` placements. Artifacts:
`over_gfp_symmetrizing_annihilates_exactly_when_p_divides_the_group_order`,
`the_reynolds_operator_produces_a_valid_symmetric_witness_when_the_order_is_invertible`,
`mod_p_one_hot_instances_land_on_the_modcount_rung_of_the_extended_ladder`,
`the_characteristic_rung_closes_the_router_ladder_audit_gap`.

**5.11 Past the fields: Nullstellensatz over the RINGS `ℤ/m`.** The characteristic axis exhausts the
finite fields; what remains of "any modulus" are the rings with zero divisors — `ℤ/6` (idempotents),
`ℤ/4` (a nilpotent), and their composites — and the engine (`polycalc_zm`) covers them at arbitrary
degree, replacing Gaussian elimination (dead without inverses) with a **Howell-style echelon**: gcd
pivoting normalizes every pivot, by a unit, to a divisor of `m`, and each pivot recursively spawns
its annihilator multiple `(m/d)·row` — validated against an *exhaustive all-combinations oracle*
(every one of the `m^rows` combinations enumerated, membership compared on every vector of
`(ℤ/m)ⁿ`) and against the field engine at prime `m`. Four theorems, all measured and pinned:

- **Completeness survives every modulus — "structureless" has no witness over any `ℤ/m`.** The
  partition-of-unity construction never divides: the atom `(1 − x) + x = 1` is a ring identity, the
  signed indicators cancel by additive inverses alone, and multilinear representations of cube
  functions are unique over any commutative ring (the Möbius transform is unipotent). So every
  unsatisfiable formula has a degree-`≤ n` certificate over **every** `ℤ/m`, `m = 2..12` swept —
  define hardness-as-structurelessness (*no certificate at any degree `≤ n`*) and **no finite
  formula fulfills it, over any modulus**: the §2.1 pole, ring-completed. The boundary travels with
  the theorem: the certificate lives in the `2ⁿ` basis (existence, not efficiency), and the *cost*
  pole is what §§5.2–5.9's certified lower bounds and §2.2's kernel incompressibility theorems
  establish — what is refuted here is the existence-form of randomness at finite `n`, not the
  asymptotic cost-form of NP-hardness, which no algebraic instrument decides (§8.2). Artifacts:
  `the_partition_of_unity_atom_is_one_over_zero_divisor_moduli`,
  `build_ns_certificate_zm_is_total_sound_and_fail_closed_over_z6_and_z4`,
  `no_finite_formula_is_structureless_over_any_modulus`.
- **Composite moduli intersect — they do not add.** CRT splits the coefficient ring and a
  certificate's coefficients split with it, so `ℤ/6`-NS refutes at degree `d` iff `GF(2)`-NS *and*
  `GF(3)`-NS both do (and `ℤ/12 = ℤ/4 ∧ ℤ/3` — coprime prime-power components in general), measured
  at every degree across the corpus. The composite ring is the **conjunction** of its parts — weaker
  than either: parity is `GF(2)`-degree-2 yet not `ℤ/6`-degree-2, and `ℤ/6`-degree(PHP(3)) `= 4 =
  max` of the component degrees. "Mod-6 reasoning" buys nothing for bounded-degree ideal membership;
  the Barrington–Beigel–Rudich mod-6 power lives in polynomial *representation*, a different
  question. Artifact: `zm_ns_at_coprime_composite_moduli_is_the_conjunction_of_its_parts`.
- **Ring dual witnesses, with a zero-divisor normalization.** Over `ℤ/m` the honest pseudo-
  expectation normalization is `L(1) ≠ 0` (`ℤ/m` is self-injective, so witnesses separating `1` from
  the span exist whenever no refutation does — but `L(1)` may be a zero divisor), and soundness is
  one field-free line: a refutation would force `L(1) = 0`. A prime witness **lifts**: `L =
  (m/p)·L_p` carries `GF(p)` lower bounds to `ℤ/m` with `L(1) = m/p` — the `GF(3)` parity witness
  certifies `ℤ/6`-degree > 2 for a family the `GF(2)` component refutes *at* 2 (the witness face of
  the conjunction). The checker rejects dropped normalizations, annihilated scalings, and perturbed
  constrained monomials. Artifact:
  `a_prime_witness_lifts_to_a_ring_witness_with_zero_divisor_normalization`.
- **Nilpotents make refutation strictly harder, never easier.** `ℤ/4` maps onto `GF(2)`, so `ℤ/4`
  refutations project (soundness, swept); the converse fails at fixed degree: parity and `Count_2(3)`
  are `GF(2)`-degree-2 but `ℤ/4`-degree-**3** — the measured Hensel tax (lifting `s ≡ 1 (mod 2)` to
  `s(2 − s) ≡ 1 (mod 4)` doubles the degree budget). This is why the CRT conjunction's components
  are `ℤ/pᵏ`, not `GF(p)`, and it inverts the naive "richer ring, stronger proofs" guess. Artifact:
  `z4_is_strictly_weaker_than_its_residue_field_at_fixed_degree`.
- **The missing fields are certifiably missing — by the same composer that builds the present
  ones.** Enumerate every abelian group of order `N` (the additive pieces) and every multiplication
  distributive over it (for a cyclic group, bilinearity forces `a·b = (ab)·u`; for a product group,
  the four generator products, each order-constrained), then judge the full field axioms per
  candidate, distributivity re-verified. At order 4 the composer *builds* fields on `ℤ/2 × ℤ/2` —
  one checked isomorphic to the engine's `GF(4)` table — and at order 9 on `ℤ/3 × ℤ/3`; at orders
  **6 and 10 every candidate over every presentation dies**, and the obstruction is composed in,
  not stumbled on: for coprime-order pieces, `ord(e₁·e₂) | gcd(2, 3) = 1` forces `e₁·e₂ = 0` in
  every distributive multiplication — composition at these orders *is* the CRT ring with its
  idempotent zero divisors, never a field. The prime-power classification, executable at the orders
  in question. Artifact: `finite_fields_compose_exactly_at_prime_power_orders`.

**5.12 The hardness-predicate ladder, decided object-by-object.** "Hardness" is a ladder of
predicates, not one — and with the axis complete (§§5.6–5.11), each rung now gets a certified
verdict: *unfulfillable* (no object satisfies it, proven universally) or *fulfilled* (a named
object satisfies it, proven by witness). **H_exist** — structureless: no certificate at any degree
`≤ n` over any coefficient ring — is UNFULFILLABLE: every UNSAT object certifies, over every
modulus (the §2.1 pole, ring-completed by §5.11). **H_max** — maximal cost at its size: NS-degree
exactly `n` — is FULFILLED at every tested `n` by the all-corners cube, simultaneously over
`GF(2)`, `GF(3)`, and the ring `ℤ/6`; and the *same* object also carries a verified structure
certificate — the two poles coexist in one formula. **H_grow** — cost growing without bound along a
family — is FULFILLED by pigeonhole, characteristic-invariantly. The theorem this pins: **the
existence-form and the cost-form of hardness provably split** — refuting the first (nothing is
random) neither refutes nor supports the second, whose finite shadows are certified *fulfilled*.
NP-hardness is the asymptotic cost predicate (Cook–Reckhow ties its proof-complexity form to NP vs
coNP); no rung of the ladder decides it, in either direction — the ladder's contribution is that
the distinction itself is now machine-checked rather than philosophical. Artifact:
`tests/hardness_witness_ladder.rs`
(`hardness_definitions_are_decided_object_by_object_across_the_ladder`).

**5.13 The Cost-Pole Attainment Theorem — the two poles fully proven, with their reasons.** The
ladder's H_max rung is upgraded from verdicts to a *uniform theorem with a certified argument*:

> **Theorem (attainment).** For every `n` and every coefficient ring `ℤ/m`: the all-corners cube
> `F_n` has Nullstellensatz degree exactly `n` — and simultaneously carries a verified structure
> certificate. Maximal hardness-as-cost is attained at every size, over every ring class, by an
> object that is provably not random.

The two halves and their proofs. *Upper half* (`≤ n`, "nothing finite is random"): the partition of
unity — kernel-certified `∀n` over `GF(2)` (§2.1, Nat recursor), with its per-variable engine, the
atom `(1 − x) + x = 1`, now a kernel theorem at **all four ring classes**: characteristic 2
(`gf2_ring_kernel`), characteristic 3 signed (`gf3_ring_kernel`), the nilpotent ring `ℤ/4` and the
idempotent ring `ℤ/6` (`cost_pole_kernel_seeds` — with the zero divisors `2·2 = 0`, `2·3 = 0` and
idempotents `3², 4²` themselves kernel-witnessed, so completeness is certified to survive the
failure of the field axioms, not to have smuggled them in); the executable construction is
certified total, sound, and fail-closed over every modulus `2..12` (§5.11). *Lower half* (`> n−1`,
the KILL-OR-ABSORB dichotomy): for every clause polynomial of `F_n` and every multiplier monomial,
the product either **dies** — the multiplier touches a positive literal and `x·(1−x) = x − x² = 0`,
a multilinear-quotient identity valid over every ring — or **absorbs** — `x·x = x`, the product
*is* the clause polynomial, degree `n` with unit top coefficient. No third case: asserted on every
one of the `8ⁿ` products per ring at `n = 2..7` with an independent multiplier implementation, so
the degree-`(n−1)` generator span is *empty* and no refutation below `n` exists over any `ℤ/m`; the
per-variable identities behind both branches are the kernel-seeded cube-point facts (`b·b = b`,
`b·(1−b) = 0`). The branches make no reference to `n` — the same two identities close every size.
And the branch invariants ride the same kernel ladder as the upper half: **dead-stays-dead**
(`∀n. D(n) = 0` — a killed product survives no further multiplication) and **the absorbed prefix is
the clause polynomial** (`∀n. M(n) = P(n)`) are kernel-certified `Nat` inductions from
model-checked per-step axioms — the exact `finite_randomness_kernel_integration` pattern — with a
negative control (dropping the annihilation axiom fails the kernel's type check). Artifacts:
`the_cost_pole_is_attained_at_every_n_over_every_ring_by_kill_or_absorb`,
`the_pou_atom_and_cube_point_seeds_are_kernel_theorems_over_z4_and_z6`,
`a_false_ring_law_is_rejected_by_the_kernel_over_z4_and_z6`,
`the_kill_and_absorb_invariants_are_kernel_certified_for_all_n`,
`skipping_the_annihilation_axiom_fails_the_kernel_type_check` (`cost_pole_kernel_ladder`).

## 6. Certified resolution-width lower bounds

Width-`w` resolution's derivable set is a finite least fixpoint; the fixpoint itself is the
certificate: a clause set containing the admissible axioms, closed under width-`≤w` resolution,
without `⊥` — re-checked with zero trust (`check_res_width_lower_bound`), under both width
conventions (axioms counted; axioms exempt — the convention under which wide-axiom families are
non-trivial). Differentially validated against the census's geometric oracle on every `n ≤ 3` orbit.
Measured and certified: PHP's wide-axiom width is `m−1` (2, 3 at `m = 3, 4` — growing); Tseitin
expanders exceed their axiom width (width 4 at 9 and 12 variables). Artifacts: `res_width.rs` —
`width_closure_matches_the_subcube_resolution_width_on_the_census`,
`resolution_width_lower_bounds_are_certified_by_the_closed_clause_set`,
`tseitin_expander_has_certified_resolution_width_lower_bounds`,
`php_resolution_width_certificate_completes_the_three_system_row`.

**6.1 The capstone: one group-theoretic number grades three certified proof systems.** §4 found PHP's hardness
protected not by affine/linear symmetry (it is rigid to both shears and symplectic transvections) but by
*permutation* symmetry — the group `Sₘ × Sₘ₋₁`, of arity `m`. That arity is not a label; it is the single
dial that sets the family's position in every hardness measure computed above, through certified chains with
no trust:

  arity `m`  →  symmetric-certificate depth `2m − 3`  →  Nullstellensatz degree `2m − 2`  →  resolution width
  `m − 1`,

each coordinate produced by a *different* proof system (the symmetry-invariant pseudo-expectation; the
Nullstellensatz dual; width-bounded resolution), each re-checkable, each strictly increasing with `m` (`3, 4`:
depth `3, 5`; degree `4, 6`; width `2, 3`). The first arrow is a closed form — **each unit of arity buys
exactly two units of certificate depth** — proven `∀m` by the uniform witness (§5.2); the second is the
witness↔refutation duality unit (`degree = depth + 1`, always); the third is the certified width bound of §6.
By Ben-Sasson–Wigderson the growing width forces super-polynomial resolution *size* — the classical
exponential lower bound the chain terminates in. This is the thesis of §4 — *symmetry = compression =
complexity* — discharged as an exact, executable, cross-system law: the amount of symmetry a family carries
determines the amount of hardness it has, end to end, in the kernel's own currency of certificates.

The honest boundary belongs in the statement, not a footnote. The chain lives entirely in the *structured /
symmetric* regime; it measures where a symmetric family sits between the two kernel poles of §2 (trivial
structure at degree 1, completeness at degree `n`). As `PROOF_SKETCH.md` records, *a fast algorithm for
structured or symmetric instances says nothing — NP-hardness lives in the worst case*: this law says nothing
about worst-case instances and is **not** a step toward P vs NP (§8). It is an exact result of the measurement
science, and it is stronger for being clear about which it is. Artifacts:
`the_symmetric_group_arity_grades_the_certificate_depth`, `the_ultimate_symmetry_to_hardness_chain_is_certified`.

## 7. The stabilization theorem, executable: ∀-scale verdicts from one finite computation

The centerpiece. A symmetric family at fixed degree `d` collapses onto **orbit types**: an invariant
functional is constant on monomial orbits; an orbit is named scale-independently by its canonical
structure (bipartite pigeon/hole graph; block-intersection hypergraph); and one constraint per joint
(multiplier-orbit × generator) representative captures the whole system — sound by the one-line
invariance lemma, machine-checked. Two independent implementations (the type-named representative
system and the everything-explicit direct solver) agree at every scale, degree and encoding, and
every positive verdict lifts to a `check_ns_lower_bound_polys`-passing witness. Artifacts:
`php_monomial_orbit_types_align_across_m_by_bipartite_graph_type`,
`an_invariant_functional_checked_on_orbit_representatives_is_checked_on_all_generators`,
`collapsed_dual_system_agrees_with_full_symmetric_ns_on_php_small_m`.

**The stabilization.** At fixed `d` the type set stabilizes, and each labeled entry of the collapsed
dual is a **bounded-degree integer polynomial in the scale** — fitted across a consecutive window in
the finite-difference basis, with window points beyond the fitting prefix as exact verifications
(the interpolation certificate), and with parity evaluable at *any* scale by Lucas
(`C(a,k)` odd ⟺ `k & a = k`). The `Count_3` machinery *finds* the hand-derived quadratic: an entry
with difference table `[1, 2, 1]` — `C(n−4, 2)`, parity period 4 — the origin of §5.5's schedule.
Both arithmetic lemmas (binomial parity periodicity; Newton-form interpolation) ride the kernel
ladder: rungs as exact integer sweeps with genuinely inductive steps, the `∀a` leap discharged by
the kernel's Nat recursor. Artifacts:
`collapsed_entry_counts_are_integer_polynomials_in_m_with_an_interpolation_certificate`,
`parity_of_binomial_entries_is_eventually_periodic_by_lucas`,
`binomial_parity_periodicity_is_a_kernel_theorem`,
`integer_polynomial_interpolation_by_finite_differences_is_a_kernel_theorem`.

**The verdicts** (`decide_invariant_witness_for_all_scales`; every window scale differentially
validated; artifact:
`fixed_degree_symmetric_ns_verdict_for_php_is_decided_for_all_m_by_finite_computation`):

| family, encoding, degree | period | invariant witness exists at |
|---|---|---|
| PHP, clause, `d = 2` | 1 | **every** `m ≥ 4` |
| PHP, linear, `d = 2` | 2 | **no scale** (`m ≥ 4`) |
| `Count_3`, linear, `d = 2` | 4 | **exactly `n ≡ 3 (mod 4)`** (`n ≥ 6`) |

Three consequences. (i) `NS-degree(Count_3(n)) ≥ 3` for **every** `n ≡ 3 (mod 4)` — including
`n = 11` (165 variables) and beyond, scales no explicit monomial basis here can even represent.
(ii) For linear-encoded pigeonhole the char-2 gap is the *rule*: general degree-2 witnesses exist
(measured at `m = 4, 5`) but **every one of them is asymmetric, at every scale** — symmetric
reasoning is structurally blind to this bound. (iii) Whether symmetry can see a lower bound depends
on the *encoding* (clause-PHP: yes, at every scale; linear-PHP: never) — a `GF(2)` phenomenon
invisible over ℚ, where Reynolds averaging makes symmetric visibility automatic. The gap map is
locked: `invariant ⟹ general` everywhere (soundness), with gap cells at `Count_3(8)` and
systematically across linear PHP. Artifact: `the_symmetric_primal_dual_gap_over_gf2_is_measured`.

**How deep is the broken symmetry? At least two markings.** The first refinement rung — restrict to
a point/hole/pigeon *stabilizer* (one marked object, its own sort; the collapsed machinery applies
verbatim) — was predicted dead by hand at all seven probed cells and measured dead at all seven:
`Count_3` stays exactly on its mod-4 schedule under a marked point, and all four marked PHP-linear
cells stay empty. The mechanism: marking shifts the counting-polynomial arguments by one, and the
dead constraint rows are dead by an *evenness* one mark cannot split. So the gap witnesses carry
**symmetry-breaking depth ≥ 2** — the certificate-level analog of §4.1's composite-shear depth, the
same invariant one level up. Artifact:
`the_off_schedule_witnesses_are_probed_for_stabilizer_invariance`.

**Trust boundary, stated exactly.** In-window verdicts are fully certified (differential agreement +
lifted, re-checked witnesses). The ∀-scale extrapolation rests on the fitted entry polynomials —
certified with spare verification points for PHP (window `m = 4..8`), exactly determined for
`Count_3` (window `n = 6..8`, plus the located closed forms) — and on the two kernel-laddered
arithmetic lemmas. Full kernel internalization of Pascal's recurrence is the next hardening level.

## 8. The Extended-Frege-class engine, and the boundary with P vs NP

**8.1 The upper-bound side.** The practically-checkable redundancy hierarchy is
`Resolution ⊊ RUP/DRAT ⊊ PR ⊊ SR` — the PR/SR tier being the Extended-Resolution/Extended-Frege
class. This repository operates there, fail-closed: `pr.rs` (the PR + SR checker), `sym_certify.rs`
(steered symmetry proofs: `heule_php_refutation` emits **exactly `m(m−1)/2`** SR steps — the
certificate carries its own clock — for the family Haken proved exponential for resolution),
`sdcl.rs` (satisfaction-driven clause learning: *discovering* PR clauses via the positive reduct, no
symmetry hints), `sym_dynamic.rs` (symmetric explanation learning: a learned clause's whole orbit is
RUP for free). External verification: DRAT/LRAT through `drat-trim`/`lrat-check` (formally
verified); SR through `sr2drat → drat-trim` — pigeonhole verified end-to-end to `PHP(18)`, a
591,346-line expanded DRAT proof (`benchmarks/sat/proofs/`). The probe instrument
(`tests/ef_class_probe.rs`) points the automatic search at the mutilated chessboard and at random
3-CNF above the threshold: measurements, not theorems — a negative result is data too.

The measurements, and they cut the way the thesis predicts. On the **mutilated chessboard** —
resolution-exponential, but with short *hand-built* PR proofs (Heule–Kiesl–Biere) — the automatic
search, fed only opaque clauses, *discovers its own*: 21 PR steps at the 4×4 board (14 ms), 56 at
the 6×6 (56 vars, 768 ms), the composed proof re-checked against the original formula. On **random
3-CNF at ratio 5** the same search mostly finds *nothing* to exploit (discovered PR = 0, falling to
a plain resolution refutation — every one externally `drat-trim`-VERIFIED), with only the occasional
lone PR shortcut. That is the empirical shadow of the two poles: automatic EF-class discovery
succeeds exactly where structure exists and comes up empty on the structureless — the search feels
the same boundary the certificates measure. Artifacts:
`sdcl_discovers_sr_refutations_of_the_mutilated_chessboard_with_measured_scaling`,
`random_threshold_cnf_sr_size_scaling_is_measured_with_external_verification`.

Measured against the field (`benchmarks/sat/run-satbench.sh`, byte-identical DIMACS to every
engine; single-run wall clock, the harness's own stated caveat; log
`logs/optimization/satbench/run-20260630-072737.log`): the *discovery* engine — fed only opaque
clauses — refutes `PHP(14/20/28)` in 6/17/51 ms against SaDiCaL's 28/155/940 ms (its proofs
`dpr-trim`-verified) with Kissat timing out at every size; Tseitin expanders stay flat at 3–19 ms
for `n = 80..160` while SaDiCaL exceeds 12 s throughout and Kissat times out from `n = 120`. The
control rows are kept honest: on random k-XOR Kissat's inprocessing is fine (37 ms — expansion, not
XOR-ness, is what bites), and on Ramsey/pebbling/odd-matching, where no specialist fires, the
decade-tuned CDCL engines match or beat ours on raw search. The separation is the proof-system gap,
not engine tuning.

**8.2 The boundary.** Everything in §§5–7 is a *lower bound* or a formalization — hardness results,
the P ≠ NP direction; none is an algorithm. Our algebraic tools live inside the algebrization
barrier (Aaronson–Wigderson) — and this includes every field of §§5.6–5.10: `GF(p)` and `GF(pᵏ)`
Nullstellensatz are finite-field algebra as squarely as `GF(2)`'s, and the certified prime
incomparability *strengthens* the reading that no single low-degree algebraic proof system
suffices, without moving one step past the barrier. The census-style structural arguments are
exactly what Razborov–Rudich warn about: these techniques cannot resolve P vs NP in either
direction. The
upper-bound direction — automated *search* for short EF-class proofs (§8.1) — is not blocked by any
formal barrier; it is "merely" conjectured hard, and Cook–Reckhow makes its asymptotics equivalent
to NP vs coNP. We treat it as an experimental subject: measure what the search finds, verify every
find externally, report the failures. **This paper is not, and cannot become, a P vs NP result.**

**The gunsight.** The boundary is now itself an executable artifact
(`tests/pvnp_gunsight.rs`): cell by cell, the Cook–Reckhow road is machine-checked — the
weak-system routes to a poly-bounded system are *certifiably closed* on pigeonhole (NS degree
`4, 6` growing, characteristic-invariant, ring-lifted to `ℤ/6`; resolution width `2, 3` growing,
the closed-set certificate re-checked), while the SAME family is certified CHEAP at the EF-class
frontier — SR proofs of exactly `m(m−1)/2` steps, `[3, 6, 10, 15, 21, 28]` at `m = 3..8`,
re-verified with zero trust. So every family this portfolio certifies as weak-system-hard is
machine-checked *dead* as a frontier witness, and the open cell is named exactly: **a family with
certified superpolynomial EF-class proof size** — the cell whose filling, for every system, is
NP ≠ coNP and hence P ≠ NP, and whose *impossibility* of filling would require a poly-bounded
system at least as strong as the EF class, every weaker candidate being certifiably excluded here.
The gunsight is *lined up*: a survivor ledger runs every candidate through the full arsenal —
PHP/Tseitin/mod-`p` are DEAD (certified dissolutions), and **threshold 3-CNF survives** (pinned
UNSAT samples route to raw CDCL with no specialist firing anywhere in the 30-route dispatcher,
ladder `BeyondBudget`) — while each sampled survivor instance is simultaneously **certified not
random** (its `ℤ/6` structure certificate built and re-checked): what survives is expensive
structure, never absent structure. And the NP-complete *class* layer is separated into its three
statements, each an artifact: problems exist (Cook–Levin, with an executable 3-COLORING → SAT
reduction verified faithful against brute force), instances are never random (the poles transport
through reductions — non-3-colorable `K₄`'s CNF is UNSAT-certified *and* structure-certified), and
class-cost-hardness is the open cell. And the "every hard family eventually met its trick" pattern is itself certified as the **trick
matrix**: row by row, every family in the corpus has a certified trick — PHP's symmetry/SR system
(*discovered unaided* by the SDCL engine — the automatic trick-finder), Tseitin's `GF(2)` route,
`Count_3`'s `GF(3)` route, parity's `GF(2)` route, and each surviving 3-CNF instance's own found
RUP proof (expensive to find, polynomial to re-check — the shape of NP itself); column by column,
every system in the portfolio has a certified hard family, with the ring `ℤ/6` column hard for
*both* primes' killers at once (the conjunction). The matrix pins the exact logical content of
"everything has a trick": **∀family ∃system (certified TRUE) versus ∃system ∀family — and the
swap of those quantifiers IS Cook–Reckhow's poly-bounded system, i.e. NP = coNP: the open cell.**
Artifacts:
`the_gunsight_closes_the_weak_routes_and_names_the_frontier_cell`,
`the_survivor_ledger_lines_up_threshold_3cnf_and_certifies_it_is_not_random`,
`np_complete_instances_inherit_the_poles_through_certified_reductions`,
`every_family_has_a_trick_and_every_portfolio_system_has_a_hard_family`.

**8.3 The mirror: reflection, executable.** The trick-to-find-the-tricks is itself a formula.
`REF(F, s)` encodes "there is a resolution refutation of `F` with `≤ s` steps" as SAT (selector
variables pick each line's parents and pivot; content variables carry each derived clause; padding
by re-derivation makes the budget monotone) — the reflection principle, the proof system looking at
itself in the instance language. Aiming our own solver at the mirror snatches both directions with
certificates. *SAT side*: the model decodes to an actual refutation of `F`, re-verified by an
independent checker that recomputes every resolvent — the trick found by the trick-finder, through
the mirror. *UNSAT side*: the solver's RUP certificate for `REF(F, s)` is a **machine-certified
proof-SIZE lower bound** — "no `s`-step refutation exists," re-checked with zero trust. Size is
the currency of the EF-class open cell: the portfolio certified degree and width bounds before;
size bounds arrive only through the mirror (pinned: minimal resolution sizes `1, 2, 5` on the tiny
corpus, each matched against an exhaustive proof-search oracle, each SAT model decoded and
verified, and the `min > 4` bound for the parity triangle carried by a 91-step re-checked RUP
certificate). The encoding itself was *proven* faithful the hard way: the differential caught a
resolvent-semantics bug during development (dropping both pivot polarities from both parents lets
a tautology resolve to `⊥`), exactly the class of error a decode-and-recheck harness exists to
catch. Alongside: one trick-finder (`sdcl_refute`, zero hints) refutes **every family in the
certified corpus** — pigeonhole, Tseitin, mod-3 Tseitin, `Count_3`, parity, the pinned survivor
3-CNF instances — every proof zero-trust re-checked, and the set of uncovered families asserted
**empty**: the corpus-level `∃trick ∀family`, true and vacuously so, with the certified-EF-hard
class provably memberless *within everything we can name*. The honest closure, which the mirror
itself enforces: Atserias–Müller (2019) proved automating resolution is NP-hard — deciding the
`REF` formulas is as hard as SAT, so the trick-finder is a member of the class it hunts, and
promoting corpus-truth to the asymptotic swap *is* NP = coNP. Self-reference cuts both ways: if
P = NP the finder is easy; if the finder is easy, P = NP. And the tower closes on itself: the
mirror formula's own `ℤ/6` structure certificate is built and re-checked — formulas about proofs
are as non-random as the formulas they describe.

The attack is then run *forward as a program*. **The witness compiler**: given a model `α` of a
satisfiable `F`, emit — with zero search, linearly in the encoding — a RUP refutation of
`REF(F, s)` for every budget `s`: the α-invariant ("every proof line contains an α-true literal";
the satisfied parent's non-pivot witness literal survives every resolution) walked up the DAG as
unit-propagating clauses, closing on the empty last line. Each compiled certificate re-checks with
zero trust. **An NP-witness becomes a coNP-witness in polynomial time** — Atserias–Müller's easy
direction, constructive; with the decode direction (`F` unsatisfiable ⟺ `REF(F, min)` satisfiable,
model decoded to a verified minimal proof) this demonstrates their corollary on the corpus: *having
a short proof is itself NP-complete*. Which localizes the swap to a dial inside this repository:
if NP = coNP, every true "`F` has no `s`-step proof" fact has a short certificate — and those
certificates are exactly what the mirror manufactures. **NP = coNP for this system ⟺ the mirror's
own lower-bound-certificate sizes stay polynomially bounded as `F` grows.** The series is measured
and re-runnable (chain family `F_k`, minimal refutation exactly `k`: certificate sizes
`[(2,1), (3,2), (4,15)]`, every point zero-trust-checked); nothing here decides its growth — the
contribution is that the question is now a curve, not a slogan. The mirror front is then opened
as a measured battleground (`the_leveled_hunter_takes_the_mirrors`): the mirrors **carry
symmetry** — the production automorphism finder recovers 9 generators inside `REF(parity, 2)` and
2–3 inside the chain mirrors: the reflected formula's structure survives the encoding into
selector/content variables — while the plain-CDCL baseline on chain mirrors grows
`2 → 15 → 129` across three scales (the Atserias–Müller hardness, visibly igniting), and the
current SDCL hunter does not yet convert the inventoried symmetry into smaller certificates
(21/63/90 steps vs the baseline — measured, re-checked, honest). The gap between "9 generators
present" and "0 generators exploited" is the sharpest open lever on the board: certified
symmetry-breaking (SEL/lex-leader) aimed at the mirrors' own automorphisms. Artifacts
(`tests/reflection_mirror.rs`):
`the_mirror_encoding_matches_brute_force_and_decodes_to_verified_proofs`,
`the_mirror_manufactures_certified_proof_size_lower_bounds`,
`the_mirror_converts_models_into_refutations_and_localizes_the_swap_to_a_curve`,
`one_trick_finder_covers_the_entire_corpus_and_the_uncovered_class_is_empty`,
`the_mirror_formula_itself_is_not_random`.

**The retreat ladder.** Family-hardness for a *fixed* system is real and exhibitable — this paper
exhibits it, with zero-trust certificates. The pattern worth stating as a locked artifact: **in the
entire certified corpus, no family is hard at its top rung** — every certified lower bound carries a
certified dissolution in a stronger or characteristic-shifted system. PHP: hard for resolution width
*and* for `GF(2)` NS (both certified) — dissolved by SR in exactly `m(m−1)/2` steps (certified).
Tseitin: width-hard (certified) — dissolved by `GF(2)` Gaussian (certified). `Count_3`:
`GF(2)`-NS-hard (certified) — dissolved by `GF(3)` Gaussian (certified). Six retreats, twelve
certificates, one test. This is the family-level companion of §2.4's dichotomy: pointwise hardness
is vacuous-or-unprovable (theorem); per-system family hardness always dissolves one rung up
(certified, this corpus — and historically, every named family anyone has exhibited); and whether
the retreat continues forever is exactly the NP vs coNP question, the one open sentence everything
else isolates. Artifact: `every_exhibited_hardness_dissolves_one_rung_up`
(`tests/hardness_retreat.rs`).

**8.4 The NP = coNP attack ledger — the proof's skeleton, every fillable cell filled.** By
Cook–Reckhow a proof of NP = coNP has exactly one shape: *name one system, prove a polynomial
bound for every tautology family*. The ledger (`tests/np_conp_attack_ledger.rs`) is that skeleton
as an executable artifact. The candidate system is named — SR, the strongest practically-checkable
system here, with **no superpolynomial lower bound known** (real candidacy). The per-family
polynomial upper bounds that exist are certified *with their exponents*: PHP's SR size is exactly
`m(m−1)/2`, FITTED (constant second finite difference across `m = 3..10` — the
interpolation-certificate pattern of §7 — with zero-trust re-checks through `m = 7`); Tseitin is
`≤ n` equations over `GF(2)` (linear, re-checked); `Count_3` is one `GF(3)` Gaussian pass
(linear). The open cells are named with their measurements: threshold 3-CNF (sizes measured,
growth class open) and the `REF`-mirrors of hard formulas (resolution-hard by Atserias–Müller; SR
size open — by §8.3's equivalence, *that* cell is the swap). And the crux the ledger sharpens:
**`3-SAT ∈ coNP ⟺ NP = coNP`** (Cook–Levin), and coNP membership splits into three obligations of
which **two are already proven here** — certificates *exist* for every UNSAT instance (the
no-finite-randomness theorem, specialized to 3-SAT: `ℤ/6` structure certificates and RUP
refutations, re-checked per instance) and are *poly-time checkable* (the zero-trust checkers).
Only the *size* obligation remains, and it is not open symmetrically: for the resolution/RUP class
it is CLOSED NEGATIVELY (Chvátal–Szemerédi 1988 — random 3-CNF requires exponential resolution
size, so the system our CDCL emits provably cannot witness `3-SAT ∈ coNP`), while for SR-and-above
it is open with a certified precedent that escalation collapses happen: PHP, exponential for
resolution (Haken), is quadratic in SR (this ledger's own fitted bound). The missing lemma,
exactly: *every unsatisfiable family has polynomial-size SR proofs* — filled, NP = coNP; refuted
for every system, NP ≠ coNP and hence P ≠ NP. Nothing here decides it; everything here is the
executable statement of what deciding it takes, with the evidence honestly two-sided: every
certified upper bound in the ledger is =-direction progress, every certified lower bound in this
paper is ≠-direction evidence. The size bar is then made undeniable by measurement: obligations
1+2 are satisfied by *every decidable language* (a truth-table trace is a checkable certificate
for anything), so the polynomial size bound is not a refinement of coNP membership — it IS the
membership; and the existence-format certificate that proves the pole measurably misses that bar
(sizes `[(8, 553), (10, 1571), (12, 10877)]` on pinned UNSAT threshold samples — tracking its
`2ⁿ` basis), while the live SR experiment on the survivor family runs beside it (certified sizes
`[(12, 12), (16, 13), (20, 7), (24, 9)]`, zero-trust re-checked) — the open cell's instrument:
the first format whose curve is *provably* polynomial for every family is `3-SAT ∈ coNP`, and no
barrier forbids one. The curve is extended through `n = 32` (sizes
`[(12,12), (16,13), (20,7), (24,9), (28,23), (32,23)]`, re-checked through 28) beside the standard
that "provably polynomial" actually means: **polynomial by construction** — a uniform generator
with a closed-form step count, never fitted points (the proof lives in the generator, exactly
where this paper's `∀n` proofs live). Three islands of 3-SAT meet the standard, certified: the
Horn fragment at **constant** size (the single-step `⊥` certificate re-checks at chain length
`10, 100, 1000` alike), the symmetric fragment at **quadratic** (`m(m−1)/2`, the generator's own
clock), the parity-encodable fragment at **linear**. The general threshold family has no known
generator; exhibiting one for all of 3-SAT is `3-SAT ∈ coNP` is NP = coNP. Finally **the
generator hunt**: for families where a generator provably exists *because it was planted* —
parity (Tseitin 3-CNF, linear), pigeonhole-in-a-haystack (PHP(4) plus growing satisfiable noise,
constant 6 steps), Horn under polarity camouflage (single-step) — the zero-hint hunter is
measured through syntactic disguise (variable renaming + clause shuffling). The plants are FOUND,
never drowned: disguised parity still routes structurally at every scale; disguised Horn's
size-1 certificate re-checks at length 500; the haystacked pigeonhole is refuted with re-checked
certificates at every noise level — with the honest tax measured: hunted sizes grow `26 → 144` as
the haystack grows `0 → 60` while the generator holds 6, so "found a certificate" and "isolated
the plant" are different achievements and the gap is now a curve. **And the tax cell is then
CLOSED as a theorem**: PHP(4) is minimally unsatisfiable (every single-clause deletion satisfies —
verified exhaustively) and the noise is satisfiable on disjoint variables, so the composite has
exactly one minimal UNSAT core — the plant — and deletion-based isolation provably lands on it.
Certified at every noise level, through full disguise: the isolated core **is the disguised plant
exactly** (canonical set equality, all 22 clauses), and the certificate on the recovered core
holds constant — sizes `[24, 24, 24, 24]` across noise `0/20/40/60`, each zero-trust re-checked.
The `26 → 144` curve is a property of the hunter, not the problem; found-certificate size is a
property of the core wherever the core can be isolated. The hunt pins the missing lemma
in its sharpest form: **`3-SAT ∈ coNP` ⟺ every UNSAT 3-CNF is secretly planted** — carries
structure some generator explains polynomially. Resolution-visible plants provably do not cover
the threshold family (Chvátal–Szemerédi); SR-visible plants are the open cell. Artifacts:
`the_ledger_certifies_every_polynomial_upper_bound_that_exists_and_names_the_open_cells`,
`three_sat_in_conp_is_the_swap_and_its_existence_half_is_already_proven`,
`the_size_bar_is_the_definition_and_the_sr_curve_is_the_open_cell_experiment`,
`the_sr_curve_extended_and_provably_polynomial_is_by_construction`,
`the_generator_hunt_recovers_planted_structure_without_hints`,
`core_isolation_eliminates_the_haystack_tax_and_recovers_the_plant_exactly`.

**8.5 The Uniform Transfer Theorem: one true family, and certificates as functorial cargo.**
Define a morph of covers `ψ : F → F′` by blocker nesting (`blocker(C) ⊆ blocker(ψ(C))`). Then,
certified in three parts (`tests/uniform_transfer_theorem.rs`): (1) **transfer** — any NS
certificate of `F` pushes forward along any morph, `g′_{C′} = Σ_{ψ(C)=C′} p_C·g_C`, sound by the
single multilinear absorption identity `p_C·p_{ψ(C)} = p_C` and therefore valid over EVERY
coefficient ring — verified for all 43 orbit families at `n = 3`, over `ℤ/2, ℤ/3, ℤ/6`, along
three named morphs each, and along *every one* of a family's morphs in a complete exhaustive
sweep; (2) **universality** — the all-corners cube refines every unsatisfiable cover (a corner's
blocker sits inside any clause falsifying it), so the cube is the SUPER-FAMILY: exactly one true
family per `n`, every family its mutant, any two families within a two-step span of each other
through the cube, and the canonical completeness construction of §2.1 revealed as the cube's
partition-of-unity certificate transported along a charging morph; (3) **functoriality** — morphs
compose and the two-hop pushforward equals the composite one-hop, coefficient-for-coefficient.
The maximal symmetry of the source is the point: the cube carries all of `Bₙ` (every bit-flip,
every neighbor exchange — the "ultimate symmetry" of the truth table), its certificate is
`Bₙ`-invariant, and the kernel certifies it `∀n` — so the entire hypercube at every scale is
covered by ONE kernel-certified symmetric source plus finite functorial transfer. The honest toll,
relocated but not shrunk: riding from the cube costs its `2ⁿ` basis, and `3-SAT ∈ coNP` becomes
*the existence of morph decompositions with polynomial toll* — any fixed exponent would suffice,
none survives the resolution-class exponential, so the cheap chains live at SR strength or above.
The toll is then MEASURED (`the_toll_ledger_measures_the_symmetry_discount_across_every_family`):
across all 43 families at `n = 3`, tolls run 3..27 against the `3ⁿ = 27` ceiling with **42 of 43
discounted — the only full-price family is the cube itself** — and cheap toll co-occurs with low
NS degree (mean 2.43 on the cheapest third vs 2.93 on the priciest): cancellation is structure.
And the terrain the toll must cross is enumerated and LOCKED
(`tests/rigid_residue_census.rs`): the mutant ratios explode favorably — `4 → 4`, `43 → 27`,
`42,263 → 403` base types (×105 compression), the generic full-degree cores collapsing to just
**309 types** with the largest morph-class holding 1,541 orbits — while the `n = 4` rigid residue
is exactly `5,416` `B₄`-symmetric `+ 3,180` shear-visible `+ 33,667` RESIDUE, with **total** cost
coupling: 100.0% of the residue at degree ≥ 3 (32,825 of 33,667 at full degree) — the set with no
symmetry under any registered lens and the set at maximal proof-cost are the same set, measured
to the orbit (sampled `n = 5`: 24/7/9, every residue member full-degree). The Toll Lemma in
terrain language: prove the discount for the 309 base types and let functorial transfer carry it
to their mutants. The full deductive chain from Cook–Reckhow down to this single open lemma is
assembled as `THE_PROOF.md` (alongside this paper in `work/`) — nine proven, artifact-backed lemmas and one
named open cell. Artifacts: `the_cube_certificate_rides_every_morph_to_every_family`,
`morphs_compose_and_transfer_is_functorial`,
`the_toll_ledger_measures_the_symmetry_discount_across_every_family`,
`the_n4_rigid_residue_is_enumerated_and_cost_profiled`, `the_mutant_ratio_and_base_type_census`,
`the_n5_rigidity_landscape_is_sampled_through_every_lens`.

## 9. The two-sided separations atlas, and reproducing

Every row two-sided, every half re-checked by its independent verifier
(`tests/separations_atlas.rs`, `the_certified_separations_atlas_is_two_sided_and_re_checkable`):

| family | lower half (certified impossibility) | upper half (certified proof) |
|---|---|---|
| `PHP(3)` | NS-degree ≥ 4 (dual witness); res-width = 2 (closed set) | SR refutation, 3 steps (PR/SR checker) |
| `PHP(4)` | NS-degree ≥ 6; res-width = 3 | SR refutation, 6 steps |
| Tseitin(6) | res-width = 4 (closed set) | `GF(2)` refutation, 6 equations (`xorsat::is_refutation`) |
| Tseitin(8) | res-width = 4 | `GF(2)` refutation, 8 equations |
| `Count_3(7)` | `GF(2)` NS-degree ≥ 3 (dual witness; ∀`n ≡ 3 (mod 4)` by §7) | `GF(3)` refutation, 7 equations (`modp::is_refutation`) |
| `Count_3(8)` | `GF(2)` NS-degree ≥ 3 | `GF(3)` refutation, 8 equations |

The `Count_3` rows are the characteristic-mismatch marquee: one family, a certified `GF(2)` hardness
floor and a one-Gaussian-pass `GF(3)` refutation — algebra sees the obstruction exactly when its
characteristic divides the count. §5.7 certifies the mirror image (`Count_2`: degree-1 over `GF(2)`,
growing certified degree over `GF(3)`), so the mismatch is a proven *incomparability*, not a
one-way convenience.

Reproduce (the proof-complexity core):

```
cargo nextest run -p logicaffeine-proof \
  -E 'test(polycalc::tests::) | test(polycalc_gfp::tests::) | test(polycalc_zm::tests::) | test(census::tests::) \
      | test(res_width::) | test(orbit_stability::) \
      | test(hypercube::tests::mod_p_one_hot_instances_land_on_the_modcount_rung_of_the_extended_ladder) \
      | binary(separations_atlas) | binary(orbit_stability_kernel) | binary(mpoly_ring_kernel) \
      | binary(gf2_ring_kernel) | binary(gf3_ring_kernel) | binary(gf2_poly_ring_kernel) \
      | binary(ait_kolmogorov) | binary(hardness_witness_ladder) | binary(cost_pole_kernel_seeds) \
      | binary(cost_pole_kernel_ladder) | binary(pvnp_gunsight) | binary(reflection_mirror) \
      | binary(hardness_dichotomy) | binary(hardness_retreat) | binary(no_randomness_at_infinity) \
      | binary(np_conp_attack_ledger) | binary(uniform_transfer_theorem) \
      | binary(ultimate_symmetry_finder) | binary(rigid_residue_census) \
      | binary(konig_compactness_kernel) | binary(martin_lof_omega_kernel) \
      | binary(finite_randomness_kernel_integration) | binary(no_finite_randomness_infinity)'
```

The heavy measurements (`count_three_scale_probe_measures_the_degree_growth`,
`count_two_scale_probe_measures_the_gf3_degree_at_scale`, `tests/ef_class_probe.rs`) are
`#[ignore]`-gated; run them explicitly or via the full suite. The
Calculus-of-Constructions kernel (`crates/logicaffeine_kernel`) is the trust root; solver-produced
certificates are re-checked by independent verifiers with zero trust in the solvers, and the SR/DRAT
chains are additionally checked by the community's external tools.
