//! AOT-native tier (HOTSWAP §Axis-3 / T4) — proof the COMPILED native code actually
//! runs inside the interpreter and is genuinely dispatched to (not silently fallen
//! through). For each scalar function: Logos → Rust → `rustc -O3` → cdylib → dlopen →
//! `install_aot_native` → run, asserting (a) output == the pure VM and (b) the loaded
//! function's call counter is non-zero, i.e. the interpreter really executed the
//! compiled machine code. `#[ignore]` — it shells out to `rustc` per function (slow).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{build_native_cdylib_cached, compile_function_to_native_rust};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::aot_tier::load_aot_native;
use logicaffeine_compile::vm::{Compiler, Vm};
use logicaffeine_jit::ForgeTier;
use logicaffeine_language::ast::Stmt;
use std::sync::atomic::Ordering;

/// Run `source` with `fn_name` installed as compiled-native; return (output, call count).
fn run_with_aot(source: &str, fn_name: &str, cache: &std::path::Path) -> (String, u64) {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let sym = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name, .. } if interner.resolve(*name) == fn_name => Some(*name),
                _ => None,
            })
            .expect("function defined");
        let fi = *program.fn_index.get(&sym).expect("in fn_index") as usize;

        let module = compile_function_to_native_rust(source, fn_name)
            .expect("parses")
            .expect("function is in the scalar AOT subset");
        let so = build_native_cdylib_cached(&module.rust, fn_name, cache).expect("cdylib builds");
        let (nf, calls) = load_aot_native(&so, &module.symbol, module.arity).expect("cdylib loads");

        let forge = ForgeTier::new();
        let mut vm = Vm::new(&program)
            .with_native_tier(&forge)
            .with_policy_ctx(&policies, interner);
        vm.install_aot_native(fi, nf);
        vm.run().expect("runs");
        (vm.output().to_string(), calls.load(Ordering::Relaxed))
    })
}

/// The pure VM (no AOT) — the reference output.
fn baseline(source: &str) -> String {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let forge = ForgeTier::new();
        let mut vm = Vm::new(&program)
            .with_native_tier(&forge)
            .with_policy_ctx(&policies, interner);
        vm.run().expect("runs");
        vm.output().to_string()
    })
}

const FIB: &str = "\
## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(20).
";

const GCD: &str = "\
## To gcd (a: Int, b: Int) -> Int:
    If b equals 0:
        Return a.
    Return gcd(b, a % b).

## Main
Show gcd(1071, 462).
";

const SUMTO: &str = "\
## To sumto (n: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 1.
    While i is at most n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show sumto(100).
";

#[test]
#[ignore = "builds cdylibs via rustc per function (slow) — the compiled-native tier proof"]
fn aot_native_tier_executes_and_is_dispatched() {
    let cache = std::env::temp_dir().join(format!("logos_aot_tier_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);

    for (src, fname, expected) in [
        (FIB, "fib", "6765"),
        (GCD, "gcd", "21"),
        (SUMTO, "sumto", "5050"),
    ] {
        assert_eq!(baseline(src).trim(), expected, "{fname}: baseline sanity");
        let (out, calls) = run_with_aot(src, fname, &cache);
        assert_eq!(
            out.trim(),
            expected,
            "{fname}: AOT-native output must match the VM"
        );
        assert!(
            calls > 0,
            "{fname}: the compiled-native function must actually be CALLED \
             (counter={calls}) — proves dispatch hit machine code, not fall-through"
        );
    }

    let _ = std::fs::remove_dir_all(&cache);
}

// ─── Failures inside a native function ──────────────────────────────────────
//
// A native function that stops (a broken contract, `Fail with`, a zero divisor) must report the VM's
// exact reason after the VM's exact output — never abort the process. The shim catches the panic and
// the VM re-runs the call on bytecode, where the kernel raises the reason at the exact point; that
// replay is sound because only effect-free functions are ever compiled native.

/// Like [`run_with_aot`], but the run may fail: (output, error, native call count).
fn run_with_aot_outcome(
    source: &str,
    fn_name: &str,
    cache: &std::path::Path,
) -> (String, Option<String>, u64) {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let sym = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name, .. } if interner.resolve(*name) == fn_name => Some(*name),
                _ => None,
            })
            .expect("function defined");
        let fi = *program.fn_index.get(&sym).expect("in fn_index") as usize;
        let module = compile_function_to_native_rust(source, fn_name)
            .expect("parses")
            .expect("function is in the scalar AOT subset");
        let so = build_native_cdylib_cached(&module.rust, fn_name, cache).expect("cdylib builds");
        let (nf, calls) = load_aot_native(&so, &module.symbol, module.arity).expect("cdylib loads");
        let forge = ForgeTier::new();
        let mut vm = Vm::new(&program)
            .with_native_tier(&forge)
            .with_policy_ctx(&policies, interner);
        vm.install_aot_native(fi, nf);
        let error = vm.run().err().map(|e| e.to_string());
        (
            vm.output().to_string(),
            error,
            calls.load(Ordering::Relaxed),
        )
    })
}

fn assert_native_failure_is_the_vms(source: &str, fn_name: &str, label: &str) {
    let dir = std::env::temp_dir().join(format!("logos_aot_fail_{label}_{}", std::process::id()));
    let (out, err, calls) = run_with_aot_outcome(source, fn_name, &dir);
    let tw = logicaffeine_compile::compile::tw_outcome(source);
    assert!(
        calls > 0,
        "{label}: the native code must actually run (0 calls)"
    );
    assert_eq!(
        out.trim(),
        tw.output.trim(),
        "{label}: output before the failure"
    );
    assert_eq!(err, tw.error, "{label}: the failure reason");
    assert!(tw.error.is_some(), "{label}: the reference run must fail");
}

#[test]
#[ignore]
fn a_broken_contract_inside_a_native_function_reports_the_vms_reason() {
    assert_native_failure_is_the_vms(
        "## To checked (n: Int) -> Int:\n    Require that n is less than 150, otherwise \"too big: \" + format(n).\n    Return n * 2.\n\n\
         ## Main\nLet mutable i be 0.\nLet mutable total be 0.\nWhile i is less than 400:\n    Set total to total + checked(i).\n    Set i to i + 1.\nShow total.\n",
        "checked",
        "contract",
    );
}

#[test]
#[ignore]
fn a_zero_divisor_inside_a_native_function_reports_the_vms_reason() {
    assert_native_failure_is_the_vms(
        "## To inv (n: Int) -> Int:\n    Return 1000 / (n - 120).\n\n\
         ## Main\nLet mutable i be 0.\nLet mutable total be 0.\nWhile i is less than 400:\n    Set total to total + inv(i).\n    Set i to i + 1.\nShow total.\n",
        "inv",
        "divzero",
    );
}

#[test]
fn a_function_with_an_effect_is_never_compiled_native() {
    let src =
        "## To noisy (n: Int) -> Int:\n    Show n.\n    Return n + 1.\n\n## Main\nShow noisy(1).\n";
    assert!(
        compile_function_to_native_rust(src, "noisy").expect("parses").is_none(),
        "a function that shows cannot be native: a failure replay would print twice, and native output bypasses the VM's"
    );
}
