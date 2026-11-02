use super::*;

#[test]
fn a_bare_word_is_a_tag() {
    assert_eq!(
        parse("li"),
        Some(vec![Compound {
            tag: Some("li".into()),
            ..Default::default()
        }])
    );
}

#[test]
fn an_id_a_class_and_an_attribute_each_parse() {
    let c = compound("li#a.done[data-id=2]").expect("parses");
    assert_eq!(c.tag.as_deref(), Some("li"));
    assert_eq!(c.id.as_deref(), Some("a"));
    assert_eq!(c.classes, ["done"]);
    assert_eq!(c.attrs, [("data-id".to_string(), Some("2".to_string()))]);
}

#[test]
fn whitespace_is_the_descendant_combinator() {
    let s = parse(".card li").expect("parses");
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].classes, ["card"]);
    assert_eq!(s[1].tag.as_deref(), Some("li"));
}

#[test]
fn an_attribute_may_be_a_presence_test() {
    assert_eq!(
        compound("[hidden]").expect("parses").attrs,
        [("hidden".to_string(), None)]
    );
}

#[test]
fn quotes_around_an_attribute_value_are_optional() {
    let a = compound("[k=\"v\"]").expect("parses");
    let b = compound("[k=v]").expect("parses");
    assert_eq!(a.attrs, b.attrs);
}

#[test]
fn nothing_and_nonsense_do_not_parse() {
    assert_eq!(parse(""), None);
    assert_eq!(parse("   "), None);
    assert_eq!(compound("#"), None);
    assert_eq!(compound("[unclosed"), None);
}
