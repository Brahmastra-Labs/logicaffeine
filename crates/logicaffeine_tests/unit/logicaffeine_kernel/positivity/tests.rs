use super::*;

#[test]
fn test_simple_recursive_arg() {
    // Nat -> Nat is valid (first Nat is direct recursive arg, second is result)
    let ty = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Nat".to_string())),
        body_type: Box::new(Term::Global("Nat".to_string())),
    };
    assert!(check_positivity("Nat", "Succ", &ty).is_ok());
}

#[test]
fn test_negative_inside_arrow() {
    // (Bad -> False) -> Bad has Bad inside an arrow within a param
    // Bad occurs in param_type `Bad -> False`, which is not directly Bad
    let bad_to_false = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Bad".to_string())),
        body_type: Box::new(Term::Global("False".to_string())),
    };
    let ty = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(bad_to_false),
        body_type: Box::new(Term::Global("Bad".to_string())),
    };
    assert!(check_positivity("Bad", "Cons", &ty).is_err());
}

#[test]
fn test_nested_negative() {
    // ((Tricky -> Nat) -> Nat) -> Tricky
    // Tricky appears inside the param type of the outer Pi
    // The outer param is ((Tricky -> Nat) -> Nat), which contains Tricky
    let tricky_to_nat = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Tricky".to_string())),
        body_type: Box::new(Term::Global("Nat".to_string())),
    };
    let inner = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(tricky_to_nat),
        body_type: Box::new(Term::Global("Nat".to_string())),
    };
    let make_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(inner),
        body_type: Box::new(Term::Global("Tricky".to_string())),
    };

    let result = check_positivity("Tricky", "Make", &make_type);
    assert!(
        result.is_err(),
        "Should reject nested negative: {:?}",
        result
    );
}

#[test]
fn test_list_cons_valid() {
    // Cons : Nat -> List -> List
    // Both params are fine: Nat doesn't contain List, second param IS List directly
    let ty = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("Nat".to_string())),
        body_type: Box::new(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::Global("List".to_string())),
            body_type: Box::new(Term::Global("List".to_string())),
        }),
    };
    assert!(check_positivity("List", "Cons", &ty).is_ok());
}
