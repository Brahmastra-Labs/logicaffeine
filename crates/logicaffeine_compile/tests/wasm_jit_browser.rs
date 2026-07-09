//! WS6 (FINISH_INTERPRETER Phase 13) — the WASM-JIT runs on the host's REAL `WebAssembly`.
//!
//! The native differential (`wasm_jit_differential.rs`) cross-checks the emitter against the
//! pure-Rust `wasmi` interpreter. That proves the *bytes* are correct, but not that they run
//! on the *production host* — the browser's (and node's) real `WebAssembly`, the V8 engine
//! that actually JITs them. This suite closes that gap: it instantiates the emitted modules
//! through `js_sys::WebAssembly` (`run_on_host`) and through the real `WasmTier` (whose
//! wasm32 host is `WebAssembly`), on the `wasm32` target, under `wasm-bindgen-test-runner`
//! (node's V8) / `wasm-pack test --headless` (a browser). Every module is cross-checked
//! against the bytecode VM — same oracle as the native suite, now on the real engine.
//!
//! It also pins the i64↔BigInt boundary: WebAssembly `i64` crosses to JS as `BigInt`, and a
//! naive `f64` round-trip would silently corrupt values past 2^53. The marshaling test
//! drives `i64::MIN`/`i64::MAX` through real `WebAssembly` and demands exact equality.
//!
//! Only builds on `wasm32` with the `wasm-jit` feature; inert on a normal `cargo test`.

#![cfg(all(target_arch = "wasm32", feature = "wasm-jit"))]

use logicaffeine_compile::compile::vm_run_source;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::wasm_jit::{
    compile_function_to_wasm, compile_region_to_wasm, run_on_host, WasmTier,
};
use logicaffeine_compile::vm::{Compiler, Constant, Op};
use wasm_bindgen_test::*;

/// Lower `fn_name` from `src` to a WASM module and run `f(arg)` on the host's real
/// `WebAssembly` — `None` if the function is not WASM-JIT-eligible (a sound deopt).
fn jit_on_host(src: &str, fn_name: &str, arg: i64) -> Option<i64> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.ok()?;
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).ok()?;
        let sym = interner.lookup(fn_name)?;
        let fi = *program.fn_index.get(&sym)? as usize;
        let bytes = compile_function_to_wasm(&program, fi)?;
        run_on_host(&bytes, &[arg])
    })
}

/// The VM ground truth for `Show fn_name(arg).` — the printed integer.
fn vm_result(src: &str) -> i64 {
    vm_run_source(src)
        .expect("VM runs")
        .trim()
        .parse()
        .expect("integer output")
}

/// `f(arg)` on real `WebAssembly` must equal `f(arg)` via the VM (when eligible). Returns
/// whether the function was eligible, so callers can assert the backend actually fired.
fn check(fn_def: &str, fn_name: &str, arg: i64) -> bool {
    let src = format!("{fn_def}\n## Main\n    Show {fn_name}({arg}).\n");
    let vm = vm_result(&src);
    match jit_on_host(&src, fn_name, arg) {
        Some(jit) => {
            assert_eq!(
                jit, vm,
                "host WebAssembly disagrees with the VM for {fn_name}({arg}): host={jit} vm={vm}\n{fn_def}"
            );
            true
        }
        None => false,
    }
}

/// Hand-built integer regions execute on the host's real `WebAssembly` with the exact
/// results — straight-line, a counted loop, and a branch. No compile pipeline involved, so
/// this isolates the codegen + browser host (and i64/BigInt marshaling) end to end.
#[wasm_bindgen_test]
fn wasm_jit_runs_on_real_webassembly_host() {
    // f(x) = x*x + x
    let poly = compile_region_to_wasm(
        &[
            Op::Mul { dst: 1, lhs: 0, rhs: 0 },
            Op::Add { dst: 2, lhs: 1, rhs: 0 },
            Op::Return { src: 2 },
        ],
        &[],
        1,
        3,
    )
    .expect("poly region emits");
    assert_eq!(run_on_host(&poly, &[5]), Some(30));
    assert_eq!(run_on_host(&poly, &[7]), Some(56));
    assert_eq!(run_on_host(&poly, &[0]), Some(0));

    // f(n) = n + (n-1) + ... + 1 via a counted loop.
    let tri = compile_region_to_wasm(
        &[
            Op::LoadConst { dst: 1, idx: 0 },
            Op::LoadConst { dst: 2, idx: 1 },
            Op::LoadConst { dst: 3, idx: 0 },
            Op::Gt { dst: 4, lhs: 0, rhs: 3 },
            Op::JumpIfFalse { cond: 4, target: 8 },
            Op::Add { dst: 1, lhs: 1, rhs: 0 },
            Op::Sub { dst: 0, lhs: 0, rhs: 2 },
            Op::Jump { target: 3 },
            Op::Return { src: 1 },
        ],
        &[Constant::Int(0), Constant::Int(1)],
        1,
        5,
    )
    .expect("loop region emits");
    assert_eq!(run_on_host(&tri, &[5]), Some(15));
    assert_eq!(run_on_host(&tri, &[100]), Some(5050));
    assert_eq!(run_on_host(&tri, &[0]), Some(0));

    // f(x) = if x < 10 { x * 2 } else { x + 100 }
    let branch = compile_region_to_wasm(
        &[
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Lt { dst: 2, lhs: 0, rhs: 1 },
            Op::JumpIfFalse { cond: 2, target: 6 },
            Op::LoadConst { dst: 3, idx: 1 },
            Op::Mul { dst: 4, lhs: 0, rhs: 3 },
            Op::Return { src: 4 },
            Op::LoadConst { dst: 5, idx: 2 },
            Op::Add { dst: 4, lhs: 0, rhs: 5 },
            Op::Return { src: 4 },
        ],
        &[Constant::Int(10), Constant::Int(2), Constant::Int(100)],
        1,
        6,
    )
    .expect("branch region emits");
    assert_eq!(run_on_host(&branch, &[3]), Some(6));
    assert_eq!(run_on_host(&branch, &[50]), Some(150));
    assert_eq!(run_on_host(&branch, &[10]), Some(110));
}

/// The i64↔BigInt boundary is lossless across the full i64 range — including the values
/// (`i64::MIN`, `i64::MAX`, and others past 2^53) where a naive `f64` marshal would corrupt.
#[wasm_bindgen_test]
fn wasm_jit_marshals_full_i64_range_losslessly() {
    // f(x) = x — the identity, so the result IS the marshaled round-trip.
    let id = compile_region_to_wasm(&[Op::Return { src: 0 }], &[], 1, 1).expect("id emits");
    for v in [
        0i64,
        1,
        -1,
        2,
        -2,
        9_007_199_254_740_993, // 2^53 + 1 — first integer f64 cannot represent
        -9_007_199_254_740_993,
        1_234_567_890_123_456_789,
        -1_234_567_890_123_456_789,
        i64::MAX,
        i64::MIN,
    ] {
        assert_eq!(
            run_on_host(&id, &[v]),
            Some(v),
            "i64 {v} did not round-trip losslessly through the host BigInt boundary"
        );
    }

    // Two-argument marshaling: f(a, b) = a - b. In-range values round-trip; the overflow case
    // TRAPS (the checked-arithmetic deopt), since integer arithmetic is EXACT — `a - b` no
    // longer wraps, it promotes in the VM, so the WASM-JIT must decline rather than wrap.
    let sub = compile_region_to_wasm(&[Op::Sub { dst: 2, lhs: 0, rhs: 1 }, Op::Return { src: 2 }], &[], 2, 3)
        .expect("sub emits");
    assert_eq!(run_on_host(&sub, &[10, 25]), Some(-15));
    assert_eq!(run_on_host(&sub, &[i64::MAX, 1]), Some(i64::MAX - 1));
    assert_eq!(run_on_host(&sub, &[i64::MAX, -1]), None, "MAX - (-1) overflows → must trap, not wrap");
}

/// On real V8: the EXACT-integer contract — checked add/sub/mul return in-range results and
/// TRAP on signed overflow (deopt to the VM), never silently wrap. And bitwise/shift match
/// the VM's `^` / `wrapping_shl` / arithmetic `wrapping_shr` (count masked mod 64).
#[wasm_bindgen_test]
fn wasm_jit_checked_overflow_and_bitwise_on_v8() {
    let sq = compile_region_to_wasm(&[Op::Mul { dst: 1, lhs: 0, rhs: 0 }, Op::Return { src: 1 }], &[], 1, 2)
        .expect("mul emits");
    assert_eq!(run_on_host(&sq, &[3_037_000_499]), Some(3_037_000_499i64 * 3_037_000_499)); // in range
    assert_eq!(run_on_host(&sq, &[4_000_000_000]), None, "x*x overflow must trap on V8");
    assert_eq!(run_on_host(&sq, &[i64::MIN]), None, "MIN*MIN overflow must trap on V8");

    // ((x ^ 0xF0F0) << 3) >> 1 — bitwise + arithmetic shift, matching the VM.
    let ops = vec![
        Op::LoadConst { dst: 1, idx: 0 },
        Op::BitXor { dst: 2, lhs: 0, rhs: 1 },
        Op::LoadConst { dst: 3, idx: 1 },
        Op::Shl { dst: 4, lhs: 2, rhs: 3 },
        Op::LoadConst { dst: 5, idx: 2 },
        Op::Shr { dst: 6, lhs: 4, rhs: 5 },
        Op::Return { src: 6 },
    ];
    let consts = vec![Constant::Int(0xF0F0), Constant::Int(3), Constant::Int(1)];
    let bytes = compile_region_to_wasm(&ops, &consts, 1, 7).expect("bitwise emits");
    for x in [0i64, 1, 255, -1, -1000, i64::MIN, i64::MAX] {
        let expect = ((x ^ 0xF0F0).wrapping_shl(3)).wrapping_shr(1);
        assert_eq!(run_on_host(&bytes, &[x]), Some(expect), "bitwise/shift for x={x} on V8");
    }
}

/// Curated real functions, compiled from source and run on the host's real `WebAssembly`,
/// each cross-checked against the VM — the differential, now on V8.
#[wasm_bindgen_test]
fn wasm_jit_browser_matches_vm_on_real_functions() {
    let cases: &[(&str, &str, &[i64])] = &[
        ("## To poly (x: Int) -> Int:\n    Return x * x + x.", "poly", &[0, 1, 5, 12]),
        (
            "## To tri (n: Int) -> Int:\n    \
             Let acc be 0.\n    \
             Let i be n.\n    \
             While i is greater than 0:\n        \
             Set acc to acc + i.\n        \
             Set i to i - 1.\n    \
             Return acc.",
            "tri",
            &[0, 1, 10, 100],
        ),
        (
            "## To pick (x: Int) -> Int:\n    \
             If x is greater than 10:\n        \
             Return x + 100.\n    \
             Return x * 2.",
            "pick",
            &[3, 9, 10, 50],
        ),
    ];
    let mut eligible = 0usize;
    for (def, name, args) in cases {
        for &arg in *args {
            if check(def, name, arg) {
                eligible += 1;
            }
        }
    }
    assert!(
        eligible >= 1,
        "no curated function was WASM-JIT-eligible — the backend never fired on V8"
    );
}

/// SplitMix64 — deterministic fuzz PRNG (no `Math.random`).
fn next_rand(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A small total integer expression over `x` and literals `0..9`, using only `+ - *`.
fn gen_expr(state: &mut u64, depth: u32) -> String {
    if depth == 0 || next_rand(state) % 3 == 0 {
        if next_rand(state) % 2 == 0 {
            "x".to_string()
        } else {
            format!("{}", next_rand(state) % 10)
        }
    } else {
        let op = match next_rand(state) % 3 {
            0 => "+",
            1 => "-",
            _ => "*",
        };
        let l = gen_expr(state, depth - 1);
        let r = gen_expr(state, depth - 1);
        format!("({l} {op} {r})")
    }
}

/// Seeded fuzz of random integer expressions — each run on the host's real `WebAssembly` and
/// cross-checked against the VM. A smaller fleet than native (each run is a real V8
/// instantiation) but enough to exercise the boundary.
#[wasm_bindgen_test]
fn wasm_jit_browser_fuzz_matches_vm() {
    let mut eligible = 0usize;
    for seed in 0..40u64 {
        let mut state = seed ^ 0xD1B5_4A32_D192_ED03;
        let body = gen_expr(&mut state, 3);
        let def = format!("## To f (x: Int) -> Int:\n    Return {body}.");
        for &arg in &[0i64, 1, 2, 5, 9] {
            if check(&def, "f", arg) {
                eligible += 1;
            }
        }
    }
    assert!(
        eligible >= 1,
        "the fuzz produced no WASM-JIT-eligible functions — the backend never fired on V8"
    );
}

/// The real `WasmTier` (whose wasm32 host IS `WebAssembly`) tiers a hot function up to a
/// host-compiled module and dispatches to it, matching the VM — the production tier-up path,
/// on the production engine.
#[wasm_bindgen_test]
fn wasm_tier_browser_fires_and_matches_vm() {
    let fn_def = "## To sq (x: Int) -> Int:\n    Return x * x.";
    let arg = 9i64;
    let src = format!("{fn_def}\n## Main\n    Show sq({arg}).\n");
    let vm = vm_result(&src); // 81

    let outcome = with_parsed_program(&src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.ok()?;
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).ok()?;
        let sym = interner.lookup("sq")?;
        let fi = *program.fn_index.get(&sym)?;
        let mut tier = WasmTier::new(3);
        let r1 = tier.on_call(&program, fi, &[arg]);
        let r2 = tier.on_call(&program, fi, &[arg]);
        let r3 = tier.on_call(&program, fi, &[arg]);
        let r4 = tier.on_call(&program, fi, &[arg]);
        Some((r1, r2, r3, r4, tier.hits()))
    });
    let (r1, r2, r3, r4, hits) = outcome.expect("sq must parse + compile");
    assert_eq!(r1, None, "call 1 is below threshold — bytecode tier");
    assert_eq!(r2, None, "call 2 is below threshold — bytecode tier");
    assert_eq!(r3, Some(vm), "call 3 tiers up onto WebAssembly and must match the VM ({vm})");
    assert_eq!(r4, Some(vm), "call 4 dispatches to the cached host module");
    assert!(hits >= 2, "the WASM-JIT tier must have fired on V8 (got {hits} hits)");
}
