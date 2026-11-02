# Proof & verification

Anyone can print a formula; the question is who checked it. LOGOS answers with one rule that runs
through this guide: search can be heuristic, but acceptance is checked. Automation, tactics, English
proof sketches and imported Lean proofs may find a proof any way they like; a proof counts only once
a small kernel has type-checked its proof term. After this guide you can state and prove theorems
in English, drive the tactic language, import a Lean 4 declaration, and say what each verdict rests
on.

The guide moves outward from the smallest trusted base:

1. **The kernel**: a small, pure type-theory core that *checks* proofs, and reads Lean 4 exports as
   one more input.
2. **The proof engine**: searches for proofs and gives Socratic hints. Everything it finds is
   checked by the kernel, so it adds search without adding trust.
3. **Z3 verification**: an optional SMT/model-checking layer for static verification and hardware
   assertions, whose verdicts include Z3 in what you trust.
4. **Translation validation** and **stencil synthesis**: the same solver turned on the compiler
   itself. One proves a program's symbolic encoding agrees with the interpreter; the other checks
   the JIT's stencils ahead of time.

[The trust story](#the-trust-story) closes the guide with what each kind of verdict rests on.

## The kernel

[`logicaffeine_kernel`](../crates/logicaffeine_kernel/README.md) is a Calculus of Constructions type
checker — the trusted base. It is pure: by the **Milner invariant** it has no path to the lexicon
(see [Architecture](architecture.md)). Core surface
([`kernel/src/lib.rs`](../crates/logicaffeine_kernel/src/lib.rs)):

- `Term`, `Universe`, `Literal`, `Context` — the term language and typing context.
- `infer_type`, `is_subtype` — type checking and cumulativity.
- `normalize` — reduction to normal form. `normalize_checked` reports a reduction it cannot
  finish (`Unreduced::{Cyclic, BudgetSpent, TooDeep}`) instead of handing back the partial term.
- `recheck` / `double_check` — a second, separately implemented CIC checker (de Bruijn terms, its
  own reduction and conversion) that re-infers a proof term's type. A term outside its fragment is
  reported as single-checked, never as independently verified.

It ships decision procedures as modules, each a checkable engine:
[`ring`](../crates/logicaffeine_kernel/src/ring.rs) (polynomial/ring equalities),
[`lia`](../crates/logicaffeine_kernel/src/lia.rs) (linear integer arithmetic),
[`omega`](../crates/logicaffeine_kernel/src/omega.rs) (Presburger/integer arithmetic),
[`cc`](../crates/logicaffeine_kernel/src/cc.rs) (congruence closure),
[`bitvector`](../crates/logicaffeine_kernel/src/bitvector.rs),
[`simp`](../crates/logicaffeine_kernel/src/simp.rs) (simplification), and
[`termination`](../crates/logicaffeine_kernel/src/termination.rs).

**Integers are defined, not assumed.** The prelude's `Int` is Lean's inductive (`Int_ofNat |
Int_negSucc`). `add`, `sub`, `mul`, `div`, `mod` and `le` are checked definitions over `Nat`
arithmetic, and the ring and order laws (`add_comm`, `mul_assoc`, `mul_distrib_add`, `sub_elim`,
`le_trans`, `le_total`, …) are kernel-checked proof terms rather than axioms. `div`/`mod` are
Euclidean and total, as Lean's `Int.ediv`/`Int.emod`: `div (-7) 3 = -3`, `mod (-7) 3 = 2`,
`div x 0 = 0`, `mod x 0 = x`.

**What a proof rests on.** Every assumption the standard prelude installs (`propext`,
`Classical_choice`, `Quot_sound`, the quotient primitives, the opaque `Float` operations, the
reflection semantics of the deep embedding, …) is enumerated and classified by `tcb_ledger_lock`
and argued in [`crates/logicaffeine_kernel/TCB.md`](../crates/logicaffeine_kernel/TCB.md).
`axiom_dependencies` computes the transitive closure a declaration rests on (statement and body,
as Lean's `#print axioms`), and `Print Axioms <name>.` reports it from the vernacular.

**Certificates.** [`certificate`](../crates/logicaffeine_kernel/src/certificate.rs) has three kinds
behind one reader (`read_certificate`): `Certificate` (a proof in the bare prelude),
`TheoremCertificate` (a proof from a disclosed environment delta over the prelude) and
`EnvironmentCertificate` (a proof with every declaration it needs, each rebuilt through the
kernel's checked gateways — the form a Lean import is packaged as). The checker rebuilds the
trusted context itself. Unknown fields are refused, a certificate minted against a retired prelude
version is refused, and a proof whose axiom closure reaches an admission (`sorryAx` or anything of
its shape) does not re-check. The standalone checker is the
[`recheck`](../crates/logicaffeine_kernel/examples/recheck.rs) example: `recheck <cert.json>`
exits 0 verified, 1 rejected, 2 checked by the primary kernel only; `recheck --package <dir>`
replays a Lean → LOGOS package.

**The vernacular.** The kernel reads its own literate documents (`Repl::execute_document`):
`Definition`, `Inductive` (with mutual blocks), `Structure`, `Quotient`, `Theorem`, `Axiom`,
`Opaque`, `Check`, `Eval`, `Print Axioms`, and the surface declarations `Namespace`/`Section`/
`Open`/`Export`, `Notation` and `Macro`. `largo check --kernel FILE` and `largo prove --kernel
FILE` run one, stopping at the first block that does not check.

**Proofs by tactics over the kernel's own goals.** A `Theorem`'s proof may be a script, `Theorem
plus_zero : forall (n : Nat), Eq Nat (plus n Zero) n := by intro n; induction n as [ | k ih]; rfl;
change Eq Nat (Succ (plus k Zero)) (Succ k); rw [ih]; rfl.` The script runs over the kernel's own
goals (`tactics::GoalState`): each goal is a metavariable with its hypotheses and target, so a script
proves what the first-order tactic language below cannot state, a dependent type, a recursion's
equation, a goal under a binder. The steps are `intro x y`, `exact t`, `refine t` (each `_` it leaves
open is a goal, the binders around it in scope), `apply f`, `assumption`, `exfalso`, `constructor`,
`left`, `right`, `exists t`, `have h : T`, `rfl`, `change T` (or `show`), `rw [h, <- h']`, `subst h`,
`injection h as [e]`, `calc [Eq A a b := p, Eq A _ c := q]`, `cc`, `decide`, `simp [h, l]`,
`induction x as [a b | c]` and `cases x as [ … ]`, separated by `;`. `induction` and `cases` generalize the indices of an indexed
family and every hypothesis that depends on the eliminated one; `cases` owes no case for a constructor
the indices exclude, so `cases h` closes a goal from `h : Eq Nat (Succ a) Zero`, and an index that is a
term is generalized with an equation the case then solves (substitution, injection, a constructor
clash), so `cases h` on `h : Leq n (Succ m)` inverts the relation. `cc` closes an equation the
hypotheses entail by congruence, `decide` one whose `Decidable` instance computes to true, and
`simp [h, l]` rewrites the goal by the named equations (the `@[simp]` lemmas when it names none) until
none applies, then closes it by `rfl` when its sides agree; each builds its proof term rather than
being trusted, `simp`'s citing each lemma it rewrote by. The finished proof term is checked by the kernel as a
written proof is, and a step that does not apply is reported with its position and the goal it ran on.

### Lean 4 → LOGOS

The kernel reads Lean 4 too. `interface::lean` (the kernel's `lean` feature) takes a `lean4export`
stream as one more input surface beside the vernacular. It is an untrusted reader: every
declaration it builds is re-decided by the kernel and by the independent `recheck`, the target's
axioms are compared with Lean's `#print axioms`, and the statement is compared with an independent
reference translation of the Lean type. The output is LOGOS itself: one literate `<Target>.lg`
document that the vernacular re-parses and re-checks, packaged with a replayable certificate.
[`largo lean import`](cli.md#lean) runs the whole loop from a Lean project; `largo lean replay`
re-verifies a package in a fresh process.

A run is a sequence of named stages, each recorded in its receipt. `lean_accepted` means the
project compiled in isolation under the pinned Lean 4.14.0 and was exported by the pinned
`lean4export`. Then come `translated`, `primary_check` (exact, non-cumulative conversion),
`independent_recheck`, `assumption_policy` (the statement and proof may rest on exactly `propext`,
`Quot.sound` and `Classical.choice`; an admission is refused by its shape, not its spelling),
`emission_reparsed` and `fresh_replay`. An `unsafe` or `partial` Lean declaration is quarantined: its name is
reserved, nothing is installed, and a safe declaration that references it is rejected. The
[package](../crates/logicaffeine_kernel/src/interface/lean/package.rs) is `manifest.json` (a
SHA-256 per file), `<Target>.lg`, `certificate.json` (an `EnvironmentCertificate`),
`correspondence.json` (Lean name ↔ kernel name ↔ LOGOS name) and `source/export.ndjson`; a replay
reads nothing else and never runs Lean. `largo lean import` exits with the outcome: 0 verified,
1 invalid source, 2 unsupported feature, 3 disallowed assumptions, 4 resource limit,
5 infrastructure failure, 64 usage. `largo prove --kernel <Target>.lg` checks the emitted document
like any other kernel-vernacular file.

## The proof engine

[`logicaffeine_proof`](../crates/logicaffeine_proof/README.md) is a backward-chaining prover over an
arena-independent IR (`ProofExpr` / `ProofTerm`). By the **Liskov invariant** it has no dependency on
the language crate, so it is reusable across front-ends. It produces a proof tree (each `leaf` tagged
with the `InferenceRule` used) and **Socratic hints** (`SocraticHint`, `SuggestedTactic`) that guide
a learner toward the next step rather than just reporting failure. A found derivation is never a
proof by itself: on every public theorem entry point it is certified to a kernel term and
type-checked, and only then reported verified. `VerifiedProof::axiom_footprint` reports what a proof
transitively rests on, split into the trusted base and what the theorem introduced.

Beyond backward chaining, the crate ships its own **Z3-free SAT/BMC core**
([`cdcl.rs`](../crates/logicaffeine_proof/src/cdcl.rs)): the `sat` module does CDCL model finding,
equivalence (`prove_equivalence`), and unsat proofs; the `bmc` module does bounded model checking,
k-induction (`prove_invariant`), counterexample search, and vacuity (`check_vacuity`). Because it
needs no external solver, it runs in the browser: it is the engine behind Studio's
[Hardware mode](studio-and-learn.md) in-browser proving (Z3 stays a native-only backend).

A theorem is written in English as a named block of `Given:` premises and a `Prove:` goal:

```logos
## Theorem: Socrates_Mortality_Verified
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
```

Each line is parsed as a sentence (see [Logic mode](logic-mode.md)); the engine then proves the goal
from the premises. The English → `ProofExpr` lowering lives in the language crate
([`proof_convert.rs`](../crates/logicaffeine_language/src/proof_convert.rs)), preserving the Liskov
boundary. Program extraction turns kernel definitions into Rust through
`logicaffeine_compile::extraction`: inductives become enums, fixpoints recursive functions, and
matches `match`.

`Given (h): …` names a premise so a proof can cite it; an unnamed premise is cited by position as
`hp0`, `hp1`, …. Two premises citable by one name, a label inside the `hp<n>` namespace that is not
its own position, and a `Given:` written below the `Prove:` line are refused.

### Arithmetic in English

A `Given:` or `Prove:` line may be a quantified arithmetic sentence
([`parser/math.rs`](../crates/logicaffeine_language/src/parser/math.rs)). `plus`, `minus`, `times`
and `divided by` are `Add`, `Sub`, `Mul` and `Div`; a lowercase-leading name is a variable, an
uppercase- or digit-leading name a constant, a number word a numeral in any case, and `f(n)` an
applied function term. It lowers to the same `ProofExpr` as the formal `Eq(Add(n, 0), n)`, so the
ring oracle decides it and the kernel checks the same certificate:

```logos
## Theorem: Add_Zero
Prove: For all n, n plus 0 equals n.
Proof: Auto.

## Theorem: Distributivity
Prove: For all a b c, a times (b plus c) equals a times b plus a times c.
Proof: Auto.
```

`divided by` is outside the ring the oracle decides, so `For all n, n divided by 1 equals n.` is
stated correctly and proved by no surface. `For some n, …` builds `∃n(…)`, which the proof layer
does not certify.

### `Proof:` lines

A `## Theorem` without a `Proof:` line, or with `Proof: Auto.`, is proved by search. Any other
`Proof:` line is a script in the tactic language below, read as an English proof **sketch**: the
script runs first, and every goal it leaves open is then discharged by the same certified
automation `Auto` uses. The count is reported: `compile_theorem` prints
`Proved! [kernel-verified] (1 residual obligation discharged by automation)` and
`compile_theorem_for_ui` returns it as `discharged`. A step that fails is never swept: a misspelt
tactic, a missing hypothesis or an inapplicable rule fails the theorem. The three special shapes:

- A one-word line (`Proof: Assumption.`) is a single step. A kernel inference-rule name given as a
  step is refused with the spelling that applies it.
- `Proof: By <Rule>.` applies the named `InferenceRule` to the facts in scope in one step (the
  `by_rule` tactic), certified before it reports success. A tactic word wins over a rule name, so
  `By contradiction.` is `by_contra` and `By reflexivity.` is `rfl`.
- `Proof: By induction on n.` is `induction n`.

```logos
## Theorem: Sketch
Given: Bob is happy.
Prove: If Bob is sad then Bob is happy.
Proof: Assume h.

## Theorem: Or_Commutes
Given (h): Bob is happy or Bob is sad.
Prove: Bob is sad or Bob is happy.
Proof: By cases on h. Right, by assumption. Left, by assumption.

## Theorem: By_Modus_Ponens
Given: If Bob is happy then Bob is sad.
Given: Bob is happy.
Prove: Bob is sad.
Proof: By ModusPonens.
```

`Sketch` names only its first step; the automation closes the goal `Assume h` leaves and the answer
says so. A `Proof:` script cannot see the document's `## Define` blocks and is refused beside them;
`Proof: Auto.` unfolds them.

### The tactic language

[`tactic_script.rs`](../crates/logicaffeine_proof/src/tactic_script.rs) compiles a script into
[`tactic`](../crates/logicaffeine_proof/src/tactic.rs) combinators over a `ProofState`; `qed` then
certifies the assembled derivation through the kernel, so no tactic is trusted.

```text
script  := chain (sep chain)*        sep: ';'  '.'  ','  'then'  (a run of them is one)
chain   := atom ('<;>' atom)*        t1 <;> t2 runs t2 on every goal t1 produced
atom    := '(' script ')' | 'first' '[' script ('|' script)* ']' | 'try' atom | 'repeat' atom
         | 'all_goals' atom | 'any_goals' atom | 'focus' atom | STEP ARG*
term    := names, '+' '-' '*', application by juxtaposition, tuples '(a, b)', holes '?_'
```

| Step | Also spelled | Does |
|------|--------------|------|
| `intro a b` | `assume`, `suppose`, `let`, `introduce`, `given`, `fix` | introduce binders / antecedents, one per name |
| `exact <term>` | `from`, `because` | close the goal with a proof term (`exact (hpq hp)`, `exact (hp, hq)`); a term with a hole is refused |
| `refine <term>` | `partially` | elaborate a term, one subgoal per `?_` hole; a bare hole is refused |
| `have h from <term>` | `note`, `observe`; connective `by`/`using`/`via`/`proved` | add a proved hypothesis to scope |
| `apply h` | `via` | reduce goal `G` to `P` by modus ponens with the named `h : P → G` |
| `cases h with a b` | `destruct`, `consider`, `casework` | eliminate ∧/∨/∃, naming what it binds |
| `rewrite h` | `rw`, `substitute`, `subst` | rewrite by equation `h`; `at k` rewrites only the goal's occurrence `k` (0-based), `at hp` / `at hp k` rewrites hypothesis `hp` |
| `exists <term>` | `use`, `witness`, `choose`, `provide` | supply an ∃ witness |
| `split` | `constructor`, `both`, `conjunction` | ∧-introduction, two subgoals |
| `left`, `right` | | ∨-introduction |
| `assumption` | `done`, `trivial`, `established`, `hypothesis` | close from a hypothesis |
| `rfl` | `reflexivity` | close `a = a` |
| `simp`, `simp only [h…]` | `simplify`, `normalize` | rewrite with the whole scope, or exactly the named rules |
| `calc h1 h2 …` | | compose a named `=`/`≤` chain; the relation may change along it |
| `by_rule R` | `byrule` | apply inference rule `R` in one certified step |
| `by_contra h` | `contra`, `contradiction` | assume the negated goal as `h`, leave `⊥` |
| `induction n` | `induct` | induction on the named variable, generalizing an inner one |
| `decide` | `compute`, `evaluate`, `calculate` | evaluate a closed decidable goal (arbitrary-precision arithmetic, Euclidean `div`/`mod`) |
| `ring` | `polynomial` | close a commutative-ring identity over `add`/`sub`/`mul` |
| `omega` | `arithmetic`, `integers` | linear arithmetic over ℤ, tightening `a < b` to `a + 1 ≤ b` |
| `linarith` | `rationals` | linear arithmetic over ℚ (Fourier–Motzkin, Farkas certificate); declines what needs integrality |
| `crush` | `grind`, `blast` | E-match `∀`-equalities in scope, then congruence closure |
| `auto` | `tauto`, `automatically`, `automation`, `trivially`, `directly` | backward-chaining search |

`focus` (`next`, `bullet`) must close the goal it focuses; `all_goals` (`everywhere`) runs on every
open goal, `any_goals` (`somewhere`) on at least one. Step names are case-insensitive. The
connective words `by`, `the`, `on`, `that`, `now`, `with`, `we`, `it`, `a`, `an`, `of`, `to` and
`and` are skipped before a step, and before an argument unless they are its last word, so
`By cases on h.` is `cases h` and `cases it` still names `it`. `linarith` and `omega` are distinct
tactics: an integer goal that needs the `+ 1` tightening (`x < y, y < x + 1 ⊢ ⊥`) is `omega`'s,
and `linarith` declines it.

### Formal developments

A `## Theory` block states axioms and theorems in the formal notation; a theorem `cites` the
earlier theorems it uses, and the development is proved in citation order:

```logos
## Theory Tarski

Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem symmetry cites reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
```

### `largo prove`

[`largo prove`](cli.md#prove) proves every `## Theory` development and English `## Theorem` in the
assembled module closure of a file or of the project entry; every ✓ is kernel-certified. `--trace` renders
each derivation; `--json` adds an `inventory` (each theorem `proved`/`unproved`, each axiom an
assumption, never a proved fact) and a `summary`, and a name that is both a theorem and an axiom is
refused. `--certificate` emits each English theorem's `TheoremCertificate` with the verdict of
re-checking it against a rebuilt prelude (`rechecks`, `two_kernel_agreement`); a `## Theory`
theorem is reported with `certificate: null` and the reason, since that path exposes no kernel
term. `--kernel FILE` checks a kernel-vernacular document instead.

## Z3 verification (optional, the full build)

[`logicaffeine_verify`](../crates/logicaffeine_verify/README.md) is an SMT-backed verification and
model-checking engine. By the **Tarski invariant** its IR (`VerifyExpr` / `VerifyType`,
[`verify/src/ir.rs`](../crates/logicaffeine_verify/src/ir.rs)) is decoupled from the main AST, so it
is front-end-agnostic. The crate is **excluded from `default-members`**: a plain build needs no Z3.
It is reached via the `verification` feature, in the full build; LOGOS is open source under the MIT
License, so nothing gates it.

It is far more than an assertion checker. Public modules
([`verify/src/lib.rs`](../crates/logicaffeine_verify/src/lib.rs)) include:

- `equivalence` (`check_equivalence`), `consistency` — tautology/contradiction/equivalence checking.
- `ic3` (IC3/PDR), `kinduction` (k-induction), `interpolation` (Craig interpolation), `liveness`,
  `multiclock`, `parameterized` — a full hardware model-checking stack.
- `solver` (`Verifier`, `VerificationSession`), `smtlib`, `incremental`, `abstraction`, `automata`,
  `strategy`, `synthesis`, `security`.

This backs the **English → SVA/PSL** path (the SVA code generator,
[`compile/src/codegen_sva/`](../crates/logicaffeine_compile/src/codegen_sva/)) for formal hardware
verification. Verification errors are written as Socratic explanations with counter-examples. Run it
with `largo verify` / `largo build --verify` (see [the CLI guide](cli.md)).

Z3 is not a strictly stronger prover: the proof engine certifies by structural induction a
`∀n:Nat` goal the Z3 oracle declines.

## Translation validation

[`logicaffeine_tv`](../crates/logicaffeine_tv/README.md) is translation validation for the
compiler, built on `logicaffeine_verify`. `summarize_logos` symbolically executes a LOGOS program
into the verifier's bitvector/boolean domain. `check_encoder_sound` proves with Z3 that this
encoding agrees with the tree-walking interpreter on the full observable behaviour (the ordered
`Show` outputs and whether an error was raised); a nondeterminate program is checked per seed
against the seeded tree-walker. The fragment is straight-line `Int`/`Bool` code plus a determinate
concurrency subset, `Int` encoded as a wrapping 64-bit bitvector; anything outside it is reported
`Unsupported`, never as agreement. Only the source side has a symbolic executor: the
source-to-emitted-Rust equivalence is not yet closed in code. It depends on both `compile` and
`verify` and, like them, is opt-in.

The tensor compiler's fusion rules are proven the same way. `tensor_graph::smt` states what a graph
of the one tensor IR computes as terms over arrays: an op that moves values is the index map it is,
every other op a function left open, an in-place write a store into a range of rows. A rewrite's
obligation says the rewritten graph computes every tensor both graphs keep as the original does, for
every input and every meaning of the open ops; `logicaffeine_tv::tensor::discharge` asks Z3, and
first checks that the facts it assumes (indices in range, intervals the program proves) can hold, so
no proof is vacuous. `identity` computes products, sums and scales exactly, for the algebraic
identities a chip computes a fused op by, at bounded shapes.

## Synthesis (EXODIA)

[`logicaffeine_synth`](../crates/logicaffeine_synth/README.md) is offline tooling that uses Z3 to
verify the EXODIA JIT's stencil specifications against witness inputs (`all_specs`,
`check_spec_with_witnesses`). It also proves that each reversible VM opcode inverts with its
declared inverse and that `RevSwap` is an involution (`all_rev_specs`, `prove_bijective`,
`prove_swap_involution`). It never runs inside the language at runtime; it validates the JIT
back-end ahead of time.

## The trust story

The kernel is the small trusted base. Proof search, tactics and scripts are untrusted: what they
find is certified to a kernel term and type-checked, and a Z3 oracle leaf carries no proof term, so
the certifier refuses it. A certificate re-checks with the kernel alone, and the second kernel
corroborates where its fragment reaches. Z3 verification and translation validation are verdicts
whose trust boundary includes Z3. Claims from outside the prover follow the same replay pattern:
`interval_replay` re-derives analog range certificates by outward-rounded interval arithmetic,
`relax_replay` and `thermal_replay` re-multiply settled linear-solve and thermal-map claims in
exact rationals, and `revgate` certifies a reversible gate as a bijection. Each is the rule this
guide opened with: search can be heuristic, but acceptance is checked.

## See also

- [Logic mode](logic-mode.md) — where the propositions come from
- [Architecture](architecture.md) — the Milner / Liskov / Tarski invariants

## Where this is tested

Every `logos` block above is compiled by `doc_examples`.

- The kernel: `phase69_kernel_coc`, `phase73_certifier`, `phase86_kernel_primitives`,
  `phase99_solver` (computational reflection), `phase103_independent_checker`, and the
  incompleteness suites `phase93_diagonal_lemma` through `phase95_incompleteness`.
- Its defined integers: `derived_integer_ring_laws`, `derived_integer_order`,
  `derived_integer_division`. Its trusted base: `tcb_ledger_lock`.
- The proof engine: `phase60_proof_engine`, `phase61_induction`, `phase96_tactics`,
  `phase97_deep_induction`; `phase102_unified_verification` requires every public theorem entry
  point to certify before it reports verified.
- English theorems end to end: `phase78_e2e_verification`. Program extraction:
  `phase84_extraction`. Arithmetic sentences and `Proof:` lines: `L_language_extra`.
- The tactic language: `tactic_script`, `T_tactics_extra`, `omega_tactic`.
- Z3 verification, built with `--features verification`: `phase_verification`,
  `phase_verification_refinement`, `phase_verification_floordiv`, and the hardware suites
  `phase_hw_z3_equiv`, `phase_hw_e2e_z3`, `phase_hw_advanced_z3`, `phase_hw_cegar_z3`,
  `phase_hw_synthesis_z3` and `phase_hw_native_vs_z3`. `phase104_induction_vs_z3` holds the
  contrast between structural induction and the Z3 oracle.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
