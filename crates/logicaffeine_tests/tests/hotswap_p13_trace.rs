//! P13 — `LOGOS_TIER_TRACE` observability (HOTSWAP §P13). Locks the public
//! tier-transition line format (other tools parse it) and proves the trace is a pure
//! side-effect: enabling it must not change program output. The warm tier's wasm
//! portability + warm==baseline correctness (the `wasm_warm_equals_baseline` half) are
//! proven by P11 (`hotswap_p11_sidetable` + the wasm32 build gate).

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::fn_bytecode::slice_function;
use logicaffeine_compile::vm::tier_trace::{format_transition, ExecTier};
use logicaffeine_compile::vm::{Compiler, Vm};
use logicaffeine_language::ast::Stmt;

const PROG: &str = "\
## To f (n: Int) -> Int:
    Return n + 1.

## Main
Show f(10).
";

#[test]
fn trace_format_is_stable_across_tiers() {
    assert_eq!(format_transition(3, "fib", ExecTier::Bytecode), "[tier] fn#3 'fib' -> bytecode");
    assert_eq!(format_transition(3, "fib", ExecTier::Warm), "[tier] fn#3 'fib' -> warm");
    assert_eq!(
        format_transition(7, "qsort", ExecTier::NativeForge),
        "[tier] fn#7 'qsort' -> native(forge)"
    );
    assert_eq!(
        format_transition(2, "add", ExecTier::NativeAot),
        "[tier] fn#2 'add' -> native(aot)"
    );
    // No interner / anonymous: index only.
    assert_eq!(format_transition(5, "", ExecTier::Warm), "[tier] fn#5 -> warm");
}

#[test]
fn trace_is_a_pure_side_effect() {
    // Enable the trace for this process, then prove a warm-installed run still produces
    // the baseline output — the trace must never perturb execution.
    std::env::set_var("LOGOS_TIER_TRACE", "1");

    let out = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let f_sym = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "f" => Some(*name),
                _ => None,
            })
            .expect("f defined");
        let fi = *program.fn_index.get(&f_sym).expect("f in fn_index") as usize;
        let body = slice_function(&program, fi);
        let mut vm = Vm::new(&program).with_policy_ctx(&policies, interner);
        vm.install_warm_bytecode(fi, &body); // fires a trace line (name resolved via interner)
        vm.run().expect("runs");
        vm.output().to_string()
    });

    std::env::remove_var("LOGOS_TIER_TRACE");
    assert_eq!(out.trim(), "11", "trace must not change program output");
}
