//! Machine-word lists have scalar elements but still own heap headers and buffers.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;
use logicaffeine_compile::compile::vm_outcome;

const CHURN: &str = r#"## To churn (n: Int) -> Int:
    Let mutable total be 0.
    Repeat for i from 1 to n:
        Let xs be [i, i + 1].
        Set total to total + item 1 of xs.
    Return total.

"#;

fn exact(source: &str, expected: &str) -> u32 {
    let oracle = vm_outcome(source);
    assert_eq!(oracle.error, None, "VM oracle must run:\n{source}");
    assert_eq!(
        oracle.output.lines().collect::<Vec<_>>(),
        expected.lines().collect::<Vec<_>>()
    );
    let got = run_outcome(source).expect("valid wasm module");
    assert_eq!(got.failure, None, "wasm must run:\n{source}");
    assert_eq!(
        got.output, expected,
        "collection preserves every live word:\n{source}"
    );
    got.heap_ptr.expect("allocating program has a heap")
}

#[test]
fn a_live_word32_literal_survives_collection_in_a_callee() {
    exact(
        &format!(
            "{CHURN}## Main\nLet kept be [word32(73), word32(74)].\nShow churn(100).\nShow kept.\n"
        ),
        "5050\n[73, 74]",
    );
}

#[test]
fn a_live_word64_literal_survives_collection_in_a_callee() {
    exact(
        &format!("{CHURN}## Main\nLet kept be [word64(4294967297), word64(4294967298)].\nShow churn(100).\nShow kept.\n"),
        "5050\n[4294967297, 4294967298]",
    );
}

fn flat_built_words(word: &str, convert: &str, unwrap: &str) {
    let mut heaps = Vec::new();
    for n in [100_u64, 1000] {
        let src = format!(
            "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let mutable xs be a new Seq of {word}.\n    Push {convert}(i) to xs.\n    Set total to total + {unwrap}(item 1 of xs).\nShow total.\n"
        );
        heaps.push(exact(&src, &(n * (n + 1) / 2).to_string()));
    }
    assert_eq!(
        heaps[0], heaps[1],
        "dropped {word} headers and buffers must be reusable"
    );
}

#[test]
fn a_word32_list_built_from_empty_is_reclaimed() {
    flat_built_words("Word32", "word32", "intOfWord32");
}

#[test]
fn a_word64_list_built_from_empty_is_reclaimed() {
    flat_built_words("Word64", "word64", "intOfWord64");
}
