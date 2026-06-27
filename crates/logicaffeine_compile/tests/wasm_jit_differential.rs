//! WS6 (FINISH_INTERPRETER Phase 13) — WASM-JIT ↔ VM differential.
//!
//! The WebAssembly module the backend emits from a function's real VM bytecode (run through
//! the conformant `wasmi` interpreter) must produce the SAME result as the bytecode VM. This
//! is the codegen-correctness oracle: a curated set of real functions PLUS a seeded fuzz of
//! random integer arithmetic, each cross-checked against the VM. It runs in the regular fast
//! suite (the runner enables `--features wasm-jit`), so a codegen regression is *caught*,
//! not hoped away. An eligibility counter makes a vacuous pass impossible — if the backend
//! never fires on real bytecode, the test fails loudly.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use logicaffeine_compile::compile::vm_run_source;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::wasm_jit::{compile_function_to_wasm, compile_region_to_wasm};
use logicaffeine_compile::vm::{Compiler, Constant, Op};

/// Run the emitted module's `f(arg)` through wasmi — `None` if it TRAPS. A trap is the
/// WASM-JIT's overflow deopt (checked add/sub/mul `unreachable` on signed overflow): the host
/// surfaces it as an error and `WasmTier::call` returns `None`, falling back to the VM. So a
/// `None` here is a sound deopt, not a failure. (Instantiation also validates the bytes.)
fn run_wasmi(module: &[u8], arg: i64) -> Option<i64> {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::<()>::new(&engine)
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let f = instance.get_typed_func::<i64, i64>(&store, "f").unwrap();
    f.call(&mut store, arg).ok()
}

/// Two-argument variant of [`run_wasmi`] (for multi-param regions like `f(a, count)`).
fn run_wasmi2(module: &[u8], a: i64, b: i64) -> Option<i64> {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::<()>::new(&engine)
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let f = instance.get_typed_func::<(i64, i64), i64>(&store, "f").unwrap();
    f.call(&mut store, (a, b)).ok()
}

/// Emit `fn_name`'s WASM module from `src` — `None` if the function is not WASM-JIT-eligible.
fn wasm_jit_bytes(src: &str, fn_name: &str) -> Option<Vec<u8>> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.ok()?;
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).ok()?;
        let sym = interner.lookup(fn_name)?;
        let fi = *program.fn_index.get(&sym)? as usize;
        compile_function_to_wasm(&program, fi)
    })
}

/// The VM ground truth for `Show fn_name(arg).` — the printed value as a string (it may be a
/// BigInt once the EXACT-integer arithmetic promotes past i64).
fn vm_output(src: &str) -> String {
    vm_run_source(src).expect("VM runs").trim().to_string()
}

/// The SOUNDNESS contract for `f(arg)`, when the function is WASM-JIT-eligible:
/// - the module returns a value ⟹ it computed the whole kernel in i64 without overflowing,
///   so it MUST equal the VM's output exactly (never a silent wrap);
/// - the module traps ⟹ some intermediate overflowed i64 → a sound deopt to the VM. We do NOT
///   compare here: the VM's *final* result may still fit i64 (it carries BigInt intermediates
///   and downsizes — e.g. `(x+5)-5` recovers `i64::MAX`), so a trap does not imply the final
///   value overflows. Trapping is always sound (the WASM-JIT returns nothing, never a wrong
///   value); the in-range correctness is what the returned case pins.
/// Returns whether the function was eligible, so callers can assert the backend actually fired.
fn check(fn_def: &str, fn_name: &str, arg: i64) -> bool {
    let src = format!("{fn_def}\n## Main\n    Show {fn_name}({arg}).\n");
    let vm = vm_output(&src);
    let Some(bytes) = wasm_jit_bytes(&src, fn_name) else {
        return false;
    };
    if let Some(jit) = run_wasmi(&bytes, arg) {
        assert_eq!(
            jit.to_string(),
            vm,
            "WASM-JIT disagrees with the VM for {fn_name}({arg}): jit={jit} vm={vm}\n{fn_def}"
        );
    }
    true
}

#[test]
fn wasm_jit_matches_vm_on_real_functions() {
    // Curated pure-integer functions whose compiled bytecode exercises straight-line
    // arithmetic, a counted loop, and a branch.
    // The large args (e.g. 4_000_000_000) overflow i64 inside the kernel, exercising the
    // checked-arithmetic deopt: the VM promotes to BigInt and the WASM-JIT must TRAP (never
    // wrap), which `check` verifies.
    let cases: &[(&str, &str, &[i64])] = &[
        ("## To poly (x: Int) -> Int:\n    Return x * x + x.", "poly", &[0, 1, 5, 12, 4_000_000_000]),
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
        "no curated function was WASM-JIT-eligible — the backend never fired on real bytecode"
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

/// A small, total integer expression over `x` and literals `0..9`, using only `+ - *` so it
/// never errors and (with the bounded depth/literals) stays well inside i64 — so VM and WASM
/// (both wrapping i64) agree exactly.
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

#[test]
fn wasm_jit_fuzz_matches_vm() {
    let mut eligible = 0usize;
    for seed in 0..150u64 {
        let mut state = seed ^ 0xD1B5_4A32_D192_ED03;
        let body = gen_expr(&mut state, 3);
        let def = format!("## To f (x: Int) -> Int:\n    Return {body}.");
        // Small args stay in range (WASM-JIT returns, must equal the VM); the large ones
        // overflow many of the generated expressions, exercising the checked-overflow deopt
        // (WASM-JIT traps, the VM promotes to BigInt) — both paths verified by `check`.
        for &arg in &[0i64, 1, 2, 5, 9, 3_000_000_000, i64::MAX] {
            if check(&def, "f", arg) {
                eligible += 1;
            }
        }
    }
    assert!(
        eligible >= 1,
        "the fuzz produced no WASM-JIT-eligible functions — the backend never fired"
    );
}

#[test]
fn wasm_tier_fires_after_threshold_and_matches_vm() {
    use logicaffeine_compile::vm::wasm_jit::WasmTier;

    let fn_def = "## To sq (x: Int) -> Int:\n    Return x * x.";
    let arg = 9i64;
    let src = format!("{fn_def}\n## Main\n    Show sq({arg}).\n");
    let vm: i64 = vm_output(&src).parse().expect("sq(9) = 81 fits i64"); // 81

    let outcome = with_parsed_program(&src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.ok()?;
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).ok()?;
        let sym = interner.lookup("sq")?;
        let fi = *program.fn_index.get(&sym)?;
        let mut tier = WasmTier::new(3);
        // First two calls warm the counter (bytecode tier); the third crosses the
        // threshold and tiers up; the rest dispatch to the compiled module.
        let r1 = tier.on_call(&program, fi, &[arg]);
        let r2 = tier.on_call(&program, fi, &[arg]);
        let r3 = tier.on_call(&program, fi, &[arg]);
        let r4 = tier.on_call(&program, fi, &[arg]);
        Some((r1, r2, r3, r4, tier.hits()))
    });
    let (r1, r2, r3, r4, hits) = outcome.expect("sq must parse + compile");
    assert_eq!(r1, None, "call 1 is below threshold — bytecode tier");
    assert_eq!(r2, None, "call 2 is below threshold — bytecode tier");
    assert_eq!(r3, Some(vm), "call 3 tiers up and must match the VM ({vm})");
    assert_eq!(r4, Some(vm), "call 4 dispatches to the cached module");
    assert!(hits >= 2, "the WASM-JIT tier must have fired (got {hits} hits)");
}

/// A function whose return value is a `Bool` (a comparison result) must NOT tier: the WASM-JIT
/// returns an i64 the caller boxes as `Int`, so a `Bool` would be mis-typed (`Show` prints `1`,
/// not `true`). It must deopt to the VM, which types the result correctly. Only declared-`Int`
/// returns are WASM-JIT-eligible — keeping the tiers in sync on return types.
#[test]
fn wasm_jit_declines_non_int_returning_function() {
    let src = "## To gt5 (x: Int) -> Bool:\n    Return x is greater than 5.\n\
               ## Main\n    Show gt5(8).\n";
    assert!(
        wasm_jit_bytes(src, "gt5").is_none(),
        "a Bool-returning function must be WASM-JIT-ineligible (deopt, not mis-boxed as Int)"
    );
    // The program still runs correctly on the VM — and prints the Bool, not `1`.
    assert_eq!(vm_output(src), "true");

    // An Int-returning function with the SAME comparison inside (used only for control flow,
    // returning an Int) stays eligible — the boundary is the RETURN type, not the ops used.
    let int_src = "## To clamp (x: Int) -> Int:\n    \
                   If x is greater than 5:\n        Return 5.\n    Return x.\n\
                   ## Main\n    Show clamp(8).\n";
    assert!(wasm_jit_bytes(int_src, "clamp").is_some(), "Int-returning fn with a compare must still tier");
}

/// The EXACT-integer contract, in sync with the VM: checked add/sub/mul return the in-range
/// result and TRAP on signed overflow (so the task deopts to the VM, which promotes to
/// BigInt) — never a silent wrap. A trap surfaces as a `None` from [`run_wasmi`].
#[test]
fn wasm_jit_checked_arith_traps_on_overflow_never_wraps() {
    // f(x) = x * x.
    let sq = compile_region_to_wasm(&[Op::Mul { dst: 1, lhs: 0, rhs: 0 }, Op::Return { src: 1 }], &[], 1, 2)
        .expect("mul region emits");
    for x in [0i64, 1, -1, 1000, -1000, 3_037_000_499] {
        // 3_037_000_499² = 9_223_372_030_926_249_001 < i64::MAX — in range.
        assert_eq!(run_wasmi(&sq, x), Some(x * x), "in-range x*x for x={x}");
    }
    for x in [4_000_000_000i64, 5_000_000_000, i64::MAX, i64::MIN, 0x1_0000_0000] {
        assert_eq!(
            run_wasmi(&sq, x),
            None,
            "x*x must TRAP on overflow for x={x}, not silently wrap to {}",
            x.wrapping_mul(x)
        );
    }

    // f(x) = x + 1 and x - 1: the boundary cases that promote in the VM.
    let inc = compile_region_to_wasm(
        &[Op::LoadConst { dst: 1, idx: 0 }, Op::Add { dst: 2, lhs: 0, rhs: 1 }, Op::Return { src: 2 }],
        &[Constant::Int(1)],
        1,
        3,
    )
    .expect("add region emits");
    assert_eq!(run_wasmi(&inc, 41), Some(42), "in-range add");
    assert_eq!(run_wasmi(&inc, i64::MAX), None, "i64::MAX + 1 must trap");

    let dec = compile_region_to_wasm(
        &[Op::LoadConst { dst: 1, idx: 0 }, Op::Sub { dst: 2, lhs: 0, rhs: 1 }, Op::Return { src: 2 }],
        &[Constant::Int(1)],
        1,
        3,
    )
    .expect("sub region emits");
    assert_eq!(run_wasmi(&dec, 43), Some(42), "in-range sub");
    assert_eq!(run_wasmi(&dec, i64::MIN), None, "i64::MIN - 1 must trap");
}

/// Bitwise XOR + arithmetic/logical shifts match the VM's exact semantics: `^`, `wrapping_shl`,
/// and `wrapping_shr` (ARITHMETIC for signed i64), with the shift count masked mod 64 — which
/// WASM's `i64.xor` / `i64.shl` / `i64.shr_s` reproduce bit-for-bit.
#[test]
fn wasm_jit_bitwise_and_shift_match_vm_semantics() {
    // f(x) = ((x ^ 0xF0F0) << 3) >> 1   (arithmetic >>, sign-preserving).
    let ops = vec![
        Op::LoadConst { dst: 1, idx: 0 },      // 0xF0F0
        Op::BitXor { dst: 2, lhs: 0, rhs: 1 }, // x ^ 0xF0F0
        Op::LoadConst { dst: 3, idx: 1 },      // 3
        Op::Shl { dst: 4, lhs: 2, rhs: 3 },    // << 3
        Op::LoadConst { dst: 5, idx: 2 },      // 1
        Op::Shr { dst: 6, lhs: 4, rhs: 5 },    // >> 1
        Op::Return { src: 6 },
    ];
    let consts = vec![Constant::Int(0xF0F0), Constant::Int(3), Constant::Int(1)];
    let bytes = compile_region_to_wasm(&ops, &consts, 1, 7).expect("bitwise region emits");
    for x in [0i64, 1, 255, -1, -1000, i64::MIN, i64::MAX, 0x1234_5678] {
        let expect = ((x ^ 0xF0F0).wrapping_shl(3)).wrapping_shr(1);
        assert_eq!(run_wasmi(&bytes, x), Some(expect), "((x ^ 0xF0F0) << 3) >> 1 for x={x}");
    }

    // Shift-count masking mod 64: a 2-param region `f(a, count) = a << count`.
    let shl = compile_region_to_wasm(&[Op::Shl { dst: 2, lhs: 0, rhs: 1 }, Op::Return { src: 2 }], &[], 2, 3)
        .expect("shl region emits");
    for (a, c) in [(1i64, 0i64), (1, 1), (1, 63), (1, 64), (1, 65), (-8, 2), (255, 4)] {
        assert_eq!(run_wasmi2(&shl, a, c), Some(a.wrapping_shl(c as u32)), "{a} << {c} (mod-64 count)");
    }
}
