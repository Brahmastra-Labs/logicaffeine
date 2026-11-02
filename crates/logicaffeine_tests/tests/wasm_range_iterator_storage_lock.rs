//! A numeric Repeat needs constant iterator storage; materializing its range is unnecessary.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;
use logicaffeine_compile::compile::compile_to_program;

#[test]
fn a_numeric_repeat_does_not_materialize_its_range() {
    let mut heaps = Vec::new();
    for n in [100_u64, 1000] {
        let src = format!("## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Set total to total + i.\nShow total.\n");
        let program = compile_to_program(&src).expect("valid program");
        let has_range_constructor = program
            .code
            .iter()
            .any(|op| matches!(op, logicaffeine_compile::vm::Op::NewRange { .. }));
        let got = run_outcome(&src).expect("valid wasm module");
        assert_eq!(got.failure, None);
        assert_eq!(got.output, (n * (n + 1) / 2).to_string());
        // A proved counted loop has no iterator frame. The fallback range constructor still must
        // expose the heap whose bound this test measures.
        heaps.push(if has_range_constructor {
            got.heap_ptr
                .expect("range constructor has an iterator heap")
        } else {
            got.heap_ptr.unwrap_or(0)
        });
    }
    assert_eq!(
        heaps[0], heaps[1],
        "a numeric iterator needs constant space regardless of its length"
    );
}

#[test]
fn range_bounds_are_captured_before_the_body_changes_them() {
    let src = "## Main\nLet mutable begin be 2.\nLet mutable end be 4.\nRepeat for i from begin to end:\n    Show i.\n    Set begin to 99.\n    Set end to 99.\n";
    let got = run_outcome(src).expect("valid wasm module");
    assert_eq!(got.failure, None);
    assert_eq!(got.output, "2\n3\n4");
}

#[test]
fn empty_and_singleton_ranges_keep_their_iteration_counts() {
    let src = "## Main\nLet mutable count be 0.\nRepeat for i from 4 to 2:\n    Set count to count + 1.\nRepeat for i from 7 to 7:\n    Set count to count + i.\nShow count.\n";
    let got = run_outcome(src).expect("valid wasm module");
    assert_eq!(got.failure, None);
    assert_eq!(got.output, "7");
}
