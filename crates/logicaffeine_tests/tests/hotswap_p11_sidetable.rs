//! P11 — Axis-1 warm-bytecode side-table (HOTSWAP §7). A re-optimized function body
//! is installed into the VM's `warm_code` buffer and dispatched on `Call` instead of
//! the baseline body — pure bytecode, no forge / no `rustc`, so it is the browser's
//! hot-swap tier. Two proofs: installing a function's OWN body leaves output identical
//! (warm dispatch is sound), and installing a DIFFERENT body changes the result
//! (the side-table is genuinely consulted, not silently bypassed).

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::fn_bytecode::slice_function;
use logicaffeine_compile::vm::{Compiler, Op, Vm};
use logicaffeine_language::ast::Stmt;

const PROG: &str = "\
## To f (n: Int) -> Int:
    Return n + 1.

## To g (n: Int) -> Int:
    Return n + n.

## Main
Show f(10).
";

#[test]
fn warm_bytecode_dispatch_matches_baseline() {
    // Baseline (no warm body installed): f(10) = 11.
    let baseline = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let mut vm = Vm::new(&program);
        vm.run().expect("runs");
        vm.output().to_string()
    });
    assert_eq!(baseline.trim(), "11");

    // Install f's OWN body as its warm tier — output must be byte-identical.
    let warm = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parses");
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
        let mut vm = Vm::new(&program);
        vm.install_warm_bytecode(fi, &body);
        vm.run().expect("runs");
        vm.output().to_string()
    });
    assert_eq!(warm.trim(), baseline.trim(), "warm dispatch must match baseline");
}

#[test]
fn warm_side_table_is_actually_consulted() {
    // Install g's body (n + n) AS f's warm tier: calling f(10) must now run g's body
    // → 20, not the baseline 11. This proves the side-table is consulted on dispatch.
    let routed = with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let find = |name: &str| {
            let sym = stmts
                .iter()
                .find_map(|s| match s {
                    Stmt::FunctionDef { name: nm, .. } if interner.resolve(*nm) == name => Some(*nm),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{name} defined"));
            *program.fn_index.get(&sym).unwrap_or_else(|| panic!("{name} in fn_index")) as usize
        };
        let f_idx = find("f");
        let g_idx = find("g");
        let g_body = slice_function(&program, g_idx);
        let mut vm = Vm::new(&program);
        vm.install_warm_bytecode(f_idx, &g_body);
        vm.run().expect("runs");
        vm.output().to_string()
    });
    assert_eq!(routed.trim(), "20", "warm body for f must execute g's logic (n + n)");
}

#[test]
fn install_rejects_malformed_and_arity_mismatched_bodies() {
    with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let f_sym = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "f" => Some(*name),
                _ => None,
            })
            .expect("f defined");
        let fi = *program.fn_index.get(&f_sym).expect("f in fn_index") as usize;
        let mut vm = Vm::new(&program);

        // A body whose last op is not a terminator is not well-formed → rejected (it
        // would otherwise fetch past the warm buffer and panic). A bare back-edge: the
        // jump is in range, but there is no Return/ReturnNothing/Halt to leave the body.
        let mut no_terminator = slice_function(&program, fi);
        no_terminator.code = vec![Op::Jump { target: 0 }];
        assert!(
            !vm.install_warm_bytecode(fi, &no_terminator),
            "body without a terminal op must be refused"
        );

        // A well-formed body whose arity disagrees with the baseline is rejected.
        let mut wrong_arity = slice_function(&program, fi);
        wrong_arity.param_count += 1;
        assert!(
            !vm.install_warm_bytecode(fi, &wrong_arity),
            "arity-mismatched body must be refused"
        );

        // The genuine body installs.
        let good = slice_function(&program, fi);
        assert!(vm.install_warm_bytecode(fi, &good), "a valid body installs");

        // After the two refusals + one accept, the program still runs correctly.
        vm.run().expect("runs");
        assert_eq!(vm.output().trim(), "11");
    });
}
