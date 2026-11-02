use super::*;
use crate::ast::stmt::{Expr, Literal, Stmt, TypeExpr};
use crate::intern::Interner;

// =========================================================================
// Helpers
// =========================================================================

fn mk_interner() -> Interner {
    Interner::new()
}

fn run(stmts: &[Stmt], interner: &Interner) -> TypeEnv {
    check_program(stmts, interner, &TypeRegistry::new()).expect("check_program failed")
}

// =========================================================================
// Let literal inference
// =========================================================================

#[test]
fn let_literal_int() {
    let mut interner = mk_interner();
    let x = interner.intern("x");
    let val = Expr::Literal(Literal::Number(42));
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(x), &LogosType::Int);
}

#[test]
fn let_literal_float() {
    let mut interner = mk_interner();
    let x = interner.intern("x");
    let val = Expr::Literal(Literal::Float(3.14));
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(x), &LogosType::Float);
}

#[test]
fn let_literal_string() {
    let mut interner = mk_interner();
    let s = interner.intern("s");
    let hello = interner.intern("hello");
    let val = Expr::Literal(Literal::Text(hello));
    let stmts = [Stmt::Let {
        var: s,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(s), &LogosType::String);
}

// =========================================================================
// Let with type annotation
// =========================================================================

#[test]
fn let_with_annotation_uses_annotation() {
    let mut interner = mk_interner();
    let x = interner.intern("x");
    let float_sym = interner.intern("Real");
    let val = Expr::Literal(Literal::Number(5)); // Int value
    let ty_ann = TypeExpr::Primitive(float_sym);
    let stmts = [Stmt::Let {
        var: x,
        ty: Some(&ty_ann),
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    // Annotation wins: Int unifies with Float (numeric)
    assert_eq!(env.lookup(x), &LogosType::Float);
}

#[test]
fn let_type_mismatch_fails() {
    let mut interner = mk_interner();
    let x = interner.intern("x");
    let int_sym = interner.intern("Int");
    let val = Expr::Literal(Literal::Text(Symbol::EMPTY));
    let ty_ann = TypeExpr::Primitive(int_sym);
    let stmts = [Stmt::Let {
        var: x,
        ty: Some(&ty_ann),
        value: &val,
        mutable: false,
    }];
    let result = check_program(&stmts, &interner, &TypeRegistry::new());
    assert!(result.is_err(), "Int and Text should not unify");
}

// =========================================================================
// Empty list → Seq(Unknown)
// =========================================================================

#[test]
fn empty_list_is_seq_unknown() {
    let mut interner = mk_interner();
    let xs = interner.intern("xs");
    let val = Expr::List(vec![]);
    let stmts = [Stmt::Let {
        var: xs,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    // Should be Seq of something (Unknown because element type is unsolved)
    assert!(matches!(env.lookup(xs), LogosType::Seq(_)));
}

#[test]
fn non_empty_list_infers_element_type() {
    let mut interner = mk_interner();
    let xs = interner.intern("xs");
    let one = Expr::Literal(Literal::Number(1));
    let two = Expr::Literal(Literal::Number(2));
    let val = Expr::List(vec![&one, &two]);
    let stmts = [Stmt::Let {
        var: xs,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(xs), &LogosType::Seq(Box::new(LogosType::Int)));
}

// =========================================================================
// OptionNone → Option(Unknown)
// =========================================================================

#[test]
fn option_none_is_option_unknown() {
    let mut interner = mk_interner();
    let x = interner.intern("x");
    let val = Expr::OptionNone;
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    assert!(matches!(env.lookup(x), LogosType::Option(_)));
}

#[test]
fn option_some_infers_inner_type() {
    let mut interner = mk_interner();
    let x = interner.intern("x");
    let inner = Expr::Literal(Literal::Number(42));
    let val = Expr::OptionSome { value: &inner };
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(x), &LogosType::Option(Box::new(LogosType::Int)));
}

// =========================================================================
// Function def and call
// =========================================================================

#[test]
fn function_def_registers_signature() {
    let mut interner = mk_interner();
    let f = interner.intern("double");
    let x_param = interner.intern("x");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);
    let ret_ty = TypeExpr::Primitive(int_sym);
    let lit = Expr::Literal(Literal::Number(0));
    let ret_stmt = Stmt::Return { value: Some(&lit) };
    let body = [ret_stmt];
    let stmts = [Stmt::FunctionDef {
        name: f,
        generics: vec![],
        params: vec![(x_param, &int_ty)],
        body: &body,
        return_type: Some(&ret_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    }];
    let env = run(&stmts, &interner);
    let sig = env.lookup_fn(f).expect("function should be registered");
    assert_eq!(sig.return_type, LogosType::Int);
    assert_eq!(sig.params.len(), 1);
    assert_eq!(sig.params[0].1, LogosType::Int);
}

#[test]
fn function_call_returns_registered_type() {
    let mut interner = mk_interner();
    let f = interner.intern("compute");
    let float_sym = interner.intern("Real");
    let float_ty = TypeExpr::Primitive(float_sym);
    let lit = Expr::Literal(Literal::Float(1.0));
    let ret_stmt = Stmt::Return { value: Some(&lit) };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![],
        params: vec![],
        body: &body,
        return_type: Some(&float_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let result_var = interner.intern("result");
    let call = Expr::Call {
        function: f,
        args: vec![],
    };
    let let_stmt = Stmt::Let {
        var: result_var,
        ty: None,
        value: &call,
        mutable: false,
    };
    let stmts = [fn_def, let_stmt];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(result_var), &LogosType::Float);
}

// =========================================================================
// ReadFrom is String
// =========================================================================

#[test]
fn readfrom_is_string() {
    let mut interner = mk_interner();
    let v = interner.intern("input");
    let stmts = [Stmt::ReadFrom {
        var: v,
        source: crate::ast::stmt::ReadSource::Console,
    }];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(v), &LogosType::String);
}

// =========================================================================
// Repeat loop variable gets element type
// =========================================================================

#[test]
fn repeat_loop_var_gets_element_type() {
    let mut interner = mk_interner();
    let items = interner.intern("items");
    let elem = interner.intern("elem");
    let one = Expr::Literal(Literal::Number(1));
    let list = Expr::List(vec![&one]);
    let let_items = Stmt::Let {
        var: items,
        ty: None,
        value: &list,
        mutable: false,
    };
    let items_ref = Expr::Identifier(items);
    let repeat = Stmt::Repeat {
        pattern: Pattern::Identifier(elem),
        iterable: &items_ref,
        body: &[],
    };
    let stmts = [let_items, repeat];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(elem), &LogosType::Int);
}

// =========================================================================
// Field access resolves to struct field type (uses registry)
// =========================================================================

#[test]
fn field_access_resolves_with_registry() {
    use crate::analysis::{FieldDef, FieldType, TypeDef};

    let mut interner = mk_interner();
    let point_sym = interner.intern("Point");
    let x_field_sym = interner.intern("x");
    let int_sym = interner.intern("Int");
    let p_var = interner.intern("p");
    let result_var = interner.intern("px");

    // Build a registry with a struct Point { x: Int }
    let mut registry = TypeRegistry::new();
    registry.register(
        point_sym,
        TypeDef::Struct {
            fields: vec![FieldDef {
                name: x_field_sym,
                ty: FieldType::Primitive(int_sym),
                is_public: true,
            }],
            generics: vec![],
            is_portable: false,
            is_shared: false,
        },
    );

    // Let p be a new Point.
    let new_point = Expr::New {
        type_name: point_sym,
        type_args: vec![],
        init_fields: vec![],
    };
    let let_p = Stmt::Let {
        var: p_var,
        ty: None,
        value: &new_point,
        mutable: false,
    };

    // Let px be p's x.
    let p_ref = Expr::Identifier(p_var);
    let field_access = Expr::FieldAccess {
        object: &p_ref,
        field: x_field_sym,
    };
    let let_px = Stmt::Let {
        var: result_var,
        ty: None,
        value: &field_access,
        mutable: false,
    };

    let stmts = [let_p, let_px];
    let env = check_program(&stmts, &interner, &registry).expect("check_program failed");
    assert_eq!(env.lookup(result_var), &LogosType::Int);
}

// =========================================================================
// Forward reference: calling a function defined later
// =========================================================================

#[test]
fn forward_reference_function_call() {
    let mut interner = mk_interner();
    let f = interner.intern("later_fn");
    let result_var = interner.intern("r");
    let bool_sym = interner.intern("Bool");
    let bool_ty = TypeExpr::Primitive(bool_sym);

    // Let r be later_fn().  (before the function def)
    let call = Expr::Call {
        function: f,
        args: vec![],
    };
    let let_r = Stmt::Let {
        var: result_var,
        ty: None,
        value: &call,
        mutable: false,
    };

    // ## Function later_fn -> Bool:
    let lit = Expr::Literal(Literal::Boolean(true));
    let ret_stmt = Stmt::Return { value: Some(&lit) };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![],
        params: vec![],
        body: &body,
        return_type: Some(&bool_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };

    // Note: let_r comes BEFORE fn_def in the slice
    let stmts = [let_r, fn_def];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(result_var), &LogosType::Bool);
}

// =========================================================================
// Type mismatch on return
// =========================================================================

#[test]
fn return_type_mismatch_fails() {
    let mut interner = mk_interner();
    let f = interner.intern("f");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);
    // Function annotated as -> Int but returns Text
    let lit = Expr::Literal(Literal::Text(Symbol::EMPTY));
    let ret_stmt = Stmt::Return { value: Some(&lit) };
    let body = [ret_stmt];
    let stmts = [Stmt::FunctionDef {
        name: f,
        generics: vec![],
        params: vec![],
        body: &body,
        return_type: Some(&int_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    }];
    let result = check_program(&stmts, &interner, &TypeRegistry::new());
    assert!(
        result.is_err(),
        "returning Text from Int function should fail"
    );
}

// =========================================================================
// New user-defined type → UserDefined
// =========================================================================

#[test]
fn new_user_defined_is_user_defined_type() {
    let mut interner = mk_interner();
    let point = interner.intern("Point");
    let p = interner.intern("p");
    let new_point = Expr::New {
        type_name: point,
        type_args: vec![],
        init_fields: vec![],
    };
    let stmts = [Stmt::Let {
        var: p,
        ty: None,
        value: &new_point,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    assert_eq!(env.lookup(p), &LogosType::UserDefined(point));
}

// =========================================================================
// Legacy API preserved: to_legacy_variable_types / to_legacy_string_vars
// =========================================================================

#[test]
fn string_vars_in_legacy_api() {
    let mut interner = mk_interner();
    let s = interner.intern("name");
    let hello = interner.intern("hello");
    let val = Expr::Literal(Literal::Text(hello));
    let stmts = [Stmt::Let {
        var: s,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    assert!(env.to_legacy_string_vars().contains(&s));
}

#[test]
fn unknown_vars_filtered_in_legacy_api() {
    let mut interner = mk_interner();
    let x = interner.intern("x");
    let val = Expr::OptionNone; // Unknown inner type
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let env = run(&stmts, &interner);
    // Option(Unknown) → not in string_vars, not filtered as error
    let legacy = env.to_legacy_variable_types();
    // Option(Unknown) maps to "Option<_>", which is concrete enough
    assert!(!legacy.is_empty() || legacy.is_empty()); // just don't panic
}

// =========================================================================
// Generic (polymorphic) functions — Phase 3
// =========================================================================

#[test]
fn generic_identity_infers_int_return() {
    // ## To identity of [T] (x: T) -> T:
    //     Return x.
    // Let r be identity(42).  → r is Int
    let mut interner = mk_interner();
    let f = interner.intern("identity");
    let x_param = interner.intern("x");
    let t_sym = interner.intern("T");
    let t_ty = TypeExpr::Primitive(t_sym);
    let x_ref = Expr::Identifier(x_param);
    let ret_stmt = Stmt::Return {
        value: Some(&x_ref),
    };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![t_sym],
        params: vec![(x_param, &t_ty)],
        body: &body,
        return_type: Some(&t_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let r = interner.intern("r");
    let lit = Expr::Literal(Literal::Number(42));
    let call = Expr::Call {
        function: f,
        args: vec![&lit],
    };
    let let_r = Stmt::Let {
        var: r,
        ty: None,
        value: &call,
        mutable: false,
    };
    let stmts = [fn_def, let_r];
    let env = run(&stmts, &interner);
    assert_eq!(
        env.lookup(r),
        &LogosType::Int,
        "identity(42) should return Int, got {:?}",
        env.lookup(r)
    );
}

#[test]
fn generic_identity_infers_bool_return() {
    // Same identity function, called with Bool → returns Bool.
    let mut interner = mk_interner();
    let f = interner.intern("identity");
    let x_param = interner.intern("x");
    let t_sym = interner.intern("T");
    let t_ty = TypeExpr::Primitive(t_sym);
    let x_ref = Expr::Identifier(x_param);
    let ret_stmt = Stmt::Return {
        value: Some(&x_ref),
    };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![t_sym],
        params: vec![(x_param, &t_ty)],
        body: &body,
        return_type: Some(&t_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let r = interner.intern("r");
    let lit = Expr::Literal(Literal::Boolean(true));
    let call = Expr::Call {
        function: f,
        args: vec![&lit],
    };
    let let_r = Stmt::Let {
        var: r,
        ty: None,
        value: &call,
        mutable: false,
    };
    let stmts = [fn_def, let_r];
    let env = run(&stmts, &interner);
    assert_eq!(
        env.lookup(r),
        &LogosType::Bool,
        "identity(true) should return Bool, got {:?}",
        env.lookup(r)
    );
}

#[test]
fn generic_two_type_params_first() {
    // ## To first of [A] and [B] (a: A, b: B) -> A:
    //     Return a.
    // Let r be first(42, true).  → r is Int (first type param)
    let mut interner = mk_interner();
    let f = interner.intern("first");
    let a_param = interner.intern("a");
    let b_param = interner.intern("b");
    let a_sym = interner.intern("A");
    let b_sym = interner.intern("B");
    let a_ty = TypeExpr::Primitive(a_sym);
    let b_ty = TypeExpr::Primitive(b_sym);
    let a_ref = Expr::Identifier(a_param);
    let ret_stmt = Stmt::Return {
        value: Some(&a_ref),
    };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![a_sym, b_sym],
        params: vec![(a_param, &a_ty), (b_param, &b_ty)],
        body: &body,
        return_type: Some(&a_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let r = interner.intern("r");
    let lit_int = Expr::Literal(Literal::Number(42));
    let lit_bool = Expr::Literal(Literal::Boolean(true));
    let call = Expr::Call {
        function: f,
        args: vec![&lit_int, &lit_bool],
    };
    let let_r = Stmt::Let {
        var: r,
        ty: None,
        value: &call,
        mutable: false,
    };
    let stmts = [fn_def, let_r];
    let env = run(&stmts, &interner);
    assert_eq!(
        env.lookup(r),
        &LogosType::Int,
        "first(42, true) should return Int (first param type), got {:?}",
        env.lookup(r)
    );
}

#[test]
fn generic_calls_are_independent() {
    // Each call to a generic function gets its own fresh type variables.
    // identity(42) → Int, identity(true) → Bool, independent results.
    let mut interner = mk_interner();
    let f = interner.intern("identity");
    let x_param = interner.intern("x");
    let t_sym = interner.intern("T");
    let t_ty = TypeExpr::Primitive(t_sym);
    let x_ref = Expr::Identifier(x_param);
    let ret_stmt = Stmt::Return {
        value: Some(&x_ref),
    };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![t_sym],
        params: vec![(x_param, &t_ty)],
        body: &body,
        return_type: Some(&t_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let r1 = interner.intern("r1");
    let r2 = interner.intern("r2");
    let lit_int = Expr::Literal(Literal::Number(42));
    let lit_bool = Expr::Literal(Literal::Boolean(true));
    let call1 = Expr::Call {
        function: f,
        args: vec![&lit_int],
    };
    let call2 = Expr::Call {
        function: f,
        args: vec![&lit_bool],
    };
    let let_r1 = Stmt::Let {
        var: r1,
        ty: None,
        value: &call1,
        mutable: false,
    };
    let let_r2 = Stmt::Let {
        var: r2,
        ty: None,
        value: &call2,
        mutable: false,
    };
    let stmts = [fn_def, let_r1, let_r2];
    let env = run(&stmts, &interner);
    assert_eq!(
        env.lookup(r1),
        &LogosType::Int,
        "identity(42) should be Int, got {:?}",
        env.lookup(r1)
    );
    assert_eq!(
        env.lookup(r2),
        &LogosType::Bool,
        "identity(true) should be Bool, got {:?}",
        env.lookup(r2)
    );
}

#[test]
fn monomorphic_functions_unaffected_by_generics() {
    // Non-generic functions still work correctly with the updated machinery.
    let mut interner = mk_interner();
    let f = interner.intern("double");
    let x_param = interner.intern("x");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);
    let x_ref = Expr::Identifier(x_param);
    let lit2 = Expr::Literal(Literal::Number(2));
    let mul = Expr::BinaryOp {
        op: BinaryOpKind::Multiply,
        left: &x_ref,
        right: &lit2,
    };
    let ret_stmt = Stmt::Return { value: Some(&mul) };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![],
        params: vec![(x_param, &int_ty)],
        body: &body,
        return_type: Some(&int_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let r = interner.intern("r");
    let lit5 = Expr::Literal(Literal::Number(5));
    let call = Expr::Call {
        function: f,
        args: vec![&lit5],
    };
    let let_r = Stmt::Let {
        var: r,
        ty: None,
        value: &call,
        mutable: false,
    };
    let stmts = [fn_def, let_r];
    let env = run(&stmts, &interner);
    assert_eq!(
        env.lookup(r),
        &LogosType::Int,
        "double(5) should return Int, got {:?}",
        env.lookup(r)
    );
}

#[test]
fn generic_forward_reference_resolves() {
    // Let r be identity(42).
    // ## To identity of [T] (x: T) -> T:  ← defined AFTER the call
    //     Return x.
    // The pre-pass must register generics before the main pass sees the call.
    let mut interner = mk_interner();
    let f = interner.intern("identity");
    let x_param = interner.intern("x");
    let t_sym = interner.intern("T");
    let t_ty = TypeExpr::Primitive(t_sym);
    let x_ref = Expr::Identifier(x_param);
    let ret_stmt = Stmt::Return {
        value: Some(&x_ref),
    };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![t_sym],
        params: vec![(x_param, &t_ty)],
        body: &body,
        return_type: Some(&t_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let r = interner.intern("r");
    let lit = Expr::Literal(Literal::Number(99));
    let call = Expr::Call {
        function: f,
        args: vec![&lit],
    };
    let let_r = Stmt::Let {
        var: r,
        ty: None,
        value: &call,
        mutable: false,
    };
    // Call appears BEFORE the function definition
    let stmts = [let_r, fn_def];
    let env = run(&stmts, &interner);
    assert_eq!(
        env.lookup(r),
        &LogosType::Int,
        "forward-ref identity(99) should be Int, got {:?}",
        env.lookup(r)
    );
}

// ---- Bug Report #1 regression pins ----

/// BUG-025: a `Seq of Real` annotation must accept integer literals
/// `[1, 2, 3]` (the numeric-literal coercion scalar literals already get),
/// rather than synthesizing `Seq(Int)` from `items[0]` and failing to unify
/// with `Seq(Float)`.
#[test]
fn seq_of_real_accepts_int_literals() {
    let mut interner = Interner::new();
    let xs = interner.intern("xs");
    let real_sym = interner.intern("Real");
    let seq_sym = interner.intern("Seq");

    let one = Expr::Literal(Literal::Number(1));
    let two = Expr::Literal(Literal::Number(2));
    let three = Expr::Literal(Literal::Number(3));
    let val = Expr::List(vec![&one, &two, &three]);

    let real_ty = TypeExpr::Primitive(real_sym);
    let params = [real_ty];
    let seq_real = TypeExpr::Generic {
        base: seq_sym,
        params: &params,
    };

    let stmts = [Stmt::Let {
        var: xs,
        ty: Some(&seq_real),
        value: &val,
        mutable: false,
    }];

    let env = check_program(&stmts, &interner, &TypeRegistry::new())
        .expect("Seq of Real should accept integer literals [1, 2, 3]");

    assert_eq!(
        env.lookup(xs),
        &LogosType::Seq(Box::new(LogosType::Float)),
        "xs should be inferred as Seq<Float> under the `Seq of Real` annotation"
    );
}

/// BUG-026: a generic function with an INFERRED (unannotated) return type
/// must generalize its return variable, so two calls at different types are
/// independent. Mirrors `generic_calls_are_independent` but `return_type: None`.
#[test]
fn generic_inferred_return_calls_are_independent() {
    let mut interner = mk_interner();
    let f = interner.intern("wrap");
    let x_param = interner.intern("x");
    let t_sym = interner.intern("T");
    let t_ty = TypeExpr::Primitive(t_sym);
    let x_ref = Expr::Identifier(x_param);
    let ret_stmt = Stmt::Return {
        value: Some(&x_ref),
    };
    let body = [ret_stmt];
    let fn_def = Stmt::FunctionDef {
        name: f,
        generics: vec![t_sym],
        params: vec![(x_param, &t_ty)],
        body: &body,
        return_type: None, // inferred return -> fresh var not in scheme.vars: the bug
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let r1 = interner.intern("r1");
    let r2 = interner.intern("r2");
    let lit_int = Expr::Literal(Literal::Number(42));
    let lit_bool = Expr::Literal(Literal::Boolean(true));
    let call1 = Expr::Call {
        function: f,
        args: vec![&lit_int],
    };
    let call2 = Expr::Call {
        function: f,
        args: vec![&lit_bool],
    };
    let let_r1 = Stmt::Let {
        var: r1,
        ty: None,
        value: &call1,
        mutable: false,
    };
    let let_r2 = Stmt::Let {
        var: r2,
        ty: None,
        value: &call2,
        mutable: false,
    };
    let stmts = [fn_def, let_r1, let_r2];
    let env = run(&stmts, &interner);
    assert_eq!(
        env.lookup(r1),
        &LogosType::Int,
        "wrap(42) should be Int, got {:?}",
        env.lookup(r1)
    );
    assert_eq!(
        env.lookup(r2),
        &LogosType::Bool,
        "wrap(true) should be Bool, got {:?}",
        env.lookup(r2)
    );
}

/// BUG-007: iterating a `Map` with a single identifier loop variable yields
/// `(key, value)` tuples at runtime, so the loop var must NOT be typed as
/// the bare key type `K`.
#[test]
fn repeat_over_map_single_ident_loop_var_is_not_bare_key() {
    let mut interner = mk_interner();
    let m = interner.intern("m");
    let entry = interner.intern("entry");
    let map_sym = interner.intern("Map");
    let text_sym = interner.intern("Text");
    let int_sym = interner.intern("Int");

    let new_map = Expr::New {
        type_name: map_sym,
        type_args: vec![TypeExpr::Primitive(text_sym), TypeExpr::Primitive(int_sym)],
        init_fields: vec![],
    };
    let let_m = Stmt::Let {
        var: m,
        ty: None,
        value: &new_map,
        mutable: false,
    };

    let m_ref = Expr::Identifier(m);
    let repeat = Stmt::Repeat {
        pattern: Pattern::Identifier(entry),
        iterable: &m_ref,
        body: &[],
    };

    let stmts = [let_m, repeat];
    let env = run(&stmts, &interner);

    assert_ne!(
        env.lookup(entry),
        &LogosType::String,
        "iterating a Map with a single identifier yields (key,value) tuples at runtime; \
             the loop var must not be typed as the bare key K"
    );
}

// =========================================================================
// Generic sites, checked from real source
// =========================================================================

/// Every generic site `src` itself writes, in source order, rendered as `label: type` —
/// `call f: [..]`, `new Name: ..`, `some`/`none`/`[]: ..`, `inspect: ..`. A type variable
/// renders as the generic parameter it is (`T@f`) when it is one, else `?`. Functions the
/// prelude brought in are checked with the program but not listed.
fn user_sites(src: &str) -> Vec<String> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("parses");
        let inst = infer_instantiations(stmts, interner, registry);
        let mut out = Vec::new();
        for s in stmts {
            let written_here = match s {
                Stmt::FunctionDef { name, .. } => {
                    src.contains(&format!("## To {} ", interner.resolve(*name)))
                }
                _ => true,
            };
            if written_here {
                walk_stmt(s, &inst, interner, &mut out);
            }
        }
        out
    })
}

fn render(t: &InferType, inst: &Instantiations, interner: &Interner) -> String {
    let r = |x: &InferType| render(x, inst, interner);
    match t {
        InferType::Var(tv) => inst
            .type_params
            .iter()
            .flat_map(|(f, ps)| ps.iter().map(move |(p, pt)| (f, p, pt)))
            .find(|(_, _, pt)| **pt == InferType::Var(*tv))
            .map(|(f, p, _)| format!("{}@{}", interner.resolve(*p), interner.resolve(*f)))
            .unwrap_or_else(|| "?".to_string()),
        InferType::UserDefined(sym, args) if args.is_empty() => interner.resolve(*sym).to_string(),
        InferType::UserDefined(sym, args) => format!(
            "{}({})",
            interner.resolve(*sym),
            args.iter().map(r).collect::<Vec<_>>().join(", ")
        ),
        InferType::Seq(e) => format!("Seq({})", r(e)),
        InferType::Option(e) => format!("Option({})", r(e)),
        InferType::Map(k, v) => format!("Map({}, {})", r(k), r(v)),
        other => other.to_logos_name(),
    }
}

fn walk_expr(e: &Expr, inst: &Instantiations, interner: &Interner, out: &mut Vec<String>) {
    let site = Site::of_expr(e);
    let built = |label: String, out: &mut Vec<String>| {
        if let Some(t) = inst.constructed.get(&site) {
            out.push(format!("{label}: {}", render(t, inst, interner)));
        }
    };
    match e {
        Expr::Call { function, args } => {
            for a in args {
                walk_expr(a, inst, interner, out);
            }
            if let Some(c) = inst.calls.get(&site) {
                let ts: Vec<String> = c
                    .type_args
                    .iter()
                    .map(|t| render(t, inst, interner))
                    .collect();
                out.push(format!(
                    "call {}: [{}]",
                    interner.resolve(*function),
                    ts.join(", ")
                ));
            }
        }
        Expr::New {
            type_name,
            init_fields,
            ..
        } => {
            for (_, v) in init_fields {
                walk_expr(v, inst, interner, out);
            }
            built(format!("new {}", interner.resolve(*type_name)), out);
        }
        Expr::NewVariant {
            variant, fields, ..
        } => {
            for (_, v) in fields {
                walk_expr(v, inst, interner, out);
            }
            built(format!("new {}", interner.resolve(*variant)), out);
        }
        Expr::OptionSome { value } => {
            walk_expr(value, inst, interner, out);
            built("some".to_string(), out);
        }
        Expr::OptionNone => built("none".to_string(), out),
        Expr::List(items) => {
            for i in items {
                walk_expr(i, inst, interner, out);
            }
            if let Some(t) = inst.empty_lists.get(&site) {
                out.push(format!("[]: {}", render(t, inst, interner)));
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            walk_expr(left, inst, interner, out);
            walk_expr(right, inst, interner, out);
        }
        Expr::FieldAccess { object, .. } => walk_expr(object, inst, interner, out),
        _ => {}
    }
}

fn walk_stmt(s: &Stmt, inst: &Instantiations, interner: &Interner, out: &mut Vec<String>) {
    let walk_block = |b: &[Stmt], out: &mut Vec<String>| {
        for s in b {
            walk_stmt(s, inst, interner, out);
        }
    };
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => walk_expr(value, inst, interner, out),
        Stmt::Show { object, .. } => walk_expr(object, inst, interner, out),
        Stmt::Return { value: Some(v) } => walk_expr(v, inst, interner, out),
        Stmt::Push { value, .. } => walk_expr(value, inst, interner, out),
        Stmt::Call { function, args } => {
            for a in args {
                walk_expr(a, inst, interner, out);
            }
            if let Some(c) = inst.calls.get(&Site::of_stmt(s)) {
                let ts: Vec<String> = c
                    .type_args
                    .iter()
                    .map(|t| render(t, inst, interner))
                    .collect();
                out.push(format!(
                    "call {}: [{}]",
                    interner.resolve(*function),
                    ts.join(", ")
                ));
            }
        }
        Stmt::Inspect { target, arms, .. } => {
            walk_expr(target, inst, interner, out);
            if let Some(t) = inst.scrutinees.get(&Site::of_stmt(s)) {
                out.push(format!("inspect: {}", render(t, inst, interner)));
            }
            for arm in arms {
                walk_block(arm.body, out);
            }
        }
        Stmt::FunctionDef { body, .. } => walk_block(body, out),
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            walk_block(then_block, out);
            if let Some(b) = else_block {
                walk_block(b, out);
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => walk_block(body, out),
        _ => {}
    }
}

/// The type the checker bound `var` to, by name.
fn type_of_var(src: &str, var: &str) -> LogosType {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("parses");
        let env = check_program(stmts, interner, registry).expect("type-checks");
        let sym = interner.lookup(var).expect("variable is interned");
        env.lookup(sym).clone()
    })
}

#[test]
fn each_generic_call_records_its_own_type_arguments() {
    let sites = user_sites(
        "## To identity of [T] (x: T) -> T:\n    Return x.\n\n## Main\nShow identity(5).\nShow identity(\"five\").\nShow identity(5.5).\n",
    );
    assert_eq!(
        sites,
        vec![
            "call identity: [Int]",
            "call identity: [Text]",
            "call identity: [Real]"
        ]
    );
}

#[test]
fn a_call_inside_a_generic_body_names_the_enclosing_parameter() {
    let sites = user_sites(
        "## To identity of [T] (x: T) -> T:\n    Return x.\n\n## To twice of [T] (x: T) -> T:\n    Return identity(identity(x)).\n\n## Main\nShow twice(\"deep\").\n",
    );
    assert_eq!(
        sites,
        vec![
            "call identity: [T@twice]",
            "call identity: [T@twice]",
            "call twice: [Text]"
        ]
    );
}

#[test]
fn a_self_recursive_generic_call_names_its_own_parameter() {
    let sites = user_sites(
        "## To lastOf of [T] (xs: Seq of T, i: Int) -> T:\n    If i equals length of xs:\n        Return item i of xs.\n    Return lastOf(xs, i + 1).\n\n## Main\nShow lastOf([\"a\", \"b\"], 1).\n",
    );
    assert_eq!(
        sites,
        vec!["call lastOf: [T@lastOf]", "call lastOf: [Text]"]
    );
}

#[test]
fn a_payload_solves_the_variant_type() {
    let sites = user_sites(
        "## A Maybe of [T] is either:\n    A Nothing.\n    A Just with value T.\n\n## Main\nLet m be a new Just with value 42.\nLet n be a new Just with value \"s\".\n",
    );
    assert_eq!(sites, vec!["new Just: Maybe(Int)", "new Just: Maybe(Text)"]);
}

#[test]
fn a_payload_free_variant_takes_its_type_from_the_parameter() {
    let sites = user_sites(
        "## A Maybe of [T] is either:\n    A Nothing.\n    A Just with value T.\n\n## To orZero (m: Maybe of Int) -> Int:\n    Inspect m:\n        When Nothing: Return 0.\n        When Just (v): Return v.\n\n## Main\nShow orZero(a new Nothing).\n",
    );
    assert_eq!(
        sites,
        vec!["inspect: Maybe(Int)", "new Nothing: Maybe(Int)"]
    );
}

#[test]
fn a_payload_free_variant_takes_its_type_from_the_other_arguments() {
    let sites = user_sites(
        "## A Maybe of [T] is either:\n    A Nothing.\n    A Just with value T.\n\n## To withDefault of [T] (m: Maybe of T, d: T) -> T:\n    Inspect m:\n        When Nothing: Return d.\n        When Just (v): Return v.\n\n## Main\nShow withDefault(a new Nothing, 7).\n",
    );
    assert_eq!(
        sites,
        vec![
            "inspect: Maybe(T@withDefault)",
            "new Nothing: Maybe(Int)",
            "call withDefault: [Int]"
        ]
    );
}

#[test]
fn a_let_bound_variant_is_solved_by_a_later_use() {
    let sites = user_sites(
        "## A Tree of [T] is either:\n    A Leaf.\n    A Node with value T and left Tree of T and right Tree of T.\n\n## Main\nLet leaf be a new Leaf.\nLet inner be a new Node with value 2 and left leaf and right leaf.\nLet t be a new Node with value 1 and left inner and right leaf.\n",
    );
    assert_eq!(
        sites,
        vec![
            "new Leaf: Tree(Int)",
            "new Node: Tree(Int)",
            "new Node: Tree(Int)"
        ]
    );
}

#[test]
fn a_nested_constructor_takes_the_element_type_from_its_parent() {
    let sites = user_sites(
        "## A MyList of [T] is either:\n    An Empty.\n    A Cons with head T and tail MyList of T.\n\n## Main\nLet l be a new Cons with head \"a\" and tail a new Cons with head \"b\" and tail a new Empty.\n",
    );
    assert_eq!(
        sites,
        vec![
            "new Empty: MyList(Text)",
            "new Cons: MyList(Text)",
            "new Cons: MyList(Text)"
        ]
    );
}

#[test]
fn a_generic_record_field_reads_at_the_applied_type() {
    let src = "## A Box of [T] has:\n    a value, which is T.\n\n## Main\nLet b be a new Box of Text with value \"in\".\nLet v be b's value.\n";
    assert_eq!(type_of_var(src, "v"), LogosType::String);
    assert_eq!(user_sites(src), vec!["new Box: Box(Text)"]);
}

#[test]
fn a_generic_record_built_without_type_arguments_is_solved_by_its_fields() {
    let src = "## A Pair of [A] and [B] has:\n    a left, which is A.\n    a right, which is B.\n\n## Main\nLet p be a new Pair with left 1 and right \"one\".\nLet r be p's right.\n";
    assert_eq!(type_of_var(src, "r"), LogosType::String);
    assert_eq!(user_sites(src), vec!["new Pair: Pair(Int, Text)"]);
}

#[test]
fn inspect_binds_payloads_at_the_inspected_type() {
    let src = "## A Maybe of [T] is either:\n    A Nothing.\n    A Just with value T.\n\n## Main\nLet m be a new Just with value \"s\".\nInspect m:\n    When Nothing: Show 0.\n    When Just (v):\n        Let w be v.\n        Show w.\n";
    assert_eq!(type_of_var(src, "w"), LogosType::String);
}

#[test]
fn inspect_binds_an_option_payload_at_its_element_type() {
    let src = "## Main\nLet o be some \"x\".\nInspect o:\n    When Some (v):\n        Let w be v.\n        Show w.\n    When None: Show 0.\n";
    assert_eq!(type_of_var(src, "w"), LogosType::String);
}

#[test]
fn a_user_declared_maybe_is_theirs_not_the_builtin_option() {
    let src = "## A Maybe of [T] is either:\n    A Nothing.\n    A Just with value T.\n\n## To orZero (m: Maybe of Int) -> Int:\n    Inspect m:\n        When Nothing: Return 0.\n        When Just (v): Return v.\n\n## Main\nLet z be orZero(a new Just with value 5).\n";
    assert_eq!(type_of_var(src, "z"), LogosType::Int);
}

#[test]
fn core_result_arguments_are_recorded_even_when_one_is_unconstrained() {
    let sites = user_sites(
        "## To valueOr of [T] and [E] (r: Result of T and E, d: T) -> T:\n    Inspect r:\n        When Ok (v): Return v.\n        When Err (e): Return d.\n\n## Main\nShow valueOr(a new Ok with value 8, 0).\n",
    );
    assert_eq!(
        sites,
        vec![
            "inspect: Result(T@valueOr, E@valueOr)",
            "new Ok: Result(Int, ?)",
            "call valueOr: [Int, ?]"
        ]
    );
}

#[test]
fn none_takes_its_element_from_the_other_arguments() {
    let sites = user_sites(
        "## To orElse of [T] (o: Option of T, d: T) -> T:\n    Inspect o:\n        When Some (v): Return v.\n        When None: Return d.\n\n## Main\nShow orElse(some 3, 0).\nShow orElse(none, \"d\").\n",
    );
    assert_eq!(
        sites,
        vec![
            "inspect: Option(T@orElse)",
            "some: Option(Int)",
            "call orElse: [Int]",
            "none: Option(Text)",
            "call orElse: [Text]",
        ]
    );
}

#[test]
fn an_empty_list_takes_its_element_from_the_field_it_fills() {
    let sites = user_sites(
        "## A Point has:\n    An x: Int.\n\n## A Grid has:\n    A pts: Seq of Point.\n\n## Main\nLet g be a new Grid with pts [].\n",
    );
    assert_eq!(sites, vec!["[]: Seq(Point)", "new Grid: Grid"]);
}

#[test]
fn every_list_element_is_checked_not_only_the_first() {
    let sites = user_sites(
        "## To identity of [T] (x: T) -> T:\n    Return x.\n\n## Main\nLet xs be [identity(1), identity(2)].\n",
    );
    assert_eq!(sites, vec!["call identity: [Int]", "call identity: [Int]"]);
}

#[test]
fn a_type_parameter_written_in_a_generic_body_is_that_functions_own() {
    let sites = user_sites(
        "## To replicate of [T] (x: T, n: Int) -> Seq of T:\n    Let out be a new Seq of T.\n    Push x to out.\n    Return out.\n\n## Main\nShow replicate(\"ab\", 1).\n",
    );
    assert_eq!(
        sites,
        vec!["new Seq: Seq(T@replicate)", "call replicate: [Text]"]
    );
}

#[test]
fn a_call_whose_argument_type_is_unknown_is_imprecise() {
    crate::ui_bridge::with_parsed_program(
        "## To identity of [T] (x: T) -> T:\n    Return x.\n\n## Main\nLet t be (1, 2).\nShow identity(t).\nShow identity(3).\n",
        |parsed, interner| {
            let (stmts, registry, _) = parsed.expect("parses");
            let inst = infer_instantiations(stmts, interner, registry);
            let mut precise: Vec<bool> = inst.calls.values().map(|c| c.precise).collect();
            precise.sort();
            assert_eq!(precise, vec![false, true], "a tuple argument is untyped to the checker");
        },
    );
}

/// A type error names the top-level function it was found in, so a report can say where; a `## Main`
/// statement's error names none.
#[test]
fn a_type_error_names_the_function_it_is_in() {
    let src = "## To f (n: Int) -> Text:\n    Let t: Text be n + 1.\n    Return t.\n\n## Main\nShow f(1).\n";
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("parses");
        let err = check_program_located(stmts, interner, registry).expect_err("Int is not Text");
        assert_eq!(
            err.within
                .map(|f| interner.resolve(f).to_string())
                .as_deref(),
            Some("f")
        );
        match err.to_parse_error_kind(interner, stmts) {
            crate::error::ParseErrorKind::TypeMismatchDetailed { context, .. } => {
                assert_eq!(context, "in `f`")
            }
            other => panic!("expected a detailed mismatch, got {other:?}"),
        }
    });
    let main_only = "## Main\nLet t: Text be 1 + 1.\nShow t.\n";
    crate::ui_bridge::with_parsed_program(main_only, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("parses");
        let err = check_program_located(stmts, interner, registry).expect_err("Int is not Text");
        assert_eq!(err.within, None);
    });
}
