mod common;
use common::{assert_exact_output, assert_interpreter_output};
use std::collections::HashSet;
use logicaffeine_base::{Interner, Symbol};
use logicaffeine_language::ast::Stmt;
use logicaffeine_compile::codegen::{codegen_stmt, RefinementContext, empty_var_caps};
use logicaffeine_compile::analysis::types::TypeEnv;
use logicaffeine_language::analysis::TypeRegistry;

fn empty_lww_fields() -> HashSet<(String, String)> {
    HashSet::new()
}
fn empty_mv_fields() -> HashSet<(String, String)> {
    HashSet::new()
}
fn empty_async_fns() -> HashSet<Symbol> {
    HashSet::new()
}
fn empty_pipe_vars() -> HashSet<Symbol> {
    HashSet::new()
}

// =============================================================================
// Codegen: Stmt::Break emits "break;\n"
// =============================================================================

#[test]
fn codegen_break_emits_break_statement() {
    let mut interner = Interner::new();
    let stmt = Stmt::Break;
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = TypeEnv::new();
    let result = codegen_stmt(
        &stmt,
        &interner,
        0,
        &HashSet::<Symbol>::new(),
        &mut ctx,
        &empty_lww_fields(),
        &empty_mv_fields(),
        &mut synced_vars,
        &empty_var_caps(),
        &empty_async_fns(),
        &empty_pipe_vars(),
        &HashSet::new(),
        &registry,
        &type_env,
    );
    assert_eq!(result, "break;\n");
}

#[test]
fn codegen_break_indented() {
    let mut interner = Interner::new();
    let stmt = Stmt::Break;
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = TypeEnv::new();
    let result = codegen_stmt(
        &stmt,
        &interner,
        1,
        &HashSet::<Symbol>::new(),
        &mut ctx,
        &empty_lww_fields(),
        &empty_mv_fields(),
        &mut synced_vars,
        &empty_var_caps(),
        &empty_async_fns(),
        &empty_pipe_vars(),
        &HashSet::new(),
        &registry,
        &type_env,
    );
    assert_eq!(result, "    break;\n");
}

// =============================================================================
// Parser: "Break." parses as Stmt::Break
// =============================================================================

#[test]
fn parser_break_in_while_loop() {
    use logicaffeine_compile::compile::compile_to_rust;
    let source = r#"## Main
Let mutable i be 0.
While i is less than 10:
    Set i to i + 1.
    Break.
Show i.
"#;
    let rust = compile_to_rust(source).expect("should compile");
    assert!(
        rust.contains("break;"),
        "Generated Rust should contain 'break;'.\nGot:\n{}",
        rust
    );
}

// =============================================================================
// E2E: break exits the loop immediately
// =============================================================================

#[test]
#[ignore = "e2e"]
fn e2e_break_exits_loop() {
    assert_exact_output(
        r#"## Main
Let mutable i be 0.
While i is less than 10:
    Set i to i + 1.
    Break.
Show i.
"#,
        "1",
    );
}

#[test]
#[ignore = "e2e"]
fn e2e_break_with_condition() {
    assert_exact_output(
        r#"## Main
Let mutable i be 0.
While i is less than 100:
    Set i to i + 1.
    If i equals 5:
        Break.
Show i.
"#,
        "5",
    );
}

// =============================================================================
// Interpreter: break works in while loops
// =============================================================================

#[test]
fn interpreter_break_exits_loop() {
    assert_interpreter_output(
        r#"## Main
Let mutable i be 0.
While i is less than 10:
    Set i to i + 1.
    Break.
Show i.
"#,
        "1",
    );
}

#[test]
fn interpreter_break_with_condition() {
    assert_interpreter_output(
        r#"## Main
Let mutable i be 0.
While i is less than 100:
    Set i to i + 1.
    If i equals 5:
        Break.
Show i.
"#,
        "5",
    );
}
