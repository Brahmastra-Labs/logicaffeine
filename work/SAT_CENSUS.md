# The small-`n` SAT-space census

An exhaustive walk over *every* minimal-unsatisfiable formula at small variable counts, classified up to
symmetry — built to find the structure our SAT solver does not yet exploit, and to turn each finding into
a concrete "break this symmetry → solve more cases" lever.

Run it:

```
cargo run -p logicaffeine-proof --example sat_census -- <max_n>
```

Source: `crates/logicaffeine_proof/src/census.rs` (driver + per-orbit record),
`crates/logicaffeine_proof/src/hypercube.rs` (the enumerator, the `Bₙ` group, canonicalization),
`crates/logicaffeine_proof/tests/sat_census.rs` (the spec).

## What is enumerated

A CNF over `n` variables is, geometrically, a set of **subcube blockers** (one per clause: the corners
that falsify it). It is UNSAT exactly when those blockers **cover** the whole hypercube `{0,1}ⁿ`. At the
truth-table level "UNSAT" is a single trivial object, so the meaningful atoms are the **minimal UNSAT
formulas** — minimal subcube covers, the MUSes. Every UNSAT formula is one of these plus redundant junk.
The census enumerates **all of them**, one representative per symmetry class.

The symmetry group is the **hyperoctahedral group** `Bₙ = (ℤ/2)ⁿ ⋊ Sₙ` (the signed permutations, order
`2ⁿ·n!`) — the automorphism group of the cube and the *complete clause-level symmetry*: permuting and
negating variables. Two formulas in the same `Bₙ` orbit are the same problem. The census reports orbits.

Enumeration is by **orderly generation**: branch on the lexicographically-least uncovered corner (which
lies in exactly `2ⁿ` subcubes), prune any branch where a chosen blocker becomes redundant, and visit each
`Bₙ`-class of partial covers once via a canonical key. This collapses the otherwise-intractable raw search
(the branch-order and up-to-`|Bₙ|` symmetric duplication) to one path per class.

## What each orbit is tagged with

| column | meaning |
|---|---|
| `cls` | number of clauses (blockers) |
| `orb` | size of the `Bₙ` orbit (how many distinct formulas it represents) |
| `stab` | `|Bₙ|/orb` — the formula's symmetry group order (orbit–stabilizer) |
| `resW` | minimum resolution width to refute it |
| `rule:f/d` | rule-orbits under the **full** stabilizer `f` vs. what the production breaker **discovers** `d` |
| `rung` | weakest *certified* proof-complexity rung: `Trivial ≺ Counting ≺ Parity ≺ Nullstellensatz{deg}` (GF(2)) |
| `shadow` | the single certified shadow the diagnoser reads off (`Counting`/`Parity`/`CuttingPlanes`) |
| `route` | which engine the full structured router (`solve_structured`) actually decides it with |
| `audit` | `ROUTER>LADDER` (router crushes it polynomially but the certified ladder cannot) and/or `SYM-UNBROKEN` (the production breaker finds less symmetry than exists) |

The two audit flags are the point. They locate, exhaustively at small `n`, two gaps between *what the
solver can do* and *what it currently proves cheaply / breaks*:

- **`SYM-UNBROKEN`** (`d > f`): the production symmetry breaker (`symmetry_detect::find_generators`, used
  in the certified `prove_unsat` cascade) discovers fewer automorphisms than the formula's full `Bₙ`
  stabilizer admits, so the symmetry-breaking predicate it injects is weaker than achievable. A scaled-up
  member of that family then costs the solver search it could have avoided. **This is the direct
  "break more symmetry → solve more" target.**
- **`ROUTER>LADDER`**: the full router decides the cover with a polynomial specialist (covering-collapse,
  XOR, mod-`p`) that the *certified* proof-complexity ladder has no rung for — so the cascade can only
  reach the same verdict through an expensive general proof. The unification gap.

## Findings

### n = 3 — 44 minimal-UNSAT orbits (enumerated in ~75 ms)

The germ of every hardness family already appears here, each with its exact symmetry group:

- **The pigeonhole germ** — `3 clauses, stab 4, Counting shadow`: the smallest counting contradiction.
- **The parity germ** — `6 clauses, stab 12, Parity rung+shadow, affine-explained`: the smallest XOR
  contradiction; its symmetry is **affine, not a clause permutation**, which is exactly why the
  clause-level breaker cannot see it and it needs the GF(2) route.
- **The fully-symmetric cover** — `8 clauses, stab 48`: invariant under the entire group.
- **11 `ROUTER>LADDER` orbits**: the router crushes them via covering-collapse / XOR while the certified
  ladder sees only a degree-3 GF(2) Nullstellensatz.
- No `Counting`-rung and no mod-`p` germ at n=3: the Count_p contradictions need more variables (they
  first appear at larger `n`), confirming the small-`n` census is the right place to catalogue *where*
  each germ is born.

### n = 4 — 42,263 minimal-UNSAT orbits

87% of families are **rigid** (trivial stabilizer — no symmetry to break), confirming clause-symmetry
breaking is exhaustively complete (0 of 42,263 underbroken) and that symmetry covers only the structured
minority. The specialist cuts recognize ~1.8% directly; the rest carry a low-degree algebraic certificate
(NS degree 2–3) the Nullstellensatz/Polynomial-Calculus cuts certify.

### The hardness spectrum (`coverage_summary`)

The durable map of *how much we cover and where the hard core is* — every family placed on the certified
proof-complexity ladder by its weakest crushing rung:

| n | orbits | trivial | parity | Nullstellensatz (by degree) | structured / rigid | max NS-degree |
|---|--------|---------|--------|------------------------------|--------------------|---------------|
| 1 | 1 | 1 | — | — | 1 / 0 | 0 |
| 2 | 4 | 3 | — | d2: 1 | 4 / 0 | 2 |
| 3 | 43 | 12 | 1 | d2: 2, d3: 28 | 37 / 6 | 3 |

The **max NS-degree climbs with n** (1 → 2 → 3) — that *is* the algebraic-hardness wall rising. Low degree
= our territory (covered cheaply); degree growing toward Θ(n) = the rigid core (P-vs-NP floor).

### The scaling bridge — the other face of the wall

A structured family stays **O(1) symmetry-collapsible at every scale**: pigeonhole collapses to **exactly 2
rule-orbits for all n** (verified n=2..20 via the clause-level quotient) — which is why we solve php30 in
milliseconds while Kissat and CaDiCaL time out. The rigid residue has no such collapse; its NS degree
climbs instead. That is the measured line between *the families we cover forever* and *the genuinely-hard
core*.

## How this feeds the solver

The census produced one decisive negative result and one decisive positive lever.

**Negative (and valuable): clause-level symmetry breaking is already complete.** Across all 42,263
minimal-UNSAT formulas at n=4, the production breaker recovers the *full* `Bₙ` stabilizer on every single
one (`SYM-UNBROKEN` = 0). 87% of orbits have no symmetry at all (`stab = 1` — the rigid, random-like
residue). So there is *no* win available from improving permutation/negation symmetry breaking; it is
exhaustively verified optimal. The place to look is the symmetry the clause-level breaker **structurally
cannot represent**.

**Positive: the certified cascade was blind to modular structure — now wired.** The `ROUTER>LADDER`
orbits (1066 at n=4 = 754 covering-collapse + 324 XOR) are crushed polynomially by `solve_structured`'s
specialists but were reached by the certified `prove_unsat` cascade only through expensive search. The
sharpest case is **mod-`p`** (`p ≥ 3`): a mod-`p` Tseitin/counting obstruction is a linear system over
`ℤ/p` whose CNF encoding is resolution-hard — the GF(2) parity cut is blind to odd characteristic and CDCL
blows up exponentially (Z3/Kissat time out), while Gaussian elimination over the right modulus is
polynomial.

**Shipped:** a modular fast-path in `sat::prove_unsat` (`refutes_modular`), placed right after the GF(2)
parity cut, that recovers the one-hot congruence system and refutes it over `GF(p)` (prime) or `ℤ/m`
(composite, by CRT) — fail-closed and re-checked, reusing `modp::recover_from_cnf` / `modp` / `modm`.
Measured on mod-3 Tseitin expanders (the cascade vs. the new cut):

| vertices | vars | `prove_unsat` before | `prove_unsat` after |
|---|---|---|---|
| 10 | 45 | 71 ms | **1.0 ms** |
| 14 | 63 | 670 ms | **1.4 ms** |
| 18 | 81 | 4,370 ms | **1.7 ms** |
| 24 | 108 | minutes (out of reach) | **~ms** |

**Also shipped: the covering/cardinality/parity collapse cut** (`refutes_by_collapse` →
`lyapunov::auto_collapse`) — closes 756 n=4 families every narrow cut missed — and the **universal
algebraic cut** (`refutes_by_nullstellensatz`): a size-gated bounded-degree Nullstellensatz over GF(2)
that asks the shape-free question (is `1` in the low-degree span of the clause polynomials?) and certifies
the low-degree-algebraic residue no structural recognizer matches. Parity is its degree-1 fragment; mod-p
is its GF(p) analogue.

### The certified cascade now (`sat::prove_unsat`)

```
pigeonhole/Hall → cutting-planes → parity(GF2) → mod-p/mod-m(GF_p) → covering/cardinality-collapse
  → bounded-degree Nullstellensatz(GF2) → symmetry-break + CDCL
```

Every cut is fail-closed and certified; each generalizes the one before. The whole stack is **one
algebraic ladder** with degree as the hardness dial:

```
parity (deg-1 NS)  ⊂  Nullstellensatz  ⊂  Polynomial Calculus  ⊂  Sum-of-Squares / Lasserre
```

Remaining levers the census points at:

1. **Climb the ladder**: Polynomial Calculus (dynamic NS, strictly stronger), then SOS — the principled
   crusher for *mixed* families (par32 = parity ⊕ counting) that no single low-degree system cracks.
2. **Decomposition** (`autocarve`): split a formula into structure-pure components and crush each — the
   mechanism for mixed/decomposable instances, not yet wired into the cascade.
3. **Treat the census as a structure library**: every orbit with `stab > 1` is a reusable pattern to
   recognize (in its scaled-up form) inside arbitrary instances and break on sight.
