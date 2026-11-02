//! Exact integers and counted heap roots share a frame without sharing storage.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

fn bounded(source: &str) {
    let mut heaps = Vec::new();
    for n in [100, 1000] {
        let source = source.replace("$N", &n.to_string());
        let expected = logicaffeine_compile::compile::vm_outcome(&source);
        assert_eq!(expected.error, None);
        let held = common::wasm_host::held_integers();
        let got = common::wasm_host::run_outcome(&source).expect("combined exact/heap module");
        assert_eq!(got.failure, None);
        assert_eq!(
            got.output.lines().collect::<Vec<_>>(),
            expected.output.lines().collect::<Vec<_>>()
        );
        assert_eq!(
            common::wasm_host::held_integers(),
            held,
            "every host integer is released"
        );
        heaps.push(got.heap_ptr.expect("allocating module"));
    }
    assert_eq!(
        heaps[0], heaps[1],
        "ten times the work keeps the same heap bound"
    );
}

#[test]
fn exact_accumulators_and_temporary_lists_share_a_frame() {
    bounded(
        r#"## No Inline
## To accumulate (n: Int) -> Int:
    Let mutable wide be 9223372036854775807.
    Repeat for i from 1 to n:
        Let values be [i, i + 1].
        Set wide to wide + item 1 of values.
    Return wide.

## Main
Show accumulate($N).
"#,
    );
}

#[test]
fn exact_callers_keep_owned_lists_across_nested_collection() {
    bounded(
        r#"## No Inline
## To churn (n: Int) -> Int:
    Let mutable wide be 9223372036854775807.
    Repeat for i from 1 to n:
        Let values be [i, i + 1].
        Set wide to wide + item 1 of values.
    Return wide.

## No Inline
## To outer (n: Int) -> Int:
    Let kept be [73, 74].
    Let answer be churn(n).
    Return answer + item 1 of kept.

## Main
Show outer($N).
"#,
    );
}

#[test]
fn an_exact_range_binding_receives_each_raw_frame_value() {
    let source = r#"## No Inline
## To accumulate (n: Int) -> Int:
    Let mutable total be 0.
    Repeat for i from 1 to n:
        Set i to i + 9223372036854775807.
        Set total to total + i.
    Return total.

## Main
Show accumulate(100).
"#;
    let expected = logicaffeine_compile::compile::vm_outcome(source);
    assert_eq!(expected.error, None);
    let held = common::wasm_host::held_integers();
    let got = common::wasm_host::run_outcome(source).expect("exact range module");
    assert_eq!(got.failure, None);
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        expected.output.lines().collect::<Vec<_>>()
    );
    assert_eq!(common::wasm_host::held_integers(), held);
}
