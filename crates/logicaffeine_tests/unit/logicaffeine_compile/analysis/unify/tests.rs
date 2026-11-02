use super::*;

// =========================================================================
// UnificationTable::fresh / find
// =========================================================================

#[test]
fn fresh_produces_distinct_vars() {
    let mut table = UnificationTable::new();
    let a = table.fresh();
    let b = table.fresh();
    assert_ne!(a, b);
}

#[test]
fn unbound_var_finds_itself() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        assert_eq!(table.find(tv), InferType::Var(tv));
    } else {
        panic!("expected Var");
    }
}

// =========================================================================
// Unify ground types
// =========================================================================

#[test]
fn unify_identical_ground_types() {
    let mut table = UnificationTable::new();
    assert!(table.unify(&InferType::Int, &InferType::Int).is_ok());
    assert!(table.unify(&InferType::Float, &InferType::Float).is_ok());
    assert!(table.unify(&InferType::Bool, &InferType::Bool).is_ok());
    assert!(table.unify(&InferType::String, &InferType::String).is_ok());
    assert!(table.unify(&InferType::Unit, &InferType::Unit).is_ok());
}

#[test]
fn unify_different_ground_types_fails() {
    let mut table = UnificationTable::new();
    let result = table.unify(&InferType::Int, &InferType::String);
    assert!(result.is_err());
    assert!(matches!(result, Err(TypeError::Mismatch { .. })));
}

#[test]
fn unify_int_float_fails() {
    let mut table = UnificationTable::new();
    let result = table.unify(&InferType::Int, &InferType::Float);
    assert!(result.is_err());
}

#[test]
fn unify_nat_int_succeeds() {
    let mut table = UnificationTable::new();
    assert!(table.unify(&InferType::Nat, &InferType::Int).is_ok());
    assert!(table.unify(&InferType::Int, &InferType::Nat).is_ok());
}

#[test]
fn unify_unknown_with_any_succeeds() {
    let mut table = UnificationTable::new();
    assert!(table.unify(&InferType::Unknown, &InferType::Int).is_ok());
    assert!(table.unify(&InferType::String, &InferType::Unknown).is_ok());
    assert!(table
        .unify(&InferType::Unknown, &InferType::Unknown)
        .is_ok());
}

// =========================================================================
// Unify variables with ground types
// =========================================================================

#[test]
fn var_unifies_with_int() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        table.unify(&InferType::Var(tv), &InferType::Int).unwrap();
        assert_eq!(table.find(tv), InferType::Int);
    }
}

#[test]
fn int_unifies_with_var() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        table.unify(&InferType::Int, &InferType::Var(tv)).unwrap();
        assert_eq!(table.find(tv), InferType::Int);
    }
}

#[test]
fn two_vars_unify_chain() {
    let mut table = UnificationTable::new();
    let va = table.fresh();
    let vb = table.fresh();
    let tva = if let InferType::Var(tv) = va {
        tv
    } else {
        panic!()
    };
    let tvb = if let InferType::Var(tv) = vb {
        tv
    } else {
        panic!()
    };
    table
        .unify(&InferType::Var(tva), &InferType::Var(tvb))
        .unwrap();
    // Bind tvb to Int, then zonk(tva) should follow chain to Int
    table.unify(&InferType::Var(tvb), &InferType::Int).unwrap();
    let zonked = table.zonk(&InferType::Var(tva));
    assert_eq!(zonked, InferType::Int);
}

#[test]
fn var_conflicting_types_fails() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        table.unify(&InferType::Var(tv), &InferType::Int).unwrap();
        let result = table.unify(&InferType::Var(tv), &InferType::String);
        // After binding tv → Int, unifying Int with String fails
        assert!(result.is_err());
    }
}

// =========================================================================
// Occurs check
// =========================================================================

#[test]
fn occurs_check_detects_infinite_type() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        let circular = InferType::Seq(Box::new(InferType::Var(tv)));
        let result = table.unify(&InferType::Var(tv), &circular);
        assert!(result.is_err());
        assert!(matches!(result, Err(TypeError::InfiniteType { .. })));
    }
}

// =========================================================================
// Zonk
// =========================================================================

#[test]
fn zonk_resolves_bound_var() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        table.unify(&InferType::Var(tv), &InferType::Bool).unwrap();
        let zonked = table.zonk(&InferType::Var(tv));
        assert_eq!(zonked, InferType::Bool);
    }
}

#[test]
fn zonk_unbound_var_becomes_unknown() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        let zonked = table.zonk(&InferType::Var(tv));
        assert_eq!(zonked, InferType::Unknown);
    }
}

#[test]
fn zonk_nested_resolves_inner_var() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        table.unify(&InferType::Var(tv), &InferType::Int).unwrap();
        let ty = InferType::Seq(Box::new(InferType::Var(tv)));
        let zonked = table.zonk(&ty);
        assert_eq!(zonked, InferType::Seq(Box::new(InferType::Int)));
    }
}

#[test]
fn zonk_chain_of_vars() {
    let mut table = UnificationTable::new();
    let tva = if let InferType::Var(tv) = table.fresh() {
        tv
    } else {
        panic!()
    };
    let tvb = if let InferType::Var(tv) = table.fresh() {
        tv
    } else {
        panic!()
    };
    let tvc = if let InferType::Var(tv) = table.fresh() {
        tv
    } else {
        panic!()
    };
    // Chain: tva → tvb → tvc → Float
    table
        .unify(&InferType::Var(tva), &InferType::Var(tvb))
        .unwrap();
    table
        .unify(&InferType::Var(tvb), &InferType::Var(tvc))
        .unwrap();
    table
        .unify(&InferType::Var(tvc), &InferType::Float)
        .unwrap();
    assert_eq!(table.zonk(&InferType::Var(tva)), InferType::Float);
}

// =========================================================================
// Nested generic unification
// =========================================================================

#[test]
fn unify_seq_of_same_type() {
    let mut table = UnificationTable::new();
    let a = InferType::Seq(Box::new(InferType::Int));
    let b = InferType::Seq(Box::new(InferType::Int));
    assert!(table.unify(&a, &b).is_ok());
}

#[test]
fn unify_seq_of_different_types_fails() {
    let mut table = UnificationTable::new();
    let a = InferType::Seq(Box::new(InferType::Int));
    let b = InferType::Seq(Box::new(InferType::String));
    assert!(table.unify(&a, &b).is_err());
}

#[test]
fn unify_seq_with_var_element() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        let a = InferType::Seq(Box::new(InferType::Var(tv)));
        let b = InferType::Seq(Box::new(InferType::Float));
        table.unify(&a, &b).unwrap();
        assert_eq!(table.find(tv), InferType::Float);
    }
}

#[test]
fn unify_map_types() {
    let mut table = UnificationTable::new();
    let a = InferType::Map(Box::new(InferType::String), Box::new(InferType::Int));
    let b = InferType::Map(Box::new(InferType::String), Box::new(InferType::Int));
    assert!(table.unify(&a, &b).is_ok());
}

#[test]
fn unify_function_types_same_arity() {
    let mut table = UnificationTable::new();
    let a = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
    let b = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
    assert!(table.unify(&a, &b).is_ok());
}

#[test]
fn unify_function_arity_mismatch_fails() {
    let mut table = UnificationTable::new();
    let a = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
    let b = InferType::Function(
        vec![InferType::Int, InferType::Int],
        Box::new(InferType::Bool),
    );
    let result = table.unify(&a, &b);
    assert!(matches!(
        result,
        Err(TypeError::ArityMismatch {
            expected: 1,
            found: 2
        })
    ));
}

// =========================================================================
// InferType → LogosType conversion (to_logos_type)
// =========================================================================

#[test]
fn to_logos_type_ground_primitives() {
    let table = UnificationTable::new();
    assert_eq!(table.to_logos_type(&InferType::Int), LogosType::Int);
    assert_eq!(table.to_logos_type(&InferType::Float), LogosType::Float);
    assert_eq!(table.to_logos_type(&InferType::Bool), LogosType::Bool);
    assert_eq!(table.to_logos_type(&InferType::String), LogosType::String);
    assert_eq!(table.to_logos_type(&InferType::Unit), LogosType::Unit);
    assert_eq!(table.to_logos_type(&InferType::Nat), LogosType::Nat);
}

#[test]
fn to_logos_type_unbound_var_becomes_unknown() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    assert_eq!(table.to_logos_type(&v), LogosType::Unknown);
}

#[test]
fn to_logos_type_bound_var_resolves() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        table.unify(&InferType::Var(tv), &InferType::Int).unwrap();
        assert_eq!(table.to_logos_type(&InferType::Var(tv)), LogosType::Int);
    }
}

#[test]
fn to_logos_type_seq_resolves_inner() {
    let mut table = UnificationTable::new();
    let v = table.fresh();
    if let InferType::Var(tv) = v {
        table
            .unify(&InferType::Var(tv), &InferType::String)
            .unwrap();
        let ty = InferType::Seq(Box::new(InferType::Var(tv)));
        assert_eq!(
            table.to_logos_type(&ty),
            LogosType::Seq(Box::new(LogosType::String))
        );
    }
}

#[test]
fn to_logos_type_function_converts_to_logos_function() {
    let table = UnificationTable::new();
    let ty = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
    assert_eq!(
        table.to_logos_type(&ty),
        LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool))
    );
}

#[test]
fn to_logos_type_function_two_params_converts() {
    let table = UnificationTable::new();
    let ty = InferType::Function(
        vec![InferType::Int, InferType::String],
        Box::new(InferType::Bool),
    );
    assert_eq!(
        table.to_logos_type(&ty),
        LogosType::Function(
            vec![LogosType::Int, LogosType::String],
            Box::new(LogosType::Bool)
        )
    );
}

#[test]
fn to_logos_type_function_zero_params_converts() {
    let table = UnificationTable::new();
    let ty = InferType::Function(vec![], Box::new(InferType::Unit));
    assert_eq!(
        table.to_logos_type(&ty),
        LogosType::Function(vec![], Box::new(LogosType::Unit))
    );
}

#[test]
fn to_logos_type_function_nested_converts() {
    // fn(fn(Int) -> Bool) -> String
    let table = UnificationTable::new();
    let inner = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
    let outer = InferType::Function(vec![inner], Box::new(InferType::String));
    assert_eq!(
        table.to_logos_type(&outer),
        LogosType::Function(
            vec![LogosType::Function(
                vec![LogosType::Int],
                Box::new(LogosType::Bool)
            )],
            Box::new(LogosType::String)
        )
    );
}

// =========================================================================
// from_type_expr
// =========================================================================

#[test]
fn from_type_expr_function_produces_function_type() {
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
    let result = InferType::from_type_expr(&fn_ty, &interner);
    assert_eq!(
        result,
        InferType::Function(vec![InferType::Int], Box::new(InferType::Bool))
    );
}

#[test]
fn from_type_expr_seq_of_int() {
    use crate::ast::stmt::TypeExpr;
    let mut interner = Interner::new();
    let seq_sym = interner.intern("Seq");
    let int_sym = interner.intern("Int");
    let int_ty = TypeExpr::Primitive(int_sym);
    let ty = TypeExpr::Generic {
        base: seq_sym,
        params: std::slice::from_ref(&int_ty),
    };
    let result = InferType::from_type_expr(&ty, &interner);
    assert_eq!(result, InferType::Seq(Box::new(InferType::Int)));
}

// =========================================================================
// from_field_type with TypeParam
// =========================================================================

#[test]
fn from_field_type_type_param_resolves_to_var() {
    let mut interner = Interner::new();
    let t_sym = interner.intern("T");
    let tv = TyVar(0);
    let mut type_params = HashMap::new();
    type_params.insert(t_sym, tv);

    let field_ty = FieldType::TypeParam(t_sym);
    let result = InferType::from_field_type(&field_ty, &interner, &type_params);
    assert_eq!(result, InferType::Var(tv));
}

#[test]
fn from_field_type_missing_type_param_becomes_unknown() {
    let mut interner = Interner::new();
    let t_sym = interner.intern("T");
    let type_params = HashMap::new();
    let field_ty = FieldType::TypeParam(t_sym);
    let result = InferType::from_field_type(&field_ty, &interner, &type_params);
    assert_eq!(result, InferType::Unknown);
}

#[test]
fn from_field_type_primitive() {
    let mut interner = Interner::new();
    let int_sym = interner.intern("Int");
    let field_ty = FieldType::Primitive(int_sym);
    let result = InferType::from_field_type(&field_ty, &interner, &HashMap::new());
    assert_eq!(result, InferType::Int);
}

#[test]
fn from_field_type_generic_seq_of_type_param() {
    let mut interner = Interner::new();
    let seq_sym = interner.intern("Seq");
    let t_sym = interner.intern("T");
    let tv = TyVar(0);
    let mut type_params = HashMap::new();
    type_params.insert(t_sym, tv);

    let field_ty = FieldType::Generic {
        base: seq_sym,
        params: vec![FieldType::TypeParam(t_sym)],
    };
    let result = InferType::from_field_type(&field_ty, &interner, &type_params);
    assert_eq!(result, InferType::Seq(Box::new(InferType::Var(tv))));
}

// =========================================================================
// unify_numeric helper
// =========================================================================

#[test]
fn numeric_float_wins() {
    assert_eq!(
        unify_numeric(&InferType::Int, &InferType::Float).unwrap(),
        InferType::Float
    );
    assert_eq!(
        unify_numeric(&InferType::Float, &InferType::Int).unwrap(),
        InferType::Float
    );
}

#[test]
fn numeric_int_plus_int_is_int() {
    assert_eq!(
        unify_numeric(&InferType::Int, &InferType::Int).unwrap(),
        InferType::Int
    );
}

#[test]
fn numeric_nat_plus_int_is_int() {
    assert_eq!(
        unify_numeric(&InferType::Nat, &InferType::Int).unwrap(),
        InferType::Int
    );
}

#[test]
fn numeric_nat_plus_nat_is_nat() {
    assert_eq!(
        unify_numeric(&InferType::Nat, &InferType::Nat).unwrap(),
        InferType::Nat
    );
}

#[test]
fn numeric_string_fails() {
    let result = unify_numeric(&InferType::String, &InferType::Int);
    assert!(result.is_err());
}

// =========================================================================
// to_logos_name
// =========================================================================

#[test]
fn logos_name_primitives() {
    assert_eq!(InferType::Int.to_logos_name(), "Int");
    assert_eq!(InferType::Float.to_logos_name(), "Real");
    assert_eq!(InferType::String.to_logos_name(), "Text");
    assert_eq!(InferType::Bool.to_logos_name(), "Bool");
}

#[test]
fn logos_name_seq() {
    let ty = InferType::Seq(Box::new(InferType::Int));
    assert_eq!(ty.to_logos_name(), "Seq of Int");
}

#[test]
fn logos_name_function() {
    let ty = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
    assert_eq!(ty.to_logos_name(), "fn(Int) -> Bool");
}

// =========================================================================
// TypeError helpers
// =========================================================================

#[test]
fn type_error_mismatch_strings() {
    let err = TypeError::Mismatch {
        expected: InferType::Int,
        found: InferType::String,
    };
    assert_eq!(err.expected_str(), "Int");
    assert_eq!(err.found_str(), "Text");
}

#[test]
fn type_error_arity_mismatch_strings() {
    let err = TypeError::ArityMismatch {
        expected: 2,
        found: 3,
    };
    assert_eq!(err.expected_str(), "2 arguments");
    assert_eq!(err.found_str(), "3 arguments");
}

// =========================================================================
// Phase 5: infer_to_logos for Function types
// =========================================================================

#[test]
fn infer_to_logos_function_single_param() {
    let ty = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
    assert_eq!(
        super::infer_to_logos(&ty),
        LogosType::Function(vec![LogosType::Int], Box::new(LogosType::Bool))
    );
}

#[test]
fn infer_to_logos_function_zero_params() {
    let ty = InferType::Function(vec![], Box::new(InferType::Unit));
    assert_eq!(
        super::infer_to_logos(&ty),
        LogosType::Function(vec![], Box::new(LogosType::Unit))
    );
}

#[test]
fn infer_to_logos_function_two_params() {
    let ty = InferType::Function(
        vec![InferType::String, InferType::Float],
        Box::new(InferType::Bool),
    );
    assert_eq!(
        super::infer_to_logos(&ty),
        LogosType::Function(
            vec![LogosType::String, LogosType::Float],
            Box::new(LogosType::Bool)
        )
    );
}

#[test]
fn infer_to_logos_function_nested() {
    // fn(fn(Int) -> Bool) -> String
    let inner = InferType::Function(vec![InferType::Int], Box::new(InferType::Bool));
    let outer = InferType::Function(vec![inner], Box::new(InferType::String));
    assert_eq!(
        super::infer_to_logos(&outer),
        LogosType::Function(
            vec![LogosType::Function(
                vec![LogosType::Int],
                Box::new(LogosType::Bool)
            )],
            Box::new(LogosType::String)
        )
    );
}

// =========================================================================
// TypeError reporting
// =========================================================================

/// A mismatch between two user types names both — `expected Outcome, found Box of Int` — rather
/// than calling each "a user-defined type".
#[test]
fn a_mismatch_names_its_user_types() {
    let mut interner = Interner::new();
    let outcome = interner.intern("Outcome");
    let boxed = interner.intern("Box");
    let err = TypeError::Mismatch {
        expected: InferType::UserDefined(outcome, vec![]),
        found: InferType::UserDefined(boxed, vec![InferType::Int]),
    };
    match err.to_parse_error_kind(&interner) {
        crate::error::ParseErrorKind::TypeMismatchDetailed {
            expected, found, ..
        } => {
            assert_eq!(expected, "Outcome");
            assert_eq!(found, "Box of Int");
        }
        other => panic!("expected a detailed mismatch, got {other:?}"),
    }
    assert_eq!(
        InferType::Seq(Box::new(InferType::UserDefined(outcome, vec![]))).describe(&interner),
        "Seq of Outcome"
    );
    assert_eq!(
        InferType::UserDefined(outcome, vec![]).to_logos_name(),
        "a user-defined type"
    );
}
