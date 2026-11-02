//! Structural equality must preserve both value kinds and each kind's identity semantics.
#![cfg(not(target_arch = "wasm32"))]
mod common;
#[path = "support/riscv_runner.rs"]
mod riscv_runner;
use logicaffeine_compile::compile::{compile_to_riscv, tw_lines, vm_outcome};

fn agrees(label: &str, source: &str, expected: &str) {
    agrees_with_vm(label, source, expected, false);
}

fn agrees_with_vm(label: &str, source: &str, expected: &str, networking: bool) {
    let tw = tw_lines(source);
    assert!(tw.error.is_none(), "{label}: tree-walker: {:?}", tw.error);
    let output: String = tw.lines.into_iter().map(|line| line + "\n").collect();
    assert_eq!(output, expected, "{label}: tree-walker specification");
    let vm = if networking {
        let result = logicaffeine_compile::compile::vm_lines_net(source);
        logicaffeine_compile::compile::RunOutcome {
            output: result.lines.into_iter().map(|line| line + "\n").collect(),
            error: result.error,
        }
    } else {
        vm_outcome(source)
    };
    assert!(vm.error.is_none(), "{label}: VM: {:?}", vm.error);
    assert_eq!(vm.output, expected, "{label}: VM specification");
    let asm = compile_to_riscv(source).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    if !(common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64")) {
        return;
    }
    let linked = riscv_runner::link(&asm, label);
    let run = riscv_runner::run(&linked, &[], &[]);
    assert_eq!(run.code, Some(0), "{label}: {}", run.describe());
    assert_eq!(run.stderr, "", "{label}: unexpected diagnostic");
    assert_eq!(run.stdout, expected, "{label}: RISC-V disagrees");
}

#[test]
fn aliased_record_still_compares_its_nan_field() {
    agrees(
        "record NaN",
        r#"## A Sample has:
    a value, which is Float.
## Main
Let n be sqrt(0.0 - 1.0).
Let a be a new Sample with value n.
Let b be a.
Show a is equal to b.
Show a is not equal to b.
"#,
        "false\ntrue\n",
    );
}

#[test]
fn aliased_tuple_still_compares_its_nan_element() {
    agrees(
        "tuple NaN",
        r#"## Main
Let n be sqrt(0.0 - 1.0).
Let a be (n, 7).
Let b be a.
Show a is equal to b.
Show a is not equal to b.
"#,
        "false\ntrue\n",
    );
}

#[test]
fn aliased_enum_still_compares_its_nan_payload() {
    agrees(
        "enum NaN",
        r#"## Definition
A Reading is either:
    A Measured with a value, which is Float.
    A Missing.
## Main
Let n be sqrt(0.0 - 1.0).
Let a be a new Measured with value n.
Let b be a.
Show a is equal to b.
Show a is not equal to b.
"#,
        "false\ntrue\n",
    );
}

#[test]
fn list_identity_is_preserved_but_a_deep_copy_compares_elements() {
    agrees(
        "list identity",
        r#"## Main
Let n be sqrt(0.0 - 1.0).
Let a be [n].
Let b be a.
Let c be copy of a.
Show a is equal to b.
Show a is equal to c.
"#,
        "true\nfalse\n",
    );
}

#[test]
fn sequences_compare_numeric_elements_by_value_across_kinds() {
    agrees(
        "numeric lists",
        r#"## Main
Let a be [1, 2].
Let b be [1.0, 2.0].
Let c be [1.0, 3.0].
Show a is equal to b.
Show b is equal to a.
Show a is equal to c.
Show a is not equal to b.
"#,
        "true\ntrue\nfalse\nfalse\n",
    );
}

#[test]
fn records_with_different_nominal_types_are_not_equal() {
    agrees(
        "nominal records",
        r#"## A First has:
    a value, which is Int.
## A Second has:
    a value, which is Int.
## Main
Let a be a new First with value 7.
Let b be a new Second with value 7.
Show a is equal to b.
"#,
        "false\n",
    );
}

#[test]
fn mixed_numeric_sequences_keep_precision_and_scalar_kinds() {
    agrees(
        "numeric boundaries",
        r#"## Main
Let a be [9007199254740993].
Let b be [9007199254740992.0].
Let c be [9007199254740992].
Show a is equal to b.
Show c is equal to b.
Let zeros be [0].
Let floatZeros be [0.0].
Show zeros is equal to floatZeros.
Let flags be [true].
Let ints be [1].
Let floats be [1.0].
Show flags is equal to ints.
Show flags is equal to floats.
"#,
        "false\ntrue\ntrue\nfalse\nfalse\n",
    );
}

#[test]
fn empty_sequences_compare_by_contents_even_with_different_element_kinds() {
    agrees(
        "empty different kinds",
        r#"## Main
Let a be a new Seq of Bool.
Let b be a new Seq of Int.
Show a is equal to b.
"#,
        "true\n",
    );
}

#[test]
fn nested_sequences_and_tuples_compare_each_numeric_cell_exactly() {
    agrees(
        "nested numeric shapes",
        r#"## Main
Let a be [[1, 2]].
Let b be [[1.0, 2.0]].
Show a is equal to b.
Let x be (1, 2.0, "same").
Let y be (1.0, 2, "same").
Show x is equal to y.
"#,
        "true\ntrue\n",
    );
}

#[test]
fn optional_payloads_compare_by_value_across_numeric_kinds() {
    agrees(
        "optional numeric shapes",
        r#"## Main
Let a be some 1.
Let b be some 1.0.
Let c be some 2.0.
Show a is equal to b.
Show a is equal to c.
"#,
        "true\nfalse\n",
    );
}

#[test]
fn rational_sequences_compare_exactly_with_float_sequences() {
    agrees(
        "rational float sequences",
        r#"## Main
Let half: Rational be 1 / 2.
Let third: Rational be 1 / 3.
Let exact be [half].
Let floating be [0.5].
Show exact is equal to floating.
Show floating is equal to exact.
Let inexact be [third].
Let rounded be [0.3333333333333333].
Show inexact is equal to rounded.
Show rounded is equal to inexact.
"#,
        "true\ntrue\nfalse\nfalse\n",
    );
}

#[test]
fn whole_rational_sequences_compare_as_their_integer_values() {
    agrees(
        "whole rational sequences",
        r#"## Main
Let whole: Rational be 6 / 3.
Let a be [whole].
Let b be [2].
Show a is equal to b.
Show b is equal to a.
Let zero: Rational be 0 / 3.
Show [zero] is equal to [0].
"#,
        "true\ntrue\ntrue\n",
    );
}

#[test]
fn rational_float_equality_keeps_sign_precision_and_extreme_exponents() {
    for (num, den, float, equal) in [
        ("1", "2", "0.5", true),
        ("-1", "2", "-0.5", true),
        ("-1", "2", "0.5", false),
        ("1", "3", "0.3333333333333333", false),
        ("9007199254740993", "2", "4503599627370496.0", false),
        ("0", "3", "-0.0", true),
        ("1", "4611686018427387904", "2.168404344971009e-19", true),
        ("1", "2", "5e-324", false),
        ("1", "2", "1e300", false),
        ("1", "2", "inf", false),
        ("1", "2", "NaN", false),
    ] {
        let source = format!(
            r#"## Main
Let numerator be parseInt("{num}").
Let denominator be parseInt("{den}").
Let exact: Rational be numerator / denominator.
Let floating be parseFloat("{float}").
Show [exact] is equal to [floating].
Show [floating] is equal to [exact].
"#
        );
        agrees(
            &format!("rational {num}/{den} vs {float}"),
            &source,
            &format!("{equal}\n{equal}\n"),
        );
    }
}

#[test]
fn temporal_sequence_elements_keep_their_nominal_kinds() {
    agrees(
        "temporal sequence kinds",
        r#"## Main
Let d be today.
Let m be now.
Show [d] is equal to [d].
Show [m] is equal to [m].
Show [d] is equal to [0].
Show [m] is equal to [0].
Show [d] is equal to [m].
"#,
        "true\ntrue\nfalse\nfalse\nfalse\n",
    );
}

#[test]
fn peer_sequence_elements_compare_by_topic_but_are_not_text() {
    agrees_with_vm(
        "peer sequence kinds",
        r#"## Main
Let p be a PeerAgent at "same".
Let q be a PeerAgent at "same".
Let r be a PeerAgent at "different".
Show [p] is equal to [q].
Show [p] is equal to [r].
Show [p] is equal to ["same"].
"#,
        "true\nfalse\nfalse\n",
        true,
    );
}

#[test]
fn runtime_rationals_compare_with_integer_sequence_elements() {
    for (num, den, integer, equal) in [
        ("6", "3", "2", true),
        ("0", "7", "0", true),
        ("-6", "3", "-2", true),
        ("6", "4", "1", false),
        ("9223372036854775807", "1", "9223372036854775807", true),
    ] {
        let source = format!(
            r#"## Main
Let numerator be parseInt("{num}").
Let denominator be parseInt("{den}").
Let exact: Rational be numerator / denominator.
Let integer be parseInt("{integer}").
Show [exact] is equal to [integer].
Show [integer] is equal to [exact].
Show [exact] is not equal to [integer].
"#
        );
        agrees(
            &format!("rational {num}/{den} vs int {integer}"),
            &source,
            &format!("{equal}\n{equal}\n{}\n", !equal),
        );
    }
}

#[test]
fn runtime_rational_ordering_does_not_wrap_its_cross_products() {
    for (num, den, other_num, other_den) in [
        ("9223372036854775807", "3", "1", "2"),
        ("9223372036854775807", "2", "-1", "2"),
    ] {
        let source = format!(
            r#"## Main
Let numerator be parseInt("{num}").
Let denominator be parseInt("{den}").
Let otherNumerator be parseInt("{other_num}").
Let otherDenominator be parseInt("{other_den}").
Let a: Rational be numerator / denominator.
Let b: Rational be otherNumerator / otherDenominator.
Show a is equal to b.
Show a is not equal to b.
Show a is greater than b.
Show a is less than b.
Show a is at least b.
Show a is at most b.
Show b is less than a.
"#
        );
        agrees(
            &format!("rational order {num}/{den} vs {other_num}/{other_den}"),
            &source,
            "false\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\n",
        );
    }
}

#[test]
fn absent_option_payloads_compare_and_copy_with_some_in_the_program() {
    agrees(
        "absent option payload",
        r#"## Main
Let present be some 3.
Let absent be none.
Let copied be copy of absent.
Show present is equal to absent.
Show absent is equal to present.
Show copied is equal to absent.
Show copied is equal to nothing.
"#,
        "false\nfalse\ntrue\ntrue\n",
    );
}

#[test]
fn result_constructors_copy_only_their_present_payload() {
    agrees(
        "absent result payload",
        r#"## Main
Let good be a new Ok with value [1, 2].
Let bad be a new Err with error ["no"].
Let copiedGood be copy of good.
Let copiedBad be copy of bad.
Show good is equal to copiedGood.
Show bad is equal to copiedBad.
Show good is equal to bad.
Show bad is equal to good.
"#,
        "true\ntrue\nfalse\nfalse\n",
    );
}

#[test]
fn nested_option_payloads_keep_their_own_numeric_layouts() {
    agrees(
        "nested option layouts",
        r#"## Main
Let absent be none.
Let integer be some 9007199254740993.
Let floating be some 9007199254740992.0.
Let integerOuter be some integer.
Let floatingOuter be some floating.
Let absentOuter be some absent.
Let copied be copy of absentOuter.
Show integerOuter is equal to floatingOuter.
Show floatingOuter is equal to integerOuter.
Show integerOuter is equal to absentOuter.
Show absentOuter is equal to copied.
"#,
        "false\nfalse\nfalse\ntrue\n",
    );
}

#[test]
fn result_payloads_compare_across_numeric_layouts() {
    agrees(
        "result numeric layouts",
        r#"## Main
Let integer be a new Ok with value 2.
Let floating be a new Ok with value 2.0.
Let errorInt be a new Err with error 9007199254740993.
Let errorFloat be a new Err with error 9007199254740992.0.
Show integer is equal to floating.
Show floating is equal to integer.
Show errorInt is equal to errorFloat.
Show errorFloat is equal to errorInt.
"#,
        "true\ntrue\nfalse\nfalse\n",
    );
}

#[test]
fn declared_option_and_result_boundaries_preserve_both_variants() {
    agrees(
        "declared variant boundaries",
        r#"## To maybe (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.
## To result (n: Int) -> Result of Int and Text:
    If n is greater than 0:
        Return a new Ok with value n.
    Return a new Err with error "no".
## Main
Let yes be maybe(3).
Let no be maybe(0).
Let good be result(3).
Let bad be result(0).
Let copiedNo be copy of no.
Let copiedBad be copy of bad.
Show yes is equal to no.
Show copiedNo is equal to no.
Show good is equal to bad.
Show copiedBad is equal to bad.
"#,
        "false\ntrue\nfalse\ntrue\n",
    );
}
