//! J1/J2 capstone differential: LOGOS source → VM bytecode → MicroOp adapter →
//! native stencil chain, checked against the bytecode VM executing the SAME
//! program. Three layers must agree: what the user wrote, what the VM runs,
//! and what the JIT compiled.
//!
//! The adapter accepts the integer subset (now WITH control flow: J2) and
//! BAILS (None) on anything else — exactly the tier-up contract: the VM keeps
//! every program it cannot yet compile. It also bails when a comparison
//! result could become OBSERVABLE (the VM shows `true`; the JIT computes 1) —
//! comparisons may only feed branches and other comparisons.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::Compiler;
use logicaffeine_forge::jit::compile_straightline;
use logicaffeine_jit::{adapt, ForgeTier};

/// Compile `source`'s Main to VM bytecode, adapt, JIT, and return the result —
/// or None when outside the J2 subset.
fn jit_run(source: &str) -> Option<i64> {
    with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, types, _policies)) => {
            let program = Compiler::compile_with_types(stmts, interner, Some(types)).ok()?;
            let (micro, frame_size) =
                adapt(&program.code, &program.constants, program.register_count)?;
            let chain = compile_straightline(&micro).ok()?;
            let mut frame = vec![0i64; frame_size.max(1)];
            Some(chain.run_with_frame(&mut frame).expect_return())
        }
        Err(_) => None,
    })
}

fn assert_jit_matches_vm(source: &str) {
    let jit = jit_run(source).expect("program should be in the JIT subset");
    let vm = vm_outcome(source);
    assert_eq!(vm.error, None, "VM errored on:\n{source}");
    assert_eq!(
        jit.to_string(),
        vm.output.trim(),
        "JIT diverged from VM for:\n{source}"
    );
}

#[test]
fn jit_matches_vm_on_straightline_programs() {
    assert_jit_matches_vm("## Main\nLet a be 3.\nLet b be 5.\nShow a + b.\n");
    assert_jit_matches_vm("## Main\nLet a be 7.\nLet b be a * a - 9.\nShow b * 2 + a.\n");
    assert_jit_matches_vm(
        "## Main\nLet mutable x be 1.\nSet x to x + 10.\nSet x to x * x.\nShow x - 21.\n",
    );
    // Wrapping at the boundary, end to end from source.
    assert_jit_matches_vm("## Main\nLet a be 9223372036854775807.\nShow a + 1.\n");
}

#[test]
fn jit_factorial_from_logos_source() {
    // THE J2 capstone: a real LOGOS while-loop, through the real VM compiler,
    // running as native code.
    for n in [0i64, 1, 5, 10, 20] {
        let src = format!(
            "## Main\n\
             Let mutable n be {n}.\n\
             Let mutable acc be 1.\n\
             While n is greater than 1:\n\
             \x20   Set acc to acc * n.\n\
             \x20   Set n to n - 1.\n\
             Show acc.\n"
        );
        assert_jit_matches_vm(&src);
    }
}

#[test]
fn jit_loops_and_ifs_from_source() {
    assert_jit_matches_vm(
        "## Main\n\
         Let mutable i be 0.\n\
         Let mutable acc be 0.\n\
         While i is less than 100:\n\
         \x20   Set acc to acc + i * i.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n",
    );
    assert_jit_matches_vm(
        "## Main\n\
         Let x be 7.\n\
         Let mutable r be 0.\n\
         If x is greater than 3:\n\
         \x20   Set r to 111.\n\
         Otherwise:\n\
         \x20   Set r to 222.\n\
         Show r.\n",
    );
    // <= and >= exercise the scratch-slot lowering.
    assert_jit_matches_vm(
        "## Main\n\
         Let mutable i be 0.\n\
         Let mutable acc be 0.\n\
         While i is at most 10:\n\
         \x20   Set acc to acc + i.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n",
    );
}

#[test]
fn jit_bails_outside_the_subset() {
    // Text and bool observation are bails.
    assert_eq!(jit_run("## Main\nShow \"hi\".\n"), None);
    // Showing a comparison result must bail (VM prints `true`, not 1).
    assert_eq!(jit_run("## Main\nLet a be 1.\nLet b be 2.\nShow a is less than b.\n"), None);
}

#[test]
fn jit_division_compiles_with_checked_semantics() {
    // M2 (EXODIA Forge deopt protocol) moved Div/Mod INTO the subset: the
    // checked stencil side-exits on a zero divisor instead of bailing the
    // whole program — so division now compiles and must match the VM.
    assert_jit_matches_vm("## Main\nShow 7 / 2.\n");
    assert_jit_matches_vm("## Main\nShow 7 % 2.\n");
    assert_jit_matches_vm("## Main\nLet a be -9223372036854775808.\nShow a / -1.\n");
}

struct SplitMix64 {
    state: u64,
}
impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15) }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

#[test]
fn jit_seeded_source_programs_with_loops_match_vm() {
    let mut translated = 0u32;
    for seed in 0..300u64 {
        let mut rng = SplitMix64::new(seed);
        let mut src = String::from("## Main\n");
        let var_count = 2 + rng.below(3);
        for k in 0..var_count {
            src.push_str(&format!("Let mutable v{k} be {}.\n", rng.below(50)));
        }
        // straight-line prelude
        for _ in 0..rng.below(4) {
            let d = rng.below(var_count);
            let a = rng.below(var_count);
            let b = rng.below(var_count);
            let op = ["+", "-", "*"][rng.below(3) as usize];
            src.push_str(&format!("Set v{d} to v{a} {op} v{b}.\n"));
        }
        // a bounded counted loop
        let trip = rng.below(8);
        src.push_str(&format!("Let mutable lc be {trip}.\n"));
        src.push_str("While lc is greater than 0:\n");
        let body = 1 + rng.below(3);
        for _ in 0..body {
            let d = rng.below(var_count);
            let a = rng.below(var_count);
            let op = ["+", "-", "*"][rng.below(3) as usize];
            src.push_str(&format!("    Set v{d} to v{d} {op} v{a}.\n"));
        }
        src.push_str("    Set lc to lc - 1.\n");
        let shown = rng.below(var_count);
        src.push_str(&format!("Show v{shown}.\n"));

        if let Some(jit) = jit_run(&src) {
            translated += 1;
            let vm = vm_outcome(&src);
            assert_eq!(vm.error, None, "seed {seed}: VM errored on:\n{src}");
            assert_eq!(
                jit.to_string(),
                vm.output.trim(),
                "seed {seed}: JIT diverged from VM for:\n{src}"
            );
        }
    }
    assert_eq!(translated, 300, "every generated program should translate");
}

// ===========================================================================
// J3: tier-up + deopt — the forge backend behind the VM's NativeTier seam.
// ===========================================================================

use logicaffeine_compile::vm::{Vm, NATIVE_TIER_THRESHOLD, REGION_TIER_THRESHOLD};

/// Run `source` twice — tiered and pure-bytecode — and return both outcomes
/// plus (compile attempts, compile successes).
fn tiered_vs_pure(source: &str) -> ((String, Option<String>), (String, Option<String>), (u32, u32)) {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compile");

        let tier = ForgeTier::new();
        let mut vm = Vm::new(&program).with_native_tier(&tier);
        let tiered = match vm.run() {
            Ok(()) => (vm.output(), None),
            Err(e) => (vm.output(), Some(e)),
        };
        let compiles = tier.function_counts();

        let mut vm2 = Vm::new(&program);
        let pure = match vm2.run() {
            Ok(()) => (vm2.output(), None),
            Err(e) => (vm2.output(), Some(e)),
        };
        (tiered, pure, compiles)
    })
}

/// Like tiered_vs_pure but reporting the REGION counters (Main-loop tiering).
fn tiered_vs_pure_regions(
    source: &str,
) -> ((String, Option<String>), (String, Option<String>), (u32, u32)) {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compile");

        let tier = ForgeTier::new();
        let mut vm = Vm::new(&program).with_native_tier(&tier);
        let tiered = match vm.run() {
            Ok(()) => (vm.output(), None),
            Err(e) => (vm.output(), Some(e)),
        };
        let compiles = tier.region_counts();

        let mut vm2 = Vm::new(&program);
        let pure = match vm2.run() {
            Ok(()) => (vm2.output(), None),
            Err(e) => (vm2.output(), Some(e)),
        };
        (tiered, pure, compiles)
    })
}

#[test]
fn tier_up_hot_function_matches_bytecode_and_compiles_once() {
    // sq(i) is called 500× (hot); the tiered run must equal the pure run and
    // compile EXACTLY once.
    let src = "\
## To sq (n: Int) -> Int:
    Return n * n.

## Main
Let mutable i be 0.
Let mutable acc be 0.
While i is less than 500:
    Set acc to acc + sq(i).
    Set i to i + 1.
Show acc.
";
    let (tiered, pure, compiles) = tiered_vs_pure(src);
    assert_eq!(tiered, pure, "tiered run diverged from bytecode");
    assert_eq!(compiles, (1, 1), "the hot function should compile exactly once");
    assert_eq!(tiered.1, None);
}

#[test]
fn tier_up_loopy_function_with_branches() {
    // A function with its own loop + early return tiers up correctly.
    let src = "\
## To fact (n: Int) -> Int:
    Let mutable acc be 1.
    Let mutable k be n.
    While k is greater than 1:
        Set acc to acc * k.
        Set k to k - 1.
    Return acc.

## Main
Let mutable i be 0.
Let mutable sum be 0.
While i is less than 300:
    Set sum to sum + fact(10).
    Set i to i + 1.
Show sum.
";
    let (tiered, pure, compiles) = tiered_vs_pure(src);
    assert_eq!(tiered, pure);
    assert_eq!(compiles, (1, 1));
}

#[test]
fn cold_function_never_compiles() {
    let src = "\
## To sq (n: Int) -> Int:
    Return n * n.

## Main
Show sq(7).
";
    let (tiered, pure, compiles) = tiered_vs_pure(src);
    assert_eq!(tiered, pure);
    assert_eq!(compiles, (0, 0), "one call is far below the threshold");
}

#[test]
fn deopt_on_float_argument_after_tier_up() {
    // dbl() goes hot on Ints, then is called with a Float — the guard must
    // route that call back to bytecode, and the outputs must match exactly.
    let src = "\
## To dbl (n: Int) -> Int:
    Return n + n.

## Main
Let mutable i be 0.
Let mutable acc be 0.
While i is less than 200:
    Set acc to acc + dbl(i).
    Set i to i + 1.
Show acc.
Show dbl(2.5).
";
    let (tiered, pure, compiles) = tiered_vs_pure(src);
    assert_eq!(tiered, pure, "deopt path diverged");
    assert_eq!(compiles, (1, 1));
}

#[test]
fn unsupported_function_fails_closed_and_stays_on_bytecode() {
    // Text concat is outside the subset: compilation bails ONCE, the function
    // keeps running on bytecode forever, results identical.
    let src = "\
## To tag (n: Int) -> Text:
    Return \"x{n}\".

## Main
Let mutable i be 0.
Let mutable out be \"\".
While i is less than 150:
    Set out to tag(i).
    Set i to i + 1.
Show out.
";
    let (tiered, pure, compiles) = tiered_vs_pure(src);
    assert_eq!(tiered, pure);
    assert_eq!(compiles, (1, 0), "one failed attempt, then Failed is sticky");
}

#[test]
fn tiered_seeded_programs_match_pure_bytecode() {
    let _ = NATIVE_TIER_THRESHOLD; // the generator drives WELL past it
    for seed in 0..120u64 {
        let mut rng = SplitMix64::new(seed);
        let body_op = ["+", "-", "*"][rng.below(3) as usize];
        let calls = 150 + rng.below(300);
        let a0 = rng.below(40);
        let src = format!(
            "## To f (n: Int) -> Int:\n\
             \x20   Let r be n {body_op} {}.\n\
             \x20   Return r * {}.\n\
             \n\
             ## Main\n\
             Let mutable i be 0.\n\
             Let mutable acc be {a0}.\n\
             While i is less than {calls}:\n\
             \x20   Set acc to acc + f(i).\n\
             \x20   Set i to i + 1.\n\
             Show acc.\n",
            rng.below(20),
            1 + rng.below(5),
        );
        let (tiered, pure, _) = tiered_vs_pure(&src);
        assert_eq!(tiered, pure, "seed {seed} diverged for:\n{src}");
    }
}

#[test]
fn bool_flowing_into_arithmetic_must_not_diverge() {
    // The VM errors ("cannot add Bool"); native code would happily compute
    // 1 + 1. The adapter must refuse to compile this function.
    // The bool path is COLD while the function warms up on the int path, so
    // the function tiers up before the VM ever hits the runtime type error —
    // the adapter must refuse statically or the tiers diverge.
    let src = "\
## To trap (n: Int) -> Int:
    If n is less than 120:
        Return n + 1.
    Let b be n is less than 999.
    Return b + 1.

## Main
Let mutable i be 0.
Let mutable acc be 0.
While i is less than 150:
    Set acc to acc + trap(i).
    Set i to i + 1.
Show acc.
";
    let (tiered, pure, _) = tiered_vs_pure(src);
    assert_eq!(tiered, pure, "bool-into-arith diverged between tiers");
}

#[test]
fn bool_returning_function_tiers_up_via_kind_inference() {
    // J4: the adapter's kind dataflow proves the return is ALWAYS a bool, so
    // the function compiles and the result re-boxes as Bool ("true"/"false").
    let src = "\
## To isBig (n: Int) -> Bool:
    Return n is greater than 100.

## Main
Let mutable i be 0.
Let mutable hits be 0.
While i is less than 200:
    If isBig(i):
        Set hits to hits + 1.
    Set i to i + 1.
Show hits.
Show isBig(500).
";
    let (tiered, pure, compiles) = tiered_vs_pure(src);
    assert_eq!(tiered, pure, "bool-returning function diverged");
    assert_eq!(compiles, (1, 1), "isBig should COMPILE under kind inference");
}

// ===========================================================================
// Sprint 14 (first slice): the proof-of-speed benchmark. All three engines
// must AGREE on the answer; the timings prove the tiers are worth existing.
// Run: cargo test -p logicaffeine-tests --release --test jit_vm_differential \
//        bench_three_tiers -- --ignored --nocapture
// ===========================================================================

#[test]
#[ignore]
fn bench_three_tiers() {
    let hot = "\
## To work (n: Int) -> Int:
    Let mutable acc be 0.
    Let mutable k be n.
    While k is greater than 0:
        Set acc to acc + k * k - 3.
        Set k to k - 1.
    Return acc.

## Main
Let mutable i be 0.
Let mutable total be 0.
While i is less than 2000:
    Set total to total + work(200).
    Set i to i + 1.
Show total.
";
    use std::time::Instant;

    let t = Instant::now();
    let tw = logicaffeine_compile::compile::tw_outcome(hot);
    let tw_ms = t.elapsed().as_secs_f64() * 1e3;

    let t = Instant::now();
    let vm = logicaffeine_compile::compile::vm_outcome(hot);
    let vm_ms = t.elapsed().as_secs_f64() * 1e3;

    let t = Instant::now();
    let (tiered, _pure, compiles) = tiered_vs_pure_timed(hot);
    let jit_ms = t.elapsed().as_secs_f64() * 1e3;

    assert_eq!(tw.output.trim(), vm.output.trim(), "TW vs VM disagree");
    assert_eq!(vm.output.trim(), tiered.0.trim(), "VM vs tiered disagree");
    assert_eq!(tiered.1, None);
    assert_eq!(compiles, (1, 1), "work() must really tier up");

    eprintln!("tree-walker : {tw_ms:9.1} ms");
    eprintln!("bytecode VM : {vm_ms:9.1} ms  ({:.1}x vs TW)", tw_ms / vm_ms);
    eprintln!("tiered (JIT): {jit_ms:9.1} ms  ({:.1}x vs TW, {:.1}x vs VM)", tw_ms / jit_ms, vm_ms / jit_ms);
}

/// Like tiered_vs_pure but only timing the TIERED run (no pure double-run).
fn tiered_vs_pure_timed(source: &str) -> ((String, Option<String>), (), (u32, u32)) {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compile");
        let tier = ForgeTier::new();
        let mut vm = Vm::new(&program).with_native_tier(&tier);
        let tiered = match vm.run() {
            Ok(()) => (vm.output(), None),
            Err(e) => (vm.output(), Some(e)),
        };
        ((tiered), (), tier.function_counts())
    })
}

// ===========================================================================
// The Futamura composition: PE-specialize the program (projection 1), then
// run the RESIDUAL on the tiered VM. Correctness chain: original-on-TW ==
// original-on-tiered == residual-on-tiered.
// Run: cargo test -p logicaffeine-tests --release --test jit_vm_differential \
//        bench_futamura_pipeline -- --ignored --nocapture
// ===========================================================================

#[test]
#[ignore]
fn bench_futamura_pipeline() {
    use std::time::Instant;
    // Static configuration around dynamic-shaped work: the PE folds the
    // config and specializes; the tier compiles what remains.
    let program = "\
## Main
Let base be 7.
Let scale be 3.
Let mutable i be 0.
Let mutable total be 0.
While i is less than 2000:
    Set total to total + (base * scale) + i.
    Set i to i + 1.
Show total.
";
    let h = std::thread::Builder::new().stack_size(256 * 1024 * 1024).spawn(move || {
        let truth = logicaffeine_compile::compile::tw_outcome(program);

        let t = Instant::now();
        let plain = logicaffeine_compile::compile::vm_outcome(program);
        let plain_ms = t.elapsed().as_secs_f64() * 1e3;

        // Futamura: specialize first.
        let t = Instant::now();
        let residual = logicaffeine_compile::compile::projection1_source_real_fast("", "", program)
            .expect("PE should specialize");
        let pe_ms = t.elapsed().as_secs_f64() * 1e3;

        let t = Instant::now();
        let specialized = logicaffeine_compile::compile::vm_outcome(&residual);
        let resid_ms = t.elapsed().as_secs_f64() * 1e3;

        assert_eq!(truth.error, None);
        assert_eq!(truth.output.trim(), plain.output.trim(), "VM vs TW");
        assert_eq!(
            truth.output.trim(),
            specialized.output.trim(),
            "RESIDUAL diverged from the original!\nresidual:\n{residual}"
        );
        eprintln!("original on VM   : {plain_ms:8.2} ms");
        eprintln!("PE (one-time)    : {pe_ms:8.2} ms");
        eprintln!("residual on VM   : {resid_ms:8.2} ms  ({:.0}x)", plain_ms / resid_ms.max(0.001));
        eprintln!("residual program :\n{residual}");
    }).unwrap();
    h.join().unwrap();
}

// ===========================================================================
// NEXT (RED): Main-loop region tiering — Main's own hot loops must reach the
// JIT (today only FUNCTIONS tier; this is the world's-fastest lever #1).
// ===========================================================================

#[test]
fn main_hot_loop_tiers_up_as_a_region() {
    let src = "\
## Main
Let mutable i be 0.
Let mutable total be 0.
While i is less than 2000:
    Set total to total + i * i.
    Set i to i + 1.
Show total.
";
    let (tiered, pure, compiles) = tiered_vs_pure_regions(src);
    assert_eq!(tiered, pure, "region-tiered Main diverged");
    assert_eq!(compiles, (1, 1), "the hot MAIN loop must compile as a region");
}

#[test]
fn main_loop_with_show_fails_closed() {
    // Show is outside the region subset: one bailed attempt, sticky Failed,
    // and the bytecode path must still produce identical output.
    let src = "\
## Main
Let mutable i be 0.
While i is less than 150:
    Show i.
    Set i to i + 1.
";
    let (tiered, pure, compiles) = tiered_vs_pure_regions(src);
    assert_eq!(tiered, pure, "failed-closed region run diverged");
    assert_eq!(compiles, (1, 0), "one failed attempt, then Failed is sticky");
}

#[test]
fn main_float_loop_compiles_with_typed_speculation() {
    // M5 (EXODIA float tier): float loops are IN the subset — the region
    // speculates on the observed register kinds, guards them per entry, and
    // computes natively with bit-exact IEEE semantics.
    let src = "\
## Main
Let mutable x be 0.5.
Let mutable i be 0.
While i is less than 150:
    Set x to x + 0.25.
    Set i to i + 1.
Show x.
";
    let (tiered, pure, compiles) = tiered_vs_pure_regions(src);
    assert_eq!(tiered, pure, "float-loop region run diverged");
    assert!(compiles.1 >= 1, "the float loop must now compile as a region");
}

#[test]
fn region_deopt_when_guard_register_turns_float() {
    // The inner loop goes hot on Ints and compiles; on a later outer
    // iteration `step` becomes a Float, so the entry guard must fail and the
    // bytecode path must take over — outputs identical.
    let src = "\
## Main
Let mutable j be 0.
Let mutable step be 1.
Let mutable total be 0.
While j is less than 5:
    If j is greater than 2:
        Set step to 0.5.
    Let mutable i be 0.
    While i is less than 100:
        Set total to total + step.
        Set i to i + 1.
    Set j to j + 1.
Show total.
";
    let (tiered, pure, compiles) = tiered_vs_pure_regions(src);
    assert_eq!(tiered, pure, "region deopt diverged");
    assert_eq!(compiles, (1, 1), "the inner loop compiles once, then guards");
}

#[test]
fn nested_main_loops_tier_and_match() {
    // Both loops are candidate regions; whatever subset compiles, the
    // observable outcome must be bit-identical to pure bytecode.
    let src = "\
## Main
Let mutable j be 0.
Let mutable total be 0.
While j is less than 60:
    Let mutable i be 0.
    While i is less than 60:
        Set total to total + i * j.
        Set i to i + 1.
    Set j to j + 1.
Show total.
";
    let (tiered, pure, compiles) = tiered_vs_pure_regions(src);
    assert_eq!(tiered, pure, "nested-loop region run diverged");
    assert!(compiles.1 >= 1, "at least the inner loop must tier up: {compiles:?}");
}

#[test]
fn region_seeded_main_loops_match_pure_bytecode() {
    let _ = REGION_TIER_THRESHOLD; // every generated loop runs well past it
    for seed in 0..120u64 {
        let mut rng = SplitMix64::new(seed);
        let op1 = ["+", "-", "*"][rng.below(3) as usize];
        let op2 = ["+", "-", "*"][rng.below(3) as usize];
        let iters = 150 + rng.below(300);
        let c1 = rng.below(20);
        let c2 = 1 + rng.below(7);
        let a0 = rng.below(40);
        let src = format!(
            "## Main\n\
             Let mutable i be 0.\n\
             Let mutable acc be {a0}.\n\
             Let mutable aux be 1.\n\
             While i is less than {iters}:\n\
             \x20   Set acc to acc {op1} i * {c2}.\n\
             \x20   Set aux to aux {op2} {c1}.\n\
             \x20   Set i to i + 1.\n\
             Show acc.\n\
             Show aux.\n"
        );
        let (tiered, pure, compiles) = tiered_vs_pure_regions(&src);
        assert_eq!(tiered, pure, "seed {seed} diverged:\n{src}");
        assert_eq!(compiles, (1, 1), "seed {seed} must region-tier:\n{src}");
    }
}

/// Regression: the JIT lowered an in-loop `Let curr be a new Seq` on a PINNED
/// list into an in-place buffer REUSE (`ListClear`). That is unsound when the
/// list is ALIASED — knapsack's `Set prev to curr` makes `prev` point at curr's
/// buffer, so the next iteration's reuse wiped prev's live DP row (the JIT
/// produced 395*n instead of the real knapsack value; the VM was correct). The
/// fix bails such a function to the VM. The tiered (JIT) run must match the
/// pure-bytecode (VM) run.
#[test]
fn knapsack_list_reuse_with_alias_jit_matches_vm() {
    let src = r#"## Main
Let n be 300.
Let capacity be n * 5.
Let mutable weights be a new Seq of Int.
Let mutable vals be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push (i * 17 + 3) % 50 + 1 to weights.
    Push (i * 31 + 7) % 100 + 1 to vals.
    Set i to i + 1.
Let cols be capacity + 1.
Let mutable prev be a new Seq of Int.
Set i to 0.
While i is less than cols:
    Push 0 to prev.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable curr be a new Seq of Int.
    Let wi be item (i + 1) of weights.
    Let vi be item (i + 1) of vals.
    Let mutable w be 0.
    While w is at most capacity:
        Let mutable best be item (w + 1) of prev.
        If w is at least wi:
            Let take be item (w - wi + 1) of prev + vi.
            If take is greater than best:
                Set best to take.
        Push best to curr.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#;
    let (tiered, pure, _compiles) = tiered_vs_pure_regions(src);
    assert_eq!(tiered.1, None, "tiered (JIT) run errored: {:?}", tiered.1);
    assert_eq!(pure.1, None, "pure (VM) run errored: {:?}", pure.1);
    assert_eq!(
        tiered.0, pure.0,
        "JIT (tiered) diverged from VM (pure) on knapsack list-reuse+alias"
    );
}
