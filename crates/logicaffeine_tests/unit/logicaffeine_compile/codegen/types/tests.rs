//! The Rust spellings the emitter writes for the program's types.
use super::*;

#[test]
fn a_field_filled_from_the_binding_of_its_name_is_written_by_its_name() {
    assert_eq!(field_init("label", "label"), "label");
    assert_eq!(field_init("label", "tone"), "label: tone");
    assert_eq!(field_init("label", "label.clone()"), "label: label.clone()");
    assert_eq!(field_init("r#type", "r#type"), "r#type");
    let shape = VariantShape::InPlace;
    let values = [("left".to_string(), "left".to_string()), ("value".to_string(), "v".to_string())];
    assert_eq!(shape.build("Tree::Node", &values), "Tree::Node { left, value: v }");
}

#[test]
fn a_type_is_written_with_its_arguments() {
    assert_eq!(type_arguments("Maybe<String>"), Some(vec!["String"]));
    assert_eq!(type_arguments("&Result<i64, String>"), Some(vec!["i64", "String"]));
    assert_eq!(type_arguments("&mut Pair<LogosSeq<i64>, Map<String, i64>>"), Some(vec!["LogosSeq<i64>", "Map<String, i64>"]));
    assert_eq!(type_arguments("Point"), None);
    assert_eq!(type_arguments("i64"), None);
}

#[test]
fn a_generic_field_is_its_declared_type_at_the_values_arguments() {
    let mut interner = Interner::new();
    let (t, e, seq, int) = (interner.intern("T"), interner.intern("E"), interner.intern("Seq"), interner.intern("Int"));
    let params = [t, e];
    let bare = FieldType::TypeParam(t);
    let list = FieldType::Generic { base: seq, params: vec![FieldType::TypeParam(e)] };
    let plain = FieldType::Primitive(int);
    let args: &[&str] = &["String", "i64"];
    assert_eq!(field_type_at(&bare, &params, Some(args), &interner).as_deref(), Some("String"));
    assert_eq!(field_type_at(&list, &params, Some(args), &interner).as_deref(), Some("LogosSeq<i64>"));
    assert_eq!(field_type_at(&plain, &params, Some(args), &interner).as_deref(), Some("i64"));
    assert_eq!(field_type_at(&bare, &params, None, &interner), None, "no arguments, no type to write");
    assert_eq!(field_type_at(&bare, &params, Some(&["String"]), &interner), None, "arguments of another arity");
    assert_eq!(field_type_at(&plain, &params, None, &interner).as_deref(), Some("i64"), "names no parameter");
    let named = FieldType::Named(t);
    assert_eq!(field_type_at(&named, &params, Some(args), &interner).as_deref(), Some("String"));
}

#[test]
fn a_pair_or_a_triple_of_its_arity_is_the_tuple_of_its_parts() {
    let ints = ["i64".to_string(), "i64".to_string()];
    let mixed = ["i64".to_string(), "bool".to_string(), "i64".to_string()];
    assert_eq!(named_tuple("Pair", &ints), Some(tuple_rust_type(&ints)));
    assert_eq!(named_tuple("Triple", &mixed), Some(tuple_rust_type(&mixed)));
    assert_eq!(named_tuple("Pair", &mixed), None, "a Pair of three parts is no Pair");
    assert_eq!(named_tuple("Triple", &ints), None);
    assert_eq!(named_tuple("Point", &ints), None);
}
