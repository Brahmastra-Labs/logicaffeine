# logicaffeine-kernel

A pure Calculus of Constructions type checker (inductive types, fixpoints, universes, quotients) and the only part of Logicaffeine a proof asks you to trust. Everything above it, from the English parser to the Lean importer, only proposes terms, so a bug up there can lose a proof but never forge one; `recheck`, an independent de Bruijn checker, gives a second opinion, and says so when it cannot reach one. Its trusted base is written down and wider than Lean's: 63 named assumptions to Lean 4's three axioms, each argued in [TCB.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_kernel/TCB.md).

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 1. Its only workspace dependency is `logicaffeine_base`; with no path to the lexicon (the Milner invariant) it never sees an English word, and new vocabulary never recompiles the type checker.

## Role in the workspace

The bottom of the proof stack, depended on by `logicaffeine_compile`, `logicaffeine_proof` and the web and CLI apps. It holds three checkers:

- the named `Term` kernel (`infer_type`, `is_subtype`, `normalize`), the trust base;
- `recheck`, a second implementation with its own de Bruijn terms, reduction and conversion, so a substitution bug in one surfaces as a disagreement instead of passing both; `double_check` runs both;
- `dag`, the hash-consed core: Lean 4's theory exactly, behind the Lean Kernel Arena checkers, and LOGOS's own.

Types, values, functions and proofs are all one `Term`: `Nat : Type 0`, `λx:Nat. x` and `refl : a = a` alike. See [proof and verification](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md) for how proposals flow down to this core.

## Example

`refl Nat Zero` proves `Eq Nat Zero Zero`, and both kernels agree:

```rust
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{double_check, infer_type, is_subtype, DoubleCheck, Term};

let ctx = StandardLibrary::context();
let g = |n: &str| Term::Global(n.to_string());
let app = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));

let proof = app(app(g("refl"), g("Nat")), g("Zero"));
let claim = app(app(app(g("Eq"), g("Nat")), g("Zero")), g("Zero"));
assert!(is_subtype(ctx, &infer_type(ctx, &proof).unwrap(), &claim));
assert_eq!(double_check(ctx, &proof), DoubleCheck::Agreed);
```

## Trusted base

The 63 assumptions `StandardLibrary::register` installs are 15 `Primitive` (Lean assumes or builds in the same, `propext`, `Classical.choice` and `Quot.sound` among them), 0 `Definable`, 37 `Reflection` (the semantics of the `Syntax`/`Derivation` embedding) and 11 `DomainSpecific` (temporal arithmetic). `axiom_dependencies` is Lean's `#print axioms` as a function; `ADMITTED_AXIOMS` lists the placeholders, such as `sorryAx` and `admit`, that mark a proof admitted, and `FOREIGN_AXIOMS` pairs each name LOGOS deliberately lacks with the diagnostic to give instead of "unbound variable".

## Public API

### Core types

- **`Term`**: `Sort(Universe)`, `Var`, `Global`, `Const { name, levels }` (a universe-polymorphic constant at explicit levels), `Pi`, `Lambda`, `App`, `Match { discriminant, motive, cases }`, `Fix { name, body }`, `MutualFix`, `Let`, `Lit(Literal)`, `Hole`.
- **`Universe`**: `SProp | Prop | Type(u32)`, plus the level algebra of universe polymorphism: level variables `Var`, `Succ`, `Max` and the impredicative `IMax`. Cumulative: `SProp ≤ Prop ≤ Type(i)`, `Type(i) ≤ Type(j)` iff `i ≤ j`. `Π` is impredicative in `Prop` (its sort is `imax`), so a universally-quantified FOL formula stays a `Prop`; any two proofs of an `SProp` are definitionally equal.
- **`Literal`**: `Int(i64)`, `BigInt` (the integers that overflow `i64`, canonically), `Nat` (an arbitrary-precision natural, definitionally equal to its unary `Succ^n Zero` in both kernels), `Float(f64)`, `Text(String)`, `Duration(i64 ns)`, `Date(i32 days)`, `Moment(i64 ns UTC)`. Computed via ALU, not recursion.
- **`Context`**: typing context. `extend` adds a local binding as an O(1) clone; the global environment (inductives, declarations, axioms, definitions, hints) is `Arc`-shared.
- **`KernelError` / `KernelResult`**: unbound variable, type mismatch, non-function/non-type, bad motive / wrong case count, positivity and termination violations, certification errors, un-inferable hole.

### Type checking

- `infer_type`: bidirectional CIC inference.
- `is_subtype`: cumulative subtyping, as a `bool`.
- `normalize`: beta/iota/delta + guarded-fix reduction with δ-cycle detection and a 2^20-step budget (`normalize_checked` reports why it stopped, as `Unreduced`); evaluates primitive ALU ops (add/sub/mul/div/mod, comparisons, ite) and the `syn_*` reflection builtins (`syn_eval`, `syn_quote`, …).

### Decision procedures

Each is a `pub mod` with a Rust entry point on `Term` (distinct from the prelude-registered `try_*` tactic terms below):

| Module | Proves | Entry point |
|--------|--------|-------------|
| `ring` | polynomial equalities | `reify` → `Polynomial::canonical_eq` |
| `lia` | linear inequalities (Fourier–Motzkin over ℚ) | `fourier_motzkin_unsat` |
| `omega` | exact integer arithmetic (discrete, GCD-normalized) | `omega_unsat` |
| `cc` | congruence closure over uninterpreted functions | `check_goal` |
| `simp` | rewriting / constant folding (fuel-limited) | `check_goal` |
| `bitvector` | reflection-symmetry identities (N-Queens) | `reflection_symmetry_proven` |

`bitvector` exhaustively machine-checks the bit-permutation identities for `n = 1..=PROOF_WIDTH` (16); edge-distance uniformity of the per-bit transport makes that a proof for all `n` (memoised via `reflection_certificate`).

Two algebraic-substrate modules build on `ring`: `field_algebra` proves identities over the prime field 𝔽_q of ML-KEM / ML-DSA, and `word_ring` proves them over the word ring ℤ/2ⁿ (`Word8`/`Word16`/`Word32`/`Word64`). Both are discharged by the kernel's own decision procedures, so the certified-crypto arithmetic never trusts an external algebra system. `eval` is the call-by-value evaluator for the computational fragment (the engine behind `native_decide`), distinct from `normalize`'s substitution-based reduction.

### Elaboration and recursors

`elaborate` is the elaborator (metavariables, unification, implicit arguments), so `id 0` becomes `id Nat 0` before the kernel sees it; `recursor` derives the dependent eliminator `I.rec` of an inductive, as Lean and Coq do; `tactics` proves a statement by tactics over the kernel's own goals (`GoalState`: `intro`, `exact`, `apply`, `refine`, `rfl`, `change`, `rewrite`, `subst`, `injection`, `induction`, `cases`, `calc`, and `cc`, `decide` and `simp`, which build their proof terms), each goal a metavariable with its hypotheses and target. All three propose terms the checker still re-verifies: `GoalState::qed` hands the kernel the finished proof.

### Representation and derived declarations

- `term_store`: hash-consed term storage. A `Term` is a `Box`-recursive tree, so a subterm occurring `n`
  times is `n` trees checked `n` times; interning gives the sharing Lean's `Expr` has, so equal subterms are
  one node and a repeated check is a pointer comparison.
- `int_inductive`: `Int` as a real inductive (`ofNat | negSucc`, Lean's shape) with its arithmetic
  defined rather than assumed. `integer_laws` proves the sixteen public ring and order laws over that
  carrier and installs each under its law's name, so the prelude's integer arithmetic rests on no axiom.
- `quotient`: Lean's four universe-polymorphic quotient primitives (`kernel.quot.type`, `.mk`, `.lift`,
  `.ind`) for kernel importers, installed in canonical form only and certified rather than admitted. The
  prelude's `Quot_rec` is Lean's definition over its own quotient primitives, not an assumption.
- `observational`: observational equality, registered on request beside the standard library
  (`observational::register`). `Obs.{u} A a b : SProp` computes on the type it compares at: pointwise at a
  function type, logical equivalence at `SProp` and `Prop`, by formers at a universe, by constructors at an
  inductive family, by the equivalence closure of the relation at a quotient. `Obs_cast` carries a value along
  an equality of types and computes along universes, function types, parameterized families and quotients,
  so a closed numeral carried by `Obs_cast` is still a numeral. Function and propositional extensionality hold
  by conversion and the standard library's `Quot_sound` is a theorem. The constants are certified as the
  quotient's are (displacing a name they rest on revokes the certificate; the same spelling without it is an
  assumption) and compute in both kernels and in the `dag` core.
- `equations`: equation lemmas. Definitions are δ-transparent, so `dbl 3 ≡ 6` already holds by
  conversion; `f_eq_def` and `f_eq_1 … f_eq_n` give each such fact a name a proof can cite.
- `tactic_cert`: the second kernel's check of reflected ring, linear-arithmetic and congruence goals.
  Linear arithmetic and congruence replay certificates, ring uses exact Kronecker substitution, and none
  calls the first kernel's engines: the rechecker trusts the certificate, not the tactic.

### Soundness gates

- `positivity::check_positivity_resolved`: strict positivity of inductives at every declaration boundary, unfolding δ-aliases and checking nested occurrences through registered containers; rejects negative occurrences that would encode Russell's paradox and declines a container it cannot decide. `check_positivity` is the syntactic, alias-blind variant.
- `termination::check_termination`: guarded structural recursion by a lexicographic measure over the inductive binders (the matched binder first), shared by a mutual block; rejects `fix f. f`. `structural_columns`/`structural_position` give both kernels' `Fix` reduction the column the guard chose.
- `inductive_compile`: the nested-inductive compiler, an untrusted front-end for inductives that recur through other inductives, kernel-rechecked on the way back.

### Standard library (`prelude`)

`StandardLibrary::register` installs `Entity` (FOL domain), `Nat`, `Bool`, `TList`, `True`, `False`, `Not`, `Eq`, `And`, `Or`, `Ex`, `Int` as the inductive `ofNat | negSucc` with its arithmetic and its ring and order laws proven, the primitive `Float`/`Text`, the reflection embedding (`Syntax`, `Derivation`), hardware ops, and the kernel-level tactic terms `try_ring`/`try_lia`/`try_cc`/`try_omega`/`try_simp` plus `try_auto` (sequencing `simp → ring → cc → omega → lia`). `StandardLibrary::context()` is that registration made once per process: borrow it to ask questions, clone it (copy-on-write, about a microsecond) for a context of your own.

### Certificates (`serde` feature): the De Bruijn criterion

`certificate::{Certificate, recheck, PRELUDE_VERSION}` (`PRELUDE_VERSION` is `logos-coc-6`; a certificate minted against another prelude is refused). A `Certificate` carries **only** `proof_term`, `claimed_type`, and `prelude_version`, never a context. `recheck` checks against the trusted axiom context it registers itself (`StandardLibrary::context`), infers the term's type, and requires it to be a subtype of the claim, so a certificate cannot smuggle in a bogus axiom. The trusted surface of a re-check is this crate plus a JSON parser: no proof search, no SMT. The `recheck` example re-checks one from a file:

```bash
cargo run -p logicaffeine-kernel --example recheck --features serde -- cert.json
```

`TheoremCertificate` (`recheck_theorem`) re-checks a proof over a disclosed delta and refuses a delta entry that reuses a prelude-owned name; `EnvironmentCertificate` carries a proof with every declaration it needs as a shared term table, rebuilt through the kernel's checked gateways. `read_certificate` dispatches any certificate by shape and schema and `recheck_document` returns a `RecheckReport`. With `--package <dir>` the `recheck` example replays a Lean → LOGOS package.

### Interface

`interface` is a vernacular text front-end (`TermParser`, `parse_command`/`Command`, `literate_parser`, `Repl`) for driving the kernel by hand: `Definition`/`Theorem`/`Opaque`/`Axiom`/`Check`/`Eval`/`Inductive`/`Structure`/`Quotient`/`Print Axioms` commands with universe parameters, namespaces, sections, notation and hygienic macros, and an English-like literate syntax. Its printer is the exact inverse of its parser. It builds `Term`s for the trusted core; it is not part of the trusted surface.

`interface::lean` (the `lean` feature) reads a Lean 4 kernel export (`lean4export` NDJSON) as one more input surface. It is an untrusted reader: every declaration it builds is re-decided by the kernel and by the independent `recheck`, the target's axioms are compared with Lean's `#print axioms`, and the result is emitted as a literate `.lg` document the vernacular re-parses and re-checks. `largo lean import` and `largo lean replay` drive it.

### The hash-consed core (`dag`)

`dag` is a type theory over a hash-consed expression DAG in two modes, Lean 4's theory exactly and LOGOS's Calculus of Inductive Constructions (`tc::Theory`): de Bruijn indices, one id per structurally distinct term, loose-bound-variable ranges and free-variable flags cached per node, and every cache keyed on ids. It checks `lean4export` streams directly (`dag::export::check_export`), and the `logos_kernel` and `logos_dual` examples (the second requires `tc` and `sem` to agree) run it as Arena checkers. `name`, `level` and `expr` are its interned stores; `env` the admitted declarations; `tc` inference, reduction and definitional equality by substitution, and `sem` the same judgment by evaluation into closures; `check`, `inductive` and `primitive` the admission rules; `json` and `export` the streaming reader.

In LOGOS's theory the same core checks the declarations of a `Context`: `logos` lowers and lifts `Term`s, `logos_env` admits them (`admit`, `replay`), and `logos_tc` holds the rules LOGOS has beside Lean's: indexed `match` with coverage over the constructors the indices allow, `fix` and mutual `fix` under the guard and checked against the type their position expects, the K-like rule, cumulative inductives, the `Int` literal bridge and the elimination restrictions. It admits every declaration of the standard library and refuses each known paradox by the rule the `Term` kernel refuses it by; the `Term` kernel still decides LOGOS's proof surfaces.

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `serde` | off | `Serialize`/`Deserialize` on `Term`/`Literal`/`Universe`, the `certificate` module and the `recheck` example |
| `lean` | off | compiles `interface::lean`, the Lean 4 export reader (implies `serde`) |
| `shadow` | off | every declaration a checked `Context` door decides is decided again by the `dag` core kept in step with the context (`dag::mirror`), and a disagreement panics with both verdicts |
| `nbe` | off | compiles `dag::nbe`, the `dag` core's closure-based (NbE) conversion engine, opt-in while it is under construction |
| `unchecked-access` | off | the `dag` core reads its stores by the ids they hand out without bounds checks; the Lean Kernel Arena entries build with it, every other build and every test keep the checks |

## Dependencies

- **Internal**: `logicaffeine-base`, for the `UnionFind` behind the `cc` e-graph.
- **External**: `rustc-hash` (id-keyed tables) and `smallvec` (application spines); `serde` and `serde_json` only with the `serde`/`lean` features, so the default trusted core has no serialization dependency.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_kernel/CHANGELOG.md)
