use super::*;

// =========================================================================
// LogosType::is_copy tests
// =========================================================================

#[test]
fn copy_types_are_copy() {
    assert!(LogosType::Int.is_copy());
    assert!(LogosType::Float.is_copy());
    assert!(LogosType::Bool.is_copy());
    assert!(LogosType::Char.is_copy());
    assert!(LogosType::Byte.is_copy());
    assert!(LogosType::Unit.is_copy());
    assert!(LogosType::Nat.is_copy());
}

#[test]
fn non_copy_types_are_not_copy() {
    assert!(!LogosType::String.is_copy());
    assert!(!LogosType::Seq(Box::new(LogosType::Int)).is_copy());
    assert!(!LogosType::Map(Box::new(LogosType::String), Box::new(LogosType::Int)).is_copy());
    assert!(!LogosType::Set(Box::new(LogosType::Int)).is_copy());
    assert!(!LogosType::Option(Box::new(LogosType::Int)).is_copy());
    assert!(!LogosType::Duration.is_copy());
    assert!(!LogosType::Unknown.is_copy());
}

// =========================================================================
// LogosType::is_numeric / is_string / is_float
// =========================================================================

#[test]
fn numeric_types() {
    assert!(LogosType::Int.is_numeric());
    assert!(LogosType::Float.is_numeric());
    assert!(LogosType::Nat.is_numeric());
    assert!(!LogosType::String.is_numeric());
    assert!(!LogosType::Bool.is_numeric());
}

#[test]
fn string_type() {
    assert!(LogosType::String.is_string());
    assert!(!LogosType::Int.is_string());
    assert!(!LogosType::Unknown.is_string());
}

#[test]
fn float_type() {
    assert!(LogosType::Float.is_float());
    assert!(!LogosType::Int.is_float());
}

// =========================================================================
// LogosType::element_type / value_type / key_type
// =========================================================================

#[test]
fn seq_element_type() {
    let seq = LogosType::Seq(Box::new(LogosType::Int));
    assert_eq!(seq.element_type(), Some(&LogosType::Int));
}

#[test]
fn set_element_type() {
    let set = LogosType::Set(Box::new(LogosType::String));
    assert_eq!(set.element_type(), Some(&LogosType::String));
}

#[test]
fn map_key_value_types() {
    let map = LogosType::Map(Box::new(LogosType::String), Box::new(LogosType::Int));
    assert_eq!(map.key_type(), Some(&LogosType::String));
    assert_eq!(map.value_type(), Some(&LogosType::Int));
}

#[test]
fn non_collection_element_type() {
    assert_eq!(LogosType::Int.element_type(), None);
    assert_eq!(LogosType::String.element_type(), None);
}

// =========================================================================
// LogosType::numeric_promotion
// =========================================================================

#[test]
fn numeric_promotion_int_int() {
    assert_eq!(
        LogosType::numeric_promotion(&LogosType::Int, &LogosType::Int),
        LogosType::Int
    );
}

#[test]
fn numeric_promotion_int_float() {
    assert_eq!(
        LogosType::numeric_promotion(&LogosType::Int, &LogosType::Float),
        LogosType::Float
    );
}

#[test]
fn numeric_promotion_float_int() {
    assert_eq!(
        LogosType::numeric_promotion(&LogosType::Float, &LogosType::Int),
        LogosType::Float
    );
}

#[test]
fn numeric_promotion_float_float() {
    assert_eq!(
        LogosType::numeric_promotion(&LogosType::Float, &LogosType::Float),
        LogosType::Float
    );
}

#[test]
fn numeric_promotion_non_numeric() {
    assert_eq!(
        LogosType::numeric_promotion(&LogosType::String, &LogosType::Int),
        LogosType::Unknown
    );
}

// =========================================================================
// LogosType::to_rust_type
// =========================================================================

#[test]
fn to_rust_type_primitives() {
    assert_eq!(LogosType::Int.to_rust_type(), "i64");
    assert_eq!(LogosType::Float.to_rust_type(), "f64");
    assert_eq!(LogosType::Bool.to_rust_type(), "bool");
    assert_eq!(LogosType::Char.to_rust_type(), "char");
    assert_eq!(LogosType::Byte.to_rust_type(), "u8");
    assert_eq!(LogosType::String.to_rust_type(), "String");
    assert_eq!(LogosType::Unit.to_rust_type(), "()");
    assert_eq!(LogosType::Nat.to_rust_type(), "u64");
}

#[test]
fn to_rust_type_temporal() {
    assert_eq!(LogosType::Duration.to_rust_type(), "std::time::Duration");
    assert_eq!(LogosType::Date.to_rust_type(), "LogosDate");
    assert_eq!(LogosType::Moment.to_rust_type(), "LogosMoment");
    assert_eq!(LogosType::Time.to_rust_type(), "LogosTime");
    assert_eq!(LogosType::Span.to_rust_type(), "LogosSpan");
}

#[test]
fn to_rust_type_collections() {
    assert_eq!(
        LogosType::Seq(Box::new(LogosType::Int)).to_rust_type(),
        "LogosSeq<i64>"
    );
    assert_eq!(
        LogosType::Map(Box::new(LogosType::String), Box::new(LogosType::Int)).to_rust_type(),
        "LogosMap<String, i64>"
    );
    assert_eq!(
        LogosType::Set(Box::new(LogosType::Int)).to_rust_type(),
        "Set<i64>"
    );
    assert_eq!(
        LogosType::Option(Box::new(LogosType::String)).to_rust_type(),
        "Option<String>"
    );
}

#[test]
fn to_rust_type_nested() {
    let nested = LogosType::Seq(Box::new(LogosType::Seq(Box::new(LogosType::Float))));
    assert_eq!(nested.to_rust_type(), "LogosSeq<LogosSeq<f64>>");
}

// =========================================================================
// LogosType::from_literal
// =========================================================================

#[test]
fn from_literal_number() {
    assert_eq!(
        LogosType::from_literal(&Literal::Number(42)),
        LogosType::Int
    );
}

#[test]
fn from_literal_float() {
    assert_eq!(
        LogosType::from_literal(&Literal::Float(3.14)),
        LogosType::Float
    );
}

#[test]
fn from_literal_text() {
    assert_eq!(
        LogosType::from_literal(&Literal::Text(Symbol::EMPTY)),
        LogosType::String
    );
}

#[test]
fn from_literal_bool() {
    assert_eq!(
        LogosType::from_literal(&Literal::Boolean(true)),
        LogosType::Bool
    );
}

#[test]
fn from_literal_nothing() {
    assert_eq!(LogosType::from_literal(&Literal::Nothing), LogosType::Unit);
}

// =========================================================================
// LogosType::from_type_expr
// =========================================================================

#[test]
fn from_type_expr_primitive_int() {
    let mut interner = Interner::new();
    let sym = interner.intern("Int");
    let ty = TypeExpr::Primitive(sym);
    assert_eq!(LogosType::from_type_expr(&ty, &interner), LogosType::Int);
}

#[test]
fn from_type_expr_named_text() {
    let mut interner = Interner::new();
    let sym = interner.intern("Text");
    let ty = TypeExpr::Named(sym);
    assert_eq!(LogosType::from_type_expr(&ty, &interner), LogosType::String);
}

// =========================================================================
// TypeEnv basics
// =========================================================================

#[test]
fn env_lookup_unknown_returns_unknown() {
    let env = TypeEnv::new();
    assert_eq!(env.lookup(Symbol::EMPTY), &LogosType::Unknown);
}

#[test]
fn env_register_and_lookup() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let x = interner.intern("x");
    env.register(x, LogosType::Int);
    assert_eq!(env.lookup(x), &LogosType::Int);
}

#[test]
fn env_register_fn_and_lookup() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let f = interner.intern("add");
    let a = interner.intern("a");
    let b = interner.intern("b");
    env.register_fn(
        f,
        FnSig {
            params: vec![(a, LogosType::Int), (b, LogosType::Int)],
            return_type: LogosType::Int,
        },
    );
    let sig = env.lookup_fn(f).unwrap();
    assert_eq!(sig.return_type, LogosType::Int);
    assert_eq!(sig.params.len(), 2);
}

// =========================================================================
// TypeEnv::infer_expr
// =========================================================================

#[test]
fn infer_expr_literal_number() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let expr = Expr::Literal(Literal::Number(42));
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
}

#[test]
fn infer_expr_literal_float() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let expr = Expr::Literal(Literal::Float(3.14));
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
}

#[test]
fn infer_expr_literal_text() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let expr = Expr::Literal(Literal::Text(Symbol::EMPTY));
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::String);
}

#[test]
fn infer_expr_identifier_registered() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let x = interner.intern("x");
    env.register(x, LogosType::Float);
    let expr = Expr::Identifier(x);
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
}

#[test]
fn infer_expr_identifier_unknown() {
    let env = TypeEnv::new();
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let expr = Expr::Identifier(x);
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Unknown);
}

#[test]
fn infer_expr_add_int_int() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let left = Expr::Literal(Literal::Number(1));
    let right = Expr::Literal(Literal::Number(2));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: &left,
        right: &right,
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
}

#[test]
fn infer_expr_add_int_float_promotes() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let left = Expr::Literal(Literal::Number(1));
    let right = Expr::Literal(Literal::Float(2.0));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: &left,
        right: &right,
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
}

#[test]
fn infer_expr_add_string_int_is_string() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let s = interner.intern("s");
    env.register(s, LogosType::String);
    let left = Expr::Identifier(s);
    let right = Expr::Literal(Literal::Number(42));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: &left,
        right: &right,
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::String);
}

#[test]
fn infer_expr_comparison_is_bool() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let left = Expr::Literal(Literal::Number(1));
    let right = Expr::Literal(Literal::Number(2));
    let expr = Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: &left,
        right: &right,
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Bool);
}

#[test]
fn infer_expr_length_is_int() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let inner = Expr::Literal(Literal::Text(Symbol::EMPTY));
    let expr = Expr::Length { collection: &inner };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
}

#[test]
fn infer_expr_call_sqrt_is_float() {
    let env = TypeEnv::new();
    let mut interner = Interner::new();
    let sqrt = interner.intern("sqrt");
    let arg = Expr::Literal(Literal::Number(4));
    let expr = Expr::Call {
        function: sqrt,
        args: vec![&arg],
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
}

#[test]
fn infer_expr_call_parseint_is_int() {
    let env = TypeEnv::new();
    let mut interner = Interner::new();
    let pi = interner.intern("parseInt");
    let arg = Expr::Literal(Literal::Text(Symbol::EMPTY));
    let expr = Expr::Call {
        function: pi,
        args: vec![&arg],
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
}

#[test]
fn infer_expr_call_user_function() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let f = interner.intern("compute");
    env.register_fn(
        f,
        FnSig {
            params: vec![],
            return_type: LogosType::Float,
        },
    );
    let expr = Expr::Call {
        function: f,
        args: vec![],
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
}

#[test]
fn infer_expr_index_vec() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let items = interner.intern("items");
    env.register(items, LogosType::Seq(Box::new(LogosType::Int)));
    let coll = Expr::Identifier(items);
    let idx = Expr::Literal(Literal::Number(1));
    let expr = Expr::Index {
        collection: &coll,
        index: &idx,
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Int);
}

#[test]
fn infer_expr_index_map() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let data = interner.intern("data");
    env.register(
        data,
        LogosType::Map(Box::new(LogosType::String), Box::new(LogosType::Float)),
    );
    let coll = Expr::Identifier(data);
    let key = Expr::Literal(Literal::Text(Symbol::EMPTY));
    let expr = Expr::Index {
        collection: &coll,
        index: &key,
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Float);
}

#[test]
fn infer_expr_list_literal() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let a = Expr::Literal(Literal::Number(1));
    let b = Expr::Literal(Literal::Number(2));
    let expr = Expr::List(vec![&a, &b]);
    assert_eq!(
        env.infer_expr(&expr, &interner),
        LogosType::Seq(Box::new(LogosType::Int))
    );
}

#[test]
fn infer_expr_option_some() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let inner = Expr::Literal(Literal::Number(42));
    let expr = Expr::OptionSome { value: &inner };
    assert_eq!(
        env.infer_expr(&expr, &interner),
        LogosType::Option(Box::new(LogosType::Int))
    );
}

#[test]
fn infer_expr_contains_is_bool() {
    let env = TypeEnv::new();
    let interner = Interner::new();
    let coll = Expr::Literal(Literal::Text(Symbol::EMPTY));
    let val = Expr::Literal(Literal::Number(1));
    let expr = Expr::Contains {
        collection: &coll,
        value: &val,
    };
    assert_eq!(env.infer_expr(&expr, &interner), LogosType::Bool);
}

// =========================================================================
// TypeEnv::to_legacy_variable_types / to_legacy_string_vars
// =========================================================================

#[test]
fn legacy_variable_types_conversion() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let y = interner.intern("y");
    let z = interner.intern("z");
    env.register(x, LogosType::Int);
    env.register(y, LogosType::Seq(Box::new(LogosType::Float)));
    env.register(z, LogosType::Unknown); // Should be filtered out
    let legacy = env.to_legacy_variable_types();
    assert_eq!(legacy.get(&x).unwrap(), "i64");
    assert_eq!(legacy.get(&y).unwrap(), "LogosSeq<f64>");
    assert!(!legacy.contains_key(&z));
}

#[test]
fn legacy_string_vars_conversion() {
    let mut env = TypeEnv::new();
    let mut interner = Interner::new();
    let s = interner.intern("s");
    let n = interner.intern("n");
    env.register(s, LogosType::String);
    env.register(n, LogosType::Int);
    let string_vars = env.to_legacy_string_vars();
    assert!(string_vars.contains(&s));
    assert!(!string_vars.contains(&n));
}

// =========================================================================
// TypeEnv::infer_program — integration tests via compile pipeline
// =========================================================================

#[test]
fn infer_program_let_literal() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let val = Expr::Literal(Literal::Number(42));
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let registry = TypeRegistry::new();
    let env = TypeEnv::infer_program(&stmts, &interner, &registry);
    assert_eq!(env.lookup(x), &LogosType::Int);
}

#[test]
fn infer_program_let_with_type_annotation() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let float_sym = interner.intern("Real");
    let val = Expr::Literal(Literal::Number(5));
    let ty = TypeExpr::Primitive(float_sym);
    let stmts = [Stmt::Let {
        var: x,
        ty: Some(&ty),
        value: &val,
        mutable: false,
    }];
    let registry = TypeRegistry::new();
    let env = TypeEnv::infer_program(&stmts, &interner, &registry);
    assert_eq!(env.lookup(x), &LogosType::Float);
}

#[test]
fn infer_program_let_string_tracked() {
    let mut interner = Interner::new();
    let s = interner.intern("name");
    let hello = interner.intern("hello");
    let val = Expr::Literal(Literal::Text(hello));
    let stmts = [Stmt::Let {
        var: s,
        ty: None,
        value: &val,
        mutable: false,
    }];
    let registry = TypeRegistry::new();
    let env = TypeEnv::infer_program(&stmts, &interner, &registry);
    assert!(env.lookup(s).is_string());
    let string_vars = env.to_legacy_string_vars();
    assert!(string_vars.contains(&s));
}

#[test]
fn infer_program_readfrom_is_string() {
    let mut interner = Interner::new();
    let input = interner.intern("input");
    let stmts = [Stmt::ReadFrom {
        var: input,
        source: crate::ast::stmt::ReadSource::Console,
    }];
    let registry = TypeRegistry::new();
    let env = TypeEnv::infer_program(&stmts, &interner, &registry);
    assert_eq!(env.lookup(input), &LogosType::String);
}

// =========================================================================
// RustNames tests
// =========================================================================

#[test]
fn rust_names_escapes_keywords() {
    let mut interner = Interner::new();
    let r#move = interner.intern("move");
    let names = RustNames::new(&interner);
    assert_eq!(names.ident(r#move), "r#move");
}

#[test]
fn rust_names_preserves_non_keywords() {
    let mut interner = Interner::new();
    let foo = interner.intern("foo");
    let names = RustNames::new(&interner);
    assert_eq!(names.ident(foo), "foo");
}

#[test]
fn rust_names_raw_returns_original() {
    let mut interner = Interner::new();
    let r#move = interner.intern("move");
    let names = RustNames::new(&interner);
    assert_eq!(names.raw(r#move), "move");
}

// =========================================================================
// Phase 5: LogosType::Function
// =========================================================================

#[test]
fn function_type_is_not_copy() {
    let fn_ty = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    assert!(!fn_ty.is_copy());
}

#[test]
fn function_type_zero_params_is_not_copy() {
    let fn_ty = LogosType::Function(vec![], Box::new(LogosType::Unit));
    assert!(!fn_ty.is_copy());
}

#[test]
fn to_rust_type_function_single_param() {
    let fn_ty = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    assert_eq!(fn_ty.to_rust_type(), "impl Fn(i64) -> bool");
}

#[test]
fn to_rust_type_function_two_params() {
    let fn_ty = LogosType::Function(
        vec![LogosType::Int, LogosType::String],
        Box::new(LogosType::Bool),
    );
    assert_eq!(fn_ty.to_rust_type(), "impl Fn(i64, String) -> bool");
}

#[test]
fn to_rust_type_function_zero_params() {
    let fn_ty = LogosType::Function(vec![], Box::new(LogosType::Int));
    assert_eq!(fn_ty.to_rust_type(), "impl Fn() -> i64");
}

#[test]
fn to_rust_type_function_nested_param() {
    // fn(fn(Int) -> Bool) -> String
    let inner_fn = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    let fn_ty = LogosType::Function(vec![inner_fn], Box::new(LogosType::String));
    assert_eq!(
        fn_ty.to_rust_type(),
        "impl Fn(impl Fn(i64) -> bool) -> String"
    );
}

#[test]
fn from_type_expr_function_one_param() {
    use crate::ast::stmt::TypeExpr;
    let mut interner = Interner::new();
    let int_sym = interner.intern("Int");
    let bool_sym = interner.intern("Bool");
    let int_ty = TypeExpr::Primitive(int_sym);
    let bool_ty = TypeExpr::Primitive(bool_sym);
    let fn_ty = TypeExpr::Function {
        inputs: std::slice::from_ref(&int_ty),
        output: &bool_ty,
    };
    assert_eq!(
        LogosType::from_type_expr(&fn_ty, &interner),
        LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool))
    );
}

#[test]
fn from_type_expr_function_zero_params() {
    use crate::ast::stmt::TypeExpr;
    let mut interner = Interner::new();
    let unit_sym = interner.intern("Unit");
    let unit_ty = TypeExpr::Primitive(unit_sym);
    let fn_ty = TypeExpr::Function {
        inputs: &[],
        output: &unit_ty,
    };
    assert_eq!(
        LogosType::from_type_expr(&fn_ty, &interner),
        LogosType::Function(vec![], Box::new(LogosType::Unit))
    );
}

#[test]
fn from_type_expr_function_two_params() {
    use crate::ast::stmt::TypeExpr;
    let mut interner = Interner::new();
    let int_sym = interner.intern("Int");
    let text_sym = interner.intern("Text");
    let bool_sym = interner.intern("Bool");
    let int_ty = TypeExpr::Primitive(int_sym);
    let text_ty = TypeExpr::Primitive(text_sym);
    let bool_ty = TypeExpr::Primitive(bool_sym);
    let inputs = [int_ty, text_ty];
    let fn_ty = TypeExpr::Function {
        inputs: &inputs,
        output: &bool_ty,
    };
    assert_eq!(
        LogosType::from_type_expr(&fn_ty, &interner),
        LogosType::Function(
            vec![LogosType::Int, LogosType::String],
            Box::new(LogosType::Bool)
        )
    );
}

#[test]
fn from_type_expr_function_nested() {
    // fn(fn(Int) -> Bool) -> Text should parse correctly
    use crate::ast::stmt::TypeExpr;
    let mut interner = Interner::new();
    let int_sym = interner.intern("Int");
    let bool_sym = interner.intern("Bool");
    let text_sym = interner.intern("Text");
    let int_ty = TypeExpr::Primitive(int_sym);
    let bool_ty = TypeExpr::Primitive(bool_sym);
    let text_ty = TypeExpr::Primitive(text_sym);
    let inner_fn = TypeExpr::Function {
        inputs: std::slice::from_ref(&int_ty),
        output: &bool_ty,
    };
    let outer_fn = TypeExpr::Function {
        inputs: std::slice::from_ref(&inner_fn),
        output: &text_ty,
    };
    let inner_logos = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    let expected = LogosType::Function(vec![inner_logos], Box::new(LogosType::String));
    assert_eq!(LogosType::from_type_expr(&outer_fn, &interner), expected);
}

#[test]
fn function_type_equality_same() {
    let a = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    let b = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    assert_eq!(a, b);
}

#[test]
fn function_type_equality_different_params() {
    let a = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    let b = LogosType::Function(vec![LogosType::String], Box::new(LogosType::Bool));
    assert_ne!(a, b);
}

#[test]
fn function_type_equality_different_return() {
    let a = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    let b = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::String));
    assert_ne!(a, b);
}

#[test]
fn function_type_not_numeric_not_string_not_float() {
    let fn_ty = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    assert!(!fn_ty.is_numeric());
    assert!(!fn_ty.is_string());
    assert!(!fn_ty.is_float());
}

#[test]
fn function_type_element_type_is_none() {
    let fn_ty = LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool));
    assert!(fn_ty.element_type().is_none());
    assert!(fn_ty.key_type().is_none());
    assert!(fn_ty.value_type().is_none());
}
