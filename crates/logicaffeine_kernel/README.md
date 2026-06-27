# logicaffeine-kernel

A pure Calculus of Constructions type checker (CIC-flavoured: inductive types, fixpoints, pattern matching) plus a set of decision procedures — the small, trusted logical base everything else in the workspace must re-check against.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 1 — depends only on logicaffeine_base. **Milner invariant**: the kernel has no path to the lexicon, so it never sees English words. Adding vocabulary never recompiles the type checker.

## Role in the workspace

The bottom of the proof stack. It is depended on by `logicaffeine_compile`, `logicaffeine_proof`, and the web/CLI apps; everything above it (parser, proof search, SMT oracles) is **untrusted** — it only proposes proof terms the kernel re-checks. See [proof and verification](../../new_docs/proof-and-verification.md) for how proposals flow down to this trusted core.

The core insight is that terms, types, and proofs share one syntactic category. Everything is a `Term`: types (`Nat : Type 0`), values (`zero : Nat`), functions (`λx:Nat. x`), and proofs (`refl : a = a`).

## Public API

```rust
use logicaffeine_kernel::{Context, Term, infer_type, is_subtype, normalize};

let ctx = Context::new();
let ty  = infer_type(&ctx, &term)?;        // bidirectional CIC inference
let sub = is_subtype(&ctx, &a, &b);        // cumulative subtyping (bool)
let nf  = normalize(&ctx, &term);          // beta/iota/delta/guarded-fix
```

### Core types

- **`Term`** — `Sort(Universe)`, `Var`, `Global`, `Pi`, `Lambda`, `App`, `Match { discriminant, motive, cases }`, `Fix { name, body }`, `Lit(Literal)`, `Hole`.
- **`Universe`** — `Prop | Type(u32)`. Cumulative: `Prop ≤ Type(i)`, `Type(i) ≤ Type(j)` iff `i ≤ j`. `Π` is impredicative in `Prop`, so a universally-quantified FOL formula stays a `Prop`.
- **`Literal`** — `Int(i64)`, `Float(f64)`, `Text(String)`, `Duration(i64 ns)`, `Date(i32 days)`, `Moment(i64 ns UTC)`. Opaque; computed via ALU, not recursion.
- **`Context`** — typing context. Local bindings grow per binder (`extend` is an O(1) clone behind `Arc`); the global env (inductives, constructors + order, declarations/axioms, transparent definitions, auto-tactic hints) is `Arc`-shared.
- **`KernelError` / `KernelResult`** — unbound variable, type mismatch, non-function/non-type, bad motive / wrong case count, positivity and termination violations, certification errors, un-inferable hole.

### Type checking

- `infer_type` — bidirectional CIC inference.
- `is_subtype` — cumulative subtyping (returns `bool`).
- `normalize` — fuel-limited (default 10000) beta/iota/delta + guarded-fix reduction; evaluates primitive ALU ops (add/sub/mul/div/mod, comparisons, ite) and the reflection builtins (`syn_size`, `syn_max_var`, `syn_lift`, `syn_subst`, `syn_beta`, `syn_step`, `syn_eval`, `syn_quote`, `syn_diag`).

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

### Soundness gates

- `positivity::check_positivity` — strict positivity of inductives; rejects negative occurrences that would encode Russell's paradox.
- `termination::check_termination` — Coq-style syntactic guard for fixpoints (structural recursion); rejects `fix f. f`.

### Standard library (`prelude`)

```rust
use logicaffeine_kernel::prelude::StandardLibrary;
let mut ctx = Context::new();
StandardLibrary::register(&mut ctx);
```

Installs `Entity` (FOL domain), `Nat`, `Bool`, `TList`, `True`, `False`, `Not`, `Eq`, `And`, `Or`, `Ex`, the primitive `Int`/`Float`/`Text`, the commutative-ring axioms for the opaque `Int` (the entire trusted arithmetic base), the reflection embedding (`Syntax`, `Derivation`), hardware ops, and the kernel-level tactic terms `try_ring`/`try_lia`/`try_cc`/`try_omega`/`try_simp` plus `try_auto` (sequencing `simp → ring → cc → omega → lia`).

### Certificates (`serde` feature) — the De Bruijn criterion

`certificate::{Certificate, recheck, PRELUDE_VERSION}`, gated on `serde`. A `Certificate` carries **only** `proof_term`, `claimed_type`, and `prelude_version` — never a context. `recheck` rebuilds the trusted axiom context itself via `StandardLibrary::register`, infers the term's type, and requires it to be a subtype of the claim, so a certificate cannot smuggle in a bogus axiom (e.g. a free proof of `False`). The trusted surface of a re-check is this crate plus a JSON parser — no proof search, no SMT. The standalone `recheck` example reads a JSON certificate from a path:

```bash
cargo run -p logicaffeine-kernel --example recheck --features serde -- cert.json
```

### Interface

`interface` is a vernacular text front-end (`TermParser`, `parse_command`/`Command`, `literate_parser`, `Repl`) for driving the kernel by hand — `Definition`/`Check`/`Eval`/`Inductive` commands and an English-like literate syntax. It builds `Term`s for the trusted core; it is not part of the trusted surface.

## Feature flags

| Feature | Default | Effect |
|---------|---------|--------|
| `serde` | off | derives `Serialize`/`Deserialize` on `Term`/`Literal`/`Universe` and compiles the `certificate` module + `recheck` example for proof-certificate (de)serialization |

The trusted core stays dependency-free unless certificates are being (de)serialized.

## Dependencies

- **Internal**: `logicaffeine-base` — the only dependency; supplies `UnionFind`, re-used by the `cc` congruence-closure e-graph. **No lexicon dependency** (Milner invariant).
- **External**: `serde` (optional). `serde_json` is a dev-dependency only — it powers the `recheck` example and the certificate tests and never enters the published library's dependency graph.

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
