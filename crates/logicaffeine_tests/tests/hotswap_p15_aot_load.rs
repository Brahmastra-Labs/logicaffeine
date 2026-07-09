//! P15 — AOT-native load + dispatch (HOTSWAP §Axis-3): build a function into a cdylib,
//! `dlopen` it, and run a program that calls it — the loaded native code executes
//! INSIDE the interpreter (via `NativeSlot::Ready`) and produces the same result as the
//! pure VM. The crown of the AOT-native tier: a hot function running in the interpreter
//! at compiled-binary speed. `#[ignore]` because it shells out to `cargo` (slow).

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::Ordering;

use logicaffeine_compile::compile::{build_native_cdylib, compile_function_to_native_rust};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::aot_tier::load_aot_native;
use logicaffeine_compile::vm::{Compiler, Vm};
use logicaffeine_jit::ForgeTier;
use logicaffeine_language::ast::Stmt;

const PROG: &str = "\
## To add (a: Int, b: Int) -> Int:
    Return a.

## Main
Show add(1, 2).
";

fn add_fi(stmts: &[Stmt], interner: &logicaffeine_compile::intern::Interner, program: &logicaffeine_compile::vm::CompiledProgram) -> usize {
    let sym = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "add" => Some(*name),
            _ => None,
        })
        .expect("add defined");
    *program.fn_index.get(&sym).expect("add in fn_index") as usize
}

#[test]
#[ignore = "builds + dlopens a cdylib via cargo (slow) — the AOT-native end-to-end proof"]
fn aot_native_function_executes_in_the_interpreter() {
    // Build + load the AOT-native `add`.
    let module = compile_function_to_native_rust(PROG, "add")
        .expect("source compiles")
        .expect("scalar target yields a module");
    let tmp = std::env::temp_dir().join(format!("logos_aot_p15_{}", std::process::id()));
    let so = build_native_cdylib(&module.rust, "logos_aot_add_p15", &tmp).expect("cdylib builds");
    let (aot_fn, calls) =
        load_aot_native(&so, &module.symbol, module.arity).expect("symbol loads");

    // Baseline: the pure VM (forge tier, no AOT).
    let baseline = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let forge = ForgeTier::new();
        let mut vm = Vm::new(&program)
            .with_native_tier(&forge)
            .with_policy_ctx(&policies, interner);
        vm.run().expect("runs");
        vm.output().to_string()
    });

    // AOT: same program, but `add` is pre-installed as the loaded native function.
    let aot_out = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let fi = add_fi(stmts, interner, &program);
        let forge = ForgeTier::new();
        let mut vm = Vm::new(&program)
            .with_native_tier(&forge)
            .with_policy_ctx(&policies, interner);
        vm.install_aot_native(fi, aot_fn);
        vm.run().expect("runs");
        vm.output().to_string()
    });

    let _ = std::fs::remove_dir_all(&tmp);

    assert_eq!(
        aot_out.trim(),
        baseline.trim(),
        "AOT-native output must match the pure VM"
    );
    assert!(
        calls.load(Ordering::Relaxed) >= 1,
        "the loaded AOT-native function must have actually executed in the interpreter"
    );
}
