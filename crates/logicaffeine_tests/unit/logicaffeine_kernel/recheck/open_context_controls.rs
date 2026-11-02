use super::*;
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}

#[test]
fn aggregate_admission_counts_the_request_and_each_demanded_type_once() {
    let c = Context::new().extend("A", sort()).extend("x", v("A"));
    let term = Term::App(Box::new(v("x")), Box::new(v("x")));
    // App + two x occurrences + stored A + stored Sort and its level = six.
    let plan = LocalPlan::prepare(&c, &term, 6).unwrap();
    assert!(plan.issue.is_none());
    assert_eq!(
        plan.bindings.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        vec!["A", "x"]
    );
    assert!(matches!(
        LocalPlan::prepare(&c, &term, 5),
        Err(ReCheckError::Unsupported(_))
    ));
    assert!(matches!(
        LocalPlan::prepare(&c, &term, 0),
        Err(ReCheckError::Unsupported(_))
    ));
}

#[test]
fn wide_match_admission_uses_a_slice_not_an_arity_sized_pending_stack() {
    let term = Term::Match {
        discriminant: Box::new(sort()),
        motive: Box::new(sort()),
        cases: vec![sort(); 1024],
    };
    let mut admission = LocalAdmission::new(2053);
    assert!(admission.free_names(&term).unwrap().is_empty());
    assert_eq!(admission.remaining, 0);
    assert!(admission.high_water <= 4);
    let mut short = LocalAdmission::new(8);
    assert!(matches!(
        short.free_names(&term),
        Err(ReCheckError::Unsupported(_))
    ));
    assert!(short.high_water <= 4);
}

#[test]
fn cycle_and_missing_diagnoses_do_not_skip_remaining_input_admission() {
    let c = Context::new().extend("A", v("B")).extend("B", v("A"));
    let term = Term::App(Box::new(v("A")), Box::new(v("Missing")));
    let plan = LocalPlan::prepare(&c, &term, 5).unwrap();
    assert!(matches!(plan.issue, Some(ReCheckError::Ill(_))));
    assert!(matches!(
        LocalPlan::prepare(&c, &term, 4),
        Err(ReCheckError::Unsupported(_))
    ));
}

#[test]
fn lexical_collection_preserves_own_domains_and_simultaneous_fix_scope() {
    let lambda = Term::Lambda {
        param: "A".into(),
        param_type: Box::new(v("A")),
        body: Box::new(v("A")),
    };
    let mut admission = LocalAdmission::new(100);
    assert_eq!(admission.free_names(&lambda).unwrap(), vec!["A"]);
    let mutual = Term::MutualFix {
        defs: vec![
            ("first".into(), v("second")),
            ("second".into(), v("external")),
        ],
        index: 0,
    };
    assert_eq!(admission.free_names(&mutual).unwrap(), vec!["external"]);
    let lets = Term::Let {
        name: "x".into(),
        ty: Box::new(sort()),
        value: Box::new(v("x")),
        body: Box::new(v("x")),
    };
    assert_eq!(admission.free_names(&lets).unwrap(), vec!["x"]);
}

#[test]
fn active_local_budget_is_shared_at_logical_depth_zero_and_by_nested_scopes() {
    assert!(!recheck_budget_active());
    let guard = LocalContextBudget::enter();
    assert_eq!(WHNF_DEPTH.with(|d| d.get()), 0);
    assert!(recheck_budget_active());
    WHNF_FUEL.with(|f| f.set(3));
    let c = Context::new();
    assert!(matches!(
        whnf(&c, &Db::Sort(Universe::Type(0))),
        Db::Sort(_)
    ));
    assert_eq!(WHNF_FUEL.with(|f| f.get()), 2);
    {
        let _nested = LocalContextBudget::enter();
        assert_eq!(WHNF_FUEL.with(|f| f.get()), 2);
        whnf(&c, &Db::Sort(Universe::Type(0)));
        assert_eq!(WHNF_FUEL.with(|f| f.get()), 1);
    }
    assert!(recheck_budget_active());
    drop(guard);
    assert!(!recheck_budget_active());
}

#[test]
fn exhaustion_is_not_refunded_or_cleared_before_a_later_local_stage() {
    let _guard = LocalContextBudget::enter();
    WHNF_FUEL.with(|f| f.set(0));
    whnf(&Context::new(), &Db::Sort(Universe::Type(0)));
    assert!(WHNF_EXHAUSTED.with(|e| e.get()));
    let _nested = LocalContextBudget::enter();
    assert_eq!(WHNF_FUEL.with(|f| f.get()), 0);
    assert!(matches!(
        local_outcome::<()>(Err(ReCheckError::ill("later mismatch"))),
        Err(ReCheckError::Unsupported(_))
    ));
    assert!(WHNF_EXHAUSTED.with(|e| e.get()));
    assert!(
        local_outcome(Ok(())).is_ok(),
        "a completed positive proof remains valid"
    );
}

#[test]
fn local_budget_ownership_is_restored_after_unwind() {
    assert!(!recheck_budget_active());
    let result = std::panic::catch_unwind(|| {
        let _guard = LocalContextBudget::enter();
        assert!(recheck_budget_active());
        panic!("intentional scoped unwind");
    });
    assert!(result.is_err());
    assert!(!recheck_budget_active());
}

#[test]
fn unused_context_bindings_do_not_enter_the_shared_open_budget() {
    let mut term = sort();
    for i in 0..64 {
        term = Term::Pi {
            param: format!("n{i}"),
            param_type: Box::new(sort()),
            body_type: Box::new(term),
        };
    }
    let empty = Context::new();
    let unused = empty.extend("irrelevant", Term::Hole);
    LOCAL_BUDGET_ENTRIES.with(|n| n.set(Some(0)));
    let without = recheck(&empty, &term).unwrap();
    let with = recheck(&unused, &term).unwrap();
    assert_eq!(without, with);
    assert_eq!(double_check(&unused, &term), DoubleCheck::Agreed);
    assert_eq!(LOCAL_BUDGET_ENTRIES.with(|n| n.replace(None)), Some(0));
}

#[test]
fn external_names_round_trip_through_pi_fix_and_mutual_raising() {
    let external = vec!["v0".into(), "v4".into(), "v5".into(), "v4_".into()];
    let values = vec![
        Db::Pi(DbBox::new(Db::Var(0)), DbBox::new(Db::Var(4))),
        Db::Fix(FixBody::new(Db::App(
            DbBox::new(Db::Var(0)),
            DbBox::new(Db::Var(4)),
        ))),
        Db::MutualFix {
            defs: vec![Db::Var(1), Db::Var(5)],
            index: 1,
        },
    ];
    for value in values {
        let raised = from_db_with_external(&value, external.len(), &external);
        assert_eq!(to_db(&raised, &mut external.clone()).unwrap(), value);
    }
    let closed = Db::Pi(
        DbBox::new(Db::Sort(Universe::Type(0))),
        DbBox::new(Db::Var(0)),
    );
    let Term::Pi {
        param, body_type, ..
    } = from_db(&closed, 0)
    else {
        panic!("Pi")
    };
    assert_eq!(param, "v0");
    assert_eq!(*body_type, v("v0"));
}
#[test]
fn term_and_universe_admission_keep_the_exact_zero_based_depth_boundary() {
    let mut term = Term::Global("Nat".into());
    for _ in 0..crate::type_checker::MAX_TERM_DEPTH {
        term = Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(term),
        };
    }
    let mut at = LocalAdmission::new(MAX_RECHECK_TERM_NODES);
    assert!(at.free_names(&term).unwrap().is_empty());
    assert_eq!(at.max_depth, crate::type_checker::MAX_TERM_DEPTH);
    let over = Term::Fix {
        name: "f".into(),
        body: Box::new(term),
    };
    assert!(matches!(
        LocalAdmission::new(MAX_RECHECK_TERM_NODES).free_names(&over),
        Err(ReCheckError::Unsupported(_))
    ));
    let mut level = Universe::Type(0);
    for _ in 0..crate::type_checker::MAX_TERM_DEPTH {
        level = Universe::Succ(Box::new(level));
    }
    let at_term = Term::Sort(level);
    let mut at = LocalAdmission::new(MAX_RECHECK_TERM_NODES);
    assert!(at.free_names(&at_term).unwrap().is_empty());
    assert_eq!(at.max_depth, crate::type_checker::MAX_TERM_DEPTH);
    let Term::Sort(level) = at_term else {
        unreachable!()
    };
    let over = Term::Sort(Universe::Succ(Box::new(level)));
    assert!(matches!(
        LocalAdmission::new(MAX_RECHECK_TERM_NODES).free_names(&over),
        Err(ReCheckError::Unsupported(_))
    ));
    let leaf = Term::Sort(Universe::Type(0));
    assert!(LocalAdmission::new(2).free_names(&leaf).is_ok());
    assert!(matches!(
        LocalAdmission::new(1).free_names(&leaf),
        Err(ReCheckError::Unsupported(_))
    ));
}

#[test]
fn output_admission_accepts_available_depth_and_refuses_before_recursive_lowering() {
    let depth = crate::term_store::INLINE_DEPTH + 1;
    let term = sort();
    let run = || {
        assert!(local_stack_admitted(depth).is_ok());
        let current = crate::term_store::kernel_stack_bytes();
        let over = ((current - crate::term_store::BASE_STACK_BYTES)
            / crate::term_store::BYTES_PER_RUNG)
            + 1;
        assert!(matches!(
            local_stack_admitted(over),
            Err(ReCheckError::Unsupported(_))
        ));
        let mut db = Db::Global("Nat".into());
        for _ in 0..depth {
            db = Db::Fix(FixBody::new(db));
        }
        let mut admitted = LocalAdmission::new(MAX_RECHECK_TERM_NODES);
        admitted.admit_db(&db).unwrap();
        assert_eq!(admitted.max_depth, depth);
        assert!(local_stack_admitted(admitted.max_depth).is_ok());
        let wide = Db::MutualFix {
            defs: vec![Db::Global("Nat".into()); 64],
            index: 0,
        };
        let mut admitted = LocalAdmission::new(8);
        assert!(matches!(
            admitted.admit_db(&wide),
            Err(ReCheckError::Unsupported(_))
        ));
        assert!(admitted.high_water <= 3);
    };
    crate::term_store::deepen_stack_with_depth(&term, depth, run).expect("outer sized stack");
}
