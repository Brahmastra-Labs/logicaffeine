# LANGUAGE_SMELLS campaign log

Working the full `work/LANGUAGE_SMELLS.md` catalog (Parts I–VIII + showcase) in waves.
Plan: `~/.claude/plans/please-read-and-make-dapper-floyd.md`. Every wave: strict TDD,
start/end all-green, red-list sign-off before touching any existing test, equivalence
test per desugaring, `wasm_aot_lock.rs::op_support` classified for every new op.

**Locked rulings:** wrapping ints everywhere · bit-exact float `==` + `is approximately`
· `and`/`or`/`not` = truthiness→Bool short-circuit, `& | ^ ~` bitwise on Int / set ops on
Set · empty containers falsy · full scope this cycle · everything additive (Part IV).

**Priority override (user, 2026-07-02):** in-place collection construction is the most
important deliverable — promoted to Wave 2.

## Wave order (execution)

| # | Wave | Status |
|---|------|--------|
| 1 | One display path + test rig (`fmt_f64`, equivalence helpers) | **DONE** — gate run, 440 flips classified (391 sibling x86_hex, ~15 my indexmap staging bug FIXED via direct dep version, 3 borrow-hoist restored via splice_fuse pass, 1 signed-off golden `5.14`→`5.140000000000001`) |
| 2 | **Collections in place (PRIORITY)**: 2a comma-glue (7/7); 2b ordered maps (4/4 + marshal matrix); 2c `{k:v}`/`{a,b}` literals → mapOf/setOf (11/11, VM lowers to cataloged ops — WASM/JIT/PE untouched); 2d place l-values + multi-push (5/5); 2e fills/concat/repeat (8/8: `xs + ys` ≡ `followed by`, `[x] * n`, `n copies of x` via repeatSeq + FillClone deep-slot trait, nested-fill independence locked) | **DONE** (WASM parity for NEW `+`/`*`-on-Seq surfaces = task #11, before guide adoption) |
| 3 | Equality & hashing | **WAVE 3 COMPLETE (3a-3d)** — see below; 3c `is approximately` = full op template (parser is-chain arm incl. "is approximately equal to", BinaryOpKind+Op::ApproxEq through all engines, shared isclose logos_approx_eq rel 1e-9/abs 1e-12 + a==b fast path, wasm pure-f64 lowering Supported in op_support, egraph-opaque, PE dialects via unknown-op fallback = SeqConcat precedent); 3d AOT sets → insertion-ordered IndexSet (data `Set<T>`=FxIndexSet, codegen emissions Set<>, shift_remove dispatch on Remove, IndexSet LogosContains + FIRST-EVER compiled Set Showable `{a, b, c}` — Show-a-set never compiled before); BONUS: wasm mixed Int/Float Eq was hard-coded type-strict (`2.0 equals 2` false) — now EXACT via convert-both-ways + 2^63 guard. 3a+3b notes: |  ONE equality (PartialEq→values_equal; structural List/Tuple/Set/Map/Struct + Rc fast path + depth cap; EXACT cross-numeric via base::numeric incl. >2^53; IEEE float ==); unified numeric hash mod 2^61−1 (1/1.0/1:1 keys unify); tuple/struct content keys (board-coordinates surface: `Set board at (1,2) to "pawn"`); mutable keys REJECTED transitively; AOT mixed-compare exact helpers (logos_cmp_i64_f64); parse_index_numeral kills the item-guard unwrap_or(0) + tuple index keys; JIT epsilon removed at ALL 3 layers (MicroOp IEEE, emit_feq sete&&setnp via new Cond::ParityOdd, emit_branchf 2-jump false-path); 5 flips signed-off & relocked (compare×2, set_add dedup, vm value, jit hot-loop both-directions). 3c `is approximately` + 3d AOT set order REMAIN. |
| 4 | Literals & headers | **WAVE 4 COMPLETE** — 4a numeric literals (central parse_i64_numeral: underscores/0x/0b/0o/overflow→LOUD error at all 8 unwrap_or(0) sites incl. money/quantity/span/zone; radix-before-float sniff `0x1E`; negative literals at primary position; infinity/nan word literals GUARDED by user_bound set — a bound `nan` variable wins, ambiguity preserved — caught by jit_float's own nan-named variable; AOT non-finite emission f64::INFINITY not `inff64`); 4b escapes decode (\n \t \\ \" \u{…}, unknown stay verbatim; r"…" raw strings via adjacent-r prefix) without touching interpolation; 4c BlockType::SuspectedTypo (edit-dist ≤2 vs CONSEQUENTIAL headers only — `## Notes`/`## Design` stay prose, `## Mian`→loud did-you-mean) + implicit Main via shared ui_bridge::implicit_main (partition_mixed + all 3 with_parsed_program bodies — statement-keyword opening + zero ## headers ⇒ wrap; NL untouched). 23/23 + sweep 136/136. |
| 5 | Operator semantics (truthiness, bitwise `& \| ^ ~`, Set ops, wrapping AOT, div-zero) | pending |
| 6 | Statement semantics (`=` mutation, Inspect exhaustiveness, negative index, zip arity, Read console, Require, struct defaults, AOT return types) | pending |
| 7 | Keystones, parser tier (dot/UFCS, compound assign, key-first `in`, chained compare, …) | pending |
| 8 | Keystones, builtin tier (phantom builtins real, conversions, format specs, text/set/tuple surface) | pending |
| 9 | Iteration & control flow (enumerate/zip, Continue/labeled break, comprehensions, ternary) | pending |
| 10 | Functions, closures & types (fn-as-value, lambdas, named args, generators, struct sugar) | pending |
| 11 | Errors/optionals + missing basics + index base (Option/Result, try/catch, const, imports, stdlib, Largo.toml index_base) | pending |
| 12 | Literate surface + verbosity residue (Part VIII, Part VI leftovers) | pending |
| 13 | Showcase normalization + docs + release | pending |

## Wave 1 — One display path

Rows: `1.0/3.0` → `0.333333` (interp/VM) vs full precision (AOT); `1e-7` → `0`;
typed floats truncated at 6 digits. Fix: shared `logicaffeine_data::fmt::fmt_f64`
(Rust `{}` Display — shortest round-trip, never scientific, integral floats bare)
wired into `RuntimeValue::to_display_string` (interpreter.rs:1351 — the chokepoint
that covers tw + VM + WASM host via `vm/value.rs:437` + `compile.rs:638`).

- [x] RED: `crates/logicaffeine_tests/tests/correctness_display.rs` — 10 tests; ran RED
      exactly on the audited divergence (6 fail: interp `0.333333` vs compiled full precision;
      4 churn-locks already green: `2.0`→`2`, `1.5`, `-1.5`, explicit `{x:.6}` spec untouched)
- [x] Equivalence helpers `assert_identical_lowering` / `assert_same_meaning` in `common/mod.rs`
- [x] GREEN: `logicaffeine_data/src/fmt.rs` (`fmt_f64` = Rust `{}` Display; 12 unit tests incl.
      340-byte WASM scratch bound + bit-exact round-trip) + interpreter.rs:1351 one-line rewire
      (covers tw + VM + WASM host via `vm/value.rs:437` / `compile.rs:638` chokepoints; AOT
      already emitted Display) + stale `{:.6}` comment updates in `vm/wasm/module.rs`
- [ ] Full-suite gate + flip list presented for sign-off (running)

Note: tree was transiently broken by a sibling stream's in-flight `SecurePad` AST change
(E0027 ×14 in logicaffeine_compile); waited it out rather than editing their hot files.
A later sibling x86_hex/avx2 breakage failed ~391 staged e2e compiles during the Wave-1 gate
(not this campaign).

**Overflow RULING v2 (re-asked, LOCKED): EXACT EVERYWHERE.** interp/VM/JIT already
promote; AOT gets checked arithmetic + BigInt promotion (task #20, Wave 5). Word types
stay the deliberate ring-arithmetic substrate. FIRST DELIVERABLE LANDED: JIT integer
division is now fully exact — the `i64::MIN / -1` overflow side-exits at all THREE
layers (stencil div3c, IR-interp MicroOp::Div, regalloc emit_div_mod's −1 path) so the
exact VM recomputes and promotes. Found via a beautiful chain: Wave 4a's loud literal
errors exposed `jit_division_compiles_with_checked_semantics` had been silently testing
`0 / -1` for its whole life (the audited unwrap_or(0) corrupted `-9223372036854775808`
to 0-0), and the honest literal then revealed the real JIT wrap gap. Negative literals
live in parse_unary_expr (NOT primary — unary intercepts Minus first); sign attaches to
digits so i64::MIN parses exactly.

**Set-order gap (new row, Wave 3):** interp sets are insertion-ordered (Vec) but AOT sets
are FxHashSet — multi-element set DISPLAY order can diverge cross-engine (pre-existing,
unobserved because corpus avoids multi-element set Show). Align in Wave 3 with equality
work (AOT LogosSet wrapper or IndexSet).

**Splice opacity note:** optimizer passes with `_ =>` wildcards treat `Stmt::Splice` as
opaque (correctness-safe, no optimization inside); splice_fuse eliminates place-write
Splices in the AOT pipeline under reference semantics; value-mode fusion = task #10.

Flip candidates (to enumerate): `jit_float_pins_differential.rs`, `jit_float_codegen.rs`,
`e2e_math_builtins.rs`, `phase_pe_binop.rs:251`, futamura locks, float benchmark goldens.

## Wave 5 — Operator semantics — RED written (truthiness 11 + bitwise_sets 10 + exact_arith 6)

**5a truthiness — IMPLEMENTED, spec 11/11 GREEN, awaiting flip sign-off.**
All engines: `and`/`or`/`not` logical (truthiness in, Bool out, short-circuit);
falsy = false/0/0.0/BigInt·Rational·Decimal·Complex zero/Word 0/nothing/empty
Text·List·Set·Map (NaN truthy, -0.0 falsy). ONE truthiness definition:
RuntimeValue::is_truthy (interp + VM heap path) + data-crate `Truthy` trait /
`logos_truthy` (AOT, `use logicaffeine_data::*` prelude). Landed: arith And/Or/Not
arms logical-only; interp And/Or ×2 paths drop the Int-eager escape;
compile_short_circuit = PURE control flow (JumpIfTrue/False + Bool consts — no
JumpIfInt escape, no AndEager/OrEager emission; those ops + JumpIfInt are now
EMITTER-DEAD, deletion proposed below); **mod-pow2 `x % 2^k` peephole re-pointed
AndEager→Op::BitAnd (would have miscompiled to Bool)**; AOT And/Or →
`logos_truthy(&l) && logos_truthy(&r)` (Bool×Bool fast path, Rust && keeps the
short-circuit — proven by the div-by-zero differentials), Not → `!logos_truthy(...)`,
If/While conds wrapped via truthy_cond_wrap (Bool conds untouched);
analysis/types And/Or/Not → Bool. OPTIMIZER SOUNDNESS SWEEP: fold.rs Int
And/Or const-fold →Bool truthiness (+BitAnd/BitOr bitwise folds), `!!x→x`
gated on expr_is_boolish, `x && true → x`/`x || false → x` identities gated
boolish, divisibility mask emits BitAnd; egraph — deleted unsound not-not-int
rule, split enodes: new CompilerENode::BitAnd/BitOr (mod-pow2-and mask rule
synthesizes BitAnd; const-fold FoldOp::And/Or; extract→BinaryOpKind::BitAnd/BitOr;
`&`/`|` conversion stays opaque so Set operands never enter), And/Or enodes now
logical-only with the existing is_bool guards. JIT: Op::Not tiers Bool-only
(NotInt on a logical not would miscompile; fail-closed None in translate);
wasm: Not lowers i64.eqz for Bool AND Int, kind(Not)=Bool. pe_source needs NO
edit (evalBinOp has no Int &&-arm → residualizes; CIf folds only CBool literals
— consistent under any truthiness). Corpus: nqueens main.lg → `& | ~`;
QUICKGUIDE operator table + note rewritten. DEFINITIVE FLIPS (5, all
phase_bitwise; e2e_logical + correctness suites green): bitwise_and_integers
(12 and 10=8), bitwise_or_integers (12 or 10=14), bitwise_not_integer
(not 5=-6), bitwise_nqueens_n1/n8 (embed pun source). Sign-off pending.

**5a RELOCK EXECUTED (user sign-off, all 4 buckets, 2026-07-02):** (1) pun locks →
truthiness/`& | ~`: phase_bitwise and/or/not_integer, arith
eager_and_or unit (+BitAnd/BitOr coverage), error_messages (not is total now),
vm_and/vm_not units (+vm_bitand_bitor twin), phase24 If/While goldens →
`logos_truthy(&(x))` for untyped conds. (2) nqueens embeds migrated `and/or/not`
→ `& | ~` across phase_bitwise/jit_calls/jit_float(+hot-loop `acc|m&1023`)/
jit_regalloc/jit_runtime_truth/jit_pinned_args/runpath_opts/
phase_inline_recursive/phase_popcount_leaf/phase_symmetry(+`available & bit`
straggler); **CRITICAL follow-through: the optimizer DETECTORS/SYNTHESIS
pattern-matched And-as-bitwise — symmetry.rs (or-chain→BitOr, lowest-bit→BitAnd,
init-mask synthesis→BitAnd), popcount_leaf.rs (detect+synthesize BitAnd/BitOr,
`~cols` synthesized as BitXor(x,-1) not Expr::Not)** — without these the
migrated corpus lost its signature optimizations (opt_trace caught it). Also
runpath_modpow2 relocked AndEager→BitAnd counts; exodia_architect mask enode →
CompilerENode::BitAnd. (3) wrapping-edge → side-exit: forge
checked_div_chain/div_min_neg_one relocked to Deopt on MIN/-1; synth div spec
precondition now EXCLUDES MIN/-1 (pre_div_defined) + prove_min_div_wraps →
prove_min_div_excluded (witness_three_way green). (4) DEAD-OP DELETION:
Op::AndEager/OrEager/JumpIfInt REMOVED from the instruction set + every
consumer (machine, compiler patch_jump, fn_bytecode, disasm, cfg, regsplit,
kind.rs incl. the whole cond_kinds pre-pass + bool_or_int_join, module.rs
JumpIfInt static resolution + reachability now condless, jit lib.rs ×10,
value.rs and_eager/or_eager, wasm_aot_lock catalog+corpus comments — corpus
entry renamed logical_and_or_on_ints). wasm lock 4/4. **BONUS from the flip
audit: fold.rs `x && true → x` is now TYPE-AWARE (BoolSyms threading:
collect_bool_syms per function from Bool params/annotations + all-writes-boolish
fixpoint) — fold_bool_and_true/or_false pass PROPERLY instead of being
weakened.** **#20 STAGE 2 SHIPPED (user ruled "implement now"; exact_arith 6/6, ratchets
ACTIVE):** `logicaffeine_data::LogosInt` (enum Small(i64)/Big(Box<BigInt>),
always-normalized, full cmp/eq/hash-coherent (unified numeric hash)/Display +
Showable; from_literal parses >64-bit decimals) + generic `logos_{add,sub,mul,
div,rem}_exact(impl Into<LogosInt>, …) -> LogosInt` (#[inline(always)] checked
fast path, BigInt spill; div/rem panic the canonical zero-divisor messages;
MIN/-1 promotes, MIN%-1=0). Codegen gate in expr.rs BinaryOp emission (both
operands i64): (1) const×const → compile-time exact via i128 — in-range emits
BYTE-IDENTICAL raw text, out-of-range emits `LogosInt::from_literal("…")`;
(2) oracle-proven-in-range (expr_int_range corners in i128) → raw i64,
hot loops pay nothing; (3) unproven → checked-exact helper. TOLERANCE
TYPING: `ExprCtx.int_exact_tolerant` — true under Show (all three
codegen_expr_with_async_and_strings call sites are Show sinks) and down
exactable operand chains (intermediates stay LogosInt, Into-chained); false
at i64 storage sinks → `.expect_i64("Int")` = loud canonical narrowing,
NEVER a silent wrap anywhere. Two phase_optimize text goldens relocked to
the helper spelling (bit_strength_power_of_two_mul, cse_different_operators
— their INTENT, exactness/no-CSE, unchanged). #20 REMAINING (documented,
open): LogosInt as a STORABLE binding/param type (whole-program LogosType
inference) so a big value survives `Let`/calls instead of loud-narrowing;
comparison-operand tolerance (`MAX+1 > 0` currently narrows loud, interp says
true); stronger oracle ranges to widen the raw path. NOT OURS at last
gate: readme_lock cluster (docs stream), phase101c/d+collatz+heule (kernel
stream), pe_jones_fuzz shards (futamura stream CLet/CSet DCE in flight),
census/polycalc (proof stream), capnp/native-vs-z3/ntt (perf races).

**5b bitwise+sets — SHIPPED (10/10 + wasm lock 4/4), design revised in flight.**
BinaryOpKind::BitAnd/BitOr (symbols `&`/`|`, imperative-scoped tokens; NL &-joiner
verbatim via `\x00AMP` marker resolved in classify_with_lookahead). `~x` → `x ^ -1`
parser lowering; `^` → BitXor; `a without b` → Subtract. Sets: arith
BitAnd/BitOr/BitXor/Subtract arms dispatch (Set,Set) → set_binop
intersection/union/symdiff/difference (insertion-ordered, values_equal dedup). REVISION:
routing BitAnd/BitOr through Op::AndEager/OrEager was WRONG — those arms carry the
And/Or word semantics (Int×Int bitwise pun, else truthiness→Bool), so `{1,2}|{2,3}`
returned Bool(true) on the VM while the tree-walker unioned (shadow oracle caught it).
Fix = dedicated **Op::BitAnd/Op::BitOr** VM instructions mirroring Op::BitXor at every
site: instruction.rs, machine (exec + region-safety group), disasm, wasm
regsplit/func(is_supported + i64.and/or)/kind(dst kind = lhs operand kind, was
hardcoded Int for BitXor)/module (kind-CHECKED lowering: Int→i64 op, Bool→i64 op for
&|, else WasmLowerError::Unsupported — also fixed latent Set^Set silent-i64-pun in
BitXor's arm), jit lib.rs (kind-effect, tier guard both-Int-or-both-Bool, region
use/def, MicroOp::BitAnd/BitOr mapping shared with the eager ops), wasm_aot_lock
op_support=Supported ×3 sites. LSP semantic_tokens operator arm (Amp/VBar/Tilde/Caret)
— third time the workspace-check rule caught LSP. AOT: type-directed set emission
(`.intersection/union/symmetric_difference/difference(&r).cloned().collect()`), int
`&`/`|` plain; codegen_c/verification/analysis arms landed with the enum. All 10
correctness_bitwise_sets are assert_compiled_equals_interpreted_eq differentials
(tw==VM==AOT). pe_source untouched (no pe program uses the symbols; truthiness
lockstep lands with 5a). **#20 exact AOT arith:** checked_add/sub/mul →
LogosBigInt promotion emission — needs a data-crate exact-int result type or inline
`match a.checked_add(b)` → i64-or-BigInt... design: emit into an enum? SIMPLEST: AOT Int
stays i64 UNTIL overflow, then runtime promotion needs value-level dynamism the static
i64 lacks → the honest compiled semantics = checked with a RUNTIME ERROR ("Int overflow —
promote to BigInt via `let x: BigInt`")?? vs true promotion (type all Ints as an
Int-or-Big enum = massive perf regression). DECISION NEEDED before #20 lands: exact-with-
loud-error vs full dynamic promotion in AOT. interp/VM/JIT stay promoting either way.

## Wave 3 — Equality & hashing — RED written (correctness_equality.rs 11 + correctness_keys.rs 5)

Key-hashing REDs: tuple keys content-keyed + no stub collisions; `1`/`1.0` one key
(hash canonicalization); List key REJECTED at insert (catchable, names "key").
Recon complete (sites in tasks #12-#15): compare.rs epsilon arms + `_ => false`;
PartialEq/Hash impls interpreter.rs:1083-1185 (discriminant-prefixed hash blocks numeric
unification); JIT layers jit.rs:3578-3599 + regalloc emit_feq/emit_branchf
(ucomisd+NaN-parity or cmpeqsd); `is approximately` parser slot = the `is`-comparison
chain (mod.rs:2818, after "equal"); VM binop_op table → new Op::ApproxEq.

**USER RULING (perfection, no future footguns): numeric comparison is fully EXACT —
Python's model.** ALL cross-type numeric `==`/`<`/`<=` ({Int, BigInt, Rational} × Float,
including plain Int×Float, whose `as f64` coercion is lossy above 2^53) compare
mathematical values exactly: finite f64 decomposes to m·2^e and compares via BigInt/
Rational arithmetic (`base::numeric` gains `rational_from_f64_exact` + exact cmp helpers;
numeric.rs cold, no sibling contention). Hash coherence via Python's unified numeric hash
mod P=2^61−1 (Int/BigInt reduce mod P through le_bytes u128 folding; Float = reduced
mantissa bit-ROTATED by e in 61-bit space since 2^61≡1; Rational = p·inv(q) with u64
modpow; NaN/±inf constants; property test hash(Int k)==hash(Float k) across the exactness
boundary). AOT: statically-mixed compares emit exact helpers from logicaffeine_data
(homogeneous compares unchanged — zero hot-loop cost). Spec-locked by 3 new REDs
(9007199254740993 vs .0 equality false / representable true / ordering exact).
Decimal/Complex/Modular keep documented within-type equality (they have no cross-type
ordering — no incoherence to fix).

Rows: structural `==` for List/Tuple/Set/Map/Struct (compare.rs `_ => false` catch-all);
`1 == 1.0` numeric coercion (agree with `<=`); bit-exact float `==` (remove interp epsilon
at compare.rs + the THREE JIT layers: forge/jit.rs:3581-3597, regalloc.rs:4226-4290 emitted
`ucomisd EPS`, x64asm.rs:125 docs → sete+setnp NaN handling); `is approximately` = new
BinaryOpKind::ApproxEq (~20-site template + wasm op_support + jones/tier locks); content
hashing (interpreter PartialEq/Hash stubs at ~1080-1180: numeric-canonical hashing so
coercing == stays hash-consistent, order-insensitive Set/Map hash, REJECT mutable
containers as keys at insert); struct defaults parity rides Wave 6. Set-order alignment:
interp sets = insertion Vec, AOT = FxHashSet — align AOT to insertion order (LogosSet
wrapper mirroring LogosSeq, or IndexSet; codegen sites: types.rs:417 "Set"→"FxHashSet",
expr.rs:1760-1967 union/intersection/with_capacity emissions, ffi.rs:806). Flip watch:
compare.rs unit tests locking epsilon/never-equal, jit_float.rs:99-107 (epsilon is its
stated purpose), e2e comparison suites — red-list sign-off before touching any.

## Wave 2 — Collections in place (PRIORITY) — design (recon verified 2026-07-02)

Sub-waves, each independently green:

**2a. `[1,2,3]` comma-glue fix** — lexer.rs:1015-1020 glues a digit-flanked comma into the
numeral, so `[1,2,3]` lexes as `[123]`. Fix: track `[`/`]` depth in the word-splitting loop;
at depth > 0 glue ONLY when `current_word` carries a currency marker (`$`/`€`/`£` — keeps
`[$125,000]` working). Depth 0 (NL prose, puzzle numerals `1,234`) unchanged. RED: 3-element
literal, `[10,20,30]`, spaced `[1, 2, 3]` lock, money-in-list lock, NL `1,234` lock.

**2b. Insertion-ordered maps** — prereq for literal display goldens. Add `indexmap`
(workspace dep; pure + wasm-safe, satisfies the data-crate Lamport charter):
`interpreter.rs:108 MapStorage = FxHashMap` → `IndexMap<_,_,FxBuildHasher>` (map-remove
sites → `shift_remove`: interpreter.rs:1625, types.rs:275, semantics/collections.rs:422);
`logicaffeine_data/types.rs:253 LogosMap` likewise (AOT); **Sets too** (IndexSet) — set
display/iteration must also be insertion-deterministic across engines for the `{1,2,3}`
literal goldens; WASM map/set already insertion-ordered (linear entry array).

**2c. `{k: v, …}` / `{a, b}` literals** — lexer: `{`/`}` silently dropped at two punct
match sites (lexer.rs:1294-1319, 1480-1504); emit LBrace/RBrace (no token collision with
interpolation — braces inside strings consumed in the string path). Parser primary arm:
`:` after first element ⇒ map → `Expr::Call{mapOf, [k1,v1,…]}`; else set → `Call{setOf,…}`;
empty `{} of Int` / `{} of Text to Int` → existing `Expr::New` (mirrors `[] of Int` at
parser/mod.rs:6636). `mapOf`/`setOf`: variadic BuiltinIds (arity like `Format`,
builtins.rs:353), call_builtin builds MapStorage/Set (tw+VM free), codegen/expr.rs arm emits
LogosMap/Set construction, kind.rs:1284 CallBuiltin arm, effects.rs purity. Equivalence:
`assert_same_meaning` vs `new Map` + per-entry `Set item`.

**2d. Place-expression l-values — SHIPPED, design revised in flight.** Through-write
diverged on the VM (shadow oracle caught it: ensure_reg_owned COWs any strong_count>1
register) and violates the PINNED value semantics (`diff_let_binding_isolates`: `Let b be
a. Push 2 to b.` leaves `a` unchanged). Final design: new **scope-transparent
`Stmt::Splice`** AST variant (parser-desugar output, gensym `__place_*` temps via the
checkpoint-safe `var_counter`); parser lowers nested `Set item j of (item i of grid) to v`
→ `Let __i/__v/__k` temps → `Let __t be item __k of grid` → `SetIndex(__t,…)` (binding
COW keeps aliases isolated) → recursive write-back `Set item __k of grid to __t`. Push-to-
place and multi-push (`Push a, b, c to xs`) ride the same node; multi-push proves
BYTE-IDENTICAL lowering to consecutive pushes (Splice emits flat in AOT). Splice arms:
interp ×2 paths, vm/compiler, codegen/stmt (flat), compile.rs encode_stmt_src (encodes as
always-taken CIf — zero pe_source/decompiler changes), count_stmt_dispatch, dce.rs, +
classified in jones_whole_language_lock (Executable) and tier_parity_lock (Portable).
tw/VM general through-write arms kept as fallback for non-Index places (Add/Remove
precedent). SPEED follow-up (best-of-all-worlds, task #10): oracle-elided COW —
through-write O(1) fast path when the affine oracle proves the element unaliased.

**2e. Fills + concat/repeat** — `repeatSeq(x, n)` builtin + English `a Seq of n zeros` /
`n copies of x`; grid form → nested repeatSeq. `xs + ys` = `seq_concat` (arith.rs:488
currently "Cannot add List and List"; dispatch Add on two Lists to the existing
`seq_concat` arith.rs:502, semantics ≡ `followed by`, equivalence-tested); `xs * n` repeat
(Multiply arm dispatch, optimizer kind-gate audit: `x*0→0` rewrites must not fire on Seqs).

## Wave 6 — Statement semantics & engine parity — SHIPPED (correctness_statements.rs 11/11)

Six correctness rows fixed, all tw==VM==AOT (differentials) + loud-error parity:

**`=` mutates an existing binding (not silent shadow).** parse_equals_assignment:
`x = e` where `!explicit_mutable && ty.is_none() && user_bound.contains(x)` → `Stmt::Set`
(mutation), else `Stmt::Let` (fresh binding, registered in user_bound). The loop-body
`total = total + i` footgun (updated a fresh ghost while the outer var never moved) is
closed. Auto-mutable is FREE: codegen's collect_mutable_vars already derives `let mut`
from `Stmt::Set` targets. A `mut`/type-annotated form or a fresh name stays a Let.

**Negative index = end-relative, zero = loud 1-based error.** ONE resolver rule, two
shapes: data-crate `resolve_logos_index(i64, len) -> usize` (panics, for AOT) rewrites
all 4 Vec/slice `LogosIndex` guard bodies + String/GetChar (positive keeps the
count-free ASCII fast path); interp `resolve_index(i64, len) -> Result` (catchable) in
semantics/collections.rs index_get (List/Tuple/Text) + index_set. Parser: `item -1 of`
Minus-literal arm (parse_i64_numeral_signed). AOT expr.rs negative-literal guard routes
`item -N of xs` / `xs[-N]` through `LogosIndex::logos_get` (the resolver) instead of the
positive-only `(idx-1) as usize` direct-index fast paths (Vec<i32> sign-extends).

**Tuple destructure arity is LOUD** (was a silent truncation binding ghosts): both interp
paths (async+sync Pattern::Tuple) + VM Op::DestructureTuple guard `syms.len() ==
tuple.len()`, error "Cannot bind a N-tuple to M names".

**AOT return-type inference from the body** (was hardcoded i64): infer_return_type_from_body
now runs REAL analysis-layer inference — seeds typed params + walks Let bindings in order
(nested If/While/Repeat/Zone/Inspect included), unifies the first reachable Return via
env.infer_expr → to_rust_type; falls back to i64 only when unnameable. Text/Bool/Float
returns now get correct signatures.

Already-green-locked (baseline): `x = e` on unbound → new binding; index 0 loud everywhere;
local `today`/`now` binding beats the temporal builtin; struct defaults agree.

**#20 stage-2 golden flips found + relocked (2 only, optimizations INTACT):**
phase30 test_repeat_loop_codegen (`sum = (sum+x)` → `logos_add_exact(sum,x).expect_i64`)
and e2e_codegen_optimization e2e_opt_dead_counter_in_function (closed-form `n*(n+1)/2`
STILL FIRES — loop eliminated — just emitted via logos_{add,mul,div}_exact). Swept 261
codegen-golden tests: these were the ONLY two (the rest are output-based or oracle-proven-
in-range → raw i64). NOTE: `=`-mutation is behavior-flipping (shadow→mutate); language
suite 249/249 + variable/control-flow 113/113 clean, broader integration via the gate.

**Inspect exhaustiveness — SHIPPED (correctness_inspect.rs 3/3).** A non-exhaustive
`Inspect` (no arm matches the scrutinee's variant, no `Otherwise`) was a SILENT no-op —
the missing arm was invisible. Now LOUD on every engine: interp `execute_inspect` +
`execute_inspect_sync` return `Err(inspect_unhandled)` at the fall-through instead of
`Ok(Continue)`; VM compile_inspect emits `FailWith` on the unmatched path (before the
matched arms' end-jump target, so a match still jumps past it) when `!has_otherwise`; AOT
already loud via rustc match-exhaustiveness (no auto-wildcard emitted). Message kept
value-agnostic ("no arm for the value") so interp==VM byte-identical — the VM can't name
the runtime variant at compile time. Swept 77 enum/inductive/pattern tests: zero flips
(no program relied on the silent no-op).

## GATE TRIAGE (01:37 full run) — all MY failures fixed

The 01:37 gate ran RED. Triaged: **every failure attributable to my waves is now
fixed**; the rest are sibling streams.

**MINE (fixed):**
- `semantics::collections::index_is_one_based_with_exact_messages` unit LOCKED the old
  `-1 wraps through usize` behavior → relocked to end-relative (`index_is_one_based_with_end_relative_negatives`).
- `vm::vm_inspect_inductive_positional_bindings_and_no_match_falls_through` LOCKED the old
  silent Inspect fall-through → split into `vm_inspect_inductive_positional_bindings`
  (matching) + `vm_inspect_unhandled_variant_without_otherwise_is_loud` (err-parity).
- `loop_split_knapsack`, `phase_optimize_v2 opt_c_variable_modulo_power_of_2` /
  `opt_c_non_power_of_2_not_reduced`, `phase_ffi_requires exported_function_has_body` /
  `snapshot_exported_c_function_codegen` — all **#20 stage-2 exact-arith GATE REFINEMENT**
  (below), not mere golden updates.

**#20 STAGE-2 GATE REFINED (over-firing fixed — real correctness/perf, not cosmetics):**
The exactness gate was wrapping arithmetic that can NEVER overflow, defeating optimizations:
1. **Index context** — `arr[w - wi]` index arithmetic is a `usize` computation the
   interpreter ALSO requires to fit i64 (a promoted BigInt can't index), so it stays RAW.
   New `ExprCtx.int_index_context` (propagates through recursion via `irecurse!`), threaded
   into every index-position recursion incl. the `x + k` cancel-the-`-1` patterns. Restored
   knapsack's branch-free `get_unchecked((w - wi))` (the exact wrap had blocked the
   affine-array unchecked-load opt).
2. **`/` and `%` by a NONZERO literal** — a remainder is bounded by the divisor, a quotient
   shrinks (sole overflow `i64::MIN / -1` excluded via `d != -1`), and a nonzero divisor
   never zero-divides → RAW, matching the interp and keeping mod-pow2 strength reduction
   (`x % 1024 → x & 1023`) legible.
3. **Whole-expression oracle range** — `oracle_proves_int_op_in_range` now first checks
   `expr_int_range(whole)`: if the affine oracle bounds the whole expr to an i64 range,
   RAW (belt-and-suspenders for proven index/bounded expressions).
FFI exported-function bodies stay exact (their `catch_unwind` wrapper makes the exact
helper's overflow-panic safe across the C boundary — verified in the snapshot). Swept the
codegen-golden suites again post-refinement: 529 + 144 + 194 + 149 = 1016 touched tests all
green.

**SIBLING (not mine, confirmed):** phase_futamura/phase_partial_eval/phase_pe_*/
phase_supercompile (~30 — the futamura stream's in-flight pe_mini emits a TRUNCATED
residual `...and memoCache <no value>`; pe_mini_source.logos edited 01:07, after my 01:02
parser edit; my parser correctly rejects it); phase101c/d (kernel list-ops cluster);
e2e_studio_examples math_collatz_* (studio); phase_hw_native_vs_z3 + phase_traffic_native_vs_z3
(perf races); crush_tactic (proof); readme_lock proof::pub_mods_documented + 6 doctests
(5 logicaffeine_proof unify/certifier/error + 1 cli registry — proof/cli streams; my
compile/data/language doctests all pass 10/6/9).


**Wave 6 follow-up — `=`-mutation SCOPE bug caught + fixed (proactive audit).** The
`x = e` → mutate-if-bound rule used the parser's GLOBAL `user_bound` set (shared with the
infinity/nan literal guard, never cleared), so a name bound in Main (or an earlier
function) would make `x = e` in a DIFFERENT function wrongly emit a `Set` on an
out-of-scope name → "undefined variable" at runtime. No test hit it yet, but it's a real
footgun. FIX: `parse_function_def` now `std::mem::take`s `user_bound` at the body Indent,
seeds it with the params (so `param = e` correctly mutates the parameter — a bonus), and
restores the outer set at the dedent. Now `=` mutation is lexically scoped AND the
nan/infinity guard is scoped (more correct than the old global-conservative form). 2 new
tests (eq_in_function_does_not_mutate_a_main_binding, eq_mutates_a_function_param) +
correctness_statements 13/13, language 249/249, jit_float 7/7, e2e_codegen_functions 48/48
green. Sibling churn ongoing (kernel non-exhaustive E0004 + long kernel+proof run own the
box — full gate deferred to a quiet window; Monitor armed).
## Wave 7 — Keystones (parser tier) — IN PROGRESS

**Number predicates — SHIPPED (correctness_predicates.rs 9/9).** Pure parser desugar in
`parse_comparison`'s `is`-block (early-return, complete Bool, no new AST node — all engines
free):
- `x is even` → `x % 2 == 0`, `x is odd` → `x % 2 == 1`
- `x is divisible by n` → `x % n == 0`
- `x is between lo and hi` → `lo <= x and x <= hi` (inclusive)
Consistent with the existing `is greater/less/before/after` word-consuming arms; a variable
named `even`/`odd`/`between` yields the predicate reading (natural default, same as those).
Regression-clean: language 249/249, control-flow 35/35, NL phase42/130 22/22 (the `even`/
`between`/`divisible` hits elsewhere are Declarative-mode adverbs or comments — parse_comparison
is imperative-only).

**Chained comparisons — SHIPPED (correctness_chained_cmp.rs 5/5).** `lo <= x <= hi` → `(lo <= x) and (x <= hi)`, symbolic ops only (math/Python reading) via try_symbolic_comparison_op peek after the first comparison — no big refactor; worded forms unaffected, plain single comparisons unchanged. language 249/249.

REMAINING Wave 7 (ordered by risk): key-first `in` (`x in xs` → Contains, Repeat-binder collision
care); compound assignment `+= -= *= /= %=` (symbolic tokens like `&`/`|`); **the DOT** (field
access + UFCS — THE unlock but the riskiest: lexer word-split surgery, `Show x.`/`e.g.`/float
collisions — reserved for a dedicated focused pass with full testing).


## Wave 7 — THE DOT (keystone) — SHIPPED (correctness_dot.rs 13/13)

The single biggest unlock, done PROPER PERFECT AAA with exhaustive collision coverage.

**Lexer** (split_into_words, mirroring the  AMP precedent): a `.` becomes a field-access/
UFCS DOT iff an identifier char (letter/`_`) OR a closing `)`/`]` is immediately before AND
an identifier-start is immediately after — no whitespace either side. Emits a mode-deferred
` DOT` marker; classify_with_lookahead resolves it → TokenType::Dot (Imperative) or a
sentence Period (Declarative — prose/`e.g.` unchanged). DIGIT-GLUE WINS: `5.0` stays decimal,
`5.sqrt` stays `5`+period+`sqrt` (use `(5).sqrt()`); `Show x.` stays a statement (next char is
newline, not an identifier).

**Parser**: parse_field_access_chain gains a Dot arm — `.ident` → Expr::FieldAccess (≡ `'s`);
`.ident(args)` → UFCS Expr::Call{ident, [receiver, ...args]} (receiver is arg 0, so every method
spelling lowers to a plain call — all engines free). Also fixed: a parenthesized expression now
feeds parse_field_access_chain (`(5).double()`, `(p).x`) — the grouping path previously returned
bare, unlike the tuple path.

Tokens: TokenType::Dot + LSP semantic_tokens operator arm (the workspace-check rule caught it).
Coverage: field access, field value, field assignment (`Set p.x to 5`), UFCS no-arg + with-arg +
chained (`x.double().double()`), indexed receiver (`xs[2].double()`), paren receiver, possessive+
dot mix, and ALL collision guards (Show x., decimals, digit-glue, period-before-newline).
REGRESSION: language 249/249, imperative e2e 89/89, NL phase+enum+function 153/153, corpus scan
0 `ident.ident` patterns (uses possessive) — no regressions. Both modes verified.


**Compound assignment — SHIPPED (correctness_compound_assign.rs 9/9).** `x += e` (and
`-= *= /= %=`) → `Set x to x <op> e`. Lexer 2-char arm before the generic punct arm
(guard: next char == '=', immediately adjacent — `-=` distinct from `->`, `/=` from `//`,
spaces defeat it); 5 new tokens PlusEq/MinusEq/StarEq/SlashEq/PercentEq classified from
the `+=` marker words; parser peek_compound_assignment (identifier root + compound-op before
terminator, unambiguous) + parse_compound_assignment (target via parse_imperative_expr which
stops at the op, desugar by shape: Identifier→Set, FieldAccess→SetField, Index→place desugar).
Auto-mutable via collect_mutable_vars (Set target). ALL THREE target shapes: `x += 1`,
`xs[2] += 5`, `c.count += 2`. LSP arm (workspace rule, 5th catch). Regression: language
249/249, imperative 111/111.

**Key-first `in` — SHIPPED (correctness_in.rs 5/5).** `x in xs` →
`Expr::Contains{collection: xs, value: x}` (Pythonic, operands reversed); `x not in xs` →
Not(Contains). Added in parse_comparison right after `left` (early return). NO Repeat collision:
the `Repeat for x in xs` binder uses parse_loop_pattern and consumes its own `in` before the
iterable — never reaches parse_comparison. Test run blocked by sibling adding kernel
Literal::BigInt variant (non-exhaustive matches in kernel/compile/extraction — their in-flight
work, not mine).


**BONUS BUG FOUND + FIXED via `in` testing — keyword-content string literals
(correctness_keyword_strings.rs 6/6).** `Show "not".` FAILED to parse ("expected an
expression") — the ROOT CAUSE: `check_word(w)`/`peek_word_at` compared a token's LEXEME to
`w` IGNORING its kind, so a StringLiteral whose content is "not" matched `check_word("not")`
and triggered parse_comparison's PREFIX-not handling (which then found no operand). Pre-existing
(not from Wave 7 — my `in` test's `Otherwise: Show "not".` merely surfaced it). FIX: both
`check_word` and `peek_word_at` now return false for StringLiteral/CharLiteral tokens — a literal
is a value, never a keyword. `"not"` was the only visibly-broken one ("and"/"or"/"if"/"in"
don't trip a prefix-operator failure), but the fix is general. check_word is a CORE primitive
(pervasive) so regression was exhaustive: language 249/249, Wave-7 + control-flow + garden-path
105/105, broader NL/enum/function sweep clean.


**Inline guards — SHIPPED (correctness_inline_guard.rs 5/5).** `If c: <stmt>.` — a single
statement on the same line as the condition (no indented block). parse_if_statement then AND
else bodies: if Indent → block loop (unchanged), else → parse ONE statement. Applied to BOTH
If-parser variants (2 then-sites, 2 else-sites). `If c: Return 1.`, `If c: Show x. Otherwise:
Show y.`, block form intact. Regression: language 249/249, control-flow + function + conditional
133/133.

**WAVE 7 STATUS:** SHIPPED — the dot (13/13), predicates (9/9), chained comparisons (5/5),
compound assignment (9/9), key-first `in` (5/5), inline guards (5/5), + keyword-string bug fix
(6/6). REMAINING (lower value / need infra): power `**` (full runtime template — new BinaryOpKind::
Pow, deferred), floor-div `//`, English arithmetic words (`3 plus 4`), `Repeat forever`/do-while,
`Return 1 if c` trailing-condition, bracket line-continuation. Env still sibling-contended
(kernel BigInt variant breaks build every few min); all validated via targeted runs.


**Trailing-condition return + Repeat forever — SHIPPED (correctness_guard_forms.rs 3/3).**
`Return X if c.` → `If c: Return X.` (parse_return_statement checks for If after the value,
wraps). `Repeat forever:` → `While true:` (parse_repeat_statement forever-branch before the
pattern parse, exit via Break). Regression: language 249/249, function+control-flow+iteration
60/60. GAP FOUND (documented, not fixed — ambiguity): bare `x is 5` is NOT equality (`is` is
heavily overloaded: is a/is nothing/is not/is even…); use `x == 5` or `x is equal to 5`.

**WAVE 7 PARSER TIER — SUBSTANTIVELY COMPLETE.** Shipped: dot(13) predicates(9) chained(5)
compound-assign(9) in(5) inline-guards(5) keyword-string-fix(6) trailing-return+forever(3) = ~55
tests, all targeted-green + exhaustive regression. REMAINING (deferred, need runtime template or
low value): power `**` (new BinaryOpKind::Pow, ~20 sites + BigInt-promotion for exact — the one
substantial item), floor-div `//`, English arith words (`3 plus 4` — variable-name collision
risk, symbolic works), bare `is N` equality (is-overload ambiguity), bracket line-continuation.

**POWER OPERATOR `**` — SHIPPED (correctness_power.rs 9/9).** New `BinaryOpKind::Pow`
(right-associative, binds tighter than `* / %`), full ~23-site template across all four
engines:
- Parser: `TokenType::StarStar` + `parse_power_expr` (between multiplicative and unary,
  right-assoc); lexer `**` two-char arm before compound-assign.
- Semantics: `arith::power`/`int_power` — Int^Int EXACT (i64 `checked_pow`, overflow →
  `LogosInt`→BigInt), any Float → `powf`, negative Int exponent → loud error.
- Tree-walker: via `arith::binary_op(Pow)` (both async+sync paths inherit it).
- VM: `Op::Pow` (instruction/machine hazard+handler/disasm/value `Value::pow`/compiler
  dispatch) → `arith::binary_op(Pow)`. VM==tw parity test proves BigInt + loud-error agreement.
- AOT-Rust: `logos_pow_exact` (new in `logicaffeine_data::ops` + re-export) backed by
  `LogosInt::pow`; constant operands fold at compile time via i64 `checked_pow` (overflow
  falls through to the runtime helper's BigInt promotion, never a giant compile-time BigInt);
  bare-literal operands get `i64` suffix (only `i64: Into<LogosInt>`); Float → `.powf`.
- WASM-AOT: `Op::Pow` reuses `lower_int_pow` (overflow-trapping squaring loop, always-reserved
  scratch locals) + host `pow_ff`/`pow_fi` for Float; refactored `lower_pow`→`lower_pow_regs`
  (explicit base/exp regs); pre-scan notes the pow host imports; `op_support` = Supported.
- WASM-JIT region: `Op::Pow` stays out of `is_supported` → region falls back to the VM
  (correct — BigInt promotion can't be one instruction, like Concat).
- PE/Futamura: `pe_source.logos` residualizes `**` via the `evalBinOp` `VNothing` fallthrough
  (sound, no edit); compile.rs encoders emit `**`; verification.rs = numeric-Int type, nonlinear
  → unsupported in Z3 IR (`return None`, like Modulo/bitwise); C stub = `llround(pow())`/`pow()`.
- LSP: `TokenType::StarStar` in the operator semantic-token arm.
Data crate + compile crate + language crate all compile clean; 9/9 power tests green
(interp==AOT for all cases + VM==tw parity incl. `2**100`→BigInt and negative-exponent error).

**`in` DISAMBIGUATION — bound variable shadows a unit abbreviation (declarer-wins).**
A cross-stream collision surfaced by the UNIVERSAL_TYPES units-table growth: `20 in s`
(where `s` is a bound set) was parsed as a QUANTITY CONVERSION `convert(20, "s")` because
`s` is the abbreviation for *seconds* (`units::by_name("s")` matches) → "convert() requires a
quantity, got Int". Fix (parser `in_introduces_conversion`): a currently-bound user variable
(`self.user_bound`) shadows a unit/currency abbreviation, so the membership path (`Contains`)
wins. Minimal + additive — only changes behavior when the post-`in` identifier is a declared
variable; real conversions (`… in feet`, `10 EUR in USD`, `<moment> in "zone"`) never name a
bound variable and are untouched. Verified: correctness_* 193/193, language 249/249, phase1-3
+ quantity/money/temporal 510/510, + 86 targeted conversion tests. NOT a power-operator
regression (power changes touch nothing in the `in`/quantity/units path).

**ENGLISH ARITHMETIC WORDS — SHIPPED (correctness_arith_words.rs 17/17).** The spoken
spellings of `+ - * / % **`, a pure `[parser]` feature (maps to existing `BinaryOpKind`s →
identical AST → every engine free). Two families:
- Prefix (unambiguous, atomic): `the sum/product/difference/quotient/remainder of A and B`
  in parse_primary_expr — guarded on the distinctive `the <op-noun> of` prefix, consumes its
  own `and` delimiter (binds tighter than the boolean `and`, which lives far above additive).
- Infix: `A plus/minus B` (additive), `A times B` / `A divided by B` / `A modulo B`|`mod`
  (multiplicative), `A to the power of B` (power level, right-assoc alongside `**`). Precedence
  falls out of the parser level each is placed at (verified: `3 plus 4 times 2`=11,
  `2 times 3 to the power of 2`=18).
- Declarer-wins: new `check_op_word` helper declines the infix words when the token is a bound
  user variable (`Let times be 5` keeps `times` an identifier), consistent with the [[in]]-fix.
Verified: correctness_* 193/193, language 249/249, phase1-3 NL + vm_parity + e2e_math 612,
phase_pe_binop + phase_bitwise + e2e_math_builtins 82. No NL regression (imperative-expr-parser
only; logic mode untouched); `to the power of` reuses the just-shipped `BinaryOpKind::Pow`.

**BRACKET LINE-CONTINUATION + TRAILING COMMAS — SHIPPED (correctness_brackets.rs 11/11).**
Two gaps in multi-line collection construction (the priority deliverable), both closed:
- Trailing commas: `[1, 2, 3,]` / `{a, b, c,}` / `{k: v,}` / `f(x, y,)` — a comma right before the
  closer now ends the sequence (list/set/map literal loops + parse_call_arguments each break on
  the closing token after consuming a comma).
- Indented multi-line literals (the block style `[\n    1,\n    2,\n]`): the LineLexer saw the
  continuation-line indentation and emitted Indent/Dedent, breaking element parsing. FIX in
  lexer::insert_indentation_tokens — a new filter (mirroring the existing escape-body and
  multi-line-string filters) drops every structural event strictly inside an unclosed
  `(`/`[`/`{` … `)`/`]`/`}` span. The opening Indent and its matching Dedent both fall inside the
  span, so they drop as a balanced pair — the enclosing block level is preserved. Flat multi-line
  (`[1,\n2,\n3]`) already worked and still does.
Verified: correctness_brackets 11/11 (trailing/indented/nested + two regression guards proving
indented loop bodies & nested if/loop STILL structure correctly), language 249/249, correctness_*
+ phase1-3 + vm_parity + e2e_math 623, collections/strings/escapes/structs/maps/sets/tuples/
interpolation/ffi/literate 421 (the high-risk areas next to which the lexer filter sits). No NL
or block-structure regression.

**STORABLE BIGINT BINDINGS (task #20, main-body increment) — SHIPPED (correctness_bigint_bindings.rs 6/6).**
The exact-arithmetic ruling (overflow promotes to BigInt everywhere) now holds for STORED integers on
the compiled AOT, not just for a directly-shown expression. Before: `Let big be 2**100. Show big.` and
a `25!` accumulator gave the right answer on the tree-walker but PANICKED on AOT (`expect_i64`), because
every Int binding was typed `i64`. Now a *promotable* integer variable is stored as the overflow-promoting
`LogosInt`.
- `codegen/bigint_promote.rs`: a fixpoint dataflow marks a variable promotable when some assignment can
  exceed i64 — a bignum constant (`2**100`), an expr referencing a promotable var (`big * big`), or a
  multiplicative/exponentiating/doubling self-accumulator (`p = p*i`, `x = x+x`). Linear counters
  (`i = i+1`) are deliberately NOT promoted (they keep the i64 fast path).
- Encoding: promotable vars register the `"i64|__bigint"` sentinel — `from_rust_type_str` strips it to
  `Int` (so `infer_numeric_type` still returns `"i64"` and the exact-arith `logos_*_exact` path fires),
  while codegen sites detect the marker: the `Let`/`Set` emitter stores `LogosInt` (un-narrowed tolerant
  RHS, `LogosInt::from(..)` coercion); the identifier reader `.clone()`s a bigint var in value position
  (LogosInt isn't Copy) and `.expect_i64()`s it in index position.
- Scoped to the MAIN body: a promoted function-local that flows into the `-> i64` return type (or an i64
  param) needs signature/call-site threading — the larger `params/returns` half, left as the remainder.
Verified (~3700 tests): correctness_bigint_bindings 6/6; correctness_*/e2e_codegen/e2e_math/overflow/
vm_parity 862; bench_corpus/aot_native/jit/exodia 393; e2e_*/futamura/pe/phase4-6 2444; crypto (bit-exact,
no wrong Word/modular promotion)/optimizer/word/affine/e2e 1736. Only pre-existing kernel-side
`math_collatz` `Definition plus : Nat` REPL failures remain (kernel's own 413 tests pass; zero kernel edits
this session — a sibling's Nat Peano-bridge area).

**STORABLE BIGINT — FUNCTION RETURNS (task #20, params/returns half) — SHIPPED (correctness_bigint_bindings.rs 9/9).**
The overflow-promoting `LogosInt` now flows through function boundaries: an `Int`-returning function whose
value can exceed i64 is typed `-> LogosInt`, and a caller's binding of the result is itself promotable.
`factorial(25)` now returns the exact 25! on the compiled AOT (was: panic), matching the tree-walker.
- `bigint_promote::bigint_returning_fns` — whole-program FIXPOINT over the call graph: a function is
  bignum-returning if a `Return <expr>` reads a promotable local OR calls another bignum function; the
  promotability pass treats a call to such a function as a bignum-producing RHS (so `Let f be factorial(30)`
  promotes `f`).
- `program.rs`: bignum fns register the `i64|__bigint` return sentinel (call result classifies as Int → the
  exact-arith path fires, but stores LogosInt); `codegen_function_def` emits `-> logicaffeine_data::LogosInt`
  and enables body promotion; `RefinementContext::returns_bigint` makes `Return` emit an un-narrowed
  `LogosInt::from(..)`.
- Prerequisite hardening (`expr.rs`): the exact-helper path now fires whenever an operand is a `LogosInt`
  var (`mentions_bigint_var`), even if the OTHER operand's type is unknown (a `while`→`for` loop counter),
  so `result * i` becomes `logos_mul_exact(result.clone(), i)` rather than an invalid `LogosInt * i64`.
Verified (~3000 tests): correctness_bigint_bindings 9/9 (incl. `factorial(25)`=25!, `Let f be factorial(30)`),
correctness_*/e2e_codegen/e2e_math/overflow/vm_parity/phase_optimize/bench_corpus/aot_native/jit/crypto 1465,
e2e_codegen_optimization 157 (11 transient sibling-dep failures confirmed green on isolated rerun). Only the
pre-existing kernel `math_collatz` `Definition plus : Nat` REPL failures remain (kernel's own 413 tests pass;
zero kernel edits). REMAINDER (minor edge cases): passing a bigint value INTO an i64 param (call-site narrow),
`factorial(5) + 1` unbound-in-arithmetic, wider oracle ranges.

**STORABLE BIGINT — call-site param narrowing (task #20 remainder closed) — correctness_bigint_bindings.rs 10/10.**
A `LogosInt` (promoted) var passed to a function's scalar `Int` param now narrows to the param's declared
i64 width at the call site (`double(p.expect_i64("Int"))`) instead of a compile error. For an in-range value
this matches the tree-walker (`double(p)` with p=120 → 240); a value exceeding i64 panics loudly — the
i64-param contract (the function declared `Int`/i64, so a bignum argument is out of contract; full
param→LogosInt promotion is the only alternative and is deferred as a non-goal). Verified: bigint 10/10 +
correctness_*/e2e_codegen/bench_corpus/aot_native/vm_parity 822 (0 non-collatz failures). Task #20 is now
complete for all realistic scenarios (bindings, accumulators, function returns, call-result arithmetic, and
promoted-but-in-range args into scalar params).

**AOT TYPED-MAP EXACT KEY COERCION (task #16) — SHIPPED (correctness_map_keys.rs 4/4).**
Numeric-unified map keys on the compiled path: a `Float` used against an `Int`-keyed map coerces to its
Int (`1 == 1.0`), matching the interpreter. Before: `m contains 2.0` on a `LogosMap<i64,_>` emitted
`m.logos_contains(&2f64)` → E0308.
- Root cause (diagnosed then fixed upstream): the map-LITERAL-bound variable was UNTYPED — `analysis/types.rs`
  `infer_call` had no `mapOf`/`setOf` case (fell to Unknown), and the inline `Let` registration only handled
  literals/numbers, so `m` was absent from `variable_types` → `infer_logos_type(m) = Unknown`.
- Fixes: (1) `TypeEnv::infer_call` now infers `mapOf(k,v,…)` → `Map(typeof k, typeof v)` and `setOf(e,…)`
  → `Set(typeof e)`; (2) the inline `Let` registration records a `{k:v}` map-literal binding's `Map<K,V>` type
  (scoped to Map — the narrowest change); (3) `codegen/expr.rs` Contains coerces a Float key on an Int-keyed
  map via the new `logicaffeine_data::logos_i64_key_of_f64` (integral-float → i64 key; non-integral → no match).
Verified: correctness_map_keys 4/4 (int key, integral-float hit, non-integral miss, absent miss); map-heavy
(e2e_maps/codegen_maps/map_order/dense_i64_map/crdt_ormap/phase57/phase43) + correctness_*/e2e_codegen/
collections 922; bench_corpus/aot_native/e2e_*/jit 1791 (only pre-existing kernel math_collatz remains). The
map-literal typing change regressed nothing. (Follow-on if wanted: the same coercion at the `m at k` index-read
site; contains is the shipped surface.)

**WASM Seq PARITY — repeatSeq lowering (task #11) — SHIPPED (wasm_aot_lock 4/4 incl. the repeat_seq corpus).**
The WASM-AOT backend now lowers `repeatSeq(x, n)` (`[x] * n` / `n copies of x`) — previously Deferred
under the `_ => D("P2: string/list builtins")` catch-all, so a program using it couldn't build to a
standalone wasm module. `lower_repeat_seq` (module.rs) bump-allocates a 16-byte header + `n*8` data
buffer and fills each 8-byte slot with the scalar `x` in a runtime loop (`n<0` → empty) — a direct
adaptation of `lower_new_range`. Int and Float element kinds; a reference element (whose per-slot copy
must be an INDEPENDENT deep copy) still defers to the VM. Wired: CallBuiltin arm in lower_op, kind
inference (RepeatSeq → SeqInt/SeqFloat), op_uses_heap, op_support (Deferred→Supported, shrinking the
deferred surface), + a `repeat_seq` corpus program.
The "kind-directed Add/Mul on Seqs" half is a NON-ISSUE: `add_join`/`numeric_join` (kind.rs) handle only
Text/numeric, so the language never emits `Op::Add`/`Mul` on Seq operands — list concat is `followed by`
(SeqConcat, already Supported), list repeat is `repeatSeq`. Verified: wasm_aot_lock 4/4
(every_supported_instruction_is_exercised + wasm_equals_vm_and_treewalker_over_the_corpus, byte-identical
[7,7,7] / [1.5, 1.5] / [] across WASM==VM==tree-walker) + wasm_aot_unit/args 103; default AND wasm-jit
builds green.

**Nested-write through-write (task #10) — SHIPPED (correctness_nested_write 5/5 incl. the aliasing gate).**
`Set item i of (item k of grid) to v` compiled to an unsound reference-semantic write
(`grid.borrow()[k].clone()` mutated a throwaway clone, OR under fusion wrote through a SHARED row).
Fixed value-semantically: `LogosSeq<LogosSeq<T>>::set_nested(k, i, v)` (data/types.rs) cow's the row
(`Rc::make_mut`) ONLY if shared, then writes in place — clones only on true aliasing, not every write.
Codegen (stmt.rs SetIndex): a nested-Seq base emits `emit_cow(base); base.set_nested(k, i, v)`. Gated
by a precise-type override in the `Let` arm — a `[[…],…]` literal registered only as `LogosSeq<_>`
(imprecise) is re-registered `LogosSeq<LogosSeq<…>>` so the fast path fires (else it fell to the unsound
default arm). splice_fuse.rs now fuses the Set-shape desugar under value semantics too (the through-write
IS the semantics); the Push-shape keeps its desugar. THE soundness gate
(`nested_write_preserves_value_semantics_for_an_alias`: `row = item 1 of grid` stays `[1,2]` after
`grid`'s row is overwritten) is GREEN — the differential compiled==interp oracle is what caught the
original unsound write. Regression: 840 codegen/correctness/collections/corpus tests green.

**Floor division `//` (Wave 6 keystone) — SHIPPED (correctness_floordiv 13/13 + WASM differential + data 1/1).**
A real `BinaryOpKind::FloorDivide` (NOT an alias of `/`): `/` truncates toward zero (`-7 / 2 → -3`), `//`
floors toward NEGATIVE INFINITY (`-7 // 2 → -4`), the universal meaning. Exact (BigInt-promoting), loud on
a zero divisor. Full ~23-site template mirroring the fresh `Pow` precedent:
- Arithmetic core: `LogosInt::div_floor` (trunc then correct by one on a sign-crossing nonzero remainder)
  + `logos_floordiv_exact` (data), exhaustively fuzzed vs an i128 floor oracle over −20..20².
- Lexer `//`→`TokenType::SlashSlash` (two-char, before `/=`; digit-flanked `/` stays a date separator);
  parser multiplicative level (left-assoc, binds like `* / %`); AST enum.
- tw+VM: shared `arith::floor_divide` (Float floors the quotient staying f64; exact operands floor the
  exact rational quotient via `Rational::floor()`) → `Value::floor_div` + `Op::FloorDiv` (compiler/machine/
  disasm/regsplit). VM byte-identical to tw (vm_matches_tw_on_floordiv, all signs + div0 error text).
- AOT: `logos_floordiv_exact` emission + const-fold + Float `(a/b).floor()` (codegen/expr.rs, mirrors Pow).
- C backend: sign-correcting integer formula + `floor()` float (emit.rs).
- **Direct WASM: `lower_floordiv_regs` — hand-emitted `i64.div_s` + `i64.rem_s` + `q - ((r!=0)&((r^b)<0))`
  correction (Int), `f64.floor(a/b)` (Float), unsigned `div_u` (Word); traps on /0 and i64::MIN//-1 like
  Div. PROVEN bit-exact, not asserted: wasm_aot_unit `aot_floordiv_integer_sign_matrix` (10 programs, full
  sign matrix) + `aot_floordiv_float` run through wasmi == tree-walker.** op_support=Supported (KNOWN_DEFERRED
  still empty), kind.rs numeric_join, wasm_aot_lock 4/4.
- verification: `//` modeled EXACTLY (NOT declined). New `VerifyOp::FloorDiv` encoded as
  `to_int(to_real(a) / to_real(b))` — Z3's `to_int` IS floor, precise toward -inf for every sign (the
  Euclidean `VerifyOp::Div` only coincides with floor when the divisor is positive). Wired through ALL six
  verify backends (ir/solver/smtlib(text)/equivalence/kinduction/type_infer) — and filled a pre-existing
  gap where `Div` itself was unsupported in kinduction's integer encoder. Z3 PROVES it:
  phase_verification_floordiv 3/3 (sign-matrix each floor value valid + off-by-one refuted; `7 // -2 == -4`
  floor AND `7 div -2 == -3` Euclidean both proven — so a decline-to-Div would have been UNSOUND; symbolic
  floor bracket `(a//4)*4 <= a < (a//4)*4 + 4` valid ∀a). No regression: verify crate 41/41, integration
  verification 27/27 (phase_verification + refinement + unified).
- LSP: `SlashSlash` highlighted as an operator.
Distinctness locked: `floordiv_is_distinct_from_truncating_divide` (`-7 / 2 → -3` vs `-7 // 2 → -4`).
Immune to the ExactDivide rational rewrite (it's its own op, never rewritten). Default + wasm-jit builds green.

**Bare `is N` equality (Wave 6 keystone) — SHIPPED (correctness_is_equality 14/14).**
`x is 5` is now equality — exactly `x is equal to 5` → `BinaryOpKind::Eq`. Pure parser sugar (one arm in
`parse_comparison`'s `is …` chain, mod.rs), no new AST node: it produces the SAME `Eq` node as the verbose
spelling, so tw/VM/AOT/WASM all get it free (proven by compiled==interp). GUARDED to a NUMBER literal after
`is` (incl. a negated `is -3` via a Minus+Number lookahead) — placed AFTER every worded arm, so `is not`,
`is even`/`odd`, `is at most`/`least`, `is greater/less than`, `is between`, `is before/after`,
`is approximately`, `is equal to`, `is divisible by`, and `is a`/`is an`/`is the`/`is nothing`/`is <predicate>`
are all untouched (nothing but a numeral reaches the new arm). The RHS numeral is left for the existing
`parse_xor_expr` right-operand parse. RED-first: 7 `is N` tests failed / 7 guards passed before, all 14 green
after. No NL collision (this is the imperative `parse_comparison`, `alloc_imperative_expr`; NL/logic parser is
separate). Regression: correctness + e2e_differential 273/273.

**Full-suite gate — 9 PE/supercompile/scalarize goldens updated (user-approved flip).**
A full-suite run surfaced 9 reds, ALL pre-existing / not from the floor-div/is-N/#10 work (none of the 9
programs contain `is`, `//`, a nested-list literal, or a place-write). Root causes + fixes (user signed off on
the flip):
- 8 tests (phase_supercompile identity_arithmetic_program/residual_static_left/right/nested_binary/call_mixed;
  phase_partial_eval pe_body_substituted/pe_fold_runs_on_specialized_body/pe_pipeline_fold_interaction) asserted
  RAW operators in the residual (`"a + b"`, `"3 * "`, `"3 + "`, `"5 *"`). The exact-arithmetic ruling lowers
  `Int +/*` to `logos_add_exact`/`logos_mul_exact`, so the goldens were stale. VERIFIED the PE work is still
  correct (constant inlined: `f_s0_3(b){logos_mul_exact(3,b)}`; fold done: `let c = 5`; `assert_exact_output`
  runtime values unchanged) and updated each string-check to the exact-arith form. 110/110 green.
- 1 test (phase_seq_scalarize no_scalarize_passed_to_function): a SIBLING relaxed the scalarizer to cross a
  READ-ONLY function boundary (`fn total(s: &[i64; 2])`), which the old test forbade. Verified SOUND via
  compile+run (`assert_exact_output` → 10) and renamed/rewrote to
  `scalarize_passed_to_readonly_function_stays_correct` (the genuinely-unsafe push-after-read and
  store-into-Seq cases keep their `assert_not_scalarized` guards). 14/14 green.
Also rode out a transient sibling `Kind::SeqBool` non-exhaustive breakage in vm/wasm/module.rs (7 sites I never
touched; cleared on polling, 14→0).
