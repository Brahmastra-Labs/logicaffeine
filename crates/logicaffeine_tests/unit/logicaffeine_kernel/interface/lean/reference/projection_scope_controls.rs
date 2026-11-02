// No additional imports: the reference reader's dependency surface is locked.
#[derive(Debug, Default)]
struct Recorder {
    calls: Vec<(
        String,
        usize,
        super::Term,
        Vec<(String, super::Term, Option<super::Term>)>,
    )>,
}

impl super::ReferenceProjection for Recorder {
    fn project(
        &mut self,
        parent: &str,
        index: usize,
        discriminant: super::Term,
        scope: &[super::ReferenceBinding<'_>],
        budget: &mut usize,
    ) -> super::Result<super::Term> {
        self.calls.push((
            parent.into(),
            index,
            discriminant.clone(),
            scope
                .iter()
                .map(|binding| {
                    (
                        binding.name.into(),
                        binding.ty.clone(),
                        binding.value.cloned(),
                    )
                })
                .collect(),
        ));
        *budget = budget.checked_sub(2).ok_or_else(|| {
            super::declined(
                super::DiagnosticCode::ResourceLimit,
                "callback output budget",
            )
        })?;
        Ok(super::Term::App(
            Box::new(super::Term::Global("projection.marker".into())),
            Box::new(discriminant),
        ))
    }
}

fn reading(expressions: Vec<serde_json::Value>, root: u64, as_type: bool) -> super::Reference {
    let mut rows = vec![
        serde_json::json!({"meta":{"format":{"version":"3.1.0"},"lean":{"version":"4.14.0"}}}),
        serde_json::json!({"in":1,"str":{"pre":0,"str":"S"}}),
        serde_json::json!({"in":2,"str":{"pre":0,"str":"scopeProbe"}}),
        serde_json::json!({"in":3,"str":{"pre":0,"str":"x"}}),
    ];
    rows.extend(expressions);
    rows.push(serde_json::json!({"def":{"name":2,"levelParams":[],
        "type":if as_type {root} else {0},"value":root,"safety":"safe"}}));
    let text: String = rows.iter().map(|row| row.to_string() + "\n").collect();
    super::Reference::read(&text).unwrap()
}

fn sort() -> super::Term {
    super::Term::Sort(super::Universe::Prop)
}
fn var(index: usize) -> super::Term {
    super::Term::Var(format!("ref.{index}"))
}
fn marked(term: super::Term) -> super::Term {
    super::Term::App(
        Box::new(super::Term::Global("projection.marker".into())),
        Box::new(term),
    )
}

#[test]
fn callback_nested_lambdas_receive_ordered_lexical_domains() {
    let input = reading(
        vec![
            serde_json::json!({"ie":0,"sort":0}),
            serde_json::json!({"ie":1,"bvar":0}),
            serde_json::json!({"ie":2,"bvar":1}),
            serde_json::json!({"ie":3,"proj":{"typeName":1,"idx":4,"struct":2}}),
            serde_json::json!({"ie":4,"lam":{"name":3,"type":1,"body":3,"binderInfo":"default"}}),
            serde_json::json!({"ie":5,"lam":{"name":3,"type":0,"body":4,"binderInfo":"default"}}),
        ],
        5,
        false,
    );
    let mut recording = Recorder::default();
    input.value_with("scopeProbe", &mut recording).unwrap();
    assert_eq!(recording.calls.len(), 1);
    assert_eq!(recording.calls[0].0, "/s:\"S\"");
    assert_eq!(recording.calls[0].1, 4);
    assert_eq!(recording.calls[0].2, var(0));
    assert_eq!(
        recording.calls[0].3,
        vec![
            ("ref.0".into(), sort(), None),
            ("ref.1".into(), var(0), None),
        ]
    );
}

#[test]
fn callback_let_value_is_outside_its_binder_and_body_sees_the_decoded_value() {
    let input = reading(
        vec![
            serde_json::json!({"ie":0,"sort":0}),
            serde_json::json!({"ie":1,"bvar":0}),
            serde_json::json!({"ie":2,"proj":{"typeName":1,"idx":0,"struct":1}}),
            serde_json::json!({"ie":3,"proj":{"typeName":1,"idx":1,"struct":1}}),
            serde_json::json!({"ie":4,"letE":{"name":3,"type":1,"value":2,"body":3,"nonDep":false}}),
            serde_json::json!({"ie":5,"lam":{"name":3,"type":0,"body":4,"binderInfo":"default"}}),
        ],
        5,
        false,
    );
    let mut recording = Recorder::default();
    input.value_with("scopeProbe", &mut recording).unwrap();
    assert_eq!(recording.calls.len(), 2);
    assert_eq!(recording.calls[0].3, vec![("ref.0".into(), sort(), None)]);
    assert_eq!(recording.calls[0].2, var(0));
    assert_eq!(recording.calls[1].2, var(1));
    assert_eq!(
        recording.calls[1].3,
        vec![
            ("ref.0".into(), sort(), None),
            ("ref.1".into(), var(0), Some(marked(var(0)))),
        ]
    );
}

#[test]
fn callback_nested_projection_receives_inner_callback_output_in_the_same_scope() {
    let input = reading(
        vec![
            serde_json::json!({"ie":0,"sort":0}),
            serde_json::json!({"ie":1,"bvar":0}),
            serde_json::json!({"ie":2,"proj":{"typeName":1,"idx":0,"struct":1}}),
            serde_json::json!({"ie":3,"proj":{"typeName":1,"idx":1,"struct":2}}),
            serde_json::json!({"ie":4,"lam":{"name":3,"type":0,"body":3,"binderInfo":"default"}}),
        ],
        4,
        true,
    );
    let mut recording = Recorder::default();
    input
        .declaration_with("scopeProbe", &mut recording)
        .unwrap();
    assert_eq!(recording.calls.len(), 2);
    assert_eq!(recording.calls[0].1, 0);
    assert_eq!(recording.calls[1].1, 1);
    assert_eq!(recording.calls[1].2, marked(var(0)));
    assert_eq!(recording.calls[0].3, recording.calls[1].3);
}

#[test]
fn callback_scope_is_restored_after_a_lambda_before_its_sibling_expression() {
    let input = reading(
        vec![
            serde_json::json!({"ie":0,"sort":0}),
            serde_json::json!({"ie":1,"bvar":0}),
            serde_json::json!({"ie":2,"proj":{"typeName":1,"idx":0,"struct":1}}),
            serde_json::json!({"ie":3,"lam":{"name":3,"type":0,"body":2,"binderInfo":"default"}}),
            serde_json::json!({"ie":4,"proj":{"typeName":1,"idx":1,"struct":0}}),
            serde_json::json!({"ie":5,"app":{"fn":3,"arg":4}}),
        ],
        5,
        false,
    );
    let mut recording = Recorder::default();
    input.value_with("scopeProbe", &mut recording).unwrap();
    assert_eq!(recording.calls.len(), 2);
    assert_eq!(recording.calls[0].3.len(), 1);
    assert!(recording.calls[1].3.is_empty());
    assert_eq!(recording.calls[1].2, sort());
}

#[test]
fn callback_apis_preflight_all_raw_children_before_invoking_a_callback() {
    for as_type in [false, true] {
        let input = reading(
            vec![
                serde_json::json!({"ie":0,"sort":0}),
                serde_json::json!({"ie":1,"proj":{"typeName":1,"idx":0,"struct":0}}),
                serde_json::json!({"ie":2,"app":{"fn":1,"arg":999}}),
            ],
            2,
            as_type,
        );
        let mut recording = Recorder::default();
        let result = if as_type {
            input
                .declaration_with("scopeProbe", &mut recording)
                .map(|declaration| declaration.ty)
        } else {
            input.value_with("scopeProbe", &mut recording)
        };
        assert!(result.is_err());
        assert!(
            recording.calls.is_empty(),
            "callback effects must follow complete raw preflight"
        );
    }
}

#[test]
fn raw_shape_reports_projection_generation_without_invoking_a_callback() {
    let input = reading(
        vec![
            serde_json::json!({"ie":0,"sort":0}),
            serde_json::json!({"ie":1,"proj":{"typeName":1,"idx":0,"struct":0}}),
        ],
        1,
        true,
    );
    let metrics = input
        .preflight(&[("scopeProbe".into(), false)], 100, 100)
        .unwrap();
    assert!(metrics.has_projection);
    let plain = reading(vec![serde_json::json!({"ie":0,"sort":0})], 0, true);
    assert!(
        !plain
            .preflight(&[("scopeProbe".into(), false)], 100, 100)
            .unwrap()
            .has_projection
    );
}

#[test]
fn default_reader_still_refuses_projection_without_a_typed_callback() {
    let input = reading(
        vec![
            serde_json::json!({"ie":0,"sort":0}),
            serde_json::json!({"ie":1,"proj":{"typeName":1,"idx":0,"struct":0}}),
        ],
        1,
        true,
    );
    for refusal in [
        input.value("scopeProbe").unwrap_err(),
        input.declaration("scopeProbe").unwrap_err(),
    ] {
        assert_eq!(
            refusal.reason().code(),
            super::DiagnosticCode::UnsupportedFeature
        );
    }
}
