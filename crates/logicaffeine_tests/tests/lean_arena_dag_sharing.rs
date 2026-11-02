//! DAG materialization must check shared templates before reusing them.
use logicaffeine_kernel::interface::lean as arena;
use serde_json::{json, Value};

struct Dag {
    rows: Vec<Value>,
    next: usize,
}
impl Dag {
    fn new() -> Self {
        Self {
            rows: vec![
                json!({"meta":{"format":{"version":"3.1.0"}}}),
                json!({"in":1,"str":{"pre":0,"str":"shared"}}),
                json!({"il":1,"succ":0}),
            ],
            next: 0,
        }
    }
    fn expr(&mut self, key: &str, value: Value) -> usize {
        let id = self.next;
        self.next += 1;
        self.rows.push(json!({"ie":id,key:value}));
        id
    }
    fn binder(&mut self, kind: &str, ty: usize, body: usize) -> usize {
        self.expr(
            kind,
            json!({"name":0,"type":ty,"body":body,"binderInfo":"default"}),
        )
    }
    fn app(&mut self, f: usize, a: usize) -> usize {
        self.expr("app", json!({"fn":f,"arg":a}))
    }
    fn finish(mut self, ty: usize, body: usize) -> String {
        self.rows.push(
            json!({"def":{"name":1,"type":ty,"value":body,"levelParams":[],
            "all":[1],"safety":"safe","hints":"opaque"}}),
        );
        self.rows
            .into_iter()
            .map(|r| r.to_string() + "\n")
            .collect()
    }
}

fn open_dag(rounds: usize, wrong_leaf: bool, dependent_lets: bool, wrong_let: bool) -> String {
    let mut d = Dag::new();
    let universe = d.expr("sort", json!(1));
    let b: Vec<_> = (0..5).map(|i| d.expr("bvar", json!(i))).collect();
    // In context A : Type, the type of a binary function on A.
    let inner = d.binder("forallE", b[1], b[2]);
    let binary = d.binder("forallE", b[0], inner);
    let mut body = if wrong_leaf { universe } else { b[0] };
    let function = if dependent_lets { b[3] } else { b[1] };
    for _ in 0..rounds {
        let partial = d.app(function, body);
        body = d.app(partial, body);
    }
    if dependent_lets {
        // Context A f x B y; B := A and y : B := x.
        body = d.expr(
            "letE",
            json!({"name":0,"type":b[0],
            "value":if wrong_let {universe} else {b[1]},"body":body,"nonDep":false}),
        );
        body = d.expr(
            "letE",
            json!({"name":0,"type":universe,"value":b[2],
            "body":body,"nonDep":false}),
        );
    }
    body = d.binder("lam", b[1], body);
    body = d.binder("lam", binary, body);
    body = d.binder("lam", universe, body);
    let ty = d.binder("forallE", b[1], b[2]);
    let ty = d.binder("forallE", binary, ty);
    let ty = d.binder("forallE", universe, ty);
    d.finish(ty, body)
}

#[test]
fn small_open_dag_is_a_checked_polymorphic_function() {
    let result = arena::check_str_with_declaration_audit(&open_dag(3, false, false, false));
    assert!(result.is_ok(), "{result:?}");
}
#[test]
fn large_open_dag_keeps_dependency_closed_parametric_sharing() {
    let result = arena::check_str_with_declaration_audit(&open_dag(18, false, false, false));
    assert!(result.is_ok(), "{result:?}");
}
#[test]
fn shared_template_cannot_hide_an_invalid_argument() {
    let result = arena::check_str(&open_dag(18, true, false, false));
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}
#[test]
fn small_dependent_let_aliases_are_checked() {
    let result = arena::check_str_with_declaration_audit(&open_dag(3, false, true, false));
    assert!(result.is_ok(), "{result:?}");
}
#[test]
fn shared_template_retains_dependent_let_values() {
    let result = arena::check_str_with_declaration_audit(&open_dag(18, false, true, false));
    assert!(result.is_ok(), "{result:?}");
}
#[test]
fn shared_template_cannot_hide_invalid_source_let_value() {
    let result = arena::check_str(&open_dag(18, false, true, true));
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}

#[test]
fn source_cannot_forge_an_internal_helper_constant_name() {
    let mut rows: Vec<Value> = open_dag(18, false, false, false)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let ty = rows.last().unwrap()["def"]["type"].clone();
    let id = rows.iter().filter_map(|r| r["ie"].as_u64()).max().unwrap() + 1;
    rows.push(json!({"in":2,"str":{"pre":0,"str":"arena.internal.dag.0"}}));
    rows.push(json!({"in":3,"str":{"pre":0,"str":"forged"}}));
    rows.push(json!({"ie":id,"const":{"name":2,"levels":[]}}));
    rows.push(
        json!({"def":{"name":3,"type":ty,"value":id,"levelParams":[],
        "all":[3],"safety":"safe","hints":"opaque"}}),
    );
    let input: String = rows.into_iter().map(|r| r.to_string() + "\n").collect();
    let result = arena::check_str(&input);
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}

#[test]
fn shared_helpers_are_reported_separately_from_source_declarations() {
    let report =
        arena::check_str_with_declaration_audit(&open_dag(18, false, false, false)).unwrap();
    let json = report.to_json();
    assert_eq!(report.declarations.len(), 1);
    let helpers = json["internal_helpers"]
        .as_array()
        .expect("explicit checked helper inventory");
    assert!(!helpers.is_empty());
    assert!(helpers
        .iter()
        .all(|h| h["status"] == "independently_checked_definition"));
    assert!(report.declarations[0].axioms.is_empty());
}

// Two genuinely universe-polymorphic source declarations, each requiring sharing.
// The later declaration operates one universe above the first, making an internal
// helper overwrite observably change both type and body.
fn two_universe_dags(include_later: bool) -> String {
    let mut rows: Vec<Value> = open_dag(18, false, false, false)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let insert = vec![
        json!({"in":2,"str":{"pre":0,"str":"u"}}),
        json!({"in":3,"str":{"pre":0,"str":"later"}}),
        json!({"il":2,"param":2}),
        json!({"il":3,"succ":2}),
    ];
    rows.splice(3..3, insert);
    for row in &mut rows {
        if row.get("sort").is_some() {
            row["sort"] = json!(2);
        }
        if let Some(def) = row.get_mut("def") {
            def["levelParams"] = json!([2]);
        }
    }
    if include_later {
        let offset = rows.iter().filter_map(|r| r["ie"].as_u64()).max().unwrap() + 1;
        for mut row in open_dag(18, false, false, false)
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
        {
            if let Some(id) = row["ie"].as_u64() {
                row["ie"] = json!(id + offset);
                if row.get("sort").is_some() {
                    row["sort"] = json!(3);
                }
                if let Some(app) = row.get_mut("app") {
                    app["fn"] = json!(app["fn"].as_u64().unwrap() + offset);
                    app["arg"] = json!(app["arg"].as_u64().unwrap() + offset);
                }
                for kind in ["lam", "forallE"] {
                    if let Some(binder) = row.get_mut(kind) {
                        binder["type"] = json!(binder["type"].as_u64().unwrap() + offset);
                        binder["body"] = json!(binder["body"].as_u64().unwrap() + offset);
                    }
                }
                rows.push(row);
            } else if let Some(def) = row.get_mut("def") {
                def["name"] = json!(3);
                def["all"] = json!([3]);
                def["levelParams"] = json!([2]);
                def["type"] = json!(def["type"].as_u64().unwrap() + offset);
                def["value"] = json!(def["value"].as_u64().unwrap() + offset);
                rows.push(row);
            }
        }
    }
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn separate_universe_declarations_reserve_distinct_internal_helper_names() {
    let (_, report) = arena::import_audited(&two_universe_dags(true)).unwrap();
    assert_eq!(report.declarations.len(), 2);
    assert!(report
        .internal_helpers
        .iter()
        .all(|h| h.level_parameters.len() == 1));
    let names: std::collections::HashSet<_> =
        report.internal_helpers.iter().map(|h| &h.name).collect();
    assert_eq!(
        names.len(),
        report.internal_helpers.len(),
        "later polymorphic source declarations must never replace earlier checked helpers"
    );
}

#[test]
fn later_universe_declaration_preserves_earlier_helper_delta_meaning() {
    use logicaffeine_kernel::{Term, Universe};
    let (before, report) = arena::import_audited(&two_universe_dags(false)).unwrap();
    let (after, _) = arena::import_audited(&two_universe_dags(true)).unwrap();
    let helper = &report.internal_helpers[0];
    let (params, ty, body) = before.get_universe_poly(&helper.name).unwrap();
    let head = Term::Const {
        name: helper.name.clone(),
        levels: params.iter().cloned().map(Universe::Var).collect(),
    };
    assert!(
        logicaffeine_kernel::defeq_for_test(&after, &head, body),
        "importing a later declaration changed an earlier checked definition's delta meaning"
    );
    let actual = logicaffeine_kernel::recheck(&after, &head).unwrap();
    assert!(
        logicaffeine_kernel::defeq_for_test(&after, &actual, ty),
        "earlier polymorphic helper must retain its independently checked type"
    );
}
