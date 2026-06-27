//! P13 — Web Worker warm tier data path (HOTSWAP §P13). The browser warm tier compiles
//! a function in a Web Worker and ships it to the main thread; a Worker cannot share
//! memory, so the body crosses as serialized bytes (`postMessage`). That boundary is
//! exactly P12's `encode`/`decode` feeding P11's `install_warm_bytecode`. This proves
//! the full pipeline — slice (producer) → encode (worker side) → decode + install (main
//! side) → run — yields byte-identical output to the baseline VM (the spec's
//! `wasm_warm_equals_baseline`). The actual Worker thread + `postMessage` are browser
//! glue over this proven data path; off-thread *re-optimization* additionally needs the
//! AST (arena-bound) and is the measurement-gated Tiered-mode refinement.

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::fn_bytecode::slice_function;
use logicaffeine_compile::vm::{tier_cache, Compiler, Vm};
use logicaffeine_language::ast::Stmt;

const PROG: &str = "\
## To work (n: Int) -> Int:
    Let total be 0.
    Let i be 0.
    While i is less than n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show work(25).
";

#[test]
fn warm_tier_via_worker_datapath_equals_baseline() {
    // Baseline: the pure VM.
    let baseline = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let mut vm = Vm::new(&program);
        vm.run().expect("runs");
        vm.output().to_string()
    });

    // Warm tier through the Worker boundary: produce → encode → (postMessage) → decode
    // → install → run.
    let warm = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let sym = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "work" => Some(*name),
                _ => None,
            })
            .expect("work defined");
        let fi = *program.fn_index.get(&sym).expect("work in fn_index") as usize;

        // --- worker side: compile the function body and serialize it for postMessage ---
        let produced = slice_function(&program, fi);
        let wire = tier_cache::encode(&produced);

        // --- main side: the message arrives as bytes; decode and install ---
        let received = tier_cache::decode(&wire).expect("worker payload decodes");
        let mut vm = Vm::new(&program);
        vm.install_warm_bytecode(fi, &received);
        vm.run().expect("runs");
        vm.output().to_string()
    });

    assert_eq!(warm.trim(), "300", "sum 0..25 = 300");
    assert_eq!(warm.trim(), baseline.trim(), "warm tier via the worker data path == baseline");
}
