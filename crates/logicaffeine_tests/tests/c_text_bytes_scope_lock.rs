//! A borrowed byte view belongs to the declaration and scope that emitted it.
#![cfg(not(target_arch = "wasm32"))]
mod common;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::optimization::decorate_source;

fn check(source: &str, expected: &str) {
    for (name, run) in [
        ("tree walker", tw_outcome(source)),
        ("VM", vm_outcome(source)),
    ] {
        assert_eq!(run.error, None, "{name}: {source}");
        assert_eq!(
            run.output.trim_end_matches('\n'),
            expected,
            "{name}: {source}"
        );
    }
    common::assert_c_output(&decorate_source(source, &["textbytesview"]), expected);
    common::assert_c_output(source, expected);
}

#[test]
fn same_byte_name_in_two_functions_keeps_each_source() {
    check(
        r#"## To sumFirst (source: Text) -> Int:
    Let bytes be text_bytes(source).
    Let mutable total be 0.
    Repeat for byte in bytes:
        Set total to total + byte.
    Return total.
## To sumSecond (other: Text) -> Int:
    Let bytes be text_bytes(other).
    Let mutable total be 0.
    Repeat for byte in bytes:
        Set total to total + byte.
    Return total.
## Main
Show sumFirst("AZ").
Show sumSecond("BC").
"#,
        "155\n133",
    );
}

#[test]
fn a_materialized_sequence_does_not_inherit_another_functions_view() {
    check(
        r#"## To sumView (source: Text) -> Int:
    Let bytes be text_bytes(source).
    Let mutable total be 0.
    Repeat for byte in bytes:
        Set total to total + byte.
    Return total.
## To sumChanged (other: Text) -> Int:
    Let bytes be text_bytes(other).
    Push 99 to bytes.
    Let mutable total be 0.
    Repeat for byte in bytes:
        Set total to total + byte.
    Return total.
## Main
Show sumView("AZ").
Show sumChanged("BC").
"#,
        "155\n232",
    );
}

#[test]
fn nested_byte_views_restore_the_outer_binding() {
    check(
        r#"## To sumBoth (source: Text, other: Text) -> Int:
    Let bytes be text_bytes(source).
    Let mutable total be 0.
    Repeat for byte in bytes:
        Set total to total + byte.
    If total is greater than 0:
        Let bytes be text_bytes(other).
        Repeat for byte in bytes:
            Set total to total + byte.
    Set total to total + item 1 of bytes.
    Return total.
## Main
Show sumBoth("AZ", "BC").
"#,
        "353",
    );
}

#[test]
fn a_shadowing_owned_sequence_keeps_its_own_representation() {
    check(
        r#"## To sumMixed (source: Text) -> Int:
    Let bytes be text_bytes(source).
    Let mutable total be 0.
    Repeat for byte in bytes:
        Set total to total + byte.
    If total is greater than 0:
        Let bytes be [7, 8].
        Push 9 to bytes.
        Set total to total + length of bytes.
        Set total to total + item 1 of bytes.
    Set total to total + item 1 of bytes.
    Return total.
## Main
Show sumMixed("AZ").
"#,
        "230",
    );
}
