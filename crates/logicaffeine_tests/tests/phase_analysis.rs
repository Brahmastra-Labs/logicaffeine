use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::ast::{BinaryOpKind, Expr, Literal, OptFlag, Stmt};
use logicaffeine_language::ast::stmt::{ClosureBody, TypeExpr};
use std::collections::HashSet;
use logicaffeine_compile::analysis::callgraph::CallGraph;
use logicaffeine_compile::analysis::liveness::LivenessResult;
use logicaffeine_compile::analysis::readonly::ReadonlyParams;
use logicaffeine_compile::analysis::types::{TypeEnv, LogosType};
use logicaffeine_language::analysis::TypeRegistry;

// =============================================================================
// Helpers
// =============================================================================

fn make_registry(interner: &mut Interner) -> TypeRegistry {
    TypeRegistry::with_primitives(interner)
}

fn funcdef<'a>(name: Symbol, params: Vec<(Symbol, &'a TypeExpr<'a>)>, body: &'a [Stmt<'a>]) -> Stmt<'a> {
    Stmt::FunctionDef {
        name,
        generics: vec![],
        params,
        body,
        return_type: None,
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: HashSet::new(),
    }
}

fn native_funcdef<'a>(name: Symbol, params: Vec<(Symbol, &'a TypeExpr<'a>)>) -> Stmt<'a> {
    Stmt::FunctionDef {
        name,
        generics: vec![],
        params,
        body: &[],
        return_type: None,
        is_native: true,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: HashSet::new(),
    }
}

// =============================================================================
// Pass 1: CallGraph
// =============================================================================

#[test]
fn callgraph_direct_call() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();

    let foo = interner.intern("foo");
    let bar = interner.intern("bar");

    let call = arena.alloc(Stmt::Call { function: bar, args: vec![] });
    let foo_body = std::slice::from_ref(call);

    let stmts = vec![
        funcdef(foo, vec![], foo_body),
        funcdef(bar, vec![], &[]),
    ];

    let cg = CallGraph::build(&stmts, &interner);

    assert!(
        cg.edges.get(&foo).map(|s| s.contains(&bar)).unwrap_or(false),
        "foo should have a direct edge to bar"
    );
    assert!(
        !cg.edges.get(&foo).map(|s| s.contains(&foo)).unwrap_or(false),
        "foo should not have a self-edge"
    );
}

#[test]
fn callgraph_transitive() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();

    let foo = interner.intern("foo");
    let bar = interner.intern("bar");
    let baz = interner.intern("baz");

    let foo_call = arena.alloc(Stmt::Call { function: bar, args: vec![] });
    let bar_call = arena.alloc(Stmt::Call { function: baz, args: vec![] });

    let stmts = vec![
        funcdef(foo, vec![], std::slice::from_ref(foo_call)),
        funcdef(bar, vec![], std::slice::from_ref(bar_call)),
        funcdef(baz, vec![], &[]),
    ];

    let cg = CallGraph::build(&stmts, &interner);
    let reachable = cg.reachable_from(foo);

    assert!(reachable.contains(&bar), "foo should transitively reach bar");
    assert!(reachable.contains(&baz), "foo should transitively reach baz (via bar)");
    assert!(!reachable.contains(&foo), "foo should not reach itself (no recursion)");
}

#[test]
fn callgraph_recursive_detected() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let factorial = interner.intern("factorial");
    let n = interner.intern("n");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);

    let n_expr = expr_arena.alloc(Expr::Identifier(n));
    let self_call = arena.alloc(Stmt::Call { function: factorial, args: vec![n_expr] });

    let stmts = vec![funcdef(factorial, vec![(n, &int_ty)], std::slice::from_ref(self_call))];

    let cg = CallGraph::build(&stmts, &interner);
    assert!(cg.is_recursive(factorial), "factorial should be detected as self-recursive");
}

#[test]
fn callgraph_mutual_recursive() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();

    let is_even = interner.intern("isEven");
    let is_odd = interner.intern("isOdd");

    let even_calls_odd = arena.alloc(Stmt::Call { function: is_odd, args: vec![] });
    let odd_calls_even = arena.alloc(Stmt::Call { function: is_even, args: vec![] });

    let stmts = vec![
        funcdef(is_even, vec![], std::slice::from_ref(even_calls_odd)),
        funcdef(is_odd, vec![], std::slice::from_ref(odd_calls_even)),
    ];

    let cg = CallGraph::build(&stmts, &interner);

    assert!(cg.is_recursive(is_even), "isEven should be recursive (mutual with isOdd)");
    assert!(cg.is_recursive(is_odd), "isOdd should be recursive (mutual with isEven)");
}

#[test]
fn callgraph_native_marked() {
    let mut interner = Interner::new();
    let printf = interner.intern("printf");

    let stmts = vec![native_funcdef(printf, vec![])];

    let cg = CallGraph::build(&stmts, &interner);
    assert!(cg.native_fns.contains(&printf), "Native function should be in native_fns set");
}

#[test]
fn callgraph_closure_calls_counted() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let helper = interner.intern("helper");
    let x = interner.intern("x");
    let f = interner.intern("f");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);

    // foo body: Let f be (x: Int) -> helper(x)
    let x_expr = expr_arena.alloc(Expr::Identifier(x));
    let helper_call_expr = expr_arena.alloc(Expr::Call { function: helper, args: vec![x_expr] });
    let closure_expr = expr_arena.alloc(Expr::Closure {
        params: vec![(x, &int_ty)],
        body: ClosureBody::Expression(helper_call_expr),
        return_type: None,
    });
    let let_stmt = arena.alloc(Stmt::Let { var: f, ty: None, value: closure_expr, mutable: false });

    let stmts = vec![
        funcdef(foo, vec![], std::slice::from_ref(let_stmt)),
        funcdef(helper, vec![], &[]),
    ];

    let cg = CallGraph::build(&stmts, &interner);

    assert!(
        cg.edges.get(&foo).map(|s| s.contains(&helper)).unwrap_or(false),
        "foo should have an edge to helper via closure call expression"
    );
}

// =============================================================================
// Pass 1: ReadonlyParams
// =============================================================================

#[test]
fn readonly_pure_reader() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let arr = interner.intern("arr");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");
    let n = interner.intern("n");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };

    // Body: only reads arr — Let n be length of arr.
    let arr_expr = expr_arena.alloc(Expr::Identifier(arr));
    let len_expr = expr_arena.alloc(Expr::Length { collection: arr_expr });
    let let_stmt = arena.alloc(Stmt::Let { var: n, ty: None, value: len_expr, mutable: false });

    let stmts = vec![funcdef(foo, vec![(arr, &seq_ty)], std::slice::from_ref(let_stmt))];

    let registry = make_registry(&mut interner);
    let type_env = TypeEnv::infer_program(&stmts, &interner, &registry);
    let cg = CallGraph::build(&stmts, &interner);
    let readonly = ReadonlyParams::analyze(&stmts, &cg, &type_env);

    assert!(readonly.is_readonly(foo, arr), "arr is only read in foo — should be readonly");
}

#[test]
fn readonly_pusher_not_readonly() {
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let arr = interner.intern("arr");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };

    // Body: Push 1 to arr.
    let one = expr_arena.alloc(Expr::Literal(Literal::Number(1)));
    let arr_expr = expr_arena.alloc(Expr::Identifier(arr));
    let push_stmt = arena.alloc(Stmt::Push { value: one, collection: arr_expr });

    let stmts = vec![funcdef(foo, vec![(arr, &seq_ty)], std::slice::from_ref(push_stmt))];

    let registry = make_registry(&mut interner);
    let type_env = TypeEnv::infer_program(&stmts, &interner, &registry);
    let cg = CallGraph::build(&stmts, &interner);
    let readonly = ReadonlyParams::analyze(&stmts, &cg, &type_env);

    assert!(!readonly.is_readonly(foo, arr), "arr is pushed to in foo — should NOT be readonly");
}

#[test]
fn readonly_transitive_mutation() {
    // caller(arr) calls pusher(arr). pusher pushes to xs.
    // arr in caller is transitively mutated → NOT readonly.
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let caller = interner.intern("caller");
    let pusher = interner.intern("pusher");
    let arr = interner.intern("arr");
    let xs = interner.intern("xs");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let seq_ty2 = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };

    // caller body: Call pusher(arr)
    let arr_id = expr_arena.alloc(Expr::Identifier(arr));
    let call_pusher = arena.alloc(Stmt::Call { function: pusher, args: vec![arr_id] });

    // pusher body: Push 1 to xs.
    let one = expr_arena.alloc(Expr::Literal(Literal::Number(1)));
    let xs_id = expr_arena.alloc(Expr::Identifier(xs));
    let push_stmt = arena.alloc(Stmt::Push { value: one, collection: xs_id });

    let stmts = vec![
        funcdef(caller, vec![(arr, &seq_ty)], std::slice::from_ref(call_pusher)),
        funcdef(pusher, vec![(xs, &seq_ty2)], std::slice::from_ref(push_stmt)),
    ];

    let registry = make_registry(&mut interner);
    let type_env = TypeEnv::infer_program(&stmts, &interner, &registry);
    let cg = CallGraph::build(&stmts, &interner);
    let readonly = ReadonlyParams::analyze(&stmts, &cg, &type_env);

    assert!(!readonly.is_readonly(pusher, xs), "xs is directly pushed to — NOT readonly");
    assert!(
        !readonly.is_readonly(caller, arr),
        "arr is passed to pusher which mutates xs — NOT readonly (transitive)"
    );
}

#[test]
fn readonly_transitive_pure() {
    // caller(arr) calls reader(arr). reader only reads xs.
    // arr in caller is NOT transitively mutated → readonly.
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let caller = interner.intern("caller");
    let reader_fn = interner.intern("reader");
    let arr = interner.intern("arr");
    let xs = interner.intern("xs");
    let n = interner.intern("n");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let seq_ty2 = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };

    // caller body: Call reader(arr)
    let arr_id = expr_arena.alloc(Expr::Identifier(arr));
    let call_reader = arena.alloc(Stmt::Call { function: reader_fn, args: vec![arr_id] });

    // reader body: Let n be length of xs.
    let xs_id = expr_arena.alloc(Expr::Identifier(xs));
    let len_expr = expr_arena.alloc(Expr::Length { collection: xs_id });
    let let_n = arena.alloc(Stmt::Let { var: n, ty: None, value: len_expr, mutable: false });

    let stmts = vec![
        funcdef(caller, vec![(arr, &seq_ty)], std::slice::from_ref(call_reader)),
        funcdef(reader_fn, vec![(xs, &seq_ty2)], std::slice::from_ref(let_n)),
    ];

    let registry = make_registry(&mut interner);
    let type_env = TypeEnv::infer_program(&stmts, &interner, &registry);
    let cg = CallGraph::build(&stmts, &interner);
    let readonly = ReadonlyParams::analyze(&stmts, &cg, &type_env);

    assert!(readonly.is_readonly(reader_fn, xs), "xs is only read — readonly");
    assert!(
        readonly.is_readonly(caller, arr),
        "arr is passed to reader which is pure — readonly (transitive)"
    );
}

#[test]
fn readonly_fixed_point_convergence() {
    // f(arr) → g(arr), g pushes to xs.
    // With left-to-right processing, this requires multiple fixed-point iterations:
    //   Pass 1: f.arr seems ok (g.arr not yet marked), g.arr: pushed → NOT readonly.
    //   Pass 2: f.arr → g.xs not readonly → f.arr also NOT readonly.
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let f = interner.intern("f");
    let g = interner.intern("g");
    let arr = interner.intern("arr");
    let xs = interner.intern("xs");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let seq_ty2 = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };

    // f body: Call g(arr)
    let arr_id = expr_arena.alloc(Expr::Identifier(arr));
    let call_g = arena.alloc(Stmt::Call { function: g, args: vec![arr_id] });

    // g body: Push 1 to xs.
    let one = expr_arena.alloc(Expr::Literal(Literal::Number(1)));
    let xs_id = expr_arena.alloc(Expr::Identifier(xs));
    let push_stmt = arena.alloc(Stmt::Push { value: one, collection: xs_id });

    let stmts = vec![
        funcdef(f, vec![(arr, &seq_ty)], std::slice::from_ref(call_g)),
        funcdef(g, vec![(xs, &seq_ty2)], std::slice::from_ref(push_stmt)),
    ];

    let registry = make_registry(&mut interner);
    let type_env = TypeEnv::infer_program(&stmts, &interner, &registry);
    let cg = CallGraph::build(&stmts, &interner);
    let readonly = ReadonlyParams::analyze(&stmts, &cg, &type_env);

    assert!(!readonly.is_readonly(g, xs), "g.xs is directly pushed to");
    assert!(
        !readonly.is_readonly(f, arr),
        "f.arr is transitively mutated via g — requires fixed-point convergence"
    );
}

#[test]
fn readonly_closure_read_only() {
    // foo(arr: Seq<Int>) body contains a closure that reads arr (via length).
    // No push to arr anywhere → arr is readonly.
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let arr = interner.intern("arr");
    let x = interner.intern("x");
    let f = interner.intern("f");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let int_ty = TypeExpr::Primitive(int_sym);

    // Closure: (x: Int) -> length of arr
    let arr_id = expr_arena.alloc(Expr::Identifier(arr));
    let len_of_arr = expr_arena.alloc(Expr::Length { collection: arr_id });
    let closure = expr_arena.alloc(Expr::Closure {
        params: vec![(x, &int_ty)],
        body: ClosureBody::Expression(len_of_arr),
        return_type: None,
    });
    let let_f = arena.alloc(Stmt::Let { var: f, ty: None, value: closure, mutable: false });

    let stmts = vec![funcdef(foo, vec![(arr, &seq_ty)], std::slice::from_ref(let_f))];

    let registry = make_registry(&mut interner);
    let type_env = TypeEnv::infer_program(&stmts, &interner, &registry);
    let cg = CallGraph::build(&stmts, &interner);
    let readonly = ReadonlyParams::analyze(&stmts, &cg, &type_env);

    assert!(
        readonly.is_readonly(foo, arr),
        "arr is only read inside the closure body — should be readonly"
    );
}

#[test]
fn readonly_closure_mutates() {
    // foo(arr: Seq<Int>) body contains a closure that calls pusher(arr).
    // pusher mutates its xs param → arr in foo is NOT readonly (via closure call edge).
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let pusher = interner.intern("pusher");
    let arr = interner.intern("arr");
    let xs = interner.intern("xs");
    let x = interner.intern("x");
    let f = interner.intern("f");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let seq_ty2 = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let int_ty = TypeExpr::Primitive(int_sym);

    // Closure in foo: (x: Int) -> pusher(arr)
    let arr_id = expr_arena.alloc(Expr::Identifier(arr));
    let call_pusher_expr = expr_arena.alloc(Expr::Call { function: pusher, args: vec![arr_id] });
    let closure = expr_arena.alloc(Expr::Closure {
        params: vec![(x, &int_ty)],
        body: ClosureBody::Expression(call_pusher_expr),
        return_type: None,
    });
    let let_f = arena.alloc(Stmt::Let { var: f, ty: None, value: closure, mutable: false });

    // pusher body: Push 1 to xs.
    let one = expr_arena.alloc(Expr::Literal(Literal::Number(1)));
    let xs_id = expr_arena.alloc(Expr::Identifier(xs));
    let push_stmt = arena.alloc(Stmt::Push { value: one, collection: xs_id });

    let stmts = vec![
        funcdef(foo, vec![(arr, &seq_ty)], std::slice::from_ref(let_f)),
        funcdef(pusher, vec![(xs, &seq_ty2)], std::slice::from_ref(push_stmt)),
    ];

    let registry = make_registry(&mut interner);
    let type_env = TypeEnv::infer_program(&stmts, &interner, &registry);
    let cg = CallGraph::build(&stmts, &interner);
    let readonly = ReadonlyParams::analyze(&stmts, &cg, &type_env);

    assert!(!readonly.is_readonly(pusher, xs), "pusher.xs is directly mutated");
    assert!(
        !readonly.is_readonly(foo, arr),
        "foo.arr passed to pusher via closure call edge — NOT readonly"
    );
}

#[test]
fn readonly_native_trusted() {
    // native fn nativeReader(xs: Seq<Int>) — no body, trusted as non-mutating.
    // caller(arr: Seq<Int>) calls nativeReader(arr).
    // arr should be readonly (native function trusted not to mutate).
    let mut interner = Interner::new();
    let arena: Arena<Stmt> = Arena::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let caller = interner.intern("caller");
    let native_reader = interner.intern("nativeReader");
    let arr = interner.intern("arr");
    let xs = interner.intern("xs");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let seq_ty2 = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };

    // caller body: Call nativeReader(arr)
    let arr_id = expr_arena.alloc(Expr::Identifier(arr));
    let call_native = arena.alloc(Stmt::Call { function: native_reader, args: vec![arr_id] });

    let stmts = vec![
        funcdef(caller, vec![(arr, &seq_ty)], std::slice::from_ref(call_native)),
        native_funcdef(native_reader, vec![(xs, &seq_ty2)]),
    ];

    let registry = make_registry(&mut interner);
    let type_env = TypeEnv::infer_program(&stmts, &interner, &registry);
    let cg = CallGraph::build(&stmts, &interner);
    let readonly = ReadonlyParams::analyze(&stmts, &cg, &type_env);

    assert!(
        readonly.is_readonly(native_reader, xs),
        "native function params are trusted as readonly by default"
    );
    assert!(
        readonly.is_readonly(caller, arr),
        "arr passed to trusted native reader — should be readonly"
    );
}

// =============================================================================
// Pass 2: LivenessResult
// =============================================================================

#[test]
fn liveness_simple_sequential() {
    // fn foo(n: Int) -> Int:
    //   Let x be n.     // stmt 0: gen={n}, kill={x}
    //   Return x.       // stmt 1: gen={x}
    // live_after[0] = {x},  live_after[1] = {}
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let n = interner.intern("n");
    let x = interner.intern("x");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);

    let n_expr = expr_arena.alloc(Expr::Identifier(n));
    let x_expr = expr_arena.alloc(Expr::Identifier(x));

    let stmt0 = Stmt::Let { var: x, ty: None, value: n_expr, mutable: false };
    let stmt1 = Stmt::Return { value: Some(x_expr) };

    let body = [stmt0, stmt1];
    let stmts = vec![funcdef(foo, vec![(n, &int_ty)], &body)];

    let result = LivenessResult::analyze(&stmts);

    assert!(result.is_live_after(foo, 0, x), "x should be live after stmt 0 (used in Return)");
    assert!(!result.is_live_after(foo, 0, n), "n should NOT be live after stmt 0");
    assert!(!result.is_live_after(foo, 1, x), "Nothing live after Return");
    assert!(!result.is_live_after(foo, 1, n), "Nothing live after Return");
}

#[test]
fn liveness_reassignment_kills() {
    // fn foo() -> Int:
    //   Let a be 1.    // stmt 0: gen={}, kill={a}
    //   Set a to 2.    // stmt 1: gen={}, kill={a}
    //   Return a.      // stmt 2: gen={a}
    // live_after[0] = {} (a killed in stmt 1 before use)
    // live_after[1] = {a}
    // live_after[2] = {}
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let a = interner.intern("a");

    let one_expr = expr_arena.alloc(Expr::Literal(Literal::Number(1)));
    let two_expr = expr_arena.alloc(Expr::Literal(Literal::Number(2)));
    let a_expr = expr_arena.alloc(Expr::Identifier(a));

    let stmt0 = Stmt::Let { var: a, ty: None, value: one_expr, mutable: false };
    let stmt1 = Stmt::Set { target: a, value: two_expr };
    let stmt2 = Stmt::Return { value: Some(a_expr) };

    let body = [stmt0, stmt1, stmt2];
    let stmts = vec![funcdef(foo, vec![], &body)];

    let result = LivenessResult::analyze(&stmts);

    assert!(!result.is_live_after(foo, 0, a), "a NOT live after stmt 0: killed by stmt 1 before read");
    assert!(result.is_live_after(foo, 1, a), "a IS live after stmt 1: used in Return");
    assert!(!result.is_live_after(foo, 2, a), "Nothing live after Return");
}

#[test]
fn liveness_branch_union() {
    // fn foo(x: Int, y: Int):
    //   Let z be 1.                                           // stmt 0
    //   If z > 0 then [Call useX(x)] else [Call useY(y)]    // stmt 1
    // live_after[0] = {z, x, y}  (all needed by stmt 1)
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();

    let foo = interner.intern("foo");
    let x = interner.intern("x");
    let y = interner.intern("y");
    let z = interner.intern("z");
    let use_x = interner.intern("useX");
    let use_y = interner.intern("useY");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);
    let int_ty2 = TypeExpr::Primitive(int_sym);

    let z_expr = expr_arena.alloc(Expr::Identifier(z));
    let x_expr = expr_arena.alloc(Expr::Identifier(x));
    let y_expr = expr_arena.alloc(Expr::Identifier(y));
    let zero_expr = expr_arena.alloc(Expr::Literal(Literal::Number(0)));
    let one_expr = expr_arena.alloc(Expr::Literal(Literal::Number(1)));

    let cond_expr = expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Gt,
        left: z_expr,
        right: zero_expr,
    });

    let call_x = stmt_arena.alloc(Stmt::Call { function: use_x, args: vec![x_expr] });
    let call_y = stmt_arena.alloc(Stmt::Call { function: use_y, args: vec![y_expr] });

    let stmt0 = Stmt::Let { var: z, ty: None, value: one_expr, mutable: false };
    let stmt1 = Stmt::If {
        cond: cond_expr,
        then_block: std::slice::from_ref(call_x),
        else_block: Some(std::slice::from_ref(call_y)),
    };

    let body = [stmt0, stmt1];
    let stmts = vec![funcdef(foo, vec![(x, &int_ty), (y, &int_ty2)], &body)];

    let result = LivenessResult::analyze(&stmts);

    assert!(result.is_live_after(foo, 0, x), "x should be live after stmt 0 (used in then-branch)");
    assert!(result.is_live_after(foo, 0, y), "y should be live after stmt 0 (used in else-branch)");
    assert!(result.is_live_after(foo, 0, z), "z should be live after stmt 0 (used in condition)");
}

#[test]
fn liveness_while_loop_fixed_point() {
    // fn foo():
    //   Let i be 0.         // stmt 0
    //   While i < 10:       // stmt 1
    //     Set i to i + 1.
    // live_after[0] should contain i (needed in While condition)
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();

    let foo = interner.intern("foo");
    let i = interner.intern("i");

    let zero_expr = expr_arena.alloc(Expr::Literal(Literal::Number(0)));
    let ten_expr = expr_arena.alloc(Expr::Literal(Literal::Number(10)));
    let one_expr = expr_arena.alloc(Expr::Literal(Literal::Number(1)));
    let i_cond = expr_arena.alloc(Expr::Identifier(i));
    let i_rhs = expr_arena.alloc(Expr::Identifier(i));

    let cond_expr = expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: i_cond,
        right: ten_expr,
    });
    let incr_expr = expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: i_rhs,
        right: one_expr,
    });

    let set_i = stmt_arena.alloc(Stmt::Set { target: i, value: incr_expr });

    let stmt0 = Stmt::Let { var: i, ty: None, value: zero_expr, mutable: false };
    let stmt1 = Stmt::While {
        cond: cond_expr,
        body: std::slice::from_ref(set_i),
        decreasing: None,
    };

    let body = [stmt0, stmt1];
    let stmts = vec![funcdef(foo, vec![], &body)];

    let result = LivenessResult::analyze(&stmts);

    assert!(result.is_live_after(foo, 0, i), "i should be live after stmt 0 (needed in While condition)");
}

#[test]
fn liveness_return_terminates() {
    // fn foo(x: Int, y: Int):
    //   Return x.           // stmt 0 — terminates
    //   Call doSomething(y). // stmt 1 — dead code
    // live_after[0] = {}  (Return is a terminator)
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let x = interner.intern("x");
    let y = interner.intern("y");
    let do_something = interner.intern("doSomething");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);
    let int_ty2 = TypeExpr::Primitive(int_sym);

    let x_expr = expr_arena.alloc(Expr::Identifier(x));
    let y_expr = expr_arena.alloc(Expr::Identifier(y));

    let stmt0 = Stmt::Return { value: Some(x_expr) };
    let stmt1 = Stmt::Call { function: do_something, args: vec![y_expr] };

    let body = [stmt0, stmt1];
    let stmts = vec![funcdef(foo, vec![(x, &int_ty), (y, &int_ty2)], &body)];

    let result = LivenessResult::analyze(&stmts);

    assert!(!result.is_live_after(foo, 0, x), "Nothing live after Return — x not live");
    assert!(!result.is_live_after(foo, 0, y), "Nothing live after Return — dead code y not live");
}

#[test]
fn liveness_last_use_detected() {
    // fn foo(arr: Seq<Int>, n: Int) -> Int:
    //   Set result to count(arr, n).   // stmt 0: gen={arr, n}, kill={result}
    //   Return result.                  // stmt 1: gen={result}
    // live_after[0] = {result}  — arr is NOT live after stmt 0
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let arr = interner.intern("arr");
    let n = interner.intern("n");
    let result = interner.intern("result");
    let count = interner.intern("count");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let int_ty = TypeExpr::Primitive(int_sym);

    let arr_expr = expr_arena.alloc(Expr::Identifier(arr));
    let n_expr = expr_arena.alloc(Expr::Identifier(n));
    let result_expr = expr_arena.alloc(Expr::Identifier(result));

    let call_expr = expr_arena.alloc(Expr::Call {
        function: count,
        args: vec![arr_expr, n_expr],
    });

    let stmt0 = Stmt::Set { target: result, value: call_expr };
    let stmt1 = Stmt::Return { value: Some(result_expr) };

    let body = [stmt0, stmt1];
    let stmts = vec![funcdef(foo, vec![(arr, &seq_ty), (n, &int_ty)], &body)];

    let result_liveness = LivenessResult::analyze(&stmts);

    assert!(!result_liveness.is_live_after(foo, 0, arr), "arr NOT live after stmt 0 — last use");
    assert!(!result_liveness.is_live_after(foo, 0, n), "n NOT live after stmt 0 — last use");
    assert!(result_liveness.is_live_after(foo, 0, result), "result IS live after stmt 0 (used in Return)");
    assert!(!result_liveness.is_live_after(foo, 1, result), "Nothing live after Return");
}

#[test]
fn liveness_not_last_use() {
    // fn foo(arr: Seq<Int>, n: Int) -> Int:
    //   Set result to count(arr, n).  // stmt 0: arr used, then ...
    //   Call print(arr).              // stmt 1: arr used AGAIN
    //   Return result.               // stmt 2
    // live_after[0] = {arr, result}  — arr IS live (used in stmt 1)
    let mut interner = Interner::new();
    let expr_arena: Arena<Expr> = Arena::new();

    let foo = interner.intern("foo");
    let arr = interner.intern("arr");
    let n = interner.intern("n");
    let result = interner.intern("result");
    let count = interner.intern("count");
    let print = interner.intern("print");
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");

    let inner_ty = TypeExpr::Primitive(int_sym);
    let seq_ty = TypeExpr::Generic { base: seq_sym, params: std::slice::from_ref(&inner_ty) };
    let int_ty = TypeExpr::Primitive(int_sym);

    let arr0 = expr_arena.alloc(Expr::Identifier(arr));
    let n0 = expr_arena.alloc(Expr::Identifier(n));
    let arr1 = expr_arena.alloc(Expr::Identifier(arr));
    let result_expr = expr_arena.alloc(Expr::Identifier(result));

    let call_count = expr_arena.alloc(Expr::Call {
        function: count,
        args: vec![arr0, n0],
    });

    let stmt0 = Stmt::Set { target: result, value: call_count };
    let stmt1 = Stmt::Call { function: print, args: vec![arr1] };
    let stmt2 = Stmt::Return { value: Some(result_expr) };

    let body = [stmt0, stmt1, stmt2];
    let stmts = vec![funcdef(foo, vec![(arr, &seq_ty), (n, &int_ty)], &body)];

    let result_liveness = LivenessResult::analyze(&stmts);

    assert!(result_liveness.is_live_after(foo, 0, arr), "arr IS live after stmt 0 (used in print at stmt 1)");
    assert!(result_liveness.is_live_after(foo, 0, result), "result IS live after stmt 0 (used in Return)");
    assert!(!result_liveness.is_live_after(foo, 0, n), "n NOT live after stmt 0");
}
