//! Mutable storage semantics hold after hot calls reach the native tier.
#![cfg(not(target_arch = "wasm32"))]
use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn check(source: &'static str, expected: &'static str) {
    check_with_floor(source, expected, 1);
}

fn check_native(source: &'static str, expected: &'static str) {
    check_with_floor(source, expected, 1);
}

fn check_with_floor(source: &'static str, expected: &'static str, minimum_compiled: u32) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let tw = tw_outcome_with_args(source, &[]);
            assert_eq!(tw.error, None, "tree-walker: {tw:?}");
            assert_eq!(
                tw.output.lines().collect::<Vec<_>>(),
                expected.lines().collect::<Vec<_>>(),
                "tree-walker"
            );
            let plain = vm_outcome_with_args(source, &[], None);
            assert_eq!(plain.error, None, "bytecode: {plain:?}");
            assert_eq!(
                plain.output.lines().collect::<Vec<_>>(),
                expected.lines().collect::<Vec<_>>(),
                "bytecode"
            );
            let tier = ForgeTier::new();
            let native = vm_outcome_with_args(source, &[], Some(&tier as &dyn NativeTier));
            assert_eq!(native.error, None, "native: {native:?}");
            assert_eq!(
                native.output.lines().collect::<Vec<_>>(),
                expected.lines().collect::<Vec<_>>(),
                "native boundary: {:?}",
                tier.runtime_stats()
            );
            assert!(
                tier.function_counts().0 > 0,
                "hot fixture must attempt function compilation"
            );
            assert!(
                tier.function_counts().1 >= minimum_compiled,
                "native compilation floor: {:?}",
                tier.function_counts()
            );
            assert!(
                tier.runtime_stats().1 > 0,
                "fixture must complete native calls: {:?}",
                tier.runtime_stats()
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn hot_mutable_scalar_returns_its_final_value() {
    check_native("## To bump (value: mutable Int) -> Int:\n    Set value to value + 1.\n    Return value.\n## Main\nLet mutable n be 0.\nRepeat for i from 1 to 600:\n    Let ignored be bump(n).\nShow n.\n", "600\n");
}

#[test]
fn hot_mutable_sequence_returns_its_replacement() {
    check("## To replace (values: mutable Seq of Int) -> Int:\n    Set values to [7].\n    Return length of values.\n## Main\nLet mutable values be [4].\nRepeat for i from 1 to 600:\n    Let ignored be replace(values).\nShow values.\n", "[7]\n");
}

#[test]
fn hot_mutable_sequence_preserves_a_local_alias() {
    check_native("## To change (values: mutable Seq of Int) -> Int:\n    Let saved be values.\n    Set item 1 of values to 9.\n    Return item 1 of saved.\n## Main\nLet mutable total be 0.\nRepeat for i from 1 to 600:\n    Let mutable values be [1].\n    Set total to total + change(values).\nShow total.\n", "600\n");
}

#[test]
fn hot_mutable_sequence_replacement_is_observed_on_every_call() {
    check(
        r#"## To replace (values: mutable Seq of Int) -> Int:
    Set values to [7].
    Return length of values.
## Main
Let mutable total be 0.
Repeat for i from 1 to 600:
    Let mutable values be [4].
    Let ignored be replace(values).
    Set total to total + item 1 of values.
Show total.
"#,
        "4200",
    );
}

#[test]
fn hot_forwarded_mutable_scalar_returns_through_both_calls() {
    check_native(
        r#"## To bump (value: mutable Int) -> Int:
    Set value to value + 1.
    Return value.
## To forward (value: mutable Int) -> Int:
    Let answer be bump(value).
    Return answer + 1.
## Main
Let mutable n be 0.
Repeat for i from 1 to 600:
    Let ignored be forward(n).
Show n.
"#,
        "600",
    );
}

#[test]
fn hot_recursive_mutable_scalar_returns_through_every_frame() {
    check_native(
        r#"## To climb (value: mutable Int, depth: Int) -> Int:
    Set value to value + 1.
    If depth is at most 0:
        Return value.
    Let answer be climb(value, depth - 1).
    Return answer + 1.
## Main
Let mutable n be 0.
Repeat for i from 1 to 200:
    Let ignored be climb(n, 4).
Show n.
"#,
        "1000",
    );
}

#[test]
fn hot_recursive_mutable_list_returns_through_every_frame() {
    check_native(
        r#"## To climb (values: mutable Seq of Int, depth: Int) -> Int:
    Set item 1 of values to item 1 of values + 1.
    If depth is at most 0:
        Return item 1 of values.
    Let answer be climb(values, depth - 1).
    Return answer + 1.
## Main
Let mutable values be [0].
Repeat for i from 1 to 200:
    Let ignored be climb(values, 4).
Show values.
"#,
        "[1000]",
    );
}

#[test]
fn hot_native_allocated_replacement_survives_each_call() {
    check_native(
        r#"## To replace (values: mutable Seq of Int) -> Int:
    Set values to [].
    Push 7 to values.
    Return 1.
## Main
Let mutable total be 0.
Repeat for i from 1 to 600:
    Let mutable values be [4].
    Let ignored be replace(values).
    Set total to total + item 1 of values.
Show total.
"#,
        "4200",
    );
}

#[test]
fn hot_native_output_and_return_keep_value_semantics() {
    check_native(
        r#"## To replace (values: mutable Seq of Int) -> Seq of Int:
    Set values to [].
    Push 7 to values.
    Return values.
## Main
Let mutable total be 0.
Repeat for i from 1 to 600:
    Let mutable values be [4].
    Let mutable answer be replace(values).
    Set item 1 of answer to 9.
    Set total to total + item 1 of values.
Show total.
"#,
        "4200",
    );
}

#[test]
fn hot_mutable_list_preserves_an_alias_made_after_a_write() {
    check_native(
        r#"## To change (values: mutable Seq of Int) -> Int:
    Set item 1 of values to 3.
    Let saved be values.
    Set item 1 of values to 9.
    Return item 1 of saved.
## Main
Let mutable total be 0.
Repeat for i from 1 to 600:
    Let mutable values be [1].
    Set total to total + change(values).
Show total.
"#,
        "1800",
    );
}

#[test]
fn recursive_mutable_list_returns_after_native_overflow_deopt() {
    check_native(
        r#"## To climb (values: mutable Seq of Int, depth: Int, large: Int) -> Int:
    Set item 1 of values to item 1 of values + 1.
    If depth is at most 0:
        Return large * large.
    Let answer be climb(values, depth - 1, large).
    Return answer + 1.
## Main
Let mutable values be [0].
Repeat for i from 1 to 200:
    Let ignored be climb(values, 4, 2).
Show climb(values, 4, 10000000000).
Show values.
"#,
        "100000000000000000004\n[1005]",
    );
}

#[test]
fn hot_mutable_float_returns_its_final_bits() {
    check_native(
        r#"## To bump (value: mutable Float) -> Int:
    Set value to value + 0.5.
    Return 1.
## Main
Let mutable value be 0.0.
Repeat for i from 1 to 600:
    Let ignored be bump(value).
Show value.
"#,
        "300",
    );
}

#[test]
fn hot_mutable_bool_used_by_a_branch_remains_an_output() {
    check_native(
        r#"## To setFlag (flag: mutable Bool, value: Int) -> Int:
    Set flag to value is greater than 0.
    If flag:
        Return 1.
    Return 0.
## Main
Let mutable total be 0.
Repeat for i from 1 to 600:
    Let mutable flag be false.
    Let ignored be setFlag(flag, 1).
    If flag:
        Set total to total + 1.
Show total.
"#,
        "600",
    );
}

#[test]
fn hot_mutable_list_output_survives_writes_to_its_copy() {
    check_native(
        r#"## To change (values: mutable Seq of Int) -> Int:
    Set item 1 of values to 3.
    Let mutable saved be values.
    Set item 1 of saved to 9.
    Return 1.
## Main
Let mutable total be 0.
Repeat for i from 1 to 600:
    Let mutable values be [1].
    Let ignored be change(values).
    Set total to total + item 1 of values.
Show total.
"#,
        "1800",
    );
}
