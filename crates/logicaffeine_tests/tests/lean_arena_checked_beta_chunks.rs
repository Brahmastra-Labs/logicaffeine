//! Generic lambda typing must be certified before closed beta specialization.
//! Source-depth handling must never turn a specialized valid body into a certificate
//! for an invalid original lambda.
use logicaffeine_kernel::interface::lean as arena;
use serde_json::{json, Value};

struct Dag {
    rows: Vec<Value>,
    next: usize,
}
impl Dag {
    fn new() -> Self {
        Self {
            next: 0,
            rows: vec![
                json!({"meta":{"format":{"version":"3.1.0"}}}),
                json!({"in":1,"str":{"pre":0,"str":"deep"}}),
                json!({"in":2,"str":{"pre":0,"str":"opaqueT"}}),
                json!({"in":3,"str":{"pre":0,"str":"u"}}),
                json!({"in":4,"str":{"pre":0,"str":"unsafeT"}}),
                json!({"il":1,"succ":0}),
                json!({"il":2,"succ":1}),
                json!({"il":3,"param":3}),
                json!({"il":4,"succ":3}),
            ],
        }
    }
    fn expr(&mut self, key: &str, value: Value) -> usize {
        let id = self.next;
        self.next += 1;
        self.rows.push(json!({"ie":id,key:value}));
        id
    }
    fn app(&mut self, f: usize, a: usize) -> usize {
        self.expr("app", json!({"fn":f,"arg":a}))
    }
    fn lam(&mut self, ty: usize, body: usize) -> usize {
        self.expr(
            "lam",
            json!({"name":0,"type":ty,"body":body,"binderInfo":"default"}),
        )
    }
    fn let_(&mut self, ty: usize, value: usize, body: usize) -> usize {
        self.expr(
            "letE",
            json!({"name":0,"type":ty,"value":value,"body":body,"nondep":false}),
        )
    }
    fn declaration(
        &mut self,
        kind: &str,
        name: usize,
        ty: usize,
        body: usize,
        params: &[usize],
        safety: &str,
    ) {
        self.rows
            .push(json!({kind:{"name":name,"type":ty,"value":body,
            "levelParams":params,"all":[name],"safety":safety,"hints":"opaque"}}));
    }
    fn finish(self) -> String {
        self.rows
            .into_iter()
            .map(|r| r.to_string() + "\n")
            .collect()
    }
}

#[derive(Clone, Copy)]
enum Variant {
    Ordinary,
    Universe,
    WrongUnusedArgument,
    CumulativeUnusedArgument,
}

fn beta_ladder(depth: usize, variant: Variant) -> String {
    let mut d = Dag::new();
    let poly = matches!(variant, Variant::Universe);
    let domain = d.expr("sort", json!(if poly { 4 } else { 1 }));
    let leaf = d.expr("sort", json!(if poly { 3 } else { 0 }));
    let high = d.expr("sort", json!(2));
    let b: Vec<_> = (0..depth.max(2))
        .map(|i| d.expr("bvar", json!(i)))
        .collect();
    let choose_inner = d.lam(domain, b[0]);
    let choose = d.lam(domain, choose_inner);
    let mut body = leaf;
    // Every lambda binder is used. A closed-source-only cache cannot represent
    // this generic body; specializing before checking it would erase obligations.
    for &variable in b.iter().take(depth) {
        let applied = d.app(choose, variable);
        body = d.app(applied, body);
    }
    for _ in 0..depth {
        let function = d.lam(domain, body);
        body = d.app(function, leaf);
    }
    match variant {
        Variant::WrongUnusedArgument => {
            let function = d.lam(domain, body);
            body = d.app(function, domain);
        }
        Variant::CumulativeUnusedArgument => {
            let function = d.lam(high, body);
            body = d.app(function, leaf);
        }
        _ => {}
    }
    d.declaration(
        "def",
        1,
        domain,
        body,
        if poly { &[3] } else { &[] },
        "safe",
    );
    d.finish()
}

#[test]
fn shallow_beta_source_remains_checked_without_internal_helpers() {
    let report = arena::check_str_with_declaration_audit(&beta_ladder(4, Variant::Ordinary))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(report.internal_helpers.is_empty());
}

#[test]
fn deep_all_binder_beta_source_has_a_checked_shallow_representation() {
    let report = arena::check_str_with_declaration_audit(&beta_ladder(640, Variant::Ordinary))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(!report.internal_helpers.is_empty());
    assert!(report.declarations.iter().all(|d| d.axioms.is_empty()));
}

#[test]
fn beta_source_preserves_formal_universe_identity() {
    let report = arena::check_str_with_declaration_audit(&beta_ladder(640, Variant::Universe))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(!report.internal_helpers.is_empty());
    assert!(report
        .internal_helpers
        .iter()
        .all(|h| h.level_parameters.len() == 1));
}

#[test]
fn unused_beta_argument_must_have_the_exact_domain_type() {
    for variant in [
        Variant::WrongUnusedArgument,
        Variant::CumulativeUnusedArgument,
    ] {
        for depth in [4, 640] {
            let result = arena::check_str(&beta_ladder(depth, variant));
            assert!(
                matches!(result, Err(arena::Verdict::Reject(_))),
                "{result:?}"
            );
        }
    }
}

/// (fun A : Sort 1 => (fun x : A => x) Prop) (Sort 0)
/// specializes to a valid identity application, but the generic application
/// under arbitrary A is ill-typed. It must not acquire a certificate by beta.
fn specialization_hides_bad_generic_body() -> String {
    let mut d = Dag::new();
    let prop = d.expr("sort", json!(0));
    let type0 = d.expr("sort", json!(1));
    let type1 = d.expr("sort", json!(2));
    let b0 = d.expr("bvar", json!(0));
    let identity = d.lam(b0, b0);
    let invalid_generic = d.app(identity, prop);
    let generic = d.lam(type1, invalid_generic);
    let specialized = d.app(generic, type0);
    d.declaration("def", 1, type0, specialized, &[], "safe");
    d.finish()
}

#[test]
fn beta_specialization_cannot_hide_an_invalid_generic_lambda_body() {
    let result = arena::check_str(&specialization_hides_bad_generic_body());
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}

fn wrap_closed_beta(
    d: &mut Dag,
    domain: usize,
    leaf: usize,
    mut body: usize,
    depth: usize,
) -> usize {
    for _ in 0..depth {
        let function = d.lam(domain, body);
        body = d.app(function, leaf);
    }
    body
}

#[test]
fn invalid_generic_lambda_cannot_hide_in_an_unused_beta_argument() {
    let mut d = Dag::new();
    let prop = d.expr("sort", json!(0));
    let type0 = d.expr("sort", json!(1));
    let type1 = d.expr("sort", json!(2));
    let b0 = d.expr("bvar", json!(0));
    let identity = d.lam(b0, b0);
    let invalid_generic = d.app(identity, prop);
    let generic = d.lam(type1, invalid_generic);
    let specialized = d.app(generic, type0);
    let ignore = d.lam(type0, prop);
    let body = d.app(ignore, specialized);
    let body = wrap_closed_beta(&mut d, type0, prop, body, 640);
    d.declaration("def", 1, type0, body, &[], "safe");
    let result = arena::check_str(&d.finish());
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}

#[test]
fn original_erased_beta_argument_keeps_its_runtime_dependency() {
    let mut d = Dag::new();
    let prop = d.expr("sort", json!(0));
    let type0 = d.expr("sort", json!(1));
    d.declaration("def", 4, type0, prop, &[], "unsafe");
    let unsafe_value = d.expr("const", json!({"name":4,"us":[]}));
    let ignore = d.lam(type0, prop);
    let body = d.app(ignore, unsafe_value);
    let body = wrap_closed_beta(&mut d, type0, prop, body, 640);
    d.declaration("def", 1, type0, body, &[], "safe");
    let result = arena::check_str(&d.finish());
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}

#[test]
fn checked_beta_chunks_keep_opaque_values_opaque() {
    use logicaffeine_kernel::{defeq_for_test, Term, Universe};
    let mut d = Dag::new();
    let prop = d.expr("sort", json!(0));
    let type0 = d.expr("sort", json!(1));
    d.declaration("opaque", 2, type0, prop, &[], "safe");
    let opaque = d.expr("const", json!({"name":2,"us":[]}));
    let body = wrap_closed_beta(&mut d, type0, prop, opaque, 640);
    d.declaration("def", 1, type0, body, &[], "safe");
    let (ctx, report) = arena::import_audited(&d.finish()).unwrap_or_else(|e| panic!("{e:?}"));
    let opaque = report
        .declarations
        .iter()
        .filter(|d| d.kind == "opaque")
        .collect::<Vec<_>>();
    assert_eq!(opaque.len(), 1);
    let name = &opaque[0].name;
    assert!(ctx.get_definition_body(name).is_none());
    assert!(!defeq_for_test(
        &ctx,
        &Term::Global(name.clone()),
        &Term::Sort(Universe::Prop)
    ));
}

#[test]
fn a_reused_source_body_is_checked_under_each_actual_domain() {
    let mut d = Dag::new();
    let prop = d.expr("sort", json!(0));
    let type0 = d.expr("sort", json!(1));
    let type1 = d.expr("sort", json!(2));
    let b0 = d.expr("bvar", json!(0));
    let id_type0 = d.lam(type0, b0);
    let shared = d.app(id_type0, b0);
    // Exactly one body source ID, used under two distinct binder types.
    let good = d.lam(type0, shared);
    let bad = d.lam(type1, shared);
    let good_type = d.expr(
        "forallE",
        json!({"name":0,"type":type0,"body":type0,"binderInfo":"default"}),
    );
    let bad_type = d.expr(
        "forallE",
        json!({"name":0,"type":type1,"body":type0,"binderInfo":"default"}),
    );
    let body = d.let_(bad_type, bad, prop);
    let body = d.let_(good_type, good, body);
    let body = wrap_closed_beta(&mut d, type0, prop, body, 640);
    d.declaration("def", 1, type0, body, &[], "safe");
    let result = arena::check_str(&d.finish());
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}
