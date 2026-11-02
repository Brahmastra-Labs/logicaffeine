//! Decoder-only controls. The small record's expressions deliberately need not form a valid
//! inductive: canonical derivation and kernel checking belong to the subsequent binding layer.
use super::Reference;
use serde_json::{json, Value};

fn records() -> Vec<Value> {
    vec![
        json!({"meta":{"lean":{"version":"4.14.0"},"format":{"version":"3.1.0"}}}),
        json!({"in":1,"str":{"pre":0,"str":"D"}}),
        json!({"in":2,"str":{"pre":1,"str":"mk"}}),
        json!({"in":3,"str":{"pre":1,"str":"rec"}}),
        json!({"in":4,"str":{"pre":0,"str":"u"}}),
        json!({"ie":0,"sort":0}),
        json!({"ie":1,"sort":0}),
        json!({"inductive":{
            "types":[{"name":1,"levelParams":[4],"type":0,"numParams":2,"numIndices":1,
                "numNested":0,"isUnsafe":false,"isRec":false,"isReflexive":true,"ctors":[2],"all":[1]}],
            "ctors":[{"name":2,"levelParams":[4],"type":0,"numParams":2,"numFields":3,
                "isUnsafe":false,"induct":1,"cidx":0}],
            "recs":[{"name":3,"levelParams":[4],"type":0,"numParams":2,"numIndices":1,
                "numMotives":1,"numMinors":1,"isUnsafe":false,"all":[1],"k":true,
                "rules":[{"ctor":2,"nfields":3,"rhs":1}]}]
        }}),
    ]
}

fn reference(records: &[Value]) -> Reference {
    let export = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    Reference::read(&export).expect("reference tables")
}

#[test]
fn the_containing_block_preserves_every_decoded_field_and_expression_occurrence() {
    let source = reference(&records());
    let block = source.recursor_block_shape("D.rec").unwrap();
    assert_eq!(
        source.recursor_block_roots("D.rec").unwrap(),
        vec![0, 0, 0, 1]
    );
    assert_eq!(
        block,
        source.recursor_block_shape("/s:\"D\"/s:\"rec\"").unwrap()
    );
    let member = &block.members[0];
    assert_eq!(member.declaration.name, "/s:\"D\"");
    assert_eq!(member.declaration.lean_name, "D");
    assert_eq!(member.declaration.level_params, vec!["/s:\"u\""]);
    assert_eq!(member.declaration.ty, 0);
    assert!(!member.declaration.is_unsafe);
    assert_eq!(
        (member.num_params, member.num_indices, member.num_nested),
        (2, 1, 0)
    );
    assert!(!member.is_recursive);
    assert!(member.is_reflexive);
    assert_eq!(member.constructors, vec!["/s:\"D\"/s:\"mk\""]);
    assert_eq!(member.all, vec!["/s:\"D\""]);
    let ctor = &block.constructors[0];
    assert_eq!(ctor.parent, member.declaration.name);
    assert_eq!((ctor.index, ctor.num_params, ctor.num_fields), (0, 2, 3));
    let rec = &block.recursors[0];
    assert_eq!(
        (
            rec.num_params,
            rec.num_indices,
            rec.num_motives,
            rec.num_minors
        ),
        (2, 1, 1, 1)
    );
    assert!(rec.is_k);
    assert_eq!(rec.all, member.all);
    assert_eq!(rec.rules[0].constructor, ctor.declaration.name);
    assert_eq!((rec.rules[0].num_fields, rec.rules[0].rhs), (3, 1));
    let translated = source.recursor_block("D.rec").unwrap();
    assert_eq!(
        translated.members[0].declaration.ty,
        super::Term::Sort(super::Universe::Prop)
    );
    assert_eq!(
        translated.recursors[0].rules[0].rhs,
        super::Term::Sort(super::Universe::Prop)
    );
}

#[test]
fn repeated_containing_blocks_and_duplicate_recursors_are_ambiguous() {
    let mut duplicated = records();
    duplicated.push(duplicated.last().unwrap().clone());
    assert!(reference(&duplicated)
        .recursor_block_shape("D.rec")
        .is_err());
    let mut same_block = records();
    let recs = same_block.last_mut().unwrap()["inductive"]["recs"]
        .as_array_mut()
        .unwrap();
    recs.push(recs[0].clone());
    assert!(reference(&same_block)
        .recursor_block_shape("D.rec")
        .is_err());
}

#[test]
fn a_colliding_top_level_name_and_a_nonrecursor_request_are_refused() {
    let mut duplicate = records();
    duplicate.push(json!({"def":{"name":3,"levelParams":[],"type":0,"value":0}}));
    assert!(reference(&duplicate).recursor_block_shape("D.rec").is_err());
    assert!(reference(&records()).recursor_block_shape("D.mk").is_err());
    assert!(reference(&records())
        .recursor_block_shape("D.missing")
        .is_err());
}

#[test]
fn malformed_flags_counts_universes_and_rule_fields_have_no_decoded_block() {
    for replacement in [json!(null), json!(1), json!("false")] {
        let mut input = records();
        input.last_mut().unwrap()["inductive"]["types"][0]["isRec"] = replacement;
        assert!(reference(&input).recursor_block_shape("D.rec").is_err());
    }
    for replacement in [json!(-1), json!(1.5), json!("2")] {
        let mut input = records();
        input.last_mut().unwrap()["inductive"]["ctors"][0]["numFields"] = replacement;
        assert!(reference(&input).recursor_block_shape("D.rec").is_err());
    }
    let mut duplicate_levels = records();
    duplicate_levels.last_mut().unwrap()["inductive"]["recs"][0]["levelParams"] = json!([4, 4]);
    assert!(reference(&duplicate_levels)
        .recursor_block_shape("D.rec")
        .is_err());
    let mut missing = records();
    missing.last_mut().unwrap()["inductive"]["recs"][0]["rules"][0]
        .as_object_mut()
        .unwrap()
        .remove("nfields");
    assert!(reference(&missing).recursor_block_shape("D.rec").is_err());
    let mut extra = records();
    extra.last_mut().unwrap()["inductive"]["recs"][0]["rules"][0]["trusted"] = json!(true);
    assert!(reference(&extra).recursor_block_shape("D.rec").is_err());
}

#[test]
fn shape_reads_rule_roots_without_translating_them() {
    let mut input = records();
    input.last_mut().unwrap()["inductive"]["recs"][0]["rules"][0]["rhs"] = json!(999);
    let source = reference(&input);
    assert_eq!(
        source.recursor_block_roots("D.rec").unwrap(),
        vec![0, 0, 0, 999]
    );
    assert!(source.recursor_block("D.rec").is_err());
}

#[test]
fn source_flags_are_data_for_the_subsequent_canonical_validator() {
    let mut input = records();
    input.last_mut().unwrap()["inductive"]["types"][0]["isUnsafe"] = json!(true);
    input.last_mut().unwrap()["inductive"]["types"][0]["isRec"] = json!(true);
    input.last_mut().unwrap()["inductive"]["recs"][0]["numMotives"] = json!(7);
    let block = reference(&input).recursor_block_shape("D.rec").unwrap();
    assert!(block.members[0].declaration.is_unsafe);
    assert!(block.members[0].is_recursive);
    assert_eq!(block.recursors[0].num_motives, 7);
}

#[test]
fn a_mutual_block_and_its_nested_auxiliary_keep_their_source_order() {
    let mut input = records();
    for record in [
        json!({"in":5,"str":{"pre":0,"str":"E"}}),
        json!({"in":6,"str":{"pre":5,"str":"mk"}}),
        json!({"in":7,"str":{"pre":5,"str":"rec"}}),
        json!({"in":8,"str":{"pre":1,"str":"rec_1"}}),
    ] {
        input.insert(input.len() - 1, record);
    }
    let block = &mut input.last_mut().unwrap()["inductive"];
    block["types"][0]["numNested"] = json!(1);
    block["types"][0]["all"] = json!([1, 5]);
    let mut member = block["types"][0].clone();
    member["name"] = json!(5);
    member["ctors"] = json!([6]);
    block["types"].as_array_mut().unwrap().push(member);
    let mut ctor = block["ctors"][0].clone();
    ctor["name"] = json!(6);
    ctor["induct"] = json!(5);
    block["ctors"].as_array_mut().unwrap().push(ctor);
    block["recs"][0]["all"] = json!([1, 5]);
    block["recs"][0]["numMotives"] = json!(3);
    block["recs"][0]["k"] = json!(false);
    let mut rec = block["recs"][0].clone();
    rec["name"] = json!(7);
    rec["rules"][0]["ctor"] = json!(6);
    block["recs"].as_array_mut().unwrap().push(rec);
    let mut aux = block["recs"][0].clone();
    aux["name"] = json!(8);
    block["recs"].as_array_mut().unwrap().push(aux);
    let source = reference(&input);
    let shape = source.recursor_block_shape("D.rec_1").unwrap();
    assert_eq!(
        shape
            .members
            .iter()
            .map(|m| m.declaration.lean_name.as_str())
            .collect::<Vec<_>>(),
        vec!["D", "E"]
    );
    assert_eq!(
        shape
            .recursors
            .iter()
            .map(|r| r.declaration.lean_name.as_str())
            .collect::<Vec<_>>(),
        vec!["D.rec", "E.rec", "D.rec_1"]
    );
    assert_eq!(shape.expression_roots(), vec![0, 0, 0, 0, 0, 1, 0, 1, 0, 1]);
    assert_eq!(shape, source.recursor_block_shape("E.rec").unwrap());
    assert_eq!(source.recursor_block("D.rec_1").unwrap().recursors.len(), 3);
}

#[test]
fn block_preflight_counts_repeated_roots_and_reserves_generated_output() {
    let source = reference(&records());
    let metrics = source
        .preflight_recursor_blocks(&["D.rec".into()], 100, 8)
        .unwrap();
    assert_eq!(
        metrics.tree_nodes, 8,
        "four repeated Sort roots each contain a level node"
    );
    assert!(metrics.has_generated);
    assert!(!metrics.has_projection);
    assert!(source
        .preflight_recursor_blocks(&["D.rec".into()], 100, 7)
        .is_err());
    let twice = source
        .preflight_recursor_blocks(&["D.rec".into(), "/s:\"D\"/s:\"rec\"".into()], 100, 8)
        .unwrap();
    assert_eq!(
        twice.tree_nodes, metrics.tree_nodes,
        "the same canonical block is admitted once"
    );
}

#[test]
fn a_late_missing_rule_root_stops_every_block_callback_before_translation() {
    struct Count(usize);
    impl super::ReferenceProjection for Count {
        fn project(
            &mut self,
            _: &str,
            _: usize,
            discriminant: super::Term,
            _: &[super::ReferenceBinding<'_>],
            _: &mut usize,
        ) -> super::Result<super::Term> {
            self.0 += 1;
            Ok(discriminant)
        }
    }
    let mut input = records();
    let at = input.len() - 1;
    input.insert(at, json!({"ie":2,"proj":{"typeName":1,"idx":0,"struct":0}}));
    input.last_mut().unwrap()["inductive"]["types"][0]["type"] = json!(2);
    input.last_mut().unwrap()["inductive"]["recs"][0]["rules"][0]["rhs"] = json!(999);
    let source = reference(&input);
    let mut calls = Count(0);
    assert!(source.recursor_block_with("D.rec", &mut calls).is_err());
    assert_eq!(
        calls.0, 0,
        "all block roots are admitted before the first member type is lowered"
    );
    // The default API must stop at raw admission too, before reaching projection refusal.
    let refusal = source.recursor_block("D.rec").unwrap_err();
    assert_eq!(
        refusal.reason().code(),
        super::DiagnosticCode::MalformedExport
    );
}

#[test]
fn general_source_preflight_includes_recursor_rule_roots_before_worker_sizing() {
    let source = reference(&records());
    let metrics = source
        .preflight(&[("D.rec".into(), false)], 100, 10)
        .unwrap();
    assert_eq!(
        metrics.tree_nodes, 10,
        "direct declaration type plus every decoded block root"
    );
    assert!(metrics.has_generated);
    let mut input = records();
    input.last_mut().unwrap()["inductive"]["recs"][0]["rules"][0]["rhs"] = json!(999);
    assert!(reference(&input)
        .preflight(&[("D.rec".into(), false)], 100, 100)
        .is_err());
}
