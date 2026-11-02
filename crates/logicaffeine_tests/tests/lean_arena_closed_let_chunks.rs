//! Deep source lets may use checked zeta substitution and closed helper cuts.
//! All source annotations and values still require exact independent checking.
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
    DependentTypeAlias,
    Universe,
    WrongUnusedValue,
    CumulativeUnusedValue,
    Opaque,
    UnsafeUnusedValue,
}

fn ladder(depth: usize, variant: Variant) -> String {
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
    let mut last = leaf;
    if matches!(variant, Variant::Opaque) {
        d.declaration("opaque", 2, domain, leaf, &[], "safe");
        last = d.expr("const", json!({"name":2,"us":[]}));
    }
    if matches!(variant, Variant::UnsafeUnusedValue) {
        d.declaration("def", 4, domain, leaf, &[], "unsafe");
    }
    // Every binder is referenced by the innermost source spine, exactly the
    // feature that rules out closed-source-only memoization of Arena's ladder.
    let mut body = last;
    for &variable in b.iter().take(depth) {
        let applied = d.app(choose, variable);
        body = d.app(applied, body);
    }
    for i in 0..depth {
        let applied = d.app(choose, b[0]);
        body = d.app(applied, body);
        let ty = if matches!(variant, Variant::DependentTypeAlias) {
            b[depth - i - 1]
        } else {
            domain
        };
        body = d.let_(ty, leaf, body);
    }
    if matches!(variant, Variant::DependentTypeAlias) {
        body = d.let_(high, domain, body);
    }
    // This extra binding is unused, but its malformed source value/type or
    // forbidden runtime dependency must still be checked before any erasure.
    match variant {
        Variant::WrongUnusedValue => body = d.let_(domain, domain, body),
        Variant::CumulativeUnusedValue => body = d.let_(high, leaf, body),
        Variant::UnsafeUnusedValue => {
            let unsafe_value = d.expr("const", json!({"name":4,"us":[]}));
            body = d.let_(domain, unsafe_value, body);
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
fn accepted(depth: usize, variant: Variant) -> arena::ImportReport {
    let result = arena::check_str_with_declaration_audit(&ladder(depth, variant));
    assert!(result.is_ok(), "{result:?}");
    result.unwrap()
}
#[test]
fn ordinary_closed_let_path_still_checks_without_helper_cuts() {
    let report = accepted(4, Variant::Ordinary);
    assert!(report.internal_helpers.is_empty());
}
#[test]
fn deep_alternating_closed_lets_fit_checked_shallow_helpers() {
    let report = accepted(640, Variant::Ordinary);
    assert!(!report.internal_helpers.is_empty());
    assert!(report.declarations.iter().all(|d| d.axioms.is_empty()));
}
#[test]
fn earlier_closed_type_let_remains_visible_to_later_annotations() {
    accepted(4, Variant::DependentTypeAlias);
    accepted(640, Variant::DependentTypeAlias);
}
#[test]
fn closed_helper_cuts_preserve_explicit_universe_parameters() {
    let report = accepted(640, Variant::Universe);
    assert!(!report.internal_helpers.is_empty());
    assert!(report
        .internal_helpers
        .iter()
        .all(|h| h.level_parameters.len() == 1));
}
#[test]
fn unused_malformed_let_value_cannot_disappear_in_depth_lowering() {
    for depth in [4, 640] {
        let result = arena::check_str(&ladder(depth, Variant::WrongUnusedValue));
        assert!(
            matches!(result, Err(arena::Verdict::Reject(_))),
            "{result:?}"
        );
    }
}
#[test]
fn exact_importer_let_annotation_policy_is_unchanged_by_lowering() {
    for depth in [4, 640] {
        let result = arena::check_str(&ladder(depth, Variant::CumulativeUnusedValue));
        assert!(
            matches!(result, Err(arena::Verdict::Reject(_))),
            "{result:?}"
        );
    }
}
#[test]
fn unused_runtime_reference_is_rejected_before_closed_lowering() {
    for depth in [4, 640] {
        let result = arena::check_str(&ladder(depth, Variant::UnsafeUnusedValue));
        assert!(
            matches!(result, Err(arena::Verdict::Reject(_))),
            "{result:?}"
        );
    }
}
#[test]
fn closed_chunks_keep_the_uniquely_opaque_declaration_opaque() {
    use logicaffeine_kernel::{defeq_for_test, Term, Universe};
    let (ctx, report) = arena::import_audited(&ladder(640, Variant::Opaque))
        .unwrap_or_else(|e| panic!("opaque ladder import: {e:?}"));
    assert!(!report.internal_helpers.is_empty());
    let declarations: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.kind == "opaque")
        .collect();
    assert_eq!(
        declarations.len(),
        1,
        "fixture defines exactly one opaque source declaration"
    );
    let name = &declarations[0].name;
    assert!(
        ctx.get_definition_body(name).is_none(),
        "opaque source body must remain hidden"
    );
    assert!(!defeq_for_test(
        &ctx,
        &Term::Global(name.clone()),
        &Term::Sort(Universe::Prop)
    ));
}
