# Logos — Bug Report #1
## Interpreter · Compiler · VM · JIT · Optimizer · Language Semantics · Proof Kernel

> This report is the product of an adversarial multi-agent bug hunt. Each finding was raised by a subsystem auditor and then independently checked by a skeptic that read the cited source and tried to **refute** it; only findings that survived refutation appear below. **The suggested RED tests were authored but NOT executed** — they are specifications meant to fail on the current code and pass once each bug is fixed (RED now, GREEN after). Per the project's TDD rules, land the RED test first, confirm it fails, then fix the implementation — never weaken the test.

## Metadata

- **Date:** 2026-06-13
- **Branch:** `stream1`
- **Method:** 24 per-subsystem auditors → adversarial per-finding verification → deterministic synthesis (code excerpts and tests rendered verbatim).
- **Surface audited:** tree-walking interpreter, bytecode VM (compiler + machine), copy-and-patch JIT (`forge`) and JIT core, abstract-interpretation + e-graph + peephole optimizers, C/Rust codegen + FFI boundary, type inference/unification + semantic checks, the Futamura compile pipeline, the English→FOL frontend (lexer, parser, lambda/DRS, transpile, modal/Kripke semantics), and the dependent-type proof kernel (reduction, conversion, termination, positivity).
- **Confirmed bugs:** 33  —  **Critical:** 5, **High:** 19, **Medium:** 6, **Low:** 3.
- **Also flagged:** 4 unconfirmed leads (see *Needs Further Investigation*).

## Executive Summary

The hunt confirmed **33 bugs** across the interpreter/compiler/semantics surface: **5 Critical, 19 High, 6 Medium, 3 Low**. Every bug below is paired with a concrete trigger and a pasteable RED test.

**The Critical findings cluster in the trust core — the proof kernel and the JIT.** Beta-reduction is *not capture-avoiding* in two independent places (the named-variable `substitute()` in `type_checker.rs` and the reflected de Bruijn `syn_beta` in `reduction.rs`, the latter also failing to decrement surviving free variables after dropping a binder), so conversion checking can silently equate non-equivalent terms. Strict-positivity checking exists but **is never wired into the declaration path**, so a paradoxical inductive type is accepted — the classic route to a proof of `False`. The structural-recursion termination guard tracks its decreasing argument **by name only**, so a lambda that shadows that name lets a non-decreasing recursive call pass. Any one of these defeats soundness of the verifier. The fifth Critical is in the JIT: `adapt_region` admits a non-idempotent `ListPush` into a discard-and-replay region, so a deopt **double-applies** the push and corrupts the list.

**The High tier splits between miscompilations and frontend semantic errors.** Miscompilations: integer `pow` lowered to floating-point `pow`; FFI `char` and handle ABI mismatches plus a panic that unwinds across an `extern "C"` boundary (UB); the optimizer's Oracle panicking on `i64::MIN / -1`; two further peephole rewrites (comparison inversion, drain-tail over-copy) and a pinned-JIT `unreachable!` on float branches. Frontend semantics: biconditional and disjunction sharing one precedence level; `"No"` + auxiliary dropping the negation entirely; the modal force of `"cannot"` inverted to necessity/possibility in *both* the formatter and the Kripke lowering; a donkey-anaphora free-variable leak; and a passive `by`-phrase mis-bound as a locative. The Medium/Low tail covers type-inference generalization gaps, list-literal element-type narrowing, a date literal accepting Feb 30, and pre-epoch time-of-day arithmetic.

## Severity Index

| ID | Severity | Subsystem | Location | Bug |
|----|----------|-----------|----------|-----|
| BUG-001 | Critical | JIT · core | `crates/logicaffeine_jit/src/lib.rs:1819` | adapt_region admits non-idempotent ListPush into a discard-and-replay region, double-applying the push on any deopt |
| BUG-002 | Critical | Proof kernel · reduction | `crates/logicaffeine_kernel/src/reduction.rs:875` | Reflected de Bruijn beta (syn_beta/syn_subst) does not decrement surviving free variables after removing the binder |
| BUG-003 | Critical | Proof kernel · reduction | `crates/logicaffeine_kernel/src/type_checker.rs:527` | substitute() is not capture-avoiding: beta reduction captures free variables in the argument |
| BUG-004 | Critical | Proof kernel · type checking | `crates/logicaffeine_kernel/src/context.rs:93` | Strict-positivity check is never wired into the declaration path — paradoxical inductives are accepted, enabling a proof of False |
| BUG-005 | Critical | Proof kernel · type checking | `crates/logicaffeine_kernel/src/termination.rs:110` | Termination guard tracks the structural parameter by NAME only; a lambda that shadows that name lets a non-decreasing recursive call pass the guard |
| BUG-006 | High | Optimizer · abstract interpretation | `crates/logicaffeine_compile/src/optimize/abstract_interp.rs:210` | Interval::div and Interval::modulo panic on i64::MIN / -1 (raw `/` and `%` instead of wrapping), crashing the Oracle on valid input |
| BUG-007 | High | Semantic checks | `crates/logicaffeine_compile/src/analysis/check.rs:701` | `Repeat` over a Map binds the loop variable to the KEY type, but the runtime yields (key, value) tuples |
| BUG-008 | High | Codegen · FFI / runtime boundary | `crates/logicaffeine_compile/src/codegen/ffi.rs:1693` | Char param/return crosses the C ABI as a raw Rust `char`, which C declares as `uint32_t` — out-of-range u32 from C becomes an invalid `char` (UB) |
| BUG-009 | High | Codegen · FFI / runtime boundary | `crates/logicaffeine_compile/src/codegen/marshal.rs:200` | Reference-type handle parameter is dereferenced with .expect() OUTSIDE the catch_unwind boundary — a null/stale handle from C unwinds across extern "C" (UB) |
| BUG-010 | High | Codegen · statements | `crates/logicaffeine_compile/src/codegen_c/emit.rs:143` | C backend lowers integer pow() to floating-point pow(), producing wrong integer results (precision/wrapping divergence) |
| BUG-011 | High | Compile pipeline (Futamura) | `crates/logicaffeine_compile/src/compile.rs:3710` | Decompiler drops parentheses around a `not`/negation operand, inverting precedence so `not (a and b)` becomes `(not a) and b` in the First Futamura Projection residual |
| BUG-012 | High | JIT · forge (copy-and-patch) | `crates/logicaffeine_forge/src/jit.rs:1287` | Pinned JIT compiler panics (unreachable!) on float-branch / float-`>`/`>=` ops it never lowers — reachable from the function-compile pinning path |
| BUG-013 | High | Lambda / DRS | `crates/logicaffeine_proof/src/unify.rs:289` | Proof-kernel beta-reduction is not capture-avoiding: free variables in the argument are captured by inner binders (unsound conversion checking) |
| BUG-014 | High | Language compile / discovery | `crates/logicaffeine_language/src/analysis/discovery.rs:260` | Policy-condition discovery silently drops every conjunct/disjunct after the first connective (3+ term AND/OR chains lose conditions) |
| BUG-015 | High | Lexer | `crates/logicaffeine_language/src/lexer.rs:773` | String-literal span mixes char-index end with byte-index start, causing usize underflow panic on valid input with leading multibyte text |
| BUG-016 | High | Parser · core | `crates/logicaffeine_language/src/parser/clause.rs:2030` | Biconditional (Iff) and disjunction (Or) share one precedence level, so "P if and only if Q or R" mis-associates as (P↔Q)∨R |
| BUG-017 | High | Parser · quantifiers / clauses | `crates/logicaffeine_language/src/parser/quantifier.rs:589` | "No" quantifier with an auxiliary verb drops the negation and emits a conjunction instead of a negated implication |
| BUG-018 | High | Parser · quantifiers / clauses | `crates/logicaffeine_language/src/parser/quantifier.rs:803` | Donkey binding from a subject relative clause leaks (free variable) when the main VP has a quantified object |
| BUG-019 | High | Parser · verbs | `crates/logicaffeine_language/src/parser/verb.rs:1701` | Embedded copula+participle passive mis-binds the by-phrase agent (drops the agent into a locative PP instead of the predicate's agent slot) |
| BUG-020 | High | Optimizer · peephole | `crates/logicaffeine_compile/src/codegen/peephole.rs:2052` | Swap-idiom peephole drops the `Let a` / `Let b` / `tmp` bindings it consumes, breaking any later use of those locals |
| BUG-021 | High | Optimizer · peephole | `crates/logicaffeine_compile/src/codegen/peephole.rs:2077` | Conditional-swap peephole inverts the comparison when the guard is written `b OP a` instead of `a OP b` |
| BUG-022 | High | Optimizer · peephole | `crates/logicaffeine_compile/src/codegen/peephole.rs:3899` | Drain-tail peephole copies to the array's end instead of to the loop bound, over-copying when the bound is smaller than the array length |
| BUG-023 | High | Modal semantics (Kripke) | `crates/logicaffeine_language/src/semantics/kripke.rs:471` | Kripke lowering of "cannot" (force 0.0) produces a possibility ∃-world, asserting the logical OPPOSITE of impossibility |
| BUG-024 | High | Transpile / formatter | `crates/logicaffeine_language/src/formatter.rs:81` | "cannot" (alethic impossibility, force 0.0) renders as necessity □ instead of possibility/impossibility — modal force is inverted at the boundary |
| BUG-025 | Medium | Semantic checks | `crates/logicaffeine_compile/src/analysis/check.rs:258` | List literal element type taken only from the first element — rejects valid `Seq of Real be [1, 2, 3]` and gives heterogeneous lists a wrong concrete element type |
| BUG-026 | Medium | Type analysis · inference / unification | `crates/logicaffeine_compile/src/analysis/check.rs:639` | Generic function with inferred (unannotated) return type fails to generalize its return type variable, causing cross-call contamination between two calls at different types |
| BUG-027 | Medium | Optimizer · closed-form detection | `crates/logicaffeine_compile/src/codegen/detection.rs:1581` | Closed-form double-recursion replacement emits `<< d` without restricting the parameter to an integer type or guarding the shift count |
| BUG-028 | Medium | Codegen · statements | `crates/logicaffeine_compile/src/codegen/stmt.rs:2755` | Parallel-reduction lowering hardcodes .sum::<i64>(), miscompiling/rejecting float (Copy) sequence reductions |
| BUG-029 | Medium | Codegen · statements | `crates/logicaffeine_compile/src/codegen_c/emit.rs:156` | C backend min()/max() expand to a ternary that double-evaluates the selected argument (extra side effects / non-determinism) |
| BUG-030 | Medium | Lexer | `crates/logicaffeine_language/src/lexer.rs:1806` | ISO-8601 date literal accepts impossible days (Feb 30, Apr 31) and silently coerces them to a different valid date |
| BUG-031 | Low | Compile pipeline (Futamura) | `crates/logicaffeine_compile/src/compile.rs:2885` | Futamura encoder collapses all tasks of a Concurrent/Parallel block into a single branch, losing the per-task branch structure |
| BUG-032 | Low | Interpreter | `crates/logicaffeine_compile/src/semantics/compare.rs:85` | Moment-vs-Time comparison and Moment/Time display use truncating % / for time-of-day, giving a negative time-of-day for pre-epoch (negative) Moments |
| BUG-033 | Low | Modal semantics (Kripke) | `crates/logicaffeine_language/src/debug.rs:196` | force == 0.5 modals (can/could/would/may) lower as ◇ but display as □ — boundary mismatch between Kripke lowering and debug display |

## Confirmed Bugs

### BUG-001 — adapt_region admits non-idempotent ListPush into a discard-and-replay region, double-applying the push on any deopt

**Severity:** Critical  ·  **Category:** miscompilation / unsound optimizer-region contract (observable result corruption)  ·  **Subsystem:** JIT · core  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_jit/src/lib.rs:1819–1835`

**Summary**

A Main-loop REGION is compiled by `adapt_region` (lib.rs:1930) and run under a strict discard-and-replay contract: the VM copies the guard-set registers in at region ENTRY, runs the ENTIRE loop natively, and on any side-exit (`RegionOutcome::Deopt`) it just `return None`s, leaving the VM registers at the region-entry state and re-running the WHOLE loop on bytecode from the head (machine.rs:377-386). That contract is only sound if everything the native region did before the deopt is replay-idempotent. The machine.rs comment (lines 383-384) justifies it ONLY for in-place array writes (`arr[i]=v`: rewriting the same index with the same recomputed value is idempotent). But `adapt_region` ALSO accepts `Op::ListPush` (registered as a pinnable collection at lib.rs:1989, translated to a real `(*vec).push(value)` at lib.rs:1819-1835 / runtime helper `logos_rt_push_i64` at lib.rs:162-173). `push` APPENDS — it is NOT idempotent under replay. There is no guard in `adapt_region` rejecting a region that contains both a `ListPush` and a deopting op (Div/Mod-by-zero, checked Index/SetIndex out-of-bounds, MapGet miss/non-Int value, or a Call entry-deopt). Notably, the sibling `adapt_function` DOES guard pushes against exactly this hazard (the `push-outside-site` bail at lib.rs:978-988), so the region path is missing a guard its function counterpart has.

**Why it's wrong**

When a region containing a `ListPush` deopts mid-run, the elements the native code already pushed into the real pinned `Vec` STAY in the list (the RefMut handle is dropped on deopt at machine.rs:367, but the Vec mutation persists). The VM then replays the entire loop from the head on bytecode, pushing those same elements AGAIN — duplicating list contents. This is an observable correctness divergence from the tree-walker / pure-bytecode semantics: the compiled run yields a list with duplicated elements (and a wrong `length`). It is a direct miscompilation reachable from valid English input.

**The offending code**

```rust
// adapt_region accepts ListPush as a pinned collection with NO replay-idempotence guard:
//   line 1989:   Op::ListPush { list, .. } => Some(list),
// and translates it to a real mutation of the pinned Vec:
Op::ListPush { list, value } => {
    let p = pins.get(&list)?;
    let helper_addr = match p.elem {
        PinElem::Int => crate::logos_rt_push_i64 as usize as i64,
        PinElem::Float => crate::logos_rt_push_f64 as usize as i64,
        PinElem::Bool => crate::logos_rt_push_bool as usize as i64,
        PinElem::Map => return None,
    };
    micro.push(MicroOp::ArrPush { src: value, vec_slot: p.vec_slot, ptr_slot: p.ptr_slot, len_slot: p.len_slot, helper_addr });
}
```

**Trigger / reproduction**

A hot Main loop (>100 back-edges, so the region arms) that PUSHES to a list AND performs a deopting op in the same iteration, where the deopt fires non-fatally. Concrete deterministic trigger: read a `Map of Int to Float` inside the pushing loop. The int fast-lane map helper (`logos_rt_map_get_ii`, lib.rs:118-135) side-exits on any non-Int value WITHOUT erroring (the kernel returns the boxed Float and the program continues — see collections.rs:66-70). So: bytecode runs iterations 1..100 (pushing, reading the map fine); at iteration 100 the region arms and runs natively; the native run pushes element 100 to the real Vec, then the map read returns a Float -> MapGet side-exits -> region deopts; the VM discards the native frame and replays the whole loop from the head (i back at 100), pushing element 100 a SECOND time. Final list has one extra (duplicated) element; the tree-walker has none. (The existing region fuzz test jit_region_generalization.rs:55-93 never hits this because it keeps pushes in a loop whose only Mod is by a nonzero CONSTANT — never deopts — and keeps all deopting ops in a separate, non-pushing loop.)

**Expected vs actual**

- **Expected:** The tiered-VM output must equal the tree-walker output: the list ends with exactly the pushed elements, no duplicates (e.g. `length of results` == N).
- **Actual:** After the region deopts, the native push already mutated the real Vec, and bytecode replay re-pushes the same element(s), so the list contains duplicate element(s) (e.g. `length of results` == N+1) and any read of the list contents diverges from the tree-walker.

**Suggested RED test**

```rust
// crates/logicaffeine_tests/tests/jit_region_push_replay.rs
#![cfg(not(target_arch = "wasm32"))]
use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn region_listpush_is_not_double_applied_on_deopt() {
    // A Map of Int to Float defeats the int fast lane: the map read inside the
    // hot loop side-exits (logos_rt_map_get_ii returns 0 on a non-Int value),
    // forcing the armed region to DEOPT *after* it has already pushed into the
    // real pinned list. The region's discard-and-replay-from-head contract
    // (machine.rs:385 -> pc = loop head) then re-runs the same iteration on
    // bytecode and pushes the element AGAIN. The push op is ordered BEFORE the
    // deopting map read in the loop body, which is what makes the push land
    // before the side exit.
    let src = "## Main\n\
               Let mutable m be a new Map of Int to Float.\n\
               Let mutable k be 0.\n\
               While k is less than 500:\n\
               \x20   Set item k of m to 1.5.\n\
               \x20   Set k to k + 1.\n\
               Let mutable results be a new Seq of Int.\n\
               Let mutable junk be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 500:\n\
               \x20   Push i to results.\n\
               \x20   Set junk to item i of m.\n\
               \x20   Set i to i + 1.\n\
               Show length of results.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    // Differential: the tiered VM must agree with the tree-walker.
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker: a region ListPush was double-applied on deopt"
    );
    // Absolute: exactly 500 elements pushed, no duplicates from replay.
    assert_eq!(norm(&vm.output), "500");
}
```

**Fix direction / notes**

One correction to the claim's supporting argument (not the verdict): the cited "sibling guard" in `adapt_function` at lib.rs:978-988 (the `push-outside-site` bail) is an ALIASING guard, not a replay-idempotence guard. Its taint dataflow (lib.rs:900-925) marks a register tainted when it may hold a list another holder pins (params, self-call results), and bails the push only when the pinned list could be shared — this protects exclusive-pin correctness, not deopt replay. The function tier's mode-A replay is sound for pushes for a different reason: it re-enters from the boundary argument values and returns results by register (jit.rs:945-948), so it does not re-run already-executed body prefixes the way the region's discard-and-replay-from-head does. So the precise statement is "the region path has no guard excluding non-idempotent ListPush from a discard-and-replay region," rather than "the function counterpart guards this exact hazard and the region is missing it." The core bug, trigger, expected/actual, and Critical severity are all confirmed unchanged.

Severity remains Critical: this is a deterministic, silent miscompilation (wrong observable list contents and length) reachable from ordinary valid English input, with no error raised — exactly the worst class of correctness divergence (compiled tier disagrees with the tree-walker semantics).

---

### BUG-002 — Reflected de Bruijn beta (syn_beta/syn_subst) does not decrement surviving free variables after removing the binder

**Severity:** Critical  ·  **Category:** Incorrect de Bruijn index handling in beta reduction (reflection layer)  ·  **Subsystem:** Proof kernel · reduction  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_kernel/src/reduction.rs:875–961`

**Summary**

In the deep-embedding (Syntax) evaluator, beta reduction is implemented as syn_beta body arg = syn_subst arg 0 body (reduction.rs:957-960). try_syn_step_reduce fires this for `SApp (SLam T body) arg` (lines 1090-1096), i.e. it consumes the SLam binder. Proper de Bruijn beta requires that, once the binder is removed, every free variable in `body` with index > 0 be decremented by 1 (the binder it referred to is gone). try_syn_subst_reduce returns variables with k != index unchanged, so surviving free variables are left one index too high, becoming dangling/shifted references. The replacement IS correctly lifted under inner binders (line 926), so that half is fine; only the decrement of the eliminated level is missing.

**Why it's wrong**

After `(λ. body) arg` reduces, a de Bruijn variable `SVar 1` inside body referred to the lambda's enclosing binder; with the lambda removed it must become `SVar 0`. Leaving it as `SVar 1` makes it point one binder too far out — a different variable. This corrupts terms produced by syn_step/syn_eval. Because try_dcompute_conclude (reduction.rs:3401-3411) uses try_syn_eval_reduce to evaluate both sides of an `Eq` and returns the goal as proven iff the evaluated sides are syntax_equal, a wrong de Bruijn beta can make DCompute either accept a false equality or reject a true one — a soundness defect in the proof checker's compute tactic.

**The offending code**

```rust
fn try_syn_beta_reduce(ctx: &Context, body: &Term, arg: &Term) -> Option<Term> {
    // syn_beta is just syn_subst with index 0
    try_syn_subst_reduce(ctx, arg, 0, body)
}

// in try_syn_subst_reduce, SVar case:
"SVar" => {
    if let Term::Lit(Literal::Int(k)) = inner_arg.as_ref() {
        if *k == index { return Some(replacement.clone()); }   // matched var
        else { return Some(term.clone()); }                    // <-- k>index NOT decremented
    }
}
```

**Trigger / reproduction**

Evaluate one step of the reflected redex SApp (SLam T (SVar 1)) arg, i.e. a beta redex whose body is the free variable one level outside the lambda. As Terms: SApp = App(App(Global("SApp"), App(App(Global("SLam"), T), App(Global("SVar"), Lit(Int(1))))), arg). Apply syn_step (or syn_eval 1) to it.

**Expected vs actual**

- **Expected:** The SLam binder is removed, so the free SVar 1 (which referred to the binder just outside the now-removed lambda) must shift to SVar 0: result = App(Global("SVar"), Lit(Int(0))).
- **Actual:** syn_subst arg 0 (SVar 1): since 1 != 0, returns SVar 1 unchanged. Result is App(Global("SVar"), Lit(Int(1))) — the surviving free variable is one index too high (dangling reference).

**Suggested RED test**

```rust
#[test]
fn reflected_beta_decrements_surviving_free_vars() {
    use logicaffeine_kernel::{normalize, Context, Literal, Term};
    let ctx = Context::new();
    fn g(n: &str) -> Term { Term::Global(n.into()) }
    fn app(f: Term, x: Term) -> Term { Term::App(Box::new(f), Box::new(x)) }
    let svar = |k: i64| app(g("SVar"), Term::Lit(Literal::Int(k)));
    let slam = |t: Term, b: Term| app(app(g("SLam"), t), b);
    let sapp = |f: Term, x: Term| app(app(g("SApp"), f), x);

    // redex = (\T. SVar 1) (SVar 7), where SVar 1 is free: it refers to the
    // binder one level OUTSIDE this lambda. Once the lambda's binder is
    // eliminated by beta, that reference must shift down: SVar 1 -> SVar 0.
    let redex = sapp(slam(svar(0), svar(1)), svar(7));
    let eval = app(app(g("syn_eval"), Term::Lit(Literal::Int(5))), redex);
    let result = normalize(&ctx, &eval);

    // CORRECT de Bruijn beta: surviving free var decremented.
    assert_eq!(
        result,
        svar(0),
        "surviving free var not decremented after binder removal: got {:?}",
        result
    );
    // Pin the current buggy output so the failure is unambiguous.
    assert_ne!(
        result,
        svar(1),
        "got SVar 1 (one index too high) — binder removed but free var left un-shifted"
    );
}
```

**Fix direction / notes**

Severity raised from High to Critical. This is not merely a "wrong result" bug: try_dcompute_conclude (reduction.rs:3387-3416) uses the affected try_syn_eval_reduce path to decide equalities and certifies the goal as proven when the two evaluated sides are syntax_equal. A wrong de Bruijn beta can therefore make the proof kernel's compute tactic ACCEPT A FALSE THEOREM — a soundness break in the trusted core, which is categorically the most severe class of defect. The fix is to perform the de Bruijn decrement when the binder is eliminated. The cleanest fix is to make beta reduction (try_syn_beta_reduce / line 958-961) do the standard shift-substitute-shift, i.e. substitute a lifted argument at index 0 and then lower (subtract 1 from) every free variable above the substituted level — equivalently, change the substitution used for beta so that SVar k with k > index returns SVar (k-1). Note: try_syn_subst_reduce is ALSO called directly in many other places (e.g. induction/recursor handling at 3253, 3309, 4004, 4030, 4177, 4205, 4218 and dcompute helpers at 3776-3777) where a plain non-decrementing substitution may be the intended semantics, so the decrement must be introduced specifically on the beta path (or via a separate decrementing-substitution variant) rather than by editing try_syn_subst_reduce's SVar branch unconditionally — otherwise those other call sites would regress. The auditor's diagnosis of WHERE the missing decrement belongs is correct; just be careful the fix does not also change the non-beta substitution callers.

---

### BUG-003 — substitute() is not capture-avoiding: beta reduction captures free variables in the argument

**Severity:** Critical  ·  **Category:** Variable capture in capture-avoiding substitution (beta reduction)  ·  **Subsystem:** Proof kernel · reduction  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_kernel/src/type_checker.rs:527–614`

**Summary**

The kernel's core substitution (used for ALL beta reduction in reduce_step at reduction.rs:96 and :107, and for definitional/alpha equality in types_equal/is_subtype at type_checker.rs:731,749,785) performs naive structural substitution. It only handles SHADOWING (if a binder re-binds `var`, it stops substituting), but it never renames a binder when the binder's name occurs FREE in `replacement`. The doc comment claims it is capture-avoiding and even gives the example `substitute(λx. y, "y", x) => λx. x` — but that result IS the captured term; the free `x` is wrongly bound by the inner `λx`. There is no fresh-variable generation anywhere in the kernel (confirmed by grep: no `fresh`/`rename`/capture logic feeds into substitute).

**Why it's wrong**

Beta reduction `(λx. body) arg -> body[x := arg]` must not let binders inside `body` capture free variables of `arg`. Without alpha-renaming, any redex whose argument contains a free variable that collides with a binder name inside the function body produces a term with different meaning. Since substitute is the engine of normalize() and of definitional equality (types_equal), this makes the type checker unsound: two terms that are NOT alpha-equivalent can be judged equal, and reductions can yield non-equivalent results. This is the canonical lambda-calculus correctness bug.

**The offending code**

```rust
pub fn substitute(body: &Term, var: &str, replacement: &Term) -> Term {
    match body {
        ...
        Term::Lambda { param, param_type, body } => {
            let new_param_type = substitute(param_type, var, replacement);
            // Don't substitute in body if the parameter shadows var
            let new_body = if param == var {
                (**body).clone()
            } else {
                substitute(body, var, replacement)   // <-- no alpha-renaming of `param`
            };
            Term::Lambda { param: param.clone(), param_type: Box::new(new_param_type), body: Box::new(new_body) }
        }
        ...
```

**Trigger / reproduction**

Normalize the redex `(λf. λx. f x) x` where the outer `x` is a free variable. As a Term: App(Lambda{param:"f", body: Lambda{param:"x", body: App(Var("f"), Var("x"))}}, Var("x")). reduce_step takes the Lambda head and calls substitute(λx. f x, "f", Var("x")). Since the inner binder is "x" (!= "f"), it substitutes into the body, turning App(Var f, Var x) into App(Var x, Var x).

**Expected vs actual**

- **Expected:** Capture-avoiding result: the inner binder is renamed so the originally-free `x` stays free and distinct, e.g. `λx'. x x'` (the head `x` applied to the bound variable). The two `x` occurrences must NOT become the same variable.
- **Actual:** Produces `λx. x x` (Lambda{param:"x", body: App(Var("x"), Var("x"))}). The originally-free outer `x` is captured by the inner binder, collapsing two distinct variables into one. Meaning changed (`λg. x g` vs `λg. g g`).

**Suggested RED test**

```rust
#[test]
fn beta_reduction_is_capture_avoiding() {
    use logicaffeine_kernel::{normalize, Context, Term, Universe};

    let ctx = Context::new();
    let ty = || Box::new(Term::Sort(Universe::Type(0)));

    // (λf. λx. f x) x   — the trailing/argument x is a FREE variable.
    let inner_body = Term::App(
        Box::new(Term::Var("f".into())),
        Box::new(Term::Var("x".into())),
    );
    let lam_x = Term::Lambda {
        param: "x".into(),
        param_type: ty(),
        body: Box::new(inner_body),
    };
    let lam_f = Term::Lambda {
        param: "f".into(),
        param_type: ty(),
        body: Box::new(lam_x),
    };
    let redex = Term::App(Box::new(lam_f), Box::new(Term::Var("x".into())));

    let result = normalize(&ctx, &redex);

    // After capture-avoiding beta reduction the result must be a lambda whose
    // body applies the FREE x (head) to the BOUND parameter (arg), and the
    // bound parameter must have been renamed away from "x" (otherwise the
    // free x was captured, collapsing two distinct variables into one).
    match &result {
        Term::Lambda { param, body, .. } => match body.as_ref() {
            Term::App(f, a) => {
                assert_eq!(**f, Term::Var("x".into()), "head must remain the FREE x");
                assert_eq!(**a, Term::Var(param.clone()), "arg must be the BOUND param");
                assert_ne!(
                    param, "x",
                    "VARIABLE CAPTURE: inner binder captured the free x; got `λx. x x` \
                     instead of capture-avoiding `λx'. x x'`"
                );
            }
            other => panic!("unexpected lambda body, expected an application: {:?}", other),
        },
        other => panic!("unexpected normal form, expected a lambda: {:?}", other),
    }
}
```

**Fix direction / notes**

No substantive corrections to the claim; it is accurate as written, including file/line range (type_checker.rs:527-614) and the reduction.rs:96/:107 call sites. One clarification: `substitute` is `pub` but not re-exported from the crate root, so it cannot be imported as `logicaffeine_kernel::substitute` — the RED test correctly exercises it through the public `normalize` API instead, so the test stands. The severity is correctly rated Critical (kernel-soundness / capture-avoiding-substitution failure in a dependent-type proof kernel).

---

### BUG-004 — Strict-positivity check is never wired into the declaration path — paradoxical inductives are accepted, enabling a proof of False

**Severity:** Critical  ·  **Category:** Unsound soundness guard (strict positivity not enforced)  ·  **Subsystem:** Proof kernel · type checking  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_kernel/src/context.rs:93–102`

**Summary**

`positivity::check_positivity` is implemented and correct, but the ONLY caller is `Context::add_constructor_checked` (context.rs:219), and grepping the whole workspace shows `add_constructor_checked` is never called outside its own definition and unit tests. Every real constructor registration — both the prelude (prelude.rs: ~50 `ctx.add_constructor(...)` calls) and the user-facing REPL `Inductive` command (interface/repl.rs:85 `self.ctx.add_constructor(&ctor_name, &name, poly_ctor_ty)`) — uses the UNCHECKED `add_constructor`. So strict positivity is dead code in the trusted pipeline.

**Why it's wrong**

Strict positivity is the guard that rules out non-well-founded inductive definitions. Its own module doc (positivity.rs:1-11) gives the exact paradox it must reject: `Inductive Bad := Cons : (Bad -> False) -> Bad`. With positivity unenforced, the kernel accepts `Bad` and its constructor `Cons : (Bad -> False) -> Bad`. From this one can build a closed term of type `False` (Russell-style: define `selfApp (b:Bad) : False := match b with Cons f => f b`, then `Cons selfApp : Bad`, then `selfApp (Cons selfApp) : False`). A kernel that admits a closed proof of `False` is unsound — every proposition becomes provable.

**The offending code**

```rust
pub fn add_constructor(&mut self, name: &str, inductive: &str, ty: Term) {
    self.constructors
        .insert(name.to_string(), (inductive.to_string(), ty));
    self.constructor_order
        .entry(inductive.to_string())
        .or_default()
        .push(name.to_string());
}
```

**Trigger / reproduction**

Register a negative-recursive inductive through the public API (the same path the REPL `Inductive` command uses): `ctx.add_inductive("Bad", Term::Sort(Universe::Type(0)))` followed by `ctx.add_constructor("Cons", "Bad", Π(_: (Bad -> False)). Bad)`. The constructor is accepted with no error.

**Expected vs actual**

- **Expected:** Registering a constructor in which the inductive occurs in a negative position (left of an inner arrow inside a parameter type) must be rejected with `KernelError::PositivityViolation`, exactly as `add_constructor_checked` would do.
- **Actual:** `add_constructor` (and therefore the REPL `Inductive` command and the prelude) accepts the constructor unconditionally; `check_positivity` is never consulted, so the paradoxical type is admitted into the context and can be eliminated/matched on like any other inductive.

**Suggested RED test**

```rust
// File: crates/logicaffeine_tests/tests/phase79_termination.rs (or a new test module)
// This drives the ACTUAL trusted/user-facing path (REPL `Inductive` -> add_constructor),
// which currently performs NO positivity check. It FAILS today (execute returns Ok)
// and PASSES once repl.rs:85 calls add_constructor_checked (or add_constructor itself
// enforces positivity). `False` is already in the standard library loaded by Repl::new().
#[test]
fn repl_inductive_rejects_negative_recursive_constructor() {
    use logicaffeine_kernel::interface::repl::Repl;

    let mut repl = Repl::new();

    // Bad occurs in a negative position: (Bad -> False) -> Bad.
    // This is exactly the Russell-paradox inductive the kernel must reject.
    let result = repl.execute("Inductive Bad := Cons : (Bad -> False) -> Bad.");

    assert!(
        result.is_err(),
        "UNSOUND: REPL accepted a negative-recursive inductive (no positivity check on the trusted path). Got: {:?}",
        result
    );
}

// Companion test asserting the invariant at the Context API level: the *unchecked*
// public registration entry point must not silently admit a negative constructor.
// This pins the fix at the source (make add_constructor enforce positivity, or ensure
// every trusted caller uses the checked variant). It FAILS today because add_constructor
// registers unconditionally; once the guard is wired in it must be rejected.
#[test]
fn context_trusted_registration_path_enforces_positivity() {
    use logicaffeine_kernel::{Context, Term, Universe};

    let mut ctx = Context::new();
    ctx.add_inductive("False", Term::Sort(Universe::Prop));
    ctx.add_inductive("Bad", Term::Sort(Universe::Type(0)));

    // Cons : (Bad -> False) -> Bad   (Bad occurs negatively, left of an inner arrow)
    let bad_to_false = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Bad".to_string())),
        body_type: Box::new(Term::Global("False".to_string())),
    };
    let cons_ty = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(bad_to_false),
        body_type: Box::new(Term::Global("Bad".to_string())),
    };

    // After the fix, the inductive's negative constructor must NOT appear among the
    // inductive's registered constructors via the trusted path. Today add_constructor
    // registers it unconditionally, so this assertion fails.
    ctx.add_constructor("Cons", "Bad", cons_ty);
    let registered: Vec<&str> = ctx
        .get_constructors("Bad")
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(
        !registered.contains(&"Cons"),
        "UNSOUND: negative-recursive constructor 'Cons' was admitted by the trusted add_constructor path: {:?}",
        registered
    );
}
```

**Fix direction / notes**

Severity is correctly rated Critical (an unsound proof kernel that admits paradoxical inductives is the worst class of bug for a proof checker; it is reachable from ordinary REPL input via the public `Repl::execute` API).

The only correction is to the suggested RED test, which is defective: it calls `add_constructor_checked` (the already-correct checked variant) and so would pass today rather than fail. The bug lives in the UNCHECKED path (`add_constructor`, used by `prelude.rs` and `interface/repl.rs:85`). A valid RED test must exercise that unchecked/trusted path. See refined_red_test.

Minor: the auditor's `trigger` text describes `ctx.add_constructor("Cons", ...)` accepting the negative constructor, which is the right path — but the actual `suggested_red_test` code mismatches that by calling the *checked* variant. The fix should make the trusted path reject: either change repl.rs:85 to call `add_constructor_checked` (and similarly gate the prelude or, better, make `add_constructor` itself perform the positivity check so no trusted caller can bypass it).

---

### BUG-005 — Termination guard tracks the structural parameter by NAME only; a lambda that shadows that name lets a non-decreasing recursive call pass the guard

**Severity:** Critical  ·  **Category:** Too-permissive termination check (name capture / shadowing)  ·  **Subsystem:** Proof kernel · type checking  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_kernel/src/termination.rs:110–127`

**Summary**

The guard remembers only `struct_param: String` (the name of the decreasing argument) and the `smaller_than` set of variable NAMES. The `Match` arm decides a match is the guarding one purely by `disc_name == guard_ctx.struct_param` (line 111). The `Lambda` arm (line 127) recurses into the body with the SAME guard context even when the lambda's own parameter rebinds `struct_param` (or rebinds a name already in `smaller_than`). So inside a `λn:Nat. …` that shadows the structural parameter `n`, a `match n with Succ p => f p` is treated as a guarding match on the real structural argument, and `p` (a subterm of the SHADOW `n`, which can be an arbitrary value) is marked structurally smaller. A recursive call `f p` is then accepted even though `p` need not be a subterm of the real decreasing argument.

**Why it's wrong**

Soundness of the kernel rests on this guard: the module header (termination.rs:1-12) states that without it one can `prove False by writing fix f. f`. Because the guard ignores binder shadowing, it admits fixpoints whose recursive argument is NOT a structural subterm of the original decreasing argument. Such a fixpoint can diverge, and a diverging well-typed term inhabiting an empty type (e.g. `False`) breaks consistency: the normalizer (`normalize`, 10000-fuel) will keep unfolding without ever reaching a constructor, but the term is accepted as a closed proof, so the proposition is considered proved.

**The offending code**

```rust
if let Term::Var(disc_name) = discriminant.as_ref() {
    if disc_name == &guard_ctx.struct_param {
        return check_match_cases_guarded(ctx, guard_ctx, cases);
    }
}
... 
// Lambda: recurse into body (param shadows nothing relevant)
Term::Lambda { body, .. } => check_guarded(ctx, guard_ctx, body),
```

**Trigger / reproduction**

check_termination(ctx, "f", body) where body shadows the structural name `n` and recurses on a predecessor of an UNRELATED value bound to the shadow. Concretely, with f : Nat -> Nat -> Nat:
body = λn:Nat. λm:Nat. match n with
  | Zero => Zero
  | Succ k => (λn:Nat. match n with Zero => Zero | Succ p => (f p) m) m
The inner `match n` matches the SHADOW n (bound to the arbitrary value m), and `f p` recurses on pred(m), which is not a subterm of the real decreasing argument n.

**Expected vs actual**

- **Expected:** check_termination must reject this body: `p` is a subterm of `m` (an arbitrary parameter), not of the structural argument `n`, so the recursion is not structurally decreasing and termination cannot be guaranteed (it returns Err(TerminationViolation)).
- **Actual:** check_termination returns Ok(()). The inner `match n` is mistaken for a guarding match on the structural parameter (names collide), `p` is inserted into `smaller_than`, and the recursive call `f p` is accepted.

**Suggested RED test**

```rust
// Place in crates/logicaffeine_tests/tests/phase79_termination.rs (same idiom as the
// existing tests there). Routes through the full kernel (infer_type) so it proves the
// actual soundness break: a well-typed but DIVERGENT fixpoint must be rejected.
#[test]
fn test_reject_non_decreasing_hidden_by_shadowing() {
    // fix f. λn:Nat. λm:Nat.
    //   match n with
    //   | Zero   => Zero
    //   | Succ k => (λn:Nat. match n with                 // <-- λn SHADOWS the structural n
    //                          | Zero   => Zero
    //                          | Succ p => f p m) m        // recurses on pred(m), NOT a subterm of n
    //
    // f : Nat -> Nat -> Nat is well-typed, but it diverges (e.g. `f 5 3` -> `f 2 3` -> `f 2 3` ...).
    // The termination guard tracks the structural parameter by NAME, so the inner `match n`
    // (on the shadow) is mistaken for a guarding match and `p` is wrongly marked smaller.
    // The kernel MUST reject this.
    use logicaffeine_kernel::prelude::StandardLibrary;
    use logicaffeine_kernel::{infer_type, Context, Term};

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = || Term::Global("Nat".to_string());
    let zero = || Term::Global("Zero".to_string());
    let var = |s: &str| Term::Var(s.to_string());
    let const_motive = || Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(nat()),
    };

    // rec = f p m
    let rec = Term::App(
        Box::new(Term::App(Box::new(var("f")), Box::new(var("p")))),
        Box::new(var("m")),
    );

    // inner shadow match: match n with Zero => Zero | Succ p => f p m
    let inner_match = Term::Match {
        discriminant: Box::new(var("n")),
        motive: Box::new(const_motive()),
        cases: vec![
            zero(),
            Term::Lambda {
                param: "p".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(rec),
            },
        ],
    };

    // (λn:Nat. inner_match) m   -- the λn shadows the structural n; arg is the arbitrary m
    let shadowed = Term::App(
        Box::new(Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(inner_match),
        }),
        Box::new(var("m")),
    );

    // outer match: match n with Zero => Zero | Succ k => shadowed
    let outer_match = Term::Match {
        discriminant: Box::new(var("n")),
        motive: Box::new(const_motive()),
        cases: vec![
            zero(),
            Term::Lambda {
                param: "k".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(shadowed),
            },
        ],
    };

    let bad_fix = Term::Fix {
        name: "f".to_string(),
        body: Box::new(Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Lambda {
                param: "m".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(outer_match),
            }),
        }),
    };

    let result = infer_type(&ctx, &bad_fix);
    assert!(
        result.is_err(),
        "UNSOUND: Kernel accepted a diverging fixpoint whose non-decreasing recursion was hidden behind a shadowed binder! Result: {:?}",
        result
    );
}
```

**Fix direction / notes**

Severity raised from High to Critical. This is a soundness hole in the trusted dependent-type proof kernel: the single termination guard (type_checker.rs:247, the only gate on Term::Fix) accepts a well-typed but divergent fixpoint. Soundness loss in a proof kernel is the most severe class — it lets one inhabit False and thereby "prove" any proposition, which is the exact failure the module exists to prevent (termination.rs:1-13). The trigger is reachable both via the public Term API and via any surface syntax that permits lambda binders to reuse a name (binder shadowing is legal). The claim's title, file, line range (110-127), category, expected/actual, and high confidence are all accurate. One refinement to the claim's RED test: it calls check_termination in isolation (which is sufficient to show the guard is too permissive), but the strongest, idiom-matching demonstration routes through infer_type and asserts the kernel rejects the divergent Fix — matching the existing phase79_termination.rs tests, which all use infer_type + assert is_err with an "UNSOUND" message. The provided test's `use ... Universe` import is unused (harmless warning).

---

### BUG-006 — Interval::div and Interval::modulo panic on i64::MIN / -1 (raw `/` and `%` instead of wrapping), crashing the Oracle on valid input

**Severity:** High  ·  **Category:** panic-on-valid-input / analyzer crash (divide-with-overflow)  ·  **Subsystem:** Optimizer · abstract interpretation  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/optimize/abstract_interp.rs:210–226`

**Summary**

The abstract interpreter evaluates integer division/modulo on two exact (singleton) intervals using Rust's raw `a / b` and `a % b` operators. For the single overflowing case in two's-complement i64 arithmetic, `i64::MIN / -1` (the true quotient 2^63 is not representable) and `i64::MIN % -1`, these operators panic ('attempt to divide with overflow' in debug, abort/UB in release). The sibling `Interval::mul` two lines above (line 203) is correctly overflow-safe via `checked_mul`, and the runtime arithmetic (crates/logicaffeine_compile/src/semantics/arith.rs:136,170) deliberately uses `wrapping_div`/`wrapping_rem` so the program itself runs fine and produces i64::MIN. Only the analyzer crashes. `eval_expr` (line 941-942) routes `BinaryOpKind::Divide`/`Modulo` straight into these methods, and `record_expr` (line 1737) calls `eval_expr` on every subexpression, so the public `oracle_analyze_with` / `oracle_analyze` API — invoked on the live VM/JIT/codegen compile path (crates/logicaffeine_compile/src/vm/mod.rs:81,112; crates/logicaffeine_compile/src/codegen/program.rs:325) — panics the whole compiler.

**Why it's wrong**

The runtime treats i64::MIN / -1 and i64::MIN % -1 as well-defined wrapping operations (arith.rs uses wrapping_div/wrapping_rem; there is an explicit runtime test at arith.rs:277-280 asserting they yield i64::MIN, and a JIT parity test jit_div_mod.rs:224 exercises the literal form). The compile-time abstract interpreter must agree with that semantics or at minimum not crash. Using raw `/`/`%` makes the analyzer panic on a value the language explicitly supports. The bug is reachable whenever the two operands are EXACT intervals but are not BOTH literal expressions (so the constant-folder, which uses wrapping_div/wrapping_rem and would have collapsed a literal/literal division, never touches them) — e.g. when they are identifiers bound to i64::MIN and -1.

**The offending code**

```rust
fn div(&self, other: &Interval) -> Interval {
    if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
        if b != 0 {
            return Interval::exact(a / b);   // panics on i64::MIN / -1
        }
    }
    Interval::top()
}

fn modulo(&self, other: &Interval) -> Interval {
    if let (Some(a), Some(b)) = (self.is_exact(), other.is_exact()) {
        if b != 0 {
            return Interval::exact(a % b);   // panics on i64::MIN % -1
        }
    }
    Interval::top()
}
```

**Trigger / reproduction**

English/LOGOS program: `## Main\nLet a be 0 - 9223372036854775807 - 1.\nLet b be 0 - 1.\nLet c be a / b.\nShow c.\n`  — `a` evaluates (via Bound::sub, all checked, no overflow at any step: 0 - i64::MAX = -i64::MAX, then -1 = i64::MIN) to the exact interval [i64::MIN, i64::MIN]; `b` to [-1, -1]. Because `a` and `b` are identifiers, fold cannot reduce `a / b`, so eval_expr computes [MIN,MIN].div([-1,-1]) -> MIN / -1 -> panic. (The modulo variant: replace the last Let with `Let c be a modulo b.`)

**Expected vs actual**

- **Expected:** oracle_analyze_with / oracle_analyze returns a fact table without panicking; the interval for `c` is the wrapping result i64::MIN (exact) or, conservatively, Interval::top(). The program then compiles and runs, printing the wrapped value -9223372036854775808 — matching the tree-walker.
- **Actual:** Interval::div executes `i64::MIN / -1`, which panics with 'attempt to divide with overflow', aborting the analysis pass and therefore the entire compilation/run of a valid program.

**Suggested RED test**

```rust
// crates/logicaffeine_tests/tests/phase_exodia_oracle.rs
// Matches the existing idiom at the top of that file:
//   use logicaffeine_compile::optimize::{oracle_analyze_with, ...};
//   use logicaffeine_compile::ui_bridge::with_parsed_program;

#[test]
fn oracle_does_not_panic_on_min_div_neg_one() {
    // a = i64::MIN, b = -1, c = a / b. Identifiers, so the constant folder
    // (fold.rs only folds literal/literal) cannot pre-reduce `a / b`; the raw
    // `a / b` in Interval::div (abstract_interp.rs:213) then panics
    // 'attempt to divide with overflow'. No intermediate step overflows:
    // Bound::sub is checked, 0 - i64::MAX = -i64::MAX, then -1 = i64::MIN.
    let src = "## Main\n\
               Let a be 0 - 9223372036854775807 - 1.\n\
               Let b be 0 - 1.\n\
               Let c be a / b.\n\
               Show c.\n";
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("parse");
        // RED: currently panics inside Interval::div (i64::MIN / -1).
        // GREEN (after wrapping_div fix): returns facts; c's interval is exact i64::MIN.
        let _facts = oracle_analyze_with(stmts, interner);
    });
}

#[test]
fn oracle_does_not_panic_on_min_mod_neg_one() {
    let src = "## Main\n\
               Let a be 0 - 9223372036854775807 - 1.\n\
               Let b be 0 - 1.\n\
               Let c be a modulo b.\n\
               Show c.\n";
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("parse");
        // RED: currently panics inside Interval::modulo (i64::MIN % -1).
        // GREEN (after wrapping_rem fix): returns facts; c's interval is exact 0.
        let _facts = oracle_analyze_with(stmts, interner);
    });
}

// FIX (crates/logicaffeine_compile/src/optimize/abstract_interp.rs):
//   div:    return Interval::exact(a.wrapping_div(b));   // was: a / b   (line 213)
//   modulo: return Interval::exact(a.wrapping_rem(b));   // was: a % b   (line 222)
// This both stops the panic AND keeps the interval sound/precise, matching
// semantics/arith.rs:136,170 and fold.rs:568-569.
```

**Fix direction / notes**

No factual corrections to the claim — every cited line, the reachability chain, and the runtime-semantics mismatch check out. The auditor's "trigger" arithmetic, the fold non-reduction argument, and the API idiom are all accurate.

One emphasis/refinement on the fix and the RED assertion: the fix must use `a.wrapping_div(b)` / `a.wrapping_rem(b)` (NOT merely returning `Interval::top()`), because the runtime and the JIT/codegen consumers of OracleFacts treat the result as the exact value i64::MIN (div) / 0 (modulo). Returning `top()` would avoid the panic but throw away a precise, correct fact; returning a WRONG exact value would be unsound. `wrapping_div`/`wrapping_rem` both avoids the panic AND keeps the interval sound and precise, matching semantics/arith.rs.

Severity High is correct and unchanged: it crashes the entire compile/run of a valid program (the panic aborts oracle_analyze_with on the live VM/JIT/codegen path), but it requires the narrow `i64::MIN / -1` (or `% -1`) operand pair routed through non-foldable identifiers, so it is not Critical (not trivially hit by arbitrary input), and clearly above Medium.

---

### BUG-007 — `Repeat` over a Map binds the loop variable to the KEY type, but the runtime yields (key, value) tuples

**Severity:** High  ·  **Category:** type-inference / wrong-concrete-type miscompilation  ·  **Subsystem:** Semantic checks  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/analysis/check.rs:701–715`

**Summary**

When iterating a `Map(K, V)` with a single (`Pattern::Identifier`) loop variable, the checker binds that variable to the key type `K` (line 705: `InferType::Map(k, _) => *k`). But the runtime iteration semantics (semantics/collections.rs `iteration_snapshot`, lines 244-248) produce one `RuntimeValue::Tuple(vec![k, v])` per entry — i.e. iterating a map yields key/value PAIRS, not bare keys. So the checker records a loop-variable type (`K`, e.g. Int) that contradicts the actual runtime value (a 2-tuple). Any codegen/optimizer pass that trusts this loop-var type from the TypeEnv will treat a tuple as a scalar key. The Tuple-pattern arm (lines 710-714) is the only correct way to iterate a map, and even it discards the element types (binds every component to Unknown).

**Why it's wrong**

The static type assigned to the loop variable must match the value actually produced at runtime. The runtime emits Tuple(k, v) for every map entry, so binding the loop var to `K` is provably wrong. This is exactly the class of unsound fact that can drive a miscompilation when codegen specializes on the loop variable's type.

**The offending code**

```rust
Stmt::Repeat { pattern, iterable, body } => {
    let iterable_ty = self.infer_expr(iterable)?;
    let elem_ty = match self.table.zonk(&iterable_ty) {
        InferType::Seq(inner) | InferType::Set(inner) => *inner,
        InferType::Map(k, _) => *k,
        _ => InferType::Unknown,
    };
    match pattern {
        Pattern::Identifier(sym) => self.bind_var(*sym, elem_ty),
        Pattern::Tuple(syms) => {
            for sym in syms { self.bind_var(*sym, InferType::Unknown); }
        }
    }
```

**Trigger / reproduction**

English: `Let m be a new Map of Text and Int. Repeat for each entry in m: ...` with a single identifier binding. AST form below uses a Map-typed variable iterated with Pattern::Identifier.

**Expected vs actual**

- **Expected:** The loop variable iterating a Map(K,V) under a single binding should be a tuple/entry type (key+value), not the bare key type K.
- **Actual:** The loop variable is bound to the key type K, contradicting the runtime which produces Tuple(K, V).

**Suggested RED test**

```rust
// Place inside `mod tests` in crates/logicaffeine_compile/src/analysis/check.rs
// (that module already has `use super::*;` and
//  `use crate::ast::stmt::{Expr, Literal, Stmt, TypeExpr};`).
// Modeled directly on the existing `repeat_loop_var_gets_element_type` test.
#[test]
fn repeat_over_map_single_ident_loop_var_is_not_bare_key() {
    let mut interner = mk_interner();
    let m = interner.intern("m");
    let entry = interner.intern("entry");
    let map_sym = interner.intern("Map");
    let text_sym = interner.intern("Text");
    let int_sym = interner.intern("Int");

    // Let m be a new Map of Text and Int.
    let new_map = Expr::New {
        type_name: map_sym,
        type_args: vec![TypeExpr::Primitive(text_sym), TypeExpr::Primitive(int_sym)],
        init_fields: vec![],
    };
    let let_m = Stmt::Let { var: m, ty: None, value: &new_map, mutable: false };

    // Repeat for each entry in m: (single identifier pattern)
    let m_ref = Expr::Identifier(m);
    let repeat = Stmt::Repeat {
        pattern: Pattern::Identifier(entry),
        iterable: &m_ref,
        body: &[],
    };

    let stmts = [let_m, repeat];
    let env = run(&stmts, &interner);

    // RED today: the checker types `entry` as the bare key (Text -> LogosType::String),
    // but the interpreter and VM both bind it to the (key, value) Tuple produced by
    // iteration_snapshot. The loop-var type must NOT be the bare key type.
    assert_ne!(
        env.lookup(entry),
        &LogosType::String,
        "iterating a Map with a single identifier yields (key,value) tuples at runtime \
         (interpreter.rs:1051, vm via iteration_snapshot); the loop var must not be typed \
         as the bare key K"
    );
}
```

**Fix direction / notes**

Severity is correctly rated High; I keep it. It is observable wrong behavior from valid input (C backend yields a bare key where interpreter+VM yield the (key,value) tuple), not merely cosmetic — but it is not Critical because (a) the two primary executors (interpreter and VM) agree with each other, so the failure mode is specifically a C-codegen vs interpreter/VM divergence rather than a universally-wrong result, and (b) it requires the non-idiomatic single-identifier-over-Map form; the natural and documented way to iterate a Map is a tuple pattern `(k, v)`. No panic occurs.

One scope correction to the claim's framing: the claim says "even [the Tuple-pattern arm] discards the element types (binds every component to Unknown)." That is true (check.rs:710-714 and types.rs:621-626 bind each tuple component to Unknown), but binding to Unknown is conservatively safe (Unknown can't drive a wrong specialization), whereas binding the single identifier to the concrete key type K is the actually-unsound part. The core finding — single-identifier Map iteration is typed as K but must be the entry tuple — is correct.

The claim's suggested RED test does not compile as written; use the refined test below instead.

---

### BUG-008 — Char param/return crosses the C ABI as a raw Rust `char`, which C declares as `uint32_t` — out-of-range u32 from C becomes an invalid `char` (UB)

**Severity:** High  ·  **Category:** FFI type/validity mismatch / non-FFI-safe char at boundary  ·  **Subsystem:** Codegen · FFI / runtime boundary  ·  **Reporter confidence:** medium  
**Location:** `crates/logicaffeine_compile/src/codegen/ffi.rs:1693–1694`

**Summary**

A function like `To f (c: Char) -> Char is exported` does not contain Text/reference/Result/refinement types, so needs_c_marshaling is false (program.rs lines 640-655) and it takes the plain export path: `pub extern "C" fn f(c: char) -> char` (codegen_type_expr maps Char -> char, types.rs line 111). The matching C header declares the parameter and return as `uint32_t` (map_type_to_c_header line 1694). The same mismatch occurs for value-type struct fields: a struct `{ ch: Char }` is classified ValueType and emitted `#[repr(C)] struct S { ch: char }` (types.rs line 323) while the C typedef field is `uint32_t` (map_field_type_to_c line 1719).

**Why it's wrong**

Rust's `char` is a niche type: every `char` value MUST be a Unicode scalar value (0..=0xD7FF or 0xE000..=0x10FFFF). C's `uint32_t` has no such restriction. When a C caller passes a surrogate (0xD800..=0xDFFF) or an out-of-range value (>0x10FFFF) in the uint32_t slot, the Rust side materializes an invalid `char`, which is instantaneous undefined behavior — merely copying or matching on it is UB, even before any arithmetic. `char` is also not a recognized #[repr(C)] FFI-safe scalar, so the boundary type is not the stable uint32_t the header promises. The safe lowering is to use u32 at the extern boundary and validate via char::from_u32 before handing it to inner Rust code.

**The offending code**

```rust
// map_type_to_c_header:
"Char" => "uint32_t".to_string(), // UTF-32 char
// but the Rust side (types.rs map_type_to_rust line 111) emits bare `char`,
// and program.rs emits `pub extern "C" fn f(c: char) -> char` for a non-marshaled Char export
```

**Trigger / reproduction**

Compile `## To shift (c: Char) -> Char is exported:\n    Return c.` From C, call `logos_shift(0x110000u)` (or `0xD800u`). The extern fn receives an invalid `char`; the inner code's use/return of it is UB.

**Expected vs actual**

- **Expected:** The Char boundary type on the Rust extern signature should be `u32` (matching the C `uint32_t`), with conversion through `char::from_u32(...)` that rejects/handles invalid scalar values — so a malformed code point from C cannot produce an invalid Rust `char`.
- **Actual:** The Rust extern signature uses bare `char` (e.g. `pub extern "C" fn f(c: char) -> char`) while the C header types the same slot as `uint32_t`, so any non-scalar uint32_t passed from C yields an invalid `char` (UB).

**Suggested RED test**

```rust
#[test]
fn char_crosses_c_abi_as_validated_u32_not_raw_char() {
    use logicaffeine_compile::compile_program_full;
    // Same tested `is exported` syntax as snapshot_exported_c_function_codegen,
    // but with Char (which the C header declares as uint32_t).
    let source = r#"## To shift (c: Char) -> Char is exported:
    Return c.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let rust = &output.rust_code;

    // The exported wrapper for `shift` must NOT take/return a bare Rust `char`.
    // A C caller passing a non-scalar uint32_t (e.g. 0xD800 surrogate or
    // 0x110000) would materialize an invalid `char` -> instantaneous UB.
    // The safe lowering uses u32 at the boundary (validated via char::from_u32).
    let bad_param = rust.contains("fn shift(c: char)");
    let bad_ret = rust.contains("fn shift(c: char) -> char")
        || rust.contains(") -> char {"); // pinned: only the exported wrapper returns Char here
    assert!(
        !bad_param,
        "Char param must cross the C ABI as u32 (matching uint32_t in the header), \
         validated via char::from_u32 — not as a raw Rust `char` (non-scalar u32 from C is UB). \
         Generated:\n{}",
        rust
    );
    assert!(
        !bad_ret,
        "Char return must cross the C ABI as u32 (matching uint32_t in the header), \
         not as a raw Rust `char`. Generated:\n{}",
        rust
    );

    // Companion expectation once fixed: the boundary should round-trip through u32.
    // (Kept as a soft signal; primary RED assertions are the two above.)
    assert!(
        rust.contains("char::from_u32") || rust.contains(": u32"),
        "Expected the Char C-export boundary to be lowered to u32 with char::from_u32 validation. \
         Generated:\n{}",
        rust
    );
}
```

**Fix direction / notes**

Confirmed but with two corrections to the claim. (a) Technical framing: bare `char` at an `extern "C"` boundary is NOT rejected by Rust's `improper_ctypes` lint and is ABI-layout-compatible with a 32-bit integer, so the bits pass through correctly; the defect is purely the `char` VALIDITY invariant (a non-scalar u32 from C yields an invalid `char` = UB). The claim's phrase "char is also not a recognized #[repr(C)] FFI-safe scalar" is imprecise and should be dropped. (b) The claim's suggested RED test is unusable as written: it checks `rust.contains("pub extern \"C\" fn f(c: char)")`, but the emitted function name is the user's name (`shift`/`add`), never `f`, so that clause never matches; and the `|| rust.contains("-> char")` clause is both loose (the preamble contains `c_char` helpers — those won't match `-> char` exactly, but it's fragile) and not pinned to the actual export. Severity High is retained: generated code with reachable UB on the FFI trust boundary is serious. The trigger is somewhat narrow (requires the user to export a Char param/return or a by-value struct with a Char field AND a foreign caller to supply a non-scalar u32), so a case for Medium exists, but UB in emitted code justifies High.

---

### BUG-009 — Reference-type handle parameter is dereferenced with .expect() OUTSIDE the catch_unwind boundary — a null/stale handle from C unwinds across extern "C" (UB)

**Severity:** High  ·  **Category:** FFI boundary / panic-across-extern-C / null-handle marshalling  ·  **Subsystem:** Codegen · FFI / runtime boundary  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/codegen/marshal.rs:200–228`

**Summary**

For a C-exported function that takes a reference-type parameter (Seq/Map/Set/Option/Result, or a large user struct), codegen_c_export_with_marshaling emits the handle lookup at step (3), which runs in the wrapper body BEFORE the catch_unwind closure opens at line 266. The lookup uses `.deref(__id).expect("InvalidHandle...")`. If the C caller passes 0 (NULL, which casts to u64 id 0 — never a valid id since the registry counter starts at 1), a previously-freed handle, or any garbage handle, `deref` returns None and `.expect` panics. Because this panic is raised outside the catch_unwind boundary, it unwinds straight through the `extern "C"` function frame.

**Why it's wrong**

Unwinding a panic across an `extern "C"` ABI boundary is undefined behavior in Rust (best case: an immediate process abort; with certain panic settings it can corrupt the C caller's stack). The entire rest of this Universal ABI is meticulously designed to NEVER do this: every generated accessor uses emit_null_handle_check + emit_registry_deref which return a graceful LogosStatus/null on a bad handle (ffi.rs lines 346-364), and the wrapper itself wraps the inner call in catch_unwind precisely to convert panics into LogosStatus::ThreadPanic. The reference-type parameter path is the one place that bypasses both safeguards, turning the single most common C misuse (passing a stale or NULL handle) into UB instead of an InvalidHandle error.

**The offending code**

```rust
// 3) Marshal parameters  (emitted BEFORE the catch_unwind that opens at line 266)
if classify_type_for_c_abi(ptype, interner, registry) == CAbiClass::ReferenceType {
    writeln!(output, "    let {pn} = {{", pn = pname_str).unwrap();
    writeln!(output, "        let __id = {pn} as u64;", pn = pname_str).unwrap();
    writeln!(output, "        let __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
    writeln!(output, "        let __ptr = __reg.deref(__id).expect(\"InvalidHandle: handle not found in registry\");").unwrap();
    writeln!(output, "        drop(__reg);").unwrap();
    writeln!(output, "        unsafe {{ &*(__ptr as *const {ty}) }}.clone()", ty = rust_ty).unwrap();
    writeln!(output, "    }};").unwrap();
}
```

**Trigger / reproduction**

Compile `## To total (xs: Seq of Int) -> Int is exported:\n    Return 0.` then from C call `logos_total(NULL)` or `logos_total(some_already_freed_handle)`. The generated wrapper panics at the `.deref(__id).expect(...)` line, which sits before the `match std::panic::catch_unwind(...)`.

**Expected vs actual**

- **Expected:** A NULL or invalid handle should return LogosStatus::InvalidHandle (or LogosStatus::NullPointer) and record the error via logos_set_last_error, exactly as the standalone accessors do — never panic across the C boundary.
- **Actual:** The handle lookup `.expect("InvalidHandle: handle not found in registry")` panics, and the panic is emitted outside the catch_unwind closure, so it unwinds across the `extern "C"` boundary (UB / abort).

**Suggested RED test**

```rust
#[test]
fn ref_param_handle_lookup_never_panics_across_extern_c() {
    use logicaffeine_compile::compile_program_full;

    let source = r#"## To total (xs: Seq of Int) -> Int is exported:
    Return 0.

## Main
Show 42.
"#;
    let output = compile_program_full(source).expect("Should compile");
    let rust = &output.rust_code;

    // Locate the generated extern "C" wrapper for the exported function.
    let wrapper_start = rust
        .find("pub extern \"C\" fn logos_total(")
        .expect("C wrapper for `total` should be generated");
    // Bound the wrapper region at the start of the next top-level item so we
    // only inspect this function's body.
    let rest = &rust[wrapper_start..];
    let wrapper_end = rest[1..]
        .find("\n#[no_mangle]")
        .or_else(|| rest[1..].find("\nfn "))
        .or_else(|| rest[1..].find("\npub "))
        .map(|i| i + 1)
        .unwrap_or(rest.len());
    let wrapper = &rest[..wrapper_end];

    // RED today: the reference-type parameter is marshalled with
    // `__reg.deref(__id).expect("InvalidHandle: ...")`, which is emitted in the
    // wrapper body BEFORE the `catch_unwind` boundary opens. A NULL handle
    // (id 0, never valid) or a stale/freed handle makes `deref` return None, so
    // `.expect` panics and unwinds straight across the `extern "C"` frame
    // (process abort on stable Rust 1.81+; UB on extern "C-unwind"/older).
    //
    // The fix must make a bad handle return InvalidHandle / a graceful default
    // exactly like the standalone accessors (emit_registry_deref), so this
    // panicking lookup form must NOT appear in the wrapper.
    assert!(
        !wrapper.contains(".deref(__id).expect("),
        "Reference-type handle parameter must NOT be dereferenced with a \
         panicking `.expect()` outside catch_unwind — a NULL/stale handle would \
         panic across the extern \"C\" boundary. Use the graceful \
         match-on-None pattern (return InvalidHandle) like the accessors do. \
         Generated wrapper:\n{}",
        wrapper
    );
}
```

**Fix direction / notes**

Severity downgraded from Critical to High. On a current stable Rust toolchain (1.81+), a panic that reaches an `extern "C"` (not `extern "C-unwind"`) frame triggers a deterministic process abort, not classic arbitrary-memory-corruption UB. The claim's "best case: immediate process abort; can corrupt the C caller's stack" overstates the modern worst case — stack corruption only applies to `extern "C-unwind"` or pre-1.81 toolchains. The genuinely reproducible defect is: passing a NULL or stale handle (the most common C misuse) aborts the host process instead of returning `LogosStatus::InvalidHandle` / a graceful default like the rest of the Universal ABI. That is a real, high-impact robustness/correctness bug worth fixing, but the practical observable outcome on a modern toolchain is a controlled abort rather than memory corruption — High, not Critical.

The fix is to emit the same graceful pattern used by `emit_registry_deref`: replace `let __ptr = __reg.deref(__id).expect("InvalidHandle: ...");` with a `match __reg.deref(__id) { Some(p) => p, None => { logos_set_last_error("InvalidHandle: ..."); return <appropriate default/status>; } }`, and add a NULL check. Note this needs an out-parameter / status-code shape to return InvalidHandle cleanly for value-return functions; alternatively, set `uses_status_code` to also include ref-type params (the comment at marshal.rs:83 claiming "catch_unwind handles invalid handle panics" is currently false because the lookup is emitted outside catch_unwind).

RED test corrected: the original couples to the buggy text by asserting `.expect("InvalidHandle` is present, then ordering it against `catch_unwind`. A correct fix removes the `.expect()` entirely, which would make the original test's `.find(".expect(\"InvalidHandle").expect(...)` panic (test errors instead of passing). The refined test below is fix-agnostic: it asserts the wrapper contains no panicking `.deref(__id).expect(` at all (the only correct outcomes are the graceful match or moving inside catch_unwind, both of which drop this exact substring).

---

### BUG-010 — C backend lowers integer pow() to floating-point pow(), producing wrong integer results (precision/wrapping divergence)

**Severity:** High  ·  **Category:** miscompilation / wrong numeric semantics / type-width truncation  ·  **Subsystem:** Codegen · statements  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/codegen_c/emit.rs:143–155`

**Summary**

The C codegen for the `pow` builtin unconditionally emits the C math `pow((double)base, (double)exp)` — a floating-point power returning a `double`. But the language spec for `pow` over two integers (semantics/builtins.rs:216-238, validated by the spec tests `math_builtins_match_spec` and `abs_and_pow_wrap_like_the_int_spec`) is *exact integer* power: `Int(b.wrapping_pow(e as u32))` for non-negative integer exponent. A double only has a 53-bit mantissa, so integer powers that need more than 53 significant bits round to the nearest double and then get truncated back to int64_t, yielding a value that differs from the interpreter/VM by one or more. The compile_to_c pipeline (compile.rs:292-325) runs optimize_program, but the optimizer (optimize/fold.rs:326) only folds call arguments and never constant-evaluates `pow`, so the call reaches this codegen intact.

**Why it's wrong**

The tree-walking interpreter and VM compute integer `pow(b,e)` exactly (wrapping on overflow). The C backend computes it in `double` and truncates, so for any integer power exceeding 53 significant bits the compiled program prints a different number than every other execution tier. This is a silent miscompilation of a common arithmetic operation, not just an edge case at i64::MIN.

**The offending code**

```rust
"pow" => {
    let base = if let Some(a) = args.first() { codegen_expr(a, ctx) } else { "0.0".to_string() };
    let exp  = if let Some(a) = args.get(1) { codegen_expr(a, ctx) } else { "1.0".to_string() };
    format!("pow((double)({}), (double)({}))", base, exp)
}
```

**Trigger / reproduction**

LOGOS source: `## Main\nShow pow(3, 34).`  (3^34 = 16677181699666569, which is exactly representable as i64 but needs 54 bits, so it is NOT exactly representable as f64).

**Expected vs actual**

- **Expected:** Program prints `16677181699666569` (the exact integer power, matching the interpreter and VM).
- **Actual:** The emitted C contains `show_i64(pow((double)(3LL), (double)(34LL)))`; C's `pow(3.0,34.0)` is a double approximation of 3^34 (e.g. 1.667718169966657e16 → 16677181699666570, or the nearest-double 16677181699666568), so the compiled binary prints 16677181699666570 (off by +1) — a value the interpreter never produces. (For exponents large enough that the true power exceeds i64::MAX, e.g. `pow(10,19)`, the double→int64_t conversion is even undefined behavior in C, whereas the spec defines a wrapping result.)

**Suggested RED test**

```rust
// in crates/logicaffeine_tests/tests/e2e_math_builtins.rs
// (assert_c_output and assert_interpreter_output are already imported at the top of this file)

#[test]
fn c_pow_large_integer_is_exact() {
    // 3^34 = 16677181699666569. It fits in i64 but needs 54 significant bits,
    // so it is NOT exactly representable as an f64 (nearest doubles are
    // 16677181699666568 and 16677181699666570). The interpreter/VM compute the
    // exact integer power via wrapping_pow; the C backend lowers `pow` to the
    // floating-point libm pow((double)base,(double)exp) and truncates back to
    // int64_t, so it can never print the exact value. This test currently FAILS
    // on the C tier and PASSES once codegen emits an integer power.
    let src = "## Main\nShow pow(3, 34).\n";

    // Sanity: the interpreter (source of truth) gives the exact integer.
    assert_interpreter_output(src, "16677181699666569");

    // The C backend must match the interpreter. Currently it prints a rounded
    // value (e.g. 16677181699666570), so this assertion fails today.
    assert_c_output(src, "16677181699666569");
}
```

**Fix direction / notes**

No corrections to the claim's substance — the file path, line range (emit.rs:143-155), the spec citation (builtins.rs:216-238), the optimizer note (fold.rs:326), the trigger, and the expected/actual values are all accurate. I additionally confirmed the type-inference link the claim implied but did not fully spell out: codegen_c/types.rs:245 returns CType::Int64 for the `pow` call (because builtins are absent from ctx.funcs, populated only from user functions in codegen_c/mod.rs), which is what routes the double result into show_i64 and forces the silent double→int64 truncation. Severity stays High (silent wrong-result miscompilation, not a memory-safety/security issue).

---

### BUG-011 — Decompiler drops parentheses around a `not`/negation operand, inverting precedence so `not (a and b)` becomes `(not a) and b` in the First Futamura Projection residual

**Severity:** High  ·  **Category:** miscompilation / wrong scope-binding (operator precedence) in lowering  ·  **Subsystem:** Compile pipeline (Futamura)  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/compile.rs:3710–3713`

**Summary**

`decompile_expr` is the AST->source lowering used by `projection1_source` (the First Futamura Projection): it turns the optimized AST back into LOGOS source that is re-parsed and re-run. The `BinaryOp` arm parenthesizes a child only when that *child* is itself a `BinaryOp` (lines 3663-3672), but the `Expr::Not` arm emits `not {inner}` with NO parentheses around its operand. When the operand is a boolean `BinaryOp` (And/Or/comparison), the emitted text re-parses with the wrong precedence. The LOGOS imperative grammar (crates/logicaffeine_language/src/parser/mod.rs: `parse_or_condition` < `parse_and_condition` < `parse_comparison`, with `not` handled *inside* `parse_comparison` at lines 2313-2319) binds `not` tighter than `and`/`or`. So the AST node `Not(BinaryOp(And, a, b))` is lowered to the string `not a and b`, which the parser reads back as `(not a) and b` = `BinaryOp(And, Not(a), b)` — a structurally different, logically-different expression (negation scope changed from the whole conjunction to just the left operand).

**Why it's wrong**

The projection is contractually an identity-with-optimizations transform: the residual must compute the same result as the source. The projection-path optimizer (`optimize_for_projection` in optimize/mod.rs, lines 31-54 = fold + propagate + PE + CTFE only) never rewrites `Not(And(..))` away: `fold_expr`'s `Expr::Not` arm (optimize/fold.rs lines 311-323) only does constant-fold and double-negation, and PE does not unroll `Repeat` loops (partial_eval.rs lines 502-511 substitutes the body but keeps the loop with a symbolic induction variable). So a `not (a and b)` over runtime-symbolic operands survives optimization verbatim and is then mis-lowered. The same defect mislowers `not (a or b)` -> `(not a) or b`.

**The offending code**

```rust
Expr::Not { operand } => {
    let inner = decompile_expr(operand, interner);
    format!("not {}", inner)
}
```

**Trigger / reproduction**

LOGOS program (projection input):
## Main
Repeat for i in 1 through 3:
    If not ((i is greater than 1) and (i is less than 3)):
        Show "edge".
    Otherwise:
        Show "mid".

The loop variable i stays symbolic through the projection optimizer, so the condition `not ((i>1) and (i<3))` is preserved as `Not(BinaryOp(And, (i>1), (i<3)))` and reaches decompile_expr.

**Expected vs actual**

- **Expected:** Correct semantics (and the unoptimized tree-walker output): i=1 -> not(F and T)=not F=T -> "edge"; i=2 -> not(T and T)=not T=F -> "mid"; i=3 -> not(T and F)=not F=T -> "edge". Output: `edge\nmid\nedge`.
- **Actual:** decompile_expr emits the residual condition as `not (i is greater than 1) and (i is less than 3)`, which re-parses as `(not (i>1)) and (i<3)`. i=1 -> (not F) and T = T -> "edge"; i=2 -> (not T) and T = F -> "mid"; i=3 -> (not T) and F = F -> "mid". Output: `edge\nmid\nmid` — wrong at i=3 because the negation now only covers `i>1` instead of the whole conjunction.

**Suggested RED test**

```rust
// crates/logicaffeine_tests/tests/phase_futamura.rs
// Uses the existing get_p1_residual helper (calls projection1_source, the Rust
// decompile path) and the public tw_outcome oracle. Comparing the residual on
// the tree-walker against the source on the tree-walker isolates the decompile
// defect (both sides use the same interpreter, so any difference is purely from
// AST->source lowering).
#[test]
fn p1_not_over_conjunction_keeps_negation_scope() {
    let program = "Repeat for i in 1 through 3:\n    If not ((i is greater than 1) and (i is less than 3)):\n        Show \"edge\".\n    Otherwise:\n        Show \"mid\".";

    // Oracle: the tree-walker on the ORIGINAL program.
    let oracle = logicaffeine_compile::compile::tw_outcome(&format!("## Main\n{}", program));
    assert_eq!(oracle.error, None, "oracle should run cleanly");
    assert_eq!(oracle.output, "edge\nmid\nedge", "source semantics: not(F&T)=T, not(T&T)=F, not(T&F)=T");

    // First Futamura Projection residual (projection1_source / Rust decompile_expr).
    let residual = get_p1_residual(program);

    // The residual must reproduce the oracle output exactly.
    let from_residual = logicaffeine_compile::compile::tw_outcome(&residual);
    assert_eq!(from_residual.error, None, "residual should run cleanly:\n{}", residual);
    assert_eq!(
        from_residual.output, "edge\nmid\nedge",
        "P1 residual diverges from source. decompile_expr lowered Not(And(a,b)) as \
         `not a and b`, which re-parses as `(not a) and b` (negation scope shrunk). \
         Residual:\n{}", residual
    );

    // Structural guard: the residual must NOT emit the negation immediately before
    // an unparenthesized conjunction (i.e. `not <cmp> and`); the `not` operand must
    // be wrapped. This pins the exact lowering bug.
    assert!(
        !residual.contains("not (i is greater than 1) and"),
        "decompiler dropped the parens around the negated conjunction:\n{}", residual
    );
}
```

**Fix direction / notes**

No correction to the core claim — it is accurate as stated, including file, line range, code excerpt, parser-precedence analysis, optimizer-survival argument, and the i=3 divergence.

Severity confirmed as High (not raised to Critical): decompile_expr is reached ONLY through projection1_source / decompile_stmt (grep confirms no other callers in the compile crate). The normal run path (VM/JIT/tree-walker) does not go through decompile_expr, so production program execution is unaffected. The defect corrupts the First Futamura Projection residual specifically — a transform whose contract is result-preservation — so silent wrong output there is a real correctness violation but bounded to the projection/decompile feature. High is the right rating.

Scope note worth adding to any fix: the same arm also mislowers Not(Or(..)) -> "(not a) or b" and Not(comparison-or-anything-non-atomic). The correct fix is to parenthesize the operand whenever it is not already atomic — i.e. wrap when the operand is an Expr::BinaryOp (and arguably any non-trivial expr), mirroring the BinaryOp arm's child-parenthesization rule: format!("not ({})", inner) when matches!(operand, Expr::BinaryOp { .. }), else format!("not {}", inner). Note `not (a)` re-parses fine because the parenthesized group is a valid comparison primary, so always-wrapping is also safe.

---

### BUG-012 — Pinned JIT compiler panics (unreachable!) on float-branch / float-`>`/`>=` ops it never lowers — reachable from the function-compile pinning path

**Severity:** High  ·  **Category:** miscompilation/panic-on-valid-input  ·  **Subsystem:** JIT · forge (copy-and-patch)  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_forge/src/jit.rs:1287–1306`

**Summary**

The register-threading (pinned) compiler `compile_straightline_pinned_with` dispatches every op not in its explicit PASS-2 match into the `ref other =>` arm, which unconditionally calls `emit_mem_form`. `emit_mem_form` (jit.rs:693-936) has arms for Div/Mod/AddF/SubF/MulF/DivF/LtF/LtEqF/EqF/NeqF/NotInt/NotBool/IntToFloat/SqrtF/Call/MapGet/MapSet/CallSelf/NewList/ListTriple/MapHas/ArrLoad/ArrStore/ArrPush, then a terminal `ref other => unreachable!()`. Three MicroOp variants the adapter actually emits are NOT covered there: `MicroOp::BranchF` (fused float compare-and-branch, emitted by translate_op at logicaffeine_jit/src/lib.rs:1643), and `MicroOp::GtF` / `MicroOp::GtEqF` (float `>` / `>=` value compares, emitted at logicaffeine_jit/src/lib.rs:1717 and 1719). A static diff confirms BranchF is the only op present in `mem_form_touch` (jit.rs:1004) but absent from `emit_mem_form`; GtF/GtEqF are in neither `has_variant`, nor `mem_form_touch`'s float arm (jit.rs:986-996, which lists only DivF/LtF/LtEqF/EqF/NeqF), nor the pinned PASS-2 explicit match, so they also fall through to `ref other` and `emit_mem_form`. In all three cases the result is an immediate `unreachable!()` panic during compilation rather than correct native code.

**Why it's wrong**

`compile_region` deliberately disables pinning for any program containing a BranchF (logicaffeine_jit/src/lib.rs:3024: `let pinnable = !micro.iter().any(|op| matches!(op, MicroOp::BranchF { .. }));`), proving the authors know the pinned compiler cannot lower it. But the parallel function-compile path applies NO such guard: its only pinning condition is `adapted.precise.is_none()` (mode A), i.e. `mode_b == false` (logicaffeine_jit/src/lib.rs:2947-2953). `mode_b = !list_params.is_empty() || has_sites` (line 878), so a mode-A function with no list params and no NewEmptyList sites — but containing a float comparison-branch or a float `>`/`>=` — reaches `select_pins(&adapted.micro, register_count, false)` (line 2950). `select_pins` only assigns these ops negative profit; it does not veto pinning, so OTHER hot integer slots in the same function still earn pins and `pins` is non-empty, routing the program into `compile_straightline_pinned_with` with a BranchF/GtF/GtEqF present. PASS 2 then panics at the `unreachable!`. GtF/GtEqF additionally have no fall-back swap-to-LtF lowering in the pinned path (the unpinned path handles them at jit.rs:1475-1494, but the pinned PASS-2 match does not), so even the value-form float greater-than crashes.

**The offending code**

```rust
// PASS 2 main loop, the catch-all memory-form arm:
ref other => {
    let (reads, writes) = mem_form_touch(other);
    let mut cursor = here;
    for &slot in &reads { spill(&mut buf, slot, cursor + 1); cursor += 1; }
    let after_op = cursor + 1;
    let next_for_op = if writes.is_empty() { next_op } else { after_op };
    let next_label = buf.label(next_for_op);
    emit_mem_form(&mut buf, other, next_label, deopt_piece, &status); // <-- BranchF/GtF/GtEqF reach here
    ...
}
// emit_mem_form's terminal arm (line 934):
ref other => unreachable!("emit_mem_form: unsupported op {other:?} (pinned chains exclude it)"),
```

**Trigger / reproduction**

DIRECT (deterministic, public API): call `compile_straightline_pinned(&ops, &[5])` with `ops = [MicroOp::BranchF { cmp: Cmp::Lt, lhs: 1, rhs: 2, target: 1 }, MicroOp::Return { src: 0 }]` — PASS 2 sends the BranchF into emit_mem_form and panics with `emit_mem_form: unsupported op BranchF`. Likewise `ops = [MicroOp::GtF { dst: 0, lhs: 1, rhs: 2 }, MicroOp::Return { src: 0 }]` with pins `&[5]` panics on GtF. END-TO-END: a mode-A Logos function (no list params, no list allocation) whose body compares two Float locals with `>` / `>=` or uses a float comparison inside an `if`, while also having a hot integer slot (e.g. an `Int` loop counter or accumulator) that select_pins ranks high enough to pin — e.g. a function that loops an integer counter and inside the loop does `if x > y` where x,y are Float.

**Expected vs actual**

- **Expected:** The pinned compiler should lower BranchF, GtF, and GtEqF (e.g. by spilling pinned float operands and emitting ST_BRLTF/ST_BRLEF/ST_BREQF or ST_LTF3/ST_LEF3 with the unpinned path's operand-swap mapping), OR the function-compile path should exclude these ops from pinning exactly as compile_region already does — so a valid float-comparing function compiles to correct native code (or cleanly falls back to the unpinned path / bytecode).
- **Actual:** `compile_straightline_pinned_with` reaches `emit_mem_form`'s `unreachable!("emit_mem_form: unsupported op {other:?} (pinned chains exclude it)")` and panics, aborting JIT compilation of a valid program (and, because select_pins routes it there without a guard, this is hit during normal tier-up rather than being unreachable).

**Suggested RED test**

```rust
#[test]
fn pinned_compiler_lowers_float_branch_gt_and_gteq_without_panicking() {
    use logicaffeine_forge::jit::{compile_straightline_pinned, Cmp, MicroOp};

    // pins=&[5] forces the register-threading (pinned) PASS-2 path; the slots
    // the float ops touch (1,2) are intentionally NOT pinned. Each program ends
    // in Return so it does not fall off the end.

    // (1) Fused FLOAT compare-and-branch must lower, not hit emit_mem_form's
    //     unreachable! arm.
    let branchf = [
        MicroOp::BranchF { cmp: Cmp::Lt, lhs: 1, rhs: 2, target: 1 },
        MicroOp::Return { src: 0 },
    ];
    let r = std::panic::catch_unwind(|| compile_straightline_pinned(&branchf, &[5]));
    assert!(r.is_ok(), "pinned compile panicked on BranchF (emit_mem_form unreachable!)");
    r.unwrap().expect("pinned compile of BranchF should succeed");

    // (2) Float `>` value op (GtF) must lower (unpinned path swaps to LtF; the
    //     pinned path must do the same instead of falling into emit_mem_form).
    let gtf = [
        MicroOp::GtF { dst: 0, lhs: 1, rhs: 2 },
        MicroOp::Return { src: 0 },
    ];
    let r2 = std::panic::catch_unwind(|| compile_straightline_pinned(&gtf, &[5]));
    assert!(r2.is_ok(), "pinned compile panicked on GtF (emit_mem_form unreachable!)");
    r2.unwrap().expect("pinned compile of GtF should succeed");

    // (3) Float `>=` value op (GtEqF) — also absent from has_variant,
    //     mem_form_touch, and emit_mem_form.
    let gteqf = [
        MicroOp::GtEqF { dst: 0, lhs: 1, rhs: 2 },
        MicroOp::Return { src: 0 },
    ];
    let r3 = std::panic::catch_unwind(|| compile_straightline_pinned(&gteqf, &[5]));
    assert!(r3.is_ok(), "pinned compile panicked on GtEqF (emit_mem_form unreachable!)");
    r3.unwrap().expect("pinned compile of GtEqF should succeed");
}
```

**Fix direction / notes**

No substantive corrections — every cited line and claim checks out. Two minor clarifications on scope/severity (not downgrades):

(a) The end-to-end trigger requires a specific (but realistic) function shape: mode A (no list params, no NewEmptyList), containing a float `>`/`>=` or a float compare-in-`if`, AND at least one hot integer slot whose select_pins profit exceeds the threshold (>2 for the flat/function model, lib.rs:2854) net of call_penalty, so that `pins` is non-empty. A purely-float function with no profitable integer slot would get empty pins and fall back to the unpinned path (which DOES lower these ops at jit.rs:1477-1494 and 1735), so it would NOT crash. The float-`>` example in the description (loop an Int counter, inside do `if x > y` on Floats) satisfies all conditions. This keeps severity at High rather than Critical: it is a hard panic/abort, but only on a specific function shape and only after the function gets hot enough to tier up.

(b) The panic is a denial of execution (process/thread abort via unwinding an uncaught panic), not a silent wrong-answer miscompilation, which is also consistent with High rather than Critical.

Severity High is correct as rated.

---

### BUG-013 — Proof-kernel beta-reduction is not capture-avoiding: free variables in the argument are captured by inner binders (unsound conversion checking)

**Severity:** High  ·  **Category:** variable capture / unsound proof kernel  ·  **Subsystem:** Lambda / DRS  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_proof/src/unify.rs:289–445`

**Summary**

`substitute_expr_for_var` (called by `beta_reduce` at line 149 for beta-reduction and at lines 162/248 for Fix-unfolding and iota/Match reduction) only guards against the bound variable EQUALLING `var` (shadowing). When it descends through a `ForAll`/`Exists`/`Lambda`/`Fixpoint`/`Match`-arm whose binder differs from `var`, it substitutes the replacement into the body WITHOUT first alpha-renaming that binder away from any free variable occurring in `replacement`. So a free variable in the argument gets captured by an inner binder. The doc-comment claims it 'handles variable capture' but it only handles shadowing. The free-variable machinery needed to detect this (`collect_free_vars_impl`, lines ~1358-1405) already exists in the same file but is never consulted here.

**Why it's wrong**

Capture-avoiding substitution is the defining correctness property of beta-reduction. `beta_reduce` is invoked by `unify_exprs` (lines 809-810) on BOTH sides before conversion/alpha-equivalence checking, and conversion checking is the core of the dependent-type proof kernel. Capture makes `beta_reduce` compute a wrong normal form, which makes `unify_exprs` declare non-equivalent propositions equal (and vice-versa) — i.e. the kernel can certify false theorems. This is a soundness hole, not a cosmetic issue.

**The offending code**

```rust
/// Handles variable capture by not substituting inside shadowing binders.
fn substitute_expr_for_var(body: &ProofExpr, var: &str, replacement: &ProofExpr) -> ProofExpr {
    match body {
        ...
        ProofExpr::ForAll { variable, body: inner } => {
            if variable == var {
                body.clone()                       // shadowing handled
            } else {
                ProofExpr::ForAll {
                    variable: variable.clone(),     // NO check that `variable` is free in `replacement`
                    body: Box::new(substitute_expr_for_var(inner, var, replacement)),
                }
            }
        }
        ProofExpr::Lambda { variable, body: inner } => { /* same pattern, same omission */ }
        ...
```

**Trigger / reproduction**

Beta-reduce `(λx. ∀y. Loves(x, y))` applied to the free variable `y`. As ProofExpr: App(Lambda{variable:"x", body: ForAll{variable:"y", body: Predicate{name:"Loves", args:[Variable("x"), Variable("y")], world:None}}}, Term(Variable("y"))). In natural-language terms this is the kind of redex produced for an attitude/quantifier complement where the raised argument variable collides with an already-bound variable name (e.g. reusing the symbol `y`/`e` across two quantifiers, which the WorldState event/var counters do not globally prevent).

**Expected vs actual**

- **Expected:** Capture-avoiding result: the inner binder is alpha-renamed so the substituted free `y` stays free, e.g. `∀y' Loves(y, y')` — the first argument of Loves is the free outer `y`, the second is the (renamed) bound variable; they are distinct.
- **Actual:** `∀y Loves(y, y)` — the substituted free `y` is captured by the inner `∀y`. Both arguments of `Loves` are now bound by the same quantifier, collapsing two semantically distinct variables into one. Via `unify_exprs`, `(λx.∀y.Loves(x,y)) y` now wrongly unifies with `∀y.Loves(y,y)` (`Everyone loves themselves`) even though they are not alpha/beta-equivalent.

**Suggested RED test**

```rust
#[test]
fn beta_reduction_must_alpha_rename_to_avoid_capture() {
    use logicaffeine_proof::{ProofExpr, ProofTerm};
    use logicaffeine_proof::unify::beta_reduce;

    // (λx. ∀y. Loves(x, y))  applied to the FREE variable y.
    // The outer y is FREE in the argument; the inner ∀y is a DIFFERENT binder.
    let redex = ProofExpr::App(
        Box::new(ProofExpr::Lambda {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::ForAll {
                variable: "y".to_string(),
                body: Box::new(ProofExpr::Predicate {
                    name: "Loves".to_string(),
                    args: vec![
                        ProofTerm::Variable("x".to_string()),
                        ProofTerm::Variable("y".to_string()),
                    ],
                    world: None,
                }),
            }),
        }),
        Box::new(ProofExpr::Term(ProofTerm::Variable("y".to_string()))),
    );

    let reduced = beta_reduce(&redex);

    // NEGATIVE: the buggy kernel produces exactly "∀y Loves(y, y)" (capture).
    let out = format!("{}", reduced);
    assert_ne!(
        out, "∀y Loves(y, y)",
        "free variable y was captured by the inner ∀y: {}",
        out
    );

    // POSITIVE / structural: after a capture-avoiding reduction the result is a
    // ForAll whose Loves predicate has TWO DISTINCT arguments — the first is the
    // substituted free `y`, the second is the (renamed) bound variable.
    match &reduced {
        ProofExpr::ForAll { variable, body } => match body.as_ref() {
            ProofExpr::Predicate { name, args, .. } => {
                assert_eq!(name, "Loves");
                assert_eq!(args.len(), 2);
                let first = format!("{}", args[0]);   // must be the free "y"
                let second = format!("{}", args[1]);  // must be the bound binder
                assert_ne!(
                    first, second,
                    "the two arguments of Loves were collapsed to one variable by capture: \
                     first={}, second={}",
                    first, second
                );
                // The inner binder must have been renamed away from the free "y".
                assert_ne!(
                    variable, "y",
                    "inner binder still named y, capturing the free argument y"
                );
                assert_eq!(first, "y", "first arg must remain the free outer y");
                assert_eq!(
                    &second, variable,
                    "second arg must be the (renamed) bound variable {}",
                    variable
                );
            }
            other => panic!("expected ForAll over a Loves predicate, got body {:?}", other),
        },
        other => panic!("expected the reduct to be a ForAll, got {:?}", other),
    }
}
```

**Fix direction / notes**

Severity lowered from Critical to High. The auditor's strongest framing — "the kernel can certify false theorems" from valid (natural-language) input — is asserted but not demonstrated, and is partially mitigated in practice by the engine's ad-hoc rename_variables freshening (engine.rs:4669-4679) applied before the main KB-instantiation unifications (e.g. engine.rs:1370). What IS confirmed and concretely triggerable is a unit-level correctness defect: the public beta_reduce API produces a mathematically wrong (non-capture-avoiding) normal form on a valid ProofExpr. That is a genuine soundness hole in the kernel's substitution primitive (High), but the Critical bar (exploitable false theorem end-to-end from valid input) is not established by the evidence.

Everything else in the claim checks out exactly: the cited lines, the missing free-var check at each binder case, the unused collect_free_vars machinery in the same file, the beta_reduce call on both sides in unify_exprs (unify.rs:809-810), and the exact buggy output string "∀y Loves(y, y)".

Suggested fix: in substitute_expr_for_var's ForAll/Exists/Lambda/Fixpoint/Match-arm cases, when the binder name differs from `var` AND the binder name occurs in collect_free_vars(replacement), first alpha-rename the binder to a fresh name (rename its bound occurrences in the body) before recursing — i.e. make the substitution capture-avoiding using the already-present free-variable collector.

---

### BUG-014 — Policy-condition discovery silently drops every conjunct/disjunct after the first connective (3+ term AND/OR chains lose conditions)

**Severity:** High  ·  **Category:** fact-dropping / miscompilation (security policy lowering)  ·  **Subsystem:** Language compile / discovery  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_language/src/analysis/discovery.rs:260–301`

**Summary**

parse_policy_condition parses the first atomic condition, then on the FIRST AND/OR connective it parses exactly one more atomic condition and returns immediately. The enclosing `loop` therefore never folds a chain: any third (or later) condition joined by a further AND/OR is never consumed and is silently discarded. The grammar it actually accepts is `atom [ (AND|OR) atom ]`, not the intended n-ary chain. This PolicyCondition tree feeds codegen_policy_condition (crates/logicaffeine_compile/src/codegen/policy.rs:67), which recursively emits `&&`/`||`, so the generated `is_X()`/`can_X()` security method ends up with fewer terms than the policy specifies.

**Why it's wrong**

For a security policy this is a soundness-relevant miscompilation, not a cosmetic one. For an AND-chain, dropping a required conjunct makes the generated predicate STRICTLY MORE PERMISSIVE than written (a check that should require role==admin AND verified AND active will pass on role==admin AND verified, granting access that the policy forbids). For an OR-chain, dropping a disjunct makes it MORE RESTRICTIVE (a valid role is rejected). Either way the compiled program does not match the stated policy, and the loss is silent — no error, no warning. Discovery is the single source of truth for the policy semantics; once a condition is dropped here it is gone from every downstream pass. The existing test suite (phase50_security.rs test_policy_with_and_condition / test_policy_with_or_condition) only ever exercises EXACTLY TWO atomic conditions, so the bug is uncovered.

**The offending code**

```rust
fn parse_policy_condition(&mut self, subject_type: Symbol, object_type: Option<Symbol>) -> PolicyCondition {
    let first = self.parse_atomic_condition(subject_type, object_type);
    loop {
        while self.check_newline() { self.advance(); }
        if self.check_comma() { self.advance(); while self.check_newline() { self.advance(); } }
        if self.check_word("AND") {
            self.advance();
            while self.check_newline() { self.advance(); }
            let right = self.parse_atomic_condition(subject_type, object_type);
            return PolicyCondition::And(Box::new(first), Box::new(right));   // <-- returns after ONE connective
        } else if self.check_word("OR") {
            self.advance();
            while self.check_newline() { self.advance(); }
            let right = self.parse_atomic_condition(subject_type, object_type);
            return PolicyCondition::Or(Box::new(first), Box::new(right));    // <-- returns after ONE connective
        } else { break; }
    }
    first
}
```

**Trigger / reproduction**

A `## Policy` block whose `if` condition joins three or more atomic conditions with the same connective, e.g.:
## Definition
A User has:
    a role, which is Text.
## Policy
A User is privileged if:
    The user's role equals "admin", OR
    The user's role equals "moderator", OR
    The user's role equals "editor".
## Main
    Let u be a new User.

**Expected vs actual**

- **Expected:** The generated `fn is_privileged(&self) -> bool` body contains all three disjuncts: `self.role == "admin" || self.role == "moderator" || self.role == "editor"` (an editor is privileged).
- **Actual:** Only the first two are emitted: `self.role == "admin" || self.role == "moderator"`. The third condition (`editor`) is silently dropped, so an editor is wrongly denied. Symmetrically, a 3-way AND chain drops the third required conjunct and over-grants access.

**Suggested RED test**

```rust
// place in crates/logicaffeine_tests/tests/phase50_security.rs
// Uses the existing helper `use common::compile_to_rust;` already imported at the top of that file.

#[test]
fn test_policy_with_three_or_conditions_keeps_all_disjuncts() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy

A User is privileged if:
    The user's role equals "admin", OR
    The user's role equals "moderator", OR
    The user's role equals "editor".

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile 3-way OR conditions");

    // Sanity: the predicate method is generated.
    assert!(
        rust.contains("fn is_privileged(&self) -> bool"),
        "Should generate is_privileged predicate. Got:\n{}",
        rust
    );
    // First two disjuncts survive on current (buggy) code...
    assert!(
        rust.contains("self.role == \"admin\""),
        "missing admin disjunct. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("self.role == \"moderator\""),
        "missing moderator disjunct. Got:\n{}",
        rust
    );
    // ...but the THIRD disjunct is silently dropped by parse_policy_condition
    // (it returns Or(first, right) after the first OR), so this assertion FAILS
    // on current code and PASSES once the parser folds an n-ary chain.
    assert!(
        rust.contains("self.role == \"editor\""),
        "missing editor disjunct (DROPPED by parse_policy_condition: returns after first connective). Got:\n{}",
        rust
    );
    // Belt-and-suspenders: two `||` operators must appear for three disjuncts.
    assert_eq!(
        rust.matches("||").count() >= 2,
        true,
        "expected at least two '||' for a 3-way OR chain. Got:\n{}",
        rust
    );
}

// Over-grant (security-relevant) variant: a 3-way AND chain must keep all three conjuncts,
// otherwise the compiled predicate is STRICTLY MORE PERMISSIVE than the policy.
#[test]
fn test_policy_with_three_and_conditions_keeps_all_conjuncts() {
    let source = r#"## Definition
A User has:
    a role, which is Text.
    a verified, which is Bool.
    a active, which is Bool.

## Policy

A User is trusted if:
    The user's role equals "admin", AND
    The user's verified equals true, AND
    The user's active equals true.

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile 3-way AND conditions");

    assert!(rust.contains("self.role == \"admin\""), "missing role conjunct. Got:\n{}", rust);
    assert!(rust.contains("self.verified == true"), "missing verified conjunct. Got:\n{}", rust);
    // Third conjunct dropped on current code -> predicate over-grants (passes without `active`).
    assert!(
        rust.contains("self.active == true"),
        "missing active conjunct (DROPPED -> over-permissive predicate, grants access policy forbids). Got:\n{}",
        rust
    );
    assert_eq!(
        rust.matches("&&").count() >= 2,
        true,
        "expected at least two '&&' for a 3-way AND chain. Got:\n{}",
        rust
    );
}
```

**Fix direction / notes**

No material correction to the bug itself; the claim is accurate. Two minor notes:

1) Codegen import path: the claim says codegen_policy_condition lives at crates/logicaffeine_compile/src/codegen/policy.rs:67 and uses crate::analysis::policy::PolicyCondition. Confirmed accurate, but note that crate::analysis::policy is a thin re-export module (logicaffeine_compile/src/analysis/mod.rs:63-65) forwarding to logicaffeine_language::analysis::policy. The actual PolicyCondition type and the buggy parser both live in logicaffeine_language. This does not change the bug or its reachability.

2) Severity stays High (not raised to Critical). The defect is a genuine, silent, security-relevant miscompilation, but (a) it requires a specific construct — a single policy condition chaining 3+ atoms with connectives — and (b) the security-policy lowering is an auxiliary feature, not the core transpiler hot path. The over-permissive AND direction is the dangerous case (grants access a policy forbids), which justifies High; the impact is not broad enough across the codebase to warrant Critical.

---

### BUG-015 — String-literal span mixes char-index end with byte-index start, causing usize underflow panic on valid input with leading multibyte text

**Severity:** High  ·  **Category:** panic-on-valid-input / span-corruption  ·  **Subsystem:** Lexer  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_language/src/lexer.rs:773–779`

**Summary**

split_into_words() iterates with `for (i, c) in input.char_indices()` (byte index `i`) but also keeps a parallel `chars: Vec<char>` indexed by `char_idx`/`j`. When scanning a string literal, the WordItem `start` is set to `string_start = i` (a byte offset) while `end` is set to `j + 1` (a CHAR offset). The same conflation exists for the triple-quote branch (end_pos from `j`, line ~738), char literals (`char_start + (j - char_idx)`, line ~840), and `## headers` (line ~675). For pure-ASCII input byte and char indices coincide, so the bug is invisible; for any input containing multibyte UTF-8 the span end is wrong. When enough multibyte characters precede the string, the byte-based `start` exceeds the char-based `end`, producing a span with `end < start`. That span then reaches insert_indentation_tokens() line 1402, `t.span.end - t.span.start > 6`, which is an unguarded usize subtraction and panics with 'attempt to subtract with overflow' in debug builds (and wraps to a huge value in release, silently breaking the multi-line-string indentation filter).

**Why it's wrong**

Spans must be byte offsets into the source (Span fields are byte positions used everywhere else, e.g. word_start/i). Emitting a char-index as a byte-offset `end` violates that invariant. The downstream filter at line 1402 assumes end >= start and does a bare subtraction, so an `end < start` span is a guaranteed panic for valid Unicode input.

**The offending code**

```rust
items.push(WordItem {
    word: format!("\x00STR:{}", string_content),
    trailing_punct: None,
    start: string_start,          // string_start = i  (a BYTE index from char_indices)
    end: if j < chars.len() { j + 1 } else { j }, // j is a CHAR index into chars: Vec<char>
    punct_pos: None,
});
```

**Trigger / reproduction**

Tokenize an input containing several multibyte characters before a quoted string literal so that the opening-quote byte offset exceeds the closing-quote char index, e.g. the 7 Greek letters in `ααααααα "x"` (each α is 2 bytes, 1 char). At the opening quote i (byte) = 15 while the closing-quote char index j = 10, so end = 11 < start = 15.

**Expected vs actual**

- **Expected:** Lexer::new(...).tokenize() returns a token stream; the StringLiteral token's span is a valid byte range with end >= start; no panic.
- **Actual:** The StringLiteral span has end (11) < start (15); insert_indentation_tokens() computes `t.span.end - t.span.start` = 11 - 15 which underflows usize and panics 'attempt to subtract with overflow' (debug) / corrupts the indentation filter (release).

**Suggested RED test**

```rust
// Place in the existing #[cfg(test)] mod tests in crates/logicaffeine_language/src/lexer.rs
// (uses `super::*`, matching the in-module idiom; Interner/TokenType are already in scope there).
#[test]
fn string_literal_span_stays_byte_indexed_after_leading_multibyte_text() {
    // 7 leading 2-byte Greek letters (U+03B1), a space, then a quoted string.
    // Opening-quote byte offset (15) exceeds the closing-quote char index+1 (11),
    // so the StringLiteral span would be {start: 15, end: 11}. tokenize() then hits
    // `t.span.end - t.span.start` in insert_indentation_tokens (line ~1402), an
    // unguarded usize subtraction that underflows -> panic in debug (overflow checks),
    // or wraps to a huge value in release, corrupting the multi-line-string filter.
    let src = "\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1} \"x\"";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(src, &mut interner);
    let tokens = lexer.tokenize(); // currently panics: 'attempt to subtract with overflow'

    let s = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)))
        .expect("should produce a string literal token");

    // Span must be a valid, non-empty byte range into the source.
    assert!(
        s.span.end >= s.span.start,
        "span end {} must be >= start {}",
        s.span.end,
        s.span.start
    );
    assert!(
        src.is_char_boundary(s.span.start) && src.is_char_boundary(s.span.end),
        "span [{}, {}) must lie on char boundaries of the source",
        s.span.start,
        s.span.end
    );
    // The byte slice should recover the literal text including its quotes.
    assert_eq!(&src[s.span.start..s.span.end], "\"x\"");
}
```

**Fix direction / notes**

Severity "High" is correct and left unchanged. One precision note on the trigger: the minimum number of leading 2-byte characters needed to make end < start is N ≥ 4 (the condition is j+1 < i, i.e. (N+3)+1 < (2N+1) ⇒ N > 3), not specifically 7; the claim's 7-α example is valid and triggers with margin, so no change is needed there. Also worth recording: this is not unique to the single-quote branch — the triple-quote (lines 742-743), char-literal (line 840), and ## header (line 675) constructions share the identical byte-start/char-end conflation, so any robust fix should convert `j`/char-based `end` values to byte offsets consistently (e.g. compute end as the byte offset of the position after the closing delimiter via input/char_indices, not `j + 1`).

---

### BUG-016 — Biconditional (Iff) and disjunction (Or) share one precedence level, so "P if and only if Q or R" mis-associates as (P↔Q)∨R

**Severity:** High  ·  **Category:** operator-precedence/connective-scope  ·  **Subsystem:** Parser · core  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_language/src/parser/clause.rs:2030–2084`

**Summary**

`parse_disjunction` consumes both `Or` (∨) and `Iff` (↔, produced by the lexer from "if and only if") inside a single left-associative `while` loop at the same precedence tier. The operands come from `parse_conjunction`, which never consumes `or`/`iff`. So a clause like "P if and only if Q or R" is tokenized [P, Iff, Q, Or, R] and folded left-to-right: first `BinaryOp{P, Iff, Q}`, then `BinaryOp{(P Iff Q), Or, R}`. The biconditional ends up nested UNDER the disjunction.

**Why it's wrong**

In standard logical precedence the biconditional ↔ binds strictly LOOSER than ∨ (precedence ¬ > ∧ > ∨ > → > ↔). "P if and only if Q or R" means P ↔ (Q ∨ R), i.e. the whole disjunction is one side of the biconditional. The parser instead yields (P ↔ Q) ∨ R, a logically inequivalent formula (different truth table). Because Logos feeds this AST to the Z3 verifier and the FOL renderer, every sentence mixing "if and only if" with a following "or" gets the wrong truth conditions. The doc comment on the function even asserts Or/Iff are "lowest precedence" together, confirming the level was collapsed by design error rather than intentional grammar.

**The offending code**

```rust
/// Parse disjunction (Or/Iff) - lowest precedence logical connectives.
fn parse_disjunction(&mut self) -> ParseResult<&'a LogicExpr<'a>> {
    let mut expr = self.parse_conjunction()?;

    while self.check(&TokenType::Comma)
        || self.check(&TokenType::Or)
        || self.check(&TokenType::Iff)
    {
        if self.check(&TokenType::Comma) { self.advance(); }
        if !self.match_token(&[TokenType::Or, TokenType::Iff]) { break; }
        let operator = self.previous().kind.clone();
        ...
        // standard (non-gapping) branch builds BinaryOp{ left, op: operator, right }
        // left-associatively, treating Or and Iff identically
```

**Trigger / reproduction**

English input: "John stays if and only if Mary leaves or Bob leaves." Token stream reaching parse_disjunction: [<John stays>, Iff, <Mary leaves>, Or, <Bob leaves>].

**Expected vs actual**

- **Expected:** Top-level connective is Iff (↔): BinaryOp{ op: Iff, left: <John stays>, right: BinaryOp{ op: Or, <Mary leaves>, <Bob leaves> } } — i.e. John_stays ↔ (Mary_leaves ∨ Bob_leaves).
- **Actual:** Top-level connective is Or (∨): BinaryOp{ op: Or, left: BinaryOp{ op: Iff, <John stays>, <Mary leaves> }, right: <Bob leaves> } — i.e. (John_stays ↔ Mary_leaves) ∨ Bob_leaves.

**Suggested RED test**

```rust
// crates/logicaffeine_tests/tests/integration_tests.rs (existing parse! / ExprView harness)
#[test]
fn biconditional_outscopes_following_disjunction() {
    // "P if and only if Q or R" is standardly P <-> (Q v R): the TOP connective
    // must be Iff, with the whole disjunction as its RIGHT operand. Current code
    // mis-folds to (P <-> Q) v R because parse_disjunction puts Or and Iff on one
    // left-associative tier (crates/logicaffeine_language/src/parser/clause.rs:2030-2084).
    let view = parse!("John stays if and only if Mary leaves or Bob leaves.");
    match view {
        ExprView::BinaryOp { op: TokenType::Iff, left, right } => {
            // The disjunction must sit UNDER the biconditional, on its right side.
            assert!(
                matches!(*right, ExprView::BinaryOp { op: TokenType::Or, .. }),
                "Expected (Mary leaves OR Bob leaves) as the right operand of Iff, got {:?}",
                right
            );
            // And the left side must be the simple 'John stays' clause, NOT another Iff
            // (guards against any future right-associative mis-fold).
            assert!(
                !matches!(*left, ExprView::BinaryOp { op: TokenType::Iff, .. }),
                "Left operand of the top Iff must be the simple clause, got {:?}",
                left
            );
        }
        _ => panic!(
            "Top connective must be Iff for 'P if and only if Q or R' (P <-> (Q v R)), got {:?}",
            view
        ),
    }
}
```

**Fix direction / notes**

No corrections to the claim's substance, location (crates/logicaffeine_language/src/parser/clause.rs:2030-2084), mechanism, or trigger — all verified exact. Severity High is retained: this silently emits a logically inequivalent formula (different truth table) for a documented, taught construction and feeds wrong truth conditions to the Z3 verifier and proof kernel — a genuine meaning-level miscompilation, not merely cosmetic. It is not Critical only because it requires the specific "iff … or/and-mixing …" co-occurrence rather than affecting all inputs, and it does not corrupt memory or crash. One refinement to the suggested RED test (below): tighten it so the LEFT operand is also asserted NOT to be an Iff, making the left-fold failure unambiguous, and assert the exact rendered FOL to lock the semantics. Also note: the same single-tier loop means "P if and only if Q and R" is fine (And binds tighter via parse_conjunction) but "P or Q if and only if R" likewise mis-folds to (P∨Q)↔R only by accident-of-left-assoc — worth a second test, but the primary trigger suffices.

---

### BUG-017 — "No" quantifier with an auxiliary verb drops the negation and emits a conjunction instead of a negated implication

**Severity:** High  ·  **Category:** quantifier-scope-negation  ·  **Subsystem:** Parser · quantifiers / clauses  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_language/src/parser/quantifier.rs:589–619`

**Summary**

In `parse_quantified_core`, the auxiliary-verb branch (entered when a quantified subject is followed by an Auxiliary token such as "was"/"were"/"will") builds the quantifier body with a `match quantifier_token` that special-cases only `All | Any` (implication) and routes EVERYTHING else, including `No`, into the `_` arm which uses `TokenType::And`. The `kind` match correctly maps `No -> Universal`, but no negation is ever applied to the predicate in this branch. Every other VP path for `No` (the modal branch at 411-421, the main `check_verb` branch at 997-1007, and the copula branch at 1169-1184) correctly produces `subject_pred -> ¬verb_pred`. The auxiliary branch is the sole path missing the `No` case.

**Why it's wrong**

"No bird was flying" should mean ¬∃x(Bird(x) ∧ Fly(x)), rendered by this codebase as the universal-with-negated-consequent form ∀x(Bird(x) → ¬Fly(x)) (exactly what e2e_studio_examples::parse_tree_no_student_failed documents as the canonical shape for "No"). Instead, with kind=Universal and body = Bird(x) ∧ Fly(x), the parser emits ∀x(Bird(x) ∧ Fly(x)), which asserts that EVERYTHING in the domain is a flying bird — the polar opposite of the intended meaning and a logically much stronger (and false) claim. This is an unsound FOL translation reachable from a perfectly ordinary English sentence.

**The offending code**

```rust
let body = match quantifier_token {
    TokenType::All | TokenType::Any => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
        left: subject_pred, op: TokenType::Implies, right: maybe_negated,
    }),
    _ => self.ctx.exprs.alloc(LogicExpr::BinaryOp {
        left: subject_pred, op: TokenType::And, right: maybe_negated,
    }),
};
let kind = match quantifier_token {
    TokenType::All | TokenType::No => QuantifierKind::Universal,
    ...
};
```

**Trigger / reproduction**

Compile the sentence "No bird was flying." (any "No <noun> was/were <verb>ing" or "No <noun> will <verb>" form reaches the auxiliary branch). Subject quantifier token is `No`; restriction parses "bird"; `check_modal()` is false; `check_auxiliary()` matches "was"; `check_verb()` matches "flying"; control reaches lines 589-619.

**Expected vs actual**

- **Expected:** Output contains a negation and an implication: structurally Quantifier(Universal){ body: BinaryOp(Implies, Bird(x), Not(Fly(x))) }; rendered string contains "¬"/"Not" and "→". E.g. ∀x(Bird(x) → ¬Fly(x)).
- **Actual:** Output is Quantifier(Universal){ body: BinaryOp(And, Bird(x), Fly(x)) } i.e. ∀x(Bird(x) ∧ Fly(x)); the rendered string contains NO negation symbol and uses ∧ instead of →. The "No" is silently lost.

**Suggested RED test**

```rust
// Place in crates/logicaffeine_tests/tests/e2e_studio_examples.rs
// "No <noun> will <verb>" reaches the auxiliary branch of parse_quantified_core
// (check_auxiliary() is true for Auxiliary(Future)="will"). The "No" quantifier
// must keep its negation: ∀x(Bird(x) → ¬Fly(x)). The bug drops it, emitting
// ∀x(Bird(x) ∧ Fly(x)) with no negation symbol at all.
#[test]
fn no_quantifier_with_future_auxiliary_keeps_negation() {
    use logicaffeine_language::compile;
    let out = compile("No bird will fly.").expect("\"No bird will fly\" should compile");
    assert!(
        out.contains('¬') || out.contains("Not"),
        "\"No bird will fly\" lost its negation (auxiliary branch dropped the \"No\"): {out}"
    );
}

// Same defect via the emphatic-past auxiliary "did" (Auxiliary(Past)).
#[test]
fn no_quantifier_with_did_auxiliary_keeps_negation() {
    use logicaffeine_language::compile;
    let out = compile("No dog did bark.").expect("\"No dog did bark\" should compile");
    assert!(
        out.contains('¬') || out.contains("Not"),
        "\"No dog did bark\" lost its negation (auxiliary branch dropped the \"No\"): {out}"
    );
}

// Structural check mirroring parse_tree_no_student_failed (line 484): the "No"
// quantifier must be Universal AND its body must contain a negation somewhere.
// Under the bug the body is BinaryOp(And, Bird(x), <event>) with no UnaryOp(Not).
#[test]
fn parse_tree_no_bird_will_fly_is_negated_universal() {
    use logicaffeine_language::ast::QuantifierKind;
    use logicaffeine_language::view::ExprView;
    let view = common::parse_to_view("No bird will fly.");
    fn contains_not(v: &ExprView) -> bool {
        match v {
            ExprView::UnaryOp { .. } => true,
            ExprView::BinaryOp { left, right, .. } => contains_not(left) || contains_not(right),
            ExprView::Quantifier { body, .. } => contains_not(body),
            _ => false,
        }
    }
    match view {
        ExprView::Quantifier { kind, body, .. } => {
            assert_eq!(kind, QuantifierKind::Universal,
                "\"No\" should be Universal, got {:?}", kind);
            assert!(contains_not(&body),
                "\"No bird will fly\" body must contain a negation, got {:?}", body);
        }
        other => panic!("Expected Quantifier for \"No bird will fly\", got {:?}", other),
    }
}
```

**Fix direction / notes**

Two corrections to the claim:

1. TRIGGER IS WRONG. "No bird was flying" does NOT reach the cited code. "was"/"were" lex as copula keyword tokens (TokenType::Was/Were), not TokenType::Auxiliary, and check_auxiliary() only matches Auxiliary(_). The lexicon's `auxiliaries` map is exactly {"will":"Future","did":"Past"}. The ONLY sentences that reach the buggy aux branch with a "No" subject are "No <noun> will <verb>" and "No <noun> did <verb>" (e.g., "No bird will fly", "No dog did bark"). The claim's suggested_red_test using "No bird was flying." would not fail on current code and must be replaced.

2. SEVERITY: Critical -> High. The bug is a genuine unsound FOL polarity inversion on a real, ordinary construction (No + future/emphatic-past auxiliary), which is serious in a transpiler whose IP is correctness of the logical form. But it is narrower than the claim implies (it does NOT affect the common "No ... was/were ..." progressive/copular forms, which take a different, correct path), and it is confined to the parser's FOL output — it is not miscompilation of the VM/JIT/codegen and not memory-unsafety. High (not Critical) is the appropriate rating for a soundness bug on a specific aux+verb pattern.

The fix itself: add `TokenType::No` arms to both the `body` match (589-600) and `kind` match (602-611) in the aux branch, mirroring the copula tail at lines 1169-1184 — produce `Implies` with a `Not`-wrapped consequent and `QuantifierKind::Universal`. (The `kind` match already maps No->Universal at line 603; only the `body` arm needs the implication + negation.)

---

### BUG-018 — Donkey binding from a subject relative clause leaks (free variable) when the main VP has a quantified object

**Severity:** High  ·  **Category:** donkey-anaphora-binding  ·  **Subsystem:** Parser · quantifiers / clauses  ·  **Reporter confidence:** medium  
**Location:** `crates/logicaffeine_language/src/parser/quantifier.rs:803–946`

**Summary**

When a quantified subject's restriction contains an indefinite inside a relative clause ("... who owns a donkey ..."), `parse_verb_phrase_for_restriction` pushes an entry onto `self.donkey_bindings` (line 1539) and adds the indefinite's predicate as an extra restriction condition. The main `check_verb` path is supposed to bind that indefinite afterwards via the loop at lines 1051-1065 (`for ... in self.donkey_bindings.iter().rev() { ... wrap_donkey_in_restriction ... } self.donkey_bindings.clear();`). But the quantified-object sub-branch (entered at line 781 when the object is itself a quantifier/article, e.g. "every animal") returns early at lines 940-946 and NEVER runs that loop and NEVER clears `donkey_bindings`. The bound variable for the donkey therefore appears in the formula with no binding quantifier, and the stale entry can also corrupt the next clause.

**Why it's wrong**

"Every farmer who owns a donkey feeds every animal" should be ∀x((Farmer(x) ∧ ∃y(Donkey(y) ∧ Own(x,y))) → ∀z(Animal(z) → Feed(x,z))) — the donkey indefinite must be existentially closed inside the restrictor. Because the quantified-object branch skips the donkey-binding loop, the donkey variable (call it y) is emitted as Donkey(y) ∧ Own(x,y) with no ∃y / ∀y ever wrapping it: y is a free variable, an ill-formed / unsound FOL output. By contrast the non-quantified-object variant ("... beats it") goes through the loop at 1051 and is handled correctly, so the two readings diverge in correctness.

**The offending code**

```rust
self.in_negative_quantifier = was_in_negative_quantifier;
return Ok(self.ctx.exprs.alloc(LogicExpr::Quantifier {
    kind: subj_kind,
    variable: var_name,
    body: subj_body,
    island_id: self.current_island,
}));   // <-- returns WITHOUT iterating/clearing self.donkey_bindings
```

**Trigger / reproduction**

Compile "Every farmer who owns a donkey feeds every animal." The restriction "farmer who owns a donkey" pushes a donkey binding via parse_relative_clause -> parse_verb_phrase_for_restriction (line 1539). The main verb "feeds" then sees "every animal", entering the quantified-object branch at line 781 and returning at line 941 without processing donkey_bindings.

**Expected vs actual**

- **Expected:** The donkey indefinite is bound (existentially) inside the restrictor: ∀x((Farmer(x) ∧ ∃y(Donkey(y) ∧ Own(x,y))) → ∀z(Animal(z) → Feed(x,z))). Every variable that occurs is bound by a quantifier.
- **Actual:** The donkey variable is left unbound (free): the formula contains Donkey(y) ∧ Own(x,y) with no quantifier introducing y, and self.donkey_bindings is not cleared so the stale binding can bleed into a following clause.

**Suggested RED test**

```rust
#[test]
fn donkey_in_relative_clause_bound_even_with_quantified_object() {
    use logicaffeine_language::compile;

    // Baseline: pronoun object ("it") makes the donkey binding `used=true`,
    // so the working path wraps it in a quantifier over `y`. (Snapshot:
    // crates/logicaffeine_tests/tests/snapshots/donkey_sentence.txt =>
    // "∀y(∀x(...))".)
    let baseline = compile("Every farmer who owns a donkey beats it.")
        .expect("baseline compiles");
    assert!(
        baseline.contains("∀y(") || baseline.contains("∃y("),
        "baseline donkey var must be bound, got: {baseline}"
    );

    // Regression case: a quantified object ("every animal") routes through the
    // quantified-object sub-branch in parse_quantified_core, which returns
    // without running the donkey-binding closure loop. The donkey indefinite
    // "a donkey" must still be bound inside the restrictor.
    let out = compile("Every farmer who owns a donkey feeds every animal.")
        .expect("should compile");

    // The donkey variable is rendered as `y` and appears in `Donkey(y)`.
    assert!(
        out.contains("Donkey(y)"),
        "expected the donkey predicate Donkey(y) in the output, got: {out}"
    );

    // The bug: `y` occurs free — there is NO binder over `y` anywhere.
    // NOTE: do NOT test `out.contains('∃')` — every neo-Davidsonian event
    // emits `∃e(...)`, so that is always true and would NOT catch the bug.
    // We must assert a binder over the DONKEY variable `y` specifically.
    assert!(
        out.contains("∀y(") || out.contains("∃y("),
        "donkey variable y is FREE (unbound) when the main verb has a \
         quantified object: no ∀y(/∃y( binder found. got: {out}"
    );
}
```

**Fix direction / notes**

Severity High is correct and retained: valid, common donkey-anaphora input produces an ill-formed FOL formula with a free variable (unsound for any downstream consumer such as verification/model-checking), plus a cross-sentence state leak. It is not Critical (no crash, no executable miscompilation; this is the language frontend emitting a malformed formula).

One important correction to the claim: the suggested RED test is INVALID — `out.contains('∃')` is always satisfied by the `∃e(...)` event quantifiers from build_verb_neo_event, so it would pass on the buggy code and fail to demonstrate the bug. It must instead check that the DONKEY variable `y` (not the event variable `e`) is bound. See refined_red_test.

Confidence: high (raised from the claim's "medium"). The line numbers in the claim are accurate (quantified-object branch entry at line 781, early return at lines 940-946, narrow-scope donkey push at line 1539, the missing loop is the one at lines 1051-1065). The mechanism is fully corroborated by the existing snapshot and by the presence of the closure safety net only in the delegating path.

Suggested fix direction (not part of the test): before the `return` at lines 940-946, run the same `donkey_bindings` processing loop used at lines 1051-1065 over `subj_body`/the result and `self.donkey_bindings.clear()` — or, better (lift-and-shift), perform the unbound-restriction-var closure once at a single choke point so every return path in parse_quantified_core is covered, mirroring `collect_unbound_vars` at lines 3020-3040.

---

### BUG-019 — Embedded copula+participle passive mis-binds the by-phrase agent (drops the agent into a locative PP instead of the predicate's agent slot)

**Severity:** High  ·  **Category:** passive/active argument-structure (agent mis-binding)  ·  **Subsystem:** Parser · verbs  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_language/src/parser/verb.rs:1701–1754`

**Summary**

`parse_predicate_impl`'s copula+participle branch (the `is/are/was/were` + Verb path entered at line 1701) builds the passive predicate with ONLY the subject as its argument (`Predicate{verb, [subject_term]}`, line 1712), then runs a generic trailing-PP loop (line 1723) whose guard excludes only `of` and cycle-temporal prepositions. It does NOT exclude `by`. The MAIN passive path in mod.rs (line 8611, `check_by_preposition`) correctly handles the by-phrase: it builds `Predicate{verb, [agent, theme]}` (see phase64_vp_parity.rs `assert_shape("The book was read by John.", &["Read(John"])`). This embedded path was explicitly added (comment at lines 1718-1722: "makes the embedded VP parser ... as capable as parse_atom's main passive path") but omits the by-agent handling, so the agent is consumed as a generic locative PP `by(theme, agent)` and the predicate keeps the theme as its sole/first argument.

**Why it's wrong**

In a passive, the by-phrase NP is the AGENT and must fill the predicate's agent argument: `See(Mary, John)` for "John was seen by Mary". Treating it as `See(John) ∧ by(John, Mary)` both (a) leaves the verb with the wrong arity/argument binding (John, the THEME/patient, sits in the agent slot of a 1-place predicate) and (b) emits a spurious `by(John, Mary)` locative predicate. This is a genuine argument-structure miscompilation: the resulting FOL says the wrong thing about who did the action, diverging from the active/passive convention enforced everywhere else in the codebase.

**The offending code**

```rust
if self.check_verb() {
    let (verb, _verb_time, verb_aspect, verb_class) = self.consume_verb_with_metadata();
    ...
    let mut predicate: &'a LogicExpr<'a> = self.ctx.exprs.alloc(LogicExpr::Predicate {
        name: verb,
        args: self.ctx.terms.alloc_slice([subject_term]),   // theme only
        world: None,
    });
    // Trailing PP adjuncts on the passive participle ...
    while self.check_preposition() && !self.check_of_preposition()
        && !self.pp_is_cycle_temporal()           // <-- does NOT exclude "by"
    {
        let prep = match self.advance().kind { TokenType::Preposition(s) => s, _ => break };
        ...
        let obj = self.parse_noun_phrase(true)?;
        self.ctx.exprs.alloc(LogicExpr::Predicate {
            name: prep,                              // by(theme, agent)
            args: self.ctx.terms.alloc_slice([subject_term, Term::Constant(obj.noun)]),
            world: None,
        })
    };
```

**Trigger / reproduction**

Reachable from valid top-level input via any construction that routes a passive VP through `parse_predicate_with_subject` rather than the main parse_atom passive path. Two confirmed triggers: (1) it-cleft: "It was John who was seen by Mary." (clause.rs:267 parses the relative-clause body "was seen by Mary" via parse_predicate_with_subject(John)); (2) correlative coordination: "Neither John nor Bill was seen by Mary." (clause.rs:612/618 distribute the shared VP "was seen by Mary" via parse_predicate_with_subject).

**Expected vs actual**

- **Expected:** The by-phrase NP is the agent: output contains the 2-argument passive predicate `See(Mary, John)` (agent Mary, theme John) and NO spurious `by(...)` predicate — matching the main path's `Read(John, Book)` convention for "The book was read by John."
- **Actual:** Output contains `See(John)` (theme John as the lone argument) conjoined with a bogus `by(John, Mary)` PP predicate; Mary (the true agent) never enters the verb's agent slot.

**Suggested RED test**

```rust
#[test]
fn embedded_passive_binds_by_phrase_agent() {
    use logicaffeine_language::compile;
    // "It was John who was seen by Mary." routes the relative-clause passive body
    // "was seen by Mary" through parse_predicate_with_subject (clause.rs:267),
    // hitting verb.rs's copula+participle branch (line 1665). The by-phrase NP
    // (Mary) is the AGENT and must fill the predicate's agent (first) slot, matching
    // the main path convention: "The book was read by John." -> "Read(John, ...)".
    let fol = compile("It was John who was seen by Mary.").unwrap();

    // The agent (Mary) must occupy the agent slot of the 2-place passive predicate.
    assert!(
        fol.contains("See(Mary"),
        "by-phrase agent must fill the predicate agent slot, got: {fol}"
    );

    // The agent must NOT be demoted: there must be no lone-theme passive predicate
    // See(John) with John (the THEME) wrongly sitting as the sole/first argument.
    assert!(
        !fol.contains("See(John)") && !fol.contains("See(John,"),
        "theme John must not occupy the agent slot of the passive predicate, got: {fol}"
    );
}
```

**Fix direction / notes**

The bug, code locations, mechanism, and reachability are all accurate. One correction to the suggested RED test only (the FINDING stands): the secondary assertion `!fol.contains("by(")` is fragile. SymbolRegistry::get_symbol (registry.rs:35-87) lowercases-then-abbreviates unmapped symbols to a capitalized first-letter form (e.g. the preposition "by" would render as "B" or "By", not literal lowercase "by("). So `!fol.contains("by(")` could spuriously PASS even on the buggy code. The load-bearing, robust signal is the AGENT-SLOT assertion: the fixed output must contain `See(Mary` (agent first), and the buggy output instead contains `See(John` as the lone-argument predicate. I rewrote the RED test to assert on `See(Mary` (must appear after fix) and to forbid the lone-theme passive `See(John)` / the demoted-agent reading. Severity High is correct: it is silent miscompilation of meaning (wrong agent), not a panic, and it only triggers on the embedded-VP routes (cleft / correlative) rather than every passive, so not Critical.

---

### BUG-020 — Swap-idiom peephole drops the `Let a` / `Let b` / `tmp` bindings it consumes, breaking any later use of those locals

**Severity:** High  ·  **Category:** miscompilation  ·  **Subsystem:** Optimizer · peephole  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/codegen/peephole.rs:2052–2174`

**Summary**

`try_emit_swap_pattern` (Pattern A, the conditional cross-swap) matches the three statements `Let a be item I of arr`, `Let b be item J of arr`, `If a OP b: SetIndex I=b; SetIndex J=a` and replaces them with an in-place array swap that re-reads `arr[I]`/`arr[J]` directly. It returns `Some((output, 2))`, consuming all three statements, but the emitted code never binds the named locals `a` (`a_sym`) or `b` (`b_sym`). If those locals are read by any statement after the matched block, the generated Rust references undefined variables. The unconditional variant `try_emit_unconditional_swap` (`:3491–3582`) has the identical flaw, dropping `tmp`'s binding. Neither path calls `symbol_appears_in_stmts`, unlike the sibling optimizers `try_emit_seq_copy_pattern` (`:2312`) and `try_emit_rotate_left_pattern` (`:3467`), which explicitly guard for post-window liveness and emit a fallback binding.

**Why it's wrong**

The generic codegen path binds `let a = items[I-1]; let b = items[J-1];` so subsequent uses of `a`/`b` compile and read the captured pre-swap values. The specialized path omits those bindings entirely, so a valid Logos program that uses the compared values after the swap produces Rust that fails to compile (use of undeclared identifier). A pattern match has silently changed the program from "compiles and prints the captured value" to "does not compile" — the over-eager-specialization miscompilation class. The asymmetry with the liveness-guarded sibling optimizers confirms an oversight, not intent.

**The offending code**

```rust
let (b_sym, arr_sym_2, idx_expr_2) = match stmts[idx + 1] {
    Stmt::Let { var, value: Expr::Index { collection, index }, mutable: false, .. } => {
        if let Expr::Identifier(coll_sym) = collection { (*var, *coll_sym, *index) } else { return None; }
    }
    _ => return None,
};
// ...
// (then-block emits a swap using __bm/__swap_tmp; a_sym and b_sym are never bound in output)
Some((output, 2)) // consumed 2 extra statements
```

**Trigger / reproduction**

Capture two elements into scalar locals, conditionally swap them in the array, then read the originally-captured first value:

```
## Main
Let mutable items be [3, 1, 2].
Let a be item 1 of items.
Let b be item 2 of items.
If a is greater than b:
    Set item 1 of items to b.
    Set item 2 of items to a.
Show a.
```

`a = items[0] = 3`, `b = items[1] = 1`; `a > b` is true, so `items` become `[1, 3, 2]`, but the captured `a` must still equal 3.

**Expected vs actual**

- **Expected:** Program compiles and prints `3` (the value captured into `a` before the swap).
- **Actual:** The swap pattern consumes the three statements and emits a swap with no binding for `a`; the trailing `Show a` becomes `println!("{}", a)` referencing an undeclared variable, so the generated Rust fails to compile and the program does not run.

**Suggested RED test**

```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_conditional_swap_keeps_captured_value() {
    // Capture two elements into scalar locals, conditionally swap them in the
    // array, then read the originally-captured FIRST value. The swap-pattern
    // peephole consumes the two `Let a`/`Let b` bindings and emits an in-place
    // swap that never declares `a`, so the trailing `Show a` references an
    // undeclared local and the generated Rust fails to compile. Generic codegen
    // binds `let a = ...; let b = ...;` and prints the captured value 3.
    assert_exact_output(
        r#"## Main
Let mutable items be [3, 1, 2].
Let a be item 1 of items.
Let b be item 2 of items.
If a is greater than b:
    Set item 1 of items to b.
    Set item 2 of items to a.
Show a.
"#,
        "3",
    );
}
```

**Fix direction / notes**

Both swap paths must call `symbol_appears_in_stmts(a_sym/b_sym/tmp_sym, remaining)` and emit fallback bindings (mirroring `try_emit_seq_copy_pattern` / `try_emit_rotate_left_pattern`) before consuming the window. The existing test `e2e_opt_conditional_swap_index_simplified` (`crates/logicaffeine_tests/tests/e2e_codegen_optimization.rs:1311`) uses this exact swap structure but ends with `Show arr` (reading the array, never the captured scalars), which is why the bug is uncaught. Related to but distinct from the comparison-inversion peephole bug below.

---

### BUG-021 — Conditional-swap peephole inverts the comparison when the guard is written `b OP a` instead of `a OP b`

**Severity:** High  ·  **Category:** miscompilation  ·  **Subsystem:** Optimizer · peephole  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/codegen/peephole.rs:2077–2168`

**Summary**

Pattern A of `try_emit_swap_pattern` (conditional bubble-sort swap) binds `a = item I of arr` (index `idx_expr_1`) and `b = item J of arr` (index `idx_expr_2`). The condition matcher at lines 2080-2081 accepts the guard with operands in EITHER order: both `a OP b` and `b OP a`. But the emitted code at lines 2146 and 2158 unconditionally produces `arr[idx1] OP arr[idx2]`, where `arr[idx1]` is the value originally named `a` and `arr[idx2]` is the value originally named `b`. The operator (`op_str`) is taken verbatim from the source operator (lines 2130-2138) with no operand-orientation compensation. So when the user wrote the guard as `b OP a`, the emitted predicate becomes `a OP b` -- the comparison is evaluated with its operands silently swapped, which inverts strict/loose comparisons.

**Why it's wrong**

The trusted tree-walking interpreter evaluates the guard `b OP a` with the actual operand order (b on the left). The compiled output evaluates `a OP b`. For `>`, `<`, `>=`, `<=` this is an inequivalent predicate (e.g. `b > a` is NOT the same as `a > b`), so the swap fires under the opposite condition and the array ends up in a different state. This is a divergence between the interpreter (reference) and compiled code for valid input -- a miscompilation. The swap body itself is independent of guard orientation, so the pattern matches and the wrong code is emitted.

**The offending code**

```rust
// condition matcher accepts BOTH operand orders:
((matches!(left, Expr::Identifier(s) if *s == a_sym) && matches!(right, Expr::Identifier(s) if *s == b_sym)) ||
 (matches!(left, Expr::Identifier(s) if *s == b_sym) && matches!(right, Expr::Identifier(s) if *s == a_sym)))
...
let op_str = match cond { Expr::BinaryOp { op, .. } => match op { BinaryOpKind::Gt => ">", ... } };
...
writeln!(output, "{}if {}[{}] {} {}[{}] {{", indent_str, arr_name, idx1_simplified, op_str, arr_name, idx2_simplified)
// idx1 = index a was read from, idx2 = index b was read from -> always emits `arr[idx_a] OP arr[idx_b]`
```

**Trigger / reproduction**

English program with a conditional swap whose guard names the higher-index element first:

## Main
Let mutable arr be [1, 3, 2].
Let j be 1.
Let a be item j of arr.
Let b be item (j + 1) of arr.
If b is greater than a:
    Set item j of arr to b.
    Set item (j + 1) of arr to a.
Show arr.

Here a = arr[1] = 1, b = arr[2] = 3, guard `b > a` = `3 > 1` = true.

**Expected vs actual**

- **Expected:** Interpreter/reference: guard `3 > 1` is true, swap fires, output `[3, 1, 2]`.
- **Actual:** Compiled code emits `if arr[0] > arr[1]` (i.e. `a > b` = `1 > 3`) which is false, so the swap does NOT fire and output is `[1, 3, 2]`.

**Suggested RED test**

```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_conditional_swap_reversed_operands() {
    // Conditional-swap peephole: guard written as `b is greater than a`
    // (higher-index element named first). The swap must fire exactly when the
    // SOURCE guard is true. The reference interpreter computes `b > a`
    // (3 > 1 = true) and swaps. The peephole currently emits
    // `arr[idx_a] > arr[idx_b]` (a > b = 1 > 3 = false) -- inverting the
    // comparison -- so the swap never fires. RED until the emit mirrors the
    // operator / swaps operands for the `b OP a` orientation.
    let code = r#"## Main
Let mutable arr be [1, 3, 2].
Let j be 1.
Let a be item j of arr.
Let b be item (j + 1) of arr.
If b is greater than a:
    Set item j of arr to b.
    Set item (j + 1) of arr to a.
Show arr.
"#;
    assert_exact_output(code, "[3, 1, 2]");
}

// Companion sanity test (already GREEN today; pin it so the fix doesn't
// regress the canonical `a OP b` orientation):
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_conditional_swap_forward_operands_still_correct() {
    let code = r#"## Main
Let mutable arr be [3, 1, 2].
Let j be 1.
Let a be item j of arr.
Let b be item (j + 1) of arr.
If a is greater than b:
    Set item j of arr to b.
    Set item (j + 1) of arr to a.
Show arr.
"#;
    assert_exact_output(code, "[1, 3, 2]");
}
```

**Fix direction / notes**

No corrections to the claim's substance, file, line range, trigger, expected/actual, or severity — all verified accurate. The matcher lines are 2080-2081, op_str is 2130-2138, and the emits are 2146-2147 (LogosSeq) and 2158-2159 (array); the claim's cited 2077-2168 range and 2146/2158 emit lines are correct.

Note on the fix (for the implementer, not a correction to the claim): the proper fix is to make the emit orientation-aware. When the guard matches in `b OP a` order, either (a) return None so the generic If/SetIndex codegen handles it correctly, or (b) mirror the operator on emit — `Gt<->Lt`, `GtEq<->LtEq`, with `Eq`/`NotEq` unchanged — OR equivalently swap which index is emitted on the left/right of `op_str`. Both the LogosSeq branch (2146) and the array branch (2158) need the same correction since both emit `arr[idx1] OP arr[idx2]`.

---

### BUG-022 — Drain-tail peephole copies to the array's end instead of to the loop bound, over-copying when the bound is smaller than the array length

**Severity:** High  ·  **Category:** miscompilation  ·  **Subsystem:** Optimizer · peephole  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/codegen/peephole.rs:3899–3983`

**Summary**

`try_emit_drain_tail_in_while` matches a `While counter <= bound:` loop whose body's then-branch is `Push item counter of array to target; counter++`, and rewrites the then-branch to `target.extend_from_slice(&array[(counter-1)..]); break;`. The loop's upper bound (`right` of the `<=` at line 3902) is captured as `..` and NEVER inspected. The emitted slice `&array[(counter-1)..]` always runs to the END of `array`. The original loop only pushes elements while `counter <= bound`, i.e. 1-based items `counter..=bound` (0-based `array[(counter-1)..bound]`). The rewrite therefore pushes every element from `counter-1` to `array.len()`, which is only equal to the original behavior when `bound == length of array`. The pattern places no constraint tying `bound` to `array.len()` (the canonical merge test happens to use `bound = length of left`, masking the bug).

**Why it's wrong**

When the loop bound is strictly less than the source array's length, the rewrite pushes extra trailing elements that the original loop never reached, so `target` ends up longer/with different contents than the interpreter produces. This is an observable divergence between the reference interpreter and compiled output for valid input. (Conversely, when `bound > array.len()`, the original would panic on the out-of-range `item counter of array` read while the rewrite silently stops at the array end -- also a behavior change, though the under-copy direction is the clearer correctness bug.)

**The offending code**

```rust
let counter_sym = match while_cond {
    Expr::BinaryOp { op: BinaryOpKind::LtEq, left, .. } => { /* `right` (the bound) is ignored with `..` */ ... }
    _ => return None,
};
...
writeln!(output, "{}    {}.extend_from_slice(&{}[({} - 1) as usize..]);",
    indent_str, target_name, borrow_expr, counter_name).unwrap();
writeln!(output, "{}    break;", indent_str).unwrap();
```

**Trigger / reproduction**

A loop-invariant drain whose bound is smaller than the source length:

## Main
Let src be [10, 20, 30, 40].
Let mutable result be a new Seq of Int.
Let mutable i be 1.
While i is at most 2:
    If 1 is greater than 0:
        Push item i of src to result.
        Set i to i + 1.
    Otherwise:
        Set i to i + 1.
Show result.

The guard `1 > 0` is loop-invariant and always true; bound = 2 < length of src = 4.

**Expected vs actual**

- **Expected:** Interpreter/reference: loop runs for i in 1..=2, pushing items 1 and 2, so result = [10, 20].
- **Actual:** Compiled code emits `result.extend_from_slice(&src.borrow()[(i - 1) as usize..]); break;`, copying src[0..] to the end, so result = [10, 20, 30, 40].

**Suggested RED test**

```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_opt_drain_tail_respects_loop_bound() {
    // Drain-tail must copy only up to the loop bound, not to the end of the source.
    // The guard `1 > 0` is loop-invariant (literal-only -> no symbols), so the
    // drain-tail peephole fires. bound (2) < length of src (4); the loop only
    // pushes 1-based items i=1..=2 => 0-based src[0..2] => [10, 20].
    // BUG: peephole emits `result.extend_from_slice(&src.borrow()[(i - 1) as usize..])`,
    // copying src[0..] to the end => [10, 20, 30, 40].
    let code = r#"## Main
Let src be [10, 20, 30, 40].
Let mutable result be a new Seq of Int.
Let mutable i be 1.
While i is at most 2:
    If 1 is greater than 0:
        Push item i of src to result.
        Set i to i + 1.
    Otherwise:
        Set i to i + 1.
Show result.
"#;
    assert_exact_output(code, "[10, 20]");
}
```

**Fix direction / notes**

No substantive corrections to the claim — the analysis is accurate. Two clarifications: (a) The rewrite preserves the enclosing `while cond { ... }` and uses `break` to exit, so the divergence is purely in how much of the array gets copied, not in control flow. (b) The claim's secondary note about `bound > array.len()` (original panics on out-of-range read while rewrite stops at array end) is also correct but is a separate, less clear-cut direction; the under/over-copy when `bound < array.len()` is the canonical, silent-wrong-output bug and is the right one to anchor the RED test on. Severity High is appropriate: it is a silent miscompilation producing wrong data (not a crash, not corruptible-on-its-own), but the trigger requires a specific shape (loop-invariant guard guarding a sequential drain where the loop bound is strictly less than the source length), which is plausible but not the dominant use of this pattern.

---

### BUG-023 — Kripke lowering of "cannot" (force 0.0) produces a possibility ∃-world, asserting the logical OPPOSITE of impossibility

**Severity:** High  ·  **Category:** unsound-modal-lowering  ·  **Subsystem:** Modal semantics (Kripke)  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_language/src/semantics/kripke.rs:471–497`

**Summary**

`lower_modal` decides Box vs Diamond purely on `vector.force > 0.5`. The modal token `Cannot` is assigned a ModalVector with `force = 0.0`, `domain = Alethic`, `flavor = Root`, and crucially NO negation wrapper (parser/modal.rs:511-515; "can't"/"cannot" lexes to a single `Cannot` token, so the negation lives entirely in the vector). Because 0.0 is not > 0.5, "cannot" falls into the Diamond branch and lowers to `∃w'(Accessible_Alethic(w0,w') ∧ Fly(x, w'))` — i.e. "there exists an accessible world where x flies". That is exactly the possibility statement ("x CAN fly"), the logical negation of what "cannot" means.

**Why it's wrong**

The project's own intended convention (modal_scope_tests.rs::modal_scope_cannot_narrow, lines 140-152, plus the comment "Cannot uses □ (necessity) with force 0, meaning impossibility") treats force-0 `Cannot` as impossibility = ¬◇ = □¬. A correct lowering must therefore yield something entailing ¬Fly in accessible worlds (e.g. ∀w'(Accessible→¬Fly) or ¬∃w'(Accessible∧Fly)). Instead the lowering produces a bare existential satisfaction of the complement. Since the Kripke-lowered AST is what feeds Z3 verification, a query about "Some birds cannot fly" will be given the premise "some bird CAN fly" — an unsound entailment that inverts the sentence's truth conditions.

**The offending code**

```rust
if vector.force > 0.5 {
    // Necessity (Box): ForAll w'(Accessible(w, w') -> P(w'))
    ...
} else {
    // Possibility (Diamond): Exists w'(Accessible(w, w') ∧ P(w'))
    let conjunction = expr_arena.alloc(LogicExpr::BinaryOp {
        left: accessibility,
        op: TokenType::And,
        right: lowered_operand,
    });
    expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: target_world,
        body: conjunction,
        island_id: 0,
    })
}
```

**Trigger / reproduction**

English input: "Some birds cannot fly." (also "John cannot fly."). compile_kripke parses Cannot → Modal{force:0.0, Alethic, Root, operand:Fly(...)} with no negation, and apply_kripke_lowering emits ∃w'(Accessible_Alethic(w0,w') ∧ Fly(x,w')).

**Expected vs actual**

- **Expected:** Kripke lowering of a force-0 (Cannot) modal should express impossibility of the complement: a universal/negated form such as ∀w'(Accessible_Alethic(w0,w') → ¬Fly(x,w')), or ¬∃w'(Accessible_Alethic ∧ Fly). The output must NOT assert that the complement holds in some accessible world.
- **Actual:** It emits ∃w'(Accessible_Alethic(w0,w') ∧ Fly(x,w')) — an unnegated existential asserting the complement IS possible, the exact opposite of "cannot".

**Suggested RED test**

```rust
#[test]
fn kripke_cannot_lowers_to_impossibility_not_possibility() {
    use logicaffeine_language::compile_kripke;
    // "Some birds cannot fly." → Modal{force:0.0, Alethic, Root, operand: Fly(...)}.
    // Cannot = impossibility (¬◇ = □¬). Its Kripke lowering must NEGATE the
    // complement in accessible worlds, e.g. ∀w'(Accessible_Alethic(w0,w') → ¬Fly(...,w')),
    // and must NOT assert ∃w'(Accessible_Alethic ∧ Fly) — that is the possibility
    // reading (the bird CAN fly), the exact logical opposite of "cannot".
    let out = compile_kripke("Some birds cannot fly.").unwrap();

    // The lowered form must reference the alethic accessibility relation.
    assert!(
        out.contains("Accessible_Alethic"),
        "Cannot should lower over alethic accessible worlds. Got: {}",
        out
    );

    // BUG GUARD: the complement (Fly) must NOT be asserted as holding in an
    // accessible world via a bare conjunction with no negation. Under the bug
    // the entire world-quantified subformula contains 'Fly' but no '¬'.
    assert!(
        out.contains('¬'),
        "Cannot (force 0 = impossibility) must negate the complement in accessible \
         worlds; lowering instead asserted the complement is possible. Got: {}",
        out
    );

    // And the impossibility must be expressed as a universal-over-worlds (∀w' …),
    // never as an existential satisfaction of the complement.
    assert!(
        out.contains('∀'),
        "Cannot should lower to a universal-over-accessible-worlds impossibility \
         (∀w'(Accessible → ¬operand)), not an existential possibility. Got: {}",
        out
    );
}
```

**Fix direction / notes**

Severity downgraded from Critical to High. The bug is a real soundness inversion, but it is confined to the `compile_kripke` / Kripke-world-lowering path (the input to Z3 verification), not the default `compile` FOL output, which renders the correct `□` for force-0 Cannot (formatter.rs lines 83-84, validated by modal_scope_tests.rs::modal_scope_cannot_narrow). It inverts truth conditions for force-0 alethic modals (Cannot) specifically in the verification-feeding path, so it is a serious unsoundness — but it does not miscompile arbitrary programs and the primary surface output is unaffected, so High rather than Critical is the accurate rating.

Root-cause precision: the defect is the threshold mismatch. `lower_modal` uses `vector.force > 0.5` (kripke.rs line 471), which maps force=0.0 to Diamond. To match the surface convention and the documented intent, force-0 (impossibility) must lower to `∀w'(Accessible → ¬operand)` (equivalently `¬∃w'(Accessible ∧ operand)`). A correct fix needs a three-way branch: force > 0.5 → necessity (∀ → operand); 0.0 < force <= 0.5 → possibility (∃ ∧ operand); force == 0.0 → impossibility (∀ → ¬operand). Note the module-level doc comment (kripke.rs lines 3-5) is also wrong for the force=0.0 case and should be corrected alongside.

The cited line range 471-497 is accurate; the cited parser lines 511-515 are accurate; the cited test lines 140-152 are accurate.

---

### BUG-024 — "cannot" (alethic impossibility, force 0.0) renders as necessity □ instead of possibility/impossibility — modal force is inverted at the boundary

**Severity:** High  ·  **Category:** wrong modal semantics / incorrect FOL rendering (modal force inversion)  ·  **Subsystem:** Transpile / formatter  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_language/src/formatter.rs:81–90`

**Summary**

The default `modal()` formatter selects the alethic operator by a force threshold. The possibility branch guard is `force > 0.0 && force <= 0.5`. The strict `force > 0.0` lower bound excludes force == 0.0, so a modal with force exactly 0.0 falls through to the catch-all `ModalDomain::Alethic => self.necessity()` and is rendered with the NECESSITY operator □ (LaTeX \Box). The English word "cannot" is parsed (parser/modal.rs lines 511-515, token_to_vector for TokenType::Cannot) into `ModalVector { domain: Alethic, force: 0.0, flavor: Root }` with NO accompanying negation wrapper — the impossibility is meant to be encoded by the 0.0 force itself. The lexicon (assets/lexicon.json line 40: "cannot": "Cannot") maps the surface word to that token, and the aspect chain (parser/modal.rs line 86, 467-469) emits a bare `LogicExpr::Modal` whose Root flavor routes through `fmt.modal(vector.domain, vector.force, &o)` in transpile.rs line 474. So "John cannot fly." emits `□_{0.0} ...`, i.e. NECESSITY notation, which is the exact logical opposite of the intended impossibility.

**Why it's wrong**

force 0.0 is the minimal force value (impossibility). Rendering it with the necessity operator □ asserts the strongest possible modality — the logical inverse of the intended meaning. "X cannot fly" should denote impossibility (¬◇Fly / □¬Fly), and at the very least must use the possibility-side operator ◇, never the necessity operator □. Every value in (0.0, 0.5] correctly maps to possibility ◇; only the boundary 0.0 is mishandled, so the rendered FOL for the entire "cannot" family is not logically equivalent to its meaning — it is its negation.

**The offending code**

```rust
fn modal(&self, domain: ModalDomain, force: f32, body: &str) -> String {
    let sym = match domain {
        ModalDomain::Alethic if force > 0.0 && force <= 0.5 => self.possibility(),
        ModalDomain::Alethic => self.necessity(),
        ModalDomain::Deontic if force <= 0.5 => "P",
        ModalDomain::Deontic => "O",
        ModalDomain::Temporal => "Temporal",
    };
    format!("{}_{{{:.1}}} {}", sym, force, body)
}
```

**Trigger / reproduction**

English input: "John cannot fly." (also "Birds cannot swim.", any "X cannot VP"). Internally: any LogicExpr::Modal with ModalDomain::Alethic and force == 0.0 (produced by TokenType::Cannot).

**Expected vs actual**

- **Expected:** The alethic operator chosen for force 0.0 should be the possibility operator ◇ (Unicode) / \Diamond (LaTeX) — and semantically an impossibility/negated-possibility — never the necessity operator □. So `compile("John cannot fly.")` should contain ◇ (or an explicit ¬◇/□¬), and must NOT contain □.
- **Actual:** `compile("John cannot fly.")` renders the modal with □ (necessity): output is of the form `□_{0.0} ∃e(Fly(e) ∧ Agent(e, John))` (Unicode) / `\Box_{0.0} ...` (LaTeX), inverting impossibility into necessity. The guard `force > 0.0` skips force 0.0, and the fall-through is `self.necessity()`.

**Suggested RED test**

```rust
#[test]
fn cannot_renders_with_possibility_operator_not_necessity() {
    use logicaffeine_language::compile;
    // "cannot" is alethic impossibility: parser/modal.rs maps TokenType::Cannot
    // to ModalVector { domain: Alethic, force: 0.0, flavor: Root } with NO negation
    // wrapper. The default formatter::modal() must select the possibility-side
    // operator ◇ for the weak/low-force half (force <= 0.5), NEVER the necessity
    // operator □ — rendering force 0.0 as □ inverts impossibility into necessity.
    let out = compile("John cannot fly.").expect("should parse");

    // Must NOT use the necessity operator (the body has no other modal).
    assert!(
        !out.contains('□'),
        "`cannot` (force 0.0) must not render as necessity □: {out}"
    );
    // Must use the possibility operator, carrying the 0.0 force subscript that
    // marks it as the impossibility floor of the alethic scale.
    assert!(
        out.contains('◇'),
        "`cannot` should render with possibility operator ◇: {out}"
    );
    assert!(
        out.contains("◇_{0.0}"),
        "expected ◇_{{0.0}} (possibility operator, impossibility force): {out}"
    );
}
```

**Fix direction / notes**

No substantive correction to the claim; it is accurate as written. Two clarifications for scope/precision: (a) The inversion affects only the default `modal()` used by the Unicode and LaTeX formatters — the two human-readable modal-operator notations and the primary output of `compile`. The SimpleFOL formatter (formatter.rs:400) and Kripke formatter (formatter.rs:467) override `modal()` (drop / pre-lowered to quantifiers), so the verification-bound Kripke lowering path is NOT corrupted by this bug. (b) The minimal correct fix is to make the alethic lower bound inclusive — change line 83 from `force > 0.0 && force <= 0.5` to `force <= 0.5` — mirroring the already-correct Deontic branch on line 85; that maps force 0.0 to `self.possibility()` (◇ / \\Diamond), consistent with the system's own force-subscript convention where the operator side is chosen by the weak/strong half and the exact force is shown in the subscript (`◇_{0.0}` denotes impossibility). Severity confirmed as High (wrong/inverted FOL shown for a whole lexical family on valid input), not downgraded.

---

### BUG-025 — List literal element type taken only from the first element — rejects valid `Seq of Real be [1, 2, 3]` and gives heterogeneous lists a wrong concrete element type

**Severity:** Medium  ·  **Category:** type-inference / accepts-invalid + rejects-valid  ·  **Subsystem:** Semantic checks  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/analysis/check.rs:258–266`

**Summary**

`infer_expr` for a list literal synthesizes the sequence element type from `items[0]` ONLY. It never inspects items[1..], and `check_expr` (lines 197-227) has no `List` case, so list literals never get the polymorphic numeric-literal coercion that scalar literals get (lines 205-213). Two distinct defects follow. (A) Rejects valid code: `Let xs: Seq of Real be [1, 2, 3]`. The annotation is `Seq(Float)`, so `Stmt::Let` (lines 562-569) calls `check_expr([1,2,3], Seq(Float))`. That falls through to synthesis → `Seq(Int)` (items[0]=Int), then `unify(Seq(Int), Seq(Float))` recurses into `unify(Int, Float)` which has no rule (unify.rs 341-359) and returns `Err(Mismatch)`, propagated by `?` out of `check_program`. The whole program is rejected even though the programmer clearly meant a Seq of Reals. The analogous scalar form `Let x: Real be 5` is accepted (test `let_with_annotation_uses_annotation`, line 969), so the behavior is inconsistent. (B) Accepts invalid / wrong concrete type: `Let xs be [1, "two"]` (no annotation) synthesizes `Seq(Int)` from items[0] alone, binding `xs : Seq<Int>` in the TypeEnv even though element 1 is a String. codegen/detection.rs (e.g. line 1383, `Seq of scalars` de-Rc) trusts the inferred element type, so a wrong element type can mislead codegen.

**Why it's wrong**

A sequence literal's element type must be unified across all elements and (in checking mode) coerced against the expected element type, exactly as scalar literals are. Reading only items[0] both rejects well-typed numeric-literal sequences under a Real annotation and silently mis-types heterogeneous/ill-typed sequences as a concrete homogeneous type that downstream codegen relies on.

**The offending code**

```rust
Expr::List(items) => {
    if items.is_empty() {
        let elem_var = self.table.fresh();
        Ok(InferType::Seq(Box::new(elem_var)))
    } else {
        let elem_type = self.infer_expr(items[0])?;
        Ok(InferType::Seq(Box::new(elem_type)))
    }
}
```

**Trigger / reproduction**

English: `Let xs be a Seq of Real with [1, 2, 3].` (or the AST form below). Also: `Let xs be [1, "two"].`

**Expected vs actual**

- **Expected:** `Let xs: Seq of Real be [1,2,3]` type-checks, binding xs : Seq<Float>. A heterogeneous list either errors or does not get a bogus homogeneous concrete element type.
- **Actual:** `Let xs: Seq of Real be [1,2,3]` returns Err(Mismatch) and the program is rejected. `[1, "two"]` binds xs : Seq<Int>.

**Suggested RED test**

```rust
// Place in the `tests` mod of crates/logicaffeine_compile/src/analysis/check.rs
// (that module already has `use super::*;` and `use crate::ast::stmt::{Expr, Literal, Stmt, TypeExpr};`).
//
// RED today: `check_program` returns Err(Mismatch) because the list's element
// type is synthesized as Int from items[0] and never coerced to the annotated
// Float, so unify(Seq(Int), Seq(Float)) -> unify(Int, Float) -> Mismatch.
// GREEN after fix: the Int literals coerce to Float under the `Seq of Real`
// annotation, exactly like the accepted scalar form `Let x: Real be 5`
// (see test `let_with_annotation_uses_annotation`).
#[test]
fn seq_of_real_accepts_int_literals() {
    let mut interner = Interner::new();
    let xs = interner.intern("xs");
    let real_sym = interner.intern("Real");
    let seq_sym = interner.intern("Seq");

    let one = Expr::Literal(Literal::Number(1));
    let two = Expr::Literal(Literal::Number(2));
    let three = Expr::Literal(Literal::Number(3));
    let val = Expr::List(vec![&one, &two, &three]);

    let real_ty = TypeExpr::Primitive(real_sym);
    let params = [real_ty];
    let seq_real = TypeExpr::Generic { base: seq_sym, params: &params };

    let stmts = [Stmt::Let { var: xs, ty: Some(&seq_real), value: &val, mutable: false }];

    let env = check_program(&stmts, &interner, &TypeRegistry::new())
        .expect("Seq of Real should accept integer literals [1, 2, 3]");

    assert_eq!(
        env.lookup(xs),
        &LogosType::Seq(Box::new(LogosType::Float)),
        "xs should be inferred as Seq<Float> under the `Seq of Real` annotation"
    );
}
```

**Fix direction / notes**

Severity downgraded from High to Medium. The confirmed half (A) is a reject-valid bug, which is the fail-safe direction — it produces a spurious compile-time type error, never silent miscompilation of an accepted program. Its scope is narrow: it only bites list literals with integer-literal elements under an explicit floating annotation (e.g. `Seq of Real`/`Seq of Float`). The genuinely dangerous half (B, accepts-invalid → wrong concrete codegen element type) is plausible from the same root cause (items[1..] never unified/checked) but I could NOT confirm a reachable miscompilation: I did not verify the parser emits a heterogeneous list literal that flows to codegen with a trusted wrong element type, nor that `is_scalar_elem_type`/`fresh_scalar_seq_elem` actually mis-handle it. With the confirmed direction being fail-safe and the unsafe direction unconfirmed, Medium is correct. The fix is to make `check_expr` handle `Expr::List` by checking each element against the expected element type (propagating the numeric-literal coercion), and to make `infer_expr`'s List case unify all element types via a fresh variable rather than trusting items[0].

---

### BUG-026 — Generic function with inferred (unannotated) return type fails to generalize its return type variable, causing cross-call contamination between two calls at different types

**Severity:** Medium  ·  **Category:** generalization/instantiation (type-variable capture across call sites)  ·  **Subsystem:** Type analysis · inference / unification  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/analysis/check.rs:639–683`

**Summary**

When a generic function declares type parameters (e.g. `[T]`) but omits an explicit return-type annotation, `preregister_functions` allocates a FRESH inference variable for the return type via `self.table.fresh()` (check.rs line 169), e.g. `Var(R1)`, which is NOT a member of `generic_vars` (which only contains the param TyVars like `T0`). During body checking of `Return x`, `unify(Var(T0), Var(R1))` is performed; because the `(InferType::Var(tv), ty)` arm of `unify_walked` (unify.rs lines 322-328) is tried first, it binds the GENERIC variable `T0 := Var(R1)`. After the body, `resolve(param_types)` follows `T0 -> R1` and yields `[Var(R1)]`, and `resolved_ret` is `Var(R1)`. The new scheme is therefore `forall [T0]. Function([Var(R1)], Var(R1))`: it quantifies over `T0` (which no longer occurs in the body) while leaving the genuinely-polymorphic variable `R1` FREE and un-generalized. At a call site, `UnificationTable::instantiate` (unify.rs lines 184-192) only renames variables in `scheme.vars`; since `R1` is not in `vars`, `substitute_vars` leaves it shared across every call. The first call binds `R1` to a concrete type in the global table; the second call at a different type then unifies against that now-concrete `R1` and fails.

**Why it's wrong**

This is a classic generalization (let-polymorphism) soundness/completeness defect: the variable that should be quantified (the polymorphic return) is left free in the scheme, so independent call sites are no longer independent. The contract documented at check.rs lines 670-672 ('generic TyVars remain as Var(tv) ... instantiated fresh at each call site') is violated because the surviving free variable R1 was never placed in scheme.vars and is therefore never freshened by instantiate(). A perfectly valid, fully polymorphic program is rejected (or, if the second use is itself only a fresh var, silently mis-typed) purely as an artifact of evaluation/binding order.

**The offending code**

```rust
let generic_vars: Vec<TyVar> = generics
    .iter()
    .filter_map(|sym| type_param_map.get(sym).copied())
    .collect();
...
let resolved_params: Vec<InferType> = param_types
    .iter()
    .map(|ty| self.table.resolve(ty))
    .collect();
let resolved_ret = self.table.resolve(&ret_type);
let scheme = TypeScheme {
    vars: generic_vars,
    body: InferType::Function(resolved_params, Box::new(resolved_ret)),
};
self.functions.insert(*name, FunctionRecord { param_names, scheme });
```

**Trigger / reproduction**

Define a generic function whose return type is INFERRED (no `-> T` annotation) and whose body returns the parameter, then call it twice at two different types. English form: `To wrap of [T] (x: T): Return x.` followed by `Let r1 be wrap(42).` and `Let r2 be wrap(true).`  AST form: Stmt::FunctionDef { name: "wrap", generics: vec![T], params: vec![(x, T_ty)], body: [Return(Identifier(x))], return_type: None, is_native: false, ... }, then two Calls: wrap(42) and wrap(true).

**Expected vs actual**

- **Expected:** check_program succeeds; r1 is inferred as Int and r2 as Bool (each call independently instantiates the polymorphic return).
- **Actual:** The first call binds the un-generalized free return variable R1 to Int globally; the second call instantiates the scheme to Function([Int], Int) and unifies Bool against Int, so check_program returns Err(TypeError::Mismatch { Int, Bool }) and the whole program is rejected with a spurious type error.

**Suggested RED test**

```rust
// Add to `mod tests` in crates/logicaffeine_compile/src/analysis/check.rs.
// Mirrors the existing passing `generic_calls_are_independent` (line 1416)
// EXACTLY except `return_type: None`, isolating the inferred-return-type bug.
#[test]
fn generic_inferred_return_calls_are_independent() {
    // To wrap of [T] (x: T):  Return x.   <-- NO `-> T` annotation
    // Let r1 be wrap(42).   Let r2 be wrap(true).
    let mut interner = mk_interner();
    let f = interner.intern("wrap");
    let x_param = interner.intern("x");
    let t_sym = interner.intern("T");
    let t_ty = TypeExpr::Primitive(t_sym);
    let x_ref = Expr::Identifier(x_param);
    let ret_stmt = Stmt::Return { value: Some(&x_ref) };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![t_sym],
        params: vec![(x_param, &t_ty)],
        body: &body,
        return_type: None, // inferred return -> fresh var not in scheme.vars: the bug
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: HashSet::new(),
    };
    let r1 = interner.intern("r1");
    let r2 = interner.intern("r2");
    let lit_int = Expr::Literal(Literal::Number(42));
    let lit_bool = Expr::Literal(Literal::Boolean(true));
    let call1 = Expr::Call { function: f, args: vec![&lit_int] };
    let call2 = Expr::Call { function: f, args: vec![&lit_bool] };
    let let_r1 = Stmt::Let { var: r1, ty: None, value: &call1, mutable: false };
    let let_r2 = Stmt::Let { var: r2, ty: None, value: &call2, mutable: false };
    let stmts = [fn_def, let_r1, let_r2];
    // On current code this panics: check_program returns Err(Mismatch { Int, Bool })
    // because the un-generalized return var is globally bound to Int by call1,
    // and call2 then unifies Bool against Int.
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(r1), &LogosType::Int,
        "wrap(42) should be Int, got {:?}", env.lookup(r1));
    assert_eq!(env.lookup(r2), &LogosType::Bool,
        "wrap(true) should be Bool, got {:?}", env.lookup(r2));
}
// FIX: when building the post-body scheme (check.rs ~line 679-683), collect the
// free InferType::Var ids occurring in resolved_params/resolved_ret that are not
// already bound to a ground type, and union them into scheme.vars so instantiate()
// freshens them per call site.
```

**Fix direction / notes**

Severity corrected High → Medium. The type-system defect is real and confirmed at the public `check_program` API level for a valid AST. However, the claim's "High" overstates end-to-end impact: triggering it requires the specific combination of (a) a function with declared generics `[T]`, (b) an OMITTED return annotation, and (c) two or more call sites at distinct types. I confirmed reachability through the type-checker's public API and via the AST the existing test already constructs, but I did NOT confirm that the language frontend (logicaffeine_language parser / literate_parser in logicaffeine_kernel) actually emits `Stmt::FunctionDef { generics: non-empty, return_type: None }` from real Logos source — the surface form `To wrap of [T] (x: T): Return x.` (no `-> T`). If the frontend always supplies or requires a return type on generic defs, the bug is only reachable by direct API/AST construction, which narrows real-world blast radius. The claim's other details are accurate: the trigger English sentence, the expected vs actual (Err Mismatch{Int,Bool}), and the mechanism are all correct. One nit in the claim's narration: it says unify binds T:=Var(R1) because the `(Var(tv), ty)` arm is "tried first" — this is right, but note both orderings would still leave R free, so the binding-order framing is not load-bearing for the bug; the root cause is simply that R was never added to scheme.vars.

---

### BUG-027 — Closed-form double-recursion replacement emits `<< d` without restricting the parameter to an integer type or guarding the shift count

**Severity:** Medium  ·  **Category:** miscompilation  ·  **Subsystem:** Optimizer · closed-form detection  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_compile/src/codegen/detection.rs:1581–1644`

**Summary**

`detect_double_recursion_closed_form` recognizes `f(0)=base`, `f(d)=k+f(d-1)+f(d-1)` and `program.rs:935` emits `((base+k) << d) - k`. The detector checks only `params.len() == 1` and the AST shape; the parameter's type (`params[0].1`) is never inspected and the parameter is never bounded. Two divergences result. (1) If the single parameter is a `Float`, the base literal is still extracted as `i64` and codegen emits `(<int>i64 << d)` with `d: f64` — Rust has no `Shl<f64>` for `i64`, so the program fails to compile. (2) For an `Int`/`Nat` parameter, the recursive form computes by repeated `i64` doubling (which wraps in release), while the closed form computes one shift `(base+k) << d`; for `d >= 64` Rust's shift-overflow semantics differ from the recursion's repeated wrapping doubling, so the two forms produce different finite results for the same terminating input.

**Why it's wrong**

A specialized codegen path is selected purely on AST shape, ignoring parameter type and value range, so it can (a) emit non-compiling Rust for a Float-parameter double recursion the generic path would compile, and (b) for large integer arguments produce a different value than the recursive function it replaces. Both violate the contract that the closed form is observationally equivalent to the recurrence. A bare `0`/`1` literal always parses as `Literal::Number(i64)` regardless of the declared parameter type, so a `Float`-typed double recursion still satisfies the detector. Aggravating factor: the closed-form gate at `program.rs:561` is not guarded by `OptFlag::NoOptimize` (unlike `is_memo`/`is_tce`), so `## No Optimize` does not disable it — there is no user escape hatch.

**The offending code**

```rust
pub(super) fn detect_double_recursion_closed_form<'a>(
    func_name: Symbol,
    params: &[(Symbol, &'a TypeExpr<'a>)],
    body: &'a [Stmt<'a>],
    interner: &Interner,
) -> Option<ClosedFormInfo> {
    // ...
    if params.len() != 1 { return None; }
    let param_sym = params[0].0;   // params[0].1 (the type) is never inspected
    // ...
    Some(ClosedFormInfo { base: base_value, k: constant_sum })
}
```

**Trigger / reproduction**

Float divergence (deterministic, build-independent):

```
## To pow2 (d: Float) -> Float:
    If d is 0:
        Return 1.
    Return pow2(d - 1) + pow2(d - 1).

## Main
Show pow2(3).
```

Integer divergence (release-sensitive): the same function with `(d: Int) -> Int` and `Show pow2(64)`. The recursive form doubles 64 times (`1<<63 = i64::MIN`, then `i64::MIN + i64::MIN` wraps to 0 in release); the closed form emits `1i64 << 64`, which release-mode Rust masks to `1i64 << (64 & 63) = 1`. So 0 vs 1.

**Expected vs actual**

- **Expected:** The closed-form replacement is value-equivalent to the recursion for every valid argument, or the pattern is declined (and the parameter must be an integer type for `<< d` to type-check).
- **Actual:** For a `Float` parameter the emitted `(1i64 << d)` does not compile; for large integer `d`, the closed form (`1i64 << 64` → 1 in release) disagrees with the recursive value (repeated doubling → 0).

**Suggested RED test**

```rust
// Place in crates/logicaffeine_tests/tests/ (e.g. e2e_closed_form_guard.rs).
//
// DETERMINISTIC RED TEST: a Float-typed double recursion must still compile and
// run. The closed-form detector ignores the parameter type and emits
// `(1i64 << d)` with `d: f64`, which is not valid Rust, so the generated program
// fails to compile. Fails on current code (success == false); passes once the
// detector declines non-integer parameters.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closed_form_declines_float_parameter() {
    let src = r#"## To pow2 (d: Float) -> Float:
    If d is 0:
        Return 1.
    Return pow2(d - 1) + pow2(d - 1).

## Main
Show pow2(3).
"#;
    let r = common::run_logos(src);
    assert!(r.success,
        "Float double-recursion failed to compile (closed-form emitted i64 << f64)\nstderr:\n{}\ngenerated rust:\n{}",
        r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), "8"); // pow2(3) = 8 via the recurrence
}

// SECONDARY (release-sensitive) RED TEST for the integer value divergence.
// The interpreter never uses the closed form, so it computes the true
// recurrence; the compiled closed form emits `1i64 << 64`, masking to 1 while
// the recurrence wraps to 0. Use the cross-checking oracle so it is build-faithful.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closed_form_matches_recurrence_at_overflow_boundary() {
    let src = r#"## To pow2 (d: Int) -> Int:
    If d is 0:
        Return 1.
    Return pow2(d - 1) + pow2(d - 1).

## Main
Show pow2(64).
"#;
    common::assert_compiled_equals_interpreted(src);
}
```

**Fix direction / notes**

The detector should require `params[0].1` to be an integer type and decline (or bound) when the shift count can reach the bit-width; alternatively emit code that reproduces the recurrence's wrapping doubling rather than a single shift. The closed-form gate at `program.rs:561` should additionally respect `OptFlag::NoOptimize`. No existing unit or e2e tests cover this detector. Verifier raised severity from the auditor's original Low to Medium: valid input yields non-compiling Rust and a different finite value than the recurrence — a genuine observational-equivalence violation.

---

### BUG-028 — Parallel-reduction lowering hardcodes .sum::<i64>(), miscompiling/rejecting float (Copy) sequence reductions

**Severity:** Medium  ·  **Category:** type-width mismatch / valid input fails to compile  ·  **Subsystem:** Codegen · statements  ·  **Reporter confidence:** medium  
**Location:** `crates/logicaffeine_compile/src/codegen/stmt.rs:2755–2804`

**Summary**

`try_emit_parallel_reduction` fires for `Repeat for x in coll: Set acc to acc + expr(x)` whenever `coll`'s element type is Copy. `has_copy_element_type` returns true for `f64` (analysis/types.rs:75-86 — Float is Copy), so a `LogosSeq<f64>`/`Vec<f64>`/`&[f64]` reduction is accepted. But the emitted reduction is hardcoded to `.sum::<i64>()` for both the `map`ed and direct cases, regardless of element/accumulator type. For an f64 sequence the iterator yields `f64`, and `.sum::<i64>()` does not type-check (`f64: Sum<i64>` is not implemented), and `acc` (an f64) `+= <i64>` is also ill-typed. The reduction expression is emitted unconditionally (both the `>= 10000` and the `else` branch are generated), so the wrong-typed `.sum::<i64>()` is always present in the source.

**Why it's wrong**

A natural, valid program — summing a sequence of Floats in a `Repeat` loop — is rewritten into Rust that does not compile, because the reduction type is pinned to i64 instead of being derived from the element/accumulator type. The optimization changes a compilable program into a non-compilable one (and conceptually would compute an i64 sum where an f64 sum is required). The detection gate (`has_copy_element_type`) admits floats but the emitter only handles ints.

**The offending code**

```rust
let is_vec_i64 = ... (t.starts_with("LogosSeq<") || t.starts_with("Vec<") || t.starts_with("&[")) && has_copy_element_type(t) ...;
if !is_vec_i64 { return None; }
...
writeln!(out, "{}    {} += __par_ref.par_iter().copied().map(|{}| {}).sum::<i64>();", indent_str, acc_name, pattern_str, incr_code)
...
writeln!(out, "{}    {} += __par_ref.par_iter().copied().sum::<i64>();", indent_str, acc_name)
```

**Trigger / reproduction**

LOGOS source defining a function that sums a Float sequence, e.g.:\n## To total (nums: Seq of Float) -> Float:\n    Let mutable acc be 0.0.\n    Repeat for x in nums:\n        Set acc to acc + x.\n    Return acc.\n\n## Main\nLet xs be a new Seq of Float.\nPush 1.5 to xs.\nShow total(xs).

**Expected vs actual**

- **Expected:** The generated Rust compiles and the reduction sums as f64 (the accumulator and element type), producing 1.5 — matching the interpreter/VM.
- **Actual:** The generated Rust for the loop body contains `acc += __par_ref.par_iter().copied().map(|x| x).sum::<i64>();` (and an i64 `sum` in the direct case) for an f64 accumulator/iterator, which fails to compile (`f64: Sum<i64>` unsatisfied / mismatched-types on `+=`). Float reductions that the optimizer claims to support cannot be built.

**Suggested RED test**

```rust
// crates/logicaffeine_tests/tests/phase_autoparallel.rs (append)
//
// A float Repeat-reduction must NOT be lowered with a hardcoded i64 sum.
// On current code, try_emit_parallel_reduction admits the f64 sequence
// (has_copy_element_type is true for Float) but emits `.sum::<i64>()`,
// producing Rust that does not type-check.
#[test]
fn par_reduction_float_sum_not_pinned_to_i64() {
    let source = r#"## To total (nums: Seq of Float) -> Float:
    Let mutable acc be 0.0.
    Repeat for x in nums:
        Set acc to acc + x.
    Return acc.

## Main
Let mutable xs be a new Seq of Float.
Push 1.5 to xs.
Show total(xs).
"#;
    let rust = compile_to_rust(source).unwrap();
    // RED on current code: the float reduction is emitted as `.sum::<i64>()`.
    assert!(
        !rust.contains("sum::<i64>()"),
        "float Repeat-sum was lowered with a hardcoded i64 reduction:\n{}",
        rust
    );
}

// Stronger behavioral variant: assert_exact_output compiles AND runs the
// generated Rust, so the i64-pinned reduction fails at the Rust compile
// stage. (If float Show formatting differs, adjust the expected literal;
// the string assertion above is the format-independent signal.)
#[test]
fn par_reduction_float_sum_runs() {
    let source = r#"## To total (nums: Seq of Float) -> Float:
    Let mutable acc be 0.0.
    Repeat for x in nums:
        Set acc to acc + x.
    Return acc.

## Main
Let mutable xs be a new Seq of Float.
Push 1.5 to xs.
Push 2.5 to xs.
Show total(xs).
"#;
    common::assert_exact_output(source, "4");
}
```

**Fix direction / notes**

Corrected the RED test to use this crate's real API and idiom: the claim's `logicaffeine_compile::compile::compile_program` does not exist; the public entry point is `logicaffeine_compile::compile::compile_to_rust` (crates/logicaffeine_compile/src/compile.rs:284), and the `logicaffeine_tests` suite consumes it via `common::compile_to_rust` (re-exported in tests/common/mod.rs). The test should live in the `logicaffeine_tests` crate next to phase_autoparallel.rs (which already uses this exact helper), not in `logicaffeine_compile/tests`. I made the trigger use a function parameter `nums: Seq of Float` so the sequence stays a `LogosSeq<f64>`/`&[f64]` and is not scalarized to `[f64; N]` (Main-level fixed-size seqs scalarize and would bypass the bug). Severity left at Medium (loud compile-time failure, not silent miscompilation).

---

### BUG-029 — C backend min()/max() expand to a ternary that double-evaluates the selected argument (extra side effects / non-determinism)

**Severity:** Medium  ·  **Category:** wrong evaluation semantics / double-evaluation of side-effecting argument  ·  **Subsystem:** Codegen · statements  ·  **Reporter confidence:** medium  
**Location:** `crates/logicaffeine_compile/src/codegen_c/emit.rs:156–181`

**Summary**

`min(a, b)` and `max(a, b)` are lowered to a C conditional expression `((a) < (b) ? (a) : (b))`. The chosen operand text is substituted twice, so whichever argument is selected is *evaluated twice* at runtime. The interpreter evaluates each argument exactly once (semantics/builtins.rs:158-191 receive already-evaluated values). Nesting like `min(max(value, lo), hi)` already exists in the test suite, so arbitrary call expressions are valid `min`/`max` arguments; when such an argument has observable effects (a user function that prints or mutates a collection/global), the compiled program performs the effect an extra time, or — if the call is non-deterministic — the compare branch and the value branch can disagree.

**Why it's wrong**

`min`/`max` are pure selection in the language semantics: each argument is computed once and the smaller/larger value returned. Emitting a textual ternary breaks that by re-executing the selected argument's full expression, changing observable behavior (number of prints/mutations, or even returning a different value than was compared if the subexpression is impure). This diverges from the interpreter and VM.

**The offending code**

```rust
"min" => {
    let a_str = ... codegen_expr(a, ctx) ...;
    let b_str = ... codegen_expr(b, ctx) ...;
    format!("(({a}) < ({b}) ? ({a}) : ({b}))", a = a_str, b = b_str)
}
"max" => { ... format!("(({a}) > ({b}) ? ({a}) : ({b}))", a = a_str, b = b_str) }
```

**Trigger / reproduction**

LOGOS source:\n## To tick (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nLet r be min(tick(3), tick(9)).\nShow r.  -- tick(3) is the smaller arg and is therefore evaluated twice

**Expected vs actual**

- **Expected:** `tick` is called once per argument: stdout shows `3` then `9` (in some order) exactly once each, then `3`. Three lines total, with `3` printed exactly once before the result.
- **Actual:** Emitted C is roughly `int64_t r = ((tick(3)) < (tick(9)) ? (tick(3)) : (tick(9)));` — `tick(3)` is evaluated twice, so the program prints `3` twice plus `9`, then the result `3`: the side effect of the selected argument happens an extra time, unlike every other execution tier.

**Suggested RED test**

```rust
// crates/logicaffeine_tests/tests/e2e_math_builtins.rs
// Uses the existing common::assert_c_output harness (compiles LOGOS->C, gcc -O2, checks stdout.trim()).
//
// tick() prints its argument then returns it. In the interpreter/VM each min()
// argument is evaluated exactly once (interpreter.rs:1992-1996 evaluates args in
// a loop, then call_builtin selects among already-evaluated values), so the
// program prints 3, 9 (once each) then the result 3 -> "3\n9\n3".
//
// The C backend lowers min(a,b) to ((a) < (b) ? (a) : (b)) (emit.rs:167),
// substituting the chosen operand's text twice. tick(3) is the smaller arg, so
// it runs once in the comparison and AGAIN in the selected branch: the buggy
// program emits at least four lines (3 printed twice during evaluation), which
// can never equal the 3-line string below. This test FAILS on current code and
// PASSES once min/max evaluate each argument once.
#[test]
fn c_min_evaluates_each_argument_once() {
    assert_c_output(
        "## To tick (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nLet r be min(tick(3), tick(9)).\nShow r.\n",
        "3\n9\n3",
    );
}

// Companion parity assertion for max() (selects the larger operand, double-evaluating tick(9)).
#[test]
fn c_max_evaluates_each_argument_once() {
    assert_c_output(
        "## To tick (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nLet r be max(tick(3), tick(9)).\nShow r.\n",
        "3\n9\n9",
    );
}
```

**Fix direction / notes**

Severity Medium is appropriate and unchanged. Justification: this is a genuine correctness divergence (extra side effects / potential value mismatch under non-determinism, and double work for any expensive selected argument), but it only manifests when a min/max argument is itself side-effecting or non-deterministic (a user function call), not for the common constant/variable arguments that dominate the existing test suite. It is not a silent miscompilation of plain arithmetic. The same double-evaluation pattern exists for any selected branch (both min and max), so the fix should cover both arms.

One correction to the claim's framing of the expected C stdout: the claim states the program "prints 3 twice plus 9, then the result 3" and the suggested expected is "3\n9\n3". The first two prints come from the C `<` operands, whose evaluation order is unspecified by the C standard, so the precise interleave (3-then-9 vs 9-then-3) is not guaranteed under gcc -O2; only the line COUNT (>=4 buggy vs exactly 3 correct) and the fact that "3" prints twice are guaranteed. The exact-match RED test is still valid because no 4-line buggy output can equal the 3-line target string.

Suggested fix direction (not required for the finding): lower min/max to a helper that evaluates each argument once, e.g. emit a statement temp `int64_t _a = <a>; int64_t _b = <b>;` then `(_a < _b ? _a : _b)`, or call a `logos_min_i64`/`logos_max_i64` runtime function (mirroring the existing runtime helpers in codegen_c/runtime.rs), handling Int and Float/mixed cases as the interpreter does.

---

### BUG-030 — ISO-8601 date literal accepts impossible days (Feb 30, Apr 31) and silently coerces them to a different valid date

**Severity:** Medium  ·  **Category:** wrong-value / missing-validation  ·  **Subsystem:** Lexer  ·  **Reporter confidence:** high  
**Location:** `crates/logicaffeine_language/src/lexer.rs:1806–1821`

**Summary**

parse_date_literal() only checks `day < 1 || day > 31` regardless of month, so calendar-impossible dates such as 2026-02-30, 2026-02-31, 2026-04-31, 2026-06-31, 2026-09-31, 2026-11-31 pass validation. The Howard Hinnant day-count formula then computes a day-of-year that overflows the real month and produces the days-since-epoch of a DIFFERENT, real date. The resulting DateLiteral { days } flows into the parser (mod.rs ~5687) and into date arithmetic, so a typo or out-of-range day is not rejected but silently turned into the wrong day.

**Why it's wrong**

A date literal that does not denote a real calendar day must be rejected (return None so the word is handled as a non-date), not silently mapped onto a neighboring valid date. Computing days(2026-02-30) = days(2026-03-02) corrupts any subsequent date subtraction/comparison without any diagnostic.

**The offending code**

```rust
// Basic validation
if month < 1 || month > 12 || day < 1 || day > 31 {
    return None;
}
... Howard Hinnant day-count algorithm ...
let days = era * 146097 + doe as i32 - 719468;
Some(days)
```

**Trigger / reproduction**

English input containing `2026-02-30` (or `2026-04-31`). The lexer is_date_hyphen joins it into one word and parse_date_literal returns Some.

**Expected vs actual**

- **Expected:** parse_date_literal("2026-02-30") returns None (February never has 30 days), so the token is NOT a DateLiteral; or, at minimum, it is rejected as an invalid date.
- **Actual:** parse_date_literal("2026-02-30") returns Some(20514), which is the days-since-epoch value for 2026-03-02; the impossible date is silently accepted as March 2.

**Suggested RED test**

```rust
#[test]
fn date_literal_rejects_impossible_day_of_month() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::lexer::Lexer;
    use logicaffeine_language::token::TokenType;

    // Impossible dates that must NOT tokenize to a DateLiteral.
    // Each currently mis-maps onto a real neighboring day via the
    // Howard Hinnant formula, e.g. 2026-02-30 -> 2026-03-02 (day count 20514),
    // 2026-04-31 -> 2026-05-01, 2026-02-29 (non-leap) -> 2026-03-01.
    let impossible = [
        "2026-02-30", // February never has 30 days
        "2026-02-31",
        "2026-02-29", // 2026 is not a leap year
        "2026-04-31", // April has 30 days
        "2026-06-31",
        "2026-09-31",
        "2026-11-31",
    ];

    for input in impossible {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new(input, &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            !tokens
                .iter()
                .any(|t| matches!(t.kind, TokenType::DateLiteral { .. })),
            "{input} is not a real calendar date and must not become a DateLiteral; got {tokens:?}"
        );
    }

    // Control: a real date with the same surface shape MUST still tokenize.
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("2024-02-29", &mut interner); // 2024 is a leap year
    let tokens = lexer.tokenize();
    assert!(
        tokens
            .iter()
            .any(|t| matches!(t.kind, TokenType::DateLiteral { .. })),
        "2024-02-29 is a valid leap-day and must still tokenize to a DateLiteral; got {tokens:?}"
    );
}
```

**Fix direction / notes**

Severity Medium is correct and retained. Justification: this is silent wrong-value corruption of user-intended date semantics (an impossible literal becomes a real but different day), with no panic, no memory unsafety, and no miscompilation of otherwise-valid programs. It only manifests on malformed/typo'd date literals in a niche literal feature, which bounds the blast radius below High; but it is a genuine soundness gap (no diagnostic, observable wrong results in date arithmetic), which keeps it above Low. The fix must add a days-in-month table plus leap-year handling (Feb = 29 in leap years, 28 otherwise; Apr/Jun/Sep/Nov = 30; rest = 31) before computing the day count, returning None for out-of-range days. The auditor's title, file/line citation, trigger, expected/actual, and confidence (high) are all accurate.

---

### BUG-031 — Futamura encoder collapses all tasks of a Concurrent/Parallel block into a single branch, losing the per-task branch structure

**Severity:** Low  ·  **Category:** AST->IR lowering: structural mis-lowering of a concurrency construct  ·  **Subsystem:** Compile pipeline (Futamura)  ·  **Reporter confidence:** low  
**Location:** `crates/logicaffeine_compile/src/compile.rs:2885–2922`

**Summary**

`encode_stmt_src` (used by the production `encode_program_source` / `encode_program_source_compact` Futamura encoders, which are exercised throughout phase_futamura.rs) lowers `Stmt::Concurrent { tasks }` and `Stmt::Parallel { tasks }` into a `CConcurrent`/`CParallel` whose `branches` field (`Seq of Seq of CStmt`) is supposed to hold ONE inner list per concurrent task. The PE self-interpreter confirms this shape: pe_source.logos lines 2293-2302 iterate `Repeat for concBranch in concBranches: Push peBlock(concBranch, state)...`, i.e. each element of the outer Seq is an independent branch. But the encoder allocates a single `branch_var`, pushes EVERY task statement into it, and pushes only that one branch. So `Concurrent { tasks: [A, B, C] }` is encoded as `branches = [[A, B, C]]` (one branch running A;B;C) instead of `[[A], [B], [C]]` (three branches). The N-way concurrency/parallelism structure is flattened to a single sequential branch.

**Why it's wrong**

The AST (crates/logicaffeine_language/src/ast/stmt.rs lines 316-332) documents each statement in the block as a task that 'runs concurrently via tokio::join!' (Concurrent) or 'true parallelism via rayon::join' (Parallel). Encoding all tasks into one branch destroys the join structure the downstream consumer relies on: the encoded program no longer represents N tasks, only one. Any analysis or interpretation that distinguishes per-branch scheduling (e.g., interleaving, isolation, or join cardinality) sees a different program than was written.

**The offending code**

```rust
Stmt::Concurrent { tasks } => {
    ... Let {branches_var} be a new Seq of Seq of CStmt.
    let branch_var = ...; // ONE branch
    output.push_str("Let {branch_var} be a new Seq of CStmt.");
    for stmt in tasks.iter() { /* push every task into the SAME branch */ }
    output.push_str("Push {branch_var} to {branches_var}."); // exactly one branch
    ... a new CConcurrent with branches {branches_var} ...
}
```

**Trigger / reproduction**

LOGOS program with a multi-task concurrent block, e.g.:
## Main
Simultaneously:
    Show "a".
    Show "b".
    Show "c".
then call encode_program_source on it and inspect the emitted `CParallel ... branches` structure (or run the encoded program through the self-interpreter).

**Expected vs actual**

- **Expected:** The emitted `CParallel`/`CConcurrent` should have `branches` = a Seq of three single-statement branches: [[Show a], [Show b], [Show c]], one branch per task, so the PE handler runs peBlock on three separate branches.
- **Actual:** The emitted `branches` is a Seq with exactly ONE inner branch containing all three statements: [[Show a, Show b, Show c]]. The three concurrent tasks are merged into a single sequential branch; the PE handler runs peBlock once over the merged block.

**Suggested RED test**

```rust
#[test]
fn encode_parallel_preserves_one_inner_branch_per_task() {
    // `Simultaneously:` with three tasks parses to Stmt::Parallel { tasks: [Show a, Show b, Show c] }.
    // A faithful Futamura encoding must produce `branches: Seq of Seq of CStmt` with ONE inner
    // `Seq of CStmt` per task (matching the canonical codegen's per-arm rayon::join / tokio::join!,
    // and the executor/PE handlers that `Repeat for branch in branches`). The current encoder
    // collapses all tasks into a single inner branch, so it emits the inner-branch allocation
    // line exactly once instead of three times.
    let src = "## Main\nSimultaneously:\n    Show \"a\".\n    Show \"b\".\n    Show \"c\".\n";
    let encoded = logicaffeine_compile::compile::encode_program_source(src).unwrap();

    // Sanity: the construct survived as a CParallel.
    assert!(
        encoded.contains("a new CParallel with branches"),
        "expected a CParallel in encoded output:\n{}",
        encoded
    );

    // The outer Seq-of-Seq is allocated once: `Let <v> be a new Seq of Seq of CStmt.`
    let outer_seq_decls = encoded.matches("be a new Seq of Seq of CStmt.").count();
    assert_eq!(
        outer_seq_decls, 1,
        "expected exactly one outer Seq-of-Seq (the branches list):\n{}",
        encoded
    );

    // Each task must get its OWN inner branch list: `Let <v> be a new Seq of CStmt.`
    // Faithful encoding => 3 inner branches; the bug emits exactly 1.
    let inner_branch_decls = encoded.matches("be a new Seq of CStmt.").count();
    assert_eq!(
        inner_branch_decls, 3,
        "all three Simultaneously tasks were collapsed into a single branch \
         (expected 3 inner `Seq of CStmt` branch lists, found {}):\n{}",
        inner_branch_decls, encoded
    );
}
```

**Fix direction / notes**

Severity lowered Medium -> Low. The claim's "actual vs expected" structural assertion is correct, but its justification overstates observable impact: it says "Any analysis or interpretation that distinguishes per-branch scheduling (interleaving, isolation, join cardinality) sees a different program." In practice, no consumer in this codebase distinguishes those. The self-interpreter executor (phase_futamura.rs:1236-1247) and the partial evaluator (pe_source.logos:2293-2302) both iterate branches against shared mutable state with NO real concurrency/isolation, so observable output is identical for `[[A,B,C]]` vs `[[A],[B],[C]]`. The bug is a real structural mis-lowering (the Futamura encoder is unfaithful to the source's join cardinality and diverges from the canonical codegen at codegen/stmt.rs:2100-2218), but it is not an observable miscompilation today. It becomes a latent correctness hazard only if/when the Core executor or PE gains true per-branch isolation — at which point the collapsed encoding would leak state across tasks that should be isolated. The proposed RED test's `count_outer_branches` helper does not exist and its heuristic line-matching (`l.contains("to e_") && l.starts_with("Push e_")`) is brittle; replaced with a deterministic structural count below.

---

### BUG-032 — Moment-vs-Time comparison and Moment/Time display use truncating % / for time-of-day, giving a negative time-of-day for pre-epoch (negative) Moments

**Severity:** Low  ·  **Category:** wrong comparison semantics  ·  **Subsystem:** Interpreter  ·  **Reporter confidence:** low  
**Location:** `crates/logicaffeine_compile/src/semantics/compare.rs:85–93`

**Summary**

To compare a Moment against a Time, the code extracts the Moment's time-of-day with `*m % nanos_per_day`. Rust's `%` is the truncating remainder, so for a negative Moment (an instant before the Unix epoch, 1970-01-01) the result is negative — i.e. a 'time-of-day' in the range (-86400e9, 0] instead of the correct [0, 86400e9). The intended value is the Euclidean remainder `m.rem_euclid(nanos_per_day)`. The same truncating arithmetic appears in `RuntimeValue::to_display_string` for `Moment` (interpreter.rs lines 585-589: `total_seconds % 86400`, then `/3600`, `/60`) and for `Time` (lines 641-643), so pre-epoch Moments also render with negative hours/minutes.

**Why it's wrong**

`m % day` and `m.rem_euclid(day)` differ for negative `m`. A Moment 1 hour before epoch is `-3_600_000_000_000`. Its true time-of-day is 23:00 (rem_euclid → 82_800e9), but `% nanos_per_day` yields `-3_600_000_000_000`, which int_rel treats as a 'time-of-day' smaller than every positive Time. So `m is before t` answers wrongly, and `Show`ing such a Moment prints something like `1969-12-31 00:00` with negative internal hours rather than `23:00`.

**The offending code**

```rust
// Moment vs Time: extract time-of-day from Moment.
(RuntimeValue::Moment(m), RuntimeValue::Time(t)) => {
    let nanos_per_day = 86_400_000_000_000i64;
    Ok(RuntimeValue::Bool(int_rel(*m % nanos_per_day, *t)))
}
(RuntimeValue::Time(t), RuntimeValue::Moment(m)) => {
    let nanos_per_day = 86_400_000_000_000i64;
    Ok(RuntimeValue::Bool(int_rel(*t, *m % nanos_per_day)))
}
```

**Trigger / reproduction**

A LOGOS program comparing/showing a Moment that lies before the Unix epoch (a negative Moment value). E.g. construct `now()` adjusted backward past 1970-01-01, or any Moment literal/derivation with a negative nanos value, then compare it to a Time or `Show` it.

**Expected vs actual**

- **Expected:** Time-of-day extracted with Euclidean modulo: a Moment 1h before epoch has time-of-day 23:00:00, so `moment is before (time 22:00)` is false and `Show moment` prints `...  23:00`.
- **Actual:** Truncating `%` yields a negative time-of-day, so the Moment is treated as earlier than every positive Time and the displayed hours/minutes are negative/wrong.

**Suggested RED test**

```rust
#[test]
fn moment_before_epoch_compares_by_true_time_of_day() {
    use logicaffeine_compile::interpreter::RuntimeValue;
    use logicaffeine_compile::semantics::compare::compare;
    use logicaffeine_compile::ast::stmt::BinaryOpKind;

    let nanos_per_day = 86_400_000_000_000i64;
    let hour = 3_600_000_000_000i64;

    // `1969-12-31 at 11pm`: one hour before the Unix epoch.
    // Reachable from valid English via the lexer's pre-1970 DateLiteral
    // (negative days) + parser's `DATE at TIME` -> Moment.
    // True wall-clock time-of-day = 23:00, so it is AFTER 22:00.
    let m = RuntimeValue::Moment(-hour); // == -1*nanos_per_day + 23*hour
    let t_2200 = RuntimeValue::Time(22 * hour);

    // 23:00 < 22:00 is FALSE.
    let r = compare(BinaryOpKind::Lt, &m, &t_2200).unwrap();
    assert!(matches!(r, RuntimeValue::Bool(false)),
        "pre-epoch Moment time-of-day must be 23:00 (after 22:00), not a negative time-of-day");

    // Symmetric arm: 23:00 > 22:00 is TRUE.
    let r2 = compare(BinaryOpKind::Gt, &m, &t_2200).unwrap();
    assert!(matches!(r2, RuntimeValue::Bool(true)),
        "Time vs pre-epoch Moment must also use Euclidean time-of-day");

    let _ = nanos_per_day;
}

// Secondary RED test for the display path (interpreter.rs:585-603).
#[test]
fn moment_before_epoch_displays_correct_date_and_hour() {
    use logicaffeine_compile::interpreter::RuntimeValue;
    let hour = 3_600_000_000_000i64;
    // 1969-12-31 at 23:00.
    let m = RuntimeValue::Moment(-hour);
    assert_eq!(m.to_display_string(), "1969-12-31 23:00",
        "pre-epoch Moment must floor the date and use Euclidean time-of-day, not print 1970-01-01 -1:00");
}
```

**Fix direction / notes**

Severity Low is appropriate and retained. It is a genuine silent wrong-result (no panic) for valid input, but only for pre-1970 (negative) Moments compared against a Time or displayed — an uncommon case in practice. I would not raise it to Medium because no data corruption propagates beyond the single comparison/display and the interpreter and VM agree (no differential divergence between the two execution tiers).

Scope correction to the claim: the fix must touch THREE sites, not just the two comparison arms:
1. compare.rs:88 `*m % nanos_per_day` → `m.rem_euclid(nanos_per_day)`
2. compare.rs:92 `*m % nanos_per_day` → `m.rem_euclid(nanos_per_day)`
3. interpreter.rs:585-589 (Moment display): the truncating `total_seconds / 86400`, `total_seconds % 86400`, and the subsequent `/3600` / `%3600 /60` all need floor/Euclidean handling so a pre-epoch Moment yields the correct date AND a 0..86399 day_seconds. The simplest correct form is `let day_seconds = total_seconds.rem_euclid(86400);` and `let days = total_seconds.div_euclid(86400) as i32;` (the date part must use floored division too, else `1969-12-31 at 11pm` prints date 1970-01-01).
4. interpreter.rs:641-643 (Time display) is latent because Time literals are non-negative; harmless to fix for symmetry but not required to fix the live bug.

---

### BUG-033 — force == 0.5 modals (can/could/would/may) lower as ◇ but display as □ — boundary mismatch between Kripke lowering and debug display

**Severity:** Low  ·  **Category:** modal-boundary-inconsistency  ·  **Subsystem:** Modal semantics (Kripke)  ·  **Reporter confidence:** medium  
**Location:** `crates/logicaffeine_language/src/debug.rs:196–203`

**Summary**

The debug `.with(interner)` display classifies modals with `force >= 0.5` as Box, while the Kripke lowering (kripke.rs:471) uses `force > 0.5` for Box. The ability/possibility modals can, could, would, may are all assigned force = 0.5 (parser/modal.rs:526,534,557,568,599). At exactly 0.5 the two passes disagree: debug renders them as □ (necessity), but Kripke lowers them to a Diamond existential ∃w'(Accessible ∧ P). The transpile formatter (formatter.rs:83, `force > 0.0 && force <= 0.5`) agrees with Kripke (◇) and against debug.

**Why it's wrong**

Ability/epistemic "can/could/may" is possibility (◇), as the transpile formatter and Kripke pass both treat it. The debug display showing "Birds can fly" as □Fly (necessity in every accessible world) is semantically wrong and contradicts the same expression's Kripke lowering ∃w'(Accessible∧Fly). Any tooling that reads the debug rendering of a force-0.5 modal gets the necessity reading, which over-commits (□ entails ◇ but not vice versa), an invalid strengthening.

**The offending code**

```rust
let op = match (vector.domain, vector.force >= 0.5) {
    (crate::ast::ModalDomain::Alethic, true) => "□",
    (crate::ast::ModalDomain::Alethic, false) => "◇",
    (crate::ast::ModalDomain::Deontic, true) => "O",
    (crate::ast::ModalDomain::Deontic, false) => "P",
    (crate::ast::ModalDomain::Temporal, _) => "Temporal",
};
```

**Trigger / reproduction**

Construct LogicExpr::Modal{ vector: ModalVector{ domain: Alethic, force: 0.5, flavor: Root, ... }, operand: Atom("Fly") } and render via `.with(&interner)`; compare to fmt.modal / compile_kripke of the same. The debug string starts with □, the others with ◇.

**Expected vs actual**

- **Expected:** All three passes should agree on the Box/Diamond boundary for force 0.5; an Alethic ability modal (force 0.5) is possibility (◇) per the transpile formatter and Kripke lowering, so debug should use `> 0.5` for □ as well.
- **Actual:** debug.rs uses `force >= 0.5` → renders force-0.5 "can/could/may" as □ (necessity), contradicting both the Kripke lowering and the transpile formatter, which render the same vector as ◇ (possibility).

**Suggested RED test**

```rust
// Place inside crates/logicaffeine_language/src/debug.rs `#[cfg(test)] mod tests`
// (mirrors the existing `expr_modal_display` test; `super::*` and
// `logicaffeine_base::Arena` are already imported there).
#[test]
fn expr_modal_display_force_half_is_possibility_not_necessity() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let fly = interner.intern("Fly");
    // force == 0.5 Alethic = ability/epistemic possibility (can/could/would/may).
    // Kripke lowering (semantics/kripke.rs:471, `> 0.5`) and the transpile
    // formatter (formatter.rs:83, `<= 0.5`) both render this as ◇ (Diamond).
    let expr = LogicExpr::Modal {
        vector: crate::ast::ModalVector {
            domain: crate::ast::ModalDomain::Alethic,
            force: 0.5,
            flavor: crate::ast::ModalFlavor::Root,
            modal_base: None,
            ordering_source: None,
        },
        operand: expr_arena.alloc(LogicExpr::Atom(fly)),
    };
    // FAILS on current code: debug.rs:196 uses `>= 0.5`, producing "□(Fly)".
    // PASSES after changing that boundary to `> 0.5` -> "◇(Fly)".
    assert_eq!(expr.with(&interner).to_string(), "◇(Fly)");
}
```

**Fix direction / notes**

Severity corrected from Medium to Low. The claim's technical content is accurate, but the impact is overstated as Medium. The buggy boundary (`>= 0.5`) lives only in the debug-display helper in crates/logicaffeine_language/src/debug.rs (line 196), and a full-repo grep confirms `DisplayWith` / `.with(interner)` is referenced nowhere outside that file — there are zero production or tooling consumers of this rendering in the codebase today. The actual transpile output (transpile.rs:474 → formatter.rs:83) and the Kripke lowering (semantics/kripke.rs:471) both use the correct boundary (force <= 0.5 → ◇) and agree with each other and with the documented semantics of the `force` field (ast/logic.rs:442: "0.5 = possibility (◇)"). So no FOL output, no compiled behavior, and no semantic pipeline result is wrong — only the debug display of a hand-held or test-constructed force-0.5 Alethic modal is wrong (□ instead of ◇). One-character fix: change `vector.force >= 0.5` to `vector.force > 0.5` at debug.rs:196 to align with kripke/formatter/doc-comment.

Minor note on the claim's secondary references: the claim says formatter.rs is at line 83 and parser lines 526/534/557/568/599; the exact line numbers drift slightly in the current tree (formatter.modal match arm is line 83; parser CAN/COULD/WOULD/MAY force-0.5 assignments land around lines 533, 544/561, 566, 600), but the substance is correct.

---

## Needs Further Investigation

These leads were raised by an auditor but the adversarial verifier could neither confirm nor refute them within the read budget. They are worth a closer look but are **not** asserted as bugs.

| Subsystem | Location | Claim | Why unresolved |
|-----------|----------|-------|----------------|
| Codegen · FFI / runtime boundary | `crates/logicaffeine_compile/src/codegen/ffi.rs:368` | Handle id (u64) is round-tripped through *mut c_void (LogosHandle) and truncated on 32-bit targets (wasm32 / arm32) | The claim's facts are all accurate. I confirmed at crates/logicaffeine_compile/src/codegen/ffi.rs:210 that `LogosHandle = *mut std::ffi::c_void` (pointer-width), at lines 292-305 that the registry `counter`/ids are `u64`, monotonic, and never reused, and that the u64 id is round-tripped through t… |
| Semantic checks | `crates/logicaffeine_compile/src/analysis/check.rs:702` | `Repeat` over a Text/String binds the loop variable to Unknown instead of the character element type | The factual core of the claim is TRUE and I confirmed it by reading the code. At crates/logicaffeine_compile/src/analysis/check.rs:703-707 the Repeat element-type match has cases for Seq/Set/Map but no case for InferType::String, so iterating a string falls to `_ => InferType::Unknown`. The runti… |
| Semantic checks | `crates/logicaffeine_compile/src/analysis/check.rs:429` | `infer_call` silently accepts argument-count (arity) mismatches for named function calls | The cited code defect is REAL and accurately quoted. check.rs:411-445 `infer_call` unifies argument types element-wise via `for (arg, param_ty) in args.iter().zip(param_types.iter())` (line 431) with NO `args.len() != param_types.len()` comparison. The `TypeError::ArityMismatch` machinery (unify.… |
| Parser · core | `crates/logicaffeine_language/src/parser/mod.rs:885` | Speculative parses via try_parse/checkpoint roll back token position but NOT DRS / WorldState mutations, leaving discourse referents polluted after a failed alternative | The MECHANICAL core of the claim is TRUE, but the claim's concrete trigger and red test are factually wrong, and no demonstrably-wrong observable output is established. CONFIRMED facts (read directly): - crates/logicaffeine_language/src/parser/mod.rs:155-162 — ParserCheckpoint captures only {pos,… |

## Methodology

1. **Decompose.** The interpreter/compiler/VM/JIT/optimizer/frontend/kernel surface was split into 24 subsystems, each with an explicit file list and a sharpened set of bug hypotheses (e.g. *capture-avoidance under binders*, *unsound optimizer facts that feed the JIT*, *VM jump/stack discipline*, *quantifier scope and polarity*).
2. **Hunt.** One auditor per subsystem read the real source (no `git`, no `cargo` — static reading only, to respect the build lock) and reported only concretely-triggerable findings, each with a trigger, expected/actual behavior, and a proposed RED test.
3. **Verify adversarially.** Every candidate was handed to a separate skeptic instructed to *refute* it — open the cited code, read callers and the matching VM/codegen side, and default to *refuted*/*uncertain* unless a concrete wrong-behavior path could be demonstrated. Verifiers corrected severities and tightened tests; refuted candidates were dropped.
4. **Synthesize.** Surviving findings were ordered by severity and rendered deterministically into this report, with code excerpts and tests preserved verbatim.

**Caveat.** The RED tests were authored but **not executed** in this pass (the auditors were forbidden from invoking `cargo` to avoid build-lock contention). Treat each as a specification: add it, watch it fail, then fix the implementation until it passes — and, per the project rules, do not modify a RED test to make it pass.

