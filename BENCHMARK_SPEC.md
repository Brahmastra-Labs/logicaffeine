# BENCHMARK DOMINATION (v2 — codebase-grounded)

Engineering specification for LogicAffeine to achieve state-of-the-art results on FVEval, VERT, and AssertionBench. Full IEEE 1800-2017 SVA compliance. Every sprint has concrete RED tests. Every test describes what it proves and why that proof is sufficient. No compromise.

**Prerequisite: CRUSH + SUPERCRUSH complete (800+ hardware tests, multi-sorted Z3 equivalence, Kripke temporal logic).**

### Codebase Baseline (verified by exploration)

| Component | Location | Status | Lines |
|---|---|---|---|
| SvaExpr enum | `codegen_sva/sva_model.rs:11-141` | **41 variants** (not 31) — includes OneHot, OneHot0, CountOnes, AcceptOn, RejectOn, GotoRepetition, NonConsecRepetition | 951 |
| SVA parser | `sva_model.rs:159-951` | 9-level precedence: toplevel → implication → or → seq_ops → and → sequence → eq → unary → atom | ~800 |
| SVA emitter | `sva_model.rs:955-1078` | All 41 variants | ~125 |
| Structural eq | `sva_model.rs:1091-1250` | All 41 variants | ~160 |
| Bounded translator | `sva_to_verify.rs:116-607` | All 41 variants → BoundedExpr | ~500 |
| FOL→SVA synthesis | `fol_to_sva.rs:84-256` | Kripke patterns → SVA text strings | ~175 |
| k-Induction | `logicaffeine_verify/kinduction.rs` | **Fully implemented** — base + inductive step, 9+ tests | 437 |
| IC3/PDR | `logicaffeine_verify/ic3.rs` | **Fully implemented** — frames, generalization, propagation, convergence, 8+ tests | 554 |
| Liveness | `logicaffeine_verify/liveness.rs` | **Fully implemented** — Biere reduction, fairness, lasso extraction, 6+ tests | 256 |
| Block types | `token.rs:83-112` | Already has `Hardware`, `Property` — need to add `Specification` | — |
| Lexer block detection | `lexer.rs:1841-1869` | Match on block name → BlockType | — |
| Temporal operators | `ast/logic.rs:165-190` | Always, Eventually, Next, Past, Future + Until, Release, WeakUntil | — |
| Kripke lowering | `semantics/kripke.rs:78-196` | G→∀w', F→∃w', X→Next_Temporal, world-enriched predicates | — |
| Hardware test files | `crates/logicaffeine_tests/tests/phase_hw_*.rs` | **61 files**, 500+ tests | — |

---

## Part I: The Target

### What We Are Proving

LogicAffeine generates SystemVerilog Assertions from natural language specifications and RTL designs. For every assertion it generates, it provides a Z3 correctness proof or flags the specification as ambiguous. The target: **100% correct or flagged as ambiguous. No wrong answers, ever.**

### The Benchmarks

| Benchmark | Source | Cases | Task | Gold Standard |
|---|---|---|---|---|
| FVEval NL2SVA-Machine | NVIDIA (NVlabs/FVEval) | 300 | Hardware NL → SVA | CSV with reference SVA |
| FVEval NL2SVA-Human | NVIDIA (NVlabs/FVEval) | 79 assertions, 13 designs | Ambiguous NL + testbench → SVA | CSV with reference SVA |
| VERT | AnandMenon12/VERT | 20,000 | SystemVerilog code → SVA | JSON Lines with reference assertions |
| AssertionBench | achieve-lab/assertion_data_for_LLM | 101 designs, 81K proven | Verilog design → assertions | HARM-mined, JasperGold-proven |

### What Makes Us Win

We compete directly against LLM-based tools. We are not a verification layer for LLMs — we are a replacement.

| Claim | Basis |
|---|---|
| Zero signal hallucination | Signals extracted from spec/RTL, not generated probabilistically |
| Z3 correctness certificate per output | Every generated SVA is formally verified against the specification |
| Cycle-accurate counterexamples | When output is wrong, we show WHY with a trace |
| Deterministic | Same input always produces same output |
| Honest about uncertainty | Ambiguous specs flagged, not hallucinated through |
| Full IEEE 1800-2017 compliance | Not a subset — the real thing |
| No JasperGold required | Z3 as open-source formal verification backend |

### Published Baselines (From Papers)

| Model | FVEval NL2SVA-Machine | Hallucination Rate |
|---|---|---|
| GPT-4 | ~65% functional correctness | ~30% |
| GPT-3.5 | ~38% | ~45% |
| LLaMA-3-70B | ~41% | ~40% |
| AssertionForge (GPT-4o + KG) | ~75% | ~20% |
| **LogicAffeine (target)** | **100% or flagged** | **0%** |

---

## Part II: Architecture

### Pipeline Design

```
                  ┌──────────────────────────────────────────────┐
                  │              ## Specification                  │
                  │   "If sig_A is high, then sig_B must         │
                  │    be high 3 clock cycles later"             │
                  └──────────────┬───────────────────────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │    LOGOS Parser          │
                    │  (extended with HW       │
                    │   vocabulary)            │
                    └────────────┬────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │   FOL + HW Primitives   │
                    │  (bitvectors, delays,    │
                    │   reduction ops)         │
                    └────────────┬────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │   Kripke Lowering        │
                    │  (world quantification)  │
                    └────────────┬────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │   FOL → SVA Synthesis    │
                    │  (pattern matching on     │
                    │   Kripke structure)       │
                    └────────┬────────┬───────┘
                             │        │
                    ┌────────▼──┐  ┌──▼──────────┐
                    │  SVA Out  │  │ BoundedExpr  │
                    │ (assert   │  │ (timestep    │
                    │  property)│  │  unrolling)  │
                    └───────────┘  └──────┬──────┘
                                          │
                              ┌────────────▼────────────┐
                              │   Z3 Equivalence Check  │
                              │  ¬(spec ↔ sva) → UNSAT  │
                              │  or counterexample trace │
                              └─────────────────────────┘
```

For RTL-based benchmarks (VERT, AssertionBench):

```
    SystemVerilog Code ──► RTL Behavioral Parser ──► SVA (direct)
                                     │
                                     └──► FOL encoding ──► Z3 verification
```

### Key Architectural Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Parser architecture | Extend existing LOGOS parser | Preserves the formal semantics pipeline. NL → FOL → Kripke → SVA. The full story. |
| Block marker | `## Specification` | Like `## Main` for programs. Activates hardware vocabulary in the parser. |
| FOL extensions | Full bitvector theory | BV(N) types, extract/concat, arithmetic. Maps to Z3 QF_BV. Not a subset — the real thing. |
| Temporal delays | Delay(n) + DelayRange(min,max) | Two separate constructs. Delay(5) = ##5. DelayRange(1,3) = ##[1:3]. |
| Lexicon | Extend existing lexicon.json | Hardware entries alongside existing words. Mode-activated by ## Specification. |
| NL grammar | Flexible — FVEval + our own | "Support all versions of the language and allow mix and matching, just like English." |
| Expression parsing | Recursive descent on NL | Full expression tree parsing for nested operator descriptions. |
| Feature gating | Always on | No Cargo feature flag. Hardware extensions are part of the core language. Zero regression required. |
| Error handling | Flag as ambiguous | Both structured diagnostic (machine) and Socratic explanation (human). |
| RTL pipeline | SVA direct + FOL for verification | Two paths: generate SVA from RTL structure, verify via FOL encoding + Z3. |
| Proof strength | Bounded first, then unbounded | k-induction + IC3 hardened. Report both bounded and unbounded columns. |
| Benchmarks | All — FVEval + VERT + AssertionBench | Full sweep. All 101 AssertionBench designs. Full behavioral analysis. |
| Harness | Separate `sva-bench` crate | Unobtrusive. Benchmark infrastructure isolated from product code. |
| LLM verifier | No | Compete directly. We are a replacement, not a complement. |
| Latency | Report ours only | PhD students will benchmark LLMs later. |

---

## Part III: IEEE 1800-2017 SVA Compliance

### Current SvaExpr Coverage

The SvaExpr enum has 31 variants after CRUSH + SUPERCRUSH. The following table maps IEEE 1800-2017 clauses to implementation status.

### What Must Be Added

#### Tier 0: FVEval-Critical (Blocks Benchmark)

| Construct | SVA Syntax | IEEE Clause | Occurrences in FVEval |
|---|---|---|---|
| Reduction AND | `&sig` | 11.4.12 | 21 |
| Reduction OR | `\|sig` | 11.4.12 | 19 |
| Reduction XOR | `^sig` | 11.4.12 | 20 |
| Reduction NAND | `~&sig` | 11.4.12 | 15 |
| Reduction NOR | `~\|sig` | 11.4.12 | 24 |
| Reduction XNOR | `~^sig` | 11.4.12 | 12 |
| Bitwise NOT | `~sig` | 11.4.5 | 22 |
| Binary XOR | `a ^ b` | 11.4.6 | ~30 |
| Identity equality | `a === b` | 11.4.5 | 77 |
| Identity inequality | `a !== b` | 11.4.5 | 150 |

#### Tier 1: Sequence Composition (Clause 16.13)

| Construct | SVA Syntax | What It Does |
|---|---|---|
| `intersect` (length-matched) | `seq1 intersect seq2` | Both match, same start AND end time |
| `throughout` | `cond throughout seq` | Boolean holds entire sequence duration |
| `within` | `seq1 within seq2` | seq1 occurs entirely inside seq2's boundaries |

**Current status:** `intersect`, `throughout`, `within` exist as SvaExpr variants but lack length-tracking semantics in BoundedExpr translation. `first_match` is identity passthrough.

#### Tier 2: Sequence Match Control (Clause 16.12)

| Construct | SVA Syntax | What It Does |
|---|---|---|
| `first_match` (proper) | `first_match(seq)` | Only first matching completion, discard subsequent |
| Endpoint methods | `seq.triggered`, `seq.matched` | Detect sequence completion in current cycle |

#### Tier 3: Advanced LTL (Clause 16.14)

| Construct | SVA Syntax | What It Does |
|---|---|---|
| `until` / `s_until` | `p until q` / `p s_until q` | Weak/strong until |
| `until_with` / `s_until_with` | `p until_with q` | Overlapping until |
| `nexttime` / `s_nexttime` | `nexttime[N] p` | Weak/strong next |
| Property `implies` | `p implies q` | Concurrent property implication (vs sequence `\|->`) |

#### Tier 4: Local Variables — The Final Boss (Clause 16.9)

| Construct | SVA Syntax | What It Does |
|---|---|---|
| Sequence-local variables | `int v; (req, v=data) ##[1:$] (ack && out==v)` | Capture value at sequence point, assert later |
| Assignment actions | `(expr, var = value)` | Bind variable during sequence match |

**Z3 translation:** Cannot use simple uninterpreted functions. Must thread state updates through bounded unrolling. `v` assigned at time `t` → Z3 expression for `v` at `t+k` must evaluate to `data@t`.

#### Tier 5: System Functions (Clause 16.12.14)

| Function | SVA Syntax | What It Does |
|---|---|---|
| `$countbits` | `$countbits(sig, '0, '1)` | Count bits matching control values |
| `$isunbounded` | `$isunbounded(param)` | Check if formal parameter is `$` |
| `$onehot` | `$onehot(sig)` | Exactly one bit is 1 |
| `$onehot0` | `$onehot0(sig)` | At most one bit is 1 |
| `$countones` | `$countones(sig)` | Count number of 1 bits |

#### Tier 6: Multi-Clocking (Clause 16.16)

| Construct | SVA Syntax | What It Does |
|---|---|---|
| Explicit clock change | `@(posedge clk1) req \|=> @(posedge clk2) ack` | Switch clocking mid-sequence |
| Clock resolution | Sampling semantics per clock domain | `$sampled` relative to each clock |

**Z3 translation:** Breaks single-step bounded unrolling. Must model clock ticks as separate booleans and transition between clock domains.

#### Tier 7: Directive Layer

| Construct | SVA Syntax | Z3 Translation |
|---|---|---|
| `assume property` | Constrains formal state space | Add to solver as constraint for all timesteps |
| `cover property` | Reachability analysis | Check SAT (not UNSAT) — find witness trace |
| `restrict property` | Limit search space | Constrain initial states / path prefixes |
| Action blocks | `$display("Pass"); else $error(...)` | Parse and discard for formal; must not choke parser |

#### Tier 8: Scope & Substitution

| Construct | SVA Syntax | What It Does |
|---|---|---|
| Property declarations | `property p(a,b); ... endproperty` | Reusable property templates |
| Sequence declarations | `sequence s(a,b); ... endsequence` | Reusable sequence templates |
| Instantiation | `assert property (p(req, ack));` | Substitute actual arguments |
| Default clocking | `default clocking @(posedge clk);` | Auto-wrap unclocked assertions |
| Default disable | `default disable iff (rst);` | Auto-wrap assertions with reset |

**Resolution pass:** AST rewrite before translation. Clone declaration body, substitute arguments, apply default clock/disable.

#### Tier 9: Liveness vs Safety

| Construct | SVA Syntax | Z3 Translation |
|---|---|---|
| `strong(seq)` | Sequence MUST complete | Liveness-to-safety conversion (Biere encoding) |
| `weak(seq)` | Sequence MAY not complete | Maps to bounded unrolling (if bound reached, pass) |
| `s_eventually` | Strong eventually (F) | Must find witness; liveness-to-safety if unbounded |
| `eventually` | Weak eventually | Pass if bound reached without violation |

#### Tier 10: Deferred Assertions & Complex Types

| Construct | SVA Syntax | What It Does |
|---|---|---|
| `assert #0` | Deferred immediate | Evaluated combinationally, not sampled |
| `assert final` | Final deferred | Evaluated at end of time step |
| Enum types in SVA | `state == ST_READ` | Lower to bitvector encodings |
| Struct fields | `req.addr == 32'hFF` | Flatten to bitvector concatenation or Z3 datatypes |

---

## Part IV: Sprint Specification

Every sprint follows TDD per CLAUDE.md: write RED tests first, implement until GREEN, run full suite for zero regressions. Tests describe what is tested and why proving that is sufficient.

### Phase A: SVA Model Completion

---

#### Sprint A0: FVEval-Critical SVA Operators

**Why:** 10 missing operator types block ~350+ FVEval reference assertions from being parsed or emitted. Without these, we cannot even COMPARE our output to the gold standard. This is the single highest-priority blocker.

**Files (with exact insertion points):**
- `sva_model.rs:140` — add new variants before closing brace of SvaExpr enum
- `sva_model.rs:560-727` (parse_unary) — add prefix `~`, `&`, `|`, `^` recognition
- `sva_model.rs:461-558` (parse_eq) — add `===` and `!==` before `==`/`!=`
- `sva_model.rs:1077` — add emitter match arms before closing brace
- `sva_model.rs:1247` — add structural eq match arms before `_ => false`
- `sva_to_verify.rs:607` — add translator match arms before closing brace
- `hw_pipeline.rs` — equivalence helpers

**What:**

Add to `SvaExpr` (at line 140, after `RejectOn`):
```rust
ReductionAnd(Box<SvaExpr>),        // &sig
ReductionOr(Box<SvaExpr>),         // |sig
ReductionXor(Box<SvaExpr>),        // ^sig
ReductionNand(Box<SvaExpr>),       // ~&sig
ReductionNor(Box<SvaExpr>),        // ~|sig
ReductionXnor(Box<SvaExpr>),       // ~^sig or ^~sig
BitwiseNot(Box<SvaExpr>),          // ~sig (distinct from logical !)
Xor(Box<SvaExpr>, Box<SvaExpr>),   // a ^ b (binary XOR)
IdentityEq(Box<SvaExpr>, Box<SvaExpr>),    // a === b
IdentityNotEq(Box<SvaExpr>, Box<SvaExpr>), // a !== b
```

Parser changes:
- `parse_eq()`: recognize `===` (3 chars) and `!==` (3 chars) before `==`/`!=`
- `parse_unary()`: prefix `~` checks for `~|`, `~&`, `~^` first, then bare `~`
- New `parse_prefix()` layer: `&sig`, `|sig`, `^sig` as unary reduction (distinguish from binary `&&`, `||`)

Emitter changes: `sva_expr_to_string()` for all 10 new variants.

Structural eq: `sva_exprs_structurally_equivalent()` for all 10 new variants.

Bounded translation: Each reduction op maps to a bitvector operation over all bits of the signal. `&sig` → `bv_eq(sig, all_ones(width))`. For boolean (1-bit), reduction is identity.

**RED tests (~40 tests) — `phase_hw_sva_operators.rs`:**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_reduction_and` | `&sig_A` parses to `ReductionAnd(Signal("sig_A"))` | Parser recognizes unary `&` as reduction, not binary |
| `parse_reduction_or` | `\|sig_B` parses to `ReductionOr(Signal("sig_B"))` | Parser recognizes unary `\|` before binary `\|\|` |
| `parse_reduction_xor` | `^sig_C` parses to `ReductionXor(Signal("sig_C"))` | Unary `^` distinct from binary XOR |
| `parse_reduction_nand` | `~&sig_D` parses to `ReductionNand(...)` | Two-char prefix handled correctly |
| `parse_reduction_nor` | `~\|sig_E` parses to `ReductionNor(...)` | Two-char prefix, not `~` + `\|sig` |
| `parse_reduction_xnor` | `~^sig_F` parses to `ReductionXnor(...)` | Two-char prefix variant |
| `parse_xnor_alt` | `^~sig_F` also parses to `ReductionXnor(...)` | IEEE 1800 allows both forms |
| `parse_bitwise_not` | `~sig_G` parses to `BitwiseNot(Signal("sig_G"))` | Bare `~` after ruling out `~&`, `~\|`, `~^` |
| `parse_binary_xor` | `sig_A ^ sig_B` parses to `Xor(...)` | Binary `^` in expression context |
| `parse_identity_eq` | `sig_A === sig_B` parses to `IdentityEq(...)` | 3-char `===` before 2-char `==` |
| `parse_identity_neq` | `sig_A !== sig_B` parses to `IdentityNotEq(...)` | 3-char `!==` before 2-char `!=` |
| `parse_nested_reduction` | `&(\|sig_A)` — reduction of reduction | Recursive parsing through parentheses |
| `parse_reduction_in_implication` | `sig_A \|-> &sig_B` — reduction in consequent | Reduction doesn't steal `\|` from `\|->` |
| `emit_reduction_and` | `ReductionAnd(Signal("x"))` emits `&x` | Emission matches SystemVerilog syntax |
| `emit_identity_eq` | `IdentityEq(...)` emits `x === y` | Three-equals emitted |
| `emit_binary_xor` | `Xor(...)` emits `x ^ y` | Caret for binary XOR |
| `roundtrip_all_10_variants` | Parse → emit → reparse produces identical AST for each variant | End-to-end consistency |
| `structural_eq_reduction_and` | Same reduction AND → structurally equal | Structural comparison handles new variants |
| `structural_neq_different_reductions` | `&sig` ≠ `\|sig` structurally | Different reduction types are not confused |
| `structural_eq_identity_vs_eq` | `a === b` ≠ `a == b` structurally | Identity and logical equality are distinct |
| `bounded_reduction_and_bool` | `&sig` for 1-bit signal → identity in BoundedExpr | Degenerate case: reduction on single bit |
| `bounded_reduction_and_bitvec` | `&sig` for 8-bit signal → all-ones check | Multi-bit: `bv_eq(sig, 0xFF)` |
| `bounded_reduction_or_bitvec` | `\|sig` for 8-bit → non-zero check | `bv_neq(sig, 0x00)` |
| `bounded_reduction_xor_bitvec` | `^sig` → parity (odd number of 1s) | XOR reduction is parity |
| `bounded_bitwise_not` | `~sig` → bitvector complement | Not logical negation |
| `bounded_binary_xor` | `a ^ b` → bitwise XOR | Distinct from logical OR |
| `bounded_identity_eq` | `a === b` → same Z3 encoding as `a == b` for formal | IEEE: `===` checks x/z too, but formal is 2-valued |
| `z3_reduction_and_equiv` | `&sig` equiv `sig == all_ones` via Z3 | Semantic correctness of reduction encoding |
| `z3_reduction_or_equiv` | `\|sig` equiv `sig != 0` via Z3 | Semantic correctness |
| `z3_demorgan_reduction` | `~&sig` equiv `\|~sig` via Z3 | De Morgan's law on reductions |
| `z3_xor_self_zero` | `sig ^ sig` equiv `0` via Z3 | XOR self-cancellation |
| `z3_identity_eq_same_as_eq` | `===` and `==` equivalent in 2-valued formal | Formal doesn't have x/z |
| `fveval_case_3_0_0` | Parse FVEval case 3_0_0 reference SVA successfully | Can handle real benchmark data |
| `fveval_case_3_1_0` | Parse case 3_1_0 (nested `!==`, `~\|`) | Complex multi-operator case |
| `fveval_case_3_2_0` | Parse case 3_2_0 (`\|-> ##5`) | Implication with delay, already-supported + new ops |
| `regression_existing_sva_parse` | All existing SVA parsing tests still pass | No regression from new operator precedence |
| `regression_existing_bounded` | All existing bounded translation tests still pass | No regression in translation pipeline |
| `parse_fveval_300_syntax_check` | Parse all 300 FVEval reference SVAs — count successes | Baseline: how many of 300 we can now parse |

---

#### Sprint A1: Sequence Composition with Length Tracking

**Why:** `intersect` currently uses `BoundedExpr::And` which ignores match length. IEEE 1800 requires both sequences start AND end at the same time. `throughout` and `within` desugar into `intersect`. Without proper length semantics, we cannot correctly verify sequence composition properties.

**Files:**
- `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs` — rewrite translate for intersect/throughout/within
- `crates/logicaffeine_compile/src/codegen_sva/sva_model.rs` — add match-length metadata if needed

**What:**

The fundamental change: sequence translation must return `(match_condition: BoundedExpr, match_length: u32)` tuples, not just `BoundedExpr`. Then:
- `intersect(s1, s2)` = `And(s1.condition, s2.condition)` where `s1.length == s2.length`
- `throughout(cond, seq)` = `And(cond_at_all_t_in_0..seq.length, seq.condition)`
- `within(inner, outer)` = `Exists offset in 0..outer.length. inner starts at offset AND fits within outer`

**RED tests (~20 tests) — extend `phase_hw_sva_operators.rs`:**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `intersect_same_length_passes` | `(a ##1 b) intersect (c ##1 d)` — length 2 both sides, AND condition | Same length sequences produce conjunction |
| `intersect_different_length_fails` | `(a ##1 b) intersect (c ##1 d ##1 e)` — length mismatch | Length 2 ≠ length 3, must be false/constrained |
| `throughout_signal_all_steps` | `valid throughout (req ##1 ##1 ack)` — valid asserted at all 3 timesteps | Boolean must hold entire sequence span |
| `throughout_not_just_endpoints` | `valid throughout (a ##2 b)` — checks valid@0, valid@1, valid@2 | Middle timesteps are not skipped |
| `within_inner_fits` | `(a ##1 b) within (c ##3 d)` — inner must occur somewhere inside outer | Existential over offsets |
| `within_inner_too_long` | `(a ##5 b) within (c ##2 d)` — inner longer than outer | Must be false |
| `intersect_z3_length_constraint` | Z3 confirms intersect rejects length mismatch | Semantic correctness via Z3 |
| `throughout_z3_all_steps` | Z3 confirms throughout checks all intermediate steps | Not just first and last |

---

#### Sprint A2: First Match with Priority Encoding

**Why:** `first_match` is currently identity passthrough. For bounded model checking, this causes combinatorial explosion from multiple overlapping matches. IEEE 1800 requires evaluation of only the first successful match.

**Files:**
- `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs` — replace passthrough with priority encoder

**What:**

`first_match(seq)` with bounded unrolling at timestep offsets `t0, t1, ..., tk`:
```
match_at_t0 OR (NOT match_at_t0 AND match_at_t1) OR (NOT match_at_t0 AND NOT match_at_t1 AND match_at_t2) ...
```

This is a nested if-then-else priority structure, not a broad disjunction.

**RED tests (~10 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `first_match_single` | Single match at t=0 — evaluates to that match | Degenerate case |
| `first_match_priority` | Multiple matches at t=0,t=1,t=2 — only t=0 matters | Later matches suppressed |
| `first_match_no_match` | No match at any timestep — evaluates to false | Empty match set |
| `first_match_z3_not_broad_disjunction` | Z3 confirms first_match ≠ broad disjunction when matches differ | Priority semantics differ from OR |
| `endpoint_triggered` | `seq.triggered` is true on completion cycle | Sequence endpoint detection |
| `endpoint_matched` | `seq.matched` is true on match cycle | Sequence endpoint detection (matched variant) |

---

#### Sprint A3: Advanced LTL Operators

**Why:** `until`, `s_until`, `nexttime`, `s_nexttime`, and property `implies` are needed for both FVEval NL2SVA-Human and AssertionBench. These are core LTL operators that our Kripke lowering already models but which are not yet in the SVA AST as first-class constructs with strong/weak distinction.

**Files:**
- `crates/logicaffeine_compile/src/codegen_sva/sva_model.rs` — add variants
- `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs` — bounded translation

**What:**

New `SvaExpr` variants:
```rust
Until { lhs: Box<SvaExpr>, rhs: Box<SvaExpr>, strong: bool },
UntilWith { lhs: Box<SvaExpr>, rhs: Box<SvaExpr>, strong: bool }, // overlapping
SNexttime(Box<SvaExpr>, u32),  // strong nexttime (existing Nexttime is weak)
PropertyImplies(Box<SvaExpr>, Box<SvaExpr>), // concurrent, not sequence-based
```

Bounded translation:
- `p until q`: `q@t0 OR (p@t0 AND q@t1) OR (p@t0 AND p@t1 AND q@t2) ...`. Weak: if bound reached without q, pass.
- `p s_until q`: Same disjunction but MUST find q before bound. Strong: if bound reached without q, fail.
- `s_nexttime[N] p`: Bound must reach N AND p holds at N. Strong: fail if bound < N.

**RED tests (~15 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_until` | `p until q` parses with strong=false | Keyword recognition |
| `parse_s_until` | `p s_until q` parses with strong=true | Strong prefix |
| `parse_until_with` | `p until_with q` — overlapping variant | Keyword variant |
| `parse_s_nexttime` | `s_nexttime[3] p` — strong, N=3 | Strong prefix + parameterized |
| `parse_property_implies` | `p implies q` — concurrent implication | Distinct from `\|->` |
| `bounded_weak_until_no_q` | Weak until: q never happens, bound reached → pass | Weak semantics: silence is acceptance |
| `bounded_strong_until_no_q` | Strong until: q never happens, bound reached → fail | Strong semantics: must find q |
| `bounded_until_q_immediate` | q holds at t=0 — passes immediately | Base case |
| `bounded_until_p_holds_until_q` | p at t=0..2, q at t=3 — passes | Standard until behavior |
| `bounded_s_nexttime_bound_too_short` | Bound=2, N=5 — strong nexttime fails | Bound insufficient for strong |
| `bounded_weak_nexttime_bound_too_short` | Bound=2, N=5 — weak nexttime passes | Weak: bound < N is acceptable |
| `z3_until_equiv_kripke` | `p until q` SVA equiv to Kripke `p U q` lowering | Cross-validation between pipeline paths |
| `implies_vs_implication` | Property `implies` ≠ sequence `\|->` | Different semantics: concurrent vs sequential |

---

#### Sprint A4: Local Variables (The Final Boss)

**Why:** Protocol verification (AXI data integrity, pipelined transactions) requires capturing values at one point in a sequence and asserting against them later. This is the hardest part of IEEE 1800 and what separates toy tools from real ones.

**Files:**
- `crates/logicaffeine_compile/src/codegen_sva/sva_model.rs` — new SequenceAction variant
- `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs` — state-threaded bounded translation

**What:**

New AST:
```rust
SequenceAction {
    expr: Box<SvaExpr>,
    assignments: Vec<(String, Box<SvaExpr>)>, // v = data_in
},
LocalVarDecl {
    name: String,
    sort: SvaSort,  // int, logic, etc.
},
```

Bounded translation: When `v = data_in` is assigned at time `t`, create Z3 variable `v_t = data_in@t`. All subsequent references to `v` at time `t+k` resolve to `v_t` (the value captured at assignment time, not the current value).

**RED tests (~15 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_local_var_decl` | `int v;` parses as LocalVarDecl | Variable declaration in sequence |
| `parse_sequence_action` | `(req, v = data)` parses with assignment | Comma-separated action |
| `parse_full_local_var_usage` | `int v; (req, v=data) ##[1:5] (ack && out==v)` | Full pattern |
| `bounded_local_var_captures` | v assigned at t=0, checked at t=3 → Z3 var `v_0` at both points | Value captured at assignment time |
| `bounded_local_var_not_current` | v assigned at t=0, data changes at t=1, check at t=2 → uses t=0 value | Capture semantics, not current value |
| `bounded_multiple_captures` | Two assignments at different times → two distinct Z3 variables | No aliasing between captures |
| `z3_axi_data_integrity` | AXI: data captured on request, verified on grant | Real protocol pattern |
| `z3_pipeline_value_tracking` | Pipelined: value enters stage1, exits stage3 unchanged | Multi-stage capture |

---

#### Sprint A5: System Functions (Extend Existing)

**Why:** `$onehot`, `$onehot0`, `$countones` ALREADY EXIST in SvaExpr (lines 105-117 of sva_model.rs) along with `IsUnknown`, `Sampled`, `Bits`, `Clog2`. What's MISSING: `$countbits`, `$isunbounded`, and proper Z3 translations for the existing ones. The existing BoundedExpr translations may be stubs.

**Files:**
- `sva_model.rs:105-117` — add `CountBits`, `IsUnbounded` variants after existing system functions
- `sva_to_verify.rs` — verify/complete bounded translations for all system functions
- `sva_model.rs` parse_unary — verify `$countbits()` parsing with multiple arguments

**What:**

New variants (extending existing system function block):
```rust
CountBits(Box<SvaExpr>, Vec<BitValue>), // $countbits(sig, '0, '1, 'x, 'z)
IsUnbounded(String),                     // $isunbounded(param)
```

Verify existing translations work correctly:
- `OneHot(sig)` → `popcount(sig) == 1` via bit-blasting
- `OneHot0(sig)` → `popcount(sig) <= 1`
- `CountOnes(sig)` → `popcount(sig)` as integer

**RED tests (~12 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_countbits` | `$countbits(sig, '0, '1)` parses with control values | Multi-argument system function |
| `parse_isunbounded` | `$isunbounded(MAX)` parses correctly | Parameter check function |
| `bounded_onehot_4bit` | 4-bit signal: only `0001`, `0010`, `0100`, `1000` satisfy | Exhaustive for small width |
| `bounded_onehot0_includes_zero` | 4-bit: `0000` also satisfies $onehot0 | Zero is allowed for onehot0 |
| `z3_onehot_equiv_popcount_1` | `$onehot(sig)` equiv `popcount(sig) == 1` via Z3 | Semantic definition verified |
| `z3_onehot_not_onehot0` | `$onehot(sig)` NOT equiv `$onehot0(sig)` when all-zero | The difference is exactly the zero case |
| `z3_countones_correct` | `$countones(0b1010)` = 2 via Z3 | Arithmetic correctness |
| `existing_system_funcs_not_stubbed` | All existing system function bounded translations produce non-trivial BoundedExpr | Ensure no stubs remain |

---

#### Sprint A6: Multi-Clocking

**Why:** Real designs have multiple clock domains. FVEval NL2SVA-Human and AssertionBench designs use CDC patterns. The current pipeline assumes single `posedge clk`.

**Files:**
- `crates/logicaffeine_compile/src/codegen_sva/sva_model.rs` — ClockingEvent variant
- `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs` — clock-aware unrolling

**What:**

New `SvaExpr` variant:
```rust
ClockingEvent {
    edge: ClockEdge,  // Posedge/Negedge/Both
    clock: String,
    body: Box<SvaExpr>,
}
```

Bounded translation: Clock ticks modeled as uninterpreted booleans. At each bounded step, `clk1_tick@t` and `clk2_tick@t` are independent. Signals are sampled relative to their clock domain.

**RED tests (~10 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_explicit_clock` | `@(posedge clk2) ack` inside a property | Clock event parsing |
| `parse_clock_change_mid_sequence` | `@(posedge clk1) req \|=> @(posedge clk2) ack` | Mid-sequence clock switch |
| `bounded_two_clocks_independent` | Two clocks tick independently in Z3 | No implicit synchronization |
| `bounded_cdc_handshake` | req on clk1, ack on clk2 — correct sampling | Cross-domain temporal |

---

#### Sprint A7: Directive Layer + Scope & Substitution

**Why:** Real SVA files use `assume`, `cover`, `restrict`, property/sequence declarations, default clocking, and action blocks. If the parser chokes on `$error(...)` action blocks, we fail on commercial IP before even evaluating the assertion.

**Files:**
- `crates/logicaffeine_compile/src/codegen_sva/sva_model.rs` — directive types, declaration types
- New resolution pass module for substitution

**What:**

Directive types:
```rust
pub enum SvaDirective {
    Assert(SvaExpr),
    Assume(SvaExpr),
    Cover(SvaExpr),
    Restrict(SvaExpr),
}

pub struct SvaDeclaration {
    pub kind: DeclKind,  // Property or Sequence
    pub name: String,
    pub params: Vec<String>,
    pub body: SvaExpr,
}
```

Resolution pass: Before translation, expand all `Instance { name, args }` nodes by looking up the symbol table, cloning the declaration body, and substituting parameters. Apply default clocking and disable iff.

Z3 translation:
- `assume` → `solver.assert(bounded_expr)` for all timesteps (constrain)
- `cover` → check SAT, extract witness trace (reverse of assert)
- `restrict` → constrain initial state path

**RED tests (~20 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_assume_property` | `assume property (valid)` parses as Assume directive | Directive keyword |
| `parse_cover_property` | `cover property (s_eventually(done))` parses as Cover | Cover directive |
| `parse_action_block` | `assert property (p) $display("ok"); else $error("fail");` doesn't crash parser | Action blocks consumed without choking |
| `parse_property_declaration` | `property p(a,b); a \|-> b; endproperty` → symbol table entry | Declaration parsing |
| `parse_sequence_declaration` | `sequence s(a,b); a ##1 b; endsequence` → symbol table entry | Sequence variant |
| `resolve_instantiation` | `assert property (p(req, ack))` → resolves to `req \|-> ack` | Substitution works |
| `resolve_default_clocking` | Unclocked assert + `default clocking @(posedge clk)` → ClockingEvent wrapping | Default application |
| `resolve_default_disable` | Assert + `default disable iff (rst)` → DisableIff wrapping | Default application |
| `z3_assume_constrains` | Assume property limits Z3 search space | Assume as constraint |
| `z3_cover_finds_witness` | Cover property returns SAT with witness trace | Cover is reachability |
| `z3_cover_unreachable` | Cover on impossible property returns UNSAT | No witness exists |

---

#### Sprint A8: Liveness vs Safety

**Why:** Strong sequences and `s_eventually` require liveness-to-safety conversion for unbounded proofs. Bounded model checking handles weak forms naturally but strong forms need explicit treatment.

**Files:**
- `crates/logicaffeine_verify/src/liveness.rs` — liveness-to-safety conversion
- `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs` — strong/weak distinction

**What:**

Bounded: `weak(seq)` — if bound reached without completion, pass. `strong(seq)` — if bound reached without completion, fail.

Unbounded: Biere's liveness-to-safety encoding. Introduce auxiliary fairness monitor that tracks whether the liveness condition has been satisfied. Convert `F(p)` to a safety property using the monitor.

**Existing (verified):**
- Liveness-to-safety reduction ALREADY IMPLEMENTED (256 lines, `liveness.rs`)
- `check_liveness()` — main entry with bounded search
- Fairness support, lasso extraction via `find_loop_point()`
- 6+ passing tests in `phase_hw_liveness.rs`
- **Gap:** Loop detection is heuristic (defaults to midpoint). No sophisticated fairness.

**RED tests (~12 tests) — extend `phase_hw_liveness.rs`:**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `bounded_weak_passes_at_bound` | Weak sequence, bound reached, no violation → pass | Weak semantics |
| `bounded_strong_fails_at_bound` | Strong sequence, bound reached, condition not met → fail | Strong semantics |
| `parse_strong_sequence` | `strong(a ##[1:$] b)` — strong modifier parsed | Keyword recognition |
| `parse_weak_sequence` | `weak(a ##[1:$] b)` — weak modifier parsed | Keyword recognition |
| `s_eventually_bounded_must_find` | `s_eventually(ack)` — must find ack within bound | Strong eventually |
| `eventually_bounded_may_not_find` | `eventually(ack)` — may not find ack, still passes | Weak eventually |
| `liveness_to_safety_sva` | SVA `s_eventually(done)` converted to safety + proved via k-ind | End-to-end unbounded liveness |
| `liveness_fveval_case` | Real FVEval case with `s_eventually` verified | Benchmark integration |

---

#### Sprint A9: Deferred Assertions & Complex Types

**Why:** Commercial IP uses `assert #0`, `assert final`, enum types, and struct fields in assertions. Parser must handle these to not fail on real-world files.

**Files:**
- `crates/logicaffeine_compile/src/codegen_sva/sva_model.rs` — deferred assertion variants, type info

**What:**

Deferred assertions use current (not sampled) values. Parser recognizes `assert #0 (expr)` and `assert final (expr)`. Translation bypasses `$sampled` logic.

Enum types: lower to bitvector encoding. `state == ST_READ` → `bv_eq(state, encode(ST_READ))`.

Struct fields: flatten to bitvector concatenation. `req.addr` → `bv_extract(req, addr_high, addr_low)`.

**RED tests (~10 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_deferred_assert` | `assert #0 (a == b)` parses | Deferred syntax |
| `parse_assert_final` | `assert final (a == b)` parses | Final deferred syntax |
| `deferred_uses_current_values` | Deferred assertion uses combinational values, not sampled | Bypass `$sampled` |
| `parse_enum_comparison` | `state === ST_IDLE` — enum name recognized | Enum in expression |
| `parse_struct_field` | `req.addr == 32'hFF` — field access | Dot notation |

---

### Phase B: LOGOS Parser Extension for Hardware NL

**Existing temporal infrastructure (verified in codebase):**

The LOGOS AST already has at `ast/logic.rs:165-190`:
- `TemporalOperator::Always` (G), `Eventually` (F), `Next` (X), `Past` (P), `Future` (F)
- `BinaryTemporalOp::Until`, `Release`, `WeakUntil`

The parser at `clause.rs:162-218` already parses "Always", "Eventually", "Next", "Never" → `TemporalOperator`. Until/Release/WeakUntil at `clause.rs:1093-1107`.

Kripke lowering at `semantics/kripke.rs:157-196`:
- `Always` → `∀w'(Accessible_Temporal(w,w') → P(w'))`
- `Eventually` → `∃w'(Reachable_Temporal(w,w') ∧ P(w'))`
- `Next` → `∀w'(Next_Temporal(w,w') → P(w'))`
- KripkeContext tracks `world_counter` and `clock_counter`

Parser architecture at `parser/mod.rs:236-301`: Recursive descent with 7 submodules (clause, noun, verb, modal, quantifier, question, pragmatics). ParserGuard with checkpoint/rollback at lines 147-207.

Compilation pipeline at `compile.rs:116-169`: Lexer → MWE → Discovery → Parser → Axioms → Kripke → Pragmatics → Format.

**What needs to be ADDED (not what exists):**
1. `## Specification` block type + lexer mode
2. Hardware-specific lexicon entries (high/low, signal, cycle, bitwise, reduction)
3. `Delay(n)` and `DelayRange(min,max)` temporal operators (NOT in current AST)
4. Hardware expression recursive descent (reduction ops, bitwise ops, edge detection)
5. Signal name recognition (identifiers as signals, not lexicon words)
6. FOL bitvector primitives (ReductionAnd, BitwiseXor, etc.)

---

#### Sprint B0: `## Specification` Block Marker

**Why:** The LOGOS parser needs to know when it's parsing hardware specification language vs general English. The `## Specification` block marker activates hardware vocabulary, signal name recognition, and temporal bound parsing.

**Existing infrastructure (verified):**
- `BlockType` enum at `token.rs:83-112` already has `Hardware` (line 106) and `Property` (line 108)
- Block classification at `lexer.rs:1841-1869` matches string → BlockType
- Mode switching at `lexer.rs:1863-1866`: `Main|Function` → Imperative, others → Declarative
- Parser entry at `parser/mod.rs:1150-1251` dispatches on block type

**Files:**
- `crates/logicaffeine_language/src/token.rs:83-112` — add `Specification` to BlockType enum
- `crates/logicaffeine_language/src/lexer.rs:1844` — add `"specification" => BlockType::Specification`
- `crates/logicaffeine_language/src/lexer.rs:1863-1866` — set mode for Specification blocks
- `crates/logicaffeine_language/src/parser/mod.rs:1189+` — add handler for BlockType::Specification

**What:**

When the parser encounters `## Specification`:
1. Lexer classifies as `BlockType::Specification` (new variant at `token.rs`)
2. Lexer sets mode — could be a new `LexerMode::Specification` or reuse `Declarative` with a flag
3. In hardware mode, identifiers matching `[a-zA-Z_][a-zA-Z0-9_]*` are recognized as signal references (not just lexicon words). This handles `sig_A`, `data_out`, `AWVALID`, etc.
4. Parser dispatches to hardware specification parsing (recursive descent on hardware NL)
5. Deactivate hardware mode at next `##` header or EOF

**RED tests (~10 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_spec_block_header` | `## Specification` recognized as block type | Block marker activation |
| `hardware_mode_activates` | Lexer enters hardware mode after `## Specification` | Mode switching |
| `hardware_mode_deactivates` | Mode deactivates at next `##` header | Scope is bounded |
| `signal_names_recognized` | `sig_A`, `data_out`, `AWVALID` parsed as signals | Identifier recognition in HW mode |
| `non_hw_block_unchanged` | `## Main` block parsing unaffected by HW extensions | Zero regression |
| `spec_and_main_coexist` | File with both `## Specification` and `## Main` parses correctly | Block coexistence |

---

#### Sprint B1: Hardware Lexicon Entries

**Why:** The parser needs to understand hardware-specific vocabulary: "high" (logic 1), "low" (logic 0), "clock" (temporal reference), "bitwise" (operator modifier), reduction operator phrases, edge detection phrases.

**Files:**
- `assets/lexicon.json` — new entries gated by hardware domain
- `crates/logicaffeine_language/src/lexer.rs` — hardware keyword handling

**What:**

New lexicon entries (domain: "hardware"):
- Adjectives: `high`, `low`, `valid`, `active`, `stable`, `unchanged`
- Nouns: `signal`, `clock`, `cycle`, `bit`, `edge`, `transition`
- Verbs: `hold`, `rise`, `fall`, `change`, `toggle`, `assert`
- Modifiers: `bitwise`, `reduction`, `identity`
- Operator phrases: `all bits of`, `at least one bit of`, `odd number of 1s in`
- Temporal: `eventually`, `always`, `never`, `afterwards`, `later`, `within`

**RED tests (~15 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `lex_high_as_adjective` | "high" → hardware adjective token | Not "tall" or "important" |
| `lex_signal_name` | "sig_A" → signal identifier token | Underscore-containing identifiers |
| `lex_clock_cycle` | "clock cycle" → temporal unit token | Two-word unit |
| `lex_bitwise` | "bitwise XOR" → operator modifier + operator | Compound operator phrase |
| `lex_all_bits_of` | "all bits of" → reduction AND phrase | Multi-word operator |
| `lex_at_least_one` | "at least one '1' bit" → reduction OR phrase | Multi-word with quotes |
| `lex_transition` | "transition from high to low" → falling edge phrase | Edge detection |
| `lex_non_hw_unchanged` | "Every dog runs" in `## Main` parses as before | No regression |

---

#### Sprint B2: Temporal Bound Parsing

**Why:** FVEval NL uses "N clock cycles later", "within 1 to 3 cycles", "on the next cycle", "afterwards". These must parse into FOL temporal constructs: `Delay(n)` and `DelayRange(min, max)`.

**Files:**
- `crates/logicaffeine_language/src/ast.rs` — add Delay and DelayRange to temporal AST
- `crates/logicaffeine_language/src/parser/temporal.rs` or equivalent — parse temporal bounds

**What:**

New AST nodes:
```rust
TemporalOperator::Delay(u32),           // "3 clock cycles later" → Delay(3)
TemporalOperator::DelayRange(u32, u32), // "within 1 to 3 cycles" → DelayRange(1, 3)
```

NL patterns to handle:
- "N clock cycles later" / "N cycles later" / "after N cycles" → Delay(N)
- "within N to M clock cycles" / "between N and M cycles" → DelayRange(N, M)
- "on the next cycle" / "one cycle later" → Delay(1)
- "afterwards" / "subsequently" → Non-overlapping (|=>)

**RED tests (~12 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_n_cycles_later` | "3 clock cycles later" → Delay(3) | Numeric extraction |
| `parse_within_range` | "within 1 to 3 cycles" → DelayRange(1, 3) | Range extraction |
| `parse_next_cycle` | "on the next cycle" → Delay(1) | Synonym handling |
| `parse_afterwards` | "afterwards" → non-overlapping implication | Keyword |
| `parse_after_n_cycles` | "after 5 cycles" → Delay(5) | Alternative phrasing |
| `parse_between_n_and_m` | "between 2 and 7 clock cycles" → DelayRange(2, 7) | Alternative range phrasing |
| `delay_to_fol` | Delay(3) → FOL with 3 nested Next operators | FOL encoding |
| `delay_range_to_fol` | DelayRange(1, 3) → FOL disjunction over offsets | FOL encoding |
| `delay_to_sva` | FOL delay → `##3` in SVA output | Full pipeline |
| `delay_range_to_sva` | FOL delay range → `##[1:3]` in SVA output | Full pipeline |

---

#### Sprint B3: Reduction and Bitwise NL Patterns

**Why:** FVEval NL uses phrases like "all bits of sig_I are high" (→ `&sig_I`), "bitwise XOR of sig_A and sig_B" (→ `sig_A ^ sig_B`), "bitwise negation" (→ `~sig`). These must parse into FOL hardware primitives.

**Files:**
- `crates/logicaffeine_language/src/parser/` — expression parsing for hardware operators
- `crates/logicaffeine_language/src/ast.rs` — FOL bitvector primitives

**What:**

New FOL constructs:
```rust
Expr::ReductionAnd(Box<Expr>),
Expr::ReductionOr(Box<Expr>),
Expr::ReductionXor(Box<Expr>),
Expr::BitwiseXor(Box<Expr>, Box<Expr>),
Expr::BitwiseNot(Box<Expr>),
Expr::BitwiseNor(Box<Expr>),
// ... etc for all reduction/bitwise variants
```

NL patterns:
- "all bits of X are high" / "every bit of X is 1" → `ReductionAnd(X)`
- "at least one bit of X" / "any bit of X is 1" → `ReductionOr(X)`
- "odd number of 1s in X" → `ReductionXor(X)`
- "bitwise XOR of X and Y" → `BitwiseXor(X, Y)`
- "bitwise negation of X" / "complement of X" → `BitwiseNot(X)`
- "NOR of X" / "bitwise NOR of X" → `ReductionNor(X)`

**RED tests (~15 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_all_bits_high` | "all bits of sig_I are high" → ReductionAnd | Multi-word phrase → unary op |
| `parse_at_least_one_bit` | "at least one '1' bit of sig_J" → ReductionOr | Reduction OR phrasing |
| `parse_odd_number_of_ones` | "sig_G has an odd number of 1s" → ReductionXor | Parity = XOR reduction |
| `parse_bitwise_xor` | "bitwise XOR of sig_A and sig_B" → BitwiseXor(A, B) | Binary operator phrase |
| `parse_bitwise_negation` | "bitwise negation of sig_C" → BitwiseNot(C) | Unary operator phrase |
| `parse_reduction_nor` | "NOR of sig_D" → ReductionNor(D) | Short form |
| `fol_reduction_and_to_sva` | ReductionAnd FOL → `&sig` SVA | Full pipeline |
| `fol_bitwise_xor_to_sva` | BitwiseXor FOL → `a ^ b` SVA | Full pipeline |
| `fveval_all_bits_pattern` | FVEval case with "all bits" → correct SVA | Real benchmark case |

---

#### Sprint B4: Edge Detection NL Patterns

**Why:** FVEval NL uses "change in X" (→ `$changed`), "transition from high to low" (→ `$fell`), "rising edge" (→ `$rose`), "remains stable" (→ `$stable`). These must parse into FOL edge detection constructs.

**Files:**
- `crates/logicaffeine_language/src/parser/` — edge detection phrase parsing
- `crates/logicaffeine_language/src/ast.rs` — FOL edge primitives

**What:**

New FOL constructs:
```rust
Expr::Rose(Box<Expr>),     // rising edge
Expr::Fell(Box<Expr>),     // falling edge
Expr::Changed(Box<Expr>),  // any change
Expr::Stable(Box<Expr>),   // no change
```

NL patterns:
- "change in X" / "X changes" / "whenever X changes" → `Changed(X)`
- "transition from high to low" / "falls" / "goes low" → `Fell(X)`
- "transition from low to high" / "rises" / "goes high" → `Rose(X)`
- "remains stable" / "stays unchanged" / "X is stable" → `Stable(X)`
- "previous value of X" → `Past(X, 1)`

**RED tests (~12 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_change_in` | "change in sig_D" → Changed(sig_D) | Keyword phrase |
| `parse_transition_high_to_low` | "transition from high to low" → Fell | Directional transition |
| `parse_transition_low_to_high` | "transition from low to high" → Rose | Opposite direction |
| `parse_rises` | "sig_A rises" → Rose(sig_A) | Verb form |
| `parse_falls` | "sig_A falls" → Fell(sig_A) | Verb form |
| `parse_remains_stable` | "sig_B remains stable" → Stable(sig_B) | Stability phrase |
| `fol_changed_to_sva` | Changed FOL → `$changed(sig)` SVA | Full pipeline |
| `fol_fell_to_sva` | Fell FOL → `$fell(sig)` SVA | Full pipeline |
| `fol_rose_to_sva` | Rose FOL → `$rose(sig)` SVA | Full pipeline |
| `fveval_change_pattern` | FVEval case with "change in" → correct SVA | Real benchmark case |

---

#### Sprint B5: Recursive Expression Descent

**Why:** FVEval NL describes deeply nested expression trees in English: "the bitwise XOR of sig_F and the reduction XOR of sig_A equals sig_H". This is an S-expression tree described in natural language. The parser must handle recursive nesting.

**Files:**
- `crates/logicaffeine_language/src/parser/` — expression-level NL recursive descent

**What:**

Operator precedence for NL expressions (lowest to highest):
1. Implication: "if...then", "implies"
2. Disjunction: "or", "either...or"
3. Conjunction: "and", "both...and"
4. Comparison: "equals", "is equal to", "differs from", "is less than", "is not equal to"
5. XOR/bitwise binary: "XOR of...and...", "bitwise XOR of"
6. Negation/reduction: "not", "all bits of", "bitwise negation of"
7. Atom: signal reference, constant, parenthesized subexpression

Key recursive pattern: "the X of A and B" where X is an operator and A/B may themselves be operator phrases. "the bitwise XOR of (sig_F) and (the reduction XOR of sig_A)" parses as `Xor(sig_F, ReductionXor(sig_A))`.

**RED tests (~15 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_nested_xor_reduction` | "bitwise XOR of sig_F and the reduction XOR of sig_A" → Xor(F, RedXor(A)) | Recursive nesting |
| `parse_deeply_nested` | "the NOR of the XOR of sig_A and sig_B" → ReductionNor(Xor(A, B)) | Depth 2 nesting |
| `parse_comparison_of_operators` | "the XOR of A and B equals C" → Eq(Xor(A,B), C) | Comparison at top level |
| `parse_negation_of_comparison` | "sig_C not being equal to the bitwise NOR of sig_D" → NotEq(C, RedNor(D)) | Negated comparison with nested operator |
| `parse_full_fveval_3_1_0` | FVEval case 3_1_0 complete NL → correct FOL | Real benchmark: the gnarliest pattern |
| `parse_full_fveval_3_8_0` | FVEval case 3_8_0 complete NL → correct FOL | Multiple nested operators |
| `parse_checked_for_equality` | "X, checked for equality with Y" → Eq(X, Y) | FVEval-specific phrasing |
| `parse_differs_from` | "X differs from Y" → NotEq(X, Y) | Alternative inequality phrasing |
| `parse_mixed_and_or` | "A and B, or C" — correct precedence | AND before OR |
| `operator_precedence_correct` | Complex expression with all precedence levels | Full precedence chain |

---

#### Sprint B6: Implication Structure Detection

**Why:** FVEval NL uses "If A, then B N cycles later", "Whenever A, B must hold", "If A is high, then B" as the top-level sentence structure. The parser must detect the implication structure and extract antecedent, consequent, and temporal modifier.

**Files:**
- `crates/logicaffeine_language/src/parser/` — sentence-level structure detection

**What:**

Sentence patterns:
- "If A, then B" → Implication(A, B, overlapping)
- "If A, then B N cycles later" → Implication(A, Delay(N, B), overlapping)
- "If A, then within N to M cycles, B" → Implication(A, DelayRange(N, M, B), overlapping)
- "If A, then afterwards B" / "then B on the next cycle" → Implication(A, B, non-overlapping)
- "Whenever A, B" → Implication(A, B, overlapping) (universally quantified)
- "It is never the case that A" → Not(A)
- "A holds at each rising clock edge" → Assert(A) (invariant)
- "A must eventually become B" → Implication(A, SEventually(B))

**RED tests (~15 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `detect_if_then` | "If sig_A is high, then sig_B is high" → Implication | Basic implication |
| `detect_if_then_delay` | "If sig_A is high, then sig_B must be high 3 cycles later" → Impl + Delay(3) | Temporal delay |
| `detect_if_then_range` | "If sig_A is high, then within 1 to 3 cycles, sig_B is high" → Impl + DelayRange | Temporal range |
| `detect_if_then_afterwards` | "If sig_A, then afterwards sig_B" → Impl(non-overlapping) | Non-overlapping |
| `detect_whenever` | "Whenever sig_A changes, sig_B must hold" → Impl | Whenever keyword |
| `detect_never` | "It is never the case that sig_A and sig_B" → Not(And) | Negation pattern |
| `detect_invariant` | "sig_A is high at each rising clock edge" → Assert(sig_A) | No implication, just invariant |
| `detect_eventually` | "sig_A must eventually become false" → SEventually(Not(A)) | Eventual modality |
| `full_pipeline_if_delay` | "If sig_A is high, then sig_B 3 cycles later" → `sig_A \|-> ##3 sig_B` SVA | End-to-end |
| `full_pipeline_invariant` | "sig_A or sig_B" → `assert property(@(posedge clk) (sig_A \|\| sig_B))` | End-to-end invariant |

---

#### Sprint B7: FVEval NL2SVA-Machine Full Coverage

**Why:** This is the integration sprint. Run all 300 FVEval NL2SVA-Machine cases through the complete pipeline. Identify remaining gaps. Target: 300/300 parse successfully, 280+/300 produce correct SVA (rest flagged as ambiguous).

**Files:**
- All Phase B files — integration testing
- New test file: `phase_hw_fveval_machine.rs`

**RED tests (~20 tests + integration):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `fveval_parse_all_300` | All 300 NL prompts parse without error (or flag as ambiguous) | No crashes on real data |
| `fveval_emit_all_parsed` | All successfully-parsed cases emit valid SVA syntax | SVA output is syntactically correct |
| `fveval_roundtrip_reference` | All 300 reference SVAs parse through our SVA parser | Our parser handles the gold standard |
| `fveval_z3_equiv_simple_cases` | First 50 cases: generated SVA Z3-equivalent to reference | Semantic correctness on easy cases |
| `fveval_z3_equiv_medium_cases` | Cases 50-150: generated SVA Z3-equivalent to reference | Semantic correctness on medium cases |
| `fveval_z3_equiv_hard_cases` | Cases 150-300: generated SVA Z3-equivalent to reference | Semantic correctness on hard cases |
| `fveval_ambiguous_flagged_not_wrong` | Cases flagged as ambiguous are genuinely unparseable, not just hard | Ambiguity detection is honest |
| `fveval_zero_wrong_answers` | No case produces incorrect SVA (wrong = not-equivalent AND not-flagged) | The 100%-or-flagged guarantee |
| `fveval_accuracy_report` | Print accuracy metrics: correct/total, flagged/total, wrong/total | Reporting |

---

### Phase C: FOL Hardware Primitives

---

#### Sprint C0: Bitvector Types in FOL

**Why:** Full bitvector theory needed for: reduction operators, bitwise operations, width-aware comparisons, signal width tracking through the pipeline.

**Files:**
- `crates/logicaffeine_language/src/ast.rs` — BV(N) type, BV operations
- `crates/logicaffeine_compile/src/codegen_sva/fol_to_sva.rs` — BV FOL → SVA synthesis

**What:**

FOL type system extension:
```rust
FolType::BitVec(u32),  // BV(N)
FolType::Int,
FolType::Bool,
FolType::Array(Box<FolType>, Box<FolType>),
```

FOL expression extensions:
```rust
Expr::BvConst(u64, u32),                    // constant with width
Expr::BvBinOp(BvOp, Box<Expr>, Box<Expr>),  // and, or, xor, add, sub, ...
Expr::BvUnaryOp(BvUnaryOp, Box<Expr>),      // not, neg
Expr::BvExtract(u32, u32, Box<Expr>),        // bit extraction
Expr::BvConcat(Box<Expr>, Box<Expr>),        // concatenation
```

**RED tests (~15 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `fol_bv_type` | `BV(8)` type in FOL AST | Type representation |
| `fol_bv_const` | `8'hFF` in FOL | Constant with width |
| `fol_bv_and` | Bitvector AND in FOL | Binary BV operation |
| `fol_bv_extract` | Bit extraction in FOL | Subfield access |
| `fol_bv_to_sva` | BV FOL → SVA with width-correct constants | Pipeline |
| `fol_bv_to_z3` | BV FOL → Z3 bitvector theory | Verification |

---

### Phase D: Verification Hardening

---

#### Sprint D0: SVA-to-SVA Z3 Equivalence

**Why:** The benchmark harness needs to check: "is our generated SVA semantically equivalent to the reference SVA?" This requires a function that takes two SVA strings and returns equivalent/not-equivalent/counterexample.

**Files:**
- `crates/logicaffeine_compile/src/codegen_sva/hw_pipeline.rs` — new public function

**What:**

```rust
pub fn check_sva_sva_z3_equivalence(
    sva_a: &str,
    sva_b: &str,
    bound: u32,
) -> Result<EquivalenceResult, HwError>
```

Built from existing infrastructure: `parse_sva()` → `translate_sva_for_equiv()` → `bounded_to_verify()` → `check_equivalence()`.

**RED tests (~10 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `sva_sva_equiv_identical` | Same SVA string → Equivalent | Identity |
| `sva_sva_equiv_commutative` | `a && b` equiv `b && a` | Semantic equivalence beyond syntax |
| `sva_sva_neq_direction` | `a \|-> b` not-equiv `b \|-> a` | Direction matters |
| `sva_sva_neq_delay` | `a \|-> ##1 b` not-equiv `a \|-> ##2 b` | Delay matters |
| `sva_sva_counterexample` | Non-equivalent → counterexample trace | Diagnostic output |
| `sva_sva_demorgan` | `!(a && b)` equiv `(!a \|\| !b)` | Boolean law |
| `sva_sva_reduction_equiv` | `&sig` equiv `sig === {N{1'b1}}` | Reduction semantics |

---

#### Sprint D1: k-Induction Hardening

**Why:** k-Induction is ALREADY FULLY IMPLEMENTED (437 lines, `kinduction.rs`) with base case + inductive step, 9+ passing tests. Hardening means: better trace extraction, property strengthening/lemma generation, handling FVEval-scale SVA properties (which are more complex than current tests), and timeout tuning.

**Existing (verified):**
- `k_induction()` — main entry with full loop k=1..max_k
- `check_base_case()` / `check_inductive_step()` — both complete
- `instantiate_at()` / `rename_timestep()` — timestep variable renaming
- `encode_to_bool()` / `encode_expr_bool()` / `encode_expr_int()` — full Z3 encoding
- Result: `Proven{k}`, `Counterexample{k, trace}`, `InductionFailed{k, trace}`, `Unknown`
- **Gap:** Trace extraction returns empty Trace in counterexamples. No lemma generation.

**Files:**
- `crates/logicaffeine_verify/src/kinduction.rs` — improve trace extraction, add lemma generation

**RED tests (~12 tests) — extend `phase_hw_kinduction.rs`:**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `kind_sva_implication_delay` | SVA `a \|-> ##3 b` encoded as k-ind property — k=4 needed | FVEval-scale temporal |
| `kind_sva_eventually` | `s_eventually(ack)` via k-induction | Liveness-adjacent |
| `kind_counterexample_has_values` | Failed property → counterexample with actual signal values | Trace extraction works |
| `kind_timeout_handling` | Complex formula → timeout → Unknown result (not hang) | Robustness |
| `kind_matches_bounded` | k-ind result consistent with bounded Z3 check on same property | Cross-validation |
| `kind_fveval_case` | Real FVEval reference SVA verified via k-induction | Benchmark integration |

---

#### Sprint D2: IC3 Hardening

**Why:** IC3 is ALREADY FULLY IMPLEMENTED (554 lines, `ic3.rs`) with frame sequences, CTI blocking, clause generalization, propagation, and convergence checking. 8+ passing tests. Hardening means: better clause generalization (currently basic literal-dropping), subsumption checking between clauses, and handling FVEval-scale properties.

**Existing (verified):**
- `ic3()` — main entry with Phase 0 (init check), Phase 1 (BMC), Phase 2 (IC3 proper)
- `is_reachable_from_init()` — recursive reachability
- `generalize_blocking_clause()` — literal dropping (basic)
- `propagate_clauses()` / `check_convergence()` — frame strengthening
- `extract_trace_from_bmc()` — counterexample with signal values
- Falls back to k-induction if frames exhaust
- **Gap:** No clause subsumption. Basic generalization only.

**Files:**
- `crates/logicaffeine_verify/src/ic3.rs` — add subsumption checking, improve generalization

**RED tests (~10 tests) — extend `phase_hw_ic3.rs`:**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `ic3_sva_implication` | SVA `req \|-> ack` verified via IC3 unbounded | FVEval-scale |
| `ic3_counterexample_has_values` | Failed property → trace with actual signal values | Diagnostic quality |
| `ic3_timeout_graceful` | Complex formula → timeout → Unknown (not hang) | Robustness |
| `ic3_matches_kind` | IC3 agrees with k-induction on same property | Cross-validation |
| `ic3_fveval_case` | Real FVEval reference SVA verified via IC3 | Benchmark integration |
| `ic3_subsumption_reduces_frames` | Subsumed clauses eliminated → faster convergence | Optimization works |

---

### Phase E: RTL Behavioral Parser (VERT + AssertionBench)

---

#### Sprint E0: If/Else/Case Block Parsing

**Why:** VERT's 20,000 test cases are if/else/case blocks → assertions. This is the core pattern.

**Files:**
- New: `crates/logicaffeine_compile/src/codegen_sva/rtl_behavioral.rs`
- Register in `crates/logicaffeine_compile/src/codegen_sva/mod.rs`

**What:**

```rust
pub struct BehavioralBlock {
    pub conditions: Vec<RtlCondition>,    // accumulated if/else conditions
    pub assignments: Vec<RtlAssignment>,  // LHS = RHS
}

pub fn parse_behavioral(code: &str) -> Result<Vec<BehavioralBlock>, RtlParseError>
pub fn behavioral_to_sva(blocks: &[BehavioralBlock], sync: bool, clock: Option<&str>) -> Vec<String>
```

Patterns:
- `if (cond) begin LHS = RHS; end` → `cond |=> LHS == RHS`
- Nested: `if (A) begin if (B) begin ... end end` → `A && B |=> ...`
- Else: `if (A) ... else ...` → `A |=> ...` + `!A |=> ...`
- Case: `case(sel) 2'b00: ... 2'b01: ... endcase` → per-case implications

**RED tests (~20 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_simple_if` | `if (a) begin x = y; end` → 1 block, condition=a, assign x=y | Base case |
| `parse_if_else` | if/else → 2 blocks, second has negated condition | Else handling |
| `parse_nested_if` | Nested if → accumulated conditions with AND | Condition accumulation |
| `parse_if_else_if` | if/else if/else chain → correct condition negation chain | Chained else-if |
| `parse_case` | case block → one block per case arm | Case statement |
| `parse_multiple_assignments` | Multiple assignments in one block → multiple assertions | Multi-assign |
| `parse_nonblocking` | `<=` assignment → same behavior | Non-blocking |
| `sva_simple_implication` | Simple if → `cond \|=> LHS == RHS` | SVA generation |
| `sva_nested_conditions` | Nested if → `(A && B) \|=> ...` | Compound conditions |
| `sva_sync_with_clock` | Synchronous code → `@(posedge clk)` wrapper | Clock handling |
| `sva_async_no_clock` | Asynchronous code → no clock wrapper | Async |
| `vert_entry_0` | First VERT entry → correct assertion | Real data |
| `vert_entry_1` | Second VERT entry → correct assertion | Real data |
| `vert_entry_complex` | Complex nested VERT entry → correct assertions | Multi-level nesting |

---

#### Sprint E1: Full RTL Behavioral Analysis (AssertionBench)

**Why:** AssertionBench's 101 designs require parsing full Verilog modules, not just code snippets. Always blocks, continuous assignments, state machines.

**Files:**
- Extend `crates/logicaffeine_compile/src/codegen_sva/rtl_behavioral.rs`
- Extend `crates/logicaffeine_compile/src/codegen_sva/rtl_extract.rs`

**What:**

Full `always @(posedge clk)` block parsing:
- Detect sequential logic (clocked always blocks)
- Detect combinational logic (`always @(*)`)
- Parse `assign` continuous assignments
- State machine detection: enum patterns, case blocks on state variable
- Counter detection: `x <= x + 1` patterns
- Handshake detection: req/ack patterns

Assertion mining strategies:
1. **Condition→assignment implications** (from E0)
2. **Counter bounds**: x increments, never exceeds MAX
3. **State machine properties**: one-hot states, valid transitions
4. **Signal stability**: outputs stable when enable is low
5. **Mutual exclusion**: conflicting outputs never both asserted

**RED tests (~20 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `parse_always_posedge` | `always @(posedge clk)` block → sequential behavioral blocks | Clocked parsing |
| `parse_always_star` | `always @(*)` → combinational blocks | Combinational |
| `parse_assign` | `assign out = a & b;` → continuous assignment block | Continuous |
| `detect_fsm_pattern` | case block on state variable → FSM detected | State machine |
| `detect_counter` | `count <= count + 1` → counter pattern | Counter |
| `mine_counter_bound` | Counter with MAX → `assert count <= MAX` | Assertion mining |
| `mine_onehot_state` | One-hot state machine → `$onehot(state)` assertion | State property |
| `mine_handshake` | req/ack pattern → handshake assertions | Protocol mining |
| `assertionbench_design_0` | First AssertionBench design → generates assertions | Real data |
| `assertionbench_bitNegator` | Simple design (50 assertions from HARM) → our assertions overlap | Coverage check |

---

### Phase F: Benchmark Harness

---

#### Sprint F0: sva-bench Crate

**Why:** The benchmark infrastructure must be isolated from product code. Separate crate keeps the benchmark runner, data loaders, and reporters clean.

**Files:**
- New crate: `crates/sva_bench/`
- `crates/sva_bench/Cargo.toml`
- `crates/sva_bench/src/lib.rs`
- `crates/sva_bench/src/fveval.rs` — FVEval NL2SVA runner
- `crates/sva_bench/src/vert.rs` — VERT runner
- `crates/sva_bench/src/assertionbench.rs` — AssertionBench runner
- `crates/sva_bench/src/report.rs` — results reporting

**What:**

```rust
// FVEval runner
pub fn run_fveval_machine(csv_path: &str) -> FvEvalResults {
    // Load CSV
    // For each row: parse NL → FOL → SVA
    // Compare with reference via Z3
    // Collect metrics
}

pub struct FvEvalResults {
    pub total: usize,
    pub correct: usize,        // Z3-equivalent to reference
    pub flagged_ambiguous: usize,
    pub wrong: usize,          // should always be 0
    pub per_case: Vec<CaseResult>,
}

pub struct CaseResult {
    pub task_id: String,
    pub generated_sva: Option<String>,
    pub reference_sva: String,
    pub status: CaseStatus, // Correct, Ambiguous, Wrong
    pub z3_result: Option<EquivalenceResult>,
    pub counterexample: Option<Trace>,
}
```

**RED tests (~10 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `load_fveval_csv` | CSV loading with correct column extraction | Data ingestion |
| `run_single_case` | Single FVEval case → CaseResult with correct status | Per-case pipeline |
| `run_batch_10` | 10 cases → aggregated FvEvalResults | Batch processing |
| `report_format` | Results render as comparison table | Reporting |
| `zero_wrong_guarantee` | wrong count is always 0 | The core guarantee |
| `load_vert_json` | VERT JSON Lines loading | VERT data ingestion |
| `load_assertionbench` | AssertionBench Verilog + assertion loading | AB data ingestion |
| `latency_measurement` | Wall-clock time recorded per case and aggregate | Our latency numbers |

---

#### Sprint F1: Ambiguity Diagnostic System

**Why:** When we can't parse a spec, we flag it as ambiguous with both a structured diagnostic (for the benchmark harness) and a Socratic explanation (for human consumption). This is our differentiator vs LLMs that hallucinate confidently.

**Files:**
- New: `crates/logicaffeine_compile/src/codegen_sva/ambiguity.rs`

**What:**

```rust
pub struct AmbiguityDiagnostic {
    pub parsed_fragments: Vec<ParsedFragment>,
    pub unparsed_text: String,
    pub suggestions: Vec<String>,
}

pub struct SocraticExplanation {
    pub message: String,      // "This specification is ambiguous because..."
    pub alternatives: Vec<String>, // "Did you mean: (a) ... or (b) ...?"
}

pub fn diagnose_ambiguity(input: &str, parse_error: &ParseError) -> (AmbiguityDiagnostic, SocraticExplanation)
```

**RED tests (~8 tests):**

| Test | What It Proves | Why Sufficient |
|---|---|---|
| `diagnostic_shows_parsed_fragments` | Partial parse → shows what WAS understood | Useful diagnostic |
| `diagnostic_shows_unparsed` | Shows exactly where parsing failed | Pinpoints the gap |
| `diagnostic_suggests_alternatives` | Suggests possible interpretations | Actionable feedback |
| `socratic_is_human_readable` | Explanation is clear English, not stack traces | User-facing quality |
| `ambiguous_not_wrong` | Ambiguous spec returns None for SVA, not incorrect SVA | Safety guarantee |

---

## Part V: Success Criteria

### The 100%-or-Flagged Guarantee

For every benchmark case:
1. LogicAffeine either produces SVA that is **Z3-verified equivalent** to the reference, OR
2. LogicAffeine **flags the specification as ambiguous** with a diagnostic explanation.
3. LogicAffeine **never produces incorrect SVA** (SVA that is not equivalent to the reference AND is not flagged).

This is the headline. This is what no LLM can do.

### Metric Targets

| Benchmark | Correct | Flagged Ambiguous | Wrong | Total |
|---|---|---|---|---|
| FVEval NL2SVA-Machine | 280+ | 20 or fewer | **0** | 300 |
| FVEval NL2SVA-Human | 60+ | 19 or fewer | **0** | 79 |
| VERT | 18,000+ | 2,000 or fewer | **0** | 20,000 |
| AssertionBench | Per-design coverage report | N/A | N/A | 101 designs |

### Proof Tiers

Every correct output includes a proof tier:

| Tier | Meaning | How |
|---|---|---|
| **Bounded** | Verified equivalent at bound K=10 | Z3 BMC |
| **k-Inductive** | Verified for all time via k-induction | k-induction tactic |
| **IC3** | Verified for all time via IC3 | IC3 tactic |
| **Ambiguous** | Cannot parse — flagged with diagnostic | Parser returns None + diagnostic |

Report: "X cases bounded-verified, Y cases k-inductive, Z cases IC3-proven, W cases flagged ambiguous. 0 wrong."

---

## Part VI: Sprint Execution Order

```
Phase A (SVA Model):
  A0 → A1 → A2 → A3 → A4 → A5 → A6 → A7 → A8 → A9

Phase B (LOGOS Parser):   [can start after A0]
  B0 → B1 → B2 → B3 → B4 → B5 → B6 → B7

Phase C (FOL Primitives): [can start after B1]
  C0

Phase D (Verification):  [can start after A0]
  D0 → D1 → D2

Phase E (RTL Parser):    [can start after A0]
  E0 → E1

Phase F (Harness):       [after B7 + D0 + E0]
  F0 → F1
```

**Critical path:** A0 → B0 → B1 → ... → B7 → F0 (FVEval NL2SVA results)

**Parallel tracks:**
- Phase A (SVA compliance) runs ahead of Phase B (NL parsing) since SVA model must support operators before NL can produce them
- Phase D (verification hardening) is independent of parsing
- Phase E (RTL parser) is independent of NL parsing

---

## Part VII: Future — Futamura Projection Bootstrap

Write a LOGOS interpreter for hardware NL, then P1-specialize it into a compiled SVA generator. The interpreter would define the NL→SVA mapping rules as data. Partial evaluation would produce a specialized, optimized compiler from the interpreter + rules.

This is a stunning demonstration of the Futamura projection system's power and a unique research contribution. Deferred to v2 after benchmark numbers are established.

---

## Total Test Count

| Phase | Sprint Count | New Tests |
|---|---|---|
| A: SVA Model | 10 sprints | ~165 |
| B: LOGOS Parser | 8 sprints | ~104 |
| C: FOL Primitives | 1 sprint | ~15 |
| D: Verification | 3 sprints | ~32 |
| E: RTL Parser | 2 sprints | ~40 |
| F: Harness | 2 sprints | ~18 |
| **Total** | **26 sprints** | **~374 new tests** |
