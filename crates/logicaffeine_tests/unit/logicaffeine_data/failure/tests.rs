use super::*;

#[test]
fn a_text_reason_is_the_report() {
    assert_eq!(
        contract_reason(&"size must be positive".to_string()),
        "size must be positive"
    );
    assert_eq!(contract_reason(&"literal"), "literal");
}

#[test]
fn a_non_text_reason_is_refused_naming_its_type() {
    assert_eq!(
        contract_reason(&42i64),
        "The reason after `otherwise` must be Text, but it was Int."
    );
    assert_eq!(
        contract_reason(&2.5f64),
        "The reason after `otherwise` must be Text, but it was Real."
    );
    assert_eq!(
        contract_reason(&true),
        "The reason after `otherwise` must be Text, but it was Bool."
    );
    assert_eq!(
        contract_reason(&'c'),
        "The reason after `otherwise` must be Text, but it was Char."
    );
}

#[test]
fn found_returns_the_value_a_lookup_found() {
    assert_eq!(found(Some(7), "k"), 7);
}

#[test]
#[should_panic(expected = "Key 'b' not found in map")]
fn found_fails_naming_the_missing_key() {
    let _: i64 = found(None, "b");
}

#[test]
#[should_panic(expected = "Key '2' not found in map")]
fn found_names_an_int_key_as_show_prints_it() {
    let _: i64 = found(None, &2i64);
}

#[test]
fn a_field_of_a_value_that_is_not_a_record_stops_naming_its_type() {
    assert_eq!(field_access_on("Nothing"), "Cannot access field on Nothing");
    assert_eq!(SET_FIELD_ON_NON_RECORD, "Cannot set field on non-struct value");
}
