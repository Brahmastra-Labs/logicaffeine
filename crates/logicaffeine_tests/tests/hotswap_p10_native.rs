//! P10 — Axis-1 native hot-swap (HOTSWAP §P10). Its active mechanisms are delivered by
//! P8 (the run path optimizes the whole program upfront, so forge native-compiles the
//! OPTIMIZED body) and P11 (a native bail returns `NativeDisposition::Interpret`, and the
//! `Call` then re-dispatches FnTable → warm → baseline — so a warm body is the bytecode
//! deopt-fallback). What P11's non-recursive test left open is M3: a tier swap must
//! replay correctly across RECURSIVE mixed-tier frames, where every `CallFrame` carries
//! its `func` so each recursive activation re-dispatches and returns on the right body.
//! This proves that — a warm-installed recursive function runs byte-identically to the
//! baseline through deep self-recursion.

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::fn_bytecode::slice_function;
use logicaffeine_compile::vm::{Compiler, Vm};
use logicaffeine_language::ast::Stmt;

const PROG: &str = "\
## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(20).
";

fn run(install_warm: bool) -> String {
    with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let mut vm = Vm::new(&program);
        if install_warm {
            let sym = stmts
                .iter()
                .find_map(|s| match s {
                    Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "fib" => Some(*name),
                    _ => None,
                })
                .expect("fib defined");
            let fi = *program.fn_index.get(&sym).expect("fib in fn_index") as usize;
            let body = slice_function(&program, fi);
            vm.install_warm_bytecode(fi, &body);
        }
        vm.run().expect("runs");
        vm.output().to_string()
    })
}

#[test]
fn warm_dispatch_is_correct_through_deep_recursion() {
    let baseline = run(false);
    let warm = run(true);
    assert_eq!(baseline.trim(), "6765", "fib(20) = 6765");
    // Every one of the thousands of recursive `fib` activations re-dispatches into the
    // warm body and returns on the right frame — byte-identical to the baseline.
    assert_eq!(warm.trim(), baseline.trim(), "warm recursive dispatch matches baseline");
}
