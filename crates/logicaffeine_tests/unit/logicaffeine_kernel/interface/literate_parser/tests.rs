use super::*;

#[test]
fn test_parse_simple_inductive() {
    let cmd = parse_inductive("A Bool is either Yes or No").unwrap().0;
    if let Command::Inductive {
        name, constructors, ..
    } = cmd
    {
        assert_eq!(name, "Bool");
        assert_eq!(constructors.len(), 2);
        assert_eq!(constructors[0].0, "Yes");
        assert_eq!(constructors[1].0, "No");
    } else {
        panic!("Expected Inductive command");
    }
}

#[test]
fn test_parse_inductive_with_article() {
    let cmd = parse_inductive("A Decision is either a Yes or a No")
        .unwrap()
        .0;
    if let Command::Inductive {
        name, constructors, ..
    } = cmd
    {
        assert_eq!(name, "Decision");
        assert_eq!(constructors.len(), 2);
    } else {
        panic!("Expected Inductive command");
    }
}

#[test]
fn test_parse_recursive_inductive() {
    let cmd = parse_inductive("A Nat is either Zero or a Succ with pred: Nat")
        .unwrap()
        .0;
    if let Command::Inductive {
        name, constructors, ..
    } = cmd
    {
        assert_eq!(name, "Nat");
        assert_eq!(constructors.len(), 2);
        assert_eq!(constructors[0].0, "Zero");
        assert_eq!(constructors[1].0, "Succ");
        // Succ should have type Nat -> Nat
        if let Term::Pi { .. } = &constructors[1].1 {
            // Good - it's a function type
        } else {
            panic!("Expected Succ to have Pi type");
        }
    } else {
        panic!("Expected Inductive command");
    }
}

#[test]
fn test_parse_simple_definition() {
    let cmd = parse_definition("## To id (x: Nat) -> Nat: Yield x")
        .unwrap()
        .0;
    if let Command::Definition { name, body, .. } = cmd {
        assert_eq!(name, "id");
        // Body should be a lambda
        if let Term::Lambda { param, .. } = body {
            assert_eq!(param, "x");
        } else {
            panic!("Expected Lambda body");
        }
    } else {
        panic!("Expected Definition command");
    }
}

#[test]
fn test_implicit_fixpoint_detection() {
    // This definition is recursive (add appears in body)
    let cmd = parse_definition(
        "## To add (n: Nat) and (m: Nat) -> Nat: Consider n: When Zero: Yield m. When Succ k: Yield Succ (add k m)."
    ).unwrap().0;

    if let Command::Definition { name, body, .. } = cmd {
        assert_eq!(name, "add");
        // Body should be wrapped in Fix
        if let Term::Fix { name: fix_name, .. } = body {
            assert_eq!(fix_name, "add");
        } else {
            panic!("Expected Fix wrapper for recursive function");
        }
    } else {
        panic!("Expected Definition command");
    }
}

#[test]
fn test_parse_given_lambda() {
    let mut parser = LiterateParser::new("given x: Nat yields Succ x");
    let term = parser.parse_given_lambda().unwrap();

    if let Term::Lambda { param, .. } = term {
        assert_eq!(param, "x");
    } else {
        panic!("Expected Lambda");
    }
}

#[test]
fn test_parse_pipe_lambda() {
    let mut parser = LiterateParser::new("|x: Nat| -> Succ x");
    let term = parser.parse_pipe_lambda().unwrap();

    if let Term::Lambda { param, .. } = term {
        assert_eq!(param, "x");
    } else {
        panic!("Expected Lambda");
    }
}

// ============================================================
// Syntax (Deep Embedding) Tests
// ============================================================

#[test]
fn test_parse_let_definition() {
    let cmd = parse_let_definition("Let T be Zero").unwrap().0;
    if let Command::Definition { name, ty, body, .. } = cmd {
        assert_eq!(name, "T");
        assert!(ty.is_none());
        if let Term::Global(g) = body {
            assert_eq!(g, "Zero");
        } else {
            panic!("Expected Global term");
        }
    } else {
        panic!("Expected Definition command");
    }
}

#[test]
fn test_parse_name_syntax() {
    let mut parser = LiterateParser::new("Name \"Not\"");
    let term = parser.parse_term().unwrap();

    // Should be SName "Not"
    if let Term::App(f, arg) = term {
        if let Term::Global(g) = *f {
            assert_eq!(g, "SName");
        } else {
            panic!("Expected Global SName");
        }
        if let Term::Lit(Literal::Text(s)) = *arg {
            assert_eq!(s, "Not");
        } else {
            panic!("Expected Text literal");
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_variable_syntax() {
    let mut parser = LiterateParser::new("Variable 0");
    let term = parser.parse_term().unwrap();

    // Should be SVar 0
    if let Term::App(f, arg) = term {
        if let Term::Global(g) = *f {
            assert_eq!(g, "SVar");
        } else {
            panic!("Expected Global SVar");
        }
        if let Term::Lit(Literal::Int(n)) = *arg {
            assert_eq!(n, 0);
        } else {
            panic!("Expected Int literal");
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_apply_syntax() {
    let mut parser = LiterateParser::new("Apply(Name \"Not\", Variable 0)");
    let term = parser.parse_term().unwrap();

    // Should be (SApp (SName "Not") (SVar 0))
    if let Term::App(outer_f, _outer_arg) = term {
        if let Term::App(inner_f, _inner_arg) = *outer_f {
            if let Term::Global(g) = *inner_f {
                assert_eq!(g, "SApp");
            } else {
                panic!("Expected Global SApp");
            }
        } else {
            panic!("Expected inner App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_diagonalization() {
    let mut parser = LiterateParser::new("the diagonalization of T");
    let term = parser.parse_term().unwrap();

    // Should be syn_diag T
    if let Term::App(f, arg) = term {
        if let Term::Global(g) = *f {
            assert_eq!(g, "syn_diag");
        } else {
            panic!("Expected Global syn_diag");
        }
        if let Term::Global(a) = *arg {
            assert_eq!(a, "T");
        } else {
            panic!("Expected Global T");
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_implies() {
    let mut parser = LiterateParser::new("A implies B");
    let term = parser.parse_term().unwrap();

    // Should be forall _ : A, B (non-dependent Pi)
    if let Term::Pi {
        param,
        param_type,
        body_type,
    } = term
    {
        assert_eq!(param, "_");
        if let Term::Global(a) = *param_type {
            assert_eq!(a, "A");
        } else {
            panic!("Expected Global A");
        }
        if let Term::Global(b) = *body_type {
            assert_eq!(b, "B");
        } else {
            panic!("Expected Global B");
        }
    } else {
        panic!("Expected Pi");
    }
}

#[test]
fn test_parse_existential() {
    let mut parser = LiterateParser::new("there exists a d: Derivation such that P");
    let term = parser.parse_term().unwrap();

    // Should be Ex Derivation (fun d => P)
    if let Term::App(outer_f, lambda) = term {
        if let Term::App(ex, typ) = *outer_f {
            if let Term::Global(g) = *ex {
                assert_eq!(g, "Ex");
            } else {
                panic!("Expected Global Ex");
            }
            if let Term::Global(t) = *typ {
                assert_eq!(t, "Derivation");
            } else {
                panic!("Expected Global Derivation");
            }
        } else {
            panic!("Expected inner App for Ex");
        }
        if let Term::Lambda { param, body, .. } = *lambda {
            assert_eq!(param, "d");
            if let Term::Global(p) = *body {
                assert_eq!(p, "P");
            } else {
                panic!("Expected Global P in lambda body");
            }
        } else {
            panic!("Expected Lambda");
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_complex_let_with_apply() {
    let cmd =
        parse_let_definition("Let T be Apply(Name \"Not\", Apply(Name \"Provable\", Variable 0)).")
            .unwrap()
            .0;
    if let Command::Definition { name, ty, body, .. } = cmd {
        assert_eq!(name, "T");
        assert!(ty.is_none());
        // Body should be nested applications
        if let Term::App(_, _) = body {
            // Good - it's an application
        } else {
            panic!("Expected App body");
        }
    } else {
        panic!("Expected Definition command");
    }
}

// ============================================================
// Predicate Syntax Tests
// ============================================================

#[test]
fn test_parse_predicate_definition() {
    // ## To be Provable (s: Syntax) -> Prop: Yield s
    let cmd = parse_definition("## To be Provable (s: Syntax) -> Prop: Yield s")
        .unwrap()
        .0;
    if let Command::Definition { name, .. } = cmd {
        assert_eq!(name, "Provable"); // NOT "be"
    } else {
        panic!("Expected Definition command");
    }
}

#[test]
fn test_parse_nullary_predicate() {
    // ## To be Consistent -> Prop: Yield True
    let cmd = parse_definition("## To be Consistent -> Prop: Yield True")
        .unwrap()
        .0;
    if let Command::Definition { name, body, .. } = cmd {
        assert_eq!(name, "Consistent");
        // Body should be True (no lambda wrapper since no params)
        if let Term::Global(g) = body {
            assert_eq!(g, "True");
        } else {
            panic!("Expected Global True");
        }
    } else {
        panic!("Expected Definition command");
    }
}

#[test]
fn test_parse_the_name_syntax() {
    // the Name "Not" → SName "Not"
    let mut parser = LiterateParser::new("the Name \"Not\"");
    let term = parser.parse_term().unwrap();

    if let Term::App(f, arg) = term {
        if let Term::Global(g) = *f {
            assert_eq!(g, "SName");
        } else {
            panic!("Expected Global SName");
        }
        if let Term::Lit(Literal::Text(s)) = *arg {
            assert_eq!(s, "Not");
        } else {
            panic!("Expected Text literal");
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_theorem() {
    // ## Theorem: MyTheorem\n    Statement: True implies True.
    let cmd = parse_theorem("## Theorem: MyTheorem\n    Statement: A implies B.")
        .unwrap()
        .0;
    if let Command::Definition { name, ty, body, .. } = cmd {
        assert_eq!(name, "MyTheorem");
        // Type should be Prop
        assert!(ty.is_some());
        if let Some(Term::Sort(Universe::Prop)) = ty {
            // Good
        } else {
            panic!("Expected Prop type");
        }
        // Body should be Pi (A implies B)
        if let Term::Pi { .. } = body {
            // Good
        } else {
            panic!("Expected Pi body (implication)");
        }
    } else {
        panic!("Expected Definition command");
    }
}

#[test]
fn test_parse_theorem_with_complex_statement() {
    // ## Theorem: Godel\n    Statement: Consistent implies Not(Provable(G)).
    let cmd =
        parse_theorem("## Theorem: Godel\n    Statement: Consistent implies Not(Provable(G)).")
            .unwrap()
            .0;
    if let Command::Definition { name, ty, .. } = cmd {
        assert_eq!(name, "Godel");
        assert!(ty.is_some());
        if let Some(Term::Sort(Universe::Prop)) = ty {
            // Good
        } else {
            panic!("Expected Prop type");
        }
    } else {
        panic!("Expected Definition command");
    }
}

#[test]
fn test_parse_equals_infix() {
    // X equals Y → Eq Hole X Y
    let mut parser = LiterateParser::new("A equals B");
    let term = parser.parse_term().unwrap();

    // Should be (Eq Hole A B) = App(App(App(Eq, Hole), A), B)
    if let Term::App(outer, rhs) = term {
        if let Term::Global(b) = *rhs {
            assert_eq!(b, "B");
        } else {
            panic!("Expected Global B");
        }
        if let Term::App(mid, lhs) = *outer {
            if let Term::Global(a) = *lhs {
                assert_eq!(a, "A");
            } else {
                panic!("Expected Global A");
            }
            if let Term::App(inner, placeholder) = *mid {
                if let Term::Global(eq) = *inner {
                    assert_eq!(eq, "Eq");
                } else {
                    panic!("Expected Global Eq");
                }
                if !matches!(*placeholder, Term::Hole) {
                    panic!("Expected Hole placeholder");
                }
            } else {
                panic!("Expected inner App");
            }
        } else {
            panic!("Expected mid App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_equals_with_application() {
    // f(x) equals y → Eq _ (f x) y
    let mut parser = LiterateParser::new("f(x) equals y");
    let term = parser.parse_term().unwrap();

    // Should be App(App(App(Eq, _), App(f, x)), y)
    if let Term::App(outer, rhs) = term {
        if let Term::Global(y) = *rhs {
            assert_eq!(y, "y");
        } else {
            panic!("Expected Global y");
        }
        if let Term::App(_mid, lhs) = *outer {
            // lhs should be App(f, x)
            if let Term::App(f_box, x_box) = *lhs {
                if let Term::Global(f) = *f_box {
                    assert_eq!(f, "f");
                }
                if let Term::Global(x) = *x_box {
                    assert_eq!(x, "x");
                }
            } else {
                panic!("Expected lhs to be App(f, x)");
            }
        } else {
            panic!("Expected mid App");
        }
    } else {
        panic!("Expected outer App");
    }
}

// ============================================================
// INFIX OPERATOR TESTS
// ============================================================

#[test]
fn test_parse_infix_le() {
    let mut parser = LiterateParser::new("x <= y");
    let term = parser.parse_term().unwrap();
    // Should produce: App(App(Global("le"), Global("x")), Global("y"))
    if let Term::App(outer, rhs) = term {
        if let Term::App(inner, lhs) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "le");
            } else {
                panic!("Expected Global le");
            }
            if let Term::Global(l) = *lhs {
                assert_eq!(l, "x");
            } else {
                panic!("Expected Global x");
            }
        } else {
            panic!("Expected inner App");
        }
        if let Term::Global(r) = *rhs {
            assert_eq!(r, "y");
        } else {
            panic!("Expected Global y");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_infix_lt() {
    let mut parser = LiterateParser::new("a < b");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "lt");
            } else {
                panic!("Expected Global lt");
            }
        } else {
            panic!("Expected inner App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_infix_ge() {
    let mut parser = LiterateParser::new("x >= y");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "ge");
            } else {
                panic!("Expected Global ge");
            }
        } else {
            panic!("Expected inner App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_infix_gt() {
    let mut parser = LiterateParser::new("x > y");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "gt");
            } else {
                panic!("Expected Global gt");
            }
        } else {
            panic!("Expected inner App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_infix_add() {
    let mut parser = LiterateParser::new("x + y");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "add");
            } else {
                panic!("Expected Global add");
            }
        } else {
            panic!("Expected inner App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_infix_sub() {
    let mut parser = LiterateParser::new("x - y");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "sub");
            } else {
                panic!("Expected Global sub");
            }
        } else {
            panic!("Expected inner App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_infix_mul() {
    let mut parser = LiterateParser::new("x * y");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "mul");
            } else {
                panic!("Expected Global mul");
            }
        } else {
            panic!("Expected inner App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_precedence_mul_over_add() {
    // x + y * z should parse as add(x, mul(y, z))
    let mut parser = LiterateParser::new("x + y * z");
    let term = parser.parse_term().unwrap();
    // Outer should be add
    if let Term::App(outer, rhs) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "add");
            } else {
                panic!("Expected add");
            }
        } else {
            panic!("Expected inner App");
        }
        // rhs should be mul(y, z)
        if let Term::App(mul_outer, _) = *rhs {
            if let Term::App(mul_inner, _) = *mul_outer {
                if let Term::Global(op) = *mul_inner {
                    assert_eq!(op, "mul");
                } else {
                    panic!("Expected mul");
                }
            } else {
                panic!("Expected mul inner App");
            }
        } else {
            panic!("Expected mul App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_left_associative_add() {
    // x + y + z should parse as add(add(x, y), z)
    let mut parser = LiterateParser::new("x + y + z");
    let term = parser.parse_term().unwrap();
    // Outer add, lhs is add(x, y), rhs is z
    if let Term::App(outer, rhs) = term {
        if let Term::Global(z) = *rhs {
            assert_eq!(z, "z");
        } else {
            panic!("Expected z");
        }
        if let Term::App(mid, lhs_add) = *outer {
            if let Term::Global(op) = *mid {
                assert_eq!(op, "add");
            }
            // lhs_add should be add(x, y)
            if let Term::App(inner_outer, _) = *lhs_add {
                if let Term::App(inner_inner, _) = *inner_outer {
                    if let Term::Global(op2) = *inner_inner {
                        assert_eq!(op2, "add");
                    }
                }
            }
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_comparison_with_arithmetic() {
    // x + 1 <= y * 2 should parse as le(add(x, 1), mul(y, 2))
    let mut parser = LiterateParser::new("x + 1 <= y * 2");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "le");
            } else {
                panic!("Expected le");
            }
        } else {
            panic!("Expected inner App");
        }
    } else {
        panic!("Expected outer App");
    }
}

#[test]
fn test_parse_unicode_le() {
    let mut parser = LiterateParser::new("x ≤ y");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "le");
            } else {
                panic!("Expected le for ≤");
            }
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_unicode_ge() {
    let mut parser = LiterateParser::new("x ≥ y");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "ge");
            } else {
                panic!("Expected ge for ≥");
            }
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_negative_number_preserved() {
    // -5 + x should NOT parse - as subtraction; should be add(-5, x)
    let mut parser = LiterateParser::new("-5 + x");
    let term = parser.parse_term().unwrap();
    // Should be add(-5, x)
    if let Term::App(outer, _) = term {
        if let Term::App(inner, lhs) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "add");
            }
            if let Term::Lit(Literal::Int(n)) = *lhs {
                assert_eq!(n, -5);
            } else {
                panic!("Expected -5 literal");
            }
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_subtraction_not_negative() {
    // x - 5 should parse as sub(x, 5)
    let mut parser = LiterateParser::new("x - 5");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, rhs) = term {
        if let Term::App(inner, _) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "sub");
            }
        }
        if let Term::Lit(Literal::Int(n)) = *rhs {
            assert_eq!(n, 5); // Should be positive 5, not -5
        } else {
            panic!("Expected 5 literal");
        }
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_infix_with_sexp_mix() {
    // Ensure S-expression syntax still works: (add x y) <= z
    let mut parser = LiterateParser::new("(add x y) <= z");
    let term = parser.parse_term().unwrap();
    if let Term::App(outer, _) = term {
        if let Term::App(inner, lhs) = *outer {
            if let Term::Global(op) = *inner {
                assert_eq!(op, "le");
            }
            // lhs should be add(x, y) from S-expression parsing
            if let Term::App(add_outer, _) = *lhs {
                if let Term::App(add_inner, _) = *add_outer {
                    if let Term::Global(add_op) = *add_inner {
                        assert_eq!(add_op, "add");
                    }
                }
            }
        }
    } else {
        panic!("Expected App");
    }
}
