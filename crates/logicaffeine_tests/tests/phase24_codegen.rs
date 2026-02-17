use std::collections::HashSet;
use logicaffeine_base::Arena;
use logicaffeine_language::ast::{Expr, Literal, Stmt, BinaryOpKind};
use logicaffeine_language::ast::stmt::SelectBranch;
use logicaffeine_compile::codegen::{
    codegen_expr, codegen_stmt, codegen_program, RefinementContext, empty_var_caps,
    collect_async_functions, collect_pipe_vars, collect_pipe_sender_params,
};
use logicaffeine_base::{Interner, Symbol};
use logicaffeine_language::analysis::{TypeRegistry, PolicyRegistry};

// Empty LWW fields set for tests that don't involve CRDTs
fn empty_lww_fields() -> HashSet<(String, String)> {
    HashSet::new()
}

// Empty MV fields set for tests that don't involve CRDTs
fn empty_mv_fields() -> HashSet<(String, String)> {
    HashSet::new()
}

// Empty async functions set for tests that don't involve concurrency
fn empty_async_fns() -> HashSet<Symbol> {
    HashSet::new()
}

// Empty pipe vars set for tests that don't involve concurrency
fn empty_pipe_vars() -> HashSet<Symbol> {
    HashSet::new()
}

// Empty type registry for tests
fn empty_registry(interner: &mut Interner) -> TypeRegistry {
    TypeRegistry::with_primitives(interner)
}

#[test]
fn codegen_module_exists() {
    let _ = codegen_expr;
    let _ = codegen_stmt;
    let _ = codegen_program;
}

#[test]
fn codegen_literal_number() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let expr = Expr::Literal(Literal::Number(42));
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "42");
}

#[test]
fn codegen_literal_boolean_true() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let expr = Expr::Literal(Literal::Boolean(true));
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "true");
}

#[test]
fn codegen_literal_boolean_false() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let expr = Expr::Literal(Literal::Boolean(false));
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "false");
}

#[test]
fn codegen_literal_text() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let text_sym = interner.intern("hello world");
    let expr = Expr::Literal(Literal::Text(text_sym));
    let result = codegen_expr(&expr, &interner, &synced_vars);
    // String::from() ensures we get String type, not &str
    assert_eq!(result, "String::from(\"hello world\")");
}

#[test]
fn codegen_literal_nothing() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let expr = Expr::Literal(Literal::Nothing);
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "()");
}

#[test]
fn codegen_identifier() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let var_sym = interner.intern("x");
    let expr = Expr::Identifier(var_sym);
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "x");
}

#[test]
fn codegen_binary_add() {
    let interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let arena: Arena<Expr> = Arena::new();
    let left = arena.alloc(Expr::Literal(Literal::Number(1)));
    let right = arena.alloc(Expr::Literal(Literal::Number(2)));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    };
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "(1 + 2)");
}

#[test]
fn codegen_binary_eq() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let left = arena.alloc(Expr::Identifier(x));
    let right = arena.alloc(Expr::Literal(Literal::Number(5)));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Eq,
        left,
        right,
    };
    let result = codegen_expr(&expr, &interner, &synced_vars);
    assert_eq!(result, "(x == 5)");
}

#[test]
fn codegen_index_1_indexed() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let list = interner.intern("list");
    let arena: Arena<Expr> = Arena::new();
    let collection = arena.alloc(Expr::Identifier(list));
    // Phase 43D: Index now takes an expression
    let index = arena.alloc(Expr::Literal(Literal::Number(1)));
    let expr = Expr::Index {
        collection,
        index,
    };
    let result = codegen_expr(&expr, &interner, &synced_vars);
    // Phase 57: Uses LogosIndex trait for polymorphic indexing
    assert_eq!(result, "LogosIndex::logos_get(&list, 1)");
}

#[test]
fn codegen_index_5_becomes_4() {
    let mut interner = Interner::new();
    let synced_vars = HashSet::<Symbol>::new();
    let items = interner.intern("items");
    let arena: Arena<Expr> = Arena::new();
    let collection = arena.alloc(Expr::Identifier(items));
    // Phase 43D: Index now takes an expression
    let index = arena.alloc(Expr::Literal(Literal::Number(5)));
    let expr = Expr::Index {
        collection,
        index,
    };
    let result = codegen_expr(&expr, &interner, &synced_vars);
    // Phase 57: Uses LogosIndex trait for polymorphic indexing
    assert_eq!(result, "LogosIndex::logos_get(&items, 5)");
}

#[test]
fn codegen_let_statement() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(42)));
    let stmt = Stmt::Let {
        var: x,
        ty: None,
        value,
        mutable: false,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert_eq!(result, "let x = 42;\n");
}

#[test]
fn codegen_let_mutable() {
    let mut interner = Interner::new();
    let count = interner.intern("count");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(0)));
    let stmt = Stmt::Let {
        var: count,
        ty: None,
        value,
        mutable: true,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert_eq!(result, "let mut count = 0;\n");
}

#[test]
fn codegen_set_statement() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(10)));
    let stmt = Stmt::Set {
        target: x,
        value,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert_eq!(result, "x = 10;\n");
}

#[test]
fn codegen_return_with_value() {
    let mut interner = Interner::new();
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(42)));
    let stmt = Stmt::Return {
        value: Some(value),
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert_eq!(result, "return 42;\n");
}

#[test]
fn codegen_return_without_value() {
    let mut interner = Interner::new();
    let stmt = Stmt::Return { value: None };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert_eq!(result, "return;\n");
}

#[test]
fn codegen_if_without_else() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();

    let cond = arena.alloc(Expr::Identifier(x));

    let stmt = Stmt::If {
        cond,
        then_block: &[],
        else_block: None,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert!(result.contains("if x {"), "Expected 'if x {{' but got: {}", result);
    assert!(result.contains("}"), "Expected '}}' but got: {}", result);
}

#[test]
fn codegen_while_loop() {
    let mut interner = Interner::new();
    let running = interner.intern("running");
    let arena: Arena<Expr> = Arena::new();

    let cond = arena.alloc(Expr::Identifier(running));

    let stmt = Stmt::While {
        cond,
        body: &[],
        decreasing: None,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert!(result.contains("while running {"), "Expected 'while running {{' but got: {}", result);
    assert!(result.contains("}"), "Expected '}}' but got: {}", result);
}

#[test]
fn codegen_indentation() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let arena: Arena<Expr> = Arena::new();
    let value = arena.alloc(Expr::Literal(Literal::Number(5)));
    let stmt = Stmt::Let {
        var: x,
        ty: None,
        value,
        mutable: false,
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 1, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert_eq!(result, "    let x = 5;\n");
}

#[test]
fn codegen_program_wraps_in_main() {
    let mut interner = Interner::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let policies = PolicyRegistry::new();
    let stmts: &[Stmt] = &[];
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::infer_program(stmts, &interner, &registry);
    let result = codegen_program(stmts, &registry, &policies, &interner, &type_env);
    assert!(result.contains("fn main()"), "Expected 'fn main()' but got: {}", result);
    assert!(result.contains("{"), "Expected '{{' but got: {}", result);
    assert!(result.contains("}"), "Expected '}}' but got: {}", result);
}

#[test]
fn codegen_call_statement() {
    let mut interner = Interner::new();
    let println = interner.intern("println");

    let stmt = Stmt::Call {
        function: println,
        args: vec![],
    };
    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();
    let result = codegen_stmt(&stmt, &interner, 0, &HashSet::<Symbol>::new(), &mut ctx, &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars, &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(), &HashSet::new(), &registry, &type_env);
    assert_eq!(result, "println();\n");
}

// =============================================================================
// Phase 54: Async/Concurrency Unit Tests
// =============================================================================

#[test]
fn test_collect_async_functions_with_sleep() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    // Create a function with Sleep - it should be detected as async
    let sleeper = interner.intern("sleeper");
    let ms = expr_arena.alloc(Expr::Literal(Literal::Number(100)));
    let sleep_stmt = arena.alloc(Stmt::Sleep { milliseconds: ms });
    let body: &[Stmt] = std::slice::from_ref(sleep_stmt);

    let func_def = Stmt::FunctionDef {
        name: sleeper,
        params: vec![],
        body,
        return_type: None,
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
    };

    let stmts = vec![func_def];
    let async_fns = collect_async_functions(&stmts);

    assert!(async_fns.contains(&sleeper), "Function with Sleep should be detected as async");
}

#[test]
fn test_collect_async_functions_with_launch_task() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();

    // Create a function with LaunchTask - it should be detected as async
    let launcher = interner.intern("launcher");
    let worker = interner.intern("worker");
    let launch_stmt = arena.alloc(Stmt::LaunchTask {
        function: worker,
        args: vec![],
    });
    let body: &[Stmt] = std::slice::from_ref(launch_stmt);

    let func_def = Stmt::FunctionDef {
        name: launcher,
        params: vec![],
        body,
        return_type: None,
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
    };

    let stmts = vec![func_def];
    let async_fns = collect_async_functions(&stmts);

    assert!(async_fns.contains(&launcher), "Function with LaunchTask should be detected as async");
}

#[test]
fn test_collect_async_functions_transitive() {
    // Bug 2: A function that calls an async function should also be detected as async
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    // Create helper function with Sleep (directly async)
    let helper = interner.intern("helper");
    let ms = expr_arena.alloc(Expr::Literal(Literal::Number(50)));
    let sleep_stmt = arena.alloc(Stmt::Sleep { milliseconds: ms });
    let helper_body: &[Stmt] = std::slice::from_ref(sleep_stmt);
    let helper_def = Stmt::FunctionDef {
        name: helper,
        params: vec![],
        body: helper_body,
        return_type: None,
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
    };

    // Create wrapper function that calls helper (should be transitively async)
    let wrapper = interner.intern("wrapper");
    let call_stmt = arena.alloc(Stmt::Call {
        function: helper,
        args: vec![],
    });
    let wrapper_body: &[Stmt] = std::slice::from_ref(call_stmt);
    let wrapper_def = Stmt::FunctionDef {
        name: wrapper,
        params: vec![],
        body: wrapper_body,
        return_type: None,
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
    };

    let stmts = vec![helper_def, wrapper_def];
    let async_fns = collect_async_functions(&stmts);

    assert!(async_fns.contains(&helper), "helper should be detected as async (has Sleep)");
    assert!(async_fns.contains(&wrapper), "wrapper should be detected as async (calls async helper)");
}

#[test]
fn test_collect_pipe_vars_basic() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();

    let pipe_var = interner.intern("jobs");
    let int_type = interner.intern("Int");

    let create_stmt = arena.alloc(Stmt::CreatePipe {
        var: pipe_var,
        element_type: int_type,
        capacity: None,
    });

    let stmts: &[Stmt] = std::slice::from_ref(create_stmt);
    let pipe_vars = collect_pipe_vars(stmts);

    assert!(pipe_vars.contains(&pipe_var), "CreatePipe variable should be collected");
}

#[test]
fn test_collect_pipe_vars_in_concurrent_block() {
    // Bug 5 (related): pipe_vars should be collected from within Concurrent blocks
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();

    let pipe_var = interner.intern("data");
    let int_type = interner.intern("Int");

    let create_stmt = arena.alloc(Stmt::CreatePipe {
        var: pipe_var,
        element_type: int_type,
        capacity: None,
    });
    let tasks: &[Stmt] = std::slice::from_ref(create_stmt);

    let concurrent = Stmt::Concurrent { tasks };
    let stmts = vec![concurrent];
    let pipe_vars = collect_pipe_vars(&stmts);

    assert!(pipe_vars.contains(&pipe_var), "CreatePipe inside Concurrent should be collected");
}

#[test]
fn test_collect_pipe_sender_params_in_concurrent() {
    // Bug 5: collect_pipe_sender_params_stmt doesn't recurse into Concurrent
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let pipe_param = interner.intern("output");
    let value = expr_arena.alloc(Expr::Literal(Literal::Number(42)));
    let pipe_expr = expr_arena.alloc(Expr::Identifier(pipe_param));

    let send_stmt = arena.alloc(Stmt::SendPipe {
        value,
        pipe: pipe_expr,
    });
    let tasks: &[Stmt] = std::slice::from_ref(send_stmt);

    let concurrent = arena.alloc(Stmt::Concurrent { tasks });
    let body: &[Stmt] = std::slice::from_ref(concurrent);

    let senders = collect_pipe_sender_params(body);

    assert!(senders.contains(&pipe_param), "SendPipe inside Concurrent should detect sender param");
}

#[test]
fn codegen_stmt_call_async_function_awaits() {
    // Bug 1: Stmt::Call doesn't check async_functions for .await
    let mut interner = Interner::new();

    let async_fn = interner.intern("async_helper");
    let stmt = Stmt::Call {
        function: async_fn,
        args: vec![],
    };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    // Mark async_helper as an async function
    let mut async_functions = HashSet::new();
    async_functions.insert(async_fn);

    let result = codegen_stmt(
        &stmt, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &async_functions, &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    assert!(result.contains(".await"), "Call to async function should have .await: {}", result);
    assert_eq!(result, "async_helper().await;\n");
}

#[test]
fn codegen_stmt_call_sync_function_no_await() {
    // Sync functions should NOT get .await
    let mut interner = Interner::new();

    let sync_fn = interner.intern("sync_helper");
    let stmt = Stmt::Call {
        function: sync_fn,
        args: vec![],
    };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    // empty async_functions means sync_helper is NOT async
    let async_functions = HashSet::new();

    let result = codegen_stmt(
        &stmt, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &async_functions, &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    assert!(!result.contains(".await"), "Call to sync function should NOT have .await: {}", result);
    assert_eq!(result, "sync_helper();\n");
}

#[test]
fn codegen_concurrent_with_async_call() {
    // Concurrent block calling async function should have .await
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();

    let async_fn = interner.intern("fetch_data");
    let call_stmt = arena.alloc(Stmt::Call {
        function: async_fn,
        args: vec![],
    });
    let tasks: &[Stmt] = std::slice::from_ref(call_stmt);
    let concurrent = Stmt::Concurrent { tasks };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    let mut async_functions = HashSet::new();
    async_functions.insert(async_fn);

    let result = codegen_stmt(
        &concurrent, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &async_functions, &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    assert!(result.contains("fetch_data().await"), "Async call in Concurrent should have .await: {}", result);
}

#[test]
fn codegen_concurrent_with_sync_call() {
    // Bug 3: Concurrent block calling SYNC function should NOT have .await
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();

    let sync_fn = interner.intern("compute");
    let call_stmt = arena.alloc(Stmt::Call {
        function: sync_fn,
        args: vec![],
    });
    let tasks: &[Stmt] = std::slice::from_ref(call_stmt);
    let concurrent = Stmt::Concurrent { tasks };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    // Empty async_functions = sync_fn is NOT async
    let async_functions = HashSet::new();

    let result = codegen_stmt(
        &concurrent, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &async_functions, &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    // The sync function should NOT have .await
    assert!(!result.contains("compute().await"), "Sync call in Concurrent should NOT have .await: {}", result);
}

#[test]
fn codegen_select_receive_with_local_pipe() {
    // Local pipe (created with CreatePipe) should use _rx suffix
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let pipe_var = interner.intern("jobs");
    let msg_var = interner.intern("msg");
    let pipe_expr = expr_arena.alloc(Expr::Identifier(pipe_var));

    let select = Stmt::Select {
        branches: vec![
            SelectBranch::Receive {
                var: msg_var,
                pipe: pipe_expr,
                body: &[],
            },
        ],
    };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    // Mark jobs as a local pipe
    let mut pipe_vars = HashSet::new();
    pipe_vars.insert(pipe_var);

    let result = codegen_stmt(
        &select, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &empty_async_fns(), &pipe_vars,
        &HashSet::new(), &registry, &type_env,
    );

    assert!(result.contains("jobs_rx.recv()"), "Local pipe should use _rx suffix: {}", result);
}

#[test]
fn codegen_select_receive_with_pipe_param() {
    // Bug 4: Pipe parameter (not local) should NOT use _rx suffix
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let pipe_param = interner.intern("input_pipe");
    let msg_var = interner.intern("msg");
    let pipe_expr = expr_arena.alloc(Expr::Identifier(pipe_param));

    let select = Stmt::Select {
        branches: vec![
            SelectBranch::Receive {
                var: msg_var,
                pipe: pipe_expr,
                body: &[],
            },
        ],
    };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    // Empty pipe_vars means input_pipe is NOT a local pipe (it's a parameter)
    let pipe_vars = HashSet::new();

    let result = codegen_stmt(
        &select, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &empty_async_fns(), &pipe_vars,
        &HashSet::new(), &registry, &type_env,
    );

    // Should use input_pipe.recv() NOT input_pipe_rx.recv()
    assert!(result.contains("input_pipe.recv()"), "Pipe parameter should NOT use _rx suffix: {}", result);
    assert!(!result.contains("input_pipe_rx"), "Pipe parameter should NOT have _rx: {}", result);
}

#[test]
fn codegen_sleep_statement() {
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let ms = expr_arena.alloc(Expr::Literal(Literal::Number(100)));
    let stmt = Stmt::Sleep { milliseconds: ms };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    let result = codegen_stmt(
        &stmt, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    assert!(result.contains("tokio::time::sleep"), "Sleep should use tokio: {}", result);
    assert!(result.contains(".await"), "Sleep should have .await: {}", result);
}

#[test]
fn codegen_create_pipe() {
    let mut interner = Interner::new();

    let pipe_var = interner.intern("jobs");
    let int_type = interner.intern("Int");

    let stmt = Stmt::CreatePipe {
        var: pipe_var,
        element_type: int_type,
        capacity: None,
    };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    let result = codegen_stmt(
        &stmt, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    assert!(result.contains("jobs_tx"), "CreatePipe should create _tx var: {}", result);
    assert!(result.contains("jobs_rx"), "CreatePipe should create _rx var: {}", result);
    assert!(result.contains("mpsc::channel"), "CreatePipe should use mpsc: {}", result);
}

#[test]
fn codegen_launch_task() {
    let mut interner = Interner::new();

    let worker_fn = interner.intern("worker");
    let stmt = Stmt::LaunchTask {
        function: worker_fn,
        args: vec![],
    };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    let result = codegen_stmt(
        &stmt, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &empty_async_fns(), &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    assert!(result.contains("tokio::spawn"), "LaunchTask should use tokio::spawn: {}", result);
    assert!(result.contains("worker()"), "LaunchTask should call the function: {}", result);
}

#[test]
fn codegen_let_with_async_call_awaits() {
    // Bug 6: Let with Expr::Call to async function should have .await
    let mut interner = Interner::new();
    let arena: Arena<Expr> = Arena::new();

    let async_fn = interner.intern("fetch_data");
    let x = interner.intern("x");
    let call_expr = arena.alloc(Expr::Call {
        function: async_fn,
        args: vec![],
    });

    let stmt = Stmt::Let {
        var: x,
        ty: None,
        value: call_expr,
        mutable: false,
    };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    // Mark fetch_data as an async function
    let mut async_functions = HashSet::new();
    async_functions.insert(async_fn);

    let result = codegen_stmt(
        &stmt, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &async_functions, &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    assert!(result.contains(".await"), "Let with async call should have .await: {}", result);
    assert_eq!(result, "let x = fetch_data().await;\n");
}

#[test]
fn codegen_let_with_sync_call_no_await() {
    // Sync function calls in Let should NOT have .await
    let mut interner = Interner::new();
    let arena: Arena<Expr> = Arena::new();

    let sync_fn = interner.intern("compute");
    let x = interner.intern("x");
    let call_expr = arena.alloc(Expr::Call {
        function: sync_fn,
        args: vec![],
    });

    let stmt = Stmt::Let {
        var: x,
        ty: None,
        value: call_expr,
        mutable: false,
    };

    let mut ctx = RefinementContext::new();
    let mut synced_vars = HashSet::<Symbol>::new();
    let registry = TypeRegistry::with_primitives(&mut interner);
    let type_env = logicaffeine_compile::analysis::types::TypeEnv::new();

    // Empty async_functions = sync_fn is NOT async
    let async_functions = HashSet::new();

    let result = codegen_stmt(
        &stmt, &interner, 0, &HashSet::new(), &mut ctx,
        &empty_lww_fields(), &empty_mv_fields(), &mut synced_vars,
        &empty_var_caps(), &async_functions, &empty_pipe_vars(),
        &HashSet::new(), &registry, &type_env,
    );

    assert!(!result.contains(".await"), "Let with sync call should NOT have .await: {}", result);
    assert_eq!(result, "let x = compute();\n");
}
