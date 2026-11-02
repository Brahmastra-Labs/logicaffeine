use super::*;
use crate::ast::stmt::{Literal, TypeExpr};

fn check(stmts: &[Stmt], interner: &Interner) -> Result<(), TypeError> {
    check_unknown_calls(stmts, interner, &TypeRegistry::new())
}

#[test]
fn unknown_call_is_rejected_with_the_name() {
    let mut interner = Interner::new();
    let ghost = interner.intern("readText");
    let call = Expr::Call {
        function: ghost,
        args: vec![],
    };
    let x = interner.intern("x");
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &call,
        mutable: false,
    }];
    match check(&stmts, &interner) {
        Err(TypeError::UnknownFunction { name }) => assert_eq!(name, ghost),
        other => panic!("expected UnknownFunction, got {other:?}"),
    }
}

#[test]
fn forward_referenced_function_is_known() {
    let mut interner = Interner::new();
    let f = interner.intern("later");
    let call = Expr::Call {
        function: f,
        args: vec![],
    };
    let x = interner.intern("x");
    let lit = Expr::Literal(Literal::Number(1));
    let ret = Stmt::Return { value: Some(&lit) };
    let body = [ret];
    let stmts = [
        Stmt::Let {
            var: x,
            ty: None,
            value: &call,
            mutable: false,
        },
        Stmt::FunctionDef {
            name: f,
            generics: vec![],
            params: vec![],
            body: &body,
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: Default::default(),
        },
    ];
    assert!(check(&stmts, &interner).is_ok());
}

#[test]
fn builtins_and_intercepts_are_known() {
    let mut interner = Interner::new();
    let x = interner.intern("x");
    let lit = Expr::Literal(Literal::Number(1));
    // `upper` and friends are PRELUDE `.lg` functions, not builtins — they
    // resolve through the spliced FunctionDefs (integration-locked in
    // unknown_call_lock.rs), so they are deliberately absent here.
    for name in ["sqrt", "length", "mapOf", "show", "args", "loaded"] {
        let sym = interner.intern(name);
        let call = Expr::Call {
            function: sym,
            args: vec![&lit],
        };
        let stmts = [Stmt::Let {
            var: x,
            ty: None,
            value: &call,
            mutable: false,
        }];
        assert!(
            check(&stmts, &interner).is_ok(),
            "`{name}` must be a known callee"
        );
    }
}

#[test]
fn bound_variables_are_callable_by_name() {
    let mut interner = Interner::new();
    let f = interner.intern("f");
    let n = interner.intern("n");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);
    let n_ref = Expr::Identifier(n);
    let closure = Expr::Closure {
        params: vec![(n, &int_ty)],
        body: ClosureBody::Expression(&n_ref),
        return_type: None,
    };
    let call = Expr::Call {
        function: f,
        args: vec![],
    };
    let y = interner.intern("y");
    let stmts = [
        Stmt::Let {
            var: f,
            ty: None,
            value: &closure,
            mutable: false,
        },
        Stmt::Let {
            var: y,
            ty: None,
            value: &call,
            mutable: false,
        },
    ];
    assert!(check(&stmts, &interner).is_ok());
}

#[test]
fn variant_constructors_are_known() {
    use crate::analysis::VariantDef;
    let mut interner = Interner::new();
    let maybe = interner.intern("Maybe");
    let just = interner.intern("Just");
    let mut registry = TypeRegistry::new();
    registry.register(
        maybe,
        TypeDef::Enum {
            variants: vec![VariantDef {
                name: just,
                fields: vec![],
            }],
            generics: vec![],
            is_portable: false,
            is_shared: false,
        },
    );
    let lit = Expr::Literal(Literal::Number(1));
    let call = Expr::Call {
        function: just,
        args: vec![&lit],
    };
    let x = interner.intern("x");
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &call,
        mutable: false,
    }];
    assert!(check_unknown_calls(&stmts, &interner, &registry).is_ok());
}

#[test]
fn proven_module_exports_harvests_every_pub_fn() {
    let module = "pub mod proven {\n\
                      \x20   pub enum MyNat { MZero, MSucc(Box<MyNat>) }\n\
                      \x20   pub const MZero: MyNat = MyNat::MZero;\n\
                      \x20   #[allow(non_snake_case)]\n\
                      \x20   pub fn MSucc(n: MyNat) -> MyNat { MyNat::MSucc(Box::new(n)) }\n\
                      \x20   pub fn my_add(n: MyNat, m: MyNat) -> MyNat { m }\n\
                      \x20   pub fn check_positive(n: i64) -> bool { n > 0 }\n\
                      \x20   fn private_helper() {}\n\
                      }\n";
    let exports = proven_module_exports(module);
    assert_eq!(
        exports,
        ["MSucc", "my_add", "check_positive"]
            .into_iter()
            .map(String::from)
            .collect(),
        "every `pub fn` and nothing else (consts and private fns are not callable-by-name)"
    );
}

#[test]
fn a_proven_export_is_a_known_callee() {
    let mut interner = Interner::new();
    let ctor = interner.intern("MSucc");
    let lit = Expr::Literal(Literal::Number(1));
    let call = Expr::Call {
        function: ctor,
        args: vec![&lit],
    };
    let x = interner.intern("x");
    let stmts = [Stmt::Let {
        var: x,
        ty: None,
        value: &call,
        mutable: false,
    }];
    assert!(
        matches!(
            check(&stmts, &interner),
            Err(TypeError::UnknownFunction { .. })
        ),
        "without the export the constructor is unknown"
    );
    let exports = proven_module_exports("pub fn MSucc(n: MyNat) -> MyNat { todo!() }\n");
    assert!(
        check_unknown_calls_with_exports(&stmts, &interner, &TypeRegistry::new(), &exports).is_ok(),
        "the bundled proven export makes the same call known"
    );
}

#[test]
fn the_harvest_records_call_args_and_value_references() {
    let mut interner = Interner::new();
    let f = interner.intern("f");
    let g = interner.intern("g");
    let x = interner.intern("x");
    let x_ref = Expr::Identifier(x);
    let lit = Expr::Literal(Literal::Number(7));
    let call = Expr::Call {
        function: f,
        args: vec![&x_ref, &lit],
    };
    let y = interner.intern("y");
    let stmts = [
        Stmt::Let {
            var: y,
            ty: None,
            value: &call,
            mutable: false,
        },
        Stmt::Call {
            function: g,
            args: vec![],
        },
    ];
    let walk = harvest_calls(&stmts);
    assert_eq!(walk.calls.len(), 2);
    assert_eq!(walk.calls[0].function, f);
    assert_eq!(walk.calls[0].args.len(), 2);
    assert_eq!(walk.calls[1].function, g);
    assert!(walk.calls[1].args.is_empty());
    assert!(
        walk.value_refs.contains(&x) && !walk.value_refs.contains(&f),
        "a call-position name is not a value reference; its identifier args are"
    );
}

#[test]
fn unknown_call_in_a_branch_is_still_rejected() {
    let mut interner = Interner::new();
    let ghost = interner.intern("ghost");
    let call = Expr::Call {
        function: ghost,
        args: vec![],
    };
    let t = interner.intern("t");
    let cond = Expr::Literal(Literal::Boolean(false));
    let inner = [Stmt::Let {
        var: t,
        ty: None,
        value: &call,
        mutable: false,
    }];
    let stmts = [Stmt::If {
        cond: &cond,
        then_block: &inner,
        else_block: None,
    }];
    assert!(matches!(
        check(&stmts, &interner),
        Err(TypeError::UnknownFunction { .. })
    ));
}
