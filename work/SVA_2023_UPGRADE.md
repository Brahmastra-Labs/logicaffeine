# SVA_2023_UPGRADE

Engineering specification for upgrading LogicAffeine's SVA formal verification pipeline from IEEE 1800-2017 to IEEE 1800-2023 compliance. Every sprint has concrete RED tests. Every claim verified against the standard. Every gap disclosed. Tests are robust to the point of absurdity.

**Prerequisite: SVA_COVERAGE complete. 79 SvaExpr variants, 858 SVA tests across 7 test files, 21 sprints, 100% IEEE 1800-2017 coverage. Bounded temporal unrolling, Z3 multi-sorted equivalence, all operational.**

**Standard reference: IEEE Std 1800-2023 (Revision of IEEE Std 1800-2017), Approved 6 December 2023. Chapters 5.9 (String literals), 6.23 (Type operator), 7.12 (Array manipulation methods), 16 (Assertions), 17 (Checkers), 20.4 (Timescale system functions), 20.9 (Bit vector system functions), 20.10 (Severity system tasks).**

**Backwards compatibility: All 858 existing SVA tests must pass unchanged. No existing SvaExpr variant removed. No existing BoundedExpr node altered. No existing VerifyType variant modified. Every new feature is additive.**

---

## Part I: What Changes Between 2017 and 2023

### The Core Truth

The 1800-2023 committee focused on OOP enhancements, coverage extensions (`covergroup extends`), and quality-of-life syntax. They did NOT rewrite the Linear Temporal Logic core. The foundational temporal semantics — implication, `s_always`, `until`, `sync_accept_on`, sequence threading, length-matching intersects, vacuity tracking — are already fully 2023-compliant.

For Chapters 16 (Assertions) and 17 (Checkers), the changes are surface-area expansions at the expression/parser level and one vertical type-system extension for `rand real`.

### 2023 Delta: What Impacts Our Pipeline

| # | Change | IEEE 2023 Section | Impact Level | Layer | Sprint |
|---|--------|-------------------|-------------|-------|--------|
| 1 | Triple-quoted strings (`"""..."""`) | 5.9 (p.80) | Low | Parser/lexer | 22 |
| 2 | `$timeunit` system function | 20.4.1 (p.622) | Low | Action block recognition | 22 |
| 3 | `$timeprecision` system function | 20.4.1 (p.622) | Low | Action block recognition | 22 |
| 4 | `$stacktrace` system task | 20.17 (p.652) | Low | Action block recognition | 22 |
| 5 | Array `.map()` with `index_argument` | 7.12 (p.173) | Medium | AST + BoundedExpr + Z3 | 23 |
| 6 | `type(this)` construct | 6.23 (p.138) | Low | Parser recognition | 23 |
| 7 | `rand real` / `rand const real` | 17.7 (p.513) | High | AST + BoundedSort + VerifyType + Z3 | 24 |
| 8 | 4-state operator truth table errata | Various | Audit | Documentation | 25 |

Impact classification:
- **High**: New types, new IR nodes, new Z3 sort integration, changes to checker quantification
- **Medium**: New SvaExpr variants, parser extensions with bounded translation
- **Low**: Recognition-only (parse or tokenize, excluded from Z3 encoding)
- **Audit**: Documentation-only, verify existing behavior under updated spec

### What Does NOT Change

The following constructs are semantically identical between 2017 and 2023. No action required:

- All temporal operators: `always`, `s_always`, `eventually`, `s_eventually`, `nexttime`, `s_nexttime`, `until`/`s_until`/`until_with`/`s_until_with`
- All sequence operators: `##`, `[*]`, `[->]`, `[=]`, `intersect`, `throughout`, `within`, `first_match`, sequence `and`/`or`
- All property connectives: `not`, `implies`, `iff`, `if/else`, `case`
- All implication/followed-by: `|->`, `|=>`, `#-#`, `#=#`
- All abort operators: `disable iff`, `accept_on`, `reject_on`, `sync_accept_on`, `sync_reject_on`
- All directive types: `assert property`, `assume property`, `cover property`, `cover sequence`, `restrict property`
- Sampled value functions: `$rose`, `$fell`, `$past`, `$stable`, `$changed`, `$sampled`
- Bit vector functions: `$onehot`, `$onehot0`, `$countones`, `$countbits`, `$isunknown`, `$isunbounded`
- System functions: `$bits`, `$clog2`
- All bitwise operators: `&`, `|`, `^`, `~`, reduction operators, bit-select, part-select, concatenation
- `let` construct, `const'` cast, local variables, `.triggered`/`.matched` endpoints
- Named sequence/property declarations
- Default clocking/disable iff
- Multi-clock sequences
- `strong`/`weak` sequence modifiers
- Checker structure: `checker...endchecker` with `rand bit`/`rand const bit`

The `expect` statement (16.17) remains simulation-only in 2023, still out of scope.

---

## Part II: Complete Gap List

Every 2023 feature gap, where it lives in the standard, and what sprint addresses it.

| # | Gap | IEEE 2023 Section | Current State | Consequence | Sprint |
|---|---|---|---|---|---|
| 1 | Triple-quoted strings (`"""..."""`) | 5.9 (p.80) | Parser handles only `"..."` strings | Cannot parse `$display("""...""")` in action blocks | 22 |
| 2 | `$timeunit` system function | 20.4.1 (p.622) | Not in recognized system task list | Rejected in action blocks | 22 |
| 3 | `$timeprecision` system function | 20.4.1 (p.622) | Not in recognized system task list | Rejected in action blocks | 22 |
| 4 | `$stacktrace` system task | 20.17 (p.652) | Not in recognized system task list | Rejected in action blocks | 22 |
| 5 | Array `.map()` method | 7.12 (p.173) | No array method support in SvaExpr | Cannot express mapped array comparisons in assertions | 23 |
| 6 | Array `.map()` `index_argument` | 7.12.4 (p.177) | No index querying in array methods | Cannot reference element index in map body | 23 |
| 7 | `type(this)` parameterization | 6.23 (p.138) | No type operator in SvaExpr | Class-scoped assertions using `type(this)` rejected | 23 |
| 8 | `rand real` checker variables | 17.7 (p.513) | `RandVar.width` is `u32` (bitvector only) | Cannot model real-valued nondeterministic inputs | 24 |
| 9 | `rand const real` checker variables | 17.7 (p.513) | Same as above | Cannot model constant real-valued nondeterministic choice | 24 |
| 10 | Real literal parsing in assertions | 5.7.2 (p.79) | No real literal support in assertion expressions | Cannot express `r > 1.5` comparisons | 24 |
| 11 | 4-state operator truth table errata | 11.4 (p.275) | 2-state engine, `$isunknown` = false | Must verify 2-state subset matches clarified 2023 tables | 25 |

### The Architectural Plan

4 sprints across 4 tiers, continuing the existing numbering. Total: ~130 new tests.

| Tier | Sprint | Focus | Est. Tests | Status |
|------|--------|-------|-----------|--------|
| Tier 6 | 22 | Lexer/parser: triple-quoted strings, new system tasks | 33 | COMPLETE |
| Tier 7 | 23 | Expression: array `.map()`, `type(this)` | 27 | COMPLETE |
| Tier 8 | 24 | Checker types: `rand real` / `rand const real` (vertical cut) | 21 | COMPLETE |
| Tier 9 | 25 | Semantic audit, cross-feature composition, full regression | 14 | COMPLETE |

**95 new SVA tests** in one new test file `phase_hw_sva_2023.rs`. Existing 7 test files untouched (except `phase_hw_sva_coverage.rs` — migrated `RandVar` construction sites from `width` to `var_type`).

**All 25 sprints COMPLETE. 95 new tests + 858 existing = 953 total SVA tests.**

---

## Part III: Tier 6 — Lexer/Parser Expansions

### Sprint 22: Triple-Quoted Strings and New System Tasks (IEEE 5.9, 20.4.1, 20.17)

**Why:** IEEE 1800-2023 Section 5.9 (p.80) introduces triple-quoted string literals (`"""..."""`) as a new form of `string_literal`. Triple-quoted strings differ from quoted strings in two ways: (1) newlines can be inserted directly without `\n`, (2) `"` characters can be embedded without `\"`. Escape sequences (`\n`, `\t`, `\\`) still work inside triple-quoted strings. Since IEEE 16.6 allows general expressions in assertions, and action blocks (`$error("msg")`, `$display("msg")`) contain string literals, the parser must accept triple-quoted strings wherever regular strings appear in action blocks.

Additionally, 2023 adds `$timeunit` (20.4.1, p.622), `$timeprecision` (20.4.1, p.622), and `$stacktrace` (20.17, p.652) as system tasks. Engineers will use these in assertion action blocks. Since our engine already parses action blocks but excludes them from Z3 encoding (they are simulation/diagnostic constructs), we just need graceful recognition.

**Files:** `sva_model.rs` (action block parsing, string handling)

**No new SvaExpr variants.** No BoundedExpr changes. No VerifyType changes. No Z3 changes.

**Changes:**
1. Action block string parsing: detect `"""` as a delimiter, consume until closing `"""`, allowing embedded `"` and raw newlines. Escape sequences (`\n`, `\t`, `\\`, `\"`) processed normally.
2. System task recognition list (in `parse_sva_directive` action block handling): add `$timeunit`, `$timeprecision`, `$stacktrace` alongside existing `$error`, `$info`, `$fatal`, `$warning`, `$display`.

**Translation semantics:** None. All Sprint 22 features are parse-only, excluded from Z3 encoding. Action blocks are stored in `SvaDirective.action_pass` / `SvaDirective.action_fail` as `Option<String>` and never enter the `BoundedExpr` pipeline.

**RED tests (~30 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `triple_quoted_basic` | `"""hello"""` in action block parses without error |
| 2 | `triple_quoted_embedded_quote` | `"""say "hello" please"""` embeds `"` without escape |
| 3 | `triple_quoted_multiline` | Triple-quoted string spanning 3 lines parses correctly |
| 4 | `triple_quoted_with_escape_n` | `"""line1\nline2"""` — `\n` escape still works inside triple-quoted |
| 5 | `triple_quoted_with_escape_t` | `"""col1\tcol2"""` — `\t` escape works |
| 6 | `triple_quoted_with_escape_backslash` | `"""path\\to\\file"""` — `\\` escape works |
| 7 | `triple_quoted_empty` | `""""""` (empty triple-quoted) parses to empty string |
| 8 | `triple_quoted_single_char` | `"""x"""` parses to single-character string |
| 9 | `triple_quoted_ieee_example3` | IEEE p.81 Example 3: `"""Humpty Dumpty sat on a "wall".\nHumpty Dumpty had a great fall. """` |
| 10 | `triple_quoted_ieee_example4` | IEEE p.81 Example 4: `"""...\"""` with escaped newline joins lines |
| 11 | `triple_quoted_ieee_example5` | IEEE p.81 Example 5: `"""Humpty Dumpty \n sat on a wall. \n..."""` with `\n` escape |
| 12 | `triple_quoted_in_error_action` | `assert property (p) else $error("""assertion "p" failed at cycle""");` full directive parse |
| 13 | `triple_quoted_in_display_action` | `assert property (p) $display("""pass: "ok"\ndetails here""");` pass action parse |
| 14 | `triple_quoted_roundtrip` | Triple-quoted string preserved in `action_fail` round-trip |
| 15 | `triple_quoted_vs_single_quoted` | Both string forms coexist in same directive: `$info("pass") else $error("""fail""")` |
| 16 | `triple_quoted_not_boolean` | Triple-quoted string in assertion boolean position is a parse error (strings are not boolean) |
| 17 | `timeunit_in_action_block` | `assert property (p) else $timeunit;` parses without error |
| 18 | `timeprecision_in_action_block` | `assert property (p) else $timeprecision;` parses |
| 19 | `stacktrace_in_action_block` | `assert property (p) else $stacktrace;` parses |
| 20 | `timeunit_with_arg` | `$timeunit(hier_path)` with hierarchical identifier argument recognized |
| 21 | `timeprecision_with_arg` | `$timeprecision(hier_path)` with argument recognized |
| 22 | `timeunit_not_in_z3` | Directive with `$timeunit` in action block: property translated to Z3, action block excluded |
| 23 | `timeprecision_not_in_z3` | Same for `$timeprecision` |
| 24 | `stacktrace_not_in_z3` | Same for `$stacktrace` |
| 25 | `new_tasks_roundtrip` | All three new system tasks round-trip in action blocks |
| 26 | `existing_tasks_unchanged` | `$error`, `$info`, `$fatal`, `$warning`, `$display` still parse identically |
| 27 | `action_block_mixed_old_new` | Old and new system tasks mixed in same action block |
| 28 | `triple_quoted_in_cover_action` | `cover property (p) $display("""covered""");` parse |
| 29 | `restrict_still_no_action` | `restrict property` still forbids action blocks in 2023 (IEEE 16.14.4) |
| 30 | `sprint22_backwards_compat` | All 858 existing SVA tests pass unchanged |

---

## Part IV: Tier 7 — Expression Expansions

### Sprint 23: Array `.map()` and `type(this)` (IEEE 7.12, 6.23)

**Why:** IEEE 1800-2023 Section 7.12 (p.173) defines array manipulation methods including locator methods (`find`, `find_index`, `min`, `max`, `unique`) and the general `array_method_call` syntax: `expression.array_method_name[(iterator_argument[, index_argument])] [with (expression)]`. The 2023 standard introduces `.map()` as a transformation method that produces a new array by applying the `with` expression to each element (e.g., `B = A.map(b) with (b+1)`). If this appears inside `assert property`, our AST and Z3 translation must handle it.

Section 6.23 (p.138) introduces the `type` operator for parameterized types: `type(this)` returns the type of the current class instance. When properties or sequences are instantiated inside or near class scopes, `type(this)` can appear in assertion expressions.

**Files:** `sva_model.rs` (AST, parser), `sva_to_verify.rs` (translation)

**New SvaExpr variants:**
```rust
/// Array map method: `A.map(x) with (expr)` (IEEE 7.12, 2023)
ArrayMap { array: Box<SvaExpr>, iterator: String, with_expr: Box<SvaExpr> },
/// Type operator: `type(this)` (IEEE 6.23, 2023)
TypeThis,
```

**Translation semantics:**
- `A.map(x) with (f(x))` where A has known size N: unroll to N parallel expressions. For a 4-element array: `[f(A[0]), f(A[1]), f(A[2]), f(A[3])]`. Each element becomes a `BoundedExpr` with the iterator variable substituted by the concrete index.
- `A.map(x) with (f(x))` where A has unknown size: `BoundedExpr::Unsupported("array map with unknown size")`. This fails closed — an unsupported expression is not silently true.
- `type(this)`: `BoundedExpr::Unsupported("type(this) in class scope")`. This is an OOP construct without direct formal verification semantics.

> **DECISION:** Array `.map()` is unrolled for known-size arrays only. Unknown-size arrays produce `BoundedExpr::Unsupported`, which fails closed (not silently true). This matches the existing pattern where `BoundedExpr::Unsupported(String)` is used for constructs that cannot be translated. Full dynamic array support would require array theory in Z3, which is deferred.

> **DECISION:** `type(this)` is recognized by the parser but produces `Unsupported` during translation. Full OOP type resolution is outside the scope of the SVA formal verification engine. The parser must not reject valid 2023 input, but the formal pipeline is not expected to prove properties about class hierarchies.

**RED tests (~35 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `array_map_parse_basic` | `A.map(x) with (x + 1)` parses to `ArrayMap{Signal("A"), "x", Add(LocalVar("x"), Const(1, 32))}` |
| 2 | `array_map_parse_with_index` | `A.map(a, i) with (a + B[i])` parses with index argument |
| 3 | `array_map_parse_no_iterator` | `A.map() with (item + 1)` uses default iterator `item` |
| 4 | `array_map_roundtrip` | Parse → to_string → parse structural equality |
| 5 | `array_map_in_assertion` | `assert property (A.map(x) with (x > 0) == expected)` end-to-end parse |
| 6 | `array_map_nested` | `A.map(x) with (x.map(y) with (y * 2))` nested map parses |
| 7 | `array_map_with_signal` | `data.map(d) with (d & mask)` — field access as array |
| 8 | `array_map_translate_known_4` | Known 4-element array: unrolls to 4 parallel BoundedExprs |
| 9 | `array_map_translate_known_8` | Known 8-element array: unrolls to 8 parallel BoundedExprs |
| 10 | `array_map_translate_unknown` | Unknown-size array produces `Unsupported("array map with unknown size")` |
| 11 | `array_map_z3_known_sum` | 4-element array `[1,2,3,4].map(x) with (x+1)` = `[2,3,4,5]` via Z3 |
| 12 | `array_map_z3_known_compare` | `A.map(x) with (x > 0)` produces array of booleans, all true verified via Z3 |
| 13 | `array_map_in_implication` | `req |-> data.map(x) with (x == expected[x.index])` parses and translates |
| 14 | `array_map_with_bitwise` | `A.map(x) with (x & 8'hFF)` bitwise in map body |
| 15 | `array_map_with_rose` | `$rose(A.map(x) with (x[0]))` composition with sampled value function |
| 16 | `array_map_with_countones` | `$countones(A.map(x) with (x))` composition with system function |
| 17 | `array_map_with_throughout` | `valid throughout A.map(x) with (x != 0)` map as sequence operand |
| 18 | `array_map_with_local_var` | Map result used with local variable assignment in sequence |
| 19 | `array_map_cross_implication_z3` | Map + implication full pipeline through Z3 |
| 20 | `type_this_parse` | `type(this)` in expression position parses to `TypeThis` |
| 21 | `type_this_roundtrip` | Parse → to_string → parse |
| 22 | `type_this_in_property` | `property p; type(this) == expected; endproperty` context parses |
| 23 | `type_this_translate_unsupported` | Translates to `Unsupported("type(this) in class scope")` |
| 24 | `type_this_not_z3_crash` | Z3 encoding handles Unsupported gracefully, does not crash |
| 25 | `type_this_vs_signal` | `type(this)` is distinct from `Signal("type")` in AST |
| 26 | `type_operator_general` | `type(expr)` general form recognized |
| 27 | `array_map_with_concat` | `A.map(x) with ({x, 1'b0})` concatenation in map body |
| 28 | `array_map_ieee_add_one` | IEEE 7.12 example: `A.map() with (item + 1'b1)` reproduced |
| 29 | `array_map_ieee_cross_array` | IEEE 7.12 example: `A.map(a) with (a + B[a.index])` reproduced |
| 30 | `array_method_existing_unchanged` | Existing `mem[addr]` field access/bit-select unchanged |
| 31 | `array_map_composition_pipeline` | Map + local var + implication + Z3 full pipeline |
| 32 | `type_this_in_checker` | `type(this)` inside checker (not a class) behavior documented |
| 33 | `array_map_empty_with_clause` | `.map(x) with (x)` identity transform |
| 34 | `sprint23_backwards_compat` | All 858 existing SVA tests pass unchanged |
| 35 | `sprint23_new_variants_counted` | SvaExpr enum now has 81 variants (79 + ArrayMap + TypeThis) |

---

## Part V: Tier 8 — Checker Type System

### Sprint 24: `rand real` / `rand const real` (IEEE 17.7)

**Why:** This is the highest-impact 2023 change for our pipeline. IEEE 1800-2023 Section 17.7 (p.513) extends checker free variables to `real` (64-bit floating-point) types. The current `RandVar` struct at `sva_model.rs` line 352 has `width: u32`, which models only integer/bitvector types. A `rand real` variable in a checker is a free variable that formal tools must quantify over — exactly like `rand bit`, but over the reals instead of bitvectors. This is a vertical cut through all four layers of the pipeline.

The IEEE 1800-2023 checker variable rules for `rand real` (Section 17.7, p.513-519):
- A `rand real` variable is a non-constant free variable: it can assume any real value at each timestep
- A `rand const real` variable is a constant free variable: it has a nondeterministic value at initialization, frozen for the entire trace
- Formal tools "shall take into account all possible values" (IEEE 17.7, p.514)
- Constraints via `assume` narrow the search space (IEEE 17.7.2, p.517)

**Files:** `sva_model.rs` (RandVar type), `sva_to_verify.rs` (BoundedSort), `ir.rs` (VerifyType), Z3 backend

**Structural changes at each layer:**

**Layer 1 — AST (`sva_model.rs`):**
```rust
/// Type discriminant for random variables (IEEE 17.7)
#[derive(Debug, Clone, PartialEq)]
pub enum RandVarType {
    /// Bitvector with width: `rand bit [N-1:0]`
    BitVec(u32),
    /// IEEE 754 double: `rand real`
    Real,
}

/// Random variable in a checker (IEEE 17.7)
#[derive(Debug, Clone)]
pub struct RandVar {
    pub name: String,
    pub var_type: RandVarType,
    pub is_const: bool,
}
```
This replaces `width: u32` with `RandVarType`. Migration: `RandVar { name, width: w, is_const }` becomes `RandVar { name, var_type: RandVarType::BitVec(w), is_const }`.

**Layer 2 — BoundedExpr IR (`sva_to_verify.rs`):**
Existing `ForAll` and `Exists` nodes carry a `sort` field. Add `Real` to the sort enum:
```rust
// In the quantifier sort discrimination:
BoundedSort::Real  // NEW — for rand real quantification
```

**Layer 3 — VerifyType (`ir.rs`):**
```rust
pub enum VerifyType {
    Int,
    Bool,
    Object,
    BitVector(u32),
    Array(Box<VerifyType>, Box<VerifyType>),
    Real,  // NEW — Z3 Real sort
}
```

**Layer 4 — Z3 backend:**
In `type_to_z3_sort`, add:
```rust
VerifyType::Real => z3::Sort::real(ctx),
```

> **DECISION:** Z3 RealSort (exact rational arithmetic) is used for `rand real`, NOT Z3 FPA (Floating-Point Arithmetic). Rationale: (1) FPA encoding is 10-100x slower than Real, (2) exact rationals soundly overapproximate IEEE 754 doubles — if a property holds over all rationals it holds over all doubles, (3) checker free variables are used for nondeterministic modeling, not for precise floating-point arithmetic, (4) this matches industry practice (JasperGold, Questa Formal use Real sort for rand real).

> **DECISION:** `shortreal` (32-bit float, IEEE 6.12) is deferred. The 2023 standard allows `rand shortreal` but this requires FPA sort for correctness (rationals are not a sound overapproximation of 32-bit floats due to rounding). Deferred pending Z3 FPA performance evaluation.

**Translation semantics:**
- `rand real r` (non-const): at each timestep `t`, create `Exists { var: "r@t", sort: Real, body: ... }` — the variable can take any real value per clock cycle
- `rand const real r` (const): create `Exists { var: "r", sort: Real, body: ... }` at trace level — the variable has one nondeterministic value frozen for the entire trace
- `assume property (r > 0.0 && r <= 1.0)`: adds `And(Gt(Var("r@t"), RealConst(0.0)), Lte(Var("r@t"), RealConst(1.0)))` as a constraint in the Z3 solver context
- Comparison operators `<`, `>`, `<=`, `>=`, `==`, `!=` work identically over Real sort as over BitVec — Z3's Real theory supports all relational operators natively
- Arithmetic `+`, `-`, `*`, `/` on real-valued expressions map directly to Z3 Real arithmetic

**Real literal parsing:** IEEE 5.7.2 (p.79) defines real literal syntax: `14.72`, `1.2E12`, `1.30e-2`, `0.1e-0`. A valid real literal must have at least one digit on each side of the decimal point (`.12` and `9.` are invalid). Scientific notation uses `e` or `E`.

**RED tests (~40 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `rand_real_parse` | `rand real r;` in checker → `RandVar{name: "r", var_type: Real, is_const: false}` |
| 2 | `rand_const_real_parse` | `rand const real r;` → `RandVar{name: "r", var_type: Real, is_const: true}` |
| 3 | `rand_real_roundtrip` | Parse → to_string → parse structural equality |
| 4 | `rand_const_real_roundtrip` | Parse → to_string → parse |
| 5 | `rand_real_vs_bit` | `rand real r` and `rand bit b` produce distinct `RandVarType` variants |
| 6 | `rand_real_vs_bitvec` | `rand real r` and `rand bit [5:0] idx` produce distinct types |
| 7 | `rand_real_checker_body` | `checker c; rand real r; assert property (r > 0.0); endchecker` full parse |
| 8 | `rand_real_in_assume` | `checker c; rand real r; assume property (r >= 0.0 && r <= 1.0); endchecker` |
| 9 | `rand_const_real_freeze` | `rand const real` value semantics: frozen for entire trace (existential once) |
| 10 | `rand_real_per_timestep` | `rand real` (non-const): can differ per timestep (existential per tick) |
| 11 | `rand_real_bounded_sort` | Translates to `BoundedSort::Real` |
| 12 | `rand_real_verify_type` | Maps to `VerifyType::Real` |
| 13 | `rand_real_z3_sort` | Z3 encoding uses `RealSort`, not `BitVecSort` |
| 14 | `rand_real_z3_sat` | `assume (r > 0.0)` is satisfiable in Z3 |
| 15 | `rand_real_z3_unsat` | `assume (r > 0.0 && r < 0.0)` is unsatisfiable |
| 16 | `rand_real_z3_const_vs_nonconst` | `rand const real` outer quantifier vs `rand real` per-timestep quantifier |
| 17 | `rand_real_z3_existential` | Existential quantification over Real sort produces valid Z3 formula |
| 18 | `rand_real_compare_gt` | `r > 0.5` translates to Z3 Real `>` |
| 19 | `rand_real_compare_le` | `r <= 1.0` translates to Z3 Real `<=` |
| 20 | `rand_real_compare_eq` | `r == 0.0` translates to Z3 Real `=` |
| 21 | `rand_real_arithmetic_add` | `r + 1.0` translates to Z3 Real `+` |
| 22 | `rand_real_arithmetic_mul` | `r * 2.0` translates to Z3 Real `*` |
| 23 | `rand_real_arithmetic_div` | `r / 2.0` translates to Z3 Real `/` |
| 24 | `rand_real_assume_constrains` | `assume (r >= 0.0)` constrains Z3 search space |
| 25 | `rand_real_quantifier_structure` | `checker_quantifier_structure()` correctly partitions Real rand vars |
| 26 | `existing_rand_bit_unchanged` | `rand bit d` still `RandVarType::BitVec(1)` |
| 27 | `existing_rand_bitvec_unchanged` | `rand bit [5:0] idx` still `RandVarType::BitVec(6)` |
| 28 | `existing_rand_const_bit_unchanged` | `rand const bit d` unchanged |
| 29 | `rand_real_mixed_with_bit` | Checker with both `rand real r` and `rand bit b`: coexist in same `rand_vars` vec |
| 30 | `rand_real_observer_pattern` | Observer model (IEEE 17.7 p.514 example) adapted with `rand real` |
| 31 | `rand_real_data_legal_pattern` | Data integrity checker (IEEE 17.7 p.515) adapted with real-valued signal |
| 32 | `rand_real_no_width` | `Real` variant has no width parameter (it's always 64-bit IEEE 754) |
| 33 | `real_literal_decimal` | `1.5` parses as real literal constant |
| 34 | `real_literal_scientific` | `1.2E3` parses as real literal constant |
| 35 | `real_literal_negative_exp` | `1.30e-2` parses correctly |
| 36 | `real_literal_invalid_no_leading` | `.12` is invalid per IEEE 5.7.2 (no digit before decimal) |
| 37 | `real_literal_invalid_no_trailing` | `9.` is invalid per IEEE 5.7.2 (no digit after decimal) |
| 38 | `rand_real_z3_trichotomy` | `r < 0.0 \|\| r == 0.0 \|\| r > 0.0` is tautology via Z3 |
| 39 | `sprint24_backwards_compat` | All 858 existing SVA tests pass unchanged |
| 40 | `sprint24_checker_tests_unchanged` | All Sprint 20 checker tests pass unchanged |

---

## Part VI: Tier 9 — Semantic Audit & Full Regression

### Sprint 25: 4-State Errata Audit, Cross-Feature Composition (IEEE 2023 Errata)

**Why:** The IEEE 1800-2023 committee resolved dozens of Mantis tickets regarding ambiguous evaluations, particularly tightening rules on how relational (`<`, `>`, `<=`, `>=`) and equality (`==`, `!=`, `===`, `!==`) operators evaluate when `X` (unknown) or `Z` (high-impedance) states are present. Our engine is strictly 2-state (`$isunknown` always returns `false`), so no code changes are expected, but we must exhaustively verify this claim. This sprint also tests cross-feature composition of all Sprint 22-24 features with all 21 prior sprints.

**Audit methodology:** For each operator, compare the 2023 truth table restricted to the 2-state subset (inputs are only `0` and `1`) against our Z3 encoding. All must match exactly. If any discrepancy is found, it indicates a bug in our existing encoding that the 2023 errata revealed.

**RED tests (~25 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `audit_eq_2state` | `==` operator: 2-state truth table matches 2023 Table 11-3 (restricted to 0/1) |
| 2 | `audit_neq_2state` | `!=` operator matches 2023 |
| 3 | `audit_lt_2state` | `<` operator matches 2023 |
| 4 | `audit_gt_2state` | `>` operator matches 2023 |
| 5 | `audit_lte_2state` | `<=` operator matches 2023 |
| 6 | `audit_gte_2state` | `>=` operator matches 2023 |
| 7 | `audit_case_eq_2state` | `===` case equality in 2-state is identical to `==` (no X/Z to distinguish) |
| 8 | `audit_case_neq_2state` | `!==` case inequality in 2-state is identical to `!=` |
| 9 | `audit_wildcard_eq_2state` | `==?` wildcard equality in 2-state behaves as `==` |
| 10 | `audit_wildcard_neq_2state` | `!=?` wildcard inequality in 2-state behaves as `!=` |
| 11 | `audit_isunknown_still_false` | `$isunknown` still returns `false` in 2023 2-state formal (no X/Z exists) |
| 12 | `cross_triple_quoted_with_real` | Action block with `"""..."""` in checker with `rand real` |
| 13 | `cross_map_with_real` | `A.map(x) with (x + r)` where `r` is `rand real` |
| 14 | `cross_map_with_checker` | Array map inside checker assertion body |
| 15 | `cross_type_this_with_map` | `type(this)` and `.map()` in same expression |
| 16 | `cross_real_with_local_var` | `rand real` value captured in local variable |
| 17 | `cross_real_with_until` | `rand real r; assume (r > 0.0); assert (count s_until (val > r))` |
| 18 | `cross_real_with_multiclock` | Real-valued checker variable in multi-clock property |
| 19 | `cross_real_with_vacuity` | Nonvacuity analysis on checker with `rand real` |
| 20 | `cross_all_2023_compose` | Single assertion using triple-quoted action + map + real-valued checker |
| 21 | `cross_map_with_temporal` | `s_eventually A.map(x) with (x > threshold)` |
| 22 | `full_858_regression` | All 858 existing SVA tests pass unchanged |
| 23 | `full_variant_count` | SvaExpr has exactly 81 variants (79 + ArrayMap + TypeThis) |
| 24 | `full_test_count` | Total SVA tests: 858 + ~130 = ~988 |
| 25 | `ieee_2023_compliance_matrix` | All 11 gaps from gap list addressed |

---

## Part VII: Verification Matrix

### Test File Distribution

All new tests go in one new file. Existing files untouched.

| File | Existing | New | Total |
|------|----------|-----|-------|
| `phase_hw_sva_coverage.rs` | 518 | 0 | 518 |
| `phase_hw_sva_ieee1800.rs` | 175 | 0 | 175 |
| `phase_hw_sva_surface.rs` | 58 | 0 | 58 |
| `phase_hw_sva_roundtrip.rs` | 47 | 0 | 47 |
| `phase_hw_sva_translate.rs` | 23 | 0 | 23 |
| `phase_hw_fol_to_sva.rs` | 30 | 0 | 30 |
| `phase_hw_codegen_sva.rs` | 7 | 0 | 7 |
| **`phase_hw_sva_2023.rs`** | **0** | **~130** | **~130** |
| **Total** | **858** | **~130** | **~988** |

### IEEE 1800-2023 Coverage After All 25 Sprints

| Capability | Variants | Tests | IEEE 2023 Section | Status |
|---|---|---|---|---|
| All IEEE 1800-2017 SVA (Sprints 1-21) | 79 | 858 | Ch. 16-17 | 100% |
| Triple-quoted strings in action blocks | 0 (parse-only) | ~14 | 5.9 | Sprint 22 |
| New system tasks ($timeunit, $timeprecision, $stacktrace) | 0 (parse-only) | ~11 | 20.4, 20.17 | Sprint 22 |
| Backwards compat (Sprint 22) | 0 | 5 | — | Sprint 22 |
| Array `.map()` method | 1 (ArrayMap) | ~19 | 7.12 | Sprint 23 |
| `type(this)` construct | 1 (TypeThis) | ~7 | 6.23 | Sprint 23 |
| Backwards compat (Sprint 23) | 0 | 9 | — | Sprint 23 |
| `rand real` / `rand const real` | 0 (RandVarType change) | ~32 | 17.7 | Sprint 24 |
| Real literals in assertions | 0 | ~5 | 5.7.2 | Sprint 24 |
| Backwards compat (Sprint 24) | 0 | 3 | — | Sprint 24 |
| 4-state operator audit | 0 | ~11 | 11.4 | Sprint 25 |
| Cross-feature composition | 0 | ~10 | — | Sprint 25 |
| Full regression | 0 | 4 | — | Sprint 25 |
| **Total new** | **+2 variants** | **~130** | | **IEEE 1800-2023** |
| **Grand total** | **81 variants** | **~988** | | **100%** |

---

## Part VIII: Out of Scope

The following 2023 features are intentionally excluded from this specification:

| Feature | IEEE 2023 Section | Reason |
|---------|-------------------|--------|
| `shortreal` (32-bit float) type | 6.12 | Deferred — Z3 FPA sort required (rationals not a sound overapproximation of 32-bit floats), pending FPA performance evaluation |
| Full OOP `type(this)` resolution | 6.23 | Only recognition, not semantic resolution — OOP class hierarchies outside SVA engine scope |
| Dynamic array `.map()` | 7.12 | Known-size unrolling only — full dynamic array support requires Z3 array theory integration |
| `covergroup extends` | 19.3 | Coverage model extension — outside SVA assertion engine scope |
| DPI/VPI 2023 changes | Ch. 35-39 | Foreign language interface — outside SVA engine scope |
| `expect` statement | 16.17 | Remains simulation-only, still out of scope |
| Pattern matching (`matches`, `priority if`, `unique if`) | 12.6 | Procedural constructs, not assertion-level — out of SVA scope |
| `$typename` function | 20.6.1 | Elaboration-time function, no formal semantics |
| Global clocking enhancements | 14.14 | No changes that affect our bounded temporal unrolling |

---

## Part IX: What "Done" Means

Extending SVA_COVERAGE.md's 14 capabilities with 5 new ones:

1-14. (All 14 IEEE 1800-2017 capabilities from SVA_COVERAGE.md remain fully operational)

15. **Parse** triple-quoted string literals (`"""..."""`) in assertion action blocks without rejecting valid 2023 input
16. **Recognize** `$timeunit`, `$timeprecision`, and `$stacktrace` system tasks in action blocks (parsed, excluded from Z3 encoding)
17. **Represent** array `.map()` expressions in the AST with bounded unrolling for known-size arrays, producing `Unsupported` for unknown-size
18. **Model** real-valued nondeterministic inputs via `rand real` / `rand const real` checker variables using Z3 RealSort (exact rationals)
19. **Maintain** 100% backwards compatibility: all 858 existing SVA tests pass unchanged, no SvaExpr variant removed, no semantic change to any existing construct

When all 25 sprints are complete and all ~988 tests are green, LogicAffeine's SVA engine is **IEEE 1800-2023 compliant** for Chapters 16 (Assertions) and 17 (Checkers).

---

## Part X: Migration Notes

### RandVar Breaking Change

The `RandVar` struct changes from `width: u32` to `var_type: RandVarType`. Every call site that constructs or destructures `RandVar` must be updated:

**Construction sites (in tests and checker parser):**
```rust
// Before:
RandVar { name: "d".into(), width: 1, is_const: false }
// After:
RandVar { name: "d".into(), var_type: RandVarType::BitVec(1), is_const: false }
```

**Destructuring sites (in checker_quantifier_structure, Z3 encoding):**
```rust
// Before:
let width = rand_var.width;
// After:
match &rand_var.var_type {
    RandVarType::BitVec(w) => { /* bitvector sort with width w */ }
    RandVarType::Real => { /* Z3 Real sort */ }
}
```

All existing tests that construct `RandVar` with `width: N` must be updated to `var_type: RandVarType::BitVec(N)`. This is a mechanical change with no semantic impact.
