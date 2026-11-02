// Focused proposal controls; not executed as part of this local-only change.
struct Generated {
    depth: usize,
    limit: usize,
    calls: usize,
}
impl super::ReferenceProjection for Generated {
    fn generated_depth_limit(&self) -> Option<usize> {
        Some(self.limit)
    }
    fn project(
        &mut self,
        _: &str,
        _: usize,
        _: super::Term,
        _: &[super::ReferenceBinding<'_>],
        budget: &mut usize,
    ) -> super::Result<super::Term> {
        self.calls += 1;
        let nodes = self.depth * 2 - 1;
        *budget = budget.checked_sub(nodes).ok_or_else(|| {
            super::declined(
                super::DiagnosticCode::ResourceLimit,
                "control callback budget",
            )
        })?;
        let mut term = super::Term::Global("leaf".into());
        for _ in 1..self.depth {
            term = super::Term::App(Box::new(super::Term::Global("f".into())), Box::new(term));
        }
        Ok(term)
    }
}

fn reading(extra: &[serde_json::Value], root: u64) -> super::Reference {
    let mut rows = vec![
        serde_json::json!({"meta":{"format":{"version":"3.1.0"},"lean":{"version":"4.14.0"}}}),
        serde_json::json!({"in":1,"str":{"pre":0,"str":"S"}}),
        serde_json::json!({"in":2,"str":{"pre":0,"str":"probe"}}),
        serde_json::json!({"ie":0,"sort":0}),
        serde_json::json!({"ie":1,"proj":{"typeName":1,"idx":0,"struct":0}}),
    ];
    rows.extend_from_slice(extra);
    rows.push(serde_json::json!({"def":{"name":2,"levelParams":[],"type":0,"value":root,"safety":"safe"}}));
    let text: String = rows.iter().map(|row| row.to_string() + "\n").collect();
    super::Reference::read(&text).unwrap()
}

fn wrapper_rows() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"ie":2,"app":{"fn":0,"arg":1}}),
        serde_json::json!({"ie":2,"app":{"fn":1,"arg":0}}),
        serde_json::json!({"ie":2,"lam":{"name":1,"type":0,"body":1,"binderInfo":"default"}}),
        serde_json::json!({"ie":2,"forallE":{"name":1,"type":1,"body":0,"binderInfo":"default"}}),
        serde_json::json!({"ie":2,"letE":{"name":1,"type":0,"value":1,"body":0,"nonDep":true}}),
        serde_json::json!({"ie":2,"letE":{"name":1,"type":1,"value":0,"body":0,"nonDep":true}}),
        serde_json::json!({"ie":2,"letE":{"name":1,"type":0,"value":0,"body":1,"nonDep":true}}),
    ]
}

#[test]
fn every_wrapper_refuses_one_level_past_callback_limit() {
    for wrapper in wrapper_rows() {
        let input = reading(&[wrapper], 2);
        let mut callback = Generated {
            depth: 8,
            limit: 8,
            calls: 0,
        };
        let error = input.value_with("probe", &mut callback).unwrap_err();
        assert_eq!(error.reason().code(), super::DiagnosticCode::ResourceLimit);
        assert_eq!(callback.calls, 1);
    }
}

#[test]
fn every_wrapper_accepts_exact_generated_depth_limit() {
    for wrapper in wrapper_rows() {
        let input = reading(&[wrapper], 2);
        let mut callback = Generated {
            depth: 7,
            limit: 8,
            calls: 0,
        };
        let term = input.value_with("probe", &mut callback).unwrap();
        let actual = super::OutputShape::callback(&term, Some(8)).unwrap();
        assert_eq!(actual.depth, 8);
        assert_eq!(callback.calls, 1);
    }
}

#[test]
fn enclosing_source_wrappers_accumulate_depth_without_rescanning_subtrees() {
    let input = reading(
        &[
            serde_json::json!({"ie":2,"app":{"fn":0,"arg":1}}),
            serde_json::json!({"ie":3,"app":{"fn":0,"arg":2}}),
        ],
        3,
    );
    let mut callback = Generated {
        depth: 7,
        limit: 8,
        calls: 0,
    };
    let error = input.value_with("probe", &mut callback).unwrap_err();
    assert_eq!(error.reason().code(), super::DiagnosticCode::ResourceLimit);
}

#[test]
fn callback_actual_output_is_checked_even_without_outer_wrapper() {
    let input = reading(&[], 1);
    let mut callback = Generated {
        depth: 9,
        limit: 8,
        calls: 0,
    };
    let error = input.value_with("probe", &mut callback).unwrap_err();
    assert_eq!(error.reason().code(), super::DiagnosticCode::ResourceLimit);
}

#[test]
fn metadata_rejects_oversized_wrapper_without_constructing_any_term() {
    let generated = super::OutputShape {
        depth: 8,
        nodes: 15,
        generated: true,
    };
    assert!(super::OutputShape::wrapper(&[generated, super::OutputShape::LEAF], Some(8)).is_err());
    let wide = super::OutputShape {
        depth: 1,
        nodes: super::REFERENCE_NODE_BUDGET,
        generated: true,
    };
    assert!(super::OutputShape::wrapper(&[wide], None).is_err());
}

#[test]
fn default_and_unused_typed_callback_preserve_the_hundred_thousand_deep_source_contract() {
    assert_eq!(
        super::ReferenceProjection::generated_depth_limit(&super::NoProjection),
        None
    );
    let mut rows = Vec::new();
    let mut root = 0;
    for ie in 2..100_002u64 {
        rows.push(serde_json::json!({"ie":ie,"forallE":{"name":1,"type":0,"body":root,"binderInfo":"default"}}));
        root = ie;
    }
    let input = reading(&rows, root);
    let mut callback = Generated {
        depth: 1,
        limit: 2,
        calls: 0,
    };
    for term in [
        input.value("probe").unwrap(),
        input.value_with("probe", &mut callback).unwrap(),
    ] {
        // Dismantle the deep spine iteratively so this control needs no large drop stack.
        let mut term = term;
        let mut depth = 0;
        loop {
            match term {
                super::Term::Pi { body_type, .. } => {
                    term = *body_type;
                    depth += 1;
                }
                _ => break,
            }
        }
        assert_eq!(depth, 100_000);
    }
    assert_eq!(callback.calls, 0);
}
