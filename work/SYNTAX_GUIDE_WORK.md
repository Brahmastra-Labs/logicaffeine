# Syntax Guide Audit — Working Log

**Owner:** `/loop` session started 2026-06-25.
**Goal:** Make the LOGOS Syntax Guide (`apps/logicaffeine_web/src/ui/pages/guide/`) world-class:
1. Every code example is factually accurate and **actually runs** (Logic → `compile_for_ui`, Imperative → `interpret_for_ui`).
2. No important language feature is **missing** from the guide.
3. Bugs found get patched **forever** via TDD (fix the implementation, never weaken a red test).
4. Audit against competitor guides (`COMPETITION_GUIDES/`), log envy items in `COMPETITION_DISCOVERIES.md`.

**Constraint:** This is primarily an AUDIT. Do not rewrite/restructure the guide prose. Fix genuine
runtime/language bugs with TDD; flag content-only errors for the user before editing prose.

## Scope (as of first pass)

- **25 sections**, **93 code examples** (87 Imperative, 5 Logic, ~1 unclassified).
- Guide rendered from `content.rs` `SECTIONS` constant.
- Examples executed in-browser via `logicaffeine_compile::{compile_for_ui, interpret_for_ui}`.
- **Gap found:** there were **zero** tests verifying the guide examples compile/run. An example can rot
  silently. First deliverable = an exhaustive harness that runs every example through the real engine.

## Section inventory

| # | id | title | examples |
|---|----|-------|----------|
| 1 | introduction | Introduction | 0 |
| 2 | getting-started | Getting Started | — |
| 3 | variables-and-types | Variables and Types | — |
| 4 | operators | Operators and Expressions | — |
| 5 | control-flow | Control Flow | — |
| 6 | functions | Functions | — |
| 7 | collections | Collections | — |
| 8 | user-defined-types | User-Defined Types | — |
| 9 | generics | Generics | — |
| 10 | memory | Memory and Ownership | — |
| 11 | zone-system | The Zone System | — |
| 12 | concurrency | Concurrency | — |
| 13 | crdts | Distributed Types (CRDTs) | — |
| 14 | policy | Policy-Based Security | — |
| 15 | p2p | P2P Networking | — |
| 16 | error-handling | Error Handling | — |
| 17 | advanced | Advanced Features | — |
| 18 | modules | Modules | — |
| 19 | cli | The CLI: largo | — |
| 20 | stdlib | Standard Library | — |
| 21 | logic-mode | Logic Mode | — |
| 22 | assertions | Assertions and Trust | — |
| 23 | z3 | Z3 Static Verification | — |
| 24 | complete-examples | Complete Examples | — |
| 25 | quick-reference | Quick Reference | — |

## Progress log

### Iteration 1 (2026-06-25) — COMPLETE
- Mapped the guide architecture: `content.rs` (data) + `mod.rs` (render) + `GuideCodeBlock` (run).
- Confirmed examples run via `compile_for_ui` (Logic) / `interpret_for_ui` (Imperative) from `logicaffeine_compile`.
- Read the ENTIRE guide (25 sections, 92 examples) for manual cross-reference.
- **Shipped harness** `apps/logicaffeine_web/tests/guide_examples.rs` — a GREEN bidirectional
  partition spec: 73 playground-runnable examples MUST run clean; 19 compiled-only MUST surface a
  graceful diagnostic (and the test fails if either side drifts). Runs each example on its own
  thread w/ timeout + `catch_unwind`.
- **Fixed a genuine end-to-end bug (policy capabilities)** via TDD — see Findings A. Touched
  `analysis/discovery.rs`, `analysis/policy.rs` (new variant), `semantics/policy.rs`,
  `codegen/policy.rs`. Regression test `policy_capability_interp.rs` (4 cases) green; existing
  `report1_policy` (2), `phase50_security` (10), `e2e_policy` (14) all still green.
- Logged 19 interpreter-gap / guide-accuracy items + 3 open questions + 4 prose mismatches.

### Iteration 2 (2026-06-25) — competitor audit (in-lane, no builds)
- Tree was busy (other agent's `nextest -p logicaffeine-tests` active) → **deferred the harness
  re-run again**, did only no-build work.
- Captured reference notes for 4 best-in-class guides in `COMPETITION_GUIDES/` (Rust Book, Go Tour,
  Gleam Tour, Inform 7 — the natural-language comparable).
- **Full scrape via `COMPETITION_GUIDES/scrape.py`** (Python, per user request — full content, not just
  notes): pulled the complete source of the two clearly-licensed guides —
  `rust-book/` (111 md, MIT/Apache) + `go-tour/` (142 lesson+code files, BSD-3) = 265 files / 3.0 MB,
  each with LICENSE + MANIFEST. Gleam tour (`gleam-lang/language-tour`) and Inform 7 docs have **no
  clear permissive license**, so they stay summary-only (notes + links) rather than bulk-copied.
- Wrote `COMPETITION_DISCOVERIES.md`: 8 prioritized envy items mapped to our guide. Top three:
  (1) a goal-indexed **Recipe Book** axis (Inform 7), (2) a narrated **build-one-complete-program**
  tutorial (Rust ch.2/12/20) — also fixes the §24 "promised Mergesort example that doesn't exist"
  gap, (3) **exercises with self-check** reusing the existing Learn grader. Noted what we already do
  well (editable+runnable examples, dual Logic/Imperative mode).
- These are recommendations only — no guide prose/structure changed (awaiting user sign-off).

### Iteration 3 (2026-06-25) — CLI factual-accuracy audit (no build; tree busy w/ full verify suite)

Verified the guide's CLI/reference prose directly against `apps/logicaffeine_cli/src/cli.rs`
(read-only — no runnable examples in those sections, so the harness can't cover them):

- **BUG (factual error): §22 references `largo audit`** — "Find all trust statements in your codebase
  with `largo audit`." **No `Audit` command exists.** Real commands: `new, init, build, verify, run,
  check, opts, publish, login, logout`. The only "audit" in the CLI is a doc-comment on `largo opts`.
  → Fix is a prose change (surface to user; or repoint to whatever actually lists Trust statements,
  if anything does — needs confirming the feature exists at all).
- **GAP: §19 CLI table is accurate but incomplete.** Every command/flag it lists is real, but it omits:
  - `largo run --interpret` (`-i`) — sub-second tree-walker feedback; this is the CLI twin of the
    playground philosophy and arguably the most important dev-loop command to document.
  - `largo opts <file> [--json]` — reports which optimizations actually fire.
  - `largo build --lib`, `--target <triple>` (`wasm` shorthand → WASM!), `--native-functions`.
  - `largo run --release`, and `largo run -- <args>` (program arg passing).
- Everything the guide DOES claim (`new`, `init`, `build [--release]`, `run`, `check`, `verify`,
  `build --verify`, `login`, `publish [--dry-run]`, `logout`) **is correct.**

**FIXED (user: "if you find bugs or factual issues you are FIXING them"):** edited `guide/content.rs`
(string constants only — safe, can't break compilation; `guide/` is this loop's lane, not the Learn
agent's):
- §22: removed the false `largo audit` claim → now "search your source for `Trust that`" (truthful;
  Trust statements are plain text + carry the mandatory `because`).
- §19: added rows for `largo run --interpret`, `largo run --release`, `largo build --target wasm`,
  `largo opts <file>`.
- §24: the prose promised **Mergesort / Working with Structs / Collection Processing** examples that
  don't exist → rewrote the headings to match the actual examples (Factorial, Fibonacci, Filtering a
  Collection). (Richer additions like a real Mergesort example are a follow-up — they need a
  build to verify they run.)

⚠ These edits are string-constant changes (guaranteed compile-safe), but **not yet build-verified**
because the tree is busy — the deferred harness re-run will confirm compile + examples-still-pass once
the tree is quiet. A "patched-forever" guard (a test scanning the guide for `largo <cmd>` mentions vs
the real command set) is still worth adding and needs a build — deferred to a quiet tree.

**Other documented-but-unverified claims to check when a build is available** (no example exists for
these, so the harness doesn't exercise them — factual-accuracy risk):
- §16 Error Handling prose promises a `Result`/Failure type + pattern matching, but the examples only
  use `If`-guards + sentinel returns. Does the language actually have `Result`? (verify, then either
  add an example or correct the prose.)
- §7 `Pop` and `… through …` slicing ("inclusive both ends") — documented, no example.
- §4 `is not` / `!=` — documented operator, no example.
- §8 `Inspect`/`When` pattern matching — documented, no example.

### Iteration 4 (2026-06-25) — verified the unexampled features + added examples (tree briefly quiet)

Probed the §16/§7/§4/§8 documented-but-unexampled claims through `interpret_for_ui_sync`:
- §7 `Pop from xs into last` ✓ (`3`, `[1, 2]`); `Pop from xs` ✓.
- §7 slice `items 2 through 3 of xs` ✓ → `[20, 30]` (1-based, inclusive). (`xs 2 through 3` / `item 2 through 3 of`
  do NOT parse — the canonical `items N through M of xs` is the form.)
- §4 BOTH `is not` and `!=` ✓ → the operators table is **accurate, no fix needed**.
- §8 `Inspect`/`When` ✓ → "north". Pattern matching works.
- §16 `Result of T and E` is a real stdlib enum (`assets/std/core.md`: `Ok(value)`/`Err(error)`); the type
  annotation parses. Prose is **accurate**, not a factual error — but I could NOT confirm the Ok/Err
  *construction* + `Inspect`-binding surface syntax in two tries (`a new Ok with value`/`When Ok with value v`
  and `Ok(..)`/`When Ok(v)` both fail). → Left §16 alone; a Result example needs the variant-field
  Inspect-binding syntax pinned (possible real gap — SURFACE, don't guess a broken example).

**ADDED verified examples (fills "missing important features/details"):**
- §7 `push-pop`: was mis-named (only showed Push) → now also demonstrates `Pop … into last`.
- §7 new `slicing` example (`items 2 through 3 of fruits`).
- §8 new `inspect-when` example (Direction pattern match) — pattern matching had NO example before.

⚠ **Harness re-run BLOCKED (not by me):** `cargo test -p logicaffeine-web --test guide_examples` failed to
compile a foundational dependency — `crates/logicaffeine_base/src/numeric.rs:1032` has a malformed hex
literal `0xR4710` (the OTHER agent's in-progress edit). No error touches `guide/content.rs` or the web
crate. My edits are compile-safe string/example constants. **Verification still pending a buildable tree.**
Left `numeric.rs` untouched (their lane).

### Iteration 5 (2026-06-25) — resolved §16 Result syntax via source (no build; tree busy w/ verify suite)

The other agent's `0xR4710` is fixed (now a valid literal), but a full `--workspace --features verification`
suite is running → still no builds. Used the time to read the corpus and crack the §16 Result syntax:
- Field-carrying enum variants are **constructed** `a new <Variant> with <field> <val>`
  (corpus: `Push a new Circle with radius 10`) and **matched** `When <Variant> (<bindings>):`
  (corpus: `When Circle (r):`, `When Success (v):`, `When Node (l, r):`).
- So my earlier Result probe was right on construction (`Return a new Ok with value (a / b)`) but wrong on
  the match arm (`When Ok with value v:` → should be **`When Ok (v):`** — positional binding).
- Likely-correct §16 example (NOT yet shipped — needs harness verify that the *stdlib* `Result` Ok/Err are
  constructible from user code; if not, define a self-contained `An Outcome is either: Success (v). Failure (m).`):
  ```
  ## To checked_div (a: Int) and (b: Int) -> Result of Int and Text:
      If b equals 0:
          Return a new Err with error "divide by zero".
      Return a new Ok with value (a / b).
  ## Main
  Let r be checked_div(10, 2).
  Inspect r:
      When Ok (v):
          Show "got " + v.
      When Err (e):
          Show "error: " + e.
  ```
  → Verify + add (or fall back to the custom-enum form) once the tree is buildable.

### Iteration 6 (2026-06-25) — type-name alias accuracy (no build; tree busy w/ workspace suite)

Spot-checked type-name aliases an "old AI model" might have invented, against the lexer/parser/type source:
- §13 `ORMap` (alias for SharedMap) — REAL (parser + discovery). ✓
- §13 `SharedSequence (YATA)` / `CollaborativeSequence` — REAL (`YATA` token, lexer). ✓
- §13 `SharedSet (AddWins)` / `(RemoveWins)` — REAL (bias modifiers). ✓
- **§3 + §25 "Float / Real" — SUSPECTED FACTUAL ERROR.** `Float` is handled as a type name throughout
  (`codegen/marshal.rs`, `interpreter.rs` `"Float" => RuntimeValue::Float`, `vm/compiler.rs` SlotKind).
  `Real` appears ONLY as the internal float-*literal* AST kind (`ast::NumberKind::Real`) and in
  doc-comments ("a real day"); there is **no `"Real"` type-name handling anywhere**. So `Let x: Real be 3.14`
  almost certainly does NOT type as a Float — `Real` looks like a hallucinated alias.
  → **Verify** `Let x: Real be 3.14.` once the tree is buildable; if it errors, **fix** §3 and §25
  primitive-type tables (drop "/ Real", leaving `Float`). Not editing on a guess (could be a valid alias
  via some normalization I can't see statically).

### Iteration 7 (2026-06-25) — RESOLVED §16 + §3 in a brief quiet window (probe ran; harness re-blocked)

A quiet window let a compile-crate probe run (then the tree went busy again before the harness):

- **§16 RESOLVED + example ADDED.** The *stdlib* `Result` Ok/Err path **PANICS** the interpreter
  (`Return a new Ok with value …` + `When Ok (v):` → panic) — a real foundational bug (SURFACE, below).
  But the **self-contained tagged-union pattern works**: `An Outcome is either: Success (value:Int).
  Failure (message:Text).` + `a new Success with value n` + `When Success (v):` → clean "ok 5". Added a
  verified §16 example `result-pattern` using that (matches the prose's "handle success and failure cases").
  ⚠ harness-confirmation of the exact two-call string still pending a quiet tree.

- **§3/§25 Float/Real — my iteration-6 suspicion was BACKWARDS. Corrected:**
  - `Let x: Real be 3.14.` ✓ works.  `Let x: Float be 3.14.` ✗ ERRORS: "expected a value of type 'Float'
    but found 'Real'" — decimal literals are typed `Real`, and `: Float` rejects them (in the interpreter).
  - So `Real` is the REAL decimal type; the guide is NOT wrong to list it. The misleading part is presenting
    `Float`/`Real` as interchangeable when `: Float` rejects a decimal literal. This is a **type-system
    inconsistency** (Float vs Real not unified) — foundational, owner's call, NOT a safe unilateral prose
    edit (unknown whether `: Float` works in *compiled* code vs the interpreter). SURFACED, not changed.

**FOUNDATIONAL BUGS SURFACED (out of lane — for the language/compile owner, not edited by this loop):**
1. Constructing the **stdlib `Result`'s `Ok`/`Err`** panics the interpreter (`a new Ok with value …`).
   The self-contained user-enum equivalent works fine, so it's specific to the prelude `Result` enum.
2. **`Float` vs `Real` type split**: decimal literals are `Real`; `Let x: Float be 3.14` is rejected by the
   interpreter's type check. Either unify them or the guide must document the distinction.

### Iteration 8 (2026-06-25) — harness run VERIFIED the session (caught a bonus promotion)

Caught a quiet window and ran the harness (`run5`). Result: **95 examples, the ONLY 2 mismatches were
`pipe-send-receive` + `select-timeout`** flagged "UNEXPECTEDLY RUNS CLEAN". Everything else passed, which
**verifies the whole session at once**:
- §19/§22/§24 prose edits compiled (the web crate built + ran).
- The 4 added/enhanced examples (push-pop w/ Pop, slicing, inspect-when, **result-pattern**) all ran clean
  (in the 77 runnable, not in the failure list).
- The `zone-mapped` → runnable move is green.

The 2 "failures" were the bidirectional guard working as designed: my earlier **Pipe parse fix** didn't
just fix parsing — the interpreter actually *runs* pipes/select, they were only blocked by the
`a new Pipe of T` parse bug. So they're now playground-runnable. **Promoted** them out of
`REQUIRES_COMPILATION` (partition is now 79 runnable + 16 compiled-only). The confirming re-run after that
one-line edit is a formality (run5's data already proves green) — tree went busy again before it landed.

- **NEW guide-prose finding:** the `pipe-send-receive` / `select-timeout` example LABELS still say
  "(Compiled Only)" and the §12 note says channels/select "don't run in the browser playground" — but they
  DO run on the interpreter now. Labels/note are stale. (NOTE: `select-timeout` uses a real `After 2 seconds`
  timer; behavior parity in WASM vs native isn't separately confirmed — so verify before relabeling.)

### Iteration 9 (2026-06-25) — GREEN + output-correctness spot-check

- **Harness re-run GREEN** (`run6`): **95 examples = 79 playground-runnable + 16 compiled-only**, test
  passes. Every session edit is verified; tree left clean.
- **Output-correctness spot-check** (deterministic examples — harness only checks no-error, this checks the
  printed values): all correct, including the examples I added this session:
  arith `13/7/30/3/1`, `5!=120`, fib `0,1,1,2,3,5,8,13,21,34`, filter `[5, 8, 3, 7]`, total `150`,
  **slicing → `[banana, cherry]`**, **result-pattern → `Halved: 5` / `Error: …`**.

### Status: core audit COMPLETE + verified-green. Remaining work needs USER input or is OUT-OF-LANE:

**Fixed + verified this session:**
- 3 language bugs (TDD, regression tests): policy capability (lexeme/lemma + cross-field), `a new Pipe of T`
  parse, `mapped from <variable>` parse.
- 3 prose factual errors: §22 `largo audit` (nonexistent), §19 missing CLI commands, §24 phantom example list.
- 4 examples added/enhanced: push-pop (now shows Pop), slicing, inspect-when (pattern matching!), result-pattern.
- 2 examples promoted to runnable (pipe/select) after the Pipe fix.
- Exhaustive harness `guide_examples.rs` (green, bidirectional partition guard) + 3 regression tests.
- Competitor audit: `COMPETITION_GUIDES/` (full Rust Book + Go Tour scrapes via `scrape.py`) + `COMPETITION_DISCOVERIES.md`.

**Surfaced for the language owner (foundational, NOT edited — other agent's lane):**
- stdlib `Result` Ok/Err construction PANICS the interpreter.
- `Float` vs `Real` type split (decimal literals are `Real`; `: Float` rejects them).

**Needs USER decision (guide prose/structure — not changed unilaterally):**
- §13: 6 CRDT examples shown runnable but interpreter defers to compiled Rust (label or implement?).
- pipe/select "(Compiled Only)" labels + §12 note now stale (they run) — verify WASM-timer parity then relabel.
- COMPETITION_DISCOVERIES recommendations: recipe/cookbook axis, build-one-program tutorial, self-check exercises.

### Optional future rigor
- Promote the output-correctness spot-check into a permanent golden-output test (web crate, reads SECTIONS).

### Iteration 10 (2026-06-25) — regression-guard run BLOCKED by upstream WIP (not mine)

Tree quiet but **un-buildable**: the other agent's numeric-tower work added `BinaryOpKind::ExactDivide`
(+ BitXor/Shl/Shr, CompressionCodec) and is mid-wiring it — non-exhaustive `match` errors across
`logicaffeine_kernel` (reduction/prelude/simp/lia), `logicaffeine_proof` (engine), and
`logicaffeine_language/build.rs`. **Zero errors in my files** (guide/content, guide_examples, the 3
regression tests). Left all upstream files untouched (their lane). My guide work remains
**verified-green as of `run6`** (95 examples: 79 runnable + 16 compiled-only); the regression-guard will
re-confirm once the tree builds again. Holding pattern — core audit is complete; nothing of mine to fix.

### Iteration 13 (2026-06-25) — regression GREEN after parser/codegen/type-analysis churn

Change-gate flagged a big batch since `run9` (parser/mod.rs, codegen stmt/types/expr, analysis
unify/types/check, optimize/fold, compile.rs — the other agent's numeric-tower work). After a few
busy-tree deferrals, `run10` ran on a quiet window: **95 examples = 79 runnable + 16 compiled-only,
PASS — no drift.** The audit is now robust to the full numeric-tower churn. New change-gate reference =
`run10`. Loop back to change-gated holding pattern; nothing of mine outstanding.

### Iteration 12 (2026-06-25) — division-pass change detected → regression check PENDING

Switched to change-gated regression checking (only rebuild when guide-relevant engine source changed
since the last green run, to avoid wasteful 4-min web-crate rebuilds on a contended tree). Detected:
`crates/logicaffeine_compile/src/resolve_division.rs` changed since `run8` (the other agent's
`Divide → ExactDivide` rewrite). That touches division semantics → must re-verify integer division.
**RESOLVED (`run9` + division probe):** harness green (95: 79+16), AND a targeted output check confirms
`10 / 3 → Quot: 3`, `10 / 2 → Half: 5`, and result-pattern `checked_halve(10) → Halved: 5` — all integer
floor, no rationals. The `ExactDivide` rewrite correctly spared integer-context division. No drift.

### Iteration 11 (2026-06-25) — regression re-confirm GREEN after upstream numeric-tower WIP

Tree buildable again (the other agent's `ExactDivide` wiring is resolved). `run8` re-ran the harness:
**95 examples = 79 runnable + 16 compiled-only, PASS — no drift.** The numeric-tower changes did not
break any guide example. Loop is now a pure regression guard; nothing of mine outstanding. Remaining
work = the 3 user-decision items above.
- **Harness confirming re-run**: `cargo test -p logicaffeine-web --test guide_examples` after the
  `zone-mapped` → runnable move. Run when no other suite is active.

### Next iterations (TODO)
- **Output-correctness audit:** the harness checks runnable examples don't ERROR; it does NOT yet
  check they produce the RIGHT output. Add golden-output assertions for representative examples
  (catches semantic bugs the no-error check misses).
- **Competitor guides:** scrape Rust Book / Python / Go / etc. into `COMPETITION_GUIDES/`; log envy
  items in `COMPETITION_DISCOVERIES.md`.
- **Prose mismatches:** §24 missing Mergesort/structs/collection examples; missing `Inspect`/`When`
  + `Pop` examples (needs user OK before editing guide content).
- **Open questions** (CRDT accuracy, compiled-only Run-button UX, network engine divergence) —
  surface to user.

## Findings (bugs / inaccuracies)

### Deeper triage (after "did you fix ALL bugs?"): 3 genuine bugs, not 1 — all FIXED

I initially fixed only the policy-capability bug and filed the other 19 as "compiled-only."
That was too quick. I then verified the real contract — **`compile_to_rust` on every imperative
example** — which is the true "is the syntax proper" test (the playground interpreter is allowed
to defer compiled-only runtimes). Result: **86/87 compiled; the 1 failure (`zone-mapped`) was a
real bug.** Two more genuine bugs surfaced. All three now fixed with TDD:

1. **Policy capability** (below, Finding A).
2. **`a new Pipe of Int` did not parse** — the channel-construction parser matched `a Pipe of T`
   but never skipped the `new` keyword the guide (and every other `a new X`) uses. Fixed in
   `parser/mod.rs` (skip optional `New`). Caused the `pipe-send-receive` AND `select-timeout`
   compile failures. Regression test `pipe_new_syntax.rs` (both `a new Pipe` and bare `a Pipe`).
3. **`mapped from <variable>` did not parse** — only a string literal was accepted, but the guide's
   `process_file(path: Text)` maps a file by parameter. Added `ast::stmt::ZoneSource{Literal,Variable}`
   (Copy enum; all optimizer pass-throughs transparent) wired through `parser` + `codegen/stmt.rs`
   (`new_mapped(&path)` vs `"lit"`). Regression test `zone_mapped_from_var.rs`. ⚠ Touches the
   foundational `logicaffeine_language` AST — see Lane note below.

**Net: 87/87 imperative examples now compile to Rust.** The remaining 18 compiled-only examples
(networking, channels, sync/persist, advanced CRDT mutations) genuinely COMPILE — they are
accurately "compiled-only," and their playground failures are graceful in the real (wasm) browser
(the native `Connect` tokio panic + PeerAgent VM/tree-walker divergence are debug/native-harness
artifacts, not browser behavior). Noted, not fixed.

> **Lane note (course-correction):** the Pipe + Zone fixes edit the shared/foundational
> `logicaffeine_language` crate while another agent works in compile/jit/forge. Going forward this
> loop stays in-lane: web-crate harness, docs, and competitor guides only; further language bugs are
> SURFACED here for the crate owner rather than fixed in-place. No builds while another suite runs.

> **⚠ Harness pending re-run:** `zone-mapped` now parses and the interpreter runs its body, so it was
> moved OUT of `REQUIRES_COMPILATION` (now expected to run clean). Needs a confirming
> `cargo test -p logicaffeine-web --test guide_examples` run **once the tree is quiet** (other
> agent's `nextest -p logicaffeine-tests` is active — not piling on).

**A. GENUINE BUG — FIXED via TDD (iteration 1):**

- **`security-capability` (§14 Policy)** — `Check that alice can edit doc` died with
  `No capability 'Edit' defined for type 'User'`, and even the compiled path generated
  invalid Rust (`self.name == the`). Two distinct root-cause bugs, both fixed:
  1. **Action symbol mismatch.** The `Check … can <verb>` site resolves the verb by its
     *lemma* (`"Edit"`); discovery registered the capability by the verb *lexeme* (`"edit"`)
     because `consume_noun_or_proper` returns the lexeme for a `Verb` token. The symbols never
     matched → every capability check missed. Fixed in `analysis/discovery.rs`: capability
     actions now take the verb lemma (matching the Check site + the dead-but-intended fallback).
  2. **Cross-field object condition unsupported.** `the user's name equals the document's owner`
     fell into the `FieldEquals` branch, which only parses a literal/identifier RHS — the object
     possessive `the document's owner` was dropped (parsed as bare `the`). Added a distinct
     `PolicyCondition::SubjectFieldEqualsObjectField` variant wired through discovery (parse),
     `semantics/policy.rs` (eval: `self.<f> == object.<g>`), and `codegen/policy.rs`
     (`self.name == document.owner`). The pre-existing whole-struct form
     `the user equals the document's owner` (`self == &document.owner`, owner:User) is left
     intact — `phase50_security` + `e2e_policy` still green (10 + 14).
  - Regression test: `crates/logicaffeine_compile/tests/policy_capability_interp.rs`
    (granted-via-admin, granted-via-owner, DENIED-when-neither, single-line). All green.

**B. INTERPRETER GAPS / GUIDE ACCURACY — 19 examples that require the compiled runtime**
(now encoded in the harness's `REQUIRES_COMPILATION` partition so the suite is green and
guards both directions):

| Example | Section | Interpreter diagnostic | Labeled "Compiled Only"? |
|---------|---------|------------------------|--------------------------|
| zone-mapped | §11 | parse error: expected file-path string (uses a *variable* `path`; doc syntax shows a literal) | yes |
| pipe-send-receive | §12 | parse error | yes |
| select-timeout | §12 | parse error | yes |
| crdt-sync-counter / -profile | §13 | "Sync requires a prior Connect to a relay" | yes |
| crdt-persistent | §13 | "VFS not initialized" | yes |
| crdt-divergent | §13 | "Resolve conflict is not supported in the interpreter" | **NO** |
| crdt-sharedset / -bias | §13 | "Add collection must be an identifier" | **NO** |
| crdt-sequence / -collaborative | §13 | "Append to sequence is not supported in the interpreter" | **NO** |
| crdt-sharedmap | §13 | "SetIndex collection must be an identifier" | **NO** |
| network-* (7) | §15 | "Listen/Send requires a prior Connect" / PeerAgent unsupported / tokio "no reactor" panic | section note only |

**Open questions for the user (NOT changed without approval):**
1. **§13 CRDT accuracy:** six CRDT examples (Divergent / SharedSet / SharedSequence /
   CollaborativeSequence / SharedMap) are presented as playground-runnable but the interpreter
   defers them to compiled Rust. Either (a) implement these CRDT mutations in the interpreter,
   or (b) mark them "(Compiled Only)" like the sync ones. The CRDT section note currently says
   only `Sync`/`Mount` need compilation — that understates it.
2. **Compiled-only UX:** ~19 examples render a "Run" button that errors/panics in the playground.
   World-class option: a `runnable: bool` (or "compile to see output") affordance on
   `CodeExample` so non-runnable examples don't present a failing Run button. Pure guide change.
3. **`network-connect` panics** ("no reactor running") and **`network-peer-agent` diverges**
   between VM and tree-walker (debug shadow-oracle) — both are compiled-only, but the VM
   rejecting PeerAgent while the tree-walker accepts it is a latent engine inconsistency worth
   a follow-up.

### Content/prose mismatches noted during manual read (not yet actioned)

- **§24 Complete Examples:** prose headings list **Mergesort**, Factorial, **Working with
  Structs**, **Collection Processing** — but the actual examples are Factorial, Fibonacci,
  Filter. Mergesort/structs/collection-processing examples are described but absent.
- **`Inspect` / `When` pattern matching** is documented (§8, Quick Reference) but NO example
  demonstrates it; the enum example only does `Let heading be North. Show heading.`
- **`Pop`** is documented (§7) but no example shows it (only `Push`).
- **`network-send-message` struct field uses `a message (Text).`** (parenthesized type) while
  every other struct uses `a field: Type.` — inconsistent, though it parses.

## Competitor audit

_(COMPETITION_GUIDES/ + COMPETITION_DISCOVERIES.md — later iterations)_
