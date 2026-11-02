//! Certified iteration counts bound local accumulators without narrowing mutable calls.
#![cfg(not(target_arch = "wasm32"))]
mod common;

#[test]
fn a_literal_bounded_count_uses_word_storage_and_preserves_output() {
    let source = r#"## Main
Let count be 0.
Let i be 0.
While i < 100:
    If i < 50:
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
    common::assert_c_output(source, "50");
    let c = logicaffeine_compile::compile::compile_to_c(source).expect("compile bounded count");
    assert!(
        c.contains("int64_t count = 0LL;"),
        "bounded count must retain word storage"
    );
}

#[test]
fn a_mutable_call_inside_an_expression_keeps_the_count_exact() {
    let source = r#"## No Inline
## To raise (count: mutable Int) -> Int:
    Set count to 2 ** 100.
    Return 0.
## Main
Let count be 0.
Let i be 0.
While i < 1:
    Show raise(count).
    Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
    let expected = "0\n1267650600228229401496703205377";
    let vm = logicaffeine_compile::compile::vm_outcome(source);
    assert_eq!(vm.error, None);
    assert_eq!(vm.output.trim_end(), expected);
    common::assert_c_output(source, expected);
}

#[test]
fn a_pure_helper_preserves_the_bounded_count_fast_path() {
    let source = r#"## No Inline
## To chosen (n: Int) -> Bool:
    Return n < 50000.
## Main
Let count be 0.
Let i be 0.
While i < 100000:
    If chosen(i):
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
    let source = logicaffeine_compile::optimization::decorate_source(source, &["inline"]);
    let vm = logicaffeine_compile::compile::vm_outcome(&source);
    assert_eq!(vm.error, None);
    assert_eq!(vm.output.trim_end(), "50000");
    common::assert_c_output(&source, "50000");
    let c =
        logicaffeine_compile::compile::compile_to_c(&source).expect("compile pure-helper count");
    assert!(
        c.contains("int64_t count = 0LL;"),
        "a proven-pure helper must not erase the count bound"
    );
}
