use super::*;
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: 3,
        max_tree_nodes: 100,
        max_store_nodes: 100,
        work: 1_000_000,
        allocation: 1_000_000,
        materialized_nodes: 100,
    }
}

#[test]
fn conditional_closure_refuses_deep_telescope_before_any_owned_output_is_materialized() {
    let reference = reference::Reference::read(include_str!(
        "../../../../../tests/fixtures/lean_conformance/cases_lean414/refusal/T02-008.coerceAnything.ndjson"
    ))
    .unwrap();
    let sources = BTreeMap::new();
    let mut budget = budget();
    let d = Construction::new(&mut budget).unwrap();
    let mut b = ConditionalBuilder {
        reference: &reference,
        sources: &sources,
        instances: BTreeMap::new(),
        active: BTreeSet::new(),
        bindings: Vec::new(),
        d,
        source_nodes: 100,
        source_payload_envelope: 1,
        serial: 0,
    };
    let domain = b.d.intern(&Term::Sort(Universe::Prop)).unwrap();
    let x = b.d.symbol("x").unwrap();
    let root = b.d.add(Node::Var(x)).unwrap();
    let instance = b.d.intern(&Term::Global("fixture".into())).unwrap();
    for n in ["a", "b", "c"] {
        let local = b.d.symbol(n).unwrap();
        b.bindings.push(Binding {
            source: n.into(),
            instance,
            local,
            ty: domain,
            body: None,
        });
    }
    let before = b.d.budget.materialized_nodes;
    let error = b.close(root, domain).unwrap_err();
    assert_eq!(error.reason().code(), DiagnosticCode::ResourceLimit);
    assert_eq!(b.d.budget.materialized_nodes, before);
}

#[test]
fn audit_text_exhaustion_is_a_resource_refusal_not_a_partial_success() {
    let mut b = budget();
    b.allocation = 0;
    let error = rendered(&"conditional statement", &mut b).unwrap_err();
    assert_eq!(error.reason().code(), DiagnosticCode::ResourceLimit);
}

fn acyclic_runtime_chain(length: usize, depth: usize, exceeds_depth: bool) {
    crate::term_store::grow_stack_for_depth(depth, || {
        let mut rows = vec![
            serde_json::json!({"meta":{"format":{"version":"3.1.0"},"lean":{"version":"4.14.0"}}}),
            serde_json::json!({"il":0,"zero":{}}),
            serde_json::json!({"ie":0,"sort":0}),
        ];
        let mut records = Vec::new();
        for i in 0..=length {
            let id = i + 1;
            rows.push(serde_json::json!({"in":id,"str":{"pre":0,"str":format!("chain{i}")}}));
            let record = if i == 0 {
                serde_json::json!({"name":id,"levelParams":[],"type":0,"isUnsafe":true})
            } else {
                rows.push(serde_json::json!({"ie":i,"const":{"name":i,"us":[]}}));
                serde_json::json!({"name":id,"levelParams":[],"type":0,"value":i,
                    "safety":"unsafe","hints":"abbrev","all":[id]})
            };
            let kind = if i == 0 {
                intake::DeclarationKind::Axiom
            } else {
                intake::DeclarationKind::Definition
            };
            rows.push(serde_json::json!({kind.as_str():record.clone()}));
            records.push((format!("chain{i}"), kind, record));
        }
        let export: String = rows.iter().map(|row| row.to_string() + "\n").collect();
        let reference = reference::Reference::read(&export).expect("acyclic runtime source tables");
        let sources: BTreeMap<_, _> = records
            .into_iter()
            .map(|(name, kind, record)| {
                let name = reference
                    .declaration(&name)
                    .expect("source declaration")
                    .name;
                (
                    name,
                    Source {
                        kind,
                        record,
                        quarantined: true,
                    },
                )
            })
            .collect();
        let target = reference
            .declaration(&format!("chain{length}"))
            .unwrap()
            .name;
        let mut allowance = RecursorBudget {
            max_depth: depth,
            max_tree_nodes: 1_000_000,
            max_store_nodes: 1_000_000,
            work: 1_000_000_000_000,
            allocation: 100_000_000,
            materialized_nodes: 10_000_000,
        };
        let d = Construction::new(&mut allowance).unwrap();
        let mut builder = ConditionalBuilder {
            reference: &reference,
            sources: &sources,
            instances: BTreeMap::new(),
            active: BTreeSet::new(),
            bindings: Vec::new(),
            d,
            source_nodes: 1_000_000,
            source_payload_envelope: export.len() as u64,
            serial: 0,
        };
        let result = builder.instance(&target, &[], 0);
        if exceeds_depth {
            let error = result
                .expect_err("deep acyclic dependency chain must refuse before closure output");
            assert_eq!(error.reason().code(), DiagnosticCode::ResourceLimit);
            assert!(
                error
                    .reason()
                    .message()
                    .contains("conditional dependency depth"),
                "{error:?}"
            );
            assert!(
                builder.bindings.is_empty(),
                "no completed dependency instance exists before the depth boundary"
            );
        } else {
            let (root, ty) = result.expect("admitted acyclic dependency chain");
            assert_eq!(builder.bindings.len(), length + 1);
            assert!(builder.active.is_empty());
            let (body, ty) = builder.close(root, ty).expect("admitted local telescope");
            let body = builder.d.extract(body).unwrap();
            let ty = builder.d.extract(ty).unwrap();
            let ctx = Context::new();
            let actual =
                super::super::lean_type(&ctx, &body).expect("closed conditional body checks");
            super::super::exact(&ctx, &actual, &ty).expect("conditional statement matches");
            let actual = crate::recheck(&ctx, &body).expect("independent closed conditional check");
            super::super::exact(&ctx, &actual, &ty)
                .expect("independent conditional statement matches");
        }
    });
}

#[test]
fn deep_acyclic_runtime_dependencies_preserve_a_checked_conditional_judgment() {
    acyclic_runtime_chain(32, 128, false);
}

#[test]
fn over_depth_acyclic_runtime_dependencies_refuse_without_a_completed_instance() {
    acyclic_runtime_chain(300, 256, true);
}
