//! P18 — background AOT-native auto-compile (HOTSWAP §Axis-3): a hot function's
//! optimized form is built to a cdylib + loaded on a WORKER thread (off the
//! interpreter), then installed and executed via the existing `NativeSlot::Ready`
//! dispatch — the AOT realization of "optimize hot functions during the run." Proves
//! the background build + install + execute round-trips and matches the pure VM.
//! `#[ignore]` because the worker shells out to `cargo` (slow).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::bg_aot::{AotRequest, BgAotCompiler};
use logicaffeine_compile::vm::{Compiler, Vm};
use logicaffeine_jit::ForgeTier;
use logicaffeine_language::ast::Stmt;

const PROG: &str = "\
## To add (a: Int, b: Int) -> Int:
    Return a.

## Main
Show add(1, 2).
";

#[test]
#[ignore = "background-builds a cdylib via cargo (slow) — bg AOT auto-compile proof"]
fn background_aot_builds_installs_and_executes() {
    let cache = std::env::temp_dir().join(format!("logos_bgaot_{}", std::process::id()));

    let aot_out = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let add_sym = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "add" => Some(*name),
                _ => None,
            })
            .expect("add defined");
        let fi = *program.fn_index.get(&add_sym).expect("add in fn_index") as usize;

        // Background-build `add` to native, off the interpreter thread.
        let mut bg = BgAotCompiler::new();
        bg.submit(AotRequest {
            fi,
            source: PROG.to_string(),
            fn_name: "add".to_string(),
            cache_dir: cache.clone(),
        });
        let results = bg.drain_blocking(); // waits for the worker's rustc build
        assert_eq!(results.len(), 1, "exactly one build result");
        let result = results.into_iter().next().unwrap();
        assert_eq!(result.fi, fi);
        let nf = result.nf.expect("background AOT build produced a native function");

        // Install the background-built native fn and run.
        let forge = ForgeTier::new();
        let mut vm = Vm::new(&program)
            .with_native_tier(&forge)
            .with_policy_ctx(&policies, interner);
        vm.install_aot_native(fi, nf);
        vm.run().expect("runs");
        vm.output().to_string()
    });

    // Baseline: the pure VM.
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

    let _ = std::fs::remove_dir_all(&cache);
    assert_eq!(
        aot_out.trim(),
        baseline.trim(),
        "background-AOT-compiled output must match the pure VM"
    );
}
