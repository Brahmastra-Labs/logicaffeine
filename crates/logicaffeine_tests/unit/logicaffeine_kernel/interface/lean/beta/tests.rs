use super::*;
fn open_application() -> Importer<'static> {
    let mut importer = Importer::new(false);
    importer.levels.insert(1, Universe::Type(0));
    importer.level_vars.insert(1, HashSet::new());
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"sort":1}));
    importer
        .exprs
        .insert(1, serde_json::json!({"ie":1,"bvar":0}));
    importer.exprs.insert(
        2,
        serde_json::json!({"ie":2,"lam":{"name":0,"type":0,"body":1,"binderInfo":"default"}}),
    );
    importer
        .exprs
        .insert(3, serde_json::json!({"ie":3,"app":{"fn":2,"arg":1}}));
    importer
}
#[test]
fn identical_source_at_same_depth_has_distinct_domain_certificates() {
    let importer = open_application();
    let plan = dag::Plan::new(&importer, 3, 1).unwrap();
    let mut budget = 10_000;
    let mut pass = Certificates::new(Context::new(), &HashSet::new(), &[], &mut budget).unwrap();
    pass.enter(&Term::Sort(Universe::Type(0)), &mut budget)
        .unwrap();
    assert!(pass.certify(&importer, &plan, 3, &mut budget).unwrap());
    let good = pass.active.last().copied().flatten().unwrap();
    pass.leave();
    pass.enter(&Term::Sort(Universe::Type(1)), &mut budget)
        .unwrap();
    let bad = pass.active.last().copied().flatten().unwrap();
    assert_ne!(good, bad);
    assert!(matches!(
        pass.certify(&importer, &plan, 3, &mut budget),
        Err(Verdict::Reject(_))
    ));
    pass.leave();
    pass.enter(&Term::Sort(Universe::Type(0)), &mut budget)
        .unwrap();
    assert_eq!(pass.active.last().copied().flatten(), Some(good));
    assert!(pass.certify(&importer, &plan, 3, &mut budget).unwrap());
}
#[test]
fn a_closed_pi_does_not_make_its_dependent_result_closed() {
    let mut budget = 10_000;
    let mut pass = Certificates::new(Context::new(), &HashSet::new(), &[], &mut budget).unwrap();
    let function = Term::Pi {
        param: "A".into(),
        param_type: Box::new(Term::Sort(Universe::Type(0))),
        body_type: Box::new(Term::Var("A".into())),
    };
    let ft = pass.closed_id(&function).expect("the whole Pi is closed");
    let at = pass.closed_id(&Term::Sort(Universe::Type(0))).unwrap();
    assert_eq!(pass.app_rule(ft, at, &mut budget).unwrap(), None);
    assert_eq!(pass.apps.get(&(ft, at)), Some(&None));
}
#[test]
fn formal_universe_scope_is_frozen_per_certificate_pass() {
    let mut importer = Importer::new(false);
    importer.levels.insert(1, Universe::Var("u".into()));
    importer.level_vars.insert(1, HashSet::from(["u".into()]));
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"sort":1}));
    let plan = dag::Plan::new(&importer, 0, 0).unwrap();
    let mut budget = 10_000;
    let mut valid = Certificates::new(
        Context::new(),
        &HashSet::from(["u".into()]),
        &[],
        &mut budget,
    )
    .unwrap();
    assert!(valid.certify(&importer, &plan, 0, &mut budget).unwrap());
    let mut absent = Certificates::new(Context::new(), &HashSet::new(), &[], &mut budget).unwrap();
    assert!(matches!(
        absent.certify(&importer, &plan, 0, &mut budget),
        Err(Verdict::Reject(_))
    ));
}
#[test]
fn binary_lexical_lookup_preserves_all_scope_boundaries() {
    let mut budget = 10_000;
    let mut pass = Certificates::new(Context::new(), &HashSet::new(), &[], &mut budget).unwrap();
    let mut types = Vec::new();
    for i in 0..65 {
        let ty = Term::Sort(Universe::Type(i % 2));
        types.push(pass.closed_id(&ty).unwrap());
        pass.enter(&ty, &mut budget).unwrap();
    }
    let env = pass.active.last().copied().flatten().unwrap();
    for index in 0..65 {
        assert_eq!(
            pass.lookup(env, index, &mut budget).unwrap(),
            Some(types[64 - index])
        );
    }
    assert_eq!(pass.lookup(env, 65, &mut budget).unwrap(), None);
    for _ in 0..65 {
        pass.leave();
    }
    assert_eq!(pass.active, vec![Some(0)]);
}
#[test]
fn a_new_helper_cannot_change_a_frozen_source_rule_environment() {
    let mut ctx = Context::new();
    let mut budget = 10_000;
    let mut pass = Certificates::new(ctx.clone(), &HashSet::new(), &[], &mut budget).unwrap();
    ctx.declare(
        "later",
        Term::Sort(Universe::Type(0)),
        Term::Sort(Universe::Prop),
    )
    .unwrap();
    assert!(ctx.get_definition_body("later").is_some());
    pass.enter(&Term::Global("later".into()), &mut budget)
        .unwrap();
    assert_eq!(pass.active.last(), Some(&None));
    pass.leave();
    assert_eq!(pass.active, vec![Some(0)]);
}
#[test]
fn source_certificate_exhaustion_does_not_authorize_specialization() {
    let importer = open_application();
    let plan = dag::Plan::new(&importer, 3, 1).unwrap();
    let mut setup = 10_000;
    let mut pass = Certificates::new(Context::new(), &HashSet::new(), &[], &mut setup).unwrap();
    pass.enter(&Term::Sort(Universe::Type(0)), &mut setup)
        .unwrap();
    let mut exhausted = 0;
    assert!(matches!(
        pass.certify(&importer, &plan, 3, &mut exhausted),
        Err(Verdict::Decline(_))
    ));
    assert_eq!(exhausted, 0);
    assert!(pass.source.is_empty());
}
