//! Word sequences preserve value semantics when indexed, appended, and passed by value.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;
use logicaffeine_compile::compile::vm_outcome;

fn exact(source: &str, expected: &str) {
    let oracle = vm_outcome(source);
    assert_eq!(oracle.error, None, "valid VM program:\n{source}");
    assert_eq!(
        oracle.output.lines().collect::<Vec<_>>(),
        expected.lines().collect::<Vec<_>>()
    );
    let got = run_outcome(source).expect("valid wasm module");
    assert_eq!(got.failure, None);
    assert_eq!(
        got.output, expected,
        "word-list writes must preserve independent values:\n{source}"
    );
}

fn write_preserves_copy(word: &str) {
    exact(&format!("## Main\nLet mutable words be [{word}(10), {word}(20)].\nLet saved be words.\nSet item 1 of words to {word}(99).\nShow saved.\nShow words.\n"), "[10, 20]\n[99, 20]");
}

fn append_preserves_copy(word: &str) {
    exact(&format!("## Main\nLet mutable words be [{word}(10), {word}(20)].\nLet saved be words.\nPush {word}(30) to words.\nShow saved.\nShow words.\n"), "[10, 20]\n[10, 20, 30]");
}

fn value_argument_preserves_caller(ty: &str, word: &str) {
    exact(&format!("## To change (words: Seq of {ty}) -> {ty}:\n    Set item 1 of words to {word}(99).\n    Return item 1 of words.\n\n## Main\nLet words be [{word}(10), {word}(20)].\nShow change(words).\nShow words.\n"), "99\n[10, 20]");
}

#[test]
fn word32_indexed_write_preserves_the_saved_copy() {
    write_preserves_copy("word32");
}
#[test]
fn word64_indexed_write_preserves_the_saved_copy() {
    write_preserves_copy("word64");
}
#[test]
fn word32_append_preserves_the_saved_copy() {
    append_preserves_copy("word32");
}
#[test]
fn word64_append_preserves_the_saved_copy() {
    append_preserves_copy("word64");
}
#[test]
fn word32_value_argument_preserves_the_caller() {
    value_argument_preserves_caller("Word32", "word32");
}
#[test]
fn word64_value_argument_preserves_the_caller() {
    value_argument_preserves_caller("Word64", "word64");
}

#[test]
fn word32_payload_bits_are_never_heap_pointers() {
    exact("## Main\nLet mutable words be [word32(4294967295)].\nLet saved be words.\nSet item 1 of words to word32(9).\nShow saved.\nShow words.\n", "[4294967295]\n[9]");
}

#[test]
fn word64_payload_bits_are_never_heap_pointers() {
    exact("## Main\nLet mutable words be [word64(0) - word64(1)].\nLet saved be words.\nSet item 1 of words to word64(9).\nShow saved.\nShow words.\n", "[18446744073709551615]\n[9]");
}

#[test]
fn word32_unmodified_clone_elements_remain_scalars() {
    exact("## Main\nLet mutable words be [word32(10), word32(4294967295)].\nLet saved be words.\nSet item 1 of words to word32(9).\nShow saved.\nShow words.\n", "[10, 4294967295]\n[9, 4294967295]");
}

#[test]
fn word64_unmodified_clone_elements_remain_scalars() {
    exact("## Main\nLet mutable words be [word64(10), word64(0) - word64(1)].\nLet saved be words.\nSet item 1 of words to word64(9).\nShow saved.\nShow words.\n", "[10, 18446744073709551615]\n[9, 18446744073709551615]");
}

fn record_word_lists(ty: &str, word: &str, deep: bool) {
    let copy = if deep { "copy of " } else { "" };
    exact(&format!("## A Box has:\n    a words, which is Seq of {ty}.\n\n## Main\nLet mutable box be a new Box with words [{word}(10), {word}(20)].\nLet saved be {copy}box.\nSet item 1 of box's words to {word}(99).\nShow saved's words.\nShow box's words.\n"), "[10, 20]\n[99, 20]");
}

fn scalar_word_copy(ty: &str, value: &str, expected: &str, boxed: bool) {
    let source = if boxed {
        format!("## A Box has:\n    a value, which is {ty}.\n\n## Main\nLet original be a new Box with value ({value}).\nLet saved be copy of original.\nShow original's value.\nShow saved's value.\n")
    } else {
        format!("## Main\nLet original be {value}.\nLet saved be copy of original.\nShow original.\nShow saved.\n")
    };
    exact(&source, &format!("{expected}\n{expected}"));
}

#[test]
fn word32_record_assignment_keeps_field_values_independent() {
    record_word_lists("Word32", "word32", false);
}
#[test]
fn word64_record_assignment_keeps_field_values_independent() {
    record_word_lists("Word64", "word64", false);
}
#[test]
fn word32_record_deep_copy_keeps_field_values_independent() {
    record_word_lists("Word32", "word32", true);
}
#[test]
fn word64_record_deep_copy_keeps_field_values_independent() {
    record_word_lists("Word64", "word64", true);
}
#[test]
fn word32_scalar_direct_copy_preserves_all_bits() {
    scalar_word_copy("Word32", "word32(4294967295)", "4294967295", false);
}
#[test]
fn word64_scalar_direct_copy_preserves_all_bits() {
    scalar_word_copy(
        "Word64",
        "word64(0) - word64(1)",
        "18446744073709551615",
        false,
    );
}
#[test]
fn word32_record_scalar_copy_preserves_all_bits() {
    scalar_word_copy("Word32", "word32(4294967295)", "4294967295", true);
}
#[test]
fn word64_record_scalar_copy_preserves_all_bits() {
    scalar_word_copy(
        "Word64",
        "word64(0) - word64(1)",
        "18446744073709551615",
        true,
    );
}
